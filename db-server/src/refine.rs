//! Pure, SQL-free conversion of refine query rows to a boot section.
//!
//! The legacy DB loader selects thirteen columns from `refine_proto`:
//! `id`, `cost`, `prob`, and five `(vnum, count)` pairs.  This module keeps
//! the values as query cells until a complete row has been checked.  In
//! particular, SQL `NULL` and a column extraction error are not silently
//! converted to zero.  No `SQLx` or `MySQL` type is part of this boundary.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootSection, BootSectionKind};
use protocol::db_records::{RefineMaterialRecord, RefineTableRecord, REFINE_TABLE_WIRE_SIZE};

/// Number of columns selected by the fixed legacy refine query.
pub const REFINE_TABLE_QUERY_COLUMNS: usize = 13;

/// Alias for [`REFINE_TABLE_QUERY_COLUMNS`].
pub const REFINE_QUERY_COLUMN_COUNT: usize = REFINE_TABLE_QUERY_COLUMNS;

/// Maximum number of records representable by the legacy `u16` section count.
pub const REFINE_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

/// Exact packed `TRefineTable` width represented by a section record.
pub const REFINE_SECTION_RECORD_SIZE: u16 = 53;

/// Maximum byte length of a section when every representable count is used.
pub const REFINE_TABLE_MAX_SECTION_BYTES: usize = REFINE_TABLE_WIRE_SIZE * REFINE_TABLE_MAX_RECORDS;

/// Column names in the fixed legacy query order.
pub const REFINE_TABLE_QUERY_COLUMN_NAMES: [&str; REFINE_TABLE_QUERY_COLUMNS] = [
    "id", "cost", "prob", "vnum0", "count0", "vnum1", "count1", "vnum2", "count2", "vnum3",
    "count3", "vnum4", "count4",
];

/// One value returned by a query-row adapter.
///
/// `Text` is the only state accepted by the strict numeric decoder. `Null`
/// and `Error` retain the two distinct facts that a database column can
/// produce.  The adapter should put its extraction diagnostic in `Error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineQueryValue {
    /// A non-NULL textual value, normally the original SQL result string.
    Text(String),
    /// A SQL `NULL` value.
    Null,
    /// An error raised while obtaining or converting one query column.
    Error(String),
}

impl RefineQueryValue {
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

impl From<String> for RefineQueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for RefineQueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

/// A row-shaped value for the thirteen-column refine query.
///
/// The column vector is intentionally retained at this boundary. A source
/// can therefore report a wrong-width result and the strict decoder can
/// return a useful [`RefineRowError::ColumnCount`] instead of indexing past
/// the input. [`Self::new`] is the normal constructor for an adapter that
/// already knows the row shape; [`Self::try_new`] validates the shape before
/// returning a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefineTableQueryRow {
    columns: Vec<RefineQueryValue>,
}

