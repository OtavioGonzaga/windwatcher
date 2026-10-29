#![cfg(feature = "mongodb")]

use mongodb::Client;
use windwatcher::{
    ConversationStore, IdentityStore, MongoDbStorage, MongoDbStorageOptions, PasswordHash, User,
    UserId,
};

#[path = "support/conversation_store_contract.rs"]
mod conversation_contract;
#[path = "support/identity_store_contract.rs"]
mod identity_contract;
#[path = "support/session_store_contract.rs"]
mod session_contract;

#[tokio::test]
#[ignore = "requires a disposable MongoDB replica set; set WINDWATCHER_TEST_MONGODB_URL"]
async fn identity_and_session_contracts_on_disposable_database() {
    let url = std::env::var("WINDWATCHER_TEST_MONGODB_URL")
        .expect("set WINDWATCHER_TEST_MONGODB_URL to a disposable replica set");
    let database = format!("windwatcher_storage_{}", uuid::Uuid::now_v7().simple());
    let storage = MongoDbStorage::connect(MongoDbStorageOptions::new(&url, &database))
        .await
        .unwrap();
    let client = Client::with_uri_str(&url).await.unwrap();

    let result = async {
        assert_eq!(storage.migration_status().await.unwrap().pending(), 4);
        assert_eq!(
            storage.migration_status().await.unwrap().current_version(),
            None
        );
        storage.migrate().await.unwrap();
        storage.migrate().await.unwrap();
        assert!(storage.check_schema().await.unwrap().is_current());

        identity_contract::run(&storage).await;

        let hash = PasswordHash::parse(
            "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA",
        )
        .unwrap();
        let now = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let owner = User::new(UserId::new(), "session_owner".parse().unwrap(), now);
        let other = User::new(UserId::new(), "session_other".parse().unwrap(), now);
        let mut disabled = User::new(UserId::new(), "session_disabled".parse().unwrap(), now);
        disabled.disable(now);
        for user in [&owner, &other, &disabled] {
            storage.create_local_user(user, &hash).await.unwrap();
        }
        session_contract::run(
            &storage,
            owner.id(),
            other.id(),
            disabled.id(),
            UserId::new(),
        )
        .await;

        let (owner, admin, member, outsider) = (
            User::new(UserId::new(), "conversation_owner".parse().unwrap(), now),
            User::new(UserId::new(), "conversation_admin".parse().unwrap(), now),
            User::new(UserId::new(), "conversation_member".parse().unwrap(), now),
            User::new(UserId::new(), "conversation_outsider".parse().unwrap(), now),
        );
        for user in [&owner, &admin, &member, &outsider] {
            storage.create_local_user(user, &hash).await.unwrap();
        }
        conversation_contract::run(&storage, owner.id(), admin.id(), member.id(), outsider.id())
            .await;
        conversation_contract::disabled_memberships(
            &storage,
            owner.clone(),
            admin.clone(),
            outsider.clone(),
        )
        .await;

        let second = MongoDbStorage::connect(MongoDbStorageOptions::new(&url, &database))
            .await
            .unwrap();
        let direct_left = User::new(UserId::new(), "direct_race_left".parse().unwrap(), now);
        let direct_right = User::new(UserId::new(), "direct_race_right".parse().unwrap(), now);
        for user in [&direct_left, &direct_right] {
            storage.create_local_user(user, &hash).await.unwrap();
        }
        let left_conversation =
            windwatcher::Conversation::new_direct(windwatcher::ConversationId::new(), now);
        let right_conversation =
            windwatcher::Conversation::new_direct(windwatcher::ConversationId::new(), now);
        let (left, right) = tokio::join!(
            storage.get_or_create_direct(&left_conversation, direct_left.id(), direct_right.id()),
            second.get_or_create_direct(&right_conversation, direct_right.id(), direct_left.id())
        );
        let left = left.unwrap();
        let right = right.unwrap();
        let left_id = match left {
            windwatcher::StartDirectStoreOutcome::Created(value)
            | windwatcher::StartDirectStoreOutcome::Existing(value) => value.id(),
        };
        let right_id = match right {
            windwatcher::StartDirectStoreOutcome::Created(value)
            | windwatcher::StartDirectStoreOutcome::Existing(value) => value.id(),
        };
        assert_eq!(left_id, right_id);

        let left = User::new(UserId::new(), "race_username".parse().unwrap(), now);
        let right = User::new(UserId::new(), "race_username".parse().unwrap(), now);
        let (a, b) = tokio::join!(
            storage.create_local_user(&left, &hash),
            second.create_local_user(&right, &hash)
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert!(
            matches!(a, Err(windwatcher::IdentityStoreError::UsernameConflict))
                || matches!(b, Err(windwatcher::IdentityStoreError::UsernameConflict))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    }
    .await;

    drop(storage);
    client.database(&database).drop().await.unwrap();
    result.unwrap();
}
