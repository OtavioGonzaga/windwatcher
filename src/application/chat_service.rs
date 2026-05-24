//! Chat service - room management, message dispatch, and read-receipt tracking.
//!
//! This module owns the chat-related use cases of Windwatcher:
//!
//! * **Direct rooms** - idempotent creation/lookup of 1:1 rooms between two
//!   users, identified by a canonical key derived from both user IDs.
//! * **Group rooms** - create named rooms with an initial set of members.
//! * **Message enqueueing** - validate a message, assign a UUIDv7 identifier,
//!   and push it to the [`JobQueue`] for asynchronous persistence and
//!   WebSocket fan-out (see the pipeline diagram below).
//! * **Message listing** - cursor-based pagination over a room's message
//!   history in reverse-chronological order.
//! * **Read receipts** - reset the unread counter and advance the last-read
//!   message pointer for a specific user/room pair.
//!
//! # Message delivery pipeline
//!
//! ```text
//! POST /rooms/:id/messages
//!   -> ChatService::enqueue_message()          <- returns message_id (202 Accepted)
//!       -> JobQueue::enqueue_chat_message()
//!           -> background worker (tokio::spawn)
//!               -> ChatRepository::add_message()
//!               -> ChatRepository::increment_unread()
//!               -> WsManager::send_to_users()    <- WebSocket broadcast
//! ```
//!
//! The caller receives the pre-assigned `message_id` **immediately** (202
//! Accepted pattern) before the message is durably written to the database.
//!
//! # Dependencies
//!
//! * [`crate::domain::ports::ChatRepository`] - data-access port (injected).
//! * [`crate::domain::ports::JobQueue`] - background job queue port (injected).

use std::sync::Arc;

use serde::Deserialize;
use uuid::Uuid;

use crate::{
    domain::{
        models::{Message, Room},
        ports::{ChatMessageJob, ChatRepository, JobQueue},
    },
    error::AppError,
};

// ── DTOs ───────────────────────────────────────────────────────────────────────

/// Data transfer object for the direct-room creation endpoint
/// (`POST /rooms/direct`).
///
/// The service guarantees idempotency: if a 1:1 room between the requesting
/// user and `other_user_id` already exists, the existing room is returned
/// rather than creating a duplicate.
#[derive(Debug, Deserialize)]
pub struct CreateDirectRoomDto {
    /// UUID of the other participant in the direct conversation.
    pub other_user_id: Uuid,
}

/// Data transfer object for the group-room creation endpoint
/// (`POST /rooms/group`).
#[derive(Debug, Deserialize)]
pub struct CreateGroupRoomDto {
    /// Human-readable name of the group room.  Must not be blank or consist
    /// solely of whitespace.
    pub title: String,
    /// UUIDs of users to add as initial members.
    ///
    /// The list may be empty, in which case the room is created with no
    /// members (useful for invite-based flows).
    pub member_ids: Vec<Uuid>,
}

/// Data transfer object for the message-send endpoint
/// (`POST /rooms/:id/messages`).
///
/// The message is **not** persisted synchronously; it is validated here and
/// then forwarded to the [`JobQueue`] for asynchronous processing.
#[derive(Debug, Deserialize)]
pub struct SendMessageDto {
    /// UUID of the room the message is being sent to.
    pub room_id: Uuid,
    /// UUID of the user who is sending the message.
    pub sender_id: Uuid,
    /// Text body of the message.  Must not be blank or consist solely of
    /// whitespace.
    pub content: String,
}

/// Data transfer object for the message-listing endpoint
/// (`GET /rooms/:id/messages`).
///
/// Messages are returned in reverse-chronological order (newest first).
/// Pagination is cursor-based: pass the `id` of the oldest message in the
/// current page as `before_id` to fetch the next page.
///
/// Because message IDs are UUIDv7 (time-ordered), the cursor is stable even
/// as new messages arrive.
#[derive(Debug, Deserialize)]
pub struct ListMessagesDto {
    /// UUID of the room whose history is being fetched.
    pub room_id: Uuid,
    /// Exclusive pagination cursor.  When supplied, only messages with an ID
    /// that is strictly less than (i.e. older than) this value are returned.
    /// Omit to start from the most recent message.
    pub before_id: Option<Uuid>,
    /// Maximum number of messages to return.  Defaults to `50`; capped at
    /// `100` regardless of the supplied value.
    pub limit: Option<u64>,
}

// ── Service ────────────────────────────────────────────────────────────────────

/// Application service that orchestrates chat rooms, messages, and read-receipts.
///
/// Constructed once at startup and shared across all request handlers through
/// [`crate::state::AppState`].  All persistence is delegated to the injected
/// [`ChatRepository`]; background message delivery is delegated to the
/// injected [`JobQueue`].
pub struct ChatService {
    /// Repository adapter for chat persistence (rooms, messages, members, …).
    pub chat_repo: Arc<dyn ChatRepository>,
    /// Background job queue for asynchronous message processing.
    ///
    /// The job queue is provided by the job runtime (see [`crate::jobs::build_job_runtime`]),
    /// which supports Apalis-backed providers (memory/sql/redis). Use the injected
    /// [`JobQueue`] implementation to enqueue messages.
    pub job_queue: Arc<dyn JobQueue>,
}

