//! `SQLx` acquisition adapter for the source-fixed `object_proto` boot table.
//!
//! The checked statement and `TABLE_POSTFIX` validation live in
//! [`crate::object_proto`]. This adapter executes only that read query, bounds
//! acquisition before rows are accumulated, checks the exact thirteen-column
//! result shape, and reads the no-bind `MySQL` text projection positionally.
//! It delegates all scalar/material policy and 96-byte packing to the pure
//! object-prototype boundary. Source order, duplicates, SQL `NULL`, and source
//! text remain distinct until the selected pure policy runs.
//!
//! A valid empty result is an empty section. Database, cap, shape, decode,
//! allocation, and pure-policy failures are never represented as an empty
//! section. No maintenance SQL, profile selection, cache/state mutation, or
//! live boot behavior is added here.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::object_proto::{
    build_object_proto_section_legacy_with_limits, build_object_proto_section_with_limits,
    ObjectProtoQueryValue, ObjectProtoSectionError, ObjectProtoTableLoader,
    ObjectProtoTableQueryRow, OBJECT_PROTO_TABLE_MAX_RECORDS, OBJECT_PROTO_TABLE_QUERY_COLUMNS,
};

/// A failure while acquiring or bounding the source-fixed object-prototype table.
#[derive(Debug)]
pub enum ObjectProtoSqlxLoadError {
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
    /// A returned row did not have the fixed thirteen-column shape.
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
    /// The selected pure row policy rejected a source cell or section limit.
    Section(ObjectProtoSectionError),
}

impl fmt::Display for ObjectProtoSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => {
                write!(formatter, "object-proto database query failed: {source}")
            }
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "object-proto source-row limit {maximum} exceeds the maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => {
                write!(
                    formatter,
                    "object-proto source returned more than {maximum} rows"
                )
            }
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "object-proto row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "object-proto row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "object-proto adapter could not allocate {requested} source row(s)"
            ),
            Self::Section(source) => {
                write!(
                    formatter,
                    "object-proto section construction failed: {source}"
                )
            }
        }
    }
}

impl Error for ObjectProtoSqlxLoadError {
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

impl From<ObjectProtoSectionError> for ObjectProtoSqlxLoadError {
    fn from(source: ObjectProtoSectionError) -> Self {
        Self::Section(source)
    }
}

/// Compatibility spelling for callers that omit the `Load` infix.
pub type ObjectProtoSqlxError = ObjectProtoSqlxLoadError;

/// Acquire raw object-prototype rows without selecting a conversion policy.
///
/// The returned vector preserves the bounded query's source order, duplicate
/// rows, SQL `NULL` values, and text cells. The query is read only from the
/// checked loader, and the configured row cap is validated before the pool
/// call. A valid empty result is returned as an empty vector.
///
/// # Errors
///
/// Returns [`ObjectProtoSqlxLoadError`] for an invalid source cap, a database
/// failure, an extra row, a wrong-width row, a text decode failure, or a
/// failed fallible row-vector reservation.
pub async fn load_object_proto_table_rows_sqlx(
    pool: &ConnectionPool,
    loader: &ObjectProtoTableLoader,
) -> Result<Vec<ObjectProtoTableQueryRow>, ObjectProtoSqlxLoadError> {
    acquire_rows(pool, loader).await
}

/// Compatibility spelling for raw row acquisition without the table infix.
///
/// # Errors
///
/// Returns the same errors as [`load_object_proto_table_rows_sqlx`].
pub async fn load_object_proto_rows_sqlx(
    pool: &ConnectionPool,
    loader: &ObjectProtoTableLoader,
) -> Result<Vec<ObjectProtoTableQueryRow>, ObjectProtoSqlxLoadError> {
    load_object_proto_table_rows_sqlx(pool, loader).await
}

/// Acquire the checked query and strictly build one object-prototype section.
///
/// Rows are handed to [`build_object_proto_section_with_limits`] in query
/// order, including duplicates. The packed-byte and `u16` count limits are
/// checked by the pure builder after bounded acquisition.
///
/// # Errors
///
/// Returns [`ObjectProtoSqlxLoadError`] for any acquisition failure or a
/// strict source-cell, packed-size, or count-policy failure.
pub async fn load_object_proto_table_section_sqlx(
    pool: &ConnectionPool,
    loader: &ObjectProtoTableLoader,
) -> Result<BootSection, ObjectProtoSqlxLoadError> {
    let rows = load_object_proto_table_rows_sqlx(pool, loader).await?;
    build_object_proto_section_with_limits(&rows, loader.limits())
        .map_err(ObjectProtoSqlxLoadError::Section)
}

/// Compatibility spelling for callers that use the generic section name.
///
/// # Errors
///
/// Returns the same errors as [`load_object_proto_table_section_sqlx`].
pub async fn load_object_proto_section_sqlx(
    pool: &ConnectionPool,
    loader: &ObjectProtoTableLoader,
) -> Result<BootSection, ObjectProtoSqlxLoadError> {
    load_object_proto_table_section_sqlx(pool, loader).await
}

/// Compatibility spelling for callers that omit the `SQLx` suffix.
///
/// # Errors
///
/// Returns the same errors as [`load_object_proto_table_section_sqlx`].
pub async fn load_object_proto_section(
    pool: &ConnectionPool,
    loader: &ObjectProtoTableLoader,
) -> Result<BootSection, ObjectProtoSqlxLoadError> {
    load_object_proto_table_section_sqlx(pool, loader).await
}

/// Acquire the same checked query and build with the named legacy policy.
///
/// This function does not execute maintenance SQL. It only selects the
/// explicitly named compatibility decoder after acquisition; callers should
/// use [`load_object_proto_table_section_sqlx`] for the strict boundary.
///
/// # Errors
///
/// Returns the same acquisition errors as the strict adapter, with legacy
/// source-cell and packed-section failures wrapped in `Section`.
pub async fn load_object_proto_table_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ObjectProtoTableLoader,
) -> Result<BootSection, ObjectProtoSqlxLoadError> {
    let rows = load_object_proto_table_rows_sqlx(pool, loader).await?;
    build_object_proto_section_legacy_with_limits(&rows, loader.limits())
        .map_err(ObjectProtoSqlxLoadError::Section)
}

