//! Parity scenarios: the scripted client plays the real `prodomo` binary over TCP.
//!
//! Each scenario is a `#[test]` whose name an inventory row in `.scratch/parity/` gives in its
//! `scenario` column; a row is `ported` only when its scenario is here and passes.
//! `inventory_rows_keep_the_rules` checks that link. The scenarios need a store, so they run only
//! when `DATABASE_URL` is set; the inventory check always runs.

#![cfg(unix)]

mod support;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use parity::client::Quiet;
use parity::inventory::{self, Status};
use parity::server::default_channels;
use parity::{Client, Server};
use prodomo::descriptor_crypto::derive_legacy_descriptor_keys;
use protocol::cg_inventory::{resolve_cg_base_size, CG_KEEP_ALIVE, HEADER_CG_PONG};
use protocol::tea::{decrypt_padded, encrypt_padded, TeaKey};
use support::{execute, ScratchDatabase};

/// This file, so the inventory check can find every scenario by name.
const SCENARIOS: &str = include_str!("parity.rs");

/// How long a scenario watches a connection that should stay open.
const QUIET_WINDOW: Duration = Duration::from_millis(300);

fn binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_prodomo"))
}

fn inventory_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../.scratch/parity")
}

// Golden values, spelled out from the legacy source rather than taken from the codecs under test.

/// `HEADER_CG_LOGIN3` (`game/packet.h:77`).
const CG_LOGIN3: u8 = 111;
/// `HEADER_GC_LOGIN_FAILURE` (`game/packet.h:107`).
const GC_LOGIN_FAILURE: u8 = 7;
/// `HEADER_GC_AUTH_SUCCESS` (`game/packet.h:211`).
const GC_AUTH_SUCCESS: u8 = 150;
/// The client's `TPacketCGLogin3`: `BYTE header`, `char login[31]`, `char passwd[17]`,
/// `DWORD adwClientKey[4]`, and a `DWORD` language (the client's `Packet.h`, under
/// `ENABLE_MULTI_LANGUAGE_SYSTEM`).
const CLIENT_LOGIN3_LEN: usize = 69;
/// `TPacketGCLoginFailure`: `BYTE header`, `char szStatus[9]`.
const LOGIN_FAILURE_LEN: usize = 10;
/// `TPacketGCAuthSuccess`: `BYTE header`, `DWORD dwLoginKey`, `BYTE bResult`.
const AUTH_SUCCESS_LEN: usize = 6;

/// `HEADER_GC_PHASE` (`game/packet.h:98`).
const GC_PHASE: u8 = 0xfd;
/// `HEADER_GC_HANDSHAKE` (`game/packet.h:100`) and `HEADER_CG_HANDSHAKE`.
const HANDSHAKE: u8 = 0xff;
/// `HEADER_GC_TIME_SYNC` and `HEADER_CG_TIME_SYNC` (`game/packet.h:10`, `:97`).
const TIME_SYNC: u8 = 0xfc;
/// `HEADER_GC_PING` (`game/packet.h:142`).
const GC_PING: u8 = 44;
/// `HEADER_CG_PONG`.
const CG_PONG: u8 = 0xfe;
/// `EPhase` (`game/packet.h:790`): `PHASE_HANDSHAKE`, `PHASE_LOGIN`, and `PHASE_AUTH`.
const PHASE_HANDSHAKE: u8 = 1;
const PHASE_LOGIN: u8 = 2;
const PHASE_AUTH: u8 = 10;
/// `TPacketGCHandshake` and `TPacketCGHandshake`: header, token, time, and a 32-bit `long`.
const HANDSHAKE_LEN: usize = 13;
/// The key `DESC::Setup` copies into both key arrays (`game/desc.cpp`).
const SETUP_KEY: TeaKey = *b"1234abcd5678efgh";
/// `HANDSHAKE_RETRY_LIMIT` (`game/desc.h`).
const HANDSHAKE_RETRY_LIMIT: usize = 32;
/// `HEADER_CG_STATE_CHECKER` (`game/packet.h:86`).
const CG_STATE_CHECKER: u8 = 206;
/// `HEADER_GC_RESPOND_CHANNELSTATUS` (`game/packet.h:224`).
const GC_RESPOND_CHANNELSTATUS: u8 = 210;

/// A `TPacketGCHandshake` as the client reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Handshake {
    header: u8,
    token: u32,
    time: u32,
    delta: i32,
}

