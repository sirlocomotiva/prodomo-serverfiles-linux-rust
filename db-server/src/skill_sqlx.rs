//! `SQLx` acquisition adapter for the source-fixed `skill` boot table.
//!
//! The checked statement and `TABLE_POSTFIX` validation live in
//! [`crate::postfix`]. This adapter executes only that read query, bounds row
//! acquisition before accumulation, checks the exact thirty-column result
//! shape, and hands lossless cells to the pure [`crate::skill`] policy. It
//! does not choose a boot profile, replace or reorder a table, maintain a
//! cache, or turn an error into an empty section.
//!
//! The pool uses the no-bind `sqlx::raw_sql` text-protocol path. Positional
//! reads use `Option<Vec<u8>>`, so raw bytes, embedded NUL bytes, and non-UTF-8 values
//! survive unchanged until the selected pure policy runs. SQL `NULL`, source
//! order, and duplicate rows also remain distinct.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::postfix::SkillTableLoader;
use crate::skill::{
    build_skill_section_legacy_with_limits, build_skill_section_with_limits, SkillQueryValue,
    SkillSectionError, SkillTableQueryRow, SKILL_TABLE_MAX_RECORDS, SKILL_TABLE_QUERY_COLUMNS,
};

/// A failure while acquiring or bounding the source-fixed skill table.
#[derive(Debug)]
pub enum SkillSqlxLoadError {
    /// The pool could not execute the checked query after its retry policy.
    Database(DbError),
    /// The configured source-row cap exceeds the legacy `u16` count limit.
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
    /// The source returned zero rows. The active legacy loader rejects this
    /// case, so it must not be represented as an empty section.
    EmptyResult,
    /// A returned row did not have the source-fixed thirty-column shape.
    RowShape {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Required column count.
        expected: usize,
        /// Actual column count.
        actual: usize,
    },
    /// A source cell could not be decoded as optional raw bytes.
    ColumnDecode {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Zero-based source column index.
        column: usize,
        /// Underlying `SQLx` decode error.
        source: db::sqlx::Error,
    },
    /// A bounded row vector or per-row cell vector could not be reserved.
    AllocationFailed {
        /// Number of elements requested by the failed reservation.
        requested: usize,
    },
    /// The selected pure row/section policy rejected the acquired rows.
    Section(SkillSectionError),
}

impl fmt::Display for SkillSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => {
                write!(formatter, "skill database query failed: {source}")
            }
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "skill source-row limit {maximum} exceeds the maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => {
                write!(formatter, "skill source returned more than {maximum} rows")
            }
            Self::EmptyResult => write!(formatter, "skill source returned no rows"),
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "skill row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "skill row {row} column {column} raw-byte decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "skill adapter could not allocate {requested} bounded element(s)"
            ),
            Self::Section(source) => {
                write!(formatter, "skill section construction failed: {source}")
            }
        }
    }
}

impl Error for SkillSqlxLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(source) => Some(source),
            Self::ColumnDecode { source, .. } => Some(source),
            Self::Section(source) => Some(source),
            Self::InvalidSourceLimit { .. }
            | Self::SourceLimitExceeded { .. }
            | Self::EmptyResult
            | Self::RowShape { .. }
            | Self::AllocationFailed { .. } => None,
        }
    }
}

impl From<SkillSectionError> for SkillSqlxLoadError {
    fn from(source: SkillSectionError) -> Self {
        Self::Section(source)
    }
}

/// Compatibility spelling for callers that omit the `Load` infix.
pub type SkillSqlxError = SkillSqlxLoadError;

