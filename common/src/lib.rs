//! Metin2 shared types and utilities
//!
//! Common data structures and helper functions used across
//! the game server, db server, and related services.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

/// Configuration file parser
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

pub use tracing::{debug, error, info, trace, warn};

/// Entity identifier type used throughout the server
pub type EntityId = u32;

/// Account identifier type
pub type AccountId = u32;

/// Character identifier type
pub type CharacterId = u32;

/// Item identifier type
pub type ItemId = u64;
