//! `SQLx` acquisition adapter for the source-fixed `object` boot table.
//!
//! This adapter executes only the immutable statement held by
//! [`crate::object::ObjectTableQuery`]. It bounds acquisition before rows are
//! accumulated, checks the ten-column result shape, reads the no-bind `MySQL`
//! `sqlx::raw_sql` text projection positionally, and delegates strict decoding plus the
//! source-compatible first-wins/ascending-ID map projection to
//! [`crate::object`]. Database failures and limit/shape/decode failures are
//! never represented as an empty table.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::object::{
    build_object_map_section_legacy_with_limits, build_object_map_section_with_limits,
    ObjectQueryValue, ObjectSectionError, ObjectTableLoader, OBJECT_TABLE_MAX_RECORDS,
    OBJECT_TABLE_QUERY_COLUMNS,
};

/// A failure while acquiring or bounding the source-fixed object table.
#[derive(Debug)]
pub enum ObjectSqlxLoadError {
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
    /// A returned row did not have the fixed ten-column shape.
    RowShape {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Required column count.
        expected: usize,
        /// Actual column count.
        actual: usize,
    },
    /// A source cell could not be decoded as optional `SQL` text.
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
    Section(ObjectSectionError),
}

impl fmt::Display for ObjectSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => write!(formatter, "object database query failed: {source}"),
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "object source-row limit {maximum} exceeds the maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => {
                write!(formatter, "object source returned more than {maximum} rows")
            }
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "object row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "object row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "object adapter could not allocate {requested} source row(s)"
            ),
            Self::Section(source) => {
                write!(formatter, "object section construction failed: {source}")
            }
        }
    }
}

impl Error for ObjectSqlxLoadError {
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

impl From<ObjectSectionError> for ObjectSqlxLoadError {
    fn from(source: ObjectSectionError) -> Self {
        Self::Section(source)
    }
}

/// Execute the immutable object query and build one strict map-equivalent section.
///
/// Acquisition is bounded by `loader.limits().max_records` before rows are
/// accumulated. Raw cells retain source order until the pure map policy selects
/// the first row for each ID and emits selected IDs in ascending order. A valid
/// empty result is an empty section; database, limit, shape, and decode failures
/// remain distinct. `query_up_to` bounds rows; packed-byte limits are checked by
/// the pure builder after acquisition.
///
/// # Errors
///
/// Returns [`ObjectSqlxLoadError::Database`] for pool/query failures,
/// `InvalidSourceLimit` for a cap above the `u16` wire count,
/// `SourceLimitExceeded` when the stream contains an extra row, typed
/// row-shape/decode/allocation errors for malformed results, and `Section`
/// for strict NULL, numeric, or packed-byte policy failures.
pub async fn load_object_table_section_sqlx(
    pool: &ConnectionPool,
    loader: &ObjectTableLoader,
) -> Result<BootSection, ObjectSqlxLoadError> {
    let limits = loader.limits();
    validate_source_limit(limits.max_records)?;
    let rows = pool
        .query_up_to(loader.query().as_str(), limits.max_records)
        .await
        .map_err(ObjectSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(ObjectSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_records,
        });
    };

    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        ObjectSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        check_object_row_shape(row_index, row.columns().len())?;
        let mut cells = std::array::from_fn(|_| ObjectQueryValue::Null);
        for (column, cell) in cells.iter_mut().enumerate() {
            // The `sqlx::raw_sql` no-bind query uses the source-compatible text projection.
            // Positional unchecked reads avoid generated column-name metadata;
            // NULL and decode failures remain explicit.
            let value = row
                .try_get_unchecked::<Option<String>, usize>(column)
                .map_err(|source| ObjectSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            *cell = text_cell_to_object_value(value);
        }
        source_rows.push(crate::object::ObjectTableQueryRow::from_typed_columns(
            cells,
        ));
    }

    build_object_map_section_with_limits(&source_rows, limits).map_err(ObjectSqlxLoadError::Section)
}

/// Compatibility spelling for callers that use the generic loader name.
///
/// # Errors
///
/// Returns the same typed errors as [`load_object_table_section_sqlx`].
pub async fn load_object_section_sqlx(
    pool: &ConnectionPool,
    loader: &ObjectTableLoader,
) -> Result<BootSection, ObjectSqlxLoadError> {
    load_object_table_section_sqlx(pool, loader).await
}

