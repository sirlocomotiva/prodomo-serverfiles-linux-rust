//! Pure, SQL-free conversion of the source-fixed `object_proto` boot table.
//!
//! The legacy loader selects thirteen cells in this exact order:
//! `vnum`, `price`, `materials`, `upgrade_vnum`, `upgrade_limit_time`, `life`,
//! `reg_1`, `reg_2`, `reg_3`, `reg_4`, `npc`, `group_vnum`, and
//! `dependent_group`. It requests `ORDER BY vnum`, appends rows in the order
//! returned by the source, and does not remove duplicate prototypes. This
//! module preserves that order and every duplicate.
//!
//! [`decode_object_proto_query_row`] is the strict default. It requires whole
//! unsigned decimal material values, decimal scalar integers, a canonical
//! `item,count/item,count` material list, and a coordinate addition that fits
//! `i32`. [`decode_object_proto_query_row_legacy`] is the separately named
//! compatibility policy for the source's permissive numeric prefixes,
//! zero-initialized SQL `NULL` scalar destinations, x86 casts, and material
//! parsing. A SQL `NULL` materials cell remains an error in both policies
//! because the legacy parser passes it directly to `strchr` instead of
//! supporting a null pointer.
//!
//! This file does not execute SQL, acquire a live result set, mutate a manager,
//! deduplicate rows, or claim live boot parity.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootSection, BootSectionKind};
use protocol::db_records::{ObjectMaterial, ObjectProtoRecord, OBJECT_PROTO_RECORD_WIRE_SIZE};

/// Number of columns selected by the fixed legacy object-prototype query.
pub const OBJECT_PROTO_TABLE_QUERY_COLUMNS: usize = 13;

/// Alias for [`OBJECT_PROTO_TABLE_QUERY_COLUMNS`].
pub const OBJECT_PROTO_QUERY_COLUMN_COUNT: usize = OBJECT_PROTO_TABLE_QUERY_COLUMNS;

/// Number of material slots in the active `building::TObjectProto` layout.
pub const OBJECT_PROTO_MATERIAL_MAX_NUM: usize = 5;

/// Maximum number of records representable by the legacy `u16` section count.
pub const OBJECT_PROTO_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Exact fixed 96-byte x86 `building::TObjectProto` section width.
pub const OBJECT_PROTO_SECTION_RECORD_SIZE: u16 = 96;

/// Alias for the source-fixed record width.
pub const OBJECT_PROTO_TABLE_WIRE_SIZE: usize = OBJECT_PROTO_RECORD_WIRE_SIZE;

/// Maximum byte length of a section when every representable count is used.
pub const OBJECT_PROTO_TABLE_MAX_SECTION_BYTES: usize =
    OBJECT_PROTO_RECORD_WIRE_SIZE * OBJECT_PROTO_TABLE_MAX_RECORDS;

/// Column names in the fixed legacy query order.
pub const OBJECT_PROTO_TABLE_QUERY_COLUMN_NAMES: [&str; OBJECT_PROTO_TABLE_QUERY_COLUMNS] = [
    "vnum",
    "price",
    "materials",
    "upgrade_vnum",
    "upgrade_limit_time",
    "life",
    "reg_1",
    "reg_2",
    "reg_3",
    "reg_4",
    "npc",
    "group_vnum",
    "dependent_group",
];

/// One raw value returned by an object-prototype query-row adapter.
///
/// `Text` is the only value accepted by strict scalar and material decoding.
/// `Null` and `Error` retain distinct source facts until a caller selects a
/// conversion policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectProtoQueryValue {
    /// A non-NULL textual value, normally the original SQL result string.
    Text(String),
    /// A SQL `NULL` value.
    Null,
    /// An error raised while obtaining or converting one query column.
    Error(String),
}

impl ObjectProtoQueryValue {
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

impl From<String> for ObjectProtoQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for ObjectProtoQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

/// A row-shaped value for the thirteen-column object-prototype query.
///
/// The vector retains malformed source width so decoders can report it without
/// indexing past the input. [`Self::try_new`] validates the exact shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectProtoTableQueryRow {
    columns: Vec<ObjectProtoQueryValue>,
}

