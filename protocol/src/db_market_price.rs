//! Source-fixed premium-market item-price update codec.
//!
//! The legacy database peer sends this unsolicited update inside a
//! `HEADER_DG_PRIVATE_SHOP` frame with handle zero. Its payload is a
//! private-shop DG subheader, a little-endian `u16` row count, and that many
//! packed 16-byte `TMarketItemPrice` records. The record itself is encoded by
//! [`MarketItemPriceRecord`]; this module only defines the count-prefixed
//! payload and the narrow frame boundary around it.
//!
//! This codec does not dispatch DB peers, authenticate a session, query SQL,
//! aggregate prices, mutate a market, or deliver gameplay state.

use std::error::Error;
use std::fmt;

use crate::db_records::{DbRecordError, MarketItemPriceRecord};
use crate::db_wire::DbFrame;

/// `HEADER_DG_PRIVATE_SHOP` in the active legacy build (`0xb8`).
pub const HEADER_DG_PRIVATE_SHOP: u8 = 0xb8;

/// Final zero-based `EPrivateShopDGSubheader` member (`0x23`).
pub const PRIVATE_SHOP_DG_SUBHEADER_MARKET_ITEM_PRICE_DATA_UPDATE: u8 = 0x23;

/// The legacy sender uses the zero peer handle for this unsolicited update.
pub const MARKET_ITEM_PRICE_UPDATE_HANDLE: u32 = 0;

/// Bytes in the subheader and little-endian `u16` count prefix.
pub const MARKET_ITEM_PRICE_UPDATE_PREFIX_SIZE: usize = 3;

/// A validated premium-market price update.
///
/// Records remain in their original order. Duplicate item virtual numbers are
/// valid and are neither sorted nor deduplicated. Signed gold values and all
/// `u32` cheque values are preserved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketItemPriceUpdate {
    /// Price rows in legacy sender order.
    pub records: Vec<MarketItemPriceRecord>,
}

impl MarketItemPriceUpdate {
    /// Construct an update while retaining record order and duplicates.
    #[must_use]
    pub fn new(records: Vec<MarketItemPriceRecord>) -> Self {
        Self { records }
    }

    /// Return the number of price rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Return whether the update has no price rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Encode the payload as subheader, `u16` count, and record bytes.
    ///
    /// The output allocation is fallible. The row count and total payload
    /// length are checked before any output bytes are written.
    ///
    /// # Errors
    ///
    /// Returns [`MarketItemPriceUpdateError::CountTooLarge`] when the vector
    /// has more than [`u16::MAX`] rows, [`MarketItemPriceUpdateError::SizeOverflow`]
    /// if the checked payload-size arithmetic fails, or
    /// [`MarketItemPriceUpdateError::PayloadAllocationFailed`] if the exact
    /// output reservation fails.
    pub fn encode(&self) -> Result<Vec<u8>, MarketItemPriceUpdateError> {
        let count = u16::try_from(self.records.len()).map_err(|_| {
            MarketItemPriceUpdateError::CountTooLarge {
                count: self.records.len(),
                maximum: usize::from(u16::MAX),
            }
        })?;
        let records_len = self
            .records
            .len()
            .checked_mul(MarketItemPriceRecord::WIRE_SIZE)
            .ok_or(MarketItemPriceUpdateError::SizeOverflow {
                count: self.records.len(),
            })?;
        let payload_len = MARKET_ITEM_PRICE_UPDATE_PREFIX_SIZE
            .checked_add(records_len)
            .ok_or(MarketItemPriceUpdateError::SizeOverflow {
                count: self.records.len(),
            })?;

        let mut payload = Vec::new();
        payload.try_reserve_exact(payload_len).map_err(|_| {
            MarketItemPriceUpdateError::PayloadAllocationFailed {
                length: payload_len,
            }
        })?;
        payload.push(PRIVATE_SHOP_DG_SUBHEADER_MARKET_ITEM_PRICE_DATA_UPDATE);
        payload.extend_from_slice(&count.to_le_bytes());
        for record in &self.records {
            let encoded = record.encode();
            debug_assert_eq!(encoded.len(), MarketItemPriceRecord::WIRE_SIZE);
            payload.extend_from_slice(&encoded);
        }
        debug_assert_eq!(payload.len(), payload_len);
        Ok(payload)
    }

