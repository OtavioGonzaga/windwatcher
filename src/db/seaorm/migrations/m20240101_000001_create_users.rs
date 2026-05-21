//! Migration 0001 - create the `users` table.
//!
//! Creates the `users` table with the following columns:
//!
//! | Column          | Type                    | Constraints              |
//! |:----------------|:------------------------|:-------------------------|
//! | `id`            | UUID                    | PK, not null             |
//! | `username`      | TEXT                    | not null                 |
//! | `email`         | TEXT                    | not null, unique         |
//! | `password_hash` | TEXT                    | not null                 |
//! | `role`          | TEXT                    | not null, default `user` |
//! | `created_at`    | TIMESTAMPTZ             | not null                 |
//! | `updated_at`    | TIMESTAMPTZ             | not null                 |
//!
//! `down` drops the table entirely.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Users::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Users::Username).string().not_null())
                    .col(
                        ColumnDef::new(Users::Email)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Users::PasswordHash).string().not_null())
                    .col(
                        ColumnDef::new(Users::Role)
                            .string()
                            .not_null()
                            .default("user"),
                    )
                    .col(
                        ColumnDef::new(Users::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Users::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Users::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Username,
    Email,
    PasswordHash,
    Role,
    CreatedAt,
    UpdatedAt,
}
