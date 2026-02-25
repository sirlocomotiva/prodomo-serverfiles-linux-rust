//! Strict, SQL-free `TABLE_POSTFIX` and fixed boot-table query boundaries.
//!
//! Legacy evidence used by this module:
//!
//! * `server/server/db/Main.cpp:165-180` reads `TABLE_POSTFIX` through a
//!   257-byte buffer and passes a 256-byte destination size to `GetValue`.
//!   `server/server/db/Config.cpp:227-237` uses `strlcpy`, so the effective
//!   legacy value is at most 255 bytes plus the terminating NUL. A missing value
//!   becomes the empty string before `SetTablePostfix` is called.
//! * `server/server/db/Main.cpp:332-342` copies every non-empty value verbatim;
//!   the legacy path does not restrict identifier characters. That behavior is
//!   intentionally not reproduced here because the value is interpolated into
//!   SQL identifiers throughout the legacy server.
//! * `server/server/libsql/AsyncSQL.cpp:115-139` enables
//!   `CLIENT_MULTI_STATEMENTS`, and `DirectQuery` forwards the constructed text
//!   to `MySQL`. Rejecting SQL metacharacters here is therefore a security
//!   boundary, not cosmetic normalization.
//! * `server/server/db/ClientManagerBoot.cpp:151-159` constructs the fixed
//!   13-column `refine_proto%s` boot query. The query below keeps that exact
//!   column order, including its source spacing, while accepting only a bounded
//!   ASCII postfix.
//! * `server/server/db/ClientManagerBoot.cpp:185-202` and
//!   `server/server/common/tables.h:1209-1224` establish the `DWORD`/`int`
//!   field types and the five `(vnum, count)` material slots.
//!   `server/server/common/item_length.h:38` fixes the slot count at five.
//! * `server/server/db/ClientManagerBoot.cpp:655-674` constructs the fixed
//!   30-column `skill_proto` query into `char query[4096]`. The checked
//!   statement below keeps its exact column expressions, table interpolation,
//!   and `ORDER BY dwVnum` spelling.
//!
//!
//! [`TablePostfix`] accepts only the empty string or 1..=255 ASCII letters,
//! digits, and underscores. Values are not trimmed. This is a deliberate,
//! compatibility-restricted safety boundary around the legacy free-form
//! suffix. The current parser (`common/src/config.rs:729-746`) stores the value
//! as an unchecked `String`; validation happens here, immediately before
//! identifier use.
//!
//! [`RefineProtoQuery`] can build only the one source-fixed read query. The
//! optional row source is injected by the caller. This module does not open a
//! database connection or execute SQL. [`RefineProtoLoader::load_section`]
//! delegates fallible row decoding and bounded wire-section construction to
//! the existing pure `crate::refine` boundary. It is not a production table
//! loader and does not manage a database pool, schema, cache, or boot snapshot.

use std::error::Error;
use std::fmt;

use protocol::db_boot::BootSection;
use protocol::db_records::{RefineMaterialRecord, RefineTableRecord};

use crate::refine::build_refine_section_with_limits;
pub use crate::refine::{
    decode_refine_query_row, RefineQueryValue, RefineRowError, RefineSectionError,
    RefineSectionLimits, RefineTableQueryRow, REFINE_TABLE_QUERY_COLUMNS,
    REFINE_TABLE_QUERY_COLUMN_NAMES,
};

// The object-prototype boundary is implemented in its own SQL-free module.
// Re-export the stable names here so callers using the established postfix
// boundary do not need to know the coordinator's module split.
pub use crate::object_proto::{
    build_object_proto_section, build_object_proto_section_legacy,
    build_object_proto_section_legacy_with_limit, build_object_proto_section_legacy_with_limits,
    build_object_proto_section_with_limit, build_object_proto_section_with_limits,
    decode_object_proto_query_row, decode_object_proto_query_row_legacy,
    object_proto_query_column_name, ObjectProtoBoundaryError, ObjectProtoLoadError,
    ObjectProtoLoader, ObjectProtoLoaderError, ObjectProtoQuery, ObjectProtoQueryBuildError,
    ObjectProtoQueryColumn, ObjectProtoQueryRow, ObjectProtoQueryValue, ObjectProtoRowError,
    ObjectProtoRowSource, ObjectProtoSectionBuilder, ObjectProtoSectionError,
    ObjectProtoSectionLimits, ObjectProtoTableBoundaryError, ObjectProtoTableBuilder,
    ObjectProtoTableCell, ObjectProtoTableLoadError, ObjectProtoTableLoader, ObjectProtoTableQuery,
    ObjectProtoTableQueryBuildError, ObjectProtoTableQueryBuilderError, ObjectProtoTableQueryRow,
    ObjectProtoTableQueryRow as ObjectProtoTableRow, ObjectProtoTableRowError,
    ObjectProtoTableRowSource, ObjectProtoTableSectionBuildError, ObjectProtoTableSectionError,
    ObjectProtoTableSectionLimits, MAX_OBJECT_PROTO_TABLE_NAME_BYTES,
    MAX_OBJECT_PROTO_TABLE_QUERY_BYTES, OBJECT_PROTO_MATERIAL_MAX_NUM,
    OBJECT_PROTO_QUERY_COLUMN_COUNT, OBJECT_PROTO_SECTION_RECORD_SIZE, OBJECT_PROTO_TABLE,
    OBJECT_PROTO_TABLE_MAX_RECORDS, OBJECT_PROTO_TABLE_MAX_SECTION_BYTES,
    OBJECT_PROTO_TABLE_QUERY_COLUMNS, OBJECT_PROTO_TABLE_QUERY_COLUMN_NAMES,
    OBJECT_PROTO_TABLE_QUERY_PREFIX, OBJECT_PROTO_TABLE_QUERY_SUFFIX, OBJECT_PROTO_TABLE_WIRE_SIZE,
};

/// Maximum postfix bytes retained by the legacy `strlcpy` destination.
pub const MAX_TABLE_POSTFIX_BYTES: usize = 255;

/// Base table name used by the source-fixed boot loader.
pub const REFINE_PROTO_TABLE: &str = "refine_proto";

/// Exact fixed-column prefix used by the legacy `refine_proto` boot query.
///
/// This is a prefix, not a format string. The only appended value is the
/// identifier fragment from a validated [`TablePostfix`].
pub const REFINE_PROTO_QUERY_PREFIX: &str = "SELECT id, cost, prob, vnum0, count0, vnum1, count1, vnum2, count2,  vnum3, count3, vnum4, count4 FROM ";

/// Maximum statement bytes that fit in the legacy `char query[2048]` buffer.
///
/// The terminating NUL occupies one byte, so the statement itself must be at
/// most 2,047 bytes.
pub const MAX_REFINE_PROTO_QUERY_BYTES: usize = 2_047;

const MAX_REFINE_PROTO_TABLE_BYTES: usize = REFINE_PROTO_TABLE.len() + MAX_TABLE_POSTFIX_BYTES;

/// A validated `TABLE_POSTFIX` identifier fragment.
///
/// Values contain only ASCII alphanumeric characters and underscores. An
/// empty value is valid because it is the legacy default. The inner string is
/// private so query construction cannot bypass validation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TablePostfix {
    value: String,
}

impl TablePostfix {
    /// Validate an optional configuration value.
    ///
    /// Missing and explicitly empty values both map to the legacy default.
    /// Whitespace is not trimmed.
    ///
    /// # Errors
    ///
    /// Returns [`TablePostfixError::TooLong`] when the UTF-8 value is longer
    /// than [`MAX_TABLE_POSTFIX_BYTES`], or
    /// [`TablePostfixError::InvalidCharacter`] for every other non-ASCII
    /// alphanumeric/underscore byte.
    pub fn from_config(value: Option<&str>) -> Result<Self, TablePostfixError> {
        value.map_or_else(|| Ok(Self::default()), Self::parse)
    }

    /// Validate one `TABLE_POSTFIX` value without trimming it.
    ///
    /// # Errors
    ///
    /// Returns [`TablePostfixError::TooLong`] or
    /// [`TablePostfixError::InvalidCharacter`].
    pub fn parse(value: &str) -> Result<Self, TablePostfixError> {
        if value.len() > MAX_TABLE_POSTFIX_BYTES {
            return Err(TablePostfixError::TooLong {
                length: value.len(),
                maximum: MAX_TABLE_POSTFIX_BYTES,
            });
        }

        if let Some((index, byte)) = value
            .bytes()
            .enumerate()
            .find(|(_, byte)| !is_table_postfix_byte(*byte))
        {
            return Err(TablePostfixError::InvalidCharacter { index, byte });
        }

        Ok(Self {
            value: value.to_owned(),
        })
    }

    /// Borrow the validated identifier fragment.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Return whether the legacy default empty postfix is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }
}

impl AsRef<str> for TablePostfix {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<&str> for TablePostfix {
    type Error = TablePostfixError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<&String> for TablePostfix {
    type Error = TablePostfixError;

    fn try_from(value: &String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<String> for TablePostfix {
    type Error = TablePostfixError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

const fn is_table_postfix_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// A validation failure for `TABLE_POSTFIX`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TablePostfixError {
    /// The configuration value exceeded the legacy input bound.
    TooLong {
        /// UTF-8 byte length supplied by the caller.
        length: usize,
        /// Maximum accepted UTF-8 byte length.
        maximum: usize,
    },
    /// A byte was outside the explicit ASCII allowlist.
    InvalidCharacter {
        /// Zero-based UTF-8 byte offset.
        index: usize,
        /// Disallowed byte value.
        byte: u8,
    },
}

impl fmt::Display for TablePostfixError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { length, maximum } => write!(
                formatter,
                "TABLE_POSTFIX is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidCharacter { index, byte } => write!(
                formatter,
                "TABLE_POSTFIX byte {byte:#04x} at offset {index} is not ASCII alphanumeric or '_'"
            ),
        }
    }
}

impl Error for TablePostfixError {}

/// A defensive failure while composing the fixed `refine_proto` query.
///
/// A normally constructed [`TablePostfix`] cannot trigger these errors. The
/// check remains inside the query constructor so a future internal invariant
/// change cannot turn arbitrary bytes into a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineProtoQueryBuildError {
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
    /// The generated statement would not fit the source-fixed query buffer.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for RefineProtoQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated refine_proto table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated refine_proto identifier has byte {byte:#04x} at offset {index}"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated refine_proto query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for RefineProtoQueryBuildError {}

/// One immutable, checked `refine_proto` read statement.
///
/// The type exposes only its statement and table name. It does not provide a
/// generic SQL formatter and cannot represent a write statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefineProtoQuery {
    statement: String,
    table_name: String,
    postfix: TablePostfix,
}

