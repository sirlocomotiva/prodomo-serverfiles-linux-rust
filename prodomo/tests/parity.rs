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

        // Controls: each descriptor has its own token, and a wrong one closes without a
        // record. `SetPhase(PHASE_CLOSE)` assigns the phase before it calls `Packet`, and
        // `Packet` returns at once for that phase, so the record it built is dropped.
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
    assert_eq!(
        client.expect_closed(),
        Vec::<u8>::new(),
        "the retry limit closes silently: Packet drops the record once the phase is PHASE_CLOSE"
    );
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
    /// The language the account's auth login chose, which the descriptor's records carry.
    language: u8,
    /// The number of the Channel the connection is on, which `GC_CHANNEL` carries.
    channel_number: u8,
    /// Whether the character this connection enters walks, its stamina at or below 0: then
    /// its own insert is followed by its walk mode. [`Keyed::loaded`] sets it from the loading
    /// burst's `POINT_STAMINA`; a fixture row is stored at 12,345 ([`add_characters`]) unless
    /// the scenario stores 0 itself.
    walking: bool,
    /// The VIDs whose revive-invisible timer this connection has seen the entry of and not yet
    /// the expiry of. [`Keyed::timer_record`] keeps it.
    timers: Vec<[u8; 4]>,
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
            language: ENGLISH,
            channel_number: 1,
            walking: true,
            timers: Vec::new(),
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

/// `LOCALE_EN`, the language every parity client logs in with unless its scenario says
/// otherwise.
const ENGLISH: u8 = 1;
/// `LOCALE_DE`.
const GERMAN: u8 = 5;

/// Log `login` in on the auth port and return its login key.
fn login_key(server: &Server, login: &[u8]) -> (Client, u32) {
    login_key_in(server, login, ENGLISH)
}

/// [`login_key`] in `language`.
fn login_key_in(server: &Server, login: &[u8], language: u8) -> (Client, u32) {
    let mut auth = auth_client(server);
    let key = log_in(&mut auth, login, ACCOUNT_PASSWORD, language.into())
        .expect("the auth login succeeds");
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
///
/// Each is stored with its stamina above every maximum, so the event that refills a spent
/// stamina does not run and its points record stays out of the quiet windows. A scenario
/// about walking stores 0 itself.
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
    sql(
        database,
        "UPDATE player SET stamina = 12345 WHERE name IN ('Alpha', 'Beta', 'Delta')",
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

/// Where Alpha of [`add_characters`] enters the game. Its saved (470000, 950000) is a BLOCK cell
/// of map 1's `server_attr`, and the first movable point `GetMovablePosition` finds around it is
/// 100 east (`G/input_login.cpp:572-585`, `G/sectree_manager.cpp:789-812`).
const ALPHA_ENTERS_AT: (i32, i32) = (470_100, 950_000);

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

const CG_CHARACTER_CREATE: u8 = 0x04;
const CG_CHARACTER_DELETE: u8 = 0x05;
const CG_CHARACTER_SELECT: u8 = 0x06;
const CG_ENTER_GAME: u8 = 0x0a;
const CG_EMPIRE: u8 = 0x5a;
const CG_CHANGE_NAME: u8 = 0x6a;
const GC_PLAYER_CREATE_SUCCESS: u8 = 8;
const GC_CREATE_FAILURE: u8 = 9;
const GC_PLAYER_DELETE_SUCCESS: u8 = 10;
const GC_PLAYER_DELETE_WRONG_SOCIAL_ID: u8 = 11;
const GC_CHANGE_NAME: u8 = 0x6b;
/// `TPacketGCPlayerCreateSuccess`: header, `bAccountCharacterIndex`, and a `TSimplePlayer`.
const PLAYER_CREATE_SUCCESS_LEN: usize = 2 + 70;
/// `TPacketGCChangeName`: header, `pid`, and `name[25]`.
const CHANGE_NAME_LEN: usize = 1 + 4 + 25;

/// The length of each select-screen answer, by header.
fn select_answer_len(header: u8) -> usize {
    match header {
        GC_PLAYER_CREATE_SUCCESS => PLAYER_CREATE_SUCCESS_LEN,
        GC_CREATE_FAILURE | GC_PLAYER_DELETE_SUCCESS | GC_EMPIRE => 2,
        GC_PLAYER_DELETE_WRONG_SOCIAL_ID => 1,
        GC_CHANGE_NAME => CHANGE_NAME_LEN,
        GC_LOGIN_SUCCESS => LOGIN_SUCCESS_LEN,
        other => panic!("unexpected select-screen header {other}"),
    }
}

/// `CG_CHARACTER_SELECT`: the header and the one slot byte, `TPacketCGPlayerSelect`.
fn client_select(index: u8) -> Vec<u8> {
    vec![CG_CHARACTER_SELECT, index]
}

/// `GC_CHARACTER_ADD` (1), `GC_CHAR_ADDITIONAL_INFO` (136) and `GC_CHAT` (4) carry a `WORD`
/// length at bytes 1 and 2 covering the whole record. `GC_ENTITY` (249) and `GC_NPC_POSITION`
/// (115) are the same shape. The rest of the two bursts are fixed width, and those widths are the
/// ones the loading phase's own golden-byte tests pin.
fn dynamic_len(header: u8) -> usize {
    match header {
        GC_CHARACTER_ADD
        | GC_CHAR_ADDITIONAL_INFO
        | GC_CHAT
        | GC_ENTITY
        | GC_SYNC_POSITION
        | GC_NPC_POSITION
        | GC_SHOP
        | GC_SCRIPT => usize::MAX,
        other => panic!("{other} is a fixed-width loading or enter-game record"),
    }
}

/// A record whose `WORD wSize` covers the whole record, or `None` when the header is a
/// fixed-width one. Reading it is how the harness sizes a variable record.
fn word_sized(record: &[u8]) -> Option<usize> {
    let header = record[0];
    if matches!(
        header,
        GC_CHAT | GC_ENTITY | GC_SYNC_POSITION | GC_NPC_POSITION | GC_SHOP | GC_SCRIPT
    ) {
        Some(usize::from(record[1]) | (usize::from(record[2]) << 8))
    } else {
        None
    }
}

/// `CG_ENTER_GAME`: the header alone, `TPacketCGEnterGame`.
fn client_enter_game() -> Vec<u8> {
    vec![CG_ENTER_GAME]
}

/// The loading and enter-game records, by header, for the two bursts.
const GC_ENTITY: u8 = 249;
const GC_MAIN_CHARACTER2_EMPIRE: u8 = 113;
const GC_CHARACTER_GOLD: u8 = 224;
const GC_PLAYER_POINTS: u8 = 16;
const GC_SKILL_LEVEL_NEW: u8 = 76;
/// The three quickslot records (`G/packet.h`), 28 to 30.
const GC_QUICKSLOT_ADD: u8 = 28;
const GC_QUICKSLOT_DEL: u8 = 29;
const GC_QUICKSLOT_SWAP: u8 = 30;
const GC_CHARACTER_ADD: u8 = 1;
const GC_CHAR_ADDITIONAL_INFO: u8 = 136;
/// `HEADER_GC_WALK_MODE`: the header, `vid` and `mode` (`G/packet.h:2383-2388`).
const GC_WALK_MODE: u8 = 0x6f;
const WALK_MODE_LEN: usize = 1 + 4 + 1;
/// `HEADER_GC_NPC_POSITION`: `TPacketGCNPCPosition`, a `WORD wSize` and a `WORD count`, then
/// `count` 34-byte `TNPCPosition` entries (`G/packet.h`).
const GC_NPC_POSITION: u8 = 115;
/// `TNPCPosition`: `bType`, `name[25]`, `x`, `y`.
const NPC_POSITION_LEN: usize = 1 + 25 + 4 + 4;
const GC_AFFECT_ADD: u8 = 126;
/// `HEADER_GC_AFFECT_REMOVE`: `TPacketGCAffectRemove`, a header, a `DWORD dwType` and a `BYTE
/// bApplyOn` (`G/packet.h`), six bytes in all.
const GC_AFFECT_REMOVE: u8 = 127;
const AFFECT_REMOVE_LEN: usize = 1 + 4 + 1;
/// `AFFECT_REVIVE_INVISIBLE`: the `AffectType` of the affect the revive timer runs (`G/char.cpp`).
const AFFECT_REVIVE_INVISIBLE: u32 = 215;
const GC_TIME: u8 = 106;
const GC_CHANNEL: u8 = 121;
const GC_CHAT: u8 = 4;
/// `PHASE_LOADING` and `PHASE_GAME` in `EPhase` (`G/desc.h`).
const PHASE_LOADING: u8 = 4;
const PHASE_GAME: u8 = 5;
/// `GC_PHASE` carries one payload byte.
const PHASE_LEN: usize = 2;
/// `GC_MAIN_CHARACTER2_EMPIRE`: header, `dwVID`, `bJob`, `szName[25]`, `x`, `y`, `z`,
/// `bEmpire`, `bSkillGroup` (46 bytes under the packed x86 profile).
const MAIN_CHARACTER_LEN: usize = 1 + 4 + 2 + 25 + 4 + 4 + 4 + 1 + 1;
/// `GC_CHARACTER_GOLD`: header and a `long long` (`ENABLE_REMOVE_LIMIT_GOLD`).
const GOLD_LEN: usize = 1 + 8;
/// `TPacketGCCharacterAdd`: header, `dwVID`, `angle`, `x`, `y`, `z`, `bType`, `wRaceNum`,
/// `bMovingSpeed`, `bAttackSpeed`, `bStateFlag`, `dwAffectFlag[2]`, with no `wSize`.
const CHARACTER_ADD_LEN: usize = 1 + 4 + 4 + 4 + 4 + 4 + 1 + 2 + 1 + 1 + 1 + 8;
/// `TPacketGCCharacterAdditionalInfo`: the own-character record, whose `bLanguage` is its last
/// field, at offset 69.
const CHAR_ADDITIONAL_INFO_LEN: usize = 70;
/// `GC_PLAYER_POINTS`: header and 255 eight-byte point entries.
const POINTS_LEN: usize = 1 + 255 * 8;
/// `GC_SKILL_LEVEL_NEW`: header and 255 six-byte skill entries.
const SKILL_LEVEL_LEN: usize = 1 + 255 * 6;
/// `GC_AFFECT_ADD`: header and one 21-byte affect element.
const AFFECT_ADD_LEN: usize = 1 + 21;
/// `GC_TIME`: header and a `long`.
const TIME_LEN: usize = 1 + 4;
/// `GC_CHANNEL`: header and one byte.
const CHANNEL_LEN: usize = 1 + 1;
/// `HEADER_GC_CHARACTER_POINT_CHANGE`, the first byte of its four-byte `int` header.
const GC_POINT_CHANGE: u8 = 17;
/// `TPacketGCPointChange`: an `int` header, `dwVID`, `type`, and two `long long`
/// (`G/packet.h:1064-1071`).
const POINT_CHANGE_LEN: usize = 4 + 4 + 1 + 8 + 8;

/// `POINT_STAMINA`, the points record's slot of the current stamina.
const POINT_STAMINA: usize = 9;

/// The length of each loading and enter-game record. A variable record reports `usize::MAX`, and
/// the caller sizes it from its own `WORD wSize`.
fn game_len(header: u8) -> usize {
    match header {
        GC_PHASE => PHASE_LEN,
        GC_MAIN_CHARACTER2_EMPIRE => MAIN_CHARACTER_LEN,
        GC_CHARACTER_GOLD => GOLD_LEN,
        GC_PLAYER_POINTS => POINTS_LEN,
        GC_SKILL_LEVEL_NEW => SKILL_LEVEL_LEN,
        GC_CHARACTER_ADD => CHARACTER_ADD_LEN,
        GC_CHAR_ADDITIONAL_INFO => CHAR_ADDITIONAL_INFO_LEN,
        GC_ENTITY | GC_CHAT | GC_NPC_POSITION | GC_SHOP | GC_SCRIPT => dynamic_len(header),
        GC_AFFECT_ADD => AFFECT_ADD_LEN,
        GC_AFFECT_REMOVE => AFFECT_REMOVE_LEN,
        GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
        GC_TIME => TIME_LEN,
        GC_CHANNEL => CHANNEL_LEN,
        GC_POINT_CHANGE => POINT_CHANGE_LEN,
        GC_GOLD_CHANGE => GOLD_CHANGE_LEN,
        GC_MOVE => GC_MOVE_LEN,
        GC_CHARACTER_POSITION => CHARACTER_POSITION_LEN,
        GC_SYNC_POSITION => usize::MAX,
        GC_OWNERSHIP => OWNERSHIP_LEN,
        ITEM_SET | GC_SAFEBOX_SET | GC_MALL_SET => ITEM_SET_LEN,
        ITEM_UPDATE => ITEM_UPDATE_LEN,
        GROUND_ADD => GROUND_ADD_LEN,
        GROUND_DEL => GROUND_DEL_LEN,
        GC_QUICKSLOT_ADD => 1 + 1 + 2,
        GC_QUICKSLOT_DEL | GC_SAFEBOX_SIZE | GC_MALL_OPEN => 1 + 1,
        GC_QUICKSLOT_SWAP => 1 + 2,
        GC_EXCHANGE => EXCHANGE_LEN,
        GC_SAFEBOX_DEL | GC_MALL_DEL | GC_CHARACTER_DEL => 1 + 4,
        GC_SAFEBOX_WRONG_PASSWORD => 1,
        GC_WARP => WARP_LEN,
        GC_WALK_MODE => WALK_MODE_LEN,
        other => panic!("unexpected loading or enter-game header {other}"),
    }
}

/// `cg.world.move`, `sys.world.move`: the headers of the movement and chat records, the widths the
/// loading phase's golden-byte tests pin, and the record builders a client needs to play them.
const CG_CHAT: u8 = 0x03;
const CG_MOVE: u8 = 0x07;
const CG_SYNC_POSITION: u8 = 0x08;
const CG_CHARACTER_POSITION: u8 = 0x1c;
/// `HEADER_CG_ON_CLICK`.
const CG_ON_CLICK: u8 = 0x1a;
/// `HEADER_CG_SHOP`: the header, then the subheader and its body.
const CG_SHOP: u8 = 50;
const GC_MOVE: u8 = 0x03;
const GC_SYNC_POSITION: u8 = 0x05;
const GC_CHARACTER_POSITION: u8 = 0x2b;

/// `TPacketCGMove` is 16 bytes: the three header bytes, then `lX`, `lY`, `dwTime`,
/// `dwDuration`, and `bFunc` on the wire.
const MOVE_LEN: usize = 16;
/// `TPacketGCMove` is 24 bytes in the struct order at `server/server/game/packet.h:1701-1712`.
const GC_MOVE_LEN: usize = 24;
/// `GC_SYNC_POSITION` is a three-byte prefix plus 12 bytes per element.
const SYNC_POSITION_LEN: usize = 3 + 12;
/// `HEADER_GC_OWNERSHIP` is 62, from `packet.h:151`.
const GC_OWNERSHIP: u8 = 62;
/// `TPacketGCOwnership` is the header plus two DWORDs.
const OWNERSHIP_LEN: usize = 1 + 4 + 4;
/// `GC_CHARACTER_POSITION` is the header, the VID, and the pose.
const CHARACTER_POSITION_LEN: usize = 6;
/// The `EChatType` byte of a talking line.
const CHAT_TALKING: u8 = 0;
/// `EPosition`, of which the Rewrite honours all three.
const POSITION_GENERAL: u8 = 0;
const POSITION_SITTING_CHAIR: u8 = 1;
const POSITION_SITTING_GROUND: u8 = 2;

/// The 16 bytes of a client `MOVE`, in `TPacketCGMove` field order with the header first.
fn client_move(function: u8, argument: u8, rotation: u8, x: i32, y: i32, time: u32) -> Vec<u8> {
    let mut fixed = vec![CG_MOVE, function, argument, rotation];
    fixed.extend_from_slice(&x.to_le_bytes());
    fixed.extend_from_slice(&y.to_le_bytes());
    fixed.extend_from_slice(&time.to_le_bytes());
    assert_eq!(fixed.len(), MOVE_LEN, "TPacketCGMove has no dwDuration");
    fixed
}

/// The `GC_MOVE` a viewer is sent for the client `MOVE` `sent`: its function, argument and
/// rotation, the mover's `vid`, its position and time as sent, then the `duration`.
fn relayed_move(sent: &[u8], vid: u32, duration: u32) -> Vec<u8> {
    let mut relay = vec![GC_MOVE];
    relay.extend_from_slice(&sent[1..4]);
    relay.extend_from_slice(&vid.to_le_bytes());
    relay.extend_from_slice(&sent[4..MOVE_LEN]);
    relay.extend_from_slice(&duration.to_le_bytes());
    assert_eq!(relay.len(), GC_MOVE_LEN, "TPacketGCMove");
    relay
}

/// The 4 header bytes plus the text of a client `CHAT`. `size` counts the whole record.
fn client_chat(chat_type: u8, text: &[u8]) -> Vec<u8> {
    let total = 4 + text.len();
    let mut record = vec![CG_CHAT];
    record.extend_from_slice(&u16::try_from(total).expect("a short line").to_le_bytes());
    record.push(chat_type);
    record.extend_from_slice(text);
    record
}

/// The 2 bytes of a client `CHARACTER_POSITION`, which are the header and the pose.
fn client_position(position: u8) -> Vec<u8> {
    vec![CG_CHARACTER_POSITION, position]
}

/// A client `SYNC_POSITION`: the three-byte prefix, then one 12-byte element per victim.
fn client_sync_position(elements: &[(u32, i32, i32)]) -> Vec<u8> {
    let total = SYNC_POSITION_LEN - 12 + 12 * elements.len();
    let mut record = vec![CG_SYNC_POSITION];
    record.extend_from_slice(&u16::try_from(total).expect("16 elements fit").to_le_bytes());
    for (vid, x, y) in elements {
        record.extend_from_slice(&vid.to_le_bytes());
        record.extend_from_slice(&x.to_le_bytes());
        record.extend_from_slice(&y.to_le_bytes());
    }
    record
}

/// Log `login` in, select slot `slot`, and enter the game, leaving the connection in the game
/// phase with every loading and enter-game record already read.
fn enter_world(server: &Server, login: &[u8], slot: u8) -> (Keyed, Listed) {
    enter_world_in(server, login, slot, ENGLISH)
}

/// [`enter_world`] for an auth login in `language`.
fn enter_world_in(server: &Server, login: &[u8], slot: u8, language: u8) -> (Keyed, Listed) {
    enter_world_on(server, 1, login, slot, language)
}

/// [`enter_world_in`] on Channel `channel`.
fn enter_world_on(
    server: &Server,
    channel: u8,
    login: &[u8],
    slot: u8,
    language: u8,
) -> (Keyed, Listed) {
    let (keyed, character, _entered) =
        enter_world_seeing_on(server, channel, login, slot, language);
    (keyed, character)
}

/// [`enter_world`], answering what the burst showed.
fn enter_world_seeing(server: &Server, login: &[u8], slot: u8) -> (Keyed, Listed, Entered) {
    enter_world_seeing_on(server, 1, login, slot, ENGLISH)
}

/// [`enter_world_on`], answering what the burst showed.
fn enter_world_seeing_on(
    server: &Server,
    channel: u8,
    login: &[u8],
    slot: u8,
    language: u8,
) -> (Keyed, Listed, Entered) {
    let (mut keyed, character, quickslots, _items) =
        load_with_quickslots_on(server, channel, login, slot, language);
    assert_eq!(quickslots, Vec::<Vec<u8>>::new(), "no quickslot is stored");
    let entered = enter_game_view(&mut keyed);
    (keyed, character, entered)
}

/// Log `login` in and select slot `slot`, reading the loading burst and the item load behind
/// it, and answer with the `GC_ITEM_SET` records in the order they arrived. The connection is
/// left in the loading phase.
///
/// `CInputDB::ItemLoad` writes one `GC_ITEM_SET` per placed item and ends with a second
/// `PointsPacket` (`G/input_db.cpp:1564`), so the gold and points records arrive twice: once
/// before the skill levels and once after the items. Nothing between the two changes a point,
/// so the second pair is the first pair's bytes.
fn load_character(server: &Server, login: &[u8], slot: u8) -> (Keyed, Listed, Vec<Vec<u8>>) {
    let (keyed, character, quickslots, items) = load_with_quickslots(server, login, slot);
    assert_eq!(quickslots, Vec::<Vec<u8>>::new(), "no quickslot is stored");
    (keyed, character, items)
}

/// [`load_character`] for a character with stored quickslots: also answers the
/// `GC_QUICKSLOT_ADD` records the load sent between the main character and the gold, where
/// `PlayerLoad` sets them (`G/input_db.cpp:439-440`).
fn load_with_quickslots(
    server: &Server,
    login: &[u8],
    slot: u8,
) -> (Keyed, Listed, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    load_with_quickslots_in(server, login, slot, ENGLISH)
}

/// [`load_with_quickslots`] for an auth login in `language`.
fn load_with_quickslots_in(
    server: &Server,
    login: &[u8],
    slot: u8,
    language: u8,
) -> (Keyed, Listed, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    load_with_quickslots_on(server, 1, login, slot, language)
}

/// [`load_with_quickslots_in`] on Channel `channel`.
fn load_with_quickslots_on(
    server: &Server,
    channel: u8,
    login: &[u8],
    slot: u8,
    language: u8,
) -> (Keyed, Listed, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let (mut keyed, _empire, list) = select_screen_on(server, channel, login, language);
    let (character, quickslots, items) = load_selected(&mut keyed, &list, slot);
    (keyed, character, quickslots, items)
}

/// Select slot `slot` of the character list `list` from the select phase and read what
/// [`load_with_quickslots`] reads: the character, its quickslot records and its item records.
fn load_selected(keyed: &mut Keyed, list: &[u8], slot: u8) -> (Listed, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let burst = read_loading_burst(keyed, list, slot);
    (burst.character, burst.quickslots, burst.items)
}

/// What [`read_loading_burst`] reads.
struct LoadingBurst {
    /// The selected character, as the list gave it.
    character: Listed,
    /// The `GC_MAIN_CHARACTER` record.
    main: Vec<u8>,
    /// The `GC_QUICKSLOT_ADD` records.
    quickslots: Vec<Vec<u8>>,
    /// The `GC_ITEM_SET` records.
    items: Vec<Vec<u8>>,
}

/// [`load_selected`], also answering the loading burst's `GC_MAIN_CHARACTER`.
fn read_loading_burst(keyed: &mut Keyed, list: &[u8], slot: u8) -> LoadingBurst {
    let character = listed(list, usize::from(slot));
    keyed.send_record(&client_select(slot));
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_LOADING]);
    assert_eq!(keyed.read_game(), [GC_ENTITY, 3, 0]);
    let main = keyed.read_game();
    assert_eq!(main[0], GC_MAIN_CHARACTER2_EMPIRE);
    let mut quickslots = Vec::new();
    let gold = loop {
        let record = keyed.read_game();
        if record[0] != GC_QUICKSLOT_ADD {
            break record;
        }
        quickslots.push(record);
    };
    assert_eq!(gold[0], GC_CHARACTER_GOLD);
    let points = keyed.read_game();
    assert_eq!(points[0], GC_PLAYER_POINTS);
    keyed.loaded(&points);
    assert_eq!(keyed.read_game()[0], GC_SKILL_LEVEL_NEW);
    let mut items = Vec::new();
    let tail = loop {
        let record = keyed.read_game();
        if record[0] != ITEM_SET {
            break record;
        }
        items.push(record);
    };
    assert_eq!(tail, gold, "the item load ends with the gold record");
    assert_eq!(keyed.read_game(), points, "then the points record");
    LoadingBurst {
        character,
        main,
        quickslots,
        items,
    }
}

/// Send `CG_ENTER_GAME` from the loading phase and read the enter-game burst, leaving the
/// connection in the game phase with nothing unread. Answers the character's own
/// `GC_CHARACTER_ADD`.
fn enter_game_burst(keyed: &mut Keyed) -> Vec<u8> {
    enter_game_view(keyed).add
}

/// [`enter_game_burst`], answering what the burst showed.
fn enter_game_view(keyed: &mut Keyed) -> Entered {
    let entered = enter_game_shown(keyed);
    assert_eq!(keyed.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));
    entered
}

/// [`enter_game_burst`] without the final quiet check, for a burst that more records follow.
fn enter_game_records(keyed: &mut Keyed) -> Vec<u8> {
    enter_game_shown(keyed).add
}

/// What the enter-game burst showed the entrant.
struct Entered {
    /// Its own `GC_CHARACTER_ADD`.
    add: Vec<u8>,
    /// Its own `GC_CHAR_ADDITIONAL_INFO`.
    info: Vec<u8>,
    /// Everything in view, after its own pair and walk mode.
    shown: Shown,
    /// Its `GC_AFFECT_ADD` for the revive-invisible affect, the record after its look update.
    affect: Vec<u8>,
}

/// [`enter_game_records`], answering what the burst showed.
fn enter_game_shown(keyed: &mut Keyed) -> Entered {
    keyed.send_record(&client_enter_game());
    let add = keyed.read_game();
    assert_eq!(add[0], GC_CHARACTER_ADD);
    let info = keyed.read_game();
    assert_eq!(info[0], GC_CHAR_ADDITIONAL_INFO);
    assert_eq!(
        info[69], keyed.language,
        "bLanguage from the descriptor: the language the auth login chose"
    );
    let own_walk = walk_mode_of(&add[1..5]);
    if keyed.walking {
        assert_eq!(keyed.read_game(), own_walk, "a spent stamina walks");
    }
    let (shown, affect) = read_shown(keyed);
    assert!(
        !shown.records.contains(&own_walk),
        "its own walk mode once at most"
    );
    let update = shown
        .update
        .as_deref()
        .expect("the entrant's GC_CHARACTER_UPDATE follows the map's records");
    assert_eq!(
        update.len(),
        CHARACTER_UPDATE_LEN,
        "no wSize: a fixed record"
    );
    assert_eq!(&update[1..5], &add[1..5], "dwVID is the entrant's own");
    assert_eq!(
        &update[20..28],
        &[0, 0, 0, 0x08, 0, 0, 0, 0],
        "the revive-invisible flag 28 is bit 27 of word 0 (V9)"
    );
    assert_eq!(affect[0], GC_AFFECT_ADD);
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_GAME]);
    assert_eq!(keyed.read_game()[0], GC_TIME);
    assert_eq!(keyed.read_game(), [GC_CHANNEL, keyed.channel_number]);
    let notice = keyed.read_game();
    assert_eq!(notice[0], GC_CHAT);
    Entered {
        add,
        info,
        shown,
        affect,
    }
}

/// What `Show` and `SendNPCPosition` send after the own-character pair: every character in view,
/// each as its `GC_CHARACTER_ADD` and, for a PC or an NPC, its `GC_CHAR_ADDITIONAL_INFO`, a
/// moving player's `GC_MOVE` and walk mode, the walk mode of a walking entrant, each item lying
/// in view as its `GC_ITEM_GROUND_ADD`, and then the map's NPC list when the map has one.
struct Shown {
    /// The insert and summary records, in the order they arrived.
    records: Vec<Vec<u8>>,
    /// The `GC_NPC_POSITION` record, when the map lists any NPC.
    list: Option<Vec<u8>>,
    /// The entrant's own `GC_CHARACTER_UPDATE`, which the burst sends after the map's list and
    /// before its `GC_AFFECT_ADD`.
    update: Option<Vec<u8>>,
}

impl Shown {
    /// The `GC_CHARACTER_ADD` records alone.
    fn inserts(&self) -> Vec<&[u8]> {
        self.records
            .iter()
            .filter(|record| record[0] == GC_CHARACTER_ADD)
            .map(Vec::as_slice)
            .collect()
    }

    /// The `GC_ITEM_GROUND_ADD` records alone, each with its place among the shown.
    fn ground_adds(&self) -> Vec<(usize, &[u8])> {
        self.records
            .iter()
            .enumerate()
            .filter(|(_, record)| record[0] == GROUND_ADD)
            .map(|(at, record)| (at, record.as_slice()))
            .collect()
    }

    /// The place among the shown of the insert of the character `vid`.
    fn insert_at(&self, vid: u32) -> usize {
        let id = vid.to_le_bytes();
        self.records
            .iter()
            .position(|record| record[0] == GC_CHARACTER_ADD && record[1..5] == id)
            .unwrap_or_else(|| panic!("{vid} is shown"))
    }

    /// The insert of the character `vid` and the summary right behind it.
    fn pair_of(&self, vid: u32) -> (Vec<u8>, Vec<u8>) {
        let id = vid.to_le_bytes();
        let at = self.insert_at(vid);
        let info = self.records[at + 1].clone();
        assert_eq!(info[0], GC_CHAR_ADDITIONAL_INFO, "{info:02x?}");
        assert_eq!(info[1..5], id, "the summary's vid");
        (self.records[at].clone(), info)
    }
}

/// `[6f, vid, 00]`: the run walk mode of the character under the little-endian `vid`, the only
/// mode the view sends.
fn walk_mode_of(vid: &[u8]) -> Vec<u8> {
    let mut record = vec![GC_WALK_MODE];
    record.extend_from_slice(vid);
    record.push(0);
    record
}

impl Keyed {
    /// Note the loading burst's points record `points`: the character walks when its
    /// `POINT_STAMINA` is at or below 0.
    fn loaded(&mut self, points: &[u8]) {
        self.walking = point_slot(points, POINT_STAMINA) <= 0;
    }

    /// Read the view insert of the character `vid`, which walks when `walking`: its walk mode
    /// first, which the viewer's own insert to it sends back (`G/char.cpp:1225-1236`), then its
    /// `GC_CHARACTER_ADD` and `GC_CHAR_ADDITIONAL_INFO`.
    fn sees_arrive(&mut self, vid: u32, walking: bool) -> (Vec<u8>, Vec<u8>) {
        let id = vid.to_le_bytes();
        if walking {
            assert_eq!(self.read_game(), walk_mode_of(&id), "the newcomer walks");
        }
        let add = self.read_game();
        assert_eq!(add[0], GC_CHARACTER_ADD, "{add:02x?}");
        assert_eq!(&add[1..5], &id, "the insert's vid");
        let info = self.read_game();
        assert_eq!(info[0], GC_CHAR_ADDITIONAL_INFO, "{info:02x?}");
        assert_eq!(&info[1..5], &id, "the summary's vid");
        (add, info)
    }

    /// Read the view removal of the character `vid`: `[02, vid]`.
    fn sees_leave(&mut self, vid: u32) {
        let mut removal = vec![GC_CHARACTER_DEL];
        removal.extend_from_slice(&vid.to_le_bytes());
        assert_eq!(self.read_game(), removal, "the removal of {vid}");
    }

    /// Read a refused move's re-encode up to the chat line it is answered with: the VIDs of
    /// its inserts, then the line. Any other record first fails.
    fn reencode_then_line(&mut self) -> (Vec<u32>, Vec<u8>) {
        let mut inserted = Vec::new();
        loop {
            let record = self.read_game();
            match record.first().copied() {
                Some(GC_CHAT) => return (inserted, record),
                Some(GC_CHARACTER_ADD) => {
                    inserted.push(u32::from_le_bytes(record[1..5].try_into().expect("a VID")));
                }
                Some(GC_CHAR_ADDITIONAL_INFO) => {}
                other => panic!("only the re-encode precedes the line, not {other:02x?}"),
            }
        }
    }
}

/// Store `value` as the stamina of the character `name`.
fn stamina(database: &ScratchDatabase, name: &str, value: i32) {
    sql(
        database,
        &format!("UPDATE player SET stamina = {value} WHERE name = '{name}'"),
    );
}

/// Read what [`Shown`] describes, answering it with the record that follows it.
fn read_shown(keyed: &mut Keyed) -> (Shown, Vec<u8>) {
    let mut records = Vec::new();
    let mut next = keyed.read_game_raw();
    while matches!(
        next[0],
        GC_CHARACTER_ADD | GC_CHAR_ADDITIONAL_INFO | GC_MOVE | GC_WALK_MODE | GROUND_ADD
    ) {
        // Noted, so the expiry of an insert that carries the revive flag is recognised.
        keyed.timer_record(&next);
        records.push(next);
        next = keyed.read_game_raw();
    }
    let list = if next[0] == GC_NPC_POSITION {
        assert_eq!(
            word_sized(&next),
            Some(next.len()),
            "wSize is the whole record"
        );
        let list = next;
        next = keyed.read_game_raw();
        Some(list)
    } else {
        None
    };
    let update = if next[0] == GC_CHARACTER_UPDATE {
        let update = next;
        // Read raw, because it is the entry the scenario asserts on. Noting it lets the
        // expiry that follows be recognised as the timer's.
        keyed.timer_record(&update);
        next = keyed.read_game_raw();
        Some(update)
    } else {
        None
    };
    (
        Shown {
            records,
            list,
            update,
        },
        next,
    )
}

/// A 25-byte Name field holding `name`, NUL-padded.
fn name_field(name: &[u8]) -> [u8; 25] {
    let mut field = [0; 25];
    field[..name.len()].copy_from_slice(name);
    field
}

/// The 34 bytes of a client `CHARACTER_CREATE`, in `TPacketCGPlayerCreate` field order. The four
/// stat bytes are not read by the server.
fn client_create(index: u8, name: &[u8], job: u16, shape: u8) -> Vec<u8> {
    let mut record = vec![CG_CHARACTER_CREATE, index];
    record.extend_from_slice(&name_field(name));
    record.extend_from_slice(&job.to_le_bytes());
    record.extend_from_slice(&[shape, 0xa1, 0xb2, 0xc3, 0xd4]);
    assert_eq!(record.len(), 34);
    record
}

/// The 10 bytes of a client `CHARACTER_DELETE`: header, index, and `private_code[8]`.
fn client_delete(index: u8, code: [u8; 8]) -> Vec<u8> {
    let mut record = vec![CG_CHARACTER_DELETE, index];
    record.extend_from_slice(&code);
    record
}

/// The 27 bytes of a client `CHANGE_NAME`: header, index, and `name[25]`.
fn client_rename(index: u8, name: &[u8]) -> Vec<u8> {
    let mut record = vec![CG_CHANGE_NAME, index];
    record.extend_from_slice(&name_field(name));
    record
}

impl Keyed {
    /// Send one record sealed on the current input key.
    fn send_record(&mut self, record: &[u8]) {
        let sealed = encrypt_padded(record, &self.input)
            .expect("short")
            .into_bytes();
        self.client.send(&sealed);
    }

    /// Send one select-screen record and read the answer.
    fn answer(&mut self, record: &[u8]) -> Vec<u8> {
        self.send_record(record);
        self.read(select_answer_len)
    }

    /// Note `record` if it is one of the revive timer's own records, and report whether it is.
    ///
    /// The timer reaches a connection that sees the character twice. The first look update
    /// for a VID with the revive flag (flag 28, bit 27 of word 0) is its entry, and the look
    /// update with no flags that follows is its expiry. An equipment change made while the
    /// character is still invisible also carries the flag, but its VID is already entered, so
    /// it is not the timer's and is not reported. The entrant's own connection also reads the
    /// affect's `GC_AFFECT_REMOVE` (type 215, apply 0) when it runs out.
    ///
    /// A view insert carries the same flag word (`dwAffectFlag`, bytes 27..31), so an insert
    /// with the flag enters the VID too: a walker that comes into view while its timer runs
    /// is never sent the look update, but the expiry still reaches the viewer. An insert is
    /// noted and not reported, and a removal forgets its VID, since the timer is then nobody
    /// else's to see end.
    fn timer_record(&mut self, record: &[u8]) -> bool {
        match record[0] {
            GC_AFFECT_REMOVE => {
                record[1..5] == AFFECT_REVIVE_INVISIBLE.to_le_bytes() && record[5] == 0
            }
            GC_CHARACTER_UPDATE => {
                let vid: [u8; 4] = record[1..5].try_into().expect("a VID");
                let flags = &record[20..28];
                let entered = self.timers.iter().position(|seen| *seen == vid);
                match (flags[3] & 0x08 != 0, entered) {
                    (true, None) => {
                        self.timers.push(vid);
                        true
                    }
                    (false, Some(at)) if flags.iter().all(|&byte| byte == 0) => {
                        self.timers.swap_remove(at);
                        true
                    }
                    _ => false,
                }
            }
            GC_CHARACTER_ADD => {
                let vid: [u8; 4] = record[1..5].try_into().expect("a VID");
                if record[30] & 0x08 != 0 && !self.timers.contains(&vid) {
                    self.timers.push(vid);
                }
                false
            }
            GC_CHARACTER_DEL => {
                let vid: [u8; 4] = record[1..5].try_into().expect("a VID");
                self.timers.retain(|seen| *seen != vid);
                false
            }
            _ => false,
        }
    }

    /// Assert that nothing but a `GC_PING` cycle arrives, and absorb the revive timer's own
    /// records ([`Keyed::timer_record`]).
    ///
    /// A ping is on its own timer and says nothing about the record just sent, so a ping
    /// inside the window is not an answer. Anything else is. The window's bytes are walked as
    /// records, each at its own width, because a look update is 55 bytes and not one unit.
    fn quiet(&mut self, note: &str) {
        self.quiet_for(note, QUIET_WINDOW);
    }

    /// [`Keyed::quiet`] over a `window` of its own, for a check that must outlast a second.
    fn quiet_for(&mut self, note: &str, window: Duration) {
        let key = self.output;
        let deadline = std::time::Instant::now() + window;
        let mut wire = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            match self.client.drain(left) {
                (bytes, Quiet::Open) if bytes.is_empty() => break,
                (bytes, Quiet::Open) => wire.extend(bytes),
                (_, closed) => panic!("{note} closed the connection: {closed:?}"),
            }
        }
        assert_eq!(
            wire.len() % 8,
            0,
            "the wire carries whole TEA units ({note})"
        );
        let plain: Vec<u8> = wire
            .chunks(8)
            .flat_map(|unit| decrypt_padded(unit, &key).expect("aligned"))
            .collect();
        let mut at = 0;
        while let Some(&header) = plain.get(at) {
            let width = match header {
                GC_PING => 1,
                GC_AFFECT_REMOVE => AFFECT_REMOVE_LEN,
                GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
                other => panic!(
                    "only a ping may arrive while nothing is expected ({note}): header {other}"
                ),
            };
            assert!(
                at + width <= plain.len(),
                "a record still arriving when the window closed ({note})"
            );
            let record = &plain[at..at + width];
            assert!(
                header == GC_PING || self.timer_record(record),
                "only a ping may arrive while nothing is expected ({note}): {record:02x?}"
            );
            at += width.div_ceil(8) * 8;
        }
    }

    /// Send one select-screen record that the server answers with nothing, leaving the
    /// connection open.
    fn unanswered(&mut self, record: &[u8]) {
        self.send_record(record);
        self.quiet("a refused record");
    }

    /// Send one select-screen record that closes the connection without an answer.
    fn closed_by(&mut self, record: &[u8]) {
        self.send_record(record);
        assert_eq!(self.client.expect_closed(), Vec::<u8>::new());
    }
    /// Read one record whose length the first TEA unit already carries, which is how every
    /// `WORD wSize` record sizes itself on the wire.
    fn read_sized(&mut self, len_of: impl Fn(u8, &[u8]) -> usize) -> Vec<u8> {
        let key = self.output;
        let mut wire = self.client.expect_bytes(8);
        let first = decrypt_padded(&wire, &key).expect("aligned");
        let len = len_of(first[0], &first);
        wire.extend(self.client.expect_bytes(len.div_ceil(8) * 8 - 8));
        let mut record = decrypt_padded(&wire, &key).expect("aligned");
        assert!(record[len..].iter().all(|&byte| byte == 0), "{record:02x?}");
        record.truncate(len);
        record
    }

    /// Read the next loading or enter-game record that is not the revive timer's own: those
    /// are noted by [`Keyed::timer_record`] and skipped. A scenario that checks the timer reads
    /// with [`Keyed::read_game_raw`] instead.
    fn read_game(&mut self) -> Vec<u8> {
        loop {
            let record = self.read_game_raw();
            if !self.timer_record(&record) {
                return record;
            }
        }
    }

    /// Read the next loading or enter-game record, sizing a `WORD wSize` record from the first
    /// TEA unit and a fixed-width one from [`game_len`].
    fn read_game_raw(&mut self) -> Vec<u8> {
        self.read_sized(|header, first| match game_len(header) {
            usize::MAX => usize::from(first[1]) | (usize::from(first[2]) << 8),
            fixed => fixed,
        })
    }

    /// Every complete record that arrives within `window`, decrypted.
    ///
    /// [`Keyed::read_game`] has to know a record's width before it can read it, which does
    /// not work for a record nobody predicted: an Operator's grant is not part of any
    /// scripted exchange. So this reads a quiet window, decrypts it, and walks the
    /// plaintext: each record declares its own width from its header byte, and legacy
    /// zero-pads every record to a multiple of eight before encrypting it, so the walk
    /// stays aligned.
    ///
    /// A record whose bytes are still arriving is **not** returned and the walk stops
    /// there: a partial record is a record in flight, and a scenario that made claims
    /// about half of one would be making them about nothing. The scenario asks for a
    /// longer window rather than reading a half-record.
    ///
    /// `width_of` sizes a record from its header byte. It is the caller's, not
    /// [`game_len`]'s, because the loading and enter-game bursts are a known set of
    /// records and a grant is not, and a total function over "everything the client might
    /// receive at any time" is exactly the kind of table that rots. A header `width_of`
    /// does not know panics, because a record nobody accounted for arriving in the window
    /// is a finding rather than noise, and a silent skip would read as "nothing arrived".
    fn drain_game(
        &mut self,
        window: Duration,
        width_of: impl Fn(u8) -> usize,
    ) -> (Vec<Vec<u8>>, Quiet) {
        let (bytes, quiet) = self.client.drain(window);
        let key = self.output;
        let plain: Vec<u8> = bytes
            .chunks(8)
            .filter(|unit| unit.len() == 8)
            .filter_map(|unit| decrypt_padded(unit, &key).ok())
            .flatten()
            .collect();
        let mut records = Vec::new();
        let mut at = 0;
        while at < plain.len() {
            let header = plain[at];
            let width = width_of(header);
            let padded = width.div_ceil(8) * 8;
            if at + width > plain.len() {
                // In flight, or the last unit of a record whose padding has not landed.
                break;
            }
            records.push(plain[at..at + width].to_vec());
            at += padded;
        }
        // Noted but not removed: a drain reports every record, the timer's included.
        for record in &records {
            self.timer_record(record);
        }
        (records, quiet)
    }

    /// Read the next loading or enter-game record, or `None` when the descriptor closed the
    /// connection without one, which is how a refused load looks from the client side.
    fn read_game_or_close(&mut self) -> Option<Vec<u8>> {
        let (bytes, quiet) = self.client.drain(Duration::from_secs(2));
        assert_eq!(
            quiet,
            Quiet::Closed,
            "the descriptor closed after {} bytes",
            bytes.len()
        );
        if bytes.is_empty() {
            return None;
        }
        let key = self.output;
        let head = decrypt_padded(&bytes[..8.min(bytes.len())], &key).expect("aligned");
        let len = match game_len(head[0]) {
            usize::MAX => usize::from(head[1]) | (usize::from(head[2]) << 8),
            fixed => fixed,
        };
        let mut wire = bytes;
        while wire.len() < len.div_ceil(8) * 8 {
            wire.extend(self.client.expect_bytes(8));
        }
        let mut record = decrypt_padded(&wire, &key).expect("aligned");
        assert!(record[len..].iter().all(|&byte| byte == 0), "{record:02x?}");
        record.truncate(len);
        Some(record)
    }
}

/// The statement that raises unless `condition` is true. A NULL condition raises too: an
/// aggregate over no rows is NULL, and `IF NOT (NULL)` would take neither branch and pass.
fn assertion(condition: &str) -> String {
    format!(
        "DO $$ BEGIN IF ({condition}) IS NOT TRUE THEN RAISE EXCEPTION 'failed'; END IF; END $$"
    )
}

/// Assert a condition on the scenario's database.
fn check(database: &ScratchDatabase, condition: &str) {
    execute(database.url(), &assertion(condition))
        .unwrap_or_else(|error| panic!("{condition}: {error}"));
}

/// Assert a condition once it holds, up to a deadline.
///
/// A save is an effect of the server, not an answer to the client, so a disconnect and the write
/// it causes are not ordered on the wire: the client sees the socket close and the row may be a
/// moment behind it. A test that reads the row straight after the close would be racing its own
/// server, and a failure would mean nothing.
fn wait_for(database: &ScratchDatabase, condition: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut last;
    loop {
        match execute(database.url(), &assertion(condition)) {
            Ok(()) => return,
            Err(error) => last = error.to_string(),
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{condition} never held: {last}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Log `login` in on Channel 1, once no closing descriptor holds it, and return the connection,
/// the empire it shows, and the character list.
fn select_screen(server: &Server, login: &[u8]) -> (Keyed, u8, Vec<u8>) {
    select_screen_in(server, login, ENGLISH)
}

/// [`select_screen`] for an auth login in `language`.
fn select_screen_in(server: &Server, login: &[u8], language: u8) -> (Keyed, u8, Vec<u8>) {
    select_screen_on(server, 1, login, language)
}

/// [`select_screen_in`] on Channel `channel`.
fn select_screen_on(
    server: &Server,
    channel: u8,
    login: &[u8],
    language: u8,
) -> (Keyed, u8, Vec<u8>) {
    let (_auth, key) = login_key_in(server, login, language);
    let mut keyed = Keyed::channel(server.channel(channel));
    keyed.language = language;
    keyed.channel_number = channel;
    let (empire, list) = login_by_key_when_free(&mut keyed, login, key).expect("logs in");
    (keyed, empire, list)
}

/// `cg.login.empire`, `sys.login.empire`: an account without an empire, or without characters,
/// chooses one; the answer is `GC_EMPIRE` and the character list again, every character moved
/// to the empire's start. An account with an empire and a character is ignored. Empire 0 and
/// empires past 3 close the connection, as does the record on the auth port.
#[test]
fn an_empire_is_chosen_until_the_account_has_one_and_a_character() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    let port = server.channel(1).port();
    create_account(&server, "alice");
    create_account(&server, "bob");

    let (mut alice, empire, _) = select_screen(&server, b"alice");
    assert_eq!(empire, 0);
    // A Divergence: legacy creates a character near (0, 0) for an account without an empire.
    assert_eq!(
        alice.answer(&client_create(0, b"Alpha", 0, 0)),
        [GC_CREATE_FAILURE, 0]
    );
    assert_eq!(alice.answer(&[CG_EMPIRE, 1]), [GC_EMPIRE, 1]);
    let list = alice.read(select_answer_len);
    assert_eq!(list[0], GC_LOGIN_SUCCESS);
    assert!(
        (0..4).all(|slot| slot_is_empty(&list, slot)),
        "no characters"
    );
    check(
        &database,
        "(SELECT empire FROM account WHERE login = 'alice') = 1",
    );
    let created = alice.answer(&client_create(0, b"Alpha", 0, 0));
    assert_eq!(created[..2], [GC_PLAYER_CREATE_SUCCESS, 0]);
    let alpha = listed(&created[1..], 0);
    assert!((459_500..=460_100).contains(&alpha.x), "{}", alpha.x);
    assert!((953_600..=954_200).contains(&alpha.y), "{}", alpha.y);
    assert_eq!((alpha.addr, alpha.port), (PUBLIC_ADDR, port));
    // With an empire and a character, the choice is made.
    alice.unanswered(&[CG_EMPIRE, 3]);
    check(
        &database,
        "(SELECT empire FROM account WHERE login = 'alice') = 1",
    );

    // An account with a character but no empire chooses one, and every character moves to its
    // start: map 21 for empire 2, which no Channel hosts.
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 3, 'Zulu', 1, 470000, \
         950000 FROM account WHERE login = 'bob'",
    );
    let (mut bob, empire, list) = select_screen(&server, b"bob");
    assert_eq!(empire, 0);
    assert_eq!(listed(&list, 3).port, port);
    assert_eq!(bob.answer(&[CG_EMPIRE, 2]), [GC_EMPIRE, 2]);
    let list = bob.read(select_answer_len);
    let zulu = listed(&list, 3);
    assert_eq!(
        (zulu.name.as_slice(), zulu.x, zulu.y, zulu.addr, zulu.port),
        (&b"Zulu"[..], 55_700, 157_900, [0; 4], 0)
    );
    check(
        &database,
        "(SELECT x = 55700 AND y = 157900 FROM player WHERE name = 'Zulu')",
    );

    // Before a Channel login the record is ignored; empire 0 and 4 close the connection.
    let mut stranger = Keyed::channel(server.channel(1));
    stranger.unanswered(&[CG_EMPIRE, 1]);
    stranger.closed_by(&[CG_EMPIRE, 0]);
    Keyed::channel(server.channel(1)).closed_by(&[CG_EMPIRE, 4]);
    let mut auth = auth_client(&server);
    auth.send(&sealed(&[CG_EMPIRE, 1]));
    assert_eq!(auth.expect_closed(), Vec::<u8>::new());
}

/// `cg.login.character_create`, `sys.login.character`, `gc.player_create_success`,
/// `gc.player_create_failure`: a character is created with its job's points near its empire's
/// create start, or refused with the legacy failure type: 0 for a Name the rules refuse (length,
/// letters and digits, banned words, mob names), a shape past 1, a slot past 3, a race past 7,
/// the 30-second cooldown, and a creation before the Channel login; 1 for the login, a taken
/// slot, or a taken Name.
#[test]
fn a_character_is_created_with_its_job_points_or_refused_with_the_legacy_type() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Zulu', 1, 470000, \
         950000 FROM account WHERE login = 'bob'",
    );

    let mut anonymous = Keyed::channel(server.channel(1));
    assert_eq!(
        anonymous.answer(&client_create(0, b"Alpha", 0, 0)),
        [GC_CREATE_FAILURE, 0]
    );

    let (mut alice, _, _) = select_screen(&server, b"alice");
    let refused = [
        (client_create(1, b"A", 0, 0), 0),
        (client_create(1, b"Al_x", 0, 0), 0),
        (client_create(1, b"Al\xe9x", 0, 0), 0),
        (client_create(1, b"aryan", 0, 0), 0),
        (client_create(1, b"Xaryanx", 0, 0), 0),
        (client_create(1, b"blackpony", 0, 0), 0),
        (client_create(1, b"BlackPony", 0, 0), 0),
        (client_create(1, b"Alpha", 0, 2), 0),
        (client_create(4, b"Alpha", 0, 0), 0),
        (client_create(1, b"Alpha", 8, 0), 0),
        (client_create(1, b"Alpha", 256, 0), 0),
        (client_create(9, b"alice", 0, 0), 1),
        (client_create(1, b"Zulu", 0, 0), 1),
        (client_create(1, b"zULU", 0, 0), 1),
    ];
    for (record, failure) in refused {
        assert_eq!(
            alice.answer(&record),
            [GC_CREATE_FAILURE, failure],
            "{record:02x?}"
        );
    }
    let mut unterminated = client_create(1, b"", 0, 0);
    unterminated[2..27].fill(b'a');
    assert_eq!(alice.answer(&unterminated), [GC_CREATE_FAILURE, 0]);

    // Race 7 is a shaman: ST 3, HT 4, DX 3, IQ 6.
    let created = alice.answer(&client_create(2, b"Alpha", 7, 1));
    assert_eq!(created[..2], [GC_PLAYER_CREATE_SUCCESS, 2]);
    let alpha = listed(&created[1..], 0);
    assert_ne!(alpha.id, 0);
    assert_eq!(
        (
            alpha.name.as_slice(),
            alpha.job,
            alpha.level,
            alpha.play_minutes,
            alpha.stats,
            alpha.main_part,
            alpha.change_name,
            alpha.hair_part,
        ),
        (&b"Alpha"[..], 7, 1, 0, [3, 4, 3, 6], 1, 0, 0)
    );
    check(
        &database,
        &format!(
            "(SELECT hp = 860 AND sp = 320 AND stamina = 800 AND part_base = 1 AND slot = 2 \
             FROM player WHERE id = {})",
            alpha.id
        ),
    );
    // Up to 300 from empire 1's create start on each axis. The offset is random; it is (0, 0)
    // once in 361,201 creations.
    check(
        &database,
        &format!(
            "(SELECT abs(x - 459800) <= 300 AND abs(y - 953900) <= 300 \
             AND (x, y) <> (459800, 953900) FROM player WHERE id = {})",
            alpha.id
        ),
    );
    // `s_createTimeByAccountID`: the next creation within 30 seconds is refused.
    assert_eq!(
        alice.answer(&client_create(3, b"Bravo", 0, 0)),
        [GC_CREATE_FAILURE, 0]
    );

    // A refusal does not start the cooldown: bob's taken slot and taken Name, then a creation.
    let (mut bob, _, _) = select_screen(&server, b"bob");
    assert_eq!(
        bob.answer(&client_create(0, b"Yankee", 0, 0)),
        [GC_CREATE_FAILURE, 1]
    );
    assert_eq!(
        bob.answer(&client_create(1, b"ALPHA", 0, 0)),
        [GC_CREATE_FAILURE, 1]
    );
    let created = bob.answer(&client_create(1, b"Yankee", 4, 0));
    assert_eq!(created[..2], [GC_PLAYER_CREATE_SUCCESS, 1]);
    assert_eq!(listed(&created[1..], 0).stats, [6, 4, 3, 3]);
}

/// `cg.login.character_create`: with `block_char_creation` every creation is refused with
/// type 0.
#[test]
fn blocked_character_creation_is_refused() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "block_char_creation = true",
    );
    create_account(&server, "alice");
    sql(&database, "UPDATE account SET empire = 1");
    let (mut alice, _, _) = select_screen(&server, b"alice");
    assert_eq!(
        alice.answer(&client_create(0, b"Alpha", 0, 0)),
        [GC_CREATE_FAILURE, 0]
    );
    check(&database, "NOT EXISTS (SELECT 1 FROM player)");
}

/// `cg.login.character_delete`, `gc.player_delete_success`, `gc.player_delete_wrong_social_id`:
/// a character is deleted when the first seven bytes of the code match the account's delete
/// code and its level is under `player_delete_level_limit` (251 by default), and its row is
/// kept in `player_deleted`. An empty slot, a wrong code, or a level at the limit is refused; a
/// slot past 3 and a delete before a Channel login are ignored.
#[test]
fn a_character_is_deleted_with_the_delete_code() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    sql(
        &database,
        "UPDATE account SET empire = 1, delete_code = 'Ab12345'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, s, n, 0, l, \
         470000, 950000 FROM account, (VALUES (0, 'Alpha', 250), (1, 'Beta', 251)) AS c(s, n, l) \
         WHERE login = 'alice'",
    );

    let mut stranger = Keyed::channel(server.channel(1));
    stranger.unanswered(&client_delete(0, *b"Ab12345\0"));

    let (mut alice, _, list) = select_screen(&server, b"alice");
    assert_eq!(listed(&list, 0).name, b"Alpha");
    alice.unanswered(&client_delete(4, *b"Ab12345\0"));
    for record in [
        client_delete(3, *b"Ab12345\0"),
        client_delete(0, *b"ab12345\0"),
        client_delete(0, *b"Ab1234\0\0"),
        client_delete(1, *b"Ab12345\0"),
    ] {
        assert_eq!(
            alice.answer(&record),
            [GC_PLAYER_DELETE_WRONG_SOCIAL_ID],
            "{record:02x?}"
        );
    }
    // `strncmp(..., 7)`: the eighth byte is not compared.
    assert_eq!(
        alice.answer(&client_delete(0, *b"Ab12345X")),
        [GC_PLAYER_DELETE_SUCCESS, 0]
    );
    assert_eq!(
        alice.answer(&client_delete(0, *b"Ab12345\0")),
        [GC_PLAYER_DELETE_WRONG_SOCIAL_ID]
    );
    check(
        &database,
        "NOT EXISTS (SELECT 1 FROM player WHERE name = 'Alpha') \
         AND (SELECT count(*) FROM player_deleted WHERE name = 'Alpha' \
         AND player ->> 'level' = '250') = 1 \
         AND EXISTS (SELECT 1 FROM player WHERE name = 'Beta')",
    );
}

/// `cg.login.change_name`, `sys.login.change_name`, `gc.change_name`: a character asked to
/// choose a new Name takes one the rules allow and no other character has, and the request is
/// cleared. A Name the rules refuse is create failure type 0; a taken Name is type 1; a
/// character not asked is ignored; an empty slot or a slot past 3 closes the connection.
#[test]
fn a_character_asked_to_rename_takes_a_free_name() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y, change_name) SELECT id, s, n, 0, \
         470000, 950000, r FROM account, (VALUES (0, 'Alpha', true), (1, 'Beta', false)) AS \
         c(s, n, r) WHERE login = 'alice'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Zulu', 1, 470000, \
         950000 FROM account WHERE login = 'bob'",
    );

    let (mut alice, _, list) = select_screen(&server, b"alice");
    let alpha = listed(&list, 0);
    assert_eq!(alpha.change_name, 1);
    alice.unanswered(&client_rename(1, b"Gamma"));
    assert_eq!(
        alice.answer(&client_rename(0, b"blackpony")),
        [GC_CREATE_FAILURE, 0]
    );
    assert_eq!(
        alice.answer(&client_rename(0, b"G")),
        [GC_CREATE_FAILURE, 0]
    );
    assert_eq!(
        alice.answer(&client_rename(0, b"zulu")),
        [GC_CREATE_FAILURE, 1]
    );
    let mut expected = vec![GC_CHANGE_NAME];
    expected.extend_from_slice(&alpha.id.to_le_bytes());
    expected.extend_from_slice(&name_field(b"Gamma"));
    assert_eq!(alice.answer(&client_rename(0, b"Gamma")), expected);
    alice.unanswered(&client_rename(0, b"Delta"));
    check(
        &database,
        &format!(
            "(SELECT name = 'Gamma' AND NOT change_name FROM player WHERE id = {})",
            alpha.id
        ),
    );
    alice.closed_by(&client_rename(2, b"Delta"));

    let (mut alice, _, _) = select_screen(&server, b"alice");
    alice.closed_by(&client_rename(4, b"Delta"));
}

/// `MainCharacterPacket` is the 46-byte empire variant, carrying the VID, the job, the Name,
/// the position, the empire, and the skill group, in source field order.
fn assert_alphas_main_character(main: &[u8], alpha: &Listed) {
    assert_eq!(main[0], GC_MAIN_CHARACTER2_EMPIRE);
    assert_eq!(main.len(), MAIN_CHARACTER_LEN);
    assert_eq!(&main[1..5], &alpha.id.to_le_bytes(), "dwVID");
    assert_eq!(&main[5..7], &3u16.to_le_bytes(), "wJob");
    assert_eq!(&main[7..32], &name_field(b"Alpha"), "szName");
    assert_eq!(&main[32..36], &470_000i32.to_le_bytes(), "x");
    assert_eq!(&main[36..40], &950_000i32.to_le_bytes(), "y");
    assert_eq!(&main[40..44], &0i32.to_le_bytes(), "z");
    assert_eq!(main[44], 1, "bEmpire");
    assert_eq!(main[45], 49, "bSkillGroup");
}

/// `cg.login.character_select`, `sys.login.enter`: `CG_CHARACTER_SELECT` loads the character
/// and answers the loading burst, and `CG_ENTER_GAME` answers the enter-game burst, each in
/// `CInputDB::PlayerLoad` and `CInputLogin::Entergame` order with `SetPhase` in the middle. An
/// empty slot and a slot past the last close; a map the Channel does not host closes after the
/// records written before the map test.
#[test]
fn a_character_is_loaded_and_the_game_is_entered_in_legacy_order() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);

    let (mut keyed, empire, list) = select_screen(&server, b"alice");
    assert_eq!(empire, 1);
    let alpha = listed(&list, 0);
    assert_eq!(alpha.name.as_slice(), b"Alpha");

    // `CG_CHARACTER_SELECT` on slot 0. `PlayerLoad` moves the descriptor to the loading phase
    // first, so `GC_PHASE` with 4 is the first record and the rest are sealed on the key the
    // `LOGIN2` installed.
    keyed.send_record(&client_select(0));
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_LOADING]);
    // `SendEntity` with nobody in view is still written: a size of 3 and no entries.
    let entity = keyed.read_game();
    assert_eq!(
        word_sized(&entity),
        Some(entity.len()),
        "wSize is the record"
    );
    assert_eq!(entity, [GC_ENTITY, 3, 0]);
    assert_alphas_main_character(&keyed.read_game(), &alpha);
    // `PointsPacket` writes the gold record immediately before the points record, because
    // `ENABLE_REMOVE_LIMIT_GOLD` is on.
    let gold = keyed.read_game();
    assert_eq!(gold.len(), GOLD_LEN);
    assert_eq!(gold[0], GC_CHARACTER_GOLD);
    assert_eq!(&gold[1..9], &0u64.to_le_bytes(), "the stored gold");
    let points = keyed.read_game();
    assert_eq!(points.len(), POINTS_LEN);
    assert_eq!(points[0], GC_PLAYER_POINTS);
    keyed.loaded(&points);
    // The 255 eight-byte slots start one byte after the header. Slot 0 is `POINT_NONE`, written
    // as zero rather than the stack garbage `TPacketGCPoints` would carry, and slot 1 is
    // `POINT_LEVEL`, whose value is the character's level.
    assert_eq!(&points[1..9], &0i64.to_le_bytes(), "POINT_NONE");
    assert_eq!(&points[9..17], &154i64.to_le_bytes(), "POINT_LEVEL");
    let levels = keyed.read_game();
    assert_eq!(levels.len(), SKILL_LEVEL_LEN);
    assert_eq!(levels[0], GC_SKILL_LEVEL_NEW);
    // `ItemLoad` comes next. Alpha owns no rows, so there is no `GC_ITEM_SET`, but the load still
    // ends with `PointsPacket` (`G/input_db.cpp:1564`): the same gold and points records again.
    assert_eq!(keyed.read_game(), gold, "the item load's gold record");
    assert_eq!(keyed.read_game(), points, "the item load's points record");
    assert_eq!(keyed.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));

    // `CG_ENTER_GAME`. `Entergame` writes the own pair, `Show` and `SendNPCPosition` the NPCs,
    // the revive-invisible affect, `SetPhase(PHASE_GAME)`, and the time, Channel and events.
    keyed.send_record(&client_enter_game());
    let add = keyed.read_game();
    assert_eq!(
        add.len(),
        CHARACTER_ADD_LEN,
        "no wSize: the client sizes it by the header"
    );
    assert_eq!(add[0], GC_CHARACTER_ADD);
    assert_eq!(word_sized(&add), None);
    assert_eq!(&add[1..5], &alpha.id.to_le_bytes(), "dwVID");
    let additional = keyed.read_game();
    assert_eq!(
        additional.len(),
        CHAR_ADDITIONAL_INFO_LEN,
        "no wSize: the client sizes it by the header"
    );
    assert_eq!(additional[0], GC_CHAR_ADDITIONAL_INFO);
    assert_eq!(word_sized(&additional), None);
    assert_eq!(&additional[1..5], &alpha.id.to_le_bytes(), "dwVID");
    assert_eq!(&additional[5..30], &name_field(b"Alpha"), "szName");
    assert_eq!(
        additional[69], 1,
        "bLanguage from the descriptor: the language the auth login chose"
    );
    let (shown, affect) = read_shown(&mut keyed);
    assert!(shown.list.is_some(), "map 1 lists its NPCs");
    assert_eq!(affect.len(), AFFECT_ADD_LEN);
    assert_eq!(affect[0], GC_AFFECT_ADD);
    assert_eq!(
        &affect[1..5],
        &215u32.to_le_bytes(),
        "AffectType is REVIVE_INVISIBLE"
    );
    assert_eq!(&affect[10..14], &28u32.to_le_bytes(), "AFFECT_FLAG 28");
    assert_eq!(&affect[14..18], &5i32.to_le_bytes(), "five seconds");
    // The phase record is between the affect and the time record, so exactly one is written.
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_GAME]);
    let time = keyed.read_game();
    assert_eq!(time.len(), TIME_LEN);
    assert_eq!(time[0], GC_TIME);
    let channel = keyed.read_game();
    assert_eq!(
        channel,
        [GC_CHANNEL, 1],
        "the Channel the client logged in through"
    );
    let chat = keyed.read_game();
    assert_eq!(chat[0], GC_CHAT);
    assert_eq!(word_sized(&chat), Some(chat.len()), "wSize is the record");
    assert_eq!(chat[3], 5, "CHAT_TYPE_COMMAND");
    assert_eq!(chat.len(), 10 + "letters_event 0".len());
    // `ChatPacket` writes `id = 0` for every line it formats itself.
    assert_eq!(&chat[4..8], &0u32.to_le_bytes(), "id");
    assert_eq!(chat[8], 1, "bEmpire");
    assert_eq!(chat[9], 1, "bCanFormat");
    assert_eq!(&chat[10..], b"letters_event 0");
    assert_eq!(keyed.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));
}

/// Log `login` in, enter the game as slot 0 and answer the connection, the position the loading
/// burst's `GC_MAIN_CHARACTER` carried and the one the enter-game `GC_CHARACTER_ADD` carries.
fn entered(server: &Server, login: &[u8]) -> (Keyed, (i32, i32), (i32, i32)) {
    let (mut keyed, _empire, list) = select_screen(server, login);
    let main = read_loading_burst(&mut keyed, &list, 0).main;
    let word = |at: usize| i32::from_le_bytes([main[at], main[at + 1], main[at + 2], main[at + 3]]);
    let loaded = (word(32), word(36));
    let add = enter_game_burst(&mut keyed);
    (keyed, loaded, inserted_at(&add))
}

/// `sys.login.enter`, `data.map.attr`: `CInputLogin::Entergame` shows a character at the first
/// movable point `GetMovablePosition` finds around its saved position and, when there is none,
/// at its empire's recall position on the map (`G/input_login.cpp:572-585`,
/// `G/sectree_manager.cpp:493-532`, `:789-812`). The loading burst's `GC_MAIN_CHARACTER`, written
/// at select, still carries the saved position; the enter-game `GC_CHARACTER_ADD` carries where
/// the character is shown, and the logout saves that.
///
/// On map 1's `server_attr`, Alpha's (470000, 950000) is a BLOCK cell with a free one 100 east.
/// Every point `GetMovablePosition` tries around (416025, 902425) is blocked, so Zulu of empire 2
/// is recalled to empire 2's town and Echo of empire 1 to empire 1's, and legacy's `sys_err`
/// line is a warning. (468800, 965200) is BANPK, which does not stop a character, so Whiskey
/// stays where it was saved; it is over 3000 from every NPC of map 1, so no warp moves it.
#[test]
fn an_entering_character_stands_on_the_first_movable_point_or_at_its_empires_recall() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    for login in ["alice", "bob", "carol", "dave"] {
        create_account(&server, login);
    }
    add_characters(&database);
    sql(
        &database,
        "UPDATE account SET empire = 2 WHERE login = 'bob'",
    );
    sql(
        &database,
        "UPDATE account SET empire = 1 WHERE login IN ('carol', 'dave')",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, c.n, 1, c.x, c.y \
         FROM account, (VALUES ('bob', 'Zulu', 416025, 902425), ('carol', 'Echo', 416025, \
         902425), ('dave', 'Whiskey', 468800, 965200)) AS c(l, n, x, y) WHERE login = c.l",
    );

    for (login, name, saved, shown) in [
        ("alice", "Alpha", (470_000, 950_000), ALPHA_ENTERS_AT),
        ("bob", "Zulu", (416_025, 902_425), (417_600, 956_100)),
        ("carol", "Echo", (416_025, 902_425), (469_300, 964_200)),
        ("dave", "Whiskey", (468_800, 965_200), (468_800, 965_200)),
    ] {
        let (keyed, loaded, inserted) = entered(&server, login.as_bytes());
        assert_eq!(
            loaded, saved,
            "{name}: the loading burst carries the saved position"
        );
        assert_eq!(
            inserted, shown,
            "{name}: the enter-game insert is where it is shown"
        );
        drop(keyed);
        wait_for(
            &database,
            &format!(
                "(SELECT x FROM player WHERE name = '{name}') = {} AND (SELECT y FROM player \
                 WHERE name = '{name}') = {}",
                shown.0, shown.1
            ),
        );
    }
    server.wait_for("name=Zulu x=416025 y=902425 map=1 to_x=417600 to_y=956100");
    server.wait_for("name=Echo x=416025 y=902425 map=1 to_x=469300 to_y=964200");
    // Whiskey's registration on its map is logged after the placement, so every line up to it
    // is collected, a warning about Whiskey included.
    server.wait_for("map=1 name=Whiskey presence=");
    assert!(
        !server.logged("name=Alpha x=") && !server.logged("name=Whiskey x="),
        "a character with a movable point is not warned about:\n{}",
        server.console()
    );
}

/// The VID the Rewrite gives the first NPC it stands up: `world::npc::FIRST_NPC_VID`. Channel 1
/// is stood up first, and map 1 is its first map.
const FIRST_NPC_VID: u32 = 0x8000_0000;

/// The name `mob_names.txt` in `country/en` gives vnum 20300, the first NPC of map 1's `npc.txt`.
const FIRST_NPC_NAME: &[u8] = b"Invatator Lupta de Corp";

/// Where map 1's warp, vnum 10001, is in `npc.txt`, counting from 0.
const WARP_INDEX: u32 = 37;

/// Map 1's base, the point its `npc.txt` and its mini-map list count from.
const MAP1_BASE: (i32, i32) = (409_600, 896_000);

/// The NPC indices on map 1 whose `npc.txt` entry has a box, half widths 1 and 1, so each stands
/// up at a random point within 100 of its listed centre (`SpawnMobRange`): `:34` and `:42`'s 20005
/// and `:43`'s 20006.
const BOXED_NPCS: [usize; 3] = [27, 33, 34];

/// `SECTREE_SIZE` (`G/sectree.h:8`): the side of one sectree.
const SECTREE_SIZE: i32 = 6400;

/// `SECTREE_MAP::Build`'s walk of the sectrees around one (`G/sectree_manager.cpp:79-120`), as
/// (column, row) steps: the sectree itself, then its eight neighbours.
const BUILD_ORDER: [(i32, i32); 9] = [
    (0, 0),
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, 1),
    (1, -1),
    (-1, -1),
    (1, 1),
];

/// `DISTANCE_APPROX` (`G/utils.h:17-41`): 123/128 of the longer side plus 51/128 of the shorter,
/// in shifts.
fn distance_approx(dx: i32, dy: i32) -> i32 {
    let (dx, dy) = (dx.abs(), dy.abs());
    let (min, max) = if dx < dy { (dx, dy) } else { (dy, dx) };
    ((max << 8) + (max << 3) - (max << 4) - (max << 1) + (min << 7) - (min << 5) + (min << 3)
        - (min << 1))
        >> 8
}

/// `CG_ON_CLICK`: the header and the clicked `dwVID`, `TPacketCGOnClick`.
fn client_click(vid: u32) -> Vec<u8> {
    let mut record = vec![CG_ON_CLICK];
    record.extend_from_slice(&vid.to_le_bytes());
    record
}

/// Alpha's view on entering map 1, given the mini-map list's entries: 15 of the 48 are within
/// `VIEW_RANGE + 500` in the 3x3 sectrees around Alpha, and the warp is not one of them. Each
/// is shown as an NPC with its summary, at its listed point or, for one with a regen box, within
/// 100 of it, in `Build` order and then VID order in each sectree.
fn assert_alphas_npcs_in_view(shown: &Shown, entries: &[&[u8]], warp: usize) {
    let point_of = |entry: &[u8]| {
        let word = |at: usize| i32::from_le_bytes(entry[at..at + 4].try_into().expect("four"));
        (MAP1_BASE.0 + word(26), MAP1_BASE.1 + word(30))
    };
    let tree_of = |(x, y): (i32, i32)| (x / SECTREE_SIZE, y / SECTREE_SIZE);
    let (column, row) = tree_of(ALPHA_ENTERS_AT);
    let walk = |at: (i32, i32)| {
        BUILD_ORDER.iter().position(|&(step_column, step_row)| {
            tree_of(at) == (column + step_column, row + step_row)
        })
    };
    let in_view: Vec<usize> = (0..entries.len())
        .filter(|&index| {
            let at = point_of(entries[index]);
            let gap = distance_approx(at.0 - ALPHA_ENTERS_AT.0, at.1 - ALPHA_ENTERS_AT.1);
            walk(at).is_some() && gap <= 5500
        })
        .collect();
    assert_eq!(in_view.len(), 15, "15 of the 48 are in view");
    assert!(!in_view.contains(&warp), "the warp stands out of view");

    // Each in view is an NPC, each with its summary, at its listed point or, for one with a box,
    // within 100 of it. Each box keeps every point of it in view, but the two 20005s' boxes cross
    // a sectree border, so the walk is checked against where each one stood up.
    assert_eq!(shown.inserts().len(), 15);
    let mut records = shown.records.iter();
    let mut walked = Vec::new();
    for _ in 0..in_view.len() {
        let insert = records.next().expect("an insert");
        assert_eq!(insert[0], GC_CHARACTER_ADD);
        let vid = u32::from_le_bytes(insert[1..5].try_into().expect("four"));
        let index =
            usize::try_from(vid.checked_sub(FIRST_NPC_VID).expect("an NPC")).expect("small");
        assert!(in_view.contains(&index), "insert {index} is in view");
        let at = inserted_at(insert);
        let listed = point_of(entries[index]);
        let slack = if BOXED_NPCS.contains(&index) { 100 } else { 0 };
        assert!(
            (at.0 - listed.0).abs() <= slack && (at.1 - listed.1).abs() <= slack,
            "insert {index} at {at:?}, listed at {listed:?}"
        );
        assert_eq!(insert[21], 1, "bType CHAR_TYPE_NPC");
        let summary = records.next().expect("a summary");
        assert_eq!(summary[0], GC_CHAR_ADDITIONAL_INFO, "summary {index}");
        assert_eq!(&summary[1..5], &vid.to_le_bytes(), "summary {index}");
        walked.push((walk(at).expect("in the 3x3"), index));
    }
    assert!(
        walked.windows(2).all(|pair| pair[0] < pair[1]),
        "Build order, then VID order in each sectree: {walked:?}"
    );
    assert_eq!(records.next(), None, "nobody else is in view");
}

/// The `GC_CHARACTER_ADD` and `GC_CHAR_ADDITIONAL_INFO` `npc.txt`'s first NPC is shown with:
/// at its point, vnum 20300, both speeds 100, no state; then its en name, map 1's empire and
/// `PK_MODE_FREE`.
fn first_npcs_records() -> (Vec<u8>, Vec<u8>) {
    let mut insert = vec![GC_CHARACTER_ADD];
    insert.extend_from_slice(&FIRST_NPC_VID.to_le_bytes());
    insert.extend_from_slice(&0f32.to_le_bytes());
    insert.extend_from_slice(&471_800i32.to_le_bytes());
    insert.extend_from_slice(&951_600i32.to_le_bytes());
    insert.extend_from_slice(&0i32.to_le_bytes());
    insert.push(1);
    insert.extend_from_slice(&20_300u16.to_le_bytes());
    insert.extend_from_slice(&[100, 100, 0]);
    insert.extend_from_slice(&[0; 8]);
    let mut summary = vec![0; CHAR_ADDITIONAL_INFO_LEN];
    summary[0] = GC_CHAR_ADDITIONAL_INFO;
    summary[1..5].copy_from_slice(&FIRST_NPC_VID.to_le_bytes());
    summary[5..30].copy_from_slice(&name_field(FIRST_NPC_NAME));
    summary[42] = 1;
    summary[57] = 2;
    (insert, summary)
}

/// `sys.world.regen`, `sys.world.view`, `cg.game.on_click`: map 1's regen files stand its NPCs up
/// at boot, and entering the map shows each one in view as `EncodeInsertPacket` writes it
/// (`G/char.cpp:1060-1110`), then lists every one of them for the mini-map (`SendNPCPosition`,
/// `G/sectree_manager.cpp:1089-1128`). In view is within `VIEW_RANGE + 500` (5500 at the default
/// range) in the 3x3 sectrees around Alpha (`CEntity::UpdateSectree`, `G/entity_view.cpp:122-236`),
/// walked in `Build` order and, in each sectree, in VID order (V1). The first is `npc.txt`'s first
/// entry, vnum 20300, at its point with direction 1, so rotation 0. Its summary carries its `en`
/// name, its map's empire and `PK_MODE_FREE`, and no level, because only a PC's level is sent.
/// The map's warp stands out of view, and is listed like an NPC.
///
/// A click on an NPC, or on a VID nobody holds, answers nothing and keeps the connection
/// (`G/input_main.cpp:1305-1316`). Only a keeper whose click trigger is the shop's opens a window
/// (`G/char.cpp:6181-6352`), and vnum 20300 is not one; the keeper's scenario is
/// [`a_keeper_opens_its_shop_and_a_buy_and_a_sale_are_stored`]. Vnum 20300 has no quest either;
/// the quest click's scenario is [`a_quest_npc_answers_a_click_with_its_dialog_and_runs_it`].
///
/// Each Channel stands up the maps it hosts, so a character entering map 72 on the Shared
/// Channel is shown map 72's NPCs and none of Channel 1's.
#[test]
fn entering_a_map_shows_its_npcs_and_a_click_keeps_the_connection() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    stamina(&database, "Alpha", 0);
    let (mut alice, alpha, _items) = load_character(&server, b"alice", 0);
    alice.send_record(&client_enter_game());
    let own = alice.read_game();
    assert_eq!(own[0], GC_CHARACTER_ADD);
    assert_eq!(
        &own[1..5],
        &alpha.id.to_le_bytes(),
        "the own insert comes first"
    );
    assert_eq!(alice.read_game()[0], GC_CHAR_ADDITIONAL_INFO);
    assert_eq!(
        alice.read_game(),
        walk_mode_of(&alpha.id.to_le_bytes()),
        "the fixture row's stamina is 0"
    );
    let (shown, affect) = read_shown(&mut alice);
    assert_eq!(affect[0], GC_AFFECT_ADD, "the NPCs come before the affect");

    // The list holds `npc.txt`'s 48 characters in the order they were stood up, so entry `i` is
    // the VID `FIRST_NPC_VID + i`, each at its point less map 1's base.
    let list = shown.list.as_ref().expect("map 1 lists its NPCs");
    let entries: Vec<&[u8]> = list[5..].chunks(NPC_POSITION_LEN).collect();
    assert_eq!(entries.len(), 48);
    let warp = usize::try_from(WARP_INDEX).expect("small");
    assert_alphas_npcs_in_view(&shown, &entries, warp);

    let (insert, summary) = first_npcs_records();
    assert_eq!(shown.records[0], insert, "npc.txt's first NPC");
    assert_eq!(
        shown.records[1], summary,
        "its en name, map 1's empire, PK_MODE_FREE and nothing else"
    );

    assert_eq!(list.len(), 5 + 48 * NPC_POSITION_LEN);
    assert_eq!(&list[1..3], &1637u16.to_le_bytes(), "wSize");
    assert_eq!(&list[3..5], &48u16.to_le_bytes(), "count");
    let mut first = vec![1];
    first.extend_from_slice(&name_field(FIRST_NPC_NAME));
    first.extend_from_slice(&62_200i32.to_le_bytes());
    first.extend_from_slice(&55_600i32.to_le_bytes());
    assert_eq!(
        &list[5..5 + NPC_POSITION_LEN],
        first.as_slice(),
        "the point less map 1's base, 409600 and 896000"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry[1..26] == name_field(b"Fierar")),
        "the smith is on the mini-map"
    );
    assert_eq!(entries[warp][0], 3, "the warp is listed as a warp");

    assert_eq!(alice.read_game(), [GC_PHASE, PHASE_GAME]);
    assert_eq!(alice.read_game()[0], GC_TIME);
    assert_eq!(alice.read_game(), [GC_CHANNEL, 1]);
    assert_eq!(alice.read_game()[0], GC_CHAT);
    assert_eq!(alice.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));

    alice.send_record(&client_click(FIRST_NPC_VID));
    alice.quiet("a click on an NPC");
    alice.send_record(&client_click(FIRST_NPC_VID - 1));
    alice.quiet("a click on a VID nobody holds");
    alice.send_record(&client_click(alpha.id));
    alice.quiet("a click on oneself");
    // The connection still answers: Alpha is level 154, so a shout reaches Alpha.
    alice.send_record(&client_chat(prodomo::chat::CHAT_SHOUT, b"hello"));
    assert_eq!(
        alice.read_game(),
        chat_packet(prodomo::chat::CHAT_SHOUT, 1, b"|Len|l Alpha : hello")
    );

    map_72_is_shown_on_the_shared_channel(&server, &database);
}

/// Charlie enters map 72 on the Shared Channel beside its warp at (99, 33), which is all that is in
/// view, and the mini-map lists map 72's `npc.txt` alone, in its order.
fn map_72_is_shown_on_the_shared_channel(server: &Server, database: &ScratchDatabase) {
    create_account(server, "carol");
    sql(
        database,
        "UPDATE account SET empire = 1 WHERE login = 'carol'",
    );
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Charlie', 2, \
         10000, 1210000 FROM account WHERE login = 'carol'",
    );
    let (mut carol, charlie, _quickslots, _items) =
        load_with_quickslots_on(server, 99, b"carol", 0, ENGLISH);
    carol.send_record(&client_enter_game());
    assert_eq!(carol.read_game()[0], GC_CHARACTER_ADD);
    assert_eq!(carol.read_game()[0], GC_CHAR_ADDITIONAL_INFO);
    assert_eq!(
        carol.read_game(),
        walk_mode_of(&charlie.id.to_le_bytes()),
        "a new row's stamina is 0"
    );
    let (shown, affect) = read_shown(&mut carol);
    assert_eq!(affect[0], GC_AFFECT_ADD);
    let races: Vec<u16> = shown
        .inserts()
        .iter()
        .map(|insert| u16::from_le_bytes([insert[22], insert[23]]))
        .collect();
    assert_eq!(
        races,
        [10_078],
        "map 72's second warp, and nobody from another map"
    );
    assert_eq!(shown.records.len(), 1, "a warp has no summary");
    let list = shown.list.expect("map 72 lists its NPCs");
    assert_eq!(&list[3..5], &9u16.to_le_bytes(), "count");
    let types: Vec<u8> = list[5..]
        .chunks(NPC_POSITION_LEN)
        .map(|entry| entry[0])
        .collect();
    assert_eq!(
        types,
        [3, 3, 1, 1, 1, 1, 1, 1, 1],
        "map 72's npc.txt on the Shared Channel, two warps and seven NPCs"
    );
}

/// `HEADER_GC_CHARACTER_DEL`: the header and the removed `dwVID`, `TPacketGCCharacterDelete`.
const GC_CHARACTER_DEL: u8 = 2;
/// `HEADER_GC_WARP`: `TPacketGCWarp`, the header, `lX`, `lY`, `lAddr` and `wPort`.
const GC_WARP: u8 = 0x41;
const WARP_LEN: usize = 1 + 4 + 4 + 4 + 2;
/// `HEADER_CG_WARP`: `TPacketCGWarp`, the header alone.
const CG_WARP: u8 = 0x41;

/// Where map 1's warp, vnum 10001, stands (`metin2_map_a1/npc.txt:48`'s point plus map 1's
/// base), and where its name, `tinutul_Yayang 4002 8995`, sends a player: a point on map 3,
/// which Channel 1 hosts.
const A1_WARP: (i32, i32) = (450_100, 903_300);
const A1_WARP_TARGET: (i32, i32) = (400_200, 899_500);

/// The x and y of a `GC_CHARACTER_ADD`, which follow the header, `dwVID` and `angle`.
fn inserted_at(insert: &[u8]) -> (i32, i32) {
    let word = |at: usize| {
        i32::from_le_bytes([insert[at], insert[at + 1], insert[at + 2], insert[at + 3]])
    };
    (word(9), word(13))
}

/// The `GC_WARP` a client is sent for `(x, y)` on `port` of the public address.
fn a_warp_record((x, y): (i32, i32), port: u16) -> Vec<u8> {
    let mut record = vec![GC_WARP];
    record.extend_from_slice(&x.to_le_bytes());
    record.extend_from_slice(&y.to_le_bytes());
    record.extend_from_slice(&PUBLIC_ADDR);
    record.extend_from_slice(&port.to_le_bytes());
    record
}

/// `event.char.warp_npc_event`, `gc.warp`, `sys.world.warp`, `cg.game.warp`: a character
/// standing within 300 of map 1's warp is sent through `WarpSet` on the event's next fire
/// (`FuncCheckWarp`, `G/char.cpp:7893-8015`; `WarpSet`, `:6694-6792`). Its own client is sent
/// the `GC_CHARACTER_DEL` of its own VID (`EncodeRemovePacket(this)`, `:6754`) and then the
/// `GC_WARP` naming the target and the address and port that host map 3, and the row holds the
/// target before either record is sent.
///
/// The old connection is left for the client to close and reads nothing more. The login was
/// released at the departure, so the key the client holds logs in again at once, while the old
/// connection is still open; the character list shows the target, and the character enters
/// map 3 there. A `CG_WARP` then finds no warp
/// pending and is consumed (`WarpEnd`, `:6800-6801`).
///
/// The old descriptor acts no more: a shout it is sent reaches nobody, and its close writes
/// nothing, so the position the new descriptor saves is the one the row keeps.
///
/// The warp fires within a few Pulses of the server starting. Legacy's `IsHack` would refuse
/// every player for the first ten seconds of uptime (a Defect the Rewrite does not reproduce).
#[test]
fn a_warp_npc_sends_its_neighbour_away_and_the_client_comes_back_at_the_target() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    // 100 north of the warp: `DISTANCE_APPROX(0, 100)` is 96, within 300.
    sql(
        &database,
        "UPDATE player SET x = 450100, y = 903400 WHERE name = 'Alpha'",
    );

    let (_auth, key) = login_key(&server, b"alice");
    let mut alice = Keyed::channel(server.channel(1));
    let (_empire, list) = login_by_key(&mut alice, b"alice", key, CLIENT_KEY).expect("logs in");
    let (alpha, _quickslots, _items) = load_selected(&mut alice, &list, 0);
    let own = enter_game_records(&mut alice);
    assert_eq!(
        inserted_at(&own),
        (450_100, 903_400),
        "Alpha enters beside the warp"
    );

    // Within one fire, the own character's removal and then the warp. The row was written first.
    let mut removed = vec![GC_CHARACTER_DEL];
    removed.extend_from_slice(&alpha.id.to_le_bytes());
    assert_eq!(alice.read_game(), removed, "EncodeRemovePacket(this)");
    let port = server.channel(1).port();
    assert_eq!(alice.read_game(), a_warp_record(A1_WARP_TARGET, port));
    check(
        &database,
        "(SELECT x FROM player WHERE name = 'Alpha') = 400200 AND (SELECT y FROM player WHERE \
         name = 'Alpha') = 899500",
    );
    // The departed descriptor drops what it reads, and the next fires find nobody to send.
    alice.send_record(&client_chat(CHAT_TALKING, b"hello"));
    alice.quiet("a departed descriptor drops a talking line");
    alice.quiet("and the warp does not fire again");

    // The key logs in again while the old descriptor is still open.
    let mut again = Keyed::channel(server.channel(1));
    let (_empire, list) =
        login_by_key(&mut again, b"alice", key, CLIENT_KEY).expect("the key logs in again");
    let listed_alpha = listed(&list, 0);
    assert_eq!(
        (
            listed_alpha.x,
            listed_alpha.y,
            listed_alpha.addr,
            listed_alpha.port
        ),
        (A1_WARP_TARGET.0, A1_WARP_TARGET.1, PUBLIC_ADDR, port)
    );
    let _loaded = load_selected(&mut again, &list, 0);
    let own = enter_game_burst(&mut again);
    assert_eq!(
        inserted_at(&own),
        A1_WARP_TARGET,
        "Alpha enters map 3 at the target"
    );
    // A shout would reach every client on every Channel, the new connection among them.
    alice.send_record(&client_chat(prodomo::chat::CHAT_SHOUT, b"gone"));
    again.quiet("the departed descriptor drops a shout");

    // Nothing is pending on the new descriptor, so `CG_WARP` is consumed and the connection kept.
    again.send_record(&[CG_WARP]);
    again.quiet("WarpEnd with no warp pending");
    again.send_record(&client_chat(prodomo::chat::CHAT_SHOUT, b"hello"));
    assert_eq!(
        again.read_game(),
        chat_packet(prodomo::chat::CHAT_SHOUT, 1, b"|Len|l Alpha : hello"),
        "the connection still answers"
    );
    alice.quiet("the old descriptor left the client set and hears no shout");

    // The new descriptor saves where Alpha steps to at its close; the old one's close, after
    // it, writes nothing over that.
    let old = alice.client.local_addr();
    again.send_record(&client_move(3, 7, 40, 400_300, 899_500, 0x5eed));
    again.quiet("PacketAround excludes the mover");
    drop(again);
    wait_for(
        &database,
        "(SELECT x FROM player WHERE name = 'Alpha') = 400300",
    );
    drop(alice);
    server.wait_for(&format!("Client connection closed addr={old}"));
    std::thread::sleep(QUIET_WINDOW);
    check(
        &database,
        "(SELECT x FROM player WHERE name = 'Alpha') = 400300",
    );
}

/// `event.char.warp_npc_event`: map 3's warp 10067 is named `TemnitaMaimute 7752 4477`, and
/// (775200, 447700) is on no map, so `WarpSet` finds no location and returns before any record
/// (`G/char.cpp:6703-6707`), fire after fire. A character standing on it stays, and its
/// connection answers.
#[test]
fn a_warp_to_no_map_sends_nothing_and_the_character_stays() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    sql(
        &database,
        "UPDATE player SET x = 407400, y = 875700 WHERE name = 'Alpha'",
    );
    let (mut alice, _alpha, _items) = load_character(&server, b"alice", 0);
    alice.send_record(&client_enter_game());
    assert_eq!(alice.read_game()[0], GC_CHARACTER_ADD);
    assert_eq!(alice.read_game()[0], GC_CHAR_ADDITIONAL_INFO);
    let (shown, affect) = read_shown(&mut alice);
    assert_eq!(affect[0], GC_AFFECT_ADD);
    assert!(
        shown.inserts().iter().any(|insert| {
            insert[21] == 3
                && insert[22..24] == 10_067u16.to_le_bytes()
                && inserted_at(insert) == (407_400, 875_700)
        }),
        "the warp stands where Alpha stands"
    );
    assert_eq!(alice.read_game(), [GC_PHASE, PHASE_GAME]);
    assert_eq!(alice.read_game()[0], GC_TIME);
    assert_eq!(alice.read_game(), [GC_CHANNEL, 1]);
    assert_eq!(alice.read_game()[0], GC_CHAT);

    // Four quiet windows are 1.2 seconds, at least two fires.
    for _ in 0..4 {
        alice.quiet("a warp to no map");
    }
    alice.send_record(&client_chat(prodomo::chat::CHAT_SHOUT, b"hello"));
    assert_eq!(
        alice.read_game(),
        chat_packet(prodomo::chat::CHAT_SHOUT, 1, b"|Len|l Alpha : hello"),
        "the connection still answers"
    );
}

/// `WarpSet`'s departure writes the row with the destination before `GC_WARP` leaves, so a
/// client is never sent to a row that does not name it. When the write fails, the descriptor
/// closes without the character's removal and without `GC_WARP`; the close's own save still
/// holds the destination and fails the same way, and the row keeps the position Alpha was
/// loaded at. A trigger that refuses any write of the target's x stands in for a store that
/// refuses the write.
#[test]
fn a_warp_whose_save_fails_closes_without_sending_the_client_away() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    sql(
        &database,
        "UPDATE player SET x = 450100, y = 903400 WHERE name = 'Alpha'",
    );
    sql(
        &database,
        "CREATE FUNCTION refuse_the_target() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE \
         EXCEPTION 'the store refuses the write'; END $$",
    );
    sql(
        &database,
        "CREATE TRIGGER refuse_the_target BEFORE UPDATE ON player FOR EACH ROW WHEN (NEW.x = \
         400200) EXECUTE FUNCTION refuse_the_target()",
    );

    let (mut alice, _alpha, _items) = load_character(&server, b"alice", 0);
    let own = enter_game_records(&mut alice);
    assert_eq!(
        inserted_at(&own),
        (450_100, 903_400),
        "Alpha enters beside the warp"
    );
    assert_eq!(
        alice.client.expect_closed(),
        Vec::<u8>::new(),
        "no GC_CHARACTER_DEL and no GC_WARP"
    );
    server.wait_for("The warp could not save the character; closing without GC_WARP");
    server.wait_for("Character disconnected; wrote the row");
    assert!(
        server.logged("written=false"),
        "the close's save fails too:\n{}",
        server.console()
    );
    check(
        &database,
        "(SELECT x FROM player WHERE name = 'Alpha') = 450100 AND (SELECT y FROM player WHERE \
         name = 'Alpha') = 903400",
    );
}

/// Where the goto NPC of [`a_goto_npc_shows_its_neighbour_at_its_target_on_the_same_map`] shows
/// a character: its name's numbers times 100 past map 1's base, 409600 and 896000.
const GOTO_TARGET: (i32, i32) = (444_100, 932_100);

/// Where map 1's 9009 of `metin2_map_a1/npc.txt:25` stands, its NPC index 18: (325, 405) times
/// 100 past map 1's base.
const IDX18_AT: (i32, i32) = (442_100, 936_500);

/// `event.char.warp_npc_event` for a goto NPC, which the owner's data spawns on no map it hosts
/// (ledger 228): the scenario spawns 10601 of `metin2_map_monkey_dungeon2`, `CHAR_TYPE_GOTO`
/// and named `. 345 361`, on map 1 at (470000, 950100), within reach of [`ALPHA_ENTERS_AT`], with
/// a line the owner's `npc.txt` does not have.
///
/// On the event's next fire Alpha is shown on its own map at the target (`FuncCheckWarp`,
/// `G/char.cpp:7971-7972`), which is in another sectree, so its own client is sent its insert
/// pair at `z` `INT_MAX` and its walk mode, then the one character in view there, the 9009 of
/// [`IDX18_AT`] (`Show`, `:1847-1917`), and no reconnect. Alpha then stands out of the NPC's
/// reach, so the next fires send nothing, and the logout saves the target.
#[test]
fn a_goto_npc_shows_its_neighbour_at_its_target_on_the_same_map() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_with_npcs(
        binary(),
        database.url(),
        "metin2_map_a1",
        "m\t604\t541\t0\t0\t0\t1\t1s\t100\t1\t10601\n",
    );
    create_account(&server, "alice");
    add_characters(&database);
    stamina(&database, "Alpha", 0);

    let (mut alice, alpha, _items) = load_character(&server, b"alice", 0);
    let own = enter_game_records(&mut alice);
    assert_eq!(inserted_at(&own), ALPHA_ENTERS_AT);
    let shown = alice.read_game();
    assert_eq!(shown[0], GC_CHARACTER_ADD);
    assert_eq!(shown[1..5], alpha.id.to_le_bytes(), "Alpha itself");
    assert_eq!(inserted_at(&shown), GOTO_TARGET);
    assert_eq!(&shown[17..21], &i32::MAX.to_le_bytes(), "Show's z, SHOW_Z");
    assert_eq!(alice.read_game()[0], GC_CHAR_ADDITIONAL_INFO);
    assert_eq!(
        alice.read_game(),
        walk_mode_of(&alpha.id.to_le_bytes()),
        "the fixture row's stamina is 0"
    );
    let neighbour = alice.read_game();
    assert_eq!(neighbour[0], GC_CHARACTER_ADD);
    assert_eq!(neighbour[21], 1, "CHAR_TYPE_NPC");
    assert_eq!(&neighbour[22..24], &9009u16.to_le_bytes(), "wRaceNum");
    assert_eq!(inserted_at(&neighbour), IDX18_AT, "5025 from the target");
    let summary = alice.read_game();
    assert_eq!(summary[0], GC_CHAR_ADDITIONAL_INFO);
    assert_eq!(summary[1..5], neighbour[1..5], "the NPC's summary");
    // Two quiet windows are 0.6 seconds, at least one fire.
    alice.quiet("Alpha stands out of the goto's reach");
    alice.quiet("and the next fire finds nobody");

    drop(alice);
    wait_for(
        &database,
        "(SELECT x FROM player WHERE name = 'Alpha') = 444100 AND (SELECT y FROM player WHERE \
         name = 'Alpha') = 932100",
    );
}

/// `IsHack` (`G/char.cpp:8226-8293`), which `FuncCheckWarp` runs before `WarpSet`
/// (`:7961-7962`): a player that comes within 300 of a warp with a trade open is told `[LS;851]`
/// on every fire and stays. Once the trade is cancelled, the trade it started is within the
/// portal limit, and the line is `[LS;852;10]`. The trader who stays away is told nothing.
#[test]
fn a_trader_beside_a_warp_is_told_why_and_stays() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(
        &database,
        "UPDATE account SET empire = 2 WHERE login = 'bob'",
    );
    // Alpha is 500 north of the warp and Yankee 800: `DISTANCE_APPROX` gives 480 and 768.
    sql(
        &database,
        "UPDATE player SET x = 450100, y = 903800 WHERE name = 'Alpha'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         450100, 904100 FROM account WHERE login = 'bob'",
    );
    let (mut alice, alpha) = enter_world(&server, b"alice", 0);
    let (mut bob, yankee) = enter_world(&server, b"bob", 0);
    alice.sees_arrive(yankee.id, true);
    alice.quiet("Alpha is out of the warp's reach");

    alice.send_record(&client_exchange(0, u64::from(yankee.id), 0, 0));
    assert_eq!(
        bob.read_game(),
        exchange_record(0, false, u64::from(alpha.id), NO_CELL)
    );
    assert_eq!(
        alice.read_game(),
        exchange_record(0, false, u64::from(yankee.id), NO_CELL)
    );

    // Alpha steps to 100 north of the warp. `FUNC_COMBO` steps without a duration.
    alice.send_record(&client_move(3, 7, 40, A1_WARP.0, A1_WARP.1 + 100, 0x5eed));
    assert_eq!(bob.read_game()[0], GC_MOVE, "the neighbour sees the step");
    let window = chat_packet(prodomo::chat::CHAT_INFO, 1, b"[LS;851]");
    assert_eq!(alice.read_game(), window, "a fire refuses the open trade");
    assert_eq!(alice.read_game(), window, "and so does the next");

    // The cancel may cross a fire, which is told the window line once more.
    alice.send_record(&client_exchange(5, 0, 0, 0));
    let cancelled = exchange_record(5, false, 0, NO_CELL);
    let mut next = alice.read_game();
    while next == window {
        next = alice.read_game();
    }
    assert_eq!(next, cancelled);
    assert_eq!(bob.read_game(), cancelled);
    assert_eq!(
        alice.read_game(),
        chat_packet(prodomo::chat::CHAT_INFO, 1, b"[LS;852;10]"),
        "the trade Alpha started is within the portal limit"
    );
    bob.quiet("Yankee stands out of the warp's reach");
}

/// `HEADER_GC_SCRIPT`: `TPacketGCScript`, a `WORD size` covering the whole record, the `skin`
/// byte and a `WORD src_size`, then the script with no terminator (`G/packet.h`).
const GC_SCRIPT: u8 = 45;
/// `HEADER_CG_SCRIPT_ANSWER`: the header and the answer byte, `TPacketCGScriptAnswer`.
const CG_SCRIPT_ANSWER: u8 = 0x1d;
/// `HEADER_CG_QUEST_INPUT_STRING`: the header and `msg[65]`, `TPacketCGQuestInputString`.
const CG_QUEST_INPUT_STRING: u8 = 0x1e;

/// Map 1's OX manager in `npc.txt`, whose `en` name is Uriel. `ox_event.quest`'s
/// `oxevent_manager` answers its chat.
const OX_MANAGER_VNUM: u16 = 20_011;

/// `say_title(mob_name(npc.get_race()) .. ":")` for the OX manager: `color256(255, 230, 186)`,
/// the name, `color256(196, 196, 196)` and `say`'s `[ENTER]`, each ratio printed as Lua 5.1's
/// `%.14g` prints it (`questlib.lua`).
const OX_TITLE: &[u8] = b"[COLOR r;1|g;0.90196078431373|b;0.72941176470588]Uriel:\
    [COLOR r;0.76862745098039|g;0.76862745098039|b;0.76862745098039][ENTER]";

/// `translate.oxevent._20_say`, the entry's first page.
const OX_FIRST_PAGE: &[u8] = b"Hey - you there! Yes, you - you look quite[ENTER]intelligent. \
    There is a Contest called the OX[ENTER]Contest. You can test your knowledge there. If[ENTER]\
    you win, you'll get a nice reward. ";

/// `translate.oxevent._30_say`, the page `oxevent_status == 0` shows.
const OX_LAST_PAGE: &[u8] = b"I can allow you to participate in the Contest[ENTER]when it starts, \
    but you can also just watch.[ENTER]The start time hasn't been determined yet. I'm[ENTER]\
    going to inform you once it's time, so be ready! ";

/// A `GC_SCRIPT` as `CQuestManager::SendScript` sends it (`G/questmanager.cpp`): the size of the
/// whole record, the skin, and the script's own length.
fn server_script(skin: u8, script: &[u8]) -> Vec<u8> {
    let src_size = u16::try_from(script.len()).expect("short");
    let mut record = vec![GC_SCRIPT];
    record.extend_from_slice(&(src_size + 6).to_le_bytes());
    record.push(skin);
    record.extend_from_slice(&src_size.to_le_bytes());
    record.extend_from_slice(script);
    record
}

/// `CG_SCRIPT_ANSWER`: the header and the answer.
fn client_script_answer(answer: u8) -> Vec<u8> {
    vec![CG_SCRIPT_ANSWER, answer]
}

/// `CG_QUEST_INPUT_STRING`: the header and `msg`, NUL-padded to 65 bytes.
fn client_quest_input(msg: &[u8]) -> Vec<u8> {
    let mut record = vec![CG_QUEST_INPUT_STRING];
    record.extend_from_slice(msg);
    record.resize(1 + 65, 0);
    record
}

/// `sys.quest.runtime`, `cg.game.on_click`, `cg.game.script_answer`,
/// `cg.game.quest_input_string`, `gc.script`: a click on an NPC runs its quests before its click
/// trigger (`G/char.cpp:6333-6338`, `CQuestManager::Click`, `G/questmanager.cpp:918-990`). Map
/// 1's OX manager, vnum 20011, has one chat quest, so the click answers with the chat menu: the
/// quest's entry and the `Inchide` every menu ends with (`G/questnpc.cpp:940-955`), in skin 1.
///
/// While the menu is open a second click is no quest's, and vnum 20011 is no keeper, so it
/// answers nothing; neither does a text with no `input()` waiting for it, nor a `wait`'s answer
/// (above 250, `G/input_main.cpp:2200-2217`) while a menu waits. Picking the entry runs it: the
/// title and the first page end in `[NEXT]`; continuing shows the last page, which ends in
/// `[DONE]` because the script ends. The next click offers the menu again, and `Inchide` closes
/// it with a `[DONE]` in skin 0, `QUEST_SKIN_NOWINDOW`.
#[test]
fn a_quest_npc_answers_a_click_with_its_dialog_and_runs_it() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    // The OX manager stands within 100 of (480900, 956500), `npc.txt:44`'s point past map 1's
    // base; Alpha enters 1000 from it, in its view.
    sql(
        &database,
        "UPDATE player SET x = 480900, y = 955500 WHERE name = 'Alpha'",
    );
    let (mut alice, _alpha, _items) = load_character(&server, b"alice", 0);
    alice.send_record(&client_enter_game());
    assert_eq!(alice.read_game()[0], GC_CHARACTER_ADD);
    assert_eq!(alice.read_game()[0], GC_CHAR_ADDITIONAL_INFO);
    let (shown, affect) = read_shown(&mut alice);
    assert_eq!(affect[0], GC_AFFECT_ADD);
    let managers: Vec<u32> = shown
        .inserts()
        .iter()
        .filter(|insert| insert[22..24] == OX_MANAGER_VNUM.to_le_bytes())
        .map(|insert| u32::from_le_bytes(insert[1..5].try_into().expect("four bytes")))
        .collect();
    assert_eq!(managers.len(), 1, "map 1 has one OX manager");
    let ox = managers[0];
    assert_eq!(alice.read_game(), [GC_PHASE, PHASE_GAME]);
    assert_eq!(alice.read_game()[0], GC_TIME);
    assert_eq!(alice.read_game(), [GC_CHANNEL, 1]);
    assert_eq!(alice.read_game()[0], GC_CHAT);
    alice.quiet("the enter-game burst is over");

    alice.send_record(&client_click(ox));
    assert_eq!(
        alice.read_game(),
        server_script(1, b"[QUESTION 1;OX Contest |2;Inchide]")
    );
    alice.quiet("the menu is the click's one answer");
    alice.unanswered(&client_click(ox));
    alice.unanswered(&client_quest_input(b"abc"));
    alice.unanswered(&client_script_answer(254));

    alice.send_record(&client_script_answer(0));
    let first = [OX_TITLE, OX_FIRST_PAGE, b"[ENTER][NEXT]"].concat();
    assert_eq!(alice.read_game(), server_script(1, &first));
    alice.quiet("one page at a time");
    alice.unanswered(&client_script_answer(0));
    alice.send_record(&client_script_answer(254));
    let last = [OX_TITLE, OX_LAST_PAGE, b"[ENTER][DONE]"].concat();
    assert_eq!(alice.read_game(), server_script(1, &last));
    alice.quiet("the script ended");
    alice.unanswered(&client_script_answer(254));

    alice.send_record(&client_click(ox));
    assert_eq!(
        alice.read_game(),
        server_script(1, b"[QUESTION 1;OX Contest |2;Inchide]"),
        "a finished script leaves the NPC's menu to the next click"
    );
    alice.send_record(&client_script_answer(1));
    assert_eq!(alice.read_game(), server_script(0, b"[DONE]"));
    alice.quiet("Inchide ends the chat");
}

/// `npc.txt`'s keeper on map 3, vnum 20042, whose click trigger is the shop's. Its shop, 9,
/// sells vnums 11901, 11903 and 50201, one of each.
const KEEPER_VNUM: u16 = 20_042;

/// `HEADER_GC_SHOP`: a `WORD wSize` covering the whole record, then the subheader.
const GC_SHOP: u8 = 38;
/// `TPacketGCShopStart`'s items start after the header, the size, the subheader and the
/// keeper's VID, 73 bytes each (`G/packet.h`).
const SHOP_ITEMS_AT: usize = 1 + 2 + 1 + 4;
const SHOP_ITEM_LEN: usize = 73;
/// `HEADER_GC_CHARACTER_GOLD_CHANGE`, the first byte of its four-byte `int` header.
const GC_GOLD_CHANGE: u8 = 225;
/// `TPacketGCGoldChange`: an `int` header, `dwVID`, `long long amount`, `unsigned long long
/// value`.
const GOLD_CHANGE_LEN: usize = 4 + 4 + 8 + 8;

/// A `GC_SHOP` that carries its subheader alone.
fn shop_answer(subheader: u8) -> Vec<u8> {
    vec![GC_SHOP, 4, 0, subheader]
}

/// A `GC_CHARACTER_GOLD_CHANGE` in `TPacketGCGoldChange` field order.
fn gold_change(vid: u32, amount: i64, value: u64) -> Vec<u8> {
    let mut record = i32::from(GC_GOLD_CHANGE).to_le_bytes().to_vec();
    record.extend_from_slice(&vid.to_le_bytes());
    record.extend_from_slice(&amount.to_le_bytes());
    record.extend_from_slice(&value.to_le_bytes());
    assert_eq!(record.len(), GOLD_CHANGE_LEN);
    record
}

/// The vnum, price and count of slot `slot` of a `GC_SHOP` window.
fn shop_slot(start: &[u8], slot: usize) -> (u32, u64, u16) {
    let at = SHOP_ITEMS_AT + slot * SHOP_ITEM_LEN;
    (
        u32::from_le_bytes(start[at..at + 4].try_into().expect("four bytes")),
        u64::from_le_bytes(start[at + 4..at + 12].try_into().expect("eight bytes")),
        u16::from_le_bytes([start[at + 12], start[at + 13]]),
    )
}

/// `cg.game.on_click`, `cg.game.shop`, `sys.npc.shop`, `table.shop`, `gc.shop`,
/// `gc.character_gold_change`, and the four `CInputMain::Shop` arms: a click on a shop keeper
/// opens its shop's window (`CShopManager::StartShopping`, `G/shop_manager.cpp:115-161`), a buy
/// takes the price and gives the item (`CShop::Buy`, `G/shop.cpp:411-690`), a sale takes the item
/// and pays a fifth of its shop price less the 3% tax (`CShopManager::Sell`,
/// `G/shop_manager.cpp:456-594`), and closing the window answers `SHOP_SUBHEADER_GC_END`.
///
/// Every buy and sale stores the item and the gold in one transaction before the records are
/// sent, so the store is checked as soon as they arrive; the save that follows the disconnect
/// leaves the gold alone, because only a Transfer writes it (ledger 225).
#[test]
fn a_keeper_opens_its_shop_and_a_buy_and_a_sale_are_stored() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    // Given: Alpha on map 3, its own empire's, a few steps from the keeper at (325200, 879200),
    // with five million gold.
    sql(
        &database,
        "UPDATE player SET x = 325500, y = 879400, gold = 5000000 WHERE name = 'Alpha'",
    );
    let (mut alice, _alpha, items) = load_character(&server, b"alice", 0);
    assert_eq!(items, Vec::<Vec<u8>>::new(), "Alpha carries nothing");
    alice.send_record(&client_enter_game());
    let own = alice.read_game();
    assert_eq!(own[0], GC_CHARACTER_ADD);
    let vid = u32::from_le_bytes(own[1..5].try_into().expect("four bytes"));
    assert_eq!(alice.read_game()[0], GC_CHAR_ADDITIONAL_INFO);
    let (shown, affect) = read_shown(&mut alice);
    assert_eq!(affect[0], GC_AFFECT_ADD);
    let keepers: Vec<u32> = shown
        .inserts()
        .iter()
        .filter(|insert| insert[22..24] == KEEPER_VNUM.to_le_bytes())
        .map(|insert| u32::from_le_bytes(insert[1..5].try_into().expect("four bytes")))
        .collect();
    let [keeper] = keepers[..] else {
        panic!("map 3 shows one keeper 20042, not {keepers:?}");
    };
    assert_eq!(alice.read_game(), [GC_PHASE, PHASE_GAME]);
    assert_eq!(alice.read_game()[0], GC_TIME);
    assert_eq!(alice.read_game(), [GC_CHANNEL, 1]);
    assert_eq!(alice.read_game()[0], GC_CHAT);
    alice.quiet("the burst is over");

    // When: nobody's window is open, a buy and a close are ignored.
    alice.unanswered(&[CG_SHOP, 1, 0, 2]);
    alice.unanswered(&[CG_SHOP, 0]);

    // When: Alpha clicks the keeper. Then: its window opens, owned by the keeper, with shop 9's
    // three items at their `item_proto` prices, untripled for its own empire, and nothing else.
    alice.send_record(&client_click(keeper));
    let start = alice.read_game();
    assert_eq!(start.len(), 2928);
    assert_eq!(&start[..4], &[GC_SHOP, 0x70, 0x0b, 0], "wSize and START");
    assert_eq!(&start[4..8], &keeper.to_le_bytes());
    assert_eq!(shop_slot(&start, 0), (11_901, 2_000_000, 1));
    assert_eq!(shop_slot(&start, 1), (11_903, 2_500_000, 1));
    assert_eq!(shop_slot(&start, 2), (50_201, 100_000, 1));
    for slot in 3..40 {
        assert_eq!(shop_slot(&start, slot), (0, 0, 0), "slot {slot} is empty");
    }
    // And: a second click on the keeper whose window is open is ignored.
    alice.unanswered(&client_click(keeper));

    // When: slot 2 is bought. Then: the gold goes, then the item lands in the first free cell,
    // and the store holds both before either record arrives.
    alice.send_record(&[CG_SHOP, 1, 0, 2]);
    assert_eq!(alice.read_game(), gold_change(vid, 0, 4_900_000));
    let inventory = common::item_slots::EWindows::Inventory as u8;
    assert_eq!(set_fields(&alice.read_game()), (inventory, 0, 50_201, 1));
    check(
        &database,
        "(SELECT gold FROM player WHERE name = 'Alpha') = 4900000 AND EXISTS (SELECT 1 FROM item \
         WHERE id = 100000000 AND vnum = 50201 AND window_type = 1 AND pos = 0 AND count = 1)",
    );

    // When: an empty slot or a slot past the window is bought. Then: the empty slot costs
    // nothing, which `CShop::Buy` answers as too little gold, and the slot past the window is
    // an invalid position. Nothing is stored.
    alice.send_record(&[CG_SHOP, 1, 0, 3]);
    assert_eq!(alice.read_game(), shop_answer(5), "NOT_ENOUGH_MONEY");
    alice.send_record(&[CG_SHOP, 1, 0, 40]);
    assert_eq!(alice.read_game(), shop_answer(8), "INVALID_POS");
    // And: an unknown subheader is logged and ignored.
    alice.unanswered(&[CG_SHOP, 9]);

    // When: the whole stack at cell 0 is sold. Then: the tax line, the cleared cell, and the
    // payment: 100000 / 5 = 20000, less 3%, is 19400.
    alice.send_record(&[CG_SHOP, 2, 0]);
    assert_eq!(alice.read_game(), an_info_line(b"[LS;881;3]"));
    assert_eq!(alice.read_game(), a_clear_record(0));
    assert_eq!(alice.read_game(), gold_change(vid, 19_400, 4_919_400));
    check(
        &database,
        "(SELECT gold FROM player WHERE name = 'Alpha') = 4919400 AND NOT EXISTS (SELECT 1 FROM \
         item WHERE id = 100000000)",
    );
    // And: selling the now-empty cell answers nothing.
    alice.unanswered(&[CG_SHOP, 2, 0]);

    // When: slot 2 is bought again, under the next id, and sold with `SELL2`, which carries a
    // count after an alignment byte the server never reads. Then: the same answers.
    alice.send_record(&[CG_SHOP, 1, 0, 2]);
    assert_eq!(alice.read_game(), gold_change(vid, 0, 4_819_400));
    assert_eq!(set_fields(&alice.read_game()), (inventory, 0, 50_201, 1));
    check(
        &database,
        "EXISTS (SELECT 1 FROM item WHERE id = 100000001 AND pos = 0)",
    );
    alice.send_record(&[CG_SHOP, 3, 0, 0xcc, 1, 0]);
    assert_eq!(alice.read_game(), an_info_line(b"[LS;881;3]"));
    assert_eq!(alice.read_game(), a_clear_record(0));
    assert_eq!(alice.read_game(), gold_change(vid, 19_400, 4_838_800));
    check(
        &database,
        "(SELECT gold FROM player WHERE name = 'Alpha') = 4838800 AND NOT EXISTS (SELECT 1 FROM \
         item)",
    );

    // When: the window is closed. Then: `SHOP_SUBHEADER_GC_END`, and a buy after it is
    // ignored.
    alice.send_record(&[CG_SHOP, 0]);
    assert_eq!(alice.read_game(), shop_answer(1), "END");
    alice.unanswered(&[CG_SHOP, 1, 0, 2]);

    // When: Alpha disconnects. Then: the gold the sale left is still stored after the save,
    // and a relog finds no item.
    drop(alice);
    server.wait_for("Character disconnected; wrote the row");
    check(
        &database,
        "(SELECT gold FROM player WHERE name = 'Alpha') = 4838800",
    );
    let (_alice, _alpha, items) = load_character(&server, b"alice", 0);
    assert_eq!(items, Vec::<Vec<u8>>::new(), "the sold item stays sold");
    assert_eq!(items_of(&database, "Alpha"), "none");
}

/// Point slot `slot` of a `GC_CHARACTER_POINTS` record: the slots start one byte after the
/// header, eight bytes each.
fn point_slot(points: &[u8], slot: usize) -> i64 {
    let at = 1 + 8 * slot;
    i64::from_le_bytes(points[at..at + 8].try_into().expect("eight bytes"))
}

/// `points` with point slot `slot` set to `value`.
fn with_point_slot(points: &[u8], slot: usize, value: i64) -> Vec<u8> {
    let mut changed = points.to_vec();
    let at = 1 + 8 * slot;
    changed[at..at + 8].copy_from_slice(&value.to_le_bytes());
    changed
}

/// A `GC_CHARACTER_POINT_CHANGE` in `TPacketGCPointChange` field order.
fn point_change(vid: u32, kind: u8, amount: i64, value: i64) -> Vec<u8> {
    let mut record = i32::from(GC_POINT_CHANGE).to_le_bytes().to_vec();
    record.extend_from_slice(&vid.to_le_bytes());
    record.push(kind);
    record.extend_from_slice(&amount.to_le_bytes());
    record.extend_from_slice(&value.to_le_bytes());
    assert_eq!(record.len(), POINT_CHANGE_LEN);
    record
}

/// `sys.char.points`, `gc.player_point_change`: the loading burst's points record carries the
/// points `SetPlayerProto` computes for the stored row, and the item load's
/// `CheckMaximumPoints` clamps the stored pools with one `GC_CHARACTER_POINT_CHANGE` each
/// before `PointsPacket` sends the pair again (`G/input_db.cpp:1563-1564`).
///
/// Alpha is a level-154 shaman with 17, 18, 19 and 20 in the four attributes, standing on
/// map 1, which demands no conqueror will. Every expected value is a hand sum of the
/// `JobInitialPoints` shaman row or of `ComputeBattlePoints` (`G/char.cpp`), not the engine's
/// output: the attack grade is 2 x 154 + (4 x 17 + 2 x 20) / 3, the defence grade
/// 154 + 18 x 4 / 5, the shown defence grade 154 + 18, the magic attack grade 2 x 154 + 2 x 20,
/// and the magic defence grade 154 + (3 x 20 + 18) / 3. The stored hit and spell points are
/// above their maxima; stamina is too, and `CheckMaximumPoints` leaves it, as legacy does. The
/// logout save writes the clamped pools.
#[test]
fn the_points_are_computed_at_load_and_the_item_load_clamps_the_pools() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    sql(
        &database,
        "UPDATE player SET hp = 100000, sp = 70000, stamina = 12345 WHERE name = 'Alpha'",
    );

    let (mut keyed, _empire, list) = select_screen(&server, b"alice");
    let alpha = listed(&list, 0);
    keyed.send_record(&client_select(0));
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_LOADING]);
    assert_eq!(keyed.read_game(), [GC_ENTITY, 3, 0]);
    assert_eq!(keyed.read_game()[0], GC_MAIN_CHARACTER2_EMPIRE);
    let gold = keyed.read_game();
    assert_eq!(gold[0], GC_CHARACTER_GOLD);
    let loaded = keyed.read_game();
    assert_eq!(loaded[0], GC_PLAYER_POINTS);
    keyed.loaded(&loaded);
    for (slot, value, what) in [
        (5, 100_000, "POINT_HP, as stored: the load does not clamp"),
        (6, 1420, "POINT_MAX_HP: 700 + 18 x 40"),
        (7, 70_000, "POINT_SP, as stored"),
        (8, 600, "POINT_MAX_SP: 200 + 20 x 20"),
        (9, 12_345, "POINT_STAMINA, as stored"),
        (10, 890, "POINT_MAX_STAMINA: 800 + 18 x 5"),
        (12, 17, "POINT_ST"),
        (13, 18, "POINT_HT"),
        (14, 19, "POINT_DX"),
        (15, 20, "POINT_IQ"),
        (16, 168, "POINT_DEF_GRADE"),
        (17, 100, "POINT_ATT_SPEED"),
        (18, 344, "POINT_ATT_GRADE"),
        (19, 100, "POINT_MOV_SPEED"),
        (20, 172, "POINT_CLIENT_DEF_GRADE"),
        (21, 100, "POINT_CASTING_SPEED"),
        (22, 348, "POINT_MAGIC_ATT_GRADE"),
        (23, 180, "POINT_MAGIC_DEF_GRADE"),
        (169, 34, "POINT_SUNGMA_STR"),
        (170, 35, "POINT_SUNGMA_HP"),
        (171, 36, "POINT_SUNGMA_MOVE"),
        (172, 37, "POINT_SUNGMA_IMMUNE"),
        (173, 33, "POINT_CONQUEROR_LEVEL"),
    ] {
        assert_eq!(point_slot(&loaded, slot), value, "{what}");
    }
    assert_eq!(keyed.read_game()[0], GC_SKILL_LEVEL_NEW);

    // `ItemLoad` ends with `CheckMaximumPoints`: `PointChange(POINT_HP, max - hp)` and the same
    // for spell points, each with `bAmount` false, so the amount is 0 and the value is the new
    // pool. Only then does `PointsPacket` write the pair, with the clamped pools in it.
    assert_eq!(keyed.read_game(), point_change(alpha.id, 5, 0, 1420));
    assert_eq!(keyed.read_game(), point_change(alpha.id, 7, 0, 600));
    assert_eq!(keyed.read_game(), gold, "the item load's gold record");
    let clamped = with_point_slot(&with_point_slot(&loaded, 5, 1420), 7, 600);
    assert_eq!(
        keyed.read_game(),
        clamped,
        "the item load's points record differs only in the two clamped pools"
    );
    assert_eq!(keyed.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));

    enter_game_burst(&mut keyed);
    drop(keyed);
    wait_for(
        &database,
        "(SELECT (hp, sp, stamina) = (1420, 600, 12345) FROM player WHERE name = 'Alpha')",
    );
}

/// `sys.char.points`: a conqueror whose map demands more will than it has is sent half its
/// movement speed and half its maximum hit points, and the item load clamps the stored hit
/// points to that half.
///
/// Map 373 (`metin2_map_eastplain_01`) demands a hit-point will of 10 and a movement will of
/// 15 (`GetSungMaWill`, `G/char.cpp:11717-11743`). Gamma is a level-90 warrior with conqueror
/// level 1, 9 hit-point sungma and 14 movement sungma, so `GetMaxHP` halves 600 + 4 x 40 to
/// 380 and `GetLimitPoint(POINT_MOV_SPEED)` halves 100 to 50. The attack speed has no will and
/// stays 100. Legacy computes both at `SetPlayerProto` with the map the saved position is on,
/// so the Channel must host map 373 for the position to be kept.
#[test]
fn a_conqueror_below_the_map_will_is_sent_half_its_speed_and_hit_points() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut channels = default_channels();
    channels[0].maps.push(373);
    let server = Server::start_with(binary(), database.url(), &channels);
    create_account(&server, "bob");
    sql(
        &database,
        "UPDATE account SET empire = 1 WHERE login = 'bob'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, st, ht, dx, iq, \
         conqueror_level, sungma_hp, sungma_move, hp, sp, stamina, x, y) SELECT id, 0, 'Gamma', \
         0, 90, 6, 4, 3, 3, 1, 9, 14, 700, 100, 820, 1100000, 500000 FROM account \
         WHERE login = 'bob'",
    );

    let (mut keyed, _empire, list) = select_screen(&server, b"bob");
    let gamma = listed(&list, 0);
    assert_eq!(
        (gamma.x, gamma.y),
        (1_100_000, 500_000),
        "Channel 1 hosts map 373"
    );
    keyed.send_record(&client_select(0));
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_LOADING]);
    assert_eq!(keyed.read_game(), [GC_ENTITY, 3, 0]);
    assert_eq!(keyed.read_game()[0], GC_MAIN_CHARACTER2_EMPIRE);
    let gold = keyed.read_game();
    assert_eq!(gold[0], GC_CHARACTER_GOLD);
    let loaded = keyed.read_game();
    assert_eq!(loaded[0], GC_PLAYER_POINTS);
    keyed.loaded(&loaded);
    for (slot, value, what) in [
        (5, 700, "POINT_HP, as stored"),
        (6, 380, "POINT_MAX_HP, halved under the hit-point will"),
        (8, 260, "POINT_MAX_SP, which has no will"),
        (17, 100, "POINT_ATT_SPEED, which has no will"),
        (19, 50, "POINT_MOV_SPEED, halved under the movement will"),
    ] {
        assert_eq!(point_slot(&loaded, slot), value, "{what}");
    }
    assert_eq!(keyed.read_game()[0], GC_SKILL_LEVEL_NEW);
    assert_eq!(keyed.read_game(), point_change(gamma.id, 5, 0, 380));
    assert_eq!(keyed.read_game(), gold, "the item load's gold record");
    assert_eq!(keyed.read_game(), with_point_slot(&loaded, 5, 380));
    assert_eq!(keyed.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));

    // `EncodeInsertPacket` writes `GetLimitPoint` of both speeds (`G/char.cpp:1089-1091`).
    let add = enter_game_burst(&mut keyed);
    assert_eq!(add.len(), CHARACTER_ADD_LEN);
    // The header, `dwVID`, `angle`, `x`, `y`, `z`, `bType` and `wRaceNum` come first.
    assert_eq!(add[24], 50, "bMovingSpeed");
    assert_eq!(add[25], 100, "bAttackSpeed");
    drop(keyed);
    wait_for(
        &database,
        "(SELECT (hp, sp, stamina) = (380, 100, 820) FROM player WHERE name = 'Gamma')",
    );
}

/// `cg.login.character_select`: an empty slot is `SetPhase(PHASE_CLOSE)`, and an index past the
/// last slot is ignored rather than closing. Legacy reads the slot array before it range-checks
/// the index, which is a Defect the Rewrite does not reproduce.
#[test]
fn an_empty_slot_closes_and_an_index_past_the_last_is_ignored() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);

    // Slot 2 is empty.
    let (mut keyed, _empire, list) = select_screen(&server, b"alice");
    assert!(slot_is_empty(&list, 2), "slot 2 is empty");
    keyed.closed_by(&client_select(2));

    // An index of 4 reads past the end of `TAccountTable::players` in legacy. Here it is
    // `Ignore`: nothing is sent and the connection stays open.
    let (mut keyed, _empire, _list) = select_screen(&server, b"alice");
    keyed.unanswered(&client_select(4));
    keyed.unanswered(&client_select(0xff));
    keyed.closed_by(&client_select(2));
}

/// `sys.login.enter`: a character standing on a map the Channel does not host is refused at
/// `map_allow_find`, after the records `PlayerLoad` writes before that test and before the gold,
/// points, and skill-level records that follow it. Slot 3 of alice stands on map 2, which no
/// Channel in the default configuration hosts.
#[test]
fn a_map_the_channel_does_not_host_warps_home_and_closes_silently_after_the_records() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);

    let (_keyed, _empire, list) = select_screen(&server, b"alice");
    // The character list moved Delta to the empire 1 start, so the refusal is about the map the
    // stored position is on, not about the moved one.
    assert_eq!(listed(&list, 3).port, server.channel(1).port());

    sql(
        &database,
        "UPDATE player SET x = 60000, y = 150000 WHERE name = 'Delta'",
    );
    let (mut keyed, _empire, _list) = select_screen(&server, b"alice");
    keyed.send_record(&client_select(3));
    // The phase record, the entity list, and the own-character record are written, then the
    // close. Nothing else.
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_LOADING]);
    assert_eq!(keyed.read_game(), [GC_ENTITY, 3, 0]);
    let main = keyed.read_game();
    assert_eq!(main[0], GC_MAIN_CHARACTER2_EMPIRE);
    assert_eq!(&main[7..32], &name_field(b"Delta"), "szName");
    // The refusal is `SetPhase(PHASE_CLOSE)`, and it is silent: the phase is assigned before
    // `Packet` is called, and `Packet` returns at once for `PHASE_CLOSE`, so the `GC_PHASE`
    // record it built is dropped. No gold, points, or skill level either, because
    // `PlayerLoad` returns straight after the call.
    assert_eq!(
        keyed.read_game_or_close(),
        None,
        "the descriptor closes without a record after the map test"
    );
    // The character was moved home in the store. Legacy sets a pending warp location and lets
    // the save on close write it (`G/input_db.cpp:432-434` then `G/char.cpp:1551-1565`); ADR-0003
    // makes that one transaction, and this is the statement that performs it. No `GC_WARP` is
    // sent, so the client learns nothing about the move except the close.
    check(
        &database,
        "(SELECT x FROM player WHERE name = 'Delta') = 469300",
    );
    check(
        &database,
        "(SELECT y FROM player WHERE name = 'Delta') = 964200",
    );
    // And the move is committed, so the next login loads the character on its home map and the
    // Channel accepts it.
    let (mut again, _empire, _list) = select_screen(&server, b"alice");
    again.send_record(&client_select(3));
    assert_eq!(again.read_game(), [GC_PHASE, PHASE_LOADING]);
    assert_eq!(again.read_game(), [GC_ENTITY, 3, 0]);
    let main = again.read_game();
    assert_eq!(
        main[0], GC_MAIN_CHARACTER2_EMPIRE,
        "the load is no longer refused"
    );
}

/// `sys.char.chat`: a talking line reaches every client on the sender's map, including the
/// sender, and no client on another map. A line that is only whitespace, an empty line, and a
/// line whose declared size is under the fixed part are all consumed without a record, and the
/// tenth line in a run schedules a disconnect.
#[test]
fn a_talking_line_reaches_the_map_including_its_sender() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    create_account(&server, "carol");
    add_characters(&database);
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         470000, 950000 FROM account WHERE login = 'bob'",
    );
    // Carol's character stands on map 3 of the same Channel, so the map filter has something to
    // exclude.
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Charlie', 2, \
         350000, 870000 FROM account WHERE login = 'carol'",
    );

    let (mut alice, alpha) = enter_world(&server, b"alice", 0);
    let (mut yankee, yankee_listed) = enter_world(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, true);
    let (mut charlie, _) = enter_world(&server, b"carol", 0);

    alice.send_record(&client_chat(CHAT_TALKING, b"hello"));
    // The sender gets its own line: `FEmpireChatPacket` filters by map index only.
    let own = alice.read_game();
    assert_eq!(own[0], GC_CHAT);
    assert_eq!(word_sized(&own), Some(own.len()));
    assert_eq!(&own[4..8], &alpha.id.to_le_bytes(), "id");
    assert_eq!(own[8], 1, "bEmpire is the character's empire");
    assert_eq!(&own[10..], b"Alpha : hello");
    // The other client on the same map gets the same bytes.
    let heard = yankee.read_game();
    assert_eq!(heard, own, "the same record reaches the neighbour");
    yankee.quiet("exactly one record");
    charlie.quiet("another map hears nothing");

    // `strlcpy` copies at most `iExtraLen + 1` bytes and stops at the first NUL, then
    // `snprintf` builds `"%s : %s"`. There is no `strlen(buf) < 1` arm, so an empty payload
    // and one whose first byte is NUL both become the bare name line and are broadcast.
    for payload in [&b""[..], &[b'\0', b'x'][..]] {
        alice.send_record(&client_chat(CHAT_TALKING, payload));
        let bare = alice.read_game();
        assert_eq!(bare[0], GC_CHAT);
        assert_eq!(word_sized(&bare), Some(bare.len()));
        assert_eq!(&bare[4..8], &alpha.id.to_le_bytes(), "id");
        assert_eq!(&bare[10..], b"Alpha : ", "the name line with no text");
        assert_eq!(
            yankee.read_game(),
            bare,
            "the neighbour sees the same record"
        );
    }
    yankee.quiet("exactly two more records");

    // `if (buflen > 1 && *buf == '/')` is checked before the counter, so a slash line costs
    // nothing and goes to the interpreter instead. `do_restart` is not ported, so it answers
    // nothing.
    alice.unanswered(&client_chat(CHAT_TALKING, b"/restart_here"));

    // One write holding a talking line and then a party line with no party: legacy writes the
    // talking line to the map while it reads it, before it reads the party line, so the sender
    // hears its own line before the refusal it is answered with (`G/input_main.cpp:926-966`).
    // Yankee sends it, because Alice's three lines leave her one short of the chat counter's
    // limit, which would drop the line unheard.
    let mut both = client_chat(CHAT_TALKING, b"again");
    both.extend(client_chat(prodomo::chat::CHAT_PARTY, b"hi"));
    yankee.send_record(&both);
    let again = yankee.read_game();
    assert_eq!(
        &again[10..],
        b"Yankee : again",
        "its own line first: {again:02x?}"
    );
    assert_eq!(
        yankee.read_game(),
        chat_packet(prodomo::chat::CHAT_INFO, again[8], b"[LS;655]"),
        "then the refusal, in the speaker's empire"
    );
    assert_eq!(alice.read_game(), again);
    alice.quiet("the neighbour hears the line and not the refusal");

    // A declared size under the record's own prefix cannot be framed. Legacy's
    // `if (size < sizeof(TPacketCGChat)) return -1;` stops consuming without
    // closing, so the descriptor stalls until the ping cycle drops it; the
    // Rewrite closes at once, which is the recorded framing Divergence.
    let mut short = client_chat(CHAT_TALKING, b"x");
    short[1] = 3;
    short[2] = 0;
    alice.closed_by(&short);
}

/// A chat line legacy answers with `CHARACTER::ChatPacket` comes back to the sender alone, as a
/// `CHAT_TYPE_INFO` line with `id` 0 and the sender's empire: a party line with no party, a
/// guild line with no guild, and a shout below the level limit (`G/input_main.cpp:888`, `:960`,
/// `:977`). The neighbour on the same map hears none of them.
///
/// Each line is looked up in the sender's language (`LC_LOCALE_TEXT`, `G/char.cpp:5151`). Bob
/// logs in in German, and the owner's German table has no pair for the shout refusal, so it goes
/// out as written, as legacy sends it.
#[test]
fn a_line_legacy_answers_itself_comes_back_to_the_sender_alone() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    // Bob's character stands beside Alice's, in another empire, and at level 1.
    sql(
        &database,
        "UPDATE account SET empire = 2 WHERE login = 'bob'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         470000, 950000 FROM account WHERE login = 'bob'",
    );
    let (mut alice, _) = enter_world(&server, b"alice", 0);
    let (mut yankee, yankee_listed) = enter_world_in(&server, b"bob", 0, GERMAN);
    alice.sees_arrive(yankee_listed.id, true);

    let info = |empire: u8, text: &[u8]| {
        let mut line = vec![GC_CHAT];
        line.extend_from_slice(&u16::try_from(10 + text.len()).unwrap().to_le_bytes());
        line.extend_from_slice(&[prodomo::chat::CHAT_INFO, 0, 0, 0, 0, empire, 1]);
        line.extend_from_slice(text);
        line
    };
    alice.send_record(&client_chat(prodomo::chat::CHAT_PARTY, b"hi"));
    assert_eq!(alice.read_game(), info(1, b"[LS;655]"), "no party");
    alice.send_record(&client_chat(prodomo::chat::CHAT_GUILD, b"hi"));
    assert_eq!(alice.read_game(), info(1, b"[LS;656]"), "no guild");
    yankee.send_record(&client_chat(prodomo::chat::CHAT_SHOUT, b"hi"));
    assert_eq!(
        yankee.read_game(),
        info(2, b"Shout can only be used at level 15 or higher."),
        "the limit is formatted into the line, and the empire is the speaker's"
    );
    yankee.quiet("the neighbour hears neither of Alice's lines");
    alice.quiet("and Alice does not hear Yankee's refusal");
}

/// A `GC_CHAT` line as `CHARACTER::ChatPacket` builds it (`G/char.cpp:5140-5189`): `id` 0, the
/// recipient's empire, `bCanFormat` 1, and the text with no terminator.
fn chat_packet(chat_type: u8, empire: u8, text: &[u8]) -> Vec<u8> {
    let mut line = vec![GC_CHAT];
    line.extend_from_slice(&u16::try_from(10 + text.len()).unwrap().to_le_bytes());
    line.extend_from_slice(&[chat_type, 0, 0, 0, 0, empire, 1]);
    line.extend_from_slice(text);
    line
}

/// Three clients in the world for a shout: alice's Alpha, level 154 in empire 1 on map 1 of
/// Channel 1, in English; bob's Yankee, level 1 in empire 2 on map 3 of Channel 1, in German;
/// and carol's Charlie, in `carol_empire` on the Shared Channel's map 72.
fn shout_cast(
    server: &Server,
    database: &ScratchDatabase,
    carol_empire: u8,
) -> (Keyed, Keyed, Keyed) {
    for login in ["alice", "bob", "carol"] {
        create_account(server, login);
    }
    add_characters(database);
    sql(
        database,
        "UPDATE account SET empire = 2 WHERE login = 'bob'",
    );
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         350000, 870000 FROM account WHERE login = 'bob'",
    );
    sql(
        database,
        &format!("UPDATE account SET empire = {carol_empire} WHERE login = 'carol'"),
    );
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Charlie', 2, \
         10000, 1210000 FROM account WHERE login = 'carol'",
    );
    let (alice, _) = enter_world(server, b"alice", 0);
    let (yankee, _) = enter_world_in(server, b"bob", 0, GERMAN);
    let (charlie, _) = enter_world_on(server, 99, b"carol", 0, ENGLISH);
    (alice, yankee, charlie)
}

/// `sys.char.chat` (shout): a shout at or above the level limit reaches every client of the
/// shouter's empire on every Channel, the shouter included, and no client of another empire
/// (`G/input_main.cpp:882-913`, `G/input_p2p.cpp:215-240`). It arrives as a `CHAT_TYPE_SHOUT` line
/// whose text opens with the shouter's country code. A second shout inside fifteen seconds of
/// Pulses reaches nobody.
///
/// Legacy refuses every shout in the first fifteen seconds of uptime, because the last shout
/// starts at Pulse 0. The Rewrite does not reproduce that Defect, so this scenario shouts at once.
#[test]
fn a_shout_reaches_its_empire_on_every_channel() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    let (mut alice, mut yankee, mut charlie) = shout_cast(&server, &database, 1);

    alice.send_record(&client_chat(prodomo::chat::CHAT_SHOUT, b"hello"));
    let line = chat_packet(prodomo::chat::CHAT_SHOUT, 1, b"|Len|l Alpha : hello");
    assert_eq!(alice.read_game(), line, "the shouter hears itself");
    assert_eq!(
        charlie.read_game(),
        line,
        "the Shared Channel hears the shouter's empire"
    );
    yankee.quiet("another empire hears nothing");

    alice.send_record(&client_chat(prodomo::chat::CHAT_SHOUT, b"again"));
    alice.quiet("the cooldown refuses without a line");
    charlie.quiet("and nobody hears the refused shout");
    yankee.quiet("not even the other empire");
}

/// `sys.char.chat` (shout), with `shout_limit_level` at 1 and global shouting off: a shout from
/// empire 2 reaches empire 2 on every Channel and nobody in empire 1. The empire `FuncShout`
/// compares is the shouter's (`G/input_p2p.cpp:227`), and the country code is the shouter's
/// language.
#[test]
fn a_shout_from_another_empire_stays_in_that_empire() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "shout_limit_level = 1",
    );
    let (mut alice, mut yankee, mut charlie) = shout_cast(&server, &database, 2);

    yankee.send_record(&client_chat(prodomo::chat::CHAT_SHOUT, b"hallo"));
    let line = chat_packet(prodomo::chat::CHAT_SHOUT, 2, b"|Lde|l Yankee : hallo");
    assert_eq!(yankee.read_game(), line, "the shouter hears itself");
    assert_eq!(
        charlie.read_game(),
        line,
        "the Shared Channel hears the shouter's empire"
    );
    alice.quiet("empire 1 hears nothing");
    yankee.quiet("one line per shout");
}

/// `sys.char.chat` (shout), with `[game] enable_global_shout` on and `shout_limit_level` at 1:
/// every client on every Channel hears every shout, each with its own empire byte, and a level 1
/// character may shout. The country code is the shouter's language, not the listener's.
#[test]
fn a_global_shout_reaches_every_empire_on_every_channel() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "enable_global_shout = true\nshout_limit_level = 1",
    );
    let (mut alice, mut yankee, mut charlie) = shout_cast(&server, &database, 3);

    // Two shouters' lines have no order between them, so each shout is heard before the next.
    for (shouter, line, text) in [
        (0, &b"hello"[..], &b"|Len|l Alpha : hello"[..]),
        (1, b"hallo", b"|Lde|l Yankee : hallo"),
    ] {
        let record = client_chat(prodomo::chat::CHAT_SHOUT, line);
        [&mut alice, &mut yankee][shouter].send_record(&record);
        for (keyed, empire) in [(&mut alice, 1), (&mut yankee, 2), (&mut charlie, 3)] {
            assert_eq!(
                keyed.read_game(),
                chat_packet(prodomo::chat::CHAT_SHOUT, empire, text),
                "empire {empire} hears {}",
                String::from_utf8_lossy(text),
            );
        }
    }
    for keyed in [&mut alice, &mut yankee, &mut charlie] {
        keyed.quiet("one line per shout");
    }
}

/// `sys.char.chat` (counter): the fourth line inside one counter window is dropped without a
/// record, and `CHARACTER_MANAGER::Update` resets every counter on each Pulse that is a multiple
/// of `PASSES_PER_SEC(5)` (`G/char_manager.cpp:700-704`), so a line is answered again once one of
/// those Pulses has passed.
#[test]
fn the_chat_counter_is_reset_every_five_seconds_of_pulses() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    let (mut alice, _) = enter_world(&server, b"alice", 0);
    let party = client_chat(prodomo::chat::CHAT_PARTY, b"hi");
    let answer = chat_packet(prodomo::chat::CHAT_INFO, 1, b"[LS;655]");
    let heard = |alice: &mut Keyed| {
        alice.send_record(&party);
        let (records, quiet) = alice.drain_game(QUIET_WINDOW, |header| match header {
            GC_CHAT => answer.len(),
            GC_PING => 1,
            other => panic!("unexpected record {other}"),
        });
        assert_eq!(quiet, Quiet::Open, "a dropped line never closes before ten");
        let lines: Vec<Vec<u8>> = records
            .into_iter()
            .filter(|record| record[0] != GC_PING)
            .collect();
        assert!(
            lines.is_empty() || lines == [answer.clone()],
            "{lines:02x?}"
        );
        !lines.is_empty()
    };

    // A reset can fall between any two lines and restart the count, so lines go out until one
    // is dropped. Seven lines take about two seconds, which holds at most one reset.
    let mut answered = 0;
    while heard(&mut alice) {
        answered += 1;
        assert!(
            answered < 7,
            "a fourth line inside one window must be dropped"
        );
    }
    // The counter is 4, and it disconnects at 10. A reset Pulse comes every 125 Pulses, so
    // three tries two seconds apart cross one, and the counter never passes 7.
    let mut dropped = 0;
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if heard(&mut alice) {
            break;
        }
        dropped += 1;
        assert!(dropped < 4, "the counter was never reset");
    }
}

/// Seat the move scenario's other two: Yankee in Alpha's empire at its point, so it hears
/// Alpha's shout, and Charlie of an account with no empire on map 1, 15700 west and 9300 north
/// of Alpha, out of view. Neither Alpha nor Yankee runs out of stamina, so neither sends a walk
/// mode.
fn seat_yankee_and_charlie(database: &ScratchDatabase) {
    sql(
        database,
        "UPDATE account SET empire = 1 WHERE login = 'bob'",
    );
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         470000, 950000 FROM account WHERE login = 'bob'",
    );
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Charlie', 2, \
         454300, 940700 FROM account WHERE login = 'carol'",
    );
    stamina(database, "Alpha", 12_345);
    stamina(database, "Yankee", 12_345);
    stamina(database, "Charlie", 12_345);
}

/// `cg.world.move`: an accepted move reaches the characters in the mover's view and never the
/// mover, and nobody out of view. The moved position is the client's own bytes, so the relay
/// carries `lX` and `lY` unchanged, and a duration only on the `FUNC_MOVE` branch, the one that
/// calls `Goto`. A move past the legacy distance limit is refused with a `Show` at the mover's
/// own point and a `Stop` (`G/input_main.cpp:1786-1811`). In the same sectree that `Show` is
/// `ViewReencode` (`G/entity_view.cpp:23-45`): every viewer is sent the mover's removal and its
/// insert pair, and the mover its own removal and pair and then the pair of each one it sees,
/// players before NPCs and each by VID (V1), with no removal first.
#[test]
fn a_move_reaches_the_view_around_the_mover_and_not_a_character_out_of_view() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    create_account(&server, "carol");
    add_characters(&database);
    seat_yankee_and_charlie(&database);

    let (mut alice, alpha, alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    let (_, alpha_info) = yankee_entered.shown.pair_of(alpha.id);
    assert_eq!(
        alpha_info, alpha_entered.info,
        "the summary Alpha got of itself"
    );
    let (_, yankee_info) = alice.sees_arrive(yankee_listed.id, false);
    assert_eq!(
        yankee_info, yankee_entered.info,
        "the summary Yankee got of itself"
    );
    // `bPKMode` is `m_bPKMode` (`G/char.cpp:1128`): Yankee, at level 1, is below the europe
    // `PK_PROTECT_LEVEL` of 15 and in `PK_MODE_PROTECT`; Alpha, at 154, is in `PK_MODE_PEACE`
    // as a `GM_PLAYER` (`test_server`'s GM protect rule waits for its audit, STATUS Topology).
    assert_eq!(yankee_info[57], 3, "Yankee's bPKMode");
    assert_eq!(alpha_info[57], 0, "Alpha's bPKMode");
    let (mut charlie, _) = enter_world(&server, b"carol", 0);
    alice.quiet("Charlie enters out of Alpha's view");
    yankee.quiet("and out of Yankee's");

    // `FUNC_COMBO` is 3. A step lands at the position, and the record carries no duration.
    let step = client_move(3, 7, 40, 470_100, 950_100, 0x5eed);
    alice.send_record(&step);
    assert_eq!(
        yankee.read_game(),
        relayed_move(&step, alpha.id, 0),
        "bFunc, bArg, bRot unmultiplied, lX, lY and dwTime are the client's; dwDuration is 0"
    );
    alice.quiet("PacketAround excludes the mover");

    // `FUNC_MOVE` is 1 and it is the branch that calls `Goto`, so the record carries the
    // duration `Goto` computed. Alpha, a shaman with no weapon, runs the general run clip at 450
    // a second, so the diagonal of (100, 100), 141.42 units, is 314 ms.
    let walk = client_move(1, 0, 0, 470_200, 950_200, 0x5eee);
    alice.send_record(&walk);
    assert_eq!(yankee.read_game(), relayed_move(&walk, alpha.id, 314));
    alice.quiet("the mover is still excluded");

    // The distance test compares against 999 units of 100, so 200000 is refused.
    alice.send_record(&client_move(1, 0, 0, 470_000 + 200_000, 950_000, 0x5eef));
    // The viewer: Alpha's removal.
    yankee.sees_leave(alpha.id);
    let (shown_again, info_again) = yankee.sees_arrive(alpha.id, false);
    assert_eq!(&shown_again[17..21], &0i32.to_le_bytes(), "at its own z");
    assert_eq!(info_again, alpha_entered.info, "then its pair");
    yankee.quiet("no move follows: the refusal stopped Alpha");
    // The mover: its own removal.
    alice.sees_leave(alpha.id);
    let (own_again, _) = alice.sees_arrive(alpha.id, false);
    assert_eq!(own_again[21], 6, "CHAR_TYPE_PC");
    alice.sees_arrive(yankee_listed.id, false);
    // Entering walked the NPCs sectree by sectree; the re-encode sends them in key order, by VID.
    let mut npcs: Vec<u32> = alpha_entered
        .shown
        .inserts()
        .iter()
        .map(|insert| u32::from_le_bytes(insert[1..5].try_into().expect("four bytes")))
        .collect();
    assert!(!npcs.is_empty(), "map 1's NPCs around Alpha");
    npcs.sort_unstable();
    for npc in npcs {
        let (insert, _) = alice.sees_arrive(npc, false);
        assert_eq!(insert[21], 1, "CHAR_TYPE_NPC");
    }
    alice.quiet("the refusal's records are over");

    // A function byte of 6 is past `FUNC_MAX_NUM`, which is the first refused value.
    alice.unanswered(&client_move(6, 0, 0, 470_100, 950_100, 0x5ef0));
    yankee.quiet("an invalid function sends nothing to anyone");
    // A function byte with the skill bit set passes the range test and steps.
    let skill = client_move(0x80, 0, 0, 470_300, 950_300, 0x5ef1);
    alice.send_record(&skill);
    assert_eq!(
        yankee.read_game(),
        relayed_move(&skill, alpha.id, 0),
        "bFunc is relayed, and a step relays no duration"
    );

    // One write holding a move refused for its distance and then a shout under the level
    // limit: legacy writes the refusal's records while it reads the move, before it reads the
    // shout, so the mover's whole re-encode comes before the info line it is answered with.
    let mut both = client_move(1, 0, 0, 470_000 + 200_000, 950_000, 0x5ef2);
    both.extend(client_chat(prodomo::chat::CHAT_SHOUT, b"hi"));
    yankee.send_record(&both);
    // The mover's own removal first.
    yankee.sees_leave(yankee_listed.id);
    let (inserted, line) = yankee.reencode_then_line();
    assert!(inserted.contains(&yankee_listed.id), "itself: {inserted:?}");
    assert!(inserted.contains(&alpha.id), "and Alpha: {inserted:?}");
    assert_eq!(line[3], prodomo::chat::CHAT_INFO, "{line:02x?}");
    assert!(line.ends_with(b"level 15 or higher."), "{line:02x?}");
    yankee.quiet("nothing follows the line");
    alice.sees_leave(yankee_listed.id);
    alice.sees_arrive(yankee_listed.id, false);
    alice.quiet("the viewer hears the refusal and not the shout");

    // The same write from a character above the limit: the viewer hears the refusal's removal
    // and insert before the shout, and the shouter hears its own re-encode first.
    let mut both = client_move(1, 0, 0, 470_000 + 200_000, 950_000, 0x5ef3);
    both.extend(client_chat(prodomo::chat::CHAT_SHOUT, b"hi"));
    alice.send_record(&both);
    let shout = chat_packet(prodomo::chat::CHAT_SHOUT, 1, b"|Len|l Alpha : hi");
    yankee.sees_leave(alpha.id);
    yankee.sees_arrive(alpha.id, false);
    assert_eq!(yankee.read_game(), shout, "the shout after the refusal");
    yankee.quiet("the viewer hears nothing more");
    alice.sees_leave(alpha.id);
    let (_, own_line) = alice.reencode_then_line();
    assert_eq!(own_line, shout, "the shouter hears itself last");
    alice.quiet("nothing follows the shout");
    charlie.quiet("out of view and of another empire, Charlie hears none of it");
}

/// `sys.char.position`: sitting and standing reach every client in the sender's view and the
/// sender, because `Standup` and `Sitdown` call `PacketAround` with no `except`, and nobody out
/// of view on the map. A pose the character is already in is ignored, and legacy collapses the
/// ground pose onto the chair value, which is a Defect the Rewrite does not reproduce.
#[test]
fn a_pose_reaches_the_view_including_its_sender() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    create_account(&server, "carol");
    add_characters(&database);
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         470000, 950000 FROM account WHERE login = 'bob'",
    );
    // Charlie stands on map 1, 15700 west and 9300 north of Alpha, out of view.
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Charlie', 2, \
         454300, 940700 FROM account WHERE login = 'carol'",
    );

    let (mut alice, alpha) = enter_world(&server, b"alice", 0);
    let (mut yankee, yankee_listed) = enter_world(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, true);
    let (mut carol, _charlie) = enter_world(&server, b"carol", 0);

    // Legacy `Sitdown` writes `POSITION_SITTING_GROUND` for both chair and ground and drops
    // its `is_ground` argument, so the chair request is a recorded Divergence: the Rewrite
    // answers the chair with `POSITION_SITTING_CHAIR` and the ground with
    // `POSITION_SITTING_GROUND`.
    alice.send_record(&client_position(POSITION_SITTING_CHAIR));
    let pose = alice.read_game();
    assert_eq!(pose.len(), CHARACTER_POSITION_LEN);
    assert_eq!(pose[0], GC_CHARACTER_POSITION);
    assert_eq!(&pose[1..5], &alpha.id.to_le_bytes(), "dwVID");
    assert_eq!(pose[5], POSITION_SITTING_CHAIR);
    assert_eq!(
        yankee.read_game(),
        pose,
        "the neighbour gets the same record"
    );

    // Standing again is a second record to everyone.
    alice.send_record(&client_position(POSITION_GENERAL));
    let stood = alice.read_game();
    assert_eq!(stood[5], POSITION_GENERAL);
    assert_eq!(yankee.read_game(), stood);

    // `Sitdown(1)` is the other arm of the same switch and reaches the same state. `/dance1`
    // needs `POS_FIGHTING`, and `interpret_command` tells a sitting character so
    // (`G/cmd.cpp:679`); only the one who typed it is told. In one write, the pose comes first:
    // legacy writes it to the view and the sender while it reads it, before it reads the line.
    let mut both = client_position(POSITION_SITTING_GROUND);
    both.extend(slash(b"/dance1"));
    alice.send_record(&both);
    let ground = alice.read_game();
    assert_eq!(
        ground[5], POSITION_SITTING_GROUND,
        "the pose first: {ground:02x?}"
    );
    assert_eq!(alice.read_game(), info_to_alpha(b"[LS;920]"));
    assert_eq!(yankee.read_game(), ground);

    // Sitting while already sitting is the `if (IsPosition(POS_SITTING)) return;` arm.
    alice.unanswered(&client_position(POSITION_SITTING_CHAIR));

    // Standing from the ground is a third record, and it leaves the character standing.
    alice.send_record(&client_position(POSITION_GENERAL));
    let stood_again = alice.read_game();
    assert_eq!(stood_again[5], POSITION_GENERAL);
    assert_eq!(yankee.read_game(), stood_again);

    // Standing while already standing is the `if (!IsPosition(POS_SITTING)) return;` arm.
    alice.unanswered(&client_position(POSITION_GENERAL));
    // Standing, `/dance1` passes the position check and reaches `do_emotion`, which this build
    // does not port, so nothing is sent.
    alice.unanswered(&slash(b"/dance1"));
    yankee.quiet("a command line reaches only the one who typed it");
    // An unknown pose byte is not a legacy arm, so nothing is sent.
    alice.unanswered(&client_position(0x7f));
    carol.quiet("out of view, Charlie is sent none of the poses");
}

/// `sys.world.move`: a sync batch is relayed to the claimer's view and never to the claimer or
/// anyone out of view, and an unknown VID or a victim of the wrong kind is skipped while the rest
/// of the batch still goes out.
#[test]
fn a_sync_batch_is_relayed_around_the_claimer_only() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    create_account(&server, "carol");
    add_characters(&database);
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         470000, 950000 FROM account WHERE login = 'bob'",
    );
    // Charlie stands on map 1 out of everyone's view.
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Charlie', 2, \
         454300, 940700 FROM account WHERE login = 'carol'",
    );
    // Yankee and Charlie never walk, so they are stored above every stamina maximum.
    stamina(&database, "Yankee", 12_345);
    stamina(&database, "Charlie", 12_345);

    // Yankee and Charlie never walk, so they are stored above every stamina maximum.
    stamina(&database, "Yankee", 12_345);
    stamina(&database, "Charlie", 12_345);

    let (mut alice, alpha) = enter_world(&server, b"alice", 0);
    let (mut yankee, yankee_id) = enter_world(&server, b"bob", 0);
    alice.sees_arrive(yankee_id.id, false);
    let (mut charlie, _) = enter_world(&server, b"carol", 0);

    // A claim on Yankee from a position 10 units away, inside every limit.
    //
    // `SetSyncOwner` writes `TPacketGCOwnership` with a plain `PacketAround` as it judges
    // the claim, so the victim sees its own ownership. The position batch follows and
    // excepts the claimer.
    alice.send_record(&client_sync_position(&[(yankee_id.id, 470_010, 950_010)]));
    let claimed = yankee.read_game();
    assert_eq!(claimed.len(), OWNERSHIP_LEN, "TPacketGCOwnership");
    assert_eq!(claimed[0], GC_OWNERSHIP);
    assert_eq!(&claimed[1..5], &alpha.id.to_le_bytes(), "dwOwnerVID");
    assert_eq!(&claimed[5..9], &yankee_id.id.to_le_bytes(), "dwVictimVID");
    let relayed = yankee.read_game();
    assert_eq!(relayed[0], GC_SYNC_POSITION);
    assert_eq!(word_sized(&relayed), Some(relayed.len()));
    assert_eq!(relayed.len(), SYNC_POSITION_LEN);
    assert_eq!(&relayed[3..7], &yankee_id.id.to_le_bytes(), "dwVID");
    assert_eq!(&relayed[7..11], &470_010i32.to_le_bytes(), "lX");
    assert_eq!(&relayed[11..15], &950_010i32.to_le_bytes(), "lY");

    // The ownership record has no exception, so the claimer reads that one and never the
    // position batch.
    let own = alice.read_game();
    assert_eq!(
        own[0], GC_OWNERSHIP,
        "the claimer sees the ownership record"
    );
    alice.quiet("the claimer never sees its own batch");

    // An unknown VID is skipped, so an empty batch produces no record at all.
    alice.unanswered(&client_sync_position(&[(999_999, 470_010, 950_010)]));
    yankee.quiet("an empty batch sends nothing");

    // `if (ch == this) { sys_err("SetSyncOwner owner == this"); return false; }` refuses a
    // character that names itself, so a client cannot move itself by claiming its own VID.
    alice.unanswered(&client_sync_position(&[(alpha.id, 470_010, 950_010)]));
    yankee.quiet("a self claim sends nothing");

    // The other direction works the same way: Yankee claims Alpha, and the ownership record
    // goes to the view around Alpha and to Alpha with no exception, so Alice reads it and
    // Yankee does not read the position batch that follows it.
    yankee.send_record(&client_sync_position(&[(alpha.id, 470_020, 950_020)]));
    let seen = alice.read_game();
    assert_eq!(seen.len(), OWNERSHIP_LEN);
    assert_eq!(seen[0], GC_OWNERSHIP);
    assert_eq!(
        &seen[1..5],
        &yankee_id.id.to_le_bytes(),
        "dwOwnerVID is Yankee"
    );
    assert_eq!(&seen[5..9], &alpha.id.to_le_bytes(), "dwVictimVID is Alpha");
    // The ownership record has no exception, so the claimer reads that one too and never the
    // position batch that follows it.
    let own_claim = yankee.read_game();
    assert_eq!(own_claim.len(), OWNERSHIP_LEN);
    assert_eq!(own_claim[0], GC_OWNERSHIP);
    yankee.quiet("the claimer never sees its own batch");
    // The batch excepts the claimer, not the victim, so Alice also reads the relayed
    // position of the character Yankee moved.
    let moved = alice.read_game();
    assert_eq!(moved[0], GC_SYNC_POSITION);
    assert_eq!(word_sized(&moved), Some(moved.len()));
    assert_eq!(&moved[3..7], &alpha.id.to_le_bytes(), "dwVID");
    assert_eq!(&moved[7..11], &470_020i32.to_le_bytes(), "lX");
    assert_eq!(&moved[11..15], &950_020i32.to_le_bytes(), "lY");
    alice.quiet("the victim sees the ownership record and the batch");

    // `if (!IsSyncOwner(ch)) return false;` keeps a second character off a target another
    // character still holds, because `ENABLE_FLY_FIX` refreshes the 100-unit claim stamp on
    // every accepted claim. Alpha is now Yankee's.
    alice.send_record(&client_sync_position(&[(alpha.id, 470_030, 950_030)]));
    alice.quiet("a refused claim writes no record to the claimer");
    yankee.quiet("a refused claim writes no record to the victim");
    charlie.quiet("a character out of view hears no claim and no batch");
}

/// Where Yankee stands in the view scenarios: 5092 from Alpha on [`GOTO_TARGET`] and 2280 from
/// idx18, in sectree row 146.
const VIEW_B: (i32, i32) = (444_100, 937_400);
/// Where Charlie stands in the view scenarios: 13228 from Alpha, 11116 from Yankee and 13396
/// from idx18, so out of every view at a range of 10000.
const VIEW_C: (i32, i32) = (454_300, 940_700);
/// A point in sectree row 147, outside the 3x3 around Alpha's row 145 and 9128 from Alpha, so
/// within reach but not in a sectree Alpha's view walks.
const ROW_147: (i32, i32) = (444_100, 941_600);
/// The VID of idx18, the 9009 at [`IDX18_AT`].
const IDX18_VID: u32 = FIRST_NPC_VID + 18;
/// `FUNC_MOVE` and `FUNC_COMBO`, `TPacketCGMove`'s `bFunc`.
const FUNC_MOVE: u8 = 1;
const FUNC_COMBO: u8 = 3;
/// `FUNC_ATTACK` (`prodomo/src/movement.rs`): a melee swing while moving. It ends the revive.
const FUNC_ATTACK: u8 = 2;
/// A walk of 1600 at the general run clip of a shaman or an assassin with no weapon, 450 a second
/// (`pc/shaman/general/run.msa` and `pc/assassin/general/run.msa`, 300 over 0.666667 s and 270
/// over 0.6 s): 3555 ms. Yankee, an assassin, and Alpha, a shaman, are the walkers here.
const LONG_WALK_MS: u32 = 3555;

/// The owner's dagger, `WEAR_WEAPON` `WEAPON_DAGGER`, which a shaman may wear.
const DAGGER: u32 = 1_000;

/// Give Alpha the dagger in the weapon cell, window 2 at 4 as the worn fan is.
fn give_alpha_dagger(database: &ScratchDatabase) {
    let protos = owners_protos();
    let dagger = protos.get(DAGGER).expect("the owner's data has it");
    assert_eq!(
        (dagger.item_type, dagger.sub_type),
        (
            gamedata::item_kind::ITEM_WEAPON,
            gamedata::item_kind::WEAPON_DAGGER
        ),
        "a dagger"
    );
    sql(
        database,
        &format!(
            "INSERT INTO item (id, owner_id, window_type, pos, count, vnum) SELECT 20, id, 2, 4, \
             1, {DAGGER} FROM player WHERE name = 'Alpha'"
        ),
    );
}

/// Alpha, a shaman at stamina 0 so it walks, and Yankee, in view of it, on the diagonal scenario's
/// map. Alpha steps to (470100, 950100), then walks 1600 south to (470100, 951700); with `dagger`
/// Alpha wears the dagger. Yankee is relayed the walk with `duration` ms.
fn alpha_walks_1600_south(database: &ScratchDatabase, dagger: bool, duration: u32) {
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    create_account(&server, "carol");
    add_characters(database);
    seat_yankee_and_charlie(database);
    if dagger {
        give_alpha_dagger(database);
    }
    stamina(database, "Alpha", 0);
    let (mut alice, alpha, _) = if dagger {
        enter_equipped_world_with(&server, b"alice", 1)
    } else {
        enter_world_seeing(&server, b"alice", 0)
    };
    let (mut yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    let (_, yankee_info) = alice.sees_arrive(yankee_listed.id, false);
    assert_eq!(
        yankee_info, yankee_entered.info,
        "the summary Yankee got of itself"
    );

    let step = client_move(FUNC_COMBO, 0, 0, 470_100, 950_100, 0x5eed);
    alice.send_record(&step);
    assert_eq!(yankee.read_game(), relayed_move(&step, alpha.id, 0));
    alice.quiet("PacketAround excludes the mover");

    let walk = client_move(FUNC_MOVE, 0, 0, 470_100, 951_700, 0x5eee);
    alice.send_record(&walk);
    assert_eq!(yankee.read_game(), relayed_move(&walk, alpha.id, duration));
}

/// A walking shaman with no weapon moves at its general walk clip, `pc/shaman/general/walk.msa`:
/// 1.0 s over 155.74 units, so the 1600-unit walk is 10273 ms (`CalculateMoveDuration`,
/// `G/char.cpp:3585-3595`).
#[test]
fn a_walking_shaman_moves_at_its_general_walk_clip() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    alpha_walks_1600_south(&database, false, 10_273);
}

/// A walking shaman with a dagger moves at its `dualhand_sword` walk clip,
/// `pc/shaman/dualhand_sword/walk.msa`: 0.8 s over 176.86 units, 221.075 a second, so the same
/// walk is 7237 ms, not the general clip's 10273. The dagger's only addon is attack speed.
#[test]
fn a_walking_dagger_wearer_moves_at_its_dualhand_walk_clip() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    alpha_walks_1600_south(&database, true, 7_237);
}

/// A Server whose view range is 10000, so its radius is 10500, with alice's Alpha, bob's Yankee
/// and carol's Charlie on map 1 at `points`, each stored at `stamina_value`.
fn view_cast(database: &ScratchDatabase, points: [(i32, i32); 3], stamina_value: i32) -> Server {
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "view_range = 10000",
    );
    for login in ["alice", "bob", "carol"] {
        create_account(&server, login);
    }
    add_characters(database);
    let [(x, y), yankee, charlie] = points;
    sql(
        database,
        &format!("UPDATE player SET x = {x}, y = {y} WHERE name = 'Alpha'"),
    );
    for (login, name, job, (x, y)) in [
        ("bob", "Yankee", 1, yankee),
        ("carol", "Charlie", 2, charlie),
    ] {
        sql(
            database,
            &format!(
                "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, '{name}', \
                 {job}, {x}, {y} FROM account WHERE login = '{login}'"
            ),
        );
    }
    for name in ["Alpha", "Yankee", "Charlie"] {
        stamina(database, name, stamina_value);
    }
    server
}

/// The bytes of a `GC_CHARACTER_ADD` between its angle and its affect flags: the point, z,
/// type, race, both speeds and the state flag. The angle and the flags are masked (R5, V9).
fn insert_body(insert: &[u8]) -> &[u8] {
    &insert[9..27]
}

/// Assert that `insert` is idx18's `GC_CHARACTER_ADD`: an NPC of race 9009 at [`IDX18_AT`].
fn assert_idx18(insert: &[u8]) {
    assert_eq!(&insert[1..5], &IDX18_VID.to_le_bytes(), "idx18's VID");
    assert_eq!(inserted_at(insert), IDX18_AT, "at its point");
    assert_eq!(insert[21], 1, "CHAR_TYPE_NPC");
    assert_eq!(&insert[22..24], &9009u16.to_le_bytes(), "race 9009");
}

/// Assert that the `GC_MOVE` an insert carries for `vid` walks to `dest` with some time left
/// of a walk of `walk_ms`: `FUNC_MOVE`, argument 0, and a duration from 1 to `walk_ms - 1`. The
/// rotation and the start time are masked.
fn assert_insert_move(record: &[u8], vid: u32, dest: (i32, i32), walk_ms: u32) {
    assert_eq!(record.len(), GC_MOVE_LEN);
    assert_eq!(&record[..3], &[GC_MOVE, FUNC_MOVE, 0], "{record:02x?}");
    assert_eq!(&record[4..8], &vid.to_le_bytes(), "the mover's VID");
    assert_eq!(&record[8..12], &dest.0.to_le_bytes(), "the destination's x");
    assert_eq!(
        &record[12..16],
        &dest.1.to_le_bytes(),
        "the destination's y"
    );
    let left = u32::from_le_bytes(record[20..24].try_into().expect("four bytes"));
    assert!(
        // The whole walk, when it is sent in the Pulse the move starts in.
        (1..=walk_ms).contains(&left),
        "iDur {left} is the walk's remainder"
    );
}

/// Wait until a walk sent at `sent` of `walk_ms` has ended, and 1.3 seconds more for its last
/// Pulses.
fn wait_out_walk(sent: std::time::Instant, walk_ms: u32) {
    let end = sent + Duration::from_millis(u64::from(walk_ms) + 1300);
    std::thread::sleep(end.saturating_duration_since(std::time::Instant::now()));
}

/// `sys.world.view`: at a view range of 10000, entering shows each character within 10500 in the
/// 3x3 sectrees around the entrant (`CEntity::UpdateSectree`, `G/entity_view.cpp:122-236`), and
/// the entrant to each player among them. Alpha enters first and sees idx18 alone, Yankee 5092
/// away sees idx18 and then Alpha in `Build` order, and Alpha sees Yankee arrive. Charlie, 13228
/// and 11116 away, sees nobody and nobody sees Charlie. No row walks, so no walk mode is sent.
#[test]
fn players_and_npcs_within_view_range_see_each_other_on_entry() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = view_cast(&database, [GOTO_TARGET, VIEW_B, VIEW_C], 12_345);

    let (mut alice, alpha, alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    let own = &alpha_entered.add;
    assert_eq!(inserted_at(own), GOTO_TARGET);
    assert_eq!(&own[17..21], &0i32.to_le_bytes(), "z 0");
    assert_eq!(own[21], 6, "CHAR_TYPE_PC");
    assert_eq!(own[24], 100, "bMovingSpeed");
    assert_eq!(alpha_entered.shown.inserts().len(), 1, "idx18 alone");
    assert_idx18(&alpha_entered.shown.records[0]);
    assert_eq!(alpha_entered.shown.records[1][0], GC_CHAR_ADDITIONAL_INFO);
    assert_eq!(alpha_entered.shown.records.len(), 2);
    assert!(alpha_entered.shown.list.is_some(), "then map 1's list");

    let (mut yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    assert_eq!(inserted_at(&yankee_entered.add), VIEW_B);
    let seen = &yankee_entered.shown.records;
    assert_eq!(seen.len(), 4, "idx18's pair and Alpha's");
    assert_idx18(&seen[0]);
    assert_eq!(
        &seen[1][..5],
        &[&[GC_CHAR_ADDITIONAL_INFO][..], &IDX18_VID.to_le_bytes()].concat()
    );
    assert_eq!(
        &seen[2][1..5],
        &alpha.id.to_le_bytes(),
        "Alpha's sectree comes second"
    );
    assert_eq!(
        insert_body(&seen[2]),
        insert_body(own),
        "as Alpha saw itself"
    );
    assert_eq!(
        seen[3], alpha_entered.info,
        "the summary Alpha got of itself"
    );
    let (add, info) = alice.sees_arrive(yankee_listed.id, false);
    assert_eq!(insert_body(&add), insert_body(&yankee_entered.add));
    assert_eq!(info, yankee_entered.info);
    alice.quiet("Yankee's pair alone");

    let (mut charlie, charlie_listed, charlie_entered) = enter_world_seeing(&server, b"carol", 0);
    assert_eq!(inserted_at(&charlie_entered.add), VIEW_C);
    assert_eq!(&charlie_entered.add[1..5], &charlie_listed.id.to_le_bytes());
    assert!(
        charlie_entered.shown.records.is_empty(),
        "nobody within reach"
    );
    assert!(charlie_entered.shown.list.is_some());
    alice.quiet("Charlie is 13228 away");
    yankee.quiet("and 11116 from Yankee");
    charlie.quiet("Charlie's burst is over");
}

/// `sys.world.view`: reach is not enough. Yankee enters in sectree row 147, 9128 from Alpha, and
/// sees idx18 at 5697, but Alpha's row 145 is outside the 3x3 around row 147, so neither sees the
/// other.
#[test]
fn a_player_in_another_row_of_sectrees_is_not_seen_even_within_range() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = view_cast(&database, [GOTO_TARGET, ROW_147, VIEW_C], 12_345);
    let (mut alice, _alpha, _) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, _, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    assert_eq!(inserted_at(&yankee_entered.add), ROW_147);
    let seen = &yankee_entered.shown.records;
    assert_eq!(seen.len(), 2, "idx18's pair alone");
    assert_idx18(&seen[0]);
    assert!(yankee_entered.shown.list.is_some());
    alice.quiet("Yankee's sectree is not around Alpha's");
    yankee.quiet("Yankee's burst is over");
}

/// `sys.world.view`: a player with nobody within reach is shown itself and the map's list.
#[test]
fn a_lone_player_far_from_everyone_sees_only_itself() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = view_cast(&database, [GOTO_TARGET, VIEW_B, VIEW_C], 12_345);
    let (mut charlie, _, entered) = enter_world_seeing(&server, b"carol", 0);
    assert!(entered.shown.records.is_empty(), "its own pair alone");
    let list = entered.shown.list.expect("map 1's list");
    assert_eq!(
        &list[1..5],
        &[&1637u16.to_le_bytes()[..], &48u16.to_le_bytes()].concat()
    );
    charlie.quiet("nothing else");
}

/// `sys.world.view`: a moving player's view is recomputed every 16 Pulses
/// (`G/char_state.cpp:797`), and only then. Yankee walks from row 147 into row 146; at the first
/// sample there, Yankee is shown Alpha, and Alpha is shown Yankee at its live point with the
/// rest of its walk and the run walk mode (`EncodeInsertPacket`, `G/char.cpp:1097-1108`,
/// `:1211-1223`). The walk back is relayed to Alpha whole, and at the first sample in row 147
/// again each is removed from the other.
#[test]
fn a_walk_into_view_inserts_both_ways_at_the_sample_and_a_walk_out_removes_both() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = view_cast(&database, [GOTO_TARGET, ROW_147, VIEW_C], 12_345);
    let (mut alice, alpha, alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    let yankee_id = yankee_listed.id;
    alice.quiet("out of view");

    let row_146 = (444_100, 940_000);
    let sent = std::time::Instant::now();
    yankee.send_record(&client_move(FUNC_MOVE, 0, 10, row_146.0, row_146.1, 0x6000));
    let (add, info) = yankee.sees_arrive(alpha.id, false);
    assert_eq!(insert_body(&add), insert_body(&alpha_entered.add));
    assert_eq!(info, alpha_entered.info);
    let (add, info) = alice.sees_arrive(yankee_id, false);
    let (x, y) = inserted_at(&add);
    assert_eq!(x, 444_100);
    assert!(
        (940_000..940_800).contains(&y),
        "the live point in row 146: {y}"
    );
    assert_eq!(info, yankee_entered.info);
    assert_insert_move(&alice.read_game(), yankee_id, row_146, LONG_WALK_MS);
    assert_eq!(
        alice.read_game(),
        walk_mode_of(&yankee_id.to_le_bytes()),
        "a moving insert's mode"
    );
    alice.quiet("one insert");
    yankee.quiet("one insert");

    wait_out_walk(sent, LONG_WALK_MS);
    yankee.send_record(&client_move(FUNC_MOVE, 2, 30, ROW_147.0, ROW_147.1, 0x6001));
    let mut relay = vec![GC_MOVE, FUNC_MOVE, 2, 30];
    relay.extend_from_slice(&yankee_id.to_le_bytes());
    relay.extend_from_slice(&ROW_147.0.to_le_bytes());
    relay.extend_from_slice(&ROW_147.1.to_le_bytes());
    relay.extend_from_slice(&0x6001u32.to_le_bytes());
    relay.extend_from_slice(&LONG_WALK_MS.to_le_bytes());
    assert_eq!(alice.read_game(), relay, "from where the first walk ended");
    alice.sees_leave(yankee_id);
    yankee.sees_leave(alpha.id);
    alice.quiet("one removal");
    yankee.quiet("one removal");
}

/// `sys.world.view`: an NPC is shown when a walker's sample finds it, and removed at the first
/// sample that does not. Alpha enters in row 144 with no NPC in view, walks into row 145, where
/// idx18 in row 146 is within the 3x3 and 10500, and walks back.
#[test]
fn an_npc_appears_and_disappears_as_a_player_walks() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let row_144 = (444_100, 927_200);
    let server = view_cast(&database, [row_144, VIEW_B, VIEW_C], 12_345);
    let (mut alice, _alpha, entered) = enter_world_seeing(&server, b"alice", 0);
    assert!(entered.shown.records.is_empty(), "no NPC in view");
    let list = entered.shown.list.expect("map 1's list");
    assert_eq!(
        &list[1..5],
        &[&1637u16.to_le_bytes()[..], &48u16.to_le_bytes()].concat()
    );

    let sent = std::time::Instant::now();
    alice.send_record(&client_move(FUNC_MOVE, 0, 0, 444_100, 928_800, 0x6100));
    let (add, _) = alice.sees_arrive(IDX18_VID, false);
    assert_idx18(&add);
    alice.quiet("idx18 alone");

    wait_out_walk(sent, LONG_WALK_MS);
    alice.send_record(&client_move(FUNC_MOVE, 0, 0, row_144.0, row_144.1, 0x6101));
    alice.sees_leave(IDX18_VID);
    alice.quiet("one removal");
}

/// `sys.world.view`: a step lands at once (`Sync`) and never recomputes the view
/// (`G/input_main.cpp:1859-1868`), while a `Goto` does at its next sample. Yankee steps from row
/// 147 into row 146, 7591 from Alpha in a shared 3x3, and neither is shown the other; Yankee's
/// next walk is, at its first sample.
#[test]
fn a_step_never_recomputes_the_view_and_a_goto_does_at_its_sample() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = view_cast(&database, [GOTO_TARGET, ROW_147, VIEW_C], 12_345);
    let (mut alice, alpha, alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    let yankee_id = yankee_listed.id;

    yankee.send_record(&client_move(FUNC_COMBO, 0, 0, 444_100, 940_000, 0x6200));
    yankee.quiet("a step shows the stepper nobody");
    alice.quiet("and nobody the stepper");

    let dest = (444_100, 938_400);
    yankee.send_record(&client_move(FUNC_MOVE, 0, 0, dest.0, dest.1, 0x6201));
    let (add, info) = alice.sees_arrive(yankee_id, false);
    let (x, y) = inserted_at(&add);
    assert_eq!(x, 444_100);
    assert!((dest.1..=940_000).contains(&y), "the live point: {y}");
    assert_eq!(info, yankee_entered.info);
    assert_insert_move(&alice.read_game(), yankee_id, dest, LONG_WALK_MS);
    assert_eq!(alice.read_game(), walk_mode_of(&yankee_id.to_le_bytes()));
    let (add, info) = yankee.sees_arrive(alpha.id, false);
    assert_eq!(insert_body(&add), insert_body(&alpha_entered.add));
    assert_eq!(info, alpha_entered.info);
    alice.quiet("one insert");
    yankee.quiet("one insert");
}

/// `sys.chat` with `sys.world.view`: a talking line reaches the whole map
/// (`FEmpireChatPacket`), so Charlie, out of every view, hears Alpha too.
#[test]
fn talking_chat_reaches_the_whole_map_including_players_out_of_view() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = view_cast(&database, [GOTO_TARGET, VIEW_B, VIEW_C], 12_345);
    let (mut alice, _alpha, _) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, false);
    let (mut charlie, _, _) = enter_world_seeing(&server, b"carol", 0);

    alice.send_record(&client_chat(CHAT_TALKING, b"hello"));
    let own = alice.read_game();
    assert_eq!(own[0], GC_CHAT);
    assert_eq!(&own[10..], b"Alpha : hello");
    assert_eq!(yankee.read_game(), own, "Yankee, in view");
    assert_eq!(charlie.read_game(), own, "Charlie, 13228 away");
    for (client, note) in [
        (&mut alice, "Alpha"),
        (&mut yankee, "Yankee"),
        (&mut charlie, "Charlie"),
    ] {
        client.quiet(note);
    }
}

/// `sys.world.view`: a player who leaves is removed from each viewer (`CEntity::Destroy`'s
/// `ViewCleanup`, `G/entity.cpp:33-40`, `G/entity_view.cpp:8-21`) and from nobody else.
#[test]
fn a_leaving_player_is_removed_from_its_viewers_only() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = view_cast(&database, [GOTO_TARGET, VIEW_B, VIEW_C], 12_345);
    let (mut alice, alpha, _) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, false);
    let (mut charlie, _, _) = enter_world_seeing(&server, b"carol", 0);
    alice.quiet("Charlie is out of view");

    drop(alice);
    server.wait_for("Character left the world");
    yankee.sees_leave(alpha.id);
    yankee.quiet("one removal");
    charlie.quiet("Charlie never saw Alpha");
}

/// `sys.world.view`: a walking player's walk mode rides with its insert. Each entrant at no
/// stamina is sent its own walk mode after its own pair; the entrant's insert to a viewer sends
/// the viewer's walk mode back after the viewer's pair, and the viewer's insert to the entrant
/// sends the entrant's walk mode first (`G/char.cpp:1225-1236`).
#[test]
fn at_zero_stamina_the_walk_mode_rides_with_each_insert() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = view_cast(&database, [GOTO_TARGET, VIEW_B, VIEW_C], 0);
    let (mut alice, alpha, alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    assert!(alice.walking, "the burst carried no stamina");
    let (mut yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    let seen = &yankee_entered.shown.records;
    assert_eq!(
        seen.len(),
        5,
        "idx18's pair, Alpha's, and Alpha's walk mode"
    );
    assert_idx18(&seen[0]);
    assert_eq!(&seen[2][1..5], &alpha.id.to_le_bytes());
    assert_eq!(seen[3], alpha_entered.info);
    assert_eq!(seen[4], walk_mode_of(&alpha.id.to_le_bytes()));
    alice.sees_arrive(yankee_listed.id, true);
    alice.quiet("Yankee's walk mode and pair alone");
    yankee.quiet("Yankee's burst is over");
}

/// A `CG_ENTER_GAME` in the game phase is in the main table and its handler does nothing
/// (`G/input_main.cpp:4126-4127`), so it is consumed without a record, it shows nobody again,
/// and the connection keeps answering.
#[test]
fn a_game_phase_enter_game_is_consumed_silently() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = view_cast(&database, [GOTO_TARGET, VIEW_B, VIEW_C], 12_345);
    let (mut alice, _alpha, _) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, false);

    alice.unanswered(&client_enter_game());
    yankee.quiet("Alpha is not shown again");
    alice.send_record(&client_chat(CHAT_TALKING, b"still here"));
    let own = alice.read_game();
    assert_eq!(&own[10..], b"Alpha : still here");
    assert_eq!(yankee.read_game(), own);
}

/// `FindCharacter` looks a sync victim up in the world, and a disconnect removes the
/// character from the world, so the lookup returns null and the element is skipped
/// (`G/input_main.cpp:2060-2063`). A claim naming a character who has gone must therefore
/// produce no record at all.
///
/// This pins the behaviour, not the bookkeeping. It passes whether or not the descriptor's
/// exit path forgets the position, because every claim lookup also filters through the
/// Channel registry, which has already dropped the lease; see the mutant note in ledger 188.
#[test]
fn a_claim_on_a_character_who_has_left_finds_nobody() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         470000, 950000 FROM account WHERE login = 'bob'",
    );

    // Alice's connection is dropped at the end of the block, so the claim below is made by
    // a descriptor that outlives hers.
    let alpha = {
        let (_alice, listed) = enter_world(&server, b"alice", 0);
        listed.id
    };
    // The leave reaches the game thread after the socket closes, so Yankee enters once the
    // world has let Alpha go, and is never shown it.
    server.wait_for("Character left the world");
    let (mut yankee, _yankee_id) = enter_world(&server, b"bob", 0);

    // `if (!victim) continue;` skips an element whose VID is not in the world, so the batch
    // ends up empty and no record is written at all.
    yankee.unanswered(&client_sync_position(&[(alpha, 470_010, 950_010)]));
}

/// `sys.char.save`: a character that walks and then disconnects has its new position in the
/// row, because `CHARACTER::Disconnect` flushes the queued save and, when nothing was queued,
/// calls `SaveReal` itself (`G/char.cpp:1786-1789`).
///
/// Nothing saves while the descriptor lives. Legacy's first save is one full
/// `save_event_second_cycle` after `CInputLogin::Entergame` calls `StartSaveEvent`
/// (`G/input_login.cpp:656`), and the default cycle is 120 seconds, so a scenario that asserts
/// an unchanged row is asserting the arming delay and not a missing write.
#[test]
fn a_logout_writes_where_the_character_stands() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    check(
        &database,
        "(SELECT x FROM player WHERE name = 'Alpha') = 470000",
    );
    check(
        &database,
        "(SELECT y FROM player WHERE name = 'Alpha') = 950000",
    );

    let (mut alice, _alpha) = enter_world(&server, b"alice", 0);
    // `FUNC_COMBO` is 3, the branch that steps without a duration.
    alice.send_record(&client_move(3, 7, 40, 470_100, 950_100, 0x5eed));
    alice.quiet("PacketAround excludes the mover");
    // The row is not written while the descriptor lives: the event is armed a full cycle out.
    check(
        &database,
        "(SELECT x FROM player WHERE name = 'Alpha') = 470000",
    );

    // The disconnect is what writes it. `drop` closes the socket; the server reads the EOF,
    // breaks the read loop, and saves before the descriptor is torn down.
    drop(alice);
    wait_for(
        &database,
        "(SELECT x FROM player WHERE name = 'Alpha') = 470100",
    );
    wait_for(
        &database,
        "(SELECT y FROM player WHERE name = 'Alpha') = 950100",
    );

    // And the row is committed, so the next login enters the game at the position the character
    // was left at rather than the one it was created at.
    let (mut again, _) = enter_world(&server, b"alice", 0);
    again.send_record(&client_move(3, 7, 0, 470_200, 950_200, 0x5eee));
    again.quiet("the reloaded character is in the game at the saved position");
    drop(again);
}

/// `sys.char.save`: the row is written by the save event too, not only at logout. This is
/// ADR-0003's "in the background at most a few seconds late", with the cycle shortened to a
/// second so a scenario can watch it.
#[test]
fn the_save_cycle_writes_the_row_while_the_character_is_still_connected() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "save_event_second_cycle = 1",
    );
    create_account(&server, "alice");
    add_characters(&database);

    let (mut alice, _alpha) = enter_world(&server, b"alice", 0);
    alice.send_record(&client_move(3, 7, 40, 470_100, 950_100, 0x5eed));
    alice.quiet("the mover is excluded from its own broadcast");
    wait_for(
        &database,
        "(SELECT x FROM player WHERE name = 'Alpha') = 470100",
    );
    // The descriptor is still open: the write came from the event, not from a disconnect.
    let (bytes, state) = alice.client.drain(std::time::Duration::from_millis(200));
    assert!(bytes.is_empty(), "a save sends no record: {bytes:02x?}");
    assert_eq!(
        state,
        parity::client::Quiet::Open,
        "the client is still connected"
    );
    drop(alice);
}

/// ADR-0002's "all worlds step on one game thread" is only true of a live client if the
/// character actually reaches that world. Before ledger 205 a live client was in the
/// broadcast set and the position table and nowhere else, so the game thread held no
/// character and nothing addressed to one could reach it.
///
/// The log lines are the evidence rather than an internal handle, because the claim under
/// test is that the live path crosses. An unlogged crossing would leave a client playing on
/// a world that has no record of it, and a grant would find nobody.
#[test]
fn a_live_client_joins_and_leaves_the_game_threads_world() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);

    // Entering: the world is told, and told once, before the client sees the game phase.
    let (alice, alpha) = enter_world(&server, b"alice", 0);
    server.wait_for("Character entered the world on the game thread");
    assert!(
        server.logged("Character entered the world on the game thread"),
        "a live client must reach the game thread's world:\n{}",
        server.console(),
    );
    assert!(
        server.logged("vid=1 name=Alpha"),
        "the world must be told which character it took, and under which VID:\n{}",
        server.console(),
    );

    // The VID the world was given is the character's own id, which is the number the
    // enter-game burst already published. A world that allocated its own would hold the
    // character under an identity the client has never been told about, and a record the
    // world wrote would reach a client that is not listening for it.
    assert_eq!(alpha.id, 1, "the scenario's first character is player id 1");

    // Leaving: the drop closes the socket, the server reads the EOF, and the descriptor
    // takes the character out of the world before it writes the row.
    drop(alice);
    server.wait_for("Character left the world");

    // And the name is free again, which is what makes a relog possible at all. Legacy
    // reuses a name the instant `PlayerDestroy` runs; a world that held the departed
    // character would refuse the new login as a duplicate, and a player who logged out
    // could not log back in.
    let (_again, _same) = enter_world(&server, b"alice", 0);
    assert!(
        server.logged("Character entered the world on the game thread"),
        "the relogged character must reach the world too:\n{}",
        server.console(),
    );
}

// ---------------------------------------------------------------------------
// `sys.item.core`: the create-and-destroy round trip, end to end, against the
// real binary with a real Operator.
// ---------------------------------------------------------------------------

/// A 72-byte `GC_ITEM_SET` and its header byte, as the client sees them.
///
/// Byte 21 is the 72-byte record. Byte 20 is the byte legacy calls `GC_ITEM_DEL` and
/// the client calls `HEADER_GC_ITEM_SET`, so the Rewrite clears a cell with a byte-21
/// record whose vnum is 0 (ledger 211.3). `protocol::gc_item_window` has the full rename
/// and the widths.
const ITEM_SET_LEN: usize = 72;
const ITEM_SET: u8 = 21;
/// `GC_ITEM_UPDATE` (byte 25, 59 bytes): a stack that changed in place.
const ITEM_UPDATE_LEN: usize = 59;
const ITEM_UPDATE: u8 = 25;
/// `GC_ITEM_GROUND_ADD`: the header, `x`, `y`, `z`, the VID and the vnum.
const GROUND_ADD: u8 = 0x1a;
const GROUND_ADD_LEN: usize = 21;
/// `GC_ITEM_GROUND_DEL`: the header and the VID.
const GROUND_DEL: u8 = 0x1b;
const GROUND_DEL_LEN: usize = 5;

/// The width of a record an Operator's item work puts in a client's window.
///
/// Only the item window records are here, because only those are what an Operator's grant
/// and destroy produce. A header outside this set panics, on the grounds that a record
/// nobody accounted for in the window is a finding.
fn an_item_window_record(header: u8) -> usize {
    match header {
        ITEM_SET => ITEM_SET_LEN,
        other => panic!(
            "a record nobody accounted for arrived in the window: header {other}. A grant \
             sends exactly one GC_ITEM_SET, so this is not a burst a scenario can ignore."
        ),
    }
}

/// The first `GC_ITEM_SET` in `records`, or `None`.
///
/// `records` is what [`Keyed::drain_game`] decrypted, so this is a search over records
/// rather than over bytes: a `chunks(72)` sweep over a ciphertext stream would find
/// nothing at all, and finding nothing is what a missing grant looks like too.
fn the_item_set(records: &[Vec<u8>]) -> Option<&[u8]> {
    records
        .iter()
        .find(|record| record.len() == ITEM_SET_LEN && record[0] == ITEM_SET)
        .map(Vec::as_slice)
}

/// The vnum this section grants, read out of the owner's prototypes before anything
/// is asserted about it.
///
/// Without this, a scenario that granted nothing would satisfy every assertion about the
/// store and the client's bytes by having nothing to look at, and the round trip would be
/// recorded as ported on the strength of an empty run. The negative control is in the
/// same sweep: the walk stops as soon as it sees a first column it cannot account for, so
/// a sweep that matched everything would fail rather than pass.
const GRANTED_VNUM: u32 = 19;

#[test]
fn the_vnum_this_section_grants_is_in_the_owners_prototypes() {
    // Read through the same reader the server uses, not through the file. The proto name
    // column is Korean in a legacy code page, so a `read_to_string` here fails outright,
    // and a test that only worked for UTF-8 Game data would be a test that stops working
    // the day this file is checked properly. `ItemProtos::load` is what the server asks,
    // so this asserts the server can grant this vnum rather than that a file parses.
    let protos = owners_protos();
    assert!(
        protos.get(a_plain_vnum()).is_some(),
        "vnum {GRANTED_VNUM} should be in the owner's item prototypes, or this section \
         grants nothing and proves nothing"
    );
    // The negative control, in the same table: a vnum nothing carries is not a vnum the
    // reader invented, so `contains` is a lookup and not a tautology.
    assert!(
        protos.get(0).is_none(),
        "vnum 0 is not a prototype, so a reader that answered `true` for everything would \
         be caught here"
    );
}

/// The owner's item prototypes, read by the reader the server uses.
fn owners_protos() -> gamedata::item_proto::ItemProtos {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
    gamedata::item_proto::ItemProtos::load(&directory)
        .expect("the owner's item prototypes should load")
}

/// An item vnum the owner's protos really carry, so the grant is not refused for the wrong
/// reason. [`the_vnum_this_section_grants_is_in_the_owners_prototypes`] checks it.
fn a_plain_vnum() -> u32 {
    GRANTED_VNUM
}

/// The rows one character's items, as `vnum:count` text, or `"none"` for no rows.
///
/// Read through the store rather than through the world, because the row is the fact a
/// relog depends on and the world is only where the item lives until the descriptor goes.
/// The rendering is `id:vnum` so a scenario compares one thing at a time.
fn items_of(database: &ScratchDatabase, name: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for (id, vnum) in rows_of(
        database,
        &format!(
            "SELECT id, vnum FROM item WHERE owner_id = (SELECT id FROM player WHERE name = \
             '{name}') ORDER BY id"
        ),
    ) {
        out.push(format!("{id}:{vnum}"));
    }
    if out.is_empty() {
        "none".to_owned()
    } else {
        out.join(" ")
    }
}

/// The `(int, int)` columns of a `SELECT`, one pair per row.
///
/// Two columns is the most a scenario needs to identify a row and check it, and a fixed
/// shape keeps the helper honest: a scenario that wants a third column says so.
fn rows_of(database: &ScratchDatabase, statement: &str) -> Vec<(i64, i64)> {
    support::rows(database.url(), statement, 2)
        .into_iter()
        .map(|mut cells| (cells.remove(0), cells.remove(0)))
        .collect()
}

/// The Operator gives an online character an item, and the character's client is told.
///
/// This is the create half of the round trip, and it is the first scenario in which an
/// item crosses the whole path at once: an Operator writes to a pipe, the console asks the
/// game thread, the game thread places the item and chooses a cell, the store writes the
/// row, and only then does the record reach the client's socket. The client's own
/// decrypted bytes and the store are the evidence, because the claim under test is that
/// the crossing happens and in that order.
#[test]
fn an_operator_gives_an_online_character_an_item_and_the_client_is_told() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    create_account(&server, "alice");
    add_characters(&database);

    // A live client, so the world has a character to grant to.
    let (mut alpha, _character) = enter_world(&server, b"alice", 0);

    // When: an Operator writes one command into the console's pipe.
    Server::write_console(&console, &format!("item give Alpha {}", a_plain_vnum()));
    server.wait_for("the client has it");

    // Then: the row is in the store, which is the fact a relog depends on.
    assert_eq!(
        items_of(&database, "Alpha"),
        format!("100000000:{}", a_plain_vnum()),
        "the granted item's row must be in the store, under the item that was granted"
    );

    // And: the client was told, over its own socket. Read off the wire rather than taken
    // from the console's answer, because a record that reached the log and not the socket
    // would satisfy every line above.
    let (records, state) = alpha.drain_game(Duration::from_millis(500), an_item_window_record);
    let record = the_item_set(&records)
        .unwrap_or_else(|| panic!("the client must receive a GC_ITEM_SET, got {records:02x?}"));
    // The vnum the Operator asked for is the vnum on the wire. The offsets are counted
    // from the struct: the 1-byte header, the 3-byte `TItemPos` cell, then the
    // little-endian `DWORD` vnum. `protocol::gc_item_window` transcribes the C.
    assert_eq!(
        &record[4..8],
        &a_plain_vnum().to_le_bytes(),
        "the vnum on the wire must be the one the Operator typed"
    );
    assert_eq!(
        &record[8..10],
        &1_u16.to_le_bytes(),
        "a grant with no count puts exactly one item in the cell"
    );
    assert_eq!(
        record[1],
        common::item_slots::EWindows::Inventory as u8,
        "a grant goes into the base inventory"
    );
    assert_eq!(
        state,
        Quiet::Open,
        "the client is still connected after the grant"
    );
}

/// An item an Operator created is still in the store after the client disconnects.
///
/// This is the persistence half of the round trip. Legacy saves an item in the background
/// and so does the Rewrite (ADR-0003), so the claim is not that the row is written at the
/// moment of the grant but that it is still there once the descriptor that received the
/// record is gone, which is the only version of the claim a player would experience as
/// durability.
#[test]
fn a_granted_item_is_still_in_the_store_after_the_client_disconnects() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    create_account(&server, "alice");
    add_characters(&database);

    let (mut alpha, _character) = enter_world(&server, b"alice", 0);
    Server::write_console(&console, &format!("item give Alpha {}", a_plain_vnum()));
    server.wait_for("the client has it");
    let _ = alpha.drain_game(Duration::from_millis(500), an_item_window_record);

    // When: the client goes away. `drop` closes the socket, the server reads the EOF, and
    // the character leaves the world before the final save.
    drop(alpha);
    server.wait_for("Character left the world");

    // Then: the row is still there, which is what a relog would load.
    assert_eq!(
        items_of(&database, "Alpha"),
        format!("100000000:{}", a_plain_vnum()),
        "a granted item must outlive the connection that received it"
    );
}

/// An Operator destroys an item, and the row goes with the world holding nothing.
///
/// The destroy half. The order is the claim: the world is asked first, because only the
/// world knows which cell an id occupies, and the row is deleted only after it has
/// answered. A destroy that deleted first would remove the row of an item a live character
/// still holds, and that item would vanish at the next login with nothing anywhere
/// recording that it had existed.
#[test]
fn an_operator_destroy_takes_the_item_out_of_the_world_and_deletes_its_row() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    create_account(&server, "alice");
    add_characters(&database);

    let (mut alpha, _character) = enter_world(&server, b"alice", 0);
    Server::write_console(&console, &format!("item give Alpha {}", a_plain_vnum()));
    server.wait_for("the client has it");
    let _ = alpha.drain_game(Duration::from_millis(500), an_item_window_record);
    let (id, _) = rows_of(
        &database,
        "SELECT id, vnum FROM item WHERE owner_id = (SELECT id FROM player WHERE name = 'Alpha')",
    )[0];

    // When: the Operator destroys that exact id.
    Server::write_console(&console, &format!("item destroy Alpha {id}"));
    server.wait_for("deleted its row");

    // Then: the row is gone.
    assert_eq!(
        items_of(&database, "Alpha"),
        "none",
        "a destroyed item must not come back at the next login"
    );

    // And: a second destroy of the same id is refused by the world and never reaches the
    // store, which is what "the world was asked first" looks like from outside. The row
    // count is checked again, because a refusal that still deleted a row would be the
    // failure this ordering exists to prevent.
    Server::write_console(&console, &format!("item destroy Alpha {id}"));
    server.wait_for("The destroy did not happen");
    let console_text = server.console();
    assert!(
        console_text.contains("the row was left alone"),
        "the refusal should say the row was not touched:\n{console_text}"
    );
    assert_eq!(items_of(&database, "Alpha"), "none", "still no row");

    // And the client was told once, with the byte-21 clear for the freed cell and not
    // legacy's byte 20, which the stock client frames at another width and drops. The
    // refused second destroy sent nothing.
    let (records, state) = alpha.drain_game(Duration::from_millis(300), an_item_window_record);
    assert_eq!(
        records,
        vec![a_clear_record(0)],
        "one clear for cell 0, and nothing for the refused destroy"
    );
    assert!(
        server.console().contains("cleared the cell on the client"),
        "the console says the client was told"
    );
    assert_eq!(
        state,
        Quiet::Open,
        "the client is still connected after the destroy"
    );
}

/// The byte-21 record that tells the client inventory `cell` is empty: vnum 0 and every
/// other field zero (ledger 211.3).
fn a_clear_record(cell: u16) -> Vec<u8> {
    let mut record = vec![ITEM_SET, common::item_slots::EWindows::Inventory as u8];
    record.extend_from_slice(&cell.to_le_bytes());
    record.resize(ITEM_SET_LEN, 0);
    record
}

/// A `CG_ITEM_MOVE` between two base-inventory cells.
fn client_item_move(from: u16, to: u16, count: u16) -> Vec<u8> {
    let inventory = common::item_slots::EWindows::Inventory as u8;
    protocol::cg_item_move::CgItemMove::new(
        protocol::item_pos::ItemPos::new(inventory, from),
        protocol::item_pos::ItemPos::new(inventory, to),
        count,
    )
    .encode()
}

/// `CG_ITEM_USE` of the inventory cell `cell`.
fn client_item_use(cell: u16) -> Vec<u8> {
    let inventory = common::item_slots::EWindows::Inventory as u8;
    protocol::cg_item_use::CgItemUse::new(protocol::item_pos::ItemPos::new(inventory, cell))
        .encode()
}

/// A one-cell stackable vnum in no custom bank, so a grant of it lands in the base inventory
/// and a move of it can split and merge.
fn a_stackable_vnum(protos: &gamedata::item_proto::ItemProtos) -> u32 {
    protos
        .rows()
        .iter()
        .find(|proto| is_stackable_outside_banks(proto))
        .map(|proto| proto.vnum)
        .expect("the owner's data has a stackable one-cell item outside every bank")
}

/// [`a_stackable_vnum`]'s kind of item that a quickslot also takes: an `ITEM_USE` one
/// (`G/input_main.cpp:1083-1113`).
fn a_stackable_use_vnum(protos: &gamedata::item_proto::ItemProtos) -> u32 {
    protos
        .rows()
        .iter()
        .find(|proto| {
            is_stackable_outside_banks(proto) && proto.item_type == gamedata::item_kind::ITEM_USE
        })
        .map(|proto| proto.vnum)
        .expect("the owner's data has a stackable one-cell use item outside every bank")
}

/// A one-cell item that stacks, in no custom bank.
fn is_stackable_outside_banks(proto: &gamedata::item_proto::ItemProto) -> bool {
    proto.size == 1
        && proto.flags & world::item::ITEM_FLAG_STACKABLE != 0
        && proto.anti_flags & world::item::ITEM_ANTIFLAG_STACK == 0
        && (0..6)
            .all(|category| !gamedata::item_custom_category::is_custom_category(proto, category))
}

/// The window byte, cell, vnum and count of a `GC_ITEM_SET`.
fn set_fields(record: &[u8]) -> (u8, u16, u32, u16) {
    assert_eq!(record.len(), ITEM_SET_LEN);
    assert_eq!(record[0], ITEM_SET);
    (
        record[1],
        u16::from_le_bytes([record[2], record[3]]),
        u32::from_le_bytes([record[4], record[5], record[6], record[7]]),
        u16::from_le_bytes([record[8], record[9]]),
    )
}

/// The window byte, cell and count of a `GC_ITEM_UPDATE`.
fn update_fields(record: &[u8]) -> (u8, u16, u16) {
    assert_eq!(record.len(), ITEM_UPDATE_LEN);
    assert_eq!(record[0], ITEM_UPDATE);
    (
        record[1],
        u16::from_le_bytes([record[2], record[3]]),
        u16::from_le_bytes([record[4], record[5]]),
    )
}

/// `cg.game.item_move`: a move, a split and a merge reach the client and the store, a refused
/// move is told why when legacy says why, and a relog finds every item where it was moved.
///
/// Each answer is read off the client's own socket, and the store is checked as soon as the
/// records arrive, because the descriptor commits the rows before it sends the records.
#[test]
fn an_item_is_moved_split_and_merged_and_a_relog_finds_it_there() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    create_account(&server, "alice");
    add_characters(&database);
    let protos = owners_protos();
    let stackable = a_stackable_vnum(&protos);
    let inventory = common::item_slots::EWindows::Inventory as u8;

    // Given: the two-cell vnum 19 at cell 0, which also covers cell 5, and ten of a stackable
    // item at cell 1.
    let (mut alpha, _character) = enter_world(&server, b"alice", 0);
    Server::write_console(&console, &format!("item give Alpha {}", a_plain_vnum()));
    server.wait_for("id 100000000; the client has it");
    assert_eq!(set_fields(&alpha.read_game()).1, 0, "the grant took cell 0");
    Server::write_console(&console, &format!("item give Alpha {stackable} 10"));
    server.wait_for("id 100000001; the client has it");
    assert_eq!(
        set_fields(&alpha.read_game()),
        (inventory, 1, stackable, 10)
    );

    // When: the two-cell item is moved from cell 0 to cell 2.
    alpha.send_record(&client_item_move(0, 2, 0));
    // Then: the old cell is cleared first, then the new one is set, and the row is already
    // at cell 2.
    assert_eq!(alpha.read_game(), a_clear_record(0));
    assert_eq!(
        set_fields(&alpha.read_game()),
        (inventory, 2, a_plain_vnum(), 1)
    );
    check(
        &database,
        "EXISTS (SELECT 1 FROM item WHERE id = 100000000 AND window_type = 1 AND pos = 2)",
    );

    // When: four of the stack are split off onto cell 3.
    alpha.send_record(&client_item_move(1, 3, 4));
    // Then: the source is updated to six, then the new stack of four is set, and both rows
    // are stored: the source's new count and a new row under the next id.
    assert_eq!(update_fields(&alpha.read_game()), (inventory, 1, 6));
    assert_eq!(set_fields(&alpha.read_game()), (inventory, 3, stackable, 4));
    check(
        &database,
        "(SELECT count FROM item WHERE id = 100000001) = 6 AND EXISTS (SELECT 1 FROM item          WHERE id = 100000002 AND window_type = 1 AND pos = 3 AND count = 4)",
    );

    // When: the four are merged back onto the six.
    alpha.send_record(&client_item_move(3, 1, 0));
    // Then: the used-up stack is cleared, the target is updated to ten, and the used-up row
    // is gone.
    assert_eq!(alpha.read_game(), a_clear_record(3));
    assert_eq!(update_fields(&alpha.read_game()), (inventory, 1, 10));
    check(
        &database,
        "(SELECT count FROM item WHERE id = 100000001) = 10 AND NOT EXISTS (SELECT 1 FROM item          WHERE id = 100000002)",
    );

    // When: a sword is moved into a belt cell.
    alpha.send_record(&client_item_move(2, 274, 0));
    // Then: legacy's info line comes back (`G/char_item.cpp:7698`) with the character's
    // empire, and nothing moved.
    let mut notice = vec![GC_CHAT, 19, 0, 1, 0, 0, 0, 0, 1, 1];
    notice.extend_from_slice(b"[LS;1097]");
    assert_eq!(alpha.read_game(), notice);
    // And: a move from an empty cell is refused with no record, as legacy's is.
    alpha.unanswered(&client_item_move(40, 41, 0));
    check(
        &database,
        "EXISTS (SELECT 1 FROM item WHERE id = 100000000 AND window_type = 1 AND pos = 2)",
    );

    // When: the character is selected again.
    drop(alpha);
    server.wait_for("Character left the world");
    let (_alpha, _character, items) = load_character(&server, b"alice", 0);
    // Then: both items come back where they were moved, and the merged stack is whole.
    let mut loaded: Vec<(u8, u16, u32, u16)> =
        items.iter().map(|record| set_fields(record)).collect();
    loaded.sort_unstable();
    assert_eq!(
        loaded,
        vec![
            (inventory, 1, stackable, 10),
            (inventory, 2, a_plain_vnum(), 1),
        ]
    );
}

/// A relogged character is sent its stored items in the loading phase, and the world holds
/// them once the game is entered.
///
/// This is the load half of the round trip: the only way a player sees an item a second
/// time. The rows come from two writers on purpose. One is the Operator's grant, the Rewrite's
/// own insert; two are written by hand, which is what a row from any other writer looks like.
/// The belt row checks legacy's window translation (`G/input_db.cpp:1491-1495`). The
/// overlapping row checks the set-aside path and its save: legacy moves an item whose cell is
/// taken to the first free cell and writes the new cell (`G/item.cpp:529`). The switchbot row
/// is the negative control: the switchbot is not loaded by this build, so its row must produce
/// no record and must still be in the store afterwards, and a load that sent a record for every
/// row would fail on the record count.
#[test]
fn a_relogged_character_is_sent_its_items_before_the_game_is_entered() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    create_account(&server, "alice");
    add_characters(&database);

    // Given: one granted item, and its client gone again.
    let (mut alpha, _character) = enter_world(&server, b"alice", 0);
    Server::write_console(&console, &format!("item give Alpha {}", a_plain_vnum()));
    server.wait_for("the client has it");
    let _ = alpha.drain_game(Duration::from_millis(500), an_item_window_record);
    drop(alpha);
    server.wait_for("Character left the world");

    // And: three rows written by hand, with ids below the grant range. Row 7 is at the fourth
    // cell of the belt window with a count of 2; row 8 is a switchbot row; row 9 is at cell 5
    // of the inventory, which the granted two-cell item's footprint already covers.
    let protos = owners_protos();
    let small = protos
        .rows()
        .iter()
        .find(|proto| proto.size == 1)
        .map(|proto| proto.vnum)
        .expect("the owner's data has a one-cell item");
    for (id, window, pos, count) in [(7, 9, 3, 2), (8, 8, 4, 1), (9, 1, 5, 1)] {
        sql(
            &database,
            &format!(
                "INSERT INTO item (id, owner_id, window_type, pos, count, vnum) SELECT {id}, id, \
                 {window}, {pos}, {count}, {small} FROM player WHERE name = 'Alpha'"
            ),
        );
    }
    // Row 7 also carries every stored field the record relays.
    let relayed = RelayedFields::distinct();
    relayed.store(&database, 7);

    // When: the character is selected again.
    let (mut alpha, _character, items) = load_character(&server, b"alice", 0);

    // Then: one `GC_ITEM_SET` for the granted item and one for the belt item, in the store's
    // window order, then one for the item that was set aside, and none for the switchbot row.
    assert_eq!(
        items.len(),
        3,
        "the granted, belt, and moved items, and nothing for the switchbot row: {items:02x?}"
    );
    let inventory = common::item_slots::EWindows::Inventory as u8;
    assert_the_granted_record(&items[0], &protos);
    // The belt item's record, whole. 277 is `BELT_INVENTORY_SLOT_START` plus 3.
    let small_proto = protos.get(small).expect("found above");
    let expected = relayed.item_set(small_proto, 277, 2);
    assert_eq!(
        items[1], expected,
        "the belt item, shown in the inventory window at 274 plus its cell, every stored \
         field relayed, the prototype's flags, and no highlight"
    );
    // The overlapping row is sent last, at the first free cell. The Rewrite sets aside any
    // overlap, where legacy tests only the anchor cell (ledger 210).
    let moved = &items[2];
    assert_eq!(moved[1], inventory, "the base inventory");
    assert_eq!(&moved[2..4], &1_u16.to_le_bytes(), "the first free cell");
    assert_eq!(&moved[4..8], &small.to_le_bytes(), "vnum");

    // And: the moved row was rewritten at its new cell before its record was sent, the
    // refused row is reported and kept as it was, and the belt row keeps its own window,
    // because legacy places it with saving off (`G/input_db.cpp:1480`).
    check(
        &database,
        "EXISTS (SELECT 1 FROM item WHERE id = 9 AND window_type = 1 AND pos = 1)",
    );
    server.wait_for("An item was not loaded; its row is kept");
    check(
        &database,
        "EXISTS (SELECT 1 FROM item WHERE id = 8 AND window_type = 8 AND pos = 4)",
    );
    check(
        &database,
        "EXISTS (SELECT 1 FROM item WHERE id = 7 AND window_type = 9 AND pos = 3)",
    );

    // When: the game is entered and the Operator grants the same vnum again.
    enter_game_burst(&mut alpha);
    Server::write_console(&console, &format!("item give Alpha {}", a_plain_vnum()));
    server.wait_for("id 100000001; the client has it");

    // Then: the new item goes to cell 2, because the world holds the loaded items on cells 0
    // and 1. A world admitted without its items would choose cell 0 again, and the store's
    // one-item-per-cell key would refuse the row.
    let (records, state) = alpha.drain_game(Duration::from_millis(500), an_item_window_record);
    let record = the_item_set(&records)
        .unwrap_or_else(|| panic!("the client must receive a GC_ITEM_SET, got {records:02x?}"));
    assert_eq!(
        &record[2..4],
        &2_u16.to_le_bytes(),
        "the first free cell beside the loaded items"
    );
    check(&database, "(SELECT pos FROM item WHERE id = 100000001) = 2");
    assert_eq!(state, Quiet::Open, "the client is still connected");
}

/// The body armour a level-9 character can wear: `LEVEL 9`, `value1` 21, `APPLY_MOV_SPEED -2`.
const WORN_ARMOUR: u32 = 11_810;
/// The fan any level can wear: `LEVEL 0`, `APPLY_ATT_SPEED 26`.
const WORN_FAN: u32 = 7_000;
/// The bell a level-9 character cannot wear: `LEVEL 10`.
const BELL: u32 = 5_000;
/// A two-cell body armour, stored past the wear cells.
const SPARE_ARMOUR: u32 = 11_804;
/// A dragon soul stone.
const STONE: u32 = 110_000;

/// `sys.item.core`, `sys.char.points`: a relogged character wears its stored equipment,
/// and the item load's points record counts it.
///
/// Alpha is lowered to level 9 and given five `EQUIPMENT` rows ([`give_alpha_equipment`]). The
/// armour in the body cell and the fan in the weapon cell are worn, which the client sees as the
/// inventory window at `INVENTORY_MAX_NUM` plus the cell (`G/char_item.cpp:658-670`). The bell
/// fails `CheckItemUseLevel` by one level and cell 64 is past the wear cells, so both are set
/// aside to the first free inventory cells and their rows rewritten (`G/input_db.cpp:1519-1531`
/// and `:1541-1561`). The dragon soul stone is refused, because its deck is not ported, and its
/// row is kept.
///
/// Every expected value is a hand sum of the owner's prototypes and `ComputeBattlePoints`
/// (`G/char.cpp:2769-2846`). The armour adds its `value1`, 21, and twice its `value5`, 0: the
/// defence grade is 9 + 18 x 4 / 5 + 21, the shown grade 9 + 18 + 21, the magic defence
/// 9 + (3 x 20 + 18) / 3 + 21 / 2. The armour's attribute adds 500 to the maximum hit points and
/// its prototype takes 2 from the movement speed; the fan's prototype adds 26 to the attack
/// speed. The stored 1500 hit points are above the item-less maximum but not the worn one, so
/// they are kept: the relog heal legacy's `ApplyPoint` would give is a Defect. The body armour
/// becomes the main part, which the logout save stores and the next list shows.
#[test]
fn a_relogged_character_wears_its_equipment_and_its_points_count_it() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    give_alpha_equipment(&database);

    // When: the character is selected.
    let (mut keyed, _empire, list) = select_screen(&server, b"alice");
    let alpha = listed(&list, 0);
    keyed.send_record(&client_select(0));
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_LOADING]);
    assert_eq!(keyed.read_game(), [GC_ENTITY, 3, 0]);
    assert_eq!(keyed.read_game()[0], GC_MAIN_CHARACTER2_EMPIRE);
    let gold = keyed.read_game();
    assert_eq!(gold[0], GC_CHARACTER_GOLD);
    let loaded = keyed.read_game();
    assert_eq!(loaded[0], GC_PLAYER_POINTS);
    keyed.loaded(&loaded);
    for (slot, value, what) in [
        (5, 1500, "POINT_HP, as stored"),
        (6, 1420, "POINT_MAX_HP, before the items: 700 + 18 x 40"),
        (16, 23, "POINT_DEF_GRADE, before the items: 9 + 14"),
        (17, 100, "POINT_ATT_SPEED, before the items"),
        (19, 100, "POINT_MOV_SPEED, before the items"),
        (20, 27, "POINT_CLIENT_DEF_GRADE, before the items: 9 + 18"),
        (23, 35, "POINT_MAGIC_DEF_GRADE, before the items: 9 + 26"),
    ] {
        assert_eq!(point_slot(&loaded, slot), value, "{what}");
    }
    assert_eq!(keyed.read_game()[0], GC_SKILL_LEVEL_NEW);

    // Then: the two worn items at their wear cells, in row order, then the two set-aside
    // items at the first free cells.
    let inventory = common::item_slots::EWindows::Inventory as u8;
    assert_eq!(
        loaded_item_sets(&mut keyed, 4),
        vec![
            (inventory, 180, WORN_ARMOUR, 1),
            (inventory, 184, WORN_FAN, 1),
            (inventory, 0, BELL, 1),
            (inventory, 1, SPARE_ARMOUR, 1),
        ],
        "worn at 180 plus the wear cell, then set aside in row order"
    );

    // And: `CheckMaximumPoints` lowers only the spell points, then `PointsPacket` sends the
    // worn points. Every other slot is the first record's.
    assert_eq!(keyed.read_game(), point_change(alpha.id, 7, 0, 600));
    assert_eq!(keyed.read_game(), gold, "the item load's gold record");
    let mut worn = loaded.clone();
    for (slot, value) in [
        (6, 1920),
        (7, 600),
        (16, 44),
        (17, 126),
        (19, 98),
        (20, 48),
        (23, 45),
    ] {
        worn = with_point_slot(&worn, slot, value);
    }
    assert_eq!(
        keyed.read_game(),
        worn,
        "the item load's points record: the maximum hit points with the attribute, the \
         defence grades with the armour, both speeds with the prototypes, and the stored \
         hit points kept"
    );
    assert_eq!(keyed.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));

    // And: the set-aside rows were rewritten at their new cells, and the worn rows and the
    // refused stone's row are as they were.
    check(
        &database,
        "(SELECT array_agg((window_type, pos) ORDER BY id)::text FROM item WHERE id \
         BETWEEN 20 AND 24) = '{\"(2,0)\",\"(1,0)\",\"(2,4)\",\"(1,1)\",\"(2,65)\"}'",
    );
    server.wait_for("An item was not loaded; its row is kept");
    assert!(
        server.logged("wearing it needs the dragon soul deck, which is not ported"),
        "the stone's refusal names the system"
    );

    // When: the game is entered. Then: both speeds are the worn ones, the armour is the main
    // part and the fan the weapon part, the hair and sash parts are the stored ones, and the
    // head and aura parts are 0.
    let (add, parts) = enter_game_with_parts(&mut keyed);
    assert_eq!(add[24], 98, "bMovingSpeed");
    assert_eq!(add[25], 126, "bAttackSpeed");
    let part = |vnum: u32| u16::try_from(vnum).expect("a part is a WORD");
    assert_eq!(
        parts,
        [part(WORN_ARMOUR), part(WORN_FAN), 0, 0xc3d4, 0xe5f6, 0]
    );

    // And: the logout save stores the main part and the kept pools, and the next list shows
    // the armour.
    drop(keyed);
    wait_for(
        &database,
        "(SELECT (part_main, hp, sp) = (11810, 1500, 600) FROM player WHERE name = 'Alpha')",
    );
    let (_keyed, _empire, list) = select_screen(&server, b"alice");
    assert_eq!(listed(&list, 0).main_part, part(WORN_ARMOUR));
}

/// `GC_CHARACTER_UPDATE` (byte 19, 55 bytes): a character whose look changed (`G/char.cpp:1277`).
const GC_CHARACTER_UPDATE: u8 = 19;
const CHARACTER_UPDATE_LEN: usize = protocol::gc_actors::GC_CHARACTER_UPDATE_WIRE_SIZE;
/// `GC_SEPCIAL_EFFECT` (byte 114): a `BYTE` type and a `DWORD` VID.
const GC_SPECIAL_EFFECT: u8 = 114;
const SPECIAL_EFFECT_LEN: usize = 6;

/// A plain body armour of the owner's data: `LEVEL 0`, `value1` 12, `value5` 18.
const SWAPPED_ARMOUR: u32 = 11_806;
/// A grade-1 sash, whose absorption legacy fixes at 1.
const SASH: u32 = 85_001;

/// One record of a move, as the scenario reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    /// A `GC_ITEM_SET` of the base window: the cell and vnum, vnum 0 for a clear.
    Set(u16, u32),
    /// A `GC_CHARACTER_POINT_CHANGE`: the kind and the new value.
    Point(u8, i64),
    /// A `GC_CHARACTER_UPDATE`: the six parts and the two speeds.
    Look([u16; 6], u8, u8),
    /// A `GC_SEPCIAL_EFFECT`: the type.
    Effect(u8),
    /// A `GC_ITEM_UPDATE` of the base window: the cell and the new count.
    Count(u16, u16),
}

/// Every record a move sends within a quiet window, each checked against the mover's VID.
fn read_a_move(keyed: &mut Keyed, vid: u32) -> Vec<Seen> {
    read_a_move_within(keyed, vid, Duration::from_millis(700))
}

/// [`read_a_move`] for a quiet window of `window`, for a move whose next tick comes soon after.
fn read_a_move_within(keyed: &mut Keyed, vid: u32, window: Duration) -> Vec<Seen> {
    let language = keyed.language;
    let (records, quiet) = keyed.drain_game(window, |header| match header {
        GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
        GC_SPECIAL_EFFECT => SPECIAL_EFFECT_LEN,
        other => game_len(other),
    });
    assert_eq!(quiet, Quiet::Open);
    records
        .iter()
        .map(|record| match record.first().copied().unwrap_or_default() {
            ITEM_SET => {
                let (window, cell, vnum, _count) = set_fields(record);
                assert_eq!(window, common::item_slots::EWindows::Inventory as u8);
                Seen::Set(cell, vnum)
            }
            ITEM_UPDATE => {
                let (window, cell, count) = update_fields(record);
                assert_eq!(window, common::item_slots::EWindows::Inventory as u8);
                Seen::Count(cell, count)
            }
            GC_POINT_CHANGE => {
                assert_eq!(&record[4..8], &vid.to_le_bytes(), "the mover's VID");
                let value = i64::from_le_bytes(record[17..25].try_into().expect("eight bytes"));
                Seen::Point(record[8], value)
            }
            GC_CHARACTER_UPDATE => {
                assert_eq!(&record[1..5], &vid.to_le_bytes(), "the mover's VID");
                let parts = std::array::from_fn(|index| {
                    let at = 5 + 2 * index;
                    u16::from_le_bytes([record[at], record[at + 1]])
                });
                // `UpdatePacket` sends the mover's descriptor's language (`G/char.cpp:1321`).
                assert_eq!(
                    record[54], language,
                    "bLanguage from the mover's descriptor"
                );
                Seen::Look(parts, record[17], record[18])
            }
            GC_SPECIAL_EFFECT => {
                assert_eq!(&record[2..6], &vid.to_le_bytes(), "the mover's VID");
                Seen::Effect(record[1])
            }
            other => panic!("a record nobody accounted for in a move: header {other}"),
        })
        .collect()
}

/// Give Alpha three `INVENTORY` rows, ids 30 to 32: the level-9 armour at cell 0, the level-0
/// armour at cell 1 and the grade-1 sash at cell 2. The prototype fields the scenario's hand
/// sums read are checked against the owner's data first.
fn give_alpha_wearables(database: &ScratchDatabase) {
    let protos = owners_protos();
    let proto = |vnum| protos.get(vnum).expect("the owner's data has it");
    assert_eq!(
        (proto(WORN_ARMOUR).values[1], proto(WORN_ARMOUR).values[5]),
        (21, 0)
    );
    assert_eq!(
        (
            proto(SWAPPED_ARMOUR).values[1],
            proto(SWAPPED_ARMOUR).values[5]
        ),
        (12, 18)
    );
    assert_eq!(proto(SWAPPED_ARMOUR).limits[0].value, 0);
    assert_eq!(
        (proto(SASH).item_type, proto(SASH).values[0]),
        (gamedata::item_kind::ITEM_COSTUME, 1),
        "a grade-1 sash"
    );
    for (id, pos, vnum) in [(30, 0, WORN_ARMOUR), (31, 1, SWAPPED_ARMOUR), (32, 2, SASH)] {
        sql(
            database,
            &format!(
                "INSERT INTO item (id, owner_id, window_type, pos, count, vnum) SELECT {id}, id, \
                 1, {pos}, 1, {vnum} FROM player WHERE name = 'Alpha'"
            ),
        );
    }
}

/// `sys.item.core`: an armour is worn, swapped for another and taken off, and a sash is worn,
/// through `CG_ITEM_MOVE`, and the client, the points and the store follow each step.
///
/// Alpha is given three `INVENTORY` rows, ids 30 to 32: the level-9 armour at cell 0, the
/// level-0 armour at cell 1 and a grade-1 sash at cell 2. `EquipItem` refuses a wear within
/// 1.5 s of the last attack or of the select (`G/char_item.cpp:8471-8477`), so the scenario
/// waits that out first. Every expected point is a hand sum of `ComputeBattlePoints`
/// (`G/char.cpp:2769-2846`) over Alpha's level 154 and 18 in the defence attribute: the
/// defence grade is 154 + 18 x 4 / 5 plus the armour's `value1` and twice its `value5`, the
/// shown grade 154 + 18 plus the same, and the magic defence grade 154 + (3 x 20 + 18) / 3 plus
/// half the armour's. Alice logs in in German, and each look carries her descriptor's language
/// (`G/char.cpp:1321`).
#[test]
fn an_armour_is_worn_swapped_and_taken_off_and_the_store_follows() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    give_alpha_wearables(&database);
    let (mut alpha, _character, quickslots, items) =
        load_with_quickslots_in(&server, b"alice", 0, GERMAN);
    assert!(quickslots.is_empty());
    let waited = std::time::Instant::now();
    assert_eq!(items.len(), 3);
    let add = enter_game_burst(&mut alpha);
    let vid = u32::from_le_bytes(add[1..5].try_into().expect("four bytes"));
    std::thread::sleep(Duration::from_millis(1600).saturating_sub(waited.elapsed()));

    // When: the level-9 armour is moved onto the body cell.
    alpha.send_record(&client_item_move(0, 180, 0));
    // Then: the cell is cleared and the wear cell set, the prototype's speed is taken, the
    // battle points are recomputed from 0 in legacy's order (a defence grade change carries
    // the shown grade with it, `G/char.cpp:4436-4441`), and the look is sent with the armour
    // as the main part. The row is in the body wear cell.
    let (hair, stored_sash) = (0xc3d4, 0xe5f6);
    let battle = |armour: i64| {
        vec![
            Seen::Point(18, 344),
            Seen::Point(20, 168 + armour),
            Seen::Point(16, 168 + armour),
            Seen::Point(20, 172 + armour),
            Seen::Point(22, 348),
            Seen::Point(23, 180 + armour / 2),
        ]
    };
    let look = |main: u32, sash: u16, speed: u8| {
        let main = u16::try_from(main).expect("a part is a WORD");
        Seen::Look([main, 0, 0, hair, sash, 0], speed, 100)
    };
    let mut expected = vec![
        Seen::Set(0, 0),
        Seen::Set(180, WORN_ARMOUR),
        Seen::Point(19, 98),
    ];
    expected.extend(battle(21));
    expected.push(look(WORN_ARMOUR, stored_sash, 98));
    assert_eq!(read_a_move(&mut alpha, vid), expected);
    check(
        &database,
        "(SELECT (window_type, pos) = (2, 0) FROM item WHERE id = 30)",
    );

    // When: the other armour is moved onto the free head cell.
    alpha.send_record(&client_item_move(1, 181, 0));
    // Then: `EquipItem` finds the body cell taken and swaps: the worn armour is taken off
    // (its speed back first, then its cell cleared and the points and look recomputed), the
    // new one is worn at the body cell, and the old one is set where the new one was.
    let mut expected = vec![Seen::Point(19, 100), Seen::Set(180, 0)];
    expected.extend(battle(0));
    expected.extend([look(0, stored_sash, 100), Seen::Set(1, 0)]);
    expected.extend([Seen::Set(180, SWAPPED_ARMOUR), Seen::Point(19, 100)]);
    expected.extend(battle(12 + 2 * 18));
    expected.extend([
        look(SWAPPED_ARMOUR, stored_sash, 100),
        Seen::Set(1, WORN_ARMOUR),
    ]);
    assert_eq!(read_a_move(&mut alpha, vid), expected);
    check(
        &database,
        "(SELECT array_agg((window_type, pos) ORDER BY id)::text FROM item WHERE id IN (30, 31)) \
         = '{\"(1,1)\",\"(2,0)\"}'",
    );

    // When: the sash is moved onto the sash wear cell.
    alpha.send_record(&client_item_move(2, 203, 0));
    // Then: it is worn, the sash part (the vnum less 85000, `G/item.cpp:1294`) is sent at once
    // and again after the recompute, and the sash effect follows (`G/char_item.cpp:8574-8578`). The row is in the sash wear cell.
    let mut expected = vec![
        Seen::Set(2, 0),
        Seen::Set(203, SASH),
        look(SWAPPED_ARMOUR, 1, 100),
    ];
    expected.extend(battle(48));
    expected.extend([look(SWAPPED_ARMOUR, 1, 100), Seen::Effect(26)]);
    assert_eq!(read_a_move(&mut alpha, vid), expected);
    check(
        &database,
        "(SELECT (window_type, pos) = (2, 23) FROM item WHERE id = 32)",
    );

    // When: the body armour is moved onto a free inventory cell.
    alpha.send_record(&client_item_move(180, 10, 0));
    // Then: it is taken off, the defence grades fall back to the bare ones, and it is set at
    // the cell asked for. The row follows it.
    let mut expected = vec![Seen::Point(19, 100), Seen::Set(180, 0)];
    expected.extend(battle(0));
    expected.extend([look(0, 1, 100), Seen::Set(10, SWAPPED_ARMOUR)]);
    assert_eq!(read_a_move(&mut alpha, vid), expected);
    check(
        &database,
        "(SELECT array_agg((window_type, pos) ORDER BY id)::text FROM item WHERE id BETWEEN 30 \
         AND 32) = '{\"(1,1)\",\"(1,10)\",\"(2,23)\"}'",
    );

    // And: the logout save stores the parts the moves left, bare body and the sash's part.
    drop(alpha);
    wait_for(
        &database,
        "(SELECT (part_main, part_sash) = (0, 1) FROM player WHERE name = 'Alpha')",
    );
}

/// Ledger 216: `CG_ITEM_USE` of an equippable item puts it on, and a use of the worn item takes
/// it off (`CHARACTER::UseItemEx`, `G/char_item.cpp:3040-3053`).
///
/// Alpha holds the rows of the move scenario above. A use names only the cell, so the wear cell
/// is the one `EquipItem` picks, and a worn item comes off to the first cell it fits.
#[test]
fn an_armour_is_used_on_swapped_and_used_off_and_the_store_follows() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    give_alpha_wearables(&database);
    let (mut alpha, _character, items) = load_character(&server, b"alice", 0);
    let waited = std::time::Instant::now();
    assert_eq!(items.len(), 3);
    let add = enter_game_burst(&mut alpha);
    let vid = u32::from_le_bytes(add[1..5].try_into().expect("four bytes"));
    std::thread::sleep(Duration::from_millis(1600).saturating_sub(waited.elapsed()));
    let (hair, stored_sash) = (0xc3d4, 0xe5f6);
    let battle = |armour: i64| {
        vec![
            Seen::Point(18, 344),
            Seen::Point(20, 168 + armour),
            Seen::Point(16, 168 + armour),
            Seen::Point(20, 172 + armour),
            Seen::Point(22, 348),
            Seen::Point(23, 180 + armour / 2),
        ]
    };
    let look = |main: u32, speed: u8| {
        let main = u16::try_from(main).expect("a part is a WORD");
        Seen::Look([main, 0, 0, hair, stored_sash, 0], speed, 100)
    };

    // When: the level-9 armour at cell 0 is used.
    alpha.send_record(&client_item_use(0));
    // Then: it is worn at the body cell, as the move onto that cell wears it.
    let mut expected = vec![
        Seen::Set(0, 0),
        Seen::Set(180, WORN_ARMOUR),
        Seen::Point(19, 98),
    ];
    expected.extend(battle(21));
    expected.push(look(WORN_ARMOUR, 98));
    assert_eq!(read_a_move(&mut alpha, vid), expected);
    check(
        &database,
        "(SELECT (window_type, pos) = (2, 0) FROM item WHERE id = 30)",
    );

    // When: the other armour at cell 1 is used.
    alpha.send_record(&client_item_use(1));
    // Then: the body cell is taken, so the worn armour is swapped out to cell 1.
    let mut expected = vec![Seen::Point(19, 100), Seen::Set(180, 0)];
    expected.extend(battle(0));
    expected.extend([look(0, 100), Seen::Set(1, 0)]);
    expected.extend([Seen::Set(180, SWAPPED_ARMOUR), Seen::Point(19, 100)]);
    expected.extend(battle(12 + 2 * 18));
    expected.extend([look(SWAPPED_ARMOUR, 100), Seen::Set(1, WORN_ARMOUR)]);
    assert_eq!(read_a_move(&mut alpha, vid), expected);

    // When: the worn armour is used.
    alpha.send_record(&client_item_use(180));
    // Then: it is taken off to the first cell it fits, cell 0, and the grades fall back.
    let mut expected = vec![Seen::Point(19, 100), Seen::Set(180, 0)];
    expected.extend(battle(0));
    expected.extend([look(0, 100), Seen::Set(0, SWAPPED_ARMOUR)]);
    assert_eq!(read_a_move(&mut alpha, vid), expected);
    check(
        &database,
        "(SELECT array_agg((window_type, pos) ORDER BY id)::text FROM item WHERE id IN (30, 31)) \
         = '{\"(1,1)\",\"(1,0)\"}'",
    );

    // And: the logout save stores the bare body.
    drop(alpha);
    wait_for(
        &database,
        "(SELECT part_main = 0 FROM player WHERE name = 'Alpha')",
    );
}

/// `cg.game.item_use` of a `USE_POTION`: each small red potion owes 300 hit points, a second
/// drunk while the first is owed adds to it, the last of the stack clears its cell, the affect
/// event pays 7% of the maximum each second on the schedule the character's entry started, and
/// the logout save keeps the paid hit points.
#[test]
fn a_drunk_potion_pays_its_hit_points_over_the_next_seconds_and_the_save_keeps_them() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    create_account(&server, "alice");
    add_characters(&database);
    sql(&database, "UPDATE player SET hp = 500 WHERE name = 'Alpha'");
    let inventory = common::item_slots::EWindows::Inventory as u8;
    let (hp, recovery) = (
        u8::try_from(common::point_slot::POINT_HP).expect("a point byte"),
        u8::try_from(common::point_slot::POINT_HP_RECOVERY).expect("a point byte"),
    );

    // Given: Alpha, at 500 hit points, holds two small red potions.
    let (mut alpha, _character, _items) = load_character(&server, b"alice", 0);
    let add = enter_game_burst(&mut alpha);
    let vid = u32::from_le_bytes(add[1..5].try_into().expect("four bytes"));
    Server::write_console(&console, "item give Alpha 27001 2");
    server.wait_for("id 100000000; the client has it");
    assert_eq!(set_fields(&alpha.read_game()), (inventory, 0, 27_001, 2));

    // When: both are drunk, one after the other.
    alpha.send_record(&client_item_use(0));
    alpha.send_record(&client_item_use(0));
    // Then: 300 hit points are owed for each, the red effect plays twice, the stack goes to
    // one and then its cell is cleared, and the row with it.
    let mut seen = read_a_move(&mut alpha, vid);
    // The potion joins the event that Alpha's entry started (`StartAffectEvent` returns when one
    // runs, `G/char_affect.cpp:238`), so the event's second is the entry's, not the drink's: its
    // first tick may already be in this window. A tick is the hit points and the owed recovery.
    let mut paid = 0;
    if seen.len() > 6 {
        let tick = seen.split_off(6);
        assert_eq!(
            tick,
            [Seen::Point(hp, 500 + 99), Seen::Point(recovery, 600 - 99)],
            "the first tick, inside the drink's window"
        );
        paid = 99;
    }
    assert_eq!(
        seen,
        [
            Seen::Point(recovery, 300),
            Seen::Effect(1),
            Seen::Count(0, 1),
            Seen::Point(recovery, 600),
            Seen::Effect(1),
            Seen::Set(0, 0),
        ]
    );
    check(&database, "NOT EXISTS (SELECT 1 FROM item)");

    // And: each second the event pays 99, which is 7% of Alpha's maximum, until the 600 are
    // paid.
    while paid < 600 {
        let step = (600 - paid).min(99);
        paid += step;
        let tick = read_a_tick(&mut alpha, vid);
        assert_eq!(
            tick,
            [
                Seen::Point(hp, 500 + paid),
                Seen::Point(recovery, 600 - paid)
            ]
        );
    }
    // And: then it stops.
    alpha.quiet_for(
        "the event has paid the 600 and ends",
        Duration::from_millis(1200),
    );

    // And: the logout save keeps what the event paid.
    drop(alpha);
    wait_for(
        &database,
        "(SELECT hp = 1100 FROM player WHERE name = 'Alpha')",
    );
}

/// `event.char_affect.affect_event` and the save cycle: with a one-second cycle, the row of a
/// character still connected is written with what the event paid, because the descriptor asks
/// the world for its points before each save.
#[test]
fn a_recovering_character_is_saved_with_what_the_event_paid_while_it_is_still_connected() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) = Server::start_with_console_configured(
        binary(),
        database.url(),
        &default_channels(),
        "save_event_second_cycle = 1",
    );
    create_account(&server, "alice");
    add_characters(&database);
    sql(&database, "UPDATE player SET hp = 500 WHERE name = 'Alpha'");
    let hp = u8::try_from(common::point_slot::POINT_HP).expect("a point byte");

    // Given: Alpha, at 500 hit points, drinks a small red potion.
    let (mut alpha, _character, _items) = load_character(&server, b"alice", 0);
    let add = enter_game_burst(&mut alpha);
    let vid = u32::from_le_bytes(add[1..5].try_into().expect("four bytes"));
    Server::write_console(&console, "item give Alpha 27001 1");
    server.wait_for("id 100000000; the client has it");
    let _set = alpha.read_game();
    alpha.send_record(&client_item_use(0));
    let _used = read_a_move(&mut alpha, vid);

    // When: the event has paid all 300.
    let mut last = Seen::Point(hp, 500);
    while last != Seen::Point(hp, 800) {
        last = read_a_tick(&mut alpha, vid)[0].clone();
    }
    // Then: a save while the client is still connected stores 800.
    wait_for(
        &database,
        "(SELECT hp = 800 FROM player WHERE name = 'Alpha')",
    );
    let (bytes, quiet) = alpha.client.drain(Duration::from_millis(200));
    assert!(bytes.is_empty(), "a save sends no record: {bytes:02x?}");
    assert_eq!(quiet, Quiet::Open, "the client is still connected");
}

/// A `CG_QUICKSLOT_ADD` of `kind` and `pos` into `slot`.
fn client_quickslot_add(slot: u8, kind: u8, pos: u8) -> Vec<u8> {
    protocol::cg_quickslot_add::CgQuickslotAdd::new(
        slot,
        protocol::TQuickslot {
            b_type: kind,
            b_pos: pos,
        },
    )
    .encode()
}

/// `cg.game.quickslot_add`, `cg.game.quickslot_del`, `cg.game.quickslot_swap`: a potion, a
/// skill and a command are put on the bar, an empty cell is refused, a second copy of the skill
/// empties the first, a swap and a delete are told, and a relog sets the stored slots again in
/// the loading burst.
#[test]
fn the_quickslots_are_set_swapped_and_deleted_and_a_relog_sets_them_again() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    create_account(&server, "alice");
    add_characters(&database);

    // Given: Alpha holds a small red potion at cell 0.
    let (mut alpha, _character) = enter_world(&server, b"alice", 0);
    Server::write_console(&console, "item give Alpha 27001 1");
    server.wait_for("id 100000000; the client has it");
    let _set = alpha.read_game();

    // When: the potion, skill 3 and command 5 are put on slots 0, 1 and 2.
    alpha.send_record(&client_quickslot_add(0, 1, 0));
    alpha.send_record(&client_quickslot_add(1, 2, 3));
    alpha.send_record(&client_quickslot_add(2, 3, 5));
    // Then: each is told.
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_ADD, 0, 1, 0]);
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_ADD, 1, 2, 3]);
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_ADD, 2, 3, 5]);
    // And: the empty cell 5 is refused with no record, as legacy's `QuickslotAdd` is.
    alpha.unanswered(&client_quickslot_add(4, 1, 5));

    // When: skill 3 is put on slot 5 too.
    alpha.send_record(&client_quickslot_add(5, 2, 3));
    // Then: slot 1 is emptied first.
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_DEL, 1]);
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_ADD, 5, 2, 3]);

    // When: slot 0 is swapped with slot 7, and slot 2 is deleted.
    alpha.send_record(&protocol::cg_quickslot_swap::CgQuickslotSwap::new(0, 7).encode());
    alpha.send_record(&protocol::cg_quickslot_del::CgQuickslotDel::new(2).encode());
    // Then: both are told.
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_SWAP, 0, 7]);
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_DEL, 2]);

    // And: the logout save stores the two slots left.
    drop(alpha);
    wait_for(
        &database,
        "(SELECT array_agg((slot, kind, pos) ORDER BY slot)::text FROM quickslot) \
         = '{\"(5,2,3)\",\"(7,1,0)\"}'",
    );

    // When: the character is selected again.
    server.wait_for("Character left the world");
    let (mut alpha, _character, quickslots, items) = load_with_quickslots(&server, b"alice", 0);
    // Then: the loading burst sets both, in slot order, before the gold.
    assert_eq!(
        quickslots,
        [
            vec![GC_QUICKSLOT_ADD, 5, 2, 3],
            vec![GC_QUICKSLOT_ADD, 7, 1, 0]
        ]
    );
    assert_eq!(items.len(), 1, "the potion is still at cell 0");

    // When: the game is entered and the two loaded slots are swapped.
    let _add = enter_game_burst(&mut alpha);
    alpha.send_record(&protocol::cg_quickslot_swap::CgQuickslotSwap::new(5, 7).encode());
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_SWAP, 5, 7]);
    // Then: the world swapped the slots the load set, and the logout save stores them.
    drop(alpha);
    wait_for(
        &database,
        "(SELECT array_agg((slot, kind, pos) ORDER BY slot)::text FROM quickslot) \
         = '{\"(5,1,0)\",\"(7,2,3)\"}'",
    );
}

/// `sys.char.quickslot`'s sync: a slot follows its potion when the potion moves, chains to the
/// next potion of its vnum when a merge uses it up, and is deleted when part of it is dropped
/// (`G/char_quickslot.cpp:12-34`, `:138-155`). The logout save stores the slots the world
/// answered.
#[test]
fn a_quickslot_follows_its_potion_chains_when_it_is_used_up_and_a_drop_deletes_it() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    create_account(&server, "alice");
    add_characters(&database);
    let inventory = common::item_slots::EWindows::Inventory as u8;

    // Given: Alpha holds two small red potions at cell 0, on slot 4.
    let (mut alpha, _character) = enter_world(&server, b"alice", 0);
    Server::write_console(&console, "item give Alpha 27001 2");
    server.wait_for("id 100000000; the client has it");
    assert_eq!(set_fields(&alpha.read_game()), (inventory, 0, 27_001, 2));
    alpha.send_record(&client_quickslot_add(4, 1, 0));
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_ADD, 4, 1, 0]);

    // When: the stack is moved whole to cell 3.
    alpha.send_record(&client_item_move(0, 3, 0));
    // Then: after the set, the slot is set again on cell 3 (`G/char_item.cpp:7822`).
    assert_eq!(alpha.read_game(), a_clear_record(0));
    assert_eq!(set_fields(&alpha.read_game()), (inventory, 3, 27_001, 2));
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_ADD, 4, 1, 3]);

    // When: a third potion is given, lands at cell 0, and the stack at cell 3 is merged into
    // it.
    Server::write_console(&console, "item give Alpha 27001 1");
    server.wait_for("id 100000001; the client has it");
    assert_eq!(set_fields(&alpha.read_game()), (inventory, 0, 27_001, 1));
    alpha.send_record(&client_item_move(3, 0, 2));
    // Then: the used-up stack at cell 3 is cleared and, because a potion used up chains
    // (`CItem::SetCount`), the slot follows the first potion left, at cell 0; then cell 0 is
    // updated to three (`G/char_item.cpp:7802-7803`).
    assert_eq!(alpha.read_game(), a_clear_record(3));
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_ADD, 4, 1, 0]);
    assert_eq!(update_fields(&alpha.read_game()), (inventory, 0, 3));

    // When: one of the three is dropped.
    alpha.send_record(&client_item_drop(0, Some(1)));
    // Then: legacy deletes the slot before the drop, even for part of the stack
    // (`G/char_item.cpp:7501`).
    assert_eq!(alpha.read_game(), [GC_QUICKSLOT_DEL, 4]);
    assert_eq!(update_fields(&alpha.read_game()), (inventory, 0, 2));
    assert_eq!(alpha.read_game(), a_ground_add(1, 27_001));
    assert_eq!(alpha.read_game(), an_info_line(b"[LS;443]"));

    // And: the logout save stores no slot, and the two potions left at cell 0.
    drop(alpha);
    wait_for(
        &database,
        "NOT EXISTS (SELECT 1 FROM quickslot) AND (SELECT count FROM item WHERE id = \
         100000001) = 2",
    );
}

/// One second of the recovery event: the pool's `GC_CHARACTER_POINT_CHANGE`, then the
/// recovery's, both for `vid`.
fn read_a_tick(keyed: &mut Keyed, vid: u32) -> [Seen; 2] {
    [keyed.read_game(), keyed.read_game()].map(|record| {
        assert_eq!(record[0], GC_POINT_CHANGE);
        assert_eq!(&record[4..8], &vid.to_le_bytes(), "the drinker's VID");
        let value = i64::from_le_bytes(record[17..25].try_into().expect("eight bytes"));
        Seen::Point(record[8], value)
    })
}

/// A `GC_ITEM_GROUND_ADD` where Alpha and Zulu stand, [`ALPHA_ENTERS_AT`], with z 0.
fn a_ground_add(vid: u32, vnum: u32) -> Vec<u8> {
    let mut record = vec![GROUND_ADD];
    for word in [ALPHA_ENTERS_AT.0, ALPHA_ENTERS_AT.1, 0] {
        record.extend_from_slice(&word.to_le_bytes());
    }
    record.extend_from_slice(&vid.to_le_bytes());
    record.extend_from_slice(&vnum.to_le_bytes());
    record
}

/// A `GC_ITEM_GROUND_DEL`.
fn a_ground_del(vid: u32) -> Vec<u8> {
    let mut record = vec![GROUND_DEL];
    record.extend_from_slice(&vid.to_le_bytes());
    record
}

/// A `CHAT_TYPE_INFO` line to a character of empire 1.
fn an_info_line(text: &[u8]) -> Vec<u8> {
    let size = u8::try_from(10 + text.len()).expect("a short line");
    let mut record = vec![GC_CHAT, size, 0, 1, 0, 0, 0, 0, 1, 1];
    record.extend_from_slice(text);
    record
}

/// Accounts for alice, bob and each of `others`, all in empire 1: Alpha for alice
/// ([`add_characters`]) and Zulu for bob, standing where Alpha stands.
fn seat_alpha_and_zulu(server: &Server, database: &ScratchDatabase, others: &[&str]) {
    create_account(server, "alice");
    create_account(server, "bob");
    for login in others {
        create_account(server, login);
    }
    add_characters(database);
    sql(database, "UPDATE account SET empire = 1");
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Zulu', 1, 470000, \
         950000 FROM account WHERE login = 'bob'",
    );
    // Zulu never walks, so it is stored above every stamina maximum like the fixtures.
    stamina(database, "Zulu", 12_345);
}

/// Load `login`'s first character and enter the game with [`enter_game_records`].
fn enter_game_unread(server: &Server, login: &[u8]) -> (Keyed, Listed) {
    let (mut keyed, character, _items) = load_character(server, login, 0);
    enter_game_records(&mut keyed);
    (keyed, character)
}

/// A `CG_ITEM_DROP2` of `count` from the inventory cell `cell`, or a `CG_ITEM_DROP` when
/// `count` is `None`.
fn client_item_drop(cell: u16, count: Option<u16>) -> Vec<u8> {
    let cell =
        protocol::item_pos::ItemPos::new(common::item_slots::EWindows::Inventory as u8, cell);
    match count {
        Some(count) => protocol::cg_item_drop2::CgItemDrop2::new(cell, 0, count).encode(),
        None => protocol::cg_item_drop::CgItemDrop::new(cell, 0).encode(),
    }
}

/// `cg.game.item_drop`, `cg.game.item_drop2`, `cg.game.item_pickup`, `sys.world.view`: part of
/// a stack is dropped, a drop inside the second after it is refused, the rest is dropped whole,
/// and each part is picked up again, one by a character that entered the map after it fell. The
/// item is an entity of the view (`G/item.cpp:563-597`): every client in view sees each item fall
/// and go, an entrant is shown what lies in its view, and a character out of view hears none of
/// it. The store follows each step.
#[test]
fn a_dropped_stack_lies_on_the_map_until_someone_picks_it_up() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    seat_alpha_and_zulu(&server, &database, &["carol"]);
    // Charlie stands on map 1 but 15700 west and 9300 north of Alpha, out of view.
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Charlie', 2, \
         454300, 940700 FROM account WHERE login = 'carol'",
    );
    let protos = owners_protos();
    let stackable = a_stackable_vnum(&protos);
    let mut picked_up = b"[LS;444;".to_vec();
    picked_up.extend_from_slice(&protos.get(stackable).expect("a proto").locale_name);
    picked_up.push(b']');
    let inventory = common::item_slots::EWindows::Inventory as u8;

    // Given: Alpha holds ten of a stackable item at cell 0.
    let (mut alpha, alpha_listed) = enter_world(&server, b"alice", 0);
    Server::write_console(&console, &format!("item give Alpha {stackable} 10"));
    server.wait_for("id 100000000; the client has it");
    assert_eq!(
        set_fields(&alpha.read_game()),
        (inventory, 0, stackable, 10)
    );

    // When: four are dropped.
    alpha.send_record(&client_item_drop(0, Some(4)));
    // Then: the stack is updated to six, the four fall under ground VID 1 and legacy's line
    // follows (`G/char_item.cpp:7536`); the four are a new item with no row yet.
    assert_eq!(update_fields(&alpha.read_game()), (inventory, 0, 6));
    assert_eq!(alpha.read_game(), a_ground_add(1, stackable));
    assert_eq!(alpha.read_game(), an_info_line(b"[LS;443]"));
    check(
        &database,
        "(SELECT count FROM item WHERE id = 100000000) = 6 AND (SELECT count(*) FROM item) = 1",
    );
    // And: a drop inside the same second is refused with legacy's line.
    alpha.send_record(&client_item_drop(0, None));
    assert_eq!(
        alpha.read_game(),
        an_info_line(b"@@(char_item.cpp)tradus:[#Unk]You cannot drop Yang yet")
    );
    let dropped_at = std::time::Instant::now();

    // When: Zulu enters the map.
    let (mut zulu, zulu_listed) = {
        let (mut keyed, character, _items) = load_character(&server, b"bob", 0);
        let entered = enter_game_view(&mut keyed);
        // Then: the item lying there is shown once, after Alpha, who stands in its sectree: a
        // sectree's players come before its ground items (V1).
        let grounds = entered.shown.ground_adds();
        assert_eq!(grounds.len(), 1, "{grounds:02x?}");
        assert_eq!(grounds[0].1, a_ground_add(1, stackable));
        assert!(entered.shown.insert_at(alpha_listed.id) < grounds[0].0);
        entered.shown.pair_of(alpha_listed.id);
        (keyed, character)
    };
    // And: Alpha sees Zulu arrive.
    alpha.sees_arrive(zulu_listed.id, false);
    // When: Charlie enters out of view. Then: nothing lying there is shown, and nobody sees
    // Charlie arrive.
    let (mut charlie, _, charlie_entered) = enter_world_seeing(&server, b"carol", 0);
    let charlie_grounds = charlie_entered.shown.ground_adds();
    assert!(charlie_grounds.is_empty(), "{charlie_grounds:02x?}");
    alpha.quiet("Charlie enters out of Alpha's view");
    zulu.quiet("and out of Zulu's");

    // When: once the second has passed, Alpha drops the rest whole and shouts, in one write.
    std::thread::sleep(Duration::from_millis(1100).saturating_sub(dropped_at.elapsed()));
    let mut both = client_item_drop(0, None);
    both.extend(client_chat(prodomo::chat::CHAT_SHOUT, b"hi"));
    alpha.send_record(&both);
    // Then: the cell is cleared, the six fall under ground VID 2, Zulu sees them fall, and the
    // row is gone. Legacy writes the fall to the view while it reads the drop, before it reads
    // the shout, so the view hears the shout after the fall.
    let shout = chat_packet(prodomo::chat::CHAT_SHOUT, 1, b"|Len|l Alpha : hi");
    assert_eq!(alpha.read_game(), a_clear_record(0));
    assert_eq!(alpha.read_game(), a_ground_add(2, stackable));
    assert_eq!(alpha.read_game(), an_info_line(b"[LS;443]"));
    assert_eq!(alpha.read_game(), shout);
    assert_eq!(zulu.read_game(), a_ground_add(2, stackable));
    assert_eq!(zulu.read_game(), shout, "the shout after the fall");
    assert_eq!(
        charlie.read_game(),
        shout,
        "Charlie is out of the item's view and hears only the shout"
    );
    check(&database, "NOT EXISTS (SELECT 1 FROM item)");

    // When: Zulu picks up the four.
    zulu.send_record(&protocol::cg_item_pickup::CgItemPickup::new(1).encode());
    // Then: they leave the ground for Zulu's cell 0, highlighted because Zulu did not drop
    // them, and Alpha sees them go.
    assert_eq!(zulu.read_game(), a_ground_del(1));
    let set = zulu.read_game();
    assert_eq!(set_fields(&set), (inventory, 0, stackable, 4));
    assert_eq!(set[26], 1, "the pick-up highlight");
    assert_eq!(zulu.read_game(), an_info_line(&picked_up));
    assert_eq!(alpha.read_game(), a_ground_del(1));
    check(
        &database,
        "EXISTS (SELECT 1 FROM item JOIN player ON player.id = item.owner_id WHERE item.id = \
         100000001 AND name = 'Zulu' AND window_type = 1 AND pos = 0 AND count = 4)",
    );

    // When: Alpha picks up the six it dropped.
    alpha.send_record(&protocol::cg_item_pickup::CgItemPickup::new(2).encode());
    // Then: they come back to cell 0 unhighlighted, under their own row again.
    assert_eq!(alpha.read_game(), a_ground_del(2));
    let set = alpha.read_game();
    assert_eq!(set_fields(&set), (inventory, 0, stackable, 6));
    assert_eq!(set[26], 0, "Alpha was the last holder");
    assert_eq!(alpha.read_game(), an_info_line(&picked_up));
    assert_eq!(zulu.read_game(), a_ground_del(2));
    check(
        &database,
        "EXISTS (SELECT 1 FROM item JOIN player ON player.id = item.owner_id WHERE item.id = \
         100000000 AND name = 'Alpha' AND window_type = 1 AND pos = 0 AND count = 6)",
    );
    charlie.quiet("Charlie hears neither pick-up");
    // And: an item already picked up is not there to pick up, and nobody is told.
    alpha.unanswered(&protocol::cg_item_pickup::CgItemPickup::new(1).encode());
    zulu.quiet("nobody is told of an item already picked up");
}

/// `event.item.item_destroy_event`: with a two-second lifetime, an item nobody picks up is
/// destroyed on time and every client in its view sees it go (`RemoveFromGround`,
/// `G/item.cpp:533-547`). A `CG_ITEM_DROP2` carrying gold is ignored before the drop limit,
/// because `DropGold` is not ported.
#[test]
fn a_dropped_item_nobody_picks_up_is_destroyed_on_time_and_its_view_sees_it_go() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) = Server::start_with_console_configured(
        binary(),
        database.url(),
        &default_channels(),
        "item_destroy_time_dropitem = 2",
    );
    seat_alpha_and_zulu(&server, &database, &[]);
    let stackable = a_stackable_vnum(&owners_protos());
    let inventory = common::item_slots::EWindows::Inventory as u8;

    // Given: Alpha holds three of a stackable item at cell 0, and Zulu stands beside it.
    let (mut alpha, _character) = enter_world(&server, b"alice", 0);
    Server::write_console(&console, &format!("item give Alpha {stackable} 3"));
    server.wait_for("id 100000000; the client has it");
    assert_eq!(set_fields(&alpha.read_game()), (inventory, 0, stackable, 3));
    let (mut zulu, zulu_listed) = enter_game_unread(&server, b"bob");
    alpha.sees_arrive(zulu_listed.id, false);

    // When: Alpha drops gold. Then: nothing answers, and the drop limit is not started.
    alpha.unanswered(
        &protocol::cg_item_drop2::CgItemDrop2::new(
            protocol::item_pos::ItemPos::new(inventory, 0),
            100,
            1,
        )
        .encode(),
    );

    // When: Alpha drops the stack whole.
    alpha.send_record(&client_item_drop(0, None));
    let dropped_at = std::time::Instant::now();
    // Then: it falls under ground VID 1 and both clients see it.
    assert_eq!(alpha.read_game(), a_clear_record(0));
    assert_eq!(alpha.read_game(), a_ground_add(1, stackable));
    assert_eq!(alpha.read_game(), an_info_line(b"[LS;443]"));
    assert_eq!(zulu.read_game(), a_ground_add(1, stackable));

    // Then: two seconds on, it is destroyed and both clients see it go.
    assert_eq!(alpha.read_game(), a_ground_del(1));
    assert_eq!(zulu.read_game(), a_ground_del(1));
    assert!(
        dropped_at.elapsed() >= Duration::from_millis(1900),
        "destroyed after {:?}",
        dropped_at.elapsed()
    );
    // And: the item is gone for good: no row, and nothing to pick up.
    check(&database, "NOT EXISTS (SELECT 1 FROM item)");
    zulu.unanswered(&protocol::cg_item_pickup::CgItemPickup::new(1).encode());
}

/// `cg.game.item_pickup`, V12: the view hears of a drop before the dropper's store writes it,
/// and a pick-up of the dropped item stores its rows only after the drop's. A whole stack keeps
/// its row's id on the ground, so the pick-up inserts the id the drop deletes; stored first, it
/// would meet the drop's row and be refused. Legacy keeps one item and writes it later
/// (`G/char_item.cpp:7972`, `G/item_manager.cpp:459-470`).
#[test]
fn a_pick_up_is_stored_only_after_the_drop_it_follows() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    seat_alpha_and_zulu(&server, &database, &[]);
    let protos = owners_protos();
    let stackable = a_stackable_vnum(&protos);
    let mut picked_up = b"[LS;444;".to_vec();
    picked_up.extend_from_slice(&protos.get(stackable).expect("a proto").locale_name);
    picked_up.push(b']');
    let inventory = common::item_slots::EWindows::Inventory as u8;

    // Given: Alpha holds three of a stackable item at cell 0, and Zulu stands beside it.
    let (mut alpha, _character) = enter_world(&server, b"alice", 0);
    Server::write_console(&console, &format!("item give Alpha {stackable} 3"));
    server.wait_for("id 100000000; the client has it");
    assert_eq!(set_fields(&alpha.read_game()), (inventory, 0, stackable, 3));
    let (mut zulu, zulu_listed) = enter_game_unread(&server, b"bob");
    alpha.sees_arrive(zulu_listed.id, false);
    // And: another transaction holds the item's row, so the store cannot delete it yet.
    let (lock, locked) = support::hold_lock(
        database.url(),
        "SELECT id FROM item WHERE id = 100000000 FOR UPDATE",
    );
    assert_eq!(locked, 1, "the lock holds the item's row");

    // When: Alpha drops the stack whole. Then: Zulu sees it fall while the store waits, and
    // Alpha, whose records follow the write, is told nothing yet.
    alpha.send_record(&client_item_drop(0, None));
    assert_eq!(zulu.read_game(), a_ground_add(1, stackable));
    // When: Zulu picks it up. Then: the pick-up waits for the drop's rows, nobody is told, and
    // the row is still Alpha's.
    zulu.unanswered(&protocol::cg_item_pickup::CgItemPickup::new(1).encode());
    alpha.quiet("the drop's records wait for its rows");
    check(
        &database,
        "EXISTS (SELECT 1 FROM item JOIN player ON player.id = item.owner_id WHERE item.id = \
         100000000 AND name = 'Alpha' AND count = 3)",
    );

    // When: the row is freed.
    lock.release();
    // Then: the drop is stored and answered, and Alpha then sees the item go.
    assert_eq!(alpha.read_game(), a_clear_record(0));
    assert_eq!(alpha.read_game(), a_ground_add(1, stackable));
    assert_eq!(alpha.read_game(), an_info_line(b"[LS;443]"));
    assert_eq!(alpha.read_game(), a_ground_del(1));
    // And: the pick-up is stored after it and answered, and the row is Zulu's.
    assert_eq!(zulu.read_game(), a_ground_del(1));
    let set = zulu.read_game();
    assert_eq!(set_fields(&set), (inventory, 0, stackable, 3));
    assert_eq!(set[26], 1, "the pick-up highlight");
    assert_eq!(zulu.read_game(), an_info_line(&picked_up));
    check(
        &database,
        "EXISTS (SELECT 1 FROM item JOIN player ON player.id = item.owner_id WHERE item.id = \
         100000000 AND name = 'Zulu' AND window_type = 1 AND pos = 0 AND count = 3) AND \
         (SELECT count(*) FROM item) = 1",
    );
}

/// Lower Alpha to level 9 with 1500 hit points and 70000 spell points, and give it five
/// `EQUIPMENT` rows, ids 20 to 24: the armour at cell 0 with a 500-point maximum hit point
/// attribute, the bell at cell 1, the fan at cell 4, the spare armour at cell 64, and the stone
/// at cell 65. The prototype fields the hand sums read are checked against the owner's data
/// first, so a changed prototype fails here and not as a wrong sum.
fn give_alpha_equipment(database: &ScratchDatabase) {
    let protos = owners_protos();
    let proto = |vnum| protos.get(vnum).expect("the owner's data has it");
    let level_limit = |vnum| {
        let limit = proto(vnum).limits[0];
        (limit.kind, limit.value)
    };
    assert_eq!(proto(WORN_ARMOUR).values[1], 21, "the armour's defence");
    assert_eq!(proto(WORN_ARMOUR).values[5], 0);
    assert_eq!(level_limit(WORN_ARMOUR), (1, 9));
    assert_eq!(level_limit(BELL), (1, 10));
    assert_eq!(level_limit(WORN_FAN), (1, 0));
    assert_eq!(proto(STONE).item_type, gamedata::item_kind::ITEM_DS);
    assert_eq!((proto(BELL).size, proto(SPARE_ARMOUR).size), (1, 2));

    sql(
        database,
        "UPDATE player SET level = 9, hp = 1500, sp = 70000 WHERE name = 'Alpha'",
    );
    for (id, pos, vnum) in [
        (20, 0, WORN_ARMOUR),
        (21, 1, BELL),
        (22, 4, WORN_FAN),
        (23, 64, SPARE_ARMOUR),
        (24, 65, STONE),
    ] {
        sql(
            database,
            &format!(
                "INSERT INTO item (id, owner_id, window_type, pos, count, vnum) SELECT {id}, id, \
                 2, {pos}, 1, {vnum} FROM player WHERE name = 'Alpha'"
            ),
        );
    }
    sql(
        database,
        "UPDATE item SET attrtype0 = 1, attrvalue0 = 500 WHERE id = 20",
    );
}

/// Read `count` `GC_ITEM_SET` records of the item load, each checked whole-width with no
/// highlight, and answer their window, cell, vnum and count.
fn loaded_item_sets(keyed: &mut Keyed, count: usize) -> Vec<(u8, u16, u32, u16)> {
    (0..count)
        .map(|_| {
            let record = keyed.read_game();
            assert_eq!(record.len(), ITEM_SET_LEN);
            assert_eq!(record[0], ITEM_SET);
            assert_eq!(record[26], 0, "no highlight");
            set_fields(&record)
        })
        .collect()
}

/// Send `CG_ENTER_GAME` and read the enter-game burst, leaving nothing unread. Answers the
/// own `GC_CHARACTER_ADD` and the six parts of `GC_CHAR_ADDITIONAL_INFO`, which follow its
/// `dwVID` and 25-byte name.
fn enter_game_with_parts(keyed: &mut Keyed) -> (Vec<u8>, [u16; 6]) {
    keyed.send_record(&client_enter_game());
    let add = keyed.read_game();
    assert_eq!(add[0], GC_CHARACTER_ADD);
    let additional = keyed.read_game();
    assert_eq!(additional.len(), CHAR_ADDITIONAL_INFO_LEN);
    assert_eq!(additional[0], GC_CHAR_ADDITIONAL_INFO);
    let parts = std::array::from_fn(|index| {
        let at = 30 + 2 * index;
        u16::from_le_bytes([additional[at], additional[at + 1]])
    });
    let (_shown, affect) = read_shown(keyed);
    assert_eq!(affect[0], GC_AFFECT_ADD);
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_GAME]);
    assert_eq!(keyed.read_game()[0], GC_TIME);
    assert_eq!(keyed.read_game(), [GC_CHANNEL, 1]);
    assert_eq!(keyed.read_game()[0], GC_CHAT);
    assert_eq!(keyed.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));
    (add, parts)
}

/// Checks the relogged character's granted item: vnum 19 at the cell the grant chose, with the
/// prototype's flags and no highlight.
fn assert_the_granted_record(granted: &[u8], protos: &gamedata::item_proto::ItemProtos) {
    let inventory = common::item_slots::EWindows::Inventory as u8;
    assert_eq!(granted.len(), ITEM_SET_LEN);
    assert_eq!(granted[1], inventory, "the granted item's window");
    assert_eq!(
        &granted[2..4],
        &0_u16.to_le_bytes(),
        "the cell the grant chose"
    );
    assert_eq!(&granted[4..8], &a_plain_vnum().to_le_bytes(), "vnum");
    assert_eq!(&granted[8..10], &1_u16.to_le_bytes(), "count");
    // The flags are the prototype's, because the store keeps none (`G/item.cpp:221-226`,
    // `G/item.h:78`). Vnum 19 carries both kinds, so a record of zeros fails here.
    let proto = protos
        .get(a_plain_vnum())
        .expect("checked by the section's first test");
    assert_ne!(
        (proto.flags, proto.anti_flags),
        (0, 0),
        "a prototype with flags"
    );
    assert_eq!(&granted[18..22], &proto.flags.to_le_bytes(), "flags");
    assert_eq!(
        &granted[22..26],
        &proto.anti_flags.to_le_bytes(),
        "anti_flags"
    );
    assert_eq!(
        granted[26], 0,
        "the last owner is the owner, so no highlight"
    );
}

/// The stored fields a `GC_ITEM_SET` relays from an item row. Each value has distinct byte
/// halves, so a field sent from the wrong column or at the wrong offset cannot pass.
struct RelayedFields {
    refine_element: u32,
    transmutation: u32,
    sockets: [i32; 6],
    attributes: [(u8, i16); 7],
}

impl RelayedFields {
    fn distinct() -> Self {
        Self {
            refine_element: 0x0a0b_0c0d,
            transmutation: 0x0102_0304,
            sockets: [0x1122_3344, -2, 0x0506_0708, 0, 0x0a0b, -0x0102_0304],
            attributes: [
                (1, 0x0203),
                (0, 0),
                (0x7f, -0x0405),
                (4, 0x0607),
                (0, 0),
                (0x10, 0x0809),
                (0x21, -0x0102),
            ],
        }
    }

    /// Writes these fields into item row `id`.
    fn store(&self, database: &ScratchDatabase, id: u32) {
        let socket_columns = self
            .sockets
            .iter()
            .enumerate()
            .map(|(index, socket)| format!("socket{index} = {socket}"))
            .collect::<Vec<_>>()
            .join(", ");
        let attribute_columns = self
            .attributes
            .iter()
            .enumerate()
            .map(|(index, (kind, value))| {
                format!("attrtype{index} = {kind}, attrvalue{index} = {value}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        sql(
            database,
            &format!(
                "UPDATE item SET refine_element = {}, transmutation = {}, {socket_columns}, \
                 {attribute_columns} WHERE id = {id}",
                self.refine_element, self.transmutation
            ),
        );
    }

    /// The whole record for an item of `proto` that carries these fields, in the base
    /// inventory at `cell` with `count`, built in `TPacketGCItemSet`'s field order
    /// (`G/packet.h:1427-1444`): the header, the cell (window byte, then a `WORD` cell), vnum,
    /// count, refine element, transmutation, flags, anti-flags, highlight, six `long` sockets,
    /// and seven attributes of a `BYTE` type and a `short` value. The highlight is 0, because
    /// the load makes the owner the last owner.
    fn item_set(&self, proto: &gamedata::item_proto::ItemProto, cell: u16, count: u16) -> Vec<u8> {
        let mut expected = vec![ITEM_SET, common::item_slots::EWindows::Inventory as u8];
        expected.extend_from_slice(&cell.to_le_bytes());
        expected.extend_from_slice(&proto.vnum.to_le_bytes());
        expected.extend_from_slice(&count.to_le_bytes());
        expected.extend_from_slice(&self.refine_element.to_le_bytes());
        expected.extend_from_slice(&self.transmutation.to_le_bytes());
        expected.extend_from_slice(&proto.flags.to_le_bytes());
        expected.extend_from_slice(&proto.anti_flags.to_le_bytes());
        expected.push(0);
        for socket in self.sockets {
            expected.extend_from_slice(&socket.to_le_bytes());
        }
        for (kind, value) in self.attributes {
            expected.push(kind);
            expected.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(expected.len(), ITEM_SET_LEN, "the expectation itself");
        expected
    }
}

/// `HEADER_CG_EXCHANGE` (`G/packet.h`).
const CG_EXCHANGE: u8 = 27;
/// `HEADER_GC_EXCHANGE` (`G/packet.h`).
const GC_EXCHANGE: u8 = 42;
/// `packet_exchange` (`G/packet.h:1627-1650`): the header, `sub_header`, `is_me`, an
/// `unsigned long long arg1`, `TItemPos arg2`, `DWORD arg3`, `TItemPos arg4`, six `long`
/// sockets, seven attributes, `dwRefineElement` and `dwTransmutation`.
const EXCHANGE_LEN: usize = 1 + 1 + 1 + 8 + 3 + 4 + 3 + 6 * 4 + 7 * 3 + 4 + 4;
/// `NPOS`: `TItemPos(RESERVED_WINDOW, WORD_MAX)`, the `arg2` of a record with no cell.
const NO_CELL: (u8, u16) = (0, u16::MAX);

/// A `TPacketCGExchange` laid out by hand: the header, `sub_header`, an eight-byte `arg1`,
/// `arg2`, and a `TItemPos` of the base inventory at `cell`.
fn client_exchange(sub_header: u8, arg1: u64, arg2: u8, cell: u16) -> Vec<u8> {
    let mut record = vec![CG_EXCHANGE, sub_header];
    record.extend_from_slice(&arg1.to_le_bytes());
    record.push(arg2);
    record.push(common::item_slots::EWindows::Inventory as u8);
    record.extend_from_slice(&cell.to_le_bytes());
    assert_eq!(record.len(), 14, "sizeof(TPacketCGExchange)");
    record
}

/// A `GC_EXCHANGE` as `exchange_packet` writes it with no item (`G/exchange.cpp:24-59`):
/// `arg3` 0, `arg4` `(RESERVED_WINDOW, 0)`, and every item field zero.
fn exchange_record(sub_header: u8, is_me: bool, arg1: u64, arg2: (u8, u16)) -> Vec<u8> {
    let mut record = vec![GC_EXCHANGE, sub_header, u8::from(is_me)];
    record.extend_from_slice(&arg1.to_le_bytes());
    record.push(arg2.0);
    record.extend_from_slice(&arg2.1.to_le_bytes());
    record.resize(EXCHANGE_LEN, 0);
    record
}

/// The `EXCHANGE_SUBHEADER_GC_ITEM_ADD` for `count` of `vnum` from inventory `cell`, shown on
/// display cell `display`, carrying `fields`, in `packet_exchange` field order.
fn offered_record(is_me: bool, vnum: u32, (display, count, cell): (u16, u32, u16)) -> Vec<u8> {
    let fields = RelayedFields::distinct();
    let mut record = vec![GC_EXCHANGE, 1, u8::from(is_me)];
    record.extend_from_slice(&u64::from(vnum).to_le_bytes());
    record.push(0);
    record.extend_from_slice(&display.to_le_bytes());
    record.extend_from_slice(&count.to_le_bytes());
    record.push(common::item_slots::EWindows::Inventory as u8);
    record.extend_from_slice(&cell.to_le_bytes());
    for socket in fields.sockets {
        record.extend_from_slice(&socket.to_le_bytes());
    }
    for (kind, value) in fields.attributes {
        record.push(kind);
        record.extend_from_slice(&value.to_le_bytes());
    }
    record.extend_from_slice(&fields.refine_element.to_le_bytes());
    record.extend_from_slice(&fields.transmutation.to_le_bytes());
    assert_eq!(record.len(), EXCHANGE_LEN, "the expectation itself");
    record
}

/// The trade scenario's two characters, on one spot of map 1: alice's Alpha, in empire 1, with
/// 5000 gold and seven of `vnum` at cell 5 in item row 30, which carries every relayed field;
/// and bob's Yankee, in empire 2, with 4000 gold.
fn seat_the_traders(database: &ScratchDatabase, vnum: u32) {
    add_characters(database);
    sql(
        database,
        "UPDATE account SET empire = 2 WHERE login = 'bob'",
    );
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y, gold) SELECT id, 0, 'Yankee', 1, \
         470000, 950000, 4000 FROM account WHERE login = 'bob'",
    );
    sql(
        database,
        "UPDATE player SET gold = 5000 WHERE name = 'Alpha'",
    );
    sql(
        database,
        &format!(
            "INSERT INTO item (id, owner_id, window_type, pos, count, vnum) SELECT 30, id, 1, 5, \
             7, {vnum} FROM player WHERE name = 'Alpha'"
        ),
    );
    RelayedFields::distinct().store(database, 30);
}

/// Alpha asks Yankee for a trade, offers the item, takes it back, offers it again, and offers
/// 300 gold; Yankee offers more gold than it holds, then 1234. Each step's records are checked
/// on both sides.
fn make_the_offers(alice: &mut Keyed, bob: &mut Keyed, vnum: u32, (alpha, yankee): (u32, u32)) {
    let (alpha, yankee) = (u64::from(alpha), u64::from(yankee));
    // When: Alpha asks Yankee, with a VID whose upper half `Find(DWORD)` never reads. Then: the
    // asked side's window opens first, each naming the other.
    alice.send_record(&client_exchange(0, (0xdead_beef << 32) | yankee, 0, 0));
    assert_eq!(bob.read_game(), exchange_record(0, false, alpha, NO_CELL));
    assert_eq!(
        alice.read_game(),
        exchange_record(0, false, yankee, NO_CELL)
    );

    // When: Alpha offers the item on display cell 2, and takes back slot 0, named by a BYTE
    // `arg1` whose next byte is set. Then: each side sees the offer and the removal, the other
    // side's removal naming the item's cell.
    alice.send_record(&client_exchange(1, 0, 2, 5));
    assert_eq!(alice.read_game(), offered_record(true, vnum, (2, 7, 5)));
    assert_eq!(bob.read_game(), offered_record(false, vnum, (2, 7, 5)));
    alice.send_record(&client_exchange(2, 0x0100, 0, 0));
    assert_eq!(alice.read_game(), exchange_record(2, true, 0, NO_CELL));
    let inventory = common::item_slots::EWindows::Inventory as u8;
    assert_eq!(
        bob.read_game(),
        exchange_record(2, false, 0, (inventory, 5))
    );
    alice.send_record(&client_exchange(1, 0, 3, 5));
    assert_eq!(alice.read_game(), offered_record(true, vnum, (3, 7, 5)));
    assert_eq!(bob.read_game(), offered_record(false, vnum, (3, 7, 5)));

    // When: Alpha offers 300 gold, and Yankee one more than it holds, then 1234. Then: the
    // short offer is answered to Yankee alone.
    alice.send_record(&client_exchange(3, 300, 0, 0));
    assert_eq!(alice.read_game(), exchange_record(3, true, 300, NO_CELL));
    assert_eq!(bob.read_game(), exchange_record(3, false, 300, NO_CELL));
    bob.send_record(&client_exchange(3, 4001, 0, 0));
    assert_eq!(bob.read_game(), exchange_record(7, false, 0, NO_CELL));
    alice.quiet("the other side is not told of a short offer");
    bob.send_record(&client_exchange(3, 1234, 0, 0));
    assert_eq!(bob.read_game(), exchange_record(3, true, 1234, NO_CELL));
    assert_eq!(alice.read_game(), exchange_record(3, false, 1234, NO_CELL));
}

/// `cg.game.exchange`, `gc.exchange`, `sys.trade.exchange`, and the six `CInputMain::Exchange`
/// arms (`G/input_main.cpp:1367-1527`): a trade opens both windows, shows each offer, a taken
/// back item and each amount of gold to both sides, refuses more gold than the side holds, and
/// settles when both sides accept (`CExchange::Accept`, `G/exchange.cpp:606-693`); a cancel
/// ends a second trade for both sides.
///
/// The settlement stores both sides in one transaction before either side is sent anything
/// (ADR-0003), so the store is checked as soon as the records arrive. `Done` runs first for
/// Yankee, whose accept completed the trade: its gold goes to Alpha; then Alpha's item goes to
/// Yankee's first free cell and Alpha's gold follows. The quickslot on the item's cell is
/// deleted, and Alpha's logout save stores the slots the trade left.
#[test]
fn a_trade_moves_an_item_and_gold_between_two_players_in_one_transaction() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    let protos = owners_protos();
    let vnum = a_stackable_use_vnum(&protos);
    let proto = protos.get(vnum).expect("a proto");
    assert_eq!(
        proto.anti_flags & world::item::ITEM_ANTIFLAG_GIVE,
        0,
        "an item that may be given"
    );
    seat_the_traders(&database, vnum);
    let (mut alice, alpha) = enter_world(&server, b"alice", 0);
    let (mut bob, yankee) = enter_world(&server, b"bob", 0);
    alice.sees_arrive(yankee.id, true);
    // Given: Alpha's slot 2 names the item's cell 5, and slot 6 skill 3.
    alice.send_record(&client_quickslot_add(2, 1, 5));
    assert_eq!(alice.read_game(), [GC_QUICKSLOT_ADD, 2, 1, 5]);
    alice.send_record(&client_quickslot_add(6, 2, 3));
    assert_eq!(alice.read_game(), [GC_QUICKSLOT_ADD, 6, 2, 3]);
    make_the_offers(&mut alice, &mut bob, vnum, (alpha.id, yankee.id));

    // When: Alpha accepts, then Yankee. Then: the first accept is shown to both sides, and the
    // second settles the trade, stored before either side reads a record.
    alice.send_record(&client_exchange(4, 0, 0, 0));
    assert_eq!(alice.read_game(), exchange_record(4, true, 1, NO_CELL));
    assert_eq!(bob.read_game(), exchange_record(4, false, 1, NO_CELL));
    bob.send_record(&client_exchange(4, 0, 0, 0));
    assert_eq!(bob.read_game(), gold_change(yankee.id, 0, 2766));
    check(
        &database,
        "(SELECT gold FROM player WHERE name = 'Alpha') = 5934 AND (SELECT gold FROM player \
         WHERE name = 'Yankee') = 3066 AND EXISTS (SELECT 1 FROM item WHERE id = 30 AND \
         owner_id = (SELECT id FROM player WHERE name = 'Yankee') AND window_type = 1 AND pos \
         = 0 AND count = 7)",
    );
    // The item reaches Yankee's first free cell whole, highlighted because Yankee never held it.
    let mut received = RelayedFields::distinct().item_set(proto, 0, 7);
    received[26] = 1;
    assert_eq!(bob.read_game(), received);
    assert_eq!(bob.read_game(), gold_change(yankee.id, 300, 3066));
    let done = |empire, other: &[u8]| {
        let mut text = b"The exchange with ".to_vec();
        text.extend_from_slice(other);
        text.extend_from_slice(b" has been completed.");
        chat_packet(prodomo::chat::CHAT_INFO, empire, &text)
    };
    assert_eq!(bob.read_game(), done(2, b"Alpha"));
    assert_eq!(bob.read_game(), exchange_record(5, false, 0, NO_CELL));
    assert_eq!(alice.read_game(), gold_change(alpha.id, 1234, 6234));
    // The slot on the item's cell is deleted before the clear (`G/exchange.cpp:543`).
    assert_eq!(alice.read_game(), [GC_QUICKSLOT_DEL, 2]);
    assert_eq!(alice.read_game(), a_clear_record(5));
    assert_eq!(alice.read_game(), gold_change(alpha.id, 0, 5934));
    assert_eq!(alice.read_game(), done(1, b"Yankee"));
    assert_eq!(alice.read_game(), exchange_record(5, false, 0, NO_CELL));
    alice.quiet("the trade is over");
    bob.quiet("the trade is over");

    // When: Yankee asks Alpha, and Alpha cancels. Then: Alpha's window opens first, and the
    // cancel ends both; an accept after it answers nothing.
    bob.send_record(&client_exchange(0, u64::from(alpha.id), 0, 0));
    let started = |other: u32| exchange_record(0, false, u64::from(other), NO_CELL);
    assert_eq!(alice.read_game(), started(yankee.id));
    assert_eq!(bob.read_game(), started(alpha.id));
    alice.send_record(&client_exchange(5, 0, 0, 0));
    assert_eq!(alice.read_game(), exchange_record(5, false, 0, NO_CELL));
    assert_eq!(bob.read_game(), exchange_record(5, false, 0, NO_CELL));
    bob.unanswered(&client_exchange(4, 0, 0, 0));
    alice.quiet("an accept with no trade reaches nobody");

    // When: Yankee leaves and comes back. Then: the save leaves the traded gold alone, and the
    // load finds the item at its new cell with every relayed field and no highlight.
    drop(bob);
    server.wait_for("Character disconnected; wrote the row");
    check(
        &database,
        "(SELECT gold FROM player WHERE name = 'Yankee') = 3066",
    );
    let (_bob, _yankee, items) = load_character(&server, b"bob", 0);
    assert_eq!(items, [RelayedFields::distinct().item_set(proto, 0, 7)]);

    // When: Alpha leaves. Then: the save stores the slots the world holds, which the trade
    // Yankee closed changed after Alpha's descriptor last held them: slot 2 is gone.
    drop(alice);
    wait_for(
        &database,
        "(SELECT array_agg((slot, kind, pos) ORDER BY slot)::text FROM quickslot WHERE \
         player_id = (SELECT id FROM player WHERE name = 'Alpha')) = '{\"(6,2,3)\"}'",
    );
}

// ---------------------------------------------------------------------------
// `sys.npc.safebox`: the safebox and the mall an account keeps (ADR-0005).
// ---------------------------------------------------------------------------

/// `HEADER_CG_MALL_CHECKOUT` (`G/packet.h:53`).
const CG_MALL_CHECKOUT: u8 = 69;
/// `HEADER_CG_SAFEBOX_CHECKIN` (`G/packet.h:54`).
const CG_SAFEBOX_CHECKIN: u8 = 70;
/// `HEADER_CG_SAFEBOX_CHECKOUT` (`G/packet.h:55`).
const CG_SAFEBOX_CHECKOUT: u8 = 71;
/// `HEADER_CG_SAFEBOX_ITEM_MOVE` (`G/packet.h:61`).
const CG_SAFEBOX_ITEM_MOVE: u8 = 77;
/// `HEADER_GC_SAFEBOX_SET` (`G/packet.h:171`).
const GC_SAFEBOX_SET: u8 = 85;
/// `HEADER_GC_SAFEBOX_DEL` (`G/packet.h:172`).
const GC_SAFEBOX_DEL: u8 = 86;
/// `HEADER_GC_SAFEBOX_WRONG_PASSWORD` (`G/packet.h:173`).
const GC_SAFEBOX_WRONG_PASSWORD: u8 = 87;
/// `HEADER_GC_SAFEBOX_SIZE` (`G/packet.h:174`).
const GC_SAFEBOX_SIZE: u8 = 88;
/// `HEADER_GC_MALL_OPEN` (`G/packet.h:199`).
const GC_MALL_OPEN: u8 = 122;
/// `HEADER_GC_MALL_SET` (`G/packet.h:200`).
const GC_MALL_SET: u8 = 128;
/// `HEADER_GC_MALL_DEL` (`G/packet.h:201`).
const GC_MALL_DEL: u8 = 129;
/// The `SAFEBOX` window (`common/length.h`).
const SAFEBOX_WINDOW: u8 = 3;
/// The `MALL` window (`common/length.h`).
const MALL_WINDOW: u8 = 4;

/// A `TPacketCGSafeboxCheckin` or `TPacketCGSafeboxCheckout` as this build's
/// `__EXTENDED_SAFEBOX__` lays it out: the header, a `DWORD` store cell, and a `TItemPos` of the
/// base inventory at `cell`.
fn client_store(header: u8, safe_pos: u32, cell: u16) -> Vec<u8> {
    let mut record = vec![header];
    record.extend_from_slice(&safe_pos.to_le_bytes());
    record.push(common::item_slots::EWindows::Inventory as u8);
    record.extend_from_slice(&cell.to_le_bytes());
    assert_eq!(record.len(), 8, "sizeof(TPacketCGSafeboxCheckin)");
    record
}

/// A `CG_SAFEBOX_ITEM_MOVE`: a `TPacketCGItemMove` between two safebox cells.
fn client_store_move(from: u16, to: u16, count: u16) -> Vec<u8> {
    let mut record = vec![CG_SAFEBOX_ITEM_MOVE, SAFEBOX_WINDOW];
    record.extend_from_slice(&from.to_le_bytes());
    record.push(SAFEBOX_WINDOW);
    record.extend_from_slice(&to.to_le_bytes());
    record.extend_from_slice(&count.to_le_bytes());
    assert_eq!(record.len(), 9, "sizeof(TPacketCGItemMove)");
    record
}

/// `CSafebox::Remove`'s `TPacketGCItemDel` under `header`: the header and a `DWORD` cell
/// (`G/safebox.cpp:117-121`).
fn store_del(header: u8, pos: u32) -> Vec<u8> {
    let mut record = vec![header];
    record.extend_from_slice(&pos.to_le_bytes());
    record
}

/// `CSafebox::Add`'s `TPacketGCItemSet` under `header`, of `count` of `proto` carrying `fields`
/// in `window` at `pos` (`G/safebox.cpp:72-88`). Legacy never writes the highlight; the
/// Rewrite sends 0.
fn store_set(
    fields: &RelayedFields,
    proto: &gamedata::item_proto::ItemProto,
    (header, window): (u8, u8),
    pos: u16,
    count: u16,
) -> Vec<u8> {
    let mut record = fields.item_set(proto, pos, count);
    record[0] = header;
    record[1] = window;
    record
}

/// The fields of a row nothing wrote them into.
const fn no_fields() -> RelayedFields {
    RelayedFields {
        refine_element: 0,
        transmutation: 0,
        sockets: [0; 6],
        attributes: [(0, 0); 7],
    }
}

/// The account ID of alice, as a subquery.
const ALICES: &str = "(SELECT id FROM account WHERE login = 'alice')";

/// The safebox scenario's cast: [`seat_the_traders`]' Alpha, holding seven of `vnum` at cell 5
/// in item row 30, and Yankee; alice's Echo, with nothing; and three of `vnum` at cell 2 of
/// alice's mall in item row 31, whose fields nothing wrote.
fn seat_the_depositors(database: &ScratchDatabase, vnum: u32) {
    seat_the_traders(database, vnum);
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 2, 'Echo', 0, \
         470000, 950000 FROM account WHERE login = 'alice'",
    );
    sql(
        database,
        &format!(
            "INSERT INTO item (id, account_id, window_type, pos, count, vnum) SELECT 31, id, 4, \
             2, 3, {vnum} FROM account WHERE login = 'alice'"
        ),
    );
}

/// A slash line as the client sends it.
fn slash(text: &[u8]) -> Vec<u8> {
    client_chat(CHAT_TALKING, text)
}

/// A `CHAT_TYPE_INFO` line to a character of empire 1.
fn info_to_alpha(text: &[u8]) -> Vec<u8> {
    chat_packet(prodomo::chat::CHAT_INFO, 1, text)
}

/// Alpha checks the item at cell 5 in at safebox cell 6, then moves it to cell 11. The
/// inventory cell is cleared before its slot is deleted (`G/input_main.cpp:2357-2360`), the
/// safebox shows the item, and the row is the account's.
fn check_in_and_move(
    alice: &mut Keyed,
    database: &ScratchDatabase,
    proto: &gamedata::item_proto::ItemProto,
) {
    let safebox = (GC_SAFEBOX_SET, SAFEBOX_WINDOW);
    let fields = RelayedFields::distinct();
    alice.send_record(&client_store(CG_SAFEBOX_CHECKIN, 6, 5));
    assert_eq!(alice.read_game(), a_clear_record(5));
    assert_eq!(alice.read_game(), [GC_QUICKSLOT_DEL, 2]);
    assert_eq!(alice.read_game(), store_set(&fields, proto, safebox, 6, 7));
    check(
        database,
        &format!(
            "EXISTS (SELECT 1 FROM item WHERE id = 30 AND owner_id IS NULL AND account_id = \
             {ALICES} AND window_type = 3 AND pos = 6 AND count = 7)"
        ),
    );
    alice.send_record(&client_store_move(6, 11, 0));
    assert_eq!(alice.read_game(), store_del(GC_SAFEBOX_DEL, 6));
    assert_eq!(alice.read_game(), store_set(&fields, proto, safebox, 11, 7));
    check(
        database,
        "EXISTS (SELECT 1 FROM item WHERE id = 30 AND window_type = 3 AND pos = 11)",
    );
}

/// Alpha opens the mall and takes its item to cell 10. The mall shows 27 rows and the item,
/// and the inventory shows it highlighted, because Alpha never held it.
fn take_from_the_mall(
    alice: &mut Keyed,
    database: &ScratchDatabase,
    proto: &gamedata::item_proto::ItemProto,
) {
    alice.send_record(&slash(b"/mall_password 000000"));
    assert_eq!(alice.read_game(), [GC_MALL_OPEN, 27]);
    let mall = (GC_MALL_SET, MALL_WINDOW);
    assert_eq!(
        alice.read_game(),
        store_set(&no_fields(), proto, mall, 2, 3)
    );
    alice.send_record(&client_store(CG_MALL_CHECKOUT, 2, 10));
    assert_eq!(alice.read_game(), store_del(GC_MALL_DEL, 2));
    let mut taken = no_fields().item_set(proto, 10, 3);
    taken[26] = 1;
    assert_eq!(alice.read_game(), taken);
    check(
        database,
        "EXISTS (SELECT 1 FROM item WHERE id = 31 AND owner_id = (SELECT id FROM player WHERE \
         name = 'Alpha') AND account_id IS NULL AND window_type = 1 AND pos = 10 AND count = 3)",
    );
}

/// Alpha closes both windows and opens each again at once: each close is a command line, and
/// each reload waits. Then Alpha changes the password, and changes it again from the old one:
/// the first is done, the second refused, and the store keeps a hash, not the password.
///
/// The tenth slash line within a second disconnects (`ENABLE_ANTI_CMD_FLOOD`,
/// `G/cmd.cpp:612`), so Alpha lets a second pass first.
fn close_and_change_the_password(alice: &mut Keyed, database: &ScratchDatabase) {
    let command = |text: &[u8]| chat_packet(prodomo::chat::CHAT_COMMAND, 1, text);
    std::thread::sleep(Duration::from_millis(1200));
    alice.send_record(&slash(b"/safebox_close"));
    assert_eq!(alice.read_game(), command(b"CloseSafebox"));
    alice.send_record(&slash(b"/mall_close"));
    assert_eq!(alice.read_game(), command(b"CloseMall"));
    alice.send_record(&slash(b"/safebox_password 000000"));
    assert_eq!(alice.read_game(), info_to_alpha(b"[LS;828]"));
    alice.send_record(&slash(b"/mall_password 000000"));
    assert_eq!(alice.read_game(), info_to_alpha(b"[LS;528]"));
    alice.unanswered(&slash(b"/safebox_close"));
    alice.send_record(&slash(b"/safebox_change_password 000000 s3cret"));
    assert_eq!(alice.read_game(), info_to_alpha(b"[LS;774]"));
    alice.send_record(&slash(b"/safebox_change_password 000000 other"));
    assert_eq!(alice.read_game(), info_to_alpha(b"[LS;775]"));
    check(
        database,
        &format!(
            "(SELECT starts_with(password_hash, chr(36) || 'argon2id' || chr(36)) AND \
             strpos(password_hash, 's3cret') = 0 FROM safebox WHERE account_id = {ALICES})"
        ),
    );
}

/// `sys.npc.safebox`, the safebox and mall commands and the four store records
/// (`G/cmd_general.cpp`, `G/input_main.cpp:2276-2472`, `G/safebox.cpp`): the password opens the
/// account's one page of nine rows, a checkin, a move inside and a checkout carry the item and
/// its row, the mall shows its 27 rows and gives its item up, a close is a command line, and a
/// reload waits ten seconds after the last load or close, even when the password was wrong.
///
/// The store rows belong to the account, not the character (ADR-0005): what Alpha checks in,
/// Echo, on the same account, takes out, and bob's account has a safebox of its own. Every step
/// is stored before its records are sent (ADR-0003), so the store is checked as soon as they
/// arrive. The tenth slash line within a second closes the connection (`sys.net.flood`).
#[test]
fn an_account_keeps_items_in_its_safebox_and_takes_them_from_its_mall() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    let protos = owners_protos();
    let vnum = a_stackable_use_vnum(&protos);
    let proto = protos.get(vnum).expect("a proto");
    assert_eq!(
        proto.anti_flags & world::item::ITEM_ANTIFLAG_SAFEBOX,
        0,
        "an item that may be stored"
    );
    seat_the_depositors(&database, vnum);
    let (mut alice, _alpha) = enter_world(&server, b"alice", 0);
    // Given: Alpha's slot 2 names the item's cell 5.
    alice.send_record(&client_quickslot_add(2, 1, 5));
    assert_eq!(alice.read_game(), [GC_QUICKSLOT_ADD, 2, 1, 5]);

    // When: Alpha clicks the safebox, types seven bytes, then the password of an account with no
    // row, then opens it again. Then: the client is asked for the password, the long one is
    // refused before the store is asked, the default opens one empty page, and a second open is
    // refused.
    alice.send_record(&slash(b"/click_safebox"));
    let asked = chat_packet(prodomo::chat::CHAT_COMMAND, 1, b"ShowMeSafeboxPassword");
    assert_eq!(alice.read_game(), asked);
    alice.send_record(&slash(b"/safebox_password 0000000"));
    assert_eq!(alice.read_game(), info_to_alpha(b"[LS;526]"));
    alice.send_record(&slash(b"/safebox_password 000000"));
    assert_eq!(alice.read_game(), [GC_SAFEBOX_SIZE, 9]);
    alice.send_record(&slash(b"/safebox_password 000000"));
    let open = info_to_alpha(b"[LS;527]");
    assert_eq!(alice.read_game(), open, "the safebox is empty");
    check_in_and_move(&mut alice, &database, proto);
    take_from_the_mall(&mut alice, &database, proto);
    close_and_change_the_password(&mut alice, &database);

    // When: Alpha leaves, and Echo opens the safebox with the new password and takes the item
    // to cell 0. Then: the account's safebox shows it, and Echo's inventory shows it
    // highlighted, because it was checked in during another opening.
    drop(alice);
    server.wait_for("Character disconnected; wrote the row");
    let (mut echo, _echo) = enter_world(&server, b"alice", 2);
    echo.send_record(&slash(b"/safebox_password s3cret"));
    assert_eq!(echo.read_game(), [GC_SAFEBOX_SIZE, 9]);
    let (safebox, fields) = ((GC_SAFEBOX_SET, SAFEBOX_WINDOW), RelayedFields::distinct());
    assert_eq!(echo.read_game(), store_set(&fields, proto, safebox, 11, 7));
    echo.send_record(&client_store(CG_SAFEBOX_CHECKOUT, 11, 0));
    assert_eq!(echo.read_game(), store_del(GC_SAFEBOX_DEL, 11));
    let mut taken = fields.item_set(proto, 0, 7);
    taken[26] = 1;
    assert_eq!(echo.read_game(), taken);
    check(
        &database,
        "EXISTS (SELECT 1 FROM item WHERE id = 30 AND owner_id = (SELECT id FROM player WHERE \
         name = 'Echo') AND account_id IS NULL AND window_type = 1 AND pos = 0 AND count = 7)",
    );
    echo.quiet("the checkout is over");

    // When: Echo clicks the mall, and types three commands short of their last letter. Then: the
    // client is asked for the mall password, and each short one asks for the whole command
    // (`do_inputall`, `G/cmd.cpp:327-331`), which the walk reaches before the longer entry.
    echo.send_record(&slash(b"/click_mall"));
    let asked = chat_packet(prodomo::chat::CHAT_COMMAND, 1, b"ShowMeMallPassword");
    assert_eq!(echo.read_game(), asked);
    for short in [
        &b"/safebox_passwor"[..],
        b"/safebox_change_passwor",
        b"/mall_passwor",
    ] {
        echo.send_record(&slash(short));
        assert_eq!(echo.read_game(), info_to_alpha(b"[LS;916]"), "{short:?}");
    }

    // When: bob's Yankee tries alice's password, then the default. Then: bob's account has no
    // row, so alice's password is wrong, and the wrong one starts the wait all the same.
    let (mut bob, _yankee) = enter_world(&server, b"bob", 0);
    bob.send_record(&slash(b"/safebox_password s3cret"));
    assert_eq!(bob.read_game(), [GC_SAFEBOX_WRONG_PASSWORD]);
    bob.send_record(&slash(b"/safebox_password 000000"));
    assert_eq!(
        bob.read_game(),
        chat_packet(prodomo::chat::CHAT_INFO, 2, b"[LS;828]")
    );

    // When: Yankee lets a second pass and types ten lines. Then: nine are answered, and the
    // tenth within the second closes the connection (`ENABLE_ANTI_CMD_FLOOD`,
    // `G/cmd.cpp:612-625`).
    std::thread::sleep(Duration::from_millis(1200));
    let whole = chat_packet(prodomo::chat::CHAT_INFO, 2, b"[LS;916]");
    for _ in 0..9 {
        bob.send_record(&slash(b"/mall_passwor"));
        assert_eq!(bob.read_game(), whole);
    }
    bob.closed_by(&slash(b"/mall_passwor"));
}

/// [`enter_equipped_world_with`] for the four items [`give_alpha_equipment`] sets: the stone is
/// refused and its row kept.
fn enter_equipped_world(server: &Server, login: &[u8]) -> (Keyed, Listed, Entered) {
    enter_equipped_world_with(server, login, 4)
}

/// [`enter_world_seeing`] for a character that wears equipment at load, whose load sets
/// `item_count` items. The load lowers the spell points between the set-aside items and the gold
/// (`point_change(.., 7, 0, 600)` in
/// `a_relogged_character_wears_its_equipment_and_its_points_count_it`), and a worn weapon's attack
/// speed changes the points after the items, which the shared loader does not allow for; this
/// reads the load through to its final points record, then enters.
fn enter_equipped_world_with(
    server: &Server,
    login: &[u8],
    item_count: usize,
) -> (Keyed, Listed, Entered) {
    let (mut keyed, _empire, list) = select_screen(server, login);
    let character = listed(&list, 0);
    keyed.send_record(&client_select(0));
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_LOADING]);
    assert_eq!(keyed.read_game(), [GC_ENTITY, 3, 0]);
    assert_eq!(keyed.read_game()[0], GC_MAIN_CHARACTER2_EMPIRE);
    let gold = keyed.read_game();
    assert_eq!(gold[0], GC_CHARACTER_GOLD);
    let loaded = keyed.read_game();
    assert_eq!(loaded[0], GC_PLAYER_POINTS);
    keyed.loaded(&loaded);
    assert_eq!(keyed.read_game()[0], GC_SKILL_LEVEL_NEW);
    let mut record = keyed.read_game();
    let mut loaded_items = 0;
    while record[0] == ITEM_SET {
        loaded_items += 1;
        record = keyed.read_game();
    }
    assert_eq!(loaded_items, item_count, "the items the load sets");
    while record != gold {
        assert_eq!(record[0], GC_POINT_CHANGE, "{record:02x?}");
        record = keyed.read_game();
    }
    assert_eq!(keyed.read_game()[0], GC_PLAYER_POINTS);
    let entered = enter_game_view(&mut keyed);
    (keyed, character, entered)
}

#[test]
fn a_look_change_inside_the_window_carries_the_flag() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    seat_yankee_and_charlie(&database);
    give_alpha_equipment(&database);

    // Given: Alpha entered wearing the armour, and Yankee entered after it, both inside the five
    // seconds the revive runs from Alpha's entry.
    let entered_at = std::time::Instant::now();
    let (mut alpha_keyed, alpha, _alpha_entered) = enter_equipped_world(&server, b"alice");
    let (mut yankee, yankee_listed, _yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    alpha_keyed.sees_arrive(yankee_listed.id, false);
    let arrival = alpha_keyed.read_game_raw();
    assert!(
        alpha_keyed.timer_record(&arrival),
        "Yankee's entry is the timer's"
    );

    // When: the armour is taken off the body cell, inside the window. Then: the look update
    // Alpha is sent carries flag 28 in word 0, and the body part is bare.
    assert!(
        entered_at.elapsed() < Duration::from_millis(4_500),
        "the take-off is inside the window"
    );
    alpha_keyed.send_record(&client_item_move(180, 10, 0));
    let (records, quiet) = alpha_keyed.drain_game(Duration::from_millis(700), revive_width);
    assert_eq!(quiet, Quiet::Open);
    let looks: Vec<&Vec<u8>> = records
        .iter()
        .filter(|record| record[0] == GC_CHARACTER_UPDATE)
        .collect();
    assert_eq!(looks.len(), 1, "one look for the take-off: {records:02x?}");
    let own = looks[0];
    assert_eq!(&own[1..5], &alpha.id.to_le_bytes(), "Alpha's VID");
    assert_eq!(
        own[20..28],
        [0, 0, 0, 0x08, 0, 0, 0, 0],
        "the flag in its own look"
    );
    assert_eq!(own[5..7], [0, 0], "the body part is bare");

    // Then: Yankee, in range, is sent the same look with the flag.
    let seen = read_past_pings(&mut yankee);
    assert_eq!(seen[0], GC_CHARACTER_UPDATE, "{seen:02x?}");
    assert_eq!(&seen[1..5], &alpha.id.to_le_bytes(), "Alpha's VID");
    assert_eq!(
        seen[20..28],
        [0, 0, 0, 0x08, 0, 0, 0, 0],
        "the flag in the viewer's look"
    );
    assert_eq!(seen[5..7], [0, 0], "the body part is bare for the viewer");
}

#[test]
fn a_potion_drunk_inside_the_window_ticks_on_the_entry_phase() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    create_account(&server, "alice");
    add_characters(&database);
    sql(&database, "UPDATE player SET hp = 500 WHERE name = 'Alpha'");
    let inventory = common::item_slots::EWindows::Inventory as u8;
    let (hp, recovery) = (
        u8::try_from(common::point_slot::POINT_HP).expect("a point byte"),
        u8::try_from(common::point_slot::POINT_HP_RECOVERY).expect("a point byte"),
    );

    // Given: Alpha, at 500 hit points, enters, and the revive's five seconds run from the entry.
    let entered_at = std::time::Instant::now();
    let (mut alpha, _character, _items) = load_character(&server, b"alice", 0);
    let add = enter_game_burst(&mut alpha);
    let vid = u32::from_le_bytes(add[1..5].try_into().expect("four bytes"));
    Server::write_console(&console, "item give Alpha 27001 2");
    server.wait_for("id 100000000; the client has it");
    assert_eq!(set_fields(&alpha.read_game()), (inventory, 0, 27_001, 2));

    // When: one potion is drunk 30 pulses (1.2 s) after the entry, inside the window. Then: 300
    // hit points are owed, the red effect plays and the stack goes to one. The potion joins the
    // event the entry started, which its due pulse does not move (`StartAffectEvent` returns
    // when one runs, `G/char_affect.cpp:238`), so the next tick is the one at 50 pulses.
    std::thread::sleep(Duration::from_millis(1200).saturating_sub(entered_at.elapsed()));
    assert!(
        entered_at.elapsed() < Duration::from_millis(4_500),
        "the drink is inside the window"
    );
    alpha.send_record(&client_item_use(0));
    assert_eq!(
        read_a_move_within(&mut alpha, vid, Duration::from_millis(300)),
        [
            Seen::Point(recovery, 300),
            Seen::Effect(1),
            Seen::Count(0, 1),
        ]
    );

    // And: the ticks at 50, 75 and 100 pulses pay 99 each, from the owed 300.
    let mut paid = 0;
    for _ in 0..3 {
        paid += 99;
        assert_eq!(
            read_a_tick(&mut alpha, vid),
            [
                Seen::Point(hp, 500 + paid),
                Seen::Point(recovery, 300 - paid)
            ]
        );
    }

    // Then: the fifth tick, at 125 pulses, pays the last 3 and the revive ends. The recovery
    // records come before the removal and the flag update (`G/char_affect.cpp:175-211` before
    // `:226`): the removal to Alpha, then its look with no flag.
    assert_eq!(
        read_a_tick(&mut alpha, vid),
        [Seen::Point(hp, 800), Seen::Point(recovery, 0)],
        "the fifth tick pays the last 3"
    );
    assert_eq!(
        read_past_pings(&mut alpha),
        [GC_AFFECT_REMOVE, 215, 0, 0, 0, 0],
        "the removal follows the recovery"
    );
    let update = read_past_pings(&mut alpha);
    assert_eq!(update[0], GC_CHARACTER_UPDATE, "{update:02x?}");
    assert_eq!(&update[1..5], &vid.to_le_bytes(), "Alpha's VID");
    assert_eq!(&update[20..28], &[0; 8], "no flag left");
    alpha.quiet("nothing else follows the revive's end");
}

/// The width of each game record a wait for the revive or the refill can see, for
/// [`Keyed::drain_game`]: a look update is 55 bytes, every other record its [`game_len`].
fn revive_width(header: u8) -> usize {
    match header {
        GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
        other => game_len(other),
    }
}

/// The next record that is not a `GC_PING`, which arrives on its own timer and says nothing
/// about the wait. The record is read raw: the caller notes it if it is the revive's.
fn read_past_pings(keyed: &mut Keyed) -> Vec<u8> {
    loop {
        let record = keyed.read_game_raw();
        if record[0] != GC_PING {
            return record;
        }
    }
}

/// The records of a drain that are not pings.
fn without_pings(records: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    records
        .into_iter()
        .filter(|record| record[0] != GC_PING)
        .collect()
}

/// The revive-invisible affect as its entrant sees it (V9, C4 P1): the look update that carries
/// the flag, then the `GC_AFFECT_ADD` of the affect, and the phase after both. Alpha's own insert
/// carries no flag, because it is sent before the affect joins the entrant's event.
#[test]
fn an_entrant_is_sent_the_revive_update_before_its_affect_add() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);

    // When: Alpha enters the game. Then: its look update has flag 28 (bit 27 of word 0), and the
    // affect that follows is type 215 with apply 0, value 0, flag 28 (the number, not the mask),
    // duration 5 (ticks, so five seconds) and SP cost 0. `read_shown` reads the records in
    // the order they are asserted: the shown records, the list, the update, the affect.
    let (_alpha, alpha, entered) = enter_world_seeing(&server, b"alice", 0);
    let update = entered
        .shown
        .update
        .as_deref()
        .expect("the entrant's update");
    assert_eq!(update[0], GC_CHARACTER_UPDATE, "{update:02x?}");
    assert_eq!(&update[1..5], &alpha.id.to_le_bytes(), "its own VID");
    assert_eq!(&update[20..28], &[0, 0, 0, 0x08, 0, 0, 0, 0], "flag 28");
    assert_eq!(
        entered.add[27..31],
        [0, 0, 0, 0],
        "its own insert is sent before the affect is added"
    );
    assert_eq!(
        entered.affect,
        [
            GC_AFFECT_ADD, // header
            215,
            0,
            0,
            0, // dwType: AFFECT_REVIVE_INVISIBLE
            0, // bApplyOn: POINT_NONE
            0,
            0,
            0,
            0, // lApplyValue
            28,
            0,
            0,
            0, // dwFlag: the number, and not the mask
            5,
            0,
            0,
            0, // lDuration: five ticks
            0,
            0,
            0,
            0, // lSPCost
        ]
    );
}

/// A viewer in range is sent the entrant's arrival pair, then its look update with the flag
/// (V9, C4 P2). The insert is sent before the affect joins the entrant's event, so the update
/// is what tells the viewer the entrant is invisible.
#[test]
fn a_viewer_in_range_gets_the_arrival_pair_then_the_update() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    create_account(&server, "carol");
    add_characters(&database);
    seat_yankee_and_charlie(&database);

    // Given: Yankee is in game before Alpha, so Alpha's arrival is the viewer's to see.
    let (mut yankee, _yankee_listed, _yankee_entered) = enter_world_seeing(&server, b"bob", 0);

    // When: Alpha enters. Then: Yankee gets Alpha's pair, then the update with the flag.
    let (_alpha, alpha, _alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    yankee.sees_arrive(alpha.id, false);
    let update = yankee.read_game_raw();
    assert_eq!(update[0], GC_CHARACTER_UPDATE, "{update:02x?}");
    assert_eq!(&update[1..5], &alpha.id.to_le_bytes(), "Alpha's VID");
    assert_eq!(&update[20..28], &[0, 0, 0, 0x08, 0, 0, 0, 0], "flag 28");
    assert!(yankee.timer_record(&update), "the entry is the timer's");
    yankee.quiet("nothing else follows the entrant's update");
}

/// A viewer that enters inside the five seconds sees the flag in the entrant's insert, and the
/// expiry reaches the viewer as a look update with no flag (V9, C4 P3, P4). The entrant's own
/// insert carries no flag, which is the order `read_shown` relies on.
#[test]
fn an_entrant_within_five_seconds_sees_the_flag_in_the_add_and_its_expiry_clears_it() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    seat_yankee_and_charlie(&database);

    // Given: Alpha entered, and the affect runs its five seconds from there.
    let entered_at = std::time::Instant::now();
    let (mut alpha_keyed, alpha, _alpha_entered) = enter_world_seeing(&server, b"alice", 0);

    // When: Yankee enters within the five seconds. Then: Alpha's insert carries flag 28 in
    // word 0, and Yankee's own insert does not.
    let (mut yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    let (alpha_insert, _alpha_info) = yankee_entered.shown.pair_of(alpha.id);
    assert_eq!(
        alpha_insert[27..31],
        [0, 0, 0, 0x08],
        "within five seconds the insert carries flag 28"
    );
    assert_eq!(
        yankee_entered.add[27..31],
        [0, 0, 0, 0],
        "Yankee's own insert"
    );

    // Alpha is sent Yankee's arrival: the insert, its summary, and Yankee's look update with the
    // flag, which the timer notes as Yankee's.
    alpha_keyed.sees_arrive(yankee_listed.id, false);
    let arrival = alpha_keyed.read_game_raw();
    assert!(
        alpha_keyed.timer_record(&arrival),
        "Yankee's entry is the timer's"
    );

    // When: the five seconds are up. Then: Yankee gets Alpha's look update with no flag.
    let expiry = read_past_pings(&mut yankee);
    assert_eq!(expiry[0], GC_CHARACTER_UPDATE, "{expiry:02x?}");
    assert_eq!(&expiry[1..5], &alpha.id.to_le_bytes(), "Alpha's VID");
    assert_eq!(&expiry[20..28], &[0; 8], "no flag left");
    assert!(
        entered_at.elapsed() >= Duration::from_millis(4_500),
        "the flag lasts five seconds"
    );

    // Then: the entrant's own connection is sent the removal, then its look update with no flag.
    assert_eq!(
        read_past_pings(&mut alpha_keyed),
        [GC_AFFECT_REMOVE, 215, 0, 0, 0, 0],
        "the entrant's removal"
    );
    let own = read_past_pings(&mut alpha_keyed);
    assert_eq!(own[0], GC_CHARACTER_UPDATE, "{own:02x?}");
    assert_eq!(&own[1..5], &alpha.id.to_le_bytes(), "Alpha's own VID");
    assert_eq!(&own[20..28], &[0; 8], "no flag left on its own update");
    alpha_keyed.quiet("nothing else follows the entrant's expiry");

    // Then: Yankee's own expiry is the same pair on its connection, a little after Alpha's.
    assert_eq!(
        read_past_pings(&mut yankee),
        [GC_AFFECT_REMOVE, 215, 0, 0, 0, 0],
        "the viewer's own removal"
    );
    let yankee_own = read_past_pings(&mut yankee);
    assert_eq!(yankee_own[0], GC_CHARACTER_UPDATE, "{yankee_own:02x?}");
    assert_eq!(
        &yankee_own[1..5],
        &yankee_listed.id.to_le_bytes(),
        "Yankee's own VID"
    );
    assert_eq!(
        &yankee_own[20..28],
        &[0; 8],
        "no flag left on Yankee's update"
    );
}

/// The revive ends five seconds after entering: the entrant gets `GC_AFFECT_REMOVE`, then the
/// look update with no flag, and the viewer gets the update (V9, C4 P4, item 12). Alpha enters
/// first, so its five seconds are the ones measured, and Yankee's own expiry follows on Alpha's
/// connection a moment later. Alpha's stamina is at its maximum, 890 (800 + 5 × 18), so the affect
/// event has nothing else to run for and ends with it. The legacy `LoadAffect` restores the
/// affect on a reload (G1), which the Rewrite does not do yet.
#[test]
fn the_revive_ends_after_five_seconds_with_remove_then_update() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    seat_yankee_and_charlie(&database);
    stamina(&database, "Alpha", 890);

    // Given: Alpha enters, and Yankee enters after it and sees Alpha's arrival. The windows are
    // measured from before Alpha's entry, so they end no later than Alpha's five seconds would.
    let entered_at = std::time::Instant::now();
    let (mut alpha_keyed, alpha, _alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    alpha_keyed.sees_arrive(yankee_listed.id, false);
    let arrival = alpha_keyed.read_game_raw();
    assert!(
        alpha_keyed.timer_record(&arrival),
        "the entry is the timer's"
    );

    // Then: nothing ends the revive before five seconds (4.5 s from the entry).
    let until_early = Duration::from_millis(4_500).saturating_sub(entered_at.elapsed());
    let (early, quiet) = alpha_keyed.drain_game(until_early, revive_width);
    assert_eq!(quiet, Quiet::Open);
    assert_eq!(
        without_pings(early),
        Vec::<Vec<u8>>::new(),
        "nothing before five seconds"
    );

    // Then: the removal reaches the entrant first, then its look update with no flag. Yankee's
    // own expiry is sent to Alpha's connection too, a moment later, and is the only other record.
    let until_ended = Duration::from_millis(7_000).saturating_sub(entered_at.elapsed());
    let (ended, quiet) = alpha_keyed.drain_game(until_ended, revive_width);
    assert_eq!(quiet, Quiet::Open);
    let ended = without_pings(ended);
    assert!(ended.len() >= 2, "{ended:02x?}");
    assert_eq!(
        ended[0],
        [GC_AFFECT_REMOVE, 215, 0, 0, 0, 0],
        "the removal of type 215 and apply 0"
    );
    assert_eq!(ended[1][0], GC_CHARACTER_UPDATE, "{:02x?}", ended[1]);
    assert_eq!(&ended[1][1..5], &alpha.id.to_le_bytes(), "Alpha's VID");
    assert_eq!(&ended[1][20..28], &[0; 8], "the flags are gone");
    for later in &ended[2..] {
        assert_eq!(later[0], GC_CHARACTER_UPDATE, "{later:02x?}");
        assert_eq!(
            &later[1..5],
            &yankee_listed.id.to_le_bytes(),
            "Yankee's own expiry"
        );
        assert_eq!(&later[20..28], &[0; 8], "no flag left on Yankee either");
    }
    alpha_keyed.quiet("nothing else follows the expiries");

    // Then: the viewer gets Alpha's look update with no flag, as the next record on its
    // connection after its own entry.
    let seen = read_past_pings(&mut yankee);
    assert_eq!(seen[0], GC_CHARACTER_UPDATE, "{seen:02x?}");
    assert_eq!(&seen[1..5], &alpha.id.to_le_bytes(), "Alpha's VID");
    assert_eq!(&seen[20..28], &[0; 8], "the viewer sees no flag");
}

/// An accepted attack ends the revive before its relay: the entrant gets the look update with
/// no flag, then `GC_AFFECT_REMOVE`, and the viewer gets the update and then the move (V9, C6
/// item 13, C4 P5). The mover is not relayed its own move.
#[test]
fn an_attack_step_ends_the_revive_before_the_move_relay() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    seat_yankee_and_charlie(&database);

    let (mut yankee, _yankee_listed, _yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    let (mut alpha_keyed, alpha, _alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    yankee.sees_arrive(alpha.id, false);
    let entry = yankee.read_game_raw();
    assert!(yankee.timer_record(&entry), "the entry is the timer's");

    // When: Alpha swings, standing where it is. Then: its own update has no flag, and the
    // removal follows it.
    let attack = client_move(FUNC_ATTACK, 7, 40, 470_100, 950_100, 0x5eed);
    alpha_keyed.send_record(&attack);
    let update = read_past_pings(&mut alpha_keyed);
    assert_eq!(update[0], GC_CHARACTER_UPDATE, "{update:02x?}");
    assert_eq!(&update[20..28], &[0; 8], "the attack takes the flag off");
    assert!(
        alpha_keyed.timer_record(&update),
        "the flag goes with the update"
    );
    assert_eq!(
        read_past_pings(&mut alpha_keyed),
        [GC_AFFECT_REMOVE, 215, 0, 0, 0, 0],
        "the removal follows the update"
    );
    alpha_keyed.quiet("the mover is not relayed its own attack");

    // Then: the viewer gets the update, and then the relayed move.
    let update = yankee.read_game_raw();
    assert_eq!(update[0], GC_CHARACTER_UPDATE, "{update:02x?}");
    assert_eq!(&update[20..28], &[0; 8], "the viewer sees no flag");
    assert!(yankee.timer_record(&update), "the expiry is the timer's");
    assert_eq!(
        yankee.read_game(),
        relayed_move(&attack, alpha.id, 0),
        "the attack's relay follows the update"
    );
}

/// A walking entrant has its stamina refilled three seconds after it stopped (V3a, C6 item 16):
/// a `GC_POINT_CHANGE` to slot 9 carrying the maximum, 890 (800 + 5 × 18). A viewer that enters
/// before the refill sees the entrant walking.
#[test]
fn a_character_below_max_stamina_is_refilled_about_three_seconds_after_entering() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    seat_yankee_and_charlie(&database);
    stamina(&database, "Alpha", 0);

    // Given: Alpha enters with no stamina, so it walks, and Yankee enters at once.
    let entered_at = std::time::Instant::now();
    let (mut alpha_keyed, alpha, _alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    let (_yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    assert!(
        yankee_entered
            .shown
            .records
            .contains(&walk_mode_of(&alpha.id.to_le_bytes())),
        "a newcomer sees a walking entrant walk"
    );

    // Alpha is sent Yankee's arrival before any refill: the insert, its summary, and Yankee's
    // look update with the flag. The refill is read past them.
    alpha_keyed.sees_arrive(yankee_listed.id, false);
    let arrival = alpha_keyed.read_game_raw();
    assert_eq!(arrival[0], GC_CHARACTER_UPDATE, "{arrival:02x?}");
    assert_eq!(
        &arrival[1..5],
        &yankee_listed.id.to_le_bytes(),
        "Yankee's VID"
    );

    // When: the refill is due. Then: it is a point change to stamina at the maximum, and it
    // comes about three seconds after the entry, not before.
    let refill = read_past_pings(&mut alpha_keyed);
    let waited = entered_at.elapsed();
    assert_eq!(refill[0], GC_POINT_CHANGE, "{refill:02x?}");
    assert_eq!(&refill[4..8], &alpha.id.to_le_bytes(), "Alpha's VID");
    assert_eq!(
        refill[8],
        u8::try_from(POINT_STAMINA).expect("a point byte"),
        "slot 9 is stamina"
    );
    assert_eq!(
        i64::from_le_bytes(refill[17..25].try_into().expect("eight bytes")),
        890,
        "refilled to the maximum"
    );
    assert!(
        waited >= Duration::from_millis(2_800),
        "the refill waits three seconds, not {waited:?}"
    );
}

/// A move restarts the three-second wait of a walking entrant, so its refill comes three
/// seconds after the move (V3a, C6 item 17). Without the move the refill would come at three
/// seconds after the entry.
#[test]
fn a_move_resets_the_stop_time_and_delays_the_refill() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    stamina(&database, "Alpha", 0);

    // Given: Alpha enters with no stamina and walks, and it moves 1.5 s after entering, while
    // the refill is still waiting.
    let entered_at = std::time::Instant::now();
    let (mut alpha_keyed, _alpha, _alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    std::thread::sleep(Duration::from_millis(1_500));
    alpha_keyed.send_record(&client_move(FUNC_MOVE, 0, 0, 470_200, 950_200, 0x5eee));

    // Then: the refill comes three seconds after the move, which is past 4.4 s after the entry.
    let refill = read_past_pings(&mut alpha_keyed);
    let waited = entered_at.elapsed();
    assert_eq!(refill[0], GC_POINT_CHANGE, "{refill:02x?}");
    assert_eq!(
        refill[8],
        u8::try_from(POINT_STAMINA).expect("a point byte"),
        "slot 9 is stamina"
    );
    assert!(
        waited >= Duration::from_millis(4_400),
        "the move restarts the wait, so the refill is not at three seconds: {waited:?}"
    );
}
