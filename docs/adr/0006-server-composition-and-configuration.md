# 0006: Server composition and configuration

- Status: Accepted
- Date: 2026-10-23

## Context

The official executable needs deterministic operator configuration, storage
composition, and observability without putting runtime concerns in the reusable
library.

## Decision

`windwatcher-server` is the composition root. It layers defaults, one TOML file,
`WINDWATCHER__` environment values, and exposed CLI overrides; unknown fields are
rejected and `.env` is not auto-loaded. User mode is default and system mode is
explicit. SQLite is inferred as the default/configured adapter and is composed by
concrete enum dispatch. Resolution is command-specific: migrations resolve storage
only, while `config check` validates the complete future runtime configuration.

JWT secrets are direct values or exact file bytes, never both. The server owns a
stderr tracing subscriber while command results use stdout. This delivery contains
operator commands only and deliberately does not add `serve`.

## Consequences

The library remains framework- and configuration-independent. Operators can run
migrations during unrelated authentication-configuration incidents, and future
server runtime work can reuse the same paths, secrets, logging, and storage wiring.
