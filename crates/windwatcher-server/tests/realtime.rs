use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{net::SocketAddr, time::Duration};
use tempfile::TempDir;
use time::OffsetDateTime;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Error as WsError, Message},
};
use tower::ServiceExt;
use windwatcher::*;
use windwatcher_server::{
    RawHttp, RawRealtime, Storage, TokioPasswordHasher,
    http::{HttpState, router},
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
struct Fixture {
    _directory: TempDir,
    storage: Storage,
    router: Router,
    address: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.task.abort();
    }
}
fn codec() -> Hs256JwtAccessTokenCodec {
    Hs256JwtAccessTokenCodec::new(
        Hs256SigningSecret::new(vec![7; 32]).unwrap(),
        "windwatcher".into(),
    )
}
impl Fixture {
    async fn new(config: RawRealtime) -> Self {
        let directory = TempDir::new().unwrap();
        let storage = Storage::connect(SqliteStorageOptions::new(
            directory.path().join("realtime.db").display().to_string(),
        ))
        .await
        .unwrap();
        storage.migrate().await.unwrap();
        let state = HttpState::new(
            storage.clone(),
            TokioPasswordHasher::default(),
            codec(),
            RegistrationPolicy::Closed,
            SessionPolicy::default(),
            MessagePolicy::default(),
        )
        .with_realtime_config(config.resolve().unwrap());
        let mut http = RawHttp::default();
        http.rate_limit.enabled = false;
        let router = router(state, http.resolve().unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let app = router.clone();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
        });
        Self {
            _directory: directory,
            storage,
            router,
            address,
            stop: Some(stop),
            task,
        }
    }
    async fn user(&self, username: &str) -> User {
        let user = User::new(
            UserId::new(),
            username.parse().unwrap(),
            OffsetDateTime::now_utc(),
        );
        let hash = PasswordHash::parse(
            "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA",
        )
        .unwrap();
        self.storage.create_local_user(&user, &hash).await.unwrap();
        user
    }
    async fn session(&self, user: &User) -> (Session, String) {
        self.session_until(user, OffsetDateTime::now_utc() + time::Duration::days(1))
            .await
    }
    async fn session_until(&self, user: &User, expires: OffsetDateTime) -> (Session, String) {
        let now = OffsetDateTime::now_utc();
        let session = Session::new(SessionId::new(), user.id(), now, expires).unwrap();
        let provider = OsRefreshTokenProvider;
        self.storage
            .create_session(&session, &provider.hash(&provider.generate().unwrap()))
            .await
            .unwrap();
        let token = codec()
            .issue(
                &AccessTokenClaims::new(
                    user.id(),
                    session.id(),
                    now,
                    now + time::Duration::hours(1),
                )
                .unwrap(),
            )
            .unwrap();
        (session, token.expose_secret().to_owned())
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Value,
        key: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json");
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        let response = self
            .router
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, headers, value)
    }
    async fn ticket(&self, token: &str) -> String {
        let (status, headers, body) = self
            .request("POST", "/v1/realtime/tickets", token, json!({}), None)
            .await;
        assert_eq!(status, 201, "{body}");
        assert_eq!(headers["cache-control"], "no-store");
        body["ticket"].as_str().unwrap().to_owned()
    }
    async fn connect_ticket(&self, ticket: &str) -> Result<Socket, WsError> {
        let (socket, response) =
            connect_async(format!("ws://{}/v1/realtime?ticket={ticket}", self.address)).await?;
        assert_eq!(response.status(), 101);
        Ok(socket)
    }
    async fn connect(&self, token: &str) -> Socket {
        self.connect_ticket(&self.ticket(token).await)
            .await
            .unwrap()
    }
    async fn mutate(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Value,
        key: Option<&str>,
    ) -> Value {
        let (status, _, body) = self.request(method, path, token, body, key).await;
        assert!(status.is_success(), "{status} {body}");
        body
    }
}
async fn event(socket: &mut Socket, kind: &str) -> Value {
    timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(value) => {
                    let event: Value = serde_json::from_str(&value).unwrap();
                    assert_eq!(event["type"], kind, "{event}");
                    return event;
                }
                Message::Ping(_) => socket.flush().await.unwrap(),
                other => panic!("unexpected frame {other:?}"),
            }
        }
    })
    .await
    .expect("event deadline")
}
async fn none(socket: &mut Socket) {
    assert!(
        timeout(Duration::from_millis(75), async {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Ping(_))) => socket.flush().await.unwrap(),
                    Some(Ok(Message::Pong(_))) => {}
                    other => panic!("unexpected delivery {other:?}"),
                }
            }
        })
        .await
        .is_err()
    );
}
async fn closed(socket: &mut Socket, expected: u16) {
    timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Close(Some(frame)) => {
                    assert_eq!(u16::from(frame.code), expected);
                    return;
                }
                Message::Ping(_) => {}
                other => panic!("unexpected frame {other:?}"),
            }
        }
    })
    .await
    .expect("close deadline");
}
fn rejected(result: Result<Socket, WsError>, expected: u16, code: &str) {
    match result {
        Err(WsError::Http(response)) => {
            assert_eq!(response.status().as_u16(), expected);
            assert!(response.headers().get("www-authenticate").is_none());
            let body: Value = serde_json::from_slice(response.body().as_ref().unwrap()).unwrap();
            assert_eq!(body["code"], code);
        }
        _ => panic!("expected handshake rejection"),
    }
}

