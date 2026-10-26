use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset};
use utoipa::ToSchema;
use windwatcher::SessionGrant;
use zeroize::Zeroizing;

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct CreateSessionRequest {
    #[schema(example = "alice")]
    pub username: String,
    #[schema(value_type = String, write_only, example = "fake example password")]
    pub password: Zeroizing<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct RefreshSessionRequest {
    #[schema(value_type = String, write_only, example = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")]
    pub refresh_token: Zeroizing<String>,
}

#[derive(Serialize, ToSchema)]
pub(in crate::http) struct SessionGrantResponse {
    #[schema(value_type = String, example = "fake.header.signature")]
    access_token: Zeroizing<String>,
    #[schema(value_type = String, example = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")]
    refresh_token: Zeroizing<String>,
    #[schema(example = "Bearer")]
    token_type: &'static str,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    access_token_expires_at: OffsetDateTime,
    #[schema(format = Uuid, example = "019a0000-0000-7000-8000-000000000001")]
    session_id: String,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    session_expires_at: OffsetDateTime,
}
impl From<SessionGrant> for SessionGrantResponse {
    fn from(grant: SessionGrant) -> Self {
        let access_token_expires_at = grant.access_token_expires_at().to_offset(UtcOffset::UTC);
        let (session, access, refresh) = grant.into_parts();
        Self {
            access_token: Zeroizing::new(access.expose_secret().to_owned()),
            refresh_token: Zeroizing::new(refresh.expose_secret().to_owned()),
            token_type: "Bearer",
            access_token_expires_at,
            session_id: session.id().to_string(),
            session_expires_at: session.expires_at().to_offset(UtcOffset::UTC),
        }
    }
}
