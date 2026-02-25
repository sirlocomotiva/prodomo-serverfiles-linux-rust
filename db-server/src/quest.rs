//! Source-verified, SQL-free quest-load query and row boundary.
//!
//! Legacy evidence used by this module:
//!
//! * `server/server/db/ClientManagerPlayer.cpp:323-355` uses the cache-hit
//!   query `SELECT dwPID,szName,szState,lValue FROM quest%s WHERE dwPID=%d AND
//!   lValue<>0` in a 1,024-byte buffer.
//! * `server/server/db/ClientManagerPlayer.cpp:394-400,415,525-531` uses the
//!   cache-miss query `SELECT dwPID, szName, szState, lValue FROM quest%s
//!   WHERE dwPID=%d` in a `QUERY_MAX_LEN + QUERY_MAX_LEN` buffer.
//! * `server/server/db/ClientManagerPlayer.cpp:1015-1051` emits
//!   `HEADER_DG_QUEST_LOAD`, the request handle, a four-byte row count, and
//!   zero or more 106-byte `TQuestTable` records. A valid zero-row result still
//!   emits a count-zero payload; a null SQL result is rejected by the composite
//!   callback before this response path (`ClientManagerPlayer.cpp:826-835`).
//!
//! The query accepts only the existing allowlisted [`TablePostfix`]. The pure
//! row boundary keeps SQL `NULL` distinct and copies raw name/state bytes using
//! the legacy C-string bounds. Numeric cells are decoded strictly as a safety
//! policy: the legacy `str_to_number` ignores malformed values and the schema
//! is not verified here, so this module reports NULL/range failures instead of
//! manufacturing zero values. It does not execute SQL, open a connection,
//! choose a cache variant, mutate player state, or implement the composite
//! player-load callback.

use std::error::Error;
use std::fmt;

use protocol::db_records::{
    encode_quest_load_with_limit, DbRecordError, QuestRecord, HEADER_DG_QUEST_LOAD,
    LEGACY_QUEST_NAME_BYTES, LEGACY_QUEST_STATE_BYTES, MAX_QUEST_RECORDS,
};
use protocol::db_wire::DbFrame;

use crate::postfix::{TablePostfix, TablePostfixError, MAX_TABLE_POSTFIX_BYTES};

/// Base table name used by the source quest queries.
pub const QUEST_TABLE: &str = "quest";

/// Number of columns promised by either source query.
pub const QUEST_QUERY_COLUMNS: usize = 4;

/// Maximum statement bytes for the cache-hit 1,024-byte source buffer.
///
/// The terminating NUL occupies one byte.
pub const MAX_QUEST_CACHE_QUERY_BYTES: usize = 1_023;

/// Maximum statement bytes for the cache-miss `QUERY_MAX_LEN * 2` buffer.
///
/// `QUERY_MAX_LEN` is 8,192 in `server/server/common/length.h`; the
/// terminating NUL occupies one byte.
pub const MAX_QUEST_LOAD_QUERY_BYTES: usize = 16_383;

const MAX_QUEST_TABLE_BYTES: usize = QUEST_TABLE.len() + MAX_TABLE_POSTFIX_BYTES;
const CACHE_QUERY_PREFIX: &str = "SELECT dwPID,szName,szState,lValue FROM ";
const LOAD_QUERY_PREFIX: &str = "SELECT dwPID, szName, szState, lValue FROM ";

/// Selects one of the two source quest query variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestQueryKind {
    /// The cache-hit query with the source `AND lValue<>0` predicate.
    CacheHit,
    /// The cache-miss query without the non-zero predicate.
    Load,
}

impl QuestQueryKind {
    /// Return the source-specific statement bound.
    #[must_use]
    pub const fn maximum_query_bytes(self) -> usize {
        match self {
            Self::CacheHit => MAX_QUEST_CACHE_QUERY_BYTES,
            Self::Load => MAX_QUEST_LOAD_QUERY_BYTES,
        }
    }
}

