//! Metin2 game world and map management
//!
//! World state, map loading, and entity management.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

/// Runtime character state and lifecycle management.
pub mod character;

/// Deterministic pulse-based event scheduling.
pub mod event;

/// Map identity and explicit sector topology.
pub mod map;

pub mod npc;

/// World coordinate conversion and packed sector keys.
pub mod sector;

/// Deterministic entity membership across configured map sectors.
pub mod spatial;

pub mod item;

pub use common;
pub use protocol;
