#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
export LOG=$tmp/log
cat > "$tmp/systemctl" <<'EOF'
#!/bin/sh
echo "$*" >> "$LOG"
exit 1
EOF
cat > "$tmp/systemd-sysusers" <<'EOF'
#!/bin/sh
echo sysusers >> "$LOG"
exit "${FAIL_SYSUSERS:-0}"
EOF
cat > "$tmp/systemd-tmpfiles" <<'EOF'
#!/bin/sh
echo tmpfiles >> "$LOG"
exit "${FAIL_TMPFILES:-0}"
EOF
chmod +x "$tmp"/systemctl "$tmp"/systemd-sysusers "$tmp"/systemd-tmpfiles
export PATH=$tmp:$PATH
for action in upgrade 1 2 unknown; do
    : > "$LOG"
    sh "$root/packaging/scripts/preremove.sh" "$action"
    [[ ! -s $LOG ]]
done
for action in remove 0; do
    : > "$LOG"
    sh "$root/packaging/scripts/preremove.sh" "$action"
    [[ $(sed -n '1s/ .*//p' "$LOG") == stop ]]
    [[ $(sed -n '2s/ .*//p' "$LOG") == disable ]]
done
: > "$LOG"
sh "$root/packaging/scripts/postinstall.sh" configure
grep -qx sysusers "$LOG"
grep -qx tmpfiles "$LOG"
grep -qx daemon-reload "$LOG"
if FAIL_SYSUSERS=1 sh "$root/packaging/scripts/postinstall.sh" configure; then
    echo 'sysusers failure was ignored' >&2; exit 1
fi
: > "$LOG"
if FAIL_TMPFILES=1 sh "$root/packaging/scripts/postinstall.sh" configure; then
    echo 'tmpfiles failure was ignored' >&2; exit 1
fi
if grep -q daemon-reload "$LOG"; then
    echo 'daemon-reload ran after provisioning failure' >&2; exit 1
fi
: > "$LOG"
sh "$root/packaging/scripts/postinstall.sh" abort-upgrade
[[ ! -s $LOG ]]
echo 'Package hook tests passed.'
