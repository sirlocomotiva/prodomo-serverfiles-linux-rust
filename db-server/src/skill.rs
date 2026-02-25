//! SQL-free conversion of the active legacy `skill_proto` rows to a boot section.
//!
//! The legacy DB loader selects thirty columns. This module retains every
//! supplied row, including duplicates, and encodes rows in source order. It
//! does not execute SQL, sort, deduplicate, or drop fields.
//!
//! [`decode_skill_query_row`] is the strict boundary. It rejects SQL `NULL`,
//! acquisition errors, wrong-width rows, non-decimal numeric text (including
//! whitespace, a leading `+`, or a numeric prefix), target-width overflow,
//! interior NUL bytes in string fields, and strings too large for a
//! zero-terminated destination.
//!
//! [`decode_skill_query_row_legacy`] is an explicit compatibility boundary.
//! It models `strlcpy` truncation to at most `capacity - 1` bytes and the
//! active x86 `str_to_number` helpers. Those helpers accept leading C
//! whitespace, an optional sign, and a numeric prefix, then cast a 32-bit
//! `strtol`/`strtoul` result to the destination type. Each legacy row is
//! zero-filled first, and the numeric helpers return without assignment for a
//! NULL pointer, so legacy numeric `NULL` safely produces zero. Legacy string
//! calls pass NULL to `strlcpy` without a verified policy, so this module keeps
//! string `NULL` as [`SkillRowError::Null`] rather than inventing a result.
//! [`SkillQueryValue::Error`] remains a typed source error in both policies.
//!
//! The active x86 `#pragma pack(1)` record is exactly 1,475 bytes. This
//! module uses the field-by-field [`SkillTableRecord`] codec from
//! `protocol::db_records`; Rust struct layout is never used as wire layout.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootSection, BootSectionKind};
pub use protocol::db_records::{
    SkillRecord, SkillTableRecord, TSkillTable, SKILL_NAME_BYTES, SKILL_POINT_ON_BYTES,
    SKILL_POLY_EXPR_BYTES, SKILL_TABLE_RECORD_WIRE_SIZE,
};

/// Source table name before the validated table postfix.
pub const SKILL_TABLE: &str = "skill_proto";

/// Number of columns selected by the fixed legacy skill query.
pub const SKILL_TABLE_QUERY_COLUMNS: usize = 30;

/// Alias for [`SKILL_TABLE_QUERY_COLUMNS`].
pub const SKILL_QUERY_COLUMN_COUNT: usize = SKILL_TABLE_QUERY_COLUMNS;

/// Maximum number of records representable by the boot section's `u16` count.
pub const SKILL_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Exact packed active x86 `TSkillTable` width represented by a section record.
pub const SKILL_SECTION_RECORD_SIZE: u16 = 1_475;

/// Alias for the source-fixed record width.
pub const SKILL_TABLE_WIRE_SIZE: usize = SKILL_TABLE_RECORD_WIRE_SIZE;

/// Maximum record-data byte length representable by the `u16` row count.
pub const SKILL_TABLE_MAX_SECTION_BYTES: usize = SKILL_TABLE_WIRE_SIZE * SKILL_TABLE_MAX_RECORDS;

/// Column names in the exact active query order.
///
/// The four `+0` expressions are retained verbatim. In particular,
/// `grand_master_add_sp_cost_poly` is queried after `szDurationPoly3`, not in
/// the record's physical wire position between the first-stage polynomials and
/// `setFlag`.
pub const SKILL_TABLE_QUERY_COLUMN_NAMES: [&str; SKILL_TABLE_QUERY_COLUMNS] = [
    "dwVnum",
    "szName",
    "bType",
    "bMaxLevel",
    "dwSplashRange",
    "szPointOn",
    "szPointPoly",
    "szSPCostPoly",
    "szDurationPoly",
    "szDurationSPCostPoly",
    "szCooldownPoly",
    "szMasterBonusPoly",
    "setFlag+0",
    "setAffectFlag+0",
    "szPointOn2",
    "szPointPoly2",
    "szDurationPoly2",
    "setAffectFlag2+0",
    "szPointOn3",
    "szPointPoly3",
    "szDurationPoly3",
    "szGrandMasterAddSPCostPoly",
    "bLevelStep",
    "bLevelLimit",
    "prerequisiteSkillVnum",
    "prerequisiteSkillLevel",
    "iMaxHit",
    "szSplashAroundDamageAdjustPoly",
    "eSkillType+0",
    "dwTargetRange",
];

/// One value returned by a query-row adapter.
///
/// [`Self::Text`] and [`Self::Bytes`] both represent non-NULL values.
/// `Text` is convenient for tests and text-first adapters. `Bytes` lets a
/// `SQLx` adapter retain the driver's raw bytes without a UTF-8 conversion.
/// Numeric and fixed-string decoding inspect the original bytes in either
/// representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillQueryValue {
    /// A non-NULL textual value.
    Text(String),
    /// A non-NULL raw SQL value.
    Bytes(Vec<u8>),
    /// A SQL `NULL` value.
    Null,
    /// An error raised while obtaining or converting one query column.
    Error(String),
}

impl SkillQueryValue {
    /// Construct a non-NULL textual value.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    /// Copy a non-NULL raw SQL value.
    #[must_use]
    pub fn bytes(value: impl AsRef<[u8]>) -> Self {
        Self::Bytes(value.as_ref().to_vec())
    }

    /// Construct a SQL `NULL` value.
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

impl From<String> for SkillQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for SkillQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for SkillQueryValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for SkillQueryValue {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

/// A row-shaped value for the thirty-column skill query.
///
/// The vector is deliberately retained so a malformed source can report its
/// actual width. [`Self::new`] accepts any width; [`Self::try_new`] validates
/// the source-fixed width during construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTableQueryRow {
    columns: Vec<SkillQueryValue>,
}

impl SkillTableQueryRow {
    /// Construct a row while retaining all supplied values and their order.
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = SkillQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub fn from_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = SkillQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct and validate a thirty-column row.
    ///
    /// # Errors
    ///
    /// Returns [`SkillRowError::ColumnCount`] when the iterator does not yield
    /// exactly [`SKILL_TABLE_QUERY_COLUMNS`] values.
    pub fn try_new<I>(columns: I) -> Result<Self, SkillRowError>
    where
        I: IntoIterator<Item = SkillQueryValue>,
    {
        let row = Self::new(columns);
        if row.columns.len() != SKILL_TABLE_QUERY_COLUMNS {
            return Err(SkillRowError::ColumnCount {
                expected: SKILL_TABLE_QUERY_COLUMNS,
                actual: row.columns.len(),
            });
        }
        Ok(row)
    }

    /// Construct a row from a caller-supplied iterable of columns.
    ///
    /// This compatibility constructor accepts both arrays and fallibly reserved
    /// vectors. It retains any width; use [`Self::try_new`] or a row decoder
    /// when the exact thirty-column shape must be validated.
    #[must_use]
    pub fn from_typed_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = SkillQueryValue>,
    {
        Self::new(columns)
    }

