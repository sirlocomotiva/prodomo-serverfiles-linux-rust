//! Row-count-bounded `SQLx` acquisition for the source-fixed Monarch query.
//!
//! Query construction and conversion policies live in [`crate::monarch`].
//! This isolated adapter executes one checked immutable statement through
//! [`ConnectionPool::query_up_to`], rejects an extra row without truncation,
//! requires the exact five-cell shape, and reads each positional cell as raw
//! `Option<Vec<u8>>`. It preserves SQL `NULL`, source order, non-UTF-8 bytes,
//! and embedded NUL until the caller-selected pure policy runs.
//!
//! The adapter is not wired into boot, cache, transport, persistence, service,
//! or gameplay code. A successful zero-row query remains a distinct
//! [`MonarchSqlxLoadError::EmptyResult`].

use std::error::Error;
use std::fmt;

use db::pool::ConnectionPool;
use db::sqlx::Row;
use db::DbError;
use protocol::db_boot::BootMonarchInfo;

use crate::monarch::{
    build_monarch_info_legacy_with_limit, build_monarch_info_with_limit, MonarchBuildError,
    MonarchLoader, MonarchQueryRow, MonarchQueryValue, MONARCH_MAX_SOURCE_ROWS,
    MONARCH_QUERY_COLUMN_COUNT,
};

/// A failure while acquiring, bounding, or building Monarch rows.
#[derive(Debug)]
pub enum MonarchSqlxLoadError {
    /// The pool could not execute the checked query after its retry policy.
    Database(DbError),
    /// The configured source cap exceeds the four fixed empire slots.
    InvalidSourceLimit {
        /// Configured source-row cap.
        maximum: usize,
        /// Maximum accepted source-row cap.
        limit: usize,
    },
    /// The bounded stream observed a row beyond the configured cap.
    SourceLimitExceeded {
        /// Configured source-row cap.
        maximum: usize,
    },
    /// The source returned zero rows.
    EmptyResult,
    /// A returned row did not have the fixed five-cell shape.
    RowShape {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Required cell count.
        expected: usize,
        /// Actual cell count.
        actual: usize,
    },
    /// A source cell could not be decoded as optional raw bytes.
    ColumnDecode {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Zero-based source cell index.
        column: usize,
        /// Underlying `SQLx` decode error.
        source: db::sqlx::Error,
    },
    /// An adapter-owned source-row or per-row cell vector could not reserve
    /// its bounded result.
    AllocationFailed {
        /// Number of elements requested by the failed reservation.
        requested: usize,
    },
    /// The selected pure row or fixed-slot policy rejected acquired rows.
    PureBuild(MonarchBuildError),
}

impl fmt::Display for MonarchSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => {
                write!(formatter, "monarch database query failed: {source}")
            }
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "monarch source-row limit {maximum} exceeds maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => {
                write!(
                    formatter,
                    "monarch source returned more than {maximum} rows"
                )
            }
            Self::EmptyResult => write!(formatter, "monarch source returned no rows"),
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "monarch row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "monarch row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "monarch adapter could not reserve {requested} bounded element(s)"
            ),
            Self::PureBuild(source) => {
                write!(formatter, "monarch pure build failed: {source}")
            }
        }
    }
}

impl Error for MonarchSqlxLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(source) => Some(source),
            Self::ColumnDecode { source, .. } => Some(source),
            Self::PureBuild(source) => Some(source),
            Self::InvalidSourceLimit { .. }
            | Self::SourceLimitExceeded { .. }
            | Self::EmptyResult
            | Self::RowShape { .. }
            | Self::AllocationFailed { .. } => None,
        }
    }
}

impl From<MonarchBuildError> for MonarchSqlxLoadError {
    fn from(source: MonarchBuildError) -> Self {
        Self::PureBuild(source)
    }
}

/// Alias for callers that use `Sqlx` rather than `SQLx` in type names.
pub type MonarchSqlxError = MonarchSqlxLoadError;

/// Acquire checked Monarch rows without selecting a conversion policy.
///
/// Returned rows preserve bounded query source order, duplicates, SQL `NULL`,
/// and raw bytes. The query and cap come only from the immutable loader.
///
/// # Errors
///
/// Returns [`MonarchSqlxLoadError`] for an invalid cap, database failure,
/// extra row, empty source, wrong-width row, cell decode failure, or a failed
/// adapter-owned source-row/cell-vector reservation.
pub async fn load_monarch_rows_sqlx(
    pool: &ConnectionPool,
    loader: &MonarchLoader,
) -> Result<Vec<MonarchQueryRow>, MonarchSqlxLoadError> {
    acquire_rows(pool, loader).await
}

