use time::OffsetDateTime;
use windwatcher::{
    IdentityStore, IdentityStoreError, LocalAuthentication, MigrationStatus, PasswordHash,
    RefreshRotationResult, RefreshTokenHash, Session, SessionId, SessionStore, SessionStoreError,
    SqliteStorage, SqliteStorageError, SqliteStorageOptions, User, UserId, Username,
};

#[derive(Clone, Debug)]
pub enum Storage {
    Sqlite(SqliteStorage),
}
impl Storage {
    pub async fn connect(options: SqliteStorageOptions) -> Result<Self, SqliteStorageError> {
        Ok(Self::Sqlite(SqliteStorage::connect(options).await?))
    }
    pub async fn migrate(&self) -> Result<(), SqliteStorageError> {
        match self {
            Self::Sqlite(storage) => storage.migrate().await,
        }
    }
    pub async fn migration_status(&self) -> Result<MigrationStatus, SqliteStorageError> {
        match self {
            Self::Sqlite(storage) => storage.migration_status().await,
        }
    }
    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "SQLite",
        }
    }
}
impl IdentityStore for Storage {
    async fn create_local_user(
        &self,
        user: &User,
        hash: &PasswordHash,
    ) -> Result<(), IdentityStoreError> {
        match self {
            Self::Sqlite(storage) => storage.create_local_user(user, hash).await,
        }
    }
    async fn find_local_auth_by_username(
        &self,
        username: &Username,
    ) -> Result<Option<LocalAuthentication>, IdentityStoreError> {
        match self {
            Self::Sqlite(storage) => storage.find_local_auth_by_username(username).await,
        }
    }
    async fn find_user(&self, id: UserId) -> Result<Option<User>, IdentityStoreError> {
        match self {
            Self::Sqlite(storage) => storage.find_user(id).await,
        }
    }
    async fn update_user(&self, user: &User) -> Result<(), IdentityStoreError> {
        match self {
            Self::Sqlite(storage) => storage.update_user(user).await,
        }
    }
}
impl SessionStore for Storage {
    async fn create_session(
        &self,
        session: &Session,
        hash: &RefreshTokenHash,
    ) -> Result<(), SessionStoreError> {
        match self {
            Self::Sqlite(storage) => storage.create_session(session, hash).await,
        }
    }
    async fn find_by_refresh_token_hash(
        &self,
        hash: &RefreshTokenHash,
    ) -> Result<Option<Session>, SessionStoreError> {
        match self {
            Self::Sqlite(storage) => storage.find_by_refresh_token_hash(hash).await,
        }
    }
    async fn rotate_refresh_token(
        &self,
        id: SessionId,
        old: &RefreshTokenHash,
        replacement: &RefreshTokenHash,
        now: OffsetDateTime,
    ) -> Result<RefreshRotationResult, SessionStoreError> {
        match self {
            Self::Sqlite(storage) => {
                storage
                    .rotate_refresh_token(id, old, replacement, now)
                    .await
            }
        }
    }
    async fn revoke_session_for_user(
        &self,
        user_id: UserId,
        session_id: SessionId,
        revoked_at: OffsetDateTime,
    ) -> Result<bool, SessionStoreError> {
        match self {
            Self::Sqlite(storage) => {
                storage
                    .revoke_session_for_user(user_id, session_id, revoked_at)
                    .await
            }
        }
    }
    async fn revoke_all_for_user(
        &self,
        user_id: UserId,
        revoked_at: OffsetDateTime,
    ) -> Result<(), SessionStoreError> {
        match self {
            Self::Sqlite(storage) => storage.revoke_all_for_user(user_id, revoked_at).await,
        }
    }
}
