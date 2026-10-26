use std::{error::Error, str::FromStr, time::Duration};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow},
};
use time::OffsetDateTime;
use uuid::Uuid;

mod messages;

use crate::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationKind,
    ConversationMember, ConversationStore, ConversationStoreError, GroupRole, GroupTitle,
    IdentityStore, IdentityStoreError, LeaveStoreOutcome, LocalAuthentication, MembershipRole,
    PasswordHash, RefreshRotationResult, RefreshTokenHash, Session, SessionId, SessionStore,
    SessionStoreError, StartDirectStoreOutcome, User, UserId, Username,
};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

/// Configuration for a [`SqliteStorage`] connection pool.
#[derive(Clone, Debug)]
pub struct SqliteStorageOptions {
    database_url: String,
    create_if_missing: bool,
    max_connections: u32,
    busy_timeout: Duration,
}

impl SqliteStorageOptions {
    /// Creates options for a SQLite URL or filesystem path.
    #[must_use]
    pub fn new(database_url: impl Into<String>) -> Self {
        Self {
            database_url: database_url.into(),
            create_if_missing: true,
            max_connections: 8,
            busy_timeout: Duration::from_secs(5),
        }
    }

    /// Controls whether SQLite may create a missing file-backed database.
    #[must_use]
    pub fn create_if_missing(mut self, create_if_missing: bool) -> Self {
        self.create_if_missing = create_if_missing;
        self
    }

    /// Sets the maximum number of pooled connections.
    #[must_use]
    pub fn max_connections(mut self, max_connections: u32) -> Self {
        self.max_connections = max_connections;
        self
    }

    /// Sets SQLite's busy timeout.
    #[must_use]
    pub fn busy_timeout(mut self, busy_timeout: Duration) -> Self {
        self.busy_timeout = busy_timeout;
        self
    }

    /// Validates URL/path and pool settings without opening a database connection.
    pub fn validate(&self) -> Result<(), SqliteStorageError> {
        self.connect_options().map(|_| ())
    }

    fn connect_options(&self) -> Result<(SqliteConnectOptions, bool), SqliteStorageError> {
        if self.max_connections == 0 {
            return Err(SqliteStorageError::Configuration(
                "max_connections must be greater than zero".into(),
            ));
        }
        if self.busy_timeout.is_zero() {
            return Err(SqliteStorageError::Configuration(
                "busy_timeout must be greater than zero".into(),
            ));
        }
        let is_memory = matches!(self.database_url.as_str(), ":memory:" | "sqlite::memory:")
            || self.database_url.contains("mode=memory");
        let connect_options =
            if self.database_url.starts_with("sqlite:") || self.database_url.starts_with("file:") {
                SqliteConnectOptions::from_str(&self.database_url)
                    .map_err(|error| SqliteStorageError::Configuration(error.to_string()))?
            } else if self.database_url == ":memory:" {
                SqliteConnectOptions::new().in_memory(true)
            } else {
                SqliteConnectOptions::new().filename(&self.database_url)
            }
            .create_if_missing(self.create_if_missing)
            .foreign_keys(true)
            .busy_timeout(self.busy_timeout);
        let connect_options = if is_memory {
            connect_options
        } else {
            connect_options.journal_mode(SqliteJournalMode::Wal)
        };
        Ok((connect_options, is_memory))
    }
}

/// Failures while constructing or operating SQLite storage infrastructure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SqliteStorageError {
    /// The supplied SQLite URL or pool options were invalid.
    #[error("invalid SQLite storage configuration: {0}")]
    Configuration(String),
    /// SQLite could not establish or maintain a connection.
    #[error("SQLite connection failure")]
    Connection(#[source] Box<dyn Error + Send + Sync>),
    /// Applying or validating embedded migrations failed.
    #[error("SQLite migration failure")]
    Migration(#[source] Box<dyn Error + Send + Sync>),
    /// Schema inspection failed.
    #[error("SQLite schema inspection failure")]
    Schema(#[source] Box<dyn Error + Send + Sync>),
    /// The migration ledger was inconsistent with embedded migrations.
    #[error("invalid SQLite migration history: {0}")]
    InvalidMigrationHistory(String),
}

/// A summary of the embedded migration state without exposing SQLx ledger rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MigrationStatus {
    current_version: Option<i64>,
    latest_version: Option<i64>,
    pending: usize,
}

impl MigrationStatus {
    /// Returns the highest successfully applied embedded migration version.
    #[must_use]
    pub fn current_version(&self) -> Option<i64> {
        self.current_version
    }
    /// Returns the newest embedded migration version.
    #[must_use]
    pub fn latest_version(&self) -> Option<i64> {
        self.latest_version
    }
    /// Returns the count of embedded migrations that have not been applied.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending
    }
    /// Returns whether every embedded migration has been applied.
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.pending == 0 && self.current_version == self.latest_version
    }
}

