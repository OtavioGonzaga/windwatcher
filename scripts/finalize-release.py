#!/usr/bin/env python3
"""Draft-first GitHub finalization with byte-checked, non-clobbering recovery."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

from release import require, version, release_body, PACKAGES, run
spec = importlib.util.spec_from_file_location("artifacts", Path(__file__).with_name("validate-release-artifacts.py"))
artifacts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(artifacts)


def gh(*args):
    return subprocess.check_output(["gh", *args], text=True).strip()


def synchronize_assets(tag, release, directory):
    expected = sorted(p.name for p in directory.iterdir()) if directory else []
    existing = [a["name"] for a in release["assets"]]
    require(len(existing) == len(set(existing)) and set(existing) <= set(expected), "Unexpected release assets")
    if not release["draft"]:
        require(set(existing) == set(expected), "Published release is missing immutable assets")
    for name in expected:
        if name not in existing:
            subprocess.check_call(["gh", "release", "upload", tag, str(directory / name)])
    for name in expected:
        with tempfile.TemporaryDirectory() as tmp:
            subprocess.check_call(["gh", "release", "download", tag, "--pattern", name, "--dir", tmp])
            require((Path(tmp) / name).read_bytes() == (directory / name).read_bytes(), "Published asset differs: " + name)
    return expected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", choices=PACKAGES)
    parser.add_argument("version")
    parser.add_argument("revision")
    parser.add_argument("--assets", type=Path)
    args = parser.parse_args()
    version(args.version)
    package, requested, sha = args.package, args.version, args.revision
    require(run(".", "git", "rev-parse", "HEAD") == sha, "Checkout is not release commit")
    tag = f"{package}-v{requested}"
    # Fetch errors (including permissions) are fatal; existing tags are never moved.
    subprocess.check_call(["git", "fetch", "origin", "main", "--tags"])
    state = run(".", "scripts/release-state.sh", package, requested)
    require(f"release_commit={sha}" in state.splitlines(), "Invalid release recovery context")
    if subprocess.run(["git", "show-ref", "--verify", "--quiet", "refs/tags/" + tag]).returncode:
        subprocess.check_call(["git", "config", "user.name", "github-actions[bot]"])
        subprocess.check_call(["git", "config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com"])
        subprocess.check_call(["git", "tag", "-a", tag, sha, "-m", f"{package} v{requested}"])
        subprocess.check_call(["git", "push", "origin", "refs/tags/" + tag])
    notes = release_body(Path(f"crates/{package}/CHANGELOG.md").read_text(), requested)
    if package == "windwatcher":
        require(args.assets is None, "Library release cannot have server assets")
        notes += f"\n\n## Installation\n\n```bash\ncargo add windwatcher@{requested}\n```\n"
    else:
        require(args.assets, "Server assets required")
        artifacts.validate(args.assets, requested)
        notes += (f"\n\n## Installation\n\n```bash\ncargo install windwatcher-server --version {requested}\n"
                  f"podman pull ghcr.io/otaviogonzaga/windwatcher-server:{requested}\n```\n\n"
                  "Download the matching architecture's DEB/RPM or tarball below. Verify downloads with "
                  "`sha256sum -c SHA256SUMS` (download all six primary assets first).\n")
    # API failures other than a confirmed 404 are fatal.
    repo = gh("repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner")
    result = subprocess.run(["gh", "api", f"repos/{repo}/releases/tags/{tag}"], text=True, capture_output=True)
    if result.returncode:
        require("(HTTP 404)" in result.stderr, result.stderr)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "notes.md"
            path.write_text(notes)
            command = ["gh", "release", "create", tag, "--verify-tag", "--draft", "--title", f"{package} v{requested}", "--notes-file", str(path)]
            if "-" in requested:
                command.append("--prerelease")
            subprocess.check_call(command)
    release = json.loads(gh("api", f"repos/{repo}/releases/tags/{tag}"))
    require(release["tag_name"] == tag and release["prerelease"] == ("-" in requested), "Release metadata mismatch")
    expected = synchronize_assets(tag, release, args.assets)
    completed = json.loads(gh("api", f"repos/{repo}/releases/tags/{tag}"))
    require(sorted(a["name"] for a in completed["assets"]) == expected, "Release asset inventory incomplete")
    if release["draft"]:
        # Notes are curated, not generated from commits. Do not edit published releases.
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "notes.md"
            path.write_text(notes)
            subprocess.check_call(["gh", "release", "edit", tag, "--notes-file", str(path), "--draft=false"])


if __name__ == "__main__":
    main()
