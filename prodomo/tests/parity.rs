//! Parity scenarios: the scripted client plays the real `prodomo` binary over TCP.
//!
//! Each scenario is a `#[test]` whose name an inventory row in `.scratch/parity/` gives in its
//! `scenario` column; a row is `ported` only when its scenario is here and passes.
//! `inventory_rows_keep_the_rules` checks that link. The scenarios need a store, so they run only
//! when `DATABASE_URL` is set; the inventory check always runs.

#![cfg(unix)]

mod support;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::cfloat::{f32_to_i32, i32_to_f32};
use gamedata::mob_proto::{CHAR_TYPE_MONSTER, CHAR_TYPE_STONE};
use parity::client::Quiet;
use parity::inventory::{self, Status};
use parity::server::default_channels;
use parity::{Client, Server};
use prodomo::descriptor_crypto::derive_legacy_descriptor_keys;
use protocol::cg_inventory::{resolve_cg_base_size, CG_KEEP_ALIVE, HEADER_CG_PONG};
use protocol::cg_mark::{CgMarkCRCList, CgMarkIdxList, CgMarkLogin, CgMarkUpload, CgSymbolCrc};
use protocol::cg_variable::encode_guild_symbol_upload;
use protocol::gc_chat::{CHAT_TYPE_BIG_NOTICE, CHAT_TYPE_NOTICE, CHAT_TYPE_TALKING};
use protocol::gc_guild::{
    GUILD_NAME_FIELD_SIZE, GUILD_SUBHEADER_GC_CHANGE_EXP, GUILD_SUBHEADER_GC_CHANGE_MEMBER_GENERAL,
    GUILD_SUBHEADER_GC_CHANGE_MEMBER_GRADE, GUILD_SUBHEADER_GC_COMMENTS, GUILD_SUBHEADER_GC_GRADE,
    GUILD_SUBHEADER_GC_GRADE_AUTH, GUILD_SUBHEADER_GC_GRADE_NAME, GUILD_SUBHEADER_GC_GUILD_INVITE,
    GUILD_SUBHEADER_GC_GUILD_NAME, GUILD_SUBHEADER_GC_GUILD_WAR_LIST, GUILD_SUBHEADER_GC_INFO,
    GUILD_SUBHEADER_GC_LIST, GUILD_SUBHEADER_GC_LOGIN, GUILD_SUBHEADER_GC_LOGOUT,
    GUILD_SUBHEADER_GC_REMOVE, GUILD_SUBHEADER_GC_SKILL_INFO, GUILD_SUBHEADER_GC_WAR,
    GUILD_SUBHEADER_GC_WAR_END_LIST, GUILD_SUBHEADER_GC_WAR_SCORE,
};
use protocol::gc_mark::{HEADER_GC_MARK_BLOCK, HEADER_GC_MARK_IDXLIST, HEADER_GC_SYMBOL_DATA};
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

/// Why a `ported` row's scenario cell names no test, or `None` when it names one.
///
/// A bare name is a scenario in this file. A `path::name` cell names a world test in the file
/// at `path`, relative to the workspace root: the owner accepts one where a binary scenario is
/// impractical, such as an ore vein's 7 to 15 minute life (2026-10-14, `.scratch/parity/spec.md`).
fn scenario_problem(cell: &str) -> Option<String> {
    let Some((path, name)) = cell.rsplit_once("::") else {
        let signature = format!("fn {cell}()");
        return (!SCENARIOS.contains(&signature)).then(|| {
            format!("names `{cell}`, which is not a scenario in prodomo/tests/parity.rs")
        });
    };
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(path);
    let Ok(text) = std::fs::read_to_string(&source) else {
        return Some(format!(
            "names `{path}`, which is not a file of the workspace"
        ));
    };
    let marker = format!("#[test]\n    fn {name}() {{");
    (!text.contains(&marker)).then(|| format!("names `{name}`, which is not a test in `{path}`"))
}

