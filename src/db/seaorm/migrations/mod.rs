//! SeaORM schema migrations for Windwatcher.
//!
//! [`Migrator`] registers all migrations in the order they must be applied.
//! Migrations are run automatically at startup via [`setup_database`].
//!
//! ## Migration order
//!
//! | Step | Module                               | Action                              |
//! |:----:|:-------------------------------------|:------------------------------------|
//! | 1    | `m20240101_000001_create_users`       | Create the `users` table            |
//! | 2    | `m20240101_000002_create_rooms`       | Create the `rooms` table            |
//! | 3    | `m20240101_000003_create_room_users`  | Create the `room_users` join table  |
//! | 4    | `m20240101_000004_create_messages`    | Create the `messages` table + index |
//!
//! Each migration includes an `up` (apply) and a `down` (rollback) method.
//!
//! [`setup_database`]: crate::db::seaorm::setup_database

mod m20240101_000001_create_users;
mod m20240101_000002_create_rooms;
mod m20240101_000003_create_room_users;
mod m20240101_000004_create_messages;

use sea_orm_migration::MigratorTrait;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![
            Box::new(m20240101_000001_create_users::Migration),
            Box::new(m20240101_000002_create_rooms::Migration),
            Box::new(m20240101_000003_create_room_users::Migration),
            Box::new(m20240101_000004_create_messages::Migration),
        ]
    }
}
