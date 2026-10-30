#!/bin/sh
set -eu

case ${1-} in
    configure|[0-9]*) ;;
    *) exit 0 ;;
esac

systemd-sysusers /usr/lib/sysusers.d/windwatcher.conf
systemd-tmpfiles --create /usr/lib/tmpfiles.d/windwatcher.conf

if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || :
    if systemctl is-active --quiet windwatcher.service || systemctl is-active --quiet windwatcher-worker.service; then
        echo 'Windwatcher services were not restarted. Follow the documented stop, migrate, doctor and start procedure.' >&2
    fi
fi

for unit in windwatcher.service windwatcher-worker.service; do
    path=/etc/systemd/system/$unit
    if [ -e "$path" ] || [ -L "$path" ]; then
        echo "Warning: $path shadows the package unit in /usr/lib/systemd/system/." >&2
    fi
done
