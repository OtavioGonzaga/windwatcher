use crate::{
    adapters::{
        auth::local::LocalAuthenticator,
        hash::argon2::Argon2Hasher,
        http::actix::{
            ApiDoc, auth::routes::routes as auth_routes, user::routes::routes as user_routes,
        },
        persistence::postgres::user::repository::PostgresUserRepository,
        token::jwt::JwtService,
    },
    application::{
        auth::login::Login,
        user::{
            create_user::CreateUserService, delete_user::DeleteUserService,
            find_user::FindUserService, update_user::UpdateUserService,
        },
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
    HttpResponse::Ok().body("Hello, WindWatcher!")
}

pub async fn build_app(http_config: HttpConfig, db: DatabaseConnection) -> Result<(), Error> {
    let user_repository: PostgresUserRepository = PostgresUserRepository::new(db);
    let hasher: Argon2Hasher = Argon2Hasher;
    let token_service: JwtService = JwtService::new(http_config.jwt_secret.clone(), 30 * 60);
    let authenticator: LocalAuthenticator<PostgresUserRepository, Argon2Hasher> =
        LocalAuthenticator::new(user_repository.clone(), hasher.clone());

    let find_user_service: FindUserService<PostgresUserRepository> =
        FindUserService::new(user_repository.clone());
    let create_user_service: CreateUserService<PostgresUserRepository, Argon2Hasher> =
        CreateUserService::new(user_repository.clone(), hasher.clone());
    let update_user_service: UpdateUserService<PostgresUserRepository, Argon2Hasher> =
        UpdateUserService::new(user_repository.clone(), hasher.clone());
    let delete_user_service: DeleteUserService<PostgresUserRepository> =
        DeleteUserService::new(user_repository.clone());
    let login: Login<LocalAuthenticator<PostgresUserRepository, Argon2Hasher>, JwtService> =
        Login::new(authenticator.clone(), token_service.clone());

    let addrs: (String, u16) = (http_config.host.clone(), http_config.port);

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(token_service.clone()))
            .app_data(web::Data::new(login.clone()))
            .app_data(web::Data::new(find_user_service.clone()))
            .app_data(web::Data::new(create_user_service.clone()))
            .app_data(web::Data::new(delete_user_service.clone()))
            .app_data(web::Data::new(update_user_service.clone()))
            .configure(user_routes)
            .configure(auth_routes)
            .service(SwaggerUi::new("/docs/{_:.*}").url("/api-doc/openapi.json", ApiDoc::openapi()))
            .service(hello)
    })
    .bind(addrs)?
    .run()
    .await
}
