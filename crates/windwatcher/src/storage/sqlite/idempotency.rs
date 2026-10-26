use sqlx::{Row, SqliteConnection};
use time::{Duration, OffsetDateTime};

use super::{Corrupt, SqliteStorage, id_bytes, to_micros, unavailable};
use crate::{
    ConversationCreateCommand, ConversationId, ConversationKind, ConversationStoreError,
    CreateCommandError as Error, CreateCommandStore, CreateDisposition as Disposition,
    CreateResult, IdempotencyKey, Message, MessageId, MessageView, QueryStoreError,
    SendMessageStoreOutcome, StartDirectStoreOutcome, UserId, message_request_fingerprint,
};

fn failure(error: sqlx::Error) -> Error {
    Error::Storage(if unavailable(&error) {
        QueryStoreError::Unavailable(Box::new(error))
    } else {
        QueryStoreError::Unexpected(Box::new(error))
    })
}
fn corrupt(message: &'static str) -> Error {
    Error::Storage(QueryStoreError::Unexpected(Box::new(Corrupt(message))))
}
fn conversation_error(error: ConversationStoreError) -> Error {
    match error {
        ConversationStoreError::ParticipantUnavailable(_) => Error::ParticipantUnavailable,
        error => Error::Conversation(error),
    }
}

struct Record {
    actor: UserId,
    operation: &'static str,
    scope: Vec<u8>,
    key_hash: [u8; 32],
    request_hash: [u8; 32],
    now: OffsetDateTime,
}

impl Record {
    async fn lookup(&self, connection: &mut SqliteConnection) -> Result<Option<Vec<u8>>, Error> {
        let now = to_micros(self.now).map_err(corrupt)?;
        sqlx::query("DELETE FROM idempotency_records WHERE expires_at <= ?")
            .bind(now)
            .execute(&mut *connection)
            .await
            .map_err(failure)?;
        let row = sqlx::query("SELECT request_hash, resource_id FROM idempotency_records WHERE actor_id = ? AND operation = ? AND scope_id = ? AND key_hash = ? AND expires_at > ?")
            .bind(id_bytes(self.actor)).bind(self.operation).bind(&self.scope).bind(self.key_hash.as_slice()).bind(now).fetch_optional(connection).await.map_err(failure)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let hash: Vec<u8> = row.try_get("request_hash").map_err(failure)?;
        if hash != self.request_hash {
            return Err(Error::KeyReused);
        }
        Ok(Some(row.try_get("resource_id").map_err(failure)?))
    }

    async fn insert(
        &self,
        connection: &mut SqliteConnection,
        resource: Vec<u8>,
    ) -> Result<(), Error> {
        let expires = self
            .now
            .checked_add(Duration::days(1))
            .ok_or_else(|| corrupt("idempotency expiry overflow"))?;
        sqlx::query("INSERT INTO idempotency_records (actor_id, operation, scope_id, key_hash, request_hash, resource_id, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(id_bytes(self.actor)).bind(self.operation).bind(&self.scope).bind(self.key_hash.as_slice()).bind(self.request_hash.as_slice()).bind(resource)
            .bind(to_micros(self.now).map_err(corrupt)?).bind(to_micros(expires).map_err(corrupt)?)
            .execute(connection).await.map_err(failure)?;
        Ok(())
    }
}

async fn is_member(
    connection: &mut SqliteConnection,
    actor: UserId,
    conversation: ConversationId,
) -> Result<bool, Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id = ? AND user_id = ?)")
        .bind(id_bytes(conversation)).bind(id_bytes(actor)).fetch_one(connection).await.map_err(failure)
}

