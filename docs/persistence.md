# Persistence

## Consumer query projections

`MessagingQueryStore` loads exact enabled-user discovery, inbox/detail and public
member projections. SQLite inbox reads use one keyset query plus one bulk
last-message hydration query in a normal read transaction. Detail and member
queries likewise authorize within the same read snapshot. Query counts do not
grow with page size; identities and reply targets are joined, not loaded per item.

Migration 0004 adds `(conversation_id, joined_at, user_id)` for member pagination.
Derived inbox activity uses primary conversation/message timestamps; query-plan
tests verify indexed message probes without adding redundant message indexes or
a materialized activity column.

`CreateCommandStore` atomically persists optional retry-deduplication records with
conversation/message creation, reusing transaction-local create primitives. SQLite
migration 0004 also adds strict binary-UUID/hash `idempotency_records` and an
expiration index. Records deliberately outlive physical resource deletion. Keys
are hashed and fingerprints use semantic input; cleanup is opportunistic within
keyed write transactions. See [ADR 0011](adr/0011-idempotent-create-commands.md).

## Ports and adapters

`IdentityStore`, `SessionStore`, `ConversationStore`, and `MessageStore` are
application-owned semantic contracts.
Official adapters may use different physical representations, but must preserve
their atomic creation, uniqueness, refresh-token rotation, and revocation
semantics. The ports remain independent of SQLx and of any async runtime.

## Official SQL adapters

The library provides SQLite, PostgreSQL, MySQL, and MongoDB implementations of all six
ports: identity, sessions, conversations, messages, messaging queries, and
atomic create commands. Enable `sqlite`, `postgres`, or `mysql` independently
on the `windwatcher` crate; the official server enables all four. Selecting a
primary storage provider is independent of queue provider selection.

Connections never apply migrations implicitly. Each adapter embeds its own
forward-only migration set and exposes `migrate`, `check_schema`, and
`migration_status` operations. The server selects its primary adapter with
`storage.provider` (`sqlite` by default, or `postgres` / `mysql` / `mongodb`) and the
explicit `windwatcher storage migrate` and `storage status` CLI commands apply
to the selected database. PostgreSQL and MySQL URLs must target dedicated,
pre-created databases. See [server configuration](server-configuration.md).

The adapters preserve the same application ports and shared contracts;
physical representations and locking strategies remain adapter-specific.
PostgreSQL uses native UUID and bytea values; MySQL uses binary UUID/hash
columns. Both network adapters store timestamps as signed Unix microseconds.

MongoDB stores UUIDs as BSON binary subtype 4 and timestamps as signed Int64 Unix
microseconds. It uses explicit versioned migrations and validates topology on
connect: transaction-capable replica sets are required, while standalones and
`mongos` are rejected. Reads use snapshot transactions and primary read
preference; writes use transactions with majority write concern. MongoDB remains
independent from queue selection; no MongoDB queue adapter is implied.

Network message mutations lock the conversation before checking current
membership/role and then lock the message. Membership changes use the same
conversation lock, so committed removal or demotion cannot be bypassed by stale
authorization. Consumer projections and multi-statement message reads use explicit
repeatable-read, read-only transactions, independently of server isolation defaults.
MySQL mutation connections explicitly use read committed to re-read current state
after waiting for locks. Keyed commands reserve their scoped identity in the same
transaction before creating resources; concurrent retries wait and then replay the
committed result (or report changed input). Failed commands roll back the reservation.

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

There is no automatic startup migration. Configuration and explicit migration
CLI commands belong to the server, not the reusable library.

## Paginated message history

`MessageStore::list_messages` receives an actor, conversation, optional decoded
`MessageId` boundary and validated `PageLimit`. It returns `MessagePageSlice`
with at most the requested size, strictly descending IDs in the requested
conversation, including tombstones. `has_more` means an older item actually
exists; an exactly-full final slice does not imply continuation. Boundaries need
not reference physical rows. Listing never changes read state.

Adapters must check conversation existence and current membership in the same
logical read as history. Missing conversations and nonmembers are separate
outcomes. These portable semantics contain no SQL or cursor encoding assumptions.

SQLite uses a deferred read transaction for authorization and retrieval, without
`BEGIN IMMEDIATE`. It queries `WHERE conversation_id = ?`, optionally adding
`AND id < ?`, then `ORDER BY id DESC LIMIT ?` with `limit + 1` rows. The extra
row determines `has_more` and is removed. No deletion filter excludes tombstones.
The existing unique `(conversation_id, id)` key supports this access path; no
migration was required. Cursor validation and encoding belong to `ListMessages`,
not SQLite. See [pagination](pagination.md).

## Real network-adapter regression tests

Set `WINDWATCHER_TEST_POSTGRES_URL` and `WINDWATCHER_TEST_MYSQL_URL` to disposable
services with database-creation privileges, then run:

```sh
cargo test -p windwatcher --all-features --test postgres_storage --test mysql_storage -- --include-ignored
cargo test -p windwatcher --all-features --lib read_snapshot_survives -- --include-ignored
```

The integration suites create isolated databases and exercise shared contracts,
concurrent keyed commands, and authorization changes while a second pool holds
the conversation lock. Snapshot tests explicitly set the reader session default
to `READ COMMITTED` and verify that a concurrent committed insert is invisible
until the read transaction ends. They create and remove uniquely named test tables
in the database named by each URL; never point these URLs at production data.
