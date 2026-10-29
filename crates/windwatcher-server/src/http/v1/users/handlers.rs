use super::dto::{RegisterUserRequest, UserResponse};
use crate::http::{
    HttpState,
    auth::AuthenticatedActor,
    extract::ApiJson,
    middleware::request_id::RequestContext,
    problem::{ApiProblem, ProblemDetails, ProblemKind},
};
use axum::{Json, extract::State, http::StatusCode};
use time::OffsetDateTime;
use windwatcher::{
    GetUser, GetUserError, RegisterLocalUser, RegisterLocalUserError, RegistrationPolicy,
    SecretPassword,
};

#[utoipa::path(get, path = "/users", operation_id = "findUsers", tag = "Users", security(("bearerAuth" = [])), params(super::dto::UsersQuery), responses((status = 200, body = crate::http::page::PageResponse<super::dto::UserSummaryResponse>), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 422, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn find_users(
    State(state): State<HttpState>,
    context: RequestContext,
    _actor: AuthenticatedActor,
    crate::http::extract::ApiQuery(query): crate::http::extract::ApiQuery<super::dto::UsersQuery>,
) -> Result<Json<crate::http::page::PageResponse<super::dto::UserSummaryResponse>>, ApiProblem> {
    let page = crate::http::extract::PaginationQuery {
        cursor: query.cursor,
        limit: query.limit,
    }
    .page(context)?;
    // Exact lookup never issues a continuation, so no supplied cursor is valid.
    if page.cursor().is_some() {
        return Err(ApiProblem::new(ProblemKind::InvalidCursor, context));
    }
    windwatcher::FindUsers::new(&state.storage)
        .execute(&query.username)
        .await
        .map(|p| Json(p.into()))
        .map_err(|e| ApiProblem::query(context, e))
}

#[utoipa::path(post, path = "/users", operation_id = "registerUser", tag = "Users",
    request_body = RegisterUserRequest,
    responses(
        (status = 201, description = "User registered; no session is created", body = UserResponse),
        (status = 400, description = "Invalid JSON structure", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Registration closed", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "Username taken", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 413, description = "Body exceeds 2 MiB", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 415, description = "JSON Content-Type required", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid username or new password", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Client request quota exceeded", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 500, description = "Internal failure", body = ProblemDetails, content_type = "application/problem+json")
    ))]
pub(super) async fn register_user(
    State(state): State<HttpState>,
    context: RequestContext,
    ApiJson(mut request): ApiJson<RegisterUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiProblem> {
    if state.registration == RegistrationPolicy::Closed {
        return Err(ApiProblem::new(ProblemKind::RegistrationClosed, context));
    }
    let password = SecretPassword::new(std::mem::take(&mut *request.password))
        .map_err(|_| ApiProblem::new(ProblemKind::InvalidPassword, context))?;
    let user = RegisterLocalUser::new(&state.storage, &state.passwords, state.registration)
        .execute(&request.username, &password, OffsetDateTime::now_utc())
        .await
        .map_err(|error| {
            let kind = match &error {
                RegisterLocalUserError::RegistrationClosed => ProblemKind::RegistrationClosed,
                RegisterLocalUserError::InvalidUsername(_) => ProblemKind::InvalidUsername,
                RegisterLocalUserError::InvalidPassword(_) => ProblemKind::InvalidPassword,
                RegisterLocalUserError::UsernameTaken => ProblemKind::UsernameTaken,
                _ => return ApiProblem::internal(context, &error),
            };
            ApiProblem::new(kind, context)
        })?;
    tracing::info!(user_id = %user.id(), "user_registered");
    Ok((StatusCode::CREATED, Json(user.into())))
}

#[utoipa::path(get, path = "/users/me", operation_id = "getCurrentUser", tag = "Users",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Current identity; an already-issued access token remains valid after disabling", body = UserResponse),
        (status = 401, description = "Invalid access token or missing identity", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Client request quota exceeded", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 500, description = "Internal failure", body = ProblemDetails, content_type = "application/problem+json")
    ))]
pub(super) async fn get_current_user(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
) -> Result<Json<UserResponse>, ApiProblem> {
    let user = GetUser::new(&state.storage)
        .execute(actor.user_id)
        .await
        .map_err(|error| match error {
            GetUserError::UserNotFound => ApiProblem::new(ProblemKind::InvalidAccessToken, context),
            _ => ApiProblem::internal(context, &error),
        })?;
    Ok(Json(user.into()))
}
