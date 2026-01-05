use actix_web::{App, Error, HttpResponse, HttpServer, Responder, Result, get};

use crate::config::http::ports::HttpConfig;

#[get("/")]
async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Hello world!")
}

pub async fn build_app(http_config: HttpConfig) -> Result<(), Error> {
    HttpServer::new(|| App::new().service(hello))
        .bind((http_config.host, http_config.port))?
        .run()
        .await?;

    Ok(())
}
