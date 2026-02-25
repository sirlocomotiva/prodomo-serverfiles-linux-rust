//! Bounded `SQLx` acquisition for the source-fixed active `item_proto` table.
//!
//! Query construction and all row policies live in [`crate::item_proto`].
//! This adapter executes only the checked immutable statement through
//! [`ConnectionPool::query_up_to`], rejects an extra row without truncation,
//! requires the exact 34-cell shape, and reads each cell as raw
//! `Option<Vec<u8>>`. It preserves SQL `NULL`, non-UTF-8 bytes, embedded NUL,
//! source order, and duplicate rows until the caller-selected pure policy runs.
//!
//! A successful zero-row query is an error because the legacy loader rejects
//! `uiNumRows == 0`. No boot profile, cache, manager mutation, service, or live
//! boot caller is added here.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::item_proto::{
    build_item_proto_section_legacy_with_limits, build_item_proto_section_with_limits,
    ItemProtoLoader, ItemProtoQueryRow, ItemProtoQueryValue, ItemProtoSectionError,
    ITEM_PROTO_QUERY_COLUMN_COUNT, ITEM_PROTO_TABLE_MAX_RECORDS,
};

/// A failure while acquiring or bounding the active `item_proto` table.
#[derive(Debug)]
pub enum ItemProtoSqlxLoadError {
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
    /// A returned row did not have the fixed 34-cell shape.
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
    /// A source-row vector or per-row cell vector could not reserve its bounded
    /// result.
    AllocationFailed {
        /// Number of elements requested by the failed reservation.
        requested: usize,
    },
    /// The selected pure row or section policy rejected acquired rows.
    Section(ItemProtoSectionError),
}

impl fmt::Display for ItemProtoSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => {
                write!(formatter, "item-proto database query failed: {source}")
            }
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "item-proto source-row limit {maximum} exceeds maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => write!(
                formatter,
                "item-proto source returned more than {maximum} rows"
            ),
            Self::EmptyResult => write!(formatter, "item-proto source returned no rows"),
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "item-proto row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "item-proto row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "item-proto adapter could not allocate {requested} bounded element(s)"
            ),
            Self::Section(source) => {
                write!(
                    formatter,
                    "item-proto section construction failed: {source}"
                )
            }
        }
    }
}

impl Error for ItemProtoSqlxLoadError {
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

impl From<ItemProtoSectionError> for ItemProtoSqlxLoadError {
    fn from(source: ItemProtoSectionError) -> Self {
        Self::Section(source)
    }
}

/// Alias for callers that use `Sqlx` rather than `SQLx` in type names.
pub type ItemProtoSqlxError = ItemProtoSqlxLoadError;

/// Acquire checked `item_proto` rows without selecting a conversion policy.
///
/// The returned vector preserves the bounded query's source order, duplicate
/// rows, SQL `NULL`, and raw bytes. The query and cap are read only from the
/// validated loader. A valid empty source is rejected rather than fabricated
/// into an empty section.
///
/// # Errors
///
/// Returns [`ItemProtoSqlxLoadError`] for an invalid cap, database failure,
/// extra row, empty source, wrong-width row, cell decode failure, or failed
/// fallible reservation.
pub async fn load_item_proto_rows_sqlx(
    pool: &ConnectionPool,
    loader: &ItemProtoLoader,
) -> Result<Vec<ItemProtoQueryRow>, ItemProtoSqlxLoadError> {
    acquire_rows(pool, loader).await
}

/// Alias emphasizing the physical table name.
///
/// # Errors
///
/// Returns the same errors as [`load_item_proto_rows_sqlx`].
pub async fn load_item_proto_table_rows_sqlx(
    pool: &ConnectionPool,
    loader: &ItemProtoLoader,
) -> Result<Vec<ItemProtoQueryRow>, ItemProtoSqlxLoadError> {
    load_item_proto_rows_sqlx(pool, loader).await
}

/// Acquire the checked query and strictly build one active item section.
///
/// Rows are handed to the strict pure policy in query order, including
/// duplicates. Packed-size and count checks occur before output allocation.
///
/// # Errors
///
/// Returns [`ItemProtoSqlxLoadError`] for acquisition failure or a strict
/// row, count, byte-limit, allocation, or encoder-width failure.
pub async fn load_item_proto_section_sqlx(
    pool: &ConnectionPool,
    loader: &ItemProtoLoader,
) -> Result<BootSection, ItemProtoSqlxLoadError> {
    let limits = loader.limits();
    let rows = load_item_proto_rows_sqlx(pool, loader).await?;
    build_item_proto_section_with_limits(&rows, limits).map_err(ItemProtoSqlxLoadError::Section)
}

/// Alias emphasizing the physical table name.
///
/// # Errors
///
/// Returns the same errors as [`load_item_proto_section_sqlx`].
pub async fn load_item_proto_table_section_sqlx(
    pool: &ConnectionPool,
    loader: &ItemProtoLoader,
) -> Result<BootSection, ItemProtoSqlxLoadError> {
    load_item_proto_section_sqlx(pool, loader).await
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
pub async fn load_item_proto_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ItemProtoLoader,
) -> Result<BootSection, ItemProtoSqlxLoadError> {
    let limits = loader.limits();
    let rows = load_item_proto_rows_sqlx(pool, loader).await?;
    build_item_proto_section_legacy_with_limits(&rows, limits)
        .map_err(ItemProtoSqlxLoadError::Section)
}

/// Alias emphasizing the physical table name.
///
/// # Errors
///
/// Returns the same errors as [`load_item_proto_section_legacy_sqlx`].
pub async fn load_item_proto_table_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ItemProtoLoader,
) -> Result<BootSection, ItemProtoSqlxLoadError> {
    load_item_proto_section_legacy_sqlx(pool, loader).await
}

