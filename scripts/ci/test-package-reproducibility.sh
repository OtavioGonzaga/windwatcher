#!/usr/bin/env bash
# Exercise the real package definitions and nFPM, not the artifact-helper mocks.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
exec python3 -B "$root/scripts/ci/test_package_reproducibility.py" "$@"
