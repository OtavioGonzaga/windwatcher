use std::{error::Error, fmt, str::FromStr, time::Duration};

use sqlx::{
    PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions, PgRow},
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::QueueMigrationStatus;
use crate::{
    ClaimedJob, Job, JobFailure, JobFailureCode, JobId, JobKind, JobLease, JobPayload, JobStatus,
    LeaseId, LeaseMutationOutcome, QueueListOptions, QueueMaintenanceReport, QueueStore,
    QueueStoreError, RetryFailedOutcome, RetryOutcome, WorkerId,
};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/queue/postgres");

/// Connection options for a dedicated PostgreSQL queue database.
#[derive(Clone)]
pub struct PostgresQueueOptions {
    url: String,
    max_connections: u32,
    acquire_timeout: Duration,
}
impl fmt::Debug for PostgresQueueOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresQueueOptions")
            .field("url", &"[REDACTED]")
            .field("max_connections", &self.max_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .finish()
    }
}
impl PostgresQueueOptions {
    /// Creates options; connecting never creates the database or applies migrations.
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
    /// Validates options without connecting or exposing connection credentials.
    pub fn validate(&self) -> Result<(), PostgresQueueError> {
        self.connect_options().map(|_| ())
    }
    fn connect_options(&self) -> Result<PgConnectOptions, PostgresQueueError> {
        if self.max_connections == 0
            || self.acquire_timeout.is_zero()
            || std::time::Instant::now()
                .checked_add(self.acquire_timeout)
                .is_none()
        {
            return Err(PostgresQueueError::Configuration(
                "pool size and acquire timeout must be positive",
            ));
        }
        if !(self.url.starts_with("postgres://") || self.url.starts_with("postgresql://")) {
            return Err(PostgresQueueError::Configuration("invalid PostgreSQL URL"));
        }
        PgConnectOptions::from_str(&self.url)
            .map_err(|_| PostgresQueueError::Configuration("invalid PostgreSQL URL"))
    }
}

/// PostgreSQL queue infrastructure failure. Display and Debug never expose source details.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum PostgresQueueError {
    /// Invalid URL or pool options.
    #[error("invalid PostgreSQL queue configuration: {0}")]
    Configuration(&'static str),
    /// Connection or query failed.
    #[error("PostgreSQL queue operation failed")]
    Database(#[source] Box<dyn Error + Send + Sync>),
    /// Migration application failed.
    #[error("PostgreSQL queue migration failed")]
    Migration(#[source] Box<dyn Error + Send + Sync>),
    /// Migration history does not match the embedded schema.
    #[error("invalid PostgreSQL queue migration history")]
    InvalidMigrationHistory,
    /// Persisted state is malformed.
    #[error("invalid persisted PostgreSQL queue job: {0}")]
    Corrupt(&'static str),
}
impl fmt::Debug for PostgresQueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl From<PostgresQueueError> for QueueStoreError {
    fn from(value: PostgresQueueError) -> Self {
        Self::new(value)
    }
}
fn db(error: impl Error + Send + Sync + 'static) -> PostgresQueueError {
    PostgresQueueError::Database(Box::new(error))
}
fn micros(value: OffsetDateTime) -> Result<i64, PostgresQueueError> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1000))
        .map_err(|_| PostgresQueueError::Corrupt("timestamp out of range"))
}
fn datetime(value: i64) -> Result<OffsetDateTime, PostgresQueueError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1000)
        .map_err(|_| PostgresQueueError::Corrupt("invalid timestamp"))
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

