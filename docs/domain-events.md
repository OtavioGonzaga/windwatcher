# Domain events

Successful mutating application operations return `Outcome<T>`. Its value is the
operation-specific result and its `DomainEvent` list records meaningful facts
committed by that operation. No-op mutations, semantic Direct reuse and
idempotency replays return no events.

Events are typed, in-process Rust values owned by the `windwatcher` library.
They are produced after primary persistence succeeds. They are not serialized,
persisted, replayable, audit records, realtime payloads or queue jobs.

The standalone server reacts to these events after the operation completes and
maps them to its unchanged WebSocket contract. Reactions are best effort; a
reaction failure does not undo a committed operation. A future durable outbox or
job representation would be separate from this model.
