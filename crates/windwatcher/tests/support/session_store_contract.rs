use time::{Duration, OffsetDateTime};
use windwatcher::{
    OsRefreshTokenProvider, RefreshRotationResult, RefreshTokenProvider, Session, SessionId,
    SessionStore, SessionStoreError, UserId,
};

fn timestamp(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}

/// Exercises implementation-independent session persistence semantics. Fixtures
/// must supply two persisted enabled owners, one persisted disabled owner and
/// one ID with no persisted identity. All IDs must be distinct.
pub async fn run<S: SessionStore>(
    store: &S,
    user_id: UserId,
    other_user_id: UserId,
    disabled_user_id: UserId,
    missing_user_id: UserId,
) {
    let provider = OsRefreshTokenProvider;
    for (owner, disabled) in [(disabled_user_id, true), (missing_user_id, false)] {
        let hash = provider.hash(&provider.generate().unwrap());
        let rejected = Session::new(SessionId::new(), owner, timestamp(10), timestamp(20)).unwrap();
        let result = store.create_session(&rejected, &hash).await;
        if disabled {
            assert!(matches!(result, Err(SessionStoreError::OwnerDisabled)));
        } else {
            assert!(matches!(result, Err(SessionStoreError::OwnerNotFound)));
        }
        assert!(
            store
                .find_by_refresh_token_hash(&hash)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !store
                .revoke_session_for_user(owner, rejected.id(), timestamp(11))
                .await
                .unwrap()
        );
    }
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
        store.find_session(session.id()).await.unwrap(),
        Some(session.clone())
    );
    assert!(
        store
            .find_session(SessionId::new())
            .await
            .unwrap()
            .is_none()
    );
    let validator = windwatcher::ValidateSession::new(store);
    assert_eq!(
        validator
            .execute(user_id, session.id(), timestamp(11))
            .await
            .unwrap(),
        session
    );
    assert!(matches!(
        validator
            .execute(other_user_id, session.id(), timestamp(11))
            .await,
        Err(windwatcher::ValidateSessionError::Inactive)
    ));
    assert!(matches!(
        validator
            .execute(user_id, session.id(), session.expires_at())
            .await,
        Err(windwatcher::ValidateSessionError::Inactive)
    ));
    assert!(matches!(
        validator
            .execute(user_id, SessionId::new(), timestamp(11))
            .await,
        Err(windwatcher::ValidateSessionError::Inactive)
    ));
    // Duplicate session IDs and refresh fingerprints are persistence conflicts,
    // never semantic owner rejections, and must not replace the existing row.
    let independent_hash = provider.hash(&provider.generate().unwrap());
    assert!(matches!(
        store.create_session(&session, &independent_hash).await,
        Err(SessionStoreError::Conflict)
    ));
    assert!(
        store
            .find_by_refresh_token_hash(&independent_hash)
            .await
            .unwrap()
            .is_none()
    );
    let duplicate_hash = Session::new(
        SessionId::new(),
        other_user_id,
        timestamp(10),
        timestamp(20),
    )
    .unwrap();
    assert!(matches!(
        store.create_session(&duplicate_hash, &old_hash).await,
        Err(SessionStoreError::Conflict)
    ));
    assert!(
        !store
            .revoke_session_for_user(other_user_id, duplicate_hash.id(), timestamp(11))
            .await
            .unwrap()
    );
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
    assert!(
        store
            .revoke_session_for_user(user_id, session.id(), timestamp(13))
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .find_by_refresh_token_hash(&replacement_hash)
            .await
            .unwrap()
            .unwrap()
            .revoked_at(),
        Some(timestamp(12))
    );
    assert_eq!(
        store
            .rotate_refresh_token(session.id(), &replacement_hash, &old_hash, timestamp(13))
            .await
            .unwrap(),
        RefreshRotationResult::Rejected
    );

    let second_token = provider.generate().unwrap();
    let second_hash = provider.hash(&second_token);
    let second_session = Session::new(
        SessionId::new(),
        user_id,
        timestamp(14),
        timestamp(14) + Duration::days(1),
    )
    .unwrap();
    store
        .create_session(&second_session, &second_hash)
        .await
        .unwrap();
    assert!(
        !store
            .revoke_session_for_user(other_user_id, second_session.id(), timestamp(15))
            .await
            .unwrap()
    );
    assert!(
        !store
            .revoke_session_for_user(user_id, SessionId::new(), timestamp(15))
            .await
            .unwrap()
    );

    let expired_token = provider.generate().unwrap();
    let expired_hash = provider.hash(&expired_token);
    let expired_replacement = provider.hash(&provider.generate().unwrap());
    let expired_session =
        Session::new(SessionId::new(), user_id, timestamp(1), timestamp(2)).unwrap();
    store
        .create_session(&expired_session, &expired_hash)
        .await
        .unwrap();
    assert_eq!(
        store
            .rotate_refresh_token(
                expired_session.id(),
                &expired_hash,
                &expired_replacement,
                timestamp(2),
            )
            .await
            .unwrap(),
        RefreshRotationResult::Rejected
    );

    let other_token = provider.generate().unwrap();
    let other_hash = provider.hash(&other_token);
    let other_session = Session::new(
        SessionId::new(),
        other_user_id,
        timestamp(14),
        timestamp(14) + Duration::days(1),
    )
    .unwrap();
    store
        .create_session(&other_session, &other_hash)
        .await
        .unwrap();
    store
        .revoke_all_for_user(user_id, timestamp(20))
        .await
        .unwrap();
    store
        .revoke_all_for_user(user_id, timestamp(21))
        .await
        .unwrap();
    assert_eq!(
        store
            .find_by_refresh_token_hash(&second_hash)
            .await
            .unwrap()
            .unwrap()
            .revoked_at(),
        Some(timestamp(20))
    );
    assert_eq!(
        store
            .find_by_refresh_token_hash(&replacement_hash)
            .await
            .unwrap()
            .unwrap()
            .revoked_at(),
        Some(timestamp(12))
    );
    assert!(
        store
            .find_by_refresh_token_hash(&other_hash)
            .await
            .unwrap()
            .unwrap()
            .revoked_at()
            .is_none()
    );
}
