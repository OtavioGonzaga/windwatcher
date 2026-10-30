# Linux installation and system services

This is a manual, source-based installation and the contract for future Linux
packages. Official `.deb` and `.rpm` packages are not yet available. Future
packages should provision the account and directories and install the canonical
units in `packaging/systemd/`, rather than maintaining copies of their contents.

## Build and provision

Build with the repository toolchain, then install the executable:

```bash
cargo build --release -p windwatcher-server --bin windwatcher
sudo install -o root -g root -m 0755 target/release/windwatcher /usr/bin/windwatcher
```

Create a system user and group named `windwatcher`, with a disabled login shell
and `/var/lib/windwatcher` as its home/state directory. For example, on a system
with `groupadd` and `useradd`:

```bash
sudo groupadd --system windwatcher
sudo useradd --system --gid windwatcher --home-dir /var/lib/windwatcher --shell /usr/sbin/nologin windwatcher
sudo install -d -o root -g windwatcher -m 2750 /etc/windwatcher
sudo install -d -o windwatcher -g windwatcher -m 0750 /var/lib/windwatcher
sudo /usr/bin/windwatcher --system init
```

Adapt account creation to the distribution. The setgid configuration directory
causes the files created by root to inherit the `windwatcher` group. The expected
contract is:

| Path | Owner:group | Mode |
| --- | --- | --- |
| `/usr/bin/windwatcher` | `root:root` | `0755` |
| `/etc/windwatcher` | `root:windwatcher` | `2750` |
| `/etc/windwatcher/config.toml` | `root:windwatcher` | `0640` |
| `/etc/windwatcher/jwt.key` | `root:windwatcher` | `0640` |
| `/var/lib/windwatcher` | `windwatcher:windwatcher` | `0750` |

The service can read configuration and the key, but cannot write the
configuration directory. `init` requires the directory to exist and does not
create accounts or change ownership. Check ownership after provisioning.
ProtectHome prevents service access to files under home directories: place
file-based secrets in an appropriate service-readable system location, normally
under `/etc/windwatcher`. Keep runtime state under `/var/lib/windwatcher`.

## Configure and migrate

Edit configuration as an administrator. Run application-state commands as the
service identity to avoid root-owned SQLite files. Start a shell as that identity
and change to the units' working directory so relative paths resolve consistently:

```bash
sudo -u windwatcher -- sh
cd /var/lib/windwatcher
/usr/bin/windwatcher --system config check
/usr/bin/windwatcher --system migrate
/usr/bin/windwatcher --system user create --username alice
/usr/bin/windwatcher --system doctor
exit
```

Enter the password interactively or use the CLI's stdin option; do not put it in
process arguments. Run queue migrations and `jobs` commands as the same identity
and from the same working directory. The service shell exits before the unit
installation commands below, leaving the administrator in the source checkout.
Migrations are always explicit and are never performed by service startup.

## Install and operate services

From the source checkout:

```bash
sudo install -m 0644 packaging/systemd/windwatcher.service /etc/systemd/system/
sudo install -m 0644 packaging/systemd/windwatcher-worker.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now windwatcher.service
systemctl status windwatcher.service
journalctl -fu windwatcher.service
```

Only when a queue is configured and migrated, optionally start the independent
worker:

```bash
sudo systemctl enable --now windwatcher-worker.service
journalctl -u windwatcher-worker.service
```

Both services run as `windwatcher`, restart on failure and stop through SIGTERM.
Their 90-second systemd stop deadline is outside the application drain budget.
If increasing an application shutdown grace period, also increase
`TimeoutStopSec` sufficiently using a systemd drop-in. Configuration is loaded
once: restart the affected unit after changing it. Use drop-ins for unusual
environment overrides; configuration files and file-based secrets are preferred.
The units grant no privileged-port capability; use a reverse proxy or a deliberate
local drop-in if a privileged port is needed. See [operations](operations.md)
and [jobs](jobs.md).

Future packages will provide account/directory creation, binary and unit
installation, configuration handling and install/remove validation. This manual
service integration does not provide packages, release artifacts or OCI images.
