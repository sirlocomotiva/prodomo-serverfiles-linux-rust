//! Pure, SQL-free conversion of the source-fixed `object` boot table.
//!
//! The legacy loader selects ten cells in this exact order:
//! `id`, `land_id`, `vnum`, `map_index`, `x`, `y`, `x_rot`, `y_rot`, `z_rot`,
//! and `life`.  The active x86 `building::TObject` is a 40-byte record with no
//! padding.  This module keeps SQL text, SQL `NULL`, and source errors distinct
//! until a caller selects a decoding policy.  It does not execute SQL or
//! mutate the legacy object map.
//!
//! [`decode_object_query_row`] is the default strict policy. It requires
//! complete decimal integers or finite decimal floats, rejects SQL `NULL` and
//! target-width overflow, and preserves source row order and duplicates in the
//! explicitly named raw-section builders. [`decode_object_query_row_legacy`]
//! is the separately named compatibility policy for C++ `str_to_number`-style
//! prefixes, zero-initialized `NULL` destinations, and target-width casts.
//! The canonical section builders and `SQLx` adapter apply the legacy map policy:
//! first row per ID wins, then selected IDs are emitted in ascending order.
//! The raw conversion remains available when acquisition diagnostics need the
//! original SQL order.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootSection, BootSectionKind};
use protocol::db_records::{ObjectRecord, OBJECT_RECORD_WIRE_SIZE};

/// Number of columns selected by the fixed legacy object query.
pub const OBJECT_TABLE_QUERY_COLUMNS: usize = 10;

/// Alias for [`OBJECT_TABLE_QUERY_COLUMNS`].
pub const OBJECT_QUERY_COLUMN_COUNT: usize = OBJECT_TABLE_QUERY_COLUMNS;

/// Maximum number of records representable by the legacy `u16` section count.
pub const OBJECT_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Exact fixed 40-byte x86 `building::TObject` section width.
pub const OBJECT_SECTION_RECORD_SIZE: u16 = 40;

/// Alias for the source-fixed record width.
pub const OBJECT_TABLE_WIRE_SIZE: usize = OBJECT_RECORD_WIRE_SIZE;

/// Maximum byte length of a section when every representable count is used.
pub const OBJECT_TABLE_MAX_SECTION_BYTES: usize =
    OBJECT_RECORD_WIRE_SIZE * OBJECT_TABLE_MAX_RECORDS;

/// Column names in the fixed legacy query order.
pub const OBJECT_TABLE_QUERY_COLUMN_NAMES: [&str; OBJECT_TABLE_QUERY_COLUMNS] = [
    "id",
    "land_id",
    "vnum",
    "map_index",
    "x",
    "y",
    "x_rot",
    "y_rot",
    "z_rot",
    "life",
];

/// One value returned by a query-row adapter.
///
/// `Text` is the only state accepted by the strict numeric decoder. `Null`
/// and `Error` retain the distinct facts a database column can produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectQueryValue {
    /// A non-NULL textual value, normally the original SQL result string.
    Text(String),
    /// A SQL `NULL` value.
    Null,
    /// An error raised while obtaining or converting one query column.
    Error(String),
}

impl ObjectQueryValue {
    /// Construct a non-NULL query value.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    /// Construct a SQL `NULL` query value.
    #[must_use]
    pub const fn null() -> Self {
        Self::Null
    }

    /// Construct a column extraction or source error.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error(message.into())
    }
}

impl From<String> for ObjectQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for ObjectQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

/// A row-shaped value for the ten-column object query.
///
/// The vector is retained so malformed source width can be reported without
/// indexing past the input. [`Self::try_new`] validates the width immediately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectTableQueryRow {
    columns: Vec<ObjectQueryValue>,
}