impl Handshake {
    fn parse(bytes: &[u8]) -> Self {
        assert_eq!(bytes.len(), HANDSHAKE_LEN, "handshake width");
        let word = |at: usize| [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        Self {
            header: bytes[0],
            token: u32::from_le_bytes(word(1)),
            time: u32::from_le_bytes(word(5)),
            delta: i32::from_le_bytes(word(9)),
        }
    }

    /// The record in source field order: `bHeader`, `dwHandshake`, `dwTime`, `lDelta`.
    fn bytes(self) -> Vec<u8> {
        let mut bytes = vec![self.header];
        bytes.extend_from_slice(&self.token.to_le_bytes());
        bytes.extend_from_slice(&self.time.to_le_bytes());
        bytes.extend_from_slice(&self.delta.to_le_bytes());
        bytes
    }

    /// An answer that echoes the server's time with no correction, so the bias legacy measures
    /// (`dwCurTime - (dwTime + lDelta)`) is the time the record spent in flight.
    fn answer(self, header: u8) -> Self {
        Self {
            header,
            token: self.token,
            time: self.time,
            delta: 0,
        }
    }
}

/// Encrypt one record as the client does: zero-padded to the TEA block.
fn sealed(record: &[u8]) -> Vec<u8> {
    encrypt_padded(record, &SETUP_KEY)
        .expect("a short record encrypts")
        .into_bytes()
}

/// Read one encrypted record of `len` plaintext bytes and return its plaintext, padding removed
/// after checking it is zero.
fn expect_sealed(client: &mut Client, len: usize) -> Vec<u8> {
    let wire = len.div_ceil(8) * 8;
    let mut plain = decrypt_padded(&client.expect_bytes(wire), &SETUP_KEY).expect("aligned");
    assert!(
        plain[len..].iter().all(|&byte| byte == 0),
        "padding {plain:02x?}"
    );
    plain.truncate(len);
    plain
}

/// Connect and read what `DESC::Setup` sends: the handshake phase, then the first handshake.
fn accepted(address: SocketAddr) -> (Client, Handshake) {
    let mut client = Client::connect(address);
    assert_eq!(client.expect_bytes(2), [GC_PHASE, PHASE_HANDSHAKE]);
    let first = Handshake::parse(&client.expect_bytes(HANDSHAKE_LEN));
    assert_eq!(first.header, HANDSHAKE);
    assert_eq!(first.delta, 0, "StartHandshake sends a zero delta");
    assert_ne!(first.token, 0, "CreateHandshake never returns zero");
    (client, first)
}

/// Finish the handshake and return the phase record, answering timing retries as a client does.
fn shake(client: &mut Client, first: Handshake) -> [u8; 2] {
    let mut offer = first;
    for _ in 0..=HANDSHAKE_RETRY_LIMIT {
        client.send(&offer.answer(HANDSHAKE).bytes());
        let header = client.expect_bytes(1)[0];
        if header == GC_PHASE {
            return [header, client.expect_bytes(1)[0]];
        }
        assert_eq!(header, HANDSHAKE, "a retry is a new handshake");
        let mut record = vec![header];
        record.extend(client.expect_bytes(HANDSHAKE_LEN - 1));
        offer = Handshake::parse(&record);
        assert_eq!(offer.token, first.token);
    }
    panic!("the handshake never completed");
}

/// The lowest header byte the legacy client table does not register.
fn unregistered_header() -> u8 {
    // Controls: the keepalive byte and PONG are registered, so the search is not vacuous.
    assert_eq!(resolve_cg_base_size(CG_KEEP_ALIVE.value()), Some(1));
    assert!(resolve_cg_base_size(HEADER_CG_PONG.value()).is_some());
    (0..=u8::MAX)
        .find(|&header| resolve_cg_base_size(header).is_none())
        .expect("some header byte is unregistered")
}

#[test]
fn inventory_rows_keep_the_rules() {
    let rows = inventory::load(&inventory_dir()).unwrap_or_else(|problems| {
        panic!(
            "the inventory does not parse:\n{}",
            problems
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let mut problems: Vec<String> = inventory::check(&rows)
        .iter()
        .map(ToString::to_string)
        .collect();
    for row in rows.iter().filter(|row| row.status == Status::Ported) {
        let signature = format!("fn {}()", row.scenario);
        if !SCENARIOS.contains(&signature) {
            problems.push(format!(
                "{}: `{}` names `{}`, which is not a scenario in prodomo/tests/parity.rs",
                row.file, row.id, row.scenario
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    // Controls: the generated tables are all there, so an empty directory cannot pass.
    for file in [
        "client-packets.md",
        "server-records.md",
        "systems.md",
        "gamedata.md",
    ] {
        assert!(
            rows.iter().any(|row| row.file == file),
            "{file} has no rows"
        );
    }
    let counts = inventory::counts(&rows);
    assert!(counts.get(&Status::Missing).copied().unwrap_or(0) > 1000);
    eprintln!("inventory: {} rows, {counts:?}", rows.len());
}

/// `cg.any.keep_alive`: a zero byte is one whole frame in every phase, with no answer.
#[test]
fn keep_alive_bytes_are_single_byte_frames() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    for address in [server.auth(), server.channel(1), server.channel(99)] {
        let mut client = Client::connect(address);
        client.send(&[CG_KEEP_ALIVE.value(); 5]);
        let (_, quiet) = client.drain(QUIET_WINDOW);
        assert_eq!(quiet, Quiet::Open, "{address} closed after keepalives");

        // The next byte is read as a header, so each zero byte was consumed alone: were a
        // keepalive wider, this byte would be swallowed as its payload and nothing would close.
        client.send(&[unregistered_header()]);
        let _ = client.expect_closed();
    }
    server.wait_for("Received client keepalive");
}

/// `cg.any.unknown_header`: a header the client table does not register closes the descriptor.
#[test]
fn an_unregistered_header_closes_the_connection() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    for address in [server.auth(), server.channel(1), server.channel(99)] {
        let mut client = Client::connect(address);
        client.send(&[unregistered_header()]);
        let _ = client.expect_closed();

        // Control: a connection that sends nothing stays open.
        let mut idle = Client::connect(address);
        let (_, quiet) = idle.drain(QUIET_WINDOW);
        assert_eq!(quiet, Quiet::Open, "{address} closed an idle client");
    }
}

/// `cg.handshake.handshake`, `cg.handshake.pong`, `gc.phase`, `gc.handshake`: on accept the server sends
/// `GC_PHASE(PHASE_HANDSHAKE)` and a zero-delta handshake in plaintext; the echoed token moves auth
/// to `PHASE_AUTH` and a Channel to `PHASE_LOGIN`, and the phase record itself is still plaintext.
#[test]
fn the_handshake_is_sent_on_accept_and_selects_the_phase() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    for (address, phase) in [
        (server.auth(), PHASE_AUTH),
        (server.channel(1), PHASE_LOGIN),
        (server.channel(99), PHASE_LOGIN),
    ] {
        let (mut client, first) = accepted(address);
        // `cg.handshake.pong`: a plaintext PONG in the handshake phase is read and not answered.
        client.send(&[CG_PONG]);
        assert_eq!(shake(&mut client, first), [GC_PHASE, phase], "{address}");
        let (_, quiet) = client.drain(QUIET_WINDOW);
        assert_eq!(quiet, Quiet::Open, "{address} closed after the handshake");

        // Controls: each descriptor has its own token, and a wrong one closes without a record.
        let (mut other, second) = accepted(address);
        assert_ne!(second.token, first.token);
        let mut wrong = second.answer(HANDSHAKE);
        wrong.token ^= 0x0101_0101;
        other.send(&wrong.bytes());
        assert_eq!(other.expect_closed(), Vec::<u8>::new(), "{address}");
    }
}

/// `sys.net.handshake` timing: a reply outside the 50 ms bias gets a new handshake whose delta is
/// half the measured gap; a negative delta is ignored; the 33rd rejected reply closes.
#[test]
fn a_mistimed_handshake_is_retried_until_the_limit() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    let (mut client, first) = accepted(server.channel(1));

    // `lDelta < 0` is refused before any retry is counted: no answer, still open.
    let mut negative = first.answer(HANDSHAKE);
    negative.delta = -1;
    client.send(&negative.bytes());
    let (sent, quiet) = client.drain(QUIET_WINDOW);
    assert_eq!((sent, quiet), (Vec::new(), Quiet::Open));

    let gap: u32 = 10_000;
    for retry in 1..=HANDSHAKE_RETRY_LIMIT {
        let mut late = first.answer(HANDSHAKE);
        late.time = first.time.wrapping_sub(gap);
        client.send(&late.bytes());
        let again = Handshake::parse(&client.expect_bytes(HANDSHAKE_LEN));
        assert_eq!(
            (again.header, again.token),
            (HANDSHAKE, first.token),
            "retry {retry}"
        );
        // `(dwCurTime - dwTime) / 2`, where the server clock has moved on by the time elapsed.
        let expected = i32::try_from(again.time.wrapping_sub(late.time) / 2).expect("small");
        assert_eq!(again.delta, expected, "retry {retry}");
        assert!(again.delta >= i32::try_from(gap / 2).expect("small"));
    }
    let mut late = first.answer(HANDSHAKE);
    late.time = first.time.wrapping_sub(gap);
    client.send(&late.bytes());
    assert_eq!(client.expect_closed(), Vec::<u8>::new());
}

/// `cg.auth.pong`, `cg.auth.handshake`: after the phase record the input is TEA under the setup
/// key. A sealed PONG is read as a PONG and a sealed handshake is consumed without an answer.
#[test]
fn the_auth_phase_reads_tea_input() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    let (mut client, first) = accepted(server.auth());
    assert_eq!(shake(&mut client, first), [GC_PHASE, PHASE_AUTH]);

    let pong = sealed(&[CG_PONG]);
    // Control: the sealed bytes do not begin with a PONG, so only a decrypting reader sees one.
    assert_ne!(pong[0], CG_PONG);
    client.send(&pong);
    client.send(&sealed(&first.answer(HANDSHAKE).bytes()));
    let (sent, quiet) = client.drain(QUIET_WINDOW);
    assert_eq!((sent, quiet), (Vec::new(), Quiet::Open));
    server.wait_for("Received client pong");

    // Control: a plaintext unregistered header is not what the reader sees any more; sealing it
    // is what closes the connection.
    client.send(&sealed(&[unregistered_header()]));
    let _ = client.expect_closed();
}

/// `cg.login.time_sync`, `cg.login.pong`, `gc.handshake_ok`: in the login phase a sealed time sync
/// inside the bias is acknowledged with a sealed `GC_TIME_SYNC`; outside it, a sealed handshake
/// with a new delta is sent, without a retry limit.
#[test]
fn the_login_phase_answers_time_sync() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    let (mut client, first) = accepted(server.channel(1));
    assert_eq!(shake(&mut client, first), [GC_PHASE, PHASE_LOGIN]);

    client.send(&sealed(&[CG_PONG]));
    server.wait_for("Received client pong");

    let gap: u32 = 10_000;
    for _ in 0..=HANDSHAKE_RETRY_LIMIT {
        let mut late = first.answer(TIME_SYNC);
        late.time = late.time.wrapping_sub(gap);
        client.send(&sealed(&late.bytes()));
        let again = Handshake::parse(&expect_sealed(&mut client, HANDSHAKE_LEN));
        assert_eq!((again.header, again.token), (HANDSHAKE, first.token));
        assert!(again.delta >= i32::try_from(gap / 2).expect("small"));
    }

    // In time: echo the latest server time. A slow machine may still miss the 50 ms window;
    // then the server offers a fresh time and the client echoes that one.
    let mut offer = first;
    let mut acknowledged = false;
    for _ in 0..=HANDSHAKE_RETRY_LIMIT {
        client.send(&sealed(&offer.answer(TIME_SYNC).bytes()));
        let mut reply = decrypt_padded(&client.expect_bytes(8), &SETUP_KEY).expect("aligned");
        if reply[0] == TIME_SYNC {
            assert_eq!(reply, [TIME_SYNC, 0, 0, 0, 0, 0, 0, 0]);
            acknowledged = true;
            break;
        }
        assert_eq!(
            reply[0], HANDSHAKE,
            "only a retry may come instead of the ack"
        );
        reply.extend(decrypt_padded(&client.expect_bytes(8), &SETUP_KEY).expect("aligned"));
        offer = Handshake::parse(&reply[..HANDSHAKE_LEN]);
        assert_ne!(offer.time, first.time);
    }
    assert!(acknowledged, "the time sync was never acknowledged");
    let (sent, quiet) = client.drain(QUIET_WINDOW);
    assert_eq!((sent, quiet), (Vec::new(), Quiet::Open));
}

/// `sys.net.heartbeat`, `gc.ping`: every `ping_event_second_cycle` the server sends a sealed ping
/// and a zero-delta handshake; a client that answered stays, one that did not is closed at the
/// next cycle.
#[test]
fn the_ping_cycle_closes_a_silent_client() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "ping_event_second_cycle = 1",
    );
    let (mut client, first) = accepted(server.channel(1));
    assert_eq!(shake(&mut client, first), [GC_PHASE, PHASE_LOGIN]);

