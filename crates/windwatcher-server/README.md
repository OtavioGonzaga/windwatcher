# Windwatcher Server

The official self-hosted HTTP and WebSocket service built on
[`windwatcher`](https://crates.io/crates/windwatcher), the Rust messaging engine.
Use it to provide a messaging backend for your own clients without embedding the
engine in a Rust application.

**Early development; not production-ready.** Contracts may change between 0.x
releases. Building from source requires Rust 1.99 or later (edition 2024).
Licensed under MIT.

The Cargo package is **`windwatcher-server`**; the installed executable is
**`windwatcher`**. The server and engine are independently versioned.

## What it provides

- A versioned JSON API for local users, authentication, sessions, direct
  conversations, groups and text messages.
- Group roles and membership management; message replies, edits and deletion;
  paginated inbox/history; unread counts and private per-user read state.
- Optional `Idempotency-Key` support for conversation creation and message sends.
- Ticket-authenticated WebSocket notifications for connected clients.
- OpenAPI 3.1 and a bundled Swagger UI, structured Problem Details errors,
  configurable CORS and client-IP rate limits.
- Explicit database migrations, user provisioning, configuration checks,
  installation diagnostics, health endpoints and graceful shutdown.

This is a backend service, not a bundled chat frontend. For direct Rust
integration without the server stack, choose the `windwatcher` library instead.

## Install and run locally

```bash
cargo install windwatcher-server --locked
windwatcher init
windwatcher config check
windwatcher migrate
windwatcher user create --username alice
windwatcher doctor
windwatcher serve
```

`init` creates a minimal configuration and a stable random JWT signing key without
overwriting existing files. `user create` prompts for a password; no public
registration is needed for this setup. The default primary database is local
SQLite, so no external database or queue service is needed.

The server listens on **`127.0.0.1:8080`** by default. Plain `windwatcher` is
equivalent to `windwatcher serve`. Once running:

| URL | Purpose |
| --- | --- |
| `http://127.0.0.1:8080/docs/` | Interactive API documentation |
| `http://127.0.0.1:8080/openapi/v1.json` | OpenAPI contract |
| `http://127.0.0.1:8080/health/live` | Process liveness |
| `http://127.0.0.1:8080/health/ready` | Primary-storage/schema readiness |

Use Swagger UI to log in through `POST /v1/sessions`, then authorize with the
returned access token to discover users, create conversations and send messages.
Clients write through HTTP; WebSockets carry best-effort notifications, not
commands. See the [client messaging journey](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/http-api.md#client-messaging-journey).

## Configuration and storage

Configuration layers built-in defaults, one TOML file, `WINDWATCHER__...`
environment variables and the exposed CLI logging overrides. Unknown fields are
rejected; `.env` files are not loaded automatically.

- Default user mode uses platform configuration/data directories. On Linux these
  are normally `~/.config/windwatcher/` and `~/.local/share/windwatcher/`.
- `--config PATH` selects an explicit configuration file; `--system` selects
  Linux system paths under `/etc/windwatcher/` and `/var/lib/windwatcher/`.
- Primary storage supports SQLite (default), PostgreSQL, MySQL and MongoDB.
  Network SQL databases must already exist; MongoDB requires a transaction-capable
  replica set, not a standalone server or sharded cluster.
- Migrations are explicit: `serve` checks the primary schema and never migrates
  automatically. Back up persistent data before upgrades.

For example, set `WINDWATCHER__SERVER__BIND=0.0.0.0:8080` to listen on all IPv4
interfaces. Before exposing the service, configure external TLS termination,
appropriate browser origins and trusted proxy CIDRs if applicable. Preserve the
signing key across restarts and keep it private.

See the [configuration guide](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/server-configuration.md)
and [example TOML](https://github.com/OtavioGonzaga/windwatcher/blob/main/config.example.toml).

## Deployment options

`cargo install` installs only the executable, not service accounts or systemd
units. Linux DEB/RPM packages and binary tarballs are distributed through
[GitHub Releases](https://github.com/OtavioGonzaga/windwatcher/releases); choose
a `windwatcher-server-vX.Y.Z` release, not an engine release.
The official OCI image is `ghcr.io/otaviogonzaga/windwatcher-server:VERSION`;
replace `VERSION` with a published server version.

Follow the [Linux installation guide](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/installation.md)
or [container deployment guide](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/container.md)
for initialization, persistence and service setup.

## Current boundaries

- Realtime tickets and connections are process-local, with no durable replay or
  cross-instance fan-out. Clients must reconcile through HTTP after reconnecting.
  Rate limits are also process-local, not distributed.
- Logout and user disable prevent token refresh, but already-issued HTTP access
  tokens remain valid until expiration. Authentication is local, not OAuth/OIDC.
- Attachments, reactions, threads, full-text search and read receipts are not
  implemented.
- The optional durable queue supports SQLite, PostgreSQL, MySQL and Redis,
  independently of primary storage. `windwatcher worker` is a separate process;
  `serve` does not need a queue or start workers. No production job handlers are
  shipped, and messaging mutations do not automatically enqueue jobs.

## Documentation

- [HTTP API](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/http-api.md) and [realtime protocol](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/realtime.md)
- [Operations and health](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/operations.md) · [CLI reference](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/cli.md)
- [Durable jobs](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/jobs.md) · [Security boundaries](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/security.md)
- [Rust composition API](https://docs.rs/windwatcher-server) · [Documentation index](https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/README.md)
- [Source and issues](https://github.com/OtavioGonzaga/windwatcher) · [Contributing](https://github.com/OtavioGonzaga/windwatcher/blob/main/CONTRIBUTING.md) · [Changelog](https://github.com/OtavioGonzaga/windwatcher/blob/main/crates/windwatcher-server/CHANGELOG.md) · [MIT license](https://github.com/OtavioGonzaga/windwatcher/blob/main/LICENSE)

Repository guides follow `main`; use the matching `windwatcher-server-vX.Y.Z`
Git tag when consulting documentation for a specific release.
