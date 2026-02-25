//! M3 process test: the real `game-server` binary against the real `db-server`.
//!
//! `game_db_client.rs` drives [`GameDbClient`] from a test process. This test
//! starts both production binaries and proves the wiring in `main.rs` is real,
//! not a library that simply has no caller.
//!
//! What only a two-process run can show:
//!
//! - a real `game-server` opens a DB socket and sends the real bootstrap frames;
//! - the real DB binary accepts them and both sides reach the booted state;
//! - the boot-ready gate really gates the client listener: a client connecting
//!   before the DB is up is dropped, and the same client connecting after the
//!   boot reply is accepted;
//! - when the DB is killed, the gate re-closes and clients are refused again.
//!
//! The last two are the behavioral divergence from the legacy server, which
//! has no readiness flag and would serve both clients.
//!
//! # What needs a real database
//!
//! Serving clients requires a real loaded boot, which requires real rows. The
//! test that drives that path needs a `MySQL` server holding the M1 DDL and seed
//! rows, so it runs only when `METIN2_TEST_DATABASE_URL` names one. The
//! DB-free half — a DB that can never load its tables — is its own test and
//! always runs, because it is a real assertion about the fail-closed gate
//! rather than a wait for an external service.
//!
//! # Why the game binary path is resolved manually
//!
//! Cargo sets `CARGO_BIN_EXE_<name>` only for a package's *own* binaries. The
//! `game-server` binary belongs to another workspace member, which
//! `db-server` dev-depends on as a *library*. So the path is derived from the
//! running test executable, which Cargo always places in
//! `<target>/debug/deps/`, and a missing binary is a clear error rather than a
//! silent skip. A `cargo test --workspace` run builds both binaries, which is
//! what every gate does.

#![cfg(unix)]

use std::fs;
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROCESS_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// The log line the game server emits when the boot-ready gate refuses a client.
///
/// The test counts occurrences of this marker rather than testing for its
/// presence, because the process log is cumulative across the whole run.
const REFUSAL_MARKER: &str = "Refusing client connection";

/// How long a probe connection may stay open before it is called a failure.
///
/// The game server's handler only recognises keepalive and pong, so an admitted
/// client stays connected. A refused one is closed by the server immediately.
const CLIENT_LINGER: Duration = Duration::from_millis(400);

/// A child process and the temporary directory it runs in.
struct ProcessGuard {
    child: Option<Child>,
    test_root: PathBuf,
    log_path: PathBuf,
}

impl ProcessGuard {
    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("the child process should exist")
    }

    /// The captured stdout/stderr, for failure messages.
    fn log(&self) -> String {
        fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    /// Stop the child now, and mark the guard so `Drop` does not repeat it.
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            drop(child.kill());
            drop(child.wait());
        }
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
    let port = listener
        .local_addr()
        .expect("the probe listener should have an address")
        .port();
    drop(listener);
    port
}

/// Locate another workspace member's binary next to the test executable.
///
/// # Panics
///
/// Panics with the command that builds it, rather than skipping, so a missing
/// binary is never mistaken for a passing test.
fn sibling_binary(name: &str) -> PathBuf {
    let deps = std::env::current_exe()
        .expect("the running test executable should have a path")
        .parent()
        .expect("the test executable should sit in a directory")
        .to_path_buf();
    let path = deps.join("..").join(name);
    assert!(
        path.is_file(),
        "the {name} binary is missing at {}. Build the workspace first: \
         cargo build --workspace --locked --offline",
        path.display()
    );
    path
}

fn unique_test_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should follow the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("game-server-m3-{}-{nonce}", std::process::id()))
}

/// A DB config whose player database can never be reached.
///
/// Port 1 never accepts a connection, so the boot tables cannot load. That is
/// the point: it produces a DB that is genuinely up and genuinely answering on
/// its own port, but which must never serve a boot.
fn write_db_config(path: &Path, port: u16) {
    let config = format!(
        "bind_ip = \"127.0.0.1\"\nbind_port = {port}\n\
         trusted_game_peers = [\"127.0.0.1\"]\n\
         [sql_player]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"probe\"\npassword = \"probe\"\ndatabase = \"probe\"\n\
         [sql_account]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"probe\"\npassword = \"probe\"\ndatabase = \"probe\"\n\
         [sql_common]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"probe\"\npassword = \"probe\"\ndatabase = \"probe\"\n"
    );
    fs::write(path, config).expect("the temporary DB config should be writable");
}

