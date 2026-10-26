use crate::{ListMessagesStoreOutcome as List, MessagePageSlice, PageLimit};
use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{Corrupt, SqliteStorage, from_micros, id_bytes, to_micros, unavailable};
use crate::{
    ConversationId, DeleteMessageStoreOutcome as Delete, EditMessageStoreOutcome as Edit,
    GetMessageStoreOutcome as Get, MarkReadStoreOutcome as Read, Message, MessageContent,
    MessageId, MessageStore, MessageStoreError, MessageView, ReplyPreview,
    SendMessageStoreOutcome as Send, UnreadCountStoreOutcome as Unread, UserId, UserSummary,
};

fn unexpected(message: &'static str) -> MessageStoreError {
    MessageStoreError::Unexpected(Box::new(Corrupt(message)))
}

fn storage_error(error: sqlx::Error) -> MessageStoreError {
    if unavailable(&error) {
        MessageStoreError::Unavailable(Box::new(error))
    } else if matches!(&error, sqlx::Error::Database(db) if db.is_unique_violation() || db.is_foreign_key_violation() || db.is_check_violation())
    {
        MessageStoreError::Conflict
    } else {
        MessageStoreError::Unexpected(Box::new(error))
    }
}

fn identifier<T: TryFrom<Uuid>>(bytes: Vec<u8>) -> Result<T, MessageStoreError> {
    Uuid::from_slice(&bytes)
        .ok()
        .and_then(|id| T::try_from(id).ok())
        .ok_or_else(|| unexpected("invalid persisted message identifier"))
}

fn read_message(row: &SqliteRow) -> Result<Message, MessageStoreError> {
    let content = row
        .try_get::<Option<String>, _>("content")
        .map_err(storage_error)?
        .map(MessageContent::new)
        .transpose()
        .map_err(|_| unexpected("invalid persisted message content"))?;
    let reply = row
        .try_get::<Option<Vec<u8>>, _>("reply_to_message_id")
        .map_err(storage_error)?
        .map(identifier)
        .transpose()?;
    let created =
        from_micros(row.try_get("created_at").map_err(storage_error)?).map_err(unexpected)?;
    let edited = row
        .try_get::<Option<i64>, _>("edited_at")
        .map_err(storage_error)?
        .map(from_micros)
        .transpose()
        .map_err(unexpected)?;
    let deleted = row
        .try_get::<Option<i64>, _>("deleted_at")
        .map_err(storage_error)?
        .map(from_micros)
        .transpose()
        .map_err(unexpected)?;
    Message::restore(
        identifier(row.try_get("id").map_err(storage_error)?)?,
        identifier(row.try_get("conversation_id").map_err(storage_error)?)?,
        identifier(row.try_get("sender_id").map_err(storage_error)?)?,
        content,
        reply,
        created,
        edited,
        deleted,
    )
    .map_err(|_| unexpected("invalid persisted message state"))
}

pub(super) const VIEW_SELECT: &str = "SELECT msg.id, msg.conversation_id, msg.sender_id, msg.content, msg.reply_to_message_id, msg.created_at, msg.edited_at, msg.deleted_at, sender.username AS sender_username, reply.sender_id AS reply_sender_id, reply_sender.username AS reply_sender_username, reply.content AS reply_content, reply.deleted_at AS reply_deleted_at FROM messages msg JOIN users sender ON sender.id = msg.sender_id LEFT JOIN messages reply ON reply.conversation_id = msg.conversation_id AND reply.id = msg.reply_to_message_id LEFT JOIN users reply_sender ON reply_sender.id = reply.sender_id";

fn view_query() -> sqlx::QueryBuilder<sqlx::Sqlite> {
    sqlx::QueryBuilder::new(VIEW_SELECT)
}

