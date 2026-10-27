use super::dto::{TicketQuery, TicketResponse};
use crate::{
    http::{
        HttpState,
        auth::AuthenticatedActor,
        extract::ApiQuery,
        middleware::request_id::RequestContext,
        problem::{ApiProblem, ProblemDetails, ProblemKind},
    },
    realtime::Error,
};
use axum::{
    Json,
    extract::{State, WebSocketUpgrade, ws::rejection::WebSocketUpgradeRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

fn problem(error: Error, context: RequestContext) -> ApiProblem {
    match error {
        Error::InvalidTicket => ApiProblem::new(ProblemKind::InvalidTicket, context),
        Error::SessionInactive => ApiProblem::new(ProblemKind::SessionInactive, context),
        other => ApiProblem::internal(context, &other),
    }
}

#[utoipa::path(post, path = "/realtime/tickets", operation_id = "createRealtimeTicket", tag = "Realtime", security(("bearerAuth" = [])), responses(
    (status = 201, body = TicketResponse, headers(("Cache-Control" = String, description = "Always no-store"))),
    (status = 401, body = ProblemDetails), (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)
))]
pub(super) async fn ticket(
    State(state): State<HttpState>,
    context: RequestContext,
    actor: AuthenticatedActor,
) -> Result<Response, ApiProblem> {
    let (ticket, expires_at) = state
        .realtime
        .issue(actor.user_id, actor.session_id)
        .await
        .map_err(|e| problem(e, context))?;
    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(TicketResponse { ticket, expires_at }),
    )
        .into_response())
}

#[utoipa::path(get, path = "/realtime", operation_id = "connectRealtime", tag = "Realtime", security(("realtimeTicket" = [])), params(("ticket" = String, Query, description = "Short-lived one-use secret, never an access JWT")), responses(
    (status = 101, description = "WebSocket established; JSON Text frames described in docs/realtime.md"),
    (status = 400, body = ProblemDetails), (status = 401, body = ProblemDetails), (status = 426, body = ProblemDetails),
    (status = 429, body = ProblemDetails), (status = 500, body = ProblemDetails)
))]
pub(super) async fn connect(
    State(state): State<HttpState>,
    context: RequestContext,
    ApiQuery(query): ApiQuery<TicketQuery>,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiProblem> {
    let ws = ws.map_err(|_| ApiProblem::new(ProblemKind::UpgradeRequired, context))?;
    let prepared = state
        .realtime
        .prepare(&query.ticket)
        .await
        .map_err(|e| problem(e, context))?;
    Ok(ws
        .max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .on_upgrade(move |socket| async move { state.realtime.connect(socket, prepared).await }))
}
