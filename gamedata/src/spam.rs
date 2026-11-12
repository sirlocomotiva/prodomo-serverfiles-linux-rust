//! The `spam_db` table rule.
//!
//! The game's `LoadSpamDB` (`server/server/game/db.cpp:760-779`) executes
//! `SELECT word, score FROM spam_db WHERE type='SPAM'` once at boot (the refresh
//! event behind `ENABLE_SPAMDB_REFRESH` is dead — `timed-events.md`,
//! `event.db.reload_spam_event`) and pushes each row into `SpamManager` with the
//! score `atoi` reads (`:798`). The owner's dump has no `spam_db` table at all.
//!
//! The caller supplies the rows. A source failure is the caller's error and
//! must never be turned into an empty table.

use std::error::Error;
use std::fmt;

use crate::sql_dump::{read_table, SqlDumpError, SqlValue};

/// The legacy statement, kept to document the two selected columns and the type filter.
pub const SPAM_LEGACY_QUERY: &str = "SELECT word, score FROM spam_db WHERE type='SPAM'";

/// Default record cap: the banword table's, whose legacy boot stream counted records in a
/// `WORD` (`gamedata::banword::BANWORD_DEFAULT_MAX_RECORDS`).
pub const SPAM_DEFAULT_MAX_RECORDS: usize = u16::MAX as usize;

/// One `SPAM` row as `SpamManager` holds it: the word `GetSpamScore` matches against the
/// spaceless line, and the score it adds on a hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpamWord {
    /// The `word` column, byte for byte. Legacy's `std::string` keeps an embedded NUL,
    /// and `WildCaseCmp` never matches past it, so the bytes are stored whole.
    pub word: Vec<u8>,
    /// The `score` column, as `atoi` reads it.
    pub score: u32,
}

/// `atoi` (`cstdlib`): the optional sign and the longest digit run, stopping at the first
/// other byte; no digits is 0. The score column is `int(3)`, and a stored negative score
/// converts to `unsigned int` at the `Insert` call (`G/db.cpp:798`).
#[must_use]
pub fn atoi_score(bytes: &[u8]) -> u32 {
    let (negative, digits) = match bytes.first() {
        Some(b'-') => (true, &bytes[1..]),
        Some(b'+') => (false, &bytes[1..]),
        _ => (false, bytes),
    };
    let mut magnitude: u32 = 0;
    for &byte in digits {
        let Some(digit) = byte
            .wrapping_sub(b'0')
            .checked_sub(10)
            .is_none()
            .then(|| byte - b'0')
        else {
            break;
        };
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add(u32::from(digit));
    }
    if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    }
}

/// Limits checked before a spam table is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpamLimits {
    /// Maximum number of source rows, including rows later skipped for NULL or type.
    pub max_source_rows: usize,
    /// Maximum number of `SPAM` records to emit.
    pub max_records: usize,
}

impl SpamLimits {
    /// Limits with the source-row cap equal to the record cap.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            max_source_rows: max_records,
            max_records,
        }
    }
}

impl Default for SpamLimits {
    fn default() -> Self {
        Self::new(SPAM_DEFAULT_MAX_RECORDS)
    }
}

/// A checked failure while building the spam table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpamTableError {
    /// The source returned more rows than the source-row cap.
    TooManySourceRows {
        /// Number of source rows received.
        count: usize,
        /// Configured source-row cap.
        maximum: usize,
    },
    /// The source holds more `SPAM` records than the record cap.
    TooManyRecords {
        /// Number of `SPAM` source records.
        count: usize,
        /// Configured record cap.
        maximum: usize,
    },
    /// The output could not reserve its checked length.
    AllocationFailed {
        /// Requested record count.
        requested: usize,
    },
}

impl fmt::Display for SpamTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySourceRows { count, maximum } => {
                write!(
                    formatter,
                    "spam source has {count} rows; maximum is {maximum}"
                )
            }
            Self::TooManyRecords { count, maximum } => {
                write!(
                    formatter,
                    "spam table has {count} records; maximum is {maximum}"
                )
            }
            Self::AllocationFailed { requested } => {
                write!(formatter, "spam allocation of {requested} records failed")
            }
        }
    }
}

impl Error for SpamTableError {}

/// One source row: the `word`, `score` and `type` columns, `None` for SQL `NULL`.
pub type SpamRow = (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>);

/// Build the spam table from one [`SpamRow`] per source row.
///
/// A `NULL` word is skipped without changing the order of the remaining rows, as
/// `banword`'s loader skips `NULL` values. A `NULL` score reads 0 — the column is
/// `NOT NULL`, so legacy never sees one. A type other than `SPAM` is skipped, as the
/// query's `WHERE` filters it. Both caps are checked before anything is allocated.
///
/// # Errors
///
/// Returns [`SpamTableError`] when a cap is exceeded or allocation fails.
pub fn build_spam_table(
    rows: &[SpamRow],
    limits: SpamLimits,
) -> Result<Vec<SpamWord>, SpamTableError> {
    if rows.len() > limits.max_source_rows {
        return Err(SpamTableError::TooManySourceRows {
            count: rows.len(),
            maximum: limits.max_source_rows,
        });
    }
    let kept = rows
        .iter()
        .filter(|(word, _, kind)| {
            word.is_some() && kind.as_deref().is_none_or(|kind| kind == b"SPAM")
        })
        .count();
    if kept > limits.max_records {
        return Err(SpamTableError::TooManyRecords {
            count: kept,
            maximum: limits.max_records,
        });
    }
    let mut records = Vec::new();
    records
        .try_reserve_exact(kept)
        .map_err(|_| SpamTableError::AllocationFailed { requested: kept })?;
    records.extend(rows.iter().filter_map(|(word, score, kind)| {
        let word = word.as_ref()?;
        if kind.as_deref().is_some_and(|kind| kind != b"SPAM") {
            return None;
        }
        Some(SpamWord {
            word: word.clone(),
            score: score.as_ref().map_or(0, |score| atoi_score(score)),
        })
    }));
    Ok(records)
}

