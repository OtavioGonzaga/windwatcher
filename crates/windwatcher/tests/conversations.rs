use time::OffsetDateTime;
use windwatcher::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationMember,
    ConversationStore, ConversationStoreError, CreateGroupConversation, GroupRole, GroupTitle,
    LeaveStoreOutcome, StartDirectConversation, StartDirectStoreOutcome, UserId,
};

struct ConversationDouble;
impl ConversationStore for ConversationDouble {
    async fn get_or_create_direct(
        &self,
        _: &Conversation,
        actor: UserId,
        _: UserId,
    ) -> Result<StartDirectStoreOutcome, ConversationStoreError> {
        Err(ConversationStoreError::ParticipantUnavailable(actor))
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
        _: &GroupTitle,
        _: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        unreachable!()
    }
}

#[tokio::test]
async fn creation_validation_and_atomic_participant_rejections_are_mapped() {
    let now = OffsetDateTime::UNIX_EPOCH;
    let store = ConversationDouble;
    assert!(matches!(
        StartDirectConversation::new(&store)
            .execute(UserId::new(), UserId::new(), now)
            .await,
        Err(windwatcher::StartDirectConversationError::ParticipantUnavailable(_))
    ));

    let id = UserId::new();
    assert!(matches!(
        CreateGroupConversation::new(&store)
            .execute(id, " \t", &[], now)
            .await,
        Err(windwatcher::CreateGroupConversationError::InvalidTitle(_))
    ));
}