/// Acquire the checked rows and strictly build a fresh fixed output value.
///
/// # Errors
///
/// Returns acquisition failure or a selected pure build failure.
pub async fn load_monarch_info_sqlx(
    pool: &ConnectionPool,
    loader: &MonarchLoader,
) -> Result<BootMonarchInfo, MonarchSqlxLoadError> {
    let max_rows = loader.limits().max_rows;
    let rows = load_monarch_rows_sqlx(pool, loader).await?;
    build_monarch_info_with_limit(&rows, max_rows).map_err(MonarchSqlxLoadError::PureBuild)
}

/// Acquire the same rows and build with the explicit legacy policy.
///
/// This function selects only pure compatibility conversion. It does not add
/// maintenance SQL or production boot integration.
///
/// # Errors
///
/// Returns acquisition failure or a selected legacy pure build failure.
pub async fn load_monarch_info_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &MonarchLoader,
) -> Result<BootMonarchInfo, MonarchSqlxLoadError> {
    let max_rows = loader.limits().max_rows;
    let rows = load_monarch_rows_sqlx(pool, loader).await?;
    build_monarch_info_legacy_with_limit(&rows, max_rows).map_err(MonarchSqlxLoadError::PureBuild)
}

/// Validate a source cap before querying or accumulating rows.
///
/// Zero is representable. Any nonempty bounded result then reports
/// [`MonarchSqlxLoadError::SourceLimitExceeded`].
///
/// # Errors
///
/// Returns [`MonarchSqlxLoadError::InvalidSourceLimit`] when `maximum`
/// exceeds the four fixed slots.
pub fn validate_monarch_source_limit(maximum: usize) -> Result<(), MonarchSqlxLoadError> {
    if maximum > MONARCH_MAX_SOURCE_ROWS {
        Err(MonarchSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: MONARCH_MAX_SOURCE_ROWS,
        })
    } else {
        Ok(())
    }
}

/// Require at least one acquired source row.
///
/// # Errors
///
/// Returns [`MonarchSqlxLoadError::EmptyResult`] for an empty source.
pub fn check_monarch_nonempty(rows: &[MonarchQueryRow]) -> Result<(), MonarchSqlxLoadError> {
    if rows.is_empty() {
        Err(MonarchSqlxLoadError::EmptyResult)
    } else {
        Ok(())
    }
}

/// Convert one decoded optional SQL byte cell without normalization.
///
/// # Examples
///
/// ```
/// use db_server::monarch::MonarchQueryValue;
/// use db_server::monarch_sqlx::byte_cell_to_monarch_value;
///
/// let value = byte_cell_to_monarch_value(Some(vec![0xff, 0, b'7']));
/// assert!(matches!(value, MonarchQueryValue::Bytes(bytes) if bytes == [0xff, 0, b'7']));
/// ```
#[must_use]
pub fn byte_cell_to_monarch_value(value: Option<Vec<u8>>) -> MonarchQueryValue {
    value.map_or(MonarchQueryValue::Null, MonarchQueryValue::Bytes)
}

/// Alias emphasizing the lossless raw-cell boundary.
#[must_use]
pub fn raw_cell_to_monarch_value(value: Option<Vec<u8>>) -> MonarchQueryValue {
    byte_cell_to_monarch_value(value)
}

/// Check the exact five-cell result shape.
///
/// # Errors
///
/// Returns [`MonarchSqlxLoadError::RowShape`] when `actual` differs from the
/// source-fixed width.
pub fn check_monarch_row_shape(row: usize, actual: usize) -> Result<(), MonarchSqlxLoadError> {
    if actual == MONARCH_QUERY_COLUMN_COUNT {
        Ok(())
    } else {
        Err(MonarchSqlxLoadError::RowShape {
            row,
            expected: MONARCH_QUERY_COLUMN_COUNT,
            actual,
        })
    }
}

