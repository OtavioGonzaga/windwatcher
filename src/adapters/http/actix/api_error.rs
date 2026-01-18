use actix_web::{HttpResponse, ResponseError};
use serde::Serialize;
use std::fmt::{Display, Formatter};

#[derive(Debug, Serialize)]
pub struct ApiError {
    message: String,
    #[serde(skip)]
    status: actix_web::http::StatusCode,
}

impl ApiError {
    pub fn new(status: actix_web::http::StatusCode, msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            status,
        }
    }

    pub fn internal_server_error() -> Self {
        Self {
            message: "Internal server error".into(),
            status: actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl ResponseError for ApiError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        self.status
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status).json(serde_json::json!({ "message": self.message }))
    }
}

impl Display for ApiError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
