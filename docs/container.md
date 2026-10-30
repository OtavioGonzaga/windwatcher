# OCI container deployment

The repository provides an official OCI image definition, not a published image
or registry tag. Install [Podman](https://podman.io/) and build from the checkout:

```bash
podman build --format=oci --file Containerfile --tag localhost/windwatcher-server:local .
```

Podman and `.containerignore` define the supported source-build workflow. OCI
images can also be consumed by compatible runtimes, including Docker-compatible
environments. Base image digests are reviewed and updated manually.

## Runtime contract

The image runs `/usr/bin/windwatcher --system` directly as PID 1, UID/GID
`10001:10001` (user and group `windwatcher`). Its working directory is
`/var/lib/windwatcher`, where relative paths resolve. The default command is
`serve`; replace it with `worker`, `init`, `migrate`, `doctor`, or `config check`.
Port 8080 is image metadata only; TLS termination is external. No initialization,
migration, config, key, database server or process supervisor is included.

System mode selects `/etc/windwatcher/config.toml`,
`/var/lib/windwatcher/windwatcher.db` and, when configured, `queue.db` in that
state directory. Unlike host packages, both image directories are owned by UID/GID
10001 and mode 0750, so initialization can run unprivileged. The image declares
no volumes: operators choose named volumes, carefully prepared bind mounts,
runtime-managed secrets or ephemeral filesystems. Persist state for SQLite and
relative local files. With entirely external persistence, local state may be
ephemeral. Persist configuration if using `init`; it is unnecessary when using
an explicit mounted configuration file. Back up both independently.

## SQLite lifecycle

The following uses rootless Podman named volumes and its `:U` ownership option.
`:U` recursively changes ownership on the volume; use it only on dedicated
volumes, never on shared host directories. For bind mounts, prepare permissions
for container UID/GID 10001 using your runtime's user-namespace mapping; do not
make secrets world-readable. SELinux hosts may also require an appropriate
relabel option. Never run the application as root to fix permissions.

```bash
podman volume create windwatcher-config
podman volume create windwatcher-state
podman run --rm -v windwatcher-config:/etc/windwatcher:U -v windwatcher-state:/var/lib/windwatcher:U localhost/windwatcher-server:local init
podman run --rm -v windwatcher-config:/etc/windwatcher:U -v windwatcher-state:/var/lib/windwatcher:U localhost/windwatcher-server:local migrate
podman run --rm -v windwatcher-config:/etc/windwatcher:U -v windwatcher-state:/var/lib/windwatcher:U localhost/windwatcher-server:local doctor
podman run -d --name windwatcher --read-only --cap-drop=ALL --security-opt=no-new-privileges \
  -v windwatcher-config:/etc/windwatcher:U -v windwatcher-state:/var/lib/windwatcher:U \
  -e WINDWATCHER__SERVER__BIND=0.0.0.0:8080 -p 127.0.0.1:8080:8080 \
  localhost/windwatcher-server:local
```

Set the bind explicitly as above or in TOML: the default is loopback *inside*
the container. Environment overrides TOML, so the image does not set it itself.
Publishing only on host loopback is intentional; use an appropriately secured
proxy for external traffic. Configure the optional queue and run a separate
container with the same image version and mounts, replacing the command with
`worker`. Do not run server and worker as one container. See [jobs](jobs.md).

## Secrets and external providers

The application reads files, not a Podman-specific API. For example, configure
`[auth.jwt] secret_file = "/run/secrets/jwt.key"`, or set
`WINDWATCHER__AUTH__JWT__SECRET_FILE=/run/secrets/jwt.key`. Runtime-managed
secrets or read-only bind mounts must be readable by UID/GID 10001. Do not embed
credentials in image build arguments, labels, or environment values. Podman can
mount a pre-created secret with:

```bash
podman secret create windwatcher-jwt /secure/path/jwt.key
podman run --rm --secret windwatcher-jwt,type=mount,target=jwt.key,uid=10001,gid=10001,mode=0400 \
  -e WINDWATCHER__AUTH__JWT__SECRET_FILE=/run/secrets/jwt.key \
  localhost/windwatcher-server:local config check
```

For sensitive database and queue URLs, mount a complete TOML as a secret at
`/run/secrets/windwatcher.toml` and pass `--config` before the command. For
example, create this file privately with real credentials and a separate JWT key:

```toml
[storage]
provider = "postgres"
[storage.postgres]
url = "postgres://user:password@database/windwatcher"
[queue]
provider = "redis"
[queue.redis]
url = "redis://queue/0"
[auth.jwt]
secret_file = "/run/secrets/jwt.key"
```

```bash
podman secret create windwatcher-config /secure/path/windwatcher.toml
podman run --rm \
  --secret windwatcher-config,type=mount,target=windwatcher.toml,uid=10001,gid=10001,mode=0400 \
  --secret windwatcher-jwt,type=mount,target=jwt.key,uid=10001,gid=10001,mode=0400 \
  localhost/windwatcher-server:local --config /run/secrets/windwatcher.toml config check
```

The explicit config does not change system-mode state paths. SQLite, PostgreSQL,
MySQL and MongoDB primary storage and SQLite, PostgreSQL, MySQL and Redis queues
are selected through existing [configuration](server-configuration.md); this
image does not bundle those services. Configure networking and secret permissions
for your environment. `config show` redacts sensitive values and does not connect.

## Health, shutdown and upgrades

`/live` is liveness; `/ready` tests primary-storage readiness. The image contains
no embedded healthcheck or HTTP client; configure probes externally in Podman,
your proxy, or an orchestrator. `podman stop` sends SIGTERM directly to Windwatcher
for graceful shutdown. Allow the configured shutdown grace period.

For an upgrade, build or obtain the new image, stop old server and worker
containers, run `migrate` using the new image with the same external mounts and
secrets, run `doctor` and configuration validation, then start new server and
worker containers. Migrations are never automatic; rolling or zero-downtime
migrations are not promised. The persistent data must outlive the containers.
