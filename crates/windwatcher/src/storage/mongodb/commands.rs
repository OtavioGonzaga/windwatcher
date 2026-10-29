use mongodb::{
    bson::{Binary, Bson, Document, doc, spec::BinarySubtype},
    error::ErrorKind,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{MongoDbStorage, conversations, messages, transactions};
use crate::{
    ConversationCreateCommand, ConversationKind, CreateCommandError as Error, CreateCommandStore,
    CreateDisposition as Disposition, CreateResult, IdempotencyKey, Message, MessageId,
    MessageView, QueryStoreError, UserId, message_request_fingerprint,
};

fn bin(id: Uuid) -> Binary {
    Binary {
        subtype: BinarySubtype::Uuid,
        bytes: id.as_bytes().to_vec(),
    }
}
fn hash(bytes: [u8; 32]) -> Binary {
    Binary {
        subtype: BinarySubtype::Generic,
        bytes: bytes.to_vec(),
    }
}
fn micros(value: OffsetDateTime) -> Result<i64, Error> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1_000)).map_err(|_| {
        Error::Storage(QueryStoreError::Unexpected(Box::new(
            std::io::Error::other("timestamp out of range"),
        )))
    })
}
fn db_error(error: mongodb::error::Error) -> Error {
    if error.contains_label("TransientTransactionError")
        || error.contains_label("UnknownTransactionCommitResult")
        || matches!(
            error.kind.as_ref(),
            ErrorKind::Io(_)
                | ErrorKind::ConnectionPoolCleared { .. }
                | ErrorKind::ServerSelection { .. }
        )
    {
        Error::Storage(QueryStoreError::Unavailable(Box::new(error)))
    } else {
        Error::Storage(QueryStoreError::Unexpected(Box::new(error)))
    }
}
fn bad(s: &'static str) -> Error {
    Error::Storage(QueryStoreError::Unexpected(Box::new(
        std::io::Error::other(s),
    )))
}
fn key_filter(actor: UserId, operation: &str, scope: Uuid, key: &IdempotencyKey) -> Document {
    doc! { "actor_id": bin(actor.as_uuid()), "operation": operation, "scope_id": bin(scope), "key_hash": hash(key.hash()) }
}
#[allow(clippy::too_many_arguments)] // Parameters encode the compound idempotency key and candidate.
async fn replay_record(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    actor: UserId,
    operation: &str,
    scope: Uuid,
    key: &IdempotencyKey,
    request_hash: [u8; 32],
    candidate: Uuid,
    now: i64,
) -> mongodb::error::Result<Option<Uuid>> {
    let filter = key_filter(actor, operation, scope, key);
    if let Some(record) = db
        .collection::<Document>("idempotency_records")
        .find_one(filter.clone())
        .session(&mut *session)
        .await?
    {
        if record
            .get_i64("expires_at")
            .map_err(|_| mongodb::error::Error::custom("invalid idempotency expiry"))?
            > now
        {
            let stored = record
                .get_binary_generic("request_hash")
                .map_err(|_| mongodb::error::Error::custom("invalid request hash"))?;
            if stored.as_slice() != request_hash {
                return Err(mongodb::error::Error::custom("IDEMPOTENCY_KEY_REUSED"));
            }
            let Some(Bson::Binary(resource)) = record.get("resource_id") else {
                return Err(mongodb::error::Error::custom("invalid resource ID"));
            };
            if resource.subtype != BinarySubtype::Uuid || resource.bytes.len() != 16 {
                return Err(mongodb::error::Error::custom("invalid resource UUID"));
            }
            return Ok(Some(Uuid::from_slice(&resource.bytes).map_err(|_| {
                mongodb::error::Error::custom("invalid resource UUID")
            })?));
        }
        db.collection::<Document>("idempotency_records")
            .delete_one(filter.clone())
            .session(&mut *session)
            .await?;
    }
    db.collection::<Document>("idempotency_records").insert_one(doc! { "actor_id": bin(actor.as_uuid()), "operation": operation, "scope_id": bin(scope), "key_hash": hash(key.hash()), "request_hash": hash(request_hash), "resource_id": bin(candidate), "created_at": now, "expires_at": now.saturating_add(86_400_000_000_i64) }).session(session).await?;
    Ok(None)
}
fn duplicate_key(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
        || matches!(error.kind.as_ref(), ErrorKind::Command(command) if command.code == 11000)
}
fn is_marker(error: &mongodb::error::Error, marker: &str) -> bool {
    matches!(error.kind.as_ref(), ErrorKind::Custom(value) if value.downcast_ref::<&str>() == Some(&marker))
}

