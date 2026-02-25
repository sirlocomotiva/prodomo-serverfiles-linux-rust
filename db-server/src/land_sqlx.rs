//! `SQLx` acquisition adapter for the source-fixed `land` boot table.
//!
//! The immutable statement and `TABLE_POSTFIX` validation live in
//! [`crate::postfix`]. This adapter executes only that read query, bounds row
//! acquisition before accumulation, checks the nine-column result shape, and
//! delegates strict decoding and 36-byte packing to [`crate::land`].
//!
//! The legacy loader's optional maintenance statements guarded by
//! `ENABLE_CLEAR_OLD_GUILDS_LANDS_BY_INACTIVITY` are intentionally not copied.
//! `ConnectionPool::query_up_to` uses `sqlx::raw_sql` for the no-bind `MySQL` text-projection path; its
//! cells are read as `Option<String>` without applying a numeric type coercion.
//! The pure Land section builder then applies the strict policy. Source order,
//! duplicates, raw text, and SQL `NULL` are retained until that policy runs.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::land::{
    build_land_section_legacy_with_limits, build_land_section_with_limits, LandQueryValue,
    LandSectionError, LAND_TABLE_MAX_RECORDS, LAND_TABLE_QUERY_COLUMNS,
};
use crate::postfix::{LandTableLoader, LandTableQueryRow};

/// A failure while acquiring or bounding the source-fixed land table.
#[derive(Debug)]
pub enum LandSqlxLoadError {
    /// The pool could not execute the checked query after its retry policy.
    Database(DbError),
    /// The configured source-row cap cannot fit the legacy `u16` count.
    InvalidSourceLimit {
        /// Configured maximum number of source rows.
        maximum: usize,
        /// Maximum representable source-row cap.
        limit: usize,
    },
    /// The bounded stream observed a row beyond the configured cap.
    SourceLimitExceeded {
        /// Configured maximum number of source rows.
        maximum: usize,
    },
    /// A returned row did not have the nine-column shape promised by the
    /// fixed query.
    RowShape {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Required column count.
        expected: usize,
        /// Actual column count.
        actual: usize,
    },
    /// A source cell could not be decoded as optional SQL text.
    ColumnDecode {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Zero-based source column index.
        column: usize,
        /// Underlying `SQLx` decode error.
        source: db::sqlx::Error,
    },
    /// The source-row vector could not reserve its bounded result.
    AllocationFailed {
        /// Number of rows returned by the bounded stream.
        requested: usize,
    },
    /// The pure row policy rejected a NULL, numeric, or section-limit failure.
    Section(LandSectionError),
}

impl fmt::Display for LandSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => write!(formatter, "land database query failed: {source}"),
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "land source-row limit {maximum} exceeds the maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => {
                write!(formatter, "land source returned more than {maximum} rows")
            }
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "land row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "land row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "land adapter could not allocate {requested} source row(s)"
            ),
            Self::Section(source) => {
                write!(formatter, "land section construction failed: {source}")
            }
        }
    }
}

impl Error for LandSqlxLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(source) => Some(source),
            Self::ColumnDecode { source, .. } => Some(source),
            Self::Section(source) => Some(source),
            Self::InvalidSourceLimit { .. }
            | Self::SourceLimitExceeded { .. }
            | Self::RowShape { .. }
            | Self::AllocationFailed { .. } => None,
        }
    }
}

impl From<LandSectionError> for LandSqlxLoadError {
    fn from(source: LandSectionError) -> Self {
        Self::Section(source)
    }
}

/// Execute the immutable query held by `loader` and build one strict Land section.
///
/// Acquisition is bounded by `loader.limits().max_records` before rows are
/// accumulated. A valid empty result is an empty section; a database error,
/// an extra row, a malformed result shape, or a decode failure is never
/// represented as an empty section. `query_up_to` bounds rows, while the
/// configured packed-byte limit is checked by the pure builder after
/// acquisition.
///
/// # Errors
///
/// Returns [`LandSqlxLoadError::Database`] for pool/query failures,
/// `InvalidSourceLimit` for a cap above the `u16` wire count,
/// `SourceLimitExceeded` when the stream contains an extra row, typed
/// row-shape/decode/allocation errors for malformed SQL results, and
/// `Section` for strict NULL, numeric, or packed-byte policy failures.
pub async fn load_land_table_section_sqlx(
    pool: &ConnectionPool,
    loader: &LandTableLoader,
) -> Result<BootSection, LandSqlxLoadError> {
    let limits = loader.limits();
    validate_source_limit(limits.max_records)?;
    let rows = pool
        .query_up_to(loader.query().as_str(), limits.max_records)
        .await
        .map_err(LandSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(LandSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_records,
        });
    };

    let mut source_rows = Vec::new();
    source_rows
        .try_reserve_exact(rows.len())
        .map_err(|_| LandSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        let actual = row.columns().len();
        check_land_row_shape(row_index, actual)?;
        let mut cells = std::array::from_fn(|_| LandQueryValue::Null);
        for (column, cell) in cells.iter_mut().enumerate() {
            // `query_up_to` uses `sqlx::raw_sql`, so the MySQL
            // result is the source-compatible text projection. The unchecked
            // type read bypasses only SQLx metadata compatibility checks;
            // NULL and UTF-8/decode failures remain explicit below.
            let value = row
                .try_get_unchecked::<Option<String>, usize>(column)
                .map_err(|source| LandSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            *cell = text_cell_to_land_value(value);
        }
        source_rows.push(LandTableQueryRow::from_typed_columns(cells));
    }

    build_land_section_with_limits(&source_rows, limits).map_err(LandSqlxLoadError::Section)
}

