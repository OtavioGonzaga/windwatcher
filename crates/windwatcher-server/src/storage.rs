use time::OffsetDateTime;
use windwatcher::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationMember,
    ConversationStore, ConversationStoreError, GroupRole, GroupTitle, IdentityStore,
    IdentityStoreError, LeaveStoreOutcome, LocalAuthentication, MigrationStatus, PasswordHash,
    RefreshRotationResult, RefreshTokenHash, Session, SessionId, SessionStore, SessionStoreError,
    SqliteStorage, SqliteStorageError, SqliteStorageOptions, StartDirectStoreOutcome, User, UserId,
    Username,
};
use windwatcher::{
    DeleteMessageStoreOutcome, EditMessageStoreOutcome, GetMessageStoreOutcome,
    MarkReadStoreOutcome, Message, MessageContent, MessageId, MessageStore, MessageStoreError,
    SendMessageStoreOutcome, UnreadCountStoreOutcome,
};

#[derive(Clone, Debug)]
/// Concrete server storage dispatch; HTTP shares the existing database adapter.
pub enum Storage {
    /// Official SQLite persistence adapter.
    Sqlite(SqliteStorage),
}
impl Storage {
    /// Opens the configured storage without applying migrations implicitly.
    pub async fn connect(options: SqliteStorageOptions) -> Result<Self, SqliteStorageError> {
        Ok(Self::Sqlite(SqliteStorage::connect(options).await?))
    }
    /// Explicitly applies the engine's embedded migrations.
    pub async fn migrate(&self) -> Result<(), SqliteStorageError> {
        match self {
            Self::Sqlite(storage) => storage.migrate().await,
        }
    }
    /// Inspects schema status without mutating it.
    pub async fn migration_status(&self) -> Result<MigrationStatus, SqliteStorageError> {
        match self {
            Self::Sqlite(storage) => storage.migration_status().await,
        }
    }
    /// Returns the non-sensitive backend label for operator output.
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
        t: &GroupTitle,
        n: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.rename_group(c, a, t, n).await,
        }
    }
}
impl MessageStore for Storage {
    async fn list_messages(
        &self,
        actor: UserId,
        conversation: ConversationId,
        before: Option<MessageId>,
        limit: windwatcher::PageLimit,
    ) -> Result<windwatcher::ListMessagesStoreOutcome, MessageStoreError> {
        match self {
            Self::Sqlite(s) => s.list_messages(actor, conversation, before, limit).await,
        }
    }
    async fn send_message(
        &self,
        message: &Message,
    ) -> Result<SendMessageStoreOutcome, MessageStoreError> {
        match self {
            Self::Sqlite(s) => s.send_message(message).await,
        }
    }
    async fn get_message(
        &self,
        actor: UserId,
        message: MessageId,
    ) -> Result<GetMessageStoreOutcome, MessageStoreError> {
        match self {
            Self::Sqlite(s) => s.get_message(actor, message).await,
        }
    }
    async fn edit_message(
        &self,
        actor: UserId,
        message: MessageId,
        content: &MessageContent,
        now: OffsetDateTime,
    ) -> Result<EditMessageStoreOutcome, MessageStoreError> {
        match self {
            Self::Sqlite(s) => s.edit_message(actor, message, content, now).await,
        }
    }
    async fn delete_message(
        &self,
        actor: UserId,
        message: MessageId,
        now: OffsetDateTime,
    ) -> Result<DeleteMessageStoreOutcome, MessageStoreError> {
        match self {
            Self::Sqlite(s) => s.delete_message(actor, message, now).await,
        }
    }
    async fn mark_read_through(
        &self,
        actor: UserId,
        conversation: ConversationId,
        message: MessageId,
    ) -> Result<MarkReadStoreOutcome, MessageStoreError> {
        match self {
            Self::Sqlite(s) => s.mark_read_through(actor, conversation, message).await,
        }
    }
    async fn unread_count(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> Result<UnreadCountStoreOutcome, MessageStoreError> {
        match self {
            Self::Sqlite(s) => s.unread_count(actor, conversation).await,
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