#[test]
fn a_scenario_cell_names_a_scenario_here_or_a_world_test_by_its_path() {
    assert_eq!(scenario_problem("inventory_rows_keep_the_rules"), None);
    let world = "prodomo/src/game_state/regen.rs::\
                 an_ore_vein_ends_at_its_time_and_its_regen_stands_another";
    assert_eq!(scenario_problem(world), None);
    // Controls: an unknown name, a helper that is not a test, and a file that does not exist.
    assert!(scenario_problem("zzqx_no_such_scenario").is_some());
    assert!(scenario_problem("prodomo/src/game_state/regen.rs::a_map").is_some());
    assert!(scenario_problem("prodomo/src/zzqx.rs::a_map").is_some());
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
        if let Some(problem) = scenario_problem(&row.scenario) {
            problems.push(format!("{}: `{}` {problem}", row.file, row.id));
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
///
/// After any `INSERT INTO player`, the pools are filled to the item-less maxima
/// (`JOB_INITIAL_POINTS[job % 4]` with `hp_per_ht` 40, `sp_per_iq` 20 and a stored `random_hp`
/// of 0), so the live heal stays silent and the quiet reads hold. An explicit pool (Gamma, a
/// created character, a pool a scenario sets later) is untouched (ledger 229l).
fn sql(database: &ScratchDatabase, statement: &str) {
    execute(database.url(), statement).expect("the statement should succeed");
    if statement.trim_start().starts_with("INSERT INTO player") {
        execute(
            database.url(),
            "UPDATE player SET hp = (ARRAY[600,650,650,700])[job % 4 + 1] + ht * 40, sp = 200              + iq * 20 WHERE hp = 0 AND sp = 0",
        )
        .expect("the fill should succeed");
    }
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

/// Create `login` with [`ACCOUNT_PASSWORD`] before the server boots: the Operator
/// command against a scratch config naming the database, so the guild seed that
/// needs the player rows can land before the boot installs the stored guilds.
fn create_account_before_boot(database: &ScratchDatabase, login: &str) {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    // Per database, not per process: the tests run in threads of one binary, so a
    // PID-and-login name made two concurrent pre-boot commands share one config and
    // race their database URLs (the guild war scenarios all make `alice`).
    let dir = std::env::temp_dir().join(format!("prodomo-preboot-{}", database.name()));
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    std::fs::write(
        dir.join("prodomo.toml"),
        format!(
            "bind_ip = \"127.0.0.1\"\n[store]\nurl = \"{}\"\n\n[auth]\nport = 0\n\n[[channel]]\nnumber \
             = 1\nports = [0, 0]\nmaps = [1, 3]\n",
            database.url()
        ),
    )
    .expect("a scratch config");
    let password = std::str::from_utf8(ACCOUNT_PASSWORD).expect("ASCII");
    let mut child = Command::new(binary())
        .arg("--config")
        .arg(dir.join("prodomo.toml"))
        .args(["account", "create", login])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("prodomo should start");
    let mut input = child.stdin.take().expect("stdin is piped");
    drop(input.write_all(format!("{password}\n").as_bytes()));
    drop(input);
    let output = child.wait_with_output().expect("the command ends");
    std::fs::remove_dir_all(&dir).ok();
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
    /// The VIDs of the monsters and stones in this connection's view, which
    /// [`Keyed::monster_record`] keeps.
    monsters: Vec<[u8; 4]>,
    /// Records read ahead and put back, which the next read takes first.
    pushed: Vec<Vec<u8>>,
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
            monsters: Vec::new(),
            pushed: Vec::new(),
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

/// [`login_key`] in `language`. The login is retried while it answers `ALREADY`: a connection
/// the scenario saw close may not have released the login yet, as
/// [`login_by_key_when_free`] waits for on the Channel.
fn login_key_in(server: &Server, login: &[u8], language: u8) -> (Client, u32) {
    let mut auth = auth_client(server);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let key = loop {
        match log_in(&mut auth, login, ACCOUNT_PASSWORD, language.into()) {
            Err(status) if status == "ALREADY" && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            answer => break answer.expect("the auth login succeeds"),
        }
    };
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
    second.still("nothing follows");

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
    keyed.still("nothing follows");
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
        | GC_SCRIPT
        | GC_GUILD => usize::MAX,
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
/// `HEADER_GC_GUILD` (75): the envelope is header + WORD size + subheader
/// (`TPacketGCGuild`, `G/packet.h`), so it sizes from its own `WORD wSize`.
const GC_GUILD: u8 = 75;
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
        GC_ENTITY | GC_CHAT | GC_NPC_POSITION | GC_SHOP | GC_SCRIPT | GC_GUILD => {
            dynamic_len(header)
        }
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
        GC_QUICKSLOT_DEL | GC_SAFEBOX_SIZE | GC_MALL_OPEN | GC_PARTY_PARAMETER => 1 + 1,
        GC_QUICKSLOT_SWAP => 1 + 2,
        GC_EXCHANGE => EXCHANGE_LEN,
        GC_SAFEBOX_DEL | GC_MALL_DEL | GC_CHARACTER_DEL | GC_STUN | GC_DEAD | GC_PARTY_INVITE
        | GC_PARTY_REMOVE => 1 + 4,
        GC_SAFEBOX_WRONG_PASSWORD => 1,
        GC_WARP => WARP_LEN,
        GC_WALK_MODE => WALK_MODE_LEN,
        GC_TARGET => TARGET_LEN,
        GC_DAMAGE_INFO => DAMAGE_INFO_LEN,
        GC_CREATE_FLY => CREATE_FLY_LEN,
        GC_FLY_TARGETING => FLY_TARGETING_LEN,
        // The party walk: ADD (header + PID + name), LINK and UNLINK (header + two
        // VIDs), UPDATE (header + PID + role + percent + seven affect shorts). The
        // invite and REMOVE fold into the `1 + 4` arm above, the PARAMETER into `1 + 1`.
        GC_PARTY_ADD => 1 + 4 + 25,
        GC_PARTY_LINK | GC_PARTY_UNLINK => 1 + 4 + 4,
        GC_PARTY_UPDATE => 1 + 4 + 1 + 1 + 2 * 7,
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
/// `HEADER_GC_CREATE_FLY` is 70 (`G/packet.h:158`): the header, the fly type, the victim
/// and the attacker (`TPacketGCCreateFly`, `:1735-1743`).
const GC_CREATE_FLY: u8 = 70;
/// `TPacketGCCreateFly` is the header, a byte, and two DWORDs.
const CREATE_FLY_LEN: usize = 1 + 1 + 4 + 4;
/// `FLY_EXP`, the exp orb (`G/char.h:91`).
const FLY_EXP: u8 = 1;
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

/// [`enter_world_seeing`] for a master whose login walk carries `members` member
/// LOGINs before its own view pair.
fn enter_world_seeing_guilded(
    server: &Server,
    login: &[u8],
    slot: u8,
    members: usize,
) -> (Keyed, Listed, Entered) {
    let (mut keyed, character, quickslots, _items) =
        load_with_quickslots_on(server, 1, login, slot, ENGLISH);
    assert_eq!(quickslots, Vec::<Vec<u8>>::new(), "no quickslot is stored");
    let entered = enter_game_view_guilded(&mut keyed, members);
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
    keyed.still("nothing follows");
    entered
}

/// [`enter_game_view`] for a master whose login walk carries `members` member LOGINs
/// before its own view pair.
fn enter_game_view_guilded(keyed: &mut Keyed, members: usize) -> Entered {
    let entered = enter_game_guilded(keyed, members);
    keyed.still("nothing follows");
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
    finish_enter_game(keyed, add, info)
}

/// [`enter_game_shown`] for a master whose login walk carries `members` member LOGINs:
/// the walk (GRADE, INFO, LIST, one LOGIN per online member including itself, the
/// skills) lands through the outbox behind the burst's own pair; the pair is read
/// first, the walk's records in order around the view records `read_shown` keeps.
fn enter_game_guilded(keyed: &mut Keyed, members: usize) -> Entered {
    keyed.send_record(&client_enter_game());
    // The NAME leads (the Show's own insert names the guild, through the view's
    // `SendGuildName`), then the burst's own pair, and the link's walk lands
    // through the outbox among the burst's view records: GRADE, INFO, LIST, one
    // LOGIN per online member including itself, the skills. Every record that is
    // not one of the walk's is pushed back for the burst below.
    let named = keyed.read_game();
    assert_eq!(named[0], GC_GUILD, "the own NAME leads: {named:02x?}");
    assert_eq!(
        guild_subheader(&named),
        GUILD_SUBHEADER_GC_GUILD_NAME,
        "the own NAME leads: {named:02x?}"
    );
    let add = keyed.read_game();
    assert_eq!(add[0], GC_CHARACTER_ADD);
    let info = keyed.read_game();
    assert_eq!(info[0], GC_CHAR_ADDITIONAL_INFO);
    let mut want = vec![
        GUILD_SUBHEADER_GC_GRADE,
        GUILD_SUBHEADER_GC_INFO,
        GUILD_SUBHEADER_GC_LIST,
    ];
    want.extend(std::iter::repeat_n(GUILD_SUBHEADER_GC_LOGIN, members));
    want.push(GUILD_SUBHEADER_GC_SKILL_INFO);
    let mut kept: Vec<Vec<u8>> = Vec::new();
    let mut subs = want.iter();
    let mut sub = subs.next();
    while sub.is_some() {
        let record = keyed.read_game_raw();
        if record[0] != GC_GUILD {
            kept.push(record);
            continue;
        }
        let got = guild_subheader(&record);
        if got == GUILD_SUBHEADER_GC_GUILD_NAME {
            kept.push(record);
            continue;
        }
        assert_eq!(Some(&got), sub, "want sub {sub:?}: {record:02x?}");
        sub = subs.next();
    }
    // The enter walk's WAR_LIST (`SendGuildWar`): with no wars it is the bare envelope,
    // read here so the burst below sees only what it knows.
    keyed.unread(kept);
    finish_enter_game(keyed, add, info)
}

/// The enter-game burst after its own pair: the walk mode, the shown, the update,
/// the affect, the phase, time, Channel and notice.
fn finish_enter_game(keyed: &mut Keyed, add: Vec<u8>, info: Vec<u8>) -> Entered {
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
    enter_war_list(keyed);
    Entered {
        add,
        info,
        shown,
        affect,
    }
}

/// The `WAR_LIST` every enter ends with: the descriptor flushes the outbox's
/// `SendGuildWar` after the burst, so it lands behind the notice. No scenario has
/// started a war at enter time yet, so it is the bare envelope; answers it.
fn enter_war_list(keyed: &mut Keyed) -> Vec<u8> {
    let list = keyed.read_game();
    assert_eq!(list[0], GC_GUILD, "the enter WAR_LIST: {list:02x?}");
    assert_eq!(
        guild_subheader(&list),
        GUILD_SUBHEADER_GC_GUILD_WAR_LIST,
        "the enter WAR_LIST: {list:02x?}"
    );
    assert_eq!(list.len(), 4, "no wars yet: the bare envelope");
    list
}

/// What `Show` and `SendNPCPosition` send after the own-character pair: every character in view,
/// each as its `GC_CHARACTER_ADD` and, for a PC or an NPC, its `GC_CHAR_ADDITIONAL_INFO`, a
/// moving player's `GC_MOVE` and walk mode, the walk mode of a walking entrant, each item lying
/// in view as its `GC_ITEM_GROUND_ADD`, and then the map's NPC list when the map has one.
struct Shown {
    /// The insert and summary records, in the order they arrived, but a monster's or a stone's.
    records: Vec<Vec<u8>>,
    /// The inserts of the monsters, stones and veins, in the order they arrived
    /// ([`Keyed::monster_record`]).
    monsters: Vec<Vec<u8>>,
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

/// Whether the `GC_CHARACTER_ADD` `insert` is of a character the boot dice placed: a monster,
/// a stone, or an ore vein of `world::npc`'s `ORE_VEINS`, races 20047-20059 and 30301-30306.
fn is_boot_drawn(insert: &[u8]) -> bool {
    let race = u16::from_le_bytes([insert[22], insert[23]]);
    matches!(insert[21], CHAR_TYPE_MONSTER | CHAR_TYPE_STONE)
        || (20_047..=20_059).contains(&race)
        || (30_301..=30_306).contains(&race)
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
    /// first, which the viewer's own insert to it sends back (`G/char.cpp:1225-1236`), its
    /// guild's NAME when `guild` names one it has not heard (`ch->SendGuildName(GetGuild())`,
    /// `G/char.cpp:1070`), then its `GC_CHARACTER_ADD` and `GC_CHAR_ADDITIONAL_INFO`.
    /// Answers the NAME, when one was read, with the pair.
    fn sees_arrive(
        &mut self,
        vid: u32,
        walking: bool,
        guild: Option<&[u8]>,
    ) -> (Option<Vec<u8>>, Vec<u8>, Vec<u8>) {
        let id = vid.to_le_bytes();
        if walking {
            assert_eq!(self.read_game(), walk_mode_of(&id), "the newcomer walks");
        }
        let named = guild.map(|name| {
            let named = self.read_game();
            assert_eq!(named[0], GC_GUILD, "{named:02x?}");
            assert_eq!(guild_subheader(&named), GUILD_SUBHEADER_GC_GUILD_NAME);
            assert_eq!(&named[8..8 + name.len()], name, "the guild's name");
            named
        });
        let add = self.read_game();
        assert_eq!(add[0], GC_CHARACTER_ADD, "{add:02x?}");
        assert_eq!(&add[1..5], &id, "the insert's vid");
        let info = self.read_game();
        assert_eq!(info[0], GC_CHAR_ADDITIONAL_INFO, "{info:02x?}");
        assert_eq!(&info[1..5], &id, "the summary's vid");
        (named, add, info)
    }

    /// Read the view removal of the character `vid`: `[02, vid]`.
    fn sees_leave(&mut self, vid: u32) {
        let mut removal = vec![GC_CHARACTER_DEL];
        removal.extend_from_slice(&vid.to_le_bytes());
        assert_eq!(self.read_game(), removal, "the removal of {vid}");
    }

    /// Read what a viewer is sent when the player `leaver` disconnects: its own release,
    /// `GC_OWNERSHIP{0, leaver}` (`ClearSync`, `G/char.cpp:5568-5575`), then its removal.
    fn sees_disconnect(&mut self, leaver: u32) {
        assert_eq!(
            self.read_game(),
            ownership_of(0, leaver),
            "the release of {leaver}"
        );
        self.sees_leave(leaver);
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
    let mut monsters = Vec::new();
    let mut next = keyed.read_game_raw();
    while matches!(
        next[0],
        GC_CHARACTER_ADD | GC_CHAR_ADDITIONAL_INFO | GC_MOVE | GC_WALK_MODE | GROUND_ADD | GC_GUILD
    ) {
        if keyed.monster_record(&next) {
            if next[0] == GC_CHARACTER_ADD {
                monsters.push(next);
            }
        } else {
            // Noted, so the expiry of an insert that carries the revive flag is recognised.
            keyed.timer_record(&next);
            records.push(next);
        }
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
            monsters,
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
            GC_AFFECT_REMOVE => is_revive_end(record),
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
    /// records ([`Keyed::timer_record`]), the view records of the monsters, stones and veins
    /// a walk's last sample brought in or took out ([`Keyed::monster_record`]), and a stamina
    /// refill ([`is_stamina_refill`]).
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
                GC_MOVE | GC_WALK_MODE | GC_CHAT => {
                    chatter_width(&plain[at..]).unwrap_or(plain.len() - at)
                }
                GC_PING => 1,
                GC_AFFECT_REMOVE => AFFECT_REMOVE_LEN,
                GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
                GC_CHARACTER_ADD => CHARACTER_ADD_LEN,
                GC_CHAR_ADDITIONAL_INFO => CHAR_ADDITIONAL_INFO_LEN,
                GC_CHARACTER_DEL => 1 + 4,
                // Sized so that `is_stamina_refill` below can judge it; any other point
                // change fails there.
                GC_POINT_CHANGE => POINT_CHANGE_LEN,
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
                header == GC_PING
                    || self.monster_record(record)
                    || self.timer_record(record)
                    || is_stamina_refill(record)
                    || is_npc_chatter(record),
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
        self.closes("a record that closes the connection");
    }

    /// Assert that the descriptor closes the connection having sent nothing but what an NPC
    /// does on its own ([`is_npc_chatter`]): a nearby NPC may walk or speak before the close
    /// lands.
    fn closes(&mut self, note: &str) {
        let bytes = self.client.expect_closed();
        assert_eq!(
            bytes.len() % 8,
            0,
            "whole TEA units before the close ({note})"
        );
        let key = self.output;
        let plain: Vec<u8> = bytes
            .chunks(8)
            .flat_map(|unit| decrypt_padded(unit, &key).expect("aligned"))
            .collect();
        let mut at = 0;
        while at < plain.len() {
            let rest = &plain[at..];
            let width = chatter_width(rest)
                .filter(|&width| width <= rest.len() && is_npc_chatter(&rest[..width]));
            let Some(width) = width else {
                panic!("{note}: a record before the close: {rest:02x?}");
            };
            at += width.div_ceil(8) * 8;
        }
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
    /// [`Keyed::read_sized`], but it waits until `deadline` across 5 s read beats instead
    /// of panicking on one quiet beat, and reports `None` when the window runs out. A
    /// partial frame's bytes wait on the wire, so the caller's next call resumes the same
    /// record.
    fn read_sized_until(
        &mut self,
        len_of: &impl Fn(u8, &[u8]) -> usize,
        deadline: std::time::Instant,
    ) -> Option<Vec<u8>> {
        let key = self.output;
        let mut wire = self.client.expect_bytes_until(8, deadline)?;
        let first = decrypt_padded(&wire, &key).expect("aligned");
        let len = len_of(first[0], &first);
        wire.extend(
            self.client
                .expect_bytes_until(len.div_ceil(8) * 8 - 8, deadline)?,
        );
        let mut record = decrypt_padded(&wire, &key).expect("aligned");
        assert!(record[len..].iter().all(|&byte| byte == 0), "{record:02x?}");
        record.truncate(len);
        Some(record)
    }

    /// Read the next loading or enter-game record that is not the revive timer's own, a
    /// monster's nor a stamina refill: the first two are noted by [`Keyed::timer_record`] and
    /// [`Keyed::monster_record`], and all three are skipped ([`is_stamina_refill`]). A scenario
    /// that checks the timer or the refill reads with [`Keyed::read_game_raw`] instead.
    fn read_game(&mut self) -> Vec<u8> {
        loop {
            let record = self.read_game_raw();
            if !self.monster_record(&record)
                && !self.timer_record(&record)
                && !is_stamina_refill(&record)
            {
                return record;
            }
        }
    }

    /// Note `record` if it is a view record of a monster, a stone or an ore vein, and report
    /// whether it is.
    ///
    /// Map 1's regen files stand its monsters, stones and veins up at points the boot dice
    /// draws, so which of them a walk brings into view differs from run to run, and a scenario
    /// about players and NPCs reads past them. An insert of `bType` `CHAR_TYPE_MONSTER` or
    /// `CHAR_TYPE_STONE`, or of a vein's race, enters its VID; a vein is a `CHAR_TYPE_NPC`, so
    /// its summary follows and is noted too; and the removal of an entered VID forgets it.
    /// [`a_map_shows_its_monsters_and_stones_in_view`] is the scenario about them.
    fn monster_record(&mut self, record: &[u8]) -> bool {
        match record[0] {
            GC_CHARACTER_ADD if is_boot_drawn(record) => {
                let vid: [u8; 4] = record[1..5].try_into().expect("a VID");
                if !self.monsters.contains(&vid) {
                    self.monsters.push(vid);
                }
                true
            }
            GC_CHAR_ADDITIONAL_INFO => self.monsters.iter().any(|seen| record[1..5] == *seen),
            GC_CHARACTER_DEL => {
                let vid: [u8; 4] = record[1..5].try_into().expect("a VID");
                let entered = self.monsters.iter().position(|seen| *seen == vid);
                entered.map(|at| self.monsters.swap_remove(at)).is_some()
            }
            _ => false,
        }
    }

    /// Read the next loading or enter-game record that is not an NPC's own doing
    /// ([`is_npc_chatter`]), sizing a `WORD wSize` record from the first TEA unit and a
    /// fixed-width one from [`game_len`].
    fn read_game_raw(&mut self) -> Vec<u8> {
        loop {
            let record = self.read_wire();
            if !is_npc_chatter(&record) {
                return record;
            }
        }
    }

    /// Read the next record as it comes, an NPC's own included — or the earliest
    /// pushed-back record, when one waits.
    fn read_wire(&mut self) -> Vec<u8> {
        if !self.pushed.is_empty() {
            return self.pushed.remove(0);
        }
        self.read_sized(|header, first| match game_len(header) {
            usize::MAX => usize::from(first[1]) | (usize::from(first[2]) << 8),
            fixed => fixed,
        })
    }

    /// Put `records` back, in order, for the next reads to take first.
    fn unread(&mut self, records: Vec<Vec<u8>>) {
        let mut records = records;
        records.extend(std::mem::take(&mut self.pushed));
        self.pushed = records;
    }
    /// [`Keyed::read_game_raw`], but it waits until `deadline` across 5 s read beats
    /// instead of panicking on one quiet beat, and reports `None` when the window runs
    /// out.
    fn read_game_raw_until(&mut self, deadline: std::time::Instant) -> Option<Vec<u8>> {
        loop {
            let record = self.read_wire_until(deadline)?;
            if !is_npc_chatter(&record) {
                return Some(record);
            }
        }
    }

    /// [`Keyed::read_wire`], but it waits until `deadline` across 5 s read beats instead
    /// of panicking on one quiet beat, and reports `None` when the window runs out. A
    /// fight's critical or penetrate shows `GC_SPECIAL_EFFECT` (byte 114), which
    /// [`game_len`] does not size, so the width comes from [`fight_len`].
    fn read_wire_until(&mut self, deadline: std::time::Instant) -> Option<Vec<u8>> {
        self.read_sized_until(
            &|header, first| match fight_len(header) {
                usize::MAX => usize::from(first[1]) | (usize::from(first[2]) << 8),
                fixed => fixed,
            },
            deadline,
        )
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
        let (records, quiet) = self.drain_wire(window, width_of);
        // An NPC's own doing is not part of any scenario's answer.
        let records: Vec<Vec<u8>> = records
            .into_iter()
            .filter(|record| !is_npc_chatter(record))
            .collect();
        // Noted but not removed: a drain reports every record, the timer's included.
        for record in &records {
            self.timer_record(record);
        }
        (records, quiet)
    }

    /// Assert that nothing arrives within [`QUIET_WINDOW`] but what an NPC does on its own
    /// ([`is_npc_chatter`]) and the end of a revive timer this client saw start
    /// ([`Keyed::timer_end`]), and that the connection stays open.
    ///
    /// The timer runs on the clock, not on the scenario, so on a slow host a neighbour's five
    /// seconds can end inside the window: a CPU-throttled run saw it end in the check after a
    /// later login (ledger 229k).
    fn still(&mut self, note: &str) {
        self.still_for(note, QUIET_WINDOW);
    }

    /// [`Keyed::still`] over a `window` of its own.
    fn still_for(&mut self, note: &str, window: Duration) {
        let (records, quiet) = self.drain_wire(window, |header| match header {
            GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
            GC_AFFECT_REMOVE => AFFECT_REMOVE_LEN,
            _ => panic!("a record nobody accounted for ({note}): header {header}"),
        });
        assert_eq!(quiet, Quiet::Open, "the client is still connected ({note})");
        let others: Vec<Vec<u8>> = records
            .into_iter()
            .filter(|record| !is_npc_chatter(record) && !self.timer_end(record))
            .collect();
        assert!(others.is_empty(), "{note}: {others:02x?}");
    }

    /// [`Keyed::timer_record`] for a timer's end only: the revive's `GC_AFFECT_REMOVE`, or the
    /// look update that clears the flags of a VID whose timer this client saw start. A look
    /// update that starts a timer is not an end.
    fn timer_end(&mut self, record: &[u8]) -> bool {
        let starts = record[0] == GC_CHARACTER_UPDATE && record[23] & 0x08 != 0;
        !starts && self.timer_record(record)
    }

    /// [`Keyed::drain_game`] with every record as it came, an NPC's own included, and none
    /// noted.
    fn drain_wire(
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
            let width = chatter_width(&plain[at..]).unwrap_or_else(|| width_of(header));
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
    keyed.still("nothing follows");

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
    enter_war_list(&mut keyed);
    keyed.still("nothing follows");
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
    let mut server = server_without_wander(binary(), database.url());
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

/// The VID the Rewrite gives the first character it stands up: `world::npc::FIRST_NPC_VID`.
/// Channel 1 is stood up first, map 1 is its first map, and its `regen.txt` its first file, so
/// its monsters take the first VIDs and `npc.txt`'s characters the ones after them, as many as
/// the boot dice let stand.
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
///
/// An insert is matched to its entry by the Name its summary carries and its point, not by its
/// VID: `npc.txt`'s group 12007 is not listed and stands up as many members as its box's cells
/// let, so the VIDs after it are not the list's indices plus a fixed base.
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
    let mut matched = Vec::new();
    for _ in 0..in_view.len() {
        let insert = records.next().expect("an insert");
        assert_eq!(insert[0], GC_CHARACTER_ADD);
        let vid = u32::from_le_bytes(insert[1..5].try_into().expect("four"));
        assert_eq!(insert[21], 1, "bType CHAR_TYPE_NPC");
        let summary = records.next().expect("a summary");
        assert_eq!(summary[0], GC_CHAR_ADDITIONAL_INFO, "summary {vid:#x}");
        assert_eq!(&summary[1..5], &vid.to_le_bytes(), "summary {vid:#x}");
        let at = inserted_at(insert);
        let index = (0..entries.len())
            .find(|&index| {
                let listed = point_of(entries[index]);
                let slack = if BOXED_NPCS.contains(&index) { 100 } else { 0 };
                !matched.contains(&index)
                    && entries[index][1..26] == summary[5..30]
                    && (at.0 - listed.0).abs() <= slack
                    && (at.1 - listed.1).abs() <= slack
            })
            .unwrap_or_else(|| panic!("insert {vid:#x} at {at:?} is a listed NPC"));
        assert!(in_view.contains(&index), "insert {index} is in view");
        matched.push(index);
        walked.push((walk(at).expect("in the 3x3"), vid));
    }
    matched.sort_unstable();
    assert_eq!(matched, in_view, "each in view is shown once");
    assert!(
        walked.windows(2).all(|pair| pair[0] < pair[1]),
        "Build order, then VID order in each sectree: {walked:?}"
    );
    assert_eq!(records.next(), None, "nobody else is in view");
}

/// The `GC_CHARACTER_ADD` and `GC_CHAR_ADDITIONAL_INFO` `npc.txt`'s first NPC is shown with:
/// at its point, vnum 20300, both speeds 100, no state; then its en name, map 1's empire and
/// `PK_MODE_FREE`.
fn first_npcs_records(vid: u32) -> (Vec<u8>, Vec<u8>) {
    let mut insert = vec![GC_CHARACTER_ADD];
    insert.extend_from_slice(&vid.to_le_bytes());
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
    summary[1..5].copy_from_slice(&vid.to_le_bytes());
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

    // The list holds `npc.txt`'s 48 listed characters in the order they were stood up, each at
    // its point less map 1's base. The first is the first shown, and `regen.txt`'s monsters took
    // the VIDs before it.
    let list = shown.list.as_ref().expect("map 1 lists its NPCs");
    let entries: Vec<&[u8]> = list[5..].chunks(NPC_POSITION_LEN).collect();
    assert_eq!(entries.len(), 48);
    let warp = usize::try_from(WARP_INDEX).expect("small");
    assert_alphas_npcs_in_view(&shown, &entries, warp);
    let base = u32::from_le_bytes(shown.records[0][1..5].try_into().expect("a VID"));
    assert!(
        base > FIRST_NPC_VID,
        "regen.txt's monsters stood up before npc.txt"
    );

    let (insert, summary) = first_npcs_records(base);
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
    enter_war_list(&mut alice);
    alice.still("nothing follows");

    alice.send_record(&client_click(base));
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

/// Where the scenario's stray dog stands: an open cell of map 1, 1503 from [`ALPHA_ENTERS_AT`].
const DOG_AT: (i32, i32) = (469_600, 948_500);
/// The centre of the scenario's stone's box, half widths 1 and 1: open cells of map 1, about
/// 2000 from [`ALPHA_ENTERS_AT`].
const STONE_AT: (i32, i32) = (470_600, 952_000);

/// A `GC_CHARACTER_ADD` of a character with no summary at `at`, z 0 and rotation 0: its
/// `bType`, race, limited speeds and first affect word, no state flag.
fn unsummarised_insert(
    vid: u32,
    at: (i32, i32),
    (char_type, race): (u8, u16),
    speeds: [u8; 2],
) -> Vec<u8> {
    let mut insert = vec![GC_CHARACTER_ADD];
    insert.extend_from_slice(&vid.to_le_bytes());
    insert.extend_from_slice(&0f32.to_le_bytes());
    insert.extend_from_slice(&at.0.to_le_bytes());
    insert.extend_from_slice(&at.1.to_le_bytes());
    insert.extend_from_slice(&0i32.to_le_bytes());
    insert.push(char_type);
    insert.extend_from_slice(&race.to_le_bytes());
    insert.extend_from_slice(&speeds);
    insert.push(0);
    insert
}

/// `sys.world.regen`, `sys.world.view`, `gc.character_add`: map 1's regen files stand its
/// monsters and stones up at boot (`regen_spawn`, `G/regen.cpp:186-262`), and entering the map
/// shows each one in view as `EncodeInsertPacket` writes it (`G/char.cpp:1060-1110`), with no
/// `GC_CHAR_ADDITIONAL_INFO`, which only a PC or an NPC is sent (`:1110`).
///
/// The scenario's `npc.txt` adds a stray dog, vnum 101, at an open point beside Alpha, and a
/// stone, vnum 8001, in a box of half widths 1 and 1 there, each with direction 1, so rotation
/// 0 and no rotation draw. The dog is a `CHAR_TYPE_MONSTER` with its proto's speeds, 100 and
/// 100, and no affect: a point spawns without the spawn motion (`G/regen.cpp:346-353`). The
/// stone stands within 100 of its box's centre, facing a drawn angle, as a `CHAR_TYPE_STONE` with
/// its proto's 109 and
/// 104, and `AFF_SPAWN` (word 0 bit 2) set, because a box's stone always spawns with the motion
/// (`SpawnMobRange`, `G/char_manager.cpp:480-481`) and nothing resets the flag (D5). Neither is
/// on the mini-map, which still lists `npc.txt`'s 48. `npc.txt` stands its characters up in
/// order, so the stone's VID follows the dog's. Every monster and stone in view, the owner's
/// `regen.txt`'s included, is in `Build` order and then VID order, within `VIEW_RANGE + 500` in
/// the 3x3 sectrees around Alpha.
#[test]
fn a_map_shows_its_monsters_and_stones_in_view() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_with_npcs(
        binary(),
        database.url(),
        "metin2_map_a1",
        "m\t600\t525\t0\t0\t0\t1\t1s\t100\t1\t101\nm\t610\t560\t1\t1\t0\t1\t1s\t100\t1\t8001\n",
    );
    create_account(&server, "alice");
    add_characters(&database);
    let (mut alice, _alpha, _items) = load_character(&server, b"alice", 0);
    let entered = enter_game_shown(&mut alice);
    assert_eq!(inserted_at(&entered.add), ALPHA_ENTERS_AT);
    let shown = &entered.shown;

    let vid_of = |insert: &[u8]| u32::from_le_bytes(insert[1..5].try_into().expect("a VID"));
    let standing_at = |at: (i32, i32)| {
        shown
            .monsters
            .iter()
            .find(|insert| inserted_at(insert) == at)
            .unwrap_or_else(|| panic!("a monster at {at:?} is shown"))
    };
    let dog = standing_at(DOG_AT);
    let dog_vid = vid_of(dog);
    let mut expected = unsummarised_insert(dog_vid, DOG_AT, (CHAR_TYPE_MONSTER, 101), [100, 100]);
    expected.extend_from_slice(&[0; 8]);
    assert_eq!(dog, &expected, "the dog, with no affect");
    let stone = shown
        .monsters
        .iter()
        .find(|insert| {
            let at = inserted_at(insert);
            insert[22..24] == 8001u16.to_le_bytes()
                && (at.0 - STONE_AT.0).abs() <= 100
                && (at.1 - STONE_AT.1).abs() <= 100
        })
        .expect("the stone is shown within 100 of its box's centre");
    assert_eq!(vid_of(stone), dog_vid + 1, "npc.txt's next character");
    // A box's character is spawned with rotation -1, so `SpawnMob` draws `number(0, 360)`.
    let angle = f32::from_le_bytes(stone[5..9].try_into().expect("four"));
    assert!(
        angle.fract() == 0.0 && (0.0..=360.0).contains(&angle),
        "a drawn whole angle: {angle}"
    );
    let mut expected = unsummarised_insert(
        dog_vid + 1,
        inserted_at(stone),
        (CHAR_TYPE_STONE, 8001),
        [109, 104],
    );
    expected.extend_from_slice(&[4, 0, 0, 0, 0, 0, 0, 0]);
    expected[5..9].copy_from_slice(&stone[5..9]);
    assert_eq!(stone, &expected, "the stone, with AFF_SPAWN");
    assert!(
        alice.monsters.contains(&dog_vid.to_le_bytes()),
        "the connection holds the dog in view"
    );

    for insert in &shown.monsters {
        let vid = vid_of(insert);
        assert!(
            !shown
                .records
                .iter()
                .any(|record| record[0] == GC_CHAR_ADDITIONAL_INFO && vid_of(record) == vid),
            "no summary for monster {vid:#x}"
        );
        assert_eq!(insert[26], 0, "no state flag");
        assert!(
            matches!(insert[27..35], [0 | 4, 0, 0, 0, 0, 0, 0, 0]),
            "AFF_SPAWN at most: {insert:02x?}"
        );
    }
    let tree_of = |(x, y): (i32, i32)| (x / SECTREE_SIZE, y / SECTREE_SIZE);
    let (column, row) = tree_of(ALPHA_ENTERS_AT);
    let walked: Vec<(usize, u32)> = shown
        .monsters
        .iter()
        .map(|insert| {
            let at = inserted_at(insert);
            let gap = distance_approx(at.0 - ALPHA_ENTERS_AT.0, at.1 - ALPHA_ENTERS_AT.1);
            assert!(gap <= 5500, "{at:?} is within reach");
            let step = BUILD_ORDER
                .iter()
                .position(|&(dc, dr)| tree_of(at) == (column + dc, row + dr))
                .expect("in the 3x3");
            (step, vid_of(insert))
        })
        .collect();
    assert!(
        walked.windows(2).all(|pair| pair[0] < pair[1]),
        "Build order, then VID order in each sectree: {walked:?}"
    );
    let list = shown.list.as_ref().expect("map 1 lists its NPCs");
    assert_eq!(&list[3..5], &48u16.to_le_bytes(), "no monster is listed");
    alice.quiet("the burst is over");
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
/// Yankee watches from out of the warp's reach and is sent Alpha's removal alone: `WarpSet`
/// takes Alpha out of its sectree first (`:6749-6755`), so the release of Alpha's claim that
/// its later `Disconnect` sends (`ClearSync`) reaches nobody, while a disconnect in view sends
/// it to every viewer ([`Keyed::sees_disconnect`]).
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
    let mut server = server_without_wander(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    // 100 north of the warp: `DISTANCE_APPROX(0, 100)` is 96, within 300. Yankee is 800 north,
    // where it gives 768.
    sql(
        &database,
        "UPDATE player SET x = 450100, y = 903400 WHERE name = 'Alpha'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Yankee', 1, \
         450100, 904100 FROM account WHERE login = 'bob'",
    );
    let (mut bob, _yankee) = enter_world(&server, b"bob", 0);

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
    bob.sees_arrive(alpha.id, false, None);

    // Within one fire, the own character's removal and then the warp. The row was written first.
    let mut removed = vec![GC_CHARACTER_DEL];
    removed.extend_from_slice(&alpha.id.to_le_bytes());
    assert_eq!(alice.read_game(), removed, "EncodeRemovePacket(this)");
    let port = server.channel(1).port();
    assert_eq!(alice.read_game(), a_warp_record(A1_WARP_TARGET, port));
    bob.sees_leave(alpha.id);
    bob.quiet("no release of Alpha's claim");
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
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "no_wander = true",
    );
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
    enter_war_list(&mut alice);

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
    let mut server = server_without_wander(binary(), database.url());
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
    alice.closes("no GC_CHARACTER_DEL and no GC_WARP");
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
    let server = Server::start_with_npcs_configured(
        binary(),
        database.url(),
        "metin2_map_a1",
        "m\t604\t541\t0\t0\t0\t1\t1s\t100\t1\t10601\n",
        "no_wander = true",
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
    alice.sees_arrive(yankee.id, true, None);
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
    enter_war_list(&mut alice);
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
    let mut server = server_without_wander(binary(), database.url());
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
    enter_war_list(&mut alice);
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
    keyed.still("nothing follows");

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
         0, 90, 6, 4, 3, 3, 1, 9, 14, 700, 260, 820, 1100000, 500000 FROM account \
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
    keyed.still("nothing follows");

    // `EncodeInsertPacket` writes `GetLimitPoint` of both speeds (`G/char.cpp:1089-1091`).
    let add = enter_game_burst(&mut keyed);
    assert_eq!(add.len(), CHARACTER_ADD_LEN);
    // The header, `dwVID`, `angle`, `x`, `y`, `z`, `bType` and `wRaceNum` come first.
    assert_eq!(add[24], 50, "bMovingSpeed");
    assert_eq!(add[25], 100, "bAttackSpeed");
    drop(keyed);
    wait_for(
        &database,
        "(SELECT (hp, sp, stamina) = (380, 260, 820) FROM player WHERE name = 'Gamma')",
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
    alice.sees_arrive(yankee_listed.id, true, None);
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
    // nothing and goes to the interpreter instead. `do_restart` tells a living player only to
    // close the restart window, and the line is not relayed (`G/cmd_general.cpp:528-533`).
    alice.send_record(&client_chat(CHAT_TALKING, b"/restart_here"));
    assert_eq!(alice.read_game(), close_restart_window());
    yankee.quiet("a command line is not relayed");

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
    alice.sees_arrive(yankee_listed.id, true, None);

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
/// `sys.party`: a party line reaches every linked member, the speaker included, and no
/// outsider on the same map. Alice invites Yankee; Yankee answers; Alice speaks; both hear
/// the type-3 line, and Charlie (same map, no party) hears nothing
/// (`G/input_main.cpp:957-975`, `G/party.h:277-285`).
/// The party link's walk: read from `who` until the link's shape has arrived — `adds`
/// ADDs, `links` LINKs, an UPDATE, and, when `parameter` is set, a PARAMETER. The
/// periodic state UPDATE may sit anywhere, so the walk gates on shape, not on count.
fn drain_party_walk(who: &mut Keyed, note: &str, adds: usize, links: usize, parameter: bool) {
    let mut headers = Vec::new();
    let mut seen_adds = 0;
    let mut seen_links = 0;
    for _ in 0..24 {
        let header = who.read_game()[0];
        headers.push(header);
        seen_adds += usize::from(header == GC_PARTY_ADD);
        seen_links += usize::from(header == GC_PARTY_LINK);
        if seen_adds >= adds
            && seen_links >= links
            && headers.contains(&GC_PARTY_UPDATE)
            && (!parameter || headers.contains(&GC_PARTY_PARAMETER))
        {
            break;
        }
    }
    assert!(
        seen_adds >= adds
            && seen_links >= links
            && headers.contains(&GC_PARTY_UPDATE)
            && (!parameter || headers.contains(&GC_PARTY_PARAMETER)),
        "{note} walked: {headers:02x?}"
    );
}

/// Read until a `GC_CHAT` line, allowing the party walk's trailing records ahead of it.
/// [`drain_party_walk`] gates on the walk's shape, which a later send can follow; a
/// scenario waiting for a command's answer therefore skips those records rather than
/// asserting an exact walk length.
fn read_told_line(who: &mut Keyed, note: &str) -> Vec<u8> {
    for _ in 0..32 {
        let record = who.read_game();
        if record[0] == GC_CHAT {
            return record;
        }
        assert!(
            matches!(
                record[0],
                GC_PARTY_ADD
                    | GC_PARTY_LINK
                    | GC_PARTY_UPDATE
                    | GC_PARTY_PARAMETER
                    | GC_PARTY_UNLINK
                    | GC_PARTY_REMOVE
            ),
            "{note}: a non-party record ahead of the chat line: {record:02x?}"
        );
    }
    panic!("{note}: no chat line within 32 records");
}
/// Read from `who` until the party line arrives; an undrained party record may still
/// sit ahead of it.
fn read_party_line(who: &mut Keyed) -> Vec<u8> {
    for _ in 0..8 {
        let record = who.read_game();
        if record[0] == GC_CHAT {
            return record;
        }
        assert!(
            record[0] == GC_PARTY_UPDATE
                || record[0] == GC_PARTY_PARAMETER
                || record[0] == GC_PARTY_ADD
                || record[0] == GC_PARTY_LINK,
            "ahead of the line: {record:02x?}"
        );
    }
    panic!("no party line arrived");
}
#[test]
fn a_party_line_reaches_the_members_and_no_outsider() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    create_account(&server, "carol");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1, \
 154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Charlie', 2, \
 470100, 950100 FROM account WHERE login = 'carol'",
    );
    // Yankee keeps the column's 0 and walks; Charlie is seeded, so it does not.
    stamina(&database, "Charlie", 12_345);
    let (mut alice, alpha, alice_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, true, None);
    let (mut charlie, charlie_listed, _) = enter_world_seeing(&server, b"carol", 0);
    alice.sees_arrive(charlie_listed.id, false, None);
    yankee.sees_arrive(charlie_listed.id, false, None);
    let yankee_vid = u32::from_le_bytes(yankee_entered.add[1..5].try_into().unwrap());
    let alpha_vid = u32::from_le_bytes(alice_entered.add[1..5].try_into().unwrap());
    let _ = alpha;
    // Alice invites Yankee; Yankee answers yes. The invite asks the guest; the
    // answer's walk (ADDs, LINKs, UPDATEs, PARAMETER) is drained by header, in
    // legacy order.
    let mut invite = vec![CG_PARTY_INVITE];
    invite.extend_from_slice(&yankee_vid.to_le_bytes());
    alice.send_record(&invite);
    let asked = yankee.read_game();
    assert_eq!(
        asked[0], GC_PARTY_INVITE,
        "the invite asks the guest: {asked:02x?}"
    );
    assert_eq!(&asked[1..5], &alpha_vid.to_le_bytes(), "the inviter's VID");
    let mut answer = vec![CG_PARTY_INVITE_ANSWER];
    answer.extend_from_slice(&alpha_vid.to_le_bytes());
    answer.push(1);
    yankee.send_record(&answer);
    // The join's one-to-all ADD reaches nobody (`member.vid` attaches only in
    // `Link`), so the guest hears its link's own ADD + every ADD (2), and the
    // leader its link's own ADD + every ADD plus the guest's join ADD and the
    // guest link's one-to-all ADD.
    drain_party_walk(&mut yankee, "the guest", 3, 2, true);
    drain_party_walk(&mut alice, "the leader", 4, 2, true);
    // Alice speaks to the party. Both members hear the same type-3 bytes.
    alice.send_record(&client_chat(prodomo::chat::CHAT_PARTY, b"hello"));
    let own = read_party_line(&mut alice);
    assert_eq!(own[3], prodomo::chat::CHAT_PARTY, "type: {own:02x?}");
    assert_eq!(&own[4..8], &alpha_vid.to_le_bytes(), "id");
    assert_eq!(own[8], 1, "bEmpire is the speaker's empire");
    assert_eq!(own[9], 0, "bCanFormat is false");
    assert_eq!(&own[10..], b"Alpha : hello");
    assert_eq!(
        read_party_line(&mut yankee),
        own,
        "the member hears the same bytes"
    );
    charlie.quiet("the outsider hears nothing");
}

/// `cmd.party_request`, `cmd.party_request_accept` and `cmd.party_request_deny`
/// (`G/cmd_general.cpp:2428-2487` over `CHARACTER::RequestToParty`, `DenyToParty` and
/// `AcceptToParty`, `G/char.cpp:5656-5812`): the requester (Yankee) names a **member**
/// (Charlie), the world retargets to the leader (Alpha), Alpha hears
/// `PartyRequest <Yankee VID>` and Yankee hears `[LS;829;Alpha]`; the deny tells Yankee
/// `PartyRequestDenied`, the denied request cannot later be accepted, a fresh request is
/// accepted into the existing party, and a member's own request is refused with `[LS;549]`.
#[test]
fn a_party_request_retargets_to_the_leader_and_is_denied_then_accepted() {
    use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};

    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    for login in ["alice", "bob", "carol"] {
        create_account(&server, login);
    }
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', \
         1, 154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Charlie', \
         2, 154, 470100, 950100 FROM account WHERE login = 'carol'",
    );
    // Charlie is seeded, so it does not walk into the world; Yankee keeps the column's 0.
    stamina(&database, "Charlie", 12_345);
    let (mut alice, _alpha, alice_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    let alpha_vid = u32::from_le_bytes(alice_entered.add[1..5].try_into().unwrap());
    let yankee_vid = u32::from_le_bytes(yankee_entered.add[1..5].try_into().unwrap());
    alice.sees_arrive(yankee_listed.id, true, None);
    let (mut charlie, charlie_listed, charlie_entered) = enter_world_seeing(&server, b"carol", 0);
    let charlie_vid = u32::from_le_bytes(charlie_entered.add[1..5].try_into().unwrap());
    alice.sees_arrive(charlie_listed.id, false, None);
    yankee.sees_arrive(charlie_listed.id, false, None);

    // Alpha forms a party with Charlie, so an accepted request has somewhere to join.
    let mut invite = vec![CG_PARTY_INVITE];
    invite.extend_from_slice(&charlie_vid.to_le_bytes());
    alice.send_record(&invite);
    let asked = charlie.read_game();
    assert_eq!(asked[0], GC_PARTY_INVITE, "{asked:02x?}");
    let mut answer = vec![CG_PARTY_INVITE_ANSWER];
    answer.extend_from_slice(&alpha_vid.to_le_bytes());
    answer.push(1);
    charlie.send_record(&answer);
    drain_party_walk(&mut charlie, "the guest", 3, 2, true);
    drain_party_walk(&mut alice, "the leader", 4, 2, true);
    yankee.quiet("an outsider hears no party record");

    // When: Yankee asks to join Charlie. Then: the leader is asked, and the line names it.
    yankee.send_record(&slash(format!("/party_request {charlie_vid}").as_bytes()));
    assert_eq!(
        yankee.read_game(),
        chat_packet(CHAT_TYPE_INFO, 1, b"[LS;829;Alpha]"),
        "the requester hears the leader's name, not the named member's"
    );
    let command = format!("PartyRequest {yankee_vid}");
    assert_eq!(
        read_told_line(&mut alice, "the leader's request line"),
        chat_packet(CHAT_TYPE_COMMAND, 1, command.as_bytes()),
        "the leader is asked with the requester's VID"
    );

    // When: Alpha denies. Then: Yankee hears the command, and the request is consumed.
    alice.send_record(&slash(
        format!("/party_request_deny {yankee_vid}").as_bytes(),
    ));
    assert_eq!(
        yankee.read_game(),
        chat_packet(CHAT_TYPE_COMMAND, 1, b"PartyRequestDenied")
    );
    alice.quiet("the denier hears nothing");
    alice.send_record(&slash(
        format!("/party_request_accept {yankee_vid}").as_bytes(),
    ));
    yankee.quiet("a denied request cannot be accepted");

    // When: a fresh request is accepted. Then: both join lines and the join walk arrive.
    yankee.send_record(&slash(format!("/party_request {alpha_vid}").as_bytes()));
    assert_eq!(
        yankee.read_game(),
        chat_packet(CHAT_TYPE_INFO, 1, b"[LS;829;Alpha]")
    );
    assert_eq!(
        read_told_line(&mut alice, "the leader's second request line"),
        chat_packet(CHAT_TYPE_COMMAND, 1, command.as_bytes())
    );
    alice.send_record(&slash(
        format!("/party_request_accept {yankee_vid}").as_bytes(),
    ));
    assert_eq!(
        yankee.read_game(),
        chat_packet(CHAT_TYPE_INFO, 1, b"[LS;839;Alpha]")
    );
    drain_party_walk(&mut yankee, "the requested guest", 4, 2, true);
    assert_eq!(
        alice.read_game(),
        chat_packet(CHAT_TYPE_INFO, 1, b"[LS;839;Yankee]")
    );
    drain_party_walk(&mut alice, "the accepting leader", 2, 1, false);

    // When: the new member asks again. Then: legacy's own-party gate refuses quietly.
    yankee.send_record(&slash(format!("/party_request {alpha_vid}").as_bytes()));
    assert_eq!(
        read_told_line(&mut yankee, "the member's own-party gate"),
        chat_packet(CHAT_TYPE_INFO, 1, b"[LS;549]")
    );
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

/// The three worlds a notice scenario needs: Alpha (Channel 1, map 1) is a `HIGH_WIZARD`;
/// Yankee is on Channel 1's other map, map 3; Charlie is on the Shared Channel's map 72.
fn notice_cast(server: &Server, database: &ScratchDatabase) -> (Keyed, Keyed, Keyed) {
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
        "UPDATE account SET empire = 1 WHERE login = 'carol'",
    );
    sql(
        database,
        "INSERT INTO player (account_id, slot, name, job, x, y) SELECT id, 0, 'Charlie', 2, \
         10000, 1210000 FROM account WHERE login = 'carol'",
    );
    sql(
        database,
        "INSERT INTO gm_grant (account_id, name, authority) SELECT id, 'Alpha', 'HIGH_WIZARD' \
         FROM account WHERE login = 'alice'",
    );
    let (alice, _) = enter_world(server, b"alice", 0);
    let (yankee, _) = enter_world_in(server, b"bob", 0, GERMAN);
    let (charlie, _) = enter_world_on(server, 99, b"carol", 0, ENGLISH);
    (alice, yankee, charlie)
}

/// `cmd.notice`, `cmd.notice_map` and `cmd.big_notice` (`G/cmd_gm.cpp:1628-1651`):
/// `/notice` says `<GM Name> :<argument>` to every character on every Channel, the
/// argument keeping the space after the command word; `/notice_map` says the argument to
/// the caller's map only, and because legacy sends it from one Channel's process, a
/// client on another Channel does not hear it even on the same map index; `/big_notice`
/// says the argument to the caller alone, in the big font (`ENABLE_FULL_NOTICE` is not
/// defined in this tree).
#[test]
fn a_gm_notice_crosses_channels_and_a_map_notice_stays() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    let (mut alice, mut yankee, mut charlie) = notice_cast(&server, &database);

    alice.send_record(&client_chat(CHAT_TALKING, b"/notice hello"));
    assert_eq!(
        alice.read_game(),
        chat_packet(CHAT_TYPE_NOTICE, 1, b"Alpha : hello"),
        "the GM hears its own notice"
    );
    assert_eq!(
        yankee.read_game(),
        chat_packet(CHAT_TYPE_NOTICE, 2, b"Alpha : hello"),
        "the other Channel 1 map hears the notice, with its own empire"
    );
    assert_eq!(
        charlie.read_game(),
        chat_packet(CHAT_TYPE_NOTICE, 1, b"Alpha : hello"),
        "the Shared Channel hears the notice"
    );

    alice.send_record(&client_chat(CHAT_TALKING, b"/notice_map hi"));
    assert_eq!(
        alice.read_game(),
        chat_packet(CHAT_TYPE_NOTICE, 1, b" hi"),
        "the argument keeps the leading space the command parser leaves"
    );
    yankee.quiet("the other map of the same Channel hears no map notice");
    charlie.quiet("no other Channel hears the map notice");

    alice.send_record(&client_chat(CHAT_TALKING, b"/big_notice big"));
    assert_eq!(
        alice.read_game(),
        chat_packet(CHAT_TYPE_BIG_NOTICE, 1, b" big"),
        "without ENABLE_FULL_NOTICE only the caller hears the big notice"
    );
    yankee.quiet("the big notice does not leave the caller");
    charlie.quiet("not even to the Shared Channel");
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
        // The revive affect ends five seconds after the entry, which a slow run reaches here:
        // its removal and look update are the timer's ([`Keyed::timer_record`]), not answers.
        let (records, quiet) = alice.drain_wire(QUIET_WINDOW, |header| match header {
            GC_CHAT => answer.len(),
            GC_PING => 1,
            GC_AFFECT_REMOVE => AFFECT_REMOVE_LEN,
            GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
            other => panic!("unexpected record {other}"),
        });
        assert_eq!(quiet, Quiet::Open, "a dropped line never closes before ten");
        let mut lines = Vec::new();
        for record in records {
            if record[0] != GC_PING && !is_npc_chatter(&record) && !alice.timer_record(&record) {
                lines.push(record);
            }
        }
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
    let (_, _, yankee_info) = alice.sees_arrive(yankee_listed.id, false, None);
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
    let (_, shown_again, info_again) = yankee.sees_arrive(alpha.id, false, None);
    assert_eq!(&shown_again[17..21], &0i32.to_le_bytes(), "at its own z");
    assert_eq!(info_again, alpha_entered.info, "then its pair");
    yankee.quiet("no move follows: the refusal stopped Alpha");
    // The mover: its own removal.
    alice.sees_leave(alpha.id);
    let (_, own_again, _) = alice.sees_arrive(alpha.id, false, None);
    assert_eq!(own_again[21], 6, "CHAR_TYPE_PC");
    alice.sees_arrive(yankee_listed.id, false, None);
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
        let (_, insert, _) = alice.sees_arrive(npc, false, None);
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
    alice.sees_arrive(yankee_listed.id, false, None);
    alice.quiet("the viewer hears the refusal and not the shout");

    // The same write from a character above the limit: the viewer hears the refusal's removal
    // and insert before the shout, and the shouter hears its own re-encode first.
    let mut both = client_move(1, 0, 0, 470_000 + 200_000, 950_000, 0x5ef3);
    both.extend(client_chat(prodomo::chat::CHAT_SHOUT, b"hi"));
    alice.send_record(&both);
    let shout = chat_packet(prodomo::chat::CHAT_SHOUT, 1, b"|Len|l Alpha : hi");
    yankee.sees_leave(alpha.id);
    yankee.sees_arrive(alpha.id, false, None);
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
    alice.sees_arrive(yankee_listed.id, true, None);
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
    alice.sees_arrive(yankee_id.id, false, None);
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
/// `FUNC_WAIT` (`G/packet.h:542`): a non-player's walk as Goto tells it.
const FUNC_WAIT: u8 = 0;
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
    let (_, _, yankee_info) = alice.sees_arrive(yankee_listed.id, false, None);
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
    // `no_wander`: the boot-drawn monsters stay where the scenarios put the players
    // (ledger 229l).
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "view_range = 10000\nno_wander = true",
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

/// Assert that `insert` is idx18's `GC_CHARACTER_ADD`: an NPC of race 9009 at [`IDX18_AT`],
/// whose VID depends on how many monsters stood up before it, and answer its VID.
fn assert_idx18(insert: &[u8]) -> u32 {
    assert!(
        u32::from_le_bytes(insert[1..5].try_into().expect("a VID")) > FIRST_NPC_VID,
        "idx18's VID"
    );
    assert_eq!(inserted_at(insert), IDX18_AT, "at its point");
    assert_eq!(insert[21], 1, "CHAR_TYPE_NPC");
    assert_eq!(&insert[22..24], &9009u16.to_le_bytes(), "race 9009");
    u32::from_le_bytes(insert[1..5].try_into().expect("a VID"))
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
    let idx18 = assert_idx18(&seen[0]);
    assert_eq!(
        &seen[1][..5],
        &[&[GC_CHAR_ADDITIONAL_INFO][..], &idx18.to_le_bytes()].concat()
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
    let (_, add, info) = alice.sees_arrive(yankee_listed.id, false, None);
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
    let (_, add, info) = yankee.sees_arrive(alpha.id, false, None);
    assert_eq!(insert_body(&add), insert_body(&alpha_entered.add));
    assert_eq!(info, alpha_entered.info);
    let (_, add, info) = alice.sees_arrive(yankee_id, false, None);
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
    let add = alice.read_game();
    assert_eq!(add[0], GC_CHARACTER_ADD, "{add:02x?}");
    let idx18 = assert_idx18(&add);
    let info = alice.read_game();
    assert_eq!(info[0], GC_CHAR_ADDITIONAL_INFO, "{info:02x?}");
    assert_eq!(&info[1..5], &idx18.to_le_bytes(), "the summary's vid");
    alice.quiet("idx18 alone");

    wait_out_walk(sent, LONG_WALK_MS);
    alice.send_record(&client_move(FUNC_MOVE, 0, 0, row_144.0, row_144.1, 0x6101));
    alice.sees_leave(idx18);
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
    let (_, add, info) = alice.sees_arrive(yankee_id, false, None);
    let (x, y) = inserted_at(&add);
    assert_eq!(x, 444_100);
    assert!((dest.1..=940_000).contains(&y), "the live point: {y}");
    assert_eq!(info, yankee_entered.info);
    assert_insert_move(&alice.read_game(), yankee_id, dest, LONG_WALK_MS);
    assert_eq!(alice.read_game(), walk_mode_of(&yankee_id.to_le_bytes()));
    let (_, add, info) = yankee.sees_arrive(alpha.id, false, None);
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
    alice.sees_arrive(yankee_listed.id, false, None);
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
    alice.sees_arrive(yankee_listed.id, false, None);
    let (mut charlie, _, _) = enter_world_seeing(&server, b"carol", 0);
    alice.quiet("Charlie is out of view");

    drop(alice);
    server.wait_for("Character left the world");
    yankee.sees_disconnect(alpha.id);
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
    alice.sees_arrive(yankee_listed.id, true, None);
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
    alice.sees_arrive(yankee_listed.id, false, None);

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
    alice.still_for("a save sends no record", Duration::from_millis(200));
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
/// speed. The stored 1920 hit points are the worn maximum, so they are kept and no heal
/// follows (the relog heal legacy's `ApplyPoint` would give is a Defect). The body armour
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
        (5, 1920, "POINT_HP, as stored"),
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
    keyed.still("nothing follows");

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
        "(SELECT (part_main, hp, sp) = (11810, 1920, 600) FROM player WHERE name = 'Alpha')",
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
        // The revive affect's end reaches the mover five seconds after its entry, wherever a
        // slow run has got to by then; a scenario about the affect reads it itself.
        .filter(|record| !is_revive_end(record))
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
    // The heal ticks land in these windows too (ledger 229l): Alpha is below its maximum
    // 1420, so every 75 Pulses add 29 (idle under 3 s) or 86. They fold into `hp_now`.
    let mut hp_now = 500;
    let (mut seen, pair_base) = without_heal_ticks(
        &read_a_move(&mut alpha, vid),
        hp,
        recovery,
        &mut hp_now,
        1420,
    );
    // The potion joins the event that Alpha's entry started (`StartAffectEvent` returns when one
    // runs, `G/char_affect.cpp:238`), so the event's second is the entry's, not the drink's: its
    // first tick may already be in this window. A tick is the hit points and the owed recovery;
    // the helper returns the pool before it, and `paid` threads through the steps below.
    let mut paid = 0;
    if let Some(base) = pair_base {
        assert_eq!(seen.len(), 8, "the pair follows the drinks");
        let tick = seen.split_off(6);
        assert_eq!(
            tick,
            [Seen::Point(hp, base + 99), Seen::Point(recovery, 600 - 99)],
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
    // paid. The heal ticks between the steps fold into `hp_now`.
    while paid < 600 {
        let step = (600 - paid).min(99);
        paid += step;
        let (step_base, potion_hp, owed) =
            read_tick_events(&mut alpha, vid, hp, recovery, &mut hp_now, 1420);
        assert_eq!(
            (potion_hp, owed),
            (step_base + step, 600 - paid),
            "the step on top of the heals"
        );
    }
    // And: then it stops. The heal ticks do not.
    quiet_past_heals(
        &mut alpha,
        "the event has paid the 600 and ends",
        Duration::from_millis(1200),
        vid,
        &mut hp_now,
        1420,
    );

    // And: the logout save keeps what the event paid plus the heals, with at most the ticks
    // between the last read and the save on top.
    drop(alpha);
    wait_for(
        &database,
        &format!("(SELECT hp >= {hp_now} FROM player WHERE name = 'Alpha')"),
    );
    let stored = query_hp(&database, "Alpha");
    assert!(
        is_tick_sum(stored - hp_now, 1420),
        "the save on top of the last read: {stored} from {hp_now}"
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
    let (hp, recovery) = (
        u8::try_from(common::point_slot::POINT_HP).expect("a point byte"),
        u8::try_from(common::point_slot::POINT_HP_RECOVERY).expect("a point byte"),
    );

    // Given: Alpha, at 500 hit points, drinks a small red potion.
    let (mut alpha, _character, _items) = load_character(&server, b"alice", 0);
    let add = enter_game_burst(&mut alpha);
    let vid = u32::from_le_bytes(add[1..5].try_into().expect("four bytes"));
    Server::write_console(&console, "item give Alpha 27001 1");
    server.wait_for("id 100000000; the client has it");
    let _set = alpha.read_game();
    alpha.send_record(&client_item_use(0));
    // The drink's window holds the owed 300, the red effect and the cleared cell, with the
    // heal ticks folded into `hp_now` — and the first step already inside it if the drink
    // joined before the tick at 25 pulses, which `paid` threads through (ledger 229l).
    let mut hp_now = 500;
    let (kept, pair_base) = without_heal_ticks(
        &read_a_move(&mut alpha, vid),
        hp,
        recovery,
        &mut hp_now,
        1420,
    );
    let mut paid = 0;
    if let Some(base) = pair_base {
        assert_eq!(
            kept,
            [
                Seen::Point(recovery, 300),
                Seen::Effect(1),
                Seen::Set(0, 0),
                Seen::Point(hp, base + 99),
                Seen::Point(recovery, 201),
            ],
            "the drink and the first step, already in the window"
        );
        paid = 99;
    } else {
        assert_eq!(
            kept,
            [Seen::Point(recovery, 300), Seen::Effect(1), Seen::Set(0, 0),],
            "the drink, before the first step"
        );
    }

    // When: the event has paid all 300, in three steps of 99 and a last 3. The heal ticks
    // between the steps fold into `hp_now`; the owed recovery is exact (ledger 229l).
    while paid < 300 {
        let step = (300 - paid).min(99);
        paid += step;
        let (step_base, potion_hp, owed) =
            read_tick_events(&mut alpha, vid, hp, recovery, &mut hp_now, 1420);
        assert_eq!(
            (potion_hp, owed),
            (step_base + step, 300 - paid),
            "the step on top of the heals"
        );
    }
    // Then: a save while the client is still connected stores the event's 800 plus the heals
    // so far, with later ticks on top.
    wait_for(
        &database,
        &format!("(SELECT hp >= {hp_now} FROM player WHERE name = 'Alpha')"),
    );
    let stored = query_hp(&database, "Alpha");
    assert!(
        is_tick_sum(hp_now - 800, 1420) && is_tick_sum(stored - hp_now, 1420),
        "the save on top of the event's 800: {stored} from {hp_now}"
    );
    alpha.still_for("a save sends no record", Duration::from_millis(200));
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

/// Whether an HP move from `before` to `after` is a heal tick: `15 + MaxHP . {1|5}/100`
/// (`hp_recovery`), capped at the maximum (ledger 229l).
fn is_heal_step(before: i64, after: i64, max_hp: i64) -> bool {
    [1, 5].iter().any(|percent| {
        after == (before + 15 + max_hp * percent / 100).min(max_hp) && after > before
    })
}

/// Whether `diff` is a sum of heal ticks: the one- and five-percent shares of `max_hp`.
fn is_tick_sum(mut diff: i64, max_hp: i64) -> bool {
    if diff < 0 {
        return false;
    }
    let shares = [15 + max_hp / 100, 15 + max_hp * 5 / 100];
    while diff > 0 {
        if diff >= shares[1] && is_tick_sum(diff - shares[1], max_hp) {
            return true;
        }
        if diff < shares[0] {
            return false;
        }
        diff -= shares[0];
    }
    true
}

/// One potion step past the heal ticks: reads until the step's HP half and recovery half both
/// arrive, folding every heal tick's HP record into `hp_now` (ledger 229l). Returns the pool
/// before the step and the step's two values. The server sends in order, so every heal the
/// step's value builds on is read first.
fn read_tick_events(
    keyed: &mut Keyed,
    vid: u32,
    hp: u8,
    recovery: u8,
    hp_now: &mut i64,
    max_hp: i64,
) -> (i64, i64, i64) {
    let point_of = |record: &Vec<u8>| {
        assert_eq!(record[0], GC_POINT_CHANGE, "{record:02x?}");
        assert_eq!(&record[4..8], &vid.to_le_bytes(), "the drinker's VID");
        let value = i64::from_le_bytes(record[17..25].try_into().expect("eight bytes"));
        (record[8], value)
    };
    let (step_base, potion_hp) = loop {
        let (kind, value) = point_of(&keyed.read_game());
        if kind == hp {
            if is_heal_step(*hp_now, value, max_hp) {
                *hp_now = value;
                continue;
            }
            let step_base = *hp_now;
            *hp_now = value;
            break (step_base, value);
        }
        panic!("a recovery half with no HP half: {kind} {value}");
    };
    loop {
        let (kind, value) = point_of(&keyed.read_game());
        if kind == recovery {
            return (step_base, potion_hp, value);
        }
        if kind == hp && is_heal_step(*hp_now, value, max_hp) {
            *hp_now = value;
            continue;
        }
        panic!("past a potion step, a record nobody accounted for: {kind} {value}");
    }
}

/// Drop the heal ticks from a drained window: a lone `Point(hp, _)` is a tick and folds
/// into `hp_now`; an HP point the recovery point follows is a potion pair and is kept, with
/// a heal tick ahead of the pair folded first. A 700 ms window holds at most one pair
/// (ticks are a second apart), and it is returned with the pool before it, so the caller
/// can tell the pair's step from the heals it builds on.
fn without_heal_ticks(
    seen: &[Seen],
    hp: u8,
    recovery: u8,
    hp_now: &mut i64,
    max_hp: i64,
) -> (Vec<Seen>, Option<i64>) {
    let mut kept = Vec::with_capacity(seen.len());
    let mut pair_base = None;
    let mut at = 0;
    let point = |index: usize| match seen.get(index) {
        Some(Seen::Point(kind, value)) => Some((*kind, *value)),
        _ => None,
    };
    while at < seen.len() {
        match (point(at), point(at + 1), point(at + 2)) {
            // A heal tick, then the potion's pair: fold the tick, keep the pair on top of
            // it. The heal pulse runs before the next potion pulse, so its HP record
            // lands first; the pair's step builds on the healed pool.
            (Some((kind, tick)), Some((hp2, hp_value)), Some((rec, _)))
                if kind == hp && hp2 == hp && rec == recovery =>
            {
                assert!(
                    is_heal_step(*hp_now, tick, max_hp),
                    "the tick before the pair: {tick} from {hp_now}"
                );
                *hp_now = tick;
                assert!(pair_base.replace(*hp_now).is_none(), "one pair per window");
                *hp_now = hp_value;
                kept.push(seen[at + 1].clone());
                kept.push(seen[at + 2].clone());
                at += 3;
            }
            // The potion's pair: keep both.
            (Some((kind, hp_value)), Some((rec, _)), _) if kind == hp && rec == recovery => {
                assert!(pair_base.replace(*hp_now).is_none(), "one pair per window");
                *hp_now = hp_value;
                kept.push(seen[at].clone());
                kept.push(seen[at + 1].clone());
                at += 2;
            }
            // A lone HP point: a heal tick.
            (Some((kind, value)), _, _) if kind == hp => {
                assert!(
                    is_heal_step(*hp_now, value, max_hp),
                    "a lone HP point is a heal tick: {value} from {hp_now}"
                );
                *hp_now = value;
                at += 1;
            }
            _ => {
                kept.push(seen[at].clone());
                at += 1;
            }
        }
    }
    (kept, pair_base)
}

/// [`Keyed::quiet_for`] past the heal ticks: every HP point record in the window folds into
/// `hp_now`; anything else must be a ping, the revive timer's, a stamina refill or an NPC's
/// own (ledger 229l). Returns nothing; `hp_now` is the pool at the window's end.
fn quiet_past_heals(
    keyed: &mut Keyed,
    note: &str,
    window: Duration,
    vid: u32,
    hp_now: &mut i64,
    max_hp: i64,
) {
    let hp = u8::try_from(common::point_slot::POINT_HP).expect("a point byte");
    let (records, quiet) = keyed.drain_game(window, |header| match header {
        GC_PING => 1,
        GC_POINT_CHANGE => POINT_CHANGE_LEN,
        GC_AFFECT_REMOVE => AFFECT_REMOVE_LEN,
        GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
        other => panic!("{note}: header {other}"),
    });
    assert_eq!(quiet, Quiet::Open, "{note} closed the connection");
    for record in records {
        if record[0] == GC_PING || keyed.timer_record(&record) || is_stamina_refill(&record) {
            continue;
        }
        assert_eq!(record[0], GC_POINT_CHANGE, "{note}: {record:02x?}");
        assert_eq!(
            &record[4..8],
            &vid.to_le_bytes(),
            "{note}: the drinker's VID"
        );
        assert_eq!(record[8], hp, "{note}: only the HP pool moves");
        let value = i64::from_le_bytes(record[17..25].try_into().expect("eight bytes"));
        assert!(
            is_heal_step(*hp_now, value, max_hp),
            "{note}: a heal tick: {value} from {hp_now}"
        );
        *hp_now = value;
    }
}

/// The stored hit points of `name`.
fn query_hp(database: &ScratchDatabase, name: &str) -> i64 {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime should start")
        .block_on(async {
            use db::sqlx::Connection;
            let mut connection = db::sqlx::PgConnection::connect(database.url())
                .await
                .expect("the scenario database should connect");
            let row: (i32,) = db::sqlx::query_as("SELECT hp FROM player WHERE name = $1")
                .bind(name)
                .fetch_one(&mut connection)
                .await
                .expect("the row should be there");
            connection.close().await.expect("the close should succeed");
            i64::from(row.0)
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
    alpha.sees_arrive(zulu_listed.id, false, None);
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
/// `G/item.cpp:533-547`).
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
    alpha.sees_arrive(zulu_listed.id, false, None);

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
    alpha.sees_arrive(zulu_listed.id, false, None);
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

/// `cg.game.item_drop` (gold), `cg.game.item_pickup` (gold): Alpha drops 1500 of its 5000
/// gold, the pile falls under ground VID 1 and Zulu sees it fall; Zulu picks it up and both
/// are told. The pile is unnumbered and rowless, and the store follows each purse change.
#[test]
fn a_dropped_gold_pile_lies_until_someone_picks_it_up() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    seat_alpha_and_zulu(&server, &database, &[]);
    sql(
        &database,
        "UPDATE player SET gold = 5000 WHERE name = 'Alpha'",
    );
    let inventory = common::item_slots::EWindows::Inventory as u8;
    let pile = owners_protos()
        .rows()
        .iter()
        .find(|proto| proto.item_type == gamedata::item_kind::ITEM_ELK)
        .expect("the owner's table has an ITEM_ELK row")
        .vnum;

    // Given: Alpha holds 5000 gold, and Zulu stands beside it.
    let (mut alpha, alpha_listed) = enter_world(&server, b"alice", 0);
    let (mut zulu, zulu_listed) = enter_game_unread(&server, b"bob");
    alpha.sees_arrive(zulu_listed.id, false, None);

    // When: Alpha drops 1500 gold.
    alpha.send_record(
        &protocol::cg_item_drop2::CgItemDrop2::new(
            protocol::item_pos::ItemPos::new(inventory, 0),
            1500,
            0,
        )
        .encode(),
    );
    // Then: the purse is 3500, the pile falls under ground VID 1 with the gold notice, and
    // Zulu sees it fall. The pile has no id and no row.
    assert_eq!(alpha.read_game(), gold_change(alpha_listed.id, 0, 3_500));
    assert_eq!(alpha.read_game(), a_ground_add(1, pile));
    assert_eq!(alpha.read_game(), an_info_line(b"[LS;1057;2]"));
    assert_eq!(zulu.read_game(), a_ground_add(1, pile));
    check(
        &database,
        "(SELECT gold FROM player WHERE name = 'Alpha') = 3500 AND NOT EXISTS (SELECT 1 FROM          item)",
    );

    // When: Zulu picks the pile up.
    zulu.send_record(&protocol::cg_item_pickup::CgItemPickup::new(1).encode());
    // Then: Zulu's purse grows by 1500, then both see the pile go, and the store follows.
    assert_eq!(zulu.read_game(), gold_change(zulu_listed.id, 1_500, 1_500));
    assert_eq!(zulu.read_game(), a_ground_del(1));
    assert_eq!(alpha.read_game(), a_ground_del(1));
    check(
        &database,
        "(SELECT gold FROM player WHERE name = 'Zulu') = 1500 AND NOT EXISTS (SELECT 1 FROM          item)",
    );

    // And: a gold drop past the purse refuses silently and lays nothing.
    alpha.unanswered(
        &protocol::cg_item_drop2::CgItemDrop2::new(
            protocol::item_pos::ItemPos::new(inventory, 0),
            3_501,
            0,
        )
        .encode(),
    );
    zulu.quiet("nobody is told of a refused gold drop");
    check(
        &database,
        "(SELECT gold FROM player WHERE name = 'Alpha') = 3500",
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
        "UPDATE player SET level = 9, hp = 1920, sp = 70000 WHERE name = 'Alpha'",
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
    enter_war_list(keyed);
    keyed.still("nothing follows");
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
    alice.sees_arrive(yankee.id, true, None);
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

/// The `CloseRestartWindow` command line to a character of empire 1.
fn close_restart_window() -> Vec<u8> {
    chat_packet(prodomo::chat::CHAT_COMMAND, 1, b"CloseRestartWindow")
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
    alpha_keyed.sees_arrive(yankee_listed.id, false, None);
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
    let started_at = std::time::Instant::now();
    let (mut alpha, _character, _items) = load_character(&server, b"alice", 0);
    let add = enter_game_burst(&mut alpha);
    let vid = u32::from_le_bytes(add[1..5].try_into().expect("four bytes"));
    // The revive's five seconds run from the entry's end, which is when the server started
    // the event, not from the load's start: the clock starts here.
    let entered_at = std::time::Instant::now();
    Server::write_console(&console, "item give Alpha 27001 2");
    server.wait_for("id 100000000; the client has it");
    assert_eq!(set_fields(&alpha.read_game()), (inventory, 0, 27_001, 2));

    // When: one potion is drunk 1.2 s after the character load starts, inside the window.
    // Then: 300 hit points are owed, the red effect plays and the stack goes to one. The
    // potion joins the event the entry started, which its due pulse does not move
    // (`StartAffectEvent` returns when one runs, `G/char_affect.cpp:238`). The 1.2 s include
    // the load, so the drink lands just past the tick at 25 pulses on a fast machine and just
    // before it on a slow one: the first step may already be in this window (ledger 229l).
    std::thread::sleep(Duration::from_millis(1200).saturating_sub(started_at.elapsed()));
    assert!(
        entered_at.elapsed() < Duration::from_millis(4_500),
        "the drink is inside the window"
    );
    alpha.send_record(&client_item_use(0));
    // The heal ticks land in these windows too (ledger 229l) and fold into `hp_now`.
    let mut hp_now = 500;
    let (kept, pair_base) = without_heal_ticks(
        &read_a_move_within(&mut alpha, vid, Duration::from_millis(300)),
        hp,
        recovery,
        &mut hp_now,
        1420,
    );
    let mut paid = 0;
    if let Some(base) = pair_base {
        assert_eq!(
            kept,
            [
                Seen::Point(recovery, 300),
                Seen::Effect(1),
                Seen::Count(0, 1),
                Seen::Point(hp, base + 99),
                Seen::Point(recovery, 201),
            ],
            "the drink and the first step, already in the window"
        );
        paid = 99;
    } else {
        assert_eq!(
            kept,
            [
                Seen::Point(recovery, 300),
                Seen::Effect(1),
                Seen::Count(0, 1),
            ],
            "the drink, before the first step"
        );
    }

    // And: the rest of the owed 300 pays in steps of 99 with a last 3. Then the revive ends:
    // the recovery records come before the removal and the flag update
    // (`G/char_affect.cpp:175-211` before `:226`): the removal to Alpha, then its look with no
    // flag.
    while paid < 300 {
        let step = (300 - paid).min(99);
        paid += step;
        let (step_base, potion_hp, owed) =
            read_tick_events(&mut alpha, vid, hp, recovery, &mut hp_now, 1420);
        assert_eq!(
            (potion_hp, owed),
            (step_base + step, 300 - paid),
            "the step on top of the heals"
        );
    }
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

/// Whether `record` is the revive-invisible affect's `GC_AFFECT_REMOVE` (type 215, apply 0),
/// as [`Keyed::timer_record`] reads it.
fn is_revive_end(record: &[u8]) -> bool {
    record.first() == Some(&GC_AFFECT_REMOVE)
        && record.get(1..5) == Some(&AFFECT_REVIVE_INVISIBLE.to_le_bytes()[..])
        && record.get(5) == Some(&0)
}

/// The width of a record an NPC may send on its own, read from its first bytes: a `GC_MOVE`, a
/// `GC_WALK_MODE`, or a `GC_CHAT` by its `WORD` size. `None` for any other header, or a chat
/// whose size has not arrived.
fn chatter_width(record: &[u8]) -> Option<usize> {
    match *record.first()? {
        GC_MOVE => Some(GC_MOVE_LEN),
        GC_WALK_MODE => Some(WALK_MODE_LEN),
        GC_CHAT => word_sized(record.get(..3)?),
        _ => None,
    }
}

/// Whether `record` is something an NPC did on its own (ledger 229i, `sys.mob.ai`): its walk's
/// `GC_MOVE`, its `GC_WALK_MODE`, or its talk, each carrying a VID from `FIRST_NPC_VID` up.
///
/// The NPCs a player stands near run their state machines: each waits, wanders one time in
/// seven and speaks now and then, all on the world's dice, so a scenario about something else
/// reads past them. [`an_npc_near_a_player_wanders_and_tells_its_view`] is the scenario about
/// them, and it reads with [`Keyed::read_wire`].
fn is_npc_chatter(record: &[u8]) -> bool {
    let vid_at = |from: usize| {
        record
            .get(from..from + 4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four bytes")))
    };
    let vid = match record.first() {
        Some(&(GC_MOVE | GC_CHAT)) => vid_at(4),
        Some(&GC_WALK_MODE) => vid_at(1),
        _ => None,
    };
    vid.is_some_and(|vid| vid >= FIRST_NPC_VID)
}

/// Whether `record` is a `GC_CHARACTER_POINT_CHANGE` of `POINT_STAMINA`.
///
/// A character below its maximum stamina that has stood still for three seconds is sent its
/// refill (ledger 229d, `G/char_affect.cpp:219-221`). A walk spends stamina, so when a run is
/// slow the refill can land inside a scenario that is about something else. The scenarios
/// about the refill read it raw ([`read_past_pings`]).
fn is_stamina_refill(record: &[u8]) -> bool {
    record.first() == Some(&GC_POINT_CHANGE)
        && record.get(8).map(|&kind| usize::from(kind)) == Some(POINT_STAMINA)
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
    yankee.sees_arrive(alpha.id, false, None);
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
    alpha_keyed.sees_arrive(yankee_listed.id, false, None);
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
    alpha_keyed.sees_arrive(yankee_listed.id, false, None);
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
    yankee.sees_arrive(alpha.id, false, None);
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
    alpha_keyed.sees_arrive(yankee_listed.id, false, None);
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

/// `cg.game.target`, `gc.target`: `HEADER_CG_TARGET` and `HEADER_GC_TARGET`, a header and the
/// target's VID one way, and `TPacketGCTarget`'s 32 bytes the other.
const CG_TARGET: u8 = 0x3d;
const GC_TARGET: u8 = 0x3f;
const TARGET_LEN: usize = 32;
/// The party headers a client needs to form one: the CG invite and answer, and the GC
/// walk records (`G/party.cpp` sends).
const CG_PARTY_INVITE: u8 = 0x48;
const CG_PARTY_INVITE_ANSWER: u8 = 0x49;
const GC_PARTY_INVITE: u8 = 0x4d;
const GC_PARTY_ADD: u8 = 0x4e;
const GC_PARTY_UPDATE: u8 = 0x4f;
const GC_PARTY_PARAMETER: u8 = 0x53;
const GC_PARTY_LINK: u8 = 0x5b;
const GC_PARTY_UNLINK: u8 = 0x5c;
const GC_PARTY_REMOVE: u8 = 0x50;

/// `CG_TARGET` naming `vid`.
fn client_target(vid: u32) -> Vec<u8> {
    let mut record = vec![CG_TARGET];
    record.extend_from_slice(&vid.to_le_bytes());
    record
}

/// `GC_TARGET` of `vid`: its percentage, hit points and maximum, the alliance fields false and
/// 0, and its element.
fn target_of(vid: u32, percent: u8, hp: i32, max_hp: i32, element: u8) -> Vec<u8> {
    let mut record = vec![GC_TARGET];
    record.extend_from_slice(&vid.to_le_bytes());
    record.push(percent);
    record.extend_from_slice(&hp.to_le_bytes());
    record.extend_from_slice(&max_hp.to_le_bytes());
    record.extend_from_slice(&[0; 17]);
    record.push(element);
    record
}

/// Read until a record that is not one of `vid`'s heal ticks: a `GC_TARGET` of `vid`
/// whose HP is a heal share above `hp_now` folds into it (ledger 229l). Anything else is
/// returned. Yankee is stored below its maximum, so its recovery event updates its targeters.
fn read_past_heal_targets(keyed: &mut Keyed, vid: u32, hp_now: &mut i64, max_hp: i64) -> Vec<u8> {
    loop {
        let record = keyed.read_game();
        let is_yankees =
            record.first() == Some(&GC_TARGET) && record.get(1..5) == Some(&vid.to_le_bytes());
        if !is_yankees {
            return record;
        }
        let hp = i32::from_le_bytes(record[6..10].try_into().expect("four bytes"));
        assert!(
            is_heal_step(*hp_now, i64::from(hp), max_hp),
            "a heal target: {hp} from {hp_now}"
        );
        *hp_now = i64::from(hp);
    }
}

/// Ledger 229g: `CInputMain::Target` and `CHARACTER::SetTarget` (`G/input_main.cpp:2249-2269`,
/// `G/char.cpp:6458-6598`), `ClearTarget` (`:654`) and the `@fixme120` consume
/// (`G/input_login.cpp:1254-1257`).
///
/// A `CG_TARGET` or `CG_ITEM_USE` in the loading phase is consumed and the entry goes on. A
/// click on the stray dog (vnum 101, `MAX_HP` 126, `RACE_FLAG` `ANIMAL`, so no element) is
/// answered with its VID, 100 and 126 twice; the same click again is answered with nothing,
/// and VID 0 clears the target with zeros. A click on Yankee, a level 1 assassin stored with 325
/// of its 650 hit points, is answered with 50, 325 and 650, and Yankee is sent nothing; when Yankee leaves, Alpha is sent VID 0 before the removal.
#[test]
fn a_click_targets_a_monster_and_a_player_and_a_leave_clears_it() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_with_npcs(
        binary(),
        database.url(),
        "metin2_map_a1",
        "m\t600\t525\t0\t0\t0\t1\t1s\t100\t1\t101\n",
    );
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    seat_yankee_and_charlie(&database);
    sql(
        &database,
        "UPDATE player SET hp = 325 WHERE name = 'Yankee'",
    );

    // Given: Alpha's client still sends a use and a click in the loading phase.
    let (mut alpha_keyed, _alpha, _items) = load_character(&server, b"alice", 0);
    alpha_keyed.send_record(&client_item_use(0));
    alpha_keyed.send_record(&client_target(1));
    alpha_keyed.quiet("the loading phase consumes a use and a click");
    let entered = enter_game_shown(&mut alpha_keyed);
    let dog = entered
        .shown
        .monsters
        .iter()
        .find(|insert| inserted_at(insert) == DOG_AT)
        .expect("the dog is shown");
    let dog_vid = u32::from_le_bytes(dog[1..5].try_into().expect("a VID"));

    // When: Alpha clicks the dog, twice. Then: one answer, its hit points full.
    alpha_keyed.send_record(&client_target(dog_vid));
    assert_eq!(
        alpha_keyed.read_game(),
        target_of(dog_vid, 100, 126, 126, 0)
    );
    alpha_keyed.send_record(&client_target(dog_vid));
    alpha_keyed.quiet("the same target again sends nothing");
    alpha_keyed.send_record(&client_target(0));
    assert_eq!(alpha_keyed.read_game(), target_of(0, 0, 0, 0, 0));

    // When: Yankee enters and Alpha clicks it. Then: its hit points, and Yankee hears nothing.
    let (mut yankee, yankee_listed, _yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    alpha_keyed.sees_arrive(yankee_listed.id, false, None);
    alpha_keyed.send_record(&client_target(yankee_listed.id));
    assert_eq!(
        alpha_keyed.read_game(),
        target_of(yankee_listed.id, 50, 325, 650, 0)
    );
    yankee.quiet("the target is not told it was clicked");

    // When: Yankee leaves. Then: its own release, then Alpha's target is cleared, then the
    // removal (`ClearSync` before `ClearTarget`, `G/char.cpp:653-654`). Yankee's heal ticks
    // reach Alpha as target updates between these (ledger 229l) and are read past.
    drop(yankee);
    let mut yankee_hp = 325;
    assert_eq!(
        read_past_heal_targets(&mut alpha_keyed, yankee_listed.id, &mut yankee_hp, 650),
        ownership_of(0, yankee_listed.id)
    );
    assert_eq!(
        read_past_heal_targets(&mut alpha_keyed, yankee_listed.id, &mut yankee_hp, 650),
        target_of(0, 0, 0, 0, 0)
    );
    alpha_keyed.sees_leave(yankee_listed.id);
}

/// Ledger 229k: `do_restart`'s living arm (`G/cmd_general.cpp:526-533`). A living player that
/// sends `/restart_here`, `/restart_town`, `/restart_now` or `/restart` is told only to close the
/// restart window: no phase, no warp, no hit points and no gold. `/restart` is `restart_here`'s
/// prefix, and `/restart_n` matches no command.
#[test]
fn a_living_player_restarting_is_told_to_close_the_restart_window() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let mut server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    sql(
        &database,
        "UPDATE player SET gold = 400000 WHERE name = 'Alpha'",
    );
    let (mut alice, _alpha) = enter_world(&server, b"alice", 0);

    // When: each restart line, back to back (5 lines, under the flood limit of 10 a second).
    for line in [
        &b"/restart_here"[..],
        b"/restart_town",
        b"/restart_now",
        b"/restart",
    ] {
        alice.send_record(&slash(line));
        // Then: only the close, even for `/restart_now` with 400000 gold.
        assert_eq!(
            alice.read_game(),
            close_restart_window(),
            "{}",
            String::from_utf8_lossy(line)
        );
    }
    alice.send_record(&slash(b"/restart_n"));
    assert_eq!(
        alice.read_game(),
        info_to_alpha(prodomo::command::NO_SUCH_COMMAND.as_bytes())
    );
    alice.quiet("no gold change, no phase");

    drop(alice);
    server.wait_for("Character left the world");
    wait_for(
        &database,
        "(SELECT gold FROM player WHERE name = 'Alpha') = 400000",
    );
}

/// Ledger 229k: `CheckTarget` in the player's recovery event (`G/char.cpp:3172-3283`,
/// `:6663`). Every 75 Pulses a player's target 4800 units or more away is dropped with a
/// `GC_TARGET` of VID 0. Yankee stands 6000 east of Alpha (5765 by `DISTANCE_APPROX`, out of
/// view) and is dropped within one run; Charlie stands 4000 east and 600 south (4082), the
/// movable cell nearest 4000 east, and is kept.
#[test]
fn a_target_4800_away_is_dropped_by_the_next_recovery_run() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    create_account(&server, "carol");
    add_characters(&database);
    seat_yankee_and_charlie(&database);
    sql(
        &database,
        "UPDATE player SET x = 476000, y = 950000, hp = 650 WHERE name = 'Yankee'",
    );
    sql(
        &database,
        "UPDATE player SET x = 474000, y = 950600 WHERE name = 'Charlie'",
    );

    let (mut alice, _alpha, _) = enter_world_seeing(&server, b"alice", 0);
    let (_yankee, yankee_listed, yankee_entered) = enter_world_seeing(&server, b"bob", 0);
    assert_eq!(
        inserted_at(&yankee_entered.add),
        (476_000, 950_000),
        "Yankee stands where it was stored"
    );
    let (_charlie, charlie_listed, charlie_entered) = enter_world_seeing(&server, b"carol", 0);
    assert_eq!(
        inserted_at(&charlie_entered.add),
        (474_000, 950_600),
        "Charlie stands where it was stored"
    );
    alice.sees_arrive(charlie_listed.id, false, None);
    alice.quiet("Yankee is out of view");

    // When: Alpha clicks Yankee. Then: the target, and VID 0 at the next recovery run.
    alice.send_record(&client_target(yankee_listed.id));
    let clicked = std::time::Instant::now();
    // Yankee is stored at its maximum, so the run it waits for heals nothing (ledger 229l).
    assert_eq!(
        alice.read_game(),
        target_of(yankee_listed.id, 100, 650, 650, 0)
    );
    assert_eq!(alice.read_game(), target_of(0, 0, 0, 0, 0));
    assert!(
        clicked.elapsed() < Duration::from_secs(3) + QUIET_WINDOW,
        "one run, 75 Pulses: {:?}",
        clicked.elapsed()
    );

    // When: Alpha clicks Charlie. Then: still held after more than one run, so the same click
    // again is answered with nothing.
    alice.send_record(&client_target(charlie_listed.id));
    let held = alice.read_game();
    assert_eq!(held[0], GC_TARGET, "{held:02x?}");
    assert_eq!(
        &held[1..5],
        &charlie_listed.id.to_le_bytes(),
        "Charlie's VID"
    );
    alice.quiet_for("a target 4082 away is kept", Duration::from_secs(4));
    alice.unanswered(&client_target(charlie_listed.id));
}

impl Keyed {
    /// The `GC_CHARACTER_ADD` records that arrive within `window`, in order, with every record
    /// [`Keyed::quiet_for`] accepts noted as it notes them; anything else panics.
    fn inserts_for(&mut self, note: &str, window: Duration) -> Vec<Vec<u8>> {
        let deadline = std::time::Instant::now() + window;
        let mut inserts = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            let (records, quiet) = self.drain_game(left, |header| match header {
                GC_PING => 1,
                GC_AFFECT_REMOVE => AFFECT_REMOVE_LEN,
                GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
                GC_CHARACTER_ADD => CHARACTER_ADD_LEN,
                GC_CHAR_ADDITIONAL_INFO => CHAR_ADDITIONAL_INFO_LEN,
                GC_CHARACTER_DEL => 1 + 4,
                GC_POINT_CHANGE => POINT_CHANGE_LEN,
                other => panic!("an unexpected record ({note}): header {other}"),
            });
            assert_eq!(quiet, Quiet::Open, "{note} closed the connection");
            for record in records {
                let _monster = self.monster_record(&record);
                assert!(
                    record[0] != GC_POINT_CHANGE || is_stamina_refill(&record),
                    "only a stamina refill may change a point ({note}): {record:02x?}"
                );
                if record[0] == GC_CHARACTER_ADD {
                    inserts.push(record);
                }
            }
        }
        inserts
    }
}

/// Ledger 229h: `regen_event` (`G/regen.cpp:582-599`), which runs `regen_spawn(regen, false)`
/// `number(0, 16)` plus its time seconds after boot and every `time` seconds after.
///
/// The stray dog's entry (`1s`, one unit) first runs within 17 seconds of boot and then every
/// second. Its count is full, so each run spawns `max_count - count`, none: Alpha, standing by
/// the dog for 19 seconds, is sent no second insert at its point. The respawn after a `Destroy`
/// is the world test `an_ore_vein_ends_at_its_time_and_its_regen_stands_another`; nothing in the
/// binary dies yet, and an ore vein ends 7 to 15 minutes after it stands.
#[test]
fn a_full_regen_stands_nothing_more_each_time_it_runs() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_with_npcs(
        binary(),
        database.url(),
        "metin2_map_a1",
        "m\t600\t525\t0\t0\t0\t1\t1s\t100\t1\t101\n",
    );
    create_account(&server, "alice");
    add_characters(&database);
    let (mut alice, _alpha, _items) = load_character(&server, b"alice", 0);
    let entered = enter_game_shown(&mut alice);
    let dogs = |inserts: &[Vec<u8>]| {
        inserts
            .iter()
            .filter(|insert| inserted_at(insert) == DOG_AT)
            .count()
    };
    assert_eq!(dogs(&entered.shown.monsters), 1, "boot stood the dog");

    // When: the dog's regen runs, at least twice. Then: no second dog.
    let inserts = alice.inserts_for("the dog's regen runs", Duration::from_secs(19));
    assert_eq!(dogs(&inserts), 0, "a full regen stands nothing more");
}

/// Ledger 229i: the state machine of the NPCs a player stands near (`sys.mob.ai`), started when
/// the player's view takes them in (`G/entity_view.cpp:110-118`).
///
/// Thirty stray dogs stand by Alpha. A dog is no aggressive monster, so each waits 3 to 5
/// seconds in Idle and one time in seven turns to a random heading and runs 300 to 700 units
/// (`__StateIdle_Monster`, `G/char_state.cpp:626-736`), which Goto tells the view as a
/// `GC_MOVE` `FUNC_WAIT`: the destination, the heading over 5, and the duration of the way at
/// the fall-back motion speed of 300 (V2) and the dog's moving speed of 100
/// (`SendMovePacket`, `G/char.cpp:3624-3636`). A dog runs, so no walk mode is sent; a dog that
/// speaks says a talk with its own VID. Over 20 seconds the dogs wait about 150 times, so the
/// chance that none walks is below one in a billion.
#[test]
fn an_npc_near_a_player_wanders_and_tells_its_view() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (_server, mut alice, dogs) = thirty_dogs_by_alpha(&database, "");

    // When: Alpha stands by the dogs for 20 seconds. Then: the dogs walk, and say it.
    let mut walks: HashMap<u32, Vec<u8>> = HashMap::new();
    for record in alice.wire_for(Duration::from_secs(20)) {
        let vid = record_vid(&record);
        match record.first() {
            // Only the idle walk is this scenario's subject: a dog that chases another
            // dog sends a move of its own, and the load decides whether one does.
            Some(&GC_MOVE) if dogs.contains_key(&vid) && record[1] == FUNC_WAIT => {
                walks.entry(vid).or_insert(record);
            }
            Some(&GC_WALK_MODE) => {
                let walker = u32::from_le_bytes(record[1..5].try_into().expect("a VID"));
                assert!(!dogs.contains_key(&walker), "a dog runs: {record:02x?}");
            }
            Some(&GC_CHAT) if dogs.contains_key(&vid) => {
                assert_eq!(record[3], CHAT_TYPE_TALKING, "a dog talks");
            }
            _ => {}
        }
    }
    assert!(!walks.is_empty(), "a dog walks in 20 seconds");
    for (vid, walk) in &walks {
        let from = dogs[vid];
        let word = |at: usize| u32::from_le_bytes(walk[at..at + 4].try_into().expect("a word"));
        let to = (
            i32::from_le_bytes(walk[8..12].try_into().expect("x")),
            i32::from_le_bytes(walk[12..16].try_into().expect("y")),
        );
        assert_eq!(walk.len(), GC_MOVE_LEN);
        assert_eq!((walk[1], walk[2]), (FUNC_WAIT, 0), "{walk:02x?}");
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let distance = (f64::from(dx)).hypot(f64::from(dy));
        assert!(
            (299.0..=701.0).contains(&distance),
            "an idle walk runs 300 to 700: {distance}"
        );
        // The heading over 5: clockwise from north, as `GetDeltaByDegree` steps.
        let bearing = f64::from(dx)
            .atan2(f64::from(dy))
            .to_degrees()
            .rem_euclid(360.0);
        let heading = f64::from(walk[3]) * 5.0;
        // The way's ends are truncated to whole units, which turns it by less than a degree.
        assert!(
            (bearing - heading + 1.0).rem_euclid(360.0) < 6.0,
            "the heading {heading} is the bearing {bearing} truncated to 5"
        );
        // `(distance / 300) * 1000` in `float`, at a moving speed of 100 (100%).
        let (fx, fy) = (i32_to_f32(dx), i32_to_f32(dy));
        let millis = f32_to_i32((fx * fx + fy * fy).sqrt() / 300.0 * 1000.0);
        assert_eq!(i32::try_from(word(20)), Ok(millis), "the way's duration");
    }
}

/// The `DWORD` at byte 4 of a record, where a `GC_MOVE` and a `GC_CHAT` carry their VID; 0 for
/// a record shorter than that.
fn record_vid(record: &[u8]) -> u32 {
    record.get(4..8).map_or(0, |bytes| {
        u32::from_le_bytes(bytes.try_into().expect("four bytes"))
    })
}

/// A server with thirty stray dogs standing within 300 of [`DOG_AT`], by Alpha, under the
/// `[game]` lines `game`; Alpha entered, and each dog's VID with the point it was shown at.
fn thirty_dogs_by_alpha(
    database: &ScratchDatabase,
    game: &str,
) -> (Server, Keyed, HashMap<u32, (i32, i32)>) {
    let server = Server::start_with_npcs_configured(
        binary(),
        database.url(),
        "metin2_map_a1",
        "m\t600\t525\t3\t3\t0\t0\t1000s\t100\t30\t101\n",
        game,
    );
    create_account(&server, "alice");
    add_characters(database);
    let (mut alice, _alpha, _items) = load_character(&server, b"alice", 0);
    let entered = enter_game_shown(&mut alice);
    let dogs: HashMap<u32, (i32, i32)> = entered
        .shown
        .monsters
        .iter()
        .filter(|insert| {
            let (x, y) = inserted_at(insert);
            (x - DOG_AT.0).abs() <= 300 && (y - DOG_AT.1).abs() <= 300
        })
        .map(|insert| {
            let vid = u32::from_le_bytes(insert[1..5].try_into().expect("a VID"));
            (vid, inserted_at(insert))
        })
        .collect();
    assert!(dogs.len() >= 30, "the dogs stand by Alpha: {}", dogs.len());
    (server, alice, dogs)
}

/// Ledger 229i, the control of [`an_npc_near_a_player_wanders_and_tells_its_view`]: under
/// `no_wander` (`G/config.cpp:1281-1283`) no monster walks, so the thirty dogs by Alpha stay
/// where they stand for 15 seconds, about 110 waits, where without it some dog walks but for a
/// chance below one in ten million.
#[test]
fn no_wander_keeps_the_monsters_near_a_player_still() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (_server, mut alice, dogs) = thirty_dogs_by_alpha(&database, "no_wander = true");
    let walks: Vec<Vec<u8>> = alice
        .wire_for(Duration::from_secs(15))
        .into_iter()
        .filter(|record| record.first() == Some(&GC_MOVE) && dogs.contains_key(&record_vid(record)))
        .collect();
    assert_eq!(walks, Vec::<Vec<u8>>::new(), "no dog walks");
}

impl Keyed {
    /// Every record that arrives within `window`, an NPC's own included, as
    /// [`Keyed::inserts_for`] accepts them; anything else panics.
    fn wire_for(&mut self, window: Duration) -> Vec<Vec<u8>> {
        let deadline = std::time::Instant::now() + window;
        let mut wire = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            let (records, quiet) = self.drain_wire(left, |header| match header {
                GC_PING => 1,
                GC_AFFECT_REMOVE => AFFECT_REMOVE_LEN,
                GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
                GC_CHARACTER_ADD => CHARACTER_ADD_LEN,
                GC_CHAR_ADDITIONAL_INFO => CHAR_ADDITIONAL_INFO_LEN,
                GC_CHARACTER_DEL => 1 + 4,
                GC_POINT_CHANGE => POINT_CHANGE_LEN,
                other => panic!("an unexpected record: header {other}"),
            });
            assert_eq!(quiet, Quiet::Open, "the connection stays open");
            wire.extend(records);
        }
        wire
    }
}

/// `cg.game.attack`: `HEADER_CG_ATTACK`, `TPacketCGAttack`'s header, `bType`, `dwVID` and the
/// two magic-cube pieces (`G/packet.h:531-538`).
const CG_ATTACK: u8 = 0x02;
/// `HEADER_CG_FLY_TARGETING`, `HEADER_CG_USE_SKILL`, `HEADER_CG_ADD_FLY_TARGETING` and
/// `HEADER_CG_SHOOT` (`G/packet.h:56-59`).
const CG_FLY_TARGETING: u8 = 0x33;
const CG_USE_SKILL: u8 = 0x34;
const CG_ADD_FLY_TARGETING: u8 = 0x35;

/// `HEADER_GC_FLY_TARGETING` (71): a shot's targeting record, 17 bytes.
const GC_FLY_TARGETING: u8 = 71;
/// The packed width of `TPacketGCFlyTargeting`: a header, two VIDs, two `long`s.
const FLY_TARGETING_LEN: usize = 17;
const CG_SHOOT: u8 = 0x36;
/// `gc.damage_info`: `HEADER_GC_DAMAGE_INFO`, `TPacketGCDamageInfo`'s header, `dwVID`, `flag`
/// and `damage` (`G/packet.h:2048-2054`).
const GC_DAMAGE_INFO: u8 = 0x87;
const DAMAGE_INFO_LEN: usize = 1 + 4 + 1 + 4;
/// `gc.stun` and `gc.dead`: `HEADER_GC_STUN` and `HEADER_GC_DEAD`, each a header and a VID.
const GC_STUN: u8 = 0x0d;
const GC_DEAD: u8 = 0x0e;
/// `DAMAGE_NORMAL`, the flag of a plain melee hit (`G/char_battle.cpp:1984`).
const DAMAGE_NORMAL: u8 = 1;

/// The stray dog's `MAX_HP` (vnum 101).
const DOG_HP: i32 = 126;
/// The damage Alpha's bare fist deals the stray dog. `CalcMeleeDamage`'s only draw is the
/// unarmed `number(0, 0)`, so the hit is exact: Alpha's attack grade is 344 and the dog's
/// defence grade 10 (`G/char.cpp:2716-2767`, `:2850-2858`); the hit rating, 274/300 less the
/// evasion's 0.23 (`G/battle.cpp:271-297`, with the attacker's level, owner question 28), is
/// 0.6833, which turns the 36 above twice Alpha's level into 24, so the attack is 332, and the
/// defence leaves 322.
const PUNCH_ON_THE_DOG: i32 = 322;
/// A stray dog (vnum 101) at regen point (605, 540), where Alpha enters, which stands once and
/// would stand again only after 1000 seconds.
const DOG_AT_ALPHA: &str = "m\t605\t540\t0\t0\t0\t1\t1000s\t100\t1\t101\n";

/// The damage of Alpha's hit on Charlie (ledger 231). Alpha is a level-154 shaman with 17 in
/// strength and 19 in dexterity, Charlie a level-154 warrior with nothing in his four
/// attributes. `CalcMeleeDamage` (`G/battle.cpp:489-666`) has no weapon to draw from, so the
/// damage is exact: the attack rating (`G/battle.cpp:272-297`) reads the attacker's 64 and the
/// victim's 51 — the victim's level is the attacker's, the legacy's own line — which makes
/// 0.6935, and the 36 above twice Alpha's level 24, so the attack is 332; Charlie's defence
/// grade is 154 (`G/char.cpp:2850-2858`), and the difference is the damage.
const PVP_DAMAGE: i32 = 178;
/// Charlie's hit points after the hit (ledger 231): his `MAX_HP` of 650, the base of the
/// warrior row at level 154, less [`PVP_DAMAGE`].
const PVP_HP_AFTER: i64 = 472;

/// `CG_ATTACK` of type 0 on `vid`, the magic-cube pieces 0.
fn client_attack(vid: u32) -> Vec<u8> {
    client_attack_typed(0, vid)
}

/// `CG_ATTACK` of `attack_type` on `vid`.
fn client_attack_typed(attack_type: u8, vid: u32) -> Vec<u8> {
    let mut record = vec![CG_ATTACK, attack_type];
    record.extend_from_slice(&vid.to_le_bytes());
    record.extend_from_slice(&[0, 0]);
    record
}

/// `CG_FLY_TARGETING` or `CG_ADD_FLY_TARGETING` under `header`, on `vid` at `(x, y)`.
fn client_fly(header: u8, vid: u32, (x, y): (i32, i32)) -> Vec<u8> {
    let mut record = vec![header];
    record.extend_from_slice(&vid.to_le_bytes());
    record.extend_from_slice(&x.to_le_bytes());
    record.extend_from_slice(&y.to_le_bytes());
    record
}

/// `GC_DAMAGE_INFO` of `damage` with `flag` on `vid`.
fn damage_info(vid: u32, flag: u8, damage: i32) -> Vec<u8> {
    let mut record = vec![GC_DAMAGE_INFO];
    record.extend_from_slice(&vid.to_le_bytes());
    record.push(flag);
    record.extend_from_slice(&damage.to_le_bytes());
    record
}

/// `GC_STUN` of `vid`.
fn stun_of(vid: u32) -> Vec<u8> {
    let mut record = vec![GC_STUN];
    record.extend_from_slice(&vid.to_le_bytes());
    record
}

/// `GC_DEAD` of `vid`.
fn dead_of(vid: u32) -> Vec<u8> {
    let mut record = vec![GC_DEAD];
    record.extend_from_slice(&vid.to_le_bytes());
    record
}

/// `GC_OWNERSHIP` of `victim` to `owner`, 0 for none.
fn ownership_of(owner: u32, victim: u32) -> Vec<u8> {
    let mut record = vec![GC_OWNERSHIP];
    record.extend_from_slice(&owner.to_le_bytes());
    record.extend_from_slice(&victim.to_le_bytes());
    record
}

/// `GC_CHARACTER_DEL` of `vid`.
fn removal_of(vid: u32) -> Vec<u8> {
    let mut record = vec![GC_CHARACTER_DEL];
    record.extend_from_slice(&vid.to_le_bytes());
    record
}

/// `POINT_EXP`, the exp slot (`G/char.h` via `common::point_slot::POINT_EXP`).
const POINT_EXP: u8 = 3;

/// A `GC_CREATE_FLY` of `fly_type` from `from` to `to`.
fn fly_of(fly_type: u8, from: u32, to: u32) -> Vec<u8> {
    let mut record = vec![GC_CREATE_FLY, fly_type];
    record.extend_from_slice(&from.to_le_bytes());
    record.extend_from_slice(&to.to_le_bytes());
    assert_eq!(record.len(), CREATE_FLY_LEN);
    record
}
impl Keyed {
    /// The next record past a ping, the revive timer's own and a stamina refill, with what an
    /// NPC does on its own skipped as [`Keyed::read_game_raw`] skips it.
    ///
    /// [`Keyed::read_game`] takes a noted monster's `GC_CHARACTER_DEL` for a walk's noise
    /// ([`Keyed::monster_record`]), and here the monster's removal is the answer.
    fn read_fight(&mut self) -> Vec<u8> {
        loop {
            let record = self.read_game_raw();
            if record[0] != GC_PING && !self.timer_record(&record) && !is_stamina_refill(&record) {
                return record;
            }
        }
    }
    /// [`Keyed::read_fight`], but it waits until `deadline` across 5 s read beats
    /// instead of panicking on one quiet beat, and reports `None` when the window runs
    /// out.
    fn read_fight_until(&mut self, deadline: std::time::Instant) -> Option<Vec<u8>> {
        loop {
            let record = self.read_game_raw_until(deadline)?;
            if record[0] != GC_PING && !self.timer_record(&record) && !is_stamina_refill(&record) {
                return Some(record);
            }
        }
    }

    /// The next record of a player's hit (ledger 231). The look updates of the arrival and of
    /// the revive's affect say nothing about the swing and are passed over.
    fn read_hit(&mut self) -> Vec<u8> {
        loop {
            let record = self.read_fight();
            if record[0] == GC_CHARACTER_UPDATE {
                continue;
            }
            return record;
        }
    }

    /// The revive's look update of the player under `vid`, past a ping, a stamina refill
    /// and what an NPC does on its own. [`Keyed::read_own`] cannot read it: the restart's own
    /// removal forgot the VID, so the timer's check would note the update and absorb it as the
    /// revive timer's (ledger 229l).
    fn read_revive_update(&mut self, vid: u32) -> Vec<u8> {
        let own: [u8; 4] = vid.to_le_bytes();
        loop {
            let record = self.read_wire();
            if record[0] == GC_PING || is_stamina_refill(&record) || is_npc_chatter(&record) {
                continue;
            }
            if record[0] == GC_CHARACTER_UPDATE && record.len() >= 5 && record[1..5] == own {
                if !self.timers.contains(&own) {
                    self.timers.push(own);
                }
                return record;
            }
            if self.timer_record(&record) {
                continue;
            }
            let npc_own = matches!(
                record[0],
                GC_CHARACTER_ADD | GC_CHARACTER_DEL | GC_CHAR_ADDITIONAL_INFO | GC_TARGET
            ) && record.len() >= 5
                && record[1..5] != own;
            if npc_own {
                continue;
            }
            return record;
        }
    }

    /// The next record of the player under `vid` past a ping, the revive timer's, a stamina
    /// refill and what an NPC does on its own: a wandering monster's insert, its removal, its
    /// additional info or its recovery target lands anywhere, even mid-restart (ledger 229l).
    fn read_own(&mut self, vid: u32) -> Vec<u8> {
        let own = vid.to_le_bytes();
        loop {
            let record = self.read_fight();
            let npc_own = matches!(
                record[0],
                GC_CHARACTER_ADD | GC_CHARACTER_DEL | GC_CHAR_ADDITIONAL_INFO | GC_TARGET
            ) && record.len() >= 5
                && record[1..5] != own;
            if npc_own {
                continue;
            }
            return record;
        }
    }
}

/// The VID of the `GC_CHARACTER_ADD` among `inserts` shown at `at`.
fn vid_shown_at<'a>(inserts: impl IntoIterator<Item = &'a [u8]>, at: (i32, i32)) -> u32 {
    let insert = inserts
        .into_iter()
        .find(|insert| inserted_at(insert) == at)
        .unwrap_or_else(|| panic!("a character is shown at {at:?}"));
    u32::from_le_bytes(insert[1..5].try_into().expect("a VID"))
}

/// A server whose map 1 has the `npc.txt` `lines` and no monster wanders, Alpha entered, its
/// VID, what the burst showed it, and its revive ended by a swing where it stands, the client's
/// `FUNC_ATTACK` move before its `CG_ATTACK`.
fn alpha_swinging_by(database: &ScratchDatabase, lines: &str) -> (Server, Keyed, u32, Entered) {
    let server = Server::start_with_npcs_configured(
        binary(),
        database.url(),
        "metin2_map_a1",
        lines,
        "no_wander = true",
    );
    create_account(&server, "alice");
    add_characters(database);
    let (mut alpha, listed, _items) = load_character(&server, b"alice", 0);
    let entered = enter_game_shown(&mut alpha);
    let swing = client_move(
        FUNC_ATTACK,
        7,
        40,
        ALPHA_ENTERS_AT.0,
        ALPHA_ENTERS_AT.1,
        0x5eed,
    );
    alpha.send_record(&swing);
    let update = read_past_pings(&mut alpha);
    assert_eq!(update[0], GC_CHARACTER_UPDATE, "{update:02x?}");
    assert!(alpha.timer_record(&update), "the swing ends the revive");
    assert_eq!(
        read_past_pings(&mut alpha),
        [GC_AFFECT_REMOVE, 215, 0, 0, 0, 0]
    );
    alpha.quiet("the swing is not relayed to the swinger");
    (server, alpha, listed.id, entered)
}

/// Ledger 229l: one swing of a monster on a player, as its victim reads it.
struct Swing {
    /// The `DAMAGE_INFO` damage.
    damage: i32,
    /// The `DAMAGE_INFO` flag.
    flag: u8,
    /// The hit points after the swing's `POINT_CHANGE`, none at clamp 0.
    hp: Option<i64>,
    /// The owner of the swing's `GC_OWNERSHIP`, none past 250 (`Kept` sends nothing).
    owner: Option<u32>,
    /// Whether the swing stunned: the `POINT_CHANGE{35}`, `{36}` and `GC_STUN` triple.
    stunned: bool,
    /// The closing `GC_CHARACTER_MOVE{2}`: rotation, destination and server time.
    rot: u8,
    x: i32,
    y: i32,
    time: u32,
}

/// The server time of a `GC_CHARACTER_MOVE`: `dwCurTime` at bytes 16..20.
fn move_time(record: &[u8]) -> u32 {
    assert_eq!(record[0], GC_MOVE, "{record:02x?}");
    u32::from_le_bytes(record[16..20].try_into().expect("four bytes"))
}

/// Read one monster swing on `victim`: skip by rule until its `DAMAGE_INFO`, then take the
/// ordered tail: the hit points, the stun triple, the claim and the closing move (ledger 229l).
///
/// Skipped: pings, the revive timer's, stamina refills, the monster's own chat, heal ticks, the
/// monster's `GC_TARGET` (its NPC recovery), and any NPC's own doing. The monster's own move is
/// tested before the chatter filter, since [`is_npc_chatter`] would skip it.
fn read_swing(keyed: &mut Keyed, mob: u32, victim: u32, deadline: std::time::Instant) -> Swing {
    let (damage, flag) = read_swing_head(keyed, mob, victim, deadline);
    swing_tail(keyed, mob, victim, damage, flag)
}

/// Skip by rule until a swing's `DAMAGE_INFO` on `victim`: pings, the revive timer's, stamina
/// refills, the monster's own chat, heal ticks, the monster's `GC_TARGET` and any NPC's own.
fn read_swing_head(
    keyed: &mut Keyed,
    mob: u32,
    victim: u32,
    deadline: std::time::Instant,
) -> (i32, u8) {
    let hp_kind = u8::try_from(common::point_slot::POINT_HP).expect("a point byte");
    let mut first = true;
    let (damage, flag) = loop {
        assert!(
            std::time::Instant::now() < deadline,
            "a swing within the window"
        );
        let record = keyed.read_game_raw();
        if record[0] == GC_PING || keyed.timer_record(&record) || is_stamina_refill(&record) {
            continue;
        }
        if record[0] == GC_DAMAGE_INFO && record[1..5] == victim.to_le_bytes() {
            let flag = record[5];
            let damage = i32::from_le_bytes(record[6..10].try_into().expect("four bytes"));
            break (damage, flag);
        }
        // The victim's own heal ticks and the monster's recovery target, between swings.
        let is_heal = record.first() == Some(&GC_POINT_CHANGE)
            && record.get(4..8) == Some(&victim.to_le_bytes())
            && record.get(8) == Some(&hp_kind);
        let is_mob_target =
            record.first() == Some(&GC_TARGET) && record.get(1..5) == Some(&mob.to_le_bytes());
        // A chat carries no VID at a fixed point; the monster's ATTACK chat arrives as
        // chatter and is skipped with it.
        let is_mob_chat = record.first() == Some(&GC_CHAT);
        assert!(
            is_heal || is_mob_target || is_mob_chat || is_npc_chatter(&record),
            "past a swing, a record nobody accounted for: {record:02x?} (first = {first})"
        );
        first = false;
    };
    (damage, flag)
}

/// Take a swing's ordered tail after its `DAMAGE_INFO`: the hit points, the stun triple, the
/// claim and the closing move.
fn swing_tail(keyed: &mut Keyed, mob: u32, victim: u32, damage: i32, flag: u8) -> Swing {
    let hp_kind = u8::try_from(common::point_slot::POINT_HP).expect("a point byte");
    let mut swing = Swing {
        damage,
        flag,
        hp: None,
        owner: None,
        stunned: false,
        rot: 0,
        x: 0,
        y: 0,
        time: 0,
    };
    let mut saw_hp = false;
    let mut saw_35 = false;
    let mut saw_36 = false;
    loop {
        // Below the chatter filter: the closing move is the monster's own.
        let record = keyed.read_wire();
        if record[0] == GC_PING || keyed.timer_record(&record) || is_stamina_refill(&record) {
            continue;
        }
        if record[0] == GC_MOVE && record[4..8] == mob.to_le_bytes() {
            assert_eq!(
                (record[1], record[2]),
                (FUNC_ATTACK, 0),
                "the swing closes with MOVE{{2}}: {record:02x?}"
            );
            swing.rot = record[3];
            swing.x = i32::from_le_bytes(record[8..12].try_into().expect("four bytes"));
            swing.y = i32::from_le_bytes(record[12..16].try_into().expect("four bytes"));
            swing.time = move_time(&record);
            return swing;
        }
        if record[0] == GC_OWNERSHIP && record[5..9] == victim.to_le_bytes() {
            assert!(swing.owner.is_none(), "one claim per swing");
            swing.owner = Some(u32::from_le_bytes(
                record[1..5].try_into().expect("four bytes"),
            ));
            continue;
        }
        if record[0] == GC_STUN && record[1..5] == victim.to_le_bytes() {
            assert!(saw_35 && saw_36, "the stun triple is ordered");
            swing.stunned = true;
            continue;
        }
        if record.first() == Some(&GC_POINT_CHANGE) && record[4..8] == victim.to_le_bytes() {
            let value = i64::from_le_bytes(record[17..25].try_into().expect("eight bytes"));
            match record.get(8).copied() {
                Some(kind) if kind == hp_kind && !saw_hp => {
                    saw_hp = true;
                    swing.hp = Some(value);
                }
                Some(kind) if kind == hp_kind => {
                    // A heal tick inside the tail; the swing's own HP record came first.
                }
                Some(35) => {
                    assert!(!saw_35, "one stun pool record");
                    saw_35 = true;
                }
                Some(36) => {
                    assert!(saw_35 && !saw_36, "the stun triple is ordered");
                    saw_36 = true;
                }
                kind => panic!("past a swing, a point nobody accounted for: {kind:?}"),
            }
            continue;
        }
        if record.first() == Some(&GC_TARGET) && record[1..5] == mob.to_le_bytes() {
            continue;
        }
        assert!(
            is_npc_chatter(&record),
            "past a swing, a record nobody accounted for: {record:02x?}"
        );
    }
}

/// The heal tick within `due`: each swing stamps `last_move`, so the tick adds exactly 29
/// to `hp_now`. A third swing may land first and is consumed (ledger 229l).
fn read_heal_tick(keyed: &mut Keyed, mob: u32, victim: u32, hp_now: i64, due: std::time::Instant) {
    let mut hp_now = hp_now;
    loop {
        assert!(
            std::time::Instant::now() < due,
            "a heal tick within four seconds of the first swing"
        );
        let record = keyed.read_game_raw();
        if record[0] == GC_PING || keyed.timer_record(&record) || is_stamina_refill(&record) {
            continue;
        }
        if record[0] == GC_DAMAGE_INFO && record[1..5] == victim.to_le_bytes() {
            let flag = record[5];
            let damage = i32::from_le_bytes(record[6..10].try_into().expect("four bytes"));
            let swing = swing_tail(keyed, mob, victim, damage, flag);
            hp_now = swing.hp.unwrap_or(hp_now);
            continue;
        }
        if record.first() == Some(&GC_POINT_CHANGE) && record[4..8] == victim.to_le_bytes() {
            let value = i64::from_le_bytes(record[17..25].try_into().expect("eight bytes"));
            assert_eq!(
                value,
                hp_now + 29,
                "each swing stamps last_move, so the tick adds 29"
            );
            return;
        }
        assert!(
            record.first() == Some(&GC_TARGET) && record[1..5] == mob.to_le_bytes()
                || is_npc_chatter(&record),
            "past the tick, a record nobody accounted for: {record:02x?}"
        );
    }
}

/// Read swings until one stuns, returning them all (ledger 229l). A heal tick that lifts the
/// victim above the first swing's damage only adds a swing.
fn until_stunned(
    keyed: &mut Keyed,
    mob: u32,
    victim: u32,
    deadline: std::time::Instant,
) -> Vec<Swing> {
    let mut swings = Vec::new();
    loop {
        let swing = read_swing(keyed, mob, victim, deadline);
        let stunned = swing.stunned;
        swings.push(swing);
        if stunned {
            return swings;
        }
    }
}

/// The first `GC_CHARACTER_MOVE` of `mob` with `func` before `deadline` (ledger 229l).
/// Wandering moves of other funcs are read past; anything else fails.
///
/// A swing of `mob` on `victim` that lands first is read whole and returned through `early`
/// (ledger 229n): the chase's last step can end inside its reach, and the swing's
/// `DAMAGE_INFO` then arrives before any wait within the window.
fn npc_move_of(
    keyed: &mut Keyed,
    mob: u32,
    victim: u32,
    func: u8,
    deadline: std::time::Instant,
    early: &mut Option<Swing>,
) -> Vec<u8> {
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "a move of {mob} with func {func} within the window"
        );
        // Below the chatter filter: the wanted move is the monster's own.
        let record = keyed.read_wire();
        if record[0] == GC_PING || keyed.timer_record(&record) || is_stamina_refill(&record) {
            continue;
        }
        if record[0] == GC_MOVE && record[4..8] == mob.to_le_bytes() {
            if record[1] == func {
                return record;
            }
            continue;
        }
        if record[0] == GC_DAMAGE_INFO && record[1..5] == victim.to_le_bytes() {
            let flag = record[5];
            let damage = i32::from_le_bytes(record[6..10].try_into().expect("four bytes"));
            *early = Some(swing_tail(keyed, mob, victim, damage, flag));
            // A move that never came: the caller takes the swing instead.
            return Vec::new();
        }
        assert!(
            is_npc_chatter(&record),
            "past a move, a record nobody accounted for: {record:02x?}"
        );
    }
}

/// Ledger 229j: `CInputMain::Attack` and `battle_melee_attack` (`G/input_main.cpp:1966-2009`,
/// `G/battle.cpp:172-210`), `CHARACTER::Damage` and its `Stun`, `StunEvent`, `Dead` and
/// `dead_event` (`G/char_battle.cpp:402-548`, `:1238-1696`, `:2179-2433`), `SetSyncOwner`
/// (`G/char.cpp:5469-5555`).
///
/// Alpha punches the stray dog it targets, standing on it. Its one hit of 322 takes all 126 of
/// the dog's hit points: Alpha is shown the damage, the dog's hit points, the stun and its claim
/// on the dog, in that order. Three seconds later the stun ends in death: Alpha's target is
/// cleared, the dog has no owner, then (ledger 229n) the empty exp record (154 levels above the
/// dog, the 1% delta row leaves nothing of 15) and the exp orb, and the dog is dead. A second
/// later the dog goes: no owner, then its removal; and its regen waits 1000 seconds.
#[test]
fn a_punch_on_a_targeted_dog_stuns_it_and_it_dies_three_seconds_later_and_leaves() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (_server, mut alpha, alpha_vid, entered) = alpha_swinging_by(&database, DOG_AT_ALPHA);
    let dog = vid_shown_at(
        entered.shown.monsters.iter().map(Vec::as_slice),
        ALPHA_ENTERS_AT,
    );
    alpha.send_record(&client_target(dog));
    assert_eq!(alpha.read_game(), target_of(dog, 100, DOG_HP, DOG_HP, 0));

    // When: Alpha punches the dog. Then: the damage before the clamp, the hit points, the
    // stun and the claim.
    alpha.send_record(&client_attack(dog));
    assert_eq!(
        alpha.read_fight(),
        damage_info(dog, DAMAGE_NORMAL, PUNCH_ON_THE_DOG)
    );
    assert_eq!(alpha.read_fight(), target_of(dog, 0, 0, DOG_HP, 0));
    assert_eq!(alpha.read_fight(), stun_of(dog));
    assert_eq!(alpha.read_fight(), ownership_of(alpha_vid, dog));
    let hit = std::time::Instant::now();
    alpha.quiet("the hit sends nothing more");

    // Then: three seconds on, the stun ends in death.
    assert_eq!(alpha.read_fight(), target_of(0, 0, 0, 0, 0));
    assert!(
        hit.elapsed() >= Duration::from_millis(2_500),
        "the stun lasts 75 Pulses: {:?}",
        hit.elapsed()
    );
    assert_eq!(alpha.read_fight(), ownership_of(0, dog));
    // Ledger 229n: the empty exp record, then the exp orb from the dog to Alpha.
    assert_eq!(alpha.read_fight(), point_change(alpha_vid, POINT_EXP, 0, 0));
    assert_eq!(alpha.read_fight(), fly_of(FLY_EXP, dog, alpha_vid));
    assert_eq!(alpha.read_fight(), dead_of(dog));
    let died = std::time::Instant::now();

    // Then: a second on, the dog goes.
    assert_eq!(alpha.read_fight(), ownership_of(0, dog));
    assert!(
        died.elapsed() >= Duration::from_millis(750),
        "the dead dog lies 25 Pulses: {:?}",
        died.elapsed()
    );
    assert_eq!(alpha.read_fight(), removal_of(dog));
    alpha.quiet_for(
        "the regen of 1000 seconds stands nothing",
        Duration::from_secs(2),
    );
}

/// Ledger 229j: a hit on a stunned victim runs `Dead` at once and deals nothing
/// (`G/char_battle.cpp:2248-2252`): no damage is shown, the dog dies (ledger 229n: with the
/// empty exp record and the exp orb, as in its twin) and goes a second later, and the stun's
/// own death never comes (`:1464`).
#[test]
fn a_second_punch_on_a_stunned_dog_kills_it_at_once_with_no_number() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (_server, mut alpha, alpha_vid, entered) = alpha_swinging_by(&database, DOG_AT_ALPHA);
    let dog = vid_shown_at(
        entered.shown.monsters.iter().map(Vec::as_slice),
        ALPHA_ENTERS_AT,
    );
    alpha.send_record(&client_target(dog));
    assert_eq!(alpha.read_game(), target_of(dog, 100, DOG_HP, DOG_HP, 0));
    alpha.send_record(&client_attack(dog));
    assert_eq!(
        alpha.read_fight(),
        damage_info(dog, DAMAGE_NORMAL, PUNCH_ON_THE_DOG)
    );
    assert_eq!(alpha.read_fight(), target_of(dog, 0, 0, DOG_HP, 0));
    assert_eq!(alpha.read_fight(), stun_of(dog));
    assert_eq!(alpha.read_fight(), ownership_of(alpha_vid, dog));

    // When: Alpha punches the stunned dog. Then: it dies at once, with no damage shown.
    alpha.send_record(&client_attack(dog));
    let hit = std::time::Instant::now();
    assert_eq!(alpha.read_fight(), target_of(0, 0, 0, 0, 0));
    assert_eq!(alpha.read_fight(), ownership_of(0, dog));
    // Ledger 229n: the empty exp record, then the exp orb from the dog to Alpha.
    assert_eq!(alpha.read_fight(), point_change(alpha_vid, POINT_EXP, 0, 0));
    assert_eq!(alpha.read_fight(), fly_of(FLY_EXP, dog, alpha_vid));
    assert_eq!(alpha.read_fight(), dead_of(dog));
    assert!(
        hit.elapsed() < Duration::from_millis(1_500),
        "no stun to wait out: {:?}",
        hit.elapsed()
    );
    assert_eq!(alpha.read_fight(), ownership_of(0, dog));
    assert_eq!(alpha.read_fight(), removal_of(dog));
    assert!(
        hit.elapsed() < Duration::from_millis(2_500),
        "the dog goes a second after its death: {:?}",
        hit.elapsed()
    );
    alpha.quiet_for(
        "no second death when the stun would have ended",
        Duration::from_secs(3),
    );
}

/// Ledger 229j: the reach and the receivers. A dog 300 east is 288 away by `DISTANCE_APPROX`,
/// within the 350 a bare fist reaches and beyond the 250 of a claim
/// (`G/battle.cpp:172-210`, `G/char.cpp:5508-5519`); one 500 east, 480 away, is out of reach.
///
/// A punch on the far dog does nothing. A punch on the near dog, which Alpha never targeted,
/// shows Alpha neither the damage (`G/char_battle.cpp:1720-1742`) nor its hit points nor a
/// claim; only the stun, then the release, (ledger 229n) the empty exp record and the exp orb,
/// and the death, then the release and the removal. The far dog is untouched.
#[test]
fn an_untargeted_punch_shows_no_number_and_one_beyond_reach_does_nothing() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let lines = "m\t608\t540\t0\t0\t0\t1\t1000s\t100\t1\t101\n\
                 m\t610\t540\t0\t0\t0\t1\t1000s\t100\t1\t101\n";
    let (_server, mut alpha, alpha_vid, entered) = alpha_swinging_by(&database, lines);
    let monsters = || entered.shown.monsters.iter().map(Vec::as_slice);
    let near = vid_shown_at(monsters(), (470_400, 950_000));
    let far = vid_shown_at(monsters(), (470_600, 950_000));

    alpha.send_record(&client_attack(far));
    alpha.quiet("a dog beyond the reach is not hit");

    alpha.send_record(&client_attack(near));
    assert_eq!(alpha.read_fight(), stun_of(near));
    let hit = std::time::Instant::now();
    alpha.quiet("no damage, no hit points and no claim for an untargeted dog 288 away");
    assert_eq!(alpha.read_fight(), ownership_of(0, near));
    assert!(hit.elapsed() >= Duration::from_millis(2_500));
    // Ledger 229n: the empty exp record, then the exp orb from the dog to Alpha.
    assert_eq!(alpha.read_fight(), point_change(alpha_vid, POINT_EXP, 0, 0));
    assert_eq!(alpha.read_fight(), fly_of(FLY_EXP, near, alpha_vid));
    assert_eq!(alpha.read_fight(), dead_of(near));
    assert_eq!(alpha.read_fight(), ownership_of(0, near));
    assert_eq!(alpha.read_fight(), removal_of(near));

    alpha.send_record(&client_target(far));
    assert_eq!(alpha.read_game(), target_of(far, 100, DOG_HP, DOG_HP, 0));
}

/// Ledger 231: a player's hit on a player (`G/battle.cpp:154-230`, `G/char_battle.cpp:1939-2267`).
///
/// Alpha of empire 1 hits Charlie of empire 0, who stands on him in the view of both. The
/// swing is [`PVP_DAMAGE`]: the attack rating makes the attack 332 and Charlie's defence
/// grade takes 154 of it, and Charlie's hit points fall from 650 to [`PVP_HP_AFTER`]. Alpha is
/// shown the damage and Charlie's claim; Charlie is shown the damage, his hit points and the
/// claim.
#[test]
fn a_players_hit_on_a_player_of_another_empire_takes_the_victims_hit_points() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "carol");
    add_characters(&database);
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Charlie', \
         1, 154, 470100, 950100 FROM account WHERE login = 'carol'",
    );
    // The stamina the other characters of the fixture carry: above zero, so Charlie enters
    // standing and the view sends no walk mode for him.
    sql(
        &database,
        "UPDATE player SET stamina = 12345 WHERE name = 'Charlie'",
    );
    let (mut alpha, alpha_listed, _alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut charlie, charlie_listed, _charlie_entered) = enter_world_seeing(&server, b"carol", 0);
    alpha.sees_arrive(charlie_listed.id, false, None);

    // When: Alpha hits Charlie. Then: the damage and the claim to Alpha, the damage, the hit
    // points and the claim to Charlie.
    alpha.send_record(&client_attack(charlie_listed.id));
    assert_eq!(
        alpha.read_hit(),
        damage_info(charlie_listed.id, DAMAGE_NORMAL, PVP_DAMAGE)
    );
    assert_eq!(
        alpha.read_hit(),
        ownership_of(alpha_listed.id, charlie_listed.id)
    );
    assert_eq!(
        charlie.read_hit(),
        damage_info(charlie_listed.id, DAMAGE_NORMAL, PVP_DAMAGE)
    );
    assert_eq!(
        charlie.read_hit(),
        point_change(
            charlie_listed.id,
            u8::try_from(common::point_slot::POINT_HP).expect("a point byte"),
            0,
            PVP_HP_AFTER
        )
    );
    assert_eq!(
        charlie.read_hit(),
        ownership_of(alpha_listed.id, charlie_listed.id)
    );
}

/// Ledger 231: the PK policy (`G/pvp.cpp:378-529`) refuses a hit on a player of the attacker's
/// own empire: the Rewrite has no PK mode, no consent duel and no alignment, so two players of
/// one empire never fight. Dave of empire 1 hits Alpha of empire 1: neither is shown a damage,
/// a hit point change or a claim.
#[test]
fn a_players_hit_on_a_player_of_his_own_empire_is_refused() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "dave");
    add_characters(&database);
    sql(
        &database,
        "UPDATE account SET empire = 1 WHERE login = 'dave'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Dave', 1, \
         154, 470100, 950100 FROM account WHERE login = 'dave'",
    );
    sql(
        &database,
        "UPDATE player SET stamina = 12345 WHERE name = 'Dave'",
    );
    let (mut alpha, alpha_listed, _alpha_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut dave, dave_listed, _dave_entered) = enter_world_seeing(&server, b"dave", 0);
    alpha.sees_arrive(dave_listed.id, false, None);

    // When: Dave hits Alpha. Then: nothing at all.
    dave.send_record(&client_attack(alpha_listed.id));
    dave.quiet("a hit on a player of his own empire is refused");
    alpha.quiet("a hit on a player of his own empire is refused");
}

/// The exp tail of a kill: the exp point change past the death's ownership releases, then
/// the exp orb from the victim to the killer, then `GC_DEAD` past the death's drops
/// (`GROUND_ADD`) and any late swing's record. The kill's drop table rolls after the exp and
/// before the dead record (`G/char_battle.cpp`'s `Dead`), so the drops are timing noise.
fn read_exp_death(alpha: &mut Keyed, killer: u32, victim: u32, exp: i64) {
    loop {
        let record = alpha.read_fight();
        if record[0] == GC_OWNERSHIP {
            continue;
        }
        assert_eq!(
            record,
            point_change(killer, POINT_EXP, exp, exp),
            "the death's exp record after the ownership"
        );
        break;
    }
    assert_eq!(alpha.read_fight(), fly_of(FLY_EXP, victim, killer));
    let mut dead = alpha.read_fight();
    while matches!(
        dead[0],
        GROUND_ADD | GC_CHARACTER_UPDATE | GC_OWNERSHIP | GC_POINT_CHANGE
    ) {
        dead = alpha.read_fight();
    }
    assert_eq!(dead, dead_of(victim), "the dead record after the drops");
}

/// Ledger 229n (P1): a kill pays the killer the monster's experience with an exp orb.
///
/// Alpha at level 1 with no exp punches the stray dog until a swing stuns it, then punches
/// the stunned dog: it dies at once, and after the death's ownership clears come the exp
/// record (15 at the 100% delta row, onto 0) and the exp orb from the dog to Alpha, then
/// `GC_DEAD` (`DistributeExp` and the live `GiveExp`, `G/char_battle.cpp:2818-2985`,
/// `:2567-2700`). The dog fights back (229l), so its swings' claims on Alpha are read past:
/// the death's clear is the one the exp record follows.
#[test]
fn a_kill_pays_the_killer_the_monsters_exp_with_an_exp_orb() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_with_npcs_configured(
        binary(),
        database.url(),
        "metin2_map_a1",
        DOG_AT_ALPHA,
        "no_wander = true",
    );
    create_account(&server, "alice");
    add_characters(&database);
    sql(
        &database,
        "UPDATE player SET level = 1, exp = 0 WHERE name = 'Alpha'",
    );
    let (mut alpha, listed, _items) = load_character(&server, b"alice", 0);
    let alpha_vid = listed.id;
    let entered = enter_game_shown(&mut alpha);
    let dog = vid_shown_at(
        entered.shown.monsters.iter().map(Vec::as_slice),
        ALPHA_ENTERS_AT,
    );
    alpha.send_record(&client_move(
        FUNC_ATTACK,
        7,
        40,
        ALPHA_ENTERS_AT.0,
        ALPHA_ENTERS_AT.1,
        0x5eed,
    ));
    let update = read_past_pings(&mut alpha);
    assert_eq!(update[0], GC_CHARACTER_UPDATE, "{update:02x?}");
    assert!(alpha.timer_record(&update), "the swing ends the revive");
    assert_eq!(
        read_past_pings(&mut alpha),
        [GC_AFFECT_REMOVE, 215, 0, 0, 0, 0]
    );
    alpha.quiet("the swing is not relayed to the swinger");
    alpha.send_record(&client_target(dog));
    assert_eq!(alpha.read_game(), target_of(dog, 100, DOG_HP, DOG_HP, 0));
    // The level-1 punch lands but does not stun: the dog keeps most of its hit points,
    // and Alpha claims it as the sync owner.
    alpha.send_record(&client_attack(dog));
    let punch = alpha.read_fight();
    assert_eq!(punch[0], GC_DAMAGE_INFO, "{punch:02x?}");
    let shown = alpha.read_fight();
    assert_eq!(shown[0], GC_TARGET, "{shown:02x?}");
    assert_eq!(alpha.read_fight(), ownership_of(alpha_vid, dog));
    // Punch until the dog stuns: each swing shows its damage (a heal tick may land
    // between swings and is skipped), the hit points, and the claim again. The dog stuns
    // when a swing takes its last hit points.
    loop {
        alpha.send_record(&client_attack(dog));
        let _punch = loop {
            let record = alpha.read_fight();
            if record[0] == GC_DAMAGE_INFO {
                break record;
            }
            assert_eq!(
                record[0], GC_POINT_CHANGE,
                "a heal tick at most: {record:02x?}"
            );
        };
        // The stun's hit shows the claim BEFORE the hit points (`show_strike` runs before
        // the stun is sent); a non-stunning hit shows the points then the claim.
        let mut shown = loop {
            let record = alpha.read_fight();
            if record[0] != GC_POINT_CHANGE {
                break record;
            }
        };
        // The monster fights back since 229l: its swing's damage or claim may land
        // between this swing's halves, and is skipped.
        while shown[0] == GC_OWNERSHIP || shown[0] == GC_DAMAGE_INFO {
            shown = alpha.read_fight();
            while shown[0] == GC_POINT_CHANGE {
                shown = alpha.read_fight();
            }
        }
        // The stun's hit: the zeroed hit points... or a plain hit's.
        assert_eq!(shown[0], GC_TARGET, "{shown:02x?}");
        if shown == target_of(dog, 0, 0, DOG_HP, 0) {
            assert_eq!(alpha.read_fight(), stun_of(dog));
            assert_eq!(alpha.read_fight(), ownership_of(alpha_vid, dog));
            break;
        }
        assert_eq!(alpha.read_fight(), ownership_of(alpha_vid, dog));
    }
    // The punch on the stunned dog kills it at once. The dog's last swings may still be
    // in flight: their claims and releases are skipped while the death's own sequence is
    // awaited. A clear followed by a claim is a late swing's; a clear followed by the exp
    // record is the death's.
    alpha.send_record(&client_attack(dog));
    assert_eq!(alpha.read_fight(), target_of(0, 0, 0, 0, 0));
    // Ownership records are skipped until the death's own exp record arrives: the death's
    // clear, the dog's late releases of Alpha, and the stun's own death (if the stun event
    // won the race, its dead_event releases too — but then no second death follows).
    read_exp_death(&mut alpha, alpha_vid, dog, 15);
}

/// Ledger 229j: what waits for the skills is consumed, and an attack on what is no monster is
/// refused, each keeping the connection. `CG_SHOOT`, `CG_FLY_TARGETING` and
/// `CG_ADD_FLY_TARGETING` do nothing (V17f; legacy's relay of a fly target
/// skips its sender, `G/char_battle.cpp:3342-3371`); a `CG_USE_SKILL` of an unlearned
/// skill and a `CG_ATTACK` of a skill's type, on the shopkeeper 100 east, on Alpha
/// itself and on a VID that names nothing do nothing (`G/input_main.cpp:1966-2009`,
/// `G/battle.cpp:99-152`, `G/char_skill.cpp:2896-3135` — the store starts every level
/// at 0). The dog is still whole after.
#[test]
fn skill_shoot_and_fly_frames_and_refused_victims_keep_the_connection() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let lines = format!("{DOG_AT_ALPHA}m\t606\t540\t0\t0\t0\t1\t1000s\t100\t1\t9001\n");
    let (_server, mut alpha, alpha_vid, entered) = alpha_swinging_by(&database, &lines);
    let dog = vid_shown_at(
        entered.shown.monsters.iter().map(Vec::as_slice),
        ALPHA_ENTERS_AT,
    );
    let shopkeeper = vid_shown_at(entered.shown.inserts(), (470_200, 950_000));
    let mut use_skill = vec![CG_USE_SKILL];
    use_skill.extend_from_slice(&1_u32.to_le_bytes());
    use_skill.extend_from_slice(&dog.to_le_bytes());
    for record in [
        vec![CG_SHOOT, 0],
        client_fly(CG_FLY_TARGETING, dog, ALPHA_ENTERS_AT),
        client_fly(CG_ADD_FLY_TARGETING, dog, ALPHA_ENTERS_AT),
        use_skill,
        client_attack_typed(1, dog),
        client_attack(shopkeeper),
        client_attack(alpha_vid),
        client_attack(0x7fff_fff0),
    ] {
        alpha.send_record(&record);
    }
    alpha.quiet("the skills' frames and the refused attacks send nothing");
    alpha.send_record(&client_target(dog));
    assert_eq!(alpha.read_game(), target_of(dog, 100, DOG_HP, DOG_HP, 0));
}

/// A 406 (a level-35 boar, MELEE, HP 1736) at regen point (605, 540), where Alpha enters.
const BOAR_AT_ALPHA: &str = "m\t605\t540\t0\t0\t0\t1\t1000s\t100\t1\t406\n";

/// Ledger 229l (P1): a hit melee monster turns on its attacker and swings every two seconds.
///
/// Alpha punches the 406 it targets, standing on it. The punch does not stun (HP 1736), so the
/// monster runs `BeginFight` and swings: `DAMAGE_INFO`, the hit points, the claim and the
/// closing `MOVE{2}`. The second swing lands 2000 ms on on the server's clock
/// (`CalculateDuration(100, 2000)`). Each swing stamps `last_move`, so the heal tick that lands
/// within four seconds of the first swing adds exactly 29 (15 + 1420/100).
#[test]
fn a_hit_monster_turns_on_its_attacker_and_swings_every_two_seconds() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (_server, mut alpha, alpha_vid, entered) = alpha_swinging_by(&database, BOAR_AT_ALPHA);
    let boar = vid_shown_at(
        entered.shown.monsters.iter().map(Vec::as_slice),
        ALPHA_ENTERS_AT,
    );
    alpha.send_record(&client_target(boar));
    let clicked = alpha.read_game();
    assert_eq!(clicked[0], GC_TARGET, "{clicked:02x?}");
    assert_eq!(&clicked[1..5], &boar.to_le_bytes(), "the boar's VID");

    // When: Alpha punches the boar. Then: the damage, the hit points, no stun, the claim.
    alpha.send_record(&client_attack(boar));
    let punch = alpha.read_fight();
    assert_eq!(punch[0], GC_DAMAGE_INFO, "{punch:02x?}");
    assert_eq!(&punch[1..5], &boar.to_le_bytes(), "the boar's VID");
    let dealt = i32::from_le_bytes(punch[6..10].try_into().expect("four bytes"));
    assert!((1..=450).contains(&dealt), "the punch lands: {dealt}");
    let shown = alpha.read_fight();
    assert_eq!(shown[0], GC_TARGET, "{shown:02x?}");
    let boar_hp = i32::from_le_bytes(shown[6..10].try_into().expect("four bytes"));
    assert_eq!(
        boar_hp,
        1736 - dealt,
        "the punch's damage stays in the hit points"
    );
    assert_eq!(alpha.read_fight(), ownership_of(alpha_vid, boar));

    // Then: the first swing within 2 s, in legacy's record order.
    let first_due = std::time::Instant::now() + Duration::from_secs(2);
    let first = read_swing(&mut alpha, boar, alpha_vid, first_due);
    assert_eq!(
        first.flag, DAMAGE_NORMAL,
        "no critical, penetrate or death blow"
    );
    assert!(
        (1..=450).contains(&first.damage),
        "the swing lands: {}",
        first.damage
    );
    assert_eq!(
        first.hp,
        Some(1420 - i64::from(first.damage)),
        "the hit points below the full 1420"
    );
    assert_eq!(first.owner, Some(boar), "the claim within 250");
    assert!(!first.stunned, "25..=92 never stuns 1420");
    assert_eq!(
        (first.x, first.y),
        ALPHA_ENTERS_AT,
        "the swing holds its point"
    );
    let first_at = std::time::Instant::now();

    // And: a second swing 2000 ms on on the server's clock, the claim sent again.
    let second = read_swing(
        &mut alpha,
        boar,
        alpha_vid,
        first_at + Duration::from_secs(4),
    );
    let gap = second.time.wrapping_sub(first.time);
    assert!(
        (1990..=3000).contains(&gap),
        "the 50-Pulse swing on the server's clock: {gap}"
    );
    // Legacy sends the claim on every accepted swing, even with the owner unchanged: the
    // TODO at the send site says so (`G/char.cpp:5542-5545`).
    assert_eq!(second.owner, Some(boar), "the claim is sent again");
    let before = first.hp.expect("the first swing's hit points");
    let ticked = [0, 29, 86].map(|tick| before + tick - i64::from(second.damage));
    assert!(
        second.hp.is_some_and(|hp| ticked.contains(&hp)),
        "the second swing on top of at most one tick: {:?} from {before}",
        second.hp
    );

    // And: a heal tick of exactly 29 within four seconds of the first swing.
    read_heal_tick(
        &mut alpha,
        boar,
        alpha_vid,
        second.hp.expect("the second swing's hit points"),
        first_at + Duration::from_secs(4),
    );
}

