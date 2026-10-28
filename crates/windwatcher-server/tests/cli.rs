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
fn realtime_config_defaults_toml_environment_and_validation() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    let base = fs::read_to_string(&path).unwrap();
    command(&path).args(["config", "check"]).assert().success();
    for settings in [
        "ticket_ttl = '0s'",
        "outgoing_buffer = 0",
        "heartbeat_interval = '0s'",
        "heartbeat_timeout = '0s'",
        "heartbeat_timeout = '30s'",
        "session_recheck_interval = '0s'",
        "dispatch_timeout = '0s'",
        "dispatch_concurrency = 0",
        "unknown = 1",
    ] {
        fs::write(&path, format!("{base}\n[realtime]\n{settings}\n")).unwrap();
        command(&path).args(["config", "check"]).assert().failure();
    }
    fs::write(
        &path,
        format!("{base}\n[realtime]\nheartbeat_timeout = '40s'\n"),
    )
    .unwrap();
    command(&path)
        .env("WINDWATCHER__REALTIME__HEARTBEAT_INTERVAL", "60s")
        .args(["config", "check"])
        .assert()
        .success();
    command(&path)
        .env("WINDWATCHER__REALTIME__OUTGOING_BUFFER", "0")
        .args(["config", "check"])
        .assert()
        .failure();
}

#[test]
fn http_config_check_validates_defaults_toml_environment_and_unknown_fields() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    let base = fs::read_to_string(&path).unwrap();
    command(&path).args(["config", "check"]).assert().success();
    for http in [
        "[http.cors]\nallowed_origins = ['https://example.com', 'http://localhost:3000']",
        "[http.cors]\nallowed_origins = ['*']",
        "[http.rate_limit]\nenabled = false\n[http.rate_limit.auth]\nrequests = 2\nperiod = '1m'\nburst = 3",
    ] {
        fs::write(&path, format!("{base}\n{http}\n")).unwrap();
        command(&path).args(["config", "check"]).assert().success();
    }
    for http in [
        "[http.cors]\nallowed_origins = ['*', 'https://example.com']",
        "[http.cors]\nallowed_origins = ['not an origin']",
        "[http.rate_limit.general]\nrequests = 0",
        "[http.rate_limit.auth]\nburst = 0",
        "[http.rate_limit.auth]\nperiod = '0s'",
        "[http.rate_limit.auth]\nperiod = 'nonsense'",
        "[http]\nbind = '127.0.0.1:8000'",
        "[http.cors]\nallow_credentials = true",
        "[http.rate_limit.auth]\nunknown = true",
    ] {
        fs::write(&path, format!("{base}\n{http}\n")).unwrap();
        command(&path).args(["config", "check"]).assert().failure();
    }
    fs::write(
        &path,
        format!("{base}\n[http.cors]\nallowed_origins = ['invalid']\n"),
    )
    .unwrap();
    command(&path)
        .env(
            "WINDWATCHER__HTTP__CORS__ALLOWED_ORIGINS",
            "[\"https://example.com\",\"http://localhost:3000\"]",
        )
        .args(["config", "check"])
        .assert()
        .success();
    command(&path)
        .env("WINDWATCHER__HTTP__CORS__ALLOWED_ORIGINS", "[]")
        .args(["config", "check"])
        .assert()
        .success();
    command(&path)
        .env(
            "WINDWATCHER__HTTP__CORS__ALLOWED_ORIGINS",
            "https://example.com",
        )
        .args(["config", "check"])
        .assert()
        .failure();
    fs::write(&path, &base).unwrap();
    command(&path)
        .env("WINDWATCHER__HTTP__RATE_LIMIT__AUTH__BURST", "0")
        .args(["config", "check"])
        .assert()
        .failure();
    command(&path)
        .env("WINDWATCHER__HTTP__UNKNOWN", "true")
        .args(["config", "check"])
        .assert()
        .failure();
    assert!(!directory.path().join("windwatcher.db").exists());
    fs::write(
        &path,
        format!("{base}\n[http.cors]\nallowed_origins = ['invalid']\n"),
    )
    .unwrap();
    command(&path).arg("migrate").assert().success();
}

