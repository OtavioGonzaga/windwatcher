use std::time::Duration;

use time::{Duration as TimeDuration, OffsetDateTime};
use windwatcher::*;

/// Complete portable queue semantics; run against an initially empty adapter.
pub async fn run<S: QueueStore>(store: &S) {
    enqueue_claim_retry(store).await;
    expired_lease_reclaim(store).await;
    exhaustion_and_manual_retry(store).await;
    temporal_precision(store).await;
    listing_and_maintenance(store).await;
    pre_epoch_and_numeric_payload(store).await;
    opaque_payload_and_failure_text(store).await;
    full_temporal_range(store).await;
}

async fn opaque_payload_and_failure_text<S: QueueStore>(store: &S) {
    let mut nested = serde_json::Value::String("leaf\0雪".into());
    // Exceeds both MySQL JSON's 100-level limit and serde_json's default 128.
    for _ in 0..256 {
        nested = serde_json::Value::Array(vec![nested]);
    }
    let payload = JobPayload::try_from(serde_json::json!({
        "nul\0key": "a\0b", "unicode": "雪🦀é", "nested": nested,
        "unsigned": u64::MAX, "signed": i64::MIN, "fraction": 0.10000000000000002_f64,
    }))
    .unwrap();
    let now = OffsetDateTime::from_unix_timestamp(-10).unwrap();
    let job = EnqueueJob::new(store, JobPolicy::default())
        .execute(
            JobKind::new("contract.opaque").unwrap(),
            1,
            payload.clone(),
            EnqueueOptions::new().max_attempts(2),
            now,
        )
        .await
        .unwrap();
    assert_eq!(listed(store, job.id()).await.payload(), &payload);
    let claimed = claim(store, now).await;
    assert_eq!(claimed.job().id(), job.id());
    assert!(
        !claimed.payload_invalid(),
        "valid deep JSON must not become payload.invalid"
    );
    assert_eq!(claimed.job().payload(), &payload);
    let failure =
        JobFailure::new(JobFailureCode::new("test.opaque").unwrap(), "foo\0bar雪").unwrap();
    let later = now + TimeDuration::seconds(1);
    assert_eq!(
        store
            .retry(job.id(), claimed.lease().id(), &failure, later, now)
            .await
            .unwrap(),
        RetryOutcome::Scheduled
    );
    assert_eq!(listed(store, job.id()).await.last_failure(), Some(&failure));
    let reclaimed = claim(store, later).await;
    assert_eq!(reclaimed.job().id(), job.id());
    assert!(!reclaimed.payload_invalid());
    assert_eq!(reclaimed.job().payload(), &payload);
    assert_eq!(
        store
            .fail(job.id(), reclaimed.lease().id(), &failure, later)
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    let restored = listed(store, job.id()).await;
    assert_eq!(restored.last_failure(), Some(&failure));
    assert_eq!(restored.payload(), &payload);
}

// Earlier contract cases may leave unrelated due work. Do not assume any claim order.
async fn claim_id<S: QueueStore>(
    store: &S,
    id: JobId,
    now: OffsetDateTime,
    duration: Duration,
) -> ClaimedJob {
    for _ in 0..100 {
        let claimed = store
            .claim_next(WorkerId::new(), LeaseId::new(), duration, now)
            .await
            .unwrap()
            .unwrap();
        if claimed.job().id() == id {
            return claimed;
        }
        assert_eq!(
            store
                .complete(claimed.job().id(), claimed.lease().id(), now)
                .await
                .unwrap(),
            LeaseMutationOutcome::Applied
        );
    }
    panic!("target job was not claimed");
}

async fn full_temporal_range<S: QueueStore>(store: &S) {
    let minimum = time::PrimitiveDateTime::MIN.assume_utc();
    let maximum = time::PrimitiveDateTime::MAX
        .assume_utc()
        .replace_nanosecond(999_999_000)
        .unwrap();
    // Covers years before PostgreSQL TIMESTAMPTZ and MySQL DATETIME minima.
    let job = enqueue(store, minimum, EnqueueOptions::new().max_attempts(2)).await;
    assert_eq!(listed(store, job.id()).await, job);
    let first = claim_id(store, job.id(), minimum, Duration::from_micros(1)).await;
    assert_eq!(
        first.lease().expires_at(),
        minimum + TimeDuration::microseconds(1)
    );
    let failure = JobFailure::new(
        JobFailureCode::new("test.range").unwrap(),
        "range\0diagnostic",
    )
    .unwrap();
    store
        .fail(job.id(), first.lease().id(), &failure, minimum)
        .await
        .unwrap();
    assert_eq!(listed(store, job.id()).await.failed_at(), Some(minimum));
    // A retention cutoff below the model's minimum remains valid integer arithmetic.
    store.maintain(minimum, Duration::MAX).await.unwrap();
    assert_eq!(listed(store, job.id()).await.last_failure(), Some(&failure));
    let near_max = maximum - TimeDuration::microseconds(4);
    assert_eq!(
        store.retry_failed(job.id(), near_max).await.unwrap(),
        RetryFailedOutcome::Retried
    );
    assert_eq!(listed(store, job.id()).await.available_at(), near_max);
    let second = claim_id(store, job.id(), near_max, Duration::from_micros(2)).await;
    let available = maximum - TimeDuration::microseconds(2);
    assert_eq!(
        store
            .retry(job.id(), second.lease().id(), &failure, available, near_max)
            .await
            .unwrap(),
        RetryOutcome::Scheduled
    );
    let final_claim = claim_id(store, job.id(), available, Duration::from_micros(2)).await;
    assert_eq!(final_claim.lease().expires_at(), maximum);
    assert_eq!(
        store
            .renew(
                job.id(),
                final_claim.lease().id(),
                Duration::from_micros(2),
                available
            )
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    let completed = maximum - TimeDuration::microseconds(1);
    assert_eq!(
        store
            .complete(job.id(), final_claim.lease().id(), completed)
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    assert_eq!(
        listed(store, job.id()).await.completed_at(),
        Some(completed)
    );
    assert_eq!(listed(store, job.id()).await.created_at(), minimum);
    let late = enqueue(store, maximum, EnqueueOptions::new()).await;
    assert_eq!(listed(store, late.id()).await, late);
    assert!(
        store
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_micros(1),
                maximum
            )
            .await
            .is_err()
    );
    assert_eq!(
        listed(store, late.id()).await,
        late,
        "expiration overflow must not mutate state"
    );
    store.maintain(maximum, Duration::MAX).await.unwrap();
    assert_eq!(
        listed(store, job.id()).await.completed_at(),
        Some(completed)
    );
    store
        .maintain(maximum, Duration::from_micros(1))
        .await
        .unwrap();
    assert!(
        !store
            .list(QueueListOptions::new(None, 100).unwrap())
            .await
            .unwrap()
            .iter()
            .any(|row| row.id() == job.id())
    );
    assert_eq!(listed(store, late.id()).await, late);
}

async fn pre_epoch_and_numeric_payload<S: QueueStore>(store: &S) {
    let raw = OffsetDateTime::from_unix_timestamp_nanos(-1).unwrap();
    let normalized = OffsetDateTime::from_unix_timestamp_nanos(-1000).unwrap();
    let payload = JobPayload::try_from(serde_json::json!({
        "unsigned": u64::MAX, "signed": i64::MIN,
        "fraction": 0.10000000000000002_f64, "nested": [null,true,{"text":"雪"}]
    }))
    .unwrap();
    let job = EnqueueJob::new(store, JobPolicy::default())
        .execute(
            JobKind::new("contract.roundtrip").unwrap(),
            u16::MAX,
            payload.clone(),
            EnqueueOptions::new().max_attempts(u32::MAX),
            raw,
        )
        .await
        .unwrap();
    assert_eq!(job.created_at(), normalized);
    assert_eq!(job.available_at(), normalized);
    assert_eq!(listed(store, job.id()).await, job);
    let claimed = store
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_nanos(1999),
            raw,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.job().payload(), &payload);
    assert_eq!(claimed.lease().expires_at(), OffsetDateTime::UNIX_EPOCH);
    store
        .complete(job.id(), claimed.lease().id(), raw)
        .await
        .unwrap();
    let restored = listed(store, job.id()).await;
    assert_eq!(restored.completed_at(), Some(normalized));
    assert_eq!(restored.payload_version(), u16::MAX);
    assert_eq!(restored.max_attempts(), u32::MAX);
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
        .list(QueueListOptions::new(None, 100).unwrap())
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
            .list(QueueListOptions::new(None, 100).unwrap())
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
            .list(QueueListOptions::new(Some(JobStatus::Leased), 10).unwrap())
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
            .list(QueueListOptions::new(Some(JobStatus::Completed), 10).unwrap())
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
        .list(QueueListOptions::new(None, 100).unwrap())
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

async fn temporal_precision<S: QueueStore>(store: &S) {
    let raw = at(55) + TimeDuration::nanoseconds(999);
    let job = enqueue(store, raw, EnqueueOptions::new().available_at(raw)).await;
    assert_eq!(job.created_at(), at(55));
    assert_eq!(job.available_at(), at(55));
    assert_eq!(listed(store, job.id()).await, job);
    for duration in [Duration::ZERO, Duration::from_nanos(999), Duration::MAX] {
        assert!(
            store
                .claim_next(WorkerId::new(), LeaseId::new(), duration, raw)
                .await
                .is_err()
        );
        assert_eq!(
            listed(store, job.id()).await,
            job,
            "invalid duration must not consume an attempt"
        );
    }
    let claimed = store
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_nanos(1999),
            raw,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.job().id(), job.id());
    assert_eq!(
        claimed.lease().expires_at(),
        at(55) + TimeDuration::microseconds(1)
    );
    assert!(
        store
            .renew(
                job.id(),
                claimed.lease().id(),
                Duration::from_nanos(999),
                raw
            )
            .await
            .is_err()
    );
    assert_eq!(listed(store, job.id()).await, *claimed.job());
    assert_eq!(
        store
            .renew(
                job.id(),
                claimed.lease().id(),
                Duration::from_nanos(2999),
                raw
            )
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    let expires = at(55) + TimeDuration::microseconds(2);
    assert_eq!(
        listed(store, job.id()).await.lease().unwrap().expires_at(),
        expires
    );
    assert_eq!(
        store
            .complete(
                job.id(),
                claimed.lease().id(),
                at(55) + TimeDuration::nanoseconds(1999)
            )
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    assert_eq!(
        listed(store, job.id()).await.completed_at(),
        Some(at(55) + TimeDuration::microseconds(1))
    );
    let completed = at(55) + TimeDuration::microseconds(1);
    store
        .maintain(
            completed + TimeDuration::nanoseconds(1001),
            Duration::from_nanos(1999),
        )
        .await
        .unwrap();
    assert!(
        !store
            .list(QueueListOptions::new(None, 100).unwrap())
            .await
            .unwrap()
            .iter()
            .any(|row| row.id() == job.id()),
        "retention floors both the clock and duration to microseconds"
    );
    let raw = at(56) + TimeDuration::nanoseconds(999);
    let failed = enqueue(store, raw, EnqueueOptions::new()).await;
    let claimed = claim(store, raw).await;
    assert_eq!(claimed.job().id(), failed.id());
    let failure =
        JobFailure::new(JobFailureCode::new("test.precision").unwrap(), "permanent").unwrap();
    store
        .fail(failed.id(), claimed.lease().id(), &failure, raw)
        .await
        .unwrap();
    assert_eq!(listed(store, failed.id()).await.failed_at(), Some(at(56)));
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
    let rows = store
        .list(QueueListOptions::new(None, 100).unwrap())
        .await
        .unwrap();
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
            .list(QueueListOptions::new(Some(status), 100).unwrap())
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
        store
            .list(QueueListOptions::new(None, 2).unwrap())
            .await
            .unwrap(),
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
    let after = store
        .list(QueueListOptions::new(None, 100).unwrap())
        .await
        .unwrap();
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
            .list(QueueListOptions::new(Some(JobStatus::Failed), 100).unwrap())
            .await
            .unwrap(),
        failed_before
    );
}
