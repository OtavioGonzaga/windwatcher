use std::time::Duration;

use time::{Duration as TimeDuration, OffsetDateTime};
use windwatcher::*;

/// Complete portable queue semantics; run against an initially empty adapter.
pub async fn run<S: QueueStore>(store: &S) {
    enqueue_claim_retry(store).await;
    expired_lease_reclaim(store).await;
    exhaustion_and_manual_retry(store).await;
    listing_and_maintenance(store).await;
}

async fn enqueue_claim_retry<S: QueueStore>(store: &S) {
    let now = OffsetDateTime::from_unix_timestamp(1_700_100_000).unwrap();
    let delayed_at = now + TimeDuration::seconds(30);
    let kind = JobKind::new("contract.work").unwrap();
    let payload = JobPayload::try_from(serde_json::json!({"key":"value"})).unwrap();
    let delayed = EnqueueJob::new(store, JobPolicy::default())
        .execute(
            kind.clone(),
            2,
            payload.clone(),
            EnqueueOptions::new()
                .available_at(delayed_at)
                .max_attempts(3),
            now,
        )
        .await
        .unwrap();
    assert_eq!(delayed.status(), JobStatus::Pending);
    assert_eq!(delayed.attempts(), 0);
    assert_eq!(delayed.max_attempts(), 3);
    assert_eq!(delayed.payload_version(), 2);
    assert_eq!(delayed.payload(), &payload);
    assert_eq!(
        delayed.id().as_uuid().get_version(),
        Some(uuid::Version::SortRand)
    );
    let before_oversized = store
        .list(QueueListOptions::new(None, 100))
        .await
        .unwrap()
        .len();
    assert!(
        EnqueueJob::new(store, JobPolicy::new(1, 2).unwrap())
            .execute(
                JobKind::new("contract.large").unwrap(),
                1,
                payload.clone(),
                EnqueueOptions::new(),
                now
            )
            .await
            .is_err()
    );
    assert_eq!(
        store
            .list(QueueListOptions::new(None, 100))
            .await
            .unwrap()
            .len(),
        before_oversized
    );
    assert!(
        store
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_secs(10),
                now
            )
            .await
            .unwrap()
            .is_none()
    );

    let ready = EnqueueJob::new(store, JobPolicy::default())
        .execute(kind, 1, payload, EnqueueOptions::new(), now)
        .await
        .unwrap();
    let worker = WorkerId::new();
    let first = store
        .claim_next(worker, LeaseId::new(), Duration::from_secs(10), now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.job().id(), ready.id());
    assert_eq!(first.job().attempts(), 1);
    assert_eq!(first.lease().worker_id(), worker);
    assert_eq!(first.lease().expires_at(), now + TimeDuration::seconds(10));
    assert_eq!(
        store
            .renew(
                ready.id(),
                first.lease().id(),
                Duration::from_secs(12),
                now + TimeDuration::seconds(1)
            )
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    assert_eq!(
        store
            .list(QueueListOptions::new(Some(JobStatus::Leased), 10))
            .await
            .unwrap()
            .iter()
            .find(|job| job.id() == ready.id())
            .unwrap()
            .lease()
            .unwrap()
            .expires_at(),
        now + TimeDuration::seconds(13)
    );
    assert_eq!(
        store
            .renew(ready.id(), LeaseId::new(), Duration::from_secs(10), now)
            .await
            .unwrap(),
        LeaseMutationOutcome::LeaseLost
    );
    assert_eq!(
        store
            .renew(
                ready.id(),
                first.lease().id(),
                Duration::from_secs(10),
                now + TimeDuration::seconds(13)
            )
            .await
            .unwrap(),
        LeaseMutationOutcome::LeaseLost
    );
    let failure =
        JobFailure::new(JobFailureCode::new("contract.retry").unwrap(), "diagnostic").unwrap();
    assert_eq!(
        store
            .retry(
                ready.id(),
                first.lease().id(),
                &failure,
                now + TimeDuration::seconds(1),
                now
            )
            .await
            .unwrap(),
        RetryOutcome::Scheduled
    );
    assert_eq!(
        store
            .complete(ready.id(), first.lease().id(), now)
            .await
            .unwrap(),
        LeaseMutationOutcome::LeaseLost
    );
    let reclaimed = store
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(10),
            now + TimeDuration::seconds(2),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reclaimed.job().id(), ready.id());
    assert_eq!(reclaimed.job().attempts(), 2);
    assert_eq!(
        store
            .complete(
                ready.id(),
                reclaimed.lease().id(),
                now + TimeDuration::seconds(3)
            )
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    assert_eq!(
        store
            .list(QueueListOptions::new(Some(JobStatus::Completed), 10))
            .await
            .unwrap()
            .iter()
            .map(Job::id)
            .collect::<Vec<_>>(),
        vec![ready.id()]
    );

    let race_job = EnqueueJob::new(store, JobPolicy::default())
        .execute(
            JobKind::new("contract.race").unwrap(),
            1,
            JobPayload::try_from(serde_json::json!({})).unwrap(),
            EnqueueOptions::new(),
            now,
        )
        .await
        .unwrap();
    let (left, right) = tokio::join!(
        store.claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(10),
            now
        ),
        store.claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(10),
            now
        ),
    );
    let claims = [left.unwrap(), right.unwrap()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].job().id(), race_job.id());
    // Finish this scenario before testing other claim behavior on the same store.
    let finish_at = now + TimeDuration::seconds(31);
    while let Some(claim) = store
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(10),
            finish_at,
        )
        .await
        .unwrap()
    {
        assert_eq!(
            store
                .complete(claim.job().id(), claim.lease().id(), finish_at)
                .await
                .unwrap(),
            LeaseMutationOutcome::Applied
        );
    }
}

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_700_200_000 + seconds).unwrap()
}

