#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 1 ]]; then echo "usage: $0 IMAGE" >&2; exit 2; fi
image=$1
prefix="ww-oci-$$-$RANDOM"
config="$prefix-config" state="$prefix-state" server="$prefix-server" worker="$prefix-worker"
keysecret="$prefix-key" configsecret="$prefix-toml"
tmp=$(mktemp -d)
cleanup() {
  podman logs "$server" 2>/dev/null || true
  podman rm -f "$server" "$worker" >/dev/null 2>&1 || true
  podman volume rm "$config" "$state" >/dev/null 2>&1 || true
  podman secret rm "$keysecret" "$configsecret" >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT
podman volume create "$config" >/dev/null
podman volume create "$state" >/dev/null
mounts=(-v "$config:/etc/windwatcher:U" -v "$state:/var/lib/windwatcher:U")
podman run --rm "${mounts[@]}" "$image" init
podman run --rm --entrypoint /bin/sh "${mounts[@]}" "$image" -ec 'test -s /etc/windwatcher/config.toml && test -s /etc/windwatcher/jwt.key'
podman run --rm "${mounts[@]}" "$image" migrate
podman run --rm "${mounts[@]}" "$image" doctor
podman run --rm --entrypoint /bin/sh "${mounts[@]}" "$image" -ec 'test -s /var/lib/windwatcher/windwatcher.db; sha256sum /etc/windwatcher/jwt.key' > "$tmp/key-before"

start_server() {
  podman run -d --name "$server" --read-only --cap-drop=ALL --security-opt=no-new-privileges \
    "${mounts[@]}" -e WINDWATCHER__SERVER__BIND=0.0.0.0:8080 -p 127.0.0.1::8080 "$image" >/dev/null
  local port
  port=$(podman port "$server" 8080/tcp | head -n1 | sed 's/.*://')
  for _ in {1..60}; do
    if curl -fsS "http://127.0.0.1:$port/live" >/dev/null && curl -fsS "http://127.0.0.1:$port/ready" >/dev/null; then return; fi
    sleep 1
  done
  echo 'server did not become ready' >&2; return 1
}
stop_server() {
  podman stop -t 35 "$server" >/dev/null
  test "$(podman inspect --format '{{.State.ExitCode}}' "$server")" = 0
  podman rm "$server" >/dev/null
}
start_server
podman exec "$server" sh -ec 'test "$(awk "/^Uid:/ {print \$2}" /proc/1/status)" = 10001; test "$(awk "/^Gid:/ {print \$2}" /proc/1/status)" = 10001; tr "\0" " " < /proc/1/cmdline | grep -q "windwatcher --system"'
stop_server
podman run --rm --entrypoint /bin/sh "${mounts[@]}" "$image" -ec 'test -s /var/lib/windwatcher/windwatcher.db; sha256sum /etc/windwatcher/jwt.key' > "$tmp/key-after"
cmp "$tmp/key-before" "$tmp/key-after"
start_server
stop_server

printf '%s' '0123456789abcdef0123456789abcdef' > "$tmp/key"
cat > "$tmp/config.toml" <<'EOF'
[storage]
provider = "postgres"
[storage.postgres]
url = "postgres://example:password@localhost/windwatcher"
[queue]
provider = "redis"
[queue.redis]
url = "redis://localhost/0"
[auth.jwt]
secret_file = "/run/secrets/jwt.key"
EOF
podman secret create "$keysecret" "$tmp/key" >/dev/null
podman secret create "$configsecret" "$tmp/config.toml" >/dev/null
secrets=(--secret "$keysecret,type=mount,target=jwt.key,uid=10001,gid=10001,mode=0400" --secret "$configsecret,type=mount,target=windwatcher.toml,uid=10001,gid=10001,mode=0400")
podman run --rm "${mounts[@]}" "${secrets[@]}" -e WINDWATCHER__AUTH__JWT__SECRET_FILE=/run/secrets/jwt.key "$image" config check
podman run --rm "${secrets[@]}" "$image" --config /run/secrets/windwatcher.toml config check
podman run --rm "${secrets[@]}" "$image" --config /run/secrets/windwatcher.toml config show > "$tmp/show"
! grep -q 'password' "$tmp/show"
grep -q postgres "$tmp/show"
grep -q redis "$tmp/show"

podman run --rm "${mounts[@]}" -e WINDWATCHER__QUEUE__PROVIDER=sqlite "$image" migrate --queue
podman run -d --name "$worker" "${mounts[@]}" -e WINDWATCHER__QUEUE__PROVIDER=sqlite "$image" worker >/dev/null
sleep 3
test "$(podman inspect --format '{{.State.Running}}' "$worker")" = true
podman stop -t 35 "$worker" >/dev/null
test "$(podman inspect --format '{{.State.ExitCode}}' "$worker")" = 0
