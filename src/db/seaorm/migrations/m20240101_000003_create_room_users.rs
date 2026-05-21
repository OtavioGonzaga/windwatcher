//! Migration 0003 - create the `room_users` join table.
//!
//! Creates the `room_users` many-to-many join table that associates users with
//! rooms and tracks per-user read state:
//!
//! | Column                | Type        | Constraints                        |
//! |:----------------------|:------------|:-----------------------------------|
//! | `room_id`             | UUID        | PK (part 1), FK -> `rooms(id)` CASCADE |
//! | `user_id`             | UUID        | PK (part 2), FK -> `users(id)` CASCADE |
//! | `unread_count`        | BIGINT      | not null, default `0`              |
//! | `joined_at`           | TIMESTAMPTZ | not null                           |
//! | `last_read_message_id`| UUID        | nullable                           |
//!
//! The composite primary key `(room_id, user_id)` enforces the invariant that
//! each user appears at most once per room.
//!
//! Both foreign keys use `ON DELETE CASCADE` so removing a room or user
//! automatically cleans up membership rows.
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
                    .table(RoomUsers::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(RoomUsers::RoomId).uuid().not_null())
                    .col(ColumnDef::new(RoomUsers::UserId).uuid().not_null())
                    .col(
                        ColumnDef::new(RoomUsers::UnreadCount)
                            .big_integer()
                            .not_null()
                            .default(0i64),
                    )
                    .col(
                        ColumnDef::new(RoomUsers::JoinedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(RoomUsers::LastReadMessageId).uuid().null())
                    // Composite primary key
                    .primary_key(
                        Index::create()
                            .col(RoomUsers::RoomId)
                            .col(RoomUsers::UserId),
                    )
                    // FK -> rooms
                    .foreign_key(
                        ForeignKey::create()
                            .from(RoomUsers::Table, RoomUsers::RoomId)
                            .to(Rooms::Table, Rooms::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    // FK -> users
                    .foreign_key(
                        ForeignKey::create()
                            .from(RoomUsers::Table, RoomUsers::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(RoomUsers::Table).to_owned())
            .await
    }
}

// ── Iden definitions ──────────────────────────────────────────────────────────

#[derive(DeriveIden)]
enum RoomUsers {
    Table,
    RoomId,
    UserId,
    UnreadCount,
    JoinedAt,
    LastReadMessageId,
}

/// Minimal Iden stubs for referenced tables (FK declarations).
#[derive(DeriveIden)]
enum Rooms {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