impl CreateCommandStore for SqliteStorage {
    async fn create_conversation_command(
        &self,
        command: &ConversationCreateCommand,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<CreateResult<ConversationId>, Error> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(failure)?;
        let record = key.map(|key| Record {
            actor: command.actor,
            operation: "create_conversation_v1",
            scope: id_bytes(command.actor),
            key_hash: key.hash(),
            request_hash: command.fingerprint(),
            now,
        });
        if let Some(record) = &record
            && let Some(resource) = record.lookup(&mut tx).await?
        {
            let id = uuid::Uuid::from_slice(&resource)
                .ok()
                .and_then(|id| ConversationId::try_from(id).ok())
                .ok_or_else(|| corrupt("invalid idempotent conversation ID"))?;
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?)")
                    .bind(&resource)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(failure)?;
            if !exists {
                return Err(Error::ResultUnavailable);
            }
            if !is_member(&mut tx, command.actor, id).await? {
                return Err(Error::NotMember);
            }
            tx.commit().await.map_err(failure)?;
            return Ok(CreateResult {
                value: id,
                disposition: Disposition::Replayed,
            });
        }
        let (id, disposition) = match command.conversation.kind() {
            ConversationKind::Direct => {
                let [peer] = command.participants.as_slice() else {
                    return Err(corrupt("Direct command needs exactly one peer"));
                };
                match Self::create_direct_in(&mut tx, &command.conversation, command.actor, *peer)
                    .await
                    .map_err(conversation_error)?
                {
                    StartDirectStoreOutcome::Created(c) => (c.id(), Disposition::Created),
                    StartDirectStoreOutcome::Existing(c) => (c.id(), Disposition::Existing),
                }
            }
            ConversationKind::Group => {
                Self::create_group_in(
                    &mut tx,
                    &command.conversation,
                    command.actor,
                    &command.participants,
                )
                .await
                .map_err(conversation_error)?;
                (command.conversation.id(), Disposition::Created)
            }
        };
        if let Some(record) = record {
            record.insert(&mut tx, id_bytes(id)).await?;
        }
        tx.commit().await.map_err(failure)?;
        Ok(CreateResult {
            value: id,
            disposition,
        })
    }

    async fn send_message_command(
        &self,
        message: &Message,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<CreateResult<MessageView>, Error> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(failure)?;
        let record = key.map(|key| Record {
            actor: message.sender_id(),
            operation: "send_message_v1",
            scope: id_bytes(message.conversation_id()),
            key_hash: key.hash(),
            request_hash: message_request_fingerprint(message),
            now,
        });
        if let Some(record) = &record
            && let Some(resource) = record.lookup(&mut tx).await?
        {
            let id = uuid::Uuid::from_slice(&resource)
                .ok()
                .and_then(|id| MessageId::try_from(id).ok())
                .ok_or_else(|| corrupt("invalid idempotent message ID"))?;
            let view = super::messages::find_global(&mut tx, id)
                .await
                .map_err(Error::Message)?
                .ok_or(Error::ResultUnavailable)?;
            if view.conversation_id() != message.conversation_id() {
                return Err(corrupt("idempotent message scope mismatch"));
            }
            if !is_member(&mut tx, message.sender_id(), view.conversation_id()).await? {
                return Err(Error::NotMember);
            }
            tx.commit().await.map_err(failure)?;
            return Ok(CreateResult {
                value: view,
                disposition: Disposition::Replayed,
            });
        }
        let view = match super::messages::send_in(&mut tx, message)
            .await
            .map_err(Error::Message)?
        {
            SendMessageStoreOutcome::Sent(view) => view,
            SendMessageStoreOutcome::NotMember => return Err(Error::NotMember),
            SendMessageStoreOutcome::ConversationNotFound => {
                return Err(Error::ConversationNotFound);
            }
            SendMessageStoreOutcome::ReplyTargetNotFound
            | SendMessageStoreOutcome::ReplyTargetDeleted => {
                return Err(Error::ReplyTargetUnavailable);
            }
        };
        if let Some(record) = record {
            record.insert(&mut tx, id_bytes(view.id())).await?;
        }
        tx.commit().await.map_err(failure)?;
        Ok(CreateResult {
            value: view,
            disposition: Disposition::Created,
        })
    }
}
