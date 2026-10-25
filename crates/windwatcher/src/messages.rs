use time::OffsetDateTime;

use crate::{
    ConversationId, DeleteMessageStoreOutcome as Delete, EditMessageStoreOutcome as Edit,
    GetMessageStoreOutcome as Get, MarkReadStoreOutcome as Read, Message, MessageContent,
    MessageContentError, MessageId, MessagePolicy, MessagePolicyError, MessageStore,
    MessageStoreError, SendMessageStoreOutcome as Send, UnreadCountStoreOutcome as Unread, UserId,
};

macro_rules! operation_error {
    ($name:ident, $description:literal, {$($variant:ident $(($inner:ty))? => $text:literal,)*}) => {
        #[doc = $description]
        #[derive(Debug, thiserror::Error)]
        #[non_exhaustive]
        pub enum $name {
            /// The requested conversation does not exist.
            #[error("conversation not found")]
            ConversationNotFound,
            /// The actor is not currently a member.
            #[error("actor is not a conversation member")]
            NotMember,
            $(#[doc = $text] #[error($text)] $variant $( (#[source] $inner) )?,)*
            /// Primary persistence failed.
            #[error("message storage failed")]
            Storage(#[from] MessageStoreError),
        }
    };
}

operation_error!(SendMessageError, "A message could not be sent.", {
    ReplyTargetNotFound => "reply target not found in conversation",
    ReplyTargetDeleted => "reply target is deleted",
    InvalidContent(MessageContentError) => "invalid message content",
    ContentTooLarge(MessagePolicyError) => "message exceeds write policy",
});
operation_error!(GetMessageError, "A message could not be retrieved.", {
    MessageNotFound => "message not found in conversation",
});
operation_error!(EditMessageError, "A message could not be edited.", {
    MessageNotFound => "message not found in conversation",
    NotAuthor => "only the author may edit a message",
    MessageDeleted => "deleted messages cannot be edited",
    InvalidContent(MessageContentError) => "invalid message content",
    ContentTooLarge(MessagePolicyError) => "message exceeds write policy",
});
operation_error!(DeleteMessageError, "A message could not be deleted.", {
    MessageNotFound => "message not found in conversation",
    NotAuthorized => "actor cannot delete this message",
});
operation_error!(MarkConversationReadError, "A read watermark could not be advanced.", {
    MessageNotFound => "message not found in conversation",
});
operation_error!(
    GetUnreadCountError,
    "Unread count could not be retrieved.",
    {}
);

/// An author's edit either changed content or preserved exact current state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditMessageOutcome {
    /// New content and edit timestamp were persisted.
    Changed(Message),
    /// Exact content matched; timestamps were preserved.
    Unchanged(Message),
}

/// An authorized deletion preserves a structural tombstone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteMessageOutcome {
    /// Content was destroyed in primary state.
    Deleted(Message),
    /// The existing tombstone and timestamps were preserved.
    AlreadyDeleted(Message),
}

/// Whether explicit marking advanced the monotonic read boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkReadOutcome {
    /// The read boundary advanced.
    Changed,
    /// An equal or greater boundary already existed.
    Unchanged,
}

/// Synchronously sends exact portable text under an embedder's write policy.
///
/// ```no_run
/// use windwatcher::{
///     ConversationId, GetUnreadCount, MarkConversationReadThrough, MessagePolicy,
///     MessageStore, SendMessage, UserId,
/// };
/// # async fn example<S: MessageStore>(storage: &S, actor: UserId,
/// # conversation: ConversationId, now: time::OffsetDateTime)
/// # -> Result<(), Box<dyn std::error::Error>> {
/// let policy = MessagePolicy::default();
/// let message = SendMessage::new(storage, &policy)
///     .execute(actor, conversation, "hello", None, now).await?;
/// let reply = SendMessage::new(storage, &policy)
///     .execute(actor, conversation, "reply", Some(message.id()), now).await?;
/// // Sending either message did not implicitly advance the read marker.
/// MarkConversationReadThrough::new(storage)
///     .execute(actor, conversation, reply.id()).await?;
/// let unread = GetUnreadCount::new(storage).execute(actor, conversation).await?;
/// # let _ = unread;
/// # Ok(())
/// # }
/// ```
pub struct SendMessage<'a, S> {
    store: &'a S,
    policy: &'a MessagePolicy,
}
impl<'a, S: MessageStore> SendMessage<'a, S> {
    /// Uses the given persistence port and new-write policy.
    pub fn new(store: &'a S, policy: &'a MessagePolicy) -> Self {
        Self { store, policy }
    }

    /// Generates an ID and durably sends as the actor, without marking history read.
    pub async fn execute(
        &self,
        actor: UserId,
        conversation: ConversationId,
        content: impl Into<String>,
        reply: Option<MessageId>,
        now: OffsetDateTime,
    ) -> Result<Message, SendMessageError> {
        let content = MessageContent::new(content).map_err(SendMessageError::InvalidContent)?;
        self.policy
            .validate(&content)
            .map_err(SendMessageError::ContentTooLarge)?;
        // A newly generated ID cannot reference an already-existing reply target as itself.
        let message = Message::new(MessageId::new(), conversation, actor, content, reply, now)
            .map_err(|error| MessageStoreError::Unexpected(Box::new(error)))?;
        match self.store.send_message(&message).await? {
            Send::Sent(message) => Ok(message),
            Send::ConversationNotFound => Err(SendMessageError::ConversationNotFound),
            Send::NotMember => Err(SendMessageError::NotMember),
            Send::ReplyTargetNotFound => Err(SendMessageError::ReplyTargetNotFound),
            Send::ReplyTargetDeleted => Err(SendMessageError::ReplyTargetDeleted),
        }
    }
}

