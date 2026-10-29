use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub(super) struct TicketResponse {
    /// Canonical unpadded base64url encoding of exactly 32 random bytes.
    #[schema(
        min_length = 43,
        max_length = 43,
        pattern = "^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$"
    )]
    pub ticket: String,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: OffsetDateTime,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TicketQuery {
    pub ticket: zeroize::Zeroizing<String>,
}
