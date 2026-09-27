//! The PostgreSQL connection pool and the schema migrations.

use std::error::Error;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use common::config::{redact_url, StoreSettings};
use sqlx::migrate::{MigrateError, Migrator};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

/// The schema migrations in `db/migrations`, embedded when the crate is built.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// The version of the newest embedded migration.
#[must_use]
pub fn schema_version() -> i64 {
    MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or_default()
}

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

    /// How long a query waits for a connection before it fails with a pool timeout.
    ///
    /// sqlx keeps retrying a refused connection until this runs out, so it is also how long an
    /// unreachable server takes to be reported. The sqlx default of 30 seconds would hide a
    /// server that is down behind a long silence.
    pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

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
            PgPoolOptions::new()
                .max_connections(self.max_connections)
                .acquire_timeout(Self::ACQUIRE_TIMEOUT),
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
    /// The schema could not be brought up to date.
    Migrate(MigrateError),
}

impl StoreError {
    /// Whether trying again later may succeed without the Operator changing anything.
    ///
    /// Only an unreachable or overloaded server is transient: an I/O or TLS failure, a pool
    /// timeout, or a server error in SQLSTATE class `08` (connection exception), `53`
    /// (insufficient resources), or `57P` (the server is shutting down or starting up). A refused
    /// password, a missing database, or a migration that was edited after it was applied needs
    /// the Operator.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Database(error)
            | Self::Migrate(
                MigrateError::Execute(error) | MigrateError::ExecuteMigration(error, _),
            ) => is_transient(error),
            _ => false,
        }
    }
}

fn is_transient(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Io(_) | sqlx::Error::Tls(_) | sqlx::Error::PoolTimedOut => true,
        sqlx::Error::Database(error) => error.code().is_some_and(|code| {
            code.starts_with("08") || code.starts_with("53") || code.starts_with("57P")
        }),
        _ => false,
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(error) => write!(f, "invalid PostgreSQL URL: {error}"),
            Self::UnsupportedScheme(scheme) => {
                write!(
                    f,
                    "unsupported database URL scheme {scheme:?}; expected postgres://"
                )
            }
            Self::ZeroConnections => f.write_str("max_connections must be at least one"),
            Self::Database(error) => write!(f, "PostgreSQL error: {error}"),
            Self::Migrate(error) => write!(f, "schema migration failed: {error}"),
        }
    }
}

impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidUrl(error) | Self::Database(error) => Some(error),
            Self::Migrate(error) => Some(error),
            Self::UnsupportedScheme(_) | Self::ZeroConnections => None,
        }
    }
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<MigrateError> for StoreError {
    fn from(error: MigrateError) -> Self {
        Self::Migrate(error)
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

    /// Apply every embedded migration the database does not have yet.
    ///
    /// Each migration runs in its own transaction, and sqlx holds an advisory lock for the whole
    /// run, so two processes migrating at once do not interfere.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Migrate`]. [`StoreError::is_transient`] says whether a retry can
    /// help.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
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
        assert!(
            !error.to_string().contains("secret"),
            "the password never leaks"
        );
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
        assert!(
            shown.contains("prodomo:***@db.local:5432/prodomo"),
            "got {shown}"
        );
    }

    #[test]
    fn only_an_unreachable_server_is_worth_retrying() {
        let refused = || sqlx::Error::Io(std::io::ErrorKind::ConnectionRefused.into());
        assert!(StoreError::Database(refused()).is_transient());
        assert!(StoreError::Database(sqlx::Error::PoolTimedOut).is_transient());
        assert!(StoreError::Migrate(MigrateError::Execute(refused())).is_transient());
        assert!(StoreError::Migrate(MigrateError::ExecuteMigration(refused(), 1)).is_transient());

        assert!(!StoreError::Database(sqlx::Error::RowNotFound).is_transient());
        assert!(!StoreError::Migrate(MigrateError::VersionMismatch(1)).is_transient());
        assert!(!StoreError::Migrate(MigrateError::VersionMissing(2)).is_transient());
        assert!(!StoreError::Migrate(MigrateError::Dirty(1)).is_transient());
        assert!(!StoreError::ZeroConnections.is_transient());
    }

    #[test]
    fn the_embedded_migrations_start_at_one_and_have_no_gaps() {
        let versions: Vec<i64> = MIGRATOR.iter().map(|migration| migration.version).collect();
        let expected: Vec<i64> = (1..=schema_version()).collect();
        assert_eq!(versions, expected);
    }

    /// A test that waits for the server's "Store ready at schema version N" line must not
    /// spell N out. Ledger 187 added migration `0004` and left
    /// `prodomo/tests/process.rs` waiting for version 3, which would have failed the moment
    /// `DATABASE_URL` was set. The assertion now reads the same `schema_version` the log
    /// line does, and this test is what keeps the two in step.
    #[test]
    fn the_schema_version_the_server_logs_is_the_one_a_migration_added() {
        assert_eq!(
            schema_version(),
            i64::try_from(MIGRATOR.iter().count()).unwrap()
        );
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
