//! `SQLx` acquisition adapter for the optional premium private-shop price table.
//!
//! The checked query, positional row type, limits, and section encoding live in
//! [`crate::market_price`]. This module only performs one bounded read through
//! [`ConnectionPool::query_up_to`], preserves the three raw positional cells,
//! and delegates to either the strict or explicitly named legacy pure builder.
//! It does not select a boot profile, compose a snapshot, install a cache,
//! mutate a manager, or provide transport behavior.
//!
//! The pool uses its no-bind `sqlx::raw_sql` text-protocol path. The returned
//! rows retain fetch order, duplicates, SQL `NULL`, embedded NUL bytes, and
//! non-UTF-8 bytes until the selected pure row policy runs. An empty query
//! result is valid and produces an empty 16-byte-record
//! `PremiumMarketPrice` section.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::market_price::{
    build_market_price_section_legacy_with_limits, build_market_price_section_with_limits,
    MarketPriceQuery, MarketPriceQueryRow, MarketPriceSectionError, MarketPriceSectionLimits,
    MARKET_PRICE_MAX_RECORDS, MARKET_PRICE_QUERY_COLUMN_COUNT,
};

/// A failure while acquiring or bounding a premium market-price section.
///
/// Database, cap, row-shape, raw-cell decode, allocation, and pure section
/// failures remain distinct. No database or decode failure is converted into
/// an empty section.
#[derive(Debug)]
pub enum MarketPriceSqlxLoadError {
    /// The pool could not execute the checked query after its retry policy.
    Database(DbError),
    /// The configured source-row cap exceeds the representable market-price
    /// output-record ceiling.
    InvalidSourceLimit {
        /// Configured maximum number of source rows.
        maximum: usize,
        /// Maximum permitted source-row cap.
        limit: usize,
    },
    /// The bounded stream observed a row beyond the configured source cap.
    SourceLimitExceeded {
        /// Configured maximum number of source rows.
        maximum: usize,
    },
    /// A source row did not have the exact three-column query shape.
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
        /// Underlying `SQLx` decode error.
        source: db::sqlx::Error,
    },
    /// The bounded source-row vector could not reserve its requested capacity.
    AllocationFailed {
        /// Number of elements requested by the failed reservation.
        requested: usize,
    },
    /// The selected pure market-price policy rejected the acquired rows.
    Section(MarketPriceSectionError),
}

impl fmt::Display for MarketPriceSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => {
                write!(formatter, "market-price database query failed: {source}")
            }
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "market-price source-row limit {maximum} exceeds the maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => write!(
                formatter,
                "market-price source returned more than {maximum} rows"
            ),
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "market-price row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column: source_column,
                source,
            } => write!(
                formatter,
                "market-price row {row} column {source_column} raw-byte decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "market-price adapter could not allocate {requested} bounded row(s)"
            ),
            Self::Section(source) => {
                write!(
                    formatter,
                    "market-price section construction failed: {source}"
                )
            }
        }
    }
}

impl Error for MarketPriceSqlxLoadError {
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

impl From<MarketPriceSectionError> for MarketPriceSqlxLoadError {
    fn from(source: MarketPriceSectionError) -> Self {
        Self::Section(source)
    }
}

/// Compatibility spelling for callers that omit the `Load` infix.
pub type MarketPriceSqlxError = MarketPriceSqlxLoadError;

/// Acquire raw rows from the checked market-price query.
///
/// The returned rows preserve source order, duplicate vnums, SQL `NULL`, and
/// every raw byte cell. The source cap is checked before the one pool call.
/// A valid zero-row result is returned as an empty vector.
///
/// # Errors
///
/// Returns [`MarketPriceSqlxLoadError`] for an invalid source cap, a database
/// failure, an extra source row, a wrong-width row, a positional raw-byte
/// decode failure, or a failed fallible row-vector reservation.
pub async fn load_market_price_rows_sqlx(
    pool: &ConnectionPool,
    query: &MarketPriceQuery,
    limits: MarketPriceSectionLimits,
) -> Result<Vec<MarketPriceQueryRow>, MarketPriceSqlxLoadError> {
    validate_source_limit(limits.max_source_rows)?;

    // This is the adapter's only pool operation. `None` means the bounded
    // stream observed a row after `max_source_rows`; it is not an empty result.
    let bounded = pool
        .query_up_to(query.as_str(), limits.max_source_rows)
        .await
        .map_err(MarketPriceSqlxLoadError::Database)?;
    let Some(rows) = bounded else {
        return Err(MarketPriceSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_source_rows,
        });
    };

    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        MarketPriceSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        check_market_price_row_shape(row_index, row.columns().len())?;

