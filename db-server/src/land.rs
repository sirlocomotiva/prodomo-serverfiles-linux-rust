//! Pure, SQL-free conversion of the active legacy `land` query to a boot section.
//!
//! The legacy loader selects nine columns in this exact order:
//! `id`, `map_index`, `x`, `y`, `width`, `height`, `guild_id`,
//! `guild_level_limit`, and `price`.  The source orders rows with `ORDER BY id`;
//! this module preserves the order supplied by its row source and does not
//! sort, deduplicate, or execute SQL.
//!
//! The default decoder is intentionally strict.  [`decode_land_query_row_legacy`]
//! is a separately named compatibility boundary for the legacy C++
//! `str_to_number` calls, which accept numeric prefixes and cast to the
//! destination type.  Neither decoder executes the optional maintenance SQL
//! guarded by `ENABLE_CLEAR_OLD_GUILDS_LANDS_BY_INACTIVITY`.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootSection, BootSectionKind};
use protocol::db_records::{LandRecord, LAND_RECORD_WIRE_SIZE};

/// Number of columns selected by the fixed legacy land query.
pub const LAND_TABLE_QUERY_COLUMNS: usize = 9;

/// Alias for [`LAND_TABLE_QUERY_COLUMNS`].
pub const LAND_QUERY_COLUMN_COUNT: usize = LAND_TABLE_QUERY_COLUMNS;

/// Maximum number of records representable by the legacy `u16` section count.
pub const LAND_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Exact fixed 36-byte x86 `building::TLand` wire width represented by a section record.
pub const LAND_SECTION_RECORD_SIZE: u16 = 36;

/// Alias for the source-fixed record width.
pub const LAND_TABLE_WIRE_SIZE: usize = LAND_RECORD_WIRE_SIZE;

/// Maximum byte length of a section when every representable count is used.
pub const LAND_TABLE_MAX_SECTION_BYTES: usize = LAND_RECORD_WIRE_SIZE * LAND_TABLE_MAX_RECORDS;

/// Column names in the fixed legacy query order.
pub const LAND_TABLE_QUERY_COLUMN_NAMES: [&str; LAND_TABLE_QUERY_COLUMNS] = [
    "id",
    "map_index",
    "x",
    "y",
    "width",
    "height",
    "guild_id",
    "guild_level_limit",
    "price",
];

/// One value returned by a query-row adapter.
///
/// `Text` is the only state accepted by the strict numeric decoder. `Null`
/// and `Error` retain the distinct facts that a database column can produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandQueryValue {
    /// A non-NULL textual value, normally the original SQL result string.
    Text(String),
    /// A SQL `NULL` value.
    Null,
    /// An error raised while obtaining or converting one query column.
    Error(String),
}

impl LandQueryValue {
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

impl From<String> for LandQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for LandQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

/// A row-shaped value for the nine-column land query.
///
/// The vector is retained so a malformed source can report its actual width.
/// [`Self::new`] retains any width; [`Self::try_new`] validates the shape at
/// construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandTableQueryRow {
    columns: Vec<LandQueryValue>,
}

impl LandTableQueryRow {
    /// Construct a row while retaining supplied values and their order.
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = LandQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub fn from_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = LandQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct and validate a nine-column row.
    ///
    /// # Errors
    ///
    /// Returns [`LandRowError::ColumnCount`] when the iterator does not
    /// produce exactly nine values.
    pub fn try_new<I>(columns: I) -> Result<Self, LandRowError>
    where
        I: IntoIterator<Item = LandQueryValue>,
    {
        let row = Self::new(columns);
        if row.columns.len() != LAND_TABLE_QUERY_COLUMNS {
            return Err(LandRowError::ColumnCount {
                expected: LAND_TABLE_QUERY_COLUMNS,
                actual: row.columns.len(),
            });
        }
        Ok(row)
    }

    /// Construct a row from a statically sized, correctly shaped column set.
    #[must_use]
    pub fn from_typed_columns(columns: [LandQueryValue; LAND_TABLE_QUERY_COLUMNS]) -> Self {
        Self::new(columns)
    }

