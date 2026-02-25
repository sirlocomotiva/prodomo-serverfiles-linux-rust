//! SQL-free base `TShopTable` boot boundary.
//!
//! The legacy loader in `server/server/db/ClientManagerBoot.cpp:299-379`
//! runs one fixed four-column query.  It rejects an empty SQL result, groups
//! rows by the signed x86 `int` shop vnum in a `std::map`, and emits shops in
//! ascending map-key order.  Every source row parses the NPC cell again.  A
//! SQL `NULL` item vnum skips that row; all other item rows append in source
//! order, including duplicate shop/item rows.  The source has a fixed
//! `SHOP_HOST_ITEM_MAX_NUM` (40) item capacity.
//!
//! This module is a pure projection boundary.  It does not execute SQL,
//! retain a connection, mutate a manager, or claim gameplay integration.  The
//! caller supplies raw query cells.  The default decoder is strict.  The
//! explicitly named legacy decoder models the C `str_to_number` prefixes,
//! zero-filled numeric `NULL`s, x86 casts, and the source's `NULL` item skip.
//! Both policies return typed errors instead of reproducing the C++ buffer
//! overrun or other memory corruption.
//!
//! The active build enables `ENABLE_RENEWAL_SHOPEX` and
//! `ENABLE_REMOVE_LIMIT_GOLD`.  Consequently each item is 68 bytes and each
//! base shop record is 2,762 bytes.  Unused item slots are made with the
//! protocol's source-shaped zero constructor, which sets `price_type` to
//! `SHOPEX_GOLD` (1) and the `TItemPos()` default `{ window_type: 1,
//! cell: 65_535 }`; all remaining scalar/array fields remain zero.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootSection, BootSectionKind};
use protocol::db_records::{RecordCodec, ShopItemPosition, ShopItemRecord, ShopTableRecord};

/// Exact statement used by the legacy base shop loader.
///
/// The spacing and ordering are part of the source boundary.  This constant
/// is intentionally not interpolated with a table postfix.
pub const SHOP_TABLE_QUERY: &str = "SELECT shop.vnum, shop.npc_vnum, shop_item.item_vnum, shop_item.count FROM shop LEFT JOIN shop_item ON shop.vnum = shop_item.shop_vnum ORDER BY shop.vnum, shop_item.item_vnum";

/// Short alias for [`SHOP_TABLE_QUERY`].
pub const SHOP_QUERY: &str = SHOP_TABLE_QUERY;

/// Number of columns promised by the fixed query.
pub const SHOP_TABLE_QUERY_COLUMNS: usize = 4;

/// Alias for [`SHOP_TABLE_QUERY_COLUMNS`].
pub const SHOP_QUERY_COLUMN_COUNT: usize = SHOP_TABLE_QUERY_COLUMNS;

/// Fixed source capacity of one base shop's item array.
pub const SHOP_ITEM_MAX_NUM: usize = 40;

/// Alias matching the legacy C++ constant name.
pub const SHOP_HOST_ITEM_MAX_NUM: usize = SHOP_ITEM_MAX_NUM;

/// Number of bytes in the source-fixed `shop_name` array, including its NUL.
pub const SHOP_NAME_BYTES: usize = 33;

/// Alias for the source `SHOP_TAB_NAME_MAX + 1` width.
pub const SHOP_TABLE_NAME_BYTES: usize = SHOP_NAME_BYTES;

/// `SHOPEX_GOLD` under the active `ENABLE_RENEWAL_SHOPEX` build.
pub const SHOPEX_GOLD: u8 = 1;

/// Legacy `TItemPos()` inventory window selector.
pub const SHOP_ITEM_DEFAULT_WINDOW_TYPE: u8 = 1;

/// Legacy `TItemPos()` invalid/empty inventory cell (`WORD_MAX`).
pub const SHOP_ITEM_DEFAULT_CELL: u16 = u16::MAX;

/// Maximum number of distinct shop groups representable by the boot count.
pub const SHOP_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Defensive raw-row ceiling derived from the fixed per-shop item capacity.
///
/// This is an acquisition bound, not the boot section's group count. The
/// derivation assumes one `shop` row per distinct `shop.vnum`; that is a
/// fail-closed acquisition policy for malformed/duplicate source rows, not a
/// schema claim made by the legacy loader. The pure builder still enforces the
/// exact group and item limits.
pub const SHOP_TABLE_MAX_SOURCE_ROWS: usize = SHOP_TABLE_MAX_RECORDS * SHOP_ITEM_MAX_NUM;

/// Derive a safe raw-row cap from a distinct-shop cap.
///
/// The multiplier is a defensive policy based on the fixed item capacity. It
/// deliberately fails closed if the source contains duplicate shop rows. It
/// does not claim that the database schema enforces that relationship. `None`
/// means the requested group cap cannot fit the legacy `u16` count and must be
/// rejected before querying.
#[must_use]
pub const fn shop_source_row_limit(max_records: usize) -> Option<usize> {
    if max_records > SHOP_TABLE_MAX_RECORDS {
        None
    } else {
        Some(max_records * SHOP_ITEM_MAX_NUM)
    }
}

/// Exact packed item width in the active x86 build.
pub const SHOP_ITEM_WIRE_SIZE: usize = ShopItemRecord::WIRE_SIZE;

/// Exact packed base-shop width in the active x86 build.
pub const SHOP_TABLE_WIRE_SIZE: usize = ShopTableRecord::WIRE_SIZE;

/// Section record-size field used by the base-shop boot stream.
#[allow(clippy::cast_possible_truncation)]
pub const SHOP_SECTION_RECORD_SIZE: u16 = ShopTableRecord::WIRE_SIZE as u16;

/// Maximum section-data length at the representable `u16` count.
pub const SHOP_TABLE_MAX_SECTION_BYTES: usize = SHOP_TABLE_WIRE_SIZE * SHOP_TABLE_MAX_RECORDS;

/// Source-order query column names/expressions for diagnostics.
pub const SHOP_TABLE_QUERY_COLUMN_NAMES: [&str; SHOP_TABLE_QUERY_COLUMNS] = [
    "shop.vnum",
    "shop.npc_vnum",
    "shop_item.item_vnum",
    "shop_item.count",
];

/// One raw value returned by a base-shop query adapter.
///
/// `Text` and `Bytes` both represent non-NULL SQL values.  `Bytes` is useful
/// for adapters that must retain driver bytes which are not UTF-8.  `Null` and
/// `Error` remain distinct facts until a conversion policy is selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShopQueryValue {
    /// A non-NULL textual SQL value.
    Text(String),
    /// A non-NULL raw SQL value.
    Bytes(Vec<u8>),
    /// A SQL `NULL` value.
    Null,
    /// An extraction or source error.
    Error(String),
}

impl ShopQueryValue {
    /// Construct a non-NULL textual value.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    /// Copy a non-NULL raw value.
    #[must_use]
    pub fn bytes(value: impl AsRef<[u8]>) -> Self {
        Self::Bytes(value.as_ref().to_vec())
    }

    /// Construct a SQL `NULL` value.
    #[must_use]
    pub const fn null() -> Self {
        Self::Null
    }

    /// Construct a source error value.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error(message.into())
    }
}

impl From<String> for ShopQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for ShopQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for ShopQueryValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for ShopQueryValue {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

/// A source-shaped row for the four-column base-shop query.
///
/// The vector intentionally retains malformed widths so the decoder can
/// report a typed `ColumnCount` error instead of indexing past the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopTableQueryRow {
    columns: Vec<ShopQueryValue>,
}

