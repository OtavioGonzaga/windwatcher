// Each invocation runs against a disposable test service. Identifiers contain
// only a fixed prefix and generated UUID hex, never external SQL input.
macro_rules! snapshot_test {
    ($storage:ty, $options:ty, $db:ty, $pool:ty, $variable:literal, $session_setup:literal) => {
        #[tokio::test]
        #[ignore = "requires a disposable database; set WINDWATCHER_TEST_*_URL"]
        async fn read_snapshot_survives_read_committed_session_default() {
            let url = std::env::var($variable).unwrap();
            let storage = <$storage>::connect(<$options>::new(&url).max_connections(1))
                .await
                .unwrap();
            // Force the pool's only connection to RC, even if the service is RR.
            let mut connection = storage.pool.acquire().await.unwrap();
            sqlx::query($session_setup)
                .execute(&mut *connection)
                .await
                .unwrap();
            drop(connection);
            let writer = <$pool>::connect(&url).await.unwrap();
            let table = format!("snapshot_{}", uuid::Uuid::now_v7().simple());
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "CREATE TABLE {table} (id BIGINT PRIMARY KEY)"
            )))
            .execute(&writer)
            .await
            .unwrap();
            let mut snapshot = storage.read_snapshot().await.unwrap();
            let mut count = sqlx::QueryBuilder::<$db>::new("SELECT COUNT(*) FROM ");
            count.push(&table);
            let before: i64 = count
                .build_query_scalar()
                .fetch_one(&mut *snapshot)
                .await
                .unwrap();
            assert_eq!(before, 0);
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "INSERT INTO {table} (id) VALUES (1)"
            )))
            .execute(&writer)
            .await
            .unwrap();
            count.reset().push(&table);
            let during: i64 = count
                .build_query_scalar()
                .fetch_one(&mut *snapshot)
                .await
                .unwrap();
            assert_eq!(
                during, 0,
                "the second statement must use the first statement's snapshot"
            );
            snapshot.commit().await.unwrap();
            count.reset().push(&table);
            let after: i64 = count.build_query_scalar().fetch_one(&writer).await.unwrap();
            assert_eq!(after, 1);
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TABLE {table}")))
                .execute(&writer)
                .await
                .unwrap();
            storage.pool.close().await;
            writer.close().await;
        }
    };
}
