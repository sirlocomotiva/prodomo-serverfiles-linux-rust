//! SQL-free `player_index` query and row boundary for login-by-key.
//!
//! Legacy evidence used by this module:
//!
//! * `server/server/db/ClientManagerLogin.cpp:146-148` constructs the
//!   login-by-key follow-up query as
//!   `SELECT pid1, pid2, pid3, pid4, empire FROM player_index%s WHERE id=%u`.
//! * `server/server/common/tables.h:380-418` defines the four account
//!   character IDs as `DWORD` values and the account empire as one byte.
//! * `server/server/common/length.h:55` gives the legacy query buffer an
//!   8,192-byte capacity, including its terminating NUL.
//! * `server/server/db/ClientManagerLogin.cpp:151-185` treats an empty
//!   `player_index` result as a distinct source condition and performs a
//!   second lookup/create flow. This module does not reproduce that stateful
//!   repair path.
//!
//! The query accepts only an already validated [`TablePostfix`]. The account
//! ID is supplied by a caller that has already resolved the login key; it is
//! not inferred from the 67-byte request. The row type keeps SQL `NULL`
//! distinct from a numeric value, and the strict decoder rejects `NULL` rather
//! than silently manufacturing zero IDs or an empire. The C++ parser passes
//! raw cells to permissive `str_to_number`; this boundary intentionally keeps
//! NULL and range failures typed because the repository has no verified SQL
//! schema for this table. This is a query/row boundary only: it does not open
//! a connection, execute SQL, create a missing index row, resolve a login, or
//! mutate login state.

use std::error::Error;
use std::fmt;

use crate::postfix::{TablePostfix, TablePostfixError, MAX_TABLE_POSTFIX_BYTES};

/// Base table name used by the source login-by-key query.
pub const PLAYER_INDEX_TABLE: &str = "player_index";

/// Exact fixed-column prefix from the source login-by-key query.
pub const PLAYER_INDEX_QUERY_PREFIX: &str = "SELECT pid1, pid2, pid3, pid4, empire FROM ";

/// Number of columns promised by [`PLAYER_INDEX_QUERY_PREFIX`].
pub const PLAYER_INDEX_QUERY_COLUMNS: usize = 5;

/// Maximum statement bytes that fit in the legacy 8,192-byte query buffer.
///
/// The terminating NUL occupies one byte, so the statement itself is bounded
/// at 8,191 bytes.
pub const MAX_PLAYER_INDEX_QUERY_BYTES: usize = 8_191;

const MAX_PLAYER_INDEX_TABLE_BYTES: usize = PLAYER_INDEX_TABLE.len() + MAX_TABLE_POSTFIX_BYTES;

/// A defensive failure while constructing the fixed `player_index` query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerIndexQueryBuildError {
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
    /// The generated statement would not fit the source query buffer.
    QueryTooLong {
        /// Generated statement byte length.
        length: usize,
        /// Maximum accepted statement byte length.
        maximum: usize,
    },
}

impl fmt::Display for PlayerIndexQueryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableNameTooLong { length, maximum } => write!(
                formatter,
                "generated player_index table name is {length} bytes; maximum is {maximum}"
            ),
            Self::InvalidTableIdentifier { index, byte } => write!(
                formatter,
                "generated player_index identifier has byte {byte:#04x} at offset {index}"
            ),
            Self::QueryTooLong { length, maximum } => write!(
                formatter,
                "generated player_index query is {length} bytes; maximum is {maximum}"
            ),
        }
    }
}

impl Error for PlayerIndexQueryBuildError {}

/// An immutable, checked `player_index` read statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerIndexQuery {
    statement: String,
    table_name: String,
    account_id: u32,
    postfix: TablePostfix,
}

