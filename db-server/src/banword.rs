//! Source-verified, SQL-free `banword` table loading.
//!
//! The legacy loader in `server/server/db/ClientManagerBoot.cpp:745-771`
//! executes the fixed statement `SELECT word FROM banword` (there is no
//! `TABLE_POSTFIX` in this query), skips `NULL` values, and preserves database
//! row order. Each non-NULL value is copied into a packed
//! `TBanwordTable::szWord` field by `strlcpy`; the active x86 record is
//! `char[BANWORD_MAX_LEN + 1]`, or 25 bytes. Consequently a source value is
//! truncated to 24 bytes and the remaining bytes are zero-filled by this
//! deterministic boundary. The legacy destination's indeterminate tail bytes
//! are not invented here.
//!
//! This module deliberately does not execute SQL or depend on `SQLx`. The
//! caller owns query execution and supplies source-shaped rows through
//! [`BanwordRowSource`]. The loader then applies checked section limits and
//! returns the same [`BootSection`] shape consumed by the existing boot
//! parser. A source failure remains an error; it is never turned into an
//! empty table.

use std::error::Error;
use std::fmt;

use common::tables::BANWORD_MAX_LEN;
use protocol::db_boot::{BootSection, BootSectionKind, BANWORD_WIRE_SIZE};

/// Exact source-fixed banword query.
pub const BANWORD_QUERY: &str = "SELECT word FROM banword";

/// Packed x86 width of one `TBanwordTable` record.
pub const BANWORD_RECORD_SIZE: usize = BANWORD_WIRE_SIZE;

/// The fixed query object exposed to an injected row source.
///
/// The query has no dynamic table name or bind parameter. Keeping it as a
/// distinct value makes it difficult for a future adapter to accidentally
/// interpolate unrelated SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BanwordQuery;

impl BanwordQuery {
    /// Return the exact source statement.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        BANWORD_QUERY
    }
}

/// Limits applied before a banword section is allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BanwordSectionLimits {
    /// Maximum number of source rows, including rows later skipped for NULL.
    pub max_source_rows: usize,
    /// Maximum number of non-NULL records to emit.
    pub max_records: usize,
    /// Maximum number of packed record-data bytes to emit.
    pub max_data_bytes: usize,
}

impl BanwordSectionLimits {
    /// Construct limits with the source-row cap equal to the wire count cap
    /// and the maximum representable packed byte count.
    #[must_use]
    pub const fn new(max_records: usize) -> Self {
        Self {
            max_source_rows: max_records,
            max_records,
            max_data_bytes: BANWORD_RECORD_SIZE * u16::MAX as usize,
        }
    }

    /// Construct all three bounds explicitly.
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

impl Default for BanwordSectionLimits {
    fn default() -> Self {
        Self::new(u16::MAX as usize)
    }
}

/// A caller-owned source of raw `banword` values.
///
/// `Some(bytes)` is one non-NULL SQL value. `None` represents SQL `NULL`,
/// which the legacy loader skips. The source must return an error rather than
/// an empty vector when query execution or column extraction fails.
pub trait BanwordRowSource {
    /// Source-specific error type.
    type Error: fmt::Display;

    /// Execute the fixed query and return its rows in database order.
    ///
    /// # Errors
    ///
    /// Returns the source error when the query or a column cannot be read.
    fn query_rows(&self, query: &BanwordQuery) -> Result<Vec<Option<Vec<u8>>>, Self::Error>;
}

impl<F, E> BanwordRowSource for F
where
    F: Fn(&BanwordQuery) -> Result<Vec<Option<Vec<u8>>>, E>,
    E: fmt::Display,
{
    type Error = E;

    fn query_rows(&self, query: &BanwordQuery) -> Result<Vec<Option<Vec<u8>>>, Self::Error> {
        self(query)
    }
}

/// A checked failure while turning banword rows into a boot section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BanwordSectionError {
    /// The source returned more rows than the configured source-row cap.
    TooManySourceRows {
        /// Number of source rows received.
        count: usize,
        /// Configured source-row cap.
        maximum: usize,
    },
    /// The source contains more non-NULL records than the configured cap.
    TooManyRecords {
        /// Number of non-NULL source records.
        count: usize,
        /// Configured record cap.
        maximum: usize,
    },
    /// The non-NULL record count cannot be represented by the boot `u16`.
    CountOverflow {
        /// Number of non-NULL source records.
        count: usize,
    },
    /// Packed data-size arithmetic overflowed `usize`.
    DataSizeOverflow {
        /// Record count used in the multiplication.
        count: usize,
    },
    /// The packed record data exceeds the configured byte cap.
    DataTooLarge {
        /// Required packed record-data length.
        length: usize,
        /// Configured byte cap.
        maximum: usize,
    },
    /// The fixed wire record width cannot be represented by the boot `u16`.
    RecordSizeOverflow {
        /// Computed record width.
        size: usize,
    },
    /// The output vector could not reserve its checked data length.
    AllocationFailed {
        /// Requested allocation size.
        requested: usize,
    },
}