/// PostgreSQL implementation of the portable queue contract.
#[derive(Clone)]
pub struct PostgresQueue {
    pool: PgPool,
}
impl fmt::Debug for PostgresQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresQueue").finish_non_exhaustive()
    }
}
impl PostgresQueue {
    /// Opens the pool without applying migrations.
    pub async fn connect(options: PostgresQueueOptions) -> Result<Self, PostgresQueueError> {
        let connect = options.connect_options()?;
        let pool = PgPoolOptions::new()
            .max_connections(options.max_connections)
            .acquire_timeout(options.acquire_timeout)
            .connect_with(connect)
            .await
            .map_err(db)?;
        Ok(Self { pool })
    }
    /// Explicitly applies embedded queue migrations.
    pub async fn migrate(&self) -> Result<(), PostgresQueueError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| PostgresQueueError::Migration(Box::new(e)))
    }
    /// Inspects schema history without applying migrations.
    pub async fn migration_status(&self) -> Result<QueueMigrationStatus, PostgresQueueError> {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        let mut result = QueueMigrationStatus {
            current: None,
            latest: MIGRATOR.iter().map(|m| m.version).max(),
            pending: MIGRATOR.iter().count(),
        };
        if !exists {
            return Ok(result);
        }
        let rows =
            sqlx::query("SELECT version,checksum,success FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&self.pool)
                .await
                .map_err(db)?;
        for row in rows {
            let version: i64 = row.try_get("version").map_err(db)?;
            let checksum: Vec<u8> = row.try_get("checksum").map_err(db)?;
            let success: bool = row.try_get("success").map_err(db)?;
            if !success
                || !MIGRATOR
                    .iter()
                    .any(|m| m.version == version && m.checksum.as_ref() == checksum)
            {
                return Err(PostgresQueueError::InvalidMigrationHistory);
            }
            result.current = Some(version);
            result.pending = result.pending.saturating_sub(1);
        }
        Ok(result)
    }
    /// Checks schema readiness without applying migrations.
    pub async fn check_schema(&self) -> Result<QueueMigrationStatus, PostgresQueueError> {
        self.migration_status().await
    }
}

fn id<T: TryFrom<Uuid>>(value: Uuid) -> Result<T, PostgresQueueError> {
    T::try_from(value).map_err(|_| PostgresQueueError::Corrupt("invalid UUID version"))
}
fn read_job(row: &PgRow) -> Result<Job, PostgresQueueError> {
    let state = match row.try_get::<String, _>("status").map_err(db)?.as_str() {
        "pending" => JobStatus::Pending,
        "leased" => JobStatus::Leased,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        _ => return Err(PostgresQueueError::Corrupt("unknown status")),
    };
    let payload_bytes: Vec<u8> = row.try_get("payload_json").map_err(db)?;
    let payload = super::payload::restore(&payload_bytes).ok();
    let invalid = payload.is_none();
    let payload = payload.unwrap_or_else(|| JobPayload::new(Default::default()));
    let lease = match (
        row.try_get::<Option<Uuid>, _>("lease_id").map_err(db)?,
        row.try_get::<Option<Uuid>, _>("lease_owner").map_err(db)?,
        row.try_get::<Option<i64>, _>("lease_expires_at")
            .map_err(db)?,
    ) {
        (Some(l), Some(w), Some(t)) => Some(JobLease::new(id(l)?, id(w)?, datetime(t)?)),
        (None, None, None) => None,
        _ => return Err(PostgresQueueError::Corrupt("incoherent lease")),
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
                    .map_err(|_| PostgresQueueError::Corrupt("invalid failure code"))?,
                String::from_utf8(m)
                    .map_err(|_| PostgresQueueError::Corrupt("invalid failure UTF-8"))?,
            )
            .map_err(|_| PostgresQueueError::Corrupt("invalid failure"))?,
        ),
        (None, None) => None,
        _ => return Err(PostgresQueueError::Corrupt("incoherent failure")),
    };
    let optional_time = |name: &str| -> Result<Option<OffsetDateTime>, PostgresQueueError> {
        row.try_get::<Option<i64>, _>(name)
            .map_err(db)?
            .map(datetime)
            .transpose()
    };
    let mut job = Job::restore(
        id(row.try_get("id").map_err(db)?)?,
        JobKind::new(row.try_get::<String, _>("kind").map_err(db)?)
            .map_err(|_| PostgresQueueError::Corrupt("invalid kind"))?,
        u16::try_from(row.try_get::<i32, _>("payload_version").map_err(db)?)
            .map_err(|_| PostgresQueueError::Corrupt("invalid version"))?,
        payload,
        state,
        datetime(row.try_get("available_at").map_err(db)?)?,
        u32::try_from(row.try_get::<i64, _>("attempts").map_err(db)?)
            .map_err(|_| PostgresQueueError::Corrupt("invalid attempts"))?,
        u32::try_from(row.try_get::<i64, _>("max_attempts").map_err(db)?)
            .map_err(|_| PostgresQueueError::Corrupt("invalid max attempts"))?,
        lease,
        failure,
        datetime(row.try_get("created_at").map_err(db)?)?,
        optional_time("completed_at")?,
        optional_time("failed_at")?,
    )
    .map_err(|_| PostgresQueueError::Corrupt("inconsistent job state"))?;
    job.payload_invalid = invalid;
    Ok(job)
}