    /// Decode one exact count-prefixed market-price update payload.
    ///
    /// # Errors
    ///
    /// Returns [`MarketItemPriceUpdateError::Truncated`] for a short prefix or
    /// record area, [`MarketItemPriceUpdateError::UnexpectedSubheader`] for
    /// another private-shop DG subheader, and
    /// [`MarketItemPriceUpdateError::LengthMismatch`] when the actual length
    /// does not equal `3 + 16 * count`.
    pub fn decode(payload: &[u8]) -> Result<Self, MarketItemPriceUpdateError> {
        if payload.len() < MARKET_ITEM_PRICE_UPDATE_PREFIX_SIZE {
            return Err(MarketItemPriceUpdateError::Truncated {
                field: "market_item_price_update_prefix",
                needed: MARKET_ITEM_PRICE_UPDATE_PREFIX_SIZE,
                available: payload.len(),
            });
        }
        let subheader = payload[0];
        if subheader != PRIVATE_SHOP_DG_SUBHEADER_MARKET_ITEM_PRICE_DATA_UPDATE {
            return Err(MarketItemPriceUpdateError::UnexpectedSubheader {
                expected: PRIVATE_SHOP_DG_SUBHEADER_MARKET_ITEM_PRICE_DATA_UPDATE,
                actual: subheader,
            });
        }

        let count = u16::from_le_bytes([payload[1], payload[2]]);
        let count_usize = usize::from(count);
        let records_len = count_usize
            .checked_mul(MarketItemPriceRecord::WIRE_SIZE)
            .ok_or(MarketItemPriceUpdateError::SizeOverflow { count: count_usize })?;
        let expected_len = MARKET_ITEM_PRICE_UPDATE_PREFIX_SIZE
            .checked_add(records_len)
            .ok_or(MarketItemPriceUpdateError::SizeOverflow { count: count_usize })?;
        if payload.len() < expected_len {
            return Err(MarketItemPriceUpdateError::Truncated {
                field: "market_item_price_update_records",
                needed: expected_len,
                available: payload.len(),
            });
        }
        if payload.len() > expected_len {
            return Err(MarketItemPriceUpdateError::LengthMismatch {
                expected: expected_len,
                actual: payload.len(),
            });
        }

        let mut records = Vec::new();
        records
            .try_reserve_exact(count_usize)
            .map_err(|_| MarketItemPriceUpdateError::RecordVectorAllocationFailed { count })?;
        for (index, bytes) in payload[MARKET_ITEM_PRICE_UPDATE_PREFIX_SIZE..]
            .chunks_exact(MarketItemPriceRecord::WIRE_SIZE)
            .enumerate()
        {
            let record = MarketItemPriceRecord::decode(bytes)
                .map_err(|source| MarketItemPriceUpdateError::RecordDecode { index, source })?;
            records.push(record);
        }
        debug_assert_eq!(records.len(), count_usize);
        Ok(Self { records })
    }

    /// Encode this payload in the legacy zero-handle DB frame.
    ///
    /// The returned [`DbFrame`] retains the existing nine-byte DB peer header
    /// when passed through [`DbFrame::encode`].
    ///
    /// # Errors
    ///
    /// Returns the same payload validation and allocation errors as
    /// [`MarketItemPriceUpdate::encode`].
    pub fn encode_frame(&self) -> Result<DbFrame, MarketItemPriceUpdateError> {
        Ok(DbFrame::new(
            HEADER_DG_PRIVATE_SHOP,
            MARKET_ITEM_PRICE_UPDATE_HANDLE,
            self.encode()?,
        ))
    }

