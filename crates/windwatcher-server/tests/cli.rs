use std::{fs, path::Path};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[cfg(unix)]
mod process_isolation {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::{SocketAddr, TcpListener, TcpStream},
        process::{Child, Command as ProcessCommand, Stdio},
        sync::mpsc,
        thread,
        time::Duration,
    };

    struct Process {
        child: Child,
        events: mpsc::Receiver<serde_json::Value>,
        reader: Option<thread::JoinHandle<String>>,
    }

    impl Process {
        fn start(config: &Path, role: &str) -> Self {
            let mut command = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("windwatcher"));
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("WINDWATCHER__") {
                    command.env_remove(key);
                }
            }
            let mut child = command
                .args([
                    "--config",
                    config.to_str().unwrap(),
                    "--log-format",
                    "json",
                    role,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let stderr = child.stderr.take().unwrap();
            let (sender, events) = mpsc::channel();
            let reader = thread::spawn(move || {
                let mut logs = String::new();
                for line in BufReader::new(stderr).lines() {
                    let line = line.unwrap();
                    if let Ok(event) = serde_json::from_str(&line) {
                        let _ = sender.send(event);
                    }
                    logs.push_str(&line);
                    logs.push('\n');
                }
                logs
            });
            Self {
                child,
                events,
                reader: Some(reader),
            }
        }

        fn wait_for(&self, name: &str) -> serde_json::Value {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                let event = self
                    .events
                    .recv_timeout(remaining)
                    .expect("process did not emit expected lifecycle event");
                if event["fields"]["message"] == name {
                    return event;
                }
            }
        }

        fn terminate(mut self) -> (String, String) {
            assert!(
                ProcessCommand::new("kill")
                    .args(["-TERM", &self.child.id().to_string()])
                    .status()
                    .unwrap()
                    .success()
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let status = loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    break status;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "process did not shut down"
                );
                thread::park_timeout(Duration::from_millis(10));
            };
            assert!(status.success());
            let mut stdout = String::new();
            self.child
                .stdout
                .take()
                .unwrap()
                .read_to_string(&mut stdout)
                .unwrap();
            (stdout, self.reader.take().unwrap().join().unwrap())
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    #[test]
    fn serve_ignores_unreachable_queue_and_logs_safe_lifecycle() {
        let directory = TempDir::new().unwrap();
        let path = complete_config(&directory, "info");
        command(&path)
            .args(["migrate", "--storage"])
            .assert()
            .success();
        // Select a local ephemeral address and verify no TCP service is listening.
        let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
        let queue_address = unavailable.local_addr().unwrap();
        drop(unavailable);
        assert!(TcpStream::connect_timeout(&queue_address, Duration::from_secs(1)).is_err());
        let queue_url = format!("redis://:TOP_SECRET@{queue_address}/0");
        let base = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{base}\n[server]\nbind = '127.0.0.1:0'\n[queue]\nprovider = 'redis'\n[queue.redis]\nurl = '{queue_url}'\n")).unwrap();
        let process = Process::start(&path, "serve");
        let listening = process.wait_for("server_listening");
        let address: SocketAddr = listening["fields"]["bind"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_ne!(address.port(), 0);
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            stream,
            "GET /health/ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("\"ready\""));
        let (stdout, logs) = process.terminate();
        let events: Vec<serde_json::Value> = logs
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(events.iter().any(
            |e| e["fields"]["message"] == "storage_ready" && e["fields"]["backend"] == "SQLite"
        ));
        assert!(
            events
                .iter()
                .any(|e| e["fields"]["message"] == "server_listening"
                    && e["fields"]["bind"] == address.to_string())
        );
        assert!(
            events
                .iter()
                .any(|e| e["fields"]["message"] == "shutdown_requested"
                    && e["fields"]["reason"] == "signal")
        );
        assert!(
            events
                .iter()
                .any(|e| e["fields"]["message"] == "shutdown_complete")
        );
        for output in [&stdout, &logs] {
            assert!(!output.contains("TOP_SECRET"));
            assert!(!output.contains(&queue_url));
            assert!(!output.contains("redis://"));
        }
    }

    #[test]
    fn worker_starts_with_valid_queue_and_invalid_primary_storage() {
        let directory = TempDir::new().unwrap();
        let path = complete_config(&directory, "info");
        let base = fs::read_to_string(&path).unwrap();
        let queue = directory.path().join("queue.db");
        fs::write(
            &path,
            format!(
                "{base}\n[queue]\n[queue.sqlite]\npath = {:?}\n",
                queue.display().to_string()
            ),
        )
        .unwrap();
        command(&path)
            .args(["migrate", "--queue"])
            .assert()
            .success();
        let broken = fs::read_to_string(&path).unwrap();
        fs::write(
            &path,
            format!("{broken}\n[storage]\nprovider = 'unusable'\n"),
        )
        .unwrap();
        command(&path).args(["config", "check"]).assert().failure();
        let process = Process::start(&path, "worker");
        process.wait_for("worker_started");
        let (_, logs) = process.terminate();
        assert!(logs.contains("worker_stopping"));
        assert!(!directory.path().join("windwatcher.db").exists());
    }
}

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
fn trusted_proxies_follow_configuration_precedence_and_validate_when_disabled() {
    let directory = TempDir::new().unwrap();
    let path = complete_config(&directory, "warn");
    let base = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        format!("{base}\n[http.rate_limit]\nenabled = false\ntrusted_proxies = ['127.0.0.1/32']\n"),
    )
    .unwrap();
    command(&path).args(["config", "check"]).assert().success();
    let shown = command(&path)
        .args(["config", "show"])
        .env(
            "WINDWATCHER__HTTP__RATE_LIMIT__TRUSTED_PROXIES",
            "[\"::1/128\"]",
        )
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: toml::Value = toml::from_str(std::str::from_utf8(&shown).unwrap()).unwrap();
    assert_eq!(
        parsed["http"]["rate_limit"]["trusted_proxies"]
            .as_array()
            .unwrap()[0]
            .as_str(),
        Some("::1/128")
    );
    command(&path)
        .args(["config", "check"])
        .env(
            "WINDWATCHER__HTTP__RATE_LIMIT__TRUSTED_PROXIES",
            "[\"SENTINEL_PROXY_SECRET\"]",
        )
        .assert()
        .failure()
        .stderr(predicate::str::contains("trusted_proxies"))
        .stderr(predicate::str::contains("SENTINEL_PROXY_SECRET").not());
}

