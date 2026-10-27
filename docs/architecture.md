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
embedding applications. The optional `sqlite` feature provides the first
official persistence adapter: `SqliteStorage` implements identity, session, and
conversation and message storage ports
while keeping SQLx and its Tokio runtime requirement at the infrastructure
boundary.

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
its existing operator CLI. There is no production listener or `serve` yet.

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
connection hub and the notifier. HTTP mutation handlers preserve committed
change/no-op outcomes and submit best-effort publication explicitly, without
awaiting notification hydration or dispatch. A bounded number of tasks use a
whole-job deadline independent of heartbeat; saturation drops work rather than
creating unbounded permit waiters. The hub accepts
explicit user audiences; it does not authorize mutations or read the database.
The notifier resolves current membership and constructs stable state-oriented
events. Session validation and member-ID lookup are the only new reusable engine
capabilities. No transport type or runtime dependency enters the library.

The hub is process-local, with guarded pending/active registration, bounded queues,
separate close signals, native heartbeat, session deadlines and persistence-backed
revocation rechecks. Local logout coordinates primary revocation and realtime cleanup
against handshake activation. Slow sockets cannot block HTTP or healthy sockets.
Database plus HTTP remain authoritative; sockets offer no replay, ACK, durable
snapshot boundary or domain ordering. Clients reconcile through HTTP after reconnect.

`RealtimeEvent` is not `DomainEvent`. A future server-side mapper/reaction may
replace explicit handler hooks without equating internal domain payloads with the
wire protocol. There is still no production listener, `serve`, health/readiness or
process shutdown lifecycle. See [realtime](realtime.md) and
[ADR 0012](adr/0012-websocket-realtime-transport.md).
