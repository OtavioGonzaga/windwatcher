# ADR 0015: MongoDB primary storage

- Status: Accepted
- Date: 2026-10-29

## Context

Windwatcher exposes six primary-storage ports and already offers relational
adapters. These semantics require transactions, so standalone MongoDB deployments
are unsuitable. MongoDB supports multi-document transactions and cross-shard
snapshot reads on sharded clusters. Supporting those deployments also requires
additional operational validation beyond the scope of this first adapter.

## Decision

Add MongoDB as an optional `windwatcher` persistence feature and an independent
server `storage.provider`. Require a replica set and reject unsupported topology
during connection. Keep migrations explicit. Store identifiers as BSON UUID
binary subtype 4 and timestamps as signed Unix microseconds. Perform multi-record
mutations in transactions, using snapshot reads and majority writes.
Sharded clusters (`mongos`) are deliberately unsupported in this version, not
inherently incapable of providing these guarantees.

Membership/role changes and message mutations write the same private conversation
revision before authorizing. Snapshot reads alone do not serialize authorization
against removal or demotion; write conflicts retry against a new snapshot.

Each migration executes its own ordered commands before recording its version.
Checksums cover those executable command files, and a unique version index
protects the ledger against concurrent migrators. DDL steps are restartable
because MongoDB DDL cannot be included in these application transactions.

The MongoDB adapter is primary storage only. Queue selection and queue
transactions remain independent; this decision does not add a MongoDB queue.

## Consequences

Applications opt in through the `mongodb` feature and provide a replica-set URL
plus explicit database name. Operators must provision a transaction-capable
replica set and run the normal explicit storage migration command. Adapter
behavior is checked against the shared storage contracts and real replica-set
integration tests.
