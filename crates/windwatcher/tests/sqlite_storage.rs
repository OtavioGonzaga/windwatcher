#![cfg(feature = "sqlite")]

use std::{sync::Arc, time::Duration as StdDuration};

use sqlx::{Connection, Executor, Row, SqliteConnection};
use tempfile::TempDir;
use time::{Duration, OffsetDateTime};
use windwatcher::{
    AddGroupMember, ConversationStore, ConversationStoreError, CreateGroupConversation,
    GetConversation, GroupRole, IdentityStore, IdentityStoreError, LeaveGroupConversation,
    LeaveGroupOutcome, MutationOutcome, OsRefreshTokenProvider, PasswordHash,
    RefreshRotationResult, RefreshToken, RefreshTokenProvider, RenameGroupConversation, Session,
    SessionId, SessionStore, SetGroupMemberRole, SqliteStorage, SqliteStorageError,
    SqliteStorageOptions, StartDirectConversation, StartDirectConversationOutcome,
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
async fn session_creation_rejects_owner_disabled_after_authentication() {
    let (_directory, storage) = store().await;
    let mut owner = persisted_user(&storage, "racing_owner").await;
    let now = timestamp(1_700_000_100);
    let session = Session::new(SessionId::new(), owner.id(), now, now + Duration::days(1)).unwrap();
    let provider = OsRefreshTokenProvider;
    let hash = provider.hash(&provider.generate().unwrap());
    owner.disable(now);
    storage.update_user(&owner).await.unwrap();
    assert!(matches!(
        storage.create_session(&session, &hash).await,
        Err(windwatcher::SessionStoreError::OwnerDisabled)
    ));
    assert!(
        storage
            .find_by_refresh_token_hash(&hash)
            .await
            .unwrap()
            .is_none()
    );
    let missing = Session::new(
        SessionId::new(),
        UserId::new(),
        now,
        now + Duration::days(1),
    )
    .unwrap();
    assert!(matches!(
        storage.create_session(&missing, &hash).await,
        Err(windwatcher::SessionStoreError::OwnerNotFound)
    ));
}

#[tokio::test]
async fn new_memberships_require_enabled_users_but_existing_state_survives() {
    let (_directory, storage) = store().await;
    let owner = persisted_user(&storage, "eligible_owner").await;
    let peer = persisted_user(&storage, "eligible_peer").await;
    let target = persisted_user(&storage, "disabled_target").await;
    support::conversation_store_contract::disabled_memberships(&storage, owner, peer, target).await;
}

#[tokio::test]
async fn direct_creation_observes_disable_committed_by_another_writer() {
    let (directory, storage) = store().await;
    let owner = persisted_user(&storage, "race_owner").await;
    let peer = persisted_user(&storage, "race_peer").await;
    let mut connection = SqliteConnection::connect(&format!(
        "sqlite://{}",
        directory.path().join("windwatcher.db").display()
    ))
    .await
    .unwrap();
    let mut tx = connection.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("UPDATE users SET disabled_at = 1 WHERE id = ?")
        .bind(peer.id().as_uuid().as_bytes().to_vec())
        .execute(&mut *tx)
        .await
        .unwrap();
    let candidate = windwatcher::Conversation::new_direct(
        windwatcher::ConversationId::new(),
        timestamp(1_700_000_100),
    );
    let other = storage.clone();
    let operation = tokio::spawn(async move {
        other
            .get_or_create_direct(&candidate, owner.id(), peer.id())
            .await
    });
    tx.commit().await.unwrap();
    assert!(matches!(
        operation.await.unwrap(),
        Err(ConversationStoreError::ParticipantUnavailable(_))
    ));
}

#[cfg(feature = "jwt-hs256")]
#[tokio::test]
async fn session_creation_keeps_atomic_rejection_after_owner_is_reenabled() {
    let (_directory, storage) = store().await;
    let authenticated = persisted_user(&storage, "reenabled_owner").await;
    let now = timestamp(1_700_000_100);
    let mut persisted = authenticated.clone();
    persisted.disable(now);
    storage.update_user(&persisted).await.unwrap();
    let codec = windwatcher::Hs256JwtAccessTokenCodec::new(
        windwatcher::Hs256SigningSecret::new(vec![7; 32]).unwrap(),
        "windwatcher".into(),
    );
    let error = windwatcher::CreateSession::new(
        &storage,
        &OsRefreshTokenProvider,
        &codec,
        windwatcher::SessionPolicy::default(),
    )
    .execute(&authenticated, now)
    .await
    .unwrap_err();
    persisted.enable(now + Duration::seconds(1));
    storage.update_user(&persisted).await.unwrap();
    assert!(
        !storage
            .find_user(authenticated.id())
            .await
            .unwrap()
            .unwrap()
            .is_disabled()
    );
    assert!(matches!(
        error,
        windwatcher::CreateSessionError::UserDisabled
    ));
}

#[tokio::test]
async fn sqlite_satisfies_portable_message_contract() {
    let (_directory, storage) = store().await;
    let owner = persisted_user(&storage, "message_owner").await;
    let admin = persisted_user(&storage, "message_admin").await;
    let member = persisted_user(&storage, "message_member").await;
    let outsider = persisted_user(&storage, "message_outsider").await;
    support::message_store_contract::run(
        &storage,
        owner.id(),
        admin.id(),
        member.id(),
        outsider.id(),
    )
    .await;
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
    assert_eq!(status.latest_version(), Some(3));
    assert_eq!(status.pending(), 3);
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
    assert_eq!(status.current_version(), Some(3));
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
    let direct = StartDirectConversation::new(&storage)
        .execute(alice.id(), bob.id(), now)
        .await
        .unwrap();
    let direct_id = match direct {
        StartDirectConversationOutcome::Created(c)
        | StartDirectConversationOutcome::Existing(c) => c.id(),
    };
    let reverse = StartDirectConversation::new(&storage)
        .execute(bob.id(), alice.id(), now)
        .await
        .unwrap();
    assert_eq!(
        match reverse {
            StartDirectConversationOutcome::Created(c)
            | StartDirectConversationOutcome::Existing(c) => c.id(),
        },
        direct_id
    );
    let self_direct = StartDirectConversation::new(&storage)
        .execute(alice.id(), alice.id(), now)
        .await
        .unwrap();
    let self_id = match self_direct {
        StartDirectConversationOutcome::Created(c)
        | StartDirectConversationOutcome::Existing(c) => c.id(),
    };
    assert!(
        storage
            .find_member(self_id, alice.id())
            .await
            .unwrap()
            .is_some()
    );

    let group = CreateGroupConversation::new(&storage)
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
    AddGroupMember::new(&storage)
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
        MutationOutcome::Changed
    );
    assert_eq!(
        TransferGroupOwnership::new(&storage)
            .execute(alice.id(), group.id(), bob.id(), timestamp(102))
            .await
            .unwrap(),
        MutationOutcome::Changed
    );
    assert_eq!(
        LeaveGroupConversation::new(&storage)
            .execute(alice.id(), group.id(), timestamp(103))
            .await
            .unwrap(),
        LeaveGroupOutcome::Left
    );
    assert_eq!(
        LeaveGroupConversation::new(&storage)
            .execute(bob.id(), group.id(), timestamp(104))
            .await
            .unwrap(),
        LeaveGroupOutcome::ConversationDeleted
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
async fn conversation_store_contract_preserves_lifecycle_semantics() {
    let (_directory, storage) = store().await;
    let owner = persisted_user(&storage, "contract_owner").await;
    let admin = persisted_user(&storage, "contract_admin").await;
    let member = persisted_user(&storage, "contract_member").await;
    let outsider = persisted_user(&storage, "contract_outsider").await;
    support::conversation_store_contract::run(
        &storage,
        owner.id(),
        admin.id(),
        member.id(),
        outsider.id(),
    )
    .await;
}

#[tokio::test]
async fn concurrent_direct_creation_converges_for_peer_and_self_pairs() {
    let (directory, storage) = store().await;
    let storage = Arc::new(storage);
    let alice = persisted_user(&storage, "alice").await;
    let bob = persisted_user(&storage, "bob").await;
    let mut direct_ids = Vec::new();
    for [(first, second), (other_first, other_second)] in [
        [(alice.id(), bob.id()), (bob.id(), alice.id())],
        [(alice.id(), alice.id()), (alice.id(), alice.id())],
    ] {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let tasks = [(first, second), (other_first, other_second)].map(|(first, second)| {
            let store = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                StartDirectConversation::new(&*store)
                    .execute(first, second, timestamp(200))
                    .await
                    .unwrap()
            })
        });
        let [left, right] = tasks;
        let id = |outcome: StartDirectConversationOutcome| match outcome {
            StartDirectConversationOutcome::Created(conversation)
            | StartDirectConversationOutcome::Existing(conversation) => conversation.id(),
        };
        let conversation_id = id(left.await.unwrap());
        assert_eq!(conversation_id, id(right.await.unwrap()));
        direct_ids.push(conversation_id);
    }
    let url = format!(
        "sqlite://{}",
        directory.path().join("windwatcher.db").display()
    );
    let inspector = sqlx::SqlitePool::connect(&url).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE kind = 'direct'")
        .fetch_one(&inspector)
        .await
        .unwrap();
    assert_eq!(count, 2);
    for (id, member_count) in [(direct_ids[0], 2_i64), (direct_ids[1], 1_i64)] {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM conversation_members WHERE conversation_id = ?",
        )
        .bind(id.as_uuid().into_bytes().to_vec())
        .fetch_one(&inspector)
        .await
        .unwrap();
        assert_eq!(count, member_count);
    }
}

