//! `SQLx` acquisition adapter for the source-fixed base shop boot table.
//!
//! The pure row and section rules live in [`crate::shop`].  This module owns
//! only the bounded database read.  It executes the exact statement exposed by
//! [`ShopTableLoader`], preserves the returned source order and duplicate rows,
//! and hands raw cells to the selected pure grouping/encoding policy.  It does
//! not choose a boot profile, build a cache, or turn a failed query into an
//! empty shop table.
//!
//! The pool's no-bind `sqlx::raw_sql` text-protocol path is used deliberately.
//! Every cell is read positionally as `Option<Vec<u8>>`; therefore SQL `NULL`,
//! embedded NUL bytes, and non-UTF-8 bytes remain distinct until the pure
//! decoder runs.  No text conversion or normalization occurs in this adapter.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::shop::{
    build_shop_section_legacy_with_limits, build_shop_section_with_limits, ShopQueryValue,
    ShopSectionError, ShopTableLoader, ShopTableQueryRow, SHOP_TABLE_MAX_RECORDS,
    SHOP_TABLE_MAX_SOURCE_ROWS, SHOP_TABLE_QUERY_COLUMNS,
};

/// A failure while acquiring or bounding a `SQLx`-backed base-shop table.
///
/// Acquisition errors are kept separate from the pure grouping and encoding
/// policy.  In particular, an empty source is rejected here because the
/// legacy `InitializeShopTable` loader does not accept an empty result.
#[derive(Debug)]
pub enum ShopSqlxLoadError {
    /// The pool could not execute the fixed query after its retry policy.
    Database(DbError),
    /// The configured distinct-shop cap cannot fit the legacy boot count, or
    /// a derived raw-row cap is outside the defensive acquisition bound.
    InvalidSourceLimit {
        /// Configured cap (distinct groups for a loader, raw rows for the
        /// standalone validator).
        maximum: usize,
        /// Applicable maximum cap.
        limit: usize,
    },
    /// The bounded stream observed a row beyond the configured cap.
    SourceLimitExceeded {
        /// Configured maximum number of source rows.
        maximum: usize,
    },
    /// The source returned zero rows.  The legacy loader rejects this result.
    EmptyResult,
    /// A returned row did not have the exact four-column query shape.
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
        /// Underlying `SQLx` error.
        source: db::sqlx::Error,
    },
    /// A bounded source-row or per-row cell vector could not reserve capacity.
    AllocationFailed {
        /// Number of elements requested by the failed reservation.
        requested: usize,
    },
    /// The pure shop grouping or section policy rejected the acquired rows.
    Section(ShopSectionError),
}

impl fmt::Display for ShopSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => write!(formatter, "shop database query failed: {source}"),
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "shop source-row limit {maximum} exceeds the maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => {
                write!(formatter, "shop source returned more than {maximum} rows")
            }
            Self::EmptyResult => write!(formatter, "shop source returned no rows"),
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "shop row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "shop row {row} column {column} raw-byte decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "shop adapter could not allocate {requested} bounded element(s)"
            ),
            Self::Section(source) => {
                write!(formatter, "shop section construction failed: {source}")
            }
        }
    }
}

