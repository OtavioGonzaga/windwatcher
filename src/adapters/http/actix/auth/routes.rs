use super::handler::login;
use actix_web::web;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/auth").route("/login", web::post().to(login)));
}