    /// Borrow all supplied columns in query order.
    #[must_use]
    pub fn columns(&self) -> &[LandQueryValue] {
        &self.columns
    }

    /// Consume the row and return its columns in query order.
    #[must_use]
    pub fn into_columns(self) -> Vec<LandQueryValue> {
        self.columns
    }

    /// Return the number of columns supplied by the source.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

impl From<[LandQueryValue; LAND_TABLE_QUERY_COLUMNS]> for LandTableQueryRow {
    fn from(columns: [LandQueryValue; LAND_TABLE_QUERY_COLUMNS]) -> Self {
        Self::from_typed_columns(columns)
    }
}

impl TryFrom<Vec<LandQueryValue>> for LandTableQueryRow {
    type Error = LandRowError;

    fn try_from(columns: Vec<LandQueryValue>) -> Result<Self, Self::Error> {
        Self::try_new(columns)
    }
}

/// Short aliases used by query-builder integrations.
pub type LandQueryRow = LandTableQueryRow;
/// Alias for a single land query-cell value.
pub type LandQueryColumn = LandQueryValue;

/// Return a stable column name for diagnostics.
#[must_use]
pub const fn land_query_column_name(index: usize) -> &'static str {
    match index {
        0 => "id",
        1 => "map_index",
        2 => "x",
        3 => "y",
        4 => "width",
        5 => "height",
        6 => "guild_id",
        7 => "guild_level_limit",
        8 => "price",
        _ => "unknown",
    }
}

/// A strict or explicitly selected compatibility error in one land row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandRowError {
    /// The row did not contain exactly nine columns.
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
    /// A non-NULL value was not a strict decimal integer.
    InvalidNumber {
        /// Zero-based query column index.
        column: usize,
        /// Original value.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A syntactically valid integer did not fit its target type.
    NumberOverflow {
        /// Zero-based query column index.
        column: usize,
        /// Original value.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
}

impl fmt::Display for LandRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => write!(
                formatter,
                "land query row has {actual} columns; expected {expected}"
            ),
            Self::Null { column } => write!(
                formatter,
                "land query column {} is NULL",
                land_query_column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "land query column {} could not be read: {message}",
                land_query_column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "land query column {} value {value:?} is not a strict {target}",
                land_query_column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "land query column {} value {value:?} overflows {target}",
                land_query_column_name(*column)
            ),
        }
    }
}

impl Error for LandRowError {}

/// Limits applied before allocating a land section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LandSectionLimits {
    /// Maximum number of source rows accepted.
    pub max_records: usize,
    /// Maximum number of packed record-data bytes accepted.
    pub max_data_bytes: usize,
}

impl LandSectionLimits {
    /// Construct limits that bound records but leave the byte limit at
    /// `usize::MAX`; the representable `u16` count still applies.
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

impl Default for LandSectionLimits {
    fn default() -> Self {
        Self {
            max_records: LAND_TABLE_MAX_RECORDS,
            max_data_bytes: LAND_TABLE_MAX_SECTION_BYTES,
        }
    }
}

/// A checked section-construction error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandSectionError {
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
    /// The output vector could not reserve its bounded length.
    AllocationFailed {
        /// Requested output byte length.
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
        source: LandRowError,
    },
}

impl fmt::Display for LandSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "land section has {count} rows; configured limit is {maximum}"
            ),
            Self::CountOverflow { count } => {
                write!(formatter, "land section count {count} does not fit u16")
            }
            Self::RecordSizeOverflow { size } => {
                write!(formatter, "land record width {size} does not fit u16")
            }
            Self::DataSizeOverflow { count } => write!(
                formatter,
                "land section byte size overflows usize for {count} rows"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "land section could not allocate {requested} output bytes"
            ),
            Self::DataTooLarge { length, maximum } => write!(
                formatter,
                "land section data length {length} exceeds limit {maximum}"
            ),
            Self::RecordSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "land row {index} encoded to {actual} bytes; expected {expected}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "land row {index} is invalid: {source}")
            }
        }
    }
}

