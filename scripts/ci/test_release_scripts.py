#!/usr/bin/env python3
"""Regression tests use only disposable Cargo workspaces and Git repositories."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tarfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import release


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


artifacts = load("artifacts", "validate-release-artifacts.py")
oci = load("oci", "publish-release-oci.py")
finalize = load("finalize", "finalize-release.py")
registry = load("registry", "registry-release.py")


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="windwatcher-release-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/windwatcher", "crates/windwatcher-server"]\nresolver = "3"\n')
        (self.root / ".gitignore").write_text("/target/\n")
        for package in release.PACKAGES:
            crate = self.root / "crates" / package
            (crate / "src").mkdir(parents=True)
            (crate / "src/lib.rs").write_text("pub fn example() {}\n")
            manifest = f'[package]\nname = "{package}"\nversion = "0.1.0"\nedition = "2024"\n'
            if package == "windwatcher-server":
                manifest += '\n[dependencies]\nwindwatcher = { path = "../windwatcher", version = "0.1.0" }\n'
            (crate / "Cargo.toml").write_text(manifest)
            (crate / "CHANGELOG.md").write_text(f"# Changelog\n\n## [Unreleased]\n\n### Added\n\n- Initial {package} behavior.\n")
        self.cmd("cargo", "generate-lockfile", "--offline")
        self.cmd("git", "init", "-b", "main")
        self.cmd("git", "config", "user.name", "Release Tests")
        self.cmd("git", "config", "user.email", "release-tests@example.invalid")
        self.commit("Initial fixture")
        self.initial = self.cmd("git", "rev-parse", "HEAD").strip()

    def cmd(self, *args, success=True):
        result = subprocess.run(args, cwd=self.root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        else:
            self.assertNotEqual(result.returncode, 0, result.stderr + result.stdout)
        return result.stdout

    def commit(self, subject):
        self.cmd("git", "add", ".")
        self.cmd("git", "commit", "-m", subject)

    def prepare(self, package="windwatcher", requested="0.2.0", success=True):
        return self.cmd(str(SCRIPTS / "prepare-release.sh"), package, requested, "--root", str(self.root), "--date", "2026-10-30", success=success)

    def state(self, package="windwatcher", requested="0.2.0", success=True):
        return self.cmd(str(SCRIPTS / "release-state.sh"), "--main-ref", "main", "--root", str(self.root), package, requested, success=success)

    def text(self, package, file="Cargo.toml"):
        return (self.root / "crates" / package / file).read_text()

    def test_semver_ordering(self):
        ordered = ["0.1.0", "0.2.0-alpha", "0.2.0-alpha.1", "0.2.0-alpha.2", "0.2.0-alpha.10", "0.2.0-beta", "0.2.0-rc.1", "0.2.0", "1.0.0"]
        self.assertEqual(sorted(ordered, key=release.version), ordered)
        for value in ("v0.1.0", "0.1", "01.2.3", "0.1.0+build", "0.2.0-rc.01", "0.1.0-01", "0.1.0-", "1.2.3\n", "1.2.3-α"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                release.version(value)

    def test_library_independent_bump(self):
        server_changelog = self.text("windwatcher-server", "CHANGELOG.md")
        self.prepare()
        self.assertEqual(release.package_version(self.text("windwatcher")), "0.2.0")
        self.assertEqual(release.package_version(self.text("windwatcher-server")), "0.1.0")
        self.assertEqual(release.library_requirement(self.text("windwatcher-server")), "0.2.0")
        self.assertEqual(self.text("windwatcher-server", "CHANGELOG.md"), server_changelog)
        self.assertIn("## [Unreleased]\n\n## [0.2.0] - 2026-10-30", self.text("windwatcher", "CHANGELOG.md"))
        self.assertIn("windwatcher-v0.2.0", self.text("windwatcher", "CHANGELOG.md"))
        self.cmd("cargo", "metadata", "--locked", "--format-version", "1", "--offline")
        self.assertIn('name = "windwatcher"\nversion = "0.2.0"', (self.root / "Cargo.lock").read_text())

    def test_server_independent_bump(self):
        library = self.text("windwatcher")
        changelog = self.text("windwatcher", "CHANGELOG.md")
        self.prepare("windwatcher-server", "0.1.1")
        self.assertEqual(self.text("windwatcher"), library)
        self.assertEqual(self.text("windwatcher", "CHANGELOG.md"), changelog)
        self.assertEqual(release.package_version(self.text("windwatcher-server")), "0.1.1")
        self.assertEqual(release.library_requirement(self.text("windwatcher-server")), "0.1.0")
        self.cmd("cargo", "metadata", "--locked", "--format-version", "1", "--offline")

    def test_initial_equal_version(self):
        for package in release.PACKAGES:
            self.prepare(package, "0.1.0")
            self.assertIn("## [0.1.0] - 2026-10-30", self.text(package, "CHANGELOG.md"))

    def test_prerelease(self):
        self.prepare(requested="0.2.0-rc.1")
        self.assertEqual(release.package_version(self.text("windwatcher")), "0.2.0-rc.1")

    def test_invalid_inputs_do_not_modify_checkout(self):
        for requested in ("0.0.9", "v0.1.0", "0.1.0+build", "0.2.0-rc.01", "invalid"):
            self.prepare(requested=requested, success=False)
            self.assertEqual(self.cmd("git", "status", "--porcelain"), "")
        self.prepare(package="other", success=False)

    def test_bad_changelogs(self):
        path = self.root / "crates/windwatcher/CHANGELOG.md"
        for text in ("# Changelog\n", "## [Unreleased]\n\n## [Unreleased]\n- Duplicate\n", "## Unreleased\n- Noncanonical\n", "## [Unreleased]\n\n### Added\n", "## [Unreleased]\n- Something\n\n## [0.2.0] - 2026-01-01\n- Previous\n"):
            with self.subTest(text=text):
                path.write_text(text)
                self.prepare(success=False)
                self.assertEqual(path.read_text(), text)
                self.assertEqual(release.package_version(self.text("windwatcher")), "0.1.0")

    def test_notes_are_package_specific(self):
        self.prepare()
        output = self.cmd(str(SCRIPTS / "extract-release-notes.sh"), "--root", str(self.root), "windwatcher", "0.2.0")
        self.assertIn("Initial windwatcher behavior", output)
        self.assertNotIn("windwatcher-server", output)
        self.assertNotIn("[Unreleased]", output)
        self.assertNotIn("https://", output)

    def test_subsequent_comparison_links(self):
        self.prepare()
        path = self.root / "crates/windwatcher/CHANGELOG.md"
        path.write_text(path.read_text().replace("## [Unreleased]\n", "## [Unreleased]\n\n### Fixed\n\n- Next change.\n"))
        self.prepare(requested="0.2.1")
        self.assertIn("compare/windwatcher-v0.2.0...windwatcher-v0.2.1", path.read_text())
        self.assertEqual(path.read_text().count("[Unreleased]:"), 1)

    def release_commit(self, package="windwatcher", requested="0.2.0"):
        self.prepare(package, requested)
        self.commit(f"chore(release): {package} v{requested}")
        return self.cmd("git", "rev-parse", "HEAD").strip()

    def test_recovery_without_tag_after_main_advances(self):
        sha = self.release_commit()
        (self.root / "later.txt").write_text("Later development\n")
        self.commit("Later work")
        self.assertIn(f"release_commit={sha}", self.state())
        self.assertIn("prepared=true", self.state())

    def test_correct_annotated_tag(self):
        sha = self.release_commit()
        self.cmd("git", "tag", "-a", "windwatcher-v0.2.0", "-m", "Release", sha)
        self.assertIn(f"release_commit={sha}", self.state())

    def test_wrong_tag_target(self):
        self.release_commit()
        self.cmd("git", "tag", "-a", "windwatcher-v0.2.0", "-m", "Wrong", self.initial)
        self.state(success=False)

    def test_lightweight_tag(self):
        self.release_commit()
        self.cmd("git", "tag", "windwatcher-v0.2.0")
        self.state(success=False)

    def test_unreachable_release_commit(self):
        self.cmd("git", "checkout", "-b", "side")
        sha = self.release_commit()
        self.assertEqual(self.state().strip(), "prepared=false")
        self.cmd("git", "tag", "-a", "windwatcher-v0.2.0", "-m", "Unreachable", sha)
        self.state(success=False)

    def test_server_recovery(self):
        sha = self.release_commit("windwatcher-server", "0.1.1")
        self.assertIn(f"release_commit={sha}", self.state("windwatcher-server", "0.1.1"))

    def test_invalid_recovery_manifest(self):
        self.prepare()
        path = self.root / "crates/windwatcher/Cargo.toml"
        path.write_text(path.read_text().replace('"0.2.0"', '"0.3.0"'))
        self.commit("chore(release): windwatcher v0.2.0")
        self.state(success=False)

    def test_release_allow_list(self):
        self.prepare()
        self.cmd("python3", str(SCRIPTS / "release.py"), "allow-list", "--root", str(self.root), "windwatcher", "0.2.0")
        (self.root / "unexpected.txt").write_text("unexpected\n")
        self.cmd("python3", str(SCRIPTS / "release.py"), "allow-list", "--root", str(self.root), "windwatcher", "0.2.0", success=False)
        self.commit("chore(release): windwatcher v0.2.0")
        self.state(success=False)

    def test_duplicate_release_commits_fail(self):
        self.release_commit()
        self.cmd("git", "commit", "--allow-empty", "-m", "chore(release): windwatcher v0.2.0")
        self.state(success=False)

    def test_lockfile_third_party_entries_are_byte_preserved(self):
        path = self.root / "Cargo.lock"
        external = '\n[[package]]\nname = "external-fixture"\nversion = "9.8.7"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "unchanged-fixture"\n'
        path.write_text(path.read_text() + external)
        self.prepare()
        self.assertTrue(path.read_text().endswith(external))

    def test_recovery_rejects_unrelated_lock_changes(self):
        self.prepare()
        path = self.root / "Cargo.lock"
        path.write_text(path.read_text().replace('version = "0.1.0"', 'version = "0.1.1"'))
        self.commit("chore(release): windwatcher v0.2.0")
        self.state(success=False)

    def test_recovery_rejects_server_package_bump_in_library_release(self):
        self.prepare()
        path = self.root / "crates/windwatcher-server/Cargo.toml"
        path.write_text(path.read_text().replace('version = "0.1.0"', 'version = "0.1.1"'))
        self.commit("chore(release): windwatcher v0.2.0")
        self.state(success=False)

    def test_assets_inventory_and_checksums(self):
        root = self.root / "assets"
        root.mkdir()
        for name in artifacts.assets("0.1.0"):
            content = name.encode()
            (root / name).write_bytes(content)
            (root / (name + ".sha256")).write_text(f"{hashlib.sha256(content).hexdigest()}  {name}\n")
        artifacts.validate(root, "0.1.0", create=True)
        artifacts.validate(root, "0.1.0")
        self.assertEqual(len((root / "SHA256SUMS").read_text().splitlines()), 6)
        (root / artifacts.assets("0.1.0")[0]).write_bytes(b"corrupted")
        with self.assertRaises(ValueError):
            artifacts.validate(root, "0.1.0")
        with self.assertRaises(ValueError):
            artifacts.validate(root, "0.1.1")


class OciTests(unittest.TestCase):
    def manifest(self, amd64="sha256:amd64", arm64="sha256:arm64"):
        return {"manifests": [{"platform": {"os": "linux", "architecture": a}, "digest": d}
                              for a, d in (("amd64", amd64), ("arm64", arm64))]}

    def test_valid_architecture_is_reused_without_build_or_push(self):
        with patch.object(sys, "argv", ["oci", "architecture", "0.1.0", "revision", "--arch", "amd64"]), \
                patch.object(oci, "architecture", return_value="sha256:existing"), \
                patch.object(oci.subprocess, "check_call") as call:
            oci.main()
            call.assert_not_called()

    def test_exact_manifest_recovery_and_prerelease_alias_policy(self):
        with patch.object(sys, "argv", ["oci", "manifest", "0.2.0-rc.1", "revision"]), \
                patch.object(oci, "architecture", side_effect=["sha256:amd64", "sha256:arm64"]), \
                patch.object(oci, "inspect", return_value=self.manifest()) as inspect, \
                patch.object(oci.subprocess, "check_call") as call:
            oci.main()
            self.assertFalse(any("push" in args[0][0] for args in call.call_args_list))
            self.assertTrue(all(args[0][0].endswith(":0.2.0-rc.1") for args in inspect.call_args_list))

    def test_wrong_exact_manifest_fails_without_push(self):
        with patch.object(sys, "argv", ["oci", "manifest", "0.1.0", "revision"]), \
                patch.object(oci, "architecture", side_effect=["sha256:amd64", "sha256:arm64"]), \
                patch.object(oci, "inspect", return_value=self.manifest(arm64="sha256:wrong")), \
                patch.object(oci.subprocess, "check_call") as call, self.assertRaises(ValueError):
            oci.main()
        self.assertFalse(any("push" in args[0][0] for args in call.call_args_list))

    def test_stable_aliases_and_no_major_alias(self):
        remote = self.manifest()
        def inspect(ref, raw=False):
            if ref.endswith(":0.1.0") or raw:
                return remote
            return None
        with patch.object(sys, "argv", ["oci", "manifest", "0.1.0", "revision"]), \
                patch.object(oci, "architecture", side_effect=["sha256:amd64", "sha256:arm64"]), \
                patch.object(oci, "inspect", side_effect=inspect), \
                patch.object(oci.subprocess, "check_call") as call:
            oci.main()
        pushes = [args[0][0][-1] for args in call.call_args_list if "push" in args[0][0]]
        self.assertEqual(pushes, ["docker://" + oci.IMAGE + ":0.1", "docker://" + oci.IMAGE + ":latest"])

    def test_recovery_does_not_downgrade_newer_aliases(self):
        def inspect(ref, raw=False):
            if raw:
                return self.manifest()
            return {"Labels": {"org.opencontainers.image.version": "0.1.1"}}
        with patch.object(sys, "argv", ["oci", "manifest", "0.1.0", "revision"]), \
                patch.object(oci, "architecture", side_effect=["sha256:amd64", "sha256:arm64"]), \
                patch.object(oci, "inspect", side_effect=inspect), \
                patch.object(oci.subprocess, "check_call") as call:
            oci.main()
        self.assertFalse(any("push" in args[0][0] for args in call.call_args_list))

    def test_platform_inventory(self):
        manifests = [{"platform": {"os": "linux", "architecture": a}, "digest": "sha256:" + a} for a in ("amd64", "arm64")]
        self.assertEqual(len(oci.platforms({"manifests": manifests})), 2)
        for value in ({"manifests": manifests[:1]}, {"manifests": [manifests[0], manifests[0]]}, {}):
            with self.assertRaises(ValueError):
                oci.platforms(value)

    def test_architecture_recovery_labels(self):
        metadata = {"Os": "linux", "Architecture": "arm64", "Digest": "sha256:abc", "Labels": {"org.opencontainers.image.version": "0.1.0", "org.opencontainers.image.revision": "commit"}}
        with patch.object(oci, "inspect", return_value=metadata):
            self.assertEqual(oci.architecture("image", "arm64", "0.1.0", "commit"), "sha256:abc")
            for arch, ver, sha in (("amd64", "0.1.0", "commit"), ("arm64", "0.2.0", "commit"), ("arm64", "0.1.0", "other")):
                with self.assertRaises(ValueError):
                    oci.architecture("image", arch, ver, sha)

    def test_inspect_fails_closed_on_network_and_auth_errors(self):
        for error in ("unauthorized", "timeout", "too many requests"):
            result = subprocess.CompletedProcess([], 1, "", error)
            with patch.object(oci.subprocess, "run", return_value=result), self.assertRaises(ValueError):
                oci.inspect("image")
        with patch.object(oci.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "", "manifest unknown")):
            self.assertIsNone(oci.inspect("image"))


class GitHubAssetsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "asset.tar.gz").write_bytes(b"canonical")
        self.remote = {}
        self.calls = []

    def command(self, args):
        self.calls.append(args)
        if args[2] == "upload":
            file = Path(args[4])
            self.remote[file.name] = file.read_bytes()
        elif args[2] == "download":
            (Path(args[7]) / args[5]).write_bytes(self.remote[args[5]])

    def test_draft_uploads_only_missing_and_verifies_bytes(self):
        with patch.object(finalize.subprocess, "check_call", side_effect=self.command):
            expected = finalize.synchronize_assets("tag", {"draft": True, "assets": []}, self.root)
            self.assertEqual(expected, ["asset.tar.gz"])
            finalize.synchronize_assets("tag", {"draft": True, "assets": [{"name": "asset.tar.gz"}]}, self.root)
        self.assertEqual(sum(c[2] == "upload" for c in self.calls), 1)
        self.assertFalse(any("--clobber" in c for c in self.calls))

    def test_published_release_is_verification_only(self):
        self.remote["asset.tar.gz"] = b"canonical"
        with patch.object(finalize.subprocess, "check_call", side_effect=self.command):
            finalize.synchronize_assets("tag", {"draft": False, "assets": [{"name": "asset.tar.gz"}]}, self.root)
        self.assertTrue(all(c[2] == "download" for c in self.calls))

    def test_mismatched_published_or_draft_asset_fails(self):
        self.remote["asset.tar.gz"] = b"different public bytes"
        for draft in (False, True):
            with patch.object(finalize.subprocess, "check_call", side_effect=self.command), self.assertRaises(ValueError):
                finalize.synchronize_assets("tag", {"draft": draft, "assets": [{"name": "asset.tar.gz"}]}, self.root)

    def test_incomplete_published_and_unexpected_assets_fail(self):
        for release_state in ({"draft": False, "assets": []}, {"draft": True, "assets": [{"name": "unexpected"}]},
                              {"draft": True, "assets": [{"name": "asset.tar.gz"}] * 2}):
            with patch.object(finalize.subprocess, "check_call") as call, self.assertRaises(ValueError):
                finalize.synchronize_assets("tag", release_state, self.root)
            call.assert_not_called()


class RegistryTests(unittest.TestCase):
    def archive(self, revision="release-sha", content=b"source"):
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            vcs = json.dumps({"git": {"sha1": revision}, "path_in_vcs": "crates/windwatcher"}).encode()
            for name, data in ((".cargo_vcs_info.json", vcs), ("src/lib.rs", content)):
                info = tarfile.TarInfo("windwatcher-0.1.0/" + name)
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
        return buffer.getvalue()

    def test_existing_registry_archive_must_match_revision_and_contents(self):
        with tempfile.TemporaryDirectory() as tmp:
            local = Path(tmp) / "candidate.crate"
            local.write_bytes(self.archive())
            for archive, valid in ((self.archive(), True), (self.archive(revision="wrong"), False), (self.archive(content=b"different"), False)):
                with patch.object(registry, "get", return_value=archive), patch.object(registry, "Path", return_value=local), \
                        patch.object(registry.subprocess, "check_output", return_value="release-sha\n"):
                    if valid:
                        registry.verify("windwatcher", "0.1.0")
                    else:
                        with self.assertRaises(ValueError):
                            registry.verify("windwatcher", "0.1.0")

    def test_exact_dependency_required_and_yanked_versions_rejected(self):
        with patch.object(registry, "get", return_value=None):
            self.assertFalse(registry.exists("windwatcher", "0.1.0"))
        with patch.object(registry, "get", return_value=b'{"version":{"num":"0.1.0","yanked":true}}'), self.assertRaises(ValueError):
            registry.exists("windwatcher", "0.1.0")

    def test_publication_timeout_recovers_but_absence_fails(self):
        for published in (True, False):
            with patch.object(sys, "argv", ["registry", "publish", "windwatcher", "0.1.0"]), \
                    patch.object(registry, "exists", side_effect=[False, published]), \
                    patch.object(registry.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)), \
                    patch.object(registry, "verify") as verify:
                if published:
                    registry.main()
                    verify.assert_called_once_with("windwatcher", "0.1.0")
                else:
                    with self.assertRaises(ValueError):
                        registry.main()
                    verify.assert_not_called()

    def test_existing_version_is_not_republished(self):
        with patch.object(sys, "argv", ["registry", "publish", "windwatcher", "0.1.0"]), \
                patch.object(registry, "exists", return_value=True), patch.object(registry, "verify") as verify, \
                patch.object(registry.subprocess, "run") as publish:
            registry.main()
            publish.assert_not_called()
            verify.assert_called_once()


if __name__ == "__main__":
    os.environ.setdefault("CARGO_BUILD_JOBS", "4")
    unittest.main(verbosity=2)