impl ChatService {
    /// Create a new [`ChatService`].
    ///
    /// # Parameters
    ///
    /// * `chat_repo` - concrete repository implementation (e.g. the SeaORM
    ///   adapter [`crate::db::seaorm::SeaOrmChatRepository`]).
    /// * `job_queue` - concrete job queue implementation. In production this is
    ///   typically obtained from [`crate::jobs::build_job_runtime`].
    pub fn new(chat_repo: Arc<dyn ChatRepository>, job_queue: Arc<dyn JobQueue>) -> Self {
        Self {
            chat_repo,
            job_queue,
        }
    }

    // ── Direct room ────────────────────────────────────────────────────────────

    /// Return the existing direct room between two users, or create one if
    /// none exists yet.
    ///
    /// This operation is **idempotent**: calling it multiple times with the
    /// same pair of user IDs always returns the same room.  The canonical room
    /// key is derived from both UUIDs by the repository adapter (typically by
    /// sorting and concatenating them), so the order of `user_a` / `user_b`
    /// does not matter.
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - the repository returned a database error
    ///   while looking up or creating the room.
    pub async fn get_or_create_direct_room(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> Result<Room, AppError> {
        self.chat_repo
            .find_or_create_direct_room(user_a, user_b)
            .await
    }

    // ── Group room ─────────────────────────────────────────────────────────────

    /// Create a new group room with the given title and initial member list.
    ///
    /// Validates that the title is non-blank before delegating to the
    /// repository.  All UUIDs in `member_ids` are added as room members
    /// atomically by the repository adapter.
    ///
    /// # Errors
    ///
    /// * [`AppError::Validation`] - the supplied `title` is blank or consists
    ///   solely of whitespace.
    /// * [`AppError::Database`] - the repository returned a database error
    ///   while creating the room or inserting the members.
    pub async fn create_group_room(&self, dto: CreateGroupRoomDto) -> Result<Room, AppError> {
        if dto.title.trim().is_empty() {
            return Err(AppError::Validation(
                "group room title must not be empty".into(),
            ));
        }

        self.chat_repo
            .create_group_room(dto.title, dto.member_ids)
            .await
    }

    // ── Send / enqueue message ─────────────────────────────────────────────────

    /// Validate and enqueue a chat message for background processing.
    ///
    /// # 202 Accepted pattern
    ///
    /// A UUIDv7 `message_id` is generated **before** the message is persisted.
    /// The ID is returned immediately to the caller (HTTP 202 Accepted) so
    /// that it can be used for idempotency checks or optimistic UI updates,
    /// while the actual write, unread-counter increment, and WebSocket
    /// broadcast happen asynchronously in the background worker.
    ///
    /// # Flow
    ///
    /// 1. Reject blank/whitespace-only content with [`AppError::Validation`].
    /// 2. Generate a new `message_id` via [`uuid::Uuid::now_v7`].
    /// 3. Push a [`crate::domain::ports::ChatMessageJob`] onto the
    ///    [`JobQueue`].
    /// 4. Return `message_id` to the caller.
    ///
    /// # Errors
    ///
    /// * [`AppError::Validation`] - the message content is empty or consists
    ///   solely of whitespace.
    /// * [`AppError::Database`] / [`AppError::Internal`] - the job queue
    ///   returned an error while enqueueing the job (propagated from
    ///   [`JobQueue::enqueue_chat_message`]).
    pub async fn enqueue_message(&self, dto: SendMessageDto) -> Result<Uuid, AppError> {
        if dto.content.trim().is_empty() {
            return Err(AppError::Validation(
                "message content must not be empty".into(),
            ));
        }

        let message_id = Uuid::now_v7();

        self.job_queue
            .enqueue_chat_message(ChatMessageJob {
                message_id,
                room_id: dto.room_id,
                sender_id: dto.sender_id,
                content: dto.content,
            })
            .await?;

        Ok(message_id)
    }

    // ── List messages ──────────────────────────────────────────────────────────

    /// Return a page of messages for a room in reverse-chronological order.
    ///
    /// # Cursor-based pagination
    ///
    /// `before_id` is an **exclusive** cursor: pass the `id` of the oldest
    /// message currently displayed to fetch the preceding page.  Omit it to
    /// start from the most recent message.
    ///
    /// Because message IDs are UUIDv7 (monotonically increasing with wall
    /// time), the cursor remains stable even when new messages arrive between
    /// page fetches.
    ///
    /// Page size is resolved as `dto.limit.unwrap_or(50).min(100)`.
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - the repository returned a database error
    ///   while querying the messages.
    pub async fn list_messages(&self, dto: ListMessagesDto) -> Result<Vec<Message>, AppError> {
        let limit = dto.limit.unwrap_or(50).min(100);

        self.chat_repo
            .list_messages(dto.room_id, dto.before_id, limit)
            .await
    }

    // ── Mark as read ───────────────────────────────────────────────────────────

    /// Reset the unread counter for `user_id` in `room_id` and advance the
    /// last-read message pointer to `message_id`.
    ///
    /// After this call, the `unread_count` for the user/room pair is set to
    /// `0` and `last_read_message_id` is updated in the `room_users` table.
    /// Subsequent calls with an older `message_id` are accepted but have no
    /// visible effect (the repository adapter handles idempotency).
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - the repository returned a database error
    ///   while updating the read-receipt record.
    pub async fn mark_as_read(
        &self,
        room_id: Uuid,
        user_id: Uuid,
        message_id: Uuid,
    ) -> Result<(), AppError> {
        self.chat_repo
            .mark_as_read(room_id, user_id, message_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        models::{Room, RoomType},
        ports::{MockChatRepository, MockJobQueue},
    };
    use chrono::Utc;
    use mockall::predicate::*;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn svc(chat: MockChatRepository, queue: MockJobQueue) -> ChatService {
        ChatService::new(Arc::new(chat), Arc::new(queue))
    }

    fn make_room() -> Room {
        Room {
            id: Uuid::now_v7(),
            room_type: RoomType::Group,
            title: Some("Test Room".into()),
            direct_room_key: None,
            created_at: Utc::now(),
        }
    }

    // ── create_group_room ────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_group_room_ok() {
        let room = make_room();
        let room_clone = room.clone();
        let mut chat = MockChatRepository::new();
        chat.expect_create_group_room()
            .returning(move |_, _| Ok(room_clone.clone()));

        let result = svc(chat, MockJobQueue::new())
            .create_group_room(CreateGroupRoomDto {
                title: "My Room".into(),
                member_ids: vec![Uuid::now_v7()],
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().room_type, RoomType::Group);
    }

    #[tokio::test]
    async fn create_group_room_empty_title() {
        let chat = MockChatRepository::new(); // repo must NOT be called
        let err = svc(chat, MockJobQueue::new())
            .create_group_room(CreateGroupRoomDto {
                title: "  ".into(), // whitespace only
                member_ids: vec![],
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    // ── enqueue_message ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn enqueue_message_ok() {
        let mut queue = MockJobQueue::new();
        queue.expect_enqueue_chat_message().returning(|_| Ok(()));

        let room_id = Uuid::now_v7();
        let msg_id = svc(MockChatRepository::new(), queue)
            .enqueue_message(SendMessageDto {
                room_id,
                sender_id: Uuid::now_v7(),
                content: "hello".into(),
            })
            .await
            .unwrap();

        // Result must be a non-nil UUID
        assert_ne!(msg_id, Uuid::nil());
    }

    #[tokio::test]
    async fn enqueue_message_empty_content() {
        let chat = MockChatRepository::new();
        let queue = MockJobQueue::new(); // must NOT be called
        let err = svc(chat, queue)
            .enqueue_message(SendMessageDto {
                room_id: Uuid::now_v7(),
                sender_id: Uuid::now_v7(),
                content: "".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn enqueue_message_whitespace_only() {
        let chat = MockChatRepository::new();
        let queue = MockJobQueue::new();
        let err = svc(chat, queue)
            .enqueue_message(SendMessageDto {
                room_id: Uuid::now_v7(),
                sender_id: Uuid::now_v7(),
                content: "   ".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    // ── list_messages ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_messages_default_limit_is_50() {
        let room_id = Uuid::now_v7();
        let mut chat = MockChatRepository::new();
        chat.expect_list_messages()
            .withf(|_, _, limit| *limit == 50)
            .returning(|_, _, _| Ok(vec![]));

        svc(chat, MockJobQueue::new())
            .list_messages(ListMessagesDto {
                room_id,
                before_id: None,
                limit: None, // None -> default 50
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_messages_limit_capped_at_100() {
        let room_id = Uuid::now_v7();
        let mut chat = MockChatRepository::new();
        chat.expect_list_messages()
            .withf(|_, _, limit| *limit == 100) // capped at 100
            .returning(|_, _, _| Ok(vec![]));

        svc(chat, MockJobQueue::new())
            .list_messages(ListMessagesDto {
                room_id,
                before_id: None,
                limit: Some(9999),
            })
            .await
            .unwrap();
    }

    // ── mark_as_read ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn mark_as_read_delegates_to_repo() {
        let room_id = Uuid::now_v7();
        let user_id = Uuid::now_v7();
        let message_id = Uuid::now_v7();

        let mut chat = MockChatRepository::new();
        chat.expect_mark_as_read()
            .with(eq(room_id), eq(user_id), eq(message_id))
            .returning(|_, _, _| Ok(()));

        svc(chat, MockJobQueue::new())
            .mark_as_read(room_id, user_id, message_id)
            .await
            .unwrap();
    }
}
