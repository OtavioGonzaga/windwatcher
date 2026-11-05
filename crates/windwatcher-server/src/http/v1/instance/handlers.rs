use axum::{Json, extract::State};
use windwatcher::RegistrationPolicy;

use super::dto::{ApiVersion, AuthMethod, InstanceResponse, Registration, Service};
use crate::http::{HttpState, problem::ProblemDetails};

#[utoipa::path(get, path = "/instance", operation_id = "getInstance", tag = "Instance",
    responses(
        (status = 200, description = "Public instance discovery metadata", body = InstanceResponse),
        (status = 429, description = "Client request quota exceeded", body = ProblemDetails, content_type = "application/problem+json")
    ))]
pub(super) async fn get_instance(State(state): State<HttpState>) -> Json<InstanceResponse> {
    Json(InstanceResponse {
        service: Service::Windwatcher,
        api_versions: vec![ApiVersion::V1],
        registration: if state.registration == RegistrationPolicy::Closed {
            Registration::Closed
        } else {
            Registration::Open
        },
        auth_methods: vec![AuthMethod::Password],
    })
}
