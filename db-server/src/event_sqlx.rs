//! `SQLx` acquisition adapter for the source-fixed event boot table.
//!
//! The pure query, row, and section rules live in [`crate::event`]. This
//! module only acquires the seven positional cells and delegates strict
//! decoding and bounded section construction. Positional extraction is
//! required because the legacy `UNIX_TIMESTAMP(...)` expressions do not
//! guarantee database-generated column names.
//!
//! The pool operation is the verified `sqlx::raw_sql` no-bind text-protocol path. It consumes
//! a bounded stream before rows are accumulated, and each cell is read
//! positionally as an unchecked optional raw-byte value. This preserves the
//! source bytes, `NULL`, fetch order, and duplicate rows until the pure decoder
//! applies its strict policy. A database failure, a row beyond the cap, a
//! wrong-width row, a decode failure, and a valid empty result remain distinct.
//! No boot profile, snapshot, event cache, status update, or event-manager
//! state is changed here.

use std::error::Error;
use std::fmt;

use db::sqlx::Row;
use db::{ConnectionPool, DbError};
use protocol::db_boot::BootSection;

use crate::event::{
    build_event_section, EventQuery, EventQueryRow, EventQueryValue, EventSectionError,
    EventSectionLimits, EVENT_QUERY_COLUMNS, EVENT_TABLE_MAX_RECORDS,
};

/// A failure while acquiring a bounded event section through `SQLx`.
#[derive(Debug)]
pub enum EventSqlxLoadError {
    /// The pool could not execute the checked query after its retry policy.
    Database(DbError),
    /// The configured source-row cap exceeds the legacy `u16` boot count.
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
    /// A row did not have the seven-column shape promised by the query.
    RowShape {
        /// Zero-based row index.
        row: usize,
        /// Required column count.
        expected: usize,
        /// Actual column count.
        actual: usize,
    },
    /// A source cell could not be decoded as optional raw bytes.
    ColumnDecode {
        /// Zero-based row index.
        row: usize,
        /// Zero-based source column index.
        column: usize,
        /// Underlying `SQLx` error.
        source: db::sqlx::Error,
    },
    /// The source-row or per-row value storage could not reserve its bounded
    /// result.
    AllocationFailed {
        /// Number of rows returned by the bounded stream.
        requested: usize,
    },
    /// The pure event section policy rejected a row, count, byte, or wire limit.
    Section(EventSectionError),
}

impl fmt::Display for EventSqlxLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(source) => write!(formatter, "event database query failed: {source}"),
            Self::InvalidSourceLimit { maximum, limit } => write!(
                formatter,
                "event source-row limit {maximum} exceeds the maximum {limit}"
            ),
            Self::SourceLimitExceeded { maximum } => {
                write!(formatter, "event source returned more than {maximum} rows")
            }
            Self::RowShape {
                row,
                expected,
                actual,
            } => write!(
                formatter,
                "event row {row} has {actual} columns; expected {expected}"
            ),
            Self::ColumnDecode {
                row,
                column,
                source,
            } => write!(
                formatter,
                "event row {row} column {column} decode failed: {source}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "event adapter could not allocate {requested} source row(s)"
            ),
            Self::Section(source) => {
                write!(formatter, "event section construction failed: {source}")
            }
        }
    }
}

