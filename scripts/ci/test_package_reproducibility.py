#!/usr/bin/env python3
"""Repackage identical binary bytes with different paths/mtimes using real nFPM."""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def rpm_header(file):
    header = file.read(16)
    assert header[:4] == b"\x8e\xad\xe8\x01", "Invalid RPM header"
    count, size = struct.unpack(">II", header[8:])
    entries = [struct.unpack(">IIII", file.read(16)) for _ in range(count)]
    data = file.read(size)
    return {tag: (kind, data[offset:], count) for tag, kind, offset, count in entries}


def verify_metadata(deb, rpm, epoch):
    # Check real archive metadata too: two builds within one clock tick must not
    # accidentally pass if SOURCE_DATE_EPOCH or the stable RPM host is removed.
    with deb.open("rb") as file:
        assert file.read(8) == b"!<arch>\n", "Invalid DEB archive"
        while header := file.read(60):
            assert len(header) == 60 and header[-2:] == b"`\n", "Invalid ar header"
            assert int(header[16:28]) == epoch, "DEB member timestamp is not commit time"
            size = int(header[48:58])
            file.seek(size + size % 2, os.SEEK_CUR)
    with rpm.open("rb") as file:
        file.seek(96)  # RPM lead, followed by signature and main headers.
        rpm_header(file)
        file.seek((-file.tell()) % 8, os.SEEK_CUR)
        fields = rpm_header(file)
    assert fields[1006][0] == 4  # RPMTAG_BUILDTIME, uint32.
    assert struct.unpack(">I", fields[1006][1][:4])[0] == epoch, "RPM build time is not commit time"
    assert fields[1007][1].split(b"\0", 1)[0] == b"windwatcher-release", "Unstable RPM build host"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("arch", choices=("x86_64", "aarch64"))
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    epoch = int(subprocess.check_output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=ROOT, text=True))
    env = {**os.environ, "SOURCE_DATE_EPOCH": str(epoch), "PACKAGE_RELEASE": "1", "CARGO_BUILD_JOBS": "4"}
    with tempfile.TemporaryDirectory(prefix="windwatcher-package-reproducibility-") as tmp:
        builds = []
        for index in range(2):
            directory = Path(tmp) / str(index)
            directory.mkdir()
            candidate = directory / "windwatcher"
            shutil.copyfile(binary, candidate)
            candidate.chmod(0o755)
            # Fresh checkouts/builds need not have matching source mtimes.
            os.utime(candidate, (epoch + 86400 * (index + 1),) * 2)
            outputs = {}
            for format in ("deb", "rpm"):
                output = subprocess.check_output([str(ROOT / "scripts/package.sh"), format, args.arch,
                                                  str(candidate), str(directory)], cwd=ROOT, env=env, text=True)
                outputs[format] = Path(output.strip())
            verify_metadata(outputs["deb"], outputs["rpm"], epoch)
            builds.append(outputs)
        for format in ("deb", "rpm"):
            first, second = (build[format] for build in builds)
            subprocess.check_call(["cmp", str(first), str(second)])
            print(f"Reproducible {format}: {hashlib.sha256(first.read_bytes()).hexdigest()}")


if __name__ == "__main__":
    main()