impl Error for LandSectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Alias emphasizing that this type is a section-build error.
pub type LandSectionBuildError = LandSectionError;

/// An injected, fallible source of land query rows.
pub trait LandTableRowSource {
    /// Error returned when the source cannot produce rows.
    type Error: fmt::Display;

    /// Load rows in the order they should appear in the boot section.
    ///
    /// # Errors
    ///
    /// Returns the source-specific error when the row set cannot be loaded.
    fn load_rows(&self) -> Result<Vec<LandTableQueryRow>, Self::Error>;
}

impl<F, E> LandTableRowSource for F
where
    F: Fn() -> Result<Vec<LandTableQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn load_rows(&self) -> Result<Vec<LandTableQueryRow>, Self::Error> {
        self()
    }
}

/// Short alias for [`LandTableRowSource`].
pub use LandTableRowSource as LandRowSource;

/// An error while obtaining rows or building their section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandLoadError<E> {
    /// The injected row source failed.
    Source(E),
    /// Rows were obtained but could not be decoded or bounded.
    Section(LandSectionError),
}

impl<E: fmt::Display> fmt::Display for LandLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "land row source failed: {source}"),
            Self::Section(source) => {
                write!(formatter, "land section construction failed: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for LandLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Section(source) => Some(source),
        }
    }
}

/// A reusable pure builder with explicit limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LandSectionBuilder {
    limits: LandSectionLimits,
}

impl LandSectionBuilder {
    /// Construct a builder with a caller-selected row cap.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            limits: LandSectionLimits::new(max_records),
        }
    }

    /// Construct a builder with both limits.
    #[must_use]
    pub const fn with_limits(limits: LandSectionLimits) -> Self {
        Self { limits }
    }

    /// Return configured limits.
    #[must_use]
    pub const fn limits(self) -> LandSectionLimits {
        self.limits
    }

    /// Decode and build a strict section, preserving source row order.
    ///
    /// # Errors
    ///
    /// Returns [`LandSectionError`] for a row/column/numeric error or when
    /// configured limits are exceeded.
    pub fn build(&self, rows: &[LandTableQueryRow]) -> Result<BootSection, LandSectionError> {
        build_land_section_with_limits(rows, self.limits)
    }

    /// Decode and build using the explicitly named legacy conversion policy.
    ///
    /// # Errors
    ///
    /// Returns [`LandSectionError`] for a source-column error or configured
    /// limit violation. Numeric text is intentionally permissive in this
    /// method because it models the legacy `str_to_number` calls.
    pub fn build_legacy(
        &self,
        rows: &[LandTableQueryRow],
    ) -> Result<BootSection, LandSectionError> {
        build_land_section_legacy_with_limits(rows, self.limits)
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

fn cell_text(value: &LandQueryValue, column: usize) -> Result<&str, LandRowError> {
    match value {
        LandQueryValue::Text(text) => Ok(text),
        LandQueryValue::Null => Err(LandRowError::Null { column }),
        LandQueryValue::Error(message) => Err(LandRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn decode_strict_u32(value: &LandQueryValue, column: usize) -> Result<u32, LandRowError> {
    let text = cell_text(value, column)?;
    if !is_strict_unsigned_decimal(text) {
        return Err(LandRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "u32",
        });
    }
    text.parse::<u32>()
        .map_err(|_| LandRowError::NumberOverflow {
            column,
            value: text.to_owned(),
            target: "u32",
        })
}

fn decode_strict_i32(value: &LandQueryValue, column: usize) -> Result<i32, LandRowError> {
    let text = cell_text(value, column)?;
    if !is_strict_signed_decimal(text) {
        return Err(LandRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "i32",
        });
    }
    text.parse::<i32>()
        .map_err(|_| LandRowError::NumberOverflow {
            column,
            value: text.to_owned(),
            target: "i32",
        })
}

fn decode_strict_u8(value: &LandQueryValue, column: usize) -> Result<u8, LandRowError> {
    let text = cell_text(value, column)?;
    if !is_strict_unsigned_decimal(text) {
        return Err(LandRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "u8",
        });
    }
    text.parse::<u8>()
        .map_err(|_| LandRowError::NumberOverflow {
            column,
            value: text.to_owned(),
            target: "u8",
        })
}

fn legacy_error(value: &LandQueryValue, column: usize) -> Option<LandRowError> {
    match value {
        LandQueryValue::Error(message) => Some(LandRowError::Source {
            column,
            message: message.clone(),
        }),
        LandQueryValue::Text(_) | LandQueryValue::Null => None,
    }
}

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Parse the C `strtol`/`strtoul`-style numeric prefix.  The returned sign
/// and magnitude are deliberately not Rust-normalized, so destination casts
/// can model the legacy calls without accepting arbitrary text as strict data.
fn legacy_numeric_prefix(text: &str) -> (bool, Option<u64>) {
    let bytes = text.as_bytes();
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
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        let digit = u64::from(bytes[index] - b'0');
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(digit))
            .unwrap_or(u64::MAX);
        index += 1;
    }
    if index == start {
        (negative, None)
    } else {
        (negative, Some(magnitude))
    }
}

