//! Verifies real-process startup, store readiness, and SIGTERM shutdown of `prodomo serve`.
//!
//! Every listener asks for port 0, and the test reads the bound addresses from the log. A port
//! reserved by the test and released before the server binds it could be taken by another
//! process in between, which made the earlier version of this test fail intermittently.
//!
//! The tests that need a store run only when `DATABASE_URL` is set.

#![cfg(unix)]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use support::ScratchDatabase;

/// Long enough for a first store attempt to time out (five seconds) with room to spare.
const PROCESS_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// A store URL nothing listens on, so the server keeps retrying it.
const UNREACHABLE_STORE: &str = "postgres://prodomo:secret-password@127.0.0.1:1/prodomo";

const LISTENING: &str = "Listening for ";
const WAITING: &str = "Waiting for the store before accepting clients";
const ACCEPTING: &str = "Accepting clients";
/// `HEADER_GC_PHASE` (`G/packet.h`), the first byte legacy sends an accepted client.
const GC_PHASE: u8 = 0xfd;

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

/// The keys naming the owner's legacy Game data folder and table dumps, read in place.
fn data_keys() -> String {
    let legacy = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy");
    format!(
        "game_data = \"{}\"\ngame_tables = \"{}\"\n",
        legacy.join("gamedata").display(),
        legacy.join("sql/gamedata").display()
    )
}

