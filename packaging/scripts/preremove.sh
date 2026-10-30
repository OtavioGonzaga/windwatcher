#!/bin/sh
set -eu

# Debian passes remove or upgrade; RPM passes 0 on erase and a positive
# count on upgrade. Unknown actions must never interrupt a running service.
case ${1-} in
    remove|0)
        if command -v systemctl >/dev/null 2>&1; then
            systemctl stop windwatcher.service windwatcher-worker.service || :
            systemctl disable windwatcher.service windwatcher-worker.service || :
        fi
        ;;
esac
