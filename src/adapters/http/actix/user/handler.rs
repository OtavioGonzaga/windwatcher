use super::dto::{CreateUserDto, UpdateUserDto, UserResponseDto};
use crate::{
    adapters::{
        hash::argon2::Argon2Hasher, http::actix::api_error::ApiError,
        persistence::postgres::user::repository::PostgresUserRepository,
    },
    application::{
        auth::authenticated_user::AuthenticatedUser,
        user::{
            create_user::{CreateUserError, CreateUserInput, CreateUserOutput, CreateUserService},
            delete_user::{DeleteUserError, DeleteUserService},
            find_user::{FindUserError, FindUserService},
            update_user::{UpdateUserError, UpdateUserInput, UpdateUserOutput, UpdateUserService},
        },
    },
    domain::user::{entity::User, error::UserError},
};
use actix_web::{HttpResponse, http::StatusCode, web};
use log::info;
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
) -> Result<HttpResponse, ApiError> {
    let id: Uuid = Uuid::parse_str(&params)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "Invalid UUID format"))?;

    let user: User = service.find_by_id(&id).await?;

    Ok(HttpResponse::Ok().json(UserResponseDto {
        id: user.id.to_string(),
        username: user.username.as_str().into(),
        name: user.name.as_str().into(),
    }))
}

#[utoipa::path(
    post,
    path = "",
    request_body = CreateUserDto,
    tag = "Users",
    responses(
        (status = 201, description = "User created successfully", body = UserResponseDto),
        (status = 400, description = "Invalid data provided"),
        (status = 403, description = "Without access"),
        (status = 409, description = "Username already exists")
    )
)]
pub async fn create_user(
    service: web::Data<CreateUserService<PostgresUserRepository, Argon2Hasher>>,
    payload: web::Json<CreateUserDto>,
    actor: AuthenticatedUser,
) -> Result<HttpResponse, ApiError> {
    let cmd: CreateUserInput = CreateUserInput {
        username: payload.username.clone(),
        name: payload.name.clone(),
        password: payload.password.clone(),
    };

    let user: CreateUserOutput = service.execute(cmd, &actor).await?;

    Ok(HttpResponse::Ok().json(UserResponseDto {
        id: user.id.to_string(),
        username: user.username,
        name: user.name,
    }))
}

#[utoipa::path(
    patch,
    path = "/{id}",
    params(
        ("id" = String, Path, description = "User UUID")
    ),
    request_body = UpdateUserDto,
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
    actor: AuthenticatedUser,
) -> Result<HttpResponse, ApiError> {
    let id: Uuid = Uuid::parse_str(&params)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "Invalid UUID format"))?;

    let update_user: UpdateUserInput = UpdateUserInput {
        username: payload.username.clone(),
        name: payload.name.clone(),
        password: payload.password.clone(),
    };

    let updated_user: UpdateUserOutput = service.execute(id, update_user, &actor).await?;

    Ok(HttpResponse::Ok().json(UserResponseDto {
        id: updated_user.id.to_string(),
        username: updated_user.username,
        name: updated_user.name,
    }))
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
        (status = 403, description = "Whitout permission"),
        (status = 404, description = "User not found")
    )
)]
pub async fn delete_user(
    service: web::Data<DeleteUserService<PostgresUserRepository>>,
    params: web::Path<String>,
    actor: AuthenticatedUser,
) -> Result<HttpResponse, ApiError> {
    let id: Uuid = Uuid::parse_str(&params)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "Invalid UUID format"))?;

    service.execute(&id, &actor).await?;

    Ok(HttpResponse::Ok().finish())
}

impl From<UserError> for ApiError {
    fn from(err: UserError) -> Self {
        match err {
            UserError::InvalidPassword(msg) | UserError::InvalidUsername(msg) => {
                ApiError::new(StatusCode::BAD_REQUEST, msg)
            }
        }
    }
}

impl From<FindUserError> for ApiError {
    fn from(value: FindUserError) -> Self {
        match value {
            FindUserError::NotFound => ApiError::new(StatusCode::NOT_FOUND, "User not found"),
            FindUserError::RepositoryError => ApiError::internal_server_error(),
        }
    }
}

impl From<CreateUserError> for ApiError {
    fn from(err: CreateUserError) -> Self {
        match err {
            CreateUserError::UserError(user_err) => ApiError::from(user_err),
            CreateUserError::Forbidden => ApiError::new(
                StatusCode::FORBIDDEN,
                "You don't have access to create users",
            ),
            CreateUserError::AlreadyExists => {
                ApiError::new(StatusCode::CONFLICT, "Username already exists")
            }
            CreateUserError::InfrastructureError => ApiError::internal_server_error(),
        }
    }
}

impl From<UpdateUserError> for ApiError {
    fn from(err: UpdateUserError) -> Self {
        match err {
            UpdateUserError::UserError(user_err) => ApiError::from(user_err),
            UpdateUserError::NotFound => ApiError::new(StatusCode::NOT_FOUND, "User not found"),
            UpdateUserError::Forbidden => ApiError::new(
                StatusCode::FORBIDDEN,
                "You don't have access to edit this user",
            ),
            UpdateUserError::AlreadyExists => {
                ApiError::new(StatusCode::CONFLICT, "Username already exists")
            }
            UpdateUserError::InfrastructureError => ApiError::internal_server_error(),
        }
    }
}

impl From<DeleteUserError> for ApiError {
    fn from(err: DeleteUserError) -> Self {
        match err {
            DeleteUserError::NotFound => ApiError::new(StatusCode::NOT_FOUND, "User not found"),
            DeleteUserError::Forbidden => ApiError::new(
                StatusCode::FORBIDDEN,
                "You don't have access to delete this user",
            ),
            DeleteUserError::InfrastructureError => ApiError::internal_server_error(),
        }
    }
}
