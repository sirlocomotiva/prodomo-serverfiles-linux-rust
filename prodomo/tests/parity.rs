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
/// length at bytes 1 and 2 covering the whole record. `GC_ENTITY` (249) is the same shape. The
/// rest of the two bursts are fixed width, and those widths are the ones the loading phase's own
/// golden-byte tests pin.
fn dynamic_len(header: u8) -> usize {
    match header {
        GC_CHARACTER_ADD | GC_CHAR_ADDITIONAL_INFO | GC_CHAT | GC_ENTITY | GC_SYNC_POSITION => {
            usize::MAX
        }
        other => panic!("{other} is a fixed-width loading or enter-game record"),
    }
}

/// A record whose `WORD wSize` covers the whole record, or `None` when the header is a
/// fixed-width one. Reading it is how the harness sizes a variable record.
fn word_sized(record: &[u8]) -> Option<usize> {
    let header = record[0];
    if matches!(header, GC_CHAT | GC_ENTITY | GC_SYNC_POSITION) {
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
const GC_AFFECT_ADD: u8 = 126;
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
        GC_ENTITY | GC_CHAT => dynamic_len(header),
        GC_AFFECT_ADD => AFFECT_ADD_LEN,
        GC_TIME => TIME_LEN,
        GC_CHANNEL => CHANNEL_LEN,
        GC_POINT_CHANGE => POINT_CHANGE_LEN,
        GC_MOVE => GC_MOVE_LEN,
        GC_CHARACTER_POSITION => CHARACTER_POSITION_LEN,
        GC_SYNC_POSITION => usize::MAX,
        GC_OWNERSHIP => OWNERSHIP_LEN,
        ITEM_SET => ITEM_SET_LEN,
        ITEM_UPDATE => ITEM_UPDATE_LEN,
        GROUND_ADD => GROUND_ADD_LEN,
        GROUND_DEL => GROUND_DEL_LEN,
        GC_QUICKSLOT_ADD => 1 + 1 + 2,
        GC_QUICKSLOT_DEL => 1 + 1,
        GC_QUICKSLOT_SWAP => 1 + 2,
        other => panic!("unexpected loading or enter-game header {other}"),
    }
}

/// `cg.world.move`, `sys.world.move`: the headers of the movement and chat records, the widths the
/// loading phase's golden-byte tests pin, and the record builders a client needs to play them.
const CG_CHAT: u8 = 0x03;
const CG_MOVE: u8 = 0x07;
const CG_SYNC_POSITION: u8 = 0x08;
const CG_CHARACTER_POSITION: u8 = 0x1c;
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
    let (mut keyed, character, _items) = load_character(server, login, slot);
    enter_game_burst(&mut keyed);
    (keyed, character)
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
    let (mut keyed, _empire, list) = select_screen(server, login);
    let character = listed(&list, usize::from(slot));
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
    (keyed, character, quickslots, items)
}

/// Send `CG_ENTER_GAME` from the loading phase and read the enter-game burst, leaving the
/// connection in the game phase with nothing unread. Answers the character's own
/// `GC_CHARACTER_ADD`.
fn enter_game_burst(keyed: &mut Keyed) -> Vec<u8> {
    let add = enter_game_records(keyed);
    assert_eq!(keyed.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));
    add
}