/// Read until the fixed-width record under `header` arrives, skipping pings, timer
/// records and stamina refills by rule.
fn read_until(
    keyed: &mut Keyed,
    header: u8,
    width: usize,
    deadline: std::time::Instant,
) -> Vec<u8> {
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "a record {header} within the window"
        );
        let record = keyed.read_game_raw();
        if record[0] == GC_PING || keyed.timer_record(&record) || is_stamina_refill(&record) {
            continue;
        }
        assert_eq!(
            (record[0], record.len()),
            (header, width),
            "the awaited record: {record:02x?}"
        );
        return record;
    }
}

/// A 403 (a RANGE monster, range 850) at regen point (605, 540), where Alpha enters.
const ARCHER_AT_ALPHA: &str = "m\t605\t540\t0\t0\t0\t1\t1000s\t100\t1\t403\n";

/// Ledger 229s (P4): a hit RANGE monster answers with a shot, and the shot's fly record
/// names its victim (`G/char_battle.cpp:267-277` for the `Attack` arm, `:3342-3371` for
/// `FlyTarget`, `:3070-3154` for `CFuncShoot`).
///
/// Alpha punches the 403 it targets, standing on it. The monster runs `BeginFight` and
/// shoots: the `GC_FLY_TARGETING` around it names it and Alpha, then the `DAMAGE_INFO`
/// lands and the claim and the closing `MOVE{2}` follow as for a swing.
#[test]
fn a_hit_range_monster_answers_with_a_shot_and_its_fly_record() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (_server, mut alpha, alpha_vid, entered) = alpha_swinging_by(&database, ARCHER_AT_ALPHA);
    let archer = vid_shown_at(
        entered.shown.monsters.iter().map(Vec::as_slice),
        ALPHA_ENTERS_AT,
    );
    alpha.send_record(&client_target(archer));
    let clicked = alpha.read_game();
    assert_eq!(clicked[0], GC_TARGET, "{clicked:02x?}");
    assert_eq!(&clicked[1..5], &archer.to_le_bytes(), "the archer's VID");

    // When: Alpha punches the archer. Then: the punch lands and the claim follows.
    alpha.send_record(&client_attack(archer));
    let punch = alpha.read_fight();
    assert_eq!(punch[0], GC_DAMAGE_INFO, "{punch:02x?}");
    let dealt = i32::from_le_bytes(punch[6..10].try_into().expect("four bytes"));
    assert!((1..=450).contains(&dealt), "the punch lands: {dealt}");
    let _shown = alpha.read_fight();
    assert_eq!(alpha.read_fight(), ownership_of(alpha_vid, archer));

    // Then: the shot within 2 s — the fly record first, then the damage.
    let first_due = std::time::Instant::now() + Duration::from_secs(2);
    let fly = read_until(&mut alpha, GC_FLY_TARGETING, FLY_TARGETING_LEN, first_due);
    assert_eq!(&fly[1..5], &archer.to_le_bytes(), "the shooter: {fly:02x?}");
    assert_eq!(
        &fly[5..9],
        &alpha_vid.to_le_bytes(),
        "the victim: {fly:02x?}"
    );
    assert_eq!(
        &fly[9..17],
        &ALPHA_ENTERS_AT
            .0
            .to_le_bytes()
            .iter()
            .chain(&ALPHA_ENTERS_AT.1.to_le_bytes())
            .copied()
            .collect::<Vec<u8>>(),
        "the victim's point: {fly:02x?}"
    );
    let shot = read_swing(&mut alpha, archer, alpha_vid, first_due);
    assert_eq!(shot.flag, DAMAGE_NORMAL, "no critical or penetrate");
    assert!(
        (1..=450).contains(&shot.damage),
        "the shot lands: {}",
        shot.damage
    );
    assert_eq!(shot.owner, Some(archer), "the claim within 250");
}

