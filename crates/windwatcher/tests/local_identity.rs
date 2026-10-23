use futures_executor::block_on;
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use time::OffsetDateTime;
use windwatcher::*;

// Test-only PHC fixture. The fake never claims to provide cryptographic security.
const ENCODED: &str = "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA";
fn hash() -> PasswordHash {
    PasswordHash::parse(ENCODED).unwrap()
}
fn password(value: &str) -> SecretPassword {
    SecretPassword::new(value.to_owned()).unwrap()
}
fn now(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}
fn unavailable() -> IdentityStoreError {
    IdentityStoreError::Unavailable(Box::new(std::io::Error::other("test storage failure")))
}

#[derive(Default)]
struct FakeHasher {
    hashes: AtomicUsize,
    verifies: AtomicUsize,
    dummies: AtomicUsize,
    fail: AtomicBool,
}
impl PasswordHasher for FakeHasher {
    async fn hash(&self, _: &SecretPassword) -> Result<PasswordHash, PasswordHashError> {
        self.hashes.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(PasswordHashError::HashingFailed(Box::new(
                std::io::Error::other("test hash failure"),
            )));
        }
        Ok(hash())
    }
    async fn verify(
        &self,
        password: &SecretPassword,
        expected: Option<&PasswordHash>,
    ) -> Result<bool, PasswordHashError> {
        self.verifies.fetch_add(1, Ordering::SeqCst);
        if expected.is_none() {
            self.dummies.fetch_add(1, Ordering::SeqCst);
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(PasswordHashError::VerificationFailed(Box::new(
                std::io::Error::other("test verify failure"),
            )));
        }
        Ok(expected.is_some() && matches!(password.expose_secret(), "password" | "short"))
    }
}

#[derive(Default)]
struct FakeStore {
    records: Mutex<HashMap<UserId, LocalAuthentication>>,
    creates: AtomicUsize,
    lookups: AtomicUsize,
    writes: AtomicUsize,
    fail: AtomicBool,
    fail_update: AtomicBool,
    disappear: AtomicBool,
}
impl IdentityStore for FakeStore {
    async fn create_local_user(
        &self,
        user: &User,
        hash: &PasswordHash,
    ) -> Result<(), IdentityStoreError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        let mut records = self.records.lock().unwrap();
        if self.fail.load(Ordering::SeqCst) {
            return Err(unavailable());
        }
        if records
            .values()
            .any(|r| r.user().username() == user.username())
        {
            return Err(IdentityStoreError::UsernameConflict);
        }
        records.insert(
            user.id(),
            LocalAuthentication::new(user.clone(), hash.clone()),
        );
        Ok(())
    }
    async fn find_local_auth_by_username(
        &self,
        username: &Username,
    ) -> Result<Option<LocalAuthentication>, IdentityStoreError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(unavailable());
        }
        Ok(self
            .records
            .lock()
            .unwrap()
            .values()
            .find(|r| r.user().username() == username)
            .cloned())
    }
    async fn find_user(&self, id: UserId) -> Result<Option<User>, IdentityStoreError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(unavailable());
        }
        Ok(self
            .records
            .lock()
            .unwrap()
            .get(&id)
            .map(|r| r.user().clone()))
    }
    async fn update_user(&self, user: &User) -> Result<(), IdentityStoreError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if self.fail_update.load(Ordering::SeqCst) {
            return Err(unavailable());
        }
        if self.disappear.load(Ordering::SeqCst) {
            return Err(IdentityStoreError::NotFound);
        }
        let mut records = self.records.lock().unwrap();
        let record = records
            .get_mut(&user.id())
            .ok_or(IdentityStoreError::NotFound)?;
        assert_eq!(record.user().username(), user.username());
        assert_eq!(record.user().created_at(), user.created_at());
        *record = LocalAuthentication::new(user.clone(), record.password_hash().clone());
        Ok(())
    }
}
impl SessionStore for FakeStore {
    async fn create_session(
        &self,
        _: &Session,
        _: &RefreshTokenHash,
    ) -> Result<(), SessionStoreError> {
        Ok(())
    }
    async fn find_by_refresh_token_hash(
        &self,
        _: &RefreshTokenHash,
    ) -> Result<Option<Session>, SessionStoreError> {
        Ok(None)
    }
    async fn rotate_refresh_token(
        &self,
        _: SessionId,
        _: &RefreshTokenHash,
        _: &RefreshTokenHash,
        _: OffsetDateTime,
    ) -> Result<RefreshRotationResult, SessionStoreError> {
        Ok(RefreshRotationResult::Rejected)
    }
    async fn revoke_session_for_user(
        &self,
        _: UserId,
        _: SessionId,
        _: OffsetDateTime,
    ) -> Result<bool, SessionStoreError> {
        Ok(false)
    }
    async fn revoke_all_for_user(
        &self,
        _: UserId,
        _: OffsetDateTime,
    ) -> Result<(), SessionStoreError> {
        Ok(())
    }
}
fn count(value: &AtomicUsize) -> usize {
    value.load(Ordering::SeqCst)
}
async fn register(store: &FakeStore, hasher: &FakeHasher) -> User {
    RegisterLocalUser::new(store, hasher, RegistrationPolicy::Open)
        .execute("Alice", &password("password"), now(10))
        .await
        .unwrap()
}

