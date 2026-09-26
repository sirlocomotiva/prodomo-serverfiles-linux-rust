//! The `event` table rule.
//!
//! The legacy loader in `server/server/db/ClientManagerBoot.cpp:1693-1734`
//! runs one statement under `__EVENT_MANAGER__`:
//!
//! ```text
//! SELECT id, type, UNIX_TIMESTAMP(start), UNIX_TIMESTAMP(end), value0, value1, completed FROM event%s ORDER BY start
//! ```
//!
//! It then copies seven positional cells into a zeroed packed `TEventTable`.
//! `NULL` and malformed source values are errors rather than zero-filled
//! records.  The `type` field retains raw bytes, truncates at the legacy
//! 63-byte content limit, stops at the first NUL, and zero-fills the 64-byte
//! destination.  The legacy loader accepts zero rows, so an empty source is an
//! empty table.

use std::error::Error;
use std::fmt;
use std::num::IntErrorKind;

use crate::records::{EventTableRecord, EVENT_TYPE_BYTES};

/// Legacy source table name.
pub const EVENT_TABLE: &str = "event";

/// The legacy statement, kept to document the column order.  The two
/// timestamp expressions are positional result columns.
pub const EVENT_LEGACY_QUERY: &str =
    "SELECT id, type, UNIX_TIMESTAMP(start), UNIX_TIMESTAMP(end), value0, value1, completed FROM event ORDER BY start";

/// Number of columns in [`EVENT_LEGACY_QUERY`].
pub const EVENT_QUERY_COLUMNS: usize = 7;

/// Default record cap: the legacy boot stream counted records in a `WORD`,
/// so no legacy table held more.
pub const EVENT_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Maximum event-type content bytes copied by legacy `strlcpy`.
pub const EVENT_TYPE_MAX_CONTENT_BYTES: usize = EVENT_TYPE_BYTES - 1;

/// Stable source-order column names/expressions for diagnostics.
pub const EVENT_QUERY_COLUMN_NAMES: [&str; EVENT_QUERY_COLUMNS] = [
    "id",
    "type",
    "UNIX_TIMESTAMP(start)",
    "UNIX_TIMESTAMP(end)",
    "value0",
    "value1",
    "completed",
];

/// A source cell retained before strict event-row decoding.
///
/// `Bytes` is the lossless representation used by the `SQLx` adapter. `Text` is
/// convenient for callers that already have UTF-8 SQL text. `Null` and
/// `Error` remain distinct until the row decoder makes an explicit decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventQueryValue {
    /// A text value supplied by a caller.
    Text(String),
    /// Raw bytes supplied by a database adapter.
    Bytes(Vec<u8>),
    /// SQL `NULL`.
    Null,
    /// A source extraction or conversion error.
    Error(String),
}

impl EventQueryValue {
    /// Construct a text cell.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    /// Construct a raw-byte cell.
    #[must_use]
    pub fn bytes(value: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(value.into())
    }

    /// Construct a SQL `NULL` cell.
    #[must_use]
    pub const fn null() -> Self {
        Self::Null
    }

    /// Construct a source error cell.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error(message.into())
    }
}

impl From<String> for EventQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for EventQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for EventQueryValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

/// One source-shaped event row, retaining its columns and source order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventQueryRow {
    columns: Vec<EventQueryValue>,
}

impl EventQueryRow {
    /// Construct a row while retaining the supplied column count and order.
    ///
    /// Use [`Self::try_new`] when the source contract should validate the
    /// seven-column shape immediately.
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = EventQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Construct and validate a seven-column row.
    ///
    /// # Errors
    ///
    /// Returns [`EventRowError::ColumnCount`] for a wrong-width row.
    pub fn try_new<I>(columns: I) -> Result<Self, EventRowError>
    where
        I: IntoIterator<Item = EventQueryValue>,
    {
        let row = Self::new(columns);
        if row.columns.len() != EVENT_QUERY_COLUMNS {
            return Err(EventRowError::ColumnCount {
                expected: EVENT_QUERY_COLUMNS,
                actual: row.columns.len(),
            });
        }
        Ok(row)
    }

