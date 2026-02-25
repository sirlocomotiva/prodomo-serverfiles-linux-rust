//! SQL-free acquisition and conversion for the legacy Monarch query.
//!
//! The source contract is `CMonarch::LoadMonarch` in
//! `server/server/db/Monarch.cpp`. This module builds one immutable checked
//! statement, preserves raw five-cell rows, and returns the existing
//! [`protocol::db_boot::BootMonarchInfo`] value. It does not execute SQL or
//! own boot, cache, transport, persistence, or gameplay state.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootMonarchInfo, MONARCH_INFO_WIRE_SIZE};

use crate::postfix::{TablePostfix, TablePostfixError};

/// Number of positional cells selected by the legacy Monarch query.
pub const MONARCH_QUERY_COLUMN_COUNT: usize = 5;

/// Alias for [`MONARCH_QUERY_COLUMN_COUNT`].
pub const MONARCH_TABLE_QUERY_COLUMNS: usize = MONARCH_QUERY_COLUMN_COUNT;

/// Maximum number of source rows that can map to the four fixed empire slots.
pub const MONARCH_MAX_SOURCE_ROWS: usize = 4;

/// Fixed byte width of the returned x86 `MonarchInfo` value.
pub const MONARCH_OUTPUT_WIRE_SIZE: usize = MONARCH_INFO_WIRE_SIZE;

/// Placeholder-free length of the exact source query.
pub const MONARCH_QUERY_BASE_BYTES: usize = 92;

/// Maximum statement bytes that fit in the legacy `char[256]` buffer.
///
/// The terminating NUL occupies one byte, so the statement itself is bounded
/// at 255 bytes.
pub const MAX_MONARCH_QUERY_BYTES: usize = 255;

/// Effective postfix bound implied by the fixed query and statement buffer.
pub const MAX_MONARCH_POSTFIX_BYTES: usize = MAX_MONARCH_QUERY_BYTES - MONARCH_QUERY_BASE_BYTES;

/// Exact query prefix before the validated player-table postfix.
pub const MONARCH_QUERY_PREFIX: &str =
    "SELECT a.empire, a.pid, b.name, a.money, a.windate FROM monarch a, player";

/// Exact query suffix after the validated player-table postfix.
pub const MONARCH_QUERY_SUFFIX: &str = " b WHERE a.pid=b.id";

/// Positional query-column names in exact source order.
pub const MONARCH_QUERY_COLUMN_NAMES: [&str; MONARCH_QUERY_COLUMN_COUNT] =
    ["empire", "pid", "name", "money", "windate"];

/// Number of content bytes that fit before a C-string terminator.
pub const MONARCH_TEXT_CONTENT_BYTES: usize = 31;

/// Number of bytes in each name and date C array.
pub const MONARCH_TEXT_BYTES: usize = 32;

/// One lossless value from a five-cell Monarch query row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonarchQueryValue {
    /// Non-NULL UTF-8 text supplied by a caller.
    Text(String),
    /// Non-NULL raw bytes supplied by a database adapter.
    Bytes(Vec<u8>),
    /// SQL `NULL`.
    Null,
    /// A source extraction or conversion error.
    Error(String),
}

impl MonarchQueryValue {
    /// Construct a non-NULL text value.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    /// Construct a non-NULL raw-byte value without UTF-8 normalization.
    #[must_use]
    pub fn bytes(value: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(value.into())
    }

    /// Construct a SQL `NULL` value.
    #[must_use]
    pub const fn null() -> Self {
        Self::Null
    }

    /// Construct a source-cell error value.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error(message.into())
    }

    /// Borrow bytes for a text or raw-byte value.
    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Text(value) => Some(value.as_bytes()),
            Self::Bytes(value) => Some(value.as_slice()),
            Self::Null | Self::Error(_) => None,
        }
    }
}

impl From<String> for MonarchQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for MonarchQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for MonarchQueryValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for MonarchQueryValue {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl<const N: usize> From<[u8; N]> for MonarchQueryValue {
    fn from(value: [u8; N]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl From<Option<Vec<u8>>> for MonarchQueryValue {
    fn from(value: Option<Vec<u8>>) -> Self {
        value.map_or(Self::Null, Self::Bytes)
    }
}

impl From<Option<String>> for MonarchQueryValue {
    fn from(value: Option<String>) -> Self {
        value.map_or(Self::Null, Self::Text)
    }
}

/// One source-shaped Monarch query row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonarchQueryRow {
    columns: Vec<MonarchQueryValue>,
}

impl MonarchQueryRow {
    /// Construct a row while retaining its supplied width.
    #[must_use]
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = MonarchQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub fn from_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = MonarchQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct and validate the exact five-column shape with bounded
    /// accumulation.
    ///
    /// The iterator is never advanced past cell six, and its length hint is
    /// not trusted for allocation.
    ///
    /// # Errors
    ///
    /// Returns [`MonarchRowError::ColumnCount`] for any width other than
    /// five, or [`MonarchRowError::AllocationFailed`] if the bounded
    /// five-cell reservation fails.
    pub fn try_new<I>(columns: I) -> Result<Self, MonarchRowError>
    where
        I: IntoIterator<Item = MonarchQueryValue>,
    {
        let mut row = Self {
            columns: Vec::new(),
        };
        row.columns
            .try_reserve_exact(MONARCH_QUERY_COLUMN_COUNT)
            .map_err(|_| MonarchRowError::AllocationFailed {
                requested: MONARCH_QUERY_COLUMN_COUNT,
            })?;
        for value in columns {
            if row.columns.len() == MONARCH_QUERY_COLUMN_COUNT {
                return Err(MonarchRowError::ColumnCount {
                    expected: MONARCH_QUERY_COLUMN_COUNT,
                    actual: MONARCH_QUERY_COLUMN_COUNT + 1,
                });
            }
            row.columns.push(value);
        }
        check_row_width(&row)?;
        Ok(row)
    }

    /// Construct from a statically sized, correctly shaped column set.
    #[must_use]
    pub fn from_typed_columns(columns: [MonarchQueryValue; MONARCH_QUERY_COLUMN_COUNT]) -> Self {
        Self::new(columns)
    }

    /// Borrow all cells in query order.
    #[must_use]
    pub fn columns(&self) -> &[MonarchQueryValue] {
        &self.columns
    }

    /// Consume the row and return all cells in query order.
    #[must_use]
    pub fn into_columns(self) -> Vec<MonarchQueryValue> {
        self.columns
    }

    /// Return the supplied cell count.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

impl From<[MonarchQueryValue; MONARCH_QUERY_COLUMN_COUNT]> for MonarchQueryRow {
    fn from(columns: [MonarchQueryValue; MONARCH_QUERY_COLUMN_COUNT]) -> Self {
        Self::from_typed_columns(columns)
    }
}

impl TryFrom<Vec<MonarchQueryValue>> for MonarchQueryRow {
    type Error = MonarchRowError;