/// A defensive failure while constructing a checked quest query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestQueryBuildError {
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
    /// The generated statement exceeded the selected source buffer.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for QuestQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated quest table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated quest identifier has byte {byte:#04x} at offset {index}"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated quest query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for QuestQueryBuildError {}

/// An immutable, checked quest-load statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestQuery {
    statement: String,
    table_name: String,
    player_id: u32,
    postfix: TablePostfix,
    kind: QuestQueryKind,
}

impl QuestQuery {
    /// Build one source variant for a caller-resolved player ID.
    ///
    /// The player ID is numeric and cannot introduce SQL syntax. The table
    /// suffix is validated before interpolation. The cache-hit and cache-miss
    /// variants retain their distinct source spacing and buffer bounds.
    ///
    /// # Errors
    ///
    /// Returns [`QuestQueryBuildError`] if a defensive identifier or statement
    /// width check fails.
    pub fn new(
        postfix: &TablePostfix,
        player_id: u32,
        kind: QuestQueryKind,
    ) -> Result<Self, QuestQueryBuildError> {
        let table_name = format!("{QUEST_TABLE}{}", postfix.as_str());
        if table_name.len() > MAX_QUEST_TABLE_BYTES {
            return Err(QuestQueryBuildError::TableNameTooLong {
                length: table_name.len(),
                maximum: MAX_QUEST_TABLE_BYTES,
            });
        }
        if let Some((index, byte)) = table_name
            .bytes()
            .enumerate()
            .find(|(_, byte)| !byte.is_ascii_alphanumeric() && *byte != b'_')
        {
            return Err(QuestQueryBuildError::InvalidTableIdentifier { index, byte });
        }

        let statement = match kind {
            QuestQueryKind::CacheHit => {
                format!("{CACHE_QUERY_PREFIX}{table_name} WHERE dwPID={player_id} AND lValue<>0")
            }
            QuestQueryKind::Load => {
                format!("{LOAD_QUERY_PREFIX}{table_name} WHERE dwPID={player_id}")
            }
        };
        let maximum = kind.maximum_query_bytes();
        if statement.len() > maximum {
            return Err(QuestQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum,
            });
        }

        Ok(Self {
            statement,
            table_name,
            player_id,
            postfix: postfix.clone(),
            kind,
        })
    }

    /// Validate an optional configured postfix and build a query.
    ///
    /// # Errors
    ///
    /// Returns [`TablePostfixError`] for an invalid configured postfix or
    /// [`QuestQueryBuildError`] for a defensive construction failure.
    pub fn from_config(
        configured_postfix: Option<&str>,
        player_id: u32,
        kind: QuestQueryKind,
    ) -> Result<Self, QuestQueryBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(QuestQueryBoundaryError::Postfix)?;
        Self::new(&postfix, player_id, kind).map_err(QuestQueryBoundaryError::Query)
    }

    /// Borrow the exact statement text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.statement
    }

    /// Borrow the generated table identifier.
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Return the numeric player ID used by the caller.
    #[must_use]
    pub const fn player_id(&self) -> u32 {
        self.player_id
    }

    /// Return the selected source query variant.
    #[must_use]
    pub const fn kind(&self) -> QuestQueryKind {
        self.kind
    }

    /// Borrow the validated table postfix.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }
}

/// A failure while validating a postfix or quest query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestQueryBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The fixed query failed a defensive construction check.
    Query(QuestQueryBuildError),
}

impl fmt::Display for QuestQueryBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for QuestQueryBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// One source-shaped quest row. `None` preserves SQL `NULL` until decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestQueryRow {
    /// `dwPID`, represented as a signed SQL integer for range checking.
    pub pid: Option<i64>,
    /// Raw `szName` bytes, before bounded C-string copying.
    pub name: Option<Vec<u8>>,
    /// Raw `szState` bytes, before bounded C-string copying.
    pub state: Option<Vec<u8>>,
    /// `lValue`, represented as a signed SQL integer for range checking.
    pub value: Option<i64>,
}

