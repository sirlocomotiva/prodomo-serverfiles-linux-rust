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
}