#[tokio::test]
async fn group_permissions_no_ops_and_direct_rejections_are_enforced() {
    let (_directory, storage) = store().await;
    let now = timestamp(300);
    let owner = persisted_user(&storage, "owner").await;
    let admin = persisted_user(&storage, "admin").await;
    let member = persisted_user(&storage, "member").await;
    let outsider = persisted_user(&storage, "outsider").await;
    let group = CreateGroupConversation::new(&storage)
        .execute(owner.id(), "Group", &[admin.id(), member.id()], now)
        .await
        .unwrap();
    assert_eq!(
        SetGroupMemberRole::new(&storage)
            .execute(
                owner.id(),
                group.id(),
                admin.id(),
                GroupRole::Admin,
                timestamp(301)
            )
            .await
            .unwrap(),
        MutationOutcome::Changed
    );
    assert!(matches!(
        AddGroupMember::new(&storage)
            .execute(member.id(), group.id(), outsider.id(), timestamp(302))
            .await,
        Err(windwatcher::AddGroupMemberError::InsufficientPermissions)
    ));
    assert_eq!(
        AddGroupMember::new(&storage)
            .execute(admin.id(), group.id(), outsider.id(), timestamp(302))
            .await
            .unwrap(),
        windwatcher::AddGroupMemberOutcome::Added
    );
    assert!(matches!(
        GetConversation::new(&storage)
            .execute(UserId::new(), group.id())
            .await,
        Err(windwatcher::GetConversationError::NotMember)
    ));
    assert!(matches!(
        AddGroupMember::new(&storage)
            .execute(owner.id(), group.id(), UserId::new(), timestamp(302))
            .await,
        Err(windwatcher::AddGroupMemberError::ParticipantUnavailable(_))
    ));
    assert!(matches!(
        TransferGroupOwnership::new(&storage)
            .execute(admin.id(), group.id(), owner.id(), timestamp(303))
            .await,
        Err(windwatcher::TransferGroupOwnershipError::InsufficientPermissions)
    ));
    assert!(matches!(
        SetGroupMemberRole::new(&storage)
            .execute(
                admin.id(),
                group.id(),
                owner.id(),
                GroupRole::Member,
                timestamp(303)
            )
            .await,
        Err(windwatcher::SetGroupMemberRoleError::OwnerProtected)
    ));
    let before = storage
        .find_conversation(group.id())
        .await
        .unwrap()
        .unwrap()
        .updated_at();
    assert_eq!(
        RenameGroupConversation::new(&storage)
            .execute(admin.id(), group.id(), "Group", timestamp(304))
            .await
            .unwrap(),
        MutationOutcome::Unchanged
    );
    assert_eq!(
        storage
            .find_conversation(group.id())
            .await
            .unwrap()
            .unwrap()
            .updated_at(),
        before
    );
    assert!(matches!(
        RenameGroupConversation::new(&storage)
            .execute(member.id(), group.id(), "new", timestamp(304))
            .await,
        Err(windwatcher::RenameGroupConversationError::InsufficientPermissions)
    ));
    assert!(matches!(
        RenameGroupConversation::new(&storage)
            .execute(owner.id(), group.id(), "  ", timestamp(304))
            .await,
        Err(windwatcher::RenameGroupConversationError::InvalidTitle(_))
    ));
    let direct = StartDirectConversation::new(&storage)
        .execute(owner.id(), member.id(), now)
        .await
        .unwrap();
    let direct_id = match direct {
        StartDirectConversationOutcome::Created(c)
        | StartDirectConversationOutcome::Existing(c) => c.id(),
    };
    assert!(matches!(
        RenameGroupConversation::new(&storage)
            .execute(owner.id(), direct_id, "x", now)
            .await,
        Err(windwatcher::RenameGroupConversationError::DirectOperationNotSupported)
    ));
    assert!(matches!(
        LeaveGroupConversation::new(&storage)
            .execute(owner.id(), direct_id, now)
            .await,
        Err(windwatcher::LeaveGroupConversationError::DirectOperationNotSupported)
    ));
}

