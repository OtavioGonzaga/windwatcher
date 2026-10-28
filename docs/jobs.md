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

## SQLite and migrations

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

Worker and jobs commands check schema compatibility but never migrate. Queue
operations receive explicit caller-supplied UTC time; distributed hosts should
maintain normal clock synchronization. All adapters use UTC microsecond precision:
timestamps and durations are floored before comparison/addition, including for
negative Unix timestamps. Enqueue returns the same normalized timestamps that
are persisted. Claim/renew reject durations below 1 µs or expiration overflow
before mutating state; lease expiry is computed by the common library helper.
PostgreSQL, MySQL and Redis adapters are
out of scope for PR 13.