    let mut times = Vec::new();
    for cycle in 1..=2 {
        assert_eq!(expect_sealed(&mut client, 1), [GC_PING], "cycle {cycle}");
        let handshake = Handshake::parse(&expect_sealed(&mut client, HANDSHAKE_LEN));
        assert_eq!(
            (handshake.header, handshake.token, handshake.delta),
            (HANDSHAKE, first.token, 0),
            "cycle {cycle}"
        );
        times.push(handshake.time);
        if cycle == 1 {
            client.send(&sealed(&[CG_PONG]));
        }
    }
    // `get_dword_time` counts milliseconds, so two handshakes one cycle apart are ~1000 apart.
    let apart = times[1].wrapping_sub(times[0]);
    assert!((900..=1_500).contains(&apart), "{apart} between two cycles");

    // No pong after the second ping: the third cycle closes without writing.
    assert_eq!(client.expect_closed(), Vec::<u8>::new());
}

/// The `RespondChannelStatus` record (`game/input_db.cpp:2412-2429`) for `ports`, each with
/// `status`: the header, an `int` count, a packed `{ short nPort; BYTE bStatus; }` per port, and
/// `bSuccess = 1`.
fn channel_status(ports: &[u16], status: u8) -> Vec<u8> {
    let mut record = vec![GC_RESPOND_CHANNELSTATUS];
    record.extend_from_slice(&i32::try_from(ports.len()).expect("few").to_le_bytes());
    for port in ports {
        record.extend_from_slice(&port.to_le_bytes());
        record.push(status);
    }
    record.push(1);
    record
}

