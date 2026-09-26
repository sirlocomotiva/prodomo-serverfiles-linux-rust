//! Metin2 networking layer
//!
//! Async TCP networking utilities for game server connections.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

#[cfg(feature = "tokio")]
pub mod client_transport;

pub use protocol;