impl RefineProtoQuery {
    /// Build the exact source-fixed read query from a validated postfix.
    ///
    /// The generated table identifier and complete statement are checked again
    /// before this type is returned.
    ///
    /// # Errors
    ///
    /// Returns [`RefineProtoQueryBuildError`] if a future internal invariant
    /// produces an oversized or non-allowlisted identifier or statement.
    pub fn new(postfix: &TablePostfix) -> Result<Self, RefineProtoQueryBuildError> {
        let table_name = format!("{REFINE_PROTO_TABLE}{}", postfix.as_str());
        if table_name.len() > MAX_REFINE_PROTO_TABLE_BYTES {
            return Err(RefineProtoQueryBuildError::TableNameTooLong {
                length: table_name.len(),
                maximum: MAX_REFINE_PROTO_TABLE_BYTES,
            });
        }
        if let Some((index, byte)) = table_name
            .bytes()
            .enumerate()
            .find(|(_, byte)| !is_table_postfix_byte(*byte))
        {
            return Err(RefineProtoQueryBuildError::InvalidTableIdentifier { index, byte });
        }

        let mut statement =
            String::with_capacity(REFINE_PROTO_QUERY_PREFIX.len() + table_name.len());
        statement.push_str(REFINE_PROTO_QUERY_PREFIX);
        statement.push_str(&table_name);
        if statement.len() > MAX_REFINE_PROTO_QUERY_BYTES {
            return Err(RefineProtoQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_REFINE_PROTO_QUERY_BYTES,
            });
        }

        Ok(Self {
            statement,
            table_name,
            postfix: postfix.clone(),
        })
    }

    /// Validate an optional configuration value and build the fixed query.
    ///
    /// # Errors
    ///
    /// Returns [`RefineProtoBoundaryError::Postfix`] for invalid configuration
    /// or [`RefineProtoBoundaryError::Query`] for a failed defensive query
    /// check.
    pub fn from_config(configured_postfix: Option<&str>) -> Result<Self, RefineProtoBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(RefineProtoBoundaryError::Postfix)?;
        Self::new(&postfix).map_err(RefineProtoBoundaryError::Query)
    }

    /// Borrow the exact query text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Borrow the generated `refine_proto` table identifier.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix used to build this query.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }
}

/// A failure while validating a loader's postfix or fixed query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineProtoBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The fixed query failed a defensive construction check.
    Query(RefineProtoQueryBuildError),
}

impl fmt::Display for RefineProtoBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for RefineProtoBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// A query-cell value used by the existing strict refine row decoder.
pub type RefineProtoCell = RefineQueryValue;

/// One source-shaped `refine_proto` query row.
pub type RefineProtoQueryRow = RefineTableQueryRow;

/// One decoded `TRefineTable` representation.
pub type RefineProtoRecord = RefineTableRecord;

/// One decoded `(vnum, count)` refine material.
pub type RefineProtoMaterial = RefineMaterialRecord;

/// A strict refine query-row error.
pub type RefineProtoRowError = RefineRowError;

/// Strictly decode one source-shaped `refine_proto` row.
///
/// This task-specific name exposes the existing SQL-free decoder without
/// duplicating its row shape or weakening its handling of `NULL`, source
/// errors, wrong widths, malformed numbers, and overflow.
///
/// # Errors
///
/// Returns [`RefineProtoRowError`] for any malformed source row.
pub fn decode_refine_proto_row(
    row: &RefineProtoQueryRow,
) -> Result<RefineProtoRecord, RefineProtoRowError> {
    decode_refine_query_row(row)
}

/// A caller-owned source for the checked `refine_proto` read query.
///
/// The trait is an integration seam, not a database implementation. A future
/// `SQLx` adapter may execute the checked statement, while fixtures and the
/// production not-ready path can remain entirely SQL-free.
pub trait RefineProtoRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw query rows for the checked statement.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source error
    /// must not be represented as an empty row set.
    fn query_rows(&self, query: &RefineProtoQuery)
        -> Result<Vec<RefineProtoQueryRow>, Self::Error>;
}

impl<F, E> RefineProtoRowSource for F
where
    F: Fn(&RefineProtoQuery) -> Result<Vec<RefineProtoQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(
        &self,
        query: &RefineProtoQuery,
    ) -> Result<Vec<RefineProtoQueryRow>, Self::Error> {
        self(query)
    }
}

/// A reusable, bounded boundary around the fixed refine table query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefineProtoLoader {
    query: RefineProtoQuery,
    limits: RefineSectionLimits,
}

impl RefineProtoLoader {
    /// Construct a loader from an already validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`RefineProtoQueryBuildError`] if the defensive query check
    /// fails.
    pub fn new(
        postfix: &TablePostfix,
        limits: RefineSectionLimits,
    ) -> Result<Self, RefineProtoQueryBuildError> {
        Ok(Self {
            query: RefineProtoQuery::new(postfix)?,
            limits,
        })
    }

    /// Validate raw configuration and construct a bounded loader.
    ///
    /// # Errors
    ///
    /// Returns [`RefineProtoBoundaryError`] for an invalid postfix or failed
    /// query construction check.
    pub fn from_config(
        configured_postfix: Option<&str>,
        limits: RefineSectionLimits,
    ) -> Result<Self, RefineProtoBoundaryError> {
        Ok(Self {
            query: RefineProtoQuery::from_config(configured_postfix)?,
            limits,
        })
    }

    /// Borrow the immutable checked query.
    #[must_use]
    pub const fn query(&self) -> &RefineProtoQuery {
        &self.query
    }

    /// Return the row and encoded-byte limits applied after row acquisition.
    #[must_use]
    pub const fn limits(&self) -> RefineSectionLimits {
        self.limits
    }

    /// Obtain raw rows from an injected source and strictly build a boot section.
    ///
    /// This method contains no SQL execution. The source receives only the
    /// immutable checked query, and existing `crate::refine` code checks the
    /// row/byte limits before allocation, strictly decodes every cell, and
    /// produces the typed refine boot section.
    ///
    /// # Errors
    ///
    /// Returns [`RefineProtoLoadError::Source`] without fabricating an empty
    /// table when the source fails. Returns
    /// [`RefineProtoLoadError::Rows`] for a row, count, allocation, or configured
    /// limit failure.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, RefineProtoLoadError<S::Error>>
    where
        S: RefineProtoRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(RefineProtoLoadError::Source)?;
        build_refine_section_with_limits(&rows, self.limits).map_err(RefineProtoLoadError::Rows)
    }
}

/// An error while obtaining or decoding rows through the postfix loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineProtoLoadError<E> {
    /// The caller-owned source failed.
    Source(E),
    /// Raw rows failed strict decoding or configured limits.
    Rows(RefineSectionError),
}

impl<E: fmt::Display> fmt::Display for RefineProtoLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "refine_proto row source failed: {source}"),
            Self::Rows(source) => {
                write!(formatter, "refine_proto rows could not be loaded: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for RefineProtoLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Rows(source) => Some(source),
        }
    }
}

// ---------------------------------------------------------------------------
// Source-fixed land query boundary
// ---------------------------------------------------------------------------

/// Base table name used by the source-fixed land boot loader.
pub const LAND_TABLE: &str = "land";

/// Exact fixed-column and predicate prefix used by the legacy land query.
///
/// This is a prefix, not a format string. The only appended value is the
/// identifier fragment from a validated [`TablePostfix`].
pub const LAND_TABLE_QUERY_PREFIX: &str =
    "SELECT id, map_index, x, y, width, height, guild_id, guild_level_limit, price FROM ";

/// Maximum statement bytes that fit in the legacy `char query[4096]` buffer.
///
/// The terminating NUL occupies one byte, so the statement itself must be at
/// most 4,095 bytes.
pub const MAX_LAND_TABLE_QUERY_BYTES: usize = 4_095;

const MAX_LAND_TABLE_BYTES: usize = LAND_TABLE.len() + MAX_TABLE_POSTFIX_BYTES;

/// A defensive failure while composing the fixed `land` query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandTableQueryBuildError {
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
    /// The generated statement would not fit the source-fixed query buffer.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for LandTableQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated land table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated land identifier has byte {byte:#04x} at offset {index}"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated land query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for LandTableQueryBuildError {}

/// One immutable, checked `land` read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandTableQuery {
    statement: String,
    table_name: String,
    postfix: TablePostfix,
}

impl LandTableQuery {
    /// Build the exact source-fixed read query from a validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`LandTableQueryBuildError`] if a defensive identifier or
    /// statement check fails.
    pub fn new(postfix: &TablePostfix) -> Result<Self, LandTableQueryBuildError> {
        let table_name = format!("{LAND_TABLE}{}", postfix.as_str());
        if table_name.len() > MAX_LAND_TABLE_BYTES {
            return Err(LandTableQueryBuildError::TableNameTooLong {
                length: table_name.len(),
                maximum: MAX_LAND_TABLE_BYTES,
            });
        }
        if let Some((index, byte)) = table_name
            .bytes()
            .enumerate()
            .find(|(_, byte)| !is_table_postfix_byte(*byte))
        {
            return Err(LandTableQueryBuildError::InvalidTableIdentifier { index, byte });
        }

        let statement =
            format!("{LAND_TABLE_QUERY_PREFIX}{table_name} WHERE enable='YES' ORDER BY id");
        if statement.len() > MAX_LAND_TABLE_QUERY_BYTES {
            return Err(LandTableQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_LAND_TABLE_QUERY_BYTES,
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
    /// Returns [`LandTableBoundaryError`] for an invalid postfix or failed
    /// defensive query construction check.
    pub fn from_config(configured_postfix: Option<&str>) -> Result<Self, LandTableBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(LandTableBoundaryError::Postfix)?;
        Self::new(&postfix).map_err(LandTableBoundaryError::Query)
    }

    /// Borrow the exact query text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Borrow the generated `land` table identifier.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix used to build this query.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }
}

/// A failure while validating a land loader's postfix or fixed query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandTableBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The fixed query failed a defensive construction check.
    Query(LandTableQueryBuildError),
}

impl fmt::Display for LandTableBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for LandTableBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// One query-cell value used by the land row decoder.
pub type LandTableCell = crate::land::LandQueryValue;
/// One source-shaped `land` query row.
pub type LandTableQueryRow = crate::land::LandTableQueryRow;
/// One decoded `building::TLand` representation.
pub type LandTableRecord = protocol::db_records::LandRecord;
/// A land query-row error.
pub type LandTableRowError = crate::land::LandRowError;
/// A land section-limit configuration.
pub type LandTableSectionLimits = crate::land::LandSectionLimits;
/// A land section-construction error.
pub type LandTableSectionError = crate::land::LandSectionError;

/// Strictly decode one source-shaped `land` row.
///
/// # Errors
///
/// Returns [`LandTableRowError`] for a malformed source row.
pub fn decode_land_table_row(
    row: &LandTableQueryRow,
) -> Result<LandTableRecord, LandTableRowError> {
    crate::land::decode_land_query_row(row)
}

/// Decode one source-shaped `land` row using the named legacy policy.
///
/// # Errors
///
/// Returns [`LandTableRowError`] for a wrong-width row or source-cell error.
pub fn decode_land_table_row_legacy(
    row: &LandTableQueryRow,
) -> Result<LandTableRecord, LandTableRowError> {
    crate::land::decode_land_query_row_legacy(row)
}

/// A caller-owned source for the checked `land` read query.
pub trait LandTableRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw query rows for the checked statement.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source error
    /// must not be represented as an empty row set.
    fn query_rows(&self, query: &LandTableQuery) -> Result<Vec<LandTableQueryRow>, Self::Error>;
}