/// Every Channel port the server bound, in ascending order.
fn channel_ports(server: &Server) -> Vec<u16> {
    let mut ports: Vec<u16> = server
        .listeners()
        .iter()
        .filter(|(role, _)| role.starts_with("channel "))
        .flat_map(|(_, addresses)| addresses.iter().map(SocketAddr::port))
        .collect();
    ports.sort_unstable();
    ports
}

/// `cg.handshake.state_checker`, `sys.net.channel_status`: in the handshake phase, on the auth
/// port and every Channel port, `STATE_CHECKER` is answered in plaintext with every Channel port
/// and status 1 (`NORMAL`, nobody in game), as often as it is asked, and the handshake still
/// completes afterwards. `shutdowned` turns every status to 0.
#[test]
fn the_channel_status_list_is_served_in_the_handshake_phase() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    let ports = channel_ports(&server);
    // Channel 1 binds two listeners and the Shared Channel one; auth reports nothing.
    assert_eq!(ports.len(), 3, "{ports:?}");
    let expected = channel_status(&ports, 1);
    for (address, phase) in [
        (server.auth(), PHASE_AUTH),
        (server.channel(1), PHASE_LOGIN),
        (server.channel(99), PHASE_LOGIN),
    ] {
        let (mut client, first) = accepted(address);
        for _ in 0..2 {
            client.send(&[CG_STATE_CHECKER]);
            assert_eq!(client.expect_bytes(expected.len()), expected, "{address}");
        }
        assert_eq!(shake(&mut client, first), [GC_PHASE, phase], "{address}");

        // Control: only the handshake analyzer handles it. After the phase change the sealed
        // header reaches the auth or login analyzer, which does not, and the connection closes.
        client.send(&sealed(&[CG_STATE_CHECKER]));
        assert_eq!(client.expect_closed(), Vec::<u8>::new(), "{address}");
    }
    drop(server);

    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "shutdowned = true",
    );
    let ports = channel_ports(&server);
    let (mut client, _) = accepted(server.auth());
    client.send(&[CG_STATE_CHECKER]);
    let closed = channel_status(&ports, 0);
    assert_eq!(client.expect_bytes(closed.len()), closed);
}

/// Run one statement in the scenario's database.
fn sql(database: &ScratchDatabase, statement: &str) {
    execute(database.url(), statement).expect("the statement should succeed");
}

/// The password every auth scenario account has: the full 16 bytes legacy keeps.
const ACCOUNT_PASSWORD: &[u8; 16] = b"0123456789abcdef";

