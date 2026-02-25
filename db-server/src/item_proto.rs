//! SQL-free query and conversion boundary for the active `item_proto` table.
//!
//! The legacy boot loader selects the 34 cells in [`ITEM_PROTO_QUERY_COLUMN_NAMES`].
//! The active build derives `dwVnumRange` instead of selecting a `vnum_range`
//! column. It also derives the two special limit indexes and leaves six socket
//! slots and `bSpecular` at zero. [`decode_item_proto_query_row`] preserves
//! those source defaults while requiring exact raw-cell shapes and whole
//! decimal integers. [`decode_item_proto_query_row_legacy`] is a separately
//! named compatibility policy for the source's C conversion behavior.
//!
//! This module does not execute SQL, choose a boot profile, populate a cache,
//! or call a live boot loader. A caller must acquire rows through an injected
//! source or the sibling `item_proto_sqlx` adapter.

use std::error::Error;
use std::fmt;

use common::enums::{EItemTypes, ELimitTypes};
use protocol::db_boot::{BootSection, BootSectionKind};
use protocol::db_records::{
    ItemApplyRecord, ItemLimitRecord, ItemTableRecord, ITEM_NAME_BYTES, ITEM_NAME_MAX_LEN,
    ITEM_SOCKET_MAX_NUM, ITEM_TABLE_RECORD_WIRE_SIZE,
};

use crate::postfix::{TablePostfix, TablePostfixError};

/// Base table name used by the source-fixed active boot query.
pub const ITEM_PROTO_TABLE: &str = "item_proto";

/// Number of cells selected by the active `item_proto` query.
pub const ITEM_PROTO_QUERY_COLUMN_COUNT: usize = 34;

/// Alias for [`ITEM_PROTO_QUERY_COLUMN_COUNT`].
pub const ITEM_PROTO_TABLE_QUERY_COLUMNS: usize = ITEM_PROTO_QUERY_COLUMN_COUNT;

/// Maximum number of records representable by the legacy `u16` section count.
pub const ITEM_PROTO_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Exact source-fixed packed width of one active `TItemTable` record.
pub const ITEM_PROTO_TABLE_WIRE_SIZE: usize = ITEM_TABLE_RECORD_WIRE_SIZE;

/// Alias for the source-fixed record width.
pub const ITEM_PROTO_SECTION_RECORD_SIZE: u16 = 204;

const _: () = assert!(ITEM_PROTO_SECTION_RECORD_SIZE as usize == ITEM_TABLE_RECORD_WIRE_SIZE);

/// Maximum packed data bytes when all representable records are present.
pub const ITEM_PROTO_TABLE_MAX_SECTION_BYTES: usize =
    ITEM_PROTO_TABLE_WIRE_SIZE * ITEM_PROTO_TABLE_MAX_RECORDS;

/// Maximum statement bytes that fit in the legacy `char[2048]` buffer,
/// excluding its terminating NUL.
pub const MAX_ITEM_PROTO_QUERY_BYTES: usize = 2_047;

/// Maximum validated locale-column identifier bytes.
pub const MAX_ITEM_PROTO_LOCALE_COLUMN_BYTES: usize = 255;

/// Default locale column selected by the active loader.
pub const DEFAULT_ITEM_PROTO_LOCALE_COLUMN: &str = "name";

/// Exact query before the validated locale and postfix values are inserted.
///
/// This is a golden template, not an interpolation API. The two placeholders
/// are documented source-format positions, not arbitrary SQL fragments.
pub const ITEM_PROTO_QUERY_TEMPLATE: &str = "SELECT vnum, type, subtype, name, {locale}, gold, shop_buy_price, weight, size, flag, wearflag, antiflag, immuneflag+0, refined_vnum, refine_set, magic_pct, socket_pct, addon_type, limittype0, limitvalue0, limittype1, limitvalue1, applytype0, applyvalue0, applytype1, applyvalue1, applytype2, applyvalue2, value0, value1, value2, value3, value4, value5 FROM item_proto{postfix} ORDER BY vnum;";

/// Exact query prefix before the validated locale identifier.
pub const ITEM_PROTO_QUERY_PREFIX: &str = "SELECT vnum, type, subtype, name, ";

/// Exact query suffix after the validated locale identifier.
pub const ITEM_PROTO_QUERY_SUFFIX: &str = ", gold, shop_buy_price, weight, size, flag, wearflag, antiflag, immuneflag+0, refined_vnum, refine_set, magic_pct, socket_pct, addon_type, limittype0, limitvalue0, limittype1, limitvalue1, applytype0, applyvalue0, applytype1, applyvalue1, applytype2, applyvalue2, value0, value1, value2, value3, value4, value5 FROM item_proto";

/// Fixed active query cell names in positional order.
pub const ITEM_PROTO_QUERY_COLUMN_NAMES: [&str; ITEM_PROTO_QUERY_COLUMN_COUNT] = [
    "vnum",
    "type",
    "subtype",
    "name",
    "locale_name",
    "gold",
    "shop_buy_price",
    "weight",
    "size",
    "flag",
    "wearflag",
    "antiflag",
    "immuneflag+0",
    "refined_vnum",
    "refine_set",
    "magic_pct",
    "socket_pct",
    "addon_type",
    "limittype0",
    "limitvalue0",
    "limittype1",
    "limitvalue1",
    "applytype0",
    "applyvalue0",
    "applytype1",
    "applyvalue1",
    "applytype2",
    "applyvalue2",
    "value0",
    "value1",
    "value2",
    "value3",
    "value4",
    "value5",
];

/// One lossless value from an `item_proto` query row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemProtoQueryValue {
    /// Non-NULL UTF-8 text supplied by a caller.
    Text(String),
    /// Non-NULL raw bytes supplied by a database adapter.
    Bytes(Vec<u8>),
    /// SQL `NULL`.
    Null,
    /// A source extraction or conversion error.
    Error(String),
}

impl ItemProtoQueryValue {
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

    /// Construct a source-error cell.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error(message.into())
    }

    /// Borrow bytes for text or byte cells.
    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Text(value) => Some(value.as_bytes()),
            Self::Bytes(value) => Some(value.as_slice()),
            Self::Null | Self::Error(_) => None,
        }
    }
}

impl From<String> for ItemProtoQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for ItemProtoQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for ItemProtoQueryValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for ItemProtoQueryValue {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl<const N: usize> From<[u8; N]> for ItemProtoQueryValue {
    fn from(value: [u8; N]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl From<Option<Vec<u8>>> for ItemProtoQueryValue {
    fn from(value: Option<Vec<u8>>) -> Self {
        value.map_or(Self::Null, Self::Bytes)
    }
}

impl From<Option<String>> for ItemProtoQueryValue {
    fn from(value: Option<String>) -> Self {
        value.map_or(Self::Null, Self::Text)
    }
}

/// One source-shaped `item_proto` query row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemProtoQueryRow {
    columns: Vec<ItemProtoQueryValue>,
}

impl ItemProtoQueryRow {
    /// Construct a row while retaining its supplied width.
    #[must_use]
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ItemProtoQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub fn from_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ItemProtoQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct and validate the exact 34-column shape.
    ///
    /// # Errors
    ///
    /// Returns [`ItemProtoRowError::ColumnCount`] for any other width.
    pub fn try_new<I>(columns: I) -> Result<Self, ItemProtoRowError>
    where
        I: IntoIterator<Item = ItemProtoQueryValue>,
    {
        let row = Self::new(columns);
        check_row_width(&row)?;
        Ok(row)
    }

    /// Construct from a statically sized, correctly shaped column set.
    #[must_use]
    pub fn from_typed_columns(
        columns: [ItemProtoQueryValue; ITEM_PROTO_QUERY_COLUMN_COUNT],
    ) -> Self {
        Self::new(columns)
    }

    /// Borrow all cells in query order.
    #[must_use]
    pub fn columns(&self) -> &[ItemProtoQueryValue] {
        &self.columns
    }

    /// Consume the row and return all cells in query order.
    #[must_use]
    pub fn into_columns(self) -> Vec<ItemProtoQueryValue> {
        self.columns
    }