impl RefineTableQueryRow {
    /// Construct a row while retaining its supplied column values and order.
    ///
    /// This constructor does not reject a wrong column count. That is useful
    /// for a source seam which must report malformed source data through the
    /// later strict decoder. Use [`Self::try_new`] when validation should
    /// happen at the source boundary.
    pub fn new<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = RefineQueryValue>,
    {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Alias for [`Self::new`].
    #[must_use]
    pub fn from_columns<I>(columns: I) -> Self
    where
        I: IntoIterator<Item = RefineQueryValue>,
    {
        Self::new(columns)
    }

    /// Construct and validate a thirteen-column row.
    ///
    /// # Errors
    ///
    /// Returns [`RefineRowError::ColumnCount`] when the iterator does not
    /// produce exactly thirteen values.
    pub fn try_new<I>(columns: I) -> Result<Self, RefineRowError>
    where
        I: IntoIterator<Item = RefineQueryValue>,
    {
        let row = Self::new(columns);
        if row.columns.len() != REFINE_TABLE_QUERY_COLUMNS {
            return Err(RefineRowError::ColumnCount {
                expected: REFINE_TABLE_QUERY_COLUMNS,
                actual: row.columns.len(),
            });
        }
        Ok(row)
    }

    /// Construct a row from a statically sized, correctly shaped column set.
    #[must_use]
    pub fn from_typed_columns(columns: [RefineQueryValue; REFINE_TABLE_QUERY_COLUMNS]) -> Self {
        Self::new(columns)
    }

    /// Borrow all supplied columns in query order.
    #[must_use]
    pub fn columns(&self) -> &[RefineQueryValue] {
        &self.columns
    }

    /// Consume the row and return its columns in query order.
    #[must_use]
    pub fn into_columns(self) -> Vec<RefineQueryValue> {
        self.columns
    }

    /// Return the number of columns supplied by the source.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

impl From<[RefineQueryValue; REFINE_TABLE_QUERY_COLUMNS]> for RefineTableQueryRow {
    fn from(columns: [RefineQueryValue; REFINE_TABLE_QUERY_COLUMNS]) -> Self {
        Self::from_typed_columns(columns)
    }
}

impl TryFrom<Vec<RefineQueryValue>> for RefineTableQueryRow {
    type Error = RefineRowError;

    fn try_from(columns: Vec<RefineQueryValue>) -> Result<Self, Self::Error> {
        Self::try_new(columns)
    }
}

/// Return a stable column name for diagnostics.
#[must_use]
pub const fn refine_query_column_name(index: usize) -> &'static str {
    match index {
        0 => "id",
        1 => "cost",
        2 => "prob",
        3 => "vnum0",
        4 => "count0",
        5 => "vnum1",
        6 => "count1",
        7 => "vnum2",
        8 => "count2",
        9 => "vnum3",
        10 => "count3",
        11 => "vnum4",
        12 => "count4",
        _ => "unknown",
    }
}

/// A strict error found in one refine query row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineRowError {
    /// The row did not contain exactly thirteen columns.
    ColumnCount {
        /// Required column count.
        expected: usize,
        /// Supplied column count.
        actual: usize,
    },
    /// A required column was SQL `NULL`.
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

impl fmt::Display for RefineRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => write!(
                formatter,
                "refine query row has {actual} columns; expected {expected}"
            ),
            Self::Null { column } => write!(
                formatter,
                "refine query column {} is NULL",
                refine_query_column_name(*column)
            ),
            Self::Source { column, message } => write!(
                formatter,
                "refine query column {} could not be read: {message}",
                refine_query_column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "refine query column {} value {value:?} is not a strict {target}",
                refine_query_column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "refine query column {} value {value:?} overflows {target}",
                refine_query_column_name(*column)
            ),
        }
    }
}

impl Error for RefineRowError {}

/// Limits applied before allocating a refine section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefineSectionLimits {
    /// Maximum number of source rows accepted.
    pub max_records: usize,
    /// Maximum number of packed record-data bytes accepted.
    pub max_data_bytes: usize,
}

impl RefineSectionLimits {
    /// Construct limits that bound records but leave a byte limit of
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

impl Default for RefineSectionLimits {
    fn default() -> Self {
        Self {
            max_records: REFINE_TABLE_MAX_RECORDS,
            max_data_bytes: REFINE_TABLE_MAX_SECTION_BYTES,
        }
    }
}

/// A checked section-construction error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineSectionError {
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
    /// A source row could not be strictly decoded.
    Row {
        /// Zero-based source row index.
        index: usize,
        /// Row error.
        source: RefineRowError,
    },
}

impl fmt::Display for RefineSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "refine section has {count} rows; configured limit is {maximum}"
            ),
            Self::CountOverflow { count } => {
                write!(formatter, "refine section count {count} does not fit u16")
            }
            Self::RecordSizeOverflow { size } => {
                write!(formatter, "refine record width {size} does not fit u16")
            }
            Self::DataSizeOverflow { count } => write!(
                formatter,
                "refine section byte size overflows usize for {count} rows"
            ),
            Self::DataTooLarge { length, maximum } => write!(
                formatter,
                "refine section data length {length} exceeds limit {maximum}"
            ),
            Self::RecordSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "refine row {index} encoded to {actual} bytes; expected {expected}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "refine row {index} is invalid: {source}")
            }
        }
    }
}

