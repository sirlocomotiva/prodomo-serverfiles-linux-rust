//! Pure, SQL-free conversion of the active legacy item-attribute tables.
//!
//! `ClientManagerBoot.cpp` reads two independent, ordered vectors:
//! `item_attr%s` and `item_attr_rare%s`.  The normal query has eighteen
//! columns and the rare query has sixteen.  Both rows become the same
//! [`ItemAttrRecord`]; the rare query simply leaves the
//! talisman and glove set-limit bytes zero.  This module keeps the raw query
//! cells (including bytes, SQL `NULL`, and source errors), performs either a
//! strict or an explicitly named legacy conversion, and returns the typed
//! records in source order.  It does not execute SQL, own a connection, sort
//! rows, or deduplicate records.
//!
//! The strict decoder is the default.  It accepts raw non-UTF-8 apply bytes as
//! long as they form a bounded C-style value without an interior NUL.  The
//! legacy decoder models the source's `strlcpy` and `str_to_number` calls.  In
//! particular, SQL `NULL` for `apply` has no defined behavior in the legacy
//! C++ call (it passes a null `MYSQL_ROW` cell to `strlcpy`).  This module
//! chooses a documented safe compatibility policy: the legacy decoder maps
//! that value to a zero-filled, empty apply array.  Callers that need to
//! reject undefined input should use the strict decoder.

use std::error::Error;
use std::fmt;

use crate::records::{ItemAttrRecord, APPLY_NAME_MAX_LEN, ITEM_ATTR_RECORD_WIRE_SIZE};

/// Number of columns selected by the fixed normal item-attribute query.
pub const ITEM_ATTR_TABLE_QUERY_COLUMNS: usize = 18;

/// Number of columns selected by the fixed rare item-attribute query.
pub const ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS: usize = 16;

/// Alias for [`ITEM_ATTR_TABLE_QUERY_COLUMNS`].
pub const ITEM_ATTR_QUERY_COLUMNS: usize = ITEM_ATTR_TABLE_QUERY_COLUMNS;

/// Alias for [`ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS`].
pub const ITEM_ATTR_RARE_QUERY_COLUMNS: usize = ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS;

/// Alias using the source table's normal-table spelling.
pub const ITEM_ATTR_NORMAL_QUERY_COLUMNS: usize = ITEM_ATTR_TABLE_QUERY_COLUMNS;

/// Alias using the source table's rare-table spelling.
pub const ITEM_ATTR_RARE_TABLE_COLUMNS: usize = ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS;

/// Number of content bytes in the legacy C `szApply` field.
pub const ITEM_ATTR_APPLY_CONTENT_MAX: usize = APPLY_NAME_MAX_LEN;

/// Complete byte width of the C `szApply` field, including its terminator.
pub const ITEM_ATTR_APPLY_FIELD_BYTES: usize = APPLY_NAME_MAX_LEN + 1;

/// Exact packed 71-byte x86 `TItemAttrTable` wire width.
pub const ITEM_ATTR_TABLE_WIRE_SIZE: usize = ITEM_ATTR_RECORD_WIRE_SIZE;

/// Default record cap: the legacy boot stream counted records in a `WORD`,
/// so no legacy table held more.
pub const ITEM_ATTR_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Alias for [`ITEM_ATTR_TABLE_MAX_RECORDS`].
pub const ITEM_ATTR_MAX_RECORDS: usize = ITEM_ATTR_TABLE_MAX_RECORDS;

/// Exact normal query column names, in source order.
pub const ITEM_ATTR_TABLE_QUERY_COLUMN_NAMES: [&str; ITEM_ATTR_TABLE_QUERY_COLUMNS] = [
    "apply", "apply+0", "prob", "lv1", "lv2", "lv3", "lv4", "lv5", "weapon", "body", "wrist",
    "foots", "neck", "head", "shield", "ear", "talisman", "glove",
];

/// Exact rare query column names, in source order.
pub const ITEM_ATTR_RARE_TABLE_QUERY_COLUMN_NAMES: [&str; ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS] = [
    "apply", "apply+0", "prob", "lv1", "lv2", "lv3", "lv4", "lv5", "weapon", "body", "wrist",
    "foots", "neck", "head", "shield", "ear",
];

/// Alias for [`ITEM_ATTR_TABLE_QUERY_COLUMN_NAMES`].
pub const ITEM_ATTR_NORMAL_QUERY_COLUMN_NAMES: [&str; ITEM_ATTR_TABLE_QUERY_COLUMNS] =
    ITEM_ATTR_TABLE_QUERY_COLUMN_NAMES;

/// Alias for [`ITEM_ATTR_RARE_TABLE_QUERY_COLUMN_NAMES`].
pub const ITEM_ATTR_RARE_QUERY_COLUMN_NAMES: [&str; ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS] =
    ITEM_ATTR_RARE_TABLE_QUERY_COLUMN_NAMES;

/// The legacy normal SQL statement, kept to document the column order.  The
/// `%s` was the legacy `TABLE_POSTFIX`; the rewrite never runs this text.
pub const ITEM_ATTR_QUERY_SQL: &str =
    "SELECT apply, apply+0, prob, lv1, lv2, lv3, lv4, lv5, weapon, body, wrist, foots, neck, head, shield, ear, talisman, glove FROM item_attr%s ORDER BY apply";