impl<F, E> LandTableRowSource for F
where
    F: Fn(&LandTableQuery) -> Result<Vec<LandTableQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &LandTableQuery) -> Result<Vec<LandTableQueryRow>, Self::Error> {
        self(query)
    }
}

/// A reusable, bounded boundary around the fixed land table query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandTableLoader {
    query: LandTableQuery,
    limits: LandTableSectionLimits,
}

impl LandTableLoader {
    /// Construct a loader from an already validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`LandTableQueryBuildError`] if a defensive query check fails.
    pub fn new(
        postfix: &TablePostfix,
        limits: LandTableSectionLimits,
    ) -> Result<Self, LandTableQueryBuildError> {
        Ok(Self {
            query: LandTableQuery::new(postfix)?,
            limits,
        })
    }

    /// Validate raw configuration and construct a bounded loader.
    ///
    /// # Errors
    ///
    /// Returns [`LandTableBoundaryError`] for an invalid postfix or failed
    /// query construction check.
    pub fn from_config(
        configured_postfix: Option<&str>,
        limits: LandTableSectionLimits,
    ) -> Result<Self, LandTableBoundaryError> {
        Ok(Self {
            query: LandTableQuery::from_config(configured_postfix)?,
            limits,
        })
    }

    /// Borrow the immutable checked query.
    #[must_use]
    pub const fn query(&self) -> &LandTableQuery {
        &self.query
    }

    /// Return row and encoded-byte limits.
    #[must_use]
    pub const fn limits(&self) -> LandTableSectionLimits {
        self.limits
    }

    /// Obtain raw rows and strictly build a land boot section.
    ///
    /// # Errors
    ///
    /// Returns [`LandTableLoadError::Source`] for a source failure or
    /// [`LandTableLoadError::Rows`] for strict row/limit failures.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, LandTableLoadError<S::Error>>
    where
        S: LandTableRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(LandTableLoadError::Source)?;
        crate::land::build_land_section_with_limits(&rows, self.limits)
            .map_err(LandTableLoadError::Rows)
    }

    /// Obtain raw rows and build using the named legacy conversion policy.
    ///
    /// # Errors
    ///
    /// Returns [`LandTableLoadError::Source`] for a source failure or
    /// [`LandTableLoadError::Rows`] for source-cell/limit failures.
    pub fn load_section_legacy<S>(
        &self,
        source: &S,
    ) -> Result<BootSection, LandTableLoadError<S::Error>>
    where
        S: LandTableRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(LandTableLoadError::Source)?;
        crate::land::build_land_section_legacy_with_limits(&rows, self.limits)
            .map_err(LandTableLoadError::Rows)
    }
}

/// An error while obtaining or decoding rows through the land postfix loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandTableLoadError<E> {
    /// The caller-owned source failed.
    Source(E),
    /// Raw rows failed strict or selected legacy decoding/limits.
    Rows(LandTableSectionError),
}

impl<E: fmt::Display> fmt::Display for LandTableLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "land row source failed: {source}"),
            Self::Rows(source) => write!(formatter, "land rows could not be loaded: {source}"),
        }
    }
}

impl<E: Error + 'static> Error for LandTableLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Rows(source) => Some(source),
        }
    }
}

/// Compatibility aliases for callers that use the shorter table name.
pub type LandQuery = LandTableQuery;
/// Alias for the defensive land query-build error.
pub type LandQueryBuilderError = LandTableQueryBuildError;
/// Alias for the bounded land table loader.
pub type LandLoader = LandTableLoader;
/// Alias for a loader error with a caller-owned source error.
pub type LandLoaderError<E> = LandTableLoadError<E>;

// ---------------------------------------------------------------------------
// Source-fixed item-attribute and rare item-attribute query boundaries
// ---------------------------------------------------------------------------

/// Base table name used by the source-fixed normal item-attribute query.
pub const ITEM_ATTR_TABLE: &str = "item_attr";

/// Base table name used by the source-fixed rare item-attribute query.
pub const ITEM_ATTR_RARE_TABLE: &str = "item_attr_rare";

/// Compatibility alias for the normal item-attribute table name.
pub const ITEM_ATTR_NORMAL_TABLE: &str = ITEM_ATTR_TABLE;

/// Exact fixed-column and table prefix used by the normal item-attribute query.
///
/// This is a prefix, not a format string. The only appended value is the
/// identifier fragment from a validated [`TablePostfix`].
pub const ITEM_ATTR_TABLE_QUERY_PREFIX: &str =
    "SELECT apply, apply+0, prob, lv1, lv2, lv3, lv4, lv5, weapon, body, wrist, foots, neck, head, shield, ear, talisman, glove FROM ";

/// Exact fixed-column and table prefix used by the rare item-attribute query.
///
/// This is a prefix, not a format string. The only appended value is the
/// identifier fragment from a validated [`TablePostfix`].
pub const ITEM_ATTR_RARE_TABLE_QUERY_PREFIX: &str =
    "SELECT apply, apply+0, prob, lv1, lv2, lv3, lv4, lv5, weapon, body, wrist, foots, neck, head, shield, ear FROM ";

/// Exact ordering clause used by both source-fixed item-attribute queries.
pub const ITEM_ATTR_TABLE_QUERY_SUFFIX: &str = " ORDER BY apply";

/// Exact ordering clause used by the rare item-attribute query.
pub const ITEM_ATTR_RARE_TABLE_QUERY_SUFFIX: &str = ITEM_ATTR_TABLE_QUERY_SUFFIX;

/// Maximum statement bytes accepted by either item-attribute query.
///
/// The legacy loaders use a `char query[4096]` buffer. Its terminating NUL
/// occupies one byte, so the complete SQL statement is limited to 4,095
/// bytes.
pub const MAX_ITEM_ATTR_TABLE_QUERY_BYTES: usize = 4_095;

/// Maximum statement bytes accepted by the rare item-attribute query.
pub const MAX_ITEM_ATTR_RARE_TABLE_QUERY_BYTES: usize = MAX_ITEM_ATTR_TABLE_QUERY_BYTES;

/// Short alias for the normal item-attribute statement bound.
pub const MAX_ITEM_ATTR_QUERY_BYTES: usize = MAX_ITEM_ATTR_TABLE_QUERY_BYTES;

/// Short alias for the rare item-attribute statement bound.
pub const MAX_ITEM_RARE_QUERY_BYTES: usize = MAX_ITEM_ATTR_RARE_TABLE_QUERY_BYTES;

/// Alias using the source table's rare-table spelling.
pub const MAX_ITEM_ATTR_RARE_QUERY_BYTES: usize = MAX_ITEM_ATTR_RARE_TABLE_QUERY_BYTES;

const MAX_ITEM_ATTR_TABLE_BYTES: usize = ITEM_ATTR_TABLE.len() + MAX_TABLE_POSTFIX_BYTES;
const MAX_ITEM_ATTR_RARE_TABLE_BYTES: usize = ITEM_ATTR_RARE_TABLE.len() + MAX_TABLE_POSTFIX_BYTES;

/// Maximum generated normal table-identifier bytes.
pub const MAX_ITEM_ATTR_TABLE_NAME_BYTES: usize = MAX_ITEM_ATTR_TABLE_BYTES;

/// Maximum generated rare table-identifier bytes.
pub const MAX_ITEM_ATTR_RARE_TABLE_NAME_BYTES: usize = MAX_ITEM_ATTR_RARE_TABLE_BYTES;

pub use crate::item_attr::{
    decode_item_attr_query_row, decode_item_attr_query_row_legacy, ItemAttrQueryValue,
    ItemAttrRowError, ItemAttrSectionError, ItemAttrSectionLimits, ItemAttrTableKind,
    ItemAttrTableQueryRow, ITEM_ATTR_NORMAL_QUERY_COLUMNS, ITEM_ATTR_NORMAL_QUERY_COLUMN_NAMES,
    ITEM_ATTR_QUERY_COLUMNS, ITEM_ATTR_QUERY_SQL, ITEM_ATTR_RARE_QUERY_COLUMNS,
    ITEM_ATTR_RARE_QUERY_COLUMN_NAMES, ITEM_ATTR_RARE_QUERY_SQL, ITEM_ATTR_RARE_TABLE_COLUMNS,
    ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS, ITEM_ATTR_RARE_TABLE_QUERY_COLUMN_NAMES,
    ITEM_ATTR_TABLE_QUERY_COLUMNS, ITEM_ATTR_TABLE_QUERY_COLUMN_NAMES,
};

/// One source-shaped item-attribute query cell.
pub type ItemAttrQueryCell = ItemAttrQueryValue;

/// One source-shaped row for either item-attribute table.
///
/// Both query widths use this one row type. The selected table kind is checked
/// by the decoder and section builder.
pub type ItemAttrQueryRow = ItemAttrTableQueryRow;

/// Alias for the shared item-attribute row type.
pub type ItemAttrTableQueryRowAlias = ItemAttrTableQueryRow;

/// The rare query uses the same row type as the normal query.
pub type ItemRareQueryRow = ItemAttrQueryRow;

