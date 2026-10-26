use time::OffsetDateTime;

use crate::{Message, MessageContent, MessageId, UserId, Username};

/// Public identity attribution, independent of credentials and enabled state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserSummary {
    /// Stable user identity.
    pub id: UserId,
    /// Canonical username, including for historical disabled users.
    pub username: Username,
}

/// Shallow current state of a reply target; deliberately contains no nested reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyPreview {
    /// Target identity.
    pub id: MessageId,
    /// Historical sender identity.
    pub sender: UserSummary,
    /// Exact current text, absent for a tombstone.
    pub content: Option<MessageContent>,
    /// Original deletion time, if deleted.
    pub deleted_at: Option<OffsetDateTime>,
}

/// Consumer-ready message state loaded in one consistent persistence operation.
///
/// Dereferencing provides read-only access to the underlying domain message.
/// Identity/reply enrichment is projection state, never part of `Message`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageView {
    /// The persisted active message or structural tombstone.
    pub message: Message,
    /// Current canonical sender username, even when the sender is disabled.
    pub sender_username: Username,
    /// Current shallow target state, if this message is a reply.
    pub reply_to: Option<ReplyPreview>,
}

impl std::ops::Deref for MessageView {
    type Target = Message;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

/// Authoritative own read watermark and unread count from one logical state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationReadState {
    /// Effective monotonic watermark, not a read receipt.
    pub last_read_message_id: Option<MessageId>,
    /// Active non-own messages above the watermark.
    pub unread_count: u64,
}
