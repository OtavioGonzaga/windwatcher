use futures_util::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{Document, doc},
    error::ErrorKind,
    options::IndexOptions,
};
use sha2::{Digest, Sha256};

use super::{MigrationStatus, MongoDbStorageError, transactions};

const LEDGER: &str = "_windwatcher_migrations";

struct Migration {
    version: i64,
    name: &'static str,
    // The checksummed bytes are the actual ordered commands, not a description.
    commands: &'static str,
}

const MIGRATIONS: [Migration; 4] = [
    Migration {
        version: 1,
        name: "identity_sessions",
        commands: include_str!("../../../migrations/mongodb/0001_identity_sessions.json"),
    },
    Migration {
        version: 2,
        name: "conversations_memberships",
        commands: include_str!("../../../migrations/mongodb/0002_conversations_memberships.json"),
    },
    Migration {
        version: 3,
        name: "messages_read_state",
        commands: include_str!("../../../migrations/mongodb/0003_messages_read_state.json"),
    },
    Migration {
        version: 4,
        name: "queries_idempotency",
        commands: include_str!("../../../migrations/mongodb/0004_queries_idempotency.json"),
    },
];

fn checksum(migration: &Migration) -> String {
    Sha256::digest(migration.commands.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn schema_error(error: mongodb::error::Error) -> MongoDbStorageError {
    MongoDbStorageError::Schema(Box::new(error))
}
fn migration_error(error: mongodb::error::Error) -> MongoDbStorageError {
    MongoDbStorageError::Migration(Box::new(error))
}
async fn collection_exists(database: &Database, name: &str) -> mongodb::error::Result<bool> {
    Ok(database
        .list_collection_names()
        .await?
        .iter()
        .any(|collection| collection == name))
}

async fn applied(database: &Database) -> Result<Vec<Document>, MongoDbStorageError> {
    if !collection_exists(database, LEDGER)
        .await
        .map_err(schema_error)?
    {
        return Ok(Vec::new());
    }
    database
        .collection::<Document>(LEDGER)
        .find(doc! {})
        .sort(doc! { "version": 1 })
        .await
        .map_err(schema_error)?
        .try_collect()
        .await
        .map_err(schema_error)
}

fn validate_history(records: &[Document]) -> Result<usize, MongoDbStorageError> {
    for (index, record) in records.iter().enumerate() {
        let invalid = |message: &str| MongoDbStorageError::InvalidMigrationHistory(message.into());
        let version = record
            .get_i64("version")
            .map_err(|_| invalid("ledger version is missing or invalid"))?;
        let name = record
            .get_str("name")
            .map_err(|_| invalid("ledger name is missing or invalid"))?;
        let stored_checksum = record
            .get_str("checksum")
            .map_err(|_| invalid("ledger checksum is missing or invalid"))?;
        record
            .get_i64("applied_at")
            .map_err(|_| invalid("ledger applied_at is missing or invalid"))?;
        let Some(expected) = MIGRATIONS
            .iter()
            .find(|migration| migration.version == version)
        else {
            return Err(invalid(&format!("unknown migration version {version}")));
        };
        if name != expected.name || stored_checksum != checksum(expected) {
            return Err(invalid(&format!(
                "migration {version} name or checksum differs"
            )));
        }
        if records[..index]
            .iter()
            .any(|record| record.get_i64("version").ok() == Some(version))
        {
            return Err(invalid(&format!("duplicate migration version {version}")));
        }
        if version != index as i64 + 1 {
            return Err(invalid("migration versions are not contiguous"));
        }
    }
    Ok(records.len())
}

pub(super) async fn status(database: &Database) -> Result<MigrationStatus, MongoDbStorageError> {
    let count = validate_history(&applied(database).await?)?;
    Ok(MigrationStatus {
        current_version: count.checked_sub(1).map(|index| MIGRATIONS[index].version),
        latest_version: MIGRATIONS.last().map(|migration| migration.version),
        pending: MIGRATIONS.len() - count,
    })
}

async fn apply_step(database: &Database, migration: &Migration) -> Result<(), MongoDbStorageError> {
    let commands: Vec<Document> = serde_json::from_str(migration.commands)
        .map_err(|error| MongoDbStorageError::Migration(Box::new(error)))?;
    for command in commands {
        let creates_collection = command.contains_key("create");
        match database.run_command(command).await {
            Ok(_) => (),
            // DDL is not transactional. These steps are deliberately restartable
            // after a crash or a concurrent migrator applying the same version.
            Err(error)
                if creates_collection
                    && matches!(error.kind.as_ref(), ErrorKind::Command(command) if command.code == 48) =>
                {}
            Err(error) => return Err(migration_error(error)),
        }
    }
    Ok(())
}

pub(super) async fn apply(database: &Database) -> Result<(), MongoDbStorageError> {
    // Fail closed on existing malformed history before changing the schema.
    validate_history(&applied(database).await?)?;
    // createIndexes also creates the ledger collection. This must precede any
    // ledger inserts so two operators cannot record the same version twice.
    database
        .collection::<Document>(LEDGER)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "version": 1 })
                .options(
                    IndexOptions::builder()
                        .name("migration_version_unique".to_owned())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .map_err(migration_error)?;
    for migration in &MIGRATIONS {
        if validate_history(&applied(database).await?)? as i64 >= migration.version {
            continue;
        }
        apply_step(database, migration).await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| MongoDbStorageError::Migration(Box::new(error)))?;
        let applied_at = i64::try_from(now.as_micros())
            .map_err(|error| MongoDbStorageError::Migration(Box::new(error)))?;
        let entry = doc! { "version": migration.version, "name": migration.name, "checksum": checksum(migration), "applied_at": applied_at };
        match database
            .collection::<Document>(LEDGER)
            .insert_one(entry)
            .await
        {
            Ok(_) => (),
            Err(error) if transactions::duplicate_key(&error) => {
                validate_history(&applied(database).await?)?;
            }
            Err(error) => return Err(migration_error(error)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(migration: &Migration) -> Document {
        doc! { "version": migration.version, "name": migration.name, "checksum": checksum(migration), "applied_at": 1_i64 }
    }
    fn rejects(records: &[Document], expected: &str) {
        let error = validate_history(records).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }

    #[test]
    fn empty_complete_and_partial_histories_are_valid() {
        assert_eq!(validate_history(&[]).unwrap(), 0);
        let records: Vec<_> = MIGRATIONS.iter().map(entry).collect();
        for count in 1..=records.len() {
            assert_eq!(validate_history(&records[..count]).unwrap(), count);
        }
    }
    #[test]
    fn invalid_histories_report_the_intended_condition() {
        rejects(&[doc! { "version": "one" }], "version is missing");
        let valid = entry(&MIGRATIONS[0]);
        let mut unknown = valid.clone();
        unknown.insert("version", 99_i64);
        rejects(&[unknown], "unknown migration version");
        rejects(
            &[valid.clone(), valid.clone()],
            "duplicate migration version",
        );
        let mut wrong = valid.clone();
        wrong.insert("checksum", "wrong");
        rejects(&[wrong], "checksum differs");
        rejects(&[entry(&MIGRATIONS[1])], "not contiguous");
        let mut missing = valid;
        missing.remove("applied_at");
        rejects(&[missing], "applied_at is missing");
    }
    #[test]
    fn migration_checksums_cover_executable_commands() {
        for migration in &MIGRATIONS {
            let commands: Vec<Document> = serde_json::from_str(migration.commands).unwrap();
            assert!(!commands.is_empty());
            let changed = Migration {
                commands: "[]",
                ..*migration
            };
            assert_ne!(checksum(migration), checksum(&changed));
        }
    }

    #[tokio::test]
    #[ignore = "requires a disposable replica set; set WINDWATCHER_TEST_MONGODB_URL"]
    async fn interrupted_steps_resume_without_applying_later_versions_early() {
        let url = std::env::var("WINDWATCHER_TEST_MONGODB_URL").unwrap();
        let client = mongodb::Client::with_uri_str(url).await.unwrap();
        let db = client.database(&format!(
            "windwatcher_migrations_{}",
            uuid::Uuid::now_v7().simple()
        ));
        // Simulate a crash after executing v1 but before recording it. There
        // must be no v2/v3/v4 schema yet, and re-running v1 must be safe.
        apply_step(&db, &MIGRATIONS[0]).await.unwrap();
        assert!(collection_exists(&db, "users").await.unwrap());
        assert!(!collection_exists(&db, "conversations").await.unwrap());
        assert!(!collection_exists(&db, "messages").await.unwrap());
        assert!(!collection_exists(&db, "idempotency_records").await.unwrap());
        assert_eq!(status(&db).await.unwrap().pending(), 4);
        apply_step(&db, &MIGRATIONS[0]).await.unwrap();
        // Simulate an already recorded prefix as well as unrecorded work.
        db.collection::<Document>(LEDGER)
            .insert_one(entry(&MIGRATIONS[0]))
            .await
            .unwrap();
        apply_step(&db, &MIGRATIONS[1]).await.unwrap();
        assert_eq!(status(&db).await.unwrap().pending(), 3);
        assert!(!collection_exists(&db, "messages").await.unwrap());
        apply(&db).await.unwrap();
        assert!(status(&db).await.unwrap().is_current());
        assert_eq!(applied(&db).await.unwrap().len(), 4);
        db.drop().await.unwrap();
    }
}
