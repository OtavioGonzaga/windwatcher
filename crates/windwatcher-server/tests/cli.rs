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
fn server_configuration_overrides_and_validation_are_static() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    for bind in ["127.0.0.1:0", "[::1]:0"] {
        command(&path)
            .env("WINDWATCHER__SERVER__BIND", bind)
            .args(["config", "check"])
            .assert()
            .success();
    }
    for (key, value) in [
        ("BIND", "localhost:8080"),
        ("SHUTDOWN_GRACE_PERIOD", "0s"),
        ("READINESS_TIMEOUT", "0s"),
        ("UNKNOWN", "true"),
    ] {
        command(&path)
            .env(format!("WINDWATCHER__SERVER__{key}"), value)
            .args(["config", "check"])
            .assert()
            .failure();
    }
    assert!(!directory.path().join("windwatcher.db").exists());
}

#[test]
fn serve_preflight_requires_existing_current_storage_before_binding() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "info");
    let base = fs::read_to_string(&path).unwrap();
    // An occupied address must not mask the earlier schema error.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    fs::write(
        &path,
        format!(
            "{base}\n[server]\nbind = '{}'\n",
            listener.local_addr().unwrap()
        ),
    )
    .unwrap();
    for args in [vec![], vec!["serve"]] {
        command(&path)
            .args(args)
            .assert()
            .failure()
            .stderr(predicate::str::contains("failed to open SQLite database"));
        assert!(!directory.path().join("windwatcher.db").exists());
    }
    fs::File::create(directory.path().join("windwatcher.db")).unwrap();
    command(&path)
        .arg("serve")
        .assert()
        .failure()
        .stderr(predicate::str::contains("windwatcher migrate --storage"))
        .stderr(predicate::str::contains("server_listening").not());
    command(&path).arg("migrate").assert().success();
    command(&path)
        .arg("serve")
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to bind server listener"));
}

#[test]
fn doctor_is_read_only_and_reports_storage_and_optional_queue_independently() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    command(&path)
        .arg("doctor")
        .assert()
        .failure()
        .stdout(predicate::str::contains("[fail] storage:"))
        .stdout(predicate::str::contains("[ok] queue: disabled"))
        .stdout(predicate::str::contains("Doctor: unhealthy"));
    assert!(!directory.path().join("windwatcher.db").exists());
    fs::File::create(directory.path().join("windwatcher.db")).unwrap();
    command(&path)
        .arg("doctor")
        .assert()
        .failure()
        .stdout(predicate::str::contains("storage migrations: pending"))
        .stdout(predicate::str::contains("windwatcher migrate --storage"));
    command(&path)
        .args(["migrate", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status: pending"));
    command(&path).arg("migrate").assert().success();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    command(&path)
        .env(
            "WINDWATCHER__SERVER__BIND",
            listener.local_addr().unwrap().to_string(),
        )
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("storage migrations: current"))
        .stdout(predicate::str::contains("Doctor: healthy"));
    let base = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        format!(
            "{base}\n[queue]\n[queue.sqlite]\npath = {:?}\n",
            directory.path().join("queue.db").display().to_string()
        ),
    )
    .unwrap();
    command(&path).arg("doctor").assert().failure();
    assert!(!directory.path().join("queue.db").exists());
    fs::File::create(directory.path().join("queue.db")).unwrap();
    command(&path)
        .arg("doctor")
        .assert()
        .failure()
        .stdout(predicate::str::contains("queue migrations: pending"))
        .stdout(predicate::str::contains("windwatcher migrate --queue"));
    command(&path)
        .args(["migrate", "--queue"])
        .assert()
        .success();
    command(&path).arg("doctor").assert().success();
    command(&path)
        .env("WINDWATCHER__STORAGE__PROVIDER", "postgres")
        .env(
            "WINDWATCHER__STORAGE__POSTGRES__URL",
            "postgresql://user:TOP_SECRET@127.0.0.1:1/db",
        )
        .arg("doctor")
        .assert()
        .failure()
        .stdout(predicate::str::contains("[fail] storage:"))
        .stdout(predicate::str::contains("[ok] queue: SQLite"))
        .stdout(predicate::str::contains("TOP_SECRET").not())
        .stderr(predicate::str::contains("TOP_SECRET").not());
}

