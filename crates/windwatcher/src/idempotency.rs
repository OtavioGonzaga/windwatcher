use std::future::Future;

use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::{
    Conversation, ConversationId, ConversationStoreError, DomainEvent, GroupTitle, GroupTitleError,
    Message, MessageContent, MessageContentError, MessageId, MessagePolicy, MessagePolicyError,
    MessageStoreError, MessageView, Outcome, QueryStoreError, UserId,
};

/// Validated exact client command key. Debug output deliberately redacts it.
#[derive(Clone, PartialEq, Eq)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Accepts 1..=128 visible ASCII bytes (0x21..=0x7e), without trimming.
    pub fn new(value: impl Into<String>) -> Result<Self, IdempotencyKeyError> {
        let value = value.into();
        if !(1..=128).contains(&value.len()) || !value.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err(IdempotencyKeyError);
        }
        Ok(Self(value))
    }
    /// Cryptographic persistence identity; adapters must never persist raw keys.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_bytes()).into()
    }
}

impl std::fmt::Debug for IdempotencyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IdempotencyKey([REDACTED])")
    }
}

/// A key failed its exact byte-length/character contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid idempotency key")]
pub struct IdempotencyKeyError;

/// Successful command disposition; first-time existing Directs are not replays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateDisposition {
    /// Newly persisted resource.
    Created,
    /// Semantic Direct existed before the first use of this key.
    Existing,
    /// Matching unexpired command identity, without another create side effect.
    Replayed,
}

/// Current resource state and command disposition, never a historical wire replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateResult<T> {
    /// Original resource identity/current state.
    pub value: T,
    /// Whether creation, semantic reuse or replay occurred.
    pub disposition: CreateDisposition,
}

/// Application conversation creation input, independent of transport JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationCreateRequest {
    /// Open the unordered actor/participant pair.
    Direct {
        /// Peer, which may be the actor.
        participant_id: UserId,
    },
    /// Create a mutable Group.
    Group {
        /// Exact title, validated by the use case.
        title: String,
        /// Initial set; actor and duplicates are removed before persistence.
        participant_ids: Vec<UserId>,
    },
}

/// Canonical application command supplied to atomic persistence.
#[derive(Clone, Debug)]
pub struct ConversationCreateCommand {
    /// New candidate metadata/identity; an existing Direct can replace the ID.
    pub conversation: Conversation,
    /// Owner/current actor and idempotency scope.
    pub actor: UserId,
    /// Direct: exactly one peer. Group: sorted distinct non-owner members.
    pub participants: Vec<UserId>,
}

impl ConversationCreateCommand {
    /// Stable versioned semantic fingerprint, excluding generated IDs/timestamps.
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"windwatcher:create_conversation:v1\0");
        if let Some(title) = self.conversation.title() {
            hash.update(b"group\0");
            hash.update((title.as_str().len() as u64).to_be_bytes());
            hash.update(title.as_str().as_bytes());
            let mut participants: Vec<_> = self
                .participants
                .iter()
                .copied()
                .filter(|id| *id != self.actor)
                .collect();
            participants.sort_unstable();
            participants.dedup();
            for id in participants {
                hash.update(id.as_uuid().as_bytes());
            }
        } else {
            hash.update(b"direct\0");
            for id in &self.participants {
                hash.update(id.as_uuid().as_bytes());
            }
        }
        hash.finalize().into()
    }
}

/// Stable exact message fingerprint, excluding generated IDs and timestamps.
pub fn message_request_fingerprint(message: &Message) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"windwatcher:send_message:v1\0");
    let content = message
        .content()
        .map(MessageContent::as_str)
        .unwrap_or_default();
    hash.update((content.len() as u64).to_be_bytes());
    hash.update(content.as_bytes());
    match message.reply_to_message_id() {
        None => hash.update([0]),
        Some(id) => {
            hash.update([1]);
            hash.update(id.as_uuid().as_bytes());
        }
    }
    hash.finalize().into()
}

