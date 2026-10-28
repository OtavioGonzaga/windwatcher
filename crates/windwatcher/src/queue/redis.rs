use crate::{
    ClaimedJob, Job, JobFailure, JobFailureCode, JobId, JobKind, JobLease, JobPayload, JobStatus,
    LeaseId, LeaseMutationOutcome, QueueListOptions, QueueMaintenanceReport, QueueStore,
    QueueStoreError, RetryFailedOutcome, RetryOutcome, WorkerId,
};
use redis::{Script, aio::ConnectionManager};
use std::{collections::HashMap, error::Error, fmt, sync::LazyLock, time::Duration};
use time::OffsetDateTime;

macro_rules! script {
    ($name:ident, $path:literal) => {
        static $name: LazyLock<Script> = LazyLock::new(|| {
            Script::new(concat!(
                include_str!("redis/common.lua"),
                "\n",
                include_str!($path)
            ))
        });
    };
}
script!(ENQUEUE, "redis/enqueue.lua");
script!(CLAIM, "redis/claim.lua");
script!(RENEW, "redis/renew.lua");
script!(COMPLETE, "redis/complete.lua");
script!(RETRY, "redis/retry.lua");
script!(FAIL, "redis/fail.lua");
script!(RETRY_FAILED, "redis/retry_failed.lua");
script!(MAINTENANCE, "redis/maintenance.lua");