/// SQLx-backed SQLite implementation of [`IdentityStore`] and [`SessionStore`].
#[derive(Clone, Debug)]
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    /// Opens a pool and configures each connection, without applying migrations.
    pub async fn connect(options: SqliteStorageOptions) -> Result<Self, SqliteStorageError> {
        let (connect_options, is_memory) = options.connect_options()?;
        let max_connections = if is_memory {
            1
        } else {
            options.max_connections
        };
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("PRAGMA foreign_keys = ON")
                        .execute(connection)
                        .await
                        .map(|_| ())
                })
            })
            .connect_with(connect_options)
            .await
            .map_err(|error| SqliteStorageError::Connection(Box::new(error)))?;
        Ok(Self { pool })
    }

    /// Explicitly applies pending embedded migrations and validates their history.
    pub async fn migrate(&self) -> Result<(), SqliteStorageError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|error| SqliteStorageError::Migration(Box::new(error)))
    }

    /// Inspects embedded and applied migration state without applying migrations.
    pub async fn migration_status(&self) -> Result<MigrationStatus, SqliteStorageError> {
        let ledger_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|error| SqliteStorageError::Schema(Box::new(error)))?;
        let latest_version = MIGRATOR.iter().map(|migration| migration.version).max();
        if ledger_exists == 0 {
            return Ok(MigrationStatus {
                current_version: None,
                latest_version,
                pending: MIGRATOR.iter().count(),
            });
        }
        let rows =
            sqlx::query("SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&self.pool)
                .await
                .map_err(|error| SqliteStorageError::Schema(Box::new(error)))?;
        let mut current_version = None;
        for row in rows {
            let version: i64 = row
                .try_get("version")
                .map_err(|error| SqliteStorageError::Schema(Box::new(error)))?;
            let checksum: Vec<u8> = row
                .try_get("checksum")
                .map_err(|error| SqliteStorageError::Schema(Box::new(error)))?;
            let success: bool = row
                .try_get("success")
                .map_err(|error| SqliteStorageError::Schema(Box::new(error)))?;
            let Some(migration) = MIGRATOR
                .iter()
                .find(|migration| migration.version == version)
            else {
                return Err(SqliteStorageError::InvalidMigrationHistory(format!(
                    "unknown migration version {version}"
                )));
            };
            if !success {
                return Err(SqliteStorageError::InvalidMigrationHistory(format!(
                    "migration {version} is dirty"
                )));
            }
            if checksum != migration.checksum.as_ref() {
                return Err(SqliteStorageError::InvalidMigrationHistory(format!(
                    "migration {version} checksum differs"
                )));
            }
            current_version = Some(version);
        }
        let applied = rows_count(&self.pool).await?;
        Ok(MigrationStatus {
            current_version,
            latest_version,
            pending: MIGRATOR.iter().count().saturating_sub(applied),
        })
    }

    /// Verifies that the schema is current without changing it.
    pub async fn check_schema(&self) -> Result<MigrationStatus, SqliteStorageError> {
        self.migration_status().await
    }
}

async fn rows_count(pool: &SqlitePool) -> Result<usize, SqliteStorageError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1")
        .fetch_one(pool)
        .await
        .map_err(|error| SqliteStorageError::Schema(Box::new(error)))?;
    usize::try_from(count)
        .map_err(|_| SqliteStorageError::InvalidMigrationHistory("invalid migration count".into()))
}

