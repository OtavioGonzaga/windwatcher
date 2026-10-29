# Standalone operations

## Quick start

Build the executable with the repository toolchain:

```bash
cargo build -p windwatcher-server --bin windwatcher
```

Use `target/debug/windwatcher` below, or install it on your PATH. Initialize a
minimal configuration and stable private JWT key:

```bash
windwatcher init
```

The generated secret is kept across restarts. `--config PATH init` selects an
explicit new configuration location. Existing files are never overwritten. See
[server configuration](server-configuration.md) for alternative secret sources.
Then:

```bash
windwatcher config check
windwatcher config show
windwatcher migrate
windwatcher user create --username alice
windwatcher doctor
windwatcher serve
```

Plain `windwatcher` is exactly the same startup path as `windwatcher serve`.
The default listener is `127.0.0.1:8080`. OpenAPI is available at
`/openapi/v1.json` and the local documentation UI at `/docs/`.

Serve connects primary storage and verifies its migration history **before**
binding. Missing or outdated persistence fails startup; run
`windwatcher migrate --storage` for pending schema. Serve never migrates, creates
an empty SQLite database or creates its data directory. Worker, doctor, jobs,
user provisioning and migration status also use existing-only connections.

Serve does not resolve queue semantics, connect queue, run job handlers or queue
maintenance, even when `[queue]` exists. Worker is a separate queue-only process;
it currently needs neither primary storage nor authentication configuration.
Configuration syntax/deserialization failures invalidate every command.

## Listener and deployment

```toml
[server]
bind = "127.0.0.1:8080"
shutdown_grace_period = "30s"
readiness_timeout = "2s"
```

Only numeric IP/port addresses are supported, including IPv6 such as `[::1]:8080`.
Port zero selects an ephemeral port and logs the actual bound address. Containers
or reverse proxies can explicitly select `0.0.0.0:8080` or `[::]:8080`; defaults
never expose the service externally. TLS termination is external in v1. No
hostname resolution, TLS listener, Unix sockets or service packaging is supplied.
Client-IP quotas use the immediate TCP peer by default. Explicit trusted proxy
CIDRs enable bounded `X-Forwarded-For` resolution; see
[HTTP configuration](server-configuration.md#http-behavior). Without opt-in,
proxy clients share the peer bucket.

## Health

Both routes are public, unversioned, JSON, `Cache-Control: no-store`, excluded from
client OpenAPI, and bypass API CORS and rate limits. They retain request IDs and
structured request traces.

| Endpoint | Healthy | Unhealthy |
| --- | --- | --- |
| `GET /health/live` | `200 {"status":"ok"}` | No external dependency check |
| `GET /health/ready` | `200 {"status":"ready"}` | `503 {"status":"not_ready"}` |

Liveness means the process can answer HTTP, not that its dependencies work.
Readiness requires a non-draining runtime and reachable primary storage with a
current schema. Each inspection uses the already-open storage client and is
bounded by `server.readiness_timeout`; backend connection/pool timeouts still
apply independently. A storage outage fails readiness but not liveness.
Readiness never checks queue, worker discovery or job consumption.
Health output never includes backend URLs, paths, topology or driver errors.

## Doctor

```bash
windwatcher doctor
```

Doctor first validates complete configuration, including optional queue policy
and MongoDB URI syntax. If valid, it checks storage connectivity/schema and
configured queue connectivity/schema independently. Absent queue is
`[ok] queue: disabled`; Redis migrations are healthy/not applicable. SQL queue
migrations remain required.

```text
[ok] configuration: valid
[ok] storage: SQLite
[ok] storage migrations: current
[ok] queue: disabled
Doctor: healthy
```

Any required failure produces a nonzero exit and `Doctor: unhealthy`. Pending
schemas include `migrate --storage` or `migrate --queue` hints. Invalid configuration
prevents dependency checks; its fixed report does not quote potentially sensitive
input. Doctor never binds the configured listener, so it can run while serve is
active. It never migrates, creates missing SQLite persistence, repairs indexes,
initializes replica sets, enqueues/retries jobs or discovers worker processes.
It has deterministic human-readable output, not a machine-readable protocol.

`config check` differs: it performs static validation, secret-file resolution
and syntax-only URI parsing, but no dependency connections or listener binding.

## Graceful shutdown

SIGINT and SIGTERM on Unix (Ctrl-C elsewhere) use the same signal abstraction for
serve and worker. Configuration remains immutable; SIGHUP reload is not supported.

Serve immediately becomes not ready, triggers Axum graceful HTTP shutdown and
closes realtime admission. Already accepted requests can finish. Unused tickets
are invalidated, new ticket/preparation requests receive
`503 realtime.shutting_down`, and active WebSockets receive close code `1001`,
reason `server shutdown`. Connection callbacks and admitted best-effort dispatch
jobs are explicitly tracked; new dispatch submissions are rejected.

HTTP, WebSocket and dispatch drains run concurrently within **one total**
`server.shutdown_grace_period`, including lifecycle coordination. Expiry logs
`shutdown_grace_expired` and finishes the operator-requested shutdown successfully;
remaining tasks are cancelled when the process runtime exits. Unexpected server
termination or signal-registration failure is an error instead.

Worker drains independently under `[queue].shutdown_grace_period`, stops claims,
continues active lease renewal/completion and leaves unfinished leases to expire
normally on timeout. No proactive lease release is introduced. Supervisors may
enforce their own outer deadline; no second-signal escalation is implemented.

## Network composition tests

Use only disposable PostgreSQL, MySQL 8, MongoDB replica set and Redis services.
Set `WINDWATCHER_TEST_POSTGRES_URL`, `WINDWATCHER_TEST_MYSQL_URL`,
`WINDWATCHER_TEST_MONGODB_URL` and `WINDWATCHER_TEST_REDIS_URL`, then run:

```bash
cargo test -p windwatcher-server --test cli -- --ignored
```

SQL tests create/drop UUID-named databases. MongoDB uses UUID-named databases
inside the disposable replica set; discard that service after testing. Redis
uses isolated test prefixes without flushing shared databases. These tests check
server/doctor composition, not a duplicate full persistence contract matrix.
Ordinary workspace tests need no external services and exercise real SQLite,
TCP listeners, WebSockets and deterministic injected shutdown futures.
