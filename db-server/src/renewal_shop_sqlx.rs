//! `SQLx` acquisition adapter for the source-verified renewal-shop boot table.
//!
//! The SQL-free row and section policy lives in [`crate::renewal_shop`]. This
//! module owns only the bounded read of its exact 28-column query. It calls the
//! pool's no-bind `sqlx::raw_sql` text-protocol path through one
//! [`ConnectionPool::query_up_to`] operation, preserves fetch order and raw
//! cell bytes, and delegates conversion and packing to the pure boundary.
//!
//! SQL `NULL`, embedded NUL bytes, and non-UTF-8 bytes remain distinct until
//! the selected strict or legacy decoder sees them. A valid empty result is an
//! empty `RenewalShop` section; unlike the base-shop boundary, it is not an
//! acquisition error.
//!
//! This is an acquisition adapter, not a live boot-database integration. It
//! does not select a boot profile, compose a `BootSnapshot`, populate a
//! cache, install a manager, prove schema readiness, or claim that
//! renewal shops are wired into the running server. The pool is supplied by
//! the caller, and no process or gameplay state is mutated here.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::renewal_shop::{
    build_renewal_shop_section_legacy_with_limits, build_renewal_shop_section_with_limits,
    renewal_shop_source_row_limit, RenewalShopQueryRow, RenewalShopQueryValue,
    RenewalShopSectionError, RenewalShopTableLoader, RENEWAL_SHOP_TABLE_MAX_RECORDS,
    RENEWAL_SHOP_TABLE_MAX_SOURCE_ROWS, RENEWAL_SHOP_TABLE_QUERY, RENEWAL_SHOP_TABLE_QUERY_COLUMNS,
};

/// A failure while acquiring or bounding a `SQLx`-backed renewal-shop table.
///
/// Acquisition failures, malformed result rows, allocation failures, and pure
/// section-policy failures remain distinct. In particular, a database failure
/// is never represented as a valid empty renewal-shop section.
#[derive(Debug)]
pub enum RenewalShopSqlxLoadError {
    /// The pool could not execute the fixed query after its retry policy.
    Database(DbError),
    /// The configured distinct-shop cap cannot fit the boot `u16` count, or a
    /// raw source-row cap supplied to [`validate_source_limit`] exceeds the
    /// defensive acquisition bound.
    InvalidSourceLimit {
        /// Configured distinct-shop cap for loader calls, or raw source-row cap
        /// for the standalone validator.
        maximum: usize,
        /// Applicable maximum cap.
        limit: usize,
    },
    /// The bounded stream observed a row beyond the derived source-row cap.
    SourceLimitExceeded {
        /// Configured maximum number of raw joined source rows.
        maximum: usize,
    },
    /// A returned row did not have the exact 28-column query shape.
    RowShape {
        /// Zero-based row index in the bounded result.
        row: usize,
        /// Required source column count.
        expected: usize,
        /// Actual source column count.
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
    /// The pure renewal-shop grouping or section policy rejected the rows.
    Section(RenewalShopSectionError),
}

impl fmt::Display for RenewalShopSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => {
                write!(formatter, "renewal-shop database query failed: {source}")
            }
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "renewal-shop source-row limit {maximum} exceeds the maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => {
                write!(
                    formatter,
                    "renewal-shop source returned more than {maximum} rows"
                )
            }
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "renewal-shop row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "renewal-shop row {row} column {column} raw-byte decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "renewal-shop adapter could not allocate {requested} bounded element(s)"
            ),
            Self::Section(source) => {
                write!(
                    formatter,
                    "renewal-shop section construction failed: {source}"
                )
            }
        }
    }
}