/// The legacy rare SQL statement, kept to document the column order.
pub const ITEM_ATTR_RARE_QUERY_SQL: &str =
    "SELECT apply, apply+0, prob, lv1, lv2, lv3, lv4, lv5, weapon, body, wrist, foots, neck, head, shield, ear FROM item_attr_rare%s ORDER BY apply";

/// Selects which fixed legacy item-attribute table is being converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemAttrTableKind {
    /// The normal `item_attr%s` table.
    Normal,
    /// The rare `item_attr_rare%s` table.
    Rare,
}

impl ItemAttrTableKind {
    /// Return a stable short name for diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Rare => "rare",
        }
    }

    /// Return the legacy source table name.
    #[must_use]
    pub const fn table_name(self) -> &'static str {
        match self {
            Self::Normal => "item_attr",
            Self::Rare => "item_attr_rare",
        }
    }

    /// Return the exact number of query columns for this table.
    #[must_use]
    pub const fn column_count(self) -> usize {
        match self {
            Self::Normal => ITEM_ATTR_TABLE_QUERY_COLUMNS,
            Self::Rare => ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS,
        }
    }

    /// Return the exact query column count (spelling used by row adapters).
    #[must_use]
    pub const fn query_column_count(self) -> usize {
        self.column_count()
    }

    /// Return the exact query column names in source order.
    #[must_use]
    pub const fn column_names(self) -> &'static [&'static str] {
        match self {
            Self::Normal => &ITEM_ATTR_TABLE_QUERY_COLUMN_NAMES,
            Self::Rare => &ITEM_ATTR_RARE_TABLE_QUERY_COLUMN_NAMES,
        }
    }

    /// Return the exact query column names (spelling used by query adapters).
    #[must_use]
    pub const fn query_column_names(self) -> &'static [&'static str] {
        self.column_names()
    }

    /// Whether this is the normal table.
    #[must_use]
    pub const fn is_normal(self) -> bool {
        matches!(self, Self::Normal)
    }

    /// Whether this is the rare table.
    #[must_use]
    pub const fn is_rare(self) -> bool {
        matches!(self, Self::Rare)
    }

    /// Number of set-limit cells present in the source query.
    #[must_use]
    pub const fn set_column_count(self) -> usize {
        match self {
            Self::Normal => 10,
            Self::Rare => 8,
        }
    }
}

/// Short alias for [`ItemAttrTableKind`].
pub type ItemAttrKind = ItemAttrTableKind;

/// One raw value from an item-attribute query row.
///
/// `Bytes` is the lossless boundary used by a `SQLx` adapter.  `Text` is a
/// convenience for callers that already have UTF-8 SQL text.  Neither variant
/// is normalized before the selected decoder runs.  `Null` and `Error` remain
/// distinct source outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemAttrQueryValue {
    /// A non-NULL text value supplied by a caller.
    Text(String),
    /// A non-NULL raw byte value supplied by a database adapter.
    Bytes(Vec<u8>),
    /// SQL `NULL`.
    Null,
    /// A source extraction or conversion error.
    Error(String),
}

impl ItemAttrQueryValue {
    /// Construct a text cell.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    /// Construct a raw-byte cell without UTF-8 conversion.
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

    /// Borrow the raw bytes of a text or byte cell.
    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Text(value) => Some(value.as_bytes()),
            Self::Bytes(value) => Some(value.as_slice()),
            Self::Null | Self::Error(_) => None,
        }
    }
}

impl From<String> for ItemAttrQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for ItemAttrQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for ItemAttrQueryValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for ItemAttrQueryValue {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl<const N: usize> From<[u8; N]> for ItemAttrQueryValue {
    fn from(value: [u8; N]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl From<Option<Vec<u8>>> for ItemAttrQueryValue {
    fn from(value: Option<Vec<u8>>) -> Self {
        value.map_or(Self::Null, Self::Bytes)
    }
}

impl From<Option<String>> for ItemAttrQueryValue {
    fn from(value: Option<String>) -> Self {
        value.map_or(Self::Null, Self::Text)
    }
}

/// One source-shaped item-attribute query row.
///
/// The row retains all supplied values and their order.  Its width is checked
/// against the selected [`ItemAttrTableKind`] by the decoder, so a malformed
/// source cannot cause an out-of-bounds index.  Use [`Self::try_new_for_kind`]
/// when a row should be checked at construction time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemAttrTableQueryRow {
    columns: Vec<ItemAttrQueryValue>,
}

impl ItemAttrTableQueryRow {
    /// Construct a row while retaining any supplied width.
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ItemAttrQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub fn from_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ItemAttrQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct and validate that a row has one of the two fixed widths.
    ///
    /// This convenience accepts either normal (18) or rare (16) cells.  Use
    /// [`Self::try_new_for_kind`] when the source kind is known and an exact
    /// kind-specific mismatch must be reported.
    ///
    /// # Errors
    ///
    /// Returns [`ItemAttrRowError::ColumnCount`] for any other width.
    pub fn try_new<I>(columns: I) -> Result<Self, ItemAttrRowError>
    where
        I: IntoIterator<Item = ItemAttrQueryValue>,
    {
        let row = Self::new(columns);
        if row.columns.len() != ITEM_ATTR_TABLE_QUERY_COLUMNS
            && row.columns.len() != ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS
        {
            return Err(ItemAttrRowError::ColumnCount {
                expected: ITEM_ATTR_TABLE_QUERY_COLUMNS,
                actual: row.columns.len(),
            });
        }
        Ok(row)
    }

