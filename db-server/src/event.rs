//! Source-verified, SQL-free `event` table loading.
//!
//! The legacy loader in `server/server/db/ClientManagerBoot.cpp:1693-1734`
//! runs one postfix-qualified statement under `__EVENT_MANAGER__`:
//!
//! ```text
//! SELECT id, type, UNIX_TIMESTAMP(start), UNIX_TIMESTAMP(end), value0, value1, completed FROM event%s ORDER BY start
//! ```
//!
//! It then copies seven positional cells into a zeroed packed `TEventTable`.
//! This module keeps the query, row conversion, limits, and boot-section
//! construction separate from `SQLx`.  `NULL` and malformed source values are
//! errors rather than zero-filled records.  The `type` field retains raw bytes,
//! truncates at the legacy 63-byte content limit, stops at the first NUL, and
//! zero-fills the 64-byte destination.  A valid empty result is a real section
//! with record width 85 and count zero.
//!
//! The optional feature profile is deliberately not selected here.  A caller
//! that enables `__EVENT_MANAGER__` may pass this section to
//! `BootSnapshot`; this module never composes or profiles a boot response.

use std::error::Error;
use std::fmt;
use std::num::IntErrorKind;

use protocol::db_boot::{BootSection, BootSectionKind};
use protocol::db_records::{EventTableRecord, EVENT_TABLE_WIRE_SIZE, EVENT_TYPE_BYTES};

use crate::postfix::{TablePostfix, TablePostfixError, MAX_TABLE_POSTFIX_BYTES};

/// Base table name used by the source event query.
pub const EVENT_TABLE: &str = "event";

/// Exact fixed-column prefix of the source event query.
///
/// The two timestamp expressions are intentionally retained.  They are
/// positional result columns, not assumed to have stable database aliases.
pub const EVENT_QUERY_PREFIX: &str =
    "SELECT id, type, UNIX_TIMESTAMP(start), UNIX_TIMESTAMP(end), value0, value1, completed FROM ";

/// Exact statement suffix used after the validated table identifier.
pub const EVENT_QUERY_SUFFIX: &str = " ORDER BY start";

/// Number of columns promised by [`EVENT_QUERY_PREFIX`].
pub const EVENT_QUERY_COLUMNS: usize = 7;

/// Maximum statement bytes that fit in the legacy `char query[4096]`.
///
/// The terminating NUL occupies one byte, so the statement itself is bounded
/// at 4,095 bytes.
pub const MAX_EVENT_QUERY_BYTES: usize = 4_095;

/// Maximum number of records representable by the boot `u16` count.
pub const EVENT_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Maximum packed section-data bytes at the representable count.
pub const EVENT_TABLE_MAX_SECTION_BYTES: usize = EVENT_TABLE_WIRE_SIZE * EVENT_TABLE_MAX_RECORDS;

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

const MAX_EVENT_TABLE_BYTES: usize = EVENT_TABLE.len() + MAX_TABLE_POSTFIX_BYTES;

/// A defensive failure while constructing the fixed event read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventQueryBuildError {
    /// The generated table identifier exceeded its bounded width.
    TableNameTooLong {
        /// Generated identifier byte length.
        length: usize,
        /// Maximum accepted identifier byte length.
        maximum: usize,
    },
    /// The generated table identifier contained a non-allowlisted byte.
    InvalidTableIdentifier {
        /// Zero-based byte offset in the generated identifier.
        index: usize,
        /// Disallowed byte value.
        byte: u8,
    },
    /// The generated statement would not fit the source query buffer.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for EventQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated event table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated event identifier has byte {byte:#04x} at offset {index}"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated event query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for EventQueryBuildError {}

/// One immutable, checked event read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventQuery {
    statement: String,
    table_name: String,
    postfix: TablePostfix,
}

impl EventQuery {
    /// Build the exact source-fixed query from a validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`EventQueryBuildError`] if a defensive identifier or statement
    /// width check fails.
    pub fn new(postfix: &TablePostfix) -> Result<Self, EventQueryBuildError> {
        let table_name = format!("{EVENT_TABLE}{}", postfix.as_str());
        if table_name.len() > MAX_EVENT_TABLE_BYTES {
            return Err(EventQueryBuildError::TableNameTooLong {
                length: table_name.len(),
                maximum: MAX_EVENT_TABLE_BYTES,
            });
        }
        if let Some((index, byte)) = table_name
            .bytes()
            .enumerate()
            .find(|(_, byte)| !is_event_identifier_byte(*byte))
        {
            return Err(EventQueryBuildError::InvalidTableIdentifier { index, byte });
        }

        let statement = format!("{EVENT_QUERY_PREFIX}{table_name}{EVENT_QUERY_SUFFIX}");
        if statement.len() > MAX_EVENT_QUERY_BYTES {
            return Err(EventQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_EVENT_QUERY_BYTES,
            });
        }

