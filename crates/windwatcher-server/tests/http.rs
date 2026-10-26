#[path = "support/http.rs"]
mod support;

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use serde_json::json;
use support::*;
use windwatcher::{
    AccessToken, AccessTokenClaims, AccessTokenCodec, DisableUser, RegistrationPolicy, SessionId,
    UserId,
};

#[test]
fn openapi_contract_has_exact_routes_security_schemas_and_statuses() {
    let spec = windwatcher_server::http::openapi();
    let encoded = serde_json::to_vec(&spec).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(doc["openapi"], "3.1.0");
    assert_eq!(doc["info"]["title"], "Windwatcher API");
    assert_eq!(doc["info"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(doc["paths"].as_object().unwrap().len(), 5);
    let mut ids = std::collections::HashSet::new();
    for (path, method, id, tag, protected, statuses) in [
        (
            "/v1/users",
            "post",
            "registerUser",
            "Users",
            false,
            vec![
                "201", "400", "403", "409", "413", "415", "422", "429", "500",
            ],
        ),
        (
            "/v1/sessions",
            "post",
            "createSession",
            "Sessions",
            false,
            vec!["201", "400", "401", "403", "413", "415", "429", "500"],
        ),
        (
            "/v1/sessions/refresh",
            "post",
            "refreshSession",
            "Sessions",
            false,
            vec!["200", "400", "401", "413", "415", "429", "500"],
        ),
        (
            "/v1/sessions/current",
            "delete",
            "revokeCurrentSession",
            "Sessions",
            true,
            vec!["204", "401", "429", "500"],
        ),
        (
            "/v1/users/me",
            "get",
            "getCurrentUser",
            "Users",
            true,
            vec!["200", "401", "429", "500"],
        ),
    ] {
        let item = &doc["paths"][path];
        assert_eq!(item.as_object().unwrap().len(), 1);
        let operation = &item[method];
        assert_eq!(operation["operationId"], id);
        assert!(ids.insert(operation["operationId"].as_str().unwrap()));
        assert_eq!(operation["tags"], json!([tag]));
        if protected {
            assert_eq!(operation["security"], json!([{"bearerAuth":[]}]))
        } else {
            assert!(operation["security"].is_null() || operation["security"] == json!([]));
        }
        assert_eq!(
            operation["responses"].as_object().unwrap().len(),
            statuses.len()
        );
        for status in statuses {
            let response = &operation["responses"][status];
            assert!(!response.is_null());
            if status.starts_with('4') || status.starts_with('5') {
                assert_eq!(
                    response["content"]["application/problem+json"]["schema"]["$ref"],
                    "#/components/schemas/ProblemDetails"
                );
                assert!(response["content"]["application/json"].is_null());
            }
        }
    }
    assert_eq!(
        doc["components"]["securitySchemes"]["bearerAuth"],
        json!({"type":"http", "scheme":"bearer", "bearerFormat":"JWT"})
    );
    let schemas = &doc["components"]["schemas"];
    for name in [
        "RegisterUserRequest",
        "UserResponse",
        "CreateSessionRequest",
        "RefreshSessionRequest",
        "SessionGrantResponse",
        "ProblemDetails",
    ] {
        assert!(schemas[name].is_object(), "{name}");
    }
    for (schema, fields) in [
        ("UserResponse", vec!["created_at", "updated_at"]),
        (
            "SessionGrantResponse",
            vec!["access_token_expires_at", "session_expires_at"],
        ),
    ] {
        for field in fields {
            assert_eq!(schemas[schema]["properties"][field]["type"], "string");
            assert_eq!(schemas[schema]["properties"][field]["format"], "date-time");
        }
    }
    for field in ["type", "title", "status", "detail", "code", "request_id"] {
        assert!(
            schemas["ProblemDetails"]["required"]
                .as_array()
                .unwrap()
                .contains(&json!(field))
        );
    }
    assert!(!doc["paths"].as_object().unwrap().contains_key("/docs"));
    assert!(
        !doc["paths"]
            .as_object()
            .unwrap()
            .contains_key("/openapi/v1.json")
    );
}

#[tokio::test]
async fn missing_peer_fails_closed_only_when_limiting_is_enabled() {
    use tower::ServiceExt;
    let mut cfg = config();
    cfg.rate_limit.enabled = true;
    let fixture = Fixture::new(cfg, RegistrationPolicy::Open, true).await;
    let response = fixture
        .router
        .clone()
        .oneshot(bearer("GET", "/v1/users/me", "invalid"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["code"], "internal.error");
    assert_eq!(body["request_id"], request_id);
    let fixture = Fixture::open().await;
    let response = fixture
        .router
        .clone()
        .oneshot(bearer("GET", "/v1/users/me", "invalid"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_refresh_collapses_to_invalid_refresh_without_sleeping() {
    let fixture = Fixture::open().await;
    let user = fixture.user().await;
    let grant = windwatcher::CreateSession::new(
        &fixture.storage,
        &windwatcher::OsRefreshTokenProvider,
        &codec(),
        windwatcher::SessionPolicy::default(),
    )
    .execute(
        &user,
        time::OffsetDateTime::now_utc() - time::Duration::days(31),
    )
    .await
    .unwrap();
    let (_, _, refresh) = grant.into_parts();
    problem(
        &send(
            &fixture.router,
            command(
                "POST",
                "/v1/sessions/refresh",
                json!({"refresh_token":refresh.expose_secret()}),
            ),
        )
        .await,
        401,
        "auth.invalid_refresh_token",
    );
}

#[tokio::test]
async fn tracing_correlates_requests_without_logging_secrets_or_query_strings() {
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };
    #[derive(Clone)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);
    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let fixture = Fixture::open().await;
    fixture.user().await;
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || Buffer(writer.clone()))
        .finish();
    // One global collector avoids callsite-interest races with parallel requests
    // in the rest of this integration-test binary.
    tracing::subscriber::set_global_default(subscriber).unwrap();
    let response = send(
        &fixture.router,
        command(
            "POST",
            "/v1/sessions?secret_query=do-not-log",
            json!({"username":"alice", "password":"test password"}),
        ),
    )
    .await;
    assert_eq!(response.0, StatusCode::CREATED);
    let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(logs.contains(response.1["x-request-id"].to_str().unwrap()));
    assert!(logs.contains("/v1/sessions"));
    assert!(logs.contains("session_created"));
    assert!(logs.contains("latency_ms"));
    for secret in [
        "test password",
        "secret_query",
        "do-not-log",
        response.2["access_token"].as_str().unwrap(),
        response.2["refresh_token"].as_str().unwrap(),
    ] {
        assert!(!logs.contains(secret));
    }
}

#[tokio::test]
async fn documentation_uses_generated_contract_and_local_assets_without_api_quotas() {
    use tower::ServiceExt;
    let mut cfg = config();
    cfg.rate_limit.enabled = true;
    cfg.rate_limit.general.burst = 1;
    cfg.rate_limit.general.period = "1h".into();
    cfg.rate_limit.auth.burst = 1;
    cfg.cors.allowed_origins = vec!["*".into()];
    let fixture = Fixture::new(cfg, RegistrationPolicy::Closed, true).await;
    for _ in 0..3 {
        // No peer information: documentation is outside the API limiter and CORS.
        let request = Request::builder()
            .uri("/openapi/v1.json")
            .header("origin", "https://other.example")
            .body(Body::empty())
            .unwrap();
        let response = fixture.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key("x-request-id"));
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        let served: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            served,
            serde_json::to_value(windwatcher_server::http::openapi()).unwrap()
        );
    }
    for path in [
        "/docs",
        "/docs/",
        "/docs/swagger-ui-bundle.js",
        "/docs/swagger-initializer.js",
    ] {
        let response = fixture
            .router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(response.headers().contains_key("x-request-id"));
        if path == "/docs" && response.status().is_redirection() {
            assert_eq!(response.headers()["location"], "/docs/");
            continue;
        }
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap();
        assert!(!bytes.is_empty());
        if path.ends_with("swagger-initializer.js") {
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains("/openapi/v1.json"));
        }
    }
    // Documentation did not spend the single general or auth request.
    problem(
        &send(
            &fixture.router,
            command(
                "POST",
                "/v1/users",
                json!({"username":"alice", "password":"test password"}),
            ),
        )
        .await,
        403,
        "registration.closed",
    );
}

#[tokio::test]
async fn registration_semantics_and_transport_rejections() {
    let fixture = Fixture::open().await;
    let registered = send(
        &fixture.router,
        command(
            "POST",
            "/v1/users",
            json!({"username":"New_User", "password":"test password"}),
        ),
    )
    .await;
    assert_eq!(registered.0, StatusCode::CREATED);
    assert_eq!(registered.2["username"], "new_user");
    assert_eq!(registered.2.as_object().unwrap().len(), 5);
    assert!(registered.2["disabled_at"].is_null());
    for field in ["created_at", "updated_at"] {
        let value = registered.2[field].as_str().unwrap();
        assert!(value.ends_with('Z'));
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).unwrap();
    }
    for (username, password, status, code) in [
        ("new_user", "test password", 409, "user.username_taken"),
        ("??", "test password", 422, "user.invalid_username"),
        ("another", "short", 422, "user.invalid_password"),
    ] {
        problem(
            &send(
                &fixture.router,
                command(
                    "POST",
                    "/v1/users",
                    json!({"username": username, "password": password}),
                ),
            )
            .await,
            status,
            code,
        );
    }
    problem(
        &send(
            &fixture.router,
            command(
                "POST",
                "/v1/users",
                json!({"username":"another", "password":"x".repeat(1025)}),
            ),
        )
        .await,
        422,
        "user.invalid_password",
    );
    let closed = Fixture::new(config(), RegistrationPolicy::Closed, true).await;
    problem(
        &send(
            &closed.router,
            command(
                "POST",
                "/v1/users",
                json!({"username":"alice", "password":"test password"}),
            ),
        )
        .await,
        403,
        "registration.closed",
    );
}

