use time::{Duration, OffsetDateTime};
use windwatcher::*;

/// Portable persisted create/replay semantics, with explicit time and no sleeps.
pub async fn run<S: CreateCommandStore + ConversationStore + MessageStore>(
    store: &S,
    actor: UserId,
    peer: UserId,
) {
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let key = IdempotencyKey::new("same-client-key").unwrap();
    let group_request = || ConversationCreateRequest::Group {
        title: "Engineering".into(),
        participant_ids: vec![peer, actor, peer],
    };
    let create = CreateConversation::new(store);
    let first = create
        .execute(actor, group_request(), Some(&key), now)
        .await
        .unwrap();
    assert_eq!(first.value().disposition, CreateDisposition::Created);
    assert!(
        matches!(first.events(), [DomainEvent::ConversationCreated { conversation_id, kind: ConversationKind::Group, occurred_at }] if *conversation_id == first.value().value && *occurred_at == now)
    );
    let replay = create
        .execute(
            actor,
            ConversationCreateRequest::Group {
                title: "Engineering".into(),
                participant_ids: vec![peer],
            },
            Some(&key),
            now,
        )
        .await
        .unwrap();
    assert_eq!(first.value().value, replay.value().value);
    assert_eq!(replay.value().disposition, CreateDisposition::Replayed);
    assert!(replay.events().is_empty());
    assert!(matches!(
        create
            .execute(
                actor,
                ConversationCreateRequest::Group {
                    title: "Different".into(),
                    participant_ids: vec![peer]
                },
                Some(&key),
                now
            )
            .await,
        Err(CreateCommandError::KeyReused)
    ));
    assert!(matches!(
        create
            .execute(
                actor,
                ConversationCreateRequest::Group {
                    title: "Engineering".into(),
                    participant_ids: vec![]
                },
                Some(&key),
                now
            )
            .await,
        Err(CreateCommandError::KeyReused)
    ));
    let policy = MessagePolicy::default();
    let send = SendMessageCommand::new(store, &policy);
    let sent = send
        .execute(actor, first.value().value, "hello", None, Some(&key), now)
        .await
        .unwrap();
    assert_eq!(sent.value().disposition, CreateDisposition::Created);
    assert!(
        matches!(sent.events(), [DomainEvent::MessageSent { message }] if message == &sent.value().value)
    );
    let again = send
        .execute(
            actor,
            first.value().value,
            "hello",
            None,
            Some(&key),
            now + Duration::hours(23),
        )
        .await
        .unwrap();
    assert_eq!(sent.value().value, again.value().value);
    assert_eq!(again.value().disposition, CreateDisposition::Replayed);
    assert!(again.events().is_empty());
    assert!(matches!(
        send.execute(
            actor,
            first.value().value,
            "different",
            None,
            Some(&key),
            now
        )
        .await,
        Err(CreateCommandError::KeyReused)
    ));
    assert!(matches!(
        send.execute(
            actor,
            first.value().value,
            "hello",
            Some(sent.value().value.id()),
            Some(&key),
            now
        )
        .await,
        Err(CreateCommandError::KeyReused)
    ));
    // Replays return current state and do not recheck a now-deleted reply target.
    let reply_key = IdempotencyKey::new("reply-key").unwrap();
    let reply = send
        .execute(
            actor,
            first.value().value,
            "reply",
            Some(sent.value().value.id()),
            Some(&reply_key),
            now,
        )
        .await
        .unwrap();
    store
        .delete_message(actor, sent.value().value.id(), now + Duration::seconds(1))
        .await
        .unwrap();
    let tombstone_replay = send
        .execute(
            actor,
            first.value().value,
            "hello",
            None,
            Some(&key),
            now + Duration::seconds(2),
        )
        .await
        .unwrap();
    assert!(tombstone_replay.value().value.is_deleted());
    let reply_replay = send
        .execute(
            actor,
            first.value().value,
            "reply",
            Some(sent.value().value.id()),
            Some(&reply_key),
            now + Duration::seconds(2),
        )
        .await
        .unwrap();
    assert_eq!(reply_replay.value().value.id(), reply.value().value.id());
    assert!(
        reply_replay
            .value()
            .value
            .reply_to
            .as_ref()
            .unwrap()
            .content
            .is_none()
    );
    let independent = send
        .execute(peer, first.value().value, "hello", None, Some(&key), now)
        .await
        .unwrap();
    assert_ne!(independent.value().value.id(), sent.value().value.id());
    let second_group = create
        .execute(
            actor,
            ConversationCreateRequest::Group {
                title: "another scope".into(),
                participant_ids: vec![],
            },
            None,
            now,
        )
        .await
        .unwrap();
    let scoped = send
        .execute(
            actor,
            second_group.value().value,
            "different payload in another scope",
            None,
            Some(&key),
            now,
        )
        .await
        .unwrap();
    assert_eq!(scoped.value().disposition, CreateDisposition::Created);
    let failed_key = IdempotencyKey::new("failed-key").unwrap();
    assert!(matches!(
        send.execute(
            actor,
            first.value().value,
            "reply",
            Some(MessageId::new()),
            Some(&failed_key),
            now
        )
        .await,
        Err(CreateCommandError::ReplyTargetUnavailable)
    ));
    assert!(
        send.execute(
            actor,
            first.value().value,
            "fixed",
            None,
            Some(&failed_key),
            now
        )
        .await
        .is_ok()
    );
    // Active strictly before expiry; reusable at expiry and after it.
    let expiry = now + Duration::days(1);
    let expired = send
        .execute(
            actor,
            first.value().value,
            "different",
            None,
            Some(&key),
            expiry,
        )
        .await
        .unwrap();
    assert_eq!(expired.value().disposition, CreateDisposition::Created);
    assert_ne!(expired.value().value.id(), sent.value().value.id());
    let after = send
        .execute(
            actor,
            first.value().value,
            "third",
            None,
            Some(&key),
            expiry + Duration::days(1) + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(after.value().disposition, CreateDisposition::Created);
    let direct_key = IdempotencyKey::new("direct-key").unwrap();
    let request = || ConversationCreateRequest::Direct {
        participant_id: peer,
    };
    let direct = create.execute(actor, request(), None, now).await.unwrap();
    let existing = create
        .execute(actor, request(), Some(&direct_key), now)
        .await
        .unwrap();
    assert_eq!(direct.value().value, existing.value().value);
    assert_eq!(existing.value().disposition, CreateDisposition::Existing);
    assert!(existing.events().is_empty());
    assert_eq!(
        create
            .execute(actor, request(), Some(&direct_key), now)
            .await
            .unwrap()
            .disposition,
        CreateDisposition::Replayed
    );
    let new_direct = create
        .execute(
            peer,
            ConversationCreateRequest::Direct {
                participant_id: peer,
            },
            Some(&direct_key),
            now,
        )
        .await
        .unwrap();
    assert_eq!(new_direct.value().disposition, CreateDisposition::Created);
    let group_key = IdempotencyKey::new("deleted-group").unwrap();
    let request = || ConversationCreateRequest::Group {
        title: "temporary".into(),
        participant_ids: vec![],
    };
    let group = create
        .execute(actor, request(), Some(&group_key), now)
        .await
        .unwrap();
    store
        .leave_group(group.value().value, actor, now)
        .await
        .unwrap();
    assert!(matches!(
        create
            .execute(actor, request(), Some(&group_key), now)
            .await,
        Err(CreateCommandError::ResultUnavailable)
    ));
    let replacement = create
        .execute(actor, request(), Some(&group_key), now + Duration::days(1))
        .await
        .unwrap();
    assert_eq!(replacement.value().disposition, CreateDisposition::Created);
    assert_ne!(replacement.value(), group.value());
    // A failed create does not establish the key mapping.
    let unused = IdempotencyKey::new("failed-group").unwrap();
    assert!(matches!(
        create
            .execute(
                actor,
                ConversationCreateRequest::Group {
                    title: "unavailable".into(),
                    participant_ids: vec![UserId::new()]
                },
                Some(&unused),
                now
            )
            .await,
        Err(CreateCommandError::ParticipantUnavailable)
    ));
    assert!(
        create
            .execute(actor, request(), Some(&unused), now)
            .await
            .is_ok()
    );
}