        let mut columns: [Option<Vec<u8>>; MARKET_PRICE_QUERY_COLUMN_COUNT] =
            std::array::from_fn(|_| None);
        for (column, value) in columns.iter_mut().enumerate() {
            // The exact row width is checked above. The unchecked positional
            // read bypasses only SQLx numeric type metadata; the index, decode
            // result, NULL state, and every raw byte remain explicit.
            *value = row
                .try_get_unchecked::<Option<Vec<u8>>, usize>(column)
                .map_err(|source| MarketPriceSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
        }
        source_rows.push(market_price_row_from_raw_columns(columns));
    }

    Ok(source_rows)
}

/// Acquire raw rows and strictly build one market-price boot section.
///
/// Rows reach the pure builder in source order, including duplicates. A valid
/// empty source builds an empty section. Scalar conversion and all output-byte
/// policy remain in [`crate::market_price`].
///
/// # Errors
///
/// Returns [`MarketPriceSqlxLoadError`] for an acquisition failure or wraps a
/// strict market-price row/section failure in
/// [`MarketPriceSqlxLoadError::Section`].
pub async fn load_market_price_section_sqlx(
    pool: &ConnectionPool,
    query: &MarketPriceQuery,
    limits: MarketPriceSectionLimits,
) -> Result<BootSection, MarketPriceSqlxLoadError> {
    let rows = load_market_price_rows_sqlx(pool, query, limits).await?;
    build_market_price_section_with_limits(&rows, limits).map_err(MarketPriceSqlxLoadError::Section)
}

/// Acquire the same raw rows and use the explicit legacy compatibility policy.
///
/// This function does not select a profile, perform maintenance SQL, or alter
/// source rows. The pure legacy builder owns all source-compatible conversion
/// behavior.
///
/// # Errors
///
/// Returns the same acquisition errors as
/// [`load_market_price_section_sqlx`] and wraps a legacy row/section failure
/// in [`MarketPriceSqlxLoadError::Section`].
pub async fn load_market_price_section_legacy_sqlx(
    pool: &ConnectionPool,
    query: &MarketPriceQuery,
    limits: MarketPriceSectionLimits,
) -> Result<BootSection, MarketPriceSqlxLoadError> {
    let rows = load_market_price_rows_sqlx(pool, query, limits).await?;
    build_market_price_section_legacy_with_limits(&rows, limits)
        .map_err(MarketPriceSqlxLoadError::Section)
}

/// Validate a source-row cap before SQL execution or row accumulation.
///
/// # Errors
///
/// Returns [`MarketPriceSqlxLoadError::InvalidSourceLimit`] when `maximum`
/// exceeds the global market-price record ceiling.
pub fn validate_source_limit(maximum: usize) -> Result<(), MarketPriceSqlxLoadError> {
    if maximum > MARKET_PRICE_MAX_RECORDS {
        Err(MarketPriceSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: MARKET_PRICE_MAX_RECORDS,
        })
    } else {
        Ok(())
    }
}

/// Check the exact three-column result shape before positional extraction.
///
/// # Errors
///
/// Returns [`MarketPriceSqlxLoadError::RowShape`] when `actual` differs from
/// [`MARKET_PRICE_QUERY_COLUMN_COUNT`].
pub fn check_market_price_row_shape(
    row: usize,
    actual: usize,
) -> Result<(), MarketPriceSqlxLoadError> {
    if actual == MARKET_PRICE_QUERY_COLUMN_COUNT {
        Ok(())
    } else {
        Err(MarketPriceSqlxLoadError::RowShape {
            row,
            expected: MARKET_PRICE_QUERY_COLUMN_COUNT,
            actual,
        })
    }
}