/// Alias for the shared rare-query row spelling.
pub type ItemRareQueryCell = ItemAttrQueryCell;

/// The shared decoded `TItemAttrTable` representation.
///
/// This is an alias, not a second record type. The rare query leaves the
/// talisman and glove set-limit values zero through the shared converter.
pub type ItemAttrRecord = protocol::db_records::ItemAttrRecord;

/// A defensive failure while composing the fixed normal item-attribute query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemAttrQueryBuildError {
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
    /// The generated statement would not fit the source-fixed query buffer.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for ItemAttrQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated item_attr table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated item_attr identifier has byte {byte:#04x} at offset {index}"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated item_attr query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for ItemAttrQueryBuildError {}

/// A defensive failure while composing the fixed rare item-attribute query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemRareQueryBuildError {
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
    /// The generated statement would not fit the source-fixed query buffer.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for ItemRareQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated item_attr_rare table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated item_attr_rare identifier has byte {byte:#04x} at offset {index}"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated item_attr_rare query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for ItemRareQueryBuildError {}

/// One immutable, checked normal `item_attr` read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemAttrQuery {
    statement: String,
    table_name: String,
    postfix: TablePostfix,
}

impl ItemAttrQuery {
    /// Build the exact source-fixed normal read query from a validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`ItemAttrQueryBuildError`] if a defensive identifier or
    /// statement-width check fails.
    pub fn new(postfix: &TablePostfix) -> Result<Self, ItemAttrQueryBuildError> {
        let table_name = format!("{ITEM_ATTR_TABLE}{}", postfix.as_str());
        if table_name.len() > MAX_ITEM_ATTR_TABLE_BYTES {
            return Err(ItemAttrQueryBuildError::TableNameTooLong {
                length: table_name.len(),
                maximum: MAX_ITEM_ATTR_TABLE_BYTES,
            });
        }
        if let Some((index, byte)) = table_name
            .bytes()
            .enumerate()
            .find(|(_, byte)| !is_table_postfix_byte(*byte))
        {
            return Err(ItemAttrQueryBuildError::InvalidTableIdentifier { index, byte });
        }

        let statement =
            format!("{ITEM_ATTR_TABLE_QUERY_PREFIX}{table_name}{ITEM_ATTR_TABLE_QUERY_SUFFIX}");
        if statement.len() > MAX_ITEM_ATTR_TABLE_QUERY_BYTES {
            return Err(ItemAttrQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_ITEM_ATTR_TABLE_QUERY_BYTES,
            });
        }

        Ok(Self {
            statement,
            table_name,
            postfix: postfix.clone(),
        })
    }

    /// Validate optional configuration and build the fixed normal query.
    ///
    /// # Errors
    ///
    /// Returns [`ItemAttrBoundaryError`] for an invalid postfix or a failed
    /// defensive query-construction check.
    pub fn from_config(configured_postfix: Option<&str>) -> Result<Self, ItemAttrBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(ItemAttrBoundaryError::Postfix)?;
        Self::new(&postfix).map_err(ItemAttrBoundaryError::Query)
    }

    /// Borrow the exact SQL statement.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Alias for [`Self::as_str`].
    #[must_use]
    pub fn statement(&self) -> &str {
        self.as_str()
    }

    /// Borrow the generated normal table identifier.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix used to build this query.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }

    /// Return the selected table kind.
    #[must_use]
    pub const fn kind(&self) -> ItemAttrTableKind {
        ItemAttrTableKind::Normal
    }
}

/// One immutable, checked rare `item_attr_rare` read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRareQuery {
    statement: String,
    table_name: String,
    postfix: TablePostfix,
}

impl ItemRareQuery {
    /// Build the exact source-fixed rare read query from a validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`ItemRareQueryBuildError`] if a defensive identifier or
    /// statement-width check fails.
    pub fn new(postfix: &TablePostfix) -> Result<Self, ItemRareQueryBuildError> {
        let table_name = format!("{ITEM_ATTR_RARE_TABLE}{}", postfix.as_str());
        if table_name.len() > MAX_ITEM_ATTR_RARE_TABLE_BYTES {
            return Err(ItemRareQueryBuildError::TableNameTooLong {
                length: table_name.len(),
                maximum: MAX_ITEM_ATTR_RARE_TABLE_BYTES,
            });
        }
        if let Some((index, byte)) = table_name
            .bytes()
            .enumerate()
            .find(|(_, byte)| !is_table_postfix_byte(*byte))
        {
            return Err(ItemRareQueryBuildError::InvalidTableIdentifier { index, byte });
        }

        let statement = format!(
            "{ITEM_ATTR_RARE_TABLE_QUERY_PREFIX}{table_name}{ITEM_ATTR_RARE_TABLE_QUERY_SUFFIX}"
        );
        if statement.len() > MAX_ITEM_ATTR_RARE_TABLE_QUERY_BYTES {
            return Err(ItemRareQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_ITEM_ATTR_RARE_TABLE_QUERY_BYTES,
            });
        }

        Ok(Self {
            statement,
            table_name,
            postfix: postfix.clone(),
        })
    }

    /// Validate optional configuration and build the fixed rare query.
    ///
    /// # Errors
    ///
    /// Returns [`ItemRareBoundaryError`] for an invalid postfix or a failed
    /// defensive query-construction check.
    pub fn from_config(configured_postfix: Option<&str>) -> Result<Self, ItemRareBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(ItemRareBoundaryError::Postfix)?;
        Self::new(&postfix).map_err(ItemRareBoundaryError::Query)
    }

    /// Borrow the exact SQL statement.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Alias for [`Self::as_str`].
    #[must_use]
    pub fn statement(&self) -> &str {
        self.as_str()
    }

    /// Borrow the generated rare table identifier.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix used to build this query.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }

    /// Return the selected table kind.
    #[must_use]
    pub const fn kind(&self) -> ItemAttrTableKind {
        ItemAttrTableKind::Rare
    }
}

/// A failure while validating a normal item-attribute loader boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemAttrBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The fixed normal query failed a defensive construction check.
    Query(ItemAttrQueryBuildError),
}

impl fmt::Display for ItemAttrBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for ItemAttrBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// A failure while validating a rare item-attribute loader boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemRareBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The fixed rare query failed a defensive construction check.
    Query(ItemRareQueryBuildError),
}

impl fmt::Display for ItemRareBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for ItemRareBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// Strictly decode one normal or rare item-attribute row.
///
/// The table kind is explicit because the two source queries have different
/// widths. The returned record type is shared.
///
/// # Errors
///
/// Returns [`ItemAttrRowError`] for a wrong-width row, source-cell error, or
/// invalid item-attribute value.
pub fn decode_item_attr_table_row(
    kind: ItemAttrTableKind,
    row: &ItemAttrQueryRow,
) -> Result<ItemAttrRecord, ItemAttrRowError> {
    crate::item_attr::decode_item_attr_table_row(kind, row)
}

/// Decode one item-attribute row using the explicitly named legacy policy.
///
/// # Errors
///
/// Returns [`ItemAttrRowError`] for a wrong-width row, source-cell error, or
/// invalid item-attribute value under the selected legacy policy.
pub fn decode_item_attr_table_row_legacy(
    kind: ItemAttrTableKind,
    row: &ItemAttrQueryRow,
) -> Result<ItemAttrRecord, ItemAttrRowError> {
    crate::item_attr::decode_item_attr_table_row_legacy(kind, row)
}

/// A caller-owned source for the checked normal item-attribute query.
pub trait ItemAttrRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw query rows for the checked statement.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source error
    /// must not be represented as an empty row set.
    fn query_rows(&self, query: &ItemAttrQuery) -> Result<Vec<ItemAttrQueryRow>, Self::Error>;
}

impl<F, E> ItemAttrRowSource for F
where
    F: Fn(&ItemAttrQuery) -> Result<Vec<ItemAttrQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &ItemAttrQuery) -> Result<Vec<ItemAttrQueryRow>, Self::Error> {
        self(query)
    }
}

/// A caller-owned source for the checked rare item-attribute query.
pub trait ItemRareRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw query rows for the checked statement.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source error
    /// must not be represented as an empty row set.
    fn query_rows(&self, query: &ItemRareQuery) -> Result<Vec<ItemRareQueryRow>, Self::Error>;
}

impl<F, E> ItemRareRowSource for F
where
    F: Fn(&ItemRareQuery) -> Result<Vec<ItemRareQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &ItemRareQuery) -> Result<Vec<ItemRareQueryRow>, Self::Error> {
        self(query)
    }
}

/// A reusable, bounded boundary around the fixed normal item-attribute query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemAttrLoader {
    query: ItemAttrQuery,
    limits: ItemAttrSectionLimits,
}

impl ItemAttrLoader {
    /// Construct a normal loader from an already validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`ItemAttrQueryBuildError`] if a defensive query check fails.
    pub fn new(
        postfix: &TablePostfix,
        limits: ItemAttrSectionLimits,
    ) -> Result<Self, ItemAttrQueryBuildError> {
        Ok(Self {
            query: ItemAttrQuery::new(postfix)?,
            limits,
        })
    }

    /// Validate raw configuration and construct a normal loader.
    ///
    /// # Errors
    ///
    /// Returns [`ItemAttrBoundaryError`] for an invalid postfix or failed
    /// query-construction check.
    pub fn from_config(
        configured_postfix: Option<&str>,
        limits: ItemAttrSectionLimits,
    ) -> Result<Self, ItemAttrBoundaryError> {
        Ok(Self {
            query: ItemAttrQuery::from_config(configured_postfix)?,
            limits,
        })
    }

    /// Borrow the immutable checked normal query.
    #[must_use]
    pub const fn query(&self) -> &ItemAttrQuery {
        &self.query
    }

    /// Return the normal row and encoded-byte limits.
    #[must_use]
    pub const fn limits(&self) -> ItemAttrSectionLimits {
        self.limits
    }

