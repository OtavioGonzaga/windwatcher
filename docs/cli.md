# CLI

The `windwatcher` executable provides explicit operator actions:

```bash
windwatcher init
windwatcher
windwatcher serve
windwatcher doctor
windwatcher migrate
windwatcher migrate status
windwatcher migrate --storage
windwatcher migrate --queue
windwatcher worker
windwatcher jobs list
windwatcher jobs retry <job-id>
windwatcher config check
windwatcher config show
windwatcher user create --username otavio
windwatcher user disable <user-id>
windwatcher user enable <user-id>
windwatcher version
```

`init` creates a minimal configuration and private signing key without
overwriting existing files; it does not create persistence, migrate or provision
users. Use `--config PATH` to select the new file. `version` is independent of
configuration. `config show` displays loaded effective configuration with secret
values and database/queue URLs redacted; it does not read secret-file contents,
validate semantic rules or connect dependencies. Logging CLI overrides are shown.

`user disable <user-id>` disables the user and revokes all existing sessions;
retrying also retries revocation. `user enable <user-id>` does not restore revoked
sessions. Both require existing storage and current schema, and are idempotent.

No subcommand means `serve`; both forms respect the existing `--system`,
`--config`, `--log-filter` and `--log-format` options before the command.
Serve runs HTTP and realtime, not worker slots or queue maintenance. Doctor
checks the whole configured installation without binding the listener. See
[operations](operations.md) for health and shutdown behavior.

Only `migrate` applies migrations. `serve`, `worker`, `doctor`, `migrate status`,
`config check`, and `user create` never migrate implicitly. Normal connections
also never create missing SQLite files or data directories, even when
`create_if_missing = true`; creation is reserved for applying migrations.
User provisioning requires a current schema and
creates only a user and local credential, not a session or tokens.

When queue configuration is present, `migrate` targets both storage and queue;
`--storage` and `--queue` select independently. Worker and `jobs` commands require
a configured queue and a current queue schema. `jobs list` shows all statuses by
default; payloads are never printed. `jobs retry` accepts only failed jobs. See
[jobs](jobs.md) for delivery and recovery semantics.

`user create` prompts securely when attached to a terminal. For automation use
`--password-stdin` or `--password-file`; their exact UTF-8 bytes are preserved and
bounded to 1024 bytes. `--password` is supported but emits a warning because
process arguments can expose it. Password sources are mutually exclusive.

Duplicate usernames return a concise operator error. Passwords, hashes, tokens,
and JWT secrets are never included in command output or tracing fields.