impl ObjectProtoTableQueryRow {
    /// Construct a row while retaining supplied values and their order.
    #[must_use]
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ObjectProtoQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub fn from_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = ObjectProtoQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct and validate a thirteen-column row.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectProtoRowError::ColumnCount`] when the iterator does not
    /// produce exactly thirteen values.
    pub fn try_new<I>(columns: I) -> Result<Self, ObjectProtoRowError>
    where
        I: IntoIterator<Item = ObjectProtoQueryValue>,
    {
        let row = Self::new(columns);
        if row.columns.len() != OBJECT_PROTO_TABLE_QUERY_COLUMNS {
            return Err(ObjectProtoRowError::ColumnCount {
                expected: OBJECT_PROTO_TABLE_QUERY_COLUMNS,
                actual: row.columns.len(),
            });
        }
        Ok(row)
    }

    /// Construct a row from a statically sized, correctly shaped column set.
    #[must_use]
    pub fn from_typed_columns(
        columns: [ObjectProtoQueryValue; OBJECT_PROTO_TABLE_QUERY_COLUMNS],
    ) -> Self {
        Self::new(columns)
    }

    /// Borrow all supplied columns in query order.
    #[must_use]
    pub fn columns(&self) -> &[ObjectProtoQueryValue] {
        &self.columns
    }

    /// Consume the row and return its columns in query order.
    #[must_use]
    pub fn into_columns(self) -> Vec<ObjectProtoQueryValue> {
        self.columns
    }

    /// Return the number of columns supplied by the source.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

impl From<[ObjectProtoQueryValue; OBJECT_PROTO_TABLE_QUERY_COLUMNS]> for ObjectProtoTableQueryRow {
    fn from(columns: [ObjectProtoQueryValue; OBJECT_PROTO_TABLE_QUERY_COLUMNS]) -> Self {
        Self::from_typed_columns(columns)
    }
}

impl TryFrom<Vec<ObjectProtoQueryValue>> for ObjectProtoTableQueryRow {
    type Error = ObjectProtoRowError;

    fn try_from(columns: Vec<ObjectProtoQueryValue>) -> Result<Self, Self::Error> {
        Self::try_new(columns)
    }
}

/// Short alias for an object-prototype query row.
pub type ObjectProtoQueryRow = ObjectProtoTableQueryRow;
/// Short alias for one object-prototype query-cell value.
pub type ObjectProtoQueryColumn = ObjectProtoQueryValue;

/// Return a stable column name for diagnostics.
#[must_use]
pub const fn object_proto_query_column_name(index: usize) -> &'static str {
    match index {
        0 => "vnum",
        1 => "price",
        2 => "materials",
        3 => "upgrade_vnum",
        4 => "upgrade_limit_time",
        5 => "life",
        6 => "reg_1",
        7 => "reg_2",
        8 => "reg_3",
        9 => "reg_4",
        10 => "npc",
        11 => "group_vnum",
        12 => "dependent_group",
        _ => "unknown",
    }
}

/// A strict or explicitly selected compatibility error in one object-proto row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectProtoRowError {
    /// The row did not contain exactly thirteen columns.
    ColumnCount {
        /// Required column count.
        expected: usize,
        /// Supplied column count.
        actual: usize,
    },
    /// A required value was SQL `NULL` in the strict decoder.
    Null {
        /// Zero-based query column index.
        column: usize,
    },
    /// A required value could not be obtained or converted by the source.
    Source {
        /// Zero-based query column index.
        column: usize,
        /// Source diagnostic.
        message: String,
    },
    /// A non-NULL scalar was not a strict decimal integer.
    InvalidNumber {
        /// Zero-based query column index.
        column: usize,
        /// Original value.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A syntactically valid scalar did not fit its target type.
    NumberOverflow {
        /// Zero-based query column index.
        column: usize,
        /// Original value.
        value: String,
        /// Target integer type.
        target: &'static str,
    },
    /// A strict material pair did not have exactly two decimal components.
    InvalidMaterialFormat {
        /// Original complete material value.
        value: String,
        /// Zero-based material pair index.
        pair_index: usize,
    },
    /// Strict decoding refused to discard material pairs beyond the fixed array.
    TooManyMaterials {
        /// Number of pairs in the source value.
        count: usize,
        /// Number of representable material slots.
        maximum: usize,
    },
    /// A strict material component was not an unsigned decimal integer.
    InvalidMaterialNumber {
        /// Zero-based material pair index.
        pair_index: usize,
        /// `item_vnum` or `count`.
        component: &'static str,
        /// Original component text.
        value: String,
    },
    /// A syntactically valid material component did not fit `u32`.
    MaterialNumberOverflow {
        /// Zero-based material pair index.
        pair_index: usize,
        /// `item_vnum` or `count`.
        component: &'static str,
        /// Original component text.
        value: String,
    },
    /// Strict NPC Y derivation overflowed signed 32-bit arithmetic.
    NpcYOverflow {
        /// `regions[1]` used by the derivation.
        region_y1: i32,
        /// `regions[3]` used by the derivation.
        region_y3: i32,
    },
}

impl fmt::Display for ObjectProtoRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => write!(
                formatter,
                "object_proto query row has {actual} columns; expected {expected}"
            ),
            Self::Null { column } => write!(
                formatter,
                "object_proto query column {} is NULL",
                object_proto_query_column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "object_proto query column {} could not be read: {message}",
                object_proto_query_column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "object_proto query column {} value {value:?} is not a strict {target}",
                object_proto_query_column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "object_proto query column {} value {value:?} overflows {target}",
                object_proto_query_column_name(*column)
            ),
            Self::InvalidMaterialFormat { value, pair_index } => write!(
                formatter,
                "object_proto material pair {pair_index} in {value:?} is not item,count"
            ),
            Self::TooManyMaterials { count, maximum } => write!(
                formatter,
                "object_proto material value has {count} pairs; fixed maximum is {maximum}"
            ),
            Self::InvalidMaterialNumber {
                pair_index,
                component,
                value,
            } => write!(
                formatter,
                "object_proto material pair {pair_index} {component} value {value:?} is not an unsigned decimal integer"
            ),
            Self::MaterialNumberOverflow {
                pair_index,
                component,
                value,
            } => write!(
                formatter,
                "object_proto material pair {pair_index} {component} value {value:?} overflows u32"
            ),
            Self::NpcYOverflow {
                region_y1,
                region_y3,
            } => write!(
                formatter,
                "object_proto NPC Y derivation from regions {region_y1} and {region_y3} plus 300 overflows i32"
            ),
        }
    }
}

impl Error for ObjectProtoRowError {}

/// Limits checked before allocating an object-prototype section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectProtoSectionLimits {
    /// Maximum number of source rows accepted.
    pub max_records: usize,
    /// Maximum number of packed record-data bytes accepted.
    pub max_data_bytes: usize,
}

impl ObjectProtoSectionLimits {
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

impl Default for ObjectProtoSectionLimits {
    fn default() -> Self {
        Self {
            max_records: OBJECT_PROTO_TABLE_MAX_RECORDS,
            max_data_bytes: OBJECT_PROTO_TABLE_MAX_SECTION_BYTES,
        }
    }
}

/// A checked object-prototype section construction error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectProtoSectionError {
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
    /// The bounded output vector could not reserve its requested length.
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
        source: ObjectProtoRowError,
    },
}

impl fmt::Display for ObjectProtoSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "object_proto section has {count} rows; configured limit is {maximum}"
            ),
            Self::CountOverflow { count } => {
                write!(
                    formatter,
                    "object_proto section count {count} does not fit u16"
                )
            }
            Self::RecordSizeOverflow { size } => write!(
                formatter,
                "object_proto record width {size} does not fit u16"
            ),
            Self::DataSizeOverflow { count } => write!(
                formatter,
                "object_proto section byte size overflows usize for {count} rows"
            ),
            Self::AllocationFailed { requested } => write!(
                formatter,
                "object_proto section could not allocate {requested} output bytes"
            ),
            Self::DataTooLarge { length, maximum } => write!(
                formatter,
                "object_proto section data length {length} exceeds limit {maximum}"
            ),
            Self::RecordSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "object_proto row {index} encoded to {actual} bytes; expected {expected}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "object_proto row {index} is invalid: {source}")
            }
        }
    }
}

impl Error for ObjectProtoSectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Base table name used by the source-fixed object-prototype loader.
pub const OBJECT_PROTO_TABLE: &str = "object_proto";

/// Exact fixed-column and `FROM` prefix used by the legacy query.
pub const OBJECT_PROTO_TABLE_QUERY_PREFIX: &str =
    "SELECT vnum, price, materials, upgrade_vnum, upgrade_limit_time, life, reg_1, reg_2, reg_3, reg_4, npc, group_vnum, dependent_group FROM ";

/// Exact ordering suffix used by the legacy query.
pub const OBJECT_PROTO_TABLE_QUERY_SUFFIX: &str = " ORDER BY vnum";

/// Maximum statement bytes accepted by the source's 4096-byte query buffer.
pub const MAX_OBJECT_PROTO_TABLE_QUERY_BYTES: usize = 4_095;

/// Maximum generated `object_proto` table-identifier bytes.
pub const MAX_OBJECT_PROTO_TABLE_NAME_BYTES: usize =
    OBJECT_PROTO_TABLE.len() + crate::postfix::MAX_TABLE_POSTFIX_BYTES;

/// A defensive failure while composing the fixed object-prototype query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectProtoQueryBuildError {
    /// The generated table identifier exceeded its bounded width.
    TableNameTooLong {
        /// Generated identifier byte length.
        length: usize,
        /// Maximum accepted identifier byte length.
        maximum: usize,
    },
    /// The generated identifier contained a non-allowlisted byte.
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

impl fmt::Display for ObjectProtoQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated object_proto table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated object_proto identifier has byte {byte:#04x} at offset {index}"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated object_proto query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for ObjectProtoQueryBuildError {}

/// A failure while validating a loader postfix or fixed query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectProtoBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(crate::postfix::TablePostfixError),
    /// The fixed query failed a defensive construction check.
    Query(ObjectProtoQueryBuildError),
}

impl fmt::Display for ObjectProtoBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for ObjectProtoBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// One immutable, checked object-prototype read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectProtoTableQuery {
    statement: String,
    table_name: String,
    postfix: crate::postfix::TablePostfix,
}

impl ObjectProtoTableQuery {
    /// Build the exact source-fixed query from a validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectProtoQueryBuildError`] if a future invariant change
    /// produces an oversized or non-allowlisted table identifier or statement.
    pub fn new(postfix: &crate::postfix::TablePostfix) -> Result<Self, ObjectProtoQueryBuildError> {
        let table_name = format!("{OBJECT_PROTO_TABLE}{}", postfix.as_str());
        if table_name.len() > MAX_OBJECT_PROTO_TABLE_NAME_BYTES {
            return Err(ObjectProtoQueryBuildError::TableNameTooLong {
                length: table_name.len(),
                maximum: MAX_OBJECT_PROTO_TABLE_NAME_BYTES,
            });
        }
        if let Some((index, byte)) = table_name
            .bytes()
            .enumerate()
            .find(|(_, byte)| !byte.is_ascii_alphanumeric() && *byte != b'_')
        {
            return Err(ObjectProtoQueryBuildError::InvalidTableIdentifier { index, byte });
        }

        let capacity = OBJECT_PROTO_TABLE_QUERY_PREFIX.len()
            + table_name.len()
            + OBJECT_PROTO_TABLE_QUERY_SUFFIX.len();
        let mut statement = String::with_capacity(capacity);
        statement.push_str(OBJECT_PROTO_TABLE_QUERY_PREFIX);
        statement.push_str(&table_name);
        statement.push_str(OBJECT_PROTO_TABLE_QUERY_SUFFIX);
        if statement.len() > MAX_OBJECT_PROTO_TABLE_QUERY_BYTES {
            return Err(ObjectProtoQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_OBJECT_PROTO_TABLE_QUERY_BYTES,
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
    pub fn from_config(configured_postfix: Option<&str>) -> Result<Self, ObjectProtoBoundaryError> {
        let postfix = crate::postfix::TablePostfix::from_config(configured_postfix)
            .map_err(ObjectProtoBoundaryError::Postfix)?;
        Self::new(&postfix).map_err(ObjectProtoBoundaryError::Query)
    }

    /// Borrow the exact query text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Borrow the generated `object_proto` table identifier.
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

/// A caller-owned source for the checked object-prototype read query.
pub trait ObjectProtoTableRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw rows for the checked statement.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source failure
    /// must not be represented as an empty row set.
    fn query_rows(
        &self,
        query: &ObjectProtoTableQuery,
    ) -> Result<Vec<ObjectProtoTableQueryRow>, Self::Error>;
}

impl<F, E> ObjectProtoTableRowSource for F
where
    F: Fn(&ObjectProtoTableQuery) -> Result<Vec<ObjectProtoTableQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(
        &self,
        query: &ObjectProtoTableQuery,
    ) -> Result<Vec<ObjectProtoTableQueryRow>, Self::Error> {
        self(query)
    }
}

/// Short alias for [`ObjectProtoTableRowSource`].
pub use ObjectProtoTableRowSource as ObjectProtoRowSource;

/// An error while obtaining or decoding object-prototype rows through a loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectProtoTableLoadError<E> {
    /// The caller-owned source failed.
    Source(E),
    /// Raw rows failed selected decoding or limits.
    Rows(ObjectProtoSectionError),
}

