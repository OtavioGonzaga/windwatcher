# Server configuration

`windwatcher-server` is the operational composition root. Configuration is
layered in this fixed order: built-in defaults, one selected TOML file,
`WINDWATCHER__...` environment variables, then the exposed CLI logging flags.
Unknown TOML and environment fields are rejected.

## Files and modes

User mode is the default. It uses platform/XDG configuration and data directories;
on Linux these are normally `~/.config/windwatcher/config.toml` and
`~/.local/share/windwatcher/windwatcher.db`. `--system` explicitly uses
`/etc/windwatcher/config.toml` and `/var/lib/windwatcher/windwatcher.db` on Linux.
Missing mode-default configuration files are allowed. `--config PATH` selects one
file and requires it to exist; it does not change the selected mode's data path.

`config.example.toml` and `.env.example` document supported values. Windwatcher
does not load `.env` automatically. Relative explicit filesystem paths are relative
to the process working directory.

`windwatcher init` creates a minimal configuration and a 32-byte random JWT key
in the selected configuration directory, without overwriting either file. The
generated configuration references the absolute key path. `--config PATH init`
selects an explicit new config file; `--system init` selects system paths.
It never creates a database or applies migrations. Run `windwatcher migrate`
separately. `config show` displays effective loaded values with sensitive URLs and
direct secrets redacted without reading secret files or connecting dependencies;
it does not perform semantic validation.

Malformed values, including invalid duration syntax, invalidate every command.
Semantic authentication failures such as a missing JWT secret or invalid session
policy are resolved by `config check`, `serve` and `doctor`, so they do
not block migration or trusted user provisioning.

## Storage, auth, and secrets

`storage.provider` selects primary persistence independently of the queue and
defaults to `sqlite`. SQLite's `[storage.sqlite]` block optionally sets `url`,
`create_if_missing`, `max_connections`, and human-readable `busy_timeout`.
Select `postgres` or `mysql` to use `[storage.postgres]` or `[storage.mysql]`;
the selected network block requires a URL and accepts `max_connections` and
`acquire_timeout` (for example, `5s`). Network URLs must target dedicated,
pre-created databases. Keep credentials in secret management, not committed
configuration. Migrations are explicit and apply only to the selected primary
provider. Queue provider configuration remains separate; see [jobs](jobs.md).
Select `mongodb` with `[storage.mongodb]`, which requires `url` and `database`.
MongoDB primary storage requires a transaction-capable replica set; standalone
servers and `mongos` are unsupported.
`config check` parses MongoDB URL syntax without connecting or resolving SRV/TXT DNS;
topology is validated when opening storage. Safe topology and migration-ledger
errors are included in operator diagnostics; driver connection details remain
hidden so credentials cannot leak.

The durable queue is optional. Supported providers are SQLite, PostgreSQL, MySQL
and Redis; SQLite defaults to the file `queue.db`, separate from
`windwatcher.db`. Queue worker and operator commands require this section;
`config check` validates its concurrency, lease, retry and retention settings.
The queue database path must not resolve to the primary storage file. Queue
connection/migration initialization is command-specific; ordinary commands do
not open it. Serve neither resolves queue semantics nor connects it, even when
configured. Worker resolves queue only, not primary storage/auth. Syntactic
deserialization errors still invalidate the entire file. Doctor and `config
check` validate all configured components. See [jobs](jobs.md) for queue semantics.

`windwatcher migrate` applies storage migrations and queue migrations when a
queue is configured. `--storage` and `--queue` target them independently, and
the same flags apply to `migrate status`. Explicit `--queue` without queue
configuration is an actionable error. Runtime and operator commands check the
schema and never migrate implicitly.
SQLite `create_if_missing` is honored only when applying migrations; normal
runtime, diagnostic and status commands force existing-only connections and
never create data directories.

JWT uses a direct `auth.jwt.secret` or exact bytes from `auth.jwt.secret_file`,
never both. Secret files are not trimmed: a final newline is part of the secret.
Generate a file safely with `printf '%s' "$SECRET" > jwt.key`. `config check`
validates JWT configuration, while migration and provisioning commands intentionally
do not resolve JWT or session policy.

When environment variables configure either JWT secret source, that source replaces
the complete TOML secret source. Setting both `WINDWATCHER__AUTH__JWT__SECRET` and
`WINDWATCHER__AUTH__JWT__SECRET_FILE` is a semantic JWT error reported by `config
check`, `serve` and `doctor`, but does not block migration or provisioning.
Direct environment-secret values are read as exact UTF-8 text rather than Figment's
structured-value syntax.

## Messages

```toml
[messages]
max_content_bytes = 131072
```

The default inclusive new-write limit is 131072 UTF-8 bytes (128 KiB), not a
character count. Zero accepts only empty content. This resolves to the library's
`MessagePolicy` for sends and edits; historical reads are not revalidated against
the current size limit. Override it with
`WINDWATCHER__MESSAGES__MAX_CONTENT_BYTES=131072`. Negative, non-integer and unknown
message fields are rejected. `config check` resolves the policy without opening
a database or creating files. No message runtime, message CLI commands or
dedicated message-limit CLI flag are introduced.

## Listener and process lifecycle

```toml
[server]
bind = "127.0.0.1:8080"
shutdown_grace_period = "30s"
readiness_timeout = "2s"
```

