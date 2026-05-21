//! Migration 0002 - create the `rooms` table.
//!
//! Creates the `rooms` table with the following columns:
//!
//! | Column            | Type        | Constraints              |
//! |:------------------|:------------|:-------------------------|
//! | `id`              | UUID        | PK, not null             |
//! | `room_type`       | TEXT        | not null (`direct`/`group`) |
//! | `title`           | TEXT        | nullable (groups only)   |
//! | `direct_room_key` | TEXT        | nullable, unique (direct rooms only) |
//! | `created_at`      | TIMESTAMPTZ | not null                 |
//!
//! `direct_room_key` is a deterministic sorted-UUID string that guarantees
//! uniqueness of direct (1-to-1) conversations.
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
                    .table(Rooms::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Rooms::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Rooms::RoomType).string().not_null())
                    .col(ColumnDef::new(Rooms::Title).string().null())
                    .col(
                        ColumnDef::new(Rooms::DirectRoomKey)
                            .string()
                            .null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(Rooms::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Rooms::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Rooms {
    Table,
    Id,
    RoomType,
    Title,
    DirectRoomKey,
    CreatedAt,
}