fn failure() -> JobFailure {
    JobFailure::new(
        JobFailureCode::new("contract.failure").unwrap(),
        "safe diagnostic",
    )
    .unwrap()
}

async fn enqueue<S: QueueStore>(store: &S, now: OffsetDateTime, options: EnqueueOptions) -> Job {
    EnqueueJob::new(store, JobPolicy::new(1024, 3).unwrap())
        .execute(
            JobKind::new("contract.lifecycle").unwrap(),
            2,
            JobPayload::try_from(serde_json::json!({"resource_id":"portable"})).unwrap(),
            options,
            now,
        )
        .await
        .unwrap()
}

async fn claim<S: QueueStore>(store: &S, now: OffsetDateTime) -> ClaimedJob {
    store
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(10),
            now,
        )
        .await
        .unwrap()
        .expect("eligible job")
}

async fn listed<S: QueueStore>(store: &S, id: JobId) -> Job {
    store
        .list(QueueListOptions::new(None, 100))
        .await
        .unwrap()
        .into_iter()
        .find(|job| job.id() == id)
        .expect("persisted job")
}

async fn assert_fenced<S: QueueStore>(store: &S, job: JobId, lease: LeaseId, now: OffsetDateTime) {
    let before = listed(store, job).await;
    assert_eq!(
        store
            .renew(job, lease, Duration::from_secs(10), now)
            .await
            .unwrap(),
        LeaseMutationOutcome::LeaseLost
    );
    assert_eq!(
        store.complete(job, lease, now).await.unwrap(),
        LeaseMutationOutcome::LeaseLost
    );
    assert_eq!(
        store.retry(job, lease, &failure(), now, now).await.unwrap(),
        RetryOutcome::LeaseLost
    );
    assert_eq!(
        store.fail(job, lease, &failure(), now).await.unwrap(),
        LeaseMutationOutcome::LeaseLost
    );
    assert_eq!(
        listed(store, job).await,
        before,
        "stale mutations must not change state"
    );
}