    fn try_from(columns: Vec<MonarchQueryValue>) -> Result<Self, Self::Error> {
        Self::try_new(columns)
    }
}

/// Return a stable diagnostic name for one selected expression.
#[must_use]
pub const fn monarch_query_column_name(index: usize) -> &'static str {
    match index {
        0 => "empire",
        1 => "pid",
        2 => "name",
        3 => "money",
        4 => "windate",
        _ => "unknown",
    }
}

/// A failure while validating or converting one Monarch row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonarchRowError {
    /// The row did not contain exactly five cells.
    ColumnCount {
        /// Required cell count.
        expected: usize,
        /// Supplied cell count.
        actual: usize,
    },
    /// The bounded typed-row constructor could not reserve five cells.
    AllocationFailed {
        /// Number of cells requested by the failed reservation.
        requested: usize,
    },
    /// A required cell was SQL `NULL`.
    Null {
        /// Zero-based query column.
        column: usize,
    },
    /// A source cell could not be obtained.
    Source {
        /// Zero-based query column.
        column: usize,
        /// Source diagnostic.
        message: String,
    },
    /// A strict numeric cell was not a complete decimal integer.
    InvalidNumber {
        /// Zero-based query column.
        column: usize,
        /// Lossy diagnostic representation of the source bytes.
        value: String,
        /// Target integer policy.
        target: &'static str,
    },
    /// A syntactically valid strict integer exceeded its target.
    NumberOverflow {
        /// Zero-based query column.
        column: usize,
        /// Lossy diagnostic representation of the source bytes.
        value: String,
        /// Target integer policy.
        target: &'static str,
    },
    /// A numeric empire did not identify one of the four fixed slots.
    EmpireOutOfRange {
        /// Converted signed legacy `int` value.
        value: i32,
    },
    /// A strict name or date exceeded the C-array content bound.
    TextTooLong {
        /// Name or date query column, 2 or 4.
        column: usize,
        /// Supplied raw byte length.
        length: usize,
        /// Maximum bytes allowed without an explicit terminal NUL.
        maximum: usize,
    },
    /// A strict name or date contained a NUL before its final byte.
    TextInteriorNul {
        /// Name or date query column, 2 or 4.
        column: usize,
        /// Byte offset of the first NUL.
        index: usize,
    },
}

impl fmt::Display for MonarchRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => write!(
                formatter,
                "monarch query row has {actual} columns; expected {expected}"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "monarch query row could not reserve {requested} bounded cells"
            ),
            Self::Null { column } => write!(
                formatter,
                "monarch query column {} is NULL",
                monarch_query_column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "monarch query column {} could not be read: {message}",
                monarch_query_column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "monarch query column {} value {value:?} is not a strict {target}",
                monarch_query_column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "monarch query column {} value {value:?} overflows {target}",
                monarch_query_column_name(*column)
            ),
            Self::EmpireOutOfRange { value } => {
                write!(formatter, "monarch empire {value} is outside 0..=3")
            }
            Self::TextTooLong {
                column,
                length,
                maximum,
            } => write!(
                formatter,
                "monarch query column {} is {length} bytes; maximum is {maximum}",
                monarch_query_column_name(*column)
            ),
            Self::TextInteriorNul { column, index } => write!(
                formatter,
                "monarch query column {} has an interior NUL at byte {index}",
                monarch_query_column_name(*column)
            ),
        }
    }
}

impl Error for MonarchRowError {}

/// One validated Monarch row ready for fixed-slot assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedMonarchRow {
    /// Validated empire slot in the inclusive range `0..=3`.
    pub empire: u8,
    /// Monarch player ID.
    pub pid: u32,
    /// Zero-filled 32-byte player name.
    pub name: [u8; MONARCH_TEXT_BYTES],
    /// Monarch money.
    pub money: i64,
    /// Zero-filled 32-byte election date.
    pub date: [u8; MONARCH_TEXT_BYTES],
}

/// A defensive failure while composing the fixed Monarch statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonarchQueryBuildError {
    /// The final statement exceeded the legacy C-buffer bound.
    QueryTooLong {
        /// Rendered statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for MonarchQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated Monarch query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for MonarchQueryBuildError {}

/// A failure while validating Monarch boundary inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonarchBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The rendered statement exceeded its fixed buffer.
    Query(MonarchQueryBuildError),
}

impl fmt::Display for MonarchBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for MonarchBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// One immutable checked Monarch read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonarchQuery {
    statement: String,
    table_name: String,
    postfix: TablePostfix,
}

impl MonarchQuery {
    /// Build the exact source query from a validated table postfix.
    ///
    /// Only the generated `player` identifier receives the postfix. A
    /// postfix accepted by [`TablePostfix`] can still make this statement too
    /// long; that condition is rejected instead of reproducing `snprintf`
    /// truncation.
    ///
    /// # Errors
    ///
    /// Returns [`MonarchQueryBuildError::QueryTooLong`] when the rendered
    /// statement exceeds 255 bytes.
    pub fn new(postfix: &TablePostfix) -> Result<Self, MonarchQueryBuildError> {
        let length = MONARCH_QUERY_BASE_BYTES
            .checked_add(postfix.as_str().len())
            .ok_or(MonarchQueryBuildError::QueryTooLong {
                length: usize::MAX,
                maximum: MAX_MONARCH_QUERY_BYTES,
            })?;
        if length > MAX_MONARCH_QUERY_BYTES {
            return Err(MonarchQueryBuildError::QueryTooLong {
                length,
                maximum: MAX_MONARCH_QUERY_BYTES,
            });
        }

        let mut statement = String::with_capacity(length);
        statement.push_str(MONARCH_QUERY_PREFIX);
        statement.push_str(postfix.as_str());
        statement.push_str(MONARCH_QUERY_SUFFIX);
        let table_name = format!("player{}", postfix.as_str());
        Ok(Self {
            statement,
            table_name,
            postfix: postfix.clone(),
        })
    }

    /// Build from optional raw configuration.
    ///
    /// # Errors
    ///
    /// Returns an invalid postfix or an overlength rendered statement.
    pub fn from_config(configured_postfix: Option<&str>) -> Result<Self, MonarchBoundaryError> {
        let postfix =
            TablePostfix::from_config(configured_postfix).map_err(MonarchBoundaryError::Postfix)?;
        Self::new(&postfix).map_err(MonarchBoundaryError::Query)
    }

    /// Borrow the exact immutable query text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Borrow the generated player table name.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }
}

/// Row-count bound used by the pure loader and `SQLx` acquisition adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonarchLimits {
    /// Maximum number of source rows requested and accepted.
    pub max_rows: usize,
}

