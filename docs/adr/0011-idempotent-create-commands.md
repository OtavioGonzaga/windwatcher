# 0011: Persisted idempotent create commands

- Status: Accepted
- Date: 2026-10-26

## Context

Client retries can duplicate Group creation or message sends. Semantic Direct
uniqueness alone does not provide a reusable client command identity. Deduplication
must survive process restarts and commit atomically with the authoritative effect.

## Decision

Accept optional exact `IdempotencyKey` values of 1..128 visible ASCII bytes, without
trimming. Persist SHA-256 key hashes, never raw client strings. Scope records by
actor, versioned operation and resource scope (actor for conversation creation;
conversation for message send). Use versioned semantic fingerprints, not raw JSON.
Group fingerprints remove the creator, deduplicate and sort initial participants.

`CreateCommandStore` defines atomic create/replay behavior. SQLite shares its
existing transaction-local conversation/message creation primitives with these
commands: resource and record commit together under `BEGIN IMMEDIATE`. Only
successful commands reserve keys. Existing Directs can establish a new mapping
without being classified as replayed.

Records are active while `expires_at > now`, with 24-hour retention. Matching
commands return the original identity and current authorized representation, not
historical response bytes. Changed input conflicts. Physically deleted results
remain unavailable until expiry instead of silently creating replacements.
Replay does not bypass current membership authorization.

SQLite opportunistically deletes expired records within keyed create transactions
using an expiration index; lookup independently enforces expiration. Resource IDs
have no generic foreign key, allowing records to outlive physical resource removal.
There is no queue, worker, in-memory-only cache or domain entity key field.

## Consequences

Official future backends must provide equivalent atomicity and expiration semantics
using their own transactions/TTL mechanisms. Cleanup is adapter housekeeping, not
a production runtime dependency. Retries may return changed/tombstoned current
state. This delivery adds no Redis implementation or retention configuration knob.
