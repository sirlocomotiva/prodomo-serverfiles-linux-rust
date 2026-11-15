//! The scripted client for Parity scenarios.
//!
//! A scenario starts the real `prodomo` binary with [`Server::start`], connects with [`Client`],
//! and plays the bytes an unmodified client would send. The scenarios themselves live in
//! `prodomo/tests/parity.rs`, because only a test of the `prodomo` package is told where Cargo
//! built the binary (`CARGO_BIN_EXE_prodomo`).
//!
//! [`scratch`] names the directories a test writes into, so that what a killed run leaves is
//! removed by the next one.
//!
//! [`inventory`] reads the Parity inventory in `.scratch/parity/` and checks the rules every row
//! must keep: a unique ID, a known status, and a scenario for every `ported` row.

pub mod client;
pub mod inventory;
pub mod scratch;
pub mod server;

pub use client::Client;
pub use server::Server;