#[test]
fn doctor_reports_invalid_configuration_without_quoting_input() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    for (key, value) in [
        ("WINDWATCHER__SERVER__BIND", "TOP_SECRET"),
        ("WINDWATCHER__QUEUE__CONCURRENCY", "TOP_SECRET"),
    ] {
        command(&path)
            .env(key, value)
            .arg("doctor")
            .assert()
            .failure()
            .stdout(predicate::str::contains("[fail] configuration"))
            .stdout(predicate::str::contains("Doctor: unhealthy"))
            .stdout(predicate::str::contains("TOP_SECRET").not())
            .stderr(predicate::str::contains("TOP_SECRET").not());
    }
}

#[test]
fn doctor_redacts_unavailable_network_dependencies() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    let base = fs::read_to_string(&path).unwrap();
    for (provider, url) in [
        ("postgres", "postgresql://user:TOP_SECRET@127.0.0.1:1/db"),
        ("mysql", "mysql://user:TOP_SECRET@127.0.0.1:1/db"),
        (
            "mongodb",
            "mongodb://user:TOP_SECRET@127.0.0.1:1/?serverSelectionTimeoutMS=10",
        ),
    ] {
        fs::write(&path, format!("{base}\n[storage]\nprovider = '{provider}'\n[storage.{provider}]\nurl = '{url}'\n{}", if provider == "mongodb" { "database = 'doctor'" } else { "acquire_timeout = '10ms'" })).unwrap();
        command(&path)
            .arg("doctor")
            .assert()
            .failure()
            .stdout(predicate::str::contains("[fail] storage:"))
            .stdout(predicate::str::contains("TOP_SECRET").not())
            .stderr(predicate::str::contains("TOP_SECRET").not());
    }
    for (provider, url) in [
        ("postgres", "postgresql://user:TOP_SECRET@127.0.0.1:1/db"),
        ("mysql", "mysql://user:TOP_SECRET@127.0.0.1:1/db"),
        ("redis", "redis://:TOP_SECRET@127.0.0.1:1/0"),
    ] {
        fs::write(
            &path,
            format!(
                "{base}\n[queue]\nprovider = '{provider}'\n[queue.{provider}]\nurl = '{url}'\n{}",
                if provider == "redis" {
                    ""
                } else {
                    "acquire_timeout = '10ms'"
                }
            ),
        )
        .unwrap();
        command(&path)
            .arg("doctor")
            .assert()
            .failure()
            .stdout(predicate::str::contains("[fail] queue:"))
            .stdout(predicate::str::contains("TOP_SECRET").not())
            .stderr(predicate::str::contains("TOP_SECRET").not());
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL, MySQL and MongoDB replica set; set WINDWATCHER_TEST_*_URL"]
async fn network_primary_doctor_composition() {
    use sqlx::{ConnectOptions, Connection};
    for (provider, variable) in [
        ("postgres", "WINDWATCHER_TEST_POSTGRES_URL"),
        ("mysql", "WINDWATCHER_TEST_MYSQL_URL"),
        ("mongodb", "WINDWATCHER_TEST_MONGODB_URL"),
    ] {
        let base_url = std::env::var(variable).expect("set the disposable database URL");
        let name = format!("windwatcher_test_{}", uuid::Uuid::now_v7().simple());
        let mut postgres = None;
        let mut mysql = None;
        let url = match provider {
            "postgres" => {
                let mut connection = sqlx::PgConnection::connect(&base_url).await.unwrap();
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
                    .execute(&mut connection)
                    .await
                    .unwrap();
                postgres = Some(connection);
                base_url
                    .parse::<sqlx::postgres::PgConnectOptions>()
                    .unwrap()
                    .database(&name)
                    .to_url_lossy()
                    .to_string()
            }
            "mysql" => {
                let mut connection = sqlx::MySqlConnection::connect(&base_url).await.unwrap();
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
                    .execute(&mut connection)
                    .await
                    .unwrap();
                mysql = Some(connection);
                base_url
                    .parse::<sqlx::mysql::MySqlConnectOptions>()
                    .unwrap()
                    .database(&name)
                    .to_url_lossy()
                    .to_string()
            }
            _ => base_url,
        };
        let directory = TempDir::new().unwrap();
        let path = complete_config(&directory, "warn");
        let base = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{base}\n[storage]\nprovider = '{provider}'\n[storage.{provider}]\nurl = {url:?}\n{}", if provider == "mongodb" { format!("database = '{name}'") } else { String::new() })).unwrap();
        command(&path)
            .arg("doctor")
            .assert()
            .failure()
            .stdout(predicate::str::contains("storage migrations: pending"));
        command(&path)
            .args(["migrate", "--storage"])
            .assert()
            .success();
        command(&path)
            .arg("doctor")
            .assert()
            .success()
            .stdout(predicate::str::contains("storage migrations: current"));
        if let Some(mut connection) = postgres {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE {name} WITH (FORCE)"
            )))
            .execute(&mut connection)
            .await
            .unwrap();
        }
        if let Some(mut connection) = mysql {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}")))
                .execute(&mut connection)
                .await
                .unwrap();
        }
        // MongoDB's UUID-named database lives only in the disposable replica set;
        // discard that service after the run, as for the adapter contract suite.
    }
}

