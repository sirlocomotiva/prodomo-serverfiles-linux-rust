//! `SQLx` acquisition adapter for the source-fixed `refine_proto` boot table.
//!
//! The exact query and `TABLE_POSTFIX` validation live in [`crate::postfix`].
//! The strict row and packed-section rules live in [`crate::refine`]. This
//! module only acquires the checked query through the bounded [`ConnectionPool`]
//! seam, checks the thirteen-column source shape, preserves fetch order, and
//! delegates decoding and encoding. The pool's `sqlx::raw_sql` no-bind path is the
//! source-compatible `MySQL` text projection, so numeric cells are read as
//! unchecked optional strings and then strictly parsed by `crate::refine`.
//! This adapter does not choose a boot profile, build a snapshot, populate a
//! cache, or turn a database failure into an empty table.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::postfix::{RefineProtoLoader, RefineProtoQueryRow};
use crate::refine::{
    build_refine_section_with_limits, RefineQueryValue, RefineSectionError,
    REFINE_TABLE_MAX_RECORDS, REFINE_TABLE_QUERY_COLUMNS,
};

/// A failure while acquiring or bounding the source-fixed refine table.
#[derive(Debug)]
pub enum RefineSqlxLoadError {
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
    /// A returned row did not have the thirteen-column shape promised by the
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
    Section(RefineSectionError),
}

impl fmt::Display for RefineSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => {
                write!(formatter, "refine_proto database query failed: {source}")
            }
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "refine_proto source-row limit {maximum} exceeds the maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => write!(
                formatter,
                "refine_proto source returned more than {maximum} rows"
            ),
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "refine_proto row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "refine_proto row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "refine_proto adapter could not allocate {requested} source row(s)"
            ),
            Self::Section(source) => {
                write!(
                    formatter,
                    "refine_proto section construction failed: {source}"
                )
            }
        }
    }
}

impl Error for RefineSqlxLoadError {
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

impl From<RefineSectionError> for RefineSqlxLoadError {
    fn from(source: RefineSectionError) -> Self {
        Self::Section(source)
    }
}

/// Execute the immutable query held by `loader` and build one refine section.
///
/// Acquisition is bounded by `loader.limits().max_records` before rows are
/// accumulated. A valid empty result is an empty section; a database error,
/// an extra row, a malformed result shape, or a decode failure is never
/// represented as an empty section. The source order returned by the pool is
/// retained exactly. `SQLx` materializes each cell before this adapter can
/// inspect it, so the cap is row-bounded, not byte-bounded, and schema-level
/// cell-size limits remain the database's responsibility. The configured
/// packed-byte limit is checked by the pure section builder after acquisition.
///
/// # Errors
///
/// Returns [`RefineSqlxLoadError::Database`] for pool/query failures,
/// `InvalidSourceLimit` for a cap above the `u16` wire count,
/// `SourceLimitExceeded` when the stream contains an extra row, typed
/// row-shape/decode/allocation errors for malformed SQL results, and
/// `Section` for strict NULL, numeric, or packed-byte policy failures.
pub async fn load_refine_proto_section_sqlx(
    pool: &ConnectionPool,
    loader: &RefineProtoLoader,
) -> Result<BootSection, RefineSqlxLoadError> {
    let limits = loader.limits();
    validate_source_limit(limits.max_records)?;

    let rows = pool
        .query_up_to(loader.query().as_str(), limits.max_records)
        .await
        .map_err(RefineSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(RefineSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_records,
        });
    };

    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        RefineSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        let actual = row.columns().len();
        check_refine_row_shape(row_index, actual)?;
        let mut cells = std::array::from_fn(|_| RefineQueryValue::Null);
        for (column, cell) in cells.iter_mut().enumerate() {
            // `query_up_to` uses `sqlx::raw_sql`, so MySQL returns
            // the source-compatible text projection. `try_get_unchecked`
            // intentionally bypasses only SQLx's type-metadata compatibility
            // check: legacy MYSQL_ROW exposes numeric cells as text too.
            // Decode still checks the index and UTF-8 validity; NULL remains
            // NULL and no value is normalized or lossily converted.
            let value = row
                .try_get_unchecked::<Option<String>, usize>(column)
                .map_err(|source| RefineSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            *cell = text_cell_to_refine_value(value);
        }
        source_rows.push(RefineProtoQueryRow::from_typed_columns(cells));
    }

