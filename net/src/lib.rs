//! Metin2 networking layer
//!
//! Async TCP networking utilities for game server connections.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod buffer;

#[cfg(feature = "tokio")]
pub mod client_transport;
#[cfg(feature = "tokio")]
pub mod db_transport;

pub use protocol;
