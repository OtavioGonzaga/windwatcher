use std::{error::Error, future::Future, time::Duration};

use time::OffsetDateTime;

use crate::{ClaimedJob, Job, JobFailure, JobId, JobStatus, LeaseId, WorkerId};

/// Operational failures from a queue adapter.
#[derive(Debug, thiserror::Error)]
#[error("queue storage operation failed")]
pub struct QueueStoreError(#[source] Box<dyn Error + Send + Sync>);
impl QueueStoreError {
    /// Wraps an adapter-specific operational error.
    pub fn new(error: impl Error + Send + Sync + 'static) -> Self {
        Self(Box::new(error))
    }
}

/// Result of a fenced lease mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseMutationOutcome {
    /// The mutation was applied.
    Applied,
    /// This claim no longer has authority.
    LeaseLost,
}

/// Result of retry scheduling using persisted attempt counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryOutcome {
    /// A subsequent attempt was scheduled.
    Scheduled,
    /// Attempt limit was reached and job failed.
    Exhausted,
    /// This claim no longer has authority.
    LeaseLost,
}

/// Result of manually retrying a failed job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryFailedOutcome {
    /// Job was reset to pending.
    Retried,
    /// No job had the supplied ID.
    NotFound,
    /// Job exists but is not failed.
    NotFailed,
}

/// Bounded maintenance counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueMaintenanceReport {
    exhausted_jobs_failed: u64,
    completed_jobs_pruned: u64,
}
impl QueueMaintenanceReport {
    /// Creates a maintenance result.
    pub fn new(exhausted_jobs_failed: u64, completed_jobs_pruned: u64) -> Self {
        Self {
            exhausted_jobs_failed,
            completed_jobs_pruned,
        }
    }
    /// Returns exhausted leases finalized as failed.
    pub fn exhausted_jobs_failed(&self) -> u64 {
        self.exhausted_jobs_failed
    }
    /// Returns pruned completed jobs.
    pub fn completed_jobs_pruned(&self) -> u64 {
        self.completed_jobs_pruned
    }
}

/// Operational job listing filter and bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueListOptions {
    status: Option<JobStatus>,
    limit: u32,
}

/// A job-list limit outside the portable inclusive range `1..=100`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("job list limit must be between 1 and 100")]
pub struct QueueListOptionsError;

impl QueueListOptions {
    /// Creates listing options, rejecting limits outside `1..=100`.
    pub fn new(status: Option<JobStatus>, limit: u32) -> Result<Self, QueueListOptionsError> {
        if !(1..=100).contains(&limit) {
            return Err(QueueListOptionsError);
        }
        Ok(Self { status, limit })
    }
    /// Returns optional state filter.
    pub fn status(&self) -> Option<JobStatus> {
        self.status
    }
    /// Returns requested result limit.
    pub fn limit(&self) -> u32 {
        self.limit
    }
}

/// Portable durable queue persistence contract.
///
/// All time-sensitive operations receive an explicit UTC `now` from the caller.
/// Adapters floor timestamps (including `now`) and durations to microseconds.
/// Claim/renew use [`crate::JobLease::expiration`] and reject durations below
/// one microsecond or expiration overflow before changing any state.
/// Implementations provide at-least-once delivery and make no processing-order
/// guarantee. Every post-claim mutation is fenced by `LeaseId`.
pub trait QueueStore: Sync {
    /// Persists a newly created pending job.
    fn enqueue(&self, job: &Job) -> impl Future<Output = Result<(), QueueStoreError>> + Send;
    /// Atomically claims one eligible job, incrementing its attempt count.
    fn claim_next(
        &self,
        worker: WorkerId,
        lease: LeaseId,
        duration: Duration,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<ClaimedJob>, QueueStoreError>> + Send;
    /// Renews an unexpired authoritative claim.
    fn renew(
        &self,
        job: JobId,
        lease: LeaseId,
        duration: Duration,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<LeaseMutationOutcome, QueueStoreError>> + Send;
    /// Completes an authoritative claim.
    fn complete(
        &self,
        job: JobId,
        lease: LeaseId,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<LeaseMutationOutcome, QueueStoreError>> + Send;
    /// Schedules a retry, atomically failing exhausted jobs.
    fn retry(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        available_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<RetryOutcome, QueueStoreError>> + Send;
    /// Permanently fails an authoritative claim.
    fn fail(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<LeaseMutationOutcome, QueueStoreError>> + Send;
    /// Lists jobs by `created_at DESC, id DESC`, with optional status filtering.
    /// This operator presentation order makes no execution-order guarantee.
    fn list(
        &self,
        options: QueueListOptions,
    ) -> impl Future<Output = Result<Vec<Job>, QueueStoreError>> + Send;
    /// Resets only failed jobs to pending with the same identity and policy.
    fn retry_failed(
        &self,
        job: JobId,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<RetryFailedOutcome, QueueStoreError>> + Send;
    /// Finalizes exhausted expired leases and prunes old completed jobs.
    fn maintain(
        &self,
        now: OffsetDateTime,
        completed_retention: Duration,
    ) -> impl Future<Output = Result<QueueMaintenanceReport, QueueStoreError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_limits_are_validated_before_reaching_any_adapter() {
        for limit in [0, 101, u32::MAX] {
            assert_eq!(
                QueueListOptions::new(None, limit),
                Err(QueueListOptionsError)
            );
        }
        for limit in [1, 100] {
            let options = QueueListOptions::new(Some(JobStatus::Failed), limit).unwrap();
            assert_eq!(options.limit(), limit);
            assert_eq!(options.status(), Some(JobStatus::Failed));
        }
    }
}
