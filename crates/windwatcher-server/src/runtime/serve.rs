use std::{
    future::{Future, IntoFuture},
    time::Duration,
};

use axum::Router;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use super::{health::RuntimeHealth, signal::ShutdownSignal};
use crate::{
    config,
    connection::{self, ConnectionPurpose},
    error::AppError,
    http::{self, HttpState},
    paths::ResolvedPaths,
    realtime::RealtimeService,
};

pub(crate) async fn run(raw: &config::RawConfig, paths: &ResolvedPaths) -> Result<(), AppError> {
    tracing::info!("server_starting");
    let resolved = config::resolve_serve(raw, paths)?;
    let config::server::ResolvedServerConfig {
        bind,
        shutdown_grace_period: grace,
        readiness_timeout,
    } = resolved.server;
    let storage =
        connection::storage(resolved.storage, paths, ConnectionPurpose::ExistingOnly).await?;
    if !storage
        .migration_status()
        .await
        .map_err(AppError::MigrationStatus)?
        .is_current()
    {
        return Err(AppError::SchemaPending);
    }
    tracing::info!(backend = storage.backend_name(), "storage_ready");
    let shutdown = CancellationToken::new();
    let health = RuntimeHealth::new(storage.clone(), shutdown.clone(), readiness_timeout);
    let state = HttpState::new_with_realtime(
        storage,
        crate::TokioPasswordHasher::default(),
        resolved.jwt,
        resolved.registration,
        resolved.session,
        resolved.messages,
        resolved.realtime,
    );
    let realtime = state.realtime();
    let router = http::runtime_router(state, resolved.http, health);
    let signal = ShutdownSignal::register().map_err(AppError::ShutdownSignal)?;
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|source| AppError::ListenerBind { bind, source })?;
    let actual = listener.local_addr().map_err(AppError::ServerRuntime)?;
    tracing::info!(bind = %actual, "server_listening");
    serve_until(listener, router, realtime, shutdown, grace, signal.wait()).await
}

