#![cfg(feature = "mysql")]

use sqlx::{ConnectOptions, Connection};
use windwatcher::{MySqlStorage, MySqlStorageOptions};

async fn disposable_database() -> (String, String) {
    let base = std::env::var("WINDWATCHER_TEST_MYSQL_URL")
        .expect("set WINDWATCHER_TEST_MYSQL_URL to a disposable MySQL 8 server");
    let name = format!("windwatcher_storage_{}", uuid::Uuid::now_v7().simple());
    let mut connection = sqlx::MySqlConnection::connect(&base).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE `{name}`")))
        .execute(&mut connection)
        .await
        .unwrap();
    let options: sqlx::mysql::MySqlConnectOptions = base.parse().unwrap();
    (options.database(&name).to_url_lossy().to_string(), name)
}

async fn drop_database(name: &str) {
    assert!(
        name.starts_with("windwatcher_storage_")
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    );
    let base = std::env::var("WINDWATCHER_TEST_MYSQL_URL").unwrap();
    let mut connection = sqlx::MySqlConnection::connect(&base).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP DATABASE `{name}`")))
        .execute(&mut connection)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires a disposable MySQL 8 server; set WINDWATCHER_TEST_MYSQL_URL"]
async fn explicit_migration_lifecycle_and_physical_schema() {
    let (url, name) = disposable_database().await;
    let result = async {
        let storage = MySqlStorage::connect(MySqlStorageOptions::new(&url)).await.unwrap();
        assert_eq!(storage.migration_status().await.unwrap().pending(), 4);
        storage.migrate().await.unwrap();
        storage.migrate().await.unwrap();
        assert!(storage.check_schema().await.unwrap().is_current());
        let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
        let types: Vec<(String, String, String)> = sqlx::query_as("SELECT table_name, column_name, data_type FROM information_schema.columns WHERE table_schema=DATABASE()")
            .fetch_all(&pool).await.unwrap();
        for (table, field) in [("users", "created_at"), ("sessions", "expires_at"), ("messages", "edited_at"), ("idempotency_records", "expires_at")] {
            assert!(types.iter().any(|(t,c,d)|t==table&&c==field&&d=="bigint"), "missing BIGINT {table}.{field}");
        }
        assert!(types.iter().any(|(t,c,d)|t=="users"&&c=="id"&&d=="binary"));
        assert!(types.iter().any(|(t,c,d)|t=="conversations"&&c=="title"&&d=="longblob"));
        assert!(types.iter().any(|(t,c,d)|t=="messages"&&c=="content"&&d=="longtext"));
        pool.close().await;
        Ok::<_, ()>(())
    }.await;
    drop_database(&name).await;
    result.unwrap();
}

#[test]
fn connection_options_redact_credentials() {
    let options = MySqlStorageOptions::new("mysql://private:secret@localhost/windwatcher");
    let rendered = format!("{options:?}");
    assert!(!rendered.contains("private"));
    assert!(!rendered.contains("secret"));
    assert!(rendered.contains("REDACTED"));
}
