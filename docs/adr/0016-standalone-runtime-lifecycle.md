# 0016: Standalone runtime lifecycle

- Status: Accepted
- Date: 2026-10-29

## Context

The standalone application needs an executable service lifecycle without moving
process concerns into the reusable engine or coupling independent deployments.

## Decision

Plain `windwatcher` defaults to the same path as `windwatcher serve`. The serve
process runs HTTP and realtime using primary storage only; it never resolves or
connects the optional queue. Worker remains a separate queue-only process.

Migrations are explicit. Normal commands open existing SQLite persistence without
creating files or data directories. Serve verifies primary schema compatibility
before binding its numeric TCP address, which defaults to loopback.

Unversioned, public operational health routes bypass API CORS and quotas while
retaining request tracing. Liveness has no dependency probe. Readiness requires
current reachable primary storage, a bounded probe, and a non-draining runtime;
it never inspects queue or worker state. Health responses disclose no diagnostics.

Both process roles share OS signal handling. Serve broadcasts cancellation and
uses one deadline for concurrent HTTP, WebSocket and dispatch drains. Realtime
admission closes under its existing lifecycle synchronization; tracked upgrades
and connections receive 1001 `server shutdown`. Dispatch remains bounded and
best-effort. Grace expiry after an operator signal is a successful shutdown.
Worker lease renewal and normal lease expiration remain unchanged.

Doctor validates the complete configured installation and independently inspects
storage and configured queue connectivity and schemas. It is read-only, does not
bind the listener, treats absent queue and Redis's absent migrations as healthy,
and reports safe diagnostics with nonzero exit status if unhealthy.

## Consequences

Each runtime starts only with its actual dependencies. Doctor can diagnose both
roles without becoming part of either service's readiness. Explicit task tracking
is necessary because upgraded WebSockets outlive Axum's HTTP connection tracking.
The bounded drain may discard remaining best-effort work. TLS, process packaging,
distributed health and configuration reload remain outside this decision.
