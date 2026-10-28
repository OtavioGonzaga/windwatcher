#![cfg(feature = "queue-mysql")]
use sqlx::{ConnectOptions, Connection, Executor};
use std::time::Duration;
use time::OffsetDateTime;
use windwatcher::*;
#[path = "support/queue_contract.rs"]
mod queue_contract;

#[tokio::test]
#[ignore = "requires disposable MySQL 8; set WINDWATCHER_TEST_MYSQL_URL"]
async fn shared_contract_and_independent_pools() {
    let base = std::env::var("WINDWATCHER_TEST_MYSQL_URL")
        .expect("set WINDWATCHER_TEST_MYSQL_URL (requires CREATE DATABASE)");
    let name = format!("windwatcher_test_{}", uuid::Uuid::now_v7().simple());
    let mut admin = sqlx::MySqlConnection::connect(&base).await.unwrap();
    // The identifier is a fixed prefix plus locally generated UUID hex, not user SQL.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&mut admin)
        .await
        .unwrap();
    let options: sqlx::mysql::MySqlConnectOptions = base.parse().unwrap();
    let url = options.database(&name).to_url_lossy().to_string();
    let a = MySqlQueue::connect(MySqlQueueOptions::new(&url))
        .await
        .unwrap();
    assert_eq!(a.migration_status().await.unwrap().pending(), 1);
    a.migrate().await.unwrap();
    a.migrate().await.unwrap();
    let status = a.check_schema().await.unwrap();
    assert!(status.is_current());
    assert_eq!(status.current_version(), Some(1));
    assert_eq!(status.latest_version(), Some(1));
    queue_contract::run(&a).await;
    let mut conn = sqlx::MySqlConnection::connect(&url).await.unwrap();
    conn.execute("DELETE FROM queue_jobs").await.unwrap();
    let b = MySqlQueue::connect(MySqlQueueOptions::new(&url))
        .await
        .unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let job = EnqueueJob::new(&a, JobPolicy::default())
        .execute(
            JobKind::new("test.race").unwrap(),
            1,
            JobPayload::new(Default::default()),
            EnqueueOptions::new(),
            now,
        )
        .await
        .unwrap();
    let (x, y) = tokio::join!(
        a.claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(1), now),
        b.claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(1), now)
    );
    let first = match (x.unwrap(), y.unwrap()) {
        (Some(c), None) | (None, Some(c)) => c,
        _ => panic!("exactly one claim must win"),
    };
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
    conn.execute("UPDATE _sqlx_migrations SET checksum=UNHEX('00')")
        .await
        .unwrap();
    assert!(a.migration_status().await.is_err());
    drop(conn);
    drop(a);
    drop(b);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}")))
        .execute(&mut admin)
        .await
        .unwrap();
}
#[test]
fn options_and_errors_redact_credentials() {
    let secret = "recognizable_fake_password";
    let options = MySqlQueueOptions::new(format!("mysql://user:{secret}@localhost/db"));
    assert!(!format!("{options:?}").contains(secret));
    assert!(options.validate().is_ok());
    assert!(MySqlQueueOptions::new("not a URL").validate().is_err());
    assert!(options.clone().max_connections(0).validate().is_err());
    assert!(options.acquire_timeout(Duration::ZERO).validate().is_err());
    let error = MySqlQueueError::Database(Box::new(std::io::Error::other(secret)));
    assert!(!format!("{error:?} {error}").contains(secret));
}
