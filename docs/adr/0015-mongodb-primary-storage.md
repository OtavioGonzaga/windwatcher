# ADR 0015: MongoDB primary storage

- Status: Accepted
- Date: 2026-10-29

## Context

Windwatcher exposes six primary-storage ports and already offers relational
adapters. MongoDB can support these semantics only when transactions are
available; standalone deployments and `mongos` cannot provide the required
atomic cross-collection updates and consistent read snapshots.

## Decision

Add MongoDB as an optional `windwatcher` persistence feature and an independent
server `storage.provider`. Require a replica set and reject unsupported topology
during connection. Keep migrations explicit. Store identifiers as BSON UUID
binary subtype 4 and timestamps as signed Unix microseconds. Perform multi-record
mutations in transactions, using snapshot reads and majority writes.

The MongoDB adapter is primary storage only. Queue selection and queue
transactions remain independent; this decision does not add a MongoDB queue.

## Consequences

Applications opt in through the `mongodb` feature and provide a replica-set URL
plus explicit database name. Operators must provision a transaction-capable
replica set and run the normal explicit storage migration command. Adapter
behavior is checked against the shared storage contracts and real replica-set
integration tests.
