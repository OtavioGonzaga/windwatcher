use anyhow::{Context, Result, ensure};
use semver::Version;

pub(crate) fn parse(value: &str) -> Result<Version> {
    let version = Version::parse(value).with_context(|| format!("invalid SemVer: {value}"))?;
    ensure!(
        version.build.is_empty(),
        "build metadata is not allowed: {value}"
    );
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semver() {
        let ordered = [
            "0.1.0",
            "0.2.0-alpha",
            "0.2.0-alpha.1",
            "0.2.0-alpha.2",
            "0.2.0-alpha.10",
            "0.2.0-beta",
            "0.2.0-rc.1",
            "0.2.0",
            "1.0.0",
        ];
        let versions: Vec<_> = ordered
            .iter()
            .map(|s| parse(s).expect("valid version"))
            .collect();
        assert!(versions.windows(2).all(|w| w[0] < w[1]));
        for bad in [
            "v0.1.0",
            "0.1",
            "01.2.3",
            "0.1.0+build",
            "0.2.0-rc.01",
            "0.1.0-01",
            "0.1.0-",
            "1.2.3\n",
            "1.2.3-α",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