impl QuestQueryRow {
    /// Construct a source-shaped row without normalizing its cells.
    #[must_use]
    pub fn new(
        pid: Option<i64>,
        name: Option<Vec<u8>>,
        state: Option<Vec<u8>>,
        value: Option<i64>,
    ) -> Self {
        Self {
            pid,
            name,
            state,
            value,
        }
    }
}

/// A failure while decoding one quest row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestRowError {
    /// The source row did not have four cells.
    ColumnCount {
        /// Required source column count.
        expected: usize,
        /// Actual source cell count.
        actual: usize,
    },
    /// A required source cell was SQL `NULL`.
    NullColumn {
        /// Zero-based source column index.
        column: usize,
    },
    /// `dwPID` did not fit the unsigned 32-bit C++ field.
    PidOutOfRange {
        /// Decoded signed SQL value.
        value: i64,
    },
    /// `lValue` did not fit the signed x86 `long` field.
    ValueOutOfRange {
        /// Decoded signed SQL value.
        value: i64,
    },
    /// The output row vector could not reserve its bounded capacity.
    AllocationFailed {
        /// Requested row count.
        requested: usize,
    },
    /// The source contained more rows than the configured bound.
    TooManyRows {
        /// Source row count.
        count: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// The configured bound exceeds the protocol's fixed count limit.
    LimitTooLarge {
        /// Requested bound.
        requested: usize,
        /// Maximum supported bound.
        maximum: usize,
    },
}

impl fmt::Display for QuestRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnCount { expected, actual } => {
                write!(
                    formatter,
                    "quest row has {actual} columns; expected {expected}"
                )
            }
            Self::NullColumn { column } => write!(formatter, "quest row column {column} is NULL"),
            Self::PidOutOfRange { value } => {
                write!(formatter, "quest PID {value} does not fit u32")
            }
            Self::ValueOutOfRange { value } => {
                write!(formatter, "quest value {value} does not fit i32")
            }
            Self::AllocationFailed { requested } => {
                write!(formatter, "quest row allocation of {requested} failed")
            }
            Self::TooManyRows { count, maximum } => {
                write!(
                    formatter,
                    "quest source has {count} rows; maximum is {maximum}"
                )
            }
            Self::LimitTooLarge { requested, maximum } => write!(
                formatter,
                "quest row limit {requested} exceeds protocol maximum {maximum}"
            ),
        }
    }
}

impl Error for QuestRowError {}

/// Decode one raw source row into the fixed x86 quest record.
///
/// Name and state bytes are copied through the first NUL, truncated to the
/// legacy field capacity, and followed by a zero-filled tail. This preserves
/// raw non-UTF-8 bytes and the source's `strlcpy` bounds without treating a
/// long value as an error. Numeric NULL/range failures are intentional safety
/// divergences from the legacy parser's ignored `str_to_number` result.
///
/// # Errors
///
/// Returns [`QuestRowError`] when a required cell is `NULL` or a numeric cell
/// does not fit its fixed C++ field.
pub fn decode_quest_row(row: &QuestQueryRow) -> Result<QuestRecord, QuestRowError> {
    let pid = row
        .pid
        .ok_or(QuestRowError::NullColumn { column: 0 })
        .and_then(|value| {
            u32::try_from(value).map_err(|_| QuestRowError::PidOutOfRange { value })
        })?;
    let name = row
        .name
        .as_deref()
        .ok_or(QuestRowError::NullColumn { column: 1 })?;
    let state = row
        .state
        .as_deref()
        .ok_or(QuestRowError::NullColumn { column: 2 })?;
    let value = row
        .value
        .ok_or(QuestRowError::NullColumn { column: 3 })
        .and_then(|value| {
            i32::try_from(value).map_err(|_| QuestRowError::ValueOutOfRange { value })
        })?;

    let mut record = QuestRecord {
        pid,
        name: [0; LEGACY_QUEST_NAME_BYTES],
        state: [0; LEGACY_QUEST_STATE_BYTES],
        value,
    };
    copy_c_string(name, &mut record.name);
    copy_c_string(state, &mut record.state);
    Ok(record)
}

