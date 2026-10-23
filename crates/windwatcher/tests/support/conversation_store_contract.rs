use time::OffsetDateTime;
use windwatcher::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationStore,
    GroupRole, GroupTitle, LeaveStoreOutcome, MembershipRole, StartDirectStoreOutcome, UserId,
};

/// Exercises adapter-independent conversation persistence semantics.
pub async fn run<S: ConversationStore>(store: &S, owner: UserId, member: UserId) {
    let now = OffsetDateTime::UNIX_EPOCH;
    let direct = Conversation::new_direct(ConversationId::new(), now);
    let direct_id = match store
        .get_or_create_direct(&direct, owner, member)
        .await
        .unwrap()
    {
        StartDirectStoreOutcome::Created(conversation)
        | StartDirectStoreOutcome::Existing(conversation) => conversation.id(),
    };
    let reverse = Conversation::new_direct(ConversationId::new(), now);
    assert_eq!(
        match store
            .get_or_create_direct(&reverse, member, owner)
            .await
            .unwrap()
        {
            StartDirectStoreOutcome::Created(conversation)
            | StartDirectStoreOutcome::Existing(conversation) => conversation.id(),
        },
        direct_id
    );
    assert_eq!(
        store
            .find_member(direct_id, owner)
            .await
            .unwrap()
            .unwrap()
            .role(),
        MembershipRole::Direct
    );

    let self_direct = Conversation::new_direct(ConversationId::new(), now);
    let self_id = match store
        .get_or_create_direct(&self_direct, owner, owner)
        .await
        .unwrap()
    {
        StartDirectStoreOutcome::Created(conversation)
        | StartDirectStoreOutcome::Existing(conversation) => conversation.id(),
    };
    assert_eq!(
        store
            .find_member(self_id, owner)
            .await
            .unwrap()
            .unwrap()
            .role(),
        MembershipRole::Direct
    );

    let group = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("group").unwrap(),
        now,
    );
    store
        .create_group(&group, owner, &[member, member])
        .await
        .unwrap();
    assert_eq!(
        store
            .find_member(group.id(), owner)
            .await
            .unwrap()
            .unwrap()
            .role(),
        MembershipRole::Group(GroupRole::Owner)
    );
    assert_eq!(
        store
            .find_member(group.id(), member)
            .await
            .unwrap()
            .unwrap()
            .role(),
        MembershipRole::Group(GroupRole::Member)
    );
    assert_eq!(
        store
            .add_group_member(group.id(), owner, member, now)
            .await
            .unwrap(),
        AddMemberStoreOutcome::AlreadyMember
    );
    assert_eq!(
        store
            .set_group_member_role(group.id(), owner, member, GroupRole::Admin, now)
            .await
            .unwrap(),
        ChangeStoreOutcome::Changed
    );
    assert_eq!(
        store
            .transfer_group_ownership(group.id(), owner, member, now)
            .await
            .unwrap(),
        ChangeStoreOutcome::Changed
    );
    assert_eq!(
        store.leave_group(group.id(), owner, now).await.unwrap(),
        LeaveStoreOutcome::Left
    );
    assert_eq!(
        store.leave_group(group.id(), member, now).await.unwrap(),
        LeaveStoreOutcome::ConversationDeleted
    );
    assert!(store.find_conversation(group.id()).await.unwrap().is_none());
}
