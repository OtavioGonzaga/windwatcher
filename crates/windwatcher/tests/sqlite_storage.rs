#![cfg(feature = "sqlite")]

use std::{sync::Arc, time::Duration as StdDuration};

use sqlx::{Connection, Executor, Row, SqliteConnection};
use tempfile::TempDir;
use time::{Duration, OffsetDateTime};
use windwatcher::{
    AddGroupMember, ChangeStoreOutcome, ConversationStore, CreateGroupConversation, GroupRole,
    IdentityStore, IdentityStoreError, LeaveGroupConversation, LeaveStoreOutcome,
    OsRefreshTokenProvider, PasswordHash, RefreshRotationResult, RefreshToken,
    RefreshTokenProvider, Session, SessionId, SessionStore, SetGroupMemberRole, SqliteStorage,
    SqliteStorageError, SqliteStorageOptions, StartDirectConversation, StartDirectStoreOutcome,
    TransferGroupOwnership, User, UserId,
};

mod support;

const HASH: &str = "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA";

fn timestamp(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}

async fn store() -> (TempDir, SqliteStorage) {
    let directory = TempDir::new().unwrap();
    let url = format!(
        "sqlite://{}",
        directory.path().join("windwatcher.db").display()
    );
    let storage = SqliteStorage::connect(
        SqliteStorageOptions::new(url)
            .max_connections(4)
            .busy_timeout(StdDuration::from_secs(2)),
    )
    .await
    .unwrap();
    storage.migrate().await.unwrap();
    (directory, storage)
}

fn user(name: &str, now: OffsetDateTime) -> User {
    User::new(UserId::new(), name.parse().unwrap(), now)
}

fn password_hash() -> PasswordHash {
    PasswordHash::parse(HASH).unwrap()
}

async fn persisted_user(storage: &SqliteStorage, name: &str) -> User {
    let user = user(name, timestamp(1_700_000_000) + Duration::nanoseconds(999));
    storage
        .create_local_user(&user, &password_hash())
        .await
        .unwrap();
    user
}

#[tokio::test]
async fn migration_is_explicit_idempotent_and_reports_status() {
    let directory = TempDir::new().unwrap();
    let url = format!("sqlite://{}", directory.path().join("fresh.db").display());
    let storage = SqliteStorage::connect(SqliteStorageOptions::new(url.clone()))
        .await
        .unwrap();

    let status = storage.migration_status().await.unwrap();
    assert_eq!(status.current_version(), None);
    assert_eq!(status.latest_version(), Some(2));
    assert_eq!(status.pending(), 2);
    assert!(!status.is_current());
    let inspector = sqlx::SqlitePool::connect(&url).await.unwrap();
    let tables: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'users'")
        .fetch_one(&inspector)
        .await
        .unwrap();
    assert_eq!(tables, 0);

    storage.migrate().await.unwrap();
    storage.migrate().await.unwrap();
    let status = storage.check_schema().await.unwrap();
    assert_eq!(status.current_version(), Some(2));
    assert!(status.is_current());

    assert!(matches!(
        SqliteStorage::connect(
            SqliteStorageOptions::new(":memory:").busy_timeout(StdDuration::ZERO),
        )
        .await,
        Err(SqliteStorageError::Configuration(_))
    ));
}