/// A 115 (an aggressive level-24 boar, sight 2000, range 175, move 130) at regen point
/// (613, 540), 800 east of where Alpha enters.
const BOAR_800_EAST: &str = "m\t613\t540\t0\t0\t0\t1\t1000s\t100\t1\t115\n";

/// Ledger 229l (P2): an aggressive monster chases a player into its reach and swings.
///
/// The 115 stands 800 east, inside its sight of 2000. Alpha ends its revive with a `FUNC_ATTACK`
/// move where it stands. The idle search finds it, Battle waits 25 Pulses, and the chase walks
/// to nine tenths of the range: the first `FUNC_WAIT` whose destination is within 158 of Alpha
/// is the chase's (a wander walks 300-700 from the monster, never that close). Then a swing —
/// unless the chase's last step ends inside its reach, and the swing's `DAMAGE_INFO` arrives
/// before any wait within the window (ledger 229n).
#[test]
fn an_aggressive_monster_chases_a_player_into_its_reach_and_swings() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_with_npcs(binary(), database.url(), "metin2_map_a1", BOAR_800_EAST);
    create_account(&server, "alice");
    add_characters(&database);
    let (mut alpha, listed, _items) = load_character(&server, b"alice", 0);
    let entered = enter_game_shown(&mut alpha);
    let boar = vid_shown_at(
        entered.shown.monsters.iter().map(Vec::as_slice),
        (470_900, 950_000),
    );
    alpha.send_record(&client_move(
        FUNC_ATTACK,
        7,
        40,
        ALPHA_ENTERS_AT.0,
        ALPHA_ENTERS_AT.1,
        0x5eed,
    ));
    let update = read_past_pings(&mut alpha);
    assert_eq!(update[0], GC_CHARACTER_UPDATE, "{update:02x?}");
    assert!(alpha.timer_record(&update), "the swing ends the revive");
    assert_eq!(
        read_past_pings(&mut alpha),
        [GC_AFFECT_REMOVE, 215, 0, 0, 0, 0]
    );
    alpha.quiet("the swing is not relayed to the swinger");

    // Then: the chase's wait within 10 s, then a swing — unless the chase's last step
    // ended inside its reach, and the swing lands first.
    let due = std::time::Instant::now() + Duration::from_secs(10);
    let mut early = None;
    let chase = loop {
        let wait = npc_move_of(&mut alpha, boar, listed.id, FUNC_WAIT, due, &mut early);
        if wait.is_empty() {
            break wait;
        }
        let x = i32::from_le_bytes(wait[8..12].try_into().expect("four bytes"));
        let y = i32::from_le_bytes(wait[12..16].try_into().expect("four bytes"));
        if distance_approx(x - ALPHA_ENTERS_AT.0, y - ALPHA_ENTERS_AT.1) <= 158 {
            break wait;
        }
    };
    let _ = chase;
    let swing = early.unwrap_or_else(|| {
        read_swing(
            &mut alpha,
            boar,
            listed.id,
            std::time::Instant::now() + Duration::from_secs(10),
        )
    });
    assert!(
        (1..=450).contains(&swing.damage),
        "the swing lands: {}",
        swing.damage
    );
}