fn legacy_i32(value: &LandQueryValue, column: usize) -> Result<i32, LandRowError> {
    if let Some(error) = legacy_error(value, column) {
        return Err(error);
    }
    let text = match value {
        LandQueryValue::Text(text) => text.as_str(),
        // The legacy helper checks for a NULL pointer or an empty string
        // before calling strtol and leaves the zero-initialized field alone.
        LandQueryValue::Null => return Ok(0),
        LandQueryValue::Error(_) => unreachable!(),
    };
    let (negative, magnitude) = legacy_numeric_prefix(text);
    let Some(magnitude) = magnitude else {
        return Ok(0);
    };
    if negative {
        if magnitude >= 2_147_483_648 {
            Ok(i32::MIN)
        } else {
            match i32::try_from(magnitude) {
                Ok(value) => Ok(-value),
                Err(_) => Ok(i32::MIN),
            }
        }
    } else if magnitude > 2_147_483_647_u64 {
        Ok(i32::MAX)
    } else {
        match i32::try_from(magnitude) {
            Ok(value) => Ok(value),
            Err(_) => Ok(i32::MAX),
        }
    }
}

fn legacy_u32(value: &LandQueryValue, column: usize) -> Result<u32, LandRowError> {
    if let Some(error) = legacy_error(value, column) {
        return Err(error);
    }
    let text = match value {
        LandQueryValue::Text(text) => text.as_str(),
        LandQueryValue::Null => return Ok(0),
        LandQueryValue::Error(_) => unreachable!(),
    };
    let (negative, magnitude) = legacy_numeric_prefix(text);
    let Some(magnitude) = magnitude else {
        return Ok(0);
    };
    if magnitude > u64::from(u32::MAX) {
        // A 32-bit strtoul returns ULONG_MAX on magnitude overflow,
        // including when the input has a leading minus sign.
        return Ok(u32::MAX);
    }
    let reduced = match u32::try_from(magnitude) {
        Ok(value) => value,
        Err(_) => u32::MAX,
    };
    if negative {
        Ok(0_u32.wrapping_sub(reduced))
    } else {
        Ok(reduced)
    }
}

fn legacy_u8(value: &LandQueryValue, column: usize) -> Result<u8, LandRowError> {
    let value = legacy_u32(value, column)?;
    match u8::try_from(value & 0xff) {
        Ok(value) => Ok(value),
        Err(_) => Ok(0),
    }
}

fn check_land_row_width(row: &LandTableQueryRow) -> Result<(), LandRowError> {
    if row.columns.len() == LAND_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(LandRowError::ColumnCount {
            expected: LAND_TABLE_QUERY_COLUMNS,
            actual: row.columns.len(),
        })
    }
}