#[tokio::test]
async fn all_command_dtos_reject_shape_errors_media_types_and_oversized_bodies() {
    let fixture = Fixture::open().await;
    for path in ["/v1/users", "/v1/sessions", "/v1/sessions/refresh"] {
        for body in [
            "{",
            "{}",
            "null",
            "[]",
            r#"{"username":1,"password":"test password","refresh_token":"x"}"#,
            r#"{"username":"alice","password":"test password","refresh_token":"x","unknown":true}"#,
        ] {
            let request = Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap();
            problem(
                &send(&fixture.router, request).await,
                400,
                "request.invalid_json",
            );
        }
        for content_type in [None, Some("text/plain")] {
            let mut request = Request::builder().method("POST").uri(path);
            if let Some(content_type) = content_type {
                request = request.header("content-type", content_type);
            }
            problem(
                &send(&fixture.router, request.body(Body::from("{}")).unwrap()).await,
                415,
                "request.unsupported_media_type",
            );
        }
        let request = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(" ".repeat(2 * 1024 * 1024 + 1)))
            .unwrap();
        problem(
            &send(&fixture.router, request).await,
            413,
            "request.payload_too_large",
        );
    }
}

#[tokio::test]
async fn login_failures_do_not_enumerate_users() {
    let fixture = Fixture::open().await;
    let user = fixture.user().await;
    let mut baseline = None;
    for (username, password) in [
        ("unknown", "test password".to_owned()),
        ("alice", "wrong password".into()),
        ("??", "test password".into()),
        ("alice", "x".repeat(1025)),
    ] {
        let response = send(
            &fixture.router,
            command(
                "POST",
                "/v1/sessions",
                json!({"username": username, "password": password}),
            ),
        )
        .await;
        problem(&response, 401, "auth.invalid_credentials");
        let mut body = response.2;
        body.as_object_mut().unwrap().remove("request_id");
        if let Some(expected) = &baseline {
            assert_eq!(&body, expected);
        } else {
            baseline = Some(body);
        }
    }
    DisableUser::new(&fixture.storage, &fixture.storage)
        .execute(user.id(), time::OffsetDateTime::now_utc())
        .await
        .unwrap();
    problem(
        &send(
            &fixture.router,
            command(
                "POST",
                "/v1/sessions",
                json!({"username":"alice", "password":"test password"}),
            ),
        )
        .await,
        403,
        "auth.user_disabled",
    );
}

