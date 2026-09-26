//! The `banword` table rule.
//!
//! The legacy loader in `server/server/db/ClientManagerBoot.cpp:745-771`
//! executes `SELECT word FROM banword`, skips `NULL` values, and preserves
//! row order. Each non-NULL value is copied into `TBanwordTable::szWord`
//! (`char[BANWORD_MAX_LEN + 1]`, 25 bytes) by `strlcpy`, so a value is cut at
//! its first NUL and truncated to 24 bytes. This module zero-fills the rest of
//! the field; the legacy destination's indeterminate tail bytes are not
//! reproduced.
//!
//! The caller supplies the rows. A source failure is the caller's error and
//! must never be turned into an empty table.

use std::error::Error;
use std::fmt;

use common::tables::BANWORD_MAX_LEN;

use crate::sql_dump::{read_table, SqlDumpError, SqlValue};

/// The legacy statement, kept to document the single selected column.
pub const BANWORD_LEGACY_QUERY: &str = "SELECT word FROM banword";

/// Width of the `szWord` field, including its terminator.
pub const BANWORD_BYTES: usize = BANWORD_MAX_LEN + 1;

/// Default record cap: the legacy boot stream counted records in a `WORD`,
/// so no legacy table held more.
pub const BANWORD_DEFAULT_MAX_RECORDS: usize = u16::MAX as usize;

/// One banned word as the legacy `TBanwordTable` held it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BanwordRecord {
    /// The raw, NUL-terminated `szWord` bytes. No encoding is assumed.
    pub word: [u8; BANWORD_BYTES],
}

impl BanwordRecord {
    /// Build a record the way `strlcpy` fills `szWord`.
    #[must_use]
    pub fn from_value(value: &[u8]) -> Self {
        let mut word = [0_u8; BANWORD_BYTES];
        // `strlcpy` stops at the first NUL, so bytes after an embedded NUL
        // are never copied.
        let source_len = value
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(value.len());
        let copy_len = source_len.min(BANWORD_MAX_LEN);
        word[..copy_len].copy_from_slice(&value[..copy_len]);
        Self { word }
    }

    /// The word up to its terminator.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        let len = self
            .word
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(BANWORD_BYTES);
        &self.word[..len]
    }
}

/// Limits checked before a banword table is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BanwordLimits {
    /// Maximum number of source rows, including rows later skipped for NULL.
    pub max_source_rows: usize,
    /// Maximum number of non-NULL records to emit.
    pub max_records: usize,
}

impl BanwordLimits {
    /// Limits with the source-row cap equal to the record cap.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            max_source_rows: max_records,
            max_records,
        }
    }
}

impl Default for BanwordLimits {
    fn default() -> Self {
        Self::new(BANWORD_DEFAULT_MAX_RECORDS)
    }
}

/// A checked failure while building the banword table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BanwordTableError {
    /// The source returned more rows than the source-row cap.
    TooManySourceRows {
        /// Number of source rows received.
        count: usize,
        /// Configured source-row cap.
        maximum: usize,
    },
    /// The source holds more non-NULL records than the record cap.
    TooManyRecords {
        /// Number of non-NULL source records.
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

impl fmt::Display for BanwordTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySourceRows { count, maximum } => {
                write!(
                    formatter,
                    "banword source has {count} rows; maximum is {maximum}"
                )
            }
            Self::TooManyRecords { count, maximum } => {
                write!(
                    formatter,
                    "banword table has {count} records; maximum is {maximum}"
                )
            }
            Self::AllocationFailed { requested } => {
                write!(
                    formatter,
                    "banword allocation of {requested} records failed"
                )
            }
        }
    }
}

impl Error for BanwordTableError {}

