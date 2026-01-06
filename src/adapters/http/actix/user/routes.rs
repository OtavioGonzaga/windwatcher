use super::handler::{create_user, find_by_id};
use actix_web::web;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/users")
            .route("", web::post().to(create_user))
            .route("/{id}", web::get().to(find_by_id)),
    );
}
