//! Row-count-bounded `SQLx` acquisition for the source-fixed active `mob_proto` table.
//!
//! Query construction and all row policies live in [`crate::mob_proto`].
//! This adapter executes only the checked immutable statement through
//! [`ConnectionPool::query_up_to`], rejects an extra row without truncation,
//! requires the exact 70-cell shape, and reads each cell as raw
//! `Option<Vec<u8>>`. It preserves SQL `NULL`, non-UTF-8 bytes, embedded NUL,
//! source order, and duplicate rows until the caller-selected pure policy runs.
//!
//! A successful zero-row query is an error because the legacy loader rejects
//! `uiNumRows == 0`. No boot profile, cache, manager mutation, service, or live
//! boot caller is added here.
//!
//! Acquisition observes at most `limits.max_records` rows. This is a row-count
//! cap, not a total-memory bound: [`ConnectionPool::query_up_to`] and `SQLx`
//! allocate their internal `MySqlRow` and raw-cell storage before this adapter
//! receives them. The adapter's typed `AllocationFailed` variant covers only
//! its own fallible source-row and cell-vector reservations. The separate
//! `limits.max_data_bytes` value is a packed-output check performed by the
//! pure builder after acquisition. It is not used to derive a second row cap
//! or as a pre-acquisition memory estimate.

use std::error::Error;
use std::fmt;

use db::pool::ConnectionPool;
use db::sqlx::Row;
use db::DbError;
use protocol::db_boot::BootSection;

use crate::mob_proto::{
    build_mob_proto_section_legacy_with_limits, build_mob_proto_section_with_limits,
    MobProtoLoader, MobProtoQueryRow, MobProtoQueryValue, MobProtoSectionError,
    MOB_PROTO_QUERY_COLUMN_COUNT, MOB_PROTO_TABLE_MAX_RECORDS,
};

/// A failure while acquiring or bounding the active `mob_proto` table.
#[derive(Debug)]
pub enum MobProtoSqlxLoadError {
    /// The pool could not execute the checked query after its retry policy.
    Database(DbError),
    /// The configured source cap exceeds the legacy `u16` section count.
    InvalidSourceLimit {
        /// Configured source-row cap.
        maximum: usize,
        /// Maximum representable source-row cap.
        limit: usize,
    },
    /// The bounded stream observed a row beyond the configured cap.
    SourceLimitExceeded {
        /// Configured source-row cap.
        maximum: usize,
    },
    /// The source returned zero rows.
    EmptyResult,
    /// A returned row did not have the fixed 70-cell shape.
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
    /// An adapter-owned source-row or per-row cell vector could not reserve its
    /// bounded result. This does not cover allocations made by
    /// [`ConnectionPool::query_up_to`] or `SQLx` for the underlying rows/cells.
    AllocationFailed {
        /// Number of elements requested by the failed reservation.
        requested: usize,
    },
    /// The selected pure row or section policy rejected acquired rows.
    Section(MobProtoSectionError),
}

impl fmt::Display for MobProtoSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => {
                write!(formatter, "mob-proto database query failed: {source}")
            }
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "mob-proto source-row limit {maximum} exceeds maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => write!(
                formatter,
                "mob-proto source returned more than {maximum} rows"
            ),
            Self::EmptyResult => write!(formatter, "mob-proto source returned no rows"),
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "mob-proto row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "mob-proto row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "mob-proto adapter could not allocate {requested} bounded element(s)"
            ),
            Self::Section(source) => {
                write!(formatter, "mob-proto section construction failed: {source}")
            }
        }
    }
}