/// Validate a source cap before querying or accumulating rows.
///
/// The cap must fit the section's `u16` count. A zero cap is representable;
/// any nonempty bounded result then reports `SourceLimitExceeded`.
///
/// # Errors
///
/// Returns [`ItemProtoSqlxLoadError::InvalidSourceLimit`] when the cap exceeds
/// the source-fixed maximum.
pub fn validate_source_limit(maximum: usize) -> Result<(), ItemProtoSqlxLoadError> {
    if maximum > ITEM_PROTO_TABLE_MAX_RECORDS {
        Err(ItemProtoSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: ITEM_PROTO_TABLE_MAX_RECORDS,
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
pub fn validate_item_proto_source_limit(maximum: usize) -> Result<(), ItemProtoSqlxLoadError> {
    validate_source_limit(maximum)
}

/// Require at least one acquired source row.
///
/// # Errors
///
/// Returns [`ItemProtoSqlxLoadError::EmptyResult`] for an empty source.
pub fn check_item_proto_nonempty(rows: &[ItemProtoQueryRow]) -> Result<(), ItemProtoSqlxLoadError> {
    if rows.is_empty() {
        Err(ItemProtoSqlxLoadError::EmptyResult)
    } else {
        Ok(())
    }
}

/// Convert one decoded optional SQL byte cell without normalization.
///
/// # Examples
///
/// ```
/// use db_server::item_proto::ItemProtoQueryValue;
/// use db_server::item_proto_sqlx::byte_cell_to_item_proto_value;
///
/// let value = byte_cell_to_item_proto_value(Some(vec![0xff, 0, b'7']));
/// assert!(matches!(value, ItemProtoQueryValue::Bytes(bytes) if bytes == [0xff, 0, b'7']));
/// ```
#[must_use]
pub fn byte_cell_to_item_proto_value(value: Option<Vec<u8>>) -> ItemProtoQueryValue {
    value.map_or(ItemProtoQueryValue::Null, ItemProtoQueryValue::Bytes)
}

/// Alias emphasizing the lossless raw-cell boundary.
#[must_use]
pub fn raw_cell_to_item_proto_value(value: Option<Vec<u8>>) -> ItemProtoQueryValue {
    byte_cell_to_item_proto_value(value)
}

/// Check the exact 34-cell result shape.
///
/// # Errors
///
/// Returns [`ItemProtoSqlxLoadError::RowShape`] when `actual` differs from the
/// source-fixed width.
pub fn check_item_proto_row_shape(row: usize, actual: usize) -> Result<(), ItemProtoSqlxLoadError> {
    if actual == ITEM_PROTO_QUERY_COLUMN_COUNT {
        Ok(())
    } else {
        Err(ItemProtoSqlxLoadError::RowShape {
            row,
            expected: ITEM_PROTO_QUERY_COLUMN_COUNT,
            actual,
        })
    }
}

async fn acquire_rows(
    pool: &ConnectionPool,
    loader: &ItemProtoLoader,
) -> Result<Vec<ItemProtoQueryRow>, ItemProtoSqlxLoadError> {
    let limits = loader.limits();
    validate_source_limit(limits.max_records)?;

    // `query` is borrowed only from an immutable checked `ItemProtoQuery`.
    // The pool returns `None` as soon as a row beyond the cap is observed.
    let bounded = pool
        .query_up_to(loader.query().as_str(), limits.max_records)
        .await
        .map_err(ItemProtoSqlxLoadError::Database)?;
    let Some(rows) = bounded else {
        return Err(ItemProtoSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_records,
        });
    };
    if rows.is_empty() {
        return Err(ItemProtoSqlxLoadError::EmptyResult);
    }

    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        ItemProtoSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        let actual = row.columns().len();
        check_item_proto_row_shape(row_index, actual)?;
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(ITEM_PROTO_QUERY_COLUMN_COUNT)
            .map_err(|_| ItemProtoSqlxLoadError::AllocationFailed {
                requested: ITEM_PROTO_QUERY_COLUMN_COUNT,
            })?;
        for column in 0..ITEM_PROTO_QUERY_COLUMN_COUNT {
            // `query_up_to` uses the MySQL text/raw path. The unchecked read
            // bypasses numeric metadata checks only; positional bounds, NULL,
            // and raw bytes remain explicit.
            let value = row
                .try_get_unchecked::<Option<Vec<u8>>, usize>(column)
                .map_err(|source| ItemProtoSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            cells.push(byte_cell_to_item_proto_value(value));
        }
        source_rows.push(ItemProtoQueryRow::from_columns(cells));
    }
    Ok(source_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_proto::{ItemProtoSectionLimits, ITEM_PROTO_TABLE_WIRE_SIZE};
    use crate::postfix::TablePostfix;

    fn loader(max_records: usize) -> ItemProtoLoader {
        ItemProtoLoader::new(
            &TablePostfix::parse("_test").unwrap(),
            ItemProtoSectionLimits::new(max_records),
        )
        .unwrap()
    }

    fn valid_row() -> ItemProtoQueryRow {
        let mut cells = Vec::with_capacity(ITEM_PROTO_QUERY_COLUMN_COUNT);
        cells.push(ItemProtoQueryValue::bytes(b"1".to_vec()));
        cells.extend(
            (0..ITEM_PROTO_QUERY_COLUMN_COUNT - 1)
                .map(|_| ItemProtoQueryValue::bytes(b"0".to_vec())),
        );
        ItemProtoQueryRow::from_columns(cells)
    }

    #[test]
    fn loader_exposes_only_the_checked_exact_query() {
        let loader = loader(7);
        assert_eq!(loader.query().table_name(), "item_proto_test");
        assert!(loader
            .query()
            .as_str()
            .ends_with(" FROM item_proto_test ORDER BY vnum;"));
        assert!(!loader.query().as_str().contains("vnum_range"));
    }

    #[test]
    fn raw_cells_preserve_null_non_utf8_and_interior_nul() {
        assert!(matches!(
            byte_cell_to_item_proto_value(None),
            ItemProtoQueryValue::Null
        ));
        let source = vec![0xff, 0, b'7', 0x80];
        assert_eq!(
            raw_cell_to_item_proto_value(Some(source.clone())),
            ItemProtoQueryValue::Bytes(source.clone())
        );
    }

    #[test]
    fn helper_checks_distinguish_cap_shape_and_empty() {
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(ITEM_PROTO_TABLE_MAX_RECORDS).is_ok());
        assert!(matches!(
            validate_item_proto_source_limit(ITEM_PROTO_TABLE_MAX_RECORDS + 1),
            Err(ItemProtoSqlxLoadError::InvalidSourceLimit { .. })
        ));
        assert!(check_item_proto_row_shape(0, 34).is_ok());
        assert!(matches!(
            check_item_proto_row_shape(3, 33),
            Err(ItemProtoSqlxLoadError::RowShape {
                row: 3,
                expected: 34,
                actual: 33
            })
        ));
        assert!(matches!(
            check_item_proto_nonempty(&[]),
            Err(ItemProtoSqlxLoadError::EmptyResult)
        ));
        assert!(check_item_proto_nonempty(&[valid_row()]).is_ok());
    }

    #[test]
    fn acquired_order_and_duplicates_reach_the_pure_builder() {
        let first = valid_row();
        let section = crate::item_proto::build_item_proto_section_with_limits(
            &[first.clone(), first],
            ItemProtoSectionLimits::new(2),
        )
        .unwrap();
        assert_eq!(section.record_size as usize, ITEM_PROTO_TABLE_WIRE_SIZE);
        assert_eq!(section.count, 2);
        assert_eq!(section.data.len(), ITEM_PROTO_TABLE_WIRE_SIZE * 2);
    }
}
