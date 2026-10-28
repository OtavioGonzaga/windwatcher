use time::OffsetDateTime;

use crate::{
    CursorError, DomainEvent, ListMessagesStoreOutcome, Outcome, Page, PageRequest, cursor,
};

use crate::{
    ConversationId, DeleteMessageStoreOutcome as Delete, EditMessageStoreOutcome as Edit,
    GetMessageStoreOutcome as Get, MarkReadStoreOutcome as Read, Message, MessageContent,
    MessageContentError, MessageId, MessagePolicy, MessagePolicyError, MessageStore,
    MessageStoreError, MessageView, SendMessageStoreOutcome as Send,
    UnreadCountStoreOutcome as Unread, UserId,
};

macro_rules! operation_error {
    ($name:ident, $description:literal, {$($variant:ident $(($inner:ty))? => $text:literal,)*}) => {
        #[doc = $description]
        #[derive(Debug, thiserror::Error)]
        #[non_exhaustive]
        pub enum $name {
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
    ConversationNotFound => "conversation not found",
    ReplyTargetNotFound => "reply target not found in conversation",
    ReplyTargetDeleted => "reply target is deleted",
    InvalidContent(MessageContentError) => "invalid message content",
    ContentTooLarge(MessagePolicyError) => "message exceeds write policy",
});
operation_error!(GetMessageError, "A message could not be retrieved.", {
    MessageNotFound => "message not found in conversation",
});
operation_error!(ListMessagesError, "Message history could not be listed.", {
    ConversationNotFound => "conversation not found",
    InvalidCursor(CursorError) => "invalid pagination cursor",
});

/// Lists newest-first history, including tombstones, without marking it read.
///
/// ```no_run
/// use windwatcher::{ConversationId, ListMessages, MessageStore, PageRequest, UserId};
/// # async fn example<S: MessageStore>(storage: &S, actor: UserId,
/// # conversation: ConversationId) -> Result<(), Box<dyn std::error::Error>> {
/// let page = ListMessages::new(storage)
///     .execute(actor, conversation, PageRequest::default()).await?;
/// for message in page.items() {
///     // Items are newest to oldest, including tombstones.
///     let _ = message.id();
/// }
/// if let Some(cursor) = page.next_cursor() {
///     let next = ListMessages::new(storage)
///         .execute(actor, conversation, PageRequest::default().with_cursor(cursor)).await?;
/// }
/// # Ok(())
/// # }
/// ```
pub struct ListMessages<'a, S> {
    store: &'a S,
}

impl<'a, S: MessageStore> ListMessages<'a, S> {
    /// Uses the given persistence port.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }

    /// Validates a cursor before storage and independently requires current membership.
    /// Continuations move toward older IDs; page size may change between requests.
    pub async fn execute(
        &self,
        actor: UserId,
        conversation: ConversationId,
        request: PageRequest,
    ) -> Result<Page<MessageView>, ListMessagesError> {
        let before = request
            .cursor()
            .map(|token| cursor::decode_message(token, conversation))
            .transpose()
            .map_err(ListMessagesError::InvalidCursor)?;
        match self
            .store
            .list_messages(actor, conversation, before, request.limit())
            .await?
        {
            ListMessagesStoreOutcome::ConversationNotFound => {
                Err(ListMessagesError::ConversationNotFound)
            }
            ListMessagesStoreOutcome::NotMember => Err(ListMessagesError::NotMember),
            ListMessagesStoreOutcome::Page(slice) => {
                let next = if slice.has_more {
                    let last = slice.items.last().ok_or_else(|| {
                        MessageStoreError::Unexpected(Box::new(std::io::Error::other(
                            "empty message slice with more history",
                        )))
                    })?;
                    Some(cursor::encode_message(conversation, last.id()))
                } else {
                    None
                };
                Ok(Page::new(slice.items, next))
            }
        }
    }
}
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
    ConversationNotFound => "conversation not found",
    MessageNotFound => "message not found in conversation",
});
operation_error!(
    GetUnreadCountError,
    "Unread count could not be retrieved.",
    { ConversationNotFound => "conversation not found", }
);