/// Build the banword table from the single `word` column of each row.
///
/// `None` is SQL `NULL` and is skipped without changing the order of the
/// remaining values. Both caps are checked before anything is allocated.
///
/// # Errors
///
/// Returns [`BanwordTableError`] when a cap is exceeded or allocation fails.
pub fn build_banword_table(
    rows: &[Option<Vec<u8>>],
    limits: BanwordLimits,
) -> Result<Vec<BanwordRecord>, BanwordTableError> {
    if rows.len() > limits.max_source_rows {
        return Err(BanwordTableError::TooManySourceRows {
            count: rows.len(),
            maximum: limits.max_source_rows,
        });
    }
    let record_count = rows.iter().filter(|row| row.is_some()).count();
    if record_count > limits.max_records {
        return Err(BanwordTableError::TooManyRecords {
            count: record_count,
            maximum: limits.max_records,
        });
    }

    let mut records = Vec::new();
    records
        .try_reserve_exact(record_count)
        .map_err(|_| BanwordTableError::AllocationFailed {
            requested: record_count,
        })?;
    records.extend(
        rows.iter()
            .flatten()
            .map(|row| BanwordRecord::from_value(row)),
    );
    Ok(records)
}

/// A banword table that could not be read from the owner's dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BanwordDumpError {
    /// The dump could not be read.
    Dump(SqlDumpError),
    /// The `banword` statements have no `word` column.
    NoWordColumn,
    /// A `word` value is a bare token, not a string or `NULL`.
    NotText {
        /// The 0-based row.
        row: usize,
    },
    /// The rows break a table cap.
    Table(BanwordTableError),
}

impl fmt::Display for BanwordDumpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dump(error) => write!(formatter, "banword dump: {error}"),
            Self::NoWordColumn => formatter.write_str("banword dump has no `word` column"),
            Self::NotText { row } => write!(formatter, "banword row {row} is not a string"),
            Self::Table(error) => error.fmt(formatter),
        }
    }
}

impl Error for BanwordDumpError {}

