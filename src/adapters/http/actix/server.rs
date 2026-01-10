use crate::{
    adapters::{
        hash::argon2::Argon2Hasher,
        http::actix::{ApiDoc, user::routes::routes as user_routes},
        persistence::postgres::user::repository::PostgresUserRepository,
    },
    application::user::{
        create_user::CreateUserService, delete_user::DeleteUserService, find_user::FindUserService,
        update_user::UpdateUserService,
    },
    config::http::ports::HttpConfig,
};
use actix_web::{App, HttpResponse, HttpServer, Responder, get, web};
use sea_orm::DatabaseConnection;
use std::io::Error;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[get("/")]
async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Hello world!")
}

pub async fn build_app(http_config: HttpConfig, db: DatabaseConnection) -> Result<(), Error> {
    let user_repo: PostgresUserRepository = PostgresUserRepository::new(db);
    let hasher: Argon2Hasher = Argon2Hasher;

    let find_user_service: FindUserService<PostgresUserRepository> =
        FindUserService::new(user_repo.clone());
    let create_user_service: CreateUserService<PostgresUserRepository, Argon2Hasher> =
        CreateUserService::new(user_repo.clone(), hasher.clone());
    let update_user_service: UpdateUserService<PostgresUserRepository, Argon2Hasher> =
        UpdateUserService::new(user_repo.clone(), hasher.clone());
    let delete_user_service: DeleteUserService<PostgresUserRepository> =
        DeleteUserService::new(user_repo.clone());

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(find_user_service.clone()))
            .app_data(web::Data::new(create_user_service.clone()))
            .app_data(web::Data::new(delete_user_service.clone()))
            .app_data(web::Data::new(update_user_service.clone()))
            .configure(user_routes)
            .service(SwaggerUi::new("/docs/{_:.*}").url("/api-doc/openapi.json", ApiDoc::openapi()))
            .service(hello)
    })
    .bind((http_config.host, http_config.port))?
    .run()
    .await
}