impl Error for RefineSectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// An injected, fallible source of refine query rows.
///
/// The associated error is intentionally caller-owned. A later `SQLx` adapter
/// can use its database error type without making this pure module depend on
/// `SQLx`. The blanket implementation for closures makes small fixture sources
/// convenient in tests and integrations.
pub trait RefineTableRowSource {
    /// Error returned when the source cannot produce rows.
    type Error: fmt::Display;

    /// Load rows in the order they should appear in the boot section.
    ///
    /// # Errors
    ///
    /// Returns the source-specific error when the row set cannot be loaded.
    fn load_rows(&self) -> Result<Vec<RefineTableQueryRow>, Self::Error>;
}

impl<F, E> RefineTableRowSource for F
where
    F: Fn() -> Result<Vec<RefineTableQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn load_rows(&self) -> Result<Vec<RefineTableQueryRow>, Self::Error> {
        self()
    }
}

/// Short alias for [`RefineTableRowSource`].
pub use RefineTableRowSource as RefineRowSource;

/// An error while obtaining rows or building their section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineLoadError<E> {
    /// The injected row source failed.
    Source(E),
    /// Rows were obtained but could not be decoded or bounded.
    Section(RefineSectionError),
}

impl<E: fmt::Display> fmt::Display for RefineLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "refine row source failed: {source}"),
            Self::Section(source) => {
                write!(formatter, "refine section construction failed: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for RefineLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Section(source) => Some(source),
        }
    }
}

/// A reusable pure builder with explicit limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefineSectionBuilder {
    limits: RefineSectionLimits,
}

impl RefineSectionBuilder {
    /// Construct a builder with a caller-selected row cap.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            limits: RefineSectionLimits::new(max_records),
        }
    }

    /// Construct a builder with both limits.
    #[must_use]
    pub const fn with_limits(limits: RefineSectionLimits) -> Self {
        Self { limits }
    }

    /// Return the configured limits.
    #[must_use]
    pub const fn limits(self) -> RefineSectionLimits {
        self.limits
    }

    /// Decode and build a section, preserving source row order.
    ///
    /// # Errors
    ///
    /// Returns [`RefineSectionError`] for a row/column/numeric error or when
    /// the configured limits are exceeded.
    pub fn build(&self, rows: &[RefineTableQueryRow]) -> Result<BootSection, RefineSectionError> {
        build_refine_section_with_limits(rows, self.limits)
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

fn cell_text(value: &RefineQueryValue, column: usize) -> Result<&str, RefineRowError> {
    match value {
        RefineQueryValue::Text(text) => Ok(text),
        RefineQueryValue::Null => Err(RefineRowError::Null { column }),
        RefineQueryValue::Error(message) => Err(RefineRowError::Source {
            column,
            message: message.clone(),
        }),
    }
}

fn decode_u32(value: &RefineQueryValue, column: usize) -> Result<u32, RefineRowError> {
    let text = cell_text(value, column)?;
    if !is_strict_unsigned_decimal(text) {
        return Err(RefineRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "u32",
        });
    }
    text.parse::<u32>()
        .map_err(|_| RefineRowError::NumberOverflow {
            column,
            value: text.to_owned(),
            target: "u32",
        })
}

fn decode_i32(value: &RefineQueryValue, column: usize) -> Result<i32, RefineRowError> {
    let text = cell_text(value, column)?;
    if !is_strict_signed_decimal(text) {
        return Err(RefineRowError::InvalidNumber {
            column,
            value: text.to_owned(),
            target: "i32",
        });
    }
    text.parse::<i32>()
        .map_err(|_| RefineRowError::NumberOverflow {
            column,
            value: text.to_owned(),
            target: "i32",
        })
}

