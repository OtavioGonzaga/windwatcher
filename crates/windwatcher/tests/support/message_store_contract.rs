use time::{Duration, OffsetDateTime};
use windwatcher::*;

async fn send<S: MessageStore>(
    store: &S,
    actor: UserId,
    conversation: ConversationId,
    text: &str,
    reply: Option<MessageId>,
    now: OffsetDateTime,
) -> Message {
    let message = Message::new(
        MessageId::new(),
        conversation,
        actor,
        MessageContent::new(text).unwrap(),
        reply,
        now,
    )
    .unwrap();
    match store.send_message(&message).await.unwrap() {
        SendMessageStoreOutcome::Sent(persisted) => {
            assert_eq!(persisted, message);
            persisted
        }
        outcome => panic!("send failed: {outcome:?}"),
    }
}

async fn get<S: MessageStore>(
    store: &S,
    actor: UserId,
    conversation: ConversationId,
    id: MessageId,
) -> Message {
    match store.get_message(actor, conversation, id).await.unwrap() {
        GetMessageStoreOutcome::Found(message) => message,
        outcome => panic!("get failed: {outcome:?}"),
    }
}

/// Adapter-independent message, authorization, reply and read-state semantics.
/// Users must already exist; this suite contains no physical database assertions.
pub async fn run<S: MessageStore + ConversationStore>(
    store: &S,
    owner: UserId,
    admin: UserId,
    member: UserId,
    outsider: UserId,
) {
    pagination_contract(store, owner, member, outsider).await;
    let now =
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap() + Duration::microseconds(123);
    let later = now + Duration::seconds(1);
    let last = later + Duration::seconds(1);
    let content = MessageContent::new("updated 😀").unwrap();
    let direct = Conversation::new_direct(ConversationId::new(), now);
    let direct = match store
        .get_or_create_direct(&direct, owner, member)
        .await
        .unwrap()
    {
        StartDirectStoreOutcome::Created(c) | StartDirectStoreOutcome::Existing(c) => c,
    };
    let c = direct.id();
    assert_eq!(
        store
            .find_member(c, owner)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        None
    );
    assert_eq!(
        store.unread_count(owner, c).await.unwrap(),
        UnreadCountStoreOutcome::Count(0)
    );
    let first = send(store, owner, c, "  😀\n  ", None, now).await;
    assert_eq!(get(store, member, c, first.id()).await, first);
    assert_eq!(first.sender_id(), owner);
    assert_eq!(first.created_at(), now);
    assert_eq!(first.edited_at(), None);
    assert_eq!(first.deleted_at(), None);
    assert_eq!(
        store
            .edit_message(owner, c, first.id(), first.content().unwrap(), later)
            .await
            .unwrap(),
        EditMessageStoreOutcome::Unchanged(first.clone())
    );
    let second = send(store, owner, c, "", Some(first.id()), later).await;
    let own = send(store, member, c, " ", None, later).await;
    assert!(first.id() < second.id() && second.id() < own.id());
    assert_eq!(
        store
            .find_member(c, member)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        None
    );
    assert_eq!(
        store.unread_count(member, c).await.unwrap(),
        UnreadCountStoreOutcome::Count(2)
    );
    assert_eq!(
        store.unread_count(owner, c).await.unwrap(),
        UnreadCountStoreOutcome::Count(1)
    );

    assert_eq!(
        store
            .edit_message(member, c, first.id(), &content, later)
            .await
            .unwrap(),
        EditMessageStoreOutcome::NotAuthor
    );
    assert_eq!(
        store
            .delete_message(member, c, first.id(), later)
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::NotAuthorized
    );
    let changed = match store
        .edit_message(owner, c, second.id(), &content, later)
        .await
        .unwrap()
    {
        EditMessageStoreOutcome::Changed(m) => m,
        other => panic!("{other:?}"),
    };
    assert_eq!(changed.content(), Some(&content));
    assert_eq!(changed.edited_at(), Some(later));
    assert_eq!(changed.created_at(), second.created_at());
    assert_eq!(changed.reply_to_message_id(), Some(first.id()));
    assert_eq!(
        store
            .edit_message(owner, c, second.id(), &content, last)
            .await
            .unwrap(),
        EditMessageStoreOutcome::Unchanged(changed.clone())
    );
    let changed_again = match store
        .edit_message(
            owner,
            c,
            second.id(),
            &MessageContent::new("again").unwrap(),
            last,
        )
        .await
        .unwrap()
    {
        EditMessageStoreOutcome::Changed(m) => m,
        other => panic!("{other:?}"),
    };
    assert_eq!(changed_again.edited_at(), Some(last));

    assert_eq!(
        store
            .mark_read_through(member, c, first.id())
            .await
            .unwrap(),
        MarkReadStoreOutcome::Changed
    );
    assert_eq!(
        store
            .mark_read_through(member, c, first.id())
            .await
            .unwrap(),
        MarkReadStoreOutcome::Unchanged
    );
    assert_eq!(
        store.unread_count(member, c).await.unwrap(),
        UnreadCountStoreOutcome::Count(1)
    );
    let tombstone = match store
        .delete_message(owner, c, second.id(), last)
        .await
        .unwrap()
    {
        DeleteMessageStoreOutcome::Deleted(m) => m,
        other => panic!("{other:?}"),
    };
    assert_eq!(tombstone.id(), second.id());
    assert_eq!(tombstone.conversation_id(), c);
    assert_eq!(tombstone.sender_id(), owner);
    assert_eq!(tombstone.reply_to_message_id(), Some(first.id()));
    assert_eq!(tombstone.created_at(), second.created_at());
    assert_eq!(tombstone.edited_at(), Some(last));
    assert_eq!(tombstone.deleted_at(), Some(last));
    assert_eq!(tombstone.content(), None);
    assert_eq!(get(store, member, c, second.id()).await, tombstone);
    assert_eq!(
        store
            .delete_message(owner, c, second.id(), last + Duration::seconds(10))
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::AlreadyDeleted(tombstone)
    );
    assert_eq!(
        store
            .delete_message(member, c, second.id(), last)
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::NotAuthorized
    );
    assert_eq!(
        store
            .edit_message(owner, c, second.id(), &content, last)
            .await
            .unwrap(),
        EditMessageStoreOutcome::Deleted
    );
    assert_eq!(
        store.unread_count(member, c).await.unwrap(),
        UnreadCountStoreOutcome::Count(0)
    );
    assert_eq!(
        store
            .find_member(c, member)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        Some(first.id())
    );
    assert_eq!(
        store
            .mark_read_through(member, c, second.id())
            .await
            .unwrap(),
        MarkReadStoreOutcome::Changed
    );
    assert_eq!(
        store
            .mark_read_through(member, c, first.id())
            .await
            .unwrap(),
        MarkReadStoreOutcome::Unchanged
    );
    assert_eq!(
        store
            .find_member(c, member)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        Some(second.id())
    );
    let newer = send(store, owner, c, "newer", None, last).await;
    assert_eq!(
        store.unread_count(member, c).await.unwrap(),
        UnreadCountStoreOutcome::Count(1)
    );
    assert_eq!(
        store
            .mark_read_through(member, c, newer.id())
            .await
            .unwrap(),
        MarkReadStoreOutcome::Changed
    );
    assert_eq!(
        store.unread_count(member, c).await.unwrap(),
        UnreadCountStoreOutcome::Count(0)
    );
    assert_eq!(
        store
            .find_conversation(c)
            .await
            .unwrap()
            .unwrap()
            .updated_at(),
        direct.updated_at()
    );

    let denied = Message::new(MessageId::new(), c, outsider, content.clone(), None, now).unwrap();
    assert_eq!(
        store.send_message(&denied).await.unwrap(),
        SendMessageStoreOutcome::NotMember
    );
    assert_eq!(
        store.get_message(outsider, c, first.id()).await.unwrap(),
        GetMessageStoreOutcome::NotMember
    );
    assert_eq!(
        store
            .edit_message(outsider, c, first.id(), &content, now)
            .await
            .unwrap(),
        EditMessageStoreOutcome::NotMember
    );
    assert_eq!(
        store
            .delete_message(outsider, c, first.id(), now)
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::NotMember
    );
    assert_eq!(
        store
            .mark_read_through(outsider, c, first.id())
            .await
            .unwrap(),
        MarkReadStoreOutcome::NotMember
    );
    assert_eq!(
        store.unread_count(outsider, c).await.unwrap(),
        UnreadCountStoreOutcome::NotMember
    );

    let missing = MessageId::new();
    assert_eq!(
        store.get_message(owner, c, missing).await.unwrap(),
        GetMessageStoreOutcome::MessageNotFound
    );
    assert_eq!(
        store
            .edit_message(owner, c, missing, &content, now)
            .await
            .unwrap(),
        EditMessageStoreOutcome::MessageNotFound
    );
    assert_eq!(
        store.delete_message(owner, c, missing, now).await.unwrap(),
        DeleteMessageStoreOutcome::MessageNotFound
    );
    assert_eq!(
        store.mark_read_through(owner, c, missing).await.unwrap(),
        MarkReadStoreOutcome::MessageNotFound
    );
    let absent = ConversationId::new();
    let absent_message =
        Message::new(MessageId::new(), absent, owner, content.clone(), None, now).unwrap();
    assert_eq!(
        store.send_message(&absent_message).await.unwrap(),
        SendMessageStoreOutcome::ConversationNotFound
    );
    assert_eq!(
        store.get_message(owner, absent, missing).await.unwrap(),
        GetMessageStoreOutcome::ConversationNotFound
    );
    assert_eq!(
        store
            .edit_message(owner, absent, missing, &content, now)
            .await
            .unwrap(),
        EditMessageStoreOutcome::ConversationNotFound
    );
    assert_eq!(
        store
            .delete_message(owner, absent, missing, now)
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::ConversationNotFound
    );
    assert_eq!(
        store
            .mark_read_through(owner, absent, missing)
            .await
            .unwrap(),
        MarkReadStoreOutcome::ConversationNotFound
    );
    assert_eq!(
        store.unread_count(owner, absent).await.unwrap(),
        UnreadCountStoreOutcome::ConversationNotFound
    );

    let self_direct = Conversation::new_direct(ConversationId::new(), now);
    let self_id = match store
        .get_or_create_direct(&self_direct, owner, owner)
        .await
        .unwrap()
    {
        StartDirectStoreOutcome::Created(c) | StartDirectStoreOutcome::Existing(c) => c.id(),
    };
    let self_message = send(store, owner, self_id, "self", None, now).await;
    assert_eq!(
        store.unread_count(owner, self_id).await.unwrap(),
        UnreadCountStoreOutcome::Count(0)
    );
    assert_eq!(
        store
            .find_member(self_id, owner)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        None
    );
    assert_eq!(
        store
            .get_message(owner, c, self_message.id())
            .await
            .unwrap(),
        GetMessageStoreOutcome::MessageNotFound
    );
    assert_eq!(
        store
            .edit_message(owner, c, self_message.id(), &content, now)
            .await
            .unwrap(),
        EditMessageStoreOutcome::MessageNotFound
    );
    assert_eq!(
        store
            .delete_message(owner, c, self_message.id(), now)
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::MessageNotFound
    );
    assert_eq!(
        store
            .mark_read_through(owner, c, self_message.id())
            .await
            .unwrap(),
        MarkReadStoreOutcome::MessageNotFound
    );

    for (reply, expected) in [
        (missing, SendMessageStoreOutcome::ReplyTargetNotFound),
        (
            self_message.id(),
            SendMessageStoreOutcome::ReplyTargetNotFound,
        ),
        (second.id(), SendMessageStoreOutcome::ReplyTargetDeleted),
    ] {
        let message = Message::new(
            MessageId::new(),
            c,
            owner,
            content.clone(),
            Some(reply),
            now,
        )
        .unwrap();
        assert_eq!(store.send_message(&message).await.unwrap(), expected);
    }
    let surviving_reply = send(store, owner, c, "active reply", Some(first.id()), last).await;
    store
        .delete_message(owner, c, first.id(), last)
        .await
        .unwrap();
    assert!(get(store, member, c, first.id()).await.is_deleted());
    assert_eq!(
        get(store, member, c, second.id())
            .await
            .reply_to_message_id(),
        Some(first.id())
    );

    assert_eq!(
        get(store, member, c, surviving_reply.id()).await,
        surviving_reply
    );

    let group = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("messages").unwrap(),
        now,
    );
    let g = group.id();
    store
        .create_group(&group, owner, &[admin, member])
        .await
        .unwrap();
    store
        .set_group_member_role(g, owner, admin, GroupRole::Admin, now)
        .await
        .unwrap();
    let original = send(store, member, g, "member message", None, now).await;
    for moderator in [owner, admin] {
        assert_eq!(
            store
                .edit_message(moderator, g, original.id(), &content, now)
                .await
                .unwrap(),
            EditMessageStoreOutcome::NotAuthor
        );
    }
    let owner_message = send(store, owner, g, "owner message", None, now).await;
    assert_eq!(
        store
            .delete_message(member, g, owner_message.id(), now)
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::NotAuthorized
    );
    // A new member joins read through the maximum, including a tombstone.
    store
        .delete_message(owner, g, owner_message.id(), later)
        .await
        .unwrap();
    assert_eq!(
        store
            .add_group_member(g, owner, outsider, later)
            .await
            .unwrap(),
        AddMemberStoreOutcome::Added
    );
    assert_eq!(
        store
            .find_member(g, outsider)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        Some(owner_message.id())
    );
    assert_eq!(
        store.unread_count(outsider, g).await.unwrap(),
        UnreadCountStoreOutcome::Count(0)
    );
    let after_join = send(store, member, g, "after join", Some(original.id()), last).await;
    assert_eq!(
        store.unread_count(outsider, g).await.unwrap(),
        UnreadCountStoreOutcome::Count(1)
    );
    assert_eq!(
        store
            .add_group_member(g, owner, outsider, last)
            .await
            .unwrap(),
        AddMemberStoreOutcome::AlreadyMember
    );
    assert_eq!(
        store
            .find_member(g, outsider)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        Some(owner_message.id())
    );
    store.leave_group(g, outsider, last).await.unwrap();
    assert_eq!(
        store.get_message(outsider, g, original.id()).await.unwrap(),
        GetMessageStoreOutcome::NotMember
    );
    assert_eq!(
        store
            .send_message(
                &Message::new(MessageId::new(), g, outsider, content.clone(), None, last).unwrap()
            )
            .await
            .unwrap(),
        SendMessageStoreOutcome::NotMember
    );
    store
        .add_group_member(g, owner, outsider, last)
        .await
        .unwrap();
    assert_eq!(
        store
            .find_member(g, outsider)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        Some(after_join.id())
    );
    assert_eq!(
        store.unread_count(outsider, g).await.unwrap(),
        UnreadCountStoreOutcome::Count(0)
    );
    // A regular author can delete their own content.
    assert!(matches!(
        store
            .delete_message(member, g, after_join.id(), last)
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::Deleted(_)
    ));
    store.leave_group(g, member, last).await.unwrap();
    // Historical sender need not remain a member for Admin moderation.
    assert!(matches!(
        store
            .delete_message(admin, g, original.id(), last)
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::Deleted(_)
    ));
    assert_eq!(
        store
            .delete_message(outsider, g, original.id(), last)
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::NotAuthorized
    );
    let admin_message = send(store, admin, g, "admin", None, now).await;
    assert!(matches!(
        store
            .delete_message(owner, g, admin_message.id(), last)
            .await
            .unwrap(),
        DeleteMessageStoreOutcome::Deleted(_)
    ));
    // Explicitly exercise Group cascade with an active reply relationship.
    let a = send(store, owner, g, "a", None, now).await;
    let b = send(store, owner, g, "b", Some(a.id()), now).await;
    store.leave_group(g, admin, last).await.unwrap();
    store.leave_group(g, outsider, last).await.unwrap();
    assert_eq!(
        store.leave_group(g, owner, last).await.unwrap(),
        LeaveStoreOutcome::ConversationDeleted
    );
    assert_eq!(store.find_conversation(g).await.unwrap(), None);
    for actor in [owner, admin, member, outsider] {
        assert_eq!(store.find_member(g, actor).await.unwrap(), None);
    }
    for id in [a.id(), b.id(), original.id()] {
        assert_eq!(
            store.get_message(owner, g, id).await.unwrap(),
            GetMessageStoreOutcome::ConversationNotFound
        );
    }

    let empty = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("empty").unwrap(),
        now,
    );
    store.create_group(&empty, owner, &[]).await.unwrap();
    store
        .add_group_member(empty.id(), owner, member, now)
        .await
        .unwrap();
    assert_eq!(
        store
            .find_member(empty.id(), member)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        None
    );
}

