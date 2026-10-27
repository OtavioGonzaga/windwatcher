use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub(super) struct TicketResponse {
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