    build_refine_section_with_limits(&source_rows, limits).map_err(RefineSqlxLoadError::Section)
}

/// Compatibility spelling matching the existing generic loader name.
///
/// # Errors
///
/// Returns the same typed errors as [`load_refine_proto_section_sqlx`].
pub async fn load_refine_section_sqlx(
    pool: &ConnectionPool,
    loader: &RefineProtoLoader,
) -> Result<BootSection, RefineSqlxLoadError> {
    load_refine_proto_section_sqlx(pool, loader).await
}

fn validate_source_limit(maximum: usize) -> Result<(), RefineSqlxLoadError> {
    if maximum > REFINE_TABLE_MAX_RECORDS {
        Err(RefineSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: REFINE_TABLE_MAX_RECORDS,
        })
    } else {
        Ok(())
    }
}

/// Convert one already-decoded SQL text cell without changing its value.
fn text_cell_to_refine_value(value: Option<String>) -> RefineQueryValue {
    value.map_or(RefineQueryValue::Null, RefineQueryValue::Text)
}

fn check_refine_row_shape(row: usize, actual: usize) -> Result<(), RefineSqlxLoadError> {
    if actual == REFINE_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(RefineSqlxLoadError::RowShape {
            row,
            expected: REFINE_TABLE_QUERY_COLUMNS,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postfix::{RefineProtoQuery, TablePostfix};
    use crate::refine::RefineSectionLimits;

    #[test]
    fn adapter_uses_the_checked_refine_query_and_representable_limit() {
        let postfix = TablePostfix::parse("_test").unwrap();
        let loader = RefineProtoLoader::new(&postfix, RefineSectionLimits::new(7)).unwrap();
        assert_eq!(
            loader.query().as_str(),
            RefineProtoQuery::new(&postfix).unwrap().as_str()
        );
        assert_eq!(
            loader.query().as_str(),
            "SELECT id, cost, prob, vnum0, count0, vnum1, count1, vnum2, count2,  vnum3, count3, vnum4, count4 FROM refine_proto_test"
        );
        assert!(loader.limits().max_records <= REFINE_TABLE_MAX_RECORDS);
    }

    #[test]
    fn text_cells_keep_null_and_unmodified_numeric_text() {
        assert!(matches!(
            text_cell_to_refine_value(None),
            RefineQueryValue::Null
        ));
        assert!(matches!(
            text_cell_to_refine_value(Some("00123".to_owned())),
            RefineQueryValue::Text(value) if value == "00123"
        ));
    }

    #[test]
    fn source_cap_and_shape_failures_are_not_empty_sections() {
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(REFINE_TABLE_MAX_RECORDS).is_ok());
        assert!(matches!(
            validate_source_limit(REFINE_TABLE_MAX_RECORDS + 1),
            Err(RefineSqlxLoadError::InvalidSourceLimit { .. })
        ));

        let error = RefineSqlxLoadError::SourceLimitExceeded { maximum: 7 };
        assert!(error.to_string().contains("more than 7 rows"));
        assert!(matches!(check_refine_row_shape(0, 13), Ok(())));
        for actual in [12, 14] {
            let Err(RefineSqlxLoadError::RowShape {
                row,
                expected,
                actual: shape_actual,
            }) = check_refine_row_shape(2, actual)
            else {
                panic!("expected row-shape error for {actual}");
            };
            assert_eq!(row, 2);
            assert_eq!(expected, 13);
            assert_eq!(shape_actual, actual);
        }
        let invalid = RefineSqlxLoadError::InvalidSourceLimit {
            maximum: REFINE_TABLE_MAX_RECORDS + 1,
            limit: REFINE_TABLE_MAX_RECORDS,
        };
        assert!(invalid.to_string().contains("exceeds"));
    }
}
