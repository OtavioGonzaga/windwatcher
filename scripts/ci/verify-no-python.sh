#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
if [[ -n $(git ls-files '*.py') ]]; then
    echo 'Python files are not allowed in repository tooling' >&2
    exit 1
fi
# Match interpreter commands/shebangs, not prose mentioning the language.
# Bracketed initial letters keep this check from matching its own patterns.
interpreter='[p]ython([0-9]+([.][0-9]+)*)?'
pattern='(^[[:space:]]*#!.*[/ ])'"$interpreter"'([[:space:]]|$)|(^[[:space:]]*(run:[[:space:]]*|exec[[:space:]]+|env[[:space:]]+)?|[;|&(`][[:space:]]*)'"$interpreter"'([[:space:]]|$)|(^|[[:space:]])'"$interpreter"'[[:space:]]+(-[^[:space:]]+|[^[:space:]]+[.]py)'
if git grep -nE "$pattern" -- .github scripts packaging ':!*.md'; then
    echo 'Python interpreter calls are not allowed in automation' >&2
    exit 1
else
    status=$?
    [[ $status == 1 ]] || exit "$status"
fi
