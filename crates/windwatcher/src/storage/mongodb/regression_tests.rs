//! Real replica-set regressions with driver events as synchronization points.
//! No sleeps, production test hooks, or failpoints are needed.
use std::{collections::HashSet, time::Duration};

use mongodb::{
    Client,
    bson::{Binary, Document, doc, spec::BinarySubtype},
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
