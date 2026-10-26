use time::OffsetDateTime;

use crate::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationKind,
    ConversationMember, ConversationStore, ConversationStoreError, GroupRole, GroupTitle,
    GroupTitleError, LeaveStoreOutcome, MembershipRole, StartDirectStoreOutcome, UserId,
};

/// Whether starting a Direct created it or found its existing semantic pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartDirectConversationOutcome {
    /// A new Direct was created.
    Created(Conversation),
    /// The Direct already existed.
    Existing(Conversation),
}

/// Result of adding a Group member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddGroupMemberOutcome {
    /// A new member was added.
    Added,
    /// The user was already a member.
    AlreadyMember,
}

/// Result of an idempotent Group mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationOutcome {
    /// A meaningful mutation was persisted.
    Changed,
    /// The requested state already existed.
    Unchanged,
}

/// Result of leaving a Group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaveGroupOutcome {
    /// The member left the Group.
    Left,
    /// The owner-only Group was deleted.
    ConversationDeleted,
}

macro_rules! application_error {
    ($documentation:literal, $name:ident { $($variants:tt)* }) => {
        #[doc = $documentation]
        #[allow(missing_docs)]
        #[derive(Debug, thiserror::Error)]
        #[non_exhaustive]
        pub enum $name { $($variants)* }
    };
}
application_error!("Failures starting a Direct conversation.", StartDirectConversationError {
    #[error("conversation participant unavailable")] ParticipantUnavailable(UserId),
    #[error("conversation storage failed")] Storage(#[source] ConversationStoreError),
});
application_error!("Failures creating a Group conversation.", CreateGroupConversationError {
    #[error("conversation participant unavailable")] ParticipantUnavailable(UserId),
    #[error("invalid group title")] InvalidTitle(#[source] GroupTitleError),
    #[error("conversation storage failed")] Storage(#[source] ConversationStoreError),
});
application_error!("Failures loading a conversation.", GetConversationError {
    #[error("conversation not found")] ConversationNotFound,
    #[error("actor is not a conversation member")] NotMember,
    #[error("conversation storage failed")] Storage(#[source] ConversationStoreError),
});
application_error!("Failures adding a Group member.", AddGroupMemberError {
    #[error("conversation not found")] ConversationNotFound,
    #[error("actor is not a conversation member")] NotMember,
    #[error("insufficient group permissions")] InsufficientPermissions,
    #[error("operation is not supported for Direct conversations")] DirectOperationNotSupported,
    #[error("conversation participant unavailable")] ParticipantUnavailable(UserId),
    #[error("conversation storage failed")] Storage(#[source] ConversationStoreError),
});
application_error!("Failures removing a Group member.", RemoveGroupMemberError {
    #[error("conversation not found")] ConversationNotFound,
    #[error("actor is not a conversation member")] NotMember,
    #[error("insufficient group permissions")] InsufficientPermissions,
    #[error("operation is not supported for Direct conversations")] DirectOperationNotSupported,
    #[error("group owner is protected")] OwnerProtected,
    #[error("target is not a group member")] TargetNotMember,
    #[error("conversation storage failed")] Storage(#[source] ConversationStoreError),
});
application_error!("Failures changing a Group member role.", SetGroupMemberRoleError {
    #[error("conversation not found")] ConversationNotFound,
    #[error("actor is not a conversation member")] NotMember,
    #[error("insufficient group permissions")] InsufficientPermissions,
    #[error("operation is not supported for Direct conversations")] DirectOperationNotSupported,
    #[error("group owner is protected")] OwnerProtected,
    #[error("target is not a group member")] TargetNotMember,
    #[error("owner role must be changed through ownership transfer")] OwnerRoleNotAssignable,
    #[error("conversation storage failed")] Storage(#[source] ConversationStoreError),
});
application_error!("Failures transferring Group ownership.", TransferGroupOwnershipError {
    #[error("conversation not found")] ConversationNotFound,
    #[error("actor is not a conversation member")] NotMember,
    #[error("insufficient group permissions")] InsufficientPermissions,
    #[error("operation is not supported for Direct conversations")] DirectOperationNotSupported,
    #[error("target is not a group member")] TargetNotMember,
    #[error("conversation storage failed")] Storage(#[source] ConversationStoreError),
});
application_error!("Failures leaving a Group conversation.", LeaveGroupConversationError {
    #[error("conversation not found")] ConversationNotFound,
    #[error("actor is not a conversation member")] NotMember,
    #[error("operation is not supported for Direct conversations")] DirectOperationNotSupported,
    #[error("ownership transfer is required before the owner leaves")] OwnershipTransferRequired,
    #[error("conversation storage failed")] Storage(#[source] ConversationStoreError),
});
application_error!("Failures renaming a Group conversation.", RenameGroupConversationError {
    #[error("conversation not found")] ConversationNotFound,
    #[error("actor is not a conversation member")] NotMember,
    #[error("insufficient group permissions")] InsufficientPermissions,
    #[error("operation is not supported for Direct conversations")] DirectOperationNotSupported,
    #[error("invalid group title")] InvalidTitle(#[source] GroupTitleError),
    #[error("conversation storage failed")] Storage(#[source] ConversationStoreError),
});

