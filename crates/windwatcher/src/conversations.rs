use time::OffsetDateTime;

use crate::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationKind,
    ConversationMember, ConversationStore, ConversationStoreError, GroupRole, GroupTitle,
    GroupTitleError, IdentityStore, IdentityStoreError, LeaveStoreOutcome, MembershipRole,
    StartDirectStoreOutcome, UserId,
};

async fn participant<I: IdentityStore>(
    identities: &I,
    id: UserId,
) -> Result<(), ConversationOperationError> {
    if identities
        .find_user(id)
        .await
        .map_err(ConversationOperationError::Identity)?
        .is_some()
    {
        Ok(())
    } else {
        Err(ConversationOperationError::ParticipantNotFound(id))
    }
}

/// Shared semantic failures for conversation application operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConversationOperationError {
    /// A referenced user does not exist.
    #[error("conversation participant does not exist")]
    ParticipantNotFound(UserId),
    /// The conversation does not exist.
    #[error("conversation not found")]
    ConversationNotFound,
    /// The actor is not a conversation member.
    #[error("actor is not a conversation member")]
    NotMember,
    /// The actor lacks the required Group permission.
    #[error("insufficient group permissions")]
    InsufficientPermissions,
    /// The operation is not meaningful for Direct conversations.
    #[error("operation is not supported for Direct conversations")]
    DirectOperationNotSupported,
    /// The Group Owner is protected from this operation.
    #[error("group owner is protected")]
    OwnerProtected,
    /// Ownership must move before the Owner can leave.
    #[error("ownership transfer is required before the owner leaves")]
    OwnershipTransferRequired,
    /// The target is not a Group member.
    #[error("target is not a group member")]
    TargetNotMember,
    /// Generic role assignment cannot assign the Owner role.
    #[error("owner role must be changed through ownership transfer")]
    OwnerRoleNotAssignable,
    /// The requested title was invalid.
    #[error("invalid group title")]
    InvalidTitle(#[source] GroupTitleError),
    /// Identity lookup failed.
    #[error("identity lookup failed")]
    Identity(#[source] IdentityStoreError),
    /// Conversation persistence failed.
    #[error("conversation storage failed")]
    Storage(#[source] ConversationStoreError),
}

/// Starts or finds an immutable Direct conversation.
pub struct StartDirectConversation<'a, I, S> {
    identities: &'a I,
    store: &'a S,
}
impl<'a, I: IdentityStore, S: ConversationStore> StartDirectConversation<'a, I, S> {
    /// Wires identity validation and persistence.
    pub fn new(identities: &'a I, store: &'a S) -> Self {
        Self { identities, store }
    }
    /// Gets or creates the unordered actor/participant pair; self Directs are valid.
    pub async fn execute(
        &self,
        actor: UserId,
        participant_id: UserId,
        now: OffsetDateTime,
    ) -> Result<StartDirectStoreOutcome, ConversationOperationError> {
        participant(self.identities, actor).await?;
        if participant_id != actor {
            participant(self.identities, participant_id).await?;
        }
        self.store
            .get_or_create_direct(
                &Conversation::new_direct(ConversationId::new(), now),
                actor,
                participant_id,
            )
            .await
            .map_err(ConversationOperationError::Storage)
    }
}

/// Creates a Group with one Owner and zero or more initial Members.
pub struct CreateGroupConversation<'a, I, S> {
    identities: &'a I,
    store: &'a S,
}
impl<'a, I: IdentityStore, S: ConversationStore> CreateGroupConversation<'a, I, S> {
    /// Wires identity validation and persistence.
    pub fn new(identities: &'a I, store: &'a S) -> Self {
        Self { identities, store }
    }
    /// Creates the Group atomically; duplicate participant IDs are a set.
    pub async fn execute(
        &self,
        actor: UserId,
        title: impl Into<String>,
        participants: &[UserId],
        now: OffsetDateTime,
    ) -> Result<Conversation, ConversationOperationError> {
        participant(self.identities, actor).await?;
        let title = GroupTitle::new(title).map_err(ConversationOperationError::InvalidTitle)?;
        let mut unique = Vec::new();
        for id in participants {
            if *id != actor && !unique.contains(id) {
                participant(self.identities, *id).await?;
                unique.push(*id);
            }
        }
        let conversation = Conversation::new_group(ConversationId::new(), title, now);
        self.store
            .create_group(&conversation, actor, &unique)
            .await
            .map_err(ConversationOperationError::Storage)?;
        Ok(conversation)
    }
}