impl<E: fmt::Display> fmt::Display for ObjectProtoTableLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => {
                write!(formatter, "object_proto row source failed: {source}")
            }
            Self::Rows(source) => {
                write!(formatter, "object_proto rows could not be loaded: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for ObjectProtoTableLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Rows(source) => Some(source),
        }
    }
}

/// A reusable bounded loader around the fixed object-prototype query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectProtoTableLoader {
    query: ObjectProtoTableQuery,
    limits: ObjectProtoSectionLimits,
}

impl ObjectProtoTableLoader {
    /// Construct a loader from an already validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectProtoQueryBuildError`] if a defensive query check fails.
    pub fn new(
        postfix: &crate::postfix::TablePostfix,
        limits: ObjectProtoSectionLimits,
    ) -> Result<Self, ObjectProtoQueryBuildError> {
        Ok(Self {
            query: ObjectProtoTableQuery::new(postfix)?,
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
        limits: ObjectProtoSectionLimits,
    ) -> Result<Self, ObjectProtoBoundaryError> {
        let postfix = crate::postfix::TablePostfix::from_config(configured_postfix)
            .map_err(ObjectProtoBoundaryError::Postfix)?;
        Self::new(&postfix, limits).map_err(ObjectProtoBoundaryError::Query)
    }

    /// Borrow the immutable checked query.
    #[must_use]
    pub const fn query(&self) -> &ObjectProtoTableQuery {
        &self.query
    }

    /// Return the configured row and encoded-byte limits.
    #[must_use]
    pub const fn limits(&self) -> ObjectProtoSectionLimits {
        self.limits
    }

    /// Obtain raw rows and strictly build a section in source order.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectProtoTableLoadError::Source`] for acquisition failure or
    /// [`ObjectProtoTableLoadError::Rows`] for strict row/limit failure.
    pub fn load_section<S>(
        &self,
        source: &S,
    ) -> Result<BootSection, ObjectProtoTableLoadError<S::Error>>
    where
        S: ObjectProtoTableRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ObjectProtoTableLoadError::Source)?;
        build_object_proto_section_with_limits(&rows, self.limits)
            .map_err(ObjectProtoTableLoadError::Rows)
    }

    /// Obtain raw rows and build a section with the named legacy policy.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectProtoTableLoadError::Source`] for acquisition failure or
    /// [`ObjectProtoTableLoadError::Rows`] for selected row/limit failure.
    pub fn load_section_legacy<S>(
        &self,
        source: &S,
    ) -> Result<BootSection, ObjectProtoTableLoadError<S::Error>>
    where
        S: ObjectProtoTableRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ObjectProtoTableLoadError::Source)?;
        build_object_proto_section_legacy_with_limits(&rows, self.limits)
            .map_err(ObjectProtoTableLoadError::Rows)
    }
}

/// Short alias for [`ObjectProtoTableQuery`].
pub type ObjectProtoQuery = ObjectProtoTableQuery;
/// Short alias for [`ObjectProtoTableLoader`].
pub type ObjectProtoLoader = ObjectProtoTableLoader;
/// Short alias for [`ObjectProtoTableLoadError`].
pub type ObjectProtoLoaderError<E> = ObjectProtoTableLoadError<E>;
/// Short alias for [`ObjectProtoTableLoadError`].
pub type ObjectProtoLoadError<E> = ObjectProtoTableLoadError<E>;
/// Table-specific alias for [`ObjectProtoQueryValue`].
pub type ObjectProtoTableCell = ObjectProtoQueryValue;
/// Table-specific alias for [`ObjectProtoTableQueryRow`].
pub type ObjectProtoTableRow = ObjectProtoTableQueryRow;
/// Table-specific alias for [`ObjectProtoRowError`].
pub type ObjectProtoTableRowError = ObjectProtoRowError;
/// Table-specific alias for [`ObjectProtoSectionLimits`].
pub type ObjectProtoTableSectionLimits = ObjectProtoSectionLimits;
/// Table-specific alias for [`ObjectProtoSectionError`].
pub type ObjectProtoTableSectionError = ObjectProtoSectionError;
/// Table-specific alias for [`ObjectProtoSectionError`].
pub type ObjectProtoTableSectionBuildError = ObjectProtoSectionError;
/// Table-specific alias for [`ObjectProtoQueryBuildError`].
pub type ObjectProtoTableQueryBuilderError = ObjectProtoQueryBuildError;
/// Table-specific alias for [`ObjectProtoQueryBuildError`].
pub type ObjectProtoTableQueryBuildError = ObjectProtoQueryBuildError;
/// Table-specific alias for [`ObjectProtoBoundaryError`].
pub type ObjectProtoTableBoundaryError = ObjectProtoBoundaryError;
/// Table-specific alias for [`ObjectProtoSectionBuilder`].
pub type ObjectProtoTableBuilder = ObjectProtoSectionBuilder;