async fn slice<S: MessageStore>(
    store: &S,
    actor: UserId,
    c: ConversationId,
    before: Option<MessageId>,
    limit: usize,
) -> MessagePageSlice {
    let ListMessagesStoreOutcome::Page(page) = store
        .list_messages(actor, c, before, PageLimit::new(limit).unwrap())
        .await
        .unwrap()
    else {
        panic!("expected authorized page")
    };
    assert!(page.items.len() <= limit);
    assert!(
        page.items
            .windows(2)
            .all(|pair| pair[0].id() > pair[1].id())
    );
    assert!(
        page.items
            .iter()
            .all(|m| m.conversation_id() == c && before.is_none_or(|b| m.id() < b))
    );
    assert!(!page.has_more || !page.items.is_empty());
    page
}

async fn pagination_contract<S: MessageStore + ConversationStore>(
    store: &S,
    owner: UserId,
    member: UserId,
    outsider: UserId,
) {
    let now = OffsetDateTime::UNIX_EPOCH;
    let group = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("pagination").unwrap(),
        now,
    );
    let c = group.id();
    store.create_group(&group, owner, &[member]).await.unwrap();
    let empty = slice(store, member, c, None, 50).await;
    assert!(empty.items.is_empty() && !empty.has_more);
    assert_eq!(
        store
            .list_messages(outsider, c, None, PageLimit::DEFAULT)
            .await
            .unwrap(),
        ListMessagesStoreOutcome::NotMember
    );
    assert_eq!(
        store
            .list_messages(owner, ConversationId::new(), None, PageLimit::DEFAULT)
            .await
            .unwrap(),
        ListMessagesStoreOutcome::ConversationNotFound
    );
    // Deliberately deterministic keys, inserted out of order, not generated-clock timing.
    let mut ids = Vec::new();
    for number in [3, 1, 5, 2, 4] {
        let id: MessageId = format!("018f6f34-0000-7000-8000-{number:012x}")
            .parse()
            .unwrap();
        let message = Message::new(
            id,
            c,
            owner,
            MessageContent::new("history").unwrap(),
            None,
            now,
        )
        .unwrap();
        assert!(matches!(
            store.send_message(&message).await.unwrap(),
            SendMessageStoreOutcome::Sent(_)
        ));
        ids.push(id);
        if ids.len() == 1 {
            let page = slice(store, member, c, None, 50).await;
            assert_eq!(page.items.len(), 1);
            assert!(!page.has_more);
        }
    }
    ids.sort_by(|a, b| b.cmp(a));
    let marker = store
        .find_member(c, member)
        .await
        .unwrap()
        .unwrap()
        .last_read_message_id();
    let full = slice(store, member, c, None, 50).await;
    assert_eq!(full.items.len(), 5);
    assert!(!full.has_more);
    let first = ListMessages::new(store)
        .execute(member, c, PageRequest::new(PageLimit::new(2).unwrap()))
        .await
        .unwrap();
    assert_eq!(
        first.items().iter().map(Message::id).collect::<Vec<_>>(),
        ids[..2]
    );
    // A newer insertion stays above the boundary and does not enter continuation.
    let newer = send(store, owner, c, "new", None, now).await;
    assert!(newer.id() > ids[0]);
    store.delete_message(owner, c, ids[2], now).await.unwrap();
    let second = ListMessages::new(store)
        .execute(
            member,
            c,
            PageRequest::new(PageLimit::new(2).unwrap()).with_cursor(first.next_cursor().unwrap()),
        )
        .await
        .unwrap();
    assert!(second.has_more());
    assert!(second.items()[0].is_deleted());
    let third = ListMessages::new(store)
        .execute(
            member,
            c,
            PageRequest::new(PageLimit::new(2).unwrap()).with_cursor(second.next_cursor().unwrap()),
        )
        .await
        .unwrap();
    assert!(!third.has_more());
    assert_eq!(third.items().len(), 1);
    let traversed: Vec<_> = first
        .items()
        .iter()
        .chain(second.items())
        .chain(third.items())
        .map(Message::id)
        .collect();
    assert_eq!(traversed, ids);
    assert_eq!(
        store
            .find_member(c, member)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        marker
    );
    // A typed boundary need not identify a stored row.
    let fabricated = "018f6f34-0000-7000-8000-000000000006".parse().unwrap();
    assert_eq!(
        slice(store, member, c, Some(fabricated), 50)
            .await
            .items
            .len(),
        5
    );
    store.leave_group(c, member, now).await.unwrap();
    assert!(matches!(
        ListMessages::new(store)
            .execute(
                member,
                c,
                PageRequest::default().with_cursor(first.next_cursor().unwrap())
            )
            .await,
        Err(ListMessagesError::NotMember)
    ));
    store.add_group_member(c, owner, member, now).await.unwrap();
    assert_eq!(slice(store, member, c, None, 50).await.items.len(), 6);
    store.leave_group(c, member, now).await.unwrap();
    store.leave_group(c, owner, now).await.unwrap();
    assert!(matches!(
        ListMessages::new(store)
            .execute(
                owner,
                c,
                PageRequest::default().with_cursor(first.next_cursor().unwrap())
            )
            .await,
        Err(ListMessagesError::ConversationNotFound)
    ));

    // Exact-limit and lookahead edge, using a self Direct.
    let direct = Conversation::new_direct(ConversationId::new(), now);
    let c = match store
        .get_or_create_direct(&direct, outsider, outsider)
        .await
        .unwrap()
    {
        StartDirectStoreOutcome::Created(c) | StartDirectStoreOutcome::Existing(c) => c.id(),
    };
    for _ in 0..50 {
        send(store, outsider, c, "self", None, now).await;
    }
    let exact = ListMessages::new(store)
        .execute(outsider, c, PageRequest::default())
        .await
        .unwrap();
    assert_eq!(exact.items().len(), 50);
    assert_eq!(exact.next_cursor(), None);
    send(store, outsider, c, "51", None, now).await;
    let page = ListMessages::new(store)
        .execute(outsider, c, PageRequest::default())
        .await
        .unwrap();
    assert_eq!(page.items().len(), 50);
    assert!(page.has_more());
    let final_page = ListMessages::new(store)
        .execute(
            outsider,
            c,
            PageRequest::default().with_cursor(page.next_cursor().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(final_page.items().len(), 1);
    assert!(!final_page.has_more());
}