impl fmt::Display for BanwordSectionError {
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
            Self::CountOverflow { count } => {
                write!(formatter, "banword record count {count} does not fit u16")
            }
            Self::DataSizeOverflow { count } => {
                write!(
                    formatter,
                    "banword data size overflows usize for {count} records"
                )
            }
            Self::DataTooLarge { length, maximum } => {
                write!(
                    formatter,
                    "banword data length {length} exceeds limit {maximum}"
                )
            }
            Self::RecordSizeOverflow { size } => {
                write!(formatter, "banword record size {size} does not fit u16")
            }
            Self::AllocationFailed { requested } => {
                write!(formatter, "banword allocation of {requested} bytes failed")
            }
        }
    }
}

impl Error for BanwordSectionError {}

/// An error while obtaining banword rows or building their section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BanwordLoadError<E> {
    /// The injected source failed.
    Source(E),
    /// Source rows could not be bounded or encoded.
    Section(BanwordSectionError),
}

impl<E: fmt::Display> fmt::Display for BanwordLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write!(formatter, "banword row source failed: {source}"),
            Self::Section(source) => {
                write!(formatter, "banword section construction failed: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for BanwordLoadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(source) => Some(source),
            Self::Section(source) => Some(source),
        }
    }
}

/// Build a checked banword boot section from source-shaped rows.
///
/// The source-row cap is checked first, then `NULL` rows are skipped without
/// changing the order of the remaining values. Every emitted value is copied
/// into a fresh 25-byte array: at most [`BANWORD_MAX_LEN`] bytes are copied and
/// all remaining bytes stay zero. This deterministic tail is safer than the
/// legacy destination's indeterminate bytes and is not claimed as byte-for-byte
/// parity.
/// The output count and byte length are checked before allocation or encoding,
/// and the output vector reserves its exact checked length with a fallible API.
///
/// # Errors
///
/// Returns [`BanwordSectionError`] when a source-row, record-count, count,
/// packed-byte, record-width, or allocation limit is exceeded.
pub fn build_banword_section(
    rows: &[Option<Vec<u8>>],
    limits: BanwordSectionLimits,
) -> Result<BootSection, BanwordSectionError> {
    if rows.len() > limits.max_source_rows {
        return Err(BanwordSectionError::TooManySourceRows {
            count: rows.len(),
            maximum: limits.max_source_rows,
        });
    }

    let record_count = rows.iter().filter(|row| row.is_some()).count();
    if record_count > limits.max_records {
        return Err(BanwordSectionError::TooManyRecords {
            count: record_count,
            maximum: limits.max_records,
        });
    }
    let count = u16::try_from(record_count).map_err(|_| BanwordSectionError::CountOverflow {
        count: record_count,
    })?;
    let record_size = u16::try_from(BANWORD_RECORD_SIZE).map_err(|_| {
        BanwordSectionError::RecordSizeOverflow {
            size: BANWORD_RECORD_SIZE,
        }
    })?;
    let data_len = record_count.checked_mul(BANWORD_RECORD_SIZE).ok_or(
        BanwordSectionError::DataSizeOverflow {
            count: record_count,
        },
    )?;
    if data_len > limits.max_data_bytes {
        return Err(BanwordSectionError::DataTooLarge {
            length: data_len,
            maximum: limits.max_data_bytes,
        });
    }

    let mut data = Vec::new();
    data.try_reserve_exact(data_len)
        .map_err(|_| BanwordSectionError::AllocationFailed {
            requested: data_len,
        })?;
    for row in rows.iter().flatten() {
        let mut record = [0_u8; BANWORD_WIRE_SIZE];
        // `strlcpy` stops at the first NUL, then zero-fills the destination.
        // Do not copy bytes after an embedded NUL even if the source vector
        // contains them.
        let source_len = row.iter().position(|&byte| byte == 0).unwrap_or(row.len());
        let copy_len = source_len.min(BANWORD_MAX_LEN);
        record[..copy_len].copy_from_slice(&row[..copy_len]);
        data.extend_from_slice(&record);
    }

    debug_assert_eq!(data.len(), data_len);
    Ok(BootSection {
        kind: BootSectionKind::Banword,
        record_size,
        count,
        data,
    })
}

/// A reusable, bounded banword loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BanwordLoader {
    limits: BanwordSectionLimits,
}

impl Default for BanwordLoader {
    fn default() -> Self {
        Self::new(BanwordSectionLimits::default())
    }
}

impl BanwordLoader {
    /// Construct a loader with explicit limits.
    #[must_use]
    pub const fn new(limits: BanwordSectionLimits) -> Self {
        Self { limits }
    }

    /// Return the configured limits.
    #[must_use]
    pub const fn limits(&self) -> BanwordSectionLimits {
        self.limits
    }