impl MonarchLimits {
    /// Construct an explicit source-row limit.
    #[must_use]
    pub const fn new(max_rows: usize) -> Self {
        Self { max_rows }
    }
}

impl Default for MonarchLimits {
    fn default() -> Self {
        Self {
            max_rows: MONARCH_MAX_SOURCE_ROWS,
        }
    }
}

/// An injected source of checked five-cell Monarch query rows.
pub trait MonarchRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw rows without relying on SQL order.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source
    /// failure must not become an empty row set.
    fn query_rows(&self, query: &MonarchQuery) -> Result<Vec<MonarchQueryRow>, Self::Error>;
}

impl<F, E> MonarchRowSource for F
where
    F: Fn(&MonarchQuery) -> Result<Vec<MonarchQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &MonarchQuery) -> Result<Vec<MonarchQueryRow>, Self::Error> {
        self(query)
    }
}

/// A failure while acquiring or building rows through an injected loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonarchLoadError<E> {
    /// The injected source failed.
    Source(E),
    /// The source returned zero rows.
    EmptyResult,
    /// Acquired rows failed the selected conversion or slot policy.
    Rows(MonarchBuildError),
}

impl<E: fmt::Display> fmt::Display for MonarchLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "monarch row source failed: {source}"),
            Self::EmptyResult => write!(formatter, "monarch source returned no rows"),
            Self::Rows(source) => {
                write!(formatter, "monarch rows could not be built: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for MonarchLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::EmptyResult => None,
            Self::Rows(source) => Some(source),
        }
    }
}

/// Reusable checked-query, source-limit, and conversion-policy holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonarchLoader {
    query: MonarchQuery,
    limits: MonarchLimits,
}

impl MonarchLoader {
    /// Construct a loader with an explicit source-row cap.
    ///
    /// # Errors
    ///
    /// Returns a defensive query-construction error.
    pub fn new(
        postfix: &TablePostfix,
        limits: MonarchLimits,
    ) -> Result<Self, MonarchQueryBuildError> {
        Ok(Self {
            query: MonarchQuery::new(postfix)?,
            limits,
        })
    }

    /// Construct from optional raw configuration.
    ///
    /// # Errors
    ///
    /// Returns invalid postfix or overlength query input.
    pub fn from_config(
        configured_postfix: Option<&str>,
        limits: MonarchLimits,
    ) -> Result<Self, MonarchBoundaryError> {
        let postfix =
            TablePostfix::from_config(configured_postfix).map_err(MonarchBoundaryError::Postfix)?;
        Self::new(&postfix, limits).map_err(MonarchBoundaryError::Query)
    }

    /// Borrow the immutable checked query.
    #[must_use]
    pub const fn query(&self) -> &MonarchQuery {
        &self.query
    }

    /// Return configured source limits.
    #[must_use]
    pub const fn limits(&self) -> MonarchLimits {
        self.limits
    }

    /// Acquire raw rows and strictly build a fresh fixed output value.
    ///
    /// # Errors
    ///
    /// Returns source, empty-result, or selected row/build failure.
    pub fn load<S>(&self, source: &S) -> Result<BootMonarchInfo, MonarchLoadError<S::Error>>
    where
        S: MonarchRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(MonarchLoadError::Source)?;
        if rows.is_empty() {
            return Err(MonarchLoadError::EmptyResult);
        }
        build_monarch_info_with_limit(&rows, self.limits.max_rows).map_err(MonarchLoadError::Rows)
    }

    /// Acquire rows and build with the explicit legacy conversion policy.
    ///
    /// # Errors
    ///
    /// Returns source, empty-result, or selected row/build failure.
    pub fn load_legacy<S>(&self, source: &S) -> Result<BootMonarchInfo, MonarchLoadError<S::Error>>
    where
        S: MonarchRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(MonarchLoadError::Source)?;
        if rows.is_empty() {
            return Err(MonarchLoadError::EmptyResult);
        }
        build_monarch_info_legacy_with_limit(&rows, self.limits.max_rows)
            .map_err(MonarchLoadError::Rows)
    }
}

/// A pure fixed-slot construction failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonarchBuildError {
    /// The source row count exceeded either the configured or four-slot cap.
    TooManyRows {
        /// Supplied source-row count.
        count: usize,
        /// Effective fixed maximum.
        maximum: usize,
    },
    /// More than one row claimed the same empire slot.
    DuplicateEmpire {
        /// Duplicate empire slot.
        empire: u8,
        /// First source-row index that claimed the slot.
        first_row: usize,
        /// Later source-row index that claimed the same slot.
        duplicate_row: usize,
    },
    /// One source row failed the selected conversion policy.
    Row {
        /// Zero-based source-row index.
        index: usize,
        /// Row conversion failure.
        source: MonarchRowError,
    },
}

impl fmt::Display for MonarchBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRows { count, maximum } => {
                write!(
                    formatter,
                    "monarch source has {count} rows; maximum is {maximum}"
                )
            }
            Self::DuplicateEmpire {
                empire,
                first_row,
                duplicate_row,
            } => write!(
                formatter,
                "monarch empire {empire} is duplicated by rows {first_row} and {duplicate_row}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "monarch row {index} is invalid: {source}")
            }
        }
    }
}

impl Error for MonarchBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            Self::TooManyRows { .. } | Self::DuplicateEmpire { .. } => None,
        }
    }
}

fn check_row_width(row: &MonarchQueryRow) -> Result<(), MonarchRowError> {
    if row.columns.len() == MONARCH_QUERY_COLUMN_COUNT {
        Ok(())
    } else {
        Err(MonarchRowError::ColumnCount {
            expected: MONARCH_QUERY_COLUMN_COUNT,
            actual: row.columns.len(),
        })
    }
}

fn cell_bytes(value: &MonarchQueryValue, column: usize) -> Result<&[u8], MonarchRowError> {
    match value {
        MonarchQueryValue::Text(text) => Ok(text.as_bytes()),
        MonarchQueryValue::Bytes(bytes) => Ok(bytes.as_slice()),
        MonarchQueryValue::Null => Err(MonarchRowError::Null { column }),
        MonarchQueryValue::Error(message) => Err(MonarchRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn diagnostic_value(value: &MonarchQueryValue) -> String {
    value.as_bytes().map_or_else(
        || "<unavailable>".to_owned(),
        |bytes| String::from_utf8_lossy(bytes).into_owned(),
    )
}

fn strict_unsigned_decimal(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)
}

fn strict_signed_decimal(bytes: &[u8]) -> bool {
    let Some(first) = bytes.first().copied() else {
        return false;
    };
    let digits = if first == b'-' { &bytes[1..] } else { bytes };
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

fn decode_strict_i32(
    value: &MonarchQueryValue,
    column: usize,
    target: &'static str,
) -> Result<i32, MonarchRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_signed_decimal(bytes) {
        return Err(MonarchRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MonarchRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target,
    })?;
    text.parse::<i32>()
        .map_err(|_| MonarchRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target,
        })
}

fn decode_strict_u32(
    value: &MonarchQueryValue,
    column: usize,
    target: &'static str,
) -> Result<u32, MonarchRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_unsigned_decimal(bytes) {
        return Err(MonarchRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MonarchRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target,
    })?;
    text.parse::<u32>()
        .map_err(|_| MonarchRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target,
        })
}

