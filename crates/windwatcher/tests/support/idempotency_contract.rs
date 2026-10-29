use time::{Duration, OffsetDateTime};
use windwatcher::*;

/// Independent instances must converge before exposing successful keyed effects.
pub async fn concurrent<S: CreateCommandStore + ConversationStore + MessageStore>(
    first: &S,
    second: &S,
    actor: UserId,
    peer: UserId,
) {
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let key = IdempotencyKey::new("concurrent-group").unwrap();
    let command = |title: &str| ConversationCreateCommand {
        conversation: Conversation::new_group(
            ConversationId::new(),
            GroupTitle::new(title).unwrap(),
            now,
        ),
        actor,
        participants: vec![peer],
    };
    let a = command("concurrent");
    let b = command("concurrent");
    let c = command("concurrent");
    let (a, b, c) = tokio::join!(
        first.create_conversation_command(&a, Some(&key), now),
        second.create_conversation_command(&b, Some(&key), now),
        second.create_conversation_command(&c, Some(&key), now)
    );
    let (a, b, c) = (a.unwrap(), b.unwrap(), c.unwrap());
    assert_eq!(a.value, b.value);
    assert_eq!(a.value, c.value);
    let dispositions = [a.disposition, b.disposition, c.disposition];
    assert_eq!(
        dispositions
            .iter()
            .filter(|d| **d == CreateDisposition::Created)
            .count(),
        1
    );
    assert_eq!(
        dispositions
            .iter()
            .filter(|d| **d == CreateDisposition::Replayed)
            .count(),
        2
    );
    let conversation = a.value;
    let key = IdempotencyKey::new("concurrent-message").unwrap();
    let message = || {
        Message::new(
            MessageId::new(),
            conversation,
            actor,
            MessageContent::new("concurrent").unwrap(),
            None,
            now,
        )
        .unwrap()
    };
    let a = message();
    let b = message();
    let (a, b) = tokio::join!(
        first.send_message_command(&a, Some(&key), now),
        second.send_message_command(&b, Some(&key), now)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.value.id(), b.value.id());
    assert!(matches!(
        (a.disposition, b.disposition),
        (CreateDisposition::Created, CreateDisposition::Replayed)
            | (CreateDisposition::Replayed, CreateDisposition::Created)
    ));
    // Different fingerprints racing for the same identity must yield KeyReused.
    let key = IdempotencyKey::new("concurrent-mismatch").unwrap();
    let a = command("first");
    let b = command("second");
    let (a, b) = tokio::join!(
        first.create_conversation_command(&a, Some(&key), now),
        second.create_conversation_command(&b, Some(&key), now)
    );
    assert!(matches!(
        (a, b),
        (Ok(_), Err(CreateCommandError::KeyReused)) | (Err(CreateCommandError::KeyReused), Ok(_))
    ));
}

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
    // Rejected input must not reserve a key, independently of the adapter.
    for request in [
        ConversationCreateRequest::Group {
            title: "valid".into(),
            participant_ids: vec![actor; 257],
        },
        ConversationCreateRequest::Group {
            title: "é".repeat(129),
            participant_ids: vec![],
        },
    ] {
        assert!(matches!(
            create.execute(actor, request, Some(&key), now).await,
            Err(CreateCommandError::TooManyParticipants | CreateCommandError::InvalidTitle(_))
        ));
    }
    // Application limits must not prevent hydration of pre-existing long titles.
    let historical = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("é".repeat(129)).unwrap(),
        now,
    );
    store.create_group(&historical, actor, &[]).await.unwrap();
    assert_eq!(
        store
            .find_conversation(historical.id())
            .await
            .unwrap()
            .unwrap()
            .title()
            .unwrap()
            .as_str()
            .len(),
        258
    );
    let renamed = RenameGroupConversation::new(store)
        .execute(actor, historical.id(), "short title", now)
        .await
        .unwrap();
    assert_eq!(renamed.events().len(), 1);
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
            .value()
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
    expiry_bounds(store, actor, first.value().value, now).await;
}

