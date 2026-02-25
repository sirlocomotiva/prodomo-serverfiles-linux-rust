//! M3 acceptance test: the real game-side DB client against a real `db-server`.
//!
//! `boot_process.rs` is the M2 test: it proves the DB binary answers BOOT and
//! SETUP. This test proves the *game half* consumes those real replies, using
//! the production [`GameDbClient`] rather than test-local parsing.
//!
//! The properties that only a real socket can show:
//!
//! - the frames [`GameDbClient`] composes are the frames the DB binary accepts,
//!   in the order the client emits them;
//! - the DB binary's real boot reply passes the client's full validation, which
//!   includes the self-declared length word, the version word, and the width
//!   word of all fourteen table sections;
//! - the phase really reaches `Booted` and the real boot-ready gate opens, which
//!   is the M3 divergence from the legacy server's ungated client listener;
//! - the real `DG_MAP_LOCATIONS` reply is recorded rather than dropped.
//!
//! `game-server` is a dev-dependency only. Neither binary depends on the other;
//! this test is the seam where the two sides meet.
//!
//! # What needs a real database
//!
//! Accepting a boot reply requires a real loaded boot, which requires real
//! rows. That half runs only when `METIN2_TEST_DATABASE_URL` names a `MySQL`
//! server holding the M1 DDL and seed rows. The DB-free half — a DB that cannot
//! load its tables — is its own test and always runs.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use game_server::db_client::{
    public_ip_field, BootReadyGate, DbClientConfig, DbClientEffect, DbConnectionPhase,
    GameDbClient, DB_BOOTSTRAP_HANDLE, DB_RECONNECT_INTERVAL,
};
use protocol::db_wire::{DbFrame, DbFrameDecoder, DB_PEER_HEADER_SIZE};

const PROCESS_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const READ_TIMEOUT: Duration = Duration::from_secs(5);

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

fn unique_test_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should follow the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("db-server-m3-{}-{nonce}", std::process::id()))
}

fn write_config(path: &Path, port: u16) {
    let config = format!(
        "bind_ip = \"127.0.0.1\"\nbind_port = {port}\n\
         trusted_game_peers = [\"127.0.0.1\"]\n\
         [sql_player]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"probe\"\npassword = \"probe\"\ndatabase = \"probe\"\n\
         [sql_account]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"probe\"\npassword = \"probe\"\ndatabase = \"probe\"\n\
         [sql_common]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"probe\"\npassword = \"probe\"\ndatabase = \"probe\"\n"
    );
    fs::write(path, config).expect("temporary TOML config should be writable");
}

fn wait_until_ready(child: &mut Child, address: SocketAddr) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if let Some(status) = child
            .try_wait()
            .expect("db server status should be readable")
        {
            panic!("db server exited before listener readiness: {status}");
        }
        if TcpStream::connect_timeout(&address, POLL_INTERVAL).is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "db server did not become ready at {address}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn start_server(binary: &str) -> (ProcessGuard, SocketAddr) {
    let test_root = unique_test_root();
    fs::create_dir_all(&test_root).expect("the test root should be creatable");
    let port = reserve_free_port();
    let config_path = test_root.join("db.toml");
    write_config(&config_path, port);

    let log_path = test_root.join("db.log");
    let log_file = fs::File::create(&log_path).expect("the log file should be creatable");
    let error_file = log_file
        .try_clone()
        .expect("the log file handle should be duplicable");
    let child = Command::new(binary)
        .arg("--config")
        .arg(&config_path)
        .env("RUST_LOG", "info")
        .current_dir(&test_root)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(error_file))
        .spawn()
        .expect("the db-server binary should start");

    let mut guard = ProcessGuard {
        child: Some(child),
        test_root,
    };
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    wait_until_ready(guard.child_mut(), address);
    (guard, address)
}

