#![cfg(feature = "postgres")]

use sqlx::{ConnectOptions, Connection};
use windwatcher::{
    IdentityStore, PasswordHasher, PostgresStorage, PostgresStorageOptions, RefreshTokenProvider,
    SessionStore,
};

#[path = "support/identity_store_contract.rs"]
mod identity_contract;
#[path = "support/session_store_contract.rs"]
mod session_contract;

async fn disposable_database() -> (String, String) {
    let base = std::env::var("WINDWATCHER_TEST_POSTGRES_URL")
        .expect("set WINDWATCHER_TEST_POSTGRES_URL to a disposable PostgreSQL server");
    let name = format!("windwatcher_storage_{}", uuid::Uuid::now_v7().simple());
    let mut connection = sqlx::PgConnection::connect(&base).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&mut connection)
        .await
        .unwrap();
    let options: sqlx::postgres::PgConnectOptions = base.parse().unwrap();
    (options.database(&name).to_url_lossy().to_string(), name)
}

async fn drop_database(name: &str) {
    assert!(
        name.starts_with("windwatcher_storage_")
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    );
    let base = std::env::var("WINDWATCHER_TEST_POSTGRES_URL").unwrap();
    let mut connection = sqlx::PgConnection::connect(&base).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {name} WITH (FORCE)"
    )))
    .execute(&mut connection)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server; set WINDWATCHER_TEST_POSTGRES_URL"]
async fn explicit_migration_lifecycle_and_physical_schema() {
    let (url, name) = disposable_database().await;
    let result = async {
        let storage = PostgresStorage::connect(PostgresStorageOptions::new(&url)).await.unwrap();
        assert_eq!(storage.migration_status().await.unwrap().pending(), 4);
        assert_eq!(storage.migration_status().await.unwrap().current_version(), None);
        storage.migrate().await.unwrap();
        storage.migrate().await.unwrap();
        assert!(storage.check_schema().await.unwrap().is_current());
        identity_contract::run(&storage).await;
        let hasher = windwatcher::Argon2idPasswordHasher::default();
        let hash = hasher.hash(&windwatcher::SecretPassword::new("integration password".to_owned()).unwrap()).await.unwrap();
        let now = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let owner = windwatcher::User::new(windwatcher::UserId::new(), "owner_one".parse().unwrap(), now);
        let other = windwatcher::User::new(windwatcher::UserId::new(), "owner_two".parse().unwrap(), now);
        let mut disabled = windwatcher::User::new(windwatcher::UserId::new(), "owner_disabled".parse().unwrap(), now);
        disabled.disable(now);
        for user in [&owner, &other, &disabled] { storage.create_local_user(user, &hash).await.unwrap(); }
        session_contract::run(&storage, owner.id(), other.id(), disabled.id(), windwatcher::UserId::new()).await;
        let second_pool = PostgresStorage::connect(PostgresStorageOptions::new(&url)).await.unwrap();
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
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        let columns: Vec<(String, String)> = sqlx::query_as("SELECT table_name || '.' || column_name, data_type FROM information_schema.columns WHERE table_schema=current_schema()")
            .fetch_all(&pool).await.unwrap();
        for field in ["users.created_at", "sessions.expires_at", "conversations.updated_at", "messages.edited_at", "idempotency_records.expires_at"] {
            assert!(columns.contains(&(field.into(), "bigint".into())), "missing BIGINT {field}");
        }
        assert!(columns.contains(&("users.id".into(), "uuid".into())));
        assert!(columns.contains(&("conversations.title".into(), "bytea".into())));
        assert!(columns.contains(&("messages.content".into(), "text".into())));
        pool.close().await;
        Ok::<_, ()>(())
    }.await;
    drop_database(&name).await;
    result.unwrap();
}

#[test]
fn connection_options_redact_credentials() {
    let options = PostgresStorageOptions::new("postgres://private:secret@localhost/windwatcher");
    let rendered = format!("{options:?}");
    assert!(!rendered.contains("private"));
    assert!(!rendered.contains("secret"));
    assert!(rendered.contains("REDACTED"));
}
