mod common;

use axum::http::StatusCode;

#[tokio::test]
async fn me_returns_authenticated_user() {
    let state = common::build_state().await;
    let token = common::register_and_login(&state, "dave@test.com", "password123").await;

    let (status, body) =
        common::req(common::app(state), "GET", "/users/me", Some(&token), None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], "dave@test.com");
    assert!(body["id"].as_str().is_some());
    assert!(body.get("password_hash").is_none());
}

#[tokio::test]
async fn me_without_token_is_unauthorized() {
    let state = common::build_state().await;
    let (status, body) = common::req(common::app(state), "GET", "/users/me", None, None).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].as_str().is_some());
}

#[tokio::test]
async fn me_with_invalid_token_is_unauthorized() {
    let state = common::build_state().await;
    let (status, _) = common::req(
        common::app(state),
        "GET",
        "/users/me",
        Some("this.is.not.a.valid.jwt"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
