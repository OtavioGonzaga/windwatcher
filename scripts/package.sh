#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 4 ]]; then
    echo "Usage: $0 <deb|rpm> <x86_64|aarch64> <binary> <output-dir>" >&2
    exit 2
fi
format=$1 arch=$2 binary=$3 output=$4
case $format in deb|rpm) ;; *) echo "Invalid package format: $format" >&2; exit 2;; esac
case $arch in x86_64|aarch64) ;; *) echo "Invalid architecture: $arch" >&2; exit 2;; esac
if [[ ! -f $binary || ! -x $binary ]]; then
    echo "Binary must be a regular executable file: $binary" >&2
    exit 2
fi
release=${PACKAGE_RELEASE:-1}
if [[ ! $release =~ ^[1-9][0-9]*$ ]]; then
    echo 'PACKAGE_RELEASE must be a positive integer' >&2
    exit 2
fi
binary=$(realpath -- "$binary")
mkdir -p -- "$output"
output=$(cd -- "$output" && pwd)
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
version=$(cargo metadata --manifest-path "$root/Cargo.toml" --no-deps --format-version 1 --locked | jq -er '.packages[] | select(.name == "windwatcher-server") | .version')
case "$format:$arch" in
    deb:x86_64) filename="windwatcher-server_${version}-${release}_amd64.deb";;
    deb:aarch64) filename="windwatcher-server_${version}-${release}_arm64.deb";;
    rpm:*) filename="windwatcher-server-${version}-${release}.${arch}.rpm";;
esac
export PACKAGE_ARCH=$arch PACKAGE_VERSION=$version PACKAGE_RELEASE=$release PACKAGE_BINARY=$binary
cd -- "$root"
nfpm package --config packaging/nfpm.yaml --packager "$format" --target "$output/$filename" >&2
printf '%s\n' "$output/$filename"
