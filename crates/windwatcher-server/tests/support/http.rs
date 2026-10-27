use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
};
use serde_json::{Value, json};
use std::net::SocketAddr;
use tempfile::TempDir;
use time::OffsetDateTime;
use tower::ServiceExt;
use windwatcher::{
    Hs256JwtAccessTokenCodec, Hs256SigningSecret, IdentityStore, PasswordHash, PasswordHasher,
    RegistrationPolicy, SecretPassword, SessionPolicy, SqliteStorageOptions, User,
};
use windwatcher_server::{
    RawHttp, Storage, TokioPasswordHasher,
    http::{HttpState, router},
};

pub struct Fixture {
    pub _directory: TempDir,
    pub storage: Storage,
    pub router: Router,
}
pub fn codec() -> Hs256JwtAccessTokenCodec {
    Hs256JwtAccessTokenCodec::new(
        Hs256SigningSecret::new(vec![7; 32]).unwrap(),
        "windwatcher".into(),
    )
}
pub fn config() -> RawHttp {
    let mut config = RawHttp::default();
    config.rate_limit.enabled = false;
    config
}
impl Fixture {
    pub async fn new(config: RawHttp, registration: RegistrationPolicy, migrate: bool) -> Self {
        let directory = TempDir::new().unwrap();
        let storage = Storage::connect(SqliteStorageOptions::new(
            directory.path().join("http.db").display().to_string(),
        ))
        .await
        .unwrap();
        if migrate {
            storage.migrate().await.unwrap();
        }
        let state = HttpState::new(
            storage.clone(),
            TokioPasswordHasher::default(),
            codec(),
            registration,
            SessionPolicy::default(),
            windwatcher::MessagePolicy::default(),
        );
        Self {
            _directory: directory,
            storage,
            router: router(state, config.resolve().unwrap()),
        }
    }
    pub async fn open() -> Self {
        Self::new(config(), RegistrationPolicy::Open, true).await
    }

    pub async fn user(&self) -> User {
        static HASH: tokio::sync::OnceCell<PasswordHash> = tokio::sync::OnceCell::const_new();
        let hash = HASH
            .get_or_init(|| async {
                TokioPasswordHasher::default()
                    .hash(&SecretPassword::new("test password".into()).unwrap())
                    .await
                    .unwrap()
            })
            .await;
        let user = User::new(
            windwatcher::UserId::new(),
            "alice".parse().unwrap(),
            OffsetDateTime::now_utc(),
        );
        self.storage.create_local_user(&user, hash).await.unwrap();
        user
    }

    pub async fn grant(&self) -> Value {
        let (status, headers, body) = send(
            &self.router,
            command(
                "POST",
                "/v1/sessions",
                json!({"username":"alice", "password":"test password"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(headers["cache-control"], "no-store");
        body
    }
}

pub fn command(method: &str, path: &str, value: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}
pub fn bearer(method: &str, path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}
pub async fn send(router: &Router, mut request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    if request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_none()
    {
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
        ));
    }
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    assert!(!headers.contains_key("set-cookie"));
    assert_eq!(
        uuid::Uuid::parse_str(headers["x-request-id"].to_str().unwrap())
            .unwrap()
            .get_version_num(),
        7
    );
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, headers, body)
}
pub fn problem(response: &(StatusCode, HeaderMap, Value), status: u16, code: &str) {
    assert_eq!(response.0.as_u16(), status, "{}", response.2);
    assert_eq!(response.1["content-type"], "application/problem+json");
    assert_eq!(response.2["status"], status);
    assert_eq!(response.2["code"], code);
    assert_eq!(
        response.2["request_id"],
        response.1["x-request-id"].to_str().unwrap()
    );
    assert!(
        response.2["type"]
            .as_str()
            .unwrap()
            .starts_with("urn:windwatcher:problem:")
    );
    assert!(!response.2["title"].as_str().unwrap().is_empty());
    assert!(!response.2["detail"].as_str().unwrap().is_empty());
    if code == "auth.invalid_access_token" {
        assert_eq!(response.1["www-authenticate"], "Bearer");
    } else if status == 401 {
        assert!(response.1.get("www-authenticate").is_none());
    }
}
