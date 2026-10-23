use time::OffsetDateTime;
use windwatcher::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationMember,
    ConversationStore, ConversationStoreError, GroupRole, IdentityStore, IdentityStoreError,
    LeaveStoreOutcome, LocalAuthentication, MigrationStatus, PasswordHash, RefreshRotationResult,
    RefreshTokenHash, Session, SessionId, SessionStore, SessionStoreError, SqliteStorage,
    SqliteStorageError, SqliteStorageOptions, StartDirectStoreOutcome, User, UserId, Username,
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
impl ConversationStore for Storage {
    async fn get_or_create_direct(
        &self,
        c: &Conversation,
        a: UserId,
        b: UserId,
    ) -> Result<StartDirectStoreOutcome, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.get_or_create_direct(c, a, b).await,
        }
    }
    async fn create_group(
        &self,
        c: &Conversation,
        owner: UserId,
        members: &[UserId],
    ) -> Result<(), ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.create_group(c, owner, members).await,
        }
    }
    async fn find_conversation(
        &self,
        id: ConversationId,
    ) -> Result<Option<Conversation>, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.find_conversation(id).await,
        }
    }
    async fn find_member(
        &self,
        c: ConversationId,
        u: UserId,
    ) -> Result<Option<ConversationMember>, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.find_member(c, u).await,
        }
    }
    async fn add_group_member(
        &self,
        c: ConversationId,
        a: UserId,
        m: UserId,
        n: OffsetDateTime,
    ) -> Result<AddMemberStoreOutcome, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.add_group_member(c, a, m, n).await,
        }
    }
    async fn remove_group_member(
        &self,
        c: ConversationId,
        a: UserId,
        m: UserId,
        n: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.remove_group_member(c, a, m, n).await,
        }
    }
    async fn set_group_member_role(
        &self,
        c: ConversationId,
        a: UserId,
        m: UserId,
        r: GroupRole,
        n: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.set_group_member_role(c, a, m, r, n).await,
        }
    }
    async fn transfer_group_ownership(
        &self,
        c: ConversationId,
        a: UserId,
        t: UserId,
        n: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.transfer_group_ownership(c, a, t, n).await,
        }
    }
    async fn leave_group(
        &self,
        c: ConversationId,
        a: UserId,
        n: OffsetDateTime,
    ) -> Result<LeaveStoreOutcome, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.leave_group(c, a, n).await,
        }
    }
    async fn rename_group(
        &self,
        c: ConversationId,
        a: UserId,
        t: &str,
        n: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.rename_group(c, a, t, n).await,
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