/// Semantic command failures, without transport status codes or supplied keys.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CreateCommandError {
    /// Valid unexpired key was used with different semantic input.
    #[error("idempotency key reused")]
    KeyReused,
    /// Original resource was physically deleted; no replacement may be created.
    #[error("idempotent result unavailable")]
    ResultUnavailable,
    /// Actor is no longer a current member.
    #[error("actor is not a conversation member")]
    NotMember,
    /// Message parent does not exist.
    #[error("conversation not found")]
    ConversationNotFound,
    /// Reply target is missing, cross-conversation or deleted.
    #[error("reply target unavailable")]
    ReplyTargetUnavailable,
    /// New membership target is missing or disabled.
    #[error("conversation participant unavailable")]
    ParticipantUnavailable,
    /// Group title is invalid.
    #[error("invalid group title")]
    InvalidTitle(#[source] GroupTitleError),
    /// Initial participant input exceeds the fixed limit, before canonicalization.
    #[error("initial group participants exceed 256 entries")]
    TooManyParticipants,
    /// Exact message content is not portable.
    #[error("invalid message content")]
    InvalidContent(#[source] MessageContentError),
    /// Current new-write policy rejects content.
    #[error("message exceeds write policy")]
    ContentTooLarge(#[source] MessagePolicyError),
    /// Conversation persistence failed.
    #[error("conversation storage failed")]
    Conversation(#[source] ConversationStoreError),
    /// Message persistence failed.
    #[error("message storage failed")]
    Message(#[source] MessageStoreError),
    /// Command record persistence failed.
    #[error("command storage failed")]
    Storage(#[from] QueryStoreError),
}

/// Atomic creation and optional persisted command deduplication.
///
/// Active records have `expires_at > now`, with a 24-hour guarantee. Resource and
/// record commit together. Failures do not reserve keys. Replay returns current
/// authorized state and never creates a replacement for a deleted original.
pub trait CreateCommandStore: Sync {
    /// Scope: actor + create_conversation_v1 + actor + key hash.
    fn create_conversation_command(
        &self,
        command: &ConversationCreateCommand,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<CreateResult<ConversationId>, CreateCommandError>> + Send;
    /// Scope: sender + send_message_v1 + conversation + key hash. Same-transaction
    /// authorization, reply checks and enriched current-state loading are required.
    fn send_message_command(
        &self,
        message: &Message,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<CreateResult<MessageView>, CreateCommandError>> + Send;
}

/// Creates/opens a conversation with optional durable retry deduplication.
pub struct CreateConversation<'a, S>(&'a S);
impl<'a, S: CreateCommandStore> CreateConversation<'a, S> {
    /// Uses semantic atomic command persistence.
    pub fn new(store: &'a S) -> Self {
        Self(store)
    }
    /// Canonicalizes Group participant sets before fingerprinting/persistence.
    /// Group input is limited to [`crate::MAX_INITIAL_GROUP_PARTICIPANTS`] entries
    /// before canonicalization and [`crate::MAX_GROUP_TITLE_BYTES`] UTF-8 title bytes,
    /// with or without an idempotency key. Rejections never reserve keys.
    pub async fn execute(
        &self,
        actor: UserId,
        request: ConversationCreateRequest,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<Outcome<CreateResult<ConversationId>>, CreateCommandError> {
        let (conversation, mut participants) = match request {
            ConversationCreateRequest::Direct { participant_id } => (
                Conversation::new_direct(ConversationId::new(), now),
                vec![participant_id],
            ),
            ConversationCreateRequest::Group {
                title,
                participant_ids,
            } => {
                if participant_ids.len() > crate::MAX_INITIAL_GROUP_PARTICIPANTS {
                    return Err(CreateCommandError::TooManyParticipants);
                }
                let title =
                    GroupTitle::for_write(title).map_err(CreateCommandError::InvalidTitle)?;
                (
                    Conversation::new_group(ConversationId::new(), title, now),
                    participant_ids
                        .into_iter()
                        .filter(|id| *id != actor)
                        .collect(),
                )
            }
        };
        if conversation.title().is_some() {
            participants.sort_unstable();
            participants.dedup();
        }
        let kind = conversation.kind();
        let result = self
            .0
            .create_conversation_command(
                &ConversationCreateCommand {
                    conversation,
                    actor,
                    participants,
                },
                key,
                now,
            )
            .await?;
        Ok(match result.disposition {
            CreateDisposition::Created => Outcome::with_event(
                result.clone(),
                DomainEvent::ConversationCreated {
                    conversation_id: result.value,
                    kind,
                    occurred_at: now,
                },
            ),
            CreateDisposition::Existing | CreateDisposition::Replayed => Outcome::new(result),
        })
    }
}

/// Synchronous message send with optional persisted retry deduplication.
pub struct SendMessageCommand<'a, S> {
    store: &'a S,
    policy: &'a MessagePolicy,
}
impl<'a, S: CreateCommandStore> SendMessageCommand<'a, S> {
    /// Uses resolved content policy and semantic command persistence.
    pub fn new(store: &'a S, policy: &'a MessagePolicy) -> Self {
        Self { store, policy }
    }
    /// Validates exact semantic input; never fingerprints raw transport encoding.
    pub async fn execute(
        &self,
        actor: UserId,
        conversation: ConversationId,
        content: impl Into<String>,
        reply: Option<MessageId>,
        key: Option<&IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<Outcome<CreateResult<MessageView>>, CreateCommandError> {
        let content = MessageContent::new(content).map_err(CreateCommandError::InvalidContent)?;
        self.policy
            .validate(&content)
            .map_err(CreateCommandError::ContentTooLarge)?;
        let message = Message::new(MessageId::new(), conversation, actor, content, reply, now)
            .map_err(|e| CreateCommandError::Message(MessageStoreError::Unexpected(Box::new(e))))?;
        let result = self.store.send_message_command(&message, key, now).await?;
        Ok(match result.disposition {
            CreateDisposition::Created => Outcome::with_event(
                result.clone(),
                DomainEvent::MessageSent {
                    message: result.value.clone(),
                },
            ),
            CreateDisposition::Existing | CreateDisposition::Replayed => Outcome::new(result),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_validation_and_redaction() {
        for key in [
            "",
            " ",
            "with space",
            "line\n",
            "é",
            "\u{7f}",
            &"a".repeat(129),
        ] {
            assert!(IdempotencyKey::new(key).is_err());
        }
        let key = IdempotencyKey::new("client-key").unwrap();
        assert_eq!(key.hash(), <[u8; 32]>::from(Sha256::digest(b"client-key")));
        assert!(!format!("{key:?}").contains("client-key"));
        assert!(IdempotencyKey::new("a".repeat(128)).is_ok());
    }
}
