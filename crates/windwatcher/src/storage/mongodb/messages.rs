use std::{error::Error, fmt};

use mongodb::{
    bson::{Binary, Bson, Document, doc, spec::BinarySubtype},
    error::ErrorKind,
};
use time::OffsetDateTime;

use super::{MongoDbStorage, transactions};
use crate::{
    ConversationId, ConversationReadState, DeleteMessageStoreOutcome as Delete,
    EditMessageStoreOutcome as Edit, GetMessageStoreOutcome as Get,
    ListMessagesStoreOutcome as List, MarkReadStoreOutcome as Read, Message, MessageContent,
    MessageId, MessagePageSlice, MessageStore, MessageStoreError, MessageView, PageLimit,
    ReplyPreview, SendMessageStoreOutcome as Send, UnreadCountStoreOutcome as Unread, UserId,
    UserSummary, Username,
};

#[derive(Debug)]
struct Invalid(&'static str);
impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Invalid {}
fn bad(message: &'static str) -> MessageStoreError {
    MessageStoreError::Unexpected(Box::new(Invalid(message)))
}
fn db_error(error: mongodb::error::Error) -> MessageStoreError {
    if error.contains_label("TransientTransactionError")
        || error.contains_label("UnknownTransactionCommitResult")
        || matches!(
            error.kind.as_ref(),
            ErrorKind::Io(_)
                | ErrorKind::ConnectionPoolCleared { .. }
                | ErrorKind::ServerSelection { .. }
        )
    {
        MessageStoreError::Unavailable(Box::new(error))
    } else if matches!(error.kind.as_ref(), ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
    {
        MessageStoreError::Conflict
    } else {
        MessageStoreError::Unexpected(Box::new(error))
    }
}
fn bin(id: uuid::Uuid) -> Binary {
    Binary {
        subtype: BinarySubtype::Uuid,
        bytes: id.as_bytes().to_vec(),
    }
}
fn uuid(doc: &Document, field: &str) -> Result<uuid::Uuid, MessageStoreError> {
    let Some(Bson::Binary(value)) = doc.get(field) else {
        return Err(bad("persisted UUID is not BSON binary"));
    };
    if value.subtype != BinarySubtype::Uuid || value.bytes.len() != 16 {
        return Err(bad("persisted UUID has invalid subtype or length"));
    }
    uuid::Uuid::from_slice(&value.bytes).map_err(|_| bad("invalid persisted UUID"))
}
fn micros(value: OffsetDateTime) -> Result<i64, MessageStoreError> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1_000))
        .map_err(|_| bad("timestamp out of range"))
}
fn time(doc: &Document, field: &str) -> Result<OffsetDateTime, MessageStoreError> {
    let micros = doc
        .get_i64(field)
        .map_err(|_| bad("persisted timestamp is not BSON Int64"))?;
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000)
        .map_err(|_| bad("invalid persisted timestamp"))
}
fn optional_time(doc: &Document, field: &str) -> Result<Option<OffsetDateTime>, MessageStoreError> {
    match doc.get(field) {
        Some(Bson::Null) => Ok(None),
        Some(Bson::Int64(value)) => {
            OffsetDateTime::from_unix_timestamp_nanos(i128::from(*value) * 1_000)
                .map(Some)
                .map_err(|_| bad("invalid persisted timestamp"))
        }
        _ => Err(bad("invalid optional timestamp")),
    }
}
fn stored_message(doc: &Document) -> Result<Message, MessageStoreError> {
    let content = match doc.get("content") {
        Some(Bson::Null) => None,
        Some(Bson::String(value)) => Some(
            MessageContent::new(value.clone())
                .map_err(|_| bad("invalid persisted message content"))?,
        ),
        _ => return Err(bad("invalid persisted content type")),
    };
    let reply = match doc.get("reply_to_message_id") {
        Some(Bson::Null) => None,
        Some(_) => Some(
            MessageId::try_from(uuid(doc, "reply_to_message_id")?)
                .map_err(|_| bad("invalid reply identifier"))?,
        ),
        _ => return Err(bad("missing persisted reply field")),
    };
    Message::restore(
        MessageId::try_from(uuid(doc, "_id")?).map_err(|_| bad("invalid message ID"))?,
        ConversationId::try_from(uuid(doc, "conversation_id")?)
            .map_err(|_| bad("invalid conversation ID"))?,
        UserId::try_from(uuid(doc, "sender_id")?).map_err(|_| bad("invalid sender ID"))?,
        content,
        reply,
        time(doc, "created_at")?,
        optional_time(doc, "edited_at")?,
        optional_time(doc, "deleted_at")?,
    )
    .map_err(|_| bad("incoherent persisted message"))
}
fn write_message(message: &Message) -> Result<Document, MessageStoreError> {
    Ok(
        doc! { "_id": bin(message.id().as_uuid()), "conversation_id": bin(message.conversation_id().as_uuid()), "sender_id": bin(message.sender_id().as_uuid()), "content": message.content().map(|value| Bson::String(value.as_str().to_owned())).unwrap_or(Bson::Null), "reply_to_message_id": message.reply_to_message_id().map(|id| Bson::Binary(bin(id.as_uuid()))).unwrap_or(Bson::Null), "created_at": micros(message.created_at())?, "edited_at": message.edited_at().map(micros).transpose()?.map(Bson::Int64).unwrap_or(Bson::Null), "deleted_at": message.deleted_at().map(micros).transpose()?.map(Bson::Int64).unwrap_or(Bson::Null) },
    )
}
fn mongo_bad(error: &'static str) -> mongodb::error::Error {
    mongodb::error::Error::custom(error)
}
pub(super) async fn view(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    record: Document,
) -> mongodb::error::Result<MessageView> {
    let m = stored_message(&record).map_err(|_| mongo_bad("invalid persisted message"))?;
    let sender = db
        .collection::<Document>("users")
        .find_one(doc! { "_id": bin(m.sender_id().as_uuid()) })
        .session(&mut *session)
        .await?
        .ok_or_else(|| mongo_bad("message sender missing"))?;
    let username = sender
        .get_str("username")
        .map_err(|_| mongo_bad("invalid sender username"))?
        .parse::<Username>()
        .map_err(|_| mongo_bad("invalid sender username"))?;
    let reply_to = if let Some(reply_id) = m.reply_to_message_id() {
        let reply = db.collection::<Document>("messages").find_one(doc! { "_id": bin(reply_id.as_uuid()), "conversation_id": bin(m.conversation_id().as_uuid()) }).session(&mut *session).await?.ok_or_else(|| mongo_bad("message reply missing"))?;
        let reply_message =
            stored_message(&reply).map_err(|_| mongo_bad("invalid reply message"))?;
        let user = db
            .collection::<Document>("users")
            .find_one(doc! { "_id": bin(reply_message.sender_id().as_uuid()) })
            .session(&mut *session)
            .await?
            .ok_or_else(|| mongo_bad("reply sender missing"))?;
        let username = user
            .get_str("username")
            .map_err(|_| mongo_bad("invalid reply username"))?
            .parse::<Username>()
            .map_err(|_| mongo_bad("invalid reply username"))?;
        Some(Box::new(ReplyPreview {
            id: reply_id,
            sender: UserSummary {
                id: reply_message.sender_id(),
                username,
            },
            content: reply_message.content().cloned(),
            deleted_at: reply_message.deleted_at(),
        }))
    } else {
        None
    };
    Ok(MessageView {
        message: m,
        sender_username: username,
        reply_to,
    })
}
async fn access(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    actor: UserId,
    conversation: ConversationId,
) -> mongodb::error::Result<Option<Option<bool>>> {
    let key = bin(conversation.as_uuid());
    let Some(conv) = db
        .collection::<Document>("conversations")
        .find_one(doc! { "_id": &key })
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    let member = db
        .collection::<Document>("conversation_members")
        .find_one(doc! { "conversation_id": &key, "user_id": bin(actor.as_uuid()) })
        .session(session)
        .await?;
    let Some(member) = member else {
        return Ok(Some(None));
    };
    let kind = conv
        .get_str("kind")
        .map_err(|_| mongo_bad("invalid conversation kind"))?;
    let role = member
        .get_str("role")
        .map_err(|_| mongo_bad("invalid membership role"))?;
    match (kind, role) {
        ("direct", "direct") | ("group", "member") => Ok(Some(Some(false))),
        ("group", "owner" | "admin") => Ok(Some(Some(true))),
        _ => Err(mongo_bad("incompatible membership role")),
    }
}
async fn unread(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    actor: UserId,
    conversation: ConversationId,
    marker: Option<MessageId>,
) -> mongodb::error::Result<u64> {
    let mut filter = doc! { "conversation_id": bin(conversation.as_uuid()), "sender_id": { "$ne": bin(actor.as_uuid()) }, "deleted_at": Bson::Null };
    if let Some(marker) = marker {
        filter.insert("_id", doc! { "$gt": bin(marker.as_uuid()) });
    }
    let count = db
        .collection::<Document>("messages")
        .count_documents(filter)
        .session(session)
        .await?;
    Ok(count)
}