async fn acquire_rows(
    pool: &ConnectionPool,
    loader: &MonarchLoader,
) -> Result<Vec<MonarchQueryRow>, MonarchSqlxLoadError> {
    let max_rows = loader.limits().max_rows;
    validate_monarch_source_limit(max_rows)?;

    // This is the adapter's only database call. The immutable checked query
    // is borrowed, and the pool returns None on the first row beyond the cap.
    let bounded = pool
        .query_up_to(loader.query().as_str(), max_rows)
        .await
        .map_err(MonarchSqlxLoadError::Database)?;
    let Some(rows) = bounded else {
        return Err(MonarchSqlxLoadError::SourceLimitExceeded { maximum: max_rows });
    };
    if rows.is_empty() {
        return Err(MonarchSqlxLoadError::EmptyResult);
    }

    // These reservations cover only adapter-owned typed row/cell vectors.
    // The shared pool already accumulated Vec<MySqlRow>, and SQLx owns each
    // raw Option<Vec<u8>> allocation.
    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        MonarchSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        let actual = row.columns().len();
        check_monarch_row_shape(row_index, actual)?;
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(MONARCH_QUERY_COLUMN_COUNT)
            .map_err(|_| MonarchSqlxLoadError::AllocationFailed {
                requested: MONARCH_QUERY_COLUMN_COUNT,
            })?;
        for column in 0..MONARCH_QUERY_COLUMN_COUNT {
            // query_up_to uses the MySQL text/raw path. The unchecked read
            // bypasses numeric metadata checks only; positional bounds, NULL,
            // and raw bytes remain explicit.
            let value = row
                .try_get_unchecked::<Option<Vec<u8>>, usize>(column)
                .map_err(|source| MonarchSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            cells.push(byte_cell_to_monarch_value(value));
        }
        source_rows.push(MonarchQueryRow::from_columns(cells));
    }
    Ok(source_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monarch::{
        build_monarch_info_with_limit, MonarchLimits, MONARCH_MAX_SOURCE_ROWS,
        MONARCH_QUERY_COLUMN_COUNT as MONARCH_QUERY_COLUMNS,
    };
    use crate::postfix::TablePostfix;

    fn loader(max_rows: usize) -> MonarchLoader {
        MonarchLoader::new(
            &TablePostfix::parse("_test").unwrap(),
            MonarchLimits::new(max_rows),
        )
        .unwrap()
    }

    fn valid_row() -> MonarchQueryRow {
        MonarchQueryRow::from_typed_columns([
            MonarchQueryValue::bytes(b"0".to_vec()),
            MonarchQueryValue::bytes(b"7".to_vec()),
            MonarchQueryValue::bytes(b"name".to_vec()),
            MonarchQueryValue::bytes(b"9".to_vec()),
            MonarchQueryValue::bytes(b"date".to_vec()),
        ])
    }

    #[test]
    fn loader_exposes_only_the_checked_exact_query_and_cap() {
        let loader = loader(3);
        assert_eq!(loader.query().table_name(), "player_test");
        assert_eq!(
            loader.query().as_str(),
            "SELECT a.empire, a.pid, b.name, a.money, a.windate FROM monarch a, player_test b WHERE a.pid=b.id"
        );
        assert_eq!(loader.limits().max_rows, 3);
    }

    #[test]
    fn raw_cells_preserve_null_non_utf8_and_embedded_nul() {
        assert_eq!(byte_cell_to_monarch_value(None), MonarchQueryValue::Null);
        let source = vec![0xff, 0, b'7', 0x80];
        assert_eq!(
            raw_cell_to_monarch_value(Some(source.clone())),
            MonarchQueryValue::Bytes(source)
        );
    }

    #[test]
    fn helpers_distinguish_invalid_cap_shape_and_empty_result() {
        assert!(validate_monarch_source_limit(0).is_ok());
        assert!(validate_monarch_source_limit(MONARCH_MAX_SOURCE_ROWS).is_ok());
        assert!(matches!(
            validate_monarch_source_limit(MONARCH_MAX_SOURCE_ROWS + 1),
            Err(MonarchSqlxLoadError::InvalidSourceLimit {
                maximum: 5,
                limit: 4
            })
        ));
        assert!(check_monarch_row_shape(0, MONARCH_QUERY_COLUMNS).is_ok());
        assert!(matches!(
            check_monarch_row_shape(2, MONARCH_QUERY_COLUMNS - 1),
            Err(MonarchSqlxLoadError::RowShape {
                row: 2,
                expected: 5,
                actual: 4,
            })
        ));
        assert!(matches!(
            check_monarch_nonempty(&[]),
            Err(MonarchSqlxLoadError::EmptyResult)
        ));
        assert!(check_monarch_nonempty(&[valid_row()]).is_ok());
    }

    #[test]
    fn acquired_order_and_duplicates_reach_the_pure_builder_unchanged() {
        let first = valid_row();
        let rows = vec![first.clone(), first];
        assert_eq!(
            build_monarch_info_with_limit(&rows, 2),
            Err(crate::monarch::MonarchBuildError::DuplicateEmpire {
                empire: 0,
                first_row: 0,
                duplicate_row: 1,
            })
        );
    }
}