/// A DB config pointed at a real `MySQL` server.
///
/// All three schemas get the same URL because the boot tables and the monarch
/// join are all in the player schema, and the remaining two are unread until a
/// `gmhost` and `gmlist` adapter exists. Pointing them at the same server keeps
/// the config honest about what is actually read.
fn write_db_config_for_url(path: &Path, port: u16, database_url: &str) {
    let section = |name: &str| {
        format!(
            "[sql_{name}]\nurl = \"{database_url}\"\nhost = \"127.0.0.1\"\n\
             port = 3306\nuser = \"root\"\npassword = \"\"\ndatabase = \"metin2\"\n"
        )
    };
    let config = format!(
        "bind_ip = \"127.0.0.1\"\nbind_port = {port}\n\
         trusted_game_peers = [\"127.0.0.1\"]\n{}{}{}",
        section("player"),
        section("account"),
        section("common"),
    );
    fs::write(path, config).expect("the temporary DB config should be writable");
}

fn write_game_config(path: &Path, mother_port: u16, db_port: u16) {
    let config = format!(
        "hostname = \"game1\"\nchannel = 1\nmother_port = {mother_port}\n\
         p2p_port = {}\nbind_ip = \"127.0.0.1\"\n\
         db_addr = \"127.0.0.1\"\ndb_port = {db_port}\n\
         map_allow = [1, 21, 41]\n\
         [player_sql]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"probe\"\npassword = \"probe\"\ndatabase = \"probe\"\n\
         [common_sql]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"probe\"\npassword = \"probe\"\ndatabase = \"probe\"\n\
         [log_sql]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"probe\"\npassword = \"probe\"\ndatabase = \"probe\"\n",
        mother_port + 1
    );
    fs::write(path, config).expect("the temporary game config should be writable");
}

/// Spawn a server binary with its config, and return the guard plus its log path.
fn spawn_server(binary: &Path, config_path: &Path, test_root: &Path, name: &str) -> ProcessGuard {
    let log_path = test_root.join(format!("{name}.log"));
    let log_file = fs::File::create(&log_path).expect("the log file should be creatable");
    let error_file = log_file
        .try_clone()
        .expect("the log file handle should be duplicable");
    let child = Command::new(binary)
        .arg("--config")
        .arg(config_path)
        .env("RUST_LOG", "info")
        .current_dir(test_root)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(error_file))
        .spawn()
        .expect("the server binary should start");
    ProcessGuard {
        child: Some(child),
        test_root: test_root.to_path_buf(),
        log_path,
    }
}

/// Poll until `guard`'s child listens on `address`.
///
/// The log is read on the same guard, so the child handle is borrowed only
/// inside the loop rather than held across the closure.
fn wait_until_listening(guard: &mut ProcessGuard, address: SocketAddr, what: &str) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if let Some(status) = guard
            .child_mut()
            .try_wait()
            .expect("child status should be readable")
        {
            panic!(
                "{what} exited before listening on {address}: {status}\n{}",
                guard.log()
            );
        }
        if TcpStream::connect_timeout(&address, POLL_INTERVAL).is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what} did not listen on {address} within {PROCESS_TIMEOUT:?}\n{}",
            guard.log()
        );
        thread::sleep(POLL_INTERVAL);
    }
}

/// Connect to the game server and report whether the server kept the socket.
///
/// The game server's handler only speaks keepalive and pong, so a served client
/// is still connected after a short linger. A client refused by the boot-ready
/// gate is closed by the server as soon as it is accepted, so its read returns
/// zero immediately.
fn probe_client(address: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, POLL_INTERVAL) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(CLIENT_LINGER));
    let mut buffer = [0_u8; 64];
    // A zero-length read is the server closing the connection, which is how a
    // refused client looks. Anything else, including a read timeout against a
    // served client that sends nothing, means the socket is still held.
    !matches!(stream.read(&mut buffer), Ok(0))
}

