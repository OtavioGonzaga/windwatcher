//! Axum request extractors for authentication and authorisation.
//!
//! This module provides two [`FromRequestParts`]
//! implementations that can be used as handler parameters:
//!
//! * [`AuthenticatedUser`] - requires a valid Bearer JWT in the
//!   `Authorization` header.  Fails with `401 Unauthorized` otherwise.
//! * [`AdminUser`] - additionally enforces `role == "admin"` on the decoded
//!   claims.  Fails with `403 Forbidden` for non-admin tokens.
//!
//! ## Usage example
//!
//! ```rust,ignore
//! async fn my_handler(
//!     AuthenticatedUser(claims): AuthenticatedUser,
//! ) -> impl IntoResponse {
//!     format!("Hello, {}!", claims.sub)
//! }
//! ```

use axum::{
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};

use crate::{application::user_service::Claims, error::AppError, state::AppState};

// ── AuthenticatedUser ──────────────────────────────────────────────────────────

/// Axum extractor that reads and validates the `Authorization: Bearer <token>`
/// request header, decodes the JWT, and yields the inner [`Claims`].
///
/// # Extraction flow
///
/// 1. Read the `Authorization` header value.
/// 2. Strip the `"Bearer "` prefix.
/// 3. Call [`UserService::decode_token`](crate::application::user_service::UserService::decode_token)
///    to verify the HMAC-SHA256 signature and check expiry.
/// 4. Wrap the resulting [`Claims`] in `AuthenticatedUser` and pass it to the
///    handler.
///
/// # Errors
///
/// Returns [`AppError::Unauthorized`]
/// (`401 Unauthorized`) if:
/// * The `Authorization` header is absent or malformed.
/// * The token signature is invalid.
/// * The token has expired.
pub struct AuthenticatedUser(pub Claims);

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| {
                AppError::Unauthorized("missing or malformed Authorization header".into())
            })?;

        let claims = app_state.user_service.decode_token(token)?;
        Ok(AuthenticatedUser(claims))
    }
}

// ── AdminUser ──────────────────────────────────────────────────────────────────

/// Axum extractor that validates the Bearer JWT **and** enforces admin-only
/// access by checking that `claims.role == "admin"`.
///
/// Internally delegates to [`AuthenticatedUser`] for token validation, then
/// performs the role check on the resulting [`Claims`].
///
/// # Errors
///
/// * `401 Unauthorized` - same conditions as [`AuthenticatedUser`].
/// * `403 Forbidden` - the token is valid but the user is not an admin.
pub struct AdminUser(pub Claims);

impl<S> FromRequestParts<S> for AdminUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AuthenticatedUser(claims) = AuthenticatedUser::from_request_parts(parts, state).await?;

        if claims.role != "admin" {
            return Err(AppError::Forbidden("admin access required".into()));
        }

        Ok(AdminUser(claims))
    }
}