/// Decode and bound a complete source row sequence.
///
/// # Errors
///
/// Returns [`QuestRowError`] when the source exceeds `maximum`, allocation
/// fails, or any row violates the strict cell policy.
pub fn decode_quest_rows(
    rows: &[QuestQueryRow],
    maximum: usize,
) -> Result<Vec<QuestRecord>, QuestRowError> {
    if maximum > MAX_QUEST_RECORDS {
        return Err(QuestRowError::LimitTooLarge {
            requested: maximum,
            maximum: MAX_QUEST_RECORDS,
        });
    }
    if rows.len() > maximum {
        return Err(QuestRowError::TooManyRows {
            count: rows.len(),
            maximum,
        });
    }
    let mut records = Vec::new();
    records
        .try_reserve_exact(rows.len())
        .map_err(|_| QuestRowError::AllocationFailed {
            requested: rows.len(),
        })?;
    for row in rows {
        records.push(decode_quest_row(row)?);
    }
    Ok(records)
}

/// A caller-owned source of raw quest rows in database order.
pub trait QuestRowSource {
    /// Source-specific query or extraction error.
    type Error: fmt::Display;

    /// Execute the checked query and return source-shaped rows.
    ///
    /// # Errors
    ///
    /// Returns the source error rather than an empty row list.
    fn query_rows(&self, query: &QuestQuery) -> Result<Vec<QuestQueryRow>, Self::Error>;
}

impl<F, E> QuestRowSource for F
where
    F: Fn(&QuestQuery) -> Result<Vec<QuestQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &QuestQuery) -> Result<Vec<QuestQueryRow>, Self::Error> {
        self(query)
    }
}

/// Bounds applied before quest rows are decoded and encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestLoadLimits {
    /// Maximum number of source rows and output records.
    pub max_rows: usize,
}

impl QuestLoadLimits {
    /// Construct a row bound explicitly.
    #[must_use]
    pub const fn new(max_rows: usize) -> Self {
        Self { max_rows }
    }
}

impl Default for QuestLoadLimits {
    fn default() -> Self {
        Self {
            max_rows: MAX_QUEST_RECORDS,
        }
    }
}

/// A source failure or pure row failure while loading quest state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestLoadError<E> {
    /// The injected source failed.
    Source(E),
    /// A source row or row bound was invalid.
    Rows(QuestRowError),
}

impl<E: fmt::Display> fmt::Display for QuestLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "quest source failed: {source}"),
            Self::Rows(source) => write!(formatter, "quest rows are invalid: {source}"),
        }
    }
}

impl<E: Error + 'static> Error for QuestLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Rows(source) => Some(source),
        }
    }
}

/// A bounded quest-load result. `Empty` is a valid SQL result, not an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestLookup {
    /// The query returned zero rows.
    Empty,
    /// The query returned one or more decoded records in source order.
    Found(Vec<QuestRecord>),
}

/// Execute a checked quest query and decode its rows.
///
/// # Errors
///
/// Returns [`QuestLoadError::Source`] for an injected source failure or
/// [`QuestLoadError::Rows`] when the bounded row policy rejects the result.
pub fn load_quest<S>(
    source: &S,
    query: &QuestQuery,
    limits: QuestLoadLimits,
) -> Result<QuestLookup, QuestLoadError<S::Error>>
where
    S: QuestRowSource,
{
    let rows = source.query_rows(query).map_err(QuestLoadError::Source)?;
    let records = decode_quest_rows(&rows, limits.max_rows).map_err(QuestLoadError::Rows)?;
    if records.is_empty() {
        Ok(QuestLookup::Empty)
    } else {
        Ok(QuestLookup::Found(records))
    }
}

