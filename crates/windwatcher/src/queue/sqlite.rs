use std::{error::Error, str::FromStr, time::Duration};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow},
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    ClaimedJob, Job, JobFailure, JobFailureCode, JobId, JobKind, JobLease, JobPayload, JobStatus,
    LeaseId, LeaseMutationOutcome, QueueListOptions, QueueMaintenanceReport, QueueStore,
    QueueStoreError, RetryFailedOutcome, RetryOutcome, WorkerId,
};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/queue/sqlite");

/// Options for a SQLite queue connection.
#[derive(Clone, Debug)]
pub struct SqliteQueueOptions {
    url: String,
    create_if_missing: bool,
    max_connections: u32,
    busy_timeout: Duration,
}
impl SqliteQueueOptions {
    /// Creates queue options for a SQLite URL or path.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            create_if_missing: true,
            max_connections: 8,
            busy_timeout: Duration::from_secs(5),
        }
    }
    /// Controls creation of missing file-backed databases.
    pub fn create_if_missing(mut self, value: bool) -> Self {
        self.create_if_missing = value;
        self
    }
    /// Sets the connection pool limit.
    pub fn max_connections(mut self, value: u32) -> Self {
        self.max_connections = value;
        self
    }
    /// Sets SQLite busy timeout.
    pub fn busy_timeout(mut self, value: Duration) -> Self {
        self.busy_timeout = value;
        self
    }
}

/// SQLite queue infrastructure errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SqliteQueueError {
    /// Invalid database URL or options.
    #[error("invalid SQLite queue configuration: {0}")]
    Configuration(String),
    /// Connection or query failure.
    #[error("SQLite queue operation failed")]
    Database(#[source] Box<dyn Error + Send + Sync>),
    /// Migration lifecycle failed.
    #[error("SQLite queue migration failed")]
    Migration(#[source] Box<dyn Error + Send + Sync>),
    /// Migration history differs from embedded schema.
    #[error("invalid SQLite queue migration history: {0}")]
    InvalidMigrationHistory(String),
    /// Persisted queue data is malformed.
    #[error("invalid persisted SQLite queue job: {0}")]
    Corrupt(&'static str),
}
impl From<SqliteQueueError> for QueueStoreError {
    fn from(value: SqliteQueueError) -> Self {
        Self::new(value)
    }
}

/// Queue migration status, independent of primary-storage migrations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueMigrationStatus {
    current: Option<i64>,
    latest: Option<i64>,
    pending: usize,
}
impl QueueMigrationStatus {
    /// Returns highest applied migration version.
    pub fn current_version(&self) -> Option<i64> {
        self.current
    }
    /// Returns newest embedded migration version.
    pub fn latest_version(&self) -> Option<i64> {
        self.latest
    }
    /// Returns number of pending migrations.
    pub fn pending(&self) -> usize {
        self.pending
    }
    /// Returns whether schema matches embedded migrations.
    pub fn is_current(&self) -> bool {
        self.pending == 0 && self.current == self.latest
    }
}

/// SQLx-backed SQLite implementation of [`QueueStore`].
#[derive(Clone, Debug)]
pub struct SqliteQueue {
    pool: SqlitePool,
}
impl SqliteQueue {
    /// Opens a queue pool without applying migrations.
    pub async fn connect(options: SqliteQueueOptions) -> Result<Self, SqliteQueueError> {
        if options.max_connections == 0 || options.busy_timeout.is_zero() {
            return Err(SqliteQueueError::Configuration(
                "pool size and busy timeout must be positive".into(),
            ));
        }
        let memory = options.url == ":memory:"
            || options.url == "sqlite::memory:"
            || options.url.contains("mode=memory");
        let connect = if options.url.starts_with("sqlite:") || options.url.starts_with("file:") {
            SqliteConnectOptions::from_str(&options.url)
                .map_err(|e| SqliteQueueError::Configuration(e.to_string()))?
        } else if options.url == ":memory:" {
            SqliteConnectOptions::new().in_memory(true)
        } else {
            SqliteConnectOptions::new().filename(&options.url)
        }
        .create_if_missing(options.create_if_missing)
        .foreign_keys(true)
        .busy_timeout(options.busy_timeout);
        let connect = if memory {
            connect
        } else {
            connect.journal_mode(SqliteJournalMode::Wal)
        };
        let pool = SqlitePoolOptions::new()
            .max_connections(if memory { 1 } else { options.max_connections })
            .connect_with(connect)
            .await
            .map_err(db_error)?;
        Ok(Self { pool })
    }
    /// Explicitly applies embedded queue migrations.
    pub async fn migrate(&self) -> Result<(), SqliteQueueError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| SqliteQueueError::Migration(Box::new(e)))
    }
    /// Inspects migration history without changing the database.
    pub async fn migration_status(&self) -> Result<QueueMigrationStatus, SqliteQueueError> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db_error)?;
        let latest = MIGRATOR.iter().map(|m| m.version).max();
        if exists == 0 {
            return Ok(QueueMigrationStatus {
                current: None,
                latest,
                pending: MIGRATOR.iter().count(),
            });
        }
        let rows =
            sqlx::query("SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&self.pool)
                .await
                .map_err(db_error)?;
        let mut current = None;
        for row in &rows {
            let version: i64 = row.try_get("version").map_err(db_error)?;
            let checksum: Vec<u8> = row.try_get("checksum").map_err(db_error)?;
            let success: bool = row.try_get("success").map_err(db_error)?;
            let Some(migration) = MIGRATOR.iter().find(|m| m.version == version) else {
                return Err(SqliteQueueError::InvalidMigrationHistory(format!(
                    "unknown migration {version}"
                )));
            };
            if !success || checksum != migration.checksum.as_ref() {
                return Err(SqliteQueueError::InvalidMigrationHistory(format!(
                    "migration {version} is dirty or checksum differs"
                )));
            }
            current = Some(version);
        }
        Ok(QueueMigrationStatus {
            current,
            latest,
            pending: MIGRATOR.iter().count().saturating_sub(rows.len()),
        })
    }
    /// Verifies that queue schema is current without applying migrations.
    pub async fn check_schema(&self) -> Result<QueueMigrationStatus, SqliteQueueError> {
        self.migration_status().await
    }
}

