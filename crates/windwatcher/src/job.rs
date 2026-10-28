use std::fmt;

use time::OffsetDateTime;

use crate::{JobId, LeaseId, WorkerId};

/// A validation error for portable queue values.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum JobValueError {
    /// A job kind or failure code had invalid syntax.
    #[error("value must contain 1..=128 lowercase ASCII letters, digits, '.', '_' or '-'")]
    InvalidCode,
    /// A payload root was not a JSON object.
    #[error("job payload must be a JSON object")]
    PayloadNotObject,
    /// Payload size exceeds the configured maximum.
    #[error("job payload exceeds the configured byte limit")]
    PayloadTooLarge,
    /// Persisted or constructed job state was incoherent.
    #[error("job state is inconsistent")]
    InvalidJobState,
    /// Failure message exceeds the portable limit.
    #[error("failure message exceeds 4096 UTF-8 bytes")]
    FailureMessageTooLong,
}

fn validate_code(value: &str) -> Result<(), JobValueError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
    {
        return Err(JobValueError::InvalidCode);
    }
    Ok(())
}

/// Open, validated job-kind identifier; kinds are not a closed enum.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobKind(String);
impl JobKind {
    /// Creates a job kind using portable lowercase ASCII syntax.
    pub fn new(value: impl Into<String>) -> Result<Self, JobValueError> {
        let value = value.into();
        validate_code(&value)?;
        Ok(Self(value))
    }
    /// Returns the kind's stable textual value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for JobKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A JSON-object job payload, independent of adapter serialization.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct JobPayload(serde_json::Map<String, serde_json::Value>);
impl JobPayload {
    /// Creates a payload from an object map.
    pub fn new(fields: serde_json::Map<String, serde_json::Value>) -> Self {
        Self(fields)
    }
    /// Borrows the payload object fields.
    pub fn as_map(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.0
    }
    /// Consumes the payload and returns its object fields.
    pub fn into_map(self) -> serde_json::Map<String, serde_json::Value> {
        self.0
    }
    /// Returns serialized UTF-8 size for the portable payload policy.
    pub fn serialized_size(&self) -> Result<usize, serde_json::Error> {
        serde_json::to_vec(&self.0).map(|v| v.len())
    }
}
impl TryFrom<serde_json::Value> for JobPayload {
    type Error = JobValueError;
    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Object(map) => Ok(Self(map)),
            _ => Err(JobValueError::PayloadNotObject),
        }
    }
}

/// The four persisted queue states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Awaiting availability or retry.
    Pending,
    /// Claimed under a renewable lease.
    Leased,
    /// Successfully handled.
    Completed,
    /// Terminal handler failure.
    Failed,
}

/// Validated machine-readable handler failure code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobFailureCode(String);
impl JobFailureCode {
    /// Creates a validated failure code.
    pub fn new(value: impl Into<String>) -> Result<Self, JobValueError> {
        let value = value.into();
        validate_code(&value)?;
        Ok(Self(value))
    }
    /// Returns the stable code.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for JobFailureCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A safe, bounded diagnostic from a handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobFailure {
    code: JobFailureCode,
    message: String,
}
impl JobFailure {
    /// Creates a failure with a message no larger than 4096 UTF-8 bytes.
    pub fn new(code: JobFailureCode, message: impl Into<String>) -> Result<Self, JobValueError> {
        let message = message.into();
        if message.len() > 4096 {
            return Err(JobValueError::FailureMessageTooLong);
        }
        Ok(Self { code, message })
    }
    /// Returns the machine-readable code.
    pub fn code(&self) -> &JobFailureCode {
        &self.code
    }
    /// Returns the diagnostic message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Current claim authority for a leased job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobLease {
    id: LeaseId,
    worker_id: WorkerId,
    expires_at: OffsetDateTime,
}
impl JobLease {
    /// Constructs a lease value.
    pub fn new(id: LeaseId, worker_id: WorkerId, expires_at: OffsetDateTime) -> Self {
        Self {
            id,
            worker_id,
            expires_at,
        }
    }
    /// Returns the fencing token.
    pub fn id(&self) -> LeaseId {
        self.id
    }
    /// Returns worker attribution.
    pub fn worker_id(&self) -> WorkerId {
        self.worker_id
    }
    /// Returns lease expiration time.
    pub fn expires_at(&self) -> OffsetDateTime {
        self.expires_at
    }
}

