//! Metin2 shared types and utilities
//!
//! Common data structures and helper functions used across
//! the `prodomo` server and its libraries.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod config;

/// Logging infrastructure using tracing
pub mod logging;

/// Game constants ported from C++ length.h and item_length.h
pub mod constants;

/// Game enums ported from C++ length.h and item_length.h
pub mod enums;

/// Feature flags documentation ported from C++ prodomodefines.h
pub mod features;

/// Virtual ID (VID) system for entity identification
pub mod vid;

/// Data structures ported from C++ tables.h
pub mod tables;

/// GM host and administrator rules ported from the legacy boot tail
pub mod gm;

/// The compiled-in level and job tables of the legacy `constants.cpp`
pub mod levels;

/// The `EPointTypes` point-slot indices of the legacy `char.h`
pub mod point_slot;

/// The `EWindows` window byte and the flat item slot space of `length.h`
pub mod item_slots;

pub use tracing::{debug, error, info, trace, warn};

/// Entity identifier type used throughout the server
pub type EntityId = u32;

/// Account identifier type
pub type AccountId = u32;

/// Character identifier type
pub type CharacterId = u32;

/// Item identifier type
pub type ItemId = u64;
