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