/// Start the DB binary against a real `MySQL` server.
fn start_server_for_url(binary: &str, database_url: &str) -> (ProcessGuard, SocketAddr) {
    let test_root = unique_test_root();
    fs::create_dir_all(&test_root).expect("the test root should be creatable");
    let port = reserve_free_port();
    let config_path = test_root.join("db.toml");
    let section = |name: &str| {
        format!(
            "[sql_{name}]\nurl = \"{database_url}\"\nhost = \"127.0.0.1\"\nport = 3306\n\
             user = \"root\"\npassword = \"\"\ndatabase = \"metin2\"\n"
        )
    };
    fs::write(
        &config_path,
        format!(
            "bind_ip = \"127.0.0.1\"\nbind_port = {port}\n\
             trusted_game_peers = [\"127.0.0.1\"]\n{}{}{}",
            section("player"),
            section("account"),
            section("common"),
        ),
    )
    .expect("the temporary DB config should be writable");

    let log_path = test_root.join("db.log");
    let log_file = fs::File::create(&log_path).expect("the log file should be creatable");
    let error_file = log_file
        .try_clone()
        .expect("the log file handle should be duplicable");
    let child = Command::new(binary)
        .arg("--config")
        .arg(&config_path)
        .env("RUST_LOG", "info")
        .current_dir(&test_root)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(error_file))
        .spawn()
        .expect("the db-server binary should start");

    let mut guard = ProcessGuard {
        child: Some(child),
        test_root,
    };
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    wait_until_ready(guard.child_mut(), address);
    (guard, address)
}

fn client_config(listen_port: u16) -> DbClientConfig {
    DbClientConfig {
        listen_port,
        p2p_port: 50900,
        channel: 1,
        public_ip: public_ip_field("127.0.0.1").expect("the loopback text fits szPublicIP"),
        map_allow: vec![1, 2, 3],
        auth_server: false,
        item_id_range: [0, 0],
    }
}

fn write_frames(stream: &mut TcpStream, effects: &[DbClientEffect]) {
    for effect in effects {
        let DbClientEffect::SendFrame(frame) = effect else {
            panic!("the connect path should only send frames, got {effect:?}");
        };

        let bytes = frame.encode().expect("the frame should encode");
        assert_eq!(bytes.len(), DB_PEER_HEADER_SIZE + frame.payload.len());
        stream
            .write_all(&bytes)
            .expect("the frame should reach the db server");
    }
    stream.flush().expect("the socket should flush");
}

/// Read exactly one DB peer frame, or `None` when the peer closed first.
fn read_frame(stream: &mut TcpStream) -> Option<DbFrame> {
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .expect("the read timeout should be settable");
    let mut decoder = DbFrameDecoder::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(frame) = decoder.try_decode().expect("the frame should decode") {
            return Some(frame);
        }
        // A zero-length read is a clean close, which is how a DB that cannot
        // load its tables refuses. A read timeout means the peer is still
        // holding the socket without answering, which is a different failure,
        // so it is reported rather than folded into the same answer. A partial
        // read goes back through the decoder, which is what fragmentation means.
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(partial) => decoder
                .feed(&chunk[..partial])
                .expect("a partial frame should still buffer"),
            Err(error) => panic!("the db server never answered the boot request: {error}"),
        }
    }
}

/// The real client stays gated while the DB cannot load its tables.
///
/// This is the DB-free half. The `db-server` here is configured with an
/// unreachable player database, so it answers the client's real BOOT frame with
/// nothing and closes. The client must stay in `AwaitingBoot`, must not accept
/// clients, and must not record item-ID ranges.
///
/// Before the table loader existed, the same server answered BOOT with fourteen
/// empty sections, which the client accepted and used to open the gate. The
/// client's own validation cannot catch that: fourteen empty sections are a
/// structurally valid version-6 boot. Only the server refusing to encode an
/// unloaded world prevents it, which is why this test exists.
#[test]
fn the_real_game_client_stays_gated_when_the_db_tables_cannot_load() {
    let binary = env!("CARGO_BIN_EXE_db-server");
    let (guard, address) = start_server(binary);
    let _guard = guard;

    let mut client = GameDbClient::new(client_config(50080));
    let gate = BootReadyGate::closed();
    assert_eq!(client.phase(), DbConnectionPhase::Idle);
    assert!(
        !gate.admit(),
        "legacy would already be accepting clients here; the M3 gate does not"
    );

    let effects = client
        .begin_connect(Duration::ZERO, &[])
        .expect("the first attempt should be allowed");
    let mut stream = TcpStream::connect(address).expect("the db server should accept");
    write_frames(&mut stream, &effects);
    client.connected().expect("the socket opened");
    assert_eq!(client.phase(), DbConnectionPhase::AwaitingBoot);

    // The real server closes without a boot frame rather than inventing one.
    assert!(
        read_frame(&mut stream).is_none(),
        "an unloaded DB must not answer BOOT with an empty world"
    );

    assert_eq!(
        client.phase(),
        DbConnectionPhase::AwaitingBoot,
        "a closed socket with no boot reply must not be treated as booted"
    );
    assert!(
        !client.accepts_clients(),
        "the gate stays closed while no boot reply has been accepted"
    );
    assert!(
        !gate.admit(),
        "a caller that never saw a boot reply must stay gated"
    );
    assert!(
        client.boot_item_id_ranges().is_none(),
        "no item-ID range may be recorded from a refused boot"
    );
    assert!(
        client.locations().is_empty(),
        "no map location may be recorded either"
    );
}

