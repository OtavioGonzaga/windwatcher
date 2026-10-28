use std::{error::Error, fmt, str::FromStr, time::Duration};

use super::MigrationStatus;
use crate::{
    IdentityStore, IdentityStoreError, LocalAuthentication, PasswordHash, RefreshRotationResult,
    RefreshTokenHash, Session, SessionId, SessionStore, SessionStoreError, User, UserId, Username,
};
use sqlx::{
    PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use time::OffsetDateTime;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
mod conversations;
mod messages;
mod queries;

/// Connection settings for a dedicated PostgreSQL primary-storage database.
#[derive(Clone)]
pub struct PostgresStorageOptions {
    url: String,
    max_connections: u32,
    acquire_timeout: Duration,
}
impl fmt::Debug for PostgresStorageOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresStorageOptions")
            .field("url", &"[REDACTED]")
            .field("max_connections", &self.max_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .finish()
    }
}
impl PostgresStorageOptions {
    /// Creates options from a PostgreSQL connection URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: 8,
            acquire_timeout: Duration::from_secs(5),
        }
    }
    /// Sets the maximum number of pooled connections.
    #[must_use]
    pub fn max_connections(mut self, value: u32) -> Self {
        self.max_connections = value;
        self
    }
    /// Sets the maximum wait for a pooled connection.
    #[must_use]
    pub fn acquire_timeout(mut self, value: Duration) -> Self {
        self.acquire_timeout = value;
        self
    }
    /// Validates the URL and pool settings without connecting.
    pub fn validate(&self) -> Result<(), PostgresStorageError> {
        self.connect_options().map(|_| ())
    }
    fn connect_options(&self) -> Result<PgConnectOptions, PostgresStorageError> {
        if self.max_connections == 0 || self.acquire_timeout.is_zero() {
            return Err(PostgresStorageError::Configuration(
                "pool limits must be positive",
            ));
        }
        PgConnectOptions::from_str(&self.url)
            .map_err(|_| PostgresStorageError::Configuration("invalid PostgreSQL URL"))
    }
}

/// Failures while constructing or operating PostgreSQL storage infrastructure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PostgresStorageError {
    /// Invalid URL or pool limits.
    #[error("invalid PostgreSQL storage configuration: {0}")]
    Configuration(&'static str),
    /// Connection failed.
    #[error("PostgreSQL storage connection failure")]
    Connection(#[source] Box<dyn Error + Send + Sync>),
    /// Migration application failed.
    #[error("PostgreSQL storage migration failure")]
    Migration(#[source] Box<dyn Error + Send + Sync>),
    /// Schema inspection failed.
    #[error("PostgreSQL storage schema inspection failure")]
    Schema(#[source] Box<dyn Error + Send + Sync>),
    /// The migration ledger is inconsistent with embedded migrations.
    #[error("invalid PostgreSQL migration history: {0}")]
    InvalidMigrationHistory(String),
}

/// PostgreSQL implementation of the Windwatcher primary storage ports.
#[derive(Clone)]
pub struct PostgresStorage {
    pub(super) pool: PgPool,
}
impl fmt::Debug for PostgresStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresStorage").finish_non_exhaustive()
    }
}
impl PostgresStorage {
    /// Opens a connection pool without applying migrations.
    pub async fn connect(options: PostgresStorageOptions) -> Result<Self, PostgresStorageError> {
        let connect = options.connect_options()?;
        let pool = PgPoolOptions::new()
            .max_connections(options.max_connections)
            .acquire_timeout(options.acquire_timeout)
            .connect_with(connect)
            .await
            .map_err(|e| PostgresStorageError::Connection(Box::new(e)))?;
        Ok(Self { pool })
    }
    /// Applies pending embedded migrations explicitly.
    pub async fn migrate(&self) -> Result<(), PostgresStorageError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| PostgresStorageError::Migration(Box::new(e)))
    }
    /// Inspects migration status without changing the database.
    pub async fn migration_status(&self) -> Result<MigrationStatus, PostgresStorageError> {
        status(&self.pool).await
    }
    /// Checks schema status without changing the database.
    pub async fn check_schema(&self) -> Result<MigrationStatus, PostgresStorageError> {
        self.migration_status().await
    }
}