    /// Construct a row from a statically sized source column set.
    #[must_use]
    pub fn from_typed_columns(columns: [EventQueryValue; EVENT_QUERY_COLUMNS]) -> Self {
        Self::new(columns)
    }

    /// Borrow all source columns in query order.
    #[must_use]
    pub fn columns(&self) -> &[EventQueryValue] {
        &self.columns
    }

    /// Consume the row and return its columns in source order.
    #[must_use]
    pub fn into_columns(self) -> Vec<EventQueryValue> {
        self.columns
    }

    /// Return the supplied column count.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

impl From<[EventQueryValue; EVENT_QUERY_COLUMNS]> for EventQueryRow {
    fn from(columns: [EventQueryValue; EVENT_QUERY_COLUMNS]) -> Self {
        Self::from_typed_columns(columns)
    }
}

impl TryFrom<Vec<EventQueryValue>> for EventQueryRow {
    type Error = EventRowError;

    fn try_from(columns: Vec<EventQueryValue>) -> Result<Self, Self::Error> {
        Self::try_new(columns)
    }
}

/// A strict event-row error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventRowError {
    /// The row did not contain exactly seven columns.
    ColumnCount {
        /// Required column count.
        expected: usize,
        /// Supplied column count.
        actual: usize,
    },
    /// A required source cell was SQL `NULL`.
    Null {
        /// Zero-based query column index.
        column: usize,
    },
    /// A source cell could not be obtained or converted.
    Source {
        /// Zero-based query column index.
        column: usize,
        /// Source diagnostic.
        message: String,
    },
    /// A numeric cell was not a strict decimal integer.
    InvalidNumber {
        /// Zero-based query column index.
        column: usize,
        /// Original value represented as UTF-8 for diagnostics.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A syntactically valid integer did not fit its target type.
    NumberOverflow {
        /// Zero-based query column index.
        column: usize,
        /// Original value represented as UTF-8 for diagnostics.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
}

impl EventRowError {
    /// Return a stable source name for a zero-based column index.
    #[must_use]
    pub const fn column_name(index: usize) -> &'static str {
        match index {
            0 => "id",
            1 => "type",
            2 => "UNIX_TIMESTAMP(start)",
            3 => "UNIX_TIMESTAMP(end)",
            4 => "value0",
            5 => "value1",
            6 => "completed",
            _ => "unknown",
        }
    }
}

impl fmt::Display for EventRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => write!(
                formatter,
                "event query row has {actual} columns; expected {expected}"
            ),
            Self::Null { column } => write!(
                formatter,
                "event query column {} is NULL",
                Self::column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "event query column {} could not be read: {message}",
                Self::column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "event query column {} value {value:?} is not a strict {target}",
                Self::column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "event query column {} value {value:?} overflows {target}",
                Self::column_name(*column)
            ),
        }
    }
}

impl Error for EventRowError {}

/// Strictly decode one source-shaped event row.
///
/// # Errors
///
/// Returns [`EventRowError`] for a wrong width, `NULL`, source error, malformed
/// number, or numeric overflow. The event type is copied as bytes and follows
/// the legacy first-NUL/63-byte-content truncation policy.
pub fn decode_event_query_row(row: &EventQueryRow) -> Result<EventTableRecord, EventRowError> {
    if row.columns.len() != EVENT_QUERY_COLUMNS {
        return Err(EventRowError::ColumnCount {
            expected: EVENT_QUERY_COLUMNS,
            actual: row.columns.len(),
        });
    }

    let id = decode_u32(&row.columns[0], 0)?;
    let event_type = decode_event_type(&row.columns[1], 1)?;
    let start_time = decode_i32(&row.columns[2], 2)?;
    let end_time = decode_i32(&row.columns[3], 3)?;
    let value0 = decode_i32(&row.columns[4], 4)?;
    let value1 = decode_i32(&row.columns[5], 5)?;
    let completed = decode_completed(&row.columns[6], 6)?;

    Ok(EventTableRecord {
        id,
        event_type,
        start_time,
        end_time,
        value0,
        value1,
        completed,
    })
}

