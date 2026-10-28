use super::{PostgresStorage, date, micros, transient};
use crate::{
    ConversationBoundary, ConversationId, ConversationMemberSummary,
    ConversationQueryOutcome as Outcome, ConversationSummary, MemberBoundary, MessageId,
    MessagingQueryStore, PageLimit, QuerySlice, QueryStoreError, UserId, UserSummary, Username,
};
use sqlx::{PgConnection, Row};
use std::{collections::HashMap, error::Error, fmt};
use uuid::Uuid;

#[derive(Debug)]
struct Bad(&'static str);
impl fmt::Display for Bad {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Bad {}
fn bad(s: &'static str) -> QueryStoreError {
    QueryStoreError::Unexpected(Box::new(Bad(s)))
}
fn db(e: sqlx::Error) -> QueryStoreError {
    if transient(&e) {
        QueryStoreError::Unavailable(Box::new(e))
    } else {
        QueryStoreError::Unexpected(Box::new(e))
    }
}
fn uid<T: TryFrom<Uuid>>(u: Uuid) -> Result<T, QueryStoreError> {
    T::try_from(u).map_err(|_| bad("invalid query identifier"))
}
const SUMMARY: &str = "SELECT c.id,c.kind,c.title,c.created_at,c.updated_at,m.conversation_id,m.user_id,m.role,m.joined_at,m.last_read_message_id,CASE WHEN c.kind='group' THEN c.title ELSE convert_to(COALESCE((SELECT u.username FROM conversation_members peer JOIN users u ON u.id=peer.user_id WHERE peer.conversation_id=c.id AND peer.user_id<>m.user_id LIMIT 1),(SELECT username FROM users WHERE id=m.user_id)),'UTF8') END AS resolved_title,(SELECT COUNT(*) FROM messages unread WHERE unread.conversation_id=c.id AND unread.sender_id<>m.user_id AND unread.deleted_at IS NULL AND (m.last_read_message_id IS NULL OR unread.id>m.last_read_message_id)) AS unread_count,(SELECT latest.id FROM messages latest WHERE latest.conversation_id=c.id ORDER BY latest.id DESC LIMIT 1) AS last_message_id,GREATEST(c.updated_at,COALESCE((SELECT latest.created_at FROM messages latest WHERE latest.conversation_id=c.id ORDER BY latest.id DESC LIMIT 1),c.updated_at)) AS activity_at FROM conversation_members m JOIN conversations c ON c.id=m.conversation_id WHERE m.user_id=";
async fn access(
    c: &mut PgConnection,
    actor: UserId,
    conv: ConversationId,
) -> Result<Option<bool>, QueryStoreError> {
    let r=sqlx::query("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=$1 AND user_id=$2) AS member FROM conversations WHERE id=$1").bind(conv.as_uuid()).bind(actor.as_uuid()).fetch_optional(c).await.map_err(db)?;
    r.map(|r| r.try_get::<bool, _>("member").map_err(db))
        .transpose()
}
async fn summaries(
    c: &mut PgConnection,
    rows: Vec<sqlx::postgres::PgRow>,
) -> Result<Vec<ConversationSummary>, QueryStoreError> {
    let ids = rows
        .iter()
        .map(|r| r.try_get::<Option<Uuid>, _>("last_message_id").map_err(db))
        .collect::<Result<Vec<_>, _>>()?;
    let mut messages = HashMap::new();
    if ids.iter().any(Option::is_some) {
        let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new(super::messages::VIEW);
        q.push(" WHERE msg.id IN (");
        let mut s = q.separated(",");
        for id in ids.iter().flatten() {
            s.push_bind(id);
        }
        s.push_unseparated(")");
        for row in q.build().fetch_all(&mut *c).await.map_err(db)? {
            let view = super::messages::read_view(&row)
                .map_err(|e| QueryStoreError::Unexpected(Box::new(e)))?;
            messages.insert(view.id(), view);
        }
    }
    rows.iter()
        .map(|r| {
            let last_id: Option<MessageId> = r
                .try_get::<Option<Uuid>, _>("last_message_id")
                .map_err(db)?
                .map(uid)
                .transpose()?;
            let last_message = last_id
                .map(|id| {
                    messages
                        .remove(&id)
                        .ok_or_else(|| bad("missing projected last message"))
                })
                .transpose()?;
            let conversation = super::conversations::read_conversation(r)
                .map_err(|e| QueryStoreError::Unexpected(Box::new(e)))?;
            let membership = super::conversations::read_member(r)
                .map_err(|e| QueryStoreError::Unexpected(Box::new(e)))?;
            let unread: i64 = r.try_get("unread_count").map_err(db)?;
            let unread_count = u64::try_from(unread).map_err(|_| bad("invalid unread count"))?;
            let activity_at = date(r.try_get("activity_at").map_err(db)?).map_err(bad)?;
            Ok(ConversationSummary {
                conversation,
                membership,
                title: String::from_utf8(r.try_get("resolved_title").map_err(db)?)
                    .map_err(|_| bad("invalid UTF-8 projected title"))?,
                unread_count,
                last_message,
                activity_at,
            })
        })
        .collect()
}
fn parse_username(v: String) -> Result<Username, QueryStoreError> {
    v.parse().map_err(|_| bad("invalid projected username"))
}
impl MessagingQueryStore for PostgresStorage {
    async fn get_conversation_member(
        &self,
        actor: UserId,
        conversation: ConversationId,
        target: UserId,
    ) -> Result<Outcome<Option<ConversationMemberSummary>>, QueryStoreError> {
        let mut tx = self.read_snapshot().await.map_err(db)?;
        match access(&mut tx, actor, conversation).await? {
            None => return Ok(Outcome::ConversationNotFound),
            Some(false) => return Ok(Outcome::NotMember),
            Some(true) => (),
        }
        let r=sqlx::query("SELECT m.conversation_id,m.user_id,m.role,m.joined_at,m.last_read_message_id,u.username FROM conversation_members m JOIN users u ON u.id=m.user_id WHERE m.conversation_id=$1 AND m.user_id=$2").bind(conversation.as_uuid()).bind(target.as_uuid()).fetch_optional(&mut *tx).await.map_err(db)?;
        let member = r
            .map(|r| {
                let m = super::conversations::read_member(&r)
                    .map_err(|e| QueryStoreError::Unexpected(Box::new(e)))?;
                Ok(ConversationMemberSummary {
                    user: UserSummary {
                        id: target,
                        username: parse_username(r.try_get("username").map_err(db)?)?,
                    },
                    role: m.role(),
                    joined_at: m.joined_at(),
                })
            })
            .transpose()?;
        tx.commit().await.map_err(db)?;
        Ok(Outcome::Found(member))
    }
    async fn find_users(&self, username: &Username) -> Result<Vec<UserSummary>, QueryStoreError> {
        let rows =
            sqlx::query("SELECT id,username FROM users WHERE username=$1 AND disabled_at IS NULL")
                .bind(username.as_str())
                .fetch_all(&self.pool)
                .await
                .map_err(db)?;
        rows.into_iter()
            .map(|r| {
                Ok(UserSummary {
                    id: uid(r.try_get("id").map_err(db)?)?,
                    username: parse_username(r.try_get("username").map_err(db)?)?,
                })
            })
            .collect()
    }
    async fn list_conversations(
        &self,
        actor: UserId,
        before: Option<ConversationBoundary>,
        limit: PageLimit,
    ) -> Result<QuerySlice<ConversationSummary>, QueryStoreError> {
        let mut tx = self.read_snapshot().await.map_err(db)?;
        let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT * FROM (");
        q.push(SUMMARY).push_bind(actor.as_uuid()).push(") inbox");
        if let Some(b) = before {
            let at = micros(b.activity_at).map_err(bad)?;
            if b.activity_at.unix_timestamp_nanos().rem_euclid(1000) != 0 {
                q.push(" WHERE activity_at <= ").push_bind(at);
            } else {
                q.push(" WHERE (activity_at,id)<(")
                    .push_bind(at)
                    .push(",")
                    .push_bind(b.conversation_id.as_uuid())
                    .push(")");
            }
        }
        let n = i64::try_from(limit.get() + 1).map_err(|_| bad("page limit out of range"))?;
        q.push(" ORDER BY activity_at DESC,id DESC LIMIT ")
            .push_bind(n);
        let mut rows = q.build().fetch_all(&mut *tx).await.map_err(db)?;
        let has_more = rows.len() > limit.get();
        rows.truncate(limit.get());
        let items = summaries(&mut tx, rows).await?;
        tx.commit().await.map_err(db)?;
        Ok(QuerySlice { items, has_more })
    }
    async fn get_conversation_summary(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> Result<Outcome<ConversationSummary>, QueryStoreError> {
        let mut tx = self.read_snapshot().await.map_err(db)?;
        match access(&mut tx, actor, conversation).await? {
            None => return Ok(Outcome::ConversationNotFound),
            Some(false) => return Ok(Outcome::NotMember),
            Some(true) => (),
        }
        let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new(SUMMARY);
        q.push_bind(actor.as_uuid())
            .push(" AND c.id=")
            .push_bind(conversation.as_uuid());
        let rows = q.build().fetch_all(&mut *tx).await.map_err(db)?;
        let item = summaries(&mut tx, rows)
            .await?
            .pop()
            .ok_or_else(|| bad("missing authorized conversation"))?;
        tx.commit().await.map_err(db)?;
        Ok(Outcome::Found(item))
    }
    async fn list_conversation_members(
        &self,
        actor: UserId,
        conversation: ConversationId,
        after: Option<MemberBoundary>,
        limit: PageLimit,
    ) -> Result<Outcome<QuerySlice<ConversationMemberSummary>>, QueryStoreError> {
        let mut tx = self.read_snapshot().await.map_err(db)?;
        match access(&mut tx, actor, conversation).await? {
            None => return Ok(Outcome::ConversationNotFound),
            Some(false) => return Ok(Outcome::NotMember),
            Some(true) => (),
        }
        let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new(
            "SELECT m.conversation_id,m.user_id,m.role,m.joined_at,m.last_read_message_id,c.kind,u.username FROM conversation_members m JOIN conversations c ON c.id=m.conversation_id JOIN users u ON u.id=m.user_id WHERE m.conversation_id=",
        );
        q.push_bind(conversation.as_uuid());
        if let Some(b) = after {
            let at = micros(b.joined_at).map_err(bad)?;
            if b.joined_at.unix_timestamp_nanos().rem_euclid(1000) != 0 {
                q.push(" AND m.joined_at>").push_bind(at);
            } else {
                q.push(" AND (m.joined_at,m.user_id)>(")
                    .push_bind(at)
                    .push(",")
                    .push_bind(b.user_id.as_uuid())
                    .push(")");
            }
        }
        let n = i64::try_from(limit.get() + 1).map_err(|_| bad("page limit out of range"))?;
        q.push(" ORDER BY m.joined_at ASC,m.user_id ASC LIMIT ")
            .push_bind(n);
        let rows = q.build().fetch_all(&mut *tx).await.map_err(db)?;
        let has_more = rows.len() > limit.get();
        let items = rows
            .into_iter()
            .take(limit.get())
            .map(|r| {
                let m = super::conversations::read_member(&r)
                    .map_err(|e| QueryStoreError::Unexpected(Box::new(e)))?;
                Ok(ConversationMemberSummary {
                    user: UserSummary {
                        id: m.user_id(),
                        username: parse_username(r.try_get("username").map_err(db)?)?,
                    },
                    role: m.role(),
                    joined_at: m.joined_at(),
                })
            })
            .collect::<Result<Vec<_>, QueryStoreError>>()?;
        tx.commit().await.map_err(db)?;
        Ok(Outcome::Found(QuerySlice { items, has_more }))
    }
}
