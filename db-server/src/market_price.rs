//! Source-verified, SQL-free optional premium private-shop market-price loading.
//!
//! Legacy evidence comes from
//! `server/server/db/ClientManagerPrivateShop.cpp:849-908`. The active optional
//! loader builds this statement in a 512-byte C buffer:
//!
//! ```text
//! SELECT vnum, gold, cheque FROM private_shop_sale_history%s WHERE DATEDIFF(time, FROM_UNIXTIME(%u)) < 3
//! ```
//!
//! The table postfix and current Unix seconds are supplied by the caller. The
//! three-day interval is fixed by
//! `VALID_MARKET_PRICE_DAY_INTERVAL` in `server/server/common/length.h`.
//! [`MarketPriceQuery`] checks the statement against the buffer before it is
//! returned. It does not execute SQL.
//!
//! Each MYSQL text cell reaches a zero-initialized C++ destination through
//! `str_to_number`. The strict decoder requires non-NULL, nonempty decimal
//! values in their exact Rust target widths. The explicitly named legacy
//! decoder instead models the x86 C conversions: bytes end at the first NUL,
//! leading ASCII C whitespace and one sign are accepted, conversion stops at
//! the first nondigit, unsigned conversion saturates at its x86 width, and the
//! result is narrowed or cast to the destination. No lossy UTF-8 conversion
//! occurs before either decoder parses a cell.
//!
//! Rows are sorted by vnum and grouped. Gold and cheque sums are divided by the
//! group size only after all rows have been accumulated. Gold uses explicit
//! Rust wrapping addition. The legacy C++ signed addition can overflow and
//! therefore has undefined behavior; wrapping is a deterministic migration
//! boundary, not a claim that every overflowing C++ execution wraps. Cheque
//! addition is defined modulo 2^32 in both boundaries.
//!
//! The legacy boot sender iterates an `unordered_map`, so neither SQL row order
//! nor legacy output order is stable. This module intentionally emits one
//! record per vnum in ascending vnum order. This is a deterministic migration
//! choice, not a recovered legacy order guarantee.
//!
//! The result is an optional [`BootSection`] containing the source-fixed
//! 16-byte `TMarketItemPrice` records. An empty source produces an empty
//! section with the correct kind and record width. This module does not select
//! a boot feature profile, compose a boot response, manage a cache, or provide
//! a live database/boot caller.

use std::error::Error;
use std::fmt;

use protocol::db_boot::{BootSection, BootSectionKind};
use protocol::db_records::{MarketItemPriceRecord, MARKET_ITEM_PRICE_WIRE_SIZE};

use crate::postfix::{TablePostfix, TablePostfixError, MAX_TABLE_POSTFIX_BYTES};

/// Base name of the legacy private-shop sale-history table.
pub const MARKET_PRICE_TABLE: &str = "private_shop_sale_history";

/// Exact selected columns in source order.
pub const MARKET_PRICE_QUERY_COLUMNS: [&str; 3] = ["vnum", "gold", "cheque"];

/// Compatibility alias for [`MARKET_PRICE_QUERY_COLUMNS`].
pub const MARKET_PRICE_QUERY_COLUMN_NAMES: [&str; 3] = MARKET_PRICE_QUERY_COLUMNS;

/// Number of columns selected by the market-price query.
pub const MARKET_PRICE_QUERY_COLUMN_COUNT: usize = MARKET_PRICE_QUERY_COLUMNS.len();

/// Exact fixed-column prefix before the generated table identifier.
pub const MARKET_PRICE_QUERY_PREFIX: &str = "SELECT vnum, gold, cheque FROM ";

/// Fixed valid-history interval, in days, from `VALID_MARKET_PRICE_DAY_INTERVAL`.
pub const MARKET_PRICE_DAY_INTERVAL: u32 = 3;

/// Size of the legacy `char query[512]` statement buffer, including its NUL.
pub const MARKET_PRICE_QUERY_BUFFER_BYTES: usize = 512;

/// Maximum statement bytes that fit in [`MARKET_PRICE_QUERY_BUFFER_BYTES`].
pub const MARKET_PRICE_MAX_QUERY_BYTES: usize = MARKET_PRICE_QUERY_BUFFER_BYTES - 1;

/// Maximum unique records representable by the boot `u16` count.
///
/// This is the exact decimal value of `u16::MAX`. It is written as a literal
/// because `usize::from(u16::MAX)` was not a constant operation on the
/// workspace's Rust 1.85 toolchain.
pub const MARKET_PRICE_MAX_RECORDS: usize = 65_535;

/// Maximum packed section-data bytes at [`MARKET_PRICE_MAX_RECORDS`].
pub const MARKET_PRICE_MAX_SECTION_BYTES: usize =
    MARKET_ITEM_PRICE_WIRE_SIZE * MARKET_PRICE_MAX_RECORDS;

/// Maximum bytes in a postfix-qualified market-price table identifier.
pub const MARKET_PRICE_MAX_TABLE_NAME_BYTES: usize =
    MARKET_PRICE_TABLE.len() + MAX_TABLE_POSTFIX_BYTES;

/// Internal spelling retained for focused invariant tests.
const MAX_MARKET_PRICE_TABLE_NAME_BYTES: usize = MARKET_PRICE_MAX_TABLE_NAME_BYTES;

/// A defensive failure while composing the market-price read statement.
///
/// A value obtained through [`TablePostfix`] should already satisfy the
/// postfix and identifier checks. These variants remain reachable only if a
/// future private invariant changes or an internal constructor is misused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketPriceQueryBuildError {
    /// The postfix exceeded the maximum retained by the legacy config copy.
    PostfixTooLong {
        /// Postfix byte length.
        length: usize,
        /// Maximum accepted postfix byte length.
        maximum: usize,
    },
    /// The postfix contained a byte outside the validated identifier set.
    InvalidPostfixCharacter {
        /// Zero-based postfix byte offset.
        index: usize,
        /// Disallowed byte.
        byte: u8,
    },
    /// The generated table identifier exceeded its bounded width.
    TableNameTooLong {
        /// Generated table-name byte length.
        length: usize,
        /// Maximum accepted table-name byte length.
        maximum: usize,
    },
    /// The generated table identifier contained a disallowed byte.
    InvalidTableIdentifier {
        /// Zero-based generated-identifier byte offset.
        index: usize,
        /// Disallowed byte.
        byte: u8,
    },
    /// The complete statement did not fit the source-fixed query buffer.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for MarketPriceQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PostfixTooLong { length, maximum } => write!(
                formatter,
                "market-price postfix is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidPostfixCharacter { index, byte } => write!(
                formatter,
                "market-price postfix byte {byte:#04x} at offset {index} is not allowed"
            ),
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated market-price table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated market-price identifier byte {byte:#04x} at offset {index} is not allowed"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "market-price query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for MarketPriceQueryBuildError {}

/// One immutable, checked market-price read statement.
///
/// The current Unix seconds are formatted as an unsigned decimal with no
/// sign or spaces. Only a validated [`TablePostfix`] can supply the table
/// suffix. This type does not provide a generic SQL formatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketPriceQuery {
    statement: String,
    table_name: String,
    postfix: TablePostfix,
    unix_seconds: u32,
}

