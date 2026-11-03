use super::*;
use crate::command::Output;
use serde_json::json;
use std::cell::RefCell;

fn image() -> Value {
    json!({"Os":"linux","Architecture":"arm64","Digest":"sha256:abc","Labels":{"org.opencontainers.image.version":"0.1.0","org.opencontainers.image.revision":"commit"}})
}
fn multi() -> Value {
    json!({"manifests":[{"platform":{"os":"linux","architecture":"amd64"},"digest":"sha256:amd64"},{"platform":{"os":"linux","architecture":"arm64"},"digest":"sha256:arm64"}]})
}

#[test]
fn architecture_metadata_and_platform_inventory() {
    assert_eq!(
        validate_arch(&image(), "arm64", "0.1.0", "commit").expect("digest"),
        "sha256:abc"
    );
    for (arch, ver, sha) in [
        ("amd64", "0.1.0", "commit"),
        ("arm64", "0.2.0", "commit"),
        ("arm64", "0.1.0", "other"),
    ] {
        assert!(validate_arch(&image(), arch, ver, sha).is_err());
    }
    assert_eq!(platforms(&multi()).expect("platforms").len(), 2);
    let mut duplicate = multi();
    duplicate["manifests"][1] = duplicate["manifests"][0].clone();
    for bad in [json!({}), json!({"manifests":[]}), duplicate] {
        assert!(platforms(&bad).is_err());
    }
}

struct Fake {
    calls: RefCell<Vec<Vec<String>>>,
    version: String,
    alias: Option<String>,
    wrong: bool,
    missing_arch: bool,
    missing_manifest: bool,
    pushed: RefCell<bool>,
    manifest_pushed: RefCell<bool>,
}
impl Runner for Fake {
    fn run(&self, _: &Path, program: &str, args: &[&str]) -> Result<Output> {
        self.calls.borrow_mut().push(
            std::iter::once(program.to_owned())
                .chain(args.iter().map(|s| s.to_string()))
                .collect(),
        );
        let mut value = json!({});
        if program == "skopeo" {
            let reference = args.last().expect("reference");
            if !args.contains(&"--raw") {
                let arch = if reference.ends_with("-amd64") {
                    "amd64"
                } else {
                    "arm64"
                };
                if (reference.ends_with("-amd64") || reference.ends_with("-arm64"))
                    && self.missing_arch
                    && !*self.pushed.borrow()
                {
                    return Ok(Output {
                        success: false,
                        stdout: vec![],
                        stderr: "manifest unknown".into(),
                    });
                }
                let is_alias = !reference.ends_with(&self.version)
                    && !reference.ends_with("-amd64")
                    && !reference.ends_with("-arm64");
                if is_alias && self.alias.is_none() {
                    return Ok(Output {
                        success: false,
                        stdout: vec![],
                        stderr: "manifest unknown".into(),
                    });
                }
                value = json!({"Os":"linux","Architecture":arch,"Digest":format!("sha256:{arch}"),"Labels":{"org.opencontainers.image.version":if is_alias {self.alias.as_ref().expect("alias")} else {&self.version},"org.opencontainers.image.revision":"commit"}});
            } else {
                if self.missing_manifest && !*self.manifest_pushed.borrow() {
                    return Ok(Output {
                        success: false,
                        stdout: vec![],
                        stderr: "manifest unknown".into(),
                    });
                }
                value = multi();
                if self.wrong {
                    value["manifests"][0]["digest"] = json!("sha256:wrong");
                }
            }
        }
        if program == "podman" && args.first() == Some(&"push") {
            *self.pushed.borrow_mut() = true;
        }
        if program == "podman" && args.get(1) == Some(&"push") {
            *self.manifest_pushed.borrow_mut() = true;
        }
        Ok(Output {
            success: true,
            stdout: serde_json::to_vec(&value)?,
            stderr: String::new(),
        })
    }
}
fn fake(ver: &str, alias: Option<&str>, wrong: bool) -> Fake {
    Fake {
        calls: RefCell::new(vec![]),
        version: ver.into(),
        alias: alias.map(str::to_owned),
        wrong,
        missing_arch: false,
        missing_manifest: false,
        pushed: RefCell::new(false),
        manifest_pushed: RefCell::new(false),
    }
}

#[test]
fn architecture_recovery_or_native_publication() {
    for absent in [false, true] {
        let mut f = fake("0.1.0", None, false);
        f.missing_arch = absent;
        architecture(&f, Path::new("."), "arm64", "0.1.0", "commit").expect("arch");
        assert_eq!(
            f.calls
                .borrow()
                .iter()
                .any(|c| c[0] == "podman" && c[1] == "build"),
            absent
        );
        assert_eq!(*f.pushed.borrow(), absent);
    }
}

#[test]
fn exact_manifest_and_alias_policy() {
    for (ver, alias, wrong, pushes, valid) in [
        ("0.2.0-rc.1", None, false, 0, true),
        ("0.1.0", None, false, 2, true),
        ("0.1.0", Some("0.0.9"), false, 2, true),
        ("0.1.0", Some("0.1.1"), false, 0, true),
        ("0.1.0", Some("0.1.0"), false, 0, true),
        ("0.1.0", None, true, 0, false),
    ] {
        let f = fake(ver, alias, wrong);
        assert_eq!(manifest(&f, Path::new("."), ver, "commit").is_ok(), valid);
        let calls = f.calls.borrow();
        let published: Vec<_> = calls
            .iter()
            .filter(|c| c[0] == "podman" && c.get(2).map(String::as_str) == Some("push"))
            .collect();
        assert_eq!(published.len(), pushes);
        assert!(
            !published
                .iter()
                .any(|c| c.last().expect("target").ends_with(":0"))
        );
    }
}

#[test]
fn absent_exact_manifest_is_published_once_and_verified() {
    let mut f = fake("0.1.0", Some("0.1.1"), false);
    f.missing_manifest = true;
    manifest(&f, Path::new("."), "0.1.0", "commit").expect("publish manifest");
    assert!(*f.manifest_pushed.borrow());
    let calls = f.calls.borrow();
    let pushes: Vec<_> = calls
        .iter()
        .filter(|c| c[0] == "podman" && c.get(2).map(String::as_str) == Some("push"))
        .collect();
    assert_eq!(pushes.len(), 1);
    assert!(pushes[0].last().expect("exact target").ends_with(":0.1.0"));
}

#[test]
fn auth_transport_and_rate_limits_never_mean_absence() {
    struct Failure {
        gh: bool,
        message: &'static str,
    }
    impl Runner for Failure {
        fn run(&self, _: &Path, program: &str, _: &[&str]) -> Result<Output> {
            Ok(Output {
                success: program == "gh" && !self.gh,
                stdout: vec![],
                stderr: self.message.into(),
            })
        }
    }
    for message in ["unauthorized", "timeout", "too many requests"] {
        assert!(
            inspect(
                &Failure { gh: false, message },
                Path::new("."),
                "image",
                false
            )
            .is_err()
        );
        assert!(
            inspect(
                &Failure { gh: true, message },
                Path::new("."),
                "image",
                false
            )
            .is_err()
        );
    }
    assert!(
        inspect(
            &Failure {
                gh: false,
                message: "manifest unknown"
            },
            Path::new("."),
            "image",
            false
        )
        .expect("absent")
        .is_none()
    );
    assert!(
        inspect(
            &Failure {
                gh: true,
                message: "gh: Not Found (HTTP 404)"
            },
            Path::new("."),
            "image",
            false
        )
        .expect("bootstrap")
        .is_none()
    );
}
