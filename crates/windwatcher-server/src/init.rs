use crate::error::AppError;
use crate::paths::RuntimeMode;
use getrandom::SysRng;
use rand_core::TryRng;
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

pub(crate) fn run(target: PathBuf, mode: RuntimeMode) -> Result<(), AppError> {
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
    match mode {
        RuntimeMode::User => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder
                .create(directory)
                .map_err(|e| failure(directory, e))?;
        }
        RuntimeMode::System if !directory.is_dir() => {
            return Err(failure(
                directory,
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "system configuration directory must be provisioned first",
                ),
            ));
        }
        RuntimeMode::System => {}
    }
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
    SysRng
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
    let mut config_created = false;
    let result = (|| {
        secret
            .write_all(key_bytes.as_ref())
            .map_err(|e| failure(&key, e))?;
        let mut config = options.open(&target).map_err(|e| failure(&target, e))?;
        config_created = true;
        let contents = format!(
            "[auth.jwt]\nsecret_file = {}\n",
            toml::Value::String(key_text.to_owned())
        );
        config
            .write_all(contents.as_bytes())
            .map_err(|e| failure(&target, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = match mode {
                RuntimeMode::User => 0o600,
                RuntimeMode::System => 0o640,
            };
            secret
                .set_permissions(fs::Permissions::from_mode(permissions))
                .map_err(|e| failure(&key, e))?;
            config
                .set_permissions(fs::Permissions::from_mode(permissions))
                .map_err(|e| failure(&target, e))?;
        }
        Ok(())
    })();
    if result.is_err() {
        drop(secret);
        let _ = fs::remove_file(&key);
        if config_created {
            let _ = fs::remove_file(&target);
        }
    }
    result?;
    println!(
        "Initialized configuration at {}. Run `windwatcher migrate` before serving.",
        target.display()
    );
    Ok(())
}