#[tokio::test]
async fn conversation_store_supports_directs_groups_and_owner_lifecycle() {
    let (_directory, storage) = store().await;
    let now = timestamp(100);
    let alice = persisted_user(&storage, "alice").await;
    let bob = persisted_user(&storage, "bob").await;
    let direct = StartDirectConversation::new(&storage, &storage)
        .execute(alice.id(), bob.id(), now)
        .await
        .unwrap();
    let direct_id = match direct {
        StartDirectStoreOutcome::Created(c) | StartDirectStoreOutcome::Existing(c) => c.id(),
    };
    let reverse = StartDirectConversation::new(&storage, &storage)
        .execute(bob.id(), alice.id(), now)
        .await
        .unwrap();
    assert_eq!(
        match reverse {
            StartDirectStoreOutcome::Created(c) | StartDirectStoreOutcome::Existing(c) => c.id(),
        },
        direct_id
    );
    let self_direct = StartDirectConversation::new(&storage, &storage)
        .execute(alice.id(), alice.id(), now)
        .await
        .unwrap();
    let self_id = match self_direct {
        StartDirectStoreOutcome::Created(c) | StartDirectStoreOutcome::Existing(c) => c.id(),
    };
    assert!(
        storage
            .find_member(self_id, alice.id())
            .await
            .unwrap()
            .is_some()
    );

    let group = CreateGroupConversation::new(&storage, &storage)
        .execute(alice.id(), "team", &[bob.id(), bob.id()], now)
        .await
        .unwrap();
    assert_eq!(
        storage
            .find_member(group.id(), alice.id())
            .await
            .unwrap()
            .unwrap()
            .role(),
        windwatcher::MembershipRole::Group(GroupRole::Owner)
    );
    AddGroupMember::new(&storage, &storage)
        .execute(alice.id(), group.id(), bob.id(), now)
        .await
        .unwrap();
    assert_eq!(
        SetGroupMemberRole::new(&storage)
            .execute(
                alice.id(),
                group.id(),
                bob.id(),
                GroupRole::Admin,
                timestamp(101)
            )
            .await
            .unwrap(),
        ChangeStoreOutcome::Changed
    );
    assert_eq!(
        TransferGroupOwnership::new(&storage)
            .execute(alice.id(), group.id(), bob.id(), timestamp(102))
            .await
            .unwrap(),
        ChangeStoreOutcome::Changed
    );
    assert_eq!(
        LeaveGroupConversation::new(&storage)
            .execute(alice.id(), group.id(), timestamp(103))
            .await
            .unwrap(),
        LeaveStoreOutcome::Left
    );
    assert_eq!(
        LeaveGroupConversation::new(&storage)
            .execute(bob.id(), group.id(), timestamp(104))
            .await
            .unwrap(),
        LeaveStoreOutcome::ConversationDeleted
    );
    assert!(
        storage
            .find_conversation(group.id())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn concurrent_direct_creation_converges_for_peer_and_self_pairs() {
    let (_directory, storage) = store().await;
    let storage = Arc::new(storage);
    let alice = persisted_user(&storage, "alice").await;
    let bob = persisted_user(&storage, "bob").await;
    for (first, second) in [(alice.id(), bob.id()), (alice.id(), alice.id())] {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let tasks = [(), ()].map(|_| {
            let store = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                StartDirectConversation::new(&*store, &*store)
                    .execute(first, second, timestamp(200))
                    .await
                    .unwrap()
            })
        });
        let [left, right] = tasks;
        let id = |outcome: StartDirectStoreOutcome| match outcome {
            StartDirectStoreOutcome::Created(conversation)
            | StartDirectStoreOutcome::Existing(conversation) => conversation.id(),
        };
        assert_eq!(id(left.await.unwrap()), id(right.await.unwrap()));
    }
}

#[tokio::test]
async fn identity_store_contract_preserves_atomicity_and_canonical_values() {
    let (_directory, storage) = store().await;
    support::identity_store_contract::run(&storage).await;

    let first = persisted_user(&storage, "bob").await;
    let loaded = storage.find_user(first.id()).await.unwrap().unwrap();
    assert_eq!(loaded.id(), first.id());
    assert_eq!(loaded.username(), first.username());
    assert_eq!(loaded.created_at(), timestamp(1_700_000_000));
    assert_eq!(loaded.updated_at(), timestamp(1_700_000_000));

    let alice = "BOB".parse().unwrap();
    let auth = storage
        .find_local_auth_by_username(&alice)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(auth.user(), &loaded);
    assert_eq!(auth.password_hash().as_encoded(), HASH);
    let missing = "missing".parse().unwrap();
    assert!(
        storage
            .find_local_auth_by_username(&missing)
            .await
            .unwrap()
            .is_none()
    );

    let duplicate = user("BOB", timestamp(2));
    assert!(matches!(
        storage
            .create_local_user(&duplicate, &password_hash())
            .await,
        Err(IdentityStoreError::UsernameConflict)
    ));
    assert!(storage.find_user(duplicate.id()).await.unwrap().is_none());

    let mut disabled = loaded.clone();
    disabled.disable(timestamp(20) + Duration::microseconds(7));
    storage.update_user(&disabled).await.unwrap();
    assert_eq!(storage.find_user(first.id()).await.unwrap(), Some(disabled));
    assert!(matches!(
        storage.update_user(&user("other", timestamp(0))).await,
        Err(IdentityStoreError::NotFound)
    ));
}