pub(crate) async fn serve_until(
    listener: TcpListener,
    router: Router,
    realtime: RealtimeService,
    shutdown: CancellationToken,
    grace: Duration,
    trigger: impl Future<Output = std::io::Result<()>>,
) -> Result<(), AppError> {
    let server = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown.clone().cancelled_owned())
    .into_future();
    tokio::pin!(server);
    let (result, finished) = tokio::select! {
        biased;
        result = &mut server => (Err(match result { Ok(()) => AppError::ServerTerminated, Err(error) => AppError::ServerRuntime(error) }), true),
        result = trigger => (result.map_err(AppError::ShutdownSignal), false),
    };
    // This deadline includes lifecycle-lock acquisition and every drain future.
    let deadline = tokio::time::Instant::now() + grace;
    shutdown.cancel();
    tracing::info!(
        reason = if result.is_ok() {
            "signal"
        } else {
            "runtime_error"
        },
        "shutdown_requested"
    );
    tracing::info!("server_draining");
    let drain = async {
        let http = async {
            if !finished {
                server.await.map_err(AppError::ServerRuntime)?;
            }
            Ok::<(), AppError>(())
        };
        let realtime = async {
            realtime.begin_shutdown().await;
            realtime.drained().await;
        };
        let (http, ()) = tokio::join!(http, realtime);
        http
    };
    match tokio::time::timeout_at(deadline, drain).await {
        Ok(drained) => {
            result?;
            drained?;
            tracing::info!("shutdown_complete");
            Ok(())
        }
        Err(_) => {
            tracing::warn!("shutdown_grace_expired");
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        sync::oneshot,
    };

    async fn request(address: std::net::SocketAddr, path: &str) -> String {
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut result = String::new();
        client.read_to_string(&mut result).await.unwrap();
        result
    }

    #[tokio::test]
    async fn real_listener_installs_connect_info_and_health_ignores_queue() {
        let directory = tempfile::tempdir().unwrap();
        let paths = ResolvedPaths {
            config_file: directory.path().join("config.toml"),
            data_dir: directory.path().to_path_buf(),
        };
        let mut raw = config::RawConfig::default();
        raw.auth.jwt.secret = Some("12345678901234567890123456789012".into());
        raw.queue = Some(config::RawQueue {
            concurrency: 0,
            ..Default::default()
        });
        let resolved = config::resolve_serve(&raw, &paths).unwrap();
        assert!(config::resolve_full(&raw, &paths).is_err());
        assert!(config::resolve_queue(&raw, &paths).is_err());
        let storage = connection::storage(resolved.storage, &paths, ConnectionPurpose::Migration)
            .await
            .unwrap();
        storage.migrate().await.unwrap();
        let stop = CancellationToken::new();
        let health = RuntimeHealth::new(storage.clone(), stop.clone(), Duration::from_secs(2));
        let state = HttpState::new_with_realtime(
            storage,
            crate::TokioPasswordHasher::default(),
            resolved.jwt,
            resolved.registration,
            resolved.session,
            resolved.messages,
            resolved.realtime,
        );
        let realtime = state.realtime();
        let mut http = crate::RawHttp::default();
        http.rate_limit.general.burst = 1;
        let router = http::runtime_router(state, http.resolve().unwrap(), health);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (send, receive) = oneshot::channel();
        let task = tokio::spawn(serve_until(
            listener,
            router,
            realtime,
            stop,
            Duration::from_secs(2),
            async { receive.await.map_err(std::io::Error::other) },
        ));
        assert!(
            request(address, "/v1/users/me")
                .await
                .starts_with("HTTP/1.1 401")
        );
        assert!(
            request(address, "/v1/users/me")
                .await
                .starts_with("HTTP/1.1 429")
        );
        for path in ["/health/live", "/health/ready", "/openapi/v1.json"] {
            assert!(request(address, path).await.starts_with("HTTP/1.1 200"));
        }
        send.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert!(TcpStream::connect(address).await.is_err());
    }

    #[tokio::test]
    async fn inflight_http_finishes_before_serve_returns() {
        let (_directory, realtime, _) = crate::realtime::tests::fixture().await;
        let (entered, started) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let controls = std::sync::Arc::new(tokio::sync::Mutex::new(Some((entered, released))));
        let router = Router::new().route(
            "/blocked",
            get(move || {
                let controls = controls.clone();
                async move {
                    let (entered, release) = controls.lock().await.take().unwrap();
                    entered.send(()).unwrap();
                    release.await.unwrap();
                    "finished"
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        let (send, receive) = oneshot::channel();
        let task = tokio::spawn(serve_until(
            listener,
            router,
            realtime,
            stop.clone(),
            Duration::from_secs(2),
            async { receive.await.map_err(std::io::Error::other) },
        ));
        let response = tokio::spawn(async move { request(address, "/blocked").await });
        started.await.unwrap();
        send.send(()).unwrap();
        stop.cancelled().await;
        assert!(!task.is_finished());
        release.send(()).unwrap();
        assert!(response.await.unwrap().contains("finished"));
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn global_grace_includes_waiting_for_realtime_lifecycle_lock() {
        let (_directory, realtime, _) = crate::realtime::tests::fixture().await;
        let lock = realtime.revocation_guard().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::time::pause();
        let before = tokio::time::Instant::now();
        serve_until(
            listener,
            Router::new(),
            realtime.clone(),
            CancellationToken::new(),
            Duration::from_secs(3),
            async { Ok(()) },
        )
        .await
        .unwrap();
        assert!(before.elapsed() >= Duration::from_secs(3));
        assert!(before.elapsed() <= Duration::from_millis(3010));
        drop(lock);
    }

    #[tokio::test]
    async fn signal_failure_is_not_successful_shutdown() {
        let (_directory, realtime, _) = crate::realtime::tests::fixture().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let result = serve_until(
            listener,
            Router::new(),
            realtime,
            CancellationToken::new(),
            Duration::from_secs(2),
            async { Err(std::io::Error::other("test")) },
        )
        .await;
        assert!(matches!(result, Err(AppError::ShutdownSignal(_))));
    }

    #[tokio::test]
    async fn real_websockets_receive_shutdown_close_and_admission_returns_503() {
        use axum::{
            body::{Body, to_bytes},
            http::{Request, StatusCode},
        };
        use futures_util::StreamExt;
        use serde_json::{Value, json};
        use tower::ServiceExt;

        async fn json_request(
            router: &Router,
            path: &str,
            body: Value,
            token: Option<&str>,
        ) -> (StatusCode, Value) {
            let mut request = Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json");
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            let response = router
                .clone()
                .oneshot(request.body(Body::from(body.to_string())).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            (status, serde_json::from_slice(&body).unwrap())
        }

        let directory = tempfile::tempdir().unwrap();
        let paths = ResolvedPaths {
            config_file: directory.path().join("config.toml"),
            data_dir: directory.path().to_path_buf(),
        };
        let mut raw = config::RawConfig::default();
        raw.auth.jwt.secret = Some("12345678901234567890123456789012".into());
        raw.auth.registration = "open".into();
        raw.http.rate_limit.enabled = false;
        let resolved = config::resolve_serve(&raw, &paths).unwrap();
        let storage = connection::storage(resolved.storage, &paths, ConnectionPurpose::Migration)
            .await
            .unwrap();
        storage.migrate().await.unwrap();
        let stop = CancellationToken::new();
        let health = RuntimeHealth::new(storage.clone(), stop.clone(), Duration::from_secs(2));
        let state = HttpState::new_with_realtime(
            storage,
            crate::TokioPasswordHasher::default(),
            resolved.jwt,
            resolved.registration,
            resolved.session,
            resolved.messages,
            resolved.realtime,
        );
        let realtime = state.realtime();
        let router = http::runtime_router(state, resolved.http, health);
        let credentials = json!({"username": "alice", "password": "test password"});
        assert_eq!(
            json_request(&router, "/v1/users", credentials.clone(), None)
                .await
                .0,
            StatusCode::CREATED
        );
        let (status, session) = json_request(&router, "/v1/sessions", credentials, None).await;
        assert_eq!(status, StatusCode::CREATED);
        let token = session["access_token"].as_str().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (send, receive) = oneshot::channel();
        let task = tokio::spawn(serve_until(
            listener,
            router.clone(),
            realtime,
            stop.clone(),
            Duration::from_secs(2),
            async { receive.await.map_err(std::io::Error::other) },
        ));
        let mut sockets = Vec::new();
        for _ in 0..2 {
            let (status, ticket) =
                json_request(&router, "/v1/realtime/tickets", json!({}), Some(token)).await;
            assert_eq!(status, StatusCode::CREATED);
            let (socket, _) = tokio_tungstenite::connect_async(format!(
                "ws://{address}/v1/realtime?ticket={}",
                ticket["ticket"].as_str().unwrap()
            ))
            .await
            .unwrap();
            sockets.push(socket);
        }
        send.send(()).unwrap();
        stop.cancelled().await;
        for mut socket in sockets {
            let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let tokio_tungstenite::tungstenite::Message::Close(Some(frame)) = frame else {
                panic!("expected close frame");
            };
            assert_eq!(u16::from(frame.code), 1001);
            assert_eq!(frame.reason, "server shutdown");
        }
        let (status, problem) =
            json_request(&router, "/v1/realtime/tickets", json!({}), Some(token)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(problem["code"], "realtime.shutting_down");
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn blocked_http_cannot_outlive_the_global_grace_budget() {
        let (_directory, realtime, _) = crate::realtime::tests::fixture().await;
        let (entered, started) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let controls = std::sync::Arc::new(tokio::sync::Mutex::new(Some((entered, released))));
        let router = Router::new().route(
            "/blocked",
            get(move || {
                let controls = controls.clone();
                async move {
                    let (entered, release) = controls.lock().await.take().unwrap();
                    entered.send(()).unwrap();
                    let _ = release.await;
                    "finished"
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (send, receive) = oneshot::channel();
        let task = tokio::spawn(serve_until(
            listener,
            router,
            realtime,
            CancellationToken::new(),
            Duration::from_millis(5),
            async { receive.await.map_err(std::io::Error::other) },
        ));
        let response = tokio::spawn(async move { request(address, "/blocked").await });
        started.await.unwrap();
        send.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        // Release the detached HTTP task in this test runtime after proving that
        // the coordinator returned without waiting for its response.
        release.send(()).unwrap();
        response.await.unwrap();
    }
}
