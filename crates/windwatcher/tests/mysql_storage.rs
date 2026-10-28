#![cfg(feature = "mysql")]

use sqlx::{ConnectOptions, Connection};
use windwatcher::{
    ConversationStore, IdentityStore, MySqlStorage, MySqlStorageOptions, PasswordHasher,
    RefreshTokenProvider, SessionStore,
};

#[path = "support/conversation_store_contract.rs"]
mod conversation_contract;
#[path = "support/identity_store_contract.rs"]
mod identity_contract;
#[path = "support/message_store_contract.rs"]
mod message_contract;
#[path = "support/session_store_contract.rs"]
mod session_contract;

async fn disposable_database() -> (String, String) {
    let base = std::env::var("WINDWATCHER_TEST_MYSQL_URL")
        .expect("set WINDWATCHER_TEST_MYSQL_URL to a disposable MySQL 8 server");
    let name = format!("windwatcher_storage_{}", uuid::Uuid::now_v7().simple());
    let mut connection = sqlx::MySqlConnection::connect(&base).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE `{name}`")))
        .execute(&mut connection)
        .await
        .unwrap();
    let options: sqlx::mysql::MySqlConnectOptions = base.parse().unwrap();
    (options.database(&name).to_url_lossy().to_string(), name)
}

async fn drop_database(name: &str) {
    assert!(
        name.starts_with("windwatcher_storage_")
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    );
    let base = std::env::var("WINDWATCHER_TEST_MYSQL_URL").unwrap();
    let mut connection = sqlx::MySqlConnection::connect(&base).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP DATABASE `{name}`")))
        .execute(&mut connection)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires a disposable MySQL 8 server; set WINDWATCHER_TEST_MYSQL_URL"]
async fn explicit_migration_lifecycle_and_physical_schema() {
    let (url, name) = disposable_database().await;
    let result = async {
        let storage = MySqlStorage::connect(MySqlStorageOptions::new(&url)).await.unwrap();
        assert_eq!(storage.migration_status().await.unwrap().pending(), 4);
        storage.migrate().await.unwrap();
        storage.migrate().await.unwrap();
        assert!(storage.check_schema().await.unwrap().is_current());
        identity_contract::run(&storage).await;
        let hasher = windwatcher::Argon2idPasswordHasher;
        let hash = hasher.hash(&windwatcher::SecretPassword::new("integration password".to_owned()).unwrap()).await.unwrap();
        let now = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let owner = windwatcher::User::new(windwatcher::UserId::new(), "owner_one".parse().unwrap(), now);
        let other = windwatcher::User::new(windwatcher::UserId::new(), "owner_two".parse().unwrap(), now);
        let mut disabled = windwatcher::User::new(windwatcher::UserId::new(), "owner_disabled".parse().unwrap(), now);
        disabled.disable(now);
        let admin = windwatcher::User::new(windwatcher::UserId::new(), "conv_admin".parse().unwrap(), now);
        let member = windwatcher::User::new(windwatcher::UserId::new(), "conv_member".parse().unwrap(), now);
        let outsider = windwatcher::User::new(windwatcher::UserId::new(), "conv_outsider".parse().unwrap(), now);
        for user in [&owner, &other, &disabled, &admin, &member, &outsider] { storage.create_local_user(user, &hash).await.unwrap(); }
        conversation_contract::run(&storage, owner.id(), admin.id(), member.id(), outsider.id()).await;
        message_contract::run(&storage, owner.id(), admin.id(), member.id(), outsider.id()).await;
        session_contract::run(&storage, owner.id(), other.id(), disabled.id(), windwatcher::UserId::new()).await;
        let second_pool = MySqlStorage::connect(MySqlStorageOptions::new(&url)).await.unwrap();
        let direct_a = windwatcher::Conversation::new_direct(windwatcher::ConversationId::new(), now);
        let direct_b = windwatcher::Conversation::new_direct(windwatcher::ConversationId::new(), now);
        let (a, b) = tokio::join!(storage.get_or_create_direct(&direct_a, owner.id(), other.id()), second_pool.get_or_create_direct(&direct_b, other.id(), owner.id()));
        let id_of = |v: windwatcher::StartDirectStoreOutcome| match v { windwatcher::StartDirectStoreOutcome::Created(c) | windwatcher::StartDirectStoreOutcome::Existing(c) => c.id() };
        assert_eq!(id_of(a.unwrap()), id_of(b.unwrap()));
        let first_race = windwatcher::User::new(windwatcher::UserId::new(), "race_username".parse().unwrap(), now);
        let second_race = windwatcher::User::new(windwatcher::UserId::new(), "race_username".parse().unwrap(), now);
        let (left, right) = tokio::join!(storage.create_local_user(&first_race, &hash), second_pool.create_local_user(&second_race, &hash));
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
        assert!(matches!(left, Err(windwatcher::IdentityStoreError::UsernameConflict)) || matches!(right, Err(windwatcher::IdentityStoreError::UsernameConflict)));
        let provider = windwatcher::OsRefreshTokenProvider;
        let old = provider.generate().unwrap();
        let replacement_a = provider.generate().unwrap();
        let replacement_b = provider.generate().unwrap();
        let session = windwatcher::Session::new(windwatcher::SessionId::new(), owner.id(), now, now + time::Duration::days(1)).unwrap();
        let old_hash = provider.hash(&old);
        let replacement_a_hash = provider.hash(&replacement_a);
        let replacement_b_hash = provider.hash(&replacement_b);
        storage.create_session(&session, &old_hash).await.unwrap();
        let rotate_at = now + time::Duration::seconds(1);
        let (a, b) = tokio::join!(storage.rotate_refresh_token(session.id(), &old_hash, &replacement_a_hash, rotate_at), second_pool.rotate_refresh_token(session.id(), &old_hash, &replacement_b_hash, rotate_at));
        assert_eq!(usize::from(a.unwrap() == windwatcher::RefreshRotationResult::Rotated) + usize::from(b.unwrap() == windwatcher::RefreshRotationResult::Rotated), 1);
        conversation_contract::disabled_memberships(&storage, owner.clone(), other.clone(), disabled.clone()).await;
        let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
        let types: Vec<(String, String, String)> = sqlx::query_as("SELECT table_name, column_name, data_type FROM information_schema.columns WHERE table_schema=DATABASE()")
            .fetch_all(&pool).await.unwrap();
        for (table, field) in [("users", "created_at"), ("sessions", "expires_at"), ("messages", "edited_at"), ("idempotency_records", "expires_at")] {
            assert!(types.iter().any(|(t,c,d)|t==table&&c==field&&d=="bigint"), "missing BIGINT {table}.{field}");
        }
        assert!(types.iter().any(|(t,c,d)|t=="users"&&c=="id"&&d=="binary"));
        assert!(types.iter().any(|(t,c,d)|t=="conversations"&&c=="title"&&d=="longblob"));
        assert!(types.iter().any(|(t,c,d)|t=="messages"&&c=="content"&&d=="longtext"));
        pool.close().await;
        Ok::<_, ()>(())
    }.await;
    drop_database(&name).await;
    result.unwrap();
}

#[test]
fn connection_options_redact_credentials() {
    let options = MySqlStorageOptions::new("mysql://private:secret@localhost/windwatcher");
    let rendered = format!("{options:?}");
    assert!(!rendered.contains("private"));
    assert!(!rendered.contains("secret"));
    assert!(rendered.contains("REDACTED"));
}