#[tokio::test]
async fn identity_creation_rolls_back_when_credential_insert_fails() {
    let (directory, storage) = store().await;
    let user = user("alice", timestamp(10));
    let url = format!(
        "sqlite://{}",
        directory.path().join("windwatcher.db").display()
    );
    let inspector = sqlx::SqlitePool::connect(&url).await.unwrap();
    inspector.execute("CREATE TRIGGER reject_credentials BEFORE INSERT ON local_credentials BEGIN SELECT RAISE(ABORT, 'test failure'); END").await.unwrap();
    assert!(matches!(
        storage.create_local_user(&user, &password_hash()).await,
        Err(IdentityStoreError::Unexpected(_))
    ));
    assert!(storage.find_user(user.id()).await.unwrap().is_none());
    inspector
        .execute("DROP TRIGGER reject_credentials")
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_username_registration_has_one_database_winner() {
    let (_directory, storage) = store().await;
    let storage = Arc::new(storage);
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let attempts = [UserId::new(), UserId::new()].map(|id| {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            let user = User::new(id, "alice".parse().unwrap(), timestamp(1));
            barrier.wait().await;
            storage.create_local_user(&user, &password_hash()).await
        })
    });
    let [first, second] = attempts;
    let first = first.await.unwrap();
    let second = second.await.unwrap();
    assert_eq!(
        [first.is_ok(), second.is_ok()]
            .into_iter()
            .filter(|success| *success)
            .count(),
        1
    );
    assert_eq!(
        [first, second]
            .into_iter()
            .filter(|result| matches!(result, Err(IdentityStoreError::UsernameConflict)))
            .count(),
        1
    );
}

