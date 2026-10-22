# 0002: UUIDv7 identifiers

- Status: Accepted
- Date: 2026-10-22

## Context

Windwatcher needs identifiers that can be generated without database
coordination, remain globally unique for practical purposes, work consistently
across persistence backends, and provide useful temporal ordering
characteristics. Identifiers for separate domain concepts must not be
accidentally interchangeable.

## Decision

Use UUIDv7 for Windwatcher domain identifiers. Model semantic concepts through
distinct Rust newtypes: `UserId`, `SessionId`, `ConversationId`, and
`MessageId`.

Generation produces UUIDv7 values, and construction or parsing of existing
values validates the UUID version. The underlying UUID remains intentionally
available through explicit APIs; it is part of the public Rust contract rather
than a hidden implementation detail.

## Consequences

Positive consequences include:

- IDs can be generated before persistence.
- Adapters do not need database-generated integer IDs.
- Identifiers work across SQL and document databases.
- Temporal ordering is available where useful.
- Rust type safety prevents accidental interchange between domain IDs.

Trade-offs include:

- UUID is part of the public Rust contract.
- External UUID input must be validated.
- Code relying on UUIDv7 ordering must preserve the version invariant.
