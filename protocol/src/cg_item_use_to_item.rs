//! Explicit codec for the fixed legacy `TPacketCGItemUseToItem` record.
//!
//! `server/server/game/packet.h` defines `HEADER_CG_ITEM_USE_TO_ITEM = 60`
//! and a packed record holding one header byte and two `TItemPos` positions.
//! `TItemPos` is itself packed, so it is exactly three bytes and this record
//! is exactly seven bytes.
//!
//! Both positions reuse the already source-verified
//! [`CgItemPos`]. Sharing that type keeps the
//! packed three-byte position identical to the item-move and item-use records
//! by construction instead of duplicating an offset table.
//!
//! # Scope
//!
//! This is a transport-free record boundary. It preserves two raw window
//! bytes and two raw little-endian cell words without interpreting any of
//! them. The legacy server entry point `CHARACTER::UseItem` performs the
//! inventory, target, timing, and window policy, and none of that belongs
//! here.
//!
//! # Legacy facts kept out of the codec
//!
//! * This is the only inbound record that passes a real destination position
//!   into `CHARACTER::UseItem`. The single-position item-use record always
//!   passes the compile-time constant `NPOS` instead, so the destination-gated
//!   item branches are reachable from this header and from no other.
//!   `TPacketCGItemMove` also carries a second position, but it reaches
//!   `MoveItem`, not `UseItem`.
//! * Each window byte is a `BYTE` field that carries an `EWindows` value whose
//!   numbering depends on the build's feature flags, so it can only be
//!   preserved, never interpreted.
//! * A cell value of `0xffff` is a normal opaque client value and must
//!   round-trip.
//! * The legacy handler never compares the source position with the target
//!   position, so an equal pair must not be rejected here.

use crate::cg_inventory::HEADER_CG_ITEM_USE_TO_ITEM;
use crate::cg_item_move::CgItemPos;
use crate::cg_wire::ClientFrame;
use std::error::Error;
use std::fmt;

/// The exact wire size of one packed legacy `TItemPos`.
pub use crate::cg_item_move::CG_ITEM_POS_SIZE;

/// The exact payload size of a framed item-use-to-item request.
pub const CG_ITEM_USE_TO_ITEM_PAYLOAD_SIZE: usize = 6;

/// The exact wire size of a complete item-use-to-item request.
pub const CG_ITEM_USE_TO_ITEM_WIRE_SIZE: usize = 7;

/// One fixed legacy `TPacketCGItemUseToItem` request.
///
/// `header` is not stored: the codec accepts only header 60 and always writes
/// it, so a second copy could only create a state that cannot be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgItemUseToItem {
    /// The source item position.
    pub from: CgItemPos,
    /// The target position supplied by this record.
    pub to: CgItemPos,
}

impl CgItemUseToItem {
    /// Build a request for the given source and target positions.
    #[must_use]
    pub const fn new(from: CgItemPos, to: CgItemPos) -> Self {
        Self { from, to }
    }