        Ok(Self {
            statement,
            table_name,
            postfix: postfix.clone(),
        })
    }

    /// Validate optional configuration and build the query.
    ///
    /// # Errors
    ///
    /// Returns [`EventBoundaryError::Postfix`] for invalid configuration or
    /// [`EventBoundaryError::Query`] for a defensive construction failure.
    pub fn from_config(configured_postfix: Option<&str>) -> Result<Self, EventBoundaryError> {
        let postfix =
            TablePostfix::from_config(configured_postfix).map_err(EventBoundaryError::Postfix)?;
        Self::new(&postfix).map_err(EventBoundaryError::Query)
    }

    /// Borrow the exact statement text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Borrow the generated event table identifier.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix used to construct the query.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }
}

/// A failure while validating an event postfix or fixed query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The fixed event query failed a defensive construction check.
    Query(EventQueryBuildError),
}

impl fmt::Display for EventBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for EventBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

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

/// Limits applied before allocating an event boot section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSectionLimits {
    /// Maximum number of source rows accepted by the pure builder.
    pub max_source_rows: usize,
    /// Maximum number of records accepted by the pure builder.
    pub max_records: usize,
    /// Maximum packed section-data bytes accepted by the pure builder.
    pub max_data_bytes: usize,
}

impl EventSectionLimits {
    /// Construct limits with the source and record caps equal to `max_records`.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            max_source_rows: max_records,
            max_records,
            max_data_bytes: EVENT_TABLE_MAX_SECTION_BYTES,
        }
    }

    /// Construct all three bounds explicitly.
    #[must_use]
    pub const fn with_limits(
        max_source_rows: usize,
        max_records: usize,
        max_data_bytes: usize,
    ) -> Self {
        Self {
            max_source_rows,
            max_records,
            max_data_bytes,
        }
    }
}

impl Default for EventSectionLimits {
    fn default() -> Self {
        Self::new(EVENT_TABLE_MAX_RECORDS)
    }
}

/// A checked event section-construction error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventSectionError {
    /// The source supplied more rows than allowed.
    TooManySourceRows {
        /// Supplied row count.
        count: usize,
        /// Configured source-row limit.
        maximum: usize,
    },
    /// The source supplied more records than allowed.
    TooManyRecords {
        /// Supplied row count.
        count: usize,
        /// Configured record limit.
        maximum: usize,
    },
    /// The record count cannot be represented by the boot `u16` count.
    CountOverflow {
        /// Supplied row count.
        count: usize,
    },
    /// The fixed record width cannot be represented by `u16`.
    RecordSizeOverflow {
        /// Fixed record width.
        size: usize,
    },
    /// Packed data-size arithmetic overflowed `usize`.
    DataSizeOverflow {
        /// Row count used in the multiplication.
        count: usize,
    },
    /// Packed section data exceeds the configured byte limit.
    DataTooLarge {
        /// Required data length.
        length: usize,
        /// Configured byte limit.
        maximum: usize,
    },
    /// A decoded record did not have the fixed wire width.
    RecordSizeMismatch {
        /// Zero-based source row index.
        index: usize,
        /// Required width.
        expected: usize,
        /// Actual encoded width.
        actual: usize,
    },
    /// A source row could not be strictly decoded.
    Row {
        /// Zero-based source row index.
        index: usize,
        /// Row error.
        source: EventRowError,
    },
    /// The output vector could not reserve its checked data length.
    AllocationFailed {
        /// Requested allocation size.
        requested: usize,
    },
}

impl fmt::Display for EventSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySourceRows { count, maximum } => write!(
                formatter,
                "event source has {count} rows; maximum is {maximum}"
            ),
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "event table has {count} records; maximum is {maximum}"
            ),
            Self::CountOverflow { count } => {
                write!(formatter, "event record count {count} does not fit u16")
            }
            Self::RecordSizeOverflow { size } => {
                write!(formatter, "event record size {size} does not fit u16")
            }
            Self::DataSizeOverflow { count } => {
                write!(
                    formatter,
                    "event data size overflows usize for {count} rows"
                )
            }
            Self::DataTooLarge { length, maximum } => {
                write!(
                    formatter,
                    "event data length {length} exceeds limit {maximum}"
                )
            }
            Self::RecordSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "event row {index} encoded to {actual} bytes; expected {expected}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "event row {index} is invalid: {source}")
            }
            Self::AllocationFailed { requested } => {
                write!(formatter, "event allocation of {requested} bytes failed")
            }
        }
    }
}

