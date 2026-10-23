use time::OffsetDateTime;
use windwatcher::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationStore,
    ConversationStoreError, GroupRole, GroupTitle, LeaveStoreOutcome, MembershipRole,
    StartDirectStoreOutcome, UserId,
};

/// Exercises adapter-independent conversation persistence semantics.
pub async fn run<S: ConversationStore>(
    store: &S,
    owner: UserId,
    admin: UserId,
    member: UserId,
    outsider: UserId,
) {
    let now = OffsetDateTime::UNIX_EPOCH;
    let later = now + time::Duration::seconds(1);
    let invalid_direct = Conversation::new_direct(ConversationId::new(), now);
    assert!(matches!(
        store.create_group(&invalid_direct, owner, &[]).await,
        Err(ConversationStoreError::InvalidConversationKind { .. })
    ));
    let invalid_group = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("not a Direct").unwrap(),
        now,
    );
    assert!(matches!(
        store
            .get_or_create_direct(&invalid_group, owner, member)
            .await,
        Err(ConversationStoreError::InvalidConversationKind { .. })
    ));
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
    assert_eq!(
        store
            .add_group_member(direct_id, owner, outsider, later)
            .await
            .unwrap(),
        AddMemberStoreOutcome::NotAuthorized
    );
    assert_eq!(
        store.leave_group(direct_id, owner, later).await.unwrap(),
        LeaveStoreOutcome::Direct
    );
    assert_eq!(
        store
            .rename_group(
                direct_id,
                owner,
                &GroupTitle::new("not a Direct rename").unwrap(),
                later,
            )
            .await
            .unwrap(),
        ChangeStoreOutcome::NotAuthorized
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
        .create_group(&group, owner, &[admin, member, member])
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
            .set_group_member_role(group.id(), owner, admin, GroupRole::Admin, now)
            .await
            .unwrap(),
        ChangeStoreOutcome::Changed
    );
    assert!(matches!(
        store
            .set_group_member_role(group.id(), owner, member, GroupRole::Owner, later)
            .await,
        Err(ConversationStoreError::InvalidGroupRole)
    ));
    assert_eq!(
        store
            .add_group_member(group.id(), member, outsider, now)
            .await
            .unwrap(),
        AddMemberStoreOutcome::NotAuthorized
    );
    assert_eq!(
        store
            .add_group_member(group.id(), admin, outsider, now)
            .await
            .unwrap(),
        AddMemberStoreOutcome::Added
    );
    assert_eq!(
        store
            .remove_group_member(group.id(), member, outsider, now)
            .await
            .unwrap(),
        ChangeStoreOutcome::NotAuthorized
    );
    assert_eq!(
        store
            .remove_group_member(group.id(), admin, outsider, now)
            .await
            .unwrap(),
        ChangeStoreOutcome::Changed
    );
    assert_eq!(
        store
            .add_group_member(group.id(), owner, member, now)
            .await
            .unwrap(),
        AddMemberStoreOutcome::AlreadyMember
    );
    let before_rename = store
        .find_conversation(group.id())
        .await
        .unwrap()
        .unwrap()
        .updated_at();
    let renamed = GroupTitle::new("renamed group").unwrap();
    assert_eq!(
        store
            .rename_group(group.id(), admin, &renamed, later)
            .await
            .unwrap(),
        ChangeStoreOutcome::Changed
    );
    let after_rename = store
        .find_conversation(group.id())
        .await
        .unwrap()
        .unwrap()
        .updated_at();
    assert!(after_rename > before_rename);
    assert_eq!(
        store
            .rename_group(group.id(), admin, &renamed, later)
            .await
            .unwrap(),
        ChangeStoreOutcome::Unchanged
    );
    assert_eq!(
        store
            .find_conversation(group.id())
            .await
            .unwrap()
            .unwrap()
            .updated_at(),
        after_rename
    );
    assert_eq!(
        store
            .set_group_member_role(group.id(), admin, member, GroupRole::Admin, now)
            .await
            .unwrap(),
        ChangeStoreOutcome::Changed
    );
    assert_eq!(
        store
            .transfer_group_ownership(group.id(), admin, member, now)
            .await
            .unwrap(),
        ChangeStoreOutcome::NotAuthorized
    );
    assert_eq!(
        store.leave_group(group.id(), owner, now).await.unwrap(),
        LeaveStoreOutcome::OwnershipTransferRequired
    );
    assert_eq!(
        store
            .transfer_group_ownership(group.id(), owner, admin, now)
            .await
            .unwrap(),
        ChangeStoreOutcome::Changed
    );
    assert_eq!(
        store
            .find_member(group.id(), admin)
            .await
            .unwrap()
            .unwrap()
            .role(),
        MembershipRole::Group(GroupRole::Owner)
    );
    assert_eq!(
        store
            .find_member(group.id(), owner)
            .await
            .unwrap()
            .unwrap()
            .role(),
        MembershipRole::Group(GroupRole::Admin)
    );
    assert_eq!(
        store.leave_group(group.id(), member, now).await.unwrap(),
        LeaveStoreOutcome::Left
    );
    assert_eq!(
        store.leave_group(group.id(), owner, now).await.unwrap(),
        LeaveStoreOutcome::Left
    );
    assert_eq!(
        store.leave_group(group.id(), admin, now).await.unwrap(),
        LeaveStoreOutcome::ConversationDeleted
    );
    assert!(store.find_conversation(group.id()).await.unwrap().is_none());
}
