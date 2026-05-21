//! Migration 0004 - create the `messages` table and its pagination index.
//!
//! Creates the `messages` table:
//!
//! | Column       | Type        | Constraints                        |
//! |:-------------|:------------|:-----------------------------------|
//! | `id`         | UUID (v7)   | PK, not null (doubles as cursor)   |
//! | `room_id`    | UUID        | not null, FK -> `rooms(id)` CASCADE |
//! | `sender_id`  | UUID        | not null, FK -> `users(id)` CASCADE |
//! | `content`    | TEXT        | not null                           |
//! | `created_at` | TIMESTAMPTZ | not null                           |
//!
//! Because `id` is a UUIDv7 (monotonically increasing), it serves as a
//! natural time-ordered cursor for keyset pagination.
//!
//! Also creates a composite index `idx_messages_room_id_id` on
//! `(room_id, id)` to accelerate the common query:
//! ```sql
//! SELECT * FROM messages
//! WHERE room_id = ? AND id < ?
//! ORDER BY id DESC
//! LIMIT ?
//! ```
//!
//! `down` drops the `messages` table (the index is dropped implicitly).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Messages::Table)
                    .if_not_exists()
                    // UUIDv7 - serves as PK and time-ordered cursor
                    .col(ColumnDef::new(Messages::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Messages::RoomId).uuid().not_null())
                    .col(ColumnDef::new(Messages::SenderId).uuid().not_null())
                    .col(ColumnDef::new(Messages::Content).text().not_null())
                    .col(
                        ColumnDef::new(Messages::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    // FK -> rooms
                    .foreign_key(
                        ForeignKey::create()
                            .from(Messages::Table, Messages::RoomId)
                            .to(Rooms::Table, Rooms::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    // FK -> users (sender)
                    .foreign_key(
                        ForeignKey::create()
                            .from(Messages::Table, Messages::SenderId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Composite index to accelerate paginated message queries:
        //   SELECT ... WHERE room_id = ? AND id < ? ORDER BY id DESC LIMIT ?
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_messages_room_id_id")
                    .table(Messages::Table)
                    .col(Messages::RoomId)
                    .col(Messages::Id)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Messages::Table).to_owned())
            .await
    }
}

// ── Iden definitions ──────────────────────────────────────────────────────────

#[derive(DeriveIden)]
enum Messages {
    Table,
    Id,
    RoomId,
    SenderId,
    Content,
    CreatedAt,
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