#[test]
fn open_registration_canonicalizes_and_atomically_persists_identity_and_credential() {
    block_on(async {
        let store = FakeStore::default();
        let hasher = FakeHasher::default();
        let user = register(&store, &hasher).await;
        assert_eq!(user.username().as_str(), "alice");
        assert_eq!(user.created_at(), now(10));
        assert!(!user.is_disabled());
        assert_eq!(count(&hasher.hashes), 1);
        assert_eq!(count(&store.creates), 1);
        assert_eq!(count(&store.lookups), 0); // No uniqueness preflight.
        assert_eq!(
            store.find_user(user.id()).await.unwrap(),
            Some(user.clone())
        );
        let auth = store
            .find_local_auth_by_username(user.username())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(auth.user(), &user);
        assert_eq!(auth.password_hash().as_encoded(), ENCODED);
        assert!(!format!("{auth:?}").contains(ENCODED));
        let err = RegisterLocalUser::new(&store, &hasher, RegistrationPolicy::Open)
            .execute("ALICE", &password("password"), now(20))
            .await
            .unwrap_err();
        assert!(matches!(err, RegisterLocalUserError::UsernameTaken));
        assert_eq!(store.records.lock().unwrap().len(), 1);
        assert_eq!(store.find_user(user.id()).await.unwrap(), Some(user));
    });
}

#[test]
fn registration_rejections_do_not_hash_or_write() {
    block_on(async {
        let store = FakeStore::default();
        let hasher = FakeHasher::default();
        let closed = RegisterLocalUser::new(&store, &hasher, RegistrationPolicy::Closed);
        assert!(matches!(
            closed.execute("!", &password("x"), now(0)).await,
            Err(RegisterLocalUserError::RegistrationClosed)
        ));
        let open = RegisterLocalUser::new(&store, &hasher, RegistrationPolicy::Open);
        assert!(matches!(
            open.execute("!", &password("password"), now(0)).await,
            Err(RegisterLocalUserError::InvalidUsername(_))
        ));
        assert!(matches!(
            open.execute("alice", &password("1234567"), now(0)).await,
            Err(RegisterLocalUserError::InvalidPassword(
                PasswordPolicyError::TooShort
            ))
        ));
        assert_eq!(count(&hasher.hashes), 0);
        assert_eq!(count(&store.creates), 0);
        assert_eq!(count(&store.lookups), 0);
    });
}

#[test]
fn trusted_creation_is_independent_from_self_registration_policy() {
    block_on(async {
        let store = FakeStore::default();
        let hasher = FakeHasher::default();
        assert!(matches!(
            RegisterLocalUser::new(&store, &hasher, RegistrationPolicy::Closed)
                .execute("alice", &password("password"), now(0))
                .await,
            Err(RegisterLocalUserError::RegistrationClosed)
        ));
        let user = CreateLocalUser::new(&store, &hasher)
            .execute("alice", &password("password"), now(1))
            .await
            .unwrap();
        assert_eq!(user.username().as_str(), "alice");
        assert_eq!(count(&hasher.hashes), 1);
        assert_eq!(count(&store.creates), 1);
        assert!(matches!(
            CreateLocalUser::new(&store, &hasher)
                .execute("ALICE", &password("password"), now(2))
                .await,
            Err(CreateLocalUserError::UsernameTaken)
        ));
    });
}

