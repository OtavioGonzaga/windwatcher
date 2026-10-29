use futures_util::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{Document, doc},
    error::ErrorKind,
    options::{Collation, IndexOptions},
};
use sha2::{Digest, Sha256};

use super::{MigrationStatus, MongoDbStorageError};

const LEDGER: &str = "_windwatcher_migrations";
const COLLECTIONS: [&str; 7] = [
    "users",
    "local_credentials",
    "sessions",
    "conversations",
    "conversation_members",
    "messages",
    "idempotency_records",
];

struct Migration {
    version: i64,
    name: &'static str,
    checksum_input: &'static str,
}

const MIGRATIONS: [Migration; 4] = [
    Migration {
        version: 1,
        name: "identity_sessions",
        checksum_input: "v1:users username unique;credentials; sessions refresh unique user index",
    },
    Migration {
        version: 2,
        name: "conversations_memberships",
        checksum_input: "v2:conversations direct partial unique; memberships identity/user-conversation/joined-user/one-owner indexes",
    },
    Migration {
        version: 3,
        name: "messages_read_state",
        checksum_input: "v3:messages conversation-id ascending and message-id descending",
    },
    Migration {
        version: 4,
        name: "queries_idempotency",
        checksum_input: "v4:idempotency actor-operation-scope-key unique; expires index",
    },
];