fn decode_strict_i64(
    value: &MonarchQueryValue,
    column: usize,
    target: &'static str,
) -> Result<i64, MonarchRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_signed_decimal(bytes) {
        return Err(MonarchRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MonarchRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target,
    })?;
    text.parse::<i64>()
        .map_err(|_| MonarchRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target,
        })
}

fn decode_text_strict(
    value: &MonarchQueryValue,
    column: usize,
) -> Result<[u8; MONARCH_TEXT_BYTES], MonarchRowError> {
    let bytes = cell_bytes(value, column)?;
    if let Some(index) = bytes.iter().position(|byte| *byte == 0) {
        if index + 1 != bytes.len() {
            return Err(MonarchRowError::TextInteriorNul { column, index });
        }
    }
    let maximum = if bytes.last() == Some(&0) {
        MONARCH_TEXT_BYTES
    } else {
        MONARCH_TEXT_CONTENT_BYTES
    };
    if bytes.len() > maximum {
        return Err(MonarchRowError::TextTooLong {
            column,
            length: bytes.len(),
            maximum,
        });
    }
    let mut output = [0_u8; MONARCH_TEXT_BYTES];
    output[..bytes.len()].copy_from_slice(bytes);
    Ok(output)
}

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn legacy_c_string_bytes(
    value: &MonarchQueryValue,
    column: usize,
) -> Result<&[u8], MonarchRowError> {
    let bytes = cell_bytes(value, column)?;
    Ok(match bytes.iter().position(|byte| *byte == 0) {
        Some(index) => &bytes[..index],
        None => bytes,
    })
}

fn legacy_numeric_prefix(bytes: &[u8]) -> (bool, Option<u64>, bool) {
    let mut index = 0;
    while index < bytes.len() && is_c_space(bytes[index]) {
        index += 1;
    }
    let mut negative = false;
    if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
        negative = bytes[index] == b'-';
        index += 1;
    }
    let start = index;
    let mut magnitude = 0_u64;
    let mut overflow = false;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        let digit = u64::from(bytes[index] - b'0');
        magnitude = if let Some(value) = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(digit))
        {
            value
        } else {
            overflow = true;
            u64::MAX
        };
        index += 1;
    }
    (negative, (index != start).then_some(magnitude), overflow)
}

fn legacy_strtol32(value: &MonarchQueryValue, column: usize) -> Result<i32, MonarchRowError> {
    let bytes = legacy_c_string_bytes(value, column)?;
    let (negative, magnitude, overflow) = legacy_numeric_prefix(bytes);
    let Some(magnitude) = magnitude else {
        return Ok(0);
    };
    if overflow {
        return Ok(if negative { i32::MIN } else { i32::MAX });
    }
    if negative {
        if magnitude >= 2_147_483_648 {
            Ok(i32::MIN)
        } else {
            Ok(i32::try_from(magnitude).unwrap_or(i32::MIN).wrapping_neg())
        }
    } else if magnitude > 2_147_483_647 {
        Ok(i32::MAX)
    } else {
        Ok(i32::try_from(magnitude).unwrap_or(i32::MAX))
    }
}

fn legacy_strtoul32(value: &MonarchQueryValue, column: usize) -> Result<u32, MonarchRowError> {
    let bytes = legacy_c_string_bytes(value, column)?;
    let (negative, magnitude, overflow) = legacy_numeric_prefix(bytes);
    let Some(magnitude) = magnitude else {
        return Ok(0);
    };
    if overflow || magnitude > u64::from(u32::MAX) {
        return Ok(u32::MAX);
    }
    let magnitude = u32::try_from(magnitude).unwrap_or(u32::MAX);
    Ok(if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    })
}

fn legacy_strtoull64(value: &MonarchQueryValue, column: usize) -> Result<u64, MonarchRowError> {
    let bytes = legacy_c_string_bytes(value, column)?;
    let (negative, magnitude, overflow) = legacy_numeric_prefix(bytes);
    let Some(magnitude) = magnitude else {
        return Ok(0);
    };
    if overflow {
        return Ok(u64::MAX);
    }
    Ok(if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    })
}

fn decode_text_legacy(
    value: &MonarchQueryValue,
    column: usize,
) -> Result<[u8; MONARCH_TEXT_BYTES], MonarchRowError> {
    let bytes = legacy_c_string_bytes(value, column)?;
    let length = bytes.len().min(MONARCH_TEXT_CONTENT_BYTES);
    let mut output = [0_u8; MONARCH_TEXT_BYTES];
    output[..length].copy_from_slice(&bytes[..length]);
    Ok(output)
}

fn decode_empire(value: &MonarchQueryValue, legacy: bool) -> Result<u8, MonarchRowError> {
    let parsed = if legacy {
        legacy_strtol32(value, 0)?
    } else {
        decode_strict_i32(value, 0, "i32 empire")?
    };
    u8::try_from(parsed)
        .ok()
        .filter(|empire| *empire < u8::try_from(MONARCH_MAX_SOURCE_ROWS).unwrap_or(u8::MAX))
        .ok_or(MonarchRowError::EmpireOutOfRange { value: parsed })
}

fn decode_row(row: &MonarchQueryRow, legacy: bool) -> Result<DecodedMonarchRow, MonarchRowError> {
    check_row_width(row)?;
    let empire = decode_empire(&row.columns[0], legacy)?;
    let pid = if legacy {
        legacy_strtoul32(&row.columns[1], 1)?
    } else {
        decode_strict_u32(&row.columns[1], 1, "u32")?
    };
    let name = if legacy {
        decode_text_legacy(&row.columns[2], 2)?
    } else {
        decode_text_strict(&row.columns[2], 2)?
    };
    let money = if legacy {
        i64::from_ne_bytes(legacy_strtoull64(&row.columns[3], 3)?.to_ne_bytes())
    } else {
        decode_strict_i64(&row.columns[3], 3, "i64")?
    };
    let date = if legacy {
        decode_text_legacy(&row.columns[4], 4)?
    } else {
        decode_text_strict(&row.columns[4], 4)?
    };
    Ok(DecodedMonarchRow {
        empire,
        pid,
        name,
        money,
        date,
    })
}

