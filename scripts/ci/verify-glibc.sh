#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 4 ]]; then
    echo "Usage: $0 <binary> <baseline-file> <deb-depends> <rpm-requires>" >&2
    exit 2
fi
binary=$1 baseline=$(<"$2") deb_depends=$3 rpm_requires=$4
if [[ ! $baseline =~ ^[0-9]+(\.[0-9]+)+$ ]]; then
    echo 'Invalid glibc baseline' >&2; exit 1
fi

# Check actual package metadata, not just the nFPM inputs. Reject alternatives,
# unversioned or duplicate libc requirements rather than guessing their meaning.
deb_matches=0
IFS=',' read -ra dependencies <<< "$deb_depends"
for dependency in "${dependencies[@]}"; do
    dependency=${dependency#"${dependency%%[![:space:]]*}"}
    dependency=${dependency%"${dependency##*[![:space:]]}"}
    if [[ $dependency == libc6* ]]; then
        ((deb_matches+=1))
        if [[ ! $dependency =~ ^libc6[[:space:]]\(\>=[[:space:]]([0-9]+(\.[0-9]+)+)\)$ ]] || [[ ${BASH_REMATCH[1]} != "$baseline" ]]; then
            echo "Unexpected DEB libc6 requirement: $dependency" >&2; exit 1
        fi
    fi
done
rpm_matches=0
while IFS= read -r dependency; do
    if [[ $dependency == glibc || $dependency == glibc\ * ]]; then
        ((rpm_matches+=1))
        if [[ ! $dependency =~ ^glibc[[:space:]]\>=[[:space:]]([0-9]+(\.[0-9]+)+)$ ]] || [[ ${BASH_REMATCH[1]} != "$baseline" ]]; then
            echo "Unexpected RPM glibc requirement: $dependency" >&2; exit 1
        fi
    fi
done <<< "$rpm_requires"
if (( deb_matches != 1 || rpm_matches != 1 )); then
    echo 'Expected exactly one versioned libc requirement in each package' >&2; exit 1
fi

# Only version NEEDS, not definitions or the version-symbol table, represent
# imported ABI requirements. Force the C locale for stable readelf labels.
versions=$(LC_ALL=C readelf --wide --version-info "$binary" | awk '
    /^Version needs section/ { needs=1; next }
    /^Version .* section/ { needs=0 }
    needs && /Name: GLIBC_/ {
        for (i=1; i<NF; i++) if ($i == "Name:") print $(i+1)
    }
')
if [[ -z $versions ]]; then echo 'No GLIBC version requirements found in ELF' >&2; exit 1; fi
while IFS= read -r version; do
    if [[ ! $version =~ ^GLIBC_[0-9]+(\.[0-9]+)+$ ]]; then
        echo "Unsupported GLIBC version requirement: $version" >&2; exit 1
    fi
done <<< "$versions"
maximum=$(printf '%s\n' "$versions" | sed 's/^GLIBC_//' | LC_ALL=C sort -V | tail -n 1)
if [[ $(printf '%s\n' "$maximum" "$baseline" | LC_ALL=C sort -V | tail -n 1) != "$baseline" ]]; then
    echo "ELF requires GLIBC_$maximum, newer than package minimum $baseline" >&2; exit 1
fi
echo "ELF requires at most GLIBC_$maximum; both packages require glibc >= $baseline."
