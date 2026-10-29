use mongodb::{
    ClientSession, Database,
    bson::{Binary, Bson, Document, doc, spec::BinarySubtype},
    error::ErrorKind,
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::{MongoDbStorage, conversations, messages, transactions};
use crate::{
    ConversationCreateCommand, ConversationId, ConversationKind, CreateCommandError as Error,
    CreateCommandStore, CreateDisposition as Disposition, CreateResult, IdempotencyKey, Message,
    MessageId, MessageView, QueryStoreError, UserId, message_request_fingerprint,
};

type Result<T> = mongodb::error::Result<T>;

#[derive(Clone, Copy)]
enum Rejected {
    KeyReused,
    ResultUnavailable,
    NotMember,
    ConversationNotFound,
    ParticipantUnavailable,
    ReplyTargetUnavailable,
}
impl From<Rejected> for mongodb::error::Error {
    fn from(value: Rejected) -> Self {
        Self::custom(value)
    }
}
fn invalid(message: &'static str) -> mongodb::error::Error {
    mongodb::error::Error::custom(message)
}
fn command_error(error: mongodb::error::Error) -> Error {
    if let ErrorKind::Custom(value) = error.kind.as_ref() {
        if let Some(rejected) = value.downcast_ref::<Rejected>() {
            return match rejected {
                Rejected::KeyReused => Error::KeyReused,
                Rejected::ResultUnavailable => Error::ResultUnavailable,
                Rejected::NotMember => Error::NotMember,
                Rejected::ConversationNotFound => Error::ConversationNotFound,
                Rejected::ParticipantUnavailable => Error::ParticipantUnavailable,
                Rejected::ReplyTargetUnavailable => Error::ReplyTargetUnavailable,
            };
        }
        if value.is::<UserId>() {
            return Error::ParticipantUnavailable;
        }
    }
    Error::Storage(
        if error.contains_label("TransientTransactionError")
            || error.contains_label("UnknownTransactionCommitResult")
            || matches!(
                error.kind.as_ref(),
                ErrorKind::Io(_)
                    | ErrorKind::ConnectionPoolCleared { .. }
                    | ErrorKind::ServerSelection { .. }
            )
        {
            QueryStoreError::Unavailable(Box::new(error))
        } else {
            QueryStoreError::Unexpected(Box::new(error))
        },
    )
}
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
fn micros(value: OffsetDateTime) -> Result<i64> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1_000))
        .map_err(|_| invalid("timestamp out of range"))
}