    /// Decode an already-framed market-price update.
    ///
    /// This validates only the exact private-shop header and zero handle. It
    /// does not provide peer authentication or dispatch.
    ///
    /// # Errors
    ///
    /// Returns [`MarketItemPriceUpdateError::UnexpectedHeader`] or
    /// [`MarketItemPriceUpdateError::UnexpectedHandle`] for a wrong frame
    /// boundary, and all payload errors from [`MarketItemPriceUpdate::decode`].
    pub fn decode_frame(frame: &DbFrame) -> Result<Self, MarketItemPriceUpdateError> {
        if frame.header != HEADER_DG_PRIVATE_SHOP {
            return Err(MarketItemPriceUpdateError::UnexpectedHeader {
                expected: HEADER_DG_PRIVATE_SHOP,
                actual: frame.header,
            });
        }
        if frame.handle != MARKET_ITEM_PRICE_UPDATE_HANDLE {
            return Err(MarketItemPriceUpdateError::UnexpectedHandle {
                expected: MARKET_ITEM_PRICE_UPDATE_HANDLE,
                actual: frame.handle,
            });
        }
        Self::decode(&frame.payload)
    }
}

/// A malformed premium-market price update payload or frame boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketItemPriceUpdateError {
    /// The input ended before a required prefix or record area.
    Truncated {
        /// Logical field being read.
        field: &'static str,
        /// Absolute required end offset.
        needed: usize,
        /// Bytes available in the payload.
        available: usize,
    },
    /// The payload contained bytes beyond its declared count-derived length.
    LengthMismatch {
        /// Exact payload length required by the count.
        expected: usize,
        /// Actual payload length.
        actual: usize,
    },
    /// The private-shop DG subheader was not the market-price update.
    UnexpectedSubheader {
        /// Required subheader.
        expected: u8,
        /// Supplied subheader.
        actual: u8,
    },
    /// An encode input has more rows than the wire count can represent.
    CountTooLarge {
        /// Number of supplied rows.
        count: usize,
        /// Largest accepted row count.
        maximum: usize,
    },
    /// Checked payload-size arithmetic could not represent the encoded length.
    SizeOverflow {
        /// Number of rows whose size was checked.
        count: usize,
    },
    /// The encoded output vector could not reserve its exact byte length.
    PayloadAllocationFailed {
        /// Exact payload length that could not be reserved.
        length: usize,
    },
    /// The decoded record vector could not reserve space for the declared rows.
    RecordVectorAllocationFailed {
        /// Number of rows that could not be reserved.
        count: u16,
    },
    /// One fixed-width market-price record could not be decoded.
    RecordDecode {
        /// Zero-based record index.
        index: usize,
        /// Record codec error.
        source: DbRecordError,
    },
    /// The DB frame used a header other than `HEADER_DG_PRIVATE_SHOP`.
    UnexpectedHeader {
        /// Required DB header.
        expected: u8,
        /// Supplied DB header.
        actual: u8,
    },
    /// The DB frame used a handle other than the legacy zero value.
    UnexpectedHandle {
        /// Required DB handle.
        expected: u32,
        /// Supplied DB handle.
        actual: u32,
    },
}

impl fmt::Display for MarketItemPriceUpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                field,
                needed,
                available,
            } => write!(
                formatter,
                "market-item price update {field} is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "market-item price update has {actual} bytes; expected exactly {expected}"
            ),
            Self::UnexpectedSubheader { expected, actual } => write!(
                formatter,
                "unexpected private-shop DG subheader {actual}; expected {expected}"
            ),
            Self::CountTooLarge { count, maximum } => write!(
                formatter,
                "market-item price update count {count} exceeds wire maximum {maximum}"
            ),
            Self::SizeOverflow { count } => write!(
                formatter,
                "market-item price update size overflows usize for {count} records"
            ),
            Self::PayloadAllocationFailed { length } => write!(
                formatter,
                "could not allocate a {length}-byte market-item price update payload"
            ),
            Self::RecordVectorAllocationFailed { count } => write!(
                formatter,
                "could not allocate the market-item price record vector for {count} rows"
            ),
            Self::RecordDecode { index, source } => write!(
                formatter,
                "market-item price record {index} could not be decoded: {source}"
            ),
            Self::UnexpectedHeader { expected, actual } => write!(
                formatter,
                "unexpected DB frame header {actual}; expected {expected}"
            ),
            Self::UnexpectedHandle { expected, actual } => write!(
                formatter,
                "unexpected DB frame handle {actual}; expected {expected}"
            ),
        }
    }
}