/// Compatibility spelling for the legacy section loader.
///
/// # Errors
///
/// Returns the same errors as [`load_object_proto_table_section_legacy_sqlx`].
pub async fn load_object_proto_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ObjectProtoTableLoader,
) -> Result<BootSection, ObjectProtoSqlxLoadError> {
    load_object_proto_table_section_legacy_sqlx(pool, loader).await
}

/// Compatibility spelling for the legacy loader without the `SQLx` suffix.
///
/// # Errors
///
/// Returns the same errors as [`load_object_proto_table_section_legacy_sqlx`].
pub async fn load_object_proto_section_legacy(
    pool: &ConnectionPool,
    loader: &ObjectProtoTableLoader,
) -> Result<BootSection, ObjectProtoSqlxLoadError> {
    load_object_proto_table_section_legacy_sqlx(pool, loader).await
}

/// Validate a source-row cap before querying or accumulating rows.
///
/// # Errors
///
/// Returns [`ObjectProtoSqlxLoadError::InvalidSourceLimit`] above the legacy
/// `u16` representable row count.
pub fn validate_source_limit(maximum: usize) -> Result<(), ObjectProtoSqlxLoadError> {
    if maximum > OBJECT_PROTO_TABLE_MAX_RECORDS {
        Err(ObjectProtoSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: OBJECT_PROTO_TABLE_MAX_RECORDS,
        })
    } else {
        Ok(())
    }
}

/// Compatibility spelling that names the source table explicitly.
///
/// # Errors
///
/// Returns [`ObjectProtoSqlxLoadError::InvalidSourceLimit`] above the legacy
/// `u16` representable row count.
pub fn validate_object_proto_source_limit(maximum: usize) -> Result<(), ObjectProtoSqlxLoadError> {
    validate_source_limit(maximum)
}

/// Convert one already-decoded SQL text cell without changing its value.
#[must_use]
pub fn text_cell_to_object_proto_value(value: Option<String>) -> ObjectProtoQueryValue {
    value.map_or(ObjectProtoQueryValue::Null, ObjectProtoQueryValue::Text)
}

/// Compatibility spelling for callers that use the generic value name.
#[must_use]
pub fn text_cell_to_object_value(value: Option<String>) -> ObjectProtoQueryValue {
    text_cell_to_object_proto_value(value)
}

/// Check the fixed thirteen-column result shape.
///
/// # Errors
///
/// Returns [`ObjectProtoSqlxLoadError::RowShape`] when `actual` differs from
/// the source-fixed column count.
pub fn check_object_proto_row_shape(
    row: usize,
    actual: usize,
) -> Result<(), ObjectProtoSqlxLoadError> {
    if actual == OBJECT_PROTO_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(ObjectProtoSqlxLoadError::RowShape {
            row,
            expected: OBJECT_PROTO_TABLE_QUERY_COLUMNS,
            actual,
        })
    }
}