/// Create `login` with [`ACCOUNT_PASSWORD`] through the Operator command.
fn create_account(server: &Server, login: &str) {
    let password = std::str::from_utf8(ACCOUNT_PASSWORD).expect("ASCII");
    let output = server.operate(&["account", "create", login], &format!("{password}\n"));
    assert!(
        output.status.success(),
        "account create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The 69 bytes the client sends: the fields laid out by hand, the language as a little-endian
/// `DWORD`. `login` and `password` are copied into their fields as given, so a scenario can fill a
/// field to the last byte.
fn client_login3(login: &[u8], password: &[u8], language: u32) -> Vec<u8> {
    let mut record = vec![CG_LOGIN3];
    let mut field = [0u8; 31];
    field[..login.len()].copy_from_slice(login);
    record.extend_from_slice(&field);
    let mut field = [0u8; 17];
    field[..password.len()].copy_from_slice(password);
    record.extend_from_slice(&field);
    for word in [0x1122_3344u32, 0x5566_7788, 0x99aa_bbcc, 0xddee_ff01] {
        record.extend_from_slice(&word.to_le_bytes());
    }
    record.extend_from_slice(&language.to_le_bytes());
    assert_eq!(record.len(), CLIENT_LOGIN3_LEN);
    record
}

/// A connection in the auth phase.
fn auth_client(server: &Server) -> Client {
    let (mut client, first) = accepted(server.auth());
    assert_eq!(shake(&mut client, first), [GC_PHASE, PHASE_AUTH]);
    client
}

/// What the server answers a `LOGIN3`: `Ok(key)` for `AUTH_SUCCESS`, or the failure status.
///
/// TEA works on independent 8-byte units, so the first unit names the record and says how many
/// more to read.
fn log_in(
    client: &mut Client,
    login: &[u8],
    password: &[u8],
    language: u32,
) -> Result<u32, String> {
    client.send(&sealed(&client_login3(login, password, language)));
    let mut wire = client.expect_bytes(8);
    let first = decrypt_padded(&wire, &SETUP_KEY).expect("aligned");
    let len = match first.first().copied().expect("a whole unit") {
        GC_AUTH_SUCCESS => AUTH_SUCCESS_LEN,
        GC_LOGIN_FAILURE => LOGIN_FAILURE_LEN,
        other => panic!("unexpected answer header {other}"),
    };
    wire.extend(client.expect_bytes(len.div_ceil(8) * 8 - 8));
    let mut record = decrypt_padded(&wire, &SETUP_KEY).expect("aligned");
    assert!(record[len..].iter().all(|&byte| byte == 0), "{record:02x?}");
    record.truncate(len);
    if record[0] == GC_AUTH_SUCCESS {
        assert_eq!(record[5], 1, "bResult");
        return Ok(u32::from_le_bytes([
            record[1], record[2], record[3], record[4],
        ]));
    }
    let status = &record[1..];
    let end = status.iter().position(|&byte| byte == 0).expect("a NUL");
    assert!(status[end..].iter().all(|&byte| byte == 0), "{status:02x?}");
    Err(String::from_utf8(status[..end].to_vec()).expect("ASCII"))
}

/// `cg.auth.login3`, `sys.auth.login`, `gc.auth_success`, `gc.login_failure`: the client's 69-byte
/// `LOGIN3` is answered with a sealed `AUTH_SUCCESS` carrying a login key, or a sealed
/// `LOGIN_FAILURE` in the legacy check order, and a refusal leaves the connection open. The three
/// surplus bytes of the client's `DWORD` language are skipped as zero headers.
#[test]
fn the_auth_login_grants_a_key_or_names_the_first_failed_check() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    // A block date the scenario can reach, so the configured value is what is compared.
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "block_login = \"29991231\"",
    );
    create_account(&server, "alice");
    let pw = &ACCOUNT_PASSWORD[..];

    let mut client = auth_client(&server);
    // `CInputAuth::Login`: the login shape first, before anything is looked up.
    assert_eq!(log_in(&mut client, b"a", pw, 1), Err("NOID".into()));
    assert_eq!(log_in(&mut client, b"al_ice", pw, 1), Err("NOID".into()));
    assert_eq!(log_in(&mut client, b"bob", pw, 1), Err("NOID".into()));
    // The `QID_AUTH_LOGIN` result: password, then the language, then success.
    assert_eq!(
        log_in(&mut client, b"alice", b"wrong", 0),
        Err("WRONGPWD".into())
    );
    assert_eq!(
        log_in(&mut client, b"alice", &pw[..15], 1),
        Err("WRONGPWD".into())
    );
    assert_eq!(log_in(&mut client, b"alice", pw, 0), Err("NOLANG".into()));
    assert_eq!(log_in(&mut client, b"alice", pw, 12), Err("INVLANG".into()));
    // `trim_and_lower` and `strlcpy(passwd, ..., 17)`: case and surrounding spaces are dropped,
    // and a 17th password byte is never read.
    let mut full = pw.to_vec();
    full.push(b'X');
    let key = log_in(&mut client, b"  ALICE ", &full, 5).expect("alice logs in");
    assert!((1..=0x7fff_ffff).contains(&key), "{key}");
    sql(
        &database,
        "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM account WHERE login = 'alice' \
         AND language = 5 AND last_play_at IS NOT NULL) THEN RAISE 'not recorded'; END IF; END $$",
    );

    // `FindByLoginName`: the descriptor now holds the login, so it and every other auth
    // descriptor are refused, before the password is checked.
    assert_eq!(log_in(&mut client, b"alice", pw, 1), Err("ALREADY".into()));
    let mut other = auth_client(&server);
    assert_eq!(
        log_in(&mut other, b"alice", b"wrong", 1),
        Err("ALREADY".into())
    );
    // Control: the refusals left both connections open.
    assert_eq!(other.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));
    drop(client);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let again = loop {
        match log_in(&mut other, b"alice", pw, 1) {
            Err(status) if status == "ALREADY" && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            answer => break answer,
        }
    };
    let again = again.expect("the login is free once its descriptor closes");
    assert_ne!(again, key);
    drop(other);

    // The account checks, in order: availability after the password, then the status, then the
    // block date, which refuses an account created on it or later.
    let mut client = auth_client(&server);
    sql(
        &database,
        "UPDATE account SET status = 'BLOCK', available_at = now() + interval '1 day', \
         created_at = '2999-12-31' WHERE login = 'alice'",
    );
    assert_eq!(
        log_in(&mut client, b"alice", b"wrong", 1),
        Err("WRONGPWD".into())
    );
    assert_eq!(log_in(&mut client, b"alice", pw, 1), Err("NOTAVAIL".into()));
    sql(
        &database,
        "UPDATE account SET available_at = now() WHERE login = 'alice'",
    );
    assert_eq!(log_in(&mut client, b"alice", pw, 0), Err("BLOCK".into()));
    sql(
        &database,
        "UPDATE account SET status = 'OK' WHERE login = 'alice'",
    );
    assert_eq!(log_in(&mut client, b"alice", pw, 0), Err("NOLANG".into()));
    assert_eq!(log_in(&mut client, b"alice", pw, 1), Err("BLKLOGIN".into()));
    sql(
        &database,
        "UPDATE account SET created_at = '2999-12-30' WHERE login = 'alice'",
    );
    assert!(log_in(&mut client, b"alice", pw, 1).is_ok());
    drop(client);
    drop(server);

    // `g_bNoMoreClient`: after the login shape, before the lookup.
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "shutdowned = true",
    );
    let mut client = auth_client(&server);
    assert_eq!(log_in(&mut client, b"a", pw, 1), Err("NOID".into()));
    assert_eq!(log_in(&mut client, b"alice", pw, 1), Err("SHUTDOWN".into()));
}

/// `cg.auth.login3`: a `LOGIN3` whose frame is not the server's 66 bytes cannot be told apart
/// from the next frame, so only the exact record is read. A `LOGIN3` in the handshake phase is
/// not handled there and closes the connection.
#[test]
fn a_login3_outside_the_auth_phase_closes_the_connection() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    let (mut client, _) = accepted(server.auth());
    client.send(&client_login3(b"alice", ACCOUNT_PASSWORD, 1));
    assert_eq!(client.expect_closed(), Vec::<u8>::new());
}

