# Persistence

## Ports and adapters

`IdentityStore`, `SessionStore`, `ConversationStore`, and `MessageStore` are
application-owned semantic contracts.
Official adapters may use different physical representations, but must preserve
their atomic creation, uniqueness, refresh-token rotation, and revocation
semantics. The ports remain independent of SQLx and of any async runtime.

## SQLite

SQLite is the first official persistence adapter. Enable it with the `sqlite`
feature on the `windwatcher` crate. It is implemented with SQLx and exposes one
`SqliteStorage` resource implementing all four storage ports.

```rust,no_run
# async fn example() -> Result<(), windwatcher::SqliteStorageError> {
use windwatcher::{SqliteStorage, SqliteStorageOptions};

let storage = SqliteStorage::connect(SqliteStorageOptions::new("sqlite:windwatcher.db")).await?;
if !storage.check_schema().await?.is_current() {
    storage.migrate().await?;
}
# Ok(())
# }
```

Connecting never applies migrations. Applications choose when to call the
explicit `migrate` operation; `check_schema` and `migration_status` are
non-mutating inspection operations.

## Schema and representations

The embedded, forward-only SQLite migrations create `users`,
`local_credentials`, `sessions`, `conversations`, `conversation_members`, and `messages`.
Migration `0002_conversations.sql` adds Direct canonical keys, membership roles,
and Group ownership constraints. SQLite-specific representations are:

- Windwatcher UUIDv7 identifiers: 16-byte BLOBs;
- refresh-token SHA-256 fingerprints: 32-byte BLOBs;
- timestamps: signed Unix microseconds in INTEGER columns;
- password hashes: encoded PHC TEXT.

Direct keys are 32-byte BLOBs made from sorted 16-byte participant IDs.
Memberships use a `(conversation_id, user_id)` primary key, a user lookup index,
and a partial unique Owner index. Conversation deletion cascades memberships;
user deletion is restricted. Group creation, ownership transfer, and leaving are
transactional semantic operations.

All tables use SQLite `STRICT` mode, so these declared storage classes are
enforced in addition to the identifier and fingerprint length checks.

The adapter enables foreign keys on every connection, configures a nonzero busy
timeout, and uses WAL as a file-backed operational improvement. Correctness
comes from constraints, transactions, and atomic SQL statements rather than
from journal mode.

Migration `0003_messages.sql` adds message persistence and the nullable 16-byte
membership read marker without rebuilding memberships or requiring a marker FK.
Messages enforce active-with-content versus deleted-without-content state. Their
conversation FK cascades; sender FK restricts. A composite same-conversation reply
FK uses NO ACTION, preserving references to tombstones while allowing complete
conversation cascade deletion. `UNIQUE(conversation_id, id)` supports canonical
UUIDv7 ordering and read queries. BLOB ordering matches `MessageId::Ord`.

Message mutations use IMMEDIATE transactions to acquire the writer lock before
checking membership/roles and reply availability. Adding a Group member uses the
same mechanism and initializes its read marker with the current maximum persisted
message ID in the insertion transaction. Read operations use consistent snapshots.
Markers only advance; unread is computed, excluding own messages and tombstones.
These are portable semantics; BLOBs, STRICT, FKs and transaction syntax are SQLite
implementation choices. Content removal is logical, not guaranteed physical
erasure from WAL, database pages or backups.

## Migrations

Migrations are embedded into the library binary from
`crates/windwatcher/migrations/sqlite` and use SQLx's `_sqlx_migrations` ledger.
They are immutable and forward-only after release. SQLx validates applied
checksums when migrations run; schema inspection also rejects unknown, dirty,
or checksum-mismatched applied migrations.

## Error normalization and contracts

SQLx and SQLite remain adapter details. `IdentityStore`, `SessionStore`, `ConversationStore`, and `MessageStore`
return their normalized application errors, while `SqliteStorageError` describes
connection and migration infrastructure failures. Contract tests exercise the
storage behavior independently from physical table names, and SQLite-specific
tests additionally verify its schema and concurrency behavior.

`tests/support/message_store_contract.rs` exercises portable message lifecycle,
authorization, reply/tombstone integrity, read state, Group join boundaries and
conversation deletion. Future official adapters should run this same contract,
without adopting SQLite-specific physical assertions.

## Current limitations

Only SQLite is implemented. There is no automatic startup migration or other
database adapter. Configuration and explicit migration CLI commands belong to
the server, not the reusable library. Message pagination remains future work.
