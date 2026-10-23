//! The PostgreSQL 18 store for the Prodomo server (ADR-0003).
//!
//! Tests that need a server run only when `DATABASE_URL` is set.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod accounts;
pub mod credentials;
pub mod guilds;
pub mod item_id_range;
pub mod items;
pub mod messenger;
pub mod player_affects;
pub mod player_skills;
pub mod players;
pub mod quest_flags;
pub mod quickslots;
pub mod safebox;
pub mod store;

pub use store::{Store, StoreConfig, StoreError};

/// Re-export of `sqlx` so callers use the same version.
pub use sqlx;
