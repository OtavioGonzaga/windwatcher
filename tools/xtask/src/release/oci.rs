use super::version::parse;
use crate::command::{Runner, text};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const IMAGE: &str = "ghcr.io/otaviogonzaga/windwatcher-server";

fn inspect(r: &dyn Runner, root: &Path, reference: &str, raw: bool) -> Result<Option<Value>> {
    let package = r.run(
        root,
        "gh",
        &[
            "api",
            "users/otaviogonzaga/packages/container/windwatcher-server",
        ],
    )?;
    if !package.success {
        ensure!(
            package.stderr.contains("(HTTP 404)"),
            "GHCR package lookup failed: {}",
            package.stderr
        );
        return Ok(None);
    }
    let reference = format!("docker://{reference}");
    let args = if raw {
        vec!["inspect", "--raw", &reference]
    } else {
        vec!["inspect", &reference]
    };
    let output = r.run(root, "skopeo", &args)?;
    if !output.success {
        let error = output.stderr.to_lowercase();
        ensure!(
            error.contains("manifest unknown") || error.contains("name unknown"),
            "OCI inspection failed: {}",
            output.stderr
        );
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&output.stdout)?))
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("missing OCI {key}"))
}

fn validate_arch(metadata: &Value, arch: &str, version: &str, revision: &str) -> Result<String> {
    ensure!(
        metadata["Os"] == "linux" && metadata["Architecture"] == arch,
        "wrong image platform, expected linux/{arch}"
    );
    ensure!(
        metadata["Labels"]["org.opencontainers.image.version"] == version,
        "wrong image version, expected {version}"
    );
    ensure!(
        metadata["Labels"]["org.opencontainers.image.revision"] == revision,
        "wrong image revision, expected {revision}"
    );
    Ok(field(metadata, "Digest")?.into())
}

fn arch_digest(
    r: &dyn Runner,
    root: &Path,
    reference: &str,
    arch: &str,
    version: &str,
    revision: &str,
) -> Result<Option<String>> {
    inspect(r, root, reference, false)?
        .map(|value| validate_arch(&value, arch, version, revision))
        .transpose()
}

fn platforms(manifest: &Value) -> Result<BTreeMap<(String, String), String>> {
    let entries = manifest["manifests"]
        .as_array()
        .context("manifest has no platforms")?;
    ensure!(
        entries.len() == 2,
        "manifest must contain exactly two platforms"
    );
    let mut map = BTreeMap::new();
    for entry in entries {
        let platform = &entry["platform"];
        let key = (
            field(platform, "os")?.into(),
            field(platform, "architecture")?.into(),
        );
        ensure!(
            map.insert(key, field(entry, "digest")?.into()).is_none(),
            "duplicate manifest platform"
        );
    }
    Ok(map)
}

fn verify_manifest(
    r: &dyn Runner,
    root: &Path,
    reference: &str,
    expected: &BTreeMap<(String, String), String>,
) -> Result<()> {
    let value = inspect(r, root, reference, true)?.context("manifest not found")?;
    ensure!(
        platforms(&value)? == *expected,
        "existing manifest {reference} differs from release architecture digests"
    );
    Ok(())
}

pub(crate) fn architecture(
    r: &dyn Runner,
    root: &Path,
    arch: &str,
    version: &str,
    revision: &str,
) -> Result<()> {
    parse(version)?;
    let reference = format!("{IMAGE}:{version}-{arch}");
    if let Some(digest) = arch_digest(r, root, &reference, arch, version, revision)? {
        println!("Reusing {reference}@{digest}");
        return Ok(());
    }
    text(
        r,
        root,
        "podman",
        &[
            "build",
            "--format=oci",
            "--file",
            "Containerfile",
            "--tag",
            &reference,
            "--build-arg",
            "CARGO_BUILD_JOBS=4",
            "--build-arg",
            &format!("VERSION={version}"),
            "--build-arg",
            &format!("REVISION={revision}"),
            ".",
        ],
    )?;
    // Verification logic belongs to automation; Containerfile/context to root.
    let tooling = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    text(
        r,
        root,
        "bash",
        &[
            tooling
                .join("scripts/ci/verify-oci.sh")
                .to_str()
                .context("verification path")?,
            &reference,
            arch,
            version,
            revision,
        ],
    )?;
    text(
        r,
        root,
        "bash",
        &[
            tooling
                .join("scripts/ci/test-oci.sh")
                .to_str()
                .context("test path")?,
            &reference,
        ],
    )?;
    if arch_digest(r, root, &reference, arch, version, revision)?.is_none() {
        text(r, root, "podman", &["push", "--format", "oci", &reference])?;
    }
    ensure!(
        arch_digest(r, root, &reference, arch, version, revision)?.is_some(),
        "published architecture {reference} missing"
    );
    Ok(())
}

pub(crate) fn manifest(r: &dyn Runner, root: &Path, version: &str, revision: &str) -> Result<()> {
    let requested = parse(version)?;
    let exact = format!("{IMAGE}:{version}");
    let mut expected = BTreeMap::new();
    for arch in ["amd64", "arm64"] {
        let digest = arch_digest(r, root, &format!("{exact}-{arch}"), arch, version, revision)?
            .with_context(|| format!("missing architecture: {arch}"))?;
        expected.insert(("linux".into(), arch.into()), digest);
    }
    let remote = inspect(r, root, &exact, true)?;
    if let Some(remote) = &remote {
        ensure!(
            platforms(remote)? == expected,
            "existing exact manifest differs from release architectures"
        );
    }
    // Unique local name avoids collisions on repeated invocations.
    let tmp = tempfile::tempdir()?;
    let local = format!(
        "localhost/windwatcher-release:{}-{}",
        version,
        tmp.path()
            .file_name()
            .context("temporary manifest name")?
            .to_string_lossy()
            .to_lowercase()
    );
    text(r, root, "podman", &["manifest", "create", &local])?;
    let result: Result<()> = (|| {
        for digest in expected.values() {
            text(
                r,
                root,
                "podman",
                &[
                    "manifest",
                    "add",
                    &local,
                    &format!("docker://{IMAGE}@{digest}"),
                ],
            )?;
        }
        let push = |reference: &str| -> Result<()> {
            text(
                r,
                root,
                "podman",
                &[
                    "manifest",
                    "push",
                    "--all",
                    "--format",
                    "oci",
                    &local,
                    &format!("docker://{reference}"),
                ],
            )?;
            verify_manifest(r, root, reference, &expected)
        };
        if remote.is_none() {
            push(&exact)?;
        }
        verify_manifest(r, root, &exact, &expected)?;
        if requested.pre.is_empty() {
            for alias in [
                format!("{}.{}", requested.major, requested.minor),
                "latest".into(),
            ] {
                let reference = format!("{IMAGE}:{alias}");
                if let Some(existing) = inspect(r, root, &reference, false)? {
                    let old = field(&existing["Labels"], "org.opencontainers.image.version")?;
                    if parse(old)? > requested {
                        continue;
                    }
                    if old == version {
                        verify_manifest(r, root, &reference, &expected)?;
                        continue;
                    }
                }
                push(&reference)?;
            }
        }
        Ok(())
    })();
    let cleanup = text(r, root, "podman", &["manifest", "rm", &local]);
    result?;
    cleanup?;
    Ok(())
}

#[cfg(test)]
mod tests;
