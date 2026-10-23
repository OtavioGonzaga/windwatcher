use time::OffsetDateTime;
use windwatcher::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationMember,
    ConversationOperationError, ConversationStore, ConversationStoreError, CreateGroupConversation,
    GroupRole, IdentityStore, IdentityStoreError, LeaveStoreOutcome, LocalAuthentication,
    PasswordHash, StartDirectConversation, StartDirectStoreOutcome, User, UserId, Username,
};

struct IdentityDouble {
    user: Option<User>,
}
impl IdentityStore for IdentityDouble {
    async fn create_local_user(
        &self,
        _: &User,
        _: &PasswordHash,
    ) -> Result<(), IdentityStoreError> {
        unreachable!()
    }
    async fn find_local_auth_by_username(
        &self,
        _: &Username,
    ) -> Result<Option<LocalAuthentication>, IdentityStoreError> {
        unreachable!()
    }
    async fn find_user(&self, _: UserId) -> Result<Option<User>, IdentityStoreError> {
        Ok(self.user.clone())
    }
    async fn update_user(&self, _: &User) -> Result<(), IdentityStoreError> {
        unreachable!()
    }
}

struct ConversationDouble;
impl ConversationStore for ConversationDouble {
    async fn get_or_create_direct(
        &self,
        _: &Conversation,
        _: UserId,
        _: UserId,
    ) -> Result<StartDirectStoreOutcome, ConversationStoreError> {
        unreachable!()
    }
    async fn create_group(
        &self,
        _: &Conversation,
        _: UserId,
        _: &[UserId],
    ) -> Result<(), ConversationStoreError> {
        unreachable!()
    }
    async fn find_conversation(
        &self,
        _: ConversationId,
    ) -> Result<Option<Conversation>, ConversationStoreError> {
        unreachable!()
    }
    async fn find_member(
        &self,
        _: ConversationId,
        _: UserId,
    ) -> Result<Option<ConversationMember>, ConversationStoreError> {
        unreachable!()
    }
    async fn add_group_member(
        &self,
        _: ConversationId,
        _: UserId,
        _: UserId,
        _: OffsetDateTime,
    ) -> Result<AddMemberStoreOutcome, ConversationStoreError> {
        unreachable!()
    }
    async fn remove_group_member(
        &self,
        _: ConversationId,
        _: UserId,
        _: UserId,
        _: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        unreachable!()
    }
    async fn set_group_member_role(
        &self,
        _: ConversationId,
        _: UserId,
        _: UserId,
        _: GroupRole,
        _: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        unreachable!()
    }
    async fn transfer_group_ownership(
        &self,
        _: ConversationId,
        _: UserId,
        _: UserId,
        _: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        unreachable!()
    }
    async fn leave_group(
        &self,
        _: ConversationId,
        _: UserId,
        _: OffsetDateTime,
    ) -> Result<LeaveStoreOutcome, ConversationStoreError> {
        unreachable!()
    }
    async fn rename_group(
        &self,
        _: ConversationId,
        _: UserId,
        _: &str,
        _: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        unreachable!()
    }
}

#[tokio::test]
async fn creation_validation_uses_ports_before_persistence() {
    let now = OffsetDateTime::UNIX_EPOCH;
    let missing = IdentityDouble { user: None };
    let store = ConversationDouble;
    assert!(matches!(
        StartDirectConversation::new(&missing, &store)
            .execute(UserId::new(), UserId::new(), now)
            .await,
        Err(ConversationOperationError::ParticipantNotFound(_))
    ));

    let id = UserId::new();
    let identity = IdentityDouble {
        user: Some(User::new(id, "alice".parse().unwrap(), now)),
    };
    assert!(matches!(
        CreateGroupConversation::new(&identity, &store)
            .execute(id, " \t", &[], now)
            .await,
        Err(ConversationOperationError::InvalidTitle(_))
    ));
}