/// Strictly decode one five-cell Monarch row.
///
/// All numeric cells must be complete ASCII decimal values. The empire must
/// be in `0..=3`. Raw non-UTF-8 names and dates are accepted. Empty strings
/// are distinct from SQL `NULL`; an empty text cell produces an all-zero C
/// array. A 32-byte raw cell is accepted only when its final byte is the one
/// terminal NUL. Earlier NULs and unterminated 32-byte cells are rejected.
///
/// # Errors
///
/// Returns [`MonarchRowError`] for a wrong-width row, SQL `NULL`, source
/// error, malformed or overflowing number, invalid empire, or unsafe text.
pub fn decode_monarch_query_row(
    row: &MonarchQueryRow,
) -> Result<DecodedMonarchRow, MonarchRowError> {
    decode_row(row, false)
}

/// Decode one five-cell row with the explicitly named legacy C policy.
///
/// SQL `NULL` and source-error cells remain failures. Empty or nonnumeric
/// prefixes convert to zero. Numeric parsing accepts C ASCII whitespace, an
/// optional sign, and a decimal prefix. PID uses active-x86 32-bit
/// `strtoul` behavior; money uses 64-bit `strtoull` followed by the source cast
/// to `i64`. Names and dates use first-NUL semantics and copy at most 31 bytes
/// before zero-filling. The resulting empire must still be in `0..=3`; the
/// source's unchecked out-of-bounds write is never reproduced.
///
/// # Errors
///
/// Returns [`MonarchRowError`] for a wrong-width row, SQL `NULL`, source
/// error, or an empire outside the four fixed slots.
pub fn decode_monarch_query_row_legacy(
    row: &MonarchQueryRow,
) -> Result<DecodedMonarchRow, MonarchRowError> {
    decode_row(row, true)
}

fn build_monarch_info_selected(
    rows: &[MonarchQueryRow],
    max_rows: usize,
    legacy: bool,
) -> Result<BootMonarchInfo, MonarchBuildError> {
    let effective_max = max_rows.min(MONARCH_MAX_SOURCE_ROWS);
    if rows.len() > effective_max {
        return Err(MonarchBuildError::TooManyRows {
            count: rows.len(),
            maximum: effective_max,
        });
    }

    let mut output = BootMonarchInfo {
        pid: [0; 4],
        money: [0; 4],
        name: [[0; MONARCH_TEXT_BYTES]; 4],
        date: [[0; MONARCH_TEXT_BYTES]; 4],
    };
    let mut first_row = [None; MONARCH_MAX_SOURCE_ROWS];
    for (index, row) in rows.iter().enumerate() {
        let decoded = if legacy {
            decode_monarch_query_row_legacy(row)
        } else {
            decode_monarch_query_row(row)
        }
        .map_err(|source| MonarchBuildError::Row { index, source })?;
        let slot = usize::from(decoded.empire);
        if let Some(first) = first_row[slot] {
            return Err(MonarchBuildError::DuplicateEmpire {
                empire: decoded.empire,
                first_row: first,
                duplicate_row: index,
            });
        }
        first_row[slot] = Some(index);
        output.pid[slot] = decoded.pid;
        output.money[slot] = decoded.money;
        output.name[slot] = decoded.name;
        output.date[slot] = decoded.date;
    }
    Ok(output)
}

/// Strictly build a fresh fixed Monarch value from at most four rows.
///
/// Rows have no reliable SQL order. The validated empire selects its output
/// slot, duplicate empires are rejected, and missing slots remain zero. The
/// builder allocates no shared state and cannot retain a prior result.
///
/// # Errors
///
/// Returns [`MonarchBuildError`] for too many rows, duplicate empires, or a
/// row rejected by the strict policy.
pub fn build_monarch_info(rows: &[MonarchQueryRow]) -> Result<BootMonarchInfo, MonarchBuildError> {
    build_monarch_info_with_limit(rows, MONARCH_MAX_SOURCE_ROWS)
}

/// Strictly build a value with an additional caller-selected row cap.
///
/// # Errors
///
/// Returns [`MonarchBuildError`] for the effective row cap, duplicate empires,
/// or a row rejected by the strict policy.
pub fn build_monarch_info_with_limit(
    rows: &[MonarchQueryRow],
    max_rows: usize,
) -> Result<BootMonarchInfo, MonarchBuildError> {
    build_monarch_info_selected(rows, max_rows, false)
}

/// Build a fresh value with the explicitly named legacy conversion policy.
///
/// Slot assignment, duplicate rejection, the four-row bound, and zero-filled
/// missing slots are identical to [`build_monarch_info`].
///
/// # Errors
///
/// Returns [`MonarchBuildError`] for too many rows, duplicate empires, or a
/// row rejected by the legacy policy.
pub fn build_monarch_info_legacy(
    rows: &[MonarchQueryRow],
) -> Result<BootMonarchInfo, MonarchBuildError> {
    build_monarch_info_legacy_with_limit(rows, MONARCH_MAX_SOURCE_ROWS)
}