impl ObjectTableQueryRow {
    /// Construct a row while retaining supplied values and their order.
    #[must_use]
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ObjectQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub fn from_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ObjectQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct and validate a ten-column row.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectRowError::ColumnCount`] when the iterator does not
    /// produce exactly ten values.
    pub fn try_new<I>(columns: I) -> Result<Self, ObjectRowError>
    where
        I: IntoIterator<Item = ObjectQueryValue>,
    {
        let row = Self::new(columns);
        if row.columns.len() != OBJECT_TABLE_QUERY_COLUMNS {
            return Err(ObjectRowError::ColumnCount {
                expected: OBJECT_TABLE_QUERY_COLUMNS,
                actual: row.columns.len(),
            });
        }
        Ok(row)
    }

    /// Construct a row from a statically sized, correctly shaped column set.
    #[must_use]
    pub fn from_typed_columns(columns: [ObjectQueryValue; OBJECT_TABLE_QUERY_COLUMNS]) -> Self {
        Self::new(columns)
    }

    /// Borrow all supplied columns in query order.
    #[must_use]
    pub fn columns(&self) -> &[ObjectQueryValue] {
        &self.columns
    }

    /// Consume the row and return its columns in query order.
    #[must_use]
    pub fn into_columns(self) -> Vec<ObjectQueryValue> {
        self.columns
    }

    /// Return the number of columns supplied by the source.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

impl From<[ObjectQueryValue; OBJECT_TABLE_QUERY_COLUMNS]> for ObjectTableQueryRow {
    fn from(columns: [ObjectQueryValue; OBJECT_TABLE_QUERY_COLUMNS]) -> Self {
        Self::from_typed_columns(columns)
    }
}

impl TryFrom<Vec<ObjectQueryValue>> for ObjectTableQueryRow {
    type Error = ObjectRowError;

    fn try_from(columns: Vec<ObjectQueryValue>) -> Result<Self, Self::Error> {
        Self::try_new(columns)
    }
}

/// Return a stable column name for diagnostics.
#[must_use]
pub const fn object_query_column_name(index: usize) -> &'static str {
    match index {
        0 => "id",
        1 => "land_id",
        2 => "vnum",
        3 => "map_index",
        4 => "x",
        5 => "y",
        6 => "x_rot",
        7 => "y_rot",
        8 => "z_rot",
        9 => "life",
        _ => "unknown",
    }
}

/// A strict or explicitly selected compatibility error in one object row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectRowError {
    /// The row did not contain exactly ten columns.
    ColumnCount {
        /// Required column count.
        expected: usize,
        /// Supplied column count.
        actual: usize,
    },
    /// A required column was SQL `NULL` in the strict decoder.
    Null {
        /// Zero-based query column index.
        column: usize,
    },
    /// A required column could not be obtained or converted by the source.
    Source {
        /// Zero-based query column index.
        column: usize,
        /// Source diagnostic.
        message: String,
    },
    /// A non-NULL value was not a strict numeric value.
    InvalidNumber {
        /// Zero-based query column index.
        column: usize,
        /// Original value.
        value: String,
        /// Target type.
        target: &'static str,
    },
    /// A syntactically valid value did not fit its target type.
    NumberOverflow {
        /// Zero-based query column index.
        column: usize,
        /// Original value.
        value: String,
        /// Target type.
        target: &'static str,
    },
}

impl fmt::Display for ObjectRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => write!(
                formatter,
                "object query row has {actual} columns; expected {expected}"
            ),
            Self::Null { column } => write!(
                formatter,
                "object query column {} is NULL",
                object_query_column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "object query column {} could not be read: {message}",
                object_query_column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "object query column {} value {value:?} is not a strict {target}",
                object_query_column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "object query column {} value {value:?} overflows {target}",
                object_query_column_name(*column)
            ),
        }
    }
}

impl Error for ObjectRowError {}

/// Limits applied before allocating an object section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectSectionLimits {
    /// Maximum number of source rows accepted.
    pub max_records: usize,
    /// Maximum number of packed record-data bytes accepted.
    pub max_data_bytes: usize,
}

impl ObjectSectionLimits {
    /// Construct limits that bound records and leave the byte limit unbounded.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            max_records,
            max_data_bytes: usize::MAX,
        }
    }

    /// Construct both record and packed-byte limits.
    #[must_use]
    pub const fn with_data_limit(max_records: usize, max_data_bytes: usize) -> Self {
        Self {
            max_records,
            max_data_bytes,
        }
    }
}

impl Default for ObjectSectionLimits {
    fn default() -> Self {
        Self {
            max_records: OBJECT_TABLE_MAX_RECORDS,
            max_data_bytes: OBJECT_TABLE_MAX_SECTION_BYTES,
        }
    }
}

/// A checked object-section construction error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectSectionError {
    /// The source supplied more rows than the caller allowed.
    TooManyRecords {
        /// Supplied row count.
        count: usize,
        /// Configured row limit.
        maximum: usize,
    },
    /// The row count cannot be represented by the legacy `u16` count.
    CountOverflow {
        /// Supplied row count.
        count: usize,
    },
    /// The fixed record width cannot be represented by `u16`.
    RecordSizeOverflow {
        /// Fixed record width.
        size: usize,
    },
    /// Packed record-data size arithmetic overflowed `usize`.
    DataSizeOverflow {
        /// Row count used in the multiplication.
        count: usize,
    },
    /// A bounded row or output vector could not reserve its requested length.
    AllocationFailed {
        /// Requested allocation units (rows or bytes).
        requested: usize,
    },
    /// The packed record data exceeds the caller's byte limit.
    DataTooLarge {
        /// Required packed record-data length.
        length: usize,
        /// Configured byte limit.
        maximum: usize,
    },
    /// A protocol encoder returned an unexpected record width.
    RecordSizeMismatch {
        /// Zero-based source row index.
        index: usize,
        /// Required packed width.
        expected: usize,
        /// Actual encoded width.
        actual: usize,
    },
    /// A source row could not be decoded.
    Row {
        /// Zero-based source row index.
        index: usize,
        /// Row error.
        source: ObjectRowError,
    },
}

impl fmt::Display for ObjectSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "object section has {count} rows; configured limit is {maximum}"
            ),
            Self::CountOverflow { count } => {
                write!(formatter, "object section count {count} does not fit u16")
            }
            Self::RecordSizeOverflow { size } => {
                write!(formatter, "object record size {size} does not fit u16")
            }
            Self::DataSizeOverflow { count } => {
                write!(
                    formatter,
                    "object section data size overflowed for {count} rows"
                )
            }
            Self::AllocationFailed { requested } => {
                write!(
                    formatter,
                    "object section could not allocate {requested} bytes"
                )
            }
            Self::DataTooLarge { length, maximum } => write!(
                formatter,
                "object section data is {length} bytes; configured limit is {maximum}"
            ),
            Self::RecordSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "object row {index} encoded to {actual} bytes; expected {expected}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "object row {index} is invalid: {source}")
            }
        }
    }
}

impl Error for ObjectSectionError {}

/// Base table name used by the source-fixed object boot loader.
pub const OBJECT_TABLE: &str = "object";

/// Exact fixed-column and ordering clause used by the legacy object query.
pub const OBJECT_TABLE_QUERY_PREFIX: &str =
    "SELECT id, land_id, vnum, map_index, x, y, x_rot, y_rot, z_rot, life FROM ";

/// Maximum statement bytes accepted by this bounded query type.
///
/// The legacy loader uses a 4096-byte buffer, including its NUL terminator.
pub const MAX_OBJECT_TABLE_QUERY_BYTES: usize = 4_095;

const MAX_OBJECT_TABLE_BYTES: usize = OBJECT_TABLE.len() + crate::postfix::MAX_TABLE_POSTFIX_BYTES;

/// A defensive failure while composing the fixed object query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectQueryBuildError {
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
    /// The generated statement exceeded its bounded width.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for ObjectQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated object table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated object identifier has byte {byte:#04x} at offset {index}"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated object query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for ObjectQueryBuildError {}

/// A failure while validating an object loader's postfix or fixed query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(crate::postfix::TablePostfixError),
    /// The fixed query failed a defensive construction check.
    Query(ObjectQueryBuildError),
}

impl fmt::Display for ObjectBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for ObjectBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// One immutable, checked object read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectTableQuery {
    statement: String,
    table_name: String,
    postfix: crate::postfix::TablePostfix,
}

impl ObjectTableQuery {
    /// Build the exact source-fixed read query from a validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectQueryBuildError`] if a defensive identifier or
    /// statement check fails.
    pub fn new(postfix: &crate::postfix::TablePostfix) -> Result<Self, ObjectQueryBuildError> {
        let table_name = format!("{OBJECT_TABLE}{}", postfix.as_str());
        if table_name.len() > MAX_OBJECT_TABLE_BYTES {
            return Err(ObjectQueryBuildError::TableNameTooLong {
                length: table_name.len(),
                maximum: MAX_OBJECT_TABLE_BYTES,
            });
        }
        if let Some((index, byte)) = table_name
            .bytes()
            .enumerate()
            .find(|(_, byte)| !(*byte).is_ascii_alphanumeric() && *byte != b'_')
        {
            return Err(ObjectQueryBuildError::InvalidTableIdentifier { index, byte });
        }
        let mut statement =
            String::with_capacity(OBJECT_TABLE_QUERY_PREFIX.len() + table_name.len());
        statement.push_str(OBJECT_TABLE_QUERY_PREFIX);
        statement.push_str(&table_name);
        statement.push_str(" ORDER BY id");
        if statement.len() > MAX_OBJECT_TABLE_QUERY_BYTES {
            return Err(ObjectQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_OBJECT_TABLE_QUERY_BYTES,
            });
        }
        Ok(Self {
            statement,
            table_name,
            postfix: postfix.clone(),
        })
    }

    /// Validate optional configuration and build the fixed query.
    ///
    /// # Errors
    ///
    /// Returns an invalid-postfix or defensive query error.
    pub fn from_config(configured_postfix: Option<&str>) -> Result<Self, ObjectBoundaryError> {
        let postfix = crate::postfix::TablePostfix::from_config(configured_postfix)
            .map_err(ObjectBoundaryError::Postfix)?;
        Self::new(&postfix).map_err(ObjectBoundaryError::Query)
    }

    /// Borrow the exact query text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Borrow the generated `object` table identifier.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix used to build this query.
    #[must_use]
    pub const fn postfix(&self) -> &crate::postfix::TablePostfix {
        &self.postfix
    }
}