/// [`enter_game_burst`] without the final quiet check, for a map whose ground is not empty.
fn enter_game_records(keyed: &mut Keyed) -> Vec<u8> {
    keyed.send_record(&client_enter_game());
    let add = keyed.read_game();
    assert_eq!(add[0], GC_CHARACTER_ADD);
    assert_eq!(keyed.read_game()[0], GC_CHAR_ADDITIONAL_INFO);
    assert_eq!(keyed.read_game()[0], GC_AFFECT_ADD);
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_GAME]);
    assert_eq!(keyed.read_game()[0], GC_TIME);
    assert_eq!(keyed.read_game(), [GC_CHANNEL, 1]);
    let notice = keyed.read_game();
    assert_eq!(notice[0], GC_CHAT);
    add
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

    /// Assert that nothing but a `GC_PING` cycle arrives.
    ///
    /// A ping is on its own timer and says nothing about the record just sent, so a ping
    /// inside the window is not an answer. Anything else is.
    fn quiet(&mut self, note: &str) {
        let key = self.output;
        let deadline = std::time::Instant::now() + QUIET_WINDOW;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return;
            }
            match self.client.drain(left) {
                (bytes, Quiet::Open) if bytes.is_empty() => return,
                (bytes, Quiet::Open) => {
                    assert_eq!(
                        bytes.len() % 8,
                        0,
                        "the wire carries whole TEA units ({note})"
                    );
                    for unit in bytes.chunks(8) {
                        let plain = decrypt_padded(unit, &key).expect("aligned");
                        assert_eq!(
                            plain[0], GC_PING,
                            "only a ping may arrive while nothing is expected ({note})"
                        );
                    }
                }
                (_, closed) => panic!("{note} closed the connection: {closed:?}"),
            }
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

    /// Read the next loading or enter-game record, sizing a `WORD wSize` record from the first
    /// TEA unit and a fixed-width one from [`game_len`].
    fn read_game(&mut self) -> Vec<u8> {
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
    let (_auth, key) = login_key(server, login);
    let mut keyed = Keyed::channel(server.channel(1));
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
    // `MainCharacterPacket` is the 46-byte empire variant, carrying the VID, the job, the Name,
    // the position, the empire, and the skill group, in source field order.
    let main = keyed.read_game();
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
    // `PointsPacket` writes the gold record immediately before the points record, because
    // `ENABLE_REMOVE_LIMIT_GOLD` is on.
    let gold = keyed.read_game();
    assert_eq!(gold.len(), GOLD_LEN);
    assert_eq!(gold[0], GC_CHARACTER_GOLD);
    assert_eq!(&gold[1..9], &0u64.to_le_bytes(), "the stored gold");
    let points = keyed.read_game();
    assert_eq!(points.len(), POINTS_LEN);
    assert_eq!(points[0], GC_PLAYER_POINTS);
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

    // `CG_ENTER_GAME`. `Entergame` writes the own-character pair, then the revive-invisible
    // affect, then `SetPhase(PHASE_GAME)`, then the time, Channel, and event records.
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
    assert_eq!(additional[69], 0, "bLanguage from the descriptor");
    let affect = keyed.read_game();
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
/// sender, and no client on another Channel or another map. A line that is only whitespace, an
/// empty line, and a line whose declared size is under the fixed part are all consumed without a
/// record, and the tenth line in a run schedules a disconnect.
#[test]
fn a_talking_line_reaches_the_map_including_its_sender() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    // Bob's character stands on another map, so the map filter has something to exclude.
    sql(
        &database,
        "UPDATE player SET x = 470000, y = 950000 WHERE name = 'Zulu'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         470000, 950000 FROM account WHERE login = 'bob'",
    );

    let (mut alice, alpha) = enter_world(&server, b"alice", 0);
    let (mut yankee, _) = enter_world(&server, b"bob", 0);

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
    // nothing and is consumed: the Rewrite has no command interpreter yet.
    alice.unanswered(&client_chat(CHAT_TALKING, b"/who"));

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
    let (mut yankee, _) = enter_world(&server, b"bob", 0);

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

/// `cg.world.move`: an accepted move reaches the clients around the mover and never the mover,
/// and a move past the legacy distance limit is refused without a record. The moved position is
/// the client's own bytes, so the broadcast relays `lX` and `lY` unchanged and carries the
/// duration only on the `FUNC_MOVE` branch.
#[test]
fn a_move_reaches_the_map_around_the_mover_and_not_the_mover() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         470000, 950000 FROM account WHERE login = 'bob'",
    );

    let (mut alice, alpha) = enter_world(&server, b"alice", 0);
    let (mut yankee, _) = enter_world(&server, b"bob", 0);

    // `FUNC_COMBO` is 3. A step lands at the position, and the record carries no duration.
    alice.send_record(&client_move(3, 7, 40, 470_100, 950_100, 0x5eed));
    let seen = yankee.read_game();
    assert_eq!(seen.len(), GC_MOVE_LEN);
    assert_eq!(seen[0], GC_MOVE);
    assert_eq!(seen[1], 3, "bFunc is relayed");
    assert_eq!(seen[2], 7, "bArg is relayed");
    assert_eq!(seen[3], 40, "bRot is relayed, not multiplied, on the wire");
    assert_eq!(&seen[4..8], &alpha.id.to_le_bytes(), "dwVID");
    assert_eq!(&seen[8..12], &470_100i32.to_le_bytes(), "lX");
    assert_eq!(&seen[12..16], &950_100i32.to_le_bytes(), "lY");
    assert_eq!(&seen[16..20], &0x5eedu32.to_le_bytes(), "dwTime");
    assert_eq!(&seen[20..24], &0u32.to_le_bytes(), "dwDuration");
    alice.quiet("PacketAround excludes the mover");

    // `FUNC_MOVE` is 1 and it is the branch that calls `Goto`, so the record carries the duration
    // the client sent. The distance test compares against 999 units of 100, so 200000 is refused.
    alice.send_record(&client_move(1, 0, 0, 470_200, 950_200, 0x5eee));
    let stepped = yankee.read_game();
    assert_eq!(stepped[1], 1, "bFunc");
    assert_eq!(&stepped[8..12], &470_200i32.to_le_bytes(), "lX");
    alice.quiet("the mover is still excluded");
    alice.send_record(&client_move(1, 0, 0, 470_000 + 200_000, 950_000, 0x5eef));
    yankee.quiet("a refused move sends nothing to anyone");
    alice
        .quiet("and the mover only has the legacy Show record, which the Rewrite has no world for");

    // A function byte of 6 is past `FUNC_MAX_NUM`, which is the first refused value.
    alice.unanswered(&client_move(6, 0, 0, 470_100, 950_100, 0x5ef0));
    // A function byte with the skill bit set passes the range test and steps.
    alice.send_record(&client_move(0x80, 0, 0, 470_300, 950_300, 0x5ef1));
    let skill = yankee.read_game();
    assert_eq!(skill[1], 0x80, "bFunc is relayed");
    assert_eq!(&skill[8..12], &470_300i32.to_le_bytes(), "lX");
}

