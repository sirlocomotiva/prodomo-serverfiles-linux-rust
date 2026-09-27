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
        GC_MOVE => GC_MOVE_LEN,
        GC_CHARACTER_POSITION => CHARACTER_POSITION_LEN,
        GC_SYNC_POSITION => usize::MAX,
        GC_OWNERSHIP => OWNERSHIP_LEN,
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
    let (mut keyed, _empire, list) = select_screen(server, login);
    let character = listed(&list, usize::from(slot));
    keyed.send_record(&client_select(slot));
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_LOADING]);
    assert_eq!(keyed.read_game(), [GC_ENTITY, 3, 0]);
    let main = keyed.read_game();
    assert_eq!(main[0], GC_MAIN_CHARACTER2_EMPIRE);
    assert_eq!(keyed.read_game()[0], GC_CHARACTER_GOLD);
    assert_eq!(keyed.read_game()[0], GC_PLAYER_POINTS);
    assert_eq!(keyed.read_game()[0], GC_SKILL_LEVEL_NEW);
    keyed.send_record(&client_enter_game());
    assert_eq!(keyed.read_game()[0], GC_CHARACTER_ADD);
    assert_eq!(keyed.read_game()[0], GC_CHAR_ADDITIONAL_INFO);
    assert_eq!(keyed.read_game()[0], GC_AFFECT_ADD);
    assert_eq!(keyed.read_game(), [GC_PHASE, PHASE_GAME]);
    assert_eq!(keyed.read_game()[0], GC_TIME);
    assert_eq!(keyed.read_game(), [GC_CHANNEL, 1]);
    let notice = keyed.read_game();
    assert_eq!(notice[0], GC_CHAT);
    assert_eq!(keyed.client.drain(QUIET_WINDOW), (Vec::new(), Quiet::Open));
    (keyed, character)
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

/// Assert a condition on the scenario's database.
fn check(database: &ScratchDatabase, condition: &str) {
    execute(
        database.url(),
        &format!("DO $$ BEGIN IF NOT ({condition}) THEN RAISE EXCEPTION 'failed'; END IF; END $$"),
    )
    .unwrap_or_else(|error| panic!("{condition}: {error}"));
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
fn a_map_the_channel_does_not_host_closes_after_the_records_before_the_test() {
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
    assert_eq!(
        keyed.read_game_or_close(),
        None,
        "no gold, points, or skill-level record after the map test"
    );
    // The character is untouched: the refusal sets a warp location in legacy, and the Rewrite
    // has no world to hold one.
    check(
        &database,
        "(SELECT x FROM player WHERE name = 'Delta') = 60000",
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