/// The record cap checked before a event table is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventLimits {
    /// Maximum number of source rows accepted.
    pub max_records: usize,
}

impl EventLimits {
    /// Limits with a caller-selected row cap.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self { max_records }
    }
}

impl Default for EventLimits {
    fn default() -> Self {
        Self::new(EVENT_TABLE_MAX_RECORDS)
    }
}

/// A checked failure while building the event table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventTableError {
    /// The source supplied more rows than the caller allowed.
    TooManyRecords {
        /// Supplied row count.
        count: usize,
        /// Configured row limit.
        maximum: usize,
    },
    /// The output could not reserve its checked length.
    AllocationFailed {
        /// Requested record count.
        requested: usize,
    },
    /// A source row could not be decoded.
    Row {
        /// Zero-based source row index.
        index: usize,
        /// Row error.
        source: EventRowError,
    },
}

impl fmt::Display for EventTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "event table has {count} rows; configured limit is {maximum}"
            ),
            Self::AllocationFailed { requested } => {
                write!(formatter, "event allocation of {requested} records failed")
            }
            Self::Row { index, source } => {
                write!(formatter, "event row {index} is invalid: {source}")
            }
        }
    }
}

impl Error for EventTableError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Compatibility alias for a source-shaped row.
pub type EventTableQueryRow = EventQueryRow;
/// Compatibility alias for the decoded packed record.
pub type EventTableRecordValue = EventTableRecord;

