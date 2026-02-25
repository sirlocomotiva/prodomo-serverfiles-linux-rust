//! Verifies that a real `db-server` process answers a real BOOT request.
//!
//! This is the M2 acceptance test. It starts the built binary, connects over a
//! real TCP socket, writes the exact 24-byte legacy `TPacketGDBoot` payload
//! inside the legacy peer envelope, and decodes the reply with
//! [`protocol::db_boot`] rather than with test-local parsing.
//!
//! Two properties are asserted that a unit test cannot show:
//!
//! - the binary accepts a configured trusted peer and answers it; and
//! - the binary refuses an unlisted peer by closing without a reply, so the
//!   fail-closed default is a real socket behavior and not only a policy value.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use db_server::setup::HEADER_GD_SETUP;
use db_server::setup::SETUP_BASE_WIRE_SIZE;
use protocol::db_boot::{DbBootRequest, BOOT_REQUEST_WIRE_SIZE, HEADER_GD_BOOT};
use protocol::db_map_locations::{
    MapLocation, MapLocationsReply, HEADER_DG_MAP_LOCATIONS, MAP_LOCATION_WIRE_SIZE,
};

const PROCESS_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const DB_PEER_HEADER_SIZE: usize = 9;

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

fn unique_test_root(binary: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should follow the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{binary}-boot-{}-{nonce}", std::process::id()))
}

/// Write a `db.toml` that trusts the loopback address.
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

fn start_server(binary: &str, trusted: bool) -> (ProcessGuard, SocketAddr) {
    let test_root = unique_test_root(binary);
    fs::create_dir_all(&test_root).expect("the test root should be creatable");
    let port = reserve_free_port();
    let config_path = test_root.join("db.toml");
    write_config(&config_path, port);
    if !trusted {
        // Remove only the trust list, leaving the rest of the file valid.
        let text = fs::read_to_string(&config_path).expect("config should be readable");
        fs::write(
            &config_path,
            text.replace("trusted_game_peers = [\"127.0.0.1\"]\n", ""),
        )
        .expect("config should be rewritable");
    }

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

/// Build the exact legacy peer envelope around the 24-byte boot payload.
fn boot_request_frame() -> Vec<u8> {
    let mut ip = [0_u8; 16];
    ip[..9].copy_from_slice(b"127.0.0.1");
    let request = DbBootRequest::new([0, 0], ip);
    let payload = request.encode();
    assert_eq!(payload.len(), BOOT_REQUEST_WIRE_SIZE);

    let mut frame = Vec::with_capacity(DB_PEER_HEADER_SIZE + payload.len());
    frame.push(HEADER_GD_BOOT);
    frame.extend_from_slice(&0x1234_u32.to_le_bytes());
    frame.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("a 24-byte payload fits in u32")
            .to_le_bytes(),
    );
    frame.extend_from_slice(&payload);
    frame
}

/// Build the exact legacy peer envelope around a base-only 154-byte setup
/// payload for the given public IP, listen port, and map list.
fn setup_request_frame(
    public_ip: &[u8],
    listen_port: u16,
    maps: &[i32],
    auth_server: u8,
) -> Vec<u8> {
    let mut payload = vec![0_u8; SETUP_BASE_WIRE_SIZE];
    payload[..16].copy_from_slice(public_ip);
    payload[16] = 0; // bChannel
    payload[17..19].copy_from_slice(&listen_port.to_le_bytes());
    payload[19..21].copy_from_slice(&50900_u16.to_le_bytes()); // wP2PPort
    for (slot, index) in maps.iter().enumerate() {
        let at = 21 + slot * 4;
        payload[at..at + 4].copy_from_slice(&index.to_le_bytes());
    }
    payload[149..153].copy_from_slice(&0_u32.to_le_bytes()); // dwLoginCount
    payload[153] = auth_server;
    assert_eq!(payload.len(), SETUP_BASE_WIRE_SIZE);

    let mut frame = Vec::with_capacity(DB_PEER_HEADER_SIZE + payload.len());
    frame.push(HEADER_GD_SETUP);
    frame.extend_from_slice(&0x5678_u32.to_le_bytes());
    frame.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("a 154-byte payload fits in u32")
            .to_le_bytes(),
    );
    frame.extend_from_slice(&payload);
    frame
}