/// `HEADER_CG_LOGIN2` (`game/packet.h:75`).
const CG_LOGIN2: u8 = 109;
/// `TPacketCGLogin2`: `BYTE header`, `char login[31]`, `DWORD dwLoginKey`, `DWORD adwClientKey[4]`
/// (`game/packet.h:472`).
const LOGIN2_LEN: usize = 52;
/// `HEADER_GC_EMPIRE` (`game/packet.h:176`) and its `BYTE bEmpire`.
const GC_EMPIRE: u8 = 90;
const EMPIRE_LEN: usize = 2;
/// `HEADER_GC_LOGIN_SUCCESS_NEWSLOT` (`game/packet.h:132`): four 70-byte `TSimplePlayer`, four
/// `DWORD` guild IDs, four `char[13]` guild names, `DWORD handle`, `DWORD random_key`
/// (`game/packet.h:830`).
const GC_LOGIN_SUCCESS: u8 = 32;
const LOGIN_SUCCESS_LEN: usize = 1 + 4 * 70 + 4 * 4 + 4 * 13 + 4 + 4;
/// `PHASE_SELECT` (`game/packet.h:795`).
const PHASE_SELECT: u8 = 3;
/// The `adwClientKey` [`client_login3`] sends, which `LOGIN2` must repeat.
const CLIENT_KEY: [u32; 4] = [0x1122_3344, 0x5566_7788, 0x99aa_bbcc, 0xddee_ff01];

/// The 52 bytes of a client `LOGIN2`, laid out by hand.
fn client_login2(login: &[u8], key: u32, client_key: [u32; 4]) -> Vec<u8> {
    let mut record = vec![CG_LOGIN2];
    let mut field = [0u8; 31];
    field[..login.len()].copy_from_slice(login);
    record.extend_from_slice(&field);
    record.extend_from_slice(&key.to_le_bytes());
    for word in client_key {
        record.extend_from_slice(&word.to_le_bytes());
    }
    assert_eq!(record.len(), LOGIN2_LEN);
    record
}

/// The key bytes of an `adwClientKey`, in memory order.
fn key_bytes(client_key: [u32; 4]) -> TeaKey {
    let mut key = [0; 16];
    for (bytes, word) in key.chunks_exact_mut(4).zip(client_key) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    key
}

/// A Channel connection as the client sees it: the key it seals input with and the key the
/// server seals output with. `SetSecurityKey` keeps the client key for input and TEA-encrypts it
/// under the Myevan key for output; `descriptor_crypto`'s own tests pin that derivation.
struct Keyed {
    client: Client,
    input: TeaKey,
    output: TeaKey,
}

impl Keyed {
    /// A connection on `address`, through the handshake, still on the setup key.
    fn channel(address: SocketAddr) -> Self {
        let (mut client, first) = accepted(address);
        assert_eq!(shake(&mut client, first), [GC_PHASE, PHASE_LOGIN]);
        Self {
            client,
            input: SETUP_KEY,
            output: SETUP_KEY,
        }
    }

    /// Send `LOGIN2`. The record goes out on the current key; the client then switches to the
    /// key pair of `client_key`, as the server does after its shutdown and user-limit checks.
    fn send_login2(&mut self, login: &[u8], key: u32, client_key: [u32; 4]) {
        let record = client_login2(login, key, client_key);
        let sealed = encrypt_padded(&record, &self.input)
            .expect("short")
            .into_bytes();
        self.client.send(&sealed);
        let keys = derive_legacy_descriptor_keys(key_bytes(client_key)).expect("derivable");
        self.input = keys.decryption_key();
        self.output = keys.encryption_key();
    }

    /// Read one record sealed on `key`, sized by its first TEA unit.
    fn read_on(&mut self, key: &TeaKey, len_of: impl Fn(u8) -> usize) -> Vec<u8> {
        let mut wire = self.client.expect_bytes(8);
        let first = decrypt_padded(&wire, key).expect("aligned");
        let len = len_of(first[0]);
        wire.extend(self.client.expect_bytes(len.div_ceil(8) * 8 - 8));
        let mut record = decrypt_padded(&wire, key).expect("aligned");
        assert!(record[len..].iter().all(|&byte| byte == 0), "{record:02x?}");
        record.truncate(len);
        record
    }

    fn read(&mut self, len_of: impl Fn(u8) -> usize) -> Vec<u8> {
        let key = self.output;
        self.read_on(&key, len_of)
    }
}

/// The `szStatus` of a `LOGIN_FAILURE` record.
fn failure_status(record: &[u8]) -> String {
    assert_eq!(record[0], GC_LOGIN_FAILURE, "{record:02x?}");
    let status = &record[1..];
    let end = status.iter().position(|&byte| byte == 0).expect("a NUL");
    assert!(status[end..].iter().all(|&byte| byte == 0), "{status:02x?}");
    String::from_utf8(status[..end].to_vec()).expect("ASCII")
}

/// What the server answers a `LOGIN2`: the empire byte and the character list, or the failure
/// status. The list is followed by the select phase.
fn login_by_key(
    keyed: &mut Keyed,
    login: &[u8],
    key: u32,
    client_key: [u32; 4],
) -> Result<(u8, Vec<u8>), String> {
    keyed.send_login2(login, key, client_key);
    let first = keyed.read(|header| match header {
        GC_EMPIRE => EMPIRE_LEN,
        GC_LOGIN_FAILURE => LOGIN_FAILURE_LEN,
        other => panic!("unexpected answer header {other}"),
    });
    if first[0] == GC_LOGIN_FAILURE {
        return Err(failure_status(&first));
    }
    let list = keyed.read(|header| {
        assert_eq!(header, GC_LOGIN_SUCCESS);
        LOGIN_SUCCESS_LEN
    });
    let phase = keyed.read(|header| {
        assert_eq!(header, GC_PHASE);
        2
    });
    assert_eq!(phase, [GC_PHASE, PHASE_SELECT]);
    Ok((first[1], list))
}