pub(super) fn read_view(row: &SqliteRow) -> Result<MessageView, MessageStoreError> {
    let message = read_message(row)?;
    let username = |column| -> Result<crate::Username, MessageStoreError> {
        row.try_get::<String, _>(column)
            .map_err(storage_error)?
            .parse()
            .map_err(|_| unexpected("invalid projected username"))
    };
    let reply_to = message
        .reply_to_message_id()
        .map(|id| {
            let content = row
                .try_get::<Option<String>, _>("reply_content")
                .map_err(storage_error)?
                .map(MessageContent::new)
                .transpose()
                .map_err(|_| unexpected("invalid reply content"))?;
            let deleted_at = row
                .try_get::<Option<i64>, _>("reply_deleted_at")
                .map_err(storage_error)?
                .map(from_micros)
                .transpose()
                .map_err(unexpected)?;
            if content.is_some() == deleted_at.is_some() {
                return Err(unexpected("invalid reply tombstone"));
            }
            Ok(ReplyPreview {
                id,
                sender: UserSummary {
                    id: identifier(row.try_get("reply_sender_id").map_err(storage_error)?)?,
                    username: username("reply_sender_username")?,
                },
                content,
                deleted_at,
            })
        })
        .transpose()?;
    Ok(MessageView {
        message,
        sender_username: username("sender_username")?,
        reply_to,
    })
}

enum Access {
    ConversationNotFound,
    NotMember,
    Member { moderator: bool },
}

// Used within a read snapshot or an IMMEDIATE write transaction. The latter
// prevents membership/role changes between authorization and mutation/commit.
async fn access(
    conn: &mut SqliteConnection,
    actor: UserId,
    conversation: ConversationId,
) -> Result<Access, MessageStoreError> {
    let row = sqlx::query("SELECT c.kind, m.role FROM conversations c LEFT JOIN conversation_members m ON m.conversation_id = c.id AND m.user_id = ? WHERE c.id = ?")
        .bind(id_bytes(actor)).bind(id_bytes(conversation)).fetch_optional(conn).await.map_err(storage_error)?;
    let Some(row) = row else {
        return Ok(Access::ConversationNotFound);
    };
    let Some(role) = row
        .try_get::<Option<String>, _>("role")
        .map_err(storage_error)?
    else {
        return Ok(Access::NotMember);
    };
    let kind: String = row.try_get("kind").map_err(storage_error)?;
    match (kind.as_str(), role.as_str()) {
        ("direct", "direct") => Ok(Access::Member { moderator: false }),
        ("group", "owner" | "admin") => Ok(Access::Member { moderator: true }),
        ("group", "member") => Ok(Access::Member { moderator: false }),
        _ => Err(unexpected("incompatible persisted membership role")),
    }
}

async fn find(
    conn: &mut SqliteConnection,
    conversation: ConversationId,
    message: MessageId,
) -> Result<Option<MessageView>, MessageStoreError> {
    view_query()
        .push(" WHERE msg.conversation_id = ")
        .push_bind(id_bytes(conversation))
        .push(" AND msg.id = ")
        .push_bind(id_bytes(message))
        .build()
        .fetch_optional(conn)
        .await
        .map_err(storage_error)?
        .map(|row| read_view(&row))
        .transpose()
}

// Global item resolution shares the caller's read snapshot or write lock with
// membership/role checks. Never ask callers to discover a conversation first.
async fn find_global(
    conn: &mut SqliteConnection,
    message: MessageId,
) -> Result<Option<MessageView>, MessageStoreError> {
    view_query()
        .push(" WHERE msg.id = ")
        .push_bind(id_bytes(message))
        .build()
        .fetch_optional(conn)
        .await
        .map_err(storage_error)?
        .map(|row| read_view(&row))
        .transpose()
}

