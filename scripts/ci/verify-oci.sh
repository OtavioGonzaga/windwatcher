#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 4 ]]; then
  echo "usage: $0 IMAGE ARCH VERSION REVISION" >&2
  exit 2
fi
image=$1 arch=$2 version=$3 revision=$4
podman image inspect "$image" | jq -e --arg arch "$arch" --arg version "$version" --arg revision "$revision" '
  .[0] | .Os == "linux" and .Architecture == $arch and
  (.Config | .User == "10001:10001" and
    .Entrypoint == ["/usr/bin/windwatcher", "--system"] and .Cmd == ["serve"] and
    .WorkingDir == "/var/lib/windwatcher" and (.ExposedPorts | has("8080/tcp")) and
    ((.Volumes // {}) | length == 0) and
    (.StopSignal == "SIGTERM" or .StopSignal == "15") and
    ((.Healthcheck // {}) | length == 0) and
    (.Labels | .["org.opencontainers.image.title"] == "windwatcher-server" and
      .["org.opencontainers.image.description"] == "Windwatcher standalone server and worker" and
      .["org.opencontainers.image.source"] == "https://github.com/OtavioGonzaga/windwatcher" and
      .["org.opencontainers.image.documentation"] == "https://github.com/OtavioGonzaga/windwatcher/blob/main/docs/container.md" and
      .["org.opencontainers.image.licenses"] == "MIT" and
      .["org.opencontainers.image.version"] == $version and
      .["org.opencontainers.image.revision"] == $revision))
' > /dev/null
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
