//! HTTP handlers for chat rooms and messages.
//!
//! All endpoints in this module require a valid Bearer JWT in the
//! `Authorization` header (enforced by the [`AuthenticatedUser`] extractor).
//!
//! ## Endpoint overview
//!
//! | Method | Path                       | Description                          |
//! | ------ | -------------------------- | ------------------------------------ |
//! | `POST` | `/rooms/direct`            | Get or create a 1:1 direct room      |
//! | `POST` | `/rooms/group`             | Create a named group room            |
//! | `POST` | `/rooms/:room_id/messages` | Enqueue a message (async, 202)       |
//! | `GET`  | `/rooms/:room_id/messages` | List messages with cursor pagination |
//! | `PUT`  | `/rooms/:room_id/read`     | Reset unread counter for the caller  |
//!
//! ## Async message delivery
//!
//! `send_message` returns `202 Accepted` immediately with a pre-assigned
//! `message_id`.  The message is enqueued through the
//! [`crate::domain::ports::JobQueue`]
//! abstraction and persisted + broadcast by the background worker.
//! This pattern keeps the request latency low even under DB pressure.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    api::http::extractors::AuthenticatedUser,
    application::chat_service::{CreateGroupRoomDto, ListMessagesDto, SendMessageDto},
    domain::models::{Message, Room},
    error::AppError,
    state::AppState,
};

// ── Direct room ──────────────────────────────────────────────────────────

/// Request body for [`create_direct_room`].
#[derive(Debug, Deserialize)]
pub struct CreateDirectRoomBody {
    /// UUID of the other participant in the direct room.
    pub other_user_id: Uuid,
}

/// `POST /rooms/direct` - get or create a 1:1 direct room with another user.
///
/// If a direct room between the caller and `other_user_id` already exists, the
/// existing room is returned instead of creating a duplicate.
///
/// # Authentication
///
/// Requires `Authorization: Bearer <jwt>`.
///
/// # Request body (`application/json`)
///
/// | Field           | Type            | Description                         |
/// | --------------- | --------------- | ----------------------------------- |
/// | `other_user_id` | `string (UUID)` | ID of the other participant         |
///
/// # Responses
///
/// | Status             | Body          | Description                           |
/// | ------------------ | ------------- | ------------------------------------- |
/// | `200 OK`           | [`Room`] JSON | Existing or newly created direct room |
/// | `401 Unauthorized` | error         | Invalid or missing JWT                |
pub async fn create_direct_room(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(body): Json<CreateDirectRoomBody>,
) -> Result<Json<Room>, AppError> {
    let caller_id = claims
        .sub
        .parse::<Uuid>()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let room = state
        .chat_service
        .get_or_create_direct_room(caller_id, body.other_user_id)
        .await?;

    Ok(Json(room))
}

// ── Group room ─────────────────────────────────────────────────────────────────

/// `POST /rooms/group` - create a named group room.
///
/// The authenticated caller is automatically added to `member_ids` before
/// room creation if they are not already included in the list.
///
/// # Authentication
///
/// Requires `Authorization: Bearer <jwt>`.
///
/// # Request body (`application/json`)
///
/// Deserialised as [`CreateGroupRoomDto`].  Key fields:
///
/// | Field        | Type                | Description                                         |
/// | ------------ | ------------------- | --------------------------------------------------- |
/// | `name`       | `string`            | Display name of the group                           |
/// | `member_ids` | `string[] (UUID[])` | Initial member list (caller appended automatically) |
///
/// # Responses
///
/// | Status             | Body          | Description              |
/// | ------------------ | ------------- | ------------------------ |
/// | `201 Created`      | [`Room`] JSON | Newly created group room |
/// | `401 Unauthorized` | error         | Invalid or missing JWT   |
pub async fn create_group_room(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(mut dto): Json<CreateGroupRoomDto>,
) -> Result<(StatusCode, Json<Room>), AppError> {
    let caller_id = claims
        .sub
        .parse::<Uuid>()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if !dto.member_ids.contains(&caller_id) {
        dto.member_ids.push(caller_id);
    }

    let room = state.chat_service.create_group_room(dto).await?;
    Ok((StatusCode::CREATED, Json(room)))
}

// ── Messages ───────────────────────────────────────────────────────────────────

