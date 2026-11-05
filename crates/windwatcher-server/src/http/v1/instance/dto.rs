use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum Registration {
    Open,
    Closed,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum Service {
    Windwatcher,
}

#[derive(Serialize, ToSchema)]
pub(super) enum ApiVersion {
    #[serde(rename = "v1")]
    V1,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum AuthMethod {
    Password,
}

#[derive(Serialize, ToSchema)]
pub(super) struct InstanceResponse {
    /// Protocol identifier, not a cryptographically verified identity.
    pub service: Service,
    /// Supported public HTTP API versions.
    pub api_versions: Vec<ApiVersion>,
    /// Public self-registration policy; existing users can still authenticate when closed.
    pub registration: Registration,
    /// Supported client authentication methods.
    pub auth_methods: Vec<AuthMethod>,
}
