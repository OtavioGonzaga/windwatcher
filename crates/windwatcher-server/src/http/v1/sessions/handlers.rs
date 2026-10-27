use super::dto::{CreateSessionRequest, RefreshSessionRequest, SessionGrantResponse};
use crate::http::{
    HttpState,
    auth::AuthenticatedActor,
    extract::ApiJson,
    middleware::request_id::RequestContext,
    problem::{ApiProblem, ProblemDetails, ProblemKind},
};
use axum::{
    Json,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use time::OffsetDateTime;
use windwatcher::{
    AuthenticateLocalUser, AuthenticateLocalUserError, CreateSession, CreateSessionError,
    RefreshSession, RefreshSessionError, RefreshToken, RevokeSession, RevokeSessionError,
    SecretPassword,
};

fn grant_response(status: StatusCode, grant: windwatcher::SessionGrant) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(SessionGrantResponse::from(grant)),
    )
        .into_response()
}

#[utoipa::path(post, path = "/sessions", operation_id = "createSession", tag = "Sessions",
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "Session persisted; JSON bearer secrets must not be cached", body = SessionGrantResponse,
            headers(("Cache-Control" = String, description = "Always no-store; bearer token grants must not be cached."))),
        (status = 400, description = "Invalid JSON structure", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 401, description = "Invalid credentials", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "User disabled", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 413, description = "Body exceeds 2 MiB", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 415, description = "JSON Content-Type required", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Peer quota exceeded", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 500, description = "Internal failure", body = ProblemDetails, content_type = "application/problem+json")
    ))]
pub(super) async fn create_session(
    State(state): State<HttpState>,
    context: RequestContext,
    ApiJson(mut request): ApiJson<CreateSessionRequest>,
) -> Result<Response, ApiProblem> {
    let password = SecretPassword::new(std::mem::take(&mut *request.password))
        .map_err(|_| ApiProblem::new(ProblemKind::InvalidCredentials, context))?;
    let user = AuthenticateLocalUser::new(&state.storage, &state.passwords)
        .execute(&request.username, &password)
        .await
        .map_err(|error| match error {
            AuthenticateLocalUserError::InvalidCredentials => {
                ApiProblem::new(ProblemKind::InvalidCredentials, context)
            }
            AuthenticateLocalUserError::UserDisabled => {
                ApiProblem::new(ProblemKind::UserDisabled, context)
            }
            _ => ApiProblem::internal(context, &error),
        })?;
    let grant = CreateSession::new(
        &state.storage,
        state.refresh_tokens.as_ref(),
        state.codec.as_ref(),
        state.session,
    )
    .execute(&user, OffsetDateTime::now_utc())
    .await
    .map_err(|error| match error {
        CreateSessionError::UserDisabled => ApiProblem::new(ProblemKind::UserDisabled, context),
        CreateSessionError::UserNotFound => {
            ApiProblem::new(ProblemKind::InvalidCredentials, context)
        }
        _ => ApiProblem::internal(context, &error),
    })?;
    tracing::info!(user_id = %user.id(), session_id = %grant.session().id(), "session_created");
    Ok(grant_response(StatusCode::CREATED, grant))
}

#[utoipa::path(post, path = "/sessions/refresh", operation_id = "refreshSession", tag = "Sessions",
    request_body = RefreshSessionRequest,
    responses(
        (status = 200, description = "Refresh rotated; the old refresh token is immediately invalid", body = SessionGrantResponse,
            headers(("Cache-Control" = String, description = "Always no-store; bearer token grants must not be cached."))),
        (status = 400, description = "Invalid JSON structure", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 401, description = "Invalid refresh token", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 413, description = "Body exceeds 2 MiB", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 415, description = "JSON Content-Type required", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Peer quota exceeded", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 500, description = "Internal failure", body = ProblemDetails, content_type = "application/problem+json")
    ))]
pub(super) async fn refresh_session(
    State(state): State<HttpState>,
    context: RequestContext,
    ApiJson(mut request): ApiJson<RefreshSessionRequest>,
) -> Result<Response, ApiProblem> {
    let token = RefreshToken::parse(std::mem::take(&mut *request.refresh_token))
        .map_err(|_| ApiProblem::new(ProblemKind::InvalidRefreshToken, context))?;
    let grant = RefreshSession::new(
        &state.storage,
        &state.storage,
        state.refresh_tokens.as_ref(),
        state.codec.as_ref(),
        state.session,
    )
    .execute(&token, OffsetDateTime::now_utc())
    .await
    .map_err(|error| match error {
        RefreshSessionError::InvalidRefreshToken => {
            ApiProblem::new(ProblemKind::InvalidRefreshToken, context)
        }
        _ => ApiProblem::internal(context, &error),
    })?;
    tracing::info!(user_id = %grant.session().user_id(), session_id = %grant.session().id(), "session_refreshed");
    Ok(grant_response(StatusCode::OK, grant))
}

/// Revokes future refresh only. Already-issued access tokens remain valid until
/// expiration; logout does not blacklist them. Repeated deletion is idempotent.
#[utoipa::path(delete, path = "/sessions/current", operation_id = "revokeCurrentSession", tag = "Sessions",
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Refresh revoked; existing access tokens remain valid until expiration"),
        (status = 401, description = "Invalid access token", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Peer quota exceeded", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 500, description = "Internal failure", body = ProblemDetails, content_type = "application/problem+json")
    ))]
pub(super) async fn revoke_current_session(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
) -> Result<StatusCode, ApiProblem> {
    // Serialize local commit+invalidation against handshake activation and issuance.
    let _lifecycle = state.realtime.revocation_guard().await;
    match RevokeSession::new(&state.storage)
        .execute(actor.user_id, actor.session_id, OffsetDateTime::now_utc())
        .await
    {
        Ok(()) | Err(RevokeSessionError::SessionNotFound) => {}
        Err(error) => return Err(ApiProblem::internal(context, &error)),
    }
    tracing::info!(user_id = %actor.user_id, session_id = %actor.session_id, "session_revoked");
    state.realtime.invalidate(actor.session_id);
    Ok(StatusCode::NO_CONTENT)
}