impl Error for MarketItemPriceUpdateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RecordDecode { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_wire::DbFrameDecoder;

    fn golden_record() -> MarketItemPriceRecord {
        MarketItemPriceRecord {
            vnum: 0x1122_3344,
            gold: -2,
            cheque: 0xa1b2_c3d4,
        }
    }

    fn golden_payload() -> Vec<u8> {
        vec![
            0x23, 0x01, 0x00, 0x44, 0x33, 0x22, 0x11, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xd4, 0xc3, 0xb2, 0xa1,
        ]
    }

    #[test]
    fn source_constants_match_the_legacy_profile() {
        assert_eq!(HEADER_DG_PRIVATE_SHOP, 0xb8);
        assert_eq!(
            PRIVATE_SHOP_DG_SUBHEADER_MARKET_ITEM_PRICE_DATA_UPDATE,
            0x23
        );
        assert_eq!(MARKET_ITEM_PRICE_UPDATE_HANDLE, 0);
        assert_eq!(MarketItemPriceRecord::WIRE_SIZE, 16);
    }

    #[test]
    fn encodes_the_golden_one_record_payload() {
        let update = MarketItemPriceUpdate::new(vec![golden_record()]);
        assert_eq!(update.encode().unwrap(), golden_payload());
    }

    #[test]
    fn encodes_the_golden_complete_db_frame() {
        let update = MarketItemPriceUpdate::new(vec![golden_record()]);
        let frame = update.encode_frame().unwrap();
        assert_eq!(frame.header, 0xb8);
        assert_eq!(frame.handle, 0);
        let encoded = frame.encode().unwrap();
        assert_eq!(encoded.len(), 28);
        assert_eq!(
            encoded,
            vec![
                0xb8, 0x00, 0x00, 0x00, 0x00, 0x13, 0x00, 0x00, 0x00, 0x23, 0x01, 0x00, 0x44, 0x33,
                0x22, 0x11, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xd4, 0xc3, 0xb2, 0xa1,
            ]
        );
        assert_eq!(MarketItemPriceUpdate::decode_frame(&frame).unwrap(), update);
    }

    #[test]
    fn accepts_an_empty_update_payload_and_frame() {
        let update = MarketItemPriceUpdate::default();
        assert!(update.is_empty());
        assert_eq!(update.encode().unwrap(), vec![0x23, 0x00, 0x00]);

        let frame = update.encode_frame().unwrap();
        assert_eq!(frame.encode().unwrap().len(), 12);
        assert_eq!(frame.encode().unwrap()[5..9], [0x03, 0x00, 0x00, 0x00]);
        assert_eq!(MarketItemPriceUpdate::decode_frame(&frame).unwrap(), update);
    }

    #[test]
    fn preserves_order_duplicates_and_full_domain_values() {
        let records = vec![
            MarketItemPriceRecord {
                vnum: 42,
                gold: i64::MIN,
                cheque: u32::MAX,
            },
            MarketItemPriceRecord {
                vnum: 7,
                gold: i64::MAX,
                cheque: 0,
            },
            MarketItemPriceRecord {
                vnum: 42,
                gold: -1,
                cheque: 17,
            },
        ];
        let update = MarketItemPriceUpdate::new(records.clone());
        let decoded = MarketItemPriceUpdate::decode(&update.encode().unwrap()).unwrap();
        assert_eq!(decoded.records, records);
        assert_eq!(decoded.records[0].vnum, 42);
        assert_eq!(decoded.records[1].vnum, 7);
        assert_eq!(decoded.records[2].vnum, 42);
        assert_eq!(decoded.records[0].gold, i64::MIN);
        assert_eq!(decoded.records[1].gold, i64::MAX);
        assert_eq!(decoded.records[0].cheque, u32::MAX);
    }

    #[test]
    fn rejects_every_short_prefix_length() {
        for available in 0..MARKET_ITEM_PRICE_UPDATE_PREFIX_SIZE {
            assert_eq!(
                MarketItemPriceUpdate::decode(&vec![0x23; available]),
                Err(MarketItemPriceUpdateError::Truncated {
                    field: "market_item_price_update_prefix",
                    needed: MARKET_ITEM_PRICE_UPDATE_PREFIX_SIZE,
                    available,
                })
            );
        }
    }

