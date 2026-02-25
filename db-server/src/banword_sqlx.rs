//! `SQLx` acquisition adapter for the source-fixed `banword` table.
//!
//! The pure row and section rules live in [`crate::banword`]. This module
//! supplies only the real database call: it runs the exact legacy statement
//! `SELECT word FROM banword`, preserves fetch order, decodes the `word`
//! column as optional raw bytes, and delegates all NULL/size/limit handling to
//! the pure builder. The pool operation consumes a bounded `SQLx` stream and
//! retains the existing retry behavior for the complete read attempt. The
//! module does not choose a schema, create tables, populate a cache, or turn a
//! query failure into an empty table.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::banword::{
    build_banword_section, BanwordSectionError, BanwordSectionLimits, BANWORD_QUERY,
};

/// A failure while acquiring or bounding a SQLx-backed banword section.
#[derive(Debug)]
pub enum BanwordSqlxLoadError {
    /// The pool could not execute the fixed query after its retry policy.
    Database(DbError),
    /// The source contained a row beyond the configured source-row cap.
    SourceLimitExceeded {
        /// Configured maximum number of source rows.
        maximum: usize,
    },
    /// A returned row did not have the one-column shape promised by the query.
    RowShape {
        /// Zero-based row index in the result stream.
        row: usize,
        /// Required column count.
        expected: usize,
        /// Actual column count.
        actual: usize,
    },
    /// The `word` column could not be decoded as optional raw bytes.
    ColumnDecode {
        /// Zero-based row index in the result stream.
        row: usize,
        /// `SQLx` decode error.
        source: db::sqlx::Error,
    },
    /// The returned rows violate the pure section policy.
    Section(BanwordSectionError),
}

impl fmt::Display for BanwordSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => write!(formatter, "banword database query failed: {source}"),
            Self::SourceLimitExceeded { maximum } => {
                write!(formatter, "banword source exceeded the {maximum}-row limit")
            }
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "banword row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode { row, source } => {
                write!(
                    formatter,
                    "banword row {row} column decode failed: {source}"
                )
            }
            Self::Section(source) => {
                write!(formatter, "banword section construction failed: {source}")
            }
        }
    }
}

impl Error for BanwordSqlxLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(source) => Some(source),
            Self::ColumnDecode { source, .. } => Some(source),
            Self::Section(source) => Some(source),
            Self::SourceLimitExceeded { .. } | Self::RowShape { .. } => None,
        }
    }
}

impl From<BanwordSectionError> for BanwordSqlxLoadError {
    fn from(source: BanwordSectionError) -> Self {
        Self::Section(source)
    }
}

/// Query the source-fixed banword table and build one boot section.
///
/// The adapter deliberately accepts only the fixed query. It does not expose
/// SQL text, table names, or connection construction to callers. A `NULL` word
/// remains `None` and is skipped by [`build_banword_section`]; non-UTF-8 bytes
/// are retained without a lossy text conversion. Output row and packed-byte
/// limits are enforced; the database schema remains responsible for limiting
/// individual source-cell allocation, because `SQLx` materializes a cell before
/// this adapter can copy its bounded C-string prefix.
///
/// # Errors
///
/// Returns [`BanwordSqlxLoadError::Database`] for pool/query failures,
/// [`BanwordSqlxLoadError::SourceLimitExceeded`] when the bounded stream sees
/// more rows than allowed, [`BanwordSqlxLoadError::RowShape`] or
/// [`BanwordSqlxLoadError::ColumnDecode`] for malformed SQL results, and
/// [`BanwordSqlxLoadError::Section`] for pure record/byte/allocation limits.
pub async fn load_banword_section_sqlx(
    pool: &ConnectionPool,
    limits: BanwordSectionLimits,
) -> Result<BootSection, BanwordSqlxLoadError> {
    let rows = pool
        .query_up_to(BANWORD_QUERY, limits.max_source_rows)
        .await
        .map_err(BanwordSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(BanwordSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_source_rows,
        });
    };

    let mut values = Vec::new();
    values.try_reserve(rows.len()).map_err(|_| {
        BanwordSqlxLoadError::Section(BanwordSectionError::AllocationFailed {
            requested: rows.len(),
        })
    })?;
    for (row_index, row) in rows.into_iter().enumerate() {
        let actual = row.columns().len();
        check_banword_row_shape(row_index, actual)?;
        let word = row
            .try_get::<Option<Vec<u8>>, _>("word")
            .map_err(|source| BanwordSqlxLoadError::ColumnDecode {
                row: row_index,
                source,
            })?;
        values.push(word);
    }

    build_banword_section(&values, limits).map_err(BanwordSqlxLoadError::Section)
}

/// Compatibility spelling for callers that want the SQL-backed loader
/// without the profile-specific name. It has the same fixed-query contract as
/// [`load_banword_section_sqlx`].
///
/// # Errors
///
/// Returns the same typed database, source, row, and section errors as
/// [`load_banword_section_sqlx`].
pub async fn load_banword_section(
    pool: &ConnectionPool,
    limits: BanwordSectionLimits,
) -> Result<BootSection, BanwordSqlxLoadError> {
    load_banword_section_sqlx(pool, limits).await
}

fn check_banword_row_shape(row: usize, actual: usize) -> Result<(), BanwordSqlxLoadError> {
    const EXPECTED: usize = 1;
    if actual == EXPECTED {
        Ok(())
    } else {
        Err(BanwordSqlxLoadError::RowShape {
            row,
            expected: EXPECTED,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_uses_the_source_fixed_query() {
        assert_eq!(BANWORD_QUERY, "SELECT word FROM banword");
    }

    #[test]
    fn row_shape_is_checked_without_a_database() {
        assert!(check_banword_row_shape(0, 1).is_ok());
        assert!(matches!(
            check_banword_row_shape(3, 0),
            Err(BanwordSqlxLoadError::RowShape {
                row: 3,
                expected: 1,
                actual: 0
            })
        ));
        assert!(matches!(
            check_banword_row_shape(4, 2),
            Err(BanwordSqlxLoadError::RowShape {
                row: 4,
                expected: 1,
                actual: 2
            })
        ));
    }

    #[test]
    fn source_and_section_errors_remain_distinct() {
        let source = BanwordSqlxLoadError::SourceLimitExceeded { maximum: 7 };
        assert!(source.to_string().contains("7-row"));
        let section = BanwordSqlxLoadError::Section(BanwordSectionError::TooManyRecords {
            count: 2,
            maximum: 1,
        });
        assert!(section.to_string().contains("section construction failed"));
    }
}