/// Strictly decode one land query row.
///
/// `NULL`, source errors, non-decimal text, and target-width overflow remain
/// errors.  Leading `+`, whitespace, numeric suffixes, and empty strings are
/// intentionally not accepted here.
///
/// # Errors
///
/// Returns [`LandRowError`] for a wrong-width row or any invalid cell.
pub fn decode_land_query_row(row: &LandTableQueryRow) -> Result<LandRecord, LandRowError> {
    check_land_row_width(row)?;
    Ok(LandRecord {
        id: decode_strict_u32(&row.columns[0], 0)?,
        map_index: decode_strict_i32(&row.columns[1], 1)?,
        x: decode_strict_i32(&row.columns[2], 2)?,
        y: decode_strict_i32(&row.columns[3], 3)?,
        width: decode_strict_i32(&row.columns[4], 4)?,
        height: decode_strict_i32(&row.columns[5], 5)?,
        guild_id: decode_strict_u32(&row.columns[6], 6)?,
        guild_level_limit: decode_strict_u8(&row.columns[7], 7)?,
        price: decode_strict_u32(&row.columns[8], 8)?,
    })
}

/// Decode one row using the explicitly named legacy `str_to_number` policy.
///
/// The legacy calls use `strtol` for signed x86 `long` fields and
/// `strtoul` for `DWORD`/`unsigned char` fields, ignore trailing text, accept
/// leading whitespace and a sign, and cast the result to the destination type.
/// The legacy helper returns without changing its zero-initialized field for
/// a NULL pointer or an empty string; this function preserves that result.
/// Numeric overflow follows the 32-bit C conversion limits before the
/// destination cast.
///
/// # Errors
///
/// Returns [`LandRowError::ColumnCount`] for a wrong-width row and
/// [`LandRowError::Source`] for a cell-extraction error. Numeric text itself
/// does not produce a syntax error in this compatibility policy.
pub fn decode_land_query_row_legacy(row: &LandTableQueryRow) -> Result<LandRecord, LandRowError> {
    check_land_row_width(row)?;
    Ok(LandRecord {
        id: legacy_u32(&row.columns[0], 0)?,
        map_index: legacy_i32(&row.columns[1], 1)?,
        x: legacy_i32(&row.columns[2], 2)?,
        y: legacy_i32(&row.columns[3], 3)?,
        width: legacy_i32(&row.columns[4], 4)?,
        height: legacy_i32(&row.columns[5], 5)?,
        guild_id: legacy_u32(&row.columns[6], 6)?,
        guild_level_limit: legacy_u8(&row.columns[7], 7)?,
        price: legacy_u32(&row.columns[8], 8)?,
    })
}

/// Alias emphasizing that the compatibility policy is opt-in.
///
/// # Errors
///
/// Returns [`LandRowError`] for a wrong-width row or source-cell error.
pub fn decode_land_query_row_compat(row: &LandTableQueryRow) -> Result<LandRecord, LandRowError> {
    decode_land_query_row_legacy(row)
}

