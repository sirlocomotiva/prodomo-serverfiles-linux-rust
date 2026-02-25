//! Explicit codec for the fixed legacy `TPacketCGItemUse` record.
//!
//! `server/server/game/packet.h` defines `HEADER_CG_ITEM_USE = 11` and a
//! packed record holding one header byte and a single `TItemPos` position.
//! `TItemPos` is itself packed, so it is exactly three bytes, which makes
//! this record exactly four bytes.
//!
//! The position is reused as the already source-verified
//! [`CgItemPos`](crate::cg_item_move::CgItemPos) from the item-move record.
//! Sharing that type keeps the two packed three-byte positions identical by
//! construction instead of duplicating an offset table.
//!
//! # Scope
//!
//! This is a transport-free record boundary. It preserves the raw window
//! byte and the raw little-endian cell word without interpreting either one.
//! The legacy server entry point `CHARACTER::UseItem` performs the
//! gameplay, inventory, timing, and window policy, and none of that belongs
//! here.
//!
//! # Legacy facts kept out of the codec
//!
//! * The record carries only one position, but the legacy server entry point
//!   is declared `UseItem(TItemPos Cell, TItemPos DestCell = NPOS)`. The
//!   default `NPOS` is a compile-time constant, so the second position is
//!   never wire data for this header and is deliberately not represented
//!   here.
//! * A `cell` value of `0xffff` is a normal opaque client value and must
//!   round-trip.
//! * The window byte is an enum whose numeric values depend on the server
//!   build's feature flags, so it can only be preserved, never interpreted.

use crate::cg_inventory::HEADER_CG_ITEM_USE;
use crate::cg_wire::ClientFrame;
use std::fmt;

/// The exact wire size of one packed legacy `TItemPos`.
pub use crate::cg_item_move::CG_ITEM_POS_SIZE;

/// The exact payload size of a framed item-use request.
pub const CG_ITEM_USE_PAYLOAD_SIZE: usize = 3;

/// The exact wire size of a complete item-use request.
pub const CG_ITEM_USE_WIRE_SIZE: usize = 4;

/// One fixed legacy `TPacketCGItemUse` request.
///
/// `header` is not stored: the codec accepts only header 11 and always
/// writes it, so keeping a second copy in memory could only create a state
/// that cannot be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgItemUse {
    /// The one source position named by the record.
    pub cell: crate::cg_item_move::CgItemPos,
}

impl CgItemUse {
    /// Build a request for the given source position.
    #[must_use]
    pub const fn new(cell: crate::cg_item_move::CgItemPos) -> Self {
        Self { cell }
    }

    /// Encode this request as the exact four wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_ITEM_USE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode this request into an existing buffer.
    ///
    /// The buffer must already have room for [`CG_ITEM_USE_WIRE_SIZE`]
    /// bytes. The output is exactly `[0x0b][window_type][cell LE]`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_CG_ITEM_USE.value());
        self.cell.encode_into(out);
    }

    /// Project this request as a `ClientFrame` with an exact three-byte
    /// payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_ITEM_USE_PAYLOAD_SIZE);
        self.cell.encode_into(&mut payload);
        ClientFrame::new(HEADER_CG_ITEM_USE.value(), payload)
    }

    /// Decode one exact four-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemUseError::Truncated`] for a short input,
    /// [`CgItemUseError::LengthMismatch`] for a long input, and
    /// [`CgItemUseError::InvalidHeader`] when the header is not 11. The
    /// length is checked before the header, and the header is checked
    /// before any field is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgItemUseError> {
        check_exact(bytes.len())?;
        check_header(bytes[0])?;
        Ok(Self {
            cell: crate::cg_item_move::CgItemPos::decode_at(&bytes[1..4]),
        })
    }

    /// Decode one framed request.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemUseError::Truncated`] for a payload shorter than
    /// three bytes, [`CgItemUseError::LengthMismatch`] for a longer payload,
    /// and [`CgItemUseError::InvalidHeader`] when the frame header is not
    /// 11. The payload length is checked before the header, and the header
    /// is checked before any payload byte is read.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgItemUseError> {
        let len = frame.payload.len();
        match len.cmp(&CG_ITEM_USE_PAYLOAD_SIZE) {
            core::cmp::Ordering::Less => {
                return Err(CgItemUseError::Truncated {
                    needed: CG_ITEM_USE_WIRE_SIZE,
                    available: len.saturating_add(1),
                })
            }
            core::cmp::Ordering::Greater => {
                return Err(CgItemUseError::LengthMismatch {
                    expected: CG_ITEM_USE_WIRE_SIZE,
                    actual: len.saturating_add(1),
                });
            }
            core::cmp::Ordering::Equal => {}
        }
        check_header(frame.header)?;
        Ok(Self {
            cell: crate::cg_item_move::CgItemPos::decode_at(&frame.payload[0..3]),
        })
    }
}

