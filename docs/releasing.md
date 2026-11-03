# Releasing Windwatcher

The library `windwatcher` and application `windwatcher-server` have independent
versions, curated changelogs and annotated tags: `windwatcher-vX.Y.Z` and
`windwatcher-server-vX.Y.Z`. The server binary is named `windwatcher`.
Release mechanics never select the next version or release the other package.

## Rust tooling

Structured release logic lives in the unpublished workspace package
`tools/xtask`. Python is not required. Bash handles short Unix orchestration;
Git, Cargo, `gh`, `curl`, Podman, Skopeo and `jq` remain external tools.

```bash
cargo xtask release --help
cargo xtask release prepare windwatcher 0.2.0
cargo xtask release state windwatcher 0.2.0
cargo xtask release notes windwatcher 0.2.0
cargo xtask release registry dependency windwatcher 0.1.0
cargo xtask release artifacts validate 0.1.0 dist
cargo xtask packaging reproducibility x86_64 target/release/windwatcher
```

`scripts/prepare-release.sh`, `scripts/release-state.sh` and
`scripts/extract-release-notes.sh` preserve the same interfaces. Preparation
supports `--date YYYY-MM-DD` (UTC today by default). Release-source commands
support `--root PATH`; state accepts `--main-ref REF`.

Post-prepare jobs use **two checkouts**: `automation/` at the workflow dispatch
SHA supplies the current xtask/recovery and verification logic; `release/` at
the validated immutable release commit supplies manifests, lockfile, changelog,
source, Containerfile, nFPM configuration, package hooks, units, example config
and other artifact inputs. Cargo package/publish/metadata/check run against
`release/`, not the tooling workspace. For example:

```bash
cargo run --quiet --locked \
  --manifest-path automation/tools/xtask/Cargo.toml -- \
  release github finalize windwatcher "$VERSION" "$RELEASE_COMMIT" \
  --root "$PWD/release"
```

This permits current recovery fixes to validate an old release without running
its historical release automation or changing its content.

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

**Required credential boundary:** in the `release` environment settings, set
**Deployment branches and tags → Selected branches and tags → Branch: main**.
Do not allow feature branches or tags. Configure this before adding the token
or dispatching any release, including recovery. These server-side rules withhold
environment secrets before a job runs; a check in a branch's own workflow or
script cannot protect secrets from that same branch modifying the check.

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

All releases, including recovery, must be dispatched from the `main` branch.
Only a new release requires the dispatch SHA to still be current main;
recovery can use an older validated release commit reachable from main.
Preparation promotes curated Unreleased entries into a UTC-dated section,
leaves an empty Unreleased, updates
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

Release packaging exports `SOURCE_DATE_EPOCH` from the release commit before
nFPM runs; the tarball uses that same timestamp. RPM metadata uses the fixed
`windwatcher-release` build host. Native package CI rebuilds both formats twice
from identical binary bytes with different paths/mtimes and checks byte equality,
commit timestamps and the RPM build host with real nFPM.

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

Dispatch a **new run of the same workflow and version from main**, especially
after a tooling fix. Do not merely rerun an old failed Actions job: that job is
tied to its old workflow/tooling SHA. Preflight finds the package-specific
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

Draft discovery lists all repository releases with pagination and selects the
exact `tag_name`, requiring at most one match. It does not use release-by-tag
lookup, which can return 404 for drafts. A draft's `untagged-*` HTML URL does not
override its authoritative `id`, `tag_name` and `draft` fields. Subsequent state,
asset and publication operations use the numeric ID returned by GitHub. The
finalizer verifies the published state after PATCH; it never clobbers assets or
edits an already published release.

Authentication, transport and rate-limit errors never mean 'not found'. Wrong
immutable registry versions, image metadata/digests or asset bytes fail closed.
Investigate instead of deleting or overwriting public state. Rebuilt distro
artifacts can differ if packaging tools or environments changed; mismatch
intentionally requires investigation rather than `--clobber`.

## Recovering the first library release

`windwatcher 0.1.0` was published successfully to crates.io from
`d8a47d9e4f71cf5287d1dc8f523e17644cbc2b4b`. Its annotated
`windwatcher-v0.1.0` tag and asset-free GitHub draft already exist. Finalization
failed because the old automation attempted to discover that draft by tag.

After this tooling fix is merged and main CI is green:

1. Leave the existing crate, annotated tag and draft untouched. Do not delete,
   recreate, move, rewrite the released changelog or publish another version as
   a workaround.
2. Start a **new** **Create windwatcher release** dispatch from **main** with
   version **0.1.0**, not a rerun of the old failed job.
3. Confirm preparation recovers the commit above and creates no second release
   commit. Packaging must use that immutable revision.
4. Confirm crates.io is verified, not republished: VCS SHA and `path_in_vcs`
   match, and every archived file equals the validated local crate.
5. Confirm the existing annotated tag is verified, the existing draft is found
   by exact `tag_name`, reused with zero assets and published successfully.
6. Only after library recovery succeeds, dispatch **Create windwatcher-server
   release**, version **0.1.0**.

This production dispatch is a post-merge maintainer action, not part of testing
the PR. Generic recovery never hardcodes a release ID.

## First server release: exact sequence

After this PR merges and main CI is green:

1. Configure environment `release` to allow deployments only from branch `main`
   (**Selected branches and tags → Branch: main**), then add `CRATES_IO_TOKEN`.
2. Complete the library recovery procedure above.
3. Verify crates.io `windwatcher 0.1.0`, annotated tag `windwatcher-v0.1.0`
   and its published GitHub Release.
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