fn db_error(error: impl Error + Send + Sync + 'static) -> SqliteQueueError {
    SqliteQueueError::Database(Box::new(error))
}
fn micros(value: OffsetDateTime) -> Result<i64, SqliteQueueError> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1_000))
        .map_err(|_| SqliteQueueError::Corrupt("timestamp out of range"))
}
fn datetime(value: i64) -> Result<OffsetDateTime, SqliteQueueError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1000)
        .map_err(|_| SqliteQueueError::Corrupt("invalid timestamp"))
}
fn uuid_bytes<T: Into<Uuid>>(id: T) -> Vec<u8> {
    id.into().into_bytes().to_vec()
}
fn read_id<T: TryFrom<Uuid>>(bytes: Vec<u8>) -> Result<T, SqliteQueueError> {
    let uuid =
        Uuid::from_slice(&bytes).map_err(|_| SqliteQueueError::Corrupt("invalid UUID bytes"))?;
    T::try_from(uuid).map_err(|_| SqliteQueueError::Corrupt("invalid UUID version"))
}
fn status(value: JobStatus) -> &'static str {
    match value {
        JobStatus::Pending => "pending",
        JobStatus::Leased => "leased",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
    }
}
fn read_status(value: &str) -> Result<JobStatus, SqliteQueueError> {
    match value {
        "pending" => Ok(JobStatus::Pending),
        "leased" => Ok(JobStatus::Leased),
        "completed" => Ok(JobStatus::Completed),
        "failed" => Ok(JobStatus::Failed),
        _ => Err(SqliteQueueError::Corrupt("unknown status")),
    }
}
fn read_job(row: &SqliteRow) -> Result<Job, SqliteQueueError> {
    let id = read_id(row.try_get("id").map_err(db_error)?)?;
    let kind = JobKind::new(row.try_get::<String, _>("kind").map_err(db_error)?)
        .map_err(|_| SqliteQueueError::Corrupt("invalid kind"))?;
    let version: i64 = row.try_get("payload_version").map_err(db_error)?;
    let version =
        u16::try_from(version).map_err(|_| SqliteQueueError::Corrupt("invalid payload version"))?;
    let payload_json: String = row.try_get("payload_json").map_err(db_error)?;
    let decoded: serde_json::Value = serde_json::from_str(&payload_json)
        .map_err(|_| SqliteQueueError::Corrupt("invalid payload JSON"))?;
    let payload = JobPayload::try_from(decoded)
        .map_err(|_| SqliteQueueError::Corrupt("payload root is not object"))?;
    let state = read_status(&row.try_get::<String, _>("status").map_err(db_error)?)?;
    let attempts = u32::try_from(row.try_get::<i64, _>("attempts").map_err(db_error)?)
        .map_err(|_| SqliteQueueError::Corrupt("invalid attempts"))?;
    let max_attempts = u32::try_from(row.try_get::<i64, _>("max_attempts").map_err(db_error)?)
        .map_err(|_| SqliteQueueError::Corrupt("invalid max attempts"))?;
    let lease_id: Option<Vec<u8>> = row.try_get("lease_id").map_err(db_error)?;
    let lease_owner: Option<Vec<u8>> = row.try_get("lease_owner").map_err(db_error)?;
    let lease_expiry: Option<i64> = row.try_get("lease_expires_at").map_err(db_error)?;
    let lease = match (lease_id, lease_owner, lease_expiry) {
        (Some(id), Some(worker), Some(expires)) => Some(JobLease::new(
            read_id::<LeaseId>(id)?,
            read_id::<WorkerId>(worker)?,
            datetime(expires)?,
        )),
        (None, None, None) => None,
        _ => return Err(SqliteQueueError::Corrupt("incoherent lease fields")),
    };
    let code: Option<String> = row.try_get("last_error_code").map_err(db_error)?;
    let message: Option<String> = row.try_get("last_error_message").map_err(db_error)?;
    let last_failure = match (code, message) {
        (Some(c), Some(m)) => Some(
            JobFailure::new(
                JobFailureCode::new(c)
                    .map_err(|_| SqliteQueueError::Corrupt("invalid failure code"))?,
                m,
            )
            .map_err(|_| SqliteQueueError::Corrupt("invalid failure message"))?,
        ),
        (None, None) => None,
        _ => return Err(SqliteQueueError::Corrupt("incoherent failure fields")),
    };
    let optional_time = |name: &str| -> Result<Option<OffsetDateTime>, SqliteQueueError> {
        row.try_get::<Option<i64>, _>(name)
            .map_err(db_error)?
            .map(datetime)
            .transpose()
    };
    Job::restore(
        id,
        kind,
        version,
        payload,
        state,
        datetime(row.try_get("available_at").map_err(db_error)?)?,
        attempts,
        max_attempts,
        lease,
        last_failure,
        datetime(row.try_get("created_at").map_err(db_error)?)?,
        optional_time("completed_at")?,
        optional_time("failed_at")?,
    )
    .map_err(|_| SqliteQueueError::Corrupt("inconsistent job state"))
}
fn queue_error(error: SqliteQueueError) -> QueueStoreError {
    QueueStoreError::new(error)
}

