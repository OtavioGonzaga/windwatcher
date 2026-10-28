#![cfg(feature = "queue-sqlite")]

use std::time::Duration;

use tempfile::TempDir;
use time::OffsetDateTime;
use windwatcher::{
    EnqueueJob, EnqueueOptions, JobFailure, JobFailureCode, JobKind, JobPayload, JobPolicy,
    JobStatus, LeaseId, QueueListOptions, QueueStore, RetryOutcome, SqliteQueue,
    SqliteQueueOptions, WorkerId,
};

#[path = "support/queue_contract.rs"]
mod queue_contract;

async fn queue() -> (TempDir, SqliteQueue) {
    let directory = TempDir::new().unwrap();
    let url = format!("sqlite://{}", directory.path().join("queue.db").display());
    let queue = SqliteQueue::connect(SqliteQueueOptions::new(url))
        .await
        .unwrap();
    assert_eq!(queue.migration_status().await.unwrap().pending(), 1);
    queue.migrate().await.unwrap();
    assert!(queue.check_schema().await.unwrap().is_current());
    (directory, queue)
}

fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
}
fn payload() -> JobPayload {
    JobPayload::try_from(serde_json::json!({"resource_id":"abc"})).unwrap()
}

#[tokio::test]
async fn queue_database_has_explicit_migrations_and_fenced_lifecycle() {
    let (_directory, queue) = queue().await;
    let initial = now();
    let job = EnqueueJob::new(&queue, JobPolicy::default())
        .execute(
            JobKind::new("test.handle").unwrap(),
            1,
            payload(),
            EnqueueOptions::new().max_attempts(2),
            initial,
        )
        .await
        .unwrap();
    assert_eq!(job.status(), JobStatus::Pending);
    assert_eq!(job.attempts(), 0);
    let worker = WorkerId::new();
    let claim = queue
        .claim_next(worker, LeaseId::new(), Duration::from_secs(60), initial)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claim.job().id(), job.id());
    assert_eq!(claim.job().attempts(), 1);
    assert_eq!(claim.job().status(), JobStatus::Leased);
    let stale = LeaseId::new();
    assert_eq!(
        queue.complete(job.id(), stale, initial).await.unwrap(),
        windwatcher::LeaseMutationOutcome::LeaseLost
    );
    let failure = JobFailure::new(
        JobFailureCode::new("test.retry").unwrap(),
        "safe diagnostic",
    )
    .unwrap();
    assert_eq!(
        queue
            .retry(
                job.id(),
                claim.lease().id(),
                &failure,
                initial + time::Duration::seconds(1),
                initial
            )
            .await
            .unwrap(),
        RetryOutcome::Scheduled
    );
    assert_eq!(
        queue
            .list(QueueListOptions::new(Some(JobStatus::Pending), 10).unwrap())
            .await
            .unwrap()[0]
            .attempts(),
        1
    );
    let second = queue
        .claim_next(
            worker,
            LeaseId::new(),
            Duration::from_secs(60),
            initial + time::Duration::seconds(2),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        queue
            .complete(
                job.id(),
                second.lease().id(),
                initial + time::Duration::seconds(3)
            )
            .await
            .unwrap(),
        windwatcher::LeaseMutationOutcome::Applied
    );
    assert_eq!(
        queue
            .list(QueueListOptions::new(Some(JobStatus::Completed), 10).unwrap())
            .await
            .unwrap()[0]
            .id(),
        job.id()
    );
}

#[tokio::test]
async fn shared_queue_contract_is_satisfied() {
    let (_directory, queue) = queue().await;
    queue_contract::run(&queue).await;
}

#[tokio::test]
async fn exhausted_retry_fails_without_an_extra_claim() {
    let (_directory, queue) = queue().await;
    let at = now();
    let job = EnqueueJob::new(&queue, JobPolicy::default())
        .execute(
            JobKind::new("test.fail").unwrap(),
            1,
            payload(),
            EnqueueOptions::new().max_attempts(1),
            at,
        )
        .await
        .unwrap();
    let claim = queue
        .claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(5), at)
        .await
        .unwrap()
        .unwrap();
    let failure = JobFailure::new(JobFailureCode::new("test.failure").unwrap(), "failed").unwrap();
    assert_eq!(
        queue
            .retry(job.id(), claim.lease().id(), &failure, at, at)
            .await
            .unwrap(),
        RetryOutcome::Exhausted
    );
    assert!(
        queue
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_secs(5),
                at + time::Duration::seconds(1)
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        queue
            .list(QueueListOptions::new(Some(JobStatus::Failed), 10).unwrap())
            .await
            .unwrap()[0]
            .id(),
        job.id()
    );
}