impl Error for EventSqlxLoadError {
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

impl From<EventSectionError> for EventSqlxLoadError {
    fn from(source: EventSectionError) -> Self {
        Self::Section(source)
    }
}

/// Execute one checked event query and build its bounded boot section.
///
/// The source-row cap is checked before the pool call and is also constrained
/// to the `u16` boot count. The pool's `sqlx::raw_sql` no-bind query returns the source-compatible
/// `MySQL` projection; positional unchecked optional-byte reads bypass only
/// `SQLx`'s numeric metadata check. The pure builder enforces packed-byte limits
/// after acquisition, and schema-level limits for individual event-type cells
/// remain the database's responsibility.
///
/// # Errors
///
/// Returns [`EventSqlxLoadError::Database`] for pool/query failures,
/// [`EventSqlxLoadError::InvalidSourceLimit`] for a cap that cannot fit the
/// boot count, [`EventSqlxLoadError::SourceLimitExceeded`] for an extra row,
/// row-shape/decode/allocation errors for malformed SQL results, and
/// [`EventSqlxLoadError::Section`] for strict event-row and byte-limit errors.
pub async fn load_event_section_sqlx(
    pool: &ConnectionPool,
    query: &EventQuery,
    limits: EventSectionLimits,
) -> Result<BootSection, EventSqlxLoadError> {
    validate_source_limit(limits.max_source_rows)?;

    let rows = pool
        .query_up_to(query.as_str(), limits.max_source_rows)
        .await
        .map_err(EventSqlxLoadError::Database)?;
    let Some(rows) = rows else {
        return Err(EventSqlxLoadError::SourceLimitExceeded {
            maximum: limits.max_source_rows,
        });
    };

    let mut source_rows = Vec::new();
    source_rows.try_reserve_exact(rows.len()).map_err(|_| {
        EventSqlxLoadError::AllocationFailed {
            requested: rows.len(),
        }
    })?;

    for (row_index, row) in rows.into_iter().enumerate() {
        let actual = row.columns().len();
        check_event_row_shape(row_index, actual)?;

        let mut values = Vec::new();
        values.try_reserve_exact(EVENT_QUERY_COLUMNS).map_err(|_| {
            EventSqlxLoadError::AllocationFailed {
                requested: EVENT_QUERY_COLUMNS,
            }
        })?;
        for column in 0..EVENT_QUERY_COLUMNS {
            // `query_up_to` uses `sqlx::raw_sql`, so the MySQL
            // result follows the source-compatible raw-byte projection. The
            // unchecked type read bypasses only SQLx's numeric metadata check;
            // the index, NULL state, and raw bytes remain explicit. Numeric
            // UTF-8 and event-type policy stays in the pure decoder.
            let value = row
                .try_get_unchecked::<Option<Vec<u8>>, usize>(column)
                .map_err(|source| EventSqlxLoadError::ColumnDecode {
                    row: row_index,
                    column,
                    source,
                })?;
            values.push(byte_cell_to_event_value(value));
        }
        source_rows.push(EventQueryRow::new(values));
    }

    build_event_section(&source_rows, limits).map_err(EventSqlxLoadError::Section)
}

/// Validate a source-row cap before querying or accumulating rows.
///
/// The cap is bounded by the legacy `u16` boot count. This check is kept
/// separate from the pure section builder so an invalid cap cannot reach SQL.
///
/// # Errors
///
/// Returns [`EventSqlxLoadError::InvalidSourceLimit`] when `maximum` cannot
/// fit the legacy event section count.
fn validate_source_limit(maximum: usize) -> Result<(), EventSqlxLoadError> {
    if maximum > EVENT_TABLE_MAX_RECORDS {
        Err(EventSqlxLoadError::InvalidSourceLimit {
            maximum,
            limit: EVENT_TABLE_MAX_RECORDS,
        })
    } else {
        Ok(())
    }
}

/// Convert one already-decoded SQL byte cell without changing its value.
fn byte_cell_to_event_value(value: Option<Vec<u8>>) -> EventQueryValue {
    value.map_or(EventQueryValue::Null, EventQueryValue::Bytes)
}

/// Check the fixed seven-column result shape before positional extraction.
///
/// # Errors
///
/// Returns [`EventSqlxLoadError::RowShape`] when the result row does not have
/// the columns promised by the checked event query.
fn check_event_row_shape(row: usize, actual: usize) -> Result<(), EventSqlxLoadError> {
    if actual == EVENT_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(EventSqlxLoadError::RowShape {
            row,
            expected: EVENT_QUERY_COLUMNS,
            actual,
        })
    }
}

/// Compatibility spelling for callers that want the event loader without the
/// table-specific suffix.
///
/// # Errors
///
/// Returns the same typed errors as [`load_event_section_sqlx`].
pub async fn load_event_section(
    pool: &ConnectionPool,
    query: &EventQuery,
    limits: EventSectionLimits,
) -> Result<BootSection, EventSqlxLoadError> {
    load_event_section_sqlx(pool, query, limits).await
}

