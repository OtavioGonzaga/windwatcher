# 0005: SQL storage and SQLite adapter

- Status: Accepted
- Date: 2026-10-23

## Context

Windwatcher needs interchangeable persistence backends while retaining strong
transactional semantics, explicit schema evolution, and domain types that do
not leak ORM or driver models. The initial identity and session contracts need a
real database implementation, including atomic registration and refresh-token
compare-and-swap rotation.

## Decision

Use SQLx, rather than an ORM, for concrete SQL adapters. Implement SQLite as
the first adapter behind the optional `sqlite` feature. The library's ports
remain runtime-independent; the adapter may use SQLx's Tokio integration.

SQLite migrations are explicit, embedded, forward-only SQL files. SQLite stores
UUIDs as BLOB(16), refresh fingerprints as BLOB(32), and timestamps as Unix
microseconds. Each official database adapter will be concrete rather than using
SQLx `AnyPool`, and must satisfy reusable application-level storage contracts.

## Consequences

This provides direct SQL control, transparent schemas, database-native atomic
operations, and no ORM models in the domain. It also requires handwritten SQL,
explicit mapping and corruption handling, database-specific migrations, and
enables SQLx/Tokio only for consumers selecting the adapter feature.
