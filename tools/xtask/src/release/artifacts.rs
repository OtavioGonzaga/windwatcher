use super::version::parse;
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

pub(crate) fn assets(version: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (arch, deb) in [("x86_64", "amd64"), ("aarch64", "arm64")] {
        names.extend([
            format!("windwatcher-server-{version}-linux-{arch}.tar.gz"),
            format!("windwatcher-server_{version}-1_{deb}.deb"),
            format!("windwatcher-server-{version}-1.{arch}.rpm"),
        ]);
    }
    names.sort();
    names
}

pub(crate) fn inventory(root: &Path) -> Result<BTreeSet<String>> {
    fs::read_dir(root)?
        .map(|entry| {
            let entry = entry?;
            ensure!(
                entry.file_type()?.is_file(),
                "unexpected non-file asset: {}",
                entry.path().display()
            );
            entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 asset filename"))
        })
        .collect()
}

pub(crate) fn validate(root: &Path, requested: &str, create: bool) -> Result<()> {
    parse(requested)?;
    let primary = assets(requested);
    let mut expected: BTreeSet<_> = primary
        .iter()
        .flat_map(|p| [p.clone(), format!("{p}.sha256")])
        .collect();
    if !create {
        expected.insert("SHA256SUMS".into());
    }
    let actual = inventory(root)?;
    ensure!(
        actual == expected,
        "unexpected/missing assets: {:?}",
        actual.symmetric_difference(&expected).collect::<Vec<_>>()
    );
    let mut aggregate = String::new();
    for name in primary {
        let digest = Sha256::digest(fs::read(root.join(&name))?);
        let digest: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        let line = format!("{digest}  {name}\n");
        ensure!(
            fs::read_to_string(root.join(format!("{name}.sha256")))? == line,
            "checksum mismatch: {name}"
        );
        aggregate.push_str(&line);
    }
    if create {
        fs::write(root.join("SHA256SUMS"), aggregate)?;
    } else {
        ensure!(
            fs::read_to_string(root.join("SHA256SUMS"))? == aggregate,
            "invalid aggregate checksums"
        );
    }
    Ok(())
}

pub(crate) fn collect(source: &Path, destination: &Path, requested: &str) -> Result<()> {
    parse(requested)?;
    let actual: BTreeSet<_> = fs::read_dir(source)?
        .map(|p| Ok(p?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_>>()?;
    ensure!(
        actual
            == BTreeSet::from([
                "windwatcher-server-dist-x86_64".into(),
                "windwatcher-server-dist-aarch64".into()
            ]),
        "expected exactly two native architecture artifacts"
    );
    fs::create_dir_all(destination)?;
    ensure!(
        fs::read_dir(destination)?.next().is_none(),
        "public asset destination must be empty"
    );
    // Validate both inputs before copying anything.
    let mut copies = Vec::new();
    for (arch, deb) in [("x86_64", "amd64"), ("aarch64", "arm64")] {
        let root = source.join(format!("windwatcher-server-dist-{arch}"));
        let primary = [
            format!("windwatcher-server-{requested}-linux-{arch}.tar.gz"),
            format!("windwatcher-server_{requested}-1_{deb}.deb"),
            format!("windwatcher-server-{requested}-1.{arch}.rpm"),
        ];
        let public: BTreeSet<_> = primary
            .iter()
            .flat_map(|p| [p.clone(), format!("{p}.sha256")])
            .collect();
        let mut expected = public.clone();
        expected.insert(format!("windwatcher-{arch}"));
        ensure!(
            inventory(&root)? == expected,
            "wrong architecture artifact contents: {arch}"
        );
        copies.extend(
            public
                .into_iter()
                .map(|name| (root.join(&name), destination.join(name))),
        );
    }
    for (from, to) in copies {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&to)?;
        std::io::copy(&mut fs::File::open(from)?, &mut output)?;
    }
    Ok(())
}