/// Strictly decode one source row into the existing protocol record.
///
/// # Errors
///
/// Returns a [`RefineRowError`] for a wrong-width row, `NULL` or failed
/// column, or a malformed or out-of-range integer.
pub fn decode_refine_query_row(
    row: &RefineTableQueryRow,
) -> Result<RefineTableRecord, RefineRowError> {
    if row.columns.len() != REFINE_TABLE_QUERY_COLUMNS {
        return Err(RefineRowError::ColumnCount {
            expected: REFINE_TABLE_QUERY_COLUMNS,
            actual: row.columns.len(),
        });
    }

    let id = decode_u32(&row.columns[0], 0)?;
    let cost = decode_i32(&row.columns[1], 1)?;
    let prob = decode_i32(&row.columns[2], 2)?;
    let mut materials = [RefineMaterialRecord::default(); 5];
    let mut material_count = 0_u8;
    for (index, material) in materials.iter_mut().enumerate() {
        let vnum_column = 3 + index * 2;
        let count_column = vnum_column + 1;
        let vnum = decode_u32(&row.columns[vnum_column], vnum_column)?;
        let count = decode_i32(&row.columns[count_column], count_column)?;
        *material = RefineMaterialRecord { vnum, count };
        if vnum != 0 {
            material_count += u8::from(vnum != 0);
        }
    }

    Ok(RefineTableRecord {
        id,
        material_count,
        cost,
        prob,
        materials,
    })
}

fn validate_section_size(
    count: usize,
    limits: RefineSectionLimits,
) -> Result<(u16, u16, usize), RefineSectionError> {
    if count > limits.max_records {
        return Err(RefineSectionError::TooManyRecords {
            count,
            maximum: limits.max_records,
        });
    }
    let wire_count =
        u16::try_from(count).map_err(|_| RefineSectionError::CountOverflow { count })?;
    let record_size = u16::try_from(REFINE_TABLE_WIRE_SIZE).map_err(|_| {
        RefineSectionError::RecordSizeOverflow {
            size: REFINE_TABLE_WIRE_SIZE,
        }
    })?;
    let data_len = REFINE_TABLE_WIRE_SIZE
        .checked_mul(count)
        .ok_or(RefineSectionError::DataSizeOverflow { count })?;
    if data_len > limits.max_data_bytes {
        return Err(RefineSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }
    Ok((wire_count, record_size, data_len))
}

/// Build a refine section with the default representable limits.
///
/// # Errors
///
/// Returns [`RefineSectionError`] when a row is invalid or a configured
/// limit is exceeded.
pub fn build_refine_section(
    rows: &[RefineTableQueryRow],
) -> Result<BootSection, RefineSectionError> {
    build_refine_section_with_limits(rows, RefineSectionLimits::default())
}

/// Build a refine section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`RefineSectionError`] when a row is invalid or the row/byte
/// limits are exceeded.
pub fn build_refine_section_with_limit(
    rows: &[RefineTableQueryRow],
    max_records: usize,
) -> Result<BootSection, RefineSectionError> {
    build_refine_section_with_limits(rows, RefineSectionLimits::new(max_records))
}

/// Decode rows and build a typed refine boot section.
///
/// Rows are encoded in exactly the order supplied by the source. The section
/// always declares the source-fixed 53-byte record width, including for an
/// empty result. The limits are checked before the output vector is reserved.
///
/// # Errors
///
/// Returns [`RefineSectionError`] for invalid rows, count conversion or
/// allocation failures, and configured limit violations.
pub fn build_refine_section_with_limits(
    rows: &[RefineTableQueryRow],
    limits: RefineSectionLimits,
) -> Result<BootSection, RefineSectionError> {
    let (count, record_size, data_len) = validate_section_size(rows.len(), limits)?;
    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| RefineSectionError::DataSizeOverflow { count: rows.len() })?;

    for (index, row) in rows.iter().enumerate() {
        let record = decode_refine_query_row(row)
            .map_err(|source| RefineSectionError::Row { index, source })?;
        let encoded = record.encode();
        if encoded.len() != REFINE_TABLE_WIRE_SIZE {
            return Err(RefineSectionError::RecordSizeMismatch {
                index,
                expected: REFINE_TABLE_WIRE_SIZE,
                actual: encoded.len(),
            });
        }
        data.extend_from_slice(&encoded);
    }

    debug_assert_eq!(data.len(), data_len);
    Ok(BootSection {
        kind: BootSectionKind::Refine,
        record_size,
        count,
        data,
    })
}

