#![cfg(feature = "queue-postgres")]

use sqlx::{ConnectOptions, Connection, Executor};
use std::time::Duration;
use time::OffsetDateTime;
use windwatcher::*;

#[path = "support/queue_contract.rs"]
mod queue_contract;

// Each test owns a dedicated database; never point this URL at production.
async fn database() -> (String, String) {
    let base = std::env::var("WINDWATCHER_TEST_POSTGRES_URL").expect("set WINDWATCHER_TEST_POSTGRES_URL to a disposable PostgreSQL server (requires CREATE DATABASE)");
    let name = format!("windwatcher_test_{}", uuid::Uuid::now_v7().simple());
    let mut conn = sqlx::PgConnection::connect(&base).await.unwrap();
    // Identifier consists only of our fixed prefix and locally generated UUID hex.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&mut conn)
        .await
        .unwrap();
    let mut options: sqlx::postgres::PgConnectOptions = base.parse().unwrap();
    options = options.database(&name);
    // Preserve URL query options, credentials and TLS settings.
    let url = options.to_url_lossy().to_string();
    (url, name)
}
async fn cleanup(name: &str) {
    let base = std::env::var("WINDWATCHER_TEST_POSTGRES_URL").unwrap();
    let mut conn = sqlx::PgConnection::connect(&base).await.unwrap();
    assert!(
        name.starts_with("windwatcher_test_")
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {name} WITH (FORCE)"
    )))
    .execute(&mut conn)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; set WINDWATCHER_TEST_POSTGRES_URL"]
async fn shared_contract_and_independent_pools() {
    let (url, name) = database().await;
    let a = PostgresQueue::connect(PostgresQueueOptions::new(&url))
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
    // The contract leaves jobs behind; race on a fresh database state.
    let mut conn = sqlx::PgConnection::connect(&url).await.unwrap();
    conn.execute("DELETE FROM queue_jobs").await.unwrap();
    let b = PostgresQueue::connect(PostgresQueueOptions::new(&url))
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
    conn.execute("UPDATE _sqlx_migrations SET checksum=decode('00','hex')")
        .await
        .unwrap();
    assert!(a.migration_status().await.is_err());
    drop(conn);
    drop(a);
    drop(b);
    cleanup(&name).await;
}

#[test]
fn options_and_errors_redact_credentials() {
    let secret = "recognizable_fake_password";
    let options = PostgresQueueOptions::new(format!("postgres://user:{secret}@localhost/db"));
    assert!(!format!("{options:?}").contains(secret));
    assert!(options.validate().is_ok());
    assert!(PostgresQueueOptions::new("not a URL").validate().is_err());
    assert!(options.clone().max_connections(0).validate().is_err());
    assert!(options.acquire_timeout(Duration::ZERO).validate().is_err());
    let error = PostgresQueueError::Database(Box::new(std::io::Error::other(secret)));
    assert!(!format!("{error:?} {error}").contains(secret));
}
