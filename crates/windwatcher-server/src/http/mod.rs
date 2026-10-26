//! HTTP v1 transport for the official server. No listener or process lifecycle.

mod auth;
mod extract;
mod middleware;
mod problem;
mod state;
mod v1;

pub use state::HttpState;

use crate::ResolvedHttpConfig;
use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, MatchedPath, Request},
    http::{StatusCode, header},
    middleware as axum_middleware,
    response::{IntoResponse, Response},
};
use middleware::{
    rate_limit::{Limits, enforce},
    request_id::{RequestContext, assign},
};
use problem::{ApiProblem, ProblemKind};
use tower_http::{
    cors::{Any, CorsLayer},
    sensitive_headers::{SetSensitiveRequestHeadersLayer, SetSensitiveResponseHeadersLayer},
    trace::TraceLayer,
};
use utoipa_axum::router::OpenApiRouter;

async fn not_found(context: RequestContext) -> ApiProblem {
    ApiProblem::new(ProblemKind::NotFound, context)
}

async fn method_problem(request: Request, next: axum_middleware::Next) -> Response {
    let context = *request
        .extensions()
        .get::<RequestContext>()
        .expect("outer context");
    let response = next.run(request).await;
    if response.status() != StatusCode::METHOD_NOT_ALLOWED {
        return response;
    }
    let allow = response.headers().get(header::ALLOW).cloned();
    let mut problem = ApiProblem::new(ProblemKind::MethodNotAllowed, context).into_response();
    if let Some(allow) = allow {
        problem.headers_mut().insert(header::ALLOW, allow);
    }
    problem
}

fn routes(config: &ResolvedHttpConfig) -> OpenApiRouter<HttpState> {
    let mut api = v1::router()
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(axum_middleware::from_fn(method_problem));
    if config.enabled {
        api = api.layer(axum_middleware::from_fn_with_state(
            Limits::new(config),
            enforce,
        ));
    }
    let cors = CorsLayer::new()
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::DELETE,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .expose_headers([
            header::HeaderName::from_static("x-request-id"),
            header::RETRY_AFTER,
        ]);
    let cors = if config.any_origin {
        cors.allow_origin(Any)
    } else {
        cors.allow_origin(config.origins.clone())
    };
    OpenApiRouter::new().nest("/v1", api.layer(cors))
}

/// Builds the HTTP adapter without binding a listener or reading configuration.
/// When limiting is enabled, serving must install `ConnectInfo<SocketAddr>` using
/// Axum's connect-info make-service; forwarded headers are deliberately ignored.
pub fn router(state: HttpState, config: ResolvedHttpConfig) -> Router {
    let (router, _) = routes(&config).split_for_parts();
    router.fallback(not_found).with_state(state)
        .layer(SetSensitiveResponseHeadersLayer::new([header::SET_COOKIE]))
        .layer(TraceLayer::new_for_http()
            .make_span_with(|request: &Request<Body>| {
                let context = request.extensions().get::<RequestContext>().expect("outer context");
                let route = request.extensions().get::<MatchedPath>().map_or("<unmatched>", MatchedPath::as_str);
                tracing::info_span!("http_request", request_id = %context.request_id, method = %request.method(), route, status = tracing::field::Empty, latency_ms = tracing::field::Empty)
            })
            .on_response(|response: &Response<Body>, latency: std::time::Duration, span: &tracing::Span| {
                span.record("status", response.status().as_u16());
                span.record("latency_ms", latency.as_millis() as u64);
                tracing::info!(parent: span, status = response.status().as_u16(), latency_ms = latency.as_millis() as u64, "request_completed");
            }))
        .layer(SetSensitiveRequestHeadersLayer::new([header::AUTHORIZATION, header::COOKIE]))
        .layer(axum_middleware::from_fn(assign))
}
