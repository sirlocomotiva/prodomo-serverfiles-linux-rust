//! The quest system: legacy's quest compiler and the Lua 5.1 runtime (ADR-0004, ADR-0006).
//!
//! - [`lex`]: legacy's quest lexer, which reads the dialect every quest chunk is written in;
//! - [`qc`]: the port of `qc`, which compiles a quest source into the files of `object/`;
//! - [`dialect`]: the translation of a dialect chunk to Lua 5.1, done at load;
//! - [`sources`]: the scripts of a locale's `quest` directory and the order they load in;
//! - [`api`]: every function legacy's quest runtime registers;
//! - [`host`]: the Lua 5.1 state, its Lua 5.0 shims and every library and script loaded;
//! - [`manager`]: what an NPC click runs, the state machine of a running script and its dialog.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod api;
pub mod dialect;
pub mod host;
pub mod lex;
pub mod manager;
pub mod qc;
pub mod sources;

pub use common;
