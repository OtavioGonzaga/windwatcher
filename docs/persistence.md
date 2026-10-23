# Persistence

## Ports and adapters

`IdentityStore` and `SessionStore` are application-owned semantic contracts.
Official adapters may use different physical representations, but must preserve
their atomic creation, uniqueness, refresh-token rotation, and revocation
semantics. The ports remain independent of SQLx and of any async runtime.

## SQLite

SQLite is the first official persistence adapter. Enable it with the `sqlite`
feature on the `windwatcher` crate. It is implemented with SQLx and exposes one
`SqliteStorage` resource implementing both storage ports.

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
`local_credentials`, and `sessions`. SQLite-specific representations are:

- Windwatcher UUIDv7 identifiers: 16-byte BLOBs;
- refresh-token SHA-256 fingerprints: 32-byte BLOBs;
- timestamps: signed Unix microseconds in INTEGER columns;
- password hashes: encoded PHC TEXT.

The adapter enables foreign keys on every connection, configures a nonzero busy
timeout, and uses WAL as a file-backed operational improvement. Correctness
comes from constraints, transactions, and atomic SQL statements rather than
from journal mode.

## Migrations

Migrations are embedded into the library binary from
`crates/windwatcher/migrations/sqlite` and use SQLx's `_sqlx_migrations` ledger.
They are immutable and forward-only after release. SQLx validates applied
checksums when migrations run; schema inspection also rejects unknown, dirty,
or checksum-mismatched applied migrations.

## Error normalization and contracts

SQLx and SQLite remain adapter details. `IdentityStore` and `SessionStore`
return their normalized application errors, while `SqliteStorageError` describes
connection and migration infrastructure failures. Contract tests exercise the
storage behavior independently from physical table names, and SQLite-specific
tests additionally verify its schema and concurrency behavior.

## Current limitations

Only SQLite is implemented. There is no automatic startup migration, runtime
database selection, environment/configuration parsing, CLI command, or other
database adapter.
