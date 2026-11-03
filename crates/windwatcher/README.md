# Windwatcher

An embeddable messaging engine for Rust applications. Build messaging into your
own application without adopting a particular HTTP framework or running a
separate Windwatcher service.

**Early development; not production-ready.** APIs may change between 0.x releases.
Requires Rust 1.99 or later (edition 2024). Licensed under MIT.

Looking for a ready-to-run HTTP/WebSocket service instead? Use
[`windwatcher-server`](https://crates.io/crates/windwatcher-server).
The library and server are independently versioned.

## What you can build with it

- Local users with Argon2id password authentication, enable/disable controls,
  sessions, refresh-token rotation and revocation.
- Direct conversations and groups with membership, Owner/Admin/Member roles and
  ownership transfer.
- Text messages, replies, edits, deletion tombstones, per-user read state and
  unread counts.
- Paginated inboxes, member lists and message history with opaque keyset cursors,
  sender attribution and reply previews.
- Optional idempotent conversation creation and message sends.
- Application reactions to committed mutations through typed `DomainEvent` values.

The crate supplies application use cases and storage traits, plus optional
official adapters. Your host owns authentication at its transport boundary,
runtime setup, configuration and event handling. HTTP, WebSockets, listeners and
the operator CLI live in the server crate, not here.

## Get started with SQLite

No optional features are enabled by default. To use the SQLite adapter:

```bash
cargo add windwatcher --features sqlite
cargo add tokio --features macros,rt-multi-thread
cargo add time
```

This complete example uses an ephemeral in-memory database, provisions a local
user, creates a group and sends a message:

```rust
use time::OffsetDateTime;
use windwatcher::{
    Argon2idPasswordHasher, CreateGroupConversation, CreateLocalUser, ListMessages,
    MessagePolicy, PageRequest, SecretPassword, SendMessage, SqliteStorage,
    SqliteStorageOptions,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let storage = SqliteStorage::connect(SqliteStorageOptions::new("sqlite::memory:")).await?;
    storage.migrate().await?;

    // Demo credentials only. Obtain real passwords securely from your caller.
    let password = SecretPassword::new("example-password".to_owned())?;
    let alice = CreateLocalUser::new(&storage, &Argon2idPasswordHasher)
        .execute("alice", &password, OffsetDateTime::now_utc())
        .await?;

    let group = CreateGroupConversation::new(&storage)
        .execute(alice.id(), "Project chat", &[], OffsetDateTime::now_utc())
        .await?
        .into_value();

    let policy = MessagePolicy::default();
    let sent = SendMessage::new(&storage, &policy)
        .execute(
            alice.id(),
            group.id(),
            "Hello, Windwatcher!",
            None,
            OffsetDateTime::now_utc(),
        )
        .await?;
    println!("Message: {}", sent.value().id());

    let history = ListMessages::new(&storage)
        .execute(alice.id(), group.id(), PageRequest::default())
        .await?;
    assert_eq!(history.items().len(), 1);
    Ok(())
}
```

Use a file path such as `windwatcher.db` instead of `sqlite::memory:` to persist
data. Connecting does not apply migrations: choose when your application calls
`migrate()`. The example deliberately discards domain events; a host that needs
reactions should handle each mutation's `Outcome<T>` events explicitly.

## Features and adapters

| Feature | Provides |
| --- | --- |
| `sqlite` | `SqliteStorage` primary persistence |
| `postgres` | `PostgresStorage` primary persistence |
| `mysql` | `MySqlStorage` primary persistence |
| `mongodb` | `MongoDbStorage` primary persistence; requires a transaction-capable replica set |
| `jwt-hs256` | HS256 JWT access-token codec |
| `queue-sqlite` | `SqliteQueue` durable job queue |
| `queue-postgres` | `PostgresQueue` durable job queue |
| `queue-mysql` | `MySqlQueue` durable job queue |
| `queue-redis` | `RedisQueue` durable job queue |

All four primary adapters implement identity, session, conversation, message,
query and atomic-create storage contracts. Primary storage and queue selection
are independent; enabling a primary adapter does not enable a queue.

Application ports return `Send` futures without prescribing an executor. The
official database and queue adapters require Tokio. The built-in Argon2id hasher
performs CPU-bound work on the polling thread; asynchronous hosts should wrap the
`PasswordHasher` port with bounded blocking-pool execution.

## Boundaries to know

- Read state is private to each user, not read receipts. Attachments, reactions,
  threads and full-text search are not implemented.
- Session revocation prevents refresh; it does not invalidate already-issued
  access tokens before their expiration.
- Domain events are in-process facts, not a persisted event log or automatic
  notifications. They are not automatically enqueued as jobs.
- Durable queues provide at-least-once delivery with fenced, renewable leases,
  not processing-order or exactly-once guarantees. There is no transactional
  outbox coupling primary storage to a queue.

## Documentation

- [Rust API reference](https://docs.rs/windwatcher)
- [Documentation index](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/README.md)
- [Authentication](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/authentication.md) and [sessions](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/sessions.md)
- [Conversations](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/conversations.md) and [messages](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/messages.md)
- [Persistence](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/persistence.md) and [durable jobs](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/jobs.md)
- [Source and issues](https://github.com/OtavioGonzaga/windwatcher) · [Contributing](https://github.com/OtavioGonzaga/windwatcher/blob/main/CONTRIBUTING.md) · [Changelog](https://github.com/OtavioGonzaga/windwatcher/blob/main/crates/windwatcher/CHANGELOG.md) · [MIT license](https://github.com/OtavioGonzaga/windwatcher/blob/main/LICENSE)

Repository guides follow `main`; use the matching `windwatcher-vX.Y.Z` Git tag
when consulting documentation for a specific release.