#[derive(Debug)]
struct Corrupt(&'static str);
impl std::fmt::Display for Corrupt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Corrupt {}

fn unexpected_identity(message: &'static str) -> IdentityStoreError {
    IdentityStoreError::Unexpected(Box::new(Corrupt(message)))
}
fn unexpected_session(message: &'static str) -> SessionStoreError {
    SessionStoreError::Unexpected(Box::new(Corrupt(message)))
}
fn sqlite_primary_code(code: &str) -> Option<u32> {
    code.parse::<u32>().ok().map(|code| code & 0xff)
}

fn unavailable(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed)
        || matches!(error, sqlx::Error::Database(database) if matches!(database.code().as_deref().and_then(sqlite_primary_code), Some(5 | 6)))
}
fn identity_error(error: sqlx::Error) -> IdentityStoreError {
    if unavailable(&error) {
        IdentityStoreError::Unavailable(Box::new(error))
    } else {
        IdentityStoreError::Unexpected(Box::new(error))
    }
}
fn session_error(error: sqlx::Error) -> SessionStoreError {
    if unavailable(&error) {
        SessionStoreError::Unavailable(Box::new(error))
    } else if matches!(&error, sqlx::Error::Database(database) if database.is_unique_violation() || database.is_foreign_key_violation())
    {
        SessionStoreError::Conflict
    } else {
        SessionStoreError::Unexpected(Box::new(error))
    }
}
fn conversation_error(error: sqlx::Error) -> ConversationStoreError {
    if unavailable(&error) {
        ConversationStoreError::Unavailable(Box::new(error))
    } else if matches!(&error, sqlx::Error::Database(database) if database.is_unique_violation() || database.is_foreign_key_violation())
    {
        ConversationStoreError::Conflict
    } else {
        ConversationStoreError::Unexpected(Box::new(error))
    }
}
fn unexpected_conversation(message: &'static str) -> ConversationStoreError {
    ConversationStoreError::Unexpected(Box::new(Corrupt(message)))
}
fn id_bytes(id: impl Into<Uuid>) -> Vec<u8> {
    id.into().into_bytes().to_vec()
}
fn read_user_id(bytes: Vec<u8>) -> Result<UserId, IdentityStoreError> {
    Uuid::from_slice(&bytes)
        .map_err(|_| unexpected_identity("invalid persisted user ID"))
        .and_then(|id| {
            UserId::try_from(id).map_err(|_| unexpected_identity("invalid persisted user ID"))
        })
}
fn read_session_id(bytes: Vec<u8>) -> Result<SessionId, SessionStoreError> {
    Uuid::from_slice(&bytes)
        .map_err(|_| unexpected_session("invalid persisted session ID"))
        .and_then(|id| {
            SessionId::try_from(id).map_err(|_| unexpected_session("invalid persisted session ID"))
        })
}
fn read_conversation_id(bytes: Vec<u8>) -> Result<ConversationId, ConversationStoreError> {
    Uuid::from_slice(&bytes)
        .map_err(|_| unexpected_conversation("invalid persisted conversation ID"))
        .and_then(|id| {
            ConversationId::try_from(id)
                .map_err(|_| unexpected_conversation("invalid persisted conversation ID"))
        })
}
fn to_micros(time: OffsetDateTime) -> Result<i64, &'static str> {
    i64::try_from(time.unix_timestamp_nanos().div_euclid(1_000))
        .map_err(|_| "timestamp does not fit Unix microseconds")
}
fn from_micros(value: i64) -> Result<OffsetDateTime, &'static str> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1_000)
        .map_err(|_| "invalid persisted timestamp")
}
fn read_user(row: &SqliteRow) -> Result<User, IdentityStoreError> {
    let id = read_user_id(row.try_get("id").map_err(identity_error)?)?;
    let username = Username::from_str(
        &row.try_get::<String, _>("username")
            .map_err(identity_error)?,
    )
    .map_err(|_| unexpected_identity("invalid persisted username"))?;
    let disabled_at = row
        .try_get::<Option<i64>, _>("disabled_at")
        .map_err(identity_error)?
        .map(|value| from_micros(value).map_err(unexpected_identity))
        .transpose()?;
    let created_at = from_micros(row.try_get("created_at").map_err(identity_error)?)
        .map_err(unexpected_identity)?;
    let updated_at = from_micros(row.try_get("updated_at").map_err(identity_error)?)
        .map_err(unexpected_identity)?;
    Ok(User::restore(
        id,
        username,
        disabled_at,
        created_at,
        updated_at,
    ))
}
fn read_session(row: &SqliteRow) -> Result<Session, SessionStoreError> {
    let id = read_session_id(row.try_get("id").map_err(session_error)?)?;
    let user_id = Uuid::from_slice(
        &row.try_get::<Vec<u8>, _>("user_id")
            .map_err(session_error)?,
    )
    .map_err(|_| unexpected_session("invalid persisted user ID"))
    .and_then(|id| {
        UserId::try_from(id).map_err(|_| unexpected_session("invalid persisted user ID"))
    })?;
    let created_at = from_micros(row.try_get("created_at").map_err(session_error)?)
        .map_err(unexpected_session)?;
    let expires_at = from_micros(row.try_get("expires_at").map_err(session_error)?)
        .map_err(unexpected_session)?;
    let revoked_at = row
        .try_get::<Option<i64>, _>("revoked_at")
        .map_err(session_error)?
        .map(|value| from_micros(value).map_err(unexpected_session))
        .transpose()?;
    Session::restore(id, user_id, created_at, expires_at, revoked_at)
        .map_err(|_| unexpected_session("invalid persisted session"))
}
fn read_conversation(row: &SqliteRow) -> Result<Conversation, ConversationStoreError> {
    let id = read_conversation_id(row.try_get("id").map_err(conversation_error)?)?;
    let kind = match row
        .try_get::<String, _>("kind")
        .map_err(conversation_error)?
        .as_str()
    {
        "direct" => ConversationKind::Direct,
        "group" => ConversationKind::Group,
        _ => {
            return Err(unexpected_conversation(
                "invalid persisted conversation kind",
            ));
        }
    };
    let title = row
        .try_get::<Option<String>, _>("title")
        .map_err(conversation_error)?
        .map(GroupTitle::new)
        .transpose()
        .map_err(|_| unexpected_conversation("invalid persisted group title"))?;
    let created_at = from_micros(row.try_get("created_at").map_err(conversation_error)?)
        .map_err(unexpected_conversation)?;
    let updated_at = from_micros(row.try_get("updated_at").map_err(conversation_error)?)
        .map_err(unexpected_conversation)?;
    Conversation::restore(id, kind, title, created_at, updated_at)
        .map_err(|_| unexpected_conversation("invalid persisted conversation"))
}
fn read_member(row: &SqliteRow) -> Result<ConversationMember, ConversationStoreError> {
    let conversation_id =
        read_conversation_id(row.try_get("conversation_id").map_err(conversation_error)?)?;
    let user_id = read_user_id(row.try_get("user_id").map_err(conversation_error)?)
        .map_err(|_| unexpected_conversation("invalid persisted user ID"))?;
    let role = match row
        .try_get::<String, _>("role")
        .map_err(conversation_error)?
        .as_str()
    {
        "direct" => MembershipRole::Direct,
        "owner" => MembershipRole::Group(GroupRole::Owner),
        "admin" => MembershipRole::Group(GroupRole::Admin),
        "member" => MembershipRole::Group(GroupRole::Member),
        _ => return Err(unexpected_conversation("invalid persisted membership role")),
    };
    let joined_at = from_micros(row.try_get("joined_at").map_err(conversation_error)?)
        .map_err(unexpected_conversation)?;
    let marker = row
        .try_get::<Option<Vec<u8>>, _>("last_read_message_id")
        .map_err(conversation_error)?
        .map(|bytes| {
            Uuid::from_slice(&bytes)
                .ok()
                .and_then(|id| crate::MessageId::try_from(id).ok())
                .ok_or_else(|| unexpected_conversation("invalid persisted read marker"))
        })
        .transpose()?;
    Ok(ConversationMember::restore(
        conversation_id,
        user_id,
        role,
        joined_at,
        marker,
    ))
}
fn direct_key(first: UserId, second: UserId) -> Vec<u8> {
    let mut a = first.as_uuid().into_bytes();
    let mut b = second.as_uuid().into_bytes();
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    [a.as_slice(), b.as_slice()].concat()
}