#[test]
fn message_config_defaults_toml_environment_and_unknown_fields() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    command(&path).args(["config", "check"]).assert().success();
    let base = fs::read_to_string(&path).unwrap();
    for maximum in [0, 4, 131072] {
        fs::write(
            &path,
            format!("{base}\n[messages]\nmax_content_bytes = {maximum}\n"),
        )
        .unwrap();
        command(&path).args(["config", "check"]).assert().success();
    }
    fs::write(
        &path,
        format!("{base}\n[messages]\nmax_content_bytes = -1\n"),
    )
    .unwrap();
    command(&path).args(["config", "check"]).assert().failure();
    command(&path)
        .env("WINDWATCHER__MESSAGES__MAX_CONTENT_BYTES", "0")
        .args(["config", "check"])
        .assert()
        .success();
    fs::write(&path, format!("{base}\n[messages]\nallow_blank = true\n")).unwrap();
    command(&path)
        .args(["config", "check"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown field"));
    fs::write(&path, &base).unwrap();
    command(&path)
        .env("WINDWATCHER__MESSAGES__MAX_CONTENT_BYTES", "invalid")
        .args(["config", "check"])
        .assert()
        .failure();
    command(&path)
        .env("WINDWATCHER__MESSAGES__ALLOW_BLANK", "true")
        .args(["config", "check"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown field"));
    assert!(!directory.path().join("windwatcher.db").exists());
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
        .stdout(predicate::str::contains("Applied 4 migration(s)."));
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
fn queue_migrations_are_separately_configured_and_targetable() {
    let directory = TempDir::new().unwrap();
    let config = config(&directory);
    command(&config)
        .args(["migrate", "--queue"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("queue is not configured"));
    let queue_config = format!(
        "\n[queue]\nprovider = \"sqlite\"\n\n[queue.sqlite]\npath = {:?}\n",
        directory.path().join("queue.db").display().to_string()
    );
    fs::write(
        &config,
        format!("{}{}", fs::read_to_string(&config).unwrap(), queue_config),
    )
    .unwrap();

    command(&config)
        .args(["migrate", "--queue"])
        .assert()
        .success();
    assert!(directory.path().join("queue.db").exists());
    assert!(!directory.path().join("windwatcher.db").exists());
    command(&config)
        .args(["migrate", "--queue", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Queue: SQLite"));
    command(&config)
        .args(["migrate", "status", "--queue"])
        .assert()
        .success();
    command(&config)
        .args(["migrate", "--storage"])
        .assert()
        .success();
    assert!(directory.path().join("windwatcher.db").exists());
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
fn password_file_preserves_exact_input_and_duplicate_user_is_operator_friendly() {
    let directory = TempDir::new().unwrap();
    let config = config(&directory);
    let password_file = directory.path().join("password");
    fs::write(&password_file, " leading password\n").unwrap();
    command(&config).arg("migrate").assert().success();
    command(&config)
        .args(["user", "create", "--username", "otavio", "--password-file"])
        .arg(&password_file)
        .assert()
        .success();
    command(&config)
        .args(["user", "create", "--username", "otavio", "--password-file"])
        .arg(&password_file)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "username \"otavio\" is already taken",
        ));
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

#[test]
fn explicit_missing_config_and_malformed_durations_fail_every_command() {
    let directory = TempDir::new().unwrap();
    command(&directory.path().join("missing.toml"))
        .arg("migrate")
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));

    let path = directory.path().join("invalid-duration.toml");
    fs::write(&path, "[auth.session]\naccess_token_ttl = \"banana\"\n").unwrap();
    command(&path)
        .arg("migrate")
        .assert()
        .failure()
        .stderr(predicate::str::contains("access_token_ttl"));
}

#[test]
fn missing_default_config_allows_environment_only_configuration() {
    let directory = TempDir::new().unwrap();
    let mut command = Command::cargo_bin("windwatcher").unwrap();
    command
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .env(
            "WINDWATCHER__STORAGE__SQLITE__URL",
            directory.path().join("windwatcher.db"),
        )
        .env(
            "WINDWATCHER__AUTH__JWT__SECRET",
            "12345678901234567890123456789012",
        )
        .args(["config", "check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Configuration valid."));
}

#[test]
fn session_policy_semantics_do_not_block_migration() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("semantic-session.toml");
    fs::write(
        &path,
        format!(
            "[storage.sqlite]\nurl = {:?}\n\n[auth.session]\naccess_token_ttl = \"500ms\"\n",
            directory
                .path()
                .join("windwatcher.db")
                .display()
                .to_string()
        ),
    )
    .unwrap();
    command(&path).arg("migrate").assert().success();
    command(&path)
        .args(["config", "check"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("at least one second"));
}

#[test]
fn environment_jwt_source_replaces_toml_source_and_preserves_literal_values() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("secret-precedence.toml");
    fs::write(
        &path,
        format!(
            "[storage.sqlite]\nurl = {:?}\n\n[auth.jwt]\nsecret_file = {:?}\n",
            directory
                .path()
                .join("windwatcher.db")
                .display()
                .to_string(),
            directory
                .path()
                .join("missing-old-secret")
                .display()
                .to_string(),
        ),
    )
    .unwrap();
    command(&path)
        .env(
            "WINDWATCHER__AUTH__JWT__SECRET",
            "[abcdefghijklmnopqrstuvwxyz123456]",
        )
        .args(["config", "check"])
        .assert()
        .success();

    let replacement_file = directory.path().join("replacement-secret");
    fs::write(&replacement_file, "abcdefghijklmnopqrstuvwxyz123456").unwrap();
    fs::write(
        &path,
        "[auth.jwt]\nsecret = \"abcdefghijklmnopqrstuvwxyz123456\"\n",
    )
    .unwrap();
    command(&path)
        .env("WINDWATCHER__AUTH__JWT__SECRET_FILE", &replacement_file)
        .args(["config", "check"])
        .assert()
        .success();
}

#[test]
fn conflicting_environment_jwt_sources_are_rejected() {
    let directory = TempDir::new().unwrap();
    let config = complete_config(&directory, "warn");
    command(&config)
        .env(
            "WINDWATCHER__AUTH__JWT__SECRET",
            "abcdefghijklmnopqrstuvwxyz123456",
        )
        .env(
            "WINDWATCHER__AUTH__JWT__SECRET_FILE",
            directory.path().join("secret"),
        )
        .arg("migrate")
        .assert()
        .success();
    command(&config)
        .env(
            "WINDWATCHER__AUTH__JWT__SECRET",
            "abcdefghijklmnopqrstuvwxyz123456",
        )
        .env(
            "WINDWATCHER__AUTH__JWT__SECRET_FILE",
            directory.path().join("secret"),
        )
        .args(["config", "check"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("are mutually exclusive"));
}

#[test]
fn config_check_validates_sqlite_options_without_creating_a_database() {
    let directory = TempDir::new().unwrap();
    let invalid = directory.path().join("invalid-sqlite.toml");
    fs::write(
        &invalid,
        "[storage.sqlite]\nurl = \"sqlite://windwatcher.db?mode=invalid\"\n\n[auth.jwt]\nsecret = \"abcdefghijklmnopqrstuvwxyz123456\"\n",
    )
    .unwrap();
    command(&invalid)
        .args(["config", "check"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "invalid SQLite storage configuration",
        ));

    let database = directory.path().join("not-created.db");
    let valid = directory.path().join("valid-sqlite.toml");
    fs::write(
        &valid,
        format!(
            "[storage.sqlite]\nurl = {:?}\n\n[auth.jwt]\nsecret = \"abcdefghijklmnopqrstuvwxyz123456\"\n",
            database.display().to_string()
        ),
    )
    .unwrap();
    command(&valid).args(["config", "check"]).assert().success();
    assert!(!database.exists());
}

#[test]
fn password_argument_warning_precedes_schema_validation() {
    let directory = TempDir::new().unwrap();
    let config = config(&directory);
    command(&config)
        .args([
            "user",
            "create",
            "--username",
            "otavio",
            "--password",
            "correct horse battery staple",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("warning: --password")
                .and(predicate::str::contains("database schema is not current")),
        );
}
