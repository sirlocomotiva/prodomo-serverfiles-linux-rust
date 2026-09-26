//! The PostgreSQL connection pool.

use std::error::Error;
use std::fmt;
use std::str::FromStr;

use common::config::{redact_url, StoreSettings};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

/// Settings for the PostgreSQL connection pool.
///
/// `Debug` prints the URL without its password.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreConfig {
    /// A `postgres://` connection URL.
    pub url: String,
    /// Upper bound on open connections. Must be at least one.
    pub max_connections: u32,
}

impl StoreConfig {
    /// Default upper bound on open connections.
    pub const DEFAULT_MAX_CONNECTIONS: u32 = 8;

    /// A configuration for `url` with the default pool size.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: Self::DEFAULT_MAX_CONNECTIONS,
        }
    }

    fn pool_options(&self) -> Result<(PgPoolOptions, PgConnectOptions), StoreError> {
        if self.max_connections == 0 {
            return Err(StoreError::ZeroConnections);
        }
        // `PgConnectOptions` ignores the scheme, so a `mysql://` URL left over
        // from the legacy configuration would otherwise be accepted.
        let scheme = self.url.split_once("://").map_or("", |(scheme, _)| scheme);
        if !matches!(scheme, "postgres" | "postgresql") {
            return Err(StoreError::UnsupportedScheme(scheme.to_owned()));
        }
        let connect = PgConnectOptions::from_str(&self.url).map_err(StoreError::InvalidUrl)?;
        Ok((
            PgPoolOptions::new().max_connections(self.max_connections),
            connect,
        ))
    }
}

impl fmt::Debug for StoreConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreConfig")
            .field("url", &redact_url(&self.url))
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

impl From<&StoreSettings> for StoreConfig {
    fn from(settings: &StoreSettings) -> Self {
        Self {
            url: settings.url.clone(),
            max_connections: settings.max_connections,
        }
    }
}

/// An error opening the store.
#[derive(Debug)]
pub enum StoreError {
    /// The URL could not be parsed. No connection was attempted.
    InvalidUrl(sqlx::Error),
    /// The URL scheme is not `postgres` or `postgresql`. Only the scheme is
    /// kept, so the error never carries a password.
    UnsupportedScheme(String),
    /// `max_connections` was zero.
    ZeroConnections,
    /// The server refused or failed a connection or query.
    Database(sqlx::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(error) => write!(f, "invalid PostgreSQL URL: {error}"),
            Self::UnsupportedScheme(scheme) => {
                write!(f, "unsupported database URL scheme {scheme:?}; expected postgres://")
            }
            Self::ZeroConnections => f.write_str("max_connections must be at least one"),
            Self::Database(error) => write!(f, "PostgreSQL error: {error}"),
        }
    }
}

impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidUrl(error) | Self::Database(error) => Some(error),
            Self::UnsupportedScheme(_) | Self::ZeroConnections => None,
        }
    }
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// A shared PostgreSQL connection pool.
#[derive(Debug, Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Build a pool without connecting.
    ///
    /// The URL is parsed now, so a malformed URL fails at startup, but no
    /// connection is opened until the first query. A lazy pool is not a
    /// readiness signal.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UnsupportedScheme`], [`StoreError::InvalidUrl`],
    /// or [`StoreError::ZeroConnections`].
    pub fn lazy(config: &StoreConfig) -> Result<Self, StoreError> {
        let (pool, connect) = config.pool_options()?;
        Ok(Self {
            pool: pool.connect_lazy_with(connect),
        })
    }

    /// Build a pool and open its first connection.
    ///
    /// # Errors
    ///
    /// Returns a configuration error, or [`StoreError::Database`] if the
    /// server cannot be reached.
    pub async fn connect(config: &StoreConfig) -> Result<Self, StoreError> {
        let (pool, connect) = config.pool_options()?;
        Ok(Self {
            pool: pool.connect_with(connect).await?,
        })
    }

    /// The underlying pool.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Close every connection and refuse new ones.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_lazy_store_parses_the_url_without_connecting() {
        let config = StoreConfig::new("postgres://prodomo@127.0.0.1:1/prodomo");
        let store = Store::lazy(&config).expect("a well-formed URL needs no server");
        assert_eq!(store.pool().size(), 0);
    }

    #[tokio::test]
    async fn a_mysql_url_is_refused_before_any_connection() {
        let config = StoreConfig::new("mysql://prodomo:secret@127.0.0.1/player");
        let error = Store::lazy(&config).expect_err("only PostgreSQL is supported");
        assert!(
            matches!(&error, StoreError::UnsupportedScheme(scheme) if scheme == "mysql"),
            "got {error:?}"
        );
        assert!(!error.to_string().contains("secret"), "the password never leaks");
    }

    #[tokio::test]
    async fn a_malformed_url_is_refused_before_any_connection() {
        let config = StoreConfig::new("postgres://prodomo@127.0.0.1:notaport/prodomo");
        assert!(matches!(
            Store::lazy(&config),
            Err(StoreError::InvalidUrl(_))
        ));
    }

    #[test]
    fn debug_output_never_shows_the_password() {
        let config = StoreConfig::new("postgres://prodomo:hunter2@db.local:5432/prodomo");
        let shown = format!("{config:?}");
        assert!(!shown.contains("hunter2"), "got {shown}");
        assert!(shown.contains("prodomo:***@db.local:5432/prodomo"), "got {shown}");
    }

    #[tokio::test]
    async fn a_zero_sized_pool_is_refused() {
        let mut config = StoreConfig::new("postgres://prodomo@127.0.0.1/prodomo");
        config.max_connections = 0;
        assert!(matches!(
            Store::lazy(&config),
            Err(StoreError::ZeroConnections)
        ));
    }
}
