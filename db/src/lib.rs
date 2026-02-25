//! Metin2 database layer
//!
//! Database access and caching utilities using `SQLx` with `MySQL` driver.
//! Provides connection pool management for player, account, and common databases.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub use common;

pub mod pool;

pub use pool::{ConnectionPool, DatabaseConfig, DatabaseManager, DbError, DbResult};

/// Re-export sqlx types for convenience.
pub use sqlx;