/// Starts or finds an immutable Direct conversation.
pub struct StartDirectConversation<'a, S> {
    store: &'a S,
}
impl<'a, S: ConversationStore> StartDirectConversation<'a, S> {
    /// Uses semantic persistence, including atomic participant eligibility.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Gets or creates the unordered actor/participant pair; self Directs are valid.
    pub async fn execute(
        &self,
        actor: UserId,
        participant_id: UserId,
        now: OffsetDateTime,
    ) -> Result<StartDirectConversationOutcome, StartDirectConversationError> {
        match self
            .store
            .get_or_create_direct(
                &Conversation::new_direct(ConversationId::new(), now),
                actor,
                participant_id,
            )
            .await
            .map_err(|error| match error {
                ConversationStoreError::ParticipantUnavailable(id) => {
                    StartDirectConversationError::ParticipantUnavailable(id)
                }
                error => StartDirectConversationError::Storage(error),
            })? {
            StartDirectStoreOutcome::Created(conversation) => {
                Ok(StartDirectConversationOutcome::Created(conversation))
            }
            StartDirectStoreOutcome::Existing(conversation) => {
                Ok(StartDirectConversationOutcome::Existing(conversation))
            }
        }
    }
}

/// Creates a Group with one Owner and zero or more initial Members.
pub struct CreateGroupConversation<'a, S> {
    store: &'a S,
}
impl<'a, S: ConversationStore> CreateGroupConversation<'a, S> {
    /// Uses semantic persistence, including atomic participant eligibility.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Creates the Group atomically; duplicate participant IDs are a set.
    pub async fn execute(
        &self,
        actor: UserId,
        title: impl Into<String>,
        participants: &[UserId],
        now: OffsetDateTime,
    ) -> Result<Conversation, CreateGroupConversationError> {
        let title = GroupTitle::new(title).map_err(CreateGroupConversationError::InvalidTitle)?;
        let mut unique = Vec::new();
        for id in participants {
            if *id != actor && !unique.contains(id) {
                unique.push(*id);
            }
        }
        let conversation = Conversation::new_group(ConversationId::new(), title, now);
        self.store
            .create_group(&conversation, actor, &unique)
            .await
            .map_err(|error| match error {
                ConversationStoreError::ParticipantUnavailable(id) => {
                    CreateGroupConversationError::ParticipantUnavailable(id)
                }
                error => CreateGroupConversationError::Storage(error),
            })?;
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
    ) -> Result<Conversation, GetConversationError> {
        let conversation = self
            .store
            .find_conversation(id)
            .await
            .map_err(GetConversationError::Storage)?
            .ok_or(GetConversationError::ConversationNotFound)?;
        self.store
            .find_member(id, actor)
            .await
            .map_err(GetConversationError::Storage)?
            .ok_or(GetConversationError::NotMember)?;
        Ok(conversation)
    }
}