    /// Obtain and build rows through the injected source.
    ///
    /// # Errors
    ///
    /// Returns [`BanwordLoadError::Source`] for a source failure and
    /// [`BanwordLoadError::Section`] for a limit or encoding failure. A source
    /// failure is never represented as an empty section.
    pub fn load_section<S>(&self, source: &S) -> Result<BootSection, BanwordLoadError<S::Error>>
    where
        S: BanwordRowSource,
    {
        let rows = source
            .query_rows(&BanwordQuery)
            .map_err(BanwordLoadError::Source)?;
        build_banword_section(&rows, self.limits).map_err(BanwordLoadError::Section)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_boot::decode_banword_section;

    fn rows(values: &[Option<&[u8]>]) -> Vec<Option<Vec<u8>>> {
        values
            .iter()
            .map(|value| value.map(<[u8]>::to_vec))
            .collect()
    }

    #[test]
    fn exposes_the_exact_source_query() {
        assert_eq!(BanwordQuery.as_str(), "SELECT word FROM banword");
    }

    #[test]
    fn skips_nulls_preserves_order_and_zero_fills_destination_tail() {
        let source_values = rows(&[None, Some(b"first"), None, Some(b"a\0b"), Some(&[b'x'; 30])]);
        let section =
            build_banword_section(&source_values, BanwordSectionLimits::default()).unwrap();
        let decoded = decode_banword_section(&section).unwrap();

        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].as_str(), "first");
        assert_eq!(
            decoded[1].bytes,
            [b'a', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        let mut truncated = [b'x'; BANWORD_RECORD_SIZE];
        truncated[BANWORD_RECORD_SIZE - 1] = 0;
        assert_eq!(decoded[2].bytes, truncated);
    }

    #[test]
    fn preserves_raw_non_utf8_empty_and_boundary_length_values() {
        let source_values = rows(&[
            Some(&[0xff, 0xfe]),
            Some(b""),
            Some(&[b'a'; 24]),
            Some(&[b'b'; 25]),
        ]);
        let section =
            build_banword_section(&source_values, BanwordSectionLimits::default()).unwrap();
        let decoded = decode_banword_section(&section).unwrap();

        assert_eq!(decoded[0].bytes[0..2], [0xff, 0xfe]);
        assert_eq!(decoded[0].bytes[2..], [0_u8; BANWORD_RECORD_SIZE - 2]);
        assert_eq!(decoded[1].bytes, [0_u8; BANWORD_RECORD_SIZE]);
        assert_eq!(decoded[2].bytes, {
            let mut expected = [b'a'; BANWORD_RECORD_SIZE];
            expected[BANWORD_RECORD_SIZE - 1] = 0;
            expected
        });
        assert_eq!(decoded[3].bytes, {
            let mut expected = [b'b'; BANWORD_RECORD_SIZE];
            expected[BANWORD_RECORD_SIZE - 1] = 0;
            expected
        });
    }

    #[test]
    fn source_failure_is_distinct_from_an_empty_table() {
        let loader = BanwordLoader::default();
        let source = |_query: &BanwordQuery| -> Result<Vec<Option<Vec<u8>>>, &'static str> {
            Err("database unavailable")
        };
        assert_eq!(
            loader.load_section(&source),
            Err(BanwordLoadError::Source("database unavailable"))
        );
    }

    #[test]
    fn null_rows_do_not_consume_record_capacity_but_source_cap_is_still_checked() {
        let values = rows(&[None, Some(b"ok"), None]);
        let limits = BanwordSectionLimits::with_limits(3, 1, BANWORD_RECORD_SIZE);
        let section = build_banword_section(&values, limits).unwrap();
        assert_eq!(section.count, 1);

        let source_too_many = rows(&[None, None, None]);
        assert_eq!(
            build_banword_section(
                &source_too_many,
                BanwordSectionLimits::with_limits(2, 1, 25)
            ),
            Err(BanwordSectionError::TooManySourceRows {
                count: 3,
                maximum: 2
            })
        );
    }

    #[test]
    fn record_and_data_limits_are_checked_before_encoding() {
        let values = rows(&[Some(b"a"), Some(b"b"), Some(b"c")]);
        assert_eq!(
            build_banword_section(&values, BanwordSectionLimits::with_limits(3, 2, 100)),
            Err(BanwordSectionError::TooManyRecords {
                count: 3,
                maximum: 2
            })
        );
        assert_eq!(
            build_banword_section(&values, BanwordSectionLimits::with_limits(3, 3, 49)),
            Err(BanwordSectionError::DataTooLarge {
                length: 75,
                maximum: 49
            })
        );
    }

    #[test]
    fn loader_passes_only_the_fixed_query_to_the_source() {
        let calls = std::cell::Cell::new(0_u8);
        let source = |query: &BanwordQuery| {
            calls.set(calls.get() + 1);
            assert_eq!(query.as_str(), BANWORD_QUERY);
            Ok::<_, &'static str>(rows(&[Some(b"ok")]))
        };
        let section = BanwordLoader::default().load_section(&source).unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(section.kind, BootSectionKind::Banword);
        assert_eq!(section.record_size, 25);
        assert_eq!(section.count, 1);
    }
}