    /// Return the supplied cell count.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

impl From<[ItemProtoQueryValue; ITEM_PROTO_QUERY_COLUMN_COUNT]> for ItemProtoQueryRow {
    fn from(columns: [ItemProtoQueryValue; ITEM_PROTO_QUERY_COLUMN_COUNT]) -> Self {
        Self::from_typed_columns(columns)
    }
}

impl TryFrom<Vec<ItemProtoQueryValue>> for ItemProtoQueryRow {
    type Error = ItemProtoRowError;

    fn try_from(columns: Vec<ItemProtoQueryValue>) -> Result<Self, Self::Error> {
        Self::try_new(columns)
    }
}

/// Alias emphasizing the physical table name.
pub type ItemProtoTableQueryRow = ItemProtoQueryRow;
/// Alias for one `item_proto` query cell.
pub type ItemProtoTableQueryValue = ItemProtoQueryValue;

/// Return a stable diagnostic name for one selected expression.
#[must_use]
pub fn item_proto_query_column_name(index: usize) -> &'static str {
    ITEM_PROTO_QUERY_COLUMN_NAMES
        .get(index)
        .copied()
        .unwrap_or("unknown")
}

/// A failure while validating or decoding one `item_proto` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemProtoRowError {
    /// The row did not contain exactly 34 cells.
    ColumnCount {
        /// Required cell count.
        expected: usize,
        /// Supplied cell count.
        actual: usize,
    },
    /// A required cell was SQL `NULL` in the strict policy.
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
        /// Target integer type.
        target: &'static str,
    },
    /// A syntactically valid integer exceeded its target.
    NumberOverflow {
        /// Zero-based query column.
        column: usize,
        /// Lossy diagnostic representation of the source bytes.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A strict name was longer than the source content bound.
    NameTooLong {
        /// Zero-based name column, either 3 or 4.
        column: usize,
        /// Supplied byte length.
        length: usize,
        /// Maximum supplied length.
        maximum: usize,
    },
    /// A strict name contained a NUL before its final byte.
    NameInteriorNul {
        /// Zero-based name column, either 3 or 4.
        column: usize,
        /// Byte offset of the first NUL.
        index: usize,
    },
}

impl fmt::Display for ItemProtoRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => write!(
                formatter,
                "item-proto query row has {actual} columns; expected {expected}"
            ),
            Self::Null { column } => write!(
                formatter,
                "item-proto query column {} is NULL",
                item_proto_query_column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "item-proto query column {} could not be read: {message}",
                item_proto_query_column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "item-proto query column {} value {value:?} is not a strict {target}",
                item_proto_query_column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "item-proto query column {} value {value:?} overflows {target}",
                item_proto_query_column_name(*column)
            ),
            Self::NameTooLong {
                column,
                length,
                maximum,
            } => write!(
                formatter,
                "item-proto name column {} is {length} bytes; maximum is {maximum}",
                item_proto_query_column_name(*column)
            ),
            Self::NameInteriorNul { column, index } => write!(
                formatter,
                "item-proto name column {} has an interior NUL at byte {index}",
                item_proto_query_column_name(*column)
            ),
        }
    }
}

impl Error for ItemProtoRowError {}

/// Bounds applied before allocating an `item_proto` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemProtoSectionLimits {
    /// Maximum accepted source rows.
    pub max_records: usize,
    /// Maximum accepted packed record-data bytes.
    pub max_data_bytes: usize,
}

impl ItemProtoSectionLimits {
    /// Construct a record cap and leave the byte cap at `usize::MAX`.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            max_records,
            max_data_bytes: usize::MAX,
        }
    }

    /// Construct both record and packed-byte caps.
    #[must_use]
    pub const fn with_data_limit(max_records: usize, max_data_bytes: usize) -> Self {
        Self {
            max_records,
            max_data_bytes,
        }
    }

    /// Alias for [`Self::with_data_limit`].
    #[must_use]
    pub const fn with_limits(max_records: usize, max_data_bytes: usize) -> Self {
        Self::with_data_limit(max_records, max_data_bytes)
    }
}

impl Default for ItemProtoSectionLimits {
    fn default() -> Self {
        Self {
            max_records: ITEM_PROTO_TABLE_MAX_RECORDS,
            max_data_bytes: ITEM_PROTO_TABLE_MAX_SECTION_BYTES,
        }
    }
}

/// A checked `item_proto` section-build failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemProtoSectionError {
    /// The source supplied more rows than the caller allowed.
    TooManyRecords {
        /// Supplied row count.
        count: usize,
        /// Configured record cap.
        maximum: usize,
    },
    /// The row count does not fit the section's `u16` count.
    CountOverflow {
        /// Supplied row count.
        count: usize,
    },
    /// The fixed record width does not fit the section's `u16` width.
    RecordSizeOverflow {
        /// Fixed record width.
        size: usize,
    },
    /// Checked record-data size arithmetic overflowed `usize`.
    DataSizeOverflow {
        /// Row count used in multiplication.
        count: usize,
    },
    /// The output vector could not reserve its bounded length.
    AllocationFailed {
        /// Requested output byte length.
        requested: usize,
    },
    /// Required packed data exceeds the configured byte cap.
    DataTooLarge {
        /// Required packed byte length.
        length: usize,
        /// Configured byte cap.
        maximum: usize,
    },
    /// A source row failed the selected conversion policy.
    Row {
        /// Zero-based source row.
        index: usize,
        /// Row failure.
        source: ItemProtoRowError,
    },
    /// A protocol encoder returned an unexpected record width.
    RecordSizeMismatch {
        /// Zero-based source row.
        index: usize,
        /// Required packed width.
        expected: usize,
        /// Actual encoded width.
        actual: usize,
    },
    /// Generated data did not have its checked length.
    PackedDataLengthMismatch {
        /// Required packed length.
        expected: usize,
        /// Actual generated length.
        actual: usize,
    },
}

impl fmt::Display for ItemProtoSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "item-proto section has {count} rows; configured limit is {maximum}"
            ),
            Self::CountOverflow { count } => {
                write!(
                    formatter,
                    "item-proto section count {count} does not fit u16"
                )
            }
            Self::RecordSizeOverflow { size } => {
                write!(formatter, "item-proto record width {size} does not fit u16")
            }
            Self::DataSizeOverflow { count } => {
                write!(
                    formatter,
                    "item-proto section byte size overflows for {count} rows"
                )
            }
            Self::AllocationFailed { requested } => write!(
                formatter,
                "item-proto section could not allocate {requested} output bytes"
            ),
            Self::DataTooLarge { length, maximum } => write!(
                formatter,
                "item-proto section data length {length} exceeds limit {maximum}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "item-proto row {index} is invalid: {source}")
            }
            Self::RecordSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "item-proto row {index} encoded to {actual} bytes; expected {expected}"
            ),
            Self::PackedDataLengthMismatch { expected, actual } => write!(
                formatter,
                "item-proto section packed {actual} bytes; expected {expected}"
            ),
        }
    }
}

impl Error for ItemProtoSectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// A failure while validating a locale-column identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemProtoLocaleColumnError {
    /// The identifier was empty.
    Empty,
    /// The identifier exceeded the bounded byte length.
    TooLong {
        /// Supplied UTF-8 byte length.
        length: usize,
        /// Maximum accepted byte length.
        maximum: usize,
    },
    /// The first byte was not an ASCII letter or underscore.
    InvalidFirstByte {
        /// First byte value.
        byte: u8,
    },
    /// A later byte was not ASCII alphanumeric or underscore.
    InvalidByte {
        /// Zero-based byte offset.
        index: usize,
        /// Disallowed byte value.
        byte: u8,
    },
}

impl fmt::Display for ItemProtoLocaleColumnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("item-proto locale column is empty"),
            Self::TooLong { length, maximum } => write!(
                formatter,
                "item-proto locale column is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidFirstByte { byte } => write!(
                formatter,
                "item-proto locale column starts with non-identifier byte {byte:#04x}"
            ),
            Self::InvalidByte { index, byte } => write!(
                formatter,
                "item-proto locale column byte {byte:#04x} at offset {index} is not ASCII alphanumeric or '_'"
            ),
        }
    }
}

