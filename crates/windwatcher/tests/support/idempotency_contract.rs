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
    assert_eq!(first.disposition, CreateDisposition::Created);
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
    assert_eq!(first.value, replay.value);
    assert_eq!(replay.disposition, CreateDisposition::Replayed);
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
        .execute(actor, first.value, "hello", None, Some(&key), now)
        .await
        .unwrap();
    assert_eq!(sent.disposition, CreateDisposition::Created);
    let again = send
        .execute(
            actor,
            first.value,
            "hello",
            None,
            Some(&key),
            now + Duration::hours(23),
        )
        .await
        .unwrap();
    assert_eq!(sent.value, again.value);
    assert_eq!(again.disposition, CreateDisposition::Replayed);
    assert!(matches!(
        send.execute(actor, first.value, "different", None, Some(&key), now)
            .await,
        Err(CreateCommandError::KeyReused)
    ));
    assert!(matches!(
        send.execute(
            actor,
            first.value,
            "hello",
            Some(sent.value.id()),
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
            first.value,
            "reply",
            Some(sent.value.id()),
            Some(&reply_key),
            now,
        )
        .await
        .unwrap();
    store
        .delete_message(actor, sent.value.id(), now + Duration::seconds(1))
        .await
        .unwrap();
    let tombstone_replay = send
        .execute(
            actor,
            first.value,
            "hello",
            None,
            Some(&key),
            now + Duration::seconds(2),
        )
        .await
        .unwrap();
    assert!(tombstone_replay.value.is_deleted());
    let reply_replay = send
        .execute(
            actor,
            first.value,
            "reply",
            Some(sent.value.id()),
            Some(&reply_key),
            now + Duration::seconds(2),
        )
        .await
        .unwrap();
    assert_eq!(reply_replay.value.id(), reply.value.id());
    assert!(reply_replay.value.reply_to.unwrap().content.is_none());
    let independent = send
        .execute(peer, first.value, "hello", None, Some(&key), now)
        .await
        .unwrap();
    assert_ne!(independent.value.id(), sent.value.id());
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
            second_group.value,
            "different payload in another scope",
            None,
            Some(&key),
            now,
        )
        .await
        .unwrap();
    assert_eq!(scoped.disposition, CreateDisposition::Created);
    let failed_key = IdempotencyKey::new("failed-key").unwrap();
    assert!(matches!(
        send.execute(
            actor,
            first.value,
            "reply",
            Some(MessageId::new()),
            Some(&failed_key),
            now
        )
        .await,
        Err(CreateCommandError::ReplyTargetUnavailable)
    ));
    assert!(
        send.execute(actor, first.value, "fixed", None, Some(&failed_key), now)
            .await
            .is_ok()
    );
    // Active strictly before expiry; reusable at expiry and after it.
    let expiry = now + Duration::days(1);
    let expired = send
        .execute(actor, first.value, "different", None, Some(&key), expiry)
        .await
        .unwrap();
    assert_eq!(expired.disposition, CreateDisposition::Created);
    assert_ne!(expired.value.id(), sent.value.id());
    let after = send
        .execute(
            actor,
            first.value,
            "third",
            None,
            Some(&key),
            expiry + Duration::days(1) + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(after.disposition, CreateDisposition::Created);
    let direct_key = IdempotencyKey::new("direct-key").unwrap();
    let request = || ConversationCreateRequest::Direct {
        participant_id: peer,
    };
    let direct = create.execute(actor, request(), None, now).await.unwrap();
    let existing = create
        .execute(actor, request(), Some(&direct_key), now)
        .await
        .unwrap();
    assert_eq!(direct.value, existing.value);
    assert_eq!(existing.disposition, CreateDisposition::Existing);
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
    assert_eq!(new_direct.disposition, CreateDisposition::Created);
    let group_key = IdempotencyKey::new("deleted-group").unwrap();
    let request = || ConversationCreateRequest::Group {
        title: "temporary".into(),
        participant_ids: vec![],
    };
    let group = create
        .execute(actor, request(), Some(&group_key), now)
        .await
        .unwrap();
    store.leave_group(group.value, actor, now).await.unwrap();
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
    assert_eq!(replacement.disposition, CreateDisposition::Created);
    assert_ne!(replacement.value, group.value);
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
