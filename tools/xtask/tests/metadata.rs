use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
}
impl Fixture {
    fn new() -> Self {
        let f = Self {
            dir: tempfile::tempdir().expect("fixture"),
        };
        f.write("Cargo.toml", "[workspace]\nmembers = [\"crates/windwatcher\", \"crates/windwatcher-server\"]\nresolver = \"3\"\n");
        f.write(".gitignore", "/target/\n");
        for package in ["windwatcher", "windwatcher-server"] {
            f.write(
                &format!("crates/{package}/src/lib.rs"),
                "pub fn example() {}\n",
            );
            let mut manifest = format!(
                "[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
            );
            if package == "windwatcher-server" {
                manifest.push_str("\n[dependencies]\nwindwatcher = { path = \"../windwatcher\", version = \"0.1.0\" }\n");
            }
            f.write(&format!("crates/{package}/Cargo.toml"), &manifest);
            f.write(
                &format!("crates/{package}/CHANGELOG.md"),
                &format!(
                    "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- Initial {package} behavior.\n"
                ),
            );
        }
        f.cmd("cargo", &["generate-lockfile", "--offline"]);
        f.cmd("git", &["init", "-b", "main"]);
        f.cmd("git", &["config", "user.name", "Release Tests"]);
        f.cmd("git", &["config", "user.email", "release@example.invalid"]);
        f.commit("Initial fixture");
        f
    }
    fn root(&self) -> &Path {
        self.dir.path()
    }
    fn write(&self, path: &str, content: &str) {
        let path = self.root().join(path);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, content).expect("write");
    }
    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root().join(path)).expect("read")
    }
    fn cmd(&self, program: &str, args: &[&str]) -> String {
        let out = Command::new(program)
            .args(args)
            .current_dir(self.root())
            .env("CARGO_BUILD_JOBS", "4")
            .output()
            .expect("command");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("UTF8").trim().into()
    }
    fn commit(&self, subject: &str) {
        self.cmd("git", &["add", "."]);
        self.cmd("git", &["commit", "--allow-empty", "-m", subject]);
    }
    fn task(&self, mode: &str, package: &str, version: &str) -> Output {
        // Deliberately run outside the release checkout: --root is the only
        // source of release metadata, not cwd/the automation checkout.
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["release", mode, package, version, "--root"])
            .arg(self.root())
            .args(["--date", "2026-10-30", "--main-ref", "main"])
            .env("CARGO_BUILD_JOBS", "4")
            .output()
            .expect("xtask")
    }
    fn success(&self, mode: &str, package: &str, version: &str) -> String {
        let out = self.task(mode, package, version);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("UTF8")
    }
    fn release(&self, package: &str, version: &str) -> String {
        self.success("prepare", package, version);
        self.commit(&format!("chore(release): {package} v{version}"));
        self.cmd("git", &["rev-parse", "HEAD"])
    }
}

#[test]
fn library_independence_and_two_roots() {
    let f = Fixture::new();
    let before = f.read("crates/windwatcher-server/CHANGELOG.md");
    let lock = f.read("Cargo.lock");
    f.success("prepare", "windwatcher", "0.2.0");
    assert!(
        f.read("crates/windwatcher/Cargo.toml")
            .contains("version = \"0.2.0\"")
    );
    let server = f.read("crates/windwatcher-server/Cargo.toml");
    assert!(server.contains("version = \"0.1.0\""));
    assert!(server.contains("version = \"0.2.0\" }"));
    assert_eq!(f.read("crates/windwatcher-server/CHANGELOG.md"), before);
    assert_eq!(
        f.read("Cargo.lock").replace(
            "name = \"windwatcher\"\nversion = \"0.2.0\"",
            "name = \"windwatcher\"\nversion = \"0.1.0\""
        ),
        lock
    );
    f.cmd(
        "cargo",
        &["metadata", "--locked", "--format-version", "1", "--offline"],
    );
    let notes = f.success("notes", "windwatcher", "0.2.0");
    assert!(notes.contains("Initial windwatcher behavior"));
    assert!(!notes.contains("windwatcher-server") && !notes.contains("https://"));
    f.success("allow-list", "windwatcher", "0.2.0");
}