impl CreateCommandStore for MongoDbStorage {
    async fn create_conversation_command(
        &self,
        command: &ConversationCreateCommand,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<CreateResult<crate::ConversationId>, Error> {
        let db = self.database();
        let now = micros(now)?;
        let cmd = command.clone();
        let key = key.cloned();
        let candidate = cmd.conversation.id();
        let fingerprint = cmd.fingerprint();
        let result = transactions::run(&self.client, false, async move |session| {
            if let Some(key) = &key
                && let Some(existing) = replay_record(&db, session, cmd.actor, "create_conversation_v1", cmd.actor.as_uuid(), key, fingerprint, candidate.as_uuid(), now).await? {
                    let record = db.collection::<Document>("conversations").find_one(doc! { "_id": bin(existing) }).session(&mut *session).await?;
                    if record.is_none() { return Err(mongodb::error::Error::custom("IDEMPOTENT_RESULT_UNAVAILABLE")); }
                    if db.collection::<Document>("conversation_members").find_one(doc! { "conversation_id": bin(existing), "user_id": bin(cmd.actor.as_uuid()) }).session(&mut *session).await?.is_none() { return Err(mongodb::error::Error::custom("ACTOR_NOT_MEMBER")); }
                    return Ok((existing, Disposition::Replayed));
            }
            let (id, disposition) = match cmd.conversation.kind() {
                ConversationKind::Direct => {
                    let [peer] = cmd.participants.as_slice() else { return Err(mongodb::error::Error::custom("invalid direct participants")); };
                    let dkey = conversations::direct_key(cmd.actor, *peer);
                    if let Some(existing) = db.collection::<Document>("conversations").find_one(doc! { "direct_key": dkey.clone() }).session(&mut *session).await? {
                        let Some(Bson::Binary(stored_id)) = existing.get("_id") else { return Err(mongodb::error::Error::custom("invalid direct ID")); };
                        if stored_id.subtype != BinarySubtype::Uuid || stored_id.bytes.len() != 16 { return Err(mongodb::error::Error::custom("invalid direct ID")); }
                        let id = crate::ConversationId::try_from(Uuid::from_slice(&stored_id.bytes).map_err(|_| mongodb::error::Error::custom("invalid direct ID"))?).map_err(|_| mongodb::error::Error::custom("invalid direct ID"))?;
                        (id, Disposition::Existing)
                    } else {
                        let mut participants = vec![cmd.actor, *peer]; participants.sort(); participants.dedup();
                        for user in participants { if !conversations::enabled(&db, session, user).await? { return Err(mongodb::error::Error::custom("PARTICIPANT_UNAVAILABLE")); } }
                        let conversation = conversations::conv_doc(&cmd.conversation, Some(dkey)).map_err(|_| mongodb::error::Error::custom("invalid conversation"))?;
                        let mut members = Vec::new();
                        for user in [cmd.actor, *peer] { if !members.iter().any(|m: &Document| m.get("user_id") == Some(&Bson::Binary(bin(user.as_uuid())))) { members.push(conversations::member_doc(candidate, user, "direct", cmd.conversation.created_at(), None).map_err(|_| mongodb::error::Error::custom("invalid membership"))?); } }
                        db.collection::<Document>("conversations").insert_one(conversation).session(&mut *session).await?;
                        db.collection::<Document>("conversation_members").insert_many(members).session(&mut *session).await?;
                        (candidate, Disposition::Created)
                    }
                }
                ConversationKind::Group => {
                    let mut users = cmd.participants.clone(); users.push(cmd.actor); users.sort(); users.dedup();
                    for user in &users { if !conversations::enabled(&db, session, *user).await? { return Err(mongodb::error::Error::custom("PARTICIPANT_UNAVAILABLE")); } }
                    let conversation = conversations::conv_doc(&cmd.conversation, None).map_err(|_| mongodb::error::Error::custom("invalid conversation"))?;
                    let mut members = Vec::new(); for user in users { members.push(conversations::member_doc(candidate, user, if user == cmd.actor { "owner" } else { "member" }, cmd.conversation.created_at(), None).map_err(|_| mongodb::error::Error::custom("invalid membership"))?); }
                    db.collection::<Document>("conversations").insert_one(conversation).session(&mut *session).await?;
                    db.collection::<Document>("conversation_members").insert_many(members).session(&mut *session).await?;
                    (candidate, Disposition::Created)
                }
            };
            if let Some(key) = &key { db.collection::<Document>("idempotency_records").update_one(key_filter(cmd.actor, "create_conversation_v1", cmd.actor.as_uuid(), key), doc! { "$set": { "resource_id": bin(id.as_uuid()) } }).session(session).await?; }
            Ok((id.as_uuid(), disposition))
        }).await;
        match result {
            Ok((id, disposition)) => Ok(CreateResult {
                value: crate::ConversationId::try_from(id)
                    .map_err(|_| bad("invalid conversation result"))?,
                disposition,
            }),
            Err(error) if is_marker(&error, "IDEMPOTENCY_KEY_REUSED") => Err(Error::KeyReused),
            Err(error) if is_marker(&error, "IDEMPOTENT_RESULT_UNAVAILABLE") => {
                Err(Error::ResultUnavailable)
            }
            Err(error) if is_marker(&error, "ACTOR_NOT_MEMBER") => Err(Error::NotMember),
            Err(error) if is_marker(&error, "PARTICIPANT_UNAVAILABLE") => {
                Err(Error::ParticipantUnavailable)
            }
            Err(error) if duplicate_key(&error) => {
                Err(Error::Storage(QueryStoreError::Unexpected(Box::new(error))))
            }
            Err(error) => Err(db_error(error)),
        }
    }
    async fn send_message_command(
        &self,
        message: &Message,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<CreateResult<MessageView>, Error> {
        let db = self.database();
        let now = micros(now)?;
        let m = message.clone();
        let key = key.cloned();
        let candidate = m.id();
        let fingerprint = message_request_fingerprint(&m);
        let result = transactions::run(&self.client, false, async move |session| {
            if db.collection::<Document>("conversations").find_one(doc! { "_id": bin(m.conversation_id().as_uuid()) }).session(&mut *session).await?.is_none() { return Err(mongodb::error::Error::custom("CONVERSATION_NOT_FOUND")); }
            if db.collection::<Document>("conversation_members").find_one(doc! { "conversation_id": bin(m.conversation_id().as_uuid()), "user_id": bin(m.sender_id().as_uuid()) }).session(&mut *session).await?.is_none() { return Err(mongodb::error::Error::custom("NOT_MEMBER")); }
            if let Some(key) = &key
                && let Some(existing) = replay_record(&db, session, m.sender_id(), "send_message_v1", m.conversation_id().as_uuid(), key, fingerprint, candidate.as_uuid(), now).await? {
                    let id = MessageId::try_from(existing).map_err(|_| mongodb::error::Error::custom("invalid message ID"))?;
                    let record = db.collection::<Document>("messages").find_one(doc! { "_id": bin(existing), "conversation_id": bin(m.conversation_id().as_uuid()) }).session(&mut *session).await?.ok_or_else(|| mongodb::error::Error::custom("IDEMPOTENT_RESULT_UNAVAILABLE"))?;
                    let view = messages::view(&db, session, record).await?;
                    let _ = id;
                    return Ok((view, Disposition::Replayed));
            }
            if let Some(reply) = m.reply_to_message_id() {
                let target = db.collection::<Document>("messages").find_one(doc! { "_id": bin(reply.as_uuid()), "conversation_id": bin(m.conversation_id().as_uuid()), "deleted_at": Bson::Null }).session(&mut *session).await?;
                if target.is_none() { return Err(mongodb::error::Error::custom("REPLY_UNAVAILABLE")); }
            }
            let mut record = doc! { "_id": bin(m.id().as_uuid()), "conversation_id": bin(m.conversation_id().as_uuid()), "sender_id": bin(m.sender_id().as_uuid()), "content": m.content().map(|c| Bson::String(c.as_str().to_owned())).unwrap_or(Bson::Null), "reply_to_message_id": m.reply_to_message_id().map(|id| Bson::Binary(bin(id.as_uuid()))).unwrap_or(Bson::Null), "created_at": micros(m.created_at()).map_err(|_| mongodb::error::Error::custom("invalid time"))?, "edited_at": Bson::Null, "deleted_at": Bson::Null };
            db.collection::<Document>("messages").insert_one(record.clone()).session(&mut *session).await?;
            if let Some(key) = &key { db.collection::<Document>("idempotency_records").update_one(key_filter(m.sender_id(), "send_message_v1", m.conversation_id().as_uuid(), key), doc! { "$set": { "resource_id": bin(m.id().as_uuid()) } }).session(&mut *session).await?; }
            let view = messages::view(&db, session, std::mem::take(&mut record)).await?;
            Ok((view, Disposition::Created))
        }).await;
        match result {
            Ok((value, disposition)) => Ok(CreateResult { value, disposition }),
            Err(error) if is_marker(&error, "IDEMPOTENCY_KEY_REUSED") => Err(Error::KeyReused),
            Err(error) if is_marker(&error, "IDEMPOTENT_RESULT_UNAVAILABLE") => {
                Err(Error::ResultUnavailable)
            }
            Err(error) if is_marker(&error, "CONVERSATION_NOT_FOUND") => {
                Err(Error::ConversationNotFound)
            }
            Err(error) if is_marker(&error, "NOT_MEMBER") => Err(Error::NotMember),
            Err(error) if is_marker(&error, "REPLY_UNAVAILABLE") => {
                Err(Error::ReplyTargetUnavailable)
            }
            Err(error) => Err(db_error(error)),
        }
    }
}