/// A server whose monsters never wander, for scenarios about something else (ledger 229l).
fn server_without_wander(binary: &std::path::Path, url: &str) -> Server {
    Server::start_configured(binary, url, &default_channels(), "no_wander = true")
}

/// Alpha at level 1 with 1 hit point at its entry point, the 406 beside it, its revive
/// ended by a swing, the boar targeted and punched once: the opening both town-restart
/// scenarios share (ledger 229l). `gold` replaces the purse when given.
fn alpha_level_1_vs_boar(
    database: &ScratchDatabase,
    gold: Option<u64>,
) -> (Server, Keyed, u32, u32) {
    let server = Server::start_with_npcs_configured(
        binary(),
        database.url(),
        "metin2_map_a1",
        BOAR_AT_ALPHA,
        "no_wander = true",
    );
    create_account(&server, "alice");
    add_characters(database);
    let purse = gold.map_or(String::new(), |gold| format!(", gold = {gold}"));
    sql(
        database,
        &format!("UPDATE player SET level = 1, hp = 1{purse} WHERE name = 'Alpha'"),
    );
    let (mut alpha, listed, _items) = load_character(&server, b"alice", 0);
    let entered = enter_game_shown(&mut alpha);
    let boar = vid_shown_at(
        entered.shown.monsters.iter().map(Vec::as_slice),
        ALPHA_ENTERS_AT,
    );
    let alpha_vid = listed.id;
    alpha.send_record(&client_move(
        FUNC_ATTACK,
        7,
        40,
        ALPHA_ENTERS_AT.0,
        ALPHA_ENTERS_AT.1,
        0x5eed,
    ));
    let update = read_past_pings(&mut alpha);
    assert_eq!(update[0], GC_CHARACTER_UPDATE, "{update:02x?}");
    assert!(alpha.timer_record(&update), "the swing ends the revive");
    assert_eq!(
        read_past_pings(&mut alpha),
        [GC_AFFECT_REMOVE, 215, 0, 0, 0, 0]
    );
    alpha.quiet("the swing is not relayed to the swinger");
    alpha.send_record(&client_target(boar));
    let clicked = alpha.read_game();
    assert_eq!(&clicked[1..5], &boar.to_le_bytes(), "the boar's VID");
    alpha.send_record(&client_attack(boar));
    let punch = alpha.read_fight();
    assert_eq!(punch[0], GC_DAMAGE_INFO, "{punch:02x?}");
    let shown = alpha.read_fight();
    assert_eq!(shown[0], GC_TARGET, "{shown:02x?}");
    assert_eq!(alpha.read_fight(), ownership_of(alpha_vid, boar));
    (server, alpha, alpha_vid, boar)
}

/// Ledger 229l (P3a): a player a monster killed restarts now and the gold is stored.
///
/// Alpha at level 1 with 1 hit point and 400000 gold punches the 406. The first swing stuns
/// (damage 178..=245), the next kills, usually: the killing swing sends no `DAMAGE_INFO` of
/// its own, only `GC_DEAD`, `DAMAGE_INFO{8, 0}` and the closing move. When a lagging server
/// lets the stun event win, the death has no killer and no tail. Either way `/restart_now`
/// stands Alpha at full hit points and stores 100000 gold.
#[test]
fn a_player_a_monster_killed_restarts_now_and_the_gold_is_stored() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (_server, mut alpha, alpha_vid, boar) = alpha_level_1_vs_boar(&database, Some(400_000));

    // Then: swings until one stuns, and the death by either path, with the path logged.
    let due = std::time::Instant::now() + Duration::from_secs(20);
    let swings = until_stunned(&mut alpha, boar, alpha_vid, due);
    assert_eq!(alpha.read_fight(), ownership_of(0, alpha_vid));
    // `ClearSync` also releases what the dying player owns: its claim on the boar.
    assert_eq!(alpha.read_fight(), ownership_of(0, boar));
    assert_eq!(alpha.read_fight(), [GC_PHASE, 6]);
    assert_eq!(alpha.read_fight(), dead_of(alpha_vid));
    let (records, quiet) = alpha.drain_game(Duration::from_secs(1), |header| match header {
        GC_PING => 1,
        GC_DAMAGE_INFO => DAMAGE_INFO_LEN,
        GC_MOVE => GC_MOVE_LEN,
        GC_TARGET => TARGET_LEN,
        GC_PHASE => 2,
        GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
        other => panic!("past the death, header {other}"),
    });
    assert_eq!(quiet, Quiet::Open, "the death leaves the connection open");
    // `drain_game` already filtered the NPC's own; the revive timer's, the stamina refill, the
    // boar's recovery target and its ATTACK chat are read past. What is left is the kill's
    // tail, if any.
    let mut tail = Vec::new();
    for record in records {
        if record[0] == GC_PING
            || alpha.timer_record(&record)
            || is_stamina_refill(&record)
            || (record.first() == Some(&GC_TARGET) && record[1..5] == boar.to_le_bytes())
            || record.first() == Some(&GC_CHAT)
        {
            continue;
        }
        tail.push(record);
    }
    // The killing swing's closing `MOVE{2}` is the monster's own, so `drain_game` filtered
    // it as chatter; the `DAMAGE_INFO{8, 0}` alone marks the swing's path.
    let swing_killed = tail == [damage_info(alpha_vid, 8, 0)];
    assert!(
        swing_killed || tail.is_empty(),
        "the kill's tail: {tail:02x?} after {} swings",
        swings.len()
    );

    // When: `/restart_now`. Then: the window closes, the game phase is back, Alpha is shown
    // standing at full hit points with its revive, and the gold change ends the records.
    alpha.send_record(&slash(b"/restart_now"));
    assert_eq!(alpha.read_own(alpha_vid), close_restart_window());
    assert_eq!(alpha.read_own(alpha_vid), [GC_PHASE, PHASE_GAME]);
    assert_eq!(alpha.read_own(alpha_vid), removal_of(alpha_vid));
    let add = alpha.read_own(alpha_vid);
    assert_eq!(add[0], GC_CHARACTER_ADD, "{add:02x?}");
    assert_eq!(&add[1..5], &alpha_vid.to_le_bytes(), "Alpha's VID");
    // `Show` sends the insert and the additional info with the flags as they stand.
    let standing = alpha.read_own(alpha_vid);
    assert_eq!(standing[0], GC_CHAR_ADDITIONAL_INFO, "{standing:02x?}");
    let full = alpha.read_own(alpha_vid);
    assert_eq!(
        full,
        point_change(alpha_vid, 5, 0, 1420),
        "full hit points at level 1 with HT 18"
    );
    if swing_killed {
        // A monster killer flags the death penalty, which level 1 does not pay.
        assert_eq!(
            alpha.read_own(alpha_vid),
            info_to_alpha(b"[LS;901]"),
            "no penalty under level 10"
        );
    }
    let look = alpha.read_revive_update(alpha_vid);
    assert_eq!(look[0], GC_CHARACTER_UPDATE, "{look:02x?}");
    let revived = alpha.read_own(alpha_vid);
    assert_eq!(revived[0], GC_AFFECT_ADD, "{revived:02x?}");
    assert_eq!(
        &revived[1..5],
        &215u32.to_le_bytes(),
        "AffectType is REVIVE_INVISIBLE"
    );
    assert_eq!(
        alpha.read_own(alpha_vid),
        gold_change(alpha_vid, 0, 100_000),
        "400000 less the 300000 cost"
    );
    drop(alpha);
    wait_for(
        &database,
        "(SELECT gold FROM player WHERE name = 'Alpha') = 100000",
    );
}

/// Ledger 229l (P3b): a player a monster killed restarts in town and is warped to the recall
/// point.
///
/// As P3a, without the gold. `/restart_town` in the first seconds is refused with an
/// `[LS;516;n]` line; the first accepted one closes the window, stands Alpha at 50 hit points
/// and warps it to empire 1's recall on map 1, which the stored position then shows.
#[test]
fn a_player_a_monster_killed_restarts_in_town_and_is_warped_to_the_recall_point() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let (_server, mut alpha, alpha_vid, boar) = alpha_level_1_vs_boar(&database, None);

    // Then: the stun, and the death by either path.
    let due = std::time::Instant::now() + Duration::from_secs(20);
    let _swings = until_stunned(&mut alpha, boar, alpha_vid, due);
    assert_eq!(alpha.read_fight(), ownership_of(0, alpha_vid));
    // `ClearSync` also releases what the dying player owns: its claim on the boar.
    assert_eq!(alpha.read_fight(), ownership_of(0, boar));
    assert_eq!(alpha.read_fight(), [GC_PHASE, 6]);
    assert_eq!(alpha.read_fight(), dead_of(alpha_vid));
    let died = std::time::Instant::now();
    // When: `/restart_town` at once, while the death is seconds old. Then: the killing
    // swing's `DAMAGE_INFO{8, 0}` first when it killed, then a wait refusal.
    alpha.send_record(&slash(b"/restart_town"));
    let mut refusal = alpha.read_own(alpha_vid);
    if refusal == damage_info(alpha_vid, 8, 0) {
        refusal = alpha.read_own(alpha_vid);
    }
    assert_eq!(refusal[0], GC_CHAT, "{refusal:02x?}");
    assert!(
        refusal.len() > 10 && refusal[10..].starts_with(b"[LS;516;"),
        "a wait refusal: {refusal:02x?}"
    );
    // The killing swing's tail, if it killed, is already in flight.
    let (records, _) = alpha.drain_game(Duration::from_secs(1), |header| match header {
        GC_PING => 1,
        GC_DAMAGE_INFO => DAMAGE_INFO_LEN,
        GC_MOVE => GC_MOVE_LEN,
        GC_TARGET => TARGET_LEN,
        GC_PHASE => 2,
        GC_CHARACTER_UPDATE => CHARACTER_UPDATE_LEN,
        other => panic!("past the death, header {other}"),
    });
    for record in records {
        if record[0] == GC_PING
            || alpha.timer_record(&record)
            || is_stamina_refill(&record)
            || (record.first() == Some(&GC_TARGET) && record[1..5] == boar.to_le_bytes())
            || record.first() == Some(&GC_CHAT)
        {
            continue;
        }
        // The killing swing's closing move is filtered as the monster's own chatter.
        assert_eq!(
            record,
            damage_info(alpha_vid, 8, 0),
            "the kill's tail: {record:02x?}"
        );
    }

    // When: `/restart_town` about once a second. Then: the warp.
    let deadline = died + Duration::from_secs(15);
    let mut last_try = std::time::Instant::now()
        .checked_sub(Duration::from_secs(1))
        .expect("the clock runs");
    let warp = loop {
        assert!(
            std::time::Instant::now() < deadline,
            "the town restart is accepted within 15 s of the death"
        );
        let wait = Duration::from_secs(1).saturating_sub(last_try.elapsed());
        std::thread::sleep(wait);
        last_try = std::time::Instant::now();
        alpha.send_record(&slash(b"/restart_town"));
        let answer = alpha.read_own(alpha_vid);
        // A refusal is an `[LS;516;n]` line; the accepted restart opens with the window's
        // close, which is a chat record too.
        if answer[0] == GC_CHAT && answer.len() > 10 && answer[10..].starts_with(b"[LS;516;") {
            continue;
        }
        assert_eq!(answer, close_restart_window(), "{answer:02x?}");
        assert_eq!(alpha.read_own(alpha_vid), [GC_PHASE, PHASE_GAME]);
        assert_eq!(
            alpha.read_own(alpha_vid),
            point_change(alpha_vid, 5, 0, 50),
            "50 hit points in town"
        );
        assert_eq!(alpha.read_own(alpha_vid), removal_of(alpha_vid));
        let warp = alpha.read_own(alpha_vid);
        assert_eq!(warp[0], GC_WARP, "{warp:02x?}");
        break warp;
    };
    let x = i32::from_le_bytes(warp[1..5].try_into().expect("four bytes"));
    let y = i32::from_le_bytes(warp[5..9].try_into().expect("four bytes"));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe/map");
    let atlas = gamedata::map_atlas::MapAtlas::load(&dir).expect("the owner's atlas");
    let recall = prodomo::warp::recall_position(&atlas, 1, 1).expect("empire 1's recall on map 1");
    assert_eq!((x, y), recall, "the warp names the recall point");
    drop(alpha);
    wait_for(
        &database,
        &format!("(SELECT (x, y) = ({x}, {y}) FROM player WHERE name = 'Alpha')"),
    );
}

/// A group 701 (leader 702 HP 1950, two 701s HP 1447, all MELEE) at regen point
/// (605, 540), where Alpha enters: the leader in the point's box, each member in a box
/// 300-500 around the one before it. The point's half width is one metre (±100), not
/// legacy's usual three: a ±300 draw can stand the leader past the punch's 250 and the
/// scenario would then depend on the dice.
const GROUP_AT_ALPHA: &str = "g\t605\t540\t1\t1\t0\t1\t1000s\t100\t1\t701\n";

/// Ledger 229t (P5): a hit group member tells its party, and the party swings back.
///
/// Alpha targets the group 701 under it and punches one member. The punch lands without
/// stunning (HP 981+), so the hit member runs `BeginFight` and sends `PM_ATTACKED_BY`,
/// and each other member that `CanBeginFight` begins on Alpha too. Three swings land on
/// Alpha — one from each VID — each with its own `DAMAGE_INFO` on Alpha, hit points, and
/// closing `MOVE{2}` naming its swinger. The swing reader skips across the other members'
/// heads and HP records by rule: every `DAMAGE_INFO` names Alpha, every HP record names
/// Alpha, every closing move names its own swinger.
#[test]
fn a_hit_group_member_tells_its_party_and_the_party_swings_back() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_with_npcs_configured(
        binary(),
        database.url(),
        "metin2_map_a1",
        GROUP_AT_ALPHA,
        "view_range = 10000\nno_wander = true",
    );
    create_account(&server, "alice");
    add_characters(&database);
    let (mut alpha, listed, _items) = load_character(&server, b"alice", 0);
    let alpha_vid = listed.id;
    let entered = enter_game_shown(&mut alpha);
    let swing = client_move(
        FUNC_ATTACK,
        7,
        40,
        ALPHA_ENTERS_AT.0,
        ALPHA_ENTERS_AT.1,
        0x5eed,
    );
    alpha.send_record(&swing);
    let update = read_past_pings(&mut alpha);
    assert_eq!(update[0], GC_CHARACTER_UPDATE, "{update:02x?}");
    assert!(alpha.timer_record(&update), "the swing ends the revive");
    assert_eq!(
        read_past_pings(&mut alpha),
        [GC_AFFECT_REMOVE, 215, 0, 0, 0, 0]
    );
    alpha.quiet("the swing is not relayed to the swinger");
    let mut members: Vec<(u32, i32)> = entered
        .shown
        .monsters
        .iter()
        .map(Vec::as_slice)
        .filter(|insert| {
            matches!(
                u16::from_le_bytes(insert[22..24].try_into().expect("two bytes")),
                701 | 702
            )
        })
        .map(|insert| {
            let at = inserted_at(insert);
            let approx = distance_approx(at.0 - ALPHA_ENTERS_AT.0, at.1 - ALPHA_ENTERS_AT.1);
            (
                u32::from_le_bytes(insert[1..5].try_into().expect("a VID")),
                approx,
            )
        })
        .collect();
    members.sort_unstable();
    assert_eq!(members.len(), 3, "the group stands its three");
    let mut by_vid: Vec<u32> = members.iter().map(|&(vid, _)| vid).collect();
    by_vid.sort_unstable();
    let mut by_range = members;
    by_range.sort_by_key(|&(_, approx)| approx);
    let members = by_vid;
    // The punch's victim: the nearest member, so the punch's own claim lands — the
    // member stands inside 250 — before any swing interleaves.
    let hit = by_range[0].0;
    assert!(
        by_range[0].1 <= 250,
        "the nearest member stands inside 250: {}",
        by_range[0].1
    );
    alpha.send_record(&client_target(hit));
    let clicked = alpha.read_game();
    assert_eq!(clicked[0], GC_TARGET, "{clicked:02x?}");
    assert_eq!(&clicked[1..5], &hit.to_le_bytes(), "the hit member's VID");

    // When: Alpha punches one member. Then: the punch lands, the hit points follow,
    // then the claim — each past whatever the party's first swings interleave. Each
    // wait carries its own window: a slow host can leave the wire quiet past one 5 s
    // beat before the punch's records land.
    alpha.send_record(&client_attack(hit));
    let punch = read_punch_head(&mut alpha, hit);
    let dealt = i32::from_le_bytes(punch[6..10].try_into().expect("four bytes"));
    assert!((1..=450).contains(&dealt), "the punch lands: {dealt}");
    read_punch_tail(&mut alpha, alpha_vid, hit);

    // Then: every member swings on Alpha — three distinct closers — each swing landing.
    // The reads wait out a slow host's quiet beats until one 25 s window, so a
    // member's walk-then-swing gap may run past a beat. A swing's flag is not
    // asserted: the 702's `ENCHANT_CRITICAL` 1 draws a critical one run in a
    // hundred, and a penetrate lands through `ENCHANT_PENETRATE` the same way.
    let due = std::time::Instant::now() + Duration::from_secs(25);
    let mut swingers = Vec::new();
    let mut swings = 0;
    while swingers.len() < 3 {
        let swing = read_party_swing_sized(&mut alpha, &members, alpha_vid, due);
        assert!(
            (1..=900).contains(&swing.damage),
            "the swing lands: {}",
            swing.damage
        );
        swings += 1;
        assert!(
            swings <= 6,
            "three members answer within six swings, else one never began"
        );
        if !swingers.contains(&swing.swung_by) {
            swingers.push(swing.swung_by);
        }
    }
    swingers.sort_unstable();
    assert_eq!(swingers, members, "every member swings back");
}
/// The punch's `DAMAGE_INFO` on `hit`, past whatever the party's first swings
/// interleave, within its own 15 s window.
fn read_punch_head(keyed: &mut Keyed, hit: u32) -> Vec<u8> {
    let wait = || std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let Some(record) = keyed.read_fight_until(wait()) else {
            panic!("the punch lands within its window");
        };
        if record[0] == GC_DAMAGE_INFO && record[1..5] == hit.to_le_bytes() {
            return record;
        }
    }
}
/// After the punch's `DAMAGE_INFO`: the hit points, then the claim, each past whatever
/// the party's first swings interleave. Each wait carries its own 15 s window: a slow
/// host can leave the wire quiet past one 5 s beat before the records land.
fn read_punch_tail(keyed: &mut Keyed, owner: u32, hit: u32) {
    let wait = || std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let Some(record) = keyed.read_fight_until(wait()) else {
            panic!("the hit points within their window");
        };
        if record[0] == GC_TARGET && record[1..5] == hit.to_le_bytes() {
            break;
        }
    }
    // The punch's own claim: the hit stands inside 250, so the claim is sent before
    // any swing. The loop reads past the interleaved swings by rule — a `DAMAGE_INFO`
    // on Alpha, Alpha's HP, a stun triple on Alpha, the swings' claims, their closing
    // moves, and the punch's own effects — and breaks on the punch's own claim.
    loop {
        let Some(record) = keyed.read_fight_until(wait()) else {
            panic!("the claim within its window");
        };
        if record == ownership_of(owner, hit) {
            break;
        }
        let is_swing_claim =
            record[0] == GC_OWNERSHIP && record.get(5..9) == Some(&owner.to_le_bytes());
        let is_swing_head =
            record[0] == GC_DAMAGE_INFO && record.get(1..5) == Some(&owner.to_le_bytes());
        let is_swing_points =
            record[0] == GC_POINT_CHANGE && record.get(4..8) == Some(&owner.to_le_bytes());
        let is_swing_stun = record[0] == GC_STUN && record.get(1..5) == Some(&owner.to_le_bytes());
        let is_swing_death = record[0] == GC_DEAD && record.get(1..5) == Some(&owner.to_le_bytes());
        let is_swing_move = record[0] == GC_MOVE;
        let is_effect = record[0] == GC_SPECIAL_EFFECT;
        let is_stun_points = record[0] == GC_TARGET && record.len() >= 5;
        assert!(
            is_swing_claim
                || is_swing_head
                || is_swing_points
                || is_swing_stun
                || is_swing_death
                || is_swing_move
                || is_effect
                || is_stun_points,
            "past the punch's tail, a record nobody accounted for: {record:02x?}"
        );
    }
}