    /// Borrow all supplied columns in query order.
    #[must_use]
    pub fn columns(&self) -> &[SkillQueryValue] {
        &self.columns
    }

    /// Consume the row and return its columns in query order.
    #[must_use]
    pub fn into_columns(self) -> Vec<SkillQueryValue> {
        self.columns
    }

    /// Return the number of columns supplied by the source.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

impl From<[SkillQueryValue; SKILL_TABLE_QUERY_COLUMNS]> for SkillTableQueryRow {
    fn from(columns: [SkillQueryValue; SKILL_TABLE_QUERY_COLUMNS]) -> Self {
        Self::from_typed_columns(columns)
    }
}

impl TryFrom<Vec<SkillQueryValue>> for SkillTableQueryRow {
    type Error = SkillRowError;

    fn try_from(columns: Vec<SkillQueryValue>) -> Result<Self, Self::Error> {
        Self::try_new(columns)
    }
}

/// Short alias for [`SkillTableQueryRow`].
pub type SkillQueryRow = SkillTableQueryRow;
/// Alias for a single skill query-cell value.
pub type SkillQueryColumn = SkillQueryValue;
/// Table-oriented alias for [`SkillQueryValue`].
pub type SkillTableCell = SkillQueryValue;
/// Table-oriented alias for [`SkillTableQueryRow`].
pub type SkillTableRow = SkillTableQueryRow;

/// Return a stable source-query column name for diagnostics.
#[must_use]
pub const fn skill_query_column_name(index: usize) -> &'static str {
    match index {
        0 => "dwVnum",
        1 => "szName",
        2 => "bType",
        3 => "bMaxLevel",
        4 => "dwSplashRange",
        5 => "szPointOn",
        6 => "szPointPoly",
        7 => "szSPCostPoly",
        8 => "szDurationPoly",
        9 => "szDurationSPCostPoly",
        10 => "szCooldownPoly",
        11 => "szMasterBonusPoly",
        12 => "setFlag+0",
        13 => "setAffectFlag+0",
        14 => "szPointOn2",
        15 => "szPointPoly2",
        16 => "szDurationPoly2",
        17 => "setAffectFlag2+0",
        18 => "szPointOn3",
        19 => "szPointPoly3",
        20 => "szDurationPoly3",
        21 => "szGrandMasterAddSPCostPoly",
        22 => "bLevelStep",
        23 => "bLevelLimit",
        24 => "prerequisiteSkillVnum",
        25 => "prerequisiteSkillLevel",
        26 => "iMaxHit",
        27 => "szSplashAroundDamageAdjustPoly",
        28 => "eSkillType+0",
        29 => "dwTargetRange",
        _ => "unknown",
    }
}

/// A strict or explicitly selected compatibility error in one skill row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillRowError {
    /// The row did not contain exactly thirty columns.
    ColumnCount {
        /// Required column count.
        expected: usize,
        /// Supplied column count.
        actual: usize,
    },
    /// A strict-policy cell or legacy-policy string was SQL `NULL`.
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
        /// Lossy diagnostic representation of the original bytes.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A syntactically valid integer did not fit its target type.
    NumberOverflow {
        /// Zero-based query column index.
        column: usize,
        /// Lossy diagnostic representation of the original bytes.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A string cell contained a NUL before its final byte.
    InteriorNul {
        /// Zero-based query column index.
        column: usize,
        /// Zero-based byte offset of the first NUL.
        offset: usize,
    },
    /// A string did not fit its fixed zero-terminated array.
    StringTooLong {
        /// Zero-based query column index.
        column: usize,
        /// Supplied byte length.
        length: usize,
        /// Fixed destination array capacity, including its NUL.
        capacity: usize,
        /// Maximum accepted source length for the detected string shape.
        maximum: usize,
    },
}

impl fmt::Display for SkillRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => write!(
                formatter,
                "skill query row has {actual} columns; expected {expected}"
            ),
            Self::Null { column } => write!(
                formatter,
                "skill query column {} is NULL",
                skill_query_column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "skill query column {} could not be read: {message}",
                skill_query_column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "skill query column {} value {value:?} is not a strict {target}",
                skill_query_column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "skill query column {} value {value:?} overflows {target}",
                skill_query_column_name(*column)
            ),
            Self::InteriorNul { column, offset } => write!(
                formatter,
                "skill query column {} contains an interior NUL at byte {offset}",
                skill_query_column_name(*column)
            ),
            Self::StringTooLong {
                column,
                length,
                capacity,
                maximum,
            } => write!(
                formatter,
                "skill query column {} is {length} bytes; capacity {capacity} allows {maximum}",
                skill_query_column_name(*column)
            ),
        }
    }
}

impl Error for SkillRowError {}

/// Limits applied before allocating a skill section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillSectionLimits {
    /// Maximum number of source rows accepted.
    pub max_records: usize,
    /// Maximum number of packed record-data bytes accepted.
    pub max_data_bytes: usize,
}

impl SkillSectionLimits {
    /// Construct a row limit while leaving the byte limit at `usize::MAX`.
    ///
    /// The representable `u16` section count and checked multiplication still
    /// apply.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            max_records,
            max_data_bytes: usize::MAX,
        }
    }

    /// Construct explicit record and packed-byte limits.
    #[must_use]
    pub const fn with_data_limit(max_records: usize, max_data_bytes: usize) -> Self {
        Self {
            max_records,
            max_data_bytes,
        }
    }
}

impl Default for SkillSectionLimits {
    fn default() -> Self {
        Self {
            max_records: SKILL_TABLE_MAX_RECORDS,
            max_data_bytes: SKILL_TABLE_MAX_SECTION_BYTES,
        }
    }
}

/// A checked section-construction error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillSectionError {
    /// The source supplied more rows than the caller allowed.
    TooManyRecords {
        /// Supplied row count.
        count: usize,
        /// Configured row limit.
        maximum: usize,
    },
    /// The row count cannot be represented by the boot section's `u16` count.
    CountOverflow {
        /// Supplied row count.
        count: usize,
    },
    /// The fixed record width cannot be represented by `u16`.
    RecordSizeOverflow {
        /// Fixed record width.
        size: usize,
    },
    /// The record-data size arithmetic overflowed `usize`.
    DataSizeOverflow {
        /// Row count used in the multiplication.
        count: usize,
    },
    /// The output vector could not reserve its already-bounded length.
    AllocationFailed {
        /// Requested output byte length.
        requested: usize,
    },
    /// The record data exceeds the caller's byte limit.
    DataTooLarge {
        /// Required record-data length.
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
        source: SkillRowError,
    },
}

