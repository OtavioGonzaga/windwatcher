use time::OffsetDateTime;
use windwatcher::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationMember,
    ConversationStore, ConversationStoreError, GroupRole, GroupTitle, IdentityStore,
    IdentityStoreError, LeaveStoreOutcome, LocalAuthentication, MigrationStatus, MongoDbStorage,
    MySqlStorage, PasswordHash, PostgresStorage, RefreshRotationResult, RefreshTokenHash, Session,
    SessionId, SessionStore, SessionStoreError, SqliteStorage, SqliteStorageOptions,
    StartDirectStoreOutcome, User, UserId, Username,
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
    /// Official PostgreSQL persistence adapter.
    Postgres(PostgresStorage),
    /// Official MySQL persistence adapter.
    MySql(MySqlStorage),
    /// Official MongoDB primary persistence adapter.
    MongoDb(MongoDbStorage),
}

impl windwatcher::CreateCommandStore for Storage {
    async fn create_conversation_command(
        &self,
        command: &windwatcher::ConversationCreateCommand,
        key: Option<&windwatcher::IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<windwatcher::CreateResult<ConversationId>, windwatcher::CreateCommandError> {
        match self {
            Self::Sqlite(s) => s.create_conversation_command(command, key, now).await,
            Self::Postgres(s) => s.create_conversation_command(command, key, now).await,
            Self::MySql(s) => s.create_conversation_command(command, key, now).await,
            Self::MongoDb(s) => s.create_conversation_command(command, key, now).await,
        }
    }
    async fn send_message_command(
        &self,
        message: &Message,
        key: Option<&windwatcher::IdempotencyKey>,
        now: OffsetDateTime,
    ) -> Result<windwatcher::CreateResult<windwatcher::MessageView>, windwatcher::CreateCommandError>
    {
        match self {
            Self::Sqlite(s) => s.send_message_command(message, key, now).await,
            Self::Postgres(s) => s.send_message_command(message, key, now).await,
            Self::MySql(s) => s.send_message_command(message, key, now).await,
            Self::MongoDb(s) => s.send_message_command(message, key, now).await,
        }
    }
}

impl windwatcher::MessagingQueryStore for Storage {
    async fn get_conversation_member(
        &self,
        actor: UserId,
        conversation: ConversationId,
        target: UserId,
    ) -> Result<
        windwatcher::ConversationQueryOutcome<Option<windwatcher::ConversationMemberSummary>>,
        windwatcher::QueryStoreError,
    > {
        match self {
            Self::Sqlite(s) => s.get_conversation_member(actor, conversation, target).await,
            Self::Postgres(s) => s.get_conversation_member(actor, conversation, target).await,
            Self::MySql(s) => s.get_conversation_member(actor, conversation, target).await,
            Self::MongoDb(s) => s.get_conversation_member(actor, conversation, target).await,
        }
    }
    async fn find_users(
        &self,
        username: &Username,
    ) -> Result<Vec<windwatcher::UserSummary>, windwatcher::QueryStoreError> {
        match self {
            Self::Sqlite(s) => s.find_users(username).await,
            Self::Postgres(s) => s.find_users(username).await,
            Self::MySql(s) => s.find_users(username).await,
            Self::MongoDb(s) => s.find_users(username).await,
        }
    }
    async fn list_conversations(
        &self,
        actor: UserId,
        before: Option<windwatcher::ConversationBoundary>,
        limit: windwatcher::PageLimit,
    ) -> Result<
        windwatcher::QuerySlice<windwatcher::ConversationSummary>,
        windwatcher::QueryStoreError,
    > {
        match self {
            Self::Sqlite(s) => s.list_conversations(actor, before, limit).await,
            Self::Postgres(s) => s.list_conversations(actor, before, limit).await,
            Self::MySql(s) => s.list_conversations(actor, before, limit).await,
            Self::MongoDb(s) => s.list_conversations(actor, before, limit).await,
        }
    }
    async fn get_conversation_summary(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> Result<
        windwatcher::ConversationQueryOutcome<windwatcher::ConversationSummary>,
        windwatcher::QueryStoreError,
    > {
        match self {
            Self::Sqlite(s) => s.get_conversation_summary(actor, conversation).await,
            Self::Postgres(s) => s.get_conversation_summary(actor, conversation).await,
            Self::MySql(s) => s.get_conversation_summary(actor, conversation).await,
            Self::MongoDb(s) => s.get_conversation_summary(actor, conversation).await,
        }
    }
    async fn list_conversation_members(
        &self,
        actor: UserId,
        conversation: ConversationId,
        after: Option<windwatcher::MemberBoundary>,
        limit: windwatcher::PageLimit,
    ) -> Result<
        windwatcher::ConversationQueryOutcome<
            windwatcher::QuerySlice<windwatcher::ConversationMemberSummary>,
        >,
        windwatcher::QueryStoreError,
    > {
        match self {
            Self::Sqlite(s) => {
                s.list_conversation_members(actor, conversation, after, limit)
                    .await
            }
            Self::Postgres(s) => {
                s.list_conversation_members(actor, conversation, after, limit)
                    .await
            }
            Self::MySql(s) => {
                s.list_conversation_members(actor, conversation, after, limit)
                    .await
            }
            Self::MongoDb(s) => {
                s.list_conversation_members(actor, conversation, after, limit)
                    .await
            }
        }
    }
}
impl Storage {
    /// Opens the configured storage without applying migrations implicitly.
    pub async fn connect_provider(
        provider: crate::config::ResolvedStorageProvider,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        match provider {
            crate::config::ResolvedStorageProvider::Sqlite { options, .. } => {
                SqliteStorage::connect(options)
                    .await
                    .map(Self::Sqlite)
                    .map_err(|e| Box::new(e) as _)
            }
            crate::config::ResolvedStorageProvider::Postgres(options) => {
                windwatcher::PostgresStorage::connect(options)
                    .await
                    .map(Self::Postgres)
                    .map_err(|e| Box::new(e) as _)
            }
            crate::config::ResolvedStorageProvider::MySql(options) => {
                windwatcher::MySqlStorage::connect(options)
                    .await
                    .map(Self::MySql)
                    .map_err(|e| Box::new(e) as _)
            }
            crate::config::ResolvedStorageProvider::MongoDb(options) => {
                MongoDbStorage::connect(options)
                    .await
                    .map(Self::MongoDb)
                    .map_err(|e| Box::new(e) as _)
            }
        }
    }
    /// Opens SQLite storage without applying migrations implicitly.
    pub async fn connect(
        options: SqliteStorageOptions,
    ) -> Result<Self, windwatcher::SqliteStorageError> {
        SqliteStorage::connect(options).await.map(Self::Sqlite)
    }
    /// Explicitly applies the engine's embedded migrations.
    pub async fn migrate(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match self {
            Self::Sqlite(storage) => storage.migrate().await.map_err(|e| Box::new(e) as _),
            Self::Postgres(storage) => storage.migrate().await.map_err(|e| Box::new(e) as _),
            Self::MySql(storage) => storage.migrate().await.map_err(|e| Box::new(e) as _),
            Self::MongoDb(storage) => storage.migrate().await.map_err(|e| Box::new(e) as _),
        }
    }
    /// Inspects schema status without mutating it.
    pub async fn migration_status(
        &self,
    ) -> Result<MigrationStatus, Box<dyn std::error::Error + Send + Sync>> {
        match self {
            Self::Sqlite(storage) => storage
                .migration_status()
                .await
                .map_err(|e| Box::new(e) as _),
            Self::Postgres(storage) => storage
                .migration_status()
                .await
                .map_err(|e| Box::new(e) as _),
            Self::MySql(storage) => storage
                .migration_status()
                .await
                .map_err(|e| Box::new(e) as _),
            Self::MongoDb(storage) => storage
                .migration_status()
                .await
                .map_err(|e| Box::new(e) as _),
        }
    }
    /// Returns the non-sensitive backend label for operator output.
    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "SQLite",
            Self::Postgres(_) => "PostgreSQL",
            Self::MySql(_) => "MySQL",
            Self::MongoDb(_) => "MongoDB",
        }
    }
}
impl ConversationStore for Storage {
    async fn member_ids(&self, id: ConversationId) -> Result<Vec<UserId>, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.member_ids(id).await,
            Self::Postgres(s) => s.member_ids(id).await,
            Self::MySql(s) => s.member_ids(id).await,
            Self::MongoDb(s) => s.member_ids(id).await,
        }
    }
    async fn get_or_create_direct(
        &self,
        c: &Conversation,
        a: UserId,
        b: UserId,
    ) -> Result<StartDirectStoreOutcome, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.get_or_create_direct(c, a, b).await,
            Self::Postgres(s) => s.get_or_create_direct(c, a, b).await,
            Self::MySql(s) => s.get_or_create_direct(c, a, b).await,
            Self::MongoDb(s) => s.get_or_create_direct(c, a, b).await,
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
            Self::Postgres(s) => s.create_group(c, owner, members).await,
            Self::MySql(s) => s.create_group(c, owner, members).await,
            Self::MongoDb(s) => s.create_group(c, owner, members).await,
        }
    }
    async fn find_conversation(
        &self,
        id: ConversationId,
    ) -> Result<Option<Conversation>, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.find_conversation(id).await,
            Self::Postgres(s) => s.find_conversation(id).await,
            Self::MySql(s) => s.find_conversation(id).await,
            Self::MongoDb(s) => s.find_conversation(id).await,
        }
    }
    async fn find_member(
        &self,
        c: ConversationId,
        u: UserId,
    ) -> Result<Option<ConversationMember>, ConversationStoreError> {
        match self {
            Self::Sqlite(s) => s.find_member(c, u).await,
            Self::Postgres(s) => s.find_member(c, u).await,
            Self::MySql(s) => s.find_member(c, u).await,
            Self::MongoDb(s) => s.find_member(c, u).await,
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
            Self::Postgres(s) => s.add_group_member(c, a, m, n).await,
            Self::MySql(s) => s.add_group_member(c, a, m, n).await,
            Self::MongoDb(s) => s.add_group_member(c, a, m, n).await,
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
            Self::Postgres(s) => s.remove_group_member(c, a, m, n).await,
            Self::MySql(s) => s.remove_group_member(c, a, m, n).await,
            Self::MongoDb(s) => s.remove_group_member(c, a, m, n).await,
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
            Self::Postgres(s) => s.set_group_member_role(c, a, m, r, n).await,
            Self::MySql(s) => s.set_group_member_role(c, a, m, r, n).await,
            Self::MongoDb(s) => s.set_group_member_role(c, a, m, r, n).await,
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
            Self::Postgres(s) => s.transfer_group_ownership(c, a, t, n).await,
            Self::MySql(s) => s.transfer_group_ownership(c, a, t, n).await,
            Self::MongoDb(s) => s.transfer_group_ownership(c, a, t, n).await,
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
            Self::Postgres(s) => s.leave_group(c, a, n).await,
            Self::MySql(s) => s.leave_group(c, a, n).await,
            Self::MongoDb(s) => s.leave_group(c, a, n).await,
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
            Self::Postgres(s) => s.rename_group(c, a, t, n).await,
            Self::MySql(s) => s.rename_group(c, a, t, n).await,
            Self::MongoDb(s) => s.rename_group(c, a, t, n).await,
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
            Self::Postgres(s) => s.list_messages(actor, conversation, before, limit).await,
            Self::MySql(s) => s.list_messages(actor, conversation, before, limit).await,
            Self::MongoDb(s) => s.list_messages(actor, conversation, before, limit).await,
        }
    }
    async fn send_message(
        &self,
        message: &Message,
    ) -> Result<SendMessageStoreOutcome, MessageStoreError> {
        match self {
            Self::Sqlite(s) => s.send_message(message).await,
            Self::Postgres(s) => s.send_message(message).await,
            Self::MySql(s) => s.send_message(message).await,
            Self::MongoDb(s) => s.send_message(message).await,
        }
    }
    async fn get_message(
        &self,
        actor: UserId,
        message: MessageId,
    ) -> Result<GetMessageStoreOutcome, MessageStoreError> {
        match self {
            Self::Sqlite(s) => s.get_message(actor, message).await,
            Self::Postgres(s) => s.get_message(actor, message).await,
            Self::MySql(s) => s.get_message(actor, message).await,
            Self::MongoDb(s) => s.get_message(actor, message).await,
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
            Self::Postgres(s) => s.edit_message(actor, message, content, now).await,
            Self::MySql(s) => s.edit_message(actor, message, content, now).await,
            Self::MongoDb(s) => s.edit_message(actor, message, content, now).await,
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
            Self::Postgres(s) => s.delete_message(actor, message, now).await,
            Self::MySql(s) => s.delete_message(actor, message, now).await,
            Self::MongoDb(s) => s.delete_message(actor, message, now).await,
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
            Self::Postgres(s) => s.mark_read_through(actor, conversation, message).await,
            Self::MySql(s) => s.mark_read_through(actor, conversation, message).await,
            Self::MongoDb(s) => s.mark_read_through(actor, conversation, message).await,
        }
    }
    async fn unread_count(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> Result<UnreadCountStoreOutcome, MessageStoreError> {
        match self {
            Self::Sqlite(s) => s.unread_count(actor, conversation).await,
            Self::Postgres(s) => s.unread_count(actor, conversation).await,
            Self::MySql(s) => s.unread_count(actor, conversation).await,
            Self::MongoDb(s) => s.unread_count(actor, conversation).await,
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
            Self::Postgres(storage) => storage.create_local_user(user, hash).await,
            Self::MySql(storage) => storage.create_local_user(user, hash).await,
            Self::MongoDb(storage) => storage.create_local_user(user, hash).await,
        }
    }
    async fn find_local_auth_by_username(
        &self,
        username: &Username,
    ) -> Result<Option<LocalAuthentication>, IdentityStoreError> {
        match self {
            Self::Sqlite(storage) => storage.find_local_auth_by_username(username).await,
            Self::Postgres(storage) => storage.find_local_auth_by_username(username).await,
            Self::MySql(storage) => storage.find_local_auth_by_username(username).await,
            Self::MongoDb(storage) => storage.find_local_auth_by_username(username).await,
        }
    }
    async fn find_user(&self, id: UserId) -> Result<Option<User>, IdentityStoreError> {
        match self {
            Self::Sqlite(storage) => storage.find_user(id).await,
            Self::Postgres(storage) => storage.find_user(id).await,
            Self::MySql(storage) => storage.find_user(id).await,
            Self::MongoDb(storage) => storage.find_user(id).await,
        }
    }
    async fn update_user(&self, user: &User) -> Result<(), IdentityStoreError> {
        match self {
            Self::Sqlite(storage) => storage.update_user(user).await,
            Self::Postgres(storage) => storage.update_user(user).await,
            Self::MySql(storage) => storage.update_user(user).await,
            Self::MongoDb(storage) => storage.update_user(user).await,
        }
    }
}
impl SessionStore for Storage {
    async fn find_session(&self, id: SessionId) -> Result<Option<Session>, SessionStoreError> {
        match self {
            Self::Sqlite(s) => s.find_session(id).await,
            Self::Postgres(s) => s.find_session(id).await,
            Self::MySql(s) => s.find_session(id).await,
            Self::MongoDb(s) => s.find_session(id).await,
        }
    }
    async fn create_session(
        &self,
        session: &Session,
        hash: &RefreshTokenHash,
    ) -> Result<(), SessionStoreError> {
        match self {
            Self::Sqlite(storage) => storage.create_session(session, hash).await,
            Self::Postgres(storage) => storage.create_session(session, hash).await,
            Self::MySql(storage) => storage.create_session(session, hash).await,
            Self::MongoDb(storage) => storage.create_session(session, hash).await,
        }
    }
    async fn find_by_refresh_token_hash(
        &self,
        hash: &RefreshTokenHash,
    ) -> Result<Option<Session>, SessionStoreError> {
        match self {
            Self::Sqlite(storage) => storage.find_by_refresh_token_hash(hash).await,
            Self::Postgres(storage) => storage.find_by_refresh_token_hash(hash).await,
            Self::MySql(storage) => storage.find_by_refresh_token_hash(hash).await,
            Self::MongoDb(storage) => storage.find_by_refresh_token_hash(hash).await,
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
            Self::Postgres(storage) => {
                storage
                    .rotate_refresh_token(id, old, replacement, now)
                    .await
            }
            Self::MySql(storage) => {
                storage
                    .rotate_refresh_token(id, old, replacement, now)
                    .await
            }
            Self::MongoDb(storage) => {
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
            Self::Postgres(storage) => {
                storage
                    .revoke_session_for_user(user_id, session_id, revoked_at)
                    .await
            }
            Self::MySql(storage) => {
                storage
                    .revoke_session_for_user(user_id, session_id, revoked_at)
                    .await
            }
            Self::MongoDb(storage) => {
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
            Self::Postgres(storage) => storage.revoke_all_for_user(user_id, revoked_at).await,
            Self::MySql(storage) => storage.revoke_all_for_user(user_id, revoked_at).await,
            Self::MongoDb(storage) => storage.revoke_all_for_user(user_id, revoked_at).await,
        }
    }
}
