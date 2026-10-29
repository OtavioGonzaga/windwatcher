#![cfg(feature = "mongodb")]

use mongodb::Client;
use windwatcher::{
    IdentityStore, MongoDbStorage, MongoDbStorageOptions, PasswordHash, User, UserId,
};

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

        let second = MongoDbStorage::connect(MongoDbStorageOptions::new(&url, &database))
            .await
            .unwrap();
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
