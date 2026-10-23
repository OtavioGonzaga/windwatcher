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
                Ok(Self::from_user_directories(
                    directories.config_dir().to_path_buf(),
                    directories.data_dir().to_path_buf(),
                ))
            }
        }
    }

    fn from_user_directories(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            config_file: config_dir.join("config.toml"),
            data_dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{ResolvedPaths, RuntimeMode};

    #[test]
    fn user_paths_use_injected_platform_directories() {
        let paths = ResolvedPaths::from_user_directories(
            PathBuf::from("/test/config/windwatcher"),
            PathBuf::from("/test/data/windwatcher"),
        );
        assert_eq!(
            paths.config_file,
            PathBuf::from("/test/config/windwatcher/config.toml")
        );
        assert_eq!(paths.data_dir, PathBuf::from("/test/data/windwatcher"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn system_paths_are_fixed_on_linux() {
        let paths = ResolvedPaths::resolve(RuntimeMode::System).unwrap();
        assert_eq!(
            paths.config_file,
            PathBuf::from("/etc/windwatcher/config.toml")
        );
        assert_eq!(paths.data_dir, PathBuf::from("/var/lib/windwatcher"));
    }
}