/// One swing of a party's answer: `read_swing`'s damage and flag plus the swinger's VID.
struct PartySwing {
    /// The `DAMAGE_INFO` damage.
    damage: i32,
    /// The VID of the closing `MOVE{2}`: the monster that swung.
    swung_by: u32,
}

/// Whether `vid` names a member of the party under `members`.
fn is_party_member(members: &[u32], vid: &[u8]) -> bool {
    vid.try_into()
        .is_ok_and(|bytes| members.contains(&u32::from_le_bytes(bytes)))
}

/// Read one swing of a party's answer on `victim`: skip by rule until a `DAMAGE_INFO` on
/// the victim, take the ordered tail — the hit points, the stun triple, the claim — and
/// return with the closing move's VID. Any member's `GC_TARGET` and any other member's
/// closing move are skipped: the party's three `MOVE{2}` arrive in swing order, one per
/// swing, so the first one past the damage is this swing's own. A `DAMAGE_INFO` on a
/// member is the punch's own head or a stun's zero-damage write, read past; the punch's
/// `GC_TARGET` and claim are read past with the members'.
fn read_party_swing_sized(
    keyed: &mut Keyed,
    members: &[u32],
    victim: u32,
    deadline: std::time::Instant,
) -> PartySwing {
    let sized = |header: u8, first: &[u8]| match fight_len(header) {
        usize::MAX => usize::from(first[1]) | (usize::from(first[2]) << 8),
        fixed => fixed,
    };
    let (damage, _flag) = read_party_swing_head(keyed, members, victim, deadline, &sized);
    party_swing_tail(keyed, members, victim, damage, deadline, &sized)
}

/// [`game_len`] plus the fight's `GC_SPECIAL_EFFECT` (byte 114), which the general table
/// does not size: a critical or penetrate shows it on the fight's victim.
fn fight_len(header: u8) -> usize {
    match header {
        GC_SPECIAL_EFFECT => SPECIAL_EFFECT_LEN,
        other => game_len(other),
    }
}

/// Skip by rule until a swing's `DAMAGE_INFO` on `victim`: pings, the revive timer's,
/// stamina refills, the victim's own heal ticks, the members' `GC_TARGET`, the punch's own
/// head and claim, the monsters' chats, and any NPC's own chatter.
fn read_party_swing_head(
    keyed: &mut Keyed,
    members: &[u32],
    victim: u32,
    deadline: std::time::Instant,
    sized: &impl Fn(u8, &[u8]) -> usize,
) -> (i32, u8) {
    let hp_kind = u8::try_from(common::point_slot::POINT_HP).expect("a point byte");
    loop {
        let Some(record) = keyed.read_sized_until(sized, deadline) else {
            panic!("a swing within the window");
        };
        if record[0] == GC_PING || keyed.timer_record(&record) || is_stamina_refill(&record) {
            continue;
        }
        if record[0] == GC_DAMAGE_INFO && record[1..5] == victim.to_le_bytes() {
            break (
                i32::from_le_bytes(record[6..10].try_into().expect("four bytes")),
                record[5],
            );
        }
        let is_heal = record.first() == Some(&GC_POINT_CHANGE)
            && record.get(4..8) == Some(&victim.to_le_bytes())
            && record.get(8) == Some(&hp_kind);
        let is_member_target = record.first() == Some(&GC_TARGET)
            && record
                .get(1..5)
                .is_some_and(|vid| is_party_member(members, vid));
        let is_member_damage = record.first() == Some(&GC_DAMAGE_INFO)
            && record
                .get(1..5)
                .is_some_and(|vid| is_party_member(members, vid));
        let is_member_claim = record.first() == Some(&GC_OWNERSHIP)
            && record
                .get(5..9)
                .is_some_and(|vid| is_party_member(members, vid));
        let is_mob_chat = record.first() == Some(&GC_CHAT);
        let is_effect = record.first() == Some(&GC_SPECIAL_EFFECT)
            && record
                .get(2..6)
                .is_some_and(|vid| vid == victim.to_le_bytes());
        assert!(
            is_heal
                || is_member_target
                || is_member_damage
                || is_member_claim
                || is_mob_chat
                || is_effect
                || is_npc_chatter(&record),
            "past a party's swings, a record nobody accounted for: {record:02x?}"
        );
    }
}

/// Take a party swing's ordered tail after its `DAMAGE_INFO`: the hit points, the stun
/// triple, the claim, and the closing move's VID. The punch's own head and claim are read
/// past with the members'.
fn party_swing_tail(
    keyed: &mut Keyed,
    members: &[u32],
    victim: u32,
    damage: i32,
    deadline: std::time::Instant,
    sized: &impl Fn(u8, &[u8]) -> usize,
) -> PartySwing {
    let hp_kind = u8::try_from(common::point_slot::POINT_HP).expect("a point byte");
    let mut saw_hp = false;
    let mut saw_35 = false;
    let mut saw_36 = false;
    loop {
        let Some(record) = keyed.read_sized_until(sized, deadline) else {
            panic!("a swing's closing move within the window");
        };
        if record[0] == GC_PING || keyed.timer_record(&record) || is_stamina_refill(&record) {
            continue;
        }
        if record[0] == GC_MOVE
            && record.get(1) == Some(&FUNC_ATTACK)
            && record.get(2) == Some(&0)
            && record.get(4..8).is_some_and(|vid| {
                members.contains(&u32::from_le_bytes(vid.try_into().expect("a VID")))
            })
        {
            return PartySwing {
                damage,
                swung_by: u32::from_le_bytes(record[4..8].try_into().expect("a VID")),
            };
        }
        if record[0] == GC_OWNERSHIP && record[5..9] == victim.to_le_bytes() {
            continue;
        }
        if record.first() == Some(&GC_SPECIAL_EFFECT)
            && record
                .get(2..6)
                .is_some_and(|vid| vid == victim.to_le_bytes())
        {
            continue;
        }
        if record.first() == Some(&GC_DAMAGE_INFO)
            && record
                .get(1..5)
                .is_some_and(|vid| is_party_member(members, vid))
        {
            continue;
        }
        if record.first() == Some(&GC_OWNERSHIP)
            && record
                .get(5..9)
                .is_some_and(|vid| is_party_member(members, vid))
        {
            continue;
        }
        if record[0] == GC_STUN && record[1..5] == victim.to_le_bytes() {
            assert!(saw_35 && saw_36, "the stun triple is ordered");
            continue;
        }
        if record.first() == Some(&GC_POINT_CHANGE) && record[4..8] == victim.to_le_bytes() {
            match record.get(8).copied() {
                Some(kind) if kind == hp_kind && !saw_hp => {
                    saw_hp = true;
                }
                Some(kind) if kind == hp_kind => {}
                Some(35) => {
                    assert!(!saw_35, "one stun pool record");
                    saw_35 = true;
                }
                Some(36) => {
                    assert!(saw_35 && !saw_36, "the stun triple is ordered");
                    saw_36 = true;
                }
                kind => panic!("past a swing, a point nobody accounted for: {kind:?}"),
            }
            continue;
        }
        if record.first() == Some(&GC_TARGET)
            && record
                .get(1..5)
                .is_some_and(|vid| is_party_member(members, vid))
        {
            continue;
        }
        assert!(
            is_npc_chatter(&record),
            "past a swing, a record nobody accounted for: {record:02x?}"
        );
    }
}
/// Ledger 230f: `/skillup 109` spends a skill point and sends byte 76 with slot
/// 109 learned, then `CG_USE_SKILL 109` heals: the SP charge and the HP point
/// change, with no affect add. Alpha is stored group 2 with 100/100, so 109 is
/// its own row (group 49 refuses `CanUseSkill`) and the heal's point change
/// goes out (a full pool sends nothing): the charge leaves 50, the heal fills
/// 1420.
#[test]
fn a_skill_up_spends_and_a_cast_heals_over_the_wire() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_with_npcs_configured(
        binary(),
        database.url(),
        "metin2_map_a1",
        DOG_AT_ALPHA,
        "no_wander = true",
    );
    create_account(&server, "alice");
    add_characters(&database);
    sql(
         &database,
         "UPDATE player SET skill_point = 1, skill_group = 2, hp = 100, sp = 100 WHERE name = 'Alpha'",
     );
    let (mut alpha, listed, _items) = load_character(&server, b"alice", 0);
    let alpha_vid = listed.id;
    let _entered = enter_game_shown(&mut alpha);
    alpha.send_record(&client_chat(CHAT_TALKING, b"/skillup 109"));
    // The spend's point change (slot 28) then byte 76.
    let mut saw_spend = false;
    let mut burst = Vec::new();
    for _ in 0..8 {
        let record = alpha.read_game();
        if record == point_change(alpha_vid, 28, 0, 0) {
            assert!(!saw_spend, "one spend");
            saw_spend = true;
            continue;
        }
        if record[0] == GC_SKILL_LEVEL_NEW {
            burst = record;
            break;
        }
        panic!("past a skill-up, a record nobody accounted for: {record:02x?}");
    }
    assert!(saw_spend, "the spend lands");
    assert_eq!(burst[0], GC_SKILL_LEVEL_NEW, "{burst:02x?}");
    assert_eq!(burst.len(), SKILL_LEVEL_LEN);
    // Slot 109: master 0, level 1 (6 bytes a slot, 1 header byte).
    assert_eq!(burst[1 + 109 * 6], 0);
    assert_eq!(burst[1 + 109 * 6 + 1], 1);
    // The cast heals: the SP charge and the HP point change, no affect add.
    let mut use_skill = vec![CG_USE_SKILL];
    use_skill.extend_from_slice(&109_u32.to_le_bytes());
    use_skill.extend_from_slice(&alpha_vid.to_le_bytes());
    alpha.send_record(&use_skill);
    // Alpha's stored 100/100 wounds it, so the heal's point change goes out.
    let mut charged = false;
    let mut healed = false;
    for _ in 0..8 {
        let record = alpha.read_game();
        if record == point_change(alpha_vid, 7, 0, 50) {
            assert!(!charged, "one charge");
            charged = true;
        } else if record == point_change(alpha_vid, 5, 0, 1420) {
            assert!(!healed, "one heal");
            healed = true;
        } else if record[0] == GC_AFFECT_ADD {
            panic!("the heal adds no affect: {record:02x?}");
        } else {
            panic!("past a heal, a record nobody accounted for: {record:02x?}");
        }
        if charged && healed {
            break;
        }
    }
    assert!(charged && healed, "the charge and the heal both land");
}

/// `HEADER_CG_ANSWER_MAKE_GUILD` (`G/packet.h:51`): 0x51.
const CG_ANSWER_MAKE_GUILD: u8 = 0x51;

/// A `CG_ANSWER_MAKE_GUILD`: the header and the 13 raw name bytes.
fn client_answer_make_guild(name: &[u8]) -> Vec<u8> {
    let mut record = vec![CG_ANSWER_MAKE_GUILD];
    let mut field = [0u8; GUILD_NAME_FIELD_SIZE];
    field[..name.len()].copy_from_slice(name);
    record.extend_from_slice(&field);
    assert_eq!(record.len(), 1 + GUILD_NAME_FIELD_SIZE);
    record
}

/// The subheader of a `GC_GUILD` envelope: byte 3.
fn guild_subheader(record: &[u8]) -> u8 {
    assert_eq!(record[0], GC_GUILD, "a guild record: {record:02x?}");
    let size = usize::from(u16::from_le_bytes([record[1], record[2]]));
    assert_eq!(
        size,
        record.len(),
        "the WORD sizes the record: {record:02x?}"
    );
    record[3]
}

/// The GRADE walk's first two grades: grade 1 is `Leader` with auth 15, grade 2 is
/// `...` with auth 0 (`G/guild.cpp:70-83`).
fn check_guild_grade_walk(grades: &[u8]) {
    assert_eq!(grades[4], 15, "15 grades");
    assert_eq!(grades[5], 1, "grade 1 first");
    assert_eq!(&grades[6..12], b"Leader", "the leader's name");
    assert_eq!(grades[14], 0, "grade 1's name pads");
    assert_eq!(grades[15], 15, "the leader's auth");
    assert_eq!(grades[16], 2, "grade 2 next");
    assert_eq!(&grades[17..20], b"...", "the followers' name");
    assert_eq!(grades[26], 0, "grade 2's auth");
}

/// The INFO walk of the one-member guild `id` mastered by `master_pid`
/// (`SendGuildInfoPacket`, `G/guild.cpp:910-939`): one member, room for 32, level 1, no
/// exp, no gold, no land.
fn check_guild_info_walk(info: &[u8], id: u32, master_pid: u32) {
    assert_eq!(u16::from_le_bytes([info[4], info[5]]), 1, "one member");
    assert_eq!(
        u16::from_le_bytes([info[6], info[7]]),
        32,
        "level 1 holds 32"
    );
    assert_eq!(u32::from_le_bytes(info[8..12].try_into().unwrap()), id);
    assert_eq!(
        u32::from_le_bytes(info[12..16].try_into().unwrap()),
        master_pid,
        "the master"
    );
    assert_eq!(
        u32::from_le_bytes(info[16..20].try_into().unwrap()),
        0,
        "no exp"
    );
    assert_eq!(info[20], 1, "level 1");
    assert_eq!(&info[21..28], b"MyGuild", "the info name");
    assert_eq!(
        u32::from_le_bytes(info[34..38].try_into().unwrap()),
        0,
        "no gold"
    );
    assert_eq!(info[38], 0, "no land");
}

/// The LIST walk's one row: the master of `master_pid`, a grade-1 non-general of job
/// 3 at level 154 with no offer (`SendListPacket`, `G/guild.cpp:427-459`).
fn check_guild_list_walk(list: &[u8], master_pid: u32) {
    assert_eq!(list.len(), 4 + 38, "one 38-byte row");
    assert_eq!(
        u32::from_le_bytes(list[4..8].try_into().unwrap()),
        master_pid,
        "the master's row"
    );
    assert_eq!(list[8], 1, "grade 1");
    assert_eq!(list[9], 0, "not a general");
    assert_eq!(list[10], 3, "Alpha's job");
    assert_eq!(list[11], 154, "Alpha's level");
    assert_eq!(
        u32::from_le_bytes(list[12..16].try_into().unwrap()),
        0,
        "no offer"
    );
    assert_eq!(list[16], 1, "the name flag");
    assert_eq!(&list[17..22], b"Alpha", "the member name");
    assert!(
        list[22..42].iter().all(|&byte| byte == 0),
        "the 25-byte name pads: {list:02x?}"
    );
}

/// The `SKILL_INFO` walk of a new guild (`SendSkillInfoPacket`,
/// `G/guild.cpp:1419-1437`): no point, no skills, power and max power 400.
fn check_guild_skill_walk(skills: &[u8]) {
    assert_eq!(skills[4], 0, "no skill point");
    assert!(
        skills[5..17].iter().all(|&byte| byte == 0),
        "no skills: {skills:02x?}"
    );
    assert_eq!(
        u16::from_le_bytes([skills[17], skills[18]]),
        400,
        "power 400"
    );
    assert_eq!(u16::from_le_bytes([skills[19], skills[20]]), 400, "max 400");
}

/// The stored guild of a creation: the row, the grades, and the member
/// (`CGuild::CGuild`'s saves, `G/guild.cpp:59-127`).
fn check_guild_rows(database: &ScratchDatabase, id: u32, master_pid: u32) {
    let guild = support::rows(
        database.url(),
        "SELECT id, master_pid, level FROM guild WHERE name = 'MyGuild'",
        3,
    );
    assert_eq!(guild.len(), 1, "one guild row");
    assert_eq!(guild[0][0], i64::from(id), "the id is the broadcast's");
    assert_eq!(guild[0][1], i64::from(master_pid), "the master");
    assert_eq!(guild[0][2], 1, "level 1");
    let grade_rows = support::rows(
        database.url(),
        "SELECT COUNT(*), SUM(auth) FROM guild_grade WHERE guild_id = 1",
        2,
    );
    assert_eq!(grade_rows[0][0], 15, "fifteen grade rows");
    assert_eq!(grade_rows[0][1], 15, "only the leader holds auth");
    let member_rows = support::rows(
        database.url(),
        "SELECT pid, grade FROM guild_member WHERE guild_id = 1",
        2,
    );
    assert_eq!(member_rows.len(), 1, "the master's member row");
    assert_eq!(member_rows[0][0], i64::from(master_pid), "the master");
    assert_eq!(member_rows[0][1], 1, "grade 1");
}

/// Invite `guest_vid` into `id` and settle: the invite, the accept, and the fans.
fn join_guild(alice: &mut Keyed, yankee: &mut Keyed, guest_vid: u32, id: u32) {
    alice.send_record(&client_guild_add_member(guest_vid));
    yankee.read_game();
    yankee.send_record(&client_guild_invite_answer(id, 1));
    for _ in 0..6 {
        yankee.read_game();
    }
    alice.read_game();
    alice.read_game();
    alice.quiet("the join settled");
    yankee.quiet("the join settled");
}

/// Both members hear the `GRADE_NAME` fan for `grade` and `name`.
fn check_guild_grade_name(alice: &mut Keyed, yankee: &mut Keyed, grade: u8, name: &[u8]) {
    let renamed = alice.read_game();
    assert_eq!(guild_subheader(&renamed), GUILD_SUBHEADER_GC_GRADE_NAME);
    assert_eq!(renamed[4], grade, "the fan names the grade");
    assert_eq!(&renamed[5..5 + name.len()], name, "the fan names the name");
    let renamed = yankee.read_game();
    assert_eq!(guild_subheader(&renamed), GUILD_SUBHEADER_GC_GRADE_NAME);
    assert_eq!(
        &renamed[5..5 + name.len()],
        name,
        "the guest hears the rename"
    );
    alice.quiet("the rename settled");
}

/// Both members hear the `GRADE_AUTH` fan for `grade` and `auth`.
fn check_guild_grade_auth(alice: &mut Keyed, yankee: &mut Keyed, grade: u8, auth: u8) {
    let reauthed = alice.read_game();
    assert_eq!(guild_subheader(&reauthed), GUILD_SUBHEADER_GC_GRADE_AUTH);
    assert_eq!(reauthed[4], grade, "the fan names the grade");
    assert_eq!(reauthed[5], auth, "the fan names the auth");
    let reauthed = yankee.read_game();
    assert_eq!(guild_subheader(&reauthed), GUILD_SUBHEADER_GC_GRADE_AUTH);
    assert_eq!(reauthed[5], auth, "the guest hears the re-auth");
    alice.quiet("the re-auth settled");
}

/// Both members hear the `subheader` fan naming `victim` and `value`.
fn check_guild_member_fan(
    alice: &mut Keyed,
    yankee: &mut Keyed,
    victim: u32,
    value: u8,
    subheader: u8,
    note: &str,
) {
    let fanned = alice.read_game();
    assert_eq!(guild_subheader(&fanned), subheader);
    assert_eq!(
        u32::from_le_bytes(fanned[4..8].try_into().unwrap()),
        victim,
        "the fan names the victim"
    );
    assert_eq!(fanned[8], value, "the fan names the value");
    let fanned = yankee.read_game();
    assert_eq!(guild_subheader(&fanned), subheader);
    alice.quiet(note);
}

/// Offer 20 400 exp and check the outcome: the speaker's debit, the `LIST`-one and
/// the `CHANGE_EXP` to both members at 204 exp and level 1.
fn check_guild_offer_outcome(alice: &mut Keyed, yankee: &mut Keyed, speaker_vid: u32) {
    alice.send_record(&client_guild_offer(20_400));
    assert_eq!(
        alice.read_game(),
        point_change(speaker_vid, POINT_EXP, -20_400, 79_600),
        "the offer debits the speaker"
    );
    let one = alice.read_game();
    assert_eq!(guild_subheader(&one), GUILD_SUBHEADER_GC_LIST);
    let changed = alice.read_game();
    assert_eq!(guild_subheader(&changed), GUILD_SUBHEADER_GC_CHANGE_EXP);
    assert_eq!(changed[4], 1, "level 1");
    assert_eq!(
        u32::from_le_bytes(changed[5..9].try_into().unwrap()),
        204,
        "204 exp"
    );
    let one = yankee.read_game();
    assert_eq!(guild_subheader(&one), GUILD_SUBHEADER_GC_LIST);
    let changed = yankee.read_game();
    assert_eq!(guild_subheader(&changed), GUILD_SUBHEADER_GC_CHANGE_EXP);
    alice.quiet("the offer settled");
    yankee.quiet("the offer settled");
}

/// The `GUILD_NAME` broadcast the master hears first: the envelope, the id, and the
/// 12-byte name (`CGuild::CGuild`, `G/guild.cpp:117-126`). Answers the new id.
fn check_guild_named(named: &[u8]) -> u32 {
    assert_eq!(named.len(), 4 + 4 + 12, "envelope + id + 12 name bytes");
    let id = u32::from_le_bytes(named[4..8].try_into().unwrap());
    assert_eq!(id, 1, "the first guild's id");
    assert_eq!(&named[8..15], b"MyGuild", "the name");
    assert!(
        named[15..20].iter().all(|&byte| byte == 0),
        "the 12-byte name pads to 12: {named:02x?}"
    );
    id
}

/// Give Alpha the 200000 fee and one 70101 in inventory cell 0.
fn give_guild_purse(database: &ScratchDatabase) {
    sql(
        database,
        "UPDATE player SET gold = 5000000 WHERE name = 'Alpha'",
    );
    sql(
        database,
        "INSERT INTO item (id, owner_id, window_type, pos, count, vnum) SELECT 20, id, 1, 0, 1, \
           70101 FROM player WHERE name = 'Alpha'",
    );
}

/// `sys.guild.core`, `cg.guild.answer`, `gc.guild.name`: Alpha founds a guild, and Yankee
/// hears its name. The master's own order is legacy's (`CGuild::CGuild`'s broadcast first,
/// `G/guild.cpp:117-126`, then `[LS;682]`, the fee, the item, the login walk,
/// `G/input_main.cpp:2712-2729` + `G/guild.cpp:270-296`); every other PC hears the
/// `GUILD_NAME` broadcast alone. The guild rows, the fee, and the spent item are all
/// stored.
#[test]
fn a_guild_is_made_and_every_pc_hears_its_name() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1, \
          154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    // Alpha holds the 200000 fee and one 70101 in inventory cell 0.
    give_guild_purse(&database);
    let (mut alice, alpha, alice_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, true, None);
    let alpha_vid = u32::from_le_bytes(alice_entered.add[1..5].try_into().unwrap());
    alice.send_record(&client_answer_make_guild(b"MyGuild"));
    // The master's own order: GUILD_NAME, [LS;682], the fee, the item, the login walk.
    let named = alice.read_game();
    assert_eq!(
        guild_subheader(&named),
        GUILD_SUBHEADER_GC_GUILD_NAME,
        "the broadcast first: {named:02x?}"
    );
    let id = check_guild_named(&named);
    assert_eq!(
        alice.read_game(),
        an_info_line(b"[LS;682;MyGuild]"),
        "the made line"
    );
    assert_eq!(
        alice.read_game(),
        gold_change(alpha_vid, 0, 4_800_000),
        "the fee leaves the purse"
    );
    assert_eq!(
        alice.read_game(),
        a_clear_record(0),
        "the one 70101 is destroyed"
    );
    let grades = alice.read_game();
    assert_eq!(
        guild_subheader(&grades),
        GUILD_SUBHEADER_GC_GRADE,
        "the grade walk: {grades:02x?}"
    );
    check_guild_grade_walk(&grades);
    let info = alice.read_game();
    assert_eq!(
        guild_subheader(&info),
        GUILD_SUBHEADER_GC_INFO,
        "the info walk: {info:02x?}"
    );
    check_guild_info_walk(&info, id, alpha.id);
    let list = alice.read_game();
    assert_eq!(
        guild_subheader(&list),
        GUILD_SUBHEADER_GC_LIST,
        "the list walk: {list:02x?}"
    );
    check_guild_list_walk(&list, alpha.id);
    let login = alice.read_game();
    assert_eq!(
        guild_subheader(&login),
        GUILD_SUBHEADER_GC_LOGIN,
        "its own login: {login:02x?}"
    );
    assert_eq!(
        u32::from_le_bytes(login[4..8].try_into().unwrap()),
        alpha.id,
        "the master's PID logs in"
    );
    let skills = alice.read_game();
    assert_eq!(
        guild_subheader(&skills),
        GUILD_SUBHEADER_GC_SKILL_INFO,
        "the skill walk: {skills:02x?}"
    );
    check_guild_skill_walk(&skills);
    // Yankee hears the broadcast alone, byte-identical.
    assert_eq!(yankee.read_game(), named, "the other PC hears the name");
    yankee.quiet("and nothing else");
    // The store holds the guild, the grades, the member, the fee, and no item.
    check_guild_rows(&database, id, alpha.id);
    assert_eq!(
        query_hp_gold(&database, "Alpha"),
        4_800_000,
        "the fee is stored"
    );
    let items = support::rows(
        database.url(),
        "SELECT COUNT(*) FROM item WHERE owner_id = (SELECT id FROM player WHERE name = \
          'Alpha')",
        1,
    );
    assert_eq!(items[0][0], 0, "the 70101 row is gone");
    drop(alice);
    check_relogin_lists_the_guild(&server, id);
}

/// `sys.guild.core`, `sys.view.spawn`, `sys.char.affect`: a newcomer who sees the master is
/// sent the guild's NAME before the master's add (`ch->SendGuildName(GetGuild())`,
/// `G/char.cpp:1070`), and the master's info and update name the guild id with 3
/// (`G/char.cpp:1176-1199`, `:1327-1352`).
#[test]
fn a_newcomer_hears_the_masters_guild_and_sees_its_id() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    create_account(&server, "carol");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, stamina, x, y) SELECT id, 0, \
          'Yankee', 1, 154, 12345, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, stamina, x, y) SELECT id, 0, \
          'Charlie', 2, 154, 12345, 470000, 950000 FROM account WHERE login = 'carol'",
    );
    give_guild_purse(&database);
    let (mut alice, alpha, _) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, false, None);
    alice.send_record(&client_answer_make_guild(b"MyGuild"));
    let named = alice.read_game();
    let id = check_guild_named(&named);
    for _ in 0..8 {
        alice.read_game();
    }
    assert_eq!(yankee.read_game(), named, "the broadcast");
    yankee.quiet("and nothing else");
    // Charlie enters seeing both: the master's NAME comes with his pair, before his add.
    let (mut charlie, charlie_listed, charlie_entered) = enter_world_seeing(&server, b"carol", 0);
    let at = charlie_entered
        .shown
        .records
        .iter()
        .position(|record| record[0] == GC_GUILD)
        .expect("the NAME in the shown");
    let record = &charlie_entered.shown.records[at];
    assert_eq!(
        guild_subheader(record),
        GUILD_SUBHEADER_GC_GUILD_NAME,
        "the subheader: {record:02x?}"
    );
    assert_eq!(&record[4..8], &id.to_le_bytes(), "the guild id");
    assert_eq!(&record[8..15], b"MyGuild", "the guild name");
    let next = &charlie_entered.shown.records[at + 1];
    assert_eq!(next[0], GC_CHARACTER_ADD, "the master's add follows");
    assert_eq!(&next[1..5], &alpha.id.to_le_bytes(), "the master's VID");
    let (_, master_info) = charlie_entered.shown.pair_of(alpha.id);
    assert_eq!(&master_info[43..47], &id.to_le_bytes(), "dwGuildID");
    assert_eq!(master_info[63], 3, "dwNewIsGuildName names the master");
    let (_, yankee_info) = charlie_entered.shown.pair_of(yankee_listed.id);
    assert_eq!(&yankee_info[43..47], &[0, 0, 0, 0], "the guildless id");
    assert_eq!(yankee_info[63], 0, "the guildless flag");
    // The others see Charlie arrive guildless, with no NAME.
    alice.sees_arrive(charlie_listed.id, false, None);
    yankee.sees_arrive(charlie_listed.id, false, None);
    // Charlie's own update names no guild: he holds none.
    let update = charlie_entered.shown.update.expect("the own update");
    assert_eq!(&update[28..32], &[0, 0, 0, 0], "the own update's id");
    assert_eq!(update[48], 0, "the own update's flag");
    charlie.quiet("and nothing else");
}
/// The next login lists the guild off the linked row (`SendLoginSuccessPacket`,
/// `G/desc.cpp:892-904`): slot 0 carries the id and the `strlcpy` name, the others zeros.
/// The old connection is dropped first, so the login is free.
fn check_relogin_lists_the_guild(server: &Server, id: u32) {
    let (_auth, key) = login_key(server, b"alice");
    let mut relog = Keyed::channel(server.channel(1));
    let (_, relist) = login_by_key_when_free(&mut relog, b"alice", key).expect("relogin");
    assert_eq!(
        u32::from_le_bytes(relist[281..285].try_into().unwrap()),
        id,
        "slot 0's guild id"
    );
    assert_eq!(&relist[297..304], b"MyGuild", "slot 0's guild name");
    assert!(
        relist[304..310].iter().all(|&byte| byte == 0),
        "the 13-byte name pads to 13: {relist:02x?}"
    );
    assert!(
        relist[285..297].iter().all(|&byte| byte == 0)
            && relist[310..349].iter().all(|&byte| byte == 0),
        "the other slots carry zeros: {relist:02x?}"
    );
}

/// The stored gold of `name`.
fn query_hp_gold(database: &ScratchDatabase, name: &str) -> i64 {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime should start")
        .block_on(async {
            use db::sqlx::Connection;
            let mut connection = db::sqlx::PgConnection::connect(database.url())
                .await
                .expect("the scenario database should connect");
            let row: (i64,) = db::sqlx::query_as("SELECT gold FROM player WHERE name = $1")
                .bind(name)
                .fetch_one(&mut connection)
                .await
                .expect("the row should be there");
            connection.close().await.expect("the close should succeed");
            row.0
        })
}

/// `sys.guild.core`, `cg.guild.answer`: a taken name answers `[LS;473]` and stores nothing,
/// and the refusal survives a reboot — the boot load re-installs the guild, so the second
/// server judges the same name taken too. Yankee holds the fee and the item both times, so
/// only the name gate can refuse.
#[test]
fn a_taken_guild_name_is_refused_and_survives_a_reboot() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1, \
          154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    sql(
        &database,
        "UPDATE player SET gold = 5000000 WHERE name IN ('Alpha', 'Yankee')",
    );
    sql(
        &database,
        "INSERT INTO item (id, owner_id, window_type, pos, count, vnum) SELECT 20, id, 1, 0, 1, \
          70101 FROM player WHERE name = 'Alpha'",
    );
    sql(
        &database,
        "INSERT INTO item (id, owner_id, window_type, pos, count, vnum) SELECT 21, id, 1, 0, 1, \
          70101 FROM player WHERE name = 'Yankee'",
    );
    let (mut alice, _, alice_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, true, None);
    let _ = alice_entered;
    alice.send_record(&client_answer_make_guild(b"MyGuild"));
    // The walk lands; only its head matters here — the name is taken now.
    let named = alice.read_game();
    assert_eq!(guild_subheader(&named), GUILD_SUBHEADER_GC_GUILD_NAME);
    // Drain the rest of the master's walk: the line, the fee, the item, five login records.
    for _ in 0..8 {
        alice.read_game();
    }
    assert_eq!(
        yankee.read_game(),
        named,
        "the other PC heard the name first"
    );
    yankee.send_record(&client_answer_make_guild(b"MyGuild"));
    assert_eq!(
        yankee.read_game(),
        an_info_line(b"[LS;473]"),
        "the taken name is refused"
    );
    yankee.quiet("a refusal sends only the line");
    drop(server);
    // The reboot re-installs the stored guild; Yankee's retry is refused again.
    let server = Server::start(binary(), database.url());
    let (mut yankee, _, _) = enter_world_seeing(&server, b"bob", 0);
    yankee.send_record(&client_answer_make_guild(b"myguild"));
    assert_eq!(
        yankee.read_game(),
        an_info_line(b"[LS;473]"),
        "the boot load judges the cased twin taken too"
    );
    yankee.quiet("still only the line");
    // Nothing of Yankee's was spent or stored: one guild row, its fee and item intact.
    let guilds = support::rows(database.url(), "SELECT COUNT(*) FROM guild", 1);
    assert_eq!(guilds[0][0], 1, "one guild row");
    assert_eq!(
        query_hp_gold(&database, "Yankee"),
        5_000_000,
        "Yankee's fee is untouched"
    );
    let items = support::rows(
        database.url(),
        "SELECT COUNT(*) FROM item WHERE owner_id = (SELECT id FROM player WHERE name = \
          'Yankee')",
        1,
    );
    assert_eq!(items[0][0], 1, "Yankee's 70101 row survives");
}

/// `HEADER_CG_GUILD` (`G/packet.h:52`): 0x50.
const CG_GUILD: u8 = 0x50;

/// A `CG_GUILD` `ADD_MEMBER`: the header, the subheader 0, and the guest's VID.
fn client_guild_add_member(guest: u32) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 0];
    record.extend_from_slice(&guest.to_le_bytes());
    record
}