/// Build a value with the legacy conversion and an additional row cap.
///
/// # Errors
///
/// Returns [`MonarchBuildError`] for the effective row cap, duplicate empires,
/// or a row rejected by the legacy policy.
pub fn build_monarch_info_legacy_with_limit(
    rows: &[MonarchQueryRow],
    max_rows: usize,
) -> Result<BootMonarchInfo, MonarchBuildError> {
    build_monarch_info_selected(rows, max_rows, true)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::error::Error as _;

    use protocol::db_boot::{BootMonarchInfo, MONARCH_INFO_WIRE_SIZE};

    use super::*;
    use crate::postfix::TablePostfix;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct SourceFailure;

    impl fmt::Display for SourceFailure {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("db down")
        }
    }

    impl Error for SourceFailure {}

    fn bytes(value: &[u8]) -> MonarchQueryValue {
        MonarchQueryValue::bytes(value.to_vec())
    }

    fn text(value: &str) -> MonarchQueryValue {
        MonarchQueryValue::text(value)
    }

    fn valid_cells() -> [MonarchQueryValue; MONARCH_QUERY_COLUMN_COUNT] {
        [
            text("0"),
            text("42"),
            text("Queen"),
            text("-9"),
            text("2026-09-24"),
        ]
    }

    fn valid_row() -> MonarchQueryRow {
        MonarchQueryRow::from_typed_columns(valid_cells())
    }

    fn row_with(
        empire: MonarchQueryValue,
        pid: MonarchQueryValue,
        name: MonarchQueryValue,
        money: MonarchQueryValue,
        date: MonarchQueryValue,
    ) -> MonarchQueryRow {
        MonarchQueryRow::from_typed_columns([empire, pid, name, money, date])
    }

    fn replace(row: &mut MonarchQueryRow, column: usize, value: MonarchQueryValue) {
        let mut cells = row.clone().into_columns();
        cells[column] = value;
        *row = MonarchQueryRow::new(cells);
    }

    fn loader(limit: usize) -> MonarchLoader {
        MonarchLoader::new(
            &TablePostfix::parse("_test").unwrap(),
            MonarchLimits::new(limit),
        )
        .unwrap()
    }

    fn expected_wire_bytes(info: &BootMonarchInfo) -> Vec<u8> {
        let mut wire = Vec::with_capacity(MONARCH_INFO_WIRE_SIZE);
        for pid in info.pid {
            wire.extend_from_slice(&pid.to_le_bytes());
        }
        for money in info.money {
            wire.extend_from_slice(&money.to_le_bytes());
        }
        for name in info.name {
            wire.extend_from_slice(&name);
        }
        for date in info.date {
            wire.extend_from_slice(&date);
        }
        assert_eq!(wire.len(), MONARCH_INFO_WIRE_SIZE);
        wire
    }

    #[test]
    fn exact_query_metadata_and_statement_are_source_fixed() {
        assert_eq!(MONARCH_QUERY_COLUMN_COUNT, 5);
        assert_eq!(MONARCH_TABLE_QUERY_COLUMNS, 5);
        assert_eq!(MONARCH_MAX_SOURCE_ROWS, 4);
        assert_eq!(MONARCH_OUTPUT_WIRE_SIZE, 304);
        assert_eq!(MONARCH_INFO_WIRE_SIZE, 304);
        assert_eq!(MONARCH_QUERY_BASE_BYTES, 92);
        assert_eq!(MAX_MONARCH_QUERY_BYTES, 255);
        assert_eq!(MAX_MONARCH_POSTFIX_BYTES, 163);
        assert_eq!(
            MONARCH_QUERY_COLUMN_NAMES,
            ["empire", "pid", "name", "money", "windate"]
        );

        let query = MonarchQuery::from_config(None).unwrap();
        assert_eq!(
            query.as_str(),
            "SELECT a.empire, a.pid, b.name, a.money, a.windate FROM monarch a, player b WHERE a.pid=b.id"
        );
        assert_eq!(query.as_str().len(), MONARCH_QUERY_BASE_BYTES);
        assert_eq!(query.table_name(), "player");
        assert!(query.postfix().is_empty());
        assert!(!query.as_str().contains("monarch_test"));
        assert!(!query.as_str().contains(';'));
        assert!(!query.as_str().contains("ORDER BY"));
        assert!(!query.as_str().contains("LIMIT"));

        let postfixed = MonarchQuery::from_config(Some("_prod")).unwrap();
        assert_eq!(postfixed.table_name(), "player_prod");
        assert!(postfixed
            .as_str()
            .contains("FROM monarch a, player_prod b WHERE"));
    }

    #[test]
    fn postfix_and_rendered_statement_bounds_never_truncate() {
        let max_postfix = "a".repeat(MAX_MONARCH_POSTFIX_BYTES);
        let query = MonarchQuery::from_config(Some(&max_postfix)).unwrap();
        assert_eq!(query.as_str().len(), MAX_MONARCH_QUERY_BYTES);
        assert_eq!(query.postfix().as_str(), max_postfix);

        let too_long = "a".repeat(MAX_MONARCH_POSTFIX_BYTES + 1);
        assert_eq!(
            MonarchQuery::from_config(Some(&too_long)),
            Err(MonarchBoundaryError::Query(
                MonarchQueryBuildError::QueryTooLong {
                    length: 256,
                    maximum: 255,
                }
            ))
        );

        assert!(matches!(
            MonarchQuery::from_config(Some("bad-postfix")),
            Err(MonarchBoundaryError::Postfix(_))
        ));
    }

    #[test]
    fn typed_row_constructor_checks_exact_width_with_bounded_pulls() {
        for width in 0..MONARCH_QUERY_COLUMN_COUNT {
            let cells = vec![text("0"); width];
            assert_eq!(
                MonarchQueryRow::try_new(cells),
                Err(MonarchRowError::ColumnCount {
                    expected: 5,
                    actual: width,
                })
            );
        }
        assert!(MonarchQueryRow::try_new(vec![text("0"); 5]).is_ok());

        let pulls = Cell::new(0_usize);
        let cells = (0..10_000)
            .inspect(|_| pulls.set(pulls.get() + 1))
            .map(|_| text("0"));
        assert_eq!(
            MonarchQueryRow::try_new(cells),
            Err(MonarchRowError::ColumnCount {
                expected: 5,
                actual: 6,
            })
        );
        assert_eq!(pulls.get(), 6);
    }

    #[test]
    fn query_cells_preserve_null_raw_nul_and_utf8_without_normalization() {
        assert_eq!(
            MonarchQueryValue::from(None::<Vec<u8>>),
            MonarchQueryValue::Null
        );
        assert_eq!(
            MonarchQueryValue::from(None::<String>),
            MonarchQueryValue::Null
        );
        let raw_value = vec![0xff, 0, b'7', 0x80];
        assert_eq!(
            MonarchQueryValue::from(raw_value.clone()),
            MonarchQueryValue::Bytes(raw_value)
        );

        let sample_row = row_with(
            bytes(&[0, b'2', 0]),
            bytes("01".as_bytes()),
            bytes(&[0xc3, 0x28, 0xff]),
            text("0"),
            bytes(&[0]),
        );
        assert_eq!(sample_row.columns()[0], bytes(&[0, b'2', 0]));
        assert_eq!(sample_row.columns()[2], bytes(&[0xc3, 0x28, 0xff]));
        assert_eq!(sample_row.columns()[4], bytes(&[0]));
    }

    #[test]
    fn strict_decoder_preserves_five_column_order_and_raw_text() {
        let row = row_with(
            text("3"),
            text("4294967295"),
            bytes(&[0xff, b'A', 0x80]),
            text("-9223372036854775808"),
            bytes(b"2026-09-24"),
        );
        let decoded = decode_monarch_query_row(&row).unwrap();
        assert_eq!(decoded.empire, 3);
        assert_eq!(decoded.pid, u32::MAX);
        assert_eq!(&decoded.name[..3], &[0xff, b'A', 0x80]);
        assert_eq!(decoded.name[3..], [0_u8; 29]);
        assert_eq!(decoded.money, i64::MIN);
        assert_eq!(&decoded.date[..10], b"2026-09-24");
        assert_eq!(decoded.date[10..], [0_u8; 22]);
    }

    #[test]
    fn strict_null_and_source_errors_remain_distinct_at_each_column() {
        for column in 0..MONARCH_QUERY_COLUMN_COUNT {
            let mut row = valid_row();
            replace(&mut row, column, MonarchQueryValue::Null);
            assert_eq!(
                decode_monarch_query_row(&row),
                Err(MonarchRowError::Null { column })
            );

            replace(&mut row, column, MonarchQueryValue::error("read failed"));
            assert_eq!(
                decode_monarch_query_row(&row),
                Err(MonarchRowError::Source {
                    column,
                    message: "read failed".to_owned(),
                })
            );
        }
    }

    #[test]
    fn strict_numeric_cells_require_complete_destination_width_text() {
        for bad in ["", " ", "+1", "-1", "12junk", "1.0", "0x10"] {
            let mut row = valid_row();
            replace(&mut row, 1, text(bad));
            assert!(matches!(
                decode_monarch_query_row(&row),
                Err(MonarchRowError::InvalidNumber { column: 1, .. })
            ));
        }
        for boundary in ["0", "4294967295"] {
            let mut row = valid_row();
            replace(&mut row, 1, text(boundary));
            assert!(decode_monarch_query_row(&row).is_ok());
        }
        let mut row = valid_row();
        replace(&mut row, 1, text("4294967296"));
        assert!(matches!(
            decode_monarch_query_row(&row),
            Err(MonarchRowError::NumberOverflow { column: 1, .. })
        ));

        for bad in ["", " 1", "+1", "1 ", "1x"] {
            let mut row = valid_row();
            replace(&mut row, 3, text(bad));
            assert!(matches!(
                decode_monarch_query_row(&row),
                Err(MonarchRowError::InvalidNumber { column: 3, .. })
            ));
        }
        for (source, expected) in [
            ("-9223372036854775808", i64::MIN),
            ("9223372036854775807", i64::MAX),
        ] {
            let mut row = valid_row();
            replace(&mut row, 3, text(source));
            assert_eq!(decode_monarch_query_row(&row).unwrap().money, expected);
        }
        for overflow in ["9223372036854775808", "-9223372036854775809"] {
            let mut row = valid_row();
            replace(&mut row, 3, text(overflow));
            assert!(matches!(
                decode_monarch_query_row(&row),
                Err(MonarchRowError::NumberOverflow { column: 3, .. })
            ));
        }
    }

    #[test]
    fn strict_empire_accepts_only_zero_through_three() {
        for (source, expected) in [("0", 0), ("3", 3)] {
            let mut row = valid_row();
            replace(&mut row, 0, text(source));
            assert_eq!(decode_monarch_query_row(&row).unwrap().empire, expected);
        }
        for source in ["-1", "4"] {
            let mut row = valid_row();
            replace(&mut row, 0, text(source));
            let expected = source.parse::<i32>().unwrap();
            assert_eq!(
                decode_monarch_query_row(&row),
                Err(MonarchRowError::EmpireOutOfRange { value: expected })
            );
        }
        let mut row = valid_row();
        replace(&mut row, 0, text("2147483648"));
        assert!(matches!(
            decode_monarch_query_row(&row),
            Err(MonarchRowError::NumberOverflow { column: 0, .. })
        ));
    }

    #[test]
    fn strict_names_and_dates_follow_32_byte_c_array_policy() {
        for column in [2, 4] {
            let mut empty = valid_row();
            replace(&mut empty, column, bytes(&[]));
            let decoded = decode_monarch_query_row(&empty).unwrap();
            let output = if column == 2 {
                decoded.name
            } else {
                decoded.date
            };
            assert_eq!(output, [0_u8; MONARCH_TEXT_BYTES]);

            let mut max_content = valid_row();
            replace(&mut max_content, column, bytes(&[b'x'; 31]));
            assert!(decode_monarch_query_row(&max_content).is_ok());

            let mut terminal = valid_row();
            let mut terminal_bytes = vec![b'x'; 31];
            terminal_bytes.push(0);
            replace(&mut terminal, column, bytes(&terminal_bytes));
            assert!(decode_monarch_query_row(&terminal).is_ok());

            let mut no_terminator = valid_row();
            replace(&mut no_terminator, column, bytes(&[b'x'; 32]));
            assert_eq!(
                decode_monarch_query_row(&no_terminator),
                Err(MonarchRowError::TextTooLong {
                    column,
                    length: 32,
                    maximum: 31,
                })
            );

            let mut interior = valid_row();
            replace(&mut interior, column, bytes(b"ab\0cd"));
            assert_eq!(
                decode_monarch_query_row(&interior),
                Err(MonarchRowError::TextInteriorNul { column, index: 2 })
            );

            let mut invalid_utf8 = valid_row();
            replace(&mut invalid_utf8, column, bytes(&[0xff, 0xfe, 0x80]));
            assert!(decode_monarch_query_row(&invalid_utf8).is_ok());
        }
    }

    #[test]
    fn legacy_decoder_keeps_null_distinct_and_models_c_prefixes() {
        for column in 0..MONARCH_QUERY_COLUMN_COUNT {
            let mut row = valid_row();
            replace(&mut row, column, MonarchQueryValue::Null);
            assert_eq!(
                decode_monarch_query_row_legacy(&row),
                Err(MonarchRowError::Null { column })
            );
        }

        let row = row_with(
            bytes(b" \t\r\n\x0b\x0c+002ignored"),
            bytes(b"  -1suffix"),
            bytes(b""),
            bytes(b"18446744073709551615tail"),
            bytes(b"date"),
        );
        let decoded = decode_monarch_query_row_legacy(&row).unwrap();
        assert_eq!(decoded.empire, 2);
        assert_eq!(decoded.pid, u32::MAX);
        assert_eq!(decoded.money, -1);
        assert_eq!(decoded.name, [0_u8; 32]);

        let mut empty_pid = valid_row();
        replace(&mut empty_pid, 1, bytes(b""));
        assert_eq!(decode_monarch_query_row_legacy(&empty_pid).unwrap().pid, 0);
        let mut empty_money = valid_row();
        replace(&mut empty_money, 3, bytes(b"not-a-number"));
        assert_eq!(
            decode_monarch_query_row_legacy(&empty_money).unwrap().money,
            0
        );
    }

    #[test]
    fn legacy_numeric_boundaries_match_active_x86_helpers() {
        let mut pid = valid_row();
        replace(&mut pid, 1, bytes(b"4294967295"));
        assert_eq!(decode_monarch_query_row_legacy(&pid).unwrap().pid, u32::MAX);
        replace(&mut pid, 1, bytes(b"4294967296"));
        assert_eq!(decode_monarch_query_row_legacy(&pid).unwrap().pid, u32::MAX);
        replace(&mut pid, 1, bytes(b"-2"));
        assert_eq!(
            decode_monarch_query_row_legacy(&pid).unwrap().pid,
            u32::MAX - 1
        );

        let mut money = valid_row();
        for (source, expected) in [
            ("0", 0_i64),
            ("-1", -1),
            ("18446744073709551615", -1),
            ("18446744073709551616", -1),
        ] {
            replace(&mut money, 3, bytes(source.as_bytes()));
            assert_eq!(
                decode_monarch_query_row_legacy(&money).unwrap().money,
                expected
            );
        }
    }

    #[test]
    fn legacy_empire_uses_strtol_prefix_but_never_invalid_slot() {
        for (source, expected) in [(" 2tail", 2), ("+3x", 3), ("junk", 0)] {
            let mut row = valid_row();
            replace(&mut row, 0, bytes(source.as_bytes()));
            assert_eq!(
                decode_monarch_query_row_legacy(&row).unwrap().empire,
                expected
            );
        }
        for (source, expected) in [("-1", -1), ("4x", 4), ("999999999999x", i32::MAX)] {
            let mut row = valid_row();
            replace(&mut row, 0, bytes(source.as_bytes()));
            assert_eq!(
                decode_monarch_query_row_legacy(&row),
                Err(MonarchRowError::EmpireOutOfRange { value: expected })
            );
        }
    }

    #[test]
    fn legacy_names_and_dates_use_first_nul_and_31_byte_truncation() {
        for column in [2, 4] {
            let mut first_nul = valid_row();
            replace(&mut first_nul, column, bytes(b"visible\0ignored-and-raw"));
            let decoded = decode_monarch_query_row_legacy(&first_nul).unwrap();
            let output = if column == 2 {
                decoded.name
            } else {
                decoded.date
            };
            assert_eq!(&output[..7], b"visible");
            assert_eq!(output[7..], [0_u8; 25]);

            let mut unterminated = valid_row();
            replace(&mut unterminated, column, bytes(&[0xff; 40]));
            let decoded = decode_monarch_query_row_legacy(&unterminated).unwrap();
            let output = if column == 2 {
                decoded.name
            } else {
                decoded.date
            };
            assert_eq!(&output[..31], &[0xff; 31]);
            assert_eq!(output[31], 0);

            let mut terminal = valid_row();
            let mut value = vec![b'x'; 31];
            value.push(0);
            replace(&mut terminal, column, bytes(&value));
            assert!(decode_monarch_query_row_legacy(&terminal).is_ok());
        }
    }

    #[test]
    fn builder_uses_empire_slots_and_zero_initializes_missing_slots() {
        let rows = vec![
            row_with(text("3"), text("3"), text("third"), text("3"), text("d3")),
            row_with(text("1"), text("1"), text("first"), text("1"), text("d1")),
        ];
        let info = build_monarch_info(&rows).unwrap();
        assert_eq!(info.pid, [0, 1, 0, 3]);
        assert_eq!(info.money, [0, 1, 0, 3]);
        assert_eq!(&info.name[1][..5], b"first");
        assert_eq!(&info.date[3][..2], b"d3");
        assert_eq!(info.name[0], [0_u8; 32]);
        assert_eq!(info.date[2], [0_u8; 32]);

        let wire = expected_wire_bytes(&info);
        assert_eq!(wire.len(), MONARCH_INFO_WIRE_SIZE);
        assert_eq!(BootMonarchInfo::decode(&wire).unwrap(), info);
    }

    #[test]
    fn builder_rejects_duplicates_extra_rows_and_wrong_widths() {
        let duplicate = vec![valid_row(), valid_row()];
        assert_eq!(
            build_monarch_info(&duplicate),
            Err(MonarchBuildError::DuplicateEmpire {
                empire: 0,
                first_row: 0,
                duplicate_row: 1,
            })
        );

        let mut rows = Vec::new();
        for empire in 0..=4 {
            let mut row = valid_row();
            replace(&mut row, 0, text(&empire.to_string()));
            rows.push(row);
        }
        assert_eq!(
            build_monarch_info(&rows),
            Err(MonarchBuildError::TooManyRows {
                count: 5,
                maximum: 4,
            })
        );
        assert_eq!(
            build_monarch_info_with_limit(&rows[..2], 1),
            Err(MonarchBuildError::TooManyRows {
                count: 2,
                maximum: 1,
            })
        );

        let short = MonarchQueryRow::new(vec![text("0"); 4]);
        assert!(matches!(
            build_monarch_info(&[short]),
            Err(MonarchBuildError::Row {
                index: 0,
                source: MonarchRowError::ColumnCount {
                    expected: 5,
                    actual: 4
                }
            })
        ));
    }

    #[test]
    fn repeated_loader_calls_replace_results_without_stale_state() {
        let first = vec![row_with(
            text("0"),
            text("10"),
            text("old"),
            text("100"),
            text("old-date"),
        )];
        let second = vec![row_with(
            text("3"),
            text("30"),
            text("new"),
            text("300"),
            text("new-date"),
        )];
        let responses = RefCell::new(vec![first, second]);
        let source = |query: &MonarchQuery| {
            assert_eq!(query.table_name(), "player_test");
            Ok::<_, std::convert::Infallible>(responses.borrow_mut().remove(0))
        };

        let loader = loader(4);
        let old = loader.load(&source).unwrap();
        let new = loader.load(&source).unwrap();
        assert_eq!(old.pid[0], 10);
        assert_eq!(new.pid, [0, 0, 0, 30]);
        assert_eq!(new.money, [0, 0, 0, 300]);
        assert_eq!(new.name[0], [0_u8; 32]);
        assert_eq!(&new.name[3][..3], b"new");
    }

    #[test]
    fn injected_loader_distinguishes_zero_rows_from_source_error() {
        let empty_loader = loader(4);
        let empty: fn(&MonarchQuery) -> Result<Vec<MonarchQueryRow>, std::convert::Infallible> =
            |_| Ok(Vec::new());
        assert!(matches!(
            empty_loader.load(&empty),
            Err(MonarchLoadError::EmptyResult)
        ));

        let failed: fn(&MonarchQuery) -> Result<Vec<MonarchQueryRow>, SourceFailure> =
            |_| Err(SourceFailure);
        let source_error = empty_loader.load(&failed);
        assert_eq!(source_error, Err(MonarchLoadError::Source(SourceFailure)));
        assert!(source_error
            .unwrap_err()
            .source()
            .is_some_and(|source| source.to_string() == "db down"));
    }

    #[test]
    fn legacy_loader_is_explicit_and_keeps_the_same_slot_policy() {
        let loader = loader(4);
        let rows = vec![row_with(
            bytes(b"1tail"),
            bytes(b"7tail"),
            bytes(&[b'x'; 40]),
            bytes(b"-1tail"),
            bytes(b"d"),
        )];
        let source = |_: &MonarchQuery| Ok::<_, std::convert::Infallible>(rows.clone());
        let info = loader.load_legacy(&source).unwrap();
        assert_eq!(info.pid[1], 7);
        assert_eq!(info.money[1], -1);
        assert_eq!(info.name[1][31], 0);
    }
}
