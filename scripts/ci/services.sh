#!/usr/bin/env bash
# Disposable GitHub CI services only; never point this helper at existing data.
set -euo pipefail

container_prefix=windwatcher-ci

containers() {
  docker ps -aq --filter "label=windwatcher.ci=true"
}

logs() {
  local container
  while read -r container; do
    [[ -n "$container" ]] || continue
    docker inspect --format '{{.Name}} {{.State.Status}}' "$container"
    docker logs --tail 100 "$container" 2>&1 || true
  done < <(containers)
}

wait_for() {
  local description=$1
  shift
  local attempt
  echo "Waiting for $description (at most 90 attempts)"
  for attempt in {1..90}; do
    if timeout 10s "$@" > /dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "Timed out waiting for $description (90 attempts)" >&2
  timeout 10s "$@" || true
  logs
  return 1
}

start() {
  local backend
  # Launch every requested process before polling so server-network startup
  # overlaps. Bind published ports only to loopback; no persistent volumes.
  for backend in "$@"; do
    case "$backend" in
      postgres)
        docker run -d --name "$container_prefix-postgres" --label windwatcher.ci=true \
          -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=password -e POSTGRES_DB=postgres \
          -p 127.0.0.1:5432:5432 postgres:18
        ;;
      mysql)
        docker run -d --name "$container_prefix-mysql" --label windwatcher.ci=true \
          -e MYSQL_ROOT_PASSWORD=password -e MYSQL_ROOT_HOST=% \
          -p 127.0.0.1:3306:3306 mysql:8.4
        ;;
      mongodb)
        # Host networking lets the replica set advertise the same loopback
        # address to mongosh inside the container and to the host-side driver.
        docker run -d --name "$container_prefix-mongodb" --label windwatcher.ci=true \
          --network host mongo:8.2 mongod --replSet rs0 --bind_ip 127.0.0.1
        ;;
      redis)
        docker run -d --name "$container_prefix-redis" --label windwatcher.ci=true \
          -p 127.0.0.1:6379:6379 redis:7
        ;;
      *) echo "Unknown CI backend: $backend" >&2; return 1 ;;
    esac
  done

  for backend in "$@"; do
    case "$backend" in
      postgres)
        wait_for 'PostgreSQL 18' docker exec "$container_prefix-postgres" \
          pg_isready -h 127.0.0.1 -U postgres -d postgres
        echo 'WINDWATCHER_TEST_POSTGRES_URL=postgresql://postgres:password@127.0.0.1:5432/postgres' >> "$GITHUB_ENV"
        ;;
      mysql)
        # TCP avoids accepting the image entrypoint's temporary socket-only
        # initialization server as ready.
        wait_for 'MySQL 8.4' docker exec "$container_prefix-mysql" \
          mysqladmin --protocol=tcp -h 127.0.0.1 -uroot -ppassword ping --silent
        echo 'WINDWATCHER_TEST_MYSQL_URL=mysql://root:password@127.0.0.1:3306/mysql' >> "$GITHUB_ENV"
        ;;
      mongodb)
        wait_for 'MongoDB 8.2' docker exec "$container_prefix-mongodb" mongosh --quiet \
          --eval 'quit(db.adminCommand({ping: 1}).ok === 1 ? 0 : 1)'
        timeout 30s docker exec "$container_prefix-mongodb" mongosh --quiet --eval \
          'const result = rs.initiate({_id: "rs0", members: [{_id: 0, host: "127.0.0.1:27017"}]}); if (result.ok !== 1) { throw new Error(JSON.stringify(result)); }'
        wait_for 'MongoDB writable primary' docker exec "$container_prefix-mongodb" mongosh --quiet \
          --eval 'quit(db.hello().isWritablePrimary === true ? 0 : 1)'
        echo 'WINDWATCHER_TEST_MONGODB_URL=mongodb://127.0.0.1:27017/?replicaSet=rs0' >> "$GITHUB_ENV"
        ;;
      redis)
        wait_for 'Redis 7' docker exec "$container_prefix-redis" redis-cli ping
        echo 'WINDWATCHER_TEST_REDIS_URL=redis://127.0.0.1:6379/0' >> "$GITHUB_ENV"
        ;;
    esac
  done
}

case "${1:-}" in
  start)
    shift
    [[ $# -gt 0 ]] || { echo 'Specify at least one CI backend' >&2; exit 1; }
    : "${GITHUB_ENV:?Set GITHUB_ENV to the environment output file}"
    start "$@"
    ;;
  logs) logs ;;
  stop)
    while read -r container; do
      [[ -n "$container" ]] || continue
      docker rm -fv "$container"
    done < <(containers)
    ;;
  *) echo 'Usage: services.sh {start BACKEND...|logs|stop}' >&2; exit 1 ;;
esac
