use std::path::PathBuf;

use directories::ProjectDirs;

use crate::error::AppError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeMode {
    User,
    System,
}

#[derive(Clone, Debug)]
pub struct ResolvedPaths {
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
}

impl ResolvedPaths {
    pub fn resolve(mode: RuntimeMode) -> Result<Self, AppError> {
        match mode {
            RuntimeMode::System => {
                #[cfg(target_os = "linux")]
                {
                    Ok(Self {
                        config_file: PathBuf::from("/etc/windwatcher/config.toml"),
                        data_dir: PathBuf::from("/var/lib/windwatcher"),
                    })
                }
                #[cfg(not(target_os = "linux"))]
                {
                    Err(AppError::Paths(
                        "system mode is supported only on Linux".into(),
                    ))
                }
            }
            RuntimeMode::User => {
                let directories = ProjectDirs::from("org", "Windwatcher", "windwatcher")
                    .ok_or_else(|| {
                        AppError::Paths("could not determine user directories".into())
                    })?;
                Ok(Self {
                    config_file: directories.config_dir().join("config.toml"),
                    data_dir: directories.data_dir().to_path_buf(),
                })
            }
        }
    }
}