/// A caller-owned source for the checked object read query.
pub trait ObjectTableRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw query rows for the checked statement.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source error
    /// must not be represented as an empty row set.
    fn query_rows(&self, query: &ObjectTableQuery)
        -> Result<Vec<ObjectTableQueryRow>, Self::Error>;
}

impl<F, E> ObjectTableRowSource for F
where
    F: Fn(&ObjectTableQuery) -> Result<Vec<ObjectTableQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(
        &self,
        query: &ObjectTableQuery,
    ) -> Result<Vec<ObjectTableQueryRow>, Self::Error> {
        self(query)
    }
}

/// A reusable, bounded boundary around the fixed object query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectTableLoader {
    query: ObjectTableQuery,
    limits: ObjectSectionLimits,
}

impl ObjectTableLoader {
    /// Construct a loader from an already validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectQueryBuildError`] if the defensive query check fails.
    pub fn new(
        postfix: &crate::postfix::TablePostfix,
        limits: ObjectSectionLimits,
    ) -> Result<Self, ObjectQueryBuildError> {
        Ok(Self {
            query: ObjectTableQuery::new(postfix)?,
            limits,
        })
    }

    /// Validate raw configuration and construct a bounded loader.
    ///
    /// # Errors
    ///
    /// Returns an invalid-postfix or defensive query error.
    pub fn from_config(
        configured_postfix: Option<&str>,
        limits: ObjectSectionLimits,
    ) -> Result<Self, ObjectBoundaryError> {
        let postfix = crate::postfix::TablePostfix::from_config(configured_postfix)
            .map_err(ObjectBoundaryError::Postfix)?;
        Self::new(&postfix, limits).map_err(ObjectBoundaryError::Query)
    }

    /// Borrow the immutable checked query.
    #[must_use]
    pub const fn query(&self) -> &ObjectTableQuery {
        &self.query
    }

    /// Return the row and encoded-byte limits.
    #[must_use]
    pub const fn limits(&self) -> ObjectSectionLimits {
        self.limits
    }

    /// Obtain raw rows and strictly build a map-equivalent object boot section.
    ///
    /// Raw rows retain source order until the pure map policy selects the first
    /// row for each ID and emits selected IDs in ascending order.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectTableLoadError::Source`] for source failures or
    /// [`ObjectTableLoadError::Rows`] for strict row/limit failures.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, ObjectTableLoadError<S::Error>>
    where
        S: ObjectTableRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ObjectTableLoadError::Source)?;
        build_object_map_section_with_limits(&rows, self.limits).map_err(ObjectTableLoadError::Rows)
    }

    /// Obtain raw rows and build a map-equivalent section with the named legacy
    /// conversion policy.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectTableLoadError::Source`] for source failures or
    /// [`ObjectTableLoadError::Rows`] for selected row/limit failures.
    pub fn load_section_legacy<S>(
        &self,
        source: &S,
    ) -> Result<BootSection, ObjectTableLoadError<S::Error>>
    where
        S: ObjectTableRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ObjectTableLoadError::Source)?;
        build_object_map_section_legacy_with_limits(&rows, self.limits)
            .map_err(ObjectTableLoadError::Rows)
    }
}

/// An error while obtaining or decoding rows through the object loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectTableLoadError<E> {
    /// The caller-owned source failed.
    Source(E),
    /// Raw rows failed strict or selected legacy decoding/limits.
    Rows(ObjectSectionError),
}

impl<E: fmt::Display> fmt::Display for ObjectTableLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "object row source failed: {source}"),
            Self::Rows(source) => write!(formatter, "object rows could not be loaded: {source}"),
        }
    }
}

impl<E: Error + 'static> Error for ObjectTableLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Rows(source) => Some(source),
        }
    }
}

fn is_strict_unsigned_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_strict_signed_decimal(value: &str) -> bool {
    let Some(first) = value.as_bytes().first().copied() else {
        return false;
    };
    let digits = if first == b'-' { &value[1..] } else { value };
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_strict_float_text(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii) {
        return false;
    }
    let mut index = usize::from(bytes[0] == b'-');
    let integer_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    let integer_digits = index - integer_start;
    let mut fraction_digits = 0;
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
            fraction_digits += 1;
        }
        if integer_digits == 0 && fraction_digits == 0 {
            return false;
        }
    } else if integer_digits == 0 {
        return false;
    }
    if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
        index += 1;
        if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
            index += 1;
        }
        let exponent_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