#[test]
fn server_independence() {
    let f = Fixture::new();
    let lib = f.read("crates/windwatcher/Cargo.toml");
    let log = f.read("crates/windwatcher/CHANGELOG.md");
    f.release("windwatcher-server", "0.1.1");
    assert_eq!(f.read("crates/windwatcher/Cargo.toml"), lib);
    assert_eq!(f.read("crates/windwatcher/CHANGELOG.md"), log);
    assert!(
        f.read("crates/windwatcher-server/Cargo.toml")
            .contains("version = \"0.1.0\" }")
    );
    assert!(
        f.success("state", "windwatcher-server", "0.1.1")
            .contains("prepared=true")
    );
}

#[test]
fn first_release_and_prerelease_and_comparison_links() {
    for package in ["windwatcher", "windwatcher-server"] {
        let f = Fixture::new();
        f.success("prepare", package, "0.1.0");
        assert!(
            f.read(&format!("crates/{package}/CHANGELOG.md"))
                .contains("## [0.1.0] - 2026-10-30")
        );
    }
    let f = Fixture::new();
    f.success("prepare", "windwatcher", "0.2.0-rc.1");
    let path = "crates/windwatcher/CHANGELOG.md";
    f.write(
        path,
        &f.read(path)
            .replace("## [Unreleased]\n", "## [Unreleased]\n\n- Next change.\n"),
    );
    f.success("prepare", "windwatcher", "0.2.0");
    let log = f.read(path);
    assert!(log.contains("compare/windwatcher-v0.2.0-rc.1...windwatcher-v0.2.0"));
    assert_eq!(log.matches("[Unreleased]:").count(), 1);
}

#[test]
fn invalid_inputs_and_changelogs_leave_files_unchanged() {
    let f = Fixture::new();
    for version in ["0.0.9", "v0.1.0", "0.1.0+build", "0.2.0-rc.01", "invalid"] {
        assert!(!f.task("prepare", "windwatcher", version).status.success());
        assert_eq!(f.cmd("git", &["status", "--porcelain"]), "");
    }
    for log in [
        "# Changelog\n",
        "## [Unreleased]\n\n## [Unreleased]\n- Duplicate\n",
        "## Unreleased\n- Noncanonical\n",
        "## [Unreleased]\n\n### Added\n",
        "## [Unreleased]\n- Something\n\n## [0.2.0] - 2026-01-01\n- Previous\n",
    ] {
        f.write("crates/windwatcher/CHANGELOG.md", log);
        assert!(!f.task("prepare", "windwatcher", "0.2.0").status.success());
        assert_eq!(f.read("crates/windwatcher/CHANGELOG.md"), log);
        assert!(
            f.read("crates/windwatcher/Cargo.toml")
                .contains("version = \"0.1.0\"")
        );
    }
}

#[test]
fn validation_failure_restores_only_our_edits() {
    let f = Fixture::new();
    let path = "crates/windwatcher-server/Cargo.toml";
    f.write(
        path,
        &f.read(path)
            .replace("version = \"0.1.0\" }", "version = \"0.9.0\" }"),
    );
    let before = [path, "Cargo.lock", "crates/windwatcher-server/CHANGELOG.md"].map(|p| f.read(p));
    assert!(
        !f.task("prepare", "windwatcher-server", "0.1.1")
            .status
            .success()
    );
    assert_eq!(
        before,
        [path, "Cargo.lock", "crates/windwatcher-server/CHANGELOG.md"].map(|p| f.read(p))
    );
}