#[test]
fn network_queue_config_selection_and_safe_errors() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    let base = fs::read_to_string(&path).unwrap();
    let secret = "recognizable_fake_password";
    for (provider, scheme, label) in [
        ("postgres", "postgresql", "PostgreSQL"),
        ("mysql", "mysql", "MySQL"),
        ("redis", "redis", "Redis"),
    ] {
        fs::write(&path,format!("{base}\n[queue]\nprovider = '{provider}'\n[queue.{provider}]\nurl = '{scheme}://user:{secret}@localhost/0'\n")).unwrap();
        command(&path)
            .args(["config", "check"])
            .assert()
            .success()
            .stdout(predicate::str::contains(format!("Queue: {label}")))
            .stdout(predicate::str::contains(secret).not())
            .stderr(predicate::str::contains(secret).not());
        fs::write(
            &path,
            format!(
                "{base}\n[queue]\nprovider = '{provider}'\n[queue.{provider}]\nurl = '{secret}'\n"
            ),
        )
        .unwrap();
        command(&path)
            .args(["config", "check"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(format!("queue.{provider}.url")))
            .stderr(predicate::str::contains(secret).not());
    }
}

#[test]
fn network_storage_config_selection_and_safe_errors() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    let base = fs::read_to_string(&path).unwrap();
    let secret = "primary_storage_fake_password";
    for (provider, scheme, label) in [
        ("postgres", "postgresql", "PostgreSQL"),
        ("mysql", "mysql", "MySQL"),
    ] {
        fs::write(&path, format!("{base}\n[storage]\nprovider = '{provider}'\n[storage.{provider}]\nurl = '{scheme}://user:{secret}@localhost/windwatcher'\n")).unwrap();
        command(&path)
            .args(["config", "check"])
            .assert()
            .success()
            .stdout(predicate::str::contains(format!("Storage: {label}")))
            .stdout(predicate::str::contains(secret).not())
            .stderr(predicate::str::contains(secret).not());
        fs::write(&path, format!("{base}\n[storage]\nprovider = '{provider}'\n[storage.{provider}]\nurl = '{secret}'\n")).unwrap();
        command(&path)
            .args(["config", "check"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(secret).not());
    }
}