impl PlayerIndexQuery {
    /// Build the exact source login-by-key query for one account ID.
    ///
    /// The account ID is a numeric `u32` and therefore cannot introduce SQL
    /// syntax. The table suffix is validated before it is interpolated.
    ///
    /// # Errors
    ///
    /// Returns [`PlayerIndexQueryBuildError`] if a defensive identifier or
    /// statement-width check fails.
    pub fn new(
        postfix: &TablePostfix,
        account_id: u32,
    ) -> Result<Self, PlayerIndexQueryBuildError> {
        let table_name = format!("{PLAYER_INDEX_TABLE}{}", postfix.as_str());
        if table_name.len() > MAX_PLAYER_INDEX_TABLE_BYTES {
            return Err(PlayerIndexQueryBuildError::TableNameTooLong {
                length: table_name.len(),
                maximum: MAX_PLAYER_INDEX_TABLE_BYTES,
            });
        }
        if let Some((index, byte)) = table_name
            .bytes()
            .enumerate()
            .find(|(_, byte)| !byte.is_ascii_alphanumeric() && *byte != b'_')
        {
            return Err(PlayerIndexQueryBuildError::InvalidTableIdentifier { index, byte });
        }

        let statement = format!("{PLAYER_INDEX_QUERY_PREFIX}{table_name} WHERE id={account_id}");
        if statement.len() > MAX_PLAYER_INDEX_QUERY_BYTES {
            return Err(PlayerIndexQueryBuildError::QueryTooLong {
                length: statement.len(),
                maximum: MAX_PLAYER_INDEX_QUERY_BYTES,
            });
        }

        Ok(Self {
            statement,
            table_name,
            account_id,
            postfix: postfix.clone(),
        })
    }

    /// Validate an optional configured postfix and build the query.
    ///
    /// # Errors
    ///
    /// Returns [`PlayerIndexQueryBuildError`] for a defensive construction
    /// failure or [`TablePostfixError`] for an invalid configured postfix.
    pub fn from_config(
        configured_postfix: Option<&str>,
        account_id: u32,
    ) -> Result<Self, PlayerIndexQueryBoundaryError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(PlayerIndexQueryBoundaryError::Postfix)?;
        Self::new(&postfix, account_id).map_err(PlayerIndexQueryBoundaryError::Query)
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

    /// Return the numeric account ID used in the source query.
    #[must_use]
    pub const fn account_id(&self) -> u32 {
        self.account_id
    }

    /// Borrow the validated postfix used to construct the query.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }
}

/// A failure while validating a postfix or fixed query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerIndexQueryBoundaryError {
    /// `TABLE_POSTFIX` failed validation.
    Postfix(TablePostfixError),
    /// The fixed query failed a defensive construction check.
    Query(PlayerIndexQueryBuildError),
}

impl fmt::Display for PlayerIndexQueryBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(source) => source.fmt(formatter),
            Self::Query(source) => source.fmt(formatter),
        }
    }
}

impl Error for PlayerIndexQueryBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postfix(source) => Some(source),
            Self::Query(source) => Some(source),
        }
    }
}

/// One source-shaped `player_index` row.
///
/// `Option` is intentional: it preserves SQL `NULL` until the strict decoder
/// makes an explicit decision. It is not a zero-filled wire record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerIndexQueryRow {
    /// `pid1` as an optional unsigned 32-bit value.
    pub pid1: Option<u32>,
    /// `pid2` as an optional unsigned 32-bit value.
    pub pid2: Option<u32>,
    /// `pid3` as an optional unsigned 32-bit value.
    pub pid3: Option<u32>,
    /// `pid4` as an optional unsigned 32-bit value.
    pub pid4: Option<u32>,
    /// `empire` as an optional source integer, checked against the C++ byte.
    pub empire: Option<u32>,
}

impl PlayerIndexQueryRow {
    /// Return the five source values in query order.
    #[must_use]
    pub fn values(self) -> [Option<u32>; 5] {
        [self.pid1, self.pid2, self.pid3, self.pid4, self.empire]
    }
}

/// A strictly decoded `player_index` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerIndexRecord {
    /// The four account character IDs in source order.
    pub player_ids: [u32; 4],
    /// The account empire byte.
    pub empire: u8,
}

/// A strict `player_index` row error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerIndexRowError {
    /// A source column was SQL `NULL` and was not converted to zero.
    NullColumn {
        /// Source column name.
        column: &'static str,
    },
    /// The source `empire` integer does not fit the C++ `BYTE` field.
    EmpireOutOfRange {
        /// Source integer value.
        value: u32,
    },
}

impl fmt::Display for PlayerIndexRowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NullColumn { column } => {
                write!(formatter, "player_index column {column} is NULL")
            }
            Self::EmpireOutOfRange { value } => write!(
                formatter,
                "player_index empire value {value} does not fit a C++ BYTE"
            ),
        }
    }
}

impl Error for PlayerIndexRowError {}

