//! HTTP handlers for user authentication.
//!
//! Exposes two unauthenticated endpoints that together provide the full
//! registration + login flow:
//!
//! | Method | Path             | Description                   |
//! | ------ | ---------------- | ----------------------------- |
//! | `POST` | `/auth/register` | Create a new user account     |
//! | `POST` | `/auth/login`    | Authenticate and obtain a JWT |
//!
//! All password hashing is performed by
//! [`UserService`](crate::application::user_service::UserService) using
//! **Argon2id**.  Plaintext passwords are never stored or logged.

use axum::{Json, extract::State, http::StatusCode};

use crate::{
    api::http::docs::{
        AuthTokenResponse as AuthTokenResponseSchema, ErrorResponse, LoginRequest,
        RegisterUserRequest, UserResponse,
    },
    application::user_service::{AuthTokenResponse, LoginDto, RegisterUserDto},
    domain::models::User,
    error::AppError,
    state::AppState,
};

/// `POST /auth/register` - create a new user account.
///
/// # Request body (`application/json`)
///
/// | Field      | Type     | Description                                           |
/// | ---------- | -------- | ----------------------------------------------------- |
/// | `username` | `string` | Unique display name                                   |
/// | `email`    | `string` | Unique e-mail address                                 |
/// | `password` | `string` | Plaintext password (hashed server-side with Argon2id) |
///
/// # Responses
///
/// | Status                     | Body          | Description                      |
/// | -------------------------- | ------------- | -------------------------------- |
/// | `201 Created`              | [`User`] JSON | Account created successfully     |
/// | `409 Conflict`             | error message | Username or e-mail already taken |
/// | `422 Unprocessable Entity` | error message | Malformed request body           |
#[utoipa::path(
    post,
    path = "/auth/register",
    tag = "auth",
    request_body = RegisterUserRequest,
    responses(
        (status = 201, description = "Account created successfully", body = UserResponse),
        (status = 409, description = "Username or e-mail already taken", body = ErrorResponse),
        (status = 422, description = "Malformed request body or validation error", body = ErrorResponse)
    )
)]
pub async fn register(
    State(state): State<AppState>,
    Json(dto): Json<RegisterUserDto>,
) -> Result<(StatusCode, Json<User>), AppError> {
    let user = state.user_service.register_user(dto).await?;
    Ok((StatusCode::CREATED, Json(user)))
}

/// `POST /auth/login` - authenticate and obtain a signed JWT.
///
/// # Request body (`application/json`)
///
/// | Field      | Type     | Description               |
/// | ---------- | -------- | ------------------------- |
/// | `email`    | `string` | Registered e-mail address |
/// | `password` | `string` | Account password          |
///
/// # Responses
///
/// | Status                     | Body              | Description                                       |
/// | -------------------------- | ----------------- | ------------------------------------------------- |
/// | `200 OK`                   | `{ token, user }` | Authentication successful; `token` is a HS256 JWT |
/// | `401 Unauthorized`         | error message     | Invalid credentials                               |
/// | `422 Unprocessable Entity` | error message     | Malformed request body                            |
///
/// The returned JWT should be included as `Authorization: Bearer <token>` in
/// subsequent requests to protected endpoints.
#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Authentication successful", body = AuthTokenResponseSchema),
        (status = 401, description = "Invalid credentials", body = ErrorResponse),
        (status = 422, description = "Malformed request body", body = ErrorResponse)
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Json(dto): Json<LoginDto>,
) -> Result<Json<AuthTokenResponse>, AppError> {
    let response = state.user_service.authenticate(dto).await?;
    Ok(Json(response))
}