    /// Obtain raw rows and strictly build a normal item-attribute section.
    ///
    /// This method does not execute SQL. It delegates row decoding and packed
    /// section limits to the shared `crate::item_attr` boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ItemAttrLoadError::Source`] for a source failure or
    /// [`ItemAttrLoadError::Rows`] for row decoding or limit failures.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, ItemAttrLoadError<S::Error>>
    where
        S: ItemAttrRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ItemAttrLoadError::Source)?;
        crate::item_attr::build_item_attr_section_with_limits(
            ItemAttrTableKind::Normal,
            &rows,
            self.limits,
        )
        .map_err(ItemAttrLoadError::Rows)
    }

    /// Obtain raw rows and build a normal section with the legacy policy.
    ///
    /// # Errors
    ///
    /// Returns [`ItemAttrLoadError::Source`] for a source failure or
    /// [`ItemAttrLoadError::Rows`] for row decoding or limit failures.
    pub fn load_section_legacy<S>(
        &self,
        source: &S,
    ) -> Result<BootSection, ItemAttrLoadError<S::Error>>
    where
        S: ItemAttrRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ItemAttrLoadError::Source)?;
        crate::item_attr::build_item_attr_section_legacy_with_limits(
            ItemAttrTableKind::Normal,
            &rows,
            self.limits,
        )
        .map_err(ItemAttrLoadError::Rows)
    }
}

/// A reusable, bounded boundary around the fixed rare item-attribute query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRareLoader {
    query: ItemRareQuery,
    limits: ItemAttrSectionLimits,
}

impl ItemRareLoader {
    /// Construct a rare loader from an already validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`ItemRareQueryBuildError`] if a defensive query check fails.
    pub fn new(
        postfix: &TablePostfix,
        limits: ItemAttrSectionLimits,
    ) -> Result<Self, ItemRareQueryBuildError> {
        Ok(Self {
            query: ItemRareQuery::new(postfix)?,
            limits,
        })
    }

    /// Validate raw configuration and construct a rare loader.
    ///
    /// # Errors
    ///
    /// Returns [`ItemRareBoundaryError`] for an invalid postfix or failed
    /// query-construction check.
    pub fn from_config(
        configured_postfix: Option<&str>,
        limits: ItemAttrSectionLimits,
    ) -> Result<Self, ItemRareBoundaryError> {
        Ok(Self {
            query: ItemRareQuery::from_config(configured_postfix)?,
            limits,
        })
    }

    /// Borrow the immutable checked rare query.
    #[must_use]
    pub const fn query(&self) -> &ItemRareQuery {
        &self.query
    }

    /// Return the rare row and encoded-byte limits.
    #[must_use]
    pub const fn limits(&self) -> ItemAttrSectionLimits {
        self.limits
    }

    /// Obtain raw rows and strictly build a rare item-attribute section.
    ///
    /// # Errors
    ///
    /// Returns [`ItemRareLoadError::Source`] for a source failure or
    /// [`ItemRareLoadError::Rows`] for row decoding or limit failures.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, ItemRareLoadError<S::Error>>
    where
        S: ItemRareRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ItemRareLoadError::Source)?;
        crate::item_attr::build_item_attr_section_with_limits(
            ItemAttrTableKind::Rare,
            &rows,
            self.limits,
        )
        .map_err(ItemRareLoadError::Rows)
    }

    /// Obtain raw rows and build a rare section with the legacy policy.
    ///
    /// # Errors
    ///
    /// Returns [`ItemRareLoadError::Source`] for a source failure or
    /// [`ItemRareLoadError::Rows`] for row decoding or limit failures.
    pub fn load_section_legacy<S>(
        &self,
        source: &S,
    ) -> Result<BootSection, ItemRareLoadError<S::Error>>
    where
        S: ItemRareRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(ItemRareLoadError::Source)?;
        crate::item_attr::build_item_attr_section_legacy_with_limits(
            ItemAttrTableKind::Rare,
            &rows,
            self.limits,
        )
        .map_err(ItemRareLoadError::Rows)
    }
}

/// An error while obtaining or decoding rows through the normal postfix loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemAttrLoadError<E> {
    /// The caller-owned source failed.
    Source(E),
    /// Raw rows failed strict or selected legacy decoding/limits.
    Rows(ItemAttrSectionError),
}

impl<E: fmt::Display> fmt::Display for ItemAttrLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "item_attr row source failed: {source}"),
            Self::Rows(source) => write!(formatter, "item_attr rows could not be loaded: {source}"),
        }
    }
}

impl<E: Error + 'static> Error for ItemAttrLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Rows(source) => Some(source),
        }
    }
}

/// An error while obtaining or decoding rows through the rare postfix loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemRareLoadError<E> {
    /// The caller-owned source failed.
    Source(E),
    /// Raw rows failed strict or selected legacy decoding/limits.
    Rows(ItemAttrSectionError),
}

impl<E: fmt::Display> fmt::Display for ItemRareLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "item_attr_rare row source failed: {source}"),
            Self::Rows(source) => {
                write!(
                    formatter,
                    "item_attr_rare rows could not be loaded: {source}"
                )
            }
        }
    }
}

impl<E: Error + 'static> Error for ItemRareLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Rows(source) => Some(source),
        }
    }
}

/// Compatibility aliases for the source table names.
pub type ItemAttrTableQuery = ItemAttrQuery;
/// Alias for the defensive normal query-build error.
pub type ItemAttrTableQueryBuildError = ItemAttrQueryBuildError;
/// Alias for the normal boundary error.
pub type ItemAttrTableBoundaryError = ItemAttrBoundaryError;
/// Alias for the normal row source seam.
pub use ItemAttrRowSource as ItemAttrTableRowSource;
/// Alias for the normal loader.
pub type ItemAttrTableLoader = ItemAttrLoader;
/// Alias for a normal loader error with a caller-owned source error.
pub type ItemAttrTableLoadError<E> = ItemAttrLoadError<E>;
/// Alias for the short normal query-builder error spelling.
pub type ItemAttrQueryBuilderError = ItemAttrQueryBuildError;

/// Alias for the source rare table's query type.
pub type ItemRareTableQuery = ItemRareQuery;
/// Alias for the defensive rare query-build error.
pub type ItemRareTableQueryBuildError = ItemRareQueryBuildError;
/// Alias for the rare boundary error.
pub type ItemRareTableBoundaryError = ItemRareBoundaryError;
/// Alias for the rare row source seam.
pub use ItemRareRowSource as ItemRareTableRowSource;
/// Alias for the rare loader.
pub type ItemRareTableLoader = ItemRareLoader;
/// Alias for a rare loader error with a caller-owned source error.
pub type ItemRareTableLoadError<E> = ItemRareLoadError<E>;
/// Alias for the short rare query-builder error spelling.
pub type ItemRareQueryBuilderError = ItemRareQueryBuildError;

/// Alias for the shared item-attribute section limits.
pub type ItemAttrTableSectionLimits = ItemAttrSectionLimits;
/// Alias for the shared item-attribute section error.
pub type ItemAttrTableSectionError = ItemAttrSectionError;
/// Alias for the shared item-attribute row error.
pub type ItemAttrTableRowError = ItemAttrRowError;
/// Alias for the shared item-attribute table kind.
pub type ItemAttrTable = ItemAttrTableKind;

// ---------------------------------------------------------------------------
// Source-fixed skill query boundary
// ---------------------------------------------------------------------------

/// Exact fixed-column and table prefix used by the source-fixed skill query.
///
/// This is a prefix, not a format string. The only appended value is the
/// identifier fragment from a validated [`TablePostfix`].
pub const SKILL_TABLE_QUERY_PREFIX: &str = "SELECT dwVnum, szName, bType, bMaxLevel, dwSplashRange, szPointOn, szPointPoly, szSPCostPoly, szDurationPoly, szDurationSPCostPoly, szCooldownPoly, szMasterBonusPoly, setFlag+0, setAffectFlag+0, szPointOn2, szPointPoly2, szDurationPoly2, setAffectFlag2+0, szPointOn3, szPointPoly3, szDurationPoly3, szGrandMasterAddSPCostPoly, bLevelStep, bLevelLimit, prerequisiteSkillVnum, prerequisiteSkillLevel, iMaxHit, szSplashAroundDamageAdjustPoly, eSkillType+0, dwTargetRange FROM ";

/// Exact ordering clause appended after the generated `skill_proto` table name.
pub const SKILL_TABLE_QUERY_SUFFIX: &str = " ORDER BY dwVnum";

/// Maximum statement bytes accepted by the checked skill query.
///
/// The active loader uses `char query[4096]`. Its terminating NUL occupies one
/// byte, so the complete SQL statement, without that NUL, is limited to 4,095
/// bytes.
pub const MAX_SKILL_TABLE_QUERY_BYTES: usize = 4_095;

/// Maximum generated `skill_proto` table-identifier bytes.
pub const MAX_SKILL_TABLE_NAME_BYTES: usize = SKILL_TABLE.len() + MAX_TABLE_POSTFIX_BYTES;

pub use crate::skill::{
    build_skill_section, build_skill_section_legacy, build_skill_section_legacy_with_limit,
    build_skill_section_legacy_with_limits, build_skill_section_strict_with_limits,
    build_skill_section_with_limit, build_skill_section_with_limits, decode_skill_query_row,
    decode_skill_query_row_legacy, decode_skill_query_row_strict, load_skill_section,
    load_skill_section_legacy, load_skill_section_legacy_with_limit, load_skill_section_with_limit,
    skill_query_column_name, SkillLoadError, SkillQueryColumn, SkillQueryRow, SkillQueryValue,
    SkillRecord, SkillRowError, SkillRowSource, SkillSectionBuildError, SkillSectionBuilder,
    SkillSectionError, SkillSectionLimits, SkillTableCell, SkillTableQueryRow, SkillTableRecord,
    SkillTableRow, SkillTableRowSource, TSkillTable, SKILL_NAME_BYTES, SKILL_POINT_ON_BYTES,
    SKILL_POLY_EXPR_BYTES, SKILL_QUERY_COLUMN_COUNT, SKILL_SECTION_RECORD_SIZE, SKILL_TABLE,
    SKILL_TABLE_MAX_RECORDS, SKILL_TABLE_MAX_SECTION_BYTES, SKILL_TABLE_QUERY_COLUMNS,
    SKILL_TABLE_QUERY_COLUMN_NAMES, SKILL_TABLE_RECORD_WIRE_SIZE, SKILL_TABLE_WIRE_SIZE,
};

/// A defensive failure while composing the fixed `skill_proto` query.
///
/// Public [`TablePostfix`] values cannot make the table name too long. The
/// identifier and statement checks remain here so a future invariant change
/// cannot turn unchecked bytes into SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillTableQueryBuildError {
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
    /// The generated statement would not fit the legacy `char[4096]` buffer.
    QueryTooLong {
        /// Generated statement byte length, excluding its terminating NUL.
        length: usize,
        /// Maximum accepted statement byte length, excluding its NUL.
        maximum: usize,
    },
}

