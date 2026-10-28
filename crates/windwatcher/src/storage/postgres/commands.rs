use super::{PostgresStorage, micros, transient};
use crate::{
    ConversationCreateCommand, ConversationId, ConversationKind, CreateCommandError as Error,
    CreateCommandStore, CreateDisposition as Disposition, CreateResult, IdempotencyKey, Message,
    MessageContent, MessageId, MessageStoreError, MessageView, QueryStoreError,
    StartDirectStoreOutcome, UserId, message_request_fingerprint,
};
use sqlx::{PgConnection, Row};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

fn qerr(e: sqlx::Error) -> Error {
    Error::Storage(if transient(&e) {
        QueryStoreError::Unavailable(Box::new(e))
    } else {
        QueryStoreError::Unexpected(Box::new(e))
    })
}
fn corrupt(s: &'static str) -> Error {
    Error::Storage(QueryStoreError::Unexpected(Box::new(
        std::io::Error::other(s),
    )))
}
fn conv_err(e: crate::ConversationStoreError) -> Error {
    match e {
        crate::ConversationStoreError::ParticipantUnavailable(_) => Error::ParticipantUnavailable,
        e => Error::Conversation(e),
    }
}
fn msg_err(e: MessageStoreError) -> Error {
    Error::Message(e)
}
struct Record {
    actor: UserId,
    operation: &'static str,
    scope: Uuid,
    key_hash: [u8; 32],
    request_hash: [u8; 32],
    now: OffsetDateTime,
}
impl Record {
    async fn lookup(&self, c: &mut PgConnection) -> Result<Option<Uuid>, Error> {
        let now = micros(self.now).map_err(corrupt)?;
        sqlx::query("DELETE FROM idempotency_records WHERE expires_at<=$1")
            .bind(now)
            .execute(&mut *c)
            .await
            .map_err(qerr)?;
        let row=sqlx::query("SELECT request_hash,resource_id FROM idempotency_records WHERE actor_id=$1 AND operation=$2 AND scope_id=$3 AND key_hash=$4 AND expires_at>$5").bind(self.actor.as_uuid()).bind(self.operation).bind(self.scope).bind(self.key_hash.as_slice()).bind(now).fetch_optional(&mut *c).await.map_err(qerr)?;
        let Some(row) = row else { return Ok(None) };
        let hash: Vec<u8> = row.try_get("request_hash").map_err(qerr)?;
        if hash != self.request_hash {
            return Err(Error::KeyReused);
        }
        Ok(Some(row.try_get("resource_id").map_err(qerr)?))
    }
    async fn insert(&self, c: &mut PgConnection, resource: Uuid) -> Result<(), Error> {
        let expiry = self
            .now
            .checked_add(Duration::days(1))
            .ok_or_else(|| corrupt("idempotency expiry overflow"))?;
        sqlx::query("INSERT INTO idempotency_records(actor_id,operation,scope_id,key_hash,request_hash,resource_id,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(self.actor.as_uuid()).bind(self.operation).bind(self.scope).bind(self.key_hash.as_slice()).bind(self.request_hash.as_slice()).bind(resource).bind(micros(self.now).map_err(corrupt)?).bind(micros(expiry).map_err(corrupt)?).execute(c).await.map_err(qerr)?;
        Ok(())
    }
}
fn record(
    actor: UserId,
    operation: &'static str,
    scope: Uuid,
    key: Option<&IdempotencyKey>,
    hash: [u8; 32],
    now: OffsetDateTime,
) -> Option<Record> {
    key.map(|key| Record {
        actor,
        operation,
        scope,
        key_hash: key.hash(),
        request_hash: hash,
        now,
    })
}
async fn member(c: &mut PgConnection, actor: UserId, conv: ConversationId) -> Result<bool, Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=$1 AND user_id=$2)",
    )
    .bind(conv.as_uuid())
    .bind(actor.as_uuid())
    .fetch_one(c)
    .await
    .map_err(qerr)
}
async fn send_in(c: &mut PgConnection, m: &Message) -> Result<MessageView, Error> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=$1 FOR UPDATE)",
    )
    .bind(m.conversation_id().as_uuid())
    .fetch_one(&mut *c)
    .await
    .map_err(qerr)?;
    if !exists {
        return Err(Error::ConversationNotFound);
    }
    if !member(c, m.sender_id(), m.conversation_id()).await? {
        return Err(Error::NotMember);
    }
    if let Some(reply) = m.reply_to_message_id() {
        let deleted: Option<Option<i64>> = sqlx::query_scalar(
            "SELECT deleted_at FROM messages WHERE conversation_id=$1 AND id=$2",
        )
        .bind(m.conversation_id().as_uuid())
        .bind(reply.as_uuid())
        .fetch_optional(&mut *c)
        .await
        .map_err(qerr)?;
        match deleted {
            None => return Err(Error::ReplyTargetUnavailable),
            Some(Some(_)) => return Err(Error::ReplyTargetUnavailable),
            Some(None) => (),
        }
    }
    sqlx::query("INSERT INTO messages(id,conversation_id,sender_id,content,reply_to_message_id,created_at,edited_at,deleted_at) VALUES($1,$2,$3,$4,$5,$6,NULL,NULL)").bind(m.id().as_uuid()).bind(m.conversation_id().as_uuid()).bind(m.sender_id().as_uuid()).bind(m.content().map(MessageContent::as_str)).bind(m.reply_to_message_id().map(|v|v.as_uuid())).bind(micros(m.created_at()).map_err(corrupt)?).execute(&mut *c).await.map_err(|e|msg_err(if transient(&e){MessageStoreError::Unavailable(Box::new(e))}else{MessageStoreError::Conflict}))?;
    super::messages::by_id(c, m.id())
        .await
        .map_err(msg_err)?
        .ok_or_else(|| corrupt("inserted message disappeared"))
}
impl CreateCommandStore for PostgresStorage {
    async fn create_conversation_command(
        &self,
        command: &ConversationCreateCommand,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<CreateResult<ConversationId>, Error> {
        let mut tx = self.pool.begin().await.map_err(qerr)?;
        let rec = record(
            command.actor,
            "create_conversation_v1",
            command.actor.as_uuid(),
            key,
            command.fingerprint(),
            now,
        );
        if let Some(rec) = &rec
            && let Some(resource) = rec.lookup(&mut tx).await?
        {
            let id = ConversationId::try_from(resource)
                .map_err(|_| corrupt("invalid idempotent conversation ID"))?;
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversations WHERE id=$1)")
                    .bind(resource)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(qerr)?;
            if !exists {
                return Err(Error::ResultUnavailable);
            }
            if !member(&mut tx, command.actor, id).await? {
                return Err(Error::NotMember);
            }
            tx.commit().await.map_err(qerr)?;
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
                match super::conversations::create_direct(
                    &mut tx,
                    &command.conversation,
                    command.actor,
                    *peer,
                )
                .await
                .map_err(conv_err)?
                {
                    StartDirectStoreOutcome::Created(c) => (c.id(), Disposition::Created),
                    StartDirectStoreOutcome::Existing(c) => (c.id(), Disposition::Existing),
                }
            }
            ConversationKind::Group => {
                super::conversations::create_group(
                    &mut tx,
                    &command.conversation,
                    command.actor,
                    &command.participants,
                )
                .await
                .map_err(conv_err)?;
                (command.conversation.id(), Disposition::Created)
            }
        };
        if let Some(rec) = rec {
            rec.insert(&mut tx, id.as_uuid()).await?
        }
        tx.commit().await.map_err(qerr)?;
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
        let mut tx = self.pool.begin().await.map_err(qerr)?;
        let rec = record(
            message.sender_id(),
            "send_message_v1",
            message.conversation_id().as_uuid(),
            key,
            message_request_fingerprint(message),
            now,
        );
        if let Some(rec) = &rec
            && let Some(resource) = rec.lookup(&mut tx).await?
        {
            let id = MessageId::try_from(resource)
                .map_err(|_| corrupt("invalid idempotent message ID"))?;
            let view = super::messages::by_id(&mut tx, id)
                .await
                .map_err(msg_err)?
                .ok_or(Error::ResultUnavailable)?;
            if view.conversation_id() != message.conversation_id() {
                return Err(corrupt("idempotent message scope mismatch"));
            }
            if !member(&mut tx, message.sender_id(), view.conversation_id()).await? {
                return Err(Error::NotMember);
            }
            tx.commit().await.map_err(qerr)?;
            return Ok(CreateResult {
                value: view,
                disposition: Disposition::Replayed,
            });
        }
        let view = send_in(&mut tx, message).await?;
        if let Some(rec) = rec {
            rec.insert(&mut tx, view.id().as_uuid()).await?
        }
        tx.commit().await.map_err(qerr)?;
        Ok(CreateResult {
            value: view,
            disposition: Disposition::Created,
        })
    }
}