fn cell_text(value: &ObjectQueryValue, column: usize) -> Result<&str, ObjectRowError> {
    match value {
        ObjectQueryValue::Text(text) => Ok(text),
        ObjectQueryValue::Null => Err(ObjectRowError::Null { column }),
        ObjectQueryValue::Error(message) => Err(ObjectRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn decode_u32(value: &ObjectQueryValue, column: usize) -> Result<u32, ObjectRowError> {
    let text = cell_text(value, column)?;
    if !is_strict_unsigned_decimal(text) {
        return Err(ObjectRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "u32",
        });
    }
    text.parse::<u32>()
        .map_err(|_| ObjectRowError::NumberOverflow {
            column,
            value: text.to_owned(),
            target: "u32",
        })
}

fn decode_i32(value: &ObjectQueryValue, column: usize) -> Result<i32, ObjectRowError> {
    let text = cell_text(value, column)?;
    if !is_strict_signed_decimal(text) {
        return Err(ObjectRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "i32",
        });
    }
    text.parse::<i32>()
        .map_err(|_| ObjectRowError::NumberOverflow {
            column,
            value: text.to_owned(),
            target: "i32",
        })
}

fn decode_f32(value: &ObjectQueryValue, column: usize) -> Result<f32, ObjectRowError> {
    let text = cell_text(value, column)?;
    if !is_strict_float_text(text) {
        return Err(ObjectRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "f32",
        });
    }
    let number = text
        .parse::<f32>()
        .map_err(|_| ObjectRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "f32",
        })?;
    if !number.is_finite() {
        return Err(ObjectRowError::NumberOverflow {
            column,
            value: text.to_owned(),
            target: "f32",
        });
    }
    Ok(number)
}

/// Strictly decode one source-shaped object row into the protocol record.
///
/// # Errors
///
/// Returns [`ObjectRowError`] for a wrong-width row, SQL `NULL`, source error,
/// malformed number, or target-width overflow.
pub fn decode_object_query_row(row: &ObjectTableQueryRow) -> Result<ObjectRecord, ObjectRowError> {
    if row.columns.len() != OBJECT_TABLE_QUERY_COLUMNS {
        return Err(ObjectRowError::ColumnCount {
            expected: OBJECT_TABLE_QUERY_COLUMNS,
            actual: row.columns.len(),
        });
    }
    Ok(ObjectRecord {
        id: decode_u32(&row.columns[0], 0)?,
        land_id: decode_u32(&row.columns[1], 1)?,
        vnum: decode_u32(&row.columns[2], 2)?,
        map_index: decode_i32(&row.columns[3], 3)?,
        x: decode_i32(&row.columns[4], 4)?,
        y: decode_i32(&row.columns[5], 5)?,
        x_rot: decode_f32(&row.columns[6], 6)?,
        y_rot: decode_f32(&row.columns[7], 7)?,
        z_rot: decode_f32(&row.columns[8], 8)?,
        life: decode_i32(&row.columns[9], 9)?,
    })
}

fn legacy_text(value: &ObjectQueryValue, column: usize) -> Result<&str, ObjectRowError> {
    match value {
        ObjectQueryValue::Text(text) => Ok(text),
        ObjectQueryValue::Null => Ok(""),
        ObjectQueryValue::Error(message) => Err(ObjectRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn skip_legacy_space(value: &str) -> &str {
    value.trim_start_matches(|character: char| character.is_ascii_whitespace())
}

fn leading_integer_text(value: &str) -> Option<(bool, &str)> {
    let value = skip_legacy_space(value);
    let bytes = value.as_bytes();
    let mut index = 0;
    let negative = match bytes.first() {
        Some(b'-') => {
            index = 1;
            true
        }
        Some(b'+') => {
            index = 1;
            false
        }
        _ => false,
    };
    let start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    (index > start).then_some((negative, &value[start..index]))
}

fn legacy_u32(value: &ObjectQueryValue, column: usize) -> Result<u32, ObjectRowError> {
    let text = legacy_text(value, column)?;
    let Some((negative, digits)) = leading_integer_text(text) else {
        return Ok(0);
    };
    let mut result = 0_u32;
    let mut overflow = false;
    for byte in digits.bytes() {
        let digit = u32::from(byte - b'0');
        if result > (u32::MAX - digit) / 10 {
            overflow = true;
            result = u32::MAX;
        } else {
            result = result * 10 + digit;
        }
    }
    if overflow {
        return Ok(u32::MAX);
    }
    Ok(if negative {
        result.wrapping_neg()
    } else {
        result
    })
}

fn legacy_i32(value: &ObjectQueryValue, column: usize) -> Result<i32, ObjectRowError> {
    let text = legacy_text(value, column)?;
    let Some((negative, digits)) = leading_integer_text(text) else {
        return Ok(0);
    };
    let mut magnitude = 0_u32;
    for byte in digits.bytes() {
        let digit = u32::from(byte - b'0');
        if magnitude > (u32::MAX - digit) / 10 {
            magnitude = u32::MAX;
        } else {
            magnitude = magnitude * 10 + digit;
        }
    }
    if negative {
        if magnitude >= 2_147_483_648 {
            Ok(i32::MIN)
        } else {
            Ok(-i32::try_from(magnitude).unwrap_or(i32::MAX))
        }
    } else if magnitude > i32::MAX as u32 {
        Ok(i32::MAX)
    } else {
        Ok(i32::try_from(magnitude).unwrap_or(i32::MAX))
    }
}

fn hex_digit_value(byte: u8) -> Option<u128> {
    match byte {
        b'0'..=b'9' => Some(u128::from(byte - b'0')),
        b'a'..=b'f' => Some(u128::from(byte - b'a') + 10),
        b'A'..=b'F' => Some(u128::from(byte - b'A') + 10),
        _ => None,
    }
}

// Hexadecimal `strtof` prefixes are converted through `f64` before the final
// `f32` narrowing. The casts are intentional at this compatibility boundary.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn legacy_hex_f32(text: &str, negative: bool) -> Option<f32> {
    let bytes = text.as_bytes();
    if bytes.len() < 2 || !bytes[..2].eq_ignore_ascii_case(b"0x") {
        return None;
    }
    let mut index = 2;
    let mut mantissa = 0_u128;
    let mut have_digit = false;
    while index < bytes.len() {
        let Some(digit) = hex_digit_value(bytes[index]) else {
            break;
        };
        have_digit = true;
        mantissa = mantissa
            .checked_mul(16)
            .and_then(|value| value.checked_add(digit))
            .unwrap_or(u128::MAX);
        index += 1;
    }
    let mut fraction_digits = 0_usize;
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() {
            let Some(digit) = hex_digit_value(bytes[index]) else {
                break;
            };
            have_digit = true;
            mantissa = mantissa
                .checked_mul(16)
                .and_then(|value| value.checked_add(digit))
                .unwrap_or(u128::MAX);
            fraction_digits = fraction_digits.saturating_add(1);
            index += 1;
        }
    }
    if !have_digit {
        return Some(if negative { -0.0 } else { 0.0 });
    }
    let mut exponent = 0_i32;
    if index < bytes.len() && matches!(bytes[index], b'p' | b'P') {
        index += 1;
        let negative_exponent = index < bytes.len() && bytes[index] == b'-';
        if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
            index += 1;
        }
        let exponent_start = index;
        let mut magnitude = 0_i32;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            let digit = i32::from(bytes[index] - b'0');
            magnitude = magnitude.saturating_mul(10).saturating_add(digit);
            index += 1;
        }
        if index == exponent_start {
            exponent = 0;
        } else {
            exponent = if negative_exponent {
                -magnitude
            } else {
                magnitude
            };
        }
    }
    let fraction_exponent = i32::try_from(fraction_digits)
        .unwrap_or(i32::MAX)
        .saturating_mul(4);
    let scale = exponent
        .saturating_sub(fraction_exponent)
        .clamp(-100_000, 100_000);
    let magnitude = if scale >= 0 {
        (mantissa as f64) * 2.0_f64.powi(scale)
    } else {
        (mantissa as f64) / 2.0_f64.powi(-scale)
    };
    let value = magnitude as f32;
    Some(if negative { -value } else { value })
}