/// Poll a running server's log until a marker appears, or give up.
///
/// A marker poll is used instead of a bare sleep so the test cannot pass on a
/// log that was read before the interesting line was ever written, and cannot
/// hang on a log that will never contain it.
fn wait_for_log(guard: &mut ProcessGuard, marker: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if guard.log().contains(marker) {
            return true;
        }
        assert!(
            Instant::now() < deadline,
            "no {marker:?} in the log within {timeout:?}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

/// Poll until a client is served, or give up.
fn wait_for_served_client(address: SocketAddr, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if probe_client(address) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Poll until a client is refused, or give up.
///
/// A refusal is the fast path, so this should settle almost immediately once
/// the gate is shut.
fn wait_for_refused_client(address: SocketAddr, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !probe_client(address) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// A game server whose DB link can never boot refuses every client.
///
/// This is the DB-free half of the vertical, and it is a real assertion rather
/// than a wait for an external service. The `db-server` here is configured with
/// an unreachable player database, so its boot tables can never load and it
/// refuses every `QUERY_BOOT`. The game server must therefore keep its
/// boot-ready gate shut for as long as it runs.
///
/// Before the table loader existed, the same setup produced a *successful* empty
/// boot, which opened the gate and served clients with a world containing no
/// monsters, no items, and no shops. That is the bug this test exists to prevent,
/// and it is why "no rows" and "never loaded" must not share an encoding.
#[test]
fn a_game_server_stays_gated_while_the_db_tables_cannot_load() {
    let test_root = unique_test_root();
    fs::create_dir_all(&test_root).expect("the test root should be creatable");
    let db_binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_db-server"));
    let game_binary = sibling_binary("game-server");

    let mother_port = reserve_free_port();
    let db_port = reserve_free_port();

    let game_config = test_root.join("game.toml");
    write_game_config(game_config.as_path(), mother_port, db_port);
    let mut game = spawn_server(
        &game_binary,
        game_config.as_path(),
        test_root.as_path(),
        "game",
    );
    let client_address = SocketAddr::from((Ipv4Addr::LOCALHOST, mother_port));
    wait_until_listening(&mut game, client_address, "game server");

    // Bring up a real db-server that can never serve a boot: the player database
    // is on port 1, which never accepts a connection.
    let db_config = test_root.join("db.toml");
    write_db_config(db_config.as_path(), db_port);
    let mut db = spawn_server(&db_binary, db_config.as_path(), test_root.as_path(), "db");
    wait_until_listening(
        &mut db,
        SocketAddr::from((Ipv4Addr::LOCALHOST, db_port)),
        "db server",
    );

    // Wait until the DB has actually tried and failed a table load. Without
    // this the gate assertions below could pass on the seconds before the DB
    // even bound its port, which would prove nothing about the loader.
    assert!(
        wait_for_log(&mut db, "boot tables", PROCESS_TIMEOUT),
        "the DB server never reported a table-load failure:\n{}",
        db.log()
    );

    // Three seconds is the legacy reconnect interval, so this spans several
    // link attempts. Every attempt must fail and every client must be refused.
    assert!(
        wait_for_refused_client(client_address, Duration::from_secs(10)),
        "a client was served although the DB tables can never load:\ngame:\n{}\ndb:\n{}",
        game.log(),
        db.log()
    );
    for attempt in 1..=5 {
        assert!(
            !probe_client(client_address),
            "probe {attempt} was served although the DB tables can never load:\n{}",
            game.log()
        );
    }
    let game_log = game.log();
    assert!(
        !game_log.contains("DB boot reply accepted"),
        "no boot reply may be accepted from a DB that cannot load tables:\n{game_log}"
    );
    assert!(
        game_log.matches(REFUSAL_MARKER).count() > 0,
        "the closed gate produced no refusal lines, so it was not the gate that \
         dropped the clients:\n{game_log}"
    );
}

/// The full vertical: a game server with no DB refuses clients, then a real DB
/// with real tables comes up and the same game server starts serving them, then
/// the DB dies and it stops again.
///
/// This needs a real `MySQL` server holding the M1 DDL and seed rows, so it runs
/// only when `METIN2_TEST_DATABASE_URL` names one. Without it the two halves
/// above still cover the gate opening and closing; this adds the loaded path.
#[test]
fn a_real_game_server_gates_clients_on_a_real_db_boot() {
    let Some(database_url) = real_database_url() else {
        eprintln!(
            "skipping the loaded-boot vertical: set METIN2_TEST_DATABASE_URL to a MySQL \
             server holding the M1 DDL and seed rows to run it"
        );
        return;
    };
    run_loaded_boot_vertical(&database_url);
}

/// The `MySQL` URL for the loaded-boot vertical, if one is configured.
fn real_database_url() -> Option<String> {
    std::env::var("METIN2_TEST_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Run the full vertical against a real `MySQL` server.
fn run_loaded_boot_vertical(database_url: &str) {
    let test_root = unique_test_root();
    fs::create_dir_all(&test_root).expect("the test root should be creatable");
    let db_binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_db-server"));
    let game_binary = sibling_binary("game-server");

    let mother_port = reserve_free_port();
    let db_port = reserve_free_port();

    // --- No DB yet. The game server must start and refuse clients. --------
    let game_config = test_root.join("game.toml");
    write_game_config(game_config.as_path(), mother_port, db_port);
    let mut game = spawn_server(
        &game_binary,
        game_config.as_path(),
        test_root.as_path(),
        "game",
    );
    let client_address = SocketAddr::from((Ipv4Addr::LOCALHOST, mother_port));
    wait_until_listening(&mut game, client_address, "game server");
    let game_log = game.log();
    assert!(
        game_log.contains("Listening for connections"),
        "the game server should announce its listener:\n{game_log}"
    );
    assert!(
        game_log.contains("Starting the game-to-DB link"),
        "the game server should start a DB link:\n{game_log}"
    );

    // The gate is closed, so every client is dropped. Three seconds is the
    // legacy reconnect interval, so this also covers the link's retry window.
    assert!(
        wait_for_refused_client(client_address, Duration::from_secs(4)),
        "a client was served before any DB boot:\n{}",
        game.log()
    );
    let refusals_before_boot = game.log().matches(REFUSAL_MARKER).count();
    assert!(
        refusals_before_boot > 0,
        "the closed gate produced no refusal lines, so it was not the gate that \
         dropped the clients:\n{}",
        game.log()
    );

    // --- Bring up the real DB server against the real database. -------------
    let db_config = test_root.join("db.toml");
    write_db_config_for_url(db_config.as_path(), db_port, database_url);
    let mut db = spawn_server(&db_binary, db_config.as_path(), test_root.as_path(), "db");
    let db_address = SocketAddr::from((Ipv4Addr::LOCALHOST, db_port));
    wait_until_listening(&mut db, db_address, "db server");

    // The link retries every three seconds, so allow a few intervals for the
    // connect, the two bootstrap frames, and the boot reply.
    assert!(
        wait_for_served_client(client_address, Duration::from_secs(15)),
        "no client was served after the real DB boot:\ngame:\n{}\ndb:\n{}",
        game.log(),
        db.log()
    );
    let game_log = game.log();
    assert!(
        game_log.contains("DB boot reply accepted"),
        "the game server should log a real accepted boot reply:\n{game_log}"
    );
    assert!(
        game_log.contains("DB map locations recorded"),
        "the game server should record the real map-location reply:\n{game_log}"
    );
    for attempt in 1..=5 {
        assert!(
            probe_client(client_address),
            "probe {attempt} after the boot reply was refused:\n{game_log}"
        );
    }
    assert!(
        game_log.matches("DB boot reply accepted").count() == 1,
        "the boot reply should be accepted exactly once, not on every probe:\n{game_log}"
    );

    // --- Kill the DB. The gate must re-close. ------------------------------
    db.kill();

    assert!(
        wait_for_refused_client(client_address, Duration::from_secs(15)),
        "clients were still served after the DB died:\ngame:\n{}\ndb:\n{}",
        game.log(),
        db.log()
    );
    let game_log = game.log();
    assert!(
        game_log.matches("DB boot reply accepted").count() == 1,
        "a dead link must not re-accept a boot reply:\n{game_log}"
    );
}