async fn expired_lease_reclaim<S: QueueStore>(store: &S) {
    let job = enqueue(store, at(0), EnqueueOptions::new()).await;
    let first = claim(store, at(0)).await;
    assert_eq!(first.job().id(), job.id());
    assert!(
        store
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_secs(10),
                at(9)
            )
            .await
            .unwrap()
            .is_none()
    );
    // Expiration itself revokes all mutation authority, even before reclaim.
    assert_fenced(store, job.id(), first.lease().id(), at(10)).await;
    let second = claim(store, at(10)).await;
    assert_eq!(second.job().id(), job.id());
    assert_ne!(second.lease().id(), first.lease().id());
    assert_eq!(second.job().attempts(), 2);
    assert_eq!(second.lease().expires_at(), at(20));
    assert_fenced(store, job.id(), first.lease().id(), at(11)).await;
    assert_fenced(store, job.id(), LeaseId::new(), at(11)).await;
    assert_eq!(
        store
            .complete(job.id(), second.lease().id(), at(11))
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    let done = listed(store, job.id()).await;
    assert_eq!(done.status(), JobStatus::Completed);
    assert_eq!(done.completed_at(), Some(at(11)));
    assert!(done.lease().is_none());
    assert!(done.last_failure().is_none());
}

async fn exhaustion_and_manual_retry<S: QueueStore>(store: &S) {
    let job = enqueue(store, at(30), EnqueueOptions::new().max_attempts(2)).await;
    let first = claim(store, at(30)).await;
    assert_eq!(
        store
            .retry(job.id(), first.lease().id(), &failure(), at(31), at(30))
            .await
            .unwrap(),
        RetryOutcome::Scheduled
    );
    let second = claim(store, at(31)).await;
    assert_eq!(second.job().attempts(), 2);
    assert_eq!(
        store
            .retry(job.id(), second.lease().id(), &failure(), at(32), at(31))
            .await
            .unwrap(),
        RetryOutcome::Exhausted
    );
    let failed = listed(store, job.id()).await;
    assert_eq!(failed.status(), JobStatus::Failed);
    assert_eq!(failed.failed_at(), Some(at(31)));
    assert_eq!(failed.last_failure(), Some(&failure()));
    assert!(failed.lease().is_none());
    assert!(
        store
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_secs(10),
                at(32)
            )
            .await
            .unwrap()
            .is_none()
    );

    assert_eq!(
        store.retry_failed(job.id(), at(33)).await.unwrap(),
        RetryFailedOutcome::Retried
    );
    let reset = listed(store, job.id()).await;
    assert_eq!(reset.id(), job.id());
    assert_eq!(reset.kind(), job.kind());
    assert_eq!(reset.payload_version(), job.payload_version());
    assert_eq!(reset.payload(), job.payload());
    assert_eq!(reset.created_at(), job.created_at());
    assert_eq!(reset.max_attempts(), 2);
    assert_eq!(reset.status(), JobStatus::Pending);
    assert_eq!(reset.attempts(), 0);
    assert_eq!(reset.available_at(), at(33));
    assert!(reset.lease().is_none() && reset.last_failure().is_none());
    assert!(reset.failed_at().is_none() && reset.completed_at().is_none());
    assert_eq!(
        store.retry_failed(job.id(), at(33)).await.unwrap(),
        RetryFailedOutcome::NotFailed
    );
    let third = claim(store, at(33)).await;
    assert_eq!(third.job().attempts(), 1);
    assert_fenced(store, job.id(), second.lease().id(), at(33)).await;
    assert_eq!(
        store.retry_failed(job.id(), at(33)).await.unwrap(),
        RetryFailedOutcome::NotFailed
    );
    // Permanent failure must not wait for max_attempts.
    assert_eq!(
        store
            .fail(job.id(), third.lease().id(), &failure(), at(34))
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    assert_eq!(listed(store, job.id()).await.status(), JobStatus::Failed);
    assert_eq!(listed(store, job.id()).await.attempts(), 1);
    assert_eq!(
        store.retry_failed(JobId::new(), at(34)).await.unwrap(),
        RetryFailedOutcome::NotFound
    );

    let abandoned = enqueue(store, at(40), EnqueueOptions::new().max_attempts(1)).await;
    let last = claim(store, at(40)).await;
    assert_eq!(last.job().id(), abandoned.id());
    assert!(
        store
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_secs(10),
                at(50)
            )
            .await
            .unwrap()
            .is_none()
    );
    let exhausted = listed(store, abandoned.id()).await;
    assert_eq!(exhausted.status(), JobStatus::Failed);
    assert_eq!(exhausted.attempts(), 1);
    assert!(exhausted.failed_at().is_some());
    assert!(exhausted.lease().is_none());
    assert!(exhausted.last_failure().is_some());
    assert_fenced(store, abandoned.id(), last.lease().id(), at(50)).await;
}