impl ShopTableQueryRow {
    /// Construct a row while retaining all supplied cells and their order.
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ShopQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub fn from_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ShopQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct and validate a four-column row.
    ///
    /// # Errors
    ///
    /// Returns [`ShopRowError::ColumnCount`] when the iterator does not yield
    /// exactly four cells.
    pub fn try_new<I>(columns: I) -> Result<Self, ShopRowError>
    where
        I: IntoIterator<Item = ShopQueryValue>,
    {
        let row = Self::new(columns);
        check_row_width(&row)?;
        Ok(row)
    }

    /// Construct a row from a statically sized column set.
    #[must_use]
    pub fn from_typed_columns(columns: [ShopQueryValue; SHOP_TABLE_QUERY_COLUMNS]) -> Self {
        Self::new(columns)
    }

    /// Borrow all cells in source-query order.
    #[must_use]
    pub fn columns(&self) -> &[ShopQueryValue] {
        &self.columns
    }

    /// Consume the row and return its cells in source-query order.
    #[must_use]
    pub fn into_columns(self) -> Vec<ShopQueryValue> {
        self.columns
    }

    /// Return the supplied width, including a malformed width.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

impl From<[ShopQueryValue; SHOP_TABLE_QUERY_COLUMNS]> for ShopTableQueryRow {
    fn from(columns: [ShopQueryValue; SHOP_TABLE_QUERY_COLUMNS]) -> Self {
        Self::from_typed_columns(columns)
    }
}

impl TryFrom<Vec<ShopQueryValue>> for ShopTableQueryRow {
    type Error = ShopRowError;

    fn try_from(columns: Vec<ShopQueryValue>) -> Result<Self, Self::Error> {
        Self::try_new(columns)
    }
}

/// Short aliases used by query adapters.
pub type ShopQueryRow = ShopTableQueryRow;
/// Alias for one query cell.
pub type ShopQueryColumn = ShopQueryValue;
/// Table-oriented alias for [`ShopQueryValue`].
pub type ShopTableCell = ShopQueryValue;
/// Table-oriented alias for [`ShopTableQueryRow`].
pub type ShopTableRow = ShopTableQueryRow;
/// Table-specific alias for [`ShopQueryValue`].
pub type ShopTableQueryValue = ShopQueryValue;
/// Table-specific alias for [`ShopRowError`].
pub type ShopTableRowError = ShopRowError;
/// Table-specific alias for [`ShopSectionLimits`].
pub type ShopTableSectionLimits = ShopSectionLimits;
/// Table-specific alias for [`ShopSectionError`].
pub type ShopTableSectionError = ShopSectionError;
/// Table-specific alias for [`ShopSectionError`].
pub type ShopTableSectionBuildError = ShopSectionError;
/// Table-specific alias for [`ShopSectionBuilder`].
pub type ShopTableBuilder = ShopSectionBuilder;
/// Table-specific alias for [`ShopTableRowSource`].
pub use ShopTableRowSource as ShopTableSource;

/// Return a stable query-column name for diagnostics.
#[must_use]
pub const fn shop_query_column_name(index: usize) -> &'static str {
    match index {
        0 => "shop.vnum",
        1 => "shop.npc_vnum",
        2 => "shop_item.item_vnum",
        3 => "shop_item.count",
        _ => "unknown",
    }
}

/// A strict or explicitly selected compatibility error in one source row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShopRowError {
    /// The row did not contain exactly four cells.
    ColumnCount {
        /// Required source width.
        expected: usize,
        /// Supplied source width.
        actual: usize,
    },
    /// A required cell was SQL `NULL` in the strict policy, or a source cell
    /// error was encountered while parsing a row.
    Null {
        /// Zero-based query-column index.
        column: usize,
    },
    /// A source adapter could not obtain or convert a cell.
    Source {
        /// Zero-based query-column index.
        column: usize,
        /// Original source diagnostic.
        message: String,
    },
    /// A non-NULL value was not a complete strict decimal integer.
    InvalidNumber {
        /// Zero-based query-column index.
        column: usize,
        /// Bounded, lossy diagnostic representation of the original value.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A syntactically valid integer did not fit its strict target type.
    NumberOverflow {
        /// Zero-based query-column index.
        column: usize,
        /// Bounded, lossy diagnostic representation of the original value.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
}

impl fmt::Display for ShopRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => {
                write!(
                    formatter,
                    "shop query row has {actual} columns; expected {expected}"
                )
            }
            Self::Null { column } => write!(
                formatter,
                "shop query column {} is NULL",
                shop_query_column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "shop query column {} could not be read: {}",
                shop_query_column_name(*column),
                bounded_display(message)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "shop query column {} value {:?} is not a strict {}",
                shop_query_column_name(*column),
                value,
                target
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "shop query column {} value {:?} overflows {}",
                shop_query_column_name(*column),
                value,
                target
            ),
        }
    }
}

impl Error for ShopRowError {}

/// The decoded scalar values for one source row.
///
/// `shop_vnum` is kept as the source's signed x86 `int` map key.  The wire
/// record stores the corresponding bit pattern in its `u32` field.  `item`
/// is `None` only for the explicitly named legacy `NULL`-item skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedShopQueryRow {
    /// Signed x86 map key used by the source.
    pub shop_vnum: i32,
    /// NPC vnum parsed from this row.
    pub npc_vnum: u32,
    /// Decoded item, or `None` for a legacy NULL item vnum.
    pub item: Option<ShopItemRecord>,
}

/// Alias for [`DecodedShopQueryRow`].
pub type ShopQueryRowData = DecodedShopQueryRow;
/// Alias for [`DecodedShopQueryRow`].
pub type ShopDecodedRow = DecodedShopQueryRow;

impl DecodedShopQueryRow {
    /// Return the item vnum, if this row carried a non-NULL item.
    #[must_use]
    pub fn item_vnum(&self) -> Option<u32> {
        self.item.as_ref().map(|item| item.vnum)
    }

    /// Consume the decoded row into its three logical parts.
    #[must_use]
    pub fn into_parts(self) -> (i32, u32, Option<ShopItemRecord>) {
        (self.shop_vnum, self.npc_vnum, self.item)
    }
}

/// Limits applied before allocating a packed shop section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopSectionLimits {
    /// Maximum number of distinct shop groups accepted.
    pub max_records: usize,
    /// Maximum packed section-data bytes accepted.
    pub max_data_bytes: usize,
}

impl ShopSectionLimits {
    /// Construct a record limit with no additional byte limit.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            max_records,
            max_data_bytes: usize::MAX,
        }
    }

    /// Construct both group and packed-byte limits.
    #[must_use]
    pub const fn with_data_limit(max_records: usize, max_data_bytes: usize) -> Self {
        Self {
            max_records,
            max_data_bytes,
        }
    }

    /// Alias emphasizing that records are distinct shop groups.
    #[must_use]
    pub const fn max_shops(&self) -> usize {
        self.max_records
    }
}

impl Default for ShopSectionLimits {
    fn default() -> Self {
        Self {
            max_records: SHOP_TABLE_MAX_RECORDS,
            max_data_bytes: SHOP_TABLE_MAX_SECTION_BYTES,
        }
    }
}

