use std::{
    fs,
    io::{self, IsTerminal, Read},
    path::Path,
};

use time::OffsetDateTime;
use windwatcher::{Argon2idPasswordHasher, CreateLocalUser, CreateLocalUserError, SecretPassword};

use crate::{cli::CreateUserArgs, error::AppError, storage::Storage};

pub async fn migrate(storage: &Storage) -> Result<(), AppError> {
    let before = storage
        .migration_status()
        .await
        .map_err(|error| AppError::MigrationStatus(Box::new(error)))?;
    storage
        .migrate()
        .await
        .map_err(|error| AppError::Migration(Box::new(error)))?;
    let after = storage
        .migration_status()
        .await
        .map_err(|error| AppError::MigrationStatus(Box::new(error)))?;
    if before.pending() == 0 {
        println!("No pending migrations.");
    } else {
        println!("Applied {} migration(s).", before.pending());
    }
    println!(
        "Schema is current at version {}.",
        after.latest_version().unwrap_or_default()
    );
    Ok(())
}

pub async fn migration_status(storage: &Storage) -> Result<(), AppError> {
    let status = storage
        .migration_status()
        .await
        .map_err(|error| AppError::MigrationStatus(Box::new(error)))?;
    println!("Storage: {}", storage.backend_name());
    println!(
        "Current version: {}",
        status
            .current_version()
            .map_or_else(|| "none".into(), |version| version.to_string())
    );
    println!(
        "Latest version: {}",
        status
            .latest_version()
            .map_or_else(|| "none".into(), |version| version.to_string())
    );
    println!("Pending migrations: {}", status.pending());
    println!(
        "Status: {}",
        if status.is_current() {
            "current"
        } else {
            "pending"
        }
    );
    Ok(())
}

pub async fn create_user(storage: &Storage, args: &CreateUserArgs) -> Result<(), AppError> {
    let status = storage
        .migration_status()
        .await
        .map_err(|error| AppError::MigrationStatus(Box::new(error)))?;
    if !status.is_current() {
        return Err(AppError::SchemaPending);
    }
    let password = password(args)?;
    tracing::info!(username = %args.username, "user provisioning started");
    let hasher = Argon2idPasswordHasher;
    let user = CreateLocalUser::new(storage, &hasher)
        .execute(&args.username, &password, OffsetDateTime::now_utc())
        .await
        .map_err(|error| match error {
            CreateLocalUserError::UsernameTaken => AppError::UsernameTaken {
                username: args.username.clone(),
            },
            other => AppError::UserCreation(other),
        })?;
    println!(
        "Created user {:?} ({})",
        user.username().as_str(),
        user.id()
    );
    tracing::info!(user_id = %user.id(), "user provisioning succeeded");
    Ok(())
}

fn password(args: &CreateUserArgs) -> Result<SecretPassword, AppError> {
    let value = match (&args.password, args.password_stdin, &args.password_file) {
        (Some(password), false, None) => {
            eprintln!(
                "warning: --password may expose the password through process arguments; prefer interactive input, --password-stdin, or --password-file"
            );
            password.clone()
        }
        (None, true, None) => read_password(&mut io::stdin())?,
        (None, false, Some(path)) => read_password_file(path)?,
        (None, false, None) => {
            if !io::stdin().is_terminal() {
                return Err(AppError::PasswordInput("stdin is not interactive; use --password, --password-stdin, or --password-file".into()));
            }
            let first = rpassword::prompt_password("Password: ")
                .map_err(|error| AppError::PasswordInput(error.to_string()))?;
            let confirmation = rpassword::prompt_password("Confirm password: ")
                .map_err(|error| AppError::PasswordInput(error.to_string()))?;
            if first != confirmation {
                return Err(AppError::PasswordInput(
                    "password confirmation does not match".into(),
                ));
            }
            first
        }
        _ => {
            return Err(AppError::PasswordInput(
                "password sources are mutually exclusive".into(),
            ));
        }
    };
    SecretPassword::new(value).map_err(|error| AppError::PasswordInput(error.to_string()))
}

fn read_password_file(path: &Path) -> Result<String, AppError> {
    fs::File::open(path)
        .map_err(|error| {
            AppError::PasswordInput(format!(
                "failed to read password file {}: {error}",
                path.display()
            ))
        })
        .and_then(|mut file| read_password(&mut file))
}
fn read_password(reader: &mut impl Read) -> Result<String, AppError> {
    let mut bytes = Vec::with_capacity(1025);
    reader
        .take(1025)
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::PasswordInput(error.to_string()))?;
    if bytes.len() > 1024 {
        return Err(AppError::PasswordInput(
            "password must contain at most 1024 UTF-8 bytes".into(),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| AppError::PasswordInput("password input must be valid UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::read_password;

    #[test]
    fn password_input_preserves_exact_utf8_bytes() {
        assert_eq!(
            read_password(&mut Cursor::new(" leading\ntrailing ")).unwrap(),
            " leading\ntrailing "
        );
        assert_eq!(
            read_password(&mut Cursor::new("å".repeat(512)))
                .unwrap()
                .len(),
            1024
        );
    }

    #[test]
    fn password_input_rejects_invalid_utf8_and_oversized_values() {
        assert!(read_password(&mut Cursor::new(vec![0xff])).is_err());
        assert!(read_password(&mut Cursor::new(vec![b'x'; 1025])).is_err());
    }
}
