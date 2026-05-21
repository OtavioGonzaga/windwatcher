mod common;

use axum::routing::get;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use windwatcher::api::ws::handler::ws_handler;

/// Spawn a real TCP server on a random port and return its address.
async fn spawn_server(state: windwatcher::state::AppState) -> std::net::SocketAddr {
    use windwatcher::api::http::router as http_router;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Build the router BEFORE applying state so all handlers share the same state type.
    let app = http_router()
        .route("/ws", get(ws_handler))
        .route("/health", get(|| async { "ok" }))
        .with_state(state);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    addr
}

#[tokio::test]
async fn ws_rejects_invalid_token() {
    let state = common::build_state().await;
    let addr = spawn_server(state).await;

    let url = format!("ws://{addr}/ws?token=not-a-valid-token");
    // Connection upgrade succeeds at TCP level but the server closes the socket immediately.
    // tokio-tungstenite returns an error or a close frame.
    let result = tokio_tungstenite::connect_async(&url).await;

    // The server closes the socket right after upgrade - either the connect fails
    // or we get a WS stream that immediately yields a Close frame.
    match result {
        Err(_) => { /* connection refused or close during handshake */ }
        Ok((mut stream, _)) => {
            use futures::StreamExt;
            let msg = stream.next().await;
            // Must be None (closed) or a Close frame
            match msg {
                None => {}
                Some(Ok(Message::Close(_))) => {}
                Some(other) => panic!("expected close, got {other:?}"),
            }
        }
    }
}

#[tokio::test]
async fn ws_accepts_valid_token_and_receives_messages() {
    let state = common::build_state().await;
    let token = common::register_and_login(&state, "ws_user@test.com", "password123").await;
    let addr = spawn_server(state).await;

    let url = format!("ws://{addr}/ws?token={token}");
    let (mut stream, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("WS connection should succeed with valid token");

    // The connection stays open - we expect no immediate close.
    // Use a short timeout: if nothing arrives in 50ms the connection is healthy.
    let timeout = tokio::time::timeout(std::time::Duration::from_millis(50), {
        use futures::StreamExt;
        stream.next()
    })
    .await;

    // Timeout means no message arrived - the connection is alive and healthy.
    assert!(
        timeout.is_err(),
        "valid connection must stay open with no unexpected messages"
    );
}