#[tokio::test]
async fn abandoned_final_claim_is_failed_without_periodic_maintenance() {
    let (_directory, queue) = queue().await;
    let at = now();
    let job = EnqueueJob::new(&queue, JobPolicy::default())
        .execute(
            JobKind::new("test.crash").unwrap(),
            1,
            payload(),
            EnqueueOptions::new().max_attempts(1),
            at,
        )
        .await
        .unwrap();
    queue
        .claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(5), at)
        .await
        .unwrap()
        .unwrap();
    assert!(
        queue
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_secs(5),
                at + time::Duration::seconds(5)
            )
            .await
            .unwrap()
            .is_none()
    );
    let failed = queue
        .list(QueueListOptions::new(Some(JobStatus::Failed), 10).unwrap())
        .await
        .unwrap();
    assert_eq!(failed[0].id(), job.id());
    assert_eq!(
        failed[0].last_failure().unwrap().code().as_str(),
        "queue.attempts_exhausted"
    );
}

#[tokio::test]
async fn manual_retry_preserves_identity_and_policy_and_clears_failure() {
    let (_directory, queue) = queue().await;
    let at = now();
    let job = EnqueueJob::new(&queue, JobPolicy::default())
        .execute(
            JobKind::new("test.operator").unwrap(),
            1,
            payload(),
            EnqueueOptions::new().max_attempts(7),
            at,
        )
        .await
        .unwrap();
    let claim = queue
        .claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(5), at)
        .await
        .unwrap()
        .unwrap();
    let failure =
        JobFailure::new(JobFailureCode::new("test.permanent").unwrap(), "diagnostic").unwrap();
    queue
        .fail(job.id(), claim.lease().id(), &failure, at)
        .await
        .unwrap();
    assert_eq!(
        queue
            .retry_failed(job.id(), at + time::Duration::seconds(1))
            .await
            .unwrap(),
        windwatcher::RetryFailedOutcome::Retried
    );
    let retried = queue
        .list(QueueListOptions::new(Some(JobStatus::Pending), 10).unwrap())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(retried.id(), job.id());
    assert_eq!(retried.attempts(), 0);
    assert_eq!(retried.max_attempts(), 7);
    assert!(retried.last_failure().is_none());
    assert_eq!(
        queue.retry_failed(job.id(), at).await.unwrap(),
        windwatcher::RetryFailedOutcome::NotFailed
    );
}

#[tokio::test]
async fn two_queue_instances_cannot_claim_the_same_initial_job() {
    let directory = TempDir::new().unwrap();
    let url = format!("sqlite://{}", directory.path().join("queue.db").display());
    let first = SqliteQueue::connect(SqliteQueueOptions::new(&url))
        .await
        .unwrap();
    first.migrate().await.unwrap();
    let second = SqliteQueue::connect(SqliteQueueOptions::new(&url))
        .await
        .unwrap();
    let at = now();
    let job = EnqueueJob::new(&first, JobPolicy::default())
        .execute(
            JobKind::new("test.race").unwrap(),
            1,
            payload(),
            EnqueueOptions::new(),
            at,
        )
        .await
        .unwrap();
    let (left, right) = tokio::join!(
        first.claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(20), at),
        second.claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(20), at),
    );
    let claims = [left.unwrap(), right.unwrap()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].job().id(), job.id());
}

#[tokio::test]
async fn maintenance_prunes_only_expired_completed_jobs_and_is_idempotent() {
    let (_directory, queue) = queue().await;
    let now = now();
    let old_time = now - time::Duration::days(30);
    let completed = EnqueueJob::new(&queue, JobPolicy::default())
        .execute(
            JobKind::new("test.retained").unwrap(),
            1,
            payload(),
            EnqueueOptions::new(),
            old_time,
        )
        .await
        .unwrap();
    let completed_claim = queue
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(10),
            old_time,
        )
        .await
        .unwrap()
        .unwrap();
    queue
        .complete(
            completed.id(),
            completed_claim.lease().id(),
            old_time + time::Duration::seconds(1),
        )
        .await
        .unwrap();

    let failed = EnqueueJob::new(&queue, JobPolicy::default())
        .execute(
            JobKind::new("test.failed").unwrap(),
            1,
            payload(),
            EnqueueOptions::new(),
            now,
        )
        .await
        .unwrap();
    let failed_claim = queue
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(10),
            now,
        )
        .await
        .unwrap()
        .unwrap();
    let failure = JobFailure::new(JobFailureCode::new("test.failed").unwrap(), "retained").unwrap();
    queue
        .fail(failed.id(), failed_claim.lease().id(), &failure, now)
        .await
        .unwrap();

    let pending = EnqueueJob::new(&queue, JobPolicy::default())
        .execute(
            JobKind::new("test.pending").unwrap(),
            1,
            payload(),
            EnqueueOptions::new().available_at(now + time::Duration::days(2)),
            now,
        )
        .await
        .unwrap();
    let leased = EnqueueJob::new(&queue, JobPolicy::default())
        .execute(
            JobKind::new("test.leased").unwrap(),
            1,
            payload(),
            EnqueueOptions::new(),
            now,
        )
        .await
        .unwrap();
    queue
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(86_400),
            now,
        )
        .await
        .unwrap()
        .unwrap();

    let (left, right) = tokio::join!(
        queue.maintain(now, Duration::from_secs(7 * 86_400)),
        queue.maintain(now, Duration::from_secs(7 * 86_400))
    );
    assert_eq!(
        left.unwrap().completed_jobs_pruned() + right.unwrap().completed_jobs_pruned(),
        1
    );
    assert!(
        queue
            .list(QueueListOptions::new(Some(JobStatus::Completed), 100).unwrap())
            .await
            .unwrap()
            .iter()
            .all(|job| job.id() != completed.id())
    );
    assert!(
        queue
            .list(QueueListOptions::new(Some(JobStatus::Failed), 100).unwrap())
            .await
            .unwrap()
            .iter()
            .any(|job| job.id() == failed.id())
    );
    assert!(
        queue
            .list(QueueListOptions::new(Some(JobStatus::Pending), 100).unwrap())
            .await
            .unwrap()
            .iter()
            .any(|job| job.id() == pending.id())
    );
    assert!(
        queue
            .list(QueueListOptions::new(Some(JobStatus::Leased), 100).unwrap())
            .await
            .unwrap()
            .iter()
            .any(|job| job.id() == leased.id())
    );
}

