//! Shared test utilities for integration tests.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use windwatcher::{
    api::{http::router as http_router, ws::manager::WsManager},
    application::{chat_service::ChatService, user_service::UserService},
    config::{AppConfig, DatabaseProvider, QueueProvider},
    db::seaorm::{SeaOrmChatRepository, SeaOrmUserRepository, setup_database},
    domain::ports,
    jobs::build_job_runtime,
    state::AppState,
};

// ── State builder ──────────────────────────────────────────────────────────────

/// Create a fresh `AppState` backed by an isolated SQLite in-memory database.
///
/// Every call gets a **separate** database - no state leaks between tests.
pub async fn build_state() -> AppState {
    let queue_db_path =
        std::env::temp_dir().join(format!("windwatcher_jobs_test_{}.db", Uuid::now_v7()));
    let queue_url = format!("sqlite://{}?mode=rwc", queue_db_path.to_string_lossy());

    let config = AppConfig {
        database_url: "sqlite::memory:".into(),
        database_provider: DatabaseProvider::Sqlite,
        queue_provider: QueueProvider::Sqlite,
        queue_url,
        queue_name: "chat_messages_test".into(),
        queue_concurrency: 2,
        server_host: "127.0.0.1".into(),
        server_port: 0,
        jwt_secret: "integration-test-secret".into(),
        jwt_expiry_secs: 3600,
    };

    let db = setup_database(&config)
        .await
        .expect("in-memory SQLite setup failed");

    let user_repo: Arc<dyn ports::UserRepository> =
        Arc::new(SeaOrmUserRepository { db: db.clone() });
    let chat_repo: Arc<dyn ports::ChatRepository> =
        Arc::new(SeaOrmChatRepository { db: db.clone() });

    let ws_manager = Arc::new(WsManager::new());
    let job_runtime = build_job_runtime(&config, Arc::clone(&chat_repo), Arc::clone(&ws_manager))
        .await
        .expect("failed to start apalis job runtime");
    let job_queue: Arc<dyn ports::JobQueue> = job_runtime.queue();

    let user_service = Arc::new(UserService::new(
        Arc::clone(&user_repo),
        config.jwt_secret.clone(),
        config.jwt_expiry_secs,
    ));
    let chat_service = Arc::new(ChatService::new(chat_repo, job_queue));

    AppState {
        config: Arc::new(config),
        user_service,
        chat_service,
        ws_manager,
    }
}

/// Build the Axum router wired to a test `AppState`.
pub fn app(state: AppState) -> Router {
    http_router().with_state(state)
}

// ── HTTP helpers ───────────────────────────────────────────────────────────────

/// Fire a single HTTP request against the router and return (status, json_body).
pub async fn req(
    router: Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);

    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }

    let (body_bytes, content_type) = match body {
        Some(v) => (Body::from(v.to_string()), Some("application/json")),
        None => (Body::empty(), None),
    };

    if let Some(ct) = content_type {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }

    let response = router
        .oneshot(builder.body(body_bytes).unwrap())
        .await
        .unwrap();

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

// ── Auth shortcuts ─────────────────────────────────────────────────────────────

/// Register a user and immediately log in, returning the JWT.
#[allow(dead_code)]
pub async fn register_and_login(state: &AppState, email: &str, password: &str) -> String {
    let a = app(state.clone());
    req(
        a,
        "POST",
        "/auth/register",
        None,
        Some(json!({
            "username": email.split('@').next().unwrap_or("user"),
            "email": email,
            "password": password
        })),
    )
    .await;

    let a2 = app(state.clone());
    let (_, body) = req(
        a2,
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": email, "password": password })),
    )
    .await;

    body["token"]
        .as_str()
        .expect("login must return a token")
        .to_string()
}