#[test]
fn mongodb_config_check_parses_urls_without_connecting_or_disclosing_them() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    let base = fs::read_to_string(&path).unwrap();
    let secret = "primary_mongo_fake_password";
    for (url, valid) in [
        (
            format!("mongodb://user:{secret}@127.0.0.1:1/?replicaSet=rs0"),
            true,
        ),
        (format!("https://user:{secret}@localhost"), false),
        (format!("mongodb://user:{secret}@localhost:invalid"), false),
    ] {
        fs::write(&path, format!("{base}\n[storage]\nprovider = 'mongodb'\n[storage.mongodb]\nurl = '{url}'\ndatabase = 'windwatcher'\n")).unwrap();
        let result = command(&path).args(["config", "check"]).assert();
        if valid {
            result
                .success()
                .stdout(predicate::str::contains("Storage: MongoDB"))
                .stdout(predicate::str::contains(secret).not())
                .stderr(predicate::str::contains(secret).not());
        } else {
            result
                .failure()
                .stderr(predicate::str::contains("invalid MongoDB URL"))
                .stdout(predicate::str::contains(secret).not())
                .stderr(predicate::str::contains(secret).not());
        }
    }
    assert!(!directory.path().join("windwatcher.db").exists());
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL, MySQL and Redis; set WINDWATCHER_TEST_*_URL"]
async fn network_queue_migration_and_jobs_cli() {
    use sqlx::{ConnectOptions, Connection};
    for (provider, variable) in [
        ("postgres", "WINDWATCHER_TEST_POSTGRES_URL"),
        ("mysql", "WINDWATCHER_TEST_MYSQL_URL"),
        ("redis", "WINDWATCHER_TEST_REDIS_URL"),
    ] {
        let base_url = std::env::var(variable)
            .expect("set all WINDWATCHER_TEST_{POSTGRES,MYSQL,REDIS}_URL values");
        let name = format!("windwatcher_test_{}", uuid::Uuid::now_v7().simple());
        let mut pg = None;
        let mut mysql = None;
        let url = match provider {
            "postgres" => {
                let mut conn = sqlx::PgConnection::connect(&base_url).await.unwrap();
                // Identifier is only a fixed prefix plus generated UUID hex.
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
                    .execute(&mut conn)
                    .await
                    .unwrap();
                pg = Some(conn);
                base_url
                    .parse::<sqlx::postgres::PgConnectOptions>()
                    .unwrap()
                    .database(&name)
                    .to_url_lossy()
                    .to_string()
            }
            "mysql" => {
                let mut conn = sqlx::MySqlConnection::connect(&base_url).await.unwrap();
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
                    .execute(&mut conn)
                    .await
                    .unwrap();
                mysql = Some(conn);
                base_url
                    .parse::<sqlx::mysql::MySqlConnectOptions>()
                    .unwrap()
                    .database(&name)
                    .to_url_lossy()
                    .to_string()
            }
            _ => base_url,
        };
        let directory = TempDir::new().unwrap();
        let path = complete_config(&directory, "warn");
        let base = fs::read_to_string(&path).unwrap();
        fs::write(
            &path,
            format!(
                "{base}\n[queue]\nprovider = '{provider}'\n[queue.{provider}]\nurl = {url:?}\n{}",
                if provider == "redis" {
                    format!("key_prefix = 'windwatcher:test:{name}'\n")
                } else {
                    String::new()
                }
            ),
        )
        .unwrap();
        command(&path)
            .args(["migrate", "--storage"])
            .assert()
            .success();
        let diagnosis = command(&path).arg("doctor").assert();
        if provider == "redis" {
            diagnosis
                .success()
                .stdout(predicate::str::contains("queue migrations: not applicable"));
        } else {
            diagnosis
                .failure()
                .stdout(predicate::str::contains("queue migrations: pending"));
        }
        let initial = command(&path)
            .args(["migrate", "status", "--queue"])
            .assert()
            .success();
        if provider == "redis" {
            initial.stdout(predicate::str::contains(
                "Queue (Redis): migrations not applicable",
            ));
        } else {
            initial.stdout(predicate::str::contains("Pending migrations: 1"));
            command(&path)
                .args(["jobs", "list"])
                .assert()
                .failure()
                .stderr(predicate::str::contains("queue schema is not current"));
            command(&path)
                .arg("worker")
                .assert()
                .failure()
                .stderr(predicate::str::contains("queue schema is not current"));
        }
        command(&path)
            .args(["migrate", "--queue"])
            .assert()
            .success();
        let current = command(&path)
            .args(["migrate", "status", "--queue"])
            .assert()
            .success();
        if provider == "redis" {
            current.stdout(predicate::str::contains("not applicable"));
        } else {
            current.stdout(predicate::str::contains("Status: current"));
        }
        command(&path).args(["jobs", "list"]).assert().success();
        command(&path)
            .arg("doctor")
            .assert()
            .success()
            .stdout(predicate::str::contains("Doctor: healthy"));
        command(&path)
            .args(["jobs", "list", "--status", "failed"])
            .assert()
            .success();
        // No jobs were enqueued: retry reaches the portable NotFound outcome, not a schema error.
        command(&path)
            .args(["jobs", "retry", &windwatcher::JobId::new().to_string()])
            .assert()
            .success()
            .stdout(predicate::str::contains("not found"));
        if let Some(mut conn) = pg {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE {name} WITH (FORCE)"
            )))
            .execute(&mut conn)
            .await
            .unwrap();
        }
        if let Some(mut conn) = mysql {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}")))
                .execute(&mut conn)
                .await
                .unwrap();
        }
    }
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
    fs::File::create(directory.path().join("windwatcher.db")).unwrap();

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
    command(&config).arg("migrate").assert().success();
    command(&config)
        .args(["migrate", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage: SQLite"))
        .stdout(predicate::str::contains("Queue: SQLite"));
}

#[test]
fn worker_requires_queue_configuration_and_refuses_pending_schema() {
    let directory = TempDir::new().unwrap();
    let config = config(&directory);
    command(&config)
        .arg("worker")
        .assert()
        .failure()
        .stderr(predicate::str::contains("queue is not configured"));
    let queue_config = format!(
        "\n[queue]\n\n[queue.sqlite]\npath = {:?}\n",
        directory.path().join("queue.db").display().to_string()
    );
    fs::write(
        &config,
        format!("{}{}", fs::read_to_string(&config).unwrap(), queue_config),
    )
    .unwrap();
    assert!(!directory.path().join("queue.db").exists());
    command(&config)
        .arg("worker")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "failed to connect to SQLite queue",
        ));
    assert!(!directory.path().join("queue.db").exists());
    fs::File::create(directory.path().join("queue.db")).unwrap();
    command(&config)
        .arg("worker")
        .assert()
        .failure()
        .stderr(predicate::str::contains("queue schema is not current"))
        .stderr(predicate::str::contains("windwatcher migrate --queue"));
    command(&config)
        .args(["jobs", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("queue schema is not current"));
}