#[test]
fn version_is_independent_of_configuration() {
    let directory = TempDir::new().unwrap();
    command(&directory.path().join("missing.toml"))
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "windwatcher {}",
            env!("CARGO_PKG_VERSION")
        )))
        .stderr(predicate::str::is_empty());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn init_creates_private_configuration_without_persistence_or_overwrite() {
    let directory = TempDir::new().unwrap();
    let target = directory.path().join("new/config.toml");
    let data = directory.path().join("data");
    command(&target)
        .env("XDG_DATA_HOME", &data)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Run `windwatcher migrate` before serving.",
        ));
    let key = target.parent().unwrap().join("jwt.key");
    let secret = fs::read(&key).unwrap();
    assert_eq!(secret.len(), 32);
    assert!(secret.iter().any(|b| *b != 0));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(target.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    command(&target)
        .args(["config", "check"])
        .assert()
        .success();
    assert!(!data.exists());
    let config_before = fs::read(&target).unwrap();
    command(&target)
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
    assert_eq!(fs::read(&target).unwrap(), config_before);
    assert_eq!(fs::read(&key).unwrap(), secret);
    fs::remove_file(&target).unwrap();
    command(&target)
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
    assert!(!target.exists());
    assert_eq!(fs::read(&key).unwrap(), secret);
}

