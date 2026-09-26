//! Parity scenarios: the scripted client plays the real `prodomo` binary over TCP.
//!
//! Each scenario is a `#[test]` whose name an inventory row in `.scratch/parity/` gives in its
//! `scenario` column; a row is `ported` only when its scenario is here and passes.
//! `inventory_rows_keep_the_rules` checks that link. The scenarios need a store, so they run only
//! when `DATABASE_URL` is set; the inventory check always runs.

#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use parity::client::Quiet;
use parity::inventory::{self, Status};
use parity::{Client, Server};
use protocol::cg_inventory::{resolve_cg_base_size, CG_KEEP_ALIVE, HEADER_CG_PONG};
use support::ScratchDatabase;

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