fn legacy_f32(value: &ObjectQueryValue, column: usize) -> Result<f32, ObjectRowError> {
    let text = skip_legacy_space(legacy_text(value, column)?);
    if text.is_empty() {
        return Ok(0.0);
    }
    let bytes = text.as_bytes();
    let negative = bytes.first() == Some(&b'-');
    let special_start = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let special_prefix = |tag: &[u8]| {
        bytes
            .get(special_start..)
            .and_then(|rest| rest.get(..tag.len()))
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(tag))
    };
    if special_prefix(b"inf") {
        return Ok(if negative {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        });
    }
    if special_prefix(b"nan") {
        return Ok(f32::NAN);
    }
    if let Some(parsed) = legacy_hex_f32(&text[special_start..], negative) {
        return Ok(parsed);
    }
    let mut end = special_start;
    let integer_start = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    let integer_digits = end - integer_start;
    let mut fraction_digits = 0;
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        let fraction_start = end;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        fraction_digits = end - fraction_start;
    }
    if integer_digits == 0 && fraction_digits == 0 {
        return Ok(0.0);
    }
    if end < bytes.len() && matches!(bytes[end], b'e' | b'E') {
        let exponent_start = end;
        end += 1;
        if end < bytes.len() && matches!(bytes[end], b'+' | b'-') {
            end += 1;
        }
        let exponent_digits_start = end;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if exponent_digits_start == end {
            end = exponent_start;
        }
    }
    let prefix = &text[..end];
    prefix
        .parse::<f32>()
        .map_err(|_| ObjectRowError::InvalidNumber {
            column,
            value: prefix.to_owned(),
            target: "f32",
        })
}

/// Decode one source-shaped object row using the named C++ compatibility policy.
///
/// This is intentionally separate from the default strict decoder. It models
/// zero-initialized destinations for SQL `NULL`/empty text, C-style leading
/// whitespace and numeric prefixes (including decimal and hexadecimal forms),
/// and target-width casts used by the legacy loader. It returns a canonical
/// NaN rather than preserving a libc NaN payload. Source extraction errors
/// remain errors.
///
/// # Errors
///
/// Returns [`ObjectRowError`] for a wrong-width row, source error, or a
/// malformed float prefix that cannot be converted.
pub fn decode_object_query_row_legacy(
    row: &ObjectTableQueryRow,
) -> Result<ObjectRecord, ObjectRowError> {
    if row.columns.len() != OBJECT_TABLE_QUERY_COLUMNS {
        return Err(ObjectRowError::ColumnCount {
            expected: OBJECT_TABLE_QUERY_COLUMNS,
            actual: row.columns.len(),
        });
    }
    Ok(ObjectRecord {
        id: legacy_u32(&row.columns[0], 0)?,
        land_id: legacy_u32(&row.columns[1], 1)?,
        vnum: legacy_u32(&row.columns[2], 2)?,
        map_index: legacy_i32(&row.columns[3], 3)?,
        x: legacy_i32(&row.columns[4], 4)?,
        y: legacy_i32(&row.columns[5], 5)?,
        x_rot: legacy_f32(&row.columns[6], 6)?,
        y_rot: legacy_f32(&row.columns[7], 7)?,
        z_rot: legacy_f32(&row.columns[8], 8)?,
        life: legacy_i32(&row.columns[9], 9)?,
    })
}

