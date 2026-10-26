use std::{error::Error, future::Future};

use time::OffsetDateTime;

use crate::{ConversationId, Message, MessageContent, MessageId, PageLimit, UserId};

/// A bounded, strictly descending message slice, including tombstones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePageSlice {
    /// At most the requested limit, all belonging to the requested conversation.
    pub items: Vec<Message>,
    /// True exactly when an older item exists beyond the returned slice.
    /// A slice with more items available must not be empty.
    pub has_more: bool,
}

/// Result of one consistently authorized message-history read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListMessagesStoreOutcome {
    /// The bounded newest-first history slice.
    Page(MessagePageSlice),
    /// The conversation does not exist.
    ConversationNotFound,
    /// The actor is not a current member.
    NotMember,
}

/// Normalized infrastructure failures, never business authorization outcomes.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MessageStoreError {
    /// A uniqueness or integrity constraint prevented persistence.
    #[error("message storage conflict")]
    Conflict,
    /// Storage is temporarily unavailable.
    #[error("message storage unavailable")]
    Unavailable(#[source] Box<dyn Error + Send + Sync>),
    /// Storage returned malformed or unexpected state.
    #[error("unexpected message storage failure")]
    Unexpected(#[source] Box<dyn Error + Send + Sync>),
}

/// Result of atomically persisting a new message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendMessageStoreOutcome {
    /// The active message is durably persisted.
    Sent(Message),
    /// The conversation does not exist.
    ConversationNotFound,
    /// The sender is not currently a member.
    NotMember,
    /// The reply target is unavailable in this conversation.
    ReplyTargetNotFound,
    /// The reply target is already a tombstone.
    ReplyTargetDeleted,
}

/// Result of an authorized single-message read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GetMessageStoreOutcome {
    /// An active message or tombstone.
    Found(Message),
    /// The conversation does not exist.
    ConversationNotFound,
    /// The actor is not a current member.
    NotMember,
    /// The message is unavailable in this conversation.
    MessageNotFound,
}

/// Result of an atomic author-only edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditMessageStoreOutcome {
    /// Content changed and the edit timestamp advanced.
    Changed(Message),
    /// Exact content already matched; no timestamps changed.
    Unchanged(Message),
    /// The conversation does not exist.
    ConversationNotFound,
    /// The actor is not a current member.
    NotMember,
    /// The message is unavailable in this conversation.
    MessageNotFound,
    /// The actor is not the author, regardless of moderation role.
    NotAuthor,
    /// The message is a tombstone and cannot be edited.
    Deleted,
}

/// Result of an authorized content-destroying soft deletion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteMessageStoreOutcome {
    /// Content was removed and the structural tombstone persisted.
    Deleted(Message),
    /// An authorized repeated deletion preserved the original timestamps.
    AlreadyDeleted(Message),
    /// The conversation does not exist.
    ConversationNotFound,
    /// The actor is not a current member.
    NotMember,
    /// The message is unavailable in this conversation.
    MessageNotFound,
    /// The actor is neither the author nor a Group moderator.
    NotAuthorized,
}

/// Result of atomically advancing a membership's read watermark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkReadStoreOutcome {
    /// The watermark advanced.
    Changed,
    /// An equal or later watermark already existed.
    Unchanged,
    /// The conversation does not exist.
    ConversationNotFound,
    /// The actor is not a current member.
    NotMember,
    /// The boundary is unavailable in this conversation.
    MessageNotFound,
}

/// Result of deriving unread count from one consistent authorized view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnreadCountStoreOutcome {
    /// Active messages after the watermark, excluding the actor's own messages.
    Count(u64),
    /// The conversation does not exist.
    ConversationNotFound,
    /// The actor is not a current member.
    NotMember,
}

/// Semantic, atomic message persistence, independent of executor and backend.
///
/// Mutations must enforce current membership and role at persistence time. IDs
/// use `MessageId::Ord`, not commit order. Reads include tombstones; unread counts
/// exclude them. No operation changes conversation metadata timestamps.
pub trait MessageStore: Sync {
    /// Lists IDs strictly below `before`, newest first by `MessageId::Ord`.
    /// Existence, current membership and history share one logical read snapshot.
    /// Includes tombstones, never marks read, and does not require the boundary
    /// to reference an existing row. `has_more` must use actual older history,
    /// not merely whether the slice fills the limit. No cursor encoding is used.
    fn list_messages(
        &self,
        actor: UserId,
        conversation: ConversationId,
        before: Option<MessageId>,
        limit: PageLimit,
    ) -> impl Future<Output = Result<ListMessagesStoreOutcome, MessageStoreError>> + Send;
    /// Persists an active, unedited message whose sender is the actor. Rejects
    /// unavailable/deleted reply targets atomically with insertion. Never advances read state.
    fn send_message(
        &self,
        message: &Message,
    ) -> impl Future<Output = Result<SendMessageStoreOutcome, MessageStoreError>> + Send;
    /// Returns a message only to a current member of the specified conversation.
    fn get_message(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
    ) -> impl Future<Output = Result<GetMessageStoreOutcome, MessageStoreError>> + Send;
    /// Conditionally edits an active message owned by a current member.
    fn edit_message(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
        content: &MessageContent,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<EditMessageStoreOutcome, MessageStoreError>> + Send;
    /// Destroys content while preserving identity and replies. Author or Group
    /// Owner/Admin permission is required even for an existing tombstone.
    fn delete_message(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<DeleteMessageStoreOutcome, MessageStoreError>> + Send;
    /// Advances only to a greater existing same-conversation ID; tombstones qualify.
    fn mark_read_through(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
    ) -> impl Future<Output = Result<MarkReadStoreOutcome, MessageStoreError>> + Send;
    /// Computes checked unread count, requiring current membership.
    fn unread_count(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<UnreadCountStoreOutcome, MessageStoreError>> + Send;
}