    /// Encode this request as the exact seven wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_ITEM_USE_TO_ITEM_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode this request into an existing buffer.
    ///
    /// The buffer must already have room for
    /// [`CG_ITEM_USE_TO_ITEM_WIRE_SIZE`] bytes. The output is exactly
    /// `[0x3c][from.window_type][from.cell LE][to.window_type][to.cell LE]`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_CG_ITEM_USE_TO_ITEM.value());
        self.from.encode_into(out);
        self.to.encode_into(out);
    }

    /// Project this request as a `ClientFrame` with an exact six-byte
    /// payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_ITEM_USE_TO_ITEM_PAYLOAD_SIZE);
        self.from.encode_into(&mut payload);
        self.to.encode_into(&mut payload);
        ClientFrame::new(HEADER_CG_ITEM_USE_TO_ITEM.value(), payload)
    }

    /// Decode one exact seven-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemUseToItemError::Truncated`] for a short input,
    /// [`CgItemUseToItemError::LengthMismatch`] for a long input, and
    /// [`CgItemUseToItemError::InvalidHeader`] when the header is not 60. The
    /// length is checked before the header, and the header is checked before
    /// any field is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgItemUseToItemError> {
        check_exact(bytes.len())?;
        check_header(bytes[0])?;
        Ok(Self {
            from: CgItemPos::decode_at(&bytes[1..4]),
            to: CgItemPos::decode_at(&bytes[4..7]),
        })
    }

    /// Decode one framed request.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemUseToItemError::Truncated`] for a payload shorter than
    /// six bytes, [`CgItemUseToItemError::LengthMismatch`] for a longer
    /// payload, and [`CgItemUseToItemError::InvalidHeader`] when the frame
    /// header is not 60. The payload length is checked before the header, and
    /// the header is checked before any payload byte is read.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgItemUseToItemError> {
        let len = frame.payload.len();
        match len.cmp(&CG_ITEM_USE_TO_ITEM_PAYLOAD_SIZE) {
            core::cmp::Ordering::Less => {
                return Err(CgItemUseToItemError::Truncated {
                    needed: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
                    available: len.saturating_add(1),
                });
            }
            core::cmp::Ordering::Greater => {
                return Err(CgItemUseToItemError::LengthMismatch {
                    expected: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
                    actual: len.saturating_add(1),
                });
            }
            core::cmp::Ordering::Equal => {}
        }
        check_header(frame.header)?;
        Ok(Self {
            from: CgItemPos::decode_at(&frame.payload[0..3]),
            to: CgItemPos::decode_at(&frame.payload[3..6]),
        })
    }
}

impl fmt::Display for CgItemUseToItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CgItemUseToItem(from={{ window_type: {}, cell: {} }}, to={{ window_type: {}, cell: {} }})",
            self.from.window_type, self.from.cell, self.to.window_type, self.to.cell
        )
    }
}

/// Every way decoding a fixed item-use-to-item request can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgItemUseToItemError {
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
    /// The leading byte was not [`HEADER_CG_ITEM_USE_TO_ITEM`].
    InvalidHeader {
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl fmt::Display for CgItemUseToItemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated item-use-to-item record: need {needed} bytes, have {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "item-use-to-item record must be {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { actual } => {
                write!(
                    f,
                    "invalid item-use-to-item header {actual:#04x}, expected 0x3c"
                )
            }
        }
    }
}

impl Error for CgItemUseToItemError {}

