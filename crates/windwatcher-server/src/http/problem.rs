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
    ShuttingDown => (SERVICE_UNAVAILABLE, "realtime.shutting_down", "realtime:shutting-down", "Server shutting down", "The server is shutting down. Reconnect later."),
    InvalidTicket => (UNAUTHORIZED, "realtime.invalid_ticket", "realtime:invalid-ticket", "Invalid realtime ticket", "The realtime ticket is invalid or unavailable."),
    SessionInactive => (UNAUTHORIZED, "realtime.session_inactive", "realtime:session-inactive", "Session inactive", "An active session is required."),
    UpgradeRequired => (UPGRADE_REQUIRED, "realtime.upgrade_required", "realtime:upgrade-required", "WebSocket upgrade required", "A valid WebSocket upgrade request is required."),
    InvalidQuery => (BAD_REQUEST, "request.invalid_query", "request:invalid-query", "Invalid query", "The query must match the expected structure."),
    InvalidPath => (BAD_REQUEST, "request.invalid_path", "request:invalid-path", "Invalid path", "The resource identifiers must be valid UUIDs."),
    InvalidIdentifier => (BAD_REQUEST, "request.invalid_identifier", "request:invalid-identifier", "Invalid identifier", "The supplied identifier must be a valid UUID."),
    InvalidLimit => (UNPROCESSABLE_ENTITY, "pagination.invalid_limit", "pagination:invalid-limit", "Invalid page limit", "The page limit must be between 1 and 100."),
    InvalidCursor => (BAD_REQUEST, "pagination.invalid_cursor", "pagination:invalid-cursor", "Invalid cursor", "The continuation cursor is invalid for this query."),
    InvalidIdempotencyKey => (BAD_REQUEST, "idempotency.invalid_key", "idempotency:invalid-key", "Invalid idempotency key", "Supply one key of 1 through 128 visible ASCII bytes."),
    IdempotencyKeyReused => (CONFLICT, "idempotency.key_reused", "idempotency:key-reused", "Idempotency key reused", "The key is already associated with another semantic request."),
    IdempotencyResultUnavailable => (CONFLICT, "idempotency.result_unavailable", "idempotency:result-unavailable", "Idempotent result unavailable", "The original resource is no longer available."),
    ConversationNotFound => (NOT_FOUND, "conversation.not_found", "conversation:not-found", "Conversation not found", "The conversation is unavailable."),
    ParticipantUnavailable => (UNPROCESSABLE_ENTITY, "conversation.participant_unavailable", "conversation:participant-unavailable", "Participant unavailable", "A new member must be an available user."),
    InvalidTitle => (UNPROCESSABLE_ENTITY, "conversation.invalid_title", "conversation:invalid-title", "Invalid title", "The Group title is invalid."),
    OperationNotSupported => (CONFLICT, "conversation.operation_not_supported", "conversation:operation-not-supported", "Operation not supported", "This operation is not supported for Direct conversations."),
    ConversationPermissions => (FORBIDDEN, "conversation.insufficient_permissions", "conversation:insufficient-permissions", "Insufficient permissions", "Your membership role does not permit this operation."),
    MemberNotFound => (NOT_FOUND, "conversation.member_not_found", "conversation:member-not-found", "Member not found", "The target is not a conversation member."),
    OwnerProtected => (CONFLICT, "conversation.owner_protected", "conversation:owner-protected", "Owner protected", "Use ownership transfer to change the Group Owner."),
    OwnershipTransferRequired => (CONFLICT, "conversation.ownership_transfer_required", "conversation:ownership-transfer-required", "Ownership transfer required", "Transfer ownership before leaving a Group with other members."),
    MessageNotFound => (NOT_FOUND, "message.not_found", "message:not-found", "Message not found", "The message is unavailable."),
    InvalidContent => (UNPROCESSABLE_ENTITY, "message.invalid_content", "message:invalid-content", "Invalid content", "Message content must not contain NUL."),
    ContentTooLarge => (UNPROCESSABLE_ENTITY, "message.content_too_large", "message:content-too-large", "Content too large", "Message content exceeds the configured byte policy."),
    ReplyTargetUnavailable => (UNPROCESSABLE_ENTITY, "message.reply_target_unavailable", "message:reply-target-unavailable", "Reply target unavailable", "The reply target must be active in the same conversation."),
    NotAuthor => (FORBIDDEN, "message.not_author", "message:not-author", "Not the author", "Only the message author may edit it."),
    MessagePermissions => (FORBIDDEN, "message.insufficient_permissions", "message:insufficient-permissions", "Insufficient permissions", "You cannot delete this message."),
    MessageDeleted => (CONFLICT, "message.deleted", "message:deleted", "Message deleted", "Deleted messages cannot be edited."),
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
    pub fn query(context: RequestContext, error: windwatcher::MessagingQueryError) -> Self {
        use windwatcher::MessagingQueryError as E;
        let kind = match &error {
            E::MemberNotFound => ProblemKind::MemberNotFound,
            E::ConversationNotFound | E::NotMember => ProblemKind::ConversationNotFound,
            E::InvalidCursor(_) => ProblemKind::InvalidCursor,
            E::InvalidUsername(_) => ProblemKind::InvalidUsername,
            _ => return Self::internal(context, &error),
        };
        Self::new(kind, context)
    }

    pub fn command(context: RequestContext, error: windwatcher::CreateCommandError) -> Self {
        use windwatcher::CreateCommandError as E;
        let kind = match &error {
            E::KeyReused => ProblemKind::IdempotencyKeyReused,
            E::ResultUnavailable => ProblemKind::IdempotencyResultUnavailable,
            E::ConversationNotFound => ProblemKind::ConversationNotFound,
            E::NotMember => ProblemKind::ConversationNotFound,
            E::ParticipantUnavailable => ProblemKind::ParticipantUnavailable,
            E::InvalidTitle(_) => ProblemKind::InvalidTitle,
            E::InvalidContent(_) => ProblemKind::InvalidContent,
            E::ContentTooLarge(_) => ProblemKind::ContentTooLarge,
            E::ReplyTargetUnavailable => ProblemKind::ReplyTargetUnavailable,
            _ => return Self::internal(context, &error),
        };
        Self::new(kind, context)
    }
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
        if matches!(self.kind, ProblemKind::InvalidAccessToken) {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
        }
        response
    }
}