fn checksum(migration: &Migration) -> String {
    let digest = Sha256::digest(migration.checksum_input.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn schema_error(error: mongodb::error::Error) -> MongoDbStorageError {
    MongoDbStorageError::Schema(Box::new(error))
}

fn migration_error(error: mongodb::error::Error) -> MongoDbStorageError {
    MongoDbStorageError::Migration(Box::new(error))
}

async fn collection_exists(database: &Database, name: &str) -> Result<bool, mongodb::error::Error> {
    Ok(database
        .list_collection_names()
        .await?
        .iter()
        .any(|collection| collection == name))
}

async fn ensure_collection(database: &Database, name: &str) -> Result<(), mongodb::error::Error> {
    if collection_exists(database, name).await? {
        return Ok(());
    }
    match database.create_collection(name).await {
        Ok(_) => Ok(()),
        Err(error) if matches!(error.kind.as_ref(), ErrorKind::Command(command) if command.code == 48) => {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn index(keys: Document, name: &str, unique: bool, partial: Option<Document>) -> IndexModel {
    let mut options = IndexOptions::builder()
        .name(name.to_owned())
        .unique(unique)
        .build();
    if let Some(filter) = partial {
        options.partial_filter_expression = Some(filter);
    }
    IndexModel::builder().keys(keys).options(options).build()
}

async fn apply_indexes(database: &Database) -> Result<(), mongodb::error::Error> {
    let mut username_index = index(doc! { "username": 1 }, "users_username_unique", true, None);
    username_index
        .options
        .as_mut()
        .expect("index options initialized")
        .collation = Some(Collation::builder().locale("simple").build());
    database
        .collection::<Document>("users")
        .create_index(username_index)
        .await?;
    database
        .collection::<Document>("sessions")
        .create_index(index(
            doc! { "refresh_token_hash": 1 },
            "sessions_refresh_hash_unique",
            true,
            None,
        ))
        .await?;
    database
        .collection::<Document>("sessions")
        .create_index(index(
            doc! { "user_id": 1 },
            "sessions_user_id",
            false,
            None,
        ))
        .await?;
    database
        .collection::<Document>("conversations")
        .create_index(index(
            doc! { "direct_key": 1 },
            "conversations_direct_key_unique",
            true,
            Some(doc! { "kind": "direct" }),
        ))
        .await?;
    database
        .collection::<Document>("conversation_members")
        .create_index(index(
            doc! { "conversation_id": 1, "user_id": 1 },
            "members_identity_unique",
            true,
            None,
        ))
        .await?;
    database
        .collection::<Document>("conversation_members")
        .create_index(index(
            doc! { "user_id": 1, "conversation_id": 1 },
            "members_user_conversation",
            false,
            None,
        ))
        .await?;
    database
        .collection::<Document>("conversation_members")
        .create_index(index(
            doc! { "conversation_id": 1, "joined_at": 1, "user_id": 1 },
            "members_conversation_joined_user",
            false,
            None,
        ))
        .await?;
    database
        .collection::<Document>("conversation_members")
        .create_index(index(
            doc! { "conversation_id": 1 },
            "members_one_owner",
            true,
            Some(doc! { "role": "owner" }),
        ))
        .await?;
    database
        .collection::<Document>("messages")
        .create_index(index(
            doc! { "conversation_id": 1, "_id": -1 },
            "messages_conversation_id",
            false,
            None,
        ))
        .await?;
    database
        .collection::<Document>("idempotency_records")
        .create_index(index(
            doc! { "actor_id": 1, "operation": 1, "scope_id": 1, "key_hash": 1 },
            "idempotency_identity_unique",
            true,
            None,
        ))
        .await?;
    database
        .collection::<Document>("idempotency_records")
        .create_index(index(
            doc! { "expires_at": 1 },
            "idempotency_expires_at",
            false,
            None,
        ))
        .await?;
    Ok(())
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
    let mut seen = Vec::new();
    for record in records {
        let version = record.get_i64("version").map_err(|_| {
            MongoDbStorageError::InvalidMigrationHistory(
                "ledger version is missing or invalid".into(),
            )
        })?;
        let name = record.get_str("name").map_err(|_| {
            MongoDbStorageError::InvalidMigrationHistory("ledger name is missing or invalid".into())
        })?;
        let stored_checksum = record.get_str("checksum").map_err(|_| {
            MongoDbStorageError::InvalidMigrationHistory(
                "ledger checksum is missing or invalid".into(),
            )
        })?;
        record.get_i64("applied_at").map_err(|_| {
            MongoDbStorageError::InvalidMigrationHistory(
                "ledger applied_at is missing or invalid".into(),
            )
        })?;
        let Some(expected) = MIGRATIONS
            .iter()
            .find(|migration| migration.version == version)
        else {
            return Err(MongoDbStorageError::InvalidMigrationHistory(format!(
                "unknown migration version {version}"
            )));
        };
        if name != expected.name || stored_checksum != checksum(expected) {
            return Err(MongoDbStorageError::InvalidMigrationHistory(format!(
                "migration {version} name or checksum differs"
            )));
        }
        if seen.contains(&version) {
            return Err(MongoDbStorageError::InvalidMigrationHistory(format!(
                "duplicate migration version {version}"
            )));
        }
        seen.push(version);
    }
    if seen
        .iter()
        .enumerate()
        .any(|(index, version)| *version != index as i64 + 1)
    {
        return Err(MongoDbStorageError::InvalidMigrationHistory(
            "migration versions are not contiguous".into(),
        ));
    }
    Ok(seen.len())
}

pub(super) async fn status(database: &Database) -> Result<MigrationStatus, MongoDbStorageError> {
    let records = applied(database).await?;
    let count = validate_history(&records)?;
    Ok(MigrationStatus {
        current_version: count.checked_sub(1).map(|index| MIGRATIONS[index].version),
        latest_version: MIGRATIONS.last().map(|migration| migration.version),
        pending: MIGRATIONS.len() - count,
    })
}

pub(super) async fn apply(database: &Database) -> Result<(), MongoDbStorageError> {
    ensure_collection(database, LEDGER)
        .await
        .map_err(migration_error)?;
    for name in COLLECTIONS {
        ensure_collection(database, name)
            .await
            .map_err(migration_error)?;
    }
    apply_indexes(database).await.map_err(migration_error)?;
    for migration in MIGRATIONS {
        let records = applied(database).await?;
        let count = validate_history(&records)?;
        if count as i64 >= migration.version {
            continue;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| MongoDbStorageError::Migration(Box::new(error)))?;
        let applied_at = i64::try_from(now.as_micros())
            .map_err(|error| MongoDbStorageError::Migration(Box::new(error)))?;
        let entry = doc! {
            "version": migration.version,
            "name": migration.name,
            "checksum": checksum(&migration),
            "applied_at": applied_at,
        };
        match database
            .collection::<Document>(LEDGER)
            .insert_one(entry)
            .await
        {
            Ok(_) => (),
            Err(error) if matches!(error.kind.as_ref(), ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000) =>
            {
                let records = applied(database).await?;
                validate_history(&records)?;
            }
            Err(error) => return Err(migration_error(error)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;

    use super::{MIGRATIONS, MongoDbStorageError, checksum, validate_history};

    #[test]
    fn empty_and_complete_histories_are_valid() {
        assert_eq!(validate_history(&[]).unwrap(), 0);
        let records: Vec<_> = MIGRATIONS
            .iter()
            .map(|migration| {
                doc! {
                    "version": migration.version,
                    "name": migration.name,
                    "checksum": checksum(migration),
                    "applied_at": 1_i64,
                }
            })
            .collect();
        assert_eq!(validate_history(&records).unwrap(), MIGRATIONS.len());
    }

    #[test]
    fn malformed_unknown_duplicate_and_mismatched_history_is_rejected() {
        assert!(matches!(
            validate_history(&[doc! { "version": "one" }]),
            Err(MongoDbStorageError::InvalidMigrationHistory(_))
        ));
        assert!(matches!(
            validate_history(&[doc! { "version": 99_i64, "name": "unknown", "checksum": "x" }]),
            Err(MongoDbStorageError::InvalidMigrationHistory(_))
        ));
        let first = &MIGRATIONS[0];
        let valid = doc! {
            "version": first.version,
            "name": first.name,
            "checksum": checksum(first),
        };
        assert!(matches!(
            validate_history(&[valid.clone(), valid]),
            Err(MongoDbStorageError::InvalidMigrationHistory(_))
        ));
        assert!(matches!(
            validate_history(&[
                doc! { "version": first.version, "name": first.name, "checksum": "wrong" }
            ]),
            Err(MongoDbStorageError::InvalidMigrationHistory(_))
        ));
    }
}