impl QueueStore for SqliteQueue {
    async fn enqueue(&self, job: &Job) -> Result<(), QueueStoreError> {
        let payload = serde_json::to_string(job.payload()).map_err(|e| queue_error(db_error(e)))?;
        sqlx::query("INSERT INTO queue_jobs (id,kind,payload_version,payload_json,status,available_at,attempts,max_attempts,created_at) VALUES (?,?,?,?,?,?,?,?,?)")
            .bind(uuid_bytes(job.id())).bind(job.kind().as_str()).bind(i64::from(job.payload_version())).bind(payload).bind(status(job.status())).bind(micros(job.available_at()).map_err(queue_error)?).bind(i64::from(job.attempts())).bind(i64::from(job.max_attempts())).bind(micros(job.created_at()).map_err(queue_error)?)
            .execute(&self.pool).await.map_err(|e| queue_error(db_error(e)))?;
        Ok(())
    }
    async fn claim_next(
        &self,
        worker: WorkerId,
        lease: LeaseId,
        duration: Duration,
        now: OffsetDateTime,
    ) -> Result<Option<ClaimedJob>, QueueStoreError> {
        if duration.is_zero() {
            return Err(queue_error(SqliteQueueError::Configuration(
                "lease duration must be positive".into(),
            )));
        }
        let now_us = micros(now).map_err(queue_error)?;
        let lease_us = i64::try_from(duration.as_micros())
            .ok()
            .and_then(|d| now_us.checked_add(d))
            .ok_or_else(|| queue_error(SqliteQueueError::Corrupt("lease expiry out of range")))?;
        sqlx::query("UPDATE queue_jobs SET status='failed', failed_at=?, lease_id=NULL, lease_owner=NULL, lease_expires_at=NULL, last_error_code='queue.attempts_exhausted', last_error_message='maximum attempts exhausted' WHERE status='leased' AND lease_expires_at<=? AND attempts>=max_attempts")
            .bind(now_us).bind(now_us).execute(&self.pool).await.map_err(|e| queue_error(db_error(e)))?;
        let row = sqlx::query("UPDATE queue_jobs SET status='leased', attempts=attempts+1, lease_id=?, lease_owner=?, lease_expires_at=? WHERE id=(SELECT id FROM queue_jobs WHERE attempts<max_attempts AND ((status='pending' AND available_at<=?) OR (status='leased' AND lease_expires_at<=?)) ORDER BY available_at,id LIMIT 1) RETURNING *")
            .bind(uuid_bytes(lease)).bind(uuid_bytes(worker)).bind(lease_us).bind(now_us).bind(now_us).fetch_optional(&self.pool).await.map_err(|e| queue_error(db_error(e)))?;
        row.map(|row| {
            let job = read_job(&row).map_err(queue_error)?;
            let lease = job.lease().cloned().ok_or_else(|| {
                queue_error(SqliteQueueError::Corrupt("claimed job has no lease"))
            })?;
            ClaimedJob::new(job, lease)
                .map_err(|_| queue_error(SqliteQueueError::Corrupt("invalid claimed state")))
        })
        .transpose()
    }
    async fn renew(
        &self,
        job: JobId,
        lease: LeaseId,
        duration: Duration,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        if duration.is_zero() {
            return Err(queue_error(SqliteQueueError::Configuration(
                "lease duration must be positive".into(),
            )));
        }
        let now_us = micros(now).map_err(queue_error)?;
        let expires = now_us
            .checked_add(i64::try_from(duration.as_micros()).unwrap_or(i64::MAX))
            .ok_or_else(|| queue_error(SqliteQueueError::Corrupt("lease expiry out of range")))?;
        let result = sqlx::query("UPDATE queue_jobs SET lease_expires_at=? WHERE id=? AND status='leased' AND lease_id=? AND lease_expires_at>?").bind(expires).bind(uuid_bytes(job)).bind(uuid_bytes(lease)).bind(now_us).execute(&self.pool).await.map_err(|e| queue_error(db_error(e)))?;
        Ok(if result.rows_affected() == 1 {
            LeaseMutationOutcome::Applied
        } else {
            LeaseMutationOutcome::LeaseLost
        })
    }
    async fn complete(
        &self,
        job: JobId,
        lease: LeaseId,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        let result = sqlx::query("UPDATE queue_jobs SET status='completed', completed_at=?, lease_id=NULL, lease_owner=NULL, lease_expires_at=NULL, last_error_code=NULL, last_error_message=NULL WHERE id=? AND status='leased' AND lease_id=? AND lease_expires_at>?").bind(micros(now).map_err(queue_error)?).bind(uuid_bytes(job)).bind(uuid_bytes(lease)).bind(micros(now).map_err(queue_error)?).execute(&self.pool).await.map_err(|e| queue_error(db_error(e)))?;
        Ok(if result.rows_affected() == 1 {
            LeaseMutationOutcome::Applied
        } else {
            LeaseMutationOutcome::LeaseLost
        })
    }
    async fn retry(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        available_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<RetryOutcome, QueueStoreError> {
        let now_us = micros(now).map_err(queue_error)?;
        let row = sqlx::query("UPDATE queue_jobs SET status=CASE WHEN attempts<max_attempts THEN 'pending' ELSE 'failed' END, available_at=?, failed_at=CASE WHEN attempts<max_attempts THEN NULL ELSE ? END, lease_id=NULL, lease_owner=NULL, lease_expires_at=NULL, last_error_code=?, last_error_message=? WHERE id=? AND status='leased' AND lease_id=? AND lease_expires_at>? RETURNING status")
            .bind(micros(available_at).map_err(queue_error)?).bind(now_us).bind(failure.code().as_str()).bind(failure.message()).bind(uuid_bytes(job)).bind(uuid_bytes(lease)).bind(now_us).fetch_optional(&self.pool).await.map_err(|e| queue_error(db_error(e)))?;
        Ok(match row {
            None => RetryOutcome::LeaseLost,
            Some(row) => match row
                .try_get::<String, _>("status")
                .map_err(|e| queue_error(db_error(e)))?
                .as_str()
            {
                "pending" => RetryOutcome::Scheduled,
                _ => RetryOutcome::Exhausted,
            },
        })
    }
    async fn fail(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        let now_us = micros(now).map_err(queue_error)?;
        let result = sqlx::query("UPDATE queue_jobs SET status='failed', failed_at=?, lease_id=NULL, lease_owner=NULL, lease_expires_at=NULL, last_error_code=?, last_error_message=? WHERE id=? AND status='leased' AND lease_id=? AND lease_expires_at>?").bind(now_us).bind(failure.code().as_str()).bind(failure.message()).bind(uuid_bytes(job)).bind(uuid_bytes(lease)).bind(now_us).execute(&self.pool).await.map_err(|e| queue_error(db_error(e)))?;
        Ok(if result.rows_affected() == 1 {
            LeaseMutationOutcome::Applied
        } else {
            LeaseMutationOutcome::LeaseLost
        })
    }
    async fn list(&self, options: QueueListOptions) -> Result<Vec<Job>, QueueStoreError> {
        if options.limit() == 0 || options.limit() > 100 {
            return Err(queue_error(SqliteQueueError::Configuration(
                "job list limit must be 1..=100".into(),
            )));
        }
        let rows = if let Some(state) = options.status() {
            sqlx::query(
                "SELECT * FROM queue_jobs WHERE status=? ORDER BY created_at DESC,id DESC LIMIT ?",
            )
            .bind(status(state))
            .bind(i64::from(options.limit()))
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query("SELECT * FROM queue_jobs ORDER BY created_at DESC,id DESC LIMIT ?")
                .bind(i64::from(options.limit()))
                .fetch_all(&self.pool)
                .await
        }
        .map_err(|e| queue_error(db_error(e)))?;
        rows.iter()
            .map(read_job)
            .collect::<Result<Vec<_>, _>>()
            .map_err(queue_error)
    }
    async fn retry_failed(
        &self,
        job: JobId,
        now: OffsetDateTime,
    ) -> Result<RetryFailedOutcome, QueueStoreError> {
        let result = sqlx::query("UPDATE queue_jobs SET status='pending', attempts=0, available_at=?, failed_at=NULL, completed_at=NULL, lease_id=NULL, lease_owner=NULL, lease_expires_at=NULL, last_error_code=NULL, last_error_message=NULL WHERE id=? AND status='failed'").bind(micros(now).map_err(queue_error)?).bind(uuid_bytes(job)).execute(&self.pool).await.map_err(|e| queue_error(db_error(e)))?;
        if result.rows_affected() == 1 {
            return Ok(RetryFailedOutcome::Retried);
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM queue_jobs WHERE id=?)")
            .bind(uuid_bytes(job))
            .fetch_one(&self.pool)
            .await
            .map_err(|e| queue_error(db_error(e)))?;
        Ok(if exists {
            RetryFailedOutcome::NotFailed
        } else {
            RetryFailedOutcome::NotFound
        })
    }
    async fn maintain(
        &self,
        now: OffsetDateTime,
        completed_retention: Duration,
    ) -> Result<QueueMaintenanceReport, QueueStoreError> {
        let now_us = micros(now).map_err(queue_error)?;
        let exhausted = sqlx::query("UPDATE queue_jobs SET status='failed', failed_at=?, lease_id=NULL, lease_owner=NULL, lease_expires_at=NULL, last_error_code='queue.attempts_exhausted', last_error_message='maximum attempts exhausted' WHERE status='leased' AND lease_expires_at<=? AND attempts>=max_attempts").bind(now_us).bind(now_us).execute(&self.pool).await.map_err(|e| queue_error(db_error(e)))?.rows_affected();
        let cutoff = now_us
            .checked_sub(i64::try_from(completed_retention.as_micros()).unwrap_or(i64::MAX))
            .unwrap_or(i64::MIN);
        let pruned =
            sqlx::query("DELETE FROM queue_jobs WHERE status='completed' AND completed_at<=?")
                .bind(cutoff)
                .execute(&self.pool)
                .await
                .map_err(|e| queue_error(db_error(e)))?
                .rows_affected();
        Ok(QueueMaintenanceReport::new(exhausted, pruned))
    }
}