impl Error for MobProtoSqlxLoadError {
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

impl From<MobProtoSectionError> for MobProtoSqlxLoadError {
    fn from(source: MobProtoSectionError) -> Self {
        Self::Section(source)
    }
}

/// Alias for callers that use `Sqlx` rather than `SQLx` in type names.
pub type MobProtoSqlxError = MobProtoSqlxLoadError;

/// Acquire checked `mob_proto` rows without selecting a conversion policy.
///
/// The returned vector preserves the bounded query's source order, duplicate
/// rows, SQL `NULL`, and raw bytes. The query and cap are read only from the
/// validated loader. A valid empty source is rejected rather than fabricated
/// into an empty section.
///
/// # Errors
///
/// Returns [`MobProtoSqlxLoadError`] for an invalid cap, database failure,
/// extra row, empty source, wrong-width row, cell decode failure, or a failed
/// adapter-owned source-row/cell-vector reservation. Pool and `SQLx` allocation
/// behavior is outside this typed error boundary.
pub async fn load_mob_proto_rows_sqlx(
    pool: &ConnectionPool,
    loader: &MobProtoLoader,
) -> Result<Vec<MobProtoQueryRow>, MobProtoSqlxLoadError> {
    acquire_rows(pool, loader).await
}

/// Alias emphasizing the physical table name.
///
/// # Errors
///
/// Returns the same errors as [`load_mob_proto_rows_sqlx`].
pub async fn load_mob_proto_table_rows_sqlx(
    pool: &ConnectionPool,
    loader: &MobProtoLoader,
) -> Result<Vec<MobProtoQueryRow>, MobProtoSqlxLoadError> {
    load_mob_proto_rows_sqlx(pool, loader).await
}

/// Acquire the checked query and strictly build one active mob-prototype section.
///
/// Rows are handed to the strict pure policy in query order, including
/// duplicates. Packed-size and count checks occur before output allocation.
///
/// # Errors
///
/// Returns [`MobProtoSqlxLoadError`] for acquisition failure or a strict
/// row, count, byte-limit, allocation, or encoder-width failure.
pub async fn load_mob_proto_section_sqlx(
    pool: &ConnectionPool,
    loader: &MobProtoLoader,
) -> Result<BootSection, MobProtoSqlxLoadError> {
    let limits = loader.limits();
    let rows = load_mob_proto_rows_sqlx(pool, loader).await?;
    build_mob_proto_section_with_limits(&rows, limits).map_err(MobProtoSqlxLoadError::Section)
}

/// Alias emphasizing the physical table name.
///
/// # Errors
///
/// Returns the same errors as [`load_mob_proto_section_sqlx`].
pub async fn load_mob_proto_table_section_sqlx(
    pool: &ConnectionPool,
    loader: &MobProtoLoader,
) -> Result<BootSection, MobProtoSqlxLoadError> {
    load_mob_proto_section_sqlx(pool, loader).await
}

/// Acquire the same rows and build with the explicit legacy policy.
///
/// This function selects only the pure compatibility conversion. It does not
/// add maintenance SQL or production boot integration.
///
/// # Errors
///
/// Returns the same acquisition errors as the strict adapter, with legacy
/// row and packed-section failures wrapped in `Section`.
pub async fn load_mob_proto_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &MobProtoLoader,
) -> Result<BootSection, MobProtoSqlxLoadError> {
    let limits = loader.limits();
    let rows = load_mob_proto_rows_sqlx(pool, loader).await?;
    build_mob_proto_section_legacy_with_limits(&rows, limits)
        .map_err(MobProtoSqlxLoadError::Section)
}

/// Alias emphasizing the physical table name.
///
/// # Errors
///
/// Returns the same errors as [`load_mob_proto_section_legacy_sqlx`].
pub async fn load_mob_proto_table_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &MobProtoLoader,
) -> Result<BootSection, MobProtoSqlxLoadError> {
    load_mob_proto_section_legacy_sqlx(pool, loader).await
}

/// Validate a source cap before querying or accumulating rows.
///
/// The cap must fit the section's `u16` count. A zero cap is representable;
/// any nonempty bounded result then reports `SourceLimitExceeded`.
///
/// # Errors
///
/// Returns [`MobProtoSqlxLoadError::InvalidSourceLimit`] when the cap exceeds
/// the source-fixed maximum.
pub fn validate_source_limit(maximum: usize) -> Result<(), MobProtoSqlxLoadError> {
    if maximum > MOB_PROTO_TABLE_MAX_RECORDS {
        Err(MobProtoSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: MOB_PROTO_TABLE_MAX_RECORDS,
        })
    } else {
        Ok(())
    }
}