/// A checked base-shop section error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShopSectionError {
    /// The source returned no rows.  The legacy loader rejects this result;
    /// it is not silently converted into an empty boot table.
    EmptySource,
    /// More distinct shops were supplied than the caller permits.
    TooManyShops {
        /// Number of distinct shop groups.
        count: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// The raw joined-row input exceeded the fixed-capacity source bound.
    TooManySourceRows {
        /// Number of source rows supplied.
        count: usize,
        /// Defensive maximum derived from the distinct-shop cap.
        maximum: usize,
    },
    /// Distinct shop count cannot be represented by the boot `u16` count.
    CountOverflow {
        /// Number of distinct shop groups.
        count: usize,
    },
    /// Fixed record width cannot be represented by the boot `u16` width.
    RecordSizeOverflow {
        /// Fixed record width.
        size: usize,
    },
    /// Packed data-size arithmetic overflowed `usize`.
    DataSizeOverflow {
        /// Shop count used in the multiplication.
        count: usize,
    },
    /// Output allocation could not reserve the checked bound.
    AllocationFailed {
        /// Requested allocation size or row count.
        requested: usize,
    },
    /// Packed data exceeds the caller's byte limit.
    DataTooLarge {
        /// Required packed data length.
        length: usize,
        /// Configured byte limit.
        maximum: usize,
    },
    /// Protocol encoding returned an unexpected record width.
    RecordSizeMismatch {
        /// Zero-based output shop index.
        index: usize,
        /// Required packed width.
        expected: usize,
        /// Actual encoded width.
        actual: usize,
    },
    /// A shop attempted to append more than its fixed 40-item capacity.
    ItemCapacityOverflow {
        /// Signed source map key.
        shop_vnum: i32,
        /// Number of items after the attempted append.
        count: usize,
        /// Fixed source capacity.
        maximum: usize,
    },
    /// A source row could not be decoded.
    Row {
        /// Zero-based source row index.
        index: usize,
        /// Row error.
        source: ShopRowError,
    },
}

impl fmt::Display for ShopSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySource => formatter.write_str("shop query returned zero rows"),
            Self::TooManyShops { count, maximum } => write!(
                formatter,
                "shop section has {count} distinct shops; configured limit is {maximum}"
            ),
            Self::TooManySourceRows { count, maximum } => write!(
                formatter,
                "shop source has {count} rows; fixed-capacity limit is {maximum}"
            ),
            Self::CountOverflow { count } => {
                write!(formatter, "shop section count {count} does not fit u16")
            }
            Self::RecordSizeOverflow { size } => {
                write!(formatter, "shop record width {size} does not fit u16")
            }
            Self::DataSizeOverflow { count } => {
                write!(
                    formatter,
                    "shop section byte size overflows usize for {count} shops"
                )
            }
            Self::AllocationFailed { requested } => write!(
                formatter,
                "shop section could not allocate {requested} units"
            ),
            Self::DataTooLarge { length, maximum } => write!(
                formatter,
                "shop section data length {length} exceeds limit {maximum}"
            ),
            Self::RecordSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "shop output {index} encoded to {actual} bytes; expected {expected}"
            ),
            Self::ItemCapacityOverflow {
                shop_vnum,
                count,
                maximum,
            } => write!(
                formatter,
                "shop {shop_vnum} has {count} items; fixed capacity is {maximum}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "shop row {index} is invalid: {source}")
            }
        }
    }
}

impl Error for ShopSectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Alias emphasizing that this is a section-build error.
pub type ShopSectionBuildError = ShopSectionError;

/// An immutable checked handle for the fixed base-shop statement.
///
/// The query has no postfix or caller interpolation.  Adapters may borrow
/// this value and pass it to their own authorized row source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopTableQuery;

impl ShopTableQuery {
    /// Construct the fixed query handle.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Borrow the exact source statement.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        SHOP_TABLE_QUERY
    }

    /// Alias for [`Self::as_str`].
    #[must_use]
    pub const fn statement(&self) -> &'static str {
        SHOP_TABLE_QUERY
    }

    /// Return the exact statement without constructing a value.
    #[must_use]
    pub const fn query() -> &'static str {
        SHOP_TABLE_QUERY
    }
}

impl Default for ShopTableQuery {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Display for ShopTableQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(SHOP_TABLE_QUERY)
    }
}

/// Short alias for [`ShopTableQuery`].
pub type ShopQuery = ShopTableQuery;

/// A caller-owned source for the checked base-shop query.
pub trait ShopTableRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw rows for the checked statement in source-query order.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source error
    /// must not be represented as an empty row set.
    fn query_rows(&self, query: &ShopTableQuery) -> Result<Vec<ShopTableQueryRow>, Self::Error>;
}

impl<F, E> ShopTableRowSource for F
where
    F: Fn(&ShopTableQuery) -> Result<Vec<ShopTableQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &ShopTableQuery) -> Result<Vec<ShopTableQueryRow>, Self::Error> {
        self(query)
    }
}

/// Short alias for [`ShopTableRowSource`].
pub use ShopTableRowSource as ShopRowSource;

/// An error while obtaining or decoding base-shop rows through a loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShopTableLoadError<E> {
    /// The caller-owned source failed.
    Source(E),
    /// Raw rows failed strict or selected legacy decoding/limits.
    Rows(ShopSectionError),
}

impl<E: fmt::Display> fmt::Display for ShopTableLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "shop row source failed: {source}"),
            Self::Rows(source) => {
                write!(formatter, "shop rows could not be loaded: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for ShopTableLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Rows(source) => Some(source),
        }
    }
}

/// Short alias for [`ShopTableLoadError`].
pub type ShopLoadError<E> = ShopTableLoadError<E>;
/// Short alias for [`ShopTableLoadError`].
pub type ShopLoaderError<E> = ShopTableLoadError<E>;
/// Short alias for [`ShopTableLoadError`].
pub type ShopTableLoaderError<E> = ShopTableLoadError<E>;

/// A reusable strict/legacy shop projection builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopSectionBuilder {
    limits: ShopSectionLimits,
}

impl ShopSectionBuilder {
    /// Construct a builder with a caller-selected distinct-shop limit.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            limits: ShopSectionLimits::new(max_records),
        }
    }

    /// Construct a builder with explicit limits.
    #[must_use]
    pub const fn with_limits(limits: ShopSectionLimits) -> Self {
        Self { limits }
    }

    /// Return configured limits.
    #[must_use]
    pub const fn limits(self) -> ShopSectionLimits {
        self.limits
    }

    /// Build a strict base-shop section.
    ///
    /// # Errors
    ///
    /// Returns [`ShopSectionError`] for malformed rows, empty input, fixed
    /// capacity violations, or configured limit failures.
    pub fn build(&self, rows: &[ShopTableQueryRow]) -> Result<BootSection, ShopSectionError> {
        build_shop_section_with_limits(rows, self.limits)
    }

    /// Build a section using the explicitly named legacy policy.
    ///
    /// # Errors
    ///
    /// Returns [`ShopSectionError`] for source errors, empty input, fixed
    /// capacity violations, or configured limit failures. Numeric text is
    /// intentionally permissive in this compatibility method.
    pub fn build_legacy(
        &self,
        rows: &[ShopTableQueryRow],
    ) -> Result<BootSection, ShopSectionError> {
        build_shop_section_legacy_with_limits(rows, self.limits)
    }
}

/// Alias for [`ShopSectionBuilder`].
pub type ShopTableSectionBuilder = ShopSectionBuilder;

/// A reusable, SQL-free base-shop loader/projection policy.
///
/// The loader owns no connection and performs no state mutation. A caller
/// supplies an authorized [`ShopTableRowSource`], which receives the fixed
/// [`ShopTableQuery`] handle. This is deliberately an injected boundary: no
/// SQL client, pool, or gameplay manager is touched here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopTableLoader {
    query: ShopTableQuery,
    limits: ShopSectionLimits,
}

impl ShopTableLoader {
    /// Construct a loader with explicit limits.
    #[must_use]
    pub const fn new(limits: ShopSectionLimits) -> Self {
        Self {
            query: ShopTableQuery::new(),
            limits,
        }
    }

