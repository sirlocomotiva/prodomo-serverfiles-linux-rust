//! `HEADER_GC_DUEL_START` (40): a size-prefixed list of opponent VIDs.
//!
//! `G/packet.h:1766-1770` declares the packed `BYTE header; WORD wSize;`
//! prefix. `G/arena.cpp:293-312,347-359` writes that prefix followed by one
//! `DWORD` opponent VID; `wSize` counts the whole record, not just the list.
//! The empty list is also sent (`G/arena.cpp:443-448`) to clear duel mode.
//! The codec does not impose the arena producer's one-opponent policy.

use std::fmt;

use crate::gc_inventory::HEADER_GC_DUEL_START;

/// Packed `TPacketGCDuelStart` prefix: header and total size word.
pub const GC_DUEL_START_WIRE_SIZE: usize = 3;
/// One packed `DWORD` opponent VID.
pub const GC_DUEL_START_ELEMENT_SIZE: usize = 4;

/// Opponents the Reference client may attack in duel mode.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcDuelStart {
    /// Raw VIDs in wire order; an empty list clears duel mode.
    pub opponent_vids: Vec<u32>,
}

impl GcDuelStart {
    /// The canonical wire header.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_DUEL_START.value()
    }

    /// Builds a record from the opponents in wire order.
    #[must_use]
    pub fn new(opponent_vids: Vec<u32>) -> Self {
        Self { opponent_vids }
    }

    /// Whole-record length, including the three-byte prefix.
    #[must_use]
    pub fn wire_size(&self) -> usize {
        GC_DUEL_START_WIRE_SIZE + self.opponent_vids.len() * GC_DUEL_START_ELEMENT_SIZE
    }

    /// Appends the packed record without changing existing output bytes.
    ///
    /// # Errors
    ///
    /// Returns [`GcDuelStartError::TooManyOpponents`] before writing anything
    /// if the total size does not fit the legacy `WORD`.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), GcDuelStartError> {
        let size =
            u16::try_from(self.wire_size()).map_err(|_| GcDuelStartError::TooManyOpponents {
                count: self.opponent_vids.len(),
            })?;
        out.push(Self::header());
        out.extend_from_slice(&size.to_le_bytes());
        for vid in &self.opponent_vids {
            out.extend_from_slice(&vid.to_le_bytes());
        }
        Ok(())
    }

    /// Decodes the record at the start of `bytes`, ignoring later records.
    ///
    /// # Errors
    ///
    /// Returns [`GcDuelStartError::Truncated`] for an incomplete prefix or
    /// record, [`GcDuelStartError::Header`] for the wrong header, and
    /// [`GcDuelStartError::Size`] unless the size is three plus whole DWORDs.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcDuelStartError> {
        if bytes.len() < GC_DUEL_START_WIRE_SIZE {
            return Err(GcDuelStartError::Truncated {
                needed: GC_DUEL_START_WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(GcDuelStartError::Header {
                expected: Self::header(),
                actual: bytes[0],
            });
        }
        let size = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
        if size < GC_DUEL_START_WIRE_SIZE
            || (size - GC_DUEL_START_WIRE_SIZE) % GC_DUEL_START_ELEMENT_SIZE != 0
        {
            return Err(GcDuelStartError::Size { size });
        }
        if bytes.len() < size {
            return Err(GcDuelStartError::Truncated {
                needed: size,
                actual: bytes.len(),
            });
        }
        let opponent_vids = bytes[GC_DUEL_START_WIRE_SIZE..size]
            .chunks_exact(GC_DUEL_START_ELEMENT_SIZE)
            .map(|vid| u32::from_le_bytes([vid[0], vid[1], vid[2], vid[3]]))
            .collect();
        Ok(Self { opponent_vids })
    }
}

/// Invalid duel-start record or an opponent list too long for its size word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcDuelStartError {
    /// The buffer ends before the prefix or declared record size.
    Truncated {
        /// Required byte count.
        needed: usize,
        /// Available byte count.
        actual: usize,
    },
    /// The first byte belongs to another record.
    Header {
        /// Canonical header.
        expected: u8,
        /// Received header.
        actual: u8,
    },
    /// The size is smaller than the prefix or contains a partial DWORD.
    Size {
        /// Received whole-record size.
        size: usize,
    },
    /// The whole record cannot fit in a `WORD`.
    TooManyOpponents {
        /// Number of opponents refused.
        count: usize,
    },
}

impl fmt::Display for GcDuelStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, actual } => {
                write!(f, "GcDuelStart: need {needed} bytes, buffer held {actual}")
            }
            Self::Header { expected, actual } => write!(
                f,
                "GcDuelStart: header {actual:#04x}, expected {expected:#04x}"
            ),
            Self::Size { size } => write!(f, "GcDuelStart: size {size} is not 3 plus whole DWORDs"),
            Self::TooManyOpponents { count } => {
                write!(f, "GcDuelStart: {count} opponents exceed the WORD size")
            }
        }
    }
}

