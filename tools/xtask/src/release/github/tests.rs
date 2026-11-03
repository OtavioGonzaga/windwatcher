use super::*;
use crate::command::Output;
use std::cell::RefCell;

struct Fake {
    releases: RefCell<Vec<Value>>,
    calls: RefCell<Vec<Vec<String>>>,
    remote: RefCell<Vec<u8>>,
}

fn draft() -> Value {
    json!({"id":432660999,"tag_name":"windwatcher-v0.1.0","draft":true,"prerelease":false,"assets":[],"html_url":"https://github.com/owner/repo/releases/tag/untagged-123"})
}

#[test]
fn full_finalizer_recovers_draft_with_existing_annotated_tag() {
    use crate::command::Process;
    let tmp = tempfile::tempdir().expect("release checkout");
    let root = tmp.path();
    fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"crates/windwatcher\",\"crates/windwatcher-server\"]\nresolver = \"3\"\n").expect("workspace");
    fs::write(root.join(".gitignore"), "/target/\n").expect("ignore");
    for package in ["windwatcher", "windwatcher-server"] {
        let dir = root.join(format!("crates/{package}"));
        fs::create_dir_all(dir.join("src")).expect("crate");
        fs::write(dir.join("src/lib.rs"), "").expect("source");
        let dep = if package == "windwatcher-server" {
            "\n[dependencies]\nwindwatcher = { path = \"../windwatcher\", version = \"0.1.0\" }\n"
        } else {
            ""
        };
        fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{dep}"
            ),
        )
        .expect("manifest");
        fs::write(
            dir.join("CHANGELOG.md"),
            "## [Unreleased]\n\n- Initial release.\n",
        )
        .expect("changelog");
    }
    let git = |args: &[&str]| text(&Process, root, "git", args).expect("git");
    text(&Process, root, "cargo", &["generate-lockfile", "--offline"]).expect("lock");
    git(&["init", "-b", "main"]);
    git(&["config", "user.name", "Release Tests"]);
    git(&["config", "user.email", "release@example.invalid"]);
    git(&["add", "."]);
    git(&["commit", "-m", "Initial fixture"]);
    metadata::prepare(&Process, root, "windwatcher", "0.1.0", Some("2026-10-30")).expect("prepare");
    git(&["add", "."]);
    git(&["commit", "-m", "chore(release): windwatcher v0.1.0"]);
    let sha = git(&["rev-parse", "HEAD"]);
    git(&["update-ref", "refs/remotes/origin/main", &sha]);
    git(&["tag", "-a", "windwatcher-v0.1.0", "-m", "Release", &sha]);
    struct Recovery(Fake);
    impl Runner for Recovery {
        fn run(&self, root: &Path, program: &str, args: &[&str]) -> Result<Output> {
            if program == "git" {
                assert!(
                    !args.contains(&"push") && !args.contains(&"-a"),
                    "must not replace existing tag"
                );
                if args[0] == "fetch" {
                    return Ok(Output {
                        success: true,
                        stdout: vec![],
                        stderr: String::new(),
                    });
                }
                return Process.run(root, program, args);
            }
            if args[0] == "repo" {
                return Ok(Output {
                    success: true,
                    stdout: b"owner/repo".to_vec(),
                    stderr: String::new(),
                });
            }
            self.0.run(root, program, args)
        }
    }
    let recovery = Recovery(fake(vec![draft()]));
    finalize(&recovery, root, "windwatcher", "0.1.0", &sha, None)
        .expect("recover existing release");
    assert_eq!(recovery.0.releases.borrow().len(), 1);
    assert_eq!(recovery.0.releases.borrow()[0]["draft"], false);
    assert_eq!(git(&["rev-parse", "windwatcher-v0.1.0^{commit}"]), sha);
}

impl Runner for Fake {
    fn run(&self, _: &Path, program: &str, args: &[&str]) -> Result<Output> {
        assert_eq!(program, "gh");
        self.calls
            .borrow_mut()
            .push(args.iter().map(|s| s.to_string()).collect());
        assert!(
            !args.iter().any(|a| a.contains("/tags/")),
            "draft-by-tag would return 404"
        );
        let endpoint = args[1];
        let mut releases = self.releases.borrow_mut();
        let response = if args.contains(&"POST") && endpoint.contains("uploads.github.com") {
            let input = args[args.iter().position(|a| *a == "--input").expect("input") + 1];
            *self.remote.borrow_mut() = fs::read(input)?;
            releases[0]["assets"] = json!([{"id":7,"name":"asset.tar.gz"}]);
            json!({})
        } else if args.contains(&"--input") {
            let input = args[args.iter().position(|a| *a == "--input").expect("input") + 1];
            let body: Value = serde_json::from_slice(&fs::read(input)?)?;
            if args.contains(&"POST") {
                let mut value = draft();
                value["tag_name"] = body["tag_name"].clone();
                value["prerelease"] = body["prerelease"].clone();
                releases.push(value.clone());
                value
            } else {
                releases[0]["draft"] = body["draft"].clone();
                releases[0].clone()
            }
        } else if endpoint.contains("/assets/") {
            return Ok(Output {
                success: true,
                stdout: self.remote.borrow().clone(),
                stderr: String::new(),
            });
        } else if endpoint.contains("?per_page=") {
            if endpoint.ends_with("page=1") {
                Value::Array(releases.clone())
            } else {
                json!([])
            }
        } else {
            releases[0].clone()
        };
        Ok(Output {
            success: true,
            stdout: serde_json::to_vec(&response)?,
            stderr: String::new(),
        })
    }
}