fn validate_section_size(
    count: usize,
    limits: ObjectSectionLimits,
) -> Result<(u16, u16, usize), ObjectSectionError> {
    if count > limits.max_records {
        return Err(ObjectSectionError::TooManyRecords {
            count,
            maximum: limits.max_records,
        });
    }
    let wire_count =
        u16::try_from(count).map_err(|_| ObjectSectionError::CountOverflow { count })?;
    let record_size = u16::try_from(OBJECT_RECORD_WIRE_SIZE).map_err(|_| {
        ObjectSectionError::RecordSizeOverflow {
            size: OBJECT_RECORD_WIRE_SIZE,
        }
    })?;
    let data_len = OBJECT_RECORD_WIRE_SIZE
        .checked_mul(count)
        .ok_or(ObjectSectionError::DataSizeOverflow { count })?;
    if data_len > limits.max_data_bytes {
        return Err(ObjectSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }
    Ok((wire_count, record_size, data_len))
}

fn validate_source_count(
    count: usize,
    limits: ObjectSectionLimits,
) -> Result<(), ObjectSectionError> {
    if count > limits.max_records {
        return Err(ObjectSectionError::TooManyRecords {
            count,
            maximum: limits.max_records,
        });
    }
    if count > OBJECT_TABLE_MAX_RECORDS {
        return Err(ObjectSectionError::CountOverflow { count });
    }
    let source_len = OBJECT_RECORD_WIRE_SIZE
        .checked_mul(count)
        .ok_or(ObjectSectionError::DataSizeOverflow { count })?;
    if source_len > limits.max_data_bytes {
        return Err(ObjectSectionError::DataTooLarge {
            length: source_len,
            maximum: limits.max_data_bytes,
        });
    }
    Ok(())
}

fn pack_object_records(
    records: &[ObjectRecord],
    limits: ObjectSectionLimits,
) -> Result<BootSection, ObjectSectionError> {
    let (count, record_size, data_len) = validate_section_size(records.len(), limits)?;
    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| ObjectSectionError::AllocationFailed {
            requested: data_len,
        })?;
    for (index, record) in records.iter().enumerate() {
        let encoded = record.encode();
        if encoded.len() != OBJECT_RECORD_WIRE_SIZE {
            return Err(ObjectSectionError::RecordSizeMismatch {
                index,
                expected: OBJECT_RECORD_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }
    debug_assert_eq!(data.len(), data_len);
    Ok(BootSection {
        kind: BootSectionKind::Object,
        record_size,
        count,
        data,
    })
}

/// Build an object section with the default representable limits.
///
/// # Errors
///
/// Returns [`ObjectSectionError`] for invalid rows or configured limit
/// violations.
pub fn build_object_section(
    rows: &[ObjectTableQueryRow],
) -> Result<BootSection, ObjectSectionError> {
    build_object_section_with_limits(rows, ObjectSectionLimits::default())
}

/// Build an object section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`ObjectSectionError`] for invalid rows or configured limits.
pub fn build_object_section_with_limit(
    rows: &[ObjectTableQueryRow],
    max_records: usize,
) -> Result<BootSection, ObjectSectionError> {
    build_object_section_with_limits(rows, ObjectSectionLimits::new(max_records))
}

/// Decode rows and build a typed object boot section.
///
/// Rows are encoded in source order. The section always declares the exact
/// source-fixed 40-byte width, including for an empty result. Limits are
/// checked before the output vector is reserved.
///
/// # Errors
///
/// Returns [`ObjectSectionError`] for invalid rows, count conversion,
/// allocation failures, or configured limit violations.
pub fn build_object_section_with_limits(
    rows: &[ObjectTableQueryRow],
    limits: ObjectSectionLimits,
) -> Result<BootSection, ObjectSectionError> {
    let (count, record_size, data_len) = validate_section_size(rows.len(), limits)?;
    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| ObjectSectionError::AllocationFailed {
            requested: data_len,
        })?;
    for (index, row) in rows.iter().enumerate() {
        let record = decode_object_query_row(row)
            .map_err(|source| ObjectSectionError::Row { index, source })?;
        let encoded = record.encode();
        if encoded.len() != OBJECT_RECORD_WIRE_SIZE {
            return Err(ObjectSectionError::RecordSizeMismatch {
                index,
                expected: OBJECT_RECORD_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }
    debug_assert_eq!(data.len(), data_len);
    Ok(BootSection {
        kind: BootSectionKind::Object,
        record_size,
        count,
        data,
    })
}

/// Decode rows and build a section with the named legacy conversion policy.
///
/// # Errors
///
/// Returns [`ObjectSectionError`] for selected source-row, count, allocation,
/// or configured-limit failures.
pub fn build_object_section_legacy_with_limits(
    rows: &[ObjectTableQueryRow],
    limits: ObjectSectionLimits,
) -> Result<BootSection, ObjectSectionError> {
    let (count, record_size, data_len) = validate_section_size(rows.len(), limits)?;
    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| ObjectSectionError::AllocationFailed {
            requested: data_len,
        })?;
    for (index, row) in rows.iter().enumerate() {
        let record = decode_object_query_row_legacy(row)
            .map_err(|source| ObjectSectionError::Row { index, source })?;
        let encoded = record.encode();
        if encoded.len() != OBJECT_RECORD_WIRE_SIZE {
            return Err(ObjectSectionError::RecordSizeMismatch {
                index,
                expected: OBJECT_RECORD_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }
    debug_assert_eq!(data.len(), data_len);
    Ok(BootSection {
        kind: BootSectionKind::Object,
        record_size,
        count,
        data,
    })
}

/// Build a map-equivalent object section with strict row decoding.
///
/// The source-row cap is checked before rows are decoded or deduplicated.
/// The first row for each ID wins and the selected records are emitted in
/// ascending numeric ID order, matching the legacy `std::map` boundary. This
/// function does not mutate a manager or perform a request.
///
/// # Errors
///
/// Returns [`ObjectSectionError`] for source limits, strict row failures,
/// allocation failures, or output packed-byte limits.
pub fn build_object_map_section_with_limits(
    rows: &[ObjectTableQueryRow],
    limits: ObjectSectionLimits,
) -> Result<BootSection, ObjectSectionError> {
    validate_source_count(rows.len(), limits)?;
    let mut records = Vec::new();
    records
        .try_reserve_exact(rows.len())
        .map_err(|_| ObjectSectionError::AllocationFailed {
            requested: rows.len(),
        })?;
    for (index, row) in rows.iter().enumerate() {
        records.push(
            decode_object_query_row(row)
                .map_err(|source| ObjectSectionError::Row { index, source })?,
        );
    }
    let selected = object_records_first_wins_by_id_bounded(&records)?;
    pack_object_records(&selected, limits)
}

/// Build a map-equivalent object section with the named legacy row policy.
///
/// # Errors
///
/// Returns [`ObjectSectionError`] for source limits, legacy row failures,
/// allocation failures, or output packed-byte limits.
pub fn build_object_map_section_legacy_with_limits(
    rows: &[ObjectTableQueryRow],
    limits: ObjectSectionLimits,
) -> Result<BootSection, ObjectSectionError> {
    validate_source_count(rows.len(), limits)?;
    let mut records = Vec::new();
    records
        .try_reserve_exact(rows.len())
        .map_err(|_| ObjectSectionError::AllocationFailed {
            requested: rows.len(),
        })?;
    for (index, row) in rows.iter().enumerate() {
        records.push(
            decode_object_query_row_legacy(row)
                .map_err(|source| ObjectSectionError::Row { index, source })?,
        );
    }
    let selected = object_records_first_wins_by_id_bounded(&records)?;
    pack_object_records(&selected, limits)
}

/// Build a map-equivalent object section with default limits.
///
/// # Errors
///
/// Returns [`ObjectSectionError`] for strict row, allocation, or limit
/// failures.
pub fn build_object_map_section(
    rows: &[ObjectTableQueryRow],
) -> Result<BootSection, ObjectSectionError> {
    build_object_map_section_with_limits(rows, ObjectSectionLimits::default())
}

/// Alias emphasizing that this is the boot-output map policy, not raw
/// acquisition order.
///
/// # Errors
///
/// Returns the same strict row, allocation, and limit errors as
/// [`build_object_map_section_with_limits`].
pub fn build_object_boot_section_with_limits(
    rows: &[ObjectTableQueryRow],
    limits: ObjectSectionLimits,
) -> Result<BootSection, ObjectSectionError> {
    build_object_map_section_with_limits(rows, limits)
}

/// Alias for the named legacy map-output policy.
///
/// # Errors
///
/// Returns the same legacy row, allocation, and limit errors as
/// [`build_object_map_section_legacy_with_limits`].
pub fn build_object_boot_section_legacy_with_limits(
    rows: &[ObjectTableQueryRow],
    limits: ObjectSectionLimits,
) -> Result<BootSection, ObjectSectionError> {
    build_object_map_section_legacy_with_limits(rows, limits)
}

fn object_records_first_wins_by_id_bounded(
    records: &[ObjectRecord],
) -> Result<Vec<ObjectRecord>, ObjectSectionError> {
    let mut selected = Vec::new();
    selected.try_reserve_exact(records.len()).map_err(|_| {
        ObjectSectionError::AllocationFailed {
            requested: records.len(),
        }
    })?;
    selected.extend_from_slice(records);
    // `sort_by` is stable, so equal IDs retain the first SQL row before the
    // adjacent duplicate is removed. This is equivalent to `std::map::insert`.
    selected.sort_by(|left, right| left.id.cmp(&right.id));
    let mut write_index = 0;
    for read_index in 1..selected.len() {
        if selected[read_index].id == selected[write_index].id {
            continue;
        }
        write_index += 1;
        selected[write_index] = selected[read_index];
    }
    selected.truncate(if selected.is_empty() {
        0
    } else {
        write_index + 1
    });
    Ok(selected)
}

/// Reproduce the legacy object map's first-wins, ascending-key selection.
///
/// This is a pure conversion seam only. It does not allocate the legacy
/// `TObject`, replace a manager, or dispatch a request. The returned records
/// are ordered by `id`, as a `std::map` would be.
#[must_use]
pub fn object_records_first_wins_by_id(records: &[ObjectRecord]) -> Vec<ObjectRecord> {
    let mut map = BTreeMap::new();
    for record in records {
        map.entry(record.id).or_insert(*record);
    }
    map.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postfix::TablePostfix;

    fn value(text: &str) -> ObjectQueryValue {
        ObjectQueryValue::text(text)
    }

    fn row() -> ObjectTableQueryRow {
        ObjectTableQueryRow::from_typed_columns([
            value("7"),
            value("8"),
            value("9"),
            value("-1"),
            value("10"),
            value("11"),
            value("1.5"),
            value("-2.25"),
            value("3.0"),
            value("-42"),
        ])
    }

    #[test]
    fn query_is_exact_and_postfix_is_validated() {
        let postfix = TablePostfix::parse("_eu").unwrap();
        let query = ObjectTableQuery::new(&postfix).unwrap();
        assert_eq!(
            query.as_str(),
            "SELECT id, land_id, vnum, map_index, x, y, x_rot, y_rot, z_rot, life FROM object_eu ORDER BY id"
        );
        assert_eq!(query.table_name(), "object_eu");
        assert!(ObjectTableQuery::from_config(Some("bad-name")).is_err());
    }

    #[test]
    fn strict_rows_decode_and_build_in_source_order() {
        let section = build_object_section(&[row()]).unwrap();
        assert_eq!(section.kind, BootSectionKind::Object);
        assert_eq!(section.record_size, 40);
        assert_eq!(section.count, 1);
        let record = ObjectRecord::decode(&section.data).unwrap();
        assert_eq!(record.id, 7);
        assert_eq!(record.land_id, 8);
        assert_eq!(record.vnum, 9);
        assert_eq!(record.x_rot.to_bits(), 1.5_f32.to_bits());
        assert_eq!(record.life, -42);
    }

    #[test]
    fn empty_rows_produce_an_exact_empty_object_section() {
        let section = build_object_section(&[]).unwrap();
        assert_eq!(section.kind, BootSectionKind::Object);
        assert_eq!(section.record_size, 40);
        assert_eq!(section.count, 0);
        assert!(section.data.is_empty());
    }

    #[test]
    fn null_and_source_errors_are_not_zeroed_by_strict_decoder() {
        let mut columns = row().into_columns();
        columns[0] = ObjectQueryValue::null();
        assert!(matches!(
            decode_object_query_row(&ObjectTableQueryRow::new(columns)),
            Err(ObjectRowError::Null { column: 0 })
        ));
        let mut columns = row().into_columns();
        columns[1] = ObjectQueryValue::error("driver failure");
        assert!(matches!(
            decode_object_query_row(&ObjectTableQueryRow::new(columns)),
            Err(ObjectRowError::Source { column: 1, .. })
        ));
    }

    #[test]
    fn strict_decoder_rejects_malformed_and_overflow_values() {
        for text in ["", " 1", "1x", "+1"] {
            let mut columns = row().into_columns();
            columns[0] = value(text);
            assert!(matches!(
                decode_object_query_row(&ObjectTableQueryRow::new(columns)),
                Err(ObjectRowError::InvalidNumber { column: 0, .. })
            ));
        }
        let mut columns = row().into_columns();
        columns[0] = value("4294967296");
        assert!(matches!(
            decode_object_query_row(&ObjectTableQueryRow::new(columns)),
            Err(ObjectRowError::NumberOverflow { column: 0, .. })
        ));
        let mut columns = row().into_columns();
        columns[6] = value("1e100");
        assert!(matches!(
            decode_object_query_row(&ObjectTableQueryRow::new(columns)),
            Err(ObjectRowError::NumberOverflow { column: 6, .. })
        ));
    }

    #[test]
    fn legacy_decoder_keeps_source_map_compatibility_separate() {
        let columns = [
            value("12x"),
            value(""),
            ObjectQueryValue::null(),
            value("-7foo"),
            value("+8"),
            value("9"),
            value("1.5tail"),
            value("2"),
            value("3"),
            value("4"),
        ];
        let record =
            decode_object_query_row_legacy(&ObjectTableQueryRow::from_typed_columns(columns))
                .unwrap();
        assert_eq!(record.id, 12);
        assert_eq!(record.land_id, 0);
        assert_eq!(record.vnum, 0);
        assert_eq!(record.map_index, -7);
        assert_eq!(record.x, 8);
        assert_eq!(record.x_rot.to_bits(), 1.5_f32.to_bits());
    }

    #[test]
    fn first_wins_selection_is_sorted_and_deduplicated() {
        let a = ObjectRecord {
            id: 2,
            land_id: 1,
            ..ObjectRecord::default()
        };
        let b = ObjectRecord {
            id: 1,
            land_id: 3,
            ..ObjectRecord::default()
        };
        let c = ObjectRecord {
            id: 2,
            land_id: 9,
            ..ObjectRecord::default()
        };
        let selected = object_records_first_wins_by_id(&[a, b, c]);
        assert_eq!(selected, vec![b, a]);
    }

    #[test]
    fn map_section_packs_selected_rows_in_id_order() {
        let mut first = row().into_columns();
        first[0] = value("2");
        first[1] = value("20");
        let mut duplicate = row().into_columns();
        duplicate[0] = value("2");
        duplicate[1] = value("99");
        let mut second = row().into_columns();
        second[0] = value("1");
        second[1] = value("10");
        let rows = [
            ObjectTableQueryRow::new(first),
            ObjectTableQueryRow::new(duplicate),
            ObjectTableQueryRow::new(second),
        ];
        let section = build_object_map_section(&rows).unwrap();
        assert_eq!(section.count, 2);
        let first_record = ObjectRecord::decode(&section.data[..40]).unwrap();
        let second_record = ObjectRecord::decode(&section.data[40..]).unwrap();
        assert_eq!((first_record.id, first_record.land_id), (1, 10));
        assert_eq!((second_record.id, second_record.land_id), (2, 20));
    }

    #[test]
    fn map_section_checks_source_cap_before_decoding() {
        let rows = vec![row(), row()];
        assert!(matches!(
            build_object_map_section_with_limits(&rows, ObjectSectionLimits::new(1)),
            Err(ObjectSectionError::TooManyRecords {
                count: 2,
                maximum: 1
            })
        ));
    }

    #[test]
    fn legacy_parser_handles_c_prefixes_saturation_and_specials() {
        let float = |input: &str| {
            decode_object_query_row_legacy(&ObjectTableQueryRow::from_typed_columns([
                value("1"),
                value("1"),
                value("1"),
                value("0"),
                value("0"),
                value("0"),
                value(input),
                value("0"),
                value("0"),
                value("0"),
            ]))
            .unwrap()
            .x_rot
        };
        assert_eq!(float("  12tail").to_bits(), 12.0_f32.to_bits());
        assert_eq!(float("  -2.5suffix").to_bits(), (-2.5_f32).to_bits());
        assert!(float("+inf").is_infinite() && float("+inf").is_sign_positive());
        assert!(float("-inf").is_infinite() && float("-inf").is_sign_negative());
        assert!(float("+nan").is_nan());
        assert!(float("-nan").is_nan());
        assert_eq!(float("0x10").to_bits(), 16.0_f32.to_bits());
        assert_eq!(float("0x1p2").to_bits(), 4.0_f32.to_bits());
        assert_eq!(float("0x1.8p1tail").to_bits(), 3.0_f32.to_bits());
        assert_eq!(float("0x1p").to_bits(), 1.0_f32.to_bits());
        let mut columns = row().into_columns();
        columns[0] = value("-4294967296");
        let record = decode_object_query_row_legacy(&ObjectTableQueryRow::new(columns)).unwrap();
        assert_eq!(record.id, u32::MAX);
        let mut columns = row().into_columns();
        columns[3] = value("2147483648");
        let record = decode_object_query_row_legacy(&ObjectTableQueryRow::new(columns)).unwrap();
        assert_eq!(record.map_index, i32::MAX);
        let mut columns = row().into_columns();
        columns[0] = value("€");
        let record = decode_object_query_row_legacy(&ObjectTableQueryRow::new(columns)).unwrap();
        assert_eq!(record.id, 0);
    }

    #[test]
    fn limits_are_checked_before_allocation() {
        let rows = vec![row()];
        assert!(matches!(
            build_object_section_with_limit(&rows, 0),
            Err(ObjectSectionError::TooManyRecords { .. })
        ));
        assert!(matches!(
            build_object_section_with_limits(&rows, ObjectSectionLimits::with_data_limit(1, 39)),
            Err(ObjectSectionError::DataTooLarge {
                length: 40,
                maximum: 39
            })
        ));
    }
}
