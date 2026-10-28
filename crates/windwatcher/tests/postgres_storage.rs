#![cfg(feature = "postgres")]

use sqlx::{ConnectOptions, Connection};
use windwatcher::{PostgresStorage, PostgresStorageOptions};

async fn disposable_database() -> (String, String) {
    let base = std::env::var("WINDWATCHER_TEST_POSTGRES_URL")
        .expect("set WINDWATCHER_TEST_POSTGRES_URL to a disposable PostgreSQL server");
    let name = format!("windwatcher_storage_{}", uuid::Uuid::now_v7().simple());
    let mut connection = sqlx::PgConnection::connect(&base).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&mut connection)
        .await
        .unwrap();
    let options: sqlx::postgres::PgConnectOptions = base.parse().unwrap();
    (options.database(&name).to_url_lossy().to_string(), name)
}

async fn drop_database(name: &str) {
    assert!(
        name.starts_with("windwatcher_storage_")
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    );
    let base = std::env::var("WINDWATCHER_TEST_POSTGRES_URL").unwrap();
    let mut connection = sqlx::PgConnection::connect(&base).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {name} WITH (FORCE)"
    )))
    .execute(&mut connection)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server; set WINDWATCHER_TEST_POSTGRES_URL"]
async fn explicit_migration_lifecycle_and_physical_schema() {
    let (url, name) = disposable_database().await;
    let result = async {
        let storage = PostgresStorage::connect(PostgresStorageOptions::new(&url)).await.unwrap();
        assert_eq!(storage.migration_status().await.unwrap().pending(), 4);
        assert_eq!(storage.migration_status().await.unwrap().current_version(), None);
        storage.migrate().await.unwrap();
        storage.migrate().await.unwrap();
        assert!(storage.check_schema().await.unwrap().is_current());
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        let columns: Vec<(String, String)> = sqlx::query_as("SELECT table_name || '.' || column_name, data_type FROM information_schema.columns WHERE table_schema=current_schema()")
            .fetch_all(&pool).await.unwrap();
        for field in ["users.created_at", "sessions.expires_at", "conversations.updated_at", "messages.edited_at", "idempotency_records.expires_at"] {
            assert!(columns.contains(&(field.into(), "bigint".into())), "missing BIGINT {field}");
        }
        assert!(columns.contains(&("users.id".into(), "uuid".into())));
        assert!(columns.contains(&("conversations.title".into(), "bytea".into())));
        assert!(columns.contains(&("messages.content".into(), "text".into())));
        pool.close().await;
        Ok::<_, ()>(())
    }.await;
    drop_database(&name).await;
    result.unwrap();
}

#[test]
fn connection_options_redact_credentials() {
    let options = PostgresStorageOptions::new("postgres://private:secret@localhost/windwatcher");
    let rendered = format!("{options:?}");
    assert!(!rendered.contains("private"));
    assert!(!rendered.contains("secret"));
    assert!(rendered.contains("REDACTED"));
}