impl fmt::Display for SkillTableQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated skill_proto table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated skill_proto identifier has byte {byte:#04x} at offset {index}"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated skill_proto query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for SkillTableQueryBuildError {}

/// One immutable, checked `skill_proto` read statement.
///
/// The type exposes only the fixed statement, generated table name, and its
/// validated [`TablePostfix`]. It cannot represent an arbitrary SQL write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTableQuery {
    statement: String,
    table_name: String,
    postfix: TablePostfix,
}

impl SkillTableQuery {
    /// Build the exact source-fixed read query from a validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`SkillTableQueryBuildError`] if a defensive table-identifier or
    /// statement-width check fails.
    pub fn new(postfix: &TablePostfix) -> Result<Self, SkillTableQueryBuildError> {
        let table_name = format!("{}{}", SKILL_TABLE, postfix.as_str());
        if table_name.len() > MAX_SKILL_TABLE_NAME_BYTES {
            return Err(SkillTableQueryBuildError::TableNameTooLong {
                length: table_name.len(),
                maximum: MAX_SKILL_TABLE_NAME_BYTES,
            });
        }
        if let Some((index, byte)) = table_name
            .bytes()
            .enumerate()
            .find(|(_, byte)| !is_table_postfix_byte(*byte))
        {
            return Err(SkillTableQueryBuildError::InvalidTableIdentifier { index, byte });
        }

        let statement = format!("{SKILL_TABLE_QUERY_PREFIX}{table_name}{SKILL_TABLE_QUERY_SUFFIX}");
        if statement.len() > MAX_SKILL_TABLE_QUERY_BYTES {
            return Err(SkillTableQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_SKILL_TABLE_QUERY_BYTES,
            });
        }

        Ok(Self {
            statement,
            table_name,
            postfix: postfix.clone(),
        })
    }

    /// Validate optional configuration and build the fixed skill query.
    ///
    /// # Errors
    ///
    /// Returns [`SkillTableBoundaryError::Postfix`] for invalid configuration
    /// or [`SkillTableBoundaryError::Query`] for a failed defensive check.
    pub fn from_config(configured_postfix: Option<&str>) -> Result<Self, SkillTableBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(SkillTableBoundaryError::Postfix)?;
        Self::new(&postfix).map_err(SkillTableBoundaryError::Query)
    }

    /// Borrow the exact SQL statement.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Alias for [`Self::as_str`].
    #[must_use]
    pub fn statement(&self) -> &str {
        self.as_str()
    }

    /// Borrow the generated `skill_proto` table identifier.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Borrow the validated postfix used to build this query.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }
}

/// A failure while validating the skill loader's postfix or fixed query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillTableBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The fixed query failed a defensive construction check.
    Query(SkillTableQueryBuildError),
}

impl fmt::Display for SkillTableBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for SkillTableBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// A caller-owned source for the checked `skill_proto` read query.
///
/// The source-neutral [`SkillTableRowSource`] trait remains available for the
/// free [`load_skill_section`] APIs. The loader-bound seam must receive the
/// exact checked query so the configured table postfix cannot be bypassed.
pub trait SkillTableQueryRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Obtain raw rows for the checked statement.
    ///
    /// # Errors
    ///
    /// Returns the source error when rows cannot be obtained. A source error
    /// must not be represented as an empty row set.
    fn query_rows(&self, query: &SkillTableQuery) -> Result<Vec<SkillTableQueryRow>, Self::Error>;
}

impl<F, E> SkillTableQueryRowSource for F
where
    F: Fn(&SkillTableQuery) -> Result<Vec<SkillTableQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &SkillTableQuery) -> Result<Vec<SkillTableQueryRow>, Self::Error> {
        self(query)
    }
}

/// A reusable, bounded boundary around the fixed skill table query.
///
/// This type owns checked query construction only. Row acquisition remains
/// caller-injected through [`SkillTableQueryRowSource`], and strict or legacy
/// row conversion is delegated to `crate::skill`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTableLoader {
    query: SkillTableQuery,
    limits: SkillSectionLimits,
}

impl SkillTableLoader {
    /// Construct a loader from an already validated postfix.
    ///
    /// # Errors
    ///
    /// Returns [`SkillTableQueryBuildError`] if a defensive query check fails.
    pub fn new(
        postfix: &TablePostfix,
        limits: SkillSectionLimits,
    ) -> Result<Self, SkillTableQueryBuildError> {
        Ok(Self {
            query: SkillTableQuery::new(postfix)?,
            limits,
        })
    }

    /// Validate raw configuration and construct a bounded loader.
    ///
    /// # Errors
    ///
    /// Returns [`SkillTableBoundaryError`] for invalid postfix or failed query
    /// construction.
    pub fn from_config(
        configured_postfix: Option<&str>,
        limits: SkillSectionLimits,
    ) -> Result<Self, SkillTableBoundaryError> {
        Ok(Self {
            query: SkillTableQuery::from_config(configured_postfix)?,
            limits,
        })
    }

    /// Borrow the immutable checked query.
    #[must_use]
    pub const fn query(&self) -> &SkillTableQuery {
        &self.query
    }

    /// Return row and encoded-byte limits applied after row acquisition.
    #[must_use]
    pub const fn limits(&self) -> SkillSectionLimits {
        self.limits
    }

    /// Obtain rows and strictly build a skill boot section.
    ///
    /// This method does not execute SQL. It delegates the source/no-row/section
    /// checks to the existing pure `crate::skill` boundary.
    ///
    /// # Errors
    ///
    /// Returns [`SkillTableLoadError`] for source failure, an empty source
    /// result, or strict row/limit failure.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, SkillTableLoadError<S::Error>>
    where
        S: SkillTableQueryRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(SkillTableLoadError::Source)?;
        if rows.is_empty() {
            return Err(SkillTableLoadError::NoRows);
        }
        build_skill_section_with_limits(&rows, self.limits).map_err(SkillTableLoadError::Section)
    }

    /// Obtain rows and build using the explicitly named legacy row policy.
    ///
    /// # Errors
    ///
    /// Returns [`SkillTableLoadError`] for source failure, an empty source
    /// result, or legacy row/limit failure.
    pub fn load_section_legacy<S>(
        &self,
        source: &S,
    ) -> Result<BootSection, SkillTableLoadError<S::Error>>
    where
        S: SkillTableQueryRowSource,
    {
        let rows = source
            .query_rows(&self.query)
            .map_err(SkillTableLoadError::Source)?;
        if rows.is_empty() {
            return Err(SkillTableLoadError::NoRows);
        }
        build_skill_section_legacy_with_limits(&rows, self.limits)
            .map_err(SkillTableLoadError::Section)
    }
}

/// Table-oriented alias for the source-neutral skill load error.
pub type SkillTableLoadError<E> = SkillLoadError<E>;

/// Short alias for the source table's loader name.
pub type SkillProtoLoader = SkillTableLoader;
/// Short alias for the source table's loader error.
pub type SkillProtoLoadError<E> = SkillTableLoadError<E>;
/// Short alias for the query-aware source-table row source.
pub use SkillTableQueryRowSource as SkillProtoRowSource;
/// Explicit alias for the source-neutral free-function row source.
pub use SkillTableRowSource as SkillProtoSourceNeutralRowSource;
/// Compatibility alias for the source table's query type.
pub type SkillProtoQuery = SkillTableQuery;
/// Compatibility alias for the source table's query-build error.
pub type SkillProtoQueryBuildError = SkillTableQueryBuildError;
/// Compatibility alias for the source table's boundary error.
pub type SkillProtoBoundaryError = SkillTableBoundaryError;

/// Compatibility aliases using the shorter query/loader spellings.
pub type SkillQuery = SkillTableQuery;
/// Compatibility alias for the defensive query-build error.
pub type SkillQueryBuilderError = SkillTableQueryBuildError;
/// Explicit table-oriented compatibility alias for the query-build error.
pub type SkillTableQueryBuilderError = SkillTableQueryBuildError;
/// Compatibility alias for the checked skill loader.
pub type SkillLoader = SkillTableLoader;
/// Compatibility alias for a loader error with a caller-owned source error.
pub type SkillLoaderError<E> = SkillTableLoadError<E>;

#[cfg(test)]
mod skill_postfix_tests {
    use super::*;

    #[test]
    fn query_matches_the_exact_source_fixed_sql() {
        let postfix = TablePostfix::parse("_prod").unwrap();
        let query = SkillTableQuery::new(&postfix).unwrap();

        assert_eq!(query.table_name(), "skill_proto_prod");
        assert_eq!(query.postfix(), &postfix);
        assert_eq!(query.statement(), query.as_str());
        assert_eq!(
            query.as_str(),
            "SELECT dwVnum, szName, bType, bMaxLevel, dwSplashRange, szPointOn, szPointPoly, szSPCostPoly, szDurationPoly, szDurationSPCostPoly, szCooldownPoly, szMasterBonusPoly, setFlag+0, setAffectFlag+0, szPointOn2, szPointPoly2, szDurationPoly2, setAffectFlag2+0, szPointOn3, szPointPoly3, szDurationPoly3, szGrandMasterAddSPCostPoly, bLevelStep, bLevelLimit, prerequisiteSkillVnum, prerequisiteSkillLevel, iMaxHit, szSplashAroundDamageAdjustPoly, eSkillType+0, dwTargetRange FROM skill_proto_prod ORDER BY dwVnum"
        );
    }

    #[test]
    fn query_rejects_sql_metacharacters_and_non_ascii_postfix_bytes() {
        for value in [
            "prod;",
            "prod name",
            "prod-1",
            "prod/*x*/",
            "prod`",
            "prod'",
            "prod\"",
            "pröd",
        ] {
            assert!(
                matches!(
                    SkillTableQuery::from_config(Some(value)),
                    Err(SkillTableBoundaryError::Postfix(
                        TablePostfixError::InvalidCharacter { .. }
                    ))
                ),
                "unexpectedly accepted {value:?}"
            );
        }
    }

    #[test]
    fn query_enforces_postfix_table_and_statement_limits() {
        assert_eq!(MAX_SKILL_TABLE_QUERY_BYTES, 4_095);
        assert_eq!(
            MAX_SKILL_TABLE_NAME_BYTES,
            SKILL_TABLE.len() + MAX_TABLE_POSTFIX_BYTES
        );

        let maximum = "a".repeat(MAX_TABLE_POSTFIX_BYTES);
        let query = SkillTableQuery::from_config(Some(&maximum)).unwrap();
        assert!(query.as_str().len() <= MAX_SKILL_TABLE_QUERY_BYTES);
        assert!(query.as_str().as_bytes().is_ascii());
        assert_eq!(query.table_name().len(), MAX_SKILL_TABLE_NAME_BYTES);

        assert!(matches!(
            SkillTableQuery::from_config(Some(&"a".repeat(MAX_TABLE_POSTFIX_BYTES + 1))),
            Err(SkillTableBoundaryError::Postfix(
                TablePostfixError::TooLong { .. }
            ))
        ));
    }