#[tokio::test]
async fn jobs_list_hides_payload_and_retry_only_resets_failed_jobs() {
    use std::time::Duration;
    use time::OffsetDateTime;
    use windwatcher::{
        EnqueueJob, EnqueueOptions, JobFailure, JobFailureCode, JobId, JobKind, JobPayload,
        JobPolicy, LeaseId, QueueStore, SqliteQueue, SqliteQueueOptions, WorkerId,
    };

    let directory = TempDir::new().unwrap();
    let storage_path = directory.path().join("windwatcher.db");
    let queue_path = directory.path().join("queue.db");
    let config = directory.path().join("config.toml");
    fs::write(&config, format!(
        "[storage.sqlite]\nurl = {:?}\n\n[queue]\n\n[queue.sqlite]\npath = {:?}\n\n[logging]\nfilter = \"warn\"\nformat = \"compact\"\n",
        storage_path.display().to_string(), queue_path.display().to_string()
    )).unwrap();

    let failed_id;
    let active_id;
    {
        let queue = SqliteQueue::connect(SqliteQueueOptions::new(format!(
            "sqlite://{}",
            queue_path.display()
        )))
        .await
        .unwrap();
        queue.migrate().await.unwrap();
        let now = OffsetDateTime::now_utc();
        let enqueuer = EnqueueJob::new(&queue, JobPolicy::default());
        let enqueue = |payload| {
            enqueuer.execute(
                JobKind::new("cli.test").unwrap(),
                1,
                payload,
                EnqueueOptions::new(),
                now,
            )
        };
        let failed = enqueue(
            JobPayload::try_from(serde_json::json!({"secret_payload":"must-not-print"})).unwrap(),
        )
        .await
        .unwrap();
        let active = enqueue(JobPayload::try_from(serde_json::json!({})).unwrap())
            .await
            .unwrap();
        failed_id = failed.id();
        active_id = active.id();
        let failed_claim = queue
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_secs(60),
                now,
            )
            .await
            .unwrap()
            .unwrap();
        queue
            .fail(
                failed.id(),
                failed_claim.lease().id(),
                &JobFailure::new(JobFailureCode::new("cli.failed").unwrap(), "safe").unwrap(),
                now,
            )
            .await
            .unwrap();
        queue
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_secs(60),
                now,
            )
            .await
            .unwrap()
            .unwrap();
    }

    command(&config)
        .args(["jobs", "list", "--status", "failed"])
        .assert()
        .success()
        .stdout(predicate::str::contains(failed_id.to_string()))
        .stdout(predicate::str::contains("must-not-print").not());
    command(&config)
        .args(["jobs", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(failed_id.to_string()))
        .stdout(predicate::str::contains(active_id.to_string()));
    command(&config)
        .args(["jobs", "list", "--limit", "101"])
        .assert()
        .failure();
    command(&config)
        .args(["jobs", "retry", &failed_id.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains("retried"));
    command(&config)
        .args(["jobs", "retry", &active_id.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains("not failed"));
    command(&config)
        .args(["jobs", "retry", &JobId::new().to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains("not found"));
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
    fs::File::create(directory.path().join("windwatcher.db")).unwrap();
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
