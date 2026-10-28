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

Malformed values, including invalid duration syntax, invalidate every command.
Semantic authentication failures such as a missing JWT secret or invalid session
policy are resolved only by `config check` and future runtime commands, so they do
not block migration or trusted user provisioning.

## Storage, auth, and secrets

SQLite is the default storage backend; `[storage.sqlite]` optionally sets `url`,
`create_if_missing`, `max_connections`, and human-readable `busy_timeout`.
Durations such as `5s`, `15m`, and `30d` are supported.

The durable queue is optional. When `[queue]` is configured, SQLite is currently
the supported provider and the default file is `queue.db`, separate from
`windwatcher.db`. Queue worker and operator commands require this section;
`config check` validates its concurrency, lease, retry and retention settings.
The queue database path must not resolve to the primary storage file. Queue
connection/migration initialization is command-specific; ordinary commands do
not open it. See [jobs](jobs.md) for queue semantics and worker operations.

`windwatcher migrate` applies storage migrations and queue migrations when a
queue is configured. `--storage` and `--queue` target them independently, and
the same flags apply to `migrate status`. Explicit `--queue` without queue
configuration is an actionable error. Runtime and operator commands check the
schema and never migrate implicitly.

JWT uses a direct `auth.jwt.secret` or exact bytes from `auth.jwt.secret_file`,
never both. Secret files are not trimmed: a final newline is part of the secret.
Generate a file safely with `printf '%s' "$SECRET" > jwt.key`. `config check`
validates JWT configuration, while migration and provisioning commands intentionally
do not resolve JWT or session policy.

When environment variables configure either JWT secret source, that source replaces
the complete TOML secret source. Setting both `WINDWATCHER__AUTH__JWT__SECRET` and
`WINDWATCHER__AUTH__JWT__SECRET_FILE` is a semantic JWT error reported by `config
check` and future runtime commands, but does not block migration or provisioning.
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
per-process and per immediate peer IP, not per forwarded client address. Behind
a reverse proxy, clients share its bucket; adjust or disable limits as needed.
Trusted-proxy support is not implemented.

Environment overrides use Figment array syntax, for example:
`WINDWATCHER__HTTP__CORS__ALLOWED_ORIGINS='["https://example.com"]'`, and nested
values such as `WINDWATCHER__HTTP__RATE_LIMIT__AUTH__BURST=20`.

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
