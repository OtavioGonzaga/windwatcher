#[path = "support/http.rs"]
mod support;

use axum::{
    body::Body,
    http::{HeaderValue, Request, StatusCode},
};
use serde_json::{Value, json};
use support::*;
use windwatcher::{
    AccessTokenClaims, AccessTokenCodec, IdentityStore, PasswordHash, SessionId, User, UserId,
};

async fn participant(f: &Fixture, name: &str) -> (User, String) {
    let now = time::OffsetDateTime::now_utc();
    let user = User::new(UserId::new(), name.parse().unwrap(), now);
    let hash = PasswordHash::parse(
        "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();
    f.storage.create_local_user(&user, &hash).await.unwrap();
    let token = codec()
        .issue(
            &AccessTokenClaims::new(
                user.id(),
                SessionId::new(),
                now,
                now + time::Duration::hours(1),
            )
            .unwrap(),
        )
        .unwrap();
    (user, token.expose_secret().to_owned())
}
fn authorized(method: &str, path: &str, token: &str, value: Value) -> Request<Body> {
    let mut request = command(method, path, value);
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    request
}
fn keyed(mut request: Request<Body>, key: &str) -> Request<Body> {
    request
        .headers_mut()
        .insert("idempotency-key", HeaderValue::from_str(key).unwrap());
    request
}
fn page(value: &Value) {
    assert_eq!(value.as_object().unwrap().len(), 3);
    assert!(value["items"].is_array());
    assert_eq!(value["has_more"], !value["next_cursor"].is_null());
}

#[tokio::test]
async fn group_limits_apply_before_ids_and_do_not_reserve_idempotency_keys() {
    let f = Fixture::open().await;
    let (actor, token) = participant(&f, "limits").await;
    for (body, code) in [
        (
            json!({"kind":"group", "title":"valid", "participant_ids":vec!["not-a-uuid";257]}),
            "conversation.too_many_participants",
        ),
        (
            json!({"kind":"group", "title":"é".repeat(129), "participant_ids":[]}),
            "conversation.invalid_title",
        ),
    ] {
        problem(
            &send(
                &f.router,
                keyed(
                    authorized("POST", "/v1/conversations", &token, body),
                    "limit-key",
                ),
            )
            .await,
            422,
            code,
        );
    }
    let title = "é".repeat(128);
    let created = send(&f.router, keyed(authorized("POST", "/v1/conversations", &token, json!({"kind":"group", "title":title, "participant_ids":vec![actor.id().to_string();256]})), "limit-key")).await;
    assert_eq!(created.0, StatusCode::CREATED, "{}", created.2);
    assert_eq!(created.2["title"], title);
    let id = created.2["id"].as_str().unwrap();
    problem(
        &send(
            &f.router,
            authorized(
                "PATCH",
                &format!("/v1/conversations/{id}"),
                &token,
                json!({"title":"a".repeat(257)}),
            ),
        )
        .await,
        422,
        "conversation.invalid_title",
    );
    let unchanged = send(
        &f.router,
        bearer("GET", &format!("/v1/conversations/{id}"), &token),
    )
    .await;
    assert_eq!(unchanged.2["title"], title);
    let doc = serde_json::to_value(windwatcher_server::http::openapi()).unwrap();
    assert_eq!(
        doc["components"]["schemas"]["CreateGroupRequest"]["properties"]["participant_ids"]["maxItems"],
        256
    );
}

#[tokio::test]
async fn authorization_matrix_rechecks_membership_and_never_grants_moderator_editing() {
    let f = Fixture::open().await;
    let (owner, owner_token) = participant(&f, "matrix_owner").await;
    let (member, member_token) = participant(&f, "matrix_member").await;
    let (_, outsider_token) = participant(&f, "matrix_outsider").await;
    let group = send(
        &f.router,
        authorized(
            "POST",
            "/v1/conversations",
            &owner_token,
            json!({"kind":"group", "title":"matrix", "participant_ids":[member.id().to_string()]}),
        ),
    )
    .await;
    assert_eq!(group.0, StatusCode::CREATED);
    let base = format!("/v1/conversations/{}", group.2["id"].as_str().unwrap());
    let content = json!({"content":"member message"});
    let message = send(
        &f.router,
        keyed(
            authorized(
                "POST",
                &format!("{base}/messages"),
                &member_token,
                content.clone(),
            ),
            "matrix-message",
        ),
    )
    .await;
    assert_eq!(message.0, StatusCode::CREATED);
    let message_id = message.2["id"].as_str().unwrap();
    let message_path = format!("/v1/messages/{message_id}");
    // Owner moderation does not confer authorship.
    problem(
        &send(
            &f.router,
            authorized(
                "PATCH",
                &message_path,
                &owner_token,
                json!({"content":"owner edit"}),
            ),
        )
        .await,
        403,
        "message.not_author",
    );
    let role = send(
        &f.router,
        authorized(
            "PATCH",
            &format!("{base}/members/{}", member.id()),
            &owner_token,
            json!({"role":"admin"}),
        ),
    )
    .await;
    assert_eq!(role.0, StatusCode::OK);
    let owned = send(
        &f.router,
        authorized(
            "POST",
            &format!("{base}/messages"),
            &owner_token,
            json!({"content":"owner message"}),
        ),
    )
    .await;
    problem(
        &send(
            &f.router,
            authorized(
                "PATCH",
                &format!("/v1/messages/{}", owned.2["id"].as_str().unwrap()),
                &member_token,
                json!({"content":"admin edit"}),
            ),
        )
        .await,
        403,
        "message.not_author",
    );
    assert_eq!(
        send(
            &f.router,
            bearer(
                "DELETE",
                &format!("{base}/members/{}", member.id()),
                &owner_token
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    for token in [&outsider_token, &member_token] {
        for (method, path, body) in [
            ("GET", base.clone(), Value::Null),
            ("GET", format!("{base}/members"), Value::Null),
            ("GET", format!("{base}/messages"), Value::Null),
            ("GET", message_path.clone(), Value::Null),
            ("POST", format!("{base}/messages"), content.clone()),
            (
                "POST",
                format!("{base}/messages"),
                json!({"content":"reply", "reply_to_message_id":message_id}),
            ),
            (
                "PUT",
                format!("{base}/read-state"),
                json!({"last_read_message_id":message_id}),
            ),
            ("PATCH", base.clone(), json!({"title":"denied"})),
            ("PUT", format!("{base}/members/{}", owner.id()), Value::Null),
            (
                "PUT",
                format!("{base}/owner"),
                json!({"user_id":owner.id().to_string()}),
            ),
        ] {
            let response = send(&f.router, authorized(method, &path, token, body)).await;
            assert_eq!(
                response.0,
                StatusCode::NOT_FOUND,
                "{method} {path}: {}",
                response.2
            );
        }
    }
    // A previously successful key must not bypass current membership.
    problem(
        &send(
            &f.router,
            keyed(
                authorized("POST", &format!("{base}/messages"), &member_token, content),
                "matrix-message",
            ),
        )
        .await,
        404,
        "conversation.not_found",
    );
    let unchanged = send(&f.router, bearer("GET", &message_path, &owner_token)).await;
    assert_eq!(unchanged.2["content"], "member message");
}

#[test]
fn openapi_messaging_surface_has_explicit_security_pages_discriminator_and_headers() {
    let doc = serde_json::to_value(windwatcher_server::http::openapi()).unwrap();
    let mut operation_ids = std::collections::HashSet::new();
    for (path, method, id, tag) in [
        ("/v1/users", "get", "findUsers", "Users"),
        (
            "/v1/conversations",
            "get",
            "listConversations",
            "Conversations",
        ),
        (
            "/v1/conversations",
            "post",
            "createConversation",
            "Conversations",
        ),
        (
            "/v1/conversations/{conversation_id}",
            "get",
            "getConversation",
            "Conversations",
        ),
        (
            "/v1/conversations/{conversation_id}",
            "patch",
            "updateConversation",
            "Conversations",
        ),
        (
            "/v1/conversations/{conversation_id}/members",
            "get",
            "listConversationMembers",
            "Conversations",
        ),
        (
            "/v1/conversations/{conversation_id}/members/{user_id}",
            "put",
            "addConversationMember",
            "Conversations",
        ),
        (
            "/v1/conversations/{conversation_id}/members/{user_id}",
            "patch",
            "updateConversationMember",
            "Conversations",
        ),
        (
            "/v1/conversations/{conversation_id}/members/{user_id}",
            "delete",
            "removeConversationMember",
            "Conversations",
        ),
        (
            "/v1/conversations/{conversation_id}/owner",
            "put",
            "transferConversationOwnership",
            "Conversations",
        ),
        (
            "/v1/conversations/{conversation_id}/members/me",
            "delete",
            "leaveConversation",
            "Conversations",
        ),
        (
            "/v1/conversations/{conversation_id}/messages",
            "get",
            "listMessages",
            "Messages",
        ),
        (
            "/v1/conversations/{conversation_id}/messages",
            "post",
            "sendMessage",
            "Messages",
        ),
        (
            "/v1/conversations/{conversation_id}/read-state",
            "put",
            "updateConversationReadState",
            "Messages",
        ),
        ("/v1/messages/{message_id}", "get", "getMessage", "Messages"),
        (
            "/v1/messages/{message_id}",
            "patch",
            "editMessage",
            "Messages",
        ),
        (
            "/v1/messages/{message_id}",
            "delete",
            "deleteMessage",
            "Messages",
        ),
    ] {
        let operation = &doc["paths"][path][method];
        assert_eq!(operation["operationId"], id);
        assert!(operation_ids.insert(id));
        assert_eq!(operation["tags"], json!([tag]));
        assert_eq!(operation["security"], json!([{"bearerAuth":[]}]));
        for code in ["401", "429", "500"] {
            assert!(operation["responses"][code].is_object(), "{id}: {code}");
        }
        for (code, response) in operation["responses"].as_object().unwrap() {
            if code.starts_with('4') || code.starts_with('5') {
                assert_eq!(
                    response["content"]["application/problem+json"]["schema"]["$ref"],
                    "#/components/schemas/ProblemDetails",
                    "{id}: {code}"
                );
            }
        }
        let params = operation["parameters"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let has_key = params.iter().any(|p| p["name"] == "Idempotency-Key");
        assert_eq!(has_key, id == "createConversation" || id == "sendMessage");
        if has_key {
            assert!(operation["responses"]["200"]["headers"]["Idempotency-Replayed"].is_object());
            assert!(operation["responses"]["201"].is_object());
        }
        for parameter in params.iter().filter(|p| p["in"] == "path") {
            assert_eq!(parameter["schema"]["format"], "uuid");
        }
    }
    let schemas = &doc["components"]["schemas"];
    let create = &schemas["CreateConversationRequest"];
    assert_eq!(create["oneOf"].as_array().unwrap().len(), 2);
    assert_eq!(create["discriminator"]["propertyName"], "kind");
    assert_eq!(
        create["discriminator"]["mapping"]["direct"],
        "#/components/schemas/CreateDirectRequest"
    );
    assert_eq!(
        create["discriminator"]["mapping"]["group"],
        "#/components/schemas/CreateGroupRequest"
    );
    assert_eq!(
        schemas["CreateDirectRequest"]["additionalProperties"],
        false
    );
    assert_eq!(schemas["CreateGroupRequest"]["additionalProperties"], false);
    assert_eq!(
        schemas["CreateDirectRequest"]["properties"]["participant_id"]["format"],
        "uuid"
    );
    assert_eq!(
        schemas["CreateGroupRequest"]["properties"]["participant_ids"]["items"]["type"],
        "string"
    );
    assert_eq!(
        schemas["CreateGroupRequest"]["properties"]["participant_ids"]["items"]["format"],
        "uuid"
    );
    for (path, schema) in [
        ("/v1/users", "UserSummaryResponse"),
        ("/v1/conversations", "ConversationResponse"),
        (
            "/v1/conversations/{conversation_id}/members",
            "MemberResponse",
        ),
        (
            "/v1/conversations/{conversation_id}/messages",
            "MessageResponse",
        ),
    ] {
        let operation = &doc["paths"][path]["get"];
        let reference =
            operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .unwrap();
        let wrapper = &schemas[reference.rsplit('/').next().unwrap()];
        for field in ["items", "next_cursor", "has_more"] {
            assert!(
                wrapper["properties"][field].is_object(),
                "{reference} {field}"
            );
        }
        let item = &wrapper["properties"]["items"]["items"];
        if let Some(reference) = item["$ref"].as_str() {
            assert_eq!(reference, format!("#/components/schemas/{schema}"));
        } else {
            assert_eq!(item, &schemas[schema], "{schema}: {item}");
        }
        let params = operation["parameters"].as_array().unwrap();
        let limit = params.iter().find(|p| p["name"] == "limit").unwrap();
        assert_eq!(limit["schema"]["default"], 50);
        assert_eq!(limit["schema"]["minimum"], 1);
        assert_eq!(limit["schema"]["maximum"], 100);
        assert!(params.iter().any(|p| p["name"] == "cursor"));
    }
    for (name, fields) in [
        (
            "MessageResponse",
            vec!["id", "conversation_id", "sender_id"],
        ),
        ("ConversationResponse", vec!["id"]),
        ("UserSummaryResponse", vec!["id"]),
    ] {
        for field in fields {
            assert_eq!(schemas[name]["properties"][field]["format"], "uuid");
        }
    }
    assert!(
        schemas["ReplyPreviewResponse"]["properties"]
            .get("reply_to")
            .is_none()
    );
    assert!(
        schemas["MemberResponse"]["properties"]
            .get("last_read_message_id")
            .is_none()
    );
}

#[tokio::test]
async fn cors_allows_messaging_methods_and_exposes_replay_header() {
    let mut raw = config();
    raw.cors.allowed_origins = vec!["https://client.example".into()];
    let f = Fixture::new(raw, windwatcher::RegistrationPolicy::Open, true).await;
    for method in ["PUT", "PATCH", "DELETE"] {
        let request = Request::builder()
            .method("OPTIONS")
            .uri("/v1/conversations")
            .header("origin", "https://client.example")
            .header("access-control-request-method", method)
            .header(
                "access-control-request-headers",
                "authorization,content-type,idempotency-key",
            )
            .body(Body::empty())
            .unwrap();
        let response = send(&f.router, request).await;
        assert_eq!(response.0, StatusCode::OK);
        assert!(
            response.1["access-control-allow-methods"]
                .to_str()
                .unwrap()
                .contains(method)
        );
        assert!(
            response.1["access-control-allow-headers"]
                .to_str()
                .unwrap()
                .contains("idempotency-key")
        );
        assert!(!response.1.contains_key("access-control-allow-credentials"));
    }
    let (_, token) = participant(&f, "cors_owner").await;
    let mut request = bearer("GET", "/v1/conversations", &token);
    request
        .headers_mut()
        .insert("origin", HeaderValue::from_static("https://client.example"));
    let response = send(&f.router, request).await;
    assert!(
        response.1["access-control-expose-headers"]
            .to_str()
            .unwrap()
            .contains("idempotency-replayed")
    );
}

#[tokio::test]
async fn client_journey_discovery_inbox_reply_tombstone_and_monotonic_read_state() {
    let f = Fixture::open().await;
    f.user().await;
    let grant = f.grant().await;
    let token = grant["access_token"].as_str().unwrap();
    let (peer, peer_token) = participant(&f, "bob").await;
    let (_, outsider) = participant(&f, "outsider").await;
    let discovery = send(&f.router, bearer("GET", "/v1/users?username=BOB", token)).await;
    assert_eq!(discovery.0, StatusCode::OK);
    page(&discovery.2);
    assert_eq!(
        discovery.2["items"],
        json!([{"id":peer.id().to_string(), "username":"bob"}])
    );
    let empty = send(&f.router, bearer("GET", "/v1/conversations", token)).await;
    assert_eq!(empty.2["items"], json!([]));
    let direct_request = json!({"kind":"direct", "participant_id":peer.id().to_string()});
    let created = send(
        &f.router,
        authorized("POST", "/v1/conversations", token, direct_request.clone()),
    )
    .await;
    assert_eq!(created.0, StatusCode::CREATED, "{}", created.2);
    assert_eq!(created.2["title"], "bob");
    assert_eq!(created.2["membership"]["role"], "direct");
    let id = created.2["id"].as_str().unwrap();
    assert_eq!(
        send(
            &f.router,
            authorized("POST", "/v1/conversations", token, direct_request)
        )
        .await
        .0,
        StatusCode::OK
    );
    let base = format!("/v1/conversations/{id}");
    let first = send(
        &f.router,
        keyed(
            authorized(
                "POST",
                &format!("{base}/messages"),
                &peer_token,
                json!({"content":"hello"}),
            ),
            "send-once",
        ),
    )
    .await;
    assert_eq!(first.0, StatusCode::CREATED, "{}", first.2);
    assert!(!first.1.contains_key("idempotency-replayed"));
    assert_eq!(first.2["sender_username"], "bob");
    let message_id = first.2["id"].as_str().unwrap();
    let replay = send(
        &f.router,
        keyed(
            authorized(
                "POST",
                &format!("{base}/messages"),
                &peer_token,
                json!({"content":"hello"}),
            ),
            "send-once",
        ),
    )
    .await;
    assert_eq!(replay.0, StatusCode::OK);
    assert_eq!(replay.1["idempotency-replayed"], "true");
    assert_eq!(replay.2["id"], message_id);
    problem(
        &send(
            &f.router,
            keyed(
                authorized(
                    "POST",
                    &format!("{base}/messages"),
                    &peer_token,
                    json!({"content":"changed"}),
                ),
                "send-once",
            ),
        )
        .await,
        409,
        "idempotency.key_reused",
    );
    let reply = send(
        &f.router,
        authorized(
            "POST",
            &format!("{base}/messages"),
            token,
            json!({"content":"reply", "reply_to_message_id":message_id}),
        ),
    )
    .await;
    assert_eq!(reply.0, StatusCode::CREATED);
    assert_eq!(reply.2["reply_to"]["sender_username"], "bob");
    assert!(reply.2["reply_to"].get("reply_to").is_none());
    let inbox = send(&f.router, bearer("GET", "/v1/conversations", token)).await;
    page(&inbox.2);
    assert_eq!(inbox.2["items"][0]["unread_count"], 1);
    assert_eq!(inbox.2["items"][0]["last_message"]["id"], reply.2["id"]);
    let history = send(
        &f.router,
        bearer("GET", &format!("{base}/messages?limit=1"), token),
    )
    .await;
    page(&history.2);
    assert_eq!(history.2["has_more"], true);
    let next = history.2["next_cursor"].as_str().unwrap();
    let older = send(
        &f.router,
        bearer("GET", &format!("{base}/messages?cursor={next}"), token),
    )
    .await;
    assert_eq!(older.2["items"][0]["id"], message_id);
    problem(
        &send(
            &f.router,
            bearer("GET", &format!("/v1/messages/{message_id}"), &outsider),
        )
        .await,
        404,
        "message.not_found",
    );
    problem(
        &send(
            &f.router,
            bearer(
                "GET",
                &format!("/v1/messages/{}", windwatcher::MessageId::new()),
                &outsider,
            ),
        )
        .await,
        404,
        "message.not_found",
    );
    problem(
        &send(&f.router, bearer("GET", &base, &outsider)).await,
        404,
        "conversation.not_found",
    );
    problem(
        &send(
            &f.router,
            authorized("PATCH", &base, &outsider, json!({"title":"leak"})),
        )
        .await,
        404,
        "conversation.not_found",
    );
    problem(
        &send(
            &f.router,
            authorized(
                "PATCH",
                &format!("/v1/messages/{message_id}"),
                token,
                json!({"content":"not mine"}),
            ),
        )
        .await,
        403,
        "message.not_author",
    );
    problem(
        &send(
            &f.router,
            bearer("DELETE", &format!("/v1/messages/{message_id}"), token),
        )
        .await,
        403,
        "message.insufficient_permissions",
    );
    let edited = send(
        &f.router,
        authorized(
            "PATCH",
            &format!("/v1/messages/{message_id}"),
            &peer_token,
            json!({"content":"edited"}),
        ),
    )
    .await;
    assert_eq!(edited.0, StatusCode::OK);
    assert!(edited.2["edited_at"].is_string());
    let noop = send(
        &f.router,
        authorized(
            "PATCH",
            &format!("/v1/messages/{message_id}"),
            &peer_token,
            json!({"content":"edited"}),
        ),
    )
    .await;
    assert_eq!(noop.2, edited.2);
    let read = send(
        &f.router,
        authorized(
            "PUT",
            &format!("{base}/read-state"),
            token,
            json!({"last_read_message_id":reply.2["id"]}),
        ),
    )
    .await;
    assert_eq!(read.0, StatusCode::OK);
    assert_eq!(read.2["unread_count"], 0);
    let stale = send(
        &f.router,
        authorized(
            "PUT",
            &format!("{base}/read-state"),
            token,
            json!({"last_read_message_id":message_id}),
        ),
    )
    .await;
    assert_eq!(stale.2, read.2);
    let deleted = send(
        &f.router,
        bearer("DELETE", &format!("/v1/messages/{message_id}"), &peer_token),
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    assert!(deleted.2["content"].is_null());
    assert!(deleted.2["deleted_at"].is_string());
    assert_eq!(
        send(
            &f.router,
            bearer("DELETE", &format!("/v1/messages/{message_id}"), &peer_token)
        )
        .await
        .2,
        deleted.2
    );
    let loaded_reply = send(
        &f.router,
        bearer(
            "GET",
            &format!("/v1/messages/{}", reply.2["id"].as_str().unwrap()),
            token,
        ),
    )
    .await;
    assert!(loaded_reply.2["reply_to"]["content"].is_null());
    assert!(loaded_reply.2["reply_to"]["deleted_at"].is_string());
    problem(
        &send(
            &f.router,
            authorized(
                "PATCH",
                &format!("/v1/messages/{message_id}"),
                &peer_token,
                json!({"content":"restore"}),
            ),
        )
        .await,
        409,
        "message.deleted",
    );
}

#[tokio::test]
async fn group_lifecycle_and_disabled_participants_preserve_contracts() {
    let f = Fixture::open().await;
    let (owner, token) = participant(&f, "owner").await;
    let (mut peer, peer_token) = participant(&f, "member").await;
    let (target, target_token) = participant(&f, "target").await;
    let request = json!({"kind":"group","title":"Engineering","participant_ids":[peer.id().to_string(),peer.id().to_string(),owner.id().to_string()]});
    let group = send(
        &f.router,
        keyed(
            authorized("POST", "/v1/conversations", &token, request),
            "group-key",
        ),
    )
    .await;
    assert_eq!(group.0, StatusCode::CREATED, "{}", group.2);
    let replay = send(&f.router, keyed(authorized("POST", "/v1/conversations", &token, json!({"kind":"group","title":"Engineering","participant_ids":[peer.id().to_string()]})), "group-key")).await;
    assert_eq!(replay.0, StatusCode::OK);
    assert_eq!(replay.2["id"], group.2["id"]);
    assert_eq!(replay.1["idempotency-replayed"], "true");
    let base = format!("/v1/conversations/{}", group.2["id"].as_str().unwrap());
    let members = send(&f.router, bearer("GET", &format!("{base}/members"), &token)).await;
    let member_page = send(
        &f.router,
        bearer("GET", &format!("{base}/members?limit=1"), &token),
    )
    .await;
    let old_cursor = member_page.2["next_cursor"].as_str().unwrap().to_owned();
    let renamed = send(
        &f.router,
        authorized("PATCH", &base, &token, json!({"title":"Renamed"})),
    )
    .await;
    assert_eq!(renamed.0, StatusCode::OK);
    assert_eq!(
        send(
            &f.router,
            authorized("PATCH", &base, &token, json!({"title":"Renamed"}))
        )
        .await
        .2,
        renamed.2
    );
    problem(
        &send(
            &f.router,
            authorized("PATCH", &base, &token, json!({"title":"  "})),
        )
        .await,
        422,
        "conversation.invalid_title",
    );
    problem(
        &send(
            &f.router,
            authorized("PATCH", &base, &peer_token, json!({"title":"Denied"})),
        )
        .await,
        403,
        "conversation.insufficient_permissions",
    );
    assert_eq!(
        send(
            &f.router,
            authorized(
                "PUT",
                &format!("{base}/owner"),
                &token,
                json!({"user_id":owner.id().to_string()})
            )
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(members.0, StatusCode::OK);
    page(&members.2);
    for member in members.2["items"].as_array().unwrap() {
        assert!(member.get("last_read_message_id").is_none());
        assert!(member.get("unread_count").is_none());
    }
    let target_path = format!("{base}/members/{}", target.id());
    problem(
        &send(&f.router, bearer("PUT", &target_path, &peer_token)).await,
        403,
        "conversation.insufficient_permissions",
    );
    assert_eq!(
        send(&f.router, bearer("PUT", &target_path, &token)).await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        send(&f.router, bearer("PUT", &target_path, &token)).await.0,
        StatusCode::OK
    );
    let role = send(
        &f.router,
        authorized("PATCH", &target_path, &token, json!({"role":"admin"})),
    )
    .await;
    assert_eq!(role.2["role"], "admin");
    problem(
        &send(
            &f.router,
            bearer(
                "DELETE",
                &format!("{base}/members/{}", owner.id()),
                &target_token,
            ),
        )
        .await,
        409,
        "conversation.owner_protected",
    );
    problem(
        &send(
            &f.router,
            authorized(
                "PATCH",
                &format!("{base}/members/{}", owner.id()),
                &token,
                json!({"role":"member"}),
            ),
        )
        .await,
        409,
        "conversation.owner_protected",
    );
    problem(
        &send(
            &f.router,
            authorized(
                "PUT",
                &format!("{base}/owner"),
                &target_token,
                json!({"user_id":target.id().to_string()}),
            ),
        )
        .await,
        403,
        "conversation.insufficient_permissions",
    );
    problem(
        &send(
            &f.router,
            authorized("PATCH", &target_path, &token, json!({"role":"owner"})),
        )
        .await,
        400,
        "request.invalid_json",
    );
    let missing_member = format!("{base}/members/{}", UserId::new());
    problem(
        &send(
            &f.router,
            authorized("PATCH", &missing_member, &token, json!({"role":"admin"})),
        )
        .await,
        404,
        "conversation.member_not_found",
    );
    problem(
        &send(&f.router, bearer("DELETE", &missing_member, &token)).await,
        404,
        "conversation.member_not_found",
    );
    problem(
        &send(
            &f.router,
            authorized(
                "PUT",
                &format!("{base}/owner"),
                &token,
                json!({"user_id":UserId::new().to_string()}),
            ),
        )
        .await,
        404,
        "conversation.member_not_found",
    );
    let original = send(
        &f.router,
        authorized(
            "POST",
            &format!("{base}/messages"),
            &peer_token,
            json!({"content":"moderate"}),
        ),
    )
    .await;
    let moderation = send(
        &f.router,
        bearer(
            "DELETE",
            &format!("/v1/messages/{}", original.2["id"].as_str().unwrap()),
            &target_token,
        ),
    )
    .await;
    assert_eq!(moderation.0, StatusCode::OK);
    assert!(moderation.2["deleted_at"].is_string());
    assert_eq!(
        send(
            &f.router,
            authorized("PATCH", &target_path, &token, json!({"role":"admin"}))
        )
        .await
        .2,
        role.2
    );
    problem(
        &send(
            &f.router,
            bearer("DELETE", &format!("{base}/members/{}", owner.id()), &token),
        )
        .await,
        403,
        "conversation.insufficient_permissions",
    );
    problem(
        &send(
            &f.router,
            bearer(
                "DELETE",
                &format!("{base}/members/{}", owner.id()),
                &peer_token,
            ),
        )
        .await,
        403,
        "conversation.insufficient_permissions",
    );
    assert_eq!(
        send(&f.router, bearer("DELETE", &target_path, &token))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    problem(
        &send(
            &f.router,
            bearer("DELETE", &format!("{base}/members/me"), &token),
        )
        .await,
        409,
        "conversation.ownership_transfer_required",
    );
    peer.disable(time::OffsetDateTime::now_utc());
    f.storage.update_user(&peer).await.unwrap();
    let discovery = send(
        &f.router,
        bearer("GET", "/v1/users?username=member", &token),
    )
    .await;
    assert_eq!(discovery.2["items"], json!([]));
    assert_eq!(
        send(
            &f.router,
            bearer("PUT", &format!("{base}/members/{}", peer.id()), &token)
        )
        .await
        .0,
        StatusCode::OK
    );
    problem(
        &send(
            &f.router,
            authorized(
                "POST",
                "/v1/conversations",
                &token,
                json!({"kind":"direct","participant_id":peer.id().to_string()}),
            ),
        )
        .await,
        422,
        "conversation.participant_unavailable",
    );
    let transferred = send(
        &f.router,
        authorized(
            "PUT",
            &format!("{base}/owner"),
            &token,
            json!({"user_id":peer.id().to_string()}),
        ),
    )
    .await;
    assert_eq!(transferred.0, StatusCode::OK);
    assert_eq!(transferred.2["membership"]["role"], "admin");
    assert_eq!(
        send(
            &f.router,
            bearer("DELETE", &format!("{base}/members/me"), &token)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    problem(
        &send(&f.router, bearer("GET", &base, &token)).await,
        404,
        "conversation.not_found",
    );
    problem(
        &send(
            &f.router,
            bearer(
                "GET",
                &format!("{base}/members?cursor={old_cursor}"),
                &token,
            ),
        )
        .await,
        404,
        "conversation.not_found",
    );
    assert_eq!(
        send(
            &f.router,
            bearer("DELETE", &format!("{base}/members/me"), &peer_token)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    problem(&send(&f.router, keyed(authorized("POST", "/v1/conversations", &token, json!({"kind":"group","title":"Engineering","participant_ids":[peer.id().to_string()]})), "group-key")).await, 409, "idempotency.result_unavailable");
}

#[tokio::test]
async fn extraction_pagination_keys_and_auth_are_normalized() {
    let f = Fixture::open().await;
    let (actor, token) = participant(&f, "extract_actor").await;
    for (query, status, code) in [
        ("", 400, "request.invalid_query"),
        ("?username=!", 422, "user.invalid_username"),
        ("?username=missing&limit=0", 422, "pagination.invalid_limit"),
        ("?username=missing&limit=abc", 400, "request.invalid_query"),
        (
            "?username=missing&cursor=bad",
            400,
            "pagination.invalid_cursor",
        ),
    ] {
        problem(
            &send(
                &f.router,
                bearer("GET", &format!("/v1/users{query}"), &token),
            )
            .await,
            status,
            code,
        );
    }
    for (query, code, status) in [
        ("?limit=0", "pagination.invalid_limit", 422),
        ("?limit=101", "pagination.invalid_limit", 422),
        ("?limit=-1", "pagination.invalid_limit", 422),
        ("?limit=oops", "request.invalid_query", 400),
        ("?cursor=bad", "pagination.invalid_cursor", 400),
        ("?sort=title", "request.invalid_query", 400),
    ] {
        problem(
            &send(
                &f.router,
                bearer("GET", &format!("/v1/conversations{query}"), &token),
            )
            .await,
            status,
            code,
        );
    }
    problem(
        &send(
            &f.router,
            bearer("GET", "/v1/conversations/not-uuid", &token),
        )
        .await,
        400,
        "request.invalid_path",
    );
    problem(
        &send(
            &f.router,
            authorized(
                "POST",
                "/v1/conversations",
                &token,
                json!({"kind":"direct","participant_id":"not-a-uuid"}),
            ),
        )
        .await,
        400,
        "request.invalid_identifier",
    );
    problem(
        &send(
            &f.router,
            authorized(
                "POST",
                "/v1/conversations",
                &token,
                json!({"kind":"group","title":"valid title","participant_ids":["not-a-uuid"]}),
            ),
        )
        .await,
        400,
        "request.invalid_identifier",
    );
    let body = json!({"kind":"direct","participant_id":actor.id().to_string()});
    for key in ["", "bad key", &"a".repeat(129)] {
        problem(
            &send(
                &f.router,
                keyed(
                    authorized("POST", "/v1/conversations", &token, body.clone()),
                    key,
                ),
            )
            .await,
            400,
            "idempotency.invalid_key",
        );
    }
    let mut duplicate = keyed(
        authorized("POST", "/v1/conversations", &token, body.clone()),
        "one",
    );
    duplicate
        .headers_mut()
        .append("idempotency-key", HeaderValue::from_static("two"));
    problem(
        &send(&f.router, duplicate).await,
        400,
        "idempotency.invalid_key",
    );
    problem(&send(&f.router, authorized("POST", "/v1/conversations", &token, json!({"kind":"direct","participant_id":actor.id().to_string(),"title":"ambiguous"}))).await, 400, "request.invalid_json");
    let self_direct = send(
        &f.router,
        authorized("POST", "/v1/conversations", &token, body),
    )
    .await;
    assert_eq!(self_direct.2["title"], "extract_actor");
    let base = format!(
        "/v1/conversations/{}",
        self_direct.2["id"].as_str().unwrap()
    );
    for request in [
        authorized(
            "PUT",
            &format!("{base}/owner"),
            &token,
            json!({"user_id":"not-a-uuid"}),
        ),
        authorized(
            "POST",
            &format!("{base}/messages"),
            &token,
            json!({"content":"reply", "reply_to_message_id":"not-a-uuid"}),
        ),
        authorized(
            "PUT",
            &format!("{base}/read-state"),
            &token,
            json!({"last_read_message_id":"not-a-uuid"}),
        ),
    ] {
        problem(
            &send(&f.router, request).await,
            400,
            "request.invalid_identifier",
        );
    }
    problem(
        &send(
            &f.router,
            authorized("PATCH", &base, &token, json!({"title":"unsupported"})),
        )
        .await,
        409,
        "conversation.operation_not_supported",
    );
    problem(
        &send(
            &f.router,
            authorized(
                "POST",
                &format!("{base}/messages"),
                &token,
                json!({"content":"\u{0}"}),
            ),
        )
        .await,
        422,
        "message.invalid_content",
    );
    problem(
        &send(
            &f.router,
            authorized(
                "POST",
                &format!("{base}/messages"),
                &token,
                json!({"content":"a".repeat(131073)}),
            ),
        )
        .await,
        422,
        "message.content_too_large",
    );
    let id = windwatcher::ConversationId::new();
    let msg = windwatcher::MessageId::new();
    for (method, path) in [
        ("GET", "/v1/users?username=alice".into()),
        ("GET", "/v1/conversations".into()),
        ("POST", "/v1/conversations".into()),
        ("GET", format!("/v1/conversations/{id}")),
        ("PATCH", format!("/v1/conversations/{id}")),
        ("GET", format!("/v1/conversations/{id}/members")),
        (
            "PUT",
            format!("/v1/conversations/{id}/members/{}", actor.id()),
        ),
        (
            "PATCH",
            format!("/v1/conversations/{id}/members/{}", actor.id()),
        ),
        (
            "DELETE",
            format!("/v1/conversations/{id}/members/{}", actor.id()),
        ),
        ("PUT", format!("/v1/conversations/{id}/owner")),
        ("DELETE", format!("/v1/conversations/{id}/members/me")),
        ("GET", format!("/v1/conversations/{id}/messages")),
        ("POST", format!("/v1/conversations/{id}/messages")),
        ("PUT", format!("/v1/conversations/{id}/read-state")),
        ("GET", format!("/v1/messages/{msg}")),
        ("PATCH", format!("/v1/messages/{msg}")),
        ("DELETE", format!("/v1/messages/{msg}")),
    ] {
        problem(
            &send(&f.router, command(method, &path, json!({}))).await,
            401,
            "auth.invalid_access_token",
        );
    }
}