/// Compatibility spelling for callers that use the generic loader name.
///
/// # Errors
///
/// Returns the same typed errors as [`load_land_table_section_sqlx`].
pub async fn load_land_section_sqlx(
    pool: &ConnectionPool,
    loader: &LandTableLoader,
) -> Result<BootSection, LandSqlxLoadError> {
    load_land_table_section_sqlx(pool, loader).await
}

/// Execute the same fixed query with the explicitly named legacy parser.
///
/// This does not execute maintenance SQL. It is provided only for callers
/// that consciously need the source-compatible `str_to_number` conversion;
/// ordinary adapters should use [`load_land_table_section_sqlx`].
///
/// # Errors
///
/// Returns the same acquisition errors as the strict adapter, with pure
/// source-cell and limit errors wrapped in `Section`.
pub async fn load_land_table_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &LandTableLoader,
) -> Result<BootSection, LandSqlxLoadError> {
    let limits = loader.limits();
    validate_source_limit(limits.max_records)?;
    let rows = pool
        .query_up_to(loader.query().as_str(), limits.max_records)
        .await
        .map_err(LandSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(LandSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_records,
        });
    };
    let mut source_rows = Vec::new();
    source_rows
        .try_reserve_exact(rows.len())
        .map_err(|_| LandSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        })?;
    for (row_index, row) in rows.into_iter().enumerate() {
        check_land_row_shape(row_index, row.columns().len())?;
        let mut cells = std::array::from_fn(|_| LandQueryValue::Null);
        for (column, cell) in cells.iter_mut().enumerate() {
            let value = row
                .try_get_unchecked::<Option<String>, usize>(column)
                .map_err(|source| LandSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            *cell = text_cell_to_land_value(value);
        }
        source_rows.push(LandTableQueryRow::from_typed_columns(cells));
    }
    build_land_section_legacy_with_limits(&source_rows, limits).map_err(LandSqlxLoadError::Section)
}

/// Validate a source-row cap before querying or accumulating rows.
///
/// # Errors
///
/// Returns [`LandSqlxLoadError::InvalidSourceLimit`] above the legacy `u16`
/// representable row count.
pub fn validate_source_limit(maximum: usize) -> Result<(), LandSqlxLoadError> {
    if maximum > LAND_TABLE_MAX_RECORDS {
        Err(LandSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: LAND_TABLE_MAX_RECORDS,
        })
    } else {
        Ok(())
    }
}

/// Convert one already-decoded SQL text cell without changing its value.
#[must_use]
pub fn text_cell_to_land_value(value: Option<String>) -> LandQueryValue {
    value.map_or(LandQueryValue::Null, LandQueryValue::Text)
}

/// Check the fixed nine-column result shape.
///
/// # Errors
///
/// Returns [`LandSqlxLoadError::RowShape`] when `actual` differs from the
/// source-fixed column count.
pub fn check_land_row_shape(row: usize, actual: usize) -> Result<(), LandSqlxLoadError> {
    if actual == LAND_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(LandSqlxLoadError::RowShape {
            row,
            expected: LAND_TABLE_QUERY_COLUMNS,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::land::LandSectionLimits;
    use crate::postfix::{LandTableQuery, TablePostfix};

    #[test]
    fn adapter_uses_checked_query_and_representable_limit() {
        let postfix = TablePostfix::parse("_test").unwrap();
        let loader = LandTableLoader::new(&postfix, LandSectionLimits::new(7)).unwrap();
        assert_eq!(
            loader.query().as_str(),
            LandTableQuery::new(&postfix).unwrap().as_str()
        );
        assert_eq!(
            loader.query().as_str(),
            "SELECT id, map_index, x, y, width, height, guild_id, guild_level_limit, price FROM land_test WHERE enable='YES' ORDER BY id"
        );
        assert!(loader.limits().max_records <= LAND_TABLE_MAX_RECORDS);
    }

    #[test]
    fn text_cells_keep_null_and_unmodified_numeric_text() {
        assert!(matches!(
            text_cell_to_land_value(None),
            LandQueryValue::Null
        ));
        assert!(matches!(
            text_cell_to_land_value(Some("00123".to_owned())),
            LandQueryValue::Text(value) if value == "00123"
        ));
    }

    #[test]
    fn source_cap_and_shape_failures_are_not_empty_sections() {
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(LAND_TABLE_MAX_RECORDS).is_ok());
        assert!(matches!(
            validate_source_limit(LAND_TABLE_MAX_RECORDS + 1),
            Err(LandSqlxLoadError::InvalidSourceLimit { .. })
        ));
        assert!(check_land_row_shape(0, 9).is_ok());
        for actual in [8, 10] {
            let Err(LandSqlxLoadError::RowShape {
                row,
                expected,
                actual: shape_actual,
            }) = check_land_row_shape(2, actual)
            else {
                panic!("expected row-shape error for {actual}");
            };
            assert_eq!(row, 2);
            assert_eq!(expected, 9);
            assert_eq!(shape_actual, actual);
        }
    }
}