// Expiration belongs to the portable datetime domain, not the (usually much
// wider) physical integer representation. Overflow must not reserve a key or
// leave a resource behind, and only keyed commands require an expiry.
async fn expiry_bounds<S: CreateCommandStore + ConversationStore + MessageStore>(
    store: &S,
    actor: UserId,
    conversation: ConversationId,
    now: OffsetDateTime,
) {
    let max = time::Date::MAX.with_hms(23, 59, 59).unwrap().assume_utc();
    let last_valid = max - Duration::days(1);
    let overflow = last_valid + Duration::seconds(1);
    for keyed_now in [overflow, max] {
        let key =
            IdempotencyKey::new(format!("expiry-bounds-{}", keyed_now.unix_timestamp())).unwrap();
        let command = ConversationCreateCommand {
            conversation: Conversation::new_group(
                ConversationId::new(),
                GroupTitle::new("expiry bounds").unwrap(),
                now,
            ),
            actor,
            participants: vec![],
        };
        assert!(matches!(
            store
                .create_conversation_command(&command, Some(&key), keyed_now)
                .await,
            Err(CreateCommandError::Storage(QueryStoreError::Unexpected(_)))
        ));
        assert!(
            store
                .find_conversation(command.conversation.id())
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .create_conversation_command(&command, Some(&key), now)
                .await
                .unwrap()
                .disposition,
            CreateDisposition::Created
        );

        let message = Message::new(
            MessageId::new(),
            conversation,
            actor,
            MessageContent::new("expiry bounds").unwrap(),
            None,
            now,
        )
        .unwrap();
        assert!(matches!(
            store
                .send_message_command(&message, Some(&key), keyed_now)
                .await,
            Err(CreateCommandError::Storage(QueryStoreError::Unexpected(_)))
        ));
        assert!(matches!(
            store.get_message(actor, message.id()).await.unwrap(),
            GetMessageStoreOutcome::MessageNotFound
        ));
        assert_eq!(
            store
                .send_message_command(&message, Some(&key), now)
                .await
                .unwrap()
                .disposition,
            CreateDisposition::Created
        );
    }
    let message = || {
        Message::new(
            MessageId::new(),
            conversation,
            actor,
            MessageContent::new("valid expiry").unwrap(),
            None,
            now,
        )
        .unwrap()
    };
    let key = IdempotencyKey::new("last-representable-expiry").unwrap();
    let sent = store
        .send_message_command(&message(), Some(&key), last_valid)
        .await
        .unwrap();
    let replay = store
        .send_message_command(&message(), Some(&key), max - Duration::seconds(1))
        .await
        .unwrap_err();
    // Like SQL, even a replay rejects a now whose new retention interval cannot
    // be represented; it must not silently extend the key beyond the domain.
    assert!(matches!(
        replay,
        CreateCommandError::Storage(QueryStoreError::Unexpected(_))
    ));
    assert_eq!(sent.disposition, CreateDisposition::Created);
    let replay = store
        .send_message_command(&message(), Some(&key), last_valid)
        .await
        .unwrap();
    assert_eq!(replay.disposition, CreateDisposition::Replayed);
    assert_eq!(replay.value.id(), sent.value.id());
    assert_eq!(
        store
            .send_message_command(&message(), None, max)
            .await
            .unwrap()
            .disposition,
        CreateDisposition::Created
    );
    let command = ConversationCreateCommand {
        conversation: Conversation::new_group(
            ConversationId::new(),
            GroupTitle::new("last expiry").unwrap(),
            now,
        ),
        actor,
        participants: vec![],
    };
    assert_eq!(
        store
            .create_conversation_command(&command, Some(&key), last_valid)
            .await
            .unwrap()
            .disposition,
        CreateDisposition::Created
    );
    assert!(matches!(
        store
            .create_conversation_command(&command, Some(&key), max)
            .await,
        Err(CreateCommandError::Storage(QueryStoreError::Unexpected(_)))
    ));
    let unkeyed = ConversationCreateCommand {
        conversation: Conversation::new_group(
            ConversationId::new(),
            GroupTitle::new("unkeyed maximum now").unwrap(),
            now,
        ),
        ..command
    };
    assert_eq!(
        store
            .create_conversation_command(&unkeyed, None, max)
            .await
            .unwrap()
            .disposition,
        CreateDisposition::Created
    );
}
