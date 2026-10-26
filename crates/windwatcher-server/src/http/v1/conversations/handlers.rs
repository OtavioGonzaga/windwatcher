use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
};
use time::OffsetDateTime;
use windwatcher::*;

use super::dto::*;
use crate::http::{
    HttpState,
    auth::AuthenticatedActor,
    extract::{ApiJson, ApiPath, ApiQuery, CommandKey, PaginationQuery, identifier},
    middleware::request_id::RequestContext,
    page::PageResponse,
    problem::{ApiProblem, ProblemDetails, ProblemKind},
};

macro_rules! group_error {
    ($name:ident, $error:ident, {$($pattern:pat => $kind:ident),* $(,)?}) => {
        fn $name(context: RequestContext, error: $error) -> ApiProblem {
            let kind = match &error {
                $error::ConversationNotFound | $error::NotMember => ProblemKind::ConversationNotFound,
                $error::DirectOperationNotSupported => ProblemKind::OperationNotSupported,
                $($pattern => ProblemKind::$kind,)*
                _ => return ApiProblem::internal(context, &error),
            };
            ApiProblem::new(kind, context)
        }
    };
}
group_error!(rename_error, RenameGroupConversationError, { RenameGroupConversationError::InsufficientPermissions => ConversationPermissions, RenameGroupConversationError::InvalidTitle(_) => InvalidTitle });
group_error!(add_error, AddGroupMemberError, { AddGroupMemberError::InsufficientPermissions => ConversationPermissions, AddGroupMemberError::ParticipantUnavailable(_) => ParticipantUnavailable });
group_error!(role_error, SetGroupMemberRoleError, { SetGroupMemberRoleError::InsufficientPermissions => ConversationPermissions, SetGroupMemberRoleError::OwnerProtected | SetGroupMemberRoleError::OwnerRoleNotAssignable => OwnerProtected, SetGroupMemberRoleError::TargetNotMember => MemberNotFound });
group_error!(remove_error, RemoveGroupMemberError, { RemoveGroupMemberError::InsufficientPermissions => ConversationPermissions, RemoveGroupMemberError::OwnerProtected => OwnerProtected, RemoveGroupMemberError::TargetNotMember => MemberNotFound });
group_error!(owner_error, TransferGroupOwnershipError, { TransferGroupOwnershipError::InsufficientPermissions => ConversationPermissions, TransferGroupOwnershipError::TargetNotMember => MemberNotFound });
group_error!(leave_error, LeaveGroupConversationError, { LeaveGroupConversationError::OwnershipTransferRequired => OwnershipTransferRequired });

pub(in crate::http) fn create_response<T>(result: CreateResult<T>) -> (StatusCode, HeaderMap, T) {
    let mut headers = HeaderMap::new();
    if result.disposition == CreateDisposition::Replayed {
        headers.insert("idempotency-replayed", HeaderValue::from_static("true"));
    }
    let status = if result.disposition == CreateDisposition::Created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    (status, headers, result.value)
}

async fn detail(
    state: &HttpState,
    actor: UserId,
    id: ConversationId,
    context: RequestContext,
) -> Result<Json<ConversationResponse>, ApiProblem> {
    GetConversationSummary::new(&state.storage)
        .execute(actor, id)
        .await
        .map(|s| Json(s.into()))
        .map_err(|e| ApiProblem::query(context, e))
}
async fn member(
    state: &HttpState,
    actor: UserId,
    conversation: ConversationId,
    target: UserId,
    context: RequestContext,
) -> Result<Json<MemberResponse>, ApiProblem> {
    GetConversationMember::new(&state.storage)
        .execute(actor, conversation, target)
        .await
        .map(|s| Json(s.into()))
        .map_err(|e| ApiProblem::query(context, e))
}

#[utoipa::path(get, path = "/conversations", operation_id = "listConversations", tag = "Conversations", security(("bearerAuth" = [])), params(PaginationQuery), responses((status = 200, body = PageResponse<ConversationResponse>), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 422, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn list(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiQuery(query): ApiQuery<PaginationQuery>,
) -> Result<Json<PageResponse<ConversationResponse>>, ApiProblem> {
    ListConversations::new(&state.storage)
        .execute(actor.user_id, query.page(context)?)
        .await
        .map(|p| Json(p.into()))
        .map_err(|e| ApiProblem::query(context, e))
}

#[utoipa::path(post, path = "/conversations", operation_id = "createConversation", tag = "Conversations", security(("bearerAuth" = [])), request_body = CreateConversationRequest,
    params(("Idempotency-Key" = Option<String>, Header, min_length = 1, max_length = 128, description = "Optional exact visible ASCII key; 24-hour semantic retry guarantee. Different input with the same active key conflicts.")),
    responses((status = 201, body = ConversationResponse), (status = 200, body = ConversationResponse, headers(("Idempotency-Replayed" = String, description = "Present as true only on a matching replay"))), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 409, body = ProblemDetails), (status = 422, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn create(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    CommandKey(key): CommandKey,
    ApiJson(request): ApiJson<CreateConversationRequest>,
) -> Result<(StatusCode, HeaderMap, Json<ConversationResponse>), ApiProblem> {
    let request = match request {
        CreateConversationRequest::Direct(request) => {
            let DirectKind::Direct = request.kind;
            ConversationCreateRequest::Direct {
                participant_id: identifier(&request.participant_id, context)?,
            }
        }
        CreateConversationRequest::Group(request) => {
            let GroupKind::Group = request.kind;
            ConversationCreateRequest::Group {
                title: request.title,
                participant_ids: request
                    .participant_ids
                    .iter()
                    .map(|s| identifier(s, context))
                    .collect::<Result<_, _>>()?,
            }
        }
    };
    let result = CreateConversation::new(&state.storage)
        .execute(
            actor.user_id,
            request,
            key.as_ref(),
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|e| ApiProblem::command(context, e))?;
    let (status, headers, id) = create_response(result);
    Ok((
        status,
        headers,
        detail(&state, actor.user_id, id, context).await?,
    ))
}

#[utoipa::path(get, path = "/conversations/{conversation_id}", operation_id = "getConversation", tag = "Conversations", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid)), responses((status = 200, body = ConversationResponse), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn get(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<ConversationResponse>, ApiProblem> {
    detail(&state, actor.user_id, identifier(&id, context)?, context).await
}

