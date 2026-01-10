use super::handler::{create_user, delete_user, find_by_id, update_user};
use actix_web::web;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/users")
            .route("", web::post().to(create_user))
            .route("/{id}", web::get().to(find_by_id))
            .route("/{id}", web::patch().to(update_user))
            .route("/{id}", web::delete().to(delete_user)),
    );
}
