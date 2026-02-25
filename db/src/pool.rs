//! Connection pool management for `MySQL` databases.
//!
//! Provides connection pooling for three database types:
//! - Player database: stores player character data
//! - Account database: stores account authentication data
//! - Common database: stores shared game data

use futures_util::TryStreamExt;
use sqlx::mysql::{MySqlPool, MySqlPoolOptions, MySqlRow};
use sqlx::{MySql, Transaction};
use std::time::Duration;
use tracing::{error, info, warn};

/// Database connection pool wrapper with retry logic.
#[derive(Debug, Clone)]
pub struct ConnectionPool {
    /// The underlying `SQLx` connection pool.
    pool: MySqlPool,
    /// Maximum number of connection retry attempts.
    max_retries: u32,
    /// Delay between retry attempts.
    retry_delay: Duration,
}

/// Configuration for database connection pools.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Database connection URL.
    pub url: String,
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Minimum number of idle connections.
    pub min_connections: u32,
    /// Connection timeout duration.
    pub connect_timeout: Duration,
    /// Idle timeout before closing a connection.
    pub idle_timeout: Option<Duration>,
    /// Maximum lifetime of a connection.
    pub max_lifetime: Option<Duration>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            max_connections: 10,
            min_connections: 1,
            connect_timeout: Duration::from_secs(30),
            idle_timeout: Some(Duration::from_secs(600)),
            max_lifetime: Some(Duration::from_secs(1800)),
        }
    }
}

/// Errors that can occur during database operations.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// `SQLx` database error.
    #[error("Database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// Retry count must be greater than zero.
    #[error("Maximum retry attempts must be greater than zero, got {max_retries}")]
    InvalidRetryCount {
        /// The invalid maximum retry count.
        max_retries: u32,
    },
    /// Connection pool creation failed after retries.
    #[error("Failed to create connection pool after {attempts} attempts: {source}")]
    PoolCreationFailed {
        /// Number of retry attempts made.
        attempts: u32,
        /// The underlying error.
        source: sqlx::Error,
    },
    /// Query execution failed after retries.
    #[error("Query failed after {attempts} attempts: {source}")]
    QueryFailed {
        /// Number of retry attempts made.
        attempts: u32,
        /// The underlying error.
        source: sqlx::Error,
    },
}

/// Result type for database operations.
pub type DbResult<T> = Result<T, DbError>;

impl ConnectionPool {
    /// Creates a new connection pool with retry logic.
    ///
    /// # Errors
    ///
    /// Returns `DbError::PoolCreationFailed` if the pool cannot be created
    /// after the specified number of retry attempts.
    pub async fn new(config: &DatabaseConfig) -> DbResult<Self> {
        Self::with_retry(config, 3, Duration::from_secs(2)).await
    }

