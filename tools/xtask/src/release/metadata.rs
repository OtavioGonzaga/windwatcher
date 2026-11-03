use super::version::parse;
use crate::command::{Runner, text};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::{fs, path::Path};
use toml_edit::DocumentMut;

const REPOSITORY: &str = "https://github.com/OtavioGonzaga/windwatcher";

pub(crate) fn manifest(package: &str) -> String {
    format!("crates/{package}/Cargo.toml")
}

pub(crate) fn package_version(input: &str) -> Result<String> {
    let doc: DocumentMut = input.parse()?;
    Ok(doc["package"]["version"]
        .as_str()
        .context("expected explicit package version")?
        .into())
}

pub(crate) fn requirement(input: &str) -> Result<String> {
    let doc: DocumentMut = input.parse()?;
    let dep = &doc["dependencies"]["windwatcher"];
    ensure!(
        dep["path"].as_str().is_some(),
        "expected path/version library dependency"
    );
    let value = dep["version"]
        .as_str()
        .context("expected explicit library version")?;
    parse(value)?;
    Ok(value.into())
}

fn set_string(item: &mut toml_edit::Item, new: &str) -> Result<()> {
    let value = item.as_value_mut().context("expected manifest value")?;
    let decor = value.decor().clone();
    *value = toml_edit::Value::from(new);
    *value.decor_mut() = decor;
    Ok(())
}

pub(crate) fn bump_manifest(input: &str, requested: &str) -> Result<String> {
    package_version(input)?;
    let mut doc: DocumentMut = input.parse()?;
    set_string(&mut doc["package"]["version"], requested)?;
    Ok(doc.to_string())
}

pub(crate) fn bump_requirement(input: &str, requested: &str) -> Result<String> {
    requirement(input)?;
    let mut doc: DocumentMut = input.parse()?;
    set_string(
        &mut doc["dependencies"]["windwatcher"]["version"],
        requested,
    )?;
    Ok(doc.to_string())
}