/// A spam table that could not be read from the owner's dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpamDumpError {
    /// The dump could not be read.
    Dump(SqlDumpError),
    /// The `spam_db` statements have no `word` column.
    NoWordColumn,
    /// The `spam_db` statements have no `score` column.
    NoScoreColumn,
    /// A `word` value is a bare token, not a string or `NULL`.
    NotText {
        /// The 0-based row.
        row: usize,
    },
    /// The rows break a table cap.
    Table(SpamTableError),
}

impl fmt::Display for SpamDumpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dump(error) => write!(formatter, "spam dump: {error}"),
            Self::NoWordColumn => formatter.write_str("spam dump has no `word` column"),
            Self::NoScoreColumn => formatter.write_str("spam dump has no `score` column"),
            Self::NotText { row } => write!(formatter, "spam row {row} is not a string"),
            Self::Table(error) => error.fmt(formatter),
        }
    }
}

impl Error for SpamDumpError {}

/// Build the spam table from the `spam_db` rows of the owner's `common.sql` dump.
///
/// A dump without the table is the owner's shape: no words, not an error. The `type`
/// column may be absent, in which case every row counts as `SPAM`, because the legacy
/// query's filter only removes rows a type column names.
///
/// # Errors
///
/// Returns [`SpamDumpError`] when the dump cannot be read, a word is not a string or
/// `NULL`, or a cap is exceeded.
pub fn spam_words_from_dump(dump: &[u8]) -> Result<Vec<SpamWord>, SpamDumpError> {
    let table = read_table(dump, "spam_db").map_err(SpamDumpError::Dump)?;
    if table.rows.is_empty() {
        return Ok(Vec::new());
    }
    let word = table.column("word").ok_or(SpamDumpError::NoWordColumn)?;
    let score = table.column("score").ok_or(SpamDumpError::NoScoreColumn)?;
    let kind = table.column("type");
    let rows = table
        .rows
        .iter()
        .enumerate()
        .map(|(row, values)| {
            let word = match &values[word] {
                SqlValue::Null => None,
                SqlValue::Text(text) => Some(text.clone()),
                SqlValue::Bare(_) => return Err(SpamDumpError::NotText { row }),
            };
            let score = match &values[score] {
                SqlValue::Null => None,
                SqlValue::Text(text) => Some(text.clone()),
                SqlValue::Bare(token) => Some(token.clone()),
            };
            let kind = kind.map(|kind| match &values[kind] {
                SqlValue::Null => Vec::new(),
                SqlValue::Text(text) => text.clone(),
                SqlValue::Bare(token) => token.clone(),
            });
            Ok((word, score, kind))
        })
        .collect::<Result<Vec<_>, _>>()?;
    build_spam_table(&rows, SpamLimits::default()).map_err(SpamDumpError::Table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoi_reads_the_sign_and_the_digit_run() {
        assert_eq!(atoi_score(b"100"), 100);
        assert_eq!(atoi_score(b"12abc"), 12);
        assert_eq!(atoi_score(b""), 0);
        assert_eq!(atoi_score(b"abc"), 0);
        assert_eq!(
            atoi_score(b"-1"),
            0xffff_ffff,
            "a negative score converts to unsigned at the Insert call"
        );
        assert_eq!(atoi_score(b"+7"), 7);
    }

    #[test]
    fn the_dump_rows_become_the_table() {
        let dump = b"INSERT INTO `spam_db` (`word`, `score`, `type`) VALUES ('*gold*',100,'SPAM'),(NULL,5,'SPAM'),('maps',3,'MAPS');";
        let words = spam_words_from_dump(dump).unwrap();
        assert_eq!(
            words,
            [SpamWord {
                word: b"*gold*".to_vec(),
                score: 100,
            }],
            "a NULL word is skipped and MAPS rows are filtered"
        );
        assert_eq!(
            spam_words_from_dump(b"INSERT INTO `spam_db` (`word`) VALUES ('a');"),
            Err(SpamDumpError::NoScoreColumn)
        );
        assert_eq!(
            spam_words_from_dump(b"INSERT INTO `spam_db` (`word`, `score`) VALUES (7,1);"),
            Err(SpamDumpError::NotText { row: 0 })
        );
        assert_eq!(spam_words_from_dump(b"-- nothing\n"), Ok(Vec::new()));

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../legacy/sql/gamedata/common.sql"
        );
        let owner = spam_words_from_dump(&std::fs::read(path).unwrap()).unwrap();
        assert!(owner.is_empty(), "the owner ships no spam words");
    }

    #[test]
    fn the_legacy_query_selects_two_columns_with_a_type_filter() {
        assert_eq!(
            SPAM_LEGACY_QUERY,
            "SELECT word, score FROM spam_db WHERE type='SPAM'"
        );
    }
}