impl QueueStore for PostgresQueue {
    async fn enqueue(&self, job: &Job) -> Result<(), QueueStoreError> {
        sqlx::query("INSERT INTO queue_jobs (id,kind,payload_version,payload_json,status,available_at,attempts,max_attempts,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(job.id().as_uuid()).bind(job.kind().as_str()).bind(i32::from(job.payload_version()))
            .bind(serde_json::to_vec(job.payload()).map_err(db)?).bind(status(job.status())).bind(micros(job.available_at())?)
            .bind(i64::from(job.attempts())).bind(i64::from(job.max_attempts())).bind(micros(job.created_at())?)
            .execute(&self.pool).await.map_err(db)?;
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
        sqlx::query("WITH exhausted AS (SELECT id FROM queue_jobs WHERE status='leased' AND lease_expires_at<=$1 AND attempts>=max_attempts LIMIT 64 FOR UPDATE SKIP LOCKED) UPDATE queue_jobs SET status='failed',failed_at=$1,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code='queue.attempts_exhausted',last_error_message='maximum attempts exhausted' WHERE id IN (SELECT id FROM exhausted)")
            .bind(now).execute(&mut *tx).await.map_err(db)?;
        let row = sqlx::query("WITH candidate AS (SELECT id FROM queue_jobs WHERE attempts<max_attempts AND ((status='pending' AND available_at<=$1) OR (status='leased' AND lease_expires_at<=$1)) ORDER BY available_at,id LIMIT 1 FOR UPDATE SKIP LOCKED) UPDATE queue_jobs SET status='leased',attempts=attempts+1,lease_id=$2,lease_owner=$3,lease_expires_at=$4 WHERE id IN (SELECT id FROM candidate) RETURNING *")
            .bind(now).bind(lease.as_uuid()).bind(worker.as_uuid()).bind(expires).fetch_optional(&mut *tx).await.map_err(db)?;
        let claim = row
            .as_ref()
            .map(|row| {
                let job = read_job(row)?;
                let authority = job
                    .lease()
                    .cloned()
                    .ok_or(PostgresQueueError::Corrupt("missing lease"))?;
                let result = ClaimedJob::new(job, authority);
                result.map_err(|_| PostgresQueueError::Corrupt("invalid claim"))
            })
            .transpose()?;
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
        Ok(applied(sqlx::query("UPDATE queue_jobs SET lease_expires_at=$1 WHERE id=$2 AND status='leased' AND lease_id=$3 AND lease_expires_at>$4")
            .bind(expires).bind(job.as_uuid()).bind(lease.as_uuid()).bind(micros(now)?).execute(&self.pool).await.map_err(db)?.rows_affected()))
    }
    async fn complete(
        &self,
        job: JobId,
        lease: LeaseId,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        Ok(applied(sqlx::query("UPDATE queue_jobs SET status='completed',completed_at=$1,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code=NULL,last_error_message=NULL WHERE id=$2 AND status='leased' AND lease_id=$3 AND lease_expires_at>$1")
            .bind(micros(now)?).bind(job.as_uuid()).bind(lease.as_uuid()).execute(&self.pool).await.map_err(db)?.rows_affected()))
    }
    async fn retry(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        available_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<RetryOutcome, QueueStoreError> {
        let row = sqlx::query("UPDATE queue_jobs SET status=CASE WHEN attempts<max_attempts THEN 'pending' ELSE 'failed' END,available_at=$1,failed_at=CASE WHEN attempts<max_attempts THEN NULL ELSE $2 END,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code=$3,last_error_message=$4 WHERE id=$5 AND status='leased' AND lease_id=$6 AND lease_expires_at>$2 RETURNING status")
            .bind(micros(available_at)?).bind(micros(now)?).bind(failure.code().as_str()).bind(failure.message().as_bytes()).bind(job.as_uuid()).bind(lease.as_uuid()).fetch_optional(&self.pool).await.map_err(db)?;
        Ok(match row {
            None => RetryOutcome::LeaseLost,
            Some(row) => {
                if row.try_get::<String, _>("status").map_err(db)? == "pending" {
                    RetryOutcome::Scheduled
                } else {
                    RetryOutcome::Exhausted
                }
            }
        })
    }
    async fn fail(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        Ok(applied(sqlx::query("UPDATE queue_jobs SET status='failed',failed_at=$1,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code=$2,last_error_message=$3 WHERE id=$4 AND status='leased' AND lease_id=$5 AND lease_expires_at>$1")
            .bind(micros(now)?).bind(failure.code().as_str()).bind(failure.message().as_bytes()).bind(job.as_uuid()).bind(lease.as_uuid()).execute(&self.pool).await.map_err(db)?.rows_affected()))
    }
    async fn list(&self, options: QueueListOptions) -> Result<Vec<Job>, QueueStoreError> {
        let rows = sqlx::query("SELECT * FROM queue_jobs WHERE ($1::text IS NULL OR status=$1) ORDER BY created_at DESC,id DESC LIMIT $2")
            .bind(options.status().map(status)).bind(i64::from(options.limit())).fetch_all(&self.pool).await.map_err(db)?;
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
        let row = sqlx::query("UPDATE queue_jobs SET status='pending',attempts=0,available_at=$1,failed_at=NULL,completed_at=NULL,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code=NULL,last_error_message=NULL WHERE id=$2 AND status='failed' RETURNING id")
            .bind(micros(now)?).bind(job.as_uuid()).fetch_optional(&self.pool).await.map_err(db)?;
        if row.is_some() {
            return Ok(RetryFailedOutcome::Retried);
        }
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM queue_jobs WHERE id=$1)")
                .bind(job.as_uuid())
                .fetch_one(&self.pool)
                .await
                .map_err(db)?;
        Ok(if exists {
            RetryFailedOutcome::NotFailed
        } else {
            RetryFailedOutcome::NotFound
        })
    }
    async fn maintain(
        &self,
        now: OffsetDateTime,
        retention: Duration,
    ) -> Result<QueueMaintenanceReport, QueueStoreError> {
        let now = micros(now)?;
        let exhausted = sqlx::query("UPDATE queue_jobs SET status='failed',failed_at=$1,lease_id=NULL,lease_owner=NULL,lease_expires_at=NULL,last_error_code='queue.attempts_exhausted',last_error_message='maximum attempts exhausted' WHERE status='leased' AND lease_expires_at<=$1 AND attempts>=max_attempts")
            .bind(now).execute(&self.pool).await.map_err(db)?.rows_affected();
        let cutoff = now
            .checked_sub(i64::try_from(retention.as_micros()).unwrap_or(i64::MAX))
            .unwrap_or(i64::MIN);
        let pruned = {
            sqlx::query("DELETE FROM queue_jobs WHERE status='completed' AND completed_at<=$1")
                .bind(cutoff)
                .execute(&self.pool)
                .await
                .map_err(db)?
                .rows_affected()
        };
        Ok(QueueMaintenanceReport::new(exhausted, pruned))
    }
}