impl fmt::Display for CgItemUse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CgItemUse(cell={{ window_type: {}, cell: {} }})",
            self.cell.window_type, self.cell.cell
        )
    }
}

/// Every way decoding a fixed item-use request can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgItemUseError {
    /// The input ended before the complete record was available.
    Truncated {
        /// The exact wire size that was required.
        needed: usize,
        /// The number of bytes that were actually available.
        available: usize,
    },
    /// The input carried more bytes than the fixed record can hold.
    LengthMismatch {
        /// The exact wire size the record requires.
        expected: usize,
        /// The number of bytes that were actually supplied.
        actual: usize,
    },
    /// The leading byte was not [`HEADER_CG_ITEM_USE`].
    InvalidHeader {
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl fmt::Display for CgItemUseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated item-use record: need {needed} bytes, have {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(f, "item-use record must be {expected} bytes, got {actual}")
            }
            Self::InvalidHeader { actual } => {
                write!(f, "invalid item-use header {actual:#04x}, expected 0x0b")
            }
        }
    }
}

impl std::error::Error for CgItemUseError {}

fn check_exact(len: usize) -> Result<(), CgItemUseError> {
    match len.cmp(&CG_ITEM_USE_WIRE_SIZE) {
        core::cmp::Ordering::Less => Err(CgItemUseError::Truncated {
            needed: CG_ITEM_USE_WIRE_SIZE,
            available: len,
        }),
        core::cmp::Ordering::Greater => Err(CgItemUseError::LengthMismatch {
            expected: CG_ITEM_USE_WIRE_SIZE,
            actual: len,
        }),
        core::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_header(actual: u8) -> Result<(), CgItemUseError> {
    if actual == HEADER_CG_ITEM_USE.value() {
        Ok(())
    } else {
        Err(CgItemUseError::InvalidHeader { actual })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_item_move::{CgItemMove, CgItemPos};
    use crate::cg_shoot::CgShoot;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};

    #[test]
    fn inventory_resolves_the_fixed_four_byte_size() {
        assert_eq!(HEADER_CG_ITEM_USE.value(), 11);
        assert_eq!(CG_ITEM_POS_SIZE, 3);
        assert_eq!(CG_ITEM_USE_WIRE_SIZE, 4);
        assert_eq!(CG_ITEM_USE_PAYLOAD_SIZE, 3);
        assert_eq!(
            resolve_client_frame_size(11).unwrap(),
            ClientFrameSize::Fixed(4)
        );
    }

    #[test]
    fn golden_record_has_exact_source_offsets() {
        // header 11 at offset 0, then the packed three-byte position:
        // window_type at 1 and the little-endian cell word at 2..4.
        let packet = CgItemUse::new(CgItemPos::new(0x01, 0x0203));
        assert_eq!(packet.encode(), vec![0x0b, 0x01, 0x03, 0x02]);
        assert_eq!(packet.to_frame().payload, vec![0x01, 0x03, 0x02]);
        assert_eq!(packet.to_frame().header, 0x0b);
        assert_eq!(
            packet.to_string(),
            "CgItemUse(cell={ window_type: 1, cell: 515 })"
        );
        assert_eq!(CgItemUse::decode(&packet.encode()).unwrap(), packet);
        assert_eq!(CgItemUse::decode_frame(&packet.to_frame()).unwrap(), packet);
    }

    #[test]
    fn shares_the_item_move_position_layout() {
        // The same CgItemPos type must place the cell word at the same
        // offsets in both records, so a move position and a use position
        // cannot drift apart.
        let pos = CgItemPos::new(0x7f, 0x1234);
        let use_bytes = CgItemUse::new(pos).encode();
        let move_bytes = CgItemMove::new(pos, pos, 0).encode();
        assert_eq!(&use_bytes[1..4], &move_bytes[1..4]);
        assert_eq!(&use_bytes[1..4], &move_bytes[4..7]);
        assert_eq!(CgItemUse::decode(&use_bytes).unwrap().cell, pos);
    }

    #[test]
    fn every_field_value_round_trips_without_inventory_policy() {
        // 0xffff is a normal opaque client cell sentinel and the window
        // byte is a feature-flag-dependent enum, so neither is rejected.
        for window_type in [0u8, 1, 2, 3, 0x7f, 0x80, u8::MAX] {
            for cell in [0u16, 1, 0x7fff, 0x8000, u16::MAX - 1, u16::MAX] {
                let packet = CgItemUse::new(CgItemPos::new(window_type, cell));
                assert_eq!(packet.encode().len(), CG_ITEM_USE_WIRE_SIZE);
                assert_eq!(packet.to_frame().payload.len(), CG_ITEM_USE_PAYLOAD_SIZE);
                assert_eq!(CgItemUse::decode(&packet.encode()).unwrap(), packet);
                assert_eq!(CgItemUse::decode_frame(&packet.to_frame()).unwrap(), packet);
            }
        }
    }

    #[test]
    fn every_raw_short_and_long_length_is_rejected_before_the_header() {
        let complete = CgItemUse::new(CgItemPos::new(1, 2)).encode();
        for len in 0..CG_ITEM_USE_WIRE_SIZE {
            for header in [0x0bu8, 0x0a, 0x00, 0xff] {
                let mut record = complete.clone();
                record[0] = header;
                record.truncate(len);
                let expected = if len < CG_ITEM_USE_WIRE_SIZE {
                    CgItemUseError::Truncated {
                        needed: 4,
                        available: len,
                    }
                } else {
                    CgItemUseError::InvalidHeader { actual: header }
                };
                assert_eq!(
                    CgItemUse::decode(&record),
                    Err(expected),
                    "raw length {len} header {header}"
                );
            }
        }
        for extra in [1usize, 2, 3, 64, 255] {
            let mut long = complete.clone();
            long.extend(std::iter::repeat(0u8).take(extra));
            assert_eq!(
                CgItemUse::decode(&long),
                Err(CgItemUseError::LengthMismatch {
                    expected: 4,
                    actual: 4 + extra,
                })
            );
        }
        for header in 0u8..=u8::MAX {
            let mut record = complete.clone();
            record[0] = header;
            let expected = if header == 11 {
                Ok(CgItemUse::new(CgItemPos::new(1, 2)))
            } else {
                Err(CgItemUseError::InvalidHeader { actual: header })
            };
            assert_eq!(CgItemUse::decode(&record), expected, "header {header}");
        }
    }

    #[test]
    fn every_framed_payload_length_and_header_is_checked() {
        let payload = CgItemUse::new(CgItemPos::new(1, 0x0203)).to_frame().payload;
        for len in 0..CG_ITEM_USE_PAYLOAD_SIZE {
            for header in [0x0bu8, 0x0a, 0x00, 0xff] {
                let frame = ClientFrame::new(header, vec![0xabu8; len]);
                let expected = if len < CG_ITEM_USE_PAYLOAD_SIZE {
                    CgItemUseError::Truncated {
                        needed: 4,
                        available: len + 1,
                    }
                } else {
                    CgItemUseError::InvalidHeader { actual: header }
                };
                assert_eq!(
                    CgItemUse::decode_frame(&frame),
                    Err(expected),
                    "payload {len} header {header}"
                );
            }
        }
        for extra in [1usize, 2, 3, 64, 255] {
            let mut long = payload.clone();
            long.extend(std::iter::repeat(0u8).take(extra));
            assert_eq!(
                CgItemUse::decode_frame(&ClientFrame::new(0x0b, long)),
                Err(CgItemUseError::LengthMismatch {
                    expected: 4,
                    actual: 4 + extra,
                })
            );
        }
        for header in 0u8..=u8::MAX {
            if header == 11 {
                continue;
            }
            assert_eq!(
                CgItemUse::decode_frame(&ClientFrame::new(header, payload.clone())),
                Err(CgItemUseError::InvalidHeader { actual: header }),
                "framed header {header}"
            );
        }
    }

    #[test]
    fn fragmented_and_coalesced_streaming_uses_the_shared_decoder() {
        let raw = CgItemUse::new(CgItemPos::new(u8::MAX, u16::MAX)).encode();
        let mut decoder = ClientFrameDecoder::new();
        for chunk in [&raw[..1], &raw[1..2], &raw[2..3]] {
            decoder.feed(chunk).unwrap();
            assert!(decoder.try_decode().unwrap().is_none());
        }
        assert_eq!(decoder.buffered_len(), 3);

        let mut coalesced = raw[3..].to_vec();
        coalesced.extend_from_slice(&CgItemUse::new(CgItemPos::new(0, 0)).encode());
        coalesced.extend_from_slice(&CgShoot::new(0x07).encode());
        decoder.feed(&coalesced).unwrap();

        for _ in 0..2 {
            let frame = decoder.try_decode().unwrap().unwrap();
            CgItemUse::decode_frame(&frame).unwrap();
        }
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgShoot::decode_frame(&frame).unwrap(), CgShoot::new(0x07));
        assert!(decoder.is_empty());
    }
}