/// One `TSimplePlayer` of a character list, in source field order.
#[derive(Debug, PartialEq, Eq)]
struct Listed {
    id: u32,
    name: Vec<u8>,
    job: u8,
    level: u8,
    play_minutes: u32,
    stats: [u8; 4],
    main_part: u16,
    change_name: u8,
    hair_part: u16,
    sash_part: u16,
    x: i32,
    y: i32,
    addr: [u8; 4],
    port: u16,
    skill_group: u8,
    conqueror_and_sungma: [u8; 5],
}

/// Slot `slot` of a `LOGIN_SUCCESS` record. The offsets are the packed `TSimplePlayer`
/// (`common/tables.h`): `dwID` 0, `szName[25]` 4, `byJob` 29, `byLevel` 30, `dwPlayMinutes` 31,
/// `byST`..`byIQ` 35, `wMainPart` 39, `bChangeName` 41, `wHairPart` 42, `wSashPart` 44,
/// `bDummy[4]` 46, `x` 50, `y` 54, `lAddr` 58, `wPort` 62, `skill_group` 64,
/// `byConquerorLevel` and the four sungma bytes 65..70.
fn listed(list: &[u8], slot: usize) -> Listed {
    let at = 1 + slot * 70;
    let bytes = &list[at..at + 70];
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let word = |i: usize| [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]];
    let name = &bytes[4..29];
    let end = name
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(name.len());
    assert!(name[end..].iter().all(|&byte| byte == 0), "{name:02x?}");
    assert_eq!(&bytes[46..50], [0; 4], "bDummy");
    Listed {
        id: u32::from_le_bytes(word(0)),
        name: name[..end].to_vec(),
        job: bytes[29],
        level: bytes[30],
        play_minutes: u32::from_le_bytes(word(31)),
        stats: [bytes[35], bytes[36], bytes[37], bytes[38]],
        main_part: u16_at(39),
        change_name: bytes[41],
        hair_part: u16_at(42),
        sash_part: u16_at(44),
        x: i32::from_le_bytes(word(50)),
        y: i32::from_le_bytes(word(54)),
        addr: word(58),
        port: u16_at(62),
        skill_group: bytes[64],
        conqueror_and_sungma: [bytes[65], bytes[66], bytes[67], bytes[68], bytes[69]],
    }
}

/// Log `login` in on the auth port and return its login key.
fn login_key(server: &Server, login: &[u8]) -> (Client, u32) {
    let mut auth = auth_client(server);
    let key = log_in(&mut auth, login, ACCOUNT_PASSWORD, 1).expect("the auth login succeeds");
    (auth, key)
}

/// `LOGIN2` until the login is no longer held by a closing descriptor.
fn login_by_key_when_free(
    keyed: &mut Keyed,
    login: &[u8],
    key: u32,
) -> Result<(u8, Vec<u8>), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match login_by_key(keyed, login, key, CLIENT_KEY) {
            Err(status) if status == "ALREADY" && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            answer => return answer,
        }
    }
}

/// The public address the default configuration names: `127.0.0.1`.
const PUBLIC_ADDR: [u8; 4] = [127, 0, 0, 1];

/// Whether character slot `slot` of a `GC_LOGIN_SUCCESS` record is all zero.
fn slot_is_empty(list: &[u8], slot: usize) -> bool {
    list[1 + slot * 70..][..70].iter().all(|&byte| byte == 0)
}

/// `QUERY_LOGIN_BY_KEY`: every refusal is `NOID`, answered on the key pair the `LOGIN2`
/// carried, and leaves the connection open. `key` is alice's login key and `bob_key` bob's.
fn every_refusal_is_noid(keyed: &mut Keyed, key: u32, bob_key: u32) {
    // Login keys are 1 to `i32::MAX`, so 0 is never granted.
    assert_eq!(
        login_by_key(keyed, b"alice", 0, CLIENT_KEY),
        Err("NOID".into())
    );
    assert_eq!(
        login_by_key(keyed, b"alice", bob_key, CLIENT_KEY),
        Err("NOID".into())
    );
    assert_eq!(
        login_by_key(keyed, b"bob", key, CLIENT_KEY),
        Err("NOID".into())
    );
    assert_eq!(
        login_by_key(keyed, b"", key, CLIENT_KEY),
        Err("NOID".into())
    );
    for word in 0..4 {
        let mut wrong = CLIENT_KEY;
        wrong[word] ^= 0x0100;
        assert_eq!(
            login_by_key(keyed, b"alice", key, wrong),
            Err("NOID".into())
        );
    }
}

/// Three characters for alice in empire 1: one on a map Channel 1 hosts, one on the Shared
/// Channel's map, and one on a map no Channel hosts, which moves to the empire 1 start (map 1).
fn add_characters(database: &ScratchDatabase) {
    sql(
        database,
        "UPDATE account SET empire = 1 WHERE login = 'alice'",
    );
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, level, playtime_minutes, st, ht, dx, \
         iq, conqueror_level, sungma_str, sungma_hp, sungma_move, sungma_immune, part_main, \
         part_hair, part_sash, x, y, skill_group, change_name) SELECT id, 0, 'Alpha', 3, 154, \
         16909060, 17, 18, 19, 20, 33, 34, 35, 36, 37, 41394, 50132, 58870, 470000, 950000, 49, \
         true FROM account WHERE login = 'alice'",
    );
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 1, 'Beta', 1, 10000, \
         1210000 FROM account WHERE login = 'alice'",
    );
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 3, 'Delta', 2, 60000, \
         150000 FROM account WHERE login = 'alice'",
    );
}

/// Slot 0 of alice's list after [`add_characters`]: every field as stored, on a map Channel 1
/// hosts through `port`.
fn assert_alpha(list: &[u8], port: u16) {
    let alpha = listed(list, 0);
    assert_eq!(
        alpha,
        Listed {
            id: alpha.id,
            name: b"Alpha".to_vec(),
            job: 3,
            level: 154,
            play_minutes: 0x0102_0304,
            stats: [17, 18, 19, 20],
            main_part: 0xa1b2,
            change_name: 1,
            hair_part: 0xc3d4,
            sash_part: 0xe5f6,
            x: 470_000,
            y: 950_000,
            addr: PUBLIC_ADDR,
            port,
            skill_group: 49,
            conqueror_and_sungma: [33, 34, 35, 36, 37],
        }
    );
    assert_ne!(alpha.id, 0);
}

