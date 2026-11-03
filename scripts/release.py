#!/usr/bin/env python3
"""Package-aware release metadata tooling (Python standard library only).

Manifest edits deliberately recognize only the repository's explicit layout.
Cargo validates the result; unsupported layouts fail rather than guessing.
"""
import argparse
import datetime
import os
from pathlib import Path
import re
import subprocess
import sys

PACKAGES = ("windwatcher", "windwatcher-server")
SEMVER = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?")
REPOSITORY = "https://github.com/OtavioGonzaga/windwatcher"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def version(value):
    match = SEMVER.fullmatch(value)
    require(match, "Invalid SemVer (no v prefix or build metadata): " + value)
    pre = match[4]
    require(not pre or all(not p.isdigit() or p == "0" or not p.startswith("0") for p in pre.split(".")),
            "Leading-zero numeric prerelease identifier")
    # Numeric identifiers sort before alphanumeric identifiers; stable sorts last.
    suffix = (1,) if pre is None else (0, tuple((0, int(p)) if p.isdigit() else (1, p) for p in pre.split(".")))
    return tuple(map(int, match.group(1, 2, 3))) + (suffix,)


def run(root, *args):
    return subprocess.check_output(args, cwd=root, text=True).strip()


def manifest_path(package):
    return f"crates/{package}/Cargo.toml"


def field(text, section, key):
    body = re.search(r"(?ms)^\[" + re.escape(section) + r"\]\n(.*?)(?=^\[|\Z)", text)
    require(body, "Missing manifest section: " + section)
    found = re.findall(r"(?m)^" + re.escape(key) + r'\s*=\s*"([^"\n]+)"', body[1])
    require(len(found) == 1, "Expected one explicit manifest field: " + key)
    return found[0]


def package_version(text):
    return field(text, "package", "version")


def library_requirement(text):
    matches = re.findall(r'(?m)^windwatcher = \{[^\n]*version = "([^"\n]+)"', text)
    require(len(matches) == 1, "Expected explicit path/version library dependency")
    version(matches[0])
    return matches[0]


def replace_version(text, old, new):
    result, count = re.subn(r'(?m)^(version\s*=\s*)"' + re.escape(old) + '"',
                            lambda m: m[1] + '"' + new + '"', text, count=1)
    require(count == 1, "Cannot update package version")
    return result


def release_body(text, requested):
    matches = list(re.finditer(r"(?m)^## \[" + re.escape(requested) + r"\] - \d{4}-\d{2}-\d{2}\s*$", text))
    require(len(matches) == 1, "Expected one dated changelog section for " + requested)
    body = re.split(r"(?m)^## |^\[[^\]]+\]:", text[matches[0].end():], maxsplit=1)[0].strip()
    require(re.search(r"(?m)^[-*] \S", body), "Empty release notes")
    return body


def allowed(package):
    files = {"Cargo.lock", manifest_path(package), f"crates/{package}/CHANGELOG.md"}
    if package == "windwatcher":
        files.add(manifest_path("windwatcher-server"))
    return files


def check_diff(root, package, base):
    changed = set(run(root, "git", "diff", "--name-only", base, "--").splitlines())
    untracked = run(root, "git", "ls-files", "--others", "--exclude-standard")
    require(not untracked, "Untracked files in release checkout")
    require(changed <= allowed(package), "Unexpected release files: " + str(changed - allowed(package)))


def lock_packages(text):
    return re.split(r"(?m)^\[\[package\]\]\n", text)[1:]


def bump_lock(text, package, old, new):
    blocks = lock_packages(text)
    found = 0
    for i, block in enumerate(blocks):
        if re.match(r'name = "' + re.escape(package) + r'"\n', block) and not re.search(r"(?m)^source =", block):
            blocks[i] = replace_version(block, old, new)
            found += 1
    require(found == 1, "Expected one workspace-local lock entry")
    return text.split("[[package]]\n", 1)[0] + "".join("[[package]]\n" + b for b in blocks)


def bump_requirement(text, requested):
    old = library_requirement(text)
    return re.sub(r'(?m)^(windwatcher = \{[^\n]*version = ")' + re.escape(old) + '"',
                  lambda m: m[1] + requested + '"', text)


