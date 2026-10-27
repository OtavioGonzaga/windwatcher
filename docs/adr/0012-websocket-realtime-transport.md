# 0012: WebSocket realtime transport

- Status: Accepted
- Date: 2026-10-27

## Context

HTTP clients need low-latency messaging notifications and synchronization across
devices. The library already owns committed state, projections, idempotency and
mutation/no-op outcomes. It must not acquire an HTTP/WebSocket runtime or a domain
event framework merely to notify connected clients.

## Decision

Keep database plus HTTP authoritative. Add WebSocket entirely in
`windwatcher-server`, with explicit post-commit handler publication of successful
real mutations. No-ops and idempotency replays do not publish. Notification failure
is logged and swallowed, never reported as rollback of a committed primary action.
Submission reserves concurrency without waiting and spawns only after obtaining
capacity. The entire notification job has a dispatch-specific deadline; neither
hydration nor audience lookup delays the authoritative HTTP response. Saturation
drops new work instead of creating unbounded tasks or a persistent queue.

Use one-use, short-lived 256-bit random tickets issued through authenticated HTTP;
never put access JWTs in URLs. Keep only ticket SHA-256 hashes and grants in memory.
Validate session ownership, revocation and expiration at issuance and connection
preparation. Support multiple sockets per user with per-session association.
Local logout coordinates commit/invalidation with handshake activation, cancels
pending registrations and closes only the relevant session's sockets. Absolute
session deadlines and periodic persistence revalidation close inactive sockets;
storage validation failure fails closed. Access-token expiry alone does not close
an established socket.

Use a process-local hub with guarded registration, explicit user audiences,
bounded per-connection event queues and a separate close channel. Publication
never waits for socket writes. Serialize each event once, share it across fan-out,
disconnect slow consumers, and use native Ping/Pong heartbeat. Clients send commands
through HTTP, not application WebSocket frames. Do not add production listener or
process lifecycle behavior in this delivery.

The stable state-oriented envelope contains only `event_id`, `type`,
`schema_version`, `occurred_at` and `payload`. UUIDv7 emission IDs are server-local
types, not ordering keys or reusable engine IDs. Message payloads match HTTP
projections. Conversation creation is deliberately actor-independent and small.
Ownership transfer is one atomic notification. Removed members receive their
terminal removal event. Read-state events go only to the same user's devices;
they are not read receipts. Normal audiences use current membership at lookup,
favoring authorization over historical audience reconstruction.

There is no durable replay, event table, outbox, ACK, cursor, resume mechanism,
exactly-once delivery, or domain ordering guarantee. Clients tolerate duplicate and
unknown notifications, and reconcile through HTTP after every connection/reconnect.
HTTP wins when race/order ambiguity prevents a safe merge.

`RealtimeEvent` is distinct from `DomainEvent`. Future domain-event publication
may feed a server mapper/reaction which builds the same realtime wire contract;
internal event names/payloads need not match public realtime names/payloads.

## Consequences

No migration is required. The engine gains only session lookup/validation and a
current conversation-member-ID query. Tickets/connections are not public library
ports. This design intentionally loses notifications across disconnection, process
exit and cancellation after primary commit. Attribution hydration or audience
lookup can fail independently of a successful mutation.

Ticket issuance and upgrade must reach the same process. A multi-process deployment
needs a later shared signaling mechanism, not reuse of a durable queue contract.
There is no speculative Redis adapter or connection-count policy. Contract tests
cover JSON/HTTP parity, no-ops, fan-out/backpressure and pending registration;
temporary Axum listeners cover real handshakes, frames, heartbeat and revocation.