/// `sys.char.position`: sitting and standing reach every client on the map including the
/// sender, because `Standup` and `Sitdown` call `PacketAround` with no `except`. A pose the
/// character is already in is ignored, and legacy collapses the ground pose onto the chair
/// value, which is a Defect the Rewrite does not reproduce.
#[test]
fn a_pose_reaches_the_map_including_its_sender() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         470000, 950000 FROM account WHERE login = 'bob'",
    );

    let (mut alice, alpha) = enter_world(&server, b"alice", 0);
    let (mut yankee, _) = enter_world(&server, b"bob", 0);

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

    // `Sitdown(1)` is the other arm of the same switch and reaches the same state.
    alice.send_record(&client_position(POSITION_SITTING_GROUND));
    let ground = alice.read_game();
    assert_eq!(ground[5], POSITION_SITTING_GROUND);
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
    // An unknown pose byte is not a legacy arm, so nothing is sent.
    alice.unanswered(&client_position(0x7f));
}

/// `sys.world.move`: a sync batch is relayed around the claimer and never to the claimer, and an
/// unknown VID or a victim of the wrong kind is skipped while the rest of the batch still goes
/// out.
#[test]
fn a_sync_batch_is_relayed_around_the_claimer_only() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         470000, 950000 FROM account WHERE login = 'bob'",
    );

    let (mut alice, alpha) = enter_world(&server, b"alice", 0);
    let (mut yankee, yankee_id) = enter_world(&server, b"bob", 0);

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
    // goes around the map with no exception, so Alice reads it and Yankee does not read the
    // position batch that follows it.
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
    let server = Server::start(binary(), database.url());
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
        .find(|proto| {
            proto.size == 1
                && proto.flags & world::item::ITEM_FLAG_STACKABLE != 0
                && proto.anti_flags & world::item::ITEM_ANTIFLAG_STACK == 0
                && (0..6).all(|category| {
                    !gamedata::item_custom_category::is_custom_category(proto, category)
                })
        })
        .map(|proto| proto.vnum)
        .expect("the owner's data has a stackable one-cell item outside every bank")
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
    let (records, quiet) = keyed.drain_game(Duration::from_millis(700), |header| match header {
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
/// half the armour's.
#[test]
fn an_armour_is_worn_swapped_and_taken_off_and_the_store_follows() {
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
/// event pays 7% of the maximum each second from a second after the first, and the logout save
/// keeps the paid hit points.
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
    assert_eq!(
        read_a_move(&mut alpha, vid),
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
    let mut paid = 0;
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
    assert_eq!(
        alpha.client.drain(Duration::from_millis(1200)),
        (Vec::new(), Quiet::Open)
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

/// A `GC_ITEM_GROUND_ADD` at Alpha's and Zulu's position, with z 0.
fn a_ground_add(vid: u32, vnum: u32) -> Vec<u8> {
    let mut record = vec![GROUND_ADD];
    for word in [470_000_i32, 950_000, 0] {
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

/// `cg.game.item_drop`, `cg.game.item_drop2`, `cg.game.item_pickup`: part of a stack is
/// dropped, a drop inside the second after it is refused, the rest is dropped whole, and each
/// part is picked up again, one by a character that entered the map after it fell. Every client
/// on the map sees each item fall and go, and the store follows each step.
#[test]
fn a_dropped_stack_lies_on_the_map_until_someone_picks_it_up() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) =
        Server::start_with_console(binary(), database.url(), &default_channels());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Zulu', 1, 470000, \
         950000 FROM account WHERE login = 'bob'",
    );
    let protos = owners_protos();
    let stackable = a_stackable_vnum(&protos);
    let mut picked_up = b"[LS;444;".to_vec();
    picked_up.extend_from_slice(&protos.get(stackable).expect("a proto").locale_name);
    picked_up.push(b']');
    let inventory = common::item_slots::EWindows::Inventory as u8;

    // Given: Alpha holds ten of a stackable item at cell 0.
    let (mut alpha, _character) = enter_world(&server, b"alice", 0);
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
    let (mut zulu, _items) = {
        let (mut keyed, _character, items) = load_character(&server, b"bob", 0);
        enter_game_records(&mut keyed);
        (keyed, items)
    };
    // Then: the item lying there is shown.
    assert_eq!(zulu.read_game(), a_ground_add(1, stackable));

    // When: once the second has passed, Alpha drops the rest whole.
    std::thread::sleep(Duration::from_millis(1100).saturating_sub(dropped_at.elapsed()));
    alpha.send_record(&client_item_drop(0, None));
    // Then: the cell is cleared, the six fall under ground VID 2, Zulu sees them fall, and the
    // row is gone.
    assert_eq!(alpha.read_game(), a_clear_record(0));
    assert_eq!(alpha.read_game(), a_ground_add(2, stackable));
    assert_eq!(alpha.read_game(), an_info_line(b"[LS;443]"));
    assert_eq!(zulu.read_game(), a_ground_add(2, stackable));
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
    // And: an item already picked up is not there to pick up, and nobody is told.
    alpha.unanswered(&protocol::cg_item_pickup::CgItemPickup::new(1).encode());
    assert_eq!(zulu.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));
}

/// `event.item.item_destroy_event`: with a two-second lifetime, an item nobody picks up is
/// destroyed on time and every client on the map sees it go. A `CG_ITEM_DROP2` carrying gold
/// is ignored before the drop limit, because `DropGold` is not ported.
#[test]
fn a_dropped_item_nobody_picks_up_is_destroyed_on_time_and_the_map_sees_it_go() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (mut server, console) = Server::start_with_console_configured(
        binary(),
        database.url(),
        &default_channels(),
        "item_destroy_time_dropitem = 2",
    );
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Zulu', 1, 470000, \
         950000 FROM account WHERE login = 'bob'",
    );
    let stackable = a_stackable_vnum(&owners_protos());
    let inventory = common::item_slots::EWindows::Inventory as u8;

    // Given: Alpha holds three of a stackable item at cell 0, and Zulu stands beside it.
    let (mut alpha, _character) = enter_world(&server, b"alice", 0);
    Server::write_console(&console, &format!("item give Alpha {stackable} 3"));
    server.wait_for("id 100000000; the client has it");
    assert_eq!(set_fields(&alpha.read_game()), (inventory, 0, stackable, 3));
    let mut zulu = {
        let (mut keyed, _character, _items) = load_character(&server, b"bob", 0);
        enter_game_records(&mut keyed);
        keyed
    };

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
    assert_eq!(keyed.read_game()[0], GC_AFFECT_ADD);
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