/// A `CG_GUILD` `INVITE_ANSWER`: the header, the subheader 11, the guild id, and the accept
/// byte.
fn client_guild_invite_answer(guild_id: u32, accept: u8) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 11];
    record.extend_from_slice(&guild_id.to_le_bytes());
    record.push(accept);
    record
}

/// A `CG_GUILD` `REMOVE_MEMBER`: the header, the subheader 1, and the victim's PID.
fn client_guild_remove_member(pid: u32) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 1];
    record.extend_from_slice(&pid.to_le_bytes());
    record
}

/// `sys.guild.doors`, `cg.guild.add`, `cg.guild.answer`, `cg.guild.remove`,
/// `gc.guild.invite`, `gc.guild.remove`: Alpha founds a guild, invites Yankee by VID,
/// Yankee accepts and walks the login sends, then Alpha removes Yankee by PID and the
/// REMOVE fans. The member row lands on accept and leaves on removal.
#[test]
fn a_guild_invites_accepts_and_removes() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1, \
          154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    give_guild_purse(&database);
    let (mut alice, _alpha, _alice_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, true, None);
    let yankee_vid = yankee_listed.id;
    let yankee_player = {
        let rows = support::rows(
            database.url(),
            "SELECT id FROM player WHERE name = 'Yankee'",
            1,
        );
        u32::try_from(rows[0][0]).unwrap()
    };
    alice.send_record(&client_answer_make_guild(b"MyGuild"));
    let named = alice.read_game();
    let id = check_guild_named(&named);
    // Drain the master's creation walk: [LS;682], the fee, the item, grades, INFO, LIST,
    // LOGIN, skills.
    for _ in 0..8 {
        alice.read_game();
    }
    assert_eq!(yankee.read_game(), named, "the other PC hears the name");
    alice.quiet("the creation settled");
    yankee.quiet("the creation settled");
    // The invite: the guest alone hears the GUILD_INVITE.
    alice.send_record(&client_guild_add_member(yankee_vid));
    alice.quiet("the inviter hears nothing");
    let invited = yankee.read_game();
    assert_eq!(guild_subheader(&invited), GUILD_SUBHEADER_GC_GUILD_INVITE);
    assert_eq!(
        u32::from_le_bytes(invited[4..8].try_into().unwrap()),
        id,
        "the invite names the guild"
    );
    assert_eq!(&invited[8..15], b"MyGuild", "the invite names the name");
    // The accept: the joiner's walk goes to Yankee, the LIST-one and LOGIN fan.
    yankee.send_record(&client_guild_invite_answer(id, 1));
    let grades = yankee.read_game();
    assert_eq!(guild_subheader(&grades), GUILD_SUBHEADER_GC_GRADE);
    let info = yankee.read_game();
    assert_eq!(guild_subheader(&info), GUILD_SUBHEADER_GC_INFO);
    assert_eq!(
        u16::from_le_bytes(info[4..6].try_into().unwrap()),
        2,
        "two members"
    );
    let list = yankee.read_game();
    assert_eq!(guild_subheader(&list), GUILD_SUBHEADER_GC_LIST);
    let login = yankee.read_game();
    assert_eq!(guild_subheader(&login), GUILD_SUBHEADER_GC_LOGIN);
    let login = yankee.read_game();
    assert_eq!(guild_subheader(&login), GUILD_SUBHEADER_GC_LOGIN);
    let skills = yankee.read_game();
    assert_eq!(guild_subheader(&skills), GUILD_SUBHEADER_GC_SKILL_INFO);
    let one = alice.read_game();
    assert_eq!(guild_subheader(&one), GUILD_SUBHEADER_GC_LIST);
    let join_login = alice.read_game();
    assert_eq!(guild_subheader(&join_login), GUILD_SUBHEADER_GC_LOGIN);
    assert_eq!(
        u32::from_le_bytes(join_login[4..8].try_into().unwrap()),
        yankee_player,
        "the LOGIN names the joiner"
    );
    yankee.quiet("the join settled");
    let members = support::rows(
        database.url(),
        "SELECT pid, grade FROM guild_member WHERE guild_id = 1 ORDER BY pid",
        2,
    );
    assert_eq!(members.len(), 2, "master and joiner rows");
    assert_eq!(members[1][0], i64::from(yankee_player), "the joiner");
    assert_eq!(members[1][1], 15, "grade 15");
    // The removal: the REMOVE fans to the master; the victim hears nothing.
    alice.send_record(&client_guild_remove_member(yankee_player));
    let removed = alice.read_game();
    assert_eq!(guild_subheader(&removed), GUILD_SUBHEADER_GC_REMOVE);
    assert_eq!(
        u32::from_le_bytes(removed[4..8].try_into().unwrap()),
        yankee_player,
        "the REMOVE names the victim"
    );
    alice.quiet("the removal settled");
    yankee.quiet("the victim hears no REMOVE");
    let members = support::rows(
        database.url(),
        "SELECT COUNT(*) FROM guild_member WHERE guild_id = 1",
        1,
    );
    assert_eq!(members[0][0], 1, "the joiner's row is gone");
}

/// A `CG_GUILD` `CHANGE_GRADE_NAME`: the header, the subheader 2, the grade byte, and
/// the 9 name field bytes.
fn client_guild_grade_name(grade: u8, name: &[u8]) -> Vec<u8> {
    let mut field = [0; 9];
    let len = name.len().min(9);
    field[..len].copy_from_slice(&name[..len]);
    let mut record = vec![CG_GUILD, 2, grade];
    record.extend_from_slice(&field);
    record
}

/// A `CG_GUILD` `CHANGE_GRADE_AUTHORITY`: the header, the subheader 3, the grade byte,
/// and the auth byte.
fn client_guild_grade_auth(grade: u8, auth: u8) -> Vec<u8> {
    vec![CG_GUILD, 3, grade, auth]
}

/// A `CG_GUILD` `CHANGE_MEMBER_GRADE`: the header, the subheader 8, the victim's PID,
/// and the grade byte.
fn client_guild_member_grade(pid: u32, grade: u8) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 8];
    record.extend_from_slice(&pid.to_le_bytes());
    record.push(grade);
    record
}

/// A `CG_GUILD` `CHANGE_MEMBER_GENERAL`: the header, the subheader 10, the victim's
/// PID, and the flag byte.
fn client_guild_member_general(pid: u32, is_general: u8) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 10];
    record.extend_from_slice(&pid.to_le_bytes());
    record.push(is_general);
    record
}

/// A `CG_GUILD` `DEPOSIT_MONEY`: the header, the subheader 13, and the gold DWORD.
fn client_guild_deposit_money(gold: u32) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 13];
    record.extend_from_slice(&gold.to_le_bytes());
    record
}

/// `sys.guild.grades`, `cg.guild.grades`, `gc.guild.grades`: Alpha founds a guild,
/// Yankee joins, and the master renames grade 15, re-auths it, re-grades Yankee to
/// 5, and flags Yankee general — each fan reaching both members, each row landing
/// in the store — while a money arm stays quiet.
#[test]
fn a_guild_renames_regrades_and_flags_its_general() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1, \
          154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    give_guild_purse(&database);
    let (mut alice, _alpha, _alice_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, true, None);
    let yankee_vid = yankee_listed.id;
    let yankee_player = {
        let rows = support::rows(
            database.url(),
            "SELECT id FROM player WHERE name = 'Yankee'",
            1,
        );
        u32::try_from(rows[0][0]).unwrap()
    };
    alice.send_record(&client_answer_make_guild(b"MyGuild"));
    let named = alice.read_game();
    let id = check_guild_named(&named);
    for _ in 0..8 {
        alice.read_game();
    }
    assert_eq!(yankee.read_game(), named, "the other PC hears the name");
    alice.quiet("the creation settled");
    yankee.quiet("the creation settled");
    join_guild(&mut alice, &mut yankee, yankee_vid, id);
    // The rename: both members hear the GRADE_NAME fan.
    alice.send_record(&client_guild_grade_name(15, b"Veteran"));
    check_guild_grade_name(&mut alice, &mut yankee, 15, b"Veteran");
    // The re-auth: both members hear the GRADE_AUTH fan.
    alice.send_record(&client_guild_grade_auth(15, 7));
    check_guild_grade_auth(&mut alice, &mut yankee, 15, 7);
    // The re-grade: both members hear the CHANGE_MEMBER_GRADE fan.
    alice.send_record(&client_guild_member_grade(yankee_player, 5));
    check_guild_member_fan(
        &mut alice,
        &mut yankee,
        yankee_player,
        5,
        GUILD_SUBHEADER_GC_CHANGE_MEMBER_GRADE,
        "the re-grade settled",
    );
    // The general flag: both members hear the CHANGE_MEMBER_GENERAL fan.
    alice.send_record(&client_guild_member_general(yankee_player, 1));
    check_guild_member_fan(
        &mut alice,
        &mut yankee,
        yankee_player,
        1,
        GUILD_SUBHEADER_GC_CHANGE_MEMBER_GENERAL,
        "the flag settled",
    );
    yankee.quiet("the flag settled");
    // The money arm stays quiet and stores nothing.
    alice.send_record(&client_guild_deposit_money(1000));
    alice.quiet("a closed money arm stays quiet");
    // Every row lands in the store: the rename as its 7 letters' length and the
    // auth as its number (`rows` reads integers only).
    let grades = support::rows(
        database.url(),
        "SELECT length(name), auth FROM guild_grade WHERE guild_id = 1 AND grade = 15",
        2,
    );
    assert_eq!(grades.len(), 1, "the grade row");
    assert_eq!(grades[0][0], 7, "Veteran lands");
    assert_eq!(grades[0][1], 7, "the auth lands");
    let members = support::rows(
        database.url(),
        "SELECT grade, is_general::int FROM guild_member WHERE pid = (SELECT id FROM player WHERE name = 'Yankee')",
        2,
    );
    assert_eq!(members.len(), 1, "the member row");
    assert_eq!(members[0][0], 5, "the re-grade lands");
    assert_eq!(members[0][1], 1, "the flag lands");
}

/// A `CG_GUILD` `OFFER`: the header, the subheader 4, and the offered exp DWORD.
fn client_guild_offer(exp: u32) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 4];
    record.extend_from_slice(&exp.to_le_bytes());
    record
}

/// A `CG_GUILD` `CHARGE_GSP`: the header, the subheader 12, and the SP DWORD.
fn client_guild_charge(sp: u32) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 12];
    record.extend_from_slice(&sp.to_le_bytes());
    record
}

/// A `CG_GUILD` `POST_COMMENT`: the header, the subheader 5, the length byte, and the
/// content.
fn client_guild_post_comment(content: &[u8]) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 5, u8::try_from(content.len()).unwrap()];
    record.extend_from_slice(content);
    record
}

/// A `CG_GUILD` `REFRESH_COMMENT`: the header and the subheader 7.
fn client_guild_refresh_comment() -> Vec<u8> {
    vec![CG_GUILD, 7]
}

/// A `CG_GUILD` `DELETE_COMMENT`: the header, the subheader 6, and the comment id.
fn client_guild_delete_comment(id: u32) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 6];
    record.extend_from_slice(&id.to_le_bytes());
    record
}

/// `sys.guild.money`, `sys.guild.comments`, the OFFER/CHARGE/COMMENT arms: Alpha founds a
/// guild, offers 20 400 exp and charges 50 SP, and the `LIST`-one, the `CHANGE_EXP` and the
/// `SKILL_INFO` fan to both members; a post and a refresh carry the `COMMENTS` self packet,
/// and a delete clears it. Every row lands in the store.
#[test]
fn a_guild_offers_charges_posts_and_deletes() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1,           154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    // The offer's exp and the charge's gold: Alpha holds 100 000 exp and the purse
    // keeps 5 000 000 past the 200 000 fee.
    sql(
        &database,
        "UPDATE player SET exp = 100000 WHERE name = 'Alpha'",
    );
    give_guild_purse(&database);
    let (mut alice, _alpha, alice_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, true, None);
    let alpha_vid = u32::from_le_bytes(alice_entered.add[1..5].try_into().unwrap());
    let yankee_vid = yankee_listed.id;
    alice.send_record(&client_answer_make_guild(b"MyGuild"));
    let named = alice.read_game();
    let id = check_guild_named(&named);
    for _ in 0..8 {
        alice.read_game();
    }
    assert_eq!(yankee.read_game(), named, "the other PC hears the name");
    join_guild(&mut alice, &mut yankee, yankee_vid, id);
    // The offer: the speaker's exp debits, and the LIST-one and the CHANGE_EXP fan to
    // both members. 20 400 offers 204: the guild holds 204 exp at level 1.
    check_guild_offer_outcome(&mut alice, &mut yankee, alpha_vid);
    // The charge: the headroom caps the 200 SP to 50 — the power is already 350 of
    // 400 — so the debit is 5 000 gold, the SKILL_INFO fans to both members at 400,
    // and the `[LS;938]` count chats the 50 to the speaker alone.
    alice.send_record(&client_guild_charge(200));
    // The point debit queues through the outbox, so it lands after the commit's
    // SKILL_INFO fan and the count line the descriptor sends directly: the fan
    // rides `write_to_client` at the commit, the count rides `send_all` after
    // it, and the judge's debit — queued first — flushes last, because the
    // session writes the direct answers before it drains the queue.
    // The count line the descriptor sends directly lands before the commit's
    // SKILL_INFO fan: `send_all` writes now, while the fan queues through the
    // outbox and flushes on the next turn.
    assert_eq!(
        alice.read_game(),
        an_info_line(b"[LS;938;0]"),
        "the count chats the capped SP"
    );
    assert_eq!(
        alice.read_game(),
        point_change(alpha_vid, 11, 0, 4_800_000),
        "a full power debits nothing"
    );
    let skills = alice.read_game();
    assert_eq!(guild_subheader(&skills), GUILD_SUBHEADER_GC_SKILL_INFO);
    let skills = yankee.read_game();
    assert_eq!(guild_subheader(&skills), GUILD_SUBHEADER_GC_SKILL_INFO);
    alice.quiet("the charge settled");
    yankee.quiet("the charge settled");
    // The post: the COMMENTS self packet carries the one row.
    alice.send_record(&client_guild_post_comment(b"hello all"));
    let comments = alice.read_game();
    assert_eq!(guild_subheader(&comments), GUILD_SUBHEADER_GC_COMMENTS);
    assert_eq!(comments[4], 1, "one row");
    assert_eq!(
        u32::from_le_bytes(comments[5..9].try_into().unwrap()),
        1,
        "the row's id"
    );
    assert_eq!(&comments[9..14], b"Alpha", "the poster");
    assert_eq!(&comments[34..43], b"hello all", "the content");
    yankee.quiet("a post is self");
    // The refresh: the same board, to Yankee's own session.
    yankee.send_record(&client_guild_refresh_comment());
    let comments = yankee.read_game();
    assert_eq!(guild_subheader(&comments), GUILD_SUBHEADER_GC_COMMENTS);
    assert_eq!(comments[4], 1, "one row");
    alice.quiet("a refresh is self");
    // The delete: the leader's delete clears the board.
    alice.send_record(&client_guild_delete_comment(1));
    let comments = alice.read_game();
    assert_eq!(guild_subheader(&comments), GUILD_SUBHEADER_GC_COMMENTS);
    assert_eq!(comments[4], 0, "no rows left");
    // Every row lands: the offer's 204, the charge's power, and no comment.
    let guild = support::rows(
        database.url(),
        "SELECT exp, level, sp FROM guild WHERE id = 1",
        3,
    );
    assert_eq!(guild[0][0], 204, "the offer lands");
    assert_eq!(guild[0][1], 1, "level 1");
    assert_eq!(guild[0][2], 400, "the power lands");
    let member = support::rows(
        database.url(),
        "SELECT offer FROM guild_member WHERE guild_id = 1 AND pid = (SELECT id FROM player \
          WHERE name = 'Alpha')",
        1,
    );
    assert_eq!(member[0][0], 204, "the member's offer lands");
    let player = support::rows(
        database.url(),
        "SELECT exp, gold FROM player WHERE name = 'Alpha'",
        2,
    );
    assert_eq!(player[0][0], 79_600, "the exp debit lands");
    assert_eq!(player[0][1], 4_800_000, "no gold leaves a full power");
    let comments = support::rows(
        database.url(),
        "SELECT COUNT(*) FROM guild_comment WHERE guild_id = 1",
        1,
    );
    assert_eq!(comments[0][0], 0, "the delete lands");
}

/// A `CG_GUILD` `USE_SKILL`: the header, the subheader 9, the vnum DWORD and the target
/// PID DWORD (`TPacketCGGuildUseSkill`, `G/packet.h`).
fn client_guild_use_skill(vnum: u32, pid: u32) -> Vec<u8> {
    let mut record = vec![CG_GUILD, 9];
    record.extend_from_slice(&vnum.to_le_bytes());
    record.extend_from_slice(&pid.to_le_bytes());
    record
}

/// The bodies of a `SKILL_INFO` fan: point, the 12 skill bytes, power, max power.
fn skill_info_body(skills: &[u8]) -> (u8, Vec<u8>, u16, u16) {
    assert_eq!(
        guild_subheader(skills),
        GUILD_SUBHEADER_GC_SKILL_INFO,
        "a SKILL_INFO: {skills:02x?}"
    );
    let point = skills[4];
    let levels = skills[5..17].to_vec();
    let power = u16::from_le_bytes(skills[17..19].try_into().unwrap());
    let max = u16::from_le_bytes(skills[19..21].try_into().unwrap());
    (point, levels, power, max)
}

/// Both members hear the `SKILL_INFO` fan with `point`, `levels`, `power` and `max`.
fn check_guild_skill_fan(
    alice: &mut Keyed,
    yankee: &mut Keyed,
    point: u8,
    levels: &[u8],
    power: u16,
    max: u16,
    note: &str,
) {
    assert_eq!(
        skill_info_body(&alice.read_game()),
        (point, levels.to_vec(), power, max),
        "{note} (the master)"
    );
    assert_eq!(
        skill_info_body(&yankee.read_game()),
        (point, levels.to_vec(), power, max),
        "{note} (the guest)"
    );
}

/// The level byte of a `GUILD_INFO` walk or fan: envelope (4), `member_count` (2),
/// `max_member_count` (2), `guild_id` (4), `master_pid` (4), `exp` (4), then the byte.
fn guild_info_level(info: &[u8]) -> u8 {
    assert_eq!(
        guild_subheader(info),
        GUILD_SUBHEADER_GC_INFO,
        "an INFO: {info:02x?}"
    );
    info[20]
}

/// Offer `offered` exp and check the one-level outcome: the speaker's debit, the
/// `LIST`-one, the `CHANGE_EXP` at `level` and `exp`, and one `INFO`, to both members.
fn check_guild_offer_level(
    alice: &mut Keyed,
    yankee: &mut Keyed,
    speaker_vid: u32,
    offered: u32,
    left: i64,
    level: u8,
    exp: u32,
) {
    alice.send_record(&client_guild_offer(offered));
    assert_eq!(
        alice.read_game(),
        point_change(speaker_vid, POINT_EXP, -i64::from(offered), left),
        "the offer debits the speaker"
    );
    let check = |member: &mut Keyed| {
        let one = member.read_game();
        assert_eq!(guild_subheader(&one), GUILD_SUBHEADER_GC_LIST);
        let changed = member.read_game();
        assert_eq!(guild_subheader(&changed), GUILD_SUBHEADER_GC_CHANGE_EXP);
        assert_eq!(changed[4], level, "level {level}");
        assert_eq!(
            u32::from_le_bytes(changed[5..9].try_into().unwrap()),
            exp,
            "{exp} exp"
        );
        assert_eq!(
            guild_info_level(&member.read_game()),
            level,
            "the INFO names level {level}"
        );
    };
    check(alice);
    check(yankee);
    alice.quiet("the offer settled");
    yankee.quiet("the offer settled");
}

/// Up 152 at level 2 and cast it: the climb fans, the pointless re-up is quiet, the
/// guest hears `[LS;524]`, the cast fans the consumed power with `[LS;937]`, and the
/// cooling re-cast answers `[LS;932]`. `levels` gains 152's byte.
fn check_guild_up_and_cast_152(alice: &mut Keyed, yankee: &mut Keyed, levels: &mut [u8; 12]) {
    alice.send_record(&slash(b"/gskillup 152"));
    levels[1] = 1;
    check_guild_skill_fan(
        &mut *alice,
        &mut *yankee,
        0,
        levels,
        500,
        500,
        "the up fans",
    );
    alice.quiet("the up settled");
    yankee.quiet("the up settled");
    // No point left: the line is quiet.
    alice.unanswered(&slash(b"/gskillup 152"));
    yankee.quiet("a quiet up fans nothing");
    // The guest is no leader: `[LS;524]`, to her own session.
    yankee.send_record(&slash(b"/gskillup 152"));
    assert_eq!(
        yankee.read_game(),
        an_info_line(b"[LS;524]"),
        "a non-leader climbs nothing"
    );
    yankee.quiet("the refusal settled");
    alice.quiet("a refusal is self");
    // The 152 cast: `150 + 150 * (1/7) = 171.4` truncates to 171, and 500 - 171 = 329.
    // The fan carries the consumed power to both members, and the war gate's `[LS;937]`
    // answers the caster alone.
    alice.send_record(&client_guild_use_skill(152, 0));
    check_guild_skill_fan(
        &mut *alice,
        &mut *yankee,
        0,
        levels,
        329,
        500,
        "the cast fans",
    );
    assert_eq!(
        alice.read_game(),
        an_info_line(b"[LS;937]"),
        "no war runs, so 937 answers"
    );
    alice.quiet("the cast settled");
    yankee.quiet("the cast settled");
    // Still cooling: the next cast answers `[LS;932]` and fans nothing.
    alice.send_record(&client_guild_use_skill(152, 0));
    assert_eq!(
        alice.read_game(),
        an_info_line(b"[LS;932]"),
        "the cooling cast refuses"
    );
    alice.quiet("the refusal settled");
    yankee.quiet("a refusal fans nothing");
}

/// Up 158 at level 3 and cast the teleport: the climb fans, the cast at an offline PID
/// answers `[LS;935]` with the empty cost consuming nothing, and the cast at Yankee warps
/// the caster to her feet. `levels` gains 158's byte.
fn check_guild_up_and_cast_teleport(
    alice: &mut Keyed,
    yankee: &mut Keyed,
    database: &ScratchDatabase,
    server: &Server,
    speaker_vid: u32,
    levels: &mut [u8; 12],
) {
    alice.send_record(&slash(b"/gskillup 158"));
    levels[7] = 1;
    check_guild_skill_fan(
        &mut *alice,
        &mut *yankee,
        0,
        levels,
        600,
        600,
        "the up fans",
    );
    alice.quiet("the up settled");
    yankee.quiet("the up settled");
    // The teleport at an offline PID: `[LS;935]`, and the empty cost consumes nothing.
    alice.send_record(&client_guild_use_skill(158, 999_999));
    check_guild_skill_fan(
        &mut *alice,
        &mut *yankee,
        0,
        levels,
        600,
        600,
        "the cast fans",
    );
    assert_eq!(
        alice.read_game(),
        an_info_line(b"[LS;935]"),
        "an offline victim chats 935"
    );
    alice.quiet("the cast settled");
    yankee.quiet("the cast settled");
    // The 0-second cooldown re-arms on the next tick; a breath lets the Pulses run.
    std::thread::sleep(Duration::from_millis(500));
    // The teleport at Yankee: the fan first, then the departure's own removal and the
    // warp to her feet, on this Channel's port.
    let victim_pid = support::rows(
        database.url(),
        "SELECT id FROM player WHERE name = 'Yankee'",
        1,
    )[0][0];
    alice.send_record(&client_guild_use_skill(
        158,
        u32::try_from(victim_pid).unwrap(),
    ));
    check_guild_skill_fan(
        &mut *alice,
        &mut *yankee,
        0,
        levels,
        600,
        600,
        "the cast fans",
    );
    let mut removed = vec![GC_CHARACTER_DEL];
    removed.extend_from_slice(&speaker_vid.to_le_bytes());
    assert_eq!(alice.read_game(), removed, "EncodeRemovePacket(this)");
    assert_eq!(
        alice.read_game(),
        a_warp_record((471_000, 951_000), server.channel(1).port()),
        "the caster warps to the victim"
    );
    let logout = yankee.read_game();
    assert_eq!(guild_subheader(&logout), GUILD_SUBHEADER_GC_LOGOUT);
    let departed = support::rows(
        database.url(),
        "SELECT id FROM player WHERE name = 'Alpha'",
        1,
    )[0][0];
    assert_eq!(
        u32::from_le_bytes(logout[4..8].try_into().unwrap()),
        u32::try_from(departed).unwrap(),
        "the LOGOUT names the departed"
    );
    yankee.sees_leave(speaker_vid);
    yankee.quiet("the warp settled");
}

/// `sys.guild.skills`, the `/gskillup` line and the `USE_SKILL` arm: Alpha founds a guild
/// and offers it to level 3, ups 152 and 158, and casts both. The 152 cast answers the
/// war gate's `[LS;937]` with the SP consumed and 932 while it cools; the 158 cast at an
/// offline PID answers `[LS;935]`, and at Yankee warps the caster to her feet. Every row
/// lands in the store.
#[test]
fn a_guild_ups_and_casts_its_skills() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    create_account(&server, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1, \
         154, 471000, 951000 FROM account WHERE login = 'bob'",
    );
    // Two offers of 600 000 and 1 800 000: level 2, then level 3, with 600 000 left.
    sql(
        &database,
        "UPDATE player SET exp = 3000000 WHERE name = 'Alpha'",
    );
    give_guild_purse(&database);
    let (mut alice, _alpha, alice_entered) = enter_world_seeing(&server, b"alice", 0);
    let (mut yankee, yankee_listed, _) = enter_world_seeing(&server, b"bob", 0);
    alice.sees_arrive(yankee_listed.id, true, None);
    let alpha_vid = u32::from_le_bytes(alice_entered.add[1..5].try_into().unwrap());
    let yankee_vid = yankee_listed.id;
    alice.send_record(&client_answer_make_guild(b"MyGuild"));
    let named = alice.read_game();
    let id = check_guild_named(&named);
    for _ in 0..8 {
        alice.read_game();
    }
    assert_eq!(yankee.read_game(), named, "the other PC hears the name");
    join_guild(&mut alice, &mut yankee, yankee_vid, id);
    // Level 2: 6 000 of the 6 000 steps, nothing left, one point, the max at 500.
    check_guild_offer_level(&mut alice, &mut yankee, alpha_vid, 600_000, 2_400_000, 2, 0);
    // `/gskillup 152`: the point is spent, 152 climbs to 1, and the fan names both.
    let mut levels = [0u8; 12];
    check_guild_up_and_cast_152(&mut alice, &mut yankee, &mut levels);
    // Level 3: 18 000 of the 18 000 steps, nothing left, one more point, the max at 600.
    check_guild_offer_level(&mut alice, &mut yankee, alpha_vid, 1_800_000, 600_000, 3, 0);
    check_guild_up_and_cast_teleport(
        &mut alice,
        &mut yankee,
        &database,
        &server,
        alpha_vid,
        &mut levels,
    );
    // Every row lands: level 3 with no exp left, the climbs, and the consumed power.
    let guild = support::rows(
        database.url(),
        "SELECT exp, level, sp, skill_point, get_byte(skills, 1), get_byte(skills, 7) FROM guild \
         WHERE id = 1",
        6,
    );
    assert_eq!(guild[0][0], 0, "no exp left");
    assert_eq!(guild[0][1], 3, "level 3");
    assert_eq!(
        guild[0][2], 600,
        "level 3 fills the power, and the teleport costs nothing"
    );
    assert_eq!(guild[0][3], 0, "no point left");
    assert_eq!(guild[0][4], 1, "152 climbed to 1");
    assert_eq!(guild[0][5], 1, "158 climbed to 1");
    let player = support::rows(
        database.url(),
        "SELECT exp, x, y FROM player WHERE name = 'Alpha'",
        3,
    );
    assert_eq!(player[0][0], 600_000, "the offer debits land");
    assert_eq!(player[0][1], 471_000, "the warp lands");
    assert_eq!(player[0][2], 951_000, "the warp lands");
}

/// Read one setup-key-sealed record whose plaintext length `len_of` reads from the first
/// decrypted unit: the mark answers are dynamic, so their head names their length.
fn expect_sealed_sized(client: &mut Client, len_of: impl Fn(&[u8]) -> usize) -> Vec<u8> {
    let mut wire = client.expect_bytes(8);
    let len = len_of(&decrypt_padded(&wire, &SETUP_KEY).expect("aligned"));
    wire.extend(client.expect_bytes(len.div_ceil(8) * 8 - 8));
    let mut plain = decrypt_padded(&wire, &SETUP_KEY).expect("aligned");
    assert!(
        plain[len..].iter().all(|&byte| byte == 0),
        "padding {plain:02x?}"
    );
    plain.truncate(len);
    plain
}

/// Found guild 1 (`MyGuild`, by Alpha) on a mark-serving `server`: the account, the
/// characters, the purse, the founding record, and the drained creation walk.
fn found_guild_for_marks(server: &Server, database: &ScratchDatabase) {
    create_account(server, "alice");
    add_characters(database);
    sql(database, "UPDATE account SET empire = 1");
    give_guild_purse(database);
    let (mut alice, _alpha, _entered) = enter_world_seeing(server, b"alice", 0);
    alice.send_record(&client_answer_make_guild(b"MyGuild"));
    let named = alice.read_game();
    assert_eq!(check_guild_named(&named), 1);
    for _ in 0..8 {
        alice.read_game();
    }
    alice.quiet("the creation settled");
}

/// A mark-downloader connection: past `MARK_LOGIN` on `server`'s Channel 1, on the setup
/// key. The login goes out plaintext with nonzero handle and key, which legacy never
/// reads, and the phase answer comes back plaintext, before TEA starts.
fn mark_downloader(server: &Server) -> Client {
    let (mut client, _) = accepted(server.channel(1));
    client.send(&CgMarkLogin::new(0x0102_0304, 0x0506_0708).encode());
    assert_eq!(client.expect_bytes(2), [GC_PHASE, PHASE_LOGIN]);
    client
}

/// The `bufSize` a mark index-list or block answer carries at bytes 1..5.
fn mark_buf_size(head: &[u8]) -> usize {
    u32::from_le_bytes(head[1..5].try_into().unwrap()) as usize
}

/// `sys.guild.mark`, `cg.handshake.mark_login`, `cg.login.mark_login`,
/// `cg.login.mark_crclist`, `cg.login.mark_idxlist`, `cg.login.mark_upload`,
/// `gc.mark_idxlist`, `gc.mark_block`: Alpha founds a guild, and a downloader logs in
/// with an unread handle and key, while a non-serving process closes the login. A login-phase `MARK_LOGIN` is consumed, the empty
/// index list answers, an upload for an unknown guild is quiet and stores nothing, the
/// upload for guild 1 lands mark id 0 with its 768 bytes in the store, the index list
/// names the pair, and the CRC list against all zeros diffs every block in ascending
/// position order. A reboot keeps the pair, and an all-zero upload deletes it.
#[test]
fn a_guild_mark_is_uploaded_listed_and_fetched() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    // A fresh guild is level 1, so the floor is 1; the judge still refuses unknown guilds.
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "guild_mark_min_level = 1",
    );
    found_guild_for_marks(&server, &database);
    let mut downloader = mark_downloader(&server);
    // A login-phase MARK_LOGIN is consumed with nothing done.
    downloader.send(&sealed(&CgMarkLogin::new(1, 2).encode()));
    let (heard, quiet) = downloader.drain(Duration::from_millis(300));
    assert!(heard.is_empty(), "consumed, no effect: {heard:02x?}");
    assert_eq!(quiet, Quiet::Open);
    // No marks yet: the empty index list.
    downloader.send(&sealed(&CgMarkIdxList::new().encode()));
    assert_eq!(
        expect_sealed_sized(&mut downloader, mark_buf_size),
        vec![HEADER_GC_MARK_IDXLIST, 7, 0, 0, 0, 0, 0]
    );
    // An upload for an unknown guild is quiet and stores nothing.
    let mut image = [0u8; 768];
    image[0..4].copy_from_slice(&0x1122_3344u32.to_le_bytes());
    downloader.send(&sealed(&CgMarkUpload::new(999, image).encode()));
    let (heard, quiet) = downloader.drain(Duration::from_millis(300));
    assert!(heard.is_empty(), "an unknown guild is quiet: {heard:02x?}");
    assert_eq!(quiet, Quiet::Open);
    // The upload for guild 1: quiet on the wire, one row in the store.
    downloader.send(&sealed(&CgMarkUpload::new(1, image).encode()));
    let (heard, quiet) = downloader.drain(Duration::from_millis(300));
    assert!(heard.is_empty(), "an upload answers nothing: {heard:02x?}");
    assert_eq!(quiet, Quiet::Open);
    let mark = support::rows(
        database.url(),
        "SELECT mark_id, length(image), get_byte(image, 0), get_byte(image, 3) FROM guild_mark \
         WHERE guild_id = 1",
        4,
    );
    assert_eq!(mark, vec![vec![0, 768, 0x44, 0x11]], "mark id 0, the bytes");
    // The index list names the pair.
    downloader.send(&sealed(&CgMarkIdxList::new().encode()));
    assert_eq!(
        expect_sealed_sized(&mut downloader, mark_buf_size),
        vec![HEADER_GC_MARK_IDXLIST, 11, 0, 0, 0, 1, 0, 1, 0, 0, 0]
    );
    // The CRC list against all zeros: every block differs, in ascending position order.
    downloader.send(&sealed(&CgMarkCRCList::new(0, [0; 80]).encode()));
    let blocks = expect_sealed_sized(&mut downloader, mark_buf_size);
    assert_eq!(blocks[0], HEADER_GC_MARK_BLOCK);
    assert_eq!(blocks[5], 0, "imgIdx");
    let count = u32::from_le_bytes(blocks[6..10].try_into().unwrap());
    assert_eq!(count, 80, "every zero CRC differs");
    let mut at = 10;
    for pos in 0..80u8 {
        assert_eq!(blocks[at], pos, "ascending position");
        let comp = u32::from_le_bytes(blocks[at + 1..at + 5].try_into().unwrap()) as usize;
        assert!(comp > 0, "block {pos} carries bytes");
        at += 5 + comp;
    }
    assert_eq!(at, blocks.len(), "bufSize covers the triples");
    assert_eq!(at, mark_buf_size(&blocks), "bufSize is exact");
    // A reboot keeps the pair: the boot install reads the row back.
    drop(downloader);
    drop(server);
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "guild_mark_min_level = 1",
    );
    let mut downloader = mark_downloader(&server);
    downloader.send(&sealed(&CgMarkIdxList::new().encode()));
    assert_eq!(
        expect_sealed_sized(&mut downloader, mark_buf_size),
        vec![HEADER_GC_MARK_IDXLIST, 11, 0, 0, 0, 1, 0, 1, 0, 0, 0],
        "the reboot kept the pair"
    );
    // An all-zero upload deletes the mark: the list empties and the row is gone.
    downloader.send(&sealed(&CgMarkUpload::new(1, [0; 768]).encode()));
    let (heard, quiet) = downloader.drain(Duration::from_millis(300));
    assert!(heard.is_empty(), "a delete answers nothing: {heard:02x?}");
    assert_eq!(quiet, Quiet::Open);
    downloader.send(&sealed(&CgMarkIdxList::new().encode()));
    assert_eq!(
        expect_sealed_sized(&mut downloader, mark_buf_size),
        vec![HEADER_GC_MARK_IDXLIST, 7, 0, 0, 0, 0, 0],
        "the delete emptied the list"
    );
    let marks = support::rows(database.url(), "SELECT COUNT(*) FROM guild_mark", 1);
    assert_eq!(marks[0][0], 0, "the row is gone");
    // A process that serves no marks closes the downloader's login instead.
    drop(downloader);
    drop(server);
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "guild_mark_server = false",
    );
    let (mut refused, _) = accepted(server.channel(1));
    refused.send(&CgMarkLogin::new(0x0102_0304, 0x0506_0708).encode());
    assert_eq!(refused.expect_closed(), Vec::<u8>::new());
}