#[derive(Debug)]
enum GroupActorError {
    ConversationNotFound,
    NotMember,
    DirectOperationNotSupported,
    Storage(ConversationStoreError),
}
async fn group_actor<S: ConversationStore>(
    store: &S,
    id: ConversationId,
    actor: UserId,
) -> Result<ConversationMember, GroupActorError> {
    let conversation = store
        .find_conversation(id)
        .await
        .map_err(GroupActorError::Storage)?
        .ok_or(GroupActorError::ConversationNotFound)?;
    let member = store
        .find_member(id, actor)
        .await
        .map_err(GroupActorError::Storage)?
        .ok_or(GroupActorError::NotMember)?;
    if conversation.kind() == ConversationKind::Direct {
        return Err(GroupActorError::DirectOperationNotSupported);
    }
    Ok(member)
}
fn manager(member: &ConversationMember) -> bool {
    matches!(
        member.role(),
        MembershipRole::Group(GroupRole::Owner | GroupRole::Admin)
    )
}
macro_rules! group_error {
    ($error:expr, $name:ident) => {
        $error.map_err(|error| match error {
            GroupActorError::ConversationNotFound => $name::ConversationNotFound,
            GroupActorError::NotMember => $name::NotMember,
            GroupActorError::DirectOperationNotSupported => $name::DirectOperationNotSupported,
            GroupActorError::Storage(error) => $name::Storage(error),
        })
    };
}

