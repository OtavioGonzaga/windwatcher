use time::OffsetDateTime;

use crate::{ConversationId, MessageId, UserId};

/// Portable text, preserved exactly, independently of write-size policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageContent(String);

/// Text cannot be represented portably.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MessageContentError {
    /// Embedded NUL is not portable across text backends.
    #[error("message content contains NUL")]
    ContainsNul,
}

impl MessageContent {
    /// Accepts exact text, including empty and whitespace-only text, except NUL.
    pub fn new(value: impl Into<String>) -> Result<Self, MessageContentError> {
        let value = value.into();
        if value.contains('\0') {
            return Err(MessageContentError::ContainsNul);
        }
        Ok(Self(value))
    }

    /// Returns the exact accepted text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Configurable acceptance policy for new sends and edits, not historical reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessagePolicy {
    max_content_bytes: usize,
}

/// Content exceeded the operator's write policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MessagePolicyError {
    /// UTF-8 bytes exceeded the inclusive maximum.
    #[error("message content has {actual_bytes} bytes; maximum is {max_bytes}")]
    ContentTooLarge {
        /// Actual UTF-8 byte count.
        actual_bytes: usize,
        /// Configured inclusive maximum.
        max_bytes: usize,
    },
}

impl Default for MessagePolicy {
    fn default() -> Self {
        Self::new(131_072)
    }
}

impl MessagePolicy {
    /// Sets an inclusive UTF-8 byte maximum. Zero accepts only empty text.
    pub const fn new(max_content_bytes: usize) -> Self {
        Self { max_content_bytes }
    }

    /// Returns the inclusive maximum in UTF-8 bytes.
    pub fn max_content_bytes(&self) -> usize {
        self.max_content_bytes
    }

    /// Validates a new write without changing the text.
    pub fn validate(&self, content: &MessageContent) -> Result<(), MessagePolicyError> {
        let actual_bytes = content.as_str().len();
        if actual_bytes > self.max_content_bytes {
            return Err(MessagePolicyError::ContentTooLarge {
                actual_bytes,
                max_bytes: self.max_content_bytes,
            });
        }
        Ok(())
    }
}

/// An active text message or a structural tombstone, ordered by its UUIDv7 ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    id: MessageId,
    conversation_id: ConversationId,
    sender_id: UserId,
    content: Option<MessageContent>,
    reply_to_message_id: Option<MessageId>,
    created_at: OffsetDateTime,
    edited_at: Option<OffsetDateTime>,
    deleted_at: Option<OffsetDateTime>,
}

/// Message state violated a structural invariant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MessageError {
    /// Active messages require content and tombstones must not retain content.
    #[error("message content does not match deletion state")]
    InvalidDeletionState,
    /// A message cannot reference itself as a reply target.
    #[error("message cannot reply to itself")]
    SelfReply,
}

impl Message {
    /// Constructs an active message; reply availability is enforced by storage.
    pub fn new(
        id: MessageId,
        conversation_id: ConversationId,
        sender_id: UserId,
        content: MessageContent,
        reply_to_message_id: Option<MessageId>,
        now: OffsetDateTime,
    ) -> Result<Self, MessageError> {
        Self::restore(
            id,
            conversation_id,
            sender_id,
            Some(content),
            reply_to_message_id,
            now,
            None,
            None,
        )
    }

