use std::{error::Error, fmt};

use mongodb::{
    Client,
    bson::{Document, doc},
    options::{ClientOptions, ReadPreference, SelectionCriteria, WriteConcern},
};

use super::MigrationStatus;
mod commands;
mod conversations;
mod identity;
mod messages;
mod migrations;
mod queries;
#[cfg(test)]
mod regression_tests;
mod transactions;
#[cfg(test)]
use mongodb::bson::{Binary, Bson, spec::BinarySubtype};

/// Connection settings for a dedicated MongoDB primary-storage database.
#[derive(Clone)]
pub struct MongoDbStorageOptions {
    url: String,
    database: String,
}

impl fmt::Debug for MongoDbStorageOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MongoDbStorageOptions")
            .field("url", &"[REDACTED]")
            .field("database", &self.database)
            .finish()
    }
}

impl MongoDbStorageOptions {
    /// Creates options from a MongoDB connection URL and an explicit database name.
    pub fn new(url: impl Into<String>, database: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            database: database.into(),
        }
    }

    /// Returns the selected database name.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.database
    }

    /// Validates configuration syntax without connecting to MongoDB.
    pub async fn validate(&self) -> Result<(), MongoDbStorageError> {
        self.client_options().await.map(|_| ())
    }

    async fn client_options(&self) -> Result<ClientOptions, MongoDbStorageError> {
        if self.url.trim().is_empty() {
            return Err(MongoDbStorageError::Configuration(
                "MongoDB URL is required",
            ));
        }
        if self.database.trim().is_empty() {
            return Err(MongoDbStorageError::Configuration(
                "MongoDB database name is required",
            ));
        }
        let mut options = ClientOptions::parse(&self.url)
            .await
            .map_err(|_| MongoDbStorageError::Configuration("invalid MongoDB URL"))?;
        options.write_concern = Some(WriteConcern::majority());
        options.selection_criteria =
            Some(SelectionCriteria::ReadPreference(ReadPreference::Primary));
        Ok(options)
    }
}