/// A reusable pure section builder with explicit limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectProtoSectionBuilder {
    limits: ObjectProtoSectionLimits,
}

impl ObjectProtoSectionBuilder {
    /// Construct a builder with a caller-selected row cap.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            limits: ObjectProtoSectionLimits::new(max_records),
        }
    }

    /// Construct a builder with both limits.
    #[must_use]
    pub const fn with_limits(limits: ObjectProtoSectionLimits) -> Self {
        Self { limits }
    }

    /// Return configured limits.
    #[must_use]
    pub const fn limits(self) -> ObjectProtoSectionLimits {
        self.limits
    }

    /// Decode and build a strict section in source order.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectProtoSectionError`] for row, count, allocation, or
    /// configured limit failures.
    pub fn build(
        &self,
        rows: &[ObjectProtoTableQueryRow],
    ) -> Result<BootSection, ObjectProtoSectionError> {
        build_object_proto_section_with_limits(rows, self.limits)
    }

    /// Decode and build a section with the named legacy policy.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectProtoSectionError`] for source, count, allocation, or
    /// configured limit failures.
    pub fn build_legacy(
        &self,
        rows: &[ObjectProtoTableQueryRow],
    ) -> Result<BootSection, ObjectProtoSectionError> {
        build_object_proto_section_legacy_with_limits(rows, self.limits)
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

fn cell_text(value: &ObjectProtoQueryValue, column: usize) -> Result<&str, ObjectProtoRowError> {
    match value {
        ObjectProtoQueryValue::Text(text) => Ok(text),
        ObjectProtoQueryValue::Null => Err(ObjectProtoRowError::Null { column }),
        ObjectProtoQueryValue::Error(message) => Err(ObjectProtoRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn decode_strict_u32(
    value: &ObjectProtoQueryValue,
    column: usize,
) -> Result<u32, ObjectProtoRowError> {
    let text = cell_text(value, column)?;
    if !is_strict_unsigned_decimal(text) {
        return Err(ObjectProtoRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "u32",
        });
    }
    text.parse::<u32>()
        .map_err(|_| ObjectProtoRowError::NumberOverflow {
            column,
            value: text.to_owned(),
            target: "u32",
        })
}

fn decode_strict_i32(
    value: &ObjectProtoQueryValue,
    column: usize,
) -> Result<i32, ObjectProtoRowError> {
    let text = cell_text(value, column)?;
    if !is_strict_signed_decimal(text) {
        return Err(ObjectProtoRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "i32",
        });
    }
    text.parse::<i32>()
        .map_err(|_| ObjectProtoRowError::NumberOverflow {
            column,
            value: text.to_owned(),
            target: "i32",
        })
}

fn parse_strict_material_component(
    value: &str,
    pair_index: usize,
    component: &'static str,
) -> Result<u32, ObjectProtoRowError> {
    if !is_strict_unsigned_decimal(value) {
        return Err(ObjectProtoRowError::InvalidMaterialNumber {
            pair_index,
            component,
            value: value.to_owned(),
        });
    }
    value
        .parse::<u32>()
        .map_err(|_| ObjectProtoRowError::MaterialNumberOverflow {
            pair_index,
            component,
            value: value.to_owned(),
        })
}

fn decode_strict_materials(
    value: &ObjectProtoQueryValue,
) -> Result<[ObjectMaterial; OBJECT_PROTO_MATERIAL_MAX_NUM], ObjectProtoRowError> {
    let text = cell_text(value, 2)?;
    if text.is_empty() {
        return Ok([ObjectMaterial::default(); OBJECT_PROTO_MATERIAL_MAX_NUM]);
    }

    let pair_count = text.bytes().filter(|byte| *byte == b'/').count() + 1;
    if pair_count > OBJECT_PROTO_MATERIAL_MAX_NUM {
        return Err(ObjectProtoRowError::TooManyMaterials {
            count: pair_count,
            maximum: OBJECT_PROTO_MATERIAL_MAX_NUM,
        });
    }

    let mut materials = [ObjectMaterial::default(); OBJECT_PROTO_MATERIAL_MAX_NUM];
    for (pair_index, pair) in text.split('/').enumerate() {
        let mut components = pair.split(',');
        let Some(item_vnum) = components.next() else {
            return Err(ObjectProtoRowError::InvalidMaterialFormat {
                value: text.to_owned(),
                pair_index,
            });
        };
        let Some(count) = components.next() else {
            return Err(ObjectProtoRowError::InvalidMaterialFormat {
                value: text.to_owned(),
                pair_index,
            });
        };
        if components.next().is_some() {
            return Err(ObjectProtoRowError::InvalidMaterialFormat {
                value: text.to_owned(),
                pair_index,
            });
        }
        materials[pair_index] = ObjectMaterial {
            item_vnum: parse_strict_material_component(item_vnum, pair_index, "item_vnum")?,
            count: parse_strict_material_component(count, pair_index, "count")?,
        };
    }
    Ok(materials)
}

fn check_row_width(row: &ObjectProtoTableQueryRow) -> Result<(), ObjectProtoRowError> {
    if row.columns.len() == OBJECT_PROTO_TABLE_QUERY_COLUMNS {
        Ok(())
    } else {
        Err(ObjectProtoRowError::ColumnCount {
            expected: OBJECT_PROTO_TABLE_QUERY_COLUMNS,
            actual: row.columns.len(),
        })
    }
}

/// Strictly decode one exact-width object-prototype row.
///
/// Scalar values must be complete decimal integers without whitespace or a
/// leading `+`. Signed values may have one leading `-`. Materials must be
/// empty or contain one to five canonical unsigned `item,count` pairs. The
/// derived NPC coordinates are `npc_x = 0` and
/// `npc_y = max(regions[1], regions[3]) + 300`.
///
/// # Errors
///
/// Returns [`ObjectProtoRowError`] for wrong width, SQL `NULL`, source error,
/// malformed or oversized material data, invalid scalar text, target-width
/// overflow, or NPC Y derivation overflow.
pub fn decode_object_proto_query_row(
    row: &ObjectProtoTableQueryRow,
) -> Result<ObjectProtoRecord, ObjectProtoRowError> {
    check_row_width(row)?;

    // Decode in the exact source-column order. This keeps error precedence
    // stable when more than one source cell is malformed.
    let vnum = decode_strict_u32(&row.columns[0], 0)?;
    let price = decode_strict_u32(&row.columns[1], 1)?;
    let materials = decode_strict_materials(&row.columns[2])?;
    let upgrade_vnum = decode_strict_u32(&row.columns[3], 3)?;
    let upgrade_limit_time = decode_strict_u32(&row.columns[4], 4)?;
    let life = decode_strict_i32(&row.columns[5], 5)?;
    let regions = [
        decode_strict_i32(&row.columns[6], 6)?,
        decode_strict_i32(&row.columns[7], 7)?,
        decode_strict_i32(&row.columns[8], 8)?,
        decode_strict_i32(&row.columns[9], 9)?,
    ];
    let npc_vnum = decode_strict_u32(&row.columns[10], 10)?;
    let group_vnum = decode_strict_u32(&row.columns[11], 11)?;
    let dependent_group_vnum = decode_strict_u32(&row.columns[12], 12)?;
    let npc_y =
        regions[1]
            .max(regions[3])
            .checked_add(300)
            .ok_or(ObjectProtoRowError::NpcYOverflow {
                region_y1: regions[1],
                region_y3: regions[3],
            })?;

    Ok(ObjectProtoRecord {
        vnum,
        price,
        materials,
        upgrade_vnum,
        upgrade_limit_time,
        life,
        regions,
        npc_vnum,
        npc_x: 0,
        npc_y,
        group_vnum,
        dependent_group_vnum,
    })
}

fn legacy_error(value: &ObjectProtoQueryValue, column: usize) -> Option<ObjectProtoRowError> {
    match value {
        ObjectProtoQueryValue::Error(message) => Some(ObjectProtoRowError::Source {
            column,
            message: message.clone(),
        }),
        ObjectProtoQueryValue::Text(_) | ObjectProtoQueryValue::Null => None,
    }
}

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Parse a 32-bit C `strtol`/`strtoul`-style numeric prefix.
fn legacy_numeric_prefix(text: &str) -> (bool, Option<u64>) {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() && is_c_space(bytes[index]) {
        index += 1;
    }
    let mut negative = false;
    if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
        negative = bytes[index] == b'-';
        index += 1;
    }
    let start = index;
    let mut magnitude = 0_u64;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        let digit = u64::from(bytes[index] - b'0');
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|number| number.checked_add(digit))
            .unwrap_or(u64::MAX);
        index += 1;
    }
    (negative, (index != start).then_some(magnitude))
}

