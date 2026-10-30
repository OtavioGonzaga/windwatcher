# Linux installation and system services

For OCI/Podman deployment instead of host packages and systemd, see
[container deployment](container.md).

Install a DEB/RPM from the server's [GitHub Releases](https://github.com/OtavioGonzaga/windwatcher/releases),
a locally built package, or use manual installation below. All paths use the
same account, directories and canonical systemd units. Alternatively,
`cargo install windwatcher-server --version X.Y.Z` installs only the binary,
not accounts or units.

## Package installation

Download assets from a `windwatcher-server-vX.Y.Z` release, not a library release.
For example, for server 0.1.0 on x86_64:

```bash
base=https://github.com/OtavioGonzaga/windwatcher/releases/download/windwatcher-server-v0.1.0
curl -fLO "$base/windwatcher-server_0.1.0-1_amd64.deb"
curl -fLO "$base/windwatcher-server_0.1.0-1_amd64.deb.sha256"
sha256sum -c windwatcher-server_0.1.0-1_amd64.deb.sha256
sudo apt install ./windwatcher-server_0.1.0-1_amd64.deb
```

Use an actually published version. RPM users download
`windwatcher-server-0.1.0-1.x86_64.rpm` and its sidecar. For aarch64 use
DEB `arm64` or RPM `aarch64`. `SHA256SUMS` verifies all six primary assets
if all are downloaded. Install a downloaded or local package using:

```bash
sudo apt install ./windwatcher-server_<version>-1_amd64.deb
# Or on an RPM-based system:
sudo dnf install ./windwatcher-server-<version>-1.x86_64.rpm
```

Substitute the actual filename and architecture. See [package development](../packaging/README.md)
to build a package. The package installs the executable and vendor units under
`/usr/lib/systemd/system/`, provisions the account and directories, and installs
an example in `/usr/share/doc/windwatcher-server/`. It does not create an active
configuration or key, migrate databases, enable units or start services.

The package validation baselines are Debian 13 and Fedora 44 on x86_64 and
aarch64. These are validated environments, not an exhaustive compatibility list.

For tarballs, download `windwatcher-server-0.1.0-linux-x86_64.tar.gz` (or
`linux-aarch64`) and its `.sha256` sidecar from the same release. Verify before
extracting. The archive contains one directory with the executable, license,
README, example config and units. Install that executable instead of compiling
below, using the included units; manually provision the same account and
directories. Tarballs do not run package hooks or initialize active configuration.

### First-time initialization

```bash
sudo /usr/bin/windwatcher --system init
```

Edit `/etc/windwatcher/config.toml` if necessary, then follow the shared
[configuration and migration](#configure-and-migrate) steps. Afterward explicitly
enable and start the server as described under [operating services](#operate-services).

## Manual installation from source

### Build and provision

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

### Install manual service units

From the source checkout:

```bash
sudo install -m 0644 packaging/systemd/windwatcher.service /etc/systemd/system/
sudo install -m 0644 packaging/systemd/windwatcher-worker.service /etc/systemd/system/
sudo systemctl daemon-reload
```

## Operate services

After package installation or manual unit installation and explicit migrations:

```bash
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

## Upgrade packages

**Installing a newer package neither migrates schemas nor restarts running
Windwatcher services.** A running process continues using its old executable
until explicitly stopped. After installing the new package, stop the units you
actually use before migrating with the new binary:

```bash
sudo systemctl stop windwatcher.service
# Only if the worker is in use:
sudo systemctl stop windwatcher-worker.service
sudo -u windwatcher -- sh
cd /var/lib/windwatcher
/usr/bin/windwatcher --system config check
/usr/bin/windwatcher --system migrate
/usr/bin/windwatcher --system doctor
exit
sudo systemctl start windwatcher.service
# Only if the worker is in use:
sudo systemctl start windwatcher-worker.service
```

For a configured worker, run any required queue migrations before starting it.
Do not migrate while an older server or worker remains running; zero-downtime
rolling upgrades are not promised.

## Remove packages

Removing the package stops and disables the units and removes package-owned
files. Debian purge has the same data-preservation policy. Configuration,
JWT key, databases, other state and the service account remain; their cleanup
is a separate, potentially destructive administrator decision.

## Migrate from a manual installation

An administrator-owned full unit in `/etc/systemd/system/` takes precedence
over the package unit in `/usr/lib/systemd/system/`. Packages warn about it but
never remove it. Before installing the package, stop and disable the units in
use. Inspect any local units and remove them only after confirming they came
from the old manual installation and have no intentional local modifications:

```bash
sudo systemctl stop windwatcher.service windwatcher-worker.service
sudo systemctl disable windwatcher.service windwatcher-worker.service
# Only after reviewing both files:
sudo rm -f /etc/systemd/system/windwatcher.service /etc/systemd/system/windwatcher-worker.service
sudo systemctl daemon-reload
```

Install the package, preserve the account and `/etc/windwatcher` and
`/var/lib/windwatcher`, then perform the explicit upgrade procedure above.
For later customization prefer drop-ins under
`/etc/systemd/system/windwatcher.service.d/` or
`/etc/systemd/system/windwatcher-worker.service.d/` instead of copying a full
vendor unit. Drop-ins are administrator-owned and are not managed by the package.