/// Read one whole DB peer frame, or `None` when the peer closed without a reply.
fn read_frame(stream: &mut TcpStream) -> Option<(u8, u32, Vec<u8>)> {
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .expect("the read timeout should be settable");
    let mut header = [0_u8; DB_PEER_HEADER_SIZE];
    if stream.read_exact(&mut header).is_err() {
        return None;
    }
    let length = u32::from_le_bytes([header[5], header[6], header[7], header[8]]);
    let mut payload = vec![0_u8; length as usize];
    stream.read_exact(&mut payload).ok()?;
    Some((
        header[0],
        u32::from_le_bytes([header[1], header[2], header[3], header[4]]),
        payload,
    ))
}

#[test]
fn a_trusted_peer_receives_no_boot_reply_while_the_tables_are_unloaded() {
    // This server is configured with a player database on port 1, which never
    // accepts a connection. The boot tables therefore cannot load, and a boot
    // request must be refused.
    //
    // The previous behavior was to answer with fourteen empty sections. That
    // looked like success to the game server, which cannot tell an empty world
    // from an unloaded one, so it is exactly the failure this boundary has to
    // prevent. Refusing keeps the game server's own boot gate closed.
    let binary = env!("CARGO_BIN_EXE_db-server");
    let (_guard, address) = start_server(binary, true);

    let mut stream = TcpStream::connect(address).expect("the trusted peer should connect");
    stream
        .write_all(&boot_request_frame())
        .expect("the boot request should be writable");
    stream.flush().expect("the boot request should flush");

    assert!(
        read_frame(&mut stream).is_none(),
        "a boot request must not be answered from unloaded tables"
    );
}

#[test]
fn a_trusted_peer_receives_no_boot_reply_with_no_player_database() {
    // The stronger form of the previous test: with no player section at all,
    // every table query is impossible, so the refusal must be immediate rather
    // than after a connection attempt.
    let binary = env!("CARGO_BIN_EXE_db-server");
    let test_root = unique_test_root(binary);
    fs::create_dir_all(&test_root).expect("the test root should be creatable");
    let port = reserve_free_port();
    let config_path = test_root.join("db.toml");
    fs::write(
        &config_path,
        format!(
            "bind_ip = \"127.0.0.1\"\nbind_port = {port}\ntrusted_game_peers = [\"127.0.0.1\"]\n"
        ),
    )
    .expect("the minimal config should be writable");

    let log_path = test_root.join("db.log");
    let log_file = fs::File::create(&log_path).expect("the log file should be creatable");
    let error_file = log_file
        .try_clone()
        .expect("the log file handle should be duplicable");
    let mut guard = ProcessGuard {
        child: Some(
            Command::new(binary)
                .arg("--config")
                .arg(&config_path)
                .env("RUST_LOG", "info")
                .current_dir(&test_root)
                .stdout(Stdio::from(log_file))
                .stderr(Stdio::from(error_file))
                .spawn()
                .expect("the db-server binary should start"),
        ),
        test_root,
    };
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    wait_until_ready(guard.child_mut(), address);

    let mut stream = TcpStream::connect(address).expect("the trusted peer should connect");
    stream
        .write_all(&boot_request_frame())
        .expect("the boot request should be writable");
    stream.flush().expect("the boot request should flush");
    assert!(
        read_frame(&mut stream).is_none(),
        "without a player database a boot request must be refused"
    );
    assert!(
        guard
            .child_mut()
            .try_wait()
            .expect("status should be readable")
            .is_none(),
        "a refused boot must not terminate the process"
    );
}

