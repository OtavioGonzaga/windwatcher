#!/usr/bin/env python3
"""Validate the exact public asset inventory and its SHA-256 sidecars."""
import argparse
import hashlib
from pathlib import Path
import shutil
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


def collect(source, destination, requested):
    # Never merge Actions artifacts blindly: duplicate architecture/filenames fail.
    architectures = {"x86_64": "amd64", "aarch64": "arm64"}
    require({p.name for p in source.iterdir()} == {"windwatcher-server-dist-" + a for a in architectures},
            "Expected exactly two native architecture artifacts")
    destination.mkdir(parents=True, exist_ok=True)
    require(not any(destination.iterdir()), "Public asset destination must be empty")
    for arch, debarch in architectures.items():
        root = source / ("windwatcher-server-dist-" + arch)
        primary = [f"windwatcher-server-{requested}-linux-{arch}.tar.gz",
                   f"windwatcher-server_{requested}-1_{debarch}.deb",
                   f"windwatcher-server-{requested}-1.{arch}.rpm"]
        public = primary + [p + ".sha256" for p in primary]
        expected = set(public + ["windwatcher-" + arch])
        require({p.name for p in root.iterdir()} == expected, "Wrong architecture artifact contents: " + arch)
        for name in public:
            require(not (destination / name).exists(), "Duplicate public asset: " + name)
            shutil.copyfile(root / name, destination / name)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("directory", type=Path)
    parser.add_argument("--create", action="store_true")
    parser.add_argument("--collect", type=Path)
    args = parser.parse_args()
    version(args.version)
    if args.collect:
        collect(args.collect, args.directory, args.version)
    validate(args.directory, args.version, args.create)