/// Redis endpoint and namespace options. Redis Cluster is not supported.
#[derive(Clone)]
pub struct RedisQueueOptions {
    url: String,
    key_prefix: String,
}
impl fmt::Debug for RedisQueueOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedisQueueOptions")
            .field("url", &"[REDACTED]")
            .field("key_prefix", &"[REDACTED]")
            .finish()
    }
}
impl RedisQueueOptions {
    /// Creates options with the `windwatcher:queue` namespace.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            key_prefix: "windwatcher:queue".into(),
        }
    }
    /// Selects an independent namespace, containing 1..=256 UTF-8 bytes and no controls.
    pub fn key_prefix(mut self, value: impl Into<String>) -> Self {
        self.key_prefix = value.into();
        self
    }
    /// Validates the endpoint and namespace without connecting.
    pub fn validate(&self) -> Result<(), RedisQueueError> {
        self.client().map(|_| ())
    }
    fn client(&self) -> Result<redis::Client, RedisQueueError> {
        if self.key_prefix.is_empty()
            || self.key_prefix.len() > 256
            || self.key_prefix.chars().any(char::is_control)
        {
            return Err(RedisQueueError::Configuration(
                "key_prefix must contain 1..=256 bytes without control characters",
            ));
        }
        if !(self.url.starts_with("redis://") || self.url.starts_with("rediss://")) {
            return Err(RedisQueueError::Configuration("invalid Redis URL"));
        }
        redis::Client::open(self.url.as_str())
            .map_err(|_| RedisQueueError::Configuration("invalid Redis URL"))
    }
}
/// Redis infrastructure failure. Debug and Display exclude source details and Redis values.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum RedisQueueError {
    /// Invalid endpoint or namespace.
    #[error("invalid Redis queue configuration: {0}")]
    Configuration(&'static str),
    /// Network, script, or command failure.
    #[error("Redis queue operation failed")]
    Database(#[source] Box<dyn Error + Send + Sync>),
    /// Malformed persisted state or inconsistent indexes.
    #[error("invalid persisted Redis queue job: {0}")]
    Corrupt(&'static str),
}
impl fmt::Debug for RedisQueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl From<RedisQueueError> for QueueStoreError {
    fn from(value: RedisQueueError) -> Self {
        Self::new(value)
    }
}
fn db(value: impl Error + Send + Sync + 'static) -> RedisQueueError {
    RedisQueueError::Database(Box::new(value))
}
fn encode(value: i64) -> String {
    format!("{:016x}", (value as u64) ^ (1_u64 << 63))
}
fn decode(value: &str) -> Result<i64, RedisQueueError> {
    if value.len() != 16
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(RedisQueueError::Corrupt("invalid timestamp encoding"));
    }
    u64::from_str_radix(value, 16)
        .map(|v| (v ^ (1_u64 << 63)) as i64)
        .map_err(|_| RedisQueueError::Corrupt("invalid timestamp"))
}
fn micros(value: OffsetDateTime) -> Result<i64, RedisQueueError> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1000))
        .map_err(|_| RedisQueueError::Corrupt("timestamp out of range"))
}
fn timestamp(value: OffsetDateTime) -> Result<String, RedisQueueError> {
    micros(value).map(encode)
}
fn datetime(value: &str) -> Result<OffsetDateTime, RedisQueueError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(decode(value)?) * 1000)
        .map_err(|_| RedisQueueError::Corrupt("timestamp out of range"))
}
fn status(value: JobStatus) -> &'static str {
    match value {
        JobStatus::Pending => "pending",
        JobStatus::Leased => "leased",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
    }
}
fn applied(value: i64) -> LeaseMutationOutcome {
    if value == 1 {
        LeaseMutationOutcome::Applied
    } else {
        LeaseMutationOutcome::LeaseLost
    }
}
type Hash = HashMap<String, String>;
fn required<'a>(row: &'a Hash, field: &str) -> Result<&'a str, RedisQueueError> {
    row.get(field)
        .map(String::as_str)
        .ok_or(RedisQueueError::Corrupt("missing job field"))
}
fn parse<T: std::str::FromStr>(value: &str) -> Result<T, RedisQueueError> {
    value
        .parse()
        .map_err(|_| RedisQueueError::Corrupt("invalid job field"))
}
fn read_job(row: &Hash) -> Result<Job, RedisQueueError> {
    let state = match required(row, "status")? {
        "pending" => JobStatus::Pending,
        "leased" => JobStatus::Leased,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        _ => return Err(RedisQueueError::Corrupt("unknown status")),
    };
    let payload = serde_json::from_str::<serde_json::Value>(required(row, "payload_json")?)
        .ok()
        .and_then(|v| v.as_object().cloned());
    let invalid = payload.is_none();
    let lease = match (
        row.get("lease_id"),
        row.get("lease_owner"),
        row.get("lease_expires_at"),
    ) {
        (Some(l), Some(w), Some(t)) => Some(JobLease::new(parse(l)?, parse(w)?, datetime(t)?)),
        (None, None, None) => None,
        _ => return Err(RedisQueueError::Corrupt("incoherent lease")),
    };
    let failure = match (row.get("last_error_code"), row.get("last_error_message")) {
        (Some(c), Some(m)) => Some(
            JobFailure::new(
                JobFailureCode::new(c.clone())
                    .map_err(|_| RedisQueueError::Corrupt("invalid failure code"))?,
                m.clone(),
            )
            .map_err(|_| RedisQueueError::Corrupt("invalid failure"))?,
        ),
        (None, None) => None,
        _ => return Err(RedisQueueError::Corrupt("incoherent failure")),
    };
    let optional_time = |name: &str| row.get(name).map(|v| datetime(v)).transpose();
    let mut job = Job::restore(
        parse(required(row, "id")?)?,
        JobKind::new(required(row, "kind")?)
            .map_err(|_| RedisQueueError::Corrupt("invalid kind"))?,
        parse(required(row, "payload_version")?)?,
        JobPayload::new(payload.unwrap_or_default()),
        state,
        datetime(required(row, "available_at")?)?,
        parse(required(row, "attempts")?)?,
        parse(required(row, "max_attempts")?)?,
        lease,
        failure,
        datetime(required(row, "created_at")?)?,
        optional_time("completed_at")?,
        optional_time("failed_at")?,
    )
    .map_err(|_| RedisQueueError::Corrupt("inconsistent job state"))?;
    job.payload_invalid = invalid;
    Ok(job)
}
/// Hash/index Redis queue. Durability depends on Redis persistence and eviction settings.
#[derive(Clone)]
pub struct RedisQueue {
    connection: ConnectionManager,
    prefix: String,
}
impl fmt::Debug for RedisQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedisQueue").finish_non_exhaustive()
    }
}
impl RedisQueue {
    /// Opens a reconnecting multiplexed connection. Redis has no Windwatcher migrations.
    pub async fn connect(options: RedisQueueOptions) -> Result<Self, RedisQueueError> {
        let connection = ConnectionManager::new(options.client()?)
            .await
            .map_err(db)?;
        Ok(Self {
            connection,
            prefix: options.key_prefix,
        })
    }
    async fn maintenance_phase(&self, phase: u8, cutoff: String) -> Result<u64, RedisQueueError> {
        let mut cursor = String::new();
        let mut total = 0;
        loop {
            let (count, next): (String, String) = MAINTENANCE
                .key(&self.prefix)
                .arg(phase)
                .arg(&cutoff)
                .arg(&cursor)
                .invoke_async(&mut self.connection.clone())
                .await
                .map_err(db)?;
            total += parse::<u64>(&count)?;
            if next.is_empty() {
                return Ok(total);
            }
            cursor = next;
        }
    }
}
impl QueueStore for RedisQueue {
    async fn enqueue(&self, job: &Job) -> Result<(), QueueStoreError> {
        let payload = serde_json::to_string(job.payload()).map_err(db)?;
        let _: i64 = ENQUEUE
            .key(&self.prefix)
            .arg(job.id().to_string())
            .arg(job.kind().as_str())
            .arg(job.payload_version())
            .arg(payload)
            .arg(timestamp(job.available_at())?)
            .arg(job.max_attempts())
            .arg(timestamp(job.created_at())?)
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(db)?;
        Ok(())
    }
    async fn claim_next(
        &self,
        worker: WorkerId,
        lease: LeaseId,
        duration: Duration,
        now: OffsetDateTime,
    ) -> Result<Option<ClaimedJob>, QueueStoreError> {
        let expires = JobLease::expiration(now, duration).map_err(QueueStoreError::new)?;
        let row: Hash = CLAIM
            .key(&self.prefix)
            .arg(lease.to_string())
            .arg(worker.to_string())
            .arg(timestamp(now)?)
            .arg(timestamp(expires)?)
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(db)?;
        if row.is_empty() {
            return Ok(None);
        }
        let job = read_job(&row)?;
        let lease = job
            .lease()
            .cloned()
            .ok_or(RedisQueueError::Corrupt("missing lease"))?;
        Ok(Some(
            ClaimedJob::new(job, lease).map_err(|_| RedisQueueError::Corrupt("invalid claim"))?,
        ))
    }
    async fn renew(
        &self,
        job: JobId,
        lease: LeaseId,
        duration: Duration,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        let expires = JobLease::expiration(now, duration).map_err(QueueStoreError::new)?;
        let result = RENEW
            .key(&self.prefix)
            .arg(job.to_string())
            .arg(lease.to_string())
            .arg(timestamp(now)?)
            .arg(timestamp(expires)?)
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(db)?;
        Ok(applied(result))
    }
    async fn complete(
        &self,
        job: JobId,
        lease: LeaseId,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        let result = COMPLETE
            .key(&self.prefix)
            .arg(job.to_string())
            .arg(lease.to_string())
            .arg(timestamp(now)?)
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(db)?;
        Ok(applied(result))
    }
    async fn retry(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        available_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<RetryOutcome, QueueStoreError> {
        let result: i64 = RETRY
            .key(&self.prefix)
            .arg(job.to_string())
            .arg(lease.to_string())
            .arg(timestamp(now)?)
            .arg(failure.code().as_str())
            .arg(failure.message())
            .arg(timestamp(available_at)?)
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(db)?;
        Ok(match result {
            1 => RetryOutcome::Scheduled,
            2 => RetryOutcome::Exhausted,
            _ => RetryOutcome::LeaseLost,
        })
    }
    async fn fail(
        &self,
        job: JobId,
        lease: LeaseId,
        failure: &JobFailure,
        now: OffsetDateTime,
    ) -> Result<LeaseMutationOutcome, QueueStoreError> {
        let result = FAIL
            .key(&self.prefix)
            .arg(job.to_string())
            .arg(lease.to_string())
            .arg(timestamp(now)?)
            .arg(failure.code().as_str())
            .arg(failure.message())
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(db)?;
        Ok(applied(result))
    }
    async fn list(&self, options: QueueListOptions) -> Result<Vec<Job>, QueueStoreError> {
        let index = match options.status() {
            Some(s) => format!("{}:index:status:{}", self.prefix, status(s)),
            None => format!("{}:index:all", self.prefix),
        };
        let members: Vec<String> = redis::cmd("ZREVRANGEBYLEX")
            .arg(index)
            .arg("+")
            .arg("-")
            .arg("LIMIT")
            .arg(0)
            .arg(options.limit())
            .query_async(&mut self.connection.clone())
            .await
            .map_err(db)?;
        if members.is_empty() {
            return Ok(Vec::new());
        }
        let mut pipeline = redis::pipe();
        let mut ids = Vec::new();
        for member in &members {
            let (time, id) = member
                .split_once(':')
                .ok_or(RedisQueueError::Corrupt("invalid index member"))?;
            decode(time)?;
            let id: JobId = parse(id)?;
            pipeline
                .cmd("HGETALL")
                .arg(format!("{}:job:{id}", self.prefix));
            ids.push(id);
        }
        let rows: Vec<Hash> = pipeline
            .query_async(&mut self.connection.clone())
            .await
            .map_err(db)?;
        let mut jobs = Vec::with_capacity(rows.len());
        for ((row, id), member) in rows.iter().zip(ids).zip(members) {
            let job = read_job(row)?;
            if job.id() != id
                || format!("{}:{id}", timestamp(job.created_at())?) != member
                || options.status().is_some_and(|s| s != job.status())
            {
                return Err(RedisQueueError::Corrupt("inconsistent listing index").into());
            }
            jobs.push(job);
        }
        Ok(jobs)
    }
    async fn retry_failed(
        &self,
        job: JobId,
        now: OffsetDateTime,
    ) -> Result<RetryFailedOutcome, QueueStoreError> {
        let result: i64 = RETRY_FAILED
            .key(&self.prefix)
            .arg(job.to_string())
            .arg(timestamp(now)?)
            .invoke_async(&mut self.connection.clone())
            .await
            .map_err(db)?;
        Ok(match result {
            2 => RetryFailedOutcome::Retried,
            1 => RetryFailedOutcome::NotFailed,
            _ => RetryFailedOutcome::NotFound,
        })
    }
    async fn maintain(
        &self,
        now: OffsetDateTime,
        retention: Duration,
    ) -> Result<QueueMaintenanceReport, QueueStoreError> {
        let now_us = micros(now)?;
        let retention = i64::try_from(retention.as_micros()).unwrap_or(i64::MAX);
        let cutoff = now_us.checked_sub(retention).unwrap_or(i64::MIN);
        let exhausted = self.maintenance_phase(0, encode(now_us)).await?;
        let pruned = self.maintenance_phase(1, encode(cutoff)).await?;
        Ok(QueueMaintenanceReport::new(exhausted, pruned))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_sortable_signed_microseconds() {
        let values = [
            i64::MIN,
            i64::MIN + 1,
            -1_000_001,
            -1,
            0,
            1,
            999_999,
            1_000_000,
            i64::MAX - 1,
            i64::MAX,
        ];
        let encoded: Vec<_> = values.iter().map(|v| encode(*v)).collect();
        assert!(encoded.windows(2).all(|v| v[0] < v[1]));
        for (value, key) in values.iter().zip(encoded) {
            assert_eq!(*value, decode(&key).unwrap());
        }
        assert_eq!(
            timestamp(OffsetDateTime::from_unix_timestamp_nanos(-1).unwrap()).unwrap(),
            encode(-1)
        );
        let a = JobId::new();
        let b = JobId::new();
        assert_eq!(
            format!("{}:{a}", encode(0)) < format!("{}:{b}", encode(0)),
            a < b
        );
        assert!(decode("bad").is_err());
    }
}
