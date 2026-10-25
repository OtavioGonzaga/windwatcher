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

## Logging

`logging.filter` accepts `tracing_subscriber::EnvFilter` syntax and `logging.format`
is `compact`, `pretty`, or `json`. Logs and warnings go to stderr; command results
go to stdout. `--log-filter` and `--log-format` override the configured values.
Configuration debug output redacts direct JWT secrets.