/// Build the banword table from the `banword` rows of the owner's `player.sql` dump.
///
/// # Errors
///
/// Returns [`BanwordDumpError`] when the dump cannot be read, a value is not a string, or a
/// cap is exceeded.
pub fn banwords_from_dump(dump: &[u8]) -> Result<Vec<BanwordRecord>, BanwordDumpError> {
    let table = read_table(dump, "banword").map_err(BanwordDumpError::Dump)?;
    let word = match table.column("word") {
        Some(word) => word,
        None if table.rows.is_empty() => return Ok(Vec::new()),
        None => return Err(BanwordDumpError::NoWordColumn),
    };
    let rows = table
        .rows
        .iter()
        .enumerate()
        .map(|(row, values)| match &values[word] {
            SqlValue::Null => Ok(None),
            SqlValue::Text(text) => Ok(Some(text.clone())),
            SqlValue::Bare(_) => Err(BanwordDumpError::NotText { row }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    build_banword_table(&rows, BanwordLimits::default()).map_err(BanwordDumpError::Table)
}

/// Whether a Name holds a banned word: legacy `CBanwordManager::CheckString`
/// (`G/banword.cpp:34-73`), which compares each word with `strncmp` at every position of the
/// Name. The match is case-sensitive, and an empty word matches every Name that is not empty.
/// Only single-byte positions are ported, because a Name reaching this check is ASCII.
#[must_use]
pub fn holds_banword(words: &[BanwordRecord], name: &[u8]) -> bool {
    words.iter().any(|record| {
        let word = record.as_bytes();
        (0..name.len()).any(|start| name[start..].starts_with(word))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(values: &[Option<&[u8]>]) -> Vec<Option<Vec<u8>>> {
        values
            .iter()
            .map(|value| value.map(<[u8]>::to_vec))
            .collect()
    }

    #[test]
    fn skips_nulls_preserves_order_and_zero_fills_the_field_tail() {
        let source = rows(&[None, Some(b"first"), None, Some(b"a\0b"), Some(&[b'x'; 30])]);
        let table = build_banword_table(&source, BanwordLimits::default()).unwrap();

        assert_eq!(table.len(), 3);
        assert_eq!(table[0].as_bytes(), b"first");
        let mut cut = [0_u8; BANWORD_BYTES];
        cut[0] = b'a';
        assert_eq!(table[1].word, cut, "strlcpy stops at the embedded NUL");
        let mut truncated = [b'x'; BANWORD_BYTES];
        truncated[BANWORD_BYTES - 1] = 0;
        assert_eq!(table[2].word, truncated);
    }

    #[test]
    fn keeps_raw_non_utf8_empty_and_boundary_length_values() {
        let source = rows(&[
            Some(&[0xff, 0xfe]),
            Some(b""),
            Some(&[b'a'; 24]),
            Some(&[b'b'; 25]),
        ]);
        let table = build_banword_table(&source, BanwordLimits::default()).unwrap();

        assert_eq!(table[0].word[0..2], [0xff, 0xfe]);
        assert_eq!(table[0].word[2..], [0_u8; BANWORD_BYTES - 2]);
        assert_eq!(table[1].word, [0_u8; BANWORD_BYTES]);
        assert_eq!(table[2].as_bytes(), &[b'a'; 24]);
        assert_eq!(table[3].as_bytes(), &[b'b'; 24], "25 bytes truncate to 24");
    }

    #[test]
    fn null_rows_do_not_use_record_capacity_but_count_as_source_rows() {
        let values = rows(&[None, Some(b"ok"), None]);
        let limits = BanwordLimits {
            max_source_rows: 3,
            max_records: 1,
        };
        assert_eq!(build_banword_table(&values, limits).unwrap().len(), 1);

        let limits = BanwordLimits {
            max_source_rows: 2,
            max_records: 1,
        };
        assert_eq!(
            build_banword_table(&rows(&[None, None, None]), limits),
            Err(BanwordTableError::TooManySourceRows {
                count: 3,
                maximum: 2
            })
        );
    }

    #[test]
    fn the_record_cap_is_checked_before_building() {
        let values = rows(&[Some(b"a"), Some(b"b"), Some(b"c")]);
        let limits = BanwordLimits {
            max_source_rows: 3,
            max_records: 2,
        };
        assert_eq!(
            build_banword_table(&values, limits),
            Err(BanwordTableError::TooManyRecords {
                count: 3,
                maximum: 2
            })
        );
    }

    #[test]
    fn the_legacy_query_selects_one_column() {
        assert_eq!(BANWORD_LEGACY_QUERY, "SELECT word FROM banword");
        assert_eq!(BANWORD_BYTES, 25);
    }

    #[test]
    fn a_banned_word_anywhere_in_the_name_refuses_it_by_case() {
        let words = build_banword_table(
            &rows(&[Some(b"ass"), Some(b"Bad")]),
            BanwordLimits::default(),
        )
        .unwrap();
        for name in [&b"ass"[..], b"xassx", b"glass", b"myBad"] {
            assert!(holds_banword(&words, name), "{name:?}");
        }
        for name in [&b"as"[..], b"ASS", b"bad", b"Ba", b""] {
            assert!(!holds_banword(&words, name), "{name:?}");
        }
        let empty = build_banword_table(&rows(&[Some(b"")]), BanwordLimits::default()).unwrap();
        assert!(holds_banword(&empty, b"x"));
        assert!(!holds_banword(&empty, b""));
    }

    #[test]
    fn the_dump_rows_become_the_table() {
        let dump = b"INSERT INTO `banword` (`word`) VALUES ('ab'),(NULL),('c\\0');";
        let words = banwords_from_dump(dump).unwrap();
        let words: Vec<&[u8]> = words.iter().map(BanwordRecord::as_bytes).collect();
        assert_eq!(
            words,
            [&b"ab"[..], b"c"],
            "an escaped NUL ends the word, as strlcpy stops there"
        );
        assert_eq!(
            banwords_from_dump(b"INSERT INTO `banword` (`word`) VALUES (7);"),
            Err(BanwordDumpError::NotText { row: 0 })
        );
        assert_eq!(
            banwords_from_dump(b"INSERT INTO `banword` (`w`) VALUES ('a');"),
            Err(BanwordDumpError::NoWordColumn)
        );
        assert_eq!(banwords_from_dump(b"-- nothing\n"), Ok(Vec::new()));

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../legacy/sql/gamedata/player.sql"
        );
        let owner = banwords_from_dump(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(owner.len(), 115);
        assert!(holds_banword(&owner, b"Xbitchy"));
        assert!(!holds_banword(&owner, b"Hero"));
    }
}
