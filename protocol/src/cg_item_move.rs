//! Explicit codec for the fixed legacy `TPacketCGItemMove` record.
//!
//! `server/server/game/packet.h` defines `HEADER_CG_ITEM_MOVE = 13` and a
//! packed record containing one header byte, two `TItemPos` values, and a
//! `WORD count`, for a complete nine-byte packet. `TItemPos` is itself packed
//! at `server/server/common/length.h:956-1057` as `BYTE window_type` followed
//! by `WORD cell`, so each position is three bytes. `packet_info.cpp:121`
//! registers the exact size.
//!
//! This module only validates and preserves the wire record. It does not
//! interpret the window type, bound a cell, resolve an item, or apply the
//! legacy stack, split, merge, lock, exchange, or custom-inventory rules.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::HEADER_CG_ITEM_MOVE;
use crate::cg_wire::ClientFrame;

/// Packed size of one `TItemPos`, including its `BYTE window_type`.
pub const CG_ITEM_POS_SIZE: usize = 3;

/// Complete packed wire size, including the one-byte header.
pub const CG_ITEM_MOVE_WIRE_SIZE: usize = 9;

/// Fixed client-to-game payload size, excluding the one-byte header.
pub const CG_ITEM_MOVE_PAYLOAD_SIZE: usize = 8;

/// One packed legacy `TItemPos`.
///
/// Both fields are preserved verbatim. `window_type` is a byte whose meaning
/// is selected by the server build's feature flags, and `cell` is an opaque
/// cell word. Neither is range-checked here. This type carries both the
/// source and the destination position of a move record, so its field docs
/// are deliberately position-neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgItemPos {
    /// Opaque `window_type` byte.
    pub window_type: u8,
    /// Opaque `cell` word.
    pub cell: u16,
}

impl CgItemPos {
    /// Construct a position without interpreting either field.
    ///
    /// Both the window byte and the cell word are preserved exactly as
    /// supplied. This type carries the source and the destination position
    /// of a move record, and the source position of an item-use record, so
    /// its field documentation is deliberately position-neutral.
    #[must_use]
    pub const fn new(window_type: u8, cell: u16) -> Self {
        Self { window_type, cell }
    }

    /// Append the exact three packed bytes to `out`.
    ///
    /// The buffer must already have room for [`CG_ITEM_POS_SIZE`] bytes.
    /// The output is exactly `[window_type][cell LE]`.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        out.push(self.window_type);
        out.extend_from_slice(&self.cell.to_le_bytes());
    }

    /// Read one packed position from the start of `data`.
    ///
    /// `data` must hold at least [`CG_ITEM_POS_SIZE`] bytes. Callers are
    /// responsible for that exact-length check.
    pub fn decode_at(data: &[u8]) -> Self {
        Self::new(data[0], u16::from_le_bytes([data[1], data[2]]))
    }
}

/// One fixed client-to-game inventory-move record.
///
/// `count` is an opaque `WORD`. Zero is a meaningful legacy value that requests
/// moving an entire stack, so this boundary does not require a positive count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgItemMove {
    /// Opaque source position.
    pub from: CgItemPos,
    /// Opaque destination position.
    pub to: CgItemPos,
    /// Opaque source `count` word.
    pub count: u16,
}

impl CgItemMove {
    /// Construct a record without interpreting any source field.
    #[must_use]
    pub const fn new(from: CgItemPos, to: CgItemPos, count: u16) -> Self {
        Self { from, to, count }
    }