impl Error for RenewalShopSqlxLoadError {
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

impl From<RenewalShopSectionError> for RenewalShopSqlxLoadError {
    fn from(source: RenewalShopSectionError) -> Self {
        Self::Section(source)
    }
}

/// Compatibility spelling for callers that omit the `Load` infix.
pub type RenewalShopSqlxError = RenewalShopSqlxLoadError;

/// Acquire raw rows from the checked renewal-shop query without selecting a
/// decoder.
///
/// The returned vector preserves query order, duplicate rows, SQL `NULL`, and
/// every raw byte cell. The caller's distinct-shop limit is checked before the
/// one pool call and is converted into a bounded joined-row cap. A valid
/// zero-row result is returned as an empty vector.
///
/// # Errors
///
/// Returns [`RenewalShopSqlxLoadError`] for an invalid group/source limit, a
/// database failure, an extra row, a wrong-width row, a raw-byte decode
/// failure, or a failed fallible reservation.
pub async fn load_renewal_shop_table_rows_sqlx(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<Vec<RenewalShopQueryRow>, RenewalShopSqlxLoadError> {
    acquire_rows(pool, loader).await
}

/// Compatibility spelling for raw renewal-shop row acquisition without the
/// table infix.
///
/// # Errors
///
/// Returns the same typed errors as [`load_renewal_shop_table_rows_sqlx`].
pub async fn load_renewal_shop_rows_sqlx(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<Vec<RenewalShopQueryRow>, RenewalShopSqlxLoadError> {
    load_renewal_shop_table_rows_sqlx(pool, loader).await
}

/// Acquire raw rows and strictly build one source-fixed renewal-shop section.
///
/// Rows reach the pure builder in source order, including duplicates. A valid
/// empty source builds an empty `RenewalShop` section. Grouping, strict row
/// decoding, fixed item capacity, output-byte limits, and section encoding stay
/// in [`crate::renewal_shop`].
///
/// # Errors
///
/// Returns [`RenewalShopSqlxLoadError`] for acquisition, row-shape, raw-byte,
/// allocation, or strict projection failures.
pub async fn load_renewal_shop_table_section_sqlx(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<BootSection, RenewalShopSqlxLoadError> {
    let rows = load_renewal_shop_table_rows_sqlx(pool, loader).await?;
    build_renewal_shop_section_with_limits(&rows, loader.limits())
        .map_err(RenewalShopSqlxLoadError::Section)
}

/// Compatibility spelling for callers that use the generic renewal-shop
/// section name.
///
/// # Errors
///
/// Returns the same typed errors as [`load_renewal_shop_table_section_sqlx`].
pub async fn load_renewal_shop_section_sqlx(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<BootSection, RenewalShopSqlxLoadError> {
    load_renewal_shop_table_section_sqlx(pool, loader).await
}

/// Compatibility spelling for the strict section loader without the `SQLx`
/// suffix.
///
/// # Errors
///
/// Returns the same typed errors as [`load_renewal_shop_table_section_sqlx`].
pub async fn load_renewal_shop_table_section(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<BootSection, RenewalShopSqlxLoadError> {
    load_renewal_shop_table_section_sqlx(pool, loader).await
}

/// Compatibility spelling for the strict loader with both generic infixes
/// omitted.
///
/// # Errors
///
/// Returns the same typed errors as [`load_renewal_shop_table_section_sqlx`].
pub async fn load_renewal_shop_section(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<BootSection, RenewalShopSqlxLoadError> {
    load_renewal_shop_table_section_sqlx(pool, loader).await
}

/// Acquire the same raw rows and build with the explicitly named legacy
/// policy.
///
/// This function does not run maintenance SQL. The pure legacy builder owns
/// the source-compatible numeric conversion and stateful slot behavior. A
/// valid empty result remains a valid empty section.
///
/// # Errors
///
/// Returns the same acquisition errors as
/// [`load_renewal_shop_table_section_sqlx`] and wraps legacy row or section
/// failures in [`RenewalShopSqlxLoadError::Section`].
pub async fn load_renewal_shop_table_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<BootSection, RenewalShopSqlxLoadError> {
    let rows = load_renewal_shop_table_rows_sqlx(pool, loader).await?;
    build_renewal_shop_section_legacy_with_limits(&rows, loader.limits())
        .map_err(RenewalShopSqlxLoadError::Section)
}

/// Compatibility spelling for the legacy renewal-shop section loader.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_renewal_shop_table_section_legacy_sqlx`].
pub async fn load_renewal_shop_section_legacy_sqlx(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<BootSection, RenewalShopSqlxLoadError> {
    load_renewal_shop_table_section_legacy_sqlx(pool, loader).await
}

/// Compatibility spelling for the legacy loader without the `SQLx` suffix.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_renewal_shop_table_section_legacy_sqlx`].
pub async fn load_renewal_shop_table_section_legacy(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<BootSection, RenewalShopSqlxLoadError> {
    load_renewal_shop_table_section_legacy_sqlx(pool, loader).await
}

/// Compatibility spelling for the legacy loader with both generic infixes
/// omitted.
///
/// # Errors
///
/// Returns the same typed errors as
/// [`load_renewal_shop_table_section_legacy_sqlx`].
pub async fn load_renewal_shop_section_legacy(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<BootSection, RenewalShopSqlxLoadError> {
    load_renewal_shop_table_section_legacy_sqlx(pool, loader).await
}

/// Validate a raw joined-source-row cap before querying or accumulating rows.
///
/// This validator is for raw rows, not the distinct-shop boot count. Loader
/// calls derive their raw cap with
/// [`crate::renewal_shop::renewal_shop_source_row_limit`] after rejecting a
/// group cap that cannot fit the boot `u16` count.
///
/// # Errors
///
/// Returns [`RenewalShopSqlxLoadError::InvalidSourceLimit`] when `maximum`
/// exceeds the defensive global source-row ceiling.
pub fn validate_source_limit(maximum: usize) -> Result<(), RenewalShopSqlxLoadError> {
    if maximum > RENEWAL_SHOP_TABLE_MAX_SOURCE_ROWS {
        Err(RenewalShopSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: RENEWAL_SHOP_TABLE_MAX_SOURCE_ROWS,
        })
    } else {
        Ok(())
    }
}

/// Compatibility spelling that names the renewal-shop source explicitly.
///
/// # Errors
///
/// Returns the same typed error as [`validate_source_limit`].
pub fn validate_renewal_shop_source_limit(maximum: usize) -> Result<(), RenewalShopSqlxLoadError> {
    validate_source_limit(maximum)
}

/// Convert one decoded SQL cell without UTF-8 conversion or normalization.
///
/// `None` remains [`RenewalShopQueryValue::Null`]. A `Some` value is copied as
/// [`RenewalShopQueryValue::Bytes`], including zero and non-UTF-8 bytes.
#[must_use]
pub fn byte_cell_to_renewal_shop_value(value: Option<Vec<u8>>) -> RenewalShopQueryValue {
    value.map_or(RenewalShopQueryValue::Null, RenewalShopQueryValue::Bytes)
}

/// Compatibility spelling for callers that name the input a raw cell.
///
/// This conversion is infallible.
#[must_use]
pub fn raw_cell_to_renewal_shop_value(value: Option<Vec<u8>>) -> RenewalShopQueryValue {
    byte_cell_to_renewal_shop_value(value)
}

/// Check the exact 28-column result shape before any positional cell read.
///
/// # Errors
///
/// Returns [`RenewalShopSqlxLoadError::RowShape`] when `actual` differs from
/// [`RENEWAL_SHOP_TABLE_QUERY_COLUMNS`].
pub fn check_renewal_shop_row_shape(
    row: usize,
    actual: usize,
) -> Result<(), RenewalShopSqlxLoadError> {
    if actual == RENEWAL_SHOP_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(RenewalShopSqlxLoadError::RowShape {
            row,
            expected: RENEWAL_SHOP_TABLE_QUERY_COLUMNS,
            actual,
        })
    }
}

fn checked_source_row_limit(group_limit: usize) -> Result<usize, RenewalShopSqlxLoadError> {
    // Reject the distinct-shop limit before deriving the multiplication. This
    // both protects the source cap and prevents the pure section count from
    // exceeding the boot u16 representation.
    let maximum = renewal_shop_source_row_limit(group_limit).ok_or(
        RenewalShopSqlxLoadError::InvalidSourceLimit {
            maximum: group_limit,
            limit: RENEWAL_SHOP_TABLE_MAX_RECORDS,
        },
    )?;
    validate_source_limit(maximum)?;
    Ok(maximum)
}

async fn acquire_rows(
    pool: &ConnectionPool,
    loader: &RenewalShopTableLoader,
) -> Result<Vec<RenewalShopQueryRow>, RenewalShopSqlxLoadError> {
    let limits = loader.limits();
    let maximum = checked_source_row_limit(limits.max_records)?;

    // This is the adapter's only pool query. The statement is fixed here so
    // callers cannot substitute table names, predicates, or column order.
    // `query_up_to` returns `None` when a row beyond the derived cap is seen;
    // it does not require a second query to distinguish that case.
    let bounded = pool
        .query_up_to(RENEWAL_SHOP_TABLE_QUERY, maximum)
        .await
        .map_err(RenewalShopSqlxLoadError::Database)?;
    let Some(rows) = bounded else {
        return Err(RenewalShopSqlxLoadError::SourceLimitExceeded { maximum });
    };

    // Unlike the base-shop adapter, an empty result is valid. It is passed to
    // the selected pure builder, which creates an empty RenewalShop section.
    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        RenewalShopSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        check_renewal_shop_row_shape(row_index, row.columns().len())?;

        let mut cells = Vec::new();
        cells
            .try_reserve_exact(RENEWAL_SHOP_TABLE_QUERY_COLUMNS)
            .map_err(|_| RenewalShopSqlxLoadError::AllocationFailed {
                requested: RENEWAL_SHOP_TABLE_QUERY_COLUMNS,
            })?;
        for column in 0..RENEWAL_SHOP_TABLE_QUERY_COLUMNS {
            // The exact row width is already known. The unchecked positional
            // read bypasses only SQLx's numeric type-metadata check; index,
            // decode, NULL, and every returned byte remain explicit and lossless.
            let value = row
                .try_get_unchecked::<Option<Vec<u8>>, usize>(column)
                .map_err(|source| RenewalShopSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            cells.push(byte_cell_to_renewal_shop_value(value));
        }
        let cells = cells
            .try_into()
            .map_err(|_| RenewalShopSqlxLoadError::RowShape {
                row: row_index,
                expected: RENEWAL_SHOP_TABLE_QUERY_COLUMNS,
                actual: RENEWAL_SHOP_TABLE_QUERY_COLUMNS,
            })?;
        source_rows.push(RenewalShopQueryRow::from_typed_columns(cells));
    }

    // Keep this binding explicit: the adapter derives row bounds and the pure
    // builder independently enforces the same configured group/data limits.
    debug_assert_eq!(loader.limits(), limits);
    Ok(source_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renewal_shop::RenewalShopSectionLimits;

    #[test]
    fn adapter_uses_the_exact_28_column_source_statement() {
        let loader = RenewalShopTableLoader::new();
        assert_eq!(loader.query().as_str(), RENEWAL_SHOP_TABLE_QUERY);
        assert_eq!(RENEWAL_SHOP_TABLE_QUERY_COLUMNS, 28);
        assert_eq!(
            RENEWAL_SHOP_TABLE_QUERY,
            "SELECT shopex.vnum, shopex.name, shopex.npc_vnum, shopex_item.item_vnum, shopex_item.count, shopex_item.price, shopex_item.price_vnum, shopex_item.price_type+0, socket0, socket1, socket2, socket3, socket4, socket5, attrtype0, attrvalue0 , attrtype1, attrvalue1 , attrtype2, attrvalue2 , attrtype3, attrvalue3 , attrtype4, attrvalue4 , attrtype5, attrvalue5 , attrtype6, attrvalue6 FROM shopex LEFT JOIN shopex_item ON shopex.vnum = shopex_item.shop_vnum ORDER BY shopex.vnum, shopex_item.item_vnum"
        );
    }

    #[test]
    fn group_and_source_limits_are_checked_before_acquisition() {
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(RENEWAL_SHOP_TABLE_MAX_SOURCE_ROWS).is_ok());
        assert!(matches!(
            validate_source_limit(RENEWAL_SHOP_TABLE_MAX_SOURCE_ROWS + 1),
            Err(RenewalShopSqlxLoadError::InvalidSourceLimit {
                maximum,
                limit,
            }) if maximum == RENEWAL_SHOP_TABLE_MAX_SOURCE_ROWS + 1
                && limit == RENEWAL_SHOP_TABLE_MAX_SOURCE_ROWS
        ));

        assert_eq!(checked_source_row_limit(0).unwrap(), 0);
        assert_eq!(checked_source_row_limit(1).unwrap(), 40);
        assert_eq!(
            checked_source_row_limit(RENEWAL_SHOP_TABLE_MAX_RECORDS).unwrap(),
            RENEWAL_SHOP_TABLE_MAX_SOURCE_ROWS
        );
        assert!(matches!(
            checked_source_row_limit(RENEWAL_SHOP_TABLE_MAX_RECORDS + 1),
            Err(RenewalShopSqlxLoadError::InvalidSourceLimit {
                maximum,
                limit,
            }) if maximum == RENEWAL_SHOP_TABLE_MAX_RECORDS + 1
                && limit == RENEWAL_SHOP_TABLE_MAX_RECORDS
        ));
    }

    #[test]
    fn exact_row_shape_is_checked_before_positional_reads() {
        assert!(check_renewal_shop_row_shape(0, 28).is_ok());
        for actual in [0, 27, 29] {
            assert!(matches!(
                check_renewal_shop_row_shape(3, actual),
                Err(RenewalShopSqlxLoadError::RowShape {
                    row,
                    expected,
                    actual: shape_actual,
                }) if row == 3
                    && expected == RENEWAL_SHOP_TABLE_QUERY_COLUMNS
                    && shape_actual == actual
            ));
        }
    }

    #[test]
    fn raw_cells_preserve_null_nul_and_non_utf8_bytes() {
        assert_eq!(
            byte_cell_to_renewal_shop_value(None),
            RenewalShopQueryValue::Null
        );
        let source = vec![0xff, 0, b'7', 0x80];
        assert_eq!(
            byte_cell_to_renewal_shop_value(Some(source.clone())),
            RenewalShopQueryValue::Bytes(source.clone())
        );
        assert_eq!(
            raw_cell_to_renewal_shop_value(Some(source.clone())),
            RenewalShopQueryValue::Bytes(source)
        );
    }

    #[test]
    fn empty_source_builds_an_empty_renewal_shop_section() {
        let limits = RenewalShopSectionLimits::new(0);
        let strict = build_renewal_shop_section_with_limits(&[], limits).unwrap();
        let legacy = build_renewal_shop_section_legacy_with_limits(&[], limits).unwrap();
        assert_eq!(strict, legacy);
        assert_eq!(strict.count, 0);
        assert!(strict.data.is_empty());
    }

    #[test]
    fn section_errors_stay_distinct_from_acquisition_errors() {
        let section = RenewalShopSectionError::TooManySourceRows {
            count: 2,
            maximum: 1,
        };
        let error = RenewalShopSqlxLoadError::from(section.clone());
        assert!(matches!(&error, RenewalShopSqlxLoadError::Section(source) if source == &section));
        assert!(error.to_string().contains("section construction failed"));
        assert!(Error::source(&error).is_some());
        assert!(
            Error::source(&RenewalShopSqlxLoadError::SourceLimitExceeded { maximum: 1 }).is_none()
        );
    }
}