/// Strictly decode one source-shaped `player_index` row.
///
/// # Errors
///
/// Returns [`PlayerIndexRowError::NullColumn`] for any SQL `NULL`; the legacy
/// permissive `str_to_number` path is intentionally not reproduced because it
/// cannot distinguish that state from a valid zero.
pub fn decode_player_index_row(
    row: &PlayerIndexQueryRow,
) -> Result<PlayerIndexRecord, PlayerIndexRowError> {
    let pid1 = row
        .pid1
        .ok_or(PlayerIndexRowError::NullColumn { column: "pid1" })?;
    let pid2 = row
        .pid2
        .ok_or(PlayerIndexRowError::NullColumn { column: "pid2" })?;
    let pid3 = row
        .pid3
        .ok_or(PlayerIndexRowError::NullColumn { column: "pid3" })?;
    let pid4 = row
        .pid4
        .ok_or(PlayerIndexRowError::NullColumn { column: "pid4" })?;
    let empire = row
        .empire
        .ok_or(PlayerIndexRowError::NullColumn { column: "empire" })?;
    let empire = u8::try_from(empire)
        .map_err(|_| PlayerIndexRowError::EmpireOutOfRange { value: empire })?;
    Ok(PlayerIndexRecord {
        player_ids: [pid1, pid2, pid3, pid4],
        empire,
    })
}

/// The result of a caller-owned `player_index` lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerIndexLookup {
    /// The source query returned no row.
    Missing,
    /// One strict source row was decoded.
    Found(PlayerIndexRecord),
}

/// A caller-owned source seam for the checked `player_index` query.
///
/// The trait is not a database implementation. A future `SQLx` adapter may
/// execute [`PlayerIndexQuery::as_str`], but it must preserve database errors,
/// `NULL`, and empty results rather than manufacturing a missing row.
pub trait PlayerIndexRowSource {
    /// Source-specific acquisition error.
    type Error: fmt::Display;

    /// Obtain at most the one row promised by the account-key query.
    ///
    /// # Errors
    ///
    /// Returns the source error when the query cannot be completed. An empty
    /// result is represented by `Ok(None)`, never by an error or fabricated row.
    fn query_row(
        &self,
        query: &PlayerIndexQuery,
    ) -> Result<Option<PlayerIndexQueryRow>, Self::Error>;
}

impl<F, E> PlayerIndexRowSource for F
where
    F: Fn(&PlayerIndexQuery) -> Result<Option<PlayerIndexQueryRow>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_row(
        &self,
        query: &PlayerIndexQuery,
    ) -> Result<Option<PlayerIndexQueryRow>, Self::Error> {
        self(query)
    }
}

/// A failure while acquiring or decoding one `player_index` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerIndexLoadError<E> {
    /// The caller-owned source failed.
    Source(E),
    /// The returned row contained SQL `NULL`.
    Row(PlayerIndexRowError),
}

impl<E: fmt::Display> fmt::Display for PlayerIndexLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "player_index source failed: {source}"),
            Self::Row(source) => write!(formatter, "player_index row is invalid: {source}"),
        }
    }
}

impl<E: Error + 'static> Error for PlayerIndexLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Row(source) => Some(source),
        }
    }
}

