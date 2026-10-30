#!/usr/bin/env python3
"""Validate the exact public asset inventory and its SHA-256 sidecars."""
import argparse
import hashlib
from pathlib import Path
from release import require, version


def assets(requested):
    return sorted(
        [f"windwatcher-server-{requested}-linux-{arch}.tar.gz" for arch in ("x86_64", "aarch64")]
        + [f"windwatcher-server_{requested}-1_{arch}.deb" for arch in ("amd64", "arm64")]
        + [f"windwatcher-server-{requested}-1.{arch}.rpm" for arch in ("x86_64", "aarch64")]
    )


def validate(root, requested, create=False):
    primary = assets(requested)
    expected = set(primary + [p + ".sha256" for p in primary])
    if not create:
        expected.add("SHA256SUMS")
    # Canonical binaries stay internal and are never uploaded to GitHub Releases.
    actual = {p.name for p in root.iterdir()}
    require(actual == expected, f"Unexpected/missing assets: {actual ^ expected}")
    lines = []
    for name in primary:
        digest = hashlib.sha256((root / name).read_bytes()).hexdigest()
        line = f"{digest}  {name}\n"
        require((root / (name + ".sha256")).read_text() == line, "Checksum mismatch: " + name)
        lines.append(line)
    aggregate = "".join(lines)
    if create:
        (root / "SHA256SUMS").write_text(aggregate)
    else:
        require((root / "SHA256SUMS").read_text() == aggregate, "Invalid aggregate checksums")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("directory", type=Path)
    parser.add_argument("--create", action="store_true")
    args = parser.parse_args()
    version(args.version)
    validate(args.directory, args.version, args.create)
