use super::*;
use crate::command::Output;
use flate2::{Compression, write::GzEncoder};
use serde_json::json;
use std::cell::RefCell;

fn archive(sha: &str, path: &str, source: &[u8]) -> Vec<u8> {
    let gzip = GzEncoder::new(Vec::new(), Compression::default());
    let mut tar = tar::Builder::new(gzip);
    let vcs = serde_json::to_vec(&json!({"git":{"sha1":sha},"path_in_vcs":path})).expect("json");
    for (name, content) in [
        (".cargo_vcs_info.json", vcs.as_slice()),
        ("src/lib.rs", source),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, format!("windwatcher-0.1.0/{name}"), content)
            .expect("tar");
    }
    tar.into_inner()
        .expect("tar end")
        .finish()
        .expect("gzip end")
}

#[test]
fn archive_requires_exact_revision_path_and_every_file() {
    let local = archive("release-sha", "crates/windwatcher", b"source");
    verify_archive(&local, &local, "windwatcher", "0.1.0", "release-sha")
        .expect("matching archive");
    for remote in [
        archive("wrong", "crates/windwatcher", b"source"),
        archive("release-sha", "wrong", b"source"),
        archive("release-sha", "crates/windwatcher", b"different"),
    ] {
        assert!(verify_archive(&remote, &local, "windwatcher", "0.1.0", "release-sha").is_err());
    }
}

struct Fake {
    present: RefCell<Vec<bool>>,
    yanked: bool,
    publish_ok: bool,
    publishes: RefCell<usize>,
    target: std::path::PathBuf,
}
impl Runner for Fake {
    fn run(&self, _: &Path, program: &str, args: &[&str]) -> Result<Output> {
        let mut stdout = Vec::new();
        let mut success = true;
        match program {
            "curl" => {
                let url = args.last().expect("url");
                let path = args[args.iter().position(|a| *a == "--output").expect("output") + 1];
                let present = if url.ends_with("/download") {
                    true
                } else {
                    self.present.borrow_mut().remove(0)
                };
                stdout = if present {
                    b"200".to_vec()
                } else {
                    b"404".to_vec()
                };
                if present {
                    let data = if url.ends_with("/download") {
                        archive("release-sha", "crates/windwatcher", b"source")
                    } else {
                        serde_json::to_vec(
                            &json!({"version":{"num":"0.1.0","yanked":self.yanked}}),
                        )?
                    };
                    fs::write(path, data)?;
                }
            }
            "cargo" if args[0] == "publish" => {
                *self.publishes.borrow_mut() += 1;
                success = self.publish_ok;
            }
            "cargo" => stdout = serde_json::to_vec(&json!({"target_directory":self.target}))?,
            "git" => stdout = b"release-sha".to_vec(),
            _ => anyhow::bail!("unexpected {program}"),
        }
        Ok(Output {
            success,
            stdout,
            stderr: String::new(),
        })
    }
}

#[test]
fn exact_presence_yanking_and_publish_timeout_recovery() {
    let temp = tempfile::tempdir().expect("tmp");
    fs::create_dir(temp.path().join("package")).expect("package dir");
    fs::write(
        temp.path().join("package/windwatcher-0.1.0.crate"),
        archive("release-sha", "crates/windwatcher", b"source"),
    )
    .expect("local crate");
    for (presence, yanked, ok, publish, expected, count) in [
        (vec![false], false, true, false, false, 0),
        (vec![true], false, true, false, true, 0),
        (vec![true], true, true, true, false, 0),
        (vec![true], false, true, true, true, 0),
        (vec![false], false, true, true, true, 1),
        (vec![false, true], false, false, true, true, 1),
        (vec![false, false], false, false, true, false, 1),
    ] {
        let f = Fake {
            present: RefCell::new(presence),
            yanked,
            publish_ok: ok,
            publishes: RefCell::new(0),
            target: temp.path().into(),
        };
        assert_eq!(
            run(&f, temp.path(), "windwatcher", "0.1.0", publish).is_ok(),
            expected
        );
        assert_eq!(*f.publishes.borrow(), count);
    }
}