// Keeps reservation and final resource mapping in the caller's transaction.
struct Record {
    filter: Document,
    fingerprint: [u8; 32],
    candidate: Uuid,
    now: i64,
    expires_at: i64,
}
impl Record {
    fn conversation(
        command: &ConversationCreateCommand,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<Option<Self>> {
        key.map(|key| Ok(Self {
            filter: doc! { "actor_id": bin(command.actor.as_uuid()), "operation": "create_conversation_v1", "scope_id": bin(command.actor.as_uuid()), "key_hash": hash(key.hash()) },
            fingerprint: command.fingerprint(), candidate: command.conversation.id().as_uuid(),
            now: micros(now)?, expires_at: Self::expiry(now)?,
        })).transpose()
    }
    fn message(
        message: &Message,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<Option<Self>> {
        key.map(|key| Ok(Self {
            filter: doc! { "actor_id": bin(message.sender_id().as_uuid()), "operation": "send_message_v1", "scope_id": bin(message.conversation_id().as_uuid()), "key_hash": hash(key.hash()) },
            fingerprint: message_request_fingerprint(message), candidate: message.id().as_uuid(),
            now: micros(now)?, expires_at: Self::expiry(now)?,
        })).transpose()
    }
    fn expiry(now: OffsetDateTime) -> Result<i64> {
        micros(
            now.checked_add(Duration::days(1))
                .ok_or_else(|| invalid("idempotency expiry overflow"))?,
        )
    }
    async fn reserve(&self, db: &Database, session: &mut ClientSession) -> Result<Option<Uuid>> {
        let collection = db.collection::<Document>("idempotency_records");
        if let Some(record) = collection
            .find_one(self.filter.clone())
            .session(&mut *session)
            .await?
        {
            let expires = record
                .get_i64("expires_at")
                .map_err(|_| invalid("invalid idempotency expiry"))?;
            if expires > self.now {
                let fingerprint = record
                    .get_binary_generic("request_hash")
                    .map_err(|_| invalid("invalid request hash"))?;
                if fingerprint.len() != 32 {
                    return Err(invalid("invalid request hash length"));
                }
                if fingerprint.as_slice() != self.fingerprint {
                    return Err(Rejected::KeyReused.into());
                }
                let Some(Bson::Binary(resource)) = record.get("resource_id") else {
                    return Err(invalid("invalid resource ID"));
                };
                if resource.subtype != BinarySubtype::Uuid || resource.bytes.len() != 16 {
                    return Err(invalid("invalid resource UUID"));
                }
                return Ok(Some(
                    Uuid::from_slice(&resource.bytes)
                        .map_err(|_| invalid("invalid resource UUID"))?,
                ));
            }
            collection
                .delete_one(self.filter.clone())
                .session(&mut *session)
                .await?;
        }
        let mut record = self.filter.clone();
        record.extend(doc! { "request_hash": hash(self.fingerprint), "resource_id": bin(self.candidate), "created_at": self.now, "expires_at": self.expires_at });
        collection.insert_one(record).session(session).await?;
        Ok(None)
    }
    async fn finish(
        &self,
        db: &Database,
        session: &mut ClientSession,
        resource: Uuid,
    ) -> Result<()> {
        db.collection::<Document>("idempotency_records")
            .update_one(
                self.filter.clone(),
                doc! { "$set": { "resource_id": bin(resource) } },
            )
            .session(session)
            .await?;
        Ok(())
    }
}

async fn is_member(
    db: &Database,
    session: &mut ClientSession,
    actor: UserId,
    conversation: ConversationId,
) -> Result<bool> {
    Ok(conversations::access(db, session, conversation, actor)
        .await?
        .and_then(|(_, member)| member)
        .is_some())
}

async fn create_in(
    db: &Database,
    session: &mut ClientSession,
    command: &ConversationCreateCommand,
    record: Option<&Record>,
) -> Result<CreateResult<ConversationId>> {
    if let Some(record) = record
        && let Some(resource) = record.reserve(db, session).await?
    {
        let id = ConversationId::try_from(resource)
            .map_err(|_| invalid("invalid conversation result"))?;
        if !conversations::touch(db, session, id).await? {
            return Err(Rejected::ResultUnavailable.into());
        }
        if !is_member(db, session, command.actor, id).await? {
            return Err(Rejected::NotMember.into());
        }
        return Ok(CreateResult {
            value: id,
            disposition: Disposition::Replayed,
        });
    }
    let (value, disposition) = match command.conversation.kind() {
        ConversationKind::Direct => {
            let [peer] = command.participants.as_slice() else {
                return Err(invalid("Direct command requires exactly one peer"));
            };
            let (document, created) =
                conversations::direct(db, session, &command.conversation, command.actor, *peer)
                    .await?;
            let conversation = conversations::read_conversation(&document)
                .map_err(|_| invalid("invalid direct conversation"))?;
            (
                conversation.id(),
                if created {
                    Disposition::Created
                } else {
                    Disposition::Existing
                },
            )
        }
        ConversationKind::Group => {
            let mut users = command.participants.clone();
            users.push(command.actor);
            users.sort();
            users.dedup();
            for user in &users {
                if !conversations::enabled(db, session, *user).await? {
                    return Err(Rejected::ParticipantUnavailable.into());
                }
            }
            let conversation = conversations::conv_doc(&command.conversation, None)
                .map_err(|_| invalid("invalid conversation"))?;
            let members = users
                .into_iter()
                .map(|user| {
                    conversations::member_doc(
                        command.conversation.id(),
                        user,
                        if user == command.actor {
                            "owner"
                        } else {
                            "member"
                        },
                        command.conversation.created_at(),
                        None,
                    )
                    .map_err(|_| invalid("invalid membership"))
                })
                .collect::<Result<Vec<_>>>()?;
            db.collection::<Document>("conversations")
                .insert_one(conversation)
                .session(&mut *session)
                .await?;
            db.collection::<Document>("conversation_members")
                .insert_many(members)
                .session(&mut *session)
                .await?;
            (command.conversation.id(), Disposition::Created)
        }
    };
    if let Some(record) = record {
        record.finish(db, session, value.as_uuid()).await?;
    }
    Ok(CreateResult { value, disposition })
}

async fn send_in(
    db: &Database,
    session: &mut ClientSession,
    message: &Message,
    document: &Document,
    record: Option<&Record>,
) -> Result<CreateResult<MessageView>> {
    let replay = match record {
        Some(record) => record.reserve(db, session).await?,
        None => None,
    };
    if !conversations::touch(db, session, message.conversation_id()).await? {
        return Err(if replay.is_some() {
            Rejected::ResultUnavailable
        } else {
            Rejected::ConversationNotFound
        }
        .into());
    }
    // Resolve replay before authorization, as in SQL: missing results are not
    // re-created and are distinct from never-existing conversation requests.
    let replay_document = if let Some(resource) = replay {
        MessageId::try_from(resource).map_err(|_| invalid("invalid message result"))?;
        Some(db.collection::<Document>("messages").find_one(doc! { "_id": bin(resource), "conversation_id": bin(message.conversation_id().as_uuid()) })
            .session(&mut *session).await?.ok_or(Rejected::ResultUnavailable)?)
    } else {
        None
    };
    if !is_member(db, session, message.sender_id(), message.conversation_id()).await? {
        return Err(Rejected::NotMember.into());
    }
    if let Some(document) = replay_document {
        return Ok(CreateResult {
            value: messages::view(db, session, document).await?,
            disposition: Disposition::Replayed,
        });
    }
    if let Some(reply) = message.reply_to_message_id() {
        let target = db.collection::<Document>("messages").find_one(doc! { "_id": bin(reply.as_uuid()), "conversation_id": bin(message.conversation_id().as_uuid()), "deleted_at": Bson::Null })
            .session(&mut *session).await?;
        if target.is_none() {
            return Err(Rejected::ReplyTargetUnavailable.into());
        }
    }
    db.collection::<Document>("messages")
        .insert_one(document.clone())
        .session(&mut *session)
        .await?;
    if let Some(record) = record {
        record.finish(db, session, message.id().as_uuid()).await?;
    }
    Ok(CreateResult {
        value: messages::view(db, session, document.clone()).await?,
        disposition: Disposition::Created,
    })
}

impl CreateCommandStore for MongoDbStorage {
    async fn create_conversation_command(
        &self,
        command: &ConversationCreateCommand,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> std::result::Result<CreateResult<ConversationId>, Error> {
        let db = self.database();
        // A uniqueness loser must restart the entire reservation/create with a
        // fresh snapshot. MongoDB does not always label 11000 as transient.
        for attempt in 0..4 {
            let (db, command) = (db.clone(), command.clone());
            let record = Record::conversation(&command, key, now).map_err(command_error)?;
            let result = transactions::run(&self.client, false, async move |session| {
                create_in(&db, session, &command, record.as_ref()).await
            })
            .await;
            if let Err(error) = &result
                && attempt < 3
                && transactions::duplicate_key(error)
            {
                continue;
            }
            return result.map_err(command_error);
        }
        unreachable!()
    }
    async fn send_message_command(
        &self,
        message: &Message,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> std::result::Result<CreateResult<MessageView>, Error> {
        let db = self.database();
        let document = messages::write_message(message).map_err(Error::Message)?;
        for attempt in 0..4 {
            let (db, message, document) = (db.clone(), message.clone(), document.clone());
            let record = Record::message(&message, key, now).map_err(command_error)?;
            let result = transactions::run(&self.client, false, async move |session| {
                send_in(&db, session, &message, &document, record.as_ref()).await
            })
            .await;
            if let Err(error) = &result
                && attempt < 3
                && transactions::duplicate_key(error)
            {
                continue;
            }
            return result.map_err(command_error);
        }
        unreachable!()
    }
}