#[tokio::test]
async fn group_creation_and_ownership_transfer_roll_back_on_persistence_failure() {
    let (directory, storage) = store().await;
    let owner = persisted_user(&storage, "owner").await;
    let member = persisted_user(&storage, "member").await;
    let url = format!(
        "sqlite://{}",
        directory.path().join("windwatcher.db").display()
    );
    let inspector = sqlx::SqlitePool::connect(&url).await.unwrap();
    inspector.execute("CREATE TRIGGER reject_member_insert BEFORE INSERT ON conversation_members WHEN NEW.role = 'member' BEGIN SELECT RAISE(ABORT, 'test failure'); END").await.unwrap();
    assert!(matches!(
        CreateGroupConversation::new(&storage)
            .execute(owner.id(), "rollback", &[member.id()], timestamp(400))
            .await,
        Err(windwatcher::CreateGroupConversationError::Storage(_))
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE title = 'rollback'")
            .fetch_one(&inspector)
            .await
            .unwrap();
    assert_eq!(count, 0);
    inspector
        .execute("DROP TRIGGER reject_member_insert")
        .await
        .unwrap();
    let group = CreateGroupConversation::new(&storage)
        .execute(owner.id(), "transfer", &[member.id()], timestamp(401))
        .await
        .unwrap();
    inspector.execute("CREATE TRIGGER reject_owner_transfer BEFORE UPDATE OF role ON conversation_members WHEN NEW.role = 'owner' BEGIN SELECT RAISE(ABORT, 'test failure'); END").await.unwrap();
    assert!(matches!(
        TransferGroupOwnership::new(&storage)
            .execute(owner.id(), group.id(), member.id(), timestamp(402))
            .await,
        Err(windwatcher::TransferGroupOwnershipError::Storage(_))
    ));
    assert_eq!(
        storage
            .find_member(group.id(), owner.id())
            .await
            .unwrap()
            .unwrap()
            .role(),
        windwatcher::MembershipRole::Group(GroupRole::Owner)
    );
    assert_eq!(
        storage
            .find_member(group.id(), member.id())
            .await
            .unwrap()
            .unwrap()
            .role(),
        windwatcher::MembershipRole::Group(GroupRole::Member)
    );
}

#[tokio::test]
async fn group_management_permission_matrix_and_owner_only_lifecycle_hold() {
    let (_directory, storage) = store().await;
    let owner = persisted_user(&storage, "matrix_owner").await;
    let admin = persisted_user(&storage, "matrix_admin").await;
    let member = persisted_user(&storage, "matrix_member").await;
    let target = persisted_user(&storage, "matrix_target").await;
    let group = CreateGroupConversation::new(&storage)
        .execute(
            owner.id(),
            "matrix",
            &[admin.id(), member.id(), target.id()],
            timestamp(600),
        )
        .await
        .unwrap();
    assert_eq!(
        SetGroupMemberRole::new(&storage)
            .execute(
                owner.id(),
                group.id(),
                admin.id(),
                GroupRole::Admin,
                timestamp(601)
            )
            .await
            .unwrap(),
        MutationOutcome::Changed
    );
    assert_eq!(
        SetGroupMemberRole::new(&storage)
            .execute(
                admin.id(),
                group.id(),
                target.id(),
                GroupRole::Admin,
                timestamp(602)
            )
            .await
            .unwrap(),
        MutationOutcome::Changed
    );
    let before = storage
        .find_conversation(group.id())
        .await
        .unwrap()
        .unwrap()
        .updated_at();
    assert_eq!(
        SetGroupMemberRole::new(&storage)
            .execute(
                admin.id(),
                group.id(),
                target.id(),
                GroupRole::Admin,
                timestamp(603)
            )
            .await
            .unwrap(),
        MutationOutcome::Unchanged
    );
    assert_eq!(
        storage
            .find_conversation(group.id())
            .await
            .unwrap()
            .unwrap()
            .updated_at(),
        before
    );
    assert_eq!(
        windwatcher::RemoveGroupMember::new(&storage)
            .execute(admin.id(), group.id(), target.id(), timestamp(604))
            .await
            .unwrap(),
        MutationOutcome::Changed
    );
    assert!(matches!(
        windwatcher::RemoveGroupMember::new(&storage)
            .execute(member.id(), group.id(), admin.id(), timestamp(605))
            .await,
        Err(windwatcher::RemoveGroupMemberError::InsufficientPermissions)
    ));
    assert_eq!(
        windwatcher::RemoveGroupMember::new(&storage)
            .execute(owner.id(), group.id(), admin.id(), timestamp(605))
            .await
            .unwrap(),
        MutationOutcome::Changed
    );
    assert!(matches!(
        LeaveGroupConversation::new(&storage)
            .execute(owner.id(), group.id(), timestamp(606))
            .await,
        Err(windwatcher::LeaveGroupConversationError::OwnershipTransferRequired)
    ));
    assert_eq!(
        LeaveGroupConversation::new(&storage)
            .execute(member.id(), group.id(), timestamp(607))
            .await
            .unwrap(),
        LeaveGroupOutcome::Left
    );

    let owner_only = CreateGroupConversation::new(&storage)
        .execute(owner.id(), "owner only", &[], timestamp(608))
        .await
        .unwrap();
    assert_eq!(
        LeaveGroupConversation::new(&storage)
            .execute(owner.id(), owner_only.id(), timestamp(609))
            .await
            .unwrap(),
        LeaveGroupOutcome::ConversationDeleted
    );
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
    let mut disabled_owner = persisted_user(&storage, "disabled_owner").await;
    disabled_owner.disable(timestamp(1_700_000_001));
    storage.update_user(&disabled_owner).await.unwrap();
    support::session_store_contract::run(
        &storage,
        owner.id(),
        other_owner.id(),
        disabled_owner.id(),
        UserId::new(),
    )
    .await;
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
        Err(windwatcher::SessionStoreError::OwnerNotFound)
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
async fn conversation_schema_uses_strict_binary_keys_and_cascading_memberships() {
    let (directory, storage) = store().await;
    let alice = persisted_user(&storage, "schema_alice").await;
    let bob = persisted_user(&storage, "schema_bob").await;
    let direct = StartDirectConversation::new(&storage)
        .execute(alice.id(), bob.id(), timestamp(500))
        .await
        .unwrap();
    let direct_id = match direct {
        StartDirectConversationOutcome::Created(c)
        | StartDirectConversationOutcome::Existing(c) => c.id(),
    };
    let url = format!(
        "sqlite://{}",
        directory.path().join("windwatcher.db").display()
    );
    let inspector = sqlx::SqlitePool::connect(&url).await.unwrap();
    for table in ["conversations", "conversation_members"] {
        let sql: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?")
                .bind(table)
                .fetch_one(&inspector)
                .await
                .unwrap();
        assert!(sql.ends_with(" STRICT"));
    }
    let owner_index: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'conversation_members_one_owner'",
    )
    .fetch_one(&inspector)
    .await
    .unwrap();
    assert_eq!(owner_index, 1);
    let row = sqlx::query("SELECT id, direct_key, created_at FROM conversations WHERE id = ?")
        .bind(direct_id.as_uuid().into_bytes().to_vec())
        .fetch_one(&inspector)
        .await
        .unwrap();
    assert_eq!(row.try_get::<Vec<u8>, _>("id").unwrap().len(), 16);
    assert_eq!(row.try_get::<Vec<u8>, _>("direct_key").unwrap().len(), 32);
    assert_eq!(row.try_get::<i64, _>("created_at").unwrap(), 500_000_000);
    assert!(sqlx::query("INSERT INTO conversations (id, kind, title, direct_key, created_at, updated_at) VALUES (?, 'direct', 'invalid', ?, 0, 0)")
        .bind(windwatcher::ConversationId::new().as_uuid().into_bytes().to_vec())
        .bind(vec![0_u8; 32]).execute(&inspector).await.is_err());
    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(direct_id.as_uuid().into_bytes().to_vec())
        .execute(&inspector)
        .await
        .unwrap();
    let members: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversation_members WHERE conversation_id = ?")
            .bind(direct_id.as_uuid().into_bytes().to_vec())
            .fetch_one(&inspector)
            .await
            .unwrap();
    assert_eq!(members, 0);
}

#[tokio::test]
async fn malformed_persisted_conversation_memberships_are_rejected() {
    let (directory, storage) = store().await;
    let alice = persisted_user(&storage, "corrupt_alice").await;
    let bob = persisted_user(&storage, "corrupt_bob").await;
    let direct = StartDirectConversation::new(&storage)
        .execute(alice.id(), bob.id(), timestamp(550))
        .await
        .unwrap();
    let direct_id = match direct {
        StartDirectConversationOutcome::Created(c)
        | StartDirectConversationOutcome::Existing(c) => c.id(),
    };
    let url = format!(
        "sqlite://{}",
        directory.path().join("windwatcher.db").display()
    );
    let inspector = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query(
        "UPDATE conversation_members SET role = 'owner' WHERE conversation_id = ? AND user_id = ?",
    )
    .bind(direct_id.as_uuid().into_bytes().to_vec())
    .bind(alice.id().as_uuid().into_bytes().to_vec())
    .execute(&inspector)
    .await
    .unwrap();
    assert!(matches!(
        storage.find_member(direct_id, alice.id()).await,
        Err(ConversationStoreError::Unexpected(_))
    ));

    let group = CreateGroupConversation::new(&storage)
        .execute(alice.id(), "corrupt group", &[bob.id()], timestamp(551))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE conversation_members SET role = 'direct' WHERE conversation_id = ? AND user_id = ?",
    )
    .bind(group.id().as_uuid().into_bytes().to_vec())
    .bind(bob.id().as_uuid().into_bytes().to_vec())
    .execute(&inspector)
    .await
    .unwrap();
    assert!(matches!(
        storage.find_member(group.id(), bob.id()).await,
        Err(ConversationStoreError::Unexpected(_))
    ));
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
