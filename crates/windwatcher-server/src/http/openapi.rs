use utoipa::openapi::{
    InfoBuilder, OpenApi, RefOr, Tag,
    header::HeaderBuilder,
    schema::{KnownFormat, ObjectBuilder, SchemaFormat, Type},
    security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme},
};

pub(super) fn finish(mut spec: OpenApi) -> OpenApi {
    #[derive(utoipa::OpenApi)]
    #[openapi(components(schemas(crate::realtime::event::RealtimeEvent)))]
    struct RealtimeSchemas;
    use utoipa::OpenApi as _;
    if let Some(components) = RealtimeSchemas::openapi().components {
        spec.components
            .get_or_insert_with(Default::default)
            .schemas
            .extend(components.schemas);
    }
    spec.info = InfoBuilder::new().title("Windwatcher API").version(env!("CARGO_PKG_VERSION")).description(Some("Windwatcher HTTP API v1. Tokens are JSON bearer secrets; this protocol is not OAuth or OIDC.")).build();
    spec.tags = Some(vec![
        Tag::new("Users"),
        Tag::new("Sessions"),
        Tag::new("Conversations"),
        Tag::new("Messages"),
        Tag::new("Realtime"),
    ]);
    spec.components
        .get_or_insert_with(Default::default)
        .add_security_scheme(
            "bearerAuth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    spec.components
        .as_mut()
        .expect("components")
        .add_security_scheme(
            "realtimeTicket",
            SecurityScheme::ApiKey(ApiKey::Query(ApiKeyValue::new("ticket"))),
        );
    document_response_headers(&mut spec);
    spec
}

/// Mirror middleware-wide guarantees across the generated operations, without
/// a second registry of routes or operation IDs. Grant-specific cache policy
/// remains declared on the responses that actually return token grants.
fn document_response_headers(spec: &mut OpenApi) {
    let request_id = HeaderBuilder::new()
        .description(Some("Always present: server-generated UUIDv7 correlation ID, replacing any client-provided value. Matches ProblemDetails.request_id on errors."))
        .schema(Some(ObjectBuilder::new().schema_type(Type::String).format(Some(SchemaFormat::KnownFormat(KnownFormat::Uuid)))))
        .build();
    let challenge = HeaderBuilder::new()
        .description(Some(
            "Bearer authentication challenge, without token values or credential failure details.",
        ))
        .schema(Some(
            ObjectBuilder::new()
                .schema_type(Type::String)
                .enum_values(Some(["Bearer"])),
        ))
        .example(Some(serde_json::json!("Bearer")))
        .build();
    let retry_after = HeaderBuilder::new()
        .description(Some(
            "Seconds to wait before retrying; rounded upward to a positive integer.",
        ))
        .schema(Some(
            ObjectBuilder::new()
                .schema_type(Type::Integer)
                .minimum(Some(1)),
        ))
        .example(Some(serde_json::json!(1)))
        .build();
    for path in spec.paths.paths.values_mut() {
        for operation in [
            &mut path.get,
            &mut path.put,
            &mut path.post,
            &mut path.delete,
            &mut path.options,
            &mut path.head,
            &mut path.patch,
            &mut path.trace,
            &mut path.query,
        ]
        .into_iter()
        .flatten()
        .chain(path.additional_operations.values_mut())
        {
            let bearer_auth = operation.security.as_ref().is_some_and(|requirements| {
                requirements.iter().any(|requirement| {
                    serde_json::to_value(requirement)
                        .ok()
                        .is_some_and(|value| value.get("bearerAuth").is_some())
                })
            });
            if operation.request_body.is_some()
                && let Some(problem) = operation.responses.responses.get("400").cloned()
            {
                for (status, description) in [
                    ("413", "Body exceeds the transport limit"),
                    ("415", "JSON Content-Type required"),
                ] {
                    let mut response = problem.clone();
                    if let RefOr::T(ref mut response) = response {
                        response.description = description.into();
                    }
                    operation
                        .responses
                        .responses
                        .entry(status.into())
                        .or_insert(response);
                }
            }
            for (status, response) in &mut operation.responses.responses {
                // Current route annotations produce inline response objects;
                // the semantic contract tests assert headers on every response.
                if let RefOr::T(response) = response {
                    if (status.starts_with('4') || status.starts_with('5'))
                        && let Some(content) = response.content.shift_remove("application/json")
                    {
                        response
                            .content
                            .insert("application/problem+json".into(), content);
                    }
                    response
                        .headers
                        .insert("X-Request-Id".into(), request_id.clone().into());
                    if status == "401" && bearer_auth {
                        response
                            .headers
                            .insert("WWW-Authenticate".into(), challenge.clone().into());
                    }
                    if status == "429" {
                        response
                            .headers
                            .insert("Retry-After".into(), retry_after.clone().into());
                    }
                }
            }
        }
    }
}