#[tokio::test]
async fn session_grants_rotation_and_idempotent_logout_preserve_access_semantics() {
    let fixture = Fixture::open().await;
    let user = fixture.user().await;
    let grant = fixture.grant().await;
    assert_eq!(grant["token_type"], "Bearer");
    assert_eq!(
        uuid::Uuid::parse_str(grant["session_id"].as_str().unwrap())
            .unwrap()
            .get_version_num(),
        7
    );
    for field in ["access_token_expires_at", "session_expires_at"] {
        time::OffsetDateTime::parse(
            grant[field].as_str().unwrap(),
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
    }
    let access = grant["access_token"].as_str().unwrap();
    let refresh = grant["refresh_token"].as_str().unwrap();
    let current = send(&fixture.router, bearer("GET", "/v1/users/me", access)).await;
    assert_eq!(current.0, StatusCode::OK);
    assert_eq!(current.2["id"], user.id().to_string());
    assert_eq!(current.2["username"], "alice");
    let replacement = send(
        &fixture.router,
        command(
            "POST",
            "/v1/sessions/refresh",
            json!({"refresh_token":refresh}),
        ),
    )
    .await;
    assert_eq!(replacement.0, StatusCode::OK);
    assert_eq!(replacement.1["cache-control"], "no-store");
    assert_ne!(replacement.2["refresh_token"], refresh);
    codec()
        .verify(
            &AccessToken::new(replacement.2["access_token"].as_str().unwrap().into()),
            time::OffsetDateTime::now_utc(),
        )
        .unwrap();
    problem(
        &send(
            &fixture.router,
            command(
                "POST",
                "/v1/sessions/refresh",
                json!({"refresh_token":refresh}),
            ),
        )
        .await,
        401,
        "auth.invalid_refresh_token",
    );
    for _ in 0..2 {
        let deleted = send(
            &fixture.router,
            bearer("DELETE", "/v1/sessions/current", access),
        )
        .await;
        assert_eq!(deleted.0, StatusCode::NO_CONTENT);
        assert!(deleted.2.is_null());
    }
    problem(
        &send(
            &fixture.router,
            command(
                "POST",
                "/v1/sessions/refresh",
                json!({"refresh_token": replacement.2["refresh_token"]}),
            ),
        )
        .await,
        401,
        "auth.invalid_refresh_token",
    );
    assert_eq!(
        send(&fixture.router, bearer("GET", "/v1/users/me", access))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn disabled_user_stays_readable_but_cannot_refresh() {
    let fixture = Fixture::open().await;
    let user = fixture.user().await;
    let grant = fixture.grant().await;
    DisableUser::new(&fixture.storage, &fixture.storage)
        .execute(user.id(), time::OffsetDateTime::now_utc())
        .await
        .unwrap();
    let current = send(
        &fixture.router,
        bearer(
            "GET",
            "/v1/users/me",
            grant["access_token"].as_str().unwrap(),
        ),
    )
    .await;
    assert_eq!(current.0, StatusCode::OK);
    assert!(current.2["disabled_at"].is_string());
    problem(
        &send(
            &fixture.router,
            command(
                "POST",
                "/v1/sessions/refresh",
                json!({"refresh_token": grant["refresh_token"]}),
            ),
        )
        .await,
        401,
        "auth.invalid_refresh_token",
    );
}

#[tokio::test]
async fn bearer_extraction_rejects_ambiguous_and_invalid_credentials() {
    let fixture = Fixture::open().await;
    fixture.user().await;
    let grant = fixture.grant().await;
    for path in ["/v1/users/me", "/v1/sessions/current"] {
        let method = if path.ends_with("me") {
            "GET"
        } else {
            "DELETE"
        };
        for value in [
            None,
            Some("Basic abc"),
            Some("Bearer"),
            Some("Bearer "),
            Some("Bearer malformed"),
            Some("Bearer a b"),
            Some("Bearer a, Bearer b"),
        ] {
            let mut request = Request::builder().method(method).uri(path);
            if let Some(value) = value {
                request = request.header("authorization", value);
            }
            problem(
                &send(&fixture.router, request.body(Body::empty()).unwrap()).await,
                401,
                "auth.invalid_access_token",
            );
        }
        let mut duplicate = bearer(method, path, grant["access_token"].as_str().unwrap());
        duplicate
            .headers_mut()
            .append("authorization", "Bearer other".parse().unwrap());
        problem(
            &send(&fixture.router, duplicate).await,
            401,
            "auth.invalid_access_token",
        );
    }
    let now = time::OffsetDateTime::now_utc();
    let claims = AccessTokenClaims::new(
        UserId::new(),
        SessionId::new(),
        now - time::Duration::hours(1),
        now - time::Duration::minutes(1),
    )
    .unwrap();
    let expired = codec().issue(&claims).unwrap();
    problem(
        &send(
            &fixture.router,
            bearer("GET", "/v1/users/me", expired.expose_secret()),
        )
        .await,
        401,
        "auth.invalid_access_token",
    );
    let claims = AccessTokenClaims::new(
        UserId::new(),
        SessionId::new(),
        now,
        now + time::Duration::hours(1),
    )
    .unwrap();
    for (key, issuer) in [(8, "windwatcher"), (7, "other")] {
        let wrong = windwatcher::Hs256JwtAccessTokenCodec::new(
            windwatcher::Hs256SigningSecret::new(vec![key; 32]).unwrap(),
            issuer.into(),
        )
        .issue(&claims)
        .unwrap();
        problem(
            &send(
                &fixture.router,
                bearer("GET", "/v1/users/me", wrong.expose_secret()),
            )
            .await,
            401,
            "auth.invalid_access_token",
        );
    }
    let missing = codec().issue(&claims).unwrap();
    problem(
        &send(
            &fixture.router,
            bearer("GET", "/v1/users/me", missing.expose_secret()),
        )
        .await,
        401,
        "auth.invalid_access_token",
    );
    // Missing session is already in the desired logged-out state.
    assert_eq!(
        send(
            &fixture.router,
            bearer("DELETE", "/v1/sessions/current", missing.expose_secret())
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn malformed_and_unknown_refresh_are_indistinguishable() {
    let fixture = Fixture::open().await;
    for token in ["malformed", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"] {
        problem(
            &send(
                &fixture.router,
                command(
                    "POST",
                    "/v1/sessions/refresh",
                    json!({"refresh_token": token}),
                ),
            )
            .await,
            401,
            "auth.invalid_refresh_token",
        );
    }
}

#[tokio::test]
async fn fallback_errors_preserve_allow_and_override_spoofed_request_ids() {
    let fixture = Fixture::open().await;
    for (method, path, status, code) in [
        ("GET", "/v1/unknown", 404, "request.not_found"),
        ("PUT", "/v1/users", 405, "request.method_not_allowed"),
        ("GET", "/v1", 404, "request.not_found"),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("x-request-id", "attacker-controlled")
            .body(Body::empty())
            .unwrap();
        let response = send(&fixture.router, request).await;
        problem(&response, status, code);
        assert_ne!(response.1["x-request-id"], "attacker-controlled");
        if status == 405 {
            assert!(response.1["allow"].to_str().unwrap().contains("POST"));
        }
    }
}

#[tokio::test]
async fn operational_errors_are_generic_problems() {
    let fixture = Fixture::new(config(), RegistrationPolicy::Open, false).await;
    let response = send(
        &fixture.router,
        command(
            "POST",
            "/v1/users",
            json!({"username":"alice", "password":"test password"}),
        ),
    )
    .await;
    problem(&response, 500, "internal.error");
    assert_eq!(response.2["detail"], "The request could not be completed.");
    assert!(!response.2.to_string().contains("SQLite"));
    let claims = AccessTokenClaims::new(
        UserId::new(),
        SessionId::new(),
        time::OffsetDateTime::now_utc(),
        time::OffsetDateTime::now_utc() + time::Duration::hours(1),
    )
    .unwrap();
    let token = codec().issue(&claims).unwrap();
    problem(
        &send(
            &fixture.router,
            bearer("GET", "/v1/users/me", token.expose_secret()),
        )
        .await,
        500,
        "internal.error",
    );
    problem(
        &send(
            &fixture.router,
            bearer("DELETE", "/v1/sessions/current", token.expose_secret()),
        )
        .await,
        500,
        "internal.error",
    );
}

#[tokio::test]
async fn cors_is_opt_in_and_preflight_bypasses_auth_limits() {
    for origins in [vec![], vec!["https://client.example"], vec!["*"]] {
        let mut cfg = config();
        cfg.cors.allowed_origins = origins.iter().map(|s| (*s).into()).collect();
        cfg.rate_limit.enabled = true;
        cfg.rate_limit.auth.burst = 1;
        let fixture = Fixture::new(cfg, RegistrationPolicy::Closed, true).await;
        for origin in ["https://client.example", "https://other.example"] {
            for _ in 0..3 {
                let request = Request::builder()
                    .method("OPTIONS")
                    .uri("/v1/sessions")
                    .header("origin", origin)
                    .header("access-control-request-method", "POST")
                    .header(
                        "access-control-request-headers",
                        "authorization,content-type",
                    )
                    .body(Body::empty())
                    .unwrap();
                let response = send(&fixture.router, request).await;
                assert_eq!(response.0, StatusCode::OK);
                let permitted = origins.contains(&"*") || origins.contains(&origin);
                assert_eq!(
                    response.1.contains_key("access-control-allow-origin"),
                    permitted
                );
                assert!(!response.1.contains_key("access-control-allow-credentials"));
                assert!(
                    response.1["access-control-allow-headers"]
                        .to_str()
                        .unwrap()
                        .contains("authorization")
                );
            }
        }
        let mut request = command(
            "POST",
            "/v1/users",
            json!({"username":"alice", "password":"test password"}),
        );
        request
            .headers_mut()
            .insert("origin", "https://client.example".parse().unwrap());
        let first = send(&fixture.router, request).await;
        problem(&first, 403, "registration.closed");
        assert_eq!(
            first.1.contains_key("access-control-allow-origin"),
            !origins.is_empty()
        );
        let mut request = command(
            "POST",
            "/v1/users",
            json!({"username":"alice", "password":"test password"}),
        );
        request
            .headers_mut()
            .insert("origin", "https://client.example".parse().unwrap());
        let limited = send(&fixture.router, request).await;
        problem(&limited, 429, "rate_limit.exceeded");
        assert_eq!(
            limited.1.contains_key("access-control-allow-origin"),
            !origins.is_empty()
        );
    }
}

#[tokio::test]
async fn quotas_are_shared_by_peer_not_forwarded_headers_and_auth_is_additional() {
    let mut cfg = config();
    cfg.rate_limit.enabled = true;
    cfg.rate_limit.auth.burst = 1;
    cfg.rate_limit.auth.period = "1h".into();
    cfg.rate_limit.general.burst = 5;
    cfg.rate_limit.general.period = "1h".into();
    let fixture = Fixture::new(cfg, RegistrationPolicy::Closed, true).await;
    problem(
        &send(
            &fixture.router,
            command(
                "POST",
                "/v1/users",
                json!({"username":"alice", "password":"test password"}),
            ),
        )
        .await,
        403,
        "registration.closed",
    );
    for (index, path) in ["/v1/sessions", "/v1/sessions/refresh"].iter().enumerate() {
        let mut request = command("POST", path, json!({}));
        request.headers_mut().insert(
            "x-forwarded-for",
            format!("10.0.0.{}", index + 1).parse().unwrap(),
        );
        request.headers_mut().insert(
            "x-real-ip",
            format!("10.0.0.{}", index + 1).parse().unwrap(),
        );
        let response = send(&fixture.router, request).await;
        problem(&response, 429, "rate_limit.exceeded");
        assert!(
            response.1["retry-after"]
                .to_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
                > 0
        );
    }
    problem(
        &send(&fixture.router, bearer("GET", "/v1/users/me", "invalid")).await,
        401,
        "auth.invalid_access_token",
    );
    problem(
        &send(
            &fixture.router,
            bearer("DELETE", "/v1/sessions/current", "invalid"),
        )
        .await,
        401,
        "auth.invalid_access_token",
    );
    problem(
        &send(&fixture.router, bearer("GET", "/v1/users/me", "invalid")).await,
        429,
        "rate_limit.exceeded",
    );
    let mut other = command(
        "POST",
        "/v1/users",
        json!({"username":"alice", "password":"test password"}),
    );
    other.extensions_mut().insert(ConnectInfo(
        "127.0.0.2:12345".parse::<std::net::SocketAddr>().unwrap(),
    ));
    problem(
        &send(&fixture.router, other).await,
        403,
        "registration.closed",
    );
}
