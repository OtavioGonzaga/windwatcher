use crate::{
    Storage,
    config::{self, ResolvedQueueProvider, ResolvedStorageProvider},
    error::AppError,
    paths::ResolvedPaths,
    queue::ConfiguredQueue,
};

#[derive(Clone, Copy)]
pub(crate) enum ConnectionPurpose {
    Migration,
    ExistingOnly,
}

fn data_directory(paths: &ResolvedPaths) -> Result<(), AppError> {
    std::fs::create_dir_all(&paths.data_dir).map_err(|source| AppError::DataDirectory {
        path: paths.data_dir.clone(),
        source,
    })
}

pub(crate) async fn queue(
    mut provider: ResolvedQueueProvider,
    paths: &ResolvedPaths,
    purpose: ConnectionPurpose,
) -> Result<ConfiguredQueue, AppError> {
    if matches!(purpose, ConnectionPurpose::Migration) && provider.needs_data_directory() {
        data_directory(paths)?;
    }
    if let ResolvedQueueProvider::Sqlite { options, .. } = &mut provider
        && matches!(purpose, ConnectionPurpose::ExistingOnly)
    {
        *options = options.clone().create_if_missing(false);
    }
    let backend = provider.backend_name();
    let queue =
        ConfiguredQueue::connect(provider)
            .await
            .map_err(|error| AppError::QueueConnection {
                backend,
                source: Box::new(error),
            })?;
    tracing::info!(queue_backend = backend, "queue backend selected");
    Ok(queue)
}

pub(crate) async fn storage(
    mut provider: ResolvedStorageProvider,
    paths: &ResolvedPaths,
    purpose: ConnectionPurpose,
) -> Result<Storage, AppError> {
    if let ResolvedStorageProvider::Sqlite {
        options,
        derived_path,
        create_if_missing,
    } = &mut provider
    {
        match purpose {
            ConnectionPurpose::Migration if *derived_path && *create_if_missing => {
                data_directory(paths)?
            }
            ConnectionPurpose::ExistingOnly => *options = options.clone().create_if_missing(false),
            _ => {}
        }
    }
    let backend = match &provider {
        ResolvedStorageProvider::Sqlite { .. } => "SQLite",
        ResolvedStorageProvider::Postgres(_) => "PostgreSQL",
        ResolvedStorageProvider::MySql(_) => "MySQL",
        ResolvedStorageProvider::MongoDb(_) => "MongoDB",
    };
    let storage = Storage::connect_provider(provider)
        .await
        .map_err(|source| AppError::Storage { backend, source })?;
    tracing::info!(backend, "storage backend selected");
    Ok(storage)
}

pub(crate) async fn connect_storage(
    raw: &config::RawConfig,
    paths: &ResolvedPaths,
    purpose: ConnectionPurpose,
) -> Result<Storage, AppError> {
    storage(config::resolve_storage(raw, paths)?, paths, purpose).await
}

pub(crate) async fn connect_queue(
    raw: &config::RawConfig,
    paths: &ResolvedPaths,
    purpose: ConnectionPurpose,
) -> Result<ConfiguredQueue, AppError> {
    queue(config::resolve_queue(raw, paths)?.provider, paths, purpose).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn existing_only_never_creates_persistence_or_directories() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ResolvedPaths {
            config_file: temp.path().join("config.toml"),
            data_dir: temp.path().join("missing"),
        };
        let raw = config::RawConfig {
            queue: Some(config::RawQueue::default()),
            ..Default::default()
        };
        assert!(
            connect_storage(&raw, &paths, ConnectionPurpose::ExistingOnly)
                .await
                .is_err()
        );
        assert!(
            connect_queue(&raw, &paths, ConnectionPurpose::ExistingOnly)
                .await
                .is_err()
        );
        assert!(!paths.data_dir.exists());
        let storage = connect_storage(&raw, &paths, ConnectionPurpose::Migration)
            .await
            .unwrap();
        assert!(!storage.migration_status().await.unwrap().is_current());
        assert!(!paths.data_dir.join("queue.db").exists());
        assert!(
            connect_queue(&raw, &paths, ConnectionPurpose::ExistingOnly)
                .await
                .is_err()
        );
        assert!(!paths.data_dir.join("queue.db").exists());
    }
}
