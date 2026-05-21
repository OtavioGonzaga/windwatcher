use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::{
    api::ws::manager::WsManager,
    config::{AppConfig, QueueProvider},
    domain::ports::{ChatRepository, JobQueue},
    error::AppError,
};

pub struct JobRuntime {
    queue: Arc<dyn JobQueue>,
    #[allow(dead_code)]
    worker_handle: JoinHandle<()>,
}

impl JobRuntime {
    pub fn queue(&self) -> Arc<dyn JobQueue> {
        Arc::clone(&self.queue)
    }
}

/// Builds and initializes the job runtime, including the job queue and its associated worker.
///
/// The specific queue implementation (e.g., in-memory, SQLite, Postgres, Redis) is determined
/// by the `queue_provider` setting in the `AppConfig`.
///
/// # Arguments
///
/// * `config` - Application configuration, used to determine the queue provider and other settings.
/// * `chat_repo` - An `Arc` to the chat repository, used by job workers to interact with chat data.
/// * `ws_manager` - An `Arc` to the WebSocket manager, used by job workers to send updates to clients.
///
/// # Returns
///
/// A `Result` which is:
/// - `Ok(JobRuntime)` containing the initialized job queue and a handle to its worker task.
/// - `Err(AppError)` if an unsupported queue provider is configured, the required feature flag
///   is not enabled, or an error occurs during the initialization of the chosen queue.
pub async fn build_job_runtime(
    config: &AppConfig,
    chat_repo: Arc<dyn ChatRepository>,
    ws_manager: Arc<WsManager>,
) -> Result<JobRuntime, AppError> {
    let (queue, worker_handle): (Arc<dyn JobQueue>, JoinHandle<()>) = match config.queue_provider {
        QueueProvider::Memory => {
            #[cfg(feature = "queue-memory")]
            {
                super::memory::build_memory_runtime(config, chat_repo, ws_manager).await?
            }
            #[cfg(not(feature = "queue-memory"))]
            {
                return Err(AppError::Internal(
                    "queue provider 'memory' requested but feature 'queue-memory' is disabled"
                        .into(),
                ));
            }
        }
        QueueProvider::Sqlite => {
            #[cfg(feature = "queue-sqlite")]
            {
                super::sql::build_sqlite_runtime(config, chat_repo, ws_manager).await?
            }
            #[cfg(not(feature = "queue-sqlite"))]
            {
                return Err(AppError::Internal(
                    "queue provider 'sqlite' requested but feature 'queue-sqlite' is disabled"
                        .into(),
                ));
            }
        }
        QueueProvider::Postgres => {
            #[cfg(feature = "queue-postgres")]
            {
                super::sql::build_postgres_runtime(config, chat_repo, ws_manager).await?
            }
            #[cfg(not(feature = "queue-postgres"))]
            {
                return Err(AppError::Internal(
                    "queue provider 'postgres' requested but feature 'queue-postgres' is disabled"
                        .into(),
                ));
            }
        }
        QueueProvider::Mysql => {
            #[cfg(feature = "queue-mysql")]
            {
                super::sql::build_mysql_runtime(config, chat_repo, ws_manager).await?
            }
            #[cfg(not(feature = "queue-mysql"))]
            {
                return Err(AppError::Internal(
                    "queue provider 'mysql' requested but feature 'queue-mysql' is disabled".into(),
                ));
            }
        }
        QueueProvider::Redis => {
            #[cfg(feature = "queue-redis")]
            {
                super::redis::build_redis_runtime(config, chat_repo, ws_manager).await?
            }
            #[cfg(not(feature = "queue-redis"))]
            {
                return Err(AppError::Internal(
                    "queue provider 'redis' requested but feature 'queue-redis' is disabled".into(),
                ));
            }
        }
    };

    Ok(JobRuntime {
        queue,
        worker_handle,
    })
}
