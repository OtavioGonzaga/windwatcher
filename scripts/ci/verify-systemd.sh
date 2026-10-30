#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
command -v systemd-analyze >/dev/null || { echo 'systemd-analyze is required' >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

for role in serve worker; do
    if [[ $role == serve ]]; then name=windwatcher; else name=windwatcher-worker; fi
    unit="$root/packaging/systemd/$name.service"
    for line in 'Type=exec' 'User=windwatcher' 'Group=windwatcher' \
        "ExecStart=/usr/bin/windwatcher --system $role" 'After=network.target' \
        'WorkingDirectory=/var/lib/windwatcher' 'StateDirectory=windwatcher' \
        'StateDirectoryMode=0750' 'UMask=0027' 'Restart=on-failure' \
        'RestartSec=5s' 'TimeoutStopSec=90s' 'StandardOutput=journal' \
        'StandardError=journal' 'NoNewPrivileges=true' 'CapabilityBoundingSet=' \
        'AmbientCapabilities=' 'ProtectSystem=strict' 'ProtectHome=true' \
        'PrivateTmp=true' 'PrivateDevices=true' 'ProtectKernelTunables=true' \
        'ProtectKernelModules=true' 'ProtectKernelLogs=true' 'ProtectControlGroups=true' \
        'RestrictSUIDSGID=true' 'RestrictRealtime=true' 'LockPersonality=true' \
        'RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6' 'WantedBy=multi-user.target'; do
        if [[ $(grep -Fxc -- "$line" "$unit" || true) != 1 ]]; then
            echo "$unit: expected exactly one $line" >&2; exit 1
        fi
    done
    if grep -Eq '^[[:space:]]*(ExecStartPre|ExecStop|ExecReload|EnvironmentFile|DynamicUser|Requires|Wants|PartOf|BindsTo|ReadWritePaths)=|network-online\.target|CAP_NET_BIND_SERVICE|^[[:space:]]*(After|Before)=.*windwatcher' "$unit"; then
        echo "$unit: prohibited service contract" >&2; exit 1
    fi
    if [[ $(grep -c '^ExecStart=' "$unit") != 1 ]]; then
        echo "$unit: unexpected ExecStart" >&2; exit 1
    fi
    # Only the verification copy uses an installed executable and current account.
    sed -e 's|^ExecStart=/usr/bin/windwatcher .*|ExecStart=/bin/true|' \
        -e "s/^User=windwatcher$/User=$(id -un)/" \
        -e "s/^Group=windwatcher$/Group=$(id -gn)/" "$unit" > "$tmp/$name.service"
done
systemd-analyze verify --man=no --recursive-errors=no \
    "$tmp/windwatcher.service" "$tmp/windwatcher-worker.service"