    #[test]
    fn loader_keeps_checked_query_and_delegates_rows() {
        let loader =
            SkillTableLoader::from_config(Some("_test"), SkillSectionLimits::new(1)).unwrap();
        assert_eq!(loader.query().table_name(), "skill_proto_test");
        assert_eq!(loader.limits().max_records, 1);

        let unavailable = |_query: &SkillTableQuery| {
            Err::<Vec<SkillTableQueryRow>, &'static str>("database unavailable")
        };
        assert!(matches!(
            loader.load_section(&unavailable),
            Err(SkillTableLoadError::Source("database unavailable"))
        ));
        let empty =
            |_query: &SkillTableQuery| Ok::<Vec<SkillTableQueryRow>, &'static str>(Vec::new());
        assert!(matches!(
            loader.load_section(&empty),
            Err(SkillTableLoadError::NoRows)
        ));
    }

    #[test]
    fn loader_passes_exact_query_and_postfix_to_source() {
        let postfix = TablePostfix::parse("_capture").unwrap();
        let expected_query = SkillTableQuery::new(&postfix).unwrap();
        let loader = SkillTableLoader::new(&postfix, SkillSectionLimits::new(1)).unwrap();
        let captured = std::cell::RefCell::new(Vec::<SkillTableQuery>::new());
        let source = |query: &SkillTableQuery| {
            captured.borrow_mut().push(query.clone());
            let row = SkillTableQueryRow::from_typed_columns(
                std::iter::repeat_with(|| SkillQueryValue::text("0"))
                    .take(SKILL_TABLE_QUERY_COLUMNS),
            );
            Ok::<_, &'static str>(vec![row])
        };

        let strict = loader.load_section(&source).unwrap();
        let legacy = loader.load_section_legacy(&source).unwrap();
        assert_eq!(strict.count, 1);
        assert_eq!(legacy.count, 1);

        let captured = captured.into_inner();
        assert_eq!(captured.len(), 2);
        for query in captured {
            assert_eq!(&query, loader.query());
            assert_eq!(&query, &expected_query);
            assert_eq!(query.postfix(), &postfix);
            assert_eq!(query.table_name(), "skill_proto_capture");
            assert_eq!(query.as_str(), expected_query.as_str());
        }
    }

    #[test]
    fn compatibility_aliases_construct_the_same_checked_boundary() {
        let postfix = TablePostfix::default();
        let query: SkillQuery = SkillQuery::new(&postfix).unwrap();
        let loader: SkillLoader =
            SkillLoader::new(&postfix, SkillSectionLimits::default()).unwrap();
        assert_eq!(query.table_name(), "skill_proto");
        assert_eq!(loader.query(), &query);
    }
}

#[cfg(test)]
mod land_tests {
    use super::*;
    use protocol::db_boot::BootSectionKind;

    fn value(value: &str) -> LandTableCell {
        LandTableCell::text(value)
    }

    fn row() -> LandTableQueryRow {
        LandTableQueryRow::new([
            value("7"),
            value("2"),
            value("3"),
            value("4"),
            value("5"),
            value("6"),
            value("8"),
            value("9"),
            value("10"),
        ])
    }

    #[test]
    fn land_query_matches_source_spelling_and_order() {
        let postfix = TablePostfix::parse("_prod").unwrap();
        let query = LandTableQuery::new(&postfix).unwrap();
        assert_eq!(query.table_name(), "land_prod");
        assert_eq!(
            query.as_str(),
            "SELECT id, map_index, x, y, width, height, guild_id, guild_level_limit, price FROM land_prod WHERE enable='YES' ORDER BY id"
        );
        let default_query = LandTableQuery::new(&TablePostfix::default()).unwrap();
        assert_eq!(
            default_query.as_str(),
            "SELECT id, map_index, x, y, width, height, guild_id, guild_level_limit, price FROM land WHERE enable='YES' ORDER BY id"
        );
        assert_eq!(default_query.as_str().len(), 118);
        let maximum_postfix = "a".repeat(MAX_TABLE_POSTFIX_BYTES);
        let maximum_query =
            LandTableQuery::new(&TablePostfix::parse(&maximum_postfix).unwrap()).unwrap();
        assert_eq!(maximum_query.as_str().len(), 373);
        assert_eq!(maximum_query.table_name().len(), 259);
        assert!(maximum_query.as_str().len() <= MAX_LAND_TABLE_QUERY_BYTES);
    }

    #[test]
    fn land_query_reuses_postfix_allowlist_and_rejects_bad_input() {
        assert!(matches!(
            LandTableQuery::from_config(Some("land; DROP TABLE land")),
            Err(LandTableBoundaryError::Postfix(_))
        ));
        let loader = LandTableLoader::new(
            &TablePostfix::parse("_test").unwrap(),
            LandTableSectionLimits::default(),
        )
        .unwrap();
        assert_eq!(loader.query().table_name(), "land_test");
    }

    #[test]
    fn injected_land_source_preserves_order_and_does_not_empty_failures() {
        let loader =
            LandTableLoader::new(&TablePostfix::default(), LandTableSectionLimits::default())
                .unwrap();
        let source = |query: &LandTableQuery| {
            assert_eq!(
                query.as_str(),
                LandTableQuery::new(&TablePostfix::default())
                    .unwrap()
                    .as_str()
            );
            Ok::<_, &'static str>(vec![row(), row()])
        };
        let section = loader.load_section(&source).unwrap();
        assert_eq!(section.kind, BootSectionKind::Land);
        assert_eq!(section.count, 2);
        let failed = |_query: &LandTableQuery| -> Result<Vec<LandTableQueryRow>, &'static str> {
            Err("database unavailable")
        };
        assert!(matches!(
            loader.load_section(&failed),
            Err(LandTableLoadError::Source("database unavailable"))
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::{decode_refine_table_section, BootSectionKind};

    fn value(text: impl Into<String>) -> RefineProtoCell {
        RefineProtoCell::text(text)
    }

    fn valid_row() -> RefineProtoQueryRow {
        RefineProtoQueryRow::new([
            value("17"),
            value("-7"),
            value("91"),
            value("11"),
            value("2"),
            value("0"),
            value("99"),
            value("33"),
            value("4"),
            value("0"),
            value("0"),
            value("55"),
            value("6"),
        ])
    }

    #[test]
    fn missing_and_empty_postfix_use_the_legacy_default() {
        let missing = TablePostfix::from_config(None).unwrap();
        let empty = TablePostfix::parse("").unwrap();

        assert!(missing.is_empty());
        assert_eq!(missing, empty);
        assert_eq!(missing.as_str(), "");
        assert_eq!(
            RefineProtoQuery::new(&missing).unwrap().table_name(),
            "refine_proto"
        );
    }

    #[test]
    fn postfix_allowlist_accepts_only_bounded_ascii_identifier_bytes() {
        for value in ["_prod", "_test", "prod7", "7", "A_b_09"] {
            assert_eq!(TablePostfix::parse(value).unwrap().as_str(), value);
        }

        let maximum = "a".repeat(MAX_TABLE_POSTFIX_BYTES);
        let parsed_maximum = TablePostfix::parse(&maximum).unwrap();
        assert_eq!(parsed_maximum.as_str(), maximum);
        let maximum_query = RefineProtoQuery::new(&parsed_maximum).unwrap();
        assert!(maximum_query.as_str().len() <= MAX_REFINE_PROTO_QUERY_BYTES);
        assert!(maximum_query.table_name().ends_with(&maximum));
        assert_eq!(
            TablePostfix::parse(&"a".repeat(MAX_TABLE_POSTFIX_BYTES + 1)),
            Err(TablePostfixError::TooLong {
                length: MAX_TABLE_POSTFIX_BYTES + 1,
                maximum: MAX_TABLE_POSTFIX_BYTES,
            })
        );
    }

    #[test]
    fn postfix_rejects_sql_metacharacters_whitespace_and_non_ascii() {
        for value in [
            "prod-1",
            "prod.name",
            "prod name",
            " prod",
            "prod ",
            "prod; DROP TABLE player",
            "prod/*comment*/",
            "prod`",
            "prod'",
            "prod\"",
            "pröd",
            "prod\n",
            "prod\0suffix",
        ] {
            assert!(
                matches!(
                    TablePostfix::parse(value),
                    Err(TablePostfixError::InvalidCharacter { .. })
                ),
                "unexpectedly accepted {value:?}"
            );
        }
    }

    #[test]
    fn refine_query_matches_the_source_fixed_column_order_and_spacing() {
        let postfix = TablePostfix::parse("_prod").unwrap();
        let query = RefineProtoQuery::new(&postfix).unwrap();

        assert_eq!(query.table_name(), "refine_proto_prod");
        assert_eq!(query.postfix(), &postfix);
        assert_eq!(
            query.as_str(),
            "SELECT id, cost, prob, vnum0, count0, vnum1, count1, vnum2, count2,  vnum3, count3, vnum4, count4 FROM refine_proto_prod"
        );
        assert!(query.as_str().len() <= MAX_REFINE_PROTO_QUERY_BYTES);
    }

    #[test]
    fn query_constructor_revalidates_its_table_identifier() {
        // Child tests can exercise the private invariant deliberately. Public
        // callers cannot create this value without `TablePostfix::parse`.
        let invalid = TablePostfix {
            value: ";DROP".to_owned(),
        };
        assert_eq!(
            RefineProtoQuery::new(&invalid),
            Err(RefineProtoQueryBuildError::InvalidTableIdentifier {
                index: REFINE_PROTO_TABLE.len(),
                byte: b';',
            })
        );
    }

    #[test]
    fn valid_refine_row_decodes_to_the_source_representation() {
        let record = decode_refine_proto_row(&valid_row()).unwrap();

        assert_eq!(record.id, 17);
        assert_eq!(record.cost, -7);
        assert_eq!(record.prob, 91);
        assert_eq!(record.material_count, 3);
        assert_eq!(
            record.materials[0],
            RefineProtoMaterial { vnum: 11, count: 2 }
        );
        assert_eq!(
            record.materials[1],
            RefineProtoMaterial { vnum: 0, count: 99 }
        );
        assert_eq!(
            record.materials[4],
            RefineProtoMaterial { vnum: 55, count: 6 }
        );
    }

    #[test]
    fn row_decoder_keeps_null_width_and_numeric_failures_distinct() {
        let mut null_row = valid_row().into_columns();
        null_row[4] = RefineProtoCell::null();
        assert!(matches!(
            decode_refine_proto_row(&RefineProtoQueryRow::new(null_row)),
            Err(RefineProtoRowError::Null { column: 4 })
        ));

        let short = RefineProtoQueryRow::new(vec![value("1"); REFINE_TABLE_QUERY_COLUMNS - 1]);
        assert!(matches!(
            decode_refine_proto_row(&short),
            Err(RefineProtoRowError::ColumnCount {
                expected: 13,
                actual: 12,
            })
        ));

        let mut overflow = valid_row().into_columns();
        overflow[0] = value("4294967296");
        assert!(matches!(
            decode_refine_proto_row(&RefineProtoQueryRow::new(overflow)),
            Err(RefineProtoRowError::NumberOverflow {
                column: 0,
                target: "u32",
                ..
            })
        ));
    }

    #[test]
    fn injected_source_receives_only_the_checked_query_and_rows_load() {
        let postfix = TablePostfix::parse("_test").unwrap();
        let loader = RefineProtoLoader::new(&postfix, RefineSectionLimits::default()).unwrap();
        let source = |query: &RefineProtoQuery| {
            assert_eq!(query.table_name(), "refine_proto_test");
            Ok::<_, &'static str>(vec![valid_row()])
        };

        let section = loader.load_section(&source).unwrap();
        assert_eq!(section.kind, BootSectionKind::Refine);
        assert_eq!(section.count, 1);
        let decoded = decode_refine_table_section(&section).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].id, 17);
    }

