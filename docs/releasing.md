# Releasing Windwatcher

The library `windwatcher` and application `windwatcher-server` have independent
versions, curated changelogs and annotated tags: `windwatcher-vX.Y.Z` and
`windwatcher-server-vX.Y.Z`. The server binary is named `windwatcher`.
Release mechanics never select the next version or release the other package.

## Maintainer setup

Create GitHub environment **release**, configure required reviewers as desired,
and add **CRATES_IO_TOKEN**, authorized to publish both crate names, including
their first publication. No manual `cargo publish` bootstrap is required.
The secret is injected only into crate publication steps, not CI/builds.
GHCR uses the job's `GITHUB_TOKEN`, not a PAT. Allow Actions to create packages
and ensure the repository has access to its associated GHCR package.
OCI preflight uses the authenticated GitHub Packages API to detect a missing
package on first publication; registry authentication failures are never used
as evidence that an exact version tag is absent.

Preparation and finalization use the protected environment and `contents: write`.
Repository rules must permit the Actions bot's release metadata pushes to
`main`; repository files cannot configure that permission. Do not disable
protections broadly. If policy forbids bot pushes, these direct-to-main
workflows cannot run under that policy.

## Dispatch

1. Review the package's single `## [Unreleased]` section. It must contain
   meaningful bullet entries; changelog prose is never generated from commits.
2. Select **Actions → Create windwatcher release** or **Create
   windwatcher-server release**, choose **main**, and supply `version`.
3. Enter SemVer without `v` or build metadata: `0.1.0`, `0.2.0`, or
   `0.2.0-rc.1`. Downgrades, leading zeroes and malformed versions fail.
   Initial equal version `0.1.0` is supported before its first release.
4. Approve environment gates and monitor the jobs. Full normal CI runs first.

Both workflows share a non-cancelling release concurrency group; ordinary CI is
independent. GitHub retains at most one pending run per concurrency group, so
avoid queueing multiple release dispatches.

New releases require dispatch from current main. Preparation promotes curated
Unreleased entries into a UTC-dated section, leaves an empty Unreleased, updates
only allow-listed manifests/changelog/lock entries, validates and commits as
`chore(release): PACKAGE vVERSION`. The bot fetches main again and fails instead
of rebasing/merging if its parent is no longer current. Lockfile third-party
entries are preserved, not broadly updated. The prepared commit receives
metadata/check/package/dry-run and script validation without repeating the
external-service matrix.

## Dependency order

Release the library first. Server preflight reads the exact numeric library
requirement from its manifest and requires that non-yanked version on crates.io
before packaging/publishing. It never publishes the library automatically.

For example, releasing library `0.2.0` from a `0.1.0` workspace changes its
package version and the server's path-plus-version dependency to `0.2.0`.
The server package stays `0.1.0` and its changelog is untouched. Only the library
crate, tag and GitHub Release are published. A later server release can choose
`0.2.0` (or another appropriate independent version), leaving the library alone.

## Outputs

Library releases publish to crates.io and a package-specific GitHub Release:
`cargo add windwatcher@VERSION`. They have no compiled binary artifacts.

Server releases publish the crate (`cargo install windwatcher-server --version
VERSION`) and six primary GitHub assets:

```text
windwatcher-server-VERSION-linux-x86_64.tar.gz
windwatcher-server-VERSION-linux-aarch64.tar.gz
windwatcher-server_VERSION-1_amd64.deb
windwatcher-server_VERSION-1_arm64.deb
windwatcher-server-VERSION-1.x86_64.rpm
windwatcher-server-VERSION-1.aarch64.rpm
```

Each has a `.sha256` sidecar. `SHA256SUMS` covers exactly those six relative
filenames. Download all six before running `sha256sum -c SHA256SUMS`, or check
one with `sha256sum -c FILE.sha256`. Native Ubuntu 22.04 x86_64/aarch64 jobs each
build and strip one binary for all three formats, using existing package
definitions and verification. No cross compilation or QEMU is involved.
Raw binaries and Actions artifacts remain internal plumbing.
Prerelease asset filenames retain SemVer (for example `0.2.0-rc.1`); nFPM
encodes the internal DEB/RPM version as `0.2.0~rc.1` for prerelease ordering.

