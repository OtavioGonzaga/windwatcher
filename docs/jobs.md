# Durable jobs

Windwatcher's optional queue stores explicit durable work. A job is not a
`DomainEvent` or `RealtimeEvent`; the server does not automatically enqueue
events. A future feature must select a domain fact and explicitly construct a
versioned job. There is no transactional outbox in v1, so a domain commit may
succeed while a later process crash loses a separately requested enqueue.

## Delivery and envelope

Delivery is **at-least-once**. A handler may execute more than once, including
after an external side effect succeeded but completion could not be persisted.
Handlers must be idempotent where practical; `JobId` is the natural external
idempotency identity. There is no processing-order guarantee across jobs,
workers, or adapters.

Every job has an open lowercase ASCII `JobKind`, a `payload_version` starting at
1, and a JSON-object payload. The object root is required; nested JSON values may
have any JSON type. The default serialized UTF-8 payload ceiling is 256 KiB.
Payloads should carry resource identifiers rather than large documents or
blobs. Handler identity is `(JobKind, payload_version)` so versions can coexist.
There is no deduplication or priority.

`max_attempts` is resolved and persisted when a job is enqueued. Enqueue can set
delayed `available_at`; delayed jobs remain pending and are not claimable until
that instant.

## State and leases

Persisted states are `pending`, `leased`, `completed`, and `failed`. Every
successful claim increments `attempts`, including claims later abandoned by a
crashed worker. A claim assigns a fresh `LeaseId`, a process-lifetime `WorkerId`,
and an expiry. `LeaseId` fences renew, complete, retry, and fail operations; a
stale claim has no authority. A lease is authority to transition that claim,
not proof that its execution is unique. Expired leases may be reclaimed, and a
crashed final attempt is finalized as failed rather than claimed beyond its
persisted maximum.

Retryable handler failures use capped exponential backoff with full jitter.
Permanent failures transition directly to failed. Unsupported kinds/versions
and invalid typed payloads are permanent failures. Failure codes are stable
machine-readable values; messages are bounded diagnostics, not an API contract.

Operators can inspect without exposing payloads:

```bash
windwatcher jobs list
windwatcher jobs list --status failed --limit 50
windwatcher jobs retry <job-id>
```

Listing defaults to all states and 50 rows, with a maximum of 100. Manual retry
is allowed only for failed jobs: it preserves `JobId`, kind, payload, version,
creation time and `max_attempts`, while resetting attempts to zero and making
the job available immediately. Failed jobs are retained indefinitely.

## Worker operations

Run one or more independent processes with `windwatcher worker`. One process has
one `WorkerId` and configurable execution slots; `concurrency` is not a count of
worker identities. A slot claims only when ready to execute. Long handlers
renew their leases; lease loss or renewal infrastructure uncertainty cancels
the local future and leaves recovery to lease expiry.

SIGINT/SIGTERM stops new claims, while active handlers continue and renew during
`shutdown_grace_period`. If grace expires, remaining futures are dropped without
releasing leases, allowing normal expiry and recovery. V1 has no global handler
timeout, job cancellation API, scheduler, recurring jobs, or production handler
registry entries.

The runtime supervises every slot and maintenance task. An unexpected exit or
panic (also during graceful drain) stops the worker with an error and cancels
the remaining tasks without releasing leases. A systemd/container supervisor
can restart the process; jobs recover through normal lease expiry rather than
silently running with fewer execution slots.

Workers periodically finalize exhausted expired leases and prune completed jobs
older than the configured retention (seven days by default). Maintenance is
idempotent and concurrency-safe; there is no distributed maintenance lock.

Adapters share the portable queue contract suite, covering expired-lease reclaim
and stale fencing, attempt exhaustion, permanent failure, manual retry, listing
order/filtering and concurrent maintenance/retention. Physical schema and
connection behavior remain adapter-specific tests.

## Adapter matrix

All four official adapters implement the same `QueueStore`, job model and lease
state machine. The library enables only requested features; the official server
includes all four so `queue.provider` selects the backend at runtime.

| Adapter / library feature | Durable backend | Migrations | Multiple processes | Multiple hosts |
| --- | --- | --- | --- | --- |
| SQLite / `queue-sqlite` | Local database file | Yes | Yes, local filesystem | No |
| PostgreSQL / `queue-postgres` | PostgreSQL database | Yes | Yes | Yes |
| MySQL / `queue-mysql` | MySQL 8 database | Yes | Yes | Yes |
| Redis / `queue-redis` | Redis, subject to its persistence configuration | Not applicable | Yes | Yes |

Network adapters use Tokio and Rustls TLS, without an OpenSSL/native-tls system
dependency. Use the clients' normal TLS URL options (`rediss://` for Redis).
PostgreSQL/MySQL queue features do not enable primary persistence adapters.

## SQL databases and migrations

Queue SQLite is a separate adapter, database file (`queue.db` by default), and
migration lifecycle from primary storage (`windwatcher.db`). The server rejects
configuration resolving both to the same file. SQLite supports multiple worker
processes sharing a local database file with WAL and a busy timeout; it is not
advertised for multi-host NFS or distributed filesystems.

Migrations are explicit:

```bash
windwatcher migrate                 # storage and configured queue
windwatcher migrate --storage
windwatcher migrate --queue
windwatcher migrate status --queue
```