#[derive(Debug)]
struct Corrupt(&'static str);
impl fmt::Display for Corrupt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Corrupt {}
fn micros(t: OffsetDateTime) -> Result<i64, &'static str> {
    i64::try_from(t.unix_timestamp_nanos().div_euclid(1000)).map_err(|_| "timestamp out of range")
}
fn date(v: i64) -> Result<OffsetDateTime, &'static str> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(v) * 1000).map_err(|_| "invalid timestamp")
}
fn transient(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed)
        || matches!(e,sqlx::Error::Database(d) if matches!(d.code().as_deref(),Some("40001"|"40P01"|"55P03"|"08000"|"08003"|"08006")))
}
fn identity_db(e: sqlx::Error) -> IdentityStoreError {
    if transient(&e) {
        IdentityStoreError::Unavailable(Box::new(e))
    } else {
        IdentityStoreError::Unexpected(Box::new(e))
    }
}
fn session_db(e: sqlx::Error) -> SessionStoreError {
    if transient(&e) {
        SessionStoreError::Unavailable(Box::new(e))
    } else if matches!(&e,sqlx::Error::Database(d)if d.is_unique_violation()||d.is_foreign_key_violation())
    {
        SessionStoreError::Conflict
    } else {
        SessionStoreError::Unexpected(Box::new(e))
    }
}
fn bad_identity(s: &'static str) -> IdentityStoreError {
    IdentityStoreError::Unexpected(Box::new(Corrupt(s)))
}
fn bad_session(s: &'static str) -> SessionStoreError {
    SessionStoreError::Unexpected(Box::new(Corrupt(s)))
}
fn read_user(row: &sqlx::postgres::PgRow) -> Result<User, IdentityStoreError> {
    let id = UserId::try_from(row.try_get::<Uuid, _>("id").map_err(identity_db)?)
        .map_err(|_| bad_identity("invalid persisted user ID"))?;
    let username = Username::from_str(&row.try_get::<String, _>("username").map_err(identity_db)?)
        .map_err(|_| bad_identity("invalid persisted username"))?;
    let ts =
        |name: &str| -> Result<i64, IdentityStoreError> { row.try_get(name).map_err(identity_db) };
    let disabled: Option<i64> = row.try_get("disabled_at").map_err(identity_db)?;
    Ok(User::restore(
        id,
        username,
        disabled
            .map(date)
            .transpose()
            .map_err(|_| bad_identity("invalid timestamp"))?,
        date(ts("created_at")?).map_err(|_| bad_identity("invalid timestamp"))?,
        date(ts("updated_at")?).map_err(|_| bad_identity("invalid timestamp"))?,
    ))
}
fn read_session(row: &sqlx::postgres::PgRow) -> Result<Session, SessionStoreError> {
    let id = SessionId::try_from(row.try_get::<Uuid, _>("id").map_err(session_db)?)
        .map_err(|_| bad_session("invalid persisted session ID"))?;
    let owner = UserId::try_from(row.try_get::<Uuid, _>("user_id").map_err(session_db)?)
        .map_err(|_| bad_session("invalid persisted user ID"))?;
    let created: i64 = row.try_get("created_at").map_err(session_db)?;
    let expires: i64 = row.try_get("expires_at").map_err(session_db)?;
    let revoked: Option<i64> = row.try_get("revoked_at").map_err(session_db)?;
    Session::restore(
        id,
        owner,
        date(created).map_err(|_| bad_session("invalid timestamp"))?,
        date(expires).map_err(|_| bad_session("invalid timestamp"))?,
        revoked
            .map(date)
            .transpose()
            .map_err(|_| bad_session("invalid timestamp"))?,
    )
    .map_err(|_| bad_session("invalid persisted session"))
}
impl IdentityStore for PostgresStorage {
    async fn create_local_user(
        &self,
        u: &User,
        p: &PasswordHash,
    ) -> Result<(), IdentityStoreError> {
        let mut tx = self.pool.begin().await.map_err(identity_db)?;
        let r=sqlx::query("INSERT INTO users(id,username,disabled_at,created_at,updated_at) VALUES($1,$2,$3,$4,$5) ON CONFLICT(username) DO NOTHING").bind(u.id().as_uuid()).bind(u.username().as_str()).bind(u.disabled_at().map(micros).transpose().map_err(|_|bad_identity("timestamp out of range"))?).bind(micros(u.created_at()).map_err(|_|bad_identity("timestamp out of range"))?).bind(micros(u.updated_at()).map_err(|_|bad_identity("timestamp out of range"))?).execute(&mut *tx).await.map_err(identity_db)?;
        if r.rows_affected() == 0 {
            return Err(IdentityStoreError::UsernameConflict);
        }
        sqlx::query("INSERT INTO local_credentials(user_id,password_hash) VALUES($1,$2)")
            .bind(u.id().as_uuid())
            .bind(p.as_encoded())
            .execute(&mut *tx)
            .await
            .map_err(identity_db)?;
        tx.commit().await.map_err(identity_db)
    }
    async fn find_local_auth_by_username(
        &self,
        n: &Username,
    ) -> Result<Option<LocalAuthentication>, IdentityStoreError> {
        let row=sqlx::query("SELECT u.id,u.username,u.disabled_at,u.created_at,u.updated_at,c.password_hash FROM users u JOIN local_credentials c ON c.user_id=u.id WHERE u.username=$1").bind(n.as_str()).fetch_optional(&self.pool).await.map_err(identity_db)?;
        row.map(|r| {
            let u = read_user(&r)?;
            let encoded: String = r.try_get("password_hash").map_err(identity_db)?;
            let hash = PasswordHash::parse(&encoded)
                .map_err(|_| bad_identity("invalid persisted password hash"))?;
            Ok(LocalAuthentication::new(u, hash))
        })
        .transpose()
    }
    async fn find_user(&self, id: UserId) -> Result<Option<User>, IdentityStoreError> {
        sqlx::query("SELECT id,username,disabled_at,created_at,updated_at FROM users WHERE id=$1")
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(identity_db)?
            .map(|r| read_user(&r))
            .transpose()
    }
    async fn update_user(&self, u: &User) -> Result<(), IdentityStoreError> {
        let r = sqlx::query("UPDATE users SET disabled_at=$1,updated_at=$2 WHERE id=$3")
            .bind(
                u.disabled_at()
                    .map(micros)
                    .transpose()
                    .map_err(|_| bad_identity("timestamp out of range"))?,
            )
            .bind(micros(u.updated_at()).map_err(|_| bad_identity("timestamp out of range"))?)
            .bind(u.id().as_uuid())
            .execute(&self.pool)
            .await
            .map_err(identity_db)?;
        if r.rows_affected() == 0 {
            Err(IdentityStoreError::NotFound)
        } else {
            Ok(())
        }
    }
}
impl SessionStore for PostgresStorage {
    async fn find_session(&self, id: SessionId) -> Result<Option<Session>, SessionStoreError> {
        sqlx::query("SELECT id,user_id,created_at,expires_at,revoked_at FROM sessions WHERE id=$1")
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(session_db)?
            .map(|r| read_session(&r))
            .transpose()
    }
    async fn create_session(
        &self,
        s: &Session,
        h: &RefreshTokenHash,
    ) -> Result<(), SessionStoreError> {
        let mut tx = self.pool.begin().await.map_err(session_db)?;
        let row = sqlx::query("SELECT disabled_at FROM users WHERE id=$1 FOR UPDATE")
            .bind(s.user_id().as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(session_db)?
            .ok_or(SessionStoreError::OwnerNotFound)?;
        if row
            .try_get::<Option<i64>, _>("disabled_at")
            .map_err(session_db)?
            .is_some()
        {
            return Err(SessionStoreError::OwnerDisabled);
        }
        sqlx::query("INSERT INTO sessions(id,user_id,refresh_token_hash,created_at,expires_at,revoked_at) VALUES($1,$2,$3,$4,$5,$6)").bind(s.id().as_uuid()).bind(s.user_id().as_uuid()).bind(h.as_bytes().as_slice()).bind(micros(s.created_at()).map_err(|_|bad_session("timestamp out of range"))?).bind(micros(s.expires_at()).map_err(|_|bad_session("timestamp out of range"))?).bind(s.revoked_at().map(micros).transpose().map_err(|_|bad_session("timestamp out of range"))?).execute(&mut *tx).await.map_err(session_db)?;
        tx.commit().await.map_err(session_db)
    }
    async fn find_by_refresh_token_hash(
        &self,
        h: &RefreshTokenHash,
    ) -> Result<Option<Session>, SessionStoreError> {
        sqlx::query("SELECT id,user_id,created_at,expires_at,revoked_at FROM sessions WHERE refresh_token_hash=$1").bind(h.as_bytes().as_slice()).fetch_optional(&self.pool).await.map_err(session_db)?.map(|r|read_session(&r)).transpose()
    }
    async fn rotate_refresh_token(
        &self,
        id: SessionId,
        old: &RefreshTokenHash,
        new: &RefreshTokenHash,
        now: OffsetDateTime,
    ) -> Result<RefreshRotationResult, SessionStoreError> {
        let n = micros(now).map_err(|_| bad_session("timestamp out of range"))?;
        let r=sqlx::query("UPDATE sessions SET refresh_token_hash=$1 WHERE id=$2 AND refresh_token_hash=$3 AND revoked_at IS NULL AND expires_at>$4").bind(new.as_bytes().as_slice()).bind(id.as_uuid()).bind(old.as_bytes().as_slice()).bind(n).execute(&self.pool).await.map_err(session_db)?;
        Ok(if r.rows_affected() == 1 {
            RefreshRotationResult::Rotated
        } else {
            RefreshRotationResult::Rejected
        })
    }
    async fn revoke_session_for_user(
        &self,
        u: UserId,
        s: SessionId,
        t: OffsetDateTime,
    ) -> Result<bool, SessionStoreError> {
        let r = sqlx::query(
            "UPDATE sessions SET revoked_at=COALESCE(revoked_at,$1) WHERE id=$2 AND user_id=$3",
        )
        .bind(micros(t).map_err(|_| bad_session("timestamp out of range"))?)
        .bind(s.as_uuid())
        .bind(u.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(session_db)?;
        Ok(r.rows_affected() != 0)
    }
    async fn revoke_all_for_user(
        &self,
        u: UserId,
        t: OffsetDateTime,
    ) -> Result<(), SessionStoreError> {
        sqlx::query("UPDATE sessions SET revoked_at=$1 WHERE user_id=$2 AND revoked_at IS NULL")
            .bind(micros(t).map_err(|_| bad_session("timestamp out of range"))?)
            .bind(u.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(session_db)?;
        Ok(())
    }
}

async fn status(pool: &PgPool) -> Result<MigrationStatus, PostgresStorageError> {
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NOT NULL")
            .fetch_one(pool)
            .await
            .map_err(|e| PostgresStorageError::Schema(Box::new(e)))?;
    let latest = MIGRATOR.iter().map(|m| m.version).max();
    if !exists {
        return Ok(MigrationStatus {
            current_version: None,
            latest_version: latest,
            pending: MIGRATOR.iter().count(),
        });
    }
    let rows =
        sqlx::query("SELECT version,checksum,success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .map_err(|e| PostgresStorageError::Schema(Box::new(e)))?;
    let mut current = None;
    for row in &rows {
        use sqlx::Row;
        let version: i64 = row
            .try_get("version")
            .map_err(|e| PostgresStorageError::Schema(Box::new(e)))?;
        let checksum: Vec<u8> = row
            .try_get("checksum")
            .map_err(|e| PostgresStorageError::Schema(Box::new(e)))?;
        let success: bool = row
            .try_get("success")
            .map_err(|e| PostgresStorageError::Schema(Box::new(e)))?;
        let Some(migration) = MIGRATOR.iter().find(|m| m.version == version) else {
            return Err(PostgresStorageError::InvalidMigrationHistory(format!(
                "unknown migration version {version}"
            )));
        };
        if !success {
            return Err(PostgresStorageError::InvalidMigrationHistory(format!(
                "migration {version} is dirty"
            )));
        }
        if checksum != migration.checksum.as_ref() {
            return Err(PostgresStorageError::InvalidMigrationHistory(format!(
                "migration {version} checksum differs"
            )));
        }
        current = Some(version);
    }
    Ok(MigrationStatus {
        current_version: current,
        latest_version: latest,
        pending: MIGRATOR.iter().count().saturating_sub(rows.len()),
    })
}
