#!/usr/bin/env python3
"""Native Podman publication; immutable exact tags are verified, never replaced."""
import argparse
import json
import subprocess
from release import require, version

IMAGE = "ghcr.io/otaviogonzaga/windwatcher-server"


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def inspect(reference, raw=False):
    args = ["skopeo", "inspect"] + (["--raw"] if raw else []) + ["docker://" + reference]
    result = subprocess.run(args, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        # Authentication, rate limiting and transport errors must never mean 'absent'.
        require("manifest unknown" in result.stderr.lower() or "name unknown" in result.stderr.lower(), result.stderr)
        return None
    return json.loads(result.stdout)


def architecture(reference, arch, requested, revision):
    metadata = inspect(reference)
    if metadata is None:
        return None
    require(metadata["Os"] == "linux" and metadata["Architecture"] == arch, "Wrong image platform")
    labels = metadata["Labels"]
    require(labels["org.opencontainers.image.version"] == requested, "Wrong image version")
    require(labels["org.opencontainers.image.revision"] == revision, "Wrong image revision")
    return metadata["Digest"]


def platforms(manifest):
    require(manifest is not None, "Manifest not found")
    entries = manifest.get("manifests", [])
    require(len(entries) == 2, "Manifest must contain exactly two platforms")
    result = {(m["platform"]["os"], m["platform"]["architecture"]): m["digest"] for m in entries}
    require(len(result) == 2, "Duplicate platform")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("architecture", "manifest"))
    parser.add_argument("version")
    parser.add_argument("revision")
    parser.add_argument("--arch", choices=("amd64", "arm64"))
    args = parser.parse_args()
    version(args.version)
    exact = IMAGE + ":" + args.version
    if args.mode == "architecture":
        require(args.arch, "Architecture required")
        reference = exact + "-" + args.arch
        digest = architecture(reference, args.arch, args.version, args.revision)
        if digest is not None:
            print("Reusing " + reference + "@" + digest)
            return
        subprocess.check_call(["podman", "build", "--format=oci", "--file", "Containerfile", "--tag", reference,
                               "--build-arg", "VERSION=" + args.version, "--build-arg", "REVISION=" + args.revision, "."])
        subprocess.check_call(["scripts/ci/verify-oci.sh", reference, args.arch, args.version, args.revision])
        subprocess.check_call(["scripts/ci/test-oci.sh", reference])
        subprocess.check_call(["podman", "push", "--format", "oci", reference])
        require(architecture(reference, args.arch, args.version, args.revision), "Published architecture missing")
        return
    expected = {}
    for arch in ("amd64", "arm64"):
        digest = architecture(exact + "-" + arch, arch, args.version, args.revision)
        require(digest, "Missing architecture: " + arch)
        expected[("linux", arch)] = digest
    remote = inspect(exact, raw=True)
    local = "localhost/windwatcher-release:" + args.version
    subprocess.check_call(["podman", "manifest", "create", local])
    for digest in expected.values():
        subprocess.check_call(["podman", "manifest", "add", local, "docker://" + IMAGE + "@" + digest])
    if remote is None:
        subprocess.check_call(["podman", "manifest", "push", "--all", "--format", "oci", local, "docker://" + exact])
    require(platforms(inspect(exact, raw=True)) == expected, "Existing exact manifest differs from release architectures")
    if "-" not in args.version:
        for alias in (".".join(args.version.split(".")[:2]), "latest"):
            ref = IMAGE + ":" + alias
            existing = inspect(ref)
            if existing:
                old = existing["Labels"]["org.opencontainers.image.version"]
                # Recovering an older release must not roll stable aliases back.
                if version(old) > version(args.version):
                    continue
                if old == args.version:
                    require(platforms(inspect(ref, raw=True)) == expected, "Alias version has unexpected platform digests")
                    continue
            subprocess.check_call(["podman", "manifest", "push", "--all", "--format", "oci", local, "docker://" + ref])
            require(platforms(inspect(ref, raw=True)) == expected, "Alias validation failed")


if __name__ == "__main__":
    main()
