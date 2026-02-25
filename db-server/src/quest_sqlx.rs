//! `SQLx` acquisition adapter for the source-verified quest-load boundary.
//!
//! This adapter executes only a checked [`QuestQuery`]. It bounds acquisition
//! before converting the returned rows into the pure [`QuestQueryRow`] and
//! `protocol::db_records::QuestRecord` representations. Database failure, an
//! over-limit result,
//! malformed column shape, decode failure, and NULL/range policy failure stay
//! distinct. The adapter does not choose the cache-hit/cache-miss variant,
//! inspect or mutate player tables, or turn a failed query into an empty quest
//! list.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_records::MAX_QUEST_RECORDS;

use crate::quest::{
    decode_quest_rows, QuestLoadLimits, QuestLookup, QuestQuery, QuestQueryRow, QuestRowError,
    QUEST_QUERY_COLUMNS,
};

/// A failure while acquiring quest rows through `SQLx`.
#[derive(Debug)]
pub enum QuestSqlxLoadError {
    /// The pool could not execute the checked query after its retry policy.
    Database(DbError),
    /// The bounded stream observed a row beyond the configured source limit.
    SourceLimitExceeded {
        /// Configured maximum number of rows.
        maximum: usize,
    },
    /// A returned row did not have the four-column query shape.
    RowShape {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Required column count.
        expected: usize,
        /// Actual column count.
        actual: usize,
    },
    /// A source cell could not be decoded as its optional SQL type.
    ColumnDecode {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Source column name.
        column: &'static str,
        /// Underlying `SQLx` decode error.
        source: db::sqlx::Error,
    },
    /// The source-row vector could not reserve its bounded capacity.
    AllocationFailed {
        /// Number of rows returned by the bounded stream.
        requested: usize,
    },
    /// The pure row policy rejected a NULL, range, or allocation violation.
    Row(QuestRowError),
}

impl fmt::Display for QuestSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => write!(formatter, "quest database query failed: {source}"),
            Self::SourceLimitExceeded { maximum } => {
                write!(formatter, "quest source returned more than {maximum} rows")
            }
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "quest row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "quest row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "quest could not allocate {requested} source row(s)"
            ),
            Self::Row(source) => write!(formatter, "quest row is invalid: {source}"),
        }
    }
}

impl Error for QuestSqlxLoadError {
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

/// Execute a checked quest query and return a strict bounded lookup.
///
/// `SQLx` materializes each source cell before this adapter can copy or
/// validate it. Database/schema cell limits therefore remain a database-side
/// responsibility; this function bounds the row count and retains source
/// order.
///
/// # Errors
///
/// Returns [`QuestSqlxLoadError::Database`] for pool/query failures,
/// `SourceLimitExceeded` for more rows than `limits.max_rows`, typed row
/// shape/decode/allocation errors for malformed SQL results, and `Row` for
/// NULL or numeric-width policy failures.
pub async fn load_quest_sqlx(
    pool: &ConnectionPool,
    query: &QuestQuery,
    limits: QuestLoadLimits,
) -> Result<QuestLookup, QuestSqlxLoadError> {
    if limits.max_rows > MAX_QUEST_RECORDS {
        return Err(QuestSqlxLoadError::Row(QuestRowError::LimitTooLarge {
            requested: limits.max_rows,
            maximum: MAX_QUEST_RECORDS,
        }));
    }
    let rows = pool
        .query_up_to(query.as_str(), limits.max_rows)
        .await
        .map_err(QuestSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(QuestSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_rows,
        });
    };

    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        QuestSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        let actual = row.columns().len();
        if actual != QUEST_QUERY_COLUMNS {
            return Err(QuestSqlxLoadError::RowShape {
                row: row_index,
                expected: QUEST_QUERY_COLUMNS,
                actual,
            });
        }
        let pid = row.try_get::<Option<i64>, _>("dwPID").map_err(|source| {
            QuestSqlxLoadError::ColumnDecode {
                row: row_index,
                column: "dwPID",
                source,
            }
        })?;
        let name = row
            .try_get::<Option<Vec<u8>>, _>("szName")
            .map_err(|source| QuestSqlxLoadError::ColumnDecode {
                row: row_index,
                column: "szName",
                source,
            })?;
        let state = row
            .try_get::<Option<Vec<u8>>, _>("szState")
            .map_err(|source| QuestSqlxLoadError::ColumnDecode {
                row: row_index,
                column: "szState",
                source,
            })?;
        let value = row.try_get::<Option<i64>, _>("lValue").map_err(|source| {
            QuestSqlxLoadError::ColumnDecode {
                row: row_index,
                column: "lValue",
                source,
            }
        })?;
        source_rows.push(QuestQueryRow::new(pid, name, state, value));
    }

    let records =
        decode_quest_rows(&source_rows, limits.max_rows).map_err(QuestSqlxLoadError::Row)?;
    if records.is_empty() {
        Ok(QuestLookup::Empty)
    } else {
        Ok(QuestLookup::Found(records))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_keeps_four_column_contract_typed() {
        let error = QuestSqlxLoadError::RowShape {
            row: 2,
            expected: QUEST_QUERY_COLUMNS,
            actual: 3,
        };
        assert!(error.to_string().contains("row 2 has 3 columns"));
    }

    #[test]
    fn database_and_source_limit_are_not_empty_lookup_errors() {
        let source = QuestSqlxLoadError::SourceLimitExceeded { maximum: 9 };
        assert!(source.to_string().contains("more than 9 rows"));
    }
}