#[utoipa::path(patch, path = "/conversations/{conversation_id}", operation_id = "updateConversation", tag = "Conversations", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid)), request_body = UpdateConversationRequest, responses((status = 200, body = ConversationResponse), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 403, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 409, body = ProblemDetails), (status = 422, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn update(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
    ApiJson(request): ApiJson<UpdateConversationRequest>,
) -> Result<Json<ConversationResponse>, ApiProblem> {
    let id = identifier(&id, context)?;
    RenameGroupConversation::new(&state.storage)
        .execute(actor.user_id, id, request.title, OffsetDateTime::now_utc())
        .await
        .map_err(|e| rename_error(context, e))?;
    detail(&state, actor.user_id, id, context).await
}

#[utoipa::path(get, path = "/conversations/{conversation_id}/members", operation_id = "listConversationMembers", tag = "Conversations", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid), PaginationQuery), responses((status = 200, body = PageResponse<MemberResponse>), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 422, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn list_members(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
    ApiQuery(query): ApiQuery<PaginationQuery>,
) -> Result<Json<PageResponse<MemberResponse>>, ApiProblem> {
    ListConversationMembers::new(&state.storage)
        .execute(
            actor.user_id,
            identifier(&id, context)?,
            query.page(context)?,
        )
        .await
        .map(|p| Json(p.into()))
        .map_err(|e| ApiProblem::query(context, e))
}

#[utoipa::path(put, path = "/conversations/{conversation_id}/members/{user_id}", operation_id = "addConversationMember", tag = "Conversations", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid), ("user_id" = String, Path, format = Uuid)), responses((status = 201, body = MemberResponse), (status = 200, body = MemberResponse), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 403, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 409, body = ProblemDetails), (status = 422, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn add_member(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath((id, target)): ApiPath<(String, String)>,
) -> Result<(StatusCode, Json<MemberResponse>), ApiProblem> {
    let id = identifier(&id, context)?;
    let target = identifier(&target, context)?;
    let outcome = AddGroupMember::new(&state.storage)
        .execute(actor.user_id, id, target, OffsetDateTime::now_utc())
        .await
        .map_err(|e| add_error(context, e))?;
    let status = if outcome == AddGroupMemberOutcome::Added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        member(&state, actor.user_id, id, target, context).await?,
    ))
}

#[utoipa::path(patch, path = "/conversations/{conversation_id}/members/{user_id}", operation_id = "updateConversationMember", tag = "Conversations", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid), ("user_id" = String, Path, format = Uuid)), request_body = UpdateMemberRequest, responses((status = 200, body = MemberResponse), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 403, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 409, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn update_member(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath((id, target)): ApiPath<(String, String)>,
    ApiJson(request): ApiJson<UpdateMemberRequest>,
) -> Result<Json<MemberResponse>, ApiProblem> {
    let id = identifier(&id, context)?;
    let target = identifier(&target, context)?;
    SetGroupMemberRole::new(&state.storage)
        .execute(
            actor.user_id,
            id,
            target,
            request.role.into(),
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|e| role_error(context, e))?;
    member(&state, actor.user_id, id, target, context).await
}

#[utoipa::path(delete, path = "/conversations/{conversation_id}/members/{user_id}", operation_id = "removeConversationMember", tag = "Conversations", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid), ("user_id" = String, Path, format = Uuid)), responses((status = 204, description = "Member removed"), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 403, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 409, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn remove_member(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath((id, target)): ApiPath<(String, String)>,
) -> Result<StatusCode, ApiProblem> {
    RemoveGroupMember::new(&state.storage)
        .execute(
            actor.user_id,
            identifier(&id, context)?,
            identifier(&target, context)?,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|e| remove_error(context, e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(put, path = "/conversations/{conversation_id}/owner", operation_id = "transferConversationOwnership", tag = "Conversations", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid)), request_body = TransferOwnerRequest, responses((status = 200, body = ConversationResponse), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 403, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 409, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn transfer_owner(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
    ApiJson(request): ApiJson<TransferOwnerRequest>,
) -> Result<Json<ConversationResponse>, ApiProblem> {
    let id = identifier(&id, context)?;
    TransferGroupOwnership::new(&state.storage)
        .execute(
            actor.user_id,
            id,
            identifier(&request.user_id, context)?,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|e| owner_error(context, e))?;
    detail(&state, actor.user_id, id, context).await
}

#[utoipa::path(delete, path = "/conversations/{conversation_id}/members/me", operation_id = "leaveConversation", tag = "Conversations", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid)), responses((status = 204, description = "Left or deleted owner-only Group"), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 409, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn leave(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiProblem> {
    LeaveGroupConversation::new(&state.storage)
        .execute(
            actor.user_id,
            identifier(&id, context)?,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|e| leave_error(context, e))?;
    Ok(StatusCode::NO_CONTENT)
}
