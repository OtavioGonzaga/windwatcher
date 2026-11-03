use super::{artifacts, metadata, state, version};
use crate::command::{Runner, bytes, text};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Clone, Deserialize)]
struct Asset {
    id: u64,
    name: String,
}
#[derive(Clone, Deserialize)]
struct Release {
    id: u64,
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

struct Api<'a> {
    runner: &'a dyn Runner,
    root: &'a Path,
    repo: String,
}
impl Api<'_> {
    fn endpoint(&self, suffix: &str) -> String {
        format!("repos/{}/releases{suffix}", self.repo)
    }
    fn get(&self, suffix: &str) -> Result<Release> {
        Ok(serde_json::from_slice(&bytes(
            self.runner,
            self.root,
            "gh",
            &["api", &self.endpoint(suffix)],
        )?)?)
    }
    fn write(&self, suffix: &str, method: &str, body: Value) -> Result<Release> {
        let temp = tempfile::NamedTempFile::new()?;
        fs::write(temp.path(), serde_json::to_vec(&body)?)?;
        Ok(serde_json::from_slice(&bytes(
            self.runner,
            self.root,
            "gh",
            &[
                "api",
                &self.endpoint(suffix),
                "--method",
                method,
                "--input",
                temp.path().to_str().context("JSON input path")?,
            ],
        )?)?)
    }
    fn find(&self, tag: &str) -> Result<Option<Release>> {
        let mut matches = Vec::new();
        // Explicit pagination works for drafts too; html_url is deliberately
        // ignored (GitHub may report an untagged-* URL for a valid draft).
        for page in 1.. {
            let releases: Vec<Release> = serde_json::from_slice(&bytes(
                self.runner,
                self.root,
                "gh",
                &["api", &self.endpoint(&format!("?per_page=100&page={page}"))],
            )?)?;
            let count = releases.len();
            matches.extend(
                releases
                    .into_iter()
                    .filter(|release| release.tag_name == tag),
            );
            ensure!(
                matches.len() <= 1,
                "multiple GitHub releases match tag {tag}"
            );
            if count < 100 {
                break;
            }
        }
        Ok(matches.pop())
    }
}

fn validate_metadata(release: &Release, tag: &str, prerelease: bool) -> Result<()> {
    ensure!(
        release.tag_name == tag,
        "release {} has tag {}, expected {tag}",
        release.id,
        release.tag_name
    );
    ensure!(
        release.prerelease == prerelease,
        "release {tag} has wrong prerelease flag"
    );
    Ok(())
}

fn inventory(release: &Release, expected: &BTreeSet<String>, complete: bool) -> Result<()> {
    let mut seen = BTreeSet::new();
    for asset in &release.assets {
        ensure!(
            expected.contains(&asset.name),
            "existing release {} has unexpected asset {}",
            release.tag_name,
            asset.name
        );
        ensure!(
            seen.insert(asset.name.clone()),
            "duplicate asset {} in {}",
            asset.name,
            release.tag_name
        );
    }
    ensure!(
        !complete || seen == *expected,
        "release {} is missing immutable assets",
        release.tag_name
    );
    Ok(())
}

fn synchronize(
    api: &Api<'_>,
    release: &Release,
    directory: Option<&Path>,
    expected: &BTreeSet<String>,
) -> Result<()> {
    inventory(release, expected, !release.draft)?;
    for name in expected {
        let local = directory.context("asset directory required")?.join(name);
        if let Some(asset) = release.assets.iter().find(|a| a.name == *name) {
            let remote = bytes(
                api.runner,
                api.root,
                "gh",
                &[
                    "api",
                    &api.endpoint(&format!("/assets/{}", asset.id)),
                    "-H",
                    "Accept: application/octet-stream",
                ],
            )?;
            ensure!(
                remote == fs::read(&local)?,
                "release {} asset {name} bytes differ",
                release.tag_name
            );
        } else {
            // Exact known inventory names need no URL escaping. No clobber, no
            // tag-dependent gh release upload/download operations for drafts.
            let endpoint = format!(
                "https://uploads.github.com/repos/{}/releases/{}/assets?name={name}",
                api.repo, release.id
            );
            bytes(
                api.runner,
                api.root,
                "gh",
                &[
                    "api",
                    &endpoint,
                    "--method",
                    "POST",
                    "-H",
                    "Content-Type: application/octet-stream",
                    "--input",
                    local.to_str().context("asset path")?,
                ],
            )?;
        }
    }
    Ok(())
}

