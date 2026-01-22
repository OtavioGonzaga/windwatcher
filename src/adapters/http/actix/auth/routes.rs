use super::handler::token;
use actix_web::web;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/oauth").route("/token", web::post().to(token)));
}
