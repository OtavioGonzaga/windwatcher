//! HTTP handlers for user-profile endpoints.
//!
//! | Method | Path        | Auth       | Description                             |
//! | ------ | ----------- | ---------- | --------------------------------------- |
//! | `GET`  | `/users/me` | Bearer JWT | Return the authenticated user's profile |
//!
//! Authentication is performed by the [`AuthenticatedUser`] extractor, which
//! decodes and validates the Bearer token before the handler is invoked.

use axum::{Json, extract::State};
use uuid::Uuid;

use crate::{
    api::http::extractors::AuthenticatedUser, domain::models::User, error::AppError,
    state::AppState,
};

/// `GET /users/me` - return the profile of the currently authenticated user.
///
/// # Authentication
///
/// Requires `Authorization: Bearer <jwt>` header.  The user identity is taken
/// from the `sub` claim of the decoded JWT.
///
/// # Responses
///
/// | Status             | Body          | Description                         |
/// | ------------------ | ------------- | ----------------------------------- |
/// | `200 OK`           | [`User`] JSON | Profile without password hash       |
/// | `401 Unauthorized` | error message | Missing or invalid JWT              |
/// | `404 Not Found`    | error message | User deleted after token was issued |
pub async fn me(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<User>, AppError> {
    let user_id = claims
        .sub
        .parse::<Uuid>()
        .map_err(|e| AppError::Internal(format!("invalid user id in token: {e}")))?;

    let user = state.user_service.get_by_id(user_id).await?;
    Ok(Json(user))
}
