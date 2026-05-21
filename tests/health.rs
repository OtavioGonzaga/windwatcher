mod common;

use axum::http::StatusCode;

#[tokio::test]
async fn health_returns_ok() {
    let state = common::build_state().await;
    let router = common::app(state).route("/health", axum::routing::get(|| async { "ok" }));

    let (status, _) = common::req(router, "GET", "/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
}