    /// Encode the exact nine-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut record = Vec::with_capacity(CG_ITEM_MOVE_WIRE_SIZE);
        record.push(HEADER_CG_ITEM_MOVE.value());
        self.from.encode_into(&mut record);
        self.to.encode_into(&mut record);
        record.extend_from_slice(&self.count.to_le_bytes());
        record
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        let mut payload = [0u8; CG_ITEM_MOVE_PAYLOAD_SIZE];
        payload[0] = self.from.window_type;
        payload[1..3].copy_from_slice(&self.from.cell.to_le_bytes());
        payload[3] = self.to.window_type;
        payload[4..6].copy_from_slice(&self.to.cell.to_le_bytes());
        payload[6..8].copy_from_slice(&self.count.to_le_bytes());
        ClientFrame::new(HEADER_CG_ITEM_MOVE.value(), payload)
    }

    /// Decode one exact complete inventory-move record.
    ///
    /// The exact length is checked before any field is read and before the
    /// header is validated.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemMoveError::Truncated`] for fewer than nine bytes,
    /// [`CgItemMoveError::LengthMismatch`] for more than nine bytes, and
    /// [`CgItemMoveError::InvalidHeader`] for another header.
    pub fn decode(data: &[u8]) -> Result<Self, CgItemMoveError> {
        check_exact(data)?;
        decode_parts(
            data[0],
            CgItemPos::decode_at(&data[1..4]),
            CgItemPos::decode_at(&data[4..7]),
            u16::from_le_bytes([data[7], data[8]]),
        )
    }

    /// Decode a [`ClientFrame`] whose payload excludes the header byte.
    ///
    /// # Errors
    ///
    /// Returns a length error when the payload is not exactly eight bytes, or
    /// [`CgItemMoveError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgItemMoveError> {
        match frame.payload.len().cmp(&CG_ITEM_MOVE_PAYLOAD_SIZE) {
            std::cmp::Ordering::Less => {
                let available = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
                Err(CgItemMoveError::Truncated {
                    needed: CG_ITEM_MOVE_WIRE_SIZE,
                    available,
                })
            }
            std::cmp::Ordering::Greater => {
                let actual = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
                Err(CgItemMoveError::LengthMismatch {
                    expected: CG_ITEM_MOVE_WIRE_SIZE,
                    actual,
                })
            }
            std::cmp::Ordering::Equal => {
                let bytes = frame.payload.as_slice();
                decode_parts(
                    frame.header,
                    CgItemPos::decode_at(&bytes[0..3]),
                    CgItemPos::decode_at(&bytes[3..6]),
                    u16::from_le_bytes([bytes[6], bytes[7]]),
                )
            }
        }
    }
}

fn decode_parts(
    header: u8,
    from: CgItemPos,
    to: CgItemPos,
    count: u16,
) -> Result<CgItemMove, CgItemMoveError> {
    if header != HEADER_CG_ITEM_MOVE.value() {
        return Err(CgItemMoveError::InvalidHeader { actual: header });
    }
    Ok(CgItemMove::new(from, to, count))
}

/// A malformed fixed inventory-move record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgItemMoveError {
    /// The complete record was shorter than its packed wire size.
    Truncated {
        /// Required complete wire size, including the header.
        needed: usize,
        /// Bytes supplied by the caller.
        available: usize,
    },
    /// The complete record had bytes beyond its packed wire size.
    LengthMismatch {
        /// Exact complete wire size, including the header.
        expected: usize,
        /// Bytes supplied by the caller.
        actual: usize,
    },
    /// The record used a header other than `13`.
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for CgItemMoveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "item-move record is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "item-move record has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => {
                write!(formatter, "expected item-move header 13, got {actual}")
            }
        }
    }
}

impl Error for CgItemMoveError {}

