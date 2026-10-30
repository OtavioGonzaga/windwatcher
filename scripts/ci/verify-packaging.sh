#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 4 ]]; then echo "Usage: $0 <deb> <rpm> <x86_64|aarch64> <binary>" >&2; exit 2; fi
deb=$(realpath "$1") rpm=$(realpath "$2") arch=$3 binary=$(realpath "$4")
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
version=$(cargo metadata --manifest-path "$root/Cargo.toml" --no-deps --format-version 1 --locked | jq -er '.packages[] | select(.name == "windwatcher-server") | .version')
case $arch in x86_64) debarch=amd64;; aarch64) debarch=arm64;; *) exit 2;; esac
case $arch in
    x86_64) readelf -h "$binary" | grep -Eq 'Machine:.*X86-64';;
    aarch64) readelf -h "$binary" | grep -Eq 'Machine:.*AArch64';;
esac
[[ $(dpkg-deb -f "$deb" Package) == windwatcher-server ]]
[[ $(dpkg-deb -f "$deb" Version) == "$version-1" ]]
[[ $(dpkg-deb -f "$deb" Architecture) == "$debarch" ]]
dpkg-deb -f "$deb" Depends | grep -Eq '(^|[, ])systemd([, ]|$)'
[[ $(rpm -qp --qf '%{NAME} %{VERSION} %{RELEASE} %{ARCH} %{LICENSE}' "$rpm") == "windwatcher-server $version 1 $arch MIT" ]]
rpm -qp --requires "$rpm" | grep -qx systemd
"$root/scripts/ci/verify-glibc.sh" "$binary" "$root/packaging/glibc-baseline" \
    "$(dpkg-deb -f "$deb" Depends)" "$(rpm -qp --requires "$rpm")"
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
mkdir "$tmp/deb" "$tmp/rpm"
dpkg-deb -x "$deb" "$tmp/deb"
dpkg-deb -e "$deb" "$tmp/control"
for script in postinst prerm postrm; do [[ -x $tmp/control/$script ]]; done
cmp "$root/packaging/scripts/postinstall.sh" "$tmp/control/postinst"
cmp "$root/packaging/scripts/preremove.sh" "$tmp/control/prerm"
cmp "$root/packaging/scripts/postremove.sh" "$tmp/control/postrm"
dpkg-deb --fsys-tarfile "$deb" | tar -tvf - | grep -E '^-rwxr-xr-x +root/root +[0-9]+ .*\./usr/bin/windwatcher$' >/dev/null
rpm -qp --qf '[%{FILENAMES} %{FILEUSERNAME} %{FILEGROUPNAME} %{FILEMODES:perms}\n]' "$rpm" | grep -x '/usr/bin/windwatcher root root -rwxr-xr-x' >/dev/null
rpm -qp --scripts "$rpm" > "$tmp/rpm-scripts"
grep -Fq 'systemd-sysusers /usr/lib/sysusers.d/windwatcher.conf' "$tmp/rpm-scripts"
grep -Fq 'systemctl stop windwatcher.service windwatcher-worker.service' "$tmp/rpm-scripts"
(cd "$tmp/rpm" && rpm2cpio "$rpm" | cpio -idm --quiet)
for format in deb rpm; do
    dir=$tmp/$format
    case $format in
        deb) license=usr/share/doc/windwatcher-server/copyright;;
        rpm) license=usr/share/licenses/windwatcher-server/LICENSE;;
    esac
    printf '%s\n' \
        usr/bin/windwatcher \
        usr/lib/systemd/system/windwatcher.service \
        usr/lib/systemd/system/windwatcher-worker.service \
        usr/lib/sysusers.d/windwatcher.conf \
        usr/lib/tmpfiles.d/windwatcher.conf \
        usr/share/doc/windwatcher-server/config.example.toml \
        "$license" | LC_ALL=C sort > "$tmp/expected"
    (cd "$dir" && find . -type f -printf '%P\n' | LC_ALL=C sort) > "$tmp/actual"
    diff -u "$tmp/expected" "$tmp/actual"
    cmp "$binary" "$dir/usr/bin/windwatcher"
    [[ $(stat -c '%a' "$dir/usr/bin/windwatcher") == 755 ]]
    for unit in windwatcher.service windwatcher-worker.service; do
        cmp "$root/packaging/systemd/$unit" "$dir/usr/lib/systemd/system/$unit"
    done
    cmp "$root/packaging/sysusers/windwatcher.conf" "$dir/usr/lib/sysusers.d/windwatcher.conf"
    cmp "$root/packaging/tmpfiles/windwatcher.conf" "$dir/usr/lib/tmpfiles.d/windwatcher.conf"
    cmp "$root/config.example.toml" "$dir/usr/share/doc/windwatcher-server/config.example.toml"
    for path in etc/windwatcher/config.toml etc/windwatcher/jwt.key var/lib/windwatcher/windwatcher.db var/lib/windwatcher/queue.db etc/systemd/system; do
        [[ ! -e $dir/$path ]]
    done
done
cmp "$root/LICENSE" "$tmp/deb/usr/share/doc/windwatcher-server/copyright"
cmp "$root/LICENSE" "$tmp/rpm/usr/share/licenses/windwatcher-server/LICENSE"
echo 'Package metadata and payload verified.'