/// The IEEE CRC-32 of `bytes`, bit by bit: the scenario's own witness, sharing no code
/// with the server. The symbol scenario checks it against the standard vector first.
fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xffff_ffff
}

/// `sys.guild.mark`, `cg.login.guild_symbol_upload`, `cg.login.symbol_crc`,
/// `gc.symbol_data`: Alpha founds a guild, and a downloader uploads 25 symbol bytes.
/// The bytes land in the store, a CRC with the wrong word is answered with the bytes,
/// and a CRC that matches both size and CRC — plus one for an unknown guild — is
/// quiet. The 25 bytes keep the 7-byte prefix a whole number of TEA units, so no
/// padding byte follows the record on the wire.
#[test]
fn a_guild_symbol_is_uploaded_and_served() {
    assert_eq!(crc32_ieee(b"123456789"), 0xcbf4_3926, "the standard vector");
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start_configured(
        binary(),
        database.url(),
        &default_channels(),
        "guild_mark_min_level = 1",
    );
    found_guild_for_marks(&server, &database);
    let mut downloader = mark_downloader(&server);
    let symbol = b"the-guild-symbol-bytes-01";
    assert_eq!(symbol.len() % 8, 1, "7 + 25 seals with no padding");
    // An upload for an unknown guild is quiet and stores nothing.
    downloader.send(&sealed(
        &encode_guild_symbol_upload(999, symbol).expect("a 25-byte symbol encodes"),
    ));
    let (heard, quiet) = downloader.drain(Duration::from_millis(300));
    assert!(heard.is_empty(), "an unknown guild is quiet: {heard:02x?}");
    assert_eq!(quiet, Quiet::Open);
    // The upload for guild 1: quiet on the wire, one row in the store.
    downloader.send(&sealed(
        &encode_guild_symbol_upload(1, symbol).expect("a 25-byte symbol encodes"),
    ));
    let (heard, quiet) = downloader.drain(Duration::from_millis(300));
    assert!(heard.is_empty(), "an upload answers nothing: {heard:02x?}");
    assert_eq!(quiet, Quiet::Open);
    let stored = support::rows(
        database.url(),
        "SELECT length(data), get_byte(data, 0), get_byte(data, 24) FROM guild_symbol WHERE \
         guild_id = 1",
        3,
    );
    assert_eq!(stored, vec![vec![25, i64::from(b't'), i64::from(b'1')]]);
    // A CRC with the wrong word is answered with the stored bytes.
    let size = u32::try_from(symbol.len()).unwrap();
    downloader.send(&sealed(&CgSymbolCrc::new(1, 0xdead_beef, size).encode()));
    let mut want = vec![HEADER_GC_SYMBOL_DATA, 32, 0, 1, 0, 0, 0];
    want.extend_from_slice(symbol);
    assert_eq!(
        expect_sealed_sized(&mut downloader, |head| usize::from(u16::from_le_bytes(
            head[1..3].try_into().unwrap()
        ))),
        want
    );
    // A CRC that matches both size and CRC is quiet, as an unknown guild's is.
    downloader.send(&sealed(
        &CgSymbolCrc::new(1, crc32_ieee(symbol), size).encode(),
    ));
    let (heard, quiet) = downloader.drain(Duration::from_millis(300));
    assert!(heard.is_empty(), "a match is quiet: {heard:02x?}");
    assert_eq!(quiet, Quiet::Open);
    downloader.send(&sealed(&CgSymbolCrc::new(999, 0, size).encode()));
    let (heard, quiet) = downloader.drain(Duration::from_millis(300));
    assert!(heard.is_empty(), "an unknown guild is quiet: {heard:02x?}");
    assert_eq!(quiet, Quiet::Open);
    let symbols = support::rows(database.url(), "SELECT COUNT(*) FROM guild_symbol", 1);
    assert_eq!(symbols[0][0], 1, "only guild 1's row");
}

/// Seed two war-ready guilds: `MyGuild` (id 1, Alpha master) and `Wolves` (id 2,
/// Yankee master), each with a positive ladder and eight member rows — the master
/// plus seven offline fillers.
fn seed_war_pair(database: &ScratchDatabase) {
    for seat in 0..14 {
        sql(
            database,
            &format!(
                "INSERT INTO account (login, password_hash, delete_code) VALUES ('filler{seat}', \
                 '$argon2id$v=19$m=16,t=2,p=1$c2FsdA$c3VwZXJzZWNyZXRwYXNzd29yZA', 'ABCDEFG') ON \
                 CONFLICT DO NOTHING"
            ),
        );
        sql(
            database,
            &format!(
                "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, \
                 'Filler{seat}', 1, 10, 470000, 950000 FROM account WHERE login = 'filler{seat}'"
            ),
        );
    }
    sql(
        database,
        "INSERT INTO guild (id, name, master_pid, ladder_point) VALUES (1, 'MyGuild', \
         (SELECT id FROM player WHERE name = 'Alpha'), 1000), (2, 'Wolves', (SELECT id \
         FROM player WHERE name = 'Yankee'), 1000)",
    );
    sql(
        database,
        "INSERT INTO guild_grade (guild_id, grade, name, auth) SELECT g, grade, CASE WHEN \
         grade = 1 THEN 'Leader' ELSE '...' END, CASE WHEN grade = 1 THEN 15 ELSE 0 END \
         FROM (VALUES (1), (2)) guilds(g) CROSS JOIN generate_series(1, 15) grade",
    );
    sql(
        database,
        "INSERT INTO guild_member (pid, guild_id, grade) SELECT id, 1, CASE WHEN name = \
         'Alpha' THEN 1 ELSE 2 END FROM player WHERE name IN ('Alpha', 'Filler0', \
         'Filler1', 'Filler2', 'Filler3', 'Filler4', 'Filler5', 'Filler6')",
    );
    sql(
        database,
        "INSERT INTO guild_member (pid, guild_id, grade) SELECT id, 2, CASE WHEN name = \
         'Yankee' THEN 1 ELSE 2 END FROM player WHERE name IN ('Yankee', 'Filler7', \
         'Filler8', 'Filler9', 'Filler10', 'Filler11', 'Filler12', 'Filler13')",
    );
}

/// A `CHAT_TYPE_NOTICE` line to a character of empire 1.
fn a_notice_line(text: &[u8]) -> Vec<u8> {
    let size = u8::try_from(10 + text.len()).expect("a short line");
    let mut record = vec![GC_CHAT, size, 0, 2, 0, 0, 0, 0, 1, 1];
    record.extend_from_slice(text);
    record
}

/// `sys.guild.war`: Alpha's `/war Wolves` declares — both masters hear the opp's
/// NAME then the WAR record, and every client hears the `[LS;475]` notice naming
/// the pair. Yankee's `/nowar MyGuild` refuses: Alpha hears `[LS;476]` naming the
/// refuser, and both ends fan WAR END. Yankee's `/war MyGuild` back is the accept,
/// which starts the war (`a_guild_war_is_accepted_scored_and_listed`).
#[test]
fn a_guild_war_is_declared_refused_and_fanned() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    create_account_before_boot(&database, "alice");
    create_account_before_boot(&database, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1, \
         154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    // Before the boot: the stored guilds install once, while the gate is closed.
    seed_war_pair(&database);
    let server = Server::start(binary(), database.url());
    let (mut alice, _alpha, alice_entered) = enter_world_seeing_guilded(&server, b"alice", 0, 1);
    let (mut yankee, yankee_listed, yankee_entered) =
        enter_world_seeing_guilded(&server, b"bob", 0, 1);
    let _alpha_vid = u32::from_le_bytes(alice_entered.add[1..5].try_into().unwrap());
    alice.sees_arrive(yankee_listed.id, true, Some(b"Wolves"));
    // Yankee's burst already showed Alpha — its insert, its summary and MyGuild's
    // NAME (consumed by the guilded enter's walk drain) — so nothing is left to
    // arrive on its side.
    let burst = &yankee_entered.shown.records;
    assert!(
        burst
            .iter()
            .any(|record| record[0] == GC_CHARACTER_ADD && record[1..5] == alice_entered.add[1..5]),
        "the burst shows Alpha: {burst:02x?}"
    );
    // The declare: Alice hears the WAR record (the NAME rode the spawn view), and
    // both hear the notice.
    alice.send_record(&client_chat(CHAT_TALKING, b"/war Wolves"));
    let war = alice.read_game();
    assert_eq!(
        guild_subheader(&war),
        GUILD_SUBHEADER_GC_WAR,
        "the fan: {war:02x?}"
    );
    assert_eq!(
        u32::from_le_bytes(war[4..8].try_into().unwrap()),
        1,
        "dwGuildSelf"
    );
    assert_eq!(
        u32::from_le_bytes(war[8..12].try_into().unwrap()),
        2,
        "dwGuildOpp"
    );
    assert_eq!(war[12], 0, "bType is field");
    assert_eq!(war[13], 1, "bWarState is SEND_DECLARE");
    let war = yankee.read_game();
    assert_eq!(
        guild_subheader(&war),
        GUILD_SUBHEADER_GC_WAR,
        "the fan: {war:02x?}"
    );
    assert_eq!(
        u32::from_le_bytes(war[4..8].try_into().unwrap()),
        2,
        "dwGuildSelf"
    );
    assert_eq!(
        u32::from_le_bytes(war[8..12].try_into().unwrap()),
        1,
        "dwGuildOpp"
    );
    assert_eq!(war[12], 0, "bType is field");
    assert_eq!(war[13], 3, "bWarState is RECV_DECLARE");
    assert_eq!(
        alice.read_game(),
        a_notice_line(b"[LS;475;MyGuild;Wolves]"),
        "Alice hears the declare"
    );
    assert_eq!(
        yankee.read_game(),
        a_notice_line(b"[LS;475;MyGuild;Wolves]"),
        "Yankee hears the declare"
    );
    alice.quiet("the declare settled");
    yankee.quiet("the declare settled");
    // The refuse: Alpha hears who refused, then both ends fan WAR END.
    yankee.send_record(&client_chat(CHAT_TALKING, b"/nowar MyGuild"));
    assert_eq!(
        alice.read_game(),
        an_info_line(b"[LS;476;Wolves]"),
        "the declarer hears who refused"
    );
    let end = alice.read_game();
    assert_eq!(
        guild_subheader(&end),
        GUILD_SUBHEADER_GC_WAR,
        "the END: {end:02x?}"
    );
    assert_eq!(end[13], 7, "bWarState is END");
    let end = yankee.read_game();
    assert_eq!(
        guild_subheader(&end),
        GUILD_SUBHEADER_GC_WAR,
        "the END: {end:02x?}"
    );
    assert_eq!(end[13], 7, "bWarState is END");
    alice.quiet("the refuse settled");
    yankee.quiet("the refuse settled");
}
/// `sys.guild.war`: Yankee's `/war MyGuild` back accepts — both masters hear the WAR
/// fan with `bWarState` `ON_WAR`, every PC hears the `WAR_LIST` naming the pair, and
/// both hear the `[LS;558]` notice. A kill across the war scores (world tests pin the
/// bytes: the `WAR_SCORE` fans and the `[LS;7801]`/`[LS;7802]` lines); the live
/// scenario proves the accept and the list, because `attack` refuses a player victim
/// until the `PvP` slice.
#[test]
fn a_guild_war_is_accepted_and_every_pc_hears_the_list() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    create_account_before_boot(&database, "alice");
    create_account_before_boot(&database, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1, \
         154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    // Before the boot: the stored guilds install once, while the gate is closed.
    seed_war_pair(&database);
    let server = Server::start(binary(), database.url());
    let (mut alice, _alpha, _alice_entered) = enter_world_seeing_guilded(&server, b"alice", 0, 1);
    let (mut yankee, yankee_listed, _yankee_entered) =
        enter_world_seeing_guilded(&server, b"bob", 0, 1);
    alice.sees_arrive(yankee_listed.id, true, Some(b"Wolves"));
    // The declare, as in the refused scenario: Alpha's WAR fan, then Yankee's.
    alice.send_record(&client_chat(CHAT_TALKING, b"/war Wolves"));
    let war = alice.read_game();
    assert_eq!(war[13], 1, "bWarState is SEND_DECLARE");
    let war = yankee.read_game();
    assert_eq!(war[13], 3, "bWarState is RECV_DECLARE");
    assert_eq!(
        alice.read_game(),
        a_notice_line(b"[LS;475;MyGuild;Wolves]"),
        "Alice hears the declare"
    );
    assert_eq!(
        yankee.read_game(),
        a_notice_line(b"[LS;475;MyGuild;Wolves]"),
        "Yankee hears the declare"
    );
    // The accept: Yankee's `/war MyGuild` back starts the war. Each master hears
    // its fan's `[LS;558]` first, then the WAR record with ON_WAR, and the
    // every-PC WAR_LIST naming the pair lands after both fans.
    yankee.send_record(&client_chat(CHAT_TALKING, b"/war MyGuild"));
    for (side, client) in [("Alice", &mut alice), ("Yankee", &mut yankee)] {
        assert_eq!(
            client.read_game(),
            an_info_line(b"[LS;558]"),
            "{side} hears the start"
        );
        let war = client.read_game();
        assert_eq!(
            guild_subheader(&war),
            GUILD_SUBHEADER_GC_WAR,
            "{side}'s fan: {war:02x?}"
        );
        assert_eq!(war[12], 0, "{side}'s bType is field");
        assert_eq!(war[13], 6, "{side}'s bWarState is ON_WAR");
        let list = client.read_game();
        assert_eq!(
            guild_subheader(&list),
            GUILD_SUBHEADER_GC_GUILD_WAR_LIST,
            "{side}'s list: {list:02x?}"
        );
        assert_eq!(
            &list[4..12],
            &[1, 0, 0, 0, 2, 0, 0, 0],
            "{side}'s list names the pair"
        );
    }
    // The accept's notice fans last: the war declaration line naming the pair.
    assert_eq!(
        alice.read_game(),
        a_notice_line(b"[LS;477;Wolves;MyGuild]"),
        "Alice hears the declaration"
    );
    assert_eq!(
        yankee.read_game(),
        a_notice_line(b"[LS;477;Wolves;MyGuild]"),
        "Yankee hears the declaration"
    );
    alice.quiet("the start settled");
    yankee.quiet("the start settled");
}

/// `sys.guild.war`: a battle war's declare lands — the map index holds the battle map
/// (`GuildWar_IsWarMap`, `G/guild_war.cpp:260-271`), so the master hears the WAR fan
/// with `bType` 1 and not the `[LS;669]` refusal, and the accept starts the war on that
/// type. The war's private copy of the map is not client-visible yet: the entry that
/// warps the members into it is ledger 237's.
#[test]
fn a_battle_war_is_declared_and_accepted() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    create_account_before_boot(&database, "alice");
    create_account_before_boot(&database, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1, \
         154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    // Before the boot: the stored guilds install once, while the gate is closed.
    seed_war_pair(&database);
    let server = Server::start(binary(), database.url());
    let (mut alice, _alpha, _alice_entered) = enter_world_seeing_guilded(&server, b"alice", 0, 1);
    let (mut yankee, yankee_listed, _yankee_entered) =
        enter_world_seeing_guilded(&server, b"bob", 0, 1);
    alice.sees_arrive(yankee_listed.id, true, Some(b"Wolves"));
    // The declare: the battle war lands, because the map index holds map 110.
    alice.send_record(&client_chat(CHAT_TALKING, b"/war Wolves 1"));
    let war = alice.read_game();
    assert_eq!(
        guild_subheader(&war),
        GUILD_SUBHEADER_GC_WAR,
        "the fan: {war:02x?}"
    );
    assert_eq!(war[12], 1, "bType is battle");
    assert_eq!(war[13], 1, "bWarState is SEND_DECLARE");
    let war = yankee.read_game();
    assert_eq!(war[12], 1, "bType is battle");
    assert_eq!(war[13], 3, "bWarState is RECV_DECLARE");
    assert_eq!(
        alice.read_game(),
        a_notice_line(b"[LS;475;MyGuild;Wolves]"),
        "Alice hears the declare"
    );
    assert_eq!(
        yankee.read_game(),
        a_notice_line(b"[LS;475;MyGuild;Wolves]"),
        "Yankee hears the declare"
    );
    // The accept: the war starts on the battle type.
    yankee.send_record(&client_chat(CHAT_TALKING, b"/war MyGuild 1"));
    for (side, client) in [("Alice", &mut alice), ("Yankee", &mut yankee)] {
        assert_eq!(
            client.read_game(),
            an_info_line(b"[LS;558]"),
            "{side} hears the start"
        );
        let war = client.read_game();
        assert_eq!(
            guild_subheader(&war),
            GUILD_SUBHEADER_GC_WAR,
            "{side}'s fan: {war:02x?}"
        );
        assert_eq!(war[12], 1, "{side}'s bType is battle");
        assert_eq!(war[13], 6, "{side}'s bWarState is ON_WAR");
        let list = client.read_game();
        assert_eq!(
            guild_subheader(&list),
            GUILD_SUBHEADER_GC_GUILD_WAR_LIST,
            "{side}'s list: {list:02x?}"
        );
        assert_eq!(
            &list[4..12],
            &[1, 0, 0, 0, 2, 0, 0, 0],
            "{side}'s list names the pair"
        );
    }
    assert_eq!(
        alice.read_game(),
        a_notice_line(b"[LS;477;Wolves;MyGuild]"),
        "Alice hears the declaration"
    );
    assert_eq!(
        yankee.read_game(),
        a_notice_line(b"[LS;477;Wolves;MyGuild]"),
        "Yankee hears the declaration"
    );
    alice.quiet("the battle war settled");
    yankee.quiet("the battle war settled");
}

/// The war scenario's opening: two war-ready guilds (`MyGuild` under Alpha, Wolves under
/// Yankee), both masters in the world and seeing each other, with Alpha granted
/// `LOW_WIZARD` before the boot — the character select reads the grant
/// (`G/input_db.cpp:1931-1955`), so a later insert would be too late. `None` without a
/// store.
fn a_war_pair_ready() -> Option<(ScratchDatabase, Server, Keyed, Keyed)> {
    let (database, server, alice, _alpha, yankee, _yankee) = a_war_pair_ready_at(None)?;
    Some((database, server, alice, yankee))
}

/// As [`a_war_pair_ready`], with Alpha's stored hit points set to `hp` before the boot - the
/// load takes them, so a hit takes it - and both masters' listed characters returned, so a
/// scenario can address them.
fn a_war_pair_ready_at(
    hp: Option<i32>,
) -> Option<(ScratchDatabase, Server, Keyed, Listed, Keyed, Listed)> {
    let database = ScratchDatabase::create()?;
    create_account_before_boot(&database, "alice");
    create_account_before_boot(&database, "bob");
    add_characters(&database);
    sql(&database, "UPDATE account SET empire = 1");
    sql(
        &database,
        "INSERT INTO player (account_id, slot, name, job, level, x, y) SELECT id, 0, 'Yankee', 1, \
         154, 470000, 950000 FROM account WHERE login = 'bob'",
    );
    if let Some(hp) = hp {
        sql(
            &database,
            &format!("UPDATE player SET hp = {hp} WHERE name = 'Alpha'"),
        );
    }
    // Before the boot: the stored guilds install once, while the gate is closed.
    seed_war_pair(&database);
    sql(
        &database,
        "INSERT INTO gm_grant (account_id, name, authority) SELECT id, 'Alpha', 'LOW_WIZARD' \
         FROM account WHERE login = 'alice'",
    );
    let server = Server::start(binary(), database.url());
    let (mut alice, alpha_listed, _alice_entered) =
        enter_world_seeing_guilded(&server, b"alice", 0, 1);
    let (yankee, yankee_listed, _yankee_entered) =
        enter_world_seeing_guilded(&server, b"bob", 0, 1);
    alice.sees_arrive(yankee_listed.id, true, Some(b"Wolves"));
    Some((database, server, alice, alpha_listed, yankee, yankee_listed))
}

/// The declare and the accept between the fixture pair, as
/// `a_guild_war_is_accepted_and_every_pc_hears_the_list` reads them: both masters' WAR
/// fans, the accept's `[LS;558]` lines, the every-PC `WAR_LIST`, and the
/// `[LS;475]`/`[LS;477]` notices.
fn declare_and_accept_war(alice: &mut Keyed, yankee: &mut Keyed) {
    alice.send_record(&client_chat(CHAT_TALKING, b"/war Wolves"));
    assert_eq!(alice.read_game()[13], 1, "bWarState is SEND_DECLARE");
    assert_eq!(yankee.read_game()[13], 3, "bWarState is RECV_DECLARE");
    assert_eq!(
        alice.read_game(),
        a_notice_line(b"[LS;475;MyGuild;Wolves]"),
        "Alice hears the declare"
    );
    assert_eq!(
        yankee.read_game(),
        a_notice_line(b"[LS;475;MyGuild;Wolves]"),
        "Yankee hears the declare"
    );
    yankee.send_record(&client_chat(CHAT_TALKING, b"/war MyGuild"));
    for (side, client) in [("Alice", &mut *alice), ("Yankee", &mut *yankee)] {
        assert_eq!(
            client.read_game(),
            an_info_line(b"[LS;558]"),
            "{side} hears the start"
        );
        let war = client.read_game();
        assert_eq!(
            guild_subheader(&war),
            GUILD_SUBHEADER_GC_WAR,
            "{side}'s fan: {war:02x?}"
        );
        assert_eq!(war[13], 6, "{side}'s bWarState is ON_WAR");
        let list = client.read_game();
        assert_eq!(
            guild_subheader(&list),
            GUILD_SUBHEADER_GC_GUILD_WAR_LIST,
            "{side}'s list: {list:02x?}"
        );
    }
    assert_eq!(
        alice.read_game(),
        a_notice_line(b"[LS;477;Wolves;MyGuild]"),
        "Alice hears the declaration"
    );
    assert_eq!(
        yankee.read_game(),
        a_notice_line(b"[LS;477;Wolves;MyGuild]"),
        "Yankee hears the declaration"
    );
}

/// Read one war end's records for `client`: the over notice `over`, then the END fan
/// naming `(self_id, opp_id)` — the field byte and `GUILD_WAR_END` — and the every-PC
/// `WAR_END_LIST` naming the sorted pair. A TALKING echo would be read here first and
/// fail the notice, which is how the cancel's quiet is asserted.
fn read_war_end(client: &mut Keyed, self_id: u32, opp_id: u32, over: &[u8]) {
    assert_eq!(client.read_game(), a_notice_line(over), "the over notice");
    let end = client.read_game();
    assert_eq!(
        guild_subheader(&end),
        GUILD_SUBHEADER_GC_WAR,
        "the END fan: {end:02x?}"
    );
    assert_eq!(&end[4..8], &self_id.to_le_bytes(), "the END self id");
    assert_eq!(&end[8..12], &opp_id.to_le_bytes(), "the END opp id");
    assert_eq!((end[12], end[13]), (0, 7), "field, END");
    let list = client.read_game();
    assert_eq!(
        guild_subheader(&list),
        GUILD_SUBHEADER_GC_WAR_END_LIST,
        "the END_LIST: {list:02x?}"
    );
    assert_eq!(
        &list[4..12],
        &[1, 0, 0, 0, 2, 0, 0, 0],
        "the list names the sorted pair"
    );
}

/// Assert both fixture guilds' war columns: one draw tallied each and the ladders the
/// seed left (1000), with no wins and no losses.
fn assert_guild_tallies(database: &ScratchDatabase, note: &str) {
    let tallies = support::rows(
        database.url(),
        "SELECT win, draw, loss, ladder_point FROM guild ORDER BY id",
        4,
    );
    assert_eq!(
        tallies,
        vec![vec![0, 1, 0, 1000], vec![0, 1, 0, 1000]],
        "{note}"
    );
}

/// Read `/gwlist`'s fixed line and one pair line, asserting the pair's text with a
/// numeric age: `MyGuild[1] vs Wolves[2] time <n> sec.`. Returns the age.
fn read_gwlist_pair(client: &mut Keyed) -> u32 {
    assert_eq!(
        client.read_game(),
        a_notice_line(b"This is a guild that is currently at war."),
        "the gwlist header"
    );
    let line = client.read_game();
    assert_eq!(line[0], GC_CHAT, "the pair line: {line:02x?}");
    assert_eq!(line[3], 2, "CHAT_TYPE_NOTICE");
    let text = &line[10..];
    let prefix = b"MyGuild[1] vs Wolves[2] time ";
    let suffix = b" sec.";
    assert!(
        text.starts_with(prefix) && text.ends_with(suffix),
        "the pair line: {:?}",
        String::from_utf8_lossy(text)
    );
    std::str::from_utf8(&text[prefix.len()..text.len() - suffix.len()])
        .expect("the age is ASCII")
        .parse()
        .expect("the age is a number")
}

/// `sys.guild.war`'s end: a GM-granted master's `/gwstop` ends the pair in a forced
/// draw — the TALKING echo, the draw over notice (`G/guild_manager.cpp:696-699`), both
/// END fans, the every-PC `WAR_END_LIST`, and the `draw` tallies in the store
/// (`db/GuildManager.cpp:404-524`). A second war's `/gwcancel` ends quiet, with both
/// masters' `[LS;482]` and the every-PC `[LS;483]`, and touches no ladder or tally
/// (`CancelWar`, `G/guild_manager.cpp:739-770`). The GM's `/gwlist` names the live pair
/// and its age (`ShowGuildWarList`, `G/guild_manager.cpp:785-800`). A non-GM typing
/// `/gwstop` is told the command does not exist (`G/cmd.cpp:610-699`), and the war
/// stays live.
#[test]
fn a_gm_stops_and_cancels_the_war_and_a_player_cannot() {
    let Some((database, _server, mut alice, mut yankee)) = a_war_pair_ready() else {
        return;
    };
    declare_and_accept_war(&mut alice, &mut yankee);
    // The GM's `/gwlist`: the fixed line, then the pair with its age. The war just
    // started, so the age is a small number of seconds.
    alice.send_record(&client_chat(CHAT_TALKING, b"/gwlist"));
    let age = read_gwlist_pair(&mut alice);
    assert!(age <= 5, "the war just started: {age} sec");
    yankee.quiet("the gwlist reaches only the GM");
    // A player's line is refused before the world is asked: Yankee hears the
    // no-such-command line, and Alice hears nothing, so the war is still live.
    yankee.send_record(&client_chat(CHAT_TALKING, b"/gwstop 1 2"));
    assert_eq!(
        yankee.read_game(),
        an_info_line(prodomo::command::NO_SUCH_COMMAND.as_bytes()),
        "a player cannot stop a war"
    );
    alice.quiet("the player's line reaches no one else");
    yankee.quiet("the player's line echoes nothing");
    // The GM's stop: the TALKING echo sorts the pair, the scores (0-0) draw the over
    // notice, then the pair's END fans and the every-PC END_LIST land.
    alice.send_record(&client_chat(CHAT_TALKING, b"/gwstop 1 2"));
    let echo = alice.read_game();
    assert_eq!(echo[0], GC_CHAT, "the echo: {echo:02x?}");
    assert_eq!(echo[3], 0, "CHAT_TYPE_TALKING");
    assert_eq!(&echo[10..], b"1 2", "the echo sorts the pair");
    read_war_end(&mut alice, 1, 2, b"[LS;479;MyGuild;Wolves]");
    read_war_end(&mut yankee, 2, 1, b"[LS;479;MyGuild;Wolves]");
    alice.quiet("the stop settled");
    yankee.quiet("the stop settled");
    // The forced draw's rows: both draws tallied, both ladders unchanged.
    wait_for(&database, "(SELECT count(*) FROM guild WHERE draw = 1) = 2");
    assert_guild_tallies(&database, "the forced draw tallies both draws");
    // A second war: declare and accept again, then the GM's `/gwcancel`.
    declare_and_accept_war(&mut alice, &mut yankee);
    // The cancel: no echo, then the draw notice, the END fans, the END_LIST, both
    // masters' `[LS;482]` and the every-PC `[LS;483]`.
    alice.send_record(&client_chat(CHAT_TALKING, b"/gwcancel 1 2"));
    read_war_end(&mut alice, 1, 2, b"[LS;479;MyGuild;Wolves]");
    assert_eq!(
        alice.read_game(),
        an_info_line(b"[LS;482]"),
        "Alice hears [LS;482]"
    );
    assert_eq!(
        alice.read_game(),
        a_notice_line(b"[LS;483;MyGuild;Wolves]"),
        "Alice hears the every-PC [LS;483]"
    );
    read_war_end(&mut yankee, 2, 1, b"[LS;479;MyGuild;Wolves]");
    assert_eq!(
        yankee.read_game(),
        an_info_line(b"[LS;482]"),
        "Yankee hears [LS;482]"
    );
    assert_eq!(
        yankee.read_game(),
        a_notice_line(b"[LS;483;MyGuild;Wolves]"),
        "Yankee hears the every-PC [LS;483]"
    );
    alice.quiet("the cancel settled");
    yankee.quiet("the cancel settled");
    // The cancel skips the math: the stop's draw tallies stay the only ones.
    assert_guild_tallies(&database, "the cancel writes no ladder and no tally");
}

/// The next record of a player's fight, read within `deadline`: a record that never comes
/// fails the scenario instead of hanging it.
fn read_fight_before(client: &mut Keyed, deadline: std::time::Instant, note: &str) -> Vec<u8> {
    client
        .read_fight_until(deadline)
        .unwrap_or_else(|| panic!("{note} never came"))
}

/// The war's score record (`GUILD_SUBHEADER_GC_WAR_SCORE`), read past the other records
/// the connection sees.
fn read_war_score(client: &mut Keyed, deadline: std::time::Instant) -> Vec<u8> {
    while let Some(record) = client.read_fight_until(deadline) {
        if record[0] == GC_GUILD && guild_subheader(&record) == GUILD_SUBHEADER_GC_WAR_SCORE {
            return record;
        }
    }
    panic!("the war's score never came")
}

/// Ledger 233: a guild war's kill scores the war (`CGuildManager::Kill`,
/// `G/char_battle.cpp:1310-1311`). Two guilds under a war, both masters in the world: the
/// war arm of `battle_is_attackable` (`G/battle.cpp:125-138`) lets the two fight whatever
/// the PK policy says - they are of one empire, and the policy would refuse - and the
/// kill scores the war with the victim's level. The score reaches the gainer's members
/// and the opp's (`SetWarScoreAgainstTo`'s field fan, `G/guild_war.cpp:198-218`).
#[test]
fn a_guild_wars_kill_scores_the_war() {
    let Some((_database, _server, mut alice, alpha_listed, mut yankee, yankee_listed)) =
        a_war_pair_ready_at(Some(1))
    else {
        return;
    };
    declare_and_accept_war(&mut alice, &mut yankee);
    alice.quiet("the war started");
    yankee.quiet("the war started");
    // When: Yankee hits Alpha, who stands with one hit point. Then: the damage, the stun
    // and the claim.
    yankee.send_record(&client_attack(alpha_listed.id));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let damage = read_fight_before(&mut yankee, deadline, "the damage");
    assert_eq!(damage[0], GC_DAMAGE_INFO, "the damage: {damage:02x?}");
    assert_eq!(
        &damage[1..5],
        &alpha_listed.id.to_le_bytes(),
        "the victim's vid"
    );
    assert_eq!(damage[5], DAMAGE_NORMAL, "the damage's flag");
    assert!(
        i32::from_le_bytes(damage[6..10].try_into().expect("the damage")) > 0,
        "the damage lands: {damage:02x?}"
    );
    assert_eq!(
        read_fight_before(&mut yankee, deadline, "the stun"),
        stun_of(alpha_listed.id)
    );
    assert_eq!(
        read_fight_before(&mut yankee, deadline, "the claim"),
        ownership_of(yankee_listed.id, alpha_listed.id)
    );
    // Then, three seconds on: the war's score with the victim's level, and the death.
    let score = read_fight_before(&mut yankee, deadline, "the war's score");
    assert_eq!(score[0], GC_GUILD, "the war's score: {score:02x?}");
    assert_eq!(guild_subheader(&score), GUILD_SUBHEADER_GC_WAR_SCORE);
    assert_eq!(&score[4..8], &2u32.to_le_bytes(), "the gainer's guild");
    assert_eq!(&score[8..12], &1u32.to_le_bytes(), "the opp's guild");
    assert_eq!(
        &score[12..16],
        &154u32.to_le_bytes(),
        "the victim's level scores the war"
    );
    // Then the claim's release (`ClearSync`, `G/char_battle.cpp:1461`) and the death.
    assert_eq!(
        read_fight_before(&mut yankee, deadline, "the claim's release"),
        ownership_of(0, alpha_listed.id)
    );
    assert_eq!(
        read_fight_before(&mut yankee, deadline, "the death"),
        dead_of(alpha_listed.id)
    );
    // The other side's master hears the same score.
    let score = read_war_score(&mut alice, deadline);
    assert_eq!(&score[4..8], &2u32.to_le_bytes(), "the gainer's guild");
    assert_eq!(&score[8..12], &1u32.to_le_bytes(), "the opp's guild");
    assert_eq!(
        &score[12..16],
        &154u32.to_le_bytes(),
        "the victim's level scores the war"
    );
}

/// Ledger 234: the alignment a character was stored with reaches the client.
///
/// The legacy keeps the alignment in the character row (`player.alignment`,
/// `G/db/model/player.cpp:26`), `SetPlayerProto` puts it in the character
/// (`G/char.cpp:2244-2245`), and the additional info shows it in tenths
/// (`m_iAlignment / 10`, `G/char.cpp:1201`). Before this slice the column did not exist,
/// so the client was shown a zero alignment whatever the row said.
#[test]
fn the_stored_alignment_reaches_the_client() {
    let Some(database) = ScratchDatabase::create() else {
        return;
    };
    let server = Server::start(binary(), database.url());
    create_account(&server, "alice");
    add_characters(&database);
    sql(
        &database,
        "UPDATE player SET alignment = 4321 WHERE name = 'Alpha'",
    );
    let (_alice, _alpha, entered) = enter_world_seeing(&server, b"alice", 0);
    assert_eq!(
        &entered.info[55..57],
        &432i16.to_le_bytes(),
        "sAlignment: the stored alignment in tenths"
    );
}
