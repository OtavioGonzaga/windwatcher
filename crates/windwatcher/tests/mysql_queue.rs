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
    let columns: Vec<(String,String)> = sqlx::query_as("SELECT column_name,data_type FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name='queue_jobs'").fetch_all(&mut conn).await.unwrap();
    for column in [
        "available_at",
        "created_at",
        "lease_expires_at",
        "completed_at",
        "failed_at",
    ] {
        assert!(columns.contains(&(column.into(), "bigint".into())));
    }
    assert!(columns.contains(&("payload_json".into(), "mediumblob".into())));
    assert!(columns.contains(&("last_error_message".into(), "blob".into())));
    let malformed = EnqueueJob::new(&a, JobPolicy::default())
        .execute(
            JobKind::new("test.invalid-payload").unwrap(),
            1,
            JobPayload::new(Default::default()),
            EnqueueOptions::new(),
            later,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE queue_jobs SET payload_json=? WHERE id=?")
        .bind(b"invalid\xff".as_slice())
        .bind(malformed.id().as_uuid().into_bytes().to_vec())
        .execute(&mut conn)
        .await
        .unwrap();
    let claimed = a
        .claim_next(
            WorkerId::new(),
            LeaseId::new(),
            Duration::from_secs(10),
            later,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.job().id(), malformed.id());
    assert!(claimed.payload_invalid());
    a.fail(
        malformed.id(),
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
    sqlx::query("UPDATE queue_jobs SET last_error_message=? WHERE id=?")
        .bind(b"invalid\xff".as_slice())
        .bind(malformed.id().as_uuid().into_bytes().to_vec())
        .execute(&mut conn)
        .await
        .unwrap();
    let error = a
        .list(QueueListOptions::new(None, 100).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        std::error::Error::source(&error)
            .unwrap()
            .downcast_ref::<MySqlQueueError>(),
        Some(MySqlQueueError::Corrupt(_))
    ));
    let checksum: Vec<u8> =
        sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version=1")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    conn.execute("UPDATE _sqlx_migrations SET success=false")
        .await
        .unwrap();
    assert!(a.migration_status().await.is_err());
    conn.execute("UPDATE _sqlx_migrations SET success=true,version=999")
        .await
        .unwrap();
    assert!(a.migration_status().await.is_err());
    conn.execute("UPDATE _sqlx_migrations SET version=1")
        .await
        .unwrap();
    conn.execute("UPDATE _sqlx_migrations SET checksum=UNHEX('00')")
        .await
        .unwrap();
    assert!(a.migration_status().await.is_err());
    sqlx::query("UPDATE _sqlx_migrations SET checksum=?")
        .bind(checksum)
        .execute(&mut conn)
        .await
        .unwrap();
    assert!(a.migration_status().await.unwrap().is_current());
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
