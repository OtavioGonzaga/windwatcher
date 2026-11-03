#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

fn automation() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn executable(path: &Path, body: &str) {
    fs::write(
        path,
        format!("#!/usr/bin/env bash\nset -euo pipefail\n{body}"),
    )
    .expect("script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

#[test]
fn dispatch_branch_gate_precedes_any_recovery_or_outputs() {
    let tmp = tempfile::tempdir().expect("tmp");
    let output = tmp.path().join("output");
    for reference in ["refs/heads/feature", "refs/tags/windwatcher-v0.1.0"] {
        let result = Command::new("bash")
            .arg(automation().join("scripts/ci/prepare-release.sh"))
            .args(["windwatcher", "0.1.0"])
            .current_dir(tmp.path())
            .env("GITHUB_REF", reference)
            .env("GITHUB_OUTPUT", &output)
            .output()
            .expect("prepare");
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("must be dispatched from main"));
        assert!(!output.exists());
    }
}

#[test]
fn no_python_guard_detects_executable_dependencies_but_allows_prose() {
    for (name, body, pass) in [
        ("note.md", "Python is not required\n", true),
        ("note.md", "python is not required\n", true),
        ("tool.py", "# tracked file\n", false),
        ("tool.sh", "#!/usr/bin/env python3\n", false),
        ("tool.sh", "python3 -c 'print(1)'\n", false),
        ("tool.sh", "python3 extensionless-script\n", false),
        ("tool.sh", "version=$(python3 helper.py)\n", false),
        ("tool.sh", "printf '{}' | python -c 'import json'\n", false),
        ("tool.yml", "run: python3 <<'PY'\n", false),
    ] {
        let tmp = tempfile::tempdir().expect("tmp");
        let scripts = tmp.path().join("scripts/ci");
        fs::create_dir_all(&scripts).expect("scripts");
        fs::copy(
            automation().join("scripts/ci/verify-no-python.sh"),
            scripts.join("verify-no-python.sh"),
        )
        .expect("guard");
        fs::write(tmp.path().join("scripts").join(name), body).expect("fixture");
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(tmp.path())
                .status()
                .expect("git init")
                .success()
        );
        assert!(
            Command::new("git")
                .args(["add", "."])
                .current_dir(tmp.path())
                .status()
                .expect("git add")
                .success()
        );
        let result = Command::new("bash")
            .arg(scripts.join("verify-no-python.sh"))
            .current_dir(tmp.path())
            .output()
            .expect("guard");
        assert_eq!(
            result.status.success(),
            pass,
            "{body}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
}

#[test]
fn artifact_helper_builds_release_root_once_and_curates_tar_payload() {
    let arch = std::env::consts::ARCH;
    if !matches!(arch, "x86_64" | "aarch64") {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let release = tmp.path().join("release");
    let tools = tmp.path().join("bin");
    let auto = tmp.path().join("automation");
    for path in [
        release.join("scripts"),
        release.join("packaging/systemd"),
        auto.join("scripts/ci"),
        tools.clone(),
    ] {
        fs::create_dir_all(path).expect("mkdir");
    }
    fs::copy(
        automation().join("scripts/build-release-artifacts.sh"),
        auto.join("scripts/build-release-artifacts.sh"),
    )
    .expect("helper");
    // Building xtask is stubbed separately; the server build must run at root.
    executable(
        &tools.join("cargo"),
        "if [[ $1 == run ]]; then exit 0; fi\necho \"$*\" >> build-invocations\nmkdir -p target/release\nprintf '#!/bin/sh\\necho \"windwatcher 0.1.0\"\\n' > target/release/windwatcher\nchmod +x target/release/windwatcher\n",
    );
    for program in ["strip", "ldd", "readelf"] {
        executable(&tools.join(program), "exit 0\n");
    }
    executable(&tools.join("git"), "echo 1700000000\n");
    executable(
        &release.join("scripts/package.sh"),
        "[[ $SOURCE_DATE_EPOCH == 1700000000 ]]\nfile=\"$4/fixture.$1\"\ncp \"$3\" \"$file\"\nprintf '%s\\n' \"$file\"\n",
    );
    executable(
        &auto.join("scripts/ci/verify-packaging.sh"),
        "[[ $5 == --root && $6 == \"$PWD\" ]]\ncmp \"$1\" \"$4\"\ncmp \"$2\" \"$4\"\n",
    );
    for name in [
        "LICENSE",
        "README.md",
        "config.example.toml",
        "packaging/systemd/windwatcher.service",
        "packaging/systemd/windwatcher-worker.service",
    ] {
        fs::write(release.join(name), "fixture\n").expect("payload");
    }
    let output = tmp.path().join("dist");
    let result = Command::new("bash")
        .arg(auto.join("scripts/build-release-artifacts.sh"))
        .args(["0.1.0", arch])
        .arg(&output)
        .arg("--root")
        .arg(&release)
        .env(
            "PATH",
            format!(
                "{}:{}",
                tools.display(),
                std::env::var("PATH").expect("PATH")
            ),
        )
        .current_dir(tmp.path())
        .output()
        .expect("helper");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fs::read_to_string(release.join("build-invocations")).expect("invocations"),
        "build --release --locked -p windwatcher-server --bin windwatcher\n"
    );
    let top = format!("windwatcher-server-0.1.0-linux-{arch}");
    let gzip = flate2::read::GzDecoder::new(
        fs::File::open(output.join(format!("{top}.tar.gz"))).expect("tarball"),
    );
    let mut tar = tar::Archive::new(gzip);
    let mut names = std::collections::BTreeSet::new();
    for entry in tar.entries().expect("entries") {
        let mut entry = entry.expect("entry");
        if entry.header().entry_type().is_file() {
            let name = entry.path().expect("path").to_string_lossy().into_owned();
            if name == format!("{top}/windwatcher") {
                assert_eq!(entry.header().mode().expect("mode"), 0o755);
                let mut content = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut content).expect("binary");
                assert_eq!(
                    content,
                    fs::read(output.join(format!("windwatcher-{arch}"))).expect("canonical")
                );
            }
            names.insert(name);
        }
    }
    assert_eq!(
        names,
        [
            "windwatcher",
            "LICENSE",
            "README.md",
            "config.example.toml",
            "windwatcher.service",
            "windwatcher-worker.service"
        ]
        .map(|n| format!("{top}/{n}"))
        .into_iter()
        .collect()
    );
    assert!(
        Command::new("sha256sum")
            .args([
                "-c",
                &format!("{top}.tar.gz.sha256"),
                "fixture.deb.sha256",
                "fixture.rpm.sha256"
            ])
            .current_dir(&output)
            .output()
            .expect("checksums")
            .status
            .success()
    );
}
