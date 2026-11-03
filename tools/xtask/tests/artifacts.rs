use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

fn task(mode: &str, directory: &Path, create: bool) -> bool {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xtask"));
    cmd.args(["release", "artifacts", mode, "0.1.0"])
        .arg(directory);
    if create {
        cmd.arg("--create");
    }
    cmd.output().expect("xtask").status.success()
}
fn names() -> Vec<String> {
    [("x86_64", "amd64"), ("aarch64", "arm64")]
        .into_iter()
        .flat_map(|(arch, deb)| {
            [
                format!("windwatcher-server-0.1.0-linux-{arch}.tar.gz"),
                format!("windwatcher-server_0.1.0-1_{deb}.deb"),
                format!("windwatcher-server-0.1.0-1.{arch}.rpm"),
            ]
        })
        .collect()
}
#[test]
fn inventory_and_checksum_failures() {
    for corruption in [
        "none",
        "missing",
        "sidecar",
        "aggregate",
        "unexpected",
        "bytes",
    ] {
        let tmp = tempfile::tempdir().expect("tmp");
        for name in names() {
            fs::write(tmp.path().join(&name), b"artifact").expect("asset");
            fs::write(
                tmp.path().join(format!("{name}.sha256")),
                format!("{:x}  {name}\n", Sha256::digest(b"artifact")),
            )
            .expect("checksum");
        }
        assert!(task("validate", tmp.path(), true));
        let first = &names()[0];
        match corruption {
            "missing" => fs::remove_file(tmp.path().join(first)).expect("remove"),
            "sidecar" => {
                fs::write(tmp.path().join(format!("{first}.sha256")), "bad").expect("corrupt")
            }
            "aggregate" => fs::write(tmp.path().join("SHA256SUMS"), "bad").expect("corrupt"),
            "unexpected" => fs::write(tmp.path().join("unexpected"), "bad").expect("extra"),
            "bytes" => fs::write(tmp.path().join(first), "bad").expect("corrupt"),
            _ => {}
        }
        assert_eq!(task("validate", tmp.path(), false), corruption == "none");
    }
}

#[test]
fn collection_rejects_duplicate_architecture_and_nonempty_destination() {
    let tmp = tempfile::tempdir().expect("tmp");
    let source = tmp.path().join("source");
    fs::create_dir(&source).expect("source");
    for (arch, deb) in [("x86_64", "amd64"), ("aarch64", "arm64")] {
        let dir = source.join(format!("windwatcher-server-dist-{arch}"));
        fs::create_dir(&dir).expect("arch");
        fs::write(dir.join(format!("windwatcher-{arch}")), "binary").expect("binary");
        for name in names()
            .into_iter()
            .filter(|n| n.contains(arch) || n.ends_with(&format!("_{deb}.deb")))
        {
            fs::write(dir.join(&name), b"artifact").expect("asset");
            fs::write(
                dir.join(format!("{name}.sha256")),
                format!("{:x}  {name}\n", Sha256::digest(b"artifact")),
            )
            .expect("checksum");
        }
    }
    let collect = |dest: &Path| {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["release", "artifacts", "collect", "0.1.0"])
            .arg(&source)
            .arg(dest)
            .output()
            .expect("collect")
            .status
            .success()
    };
    let output = tmp.path().join("public");
    assert!(collect(&output));
    assert!(task("validate", &output, true));
    assert!(!collect(&output));
    fs::create_dir(source.join("windwatcher-server-dist-duplicate")).expect("duplicate");
    assert!(!collect(&tmp.path().join("second")));
}
