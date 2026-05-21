use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    api::ws::manager::{SocketMessage, WsManager},
    domain::ports::{ChatMessageJob, ChatRepository, JobQueue},
    error::AppError,
};

// ── In-memory job queue ────────────────────────────────────────────────────────

/// [`JobQueue`] implementation backed by a tokio mpsc channel.
///
/// Suitable for development and low-traffic deployments. Replace with an
/// Apalis-backed implementation when durability across restarts is required.
pub struct InMemoryJobQueue {
    tx: mpsc::Sender<ChatMessageJob>,
}

impl InMemoryJobQueue {
    pub fn new(tx: mpsc::Sender<ChatMessageJob>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl JobQueue for InMemoryJobQueue {
    async fn enqueue_chat_message(&self, job: ChatMessageJob) -> Result<(), AppError> {
        self.tx
            .send(job)
            .await
            .map_err(|e| AppError::Internal(format!("job queue send error: {e}")))
    }
}

// ── Core processing logic ──────────────────────────────────────────────────────

/// Process a single chat message job:
///
/// 1. Build a `Message` domain model from the job payload.
/// 2. Persist it via [`ChatRepository::add_message`].
/// 3. Increment unread counters for every other room member.
/// 4. Broadcast the saved message over WebSocket to all online members.
pub async fn process_chat_message(
    job: ChatMessageJob,
    chat_repo: Arc<dyn ChatRepository>,
    ws_manager: Arc<WsManager>,
) -> Result<(), AppError> {
    // Extract Copy fields before partially moving the job into the Message.
    let room_id: Uuid = job.room_id;
    let sender_id: Uuid = job.sender_id;

    let message = crate::domain::models::Message {
        id: job.message_id,
        room_id,
        sender_id,
        content: job.content,
        created_at: chrono::Utc::now(),
    };

    // 1. Persist the message.
    let saved = chat_repo.add_message(message).await?;

    // 2. Increment unread counters for everyone except the sender.
    chat_repo.increment_unread(room_id, sender_id).await?;

    // 3. Resolve room members for the WebSocket broadcast.
    let members = chat_repo.get_room_members(room_id).await?;

    // 4. Push to all currently connected members.
    ws_manager
        .send_to_users(&members, SocketMessage::NewMessage(saved))
        .await;

    Ok(())
}

// ── Background worker ──────────────────────────────────────────────────────────

/// Spawn a background task that drains jobs from `rx` and processes them.
///
/// Each job is handled in its own `tokio::spawn` so a slow or failing job
/// does not stall subsequent ones.  Call this **once** at application startup,
/// passing the *receiver* side of the mpsc channel whose *sender* side was
/// wrapped in [`InMemoryJobQueue`].
pub fn start_worker(
    mut rx: mpsc::Receiver<ChatMessageJob>,
    chat_repo: Arc<dyn ChatRepository>,
    ws_manager: Arc<WsManager>,
) {
    tokio::spawn(async move {
        tracing::info!("chat message worker started");

        while let Some(job) = rx.recv().await {
            let message_id = job.message_id;
            let repo = Arc::clone(&chat_repo);
            let wsm = Arc::clone(&ws_manager);

            tokio::spawn(async move {
                if let Err(e) = process_chat_message(job, repo, wsm).await {
                    tracing::error!(%message_id, "failed to process chat message: {e}");
                }
            });
        }

        tracing::warn!("chat message worker channel closed - worker shutting down");
    });
}

// ── Unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::mpsc;
    use uuid::Uuid;

    use crate::{
        api::ws::manager::WsManager,
        domain::ports::{ChatMessageJob, JobQueue, MockChatRepository},
        error::AppError,
    };

    use super::{InMemoryJobQueue, process_chat_message, start_worker};

    fn make_job() -> ChatMessageJob {
        ChatMessageJob {
            message_id: Uuid::now_v7(),
            room_id: Uuid::now_v7(),
            sender_id: Uuid::now_v7(),
            content: "hello".into(),
        }
    }

    // Test 1: enqueue puts job on channel
    #[tokio::test]
    async fn enqueue_sends_job_to_channel() {
        let (tx, mut rx) = mpsc::channel(10);
        let queue = InMemoryJobQueue::new(tx);
        let job = make_job();
        let result = queue.enqueue_chat_message(job.clone()).await;
        assert!(result.is_ok());
        let received = rx.try_recv().unwrap();
        assert_eq!(received.message_id, job.message_id);
    }

    // Test 2: enqueue on closed channel (receiver dropped) returns Internal error
    #[tokio::test]
    async fn enqueue_on_closed_channel_returns_error() {
        let (tx, rx) = mpsc::channel::<ChatMessageJob>(10);
        // Drop the receiver so that any send attempt reports a closed channel.
        drop(rx);
        let queue = InMemoryJobQueue::new(tx);
        let result = queue.enqueue_chat_message(make_job()).await;
        assert!(
            matches!(result, Err(AppError::Internal(_))),
            "expected Internal error, got {result:?}"
        );
    }

    // Test 3: process_chat_message calls add_message, increment_unread, get_room_members
    #[tokio::test]
    async fn process_calls_repo_methods() {
        let mut mock = MockChatRepository::new();
        mock.expect_add_message().once().returning(|m| Ok(m));
        mock.expect_increment_unread()
            .once()
            .returning(|_, _| Ok(()));
        mock.expect_get_room_members()
            .once()
            .returning(|_| Ok(vec![]));

        let ws = Arc::new(WsManager::new());
        let result = process_chat_message(make_job(), Arc::new(mock), ws).await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    // Test 4: process_chat_message propagates add_message error and does NOT
    //         call increment_unread or get_room_members.
    #[tokio::test]
    async fn process_propagates_repo_error() {
        let mut mock = MockChatRepository::new();
        mock.expect_add_message()
            .once()
            .returning(|_| Err(AppError::Database("db error".into())));
        // No expectations for increment_unread / get_room_members - mockall
        // will panic if either is called unexpectedly.

        let ws = Arc::new(WsManager::new());
        let result = process_chat_message(make_job(), Arc::new(mock), ws).await;
        assert!(
            matches!(result, Err(AppError::Database(_))),
            "expected Database error, got {result:?}"
        );
    }

    // Test 5: start_worker picks up an enqueued job and processes it end-to-end
    #[tokio::test]
    async fn start_worker_processes_job() {
        let mut mock = MockChatRepository::new();
        mock.expect_add_message().returning(|m| Ok(m));
        mock.expect_increment_unread().returning(|_, _| Ok(()));
        mock.expect_get_room_members().returning(|_| Ok(vec![]));

        let (tx, rx) = mpsc::channel(10);
        let ws = Arc::new(WsManager::new());
        start_worker(rx, Arc::new(mock), ws);

        let queue = InMemoryJobQueue::new(tx);
        queue.enqueue_chat_message(make_job()).await.unwrap();

        // Give the worker's spawned task time to process.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