fn validate_section_size(
    count: usize,
    limits: LandSectionLimits,
) -> Result<(u16, u16, usize), LandSectionError> {
    if count > limits.max_records {
        return Err(LandSectionError::TooManyRecords {
            count,
            maximum: limits.max_records,
        });
    }
    let wire_count = u16::try_from(count).map_err(|_| LandSectionError::CountOverflow { count })?;
    let record_size =
        u16::try_from(LAND_RECORD_WIRE_SIZE).map_err(|_| LandSectionError::RecordSizeOverflow {
            size: LAND_RECORD_WIRE_SIZE,
        })?;
    let data_len = LAND_RECORD_WIRE_SIZE
        .checked_mul(count)
        .ok_or(LandSectionError::DataSizeOverflow { count })?;
    if data_len > limits.max_data_bytes {
        return Err(LandSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }
    Ok((wire_count, record_size, data_len))
}

fn append_records(
    rows: &[LandTableQueryRow],
    limits: LandSectionLimits,
    legacy: bool,
) -> Result<BootSection, LandSectionError> {
    let (count, record_size, data_len) = validate_section_size(rows.len(), limits)?;
    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| LandSectionError::AllocationFailed {
            requested: data_len,
        })?;

    for (index, row) in rows.iter().enumerate() {
        let record = if legacy {
            decode_land_query_row_legacy(row)
        } else {
            decode_land_query_row(row)
        }
        .map_err(|source| LandSectionError::Row { index, source })?;
        let encoded = record.encode();
        if encoded.len() != LAND_RECORD_WIRE_SIZE {
            return Err(LandSectionError::RecordSizeMismatch {
                index,
                expected: LAND_RECORD_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }

    debug_assert_eq!(data.len(), data_len);
    Ok(BootSection {
        kind: BootSectionKind::Land,
        record_size,
        count,
        data,
    })
}

/// Build a strict land section with default limits.
///
/// # Errors
///
/// Returns [`LandSectionError`] for an invalid row or configured limit
/// violation.
pub fn build_land_section(rows: &[LandTableQueryRow]) -> Result<BootSection, LandSectionError> {
    build_land_section_with_limits(rows, LandSectionLimits::default())
}

/// Build a strict land section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`LandSectionError`] for an invalid row or configured limit
/// violation.
pub fn build_land_section_with_limit(
    rows: &[LandTableQueryRow],
    max_records: usize,
) -> Result<BootSection, LandSectionError> {
    build_land_section_with_limits(rows, LandSectionLimits::new(max_records))
}

/// Decode rows and build a strict typed land boot section.
///
/// Rows are encoded in exactly the order supplied by the source. The section
/// always declares the source-fixed 36-byte width, including for an empty
/// result. Limits are checked before output storage is reserved.
///
/// # Errors
///
/// Returns [`LandSectionError`] for invalid rows, count conversion or
/// allocation failures, and configured limit violations.
pub fn build_land_section_with_limits(
    rows: &[LandTableQueryRow],
    limits: LandSectionLimits,
) -> Result<BootSection, LandSectionError> {
    append_records(rows, limits, false)
}

/// Build a land section with the explicitly named legacy conversion policy.
///
/// # Errors
///
/// Returns [`LandSectionError`] for source-cell errors, allocation failures,
/// or configured limit violations. Numeric prefixes and destination casts are
/// intentionally accepted here; use [`build_land_section_with_limits`] for
/// the strict default.
pub fn build_land_section_legacy(
    rows: &[LandTableQueryRow],
) -> Result<BootSection, LandSectionError> {
    build_land_section_legacy_with_limits(rows, LandSectionLimits::default())
}

/// Build a legacy-policy land section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`LandSectionError`] for source-cell errors or configured limit
/// violations.
pub fn build_land_section_legacy_with_limit(
    rows: &[LandTableQueryRow],
    max_records: usize,
) -> Result<BootSection, LandSectionError> {
    build_land_section_legacy_with_limits(rows, LandSectionLimits::new(max_records))
}

/// Decode rows and build a land section with the legacy conversion policy.
///
/// # Errors
///
/// Returns [`LandSectionError`] for source-cell errors, allocation failures,
/// or configured limit violations.
pub fn build_land_section_legacy_with_limits(
    rows: &[LandTableQueryRow],
    limits: LandSectionLimits,
) -> Result<BootSection, LandSectionError> {
    append_records(rows, limits, true)
}

/// Load rows from an injected source and build a strict land section.
///
/// # Errors
///
/// Returns [`LandLoadError::Source`] for a source failure and
/// [`LandLoadError::Section`] for row or limit failures. A source failure is
/// never represented as an empty table.
pub fn load_land_section<S>(
    source: &S,
    limits: LandSectionLimits,
) -> Result<BootSection, LandLoadError<S::Error>>
where
    S: LandTableRowSource,
{
    let rows = source.load_rows().map_err(LandLoadError::Source)?;
    build_land_section_with_limits(&rows, limits).map_err(LandLoadError::Section)
}

/// Load rows and build a strict section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`LandLoadError::Source`] for a source failure and
/// [`LandLoadError::Section`] for row or limit failures.
pub fn load_land_section_with_limit<S>(
    source: &S,
    max_records: usize,
) -> Result<BootSection, LandLoadError<S::Error>>
where
    S: LandTableRowSource,
{
    load_land_section(source, LandSectionLimits::new(max_records))
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_records::LandRecord;

    fn text(value: &str) -> LandQueryValue {
        LandQueryValue::text(value)
    }

    fn row(values: [&str; LAND_TABLE_QUERY_COLUMNS]) -> LandTableQueryRow {
        LandTableQueryRow::new(values.into_iter().map(text))
    }

    #[test]
    fn strict_rows_preserve_values_and_emit_zero_padding() {
        let source = row(["42", "-7", "11", "12", "13", "14", "99", "5", "1000"]);
        let record = decode_land_query_row(&source).unwrap();
        assert_eq!(
            record,
            LandRecord {
                id: 42,
                map_index: -7,
                x: 11,
                y: 12,
                width: 13,
                height: 14,
                guild_id: 99,
                guild_level_limit: 5,
                price: 1000,
            }
        );
        let section = build_land_section(&[source.clone(), source]).unwrap();
        assert_eq!(section.kind, BootSectionKind::Land);
        assert_eq!(section.record_size, LAND_SECTION_RECORD_SIZE);
        assert_eq!(section.count, 2);
        assert_eq!(section.data.len(), 72);
        assert_eq!(&section.data[29..32], &[0, 0, 0]);
        assert_eq!(&section.data[65..68], &[0, 0, 0]);
    }

    #[test]
    fn empty_result_is_a_valid_zero_count_land_section() {
        let section = build_land_section(&[]).unwrap();
        assert_eq!(section.kind, BootSectionKind::Land);
        assert_eq!(section.record_size, 36);
        assert_eq!(section.count, 0);
        assert!(section.data.is_empty());
    }

    #[test]
    fn strict_policy_rejects_null_malformed_and_overflow_values() {
        let mut values = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
        let null_row = LandTableQueryRow::new([
            LandQueryValue::Null,
            text("2"),
            text("3"),
            text("4"),
            text("5"),
            text("6"),
            text("7"),
            text("8"),
            text("9"),
        ]);
        assert!(matches!(
            decode_land_query_row(&null_row),
            Err(LandRowError::Null { column: 0 })
        ));

        values[0] = "1x";
        assert!(matches!(
            decode_land_query_row(&row(values)),
            Err(LandRowError::InvalidNumber { column: 0, .. })
        ));
        values[0] = "4294967296";
        assert!(matches!(
            decode_land_query_row(&row(values)),
            Err(LandRowError::NumberOverflow {
                column: 0,
                target: "u32",
                ..
            })
        ));
        values[0] = "1";
        values[7] = "256";
        assert!(matches!(
            decode_land_query_row(&row(values)),
            Err(LandRowError::NumberOverflow {
                column: 7,
                target: "u8",
                ..
            })
        ));
    }

    #[test]
    fn legacy_policy_exposes_prefix_wrapping_and_null_conversion() {
        let source = LandTableQueryRow::new([
            text("12abc"),
            text(" -7x"),
            text("+4"),
            text(""),
            text("999999999999999999999"),
            text("  -1"),
            text("-1"),
            text("256"),
            LandQueryValue::Null,
        ]);
        let record = decode_land_query_row_legacy(&source).unwrap();
        assert_eq!(record.id, 12);
        assert_eq!(record.map_index, -7);
        assert_eq!(record.x, 4);
        assert_eq!(record.y, 0);
        assert_eq!(record.width, i32::MAX);
        assert_eq!(record.height, -1);
        assert_eq!(record.guild_id, u32::MAX);
        assert_eq!(record.guild_level_limit, 0);
        assert_eq!(record.price, 0);
    }

    #[test]
    fn legacy_policy_covers_x86_integer_boundaries_and_casts() {
        let signed_cases = [
            ("2147483647", i32::MAX),
            ("2147483648", i32::MAX),
            ("-2147483648", i32::MIN),
            ("-2147483649", i32::MIN),
            ("", 0),
            ("   +12suffix", 12),
            ("no-digits", 0),
        ];
        for (text, expected) in signed_cases {
            let row = LandTableQueryRow::new([
                LandQueryValue::text("0"),
                LandQueryValue::text(text),
                LandQueryValue::text("0"),
                LandQueryValue::text("0"),
                LandQueryValue::text("0"),
                LandQueryValue::text("0"),
                LandQueryValue::text("0"),
                LandQueryValue::text("0"),
                LandQueryValue::text("0"),
            ]);
            assert_eq!(
                decode_land_query_row_legacy(&row).unwrap().map_index,
                expected,
                "signed compatibility value {text:?}"
            );
        }

        let unsigned_cases = [
            ("4294967295", u32::MAX),
            ("4294967296", u32::MAX),
            ("-1", u32::MAX),
            ("-4294967296", u32::MAX),
            ("-4294967295", 1),
            ("-4294967298", u32::MAX),
            ("256", 256),
        ];
        for (text, expected) in unsigned_cases {
            let row = LandTableQueryRow::new([
                LandQueryValue::text(text),
                LandQueryValue::text("0"),
                LandQueryValue::text("0"),
                LandQueryValue::text("0"),
                LandQueryValue::text("0"),
                LandQueryValue::text("0"),
                LandQueryValue::text(text),
                LandQueryValue::text("0"),
                LandQueryValue::text(text),
            ]);
            let record = decode_land_query_row_legacy(&row).unwrap();
            assert_eq!(record.id, expected, "id compatibility value {text:?}");
            assert_eq!(
                record.guild_id, expected,
                "guild compatibility value {text:?}"
            );
            assert_eq!(record.price, expected, "price compatibility value {text:?}");
        }

        let row = LandTableQueryRow::new([
            LandQueryValue::null(),
            LandQueryValue::null(),
            LandQueryValue::text("0"),
            LandQueryValue::text("0"),
            LandQueryValue::text("0"),
            LandQueryValue::text("0"),
            LandQueryValue::null(),
            LandQueryValue::text("256"),
            LandQueryValue::text(""),
        ]);
        let record = decode_land_query_row_legacy(&row).unwrap();
        assert_eq!(record.id, 0);
        assert_eq!(record.map_index, 0);
        assert_eq!(record.guild_id, 0);
        assert_eq!(record.guild_level_limit, 0);
        assert_eq!(record.price, 0);
    }

    #[test]
    fn source_errors_and_widths_are_not_hidden_by_compatibility_mode() {
        let source_error = LandTableQueryRow::new([
            LandQueryValue::error("driver failure"),
            text("1"),
            text("2"),
            text("3"),
            text("4"),
            text("5"),
            text("6"),
            text("7"),
            text("8"),
        ]);
        assert!(matches!(
            decode_land_query_row_legacy(&source_error),
            Err(LandRowError::Source { column: 0, .. })
        ));
        let short = LandTableQueryRow::new(vec![text("1"); 8]);
        assert!(matches!(
            decode_land_query_row_legacy(&short),
            Err(LandRowError::ColumnCount { .. })
        ));
    }

    #[test]
    fn limits_are_checked_before_encoding_and_source_errors_stay_distinct() {
        let valid = row(["1", "2", "3", "4", "5", "6", "7", "8", "9"]);
        assert!(matches!(
            build_land_section_with_limit(&[valid.clone(), valid.clone()], 1),
            Err(LandSectionError::TooManyRecords {
                count: 2,
                maximum: 1
            })
        ));
        assert!(matches!(
            build_land_section_with_limits(&[valid], LandSectionLimits::with_data_limit(1, 35)),
            Err(LandSectionError::DataTooLarge {
                length: 36,
                maximum: 35
            })
        ));
        let source =
            || -> Result<Vec<LandTableQueryRow>, &'static str> { Err("database unavailable") };
        assert_eq!(
            load_land_section(&source, LandSectionLimits::default()),
            Err(LandLoadError::Source("database unavailable"))
        );
    }
}
