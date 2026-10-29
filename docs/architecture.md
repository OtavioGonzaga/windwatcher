# Architecture

## Project model

Windwatcher is library-first:

```text
windwatcher
→ reusable and embeddable library

windwatcher-server
→ official standalone application built on the library
```

## Dependency direction

```text
windwatcher-server
        ↓
   windwatcher
```

The library must not depend on server-specific concerns.

The server is the application composition root: CLI parsing, runtime paths,
configuration, secret resolution, tracing, adapter selection, and operator
workflows belong there rather than in the reusable crate.

Primary persistence is selected independently from the durable job queue.
The library provides SQLite, PostgreSQL, MySQL, and MongoDB adapters for its persistence
ports; the official server resolves `storage.provider` and dispatches through a
single storage facade. Migrations remain explicit operator actions.

## Architectural principles

- Design library-first.
- Use Ports and Adapters as the architectural direction.
- Keep application and domain behavior independent of HTTP.
- Keep infrastructure adapters replaceable.
- Make public Rust APIs deliberate and avoid exposing implementation internals by
  default.
- Point dependencies inward rather than forcing the library to depend on hosting
  concerns.
- Avoid speculative abstractions.

## Current state

The reusable `windwatcher` crate contains UUIDv7 identifiers, the user identity
model, conversation and membership application services, local identity
application services, session lifecycle use cases, and core text-message and
read-state use cases, and reusable pagination infrastructure with message history.
Registration, authentication, and user state depend on `IdentityStore` and
`PasswordHasher`; session creation, refresh, and revocation depend on
`SessionStore`, `RefreshTokenProvider`, and `AccessTokenCodec`. The optional
HS256 JWT adapter implements the access-token port, but JWT is not required by
embedding applications. The optional `sqlite`, `postgres`, `mysql`, and
`mongodb` features provide official persistence adapters; `MongoDbStorage`
implements all six persistence ports and requires a transaction-capable replica
set. Primary persistence remains independent of queue selection. SQLx, MongoDB's
driver, and their Tokio requirements stay at the infrastructure boundary.

The reusable crate also owns the portable durable-job envelope, enqueue use case
and `QueueStore` port. Optional `queue-sqlite`, `queue-postgres`, `queue-mysql` and
`queue-redis` features provide concrete adapters. SQL queues have independent,
explicit migrations; Redis uses atomic Lua transitions with no schema migration
lifecycle. The official server compiles all four and selects a provider at runtime;
library consumers compile only their requested adapters. Adapter mechanics differ,
but observable queue semantics do not:

```text
QueueStore
├── SQLite
├── PostgreSQL
├── MySQL
└── Redis
```

The official server
composes the optional queue only for worker, job-operator and queue-migration
commands; queue schema management is not part of `QueueStore`.

Message application operations depend on `MessageStore`, implemented by the
optional SQLite adapter:

```text
Message use cases → MessageStore → SQLite adapter
```

`MessageContent` defines exact portable text (only NUL is rejected), while
`MessagePolicy` controls UTF-8 byte acceptance for new sends/edits. `Message`
defines active/tombstone structural state independently of current size policy.
The application generates UUIDv7 ordering keys; persistence atomically enforces
membership, reply availability, edit/delete permissions and monotonic read
markers. Membership owns read state and unread counts are derived. The server
resolves its message configuration to the reusable library policy and delegates
the message port through concrete storage enum dispatch. See
[messages](messages.md) and [ADR 0008](adr/0008-message-lifecycle-and-read-state.md).
That message delivery did not introduce realtime, events or workers.

`PageRequest`, `PageLimit` and `Page<T>` are shared, transport-independent
application pagination infrastructure. `ListMessages` is the first consumer:

```text
ListMessages → decode/validate opaque cursor → MessageStore keyset query
            → encode next cursor → Page<MessageView> (newest first)
```

Only the application knows cursor encoding. The persistence port receives a
typed `MessageId` boundary and validated limit, and returns a message slice plus
lookahead information. Ordering remains resource-specific, while representation
is reusable. Authorization is independent of cursor possession and shares the
storage read snapshot. See [pagination](pagination.md) and
[ADR 0009](adr/0009-opaque-keyset-pagination.md).

Message operations return application read projections containing sender username
and a shallow current reply preview. SQLite joins attribution and reply state in
the same snapshot/transaction as the message operation; `Message` itself remains
unchanged and transport-independent.

`MessagingQueryStore` supplies reusable consumer queries: exact enabled username
discovery, actor inbox/detail summaries and public member lists. Projections are
application models rather than enriched domain entities. SQLite batches inbox
hydration within one read snapshot; cursors remain application-owned and typed
boundaries remain independent of transport and storage encoding.