    /// Construct and validate a row against a named table kind.
    ///
    /// # Errors
    ///
    /// Returns [`ItemAttrRowError::ColumnCount`] when the iterator does not
    /// produce exactly the selected kind's fixed column count.
    pub fn try_new_for_kind<I>(
        kind: ItemAttrTableKind,
        columns: I,
    ) -> Result<Self, ItemAttrRowError>
    where
        I: IntoIterator<Item = ItemAttrQueryValue>,
    {
        let row = Self::new(columns);
        if row.columns.len() != kind.column_count() {
            return Err(ItemAttrRowError::ColumnCount {
                expected: kind.column_count(),
                actual: row.columns.len(),
            });
        }
        Ok(row)
    }

    /// Construct a row from statically sized columns.  The generic form keeps
    /// both normal and rare callers concise; [`Self::try_new_for_kind`] can
    /// validate its width.
    #[must_use]
    pub fn from_typed_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ItemAttrQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct a statically sized normal row.
    #[must_use]
    pub fn from_normal_typed_columns(
        columns: [ItemAttrQueryValue; ITEM_ATTR_TABLE_QUERY_COLUMNS],
    ) -> Self {
        Self::new(columns)
    }

    /// Construct a statically sized rare row.
    #[must_use]
    pub fn from_rare_typed_columns(
        columns: [ItemAttrQueryValue; ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS],
    ) -> Self {
        Self::new(columns)
    }

    /// Borrow all supplied columns in query order.
    #[must_use]
    pub fn columns(&self) -> &[ItemAttrQueryValue] {
        &self.columns
    }

    /// Consume the row and return its columns in query order.
    #[must_use]
    pub fn into_columns(self) -> Vec<ItemAttrQueryValue> {
        self.columns
    }

    /// Return the number of columns supplied by the source.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

/// Short aliases for query adapters.
pub type ItemAttrQueryRow = ItemAttrTableQueryRow;
/// Short alias for a single item-attribute query cell.
pub type ItemAttrQueryCell = ItemAttrQueryValue;
/// Alias for [`ItemAttrTableQueryRow`].
pub type ItemAttrTableRow = ItemAttrTableQueryRow;

/// Return a stable diagnostic name for a query column.
#[must_use]
pub const fn item_attr_query_column_name(index: usize) -> &'static str {
    match index {
        0 => "apply",
        1 => "apply+0",
        2 => "prob",
        3 => "lv1",
        4 => "lv2",
        5 => "lv3",
        6 => "lv4",
        7 => "lv5",
        8 => "weapon",
        9 => "body",
        10 => "wrist",
        11 => "foots",
        12 => "neck",
        13 => "head",
        14 => "shield",
        15 => "ear",
        16 => "talisman",
        17 => "glove",
        _ => "unknown",
    }
}

/// An error while decoding one item-attribute query row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemAttrRowError {
    /// The row did not contain the selected kind's exact column count.
    ColumnCount {
        /// Required column count.
        expected: usize,
        /// Supplied column count.
        actual: usize,
    },
    /// A required cell was SQL `NULL` in the strict decoder.
    Null {
        /// Zero-based query column index.
        column: usize,
    },
    /// A cell could not be obtained or converted by the source.
    Source {
        /// Zero-based query column index.
        column: usize,
        /// Source diagnostic.
        message: String,
    },
    /// A non-NULL cell was not a strict decimal integer.
    InvalidNumber {
        /// Zero-based query column index.
        column: usize,
        /// Diagnostic representation of the original value.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A syntactically valid integer did not fit its target type.
    NumberOverflow {
        /// Zero-based query column index.
        column: usize,
        /// Diagnostic representation of the original value.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// The apply bytes contain a NUL followed by non-NUL content.
    ApplyInteriorNul {
        /// Zero-based query column index (always zero for `apply`).
        column: usize,
        /// Byte offset of the first non-NUL byte after the terminator.
        index: usize,
    },
    /// The apply value cannot fit the source's 32-byte content field.
    ApplyTooLong {
        /// Zero-based query column index (always zero for `apply`).
        column: usize,
        /// Supplied byte length.
        length: usize,
        /// Maximum complete source-field length.
        maximum: usize,
    },
}

impl fmt::Display for ItemAttrRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => write!(
                formatter,
                "item-attribute query row has {actual} columns; expected {expected}"
            ),
            Self::Null { column } => write!(
                formatter,
                "item-attribute query column {} is NULL",
                item_attr_query_column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "item-attribute query column {} could not be read: {message}",
                item_attr_query_column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "item-attribute query column {} value {value:?} is not a strict {target}",
                item_attr_query_column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "item-attribute query column {} value {value:?} overflows {target}",
                item_attr_query_column_name(*column)
            ),
            Self::ApplyInteriorNul { column, index } => write!(
                formatter,
                "item-attribute query column {} has non-NUL content at byte {index} after NUL",
                item_attr_query_column_name(*column)
            ),
            Self::ApplyTooLong {
                column,
                length,
                maximum,
            } => write!(
                formatter,
                "item-attribute query column {} is {length} bytes; maximum field length is {maximum}",
                item_attr_query_column_name(*column)
            ),
        }
    }
}

impl Error for ItemAttrRowError {}

/// Alias emphasizing that this is a row decoding error.
pub type ItemAttrRowDecodeError = ItemAttrRowError;

/// The record cap checked before a item attribute table is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemAttrLimits {
    /// Maximum number of source rows accepted.
    pub max_records: usize,
}

