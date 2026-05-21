use std::sync::Arc;

use uuid::Uuid;

use crate::{
    api::ws::manager::{SocketMessage, WsManager},
    domain::ports::{ChatMessageJob, ChatRepository},
    error::AppError,
};

/// Shared chat-message job handler used by all queue adapters.
pub async fn process_chat_message(
    job: ChatMessageJob,
    chat_repo: Arc<dyn ChatRepository>,
    ws_manager: Arc<WsManager>,
) -> Result<(), AppError> {
    let room_id: Uuid = job.room_id;
    let sender_id: Uuid = job.sender_id;

    let message = crate::domain::models::Message {
        id: job.message_id,
        room_id,
        sender_id,
        content: job.content,
        created_at: chrono::Utc::now(),
    };

    let saved = chat_repo.add_message(message).await?;
    chat_repo.increment_unread(room_id, sender_id).await?;
    let members = chat_repo.get_room_members(room_id).await?;

    ws_manager
        .send_to_users(&members, SocketMessage::NewMessage(saved))
        .await;

    Ok(())
}
