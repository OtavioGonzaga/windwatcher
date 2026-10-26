# 0009: Opaque keyset pagination

- Status: Accepted
- Date: 2026-10-25

## Context

Growing histories need efficient traversal without offset drift. The embeddable
library needs an HTTP-independent contract that portable storage adapters can
implement while preserving resource-specific ordering. Continuation tokens must
evolve without exposing their internal representation as a consumer API.

## Decision

Use keyset pagination with next-only traversal and opaque, versioned cursors.
`PageRequest` holds an optional token and validated `PageLimit`; `Page<T>` holds
items and an optional next cursor, deriving `has_more` from cursor presence.
Limits default to 50 and have an inclusive maximum of 100; zero and oversized
requests fail instead of being clamped. No total counts or offset/page numbers
are provided.

V1 cursors use base64url without padding, are unsigned, and bind both resource
kind and resource scope. There is no cursor persistence, expiration, encryption,
revocation or embedded page size. The payload stays private and strict; a
hard-coded issued-token fixture protects v1 decoder compatibility. Unknown
versions are rejected rather than interpreted as v1.

`ListMessages` is the first consumer. Messages sort by `MessageId DESC` and
continue toward older IDs using a strict `MessageId` boundary scoped to the
conversation. The boundary need not reference a physical row. Tombstones remain
in history, and listing never marks read. Current membership is checked for every
page in the same logical read as existence and message retrieval.

The application owns cursor decoding/encoding; `MessageStore` receives a typed
boundary and validated limit, returning a bounded descending slice plus actual
`has_more`. SQLite uses a read transaction and `limit + 1`, with the existing
`(conversation_id, id)` index; no migration or writer lock is introduced. The next
cursor uses the last returned item, not the lookahead row. Other resource lists
will adopt the same `Page`/`PageRequest` contract when implemented, defining their
own resource-specific keysets rather than imposing message semantics on them.

## Consequences

Callers cannot jump to arbitrary numbered pages or request totals. Indexed
keysets avoid offset drift and scale with the requested slice. Changing page size
between requests is allowed. Unsigned tokens can be tampered with but cannot
bypass independent authorization. Format changes require explicit version
handling and compatibility decisions.

Message pagination inherits ADR 0008's UUIDv7 ordering guarantees, not globally
coordinated commit order. Multiple requests do not form one snapshot: newer IDs
stay above the continuation boundary, later lower IDs may enter subsequent
pages, deletions produce tombstones, membership can be removed, and conversation
deletion makes a later request unavailable.
