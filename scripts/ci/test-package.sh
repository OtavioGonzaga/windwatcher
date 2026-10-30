#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 3 ]]; then
    echo "Usage: $0 <debian|fedora> <release-1-package> <release-2-package>" >&2
    exit 2
fi
distro=$1
package1=$(realpath "$2")
package2=$(realpath "$3")
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
tmp=$(mktemp -d)
image="windwatcher-package-test-$$"
container="windwatcher-package-test-$$"
cleanup() {
    status=$?
    if (( status != 0 )); then
        docker logs "$container" >&2 || :
        docker exec "$container" journalctl -u windwatcher.service --no-pager -n 100 >&2 || :
    fi
    docker rm -f "$container" >/dev/null 2>&1 || :
    docker image rm "$image" >/dev/null 2>&1 || :
    rm -rf -- "$tmp"
    exit "$status"
}
trap cleanup EXIT
case $distro in
    debian)
        extension=deb
        cat > "$tmp/Dockerfile" <<'EOF'
FROM debian:13
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq systemd systemd-sysv dbus sudo curl sqlite3 && apt-get clean
CMD ["/sbin/init"]
EOF
        ;;
    fedora)
        extension=rpm
        cat > "$tmp/Dockerfile" <<'EOF'
FROM fedora:44
RUN dnf install -y systemd dbus-broker sudo curl shadow-utils sqlite && dnf clean all
CMD ["/sbin/init"]
EOF
        ;;
    *) echo "Unknown distribution: $distro" >&2; exit 2;;
esac
cp -- "$package1" "$tmp/one.$extension"
cp -- "$package2" "$tmp/two.$extension"
docker build -t "$image" "$tmp"
# An independent systemd manager needs a writable delegated cgroup hierarchy.
# Use this only on a disposable Linux host/runner.
docker run -d --name "$container" --privileged --cgroupns=host \
    --tmpfs /run --tmpfs /run/lock -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
    -v "$tmp:/packages:ro" \
    -v "$root/scripts/ci/test-package-inside.sh:/test-package-inside.sh:ro" \
    "$image" >/dev/null
ready=0
for _ in {1..60}; do
    if docker exec "$container" systemctl show --property=Version >/dev/null 2>&1; then ready=1; break; fi
    sleep 1
done
if (( ! ready )); then echo 'systemd did not start in the package test container' >&2; exit 1; fi
docker exec "$container" bash /test-package-inside.sh "$distro"