#[tokio::test]
async fn sessions_use_atomic_refresh_cas_and_preserve_revocation() {
    let (_directory, storage) = store().await;
    let owner = persisted_user(&storage, "alice").await;
    let other_owner = persisted_user(&storage, "other_user").await;
    support::session_store_contract::run(&storage, owner.id(), other_owner.id()).await;
    let provider = OsRefreshTokenProvider;
    let first = RefreshToken::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into()).unwrap();
    let second = RefreshToken::parse("AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE".into()).unwrap();
    let old = provider.hash(&first);
    let replacement = provider.hash(&second);
    let created = timestamp(10) + Duration::nanoseconds(999);
    let session = Session::new(
        SessionId::new(),
        owner.id(),
        created,
        created + Duration::days(1),
    )
    .unwrap();
    storage.create_session(&session, &old).await.unwrap();
    let loaded = storage
        .find_by_refresh_token_hash(&old)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.created_at(), timestamp(10));
    assert_eq!(loaded.expires_at(), timestamp(10) + Duration::days(1));

    let storage = Arc::new(storage);
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let session_id = session.id();
    let results = [0, 1].map(|_| {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            storage
                .rotate_refresh_token(session_id, &old, &replacement, timestamp(11))
                .await
                .unwrap()
        })
    });
    let [left, right] = results;
    let left = left.await.unwrap();
    let right = right.await.unwrap();
    assert_eq!(
        [left, right]
            .into_iter()
            .filter(|result| *result == RefreshRotationResult::Rotated)
            .count(),
        1
    );
    assert_eq!(
        [left, right]
            .into_iter()
            .filter(|result| *result == RefreshRotationResult::Rejected)
            .count(),
        1
    );
    assert!(
        storage
            .find_by_refresh_token_hash(&old)
            .await
            .unwrap()
            .is_none()
    );

    assert!(
        storage
            .revoke_session_for_user(owner.id(), session.id(), timestamp(12))
            .await
            .unwrap()
    );
    assert!(
        storage
            .revoke_session_for_user(owner.id(), session.id(), timestamp(13))
            .await
            .unwrap()
    );
    assert_eq!(
        storage
            .find_by_refresh_token_hash(&replacement)
            .await
            .unwrap()
            .unwrap()
            .revoked_at(),
        Some(timestamp(12))
    );
    assert!(
        !storage
            .revoke_session_for_user(UserId::new(), session.id(), timestamp(14))
            .await
            .unwrap()
    );
    assert!(
        !storage
            .revoke_session_for_user(owner.id(), SessionId::new(), timestamp(14))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn storage_enforces_foreign_keys_and_physical_representations() {
    let (directory, storage) = store().await;
    let provider = OsRefreshTokenProvider;
    let token = provider.generate().unwrap();
    let orphan = Session::new(SessionId::new(), UserId::new(), timestamp(1), timestamp(2)).unwrap();
    assert!(matches!(
        storage
            .create_session(&orphan, &provider.hash(&token))
            .await,
        Err(windwatcher::SessionStoreError::Conflict)
    ));

    let user = persisted_user(&storage, "alice").await;
    let url = format!(
        "sqlite://{}",
        directory.path().join("windwatcher.db").display()
    );
    let inspector = sqlx::SqlitePool::connect(&url).await.unwrap();
    let row = sqlx::query("SELECT id, created_at FROM users WHERE id = ?")
        .bind(user.id().as_uuid().into_bytes().to_vec())
        .fetch_one(&inspector)
        .await
        .unwrap();
    let id: Vec<u8> = row.try_get("id").unwrap();
    let created_at: i64 = row.try_get("created_at").unwrap();
    assert_eq!(id.len(), 16);
    assert_eq!(created_at, 1_700_000_000_000_000);
}

#[tokio::test]
async fn strict_tables_reject_incorrect_storage_classes_and_lengths() {
    let (directory, storage) = store().await;
    let user = persisted_user(&storage, "alice").await;
    let url = format!(
        "sqlite://{}",
        directory.path().join("windwatcher.db").display()
    );
    let inspector = sqlx::SqlitePool::connect(&url).await.unwrap();

    assert!(sqlx::query("INSERT INTO users (id, username, created_at, updated_at) VALUES ('text-id', 'text_id', 0, 0)")
        .execute(&inspector).await.is_err());
    assert!(sqlx::query("INSERT INTO users (id, username, created_at, updated_at) VALUES (?, 'text_timestamp', 'not-an-integer', 0)")
        .bind(UserId::new().as_uuid().into_bytes().to_vec())
        .execute(&inspector).await.is_err());
    assert!(sqlx::query("INSERT INTO sessions (id, user_id, refresh_token_hash, created_at, expires_at) VALUES (?, ?, 'text-hash', 0, 1)")
        .bind(SessionId::new().as_uuid().into_bytes().to_vec())
        .bind(user.id().as_uuid().into_bytes().to_vec())
        .execute(&inspector).await.is_err());
    assert!(sqlx::query("INSERT INTO sessions (id, user_id, refresh_token_hash, created_at, expires_at) VALUES (?, ?, x'00', 0, 1)")
        .bind(SessionId::new().as_uuid().into_bytes().to_vec())
        .bind(user.id().as_uuid().into_bytes().to_vec())
        .execute(&inspector).await.is_err());

    let credential_user_id = UserId::new();
    inspector.execute(sqlx::query("INSERT INTO users (id, username, created_at, updated_at) VALUES (?, 'credential_user', 0, 0)")
        .bind(credential_user_id.as_uuid().into_bytes().to_vec())).await.unwrap();
    assert!(
        sqlx::query("INSERT INTO local_credentials (user_id, password_hash) VALUES (?, x'00')")
            .bind(credential_user_id.as_uuid().into_bytes().to_vec())
            .execute(&inspector)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn locked_database_is_normalized_as_unavailable() {
    let directory = TempDir::new().unwrap();
    let url = format!("sqlite://{}", directory.path().join("locked.db").display());
    let storage = SqliteStorage::connect(
        SqliteStorageOptions::new(url.clone())
            .max_connections(1)
            .busy_timeout(StdDuration::from_millis(1)),
    )
    .await
    .unwrap();
    storage.migrate().await.unwrap();
    let mut user = persisted_user(&storage, "alice").await;
    let mut lock = SqliteConnection::connect(&url).await.unwrap();
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&mut lock)
        .await
        .unwrap();
    user.disable(timestamp(10));
    let result = storage.update_user(&user).await;
    sqlx::query("ROLLBACK").execute(&mut lock).await.unwrap();
    assert!(matches!(result, Err(IdentityStoreError::Unavailable(_))));
}
