//! The `refine_proto` table rule.
//!
//! The legacy DB loader selects thirteen columns from `refine_proto`:
//! `id`, `cost`, `prob`, and five `(vnum, count)` pairs.  This module keeps
//! the values as query cells until a complete row has been checked.  In
//! particular, SQL `NULL` and a column extraction error are not silently
//! converted to zero.

use std::error::Error;
use std::fmt;

use crate::records::{RefineMaterialRecord, RefineTableRecord};

/// Number of columns selected by the fixed legacy refine query.
pub const REFINE_TABLE_QUERY_COLUMNS: usize = 13;

/// Alias for [`REFINE_TABLE_QUERY_COLUMNS`].
pub const REFINE_QUERY_COLUMN_COUNT: usize = REFINE_TABLE_QUERY_COLUMNS;

/// Default record cap: the legacy boot stream counted records in a `WORD`,
/// so no legacy table held more.
pub const REFINE_TABLE_MAX_RECORDS: usize = u16::MAX as usize;

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

/// The record cap checked before a refine table is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefineLimits {
    /// Maximum number of source rows accepted.
    pub max_records: usize,
}

impl RefineLimits {
    /// Limits with a caller-selected row cap.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self { max_records }
    }
}

impl Default for RefineLimits {
    fn default() -> Self {
        Self::new(REFINE_TABLE_MAX_RECORDS)
    }
}

/// A checked failure while building the refine table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineTableError {
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
    /// A source row could not be strictly decoded.
    Row {
        /// Zero-based source row index.
        index: usize,
        /// Row error.
        source: RefineRowError,
    },
}

impl fmt::Display for RefineTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "refine table has {count} rows; configured limit is {maximum}"
            ),
            Self::AllocationFailed { requested } => {
                write!(formatter, "refine allocation of {requested} records failed")
            }
            Self::Row { index, source } => {
                write!(formatter, "refine row {index} is invalid: {source}")
            }
        }
    }
}

impl Error for RefineTableError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
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

/// Decode rows into refine records, preserving source row order.
///
/// The row cap is checked before the output is reserved.
///
/// # Errors
///
/// Returns [`RefineTableError`] for an invalid row, the row cap, or an
/// allocation failure.
pub fn build_refine_table(
    rows: &[RefineTableQueryRow],
    limits: RefineLimits,
) -> Result<Vec<RefineTableRecord>, RefineTableError> {
    if rows.len() > limits.max_records {
        return Err(RefineTableError::TooManyRecords {
            count: rows.len(),
            maximum: limits.max_records,
        });
    }
    let mut records = Vec::new();
    records
        .try_reserve_exact(rows.len())
        .map_err(|_| RefineTableError::AllocationFailed {
            requested: rows.len(),
        })?;
    for (index, row) in rows.iter().enumerate() {
        let record = decode_refine_query_row(row)
            .map_err(|source| RefineTableError::Row { index, source })?;
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let decoded = build_refine_table(&[first, second], RefineLimits::default()).unwrap();

        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].id, 42);
        assert_eq!(decoded[0].material_count, 3);
        assert_eq!(decoded[0].cost, -7);
        assert_eq!(decoded[0].prob, 91);
        assert_eq!(decoded[1].id, 42);
        assert_eq!(decoded[1].materials[1].vnum, 2);
    }

    #[test]
    fn empty_source_produces_an_empty_table() {
        assert!(build_refine_table(&[], RefineLimits::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn null_is_not_silently_converted_to_zero() {
        let mut columns = vec![value("1"), RefineQueryValue::null()];
        columns.extend((0..11).map(|_| value("0")));
        let rows = [RefineTableQueryRow::new(columns)];
        let error = build_refine_table(&rows, RefineLimits::default()).unwrap_err();
        assert!(matches!(
            error,
            RefineTableError::Row {
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
            build_refine_table(&[long], RefineLimits::default()),
            Err(RefineTableError::Row {
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
    fn the_record_cap_is_checked_before_decoding() {
        let row = row_with_pairs(&[(1, 1), (0, 0), (0, 0), (0, 0), (0, 0)]);
        assert!(matches!(
            build_refine_table(&[row.clone(), row], RefineLimits::new(1)),
            Err(RefineTableError::TooManyRecords {
                count: 2,
                maximum: 1
            })
        ));
    }
}
