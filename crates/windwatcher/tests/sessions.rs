use std::{
    collections::HashMap,
    sync::{
        Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_executor::block_on;
use time::{Duration, OffsetDateTime};
use windwatcher::*;

fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
}
fn user() -> User {
    User::new(UserId::new(), "alice".parse().unwrap(), now())
}

#[derive(Default)]
struct Identities(Mutex<HashMap<UserId, User>>);
impl Identities {
    fn insert(&self, user: User) {
        self.0.lock().unwrap().insert(user.id(), user);
    }
}
impl IdentityStore for Identities {
    async fn create_local_user(
        &self,
        user: &User,
        _: &PasswordHash,
    ) -> Result<(), IdentityStoreError> {
        self.insert(user.clone());
        Ok(())
    }
    async fn find_local_auth_by_username(
        &self,
        _: &Username,
    ) -> Result<Option<LocalAuthentication>, IdentityStoreError> {
        Ok(None)
    }
    async fn find_user(&self, id: UserId) -> Result<Option<User>, IdentityStoreError> {
        Ok(self.0.lock().unwrap().get(&id).cloned())
    }
    async fn update_user(&self, user: &User) -> Result<(), IdentityStoreError> {
        self.0.lock().unwrap().insert(user.id(), user.clone());
        Ok(())
    }
}

#[derive(Default)]
struct Sessions {
    records: Mutex<HashMap<SessionId, (Session, RefreshTokenHash)>>,
    revoke_all_calls: AtomicUsize,
    fail_revoke_all: AtomicUsize,
    create_failure: Mutex<Option<SessionStoreError>>,
}
impl SessionStore for Sessions {
    async fn create_session(
        &self,
        session: &Session,
        hash: &RefreshTokenHash,
    ) -> Result<(), SessionStoreError> {
        if let Some(error) = self.create_failure.lock().unwrap().take() {
            return Err(error);
        }
        self.records
            .lock()
            .unwrap()
            .insert(session.id(), (session.clone(), *hash));
        Ok(())
    }
    async fn find_by_refresh_token_hash(
        &self,
        hash: &RefreshTokenHash,
    ) -> Result<Option<Session>, SessionStoreError> {
        Ok(self
            .records
            .lock()
            .unwrap()
            .values()
            .find(|(_, current)| current == hash)
            .map(|(session, _)| session.clone()))
    }
    async fn rotate_refresh_token(
        &self,
        id: SessionId,
        old: &RefreshTokenHash,
        replacement: &RefreshTokenHash,
        at: OffsetDateTime,
    ) -> Result<RefreshRotationResult, SessionStoreError> {
        let mut sessions = self.records.lock().unwrap();
        let Some((session, current)) = sessions.get_mut(&id) else {
            return Ok(RefreshRotationResult::Rejected);
        };
        if current != old || !session.is_active(at) {
            return Ok(RefreshRotationResult::Rejected);
        }
        *current = *replacement;
        Ok(RefreshRotationResult::Rotated)
    }
    async fn revoke_session_for_user(
        &self,
        owner: UserId,
        id: SessionId,
        at: OffsetDateTime,
    ) -> Result<bool, SessionStoreError> {
        let mut sessions = self.records.lock().unwrap();
        let Some((session, _)) = sessions.get_mut(&id) else {
            return Ok(false);
        };
        if session.user_id() != owner {
            return Ok(false);
        }
        session.revoke(at);
        Ok(true)
    }
    async fn revoke_all_for_user(
        &self,
        owner: UserId,
        at: OffsetDateTime,
    ) -> Result<(), SessionStoreError> {
        self.revoke_all_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_revoke_all.load(Ordering::SeqCst) != 0 {
            return Err(SessionStoreError::Unexpected(Box::new(
                std::io::Error::other("test revoke failure"),
            )));
        }
        for (session, _) in self.records.lock().unwrap().values_mut() {
            if session.user_id() == owner {
                session.revoke(at);
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct Tokens(AtomicUsize);
impl RefreshTokenProvider for Tokens {
    fn generate(&self) -> Result<RefreshToken, RefreshTokenError> {
        let n = self.0.fetch_add(1, Ordering::SeqCst) as u8;
        RefreshToken::parse(URL_SAFE_NO_PAD.encode([n; 32]))
    }
    fn hash(&self, token: &RefreshToken) -> RefreshTokenHash {
        OsRefreshTokenProvider.hash(token)
    }
}
struct Codec;
impl AccessTokenCodec for Codec {
    fn issue(&self, claims: &AccessTokenClaims) -> Result<AccessToken, AccessTokenError> {
        Ok(AccessToken::new(format!(
            "{}:{}",
            claims.user_id(),
            claims.session_id()
        )))
    }
    fn verify(
        &self,
        _: &AccessToken,
        _: OffsetDateTime,
    ) -> Result<AccessTokenClaims, AccessTokenError> {
        Err(AccessTokenError::InvalidToken)
    }
}

#[test]
fn session_creation_maps_owner_rejections_without_reclassifying_conflicts() {
    // The authenticated snapshot is enabled throughout. Only the port knows the
    // atomic owner state; no identity reread can reinterpret its rejection.
    let user = user();
    for failure in [
        SessionStoreError::OwnerDisabled,
        SessionStoreError::OwnerNotFound,
        SessionStoreError::Conflict,
    ] {
        let disabled = matches!(failure, SessionStoreError::OwnerDisabled);
        let missing = matches!(failure, SessionStoreError::OwnerNotFound);
        let sessions = Sessions {
            create_failure: Mutex::new(Some(failure)),
            ..Default::default()
        };
        let tokens = Tokens::default();
        let error = block_on(
            CreateSession::new(&sessions, &tokens, &Codec, SessionPolicy::default())
                .execute(&user, now()),
        )
        .unwrap_err();
        if disabled {
            assert!(matches!(error, CreateSessionError::UserDisabled));
        } else if missing {
            assert!(matches!(error, CreateSessionError::UserNotFound));
        } else {
            assert!(matches!(
                error,
                CreateSessionError::Storage(SessionStoreError::Conflict)
            ));
        }
        assert!(sessions.records.lock().unwrap().is_empty());
    }
}

#[test]
fn refresh_rotates_without_extending_and_revocation_is_individual() {
    let identities = Identities::default();
    let sessions = Sessions::default();
    let tokens = Tokens::default();
    let codec = Codec;
    let user = user();
    identities.insert(user.clone());
    let create = CreateSession::new(&sessions, &tokens, &codec, SessionPolicy::default());
    let first = block_on(create.execute(&user, now())).unwrap();
    let second = block_on(create.execute(&user, now())).unwrap();
    let first_expiry = first.session().expires_at();
    let (_, _, first_refresh) = first.into_parts();
    let refresh = RefreshSession::new(
        &sessions,
        &identities,
        &tokens,
        &codec,
        SessionPolicy::default(),
    );
    let rotated = block_on(refresh.execute(&first_refresh, now() + Duration::days(1))).unwrap();
    assert_eq!(rotated.session().expires_at(), first_expiry);
    assert!(matches!(
        block_on(refresh.execute(&first_refresh, now() + Duration::days(1))),
        Err(RefreshSessionError::InvalidRefreshToken)
    ));
    let (first_session, _, replacement) = rotated.into_parts();
    block_on(RevokeSession::new(&sessions).execute(user.id(), first_session.id(), now())).unwrap();
    assert!(matches!(
        block_on(refresh.execute(&replacement, now())),
        Err(RefreshSessionError::InvalidRefreshToken)
    ));
    let (_, _, second_refresh) = second.into_parts();
    assert!(block_on(refresh.execute(&second_refresh, now())).is_ok());
}

#[test]
fn session_grant_reports_the_canonical_access_token_expiration() {
    let identities = Identities::default();
    let sessions = Sessions::default();
    let tokens = Tokens::default();
    let codec = Codec;
    let user = user();
    identities.insert(user.clone());
    let precise_now = now() + Duration::milliseconds(500);
    let grant = block_on(
        CreateSession::new(&sessions, &tokens, &codec, SessionPolicy::default())
            .execute(&user, precise_now),
    )
    .unwrap();
    assert_eq!(
        grant.access_token_expires_at(),
        now() + Duration::minutes(15)
    );
    let (_, _, refresh_token) = grant.into_parts();
    let refreshed = block_on(
        RefreshSession::new(
            &sessions,
            &identities,
            &tokens,
            &codec,
            SessionPolicy::default(),
        )
        .execute(&refresh_token, precise_now + Duration::seconds(1)),
    )
    .unwrap();
    assert_eq!(
        refreshed.access_token_expires_at(),
        now() + Duration::minutes(15) + Duration::seconds(1)
    );
}

#[test]
fn disable_revokes_sessions_and_enable_does_not_restore_them() {
    let identities = Identities::default();
    let sessions = Sessions::default();
    let tokens = Tokens::default();
    let codec = Codec;
    let user = user();
    identities.insert(user.clone());
    let grant = block_on(
        CreateSession::new(&sessions, &tokens, &codec, SessionPolicy::default())
            .execute(&user, now()),
    )
    .unwrap();
    let (_, _, refresh_token) = grant.into_parts();
    block_on(DisableUser::new(&identities, &sessions).execute(user.id(), now())).unwrap();
    let refresh = RefreshSession::new(
        &sessions,
        &identities,
        &tokens,
        &codec,
        SessionPolicy::default(),
    );
    assert!(matches!(
        block_on(refresh.execute(&refresh_token, now())),
        Err(RefreshSessionError::InvalidRefreshToken)
    ));
    block_on(EnableUser::new(&identities).execute(user.id(), now())).unwrap();
    assert!(matches!(
        block_on(refresh.execute(&refresh_token, now())),
        Err(RefreshSessionError::InvalidRefreshToken)
    ));
}

#[test]
fn disable_retry_revokes_sessions_after_partial_failure() {
    let identities = Identities::default();
    let sessions = Sessions::default();
    let tokens = Tokens::default();
    let codec = Codec;
    let user = user();
    identities.insert(user.clone());
    let grant = block_on(
        CreateSession::new(&sessions, &tokens, &codec, SessionPolicy::default())
            .execute(&user, now()),
    )
    .unwrap();
    let (_, _, refresh_token) = grant.into_parts();
    sessions.fail_revoke_all.store(1, Ordering::SeqCst);
    assert!(matches!(
        block_on(DisableUser::new(&identities, &sessions).execute(user.id(), now())),
        Err(ManageUserError::SessionStorage(_))
    ));
    assert!(
        block_on(identities.find_user(user.id()))
            .unwrap()
            .unwrap()
            .is_disabled()
    );
    let refresh = RefreshSession::new(
        &sessions,
        &identities,
        &tokens,
        &codec,
        SessionPolicy::default(),
    );
    assert!(matches!(
        block_on(refresh.execute(&refresh_token, now())),
        Err(RefreshSessionError::InvalidRefreshToken)
    ));
    sessions.fail_revoke_all.store(0, Ordering::SeqCst);
    block_on(DisableUser::new(&identities, &sessions).execute(user.id(), now())).unwrap();
    assert_eq!(sessions.revoke_all_calls.load(Ordering::SeqCst), 2);
    assert!(matches!(
        block_on(refresh.execute(&refresh_token, now())),
        Err(RefreshSessionError::InvalidRefreshToken)
    ));
}

#[test]
fn concurrent_refresh_of_one_token_has_exactly_one_winner() {
    let identities = Identities::default();
    let sessions = Sessions::default();
    let tokens = Tokens::default();
    let codec = Codec;
    let user = user();
    identities.insert(user.clone());
    let grant = block_on(
        CreateSession::new(&sessions, &tokens, &codec, SessionPolicy::default())
            .execute(&user, now()),
    )
    .unwrap();
    let (_, _, refresh_token) = grant.into_parts();
    let raw = refresh_token.expose_secret().to_owned();
    let barrier = Barrier::new(2);
    let left_raw = raw.clone();
    let sessions = &sessions;
    let identities = &identities;
    let tokens = &tokens;
    let codec = &codec;
    let barrier = &barrier;
    let results = std::thread::scope(|scope| {
        let left = scope.spawn(move || {
            let token = RefreshToken::parse(left_raw).unwrap();
            barrier.wait();
            block_on(
                RefreshSession::new(
                    sessions,
                    identities,
                    tokens,
                    codec,
                    SessionPolicy::default(),
                )
                .execute(&token, now()),
            )
            .is_ok()
        });
        let right = scope.spawn(move || {
            let token = RefreshToken::parse(raw).unwrap();
            barrier.wait();
            block_on(
                RefreshSession::new(
                    sessions,
                    identities,
                    tokens,
                    codec,
                    SessionPolicy::default(),
                )
                .execute(&token, now()),
            )
            .is_ok()
        });
        [left.join().unwrap(), right.join().unwrap()]
    });
    assert_eq!(results.into_iter().filter(|success| *success).count(), 1);
}
