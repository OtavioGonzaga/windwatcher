//! OpenAPI documentation for the HTTP API.

use chrono::{DateTime, Utc};
use utoipa::{
    Modify, OpenApi, ToSchema,
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};
use uuid::Uuid;

use crate::api::http::{auth, chat, users};

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        auth::register,
        auth::login,
        users::me,
        chat::create_direct_room,
        chat::create_group_room,
        chat::send_message,
        chat::list_messages,
        chat::mark_as_read
    ),
    components(schemas(
        AuthTokenResponse,
        CreateDirectRoomRequest,
        CreateGroupRoomRequest,
        ErrorResponse,
        LoginRequest,
        MarkAsReadRequest,
        MessageResponse,
        RegisterUserRequest,
        RoomResponse,
        RoomTypeResponse,
        SendMessageAcceptedResponse,
        SendMessageRequest,
        UserResponse,
        UserRoleResponse
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "auth", description = "Registration and JWT authentication"),
        (name = "users", description = "Authenticated user profile"),
        (name = "rooms", description = "Chat room management"),
        (name = "messages", description = "Chat messages and read receipts")
    ),
    info(
        title = "Windwatcher API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Self-hosted real-time chat API built with Axum.",
        license(name = "MIT")
    )
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
        }
    }
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "Service is healthy", body = String, content_type = "text/plain")
    )
)]
pub async fn health() {}

#[derive(Debug, ToSchema)]
pub struct RegisterUserRequest {
    #[schema(example = "John Doe")]
    pub username: String,
    #[schema(example = "john@example.com")]
    pub email: String,
    #[schema(example = "correct-horse-battery-staple", min_length = 8)]
    pub password: String,
}

#[derive(Debug, ToSchema)]
pub struct LoginRequest {
    #[schema(example = "john@example.com")]
    pub email: String,
    #[schema(example = "correct-horse-battery-staple")]
    pub password: String,
}

#[derive(Debug, ToSchema)]
pub struct AuthTokenResponse {
    pub token: String,
    pub user: UserResponse,
}

#[derive(Debug, ToSchema)]
pub struct UserResponse {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub role: UserRoleResponse,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, ToSchema)]
#[schema(rename_all = "lowercase")]
pub enum UserRoleResponse {
    User,
    Admin,
}

#[derive(Debug, ToSchema)]
pub struct CreateDirectRoomRequest {
    pub other_user_id: Uuid,
}

#[derive(Debug, ToSchema)]
pub struct CreateGroupRoomRequest {
    #[schema(example = "Engineering")]
    pub title: String,
    pub member_ids: Vec<Uuid>,
}

#[derive(Debug, ToSchema)]
pub struct RoomResponse {
    pub id: Uuid,
    pub room_type: RoomTypeResponse,
    pub title: Option<String>,
    pub direct_room_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, ToSchema)]
#[schema(rename_all = "lowercase")]
pub enum RoomTypeResponse {
    Direct,
    Group,
}

#[derive(Debug, ToSchema)]
pub struct SendMessageRequest {
    #[schema(example = "hello")]
    pub content: String,
}

#[derive(Debug, ToSchema)]
pub struct SendMessageAcceptedResponse {
    pub message_id: Uuid,
}

#[derive(Debug, ToSchema)]
pub struct MessageResponse {
    pub id: Uuid,
    pub room_id: Uuid,
    pub sender_id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, ToSchema)]
pub struct MarkAsReadRequest {
    pub message_id: Uuid,
}

#[derive(Debug, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}