/// Durable job state returned by queue storage.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub(crate) id: JobId,
    pub(crate) kind: JobKind,
    pub(crate) payload_version: u16,
    pub(crate) payload: JobPayload,
    pub(crate) status: JobStatus,
    pub(crate) available_at: OffsetDateTime,
    pub(crate) attempts: u32,
    pub(crate) max_attempts: u32,
    pub(crate) lease: Option<JobLease>,
    pub(crate) last_failure: Option<JobFailure>,
    pub(crate) created_at: OffsetDateTime,
    pub(crate) completed_at: Option<OffsetDateTime>,
    pub(crate) failed_at: Option<OffsetDateTime>,
    pub(crate) payload_invalid: bool,
}
impl Job {
    /// Restores persisted state while enforcing structural invariants.
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: JobId,
        kind: JobKind,
        payload_version: u16,
        payload: JobPayload,
        status: JobStatus,
        available_at: OffsetDateTime,
        attempts: u32,
        max_attempts: u32,
        lease: Option<JobLease>,
        last_failure: Option<JobFailure>,
        created_at: OffsetDateTime,
        completed_at: Option<OffsetDateTime>,
        failed_at: Option<OffsetDateTime>,
    ) -> Result<Self, JobValueError> {
        let coherent = payload_version > 0
            && max_attempts > 0
            && attempts <= max_attempts
            && match status {
                JobStatus::Pending => {
                    lease.is_none()
                        && attempts < max_attempts
                        && completed_at.is_none()
                        && failed_at.is_none()
                }
                JobStatus::Leased => {
                    lease.is_some() && attempts > 0 && completed_at.is_none() && failed_at.is_none()
                }
                JobStatus::Completed => {
                    lease.is_none()
                        && attempts > 0
                        && completed_at.is_some()
                        && failed_at.is_none()
                        && last_failure.is_none()
                }
                JobStatus::Failed => {
                    lease.is_none()
                        && attempts > 0
                        && failed_at.is_some()
                        && completed_at.is_none()
                        && last_failure.is_some()
                }
            };
        if !coherent {
            return Err(JobValueError::InvalidJobState);
        }
        Ok(Self {
            id,
            kind,
            payload_version,
            payload,
            status,
            available_at,
            attempts,
            max_attempts,
            lease,
            last_failure,
            created_at,
            completed_at,
            failed_at,
            payload_invalid: false,
        })
    }
    /// Returns job identifier.
    pub fn id(&self) -> JobId {
        self.id
    }
    /// Returns job kind.
    pub fn kind(&self) -> &JobKind {
        &self.kind
    }
    /// Returns payload schema version.
    pub fn payload_version(&self) -> u16 {
        self.payload_version
    }
    /// Returns object payload.
    pub fn payload(&self) -> &JobPayload {
        &self.payload
    }
    /// Returns persisted status.
    pub fn status(&self) -> JobStatus {
        self.status
    }
    /// Returns next availability time.
    pub fn available_at(&self) -> OffsetDateTime {
        self.available_at
    }
    /// Returns successful claim count.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
    /// Returns persisted attempt limit.
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }
    /// Returns current lease if leased.
    pub fn lease(&self) -> Option<&JobLease> {
        self.lease.as_ref()
    }
    /// Returns last handler failure, if any.
    pub fn last_failure(&self) -> Option<&JobFailure> {
        self.last_failure.as_ref()
    }
    /// Returns creation time.
    pub fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
    /// Returns completion time.
    pub fn completed_at(&self) -> Option<OffsetDateTime> {
        self.completed_at
    }
    /// Returns terminal failure time.
    pub fn failed_at(&self) -> Option<OffsetDateTime> {
        self.failed_at
    }
    /// Returns whether the adapter restored malformed persisted payload JSON.
    pub fn payload_invalid(&self) -> bool {
        self.payload_invalid
    }
}

/// Job together with the lease proving this claim's authority.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimedJob {
    job: Job,
    lease: JobLease,
    payload_invalid: bool,
}
impl ClaimedJob {
    /// Creates a claimed view when the job and lease agree.
    pub fn new(job: Job, lease: JobLease) -> Result<Self, JobValueError> {
        if job.status != JobStatus::Leased || job.lease.as_ref() != Some(&lease) {
            return Err(JobValueError::InvalidJobState);
        }
        let payload_invalid = job.payload_invalid;
        Ok(Self {
            job,
            lease,
            payload_invalid,
        })
    }
    #[cfg(feature = "queue-sqlite")]
    pub(crate) fn invalid_payload(job: Job, lease: JobLease) -> Result<Self, JobValueError> {
        let mut claimed = Self::new(job, lease)?;
        claimed.payload_invalid = true;
        Ok(claimed)
    }
    /// Returns the job state.
    pub fn job(&self) -> &Job {
        &self.job
    }
    /// Returns authoritative claim lease.
    pub fn lease(&self) -> &JobLease {
        &self.lease
    }
    /// Returns whether the adapter found malformed persisted payload JSON.
    pub fn payload_invalid(&self) -> bool {
        self.payload_invalid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_open_kinds_and_failure_codes() {
        assert!(JobKind::new("push.deliver-v1").is_ok());
        for invalid in ["", "UPPER", "a/b", "é", &"a".repeat(129)] {
            assert!(JobKind::new(invalid).is_err(), "{invalid:?}");
        }
        assert!(JobFailureCode::new("http.timeout").is_ok());
    }

    #[test]
    fn payload_accepts_only_object_roots() {
        let payload = JobPayload::try_from(serde_json::json!({"nested": [1, null]})).unwrap();
        assert_eq!(payload.as_map().len(), 1);
        for value in [
            serde_json::json!(null),
            serde_json::json!([]),
            serde_json::json!("x"),
        ] {
            assert_eq!(
                JobPayload::try_from(value),
                Err(JobValueError::PayloadNotObject)
            );
        }
    }

    #[test]
    fn failure_messages_are_bounded_by_utf8_bytes() {
        let code = JobFailureCode::new("provider.unavailable").unwrap();
        assert!(JobFailure::new(code.clone(), "x".repeat(4096)).is_ok());
        assert_eq!(
            JobFailure::new(code, "x".repeat(4097)),
            Err(JobValueError::FailureMessageTooLong)
        );
    }

    #[test]
    fn job_restore_rejects_incoherent_state() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let payload = JobPayload::try_from(serde_json::json!({})).unwrap();
        let result = Job::restore(
            JobId::new(),
            JobKind::new("test.run").unwrap(),
            1,
            payload,
            JobStatus::Leased,
            now,
            1,
            3,
            None,
            None,
            now,
            None,
            None,
        );
        assert_eq!(result, Err(JobValueError::InvalidJobState));
    }
}
