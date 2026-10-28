use std::time::Duration;

use time::{Duration as TimeDuration, OffsetDateTime};
use windwatcher::*;

/// Portable enqueue, lease fencing, retry and completion behavior.
pub async fn run<S: QueueStore>(store: &S) {
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
}
