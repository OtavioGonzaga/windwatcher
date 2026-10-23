use time::{Duration, OffsetDateTime};
use windwatcher::{IdentityStore, IdentityStoreError, PasswordHash, User, UserId, Username};

const HASH: &str = "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA";

fn timestamp(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}

/// Exercises implementation-independent identity persistence semantics.
pub async fn run<S: IdentityStore>(store: &S) {
    let created_at = timestamp(1_700_000_000) + Duration::nanoseconds(999);
    let username: Username = "alice".parse().unwrap();
    let user = User::new(UserId::new(), username.clone(), created_at);
    let hash = PasswordHash::parse(HASH).unwrap();
    store.create_local_user(&user, &hash).await.unwrap();

    let loaded = store.find_user(user.id()).await.unwrap().unwrap();
    assert_eq!(loaded.id(), user.id());
    assert_eq!(loaded.username(), &username);
    assert_eq!(loaded.created_at(), timestamp(1_700_000_000));
    let authentication = store
        .find_local_auth_by_username(&username)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(authentication.password_hash().as_encoded(), HASH);

    let duplicate = User::new(UserId::new(), username, timestamp(2));
    assert!(matches!(
        store.create_local_user(&duplicate, &hash).await,
        Err(IdentityStoreError::UsernameConflict)
    ));
    assert!(store.find_user(duplicate.id()).await.unwrap().is_none());

    let mut disabled = loaded;
    disabled.disable(timestamp(20));
    store.update_user(&disabled).await.unwrap();
    assert_eq!(store.find_user(user.id()).await.unwrap(), Some(disabled));
    let missing = User::new(UserId::new(), "other".parse().unwrap(), timestamp(0));
    assert!(matches!(
        store.update_user(&missing).await,
        Err(IdentityStoreError::NotFound)
    ));
}