impl Error for EventSectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// An injected source of source-shaped event rows in database order.
pub trait EventTableRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain rows in the order returned by the database.
    ///
    /// # Errors
    ///
    /// Returns the source error when the query or extraction fails. A failed
    /// source must not be represented as an empty row set.
    fn query_rows(&self, query: &EventQuery) -> Result<Vec<EventQueryRow>, Self::Error>;
}

impl<F, E> EventTableRowSource for F
where
    F: Fn(&EventQuery) -> Result<Vec<EventQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &EventQuery) -> Result<Vec<EventQueryRow>, Self::Error> {
        self(query)
    }
}

/// An error while obtaining or decoding event rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventLoadError<E> {
    /// The injected source failed.
    Source(E),
    /// Rows could not be decoded or bounded into a section.
    Section(EventSectionError),
}

impl<E: fmt::Display> fmt::Display for EventLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "event row source failed: {source}"),
            Self::Section(source) => {
                write!(formatter, "event section construction failed: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for EventLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Section(source) => Some(source),
        }
    }
}

/// A reusable, bounded pure event loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventTableLoader {
    query: EventQuery,
    limits: EventSectionLimits,
}

impl EventTableLoader {
    /// Construct a loader from a validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`EventQueryBuildError`] if a defensive query check fails.
    pub fn new(
        postfix: &TablePostfix,
        limits: EventSectionLimits,
    ) -> Result<Self, EventQueryBuildError> {
        Ok(Self {
            query: EventQuery::new(postfix)?,
            limits,
        })
    }

    /// Validate raw configuration and construct a loader.
    ///
    /// # Errors
    ///
    /// Returns [`EventBoundaryError`] for an invalid postfix or query check.
    pub fn from_config(
        configured_postfix: Option<&str>,
        limits: EventSectionLimits,
    ) -> Result<Self, EventBoundaryError> {
        Ok(Self {
            query: EventQuery::from_config(configured_postfix)?,
            limits,
        })
    }

    /// Borrow the immutable checked query.
    #[must_use]
    pub const fn query(&self) -> &EventQuery {
        &self.query
    }

    /// Return the configured section limits.
    #[must_use]
    pub const fn limits(&self) -> EventSectionLimits {
        self.limits
    }

    /// Obtain rows through an injected source and build the event section.
    ///
    /// # Errors
    ///
    /// Returns [`EventLoadError::Source`] for source failure and
    /// [`EventLoadError::Section`] for row, count, byte, or allocation failure.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, EventLoadError<S::Error>>
    where
        S: EventTableRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(EventLoadError::Source)?;
        build_event_section(&rows, self.limits).map_err(EventLoadError::Section)
    }
}