    /// Construct a loader with a caller-selected distinct-shop cap.
    #[must_use]
    pub const fn with_limit(max_records: usize) -> Self {
        Self::new(ShopSectionLimits::new(max_records))
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub const fn with_limits(limits: ShopSectionLimits) -> Self {
        Self::new(limits)
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub const fn from_limits(limits: ShopSectionLimits) -> Self {
        Self::new(limits)
    }

    /// Construct a loader with the default section limits.
    #[must_use]
    pub fn default_with_limits() -> Self {
        Self::new(ShopSectionLimits::default())
    }

    /// Borrow the immutable fixed-query handle.
    #[must_use]
    pub const fn query(&self) -> &ShopTableQuery {
        &self.query
    }

    /// Borrow the exact statement text.
    #[must_use]
    pub const fn statement(&self) -> &'static str {
        SHOP_TABLE_QUERY
    }

    /// Alias for [`Self::statement`].
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        SHOP_TABLE_QUERY
    }

    /// Return this loader's limits.
    #[must_use]
    pub const fn limits(&self) -> ShopSectionLimits {
        self.limits
    }

    /// Return the derived defensive raw-row cap for this loader.
    ///
    /// The result is `None` when the configured distinct-shop cap cannot fit
    /// the legacy boot `u16` count. The `SQLx` adapter uses this separately from
    /// the group limit when calling its bounded query.
    #[must_use]
    pub const fn source_row_limit(&self) -> Option<usize> {
        shop_source_row_limit(self.limits.max_records)
    }