fn market_price_row_from_raw_columns(
    columns: [Option<Vec<u8>>; MARKET_PRICE_QUERY_COLUMN_COUNT],
) -> MarketPriceQueryRow {
    MarketPriceQueryRow::from_cells(columns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::BootSectionKind;

    fn valid_row() -> MarketPriceQueryRow {
        MarketPriceQueryRow::from_cells([
            Some(b"17".to_vec()),
            Some(b"-200".to_vec()),
            Some(b"7".to_vec()),
        ])
    }

    #[test]
    fn source_cap_and_exact_row_shape_are_checked() {
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(MARKET_PRICE_MAX_RECORDS).is_ok());
        assert!(matches!(
            validate_source_limit(MARKET_PRICE_MAX_RECORDS + 1),
            Err(MarketPriceSqlxLoadError::InvalidSourceLimit {
                maximum,
                limit,
            }) if maximum == MARKET_PRICE_MAX_RECORDS + 1
                && limit == MARKET_PRICE_MAX_RECORDS
        ));

        assert_eq!(MARKET_PRICE_QUERY_COLUMN_COUNT, 3);
        assert!(check_market_price_row_shape(2, MARKET_PRICE_QUERY_COLUMN_COUNT).is_ok());
        for actual in [0, 2, 4] {
            assert!(matches!(
                check_market_price_row_shape(2, actual),
                Err(MarketPriceSqlxLoadError::RowShape {
                    row,
                    expected,
                    actual: shape_actual,
                }) if row == 2
                    && expected == MARKET_PRICE_QUERY_COLUMN_COUNT
                    && shape_actual == actual
            ));
        }
    }

    #[test]
    fn raw_cells_preserve_null_order_nul_and_non_utf8_bytes() {
        let columns = [Some(vec![0xff, 0, b'1']), None, Some(vec![0x80, b'2', 0])];
        let row = market_price_row_from_raw_columns(columns.clone());
        assert_eq!(row.into_cells(), columns);
    }

    #[test]
    fn empty_source_builds_an_empty_16_byte_record_section() {
        let limits = MarketPriceSectionLimits {
            max_source_rows: 0,
            max_records: 0,
            max_data_bytes: 0,
        };
        let strict = build_market_price_section_with_limits(&[], limits).unwrap();
        let legacy = build_market_price_section_legacy_with_limits(&[], limits).unwrap();
        assert_eq!(strict, legacy);
        assert_eq!(strict.kind, BootSectionKind::PremiumMarketPrice);
        assert_eq!(strict.record_size, 16);
        assert_eq!(strict.count, 0);
        assert!(strict.data.is_empty());
    }

    #[test]
    fn typed_errors_have_stable_display_and_source_semantics() {
        let database =
            MarketPriceSqlxLoadError::Database(DbError::InvalidRetryCount { max_retries: 0 });
        assert!(database.to_string().contains("database query failed"));
        assert!(Error::source(&database).is_some());

        let decode = MarketPriceSqlxLoadError::ColumnDecode {
            row: 4,
            column: 2,
            source: db::sqlx::Error::RowNotFound,
        };
        assert!(decode.to_string().contains("row 4 column 2"));
        assert!(Error::source(&decode).is_some());

        let shape = MarketPriceSqlxLoadError::RowShape {
            row: 1,
            expected: MARKET_PRICE_QUERY_COLUMN_COUNT,
            actual: 2,
        };
        assert!(shape.to_string().contains("expected 3"));
        assert!(Error::source(&shape).is_none());

        let allocation = MarketPriceSqlxLoadError::AllocationFailed { requested: 9 };
        assert!(allocation.to_string().contains("allocate 9"));
        assert!(Error::source(&allocation).is_none());
    }

    #[test]
    fn pure_section_failure_converts_to_the_adapter_error() {
        let section_error = build_market_price_section_with_limits(
            &[valid_row()],
            MarketPriceSectionLimits {
                max_source_rows: 1,
                max_records: 1,
                max_data_bytes: 0,
            },
        )
        .unwrap_err();
        let error = MarketPriceSqlxLoadError::from(section_error);
        assert!(matches!(&error, MarketPriceSqlxLoadError::Section(_)));
        assert!(error.to_string().contains("section construction failed"));
        assert!(Error::source(&error).is_some());
    }
}
