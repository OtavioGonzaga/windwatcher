#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 4 ]]; then
  echo "usage: $0 IMAGE ARCH VERSION REVISION" >&2
  exit 2
fi
image=$1 arch=$2 version=$3 revision=$4
podman image inspect "$image" | python3 -c '
import json, sys
image = json.load(sys.stdin)[0]
arch, version, revision = sys.argv[1:]
assert image["Os"] == "linux", image["Os"]
assert image["Architecture"] == arch, image["Architecture"]
c = image["Config"]
assert c["User"] == "10001:10001", c
assert c["Entrypoint"] == ["/usr/bin/windwatcher", "--system"], c
assert c["Cmd"] == ["serve"], c
assert c["WorkingDir"] == "/var/lib/windwatcher", c
assert "8080/tcp" in c["ExposedPorts"], c
assert not c.get("Volumes"), c
assert c.get("StopSignal") in ("SIGTERM", "15"), c
assert not c.get("Healthcheck"), c
labels = c["Labels"]
expected = {"title": "windwatcher-server", "description": "Windwatcher standalone server and worker", "source": "https://github.com/OtavioGonzaga/windwatcher", "documentation": "https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/container.md", "licenses": "MIT", "version": version, "revision": revision}
for key, value in expected.items():
    assert labels.get("org.opencontainers.image." + key) == value, (key, labels)
' "$arch" "$version" "$revision"
podman run --rm --entrypoint /bin/sh "$image" -ec '
  test -x /usr/bin/windwatcher
  test -d /etc/windwatcher && test -w /etc/windwatcher
  test -d /var/lib/windwatcher && test -w /var/lib/windwatcher
  test "$(id -u)" = 10001 && test "$(id -g)" = 10001
  test "$(id -un)" = windwatcher && test "$(id -gn)" = windwatcher
  test -f /usr/share/licenses/windwatcher-server/LICENSE
  test ! -e /etc/windwatcher/config.toml && test ! -e /etc/windwatcher/jwt.key
  for command in cargo rustc gcc systemctl postgres mysqld mongod redis-server; do
    if command -v "$command" >/dev/null 2>&1; then echo "unexpected $command" >&2; exit 1; fi
  done
'
podman run --rm "$image" version
