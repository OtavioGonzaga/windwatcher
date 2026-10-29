use crate::error::AppError;
use rand_core::{OsRng, RngCore};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

fn failure(path: &Path, source: io::Error) -> AppError {
    AppError::Init {
        path: path.to_owned(),
        source,
    }
}

pub(crate) fn run(target: PathBuf) -> Result<(), AppError> {
    let target = if target.is_absolute() {
        target
    } else {
        std::env::current_dir()
            .map_err(|e| failure(&target, e))?
            .join(target)
    };
    let directory = target.parent().ok_or_else(|| {
        failure(
            &target,
            io::Error::other("configuration has no parent directory"),
        )
    })?;
    fs::create_dir_all(directory).map_err(|e| failure(directory, e))?;
    let directory = fs::canonicalize(directory).map_err(|e| failure(directory, e))?;
    let target = directory.join(
        target
            .file_name()
            .ok_or_else(|| failure(&target, io::Error::other("invalid configuration filename")))?,
    );
    let key = directory.join("jwt.key");
    let key_text = key.to_str().ok_or_else(|| {
        failure(
            &key,
            io::Error::new(io::ErrorKind::InvalidInput, "key path must be valid UTF-8"),
        )
    })?;
    for path in [&target, &key] {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(failure(
                    path,
                    io::Error::new(io::ErrorKind::AlreadyExists, "file already exists"),
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(failure(path, e)),
        }
    }
    if target == key {
        return Err(failure(
            &target,
            io::Error::other("configuration and key paths must differ"),
        ));
    }
    let mut key_bytes = Zeroizing::new([0u8; 32]);
    OsRng
        .try_fill_bytes(key_bytes.as_mut())
        .map_err(|e| failure(&key, io::Error::other(e)))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut secret = options.open(&key).map_err(|e| failure(&key, e))?;
    let result = (|| {
        secret
            .write_all(key_bytes.as_ref())
            .map_err(|e| failure(&key, e))?;
        let mut config = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| failure(&target, e))?;
        let contents = format!(
            "[auth.jwt]\nsecret_file = {}\n",
            toml::Value::String(key_text.to_owned())
        );
        if let Err(error) = config.write_all(contents.as_bytes()) {
            drop(config);
            let _ = fs::remove_file(&target);
            return Err(failure(&target, error));
        }
        Ok(())
    })();
    if result.is_err() {
        drop(secret);
        let _ = fs::remove_file(&key);
    }
    result?;
    println!(
        "Initialized configuration at {}. Run `windwatcher migrate` before serving.",
        target.display()
    );
    Ok(())
}