#[test]
fn registration_operational_failures_never_leave_partial_users() {
    block_on(async {
        let store = FakeStore::default();
        let hasher = FakeHasher::default();
        let service = RegisterLocalUser::new(&store, &hasher, RegistrationPolicy::Open);
        hasher.fail.store(true, Ordering::SeqCst);
        assert!(matches!(
            service
                .execute("alice", &password("password"), now(0))
                .await,
            Err(RegisterLocalUserError::PasswordHashing(_))
        ));
        assert_eq!(count(&store.creates), 0);
        hasher.fail.store(false, Ordering::SeqCst);
        store.fail.store(true, Ordering::SeqCst);
        assert!(matches!(
            service
                .execute("alice", &password("password"), now(0))
                .await,
            Err(RegisterLocalUserError::Storage(
                IdentityStoreError::Unavailable(_)
            ))
        ));
        assert!(store.records.lock().unwrap().is_empty());
    });
}

#[test]
fn authentication_hides_missing_invalid_and_wrong_credentials() {
    block_on(async {
        let store = FakeStore::default();
        let hasher = FakeHasher::default();
        let user = register(&store, &hasher).await;
        let service = AuthenticateLocalUser::new(&store, &hasher);
        assert_eq!(
            service
                .execute("ALICE", &password("password"))
                .await
                .unwrap(),
            user
        );
        for name in ["alice", "missing", "!"] {
            assert!(matches!(
                service.execute(name, &password("wrong")).await,
                Err(AuthenticateLocalUserError::InvalidCredentials)
            ));
        }
        assert_eq!(count(&hasher.verifies), 4);
        assert_eq!(count(&hasher.dummies), 2);
        assert_eq!(count(&store.lookups), 3); // Invalid syntax skipped storage, not verification.
        // Authentication does not apply the minimum length policy to old credentials.
        assert_eq!(
            service.execute("alice", &password("short")).await.unwrap(),
            user
        );
    });
}

#[test]
fn disabled_state_is_disclosed_only_after_password_verification() {
    block_on(async {
        let store = FakeStore::default();
        let hasher = FakeHasher::default();
        let user = register(&store, &hasher).await;
        DisableUser::new(&store, &store)
            .execute(user.id(), now(20))
            .await
            .unwrap();
        let service = AuthenticateLocalUser::new(&store, &hasher);
        assert!(matches!(
            service.execute("alice", &password("wrong")).await,
            Err(AuthenticateLocalUserError::InvalidCredentials)
        ));
        assert!(matches!(
            service.execute("alice", &password("password")).await,
            Err(AuthenticateLocalUserError::UserDisabled)
        ));
        assert_eq!(count(&hasher.verifies), 2);
        assert_eq!(count(&hasher.dummies), 0);
    });
}

#[test]
fn authentication_operational_failures_have_generic_semantics_and_diagnostic_sources() {
    use std::error::Error;
    block_on(async {
        let store = FakeStore::default();
        let hasher = FakeHasher::default();
        register(&store, &hasher).await;
        let service = AuthenticateLocalUser::new(&store, &hasher);
        hasher.fail.store(true, Ordering::SeqCst);
        for name in ["alice", "missing", "!"] {
            let err = service
                .execute(name, &password("password"))
                .await
                .unwrap_err();
            assert!(matches!(err, AuthenticateLocalUserError::Internal(_)));
            assert_eq!(err.to_string(), "authentication failed internally");
            assert!(
                err.source()
                    .unwrap()
                    .downcast_ref::<PasswordHashError>()
                    .is_some()
            );
        }
        hasher.fail.store(false, Ordering::SeqCst);
        store.fail.store(true, Ordering::SeqCst);
        let err = service
            .execute("alice", &password("password"))
            .await
            .unwrap_err();
        assert!(matches!(err, AuthenticateLocalUserError::Internal(_)));
        assert_eq!(err.to_string(), "authentication failed internally");
        assert!(
            err.source()
                .unwrap()
                .downcast_ref::<IdentityStoreError>()
                .is_some()
        );
    });
}