async fn acquire_rows(
    pool: &ConnectionPool,
    loader: &ObjectProtoTableLoader,
) -> Result<Vec<ObjectProtoTableQueryRow>, ObjectProtoSqlxLoadError> {
    let limits = loader.limits();
    validate_source_limit(limits.max_records)?;

    // The query is obtained only from the checked loader. Keep this one pool
    // call before any row/cell accumulation; `None` means the bounded stream
    // observed an extra row.
    let bounded = pool
        .query_up_to(loader.query().as_str(), limits.max_records)
        .await
        .map_err(ObjectProtoSqlxLoadError::Database)?;
    let Some(rows) = bounded else {
        return Err(ObjectProtoSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_records,
        });
    };

    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        ObjectProtoSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        check_object_proto_row_shape(row_index, row.columns().len())?;
        let mut cells = std::array::from_fn(|_| ObjectProtoQueryValue::Null);
        for (column, cell) in cells.iter_mut().enumerate() {
            // `query_up_to` uses `sqlx::raw_sql`, so MySQL returns
            // the source-compatible text projection. The unchecked positional
            // read bypasses only SQLx's generated column metadata; NULL and
            // decode failures remain explicit.
            let value = row
                .try_get_unchecked::<Option<String>, usize>(column)
                .map_err(|source| ObjectProtoSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            *cell = text_cell_to_object_proto_value(value);
        }
        source_rows.push(ObjectProtoTableQueryRow::from_typed_columns(cells));
    }

    Ok(source_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object_proto::{ObjectProtoSectionLimits, ObjectProtoTableQuery};
    use crate::postfix::TablePostfix;

    fn loader() -> ObjectProtoTableLoader {
        ObjectProtoTableLoader::new(
            &TablePostfix::parse("_test").unwrap(),
            ObjectProtoSectionLimits::new(7),
        )
        .unwrap()
    }

    #[test]
    fn adapter_uses_checked_query_and_representable_limit() {
        let loader = loader();
        assert_eq!(
            loader.query().as_str(),
            ObjectProtoTableQuery::new(&TablePostfix::parse("_test").unwrap())
                .unwrap()
                .as_str()
        );
        assert_eq!(
            loader.query().as_str(),
            "SELECT vnum, price, materials, upgrade_vnum, upgrade_limit_time, life, reg_1, reg_2, reg_3, reg_4, npc, group_vnum, dependent_group FROM object_proto_test ORDER BY vnum"
        );
        assert!(loader.limits().max_records <= OBJECT_PROTO_TABLE_MAX_RECORDS);
    }

    #[test]
    fn text_cells_keep_null_and_unmodified_text() {
        assert!(matches!(
            text_cell_to_object_proto_value(None),
            ObjectProtoQueryValue::Null
        ));
        assert!(matches!(
            text_cell_to_object_proto_value(Some("001.25".to_owned())),
            ObjectProtoQueryValue::Text(value) if value == "001.25"
        ));
        assert!(matches!(
            text_cell_to_object_value(Some(" 7suffix".to_owned())),
            ObjectProtoQueryValue::Text(value) if value == " 7suffix"
        ));
    }

    #[test]
    fn source_cap_and_shape_failures_are_not_empty_sections() {
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(OBJECT_PROTO_TABLE_MAX_RECORDS).is_ok());
        assert!(matches!(
            validate_source_limit(OBJECT_PROTO_TABLE_MAX_RECORDS + 1),
            Err(ObjectProtoSqlxLoadError::InvalidSourceLimit { .. })
        ));
        assert!(check_object_proto_row_shape(0, 13).is_ok());
        for actual in [12, 14] {
            let Err(ObjectProtoSqlxLoadError::RowShape {
                row,
                expected,
                actual: shape_actual,
            }) = check_object_proto_row_shape(2, actual)
            else {
                panic!("expected row-shape error for {actual}");
            };
            assert_eq!(row, 2);
            assert_eq!(expected, 13);
            assert_eq!(shape_actual, actual);
        }
    }

    #[test]
    fn empty_rows_are_not_an_error_and_policy_errors_remain_section_errors() {
        let error = ObjectProtoSqlxLoadError::Section(
            crate::object_proto::ObjectProtoSectionError::TooManyRecords {
                count: 1,
                maximum: 0,
            },
        );
        assert!(error.to_string().contains("section construction failed"));
        assert!(std::error::Error::source(&error).is_some());
    }
}
