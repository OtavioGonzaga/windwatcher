use super::QueueMigrationStatus;
use crate::{
    ClaimedJob, Job, JobFailure, JobFailureCode, JobId, JobKind, JobLease, JobPayload, JobStatus,
    LeaseId, LeaseMutationOutcome, QueueListOptions, QueueMaintenanceReport, QueueStore,
    QueueStoreError, RetryFailedOutcome, RetryOutcome, WorkerId,
};
use sqlx::{
    MySqlPool, Row,
    mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlRow},
};
use std::{error::Error, fmt, str::FromStr, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/queue/mysql");

/// Connection options for a dedicated MySQL 8 queue database.
#[derive(Clone)]
pub struct MySqlQueueOptions {
    url: String,
    max_connections: u32,
    acquire_timeout: Duration,
}
impl fmt::Debug for MySqlQueueOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MySqlQueueOptions")
            .field("url", &"[REDACTED]")
            .field("max_connections", &self.max_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .finish()
    }
}
impl MySqlQueueOptions {
    /// Creates options without opening a connection.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: 8,
            acquire_timeout: Duration::from_secs(5),
        }
    }
    /// Sets the maximum pool size.
    pub fn max_connections(mut self, value: u32) -> Self {
        self.max_connections = value;
        self
    }
    /// Sets the pool acquisition timeout.
    pub fn acquire_timeout(mut self, value: Duration) -> Self {
        self.acquire_timeout = value;
        self
    }
    /// Validates options without connecting or disclosing the URL.
    pub fn validate(&self) -> Result<(), MySqlQueueError> {
        self.connect_options().map(|_| ())
    }
    fn connect_options(&self) -> Result<MySqlConnectOptions, MySqlQueueError> {
        if self.max_connections == 0
            || self.acquire_timeout.is_zero()
            || std::time::Instant::now()
                .checked_add(self.acquire_timeout)
                .is_none()
        {
            return Err(MySqlQueueError::Configuration(
                "pool size and acquire timeout must be positive",
            ));
        }
        if !self.url.starts_with("mysql://") {
            return Err(MySqlQueueError::Configuration("invalid MySQL URL"));
        }
        MySqlConnectOptions::from_str(&self.url)
            .map_err(|_| MySqlQueueError::Configuration("invalid MySQL URL"))
    }
}
/// MySQL queue infrastructure failure; source details are excluded from diagnostics.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum MySqlQueueError {
    /// Invalid URL or pool options.
    #[error("invalid MySQL queue configuration: {0}")]
    Configuration(&'static str),
    /// Connection or query failure.
    #[error("MySQL queue operation failed")]
    Database(#[source] Box<dyn Error + Send + Sync>),
    /// Migration application failed.
    #[error("MySQL queue migration failed")]
    Migration(#[source] Box<dyn Error + Send + Sync>),
    /// Unknown, dirty, or modified migration history.
    #[error("invalid MySQL queue migration history")]
    InvalidMigrationHistory,
    /// Malformed persisted state.
    #[error("invalid persisted MySQL queue job: {0}")]
    Corrupt(&'static str),
}
impl fmt::Debug for MySqlQueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl From<MySqlQueueError> for QueueStoreError {
    fn from(value: MySqlQueueError) -> Self {
        Self::new(value)
    }
}
fn db(error: impl Error + Send + Sync + 'static) -> MySqlQueueError {
    MySqlQueueError::Database(Box::new(error))
}
fn micros(value: OffsetDateTime) -> Result<i64, MySqlQueueError> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1000))
        .map_err(|_| MySqlQueueError::Corrupt("timestamp out of range"))
}
fn datetime(value: i64) -> Result<OffsetDateTime, MySqlQueueError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1000)
        .map_err(|_| MySqlQueueError::Corrupt("invalid timestamp"))
}
fn bytes<T: Into<Uuid>>(value: T) -> Vec<u8> {
    value.into().into_bytes().to_vec()
}
fn id<T: TryFrom<Uuid>>(value: Vec<u8>) -> Result<T, MySqlQueueError> {
    let uuid =
        Uuid::from_slice(&value).map_err(|_| MySqlQueueError::Corrupt("invalid UUID bytes"))?;
    T::try_from(uuid).map_err(|_| MySqlQueueError::Corrupt("invalid UUID version"))
}
fn status(value: JobStatus) -> &'static str {
    match value {
        JobStatus::Pending => "pending",
        JobStatus::Leased => "leased",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
    }
}
fn applied(rows: u64) -> LeaseMutationOutcome {
    if rows == 1 {
        LeaseMutationOutcome::Applied
    } else {
        LeaseMutationOutcome::LeaseLost
    }
}
/// MySQL implementation of the portable queue contract.
#[derive(Clone)]
pub struct MySqlQueue {
    pool: MySqlPool,
}
impl fmt::Debug for MySqlQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MySqlQueue").finish_non_exhaustive()
    }
}
impl MySqlQueue {
    /// Opens a pool without creating a database or applying migrations.
    pub async fn connect(options: MySqlQueueOptions) -> Result<Self, MySqlQueueError> {
        let connect = options.connect_options()?;
        let pool = MySqlPoolOptions::new()
            .max_connections(options.max_connections)
            .acquire_timeout(options.acquire_timeout)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    // Queue locking reads need row locks, not repeatable-read gap locks.
                    sqlx::query("SET SESSION TRANSACTION ISOLATION LEVEL READ COMMITTED")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(connect)
            .await
            .map_err(db)?;
        Ok(Self { pool })
    }
    /// Explicitly applies embedded queue migrations.
    pub async fn migrate(&self) -> Result<(), MySqlQueueError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| MySqlQueueError::Migration(Box::new(e)))
    }
    /// Inspects migration history without changing the schema.
    pub async fn migration_status(&self) -> Result<QueueMigrationStatus, MySqlQueueError> {
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM information_schema.tables WHERE table_schema=DATABASE() AND table_name='_sqlx_migrations'").fetch_one(&self.pool).await.map_err(db)?;
        let mut result = QueueMigrationStatus {
            current: None,
            latest: MIGRATOR.iter().map(|m| m.version).max(),
            pending: MIGRATOR.iter().count(),
        };
        if exists == 0 {
            return Ok(result);
        }
        for row in
            sqlx::query("SELECT version,checksum,success FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&self.pool)
                .await
                .map_err(db)?
        {
            let version: i64 = row.try_get("version").map_err(db)?;
            let checksum: Vec<u8> = row.try_get("checksum").map_err(db)?;
            let success: bool = row.try_get("success").map_err(db)?;
            if !success
                || !MIGRATOR
                    .iter()
                    .any(|m| m.version == version && m.checksum.as_ref() == checksum)
            {
                return Err(MySqlQueueError::InvalidMigrationHistory);
            }
            result.current = Some(version);
            result.pending = result.pending.saturating_sub(1);
        }
        Ok(result)
    }
    /// Checks schema readiness without applying migrations.
    pub async fn check_schema(&self) -> Result<QueueMigrationStatus, MySqlQueueError> {
        self.migration_status().await
    }
}
fn read_job(row: &MySqlRow) -> Result<Job, MySqlQueueError> {
    let state = match row.try_get::<String, _>("status").map_err(db)?.as_str() {
        "pending" => JobStatus::Pending,
        "leased" => JobStatus::Leased,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        _ => return Err(MySqlQueueError::Corrupt("unknown status")),
    };
    let payload_bytes: Vec<u8> = row.try_get("payload_json").map_err(db)?;
    let payload = super::payload::restore(&payload_bytes).ok();
    let invalid = payload.is_none();
    let payload = payload.unwrap_or_else(|| JobPayload::new(Default::default()));
    let lease = match (
        row.try_get::<Option<Vec<u8>>, _>("lease_id").map_err(db)?,
        row.try_get::<Option<Vec<u8>>, _>("lease_owner")
            .map_err(db)?,
        row.try_get::<Option<i64>, _>("lease_expires_at")
            .map_err(db)?,
    ) {
        (Some(l), Some(w), Some(t)) => Some(JobLease::new(id(l)?, id(w)?, datetime(t)?)),
        (None, None, None) => None,
        _ => return Err(MySqlQueueError::Corrupt("incoherent lease")),
    };
    let failure = match (
        row.try_get::<Option<String>, _>("last_error_code")
            .map_err(db)?,
        row.try_get::<Option<Vec<u8>>, _>("last_error_message")
            .map_err(db)?,
    ) {
        (Some(c), Some(m)) => Some(
            JobFailure::new(
                JobFailureCode::new(c)
                    .map_err(|_| MySqlQueueError::Corrupt("invalid failure code"))?,
                String::from_utf8(m)
                    .map_err(|_| MySqlQueueError::Corrupt("invalid failure UTF-8"))?,
            )
            .map_err(|_| MySqlQueueError::Corrupt("invalid failure"))?,
        ),
        (None, None) => None,
        _ => return Err(MySqlQueueError::Corrupt("incoherent failure")),
    };
    let optional = |name: &str| -> Result<Option<OffsetDateTime>, MySqlQueueError> {
        row.try_get::<Option<i64>, _>(name)
            .map_err(db)?
            .map(datetime)
            .transpose()
    };
    let mut job = Job::restore(
        id(row.try_get("id").map_err(db)?)?,
        JobKind::new(row.try_get::<String, _>("kind").map_err(db)?)
            .map_err(|_| MySqlQueueError::Corrupt("invalid kind"))?,
        u16::try_from(row.try_get::<i32, _>("payload_version").map_err(db)?)
            .map_err(|_| MySqlQueueError::Corrupt("invalid version"))?,
        payload,
        state,
        datetime(row.try_get("available_at").map_err(db)?)?,
        u32::try_from(row.try_get::<i64, _>("attempts").map_err(db)?)
            .map_err(|_| MySqlQueueError::Corrupt("invalid attempts"))?,
        u32::try_from(row.try_get::<i64, _>("max_attempts").map_err(db)?)
            .map_err(|_| MySqlQueueError::Corrupt("invalid max attempts"))?,
        lease,
        failure,
        datetime(row.try_get("created_at").map_err(db)?)?,
        optional("completed_at")?,
        optional("failed_at")?,
    )
    .map_err(|_| MySqlQueueError::Corrupt("inconsistent job state"))?;
    job.payload_invalid = invalid;
    Ok(job)
}
impl QueueStore for MySqlQueue {
    async fn enqueue(&self, job: &Job) -> Result<(), QueueStoreError> {
        sqlx::query("INSERT INTO queue_jobs (id,kind,payload_version,payload_json,status,available_at,attempts,max_attempts,created_at) VALUES (?,?,?,?,?,?,?,?,?)")
            .bind(bytes(job.id())).bind(job.kind().as_str()).bind(i32::from(job.payload_version())).bind(serde_json::to_vec(job.payload()).map_err(db)?).bind(status(job.status())).bind(micros(job.available_at())?).bind(i64::from(job.attempts())).bind(i64::from(job.max_attempts())).bind(micros(job.created_at())?).execute(&self.pool).await.map_err(db)?;
        Ok(())
    }
    async fn claim_next(
        &self,
        worker: WorkerId,
        lease: LeaseId,
        duration: Duration,
        now: OffsetDateTime,
    ) -> Result<Option<ClaimedJob>, QueueStoreError> {
        let expires = micros(JobLease::expiration(now, duration).map_err(QueueStoreError::new)?)?;
        let now = micros(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        let exhausted: Vec<Vec<u8>> = sqlx::query_scalar("SELECT id FROM queue_jobs WHERE status='leased' AND lease_expires_at<=? AND attempts>=max_attempts LIMIT 64 FOR UPDATE SKIP LOCKED").bind(now).fetch_all(&mut *tx).await.map_err(db)?;
        for id in exhausted {
            sqlx::query("UPDATE queue_jobs SET status='failed',failed_at=?,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code='queue.attempts_exhausted',last_error_message='maximum attempts exhausted' WHERE id=?").bind(now).bind(id).execute(&mut *tx).await.map_err(db)?;
        }
        let candidate: Option<Vec<u8>> = sqlx::query_scalar("SELECT id FROM queue_jobs WHERE attempts<max_attempts AND ((status='pending' AND available_at<=?) OR (status='leased' AND lease_expires_at<=?)) ORDER BY available_at,id LIMIT 1 FOR UPDATE SKIP LOCKED").bind(now).bind(now).fetch_optional(&mut *tx).await.map_err(db)?;
        let claim = if let Some(id) = candidate {
            sqlx::query("UPDATE queue_jobs SET status='leased',attempts=attempts+1,lease_id=?,lease_owner=?,lease_expires_at=? WHERE id=?").bind(bytes(lease)).bind(bytes(worker)).bind(expires).bind(&id).execute(&mut *tx).await.map_err(db)?;
            let row = sqlx::query("SELECT * FROM queue_jobs WHERE id=?")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
            let job = read_job(&row)?;
            let authority = job
                .lease()
                .cloned()
                .ok_or(MySqlQueueError::Corrupt("missing lease"))?;
            Some(
                ClaimedJob::new(job, authority)
                    .map_err(|_| MySqlQueueError::Corrupt("invalid claim"))?,
            )
        } else {
            None
        };
        tx.commit().await.map_err(db)?;
        Ok(claim)
    }
    async fn renew(
        &self,
        job: JobId,
        lease: LeaseId,
        duration: Duration,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        let expires = micros(JobLease::expiration(now, duration).map_err(QueueStoreError::new)?)?;
        Ok(applied(sqlx::query("UPDATE queue_jobs SET lease_expires_at=? WHERE id=? AND status='leased' AND lease_id=? AND lease_expires_at>?").bind(expires).bind(bytes(job)).bind(bytes(lease)).bind(micros(now)?).execute(&self.pool).await.map_err(db)?.rows_affected()))
    }
    async fn complete(
        &self,
        job: JobId,
        lease: LeaseId,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        let now = micros(now)?;
        Ok(applied(sqlx::query("UPDATE queue_jobs SET status='completed',completed_at=?,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code=NULL,last_error_message=NULL WHERE id=? AND status='leased' AND lease_id=? AND lease_expires_at>?").bind(now).bind(bytes(job)).bind(bytes(lease)).bind(now).execute(&self.pool).await.map_err(db)?.rows_affected()))
    }
    async fn retry(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        available_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<RetryOutcome, QueueStoreError> {
        let now = micros(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        let changed = sqlx::query("UPDATE queue_jobs SET status=CASE WHEN attempts<max_attempts THEN 'pending' ELSE 'failed' END,available_at=?,failed_at=CASE WHEN attempts<max_attempts THEN NULL ELSE ? END,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code=?,last_error_message=? WHERE id=? AND status='leased' AND lease_id=? AND lease_expires_at>?")
            .bind(micros(available_at)?).bind(now).bind(failure.code().as_str()).bind(failure.message().as_bytes()).bind(bytes(job)).bind(bytes(lease)).bind(now).execute(&mut *tx).await.map_err(db)?.rows_affected();
        let result = if changed == 0 {
            RetryOutcome::LeaseLost
        } else {
            let state: String = sqlx::query_scalar("SELECT status FROM queue_jobs WHERE id=?")
                .bind(bytes(job))
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
            if state == "pending" {
                RetryOutcome::Scheduled
            } else {
                RetryOutcome::Exhausted
            }
        };
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    async fn fail(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        let now = micros(now)?;
        Ok(applied(sqlx::query("UPDATE queue_jobs SET status='failed',failed_at=?,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code=?,last_error_message=? WHERE id=? AND status='leased' AND lease_id=? AND lease_expires_at>?").bind(now).bind(failure.code().as_str()).bind(failure.message().as_bytes()).bind(bytes(job)).bind(bytes(lease)).bind(now).execute(&self.pool).await.map_err(db)?.rows_affected()))
    }
    async fn list(&self, options: QueueListOptions) -> Result<Vec<Job>, QueueStoreError> {
        let rows = if let Some(state) = options.status() {
            sqlx::query(
                "SELECT * FROM queue_jobs WHERE status=? ORDER BY created_at DESC,id DESC LIMIT ?",
            )
            .bind(status(state))
            .bind(options.limit())
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query("SELECT * FROM queue_jobs ORDER BY created_at DESC,id DESC LIMIT ?")
                .bind(options.limit())
                .fetch_all(&self.pool)
                .await
        }
        .map_err(db)?;
        rows.iter()
            .map(read_job)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }
    async fn retry_failed(
        &self,
        job: JobId,
        now: OffsetDateTime,
    ) -> Result<RetryFailedOutcome, QueueStoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let changed = sqlx::query("UPDATE queue_jobs SET status='pending',attempts=0,available_at=?,failed_at=NULL,completed_at=NULL,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code=NULL,last_error_message=NULL WHERE id=? AND status='failed'").bind(micros(now)?).bind(bytes(job)).execute(&mut *tx).await.map_err(db)?.rows_affected();
        let result = if changed == 1 {
            RetryFailedOutcome::Retried
        } else {
            let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM queue_jobs WHERE id=?")
                .bind(bytes(job))
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
            if exists > 0 {
                RetryFailedOutcome::NotFailed
            } else {
                RetryFailedOutcome::NotFound
            }
        };
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    async fn maintain(
        &self,
        now: OffsetDateTime,
        retention: Duration,
    ) -> Result<QueueMaintenanceReport, QueueStoreError> {
        let now_sql = micros(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        let ids: Vec<Vec<u8>> = sqlx::query_scalar("SELECT id FROM queue_jobs WHERE status='leased' AND lease_expires_at<=? AND attempts>=max_attempts FOR UPDATE SKIP LOCKED").bind(now_sql).fetch_all(&mut *tx).await.map_err(db)?;
        let exhausted = ids.len() as u64;
        for id in ids {
            sqlx::query("UPDATE queue_jobs SET status='failed',failed_at=?,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code='queue.attempts_exhausted',last_error_message='maximum attempts exhausted' WHERE id=?").bind(now_sql).bind(id).execute(&mut *tx).await.map_err(db)?;
        }
        let cutoff = now_sql
            .checked_sub(i64::try_from(retention.as_micros()).unwrap_or(i64::MAX))
            .unwrap_or(i64::MIN);
        let ids: Vec<Vec<u8>> = sqlx::query_scalar("SELECT id FROM queue_jobs WHERE status='completed' AND completed_at<=? FOR UPDATE SKIP LOCKED").bind(cutoff).fetch_all(&mut *tx).await.map_err(db)?;
        let pruned = ids.len() as u64;
        for id in ids {
            sqlx::query("DELETE FROM queue_jobs WHERE id=?")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(QueueMaintenanceReport::new(exhausted, pruned))
    }
}