/// Retrieves an active message or tombstone for a current conversation member.
pub struct GetMessage<'a, S> {
    store: &'a S,
}
impl<'a, S: MessageStore> GetMessage<'a, S> {
    /// Uses the given persistence port.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Requires current membership without applying a new-write size policy.
    pub async fn execute(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
    ) -> Result<Message, GetMessageError> {
        match self.store.get_message(actor, conversation, message).await? {
            Get::Found(message) => Ok(message),
            Get::ConversationNotFound => Err(GetMessageError::ConversationNotFound),
            Get::NotMember => Err(GetMessageError::NotMember),
            Get::MessageNotFound => Err(GetMessageError::MessageNotFound),
        }
    }
}

/// Applies author-only edits under the current write policy.
pub struct EditMessage<'a, S> {
    store: &'a S,
    policy: &'a MessagePolicy,
}
impl<'a, S: MessageStore> EditMessage<'a, S> {
    /// Uses the given persistence port and write policy.
    pub fn new(store: &'a S, policy: &'a MessagePolicy) -> Self {
        Self { store, policy }
    }
    /// Edits exact text; identical content preserves the previous edit timestamp.
    pub async fn execute(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
        content: impl Into<String>,
        now: OffsetDateTime,
    ) -> Result<EditMessageOutcome, EditMessageError> {
        let content = MessageContent::new(content).map_err(EditMessageError::InvalidContent)?;
        self.policy
            .validate(&content)
            .map_err(EditMessageError::ContentTooLarge)?;
        match self
            .store
            .edit_message(actor, conversation, message, &content, now)
            .await?
        {
            Edit::Changed(message) => Ok(EditMessageOutcome::Changed(message)),
            Edit::Unchanged(message) => Ok(EditMessageOutcome::Unchanged(message)),
            Edit::ConversationNotFound => Err(EditMessageError::ConversationNotFound),
            Edit::NotMember => Err(EditMessageError::NotMember),
            Edit::MessageNotFound => Err(EditMessageError::MessageNotFound),
            Edit::NotAuthor => Err(EditMessageError::NotAuthor),
            Edit::Deleted => Err(EditMessageError::MessageDeleted),
        }
    }
}

/// Deletes content as its author or as a Group Owner/Admin moderator.
pub struct DeleteMessage<'a, S> {
    store: &'a S,
}
impl<'a, S: MessageStore> DeleteMessage<'a, S> {
    /// Uses the given persistence port.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Preserves identity and replies; repeated deletion still requires permission.
    pub async fn execute(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
        now: OffsetDateTime,
    ) -> Result<DeleteMessageOutcome, DeleteMessageError> {
        match self
            .store
            .delete_message(actor, conversation, message, now)
            .await?
        {
            Delete::Deleted(message) => Ok(DeleteMessageOutcome::Deleted(message)),
            Delete::AlreadyDeleted(message) => Ok(DeleteMessageOutcome::AlreadyDeleted(message)),
            Delete::ConversationNotFound => Err(DeleteMessageError::ConversationNotFound),
            Delete::NotMember => Err(DeleteMessageError::NotMember),
            Delete::MessageNotFound => Err(DeleteMessageError::MessageNotFound),
            Delete::NotAuthorized => Err(DeleteMessageError::NotAuthorized),
        }
    }
}

/// Explicitly advances a current member's monotonic UUIDv7 read boundary.
pub struct MarkConversationReadThrough<'a, S> {
    store: &'a S,
}
impl<'a, S: MessageStore> MarkConversationReadThrough<'a, S> {
    /// Uses the given persistence port.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Marks through an existing same-conversation message, including a tombstone.
    pub async fn execute(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
    ) -> Result<MarkReadOutcome, MarkConversationReadError> {
        match self
            .store
            .mark_read_through(actor, conversation, message)
            .await?
        {
            Read::Changed => Ok(MarkReadOutcome::Changed),
            Read::Unchanged => Ok(MarkReadOutcome::Unchanged),
            Read::ConversationNotFound => Err(MarkConversationReadError::ConversationNotFound),
            Read::NotMember => Err(MarkConversationReadError::NotMember),
            Read::MessageNotFound => Err(MarkConversationReadError::MessageNotFound),
        }
    }
}

/// Computes unread active messages from primary state, excluding own messages.
pub struct GetUnreadCount<'a, S> {
    store: &'a S,
}
impl<'a, S: MessageStore> GetUnreadCount<'a, S> {
    /// Uses the given persistence port.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Requires current membership and returns a checked portable count.
    pub async fn execute(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> Result<u64, GetUnreadCountError> {
        match self.store.unread_count(actor, conversation).await? {
            Unread::Count(count) => Ok(count),
            Unread::ConversationNotFound => Err(GetUnreadCountError::ConversationNotFound),
            Unread::NotMember => Err(GetUnreadCountError::NotMember),
        }
    }
}
