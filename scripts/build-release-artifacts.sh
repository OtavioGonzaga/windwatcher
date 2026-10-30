#!/usr/bin/env bash
set -euo pipefail
[[ $# == 3 ]] || { echo "Usage: $0 VERSION ARCH OUTPUT" >&2; exit 2; }
version=$1 arch=$2 output=$3
python3 scripts/release.py validate-version windwatcher-server "$version"
case $arch in x86_64|aarch64) ;; *) exit 2;; esac
[[ $(uname -m) == "$arch" ]]
mkdir -p "$output"
output=$(realpath "$output")
[[ -z $(find "$output" -mindepth 1 -print -quit) ]]
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
cargo build --release --locked -p windwatcher-server --bin windwatcher
strip --strip-unneeded target/release/windwatcher
binary=$(realpath target/release/windwatcher)
"$binary" version | grep -Fx "windwatcher $version"
"$binary" --help > /dev/null
ldd "$binary"
readelf -h -d --version-info "$binary"
deb=$(PACKAGE_RELEASE=1 scripts/package.sh deb "$arch" "$binary" "$output")
rpm=$(PACKAGE_RELEASE=1 scripts/package.sh rpm "$arch" "$binary" "$output")
scripts/ci/verify-packaging.sh "$deb" "$rpm" "$arch" "$binary"
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
name=windwatcher-server-$version-linux-$arch
mkdir "$tmp/$name"
install -m 0755 "$binary" "$tmp/$name/windwatcher"
install -m 0644 LICENSE README.md config.example.toml packaging/systemd/*.service "$tmp/$name/"
# Stable tar metadata and gzip header make recovery builds as reproducible as possible.
epoch=$(git show -s --format=%ct HEAD)
tar --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner -C "$tmp" -cf - "$name" | gzip -n > "$output/$name.tar.gz"
cmp "$binary" "$tmp/$name/windwatcher"
install -m 0755 "$binary" "$output/windwatcher-$arch"
cd "$output"
for file in *.tar.gz *.deb *.rpm; do sha256sum "$file" > "$file.sha256"; done