impl ItemAttrLimits {
    /// Limits with a caller-selected row cap.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self { max_records }
    }
}

impl Default for ItemAttrLimits {
    fn default() -> Self {
        Self::new(ITEM_ATTR_TABLE_MAX_RECORDS)
    }
}

/// A checked failure while building the item attribute table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemAttrTableError {
    /// The source held no rows. The legacy loader refuses to boot without
    /// either attribute table (`ClientManagerBoot.cpp:783-787`, `857-861`).
    EmptySource {
        /// The table that was empty.
        kind: ItemAttrTableKind,
    },
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
        source: ItemAttrRowError,
    },
}

impl fmt::Display for ItemAttrTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySource { kind } => {
                write!(formatter, "item attribute {} table has no rows", kind.as_str())
            }
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "item attribute table has {count} rows; configured limit is {maximum}"
            ),
            Self::AllocationFailed { requested } => {
                write!(formatter, "item attribute allocation of {requested} records failed")
            }
            Self::Row { index, source } => {
                write!(formatter, "item attribute row {index} is invalid: {source}")
            }
        }
    }
}

impl Error for ItemAttrTableError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn cell_bytes(value: &ItemAttrQueryValue, column: usize) -> Result<&[u8], ItemAttrRowError> {
    match value {
        ItemAttrQueryValue::Text(text) => Ok(text.as_bytes()),
        ItemAttrQueryValue::Bytes(bytes) => Ok(bytes.as_slice()),
        ItemAttrQueryValue::Null => Err(ItemAttrRowError::Null { column }),
        ItemAttrQueryValue::Error(message) => Err(ItemAttrRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn diagnostic_value(value: &ItemAttrQueryValue) -> String {
    match value {
        ItemAttrQueryValue::Text(text) => text.clone(),
        ItemAttrQueryValue::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        ItemAttrQueryValue::Null => "<NULL>".to_owned(),
        ItemAttrQueryValue::Error(message) => message.clone(),
    }
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

fn decode_strict_u32(value: &ItemAttrQueryValue, column: usize) -> Result<u32, ItemAttrRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_unsigned_decimal(bytes) {
        return Err(ItemAttrRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "u32",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ItemAttrRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "u32",
    })?;
    text.parse::<u32>()
        .map_err(|_| ItemAttrRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "u32",
        })
}

fn decode_strict_i32(value: &ItemAttrQueryValue, column: usize) -> Result<i32, ItemAttrRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_signed_decimal(bytes) {
        return Err(ItemAttrRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "i32",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ItemAttrRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "i32",
    })?;
    text.parse::<i32>()
        .map_err(|_| ItemAttrRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "i32",
        })
}

fn decode_strict_u8(value: &ItemAttrQueryValue, column: usize) -> Result<u8, ItemAttrRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_unsigned_decimal(bytes) {
        return Err(ItemAttrRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "u8",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ItemAttrRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "u8",
    })?;
    text.parse::<u8>()
        .map_err(|_| ItemAttrRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "u8",
        })
}

/// Parse a C `strtol`/`strtoul`-style numeric prefix from raw bytes.
/// The magnitude saturates at `u64::MAX`; the destination-specific helpers
/// below then apply the active x86 32-bit conversion limits.
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