impl MessageStore for SqliteStorage {
    async fn list_messages(
        &self,
        actor: UserId,
        conversation: ConversationId,
        before: Option<MessageId>,
        limit: PageLimit,
    ) -> Result<List, MessageStoreError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        match access(&mut tx, actor, conversation).await? {
            Access::ConversationNotFound => return Ok(List::ConversationNotFound),
            Access::NotMember => return Ok(List::NotMember),
            Access::Member { .. } => (),
        }
        let count = (limit.get() + 1) as i64;
        let rows = if let Some(boundary) = before {
            view_query()
                .push(" WHERE msg.conversation_id = ")
                .push_bind(id_bytes(conversation))
                .push(" AND msg.id < ")
                .push_bind(id_bytes(boundary))
                .push(" ORDER BY msg.id DESC LIMIT ")
                .push_bind(count)
                .build()
                .fetch_all(&mut *tx)
                .await
                .map_err(storage_error)?
        } else {
            view_query()
                .push(" WHERE msg.conversation_id = ")
                .push_bind(id_bytes(conversation))
                .push(" ORDER BY msg.id DESC LIMIT ")
                .push_bind(count)
                .build()
                .fetch_all(&mut *tx)
                .await
                .map_err(storage_error)?
        };
        let has_more = rows.len() > limit.get();
        let items = rows
            .iter()
            .take(limit.get())
            .map(read_view)
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await.map_err(storage_error)?;
        Ok(List::Page(MessagePageSlice { items, has_more }))
    }
    async fn send_message(&self, message: &Message) -> Result<Send, MessageStoreError> {
        if message.is_deleted() || message.edited_at().is_some() {
            return Err(unexpected("send requires an active unedited message"));
        }
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        match access(&mut tx, message.sender_id(), message.conversation_id()).await? {
            Access::ConversationNotFound => return Ok(Send::ConversationNotFound),
            Access::NotMember => return Ok(Send::NotMember),
            Access::Member { .. } => (),
        }
        if let Some(reply) = message.reply_to_message_id() {
            match find(&mut tx, message.conversation_id(), reply).await? {
                None => return Ok(Send::ReplyTargetNotFound),
                Some(target) if target.is_deleted() => return Ok(Send::ReplyTargetDeleted),
                Some(_) => (),
            }
        }
        sqlx::query("INSERT INTO messages (id, conversation_id, sender_id, content, reply_to_message_id, created_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(id_bytes(message.id())).bind(id_bytes(message.conversation_id())).bind(id_bytes(message.sender_id()))
            .bind(message.content().map(MessageContent::as_str)).bind(message.reply_to_message_id().map(id_bytes))
            .bind(to_micros(message.created_at()).map_err(unexpected)?).execute(&mut *tx).await.map_err(storage_error)?;
        let persisted = find(&mut tx, message.conversation_id(), message.id())
            .await?
            .ok_or_else(|| unexpected("persisted message disappeared"))?;
        tx.commit().await.map_err(storage_error)?;
        Ok(Send::Sent(persisted))
    }

    async fn get_message(
        &self,
        actor: UserId,
        message: MessageId,
    ) -> Result<Get, MessageStoreError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let Some(current) = find_global(&mut tx, message).await? else {
            return Ok(Get::MessageNotFound);
        };
        match access(&mut tx, actor, current.conversation_id()).await? {
            Access::ConversationNotFound => return Ok(Get::MessageNotFound),
            Access::NotMember => return Ok(Get::NotMember),
            Access::Member { .. } => (),
        }
        tx.commit().await.map_err(storage_error)?;
        Ok(Get::Found(current))
    }

    async fn edit_message(
        &self,
        actor: UserId,
        message: MessageId,
        content: &MessageContent,
        now: OffsetDateTime,
    ) -> Result<Edit, MessageStoreError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let Some(current) = find_global(&mut tx, message).await? else {
            return Ok(Edit::MessageNotFound);
        };
        let conversation = current.conversation_id();
        match access(&mut tx, actor, conversation).await? {
            Access::ConversationNotFound => return Ok(Edit::MessageNotFound),
            Access::NotMember => return Ok(Edit::NotMember),
            Access::Member { .. } => (),
        }
        if current.sender_id() != actor {
            return Ok(Edit::NotAuthor);
        }
        if current.is_deleted() {
            return Ok(Edit::Deleted);
        }
        if current.content() == Some(content) {
            return Ok(Edit::Unchanged(current));
        }
        sqlx::query("UPDATE messages SET content = ?, edited_at = ? WHERE conversation_id = ? AND id = ? AND sender_id = ? AND deleted_at IS NULL")
            .bind(content.as_str()).bind(to_micros(now).map_err(unexpected)?).bind(id_bytes(conversation)).bind(id_bytes(message)).bind(id_bytes(actor))
            .execute(&mut *tx).await.map_err(storage_error)?;
        let persisted = find(&mut tx, conversation, message)
            .await?
            .ok_or_else(|| unexpected("edited message disappeared"))?;
        tx.commit().await.map_err(storage_error)?;
        Ok(Edit::Changed(persisted))
    }

    async fn delete_message(
        &self,
        actor: UserId,
        message: MessageId,
        now: OffsetDateTime,
    ) -> Result<Delete, MessageStoreError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let Some(current) = find_global(&mut tx, message).await? else {
            return Ok(Delete::MessageNotFound);
        };
        let conversation = current.conversation_id();
        let moderator = match access(&mut tx, actor, conversation).await? {
            Access::ConversationNotFound => return Ok(Delete::MessageNotFound),
            Access::NotMember => return Ok(Delete::NotMember),
            Access::Member { moderator } => moderator,
        };
        if current.sender_id() != actor && !moderator {
            return Ok(Delete::NotAuthorized);
        }
        if current.is_deleted() {
            return Ok(Delete::AlreadyDeleted(current));
        }
        sqlx::query("UPDATE messages SET content = NULL, deleted_at = ? WHERE conversation_id = ? AND id = ? AND deleted_at IS NULL")
            .bind(to_micros(now).map_err(unexpected)?).bind(id_bytes(conversation)).bind(id_bytes(message))
            .execute(&mut *tx).await.map_err(storage_error)?;
        let persisted = find(&mut tx, conversation, message)
            .await?
            .ok_or_else(|| unexpected("deleted message disappeared"))?;
        tx.commit().await.map_err(storage_error)?;
        Ok(Delete::Deleted(persisted))
    }

    async fn mark_read_through(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
    ) -> Result<Read, MessageStoreError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        match access(&mut tx, actor, conversation).await? {
            Access::ConversationNotFound => return Ok(Read::ConversationNotFound),
            Access::NotMember => return Ok(Read::NotMember),
            Access::Member { .. } => (),
        }
        if find(&mut tx, conversation, message).await?.is_none() {
            return Ok(Read::MessageNotFound);
        }
        let result = sqlx::query("UPDATE conversation_members SET last_read_message_id = ? WHERE conversation_id = ? AND user_id = ? AND (last_read_message_id IS NULL OR last_read_message_id < ?)")
            .bind(id_bytes(message)).bind(id_bytes(conversation)).bind(id_bytes(actor)).bind(id_bytes(message))
            .execute(&mut *tx).await.map_err(storage_error)?;
        let row = sqlx::query("SELECT m.last_read_message_id, (SELECT COUNT(*) FROM messages msg WHERE msg.conversation_id = m.conversation_id AND msg.sender_id <> m.user_id AND msg.deleted_at IS NULL AND (m.last_read_message_id IS NULL OR msg.id > m.last_read_message_id)) AS unread_count FROM conversation_members m WHERE m.conversation_id = ? AND m.user_id = ?")
            .bind(id_bytes(conversation)).bind(id_bytes(actor)).fetch_one(&mut *tx).await.map_err(storage_error)?;
        let state = crate::ConversationReadState {
            last_read_message_id: row
                .try_get::<Option<Vec<u8>>, _>("last_read_message_id")
                .map_err(storage_error)?
                .map(identifier)
                .transpose()?,
            unread_count: u64::try_from(
                row.try_get::<i64, _>("unread_count")
                    .map_err(storage_error)?,
            )
            .map_err(|_| unexpected("invalid unread count"))?,
        };
        tx.commit().await.map_err(storage_error)?;
        Ok(if result.rows_affected() == 1 {
            Read::Changed(state)
        } else {
            Read::Unchanged(state)
        })
    }

    async fn unread_count(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> Result<Unread, MessageStoreError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        match access(&mut tx, actor, conversation).await? {
            Access::ConversationNotFound => return Ok(Unread::ConversationNotFound),
            Access::NotMember => return Ok(Unread::NotMember),
            Access::Member { .. } => (),
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages msg JOIN conversation_members m ON m.conversation_id = msg.conversation_id WHERE m.conversation_id = ? AND m.user_id = ? AND msg.sender_id <> m.user_id AND msg.deleted_at IS NULL AND (m.last_read_message_id IS NULL OR msg.id > m.last_read_message_id)")
            .bind(id_bytes(conversation)).bind(id_bytes(actor)).fetch_one(&mut *tx).await.map_err(storage_error)?;
        let count = u64::try_from(count).map_err(|_| unexpected("invalid unread count"))?;
        tx.commit().await.map_err(storage_error)?;
        Ok(Unread::Count(count))
    }
}
