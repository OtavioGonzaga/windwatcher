//! Centralised error type for the entire application.
//!
//! [`AppError`] is the single error type returned by all service methods and
//! Axum handlers.  Its [`axum::response::IntoResponse`] implementation converts
//! each variant to an appropriate HTTP status code and a JSON body of the form
//! `{ "error": "<message>" }`.
//!
//! Third-party library errors (SeaORM, jsonwebtoken, Argon2) are converted via
//! blanket [`From`] implementations so that callers can use the `?` operator
//! without manual mapping.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

/// Application-level error type returned by all services and handlers.
///
/// Each variant maps to a distinct HTTP status code (see the [`IntoResponse`]
/// implementation below).  The inner `String` carries a human-readable description
/// that is forwarded to the HTTP client for client-facing errors, or logged and
/// replaced with a generic message for server-side errors.
///
/// # Error strategy
///
/// - **Client errors** (`4xx`) - the inner message is sent directly to the
///   caller in the `"error"` JSON field.
/// - **Server errors** (`5xx`) - the inner message is written to the
///   [`tracing`] log and the client receives only a generic message to avoid
///   leaking internal details.
#[derive(Debug, Error)]
pub enum AppError {
    /// The requested resource does not exist.
    ///
    /// Maps to **HTTP 404 Not Found**.
    #[error("resource not found: {0}")]
    NotFound(String),

    /// The request lacks valid authentication credentials.
    ///
    /// Maps to **HTTP 401 Unauthorized**.  Typically returned when a JWT is
    /// missing, expired, or has an invalid signature.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// The authenticated user does not have permission to perform the action.
    ///
    /// Maps to **HTTP 403 Forbidden**.
    #[error("forbidden: {0}")]
    Forbidden(String),

    /// The request conflicts with the current state (e.g. duplicate username).
    ///
    /// Maps to **HTTP 409 Conflict**.
    #[error("conflict: {0}")]
    Conflict(String),

    /// The request payload failed validation.
    ///
    /// Maps to **HTTP 422 Unprocessable Entity**.
    #[error("validation error: {0}")]
    Validation(String),

    /// A database operation failed.
    ///
    /// Maps to **HTTP 500 Internal Server Error**.  The inner message is
    /// written to the error log; the client receives only `"database error"`.
    #[error("database error: {0}")]
    Database(String),

    /// An unexpected server-side error occurred.
    ///
    /// Maps to **HTTP 500 Internal Server Error**.  The inner message is
    /// written to the error log; the client receives only
    /// `"internal server error"`.
    #[error("internal server error: {0}")]
    Internal(String),
}

// ── HTTP response mapping ─────────────────────────────────────────────────────
//
// Each AppError variant is converted into an (StatusCode, JSON) tuple.
// Server-side variants (Database, Internal) suppress their inner messages and
// emit them to the tracing log instead, preventing information leakage.

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            AppError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.clone()),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            AppError::Validation(m) => (StatusCode::UNPROCESSABLE_ENTITY, m.clone()),
            AppError::Database(m) => {
                tracing::error!("database error: {m}");
                (StatusCode::INTERNAL_SERVER_ERROR, "database error".into())
            }
            AppError::Internal(m) => {
                tracing::error!("internal error: {m}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".into(),
                )
            }
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

// ── Conversions from third-party error types ───────────────────────────────────
//
// These blanket From impls allow service code to use the `?` operator directly
// on SeaORM, jsonwebtoken, and Argon2 results without manual error mapping.

/// Converts a SeaORM database error into [`AppError::Database`].
impl From<sea_orm::DbErr> for AppError {
    fn from(e: sea_orm::DbErr) -> Self {
        AppError::Database(e.to_string())
    }
}

/// Converts a JWT error into [`AppError::Unauthorized`] (invalid / expired token).
impl From<jsonwebtoken::errors::Error> for AppError {
    fn from(e: jsonwebtoken::errors::Error) -> Self {
        AppError::Unauthorized(e.to_string())
    }
}

/// Converts an Argon2 password-hashing error into [`AppError::Internal`].
impl From<argon2::password_hash::Error> for AppError {
    fn from(e: argon2::password_hash::Error) -> Self {
        AppError::Internal(format!("password hashing error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};

    /// Helper: convert an `AppError` into its HTTP status code.
    fn status(err: AppError) -> StatusCode {
        err.into_response().status()
    }

    // ── Status code mapping ───────────────────────────────────────────────────

    #[test]
    fn not_found_is_404() {
        assert_eq!(
            status(AppError::NotFound("x".into())),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn unauthorized_is_401() {
        assert_eq!(
            status(AppError::Unauthorized("x".into())),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn forbidden_is_403() {
        assert_eq!(
            status(AppError::Forbidden("x".into())),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn conflict_is_409() {
        assert_eq!(status(AppError::Conflict("x".into())), StatusCode::CONFLICT);
    }

    #[test]
    fn validation_is_422() {
        assert_eq!(
            status(AppError::Validation("x".into())),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn database_is_500() {
        assert_eq!(
            status(AppError::Database("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn internal_is_500() {
        assert_eq!(
            status(AppError::Internal("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    // ── Response body ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn response_body_has_error_key() {
        let resp = AppError::NotFound("x".into()).into_response();
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("error").is_some());
    }
}