fn legacy_i32_text(text: &str) -> i32 {
    let (negative, magnitude) = legacy_numeric_prefix(text);
    let Some(magnitude) = magnitude else {
        return 0;
    };
    if negative {
        if magnitude >= 2_147_483_648 {
            i32::MIN
        } else {
            i32::try_from(magnitude).map_or(i32::MIN, |value| -value)
        }
    } else if magnitude > i32::MAX as u64 {
        i32::MAX
    } else {
        i32::try_from(magnitude).unwrap_or(i32::MAX)
    }
}

fn legacy_i32(value: &ObjectProtoQueryValue, column: usize) -> Result<i32, ObjectProtoRowError> {
    if let Some(error) = legacy_error(value, column) {
        return Err(error);
    }
    let text = match value {
        ObjectProtoQueryValue::Text(text) => text,
        ObjectProtoQueryValue::Null => return Ok(0),
        ObjectProtoQueryValue::Error(_) => unreachable!(),
    };
    Ok(legacy_i32_text(text))
}

fn legacy_u32(value: &ObjectProtoQueryValue, column: usize) -> Result<u32, ObjectProtoRowError> {
    if let Some(error) = legacy_error(value, column) {
        return Err(error);
    }
    let text = match value {
        ObjectProtoQueryValue::Text(text) => text,
        ObjectProtoQueryValue::Null => return Ok(0),
        ObjectProtoQueryValue::Error(_) => unreachable!(),
    };
    let (negative, magnitude) = legacy_numeric_prefix(text);
    let Some(magnitude) = magnitude else {
        return Ok(0);
    };
    if magnitude > u64::from(u32::MAX) {
        return Ok(u32::MAX);
    }
    let reduced = u32::try_from(magnitude).unwrap_or(u32::MAX);
    Ok(if negative {
        0_u32.wrapping_sub(reduced)
    } else {
        reduced
    })
}

fn legacy_32_byte_prefix(value: &str) -> &str {
    if value.len() <= 32 {
        return value;
    }
    let mut end = 32;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn decode_legacy_materials(
    value: &ObjectProtoQueryValue,
) -> Result<[ObjectMaterial; OBJECT_PROTO_MATERIAL_MAX_NUM], ObjectProtoRowError> {
    if let Some(error) = legacy_error(value, 2) {
        return Err(error);
    }
    let text = match value {
        ObjectProtoQueryValue::Text(text) => text.split('\0').next().unwrap_or(""),
        ObjectProtoQueryValue::Null => {
            return Err(ObjectProtoRowError::Null { column: 2 });
        }
        ObjectProtoQueryValue::Error(_) => unreachable!(),
    };
    let segments: Vec<&str> = text.split('/').collect();
    let mut materials = [ObjectMaterial::default(); OBJECT_PROTO_MATERIAL_MAX_NUM];
    let mut output_index = 0;

    for (source_index, segment) in segments.iter().enumerate() {
        if output_index == OBJECT_PROTO_MATERIAL_MAX_NUM {
            break;
        }
        if !segment.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            continue;
        }

        // The source copies at most 32 bytes before finding a comma. For the
        // final segment it finds the comma in the original string, but still
        // parses the first component from the 32-byte copy.
        let copied = legacy_32_byte_prefix(segment);
        let comma_search = if source_index + 1 == segments.len() {
            *segment
        } else {
            copied
        };
        let (first_text, count_text) = if let Some(comma) = comma_search.find(',') {
            let count_start = comma + 1;
            let copied_count_end = copied.len().min(segment.len());
            let count_end = if source_index + 1 == segments.len() {
                segment.len()
            } else {
                copied_count_end
            };
            let first_end = comma.min(copied.len());
            (
                &segment[..first_end],
                &segment[count_start..count_end.max(count_start)],
            )
        } else {
            (copied, "")
        };

        let first = legacy_i32_text(first_text);
        let count = legacy_i32_text(count_text);
        materials[output_index] = ObjectMaterial {
            // The legacy helper stores signed `int` values into DWORD
            // destinations. Preserve the active x86 two's-complement bits
            // without a host-layout cast.
            item_vnum: u32::from_le_bytes(first.to_le_bytes()),
            count: u32::from_le_bytes(count.to_le_bytes()),
        };
        output_index += 1;
    }
    Ok(materials)
}

