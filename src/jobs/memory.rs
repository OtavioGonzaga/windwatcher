use std::sync::Arc;

use apalis::{
    layers::WorkerBuilderExt,
    prelude::{MemoryStorage, MessageQueue, WorkerBuilder, WorkerFactoryFn},
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

pub async fn build_memory_runtime(
    config: &AppConfig,
    chat_repo: Arc<dyn ChatRepository>,
    ws_manager: Arc<WsManager>,
) -> Result<(Arc<dyn JobQueue>, JoinHandle<()>), AppError> {
    let storage = MemoryStorage::<ChatMessageJob>::new();
    let queue: Arc<dyn JobQueue> = Arc::new(MemoryJobQueue {
        storage: Arc::new(Mutex::new(storage.clone())),
    });

    let worker_name = format!("{}-memory", config.queue_name);
    let worker = WorkerBuilder::new(worker_name)
        .concurrency(config.queue_concurrency)
        .backend(storage)
        .build_fn(move |job: ChatMessageJob| {
            let chat_repo = Arc::clone(&chat_repo);
            let ws_manager = Arc::clone(&ws_manager);
            async move {
                if let Err(e) = process_chat_message(job, chat_repo, ws_manager).await {
                    tracing::error!("memory queue job failed: {e}");
                }
            }
        });

    let handle = tokio::spawn(async move {
        worker.run().await;
    });

    Ok((queue, handle))
}

struct MemoryJobQueue {
    storage: Arc<Mutex<MemoryStorage<ChatMessageJob>>>,
}

#[async_trait]
impl JobQueue for MemoryJobQueue {
    async fn enqueue_chat_message(&self, job: ChatMessageJob) -> Result<(), AppError> {
        let mut storage = self.storage.lock().await;
        storage
            .enqueue(job)
            .await
            .map_err(|_| AppError::Internal("memory queue enqueue error".into()))?;
        Ok(())
    }
}