pub(crate) fn bump_lock(input: &str, package: &str, old: &str, new: &str) -> Result<String> {
    // Replace only the selected block: even formatting of unrelated entries is
    // immutable. Parsing each block avoids matching a registry package by name.
    let mut parts = input.split("[[package]]\n");
    let mut output = parts.next().unwrap_or_default().to_owned();
    let mut found = 0;
    for block in parts {
        output.push_str("[[package]]\n");
        let doc: DocumentMut = block.parse()?;
        if doc["name"].as_str() == Some(package) && doc.get("source").is_none() {
            ensure!(
                doc["version"].as_str() == Some(old),
                "wrong local lock version for {package}"
            );
            let re = Regex::new(&format!(r#"(?m)^(version\s*=\s*)"{}""#, regex::escape(old)))?;
            ensure!(
                re.find_iter(block).count() == 1,
                "expected one local lock version"
            );
            output.push_str(&re.replace(block, |c: &regex::Captures<'_>| {
                format!("{}\"{new}\"", &c[1])
            }));
            found += 1;
        } else {
            output.push_str(block);
        }
    }
    ensure!(
        found == 1,
        "expected one workspace-local lock entry for {package}"
    );
    Ok(output)
}

fn section_body(tail: &str) -> &str {
    let mut end = tail.len();
    let mut offset = 0;
    for line in tail.split_inclusive('\n') {
        if line.starts_with("## ") || (line.starts_with('[') && line.contains("]:")) {
            end = offset;
            break;
        }
        offset += line.len();
    }
    tail[..end].trim()
}

fn worthy(body: &str) -> Result<()> {
    ensure!(
        Regex::new(r"(?m)^[-*] \S")?.is_match(body),
        "changelog has no release-worthy entries"
    );
    Ok(())
}

pub(crate) fn release_body(input: &str, requested: &str) -> Result<String> {
    parse(requested)?;
    let re = Regex::new(&format!(
        r"(?m)^## \[{}\] - \d{{4}}-\d{{2}}-\d{{2}}[ \t]*$",
        regex::escape(requested)
    ))?;
    let matches: Vec<_> = re.find_iter(input).collect();
    ensure!(
        matches.len() == 1,
        "expected one dated changelog section for {requested}"
    );
    let body = section_body(&input[matches[0].end()..]);
    worthy(body)?;
    Ok(body.into())
}

pub(crate) fn notes(root: &Path, package: &str, requested: &str) -> Result<String> {
    release_body(
        &fs::read_to_string(root.join(format!("crates/{package}/CHANGELOG.md")))?,
        requested,
    )
}

fn valid_date(date: &str) -> Result<()> {
    ensure!(
        Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$")?.is_match(date),
        "invalid date: {date}"
    );
    let year: u32 = date[..4].parse()?;
    let month: usize = date[5..7].parse()?;
    let day: u32 = date[8..].parse()?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    ensure!(
        year > 0 && (1..=12).contains(&month) && (1..=days[month - 1]).contains(&day),
        "invalid date: {date}"
    );
    Ok(())
}

pub(crate) fn promote(input: &str, package: &str, requested: &str, date: &str) -> Result<String> {
    valid_date(date)?;
    let unreleased = Regex::new(r"(?m)^## (?:\[Unreleased\]|Unreleased)[ \t]*$")?;
    ensure!(
        unreleased.find_iter(input).count() == 1 && input.contains("## [Unreleased]\n"),
        "expected exactly one canonical Unreleased section"
    );
    ensure!(
        !Regex::new(&format!(r"(?m)^## \[{}\]", regex::escape(requested)))?.is_match(input),
        "version already in changelog: {requested}"
    );
    let start = input
        .find("## [Unreleased]\n")
        .context("missing Unreleased")?
        + "## [Unreleased]\n".len();
    worthy(section_body(&input[start..]))?;
    let previous_re = Regex::new(r"(?m)^## \[([^\]]+)\] - \d{4}-\d{2}-\d{2}")?;
    let previous = previous_re.captures(input).map(|c| c[1].to_owned());
    let promoted = format!(
        "{}\n## [{requested}] - {date}\n{}",
        &input[..start],
        &input[start..]
    );
    let links = Regex::new(&format!(
        r"(?m)^\[(?:Unreleased|{})\]:.*\n?",
        regex::escape(requested)
    ))?;
    let promoted = links.replace_all(&promoted, "");
    let tag = format!("{package}-v{requested}");
    let link = match previous {
        Some(old) => format!("{REPOSITORY}/compare/{package}-v{old}...{tag}"),
        None => format!("{REPOSITORY}/releases/tag/{tag}"),
    };
    Ok(format!(
        "{}\n\n[Unreleased]: {REPOSITORY}/compare/{tag}...HEAD\n[{requested}]: {link}\n",
        promoted.trim_end()
    ))
}

pub(crate) fn prepare(
    r: &dyn Runner,
    root: &Path,
    package: &str,
    requested: &str,
    date: Option<&str>,
) -> Result<()> {
    let requested_version = parse(requested)?;
    let path = manifest(package);
    let input = fs::read_to_string(root.join(&path))?;
    let old = package_version(&input)?;
    ensure!(
        requested_version >= parse(&old)?,
        "release version would downgrade {package}"
    );
    let today;
    let date = match date {
        Some(date) => date,
        None => {
            today = text(r, root, "date", &["-u", "+%Y-%m-%d"])?;
            &today
        }
    };
    let changelog = format!("crates/{package}/CHANGELOG.md");
    let mut edits = vec![
        (path, bump_manifest(&input, requested)?),
        (
            changelog.clone(),
            promote(
                &fs::read_to_string(root.join(changelog))?,
                package,
                requested,
                date,
            )?,
        ),
        (
            "Cargo.lock".into(),
            bump_lock(
                &fs::read_to_string(root.join("Cargo.lock"))?,
                package,
                &old,
                requested,
            )?,
        ),
    ];
    if package == "windwatcher" {
        let path = manifest("windwatcher-server");
        edits.push((
            path.clone(),
            bump_requirement(&fs::read_to_string(root.join(path))?, requested)?,
        ));
    }
    let originals: Vec<_> = edits
        .iter()
        .map(|(p, _)| Ok((p.clone(), fs::read(root.join(p))?)))
        .collect::<Result<_>>()?;
    let result = (|| {
        for (p, content) in &edits {
            fs::write(root.join(p), content)?;
        }
        text(
            r,
            root,
            "cargo",
            &["metadata", "--format-version", "1", "--locked"],
        )?;
        ensure!(
            fs::read_to_string(root.join("Cargo.lock"))? == edits[2].1,
            "Cargo changed lockfile unexpectedly"
        );
        Ok(())
    })();
    if result.is_err() {
        for (p, content) in originals {
            fs::write(root.join(p), content).context("restoring our release edits")?;
        }
    }
    result?;
    println!("Prepared {package} {requested}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_lock_bytes_are_preserved() {
        let external = "\n[[package]]\nname = \"external-fixture\"\nversion = \"9.8.7\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"unchanged-fixture\"\n\n";
        let original = format!(
            "version = 4\n\n[[package]]\nname = \"windwatcher\"\nversion = \"0.1.0\"\n{external}"
        );
        let updated = bump_lock(&original, "windwatcher", "0.1.0", "0.2.0").expect("bump");
        assert!(updated.ends_with(external));
        assert_eq!(
            updated.replace(
                "name = \"windwatcher\"\nversion = \"0.2.0\"",
                "name = \"windwatcher\"\nversion = \"0.1.0\""
            ),
            original
        );
    }

    #[test]
    fn dates_are_canonical_and_calendar_valid() {
        for good in ["2026-10-30", "2024-02-29"] {
            valid_date(good).expect("date");
        }
        for bad in [
            "2026-02-29",
            "2026-13-01",
            "2026-00-01",
            "2026-1-1",
            "٢٠٢٦-١٠-٣٠",
            "0000-01-01",
        ] {
            assert!(valid_date(bad).is_err(), "{bad}");
        }
    }
}