/// Decode one row with the explicitly named legacy conversion policy.
///
/// Scalar `NULL` and empty values leave zero-initialized destinations at zero.
/// Decimal prefixes, whitespace, signs, suffix text, 32-bit conversion limits,
/// and destination casts follow the active x86 helpers. Legacy material parsing
/// keeps only source pairs that begin with an ASCII digit, copies each pair
/// through the source's 32-byte scratch bound, defaults a missing count to
/// zero, and retains only the first five resulting slots. Defined no-conversion
/// cases are normalized to zero rather than reproducing the C++ uninitialized
/// pair state. The compatibility coordinate addition uses explicit x86
/// two's-complement wrapping.
///
/// A SQL `NULL` materials cell returns [`ObjectProtoRowError::Null`] because
/// the source would call `strchr` with a null pointer. This safe boundary does
/// not reproduce that process crash.
///
/// # Errors
///
/// Returns [`ObjectProtoRowError`] for wrong row width, a source extraction
/// error, a null materials cell, or NPC Y derivation overflow.
pub fn decode_object_proto_query_row_legacy(
    row: &ObjectProtoTableQueryRow,
) -> Result<ObjectProtoRecord, ObjectProtoRowError> {
    check_row_width(row)?;

    // Decode in the exact source-column order. This keeps error precedence
    // stable when more than one source cell is malformed.
    let vnum = legacy_u32(&row.columns[0], 0)?;
    let price = legacy_u32(&row.columns[1], 1)?;
    let materials = decode_legacy_materials(&row.columns[2])?;
    let upgrade_vnum = legacy_u32(&row.columns[3], 3)?;
    let upgrade_limit_time = legacy_u32(&row.columns[4], 4)?;
    let life = legacy_i32(&row.columns[5], 5)?;
    let regions = [
        legacy_i32(&row.columns[6], 6)?,
        legacy_i32(&row.columns[7], 7)?,
        legacy_i32(&row.columns[8], 8)?,
        legacy_i32(&row.columns[9], 9)?,
    ];
    let npc_vnum = legacy_u32(&row.columns[10], 10)?;
    let group_vnum = legacy_u32(&row.columns[11], 11)?;
    let dependent_group_vnum = legacy_u32(&row.columns[12], 12)?;
    let npc_y = regions[1].max(regions[3]).wrapping_add(300);

    Ok(ObjectProtoRecord {
        vnum,
        price,
        materials,
        upgrade_vnum,
        upgrade_limit_time,
        life,
        regions,
        npc_vnum,
        npc_x: 0,
        npc_y,
        group_vnum,
        dependent_group_vnum,
    })
}

