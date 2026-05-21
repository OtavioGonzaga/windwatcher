use std::sync::Arc;

use apalis::{
    layers::WorkerBuilderExt,
    prelude::{Storage, WorkerBuilder, WorkerFactoryFn},
};
use async_trait::async_trait;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    api::ws::manager::WsManager,
    config::AppConfig,
    domain::ports::{ChatMessageJob, ChatRepository, JobQueue},
    error::AppError,
    jobs::processor::process_chat_message,
};

#[cfg(feature = "queue-mysql")]
use apalis_sql::mysql::{MySqlPool, MysqlStorage};
#[cfg(feature = "queue-postgres")]
use apalis_sql::postgres::{PgPool, PostgresStorage};
#[cfg(feature = "queue-sqlite")]
use apalis_sql::sqlite::{SqlitePool, SqliteStorage};

#[cfg(feature = "queue-sqlite")]
pub async fn build_sqlite_runtime(
    config: &AppConfig,
    chat_repo: Arc<dyn ChatRepository>,
    ws_manager: Arc<WsManager>,
) -> Result<(Arc<dyn JobQueue>, JoinHandle<()>), AppError> {
    let pool = SqlitePool::connect(&config.queue_url)
        .await
        .map_err(|e| AppError::Internal(format!("sqlite queue pool error: {e}")))?;
    SqliteStorage::setup(&pool)
        .await
        .map_err(|e| AppError::Internal(format!("sqlite queue setup error: {e}")))?;

    let storage = SqliteStorage::new(pool);
    let queue: Arc<dyn JobQueue> = Arc::new(SqliteJobQueue {
        storage: Arc::new(Mutex::new(storage.clone())),
    });

    let worker_name = format!("{}-sqlite", config.queue_name);
    let worker = WorkerBuilder::new(worker_name)
        .concurrency(config.queue_concurrency)
        .backend(storage)
        .build_fn(move |job: ChatMessageJob| {
            let chat_repo = Arc::clone(&chat_repo);
            let ws_manager = Arc::clone(&ws_manager);
            async move {
                if let Err(e) = process_chat_message(job, chat_repo, ws_manager).await {
                    tracing::error!("sqlite queue job failed: {e}");
                }
            }
        });

    let handle = tokio::spawn(async move {
        worker.run().await;
    });

    Ok((queue, handle))
}

#[cfg(feature = "queue-postgres")]
pub async fn build_postgres_runtime(
    config: &AppConfig,
    chat_repo: Arc<dyn ChatRepository>,
    ws_manager: Arc<WsManager>,
) -> Result<(Arc<dyn JobQueue>, JoinHandle<()>), AppError> {
    let pool = PgPool::connect(&config.queue_url)
        .await
        .map_err(|e| AppError::Internal(format!("postgres queue pool error: {e}")))?;
    PostgresStorage::setup(&pool)
        .await
        .map_err(|e| AppError::Internal(format!("postgres queue setup error: {e}")))?;

    let storage = PostgresStorage::new(pool);
    let queue: Arc<dyn JobQueue> = Arc::new(PostgresJobQueue {
        storage: Arc::new(Mutex::new(storage.clone())),
    });

    let worker_name = format!("{}-postgres", config.queue_name);
    let worker = WorkerBuilder::new(worker_name)
        .concurrency(config.queue_concurrency)
        .backend(storage)
        .build_fn(move |job: ChatMessageJob| {
            let chat_repo = Arc::clone(&chat_repo);
            let ws_manager = Arc::clone(&ws_manager);
            async move {
                if let Err(e) = process_chat_message(job, chat_repo, ws_manager).await {
                    tracing::error!("postgres queue job failed: {e}");
                }
            }
        });

    let handle = tokio::spawn(async move {
        worker.run().await;
    });

    Ok((queue, handle))
}

#[cfg(feature = "queue-mysql")]
pub async fn build_mysql_runtime(
    config: &AppConfig,
    chat_repo: Arc<dyn ChatRepository>,
    ws_manager: Arc<WsManager>,
) -> Result<(Arc<dyn JobQueue>, JoinHandle<()>), AppError> {
    let pool = MySqlPool::connect(&config.queue_url)
        .await
        .map_err(|e| AppError::Internal(format!("mysql queue pool error: {e}")))?;
    MysqlStorage::setup(&pool)
        .await
        .map_err(|e| AppError::Internal(format!("mysql queue setup error: {e}")))?;

    let storage = MysqlStorage::new(pool);
    let queue: Arc<dyn JobQueue> = Arc::new(MysqlJobQueue {
        storage: Arc::new(Mutex::new(storage.clone())),
    });

    let worker_name = format!("{}-mysql", config.queue_name);
    let worker = WorkerBuilder::new(worker_name)
        .concurrency(config.queue_concurrency)
        .backend(storage)
        .build_fn(move |job: ChatMessageJob| {
            let chat_repo = Arc::clone(&chat_repo);
            let ws_manager = Arc::clone(&ws_manager);
            async move {
                if let Err(e) = process_chat_message(job, chat_repo, ws_manager).await {
                    tracing::error!("mysql queue job failed: {e}");
                }
            }
        });

    let handle = tokio::spawn(async move {
        worker.run().await;
    });

    Ok((queue, handle))
}

#[cfg(feature = "queue-sqlite")]
struct SqliteJobQueue {
    storage: Arc<Mutex<SqliteStorage<ChatMessageJob>>>,
}

#[cfg(feature = "queue-sqlite")]
#[async_trait]
impl JobQueue for SqliteJobQueue {
    async fn enqueue_chat_message(&self, job: ChatMessageJob) -> Result<(), AppError> {
        let mut storage = self.storage.lock().await;
        storage
            .push(job)
            .await
            .map_err(|e| AppError::Internal(format!("sqlite queue enqueue error: {e}")))?;
        Ok(())
    }
}

#[cfg(feature = "queue-postgres")]
struct PostgresJobQueue {
    storage: Arc<Mutex<PostgresStorage<ChatMessageJob>>>,
}

#[cfg(feature = "queue-postgres")]
#[async_trait]
impl JobQueue for PostgresJobQueue {
    async fn enqueue_chat_message(&self, job: ChatMessageJob) -> Result<(), AppError> {
        let mut storage = self.storage.lock().await;
        storage
            .push(job)
            .await
            .map_err(|e| AppError::Internal(format!("postgres queue enqueue error: {e}")))?;
        Ok(())
    }
}

#[cfg(feature = "queue-mysql")]
struct MysqlJobQueue {
    storage: Arc<Mutex<MysqlStorage<ChatMessageJob>>>,
}

#[cfg(feature = "queue-mysql")]
#[async_trait]
impl JobQueue for MysqlJobQueue {
    async fn enqueue_chat_message(&self, job: ChatMessageJob) -> Result<(), AppError> {
        let mut storage = self.storage.lock().await;
        storage
            .push(job)
            .await
            .map_err(|e| AppError::Internal(format!("mysql queue enqueue error: {e}")))?;
        Ok(())
    }
}