fn fake(releases: Vec<Value>) -> Fake {
    Fake {
        releases: RefCell::new(releases),
        calls: RefCell::new(Vec::new()),
        remote: RefCell::new(b"canonical".to_vec()),
    }
}
fn complete(f: &Fake, dir: Option<&Path>, expected: &BTreeSet<String>) -> Result<()> {
    finish(
        &Api {
            runner: f,
            root: Path::new("."),
            repo: "owner/repo".into(),
        },
        "windwatcher-v0.1.0",
        "windwatcher",
        "0.1.0",
        "curated",
        dir,
        expected,
    )
}

#[test]
fn first_production_draft_is_reused_and_published_without_tag_lookup() {
    let f = fake(vec![draft()]);
    complete(&f, None, &BTreeSet::new()).expect("recover draft");
    assert_eq!(f.releases.borrow().len(), 1);
    assert_eq!(f.releases.borrow()[0]["id"], 432660999_u64);
    assert_eq!(f.releases.borrow()[0]["draft"], false);
    assert!(!f.calls.borrow().iter().any(|c| c.contains(&"POST".into())));
    assert!(
        f.calls
            .borrow()
            .iter()
            .any(|c| c.contains(&"PATCH".into()) && c[1].ends_with("/432660999"))
    );
}

#[test]
fn creation_uses_returned_id_and_published_recovery_does_not_edit() {
    let f = fake(Vec::new());
    complete(&f, None, &BTreeSet::new()).expect("create");
    assert_eq!(
        f.calls
            .borrow()
            .iter()
            .filter(|c| c.contains(&"POST".into()))
            .count(),
        1
    );
    f.calls.borrow_mut().clear();
    complete(&f, None, &BTreeSet::new()).expect("verify published");
    assert!(
        !f.calls
            .borrow()
            .iter()
            .any(|c| c.contains(&"PATCH".into()) || c.contains(&"POST".into()))
    );
}

#[test]
fn duplicates_metadata_and_asset_inventory_fail_closed() {
    let mut pre = draft();
    pre["prerelease"] = json!(true);
    let mut asset = draft();
    asset["assets"] = json!([{"id":7,"name":"unexpected"}]);
    for releases in [vec![draft(), draft()], vec![pre], vec![asset]] {
        let f = fake(releases);
        assert!(complete(&f, None, &BTreeSet::new()).is_err());
        assert!(
            !f.calls
                .borrow()
                .iter()
                .any(|c| c.contains(&"PATCH".into()) || c.contains(&"POST".into()))
        );
    }
}

#[test]
fn pagination_discovers_draft_after_first_page() {
    struct Pages;
    impl Runner for Pages {
        fn run(&self, _: &Path, _: &str, args: &[&str]) -> Result<Output> {
            let values = if args[1].ends_with("page=1") {
                let mut other = draft();
                other["tag_name"] = json!("other");
                vec![other; 100]
            } else {
                vec![draft()]
            };
            Ok(Output {
                success: true,
                stdout: serde_json::to_vec(&values)?,
                stderr: String::new(),
            })
        }
    }
    assert_eq!(
        Api {
            runner: &Pages,
            root: Path::new("."),
            repo: "repo".into()
        }
        .find("windwatcher-v0.1.0")
        .expect("list")
        .expect("found")
        .id,
        432660999
    );
}

#[test]
fn draft_assets_upload_missing_and_compare_existing_without_clobber() {
    let dir = tempfile::tempdir().expect("tmp");
    fs::write(dir.path().join("asset.tar.gz"), b"canonical").expect("asset");
    let expected = BTreeSet::from(["asset.tar.gz".into()]);
    let f = fake(vec![draft()]);
    complete(&f, Some(dir.path()), &expected).expect("upload and publish");
    assert_eq!(
        f.calls
            .borrow()
            .iter()
            .filter(|c| c.contains(&"POST".into()))
            .count(),
        1
    );
    f.calls.borrow_mut().clear();
    complete(&f, Some(dir.path()), &expected).expect("published verify");
    assert!(
        !f.calls
            .borrow()
            .iter()
            .any(|c| c.contains(&"POST".into()) || c.contains(&"PATCH".into()))
    );
    for is_draft in [false, true] {
        f.releases.borrow_mut()[0]["draft"] = json!(is_draft);
        *f.remote.borrow_mut() = b"wrong".to_vec();
        assert!(complete(&f, Some(dir.path()), &expected).is_err());
    }
    let mut missing = draft();
    missing["draft"] = json!(false);
    assert!(complete(&fake(vec![missing]), Some(dir.path()), &expected).is_err());
}