/// A pure response adapter for the already-resolved quest lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestResponseAdapter {
    max_rows: usize,
}

impl Default for QuestResponseAdapter {
    fn default() -> Self {
        Self::standard()
    }
}

impl QuestResponseAdapter {
    /// Construct an adapter with an explicit output row bound.
    #[must_use]
    pub const fn new(max_rows: usize) -> Self {
        Self { max_rows }
    }

    /// Encode a valid empty or non-empty result as a DB peer frame.
    ///
    /// The frame header is `HEADER_DG_QUEST_LOAD`; the caller-owned handle is
    /// preserved. An empty result still carries a four-byte zero count.
    ///
    /// # Errors
    ///
    /// Returns the protocol record error when the row count or payload limit is
    /// exceeded.
    pub fn response_for(
        &self,
        handle: u32,
        lookup: &QuestLookup,
    ) -> Result<DbFrame, DbRecordError> {
        let records = match lookup {
            QuestLookup::Empty => &[][..],
            QuestLookup::Found(records) => records.as_slice(),
        };
        let payload = encode_quest_load_with_limit(records, self.max_rows)?;
        Ok(DbFrame::new(HEADER_DG_QUEST_LOAD, handle, payload))
    }
}

impl QuestResponseAdapter {
    /// Construct an adapter using the protocol's default row bound.
    #[must_use]
    pub const fn standard() -> Self {
        Self::new(MAX_QUEST_RECORDS)
    }
}