/// Execute the same fixed query with the explicitly named legacy parser and
/// map-equivalent first-wins/ascending-ID projection.
///
/// # Errors
///
/// Returns the same acquisition errors as the strict adapter, with pure
/// source-cell and limit errors wrapped in `Section`.
pub async fn load_object_table_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ObjectTableLoader,
) -> Result<BootSection, ObjectSqlxLoadError> {
    let limits = loader.limits();
    validate_source_limit(limits.max_records)?;
    let rows = pool
        .query_up_to(loader.query().as_str(), limits.max_records)
        .await
        .map_err(ObjectSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(ObjectSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_records,
        });
    };
    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        ObjectSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;
    for (row_index, row) in rows.into_iter().enumerate() {
        check_object_row_shape(row_index, row.columns().len())?;
        let mut cells = std::array::from_fn(|_| ObjectQueryValue::Null);
        for (column, cell) in cells.iter_mut().enumerate() {
            let value = row
                .try_get_unchecked::<Option<String>, usize>(column)
                .map_err(|source| ObjectSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            *cell = text_cell_to_object_value(value);
        }
        source_rows.push(crate::object::ObjectTableQueryRow::from_typed_columns(
            cells,
        ));
    }
    build_object_map_section_legacy_with_limits(&source_rows, limits)
        .map_err(ObjectSqlxLoadError::Section)
}

/// Validate a source-row cap before querying or accumulating rows.
///
/// # Errors
///
/// Returns [`ObjectSqlxLoadError::InvalidSourceLimit`] above the legacy `u16`
/// representable row count.
pub fn validate_source_limit(maximum: usize) -> Result<(), ObjectSqlxLoadError> {
    if maximum > OBJECT_TABLE_MAX_RECORDS {
        Err(ObjectSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: OBJECT_TABLE_MAX_RECORDS,
        })
    } else {
        Ok(())
    }
}

/// Convert one already-decoded SQL text cell without changing its value.
#[must_use]
pub fn text_cell_to_object_value(value: Option<String>) -> ObjectQueryValue {
    value.map_or(ObjectQueryValue::Null, ObjectQueryValue::Text)
}

/// Check the fixed ten-column result shape.
///
/// # Errors
///
/// Returns [`ObjectSqlxLoadError::RowShape`] when `actual` differs from the
/// source-fixed column count.
pub fn check_object_row_shape(row: usize, actual: usize) -> Result<(), ObjectSqlxLoadError> {
    if actual == OBJECT_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(ObjectSqlxLoadError::RowShape {
            row,
            expected: OBJECT_TABLE_QUERY_COLUMNS,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{ObjectSectionLimits, ObjectTableLoader};
    use crate::postfix::TablePostfix;

    #[test]
    fn adapter_uses_checked_query_and_representable_limit() {
        let postfix = TablePostfix::parse("_test").unwrap();
        let loader = ObjectTableLoader::new(&postfix, ObjectSectionLimits::new(7)).unwrap();
        assert_eq!(
            loader.query().as_str(),
            "SELECT id, land_id, vnum, map_index, x, y, x_rot, y_rot, z_rot, life FROM object_test ORDER BY id"
        );
        assert!(loader.limits().max_records <= OBJECT_TABLE_MAX_RECORDS);
    }

    #[test]
    fn text_cells_keep_null_and_unmodified_text() {
        assert!(matches!(
            text_cell_to_object_value(None),
            ObjectQueryValue::Null
        ));
        assert!(matches!(
            text_cell_to_object_value(Some("001.25".to_owned())),
            ObjectQueryValue::Text(value) if value == "001.25"
        ));
    }

    #[test]
    fn source_cap_and_shape_failures_are_not_empty_sections() {
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(OBJECT_TABLE_MAX_RECORDS).is_ok());
        assert!(matches!(
            validate_source_limit(OBJECT_TABLE_MAX_RECORDS + 1),
            Err(ObjectSqlxLoadError::InvalidSourceLimit { .. })
        ));
        assert!(check_object_row_shape(0, 10).is_ok());
        for actual in [9, 11] {
            let Err(ObjectSqlxLoadError::RowShape {
                row,
                expected,
                actual: shape_actual,
            }) = check_object_row_shape(2, actual)
            else {
                panic!("expected row-shape error for {actual}");
            };
            assert_eq!(row, 2);
            assert_eq!(expected, 10);
            assert_eq!(shape_actual, actual);
        }
    }
}