/// Acquire and strictly decode one caller-resolved `player_index` row.
///
/// # Errors
///
/// Returns [`PlayerIndexLoadError::Source`] for acquisition failures and
/// [`PlayerIndexLoadError::Row`] for SQL `NULL` values. An empty source result
/// is [`PlayerIndexLookup::Missing`], not an error.
pub fn load_player_index<S: PlayerIndexRowSource>(
    source: &S,
    query: &PlayerIndexQuery,
) -> Result<PlayerIndexLookup, PlayerIndexLoadError<S::Error>> {
    let row = source
        .query_row(query)
        .map_err(PlayerIndexLoadError::Source)?;
    match row {
        None => Ok(PlayerIndexLookup::Missing),
        Some(row) => decode_player_index_row(&row)
            .map(PlayerIndexLookup::Found)
            .map_err(PlayerIndexLoadError::Row),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> PlayerIndexQuery {
        PlayerIndexQuery::from_config(None, 7).unwrap()
    }

    fn row() -> PlayerIndexQueryRow {
        PlayerIndexQueryRow {
            pid1: Some(11),
            pid2: Some(22),
            pid3: Some(33),
            pid4: Some(44),
            empire: Some(2),
        }
    }

    #[test]
    fn query_matches_the_source_default_statement() {
        let query = query();
        assert_eq!(
            query.as_str(),
            "SELECT pid1, pid2, pid3, pid4, empire FROM player_index WHERE id=7"
        );
        assert_eq!(query.table_name(), "player_index");
        assert_eq!(query.account_id(), 7);
        assert!(query.postfix().is_empty());
    }

    #[test]
    fn query_validates_postfix_and_preserves_account_id() {
        let query = PlayerIndexQuery::from_config(Some("eu_2"), u32::MAX).unwrap();
        assert_eq!(
            query.as_str(),
            "SELECT pid1, pid2, pid3, pid4, empire FROM player_indexeu_2 WHERE id=4294967295"
        );
        assert_eq!(query.table_name(), "player_indexeu_2");
        assert!(matches!(
            PlayerIndexQuery::from_config(Some("bad-postfix"), 1),
            Err(PlayerIndexQueryBoundaryError::Postfix(_))
        ));
    }

    #[test]
    fn row_values_are_kept_in_source_order() {
        assert_eq!(
            row().values(),
            [Some(11), Some(22), Some(33), Some(44), Some(2)]
        );
        assert_eq!(
            decode_player_index_row(&row()).unwrap(),
            PlayerIndexRecord {
                player_ids: [11, 22, 33, 44],
                empire: 2,
            }
        );
    }

    #[test]
    fn null_columns_are_distinct_and_never_become_zero() {
        for column in ["pid1", "pid2", "pid3", "pid4", "empire"] {
            let mut value = row();
            match column {
                "pid1" => value.pid1 = None,
                "pid2" => value.pid2 = None,
                "pid3" => value.pid3 = None,
                "pid4" => value.pid4 = None,
                "empire" => value.empire = None,
                _ => unreachable!(),
            }
            assert_eq!(
                decode_player_index_row(&value),
                Err(PlayerIndexRowError::NullColumn { column })
            );
        }
    }

    #[test]
    fn empire_is_checked_against_the_source_byte_width() {
        for empire in [0, 255] {
            let mut value = row();
            value.empire = Some(empire);
            assert_eq!(
                decode_player_index_row(&value).unwrap().empire,
                u8::try_from(empire).unwrap()
            );
        }
        let mut value = row();
        value.empire = Some(256);
        assert_eq!(
            decode_player_index_row(&value),
            Err(PlayerIndexRowError::EmpireOutOfRange { value: 256 })
        );
    }

    #[test]
    fn zero_player_ids_are_valid_source_values() {
        let value = PlayerIndexQueryRow {
            pid1: Some(0),
            pid2: Some(0),
            pid3: Some(0),
            pid4: Some(0),
            empire: Some(0),
        };
        assert_eq!(
            decode_player_index_row(&value).unwrap(),
            PlayerIndexRecord {
                player_ids: [0, 0, 0, 0],
                empire: 0,
            }
        );
    }

    #[test]
    fn injected_source_distinguishes_missing_found_and_failure() {
        let query = query();
        let found = |_: &PlayerIndexQuery| Ok::<_, &'static str>(Some(row()));
        assert_eq!(
            load_player_index(&found, &query).unwrap(),
            PlayerIndexLookup::Found(PlayerIndexRecord {
                player_ids: [11, 22, 33, 44],
                empire: 2,
            })
        );

        let missing = |_: &PlayerIndexQuery| Ok::<_, &'static str>(None);
        assert_eq!(
            load_player_index(&missing, &query).unwrap(),
            PlayerIndexLookup::Missing
        );

        let failed =
            |_: &PlayerIndexQuery| Err::<Option<PlayerIndexQueryRow>, &'static str>("db down");
        assert_eq!(
            load_player_index(&failed, &query),
            Err(PlayerIndexLoadError::Source("db down"))
        );
    }

    #[test]
    fn source_query_is_fixed_and_bounded() {
        assert_eq!(PLAYER_INDEX_QUERY_COLUMNS, 5);
        assert_eq!(MAX_PLAYER_INDEX_QUERY_BYTES, 8_191);
        assert_eq!(
            PlayerIndexQuery::from_config(Some("a"), 1)
                .unwrap()
                .as_str(),
            "SELECT pid1, pid2, pid3, pid4, empire FROM player_indexa WHERE id=1"
        );
    }
}