#[tokio::test]
async fn malformed_persisted_payload_is_claimed_for_permanent_failure() {
    let directory = TempDir::new().unwrap();
    let url = format!("sqlite://{}", directory.path().join("queue.db").display());
    let queue = SqliteQueue::connect(SqliteQueueOptions::new(&url))
        .await
        .unwrap();
    queue.migrate().await.unwrap();
    let at = now();
    let job = EnqueueJob::new(&queue, JobPolicy::default())
        .execute(
            JobKind::new("test.corrupt").unwrap(),
            1,
            payload(),
            EnqueueOptions::new(),
            at,
        )
        .await
        .unwrap();
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query("UPDATE queue_jobs SET payload_json='not-json' WHERE id=?")
        .bind(job.id().as_uuid().as_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
    let claim = queue
        .claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(10), at)
        .await
        .unwrap()
        .unwrap();
    assert!(claim.payload_invalid());
    let failure = JobFailure::new(
        JobFailureCode::new("payload.invalid").unwrap(),
        "persisted job payload is invalid",
    )
    .unwrap();
    queue
        .fail(job.id(), claim.lease().id(), &failure, at)
        .await
        .unwrap();
    assert_eq!(
        queue
            .list(QueueListOptions::new(Some(JobStatus::Failed), 10).unwrap())
            .await
            .unwrap()[0]
            .last_failure()
            .unwrap()
            .code()
            .as_str(),
        "payload.invalid"
    );
}

#[tokio::test]
async fn sqlite_schema_constraints_indexes_and_physical_conventions_are_enforced() {
    let directory = TempDir::new().unwrap();
    let url = format!("sqlite://{}", directory.path().join("queue.db").display());
    let queue =
        SqliteQueue::connect(SqliteQueueOptions::new(&url).busy_timeout(Duration::from_secs(3)))
            .await
            .unwrap();
    queue.migrate().await.unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
        + time::Duration::nanoseconds(999);
    let job = EnqueueJob::new(&queue, JobPolicy::default())
        .execute(
            JobKind::new("test.schema").unwrap(),
            1,
            payload(),
            EnqueueOptions::new(),
            now,
        )
        .await
        .unwrap();
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    let id: Vec<u8> = sqlx::query_scalar("SELECT id FROM queue_jobs WHERE id=?")
        .bind(job.id().as_uuid().as_bytes().to_vec())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(id.len(), 16);
    let timestamp: i64 = sqlx::query_scalar("SELECT created_at FROM queue_jobs WHERE id=?")
        .bind(id.clone())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(timestamp, 1_700_000_000_000_000);
    let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(journal.to_ascii_lowercase(), "wal");
    for index in [
        "queue_jobs_pending_claim",
        "queue_jobs_expired_lease",
        "queue_jobs_completed_retention",
        "queue_jobs_operator_listing",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?)",
        )
        .bind(index)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "missing index {index}");
    }
    let invalid = sqlx::query("UPDATE queue_jobs SET status='leased' WHERE id=?")
        .bind(id)
        .execute(&pool)
        .await;
    assert!(
        invalid.is_err(),
        "incoherent lease state must be rejected by SQLite"
    );
}

#[tokio::test]
async fn queue_migration_checksum_tampering_is_rejected() {
    let directory = TempDir::new().unwrap();
    let url = format!("sqlite://{}", directory.path().join("queue.db").display());
    let queue = SqliteQueue::connect(SqliteQueueOptions::new(&url))
        .await
        .unwrap();
    queue.migrate().await.unwrap();
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query(
        "UPDATE _sqlx_migrations SET checksum = zeroblob(length(checksum)) WHERE version=1",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(queue.migration_status().await.is_err());
}
