use time::OffsetDateTime;

use crate::{Job, JobId, JobKind, JobPayload, JobStatus, QueueStore, QueueStoreError};

/// Enqueue validation and defaults shared by all adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JobPolicy {
    max_payload_bytes: usize,
    default_max_attempts: u32,
}
impl JobPolicy {
    /// Creates a policy with positive payload and attempt limits.
    pub fn new(
        max_payload_bytes: usize,
        default_max_attempts: u32,
    ) -> Result<Self, JobPolicyError> {
        if max_payload_bytes == 0 {
            return Err(JobPolicyError::InvalidPayloadLimit);
        }
        if default_max_attempts == 0 {
            return Err(JobPolicyError::InvalidAttempts);
        }
        Ok(Self {
            max_payload_bytes,
            default_max_attempts,
        })
    }
    /// Returns maximum serialized payload bytes.
    pub fn max_payload_bytes(&self) -> usize {
        self.max_payload_bytes
    }
    /// Returns default attempt limit.
    pub fn default_max_attempts(&self) -> u32 {
        self.default_max_attempts
    }
}
impl Default for JobPolicy {
    fn default() -> Self {
        Self {
            max_payload_bytes: 262_144,
            default_max_attempts: 5,
        }
    }
}

/// Invalid enqueue policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum JobPolicyError {
    /// Payload maximum must be positive.
    #[error("max_payload_bytes must be greater than zero")]
    InvalidPayloadLimit,
    /// Attempts must be positive.
    #[error("max_attempts must be greater than zero")]
    InvalidAttempts,
}

/// Per-job scheduling overrides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnqueueOptions {
    available_at: Option<OffsetDateTime>,
    max_attempts: Option<u32>,
}
impl EnqueueOptions {
    /// Creates default options (available now, policy attempts).
    pub fn new() -> Self {
        Self::default()
    }
    /// Sets delayed availability.
    pub fn available_at(mut self, value: OffsetDateTime) -> Self {
        self.available_at = Some(value);
        self
    }
    /// Sets an attempt limit for this job.
    pub fn max_attempts(mut self, value: u32) -> Self {
        self.max_attempts = Some(value);
        self
    }
}

/// Validates and persists a newly scheduled job.
pub struct EnqueueJob<'a, S> {
    store: &'a S,
    policy: JobPolicy,
}
impl<'a, S: QueueStore> EnqueueJob<'a, S> {
    /// Creates an enqueue operation.
    pub fn new(store: &'a S, policy: JobPolicy) -> Self {
        Self { store, policy }
    }
    /// Validates and persists a new pending job at the caller-supplied time.
    /// Returned and persisted timestamps are floored to UTC microseconds.
    pub async fn execute(
        &self,
        kind: JobKind,
        payload_version: u16,
        payload: JobPayload,
        options: EnqueueOptions,
        now: OffsetDateTime,
    ) -> Result<Job, EnqueueJobError> {
        if payload_version == 0 {
            return Err(EnqueueJobError::InvalidVersion);
        }
        let size = payload
            .serialized_size()
            .map_err(|_| EnqueueJobError::InvalidPayload)?;
        if size > self.policy.max_payload_bytes {
            return Err(EnqueueJobError::PayloadTooLarge);
        }
        let max_attempts = options
            .max_attempts
            .unwrap_or(self.policy.default_max_attempts);
        if max_attempts == 0 {
            return Err(EnqueueJobError::InvalidAttempts);
        }
        let job = Job::restore(
            JobId::new(),
            kind,
            payload_version,
            payload,
            JobStatus::Pending,
            options.available_at.unwrap_or(now),
            0,
            max_attempts,
            None,
            None,
            now,
            None,
            None,
        )
        .map_err(|_| EnqueueJobError::InvalidPayload)?;
        self.store
            .enqueue(&job)
            .await
            .map_err(EnqueueJobError::Store)?;
        Ok(job)
    }
}

/// Enqueue operation failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EnqueueJobError {
    /// Version must start at one.
    #[error("payload_version must be at least one")]
    InvalidVersion,
    /// Payload exceeded policy.
    #[error("job payload exceeds configured maximum")]
    PayloadTooLarge,
    /// Payload couldn't serialize.
    #[error("job payload is invalid")]
    InvalidPayload,
    /// Attempt override must be positive.
    #[error("max_attempts must be greater than zero")]
    InvalidAttempts,
    /// Persistence failed.
    #[error(transparent)]
    Store(#[from] QueueStoreError),
}
