use crate::adapters::hash::argon2::Argon2Hasher;
use crate::adapters::http::actix::user::dto::{CreateUserHttpDto, CreateUserResponseDto};
use crate::adapters::persistence::postgres::user::repository::PostgresUserRepository;
use crate::domain::user::{
    error::UserError,
    service::{CreateUserInput, UserService},
};
use actix_web::{HttpResponse, web};
use serde_json::json;
use uuid::Uuid;

pub type UserAppService = UserService<PostgresUserRepository, Argon2Hasher>;

#[utoipa::path(
    get,
    path = "/users/{id}",
    params(
        ("id" = String, Path, description = "User UUID")
    ),
    tag = "Users",
    responses(
        (status = 200, description = "User retrieved successfully"),
        (status = 400, description = "Invalid data provided"),
        (status = 404, description = "User not found")
    )
)]
pub async fn find_by_id(
    service: web::Data<UserAppService>,
    params: web::Path<String>,
) -> HttpResponse {
    let id: Result<Uuid, uuid::Error> = Uuid::parse_str(&params);

    if id.is_err() {
        return HttpResponse::BadRequest().json(json!({"message": "Invalid UUID format"}));
    }

    match service.find_by_id(id.unwrap()).await {
        Ok(user) => HttpResponse::Ok().json(CreateUserResponseDto::from_domain(user)),
        Err(UserError::NotFound) => {
            HttpResponse::NotFound().json(json!({"message": "User not found"}))
        }
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

#[utoipa::path(
    post,
    path = "/users",
    request_body = CreateUserHttpDto,
    tag = "Users",
    responses(
        (status = 201, description = "User created successfully", body = CreateUserResponseDto),
        (status = 409, description = "Username already exists"),
        (status = 400, description = "Invalid data provided")
    )
)]
pub async fn create_user(
    service: web::Data<UserAppService>,
    payload: web::Json<CreateUserHttpDto>,
) -> HttpResponse {
    let cmd: CreateUserInput = CreateUserInput {
        username: payload.username.clone(),
        name: payload.name.clone(),
        password: payload.password.clone(),
    };

    match service.create_user(cmd).await {
        Ok(user) => HttpResponse::Created().json(CreateUserResponseDto::from_domain(user)),
        Err(UserError::AlreadyExists) => HttpResponse::Conflict()
            .json(json!({"message": "Already exists a user with this username"})),
        Err(UserError::InvalidUsername(message)) | Err(UserError::InvalidPassword(message)) => {
            HttpResponse::BadRequest().json(json!({"message": message}))
        }
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}
