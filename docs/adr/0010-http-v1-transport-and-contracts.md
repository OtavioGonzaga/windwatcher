# 0010: HTTP v1 transport and contracts

- Status: Accepted
- Date: 2026-10-26

## Context

The first HTTP delivery must establish transport contracts without coupling the
embeddable engine to a framework or prematurely implementing the serving runtime.
Existing identity/session use cases and storage already provide the behavior.

## Decision

Own HTTP entirely in `windwatcher-server`, using Axum, Tower and tower-http.
Organize version then resource, with explicit handlers and transport DTOs for
each resource. A concrete state consumes resolved adapters and policies. A small
server-package library target makes router composition available without a fake
CLI command, listener, broad dead-code suppression or a second reusable domain.

Expose only registration, session creation/refresh/current revocation and current
identity. `GetUser` is the reusable identity lookup, without authorization.
The existing SQLite session INSERT additionally checks the persisted owner's
enabled state atomically, preventing a real disable-between-auth-and-create race
without changing the port API or adding a migration.

Use JSON bearer transport without cookies. Verify access claims without a storage
lookup on every request; revocation blocks refresh, not already-issued JWTs.
Keep enumeration-resistant login and collapsed refresh failures. Offload the
existing Argon2 adapter through bounded Tokio blocking workers, retaining worker
permits through cancellation and zeroizing the necessary owned secret copy.

Centralize RFC 9457 problems, stable codes/type URNs, explicit transport/semantic
status mappings and server-owned UUIDv7 correlation IDs. Trace safe metadata only.
Generate OpenAPI 3.1 from Utoipa-aware route registration, with stable operation
IDs, explicit security and schemas. Serve versioned JSON metadata and vendored
Swagger assets; do not maintain a separate path list or committed generated spec.

Apply deny-by-default configurable non-credentialed CORS to the API. Use governor
directly behind a thin middleware to retain Windwatcher's 429 problem contract
and Retry-After. Per-process immediate-peer buckets enforce general plus stricter
authentication quotas. Do not trust forwarded addresses. Preflights and docs do
not consume authentication quotas. Resolve and validate HTTP configuration before
router construction, including through `config check`.

## Consequences

The reusable crate gains no transport dependencies. Router-level tests exercise
real SQLite/application behavior without TCP. The later runtime must supply
connect-info and implement actual lifecycle/health behavior. Reverse-proxy users
must adjust or disable peer-based quotas until trusted proxies are supported.
The limiter is neither distributed nor persistent. Access revocation remains
bounded by token lifetime; this is documented rather than hidden by per-request
session lookups. Cookies, idempotency, discovery and messaging HTTP remain absent.