#[test]
fn persisted_state_transitions_preserve_identity_and_no_ops_skip_writes() {
    block_on(async {
        let store = FakeStore::default();
        let hasher = FakeHasher::default();
        let user = register(&store, &hasher).await;
        let enable = EnableUser::new(&store);
        let disable = DisableUser::new(&store, &store);
        assert_eq!(enable.execute(user.id(), now(15)).await.unwrap(), user);
        assert_eq!(count(&store.writes), 0);
        let disabled = disable.execute(user.id(), now(20)).await.unwrap();
        assert_eq!(disabled.disabled_at(), Some(now(20)));
        assert_eq!(disabled.updated_at(), now(20));
        assert_eq!(disable.execute(user.id(), now(25)).await.unwrap(), disabled);
        assert_eq!(count(&store.writes), 1);
        let enabled = enable.execute(user.id(), now(30)).await.unwrap();
        assert!(!enabled.is_disabled());
        assert_eq!(enabled.updated_at(), now(30));
        assert_eq!(enabled.created_at(), user.created_at());
        assert_eq!(enabled.username(), user.username());
        assert_eq!(enable.execute(user.id(), now(35)).await.unwrap(), enabled);
        assert_eq!(count(&store.writes), 2);
        assert_eq!(store.find_user(user.id()).await.unwrap(), Some(enabled));
        assert_eq!(
            store
                .find_local_auth_by_username(user.username())
                .await
                .unwrap()
                .unwrap()
                .password_hash()
                .as_encoded(),
            ENCODED
        );
    });
}

#[test]
fn management_reports_missing_users_and_storage_failures() {
    block_on(async {
        let store = FakeStore::default();
        let hasher = FakeHasher::default();
        let enable = EnableUser::new(&store);
        let disable = DisableUser::new(&store, &store);
        assert!(matches!(
            enable.execute(UserId::new(), now(0)).await,
            Err(ManageUserError::UserNotFound)
        ));
        assert!(matches!(
            disable.execute(UserId::new(), now(0)).await,
            Err(ManageUserError::UserNotFound)
        ));
        let user = register(&store, &hasher).await;
        store.fail.store(true, Ordering::SeqCst);
        assert!(matches!(
            enable.execute(user.id(), now(20)).await,
            Err(ManageUserError::Storage(_))
        ));
        assert!(matches!(
            disable.execute(user.id(), now(20)).await,
            Err(ManageUserError::Storage(_))
        ));
        store.fail.store(false, Ordering::SeqCst);
        store.fail_update.store(true, Ordering::SeqCst);
        assert!(matches!(
            disable.execute(user.id(), now(20)).await,
            Err(ManageUserError::Storage(_))
        ));
        assert_eq!(
            store.find_user(user.id()).await.unwrap(),
            Some(user.clone())
        );
        store.fail_update.store(false, Ordering::SeqCst);
        disable.execute(user.id(), now(20)).await.unwrap();
        store.fail_update.store(true, Ordering::SeqCst);
        assert!(matches!(
            enable.execute(user.id(), now(30)).await,
            Err(ManageUserError::Storage(_))
        ));
        store.fail_update.store(false, Ordering::SeqCst);
        store.disappear.store(true, Ordering::SeqCst);
        assert!(matches!(
            enable.execute(user.id(), now(30)).await,
            Err(ManageUserError::UserNotFound)
        ));
        store.disappear.store(false, Ordering::SeqCst);
        enable.execute(user.id(), now(30)).await.unwrap();
        store.disappear.store(true, Ordering::SeqCst);
        assert!(matches!(
            disable.execute(user.id(), now(40)).await,
            Err(ManageUserError::UserNotFound)
        ));
    });
}

#[test]
fn fake_store_atomic_create_is_unique_under_concurrent_registration() {
    let store = FakeStore::default();
    let hasher = FakeHasher::default();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let attempt = || {
            barrier.wait();
            block_on(
                RegisterLocalUser::new(&store, &hasher, RegistrationPolicy::Open).execute(
                    "alice",
                    &password("password"),
                    now(10),
                ),
            )
        };
        let a = scope.spawn(attempt);
        let b = scope.spawn(attempt);
        let results = [a.join().unwrap(), b.join().unwrap()];
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(RegisterLocalUserError::UsernameTaken)))
                .count(),
            1
        );
    });
    assert_eq!(store.records.lock().unwrap().len(), 1);
}
