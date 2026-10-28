use super::{PostgresStorage, date, micros, transient};
use crate::{
    ConversationId, ConversationReadState, DeleteMessageStoreOutcome as Delete,
    EditMessageStoreOutcome as Edit, GetMessageStoreOutcome as Get,
    ListMessagesStoreOutcome as List, MarkReadStoreOutcome as Read, Message, MessageContent,
    MessageId, MessagePageSlice, MessageStore, MessageStoreError, MessageView, PageLimit,
    ReplyPreview, SendMessageStoreOutcome as Send, UnreadCountStoreOutcome as Unread, UserId,
    UserSummary,
};
use sqlx::{PgConnection, Row};
use std::{error::Error, fmt};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug)]
struct Corrupt(&'static str);
impl fmt::Display for Corrupt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Corrupt {}
fn bad(s: &'static str) -> MessageStoreError {
    MessageStoreError::Unexpected(Box::new(Corrupt(s)))
}
fn db(e: sqlx::Error) -> MessageStoreError {
    if transient(&e) {
        MessageStoreError::Unavailable(Box::new(e))
    } else if matches!(&e,sqlx::Error::Database(d)if d.is_unique_violation()||d.is_foreign_key_violation()||d.is_check_violation())
    {
        MessageStoreError::Conflict
    } else {
        MessageStoreError::Unexpected(Box::new(e))
    }
}
fn id<T: TryFrom<Uuid>>(v: Uuid) -> Result<T, MessageStoreError> {
    T::try_from(v).map_err(|_| bad("invalid persisted message identifier"))
}
fn read_message(r: &sqlx::postgres::PgRow) -> Result<Message, MessageStoreError> {
    let content: Option<String> = r.try_get("content").map_err(db)?;
    let content = content
        .map(MessageContent::new)
        .transpose()
        .map_err(|_| bad("invalid message content"))?;
    let reply: Option<Uuid> = r.try_get("reply_to_message_id").map_err(db)?;
    let reply = reply.map(id).transpose()?;
    let created = date(r.try_get("created_at").map_err(db)?).map_err(bad)?;
    let edited: Option<i64> = r.try_get("edited_at").map_err(db)?;
    let deleted: Option<i64> = r.try_get("deleted_at").map_err(db)?;
    Message::restore(
        id(r.try_get("id").map_err(db)?)?,
        id(r.try_get("conversation_id").map_err(db)?)?,
        id(r.try_get("sender_id").map_err(db)?)?,
        content,
        reply,
        created,
        edited.map(date).transpose().map_err(bad)?,
        deleted.map(date).transpose().map_err(bad)?,
    )
    .map_err(|_| bad("incoherent persisted message"))
}
pub(super) const VIEW: &str = "SELECT msg.id,msg.conversation_id,msg.sender_id,msg.content,msg.reply_to_message_id,msg.created_at,msg.edited_at,msg.deleted_at,sender.username AS sender_username,reply.sender_id AS reply_sender_id,reply_sender.username AS reply_sender_username,reply.content AS reply_content,reply.deleted_at AS reply_deleted_at FROM messages msg JOIN users sender ON sender.id=msg.sender_id LEFT JOIN messages reply ON reply.conversation_id=msg.conversation_id AND reply.id=msg.reply_to_message_id LEFT JOIN users reply_sender ON reply_sender.id=reply.sender_id";
pub(super) fn read_view(r: &sqlx::postgres::PgRow) -> Result<MessageView, MessageStoreError> {
    let message = read_message(r)?;
    let username = |col: &str| -> Result<crate::Username, MessageStoreError> {
        r.try_get::<String, _>(col)
            .map_err(db)?
            .parse()
            .map_err(|_| bad("invalid projected username"))
    };
    let reply_to = message
        .reply_to_message_id()
        .map(|reply_id| {
            let content: Option<String> = r.try_get("reply_content").map_err(db)?;
            let content = content
                .map(MessageContent::new)
                .transpose()
                .map_err(|_| bad("invalid reply content"))?;
            let deleted: Option<i64> = r.try_get("reply_deleted_at").map_err(db)?;
            let deleted = deleted.map(date).transpose().map_err(bad)?;
            if content.is_some() == deleted.is_some() {
                return Err(bad("invalid reply tombstone"));
            }
            Ok(ReplyPreview {
                id: reply_id,
                sender: UserSummary {
                    id: id(r.try_get("reply_sender_id").map_err(db)?)?,
                    username: username("reply_sender_username")?,
                },
                content,
                deleted_at: deleted,
            })
        })
        .transpose()?;
    Ok(MessageView {
        message,
        sender_username: username("sender_username")?,
        reply_to: reply_to.map(Box::new),
    })
}
#[derive(Clone, Copy)]
enum Access {
    Missing,
    NotMember,
    Member { moderator: bool },
}
async fn access(
    c: &mut PgConnection,
    actor: UserId,
    conv: ConversationId,
) -> Result<Access, MessageStoreError> {
    let row=sqlx::query("SELECT c.kind,m.role FROM conversations c LEFT JOIN conversation_members m ON m.conversation_id=c.id AND m.user_id=$1 WHERE c.id=$2").bind(actor.as_uuid()).bind(conv.as_uuid()).fetch_optional(&mut *c).await.map_err(db)?;
    let Some(r) = row else {
        return Ok(Access::Missing);
    };
    let role: Option<String> = r.try_get("role").map_err(db)?;
    let Some(role) = role else {
        return Ok(Access::NotMember);
    };
    let kind: String = r.try_get("kind").map_err(db)?;
    match (kind.as_str(), role.as_str()) {
        ("direct", "direct") | ("group", "member") => Ok(Access::Member { moderator: false }),
        ("group", "owner" | "admin") => Ok(Access::Member { moderator: true }),
        _ => Err(bad("invalid persisted membership role")),
    }
}
async fn lock_message_conversation(
    c: &mut PgConnection,
    message: MessageId,
) -> Result<Option<ConversationId>, MessageStoreError> {
    let raw: Option<Uuid> = sqlx::query_scalar("SELECT conversation_id FROM messages WHERE id=$1")
        .bind(message.as_uuid())
        .fetch_optional(&mut *c)
        .await
        .map_err(db)?;
    raw.map(id).transpose()
}
async fn by_id(
    c: &mut PgConnection,
    message: MessageId,
) -> Result<Option<MessageView>, MessageStoreError> {
    let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new(VIEW);
    query.push(" WHERE msg.id=").push_bind(message.as_uuid());
    query
        .build()
        .fetch_optional(&mut *c)
        .await
        .map_err(db)?
        .map(|r| read_view(&r))
        .transpose()
}
async fn unread(
    c: &mut PgConnection,
    conv: ConversationId,
    actor: UserId,
) -> Result<u64, MessageStoreError> {
    let n:i64=sqlx::query_scalar("SELECT COUNT(*) FROM messages msg JOIN conversation_members m ON m.conversation_id=msg.conversation_id WHERE msg.conversation_id=$1 AND m.user_id=$2 AND msg.sender_id<>$2 AND msg.deleted_at IS NULL AND (m.last_read_message_id IS NULL OR msg.id>m.last_read_message_id)").bind(conv.as_uuid()).bind(actor.as_uuid()).fetch_one(&mut *c).await.map_err(db)?;
    u64::try_from(n).map_err(|_| bad("invalid unread count"))
}
impl MessageStore for PostgresStorage {
    async fn list_messages(
        &self,
        actor: UserId,
        conversation: ConversationId,
        before: Option<MessageId>,
        limit: PageLimit,
    ) -> Result<List, MessageStoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let isolated = sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let _ = isolated;
        match access(&mut tx, actor, conversation).await? {
            Access::Missing => return Ok(List::ConversationNotFound),
            Access::NotMember => return Ok(List::NotMember),
            Access::Member { .. } => {}
        }
        let max = i64::try_from(limit.get())
            .map_err(|_| bad("message page limit exceeds database range"))?
            + 1;
        let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new(VIEW);
        q.push(" WHERE msg.conversation_id=")
            .push_bind(conversation.as_uuid());
        if let Some(before) = before {
            q.push(" AND msg.id<").push_bind(before.as_uuid());
        }
        q.push(" ORDER BY msg.id DESC LIMIT ").push_bind(max);
        let rows = q.build().fetch_all(&mut *tx).await.map_err(db)?;
        let has_more = rows.len() > limit.get();
        let items = rows
            .into_iter()
            .take(limit.get())
            .map(|r| read_view(&r))
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await.map_err(db)?;
        Ok(List::Page(MessagePageSlice { items, has_more }))
    }
    async fn send_message(&self, m: &Message) -> Result<Send, MessageStoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let locked: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM conversations WHERE id=$1 FOR UPDATE")
                .bind(m.conversation_id().as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?;
        if locked.is_none() {
            return Ok(Send::ConversationNotFound);
        }
        match access(&mut tx, m.sender_id(), m.conversation_id()).await? {
            Access::NotMember => return Ok(Send::NotMember),
            Access::Missing => return Ok(Send::ConversationNotFound),
            _ => (),
        }
        if let Some(reply) = m.reply_to_message_id() {
            let deleted: Option<Option<i64>> = sqlx::query_scalar(
                "SELECT deleted_at FROM messages WHERE conversation_id=$1 AND id=$2",
            )
            .bind(m.conversation_id().as_uuid())
            .bind(reply.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
            match deleted {
                None => return Ok(Send::ReplyTargetNotFound),
                Some(Some(_)) => return Ok(Send::ReplyTargetDeleted),
                Some(None) => (),
            }
        }
        sqlx::query("INSERT INTO messages(id,conversation_id,sender_id,content,reply_to_message_id,created_at,edited_at,deleted_at) VALUES($1,$2,$3,$4,$5,$6,NULL,NULL)").bind(m.id().as_uuid()).bind(m.conversation_id().as_uuid()).bind(m.sender_id().as_uuid()).bind(m.content().map(MessageContent::as_str)).bind(m.reply_to_message_id().map(|v|v.as_uuid())).bind(micros(m.created_at()).map_err(bad)?).execute(&mut *tx).await.map_err(db)?;
        let view = by_id(&mut tx, m.id())
            .await?
            .ok_or_else(|| bad("inserted message disappeared"))?;
        tx.commit().await.map_err(db)?;
        Ok(Send::Sent(view))
    }
    async fn get_message(
        &self,
        actor: UserId,
        message: MessageId,
    ) -> Result<Get, MessageStoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let Some(conv) = lock_message_conversation(&mut tx, message).await? else {
            return Ok(Get::MessageNotFound);
        };
        match access(&mut tx, actor, conv).await? {
            Access::NotMember | Access::Missing => return Ok(Get::NotMember),
            _ => (),
        }
        let view = by_id(&mut tx, message)
            .await?
            .ok_or_else(|| bad("message disappeared in read snapshot"))?;
        tx.commit().await.map_err(db)?;
        Ok(Get::Found(view))
    }
    async fn edit_message(
        &self,
        actor: UserId,
        message: MessageId,
        content: &MessageContent,
        now: OffsetDateTime,
    ) -> Result<Edit, MessageStoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let Some(conv) = lock_message_conversation(&mut tx, message).await? else {
            return Ok(Edit::MessageNotFound);
        };
        match access(&mut tx, actor, conv).await? {
            Access::NotMember | Access::Missing => return Ok(Edit::NotMember),
            _ => (),
        }
        let row =
            sqlx::query("SELECT sender_id,content,deleted_at FROM messages WHERE id=$1 FOR UPDATE")
                .bind(message.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?;
        let Some(row) = row else {
            return Ok(Edit::MessageNotFound);
        };
        let sender: Uuid = row.try_get("sender_id").map_err(db)?;
        if sender != actor.as_uuid() {
            return Ok(Edit::NotAuthor);
        }
        let deleted: Option<i64> = row.try_get("deleted_at").map_err(db)?;
        if deleted.is_some() {
            return Ok(Edit::Deleted);
        }
        let old: Option<String> = row.try_get("content").map_err(db)?;
        if old.as_deref() == Some(content.as_str()) {
            let view = by_id(&mut tx, message)
                .await?
                .ok_or_else(|| bad("message disappeared"))?;
            tx.commit().await.map_err(db)?;
            return Ok(Edit::Unchanged(view));
        }
        sqlx::query("UPDATE messages SET content=$1,edited_at=$2 WHERE id=$3")
            .bind(content.as_str())
            .bind(micros(now).map_err(bad)?)
            .bind(message.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let view = by_id(&mut tx, message)
            .await?
            .ok_or_else(|| bad("message disappeared"))?;
        tx.commit().await.map_err(db)?;
        Ok(Edit::Changed(view))
    }
    async fn delete_message(
        &self,
        actor: UserId,
        message: MessageId,
        now: OffsetDateTime,
    ) -> Result<Delete, MessageStoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let Some(conv) = lock_message_conversation(&mut tx, message).await? else {
            return Ok(Delete::MessageNotFound);
        };
        let moderator = match access(&mut tx, actor, conv).await? {
            Access::NotMember | Access::Missing => return Ok(Delete::NotMember),
            Access::Member { moderator } => moderator,
        };
        let row = sqlx::query("SELECT sender_id,deleted_at FROM messages WHERE id=$1 FOR UPDATE")
            .bind(message.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
        let Some(row) = row else {
            return Ok(Delete::MessageNotFound);
        };
        let sender: Uuid = row.try_get("sender_id").map_err(db)?;
        if sender != actor.as_uuid() && !moderator {
            return Ok(Delete::NotAuthorized);
        }
        let already: Option<i64> = row.try_get("deleted_at").map_err(db)?;
        if already.is_some() {
            let view = by_id(&mut tx, message)
                .await?
                .ok_or_else(|| bad("message disappeared"))?;
            tx.commit().await.map_err(db)?;
            return Ok(Delete::AlreadyDeleted(view));
        }
        sqlx::query("UPDATE messages SET content=NULL,deleted_at=$1 WHERE id=$2")
            .bind(micros(now).map_err(bad)?)
            .bind(message.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let view = by_id(&mut tx, message)
            .await?
            .ok_or_else(|| bad("message disappeared"))?;
        tx.commit().await.map_err(db)?;
        Ok(Delete::Deleted(view))
    }
    async fn mark_read_through(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
    ) -> Result<Read, MessageStoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let locked: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM conversations WHERE id=$1 FOR UPDATE")
                .bind(conversation.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?;
        if locked.is_none() {
            return Ok(Read::ConversationNotFound);
        }
        match access(&mut tx, actor, conversation).await? {
            Access::NotMember => return Ok(Read::NotMember),
            Access::Missing => return Ok(Read::ConversationNotFound),
            _ => (),
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=$1 AND id=$2)",
        )
        .bind(conversation.as_uuid())
        .bind(message.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        if !exists {
            return Ok(Read::MessageNotFound);
        }
        let r=sqlx::query("UPDATE conversation_members SET last_read_message_id=$1 WHERE conversation_id=$2 AND user_id=$3 AND (last_read_message_id IS NULL OR last_read_message_id<$1)").bind(message.as_uuid()).bind(conversation.as_uuid()).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(db)?;
        let marker:Option<Uuid>=sqlx::query_scalar("SELECT last_read_message_id FROM conversation_members WHERE conversation_id=$1 AND user_id=$2").bind(conversation.as_uuid()).bind(actor.as_uuid()).fetch_one(&mut *tx).await.map_err(db)?;
        let marker = marker.map(id).transpose()?;
        let count = unread(&mut tx, conversation, actor).await?;
        tx.commit().await.map_err(db)?;
        let state = ConversationReadState {
            last_read_message_id: marker,
            unread_count: count,
        };
        Ok(if r.rows_affected() == 1 {
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
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        match access(&mut tx, actor, conversation).await? {
            Access::Missing => return Ok(Unread::ConversationNotFound),
            Access::NotMember => return Ok(Unread::NotMember),
            _ => (),
        }
        let count = unread(&mut tx, conversation, actor).await?;
        tx.commit().await.map_err(db)?;
        Ok(Unread::Count(count))
    }
}
