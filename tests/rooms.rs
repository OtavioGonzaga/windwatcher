mod common;

use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn create_direct_room() {
    let state = common::build_state().await;
    let token_a = common::register_and_login(&state, "a@test.com", "password123").await;
    let token_b = common::register_and_login(&state, "b@test.com", "password123").await;

    // Get user B's ID
    let (_, b_profile) = common::req(
        common::app(state.clone()),
        "GET",
        "/users/me",
        Some(&token_b),
        None,
    )
    .await;
    let b_id = b_profile["id"].as_str().unwrap().to_string();

    let (status, body) = common::req(
        common::app(state),
        "POST",
        "/rooms/direct",
        Some(&token_a),
        Some(json!({ "other_user_id": b_id })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["room_type"], "direct");
    assert!(body["direct_room_key"].as_str().is_some());
}

#[tokio::test]
async fn create_direct_room_is_idempotent() {
    let state = common::build_state().await;
    let token_a = common::register_and_login(&state, "aa@test.com", "password123").await;
    let token_b = common::register_and_login(&state, "bb@test.com", "password123").await;

    let (_, b_profile) = common::req(
        common::app(state.clone()),
        "GET",
        "/users/me",
        Some(&token_b),
        None,
    )
    .await;
    let b_id = b_profile["id"].as_str().unwrap().to_string();

    let (_, room1) = common::req(
        common::app(state.clone()),
        "POST",
        "/rooms/direct",
        Some(&token_a),
        Some(json!({ "other_user_id": b_id })),
    )
    .await;

    let (_, room2) = common::req(
        common::app(state),
        "POST",
        "/rooms/direct",
        Some(&token_a),
        Some(json!({ "other_user_id": b_id })),
    )
    .await;

    assert_eq!(room1["id"], room2["id"], "same room must be returned");
}

#[tokio::test]
async fn create_group_room() {
    let state = common::build_state().await;
    let token = common::register_and_login(&state, "owner@test.com", "password123").await;

    let (status, body) = common::req(
        common::app(state),
        "POST",
        "/rooms/group",
        Some(&token),
        Some(json!({ "title": "Alpha Team", "member_ids": [] })),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["room_type"], "group");
    assert_eq!(body["title"], "Alpha Team");
}

#[tokio::test]
async fn create_group_room_empty_title_is_validation_error() {
    let state = common::build_state().await;
    let token = common::register_and_login(&state, "owner2@test.com", "password123").await;

    let (status, _) = common::req(
        common::app(state),
        "POST",
        "/rooms/group",
        Some(&token),
        Some(json!({ "title": "   ", "member_ids": [] })),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn create_room_without_token_is_unauthorized() {
    let state = common::build_state().await;
    let (status, _) = common::req(
        common::app(state),
        "POST",
        "/rooms/group",
        None,
        Some(json!({ "title": "test", "member_ids": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