impl IdentityStore for SqliteStorage {
    async fn create_local_user(
        &self,
        user: &User,
        password_hash: &PasswordHash,
    ) -> Result<(), IdentityStoreError> {
        let mut transaction = self.pool.begin().await.map_err(identity_error)?;
        let result = sqlx::query("INSERT INTO users (id, username, disabled_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(username) DO NOTHING")
            .bind(id_bytes(user.id())).bind(user.username().as_str())
            .bind(user.disabled_at().map(to_micros).transpose().map_err(unexpected_identity)?)
            .bind(to_micros(user.created_at()).map_err(unexpected_identity)?)
            .bind(to_micros(user.updated_at()).map_err(unexpected_identity)?)
            .execute(&mut *transaction).await.map_err(identity_error)?;
        if result.rows_affected() == 0 {
            return Err(IdentityStoreError::UsernameConflict);
        }
        sqlx::query("INSERT INTO local_credentials (user_id, password_hash) VALUES (?, ?)")
            .bind(id_bytes(user.id()))
            .bind(password_hash.as_encoded())
            .execute(&mut *transaction)
            .await
            .map_err(identity_error)?;
        transaction.commit().await.map_err(identity_error)
    }
    async fn find_local_auth_by_username(
        &self,
        username: &Username,
    ) -> Result<Option<LocalAuthentication>, IdentityStoreError> {
        let row = sqlx::query("SELECT u.id, u.username, u.disabled_at, u.created_at, u.updated_at, c.password_hash FROM users u JOIN local_credentials c ON c.user_id = u.id WHERE u.username = ?")
            .bind(username.as_str()).fetch_optional(&self.pool).await.map_err(identity_error)?;
        row.map(|row| {
            let user = read_user(&row)?;
            let encoded: String = row.try_get("password_hash").map_err(identity_error)?;
            let hash = PasswordHash::parse(&encoded)
                .map_err(|_| unexpected_identity("invalid persisted password hash"))?;
            Ok(LocalAuthentication::new(user, hash))
        })
        .transpose()
    }
    async fn find_user(&self, id: UserId) -> Result<Option<User>, IdentityStoreError> {
        sqlx::query(
            "SELECT id, username, disabled_at, created_at, updated_at FROM users WHERE id = ?",
        )
        .bind(id_bytes(id))
        .fetch_optional(&self.pool)
        .await
        .map_err(identity_error)?
        .map(|row| read_user(&row))
        .transpose()
    }
    async fn update_user(&self, user: &User) -> Result<(), IdentityStoreError> {
        let result = sqlx::query("UPDATE users SET disabled_at = ?, updated_at = ? WHERE id = ?")
            .bind(
                user.disabled_at()
                    .map(to_micros)
                    .transpose()
                    .map_err(unexpected_identity)?,
            )
            .bind(to_micros(user.updated_at()).map_err(unexpected_identity)?)
            .bind(id_bytes(user.id()))
            .execute(&self.pool)
            .await
            .map_err(identity_error)?;
        if result.rows_affected() == 0 {
            Err(IdentityStoreError::NotFound)
        } else {
            Ok(())
        }
    }
}

