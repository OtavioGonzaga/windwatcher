#!/usr/bin/env python3
"""Check exact crates.io versions and verify published source revision on recovery."""
import argparse
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import urllib.error
import urllib.request

from release import require, version, PACKAGES


def get(url):
    request = urllib.request.Request(url, headers={"User-Agent": "windwatcher-release"})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.read()
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise


def exists(package, requested):
    result = get(f"https://crates.io/api/v1/crates/{package}/{requested}")
    if result is None:
        return False
    entry = json.loads(result)["version"]
    require(entry["num"] == requested and not entry["yanked"], "Registry version is wrong or yanked")
    return True


def verify(package, requested):
    archive = get(f"https://crates.io/api/v1/crates/{package}/{requested}/download")
    require(archive, "Published archive unavailable")
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as crate:
        member = crate.extractfile(f"{package}-{requested}/.cargo_vcs_info.json")
        require(member is not None, "Published crate has no VCS evidence")
        vcs = json.load(member)
        sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
        require(vcs["git"]["sha1"] == sha and not vcs["git"].get("dirty", False),
                "Published crate does not belong to this exact clean release commit")
        require(vcs["path_in_vcs"] == f"crates/{package}", "Wrong published package path")
        # Compare every packaged file, not only self-reported VCS metadata.
        local = Path(f"target/package/{package}-{requested}.crate")
        with tarfile.open(local, mode="r:gz") as candidate:
            def contents(archive):
                return {m.name: archive.extractfile(m).read() for m in archive.getmembers() if m.isfile()}
            require(contents(crate) == contents(candidate), "Published crate bytes differ from validated package")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("dependency", "publish"))
    parser.add_argument("package", choices=PACKAGES)
    parser.add_argument("version")
    args = parser.parse_args()
    version(args.version)
    present = exists(args.package, args.version)
    if args.command == "dependency":
        require(present, f"Release {args.package} {args.version} first: exact server dependency is not published to crates.io")
        return
    if not present:
        result = subprocess.run(["cargo", "publish", "-p", args.package, "--locked", "--registry", "crates-io"])
        # A successful upload can be followed by a client-side timeout.
        require(result.returncode == 0 or exists(args.package, args.version), "Cargo publication failed")
    verify(args.package, args.version)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, subprocess.CalledProcessError, urllib.error.URLError) as error:
        sys.exit(str(error))