impl MarketPriceQuery {
    /// Build the exact source-fixed statement from a validated postfix.
    ///
    /// The postfix, generated identifier, and final statement are checked
    /// again before the query is returned.
    ///
    /// # Errors
    ///
    /// Returns [`MarketPriceQueryBuildError`] if a defensive postfix,
    /// identifier, or statement-width invariant fails.
    pub fn new(
        postfix: &TablePostfix,
        unix_seconds: u32,
    ) -> Result<Self, MarketPriceQueryBuildError> {
        validate_query_postfix(postfix.as_str())?;
        let table_name = format!("{MARKET_PRICE_TABLE}{}", postfix.as_str());
        validate_query_table_name(&table_name)?;

        let statement = format!(
            "{MARKET_PRICE_QUERY_PREFIX}{table_name} WHERE DATEDIFF(time, FROM_UNIXTIME({unix_seconds})) < {MARKET_PRICE_DAY_INTERVAL}"
        );
        validate_query_statement(&statement)?;

        Ok(Self {
            statement,
            table_name,
            postfix: postfix.clone(),
            unix_seconds,
        })
    }

    /// Validate optional raw configuration and build the fixed query.
    ///
    /// # Errors
    ///
    /// Returns [`MarketPriceBoundaryError::Postfix`] when `TABLE_POSTFIX` is
    /// invalid, or [`MarketPriceBoundaryError::Query`] when a defensive query
    /// invariant fails.
    pub fn from_config(
        configured_postfix: Option<&str>,
        unix_seconds: u32,
    ) -> Result<Self, MarketPriceBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(MarketPriceBoundaryError::Postfix)?;
        Self::new(&postfix, unix_seconds).map_err(MarketPriceBoundaryError::Query)
    }

    /// Borrow the exact statement text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Borrow the generated postfix-qualified table name.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix used to construct this query.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }

    /// Return the caller-supplied current Unix seconds used in the statement.
    #[must_use]
    pub const fn unix_seconds(&self) -> u32 {
        self.unix_seconds
    }
}

/// A failure while validating a market-price postfix or fixed query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketPriceBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The fixed query failed a defensive construction check.
    Query(MarketPriceQueryBuildError),
}

impl fmt::Display for MarketPriceBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for MarketPriceBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

fn is_market_price_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn validate_query_postfix(postfix: &str) -> Result<(), MarketPriceQueryBuildError> {
    if postfix.len() > MAX_TABLE_POSTFIX_BYTES {
        return Err(MarketPriceQueryBuildError::PostfixTooLong {
            length: postfix.len(),
            maximum: MAX_TABLE_POSTFIX_BYTES,
        });
    }
    if let Some((index, byte)) = postfix
        .bytes()
        .enumerate()
        .find(|(_, byte)| !is_market_price_identifier_byte(*byte))
    {
        return Err(MarketPriceQueryBuildError::InvalidPostfixCharacter { index, byte });
    }
    Ok(())
}

fn validate_query_table_name(table_name: &str) -> Result<(), MarketPriceQueryBuildError> {
    if table_name.len() > MAX_MARKET_PRICE_TABLE_NAME_BYTES {
        return Err(MarketPriceQueryBuildError::TableNameTooLong {
            length: table_name.len(),
            maximum: MAX_MARKET_PRICE_TABLE_NAME_BYTES,
        });
    }
    if let Some((index, byte)) = table_name
        .bytes()
        .enumerate()
        .find(|(_, byte)| !is_market_price_identifier_byte(*byte))
    {
        return Err(MarketPriceQueryBuildError::InvalidTableIdentifier { index, byte });
    }
    Ok(())
}

fn validate_query_statement(statement: &str) -> Result<(), MarketPriceQueryBuildError> {
    if statement.len() > MARKET_PRICE_MAX_QUERY_BYTES {
        return Err(MarketPriceQueryBuildError::QueryTooLong {
            length: statement.len(),
            maximum: MARKET_PRICE_MAX_QUERY_BYTES,
        });
    }
    Ok(())
}

/// One fixed-width, source-shaped market-price row.
///
/// Each cell is raw MYSQL text. `None` represents SQL `NULL`; `Some` retains
/// arbitrary bytes until the selected decoder handles them. The row has exactly
/// three named cells, so a row cannot have a driver-reported column-count
/// mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketPriceQueryRow {
    columns: [Option<Vec<u8>>; MARKET_PRICE_QUERY_COLUMN_COUNT],
}

impl MarketPriceQueryRow {
    /// Construct a row from the three raw cells in query order.
    #[must_use]
    pub fn new(vnum: Option<Vec<u8>>, gold: Option<Vec<u8>>, cheque: Option<Vec<u8>>) -> Self {
        Self::from_typed_columns([vnum, gold, cheque])
    }

    /// Construct a row from a statically sized cell array.
    #[must_use]
    pub fn from_cells(cells: [Option<Vec<u8>>; MARKET_PRICE_QUERY_COLUMN_COUNT]) -> Self {
        Self::from_typed_columns(cells)
    }

    /// Construct a row from a statically typed three-cell array.
    ///
    /// This is an alias for [`Self::from_cells`] and makes the fixed source
    /// shape explicit at call sites.
    #[must_use]
    pub fn from_typed_columns(columns: [Option<Vec<u8>>; MARKET_PRICE_QUERY_COLUMN_COUNT]) -> Self {
        Self { columns }
    }

    /// Borrow all three owned raw cells in query order.
    #[must_use]
    pub const fn columns(&self) -> &[Option<Vec<u8>>; MARKET_PRICE_QUERY_COLUMN_COUNT] {
        &self.columns
    }

    /// Borrow the raw `vnum` cell, or `None` for SQL `NULL`.
    #[must_use]
    pub fn vnum(&self) -> Option<&[u8]> {
        self.columns[0].as_deref()
    }

    /// Borrow the raw `gold` cell, or `None` for SQL `NULL`.
    #[must_use]
    pub fn gold(&self) -> Option<&[u8]> {
        self.columns[1].as_deref()
    }

    /// Borrow the raw `cheque` cell, or `None` for SQL `NULL`.
    #[must_use]
    pub fn cheque(&self) -> Option<&[u8]> {
        self.columns[2].as_deref()
    }

    /// Borrow all three cells as byte slices in query order.
    #[must_use]
    pub fn cells(&self) -> [Option<&[u8]>; MARKET_PRICE_QUERY_COLUMN_COUNT] {
        [
            self.columns[0].as_deref(),
            self.columns[1].as_deref(),
            self.columns[2].as_deref(),
        ]
    }

    /// Consume the row and return all three owned raw cells in query order.
    #[must_use]
    pub fn into_columns(self) -> [Option<Vec<u8>>; MARKET_PRICE_QUERY_COLUMN_COUNT] {
        self.columns
    }

    /// Alias for [`Self::into_columns`].
    #[must_use]
    pub fn into_cells(self) -> [Option<Vec<u8>>; MARKET_PRICE_QUERY_COLUMN_COUNT] {
        self.into_columns()
    }
}