/// Load rows from an injected source and build a refine section.
///
/// This function is the only convenience path that invokes the source. The
/// source error is kept distinct from row and section errors; no database
/// error is represented as an empty table.
///
/// # Errors
///
/// Returns [`RefineLoadError::Source`] for a source failure or
/// [`RefineLoadError::Section`] for row decoding and limit failures.
pub fn load_refine_section<S>(
    source: &S,
    limits: RefineSectionLimits,
) -> Result<BootSection, RefineLoadError<S::Error>>
where
    S: RefineTableRowSource,
{
    let rows = source.load_rows().map_err(RefineLoadError::Source)?;
    build_refine_section_with_limits(&rows, limits).map_err(RefineLoadError::Section)
}

/// Load rows and build a section with a caller-selected row cap.
///
/// # Errors
///
/// Returns [`RefineLoadError::Source`] for a source failure or
/// [`RefineLoadError::Section`] for row decoding and limit failures.
pub fn load_refine_section_with_limit<S>(
    source: &S,
    max_records: usize,
) -> Result<BootSection, RefineLoadError<S::Error>>
where
    S: RefineTableRowSource,
{
    load_refine_section(source, RefineSectionLimits::new(max_records))
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::decode_refine_table_section;

    fn value(text: &str) -> RefineQueryValue {
        RefineQueryValue::text(text)
    }

    fn row_with_pairs(pairs: &[(u32, i32); 5]) -> RefineTableQueryRow {
        let mut columns = vec![value("42"), value("-7"), value("91")];
        for (vnum, count) in pairs {
            columns.push(value(&vnum.to_string()));
            columns.push(value(&count.to_string()));
        }
        RefineTableQueryRow::new(columns)
    }

    #[test]
    fn valid_rows_decode_and_round_trip_in_source_order() {
        let first = row_with_pairs(&[(11, 2), (0, 99), (33, 4), (0, 0), (55, 6)]);
        let second = row_with_pairs(&[(1, 1), (2, 2), (0, 0), (0, 0), (0, 0)]);
        let section = build_refine_section(&[first.clone(), second.clone()]).unwrap();

        assert_eq!(section.kind, BootSectionKind::Refine);
        assert_eq!(section.record_size, REFINE_SECTION_RECORD_SIZE);
        assert_eq!(section.count, 2);
        assert_eq!(section.data.len(), 2 * REFINE_TABLE_WIRE_SIZE);
        assert_eq!(decode_refine_table_section(&section).unwrap().len(), 2);

        let decoded = decode_refine_table_section(&section).unwrap();
        assert_eq!(decoded[0].id, 42);
        assert_eq!(decoded[0].material_count, 3);
        assert_eq!(decoded[0].cost, -7);
        assert_eq!(decoded[0].prob, 91);
        assert_eq!(decoded[1].id, 42);
        assert_eq!(decoded[1].materials[1].vnum, 2);
    }

    #[test]
    fn empty_source_produces_an_exact_empty_refine_section() {
        let section = build_refine_section(&[]).unwrap();
        assert_eq!(section.kind, BootSectionKind::Refine);
        assert_eq!(section.record_size, REFINE_SECTION_RECORD_SIZE);
        assert_eq!(section.count, 0);
        assert!(section.data.is_empty());
        assert!(section.is_empty());
        assert!(decode_refine_table_section(&section).unwrap().is_empty());
    }

    #[test]
    fn null_is_not_silently_converted_to_zero() {
        let mut columns = vec![value("1"), RefineQueryValue::null()];
        columns.extend((0..11).map(|_| value("0")));
        let error = build_refine_section(&[RefineTableQueryRow::new(columns)]).unwrap_err();
        assert!(matches!(
            error,
            RefineSectionError::Row {
                index: 0,
                source: RefineRowError::Null { column: 1 }
            }
        ));
    }

    #[test]
    fn source_column_errors_are_preserved() {
        let mut columns = vec![
            value("1"),
            RefineQueryValue::error("driver could not decode cost"),
        ];
        columns.extend((0..11).map(|_| value("0")));
        let error = decode_refine_query_row(&RefineTableQueryRow::new(columns)).unwrap_err();
        assert_eq!(
            error,
            RefineRowError::Source {
                column: 1,
                message: "driver could not decode cost".to_owned()
            }
        );
    }

    #[test]
    fn malformed_and_empty_values_are_rejected_strictly() {
        for text in ["", " 1", "1x", "1.0", "+1"] {
            let mut columns = vec![value(text), value("1"), value("1")];
            columns.extend((0..10).map(|_| value("0")));
            let error = decode_refine_query_row(&RefineTableQueryRow::new(columns)).unwrap_err();
            assert!(matches!(
                error,
                RefineRowError::InvalidNumber {
                    column: 0,
                    target: "u32",
                    ..
                }
            ));
        }
    }

    #[test]
    fn numeric_overflow_is_reported_separately_from_syntax_errors() {
        let mut columns = vec![value("4294967296"), value("1"), value("1")];
        columns.extend((0..10).map(|_| value("0")));
        assert!(matches!(
            decode_refine_query_row(&RefineTableQueryRow::new(columns)),
            Err(RefineRowError::NumberOverflow {
                column: 0,
                target: "u32",
                ..
            })
        ));

        let mut columns = vec![value("1"), value("2147483648"), value("1")];
        columns.extend((0..10).map(|_| value("0")));
        assert!(matches!(
            decode_refine_query_row(&RefineTableQueryRow::new(columns)),
            Err(RefineRowError::NumberOverflow {
                column: 1,
                target: "i32",
                ..
            })
        ));
    }

    #[test]
    fn wrong_column_counts_are_rejected_by_construction_and_decoding() {
        let short = RefineTableQueryRow::new(vec![value("1"); 12]);
        assert!(matches!(
            RefineTableQueryRow::try_new(vec![value("1"); 12]),
            Err(RefineRowError::ColumnCount {
                expected: 13,
                actual: 12
            })
        ));
        assert!(matches!(
            decode_refine_query_row(&short),
            Err(RefineRowError::ColumnCount {
                expected: 13,
                actual: 12
            })
        ));
        let long = RefineTableQueryRow::new(vec![value("1"); 14]);
        assert!(matches!(
            build_refine_section(&[long]),
            Err(RefineSectionError::Row {
                source: RefineRowError::ColumnCount { actual: 14, .. },
                ..
            })
        ));
    }

    #[test]
    fn material_count_counts_nonzero_vnums_not_nonzero_counts() {
        let row = row_with_pairs(&[(0, 123), (7, 0), (0, 0), (9, -1), (11, 0)]);
        let record = decode_refine_query_row(&row).unwrap();
        assert_eq!(record.material_count, 3);
        assert_eq!(record.materials[0].vnum, 0);
        assert_eq!(record.materials[0].count, 123);
    }

    #[test]
    fn record_and_byte_limits_are_checked_before_encoding() {
        let row = row_with_pairs(&[(1, 1), (0, 0), (0, 0), (0, 0), (0, 0)]);
        assert!(matches!(
            build_refine_section_with_limit(&[row.clone(), row.clone()], 1),
            Err(RefineSectionError::TooManyRecords {
                count: 2,
                maximum: 1
            })
        ));
        let result = build_refine_section_with_limits(
            &[row],
            RefineSectionLimits::with_data_limit(1, REFINE_TABLE_WIRE_SIZE - 1),
        );
        assert!(matches!(
            result,
            Err(RefineSectionError::DataTooLarge {
                length: 53,
                maximum: 52
            })
        ));
    }

    #[test]
    fn injected_source_errors_are_not_converted_to_empty_sections() {
        let source =
            || -> Result<Vec<RefineTableQueryRow>, &'static str> { Err("database unavailable") };
        assert_eq!(
            load_refine_section(&source, RefineSectionLimits::default()),
            Err(RefineLoadError::Source("database unavailable"))
        );
    }
}