impl Error for ItemProtoLocaleColumnError {}

/// A validated standalone SQL identifier for the locale-name projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemProtoLocaleColumn {
    value: String,
}

impl ItemProtoLocaleColumn {
    /// Validate one locale identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ItemProtoLocaleColumnError`] if the value is empty, too long,
    /// starts with a non-ASCII-letter, or contains a byte outside the ASCII
    /// identifier allowlist.
    pub fn parse(value: &str) -> Result<Self, ItemProtoLocaleColumnError> {
        if value.is_empty() {
            return Err(ItemProtoLocaleColumnError::Empty);
        }
        if value.len() > MAX_ITEM_PROTO_LOCALE_COLUMN_BYTES {
            return Err(ItemProtoLocaleColumnError::TooLong {
                length: value.len(),
                maximum: MAX_ITEM_PROTO_LOCALE_COLUMN_BYTES,
            });
        }
        let first = value.as_bytes()[0];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return Err(ItemProtoLocaleColumnError::InvalidFirstByte { byte: first });
        }
        if let Some((index, byte)) = value
            .bytes()
            .enumerate()
            .skip(1)
            .find(|(_, byte)| !byte.is_ascii_alphanumeric() && *byte != b'_')
        {
            return Err(ItemProtoLocaleColumnError::InvalidByte { index, byte });
        }
        Ok(Self {
            value: value.to_owned(),
        })
    }

    /// Select the default `name` column, or validate an explicit override.
    ///
    /// `None` selects the legacy default. `Some("")` is rejected rather than
    /// silently normalized.
    ///
    /// # Errors
    ///
    /// Returns [`ItemProtoLocaleColumnError`] for an invalid explicit value.
    pub fn from_config(value: Option<&str>) -> Result<Self, ItemProtoLocaleColumnError> {
        value.map_or_else(|| Ok(Self::default()), Self::parse)
    }

    /// Borrow the validated identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl Default for ItemProtoLocaleColumn {
    fn default() -> Self {
        Self {
            value: DEFAULT_ITEM_PROTO_LOCALE_COLUMN.to_owned(),
        }
    }
}

impl TryFrom<&str> for ItemProtoLocaleColumn {
    type Error = ItemProtoLocaleColumnError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<String> for ItemProtoLocaleColumn {
    type Error = ItemProtoLocaleColumnError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

/// Alias emphasizing the selected value's purpose.
pub type ItemLocaleColumn = ItemProtoLocaleColumn;

/// A defensive failure while composing the fixed `item_proto` query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemProtoQueryBuildError {
    /// The final statement exceeded the legacy buffer bound.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for ItemProtoQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated item-proto query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for ItemProtoQueryBuildError {}

/// A failure while validating query inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemProtoBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The locale-column identifier failed validation.
    LocaleColumn(ItemProtoLocaleColumnError),
    /// A defensive query construction check failed.
    Query(ItemProtoQueryBuildError),
}

impl fmt::Display for ItemProtoBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::LocaleColumn(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for ItemProtoBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::LocaleColumn(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// One immutable checked `item_proto` read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemProtoQuery {
    statement: String,
    table_name: String,
    postfix: TablePostfix,
    locale_column: ItemProtoLocaleColumn,
}

impl ItemProtoQuery {
    /// Build the exact active query from two validated identifier values.
    ///
    /// # Errors
    ///
    /// Returns [`ItemProtoQueryBuildError`] if a future invariant change makes
    /// the final statement exceed the legacy `char[2048]` bound.
    pub fn new(
        postfix: &TablePostfix,
        locale_column: &ItemProtoLocaleColumn,
    ) -> Result<Self, ItemProtoQueryBuildError> {
        let table_name = format!("{ITEM_PROTO_TABLE}{}", postfix.as_str());
        let capacity = ITEM_PROTO_QUERY_PREFIX.len()
            + locale_column.as_str().len()
            + ITEM_PROTO_QUERY_SUFFIX.len()
            + postfix.as_str().len()
            + " ORDER BY vnum;".len();
        let mut statement = String::with_capacity(capacity);
        statement.push_str(ITEM_PROTO_QUERY_PREFIX);
        statement.push_str(locale_column.as_str());
        statement.push_str(ITEM_PROTO_QUERY_SUFFIX);
        statement.push_str(postfix.as_str());
        statement.push_str(" ORDER BY vnum;");
        if statement.len() > MAX_ITEM_PROTO_QUERY_BYTES {
            return Err(ItemProtoQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_ITEM_PROTO_QUERY_BYTES,
            });
        }
        Ok(Self {
            statement,
            table_name,
            postfix: postfix.clone(),
            locale_column: locale_column.clone(),
        })
    }

    /// Build from optional validated configuration values.
    ///
    /// # Errors
    ///
    /// Returns an invalid postfix, locale identifier, or defensive query error.
    pub fn from_config(
        configured_postfix: Option<&str>,
        configured_locale_column: Option<&str>,
    ) -> Result<Self, ItemProtoBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(ItemProtoBoundaryError::Postfix)?;
        let locale = ItemProtoLocaleColumn::from_config(configured_locale_column)
            .map_err(ItemProtoBoundaryError::LocaleColumn)?;
        Self::new(&postfix, &locale).map_err(ItemProtoBoundaryError::Query)
    }

    /// Borrow the exact query text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Borrow the generated table identifier.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }

    /// Borrow the validated locale-column identifier.
    #[must_use]
    pub const fn locale_column(&self) -> &ItemProtoLocaleColumn {
        &self.locale_column
    }
}

/// Alias emphasizing the physical table name.
pub type ItemProtoTableQuery = ItemProtoQuery;
/// Alias emphasizing the physical table name.
pub type ItemProtoTableQueryBuildError = ItemProtoQueryBuildError;
/// Alias emphasizing the physical table name.
pub type ItemProtoTableSectionLimits = ItemProtoSectionLimits;
/// Alias emphasizing the physical table name.
pub type ItemProtoTableRowError = ItemProtoRowError;
/// Alias emphasizing the physical table name.
pub type ItemProtoTableSectionError = ItemProtoSectionError;

/// An injected source of checked `item_proto` query rows.
pub trait ItemProtoRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw rows in the order required by the section.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source failure
    /// must not become an empty row set.
    fn query_rows(&self, query: &ItemProtoQuery) -> Result<Vec<ItemProtoQueryRow>, Self::Error>;
}

impl<F, E> ItemProtoRowSource for F
where
    F: Fn(&ItemProtoQuery) -> Result<Vec<ItemProtoQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &ItemProtoQuery) -> Result<Vec<ItemProtoQueryRow>, Self::Error> {
        self(query)
    }
}

/// Alias emphasizing the physical table name.
pub use ItemProtoRowSource as ItemProtoTableRowSource;

/// A failure while obtaining or building rows through an injected loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemProtoLoadError<E> {
    /// The injected source failed.
    Source(E),
    /// The source returned zero rows, which the legacy loader rejects.
    EmptyResult,
    /// Raw rows failed the selected conversion or section limits.
    Rows(ItemProtoSectionError),
}

impl<E: fmt::Display> fmt::Display for ItemProtoLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "item-proto row source failed: {source}"),
            Self::EmptyResult => write!(formatter, "item-proto source returned no rows"),
            Self::Rows(source) => {
                write!(formatter, "item-proto rows could not be loaded: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for ItemProtoLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::EmptyResult => None,
            Self::Rows(source) => Some(source),
        }
    }
}

/// Reusable checked-query, row-limit, and packing-policy holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemProtoLoader {
    query: ItemProtoQuery,
    limits: ItemProtoSectionLimits,
}

impl ItemProtoLoader {
    /// Construct a loader with the default `name` locale projection.
    ///
    /// # Errors
    ///
    /// Returns a defensive query-construction error.
    pub fn new(
        postfix: &TablePostfix,
        limits: ItemProtoSectionLimits,
    ) -> Result<Self, ItemProtoQueryBuildError> {
        Self::new_with_locale(postfix, &ItemProtoLocaleColumn::default(), limits)
    }