    /// Restores structural state without applying a current write-size policy.
    #[expect(
        clippy::too_many_arguments,
        reason = "Explicit persisted message fields"
    )]
    pub fn restore(
        id: MessageId,
        conversation_id: ConversationId,
        sender_id: UserId,
        content: Option<MessageContent>,
        reply_to_message_id: Option<MessageId>,
        created_at: OffsetDateTime,
        edited_at: Option<OffsetDateTime>,
        deleted_at: Option<OffsetDateTime>,
    ) -> Result<Self, MessageError> {
        if content.is_some() == deleted_at.is_some() {
            return Err(MessageError::InvalidDeletionState);
        }
        if reply_to_message_id == Some(id) {
            return Err(MessageError::SelfReply);
        }
        Ok(Self {
            id,
            conversation_id,
            sender_id,
            content,
            reply_to_message_id,
            created_at,
            edited_at,
            deleted_at,
        })
    }

    /// Returns the canonical identity and ordering key.
    pub fn id(&self) -> MessageId {
        self.id
    }
    /// Returns the owning conversation.
    pub fn conversation_id(&self) -> ConversationId {
        self.conversation_id
    }
    /// Returns the historical sender, independent of current membership.
    pub fn sender_id(&self) -> UserId {
        self.sender_id
    }
    /// Returns active content, or None for a tombstone.
    pub fn content(&self) -> Option<&MessageContent> {
        self.content.as_ref()
    }
    /// Returns the immutable lightweight reply reference.
    pub fn reply_to_message_id(&self) -> Option<MessageId> {
        self.reply_to_message_id
    }
    /// Returns creation time.
    pub fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
    /// Returns the last meaningful edit time, including on tombstones.
    pub fn edited_at(&self) -> Option<OffsetDateTime> {
        self.edited_at
    }
    /// Returns the first deletion time.
    pub fn deleted_at(&self) -> Option<OffsetDateTime> {
        self.deleted_at
    }
    /// Whether this message is a tombstone.
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_preserves_all_portable_text() {
        for value in [
            "",
            " ",
            "     ",
            "\n\n",
            "hello",
            "  hello  ",
            "😀",
            "e\u{301}",
        ] {
            assert_eq!(MessageContent::new(value).unwrap().as_str(), value);
        }
        assert_eq!(
            MessageContent::new("before\0after"),
            Err(MessageContentError::ContainsNul)
        );
    }

    #[test]
    fn policy_counts_bytes_inclusively() {
        assert_eq!(MessagePolicy::default().max_content_bytes(), 131_072);
        let emoji = MessageContent::new("😀").unwrap();
        assert!(MessagePolicy::new(4).validate(&emoji).is_ok());
        assert_eq!(
            MessagePolicy::new(3).validate(&emoji),
            Err(MessagePolicyError::ContentTooLarge {
                actual_bytes: 4,
                max_bytes: 3
            })
        );
        assert!(
            MessagePolicy::new(0)
                .validate(&MessageContent::new("").unwrap())
                .is_ok()
        );
        assert!(
            MessagePolicy::new(0)
                .validate(&MessageContent::new(" ").unwrap())
                .is_err()
        );
        assert!(
            MessagePolicy::default()
                .validate(&MessageContent::new("a".repeat(131_072)).unwrap())
                .is_ok()
        );
        assert!(
            MessagePolicy::default()
                .validate(&MessageContent::new("a".repeat(131_073)).unwrap())
                .is_err()
        );
    }

    #[test]
    fn message_restore_checks_tombstones_and_self_replies() {
        let id = MessageId::new();
        let conversation = ConversationId::new();
        let sender = UserId::new();
        let now = OffsetDateTime::UNIX_EPOCH;
        let content = MessageContent::new("").unwrap();
        let reply = Some(MessageId::new());
        let active = Message::new(id, conversation, sender, content.clone(), reply, now).unwrap();
        assert_eq!(active.content(), Some(&content));
        assert_eq!(active.reply_to_message_id(), reply);
        assert!(!active.is_deleted());
        assert_eq!(
            Message::restore(id, conversation, sender, None, reply, now, None, None),
            Err(MessageError::InvalidDeletionState)
        );
        assert_eq!(
            Message::restore(
                id,
                conversation,
                sender,
                Some(content.clone()),
                reply,
                now,
                None,
                Some(now)
            ),
            Err(MessageError::InvalidDeletionState)
        );
        let deleted = Message::restore(
            id,
            conversation,
            sender,
            None,
            reply,
            now,
            Some(now),
            Some(now),
        )
        .unwrap();
        assert!(deleted.is_deleted());
        assert_eq!(deleted.edited_at(), Some(now));
        assert!(
            Message::restore(
                id,
                conversation,
                sender,
                Some(content.clone()),
                reply,
                now,
                None,
                None
            )
            .is_ok()
        );
        assert_eq!(
            Message::new(id, conversation, sender, content, Some(id), now),
            Err(MessageError::SelfReply)
        );
    }
}
