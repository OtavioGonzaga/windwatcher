use crate::ResolvedHttpConfig;
use crate::http::{
    middleware::request_id::RequestContext,
    problem::{ApiProblem, ProblemKind},
};
use axum::{
    extract::{ConnectInfo, MatchedPath, Request, State},
    http::{Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{
    DefaultKeyedRateLimiter, RateLimiter,
    clock::{Clock, DefaultClock},
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone)]
pub(in crate::http) struct Limits(Arc<Inner>);
struct Inner {
    general: DefaultKeyedRateLimiter<IpAddr>,
    auth: DefaultKeyedRateLimiter<IpAddr>,
    requests: AtomicU64,
}
impl Limits {
    pub fn new(config: &ResolvedHttpConfig) -> Self {
        Self(Arc::new(Inner {
            general: RateLimiter::keyed(config.general),
            auth: RateLimiter::keyed(config.auth),
            requests: AtomicU64::new(0),
        }))
    }
}

pub async fn enforce(State(limits): State<Limits>, request: Request, next: Next) -> Response {
    let context = *request
        .extensions()
        .get::<RequestContext>()
        .expect("outer context");
    let Some(ConnectInfo(peer)) = request.extensions().get::<ConnectInfo<SocketAddr>>() else {
        return ApiProblem::internal(
            context,
            &std::io::Error::other("HTTP peer ConnectInfo missing"),
        )
        .into_response();
    };
    let ip = peer.ip();
    // Opportunistic governor cleanup: no background runtime/lifecycle is needed.
    if limits
        .0
        .requests
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(1024)
    {
        limits.0.general.retain_recent();
        limits.0.auth.retain_recent();
    }
    let auth_request = request.method() == Method::POST
        && request
            .extensions()
            .get::<MatchedPath>()
            .is_some_and(|path| {
                matches!(
                    path.as_str(),
                    "/v1/users" | "/v1/sessions" | "/v1/sessions/refresh"
                )
            });
    let result = limits.0.general.check_key(&ip).and_then(|()| {
        if auth_request {
            limits.0.auth.check_key(&ip)
        } else {
            Ok(())
        }
    });
    if let Err(denied) = result {
        let wait = denied.wait_time_from(DefaultClock::default().now());
        let seconds = wait
            .as_secs()
            .saturating_add(u64::from(wait.subsec_nanos() > 0))
            .max(1);
        let mut response = ApiProblem::new(ProblemKind::RateLimitExceeded, context).into_response();
        response.headers_mut().insert(
            header::RETRY_AFTER,
            seconds.to_string().parse().expect("integer header"),
        );
        return response;
    }
    next.run(request).await
}
