# CLI

The `windwatcher` executable provides explicit operator actions:

```bash
windwatcher migrate
windwatcher migrate status
windwatcher config check
windwatcher user create --username otavio
```

Only `migrate` applies migrations. `migrate status`, `config check`, and `user
create` never migrate implicitly. User provisioning requires a current schema and
creates only a user and local credential, not a session or tokens.

`user create` prompts securely when attached to a terminal. For automation use
`--password-stdin` or `--password-file`; their exact UTF-8 bytes are preserved and
bounded to 1024 bytes. `--password` is supported but emits a warning because
process arguments can expose it. Password sources are mutually exclusive.

Duplicate usernames return a concise operator error. Passwords, hashes, tokens,
and JWT secrets are never included in command output or tracing fields.