fn check_exact(data: &[u8]) -> Result<(), CgItemMoveError> {
    match data.len().cmp(&CG_ITEM_MOVE_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(CgItemMoveError::Truncated {
            needed: CG_ITEM_MOVE_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(CgItemMoveError::LengthMismatch {
            expected: CG_ITEM_MOVE_WIRE_SIZE,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_account::CgEnterGame;
    use crate::cg_shoot::CgShoot;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};

    #[test]
    fn inventory_resolves_the_fixed_nine_byte_size() {
        assert_eq!(HEADER_CG_ITEM_MOVE.value(), 13);
        assert_eq!(CG_ITEM_POS_SIZE, 3);
        assert_eq!(CG_ITEM_MOVE_WIRE_SIZE, 9);
        assert_eq!(CG_ITEM_MOVE_PAYLOAD_SIZE, 8);
        assert_eq!(
            resolve_client_frame_size(13).unwrap(),
            ClientFrameSize::Fixed(9)
        );
    }

    #[test]
    fn golden_record_has_exact_source_offsets() {
        // header 13; from.window_type 1, from.cell 2..4; to.window_type 4,
        // to.cell 5..7; count 7..9. Both words are little-endian.
        let packet = CgItemMove::new(
            CgItemPos::new(0x01, 0x0203),
            CgItemPos::new(0x04, 0x0506),
            0x0708,
        );
        assert_eq!(
            packet.encode(),
            vec![0x0d, 0x01, 0x03, 0x02, 0x04, 0x06, 0x05, 0x08, 0x07]
        );
        assert_eq!(CgItemMove::decode(&packet.encode()).unwrap(), packet);
        let frame = packet.to_frame();
        assert_eq!(frame.header, 0x0d);
        assert_eq!(frame.payload.len(), 8);
        assert_eq!(
            frame.payload,
            vec![0x01, 0x03, 0x02, 0x04, 0x06, 0x05, 0x08, 0x07]
        );
        assert_eq!(CgItemMove::decode_frame(&frame).unwrap(), packet);
    }

    #[test]
    fn every_field_value_round_trips_without_inventory_policy() {
        // Zero count is the legacy "move the whole stack" request, the
        // window byte is a feature-flag-dependent enum, and 0xffff is a
        // normal opaque client cell sentinel, so none of them is rejected
        // here.
        let positions = [
            CgItemPos::new(0, 0),
            CgItemPos::new(1, 0),
            CgItemPos::new(0xff, 0),
            CgItemPos::new(0x80, u16::MAX),
            CgItemPos::new(u8::MAX, u16::MAX - 1),
        ];
        for from in positions {
            for to in positions {
                for count in [0u16, 1, 0x7fff, 0x8000, u16::MAX - 1, u16::MAX] {
                    let packet = CgItemMove::new(from, to, count);
                    assert_eq!(packet.encode().len(), CG_ITEM_MOVE_WIRE_SIZE);
                    assert_eq!(CgItemMove::decode(&packet.encode()).unwrap(), packet);
                    assert_eq!(
                        CgItemMove::decode_frame(&packet.to_frame()).unwrap(),
                        packet
                    );
                }
            }
        }
    }

    #[test]
    fn identical_source_and_destination_is_not_rejected() {
        // The legacy handler has no explicit Cell == DestCell check. The wire
        // boundary must not invent one.
        let same = CgItemPos::new(1, 5);
        let packet = CgItemMove::new(same, same, 3);
        assert_eq!(CgItemMove::decode(&packet.encode()).unwrap(), packet);
        assert_eq!(
            CgItemMove::decode_frame(&packet.to_frame()).unwrap(),
            packet
        );
    }

    #[test]
    fn every_raw_short_and_long_length_is_rejected_before_the_header() {
        let complete = CgItemMove::new(CgItemPos::new(1, 2), CgItemPos::new(3, 4), 5).encode();
        for len in 0..CG_ITEM_MOVE_WIRE_SIZE {
            assert_eq!(
                CgItemMove::decode(&complete[..len]),
                Err(CgItemMoveError::Truncated {
                    needed: 9,
                    available: len,
                }),
                "raw length {len} must be truncated"
            );
        }
        for extra in [1usize, 2, 3, 64, 255] {
            let mut long = complete.clone();
            long.extend(std::iter::repeat(0u8).take(extra));
            assert_eq!(
                CgItemMove::decode(&long),
                Err(CgItemMoveError::LengthMismatch {
                    expected: 9,
                    actual: 9 + extra,
                })
            );
        }
        for header in 0u8..=u8::MAX {
            let mut record = complete.clone();
            record[0] = header;
            let expected = if header == 13 {
                Ok(CgItemMove::new(
                    CgItemPos::new(1, 2),
                    CgItemPos::new(3, 4),
                    5,
                ))
            } else {
                Err(CgItemMoveError::InvalidHeader { actual: header })
            };
            assert_eq!(CgItemMove::decode(&record), expected, "header {header}");
        }
        assert_eq!(
            CgItemMove::decode(&[0x0e, 0, 0, 0, 0, 0, 0, 0]),
            Err(CgItemMoveError::Truncated {
                needed: 9,
                available: 8,
            })
        );
    }

    #[test]
    fn every_framed_payload_length_and_header_is_checked() {
        let packet = CgItemMove::new(CgItemPos::new(1, 0x0203), CgItemPos::new(4, 0x0506), 0x0708);
        assert_eq!(
            CgItemMove::decode_frame(&packet.to_frame()).unwrap(),
            packet
        );
        for len in 0..CG_ITEM_MOVE_PAYLOAD_SIZE {
            for header in [0x0du8, 0x0e, 0x00, 0xff] {
                let frame = ClientFrame::new(header, vec![0xabu8; len]);
                let expected = if len < CG_ITEM_MOVE_PAYLOAD_SIZE {
                    CgItemMoveError::Truncated {
                        needed: 9,
                        available: len + 1,
                    }
                } else {
                    CgItemMoveError::InvalidHeader { actual: header }
                };
                assert_eq!(
                    CgItemMove::decode_frame(&frame),
                    Err(expected),
                    "payload {len} header {header}"
                );
            }
        }
        for extra in [1usize, 2, 3, 64, 255] {
            let mut payload = packet.to_frame().payload;
            payload.extend(std::iter::repeat(0u8).take(extra));
            assert_eq!(
                CgItemMove::decode_frame(&ClientFrame::new(0x0d, payload)),
                Err(CgItemMoveError::LengthMismatch {
                    expected: 9,
                    actual: 9 + extra,
                })
            );
        }
        for header in 0u8..=u8::MAX {
            if header == 13 {
                continue;
            }
            assert_eq!(
                CgItemMove::decode_frame(&ClientFrame::new(header, packet.to_frame().payload)),
                Err(CgItemMoveError::InvalidHeader { actual: header }),
                "framed header {header}"
            );
        }
    }

    #[test]
    fn fragmented_and_coalesced_streaming_uses_the_shared_decoder() {
        let raw = CgItemMove::new(CgItemPos::default(), CgItemPos::default(), 0).encode();
        let mut decoder = ClientFrameDecoder::new();
        decoder.feed(&raw[..1]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&raw[1..4]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&raw[4..8]).unwrap();
        assert_eq!(decoder.buffered_len(), 8);
        assert!(decoder.try_decode().unwrap().is_none());

        let mut coalesced = raw[8..].to_vec();
        coalesced.extend_from_slice(
            &CgItemMove::new(
                CgItemPos::new(u8::MAX, u16::MAX),
                CgItemPos::new(0x80, 0x8000),
                u16::MAX,
            )
            .encode(),
        );
        coalesced.extend_from_slice(&CgShoot::new(0x07).encode());
        coalesced.push(CgEnterGame::new().encode()[0]);
        decoder.feed(&coalesced).unwrap();

        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgItemMove::decode_frame(&frame).unwrap(),
            CgItemMove::new(CgItemPos::default(), CgItemPos::default(), 0)
        );
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgItemMove::decode_frame(&frame).unwrap(),
            CgItemMove::new(
                CgItemPos::new(u8::MAX, u16::MAX),
                CgItemPos::new(0x80, 0x8000),
                u16::MAX
            )
        );
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgShoot::decode_frame(&frame).unwrap(), CgShoot::new(0x07));
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgEnterGame::decode_frame(&frame).unwrap(), CgEnterGame);
        assert!(decoder.is_empty());
    }
}