    /// Construct a loader with a validated locale projection.
    ///
    /// # Errors
    ///
    /// Returns a defensive query-construction error.
    pub fn new_with_locale(
        postfix: &TablePostfix,
        locale_column: &ItemProtoLocaleColumn,
        limits: ItemProtoSectionLimits,
    ) -> Result<Self, ItemProtoQueryBuildError> {
        Ok(Self {
            query: ItemProtoQuery::new(postfix, locale_column)?,
            limits,
        })
    }

    /// Construct from raw optional configuration values.
    ///
    /// # Errors
    ///
    /// Returns invalid postfix, locale, or defensive query input.
    pub fn from_config(
        configured_postfix: Option<&str>,
        configured_locale_column: Option<&str>,
        limits: ItemProtoSectionLimits,
    ) -> Result<Self, ItemProtoBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(ItemProtoBoundaryError::Postfix)?;
        let locale = ItemProtoLocaleColumn::from_config(configured_locale_column)
            .map_err(ItemProtoBoundaryError::LocaleColumn)?;
        Self::new_with_locale(&postfix, &locale, limits).map_err(ItemProtoBoundaryError::Query)
    }

    /// Borrow the immutable checked query.
    #[must_use]
    pub const fn query(&self) -> &ItemProtoQuery {
        &self.query
    }

    /// Return configured source and packed-byte limits.
    #[must_use]
    pub const fn limits(&self) -> ItemProtoSectionLimits {
        self.limits
    }

    /// Acquire raw rows and strictly build a section.
    ///
    /// # Errors
    ///
    /// Returns source or selected row/section failure.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, ItemProtoLoadError<S::Error>>
    where
        S: ItemProtoRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ItemProtoLoadError::Source)?;
        if rows.is_empty() {
            return Err(ItemProtoLoadError::EmptyResult);
        }
        build_item_proto_section_with_limits(&rows, self.limits).map_err(ItemProtoLoadError::Rows)
    }

    /// Acquire rows and build with the explicit legacy conversion policy.
    ///
    /// # Errors
    ///
    /// Returns source or selected row/section failure.
    pub fn load_section_legacy<S>(
        &self,
        source: &S,
    ) -> Result<BootSection, ItemProtoLoadError<S::Error>>
    where
        S: ItemProtoRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ItemProtoLoadError::Source)?;
        if rows.is_empty() {
            return Err(ItemProtoLoadError::EmptyResult);
        }
        build_item_proto_section_legacy_with_limits(&rows, self.limits)
            .map_err(ItemProtoLoadError::Rows)
    }
}

/// Alias emphasizing the physical table name.
pub type ItemProtoTableLoader = ItemProtoLoader;

fn check_row_width(row: &ItemProtoQueryRow) -> Result<(), ItemProtoRowError> {
    if row.columns.len() == ITEM_PROTO_QUERY_COLUMN_COUNT {
        Ok(())
    } else {
        Err(ItemProtoRowError::ColumnCount {
            expected: ITEM_PROTO_QUERY_COLUMN_COUNT,
            actual: row.columns.len(),
        })
    }
}

fn cell_bytes(value: &ItemProtoQueryValue, column: usize) -> Result<&[u8], ItemProtoRowError> {
    match value {
        ItemProtoQueryValue::Text(text) => Ok(text.as_bytes()),
        ItemProtoQueryValue::Bytes(bytes) => Ok(bytes.as_slice()),
        ItemProtoQueryValue::Null => Err(ItemProtoRowError::Null { column }),
        ItemProtoQueryValue::Error(message) => Err(ItemProtoRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn diagnostic_value(value: &ItemProtoQueryValue) -> String {
    match value {
        ItemProtoQueryValue::Text(text) => text.clone(),
        ItemProtoQueryValue::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        ItemProtoQueryValue::Null => "<NULL>".to_owned(),
        ItemProtoQueryValue::Error(message) => message.clone(),
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

fn decode_strict_u32(value: &ItemProtoQueryValue, column: usize) -> Result<u32, ItemProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_unsigned_decimal(bytes) {
        return Err(ItemProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "u32",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ItemProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "u32",
    })?;
    text.parse::<u32>()
        .map_err(|_| ItemProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "u32",
        })
}

fn decode_strict_u8(value: &ItemProtoQueryValue, column: usize) -> Result<u8, ItemProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_unsigned_decimal(bytes) {
        return Err(ItemProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "u8",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ItemProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "u8",
    })?;
    text.parse::<u8>()
        .map_err(|_| ItemProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "u8",
        })
}

fn decode_strict_u16(value: &ItemProtoQueryValue, column: usize) -> Result<u16, ItemProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_unsigned_decimal(bytes) {
        return Err(ItemProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "u16",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ItemProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "u16",
    })?;
    text.parse::<u16>()
        .map_err(|_| ItemProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "u16",
        })
}

fn decode_strict_u64(value: &ItemProtoQueryValue, column: usize) -> Result<u64, ItemProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_unsigned_decimal(bytes) {
        return Err(ItemProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "u64",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ItemProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "u64",
    })?;
    text.parse::<u64>()
        .map_err(|_| ItemProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "u64",
        })
}

fn decode_strict_i32(value: &ItemProtoQueryValue, column: usize) -> Result<i32, ItemProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_signed_decimal(bytes) {
        return Err(ItemProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "i32",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ItemProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "i32",
    })?;
    text.parse::<i32>()
        .map_err(|_| ItemProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "i32",
        })
}