    /// Creates a pool that defers its first connection to the first query.
    ///
    /// Unlike [`Self::new`] and [`Self::with_retry`], this performs no network
    /// I/O. The URL is parsed and stored, so a malformed URL is still reported
    /// here, but no TCP connection is attempted until a query runs. This is the
    /// seam a server needs when its tables are already in memory and a missing
    /// database must not stop the listener from coming up.
    ///
    /// A pool built this way is not a readiness signal. The first query still
    /// fails if no server is listening, and it fails then, not at startup.
    ///
    /// # Errors
    ///
    /// Returns `DbError::PoolCreationFailed` if the URL cannot be parsed.
    pub fn lazy(config: &DatabaseConfig) -> DbResult<Self> {
        let pool = MySqlPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.connect_timeout)
            .idle_timeout(config.idle_timeout)
            .max_lifetime(config.max_lifetime)
            .connect_lazy(&config.url)
            .map_err(|source| DbError::PoolCreationFailed {
                attempts: 0,
                source,
            })?;
        Ok(Self {
            pool,
            max_retries: 1,
            retry_delay: Duration::ZERO,
        })
    }

    /// Creates a new connection pool with custom retry settings.
    ///
    /// # Errors
    ///
    /// Returns `DbError::InvalidRetryCount` if `max_retries` is zero, or
    /// `DbError::PoolCreationFailed` if the pool cannot be created after the
    /// specified number of retry attempts.
    pub async fn with_retry(
        config: &DatabaseConfig,
        max_retries: u32,
        retry_delay: Duration,
    ) -> DbResult<Self> {
        if max_retries == 0 {
            return Err(DbError::InvalidRetryCount { max_retries });
        }

        let mut attempt = 1;
        loop {
            match Self::create_pool(config).await {
                Ok(pool) => {
                    info!(
                        "Successfully created connection pool (attempt {}/{})",
                        attempt, max_retries
                    );
                    return Ok(Self {
                        pool,
                        max_retries,
                        retry_delay,
                    });
                }
                Err(error) => {
                    warn!(
                        "Failed to create connection pool (attempt {}/{}): {}",
                        attempt, max_retries, error
                    );
                    if attempt == max_retries {
                        return Err(DbError::PoolCreationFailed {
                            attempts: attempt,
                            source: error,
                        });
                    }
                    tokio::time::sleep(retry_delay).await;
                    attempt += 1;
                }
            }
        }
    }

    /// Internal method to create the pool from configuration.
    async fn create_pool(config: &DatabaseConfig) -> Result<MySqlPool, sqlx::Error> {
        MySqlPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.connect_timeout)
            .idle_timeout(config.idle_timeout)
            .max_lifetime(config.max_lifetime)
            .connect(&config.url)
            .await
    }

    /// Executes a query that returns multiple rows.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the query fails after retries.
    pub async fn query(&self, sql: &str) -> DbResult<Vec<MySqlRow>> {
        self.execute_with_retry(|| async { sqlx::query(sql).fetch_all(&self.pool).await })
            .await
    }

    /// Executes a read query while retaining at most `max_rows` rows.
    ///
    /// Rows are fetched with [`sqlx::raw_sql`], which uses the `MySQL` text
    /// protocol (`COM_QUERY`) for the acquisition projection. The plain
    /// [`sqlx::query()`] path has an empty argument set and uses the binary
    /// prepared-statement protocol (`COM_STMT_EXECUTE`), which is not the
    /// projection required by this boundary.
    ///
    /// `Some(rows)` means the query completed with no more than `max_rows`
    /// rows. `None` means that a row beyond the cap was observed; the extra
    /// row is discarded and the stream is stopped. An empty result is
    /// `Some(Vec::new())`. Query and connection failures retain the existing
    /// retry/error behavior and are never represented as an empty result.
    ///
    /// The stream is retried from the beginning after a retryable `SQLx` error;
    /// partial rows from the failed attempt are discarded.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::InvalidRetryCount`] when the pool has retries
    /// disabled, [`DbError::Sqlx`] for a non-retryable `SQLx` error, or
    /// [`DbError::QueryFailed`] when retries are exhausted.
    pub async fn query_up_to(&self, sql: &str, max_rows: usize) -> DbResult<Option<Vec<MySqlRow>>> {
        self.execute_with_retry(|| async {
            let mut stream = sqlx::raw_sql(sql).fetch(&self.pool);
            let mut rows = Vec::new();
            while let Some(row) = stream.try_next().await? {
                if !query_row_allowed(rows.len(), max_rows) {
                    return Ok(None);
                }
                rows.push(row);
            }
            Ok(Some(rows))
        })
        .await
    }

    /// Executes a query with a single string parameter that returns multiple rows.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the query fails after retries.
    pub async fn query_with_str(&self, sql: &str, param: &str) -> DbResult<Vec<MySqlRow>> {
        self.execute_with_retry(|| async {
            sqlx::query(sql).bind(param).fetch_all(&self.pool).await
        })
        .await
    }

    /// Executes a query with a single i64 parameter that returns multiple rows.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the query fails after retries.
    pub async fn query_with_i64(&self, sql: &str, param: i64) -> DbResult<Vec<MySqlRow>> {
        self.execute_with_retry(|| async {
            sqlx::query(sql).bind(param).fetch_all(&self.pool).await
        })
        .await
    }

    /// Executes a query that returns a single optional row.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the query fails after retries.
    pub async fn fetch_optional(&self, sql: &str) -> DbResult<Option<MySqlRow>> {
        self.execute_with_retry(|| async { sqlx::query(sql).fetch_optional(&self.pool).await })
            .await
    }

    /// Executes a query with a single string parameter that returns an optional row.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the query fails after retries.
    pub async fn fetch_optional_with_str(
        &self,
        sql: &str,
        param: &str,
    ) -> DbResult<Option<MySqlRow>> {
        self.execute_with_retry(|| async {
            sqlx::query(sql)
                .bind(param)
                .fetch_optional(&self.pool)
                .await
        })
        .await
    }

    /// Executes a query with a single i64 parameter that returns an optional row.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the query fails after retries.
    pub async fn fetch_optional_with_i64(
        &self,
        sql: &str,
        param: i64,
    ) -> DbResult<Option<MySqlRow>> {
        self.execute_with_retry(|| async {
            sqlx::query(sql)
                .bind(param)
                .fetch_optional(&self.pool)
                .await
        })
        .await
    }

    /// Executes a query that returns a single row.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the query fails or returns no rows.
    pub async fn fetch_one(&self, sql: &str) -> DbResult<MySqlRow> {
        self.execute_with_retry(|| async { sqlx::query(sql).fetch_one(&self.pool).await })
            .await
    }

    /// Executes a query with a single string parameter that returns a single row.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the query fails or returns no rows.
    pub async fn fetch_one_with_str(&self, sql: &str, param: &str) -> DbResult<MySqlRow> {
        self.execute_with_retry(|| async {
            sqlx::query(sql).bind(param).fetch_one(&self.pool).await
        })
        .await
    }

    /// Executes a query with a single i64 parameter that returns a single row.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the query fails or returns no rows.
    pub async fn fetch_one_with_i64(&self, sql: &str, param: i64) -> DbResult<MySqlRow> {
        self.execute_with_retry(|| async {
            sqlx::query(sql).bind(param).fetch_one(&self.pool).await
        })
        .await
    }

    /// Executes a statement that does not return rows (INSERT, UPDATE, DELETE).
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the statement fails after retries.
    pub async fn execute(&self, sql: &str) -> DbResult<u64> {
        self.execute_with_retry(|| async { sqlx::query(sql).execute(&self.pool).await })
            .await
            .map(|result| result.rows_affected())
    }

    /// Executes a statement with a single string parameter.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the statement fails after retries.
    pub async fn execute_with_str(&self, sql: &str, param: &str) -> DbResult<u64> {
        self.execute_with_retry(|| async { sqlx::query(sql).bind(param).execute(&self.pool).await })
            .await
            .map(|result| result.rows_affected())
    }

    /// Executes a statement with a single i64 parameter.
    ///
    /// # Errors
    ///
    /// Returns `DbError::QueryFailed` if the statement fails after retries.
    pub async fn execute_with_i64(&self, sql: &str, param: i64) -> DbResult<u64> {
        self.execute_with_retry(|| async { sqlx::query(sql).bind(param).execute(&self.pool).await })
            .await
            .map(|result| result.rows_affected())
    }

    /// Begins a new database transaction.
    ///
    /// # Errors
    ///
    /// Returns `DbError::Sqlx` if the transaction cannot be started.
    pub async fn begin(&self) -> DbResult<Transaction<'_, MySql>> {
        self.pool.begin().await.map_err(DbError::Sqlx)
    }

    /// Returns a reference to the underlying pool.
    #[must_use]
    pub fn inner(&self) -> &MySqlPool {
        &self.pool
    }

    /// Returns the number of idle connections in the pool.
    #[must_use]
    pub fn idle_connections(&self) -> usize {
        self.pool.num_idle()
    }

    /// Returns the total size of the pool.
    #[must_use]
    pub fn size(&self) -> u32 {
        self.pool.size()
    }

    /// Internal retry wrapper for database operations.
    async fn execute_with_retry<F, Fut, T>(&self, operation: F) -> DbResult<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, sqlx::Error>>,
    {
        if self.max_retries == 0 {
            return Err(DbError::InvalidRetryCount {
                max_retries: self.max_retries,
            });
        }

        let mut attempt = 1;
        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(error) => {
                    if !Self::is_retryable_error(&error) {
                        error!("Non-retryable database error: {}", error);
                        return Err(DbError::Sqlx(error));
                    }

                    warn!(
                        "Database operation failed (attempt {}/{}): {}",
                        attempt, self.max_retries, error
                    );
                    if attempt == self.max_retries {
                        return Err(DbError::QueryFailed {
                            attempts: attempt,
                            source: error,
                        });
                    }
                    tokio::time::sleep(self.retry_delay).await;
                    attempt += 1;
                }
            }
        }
    }

    /// Determines if an error is retryable.
    fn is_retryable_error(error: &sqlx::Error) -> bool {
        match error {
            sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => true,
            sqlx::Error::Database(db_err) => {
                let code = db_err.code().unwrap_or_default();
                matches!(
                    code.parse::<i32>().unwrap_or(0),
                    1040 | 1053 | 1205 | 1213 | 2006 | 2013
                )
            }
            _ => false,
        }
    }
}