/// Acquire raw rows from the checked skill query without selecting a decoder.
///
/// The returned vector preserves query order, duplicate rows, SQL `NULL`, and
/// every raw byte cell. The configured cap is checked before the one pool
/// call. The active legacy loader rejects an empty source, so zero rows return
/// [`SkillSqlxLoadError::EmptyResult`].
///
/// # Errors
///
/// Returns [`SkillSqlxLoadError`] for an invalid cap, a database failure, an
/// extra row, an empty source, a wrong-width row, a raw-byte decode failure,
/// or a failed fallible vector reservation.
pub async fn load_skill_table_rows_sqlx(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<Vec<SkillTableQueryRow>, SkillSqlxLoadError> {
    acquire_rows(pool, loader).await
}

/// Compatibility spelling for raw row acquisition without the table infix.
///
/// # Errors
///
/// Returns the same typed errors as [`load_skill_table_rows_sqlx`].
pub async fn load_skill_rows_sqlx(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<Vec<SkillTableQueryRow>, SkillSqlxLoadError> {
    load_skill_table_rows_sqlx(pool, loader).await
}

/// Acquire raw rows and strictly build one source-fixed skill section.
///
/// Rows reach the pure builder in source order, including duplicates. The
/// builder applies all scalar and section-limit policy. No boot profile,
/// transport, cache, or gameplay state is involved.
///
/// # Errors
///
/// Returns [`SkillSqlxLoadError`] for any acquisition failure, including an
/// empty source, or wraps a strict row/section failure in `Section`.
pub async fn load_skill_table_section_sqlx(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<BootSection, SkillSqlxLoadError> {
    let rows = load_skill_table_rows_sqlx(pool, loader).await?;
    build_skill_section_with_limits(&rows, loader.limits()).map_err(SkillSqlxLoadError::Section)
}

/// Compatibility spelling for callers that use the generic section name.
///
/// # Errors
///
/// Returns the same typed errors as [`load_skill_table_section_sqlx`].
pub async fn load_skill_section_sqlx(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<BootSection, SkillSqlxLoadError> {
    load_skill_table_section_sqlx(pool, loader).await
}

/// Compatibility spelling for the strict loader without the `SQLx` suffix.
///
/// # Errors
///
/// Returns the same typed errors as [`load_skill_table_section_sqlx`].
pub async fn load_skill_table_section(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<BootSection, SkillSqlxLoadError> {
    load_skill_table_section_sqlx(pool, loader).await
}

/// Compatibility spelling for the strict loader with both generic infixes
/// omitted.
///
/// # Errors
///
/// Returns the same typed errors as [`load_skill_table_section_sqlx`].
pub async fn load_skill_section(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<BootSection, SkillSqlxLoadError> {
    load_skill_table_section_sqlx(pool, loader).await
}

/// Acquire the same rows and build with the explicitly named legacy policy.
///
/// This function performs no maintenance SQL and selects no boot profile. The
/// pure legacy builder owns all source-compatible numeric and text behavior.
///
/// # Errors
///
/// Returns the same typed acquisition errors as
/// [`load_skill_table_section_sqlx`] and wraps legacy row/section failures in
/// `Section`.
pub async fn load_skill_table_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<BootSection, SkillSqlxLoadError> {
    let rows = load_skill_table_rows_sqlx(pool, loader).await?;
    build_skill_section_legacy_with_limits(&rows, loader.limits())
        .map_err(SkillSqlxLoadError::Section)
}

/// Compatibility spelling for the legacy section loader.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_skill_table_section_legacy_sqlx`].
pub async fn load_skill_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<BootSection, SkillSqlxLoadError> {
    load_skill_table_section_legacy_sqlx(pool, loader).await
}

/// Compatibility spelling for the legacy loader without the `SQLx` suffix.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_skill_table_section_legacy_sqlx`].
pub async fn load_skill_table_section_legacy(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<BootSection, SkillSqlxLoadError> {
    load_skill_table_section_legacy_sqlx(pool, loader).await
}

/// Compatibility spelling for the legacy loader with both generic infixes
/// omitted.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_skill_table_section_legacy_sqlx`].
pub async fn load_skill_section_legacy(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<BootSection, SkillSqlxLoadError> {
    load_skill_table_section_legacy_sqlx(pool, loader).await
}

/// Validate a source-row cap before querying or accumulating rows.
///
/// # Errors
///
/// Returns [`SkillSqlxLoadError::InvalidSourceLimit`] when `maximum` exceeds
/// the source-fixed legacy count limit.
pub fn validate_source_limit(maximum: usize) -> Result<(), SkillSqlxLoadError> {
    if maximum > SKILL_TABLE_MAX_RECORDS {
        Err(SkillSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: SKILL_TABLE_MAX_RECORDS,
        })
    } else {
        Ok(())
    }
}

/// Compatibility spelling that names the source table explicitly.
///
/// # Errors
///
/// Returns [`SkillSqlxLoadError::InvalidSourceLimit`] when `maximum` exceeds
/// the source-fixed legacy count limit.
pub fn validate_skill_source_limit(maximum: usize) -> Result<(), SkillSqlxLoadError> {
    validate_source_limit(maximum)
}

/// Reject a bounded empty result at the `SQLx` boundary.
///
/// # Errors
///
/// Returns [`SkillSqlxLoadError::EmptyResult`] for zero acquired rows.
pub fn check_skill_nonempty(rows: &[SkillTableQueryRow]) -> Result<(), SkillSqlxLoadError> {
    if rows.is_empty() {
        Err(SkillSqlxLoadError::EmptyResult)
    } else {
        Ok(())
    }
}

/// Convert one decoded SQL cell without UTF-8 conversion or normalization.
///
/// Non-`NULL` values retain every byte, including zero and non-UTF-8 bytes.
#[must_use]
pub fn byte_cell_to_skill_value(value: Option<Vec<u8>>) -> SkillQueryValue {
    value.map_or(SkillQueryValue::Null, SkillQueryValue::Bytes)
}

/// Compatibility spelling for callers that name the input a raw cell.
#[must_use]
pub fn raw_cell_to_skill_value(value: Option<Vec<u8>>) -> SkillQueryValue {
    byte_cell_to_skill_value(value)
}

/// Check the exact thirty-column result shape before any cell is read.
///
/// # Errors
///
/// Returns [`SkillSqlxLoadError::RowShape`] when `actual` differs from
/// `SKILL_TABLE_QUERY_COLUMNS`.
pub fn check_skill_row_shape(row: usize, actual: usize) -> Result<(), SkillSqlxLoadError> {
    if actual == SKILL_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(SkillSqlxLoadError::RowShape {
            row,
            expected: SKILL_TABLE_QUERY_COLUMNS,
            actual,
        })
    }
}

async fn acquire_rows(
    pool: &ConnectionPool,
    loader: &SkillTableLoader,
) -> Result<Vec<SkillTableQueryRow>, SkillSqlxLoadError> {
    validate_source_limit(loader.limits().max_records)?;

    // This is the adapter's only pool query. The statement and cap both come
    // from the checked loader. `None` means an extra row was observed.
    let bounded = pool
        .query_up_to(loader.query().as_str(), loader.limits().max_records)
        .await
        .map_err(SkillSqlxLoadError::Database)?;
    let Some(rows) = bounded else {
        return Err(SkillSqlxLoadError::SourceLimitExceeded {
            maximum: loader.limits().max_records,
        });
    };
    if rows.is_empty() {
        return Err(SkillSqlxLoadError::EmptyResult);
    }

    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        SkillSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        check_skill_row_shape(row_index, row.columns().len())?;

        let mut cells = Vec::new();
        cells
            .try_reserve_exact(SKILL_TABLE_QUERY_COLUMNS)
            .map_err(|_| SkillSqlxLoadError::AllocationFailed {
                requested: SKILL_TABLE_QUERY_COLUMNS,
            })?;
        for column in 0..SKILL_TABLE_QUERY_COLUMNS {
            // The unchecked positional read bypasses only SQLx type metadata.
            // Bounds were checked against the exact row width above; decode
            // failures and SQL NULL remain explicit and lossless.
            let value = row
                .try_get_unchecked::<Option<Vec<u8>>, usize>(column)
                .map_err(|source| SkillSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            cells.push(byte_cell_to_skill_value(value));
        }
        source_rows.push(SkillTableQueryRow::from_typed_columns(cells));
    }

    Ok(source_rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_cells_preserve_null_nul_and_non_utf8_bytes() {
        assert!(matches!(
            byte_cell_to_skill_value(None),
            SkillQueryValue::Null
        ));
        let source = vec![0xff, 0, b'7', 0x80];
        assert!(matches!(
            byte_cell_to_skill_value(Some(source.clone())),
            SkillQueryValue::Bytes(value) if value == source
        ));
        assert!(matches!(
            raw_cell_to_skill_value(Some(source.clone())),
            SkillQueryValue::Bytes(value) if value == source
        ));
    }

    #[test]
    fn cap_and_shape_failures_remain_typed() {
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(SKILL_TABLE_MAX_RECORDS).is_ok());
        assert!(matches!(
            validate_source_limit(SKILL_TABLE_MAX_RECORDS + 1),
            Err(SkillSqlxLoadError::InvalidSourceLimit { .. })
        ));
        assert_eq!(SKILL_TABLE_QUERY_COLUMNS, 30);
        assert!(check_skill_row_shape(0, 30).is_ok());
        for actual in [29, 31] {
            assert!(matches!(
                check_skill_row_shape(2, actual),
                Err(SkillSqlxLoadError::RowShape {
                    row: 2,
                    expected: 30,
                    actual: shape_actual,
                }) if shape_actual == actual
            ));
        }
    }

    #[test]
    fn empty_source_is_not_an_empty_section() {
        assert!(matches!(
            check_skill_nonempty(&[]),
            Err(SkillSqlxLoadError::EmptyResult)
        ));
    }
}