/// Alias for [`validate_source_limit`].
///
/// # Errors
///
/// Returns the same invalid-cap error as [`validate_source_limit`].
pub fn validate_mob_proto_source_limit(maximum: usize) -> Result<(), MobProtoSqlxLoadError> {
    validate_source_limit(maximum)
}

/// Require at least one acquired source row.
///
/// # Errors
///
/// Returns [`MobProtoSqlxLoadError::EmptyResult`] for an empty source.
pub fn check_mob_proto_nonempty(rows: &[MobProtoQueryRow]) -> Result<(), MobProtoSqlxLoadError> {
    if rows.is_empty() {
        Err(MobProtoSqlxLoadError::EmptyResult)
    } else {
        Ok(())
    }
}

/// Convert one decoded optional SQL byte cell without normalization.
///
/// # Examples
///
/// ```
/// use db_server::mob_proto::MobProtoQueryValue;
/// use db_server::mob_proto_sqlx::byte_cell_to_mob_proto_value;
///
/// let value = byte_cell_to_mob_proto_value(Some(vec![0xff, 0, b'7']));
/// assert!(matches!(value, MobProtoQueryValue::Bytes(bytes) if bytes == [0xff, 0, b'7']));
/// ```
#[must_use]
pub fn byte_cell_to_mob_proto_value(value: Option<Vec<u8>>) -> MobProtoQueryValue {
    value.map_or(MobProtoQueryValue::Null, MobProtoQueryValue::Bytes)
}

/// Alias emphasizing the lossless raw-cell boundary.
#[must_use]
pub fn raw_cell_to_mob_proto_value(value: Option<Vec<u8>>) -> MobProtoQueryValue {
    byte_cell_to_mob_proto_value(value)
}

/// Check the exact 70-cell result shape.
///
/// # Errors
///
/// Returns [`MobProtoSqlxLoadError::RowShape`] when `actual` differs from the
/// source-fixed width.
pub fn check_mob_proto_row_shape(row: usize, actual: usize) -> Result<(), MobProtoSqlxLoadError> {
    if actual == MOB_PROTO_QUERY_COLUMN_COUNT {
        Ok(())
    } else {
        Err(MobProtoSqlxLoadError::RowShape {
            row,
            expected: MOB_PROTO_QUERY_COLUMN_COUNT,
            actual,
        })
    }
}

