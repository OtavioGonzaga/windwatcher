use time::{Duration, OffsetDateTime};
use windwatcher::*;

/// Consumer projections and cursor semantics shared by future official adapters.
pub async fn run<S: MessagingQueryStore + ConversationStore + MessageStore + IdentityStore>(
    store: &S,
    owner: User,
    mut peer: User,
    outsider: User,
) {
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
        + Duration::microseconds(123456);
    let actor = owner.id();
    assert!(
        ListConversations::new(store)
            .execute(actor, PageRequest::default())
            .await
            .unwrap()
            .items()
            .is_empty()
    );
    let users = FindUsers::new(store)
        .execute(&owner.username().as_str().to_uppercase())
        .await
        .unwrap();
    assert_eq!(users.items()[0].id, actor);
    assert!(!users.has_more());
    assert!(
        FindUsers::new(store)
            .execute("missing_name")
            .await
            .unwrap()
            .items()
            .is_empty()
    );
    assert!(matches!(
        FindUsers::new(store).execute("!").await,
        Err(MessagingQueryError::InvalidUsername(_))
    ));
    let mut ids = Vec::new();
    for n in 0..5 {
        let group = Conversation::new_group(
            ConversationId::new(),
            GroupTitle::new(format!("group {n}")).unwrap(),
            now + Duration::seconds(n),
        );
        store
            .create_group(&group, actor, &[peer.id()])
            .await
            .unwrap();
        ids.push(group.id());
    }
    let limit = PageLimit::new(2).unwrap();
    let first = ListConversations::new(store)
        .execute(actor, PageRequest::new(limit))
        .await
        .unwrap();
    let second = ListConversations::new(store)
        .execute(
            actor,
            PageRequest::new(limit).with_cursor(first.next_cursor().unwrap()),
        )
        .await
        .unwrap();
    let third = ListConversations::new(store)
        .execute(
            actor,
            PageRequest::new(limit).with_cursor(second.next_cursor().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(
        (
            first.items().len(),
            second.items().len(),
            third.items().len()
        ),
        (2, 2, 1)
    );
    let traversed = first
        .items()
        .iter()
        .chain(second.items())
        .chain(third.items())
        .map(|s| s.conversation.id())
        .collect::<Vec<_>>();
    assert_eq!(traversed, ids.iter().rev().copied().collect::<Vec<_>>());
    assert!(matches!(
        ListConversations::new(store)
            .execute(
                peer.id(),
                PageRequest::new(limit).with_cursor(first.next_cursor().unwrap())
            )
            .await,
        Err(MessagingQueryError::InvalidCursor(CursorError::WrongScope))
    ));
    let policy = MessagePolicy::default();
    let message = SendMessage::new(store, &policy)
        .execute(
            peer.id(),
            ids[0],
            "new activity",
            None,
            now + Duration::seconds(10),
        )
        .await
        .unwrap();
    let detail = GetConversationSummary::new(store)
        .execute(actor, ids[0])
        .await
        .unwrap();
    assert_eq!(detail.activity_at, message.created_at());
    assert_eq!(detail.unread_count, 1);
    assert_eq!(detail.membership.last_read_message_id(), None);
    assert_eq!(detail.last_message.as_ref().unwrap(), &message);
    assert!(matches!(
        GetConversationSummary::new(store)
            .execute(outsider.id(), ids[0])
            .await,
        Err(MessagingQueryError::NotMember)
    ));
    let refreshed = ListConversations::new(store)
        .execute(actor, PageRequest::new(limit))
        .await
        .unwrap();
    assert_eq!(refreshed.items()[0].conversation.id(), ids[0]);
    let continued = ListConversations::new(store)
        .execute(
            actor,
            PageRequest::new(limit).with_cursor(first.next_cursor().unwrap()),
        )
        .await
        .unwrap();
    assert!(continued.items().iter().all(|s| {
        !first
            .items()
            .iter()
            .any(|seen| seen.conversation.id() == s.conversation.id())
    }));
    EditMessage::new(store, &policy)
        .execute(
            peer.id(),
            message.id(),
            "edited",
            now + Duration::seconds(20),
        )
        .await
        .unwrap();
    store
        .delete_message(peer.id(), message.id(), now + Duration::seconds(30))
        .await
        .unwrap();
    store
        .mark_read_through(actor, ids[0], message.id())
        .await
        .unwrap();
    let tombstone = GetConversationSummary::new(store)
        .execute(actor, ids[0])
        .await
        .unwrap();
    assert_eq!(tombstone.activity_at, detail.activity_at);
    assert_eq!(
        tombstone.conversation.updated_at(),
        detail.conversation.updated_at()
    );
    assert_eq!(tombstone.unread_count, 0);
    assert!(tombstone.last_message.unwrap().is_deleted());
    store
        .rename_group(
            ids[0],
            actor,
            &GroupTitle::new("renamed").unwrap(),
            now + Duration::seconds(40),
        )
        .await
        .unwrap();
    assert_eq!(
        GetConversationSummary::new(store)
            .execute(actor, ids[0])
            .await
            .unwrap()
            .activity_at,
        now + Duration::seconds(40)
    );
    let members = ListConversationMembers::new(store)
        .execute(
            peer.id(),
            ids[0],
            PageRequest::new(PageLimit::new(1).unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(members.items().len(), 1);
    let rest = ListConversationMembers::new(store)
        .execute(
            peer.id(),
            ids[0],
            PageRequest::default().with_cursor(members.next_cursor().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(rest.items().len(), 1);
    assert!(members.items()[0].user.id < rest.items()[0].user.id);
    store.leave_group(ids[0], peer.id(), now).await.unwrap();
    assert!(matches!(
        ListConversationMembers::new(store)
            .execute(
                peer.id(),
                ids[0],
                PageRequest::default().with_cursor(members.next_cursor().unwrap())
            )
            .await,
        Err(MessagingQueryError::NotMember)
    ));
    let direct = Conversation::new_direct(ConversationId::new(), now);
    store
        .get_or_create_direct(&direct, actor, peer.id())
        .await
        .unwrap();
    assert_eq!(
        GetConversationSummary::new(store)
            .execute(actor, direct.id())
            .await
            .unwrap()
            .title,
        peer.username().as_str()
    );
    assert_eq!(
        GetConversationSummary::new(store)
            .execute(peer.id(), direct.id())
            .await
            .unwrap()
            .title,
        owner.username().as_str()
    );
    peer.disable(now);
    store.update_user(&peer).await.unwrap();
    let historical = GetMessage::new(store)
        .execute(actor, message.id())
        .await
        .unwrap();
    assert_eq!(historical.sender_username, *peer.username());
    assert!(
        FindUsers::new(store)
            .execute(peer.username().as_str())
            .await
            .unwrap()
            .items()
            .is_empty()
    );
    assert_eq!(
        GetConversationSummary::new(store)
            .execute(actor, direct.id())
            .await
            .unwrap()
            .title,
        peer.username().as_str()
    );
    let direct_members = ListConversationMembers::new(store)
        .execute(actor, direct.id(), PageRequest::default())
        .await
        .unwrap();
    assert_eq!(direct_members.items().len(), 2);
    let self_direct = Conversation::new_direct(ConversationId::new(), now);
    store
        .get_or_create_direct(&self_direct, actor, actor)
        .await
        .unwrap();
    assert_eq!(
        GetConversationSummary::new(store)
            .execute(actor, self_direct.id())
            .await
            .unwrap()
            .title,
        owner.username().as_str()
    );
    assert_eq!(
        ListConversationMembers::new(store)
            .execute(actor, self_direct.id(), PageRequest::default())
            .await
            .unwrap()
            .items()
            .len(),
        1
    );
    peer.enable(now);
    store.update_user(&peer).await.unwrap();
    activity_and_unread(
        store,
        actor,
        outsider.id(),
        peer.id(),
        now + Duration::seconds(100),
    )
    .await;
}

async fn activity_and_unread<S: MessagingQueryStore + ConversationStore + MessageStore>(
    store: &S,
    actor: UserId,
    other: UserId,
    joining: UserId,
    now: OffsetDateTime,
) {
    let a = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("activity a").unwrap(),
        now,
    );
    let b = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("activity b").unwrap(),
        now,
    );
    store.create_group(&a, actor, &[other]).await.unwrap();
    store.create_group(&b, actor, &[other]).await.unwrap();
    let canonical = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("canonical activity").unwrap(),
        now,
    );
    store.create_group(&canonical, actor, &[]).await.unwrap();
    let lower_id: MessageId = "018f6f34-0000-7000-8000-000000000000".parse().unwrap();
    let higher_id: MessageId = "018f6f35-0000-7000-8000-000000000001".parse().unwrap();
    let older_by_id = Message::new(
        lower_id,
        canonical.id(),
        actor,
        MessageContent::new("later timestamp").unwrap(),
        None,
        now + Duration::seconds(10),
    )
    .unwrap();
    let newer_by_id = Message::new(
        higher_id,
        canonical.id(),
        actor,
        MessageContent::new("canonical latest").unwrap(),
        None,
        now + Duration::seconds(1),
    )
    .unwrap();
    store.send_message(&older_by_id).await.unwrap();
    store.send_message(&newer_by_id).await.unwrap();
    let canonical_summary = GetConversationSummary::new(store)
        .execute(actor, canonical.id())
        .await
        .unwrap();
    assert_eq!(canonical_summary.last_message.unwrap().id(), higher_id);
    assert_eq!(canonical_summary.activity_at, newer_by_id.created_at());
    let first = ListConversations::new(store)
        .execute(actor, PageRequest::new(PageLimit::new(1).unwrap()))
        .await
        .unwrap();
    assert_eq!(first.items()[0].conversation.id(), canonical.id());
    assert_eq!(first.items()[0].activity_at, newer_by_id.created_at());
    let policy = MessagePolicy::default();
    let own = SendMessage::new(store, &policy)
        .execute(actor, a.id(), "own", None, now + Duration::seconds(1))
        .await
        .unwrap();
    assert_eq!(
        GetConversationSummary::new(store)
            .execute(actor, a.id())
            .await
            .unwrap()
            .unread_count,
        0
    );
    assert_eq!(
        GetConversationSummary::new(store)
            .execute(other, a.id())
            .await
            .unwrap()
            .unread_count,
        1
    );
    let incoming = SendMessage::new(store, &policy)
        .execute(other, a.id(), "incoming", None, now + Duration::seconds(2))
        .await
        .unwrap();
    assert_eq!(
        GetConversationSummary::new(store)
            .execute(actor, a.id())
            .await
            .unwrap()
            .unread_count,
        1
    );
    store
        .add_group_member(a.id(), actor, joining, now + Duration::seconds(3))
        .await
        .unwrap();
    let joined = GetConversationSummary::new(store)
        .execute(joining, a.id())
        .await
        .unwrap();
    assert_eq!(
        joined.membership.last_read_message_id(),
        Some(incoming.id())
    );
    assert_eq!(joined.unread_count, 0);
    let latest = SendMessage::new(store, &policy)
        .execute(actor, a.id(), "latest", None, now + Duration::seconds(4))
        .await
        .unwrap();
    assert_eq!(
        GetConversationSummary::new(store)
            .execute(joining, a.id())
            .await
            .unwrap()
            .unread_count,
        1
    );
    store
        .delete_message(actor, latest.id(), now + Duration::seconds(5))
        .await
        .unwrap();
    store
        .mark_read_through(actor, a.id(), incoming.id())
        .await
        .unwrap();
    EditMessage::new(store, &policy)
        .execute(actor, own.id(), "old edit", now + Duration::seconds(6))
        .await
        .unwrap();
    let summary = GetConversationSummary::new(store)
        .execute(actor, a.id())
        .await
        .unwrap();
    assert_eq!(summary.activity_at, latest.created_at());
    assert_eq!(
        summary.conversation.updated_at(),
        now + Duration::seconds(3)
    );
    assert!(summary.last_message.unwrap().is_deleted());
    assert_eq!(summary.unread_count, 0);
    assert_eq!(
        GetConversationSummary::new(store)
            .execute(joining, a.id())
            .await
            .unwrap()
            .unread_count,
        0
    );
    store
        .rename_group(
            b.id(),
            actor,
            &GroupTitle::new("metadata activity").unwrap(),
            now + Duration::seconds(10),
        )
        .await
        .unwrap();
    let refreshed = ListConversations::new(store)
        .execute(actor, PageRequest::new(PageLimit::new(1).unwrap()))
        .await
        .unwrap();
    assert_eq!(refreshed.items()[0].conversation.id(), b.id());
    let newly_active = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("new after first page").unwrap(),
        now + Duration::seconds(20),
    );
    store.create_group(&newly_active, actor, &[]).await.unwrap();
    let continuation = ListConversations::new(store)
        .execute(
            actor,
            PageRequest::default().with_cursor(refreshed.next_cursor().unwrap()),
        )
        .await
        .unwrap();
    assert!(
        continuation
            .items()
            .iter()
            .all(|s| s.conversation.id() != newly_active.id() && s.conversation.id() != b.id())
    );
    store.leave_group(a.id(), joining, now).await.unwrap();
    assert!(
        ListConversations::new(store)
            .execute(joining, PageRequest::default())
            .await
            .unwrap()
            .items()
            .iter()
            .all(|s| s.conversation.id() != a.id())
    );
}
