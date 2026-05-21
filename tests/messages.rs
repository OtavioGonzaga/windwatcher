mod common;

use axum::http::StatusCode;
use serde_json::json;
use std::time::Duration;

/// Creates a group room and returns its ID string.
async fn create_room(state: &windwatcher::state::AppState, token: &str) -> String {
    let (_, body) = common::req(
        common::app(state.clone()),
        "POST",
        "/rooms/group",
        Some(token),
        Some(json!({ "title": "Chat", "member_ids": [] })),
    )
    .await;
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn send_message_returns_202_and_message_id() {
    let state = common::build_state().await;
    let token = common::register_and_login(&state, "sender@test.com", "password123").await;
    let room_id = create_room(&state, &token).await;

    let (status, body) = common::req(
        common::app(state),
        "POST",
        &format!("/rooms/{room_id}/messages"),
        Some(&token),
        Some(json!({ "content": "Hello, world!" })),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(
        body["message_id"].as_str().is_some(),
        "must return a message_id"
    );
}

#[tokio::test]
async fn send_message_empty_content_is_validation_error() {
    let state = common::build_state().await;
    let token = common::register_and_login(&state, "sender2@test.com", "password123").await;
    let room_id = create_room(&state, &token).await;

    let (status, _) = common::req(
        common::app(state),
        "POST",
        &format!("/rooms/{room_id}/messages"),
        Some(&token),
        Some(json!({ "content": "" })),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn list_messages_empty_room() {
    let state = common::build_state().await;
    let token = common::register_and_login(&state, "reader@test.com", "password123").await;
    let room_id = create_room(&state, &token).await;

    let (status, body) = common::req(
        common::app(state),
        "GET",
        &format!("/rooms/{room_id}/messages"),
        Some(&token),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_messages_after_send() {
    let state = common::build_state().await;
    let token = common::register_and_login(&state, "chatter@test.com", "password123").await;
    let room_id = create_room(&state, &token).await;

    // Send 2 messages
    for i in 0..2 {
        common::req(
            common::app(state.clone()),
            "POST",
            &format!("/rooms/{room_id}/messages"),
            Some(&token),
            Some(json!({ "content": format!("msg {i}") })),
        )
        .await;
    }

    for _ in 0..40 {
        let (status, body) = common::req(
            common::app(state.clone()),
            "GET",
            &format!("/rooms/{room_id}/messages"),
            Some(&token),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let msgs = body.as_array().unwrap();
        if msgs.len() == 2 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    panic!("messages were not persisted within timeout");
}

#[tokio::test]
async fn mark_as_read_returns_no_content() {
    let state = common::build_state().await;
    let token = common::register_and_login(&state, "reader2@test.com", "password123").await;
    let room_id = create_room(&state, &token).await;

    // Send one message and wait for it to be persisted
    let (_, resp) = common::req(
        common::app(state.clone()),
        "POST",
        &format!("/rooms/{room_id}/messages"),
        Some(&token),
        Some(json!({ "content": "hi" })),
    )
    .await;
    let msg_id = resp["message_id"].as_str().unwrap().to_string();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let (status, _) = common::req(
        common::app(state),
        "PUT",
        &format!("/rooms/{room_id}/read"),
        Some(&token),
        Some(json!({ "message_id": msg_id })),
    )
    .await;

    assert_eq!(status, StatusCode::NO_CONTENT);
}
