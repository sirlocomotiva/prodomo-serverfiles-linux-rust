//! The PostgreSQL 18 store for the Prodomo server (ADR-0003).
//!
//! Tests that need a server run only when `DATABASE_URL` is set.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod accounts;
pub mod credentials;
pub mod item_id_range;
pub mod items;
pub mod players;
pub mod store;

pub use store::{Store, StoreConfig, StoreError};

/// Re-export of `sqlx` so callers use the same version.
pub use sqlx;
