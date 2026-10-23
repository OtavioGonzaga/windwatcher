use time::{Duration, OffsetDateTime};
use windwatcher::{
    OsRefreshTokenProvider, RefreshRotationResult, RefreshTokenProvider, Session, SessionId,
    SessionStore, UserId,
};

fn timestamp(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}

/// Exercises implementation-independent session persistence semantics.
pub async fn run<S: SessionStore>(store: &S, user_id: UserId) {
    let provider = OsRefreshTokenProvider;
    let first = provider.generate().unwrap();
    let replacement = provider.generate().unwrap();
    let old_hash = provider.hash(&first);
    let replacement_hash = provider.hash(&replacement);
    let session = Session::new(
        SessionId::new(),
        user_id,
        timestamp(10),
        timestamp(10) + Duration::days(1),
    )
    .unwrap();
    store.create_session(&session, &old_hash).await.unwrap();
    assert_eq!(
        store
            .find_by_refresh_token_hash(&old_hash)
            .await
            .unwrap()
            .unwrap()
            .id(),
        session.id()
    );
    assert_eq!(
        store
            .rotate_refresh_token(session.id(), &old_hash, &replacement_hash, timestamp(11))
            .await
            .unwrap(),
        RefreshRotationResult::Rotated
    );
    assert_eq!(
        store
            .rotate_refresh_token(session.id(), &old_hash, &replacement_hash, timestamp(11))
            .await
            .unwrap(),
        RefreshRotationResult::Rejected
    );
    assert!(
        store
            .revoke_session_for_user(user_id, session.id(), timestamp(12))
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .rotate_refresh_token(session.id(), &replacement_hash, &old_hash, timestamp(13))
            .await
            .unwrap(),
        RefreshRotationResult::Rejected
    );
}