def prepare(args):
    root, package, requested = args.root, args.package, args.version
    path = root / manifest_path(package)
    manifest = path.read_text()
    current = package_version(manifest)
    require(version(requested) >= version(current), "Release version would downgrade package")
    date = args.date or datetime.datetime.now(datetime.timezone.utc).date().isoformat()
    require(datetime.date.fromisoformat(date).isoformat() == date, "Invalid date")
    changelog_path = root / f"crates/{package}/CHANGELOG.md"
    changelog = changelog_path.read_text()
    require(len(re.findall(r"(?m)^## (?:\[Unreleased\]|Unreleased)\s*$", changelog)) == 1
            and "## [Unreleased]\n" in changelog, "Expected exactly one canonical Unreleased section")
    require(not re.search(r"(?m)^## \[" + re.escape(requested) + r"\]", changelog), "Version already in changelog")
    start = changelog.index("## [Unreleased]\n") + len("## [Unreleased]\n")
    tail = changelog[start:]
    body = re.split(r"(?m)^## |^\[[^\]]+\]:", tail, maxsplit=1)[0]
    require(re.search(r"(?m)^[-*] \S", body), "Unreleased has no release-worthy entries")
    previous = re.findall(r"(?m)^## \[([^\]]+)\] - \d{4}-\d{2}-\d{2}", changelog)
    promoted = changelog[:start] + f"\n## [{requested}] - {date}\n" + tail
    promoted = re.sub(r"(?m)^\[(?:Unreleased|" + re.escape(requested) + r")\]:.*\n?", "", promoted).rstrip()
    tag = f"{package}-v{requested}"
    link = f"{REPOSITORY}/compare/{package}-v{previous[0]}...{tag}" if previous else f"{REPOSITORY}/releases/tag/{tag}"
    promoted += f"\n\n[Unreleased]: {REPOSITORY}/compare/{tag}...HEAD\n[{requested}]: {link}\n"
    lock_path = root / "Cargo.lock"
    original_lock = lock_path.read_text()
    updated_lock = bump_lock(original_lock, package, current, requested)
    edits = {path: replace_version(manifest, current, requested), changelog_path: promoted, lock_path: updated_lock}
    if package == "windwatcher":
        server_path = root / manifest_path("windwatcher-server")
        server = server_path.read_text()
        edits[server_path] = bump_requirement(server, requested)
    # Roll back our own edits if Cargo rejects the candidate; never update dependencies.
    originals = {p: p.read_text() for p in edits}
    try:
        for p, content in edits.items():
            p.write_text(content)
        run(root, "cargo", "metadata", "--format-version", "1", "--locked")
        require(lock_path.read_text() == updated_lock, "Cargo changed lockfile unexpectedly")
    except Exception:
        for p, content in originals.items():
            p.write_text(content)
        raise
    print(f"Prepared {package} {requested}")


def state(args):
    root, package, requested = args.root, args.package, args.version
    subject = f"chore(release): {package} v{requested}"
    commits = run(root, "git", "log", args.main_ref, "--format=%H%x09%s").splitlines()
    candidates = [line.split("\t")[0] for line in commits if line.split("\t", 1)[1] == subject]
    require(len(candidates) <= 1, "Multiple release commits found")
    tag = f"{package}-v{requested}"
    exists = subprocess.run(["git", "show-ref", "--verify", "--quiet", "refs/tags/" + tag], cwd=root).returncode == 0
    if not candidates:
        require(not exists, "Tag exists without a reachable release commit")
        print("prepared=false")
        return
    sha = candidates[0]
    show = lambda path: run(root, "git", "show", f"{sha}:{path}")
    require(package_version(show(manifest_path(package))) == requested, "Release commit has wrong version")
    release_body(show(f"crates/{package}/CHANGELOG.md"), requested)
    if package == "windwatcher":
        require(library_requirement(show(manifest_path("windwatcher-server"))) == requested, "Wrong server dependency")
    parents = run(root, "git", "rev-list", "--parents", "-n", "1", sha).split()
    require(len(parents) == 2, "Release must have exactly one parent")
    before = lambda path: run(root, "git", "show", f"{parents[1]}:{path}")
    old_manifest = before(manifest_path(package))
    old_version = package_version(old_manifest)
    require(version(requested) >= version(old_version), "Release commit downgraded package")
    require(show(manifest_path(package)) == replace_version(old_manifest, old_version, requested),
            "Release manifest contains non-version changes")
    require(show("Cargo.lock") == bump_lock(before("Cargo.lock"), package, old_version, requested),
            "Release changed unrelated lockfile metadata/dependencies")
    if package == "windwatcher":
        server_path = manifest_path("windwatcher-server")
        require(show(server_path) == bump_requirement(before(server_path), requested),
                "Library release changed unrelated server manifest metadata")
    changed = set(run(root, "git", "diff", "--name-only", parents[1], sha).splitlines())
    require(changed <= allowed(package), "Release commit violates changed-file allow-list")
    if exists:
        require(run(root, "git", "cat-file", "-t", "refs/tags/" + tag) == "tag", "Release tag is not annotated")
        require(run(root, "git", "rev-parse", tag + "^{commit}") == sha, "Release tag points to wrong commit")
    print(f"prepared=true\nrelease_commit={sha}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "state", "notes", "allow-list", "dependency", "validate-version"))
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--date")
    parser.add_argument("--main-ref", default="origin/main")
    parser.add_argument("package", choices=PACKAGES)
    parser.add_argument("version")
    args = parser.parse_args()
    version(args.version)
    if args.command == "prepare":
        prepare(args)
    elif args.command == "state":
        state(args)
    elif args.command == "notes":
        print(release_body((args.root / f"crates/{args.package}/CHANGELOG.md").read_text(), args.version))
    elif args.command == "allow-list":
        check_diff(args.root, args.package, "HEAD")
    elif args.command == "dependency":
        print(library_requirement((args.root / manifest_path("windwatcher-server")).read_text()))


if __name__ == "__main__":
    os.environ.setdefault("CARGO_BUILD_JOBS", "4")
    try:
        main()
    except (ValueError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