fn decode_strict_i16(value: &ItemProtoQueryValue, column: usize) -> Result<i16, ItemProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if !strict_signed_decimal(bytes) {
        return Err(ItemProtoRowError::InvalidNumber {
            column,
            value: diagnostic_value(value),
            target: "i16",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ItemProtoRowError::InvalidNumber {
        column,
        value: diagnostic_value(value),
        target: "i16",
    })?;
    text.parse::<i16>()
        .map_err(|_| ItemProtoRowError::NumberOverflow {
            column,
            value: diagnostic_value(value),
            target: "i16",
        })
}

fn decode_name_strict(
    value: &ItemProtoQueryValue,
    column: usize,
) -> Result<[u8; ITEM_NAME_BYTES], ItemProtoRowError> {
    let bytes = cell_bytes(value, column)?;
    if let Some(index) = bytes.iter().position(|byte| *byte == 0) {
        if index + 1 != bytes.len() {
            return Err(ItemProtoRowError::NameInteriorNul { column, index });
        }
    }
    let maximum = if bytes.last() == Some(&0) {
        ITEM_NAME_BYTES
    } else {
        ITEM_NAME_MAX_LEN
    };
    if bytes.len() > maximum {
        return Err(ItemProtoRowError::NameTooLong {
            column,
            length: bytes.len(),
            maximum,
        });
    }
    let mut output = [0_u8; ITEM_NAME_BYTES];
    output[..bytes.len()].copy_from_slice(bytes);
    Ok(output)
}

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn legacy_c_string_bytes(
    value: &ItemProtoQueryValue,
    column: usize,
) -> Result<&[u8], ItemProtoRowError> {
    let bytes = match value {
        ItemProtoQueryValue::Null => &[][..],
        ItemProtoQueryValue::Text(text) => text.as_bytes(),
        ItemProtoQueryValue::Bytes(bytes) => bytes.as_slice(),
        ItemProtoQueryValue::Error(message) => {
            return Err(ItemProtoRowError::Source {
                column,
                message: message.clone(),
            })
        }
    };
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

fn legacy_strtoul32(value: &ItemProtoQueryValue, column: usize) -> Result<u32, ItemProtoRowError> {
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

fn legacy_strtoull64(value: &ItemProtoQueryValue, column: usize) -> Result<u64, ItemProtoRowError> {
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

fn legacy_strtol32(value: &ItemProtoQueryValue, column: usize) -> Result<i32, ItemProtoRowError> {
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

fn legacy_u8(value: &ItemProtoQueryValue, column: usize) -> Result<u8, ItemProtoRowError> {
    let narrowed = legacy_strtoul32(value, column)? & 0xff;
    u8::try_from(narrowed).map_err(|_| ItemProtoRowError::Source {
        column,
        message: "u8 narrowing failed".to_owned(),
    })
}

fn legacy_u16(value: &ItemProtoQueryValue, column: usize) -> Result<u16, ItemProtoRowError> {
    let narrowed = legacy_strtoul32(value, column)? & 0xffff;
    u16::try_from(narrowed).map_err(|_| ItemProtoRowError::Source {
        column,
        message: "u16 narrowing failed".to_owned(),
    })
}

fn legacy_u32(value: &ItemProtoQueryValue, column: usize) -> Result<u32, ItemProtoRowError> {
    legacy_strtoul32(value, column)
}

fn legacy_u64(value: &ItemProtoQueryValue, column: usize) -> Result<u64, ItemProtoRowError> {
    legacy_strtoull64(value, column)
}

fn legacy_i16(value: &ItemProtoQueryValue, column: usize) -> Result<i16, ItemProtoRowError> {
    let value = legacy_strtol32(value, column)?;
    let bytes = value.to_le_bytes();
    Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
}

fn legacy_i32(value: &ItemProtoQueryValue, column: usize) -> Result<i32, ItemProtoRowError> {
    legacy_strtol32(value, column)
}

fn decode_name_legacy(
    value: &ItemProtoQueryValue,
    column: usize,
) -> Result<[u8; ITEM_NAME_BYTES], ItemProtoRowError> {
    let bytes = legacy_c_string_bytes(value, column)?;
    let length = bytes.len().min(ITEM_NAME_MAX_LEN);
    let mut output = [0_u8; ITEM_NAME_BYTES];
    output[..length].copy_from_slice(&bytes[..length]);
    Ok(output)
}

fn update_special_limit_indexes(record: &mut ItemTableRecord, slot: usize, limit_type: u8) {
    if limit_type == ELimitTypes::RealTimeStartFirstUse as u8 {
        record.limit_real_time_first_use_index = i8::try_from(slot).unwrap_or(-1);
    }
    if limit_type == ELimitTypes::TimerBasedOnWear as u8 {
        record.limit_timer_based_on_wear_index = i8::try_from(slot).unwrap_or(-1);
    }
}

fn derived_record() -> ItemTableRecord {
    ItemTableRecord {
        limit_real_time_first_use_index: -1,
        limit_timer_based_on_wear_index: -1,
        ..ItemTableRecord::default()
    }
}

/// Strictly decode one active `item_proto` row.
///
/// Every numeric cell must be a complete ASCII decimal integer. Names may be
/// raw non-UTF-8 bytes but must fit one C-style 37-byte field. This function
/// derives the active vnum range, special limit indexes, and all absent fields;
/// it performs no semantic enum, uniqueness, or ordering validation.
///
/// # Errors
///
/// Returns [`ItemProtoRowError`] for a wrong-width row, SQL `NULL`, source
/// error, malformed or overflowing number, or unsafe name shape.
pub fn decode_item_proto_query_row(
    row: &ItemProtoQueryRow,
) -> Result<ItemTableRecord, ItemProtoRowError> {
    check_row_width(row)?;
    let vnum = decode_strict_u32(&row.columns[0], 0)?;
    let item_type = decode_strict_u8(&row.columns[1], 1)?;
    let sub_type = decode_strict_u8(&row.columns[2], 2)?;
    let name = decode_name_strict(&row.columns[3], 3)?;
    let locale_name = decode_name_strict(&row.columns[4], 4)?;
    let gold = decode_strict_u64(&row.columns[5], 5)?;
    let shop_buy_price = decode_strict_u64(&row.columns[6], 6)?;
    let weight = decode_strict_u8(&row.columns[7], 7)?;
    let size = decode_strict_u8(&row.columns[8], 8)?;
    let flags = decode_strict_u32(&row.columns[9], 9)?;
    let wear_flags = decode_strict_u32(&row.columns[10], 10)?;
    let anti_flags = decode_strict_u32(&row.columns[11], 11)?;
    let immune_flag = decode_strict_u32(&row.columns[12], 12)?;
    let refined_vnum = decode_strict_u32(&row.columns[13], 13)?;
    let refine_set = decode_strict_u16(&row.columns[14], 14)?;
    let magic_pct = decode_strict_u8(&row.columns[15], 15)?;
    let socket_pct = decode_strict_u8(&row.columns[16], 16)?;
    let addon_type = decode_strict_i16(&row.columns[17], 17)?;
    let limits = [
        ItemLimitRecord {
            limit_type: decode_strict_u8(&row.columns[18], 18)?,
            value: decode_strict_i32(&row.columns[19], 19)?,
        },
        ItemLimitRecord {
            limit_type: decode_strict_u8(&row.columns[20], 20)?,
            value: decode_strict_i32(&row.columns[21], 21)?,
        },
    ];
    let applies = [
        ItemApplyRecord {
            apply_type: decode_strict_u8(&row.columns[22], 22)?,
            value: decode_strict_i32(&row.columns[23], 23)?,
        },
        ItemApplyRecord {
            apply_type: decode_strict_u8(&row.columns[24], 24)?,
            value: decode_strict_i32(&row.columns[25], 25)?,
        },
        ItemApplyRecord {
            apply_type: decode_strict_u8(&row.columns[26], 26)?,
            value: decode_strict_i32(&row.columns[27], 27)?,
        },
    ];
    let values = [
        decode_strict_i32(&row.columns[28], 28)?,
        decode_strict_i32(&row.columns[29], 29)?,
        decode_strict_i32(&row.columns[30], 30)?,
        decode_strict_i32(&row.columns[31], 31)?,
        decode_strict_i32(&row.columns[32], 32)?,
        decode_strict_i32(&row.columns[33], 33)?,
    ];
    let mut record = ItemTableRecord {
        vnum,
        vnum_range: if item_type == EItemTypes::Ds as u8 {
            99
        } else {
            0
        },
        name,
        locale_name,
        item_type,
        sub_type,
        weight,
        size,
        anti_flags,
        flags,
        wear_flags,
        immune_flag,
        gold,
        shop_buy_price,
        limits,
        applies,
        values,
        sockets: [0; ITEM_SOCKET_MAX_NUM],
        refined_vnum,
        refine_set,
        alter_to_magic_item_pct: magic_pct,
        specular: 0,
        gain_socket_pct: socket_pct,
        addon_type,
        ..derived_record()
    };
    update_special_limit_indexes(&mut record, 0, limits[0].limit_type);
    update_special_limit_indexes(&mut record, 1, limits[1].limit_type);
    Ok(record)
}

/// Alias emphasizing the table-row spelling.
///
/// # Errors
///
/// Returns [`ItemProtoRowError`] for a wrong-width row or invalid source cell.
pub fn decode_item_proto_table_row(
    row: &ItemProtoQueryRow,
) -> Result<ItemTableRecord, ItemProtoRowError> {
    decode_item_proto_query_row(row)
}

/// Decode one row with the explicitly named legacy conversion policy.
///
/// SQL `NULL`, empty, and first-NUL cells retain their zero-initialized
/// destination value. Other cells accept C whitespace, an optional sign, and
/// a numeric prefix. Unsigned and signed conversion limits and destination
/// casts follow the active x86 helpers. Names use first-NUL, 36-byte-copy
/// `strlcpy` semantics. Source `Error` cells and wrong row width still fail.
///
/// # Errors
///
/// Returns [`ItemProtoRowError`] for a wrong-width row or source-cell error.
pub fn decode_item_proto_query_row_legacy(
    row: &ItemProtoQueryRow,
) -> Result<ItemTableRecord, ItemProtoRowError> {
    check_row_width(row)?;
    let vnum = legacy_u32(&row.columns[0], 0)?;
    let item_type = legacy_u8(&row.columns[1], 1)?;
    let sub_type = legacy_u8(&row.columns[2], 2)?;
    let name = decode_name_legacy(&row.columns[3], 3)?;
    let locale_name = decode_name_legacy(&row.columns[4], 4)?;
    let gold = legacy_u64(&row.columns[5], 5)?;
    let shop_buy_price = legacy_u64(&row.columns[6], 6)?;
    let weight = legacy_u8(&row.columns[7], 7)?;
    let size = legacy_u8(&row.columns[8], 8)?;
    let flags = legacy_u32(&row.columns[9], 9)?;
    let wear_flags = legacy_u32(&row.columns[10], 10)?;
    let anti_flags = legacy_u32(&row.columns[11], 11)?;
    let immune_flag = legacy_u32(&row.columns[12], 12)?;
    let refined_vnum = legacy_u32(&row.columns[13], 13)?;
    let refine_set = legacy_u16(&row.columns[14], 14)?;
    let magic_pct = legacy_u8(&row.columns[15], 15)?;
    let socket_pct = legacy_u8(&row.columns[16], 16)?;
    let addon_type = legacy_i16(&row.columns[17], 17)?;
    let limits = [
        ItemLimitRecord {
            limit_type: legacy_u8(&row.columns[18], 18)?,
            value: legacy_i32(&row.columns[19], 19)?,
        },
        ItemLimitRecord {
            limit_type: legacy_u8(&row.columns[20], 20)?,
            value: legacy_i32(&row.columns[21], 21)?,
        },
    ];
    let applies = [
        ItemApplyRecord {
            apply_type: legacy_u8(&row.columns[22], 22)?,
            value: legacy_i32(&row.columns[23], 23)?,
        },
        ItemApplyRecord {
            apply_type: legacy_u8(&row.columns[24], 24)?,
            value: legacy_i32(&row.columns[25], 25)?,
        },
        ItemApplyRecord {
            apply_type: legacy_u8(&row.columns[26], 26)?,
            value: legacy_i32(&row.columns[27], 27)?,
        },
    ];
    let values = [
        legacy_i32(&row.columns[28], 28)?,
        legacy_i32(&row.columns[29], 29)?,
        legacy_i32(&row.columns[30], 30)?,
        legacy_i32(&row.columns[31], 31)?,
        legacy_i32(&row.columns[32], 32)?,
        legacy_i32(&row.columns[33], 33)?,
    ];
    let mut record = ItemTableRecord {
        vnum,
        vnum_range: if item_type == EItemTypes::Ds as u8 {
            99
        } else {
            0
        },
        name,
        locale_name,
        item_type,
        sub_type,
        weight,
        size,
        anti_flags,
        flags,
        wear_flags,
        immune_flag,
        gold,
        shop_buy_price,
        limits,
        applies,
        values,
        sockets: [0; ITEM_SOCKET_MAX_NUM],
        refined_vnum,
        refine_set,
        alter_to_magic_item_pct: magic_pct,
        specular: 0,
        gain_socket_pct: socket_pct,
        addon_type,
        ..derived_record()
    };
    update_special_limit_indexes(&mut record, 0, limits[0].limit_type);
    update_special_limit_indexes(&mut record, 1, limits[1].limit_type);
    Ok(record)
}

/// Legacy alias emphasizing the table-row spelling.
///
/// # Errors
///
/// Returns [`ItemProtoRowError`] for a wrong-width row or source-cell error.
pub fn decode_item_proto_table_row_legacy(
    row: &ItemProtoQueryRow,
) -> Result<ItemTableRecord, ItemProtoRowError> {
    decode_item_proto_query_row_legacy(row)
}

fn validate_section_size(
    count: usize,
    limits: ItemProtoSectionLimits,
) -> Result<(u16, u16, usize), ItemProtoSectionError> {
    if count > limits.max_records {
        return Err(ItemProtoSectionError::TooManyRecords {
            count,
            maximum: limits.max_records,
        });
    }
    let wire_count =
        u16::try_from(count).map_err(|_| ItemProtoSectionError::CountOverflow { count })?;
    let record_size = u16::try_from(ITEM_PROTO_TABLE_WIRE_SIZE).map_err(|_| {
        ItemProtoSectionError::RecordSizeOverflow {
            size: ITEM_PROTO_TABLE_WIRE_SIZE,
        }
    })?;
    let data_len = ITEM_PROTO_TABLE_WIRE_SIZE
        .checked_mul(count)
        .ok_or(ItemProtoSectionError::DataSizeOverflow { count })?;
    if data_len > limits.max_data_bytes {
        return Err(ItemProtoSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }
    Ok((wire_count, record_size, data_len))
}

fn append_rows(
    rows: &[ItemProtoQueryRow],
    limits: ItemProtoSectionLimits,
    legacy: bool,
) -> Result<BootSection, ItemProtoSectionError> {
    let (count, record_size, data_len) = validate_section_size(rows.len(), limits)?;
    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| ItemProtoSectionError::AllocationFailed {
            requested: data_len,
        })?;

    for (index, row) in rows.iter().enumerate() {
        let record = if legacy {
            decode_item_proto_query_row_legacy(row)
        } else {
            decode_item_proto_query_row(row)
        }
        .map_err(|source| ItemProtoSectionError::Row { index, source })?;
        let encoded = record.encode();
        if encoded.len() != ITEM_PROTO_TABLE_WIRE_SIZE {
            return Err(ItemProtoSectionError::RecordSizeMismatch {
                index,
                expected: ITEM_PROTO_TABLE_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }
    if data.len() != data_len {
        return Err(ItemProtoSectionError::PackedDataLengthMismatch {
            expected: data_len,
            actual: data.len(),
        });
    }
    Ok(BootSection {
        kind: BootSectionKind::Item,
        record_size,
        count,
        data,
    })
}

/// Build a strict `item_proto` section with default limits.
///
/// Rows are encoded in exactly the supplied source order. Duplicates are
/// retained. An empty input is represented by a validated empty section at
/// this pure boundary; the `SQLx` acquisition boundary rejects an empty source.
///
/// # Errors
///
/// Returns [`ItemProtoSectionError`] for invalid rows or configured limits.
pub fn build_item_proto_section(
    rows: &[ItemProtoQueryRow],
) -> Result<BootSection, ItemProtoSectionError> {
    build_item_proto_section_with_limits(rows, ItemProtoSectionLimits::default())
}

/// Build a strict section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`ItemProtoSectionError`] for invalid rows or configured limits.
pub fn build_item_proto_section_with_limit(
    rows: &[ItemProtoQueryRow],
    max_records: usize,
) -> Result<BootSection, ItemProtoSectionError> {
    build_item_proto_section_with_limits(rows, ItemProtoSectionLimits::new(max_records))
}

/// Build a strict typed active `item_proto` boot section.
///
/// # Errors
///
/// Returns [`ItemProtoSectionError`] for invalid rows, allocation failures, or
/// configured limit violations.
pub fn build_item_proto_section_with_limits(
    rows: &[ItemProtoQueryRow],
    limits: ItemProtoSectionLimits,
) -> Result<BootSection, ItemProtoSectionError> {
    append_rows(rows, limits, false)
}

/// Build a section with the explicitly named legacy conversion policy.
///
/// # Errors
///
/// Returns [`ItemProtoSectionError`] for source, allocation, or configured
/// limit failures.
pub fn build_item_proto_section_legacy(
    rows: &[ItemProtoQueryRow],
) -> Result<BootSection, ItemProtoSectionError> {
    build_item_proto_section_legacy_with_limits(rows, ItemProtoSectionLimits::default())
}

/// Build a legacy-policy section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`ItemProtoSectionError`] for source, allocation, or configured
/// limit failures.
pub fn build_item_proto_section_legacy_with_limit(
    rows: &[ItemProtoQueryRow],
    max_records: usize,
) -> Result<BootSection, ItemProtoSectionError> {
    build_item_proto_section_legacy_with_limits(rows, ItemProtoSectionLimits::new(max_records))
}

/// Build a legacy-policy section with caller-selected record and byte caps.
///
/// # Errors
///
/// Returns [`ItemProtoSectionError`] for source, allocation, or configured
/// limit failures.
pub fn build_item_proto_section_legacy_with_limits(
    rows: &[ItemProtoQueryRow],
    limits: ItemProtoSectionLimits,
) -> Result<BootSection, ItemProtoSectionError> {
    append_rows(rows, limits, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::decode_item_table_section;

    fn text(value: &str) -> ItemProtoQueryValue {
        ItemProtoQueryValue::text(value)
    }

    fn valid_row() -> ItemProtoQueryRow {
        ItemProtoQueryRow::from_typed_columns([
            text("42"),
            text("1"),
            text("2"),
            text("Sword"),
            text("Sword-locale"),
            text("18446744073709551615"),
            text("99"),
            text("7"),
            text("5"),
            text("11"),
            text("12"),
            text("13"),
            text("14"),
            text("43"),
            text("65535"),
            text("3"),
            text("4"),
            text("-32768"),
            text("7"),
            text("-11"),
            text("8"),
            text("22"),
            text("1"),
            text("-101"),
            text("2"),
            text("202"),
            text("3"),
            text("-303"),
            text("-1"),
            text("2"),
            text("-3"),
            text("4"),
            text("-5"),
            text("6"),
        ])
    }

    fn replace(row: &mut ItemProtoQueryRow, column: usize, value: ItemProtoQueryValue) {
        let mut columns = row.clone().into_columns();
        columns[column] = value;
        *row = ItemProtoQueryRow::new(columns);
    }

    #[test]
    fn exact_query_metadata_and_length_are_bounded() {
        assert_eq!(ITEM_PROTO_QUERY_COLUMN_COUNT, 34);
        assert_eq!(ITEM_PROTO_QUERY_COLUMN_NAMES.len(), 34);
        assert_eq!(ITEM_PROTO_TABLE_WIRE_SIZE, 204);
        assert_eq!(ITEM_PROTO_SECTION_RECORD_SIZE, 204);
        assert_eq!(ITEM_PROTO_TABLE_MAX_RECORDS, 65_535);
        assert_eq!(ITEM_PROTO_TABLE_MAX_SECTION_BYTES, 13_369_140);

        let default = ItemProtoQuery::from_config(None, None).unwrap();
        assert_eq!(default.table_name(), "item_proto");
        assert_eq!(default.locale_column().as_str(), "name");
        assert_eq!(default.as_str().len(), 379);
        assert_eq!(
            default.as_str(),
            "SELECT vnum, type, subtype, name, name, gold, shop_buy_price, weight, size, flag, wearflag, antiflag, immuneflag+0, refined_vnum, refine_set, magic_pct, socket_pct, addon_type, limittype0, limitvalue0, limittype1, limitvalue1, applytype0, applyvalue0, applytype1, applyvalue1, applytype2, applyvalue2, value0, value1, value2, value3, value4, value5 FROM item_proto ORDER BY vnum;"
        );

        let base = ITEM_PROTO_QUERY_TEMPLATE
            .replace("{locale}", "")
            .replace("{postfix}", "");
        assert_eq!(base.len(), 375);
        let custom = ItemProtoQuery::from_config(Some("_eu2"), Some("gb2312name")).unwrap();
        assert_eq!(custom.table_name(), "item_proto_eu2");
        assert!(custom.as_str().contains(", gb2312name, gold,"));
        assert!(custom
            .as_str()
            .ends_with(" FROM item_proto_eu2 ORDER BY vnum;"));
        assert!(custom.as_str().len() <= MAX_ITEM_PROTO_QUERY_BYTES);
        assert_eq!(
            custom.as_str().matches(',').count() + 1,
            ITEM_PROTO_QUERY_COLUMN_COUNT
        );

        let long_locale = "a".repeat(MAX_ITEM_PROTO_LOCALE_COLUMN_BYTES);
        let long_postfix = "p".repeat(crate::postfix::MAX_TABLE_POSTFIX_BYTES);
        let maximum_inputs =
            ItemProtoQuery::from_config(Some(&long_postfix), Some(&long_locale)).unwrap();
        assert_eq!(maximum_inputs.as_str().len(), 885);
        assert!(maximum_inputs.as_str().len() <= MAX_ITEM_PROTO_QUERY_BYTES);
    }

    #[test]
    fn locale_and_postfix_validation_reject_unsafe_inputs() {
        assert_eq!(ItemProtoLocaleColumn::default().as_str(), "name");
        for invalid in ["", "1name", "na-me", "na me", "na.me", "na;me"] {
            assert!(
                ItemProtoLocaleColumn::parse(invalid).is_err(),
                "{invalid:?}"
            );
        }
        assert!(ItemProtoLocaleColumn::parse("_gb2312_2").is_ok());
        let long = "a".repeat(MAX_ITEM_PROTO_LOCALE_COLUMN_BYTES);
        assert!(ItemProtoLocaleColumn::parse(&long).is_ok());
        assert!(ItemProtoLocaleColumn::parse(&format!("{long}b")).is_err());
        assert!(matches!(
            ItemProtoQuery::from_config(None, Some("bad-name")),
            Err(ItemProtoBoundaryError::LocaleColumn(_))
        ));
        assert!(matches!(
            ItemProtoQuery::from_config(Some("bad-name"), None),
            Err(ItemProtoBoundaryError::Postfix(_))
        ));
    }

    #[test]
    fn strict_row_maps_every_field_and_active_defaults() {
        let record = decode_item_proto_query_row(&valid_row()).unwrap();
        assert_eq!(record.vnum, 42);
        assert_eq!(record.vnum_range, 0);
        assert_eq!(&record.name[..5], b"Sword");
        assert_eq!(&record.locale_name[..12], b"Sword-locale");
        assert_eq!(record.gold, u64::MAX);
        assert_eq!(record.shop_buy_price, 99);
        assert_eq!(record.item_type, 1);
        assert_eq!(record.sub_type, 2);
        assert_eq!(record.weight, 7);
        assert_eq!(record.size, 5);
        assert_eq!(record.flags, 11);
        assert_eq!(record.wear_flags, 12);
        assert_eq!(record.anti_flags, 13);
        assert_eq!(record.immune_flag, 14);
        assert_eq!(record.refined_vnum, 43);
        assert_eq!(record.refine_set, 65_535);
        assert_eq!(record.alter_to_magic_item_pct, 3);
        assert_eq!(record.gain_socket_pct, 4);
        assert_eq!(record.addon_type, i16::MIN);
        assert_eq!(record.limits[0].value, -11);
        assert_eq!(record.limits[1].value, 22);
        assert_eq!(record.applies[0].value, -101);
        assert_eq!(record.applies[2].value, -303);
        assert_eq!(record.values, [-1, 2, -3, 4, -5, 6]);
        assert_eq!(record.sockets, [0; 6]);
        assert_eq!(record.specular, 0);
        assert_eq!(record.limit_real_time_first_use_index, 0);
        assert_eq!(record.limit_timer_based_on_wear_index, 1);
        assert_eq!(record.encode().len(), ITEM_PROTO_TABLE_WIRE_SIZE);
    }

    #[test]
    fn active_vnum_range_and_limit_defaults_are_derived() {
        let mut row = valid_row();
        replace(&mut row, 1, text("29"));
        let ds = decode_item_proto_query_row(&row).unwrap();
        assert_eq!(ds.vnum_range, 99);

        let mut row = valid_row();
        replace(&mut row, 18, text("0"));
        replace(&mut row, 20, text("0"));
        let defaults = decode_item_proto_query_row(&row).unwrap();
        assert_eq!(defaults.limit_real_time_first_use_index, -1);
        assert_eq!(defaults.limit_timer_based_on_wear_index, -1);

        let mut row = valid_row();
        replace(&mut row, 18, text("8"));
        replace(&mut row, 20, text("7"));
        let last = decode_item_proto_query_row(&row).unwrap();
        assert_eq!(last.limit_real_time_first_use_index, 1);
        assert_eq!(last.limit_timer_based_on_wear_index, 0);
    }

    #[test]
    fn strict_policy_rejects_null_error_width_and_empty_numbers() {
        assert_eq!(
            decode_item_proto_query_row(&ItemProtoQueryRow::new(Vec::new())),
            Err(ItemProtoRowError::ColumnCount {
                expected: 34,
                actual: 0
            })
        );
        let mut row = valid_row();
        replace(&mut row, 5, ItemProtoQueryValue::Null);
        assert_eq!(
            decode_item_proto_query_row(&row),
            Err(ItemProtoRowError::Null { column: 5 })
        );
        replace(&mut row, 5, ItemProtoQueryValue::error("driver failed"));
        assert_eq!(
            decode_item_proto_query_row(&row),
            Err(ItemProtoRowError::Source {
                column: 5,
                message: "driver failed".to_owned()
            })
        );
        for invalid in ["", " 1", "1 ", "1x", "+1", "-1"] {
            let mut row = valid_row();
            replace(&mut row, 0, text(invalid));
            assert!(matches!(
                decode_item_proto_query_row(&row),
                Err(ItemProtoRowError::InvalidNumber { column: 0, .. })
            ));
        }
    }

    #[test]
    fn strict_policy_rejects_overflow_and_unsafe_names_but_allows_raw_bytes() {
        let mut row = valid_row();
        replace(&mut row, 0, text("4294967296"));
        assert!(matches!(
            decode_item_proto_query_row(&row),
            Err(ItemProtoRowError::NumberOverflow { column: 0, .. })
        ));
        replace(&mut row, 0, text("42"));
        replace(&mut row, 19, text(&(i64::from(i32::MAX) + 1).to_string()));
        assert!(matches!(
            decode_item_proto_query_row(&row),
            Err(ItemProtoRowError::NumberOverflow { column: 19, .. })
        ));

        let mut row = valid_row();
        replace(&mut row, 3, ItemProtoQueryValue::bytes(vec![b'A'; 36]));
        let record = decode_item_proto_query_row(&row).unwrap();
        assert_eq!(&record.name[..36], &[b'A'; 36]);
        assert_eq!(record.name[36], 0);

        replace(
            &mut row,
            3,
            ItemProtoQueryValue::bytes(vec![0xff, 0x80, b'X']),
        );
        assert_eq!(decode_item_proto_query_row(&row).unwrap().name[0], 0xff);

        replace(&mut row, 3, ItemProtoQueryValue::bytes(vec![b'A'; 37]));
        assert!(matches!(
            decode_item_proto_query_row(&row),
            Err(ItemProtoRowError::NameTooLong {
                column: 3,
                length: 37,
                maximum: 36
            })
        ));
        replace(&mut row, 3, ItemProtoQueryValue::bytes(vec![b'A', 0, b'B']));
        assert!(matches!(
            decode_item_proto_query_row(&row),
            Err(ItemProtoRowError::NameInteriorNul {
                column: 3,
                index: 1
            })
        ));
    }

    #[test]
    fn legacy_policy_models_null_prefixes_wrap_saturation_and_narrowing() {
        let mut row = valid_row();
        for column in 0..34 {
            replace(&mut row, column, ItemProtoQueryValue::Null);
        }
        let zero = decode_item_proto_query_row_legacy(&row).unwrap();
        assert_eq!(zero, derived_record());

        let mut row = valid_row();
        replace(&mut row, 0, text(" 12junk"));
        replace(&mut row, 1, text("-1"));
        replace(&mut row, 5, text("-2"));
        replace(&mut row, 14, text("65535"));
        replace(&mut row, 17, text("32768"));
        replace(&mut row, 19, text("4294967295"));
        replace(&mut row, 23, text("invalid"));
        let legacy = decode_item_proto_query_row_legacy(&row).unwrap();
        assert_eq!(legacy.vnum, 12);
        assert_eq!(legacy.item_type, 255);
        assert_eq!(legacy.gold, u64::MAX - 1);
        assert_eq!(legacy.refine_set, 65_535);
        assert_eq!(legacy.addon_type, -32_768);
        assert_eq!(legacy.limits[0].value, i32::MAX);
        assert_eq!(legacy.applies[0].value, 0);
    }

    #[test]
    fn legacy_unsigned_boundaries_match_x86_conversion_helpers() {
        for (input, expected_u32, expected_u64) in [
            ("0", 0, 0),
            ("-1", u32::MAX, u64::MAX),
            ("-4294967295", 1, 18_446_744_069_414_584_321),
            ("-4294967296", u32::MAX, 18_446_744_069_414_584_320),
            ("4294967295", u32::MAX, 4_294_967_295),
            ("4294967296", u32::MAX, 4_294_967_296),
            ("18446744073709551615", u32::MAX, u64::MAX),
            ("-9223372036854775808", u32::MAX, 9_223_372_036_854_775_808),
            ("-9223372036854775809", u32::MAX, 9_223_372_036_854_775_807),
            ("-18446744073709551615", u32::MAX, 1),
            ("18446744073709551616", u32::MAX, u64::MAX),
            ("-18446744073709551616", u32::MAX, u64::MAX),
        ] {
            let mut row = valid_row();
            replace(&mut row, 0, text(input));
            replace(&mut row, 5, text(input));
            let record = decode_item_proto_query_row_legacy(&row).unwrap();
            assert_eq!(record.vnum, expected_u32, "u32 input {input:?}");
            assert_eq!(record.gold, expected_u64, "u64 input {input:?}");
        }
    }

    #[test]
    fn legacy_name_uses_first_nul_and_truncation() {
        let mut row = valid_row();
        let long_name = vec![b'x'; 50];
        replace(
            &mut row,
            3,
            ItemProtoQueryValue::bytes([long_name, vec![0, b'Y', b'Z']].concat()),
        );
        let record = decode_item_proto_query_row_legacy(&row).unwrap();
        assert_eq!(&record.name[..36], &[b'x'; 36]);
        assert!(record.name[36..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn section_preserves_order_duplicates_limits_and_boot_round_trip() {
        let first = valid_row();
        let mut second = first.clone();
        replace(&mut second, 0, text("7"));
        let section = build_item_proto_section_with_limits(
            &[first.clone(), first, second],
            ItemProtoSectionLimits::with_data_limit(3, 612),
        )
        .unwrap();
        assert_eq!(section.kind, BootSectionKind::Item);
        assert_eq!(section.record_size, 204);
        assert_eq!(section.count, 3);
        assert_eq!(section.data.len(), 612);
        let decoded = decode_item_table_section(&section).unwrap();
        assert_eq!(decoded[0].vnum, 42);
        assert_eq!(decoded[1].vnum, 42);
        assert_eq!(decoded[2].vnum, 7);

        assert!(matches!(
            build_item_proto_section_with_limits(
                &[valid_row(), valid_row()],
                ItemProtoSectionLimits::new(1)
            ),
            Err(ItemProtoSectionError::TooManyRecords { .. })
        ));
        assert!(matches!(
            build_item_proto_section_with_limits(
                &[valid_row()],
                ItemProtoSectionLimits::with_data_limit(1, 203)
            ),
            Err(ItemProtoSectionError::DataTooLarge { .. })
        ));
    }

    #[test]
    fn injected_loader_preserves_source_failure_and_uses_checked_query() {
        let postfix = TablePostfix::parse("_test").unwrap();
        let loader = ItemProtoLoader::new(&postfix, ItemProtoSectionLimits::new(1)).unwrap();
        let offline: fn(&ItemProtoQuery) -> Result<Vec<ItemProtoQueryRow>, String> =
            |_| Err("offline".to_owned());
        assert!(matches!(
            loader.load_section(&offline),
            Err(ItemProtoLoadError::Source(_))
        ));
        let empty: fn(&ItemProtoQuery) -> Result<Vec<ItemProtoQueryRow>, String> =
            |_| Ok(Vec::new());
        assert!(matches!(
            loader.load_section(&empty),
            Err(ItemProtoLoadError::EmptyResult)
        ));
        assert!(matches!(
            loader.load_section_legacy(&empty),
            Err(ItemProtoLoadError::EmptyResult)
        ));
        let rows: fn(&ItemProtoQuery) -> Result<Vec<ItemProtoQueryRow>, String> = |query| {
            assert!(query.as_str().contains("item_proto_test"));
            Ok(vec![valid_row()])
        };
        assert_eq!(loader.load_section(&rows).unwrap().count, 1);
    }
}