async fn listing_and_maintenance<S: QueueStore>(store: &S) {
    let old = enqueue(store, at(60), EnqueueOptions::new()).await;
    let old_claim = claim(store, at(60)).await;
    assert_eq!(
        store
            .complete(old.id(), old_claim.lease().id(), at(61))
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    assert_eq!(
        store.retry_failed(old.id(), at(61)).await.unwrap(),
        RetryFailedOutcome::NotFailed
    );
    let recent = enqueue(store, at(100), EnqueueOptions::new()).await;
    let recent_claim = claim(store, at(100)).await;
    assert_eq!(
        store
            .complete(recent.id(), recent_claim.lease().id(), at(101))
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    let exhausted = enqueue(store, at(110), EnqueueOptions::new().max_attempts(1)).await;
    store
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(15),
            at(110),
        )
        .await
        .unwrap()
        .unwrap();
    let pending = enqueue(store, at(120), EnqueueOptions::new().available_at(at(1000))).await;
    let leased = enqueue(store, at(120), EnqueueOptions::new()).await;
    let active = claim(store, at(120)).await;
    assert_eq!(active.job().id(), leased.id());
    // Same created_at exercises the ID tie-breaker without promising claim order.
    let rows = store.list(QueueListOptions::new(None, 100)).await.unwrap();
    let mut expected = rows.clone();
    expected.sort_by_key(|job| std::cmp::Reverse((job.created_at(), job.id())));
    assert_eq!(rows, expected);
    for status in [
        JobStatus::Pending,
        JobStatus::Leased,
        JobStatus::Completed,
        JobStatus::Failed,
    ] {
        let filtered = store
            .list(QueueListOptions::new(Some(status), 100))
            .await
            .unwrap();
        assert_eq!(
            filtered,
            rows.iter()
                .filter(|job| job.status() == status)
                .cloned()
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        store.list(QueueListOptions::new(None, 2)).await.unwrap(),
        rows[..2]
    );
    let completed_before = rows
        .iter()
        .filter(|job| job.status() == JobStatus::Completed && job.completed_at().unwrap() <= at(95))
        .count() as u64;
    let (left, right) = tokio::join!(
        store.maintain(at(125), Duration::from_secs(30)),
        store.maintain(at(125), Duration::from_secs(30)),
    );
    let (left, right) = (left.unwrap(), right.unwrap());
    assert_eq!(
        left.completed_jobs_pruned() + right.completed_jobs_pruned(),
        completed_before
    );
    assert_eq!(
        left.exhausted_jobs_failed() + right.exhausted_jobs_failed(),
        1
    );
    let again = store
        .maintain(at(125), Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(again, QueueMaintenanceReport::new(0, 0));
    let after = store.list(QueueListOptions::new(None, 100)).await.unwrap();
    assert!(!after.iter().any(|job| job.id() == old.id()));
    assert_eq!(
        listed(store, recent.id()).await.status(),
        JobStatus::Completed
    );
    assert_eq!(
        listed(store, pending.id()).await.status(),
        JobStatus::Pending
    );
    assert_eq!(listed(store, leased.id()).await, *active.job());
    assert_eq!(
        listed(store, exhausted.id()).await.status(),
        JobStatus::Failed
    );
    assert!(after.iter().any(|job| job.status() == JobStatus::Failed));
    // Failed retention is indefinite, even long after completed retention elapses.
    let failed_before = after
        .iter()
        .filter(|job| job.status() == JobStatus::Failed)
        .cloned()
        .collect::<Vec<_>>();
    store
        .maintain(at(10_000), Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(
        store
            .list(QueueListOptions::new(Some(JobStatus::Failed), 100))
            .await
            .unwrap(),
        failed_before
    );
}