/// The real client accepts a real loaded boot reply.
///
/// This needs a real `MySQL` server holding the M1 DDL and seed rows, so it runs
/// only when `METIN2_TEST_DATABASE_URL` names one. Without it the DB-free test
/// above still covers the closed gate; this adds the loaded path.
#[test]
fn the_real_game_client_boots_against_the_real_db_server() {
    let Some(database_url) = real_database_url() else {
        eprintln!(
            "skipping the loaded-boot client test: set METIN2_TEST_DATABASE_URL to a MySQL \
             server holding the M1 DDL and seed rows to run it"
        );
        return;
    };
    let binary = env!("CARGO_BIN_EXE_db-server");
    let (guard, address) = start_server_for_url(binary, &database_url);
    let _guard = guard;

    let mut client = GameDbClient::new(client_config(50080));
    let gate = BootReadyGate::closed();
    assert_eq!(client.phase(), DbConnectionPhase::Idle);
    assert!(
        !gate.admit(),
        "legacy would already be accepting clients here; the M3 gate does not"
    );

    let effects = client
        .begin_connect(Duration::ZERO, &[])
        .expect("the first attempt should be allowed");
    let mut stream = TcpStream::connect(address).expect("the db server should accept");
    write_frames(&mut stream, &effects);
    client.connected().expect("the socket opened");
    assert_eq!(client.phase(), DbConnectionPhase::AwaitingBoot);
    assert!(
        !client.accepts_clients(),
        "the gate stays closed while the boot reply is outstanding"
    );

    // The DB binary answers GD_BOOT with the real version-6 empty snapshot.
    let boot_reply = read_frame(&mut stream).expect("a loaded DB must answer BOOT");
    assert_eq!(boot_reply.handle, DB_BOOTSTRAP_HANDLE);
    let boot_effects = client
        .on_frame(&boot_reply)
        .expect("the real boot reply should be accepted");
    assert!(
        boot_effects
            .iter()
            .any(|effect| matches!(effect, DbClientEffect::BootAccepted { .. })),
        "expected a BootAccepted effect, got {boot_effects:?}"
    );
    assert_eq!(client.phase(), DbConnectionPhase::Booted);
    if client.accepts_clients() {
        gate.open();
    }
    assert!(gate.admit(), "the gate opens on the real boot reply");
    assert!(
        client.boot_item_id_ranges().is_some(),
        "both item-ID ranges came from the real reply"
    );

    // The DB binary then answers GD_SETUP with the real map-location frame.
    let setup_reply = read_frame(&mut stream).expect("a loaded DB must answer SETUP");
    let setup_effects = client
        .on_frame(&setup_reply)
        .expect("the real setup reply should be accepted");
    assert!(
        setup_effects
            .iter()
            .any(|effect| matches!(effect, DbClientEffect::MapLocationsAccepted { .. })),
        "expected a MapLocationsAccepted effect, got {setup_effects:?}"
    );
    assert_eq!(
        client.locations().len(),
        1,
        "a one-peer DB reports one location"
    );
    assert_eq!(
        client.locations()[0].port,
        50080,
        "the listen port the game advertised comes back verbatim"
    );

    // A dropped link closes the gate again. Legacy keeps serving on stale
    // tables after a DB restart; this is the other half of the M3 divergence.
    client.disconnected();
    gate.close();
    assert!(!client.accepts_clients());
    assert!(!gate.admit());
    assert_eq!(client.phase(), DbConnectionPhase::Failed);
    assert!(
        client.connect_allowed(DB_RECONNECT_INTERVAL).is_ok(),
        "a reconnect is allowed once the three-second window has elapsed"
    );
}

/// The `MySQL` URL for the loaded-boot test, if one is configured.
fn real_database_url() -> Option<String> {
    std::env::var("METIN2_TEST_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}