fn query_row_allowed(current: usize, maximum: usize) -> bool {
    current < maximum
}

/// Manages connection pools for all three database types.
#[derive(Debug, Clone)]
pub struct DatabaseManager {
    /// Player database connection pool.
    pub player: ConnectionPool,
    /// Account database connection pool.
    pub account: ConnectionPool,
    /// Common database connection pool.
    pub common: ConnectionPool,
}

impl DatabaseManager {
    /// Creates a new `DatabaseManager` with connection pools for all databases.
    ///
    /// # Errors
    ///
    /// Returns `DbError::PoolCreationFailed` if any pool cannot be created.
    pub async fn new(
        player_config: &DatabaseConfig,
        account_config: &DatabaseConfig,
        common_config: &DatabaseConfig,
    ) -> DbResult<Self> {
        info!("Initializing database connections...");

        let player = ConnectionPool::new(player_config).await?;
        let account = ConnectionPool::new(account_config).await?;
        let common = ConnectionPool::new(common_config).await?;

        info!("All database connections established successfully");

        Ok(Self {
            player,
            account,
            common,
        })
    }

    /// Creates a new `DatabaseManager` with custom retry settings.
    ///
    /// # Errors
    ///
    /// Returns `DbError::InvalidRetryCount` if `max_retries` is zero, or
    /// `DbError::PoolCreationFailed` if any pool cannot be created.
    pub async fn with_retry(
        player_config: &DatabaseConfig,
        account_config: &DatabaseConfig,
        common_config: &DatabaseConfig,
        max_retries: u32,
        retry_delay: Duration,
    ) -> DbResult<Self> {
        info!("Initializing database connections with retry settings...");

        let player = ConnectionPool::with_retry(player_config, max_retries, retry_delay).await?;
        let account = ConnectionPool::with_retry(account_config, max_retries, retry_delay).await?;
        let common = ConnectionPool::with_retry(common_config, max_retries, retry_delay).await?;

        info!("All database connections established successfully");

        Ok(Self {
            player,
            account,
            common,
        })
    }

