#![cfg(feature = "postgres")]

use sqlx::{ConnectOptions, Connection};
use windwatcher::{
    ConversationStore, IdentityStore, PasswordHasher, PostgresStorage, PostgresStorageOptions,
    RefreshTokenProvider, SessionStore,
};

#[path = "support/conversation_store_contract.rs"]
mod conversation_contract;
#[path = "support/idempotency_contract.rs"]
mod idempotency_contract;
#[path = "support/identity_store_contract.rs"]
mod identity_contract;
#[path = "support/message_store_contract.rs"]
mod message_contract;
#[path = "support/query_store_contract.rs"]
mod query_contract;
#[path = "support/session_store_contract.rs"]
mod session_contract;
#[macro_use]
#[path = "support/sql_authorization_races.rs"]
mod sql_authorization_races;

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
        let query_owner=windwatcher::User::new(windwatcher::UserId::new(), "query_owner".parse().unwrap(), now);
        let query_peer=windwatcher::User::new(windwatcher::UserId::new(), "query_peer".parse().unwrap(), now);
        let query_outsider=windwatcher::User::new(windwatcher::UserId::new(), "query_outsider".parse().unwrap(), now);
        for user in [&query_owner,&query_peer,&query_outsider] { storage.create_local_user(user,&hash).await.unwrap(); }
        query_contract::run(&storage, query_owner, query_peer, query_outsider).await;
        let command_actor = windwatcher::User::new(windwatcher::UserId::new(), "command_actor".parse().unwrap(), now);
        let command_peer = windwatcher::User::new(windwatcher::UserId::new(), "command_peer".parse().unwrap(), now);
        for user in [&command_actor, &command_peer] { storage.create_local_user(user, &hash).await.unwrap(); }
        idempotency_contract::run(&storage, command_actor.id(), command_peer.id()).await;
        let second_pool = PostgresStorage::connect(PostgresStorageOptions::new(&url)).await.unwrap();
        idempotency_contract::concurrent(&storage, &second_pool, command_actor.id(), command_peer.id()).await;
        let direct_a = windwatcher::Conversation::new_direct(windwatcher::ConversationId::new(), now);
        let direct_b = windwatcher::Conversation::new_direct(windwatcher::ConversationId::new(), now);
        let (direct_result_a, direct_result_b) = tokio::join!(storage.get_or_create_direct(&direct_a, owner.id(), other.id()), second_pool.get_or_create_direct(&direct_b, other.id(), owner.id()));
        let id_of = |outcome: windwatcher::StartDirectStoreOutcome| match outcome { windwatcher::StartDirectStoreOutcome::Created(c) | windwatcher::StartDirectStoreOutcome::Existing(c) => c.id() };
        assert_eq!(id_of(direct_result_a.unwrap()), id_of(direct_result_b.unwrap()));
        session_contract::run(&storage, owner.id(), other.id(), disabled.id(), windwatcher::UserId::new()).await;
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
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        authorization_races!(&second_pool, &pool, sqlx::Postgres, |id: uuid::Uuid| id, command_actor.id(), command_peer.id());
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