#[tokio::test]
async fn tickets_upgrade_errors_and_session_lifecycle() {
    let f = Fixture::new(RawRealtime::default()).await;
    let user = f.user("alice").await;
    let (session, token) = f.session(&user).await;
    let (_, other_token) = f.session(&user).await;
    let ticket = f.ticket(&token).await;
    let (status, _, _) = f
        .request(
            "GET",
            &format!("/v1/realtime?ticket={ticket}"),
            "",
            Value::Null,
            None,
        )
        .await;
    assert_eq!(status, 426); // A plain GET did not consume the grant.
    let mut a = f.connect_ticket(&ticket).await.unwrap();
    let mut same_session = f.connect(&token).await;
    let mut b = f.connect(&other_token).await;
    rejected(
        f.connect_ticket(&ticket).await,
        401,
        "realtime.invalid_ticket",
    );
    rejected(
        f.connect_ticket("invalid").await,
        401,
        "realtime.invalid_ticket",
    );
    rejected(
        connect_async(format!("ws://{}/v1/realtime", f.address))
            .await
            .map(|(s, _)| s),
        400,
        "request.invalid_query",
    );
    rejected(
        connect_async(format!(
            "ws://{}/v1/realtime?ticket=x&after_event=x",
            f.address
        ))
        .await
        .map(|(s, _)| s),
        400,
        "request.invalid_query",
    );
    let outstanding = f.ticket(&token).await;
    f.mutate("DELETE", "/v1/sessions/current", &token, json!({}), None)
        .await;
    closed(&mut a, 1008).await;
    closed(&mut same_session, 1008).await;
    rejected(
        f.connect_ticket(&outstanding).await,
        401,
        "realtime.invalid_ticket",
    );
    let (status, headers, body) = f
        .request("POST", "/v1/realtime/tickets", &token, json!({}), None)
        .await;
    assert_eq!(status, 401);
    assert_eq!(body["code"], "realtime.session_inactive");
    assert!(headers.get("www-authenticate").is_none());
    assert!(
        f.storage
            .find_session(session.id())
            .await
            .unwrap()
            .unwrap()
            .is_revoked()
    );
    b.send(Message::Ping(vec![1].into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), b.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Pong(_)
    ));
    b.send(Message::Text("{}".into())).await.unwrap();
    closed(&mut b, 1003).await;
}