impl std::error::Error for GcDuelStartError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_opponent_has_golden_distinct_halves() {
        let golden = [40, 7, 0, 0x78, 0x56, 0x34, 0x12];
        let record = GcDuelStart::new(vec![0x1234_5678]);
        let mut encoded = Vec::new();
        record.encode_into(&mut encoded).expect("fits");
        assert_eq!(encoded, golden);
        assert_eq!(GcDuelStart::decode(&golden).expect("reads"), record);
        assert_eq!(record.wire_size(), 7);
    }

    #[test]
    fn multiple_opponents_preserve_golden_order_and_opaque_vids() {
        let golden = [
            40, 15, 0, 0x78, 0x56, 0x34, 0x12, 0xef, 0xcd, 0xab, 0x89, 0, 0, 0, 0,
        ];
        let record = GcDuelStart::new(vec![0x1234_5678, 0x89ab_cdef, 0]);
        let mut encoded = vec![0xcc];
        record.encode_into(&mut encoded).expect("fits");
        assert_eq!(encoded[0], 0xcc);
        assert_eq!(&encoded[1..], golden);
        assert_eq!(GcDuelStart::decode(&golden).expect("reads"), record);
    }

    #[test]
    fn size_word_has_golden_distinct_halves() {
        // 3 + 67 * 4 = 271 = 0x010f, independent of the encoder.
        let mut golden = vec![40, 0x0f, 0x01];
        for _ in 0..67 {
            golden.extend_from_slice(&[0x78, 0x56, 0x34, 0x12]);
        }
        let record = GcDuelStart::new(vec![0x1234_5678; 67]);
        let mut encoded = Vec::new();
        record.encode_into(&mut encoded).expect("fits");
        assert_eq!(encoded, golden);
        assert_eq!(GcDuelStart::decode(&golden).expect("reads"), record);
    }

    #[test]
    fn empty_opponents_clear_duel_mode_with_golden_prefix() {
        let golden = [40, 3, 0];
        let record = GcDuelStart::default();
        let mut encoded = Vec::new();
        record.encode_into(&mut encoded).expect("fits");
        assert_eq!(encoded, golden);
        assert_eq!(GcDuelStart::decode(&golden).expect("reads"), record);
        assert_eq!(record.wire_size(), 3);
    }

    #[test]
    fn decode_leaves_following_records_alone() {
        assert_eq!(
            GcDuelStart::decode(&[40, 7, 0, 0x78, 0x56, 0x34, 0x12, 40, 3, 0])
                .expect("reads only first record"),
            GcDuelStart::new(vec![0x1234_5678])
        );
    }

    #[test]
    fn decode_rejects_every_truncation_and_wrong_header() {
        let golden = [40, 7, 0, 0x78, 0x56, 0x34, 0x12];
        for actual in 0..golden.len() {
            assert_eq!(
                GcDuelStart::decode(&golden[..actual]),
                Err(GcDuelStartError::Truncated {
                    needed: if actual < 3 { 3 } else { 7 },
                    actual,
                })
            );
        }
        assert_eq!(
            GcDuelStart::decode(&[41, 3, 0]),
            Err(GcDuelStartError::Header {
                expected: 40,
                actual: 41,
            })
        );
    }

    #[test]
    fn decode_rejects_undersized_prefix_and_partial_dwords() {
        for size in [0u8, 1, 2, 4, 5, 6, 8, 9, 10] {
            assert_eq!(
                GcDuelStart::decode(&[40, size, 0]),
                Err(GcDuelStartError::Size {
                    size: usize::from(size),
                })
            );
        }
    }

    #[test]
    fn word_size_boundary_is_checked_before_appending() {
        let mut encoded = Vec::new();
        GcDuelStart::new(vec![0x1234_5678; 16_383])
            .encode_into(&mut encoded)
            .expect("largest fitting list");
        assert_eq!(encoded.len(), 65_535);
        assert_eq!(&encoded[..3], &[40, 0xff, 0xff]);
        assert_eq!(
            GcDuelStart::decode(&encoded)
                .expect("reads")
                .opponent_vids
                .len(),
            16_383
        );
        let mut untouched = vec![0xcc];
        assert_eq!(
            GcDuelStart::new(vec![0x1234_5678; 16_384]).encode_into(&mut untouched),
            Err(GcDuelStartError::TooManyOpponents { count: 16_384 })
        );
        assert_eq!(untouched, [0xcc]);
    }
}
