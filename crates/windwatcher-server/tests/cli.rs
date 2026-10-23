use std::{fs, path::Path};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn config(directory: &TempDir) -> std::path::PathBuf {
    let path = directory.path().join("config.toml");
    fs::write(
        &path,
        format!(
            "[storage.sqlite]\nurl = {:?}\n\n[logging]\nfilter = \"warn\"\nformat = \"compact\"\n",
            directory
                .path()
                .join("windwatcher.db")
                .display()
                .to_string()
        ),
    )
    .unwrap();
    path
}

fn complete_config(directory: &TempDir, filter: &str) -> std::path::PathBuf {
    let path = directory.path().join("complete.toml");
    fs::write(
        &path,
        format!(
            "[storage.sqlite]\nurl = {:?}\n\n[auth.jwt]\nsecret = \"12345678901234567890123456789012\"\n\n[logging]\nfilter = {:?}\nformat = \"compact\"\n",
            directory.path().join("windwatcher.db").display().to_string(), filter
        ),
    )
    .unwrap();
    path
}

fn command(config: &Path) -> Command {
    let mut command = Command::cargo_bin("windwatcher").unwrap();
    command.arg("--config").arg(config);
    command
}

#[test]
fn migrations_are_explicit_and_user_provisioning_uses_current_schema() {
    let directory = TempDir::new().unwrap();
    let config = config(&directory);

    command(&config)
        .args(["user", "create", "--username", "otavio", "--password-stdin"])
        .write_stdin("correct horse battery staple")
        .assert()
        .failure()
        .stderr(predicate::str::contains("database schema is not current"));

    command(&config)
        .arg("migrate")
        .assert()
        .success()
        .stdout(predicate::str::contains("Applied 1 migration(s)."));
    command(&config)
        .args(["migrate", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status: current"));

    command(&config)
        .args(["user", "create", "--username", "otavio", "--password-stdin"])
        .write_stdin("correct horse battery staple")
        .assert()
        .success()
        .stdout(predicate::str::contains("Created user \"otavio\""));
}

#[test]
fn config_check_resolves_jwt_but_migrate_does_not() {
    let directory = TempDir::new().unwrap();
    let config = config(&directory);
    command(&config).arg("migrate").assert().success();
    command(&config)
        .args(["config", "check"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("JWT signing secret is required"));
}

#[test]
fn argument_password_warns_without_echoing_the_secret() {
    let directory = TempDir::new().unwrap();
    let config = config(&directory);
    command(&config).arg("migrate").assert().success();
    let secret = "correct horse battery staple";
    command(&config)
        .args([
            "user",
            "create",
            "--username",
            "otavio",
            "--password",
            secret,
        ])
        .assert()
        .success()
        .stderr(
            predicate::str::contains("warning: --password")
                .and(predicate::str::contains(secret).not()),
        );
}

#[test]
fn logging_cli_and_environment_overrides_take_precedence_over_toml() {
    let directory = TempDir::new().unwrap();
    let config = complete_config(&directory, "[");
    command(&config)
        .args(["--log-filter", "warn", "config", "check"])
        .assert()
        .success();
    command(&config)
        .env("WINDWATCHER__LOGGING__FILTER", "warn")
        .args(["config", "check"])
        .assert()
        .success();
}

#[test]
fn unknown_configuration_fields_are_rejected() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("unknown.toml");
    fs::write(&path, "[storage.sqlite]\nmax_conections = 8\n").unwrap();
    command(&path)
        .arg("migrate")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown field"));
}
