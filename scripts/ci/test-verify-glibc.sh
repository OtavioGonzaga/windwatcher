#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
cat > "$tmp/readelf" <<'EOF'
#!/bin/sh
cat "$ELF_VERSION_FIXTURE"
EOF
chmod +x "$tmp/readelf"
export PATH=$tmp:$PATH ELF_VERSION_FIXTURE=$tmp/versions
printf '2.35\n' > "$tmp/baseline"
deb='systemd, libc6 (>= 2.35), libgcc-s1'
rpm=$'systemd\nglibc >= 2.35\nlibgcc'
verify() { "$root/scripts/ci/verify-glibc.sh" /unused "$tmp/baseline" "$1" "$2"; }
reject() {
    if verify "$1" "$2" >"$tmp/output" 2>&1; then
        echo 'Expected glibc verification to fail' >&2; exit 1
    fi
}
cat > "$ELF_VERSION_FIXTURE" <<'EOF'
Version symbols section '.gnu.version' contains 1 entry:
  0: (GLIBC_9.99)
Version needs section '.gnu.version_r' contains 1 entry:
  0x0010: Name: GLIBC_2.9  Flags: none  Version: 2
  0x0020: Name: GLIBC_2.34  Flags: none  Version: 3
  0x0030: Name: GLIBC_2.35  Flags: none  Version: 4
EOF
verify "$deb" "$rpm"
reject 'systemd, libc6 (>= 2.34), libgcc-s1' "$rpm"
reject "$deb" $'systemd\nglibc >= 2.34\nlibgcc'
reject 'systemd, libc6, libgcc-s1' "$rpm"
reject 'systemd, libc6 (>= 2.35) | libc6-alt' "$rpm"
reject "$deb" $'systemd\nglibc >= 2.35\nglibc >= 2.35'
printf '  0x0040: Name: GLIBC_2.36  Flags: none  Version: 5\n' >> "$ELF_VERSION_FIXTURE"
reject "$deb" "$rpm"
printf 'Version needs section '\''.gnu.version_r'\'' contains 1 entry:\n  0x0010: Name: GLIBC_PRIVATE  Flags: none  Version: 2\n' > "$ELF_VERSION_FIXTURE"
reject "$deb" "$rpm"
printf 'Version symbols section '\''.gnu.version'\'' contains 1 entry:\n' > "$ELF_VERSION_FIXTURE"
reject "$deb" "$rpm"
echo 'glibc compatibility regression tests passed.'