/// Failures while constructing or operating MongoDB storage infrastructure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MongoDbStorageError {
    /// Invalid connection URL or database name.
    #[error("invalid MongoDB storage configuration: {0}")]
    Configuration(&'static str),
    /// Connection or topology inspection failed.
    #[error("MongoDB storage connection failure")]
    Connection(#[source] Box<dyn Error + Send + Sync>),
    /// The topology is outside this adapter's supported operational scope.
    #[error("unsupported MongoDB topology: {0}")]
    UnsupportedTopology(&'static str),
    /// Migration application failed.
    #[error("MongoDB storage migration failure")]
    Migration(#[source] Box<dyn Error + Send + Sync>),
    /// Schema inspection failed.
    #[error("MongoDB storage schema inspection failure")]
    Schema(#[source] Box<dyn Error + Send + Sync>),
    /// The migration ledger is inconsistent with embedded migrations.
    #[error("invalid MongoDB migration history: {0}")]
    InvalidMigrationHistory(String),
}

/// MongoDB implementation of the Windwatcher primary storage ports.
#[derive(Clone)]
pub struct MongoDbStorage {
    client: Client,
    database: String,
}

impl fmt::Debug for MongoDbStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MongoDbStorage")
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

impl MongoDbStorage {
    /// Connects to a supported replica set without applying migrations.
    pub async fn connect(options: MongoDbStorageOptions) -> Result<Self, MongoDbStorageError> {
        let database = options.database.clone();
        let client_options = options.client_options().await?;
        let client = Client::with_options(client_options)
            .map_err(|error| MongoDbStorageError::Connection(Box::new(error)))?;
        let hello = client
            .database("admin")
            .run_command(doc! { "hello": 1 })
            .await
            .map_err(|error| MongoDbStorageError::Connection(Box::new(error)))?;
        validate_topology(&hello)?;
        Ok(Self { client, database })
    }

    /// Applies pending embedded migrations explicitly.
    pub async fn migrate(&self) -> Result<(), MongoDbStorageError> {
        migrations::apply(&self.database()).await
    }

    /// Inspects migration status without changing the database.
    pub async fn migration_status(&self) -> Result<MigrationStatus, MongoDbStorageError> {
        migrations::status(&self.database()).await
    }

    /// Checks schema status without changing the database.
    pub async fn check_schema(&self) -> Result<MigrationStatus, MongoDbStorageError> {
        self.migration_status().await
    }

    fn database(&self) -> mongodb::Database {
        self.client.database(&self.database)
    }
}

fn validate_topology(hello: &Document) -> Result<(), MongoDbStorageError> {
    if hello.get_str("msg").ok() == Some("isdbgrid") {
        return Err(MongoDbStorageError::UnsupportedTopology(
            "mongos/sharded deployments are not supported",
        ));
    }
    if hello.get_str("setName").is_err()
        || hello.get_i32("maxWireVersion").unwrap_or_default() < 7
        || hello.get("logicalSessionTimeoutMinutes").is_none()
    {
        return Err(MongoDbStorageError::UnsupportedTopology(
            "a transaction-capable replica set is required; standalone servers are unsupported",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn uuid_binary(bytes: [u8; 16]) -> Binary {
    Binary {
        subtype: BinarySubtype::Uuid,
        bytes: bytes.to_vec(),
    }
}

#[cfg(test)]
fn uuid_from_bson(value: &Bson) -> Result<[u8; 16], &'static str> {
    let Bson::Binary(binary) = value else {
        return Err("persisted UUID is not BSON binary");
    };
    if binary.subtype != BinarySubtype::Uuid || binary.bytes.len() != 16 {
        return Err("persisted UUID has invalid subtype or length");
    }
    binary
        .bytes
        .as_slice()
        .try_into()
        .map_err(|_| "persisted UUID has invalid length")
}

#[cfg(test)]
fn micros(value: time::OffsetDateTime) -> Result<i64, &'static str> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1_000))
        .map_err(|_| "timestamp out of range")
}

#[cfg(test)]
fn date(value: i64) -> Result<time::OffsetDateTime, &'static str> {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1_000)
        .map_err(|_| "invalid timestamp")
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, doc, spec::BinarySubtype};

    use super::{
        MongoDbStorageOptions, date, micros, uuid_binary, uuid_from_bson, validate_topology,
    };

    #[test]
    fn options_debug_redacts_url_and_names_database() {
        let options = MongoDbStorageOptions::new(
            "mongodb://private:secret@localhost/?replicaSet=rs0",
            "windwatcher",
        );
        let debug = format!("{options:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("windwatcher"));
        assert!(!debug.contains("private"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn topology_requires_replica_set_and_rejects_mongos() {
        assert!(
            validate_topology(&doc! {
                "setName": "rs0",
                "maxWireVersion": 25,
                "logicalSessionTimeoutMinutes": 30,
            })
            .is_ok()
        );
        assert!(validate_topology(&doc! { "ok": 1 }).is_err());
        assert!(validate_topology(&doc! { "msg": "isdbgrid", "setName": "ignored" }).is_err());
    }

    #[test]
    fn uuid_encoding_requires_canonical_binary_subtype_and_length() {
        let bytes = *uuid::Uuid::now_v7().as_bytes();
        let binary = uuid_binary(bytes);
        assert_eq!(binary.subtype, BinarySubtype::Uuid);
        assert_eq!(uuid_from_bson(&Bson::Binary(binary)).unwrap(), bytes);
        assert!(
            uuid_from_bson(&Bson::Binary(mongodb::bson::Binary {
                subtype: BinarySubtype::Generic,
                bytes: bytes.to_vec(),
            }))
            .is_err()
        );
    }

    #[test]
    fn timestamps_keep_signed_microsecond_precision() {
        let instant = time::OffsetDateTime::from_unix_timestamp_nanos(-1_001).unwrap();
        let encoded = micros(instant).unwrap();
        assert_eq!(encoded, -2);
        assert_eq!(
            date(encoded).unwrap(),
            time::OffsetDateTime::from_unix_timestamp_nanos(-2_000).unwrap()
        );
    }
}
