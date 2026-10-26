use std::{error::Error, future::Future};

use crate::{
    ConversationBoundary, ConversationId, ConversationMemberSummary, ConversationSummary,
    CursorError, MemberBoundary, Page, PageLimit, PageRequest, UserId, UserSummary, Username,
    UsernameError, cursor,
};

/// Bounded storage projection slice, without transport or cursor representation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuerySlice<T> {
    /// At most the requested limit, in canonical resource order.
    pub items: Vec<T>,
    /// Whether an actual item exists past this slice.
    pub has_more: bool,
}

/// Consistent authorization result for a conversation-scoped query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationQueryOutcome<T> {
    /// Authorized query result.
    Found(T),
    /// Parent does not exist.
    ConversationNotFound,
    /// Actor is not a current member.
    NotMember,
}

/// Infrastructure failures loading reusable query projections.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum QueryStoreError {
    /// Temporary persistence unavailability.
    #[error("query storage unavailable")]
    Unavailable(#[source] Box<dyn Error + Send + Sync>),
    /// Malformed persisted state or another unexpected failure.
    #[error("unexpected query storage failure")]
    Unexpected(#[source] Box<dyn Error + Send + Sync>),
}

/// Semantic consumer queries, independent of HTTP and backend representation.
///
/// Each operation must use one consistent read snapshot and a bounded number of
/// queries independent of page size. Current membership is rechecked every page.
pub trait MessagingQueryStore: Sync {
    /// Exact canonical username lookup, enabled users only, at most one result.
    fn find_users(
        &self,
        username: &Username,
    ) -> impl Future<Output = Result<Vec<UserSummary>, QueryStoreError>> + Send;
    /// Actor inbox ordered by effective activity DESC, conversation ID DESC.
    fn list_conversations(
        &self,
        actor: UserId,
        before: Option<ConversationBoundary>,
        limit: PageLimit,
    ) -> impl Future<Output = Result<QuerySlice<ConversationSummary>, QueryStoreError>> + Send;
    /// Detail with the same projection and visibility as an inbox item.
    fn get_conversation_summary(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> impl Future<Output = Result<ConversationQueryOutcome<ConversationSummary>, QueryStoreError>>
    + Send;
    /// Both Direct/Group members ordered by joined_at ASC, user ID ASC. Disabled
    /// existing users remain visible; other members' read markers never appear.
    fn list_conversation_members(
        &self,
        actor: UserId,
        conversation: ConversationId,
        after: Option<MemberBoundary>,
        limit: PageLimit,
    ) -> impl Future<
        Output = Result<
            ConversationQueryOutcome<QuerySlice<ConversationMemberSummary>>,
            QueryStoreError,
        >,
    > + Send;
}

/// Errors shared by consumer queries; supplied cursors never appear in errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MessagingQueryError {
    /// Username is not a valid canonicalizable identity.
    #[error("invalid username")]
    InvalidUsername(#[source] UsernameError),
    /// Cursor kind, version, payload or scope is invalid.
    #[error("invalid pagination cursor")]
    InvalidCursor(#[source] CursorError),
    /// Conversation does not exist.
    #[error("conversation not found")]
    ConversationNotFound,
    /// Actor is not currently a member.
    #[error("actor is not a conversation member")]
    NotMember,
    /// Persistence failed.
    #[error("query storage failed")]
    Storage(#[from] QueryStoreError),
}

fn authorized<T>(outcome: ConversationQueryOutcome<T>) -> Result<T, MessagingQueryError> {
    match outcome {
        ConversationQueryOutcome::Found(value) => Ok(value),
        ConversationQueryOutcome::ConversationNotFound => {
            Err(MessagingQueryError::ConversationNotFound)
        }
        ConversationQueryOutcome::NotMember => Err(MessagingQueryError::NotMember),
    }
}

fn page<T>(
    slice: QuerySlice<T>,
    encode: impl FnOnce(&T) -> String,
) -> Result<Page<T>, MessagingQueryError> {
    let cursor = if slice.has_more {
        Some(encode(slice.items.last().ok_or_else(|| {
            QueryStoreError::Unexpected(Box::new(std::io::Error::other(
                "empty query slice with more items",
            )))
        })?))
    } else {
        None
    };
    Ok(Page::new(slice.items, cursor))
}

/// Finds at most one enabled user by exact canonical username, without credentials.
pub struct FindUsers<'a, S>(&'a S);
impl<'a, S: MessagingQueryStore> FindUsers<'a, S> {
    /// Uses semantic query persistence.
    pub fn new(store: &'a S) -> Self {
        Self(store)
    }
    /// Missing and disabled users both produce an empty terminal page.
    pub async fn execute(&self, username: &str) -> Result<Page<UserSummary>, MessagingQueryError> {
        let username = username
            .parse::<Username>()
            .map_err(MessagingQueryError::InvalidUsername)?;
        Ok(Page::new(self.0.find_users(&username).await?, None))
    }
}

/// Lists the actor's consumer-ready inbox without per-item hydration requests.
pub struct ListConversations<'a, S>(&'a S);
impl<'a, S: MessagingQueryStore> ListConversations<'a, S> {
    /// Uses semantic query persistence.
    pub fn new(store: &'a S) -> Self {
        Self(store)
    }
    /// Continues toward older effective activity; page size may change.
    pub async fn execute(
        &self,
        actor: UserId,
        request: PageRequest,
    ) -> Result<Page<ConversationSummary>, MessagingQueryError> {
        let before = request
            .cursor()
            .map(|s| cursor::decode_conversation(s, actor))
            .transpose()
            .map_err(MessagingQueryError::InvalidCursor)?;
        page(
            self.0
                .list_conversations(actor, before, request.limit())
                .await?,
            |last| {
                cursor::encode_conversation(
                    actor,
                    ConversationBoundary {
                        activity_at: last.activity_at,
                        conversation_id: last.conversation.id(),
                    },
                )
            },
        )
    }
}

/// Loads the same inbox projection for a single current-member conversation.
pub struct GetConversationSummary<'a, S>(&'a S);
impl<'a, S: MessagingQueryStore> GetConversationSummary<'a, S> {
    /// Uses semantic query persistence.
    pub fn new(store: &'a S) -> Self {
        Self(store)
    }
    /// Resolves existence, membership and projection in the same read snapshot.
    pub async fn execute(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> Result<ConversationSummary, MessagingQueryError> {
        authorized(self.0.get_conversation_summary(actor, conversation).await?)
    }
}

/// Lists public membership attribution without exposing other users' read state.
pub struct ListConversationMembers<'a, S>(&'a S);
impl<'a, S: MessagingQueryStore> ListConversationMembers<'a, S> {
    /// Uses semantic query persistence.
    pub fn new(store: &'a S) -> Self {
        Self(store)
    }
    /// Reauthorizes membership even when a valid old cursor is supplied.
    pub async fn execute(
        &self,
        actor: UserId,
        conversation: ConversationId,
        request: PageRequest,
    ) -> Result<Page<ConversationMemberSummary>, MessagingQueryError> {
        let after = request
            .cursor()
            .map(|s| cursor::decode_member(s, conversation))
            .transpose()
            .map_err(MessagingQueryError::InvalidCursor)?;
        page(
            authorized(
                self.0
                    .list_conversation_members(actor, conversation, after, request.limit())
                    .await?,
            )?,
            |last| {
                cursor::encode_member(
                    conversation,
                    MemberBoundary {
                        joined_at: last.joined_at,
                        user_id: last.user.id,
                    },
                )
            },
        )
    }
}