fn legacy_i32(value: &ItemAttrQueryValue, column: usize) -> Result<i32, ItemAttrRowError> {
    let bytes = cell_bytes_or_zero(value, column)?;
    let (negative, magnitude) = legacy_numeric_prefix(bytes);
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

fn cell_bytes_or_zero(
    value: &ItemAttrQueryValue,
    column: usize,
) -> Result<&[u8], ItemAttrRowError> {
    match value {
        ItemAttrQueryValue::Null => Ok(&[]),
        other => cell_bytes(other, column),
    }
}

fn legacy_u32(value: &ItemAttrQueryValue, column: usize) -> Result<u32, ItemAttrRowError> {
    let bytes = cell_bytes_or_zero(value, column)?;
    let (negative, magnitude) = legacy_numeric_prefix(bytes);
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

fn legacy_u8(value: &ItemAttrQueryValue, column: usize) -> Result<u8, ItemAttrRowError> {
    let value = legacy_u32(value, column)?;
    Ok((value & 0xff) as u8)
}

fn decode_apply_strict(
    value: &ItemAttrQueryValue,
    column: usize,
) -> Result<[u8; ITEM_ATTR_APPLY_FIELD_BYTES], ItemAttrRowError> {
    let bytes = cell_bytes(value, column)?;
    if bytes.len() > ITEM_ATTR_APPLY_FIELD_BYTES {
        return Err(ItemAttrRowError::ApplyTooLong {
            column,
            length: bytes.len(),
            maximum: ITEM_ATTR_APPLY_FIELD_BYTES,
        });
    }
    if let Some(first_nul) = bytes.iter().position(|byte| *byte == 0) {
        if first_nul + 1 < bytes.len() {
            return Err(ItemAttrRowError::ApplyInteriorNul {
                column,
                index: first_nul + 1,
            });
        }
    } else if bytes.len() > ITEM_ATTR_APPLY_CONTENT_MAX {
        return Err(ItemAttrRowError::ApplyTooLong {
            column,
            length: bytes.len(),
            maximum: ITEM_ATTR_APPLY_FIELD_BYTES,
        });
    }

    let mut apply = [0_u8; ITEM_ATTR_APPLY_FIELD_BYTES];
    apply[..bytes.len()].copy_from_slice(bytes);
    Ok(apply)
}

/// Decode the apply cell using the explicitly named legacy `strlcpy` policy.
///
/// SQL `NULL` is mapped to an empty array as a safe, documented divergence:
/// the source passes that null pointer to `strlcpy` and has no defined result.
/// A first NUL terminates the copied content, content longer than 32 bytes is
/// truncated, and all bytes not occupied by the copied content are zeroed.
fn decode_apply_legacy(
    value: &ItemAttrQueryValue,
    column: usize,
) -> Result<[u8; ITEM_ATTR_APPLY_FIELD_BYTES], ItemAttrRowError> {
    let bytes = match value {
        ItemAttrQueryValue::Null => &[][..],
        ItemAttrQueryValue::Text(text) => text.as_bytes(),
        ItemAttrQueryValue::Bytes(bytes) => bytes.as_slice(),
        ItemAttrQueryValue::Error(message) => {
            return Err(ItemAttrRowError::Source {
                column,
                message: message.clone(),
            })
        }
    };
    let content_end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let copy_len = content_end.min(ITEM_ATTR_APPLY_CONTENT_MAX);
    let mut apply = [0_u8; ITEM_ATTR_APPLY_FIELD_BYTES];
    apply[..copy_len].copy_from_slice(&bytes[..copy_len]);
    Ok(apply)
}

fn check_row_width(
    kind: ItemAttrTableKind,
    row: &ItemAttrTableQueryRow,
) -> Result<(), ItemAttrRowError> {
    if row.columns.len() == kind.column_count() {
        Ok(())
    } else {
        Err(ItemAttrRowError::ColumnCount {
            expected: kind.column_count(),
            actual: row.columns.len(),
        })
    }
}

/// Strictly decode one item-attribute query row.
///
/// `NULL`, source errors, wrong-width rows, malformed decimal text, numeric
/// overflow, and unsafe apply bytes remain errors.  Raw non-UTF-8 apply bytes
/// are accepted when they are a bounded C-style value without an interior
/// NUL; numeric cells remain ASCII decimal only.
///
/// # Errors
///
/// Returns [`ItemAttrRowError`] for a wrong-width row or an invalid cell.
pub fn decode_item_attr_query_row(
    kind: ItemAttrTableKind,
    row: &ItemAttrTableQueryRow,
) -> Result<ItemAttrRecord, ItemAttrRowError> {
    check_row_width(kind, row)?;
    let apply = decode_apply_strict(&row.columns[0], 0)?;
    let apply_index = decode_strict_u32(&row.columns[1], 1)?;
    let prob = decode_strict_u32(&row.columns[2], 2)?;
    let mut values = [0_i32; 5];
    for (index, value) in values.iter_mut().enumerate() {
        *value = decode_strict_i32(&row.columns[3 + index], 3 + index)?;
    }
    let mut max_level_by_set = [0_u8; 10];
    for (index, value) in max_level_by_set
        .iter_mut()
        .enumerate()
        .take(kind.set_column_count())
    {
        *value = decode_strict_u8(&row.columns[8 + index], 8 + index)?;
    }
    Ok(ItemAttrRecord {
        apply,
        apply_index,
        prob,
        values,
        max_level_by_set,
    })
}

/// Alias emphasizing the table-row spelling.
///
/// # Errors
///
/// Returns [`ItemAttrRowError`] for a wrong-width row or invalid source cell.
pub fn decode_item_attr_table_row(
    kind: ItemAttrTableKind,
    row: &ItemAttrTableQueryRow,
) -> Result<ItemAttrRecord, ItemAttrRowError> {
    decode_item_attr_query_row(kind, row)
}

/// Alias for callers that use the shorter row spelling.
///
/// # Errors
///
/// Returns [`ItemAttrRowError`] for a wrong-width row or invalid source cell.
pub fn decode_item_attr_row(
    kind: ItemAttrTableKind,
    row: &ItemAttrTableQueryRow,
) -> Result<ItemAttrRecord, ItemAttrRowError> {
    decode_item_attr_query_row(kind, row)
}

/// Decode one row using the explicitly named legacy conversion policy.
///
/// Numeric cells model the source's `str_to_number` calls: leading C
/// whitespace and a sign are accepted, only a numeric prefix is consumed, and
/// the active x86 destination casts are applied.  Empty, non-numeric, and SQL
/// `NULL` numeric cells leave the zero-initialized destination at zero.  The
/// apply cell uses the documented safe zero-empty policy for SQL `NULL` and
/// the legacy `strlcpy` truncation/termination behavior otherwise.
///
/// # Errors
///
/// Returns [`ItemAttrRowError::ColumnCount`] for a wrong-width row and
/// [`ItemAttrRowError::Source`] for a cell-extraction error. Numeric text
/// itself does not produce a syntax error in this compatibility policy.
pub fn decode_item_attr_query_row_legacy(
    kind: ItemAttrTableKind,
    row: &ItemAttrTableQueryRow,
) -> Result<ItemAttrRecord, ItemAttrRowError> {
    check_row_width(kind, row)?;
    let apply = decode_apply_legacy(&row.columns[0], 0)?;
    let apply_index = legacy_u32(&row.columns[1], 1)?;
    let prob = legacy_u32(&row.columns[2], 2)?;
    let mut values = [0_i32; 5];
    for (index, value) in values.iter_mut().enumerate() {
        *value = legacy_i32(&row.columns[3 + index], 3 + index)?;
    }
    let mut max_level_by_set = [0_u8; 10];
    for (index, value) in max_level_by_set
        .iter_mut()
        .enumerate()
        .take(kind.set_column_count())
    {
        *value = legacy_u8(&row.columns[8 + index], 8 + index)?;
    }
    Ok(ItemAttrRecord {
        apply,
        apply_index,
        prob,
        values,
        max_level_by_set,
    })
}

/// Legacy alias emphasizing the table-row spelling.
///
/// # Errors
///
/// Returns [`ItemAttrRowError`] for a wrong-width row or source-cell error.
pub fn decode_item_attr_table_row_legacy(
    kind: ItemAttrTableKind,
    row: &ItemAttrTableQueryRow,
) -> Result<ItemAttrRecord, ItemAttrRowError> {
    decode_item_attr_query_row_legacy(kind, row)
}

/// Legacy alias for callers that use the shorter row spelling.
///
/// # Errors
///
/// Returns [`ItemAttrRowError`] for a wrong-width row or source-cell error.
pub fn decode_item_attr_row_legacy(
    kind: ItemAttrTableKind,
    row: &ItemAttrTableQueryRow,
) -> Result<ItemAttrRecord, ItemAttrRowError> {
    decode_item_attr_query_row_legacy(kind, row)
}

type RowDecoder =
    fn(ItemAttrTableKind, &ItemAttrTableQueryRow) -> Result<ItemAttrRecord, ItemAttrRowError>;

fn build_records(
    kind: ItemAttrTableKind,
    rows: &[ItemAttrTableQueryRow],
    limits: ItemAttrLimits,
    decode: RowDecoder,
) -> Result<Vec<ItemAttrRecord>, ItemAttrTableError> {
    if rows.is_empty() {
        return Err(ItemAttrTableError::EmptySource { kind });
    }
    if rows.len() > limits.max_records {
        return Err(ItemAttrTableError::TooManyRecords {
            count: rows.len(),
            maximum: limits.max_records,
        });
    }
    let mut records = Vec::new();
    records
        .try_reserve_exact(rows.len())
        .map_err(|_| ItemAttrTableError::AllocationFailed {
            requested: rows.len(),
        })?;
    for (index, row) in rows.iter().enumerate() {
        let record = decode(kind, row).map_err(|source| ItemAttrTableError::Row { index, source })?;
        records.push(record);
    }
    Ok(records)
}

/// Strictly decode `kind` rows into item-attribute records, preserving
/// source row order and duplicates.
///
/// An empty source is refused, as the legacy loader refused it. The row cap
/// is checked before the output is reserved.
///
/// # Errors
///
/// Returns [`ItemAttrTableError`] for an empty source, an invalid row, the
/// row cap, or an allocation failure.
pub fn build_item_attr_table(
    kind: ItemAttrTableKind,
    rows: &[ItemAttrTableQueryRow],
    limits: ItemAttrLimits,
) -> Result<Vec<ItemAttrRecord>, ItemAttrTableError> {
    build_records(kind, rows, limits, decode_item_attr_query_row)
}

/// Decode `kind` rows with the legacy conversion policy, preserving source
/// row order and duplicates.
///
/// # Errors
///
/// Returns [`ItemAttrTableError`] for an empty source, a row the legacy
/// policy still rejects, the row cap, or an allocation failure.
pub fn build_item_attr_table_legacy(
    kind: ItemAttrTableKind,
    rows: &[ItemAttrTableQueryRow],
    limits: ItemAttrLimits,
) -> Result<Vec<ItemAttrRecord>, ItemAttrTableError> {
    build_records(kind, rows, limits, decode_item_attr_query_row_legacy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(value: &[u8]) -> ItemAttrQueryValue {
        ItemAttrQueryValue::bytes(value.to_vec())
    }

    fn normal_row() -> ItemAttrTableQueryRow {
        ItemAttrTableQueryRow::from_normal_typed_columns([
            bytes(b"ATTR"),
            bytes(b"7"),
            bytes(b"91"),
            bytes(b"1"),
            bytes(b"-2"),
            bytes(b"3"),
            bytes(b"-4"),
            bytes(b"5"),
            bytes(b"6"),
            bytes(b"7"),
            bytes(b"8"),
            bytes(b"9"),
            bytes(b"10"),
            bytes(b"11"),
            bytes(b"12"),
            bytes(b"13"),
            bytes(b"14"),
            bytes(b"15"),
        ])
    }

    fn rare_row() -> ItemAttrTableQueryRow {
        ItemAttrTableQueryRow::from_rare_typed_columns([
            bytes(b"RARE"),
            bytes(b"8"),
            bytes(b"92"),
            bytes(b"6"),
            bytes(b"5"),
            bytes(b"4"),
            bytes(b"3"),
            bytes(b"2"),
            bytes(b"1"),
            bytes(b"2"),
            bytes(b"3"),
            bytes(b"4"),
            bytes(b"5"),
            bytes(b"6"),
            bytes(b"7"),
            bytes(b"8"),
        ])
    }

    #[test]
    fn query_shapes_names_and_wire_width_are_exact() {
        assert_eq!(ItemAttrTableKind::Normal.column_count(), 18);
        assert_eq!(ItemAttrTableKind::Rare.column_count(), 16);
        assert_eq!(
            ItemAttrTableKind::Normal.column_names(),
            ITEM_ATTR_TABLE_QUERY_COLUMN_NAMES.as_slice()
        );
        assert_eq!(
            ItemAttrTableKind::Rare.column_names(),
            ITEM_ATTR_RARE_TABLE_QUERY_COLUMN_NAMES.as_slice()
        );
        assert_eq!(ItemAttrTableKind::Normal.table_name(), "item_attr");
        assert_eq!(ItemAttrTableKind::Rare.table_name(), "item_attr_rare");
        assert_eq!(ITEM_ATTR_TABLE_WIRE_SIZE, 71);
        assert!(ITEM_ATTR_QUERY_SQL.ends_with("item_attr%s ORDER BY apply"));
        assert!(ITEM_ATTR_RARE_QUERY_SQL.ends_with("item_attr_rare%s ORDER BY apply"));
        assert_eq!(
            ITEM_ATTR_QUERY_SQL.matches(',').count() + 1,
            ITEM_ATTR_TABLE_QUERY_COLUMNS
        );
        assert_eq!(
            ITEM_ATTR_RARE_QUERY_SQL.matches(',').count() + 1,
            ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS
        );
    }

    #[test]
    fn strict_normal_row_maps_every_field_and_packs_71_bytes() {
        let record = decode_item_attr_query_row(ItemAttrTableKind::Normal, &normal_row()).unwrap();
        assert_eq!(&record.apply[..4], b"ATTR");
        assert!(record.apply[4..].iter().all(|byte| *byte == 0));
        assert_eq!(record.apply_index, 7);
        assert_eq!(record.prob, 91);
        assert_eq!(record.values, [1, -2, 3, -4, 5]);
        assert_eq!(
            record.max_level_by_set,
            [6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        );

        let table = build_item_attr_table(
            ItemAttrTableKind::Normal,
            &[normal_row()],
            ItemAttrLimits::default(),
        )
        .unwrap();
        assert_eq!(table, [record]);
        let wire = record.encode();
        assert_eq!(wire.len(), 71);
        assert_eq!(&wire[0..4], b"ATTR");
        assert_eq!(&wire[33..37], &7_u32.to_le_bytes());
        assert_eq!(&wire[37..41], &91_u32.to_le_bytes());
        assert_eq!(&wire[41..45], &1_i32.to_le_bytes());
        assert_eq!(&wire[61..71], &[6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        assert_eq!(ItemAttrRecord::decode(&wire).unwrap(), record);
    }

    #[test]
    fn rare_uses_the_shared_record_and_zeroes_missing_set_slots() {
        let record = decode_item_attr_query_row(ItemAttrTableKind::Rare, &rare_row()).unwrap();
        assert_eq!(record.values, [6, 5, 4, 3, 2]);
        assert_eq!(&record.max_level_by_set[..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&record.max_level_by_set[8..], &[0, 0]);

        let table = build_item_attr_table(
            ItemAttrTableKind::Rare,
            &[rare_row()],
            ItemAttrLimits::default(),
        )
        .unwrap();
        assert_eq!(table, [record]);
        assert_eq!(&record.encode()[69..71], &[0, 0]);
    }

    #[test]
    fn strict_rejects_null_errors_bad_decimals_overflow_and_unsafe_apply() {
        let mut values = normal_row().into_columns();
        values[1] = ItemAttrQueryValue::Null;
        assert!(matches!(
            decode_item_attr_query_row(
                ItemAttrTableKind::Normal,
                &ItemAttrTableQueryRow::new(values)
            ),
            Err(ItemAttrRowError::Null { column: 1 })
        ));

        let mut values = normal_row().into_columns();
        values[1] = ItemAttrQueryValue::error("driver failed");
        assert!(matches!(
            decode_item_attr_query_row(
                ItemAttrTableKind::Normal,
                &ItemAttrTableQueryRow::new(values)
            ),
            Err(ItemAttrRowError::Source { column: 1, .. })
        ));

        let mut values = normal_row().into_columns();
        values[1] = bytes(b"7x");
        assert!(matches!(
            decode_item_attr_query_row(
                ItemAttrTableKind::Normal,
                &ItemAttrTableQueryRow::new(values)
            ),
            Err(ItemAttrRowError::InvalidNumber { column: 1, .. })
        ));

        let mut values = normal_row().into_columns();
        values[1] = bytes(b"4294967296");
        assert!(matches!(
            decode_item_attr_query_row(
                ItemAttrTableKind::Normal,
                &ItemAttrTableQueryRow::new(values)
            ),
            Err(ItemAttrRowError::NumberOverflow {
                column: 1,
                target: "u32",
                ..
            })
        ));

        let mut values = normal_row().into_columns();
        values[3] = bytes(b"2147483648");
        assert!(matches!(
            decode_item_attr_query_row(
                ItemAttrTableKind::Normal,
                &ItemAttrTableQueryRow::new(values)
            ),
            Err(ItemAttrRowError::NumberOverflow {
                column: 3,
                target: "i32",
                ..
            })
        ));

        let mut values = normal_row().into_columns();
        values[8] = bytes(b"256");
        assert!(matches!(
            decode_item_attr_query_row(
                ItemAttrTableKind::Normal,
                &ItemAttrTableQueryRow::new(values)
            ),
            Err(ItemAttrRowError::NumberOverflow {
                column: 8,
                target: "u8",
                ..
            })
        ));

        let mut values = normal_row().into_columns();
        values[0] = bytes(b"a\0b");
        assert!(matches!(
            decode_item_attr_query_row(
                ItemAttrTableKind::Normal,
                &ItemAttrTableQueryRow::new(values)
            ),
            Err(ItemAttrRowError::ApplyInteriorNul {
                column: 0,
                index: 2
            })
        ));

        let mut values = normal_row().into_columns();
        values[0] = bytes(&[b'a'; ITEM_ATTR_APPLY_FIELD_BYTES]);
        assert!(matches!(
            decode_item_attr_query_row(
                ItemAttrTableKind::Normal,
                &ItemAttrTableQueryRow::new(values)
            ),
            Err(ItemAttrRowError::ApplyTooLong {
                column: 0,
                length: 33,
                maximum: 33
            })
        ));
    }

    #[test]
    fn strict_accepts_raw_non_utf8_apply_and_preserves_terminal_bytes() {
        let mut values = normal_row().into_columns();
        values[0] = bytes(&[0xff, b'A', 0]);
        let record = decode_item_attr_query_row(
            ItemAttrTableKind::Normal,
            &ItemAttrTableQueryRow::new(values),
        )
        .unwrap();
        assert_eq!(&record.apply[..3], &[0xff, b'A', 0]);

        let mut values = normal_row().into_columns();
        values[0] = bytes(&[b'a'; ITEM_ATTR_APPLY_CONTENT_MAX]);
        let record = decode_item_attr_query_row(
            ItemAttrTableKind::Normal,
            &ItemAttrTableQueryRow::new(values),
        )
        .unwrap();
        assert_eq!(&record.apply[..32], &[b'a'; 32]);
        assert_eq!(record.apply[32], 0);
    }

    #[test]
    fn legacy_numeric_prefixes_casts_and_null_policy_are_explicit() {
        let mut values = normal_row().into_columns();
        values[0] = ItemAttrQueryValue::Null;
        values[1] = bytes(b"  +12suffix");
        values[2] = bytes(b"4294967296");
        values[3] = bytes(b"");
        values[4] = bytes(b"2147483648");
        values[5] = bytes(b"no-digits");
        values[6] = bytes(b"-1");
        values[7] = bytes(b"-2147483649");
        values[8] = bytes(b"256");
        values[9] = bytes(b"-1");
        values[10] = bytes(b"300");
        let record = decode_item_attr_query_row_legacy(
            ItemAttrTableKind::Normal,
            &ItemAttrTableQueryRow::new(values),
        )
        .unwrap();
        assert!(record.apply.iter().all(|byte| *byte == 0));
        assert_eq!(record.apply_index, 12);
        assert_eq!(record.prob, u32::MAX);
        assert_eq!(record.values, [0, i32::MAX, 0, -1, i32::MIN]);
        assert_eq!(&record.max_level_by_set[..3], &[0, u8::MAX, 44]);

        let mut values = rare_row().into_columns();
        values[0] = bytes(b"01234567890123456789012345678901rest\0tail");
        let record = decode_item_attr_query_row_legacy(
            ItemAttrTableKind::Rare,
            &ItemAttrTableQueryRow::new(values),
        )
        .unwrap();
        assert_eq!(&record.apply[..32], b"01234567890123456789012345678901");
        assert!(record.apply[32..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn legacy_preserves_source_order_and_duplicates() {
        let first = normal_row();
        let second = normal_row();
        let mut changed = second.clone();
        changed.columns[2] = bytes(b"44");
        let table = build_item_attr_table_legacy(
            ItemAttrTableKind::Normal,
            &[first.clone(), changed, second, first],
            ItemAttrLimits::default(),
        )
        .unwrap();
        assert_eq!(table.len(), 4);
        assert_eq!(table[0], table[2]);
        assert_eq!(table[0], table[3]);
        assert_eq!(table[1].prob, 44, "source order is kept, not sorted");
    }

    #[test]
    fn the_record_cap_is_checked_before_decoding() {
        let row = normal_row();
        let mut bad = row.clone();
        bad.columns[2] = bytes(b"not a number");
        assert_eq!(
            build_item_attr_table(
                ItemAttrTableKind::Normal,
                &[row.clone(), bad],
                ItemAttrLimits::new(1)
            ),
            Err(ItemAttrTableError::TooManyRecords {
                count: 2,
                maximum: 1
            })
        );
        assert!(build_item_attr_table(
            ItemAttrTableKind::Normal,
            &[row],
            ItemAttrLimits::new(ITEM_ATTR_TABLE_MAX_RECORDS + 1),
        )
        .is_ok());
    }

    #[test]
    fn an_empty_source_is_refused_and_a_bad_row_names_its_index() {
        for kind in [ItemAttrTableKind::Normal, ItemAttrTableKind::Rare] {
            assert_eq!(
                build_item_attr_table(kind, &[], ItemAttrLimits::default()),
                Err(ItemAttrTableError::EmptySource { kind })
            );
            assert_eq!(
                build_item_attr_table_legacy(kind, &[], ItemAttrLimits::default()),
                Err(ItemAttrTableError::EmptySource { kind })
            );
        }

        let mut bad = normal_row();
        bad.columns[2] = ItemAttrQueryValue::Null;
        let error = build_item_attr_table(
            ItemAttrTableKind::Normal,
            &[normal_row(), bad],
            ItemAttrLimits::default(),
        )
        .unwrap_err();
        assert!(
            matches!(error, ItemAttrTableError::Row { index: 1, .. }),
            "got {error:?}"
        );
    }
}
