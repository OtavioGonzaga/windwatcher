use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

use super::middleware::request_id::RequestContext;

/// RFC 9457 plus stable Windwatcher code and authoritative correlation ID.
#[derive(Serialize, ToSchema)]
pub(super) struct ProblemDetails {
    #[serde(rename = "type")]
    #[schema(example = "urn:windwatcher:problem:auth:invalid-credentials")]
    type_uri: &'static str,
    #[schema(example = "Invalid credentials")]
    title: &'static str,
    #[schema(example = 401)]
    status: u16,
    #[schema(example = "The supplied credentials are invalid.")]
    detail: &'static str,
    #[schema(example = "auth.invalid_credentials")]
    code: &'static str,
    #[schema(format = Uuid, example = "019a0000-0000-7000-8000-000000000001")]
    request_id: String,
}

macro_rules! catalog {
    ($($kind:ident => ($status:ident, $code:literal, $urn:literal, $title:literal, $detail:literal)),+ $(,)?) => {
        #[derive(Clone, Copy)]
        pub(super) enum ProblemKind { $($kind),+ }
        impl ProblemKind {
            fn details(self) -> (StatusCode, &'static str, &'static str, &'static str, &'static str) {
                match self { $(Self::$kind => (StatusCode::$status, $code, concat!("urn:windwatcher:problem:", $urn), $title, $detail)),+ }
            }
        }
    }
}

catalog! {
    InvalidJson => (BAD_REQUEST, "request.invalid_json", "request:invalid-json", "Invalid JSON request", "The body must match the expected JSON structure."),
    UnsupportedMediaType => (UNSUPPORTED_MEDIA_TYPE, "request.unsupported_media_type", "request:unsupported-media-type", "Unsupported media type", "A JSON Content-Type is required."),
    PayloadTooLarge => (PAYLOAD_TOO_LARGE, "request.payload_too_large", "request:payload-too-large", "Payload too large", "The request body exceeds the transport limit."),
    NotFound => (NOT_FOUND, "request.not_found", "request:not-found", "Not found", "The requested API route does not exist."),
    MethodNotAllowed => (METHOD_NOT_ALLOWED, "request.method_not_allowed", "request:method-not-allowed", "Method not allowed", "The method is not supported by this API route."),
    InvalidUsername => (UNPROCESSABLE_ENTITY, "user.invalid_username", "user:invalid-username", "Invalid username", "The username does not satisfy the username policy."),
    InvalidPassword => (UNPROCESSABLE_ENTITY, "user.invalid_password", "user:invalid-password", "Invalid password", "The password does not satisfy the new-password policy."),
    UsernameTaken => (CONFLICT, "user.username_taken", "user:username-taken", "Username taken", "The username is already in use."),
    RegistrationClosed => (FORBIDDEN, "registration.closed", "registration:closed", "Registration closed", "Public registration is closed."),
    InvalidCredentials => (UNAUTHORIZED, "auth.invalid_credentials", "auth:invalid-credentials", "Invalid credentials", "The supplied credentials are invalid."),
    UserDisabled => (FORBIDDEN, "auth.user_disabled", "auth:user-disabled", "User disabled", "The user is disabled."),
    InvalidAccessToken => (UNAUTHORIZED, "auth.invalid_access_token", "auth:invalid-access-token", "Invalid access token", "A valid bearer access token is required."),
    InvalidRefreshToken => (UNAUTHORIZED, "auth.invalid_refresh_token", "auth:invalid-refresh-token", "Invalid refresh token", "The supplied refresh token is invalid."),
    RateLimitExceeded => (TOO_MANY_REQUESTS, "rate_limit.exceeded", "rate-limit:exceeded", "Rate limit exceeded", "The peer request quota has been exceeded."),
    Internal => (INTERNAL_SERVER_ERROR, "internal.error", "internal:error", "Internal server error", "The request could not be completed."),
}

pub(super) struct ApiProblem {
    kind: ProblemKind,
    context: RequestContext,
}
impl ApiProblem {
    pub fn new(kind: ProblemKind, context: RequestContext) -> Self {
        Self { kind, context }
    }

    pub fn internal(context: RequestContext, error: &dyn std::error::Error) -> Self {
        // Operational errors retain their diagnostic sources on the server;
        // the response never includes this chain. Ports forbid secret sources.
        tracing::error!(request_id = %context.request_id, error = ?error, "HTTP operation failed");
        Self::new(ProblemKind::Internal, context)
    }
}
impl IntoResponse for ApiProblem {
    fn into_response(self) -> Response {
        let (status, code, type_uri, title, detail) = self.kind.details();
        let mut response = (
            status,
            Json(ProblemDetails {
                type_uri,
                title,
                status: status.as_u16(),
                detail,
                code,
                request_id: self.context.request_id.to_string(),
            }),
        )
            .into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/problem+json"),
        );
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
        }
        response
    }
}
