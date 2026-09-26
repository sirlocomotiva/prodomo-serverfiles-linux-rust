//! Verifies real-process startup and graceful SIGTERM shutdown of `prodomo serve`.
//!
//! Every listener asks for port 0, and the test reads the bound addresses from the log. A port
//! reserved by the test and released before the server binds it could be taken by another
//! process in between, which made the earlier version of this test fail intermittently.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROCESS_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// A store URL that is never connected to: the pool is lazy.
const UNUSED_STORE: &str = "postgres://prodomo:secret-password@127.0.0.1:1/prodomo";

const LISTENING: &str = "Listening for ";

struct ProcessGuard {
    child: Option<Child>,
    test_root: PathBuf,
}

impl ProcessGuard {
    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child process should exist")
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

fn unique_test_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should follow the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("prodomo-process-{}-{nonce}", std::process::id()))
}

fn write_config(path: &Path) {
    let config = format!(
        r#"bind_ip = "127.0.0.1"

[store]
url = "{UNUSED_STORE}"

[auth]
port = 0

[[channel]]
number = 1
ports = [0, 0]
maps = [1, 3]

[[channel]]
number = 99
ports = [0]
maps = [72]
"#
    );
    fs::write(path, config).expect("temporary TOML config should be writable");
}

fn spawn_server(test_root: &Path, config: &Path, log_dir: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_prodomo"))
        .arg("--config")
        .arg(config)
        .arg("serve")
        .arg("--verbose")
        .current_dir(test_root)
        .env("LOG_ANSI", "false")
        .env("LOG_DIR", log_dir)
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("prodomo process should start")
}

/// Forward every stdout line to the test as it is written.
fn read_lines(stdout: ChildStdout) -> (Receiver<String>, JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    (receiver, reader)
}

/// Collect stdout until `Accepting clients`, returning every line seen.
fn wait_until_accepting(child: &mut Child, lines: &Receiver<String>) -> Vec<String> {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    let mut seen = Vec::new();
    loop {
        if let Some(status) = child.try_wait().expect("status should be readable") {
            panic!("prodomo exited before accepting clients: {status}\n{}", seen.join("\n"));
        }
        match lines.recv_timeout(POLL_INTERVAL) {
            Ok(line) => {
                let done = line.contains("Accepting clients");
                seen.push(line);
                if done {
                    return seen;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("stdout closed:\n{}", seen.join("\n")),
        }
        assert!(
            Instant::now() < deadline,
            "prodomo did not start accepting clients:\n{}",
            seen.join("\n")
        );
    }
}

/// Parse `Listening for <role> clients on <addr>` lines into role -> addresses.
fn listeners(lines: &[String]) -> BTreeMap<String, Vec<SocketAddr>> {
    let mut found: BTreeMap<String, Vec<SocketAddr>> = BTreeMap::new();
    for line in lines {
        let Some(start) = line.find(LISTENING) else {
            continue;
        };
        let rest = &line[start + LISTENING.len()..];
        let (role, addr) = rest
            .split_once(" clients on ")
            .unwrap_or_else(|| panic!("unexpected listener line {line:?}"));
        let addr = addr.trim().parse().expect("the logged address should parse");
        found.entry(role.to_owned()).or_default().push(addr);
    }
    found
}

fn wait_until_exited(child: &mut Child, timeout_message: &str) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if child
            .try_wait()
            .expect("status should be readable")
            .is_some()
        {
            return;
        }
        assert!(Instant::now() < deadline, "{timeout_message}");
        thread::sleep(POLL_INTERVAL);
    }
}

