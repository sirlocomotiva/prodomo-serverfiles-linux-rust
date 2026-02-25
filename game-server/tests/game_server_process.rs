//! Verifies real-process startup and graceful SIGTERM shutdown.

#![cfg(unix)]

use std::fs;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROCESS_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct ProcessGuard {
    child: Option<Child>,
    test_root: PathBuf,
}

impl ProcessGuard {
    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child process should exist")
    }

    fn wait_with_output(&mut self) -> Output {
        self.child
            .take()
            .expect("child process should exist")
            .wait_with_output()
            .expect("game server output should be readable")
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            drop(child.kill());
            drop(child.wait());
        }
        drop(fs::remove_dir_all(&self.test_root));
    }
}

fn reserve_free_port() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("an ephemeral loopback port should be available");
    listener
        .local_addr()
        .expect("the probe listener should have an address")
        .port()
}

fn unique_test_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should follow the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "game-server-process-{}-{nonce}",
        std::process::id()
    ))
}

fn write_config(path: &Path, port: u16) {
    // TOML, not the legacy `key=value` CONFIG format. `mother_port` is the TOML key; the
    // legacy `port` key is rejected as unknown.
    let config = format!(
        "hostname = \"process-test\"\nchannel = 1\nmother_port = {port}\np2p_port = 50900\n"
    );
    fs::write(path, config).expect("temporary TOML config should be writable");
}

fn wait_until_ready(child: &mut Child, address: SocketAddr) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if let Some(status) = child
            .try_wait()
            .expect("game server status should be readable")
        {
            panic!("game server exited before listener readiness: {status}");
        }

        if TcpStream::connect_timeout(&address, POLL_INTERVAL).is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "game server did not become ready at {address}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_until_exited(child: &mut Child, timeout_message: &str) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if child
            .try_wait()
            .expect("game server status should be readable")
            .is_some()
        {
            return;
        }
        assert!(Instant::now() < deadline, "{timeout_message}");
        thread::sleep(POLL_INTERVAL);
    }
}

#[test]
fn game_server_process_starts_and_shuts_down_cleanly_on_sigterm() {
    // Given: a real server process using isolated configuration, logs, and port.
    let port = reserve_free_port();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let test_root = unique_test_root();
    let config_path = test_root.join("game.toml");
    let log_dir = test_root.join("log");
    fs::create_dir_all(&log_dir).expect("temporary log directory should be creatable");
    write_config(&config_path, port);

    let child = Command::new(env!("CARGO_BIN_EXE_game-server"))
        .arg("--config")
        .arg(&config_path)
        .arg("--verbose")
        .current_dir(&test_root)
        .env("LOG_ANSI", "false")
        .env("LOG_DIR", &log_dir)
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("game server process should start");
    let mut process = ProcessGuard {
        child: Some(child),
        test_root: test_root.clone(),
    };

    // When: readiness is observed and the operating system sends SIGTERM.
    wait_until_ready(process.child_mut(), address);
    let child_pid = process
        .child_mut()
        .id()
        .try_into()
        .expect("child process ID should fit in pid_t");
    let child_pid =
        rustix::process::Pid::from_raw(child_pid).expect("child process ID should be non-zero");
    rustix::process::kill_process(child_pid, rustix::process::Signal::TERM)
        .expect("SIGTERM should be delivered");
    wait_until_exited(
        process.child_mut(),
        "game server did not exit after SIGTERM",
    );
    let output = process.wait_with_output();

    // Then: shutdown succeeds, lifecycle observables are emitted, and resources close.
    assert!(
        output.status.success(),
        "game server should exit successfully"
    );
    assert!(
        TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err(),
        "listener port should be closed after process exit"
    );
    let console = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for observable in [
        format!("Listening for connections on port {port}"),
        "Dedicated game loop started".to_string(),
        "Game loop stop acknowledged".to_string(),
        "Game loop thread joined".to_string(),
        "Server shutdown complete".to_string(),
    ] {
        assert!(
            console.contains(&observable),
            "missing lifecycle observable {observable:?} in:\n{console}"
        );
    }
    assert!(
        fs::read_dir(&log_dir)
            .expect("temporary log directory should be readable")
            .next()
            .is_some(),
        "server should create a log file"
    );

    drop(process);
    assert!(!test_root.exists(), "temporary artifacts should be removed");
}

#[test]
fn game_server_process_rejects_a_missing_config() {
    // Given: an isolated process root whose requested TOML config does not exist.
    let test_root = unique_test_root();
    fs::create_dir_all(&test_root).expect("temporary process root should be creatable");
    let missing_config = test_root.join("missing.toml");
    let child = Command::new(env!("CARGO_BIN_EXE_game-server"))
        .arg("--config")
        .arg(&missing_config)
        .current_dir(&test_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("game server process should start");
    let mut process = ProcessGuard {
        child: Some(child),
        test_root: test_root.clone(),
    };

    // When: startup attempts to load the missing file.
    wait_until_exited(
        process.child_mut(),
        "game server did not reject a missing config",
    );
    let output = process.wait_with_output();

    // Then: startup fails clearly and leaves no temporary process artifacts.
    assert!(
        !output.status.success(),
        "a missing config should fail startup"
    );
    let console = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        console.contains("Failed to parse config"),
        "missing config error should be observable in:\n{console}"
    );

    drop(process);
    assert!(!test_root.exists(), "temporary artifacts should be removed");
}
