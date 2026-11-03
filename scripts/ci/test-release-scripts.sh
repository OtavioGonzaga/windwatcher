#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
exec cargo test --manifest-path "$root/Cargo.toml" -p xtask --locked -- --test-threads=1