/// Compatibility spelling matching the source table name.
///
/// # Errors
///
/// Returns the same typed errors as [`load_event_section_sqlx`].
pub async fn load_event_table_section_sqlx(
    pool: &ConnectionPool,
    query: &EventQuery,
    limits: EventSectionLimits,
) -> Result<BootSection, EventSqlxLoadError> {
    load_event_section_sqlx(pool, query, limits).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EVENT_TABLE_MAX_SECTION_BYTES;
    use crate::event::{decode_event_query_row, EventQueryValue};
    use crate::postfix::TablePostfix;
    use protocol::db_records::EVENT_TABLE_WIRE_SIZE;

    fn bytes(value: &[u8]) -> EventQueryValue {
        EventQueryValue::bytes(value)
    }

    fn valid_row() -> EventQueryRow {
        EventQueryRow::from_typed_columns([
            bytes(b"17"),
            bytes(b"birthday"),
            bytes(b"-10"),
            bytes(b"20"),
            bytes(b"-7"),
            bytes(b"99"),
            bytes(b"1"),
        ])
    }

    #[test]
    fn adapter_contract_keeps_exact_query_and_representable_cap() {
        let postfix = TablePostfix::parse("_prod").unwrap();
        let query = EventQuery::new(&postfix).unwrap();
        assert_eq!(
            query.as_str(),
            "SELECT id, type, UNIX_TIMESTAMP(start), UNIX_TIMESTAMP(end), value0, value1, completed FROM event_prod ORDER BY start"
        );
        assert!(validate_source_limit(0).is_ok());
        assert!(validate_source_limit(EVENT_TABLE_MAX_RECORDS).is_ok());
        assert!(matches!(
            validate_source_limit(EVENT_TABLE_MAX_RECORDS + 1),
            Err(EventSqlxLoadError::InvalidSourceLimit { .. })
        ));
    }

    #[test]
    fn acquisition_shape_is_checked_before_positional_reads() {
        assert!(check_event_row_shape(2, EVENT_QUERY_COLUMNS).is_ok());
        for actual in [0, EVENT_QUERY_COLUMNS - 1, EVENT_QUERY_COLUMNS + 1] {
            let Err(EventSqlxLoadError::RowShape {
                row,
                expected,
                actual: shape_actual,
            }) = check_event_row_shape(2, actual)
            else {
                panic!("expected row-shape error for {actual}");
            };
            assert_eq!(row, 2);
            assert_eq!(expected, EVENT_QUERY_COLUMNS);
            assert_eq!(shape_actual, actual);
        }
    }

    #[test]
    fn byte_cells_preserve_raw_source_and_null() {
        assert!(matches!(
            byte_cell_to_event_value(None),
            EventQueryValue::Null
        ));
        let source = vec![0xff, b'a', 0, b'b'];
        assert_eq!(
            byte_cell_to_event_value(Some(source.clone())),
            EventQueryValue::Bytes(source.clone())
        );

        let row = EventQueryRow::from_typed_columns([
            byte_cell_to_event_value(Some(b"0017".to_vec())),
            byte_cell_to_event_value(Some(source.clone())),
            byte_cell_to_event_value(Some(b"-0010".to_vec())),
            byte_cell_to_event_value(Some(b"0020".to_vec())),
            byte_cell_to_event_value(Some(b"-0007".to_vec())),
            byte_cell_to_event_value(Some(b"0099".to_vec())),
            byte_cell_to_event_value(Some(b"01".to_vec())),
        ]);
        assert_eq!(row.columns()[0], EventQueryValue::Bytes(b"0017".to_vec()));
        assert_eq!(row.columns()[1], EventQueryValue::Bytes(source));
        let record = decode_event_query_row(&row).unwrap();
        assert_eq!(record.id, 17);
        assert_eq!(&record.event_type[..2], &[0xff, b'a']);
        assert!(record.event_type[2..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn source_order_duplicates_and_post_acquisition_byte_limit_are_explicit() {
        let row = valid_row();
        let section = build_event_section(
            std::slice::from_ref(&row),
            EventSectionLimits::with_limits(1, 1, EVENT_TABLE_MAX_SECTION_BYTES),
        )
        .unwrap();
        assert_eq!(section.count, 1);

        let duplicate_section = build_event_section(
            &[row.clone(), row.clone()],
            EventSectionLimits::with_limits(2, 2, EVENT_TABLE_MAX_SECTION_BYTES),
        )
        .unwrap();
        assert_eq!(duplicate_section.count, 2);
        let duplicate_records = protocol::db_boot::decode_event_table_section(
            &duplicate_section,
            protocol::db_boot::BootFeatureProfile::new(false, true, false),
        )
        .unwrap();
        assert_eq!(
            duplicate_records
                .iter()
                .map(|record| record.id)
                .collect::<Vec<_>>(),
            [17, 17]
        );

        assert!(matches!(
            build_event_section(
                std::slice::from_ref(&row),
                EventSectionLimits::with_limits(1, 1, EVENT_TABLE_WIRE_SIZE - 1),
            ),
            Err(EventSectionError::DataTooLarge { .. })
        ));
    }
}