The default listener is loopback-only, not externally reachable. Bind must be a
numeric `SocketAddr`, such as `127.0.0.1:8080`, `[::1]:8080` or `[::]:8080`;
hostnames/DNS are not supported. Port zero is valid and the actual bound address
is logged. Containers or reverse proxies may explicitly select `0.0.0.0:8080`.
TLS termination remains external in v1.

Both durations must be positive and representable by the runtime clock.
`shutdown_grace_period` is one total budget for HTTP, WebSockets and dispatch,
including lifecycle coordination, not a separate timeout per component.
`readiness_timeout` bounds one primary-schema dependency inspection, not backend
connection/pool configuration. Worker keeps its independent `[queue]` grace.

Environment overrides include `WINDWATCHER__SERVER__BIND`,
`WINDWATCHER__SERVER__SHUTDOWN_GRACE_PERIOD` and
`WINDWATCHER__SERVER__READINESS_TIMEOUT`. There are no matching listener CLI flags.
Listener/lifecycle configuration belongs here, not under `[http]` or `[realtime]`.
See [operations](operations.md).

## HTTP behavior

`[http.cors].allowed_origins` defaults to `[]`. Use explicit web origins such as
`["https://example.com", "http://localhost:3000"]`, without trailing slashes, or
`["*"]` alone. Credentialed CORS is not supported.

`[http.rate_limit].enabled` defaults to `true`. The `general` policy defaults to
`requests = 100`, `period = "1s"`, `burst = 200`; the additional `auth` policy
defaults to `10`, `"1m"`, `10`. These are token buckets with sustained refill and
an initial/maximum burst, not fixed-window counters. Counts and durations must
be positive. Nanosecond refill intervals round upward, never allowing a faster
rate than configured. Invalid quotas are rejected even when disabled.

`config check` validates HTTP settings without binding a listener. Limits are
per-process and use the immediate peer IP unless trusted proxies are explicitly
configured:

```toml
[http.rate_limit]
trusted_proxies = ["127.0.0.1/32", "::1/128"]
```

The default is `[]`. Entries must be explicit IPv4/IPv6 CIDRs, validated even
when limiting is disabled. Trust only the smallest networks actually occupied by
your proxies, not all private networks or the entire Internet. The proxy must
overwrite `X-Forwarded-For` or append the IP it actually observed; trusting a
proxy cannot repair an unsafe header-passthrough configuration.

Only a trusted TCP peer may supply `X-Forwarded-For`. The server walks the chain
right to left, skips trusted hops and uses the first untrusted IP for both quotas.
IPv4-mapped IPv6 addresses are normalized to IPv4; configure their networks as
IPv4 CIDRs. `Forwarded`, `X-Real-IP` and vendor headers are ignored. Header values
accept bare IPs only, with optional surrounding spaces/tabs, not ports or hostnames.
Missing, repeated, invalid or excessive headers, and chains containing only trusted
IPs, fall back to the peer bucket. Parsing is capped at 4096 bytes and 32 hops;
invalid chains are never partially salvaged. Without opt-in, clients behind a
proxy share its bucket. These quotas are neither persistent nor distributed.

Environment overrides use Figment array syntax, for example:
`WINDWATCHER__HTTP__CORS__ALLOWED_ORIGINS='["https://example.com"]'`, and nested
values such as `WINDWATCHER__HTTP__RATE_LIMIT__AUTH__BURST=20`.
Trusted proxies use the same array syntax:
`WINDWATCHER__HTTP__RATE_LIMIT__TRUSTED_PROXIES='["127.0.0.1/32","::1/128"]'`.

## Realtime behavior

```toml
[realtime]
ticket_ttl = "30s"
outgoing_buffer = 32
heartbeat_interval = "30s"
heartbeat_timeout = "10s"
session_recheck_interval = "30s"
dispatch_concurrency = 32
dispatch_timeout = "5s"
```

All durations and the per-connection event buffer must be positive and
representable; heartbeat timeout must be less than heartbeat interval. These are
operator defaults, not fixed event-contract guarantees. `config check` validates
the policy without starting a listener. Environment overrides use, for example,
`WINDWATCHER__REALTIME__OUTGOING_BUFFER=64`. The composition API consumes
`RawRealtime::resolve()` through `HttpState::with_realtime_config` before router
construction; cloned routers share the runtime. See [realtime](realtime.md).

Notification submission is nonblocking. `dispatch_concurrency` bounds running
best-effort tasks (including hydration and audience lookup), not socket count;
saturation drops the new notification and logs safe metadata. `dispatch_timeout`
limits the entire task independently of socket heartbeat policy. Both must be
positive and representable. There is no queue of tasks waiting for capacity.

## Logging output

`logging.filter` accepts `tracing_subscriber::EnvFilter` syntax and `logging.format`
is `compact`, `pretty`, or `json`. Logs and warnings go to stderr; command results
go to stdout. `--log-filter` and `--log-format` override the configured values.
Configuration debug output redacts direct JWT secrets.
Raw Tungstenite protocol diagnostics are suppressed even under a broad trace
filter because they can contain message frames and handshake secrets. Embedding
hosts that install their own logger must likewise disable `tungstenite` and
`tokio_tungstenite` targets and avoid URI-query/body logging.