fn validate_section_size(
    count: usize,
    limits: ObjectProtoSectionLimits,
) -> Result<(u16, u16, usize), ObjectProtoSectionError> {
    if count > limits.max_records {
        return Err(ObjectProtoSectionError::TooManyRecords {
            count,
            maximum: limits.max_records,
        });
    }
    let wire_count =
        u16::try_from(count).map_err(|_| ObjectProtoSectionError::CountOverflow { count })?;
    let record_size = u16::try_from(OBJECT_PROTO_RECORD_WIRE_SIZE).map_err(|_| {
        ObjectProtoSectionError::RecordSizeOverflow {
            size: OBJECT_PROTO_RECORD_WIRE_SIZE,
        }
    })?;
    let data_len = OBJECT_PROTO_RECORD_WIRE_SIZE
        .checked_mul(count)
        .ok_or(ObjectProtoSectionError::DataSizeOverflow { count })?;
    if data_len > limits.max_data_bytes {
        return Err(ObjectProtoSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }
    Ok((wire_count, record_size, data_len))
}

fn append_rows(
    rows: &[ObjectProtoTableQueryRow],
    limits: ObjectProtoSectionLimits,
    legacy: bool,
) -> Result<BootSection, ObjectProtoSectionError> {
    let (count, record_size, data_len) = validate_section_size(rows.len(), limits)?;
    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| ObjectProtoSectionError::AllocationFailed {
            requested: data_len,
        })?;

    for (index, row) in rows.iter().enumerate() {
        let record = if legacy {
            decode_object_proto_query_row_legacy(row)
        } else {
            decode_object_proto_query_row(row)
        }
        .map_err(|source| ObjectProtoSectionError::Row { index, source })?;
        let encoded = record.encode();
        if encoded.len() != OBJECT_PROTO_RECORD_WIRE_SIZE {
            return Err(ObjectProtoSectionError::RecordSizeMismatch {
                index,
                expected: OBJECT_PROTO_RECORD_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }
    debug_assert_eq!(data.len(), data_len);

    Ok(BootSection {
        kind: BootSectionKind::ObjectProto,
        record_size,
        count,
        data,
    })
}

/// Build a strict object-prototype section with default limits.
///
/// # Errors
///
/// Returns [`ObjectProtoSectionError`] for invalid rows or configured limits.
pub fn build_object_proto_section(
    rows: &[ObjectProtoTableQueryRow],
) -> Result<BootSection, ObjectProtoSectionError> {
    build_object_proto_section_with_limits(rows, ObjectProtoSectionLimits::default())
}

/// Build a strict object-prototype section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`ObjectProtoSectionError`] for invalid rows or configured limits.
pub fn build_object_proto_section_with_limit(
    rows: &[ObjectProtoTableQueryRow],
    max_records: usize,
) -> Result<BootSection, ObjectProtoSectionError> {
    build_object_proto_section_with_limits(rows, ObjectProtoSectionLimits::new(max_records))
}

/// Decode rows and build a strict typed object-prototype boot section.
///
/// Rows are encoded in source order, including adjacent duplicates. The section
/// always declares the source-fixed 96-byte width, including when empty.
/// Limits are checked before the output vector is reserved.
///
/// # Errors
///
/// Returns [`ObjectProtoSectionError`] for row, count, allocation, or
/// configured limit failures.
pub fn build_object_proto_section_with_limits(
    rows: &[ObjectProtoTableQueryRow],
    limits: ObjectProtoSectionLimits,
) -> Result<BootSection, ObjectProtoSectionError> {
    append_rows(rows, limits, false)
}

/// Build an object-prototype section with the named legacy conversion policy.
///
/// # Errors
///
/// Returns [`ObjectProtoSectionError`] for source, count, allocation, or
/// configured limit failures.
pub fn build_object_proto_section_legacy(
    rows: &[ObjectProtoTableQueryRow],
) -> Result<BootSection, ObjectProtoSectionError> {
    build_object_proto_section_legacy_with_limits(rows, ObjectProtoSectionLimits::default())
}

/// Build a legacy-policy section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`ObjectProtoSectionError`] for source, count, allocation, or
/// configured limit failures.
pub fn build_object_proto_section_legacy_with_limit(
    rows: &[ObjectProtoTableQueryRow],
    max_records: usize,
) -> Result<BootSection, ObjectProtoSectionError> {
    build_object_proto_section_legacy_with_limits(rows, ObjectProtoSectionLimits::new(max_records))
}

/// Decode rows and build a section with the named legacy conversion policy.
///
/// # Errors
///
/// Returns [`ObjectProtoSectionError`] for source, count, allocation, or
/// configured limit failures.
pub fn build_object_proto_section_legacy_with_limits(
    rows: &[ObjectProtoTableQueryRow],
    limits: ObjectProtoSectionLimits,
) -> Result<BootSection, ObjectProtoSectionError> {
    append_rows(rows, limits, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_source(
        _query: &ObjectProtoTableQuery,
    ) -> Result<Vec<ObjectProtoTableQueryRow>, String> {
        Err("offline".to_owned())
    }

    fn text(value: &str) -> ObjectProtoQueryValue {
        ObjectProtoQueryValue::text(value)
    }

    fn row_with_materials(vnum: &str, materials: &str) -> ObjectProtoTableQueryRow {
        ObjectProtoTableQueryRow::from_typed_columns([
            text(vnum),
            text("100"),
            text(materials),
            text("200"),
            text("300"),
            text("400"),
            text("-10"),
            text("20"),
            text("30"),
            text("40"),
            text("500"),
            text("600"),
            text("700"),
        ])
    }

    fn valid_row(vnum: u32) -> ObjectProtoTableQueryRow {
        row_with_materials(&vnum.to_string(), "10,1/20,3/300,50")
    }

    fn decode_section_records(section: &BootSection) -> Vec<ObjectProtoRecord> {
        section
            .data
            .chunks_exact(OBJECT_PROTO_RECORD_WIRE_SIZE)
            .map(|bytes| ObjectProtoRecord::decode(bytes).expect("fixed record should decode"))
            .collect()
    }

    #[test]
    fn query_metadata_and_exact_statement_are_fixed() {
        assert_eq!(OBJECT_PROTO_TABLE_QUERY_COLUMNS, 13);
        assert_eq!(OBJECT_PROTO_TABLE_QUERY_COLUMN_NAMES.len(), 13);
        assert_eq!(OBJECT_PROTO_SECTION_RECORD_SIZE, 96);
        assert_eq!(OBJECT_PROTO_TABLE_WIRE_SIZE, 96);
        assert_eq!(OBJECT_PROTO_TABLE_MAX_RECORDS, 65_535);
        assert_eq!(OBJECT_PROTO_TABLE_MAX_SECTION_BYTES, 6_291_360);

        let query = ObjectProtoTableQuery::from_config(None).expect("empty postfix is valid");
        assert_eq!(query.table_name(), "object_proto");
        assert_eq!(query.postfix().as_str(), "");
        assert_eq!(
            query.as_str(),
            "SELECT vnum, price, materials, upgrade_vnum, upgrade_limit_time, life, reg_1, reg_2, reg_3, reg_4, npc, group_vnum, dependent_group FROM object_proto ORDER BY vnum"
        );

        let postfixed = ObjectProtoTableQuery::from_config(Some("_eu")).expect("valid postfix");
        assert_eq!(postfixed.table_name(), "object_proto_eu");
        assert!(postfixed
            .as_str()
            .ends_with(" FROM object_proto_eu ORDER BY vnum"));
        assert!(postfixed.as_str().len() <= MAX_OBJECT_PROTO_TABLE_QUERY_BYTES);
    }

    #[test]
    fn query_rejects_unvalidated_postfix_configuration() {
        let error = ObjectProtoTableQuery::from_config(Some("bad-postfix"))
            .expect_err("hyphen is not allowlisted");
        assert!(matches!(error, ObjectProtoBoundaryError::Postfix(_)));
    }

    #[test]
    fn raw_values_and_exact_row_shape_are_retained() {
        let values = vec![
            text("1"),
            ObjectProtoQueryValue::Null,
            ObjectProtoQueryValue::error("conversion failed"),
        ];
        let malformed = ObjectProtoTableQueryRow::new(values.clone());
        assert_eq!(malformed.column_count(), 3);
        assert_eq!(malformed.columns(), values.as_slice());
        assert!(matches!(
            ObjectProtoTableQueryRow::try_new(values).expect_err("wrong width"),
            ObjectProtoRowError::ColumnCount {
                expected: 13,
                actual: 3
            }
        ));

        let exact = valid_row(1);
        assert_eq!(exact.column_count(), OBJECT_PROTO_TABLE_QUERY_COLUMNS);
        assert_eq!(
            exact.columns()[1],
            ObjectProtoQueryValue::Text("100".to_owned())
        );
        assert_eq!(exact.into_columns().len(), 13);
    }

    #[test]
    fn strict_decoder_accepts_five_pairs_and_derives_coordinates() {
        let row = row_with_materials("42", "10,1/20,3/300,50/40,5/50,6");
        let record = decode_object_proto_query_row(&row).expect("strict row should decode");
        assert_eq!(record.vnum, 42);
        assert_eq!(record.price, 100);
        assert_eq!(
            record.materials,
            [
                ObjectMaterial {
                    item_vnum: 10,
                    count: 1
                },
                ObjectMaterial {
                    item_vnum: 20,
                    count: 3
                },
                ObjectMaterial {
                    item_vnum: 300,
                    count: 50
                },
                ObjectMaterial {
                    item_vnum: 40,
                    count: 5
                },
                ObjectMaterial {
                    item_vnum: 50,
                    count: 6
                },
            ]
        );
        assert_eq!(record.upgrade_vnum, 200);
        assert_eq!(record.upgrade_limit_time, 300);
        assert_eq!(record.life, 400);
        assert_eq!(record.regions, [-10, 20, 30, 40]);
        assert_eq!(record.npc_vnum, 500);
        assert_eq!(record.group_vnum, 600);
        assert_eq!(record.dependent_group_vnum, 700);
        assert_eq!(record.npc_x, 0);
        assert_eq!(record.npc_y, 340);
    }

    #[test]
    fn strict_empty_materials_leave_all_slots_zero() {
        let record = decode_object_proto_query_row(&row_with_materials("1", ""))
            .expect("empty material list is valid");
        assert_eq!(record.materials, [ObjectMaterial::default(); 5]);
    }

    #[test]
    fn strict_material_format_and_numeric_errors_are_distinct() {
        for malformed in ["1", "1,", ",2", "1,,2", "1,2,3", "/", "1,2/"] {
            let error = decode_object_proto_query_row(&row_with_materials("1", malformed))
                .expect_err("malformed strict materials must fail");
            assert!(
                matches!(
                    error,
                    ObjectProtoRowError::InvalidMaterialFormat { .. }
                        | ObjectProtoRowError::InvalidMaterialNumber { .. }
                ),
                "unexpected error for {malformed:?}: {error}"
            );
        }

        let error = decode_object_proto_query_row(&row_with_materials("1", "4294967296,1"))
            .expect_err("strict material overflow must fail");
        assert!(matches!(
            error,
            ObjectProtoRowError::MaterialNumberOverflow {
                component: "item_vnum",
                ..
            }
        ));

        let error =
            decode_object_proto_query_row(&row_with_materials("1", "1,2/3,4/5,6/7,8/9,10/11,12"))
                .expect_err("strict policy must not silently discard a sixth pair");
        assert_eq!(
            error,
            ObjectProtoRowError::TooManyMaterials {
                count: 6,
                maximum: 5
            }
        );
    }

    #[test]
    fn strict_null_and_source_errors_remain_distinct() {
        let mut row = valid_row(1);
        row.columns[0] = ObjectProtoQueryValue::Null;
        assert_eq!(
            decode_object_proto_query_row(&row),
            Err(ObjectProtoRowError::Null { column: 0 })
        );

        row.columns[0] = ObjectProtoQueryValue::error("driver conversion failed");
        assert_eq!(
            decode_object_proto_query_row(&row),
            Err(ObjectProtoRowError::Source {
                column: 0,
                message: "driver conversion failed".to_owned()
            })
        );
    }

    #[test]
    fn strict_scalar_policy_rejects_prefixes_and_reports_overflow() {
        let mut row = valid_row(1);
        row.columns[0] = text(" 1");
        assert!(matches!(
            decode_object_proto_query_row(&row),
            Err(ObjectProtoRowError::InvalidNumber {
                column: 0,
                target: "u32",
                ..
            })
        ));

        row.columns[0] = text("4294967296");
        assert!(matches!(
            decode_object_proto_query_row(&row),
            Err(ObjectProtoRowError::NumberOverflow {
                column: 0,
                target: "u32",
                ..
            })
        ));

        row.columns[0] = text("1");
        row.columns[5] = text("2147483648");
        assert!(matches!(
            decode_object_proto_query_row(&row),
            Err(ObjectProtoRowError::NumberOverflow {
                column: 5,
                target: "i32",
                ..
            })
        ));
    }

    #[test]
    fn strict_npc_y_overflow_is_rejected() {
        let mut row = valid_row(1);
        row.columns[7] = text("2147483647");
        row.columns[9] = text("12");
        assert_eq!(
            decode_object_proto_query_row(&row),
            Err(ObjectProtoRowError::NpcYOverflow {
                region_y1: i32::MAX,
                region_y3: 12
            })
        );
    }

    #[test]
    fn legacy_policy_keeps_prefix_casts_nulls_and_material_quirks() {
        let mut row = ObjectProtoTableQueryRow::from_typed_columns([
            text("-1"),
            text(" 12suffix"),
            text("bad/-1,9/1x,2y/3/4,5/6,7/8,9/9,10/10,11"),
            ObjectProtoQueryValue::Null,
            text("+4"),
            text("-6tail"),
            text("not-a-number"),
            text("2147483648"),
            text("9"),
            text("10"),
            text("bad"),
            text(""),
            text("6"),
        ]);
        row.columns[9] = ObjectProtoQueryValue::Null;

        let record = decode_object_proto_query_row_legacy(&row)
            .expect("defined legacy conversions should not fail");
        assert_eq!(record.vnum, u32::MAX);
        assert_eq!(record.price, 12);
        assert_eq!(
            record.materials,
            [
                ObjectMaterial {
                    item_vnum: 1,
                    count: 2
                },
                ObjectMaterial {
                    item_vnum: 3,
                    count: 0
                },
                ObjectMaterial {
                    item_vnum: 4,
                    count: 5
                },
                ObjectMaterial {
                    item_vnum: 6,
                    count: 7
                },
                ObjectMaterial {
                    item_vnum: 8,
                    count: 9
                },
            ]
        );
        assert_eq!(record.upgrade_vnum, 0);
        assert_eq!(record.upgrade_limit_time, 4);
        assert_eq!(record.life, -6);
        assert_eq!(record.regions[0], 0);
        assert_eq!(record.regions[1], i32::MAX);
        assert_eq!(record.regions[2], 9);
        assert_eq!(record.regions[3], 0);
        assert_eq!(record.npc_vnum, 0);
        assert_eq!(record.group_vnum, 0);
        assert_eq!(record.dependent_group_vnum, 6);
        assert_eq!(record.npc_x, 0);
        assert_eq!(record.npc_y, i32::MAX.wrapping_add(300));
    }

    #[test]
    fn legacy_policy_keeps_null_material_a_safe_error() {
        let mut row = valid_row(1);
        row.columns[2] = ObjectProtoQueryValue::Null;
        assert_eq!(
            decode_object_proto_query_row_legacy(&row),
            Err(ObjectProtoRowError::Null { column: 2 })
        );
    }

    #[test]
    fn legacy_policy_keeps_source_errors() {
        let mut row = valid_row(1);
        row.columns[1] = ObjectProtoQueryValue::error("source failed");
        assert_eq!(
            decode_object_proto_query_row_legacy(&row),
            Err(ObjectProtoRowError::Source {
                column: 1,
                message: "source failed".to_owned()
            })
        );
    }

    #[test]
    fn empty_section_has_fixed_metadata_and_no_data() {
        let section = build_object_proto_section(&[]).expect("empty strict section");
        assert_eq!(section.kind, BootSectionKind::ObjectProto);
        assert_eq!(section.record_size, 96);
        assert_eq!(section.count, 0);
        assert!(section.data.is_empty());
    }

    #[test]
    fn section_preserves_source_order_and_duplicates() {
        let rows = vec![valid_row(20), valid_row(10), valid_row(20)];
        let section = build_object_proto_section(&rows).expect("strict section");
        assert_eq!(section.count, 3);
        assert_eq!(section.data.len(), 3 * OBJECT_PROTO_RECORD_WIRE_SIZE);
        let records = decode_section_records(&section);
        assert_eq!(
            records.iter().map(|record| record.vnum).collect::<Vec<_>>(),
            vec![20, 10, 20]
        );
    }

    #[test]
    fn section_checks_row_and_packed_byte_limits_before_allocation() {
        let rows = vec![valid_row(1)];
        let error = build_object_proto_section_with_limit(&rows, 0)
            .expect_err("row cap zero must reject one row");
        assert_eq!(
            error,
            ObjectProtoSectionError::TooManyRecords {
                count: 1,
                maximum: 0
            }
        );

        let error = build_object_proto_section_with_limits(
            &rows,
            ObjectProtoSectionLimits::with_data_limit(1, 95),
        )
        .expect_err("one 96-byte record must exceed a 95-byte cap");
        assert_eq!(
            error,
            ObjectProtoSectionError::DataTooLarge {
                length: 96,
                maximum: 95
            }
        );

        build_object_proto_section_with_limits(
            &rows,
            ObjectProtoSectionLimits::with_data_limit(1, 96),
        )
        .expect("exact packed-byte cap should pass");
    }

    #[test]
    fn u16_count_overflow_is_checked() {
        let count = OBJECT_PROTO_TABLE_MAX_RECORDS + 1;
        let error = validate_section_size(
            count,
            ObjectProtoSectionLimits {
                max_records: usize::MAX,
                max_data_bytes: usize::MAX,
            },
        )
        .expect_err("65,536 records cannot fit the section count");
        assert_eq!(error, ObjectProtoSectionError::CountOverflow { count });
    }

    #[test]
    fn builder_preserves_duplicates_in_both_policies() {
        let rows = vec![valid_row(7), valid_row(7)];
        let strict = ObjectProtoSectionBuilder::new(2)
            .build(&rows)
            .expect("strict builder");
        let legacy = ObjectProtoSectionBuilder::new(2)
            .build_legacy(&rows)
            .expect("legacy builder");
        assert_eq!(strict.count, 2);
        assert_eq!(legacy.count, 2);
        assert_eq!(strict.data, legacy.data);
    }

    #[test]
    fn loader_passes_exact_query_and_preserves_source_error() {
        let postfix = crate::postfix::TablePostfix::default();
        let loader = ObjectProtoTableLoader::new(&postfix, ObjectProtoSectionLimits::default())
            .expect("loader query");
        let source = |query: &ObjectProtoTableQuery| {
            assert_eq!(query.as_str(), loader.query().as_str());
            Ok::<_, String>(vec![valid_row(9)])
        };
        let section = loader
            .load_section(&source)
            .expect("injected source should load");
        assert_eq!(section.count, 1);
        assert_eq!(decode_section_records(&section)[0].vnum, 9);

        let failed: Result<BootSection, ObjectProtoTableLoadError<String>> =
            loader.load_section(&failed_source);
        assert_eq!(
            failed,
            Err(ObjectProtoTableLoadError::Source("offline".to_owned()))
        );
    }
}