    /// Closes all database connection pools.
    pub async fn close(&self) {
        info!("Closing all database connections...");
        self.player.inner().close().await;
        self.account.inner().close().await;
        self.common.inner().close().await;
        info!("All database connections closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn test_query_up_to_cap_helper_is_inclusive() {
        assert!(!query_row_allowed(0, 0));
        assert!(query_row_allowed(0, 1));
        assert!(!query_row_allowed(1, 1));
        assert!(!query_row_allowed(usize::MAX, usize::MAX));
    }

    #[tokio::test]
    async fn test_with_retry_rejects_zero_retries() {
        let result =
            ConnectionPool::with_retry(&DatabaseConfig::default(), 0, Duration::ZERO).await;

        assert!(matches!(
            result,
            Err(DbError::InvalidRetryCount { max_retries: 0 })
        ));
    }

    #[tokio::test]
    async fn test_execute_with_retry_rejects_zero_retries_without_running_operation() -> DbResult<()>
    {
        let pool = MySqlPoolOptions::new().connect_lazy("mysql://localhost/test")?;
        let connection_pool = ConnectionPool {
            pool,
            max_retries: 0,
            retry_delay: Duration::ZERO,
        };
        let operation_called = Cell::new(false);

        let result = connection_pool
            .execute_with_retry(|| {
                operation_called.set(true);
                async { Err::<(), sqlx::Error>(sqlx::Error::PoolClosed) }
            })
            .await;

        assert!(matches!(
            result,
            Err(DbError::InvalidRetryCount { max_retries: 0 })
        ));
        assert!(!operation_called.get());
        Ok(())
    }

    #[test]
    fn test_database_config_default() {
        let config = DatabaseConfig::default();
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.min_connections, 1);
        assert_eq!(config.connect_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_is_retryable_error() {
        let err = sqlx::Error::RowNotFound;
        assert!(!ConnectionPool::is_retryable_error(&err));

        let err = sqlx::Error::PoolTimedOut;
        assert!(ConnectionPool::is_retryable_error(&err));
    }
}