#[cfg(target_os = "linux")]
#[test]
fn system_init_requires_prepared_directory_and_preserves_existing_files() {
    use std::os::unix::fs::PermissionsExt;
    let directory = TempDir::new().unwrap();
    let parent = directory.path().join("prepared");
    let target = parent.join("config.toml");
    let key = parent.join("jwt.key");
    command(&target)
        .args(["--system", "init"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("provisioned first"));
    assert!(!parent.exists());
    fs::create_dir(&parent).unwrap();
    command(&target)
        .args(["--system", "init"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Run `windwatcher --system migrate` as the service account before serving.",
        ));
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let config = fs::read(&target).unwrap();
    let secret = fs::read(&key).unwrap();
    assert_eq!(secret.len(), 32);
    assert!(String::from_utf8_lossy(&config).contains(key.to_str().unwrap()));
    command(&target)
        .args(["--system", "init"])
        .assert()
        .failure();
    assert_eq!(fs::read(&target).unwrap(), config);
    assert_eq!(fs::read(&key).unwrap(), secret);
    fs::remove_file(&target).unwrap();
    command(&target)
        .args(["--system", "init"])
        .assert()
        .failure();
    assert!(!target.exists());
    assert_eq!(fs::read(&key).unwrap(), secret);
    fs::remove_file(&key).unwrap();
    fs::write(&target, &config).unwrap();
    command(&target)
        .args(["--system", "init"])
        .assert()
        .failure();
    assert_eq!(fs::read(&target).unwrap(), config);
    assert!(!key.exists());
}

#[test]
fn config_show_displays_effective_redacted_values_without_connecting_or_reading_secret() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("show.toml");
    fs::write(&path, "[auth.jwt]\nsecret = 'JWT_TOP_SECRET'\n[storage]\nprovider = 'postgres'\n[storage.sqlite]\nurl = 'sqlite://SQLITE_TOP_SECRET'\n[storage.postgres]\nurl = 'postgres://u:PG_TOP_SECRET@127.0.0.1:1/db'\n[storage.mysql]\nurl = 'mysql://u:MYSQL_TOP_SECRET@127.0.0.1:1/db'\n[storage.mongodb]\nurl = 'mongodb://u:MONGO_TOP_SECRET@127.0.0.1:1/'\n[queue]\nprovider = 'redis'\n[queue.postgres]\nurl = 'postgres://u:QUEUE_PG_TOP_SECRET@127.0.0.1:1/db'\n[queue.mysql]\nurl = 'mysql://u:QUEUE_MYSQL_TOP_SECRET@127.0.0.1:1/db'\n[queue.redis]\nurl = 'redis://:REDIS_TOP_SECRET@127.0.0.1:1/'\n").unwrap();
    let output = command(&path)
        .env("WINDWATCHER__SERVER__BIND", "[::1]:1234")
        .args([
            "--log-filter",
            "debug",
            "--log-format",
            "json",
            "config",
            "show",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    for secret in [
        "JWT_TOP_SECRET",
        "SQLITE_TOP_SECRET",
        "PG_TOP_SECRET",
        "MYSQL_TOP_SECRET",
        "MONGO_TOP_SECRET",
        "QUEUE_PG_TOP_SECRET",
        "QUEUE_MYSQL_TOP_SECRET",
        "REDIS_TOP_SECRET",
    ] {
        assert!(!text.contains(secret));
        assert!(!stderr.contains(secret));
    }
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(parsed["server"]["bind"].as_str(), Some("[::1]:1234"));
    assert_eq!(parsed["server"]["readiness_timeout"].as_str(), Some("2s"));
    assert_eq!(parsed["logging"]["filter"].as_str(), Some("debug"));
    assert_eq!(parsed["logging"]["format"].as_str(), Some("json"));
    assert_eq!(parsed["auth"]["jwt"]["secret"].as_str(), Some("<redacted>"));
    for (section, provider) in [
        ("storage", "sqlite"),
        ("storage", "postgres"),
        ("storage", "mysql"),
        ("storage", "mongodb"),
        ("queue", "postgres"),
        ("queue", "mysql"),
        ("queue", "redis"),
    ] {
        assert_eq!(
            parsed[section][provider]["url"].as_str(),
            Some("<redacted>")
        );
    }
    command(&path)
        .env(
            "WINDWATCHER__AUTH__JWT__SECRET_FILE",
            directory.path().join("missing-secret"),
        )
        .args(["config", "show"])
        .assert()
        .success();
    assert!(!directory.path().join("windwatcher.db").exists());
}

#[tokio::test]
async fn user_management_is_idempotent_and_disable_revokes_sessions() {
    use windwatcher::{IdentityStore, RefreshTokenProvider, SessionStore};
    let directory = TempDir::new().unwrap();
    let path = config(&directory);
    let id = windwatcher::UserId::new();
    command(&path)
        .args(["user", "disable", &id.to_string()])
        .assert()
        .failure();
    assert!(!directory.path().join("windwatcher.db").exists());
    fs::File::create(directory.path().join("windwatcher.db")).unwrap();
    command(&path)
        .args(["user", "disable", &id.to_string()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("windwatcher migrate --storage"));
    command(&path).arg("migrate").assert().success();
    command(&path)
        .args(["user", "disable", "not-a-uuid"])
        .assert()
        .failure();
    command(&path)
        .args(["user", "enable", &id.to_string()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!("user {id} was not found")));
    let output = command(&path)
        .args(["user", "create", "--username", "alice", "--password-stdin"])
        .write_stdin("test password")
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    let user_id: windwatcher::UserId = output
        .split('(')
        .nth(1)
        .unwrap()
        .trim()
        .trim_end_matches(')')
        .parse()
        .unwrap();
    let storage = windwatcher_server::Storage::connect(windwatcher::SqliteStorageOptions::new(
        directory
            .path()
            .join("windwatcher.db")
            .display()
            .to_string(),
    ))
    .await
    .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let session = windwatcher::Session::new(
        windwatcher::SessionId::new(),
        user_id,
        now,
        now + time::Duration::hours(1),
    )
    .unwrap();
    let provider = windwatcher::OsRefreshTokenProvider;
    storage
        .create_session(&session, &provider.hash(&provider.generate().unwrap()))
        .await
        .unwrap();
    for _ in 0..2 {
        command(&path)
            .args(["user", "disable", &user_id.to_string()])
            .assert()
            .success();
    }
    assert!(
        storage
            .find_user(user_id)
            .await
            .unwrap()
            .unwrap()
            .is_disabled()
    );
    assert!(
        !storage
            .find_session(session.id())
            .await
            .unwrap()
            .unwrap()
            .is_active(time::OffsetDateTime::now_utc())
    );
    for _ in 0..2 {
        command(&path)
            .args(["user", "enable", &user_id.to_string()])
            .assert()
            .success();
    }
    assert!(
        !storage
            .find_user(user_id)
            .await
            .unwrap()
            .unwrap()
            .is_disabled()
    );
    assert!(
        !storage
            .find_session(session.id())
            .await
            .unwrap()
            .unwrap()
            .is_active(time::OffsetDateTime::now_utc())
    );
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
            format!("mongodb+srv://user:{secret}@does-not-exist.example.invalid/"),
            true,
        ),
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
