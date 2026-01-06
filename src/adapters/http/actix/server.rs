use crate::adapters::http::actix::ApiDoc;
use crate::adapters::persistence::postgres::user::repository::PostgresUserRepository;
use crate::adapters::{
    hash::argon2::Argon2Hasher, http::actix::user::routes::routes as user_routes,
};
use crate::config::http::ports::HttpConfig;
use crate::domain::user::service::UserService;
use actix_web::{App, HttpServer, get, web};
use actix_web::{HttpResponse, Responder};
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
    let user_service: UserService<PostgresUserRepository, Argon2Hasher> =
        UserService::new(user_repo, hasher);

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(user_service.clone()))
            .configure(user_routes)
            .service(SwaggerUi::new("/docs/{_:.*}").url("/api-doc/openapi.json", ApiDoc::openapi()))
            .service(hello)
    })
    .bind((http_config.host, http_config.port))?
    .run()
    .await
}