/// Reads a conversation only for one of its members.
pub struct GetConversation<'a, S> {
    store: &'a S,
}
impl<'a, S: ConversationStore> GetConversation<'a, S> {
    /// Wires conversation persistence.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Loads a conversation after confirming the actor is a member.
    pub async fn execute(
        &self,
        actor: UserId,
        id: ConversationId,
    ) -> Result<Conversation, ConversationOperationError> {
        let conversation = self
            .store
            .find_conversation(id)
            .await
            .map_err(ConversationOperationError::Storage)?
            .ok_or(ConversationOperationError::ConversationNotFound)?;
        self.store
            .find_member(id, actor)
            .await
            .map_err(ConversationOperationError::Storage)?
            .ok_or(ConversationOperationError::NotMember)?;
        Ok(conversation)
    }
}

async fn group_actor<S: ConversationStore>(
    store: &S,
    id: ConversationId,
    actor: UserId,
) -> Result<(Conversation, ConversationMember), ConversationOperationError> {
    let conversation = store
        .find_conversation(id)
        .await
        .map_err(ConversationOperationError::Storage)?
        .ok_or(ConversationOperationError::ConversationNotFound)?;
    if conversation.kind() == ConversationKind::Direct {
        return Err(ConversationOperationError::DirectOperationNotSupported);
    }
    let member = store
        .find_member(id, actor)
        .await
        .map_err(ConversationOperationError::Storage)?
        .ok_or(ConversationOperationError::NotMember)?;
    Ok((conversation, member))
}
fn manager(member: &ConversationMember) -> bool {
    matches!(
        member.role(),
        MembershipRole::Group(GroupRole::Owner | GroupRole::Admin)
    )
}

/// Adds a Group Member.
pub struct AddGroupMember<'a, I, S> {
    identities: &'a I,
    store: &'a S,
}
impl<'a, I: IdentityStore, S: ConversationStore> AddGroupMember<'a, I, S> {
    /// Wires identity validation and persistence.
    pub fn new(identities: &'a I, store: &'a S) -> Self {
        Self { identities, store }
    }
    /// Adds a participant as Member, or reports an existing membership.
    pub async fn execute(
        &self,
        actor: UserId,
        id: ConversationId,
        target: UserId,
        now: OffsetDateTime,
    ) -> Result<AddMemberStoreOutcome, ConversationOperationError> {
        let (_, member) = group_actor(self.store, id, actor).await?;
        if !manager(&member) {
            return Err(ConversationOperationError::InsufficientPermissions);
        }
        participant(self.identities, target).await?;
        self.store
            .add_group_member(id, actor, target, now)
            .await
            .map_err(ConversationOperationError::Storage)
    }
}