fn copy_c_string(source: &[u8], destination: &mut [u8]) {
    destination.fill(0);
    let content_len = source
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(source.len())
        .min(destination.len().saturating_sub(1));
    destination[..content_len].copy_from_slice(&source[..content_len]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_records::{QUEST_COUNT_WIRE_SIZE, QUEST_RECORD_WIRE_SIZE};

    fn row() -> QuestQueryRow {
        QuestQueryRow::new(
            Some(42),
            Some(b"quest\0suffix".to_vec()),
            Some(b"state".to_vec()),
            Some(-7),
        )
    }

    #[test]
    fn both_source_query_variants_are_exact_and_separately_bounded() {
        let postfix = TablePostfix::default();
        let cache = QuestQuery::new(&postfix, 42, QuestQueryKind::CacheHit).unwrap();
        let load = QuestQuery::new(&postfix, 42, QuestQueryKind::Load).unwrap();
        assert_eq!(
            cache.as_str(),
            "SELECT dwPID,szName,szState,lValue FROM quest WHERE dwPID=42 AND lValue<>0"
        );
        assert_eq!(
            load.as_str(),
            "SELECT dwPID, szName, szState, lValue FROM quest WHERE dwPID=42"
        );
        assert!(cache.as_str().len() <= MAX_QUEST_CACHE_QUERY_BYTES);
        assert!(load.as_str().len() <= MAX_QUEST_LOAD_QUERY_BYTES);
    }

    #[test]
    fn query_revalidates_postfix_and_preserves_player_id() {
        let postfix = TablePostfix::parse("_eu").unwrap();
        let query = QuestQuery::new(&postfix, u32::MAX, QuestQueryKind::Load).unwrap();
        assert_eq!(query.player_id(), u32::MAX);
        assert_eq!(query.table_name(), "quest_eu");
        assert!(matches!(
            QuestQuery::from_config(Some("bad-postfix"), 1, QuestQueryKind::Load),
            Err(QuestQueryBoundaryError::Postfix(_))
        ));
    }

    #[test]
    fn raw_name_and_state_copy_through_nul_and_truncate_with_zero_tail() {
        let mut source = row();
        source.name = Some(vec![b'a'; 80]);
        source.state = Some(b"abc\0ignored".to_vec());
        let record = decode_quest_row(&source).unwrap();
        assert_eq!(&record.name[..32], [b'a'; 32]);
        assert!(record.name[32..].iter().all(|byte| *byte == 0));
        assert_eq!(&record.state[..4], b"abc\0");
        assert!(record.state[4..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn null_and_numeric_range_failures_remain_typed() {
        let mut null = row();
        null.pid = None;
        assert_eq!(
            decode_quest_row(&null),
            Err(QuestRowError::NullColumn { column: 0 })
        );
        let mut pid = row();
        pid.pid = Some(-1);
        assert_eq!(
            decode_quest_row(&pid),
            Err(QuestRowError::PidOutOfRange { value: -1 })
        );
        let mut value = row();
        value.value = Some(i64::from(i32::MAX) + 1);
        assert_eq!(
            decode_quest_row(&value),
            Err(QuestRowError::ValueOutOfRange {
                value: 2_147_483_648
            })
        );
    }

    #[test]
    fn source_error_empty_and_found_results_are_distinct() {
        let postfix = TablePostfix::default();
        let query = QuestQuery::new(&postfix, 1, QuestQueryKind::Load).unwrap();
        let empty = |_query: &QuestQuery| Ok::<_, &'static str>(Vec::new());
        assert_eq!(
            load_quest(&empty, &query, QuestLoadLimits::default()).unwrap(),
            QuestLookup::Empty
        );
        let found = |_query: &QuestQuery| Ok::<_, &'static str>(vec![row()]);
        assert!(matches!(
            load_quest(&found, &query, QuestLoadLimits::default()).unwrap(),
            QuestLookup::Found(records) if records.len() == 1
        ));
        let failed = |_query: &QuestQuery| Err::<Vec<QuestQueryRow>, _>("database down");
        assert!(matches!(
            load_quest(&failed, &query, QuestLoadLimits::default()),
            Err(QuestLoadError::Source("database down"))
        ));
    }

    #[test]
    fn row_limit_is_checked_before_allocation() {
        let rows = vec![row(), row()];
        assert_eq!(
            decode_quest_rows(&rows, 1),
            Err(QuestRowError::TooManyRows {
                count: 2,
                maximum: 1
            })
        );
        assert_eq!(
            decode_quest_rows(&[], MAX_QUEST_RECORDS + 1),
            Err(QuestRowError::LimitTooLarge {
                requested: MAX_QUEST_RECORDS + 1,
                maximum: MAX_QUEST_RECORDS
            })
        );
    }

    #[test]
    fn response_adapter_preserves_handle_and_empty_count() {
        let adapter = QuestResponseAdapter::standard();
        let frame = adapter
            .response_for(0x1234_5678, &QuestLookup::Empty)
            .unwrap();
        assert_eq!(frame.header, HEADER_DG_QUEST_LOAD);
        assert_eq!(frame.handle, 0x1234_5678);
        assert_eq!(frame.payload, vec![0, 0, 0, 0]);
        assert_eq!(frame.payload.len(), QUEST_COUNT_WIRE_SIZE);
        assert_eq!(frame.encode().unwrap().len(), 9 + QUEST_COUNT_WIRE_SIZE);
    }

    #[test]
    fn response_adapter_encodes_found_records_in_order() {
        let mut first = row();
        first.pid = Some(1);
        let mut second = row();
        second.pid = Some(2);
        let records = vec![
            decode_quest_row(&first).unwrap(),
            decode_quest_row(&second).unwrap(),
        ];
        let frame = QuestResponseAdapter::standard()
            .response_for(7, &QuestLookup::Found(records))
            .unwrap();
        assert_eq!(
            frame.payload.len(),
            QUEST_COUNT_WIRE_SIZE + 2 * QUEST_RECORD_WIRE_SIZE
        );
        assert_eq!(
            frame.encode().unwrap().len(),
            9 + QUEST_COUNT_WIRE_SIZE + 2 * QUEST_RECORD_WIRE_SIZE
        );
        assert_eq!(&frame.payload[..4], &[2, 0, 0, 0]);
    }
}
