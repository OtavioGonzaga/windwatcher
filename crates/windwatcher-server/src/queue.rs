use std::time::Duration;

use time::OffsetDateTime;
use windwatcher::{
    ClaimedJob, Job, JobFailure, JobId, LeaseId, LeaseMutationOutcome, QueueListOptions,
    QueueMaintenanceReport, QueueStore, QueueStoreError, RetryFailedOutcome, RetryOutcome,
    SqliteQueue, SqliteQueueError, SqliteQueueOptions, WorkerId,
};

/// Concrete server-side queue adapter dispatch.
#[derive(Clone, Debug)]
pub enum ConfiguredQueue {
    /// Official SQLite queue adapter.
    Sqlite(SqliteQueue),
}
impl ConfiguredQueue {
    /// Opens the configured queue without applying migrations.
    pub async fn connect(options: SqliteQueueOptions) -> Result<Self, SqliteQueueError> {
        Ok(Self::Sqlite(SqliteQueue::connect(options).await?))
    }
    /// Applies the adapter's embedded migrations.
    pub async fn migrate(&self) -> Result<(), SqliteQueueError> {
        match self {
            Self::Sqlite(queue) => queue.migrate().await,
        }
    }
    /// Inspects adapter migration status.
    pub async fn migration_status(
        &self,
    ) -> Result<windwatcher::QueueMigrationStatus, SqliteQueueError> {
        match self {
            Self::Sqlite(queue) => queue.migration_status().await,
        }
    }
    /// Returns backend label for operator output.
    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "SQLite",
        }
    }
}
impl QueueStore for ConfiguredQueue {
    async fn enqueue(&self, job: &Job) -> Result<(), QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.enqueue(job).await,
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
        }
    }
    async fn list(&self, options: QueueListOptions) -> Result<Vec<Job>, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.list(options).await,
        }
    }
    async fn retry_failed(
        &self,
        job: JobId,
        now: OffsetDateTime,
    ) -> Result<RetryFailedOutcome, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.retry_failed(job, now).await,
        }
    }
    async fn maintain(
        &self,
        now: OffsetDateTime,
        retention: Duration,
    ) -> Result<QueueMaintenanceReport, QueueStoreError> {
        match self {
            Self::Sqlite(q) => q.maintain(now, retention).await,
        }
    }
}