fn source_bytes(value: &EventQueryValue, column: usize) -> Result<&[u8], EventRowError> {
    match value {
        EventQueryValue::Text(text) => Ok(text.as_bytes()),
        EventQueryValue::Bytes(bytes) => Ok(bytes),
        EventQueryValue::Null => Err(EventRowError::Null { column }),
        EventQueryValue::Error(message) => Err(EventRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn numeric_text(value: &EventQueryValue, column: usize) -> Result<&str, EventRowError> {
    let bytes = source_bytes(value, column)?;
    std::str::from_utf8(bytes).map_err(|_| EventRowError::InvalidNumber {
        column,
        value: format!("{bytes:?}"),
        target: "ASCII integer",
    })
}

fn is_integer_overflow(kind: &IntErrorKind) -> bool {
    matches!(kind, IntErrorKind::PosOverflow | IntErrorKind::NegOverflow)
}

fn decode_u32(value: &EventQueryValue, column: usize) -> Result<u32, EventRowError> {
    let text = numeric_text(value, column)?;
    text.parse::<u32>().map_err(|error| {
        if is_integer_overflow(error.kind()) {
            EventRowError::NumberOverflow {
                column,
                value: text.to_owned(),
                target: "u32",
            }
        } else {
            EventRowError::InvalidNumber {
                column,
                value: text.to_owned(),
                target: "u32",
            }
        }
    })
}

fn decode_i32(value: &EventQueryValue, column: usize) -> Result<i32, EventRowError> {
    let text = numeric_text(value, column)?;
    text.parse::<i32>().map_err(|error| {
        if is_integer_overflow(error.kind()) {
            EventRowError::NumberOverflow {
                column,
                value: text.to_owned(),
                target: "i32",
            }
        } else {
            EventRowError::InvalidNumber {
                column,
                value: text.to_owned(),
                target: "i32",
            }
        }
    })
}

fn decode_event_type(
    value: &EventQueryValue,
    column: usize,
) -> Result<[u8; EVENT_TYPE_BYTES], EventRowError> {
    let source = source_bytes(value, column)?;
    let content_len = source
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(source.len());
    let copy_len = content_len.min(EVENT_TYPE_MAX_CONTENT_BYTES);
    let mut result = [0_u8; EVENT_TYPE_BYTES];
    result[..copy_len].copy_from_slice(&source[..copy_len]);
    Ok(result)
}

fn decode_completed(value: &EventQueryValue, column: usize) -> Result<u8, EventRowError> {
    let text = numeric_text(value, column)?;
    if text.is_empty() {
        // The destination was zeroed by legacy memset and the bool overload
        // returns false for an empty string.
        return Ok(0);
    }
    let parsed = text.parse::<i32>().map_err(|error| {
        if is_integer_overflow(error.kind()) {
            EventRowError::NumberOverflow {
                column,
                value: text.to_owned(),
                target: "i32",
            }
        } else {
            EventRowError::InvalidNumber {
                column,
                value: text.to_owned(),
                target: "i32",
            }
        }
    })?;
    Ok(u8::from(parsed != 0))
}

fn build_records(
    rows: &[EventQueryRow],
    limits: EventLimits,
    decode: fn(&EventQueryRow) -> Result<EventTableRecord, EventRowError>,
) -> Result<Vec<EventTableRecord>, EventTableError> {
    if rows.len() > limits.max_records {
        return Err(EventTableError::TooManyRecords {
            count: rows.len(),
            maximum: limits.max_records,
        });
    }
    let mut records = Vec::new();
    records
        .try_reserve_exact(rows.len())
        .map_err(|_| EventTableError::AllocationFailed {
            requested: rows.len(),
        })?;
    for (index, row) in rows.iter().enumerate() {
        records.push(decode(row).map_err(|source| EventTableError::Row { index, source })?);
    }
    Ok(records)
}

/// Strictly decode rows into event records, preserving source row order.
///
/// The row cap is checked before the output is reserved.
///
/// # Errors
///
/// Returns [`EventTableError`] for an invalid row, the row cap, or an
/// allocation failure.
pub fn build_event_table(
    rows: &[EventQueryRow],
    limits: EventLimits,
) -> Result<Vec<EventTableRecord>, EventTableError> {
    build_records(rows, limits, decode_event_query_row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::EVENT_TABLE_WIRE_SIZE;

    fn value(text: &str) -> EventQueryValue {
        EventQueryValue::text(text)
    }

    fn valid_row() -> EventQueryRow {
        EventQueryRow::new([
            value("17"),
            value("birthday"),
            value("-10"),
            value("20"),
            value("-7"),
            value("99"),
            value("1"),
        ])
    }

    #[test]
    fn the_legacy_query_names_the_seven_columns_in_order() {
        let columns = EVENT_QUERY_COLUMN_NAMES.join(", ");
        assert_eq!(
            EVENT_LEGACY_QUERY,
            format!("SELECT {columns} FROM {EVENT_TABLE} ORDER BY start")
        );
        assert_eq!(EVENT_TABLE_WIRE_SIZE, 85);
    }

    #[test]
    fn valid_row_decodes_and_preserves_packed_field_offsets() {
        let record = decode_event_query_row(&valid_row()).unwrap();
        assert_eq!(record.id, 17);
        assert_eq!(&record.event_type[..8], b"birthday");
        assert_eq!(&record.event_type[8..], &[0_u8; 56]);
        assert_eq!(record.start_time, -10);
        assert_eq!(record.end_time, 20);
        assert_eq!(record.value0, -7);
        assert_eq!(record.value1, 99);
        assert_eq!(record.completed, 1);

        let bytes = record.encode();
        assert_eq!(bytes.len(), EVENT_TABLE_WIRE_SIZE);
        assert_eq!(&bytes[0..4], &17_u32.to_le_bytes());
        assert_eq!(&bytes[4..12], b"birthday");
        assert_eq!(&bytes[68..72], &(-10_i32).to_le_bytes());
        assert_eq!(bytes[84], 1);
    }

    #[test]
    fn type_bytes_stop_at_nul_truncate_and_remain_raw() {
        let mut row = valid_row().into_columns();
        row[1] = EventQueryValue::bytes([b'a', 0, b'b'].as_slice());
        let record = decode_event_query_row(&EventQueryRow::new(row)).unwrap();
        assert_eq!(&record.event_type[..3], &[b'a', 0, 0]);
        assert!(record.event_type[3..].iter().all(|&byte| byte == 0));

        let mut long = valid_row().into_columns();
        long[1] = EventQueryValue::bytes(vec![b'x'; EVENT_TYPE_BYTES + 10]);
        let record = decode_event_query_row(&EventQueryRow::new(long)).unwrap();
        assert_eq!(
            &record.event_type[..EVENT_TYPE_MAX_CONTENT_BYTES],
            &[b'x'; 63]
        );
        assert_eq!(record.event_type[63], 0);
    }

    #[test]
    fn null_wrong_width_and_source_errors_are_not_zero_filled() {
        let mut row = valid_row().into_columns();
        row[0] = EventQueryValue::null();
        assert_eq!(
            decode_event_query_row(&EventQueryRow::new(row)),
            Err(EventRowError::Null { column: 0 })
        );
        assert_eq!(
            decode_event_query_row(&EventQueryRow::new(vec![value("1"); 6])),
            Err(EventRowError::ColumnCount {
                expected: 7,
                actual: 6
            })
        );
        let mut error = valid_row().into_columns();
        error[4] = EventQueryValue::error("driver failed");
        assert!(matches!(
            decode_event_query_row(&EventQueryRow::new(error)),
            Err(EventRowError::Source { column: 4, .. })
        ));
    }

    #[test]
    fn numeric_malformed_and_overflow_values_are_rejected() {
        let mut malformed = valid_row().into_columns();
        malformed[2] = value("not-a-time");
        assert!(matches!(
            decode_event_query_row(&EventQueryRow::new(malformed)),
            Err(EventRowError::InvalidNumber { column: 2, .. })
        ));

        let mut overflow = valid_row().into_columns();
        overflow[3] = value("2147483648");
        assert!(matches!(
            decode_event_query_row(&EventQueryRow::new(overflow)),
            Err(EventRowError::NumberOverflow {
                column: 3,
                target: "i32",
                ..
            })
        ));

        let mut unsigned_overflow = valid_row().into_columns();
        unsigned_overflow[0] = value("4294967296");
        assert!(matches!(
            decode_event_query_row(&EventQueryRow::new(unsigned_overflow)),
            Err(EventRowError::NumberOverflow {
                column: 0,
                target: "u32",
                ..
            })
        ));
    }

    #[test]
    fn completed_uses_legacy_zero_empty_and_nonzero_policy() {
        for (source, expected) in [("", 0), ("0", 0), ("-0", 0), ("1", 1), ("2", 1), ("-1", 1)] {
            let mut row = valid_row().into_columns();
            row[6] = value(source);
            assert_eq!(
                decode_event_query_row(&EventQueryRow::new(row))
                    .unwrap()
                    .completed,
                expected
            );
        }
        let mut malformed = valid_row().into_columns();
        malformed[6] = value("true");
        assert!(matches!(
            decode_event_query_row(&EventQueryRow::new(malformed)),
            Err(EventRowError::InvalidNumber { column: 6, .. })
        ));
    }

    #[test]
    fn an_empty_source_is_an_empty_table() {
        assert_eq!(
            build_event_table(&[], EventLimits::default()),
            Ok(Vec::new())
        );
    }

    #[test]
    fn the_table_preserves_source_order_and_checks_the_cap_first() {
        let first = valid_row();
        let mut second = first.clone().into_columns();
        second[0] = value("18");
        let second = EventQueryRow::new(second);
        let table = build_event_table(&[first, second], EventLimits::new(2)).unwrap();
        assert_eq!(
            table.iter().map(|record| record.id).collect::<Vec<_>>(),
            [17, 18]
        );

        let mut bad = valid_row().into_columns();
        bad[0] = EventQueryValue::null();
        let bad = EventQueryRow::new(bad);
        assert_eq!(
            build_event_table(&[valid_row(), bad.clone()], EventLimits::new(1)),
            Err(EventTableError::TooManyRecords {
                count: 2,
                maximum: 1
            })
        );
        assert_eq!(
            build_event_table(&[valid_row(), bad], EventLimits::default()),
            Err(EventTableError::Row {
                index: 1,
                source: EventRowError::Null { column: 0 }
            })
        );
    }
}