/// An author's edit either changed content or preserved exact current state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditMessageOutcome {
    /// New content and edit timestamp were persisted.
    Changed(MessageView),
    /// Exact content matched; timestamps were preserved.
    Unchanged(MessageView),
}

/// An authorized deletion preserves a structural tombstone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteMessageOutcome {
    /// Content was destroyed in primary state.
    Deleted(MessageView),
    /// The existing tombstone and timestamps were preserved.
    AlreadyDeleted(MessageView),
}

/// Whether explicit marking advanced the monotonic read boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkReadOutcome {
    /// The read boundary advanced.
    Changed(crate::ConversationReadState),
    /// An equal or greater boundary already existed.
    Unchanged(crate::ConversationReadState),
}

impl MarkReadOutcome {
    /// Authoritative resulting state, including for an older/same target no-op.
    pub fn state(self) -> crate::ConversationReadState {
        match self {
            Self::Changed(state) | Self::Unchanged(state) => state,
        }
    }
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
/// let message_outcome = SendMessage::new(storage, &policy)
///     .execute(actor, conversation, "hello", None, now).await?;
/// let message = message_outcome.value();
/// let reply_outcome = SendMessage::new(storage, &policy)
///     .execute(actor, conversation, "reply", Some(message.id()), now).await?;
/// let reply = reply_outcome.value();
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
    ) -> Result<Outcome<MessageView>, SendMessageError> {
        let content = MessageContent::new(content).map_err(SendMessageError::InvalidContent)?;
        self.policy
            .validate(&content)
            .map_err(SendMessageError::ContentTooLarge)?;
        // A newly generated ID cannot reference an already-existing reply target as itself.
        let message = Message::new(MessageId::new(), conversation, actor, content, reply, now)
            .map_err(|error| MessageStoreError::Unexpected(Box::new(error)))?;
        match self.store.send_message(&message).await? {
            Send::Sent(message) => Ok(Outcome::with_event(
                message.clone(),
                DomainEvent::MessageSent { message },
            )),
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
        message: MessageId,
    ) -> Result<MessageView, GetMessageError> {
        match self.store.get_message(actor, message).await? {
            Get::Found(message) => Ok(message),
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
        message: MessageId,
        content: impl Into<String>,
        now: OffsetDateTime,
    ) -> Result<Outcome<EditMessageOutcome>, EditMessageError> {
        let content = MessageContent::new(content).map_err(EditMessageError::InvalidContent)?;
        self.policy
            .validate(&content)
            .map_err(EditMessageError::ContentTooLarge)?;
        match self
            .store
            .edit_message(actor, message, &content, now)
            .await?
        {
            Edit::Changed(message) => Ok(Outcome::with_event(
                EditMessageOutcome::Changed(message.clone()),
                DomainEvent::MessageEdited { message },
            )),
            Edit::Unchanged(message) => Ok(Outcome::new(EditMessageOutcome::Unchanged(message))),
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
        message: MessageId,
        now: OffsetDateTime,
    ) -> Result<Outcome<DeleteMessageOutcome>, DeleteMessageError> {
        match self.store.delete_message(actor, message, now).await? {
            Delete::Deleted(message) => Ok(Outcome::with_event(
                DeleteMessageOutcome::Deleted(message.clone()),
                DomainEvent::MessageDeleted { message },
            )),
            Delete::AlreadyDeleted(message) => {
                Ok(Outcome::new(DeleteMessageOutcome::AlreadyDeleted(message)))
            }
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
    ) -> Result<Outcome<MarkReadOutcome>, MarkConversationReadError> {
        match self
            .store
            .mark_read_through(actor, conversation, message)
            .await?
        {
            Read::Changed(state) => Ok(Outcome::with_event(
                MarkReadOutcome::Changed(state),
                DomainEvent::ConversationReadStateAdvanced {
                    user_id: actor,
                    conversation_id: conversation,
                    state,
                },
            )),
            Read::Unchanged(state) => Ok(Outcome::new(MarkReadOutcome::Unchanged(state))),
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
