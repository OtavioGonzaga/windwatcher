#!/usr/bin/env bash
# Called only by manually dispatched release workflows after the full reusable CI.
set -euo pipefail
package=$1 version=$2
[[ $GITHUB_REF == refs/heads/main ]] || { echo 'Releases and recovery must be dispatched from main' >&2; exit 1; }
automation=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
root=$PWD
xtask() { cargo run --quiet --locked --manifest-path "$automation/tools/xtask/Cargo.toml" -- release "$@" --root "$root"; }
git fetch origin main --tags
state=$(xtask state "$package" "$version")
if grep -qx 'prepared=true' <<< "$state"; then
    printf '%s\n' "$state" >> "$GITHUB_OUTPUT"
    exit 0
fi
[[ $GITHUB_SHA == "$(git rev-parse origin/main)" ]] || { echo 'main advanced; restart from main' >&2; exit 1; }
[[ -z $(git status --porcelain) ]]
git checkout --detach origin/main
xtask prepare "$package" "$version"
xtask allow-list "$package" "$version"
cargo metadata --locked --format-version 1 > /dev/null
cargo check --workspace --all-features --locked
"$automation/scripts/ci/test-release-scripts.sh"
xtask allow-list "$package" "$version"
git config user.name 'github-actions[bot]'
git config user.email '41898282+github-actions[bot]@users.noreply.github.com'
git add Cargo.lock "crates/$package/Cargo.toml" "crates/$package/CHANGELOG.md"
if [[ $package == windwatcher ]]; then git add crates/windwatcher-server/Cargo.toml; fi
git commit -m "chore(release): $package v$version"
xtask state --main-ref HEAD "$package" "$version"
git fetch origin main
[[ $(git rev-parse HEAD^) == "$(git rev-parse origin/main)" ]] || { echo 'main advanced; refusing to rebase or merge' >&2; exit 1; }
git push origin HEAD:refs/heads/main
echo "release_commit=$(git rev-parse HEAD)" >> "$GITHUB_OUTPUT"
