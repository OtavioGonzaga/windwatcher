use crate::{
    Conversation, ConversationId, ConversationKind, ConversationMember, GroupRole, GroupTitle,
    UserId,
};
use std::{error::Error, future::Future};
use time::OffsetDateTime;

/// Normalized failures at the conversation persistence boundary.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConversationStoreError {
    /// A uniqueness or integrity constraint prevented the operation.
    #[error("conversation storage conflict")]
    Conflict,
    /// A conversation did not match the required kind for this operation.
    #[error("conversation kind is invalid for this storage operation")]
    InvalidConversationKind {
        /// The kind required by the operation.
        expected: ConversationKind,
        /// The kind supplied to the operation.
        actual: ConversationKind,
    },
    /// A generic role mutation received the Owner role.
    #[error("owner role must be changed through ownership transfer")]
    InvalidGroupRole,
    /// Storage is temporarily unavailable.
    #[error("conversation storage unavailable")]
    Unavailable(#[source] Box<dyn Error + Send + Sync>),
    /// Storage returned malformed or unexpected data.
    #[error("unexpected conversation storage failure")]
    Unexpected(#[source] Box<dyn Error + Send + Sync>),
}

/// Whether a Direct was newly persisted or already existed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartDirectStoreOutcome {
    /// The Direct was persisted by this operation.
    Created(Conversation),
    /// An equivalent Direct already existed.
    Existing(Conversation),
}
/// Result of adding a Group member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddMemberStoreOutcome {
    /// A new Member was persisted.
    Added,
    /// The target was already a member.
    AlreadyMember,
    /// The actor no longer had Group-management permission at mutation time.
    NotAuthorized,
}
/// Result of an idempotent Group metadata mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeStoreOutcome {
    /// A meaningful mutation was persisted.
    Changed,
    /// The requested state already existed.
    Unchanged,
    /// The actor no longer had permission at mutation time.
    NotAuthorized,
}
/// Result of leaving a Group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaveStoreOutcome {
    /// A non-owner member left.
    Left,
    /// The owner-only Group was deleted.
    ConversationDeleted,
    /// The Owner must transfer ownership before leaving.
    OwnershipTransferRequired,
    /// The actor was not a member.
    NotMember,
    /// The conversation is a Direct.
    Direct,
}

/// Application-driven persistence operations for conversation lifecycles.
pub trait ConversationStore: Sync {
    /// Atomically gets or creates the unordered Direct participant pair.
    fn get_or_create_direct(
        &self,
        conversation: &Conversation,
        first: UserId,
        second: UserId,
    ) -> impl Future<Output = Result<StartDirectStoreOutcome, ConversationStoreError>> + Send;
    /// Atomically creates a Group, its Owner, and all distinct initial Members.
    fn create_group(
        &self,
        conversation: &Conversation,
        owner: UserId,
        participants: &[UserId],
    ) -> impl Future<Output = Result<(), ConversationStoreError>> + Send;
    /// Finds a conversation by ID.
    fn find_conversation(
        &self,
        id: ConversationId,
    ) -> impl Future<Output = Result<Option<Conversation>, ConversationStoreError>> + Send;
    /// Finds a member in a conversation.
    fn find_member(
        &self,
        conversation_id: ConversationId,
        user_id: UserId,
    ) -> impl Future<Output = Result<Option<ConversationMember>, ConversationStoreError>> + Send;
    /// Adds a Group Member only when the actor is Owner or Admin. Atomically
    /// initializes a new membership's read marker to the maximum persisted
    /// message ID (including tombstones), or None for empty history. An existing
    /// member's marker is never reset; rejoining creates a fresh boundary.
    fn add_group_member(
        &self,
        conversation_id: ConversationId,
        actor: UserId,
        member: UserId,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<AddMemberStoreOutcome, ConversationStoreError>> + Send;
    /// Removes a non-Owner Group member under the Owner/Admin permission rules.
    fn remove_group_member(
        &self,
        conversation_id: ConversationId,
        actor: UserId,
        member: UserId,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<ChangeStoreOutcome, ConversationStoreError>> + Send;
    /// Changes a non-Owner Group member between Admin and Member.
    fn set_group_member_role(
        &self,
        conversation_id: ConversationId,
        actor: UserId,
        member: UserId,
        role: GroupRole,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<ChangeStoreOutcome, ConversationStoreError>> + Send;
    /// Atomically moves ownership, demoting the previous Owner to Admin.
    fn transfer_group_ownership(
        &self,
        conversation_id: ConversationId,
        actor: UserId,
        target: UserId,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<ChangeStoreOutcome, ConversationStoreError>> + Send;
    /// Leaves a Group or deletes an owner-only Group atomically.
    fn leave_group(
        &self,
        conversation_id: ConversationId,
        actor: UserId,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<LeaveStoreOutcome, ConversationStoreError>> + Send;
    /// Renames a Group when the actor is Owner or Admin.
    fn rename_group(
        &self,
        conversation_id: ConversationId,
        actor: UserId,
        title: &GroupTitle,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<ChangeStoreOutcome, ConversationStoreError>> + Send;
}
