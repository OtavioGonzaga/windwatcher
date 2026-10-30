# Linux packages

nFPM uses `nfpm.yaml` to build both DEB and RPM from the same binary and the
canonical files in this directory. The package is named `windwatcher-server`;
the executable remains `windwatcher`. Its version comes from Cargo metadata and
its package revision defaults to 1, independently of the application version.

Install nFPM 2.48.0 separately, or use `scripts/ci/install-nfpm.sh` to download
the pinned release into a local tool directory. Then, from the repository root:

```bash
cargo build --release --locked -p windwatcher-server --bin windwatcher
scripts/package.sh deb x86_64 target/release/windwatcher dist
scripts/package.sh rpm x86_64 target/release/windwatcher dist
```

Use `aarch64` on ARM. Build on a compatible Linux baseline; package generation
does not cross-compile or verify the binary's architecture. `PACKAGE_RELEASE=2`
overrides the revision, for example to test upgrades. These packages are unsigned.

The package installs vendor units under `/usr/lib`, provisions the service
account and directories with systemd-sysusers and systemd-tmpfiles, and reloads
systemd where available. It never initializes configuration, runs migrations,
starts services, or restarts them during upgrades. Removal preserves the account,
configuration and state. See [installation](../docs/installation.md) for the
complete operator lifecycle and migration from manual units.

## Validation and lint policy

The CI packaging matrix builds once per architecture on Ubuntu 22.04, verifies
the ELF and package payloads, and exercises DEB on Debian 13 and RPM on Fedora 44
with a live systemd manager. The integration test uses privileged disposable
containers with access to the host cgroup hierarchy; use a disposable Linux host
or runner. It does not use external storage or queue services. CI strips the
binary once before producing both package formats.

`scripts/ci/lint-packaging.sh` runs lintian and rpmlint and fails on new findings.
Its narrow, package-specific exceptions cover only the following deliberate
choices:

- Debian `no-changelog` and RPM `no-changelogname-tag`: this is an upstream
  binary package, not a native Debian/Fedora source-package release; application
  changes are tracked in the server changelog instead.
- Debian `maintainer-script-calls-systemctl`: the two `prerm` calls implement
  best-effort stop and disable on final removal, never on upgrade.
- Missing manual page: man pages are outside this packaging delivery; the CLI
  has built-in help and the operator guide documents its lifecycle.
- RPM `no-signature`: signing is intentionally deferred to release automation.
- RPM `explicit-lib-dependency libgcc`: nFPM does not generate the usual RPM
  automatic ELF dependencies; the dynamically linked binary needs libgcc.
- RPM `tmpfile-not-in-filelist` for the two state directories: those directories
  intentionally are not package-owned so removal preserves operator data.
- Older rpmlint versions also report `only-non-binary-in-usr-lib` for vendor
  systemd files, `no-group-tag` for an obsolete RPM group and `invalid-license
  MIT` despite MIT being an SPDX identifier. These exact findings are filtered;
  no general warning category is suppressed.
