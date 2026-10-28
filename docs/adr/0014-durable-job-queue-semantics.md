# 0014: Durable job queue semantics

- Status: Accepted
- Date: 2026-10-28

## Decision

Windwatcher v1 models durable jobs independently of domain and realtime events.
Queue delivery is at-least-once and has no processing-order guarantee. Handlers
must be idempotent where possible, using `JobId` as an idempotency identity.
Queue payloads are versioned JSON objects; kinds are open validated strings and
the resolved maximum attempt count is persisted per job.

Claims use renewable leases identified by a fresh UUIDv7 `LeaseId`; every
post-claim mutation is fenced by that lease, while `WorkerId` identifies one
worker process for attribution. A lease is authority to transition that claim,
not proof that its execution is unique. Each successful claim consumes one
attempt. Retryable failures use bounded exponential backoff with full jitter;
permanent failures fail immediately. Delayed work remains pending until available.

The portable clock precision is UTC microseconds. Timestamps and durations are
floored before comparison/addition; enqueue results match persisted precision.
Claim/renew reject leases below one microsecond and expiration overflow before
mutation, using the common library expiration calculation.

The standalone worker is a separate process role with configurable execution
slots. SQL queue state uses a dedicated database and explicit migration lifecycle
separate from primary storage. Redis uses atomic server-side scripts and has no
Windwatcher schema migration lifecycle. Maintenance is idempotent and safe under concurrent workers,
without a distributed maintenance lock. Domain commits do not automatically
enqueue jobs: there is no outbox, and a future business-critical enqueue may
require a transactional outbox decision.

V1 does not include queue ordering guarantees, deduplication, priority,
cancellation, recurring scheduling, attempt history, or automatic
`DomainEvent`-to-job mapping.

## Consequences

Jobs can be retried and recovered after worker failure without claiming
exactly-once execution. External effects can repeat, and domain commit followed
by process failure can leave a selected future job unenqueued. SQLite supports
multiple worker processes on a local filesystem; PostgreSQL, MySQL and Redis
support multi-host workers without changing queue semantics. Redis persistence
and protection against key eviction remain operator responsibilities; the adapter
cannot promise durability beyond the configured Redis service.