    #[test]
    fn source_failure_is_not_an_empty_table_and_row_cap_is_enforced() {
        let postfix = TablePostfix::default();
        let unavailable =
            |_query: &RefineProtoQuery| -> Result<Vec<RefineProtoQueryRow>, &'static str> {
                Err("database unavailable")
            };
        let loader = RefineProtoLoader::new(&postfix, RefineSectionLimits::default()).unwrap();
        assert_eq!(
            loader.load_section(&unavailable),
            Err(RefineProtoLoadError::Source("database unavailable"))
        );

        let too_many =
            |_query: &RefineProtoQuery| Ok::<_, &'static str>(vec![valid_row(), valid_row()]);
        let bounded = RefineProtoLoader::new(&postfix, RefineSectionLimits::new(1)).unwrap();
        assert!(matches!(
            bounded.load_section(&too_many),
            Err(RefineProtoLoadError::Rows(
                RefineSectionError::TooManyRecords {
                    count: 2,
                    maximum: 1
                }
            ))
        ));
    }
}

#[cfg(test)]
mod item_attr_tests {
    use super::*;
    use protocol::db_boot::BootSectionKind;

    fn normal_row() -> ItemAttrQueryRow {
        ItemAttrTableQueryRow::from_normal_typed_columns([
            ItemAttrQueryValue::bytes(b"ATTR"),
            ItemAttrQueryValue::bytes(b"7"),
            ItemAttrQueryValue::bytes(b"91"),
            ItemAttrQueryValue::bytes(b"1"),
            ItemAttrQueryValue::bytes(b"-2"),
            ItemAttrQueryValue::bytes(b"3"),
            ItemAttrQueryValue::bytes(b"-4"),
            ItemAttrQueryValue::bytes(b"5"),
            ItemAttrQueryValue::bytes(b"6"),
            ItemAttrQueryValue::bytes(b"7"),
            ItemAttrQueryValue::bytes(b"8"),
            ItemAttrQueryValue::bytes(b"9"),
            ItemAttrQueryValue::bytes(b"10"),
            ItemAttrQueryValue::bytes(b"11"),
            ItemAttrQueryValue::bytes(b"12"),
            ItemAttrQueryValue::bytes(b"13"),
            ItemAttrQueryValue::bytes(b"14"),
            ItemAttrQueryValue::bytes(b"15"),
        ])
    }

    fn rare_row() -> ItemRareQueryRow {
        ItemAttrTableQueryRow::from_rare_typed_columns([
            ItemAttrQueryValue::bytes(b"RARE"),
            ItemAttrQueryValue::bytes(b"8"),
            ItemAttrQueryValue::bytes(b"92"),
            ItemAttrQueryValue::bytes(b"6"),
            ItemAttrQueryValue::bytes(b"5"),
            ItemAttrQueryValue::bytes(b"4"),
            ItemAttrQueryValue::bytes(b"3"),
            ItemAttrQueryValue::bytes(b"2"),
            ItemAttrQueryValue::bytes(b"1"),
            ItemAttrQueryValue::bytes(b"2"),
            ItemAttrQueryValue::bytes(b"3"),
            ItemAttrQueryValue::bytes(b"4"),
            ItemAttrQueryValue::bytes(b"5"),
            ItemAttrQueryValue::bytes(b"6"),
            ItemAttrQueryValue::bytes(b"7"),
            ItemAttrQueryValue::bytes(b"8"),
        ])
    }

    #[test]
    fn normal_and_rare_queries_match_source_spelling() {
        let empty = TablePostfix::default();
        assert_eq!(
            ItemAttrQuery::new(&empty).unwrap().as_str(),
            "SELECT apply, apply+0, prob, lv1, lv2, lv3, lv4, lv5, weapon, body, wrist, foots, neck, head, shield, ear, talisman, glove FROM item_attr ORDER BY apply"
        );
        assert_eq!(
            ItemRareQuery::new(&empty).unwrap().as_str(),
            "SELECT apply, apply+0, prob, lv1, lv2, lv3, lv4, lv5, weapon, body, wrist, foots, neck, head, shield, ear FROM item_attr_rare ORDER BY apply"
        );

        let postfix = TablePostfix::parse("_prod").unwrap();
        assert!(ItemAttrQuery::new(&postfix)
            .unwrap()
            .as_str()
            .contains("FROM item_attr_prod ORDER BY apply"));
        assert!(ItemRareQuery::new(&postfix)
            .unwrap()
            .as_str()
            .contains("FROM item_attr_rare_prod ORDER BY apply"));
    }

    #[test]
    fn query_lengths_and_column_counts_remain_bounded() {
        let postfix = TablePostfix::parse(&"a".repeat(MAX_TABLE_POSTFIX_BYTES)).unwrap();
        let normal = ItemAttrQuery::new(&postfix).unwrap();
        let rare = ItemRareQuery::new(&postfix).unwrap();
        assert!(normal.as_str().len() <= MAX_ITEM_ATTR_TABLE_QUERY_BYTES);
        assert!(rare.as_str().len() <= MAX_ITEM_ATTR_RARE_TABLE_QUERY_BYTES);
        assert_eq!(normal.kind().column_count(), ITEM_ATTR_TABLE_QUERY_COLUMNS);
        assert_eq!(
            rare.kind().column_count(),
            ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS
        );
        assert_eq!(ITEM_ATTR_TABLE_QUERY_COLUMNS, 18);
        assert_eq!(ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS, 16);
    }

    #[test]
    fn invalid_postfix_is_rejected_before_query_construction() {
        assert!(matches!(
            ItemAttrQuery::from_config(Some("bad;")),
            Err(ItemAttrBoundaryError::Postfix(
                TablePostfixError::InvalidCharacter {
                    index: 3,
                    byte: b';'
                }
            ))
        ));
        assert!(matches!(
            ItemRareQuery::from_config(Some("bad name")),
            Err(ItemRareBoundaryError::Postfix(
                TablePostfixError::InvalidCharacter {
                    index: 3,
                    byte: b' '
                }
            ))
        ));
        assert!(matches!(
            ItemAttrQuery::from_config(Some(&"x".repeat(MAX_TABLE_POSTFIX_BYTES + 1))),
            Err(ItemAttrBoundaryError::Postfix(
                TablePostfixError::TooLong { .. }
            ))
        ));
    }

    #[test]
    fn one_shared_row_type_keeps_normal_and_rare_widths_distinct() {
        let normal_record =
            decode_item_attr_table_row(ItemAttrTableKind::Normal, &normal_row()).unwrap();
        let rare_record = decode_item_attr_table_row(ItemAttrTableKind::Rare, &rare_row()).unwrap();
        assert_eq!(normal_record.max_level_by_set.len(), 10);
        assert_eq!(rare_record.max_level_by_set[8..], [0, 0]);
        assert!(matches!(
            decode_item_attr_table_row(ItemAttrTableKind::Normal, &rare_row()),
            Err(ItemAttrRowError::ColumnCount {
                expected: ITEM_ATTR_TABLE_QUERY_COLUMNS,
                actual: ITEM_ATTR_RARE_TABLE_QUERY_COLUMNS
            })
        ));
    }

    #[test]
    fn loaders_delegate_rows_and_keep_query_and_limits_public() {
        let postfix = TablePostfix::parse("_test").unwrap();
        let limits = ItemAttrSectionLimits::new(1);
        let normal_loader = ItemAttrLoader::new(&postfix, limits).unwrap();
        let rare_loader = ItemRareLoader::new(&postfix, limits).unwrap();
        assert!(normal_loader
            .query()
            .as_str()
            .ends_with("item_attr_test ORDER BY apply"));
        assert!(rare_loader
            .query()
            .as_str()
            .ends_with("item_attr_rare_test ORDER BY apply"));
        assert_eq!(normal_loader.limits().max_records, 1);
        assert_eq!(rare_loader.limits().max_records, 1);

        let normal_source = |query: &ItemAttrQuery| {
            assert_eq!(query.table_name(), "item_attr_test");
            Ok::<_, &'static str>(vec![normal_row()])
        };
        let normal_section = normal_loader.load_section(&normal_source).unwrap();
        assert_eq!(normal_section.kind, BootSectionKind::ItemAttr);
        assert_eq!(normal_section.count, 1);

        let rare_source = |query: &ItemRareQuery| {
            assert_eq!(query.table_name(), "item_attr_rare_test");
            Ok::<_, &'static str>(vec![rare_row()])
        };
        let rare_section = rare_loader.load_section_legacy(&rare_source).unwrap();
        assert_eq!(rare_section.kind, BootSectionKind::ItemRare);
        assert_eq!(rare_section.count, 1);
    }
}