/// Removes another non-Owner Group member.
pub struct RemoveGroupMember<'a, S> {
    store: &'a S,
}
impl<'a, S: ConversationStore> RemoveGroupMember<'a, S> {
    /// Wires persistence.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Removes a Member or Admin according to Group permissions.
    pub async fn execute(
        &self,
        actor: UserId,
        id: ConversationId,
        target: UserId,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationOperationError> {
        if actor == target {
            return Err(ConversationOperationError::InsufficientPermissions);
        }
        let (_, member) = group_actor(self.store, id, actor).await?;
        if !manager(&member) {
            return Err(ConversationOperationError::InsufficientPermissions);
        }
        let target_member = self
            .store
            .find_member(id, target)
            .await
            .map_err(ConversationOperationError::Storage)?
            .ok_or(ConversationOperationError::TargetNotMember)?;
        if target_member.role() == MembershipRole::Group(GroupRole::Owner) {
            return Err(ConversationOperationError::OwnerProtected);
        }
        self.store
            .remove_group_member(id, actor, target, now)
            .await
            .map_err(ConversationOperationError::Storage)
    }
}

/// Changes a non-Owner Group member between Admin and Member.
pub struct SetGroupMemberRole<'a, S> {
    store: &'a S,
}
impl<'a, S: ConversationStore> SetGroupMemberRole<'a, S> {
    /// Wires persistence.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Applies an idempotent Admin/Member role change.
    pub async fn execute(
        &self,
        actor: UserId,
        id: ConversationId,
        target: UserId,
        role: GroupRole,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationOperationError> {
        if role == GroupRole::Owner {
            return Err(ConversationOperationError::OwnerRoleNotAssignable);
        }
        let (_, member) = group_actor(self.store, id, actor).await?;
        if !manager(&member) {
            return Err(ConversationOperationError::InsufficientPermissions);
        }
        let target_member = self
            .store
            .find_member(id, target)
            .await
            .map_err(ConversationOperationError::Storage)?
            .ok_or(ConversationOperationError::TargetNotMember)?;
        if target_member.role() == MembershipRole::Group(GroupRole::Owner) {
            return Err(ConversationOperationError::OwnerProtected);
        }
        self.store
            .set_group_member_role(id, actor, target, role, now)
            .await
            .map_err(ConversationOperationError::Storage)
    }
}

/// Transfers Group ownership.
pub struct TransferGroupOwnership<'a, S> {
    store: &'a S,
}
impl<'a, S: ConversationStore> TransferGroupOwnership<'a, S> {
    /// Wires persistence.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Makes the target Owner and the prior Owner Admin atomically.
    pub async fn execute(
        &self,
        actor: UserId,
        id: ConversationId,
        target: UserId,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationOperationError> {
        let (_, member) = group_actor(self.store, id, actor).await?;
        if member.role() != MembershipRole::Group(GroupRole::Owner) {
            return Err(ConversationOperationError::InsufficientPermissions);
        }
        if self
            .store
            .find_member(id, target)
            .await
            .map_err(ConversationOperationError::Storage)?
            .is_none()
        {
            return Err(ConversationOperationError::TargetNotMember);
        }
        self.store
            .transfer_group_ownership(id, actor, target, now)
            .await
            .map_err(ConversationOperationError::Storage)
    }
}

/// Leaves a Group.
pub struct LeaveGroupConversation<'a, S> {
    store: &'a S,
}
impl<'a, S: ConversationStore> LeaveGroupConversation<'a, S> {
    /// Wires persistence.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Leaves, or deletes an owner-only Group.
    pub async fn execute(
        &self,
        actor: UserId,
        id: ConversationId,
        now: OffsetDateTime,
    ) -> Result<LeaveStoreOutcome, ConversationOperationError> {
        group_actor(self.store, id, actor).await?;
        match self
            .store
            .leave_group(id, actor, now)
            .await
            .map_err(ConversationOperationError::Storage)?
        {
            LeaveStoreOutcome::OwnershipTransferRequired => {
                Err(ConversationOperationError::OwnershipTransferRequired)
            }
            LeaveStoreOutcome::NotMember => Err(ConversationOperationError::NotMember),
            LeaveStoreOutcome::Direct => {
                Err(ConversationOperationError::DirectOperationNotSupported)
            }
            value => Ok(value),
        }
    }
}

/// Renames a Group.
pub struct RenameGroupConversation<'a, S> {
    store: &'a S,
}
impl<'a, S: ConversationStore> RenameGroupConversation<'a, S> {
    /// Wires persistence.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Applies an idempotent title rename.
    pub async fn execute(
        &self,
        actor: UserId,
        id: ConversationId,
        title: impl Into<String>,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationOperationError> {
        let (_, member) = group_actor(self.store, id, actor).await?;
        if !manager(&member) {
            return Err(ConversationOperationError::InsufficientPermissions);
        }
        let title = GroupTitle::new(title).map_err(ConversationOperationError::InvalidTitle)?;
        self.store
            .rename_group(id, actor, title.as_str(), now)
            .await
            .map_err(ConversationOperationError::Storage)
    }
}