#[tokio::test]
async fn message_events_origin_multiple_devices_idempotency_and_read_state() {
    let f = Fixture::new(RawRealtime::default()).await;
    let a = f.user("alice").await;
    let b = f.user("bob").await;
    let (_, at) = f.session(&a).await;
    let (_, at2) = f.session(&a).await;
    let (_, bt) = f.session(&b).await;
    let mut a1 = f.connect(&at).await;
    let mut a2 = f.connect(&at2).await;
    let mut bs = f.connect(&bt).await;
    let create = json!({"kind":"direct", "participant_id": b.id().to_string()});
    let conversation = f
        .mutate("POST", "/v1/conversations", &at, create.clone(), None)
        .await;
    let id = conversation["id"].as_str().unwrap();
    for socket in [&mut a1, &mut a2, &mut bs] {
        event(socket, "conversation.created").await;
    }
    f.mutate("POST", "/v1/conversations", &at, create, None)
        .await;
    for socket in [&mut a1, &mut a2, &mut bs] {
        none(socket).await;
    }
    let path = format!("/v1/conversations/{id}/messages");
    let message = f
        .mutate(
            "POST",
            &path,
            &bt,
            json!({"content":"hello"}),
            Some("message-key"),
        )
        .await;
    for socket in [&mut a1, &mut a2, &mut bs] {
        let e = event(socket, "message.created").await;
        assert_eq!(e["payload"]["message"], message);
    }
    let (status, headers, replay) = f
        .request(
            "POST",
            &path,
            &bt,
            json!({"content":"hello"}),
            Some("message-key"),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(headers["idempotency-replayed"], "true");
    assert_eq!(message, replay);
    for socket in [&mut a1, &mut a2, &mut bs] {
        none(socket).await;
    }
    let read = json!({"last_read_message_id":message["id"]});
    let read_path = format!("/v1/conversations/{id}/read-state");
    let read_state = f.mutate("PUT", &read_path, &at, read.clone(), None).await;
    for socket in [&mut a1, &mut a2] {
        let e = event(socket, "conversation.read_state.updated").await;
        assert_eq!(
            e["payload"]["last_read_message_id"],
            read_state["last_read_message_id"]
        );
        assert_eq!(e["payload"]["unread_count"], 0);
    }
    none(&mut bs).await;
    f.mutate("PUT", &read_path, &at, read, None).await;
    for socket in [&mut a1, &mut a2, &mut bs] {
        none(socket).await;
    }
    let item = format!("/v1/messages/{}", message["id"].as_str().unwrap());
    let edited = f
        .mutate("PATCH", &item, &bt, json!({"content":"edited"}), None)
        .await;
    for socket in [&mut a1, &mut a2, &mut bs] {
        assert_eq!(
            event(socket, "message.updated").await["payload"]["message"],
            edited
        );
    }
    f.mutate("PATCH", &item, &bt, json!({"content":"edited"}), None)
        .await;
    for socket in [&mut a1, &mut a2, &mut bs] {
        none(socket).await;
    }
    let deleted = f.mutate("DELETE", &item, &bt, json!({}), None).await;
    assert!(deleted["content"].is_null());
    for socket in [&mut a1, &mut a2, &mut bs] {
        assert_eq!(
            event(socket, "message.deleted").await["payload"]["message"],
            deleted
        );
    }
    f.mutate("DELETE", &item, &bt, json!({}), None).await;
    for socket in [&mut a1, &mut a2, &mut bs] {
        none(socket).await;
    }
}

#[tokio::test]
async fn group_membership_audiences_atomic_ownership_and_no_ops() {
    let f = Fixture::new(RawRealtime::default()).await;
    let a = f.user("alice").await;
    let b = f.user("bob").await;
    let c = f.user("carol").await;
    let (_, at) = f.session(&a).await;
    let (_, bt) = f.session(&b).await;
    let (_, ct) = f.session(&c).await;
    let mut asock = f.connect(&at).await;
    let mut bsock = f.connect(&bt).await;
    let mut csock = f.connect(&ct).await;
    let create = json!({"kind":"group", "title":"Team", "participant_ids":[b.id().to_string()]});
    let group = f
        .mutate(
            "POST",
            "/v1/conversations",
            &at,
            create.clone(),
            Some("group-key"),
        )
        .await;
    let id = group["id"].as_str().unwrap();
    let path = format!("/v1/conversations/{id}");
    for s in [&mut asock, &mut bsock] {
        event(s, "conversation.created").await;
    }
    none(&mut csock).await;
    f.mutate("POST", "/v1/conversations", &at, create, Some("group-key"))
        .await;
    for s in [&mut asock, &mut bsock, &mut csock] {
        none(s).await;
    }
    f.mutate("PATCH", &path, &at, json!({"title":"Engineering"}), None)
        .await;
    for s in [&mut asock, &mut bsock] {
        assert_eq!(
            event(s, "conversation.updated").await["payload"]["title"],
            "Engineering"
        );
    }
    let member_path = format!("{path}/members/{}", c.id());
    f.mutate("PUT", &member_path, &at, json!({}), None).await;
    for s in [&mut asock, &mut bsock, &mut csock] {
        assert_eq!(
            event(s, "conversation.member.added").await["payload"]["member"]["user"]["id"],
            c.id().to_string()
        );
    }
    let (status, _, _) = f.request("GET", &path, &ct, json!({}), None).await;
    assert_eq!(status, 200);
    for (method, target, body) in [
        ("PUT", member_path.clone(), json!({})),
        ("PATCH", member_path.clone(), json!({"role":"member"})),
        ("PATCH", path.clone(), json!({"title":"Engineering"})),
        (
            "PUT",
            format!("{path}/owner"),
            json!({"user_id":a.id().to_string()}),
        ),
    ] {
        f.mutate(method, &target, &at, body, None).await;
        for s in [&mut asock, &mut bsock, &mut csock] {
            none(s).await;
        }
    }
    f.mutate("PATCH", &member_path, &at, json!({"role":"admin"}), None)
        .await;
    for s in [&mut asock, &mut bsock, &mut csock] {
        assert_eq!(
            event(s, "conversation.member.updated").await["payload"]["member"]["role"],
            "admin"
        );
    }
    f.mutate(
        "PUT",
        &format!("{path}/owner"),
        &at,
        json!({"user_id":b.id().to_string()}),
        None,
    )
    .await;
    for s in [&mut asock, &mut bsock, &mut csock] {
        let e = event(s, "conversation.owner.transferred").await;
        assert_eq!(e["payload"]["previous_owner"]["role"], "admin");
        assert_eq!(e["payload"]["new_owner"]["role"], "owner");
    }
    f.mutate("DELETE", &member_path, &bt, json!({}), None).await;
    for s in [&mut asock, &mut bsock, &mut csock] {
        event(s, "conversation.member.removed").await;
    }
    f.mutate(
        "POST",
        &format!("{path}/messages"),
        &bt,
        json!({"content":"members only"}),
        None,
    )
    .await;
    for s in [&mut asock, &mut bsock] {
        event(s, "message.created").await;
    }
    none(&mut csock).await;
    f.mutate(
        "DELETE",
        &format!("{path}/members/me"),
        &at,
        json!({}),
        None,
    )
    .await;
    for s in [&mut asock, &mut bsock] {
        event(s, "conversation.member.removed").await;
    }
    f.mutate(
        "DELETE",
        &format!("{path}/members/me"),
        &bt,
        json!({}),
        None,
    )
    .await;
    event(&mut bsock, "conversation.deleted").await;
    none(&mut asock).await;
    none(&mut csock).await;
}

#[tokio::test]
async fn external_revocation_and_session_expiration_close_connections() {
    let f = Fixture::new(RawRealtime {
        session_recheck_interval: "30ms".into(),
        ..RawRealtime::default()
    })
    .await;
    let user = f.user("alice").await;
    let (session, token) = f.session(&user).await;
    let ticket = f.ticket(&token).await;
    self_revoke(&f, &session).await;
    rejected(
        f.connect_ticket(&ticket).await,
        401,
        "realtime.session_inactive",
    );
    let (session, token) = f.session(&user).await;
    let mut socket = f.connect(&token).await;
    self_revoke(&f, &session).await;
    closed(&mut socket, 1008).await;
    let (_, token) = f
        .session_until(
            &user,
            OffsetDateTime::now_utc() + time::Duration::milliseconds(750),
        )
        .await;
    let mut socket = f.connect(&token).await;
    closed(&mut socket, 1008).await;
}
async fn self_revoke(f: &Fixture, session: &Session) {
    RevokeSession::new(&f.storage)
        .execute(session.user_id(), session.id(), OffsetDateTime::now_utc())
        .await
        .unwrap();
}

#[tokio::test]
async fn heartbeat_matching_pongs_keep_alive_and_missing_pong_closes() {
    let f = Fixture::new(RawRealtime {
        heartbeat_interval: "200ms".into(),
        heartbeat_timeout: "100ms".into(),
        ..RawRealtime::default()
    })
    .await;
    let user = f.user("alice").await;
    let (_, token) = f.session(&user).await;
    let mut healthy = f.connect(&token).await;
    for _ in 0..3 {
        let frame = timeout(Duration::from_secs(2), healthy.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(frame, Message::Ping(_)));
        healthy.flush().await.unwrap(); // Tungstenite sends the matching automatic Pong.
    }
    healthy.close(None).await.unwrap();
    let mut dead = f.connect(&token).await;
    // Do not poll Tungstenite: polling would automatically answer the Ping.
    tokio::time::sleep(Duration::from_millis(400)).await;
    closed(&mut dead, 1001).await;
}

#[test]
fn realtime_openapi_contract() {
    let spec = serde_json::to_value(windwatcher_server::http::openapi()).unwrap();
    let ticket = &spec["paths"]["/v1/realtime/tickets"]["post"];
    let connect = &spec["paths"]["/v1/realtime"]["get"];
    assert_eq!(ticket["operationId"], "createRealtimeTicket");
    assert_eq!(ticket["security"], json!([{"bearerAuth":[]}]));
    assert_eq!(connect["operationId"], "connectRealtime");
    assert_eq!(connect["security"], json!([{"realtimeTicket":[]}]));
    assert!(connect["responses"]["101"]["content"].is_null());
    for code in ["101", "400", "401", "426", "429", "500"] {
        assert!(connect["responses"][code].is_object());
    }
    assert!(connect["responses"]["401"]["headers"]["WWW-Authenticate"].is_null());
    assert_eq!(
        spec["components"]["securitySchemes"]["realtimeTicket"],
        json!({"type":"apiKey","in":"query","name":"ticket"})
    );
    assert!(spec["components"]["schemas"]["RealtimeEvent"].is_object());
}

async fn database(f: &Fixture) -> sqlx::SqliteConnection {
    use sqlx::Connection;
    sqlx::SqliteConnection::connect(&format!(
        "sqlite://{}",
        f._directory.path().join("realtime.db").display()
    ))
    .await
    .unwrap()
}

#[tokio::test]
async fn session_storage_failure_closes_active_socket_fail_closed() {
    let f = Fixture::new(RawRealtime {
        session_recheck_interval: "30ms".into(),
        ..RawRealtime::default()
    })
    .await;
    let user = f.user("alice").await;
    let (_, token) = f.session(&user).await;
    let mut socket = f.connect(&token).await;
    sqlx::query("DROP TABLE sessions")
        .execute(&mut database(&f).await)
        .await
        .unwrap();
    closed(&mut socket, 1011).await;
}

#[tokio::test]
async fn audience_lookup_failure_does_not_roll_back_committed_http_mutation() {
    let f = Fixture::new(RawRealtime::default()).await;
    let user = f.user("alice").await;
    let (_, token) = f.session(&user).await;
    let mut socket = f.connect(&token).await;
    let group = f
        .mutate(
            "POST",
            "/v1/conversations",
            &token,
            json!({"kind":"group", "title":"Team", "participant_ids":[]}),
            None,
        )
        .await;
    event(&mut socket, "conversation.created").await;
    let id: ConversationId = group["id"].as_str().unwrap().parse().unwrap();
    let mut db = database(&f).await;
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut db)
        .await
        .unwrap();
    // Simulate externally corrupted audience data without breaking the actor's
    // valid membership or the primary message transaction.
    sqlx::query("INSERT INTO conversation_members (conversation_id, user_id, role, joined_at) VALUES (?, zeroblob(16), 'member', 0)")
        .bind(uuid::Uuid::from(id).as_bytes().to_vec()).execute(&mut db).await.unwrap();
    let response = f
        .request(
            "POST",
            &format!("/v1/conversations/{id}/messages"),
            &token,
            json!({"content":"committed"}),
            None,
        )
        .await;
    assert_eq!(response.0, 201, "{}", response.2);
    none(&mut socket).await;
    let found = f
        .mutate(
            "GET",
            &format!("/v1/messages/{}", response.2["id"].as_str().unwrap()),
            &token,
            json!({}),
            None,
        )
        .await;
    assert_eq!(found["content"], "committed");
}

#[tokio::test]
async fn concurrent_network_upgrades_have_one_ticket_winner_and_expired_ticket_is_rejected() {
    let f = Fixture::new(RawRealtime {
        ticket_ttl: "100ms".into(),
        ..RawRealtime::default()
    })
    .await;
    let user = f.user("alice").await;
    let (_, token) = f.session(&user).await;
    let raw = f.ticket(&token).await;
    let (a, b) = tokio::join!(f.connect_ticket(&raw), f.connect_ticket(&raw));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    if a.is_err() {
        rejected(a, 401, "realtime.invalid_ticket");
    } else {
        rejected(b, 401, "realtime.invalid_ticket");
    }
    let raw = f.ticket(&token).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    rejected(f.connect_ticket(&raw).await, 401, "realtime.invalid_ticket");
}