async fn acquire_rows(
    pool: &ConnectionPool,
    loader: &MobProtoLoader,
) -> Result<Vec<MobProtoQueryRow>, MobProtoSqlxLoadError> {
    let limits = loader.limits();
    validate_source_limit(limits.max_records)?;

    // `query` is borrowed only from an immutable checked `MobProtoQuery`.
    // The pool returns `None` as soon as a row beyond the cap is observed.
    let bounded = pool
        .query_up_to(loader.query().as_str(), limits.max_records)
        .await
        .map_err(MobProtoSqlxLoadError::Database)?;
    let Some(rows) = bounded else {
        return Err(MobProtoSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_records,
        });
    };
    if rows.is_empty() {
        return Err(MobProtoSqlxLoadError::EmptyResult);
    }

    // These reservations cover only the adapter-owned typed row/cell vectors.
    // The shared pool already accumulated `Vec<MySqlRow>` with infallible
    // pushes, and SQLx owns each raw `Option<Vec<u8>>` allocation.
    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        MobProtoSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        let actual = row.columns().len();
        check_mob_proto_row_shape(row_index, actual)?;
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(MOB_PROTO_QUERY_COLUMN_COUNT)
            .map_err(|_| MobProtoSqlxLoadError::AllocationFailed {
                requested: MOB_PROTO_QUERY_COLUMN_COUNT,
            })?;
        for column in 0..MOB_PROTO_QUERY_COLUMN_COUNT {
            // `query_up_to` uses the MySQL text/raw path. The unchecked read
            // bypasses numeric metadata checks only; positional bounds, NULL,
            // and raw bytes remain explicit.
            let value = row
                .try_get_unchecked::<Option<Vec<u8>>, usize>(column)
                .map_err(|source| MobProtoSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            cells.push(byte_cell_to_mob_proto_value(value));
        }
        source_rows.push(MobProtoQueryRow::from_columns(cells));
    }
    Ok(source_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mob_proto::{
        MobProtoSectionLimits, MOB_PROTO_QUERY_COLUMN_COUNT as MOB_PROTO_QUERY_COLUMNS,
        MOB_PROTO_TABLE_WIRE_SIZE,
    };
    use crate::postfix::TablePostfix;

    fn loader(max_records: usize) -> MobProtoLoader {
        MobProtoLoader::new(
            &TablePostfix::parse("_test").unwrap(),
            MobProtoSectionLimits::new(max_records),
        )
        .unwrap()
    }

    fn valid_row() -> MobProtoQueryRow {
        let mut cells = Vec::with_capacity(MOB_PROTO_QUERY_COLUMN_COUNT);
        cells.push(MobProtoQueryValue::bytes(b"1".to_vec()));
        cells.extend(
            (0..MOB_PROTO_QUERY_COLUMN_COUNT - 1).map(|_| MobProtoQueryValue::bytes(b"0".to_vec())),
        );
        MobProtoQueryRow::from_columns(cells)
    }

    #[test]
    fn loader_exposes_only_the_checked_exact_query() {
        let loader = loader(7);
        assert_eq!(loader.query().table_name(), "mob_proto_test");
        assert!(loader
            .query()
            .as_str()
            .ends_with(" FROM mob_proto_test ORDER BY vnum;"));
        assert!(!loader.query().as_str().contains("vnum_range"));
    }

    #[test]
    fn raw_cells_preserve_null_non_utf8_and_interior_nul() {
        assert!(matches!(
            byte_cell_to_mob_proto_value(None),
            MobProtoQueryValue::Null
        ));
        let source = vec![0xff, 0, b'7', 0x80];
        assert_eq!(
            raw_cell_to_mob_proto_value(Some(source.clone())),
            MobProtoQueryValue::Bytes(source)
        );
    }

    #[test]
    fn helpers_distinguish_cap_shape_and_empty_result() {
        const SHORT_COLUMNS: usize = MOB_PROTO_QUERY_COLUMNS - 1;

        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(MOB_PROTO_TABLE_MAX_RECORDS).is_ok());
        assert!(matches!(
            validate_mob_proto_source_limit(MOB_PROTO_TABLE_MAX_RECORDS + 1),
            Err(MobProtoSqlxLoadError::InvalidSourceLimit { .. })
        ));
        assert!(check_mob_proto_row_shape(0, MOB_PROTO_QUERY_COLUMNS).is_ok());
        assert!(matches!(
            check_mob_proto_row_shape(3, SHORT_COLUMNS),
            Err(MobProtoSqlxLoadError::RowShape {
                row: 3,
                expected: MOB_PROTO_QUERY_COLUMNS,
                actual: SHORT_COLUMNS
            })
        ));
        assert!(matches!(
            check_mob_proto_nonempty(&[]),
            Err(MobProtoSqlxLoadError::EmptyResult)
        ));
        assert!(check_mob_proto_nonempty(&[valid_row()]).is_ok());
    }

    #[test]
    fn acquired_order_and_duplicates_reach_the_pure_builder() {
        let first = valid_row();
        let section = crate::mob_proto::build_mob_proto_section_with_limits(
            &[first.clone(), first],
            MobProtoSectionLimits::new(2),
        )
        .unwrap();
        assert_eq!(section.record_size as usize, MOB_PROTO_TABLE_WIRE_SIZE);
        assert_eq!(section.count, 2);
        assert_eq!(section.data.len(), MOB_PROTO_TABLE_WIRE_SIZE * 2);
    }
}
