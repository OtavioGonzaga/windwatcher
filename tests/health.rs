mod common;

use axum::http::StatusCode;

#[tokio::test]
async fn health_returns_ok() {
    let state = common::build_state().await;
    let router = common::app(state).route("/health", axum::routing::get(|| async { "ok" }));

    let (status, _) = common::req(router, "GET", "/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn openapi_json_is_served() {
    let state = common::build_state().await;
    let router = common::app(state);

    let (status, body) = common::req(router, "GET", "/api-docs/openapi.json", None, None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["info"]["title"], "Windwatcher API");
    assert!(body["paths"].get("/auth/login").is_some());
    assert!(
        body["components"]["securitySchemes"]
            .get("bearer_auth")
            .is_some()
    );
}