fn check_exact(len: usize) -> Result<(), CgItemUseToItemError> {
    match len.cmp(&CG_ITEM_USE_TO_ITEM_WIRE_SIZE) {
        core::cmp::Ordering::Less => Err(CgItemUseToItemError::Truncated {
            needed: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
            available: len,
        }),
        core::cmp::Ordering::Greater => Err(CgItemUseToItemError::LengthMismatch {
            expected: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
            actual: len,
        }),
        core::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_header(actual: u8) -> Result<(), CgItemUseToItemError> {
    if actual == HEADER_CG_ITEM_USE_TO_ITEM.value() {
        Ok(())
    } else {
        Err(CgItemUseToItemError::InvalidHeader { actual })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_item_move::{CgItemMove, CG_ITEM_MOVE_WIRE_SIZE};
    use crate::cg_shoot::CgShoot;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};

    const HEADER: u8 = 0x3c;

    fn sample() -> CgItemUseToItem {
        CgItemUseToItem::new(
            CgItemPos {
                window_type: 0x02,
                cell: 0x0304,
            },
            CgItemPos {
                window_type: 0x28,
                cell: 0x0605,
            },
        )
    }

    #[test]
    fn inventory_resolves_the_fixed_seven_byte_size() {
        assert_eq!(CG_ITEM_USE_TO_ITEM_WIRE_SIZE, 7);
        assert_eq!(CG_ITEM_USE_TO_ITEM_PAYLOAD_SIZE, 6);
        assert_eq!(2 * CG_ITEM_POS_SIZE, CG_ITEM_USE_TO_ITEM_PAYLOAD_SIZE);
        assert_eq!(
            CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
            CG_ITEM_USE_TO_ITEM_PAYLOAD_SIZE + 1
        );
        assert_eq!(
            resolve_client_frame_size(HEADER_CG_ITEM_USE_TO_ITEM.value()),
            Ok(ClientFrameSize::Fixed(CG_ITEM_USE_TO_ITEM_WIRE_SIZE))
        );
    }

    #[test]
    fn golden_record_has_exact_source_offsets() {
        // packet.h:605-610 packs one header byte and two TItemPos values.
        assert_eq!(
            sample().encode(),
            vec![0x3c, 0x02, 0x04, 0x03, 0x28, 0x05, 0x06]
        );
        let frame = sample().to_frame();
        assert_eq!(frame.header, HEADER);
        assert_eq!(frame.payload, vec![0x02, 0x04, 0x03, 0x28, 0x05, 0x06]);
        assert_eq!(
            frame.encode().expect("frame encodes"),
            vec![0x3c, 0x02, 0x04, 0x03, 0x28, 0x05, 0x06]
        );
        assert_eq!(CgItemUseToItem::decode(&sample().encode()), Ok(sample()));
        assert_eq!(CgItemUseToItem::decode_frame(&frame), Ok(sample()));
    }

    #[test]
    fn shares_the_item_move_position_layout() {
        // The two records must place the same packed 3-byte positions at
        // identical offsets, so the offsets cannot drift independently.
        let item_move = CgItemMove::new(
            CgItemPos {
                window_type: 0x02,
                cell: 0x0304,
            },
            CgItemPos {
                window_type: 0x28,
                cell: 0x0605,
            },
            7,
        );
        let move_bytes = item_move.encode();
        let use_bytes = sample().encode();
        assert_eq!(move_bytes.len(), CG_ITEM_MOVE_WIRE_SIZE);
        assert_eq!(&move_bytes[1..4], &use_bytes[1..4]);
        assert_eq!(&move_bytes[4..7], &use_bytes[4..7]);
        // The two records share the six position bytes; the item-move record
        // then adds a two-byte count that this record does not carry.
        assert_eq!(&move_bytes[1..7], &use_bytes[1..7]);
        assert_eq!(&move_bytes[7..], &[7, 0]);
        assert_eq!(&use_bytes[1..], &move_bytes[1..7]);
        assert_eq!(use_bytes[0], 0x3c);
        assert_ne!(move_bytes[0], use_bytes[0]);
        // The same type decodes both records.
        assert_eq!(CgItemPos::decode_at(&use_bytes[1..4]).window_type, 0x02);
        assert_eq!(CgItemPos::decode_at(&use_bytes[4..7]).cell, 0x0605);
    }

    #[test]
    fn every_field_value_round_trips_without_inventory_policy() {
        // The window byte is a BYTE that carries a build-flag-dependent
        // EWindows value, and 0xffff is a normal opaque cell value, so both
        // must survive untouched. Equal source and target positions are never
        // compared by the legacy handler and must not be rejected.
        let cases = [
            CgItemUseToItem::new(
                CgItemPos {
                    window_type: 0,
                    cell: 0,
                },
                CgItemPos {
                    window_type: 0,
                    cell: 0,
                },
            ),
            CgItemUseToItem::new(
                CgItemPos {
                    window_type: 0xff,
                    cell: 0xffff,
                },
                CgItemPos {
                    window_type: 0xff,
                    cell: 0xffff,
                },
            ),
            CgItemUseToItem::new(
                CgItemPos {
                    window_type: 0x22,
                    cell: 0xffff,
                },
                CgItemPos {
                    window_type: 0x2a,
                    cell: 0,
                },
            ),
            // Identical source and target is a wire-legal value.
            CgItemUseToItem::new(
                CgItemPos {
                    window_type: 0x0b,
                    cell: 0x1234,
                },
                CgItemPos {
                    window_type: 0x0b,
                    cell: 0x1234,
                },
            ),
        ];
        for case in cases {
            let bytes = case.encode();
            assert_eq!(bytes.len(), CG_ITEM_USE_TO_ITEM_WIRE_SIZE);
            assert_eq!(bytes[0], HEADER);
            assert_eq!(CgItemUseToItem::decode(&bytes), Ok(case));
            let frame = case.to_frame();
            assert_eq!(frame.payload.len(), CG_ITEM_USE_TO_ITEM_PAYLOAD_SIZE);
            assert_eq!(CgItemUseToItem::decode_frame(&frame), Ok(case));
            assert_eq!(frame.encode().expect("frame encodes"), bytes);
        }
        // Every window byte and cell word round-trips in each slot.
        for window in 0u8..=u8::MAX {
            for cell in [0u16, 1, 0xfffe, 0xffff] {
                let pos = CgItemPos {
                    window_type: window,
                    cell,
                };
                let rec = CgItemUseToItem::new(pos, pos);
                assert_eq!(CgItemUseToItem::decode(&rec.encode()), Ok(rec));
            }
        }
    }

    #[test]
    fn every_raw_short_and_long_length_is_rejected_before_the_header() {
        let good = sample().encode();
        for len in 0..CG_ITEM_USE_TO_ITEM_WIRE_SIZE {
            let bytes = vec![0xff; len];
            assert_eq!(
                CgItemUseToItem::decode(&bytes),
                Err(CgItemUseToItemError::Truncated {
                    needed: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
                    available: len,
                }),
                "raw length {len} must be Truncated"
            );
        }
        for extra in 1..8usize {
            let mut bytes = good.clone();
            bytes.extend(std::iter::repeat(0xff).take(extra));
            assert_eq!(
                CgItemUseToItem::decode(&bytes),
                Err(CgItemUseToItemError::LengthMismatch {
                    expected: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
                    actual: CG_ITEM_USE_TO_ITEM_WIRE_SIZE + extra,
                }),
                "raw length {extra} over must be LengthMismatch"
            );
        }
        // A wrong header at a correct length is the only InvalidHeader case.
        for header in 0u8..=u8::MAX {
            let mut bytes = good.clone();
            bytes[0] = header;
            let expected = if header == HEADER {
                Ok(sample())
            } else {
                Err(CgItemUseToItemError::InvalidHeader { actual: header })
            };
            assert_eq!(CgItemUseToItem::decode(&bytes), expected);
        }
        // A decoder that checked the header before the length would pass every
        // assertion above, because the wrong-length cases all carry a correct
        // header and the wrong-header cases all carry a correct length. Pair
        // each wrong length with each wrong header and require the length
        // error every time.
        for len in 0..CG_ITEM_USE_TO_ITEM_WIRE_SIZE {
            for header in [0x00u8, 0x01, 0x3b, 0x3d, 0xff] {
                let bytes = vec![header; len];
                assert_eq!(
                    CgItemUseToItem::decode(&bytes),
                    Err(CgItemUseToItemError::Truncated {
                        needed: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
                        available: len,
                    }),
                    "raw length {len} with header {header:#04x} must be Truncated"
                );
            }
        }
        for extra in 1..8usize {
            for header in [0x00u8, 0x01, 0x3b, 0x3d, 0xff] {
                let mut bytes = good.clone();
                bytes[0] = header;
                bytes.extend(std::iter::repeat(0xff).take(extra));
                assert_eq!(
                    CgItemUseToItem::decode(&bytes),
                    Err(CgItemUseToItemError::LengthMismatch {
                        expected: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
                        actual: CG_ITEM_USE_TO_ITEM_WIRE_SIZE + extra,
                    }),
                    "raw length {extra} over with header {header:#04x} must be LengthMismatch"
                );
            }
        }
    }

    #[test]
    fn every_framed_payload_length_and_header_is_checked() {
        let good = sample().encode();
        for len in 0..CG_ITEM_USE_TO_ITEM_PAYLOAD_SIZE {
            let frame = ClientFrame::new(HEADER, vec![0xff; len]);
            assert_eq!(
                CgItemUseToItem::decode_frame(&frame),
                Err(CgItemUseToItemError::Truncated {
                    needed: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
                    available: len + 1,
                }),
                "framed payload {len} must be Truncated"
            );
        }
        for extra in 1..8usize {
            let mut payload = good[1..].to_vec();
            payload.extend(std::iter::repeat(0xff).take(extra));
            let frame = ClientFrame::new(HEADER, payload);
            assert_eq!(
                CgItemUseToItem::decode_frame(&frame),
                Err(CgItemUseToItemError::LengthMismatch {
                    expected: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
                    actual: CG_ITEM_USE_TO_ITEM_WIRE_SIZE + extra,
                }),
                "framed payload {extra} over must be LengthMismatch"
            );
        }
        for header in 0u8..=u8::MAX {
            let frame = ClientFrame::new(header, &good[1..]);
            let expected = if header == HEADER {
                Ok(sample())
            } else {
                Err(CgItemUseToItemError::InvalidHeader { actual: header })
            };
            assert_eq!(CgItemUseToItem::decode_frame(&frame), expected);
        }
        // Same precedence requirement on the framed path.
        for len in 0..CG_ITEM_USE_TO_ITEM_PAYLOAD_SIZE {
            for header in [0x00u8, 0x01, 0x3b, 0x3d, 0xff] {
                let frame = ClientFrame::new(header, vec![0xff; len]);
                assert_eq!(
                    CgItemUseToItem::decode_frame(&frame),
                    Err(CgItemUseToItemError::Truncated {
                        needed: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
                        available: len + 1,
                    }),
                    "framed payload {len} with header {header:#04x} must be Truncated"
                );
            }
        }
        for extra in 1..8usize {
            for header in [0x00u8, 0x01, 0x3b, 0x3d, 0xff] {
                let mut payload = good[1..].to_vec();
                payload.extend(std::iter::repeat(0xff).take(extra));
                let frame = ClientFrame::new(header, payload);
                assert_eq!(
                    CgItemUseToItem::decode_frame(&frame),
                    Err(CgItemUseToItemError::LengthMismatch {
                        expected: CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
                        actual: CG_ITEM_USE_TO_ITEM_WIRE_SIZE + extra,
                    }),
                    "framed payload {extra} over with header {header:#04x} must be LengthMismatch"
                );
            }
        }
    }

    #[test]
    fn fragmented_and_coalesced_streaming_uses_the_shared_decoder() {
        let mut wire = Vec::new();
        let first = CgItemUseToItem::new(
            CgItemPos {
                window_type: 0x01,
                cell: 0x0002,
            },
            CgItemPos {
                window_type: 0x01,
                cell: 0x0003,
            },
        );
        let second = CgItemUseToItem::new(
            CgItemPos {
                window_type: 0xff,
                cell: 0xfffe,
            },
            CgItemPos {
                window_type: 0x00,
                cell: 0x0000,
            },
        );
        let shoot = CgShoot::new(0x07);
        wire.extend(first.encode());
        wire.extend(shoot.encode());
        wire.extend(second.encode());

        // Byte-at-a-time delivery must not lose or duplicate a frame.
        let mut decoder = ClientFrameDecoder::new();
        let mut framed = Vec::new();
        for byte in &wire {
            decoder.feed(&[*byte]).expect("single byte feed");
            while let Some(frame) = decoder.try_decode().expect("try_decode") {
                framed.push(frame);
            }
        }
        assert!(decoder.is_empty());
        assert_eq!(framed.len(), 3);
        assert_eq!(CgItemUseToItem::decode_frame(&framed[0]), Ok(first));
        assert_eq!(framed[1].header, 0x36);
        assert_eq!(
            CgShoot::decode_frame(&framed[1]).expect("shoot decodes"),
            shoot
        );
        assert_eq!(CgItemUseToItem::decode_frame(&framed[2]), Ok(second));

        // Whole-buffer delivery must produce the same three frames.
        let mut bulk = ClientFrameDecoder::new();
        bulk.feed(&wire).expect("coalesced feed");
        let mut bulk_frames = Vec::new();
        while let Some(frame) = bulk.try_decode().expect("try_decode") {
            bulk_frames.push(frame);
        }
        assert!(bulk.is_empty());
        assert_eq!(bulk_frames.len(), 3);
        assert_eq!(CgItemUseToItem::decode_frame(&bulk_frames[0]), Ok(first));
        assert_eq!(bulk_frames[1].header, 0x36);
        assert_eq!(
            CgShoot::decode_frame(&bulk_frames[1]).expect("shoot decodes"),
            shoot
        );
        assert_eq!(CgItemUseToItem::decode_frame(&bulk_frames[2]), Ok(second));
    }
}
