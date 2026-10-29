//! Real replica-set regressions with driver events as synchronization points.
//! No sleeps, production test hooks, or failpoints are needed.
use std::{collections::HashSet, time::Duration};

use mongodb::{
    Client,
    bson::{Binary, Bson, Document, doc, spec::BinarySubtype},
    event::{EventHandler, command::CommandEvent},
    options::{ClientOptions, ReadConcern, WriteConcern},
};
use tokio::sync::mpsc;

use super::{MongoDbStorage, MongoDbStorageOptions, conversations};
use crate::*;

fn bin(id: uuid::Uuid) -> Binary {
    Binary {
        subtype: BinarySubtype::Uuid,
        bytes: id.as_bytes().to_vec(),
    }
}

async fn corruption_fixture(label: &str) -> (MongoDbStorage, User, User, time::OffsetDateTime) {
    let url = std::env::var("WINDWATCHER_TEST_MONGODB_URL").unwrap();
    let database = format!("windwatcher_{label}_{}", uuid::Uuid::now_v7().simple());
    let storage = MongoDbStorage::connect(MongoDbStorageOptions::new(&url, &database))
        .await
        .unwrap();
    storage.migrate().await.unwrap();
    let now = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let hash = PasswordHash::parse(
        "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();
    let owner = User::new(UserId::new(), "owner".parse().unwrap(), now);
    let actor = User::new(UserId::new(), "actor".parse().unwrap(), now);
    for user in [&owner, &actor] {
        storage.create_local_user(user, &hash).await.unwrap();
    }
    (storage, owner, actor, now)
}

async fn stored_state(storage: &MongoDbStorage) -> Vec<Vec<Document>> {
    use futures_util::TryStreamExt;
    let mut state = Vec::new();
    for collection in [
        "users",
        "sessions",
        "conversations",
        "conversation_members",
        "messages",
        "idempotency_records",
    ] {
        state.push(
            storage
                .database()
                .collection::<Document>(collection)
                .find(doc! {})
                .sort(doc! { "_id": 1 })
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap(),
        );
    }
    state
}

#[tokio::test]
#[ignore = "requires a disposable replica set; set WINDWATCHER_TEST_MONGODB_URL"]
async fn incompatible_membership_roles_fail_closed_without_effects() {
    let (storage, peer, actor, now) = corruption_fixture("roles").await;
    for (kind, role) in [
        (ConversationKind::Group, "direct"),
        (ConversationKind::Direct, "member"),
        (ConversationKind::Direct, "admin"),
        (ConversationKind::Direct, "owner"),
    ] {
        let conversation = match kind {
            ConversationKind::Group => Conversation::new_group(
                ConversationId::new(),
                GroupTitle::new("corruption").unwrap(),
                now,
            ),
            ConversationKind::Direct => Conversation::new_direct(ConversationId::new(), now),
        };
        let command = ConversationCreateCommand {
            conversation,
            actor: actor.id(),
            participants: vec![peer.id()],
        };
        let create_key = IdempotencyKey::new(format!("corrupt-create-{role}")).unwrap();
        let conversation = storage
            .create_conversation_command(&command, Some(&create_key), now)
            .await
            .unwrap()
            .value;
        if kind == ConversationKind::Group {
            storage
                .set_group_member_role(conversation, actor.id(), peer.id(), GroupRole::Admin, now)
                .await
                .unwrap();
        }
        let message = Message::new(
            MessageId::new(),
            conversation,
            actor.id(),
            MessageContent::new("unchanged").unwrap(),
            None,
            now,
        )
        .unwrap();
        let send_key = IdempotencyKey::new(format!("corrupt-send-{role}")).unwrap();
        storage
            .send_message_command(&message, Some(&send_key), now)
            .await
            .unwrap();
        let peer_message = Message::new(
            MessageId::new(),
            conversation,
            peer.id(),
            MessageContent::new("not moderated").unwrap(),
            None,
            now,
        )
        .unwrap();
        storage.send_message(&peer_message).await.unwrap();
        let members = storage
            .database()
            .collection::<Document>("conversation_members");
        let filter = doc! { "conversation_id": bin(conversation.as_uuid()), "user_id": bin(actor.id().as_uuid()) };
        let original = members.find_one(filter.clone()).await.unwrap().unwrap();
        members
            .update_one(filter.clone(), doc! { "$set": { "role": role } })
            .await
            .unwrap();
        let before = stored_state(&storage).await;
        let limit = PageLimit::default();

        assert!(matches!(
            storage.find_member(conversation, actor.id()).await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage.member_ids(conversation).await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage
                .get_conversation_summary(actor.id(), conversation)
                .await,
            Err(QueryStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage.list_conversations(actor.id(), None, limit).await,
            Err(QueryStoreError::Unexpected(_))
        ));
        // The actor and the projected target must both be validated.
        for reader in [actor.id(), peer.id()] {
            assert!(matches!(
                storage
                    .get_conversation_member(reader, conversation, actor.id())
                    .await,
                Err(QueryStoreError::Unexpected(_))
            ));
            assert!(matches!(
                storage
                    .list_conversation_members(reader, conversation, None, limit)
                    .await,
                Err(QueryStoreError::Unexpected(_))
            ));
        }
        assert!(matches!(
            storage.get_message(actor.id(), message.id()).await,
            Err(MessageStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage
                .list_messages(actor.id(), conversation, None, limit)
                .await,
            Err(MessageStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage.unread_count(actor.id(), conversation).await,
            Err(MessageStoreError::Unexpected(_))
        ));
        let fresh = Message::new(
            MessageId::new(),
            conversation,
            actor.id(),
            MessageContent::new("must not persist").unwrap(),
            None,
            now,
        )
        .unwrap();
        assert!(matches!(
            storage.send_message(&fresh).await,
            Err(MessageStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage
                .edit_message(
                    actor.id(),
                    message.id(),
                    &MessageContent::new("must not persist").unwrap(),
                    now
                )
                .await,
            Err(MessageStoreError::Unexpected(_))
        ));
        // In a Direct, corrupt owner/admin must never confer moderation power.
        assert!(matches!(
            storage
                .delete_message(actor.id(), peer_message.id(), now)
                .await,
            Err(MessageStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage
                .mark_read_through(actor.id(), conversation, message.id())
                .await,
            Err(MessageStoreError::Unexpected(_))
        ));
        for key in [None, Some(&send_key)] {
            assert!(matches!(
                storage.send_message_command(&message, key, now).await,
                Err(CreateCommandError::Storage(QueryStoreError::Unexpected(_)))
            ));
        }
        let fresh_key = IdempotencyKey::new("new-corrupt-send").unwrap();
        assert!(matches!(
            storage
                .send_message_command(&fresh, Some(&fresh_key), now)
                .await,
            Err(CreateCommandError::Storage(QueryStoreError::Unexpected(_)))
        ));
        assert!(matches!(
            storage
                .create_conversation_command(&command, Some(&create_key), now)
                .await,
            Err(CreateCommandError::Storage(QueryStoreError::Unexpected(_)))
        ));
        assert!(matches!(
            storage
                .add_group_member(conversation, actor.id(), peer.id(), now)
                .await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage
                .remove_group_member(conversation, actor.id(), peer.id(), now)
                .await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage
                .set_group_member_role(conversation, actor.id(), peer.id(), GroupRole::Admin, now)
                .await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage
                .transfer_group_ownership(conversation, actor.id(), peer.id(), now)
                .await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage
                .rename_group(
                    conversation,
                    actor.id(),
                    &GroupTitle::new("must not persist").unwrap(),
                    now
                )
                .await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage.leave_group(conversation, actor.id(), now).await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        if kind == ConversationKind::Group {
            // A valid moderator must not silently ignore or overwrite a corrupt
            // target membership via the target-role filters either.
            assert!(matches!(
                storage
                    .add_group_member(conversation, peer.id(), actor.id(), now)
                    .await,
                Err(ConversationStoreError::Unexpected(_))
            ));
            assert!(matches!(
                storage
                    .remove_group_member(conversation, peer.id(), actor.id(), now)
                    .await,
                Err(ConversationStoreError::Unexpected(_))
            ));
            assert!(matches!(
                storage
                    .set_group_member_role(
                        conversation,
                        peer.id(),
                        actor.id(),
                        GroupRole::Member,
                        now
                    )
                    .await,
                Err(ConversationStoreError::Unexpected(_))
            ));
        } else {
            assert!(matches!(
                storage
                    .get_or_create_direct(&command.conversation, actor.id(), peer.id())
                    .await,
                Err(ConversationStoreError::Unexpected(_))
            ));
            assert!(matches!(
                storage
                    .create_conversation_command(&command, None, now)
                    .await,
                Err(CreateCommandError::Storage(QueryStoreError::Unexpected(_)))
            ));
        }
        assert_eq!(
            stored_state(&storage).await,
            before,
            "corrupt role {role} changed persisted state"
        );
        members.replace_one(filter, original).await.unwrap();
    }
    storage.database().drop().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a disposable replica set; set WINDWATCHER_TEST_MONGODB_URL"]
async fn malformed_disabled_timestamps_cannot_authorize_new_effects() {
    let (storage, owner, actor, now) = corruption_fixture("disabled").await;
    let group = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("existing").unwrap(),
        now,
    );
    storage.create_group(&group, owner.id(), &[]).await.unwrap();
    let users = storage.database().collection::<Document>("users");
    let filter = doc! { "_id": bin(actor.id().as_uuid()) };
    for value in [
        None,
        Some(Bson::String("invalid".into())),
        Some(Bson::Int32(1)),
        Some(Bson::Int64(i64::MAX)),
    ] {
        match value {
            None => users
                .update_one(filter.clone(), doc! { "$unset": { "disabled_at": "" } })
                .await
                .unwrap(),
            Some(value) => users
                .update_one(filter.clone(), doc! { "$set": { "disabled_at": value } })
                .await
                .unwrap(),
        };
        let before = stored_state(&storage).await;
        assert!(matches!(
            storage.find_user(actor.id()).await,
            Err(IdentityStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage.find_users(actor.username()).await,
            Err(QueryStoreError::Unexpected(_))
        ));
        let session = Session::new(
            SessionId::new(),
            actor.id(),
            now,
            now + time::Duration::hours(1),
        )
        .unwrap();
        let provider = OsRefreshTokenProvider;
        let hash = provider.hash(&provider.generate().unwrap());
        assert!(matches!(
            storage.create_session(&session, &hash).await,
            Err(SessionStoreError::Unexpected(_))
        ));
        let direct = Conversation::new_direct(ConversationId::new(), now);
        assert!(matches!(
            storage
                .get_or_create_direct(&direct, owner.id(), actor.id())
                .await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        let candidate = Conversation::new_group(
            ConversationId::new(),
            GroupTitle::new("must not persist").unwrap(),
            now,
        );
        assert!(matches!(
            storage
                .create_group(&candidate, owner.id(), &[actor.id()])
                .await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        assert!(matches!(
            storage
                .add_group_member(group.id(), owner.id(), actor.id(), now)
                .await,
            Err(ConversationStoreError::Unexpected(_))
        ));
        let key = IdempotencyKey::new("corrupt-user-create").unwrap();
        let command = ConversationCreateCommand {
            conversation: candidate,
            actor: owner.id(),
            participants: vec![actor.id()],
        };
        assert!(matches!(
            storage
                .create_conversation_command(&command, Some(&key), now)
                .await,
            Err(CreateCommandError::Storage(QueryStoreError::Unexpected(_)))
        ));
        assert_eq!(stored_state(&storage).await, before);
    }
    storage.database().drop().await.unwrap();
}

async fn observed_storage(
    url: &str,
    database: &str,
) -> (MongoDbStorage, mpsc::UnboundedReceiver<CommandEvent>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let mut options = ClientOptions::parse(url).await.unwrap();
    options.write_concern = Some(WriteConcern::majority());
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        let _ = sender.send(event);
    }));
    (
        MongoDbStorage {
            client: Client::with_options(options).unwrap(),
            database: database.to_owned(),
        },
        receiver,
    )
}

// A membership writer already holds the conversation lock. A correct message
// writer conflicts on that document and retries. The old implementation instead
// succeeds in reading the pre-change membership from a stale snapshot. Observe
// either event, then let the membership writer commit before checking the result.
async fn authorization_attempt(events: &mut mpsc::UnboundedReceiver<CommandEvent>) {
    let mut membership_reads = HashSet::new();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match events.recv().await.expect("command stream closed") {
                CommandEvent::Started(event)
                    if event.command.get_str("find").ok() == Some("conversation_members") =>
                {
                    membership_reads.insert(event.request_id);
                }
                CommandEvent::Succeeded(event) if membership_reads.contains(&event.request_id) => {
                    break;
                }
                CommandEvent::Failed(event)
                    if event.command_name == "update"
                        && event.failure.contains_label("TransientTransactionError") =>
                {
                    break;
                }
                _ => (),
            }
        }
    })
    .await
    .expect("message operation did not reach authorization");
}

#[tokio::test]
#[ignore = "requires a disposable replica set; set WINDWATCHER_TEST_MONGODB_URL"]
async fn message_authorization_serializes_with_removal_and_demotion() {
    let url = std::env::var("WINDWATCHER_TEST_MONGODB_URL").unwrap();
    let database = format!(
        "windwatcher_authorization_{}",
        uuid::Uuid::now_v7().simple()
    );
    let storage = MongoDbStorage::connect(MongoDbStorageOptions::new(&url, &database))
        .await
        .unwrap();
    storage.migrate().await.unwrap();
    let db = storage.database();
    let now = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let hash = PasswordHash::parse(
        "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();
    let owner = User::new(UserId::new(), "owner".parse().unwrap(), now);
    let actor = User::new(UserId::new(), "actor".parse().unwrap(), now);
    for user in [&owner, &actor] {
        storage.create_local_user(user, &hash).await.unwrap();
    }

    for operation in ["send", "keyed_send", "edit", "delete"] {
        let group = Conversation::new_group(
            ConversationId::new(),
            GroupTitle::new(operation).unwrap(),
            now,
        );
        storage
            .create_group(&group, owner.id(), &[actor.id()])
            .await
            .unwrap();
        let message = Message::new(
            MessageId::new(),
            group.id(),
            if operation == "delete" {
                owner.id()
            } else {
                actor.id()
            },
            MessageContent::new("original").unwrap(),
            None,
            now,
        )
        .unwrap();
        if matches!(operation, "edit" | "delete") {
            storage.send_message(&message).await.unwrap();
        }
        if operation == "delete" {
            storage
                .set_group_member_role(group.id(), owner.id(), actor.id(), GroupRole::Admin, now)
                .await
                .unwrap();
        }
        let (writer, mut events) = observed_storage(&url, &database).await;
        let mut session = storage.client.start_session().await.unwrap();
        session
            .start_transaction()
            .read_concern(ReadConcern::snapshot())
            .write_concern(WriteConcern::majority())
            .await
            .unwrap();
        conversations::touch(&db, &mut session, group.id())
            .await
            .unwrap();
        let members = db.collection::<Document>("conversation_members");
        let filter = doc! { "conversation_id": bin(group.id().as_uuid()), "user_id": bin(actor.id().as_uuid()) };
        if operation == "delete" {
            members
                .update_one(filter, doc! { "$set": { "role": "member" } })
                .session(&mut session)
                .await
                .unwrap();
        } else {
            members
                .delete_one(filter)
                .session(&mut session)
                .await
                .unwrap();
        }

        let actor_id = actor.id();
        let id = message.id();
        let pending = tokio::spawn(async move {
            match operation {
                "send" => assert!(matches!(
                    writer.send_message(&message).await.unwrap(),
                    SendMessageStoreOutcome::NotMember
                )),
                "keyed_send" => assert!(matches!(
                    writer
                        .send_message_command(
                            &message,
                            Some(&IdempotencyKey::new("race").unwrap()),
                            now
                        )
                        .await,
                    Err(CreateCommandError::NotMember)
                )),
                "edit" => assert!(matches!(
                    writer
                        .edit_message(actor_id, id, &MessageContent::new("edited").unwrap(), now)
                        .await
                        .unwrap(),
                    EditMessageStoreOutcome::NotMember
                )),
                "delete" => assert!(matches!(
                    writer.delete_message(actor_id, id, now).await.unwrap(),
                    DeleteMessageStoreOutcome::NotAuthorized
                )),
                _ => unreachable!(),
            }
        });
        authorization_attempt(&mut events).await;
        session.commit_transaction().await.unwrap();
        tokio::time::timeout(Duration::from_secs(20), pending)
            .await
            .unwrap()
            .unwrap();
        let persisted = db
            .collection::<Document>("messages")
            .find_one(doc! { "_id": bin(id.as_uuid()) })
            .await
            .unwrap();
        if matches!(operation, "send" | "keyed_send") {
            assert!(persisted.is_none());
        } else {
            assert_eq!(persisted.unwrap().get_str("content").unwrap(), "original");
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a disposable replica set; set WINDWATCHER_TEST_MONGODB_URL"]
async fn paginated_queries_bound_results_and_round_trips() {
    let url = std::env::var("WINDWATCHER_TEST_MONGODB_URL").unwrap();
    let database = format!("windwatcher_pagination_{}", uuid::Uuid::now_v7().simple());
    let storage = MongoDbStorage::connect(MongoDbStorageOptions::new(&url, &database))
        .await
        .unwrap();
    storage.migrate().await.unwrap();
    let now = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let hash = PasswordHash::parse(
        "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();
    let mut users = Vec::new();
    for i in 0..24 {
        let user = User::new(UserId::new(), format!("user_{i}").parse().unwrap(), now);
        storage.create_local_user(&user, &hash).await.unwrap();
        users.push(user.id());
    }
    for i in 0..24 {
        let group = Conversation::new_group(
            ConversationId::new(),
            GroupTitle::new(format!("group {i}")).unwrap(),
            now,
        );
        storage
            .create_group(&group, users[0], &users[1..])
            .await
            .unwrap();
        let original = Message::new(
            MessageId::new(),
            group.id(),
            users[1],
            MessageContent::new("original").unwrap(),
            None,
            now,
        )
        .unwrap();
        storage.send_message(&original).await.unwrap();
        let reply = Message::new(
            MessageId::new(),
            group.id(),
            users[2],
            MessageContent::new("reply").unwrap(),
            Some(original.id()),
            now,
        )
        .unwrap();
        storage.send_message(&reply).await.unwrap();
    }
    let (reader, mut events) = observed_storage(&url, &database).await;
    for limit in [1, 10] {
        let page = reader
            .list_conversations(users[0], None, PageLimit::new(limit).unwrap())
            .await
            .unwrap();
        assert_eq!(page.items.len(), limit);
        assert!(page.has_more);
        assert!(
            page.items.iter().all(|summary| summary
                .last_message
                .as_ref()
                .unwrap()
                .reply_to
                .is_some())
        );
        let mut reads = 0;
        let mut returned = 0;
        while let Ok(event) = events.try_recv() {
            if let CommandEvent::Started(event) = &event
                && matches!(
                    event.command_name.as_str(),
                    "find" | "aggregate" | "getMore"
                )
            {
                reads += 1;
            }
            if let CommandEvent::Succeeded(event) = event
                && let Ok(cursor) = event.reply.get_document("cursor")
                && let Ok(batch) = cursor.get_array("firstBatch")
            {
                returned += batch.len();
            }
        }
        assert_eq!(
            reads, 3,
            "one aggregation plus batched reply and user hydration"
        );
        assert!(
            returned <= (limit + 1) * 4,
            "unbounded result materialization: {returned}"
        );
        let group = page.items[0].conversation.id();
        let members = reader
            .list_conversation_members(users[0], group, None, PageLimit::new(limit).unwrap())
            .await
            .unwrap();
        let ConversationQueryOutcome::Found(members) = members else {
            panic!("missing members");
        };
        assert_eq!(members.items.len(), limit);
        assert!(members.has_more);
        let mut reads = 0;
        while let Ok(event) = events.try_recv() {
            if let CommandEvent::Started(event) = event
                && matches!(
                    event.command_name.as_str(),
                    "find" | "aggregate" | "getMore"
                )
            {
                reads += 1;
            }
        }
        assert_eq!(
            reads, 3,
            "two authorization reads plus one bounded member aggregation"
        );
    }
    storage.database().drop().await.unwrap();
}
