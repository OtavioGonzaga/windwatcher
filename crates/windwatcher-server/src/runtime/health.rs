use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::Storage;

#[derive(Clone)]
pub(crate) struct RuntimeHealth {
    storage: Storage,
    shutdown: CancellationToken,
    timeout: Duration,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

fn response(status: StatusCode, value: &'static str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(HealthResponse { status: value }),
    )
        .into_response()
}

impl RuntimeHealth {
    pub(crate) fn new(storage: Storage, shutdown: CancellationToken, timeout: Duration) -> Self {
        Self {
            storage,
            shutdown,
            timeout,
        }
    }

    pub(crate) fn router(self) -> Router {
        Router::new()
            .route("/health/live", get(live))
            .route("/health/ready", get(ready))
            .with_state(self)
    }

    async fn is_ready(&self) -> bool {
        inspect(&self.shutdown, self.timeout, async {
            self.storage
                .migration_status()
                .await
                .is_ok_and(|status| status.is_current())
        })
        .await
    }
}

async fn inspect(
    shutdown: &CancellationToken,
    timeout: Duration,
    dependency: impl std::future::Future<Output = bool>,
) -> bool {
    if shutdown.is_cancelled() {
        return false;
    }
    let inspection = tokio::select! {
        biased;
        _ = shutdown.cancelled() => return false,
        result = tokio::time::timeout(timeout, dependency) => result,
    };
    match inspection {
        Ok(true) => !shutdown.is_cancelled(),
        Ok(false) => {
            tracing::warn!("readiness_storage_unavailable");
            false
        }
        Err(_) => {
            tracing::warn!("readiness_timeout");
            false
        }
    }
}

async fn live() -> Response {
    response(StatusCode::OK, "ok")
}

async fn ready(State(health): State<RuntimeHealth>) -> Response {
    if health.is_ready().await {
        response(StatusCode::OK, "ready")
    } else {
        response(StatusCode::SERVICE_UNAVAILABLE, "not_ready")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    async fn fixture() -> (tempfile::TempDir, RuntimeHealth) {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::connect(windwatcher::SqliteStorageOptions::new(
            directory.path().join("health.db").display().to_string(),
        ))
        .await
        .unwrap();
        storage.migrate().await.unwrap();
        (
            directory,
            RuntimeHealth::new(storage, CancellationToken::new(), Duration::from_secs(2)),
        )
    }

    async fn probe(router: Router, path: &str, expected: StatusCode, status: &str) {
        let result = router
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(result.status(), expected);
        assert_eq!(result.headers()[header::CACHE_CONTROL], "no-store");
        let body = to_bytes(result.into_body(), 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"status": status})
        );
    }

    #[tokio::test]
    async fn live_is_independent_of_storage_schema_and_draining() {
        let (directory, health) = fixture().await;
        probe(
            health.clone().router(),
            "/health/ready",
            StatusCode::OK,
            "ready",
        )
        .await;
        use sqlx::Connection;
        let mut connection = sqlx::SqliteConnection::connect(
            &directory.path().join("health.db").display().to_string(),
        )
        .await
        .unwrap();
        sqlx::raw_sql("DROP TABLE _sqlx_migrations")
            .execute(&mut connection)
            .await
            .unwrap();
        probe(
            health.clone().router(),
            "/health/live",
            StatusCode::OK,
            "ok",
        )
        .await;
        probe(
            health.clone().router(),
            "/health/ready",
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
        )
        .await;
        health.shutdown.cancel();
        probe(
            health.router(),
            "/health/ready",
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
        )
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_and_drain_do_not_wait_for_a_busy_storage_pool() {
        let shutdown = CancellationToken::new();
        assert!(!inspect(&shutdown, Duration::from_secs(2), std::future::pending()).await);
        shutdown.cancel();
        assert!(!inspect(&shutdown, Duration::from_secs(2), std::future::pending()).await);
    }

    #[tokio::test]
    async fn operational_routes_bypass_api_policy_but_keep_request_ids() {
        let (_directory, health) = fixture().await;
        let mut raw = crate::config::RawConfig::default();
        raw.auth.jwt.secret = Some("12345678901234567890123456789012".into());
        raw.http.rate_limit.general.burst = 1;
        raw.http.cors.allowed_origins = vec!["*".into()];
        let state = crate::http::HttpState::new(
            health.storage.clone(),
            crate::TokioPasswordHasher::default(),
            crate::config::resolve_jwt(&raw).unwrap(),
            crate::config::resolve_registration(&raw).unwrap(),
            crate::config::resolve_session(&raw).unwrap(),
            crate::config::resolve_messages(&raw),
        );
        let router =
            crate::http::runtime_router(state, raw.http.resolve().unwrap(), health.clone());
        for path in ["/health/live", "/health/ready"] {
            for _ in 0..5 {
                let result = router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(path)
                            .header("Origin", "https://example.com")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(result.status(), StatusCode::OK);
                assert!(result.headers().contains_key("x-request-id"));
                assert!(!result.headers().contains_key("access-control-allow-origin"));
            }
        }
        health.shutdown.cancel();
        probe(
            router,
            "/health/ready",
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
        )
        .await;
        let spec = serde_json::to_value(crate::http::openapi()).unwrap();
        assert!(
            !spec["paths"]
                .as_object()
                .unwrap()
                .keys()
                .any(|p| p.starts_with("/health"))
        );
    }
}
