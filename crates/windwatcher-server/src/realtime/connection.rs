use super::{
    Prepared, RealtimeService,
    hub::{Close, INTERNAL, POLICY},
};
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::time::{Instant, sleep_until, timeout};

async fn write(
    future: impl std::future::Future<Output = Result<(), axum::Error>>,
    expires: Instant,
    wait: std::time::Duration,
    control: &mut tokio::sync::watch::Receiver<Option<Close>>,
) -> Result<(), Close> {
    tokio::select! {
        biased;
        _ = sleep_until(expires) => Err(POLICY),
        _ = control.changed() => Err(control.borrow().unwrap_or(INTERNAL)),
        result = timeout(wait, future) => match result {
            Ok(Ok(())) => Ok(()),
            _ => Err(INTERNAL),
        }
    }
}

pub(super) async fn run(service: RealtimeService, socket: WebSocket, prepared: Prepared) {
    let Prepared {
        mut registration,
        session,
    } = prepared;
    let config = &service.0.config;
    let (mut sink, mut source) = socket.split();
    let expires_in = (session.expires_at() - time::OffsetDateTime::now_utc()).unsigned_abs();
    let expires = if session.is_expired(time::OffsetDateTime::now_utc()) {
        Instant::now()
    } else {
        Instant::now() + expires_in
    };
    let mut heartbeat = Instant::now() + config.heartbeat_interval;
    let mut recheck = Instant::now() + config.session_recheck_interval;
    let mut pending: Option<(Vec<u8>, Instant)> = None;
    let mut sequence = 0u64;
    tracing::info!(connection_id = %registration.guard.id.0, user_id = %session.user_id(), session_id = %session.id(), "connection_opened");
    let close = loop {
        if let Some(close) = *registration.control.borrow() {
            break close;
        }
        let pong_deadline = pending
            .as_ref()
            .map_or(heartbeat, |(_, deadline)| *deadline);
        tokio::select! {
            biased;
            _ = sleep_until(expires) => break POLICY,
            changed = registration.control.changed() => {
                if changed.is_err() { break INTERNAL; }
            }
            _ = sleep_until(pong_deadline) => {
                if pending.is_some() {
                    tracing::info!(connection_id = %registration.guard.id.0, "heartbeat_timeout");
                    break Close { code: 1001, reason: "heartbeat timeout" };
                }
                sequence = sequence.wrapping_add(1);
                let nonce = sequence.to_be_bytes().to_vec();
                if let Err(close) = write(sink.send(Message::Ping(nonce.clone().into())), expires, config.heartbeat_timeout, &mut registration.control).await { break close; }
                pending = Some((nonce, Instant::now() + config.heartbeat_timeout));
                heartbeat = Instant::now() + config.heartbeat_interval;
            }
            _ = sleep_until(recheck) => {
                let validation = tokio::select! {
                    biased;
                    _ = sleep_until(expires) => break POLICY,
                    _ = registration.control.changed() => continue,
                    result = timeout(config.heartbeat_timeout, service.validate(session.user_id(), session.id())) => result,
                };
                match validation {
                    Ok(Ok(_)) => {},
                    Ok(Err(super::Error::SessionInactive)) => break POLICY,
                    _ => break INTERNAL,
                }
                recheck = Instant::now() + config.session_recheck_interval;
            }
            message = source.next() => match message {
                Some(Ok(Message::Pong(value))) => {
                    if pending.as_ref().is_some_and(|(nonce, _)| nonce.as_slice() == value.as_ref()) { pending = None; }
                },
                Some(Ok(Message::Ping(_))) => {
                    // Tungstenite queues the protocol-mandated Pong; flushing sends it.
                    if let Err(close) = write(sink.flush(), expires, config.heartbeat_timeout, &mut registration.control).await { break close; }
                },
                Some(Ok(Message::Close(_))) | None => break Close { code: 1000, reason: "normal close" },
                Some(Ok(Message::Text(_) | Message::Binary(_))) => break Close { code: 1003, reason: "client application data unsupported" },
                Some(Err(_)) => break INTERNAL,
            },
            event = registration.events.recv() => {
                let Some(frame) = event else { break INTERNAL; };
                if let Err(close) = write(sink.send(Message::Text(frame.as_ref().to_owned().into())), expires, config.heartbeat_timeout, &mut registration.control).await { break close; }
            }
        }
    };
    tracing::info!(connection_id = %registration.guard.id.0, close_code = close.code, reason = close.reason, "connection_closing");
    // Stop eligibility before waiting on an unresponsive peer's closing handshake.
    drop(registration.guard);
    let _ = timeout(
        config.heartbeat_timeout,
        sink.send(Message::Close(Some(CloseFrame {
            code: close.code,
            reason: close.reason.into(),
        }))),
    )
    .await;
}