/// `POST /rooms/:room_id/messages` - enqueue a chat message for async delivery.
///
/// This endpoint follows the **fire-and-acknowledge** pattern: the message is
/// assigned a UUIDv7 identifier immediately and placed onto the in-memory job
/// queue.  The handler returns `202 Accepted` before the message is written to
/// the database or broadcast over WebSocket.  The background worker takes care
/// of persistence and real-time delivery.
///
/// # Authentication
///
/// Requires `Authorization: Bearer <jwt>`.
///
/// # Path parameters
///
/// | Parameter | Type   | Description |
/// | --------- | ------ | ----------- |
/// | `room_id` | `UUID` | Target room |
///
/// # Request body (`application/json`)
///
/// | Field     | Type     | Description             |
/// | --------- | -------- | ----------------------- |
/// | `content` | `string` | Message text (required) |
///
/// # Responses
///
/// | Status             | Body             | Description                                  |
/// | ------------------ | ---------------- | -------------------------------------------- |
/// | `202 Accepted`     | `{ message_id }` | Message enqueued; `message_id` is the UUIDv7 |
/// | `400 Bad Request`  | error            | `content` field missing                      |
/// | `401 Unauthorized` | error            | Invalid or missing JWT                       |
pub async fn send_message(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(room_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let sender_id = claims
        .sub
        .parse::<Uuid>()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let content = body
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Validation("field 'content' is required".into()))?
        .to_string();

    let message_id = state
        .chat_service
        .enqueue_message(SendMessageDto {
            room_id,
            sender_id,
            content,
        })
        .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "message_id": message_id })),
    ))
}

// ── List messages ──────────────────────────────────────────────────────────

/// Query-string parameters for [`list_messages`].
#[derive(Debug, Deserialize)]
pub struct ListMessagesParams {
    /// Cursor for backward pagination: return only messages whose UUIDv7 is
    /// strictly less than (i.e., older than) this value.
    /// Omit to start from the most recent message.
    pub before: Option<Uuid>,
    /// Maximum number of messages to return.  Defaults to `50` when omitted.
    pub limit: Option<u64>,
}

/// `GET /rooms/:room_id/messages` - fetch a paginated page of message history.
///
/// Uses **cursor-based pagination** via UUIDv7: because UUIDv7 values are
/// monotonically increasing with time, passing the `before` query parameter
/// efficiently retrieves the page of messages that precede a known point.
///
/// # Authentication
///
/// Requires `Authorization: Bearer <jwt>`.
///
/// # Path parameters
///
/// | Parameter | Type   | Description                |
/// | --------- | ------ | -------------------------- |
/// | `room_id` | `UUID` | Room to fetch history from |
///
/// # Query parameters
///
/// | Parameter | Type              | Default | Description                               |
/// | --------- | ----------------- | ------- | ----------------------------------------- |
/// | `before`  | `UUID` (optional) | -       | Cursor: fetch messages older than this ID |
/// | `limit`   | `u64` (optional)  | 50      | Page size (max messages returned)         |
///
/// # Responses
///
/// | Status             | Body             | Description                    |
/// | ------------------ | ---------------- | ------------------------------ |
/// | `200 OK`           | `Message[]` JSON | List of messages, newest-first |
/// | `401 Unauthorized` | error            | Invalid or missing JWT         |
pub async fn list_messages(
    State(state): State<AppState>,
    AuthenticatedUser(_claims): AuthenticatedUser,
    Path(room_id): Path<Uuid>,
    Query(params): Query<ListMessagesParams>,
) -> Result<Json<Vec<Message>>, AppError> {
    let messages = state
        .chat_service
        .list_messages(ListMessagesDto {
            room_id,
            before_id: params.before,
            limit: params.limit,
        })
        .await?;

    Ok(Json(messages))
}

// ── Mark as read ───────────────────────────────────────────────────────────────

/// `PUT /rooms/:room_id/read` - acknowledge messages and reset the unread counter.
///
/// Records that the caller has read up to and including `message_id`, then
/// resets their unread count for the specified room to zero.
///
/// # Authentication
///
/// Requires `Authorization: Bearer <jwt>`.
///
/// # Path parameters
///
/// | Parameter | Type   | Description          |
/// | --------- | ------ | -------------------- |
/// | `room_id` | `UUID` | Room to mark as read |
///
/// # Request body (`application/json`)
///
/// | Field        | Type            | Description                               |
/// | ------------ | --------------- | ----------------------------------------- |
/// | `message_id` | `string (UUID)` | ID of the last message seen by the caller |
///
/// # Responses
///
/// | Status             | Body  | Description                              |
/// | ------------------ | ----- | ---------------------------------------- |
/// | `204 No Content`   | -     | Unread counter reset successfully        |
/// | `400 Bad Request`  | error | `message_id` missing or not a valid UUID |
/// | `401 Unauthorized` | error | Invalid or missing JWT                   |
pub async fn mark_as_read(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(room_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Result<StatusCode, AppError> {
    let user_id = claims
        .sub
        .parse::<Uuid>()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let message_id = body
        .get("message_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::Validation("field 'message_id' must be a valid UUID".into()))?;

    state
        .chat_service
        .mark_as_read(room_id, user_id, message_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