/// Build an event boot section from source-shaped rows.
///
/// The row and packed-byte limits are checked before the output vector is
/// reserved. Every row is then strictly decoded and encoded with the existing
/// fixed `TEventTable` codec. An empty input produces an explicit empty event
/// section, not an error or a fabricated row.
///
/// # Errors
///
/// Returns [`EventSectionError`] when a source/record/data limit is exceeded,
/// a row is malformed, a record width is invalid, or the output allocation
/// cannot be reserved.
pub fn build_event_section(
    rows: &[EventQueryRow],
    limits: EventSectionLimits,
) -> Result<BootSection, EventSectionError> {
    if rows.len() > limits.max_source_rows {
        return Err(EventSectionError::TooManySourceRows {
            count: rows.len(),
            maximum: limits.max_source_rows,
        });
    }
    if rows.len() > limits.max_records {
        return Err(EventSectionError::TooManyRecords {
            count: rows.len(),
            maximum: limits.max_records,
        });
    }

    let count = u16::try_from(rows.len())
        .map_err(|_| EventSectionError::CountOverflow { count: rows.len() })?;
    let record_size = u16::try_from(EVENT_TABLE_WIRE_SIZE).map_err(|_| {
        EventSectionError::RecordSizeOverflow {
            size: EVENT_TABLE_WIRE_SIZE,
        }
    })?;
    let data_len = EVENT_TABLE_WIRE_SIZE
        .checked_mul(rows.len())
        .ok_or(EventSectionError::DataSizeOverflow { count: rows.len() })?;
    if data_len > limits.max_data_bytes {
        return Err(EventSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }

    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| EventSectionError::AllocationFailed {
            requested: data_len,
        })?;
    for (index, row) in rows.iter().enumerate() {
        let record = decode_event_query_row(row)
            .map_err(|source| EventSectionError::Row { index, source })?;
        let encoded = record.encode();
        if encoded.len() != EVENT_TABLE_WIRE_SIZE {
            return Err(EventSectionError::RecordSizeMismatch {
                index,
                expected: EVENT_TABLE_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }

    Ok(BootSection {
        kind: BootSectionKind::Event,
        record_size,
        count,
        data,
    })
}

/// Compatibility alias for the source table name.
pub type EventTableQuery = EventQuery;
/// Compatibility alias for a source-shaped row.
pub type EventTableQueryRow = EventQueryRow;
/// Compatibility alias for the decoded packed record.
pub type EventTableRecordValue = EventTableRecord;
/// Compatibility alias for a pure loader error.
pub type EventTableLoadError<E> = EventLoadError<E>;

fn is_event_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::decode_event_table_section;
    use protocol::db_boot::BootFeatureProfile;

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
    fn query_matches_source_spacing_order_and_postfix() {
        let query = EventQuery::from_config(Some("_prod")).unwrap();
        assert_eq!(query.table_name(), "event_prod");
        assert_eq!(
            query.as_str(),
            "SELECT id, type, UNIX_TIMESTAMP(start), UNIX_TIMESTAMP(end), value0, value1, completed FROM event_prod ORDER BY start"
        );
        assert!(query.as_str().len() <= MAX_EVENT_QUERY_BYTES);
        assert_eq!(EventQuery::from_config(None).unwrap().table_name(), "event");
        assert!(matches!(
            EventQuery::from_config(Some("bad-name")),
            Err(EventBoundaryError::Postfix(_))
        ));
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
    fn empty_result_is_an_explicit_empty_event_section() {
        let section = build_event_section(&[], EventSectionLimits::default()).unwrap();
        assert_eq!(section.kind, BootSectionKind::Event);
        assert_eq!(section.record_size, 85);
        assert_eq!(section.count, 0);
        assert!(section.data.is_empty());
        assert!(
            decode_event_table_section(&section, BootFeatureProfile::new(false, true, false))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn section_preserves_source_order_and_checks_limits_before_encoding() {
        let first = valid_row();
        let mut second = first.clone().into_columns();
        second[0] = value("18");
        let second = EventQueryRow::new(second);
        let section = build_event_section(
            &[first, second],
            EventSectionLimits::with_limits(2, 2, EVENT_TABLE_WIRE_SIZE * 2),
        )
        .unwrap();
        let decoded =
            decode_event_table_section(&section, BootFeatureProfile::new(false, true, false))
                .unwrap();
        assert_eq!(
            decoded.iter().map(|record| record.id).collect::<Vec<_>>(),
            [17, 18]
        );

        assert_eq!(
            build_event_section(
                &[valid_row(), valid_row()],
                EventSectionLimits::with_limits(1, 2, EVENT_TABLE_WIRE_SIZE * 2)
            ),
            Err(EventSectionError::TooManySourceRows {
                count: 2,
                maximum: 1
            })
        );
        assert_eq!(
            build_event_section(
                &[valid_row()],
                EventSectionLimits::with_limits(1, 1, EVENT_TABLE_WIRE_SIZE - 1)
            ),
            Err(EventSectionError::DataTooLarge {
                length: EVENT_TABLE_WIRE_SIZE,
                maximum: EVENT_TABLE_WIRE_SIZE - 1
            })
        );
    }

    #[test]
    fn injected_source_failure_is_not_an_empty_section() {
        let loader = EventTableLoader::from_config(None, EventSectionLimits::default()).unwrap();
        let source = |_query: &EventQuery| -> Result<Vec<EventQueryRow>, &'static str> {
            Err("database unavailable")
        };
        assert_eq!(
            loader.load_section(&source),
            Err(EventLoadError::Source("database unavailable"))
        );
    }

    #[test]
    fn loader_passes_the_checked_query_and_builds_a_section() {
        let postfix = TablePostfix::parse("_test").unwrap();
        let loader = EventTableLoader::new(&postfix, EventSectionLimits::default()).unwrap();
        let source = |query: &EventQuery| {
            assert_eq!(query.table_name(), "event_test");
            Ok::<_, &'static str>(vec![valid_row()])
        };
        let section = loader.load_section(&source).unwrap();
        assert_eq!(section.kind, BootSectionKind::Event);
        assert_eq!(section.count, 1);
    }
}