#[test]
fn serve_binds_every_listener_and_shuts_down_cleanly_on_sigterm() {
    // Given: a real server process with isolated configuration and logs, every port chosen by the
    // operating system.
    let test_root = unique_test_root();
    let config_path = test_root.join("prodomo.toml");
    let log_dir = test_root.join("log");
    fs::create_dir_all(&log_dir).expect("temporary log directory should be creatable");
    write_config(&config_path);
    let mut process = ProcessGuard {
        child: Some(spawn_server(&test_root, &config_path, &log_dir)),
        test_root: test_root.clone(),
    };
    let stdout = process.child_mut().stdout.take().expect("stdout is piped");
    let (lines, reader) = read_lines(stdout);

    // When: the server reports it is accepting clients.
    let mut console = wait_until_accepting(process.child_mut(), &lines);

    // Then: auth and every Channel port are bound and accept TCP connections.
    let bound = listeners(&console);
    let roles: Vec<(&str, usize)> = bound
        .iter()
        .map(|(role, addrs)| (role.as_str(), addrs.len()))
        .collect();
    assert_eq!(
        roles,
        vec![("auth", 1), ("channel 1", 2), ("channel 99", 1)],
        "listeners in:\n{}",
        console.join("\n")
    );
    let addresses: Vec<SocketAddr> = bound.values().flatten().copied().collect();
    for &address in &addresses {
        assert_ne!(address.port(), 0, "the real port is logged, not 0");
        TcpStream::connect_timeout(&address, PROCESS_TIMEOUT)
            .unwrap_or_else(|error| panic!("{address} should accept: {error}"));
    }

    // When: the operating system sends SIGTERM.
    let child_pid = process
        .child_mut()
        .id()
        .try_into()
        .expect("child process ID should fit in pid_t");
    let child_pid =
        rustix::process::Pid::from_raw(child_pid).expect("child process ID should be non-zero");
    rustix::process::kill_process(child_pid, rustix::process::Signal::TERM)
        .expect("SIGTERM should be delivered");
    wait_until_exited(process.child_mut(), "prodomo did not exit after SIGTERM");

    // Then: shutdown succeeds, lifecycle observables are emitted, and every port closes.
    let mut child = process.child.take().expect("child process should exist");
    let status = child.wait().expect("status should be readable");
    reader.join().expect("the stdout reader should finish");
    console.extend(lines.try_iter());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr is piped")
        .read_to_string(&mut stderr)
        .expect("stderr should be readable");
    let console = format!("{}\n{stderr}", console.join("\n"));
    assert!(status.success(), "prodomo should exit successfully, got {status} with:\n{console}");
    for &address in &addresses {
        assert!(
            TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err(),
            "{address} should be closed after process exit"
        );
    }
    for observable in [
        "Store configured; no connection opened yet",
        "Dedicated game loop started",
        "Accepting clients",
        "Received SIGTERM",
        "Game loop stop acknowledged",
        "Game loop thread joined",
        "Store closed",
        "Server shutdown complete",
    ] {
        assert!(
            console.contains(observable),
            "missing lifecycle observable {observable:?} in:\n{console}"
        );
    }
    assert!(
        !console.contains("secret-password"),
        "the store password must never be logged:\n{console}"
    );
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

/// Run `prodomo --config <path> serve` against `config` and return its exit status and console.
fn run_to_exit(config: Option<&str>) -> (std::process::ExitStatus, String) {
    let test_root = unique_test_root();
    fs::create_dir_all(&test_root).expect("temporary process root should be creatable");
    let config_path = test_root.join("prodomo.toml");
    if let Some(config) = config {
        fs::write(&config_path, config).expect("temporary TOML config should be writable");
    }
    let log_dir = test_root.join("log");
    let mut process = ProcessGuard {
        child: Some(spawn_server(&test_root, &config_path, &log_dir)),
        test_root: test_root.clone(),
    };
    wait_until_exited(process.child_mut(), "prodomo did not reject the config");
    let output = process
        .child
        .take()
        .expect("child process should exist")
        .wait_with_output()
        .expect("output should be readable");
    let console = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    drop(process);
    assert!(!test_root.exists(), "temporary artifacts should be removed");
    (output.status, console)
}

#[test]
fn serve_rejects_a_missing_config() {
    let (status, console) = run_to_exit(None);
    assert!(!status.success(), "a missing config should fail startup");
    assert!(
        console.contains("Failed to load config") && console.contains("failed to read"),
        "missing config error should be observable in:\n{console}"
    );
}

#[test]
fn serve_rejects_an_impossible_topology_before_binding_anything() {
    let config = format!(
        "[store]\nurl = \"{UNUSED_STORE}\"\n[auth]\nport = 0\n\
         [[channel]]\nnumber = 99\nports = [0]\nmaps = [72]\n"
    );
    let (status, console) = run_to_exit(Some(&config));
    assert!(!status.success(), "a config with no login channel should fail startup");
    assert!(
        console.contains("Failed to load config") && console.contains("no channel other than"),
        "topology error should be observable in:\n{console}"
    );
    assert!(!console.contains(LISTENING), "nothing is bound:\n{console}");
}

#[test]
fn serve_rejects_a_non_postgres_store_before_binding_anything() {
    let config = "[store]\nurl = \"mysql://root:hunter2@127.0.0.1/player\"\n[auth]\nport = 0\n\
                  [[channel]]\nnumber = 1\nports = [0]\nmaps = [1]\n";
    let (status, console) = run_to_exit(Some(config));
    assert!(!status.success(), "a MySQL URL should fail startup");
    assert!(
        console.contains("Invalid store configuration"),
        "store error should be observable in:\n{console}"
    );
    assert!(!console.contains("hunter2"), "the password must never be printed:\n{console}");
    assert!(!console.contains(LISTENING), "nothing is bound:\n{console}");
}
