# 0013: Explicit domain event outcomes

- Status: Accepted
- Date: 2026-10-28

## Decision

Mutating library use cases return `Outcome<T>`, containing their ordinary result
and typed `DomainEvent` facts created only after successful persistence. Events
are library-owned, non-serialized in-process values. No-ops, semantic reuse and
idempotency replays emit none.

The server maps these facts to its separate `RealtimeEvent` contract through one
reaction boundary. Reactions remain best effort. This introduces no event bus,
event storage, outbox, queue, audit log, replay facility or migration.
