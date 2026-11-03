#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
export PYTHONDONTWRITEBYTECODE=1
python3 -B "$root/scripts/ci/test_release_scripts.py"