#[test]
fn recovery_after_main_advances_with_annotated_tag() {
    let f = Fixture::new();
    let sha = f.release("windwatcher", "0.2.0");
    f.cmd(
        "git",
        &["tag", "-a", "windwatcher-v0.2.0", "-m", "Release", &sha],
    );
    f.write("later.txt", "Later development\n");
    f.commit("Later work");
    assert!(f.success("state", "windwatcher", "0.2.0").contains(&sha));
}

#[test]
fn main_dispatch_recovers_historical_release_before_stale_dispatch_sha_gate() {
    let f = Fixture::new();
    let initial = f.cmd("git", &["rev-parse", "HEAD"]);
    let sha = f.release("windwatcher", "0.2.0");
    f.write("later.txt", "Later work\n");
    f.commit("Later work");
    f.cmd(
        "git",
        &["remote", "add", "origin", f.root().to_str().expect("root")],
    );
    let output = f.root().join("workflow-output");
    let result = Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/ci/prepare-release.sh"))
        .args(["windwatcher", "0.2.0"])
        .current_dir(f.root())
        .env("GITHUB_REF", "refs/heads/main")
        .env("GITHUB_SHA", initial)
        .env("GITHUB_OUTPUT", &output)
        .env("CARGO_BUILD_JOBS", "4")
        .output()
        .expect("dispatch");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        fs::read_to_string(output)
            .expect("outputs")
            .contains(&format!("release_commit={sha}"))
    );
    assert_eq!(
        f.cmd("git", &["log", "--format=%s"])
            .lines()
            .filter(|s| *s == "chore(release): windwatcher v0.2.0")
            .count(),
        1
    );
}

#[test]
fn recovery_rejects_lightweight_and_wrong_target_tags() {
    for lightweight in [true, false] {
        let f = Fixture::new();
        let initial = f.cmd("git", &["rev-parse", "HEAD"]);
        f.release("windwatcher", "0.2.0");
        if lightweight {
            f.cmd("git", &["tag", "windwatcher-v0.2.0"]);
        } else {
            f.cmd(
                "git",
                &["tag", "-a", "windwatcher-v0.2.0", "-m", "Wrong", &initial],
            );
        }
        assert!(!f.task("state", "windwatcher", "0.2.0").status.success());
    }
}

#[test]
fn unreachable_release_and_orphan_tag() {
    let f = Fixture::new();
    f.cmd("git", &["checkout", "-b", "side"]);
    let sha = f.release("windwatcher", "0.2.0");
    assert_eq!(
        f.success("state", "windwatcher", "0.2.0").trim(),
        "prepared=false"
    );
    f.cmd(
        "git",
        &["tag", "-a", "windwatcher-v0.2.0", "-m", "Unreachable", &sha],
    );
    assert!(!f.task("state", "windwatcher", "0.2.0").status.success());
}

#[test]
fn invalid_release_commits_fail_closed() {
    for corruption in ["manifest", "lock", "server", "unexpected", "duplicate"] {
        let f = Fixture::new();
        f.success("prepare", "windwatcher", "0.2.0");
        match corruption {
            "manifest" => {
                let p = "crates/windwatcher/Cargo.toml";
                f.write(p, &f.read(p).replace("0.2.0", "0.3.0"));
            }
            "lock" => {
                let p = "Cargo.lock";
                f.write(p, &f.read(p).replace("0.1.0", "0.1.1"));
            }
            "server" => {
                let p = "crates/windwatcher-server/Cargo.toml";
                f.write(p, &f.read(p).replace("0.1.0", "0.1.1"));
            }
            "unexpected" => {
                f.write("unexpected.txt", "unexpected\n");
                assert!(
                    !f.task("allow-list", "windwatcher", "0.2.0")
                        .status
                        .success()
                );
            }
            _ => {}
        }
        f.commit("chore(release): windwatcher v0.2.0");
        if corruption == "duplicate" {
            f.commit("chore(release): windwatcher v0.2.0");
        }
        assert!(
            !f.task("state", "windwatcher", "0.2.0").status.success(),
            "{corruption}"
        );
    }
}