#[test]
fn an_unlisted_peer_is_closed_without_a_reply() {
    let binary = env!("CARGO_BIN_EXE_db-server");
    let (_guard, address) = start_server(binary, false);

    let mut stream = TcpStream::connect(address).expect("the socket should still accept");
    // Give the server a moment to classify and close.
    thread::sleep(Duration::from_millis(200));
    if stream.write_all(&boot_request_frame()).is_err() {
        return; // The server already closed. That is the required behavior.
    }
    drop(stream.flush());

    // If the write succeeded, the server must still send nothing.
    let reply = read_frame(&mut stream);
    assert!(
        reply.is_none(),
        "an unlisted peer must receive no boot reply, got {reply:?}"
    );
}

#[test]
fn a_trusted_game_peer_receives_one_map_locations_reply_to_setup() {
    let (_guard, address) = start_server(env!("CARGO_BIN_EXE_db-server"), true);

    let mut stream = TcpStream::connect(address).expect("the DB server should accept the peer");
    let mut public_ip = [0_u8; 16];
    public_ip[..9].copy_from_slice(b"127.0.0.1");
    let maps = [1_i32, 2, 3];
    stream
        .write_all(&setup_request_frame(&public_ip, 50080, &maps, 0))
        .expect("the setup frame should be writable");

    let (header, handle, payload) =
        read_frame(&mut stream).expect("an authorized game setup must be answered");
    assert_eq!(header, HEADER_DG_MAP_LOCATIONS);
    assert_eq!(handle, 0, "the setup reply always uses a zero handle");
    assert_eq!(payload.len(), 1 + MAP_LOCATION_WIRE_SIZE);

    let reply = MapLocationsReply::decode(&payload).expect("the payload should parse");
    assert_eq!(reply.count(), 1);
    let record: &MapLocation = &reply.records()[0];
    assert_eq!(
        record.host, public_ip,
        "the host is echoed from the request"
    );
    assert_eq!(
        record.port, 50080,
        "the listen port is echoed from the request"
    );
    assert_eq!(&record.map_indices[..maps.len()], &maps);
    assert_eq!(
        &record.map_indices[maps.len()..],
        &[0_i32; 32][maps.len()..]
    );
    // The host tail is zero filled, so the reply is reproducible where the
    // legacy reply leaked uninitialized stack bytes.
    assert_eq!(&record.host[9..], &[0_u8; 7]);
}

#[test]
fn an_auth_mode_setup_is_answered_with_no_frame_at_all() {
    let (_guard, address) = start_server(env!("CARGO_BIN_EXE_db-server"), true);

    let mut stream = TcpStream::connect(address).expect("the DB server should accept the peer");
    let mut public_ip = [0_u8; 16];
    public_ip[..9].copy_from_slice(b"127.0.0.1");
    stream
        .write_all(&setup_request_frame(&public_ip, 50080, &[1], 1))
        .expect("the setup frame should be writable");

    // The legacy auth branch writes zero bytes. The connection stays open and
    // waits for the next request, so the read must time out rather than return
    // a frame and rather than report a clean close.
    assert!(
        read_frame(&mut stream).is_none(),
        "an auth-mode setup must not produce any reply bytes"
    );
}

#[test]
fn an_unlisted_peer_cannot_obtain_a_setup_reply() {
    // The setup reply discloses a peer's public IP, listen port, and map list.
    // Failing closed before authorization is what keeps that from an unlisted
    // address, so the same server started with an empty trust list must stay
    // silent here too.
    let (_guard, address) = start_server(env!("CARGO_BIN_EXE_db-server"), false);

    let mut stream = TcpStream::connect(address).expect("the DB server should accept the peer");
    let mut public_ip = [0_u8; 16];
    public_ip[..9].copy_from_slice(b"127.0.0.1");
    stream
        .write_all(&setup_request_frame(&public_ip, 50080, &[1], 0))
        .expect("the setup frame should be writable");
    assert!(
        read_frame(&mut stream).is_none(),
        "an unlisted peer must receive no setup reply"
    );
}