impl fmt::Display for SkillSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "skill section has {count} rows; configured limit is {maximum}"
            ),
            Self::CountOverflow { count } => {
                write!(formatter, "skill section count {count} does not fit u16")
            }
            Self::RecordSizeOverflow { size } => {
                write!(formatter, "skill record width {size} does not fit u16")
            }
            Self::DataSizeOverflow { count } => write!(
                formatter,
                "skill section byte size overflows usize for {count} rows"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "skill section could not allocate {requested} output bytes"
            ),
            Self::DataTooLarge { length, maximum } => write!(
                formatter,
                "skill section data length {length} exceeds limit {maximum}"
            ),
            Self::RecordSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "skill row {index} encoded to {actual} bytes; expected {expected}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "skill row {index} is invalid: {source}")
            }
        }
    }
}

impl Error for SkillSectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Alias emphasizing that this type is a section-build error.
pub type SkillSectionBuildError = SkillSectionError;

/// An injected, fallible source of skill query rows.
pub trait SkillTableRowSource {
    /// Error returned when the source cannot produce rows.
    type Error: fmt::Display;

    /// Load rows in the order they should appear in the boot section.
    ///
    /// # Errors
    ///
    /// Returns the source-specific error when rows cannot be loaded.
    fn load_rows(&self) -> Result<Vec<SkillTableQueryRow>, Self::Error>;
}

impl<F, E> SkillTableRowSource for F
where
    F: Fn() -> Result<Vec<SkillTableQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn load_rows(&self) -> Result<Vec<SkillTableQueryRow>, Self::Error> {
        self()
    }
}

/// Short alias for [`SkillTableRowSource`].
pub use SkillTableRowSource as SkillRowSource;

/// An error while obtaining skill rows or building their section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillLoadError<E> {
    /// The source returned no rows. The active legacy loader rejects this.
    NoRows,
    /// The injected row source failed.
    Source(E),
    /// Rows were obtained but could not be decoded or bounded.
    Section(SkillSectionError),
}

impl<E: fmt::Display> fmt::Display for SkillLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRows => formatter.write_str("skill source returned no rows"),
            Self::Source(source) => write!(formatter, "skill row source failed: {source}"),
            Self::Section(source) => {
                write!(formatter, "skill section construction failed: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for SkillLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Section(source) => Some(source),
            Self::NoRows => None,
        }
    }
}

impl<E> From<SkillSectionError> for SkillLoadError<E> {
    fn from(source: SkillSectionError) -> Self {
        Self::Section(source)
    }
}

/// A reusable pure section builder with explicit limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillSectionBuilder {
    limits: SkillSectionLimits,
}

impl SkillSectionBuilder {
    /// Construct a builder with a caller-selected row cap.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            limits: SkillSectionLimits::new(max_records),
        }
    }

    /// Construct a builder with both limits.
    #[must_use]
    pub const fn with_limits(limits: SkillSectionLimits) -> Self {
        Self { limits }
    }

    /// Return the configured limits.
    #[must_use]
    pub const fn limits(self) -> SkillSectionLimits {
        self.limits
    }

    /// Decode and build a strict section, preserving source row order.
    ///
    /// # Errors
    ///
    /// Returns [`SkillSectionError`] for an invalid row or configured limit.
    pub fn build(&self, rows: &[SkillTableQueryRow]) -> Result<BootSection, SkillSectionError> {
        build_skill_section_with_limits(rows, self.limits)
    }

    /// Decode and build with the explicitly named legacy conversion policy.
    ///
    /// # Errors
    ///
    /// Returns [`SkillSectionError`] for a source-cell error or configured
    /// limit. Numeric syntax is intentionally permissive in this method.
    pub fn build_legacy(
        &self,
        rows: &[SkillTableQueryRow],
    ) -> Result<BootSection, SkillSectionError> {
        build_skill_section_legacy_with_limits(rows, self.limits)
    }
}