impl SessionStore for SqliteStorage {
    async fn create_session(
        &self,
        session: &Session,
        hash: &RefreshTokenHash,
    ) -> Result<(), SessionStoreError> {
        // Classify owner rejection and insert under the same write lock. No
        // re-enable/delete can change the meaning between these operations.
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(session_error)?;
        let owner = sqlx::query("SELECT disabled_at FROM users WHERE id = ?")
            .bind(id_bytes(session.user_id()))
            .fetch_optional(&mut *transaction)
            .await
            .map_err(session_error)?
            .ok_or(SessionStoreError::OwnerNotFound)?;
        if owner
            .try_get::<Option<i64>, _>("disabled_at")
            .map_err(session_error)?
            .is_some()
        {
            return Err(SessionStoreError::OwnerDisabled);
        }
        sqlx::query("INSERT INTO sessions (id, user_id, refresh_token_hash, created_at, expires_at, revoked_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(id_bytes(session.id())).bind(id_bytes(session.user_id())).bind(hash.as_bytes().to_vec())
            .bind(to_micros(session.created_at()).map_err(unexpected_session)?)
            .bind(to_micros(session.expires_at()).map_err(unexpected_session)?)
            .bind(session.revoked_at().map(to_micros).transpose().map_err(unexpected_session)?)
            .execute(&mut *transaction).await.map_err(session_error)?;
        transaction.commit().await.map_err(session_error)
    }
    async fn find_by_refresh_token_hash(
        &self,
        hash: &RefreshTokenHash,
    ) -> Result<Option<Session>, SessionStoreError> {
        sqlx::query("SELECT id, user_id, created_at, expires_at, revoked_at FROM sessions WHERE refresh_token_hash = ?")
            .bind(hash.as_bytes().to_vec()).fetch_optional(&self.pool).await.map_err(session_error)?
            .map(|row| read_session(&row)).transpose()
    }
    async fn rotate_refresh_token(
        &self,
        id: SessionId,
        old: &RefreshTokenHash,
        replacement: &RefreshTokenHash,
        now: OffsetDateTime,
    ) -> Result<RefreshRotationResult, SessionStoreError> {
        let result = sqlx::query("UPDATE sessions SET refresh_token_hash = ? WHERE id = ? AND refresh_token_hash = ? AND revoked_at IS NULL AND expires_at > ?")
            .bind(replacement.as_bytes().to_vec()).bind(id_bytes(id)).bind(old.as_bytes().to_vec())
            .bind(to_micros(now).map_err(unexpected_session)?)
            .execute(&self.pool).await.map_err(session_error)?;
        Ok(if result.rows_affected() == 1 {
            RefreshRotationResult::Rotated
        } else {
            RefreshRotationResult::Rejected
        })
    }
    async fn revoke_session_for_user(
        &self,
        user_id: UserId,
        session_id: SessionId,
        revoked_at: OffsetDateTime,
    ) -> Result<bool, SessionStoreError> {
        let result = sqlx::query(
            "UPDATE sessions SET revoked_at = COALESCE(revoked_at, ?) WHERE id = ? AND user_id = ?",
        )
        .bind(to_micros(revoked_at).map_err(unexpected_session)?)
        .bind(id_bytes(session_id))
        .bind(id_bytes(user_id))
        .execute(&self.pool)
        .await
        .map_err(session_error)?;
        Ok(result.rows_affected() != 0)
    }
    async fn revoke_all_for_user(
        &self,
        user_id: UserId,
        revoked_at: OffsetDateTime,
    ) -> Result<(), SessionStoreError> {
        sqlx::query("UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
            .bind(to_micros(revoked_at).map_err(unexpected_session)?)
            .bind(id_bytes(user_id))
            .execute(&self.pool)
            .await
            .map_err(session_error)?;
        Ok(())
    }
}

impl ConversationStore for SqliteStorage {
    async fn get_or_create_direct(
        &self,
        conversation: &Conversation,
        first: UserId,
        second: UserId,
    ) -> Result<StartDirectStoreOutcome, ConversationStoreError> {
        if conversation.kind() != ConversationKind::Direct {
            return Err(ConversationStoreError::InvalidConversationKind {
                expected: ConversationKind::Direct,
                actual: conversation.kind(),
            });
        }
        let key = direct_key(first, second);
        let mut transaction = self.pool.begin().await.map_err(conversation_error)?;
        let inserted = sqlx::query("INSERT INTO conversations (id, kind, title, direct_key, created_at, updated_at) VALUES (?, 'direct', NULL, ?, ?, ?) ON CONFLICT(direct_key) DO NOTHING")
            .bind(id_bytes(conversation.id())).bind(&key)
            .bind(to_micros(conversation.created_at()).map_err(unexpected_conversation)?)
            .bind(to_micros(conversation.updated_at()).map_err(unexpected_conversation)?)
            .execute(&mut *transaction).await.map_err(conversation_error)?;
        if inserted.rows_affected() == 1 {
            for user_id in [first, second] {
                sqlx::query("INSERT INTO conversation_members (conversation_id, user_id, role, joined_at) VALUES (?, ?, 'direct', ?) ON CONFLICT(conversation_id, user_id) DO NOTHING")
                    .bind(id_bytes(conversation.id())).bind(id_bytes(user_id))
                    .bind(to_micros(conversation.created_at()).map_err(unexpected_conversation)?)
                    .execute(&mut *transaction).await.map_err(conversation_error)?;
            }
            transaction.commit().await.map_err(conversation_error)?;
            Ok(StartDirectStoreOutcome::Created(conversation.clone()))
        } else {
            let row = sqlx::query("SELECT id, kind, title, created_at, updated_at FROM conversations WHERE direct_key = ?")
                .bind(key).fetch_one(&mut *transaction).await.map_err(conversation_error)?;
            let existing = read_conversation(&row)?;
            transaction.commit().await.map_err(conversation_error)?;
            Ok(StartDirectStoreOutcome::Existing(existing))
        }
    }

    async fn create_group(
        &self,
        conversation: &Conversation,
        owner: UserId,
        participants: &[UserId],
    ) -> Result<(), ConversationStoreError> {
        if conversation.kind() != ConversationKind::Group {
            return Err(ConversationStoreError::InvalidConversationKind {
                expected: ConversationKind::Group,
                actual: conversation.kind(),
            });
        }
        let title =
            conversation
                .title()
                .ok_or(ConversationStoreError::InvalidConversationKind {
                    expected: ConversationKind::Group,
                    actual: ConversationKind::Direct,
                })?;
        let mut transaction = self.pool.begin().await.map_err(conversation_error)?;
        sqlx::query("INSERT INTO conversations (id, kind, title, direct_key, created_at, updated_at) VALUES (?, 'group', ?, NULL, ?, ?)")
            .bind(id_bytes(conversation.id())).bind(title.as_str())
            .bind(to_micros(conversation.created_at()).map_err(unexpected_conversation)?)
            .bind(to_micros(conversation.updated_at()).map_err(unexpected_conversation)?)
            .execute(&mut *transaction).await.map_err(conversation_error)?;
        sqlx::query("INSERT INTO conversation_members (conversation_id, user_id, role, joined_at) VALUES (?, ?, 'owner', ?)")
            .bind(id_bytes(conversation.id())).bind(id_bytes(owner)).bind(to_micros(conversation.created_at()).map_err(unexpected_conversation)?)
            .execute(&mut *transaction).await.map_err(conversation_error)?;
        for user_id in participants {
            if *user_id != owner {
                sqlx::query("INSERT INTO conversation_members (conversation_id, user_id, role, joined_at) VALUES (?, ?, 'member', ?) ON CONFLICT(conversation_id, user_id) DO NOTHING")
                    .bind(id_bytes(conversation.id())).bind(id_bytes(*user_id)).bind(to_micros(conversation.created_at()).map_err(unexpected_conversation)?)
                    .execute(&mut *transaction).await.map_err(conversation_error)?;
            }
        }
        transaction.commit().await.map_err(conversation_error)
    }

    async fn find_conversation(
        &self,
        id: ConversationId,
    ) -> Result<Option<Conversation>, ConversationStoreError> {
        sqlx::query(
            "SELECT id, kind, title, created_at, updated_at FROM conversations WHERE id = ?",
        )
        .bind(id_bytes(id))
        .fetch_optional(&self.pool)
        .await
        .map_err(conversation_error)?
        .map(|row| read_conversation(&row))
        .transpose()
    }
    async fn find_member(
        &self,
        conversation_id: ConversationId,
        user_id: UserId,
    ) -> Result<Option<ConversationMember>, ConversationStoreError> {
        sqlx::query("SELECT m.conversation_id, m.user_id, m.role, m.joined_at, m.last_read_message_id, c.kind FROM conversation_members m JOIN conversations c ON c.id = m.conversation_id WHERE m.conversation_id = ? AND m.user_id = ?")
            .bind(id_bytes(conversation_id)).bind(id_bytes(user_id)).fetch_optional(&self.pool).await.map_err(conversation_error)?
            .map(|row| {
                let member = read_member(&row)?;
                let kind: String = row.try_get("kind").map_err(conversation_error)?;
                match (kind.as_str(), member.role()) {
                    ("direct", MembershipRole::Direct) | ("group", MembershipRole::Group(_)) => Ok(member),
                    _ => Err(unexpected_conversation("incompatible persisted conversation membership role")),
                }
            }).transpose()
    }
    async fn add_group_member(
        &self,
        id: ConversationId,
        actor: UserId,
        member: UserId,
        now: OffsetDateTime,
    ) -> Result<AddMemberStoreOutcome, ConversationStoreError> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(conversation_error)?;
        let can_manage: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations c JOIN conversation_members m ON m.conversation_id = c.id WHERE c.id = ? AND c.kind = 'group' AND m.user_id = ? AND m.role IN ('owner', 'admin')").bind(id_bytes(id)).bind(id_bytes(actor)).fetch_one(&mut *transaction).await.map_err(conversation_error)?;
        if can_manage == 0 {
            transaction.rollback().await.map_err(conversation_error)?;
            return Ok(AddMemberStoreOutcome::NotAuthorized);
        }
        let inserted = sqlx::query("INSERT INTO conversation_members (conversation_id, user_id, role, joined_at, last_read_message_id) VALUES (?, ?, 'member', ?, (SELECT MAX(id) FROM messages WHERE conversation_id = ?)) ON CONFLICT(conversation_id, user_id) DO NOTHING").bind(id_bytes(id)).bind(id_bytes(member)).bind(to_micros(now).map_err(unexpected_conversation)?).bind(id_bytes(id)).execute(&mut *transaction).await.map_err(conversation_error)?;
        if inserted.rows_affected() == 1 {
            sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
                .bind(to_micros(now).map_err(unexpected_conversation)?)
                .bind(id_bytes(id))
                .execute(&mut *transaction)
                .await
                .map_err(conversation_error)?;
        }
        transaction.commit().await.map_err(conversation_error)?;
        Ok(if inserted.rows_affected() == 1 {
            AddMemberStoreOutcome::Added
        } else {
            AddMemberStoreOutcome::AlreadyMember
        })
    }
    async fn remove_group_member(
        &self,
        id: ConversationId,
        actor: UserId,
        member: UserId,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let mut tx = self.pool.begin().await.map_err(conversation_error)?;
        let authorized: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations c JOIN conversation_members m ON m.conversation_id = c.id WHERE c.id = ? AND c.kind = 'group' AND m.user_id = ? AND m.role IN ('owner', 'admin')").bind(id_bytes(id)).bind(id_bytes(actor)).fetch_one(&mut *tx).await.map_err(conversation_error)?;
        if authorized == 0 {
            tx.rollback().await.map_err(conversation_error)?;
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let result = sqlx::query("DELETE FROM conversation_members WHERE conversation_id = ? AND user_id = ? AND role IN ('admin', 'member') AND EXISTS (SELECT 1 FROM conversation_members a WHERE a.conversation_id = ? AND a.user_id = ? AND a.role IN ('owner', 'admin'))").bind(id_bytes(id)).bind(id_bytes(member)).bind(id_bytes(id)).bind(id_bytes(actor)).execute(&mut *tx).await.map_err(conversation_error)?;
        if result.rows_affected() == 1 {
            sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ? AND kind = 'group'")
                .bind(to_micros(now).map_err(unexpected_conversation)?)
                .bind(id_bytes(id))
                .execute(&mut *tx)
                .await
                .map_err(conversation_error)?;
        }
        tx.commit().await.map_err(conversation_error)?;
        Ok(if result.rows_affected() == 1 {
            ChangeStoreOutcome::Changed
        } else {
            ChangeStoreOutcome::Unchanged
        })
    }
    async fn set_group_member_role(
        &self,
        id: ConversationId,
        actor: UserId,
        member: UserId,
        role: GroupRole,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let value = match role {
            GroupRole::Admin => "admin",
            GroupRole::Member => "member",
            GroupRole::Owner => return Err(ConversationStoreError::InvalidGroupRole),
        };
        let mut tx = self.pool.begin().await.map_err(conversation_error)?;
        let authorized: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations c JOIN conversation_members m ON m.conversation_id = c.id WHERE c.id = ? AND c.kind = 'group' AND m.user_id = ? AND m.role IN ('owner', 'admin')").bind(id_bytes(id)).bind(id_bytes(actor)).fetch_one(&mut *tx).await.map_err(conversation_error)?;
        if authorized == 0 {
            tx.rollback().await.map_err(conversation_error)?;
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let result = sqlx::query("UPDATE conversation_members SET role = ? WHERE conversation_id = ? AND user_id = ? AND role IN ('admin', 'member') AND role <> ? AND EXISTS (SELECT 1 FROM conversation_members a WHERE a.conversation_id = ? AND a.user_id = ? AND a.role IN ('owner', 'admin'))").bind(value).bind(id_bytes(id)).bind(id_bytes(member)).bind(value).bind(id_bytes(id)).bind(id_bytes(actor)).execute(&mut *tx).await.map_err(conversation_error)?;
        if result.rows_affected() == 1 {
            sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
                .bind(to_micros(now).map_err(unexpected_conversation)?)
                .bind(id_bytes(id))
                .execute(&mut *tx)
                .await
                .map_err(conversation_error)?;
        }
        tx.commit().await.map_err(conversation_error)?;
        Ok(if result.rows_affected() == 1 {
            ChangeStoreOutcome::Changed
        } else {
            ChangeStoreOutcome::Unchanged
        })
    }
    async fn transfer_group_ownership(
        &self,
        id: ConversationId,
        actor: UserId,
        target: UserId,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        if actor == target {
            return Ok(ChangeStoreOutcome::Unchanged);
        }
        let mut tx = self.pool.begin().await.map_err(conversation_error)?;
        let authorized: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations c JOIN conversation_members m ON m.conversation_id = c.id WHERE c.id = ? AND c.kind = 'group' AND m.user_id = ? AND m.role = 'owner'").bind(id_bytes(id)).bind(id_bytes(actor)).fetch_one(&mut *tx).await.map_err(conversation_error)?;
        if authorized == 0 {
            tx.rollback().await.map_err(conversation_error)?;
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let demoted = sqlx::query("UPDATE conversation_members SET role = 'admin' WHERE conversation_id = ? AND user_id = ? AND role = 'owner'").bind(id_bytes(id)).bind(id_bytes(actor)).execute(&mut *tx).await.map_err(conversation_error)?;
        if demoted.rows_affected() == 0 {
            tx.rollback().await.map_err(conversation_error)?;
            return Ok(ChangeStoreOutcome::Unchanged);
        }
        let promoted = sqlx::query("UPDATE conversation_members SET role = 'owner' WHERE conversation_id = ? AND user_id = ? AND role IN ('admin', 'member')").bind(id_bytes(id)).bind(id_bytes(target)).execute(&mut *tx).await.map_err(conversation_error)?;
        if promoted.rows_affected() == 0 {
            tx.rollback().await.map_err(conversation_error)?;
            return Ok(ChangeStoreOutcome::Unchanged);
        }
        sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ? AND kind = 'group'")
            .bind(to_micros(now).map_err(unexpected_conversation)?)
            .bind(id_bytes(id))
            .execute(&mut *tx)
            .await
            .map_err(conversation_error)?;
        tx.commit().await.map_err(conversation_error)?;
        Ok(ChangeStoreOutcome::Changed)
    }
    async fn leave_group(
        &self,
        id: ConversationId,
        actor: UserId,
        now: OffsetDateTime,
    ) -> Result<LeaveStoreOutcome, ConversationStoreError> {
        let mut tx = self.pool.begin().await.map_err(conversation_error)?;
        let kind: Option<String> =
            sqlx::query_scalar("SELECT kind FROM conversations WHERE id = ?")
                .bind(id_bytes(id))
                .fetch_optional(&mut *tx)
                .await
                .map_err(conversation_error)?;
        if kind.as_deref() == Some("direct") {
            tx.rollback().await.map_err(conversation_error)?;
            return Ok(LeaveStoreOutcome::Direct);
        }
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM conversation_members WHERE conversation_id = ? AND user_id = ?",
        )
        .bind(id_bytes(id))
        .bind(id_bytes(actor))
        .fetch_optional(&mut *tx)
        .await
        .map_err(conversation_error)?;
        let Some(role) = role else {
            tx.rollback().await.map_err(conversation_error)?;
            return Ok(LeaveStoreOutcome::NotMember);
        };
        if role == "owner" {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM conversation_members WHERE conversation_id = ?",
            )
            .bind(id_bytes(id))
            .fetch_one(&mut *tx)
            .await
            .map_err(conversation_error)?;
            if count != 1 {
                tx.rollback().await.map_err(conversation_error)?;
                return Ok(LeaveStoreOutcome::OwnershipTransferRequired);
            }
            sqlx::query("DELETE FROM conversations WHERE id = ?")
                .bind(id_bytes(id))
                .execute(&mut *tx)
                .await
                .map_err(conversation_error)?;
            tx.commit().await.map_err(conversation_error)?;
            return Ok(LeaveStoreOutcome::ConversationDeleted);
        }
        sqlx::query("DELETE FROM conversation_members WHERE conversation_id = ? AND user_id = ?")
            .bind(id_bytes(id))
            .bind(id_bytes(actor))
            .execute(&mut *tx)
            .await
            .map_err(conversation_error)?;
        sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
            .bind(to_micros(now).map_err(unexpected_conversation)?)
            .bind(id_bytes(id))
            .execute(&mut *tx)
            .await
            .map_err(conversation_error)?;
        tx.commit().await.map_err(conversation_error)?;
        Ok(LeaveStoreOutcome::Left)
    }
    async fn rename_group(
        &self,
        id: ConversationId,
        actor: UserId,
        title: &GroupTitle,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let result = sqlx::query("UPDATE conversations SET title = ?, updated_at = ? WHERE id = ? AND kind = 'group' AND title <> ? AND EXISTS (SELECT 1 FROM conversation_members m WHERE m.conversation_id = conversations.id AND m.user_id = ? AND m.role IN ('owner', 'admin'))").bind(title.as_str()).bind(to_micros(now).map_err(unexpected_conversation)?).bind(id_bytes(id)).bind(title.as_str()).bind(id_bytes(actor)).execute(&self.pool).await.map_err(conversation_error)?;
        if result.rows_affected() == 1 {
            return Ok(ChangeStoreOutcome::Changed);
        }
        let authorized: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations c JOIN conversation_members m ON m.conversation_id = c.id WHERE c.id = ? AND c.kind = 'group' AND m.user_id = ? AND m.role IN ('owner', 'admin')").bind(id_bytes(id)).bind(id_bytes(actor)).fetch_one(&self.pool).await.map_err(conversation_error)?;
        Ok(if authorized == 0 {
            ChangeStoreOutcome::NotAuthorized
        } else {
            ChangeStoreOutcome::Unchanged
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{SqliteStorageOptions, sqlite_primary_code};

    #[test]
    fn sqlite_primary_code_normalizes_extended_busy_and_locked_codes() {
        assert_eq!(sqlite_primary_code("5"), Some(5));
        assert_eq!(sqlite_primary_code("517"), Some(5));
        assert_eq!(sqlite_primary_code("262"), Some(6));
        assert_eq!(sqlite_primary_code("not-a-code"), None);
    }

    #[test]
    fn options_validate_without_opening_a_database() {
        assert!(
            SqliteStorageOptions::new("sqlite:missing.db")
                .validate()
                .is_ok()
        );
        assert!(
            SqliteStorageOptions::new("sqlite://windwatcher.db?mode=invalid")
                .validate()
                .is_err()
        );
    }
}
