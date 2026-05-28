use async_trait::async_trait;
use uuid::Uuid;

use crate::error::AppError;

/// Port for submitting background work items.
///
/// Job runtime implementations live in [`crate::jobs`] and are Apalis-backed
/// (e.g. `crate::jobs::memory`, `crate::jobs::sql`, `crate::jobs::redis`).
/// For production durability prefer a persisted provider (sqlite/postgres/mysql/redis).
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait JobQueue: Send + Sync {
    /// Submit a chat-message job to the queue for asynchronous processing.
    ///
    /// The worker will dequeue the job, persist the message via
    /// [`ChatRepository::add_message`][crate::domain::ports::ChatRepository::add_message],
    /// update unread counters, and broadcast the message to online WebSocket clients.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Internal`] if the underlying channel is closed or
    /// the send fails (e.g. the worker task has panicked).
    async fn enqueue_chat_message(&self, job: ChatMessageJob) -> Result<(), AppError>;
}

/// Payload carried by a chat-message background job.
///
/// All fields needed to persist a message and deliver it to connected clients
/// are bundled here so the worker is self-contained and does not need to
/// re-query the database for basic message metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessageJob {
    /// Pre-generated UUIDv7 for the message, created by the HTTP handler
    /// *before* enqueueing so the 202 response can include it.
    pub message_id: Uuid,

    /// The room the message was sent to.
    pub room_id: Uuid,

    /// The user who sent the message.
    pub sender_id: Uuid,

    /// Plain-text message body.
    pub content: String,
}