    /// Obtain raw rows and strictly build a base-shop section.
    ///
    /// # Errors
    ///
    /// Returns [`ShopTableLoadError::Source`] for source failure or
    /// [`ShopTableLoadError::Rows`] for strict row/limit failure.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, ShopTableLoadError<S::Error>>
    where
        S: ShopTableRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ShopTableLoadError::Source)?;
        build_shop_section_with_limits(&rows, self.limits).map_err(ShopTableLoadError::Rows)
    }

    /// Obtain raw rows and build a section with the named legacy policy.
    ///
    /// # Errors
    ///
    /// Returns [`ShopTableLoadError::Source`] for source failure or
    /// [`ShopTableLoadError::Rows`] for selected row/limit failure.
    pub fn load_section_legacy<S>(
        &self,
        source: &S,
    ) -> Result<BootSection, ShopTableLoadError<S::Error>>
    where
        S: ShopTableRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ShopTableLoadError::Source)?;
        build_shop_section_legacy_with_limits(&rows, self.limits).map_err(ShopTableLoadError::Rows)
    }

    /// Project caller-supplied rows directly with the strict policy.
    ///
    /// # Errors
    ///
    /// Returns [`ShopSectionError`] for empty input, malformed rows, capacity
    /// or count overflow, allocation failure, or a configured limit failure.
    pub fn load(&self, rows: &[ShopTableQueryRow]) -> Result<BootSection, ShopSectionError> {
        build_shop_section_with_limits(rows, self.limits)
    }

    /// Alias for [`Self::load`].
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::load`].
    pub fn build(&self, rows: &[ShopTableQueryRow]) -> Result<BootSection, ShopSectionError> {
        self.load(rows)
    }

    /// Project caller-supplied rows directly with the named legacy policy.
    ///
    /// # Errors
    ///
    /// Returns [`ShopSectionError`] for empty input, source errors, capacity
    /// or count overflow, allocation failure, or a configured limit failure.
    pub fn load_legacy(&self, rows: &[ShopTableQueryRow]) -> Result<BootSection, ShopSectionError> {
        build_shop_section_legacy_with_limits(rows, self.limits)
    }

    /// Load an injected source with the strict policy.
    ///
    /// # Errors
    ///
    /// Returns [`ShopTableLoadError`] for source or section failure.
    pub fn load_source<S>(&self, source: &S) -> Result<BootSection, ShopTableLoadError<S::Error>>
    where
        S: ShopTableRowSource,
    {
        self.load_section(source)
    }
}

impl Default for ShopTableLoader {
    fn default() -> Self {
        Self::new(ShopSectionLimits::default())
    }
}

/// Short alias for [`ShopTableLoader`].
pub type ShopLoader = ShopTableLoader;

fn bounded_display(value: &str) -> String {
    const MAX: usize = 128;
    if value.len() <= MAX {
        return value.to_owned();
    }
    let mut end = MAX;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn bounded_bytes(value: &[u8]) -> String {
    const MAX: usize = 128;
    if value.len() <= MAX {
        return String::from_utf8_lossy(value).into_owned();
    }
    // Bound the conversion work as well as the resulting diagnostic. A source
    // cell is already retained by the adapter, but an invalid value must not
    // cause a second unbounded lossy-UTF-8 allocation in the error path.
    String::from_utf8_lossy(&value[..MAX]).into_owned()
}

fn check_row_width(row: &ShopTableQueryRow) -> Result<(), ShopRowError> {
    if row.columns.len() == SHOP_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(ShopRowError::ColumnCount {
            expected: SHOP_TABLE_QUERY_COLUMNS,
            actual: row.columns.len(),
        })
    }
}

fn cell_bytes(value: &ShopQueryValue, column: usize) -> Result<&[u8], ShopRowError> {
    match value {
        ShopQueryValue::Text(text) => Ok(text.as_bytes()),
        ShopQueryValue::Bytes(bytes) => Ok(bytes),
        ShopQueryValue::Null => Err(ShopRowError::Null { column }),
        ShopQueryValue::Error(message) => Err(ShopRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn is_strict_unsigned_decimal(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)
}

fn is_strict_signed_decimal(bytes: &[u8]) -> bool {
    let Some(first) = bytes.first().copied() else {
        return false;
    };
    let digits = if first == b'-' { &bytes[1..] } else { bytes };
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

fn strict_integer<T: std::str::FromStr>(
    value: &ShopQueryValue,
    column: usize,
    target: &'static str,
) -> Result<T, ShopRowError> {
    let bytes = cell_bytes(value, column)?;
    let valid = if target == "i32" {
        is_strict_signed_decimal(bytes)
    } else {
        is_strict_unsigned_decimal(bytes)
    };
    if !valid {
        return Err(ShopRowError::InvalidNumber {
            column,
            value: bounded_bytes(bytes),
            target,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ShopRowError::InvalidNumber {
        column,
        value: bounded_bytes(bytes),
        target,
    })?;
    text.parse::<T>().map_err(|_| ShopRowError::NumberOverflow {
        column,
        value: bounded_bytes(bytes),
        target,
    })
}

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Parse the decimal prefix accepted by C `strtol`/`strtoul` in base 10.
/// The magnitude saturates at `u64::MAX`; callers apply the x86 limits.
fn legacy_numeric_prefix(bytes: &[u8]) -> (bool, Option<u64>) {
    let mut index = 0;
    // The C helper receives a NUL-terminated string. Raw SQLx cells may carry
    // bytes after an embedded NUL, but those bytes are not visible to it.
    while index < bytes.len() && bytes[index] != 0 && is_c_space(bytes[index]) {
        index += 1;
    }
    let mut negative = false;
    if index < bytes.len() && bytes[index] != 0 && (bytes[index] == b'+' || bytes[index] == b'-') {
        negative = bytes[index] == b'-';
        index += 1;
    }
    let first_digit = index;
    let mut magnitude = 0_u64;
    while index < bytes.len() && bytes[index] != 0 && bytes[index].is_ascii_digit() {
        let digit = u64::from(bytes[index] - b'0');
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(digit))
            .unwrap_or(u64::MAX);
        index += 1;
    }
    if index == first_digit {
        (negative, None)
    } else {
        (negative, Some(magnitude))
    }
}

fn legacy_error(value: &ShopQueryValue, column: usize) -> Option<ShopRowError> {
    match value {
        ShopQueryValue::Error(message) => Some(ShopRowError::Source {
            column,
            message: message.clone(),
        }),
        ShopQueryValue::Text(_) | ShopQueryValue::Bytes(_) | ShopQueryValue::Null => None,
    }
}

fn legacy_cell_was_written(value: &ShopQueryValue) -> bool {
    match value {
        ShopQueryValue::Text(text) => !matches!(text.as_bytes().first(), None | Some(0)),
        ShopQueryValue::Bytes(bytes) => !matches!(bytes.first(), None | Some(0)),
        ShopQueryValue::Null | ShopQueryValue::Error(_) => false,
    }
}

fn legacy_i32(value: &ShopQueryValue, column: usize) -> Result<i32, ShopRowError> {
    if let Some(error) = legacy_error(value, column) {
        return Err(error);
    }
    let bytes = match value {
        ShopQueryValue::Text(text) => text.as_bytes(),
        ShopQueryValue::Bytes(bytes) => bytes,
        // A NULL/empty helper input leaves the zero-initialized destination.
        ShopQueryValue::Null => return Ok(0),
        ShopQueryValue::Error(_) => unreachable!(),
    };
    if bytes.is_empty() {
        return Ok(0);
    }
    let (negative, magnitude) = legacy_numeric_prefix(bytes);
    let Some(magnitude) = magnitude else {
        return Ok(0);
    };
    if negative {
        if magnitude >= 2_147_483_648 {
            Ok(i32::MIN)
        } else {
            Ok(-i32::try_from(magnitude).unwrap_or(i32::MAX))
        }
    } else if magnitude > 2_147_483_647 {
        Ok(i32::MAX)
    } else {
        Ok(i32::try_from(magnitude).unwrap_or(i32::MAX))
    }
}

fn legacy_u32(value: &ShopQueryValue, column: usize) -> Result<u32, ShopRowError> {
    if let Some(error) = legacy_error(value, column) {
        return Err(error);
    }
    let bytes = match value {
        ShopQueryValue::Text(text) => text.as_bytes(),
        ShopQueryValue::Bytes(bytes) => bytes,
        ShopQueryValue::Null => return Ok(0),
        ShopQueryValue::Error(_) => unreachable!(),
    };
    if bytes.is_empty() {
        return Ok(0);
    }
    let (negative, magnitude) = legacy_numeric_prefix(bytes);
    let Some(magnitude) = magnitude else {
        return Ok(0);
    };
    if negative {
        // On the legacy x86 target, `strtoul` returns ULONG_MAX when the
        // magnitude exceeds ULONG_MAX. Do not reduce that magnitude to
        // UINT32_MAX and then negate it: `-2^32` would incorrectly become
        // `1`. Values within the x86 range use the usual modulo conversion.
        if magnitude > u64::from(u32::MAX) {
            Ok(u32::MAX)
        } else {
            let reduced = u32::try_from(magnitude).unwrap_or(u32::MAX);
            Ok(0_u32.wrapping_sub(reduced))
        }
    } else if magnitude > u64::from(u32::MAX) {
        Ok(u32::MAX)
    } else {
        Ok(u32::try_from(magnitude).unwrap_or(u32::MAX))
    }
}

fn legacy_u16(value: &ShopQueryValue, column: usize) -> Result<u16, ShopRowError> {
    let value = legacy_u32(value, column)?;
    // The source narrows with an x86 unsigned-short cast. Masking the low
    // bits expresses that modulo conversion without a Clippy-truncating cast.
    Ok(u16::try_from(value & u32::from(u16::MAX)).unwrap())
}

fn make_item(vnum: u32, count: u16) -> ShopItemRecord {
    let mut item = ShopItemRecord::zeroed();
    item.vnum = vnum;
    item.count = count;
    // `TItemPos()` in the legacy base-shop item constructor uses INVENTORY
    // and WORD_MAX, rather than a packed zero position.
    item.pos = ShopItemPosition::new(SHOP_ITEM_DEFAULT_WINDOW_TYPE, SHOP_ITEM_DEFAULT_CELL);
    item
}

/// Strictly decode one base-shop row.
///
/// The signed shop key is retained for map projection.  Strict numeric cells
/// require complete decimal text (a leading `-` is accepted only for the shop
/// key), reject `NULL`, source errors, malformed bytes, and target overflow.
///
/// # Errors
///
/// Returns [`ShopRowError`] for a wrong-width row or invalid cell.
pub fn decode_shop_query_row(row: &ShopTableQueryRow) -> Result<DecodedShopQueryRow, ShopRowError> {
    check_row_width(row)?;
    let shop_vnum = strict_integer::<i32>(&row.columns[0], 0, "i32")?;
    let npc_vnum = strict_integer::<u32>(&row.columns[1], 1, "u32")?;
    let item_vnum = strict_integer::<u32>(&row.columns[2], 2, "u32")?;
    let item_count = strict_integer::<u16>(&row.columns[3], 3, "u16")?;
    Ok(DecodedShopQueryRow {
        shop_vnum,
        npc_vnum,
        item: Some(make_item(item_vnum, item_count)),
    })
}

/// Decode one row using the explicitly named legacy `str_to_number` policy.
///
/// Numeric cells accept C whitespace, an optional sign, and a decimal prefix;
/// x86 `strtol`/`strtoul` overflow and destination casts are modeled.  A
/// numeric `NULL` or empty cell leaves the source's zero-initialized scalar
/// unchanged.  A `NULL` item-vnum cell skips the complete row before its count
/// cell is inspected, matching the C++ LEFT JOIN path.
///
/// # Errors
///
/// Returns [`ShopRowError::ColumnCount`] for a wrong-width row and
/// [`ShopRowError::Source`] for a source error. Numeric text does not produce
/// a syntax error in this compatibility policy.
pub fn decode_shop_query_row_legacy(
    row: &ShopTableQueryRow,
) -> Result<DecodedShopQueryRow, ShopRowError> {
    check_row_width(row)?;
    let shop_vnum = legacy_i32(&row.columns[0], 0)?;
    let npc_vnum = legacy_u32(&row.columns[1], 1)?;
    if matches!(row.columns[2], ShopQueryValue::Null) {
        return Ok(DecodedShopQueryRow {
            shop_vnum,
            npc_vnum,
            item: None,
        });
    }
    let item_vnum = legacy_u32(&row.columns[2], 2)?;
    let item_count = legacy_u16(&row.columns[3], 3)?;
    Ok(DecodedShopQueryRow {
        shop_vnum,
        npc_vnum,
        item: Some(make_item(item_vnum, item_count)),
    })
}

/// Alias emphasizing that the compatibility policy is opt-in.
///
/// # Errors
///
/// Returns [`ShopRowError`] for a wrong-width row or source error.
pub fn decode_shop_query_row_compat(
    row: &ShopTableQueryRow,
) -> Result<DecodedShopQueryRow, ShopRowError> {
    decode_shop_query_row_legacy(row)
}

#[derive(Debug)]
struct ShopGroup {
    npc_vnum: u32,
    items: Vec<ShopItemRecord>,
}

fn validate_section_size(
    count: usize,
    limits: ShopSectionLimits,
) -> Result<(u16, u16, usize), ShopSectionError> {
    if count > limits.max_records {
        return Err(ShopSectionError::TooManyShops {
            count,
            maximum: limits.max_records,
        });
    }
    let wire_count = u16::try_from(count).map_err(|_| ShopSectionError::CountOverflow { count })?;
    let record_size =
        u16::try_from(SHOP_TABLE_WIRE_SIZE).map_err(|_| ShopSectionError::RecordSizeOverflow {
            size: SHOP_TABLE_WIRE_SIZE,
        })?;
    let data_len = SHOP_TABLE_WIRE_SIZE
        .checked_mul(count)
        .ok_or(ShopSectionError::DataSizeOverflow { count })?;
    if data_len > limits.max_data_bytes {
        return Err(ShopSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }
    Ok((wire_count, record_size, data_len))
}

/// Validate the count/width/data arithmetic without allocating or reading
/// rows.  This is useful to callers that need a fail-closed preflight.
///
/// # Errors
///
/// Returns [`ShopSectionError`] for configured limits, `u16` count/width
/// overflow, or checked `usize` data-size overflow.
pub fn checked_shop_section_size(
    count: usize,
    limits: ShopSectionLimits,
) -> Result<(u16, u16, usize), ShopSectionError> {
    validate_section_size(count, limits)
}

fn append_projection(
    rows: &[ShopTableQueryRow],
    limits: ShopSectionLimits,
    legacy: bool,
) -> Result<BootSection, ShopSectionError> {
    if rows.is_empty() {
        return Err(ShopSectionError::EmptySource);
    }
    // A joined query normally contributes at most the fixed item capacity per
    // distinct shop. Reject an excessive injected source before decoding or
    // allocating group nodes. A zero group cap is handled by the per-group
    // check below so its error remains `TooManyShops`.
    if limits.max_records > 0 {
        // An unrepresentable group cap cannot derive a narrower raw bound;
        // fall back to the global defensive ceiling rather than leaving the
        // injected source unbounded.
        let maximum =
            shop_source_row_limit(limits.max_records).unwrap_or(SHOP_TABLE_MAX_SOURCE_ROWS);
        if rows.len() > maximum {
            return Err(ShopSectionError::TooManySourceRows {
                count: rows.len(),
                maximum,
            });
        }
    }

    // `BTreeMap` has no stable fallible reservation API.  Group nodes are
    // bounded by the source row count and each item vector below still uses
    // checked reservation, so allocation failure is handled before output
    // encoding.
    let mut groups: BTreeMap<i32, ShopGroup> = BTreeMap::new();

    for (index, row) in rows.iter().enumerate() {
        let decoded = if legacy {
            decode_shop_query_row_legacy(row)
        } else {
            decode_shop_query_row(row)
        }
        .map_err(|source| ShopSectionError::Row { index, source })?;

        let was_present = groups.contains_key(&decoded.shop_vnum);
        if !was_present && groups.len() >= limits.max_records {
            return Err(ShopSectionError::TooManyShops {
                count: groups.len() + 1,
                maximum: limits.max_records,
            });
        }
        let group = groups
            .entry(decoded.shop_vnum)
            .or_insert_with(|| ShopGroup {
                npc_vnum: decoded.npc_vnum,
                items: Vec::new(),
            });
        // `str_to_number` leaves its destination unchanged for NULL/empty
        // input. The source therefore keeps the prior NPC value on a later
        // duplicate row; only a non-empty cell replaces it. The strict policy
        // has already rejected NULL and always assigns the parsed value.
        if !legacy || legacy_cell_was_written(&row.columns()[1]) {
            group.npc_vnum = decoded.npc_vnum;
        }
        if let Some(item) = decoded.item {
            if group.items.len() >= SHOP_ITEM_MAX_NUM {
                return Err(ShopSectionError::ItemCapacityOverflow {
                    shop_vnum: decoded.shop_vnum,
                    count: group.items.len() + 1,
                    maximum: SHOP_ITEM_MAX_NUM,
                });
            }
            group
                .items
                .try_reserve(1)
                .map_err(|_| ShopSectionError::AllocationFailed { requested: 1 })?;
            group.items.push(item);
        }
    }

    let (count, record_size, data_len) = validate_section_size(groups.len(), limits)?;
    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| ShopSectionError::AllocationFailed {
            requested: data_len,
        })?;

    for (index, (shop_vnum, group)) in groups.into_iter().enumerate() {
        let mut record = ShopTableRecord::zeroed_base_shop();
        // Keep the source constructor semantics even while protocol versions
        // that predate the TItemPos correction are still in the dependency
        // graph.  Every slot, including unused slots, has this default pos.
        for slot in &mut record.items {
            slot.pos = ShopItemPosition::new(SHOP_ITEM_DEFAULT_WINDOW_TYPE, SHOP_ITEM_DEFAULT_CELL);
        }
        record.vnum = u32::from_le_bytes(shop_vnum.to_le_bytes());
        record.npc_vnum = group.npc_vnum;
        record.item_count = u8::try_from(group.items.len()).map_err(|_| {
            ShopSectionError::ItemCapacityOverflow {
                shop_vnum,
                count: group.items.len(),
                maximum: SHOP_ITEM_MAX_NUM,
            }
        })?;
        for (slot, item) in record.items.iter_mut().zip(group.items) {
            *slot = item;
        }
        let encoded = RecordCodec::encode(&record);
        if encoded.len() != SHOP_TABLE_WIRE_SIZE {
            return Err(ShopSectionError::RecordSizeMismatch {
                index,
                expected: SHOP_TABLE_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }

    debug_assert_eq!(data.len(), data_len);
    Ok(BootSection {
        kind: BootSectionKind::Shop,
        record_size,
        count,
        data,
    })
}

/// Build a strict base-shop section with default limits.
///
/// # Errors
///
/// Returns [`ShopSectionError`] for empty input, an invalid row, a fixed
/// item-capacity violation, or a configured limit failure.
pub fn build_shop_section(rows: &[ShopTableQueryRow]) -> Result<BootSection, ShopSectionError> {
    build_shop_section_with_limits(rows, ShopSectionLimits::default())
}

/// Build a strict base-shop section with a caller-selected shop limit.
///
/// # Errors
///
/// Returns [`ShopSectionError`] for empty input, an invalid row, or a limit
/// failure.
pub fn build_shop_section_with_limit(
    rows: &[ShopTableQueryRow],
    max_records: usize,
) -> Result<BootSection, ShopSectionError> {
    build_shop_section_with_limits(rows, ShopSectionLimits::new(max_records))
}

/// Build a strict typed base-shop section with explicit limits.
///
/// Rows are grouped by signed shop key and emitted in ascending key order.
/// Item rows within each shop retain source order and duplicates.  The section
/// always declares the exact 2,762-byte width.
///
/// # Errors
///
/// Returns [`ShopSectionError`] for invalid rows, empty input, capacity or
/// count overflow, allocation failure, or configured limit failure.
pub fn build_shop_section_with_limits(
    rows: &[ShopTableQueryRow],
    limits: ShopSectionLimits,
) -> Result<BootSection, ShopSectionError> {
    append_projection(rows, limits, false)
}

/// Build a base-shop section using the explicitly named legacy policy.
///
/// # Errors
///
/// Returns [`ShopSectionError`] for empty input, source errors, item-capacity
/// violation, count/width/size overflow, allocation failure, or configured
/// limit failure.
pub fn build_shop_section_legacy(
    rows: &[ShopTableQueryRow],
) -> Result<BootSection, ShopSectionError> {
    build_shop_section_legacy_with_limits(rows, ShopSectionLimits::default())
}

/// Build a legacy-policy section with a caller-selected shop limit.
///
/// # Errors
///
/// Returns [`ShopSectionError`] for empty input, source errors, or a limit
/// failure.
pub fn build_shop_section_legacy_with_limit(
    rows: &[ShopTableQueryRow],
    max_records: usize,
) -> Result<BootSection, ShopSectionError> {
    build_shop_section_legacy_with_limits(rows, ShopSectionLimits::new(max_records))
}

/// Build a legacy-policy section with explicit limits.
///
/// # Errors
///
/// Returns [`ShopSectionError`] for source errors, empty input, capacity or
/// count overflow, allocation failure, or configured limit failure.
pub fn build_shop_section_legacy_with_limits(
    rows: &[ShopTableQueryRow],
    limits: ShopSectionLimits,
) -> Result<BootSection, ShopSectionError> {
    append_projection(rows, limits, true)
}

/// Load rows from an injected source and build a strict base-shop section.
///
/// # Errors
///
/// Returns [`ShopLoadError::Source`] for a source failure and
/// [`ShopTableLoadError::Rows`] for an empty or invalid projection. A source
/// failure is never represented as an empty table.
pub fn load_shop_section<S>(
    source: &S,
    limits: ShopSectionLimits,
) -> Result<BootSection, ShopLoadError<S::Error>>
where
    S: ShopTableRowSource,
{
    let query = ShopTableQuery::new();
    let rows = source
        .query_rows(&query)
        .map_err(ShopTableLoadError::Source)?;
    build_shop_section_with_limits(&rows, limits).map_err(ShopTableLoadError::Rows)
}

/// Load rows and build a strict section with a caller-selected shop limit.
///
/// # Errors
///
/// Returns [`ShopLoadError`] for a source or section failure.
pub fn load_shop_section_with_limit<S>(
    source: &S,
    max_records: usize,
) -> Result<BootSection, ShopLoadError<S::Error>>
where
    S: ShopTableRowSource,
{
    load_shop_section(source, ShopSectionLimits::new(max_records))
}

/// Load rows and build a section using the explicitly named legacy policy.
///
/// # Errors
///
/// Returns [`ShopLoadError`] for a source or section failure.
pub fn load_shop_section_legacy<S>(
    source: &S,
    limits: ShopSectionLimits,
) -> Result<BootSection, ShopLoadError<S::Error>>
where
    S: ShopTableRowSource,
{
    let query = ShopTableQuery::new();
    let rows = source
        .query_rows(&query)
        .map_err(ShopTableLoadError::Source)?;
    build_shop_section_legacy_with_limits(&rows, limits).map_err(ShopTableLoadError::Rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_records::ShopItemAttribute;

    fn text(value: impl Into<String>) -> ShopQueryValue {
        ShopQueryValue::text(value)
    }

    fn row(values: [&str; SHOP_TABLE_QUERY_COLUMNS]) -> ShopTableQueryRow {
        ShopTableQueryRow::new(values.into_iter().map(text))
    }

    fn item_row(shop: i32, npc: u32, item: u32, count: u16) -> ShopTableQueryRow {
        ShopTableQueryRow::new([
            text(shop.to_string()),
            text(npc.to_string()),
            text(item.to_string()),
            text(count.to_string()),
        ])
    }

    fn decode_records(section: &BootSection) -> Vec<ShopTableRecord> {
        assert_eq!(section.kind, BootSectionKind::Shop);
        assert_eq!(usize::from(section.record_size), SHOP_TABLE_WIRE_SIZE);
        assert_eq!(
            section.data.len(),
            SHOP_TABLE_WIRE_SIZE * usize::from(section.count)
        );
        section
            .data
            .chunks_exact(SHOP_TABLE_WIRE_SIZE)
            .map(ShopTableRecord::decode)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn query_and_wire_widths_are_exact() {
        assert_eq!(
            SHOP_TABLE_QUERY,
            "SELECT shop.vnum, shop.npc_vnum, shop_item.item_vnum, shop_item.count FROM shop LEFT JOIN shop_item ON shop.vnum = shop_item.shop_vnum ORDER BY shop.vnum, shop_item.item_vnum"
        );
        assert_eq!(SHOP_ITEM_WIRE_SIZE, 68);
        assert_eq!(SHOP_TABLE_WIRE_SIZE, 2_762);
        assert_eq!(SHOP_SECTION_RECORD_SIZE, 2_762);
        assert_eq!(SHOP_TABLE_QUERY_COLUMNS, 4);
        assert_eq!(SHOP_TABLE_MAX_SOURCE_ROWS, 2_621_400);
        assert_eq!(shop_source_row_limit(1), Some(SHOP_ITEM_MAX_NUM));
        assert_eq!(
            shop_source_row_limit(SHOP_TABLE_MAX_RECORDS),
            Some(SHOP_TABLE_MAX_SOURCE_ROWS)
        );
        assert_eq!(shop_source_row_limit(SHOP_TABLE_MAX_RECORDS + 1), None);
        assert_eq!(ShopTableLoader::with_limit(1).source_row_limit(), Some(40));
        assert_eq!(ShopTableLoader::with_limit(0).source_row_limit(), Some(0));
    }

    #[test]
    fn legacy_negative_unsigned_overflow_uses_x86_strtoul_saturation() {
        assert_eq!(legacy_u32(&text("-1"), 1).unwrap(), u32::MAX);
        assert_eq!(legacy_u32(&text("-4294967295"), 1).unwrap(), 1);
        assert_eq!(legacy_u32(&text("-4294967296"), 1).unwrap(), u32::MAX);
        assert_eq!(legacy_u32(&text("-4294967297"), 1).unwrap(), u32::MAX);
        assert_eq!(legacy_u32(&text("4294967296"), 1).unwrap(), u32::MAX);
        assert_eq!(
            legacy_u32(&ShopQueryValue::bytes([b'1', 0, b'9']), 1).unwrap(),
            1
        );
        assert_eq!(legacy_i32(&ShopQueryValue::bytes([0, b'9']), 0).unwrap(), 0);
    }

    #[test]
    fn legacy_unsigned_overflow_reaches_npc_item_and_count_fields() {
        let npc = decode_records(
            &build_shop_section_legacy(&[row(["1", "-4294967296", "10", "1"])]).unwrap(),
        );
        assert_eq!(npc[0].npc_vnum, u32::MAX);

        let item = decode_records(
            &build_shop_section_legacy(&[row(["1", "2", "-4294967297", "1"])]).unwrap(),
        );
        assert_eq!(item[0].items[0].vnum, u32::MAX);

        let count = decode_records(
            &build_shop_section_legacy(&[row(["1", "2", "10", "-4294967296"])]).unwrap(),
        );
        assert_eq!(count[0].items[0].count, u16::MAX);
    }

    #[test]
    fn strict_and_legacy_values_are_distinct() {
        let strict = decode_shop_query_row(&row(["-7", "42", "100", "9"])).unwrap();
        assert_eq!(strict.shop_vnum, -7);
        assert_eq!(strict.npc_vnum, 42);
        assert_eq!(strict.item.unwrap().vnum, 100);

        let legacy =
            decode_shop_query_row_legacy(&row([" -8junk", " 42xyz", " 100abc", " 9tail"])).unwrap();
        assert_eq!(legacy.shop_vnum, -8);
        assert_eq!(legacy.npc_vnum, 42);
        let item = legacy.item.unwrap();
        assert_eq!((item.vnum, item.count), (100, 9));
    }

    #[test]
    fn strict_rejects_null_and_legacy_skips_null_item() {
        let strict = ShopTableQueryRow::new([
            text("1"),
            text("2"),
            ShopQueryValue::null(),
            ShopQueryValue::error("must not be read"),
        ]);
        assert!(matches!(
            decode_shop_query_row(&strict),
            Err(ShopRowError::Null { column: 2 })
        ));

        let legacy = decode_shop_query_row_legacy(&strict).unwrap();
        assert_eq!(legacy.shop_vnum, 1);
        assert_eq!(legacy.npc_vnum, 2);
        assert!(legacy.item.is_none());
    }

    #[test]
    fn legacy_null_npc_keeps_the_previous_destination_value() {
        let rows = vec![
            item_row(1, 42, 10, 1),
            ShopTableQueryRow::new([
                text("1"),
                ShopQueryValue::null(),
                ShopQueryValue::null(),
                ShopQueryValue::null(),
            ]),
            ShopTableQueryRow::new([
                text("1"),
                ShopQueryValue::bytes([0, b'4']),
                ShopQueryValue::null(),
                ShopQueryValue::null(),
            ]),
        ];
        let records = decode_records(&build_shop_section_legacy(&rows).unwrap());
        assert_eq!(records[0].npc_vnum, 42);
        assert_eq!(records[0].item_count, 1);
    }

    #[test]
    fn map_projection_orders_signed_keys_and_repeats_npc_parse() {
        let rows = vec![
            item_row(10, 1, 5, 1),
            item_row(-2, 2, 99, 1),
            item_row(10, 3, 2, 2),
            item_row(10, 3, 4, 3),
        ];
        let section = build_shop_section(&rows).unwrap();
        let records = decode_records(&section);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].vnum, u32::from_le_bytes((-2_i32).to_le_bytes()));
        assert_eq!(records[0].item_count, 1);
        assert_eq!(records[0].items[0].vnum, 99);
        assert_eq!(records[1].vnum, 10);
        assert_eq!(records[1].npc_vnum, 3);
        assert_eq!(records[1].item_count, 3);
        assert_eq!(records[1].items[0].vnum, 5);
        assert_eq!(records[1].items[1].vnum, 2);
        assert_eq!(records[1].items[2].vnum, 4);
    }

    #[test]
    fn forty_item_boundary_is_explicit() {
        let rows: Vec<_> = (0..SHOP_ITEM_MAX_NUM)
            .map(|item| item_row(7, 8, u32::try_from(item).unwrap(), 1))
            .collect();
        let section = build_shop_section(&rows).unwrap();
        assert_eq!(decode_records(&section)[0].item_count, 40);

        let overflow: Vec<_> = (0..=SHOP_ITEM_MAX_NUM)
            .map(|item| item_row(7, 8, u32::try_from(item).unwrap(), 1))
            .collect();
        assert!(matches!(
            build_shop_section(&overflow),
            Err(ShopSectionError::ItemCapacityOverflow {
                shop_vnum: 7,
                count: 41,
                maximum: 40
            })
        ));
    }

    #[test]
    fn checked_count_overflow_is_reported_before_encoding() {
        let error =
            checked_shop_section_size(65_536, ShopSectionLimits::new(usize::MAX)).unwrap_err();
        assert_eq!(error, ShopSectionError::CountOverflow { count: 65_536 });
        assert_eq!(
            checked_shop_section_size(usize::MAX, ShopSectionLimits::new(usize::MAX)).unwrap_err(),
            ShopSectionError::CountOverflow { count: usize::MAX }
        );
    }

    #[test]
    fn unused_slots_use_source_constructor_semantics() {
        let section = build_shop_section(&[item_row(1, 2, 3, 4)]).unwrap();
        let record = decode_records(&section).remove(0);
        let unused = record.items[1];
        assert_eq!(unused.vnum, 0);
        assert_eq!(unused.count, 0);
        assert_eq!(unused.price, 0);
        assert_eq!(unused.display_pos, 0);
        assert_eq!(unused.price_type, SHOPEX_GOLD);
        assert_eq!(unused.price_vnum, 0);
        assert_eq!(unused.pos.window_type, 1);
        assert_eq!(unused.pos.cell, u16::MAX);
        assert_eq!(unused.sockets, [0; 6]);
        assert!(unused
            .attrs
            .iter()
            .all(|attr: &ShopItemAttribute| attr.attr_type == 0 && attr.value == 0));
        assert_eq!(record.shop_name, [0; SHOP_NAME_BYTES]);
    }

    #[test]
    fn empty_source_is_not_an_empty_section() {
        assert_eq!(
            build_shop_section(&[]).unwrap_err(),
            ShopSectionError::EmptySource
        );
        assert_eq!(
            build_shop_section_legacy(&[]).unwrap_err(),
            ShopSectionError::EmptySource
        );
    }

    #[test]
    fn wrong_width_and_raw_cell_facts_are_safe() {
        for width in 0..SHOP_TABLE_QUERY_COLUMNS {
            let cells = (0..width)
                .map(|index| if index == 0 { text("1") } else { text("2") })
                .collect::<Vec<_>>();
            let row = ShopTableQueryRow::new(cells);
            assert!(matches!(
                decode_shop_query_row(&row),
                Err(ShopRowError::ColumnCount { expected: 4, actual }) if actual == width
            ));
        }
        let raw = ShopTableQueryRow::new([
            ShopQueryValue::bytes([0xff, b'1']),
            ShopQueryValue::null(),
            ShopQueryValue::text("2"),
            ShopQueryValue::Error("source".into()),
        ]);
        assert!(matches!(
            decode_shop_query_row(&raw),
            Err(ShopRowError::InvalidNumber { column: 0, .. })
        ));
        assert!(matches!(
            decode_shop_query_row_legacy(&raw),
            Err(ShopRowError::Source { column: 3, .. })
        ));
        assert_eq!(raw.columns()[1], ShopQueryValue::Null);
    }

    #[test]
    fn loader_owns_query_and_injects_source_rows() {
        let loader = ShopTableLoader::new(ShopSectionLimits::new(7));
        assert_eq!(loader.query().as_str(), SHOP_TABLE_QUERY);
        assert_eq!(loader.limits().max_records, 7);
        let source = |query: &ShopTableQuery| {
            assert_eq!(query.as_str(), SHOP_TABLE_QUERY);
            Ok::<_, String>(vec![item_row(4, 9, 11, 2)])
        };
        let section = loader.load_section(&source).unwrap();
        assert_eq!(section.kind, BootSectionKind::Shop);
        assert_eq!(section.count, 1);
        assert_eq!(decode_records(&section)[0].items[0].vnum, 11);
    }

    #[test]
    fn loader_keeps_source_failures_typed() {
        let loader = ShopTableLoader::default();
        let source = |_query: &ShopTableQuery| Err::<Vec<ShopTableQueryRow>, _>("db unavailable");
        assert!(matches!(
            loader.load_section(&source),
            Err(ShopTableLoadError::Source(message)) if message == "db unavailable"
        ));
        let legacy_source =
            |_query: &ShopTableQuery| Ok::<_, String>(vec![row(["1", "2", "junk", "4"])]);
        assert!(matches!(
            loader.load_section_legacy(&legacy_source),
            Ok(section) if section.count == 1
        ));
    }

    #[test]
    fn limits_and_allocation_are_checked_before_output() {
        let rows = [item_row(1, 2, 3, 4)];
        assert!(matches!(
            build_shop_section_with_limit(&rows, 0),
            Err(ShopSectionError::TooManyShops {
                count: 1,
                maximum: 0
            })
        ));
        assert!(matches!(
            build_shop_section_with_limit(&[], 0),
            Err(ShopSectionError::EmptySource)
        ));
        assert!(matches!(
            build_shop_section_with_limits(
                &rows,
                ShopSectionLimits::with_data_limit(1, SHOP_TABLE_WIRE_SIZE - 1),
            ),
            Err(ShopSectionError::DataTooLarge { .. })
        ));
    }

    #[test]
    fn source_row_bound_allows_joined_items_but_rejects_excess_null_rows() {
        let joined = vec![item_row(1, 2, 10, 1), item_row(1, 2, 11, 1)];
        let section = build_shop_section_with_limit(&joined, 1).unwrap();
        assert_eq!(section.count, 1);

        let null_rows = (0..41).map(|_| {
            ShopTableQueryRow::new([
                text("1"),
                text("2"),
                ShopQueryValue::null(),
                ShopQueryValue::null(),
            ])
        });
        let null_rows = null_rows.collect::<Vec<_>>();
        assert!(matches!(
            build_shop_section_legacy_with_limit(&null_rows, 1),
            Err(ShopSectionError::TooManySourceRows {
                count: 41,
                maximum: 40
            })
        ));
    }
}
