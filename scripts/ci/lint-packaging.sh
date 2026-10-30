#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 2 ]]; then echo "Usage: $0 <deb> <rpm>" >&2; exit 2; fi
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
deb_status=0
lintian --fail-on warning "$1" >"$tmp/deb" 2>&1 || deb_status=$?
cat "$tmp/deb"
(( deb_status <= 2 )) || exit "$deb_status"
# The exact exceptions and their rationale are documented in packaging/README.md.
grep -E '^[EFW]:' "$tmp/deb" | grep -Ev '^E: windwatcher-server: no-changelog usr/share/doc/windwatcher-server/changelog\.Debian\.gz \(non-native package\)$|^W: windwatcher-server: maintainer-script-calls-systemctl \[prerm:[0-9]+\]$|^W: windwatcher-server: no-manual-page \[?/?usr/bin/windwatcher\]?$' >"$tmp/deb-unexpected" || :
if [[ -s $tmp/deb-unexpected ]]; then cat "$tmp/deb-unexpected" >&2; exit 1; fi
if (( deb_status != 0 )) && ! grep -Eq '^[EW]:' "$tmp/deb"; then exit "$deb_status"; fi
rpm_status=0
rpmlint "$2" >"$tmp/rpm" 2>&1 || rpm_status=$?
cat "$tmp/rpm"
# rpmlint 2.x reports findings with exit code 64; other failures are errors.
if (( rpm_status != 0 && rpm_status != 64 )); then exit "$rpm_status"; fi
grep -E '^windwatcher-server\.[^:]+: [EFW]:' "$tmp/rpm" | grep -Ev '^windwatcher-server\.[^:]+: (W: only-non-binary-in-usr-lib|E: no-signature|E: no-group-tag|E: no-changelogname-tag|W: invalid-license MIT|E: explicit-lib-dependency libgcc|W: no-manual-page-for-binary windwatcher|W: tmpfile-not-in-filelist /(etc|var/lib)/windwatcher)$' >"$tmp/rpm-unexpected" || :
if [[ -s $tmp/rpm-unexpected ]]; then cat "$tmp/rpm-unexpected" >&2; exit 1; fi
if (( rpm_status != 0 )) && ! grep -Eq '^windwatcher-server\.[^:]+: [EW]:' "$tmp/rpm"; then exit "$rpm_status"; fi
