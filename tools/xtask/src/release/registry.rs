use super::version::parse;
use crate::command::{Runner, bytes, text};
use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use serde_json::Value;
use std::{collections::BTreeMap, fs, io::Read, path::Path};

fn get(r: &dyn Runner, root: &Path, url: &str) -> Result<Option<Vec<u8>>> {
    // curl keeps HTTP status separate from the binary body. Transport failures
    // and every status other than 200/404 fail closed.
    let temp = tempfile::NamedTempFile::new()?;
    let path = temp.path().to_str().context("temporary HTTP path")?;
    let status = text(
        r,
        root,
        "curl",
        &[
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "60",
            "--user-agent",
            "windwatcher-release",
            "--output",
            path,
            "--write-out",
            "%{http_code}",
            url,
        ],
    )?;
    match status.as_str() {
        "404" => Ok(None),
        "200" => Ok(Some(fs::read(temp.path())?)),
        _ => anyhow::bail!("crates.io request failed with HTTP {status}: {url}"),
    }
}

fn exists(r: &dyn Runner, root: &Path, package: &str, requested: &str) -> Result<bool> {
    let Some(body) = get(
        r,
        root,
        &format!("https://crates.io/api/v1/crates/{package}/{requested}"),
    )?
    else {
        return Ok(false);
    };
    let data: Value = serde_json::from_slice(&body)?;
    ensure!(
        data["version"]["num"] == requested && data["version"]["yanked"] == false,
        "registry {package} {requested} is wrong or yanked"
    );
    Ok(true)
}

fn contents(data: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut archive = tar::Archive::new(GzDecoder::new(data));
    let mut files = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        ensure!(
            entry.header().entry_type().is_file() || entry.header().entry_type().is_dir(),
            "unexpected crate archive entry type"
        );
        if entry.header().entry_type().is_file() {
            let name = entry
                .path()?
                .to_str()
                .context("non-UTF8 crate path")?
                .to_owned();
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            ensure!(
                files.insert(name.clone(), data).is_none(),
                "duplicate crate archive member: {name}"
            );
        }
    }
    Ok(files)
}

fn verify_archive(
    remote: &[u8],
    local: &[u8],
    package: &str,
    requested: &str,
    sha: &str,
) -> Result<()> {
    let remote = contents(remote)?;
    let key = format!("{package}-{requested}/.cargo_vcs_info.json");
    let vcs: Value = serde_json::from_slice(
        remote
            .get(&key)
            .context("published crate has no VCS evidence")?,
    )?;
    ensure!(
        vcs["git"]["sha1"] == sha && vcs["git"]["dirty"].as_bool() != Some(true),
        "published {package} {requested} belongs to revision {}, expected {sha} (clean)",
        vcs["git"]["sha1"]
    );
    ensure!(
        vcs["path_in_vcs"] == format!("crates/{package}"),
        "wrong published path_in_vcs: {}",
        vcs["path_in_vcs"]
    );
    ensure!(
        remote == contents(local)?,
        "published {package} {requested} archive differs from validated local package"
    );
    Ok(())
}

pub(crate) fn run(
    r: &dyn Runner,
    root: &Path,
    package: &str,
    requested: &str,
    publish: bool,
) -> Result<()> {
    parse(requested)?;
    let present = exists(r, root, package, requested)?;
    if !publish {
        ensure!(
            present,
            "release {package} {requested} first: exact server dependency is not published to crates.io"
        );
        return Ok(());
    }
    if !present {
        println!("Publishing crates.io {package} {requested}");
        let output = r.run(
            root,
            "cargo",
            &[
                "publish",
                "-p",
                package,
                "--locked",
                "--registry",
                "crates-io",
            ],
        )?;
        ensure!(
            output.success || exists(r, root, package, requested)?,
            "Cargo publication failed before exact version appeared"
        );
    } else {
        println!("Verifying existing crates.io {package} {requested}; not republishing");
    }
    let remote = get(
        r,
        root,
        &format!("https://crates.io/api/v1/crates/{package}/{requested}/download"),
    )?
    .context("published archive unavailable")?;
    let sha = text(r, root, "git", &["rev-parse", "HEAD"])?;
    // Cargo's default target dir can be overridden by the caller/configuration.
    let metadata: Value = serde_json::from_slice(&bytes(
        r,
        root,
        "cargo",
        &["metadata", "--format-version", "1", "--no-deps", "--locked"],
    )?)?;
    let target = metadata["target_directory"]
        .as_str()
        .context("Cargo target_directory missing")?;
    let local = fs::read(Path::new(target).join(format!("package/{package}-{requested}.crate")))?;
    verify_archive(&remote, &local, package, requested, &sha)?;
    println!(
        "Verified crates.io {package} {requested}: revision {sha}, path_in_vcs and all packaged files match"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