fn finish(
    api: &Api<'_>,
    tag: &str,
    package: &str,
    requested: &str,
    notes: &str,
    directory: Option<&Path>,
    expected: &BTreeSet<String>,
) -> Result<()> {
    let prerelease = !version::parse(requested)?.pre.is_empty();
    let release = match api.find(tag)? {
        Some(release) => {
            println!(
                "Reusing {} release {tag} (id {})",
                if release.draft { "draft" } else { "published" },
                release.id
            );
            release
        }
        None => {
            println!("Creating draft release {tag}");
            api.write("", "POST", json!({"tag_name": tag, "name": format!("{package} v{requested}"), "draft": true, "prerelease": prerelease, "body": notes}))?
        }
    };
    validate_metadata(&release, tag, prerelease)?;
    synchronize(api, &release, directory, expected)?;
    let completed = api.get(&format!("/{}", release.id))?;
    validate_metadata(&completed, tag, prerelease)?;
    inventory(&completed, expected, true)?;
    // Download again after uploads, so newly uploaded bytes are verified too.
    synchronize(api, &completed, directory, expected)?;
    if release.draft {
        ensure!(
            completed.draft,
            "draft {tag} was concurrently published; refusing to edit"
        );
        println!("Publishing existing draft {tag} (id {})", release.id);
        api.write(
            &format!("/{}", release.id),
            "PATCH",
            json!({"draft": false, "body": notes, "prerelease": prerelease}),
        )?;
    }
    let published = api.get(&format!("/{}", release.id))?;
    validate_metadata(&published, tag, prerelease)?;
    ensure!(!published.draft, "release {tag} is still a draft");
    inventory(&published, expected, true)?;
    synchronize(api, &published, directory, expected)?;
    println!("Verified published release {tag} (id {})", published.id);
    Ok(())
}

pub(crate) fn finalize(
    r: &dyn Runner,
    root: &Path,
    package: &str,
    requested: &str,
    revision: &str,
    directory: Option<&Path>,
) -> Result<()> {
    version::parse(requested)?;
    ensure!(
        text(r, root, "git", &["rev-parse", "HEAD"])? == revision,
        "checkout is not release commit {revision}"
    );
    text(r, root, "git", &["fetch", "origin", "main", "--tags"])?;
    let state = state::state(r, root, package, requested, "origin/main")?;
    ensure!(
        state
            .lines()
            .any(|l| l == format!("release_commit={revision}")),
        "invalid release recovery context"
    );
    let tag = format!("{package}-v{requested}");
    let mut notes = metadata::notes(root, package, requested)?;
    let expected = if package == "windwatcher" {
        ensure!(
            directory.is_none(),
            "library release cannot have server assets"
        );
        notes.push_str(&format!(
            "\n\n## Installation\n\n```bash\ncargo add windwatcher@{requested}\n```\n"
        ));
        BTreeSet::new()
    } else {
        let directory = directory.context("server assets required")?;
        artifacts::validate(directory, requested, false)?;
        notes.push_str(&format!("\n\n## Installation\n\n```bash\ncargo install windwatcher-server --version {requested}\npodman pull ghcr.io/otaviogonzaga/windwatcher-server:{requested}\n```\n\nDownload the matching architecture's DEB/RPM or tarball below. Verify downloads with `sha256sum -c SHA256SUMS` (download all six primary assets first).\n"));
        artifacts::inventory(directory)?
    };
    if text(r, root, "git", &["tag", "--list", &tag])?.is_empty() {
        text(
            r,
            root,
            "git",
            &["config", "user.name", "github-actions[bot]"],
        )?;
        text(
            r,
            root,
            "git",
            &[
                "config",
                "user.email",
                "41898282+github-actions[bot]@users.noreply.github.com",
            ],
        )?;
        text(
            r,
            root,
            "git",
            &[
                "tag",
                "-a",
                &tag,
                revision,
                "-m",
                &format!("{package} v{requested}"),
            ],
        )?;
        text(
            r,
            root,
            "git",
            &["push", "origin", &format!("refs/tags/{tag}")],
        )?;
    }
    let repo = text(
        r,
        root,
        "gh",
        &[
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ],
    )?;
    finish(
        &Api {
            runner: r,
            root,
            repo,
        },
        &tag,
        package,
        requested,
        &notes,
        directory,
        &expected,
    )
}

#[cfg(test)]
mod tests;