impl From<[Option<Vec<u8>>; MARKET_PRICE_QUERY_COLUMN_COUNT]> for MarketPriceQueryRow {
    fn from(columns: [Option<Vec<u8>>; MARKET_PRICE_QUERY_COLUMN_COUNT]) -> Self {
        Self::from_typed_columns(columns)
    }
}

/// A strict numeric decoding error in one market-price row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketPriceRowError {
    /// A required source cell was SQL `NULL`.
    Null {
        /// Zero-based query column index.
        column: usize,
    },
    /// A non-NULL source cell was empty.
    Empty {
        /// Zero-based query column index.
        column: usize,
    },
    /// A source cell was not UTF-8 and therefore was not a strict text value.
    NonUtf8 {
        /// Zero-based query column index.
        column: usize,
        /// Lossless source bytes.
        value: Vec<u8>,
    },
    /// A cell was not an exact decimal integer in the accepted grammar.
    InvalidNumber {
        /// Zero-based query column index.
        column: usize,
        /// Lossless source bytes.
        value: Vec<u8>,
        /// Target integer type.
        target: &'static str,
    },
    /// A valid decimal integer did not fit its exact target width.
    NumberOverflow {
        /// Zero-based query column index.
        column: usize,
        /// Lossless source bytes.
        value: Vec<u8>,
        /// Target integer type.
        target: &'static str,
    },
}

impl MarketPriceRowError {
    fn column_name(column: usize) -> &'static str {
        match column {
            0 => "vnum",
            1 => "gold",
            2 => "cheque",
            _ => "unknown",
        }
    }
}

impl fmt::Display for MarketPriceRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null { column } => write!(
                formatter,
                "market-price column {} is NULL",
                Self::column_name(*column)
            ),
            Self::Empty { column } => write!(
                formatter,
                "market-price column {} is empty",
                Self::column_name(*column)
            ),
            Self::NonUtf8 { column, value } => write!(
                formatter,
                "market-price column {} bytes {value:?} are not UTF-8",
                Self::column_name(*column)
            ),
            Self::InvalidNumber {
                column,
                value,
                target,
            } => write!(
                formatter,
                "market-price column {} bytes {value:?} are not a strict {target}",
                Self::column_name(*column)
            ),
            Self::NumberOverflow {
                column,
                value,
                target,
            } => write!(
                formatter,
                "market-price column {} bytes {value:?} overflow {target}",
                Self::column_name(*column)
            ),
        }
    }
}

impl Error for MarketPriceRowError {}

fn is_strict_unsigned_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_strict_signed_decimal(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first().copied() else {
        return false;
    };
    let digits = if first == b'-' { &bytes[1..] } else { bytes };
    !digits.is_empty() && digits.iter().copied().all(|byte| byte.is_ascii_digit())
}

fn strict_cell(value: Option<&[u8]>, column: usize) -> Result<&str, MarketPriceRowError> {
    let bytes = value.ok_or(MarketPriceRowError::Null { column })?;
    if bytes.is_empty() {
        return Err(MarketPriceRowError::Empty { column });
    }
    std::str::from_utf8(bytes).map_err(|_| MarketPriceRowError::NonUtf8 {
        column,
        value: bytes.to_vec(),
    })
}

fn strict_u32(value: Option<&[u8]>, column: usize) -> Result<u32, MarketPriceRowError> {
    let text = strict_cell(value, column)?;
    if !is_strict_unsigned_decimal(text) {
        return Err(MarketPriceRowError::InvalidNumber {
            column,
            value: text.as_bytes().to_vec(),
            target: "u32",
        });
    }
    text.parse::<u32>()
        .map_err(|_| MarketPriceRowError::NumberOverflow {
            column,
            value: text.as_bytes().to_vec(),
            target: "u32",
        })
}

fn strict_i64(value: Option<&[u8]>, column: usize) -> Result<i64, MarketPriceRowError> {
    let text = strict_cell(value, column)?;
    if !is_strict_signed_decimal(text) {
        return Err(MarketPriceRowError::InvalidNumber {
            column,
            value: text.as_bytes().to_vec(),
            target: "i64",
        });
    }
    text.parse::<i64>()
        .map_err(|_| MarketPriceRowError::NumberOverflow {
            column,
            value: text.as_bytes().to_vec(),
            target: "i64",
        })
}

/// Strictly decode one three-cell market-price row.
///
/// The vnum and cheque cells must be nonempty ASCII decimal `u32` values.
/// Gold must be a nonempty ASCII decimal `i64`, with an optional leading `-`.
/// Whitespace, `+`, prefixes, NUL bytes, and non-UTF-8 data are rejected.
/// Values outside the exact target width are rejected rather than truncated.
///
/// # Errors
///
/// Returns [`MarketPriceRowError`] for a NULL, empty, non-UTF-8, malformed, or
/// overflowing cell.
pub fn decode_market_price_query_row(
    row: &MarketPriceQueryRow,
) -> Result<MarketItemPriceRecord, MarketPriceRowError> {
    Ok(MarketItemPriceRecord {
        vnum: strict_u32(row.vnum(), 0)?,
        gold: strict_i64(row.gold(), 1)?,
        cheque: strict_u32(row.cheque(), 2)?,
    })
}

/// Decode one row with the explicitly modeled legacy C conversion policy.
///
/// `None`, an empty C string, leading whitespace without digits, or a sign
/// without digits leaves the zero-initialized destination at zero. Conversion
/// ends at the first NUL. One leading ASCII C whitespace run and one optional
/// sign are accepted. Decimal conversion stops at the first nondigit. On the
/// active x86 build, `strtoul` saturates at 32 bits and `strtoull` saturates at
/// 64 bits before the destination cast. Negative unsigned conversions use
/// unsigned negation, and the signed 64-bit result uses the same bit pattern.
///
/// This function is intentionally separate from the strict decoder. It does
/// not establish live database parity beyond the cited conversion helpers.
///
/// # Errors
///
/// This source-shaped, fixed-width operation has no error case. The result
/// type is retained so both row decoders have the same calling shape.
pub fn decode_market_price_query_row_legacy(
    row: &MarketPriceQueryRow,
) -> Result<MarketItemPriceRecord, MarketPriceRowError> {
    Ok(MarketItemPriceRecord {
        vnum: legacy_x86_dword(row.vnum()),
        gold: legacy_x86_long_long(row.gold()),
        cheque: legacy_x86_dword(row.cheque()),
    })
}

/// Alias for [`decode_market_price_query_row`].
///
/// # Errors
///
/// Returns [`MarketPriceRowError`] when a strict source cell is missing,
/// malformed, or outside its destination width.
pub fn decode_market_price_row(
    row: &MarketPriceQueryRow,
) -> Result<MarketItemPriceRecord, MarketPriceRowError> {
    decode_market_price_query_row(row)
}

/// Alias for [`decode_market_price_query_row_legacy`].
///
/// # Errors
///
/// The modeled legacy conversion has no error case; the result type is kept
/// identical to the strict decoder for call-site symmetry.
pub fn decode_market_price_row_legacy(
    row: &MarketPriceQueryRow,
) -> Result<MarketItemPriceRecord, MarketPriceRowError> {
    decode_market_price_query_row_legacy(row)
}