/// `cg.login.login2`, `sys.login.by_key`, `gc.empire`, `gc.login_success`: a Channel `LOGIN2`
/// carrying the login key, the login, and the client key of the auth login is answered on the
/// client key pair with `GC_EMPIRE`, the 357-byte character list, and the select phase. Each
/// character's position is kept when a Channel hosts its map and moved to its empire's start
/// otherwise, with the address and port of the Channel serving it. A wrong key, login, or client
/// key is `NOID`; a second holder is `ALREADY` and closes the first.
#[test]
fn the_channel_login_lists_the_characters_or_names_the_first_failed_check() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    let (_alice_auth, key) = login_key(&server, b"alice");
    let (_bob_auth, bob_key) = login_key(&server, b"bob");
    let ports: Vec<u16> = server.listeners()["channel 1"]
        .iter()
        .map(SocketAddr::port)
        .collect();
    let shared_port = server.channel(99).port();

    let mut first = Keyed::channel(server.channel(1));
    every_refusal_is_noid(&mut first, key, bob_key);
    // `trim_and_lower`, then `strcasecmp`. An account without characters shows empire 0 and
    // four empty slots.
    let (empire, list) =
        login_by_key(&mut first, b" ALICE ", key, CLIENT_KEY).expect("alice logs in");
    assert_eq!(empire, 0);
    assert!((0..4).all(|slot| slot_is_empty(&list, slot)), "empty slots");
    assert!(list[281..349].iter().all(|&byte| byte == 0), "no guilds");
    let handle = u32::from_le_bytes([list[349], list[350], list[351], list[352]]);
    let random_key = u32::from_le_bytes([list[353], list[354], list[355], list[356]]);
    assert_ne!(handle, 0);
    // `MakeRandomKey`: zero only once in 2^32 logins.
    assert_ne!(random_key, 0);

    // A second holder on another Channel port is refused, and the first descriptor is kicked
    // without a word; the refused one stays open.
    let mut second = Keyed::channel(SocketAddr::new(server.channel(1).ip(), ports[1]));
    assert_eq!(
        login_by_key(&mut second, b"alice", key, CLIENT_KEY),
        Err("ALREADY".into())
    );
    assert_eq!(first.client.expect_closed(), Vec::<u8>::new());
    assert_eq!(second.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));

    add_characters(&database);
    let (empire, list) =
        login_by_key_when_free(&mut second, b"alice", key).expect("the login is free again");
    assert_eq!(empire, 1);
    assert_alpha(&list, ports[1]);
    let beta = listed(&list, 1);
    assert_eq!(
        (
            beta.name.as_slice(),
            beta.x,
            beta.y,
            beta.addr,
            beta.port,
            beta.level
        ),
        (&b"Beta"[..], 10_000, 1_210_000, PUBLIC_ADDR, shared_port, 1)
    );
    assert!(slot_is_empty(&list, 2), "slot 2 is empty");
    let delta = listed(&list, 3);
    assert_eq!(
        (
            delta.name.as_slice(),
            delta.x,
            delta.y,
            delta.addr,
            delta.port
        ),
        (&b"Delta"[..], 469_300, 964_200, PUBLIC_ADDR, ports[1])
    );

    // The descriptor logs in again with the login it holds: it kicks itself, and legacy drops
    // the `ALREADY` it would send in `PHASE_CLOSE`.
    second.send_login2(b"alice", key, CLIENT_KEY);
    assert_eq!(second.client.expect_closed(), Vec::<u8>::new());

    // Empire 2's start is on map 21, which no Channel hosts: the character moves there with no
    // address and no port.
    sql(
        &database,
        "UPDATE account SET empire = 2 WHERE login = 'alice'",
    );
    let mut third = Keyed::channel(server.channel(1));
    let (empire, list) =
        login_by_key_when_free(&mut third, b"alice", key).expect("the login is free again");
    assert_eq!(empire, 2);
    let delta = listed(&list, 3);
    assert_eq!(
        (delta.x, delta.y, delta.addr, delta.port),
        (55_700, 157_900, [0; 4], 0)
    );

    // `LOGIN2` is read in the select phase too: another account's key logs this descriptor in
    // as that account.
    let (empire, list) = login_by_key(&mut third, b"bob", bob_key, CLIENT_KEY).expect("bob");
    assert_eq!(empire, 0);
    assert!(
        (0..4).all(|slot| slot_is_empty(&list, slot)),
        "bob has no characters"
    );
}

/// `cg.login.login2`: a closing server refuses `LOGIN2` with `SHUTDOWN` before it installs the
/// client key, so the answer and the next record stay on the setup key.
#[test]
fn a_closing_server_refuses_login2_on_the_setup_key() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "shutdowned = true",
    );
    let mut keyed = Keyed::channel(server.channel(1));
    for _ in 0..2 {
        keyed
            .client
            .send(&sealed(&client_login2(b"alice", 1, CLIENT_KEY)));
        let answer = keyed.read_on(&SETUP_KEY, |header| {
            assert_eq!(header, GC_LOGIN_FAILURE);
            LOGIN_FAILURE_LEN
        });
        assert_eq!(failure_status(&answer), "SHUTDOWN");
    }
    assert_eq!(keyed.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));
}

/// `cg.login.login2`: only the login, select, and loading phases read `LOGIN2`. In the auth
/// phase and in the handshake phase it closes the connection.
#[test]
fn a_login2_outside_the_login_phases_closes_the_connection() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    let mut auth = auth_client(&server);
    auth.send(&sealed(&client_login2(b"alice", 1, CLIENT_KEY)));
    assert_eq!(auth.expect_closed(), Vec::<u8>::new());
    let (mut client, _) = accepted(server.channel(1));
    client.send(&client_login2(b"alice", 1, CLIENT_KEY));
    assert_eq!(client.expect_closed(), Vec::<u8>::new());
}
