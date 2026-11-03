use crate::command::{Process, text};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::Command,
};

fn u32be(bytes: &[u8]) -> Result<u32> {
    Ok(u32::from_be_bytes(bytes.try_into()?))
}

fn rpm_header(file: &mut fs::File) -> Result<BTreeMap<u32, (u32, Vec<u8>)>> {
    let mut header = [0; 16];
    file.read_exact(&mut header)?;
    ensure!(header[..4] == [0x8e, 0xad, 0xe8, 1], "invalid RPM header");
    let count = u32be(&header[8..12])?;
    let size = u32be(&header[12..])?;
    ensure!(
        count <= 100_000 && size <= 64 * 1024 * 1024,
        "oversized RPM metadata"
    );
    let mut entries = vec![0; count as usize * 16];
    file.read_exact(&mut entries)?;
    let mut data = vec![0; size as usize];
    file.read_exact(&mut data)?;
    entries
        .as_chunks::<16>()
        .0
        .iter()
        .map(|entry| {
            let offset = u32be(&entry[8..12])? as usize;
            let value = data
                .get(offset..)
                .context("RPM offset out of bounds")?
                .to_vec();
            Ok((u32be(&entry[..4])?, (u32be(&entry[4..8])?, value)))
        })
        .collect()
}

fn verify_metadata(deb: &Path, rpm: &Path, epoch: u32) -> Result<()> {
    let mut file = fs::File::open(deb)?;
    let mut magic = [0; 8];
    file.read_exact(&mut magic)?;
    ensure!(&magic == b"!<arch>\n", "invalid DEB archive");
    while file.stream_position()? < file.metadata()?.len() {
        let mut header = [0; 60];
        file.read_exact(&mut header)?;
        ensure!(&header[58..] == b"`\n", "invalid ar header");
        let timestamp: u32 = std::str::from_utf8(&header[16..28])?.trim().parse()?;
        ensure!(
            timestamp == epoch,
            "DEB member timestamp is not commit time"
        );
        let size: i64 = std::str::from_utf8(&header[48..58])?.trim().parse()?;
        ensure!(size >= 0, "invalid ar member size");
        file.seek(SeekFrom::Current(size + size % 2))?;
    }
    let mut file = fs::File::open(rpm)?;
    file.seek(SeekFrom::Start(96))?;
    rpm_header(&mut file)?;
    let offset = file.stream_position()?;
    file.seek(SeekFrom::Current(((8 - offset % 8) % 8) as i64))?;
    let fields = rpm_header(&mut file)?;
    let (kind, time) = fields.get(&1006).context("RPM build time missing")?;
    ensure!(
        *kind == 4 && u32be(time.get(..4).context("RPM build time truncated")?)? == epoch,
        "RPM build time is not commit time"
    );
    let (_, host) = fields.get(&1007).context("RPM build host missing")?;
    ensure!(
        host.split(|b| *b == 0).next() == Some(b"windwatcher-release"),
        "unstable RPM build host"
    );
    Ok(())
}

pub(crate) fn reproducibility(root: &Path, arch: &str, binary: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let binary = binary.canonicalize()?;
    let epoch: u32 = text(
        &Process,
        &root,
        "git",
        &["show", "-s", "--format=%ct", "HEAD"],
    )?
    .parse()?;
    let tmp = tempfile::tempdir()?;
    let mut builds = Vec::new();
    for index in 0..2 {
        let directory = tmp.path().join(index.to_string());
        fs::create_dir(&directory)?;
        let candidate = directory.join("windwatcher");
        fs::copy(&binary, &candidate)?;
        let file = fs::File::open(&candidate)?;
        let time = std::time::UNIX_EPOCH
            + std::time::Duration::from_secs(u64::from(epoch) + 86400 * (index + 1));
        file.set_times(fs::FileTimes::new().set_modified(time).set_accessed(time))?;
        let mut paths = Vec::new();
        for format in ["deb", "rpm"] {
            let output = Command::new(root.join("scripts/package.sh"))
                .args([format, arch])
                .arg(&candidate)
                .arg(&directory)
                .current_dir(&root)
                .env("SOURCE_DATE_EPOCH", epoch.to_string())
                .env("PACKAGE_RELEASE", "1")
                .env("CARGO_BUILD_JOBS", "4")
                .env_remove("GH_TOKEN")
                .env_remove("GITHUB_TOKEN")
                .env_remove("CRATES_IO_TOKEN")
                .env_remove("CARGO_REGISTRY_TOKEN")
                .output()?;
            ensure!(
                output.status.success(),
                "{format} packaging failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            paths.push(std::path::PathBuf::from(
                String::from_utf8(output.stdout)?.trim(),
            ));
        }
        verify_metadata(&paths[0], &paths[1], epoch)?;
        builds.push(paths);
    }
    for (index, format) in ["deb", "rpm"].iter().enumerate() {
        ensure!(
            fs::read(&builds[0][index])? == fs::read(&builds[1][index])?,
            "{format} builds differ byte-for-byte"
        );
        println!("Reproducible {format}");
    }
    Ok(())
}
