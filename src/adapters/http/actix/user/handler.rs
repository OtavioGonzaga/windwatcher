use super::dto::{CreateUserDto, UpdateUserDto, UserResponseDto};
use crate::adapters::{
    hash::argon2::Argon2Hasher, persistence::postgres::user::repository::PostgresUserRepository,
};
use crate::application::user::{
    create_user::{CreateUserError, CreateUserInput, CreateUserService},
    delete_user::{DeleteUserError, DeleteUserService},
    find_user::{FindUserError, FindUserService},
    update_user::{UpdateUserError, UpdateUserInput, UpdateUserService},
};
use crate::domain::user::error::UserError;
use actix_web::{HttpResponse, web};
use serde_json::json;
use uuid::Uuid;

#[utoipa::path(
    get,
    path = "/{id}",
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
    service: web::Data<FindUserService<PostgresUserRepository>>,
    params: web::Path<String>,
) -> HttpResponse {
    let id: Result<Uuid, uuid::Error> = Uuid::parse_str(&params);

    if id.is_err() {
        return HttpResponse::BadRequest().json(json!({"message": "Invalid UUID format"}));
    }

    match service.find_by_id(id.unwrap()).await {
        Ok(user) => HttpResponse::Ok().json(UserResponseDto {
            id: user.id.to_string(),
            username: user.username.as_str().into(),
            name: user.name.as_str().into(),
        }),
        Err(FindUserError::NotFound) => {
            HttpResponse::NotFound().json(json!({"message": "User not found"}))
        }
        Err(FindUserError::RepositoryError) => HttpResponse::InternalServerError().finish(),
    }
}

#[utoipa::path(
    post,
    path = "",
    request_body = CreateUserDto,
    tag = "Users",
    responses(
        (status = 201, description = "User created successfully", body = UserResponseDto),
        (status = 409, description = "Username already exists"),
        (status = 400, description = "Invalid data provided")
    )
)]
pub async fn create_user(
    service: web::Data<CreateUserService<PostgresUserRepository, Argon2Hasher>>,
    payload: web::Json<CreateUserDto>,
) -> HttpResponse {
    let cmd: CreateUserInput = CreateUserInput {
        username: payload.username.clone(),
        name: payload.name.clone(),
        password: payload.password.clone(),
    };

    match service.execute(cmd).await {
        Ok(user) => HttpResponse::Created().json(UserResponseDto {
            id: user.id.to_string(),
            username: user.username,
            name: user.name,
        }),
        Err(CreateUserError::AlreadyExists) => HttpResponse::Conflict()
            .json(json!({"message": "Already exists a user with this username"})),
        Err(CreateUserError::UserError(user_error)) => match user_error {
            UserError::InvalidUsername(message) => {
                HttpResponse::BadRequest().json(json!({"message": message}))
            }
            UserError::InvalidPassword(message) => {
                HttpResponse::BadRequest().json(json!({"message": message}))
            }
        },
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

#[utoipa::path(
    patch,
    path = "/{id}",
    params(
        ("id" = String, Path, description = "User UUID")
    ),
    tag = "Users",
    responses(
        (status = 200, description = "User updated successfully"),
        (status = 400, description = "Invalid data provided"),
        (status = 404, description = "User not found")
    )
)]
pub async fn update_user(
    service: web::Data<UpdateUserService<PostgresUserRepository, Argon2Hasher>>,
    params: web::Path<String>,
    payload: web::Json<UpdateUserDto>,
) -> HttpResponse {
    let id: Result<Uuid, uuid::Error> = Uuid::parse_str(&params);

    if id.is_err() {
        return HttpResponse::BadRequest().json(json!({"message": "Invalid UUID format"}));
    }

    let update_user: UpdateUserInput = UpdateUserInput {
        username: payload.username.clone(),
        name: payload.name.clone(),
        password: payload.password.clone(),
    };

    match service.execute(id.unwrap(), update_user).await {
        Ok(updated_user) => HttpResponse::Ok().json(UserResponseDto {
            id: updated_user.id.to_string(),
            username: updated_user.username,
            name: updated_user.name,
        }),
        Err(UpdateUserError::UserError(user_error)) => match user_error {
            UserError::InvalidUsername(message) => {
                HttpResponse::BadRequest().json(json!({"message": message}))
            }
            UserError::InvalidPassword(message) => {
                HttpResponse::BadRequest().json(json!({"message": message}))
            }
        },
        Err(UpdateUserError::AlreadyExists) => HttpResponse::Conflict()
            .json(json!({"message": "Already exists a user with this username"})),
        Err(UpdateUserError::NotFound) => {
            HttpResponse::NotFound().json(json!({"message": "User not found"}))
        }
        Err(UpdateUserError::InfrastructureError) => HttpResponse::InternalServerError().finish(),
    }
}

#[utoipa::path(
    delete,
    path = "/{id}",
    params(
        ("id" = String, Path, description = "User UUID")
    ),
    tag = "Users",
    responses(
        (status = 204, description = "User deleted successfully"),
        (status = 400, description = "Invalid data provided"),
        (status = 404, description = "User not found")
    )
)]
pub async fn delete_user(
    service: web::Data<DeleteUserService<PostgresUserRepository>>,
    params: web::Path<String>,
) -> HttpResponse {
    let id: Result<Uuid, uuid::Error> = Uuid::parse_str(&params);

    if id.is_err() {
        return HttpResponse::BadRequest().json(json!({"message": "Invalid UUID format"}));
    }

    match service.execute(id.unwrap()).await {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(DeleteUserError::NotFound) => {
            HttpResponse::NotFound().json(json!({"message": "User not found"}))
        }
        Err(DeleteUserError::InfrastructureError) => HttpResponse::InternalServerError().finish(),
    }
}