fn is_ascii_c_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn legacy_c_string_prefix(bytes: &[u8]) -> &[u8] {
    let content_len = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    &bytes[..content_len]
}

fn x86_dword_decimal_prefix(bytes: &[u8]) -> u32 {
    let bytes = legacy_c_string_prefix(bytes);
    let mut index = 0;
    while index < bytes.len() && is_ascii_c_whitespace(bytes[index]) {
        index += 1;
    }

    let negative = match bytes.get(index) {
        Some(b'-') => {
            index += 1;
            true
        }
        Some(b'+') => {
            index += 1;
            false
        }
        _ => false,
    };

    if bytes.get(index).is_none_or(|byte| !byte.is_ascii_digit()) {
        return 0;
    }

    let mut value = 0_u32;
    while index < bytes.len() {
        let byte = bytes[index];
        if !byte.is_ascii_digit() {
            break;
        }
        let digit = u32::from(byte - b'0');
        value = match value
            .checked_mul(10)
            .and_then(|current| current.checked_add(digit))
        {
            Some(current) => current,
            None => return u32::MAX,
        };
        index += 1;
    }

    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

fn x86_ulonglong_decimal_prefix(bytes: &[u8]) -> u64 {
    let bytes = legacy_c_string_prefix(bytes);
    let mut index = 0;
    while index < bytes.len() && is_ascii_c_whitespace(bytes[index]) {
        index += 1;
    }

    let negative = match bytes.get(index) {
        Some(b'-') => {
            index += 1;
            true
        }
        Some(b'+') => {
            index += 1;
            false
        }
        _ => false,
    };

    if bytes.get(index).is_none_or(|byte| !byte.is_ascii_digit()) {
        return 0;
    }

    let mut value = 0_u64;
    while index < bytes.len() {
        let byte = bytes[index];
        if !byte.is_ascii_digit() {
            break;
        }
        let digit = u64::from(byte - b'0');
        value = match value
            .checked_mul(10)
            .and_then(|current| current.checked_add(digit))
        {
            Some(current) => current,
            None => return u64::MAX,
        };
        index += 1;
    }

    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

fn legacy_x86_dword(value: Option<&[u8]>) -> u32 {
    x86_dword_decimal_prefix(value.unwrap_or(&[]))
}

fn legacy_x86_long_long(value: Option<&[u8]>) -> i64 {
    let converted = x86_ulonglong_decimal_prefix(value.unwrap_or(&[]));
    // `from_ne_bytes` preserves the numeric bit pattern on the supported
    // two's-complement targets and performs the C++ unsigned-to-signed cast
    // without a lossy value conversion.
    i64::from_ne_bytes(converted.to_ne_bytes())
}

/// Limits applied before a market-price section is allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketPriceSectionLimits {
    /// Maximum source rows accepted, including rows that share a vnum.
    pub max_source_rows: usize,
    /// Maximum unique vnum records emitted.
    pub max_records: usize,
    /// Maximum packed section-data bytes emitted.
    pub max_data_bytes: usize,
}

impl MarketPriceSectionLimits {
    /// Construct limits with source and unique-record caps equal to `max_records`.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            max_source_rows: max_records,
            max_records,
            max_data_bytes: MARKET_PRICE_MAX_SECTION_BYTES,
        }
    }

    /// Construct all three caller-selected bounds explicitly.
    #[must_use]
    pub const fn with_limits(
        max_source_rows: usize,
        max_records: usize,
        max_data_bytes: usize,
    ) -> Self {
        Self {
            max_source_rows,
            max_records,
            max_data_bytes,
        }
    }
}

impl Default for MarketPriceSectionLimits {
    fn default() -> Self {
        Self::new(MARKET_PRICE_MAX_RECORDS)
    }
}

/// A checked failure while converting market-price rows to a boot section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketPriceSectionError {
    /// The source supplied more rows than allowed.
    TooManySourceRows {
        /// Supplied source-row count.
        count: usize,
        /// Configured source-row limit.
        maximum: usize,
    },
    /// The source contains more unique vnums than allowed.
    TooManyRecords {
        /// Unique vnum count.
        count: usize,
        /// Configured unique-record limit.
        maximum: usize,
    },
    /// The unique output count cannot be represented by the boot `u16` count.
    CountOverflow {
        /// Unique output count.
        count: usize,
    },
    /// A group size was zero or could not be represented by an averaging divisor.
    InvalidGroupSize {
        /// Invalid source group size.
        group_size: usize,
    },
    /// The fixed record width cannot be represented by the boot `u16` width.
    RecordSizeOverflow {
        /// Fixed record width in bytes.
        size: usize,
    },
    /// Checked packed data-size multiplication overflowed `usize`.
    DataSizeOverflow {
        /// Output record count.
        count: usize,
        /// Record width used in the multiplication.
        record_size: usize,
    },
    /// The required packed data length exceeds the configured byte cap.
    DataTooLarge {
        /// Required packed data length.
        length: usize,
        /// Configured byte cap.
        maximum: usize,
    },
    /// A source row could not be decoded.
    Row {
        /// Zero-based source-row index.
        index: usize,
        /// Strict row error.
        source: MarketPriceRowError,
    },
    /// The decoded source-record vector could not reserve its capacity.
    RecordAllocationFailed {
        /// Requested record-vector capacity.
        requested_records: usize,
    },
    /// The packed output vector could not reserve its checked capacity.
    DataAllocationFailed {
        /// Requested output-data byte capacity.
        requested_bytes: usize,
    },
    /// The protocol encoder returned an unexpected fixed record width.
    RecordSizeMismatch {
        /// Zero-based output-record index.
        index: usize,
        /// Required record width.
        expected: usize,
        /// Actual encoded width.
        actual: usize,
    },
}

impl fmt::Display for MarketPriceSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySourceRows { count, maximum } => write!(
                formatter,
                "market-price source has {count} rows; maximum is {maximum}"
            ),
            Self::TooManyRecords { count, maximum } => write!(
                formatter,
                "market-price source has {count} unique vnums; maximum is {maximum}"
            ),
            Self::CountOverflow { count } => write!(
                formatter,
                "market-price record count {count} does not fit u16"
            ),
            Self::InvalidGroupSize { group_size } => write!(
                formatter,
                "market-price group size {group_size} is not a valid averaging divisor"
            ),
            Self::RecordSizeOverflow { size } => write!(
                formatter,
                "market-price record width {size} does not fit u16"
            ),
            Self::DataSizeOverflow { count, record_size } => write!(
                formatter,
                "market-price data size overflows usize for {count} records of {record_size} bytes"
            ),
            Self::DataTooLarge { length, maximum } => write!(
                formatter,
                "market-price data length {length} exceeds limit {maximum}"
            ),
            Self::Row { index, source } => {
                write!(formatter, "market-price row {index} is invalid: {source}")
            }
            Self::RecordAllocationFailed { requested_records } => write!(
                formatter,
                "could not reserve {requested_records} decoded market-price records"
            ),
            Self::DataAllocationFailed { requested_bytes } => write!(
                formatter,
                "could not reserve {requested_bytes} market-price output bytes"
            ),
            Self::RecordSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "market-price record {index} encoded to {actual} bytes; expected {expected}"
            ),
        }
    }
}

