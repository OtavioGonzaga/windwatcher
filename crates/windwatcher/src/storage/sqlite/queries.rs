use std::collections::HashMap;

use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};

use super::{
    Corrupt, SqliteStorage, from_micros, id_bytes, read_conversation, read_member, to_micros,
    unavailable,
};
use crate::{
    ConversationBoundary, ConversationId, ConversationMemberSummary,
    ConversationQueryOutcome as Outcome, ConversationSummary, MemberBoundary, MessageId,
    MessagingQueryStore, PageLimit, QuerySlice, QueryStoreError, UserId, UserSummary, Username,
};

fn failure(error: sqlx::Error) -> QueryStoreError {
    if unavailable(&error) {
        QueryStoreError::Unavailable(Box::new(error))
    } else {
        QueryStoreError::Unexpected(Box::new(error))
    }
}
fn corrupt(message: &'static str) -> QueryStoreError {
    QueryStoreError::Unexpected(Box::new(Corrupt(message)))
}
fn identifier<T: TryFrom<uuid::Uuid>>(bytes: Vec<u8>) -> Result<T, QueryStoreError> {
    uuid::Uuid::from_slice(&bytes)
        .ok()
        .and_then(|id| T::try_from(id).ok())
        .ok_or_else(|| corrupt("invalid query identifier"))
}

const SUMMARY: &str = "SELECT c.id, c.kind, c.title, c.created_at, c.updated_at, m.conversation_id, m.user_id, m.role, m.joined_at, m.last_read_message_id,
    CASE WHEN c.kind = 'group' THEN c.title ELSE
        COALESCE((SELECT u.username FROM conversation_members peer JOIN users u ON u.id = peer.user_id WHERE peer.conversation_id = c.id AND peer.user_id <> m.user_id LIMIT 1), (SELECT username FROM users WHERE id = m.user_id)) END AS resolved_title,
    (SELECT COUNT(*) FROM messages unread WHERE unread.conversation_id = c.id AND unread.sender_id <> m.user_id AND unread.deleted_at IS NULL AND (m.last_read_message_id IS NULL OR unread.id > m.last_read_message_id)) AS unread_count,
    (SELECT MAX(latest.id) FROM messages latest WHERE latest.conversation_id = c.id) AS last_message_id,
    MAX(c.updated_at, COALESCE((SELECT MAX(activity.created_at) FROM messages activity WHERE activity.conversation_id = c.id), c.updated_at)) AS activity_at
    FROM conversation_members m JOIN conversations c ON c.id = m.conversation_id WHERE m.user_id = ";

async fn access(
    connection: &mut SqliteConnection,
    actor: UserId,
    conversation: ConversationId,
) -> Result<Option<bool>, QueryStoreError> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members m WHERE m.conversation_id = c.id AND m.user_id = ?) FROM conversations c WHERE c.id = ?")
        .bind(id_bytes(actor)).bind(id_bytes(conversation)).fetch_optional(connection).await.map_err(failure)
}

