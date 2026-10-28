use super::MigrationStatus;
use sqlx::{
    MySqlPool,
    mysql::{MySqlConnectOptions, MySqlPoolOptions},
};
use std::{error::Error, fmt, str::FromStr, time::Duration};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/mysql");

/// Connection settings for a dedicated MySQL 8 primary-storage database.
#[derive(Clone)]
pub struct MySqlStorageOptions {
    url: String,
    max_connections: u32,
    acquire_timeout: Duration,
}
impl fmt::Debug for MySqlStorageOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MySqlStorageOptions")
            .field("url", &"[REDACTED]")
            .field("max_connections", &self.max_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .finish()
    }
}
impl MySqlStorageOptions {
    /// Creates options from a MySQL connection URL.
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
    /// Validates URL and pool settings without connecting.
    pub fn validate(&self) -> Result<(), MySqlStorageError> {
        self.connect_options().map(|_| ())
    }
    fn connect_options(&self) -> Result<MySqlConnectOptions, MySqlStorageError> {
        if self.max_connections == 0 || self.acquire_timeout.is_zero() {
            return Err(MySqlStorageError::Configuration(
                "pool limits must be positive",
            ));
        }
        MySqlConnectOptions::from_str(&self.url)
            .map_err(|_| MySqlStorageError::Configuration("invalid MySQL URL"))
    }
}
/// Failures while constructing or operating MySQL storage infrastructure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MySqlStorageError {
    /// Invalid URL or pool limits.
    #[error("invalid MySQL storage configuration: {0}")]
    Configuration(&'static str),
    /// Connection failed.
    #[error("MySQL storage connection failure")]
    Connection(#[source] Box<dyn Error + Send + Sync>),
    /// Migration application failed.
    #[error("MySQL storage migration failure")]
    Migration(#[source] Box<dyn Error + Send + Sync>),
    /// Schema inspection failed.
    #[error("MySQL storage schema inspection failure")]
    Schema(#[source] Box<dyn Error + Send + Sync>),
    /// Migration ledger is inconsistent with embedded migrations.
    #[error("invalid MySQL migration history: {0}")]
    InvalidMigrationHistory(String),
}
/// MySQL implementation of the Windwatcher primary storage ports.
#[derive(Clone)]
pub struct MySqlStorage {
    pub(super) pool: MySqlPool,
}
impl fmt::Debug for MySqlStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MySqlStorage").finish_non_exhaustive()
    }
}
impl MySqlStorage {
    /// Opens a connection pool without applying migrations.
    pub async fn connect(options: MySqlStorageOptions) -> Result<Self, MySqlStorageError> {
        let connect = options.connect_options()?;
        let pool = MySqlPoolOptions::new()
            .max_connections(options.max_connections)
            .acquire_timeout(options.acquire_timeout)
            .connect_with(connect)
            .await
            .map_err(|e| MySqlStorageError::Connection(Box::new(e)))?;
        Ok(Self { pool })
    }
    /// Applies pending embedded migrations explicitly.
    pub async fn migrate(&self) -> Result<(), MySqlStorageError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| MySqlStorageError::Migration(Box::new(e)))
    }
    /// Inspects migration status without changing the database.
    pub async fn migration_status(&self) -> Result<MigrationStatus, MySqlStorageError> {
        status(&self.pool).await
    }
    /// Checks schema status without changing the database.
    pub async fn check_schema(&self) -> Result<MigrationStatus, MySqlStorageError> {
        self.migration_status().await
    }
}
async fn status(pool: &MySqlPool) -> Result<MigrationStatus, MySqlStorageError> {
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema=DATABASE() AND table_name='_sqlx_migrations')").fetch_one(pool).await.map_err(|e|MySqlStorageError::Schema(Box::new(e)))?;
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
            .map_err(|e| MySqlStorageError::Schema(Box::new(e)))?;
    let mut current = None;
    for row in &rows {
        use sqlx::Row;
        let version: i64 = row
            .try_get("version")
            .map_err(|e| MySqlStorageError::Schema(Box::new(e)))?;
        let checksum: Vec<u8> = row
            .try_get("checksum")
            .map_err(|e| MySqlStorageError::Schema(Box::new(e)))?;
        let success: bool = row
            .try_get("success")
            .map_err(|e| MySqlStorageError::Schema(Box::new(e)))?;
        let Some(m) = MIGRATOR.iter().find(|m| m.version == version) else {
            return Err(MySqlStorageError::InvalidMigrationHistory(format!(
                "unknown migration version {version}"
            )));
        };
        if !success {
            return Err(MySqlStorageError::InvalidMigrationHistory(format!(
                "migration {version} is dirty"
            )));
        }
        if checksum != m.checksum.as_ref() {
            return Err(MySqlStorageError::InvalidMigrationHistory(format!(
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
