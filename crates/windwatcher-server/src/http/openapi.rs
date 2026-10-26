use utoipa::openapi::{
    InfoBuilder, OpenApi, RefOr, Tag,
    header::HeaderBuilder,
    schema::{KnownFormat, ObjectBuilder, SchemaFormat, Type},
    security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};

pub(super) fn finish(mut spec: OpenApi) -> OpenApi {
    spec.info = InfoBuilder::new().title("Windwatcher API").version(env!("CARGO_PKG_VERSION")).description(Some("Windwatcher HTTP API v1. Tokens are JSON bearer secrets; this protocol is not OAuth or OIDC.")).build();
    spec.tags = Some(vec![Tag::new("Users"), Tag::new("Sessions")]);
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
            for (status, response) in &mut operation.responses.responses {
                // Current route annotations produce inline response objects;
                // the semantic contract tests assert headers on every response.
                if let RefOr::T(response) = response {
                    response
                        .headers
                        .insert("X-Request-Id".into(), request_id.clone().into());
                    if status == "401" {
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
