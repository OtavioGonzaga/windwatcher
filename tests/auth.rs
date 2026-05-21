mod common;

use axum::http::StatusCode;
use serde_json::json;

// ── register ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn register_creates_user() {
    let state = common::build_state().await;
    let (status, body) = common::req(
        common::app(state),
        "POST",
        "/auth/register",
        None,
        Some(json!({ "username": "alice", "email": "alice@test.com", "password": "password123" })),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["username"], "alice");
    assert_eq!(body["email"], "alice@test.com");
    assert_eq!(body["role"], "user");
    assert!(
        body.get("password_hash").is_none(),
        "hash must not be exposed"
    );
    assert!(body["id"].as_str().is_some());
}

#[tokio::test]
async fn register_duplicate_email_is_conflict() {
    let state = common::build_state().await;

    common::req(
        common::app(state.clone()),
        "POST",
        "/auth/register",
        None,
        Some(json!({ "username": "alice", "email": "dup@test.com", "password": "password123" })),
    )
    .await;

    let (status, body) = common::req(
        common::app(state),
        "POST",
        "/auth/register",
        None,
        Some(json!({ "username": "alice2", "email": "dup@test.com", "password": "password123" })),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().is_some());
}

#[tokio::test]
async fn register_empty_username_is_validation_error() {
    let state = common::build_state().await;
    let (status, _) = common::req(
        common::app(state),
        "POST",
        "/auth/register",
        None,
        Some(json!({ "username": "", "email": "a@b.com", "password": "password123" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn register_invalid_email_is_validation_error() {
    let state = common::build_state().await;
    let (status, _) = common::req(
        common::app(state),
        "POST",
        "/auth/register",
        None,
        Some(json!({ "username": "alice", "email": "not-an-email", "password": "password123" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn register_short_password_is_validation_error() {
    let state = common::build_state().await;
    let (status, _) = common::req(
        common::app(state),
        "POST",
        "/auth/register",
        None,
        Some(json!({ "username": "alice", "email": "a@b.com", "password": "short" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ── login ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn login_returns_jwt() {
    let state = common::build_state().await;
    let token = common::register_and_login(&state, "bob@test.com", "password123").await;
    assert!(!token.is_empty());
    // JWT has 3 dot-separated parts
    assert_eq!(token.split('.').count(), 3);
}

#[tokio::test]
async fn login_wrong_password_is_unauthorized() {
    let state = common::build_state().await;
    common::register_and_login(&state, "carol@test.com", "correct-password").await;

    let (status, body) = common::req(
        common::app(state),
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "carol@test.com", "password": "wrong-password" })),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].as_str().is_some());
}

#[tokio::test]
async fn login_unknown_email_is_unauthorized() {
    let state = common::build_state().await;
    let (status, _) = common::req(
        common::app(state),
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "nobody@test.com", "password": "password" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