fn cell_bytes(value: &SkillQueryValue, column: usize) -> Result<&[u8], SkillRowError> {
    match value {
        SkillQueryValue::Text(text) => Ok(text.as_bytes()),
        SkillQueryValue::Bytes(bytes) => Ok(bytes),
        SkillQueryValue::Null => Err(SkillRowError::Null { column }),
        SkillQueryValue::Error(message) => Err(SkillRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn legacy_numeric_bytes(value: &SkillQueryValue, column: usize) -> Result<&[u8], SkillRowError> {
    match value {
        SkillQueryValue::Null => Ok(&[]),
        SkillQueryValue::Text(text) => Ok(text.as_bytes()),
        SkillQueryValue::Bytes(bytes) => Ok(bytes),
        SkillQueryValue::Error(message) => Err(SkillRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn is_strict_unsigned_decimal(value: &[u8]) -> bool {
    !value.is_empty() && value.iter().all(u8::is_ascii_digit)
}

fn is_strict_signed_decimal(value: &[u8]) -> bool {
    let Some(first) = value.first().copied() else {
        return false;
    };
    let digits = if first == b'-' { &value[1..] } else { value };
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

fn diagnostic_value(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn decode_strict_u8(value: &SkillQueryValue, column: usize) -> Result<u8, SkillRowError> {
    let bytes = cell_bytes(value, column)?;
    if !is_strict_unsigned_decimal(bytes) {
        return Err(SkillRowError::InvalidNumber {
            column,
            value: diagnostic_value(bytes),
            target: "u8",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| SkillRowError::InvalidNumber {
        column,
        value: diagnostic_value(bytes),
        target: "u8",
    })?;
    text.parse::<u8>()
        .map_err(|_| SkillRowError::NumberOverflow {
            column,
            value: diagnostic_value(bytes),
            target: "u8",
        })
}

fn decode_strict_u32(value: &SkillQueryValue, column: usize) -> Result<u32, SkillRowError> {
    let bytes = cell_bytes(value, column)?;
    if !is_strict_unsigned_decimal(bytes) {
        return Err(SkillRowError::InvalidNumber {
            column,
            value: diagnostic_value(bytes),
            target: "u32",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| SkillRowError::InvalidNumber {
        column,
        value: diagnostic_value(bytes),
        target: "u32",
    })?;
    text.parse::<u32>()
        .map_err(|_| SkillRowError::NumberOverflow {
            column,
            value: diagnostic_value(bytes),
            target: "u32",
        })
}

fn decode_strict_i32(value: &SkillQueryValue, column: usize) -> Result<i32, SkillRowError> {
    let bytes = cell_bytes(value, column)?;
    if !is_strict_signed_decimal(bytes) {
        return Err(SkillRowError::InvalidNumber {
            column,
            value: diagnostic_value(bytes),
            target: "i32",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| SkillRowError::InvalidNumber {
        column,
        value: diagnostic_value(bytes),
        target: "i32",
    })?;
    text.parse::<i32>()
        .map_err(|_| SkillRowError::NumberOverflow {
            column,
            value: diagnostic_value(bytes),
            target: "i32",
        })
}

fn decode_strict_string<const CAPACITY: usize>(
    value: &SkillQueryValue,
    column: usize,
) -> Result<[u8; CAPACITY], SkillRowError> {
    let bytes = cell_bytes(value, column)?;
    if bytes.len() > CAPACITY {
        return Err(SkillRowError::StringTooLong {
            column,
            length: bytes.len(),
            capacity: CAPACITY,
            maximum: CAPACITY,
        });
    }
    if let Some(offset) = bytes.iter().position(|byte| *byte == 0) {
        if offset + 1 != bytes.len() {
            return Err(SkillRowError::InteriorNul { column, offset });
        }
    }

    let maximum = if bytes.last() == Some(&0) {
        CAPACITY
    } else {
        CAPACITY.saturating_sub(1)
    };
    if bytes.len() > maximum {
        return Err(SkillRowError::StringTooLong {
            column,
            length: bytes.len(),
            capacity: CAPACITY,
            maximum,
        });
    }

    let mut result = [0_u8; CAPACITY];
    result[..bytes.len()].copy_from_slice(bytes);
    Ok(result)
}

fn check_skill_row_width(row: &SkillTableQueryRow) -> Result<(), SkillRowError> {
    if row.columns.len() == SKILL_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(SkillRowError::ColumnCount {
            expected: SKILL_TABLE_QUERY_COLUMNS,
            actual: row.columns.len(),
        })
    }
}

/// Strictly decode one thirty-column skill query row.
///
/// The query's field order differs from the record's physical order for
/// `szGrandMasterAddSPCostPoly`; this function maps both orders explicitly.
/// Strict numeric cells contain ASCII decimal digits only. Signed `i32`
/// values may have one leading `-`; whitespace, `+`, empty strings, and
/// trailing text are rejected. Fixed string cells may end in one NUL, reject
/// any earlier NUL, and otherwise fit with a zero terminator in their array.
///
/// # Errors
///
/// Returns [`SkillRowError`] for a wrong-width row or any invalid cell.
pub fn decode_skill_query_row(row: &SkillTableQueryRow) -> Result<SkillTableRecord, SkillRowError> {
    check_skill_row_width(row)?;
    Ok(SkillTableRecord {
        vnum: decode_strict_u32(&row.columns[0], 0)?,
        name: decode_strict_string::<SKILL_NAME_BYTES>(&row.columns[1], 1)?,
        skill_type: decode_strict_u8(&row.columns[2], 2)?,
        max_level: decode_strict_u8(&row.columns[3], 3)?,
        splash_range: decode_strict_u32(&row.columns[4], 4)?,
        point_on: decode_strict_string::<SKILL_POINT_ON_BYTES>(&row.columns[5], 5)?,
        point_poly: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[6], 6)?,
        sp_cost_poly: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[7], 7)?,
        duration_poly: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[8], 8)?,
        duration_sp_cost_poly: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[9], 9)?,
        cooldown_poly: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[10], 10)?,
        master_bonus_poly: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[11], 11)?,
        flag: decode_strict_u32(&row.columns[12], 12)?,
        affect_flag: decode_strict_u32(&row.columns[13], 13)?,
        point_on2: decode_strict_string::<SKILL_POINT_ON_BYTES>(&row.columns[14], 14)?,
        point_poly2: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[15], 15)?,
        duration_poly2: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[16], 16)?,
        affect_flag2: decode_strict_u32(&row.columns[17], 17)?,
        point_on3: decode_strict_string::<SKILL_POINT_ON_BYTES>(&row.columns[18], 18)?,
        point_poly3: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[19], 19)?,
        duration_poly3: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[20], 20)?,
        grand_master_add_sp_cost_poly: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(
            &row.columns[21],
            21,
        )?,
        level_step: decode_strict_u8(&row.columns[22], 22)?,
        level_limit: decode_strict_u8(&row.columns[23], 23)?,
        pre_skill_vnum: decode_strict_u32(&row.columns[24], 24)?,
        pre_skill_level: decode_strict_u8(&row.columns[25], 25)?,
        max_hit: decode_strict_i32(&row.columns[26], 26)?,
        splash_around_damage_adjust_poly: decode_strict_string::<SKILL_POLY_EXPR_BYTES>(
            &row.columns[27],
            27,
        )?,
        skill_attr_type: decode_strict_u8(&row.columns[28], 28)?,
        target_range: decode_strict_u32(&row.columns[29], 29)?,
    })
}

/// Explicitly named spelling of the strict row decoder.
///
/// # Errors
///
/// Returns the same errors as [`decode_skill_query_row`].
pub fn decode_skill_query_row_strict(
    row: &SkillTableQueryRow,
) -> Result<SkillTableRecord, SkillRowError> {
    decode_skill_query_row(row)
}

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Parse the x86 32-bit C `strtol`/`strtoul` numeric prefix.
///
/// The sign and magnitude remain separate so the caller can apply the correct
/// 32-bit saturation and destination cast. A NUL terminates the C string.
/// Empty or nonnumeric text produces a zero magnitude.
fn legacy_numeric_prefix(bytes: &[u8]) -> (bool, Option<u64>) {
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

fn legacy_i32(value: &SkillQueryValue, column: usize) -> Result<i32, SkillRowError> {
    let bytes = legacy_numeric_bytes(value, column)?;
    let c_string = bytes
        .iter()
        .position(|byte| *byte == 0)
        .map_or(bytes, |end| &bytes[..end]);
    let (negative, magnitude) = legacy_numeric_prefix(c_string);
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
    } else if magnitude > 2_147_483_647 {
        Ok(i32::MAX)
    } else {
        match i32::try_from(magnitude) {
            Ok(value) => Ok(value),
            Err(_) => Ok(i32::MAX),
        }
    }
}

fn legacy_u32(value: &SkillQueryValue, column: usize) -> Result<u32, SkillRowError> {
    let bytes = legacy_numeric_bytes(value, column)?;
    let c_string = bytes
        .iter()
        .position(|byte| *byte == 0)
        .map_or(bytes, |end| &bytes[..end]);
    let (negative, magnitude) = legacy_numeric_prefix(c_string);
    let Some(magnitude) = magnitude else {
        return Ok(0);
    };

    if magnitude > u64::from(u32::MAX) {
        return Ok(u32::MAX);
    }
    let reduced = u32::try_from(magnitude).unwrap_or(u32::MAX);
    if negative {
        Ok(0_u32.wrapping_sub(reduced))
    } else {
        Ok(reduced)
    }
}

fn legacy_u8(value: &SkillQueryValue, column: usize) -> Result<u8, SkillRowError> {
    let value = legacy_u32(value, column)?;
    Ok(u8::try_from(value & 0xff).unwrap_or(0))
}

fn decode_legacy_string<const CAPACITY: usize>(
    value: &SkillQueryValue,
    column: usize,
) -> Result<[u8; CAPACITY], SkillRowError> {
    let bytes = cell_bytes(value, column)?;
    let copy_length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())
        .min(CAPACITY.saturating_sub(1));
    let mut result = [0_u8; CAPACITY];
    result[..copy_length].copy_from_slice(&bytes[..copy_length]);
    Ok(result)
}

/// Decode one row with the explicitly named legacy conversion policy.
///
/// Fixed strings follow `strlcpy`: at most `capacity - 1` bytes are copied,
/// a NUL terminates copying, and the rest of the destination is zero-filled.
/// Numeric fields follow the active x86 helpers: `strtol` for `long`,
/// `strtoul` for `DWORD` and `BYTE`, including numeric prefixes, destination
/// casts, and 32-bit saturation. A NULL numeric pointer makes the helper leave
/// its zero-initialized destination unchanged. SQL `NULL` in a string remains
/// a typed error because the unchecked `strlcpy` result is not verified.
///
/// # Errors
///
/// Returns [`SkillRowError`] for a wrong-width row, a string `NULL`, or a
/// source cell error. Numeric and string syntax do not fail in this policy.
pub fn decode_skill_query_row_legacy(
    row: &SkillTableQueryRow,
) -> Result<SkillTableRecord, SkillRowError> {
    check_skill_row_width(row)?;
    Ok(SkillTableRecord {
        vnum: legacy_u32(&row.columns[0], 0)?,
        name: decode_legacy_string::<SKILL_NAME_BYTES>(&row.columns[1], 1)?,
        skill_type: legacy_u8(&row.columns[2], 2)?,
        max_level: legacy_u8(&row.columns[3], 3)?,
        splash_range: legacy_u32(&row.columns[4], 4)?,
        point_on: decode_legacy_string::<SKILL_POINT_ON_BYTES>(&row.columns[5], 5)?,
        point_poly: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[6], 6)?,
        sp_cost_poly: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[7], 7)?,
        duration_poly: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[8], 8)?,
        duration_sp_cost_poly: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[9], 9)?,
        cooldown_poly: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[10], 10)?,
        master_bonus_poly: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[11], 11)?,
        flag: legacy_u32(&row.columns[12], 12)?,
        affect_flag: legacy_u32(&row.columns[13], 13)?,
        point_on2: decode_legacy_string::<SKILL_POINT_ON_BYTES>(&row.columns[14], 14)?,
        point_poly2: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[15], 15)?,
        duration_poly2: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[16], 16)?,
        affect_flag2: legacy_u32(&row.columns[17], 17)?,
        point_on3: decode_legacy_string::<SKILL_POINT_ON_BYTES>(&row.columns[18], 18)?,
        point_poly3: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[19], 19)?,
        duration_poly3: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(&row.columns[20], 20)?,
        grand_master_add_sp_cost_poly: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(
            &row.columns[21],
            21,
        )?,
        level_step: legacy_u8(&row.columns[22], 22)?,
        level_limit: legacy_u8(&row.columns[23], 23)?,
        pre_skill_vnum: legacy_u32(&row.columns[24], 24)?,
        pre_skill_level: legacy_u8(&row.columns[25], 25)?,
        max_hit: legacy_i32(&row.columns[26], 26)?,
        splash_around_damage_adjust_poly: decode_legacy_string::<SKILL_POLY_EXPR_BYTES>(
            &row.columns[27],
            27,
        )?,
        skill_attr_type: legacy_u8(&row.columns[28], 28)?,
        target_range: legacy_u32(&row.columns[29], 29)?,
    })
}