async fn summaries(
    connection: &mut SqliteConnection,
    rows: Vec<SqliteRow>,
) -> Result<Vec<ConversationSummary>, QueryStoreError> {
    let ids = rows
        .iter()
        .filter_map(|r| {
            r.try_get::<Option<Vec<u8>>, _>("last_message_id")
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(failure)?;
    let mut messages = HashMap::new();
    if !ids.is_empty() {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(super::messages::VIEW_SELECT);
        query.push(" WHERE msg.id IN (");
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        for row in query
            .build()
            .fetch_all(&mut *connection)
            .await
            .map_err(failure)?
        {
            let view = super::messages::read_view(&row)
                .map_err(|e| QueryStoreError::Unexpected(Box::new(e)))?;
            messages.insert(view.id(), view);
        }
    }
    rows.iter()
        .map(|row| {
            let last_id: Option<MessageId> = row
                .try_get::<Option<Vec<u8>>, _>("last_message_id")
                .map_err(failure)?
                .map(identifier)
                .transpose()?;
            let last_message = last_id
                .map(|id| {
                    messages
                        .remove(&id)
                        .ok_or_else(|| corrupt("missing projected last message"))
                })
                .transpose()?;
            Ok(ConversationSummary {
                conversation: read_conversation(row)
                    .map_err(|e| QueryStoreError::Unexpected(Box::new(e)))?,
                membership: read_member(row)
                    .map_err(|e| QueryStoreError::Unexpected(Box::new(e)))?,
                title: row.try_get("resolved_title").map_err(failure)?,
                unread_count: u64::try_from(
                    row.try_get::<i64, _>("unread_count").map_err(failure)?,
                )
                .map_err(|_| corrupt("invalid unread count"))?,
                activity_at: from_micros(row.try_get("activity_at").map_err(failure)?)
                    .map_err(corrupt)?,
                last_message,
            })
        })
        .collect()
}

impl MessagingQueryStore for SqliteStorage {
    async fn find_users(&self, username: &Username) -> Result<Vec<UserSummary>, QueryStoreError> {
        sqlx::query("SELECT id, username FROM users WHERE username = ? AND disabled_at IS NULL")
            .bind(username.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(failure)?
            .iter()
            .map(|row| {
                Ok(UserSummary {
                    id: identifier(row.try_get("id").map_err(failure)?)?,
                    username: row
                        .try_get::<String, _>("username")
                        .map_err(failure)?
                        .parse()
                        .map_err(|_| corrupt("invalid projected username"))?,
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
        let mut tx = self.pool.begin().await.map_err(failure)?;
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT * FROM (");
        query
            .push(SUMMARY)
            .push_bind(id_bytes(actor))
            .push(") inbox");
        if let Some(boundary) = before {
            // Stored timestamps use microseconds. Preserve the exact comparison:
            // a sub-microsecond boundary lies strictly after its floored value.
            let at = to_micros(boundary.activity_at).map_err(corrupt)?;
            if boundary
                .activity_at
                .unix_timestamp_nanos()
                .rem_euclid(1_000)
                != 0
            {
                query.push(" WHERE activity_at <= ").push_bind(at);
            } else {
                query
                    .push(" WHERE (activity_at, id) < (")
                    .push_bind(at)
                    .push(", ")
                    .push_bind(id_bytes(boundary.conversation_id))
                    .push(")");
            }
        }
        query
            .push(" ORDER BY activity_at DESC, id DESC LIMIT ")
            .push_bind((limit.get() + 1) as i64);
        let mut rows = query.build().fetch_all(&mut *tx).await.map_err(failure)?;
        let has_more = rows.len() > limit.get();
        rows.truncate(limit.get());
        let items = summaries(&mut tx, rows).await?;
        tx.commit().await.map_err(failure)?;
        Ok(QuerySlice { items, has_more })
    }

    async fn get_conversation_summary(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> Result<Outcome<ConversationSummary>, QueryStoreError> {
        let mut tx = self.pool.begin().await.map_err(failure)?;
        match access(&mut tx, actor, conversation).await? {
            None => return Ok(Outcome::ConversationNotFound),
            Some(false) => return Ok(Outcome::NotMember),
            Some(true) => (),
        }
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(SUMMARY);
        query
            .push_bind(id_bytes(actor))
            .push(" AND c.id = ")
            .push_bind(id_bytes(conversation));
        let rows = query.build().fetch_all(&mut *tx).await.map_err(failure)?;
        let summary = summaries(&mut tx, rows)
            .await?
            .pop()
            .ok_or_else(|| corrupt("missing authorized conversation"))?;
        tx.commit().await.map_err(failure)?;
        Ok(Outcome::Found(summary))
    }

    async fn list_conversation_members(
        &self,
        actor: UserId,
        conversation: ConversationId,
        after: Option<MemberBoundary>,
        limit: PageLimit,
    ) -> Result<Outcome<QuerySlice<ConversationMemberSummary>>, QueryStoreError> {
        let mut tx = self.pool.begin().await.map_err(failure)?;
        match access(&mut tx, actor, conversation).await? {
            None => return Ok(Outcome::ConversationNotFound),
            Some(false) => return Ok(Outcome::NotMember),
            Some(true) => (),
        }
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT m.conversation_id, m.user_id, m.role, m.joined_at, m.last_read_message_id, c.kind, u.username FROM conversation_members m JOIN conversations c ON c.id = m.conversation_id JOIN users u ON u.id = m.user_id WHERE m.conversation_id = ",
        );
        query.push_bind(id_bytes(conversation));
        if let Some(boundary) = after {
            let at = to_micros(boundary.joined_at).map_err(corrupt)?;
            if boundary.joined_at.unix_timestamp_nanos().rem_euclid(1_000) != 0 {
                query.push(" AND m.joined_at > ").push_bind(at);
            } else {
                query
                    .push(" AND (m.joined_at, m.user_id) > (")
                    .push_bind(at)
                    .push(", ")
                    .push_bind(id_bytes(boundary.user_id))
                    .push(")");
            }
        }
        query
            .push(" ORDER BY m.joined_at ASC, m.user_id ASC LIMIT ")
            .push_bind((limit.get() + 1) as i64);
        let rows = query.build().fetch_all(&mut *tx).await.map_err(failure)?;
        let has_more = rows.len() > limit.get();
        let items = rows
            .iter()
            .take(limit.get())
            .map(|row| {
                let member =
                    read_member(row).map_err(|e| QueryStoreError::Unexpected(Box::new(e)))?;
                Ok(ConversationMemberSummary {
                    user: UserSummary {
                        id: member.user_id(),
                        username: row
                            .try_get::<String, _>("username")
                            .map_err(failure)?
                            .parse()
                            .map_err(|_| corrupt("invalid projected username"))?,
                    },
                    role: member.role(),
                    joined_at: member.joined_at(),
                })
            })
            .collect::<Result<Vec<_>, QueryStoreError>>()?;
        tx.commit().await.map_err(failure)?;
        Ok(Outcome::Found(QuerySlice { items, has_more }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn derived_activity_and_member_plans_use_existing_lookup_indexes() {
        let directory = tempfile::TempDir::new().unwrap();
        let store = SqliteStorage::connect(super::super::SqliteStorageOptions::new(format!(
            "sqlite://{}",
            directory.path().join("plans.db").display()
        )))
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("EXPLAIN QUERY PLAN ");
        query
            .push(SUMMARY)
            .push_bind(id_bytes(UserId::new()))
            .push(" ORDER BY activity_at DESC, c.id DESC LIMIT 51");
        let plan = query
            .build()
            .fetch_all(&store.pool)
            .await
            .unwrap()
            .iter()
            .map(|r| r.get::<String, _>("detail"))
            .collect::<Vec<_>>()
            .join("\n");
        // Derived activity intentionally needs sorting, but message probes must
        // use the existing conversation/ID index instead of scanning all history.
        assert!(plan.contains("SEARCH unread USING INDEX"), "{plan}");
        assert!(
            plan.contains("SEARCH latest USING COVERING INDEX"),
            "{plan}"
        );
        assert!(!plan.contains("SCAN unread"), "{plan}");
        let plan = sqlx::query("EXPLAIN QUERY PLAN SELECT user_id FROM conversation_members WHERE conversation_id = ? AND (joined_at, user_id) > (?, ?) ORDER BY joined_at, user_id LIMIT 51")
            .bind(id_bytes(ConversationId::new())).bind(0_i64).bind(id_bytes(UserId::new())).fetch_all(&store.pool).await.unwrap().iter().map(|r| r.get::<String, _>("detail")).collect::<Vec<_>>().join("\n");
        assert!(plan.contains("conversation_members_joined"), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }
}
