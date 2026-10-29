# Changelog

## Unreleased

- Validate MongoDB URL syntax during `config check` and expose safe, actionable
  unsupported-topology and invalid-migration-history diagnostics.

- Add MongoDB as an independently selected primary-storage provider.

- Report selected storage backends and invalid network migration histories
  accurately; ignore inactive SQLite storage paths when validating queue files.

- Add runtime primary-storage provider selection for SQLite, PostgreSQL, and
  MySQL, including configuration and explicit migration/status support.

All notable user-facing changes to this package are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Standalone `serve` runtime, also the default when no subcommand is supplied,
  with configurable loopback listener and storage-schema preflight before bind.
- Public unversioned liveness/readiness endpoints, independent of API quotas and
  queue availability, with bounded primary-storage/schema readiness inspection.
- Read-only `doctor` diagnostics for complete configuration, storage and optional
  queue connectivity/schema, with independent checks and credential-safe output.
- Coordinated HTTP, WebSocket and realtime-dispatch shutdown under one global
  grace deadline, with 1001 shutdown closes and shared worker OS signal handling.

- Runtime queue provider selection for SQLite, PostgreSQL, MySQL and Redis in the
  official binary, with common enqueue/worker policy and redacted connection URLs.
- Redis migration commands report "migrations not applicable"; SQL queues retain
  explicit migrations and schema checks before worker/job operations.

- Optional SQLite queue configuration and independently targeted storage/queue
  migration and migration-status commands.
- Separate `windwatcher worker` process role with bounded execution slots,
  renewable leases, retry backoff and graceful shutdown.
- `jobs list` inspection and operator retry for failed jobs.

### Fixed

- Queue configuration preserves the resolved enqueue payload and attempt policy.
- Worker task termination/panic is supervised and fails the process instead of
  silently reducing concurrency; graceful shutdown still drains active work.

### Changed

- Normal runtime, inspection and operator connections never create missing SQLite
  databases or data directories; only applying explicit migrations may create them.
- Realtime admission during drain returns `503 realtime.shutting_down`.

- Realtime publication is driven by library `DomainEvent` outcomes; the WebSocket
  wire contract is unchanged.

- Conversation and membership notification metadata timestamps come from committed
  mutation outcomes, exactly matching persistence precision rather than handler clocks.

- Ticket issuance rejects sessions that expire after lookup and never inserts an
  already-expired grant, including at the exact expiration boundary.

- Member removal/leave preserves the known terminal recipient even when current
  audience hydration fails or dispatch times out.

- Realtime notification hydration and fan-out run in bounded best-effort tasks,
  outside HTTP response latency, with independent dispatch concurrency/deadline policy.

- Bearer challenges are returned only for access-token authentication failures,
  not for invalid login or refresh credentials.

### Added

- Process-local WebSocket realtime transport at `/v1/realtime`, authenticated
  through short-lived one-use session-bound tickets from `/v1/realtime/tickets`.
- Stable versioned state-change event contracts and best-effort post-commit
  notifications for messages, conversations, membership, ownership and own read state.
- Multiple sockets per user/session, bounded fan-out queues, slow-consumer closure,
  native heartbeat, session-expiry deadlines, logout cleanup and revocation rechecks.
- Validated `[realtime]` policy, OpenAPI ticket/upgrade security and frame schemas,
  and realtime reconciliation documentation. No production listener or migration.

- Authenticated exact user discovery, consumer-ready conversation inbox/detail,
  Group membership/role/ownership lifecycle and message history/send/reply/global
  item edit/delete HTTP APIs, including authoritative own read-state updates.
- Standard paginated collection responses, normalized query/path errors,
  optional persisted `Idempotency-Key` creation and `Idempotency-Replayed` headers.
- Full messaging OpenAPI contracts, discriminated conversation creation schemas,
  and browser CORS support for PUT/PATCH/DELETE and idempotency headers.

- HTTP v1 router with public registration, local login, refresh rotation,
  idempotent current-session logout and current-user retrieval; no listener yet.
- Server-owned transport DTOs, RFC 9457 problems with stable codes, UUIDv7 request
  correlation, bearer extraction, safe structured tracing and no-store grants.
- Code-first OpenAPI 3.1 at `/openapi/v1.json` and vendored Swagger UI at `/docs`.
- OpenAPI response-header contracts for correlation IDs, no-store token grants,
  bearer challenges and retry delays, plus UUID formats for transport IDs.
- Validated configurable deny-by-default CORS and general/authentication peer-IP
  governor quotas, including `config check` and environment overrides.
- Bounded Tokio blocking adapter preserving the engine's Argon2 semantics and
  zeroization, including cancellation-safe concurrency permits.

- `[messages].max_content_bytes` configuration resolving to the library's
  new-write policy (131072 UTF-8 bytes by default), including environment
  overrides and full config validation.
- Message storage delegation and explicit migration support for message/read state.

- Server CLI foundation with layered TOML/environment configuration, user/system
  runtime paths, stderr tracing, explicit SQLite migration commands, configuration
  validation, and trusted local-user provisioning.

- Enable the library's optional HS256 JWT access-token adapter for future server wiring.