Ports return native Rust `Send` futures and impose no executor. The initial
Argon2id adapter performs CPU-bound work on the polling thread; hosts can wrap
the port with runtime-specific blocking-pool execution. See
[local authentication](authentication.md) and
[ADR 0003](adr/0003-local-credentials-and-authentication.md).

## HTTP adapter

The first public transport belongs entirely to `windwatcher-server`:

```text
HTTP adapter (Axum / Tower, server-owned DTOs and Problem Details)
    ↓
windwatcher application use cases (including GetUser)
    ↓
ports
    ↓
server Storage dispatch → SQLite adapter
```

Handlers consume concrete resolved state; they do not parse config, resolve
secrets or execute SQL. Resources are organized by API version then resource,
with explicit handler modules. Utoipa-aware route composition generates the
OpenAPI 3.1 contract and Swagger uses vendored assets. A server-package library
target exposes deliberate composition entry points, while the executable retains
its operator CLI. Private server runtime composition adds operational routes,
binds the listener with peer ConnectInfo and owns bounded graceful shutdown;
the public router remains constructable without process lifecycle.

The messaging surface includes authenticated discovery, inbox/detail/member
queries, Group lifecycle, enriched message history/global items and own read-state
updates. `HttpState` consumes resolved `MessagePolicy`. `CreateCommandStore`
atomically joins primary create effects and hashed-key command records; SQLite
shares transaction-local primitives with ordinary conversation/message storage.
Replay is current authorized state, not cached transport bytes. See
[ADR 0011](adr/0011-idempotent-create-commands.md).

HTTP status codes, headers, framework types and error representations never enter
the reusable engine. Its Argon2 adapter remains runtime-independent; the server's
bounded blocking wrapper handles HTTP hashing/verification. See [HTTP API](http-api.md)
and [ADR 0010](adr/0010-http-v1-transport-and-contracts.md).

## Realtime adapter

WebSocket transport also belongs entirely to `windwatcher-server`. A shared
`RealtimeService` holds resolved policy, in-memory hashed tickets, a multi-socket
connection hub and the notifier. Application mutations return `Outcome<T>` with
committed library `DomainEvent` facts; HTTP handlers hand those facts to the
server reaction boundary without inferring mutations from transport outcomes,
and without awaiting notification hydration or dispatch. A bounded number of tasks use a
whole-job deadline independent of heartbeat; saturation drops work rather than
creating unbounded permit waiters. The hub accepts
explicit user audiences; it does not authorize mutations or read the database.
The notifier resolves current membership and constructs stable state-oriented
events. Session validation, member-ID lookup and committed mutation timestamps
are reusable engine capabilities. No transport type or runtime dependency enters
the library. Group outcomes preserve their operation's exact persisted metadata
timestamp so asynchronous notifications do not borrow a later mutation's value.

The hub is process-local, with guarded pending/active registration, bounded queues,
separate close signals, native heartbeat, session deadlines and persistence-backed
revocation rechecks. Local logout coordinates primary revocation and realtime cleanup
against handshake activation. Slow sockets cannot block HTTP or healthy sockets.
Database plus HTTP remain authoritative; sockets offer no replay, ACK, durable
snapshot boundary or domain ordering. Clients reconcile through HTTP after reconnect.

`RealtimeEvent` is not `DomainEvent`. The server maps library facts to the wire
protocol without equating internal names or payloads. The standalone `serve`
process runs HTTP/realtime with primary storage only. Operational liveness is
dependency-free; readiness inspects primary schema with a timeout and fails
immediately when draining. One cancellation token triggers HTTP shutdown;
WebSocket and dispatch tasks are explicitly tracked and drain concurrently under
one total deadline. Doctor inspects the full installation without mutations or
listener binding. See [operations](operations.md) and [ADR 0016](adr/0016-standalone-runtime-lifecycle.md).
The standalone `worker` process is implemented separately and consumes `QueueStore`;
it does not start HTTP and currently has no official production handlers.
`DomainEvent` does not automatically enqueue jobs. Primary storage and queue
storage are operationally independent and are not transactionally coupled:

```text
                    windwatcher-server
                           │
             ┌─────────────┴─────────────┐
             │                           │
       primary application          worker process
             │                           │
        DomainEvent[]                 QueueStore
             │                           │
        realtime reaction       configured queue → queue backend
             │
       SqliteStorage → windwatcher.db
```

Queue delivery is at-least-once with no processing-order guarantee. Renewable
leases use `LeaseId` fencing; worker concurrency is a number of execution slots
per process. There is no outbox, automatic event-to-job mapping, cancellation,
priority or deduplication in v1. See [realtime](realtime.md), [jobs](jobs.md),
[ADR 0012](adr/0012-websocket-realtime-transport.md), and
[ADR 0014](adr/0014-durable-job-queue-semantics.md).