PostgreSQL and MySQL require an existing **dedicated queue database**, independent
of primary persistence and its SQLx migration history. Using the same database
server is fine; sharing the primary database is not the supported v1 topology.
Provision the queue database and credentials separately, then run
`windwatcher migrate --queue` before starting workers. `connect()` never creates a
network database or applies migrations. MySQL targets MySQL 8, not MariaDB
certification. Claims use short locking transactions and `SKIP LOCKED`; no handler
runs inside a database transaction.

Worker and jobs commands check SQL schema compatibility but never migrate. Queue
operations receive explicit caller-supplied UTC time; distributed hosts should
maintain normal clock synchronization. All adapters use UTC microsecond precision:
timestamps and durations are floored before comparison/addition, including for
negative Unix timestamps. Enqueue returns the same normalized timestamps that
are persisted. Claim/renew reject durations below 1 µs or expiration overflow
before mutating state; lease expiry is computed by the common library helper.
All adapters persist signed Unix microseconds, not native database date/time
types. SQLite uses `INTEGER`, PostgreSQL/MySQL use signed `BIGINT`, and Redis
encodes the same `i64` lexicographically. The supported `OffsetDateTime` range is
the common model's range, including its minimum and maximum after microsecond
normalization; no adapter imposes its own year or timezone/session restrictions.

Payloads are opaque serialized UTF-8 JSON: PostgreSQL uses `BYTEA`, MySQL uses
`MEDIUMBLOB`, SQLite retains text, and Redis retains strings. No database JSON
parser transforms or restricts their contents. Valid payloads preserve NUL,
Unicode, numeric values and nesting beyond database JSON/default decoder limits.
Restoration uses a shared stack-growing decoder without a fixed nesting limit;
malformed JSON or non-object roots still follow the invalid-payload failure path.
Diagnostic messages likewise preserve all UTF-8 allowed by `JobFailure`, including
NUL, within its existing 4096-byte limit; PostgreSQL/MySQL store them as bytes.

## Redis operations

Redis uses hashes, lexicographic sorted indexes and atomic Lua transitions, not
Streams, Pub/Sub or blocking lists. Times are exact signed microsecond encodings;
sorted-set floating-point scores are not semantic timestamps. Claim normalization
and maintenance scripts process bounded batches. Listing and claims use indexes,
not database scans. Redis scripts are reloadable after a restart/cache loss.

Use a standard Redis endpoint; Redis Cluster and Sentinel-specific discovery are
not supported in v1. Choose a dedicated `key_prefix` for each independent queue.
Prefixes must contain 1..=256 UTF-8 bytes without control characters.

**Durability is an operator responsibility:** configure Redis persistence to meet
your durability requirements and avoid eviction policies that can silently evict
queue keys under memory pressure. Windwatcher does not change Redis persistence
or eviction configuration and does not require privileged `CONFIG` commands.
It cannot recover queue data lost by Redis. Pub/Sub is not queue persistence.

Redis has no Windwatcher schema migrations. Both migration commands succeed and
report `Queue (Redis): migrations not applicable`; worker and jobs commands do
not require a fake schema version.

## Minimal provider configuration

Queue configuration is optional. When `[queue]` exists, its default provider is
SQLite. Worker/retry settings and `JobPolicy` stay common under `[queue]`; only the
selected provider block is resolved. The following are alternative configurations,
not sections to concatenate. Credentials shown are development placeholders.

```toml
[queue]
provider = "sqlite"
[queue.sqlite]
path = "queue.db"
```

```toml
[queue]
provider = "postgres"
[queue.postgres]
url = "postgresql://windwatcher:password@localhost/windwatcher_queue"
max_connections = 8
acquire_timeout = "5s"
```

```toml
[queue]
provider = "mysql"
[queue.mysql]
url = "mysql://windwatcher:password@localhost/windwatcher_queue"
max_connections = 8
acquire_timeout = "5s"
```

```toml
[queue]
provider = "redis"
[queue.redis]
url = "redis://localhost/0"
key_prefix = "windwatcher:queue"
```

Network connection URLs are sensitive; diagnostics do not print them. Do not put
real credentials in version control or command-line arguments. The server also
supports its existing `WINDWATCHER__QUEUE__...` configuration environment layer.

## Real adapter tests

Ordinary `cargo test --workspace --all-targets --all-features` runs portable and
SQLite tests without external services. Network integration tests are explicitly
ignored, not silently passed. Start disposable PostgreSQL, MySQL 8 and Redis
services, and export their URLs:

```bash
export WINDWATCHER_TEST_POSTGRES_URL='postgresql://postgres:password@localhost:5432/postgres'
export WINDWATCHER_TEST_MYSQL_URL='mysql://root:password@localhost:3306/mysql'
export WINDWATCHER_TEST_REDIS_URL='redis://localhost:6379/0'

cargo test -p windwatcher --no-default-features --features queue-postgres --test postgres_queue -- --ignored
cargo test -p windwatcher --no-default-features --features queue-mysql --test mysql_queue -- --ignored
cargo test -p windwatcher --no-default-features --features queue-redis --test redis_queue -- --ignored
cargo test -p windwatcher-server --test cli network_queue_migration_and_jobs_cli -- --ignored
```

These commands fail if required URLs are absent. SQL tests require permission to
create/drop uniquely named disposable databases; never use production/shared
databases. Redis tests use UUID-scoped `windwatcher:test:*` namespaces and clean
only their own keys with bounded `SCAN`/`DEL`, never `FLUSHDB` or `FLUSHALL`.
Failed assertions may leave test resources behind; discard the development
services after the run. All adapters execute the same shared queue contract,
plus independent pool/client race and schema/namespace-specific tests.
