#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 1 ]]; then echo "Usage: $0 <tool-directory>" >&2; exit 2; fi
case $(uname -m) in
    x86_64) arch=x86_64; checksum=6d7f9a28b2f397934403950fcbba83f598b08d02edbe79a0815b3b4897341750;;
    aarch64) arch=arm64; checksum=a9a75213763783b7669719e505474c748c5ad7dc66591a195619deb50f85c2f4;;
    *) echo 'Unsupported host architecture' >&2; exit 1;;
esac
mkdir -p -- "$1"
dest=$(cd -- "$1" && pwd)
archive=$(mktemp)
trap 'rm -f -- "$archive"' EXIT
curl --fail --location --retry 3 "https://github.com/goreleaser/nfpm/releases/download/v2.48.0/nfpm_2.48.0_Linux_${arch}.tar.gz" -o "$archive"
printf '%s  %s\n' "$checksum" "$archive" | sha256sum -c -
tar -xzf "$archive" -C "$dest" nfpm
"$dest/nfpm" --version
