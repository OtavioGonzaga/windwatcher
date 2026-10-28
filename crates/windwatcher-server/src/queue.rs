use std::time::Duration;

use crate::config::ResolvedQueueProvider;
use time::OffsetDateTime;
use windwatcher::{
    ClaimedJob, Job, JobFailure, JobId, LeaseId, LeaseMutationOutcome, MySqlQueue, MySqlQueueError,
    PostgresQueue, PostgresQueueError, QueueListOptions, QueueMaintenanceReport, QueueStore,
    QueueStoreError, RedisQueue, RedisQueueError, RetryFailedOutcome, RetryOutcome, SqliteQueue,
    SqliteQueueError, WorkerId,
};

#[derive(Debug, thiserror::Error)]
pub enum QueueLifecycleError {
    #[error("SQLite queue lifecycle failed")]
    Sqlite(#[from] SqliteQueueError),
    #[error("PostgreSQL queue lifecycle failed")]
    Postgres(#[from] PostgresQueueError),
    #[error("MySQL queue lifecycle failed")]
    MySql(#[from] MySqlQueueError),
    #[error("Redis queue lifecycle failed")]
    Redis(#[from] RedisQueueError),
}
/// Schema-backed queues expose versions; Redis does not have schema migrations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueMigrationState {
    Applicable(windwatcher::QueueMigrationStatus),
    NotApplicable,
}
impl QueueMigrationState {
    pub fn is_current(&self) -> bool {
        match self {
            Self::Applicable(status) => status.is_current(),
            Self::NotApplicable => true,
        }
    }
}

/// Concrete server-side queue adapter dispatch.
#[derive(Clone, Debug)]
pub enum ConfiguredQueue {
    /// Official SQLite queue adapter.
    Sqlite(SqliteQueue),
    /// Official PostgreSQL queue adapter.
    Postgres(PostgresQueue),
    /// Official MySQL queue adapter.
    MySql(MySqlQueue),
    /// Official Redis queue adapter.
    Redis(RedisQueue),
}
impl ConfiguredQueue {
    /// Opens the configured queue without applying migrations.
    pub async fn connect(provider: ResolvedQueueProvider) -> Result<Self, QueueLifecycleError> {
        Ok(match provider {
            ResolvedQueueProvider::Sqlite { options, .. } => {
                Self::Sqlite(SqliteQueue::connect(options).await?)
            }
            ResolvedQueueProvider::Postgres(options) => {
                Self::Postgres(PostgresQueue::connect(options).await?)
            }
            ResolvedQueueProvider::MySql(options) => {
                Self::MySql(MySqlQueue::connect(options).await?)
            }
            ResolvedQueueProvider::Redis(options) => {
                Self::Redis(RedisQueue::connect(options).await?)
            }
        })
    }
    /// Applies the adapter's embedded migrations.
    pub async fn migrate(&self) -> Result<(), QueueLifecycleError> {
        match self {
            Self::Sqlite(queue) => queue.migrate().await?,
            Self::Postgres(queue) => queue.migrate().await?,
            Self::MySql(queue) => queue.migrate().await?,
            Self::Redis(_) => {}
        }
        Ok(())
    }
    /// Inspects adapter migration status.
    pub async fn migration_status(&self) -> Result<QueueMigrationState, QueueLifecycleError> {
        Ok(match self {
            Self::Sqlite(queue) => QueueMigrationState::Applicable(queue.migration_status().await?),
            Self::Postgres(queue) => {
                QueueMigrationState::Applicable(queue.migration_status().await?)
            }
            Self::MySql(queue) => QueueMigrationState::Applicable(queue.migration_status().await?),
            Self::Redis(_) => QueueMigrationState::NotApplicable,
        })
    }
    /// Returns backend label for operator output.
    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "SQLite",
            Self::Postgres(_) => "PostgreSQL",
            Self::MySql(_) => "MySQL",
            Self::Redis(_) => "Redis",
        }
    }
}
impl QueueStore for ConfiguredQueue {
    async fn enqueue(&self, job: &Job) -> Result<(), QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.enqueue(job).await,
            Self::Postgres(q) => q.enqueue(job).await,
            Self::MySql(q) => q.enqueue(job).await,
            Self::Redis(q) => q.enqueue(job).await,
        }
    }
    async fn claim_next(
        &self,
        worker: WorkerId,
        lease: LeaseId,
        duration: Duration,
        now: OffsetDateTime,
    ) -> Result<Option<ClaimedJob>, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.claim_next(worker, lease, duration, now).await,
            Self::Postgres(q) => q.claim_next(worker, lease, duration, now).await,
            Self::MySql(q) => q.claim_next(worker, lease, duration, now).await,
            Self::Redis(q) => q.claim_next(worker, lease, duration, now).await,
        }
    }
    async fn renew(
        &self,
        job: JobId,
        lease: LeaseId,
        duration: Duration,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.renew(job, lease, duration, now).await,
            Self::Postgres(q) => q.renew(job, lease, duration, now).await,
            Self::MySql(q) => q.renew(job, lease, duration, now).await,
            Self::Redis(q) => q.renew(job, lease, duration, now).await,
        }
    }
    async fn complete(
        &self,
        job: JobId,
        lease: LeaseId,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.complete(job, lease, now).await,
            Self::Postgres(q) => q.complete(job, lease, now).await,
            Self::MySql(q) => q.complete(job, lease, now).await,
            Self::Redis(q) => q.complete(job, lease, now).await,
        }
    }
    async fn retry(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        available_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<RetryOutcome, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.retry(job, lease, failure, available_at, now).await,
            Self::Postgres(q) => q.retry(job, lease, failure, available_at, now).await,
            Self::MySql(q) => q.retry(job, lease, failure, available_at, now).await,
            Self::Redis(q) => q.retry(job, lease, failure, available_at, now).await,
        }
    }
    async fn fail(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.fail(job, lease, failure, now).await,
            Self::Postgres(q) => q.fail(job, lease, failure, now).await,
            Self::MySql(q) => q.fail(job, lease, failure, now).await,
            Self::Redis(q) => q.fail(job, lease, failure, now).await,
        }
    }
    async fn list(&self, options: QueueListOptions) -> Result<Vec<Job>, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.list(options).await,
            Self::Postgres(q) => q.list(options).await,
            Self::MySql(q) => q.list(options).await,
            Self::Redis(q) => q.list(options).await,
        }
    }
    async fn retry_failed(
        &self,
        job: JobId,
        now: OffsetDateTime,
    ) -> Result<RetryFailedOutcome, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.retry_failed(job, now).await,
            Self::Postgres(q) => q.retry_failed(job, now).await,
            Self::MySql(q) => q.retry_failed(job, now).await,
            Self::Redis(q) => q.retry_failed(job, now).await,
        }
    }
    async fn maintain(
        &self,
        now: OffsetDateTime,
        retention: Duration,
    ) -> Result<QueueMaintenanceReport, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.maintain(now, retention).await,
            Self::Postgres(q) => q.maintain(now, retention).await,
            Self::MySql(q) => q.maintain(now, retention).await,
            Self::Redis(q) => q.maintain(now, retention).await,
        }
    }
}