impl MessageStore for MongoDbStorage {
    async fn list_messages(
        &self,
        actor: UserId,
        conversation: ConversationId,
        before: Option<MessageId>,
        limit: PageLimit,
    ) -> Result<List, MessageStoreError> {
        let db = self.database();
        let result = transactions::run(&self.client, true, async move |session| {
            match access(&db, session, actor, conversation).await? {
                None => return Ok((0_u8, Vec::new())),
                Some(None) => return Ok((1, Vec::new())),
                Some(Some(_)) => (),
            }
            let mut filter = doc! { "conversation_id": bin(conversation.as_uuid()) };
            if let Some(before) = before {
                filter.insert("_id", doc! { "$lt": bin(before.as_uuid()) });
            }
            let mut cursor = db
                .collection::<Document>("messages")
                .find(filter)
                .sort(doc! { "_id": -1 })
                .limit(
                    i64::try_from(limit.get())
                        .unwrap_or(i64::MAX)
                        .saturating_add(1),
                )
                .session(&mut *session)
                .await?;
            let mut docs = Vec::new();
            while let Some(doc) = cursor.next(&mut *session).await {
                docs.push(doc?);
            }
            let mut items = Vec::new();
            for doc in docs {
                items.push(view(&db, session, doc).await?);
            }
            Ok((2, items))
        })
        .await
        .map_err(db_error)?;
        match result.0 {
            0 => Ok(List::ConversationNotFound),
            1 => Ok(List::NotMember),
            _ => {
                let has_more = result.1.len() > limit.get();
                let mut items = result.1;
                if has_more {
                    items.pop();
                }
                Ok(List::Page(MessagePageSlice { items, has_more }))
            }
        }
    }
    async fn send_message(&self, message: &Message) -> Result<Send, MessageStoreError> {
        let record = write_message(message)?;
        let db = self.database();
        let actor = message.sender_id();
        let conv = message.conversation_id();
        let reply = message.reply_to_message_id();
        let result = transactions::run(&self.client, false, async move |session| {
            let key = bin(conv.as_uuid());
            if db
                .collection::<Document>("conversations")
                .find_one(doc! { "_id": &key })
                .session(&mut *session)
                .await?
                .is_none()
            {
                return Ok((0_u8, None));
            }
            if db
                .collection::<Document>("conversation_members")
                .find_one(doc! { "conversation_id": &key, "user_id": bin(actor.as_uuid()) })
                .session(&mut *session)
                .await?
                .is_none()
            {
                return Ok((1, None));
            }
            if let Some(reply) = reply {
                let target = db
                    .collection::<Document>("messages")
                    .find_one(doc! { "_id": bin(reply.as_uuid()), "conversation_id": &key })
                    .session(&mut *session)
                    .await?;
                let Some(target) = target else {
                    return Ok((2, None));
                };
                if !matches!(target.get("deleted_at"), Some(Bson::Null)) {
                    return Ok((3, None));
                }
            }
            db.collection::<Document>("messages")
                .insert_one(record.clone())
                .session(&mut *session)
                .await?;
            let inserted = db
                .collection::<Document>("messages")
                .find_one(doc! { "_id": record.get("_id").cloned().unwrap_or(Bson::Null) })
                .session(&mut *session)
                .await?
                .ok_or_else(|| mongo_bad("inserted message vanished"))?;
            Ok((4, Some(view(&db, session, inserted).await?)))
        })
        .await;
        match result {
            Ok((0, _)) => Ok(Send::ConversationNotFound),
            Ok((1, _)) => Ok(Send::NotMember),
            Ok((2, _)) => Ok(Send::ReplyTargetNotFound),
            Ok((3, _)) => Ok(Send::ReplyTargetDeleted),
            Ok((4, Some(result))) => Ok(Send::Sent(result)),
            Err(error) => Err(db_error(error)),
            _ => Err(bad("invalid send result")),
        }
    }
    async fn get_message(
        &self,
        actor: UserId,
        message_id: MessageId,
    ) -> Result<Get, MessageStoreError> {
        let db = self.database();
        let result = transactions::run(&self.client, true, async move |session| {
            let Some(record) = db
                .collection::<Document>("messages")
                .find_one(doc! { "_id": bin(message_id.as_uuid()) })
                .session(&mut *session)
                .await?
            else {
                return Ok((0_u8, None));
            };
            let conv = ConversationId::try_from(
                uuid(&record, "conversation_id")
                    .map_err(|_| mongo_bad("invalid message conversation"))?,
            )
            .map_err(|_| mongo_bad("invalid conversation id"))?;
            match access(&db, session, actor, conv).await? {
                None | Some(None) => return Ok((1, None)),
                Some(Some(_)) => (),
            }
            Ok((2, Some(view(&db, session, record).await?)))
        })
        .await
        .map_err(db_error)?;
        match result {
            (0, _) => Ok(Get::MessageNotFound),
            (1, _) => Ok(Get::NotMember),
            (_, Some(view)) => Ok(Get::Found(view)),
            _ => Err(bad("message read result missing")),
        }
    }
    async fn edit_message(
        &self,
        actor: UserId,
        message_id: MessageId,
        content: &MessageContent,
        now: OffsetDateTime,
    ) -> Result<Edit, MessageStoreError> {
        let timestamp = micros(now)?;
        let content = content.as_str().to_owned();
        let db = self.database();
        let result = transactions::run(&self.client, false, async move |session| {
            let Some(record) = db.collection::<Document>("messages").find_one(doc! { "_id": bin(message_id.as_uuid()) }).session(&mut *session).await? else { return Ok((0_u8, None)); };
            let conv = ConversationId::try_from(uuid(&record, "conversation_id").map_err(|_| mongo_bad("invalid message conversation"))?).map_err(|_| mongo_bad("invalid conversation id"))?;
            let member = db.collection::<Document>("conversation_members").find_one(doc! { "conversation_id": bin(conv.as_uuid()), "user_id": bin(actor.as_uuid()) }).session(&mut *session).await?;
            if member.is_none() { return Ok((1, None)); }
            if uuid(&record, "sender_id").map_err(|_| mongo_bad("invalid message author"))? != actor.as_uuid() { return Ok((2, None)); }
            if !matches!(record.get("deleted_at"), Some(Bson::Null)) { return Ok((3, None)); }
            if record.get_str("content").ok() == Some(content.as_str()) { return Ok((4, Some(view(&db, session, record).await?))); }
            db.collection::<Document>("messages").update_one(doc! { "_id": bin(message_id.as_uuid()), "deleted_at": Bson::Null }, doc! { "$set": { "content": &content, "edited_at": timestamp } }).session(&mut *session).await?;
            let updated = db.collection::<Document>("messages").find_one(doc! { "_id": bin(message_id.as_uuid()) }).session(&mut *session).await?.ok_or_else(|| mongo_bad("updated message missing"))?;
            Ok((5, Some(view(&db, session, updated).await?)))
        }).await.map_err(db_error)?;
        match result {
            (0, _) => Ok(Edit::MessageNotFound),
            (1, _) => Ok(Edit::NotMember),
            (2, _) => Ok(Edit::NotAuthor),
            (3, _) => Ok(Edit::Deleted),
            (code @ (4 | 5), Some(view)) => Ok(if code == 4 {
                Edit::Unchanged(view)
            } else {
                Edit::Changed(view)
            }),
            _ => Err(bad("invalid edit result")),
        }
    }
    async fn delete_message(
        &self,
        actor: UserId,
        message_id: MessageId,
        now: OffsetDateTime,
    ) -> Result<Delete, MessageStoreError> {
        let timestamp = micros(now)?;
        let db = self.database();
        let result = transactions::run(&self.client, false, async move |session| {
            let Some(record) = db.collection::<Document>("messages").find_one(doc! { "_id": bin(message_id.as_uuid()) }).session(&mut *session).await? else { return Ok((0_u8, None)); };
            let conv = ConversationId::try_from(uuid(&record, "conversation_id").map_err(|_| mongo_bad("invalid message conversation"))?).map_err(|_| mongo_bad("invalid conversation id"))?;
            let member = db.collection::<Document>("conversation_members").find_one(doc! { "conversation_id": bin(conv.as_uuid()), "user_id": bin(actor.as_uuid()) }).session(&mut *session).await?;
            let Some(member) = member else { return Ok((1, None)); };
            let author = uuid(&record, "sender_id").map_err(|_| mongo_bad("invalid message author"))? == actor.as_uuid();
            let role = member.get_str("role").map_err(|_| mongo_bad("invalid membership role"))?;
            let moderator = matches!(role, "owner" | "admin");
            if !author && !moderator { return Ok((2, None)); }
            if !matches!(record.get("deleted_at"), Some(Bson::Null)) { return Ok((4, Some(view(&db, session, record).await?))); }
            db.collection::<Document>("messages").update_one(doc! { "_id": bin(message_id.as_uuid()), "deleted_at": Bson::Null }, doc! { "$set": { "content": Bson::Null, "deleted_at": timestamp } }).session(&mut *session).await?;
            let updated = db.collection::<Document>("messages").find_one(doc! { "_id": bin(message_id.as_uuid()) }).session(&mut *session).await?.ok_or_else(|| mongo_bad("deleted message missing"))?;
            Ok((3, Some(view(&db, session, updated).await?)))
        }).await.map_err(db_error)?;
        match result {
            (0, _) => Ok(Delete::MessageNotFound),
            (1, _) => Ok(Delete::NotMember),
            (2, _) => Ok(Delete::NotAuthorized),
            (code @ (3 | 4), Some(view)) => Ok(if code == 3 {
                Delete::Deleted(view)
            } else {
                Delete::AlreadyDeleted(view)
            }),
            _ => Err(bad("invalid delete result")),
        }
    }
    async fn mark_read_through(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message_id: MessageId,
    ) -> Result<Read, MessageStoreError> {
        let db = self.database();
        let result = transactions::run(&self.client, false, async move |session| {
            match access(&db, session, actor, conversation).await? {
                None => return Ok((0_u8, None)),
                Some(None) => return Ok((4_u8, None)),
                Some(Some(_)) => (),
            }
            let key = bin(conversation.as_uuid());
            let actor_key = bin(actor.as_uuid());
            let target = db
                .collection::<Document>("messages")
                .find_one(doc! { "_id": bin(message_id.as_uuid()), "conversation_id": &key })
                .session(&mut *session)
                .await?;
            if target.is_none() {
                return Ok((1, None));
            }
            let member = db
                .collection::<Document>("conversation_members")
                .find_one(doc! { "conversation_id": &key, "user_id": &actor_key })
                .session(&mut *session)
                .await?
                .ok_or_else(|| mongo_bad("membership disappeared"))?;
            let current = match member.get("last_read_message_id") {
                Some(Bson::Binary(value))
                    if value.subtype == BinarySubtype::Uuid && value.bytes.len() == 16 =>
                {
                    Some(
                        MessageId::try_from(
                            uuid::Uuid::from_slice(&value.bytes)
                                .map_err(|_| mongo_bad("invalid read marker"))?,
                        )
                        .map_err(|_| mongo_bad("invalid read marker"))?,
                    )
                }
                Some(Bson::Null) => None,
                _ => return Err(mongo_bad("invalid read marker representation")),
            };
            let changed = current.is_none_or(|old| message_id > old);
            let marker = if changed { Some(message_id) } else { current };
            if changed {
                db.collection::<Document>("conversation_members")
                    .update_one(
                        doc! { "conversation_id": &key, "user_id": &actor_key },
                        doc! { "$set": { "last_read_message_id": bin(message_id.as_uuid()) } },
                    )
                    .session(&mut *session)
                    .await?;
            }
            let count = unread(&db, session, actor, conversation, marker).await?;
            Ok((
                if changed { 2 } else { 3 },
                Some(ConversationReadState {
                    last_read_message_id: marker,
                    unread_count: count,
                }),
            ))
        })
        .await
        .map_err(db_error)?;
        match result {
            (0, _) => Ok(Read::ConversationNotFound),
            (1, _) => Ok(Read::MessageNotFound),
            (4, _) => Ok(Read::NotMember),
            (2, Some(state)) => Ok(Read::Changed(state)),
            (3, Some(state)) => Ok(Read::Unchanged(state)),
            _ => Err(bad("invalid read-state result")),
        }
    }
    async fn unread_count(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> Result<Unread, MessageStoreError> {
        let db = self.database();
        let result = transactions::run(&self.client, true, async move |session| {
            match access(&db, session, actor, conversation).await? { None => return Ok((0_u8, 0_u64)), Some(None) => return Ok((2_u8, 0_u64)), Some(Some(_)) => () }
            let member = db.collection::<Document>("conversation_members").find_one(doc! { "conversation_id": bin(conversation.as_uuid()), "user_id": bin(actor.as_uuid()) }).session(&mut *session).await?.ok_or_else(|| mongo_bad("membership disappeared"))?;
            let marker = match member.get("last_read_message_id") { Some(Bson::Binary(value)) if value.subtype == BinarySubtype::Uuid && value.bytes.len() == 16 => Some(MessageId::try_from(uuid::Uuid::from_slice(&value.bytes).map_err(|_| mongo_bad("invalid read marker"))?).map_err(|_| mongo_bad("invalid read marker"))?), Some(Bson::Null) => None, _ => return Err(mongo_bad("invalid read marker representation")) };
            Ok((1, unread(&db, session, actor, conversation, marker).await?))
        }).await.map_err(db_error)?;
        Ok(match result.0 {
            0 => Unread::ConversationNotFound,
            2 => Unread::NotMember,
            _ => Unread::Count(result.1),
        })
    }
}
