use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use time::OffsetDateTime;
use windwatcher::*;

use super::dto::*;
use crate::http::{
    HttpState,
    auth::AuthenticatedActor,
    extract::{
        ApiJson, ApiPath, ApiQuery, CommandKey, PaginationQuery, body_identifier, path_identifier,
    },
    middleware::request_id::RequestContext,
    page::PageResponse,
    problem::{ApiProblem, ProblemDetails, ProblemKind},
};

#[utoipa::path(get, path = "/conversations/{conversation_id}/messages", operation_id = "listMessages", tag = "Messages", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid), PaginationQuery), responses((status = 200, body = PageResponse<MessageResponse>), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 422, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn list(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
    ApiQuery(query): ApiQuery<PaginationQuery>,
) -> Result<Json<PageResponse<MessageResponse>>, ApiProblem> {
    ListMessages::new(&state.storage)
        .execute(
            actor.user_id,
            path_identifier(&id, context)?,
            query.page(context)?,
        )
        .await
        .map(|p| Json(p.into()))
        .map_err(|e| {
            let kind = match &e {
                ListMessagesError::ConversationNotFound | ListMessagesError::NotMember => {
                    ProblemKind::ConversationNotFound
                }
                ListMessagesError::InvalidCursor(_) => ProblemKind::InvalidCursor,
                _ => return ApiProblem::internal(context, &e),
            };
            ApiProblem::new(kind, context)
        })
}

#[utoipa::path(post, path = "/conversations/{conversation_id}/messages", operation_id = "sendMessage", tag = "Messages", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid), ("Idempotency-Key" = Option<String>, Header, min_length = 1, max_length = 128, description = "Optional exact visible ASCII key; 24-hour semantic retry guarantee. Different input with the same active key conflicts.")), request_body = SendMessageRequest,
    responses((status = 201, body = MessageResponse), (status = 200, body = MessageResponse, headers(("Idempotency-Replayed" = String, description = "Present as true only on a matching replay"))), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 409, body = ProblemDetails), (status = 422, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn send(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
    CommandKey(key): CommandKey,
    ApiJson(request): ApiJson<SendMessageRequest>,
) -> Result<(StatusCode, HeaderMap, Json<MessageResponse>), ApiProblem> {
    let reply = request
        .reply_to_message_id
        .as_deref()
        .map(|s| body_identifier(s, context))
        .transpose()?;
    let result = SendMessageCommand::new(&state.storage, &state.messages)
        .execute(
            actor.user_id,
            path_identifier(&id, context)?,
            request.content,
            reply,
            key.as_ref(),
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|e| ApiProblem::command(context, e))?;
    let (status, headers, message) = super::super::conversations::handlers::create_response(result);
    Ok((status, headers, Json(message.into())))
}

#[utoipa::path(get, path = "/messages/{message_id}", operation_id = "getMessage", tag = "Messages", security(("bearerAuth" = [])), params(("message_id" = String, Path, format = Uuid)), responses((status = 200, body = MessageResponse), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn get(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<MessageResponse>, ApiProblem> {
    GetMessage::new(&state.storage)
        .execute(actor.user_id, path_identifier(&id, context)?)
        .await
        .map(|m| Json(m.into()))
        .map_err(|e| match &e {
            GetMessageError::MessageNotFound | GetMessageError::NotMember => {
                ApiProblem::new(ProblemKind::MessageNotFound, context)
            }
            _ => ApiProblem::internal(context, &e),
        })
}

#[utoipa::path(patch, path = "/messages/{message_id}", operation_id = "editMessage", tag = "Messages", security(("bearerAuth" = [])), params(("message_id" = String, Path, format = Uuid)), request_body = EditMessageRequest, responses((status = 200, body = MessageResponse), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 403, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 409, body = ProblemDetails), (status = 422, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn edit(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
    ApiJson(request): ApiJson<EditMessageRequest>,
) -> Result<Json<MessageResponse>, ApiProblem> {
    let result = EditMessage::new(&state.storage, &state.messages)
        .execute(
            actor.user_id,
            path_identifier(&id, context)?,
            request.content,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|e| {
            let kind = match &e {
                EditMessageError::MessageNotFound | EditMessageError::NotMember => {
                    ProblemKind::MessageNotFound
                }
                EditMessageError::NotAuthor => ProblemKind::NotAuthor,
                EditMessageError::MessageDeleted => ProblemKind::MessageDeleted,
                EditMessageError::InvalidContent(_) => ProblemKind::InvalidContent,
                EditMessageError::ContentTooLarge(_) => ProblemKind::ContentTooLarge,
                _ => return ApiProblem::internal(context, &e),
            };
            ApiProblem::new(kind, context)
        })?;
    let (EditMessageOutcome::Changed(message) | EditMessageOutcome::Unchanged(message)) = result;
    Ok(Json(message.into()))
}

#[utoipa::path(delete, path = "/messages/{message_id}", operation_id = "deleteMessage", tag = "Messages", security(("bearerAuth" = [])), params(("message_id" = String, Path, format = Uuid)), responses((status = 200, body = MessageResponse), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 403, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn delete(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<MessageResponse>, ApiProblem> {
    let result = DeleteMessage::new(&state.storage)
        .execute(
            actor.user_id,
            path_identifier(&id, context)?,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|e| {
            let kind = match &e {
                DeleteMessageError::MessageNotFound | DeleteMessageError::NotMember => {
                    ProblemKind::MessageNotFound
                }
                DeleteMessageError::NotAuthorized => ProblemKind::MessagePermissions,
                _ => return ApiProblem::internal(context, &e),
            };
            ApiProblem::new(kind, context)
        })?;
    let (DeleteMessageOutcome::Deleted(message) | DeleteMessageOutcome::AlreadyDeleted(message)) =
        result;
    Ok(Json(message.into()))
}

#[utoipa::path(put, path = "/conversations/{conversation_id}/read-state", operation_id = "updateConversationReadState", tag = "Messages", security(("bearerAuth" = [])), params(("conversation_id" = String, Path, format = Uuid)), request_body = ReadStateRequest, responses((status = 200, body = ReadStateResponse), (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 404, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)))]
pub(super) async fn read_state(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
    ApiPath(id): ApiPath<String>,
    ApiJson(request): ApiJson<ReadStateRequest>,
) -> Result<Json<ReadStateResponse>, ApiProblem> {
    MarkConversationReadThrough::new(&state.storage)
        .execute(
            actor.user_id,
            path_identifier(&id, context)?,
            body_identifier(&request.last_read_message_id, context)?,
        )
        .await
        .map(|r| Json(r.state().into()))
        .map_err(|e| {
            let kind = match &e {
                MarkConversationReadError::ConversationNotFound
                | MarkConversationReadError::NotMember => ProblemKind::ConversationNotFound,
                MarkConversationReadError::MessageNotFound => ProblemKind::MessageNotFound,
                _ => return ApiProblem::internal(context, &e),
            };
            ApiProblem::new(kind, context)
        })
}