/// Adds a Group Member.
pub struct AddGroupMember<'a, S> {
    store: &'a S,
}
impl<'a, S: ConversationStore> AddGroupMember<'a, S> {
    /// Uses semantic persistence, including atomic participant eligibility.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Adds a participant as Member, or reports an existing membership.
    pub async fn execute(
        &self,
        actor: UserId,
        id: ConversationId,
        target: UserId,
        now: OffsetDateTime,
    ) -> Result<AddGroupMemberOutcome, AddGroupMemberError> {
        let member = group_error!(
            group_actor(self.store, id, actor).await,
            AddGroupMemberError
        )?;
        if !manager(&member) {
            return Err(AddGroupMemberError::InsufficientPermissions);
        }
        match self
            .store
            .add_group_member(id, actor, target, now)
            .await
            .map_err(|error| match error {
                ConversationStoreError::ParticipantUnavailable(id) => {
                    AddGroupMemberError::ParticipantUnavailable(id)
                }
                error => AddGroupMemberError::Storage(error),
            })? {
            AddMemberStoreOutcome::Added => Ok(AddGroupMemberOutcome::Added),
            AddMemberStoreOutcome::AlreadyMember => Ok(AddGroupMemberOutcome::AlreadyMember),
            AddMemberStoreOutcome::NotAuthorized => {
                Err(AddGroupMemberError::InsufficientPermissions)
            }
        }
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
    ) -> Result<MutationOutcome, RemoveGroupMemberError> {
        let member = group_error!(
            group_actor(self.store, id, actor).await,
            RemoveGroupMemberError
        )?;
        if actor == target {
            return Err(RemoveGroupMemberError::InsufficientPermissions);
        }
        if !manager(&member) {
            return Err(RemoveGroupMemberError::InsufficientPermissions);
        }
        let target_member = self
            .store
            .find_member(id, target)
            .await
            .map_err(RemoveGroupMemberError::Storage)?
            .ok_or(RemoveGroupMemberError::TargetNotMember)?;
        if target_member.role() == MembershipRole::Group(GroupRole::Owner) {
            return Err(RemoveGroupMemberError::OwnerProtected);
        }
        match self
            .store
            .remove_group_member(id, actor, target, now)
            .await
            .map_err(RemoveGroupMemberError::Storage)?
        {
            ChangeStoreOutcome::Changed => Ok(MutationOutcome::Changed),
            ChangeStoreOutcome::Unchanged => Ok(MutationOutcome::Unchanged),
            ChangeStoreOutcome::NotAuthorized => {
                Err(RemoveGroupMemberError::InsufficientPermissions)
            }
        }
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
    ) -> Result<MutationOutcome, SetGroupMemberRoleError> {
        if role == GroupRole::Owner {
            return Err(SetGroupMemberRoleError::OwnerRoleNotAssignable);
        }
        let member = group_error!(
            group_actor(self.store, id, actor).await,
            SetGroupMemberRoleError
        )?;
        if !manager(&member) {
            return Err(SetGroupMemberRoleError::InsufficientPermissions);
        }
        let target_member = self
            .store
            .find_member(id, target)
            .await
            .map_err(SetGroupMemberRoleError::Storage)?
            .ok_or(SetGroupMemberRoleError::TargetNotMember)?;
        if target_member.role() == MembershipRole::Group(GroupRole::Owner) {
            return Err(SetGroupMemberRoleError::OwnerProtected);
        }
        match self
            .store
            .set_group_member_role(id, actor, target, role, now)
            .await
            .map_err(SetGroupMemberRoleError::Storage)?
        {
            ChangeStoreOutcome::Changed => Ok(MutationOutcome::Changed),
            ChangeStoreOutcome::Unchanged => Ok(MutationOutcome::Unchanged),
            ChangeStoreOutcome::NotAuthorized => {
                Err(SetGroupMemberRoleError::InsufficientPermissions)
            }
        }
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
    ) -> Result<MutationOutcome, TransferGroupOwnershipError> {
        let member = group_error!(
            group_actor(self.store, id, actor).await,
            TransferGroupOwnershipError
        )?;
        if member.role() != MembershipRole::Group(GroupRole::Owner) {
            return Err(TransferGroupOwnershipError::InsufficientPermissions);
        }
        if self
            .store
            .find_member(id, target)
            .await
            .map_err(TransferGroupOwnershipError::Storage)?
            .is_none()
        {
            return Err(TransferGroupOwnershipError::TargetNotMember);
        }
        match self
            .store
            .transfer_group_ownership(id, actor, target, now)
            .await
            .map_err(TransferGroupOwnershipError::Storage)?
        {
            ChangeStoreOutcome::Changed => Ok(MutationOutcome::Changed),
            ChangeStoreOutcome::Unchanged => Ok(MutationOutcome::Unchanged),
            ChangeStoreOutcome::NotAuthorized => {
                Err(TransferGroupOwnershipError::InsufficientPermissions)
            }
        }
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
    ) -> Result<LeaveGroupOutcome, LeaveGroupConversationError> {
        group_error!(
            group_actor(self.store, id, actor).await,
            LeaveGroupConversationError
        )?;
        match self
            .store
            .leave_group(id, actor, now)
            .await
            .map_err(LeaveGroupConversationError::Storage)?
        {
            LeaveStoreOutcome::Left => Ok(LeaveGroupOutcome::Left),
            LeaveStoreOutcome::ConversationDeleted => Ok(LeaveGroupOutcome::ConversationDeleted),
            LeaveStoreOutcome::OwnershipTransferRequired => {
                Err(LeaveGroupConversationError::OwnershipTransferRequired)
            }
            LeaveStoreOutcome::NotMember => Err(LeaveGroupConversationError::NotMember),
            LeaveStoreOutcome::Direct => {
                Err(LeaveGroupConversationError::DirectOperationNotSupported)
            }
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
    ) -> Result<MutationOutcome, RenameGroupConversationError> {
        let member = group_error!(
            group_actor(self.store, id, actor).await,
            RenameGroupConversationError
        )?;
        if !manager(&member) {
            return Err(RenameGroupConversationError::InsufficientPermissions);
        }
        let title = GroupTitle::new(title).map_err(RenameGroupConversationError::InvalidTitle)?;
        match self
            .store
            .rename_group(id, actor, &title, now)
            .await
            .map_err(RenameGroupConversationError::Storage)?
        {
            ChangeStoreOutcome::Changed => Ok(MutationOutcome::Changed),
            ChangeStoreOutcome::Unchanged => Ok(MutationOutcome::Unchanged),
            ChangeStoreOutcome::NotAuthorized => {
                Err(RenameGroupConversationError::InsufficientPermissions)
            }
        }
    }
}
