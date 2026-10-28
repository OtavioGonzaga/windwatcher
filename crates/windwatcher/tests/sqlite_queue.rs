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
            .list(QueueListOptions::new(Some(JobStatus::Pending), 10))
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
            .list(QueueListOptions::new(Some(JobStatus::Completed), 10))
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
            .list(QueueListOptions::new(Some(JobStatus::Failed), 10))
            .await
            .unwrap()[0]
            .id(),
        job.id()
    );
}