impl Error for MarketPriceSectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Row { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn checked_market_price_data_length(
    count: usize,
    record_size: usize,
) -> Result<usize, MarketPriceSectionError> {
    record_size
        .checked_mul(count)
        .ok_or(MarketPriceSectionError::DataSizeOverflow { count, record_size })
}

fn validate_section_shape(
    count: usize,
    record_size: usize,
    limits: MarketPriceSectionLimits,
) -> Result<(u16, u16, usize), MarketPriceSectionError> {
    if count > limits.max_records {
        return Err(MarketPriceSectionError::TooManyRecords {
            count,
            maximum: limits.max_records,
        });
    }
    let wire_count =
        u16::try_from(count).map_err(|_| MarketPriceSectionError::CountOverflow { count })?;
    let wire_record_size = u16::try_from(record_size)
        .map_err(|_| MarketPriceSectionError::RecordSizeOverflow { size: record_size })?;
    let data_len = checked_market_price_data_length(count, record_size)?;
    if data_len > limits.max_data_bytes {
        return Err(MarketPriceSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }
    Ok((wire_count, wire_record_size, data_len))
}

fn allocate_decoded_records(
    requested_records: usize,
) -> Result<Vec<MarketItemPriceRecord>, MarketPriceSectionError> {
    let mut records = Vec::new();
    records
        .try_reserve_exact(requested_records)
        .map_err(|_| MarketPriceSectionError::RecordAllocationFailed { requested_records })?;
    Ok(records)
}

fn allocate_section_data(requested_bytes: usize) -> Result<Vec<u8>, MarketPriceSectionError> {
    let mut data = Vec::new();
    data.try_reserve_exact(requested_bytes)
        .map_err(|_| MarketPriceSectionError::DataAllocationFailed { requested_bytes })?;
    Ok(data)
}

fn validate_encoded_record(index: usize, encoded: &[u8]) -> Result<(), MarketPriceSectionError> {
    if encoded.len() != MARKET_ITEM_PRICE_WIRE_SIZE {
        return Err(MarketPriceSectionError::RecordSizeMismatch {
            index,
            expected: MARKET_ITEM_PRICE_WIRE_SIZE,
            actual: encoded.len(),
        });
    }
    Ok(())
}

fn average_i64_wrapping_sum(value: i64, group_size: usize) -> Result<i64, MarketPriceSectionError> {
    let divisor = i64::try_from(group_size)
        .map_err(|_| MarketPriceSectionError::InvalidGroupSize { group_size })?;
    value
        .checked_div(divisor)
        .ok_or(MarketPriceSectionError::InvalidGroupSize { group_size })
}

fn average_u32_wrapping_sum(value: u32, group_size: usize) -> Result<u32, MarketPriceSectionError> {
    let divisor = u32::try_from(group_size)
        .map_err(|_| MarketPriceSectionError::InvalidGroupSize { group_size })?;
    value
        .checked_div(divisor)
        .ok_or(MarketPriceSectionError::InvalidGroupSize { group_size })
}

fn build_market_price_section_with_decoder<F>(
    rows: &[MarketPriceQueryRow],
    limits: MarketPriceSectionLimits,
    mut decode: F,
) -> Result<BootSection, MarketPriceSectionError>
where
    F: FnMut(&MarketPriceQueryRow) -> Result<MarketItemPriceRecord, MarketPriceRowError>,
{
    if rows.len() > limits.max_source_rows {
        return Err(MarketPriceSectionError::TooManySourceRows {
            count: rows.len(),
            maximum: limits.max_source_rows,
        });
    }

    let mut decoded = allocate_decoded_records(rows.len())?;
    for (index, row) in rows.iter().enumerate() {
        let record =
            decode(row).map_err(|source| MarketPriceSectionError::Row { index, source })?;
        decoded.push(record);
    }

    // SQL has no ORDER BY in the source statement. Sorting first makes the
    // unique-record order independent of database row order.
    decoded.sort_unstable_by_key(|record| record.vnum);

    let mut record_count = 0_usize;
    let mut previous_vnum = None;
    for record in &decoded {
        if previous_vnum != Some(record.vnum) {
            record_count = record_count
                .checked_add(1)
                .ok_or(MarketPriceSectionError::CountOverflow { count: usize::MAX })?;
            previous_vnum = Some(record.vnum);
        }
    }

    let (count, record_size, data_len) =
        validate_section_shape(record_count, MARKET_ITEM_PRICE_WIRE_SIZE, limits)?;
    let mut data = allocate_section_data(data_len)?;

    let mut group_start = 0_usize;
    let mut output_index = 0_usize;
    while group_start < decoded.len() {
        let vnum = decoded[group_start].vnum;
        let mut group_end = group_start
            .checked_add(1)
            .ok_or(MarketPriceSectionError::CountOverflow { count: usize::MAX })?;
        while group_end < decoded.len() && decoded[group_end].vnum == vnum {
            group_end = group_end
                .checked_add(1)
                .ok_or(MarketPriceSectionError::CountOverflow { count: usize::MAX })?;
        }

        let group_size = group_end - group_start;
        let mut gold_sum = 0_i64;
        let mut cheque_sum = 0_u32;
        for record in &decoded[group_start..group_end] {
            // Rust wrapping is explicit. The C++ signed operation can invoke
            // undefined behavior on overflow, so this is not a general C++
            // overflow guarantee.
            gold_sum = gold_sum.wrapping_add(record.gold);
            cheque_sum = cheque_sum.wrapping_add(record.cheque);
        }

        let average = MarketItemPriceRecord {
            vnum,
            gold: average_i64_wrapping_sum(gold_sum, group_size)?,
            cheque: average_u32_wrapping_sum(cheque_sum, group_size)?,
        };
        let encoded = average.encode();
        validate_encoded_record(output_index, &encoded)?;
        data.extend_from_slice(&encoded);

        output_index = output_index
            .checked_add(1)
            .ok_or(MarketPriceSectionError::CountOverflow { count: usize::MAX })?;
        group_start = group_end;
    }

    Ok(BootSection {
        kind: BootSectionKind::PremiumMarketPrice,
        record_size,
        count,
        data,
    })
}

/// Strictly build a bounded market-price section with default limits.
///
/// Rows are grouped by vnum and emitted in ascending vnum order. Sums are
/// divided by each group's source-row count. Empty input produces an empty
/// optional section with the correct kind and record width.
///
/// # Errors
///
/// Returns [`MarketPriceSectionError`] for a limit, strict row, checked size,
/// fixed-width, or fallible allocation failure.
pub fn build_market_price_section(
    rows: &[MarketPriceQueryRow],
) -> Result<BootSection, MarketPriceSectionError> {
    build_market_price_section_with_limits(rows, MarketPriceSectionLimits::default())
}

/// Strictly build a market-price section with explicit limits.
///
/// # Errors
///
/// Returns [`MarketPriceSectionError`] for a source, unique-record, row,
/// count, packed-byte, record-width, or allocation failure.
pub fn build_market_price_section_with_limits(
    rows: &[MarketPriceQueryRow],
    limits: MarketPriceSectionLimits,
) -> Result<BootSection, MarketPriceSectionError> {
    build_market_price_section_with_decoder(rows, limits, decode_market_price_query_row)
}

/// Build a bounded section with the explicitly modeled legacy row decoder.
///
/// # Errors
///
/// Returns [`MarketPriceSectionError`] for a limit, checked size,
/// fixed-width, or allocation failure. The selected legacy cell conversion
/// itself has no error case.
pub fn build_market_price_section_legacy(
    rows: &[MarketPriceQueryRow],
) -> Result<BootSection, MarketPriceSectionError> {
    build_market_price_section_legacy_with_limits(rows, MarketPriceSectionLimits::default())
}

/// Build a section with the legacy row decoder and explicit limits.
///
/// # Errors
///
/// Returns [`MarketPriceSectionError`] for a source, unique-record, checked
/// size, fixed-width, or allocation failure.
pub fn build_market_price_section_legacy_with_limits(
    rows: &[MarketPriceQueryRow],
    limits: MarketPriceSectionLimits,
) -> Result<BootSection, MarketPriceSectionError> {
    build_market_price_section_with_decoder(rows, limits, decode_market_price_query_row_legacy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::{decode_market_item_price_section, BootFeatureProfile};

    fn optional_bytes(value: Option<&[u8]>) -> Option<Vec<u8>> {
        value.map(<[u8]>::to_vec)
    }

    fn strict_row(vnum: &[u8], gold: &[u8], cheque: &[u8]) -> MarketPriceQueryRow {
        MarketPriceQueryRow::new(
            optional_bytes(Some(vnum)),
            optional_bytes(Some(gold)),
            optional_bytes(Some(cheque)),
        )
    }

    fn legacy_row(
        vnum: Option<&[u8]>,
        gold: Option<&[u8]>,
        cheque: Option<&[u8]>,
    ) -> MarketPriceQueryRow {
        MarketPriceQueryRow::new(
            vnum.map(<[u8]>::to_vec),
            gold.map(<[u8]>::to_vec),
            cheque.map(<[u8]>::to_vec),
        )
    }

    #[test]
    fn query_matches_exact_source_spelling_default_and_postfix() {
        let default = MarketPriceQuery::new(&TablePostfix::default(), 1_700_000_000).unwrap();
        assert_eq!(default.table_name(), MARKET_PRICE_TABLE);
        assert_eq!(default.postfix(), &TablePostfix::default());
        assert_eq!(default.unix_seconds(), 1_700_000_000);
        assert_eq!(
            default.as_str(),
            "SELECT vnum, gold, cheque FROM private_shop_sale_history WHERE DATEDIFF(time, FROM_UNIXTIME(1700000000)) < 3"
        );
        assert_eq!(MARKET_PRICE_QUERY_BUFFER_BYTES, 512);
        assert_eq!(MARKET_PRICE_MAX_QUERY_BYTES, 511);
        assert_eq!(MARKET_PRICE_DAY_INTERVAL, 3);
        assert_eq!(MARKET_PRICE_QUERY_COLUMNS, ["vnum", "gold", "cheque"]);

        let postfix = TablePostfix::parse("_eu").unwrap();
        let suffixed = MarketPriceQuery::new(&postfix, 42).unwrap();
        assert_eq!(suffixed.table_name(), "private_shop_sale_history_eu");
        assert_eq!(
            suffixed.as_str(),
            "SELECT vnum, gold, cheque FROM private_shop_sale_history_eu WHERE DATEDIFF(time, FROM_UNIXTIME(42)) < 3"
        );
        assert_eq!(suffixed.postfix().as_str(), "_eu");

        let from_config = MarketPriceQuery::from_config(Some("_eu"), 7).unwrap();
        assert_eq!(from_config, suffixed.with_unix_seconds_for_test(7));
    }

    impl MarketPriceQuery {
        fn with_unix_seconds_for_test(&self, unix_seconds: u32) -> Self {
            Self::new(self.postfix(), unix_seconds).unwrap()
        }
    }

    #[test]
    fn longest_valid_postfix_still_fits_the_legacy_buffer() {
        let postfix_text = "a".repeat(MAX_TABLE_POSTFIX_BYTES);
        let postfix = TablePostfix::parse(&postfix_text).unwrap();
        let query = MarketPriceQuery::new(&postfix, u32::MAX).unwrap();
        assert!(query.as_str().len() <= MARKET_PRICE_MAX_QUERY_BYTES);
        assert!(query.as_str().len() < MARKET_PRICE_QUERY_BUFFER_BYTES);
        assert!(query.as_str().ends_with("FROM_UNIXTIME(4294967295)) < 3"));
    }

    #[test]
    fn invalid_configuration_and_defensive_bounds_are_typed() {
        assert!(matches!(
            MarketPriceQuery::from_config(Some("bad-name"), 1),
            Err(MarketPriceBoundaryError::Postfix(_))
        ));
        let too_long = "a".repeat(MAX_TABLE_POSTFIX_BYTES + 1);
        assert!(matches!(
            MarketPriceQuery::from_config(Some(&too_long), 1),
            Err(MarketPriceBoundaryError::Postfix(_))
        ));

        assert_eq!(
            validate_query_postfix(&too_long),
            Err(MarketPriceQueryBuildError::PostfixTooLong {
                length: MAX_TABLE_POSTFIX_BYTES + 1,
                maximum: MAX_TABLE_POSTFIX_BYTES,
            })
        );
        assert!(matches!(
            validate_query_postfix("bad-name"),
            Err(MarketPriceQueryBuildError::InvalidPostfixCharacter { .. })
        ));

        let invalid_table = "private_shop-history";
        assert!(matches!(
            validate_query_table_name(invalid_table),
            Err(MarketPriceQueryBuildError::InvalidTableIdentifier { .. })
        ));
        let long_table = "a".repeat(MAX_MARKET_PRICE_TABLE_NAME_BYTES + 1);
        assert!(matches!(
            validate_query_table_name(&long_table),
            Err(MarketPriceQueryBuildError::TableNameTooLong { .. })
        ));

        let long_query = "x".repeat(MARKET_PRICE_MAX_QUERY_BYTES + 1);
        assert_eq!(
            validate_query_statement(&long_query),
            Err(MarketPriceQueryBuildError::QueryTooLong {
                length: MARKET_PRICE_MAX_QUERY_BYTES + 1,
                maximum: MARKET_PRICE_MAX_QUERY_BYTES,
            })
        );
    }

    #[test]
    fn row_accessors_retain_raw_cells_and_nulls() {
        let cells = [
            optional_bytes(Some(b"17")),
            None,
            optional_bytes(Some(b"\xff")),
        ];
        let row = MarketPriceQueryRow::from_typed_columns(cells.clone());
        assert_eq!(row.columns(), &cells);
        assert_eq!(row.vnum(), Some(&b"17"[..]));
        assert_eq!(row.gold(), None);
        assert_eq!(row.cheque(), Some(&b"\xff"[..]));
        assert_eq!(row.cells().len(), MARKET_PRICE_QUERY_COLUMN_COUNT);
        assert_eq!(row.into_columns(), cells);
    }

    #[test]
    fn strict_decoder_accepts_exact_target_width_values() {
        let row = strict_row(b"4294967295", b"-9223372036854775808", b"0");
        assert_eq!(
            decode_market_price_query_row(&row),
            Ok(MarketItemPriceRecord {
                vnum: u32::MAX,
                gold: i64::MIN,
                cheque: 0,
            })
        );
        assert_eq!(
            decode_market_price_row(&row),
            decode_market_price_query_row(&row)
        );
    }

    #[test]
    fn strict_decoder_rejects_null_empty_whitespace_plus_prefix_nul_and_non_utf8() {
        let null =
            MarketPriceQueryRow::new(None, optional_bytes(Some(b"1")), optional_bytes(Some(b"2")));
        assert_eq!(
            decode_market_price_query_row(&null),
            Err(MarketPriceRowError::Null { column: 0 })
        );

        let empty = strict_row(b"", b"1", b"2");
        assert_eq!(
            decode_market_price_query_row(&empty),
            Err(MarketPriceRowError::Empty { column: 0 })
        );

        for (column, row) in [
            (0_usize, strict_row(b" 1", b"1", b"2")),
            (0, strict_row(b"+1", b"1", b"2")),
            (0, strict_row(b"1tail", b"1", b"2")),
            (0, strict_row(b"1\0", b"1", b"2")),
            (0, strict_row(b"\xff", b"1", b"2")),
            (1, strict_row(b"1", b" 1", b"2")),
            (1, strict_row(b"1", b"+1", b"2")),
            (1, strict_row(b"1", b"1tail", b"2")),
            (1, strict_row(b"1", b"1\0", b"2")),
            (1, strict_row(b"1", b"\xff", b"2")),
            (2, strict_row(b"1", b"1", b" 2")),
            (2, strict_row(b"1", b"1", b"+2")),
            (2, strict_row(b"1", b"1", b"2tail")),
            (2, strict_row(b"1", b"1", b"2\0")),
            (2, strict_row(b"1", b"1", b"\xff")),
        ] {
            let error = decode_market_price_query_row(&row).unwrap_err();
            match (column, error) {
                (
                    0,
                    MarketPriceRowError::NonUtf8 { column: 0, .. }
                    | MarketPriceRowError::InvalidNumber { column: 0, .. },
                )
                | (
                    1,
                    MarketPriceRowError::NonUtf8 { column: 1, .. }
                    | MarketPriceRowError::InvalidNumber { column: 1, .. },
                )
                | (
                    2,
                    MarketPriceRowError::NonUtf8 { column: 2, .. }
                    | MarketPriceRowError::InvalidNumber { column: 2, .. },
                ) => {}
                other => panic!("unexpected error for column {column}: {other:?}"),
            }
        }
    }

    #[test]
    fn strict_decoder_rejects_signed_unsigned_and_width_overflow() {
        assert!(matches!(
            decode_market_price_query_row(&strict_row(b"-1", b"1", b"1")),
            Err(MarketPriceRowError::InvalidNumber { column: 0, .. })
        ));
        assert!(matches!(
            decode_market_price_query_row(&strict_row(b"4294967296", b"1", b"1")),
            Err(MarketPriceRowError::NumberOverflow {
                column: 0,
                target: "u32",
                ..
            })
        ));
        assert!(matches!(
            decode_market_price_query_row(&strict_row(b"1", b"9223372036854775808", b"1")),
            Err(MarketPriceRowError::NumberOverflow {
                column: 1,
                target: "i64",
                ..
            })
        ));
        assert!(matches!(
            decode_market_price_query_row(&strict_row(b"1", b"1", b"4294967296")),
            Err(MarketPriceRowError::NumberOverflow {
                column: 2,
                target: "u32",
                ..
            })
        ));
    }

    #[test]
    fn legacy_decoder_models_null_empty_whitespace_sign_and_prefix() {
        assert_eq!(
            decode_market_price_query_row_legacy(&legacy_row(None, None, None)),
            Ok(MarketItemPriceRecord::default())
        );
        assert_eq!(
            decode_market_price_query_row_legacy(&legacy_row(Some(b""), Some(b""), Some(b""))),
            Ok(MarketItemPriceRecord::default())
        );

        let dword_cases: &[(&[u8], u32)] = &[
            (b" \t\n\x0b\x0c\r+17tail", 17),
            (b"-1", u32::MAX),
            (b"-2", 4_294_967_294),
            (b"-17", 4_294_967_279),
            (b"4294967295", u32::MAX),
            (b"4294967296", u32::MAX),
            (b"-4294967295", 1),
            (b"-4294967296", u32::MAX),
            (b"12\0ignored", 12),
            (b"12ignored\xff", 12),
            (b"sign", 0),
            (b"-", 0),
        ];
        for (source, expected) in dword_cases {
            let record = decode_market_price_query_row_legacy(&legacy_row(
                Some(source),
                Some(b"1"),
                Some(b"1"),
            ))
            .unwrap();
            assert_eq!(record.vnum, *expected, "source {source:?}");
        }

        let gold_cases: &[(&[u8], i64)] = &[
            (b" \t\n\x0b\x0c\r-7tail", -7),
            (b"9223372036854775807", i64::MAX),
            (b"9223372036854775808", i64::MIN),
            (b"-9223372036854775808", i64::MIN),
            (b"18446744073709551615", -1),
            (b"18446744073709551616", -1),
            (b"-18446744073709551616", -1),
            (b"9\0ignored", 9),
            (b"bad", 0),
        ];
        for (source, expected) in gold_cases {
            let record = decode_market_price_query_row_legacy(&legacy_row(
                Some(b"1"),
                Some(source),
                Some(b"1"),
            ))
            .unwrap();
            assert_eq!(record.gold, *expected, "source {source:?}");
        }

        let cheque = decode_market_price_query_row_legacy(&legacy_row(
            Some(b"1"),
            Some(b"1"),
            Some(b"4294967296ignored"),
        ))
        .unwrap();
        assert_eq!(cheque.cheque, u32::MAX);
        assert_eq!(
            decode_market_price_row_legacy(&legacy_row(None, None, None)),
            Ok(MarketItemPriceRecord::default())
        );
    }

    #[test]
    fn grouping_averages_duplicates_and_sorts_ascending() {
        let rows = vec![
            strict_row(b"20", b"-10", b"4294967295"),
            strict_row(b"3", b"5", b"1"),
            strict_row(b"20", b"7", b"2"),
            strict_row(b"3", b"-1", b"4294967295"),
        ];
        let section = build_market_price_section(&rows).unwrap();
        assert_eq!(section.kind, BootSectionKind::PremiumMarketPrice);
        assert_eq!(section.record_size, 16);
        assert_eq!(section.count, 2);
        let decoded =
            decode_market_item_price_section(&section, BootFeatureProfile::new(false, false, true))
                .unwrap();
        assert_eq!(
            decoded,
            vec![
                MarketItemPriceRecord {
                    vnum: 3,
                    gold: 2,
                    cheque: 0,
                },
                MarketItemPriceRecord {
                    vnum: 20,
                    gold: -1,
                    cheque: 0,
                },
            ]
        );
    }

    #[test]
    fn gold_and_cheque_group_sums_wrap_before_division() {
        let rows = vec![
            strict_row(b"9", b"9223372036854775807", b"4294967295"),
            strict_row(b"9", b"1", b"1"),
        ];
        let section = build_market_price_section(&rows).unwrap();
        let decoded =
            decode_market_item_price_section(&section, BootFeatureProfile::new(false, false, true))
                .unwrap();
        assert_eq!(
            decoded,
            vec![MarketItemPriceRecord {
                vnum: 9,
                gold: i64::MIN / 2,
                cheque: 0,
            }]
        );
    }

    #[test]
    fn legacy_builder_zero_fills_null_cells_and_still_groups() {
        let rows = vec![
            legacy_row(Some(b"5"), None, Some(b"8")),
            legacy_row(Some(b"5"), Some(b"invalid"), None),
        ];
        let section = build_market_price_section_legacy(&rows).unwrap();
        let decoded =
            decode_market_item_price_section(&section, BootFeatureProfile::new(false, false, true))
                .unwrap();
        assert_eq!(
            decoded,
            vec![MarketItemPriceRecord {
                vnum: 5,
                gold: 0,
                cheque: 4,
            }]
        );
    }

    #[test]
    fn source_and_unique_record_limits_fail_before_output() {
        let rows = vec![strict_row(b"1", b"1", b"1"), strict_row(b"2", b"1", b"1")];
        assert_eq!(
            build_market_price_section_with_limits(
                &rows,
                MarketPriceSectionLimits::with_limits(1, 2, MARKET_PRICE_MAX_SECTION_BYTES),
            ),
            Err(MarketPriceSectionError::TooManySourceRows {
                count: 2,
                maximum: 1,
            })
        );
        assert_eq!(
            build_market_price_section_with_limits(
                &rows,
                MarketPriceSectionLimits::with_limits(2, 1, MARKET_PRICE_MAX_SECTION_BYTES),
            ),
            Err(MarketPriceSectionError::TooManyRecords {
                count: 2,
                maximum: 1,
            })
        );
    }

    #[test]
    fn decoded_row_errors_retain_source_index() {
        let rows = vec![strict_row(b"1", b"1", b"1"), strict_row(b"bad", b"1", b"1")];
        let error = build_market_price_section(&rows).unwrap_err();
        assert!(matches!(
            error,
            MarketPriceSectionError::Row {
                index: 1,
                source: MarketPriceRowError::InvalidNumber { column: 0, .. },
            }
        ));
    }

    #[test]
    fn data_record_count_and_fixed_width_branches_are_checked() {
        let one = [strict_row(b"1", b"1", b"1")];
        assert_eq!(
            build_market_price_section_with_limits(
                &one,
                MarketPriceSectionLimits::with_limits(1, 1, MARKET_ITEM_PRICE_WIRE_SIZE - 1,),
            ),
            Err(MarketPriceSectionError::DataTooLarge {
                length: MARKET_ITEM_PRICE_WIRE_SIZE,
                maximum: MARKET_ITEM_PRICE_WIRE_SIZE - 1,
            })
        );

        let unbounded = MarketPriceSectionLimits::with_limits(usize::MAX, usize::MAX, usize::MAX);
        let unrepresentable_count = usize::from(u16::MAX).checked_add(1).unwrap();
        assert!(matches!(
            validate_section_shape(
                unrepresentable_count,
                MARKET_ITEM_PRICE_WIRE_SIZE,
                unbounded
            ),
            Err(MarketPriceSectionError::CountOverflow { .. })
        ));
        assert!(matches!(
            validate_section_shape(0, usize::MAX, unbounded),
            Err(MarketPriceSectionError::RecordSizeOverflow { .. })
        ));
        assert!(matches!(
            checked_market_price_data_length(2, usize::MAX),
            Err(MarketPriceSectionError::DataSizeOverflow { .. })
        ));
        assert!(matches!(
            validate_encoded_record(0, &[0_u8; MARKET_ITEM_PRICE_WIRE_SIZE - 1]),
            Err(MarketPriceSectionError::RecordSizeMismatch { index: 0, .. })
        ));
    }

    #[test]
    fn invalid_or_unrepresentable_group_sizes_are_explicit_errors() {
        assert_eq!(
            average_i64_wrapping_sum(1, 0),
            Err(MarketPriceSectionError::InvalidGroupSize { group_size: 0 })
        );
        if let Ok(too_large_for_u32) = usize::try_from(u64::from(u32::MAX) + 1) {
            assert_eq!(
                average_u32_wrapping_sum(1, too_large_for_u32),
                Err(MarketPriceSectionError::InvalidGroupSize {
                    group_size: too_large_for_u32,
                })
            );
        }
    }

    #[test]
    fn fallible_record_and_data_allocations_report_capacity_failures() {
        assert_eq!(
            allocate_decoded_records(usize::MAX),
            Err(MarketPriceSectionError::RecordAllocationFailed {
                requested_records: usize::MAX,
            })
        );
        assert_eq!(
            allocate_section_data(usize::MAX),
            Err(MarketPriceSectionError::DataAllocationFailed {
                requested_bytes: usize::MAX,
            })
        );
    }

    #[test]
    fn empty_source_builds_and_round_trips_an_empty_optional_section() {
        let section = build_market_price_section(&[]).unwrap();
        assert_eq!(section.kind, BootSectionKind::PremiumMarketPrice);
        assert_eq!(section.record_size, 16);
        assert_eq!(section.count, 0);
        assert!(section.data.is_empty());
        assert_eq!(MARKET_PRICE_MAX_RECORDS, 65_535);
        assert_eq!(MARKET_PRICE_MAX_SECTION_BYTES, 1_048_560);
        assert!(decode_market_item_price_section(
            &section,
            BootFeatureProfile::new(false, false, true),
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn section_round_trips_through_the_protocol_decoder() {
        let rows = vec![
            strict_row(b"12", b"-100", b"2"),
            strict_row(b"12", b"200", b"4"),
            strict_row(b"4", b"50", b"1"),
        ];
        let expected = vec![
            MarketItemPriceRecord {
                vnum: 4,
                gold: 50,
                cheque: 1,
            },
            MarketItemPriceRecord {
                vnum: 12,
                gold: 50,
                cheque: 3,
            },
        ];
        let section = build_market_price_section_with_limits(
            &rows,
            MarketPriceSectionLimits::with_limits(3, 2, MARKET_ITEM_PRICE_WIRE_SIZE * 2),
        )
        .unwrap();
        assert_eq!(section.data.len(), MARKET_ITEM_PRICE_WIRE_SIZE * 2);
        assert_eq!(
            decode_market_item_price_section(
                &section,
                BootFeatureProfile::new(false, false, true),
            )
            .unwrap(),
            expected
        );
    }
}
