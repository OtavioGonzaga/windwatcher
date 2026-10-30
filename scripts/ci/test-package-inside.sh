#!/usr/bin/env bash
set -euo pipefail
distro=$1
case $distro in
    debian) ext=deb;;
    fedora) ext=rpm;;
    *) exit 2;;
esac
install_package() {
    case $distro in
        debian) DEBIAN_FRONTEND=noninteractive apt-get install -y "$1";;
        fedora) dnf install -y "$1";;
    esac
}
upgrade_package() {
    case $distro in
        debian) DEBIAN_FRONTEND=noninteractive apt-get install -y "$1";;
        fedora) dnf upgrade -y "$1";;
    esac
}
as_windwatcher() {
    sudo -u windwatcher -- sh -c 'cd /var/lib/windwatcher && exec /usr/bin/windwatcher --system "$@"' sh "$@"
}
assert_unmodified() {
    [[ $(sha256sum /etc/windwatcher/config.toml | cut -d' ' -f1) == "$config_hash" ]]
    [[ $(sha256sum /etc/windwatcher/jwt.key | cut -d' ' -f1) == "$key_hash" ]]
    [[ $(cat /var/lib/windwatcher/package-test-sentinel) == preserved ]]
    [[ -f /var/lib/windwatcher/windwatcher.db ]]
    [[ $(sudo -u windwatcher -- sqlite3 /var/lib/windwatcher/windwatcher.db 'SELECT value FROM package_test_sentinel;') == preserved ]]
}
wait_health() {
    for _ in {1..60}; do
        if curl -fs http://127.0.0.1:8080/health/live >/dev/null && curl -fs http://127.0.0.1:8080/health/ready >/dev/null; then return; fi
        sleep 1
    done
    echo 'Server health check timed out' >&2; return 1
}

# A dangling local unit must be reported but never removed by the package.
ln -s /nonexistent/local-worker /etc/systemd/system/windwatcher-worker.service
install_package "/packages/one.$ext" 2>&1 | tee /run/install-output
grep -F 'shadows the package unit' /run/install-output
[[ -L /etc/systemd/system/windwatcher-worker.service ]]
rm /etc/systemd/system/windwatcher-worker.service
systemctl daemon-reload

getent passwd windwatcher | grep -Eq '^windwatcher:[^:]*:[0-9]+:[0-9]+:[^:]*:/var/lib/windwatcher:/(usr/)?sbin/nologin$'
getent group windwatcher >/dev/null
[[ $(stat -c '%U:%G %a' /etc/windwatcher) == 'root:windwatcher 2750' ]]
[[ $(stat -c '%U:%G %a' /var/lib/windwatcher) == 'windwatcher:windwatcher 750' ]]
[[ -x /usr/bin/windwatcher ]]
for file in /usr/lib/systemd/system/windwatcher.service /usr/lib/systemd/system/windwatcher-worker.service /usr/lib/sysusers.d/windwatcher.conf /usr/lib/tmpfiles.d/windwatcher.conf /usr/share/doc/windwatcher-server/config.example.toml; do [[ -f $file ]]; done
[[ ! -e /etc/windwatcher/config.toml && ! -e /etc/windwatcher/jwt.key ]]
for unit in windwatcher.service windwatcher-worker.service; do
    ! systemctl is-active --quiet "$unit"
    ! systemctl is-enabled --quiet "$unit"
done

/usr/bin/windwatcher --system init
[[ $(stat -c '%U:%G %a' /etc/windwatcher/config.toml) == 'root:windwatcher 640' ]]
[[ $(stat -c '%U:%G %a' /etc/windwatcher/jwt.key) == 'root:windwatcher 640' ]]
as_windwatcher config check
as_windwatcher migrate
as_windwatcher doctor
sudo -u windwatcher -- sqlite3 /var/lib/windwatcher/windwatcher.db "CREATE TABLE package_test_sentinel(value TEXT); INSERT INTO package_test_sentinel VALUES('preserved');"
printf 'preserved\n' > /var/lib/windwatcher/package-test-sentinel
config_hash=$(sha256sum /etc/windwatcher/config.toml | cut -d' ' -f1)
key_hash=$(sha256sum /etc/windwatcher/jwt.key | cut -d' ' -f1)
systemctl enable --now windwatcher.service
wait_health
pid=$(systemctl show -p MainPID --value windwatcher.service)
[[ $pid != 0 ]]
[[ $(stat -c '%u' "/proc/$pid") == "$(id -u windwatcher)" ]]
journalctl -u windwatcher.service --no-pager | grep -q server_listening

upgrade_package "/packages/two.$ext"
case $distro in
    debian) [[ $(dpkg-query -W -f='${Version}' windwatcher-server) == *-2 ]];;
    fedora) [[ $(rpm -q --qf '%{RELEASE}' windwatcher-server) == 2 ]];;
esac
systemctl is-active --quiet windwatcher.service
systemctl is-enabled --quiet windwatcher.service
[[ $(systemctl show -p MainPID --value windwatcher.service) == "$pid" ]]
assert_unmodified
systemctl stop windwatcher.service
as_windwatcher config check
as_windwatcher migrate
as_windwatcher doctor
systemctl start windwatcher.service
wait_health
[[ $(systemctl show -p MainPID --value windwatcher.service) != "$pid" ]]
assert_unmodified

case $distro in
    debian) apt-get remove -y windwatcher-server; dpkg --purge windwatcher-server;;
    fedora) dnf remove -y windwatcher-server;;
esac
! systemctl is-active --quiet windwatcher.service
! systemctl is-enabled --quiet windwatcher.service
for file in /usr/bin/windwatcher /usr/lib/systemd/system/windwatcher.service /usr/lib/systemd/system/windwatcher-worker.service /usr/lib/sysusers.d/windwatcher.conf /usr/lib/tmpfiles.d/windwatcher.conf /usr/share/doc/windwatcher-server/config.example.toml /usr/share/licenses/windwatcher-server/LICENSE; do [[ ! -e $file ]]; done
getent passwd windwatcher >/dev/null
getent group windwatcher >/dev/null
assert_unmodified
install_package "/packages/two.$ext"
assert_unmodified
as_windwatcher doctor
echo "$distro package lifecycle passed."