    #[test]
    fn rejects_a_wrong_subheader() {
        let mut payload = golden_payload();
        payload[0] = 0x22;
        assert_eq!(
            MarketItemPriceUpdate::decode(&payload),
            Err(MarketItemPriceUpdateError::UnexpectedSubheader {
                expected: 0x23,
                actual: 0x22,
            })
        );
    }

    #[test]
    fn rejects_truncated_and_trailing_record_areas() {
        let payload = golden_payload();
        assert_eq!(
            MarketItemPriceUpdate::decode(&payload[..payload.len() - 1]),
            Err(MarketItemPriceUpdateError::Truncated {
                field: "market_item_price_update_records",
                needed: 19,
                available: 18,
            })
        );

        let mut trailing = payload;
        trailing.push(0);
        assert_eq!(
            MarketItemPriceUpdate::decode(&trailing),
            Err(MarketItemPriceUpdateError::LengthMismatch {
                expected: 19,
                actual: 20,
            })
        );
    }

    #[test]
    fn rejects_both_count_and_length_mismatch_directions() {
        let payload = golden_payload();
        let mut count_too_high = payload.clone();
        count_too_high[1..3].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            MarketItemPriceUpdate::decode(&count_too_high),
            Err(MarketItemPriceUpdateError::Truncated {
                field: "market_item_price_update_records",
                needed: 35,
                available: 19,
            })
        );

        let mut count_too_low = payload;
        count_too_low[1..3].copy_from_slice(&0_u16.to_le_bytes());
        assert_eq!(
            MarketItemPriceUpdate::decode(&count_too_low),
            Err(MarketItemPriceUpdateError::LengthMismatch {
                expected: 3,
                actual: 19,
            })
        );
    }

    #[test]
    fn rejects_wrong_frame_header_and_handle() {
        let update = MarketItemPriceUpdate::new(vec![golden_record()]);
        let frame = update.encode_frame().unwrap();

        let mut wrong_header = frame.clone();
        wrong_header.header = 0xb7;
        assert_eq!(
            MarketItemPriceUpdate::decode_frame(&wrong_header),
            Err(MarketItemPriceUpdateError::UnexpectedHeader {
                expected: 0xb8,
                actual: 0xb7,
            })
        );

        let mut wrong_handle = frame;
        wrong_handle.handle = 1;
        assert_eq!(
            MarketItemPriceUpdate::decode_frame(&wrong_handle),
            Err(MarketItemPriceUpdateError::UnexpectedHandle {
                expected: 0,
                actual: 1,
            })
        );
    }

    #[test]
    fn rejects_an_encode_count_above_u16_max() {
        let update = MarketItemPriceUpdate::new(vec![
            MarketItemPriceRecord::default();
            usize::from(u16::MAX) + 1
        ]);
        assert_eq!(
            update.encode(),
            Err(MarketItemPriceUpdateError::CountTooLarge {
                count: 65_536,
                maximum: 65_535,
            })
        );
    }

    #[test]
    fn generic_decoder_handles_byte_fragments_and_a_coalesced_following_frame() {
        let update = MarketItemPriceUpdate::new(vec![golden_record()]);
        let first = update.encode_frame().unwrap().encode().unwrap();
        let following = DbFrame::new(0x2a, 9, vec![1, 2, 3]).encode().unwrap();
        let mut stream = first.clone();
        stream.extend_from_slice(&following);

        let mut decoder = DbFrameDecoder::new();
        for byte in &stream {
            decoder.feed(std::slice::from_ref(byte)).unwrap();
            if decoder.buffered_len() < first.len() {
                assert_eq!(decoder.try_decode().unwrap(), None);
            }
        }

        let first_frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            MarketItemPriceUpdate::decode_frame(&first_frame).unwrap(),
            update
        );
        assert_eq!(
            decoder.try_decode().unwrap(),
            Some(DbFrame::new(0x2a, 9, vec![1, 2, 3]))
        );
        assert_eq!(decoder.try_decode().unwrap(), None);
    }
}