fn validate_section_size(
    count: usize,
    limits: SkillSectionLimits,
) -> Result<(u16, u16, usize), SkillSectionError> {
    if count > limits.max_records {
        return Err(SkillSectionError::TooManyRecords {
            count,
            maximum: limits.max_records,
        });
    }
    let wire_count =
        u16::try_from(count).map_err(|_| SkillSectionError::CountOverflow { count })?;
    let record_size = u16::try_from(SKILL_TABLE_WIRE_SIZE).map_err(|_| {
        SkillSectionError::RecordSizeOverflow {
            size: SKILL_TABLE_WIRE_SIZE,
        }
    })?;
    let data_len = SKILL_TABLE_WIRE_SIZE
        .checked_mul(count)
        .ok_or(SkillSectionError::DataSizeOverflow { count })?;
    if data_len > limits.max_data_bytes {
        return Err(SkillSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }
    Ok((wire_count, record_size, data_len))
}

fn append_rows(
    rows: &[SkillTableQueryRow],
    limits: SkillSectionLimits,
    legacy: bool,
) -> Result<BootSection, SkillSectionError> {
    let (count, record_size, data_len) = validate_section_size(rows.len(), limits)?;
    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| SkillSectionError::AllocationFailed {
            requested: data_len,
        })?;

    for (index, row) in rows.iter().enumerate() {
        let record = if legacy {
            decode_skill_query_row_legacy(row)
        } else {
            decode_skill_query_row(row)
        }
        .map_err(|source| SkillSectionError::Row { index, source })?;
        let encoded = record.encode();
        if encoded.len() != SKILL_TABLE_WIRE_SIZE {
            return Err(SkillSectionError::RecordSizeMismatch {
                index,
                expected: SKILL_TABLE_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }

    debug_assert_eq!(data.len(), data_len);
    Ok(BootSection {
        kind: BootSectionKind::Skill,
        record_size,
        count,
        data,
    })
}

/// Build a strict skill section with default limits.
///
/// A pure bounded empty input produces an exact zero-count section. The
/// source-level loaders apply the legacy loader's separate no-rows rejection.
///
/// # Errors
///
/// Returns [`SkillSectionError`] for an invalid row or configured limit.
pub fn build_skill_section(rows: &[SkillTableQueryRow]) -> Result<BootSection, SkillSectionError> {
    build_skill_section_with_limits(rows, SkillSectionLimits::default())
}

/// Build a strict skill section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`SkillSectionError`] for an invalid row or configured limit.
pub fn build_skill_section_with_limit(
    rows: &[SkillTableQueryRow],
    max_records: usize,
) -> Result<BootSection, SkillSectionError> {
    build_skill_section_with_limits(rows, SkillSectionLimits::new(max_records))
}

/// Decode rows and build a strict typed skill boot section.
///
/// Row order and duplicates are preserved. Limits, `u16` count conversion,
/// `1,475 * count` arithmetic, and fallible output reservation are checked
/// before encoding begins.
///
/// # Errors
///
/// Returns [`SkillSectionError`] for an invalid row, count conversion,
/// allocation failure, or configured limit violation.
pub fn build_skill_section_with_limits(
    rows: &[SkillTableQueryRow],
    limits: SkillSectionLimits,
) -> Result<BootSection, SkillSectionError> {
    append_rows(rows, limits, false)
}

/// Explicitly named spelling of the strict section builder.
///
/// # Errors
///
/// Returns the same errors as [`build_skill_section_with_limits`].
pub fn build_skill_section_strict_with_limits(
    rows: &[SkillTableQueryRow],
    limits: SkillSectionLimits,
) -> Result<BootSection, SkillSectionError> {
    build_skill_section_with_limits(rows, limits)
}

/// Build a skill section with the explicitly named legacy conversion policy.
///
/// # Errors
///
/// Returns [`SkillSectionError`] for SQL `NULL`, a source-cell error,
/// allocation failure, or configured limit violation.
pub fn build_skill_section_legacy(
    rows: &[SkillTableQueryRow],
) -> Result<BootSection, SkillSectionError> {
    build_skill_section_legacy_with_limits(rows, SkillSectionLimits::default())
}

/// Build a legacy-policy skill section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`SkillSectionError`] for SQL `NULL`, a source-cell error,
/// allocation failure, or configured limit violation.
pub fn build_skill_section_legacy_with_limit(
    rows: &[SkillTableQueryRow],
    max_records: usize,
) -> Result<BootSection, SkillSectionError> {
    build_skill_section_legacy_with_limits(rows, SkillSectionLimits::new(max_records))
}

/// Decode rows and build a section with the legacy conversion policy.
///
/// # Errors
///
/// Returns [`SkillSectionError`] for SQL `NULL`, a source-cell error,
/// allocation failure, or configured limit violation.
pub fn build_skill_section_legacy_with_limits(
    rows: &[SkillTableQueryRow],
    limits: SkillSectionLimits,
) -> Result<BootSection, SkillSectionError> {
    append_rows(rows, limits, true)
}

fn load_rows<S>(source: &S) -> Result<Vec<SkillTableQueryRow>, SkillLoadError<S::Error>>
where
    S: SkillTableRowSource,
{
    let rows = source.load_rows().map_err(SkillLoadError::Source)?;
    if rows.is_empty() {
        Err(SkillLoadError::NoRows)
    } else {
        Ok(rows)
    }
}

/// Load rows from an injected source and build a strict skill section.
///
/// The active legacy loader rejects a zero-row query result. This function
/// exposes that as [`SkillLoadError::NoRows`] instead of fabricating a boot
/// section. Call [`build_skill_section`] directly only when an empty pure
/// section is explicitly desired.
///
/// # Errors
///
/// Returns [`SkillLoadError::Source`] for a source failure,
/// [`SkillLoadError::NoRows`] for an empty result, and
/// [`SkillLoadError::Section`] for row or limit failures.
pub fn load_skill_section<S>(
    source: &S,
    limits: SkillSectionLimits,
) -> Result<BootSection, SkillLoadError<S::Error>>
where
    S: SkillTableRowSource,
{
    let rows = load_rows(source)?;
    build_skill_section_with_limits(&rows, limits).map_err(SkillLoadError::Section)
}

/// Load rows and build a strict section with a caller-selected row cap.
///
/// # Errors
///
/// Returns the same errors as [`load_skill_section`].
pub fn load_skill_section_with_limit<S>(
    source: &S,
    max_records: usize,
) -> Result<BootSection, SkillLoadError<S::Error>>
where
    S: SkillTableRowSource,
{
    load_skill_section(source, SkillSectionLimits::new(max_records))
}

/// Load rows from an injected source and build with the legacy policy.
///
/// # Errors
///
/// Returns the same source, empty-result, row, and limit errors as
/// [`load_skill_section`]. The selected row policy is explicitly legacy.
pub fn load_skill_section_legacy<S>(
    source: &S,
    limits: SkillSectionLimits,
) -> Result<BootSection, SkillLoadError<S::Error>>
where
    S: SkillTableRowSource,
{
    let rows = load_rows(source)?;
    build_skill_section_legacy_with_limits(&rows, limits).map_err(SkillLoadError::Section)
}

/// Load rows and build a legacy-policy section with a row cap.
///
/// # Errors
///
/// Returns the same errors as [`load_skill_section_legacy`].
pub fn load_skill_section_legacy_with_limit<S>(
    source: &S,
    max_records: usize,
) -> Result<BootSection, SkillLoadError<S::Error>>
where
    S: SkillTableRowSource,
{
    load_skill_section_legacy(source, SkillSectionLimits::new(max_records))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::array;

    fn text(value: &str) -> SkillQueryValue {
        SkillQueryValue::text(value)
    }

    fn valid_columns() -> [SkillQueryValue; SKILL_TABLE_QUERY_COLUMNS] {
        array::from_fn(|_| text("0"))
    }

    fn valid_row() -> SkillTableQueryRow {
        SkillTableQueryRow::from_typed_columns(valid_columns())
    }

    fn row_with(columns: [SkillQueryValue; SKILL_TABLE_QUERY_COLUMNS]) -> SkillTableQueryRow {
        SkillTableQueryRow::from_typed_columns(columns)
    }

    #[test]
    fn source_query_has_exact_thirty_column_shape_and_order() {
        assert_eq!(SKILL_TABLE_QUERY_COLUMNS, 30);
        assert_eq!(SKILL_QUERY_COLUMN_COUNT, 30);
        assert_eq!(SKILL_TABLE_QUERY_COLUMN_NAMES.len(), 30);
        assert_eq!(SKILL_TABLE_QUERY_COLUMN_NAMES[0], "dwVnum");
        assert_eq!(SKILL_TABLE_QUERY_COLUMN_NAMES[12], "setFlag+0");
        assert_eq!(SKILL_TABLE_QUERY_COLUMN_NAMES[17], "setAffectFlag2+0");
        assert_eq!(SKILL_TABLE_QUERY_COLUMN_NAMES[20], "szDurationPoly3");
        assert_eq!(
            SKILL_TABLE_QUERY_COLUMN_NAMES[21],
            "szGrandMasterAddSPCostPoly"
        );
        assert_eq!(SKILL_TABLE_QUERY_COLUMN_NAMES[28], "eSkillType+0");

        for (index, expected) in SKILL_TABLE_QUERY_COLUMN_NAMES.into_iter().enumerate() {
            assert_eq!(skill_query_column_name(index), expected);
        }
        assert_eq!(skill_query_column_name(30), "unknown");

        assert!(matches!(
            SkillTableQueryRow::try_new(vec![text("0"); 29]),
            Err(SkillRowError::ColumnCount {
                expected: 30,
                actual: 29
            })
        ));
        let short = SkillTableQueryRow::new(vec![text("0"); 31]);
        assert!(matches!(
            decode_skill_query_row(&short),
            Err(SkillRowError::ColumnCount {
                expected: 30,
                actual: 31
            })
        ));
    }

    #[test]
    fn strict_decoder_maps_query_order_to_record_order() {
        let mut columns = valid_columns();
        columns[0] = text("42");
        columns[1] = text("Sword");
        columns[2] = text("3");
        columns[3] = text("9");
        columns[4] = text("4");
        columns[12] = text("5");
        columns[13] = text("6");
        columns[17] = text("7");
        columns[18] = text("Point3");
        columns[21] = text("GM");
        columns[22] = text("2");
        columns[23] = text("8");
        columns[24] = text("11");
        columns[25] = text("1");
        columns[26] = text("-12");
        columns[27] = text("Adjust");
        columns[28] = text("10");
        columns[29] = text("13");

        let record = decode_skill_query_row(&row_with(columns)).unwrap();
        assert_eq!(record.vnum, 42);
        assert_eq!(&record.name[..5], b"Sword");
        assert_eq!(record.skill_type, 3);
        assert_eq!(record.max_level, 9);
        assert_eq!(record.splash_range, 4);
        assert_eq!(record.flag, 5);
        assert_eq!(record.affect_flag, 6);
        assert_eq!(record.affect_flag2, 7);
        assert_eq!(&record.point_on3[..6], b"Point3");
        assert_eq!(&record.grand_master_add_sp_cost_poly[..2], b"GM");
        assert_eq!(record.level_step, 2);
        assert_eq!(record.level_limit, 8);
        assert_eq!(record.pre_skill_vnum, 11);
        assert_eq!(record.pre_skill_level, 1);
        assert_eq!(record.max_hit, -12);
        assert_eq!(&record.splash_around_damage_adjust_poly[..6], b"Adjust");
        assert_eq!(record.skill_attr_type, 10);
        assert_eq!(record.target_range, 13);
    }

    #[test]
    fn strict_policy_keeps_null_source_error_and_wrong_width_distinct() {
        let mut null_number = valid_columns();
        null_number[0] = SkillQueryValue::null();
        assert!(matches!(
            decode_skill_query_row(&row_with(null_number)),
            Err(SkillRowError::Null { column: 0 })
        ));

        let mut null_string = valid_columns();
        null_string[1] = SkillQueryValue::null();
        assert!(matches!(
            decode_skill_query_row(&row_with(null_string)),
            Err(SkillRowError::Null { column: 1 })
        ));

        let mut source_error = valid_columns();
        source_error[21] = SkillQueryValue::error("driver failure");
        assert!(matches!(
            decode_skill_query_row(&row_with(source_error)),
            Err(SkillRowError::Source {
                column: 21,
                ref message
            }) if message == "driver failure"
        ));
    }

    #[test]
    fn strict_numeric_policy_rejects_prefix_whitespace_plus_and_overflow() {
        for invalid in ["", " 1", "+1", "1suffix", "-1"] {
            let mut columns = valid_columns();
            columns[0] = text(invalid);
            assert!(
                matches!(
                    decode_skill_query_row(&row_with(columns)),
                    Err(SkillRowError::InvalidNumber {
                        column: 0,
                        target: "u32",
                        ..
                    })
                ),
                "unexpected strict result for {invalid:?}"
            );
        }

        let mut unsigned_overflow = valid_columns();
        unsigned_overflow[0] = text("4294967296");
        assert!(matches!(
            decode_skill_query_row(&row_with(unsigned_overflow)),
            Err(SkillRowError::NumberOverflow {
                column: 0,
                target: "u32",
                ..
            })
        ));

        let mut byte_overflow = valid_columns();
        byte_overflow[2] = text("256");
        assert!(matches!(
            decode_skill_query_row(&row_with(byte_overflow)),
            Err(SkillRowError::NumberOverflow {
                column: 2,
                target: "u8",
                ..
            })
        ));

        let mut signed_overflow = valid_columns();
        signed_overflow[26] = text("2147483648");
        assert!(matches!(
            decode_skill_query_row(&row_with(signed_overflow)),
            Err(SkillRowError::NumberOverflow {
                column: 26,
                target: "i32",
                ..
            })
        ));
    }

    #[test]
    fn strict_strings_are_zero_padded_bounded_and_nul_free() {
        let mut columns = valid_columns();
        let name = "A".repeat(32);
        let point_on = "B".repeat(63);
        let point_poly = "C".repeat(100);
        columns[1] = text(&name);
        columns[5] = text(&point_on);
        columns[6] = text(&point_poly);
        let record = decode_skill_query_row(&row_with(columns)).unwrap();
        assert_eq!(record.name[0..32], [b'A'; 32]);
        assert_eq!(record.name[32], 0);
        assert_eq!(record.point_on[0..63], [b'B'; 63]);
        assert_eq!(record.point_on[63], 0);
        assert_eq!(record.point_poly[0..100], [b'C'; 100]);
        assert_eq!(record.point_poly[100], 0);

        let mut terminated_at_capacity = valid_columns();
        let mut maximum_name = vec![b'A'; SKILL_NAME_BYTES - 1];
        maximum_name.push(0);
        terminated_at_capacity[1] = SkillQueryValue::Bytes(maximum_name);
        let record = decode_skill_query_row(&row_with(terminated_at_capacity)).unwrap();
        assert_eq!(&record.name[..SKILL_NAME_BYTES - 1], [b'A'; 32]);
        assert_eq!(record.name[SKILL_NAME_BYTES - 1], 0);

        let mut too_long = valid_columns();
        too_long[1] = text("A".repeat(33).as_str());
        assert!(matches!(
            decode_skill_query_row(&row_with(too_long)),
            Err(SkillRowError::StringTooLong {
                column: 1,
                length: 33,
                capacity: 33,
                maximum: 32
            })
        ));

        let mut embedded_nul = valid_columns();
        embedded_nul[1] = SkillQueryValue::bytes(b"A\0B");
        assert!(matches!(
            decode_skill_query_row(&row_with(embedded_nul)),
            Err(SkillRowError::InteriorNul {
                column: 1,
                offset: 1
            })
        ));

        let mut explicit_terminator = valid_columns();
        explicit_terminator[1] = SkillQueryValue::bytes(b"Name\0");
        let record = decode_skill_query_row(&row_with(explicit_terminator)).unwrap();
        assert_eq!(&record.name[..5], b"Name\0");
        assert!(record.name[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn raw_bytes_are_not_lossy_text_and_feed_both_kinds_of_field() {
        let mut columns = valid_columns();
        columns[0] = SkillQueryValue::bytes(b"17");
        columns[1] = SkillQueryValue::bytes([0xff, b'X']);
        let record = decode_skill_query_row(&row_with(columns)).unwrap();
        assert_eq!(record.vnum, 17);
        assert_eq!(&record.name[..2], &[0xff, b'X']);
        assert!(record.name[2..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn legacy_policy_models_strlcpy_and_x86_number_conversions() {
        let mut columns = valid_columns();
        columns[0] = text(" -1suffix");
        columns[1] = SkillQueryValue::bytes(vec![b'A'; 40]);
        columns[2] = text("256tail");
        columns[3] = text("");
        columns[4] = text("4294967296tail");
        columns[5] = SkillQueryValue::bytes(vec![b'B'; 70]);
        columns[6] = SkillQueryValue::bytes(b"C\0ignored");
        columns[26] = text(" -7anything");

        let record = decode_skill_query_row_legacy(&row_with(columns)).unwrap();
        assert_eq!(record.vnum, u32::MAX);
        assert_eq!(&record.name[..32], [b'A'; 32]);
        assert_eq!(record.name[32], 0);
        assert_eq!(record.skill_type, 0);
        assert_eq!(record.max_level, 0);
        assert_eq!(record.splash_range, u32::MAX);
        assert_eq!(&record.point_on[..63], [b'B'; 63]);
        assert_eq!(record.point_on[63], 0);
        assert_eq!(&record.point_poly[..2], b"C\0");
        assert_eq!(record.point_poly[2], 0);
        assert_eq!(record.max_hit, -7);
    }

    #[test]
    fn legacy_policy_zeroes_numeric_null_but_keeps_string_null_typed() {
        let mut numeric_nulls = valid_columns();
        numeric_nulls[0] = SkillQueryValue::null();
        numeric_nulls[2] = SkillQueryValue::null();
        numeric_nulls[26] = SkillQueryValue::null();
        let record = decode_skill_query_row_legacy(&row_with(numeric_nulls)).unwrap();
        assert_eq!(record.vnum, 0);
        assert_eq!(record.skill_type, 0);
        assert_eq!(record.max_hit, 0);

        let mut string_null = valid_columns();
        string_null[1] = SkillQueryValue::null();
        assert!(matches!(
            decode_skill_query_row_legacy(&row_with(string_null)),
            Err(SkillRowError::Null { column: 1 })
        ));

        let mut source_error = valid_columns();
        source_error[26] = SkillQueryValue::error("driver failure");
        assert!(matches!(
            decode_skill_query_row_legacy(&row_with(source_error)),
            Err(SkillRowError::Source {
                column: 26,
                ref message
            }) if message == "driver failure"
        ));
    }

    #[test]
    fn section_preserves_source_order_and_duplicates() {
        let mut first_columns = valid_columns();
        first_columns[0] = text("9");
        let mut second_columns = valid_columns();
        second_columns[0] = text("2");
        let first = row_with(first_columns);
        let second = row_with(second_columns);
        let section = build_skill_section(&[first.clone(), second, first.clone()]).unwrap();

        assert_eq!(section.count, 3);
        assert_eq!(&section.data[0..4], &9_u32.to_le_bytes());
        assert_eq!(&section.data[1475..1479], &2_u32.to_le_bytes());
        assert_eq!(&section.data[2950..2954], &9_u32.to_le_bytes());
    }

    #[test]
    fn one_record_has_exact_1475_byte_wire_section() {
        assert_eq!(SKILL_TABLE_RECORD_WIRE_SIZE, 1_475);
        assert_eq!(SKILL_TABLE_WIRE_SIZE, 1_475);
        assert_eq!(SKILL_SECTION_RECORD_SIZE, 1_475);
        assert_eq!(SKILL_TABLE_MAX_SECTION_BYTES, 96_664_125);

        let mut columns = valid_columns();
        columns[0] = text("77");
        columns[1] = text("N");
        let source = row_with(columns);
        let record = decode_skill_query_row(&source).unwrap();
        let section = build_skill_section(&[source]).unwrap();

        assert_eq!(section.kind, BootSectionKind::Skill);
        assert_eq!(section.record_size, 1_475);
        assert_eq!(section.count, 1);
        assert_eq!(section.data.len(), 1_475);
        assert_eq!(&section.data[0..4], &77_u32.to_le_bytes());
        assert_eq!(section.data[4], b'N');
        assert!(section.data[5..37].iter().all(|byte| *byte == 0));
        assert_eq!(SkillTableRecord::decode(&section.data).unwrap(), record);
    }

    #[test]
    fn pure_empty_section_is_distinct_from_source_no_rows_policy() {
        let empty = build_skill_section(&[]).unwrap();
        assert_eq!(empty.kind, BootSectionKind::Skill);
        assert_eq!(empty.record_size, 1_475);
        assert_eq!(empty.count, 0);
        assert!(empty.data.is_empty());

        let empty_source = || -> Result<Vec<SkillTableQueryRow>, &'static str> { Ok(Vec::new()) };
        assert_eq!(
            load_skill_section(&empty_source, SkillSectionLimits::default()),
            Err(SkillLoadError::NoRows)
        );
        assert_eq!(
            load_skill_section_legacy(&empty_source, SkillSectionLimits::default()),
            Err(SkillLoadError::NoRows)
        );
    }

    #[test]
    fn limits_count_conversion_and_record_data_are_checked() {
        let valid = valid_row();
        assert!(matches!(
            build_skill_section_with_limit(&[valid.clone(), valid.clone()], 1),
            Err(SkillSectionError::TooManyRecords {
                count: 2,
                maximum: 1
            })
        ));
        assert!(matches!(
            build_skill_section_with_limits(
                &[valid.clone()],
                SkillSectionLimits::with_data_limit(1, 1_474),
            ),
            Err(SkillSectionError::DataTooLarge {
                length: 1_475,
                maximum: 1_474,
            })
        ));

        let count = SKILL_TABLE_MAX_RECORDS + 1;
        assert!(matches!(
            validate_section_size(count, SkillSectionLimits::new(count)),
            Err(SkillSectionError::CountOverflow { count: 65_536 })
        ));

        let builder =
            SkillSectionBuilder::with_limits(SkillSectionLimits::with_data_limit(1, 1_474));
        assert!(matches!(
            builder.build(&[valid]),
            Err(SkillSectionError::DataTooLarge { .. })
        ));
    }

    #[test]
    fn injected_source_failure_is_not_an_empty_section() {
        let unavailable =
            || -> Result<Vec<SkillTableQueryRow>, &'static str> { Err("database unavailable") };
        assert_eq!(
            load_skill_section(&unavailable, SkillSectionLimits::default()),
            Err(SkillLoadError::Source("database unavailable"))
        );
        assert_eq!(
            load_skill_section_legacy(&unavailable, SkillSectionLimits::default()),
            Err(SkillLoadError::Source("database unavailable"))
        );

        let valid = valid_row();
        let available =
            || -> Result<Vec<SkillTableQueryRow>, &'static str> { Ok(vec![valid.clone()]) };
        let section = load_skill_section(&available, SkillSectionLimits::default()).unwrap();
        assert_eq!(section.count, 1);
        assert_eq!(section.data.len(), 1_475);
    }

    #[test]
    fn section_row_error_retains_source_index_and_policy() {
        let mut columns = valid_columns();
        columns[0] = SkillQueryValue::error("read failed");
        let bad = row_with(columns);
        let section_error = build_skill_section(&[valid_row(), bad]).unwrap_err();
        assert!(matches!(
            section_error,
            SkillSectionError::Row {
                index: 1,
                source: SkillRowError::Source { column: 0, .. },
            }
        ));
    }
}