impl Error for ShopSqlxLoadError {
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

impl From<ShopSectionError> for ShopSqlxLoadError {
    fn from(source: ShopSectionError) -> Self {
        Self::Section(source)
    }
}

/// Compatibility spelling for callers that omit the `Load` infix.
pub type ShopSqlxError = ShopSqlxLoadError;

/// Acquire raw rows from the checked base-shop query without selecting a
/// decoder.
///
/// The returned vector preserves query order, duplicate rows, SQL `NULL`, and
/// every raw byte cell.  The configured source cap is checked before the one
/// pool call.  A valid zero-row result is rejected because the legacy loader
/// requires at least one source row.
///
/// # Errors
///
/// Returns [`ShopSqlxLoadError`] for an invalid cap, a database failure, an
/// extra row, an empty source, a wrong-width row, a raw-byte decode failure,
/// or a failed fallible reservation.
pub async fn load_shop_table_rows_sqlx(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<Vec<ShopTableQueryRow>, ShopSqlxLoadError> {
    acquire_rows(pool, loader).await
}

/// Compatibility spelling for raw row acquisition without the table infix.
///
/// # Errors
///
/// Returns the same typed errors as [`load_shop_table_rows_sqlx`].
pub async fn load_shop_rows_sqlx(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<Vec<ShopTableQueryRow>, ShopSqlxLoadError> {
    load_shop_table_rows_sqlx(pool, loader).await
}

/// Acquire raw rows and strictly build one source-fixed base-shop section.
///
/// Rows reach the pure builder in source order, including duplicates.  The
/// builder applies all grouping, decoding, capacity, and section-limit policy.
/// No boot profile, transport, cache, or gameplay state is involved.
///
/// # Errors
///
/// Returns [`ShopSqlxLoadError`] for any acquisition failure, including an
/// empty source, or wraps a strict row/section failure in `Section`.
pub async fn load_shop_table_section_sqlx(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<BootSection, ShopSqlxLoadError> {
    let rows = load_shop_table_rows_sqlx(pool, loader).await?;
    build_shop_section_with_limits(&rows, loader.limits()).map_err(ShopSqlxLoadError::Section)
}

/// Compatibility spelling for callers that use the generic section name.
///
/// # Errors
///
/// Returns the same typed errors as [`load_shop_table_section_sqlx`].
pub async fn load_shop_section_sqlx(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<BootSection, ShopSqlxLoadError> {
    load_shop_table_section_sqlx(pool, loader).await
}

/// Compatibility spelling for the strict loader without the `SQLx` suffix.
///
/// # Errors
///
/// Returns the same typed errors as [`load_shop_table_section_sqlx`].
pub async fn load_shop_table_section(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<BootSection, ShopSqlxLoadError> {
    load_shop_table_section_sqlx(pool, loader).await
}

/// Compatibility spelling for the strict loader with both generic infixes
/// omitted.
///
/// # Errors
///
/// Returns the same typed errors as [`load_shop_table_section_sqlx`].
pub async fn load_shop_section(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<BootSection, ShopSqlxLoadError> {
    load_shop_table_section_sqlx(pool, loader).await
}

/// Acquire the same rows and build with the explicitly named legacy policy.
///
/// This function performs no maintenance SQL and selects no boot profile.  The
/// pure legacy builder owns all source-compatible numeric and text behavior.
///
/// # Errors
///
/// Returns the same typed acquisition errors as
/// [`load_shop_table_section_sqlx`] and wraps legacy row/section failures in
/// `Section`.
pub async fn load_shop_table_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<BootSection, ShopSqlxLoadError> {
    let rows = load_shop_table_rows_sqlx(pool, loader).await?;
    build_shop_section_legacy_with_limits(&rows, loader.limits())
        .map_err(ShopSqlxLoadError::Section)
}

/// Compatibility spelling for the legacy section loader.
///
/// # Errors
///
/// Returns the same typed errors as [`load_shop_table_section_legacy_sqlx`].
pub async fn load_shop_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<BootSection, ShopSqlxLoadError> {
    load_shop_table_section_legacy_sqlx(pool, loader).await
}

/// Compatibility spelling for the legacy loader without the `SQLx` suffix.
///
/// # Errors
///
/// Returns the same typed errors as [`load_shop_table_section_legacy_sqlx`].
pub async fn load_shop_table_section_legacy(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<BootSection, ShopSqlxLoadError> {
    load_shop_table_section_legacy_sqlx(pool, loader).await
}

/// Compatibility spelling for the legacy loader with both generic infixes
/// omitted.
///
/// # Errors
///
/// Returns the same typed errors as [`load_shop_table_section_legacy_sqlx`].
pub async fn load_shop_section_legacy(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<BootSection, ShopSqlxLoadError> {
    load_shop_table_section_legacy_sqlx(pool, loader).await
}

/// Validate a raw source-row cap before querying or accumulating rows.
///
/// This validator is for raw joined rows, not the distinct-shop boot count.
/// [`ShopTableLoader::source_row_limit`] derives the adapter's actual cap
/// from the caller's group limit.
///
/// # Errors
///
/// Returns [`ShopSqlxLoadError::InvalidSourceLimit`] when `maximum` exceeds
/// the defensive fixed-capacity source-row bound.
pub fn validate_source_limit(maximum: usize) -> Result<(), ShopSqlxLoadError> {
    if maximum > SHOP_TABLE_MAX_SOURCE_ROWS {
        Err(ShopSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: SHOP_TABLE_MAX_SOURCE_ROWS,
        })
    } else {
        Ok(())
    }
}

/// Compatibility spelling that names the source table explicitly.
///
/// # Errors
///
/// Returns [`ShopSqlxLoadError::InvalidSourceLimit`] when `maximum` exceeds
/// the defensive fixed-capacity source-row bound.
pub fn validate_shop_source_limit(maximum: usize) -> Result<(), ShopSqlxLoadError> {
    validate_source_limit(maximum)
}

/// Reject a bounded empty result at the `SQLx` boundary.
///
/// # Errors
///
/// Returns [`ShopSqlxLoadError::EmptyResult`] for zero acquired rows.
pub fn check_shop_nonempty(rows: &[ShopTableQueryRow]) -> Result<(), ShopSqlxLoadError> {
    if rows.is_empty() {
        Err(ShopSqlxLoadError::EmptyResult)
    } else {
        Ok(())
    }
}

/// Convert one decoded SQL cell without UTF-8 conversion or normalization.
///
/// Non-`NULL` values retain every byte, including zero and non-UTF-8 bytes.
#[must_use]
pub fn byte_cell_to_shop_value(value: Option<Vec<u8>>) -> ShopQueryValue {
    value.map_or(ShopQueryValue::Null, ShopQueryValue::Bytes)
}

/// Compatibility spelling for callers that name the input a raw cell.
#[must_use]
pub fn raw_cell_to_shop_value(value: Option<Vec<u8>>) -> ShopQueryValue {
    byte_cell_to_shop_value(value)
}

/// Check the exact four-column result shape before any cell is read.
///
/// # Errors
///
/// Returns [`ShopSqlxLoadError::RowShape`] when `actual` differs from
/// `SHOP_TABLE_QUERY_COLUMNS`.
pub fn check_shop_row_shape(row: usize, actual: usize) -> Result<(), ShopSqlxLoadError> {
    if actual == SHOP_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(ShopSqlxLoadError::RowShape {
            row,
            expected: SHOP_TABLE_QUERY_COLUMNS,
            actual,
        })
    }
}

async fn acquire_rows(
    pool: &ConnectionPool,
    loader: &ShopTableLoader,
) -> Result<Vec<ShopTableQueryRow>, ShopSqlxLoadError> {
    let group_limit = loader.limits().max_records;
    // A joined shop query can return many item rows for one shop. Use a
    // fail-closed defensive multiplier based on the fixed per-shop capacity,
    // while keeping the caller's distinct-group cap for the pure projection
    // policy. This is not a claim that the legacy schema rejects duplicate shop
    // rows; such a source is rejected rather than silently under-bounded.
    let maximum = loader
        .source_row_limit()
        .ok_or(ShopSqlxLoadError::InvalidSourceLimit {
            maximum: group_limit,
            limit: SHOP_TABLE_MAX_RECORDS,
        })?;
    validate_source_limit(maximum)?;

    // This is the adapter's only pool query.  The loader owns the exact
    // source statement, and `query_up_to` returns `None` when an extra row is
    // observed.  In particular, do not call a second query to distinguish an
    // empty result from a cap failure.
    let bounded = pool
        .query_up_to(loader.query().as_str(), maximum)
        .await
        .map_err(ShopSqlxLoadError::Database)?;
    let Some(rows) = bounded else {
        return Err(ShopSqlxLoadError::SourceLimitExceeded { maximum });
    };
    if rows.is_empty() {
        return Err(ShopSqlxLoadError::EmptyResult);
    }

    let mut source_rows = Vec::new();
    source_rows
        .try_reserve_exact(rows.len())
        .map_err(|_| ShopSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        check_shop_row_shape(row_index, row.columns().len())?;

        let mut cells = Vec::new();
        cells
            .try_reserve_exact(SHOP_TABLE_QUERY_COLUMNS)
            .map_err(|_| ShopSqlxLoadError::AllocationFailed {
                requested: SHOP_TABLE_QUERY_COLUMNS,
            })?;
        for column in 0..SHOP_TABLE_QUERY_COLUMNS {
            // The unchecked positional read bypasses only SQLx's numeric type
            // metadata check.  The exact row width was checked above; decode
            // failures and SQL NULL remain explicit and lossless.
            let value = row
                .try_get_unchecked::<Option<Vec<u8>>, usize>(column)
                .map_err(|source| ShopSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            cells.push(byte_cell_to_shop_value(value));
        }
        let cells = cells.try_into().map_err(|_| ShopSqlxLoadError::RowShape {
            row: row_index,
            expected: SHOP_TABLE_QUERY_COLUMNS,
            actual: SHOP_TABLE_QUERY_COLUMNS,
        })?;
        source_rows.push(ShopTableQueryRow::from_typed_columns(cells));
    }

    Ok(source_rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_is_the_exact_unpostfixed_source_statement() {
        let expected = "SELECT shop.vnum, shop.npc_vnum, shop_item.item_vnum, shop_item.count FROM shop LEFT JOIN shop_item ON shop.vnum = shop_item.shop_vnum ORDER BY shop.vnum, shop_item.item_vnum";
        let loader = ShopTableLoader::new(crate::shop::ShopSectionLimits::new(1));
        assert_eq!(loader.query().as_str(), expected);
        assert_eq!(crate::shop::ShopTableQuery::query(), expected);
        assert_eq!(crate::shop::SHOP_TABLE_QUERY, expected);
        assert_eq!(SHOP_TABLE_QUERY_COLUMNS, 4);
    }

    #[test]
    fn cap_and_shape_failures_remain_typed() {
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(SHOP_TABLE_MAX_SOURCE_ROWS).is_ok());
        assert!(matches!(
            validate_source_limit(SHOP_TABLE_MAX_SOURCE_ROWS + 1),
            Err(ShopSqlxLoadError::InvalidSourceLimit { maximum, limit })
                if maximum == SHOP_TABLE_MAX_SOURCE_ROWS + 1
                    && limit == SHOP_TABLE_MAX_SOURCE_ROWS
        ));
        let one_shop = ShopTableLoader::with_limit(1);
        assert_eq!(one_shop.source_row_limit(), Some(40));
        assert_eq!(one_shop.limits().max_records, 1);
        assert_eq!(ShopTableLoader::with_limit(0).source_row_limit(), Some(0));
        assert!(ShopTableLoader::with_limit(SHOP_TABLE_MAX_RECORDS + 1)
            .source_row_limit()
            .is_none());
        assert!(check_shop_row_shape(0, 4).is_ok());
        for actual in [3, 5] {
            assert!(matches!(
                check_shop_row_shape(2, actual),
                Err(ShopSqlxLoadError::RowShape {
                    row: 2,
                    expected: 4,
                    actual: shape_actual,
                }) if shape_actual == actual
            ));
        }
    }

    #[test]
    fn empty_source_is_not_an_empty_section() {
        assert!(matches!(
            check_shop_nonempty(&[]),
            Err(ShopSqlxLoadError::EmptyResult)
        ));
    }

    #[test]
    fn raw_cells_preserve_null_nul_and_non_utf8_bytes() {
        assert_eq!(byte_cell_to_shop_value(None), ShopQueryValue::Null);
        let source = vec![0xff, 0, b'7', 0x80];
        assert_eq!(
            byte_cell_to_shop_value(Some(source.clone())),
            ShopQueryValue::Bytes(source.clone())
        );
        assert_eq!(
            raw_cell_to_shop_value(Some(source.clone())),
            ShopQueryValue::Bytes(source)
        );
    }

    #[test]
    fn section_errors_are_distinct_from_empty_acquisition() {
        let section = ShopSectionError::EmptySource;
        let error = ShopSqlxLoadError::from(section.clone());
        assert!(matches!(&error, ShopSqlxLoadError::Section(source) if source == &section));
        assert!(error.to_string().contains("section construction failed"));
    }
}
