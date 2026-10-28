# 0013: Explicit domain event outcomes

- Status: Accepted
- Date: 2026-10-28

## Decision

Mutating library use cases return `Outcome<T>`, containing their ordinary result
and typed `DomainEvent` facts created only after successful persistence. Events
are library-owned, non-serialized in-process values. No-ops, semantic reuse and
idempotency replays emit none. Storage adapters remain unaware of events.
`Outcome<T>` is an explicit wrapper, not transparently dereferenceable to its
value, so callers deliberately handle the result and its facts.

The server maps these facts to its separate serialized `RealtimeEvent` contract
through one reaction boundary. Reactions remain best effort: a primary commit can
succeed even if process exit or dispatch failure loses its reaction.

This introduces no dispatcher in the engine, event bus, event storage, outbox,
queue, audit log, integration-event API, replay facility or migration. A later
queue or durable outbox must define its own selected-job representation rather
than serialize `DomainEvent` by default.