fn write_config(path: &Path, store_url: &str) {
    let data_keys = data_keys();
    let config = format!(
        r#"bind_ip = "127.0.0.1"
{data_keys}
[store]
url = "{store_url}"

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

/// A running server with its stdout forwarded line by line.
struct Server {
    process: ProcessGuard,
    lines: Receiver<String>,
    reader: JoinHandle<()>,
    console: Vec<String>,
    log_dir: PathBuf,
}

impl Server {
    fn start(store_url: &str) -> Self {
        let test_root = unique_test_root();
        let config_path = test_root.join("prodomo.toml");
        let log_dir = test_root.join("log");
        fs::create_dir_all(&log_dir).expect("temporary log directory should be creatable");
        write_config(&config_path, store_url);
        let mut process = ProcessGuard {
            child: Some(spawn_server(&test_root, &config_path, &log_dir)),
            test_root,
        };
        let stdout = process.child_mut().stdout.take().expect("stdout is piped");
        let (lines, reader) = read_lines(stdout);
        Self {
            process,
            lines,
            reader,
            console: Vec::new(),
            log_dir,
        }
    }

    /// Collect stdout until a line contains `needle`.
    fn wait_for(&mut self, needle: &str) {
        let deadline = Instant::now() + PROCESS_TIMEOUT;
        loop {
            let child = self.process.child_mut();
            if let Some(status) = child.try_wait().expect("status should be readable") {
                self.console.extend(self.lines.try_iter());
                panic!(
                    "prodomo exited before logging {needle:?}: {status}\n{}",
                    self.console.join("\n")
                );
            }
            match self.lines.recv_timeout(POLL_INTERVAL) {
                Ok(line) => {
                    let found = line.contains(needle);
                    self.console.push(line);
                    if found {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("stdout closed:\n{}", self.console.join("\n"))
                }
            }
            assert!(
                Instant::now() < deadline,
                "prodomo did not log {needle:?}:\n{}",
                self.console.join("\n")
            );
        }
    }

    fn logged(&self, needle: &str) -> bool {
        self.console.iter().any(|line| line.contains(needle))
    }

    /// Send SIGTERM, wait for the exit, and return the status and the whole console.
    fn terminate(mut self) -> (ExitStatus, String) {
        let child_pid = self
            .process
            .child_mut()
            .id()
            .try_into()
            .expect("child process ID should fit in pid_t");
        let child_pid =
            rustix::process::Pid::from_raw(child_pid).expect("child process ID should be non-zero");
        rustix::process::kill_process(child_pid, rustix::process::Signal::TERM)
            .expect("SIGTERM should be delivered");
        wait_until_exited(
            self.process.child_mut(),
            "prodomo did not exit after SIGTERM",
        );

        let mut child = self
            .process
            .child
            .take()
            .expect("child process should exist");
        let status = child.wait().expect("status should be readable");
        self.reader.join().expect("the stdout reader should finish");
        self.console.extend(self.lines.try_iter());
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .expect("stderr is piped")
            .read_to_string(&mut stderr)
            .expect("stderr should be readable");
        assert!(
            fs::read_dir(&self.log_dir)
                .expect("temporary log directory should be readable")
                .next()
                .is_some(),
            "server should create a log file"
        );
        let test_root = self.process.test_root.clone();
        drop(self.process);
        assert!(!test_root.exists(), "temporary artifacts should be removed");
        (status, format!("{}\n{stderr}", self.console.join("\n")))
    }
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
        let addr = addr
            .trim()
            .parse()
            .expect("the logged address should parse");
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

/// Every bound address, after checking that auth and every Channel port were bound.
fn bound_addresses(console: &[String]) -> Vec<SocketAddr> {
    let bound = listeners(console);
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
    for address in &addresses {
        assert_ne!(address.port(), 0, "the real port is logged, not 0");
    }
    addresses
}

/// The first byte the server sends a new connection before the client sends anything, or `None`
/// when it closes the connection instead. An admitted client is sent `GC_PHASE` (0xfd) first,
/// because legacy `DESC::Setup` enters the handshake phase before it starts the handshake.
fn first_byte(address: SocketAddr) -> Option<u8> {
    let mut stream = TcpStream::connect_timeout(&address, PROCESS_TIMEOUT)
        .unwrap_or_else(|error| panic!("{address} should accept: {error}"));
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .expect("a read timeout should be settable");
    let mut byte = [0; 1];
    match stream.read(&mut byte) {
        Ok(0) => None,
        Ok(_) => Some(byte[0]),
        Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            panic!("{address} neither greeted nor closed a new client")
        }
        Err(error) if error.kind() == ErrorKind::ConnectionReset => None,
        Err(error) => panic!("{address} failed: {error}"),
    }
}

fn assert_ports_closed(addresses: &[SocketAddr]) {
    for &address in addresses {
        assert!(
            TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err(),
            "{address} should be closed after process exit"
        );
    }
}

fn assert_logged(console: &str, observables: &[&str]) {
    for observable in observables {
        assert!(
            console.contains(observable),
            "missing lifecycle observable {observable:?} in:\n{console}"
        );
    }
}

#[test]
fn serve_refuses_clients_until_the_store_is_reachable_and_shuts_down_cleanly_on_sigterm() {
    // Given: a real server process whose store never answers, every port chosen by the operating
    // system.
    let mut server = Server::start(UNREACHABLE_STORE);

    // When: the server has bound its ports and is waiting for the store.
    server.wait_for(WAITING);

    // Then: auth and every Channel port are bound, and each closes a client at once.
    let addresses = bound_addresses(&server.console);
    for &address in &addresses {
        assert_eq!(
            first_byte(address),
            None,
            "{address} should refuse a client"
        );
    }
    server.wait_for("Refusing client connection before startup has finished");

    // And: an unreachable store is retried rather than fatal.
    server.wait_for("Store unreachable; retrying");
    assert!(!server.logged(ACCEPTING), "the gate stays closed");

    // When: the operating system sends SIGTERM.
    let (status, console) = server.terminate();

    // Then: shutdown succeeds, lifecycle observables are emitted, and every port closes.
    assert!(
        status.success(),
        "prodomo should exit successfully, got {status} with:\n{console}"
    );
    assert_ports_closed(&addresses);
    assert_logged(
        &console,
        &[
            "Store configured; no connection opened yet",
            "Dedicated game loop started",
            WAITING,
            "Received SIGTERM",
            "Game loop stop acknowledged",
            "Game loop thread joined",
            "Store closed",
            "Server shutdown complete",
        ],
    );
    assert!(
        !console.contains(ACCEPTING),
        "the gate never opened:\n{console}"
    );
    assert!(
        !console.contains("secret-password"),
        "the store password must never be logged:\n{console}"
    );
}

#[test]
fn serve_admits_clients_once_the_store_is_migrated() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(database.url());

    server.wait_for(ACCEPTING);
    assert!(server.logged(WAITING), "the gate waited for the store");
    assert!(
        server.logged(&format!(
            "Store ready at schema version {}",
            db::store::schema_version()
        )),
        "the schema was migrated:\n{}",
        server.console.join("\n")
    );
    // The id range is resolved against the migrated table and installed into the world
    // that owns it, and it happens before the gate opens. The three log lines are
    // ordered by this test's reading of the log, which is all the ordering guarantee
    // the console can give: a process has one stdout and the lines arrive in the order
    // it wrote them.
    let console = server.console.join("\n");
    let resolved = console
        .find("Item id range resolved")
        .unwrap_or_else(|| panic!("the id range was not resolved:\n{console}"));
    let accepting = console
        .find(ACCEPTING)
        .unwrap_or_else(|| panic!("the gate never opened:\n{console}"));
    assert!(
        resolved < accepting,
        "the id range must be installed before the gate opens, or a client can reach a \
         world that refuses every grant:\n{console}"
    );

    let addresses = bound_addresses(&server.console);
    for &address in &addresses {
        assert_eq!(
            first_byte(address),
            Some(GC_PHASE),
            "{address} should admit a client"
        );
    }
    server.wait_for("New client connection");

    let (status, console) = server.terminate();
    assert!(
        status.success(),
        "prodomo should exit successfully, got {status} with:\n{console}"
    );
    assert_ports_closed(&addresses);
    assert!(
        !console.contains("Refusing client connection"),
        "no client was refused:\n{console}"
    );
}

#[test]
fn serve_exits_when_the_store_refuses_it_for_good() {
    let Some(admin_url) = support::database_url() else {
        return;
    };
    // The server exists, but the database does not: PostgreSQL answers, and the answer is final.
    let missing = support::with_database(&admin_url, "prodomo_no_such_database");
    let config = format!(
        "{}[store]\nurl = \"{missing}\"\n[auth]\nport = 0\n\
         [[channel]]\nnumber = 1\nports = [0]\nmaps = [1]\n",
        data_keys()
    );
    let (status, console) = run_to_exit(Some(&config));
    assert!(
        !status.success(),
        "a store that refuses the server should stop it"
    );
    assert!(
        console.contains(WAITING) && console.contains("Store unusable"),
        "the refusal should be observable in:\n{console}"
    );
    assert!(
        !console.contains(ACCEPTING),
        "the gate never opened:\n{console}"
    );
    assert!(
        console.contains("Store closed"),
        "the store is closed on the way out:\n{console}"
    );
}

#[test]
fn serve_refuses_to_start_when_the_configured_item_id_span_is_too_narrow() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    // A span of 9_000 ids, against a `MINIMUM_REMAIN_COUNT` of 10_000. Legacy refused
    // this at boot, after its listeners were up; here it must stop the process before
    // the gate opens, because a world with no allocator answers no grant at all.
    let config = format!(
        "{}[store]\nurl = \"{}\"\n[auth]\nport = 0\n\
         [[channel]]\nnumber = 1\nports = [0]\nmaps = [1]\n\
         [game]\nitem_id_range = [1, 9000]\n",
        data_keys(),
        database.url()
    );
    let (status, console) = run_to_exit(Some(&config));
    assert!(!status.success(), "a dead id space should stop the server");
    assert!(
        // The span renders as `first..=last`, which is how the refusal is logged.
        console.contains("Item id range 1..=9000 is unusable")
            && console.contains("which leaves fewer than 10000 ids"),
        "the refusal should name the span:\n{console}"
    );
    assert!(
        !console.contains(ACCEPTING),
        "the gate must not open on a world that cannot give items out:\n{console}"
    );
    assert!(
        console.contains("Store closed"),
        "the store is closed on the way out:\n{console}"
    );
}

/// Run `prodomo --config <path> serve` against `config` and return its exit status and console.
fn run_to_exit(config: Option<&str>) -> (ExitStatus, String) {
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
        "[store]\nurl = \"{UNREACHABLE_STORE}\"\n[auth]\nport = 0\n\
         [[channel]]\nnumber = 99\nports = [0]\nmaps = [72]\n"
    );
    let (status, console) = run_to_exit(Some(&config));
    assert!(
        !status.success(),
        "a config with no login channel should fail startup"
    );
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
    assert!(
        !console.contains("hunter2"),
        "the password must never be printed:\n{console}"
    );
    assert!(!console.contains(LISTENING), "nothing is bound:\n{console}");
}
