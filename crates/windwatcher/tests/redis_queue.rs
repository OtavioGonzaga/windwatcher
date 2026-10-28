#![cfg(feature = "queue-redis")]
use std::time::Duration;
use time::OffsetDateTime;
use windwatcher::*;
#[path = "support/queue_contract.rs"]
mod queue_contract;

async fn cleanup(url: &str, prefix: &str) {
    let mut conn = redis::Client::open(url)
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let mut cursor = 0_u64;
    loop {
        let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(format!("{prefix}:*"))
            .arg("COUNT")
            .arg(100)
            .query_async(&mut conn)
            .await
            .unwrap();
        if !keys.is_empty() {
            let _: u64 = redis::cmd("DEL")
                .arg(keys)
                .query_async(&mut conn)
                .await
                .unwrap();
        }
        cursor = next;
        if cursor == 0 {
            break;
        }
    }
}
#[tokio::test]
#[ignore = "requires disposable Redis; set WINDWATCHER_TEST_REDIS_URL"]
async fn shared_contract_independent_clients_and_namespace_isolation() {
    let url = std::env::var("WINDWATCHER_TEST_REDIS_URL")
        .expect("set WINDWATCHER_TEST_REDIS_URL to a disposable Redis endpoint");
    let prefix = format!("windwatcher:test:{}", uuid::Uuid::now_v7());
    let options = RedisQueueOptions::new(&url).key_prefix(&prefix);
    let a = RedisQueue::connect(options.clone()).await.unwrap();
    queue_contract::run(&a).await;
    cleanup(&url, &prefix).await;
    let b = RedisQueue::connect(options).await.unwrap();
    let other_prefix = format!("windwatcher:test:{}", uuid::Uuid::now_v7());
    let other = RedisQueue::connect(RedisQueueOptions::new(&url).key_prefix(&other_prefix))
        .await
        .unwrap();
    let now = OffsetDateTime::from_unix_timestamp_nanos(-1).unwrap();
    let job = EnqueueJob::new(&a, JobPolicy::default())
        .execute(
            JobKind::new("test.race").unwrap(),
            1,
            JobPayload::try_from(
                serde_json::json!({"large":18446744073709551615_u64,"nested":{"a":1}}),
            )
            .unwrap(),
            EnqueueOptions::new(),
            now,
        )
        .await
        .unwrap();
    assert!(
        other
            .list(QueueListOptions::new(None, 100).unwrap())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(a.enqueue(&job).await.is_err());
    let (x, y) = tokio::join!(
        a.claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(1), now),
        b.claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(1), now)
    );
    let first = match (x.unwrap(), y.unwrap()) {
        (Some(c), None) | (None, Some(c)) => c,
        _ => panic!("exactly one claim must win"),
    };
    assert_eq!(first.job().payload(), job.payload());
    let later = now + time::Duration::seconds(1);
    let second = b
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(10),
            later,
        )
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first.lease().id(), second.lease().id());
    assert_eq!(
        a.complete(job.id(), first.lease().id(), later)
            .await
            .unwrap(),
        LeaseMutationOutcome::LeaseLost
    );
    assert_eq!(
        b.complete(job.id(), second.lease().id(), later)
            .await
            .unwrap(),
        LeaseMutationOutcome::Applied
    );
    // Invalid persisted payload follows the permanent payload.invalid worker path.
    let corrupt = EnqueueJob::new(&a, JobPolicy::default())
        .execute(
            JobKind::new("test.corrupt").unwrap(),
            1,
            JobPayload::new(Default::default()),
            EnqueueOptions::new(),
            later,
        )
        .await
        .unwrap();
    let mut conn = redis::Client::open(url.as_str())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let _: () = redis::cmd("HSET")
        .arg(format!("{prefix}:job:{}", corrupt.id()))
        .arg("payload_json")
        .arg("not JSON")
        .query_async(&mut conn)
        .await
        .unwrap();
    let claimed = b
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(10),
            later,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(claimed.payload_invalid());
    a.fail(
        corrupt.id(),
        claimed.lease().id(),
        &JobFailure::new(
            JobFailureCode::new("payload.invalid").unwrap(),
            "invalid payload",
        )
        .unwrap(),
        later,
    )
    .await
    .unwrap();
    // Missing hydration is an error, not silent omission.
    let _: u64 = redis::cmd("DEL")
        .arg(format!("{prefix}:job:{}", corrupt.id()))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(
        a.list(QueueListOptions::new(None, 100).unwrap())
            .await
            .is_err()
    );
    cleanup(&url, &prefix).await;
    cleanup(&url, &other_prefix).await;
}
#[test]
fn options_and_errors_redact_credentials_and_validate_prefix() {
    let secret = "recognizable_fake_password";
    let options = RedisQueueOptions::new(format!("redis://user:{secret}@localhost/0"));
    assert!(options.validate().is_ok());
    assert!(!format!("{options:?}").contains(secret));
    for prefix in ["".to_string(), "bad\nprefix".to_string(), "x".repeat(257)] {
        assert!(options.clone().key_prefix(prefix).validate().is_err());
    }
    assert!(RedisQueueOptions::new("not a URL").validate().is_err());
    let error = RedisQueueError::Database(Box::new(std::io::Error::other(secret)));
    assert!(!format!("{error:?} {error}").contains(secret));
}
