//! `SQLx` acquisition adapter for the source-fixed `player_index` query.
//!
//! This module executes only a [`PlayerIndexQuery`] built by the SQL-free
//! boundary. It uses the pool's bounded `query_up_to` stream with a one-row
//! cap, checks the five-column result shape, decodes all source cells as
//! optional 32-bit integers, and delegates NULL/range policy to
//! [`crate::player_index`]. Database errors, malformed rows, and an extra row
//! remain distinct from an empty (`Missing`) result. No login state, account
//! row, `player_index` repair, or player table is created here.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};

use crate::player_index::{
    decode_player_index_row, PlayerIndexLookup, PlayerIndexQuery, PlayerIndexQueryRow,
    PlayerIndexRowError, PLAYER_INDEX_QUERY_COLUMNS,
};

/// A failure while acquiring one `player_index` row through `SQLx`.
#[derive(Debug)]
pub enum PlayerIndexSqlxLoadError {
    /// The pool could not execute the checked query after its retry policy.
    Database(DbError),
    /// More than one row was returned for the account-key query.
    SourceLimitExceeded {
        /// Configured maximum number of rows.
        maximum: usize,
    },
    /// A row did not have the five-column shape promised by the query.
    RowShape {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Required column count.
        expected: usize,
        /// Actual column count.
        actual: usize,
    },
    /// A source cell could not be decoded as an optional integer.
    ColumnDecode {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Source column name.
        column: &'static str,
        /// Underlying `SQLx` decode error.
        source: db::sqlx::Error,
    },
    /// The row allocation could not reserve its bounded result.
    AllocationFailed {
        /// Number of rows returned by the bounded stream.
        requested: usize,
    },
    /// The pure row policy rejected a NULL or out-of-range value.
    Row(PlayerIndexRowError),
}

impl fmt::Display for PlayerIndexSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => {
                write!(formatter, "player_index database query failed: {source}")
            }
            Self::SourceLimitExceeded { maximum } => write!(
                formatter,
                "player_index source returned more than {maximum} row"
            ),
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "player_index row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "player_index row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "player_index could not allocate {requested} source row(s)"
            ),
            Self::Row(source) => write!(formatter, "player_index row is invalid: {source}"),
        }
    }
}

impl Error for PlayerIndexSqlxLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(source) => Some(source),
            Self::ColumnDecode { source, .. } => Some(source),
            Self::Row(source) => Some(source),
            Self::SourceLimitExceeded { .. }
            | Self::RowShape { .. }
            | Self::AllocationFailed { .. } => None,
        }
    }
}

/// Execute one checked `player_index` query and return a strict lookup result.
///
/// The query is expected to identify one account by its primary key. The pool
/// stream is capped at one row before rows are accumulated, so an unexpected
/// multi-row result is an error rather than an arbitrary first-row choice.
/// `SQLx` materializes each cell before this adapter can inspect it; schema
/// limits remain the responsibility of the database.
///
/// # Errors
///
/// Returns [`PlayerIndexSqlxLoadError::Database`] for pool/query failures,
/// [`PlayerIndexSqlxLoadError::SourceLimitExceeded`] for more than one row,
/// row-shape/decode/allocation errors for malformed SQL results, and
/// [`PlayerIndexSqlxLoadError::Row`] for NULL or empire-width violations.
pub async fn load_player_index_sqlx(
    pool: &ConnectionPool,
    query: &PlayerIndexQuery,
) -> Result<PlayerIndexLookup, PlayerIndexSqlxLoadError> {
    let rows = pool
        .query_up_to(query.as_str(), 1)
        .await
        .map_err(PlayerIndexSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(PlayerIndexSqlxLoadError::SourceLimitExceeded { maximum: 1 });
    };

    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        PlayerIndexSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;
    for (row_index, row) in rows.into_iter().enumerate() {
        let actual = row.columns().len();
        if actual != PLAYER_INDEX_QUERY_COLUMNS {
            return Err(PlayerIndexSqlxLoadError::RowShape {
                row: row_index,
                expected: PLAYER_INDEX_QUERY_COLUMNS,
                actual,
            });
        }
        let value = |column: &'static str| {
            row.try_get::<Option<u32>, _>(column).map_err(|source| {
                PlayerIndexSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                }
            })
        };
        source_rows.push(PlayerIndexQueryRow {
            pid1: value("pid1")?,
            pid2: value("pid2")?,
            pid3: value("pid3")?,
            pid4: value("pid4")?,
            empire: value("empire")?,
        });
    }

    let Some(row) = source_rows.first().copied() else {
        return Ok(PlayerIndexLookup::Missing);
    };
    decode_player_index_row(&row)
        .map(PlayerIndexLookup::Found)
        .map_err(PlayerIndexSqlxLoadError::Row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_index::PlayerIndexQuery;

    #[test]
    fn adapter_contract_keeps_the_one_row_cap() {
        assert_eq!(PLAYER_INDEX_QUERY_COLUMNS, 5);
        assert_eq!(
            PlayerIndexQuery::from_config(None, 9).unwrap().as_str(),
            "SELECT pid1, pid2, pid3, pid4, empire FROM player_index WHERE id=9"
        );
    }

    #[test]
    fn source_limit_error_is_not_an_empty_lookup() {
        let error = PlayerIndexSqlxLoadError::SourceLimitExceeded { maximum: 1 };
        assert!(error.to_string().contains("more than 1 row"));
    }
}
