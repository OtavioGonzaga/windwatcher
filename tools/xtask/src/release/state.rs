use super::{metadata::*, version::parse};
use crate::command::{Runner, bytes, text};
use anyhow::{Result, ensure};
use std::{collections::BTreeSet, path::Path};

pub(crate) fn allowed(package: &str) -> BTreeSet<String> {
    let mut files = BTreeSet::from([
        "Cargo.lock".into(),
        manifest(package),
        format!("crates/{package}/CHANGELOG.md"),
    ]);
    if package == "windwatcher" {
        files.insert(manifest("windwatcher-server"));
    }
    files
}

pub(crate) fn check_diff(r: &dyn Runner, root: &Path, package: &str) -> Result<()> {
    let changed = text(r, root, "git", &["diff", "--name-only", "HEAD", "--"])?;
    let untracked = text(
        r,
        root,
        "git",
        &["ls-files", "--others", "--exclude-standard"],
    )?;
    ensure!(
        untracked.is_empty(),
        "untracked files in release checkout: {untracked}"
    );
    check_files(package, &changed)
}

fn check_files(package: &str, changed: &str) -> Result<()> {
    let allowed = allowed(package);
    for path in changed.lines() {
        ensure!(allowed.contains(path), "unexpected release file: {path}");
    }
    Ok(())
}

pub(crate) fn state(
    r: &dyn Runner,
    root: &Path,
    package: &str,
    requested: &str,
    main_ref: &str,
) -> Result<String> {
    parse(requested)?;
    let subject = format!("chore(release): {package} v{requested}");
    let log = text(r, root, "git", &["log", main_ref, "--format=%H%x09%s"])?;
    let candidates: Vec<_> = log
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(_, s)| *s == subject)
        .map(|(sha, _)| sha)
        .collect();
    ensure!(
        candidates.len() <= 1,
        "multiple release commits found for {package} {requested}"
    );
    let tag = format!("{package}-v{requested}");
    // A successful empty tag list alone means absent.
    let tags = text(r, root, "git", &["tag", "--list", &tag])?;
    let exists = !tags.is_empty();
    let Some(sha) = candidates.first() else {
        ensure!(
            !exists,
            "tag {tag} exists without a reachable release commit"
        );
        return Ok("prepared=false".into());
    };
    let show = |revision: &str, path: &str| -> Result<String> {
        Ok(String::from_utf8(bytes(
            r,
            root,
            "git",
            &["show", &format!("{revision}:{path}")],
        )?)?)
    };
    let path = manifest(package);
    ensure!(
        package_version(&show(sha, &path)?)? == requested,
        "release commit has wrong package version"
    );
    release_body(
        &show(sha, &format!("crates/{package}/CHANGELOG.md"))?,
        requested,
    )?;
    let parents = text(r, root, "git", &["rev-list", "--parents", "-n", "1", sha])?;
    let parents: Vec<_> = parents.split_whitespace().collect();
    ensure!(parents.len() == 2, "release must have exactly one parent");
    let parent = parents[1];
    let old_manifest = show(parent, &path)?;
    let old = package_version(&old_manifest)?;
    ensure!(
        parse(requested)? >= parse(&old)?,
        "release commit downgraded package"
    );
    ensure!(
        show(sha, &path)? == bump_manifest(&old_manifest, requested)?,
        "release manifest contains non-version changes"
    );
    ensure!(
        show(sha, "Cargo.lock")?
            == bump_lock(&show(parent, "Cargo.lock")?, package, &old, requested)?,
        "release changed unrelated lockfile metadata/dependencies"
    );
    if package == "windwatcher" {
        let path = manifest("windwatcher-server");
        ensure!(
            requirement(&show(sha, &path)?)? == requested,
            "wrong server library dependency"
        );
        ensure!(
            show(sha, &path)? == bump_requirement(&show(parent, &path)?, requested)?,
            "library release changed unrelated server manifest metadata"
        );
    }
    check_files(
        package,
        &text(r, root, "git", &["diff", "--name-only", parent, sha])?,
    )?;
    if exists {
        ensure!(
            text(
                r,
                root,
                "git",
                &["cat-file", "-t", &format!("refs/tags/{tag}")]
            )? == "tag",
            "release tag {tag} is not annotated"
        );
        ensure!(
            text(r, root, "git", &["rev-parse", &format!("{tag}^{{commit}}")])? == *sha,
            "release tag {tag} points to wrong commit"
        );
    }
    Ok(format!("prepared=true\nrelease_commit={sha}"))
}