Tarballs contain one directory with the executable, LICENSE, README, example
config and both units, not source/build trees, active config or secrets.
Follow [manual installation](installation.md#manual-installation-from-source)
to provision accounts/directories. Packages retain explicit
init/config/migrate/doctor/enable/start behavior.

Native Podman builds validate OCI metadata and lifecycle, then publish
`ghcr.io/otaviogonzaga/windwatcher-server`:

- `X.Y.Z-amd64`, `X.Y.Z-arm64`: immutable architecture/recovery tags.
- `X.Y.Z`: immutable manifest containing exactly linux/amd64 and linux/arm64.
- `X.Y`, `latest`: stable aliases moved after exact-manifest verification.
  Recovering an older release does not roll newer aliases back.
- Prereleases publish exact and architecture tags only, never stable aliases.
  There is no major-only alias, including `0`.

Tags are created only after crate publication (and for the server, artifact
validation and exact OCI publication). GitHub Releases are draft-first: assets
are verified before publication. A version containing `-` becomes a prerelease.

## Recovery

Re-run the **same workflow and version**. Preflight finds the package-specific
release commit reachable from main, verifies metadata, changed-file allow-list
and any existing tag, and checks out that exact commit. Main/dispatch SHA may
have advanced; no second release commit is created. Recovery requires a release
commit produced by this automation, not a version predating these scripts.

- Before a metadata commit: restart normally from current main.
- Commit pushed, nothing published: rebuild and continue from that SHA.
- Cargo upload succeeded but later work failed: verify the exact crate's VCS
  revision and packaged contents, then continue without republishing.
- One OCI architecture exists: verify OS/architecture/version/revision and reuse
  it; publish only the missing image.
- Exact OCI manifest exists: verify both architecture digests; never replace it.
- Tag exists: require an annotated tag resolving exactly to the release commit;
  wrong/lightweight tags fail. Tags are never force-moved.
- Draft exists: compare existing asset bytes, upload missing files and finalize
  only after the complete inventory exists.
- Published release exists: require the complete inventory and matching bytes;
  successful verification finishes without recreating or editing it.

Authentication, transport and rate-limit errors never mean 'not found'. Wrong
immutable registry versions, image metadata/digests or asset bytes fail closed.
Investigate instead of deleting or overwriting public state. Rebuilt distro
artifacts can differ if packaging tools or environments changed; mismatch
intentionally requires investigation rather than `--clobber`.

## First release: exact sequence

After this PR merges and main CI is green:

1. Configure environment `release` and its `CRATES_IO_TOKEN`.
2. Run **Create windwatcher release**, version **0.1.0**.
3. Verify crates.io `windwatcher 0.1.0`, annotated tag `windwatcher-v0.1.0`
   and its GitHub Release.
4. Run **Create windwatcher-server release**, version **0.1.0**.
5. Verify crates.io `windwatcher-server 0.1.0`, tarballs, DEBs, RPMs, sidecars,
   `SHA256SUMS`, amd64/arm64 images, multiarch `0.1.0`, annotated tag
   `windwatcher-server-v0.1.0` and the complete GitHub Release.
6. After the workflow creates the GHCR package, verify repository association
   and set package visibility to **Public** in GitHub package settings. This is
   a one-time manual bootstrap, not a broad-PAT visibility automation.
7. Verify anonymous pull using a clean authfile:

   ```bash
   authdir=$(mktemp -d)
   printf '{"auths":{}}\n' > "$authdir/auth.json"
   podman pull --authfile "$authdir/auth.json" ghcr.io/otaviogonzaga/windwatcher-server:0.1.0
   rm "$authdir/auth.json"
   rmdir "$authdir"
   ```

The first server release is operationally complete only after public pull works.
Future releases require no visibility changes. Merging automation does not
itself publish the first versions. Signing, SBOM/provenance, distro repositories
and additional deployment orchestrators are outside this release contract.
