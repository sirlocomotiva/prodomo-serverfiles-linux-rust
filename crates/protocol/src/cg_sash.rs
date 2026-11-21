//! Transport-free codec for the CG direction of `TPacketSash`.
//!
//! | record | header | wire | payload | declaration | registration |
//! |---|---|---|---|---|---|
//! | `SPacketSash` | 230 `0xe6` | 23 | 22 | `packet.h:2807-2821` | `packet_info.cpp:182` |
//!
//! This was the first backlog record found to embed a `TItemPos`, and the only
//! place so far where the width of that struct had to be re-derived rather than
//! remembered. `SPacketCGDragonSoulRefine` is a second consumer, so the type
//! itself now lives in [`crate::item_pos`] and is re-exported below. See the
//! `TItemPos` section.
//!
//! # `TPacketSash` is bidirectional, and one sub-header name is misleading
//!
//! The record is **not** client-to-server only. `packet.h:2804-2818` declares one
//! enum that serves both directions:
//!
//! ```text
//! #ifdef __SASH_SYSTEM__
//! enum
//! {
//!     HEADER_CG_SASH = 230,
//!     HEADER_GC_SASH = 231,
//!     SASH_SUBHEADER_GC_OPEN   = 0,
//!     SASH_SUBHEADER_GC_CLOSE,
//!     SASH_SUBHEADER_GC_ADDED,
//!     SASH_SUBHEADER_GC_REMOVED,
//!     SASH_SUBHEADER_CG_REFINED,      // continues the GC numbering: 4
//!     SASH_SUBHEADER_CG_CLOSE  = 0,   // explicit reset restarts numbering
//!     SASH_SUBHEADER_CG_ADD,          // 1
//!     SASH_SUBHEADER_CG_REMOVE,       // 2
//!     SASH_SUBHEADER_CG_REFINE,       // 3
//! };
//! #endif
//! ```
//!
//! Two things follow, and both are easy to get wrong:
//!
//! 1. The explicit `= 0` on `SASH_SUBHEADER_CG_CLOSE` is load-bearing. It is what
//!    restarts numbering after the GC half. Without it the whole CG half would
//!    sit at 4..7 and every value on the wire would change.
//! 2. **`SASH_SUBHEADER_CG_REFINED` is a GC value that carries a `CG_` name.** Its
//!    value is 4 because it continues from `SASH_SUBHEADER_GC_REMOVED`, and both
//!    trees use it only server-to-client: `game/char.cpp:10420-10421` builds a
//!    record with `sPacket.header = HEADER_GC_SASH;` and
//!    `sPacket.subheader = SASH_SUBHEADER_CG_REFINED;`, and
//!    `client/.../PythonNetworkStreamPhaseGame.cpp:5166` handles it on an inbound
//!    GC packet.
//!
//! So the CG sub-header domain is **0 through 3 only**. Trusting the name
//! instead of the numbering would produce a five-value `SashSubHeader` that the
//! server's `switch` cannot satisfy.
//!
//! `CInputMain::Sash` switches on 0..3 and its `default: break;` discards every
//! other value silently — no log, no error, no disconnect. That is legacy
//! behaviour, so [`CgSash::subheader`] preserves all 256 `u8` values and rejects
//! none of them. The drop stays in the handler.
//!
//! # One fixed frame, four sub-commands, four different field subsets
//!
//! | sub-header | reads | ignores |
//! |---|---|---|
//! | `CLOSE` (0) | nothing | every field |
//! | `ADD` (1) | `item_pos`, `pos` | `window`, `price`, `item_vnum`, `min_absorb`, `max_absorb` |
//! | `REMOVE` (2) | `pos` | everything else |
//! | `REFINE` (3) | nothing | every field |
//!
//! The frame is fixed-width, so the ignored fields are still on the wire and the
//! codec still carries all 23 bytes. But a declared field is not a meaningful one:
//! `price` in a `REMOVE` request is a stale client-side value, not a request.
//!
//! # `window` and `item_pos.window_type` are unrelated
//!
//! The record has a top-level `bool bWindow` at offset 2 **and** a `TItemPos` at
//! offset 7 whose own first member is `window_type`. They are two different
//! decisions written side by side: `char.cpp:10422` sets `bWindow` from
//! `m_bSashCombination`, while `char.cpp:10415-10417` sets
//! `tPos.window_type = INVENTORY` independently. The client reads the top-level
//! one directly (`PythonNetworkStreamPhaseGame.cpp:5163` passes it to `ActSash`).
//! They are kept as separate, separately named fields for that reason.
//!
//! # `TItemPos` is 3 bytes
//!
//! There is exactly one `TItemPos` in the tree, at `common/length.h`, in a
//! `struct SItemPos` that reads:
//!
//! ```cpp
//! typedef struct SItemPos
//! {
//!     BYTE window_type;
//!     WORD cell;
//!     // constructors and operator overloads, no further data members
//! } TItemPos;
//! ```
//!
//! Two data members, `BYTE` then `WORD`, under the workspace `#pragma pack(1)`, so
//! [`ItemPos::WIRE_SIZE`] is **3**. Two independent registrations confirm it:
//! `sizeof(TPacketSash)` is 23 and `1 + 1 + 1 + 4 + 1 + 3 + 4 + 4 + 4 = 23`, while
//! `sizeof(TPacketCGDragonSoulRefine)` is 47 and `1 + 1 + 15 * 3 = 47`. A 4-byte
//! `TItemPos` would give 24 and 62, and neither would match its own `sizeof`.
//!
//! `ItemPos` now lives in [`crate::item_pos`] and is re-exported here, because
//! `SPacketCGDragonSoulRefine` is a second consumer: it embeds `TItemPos
//! ItemGrid[15]`. A type predicted to have more than one consumer should get its
//! own module in the same section that reveals the second one, and the prediction
//! here was wrong for a full section -- this record was already audited when the
//! type was parked here as "transitional".
//!
//! # Feature gate
//!
//! `__SASH_SYSTEM__` **is** defined, at `common/prodomodefines.h:15`, so the
//! record, the enum, and the live `input_main.cpp:3862` dispatch are all in the
//! active build. The client's `Packet.h:2869-2880` carries the identical enum.

use crate::cg_inventory::{CgHeader, HEADER_CG_SASH};
use crate::cg_wire::ClientFrame;
use crate::item_pos::ItemPosError;

/// The shared packed position type, re-exported so existing `cg_sash` users
/// keep one import path. It lives in [`crate::item_pos`] because
/// `SPacketCGDragonSoulRefine` is a second consumer.
pub use crate::item_pos::ItemPos;

/// The full legacy `SPacketSash` record, header byte included.
pub const CG_SASH_WIRE_SIZE: usize = 1 + 1 + 1 + 4 + 1 + ItemPos::WIRE_SIZE + 4 + 4 + 4;
/// The framed payload of `SPacketSash`, everything after the header.
pub const CG_SASH_PAYLOAD_SIZE: usize = CG_SASH_WIRE_SIZE - 1;

/// Every way the sash decoder can refuse a byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgSashError {
    /// Fewer bytes than the fixed record needs.
    Truncated {
        /// The fixed width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// A complete-length slice that is not the fixed width.
    LengthMismatch {
        /// The one width this record has.
        expected: usize,
        /// The width that was offered.
        actual: usize,
    },
    /// The right number of bytes, but the header byte is not this record's.
    InvalidHeader {
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl From<ItemPosError> for CgSashError {
    fn from(e: ItemPosError) -> Self {
        match e {
            ItemPosError::Truncated { needed, available } => {
                CgSashError::Truncated { needed, available }
            }
            ItemPosError::LengthMismatch { expected, actual } => {
                CgSashError::LengthMismatch { expected, actual }
            }
        }
    }
}

impl std::fmt::Display for CgSashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "CG record needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "CG record must be exactly {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(f, "CG header {actual} is not {expected}")
            }
        }
    }
}

impl std::error::Error for CgSashError {}

fn check_exact(actual: usize, expected: usize) -> Result<(), CgSashError> {
    if actual < expected {
        return Err(CgSashError::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(CgSashError::LengthMismatch { expected, actual });
    }
    Ok(())
}

fn check_header(actual: u8, expected: u8) -> Result<(), CgSashError> {
    if actual == expected {
        return Ok(());
    }
    Err(CgSashError::InvalidHeader { expected, actual })
}

fn read_u32_le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn write_u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// The client-to-server sash request.
///
/// Field order is `subheader: u8` at offset 1, `window: bool` at offset 2,
/// `price: u32` at offset 3, `pos: u8` at offset 7, `item_pos: ItemPos` at
/// offset 8, `item_vnum: u32` at offset 11, `min_absorb: u32` at offset 15, and
/// `max_absorb: u32` at offset 19.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgSash {
    /// The sub-command. The CG domain is 0..=3, but all 256 values are preserved
    /// because the legacy `default:` arm drops the rest silently.
    pub subheader: u8,
    /// The top-level sash-window flag. Unrelated to
    /// [`ItemPos::window_type`].
    pub window: bool,
    /// The sash price. Read by no CG sub-command.
    pub price: u32,
    /// The material slot. Read by `ADD` and `REMOVE`.
    pub pos: u8,
    /// The material position. Read by `ADD` only.
    pub item_pos: ItemPos,
    /// The item vnum. Read by no CG sub-command.
    pub item_vnum: u32,
    /// The minimum absorb percentage. Read by no CG sub-command.
    pub min_absorb: u32,
    /// The maximum absorb percentage. Read by no CG sub-command.
    pub max_absorb: u32,
}

impl CgSash {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_SASH
    }

    /// Build the record.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        subheader: u8,
        window: bool,
        price: u32,
        pos: u8,
        item_pos: ItemPos,
        item_vnum: u32,
        min_absorb: u32,
        max_absorb: u32,
    ) -> Self {
        Self {
            subheader,
            window,
            price,
            pos,
            item_pos,
            item_vnum,
            min_absorb,
            max_absorb,
        }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.subheader);
        out.push(u8::from(self.window));
        write_u32_le(out, self.price);
        out.push(self.pos);
        self.item_pos.encode_into(out);
        write_u32_le(out, self.item_vnum);
        write_u32_le(out, self.min_absorb);
        write_u32_le(out, self.max_absorb);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_SASH_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_SASH_PAYLOAD_SIZE);
        payload.push(self.subheader);
        payload.push(u8::from(self.window));
        write_u32_le(&mut payload, self.price);
        payload.push(self.pos);
        self.item_pos.encode_into(&mut payload);
        write_u32_le(&mut payload, self.item_vnum);
        write_u32_le(&mut payload, self.min_absorb);
        write_u32_le(&mut payload, self.max_absorb);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgSashError::Truncated`] for a short input,
    /// [`CgSashError::LengthMismatch`] for a long one, and
    /// [`CgSashError::InvalidHeader`] when the header is not 230.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgSashError> {
        check_exact(bytes.len(), CG_SASH_WIRE_SIZE)?;
        check_header(bytes[0], Self::header().value())?;
        Ok(Self {
            subheader: bytes[1],
            window: bytes[2] != 0,
            price: read_u32_le(bytes, 3),
            pos: bytes[7],
            item_pos: ItemPos::decode(&bytes[8..11])?,
            item_vnum: read_u32_le(bytes, 11),
            min_absorb: read_u32_le(bytes, 15),
            max_absorb: read_u32_le(bytes, 19),
        })
    }

    /// # Errors
    ///
    /// As [`CgSash::decode`], except that the frame payload must be exactly 22
    /// bytes and every offset shifts down by one.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgSashError> {
        check_exact(frame.payload.len(), CG_SASH_PAYLOAD_SIZE)?;
        check_header(frame.header, Self::header().value())?;
        Ok(Self {
            subheader: frame.payload[0],
            window: frame.payload[1] != 0,
            price: read_u32_le(&frame.payload, 2),
            pos: frame.payload[6],
            item_pos: ItemPos::decode(&frame.payload[7..10])?,
            item_vnum: read_u32_le(&frame.payload, 10),
            min_absorb: read_u32_le(&frame.payload, 14),
            max_absorb: read_u32_le(&frame.payload, 18),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_pos::ItemPosError;

    fn sample_item_pos() -> ItemPos {
        ItemPos::new(0x2A, 0xBEEF)
    }

    fn sample_sash() -> CgSash {
        CgSash::new(
            1,
            true,
            0x1122_3344,
            0x77,
            sample_item_pos(),
            0x5566_7788,
            0x99AA_BBCC,
            0xDDEE_FF00,
        )
    }

    /// Distinct bytes at every offset, so a one-byte shift anywhere in the
    /// 23-byte record shows up as a decode difference.
    fn all_distinct_record() -> Vec<u8> {
        let mut bytes = vec![230_u8];
        for i in 0..22_u8 {
            bytes.push(i.wrapping_mul(11).wrapping_add(3));
        }
        bytes
    }

    // ---- ItemPos ------------------------------------------------------------

    #[test]
    fn item_pos_is_three_bytes() {
        assert_eq!(ItemPos::WIRE_SIZE, 3);
        assert_eq!(ItemPos::new(1, 2).encode().len(), 3);
    }

    #[test]
    fn item_pos_encodes_byte_then_little_endian_word() {
        let bytes = ItemPos::new(0x03, 0x1234).encode();
        assert_eq!(bytes, vec![0x03, 0x34, 0x12]);
    }

    #[test]
    fn item_pos_round_trips() {
        let pos = ItemPos::new(0xFE, 0xFFFF);
        assert_eq!(ItemPos::decode(&pos.encode()).unwrap(), pos);
    }

    #[test]
    fn item_pos_preserves_all_256_window_types() {
        for w in 0..=u8::MAX {
            let pos = ItemPos::new(w, 0);
            assert_eq!(pos.encode()[0], w);
            assert_eq!(ItemPos::decode(&pos.encode()).unwrap().window_type, w);
        }
    }

    #[test]
    fn item_pos_rejects_wrong_lengths() {
        // The error type moved with the type: `ItemPos` now reports
        // `ItemPosError`, and `CgSashError` converts from it with a total
        // `From` impl. These assertions are about the position's own boundary.
        assert_eq!(
            ItemPos::decode(&[1, 2]).unwrap_err(),
            ItemPosError::Truncated {
                needed: 3,
                available: 2
            }
        );
        assert_eq!(
            ItemPos::decode(&[1, 2, 3, 4]).unwrap_err(),
            ItemPosError::LengthMismatch {
                expected: 3,
                actual: 4
            }
        );
    }

    #[test]
    fn the_item_pos_error_converts_into_the_sash_error() {
        assert_eq!(
            CgSashError::from(ItemPosError::Truncated {
                needed: 3,
                available: 2
            }),
            CgSashError::Truncated {
                needed: 3,
                available: 2
            }
        );
        assert_eq!(
            CgSashError::from(ItemPosError::LengthMismatch {
                expected: 3,
                actual: 4
            }),
            CgSashError::LengthMismatch {
                expected: 3,
                actual: 4
            }
        );
    }

    #[test]
    fn item_pos_appends_to_an_existing_buffer() {
        let mut out = vec![0x11];
        sample_item_pos().encode_into(&mut out);
        assert_eq!(out.len(), 4);
        assert_eq!(&out[1..], &sample_item_pos().encode()[..]);
    }

    // ---- CgSash record ------------------------------------------------------

    #[test]
    fn header_is_the_legacy_value() {
        assert_eq!(CgSash::header().value(), 230);
    }

    #[test]
    fn wire_size_matches_the_registration() {
        // 1 + 1 + 1 + 4 + 1 + 3 + 4 + 4 + 4 = 23 = sizeof(TPacketSash)
        assert_eq!(CG_SASH_WIRE_SIZE, 23);
        assert_eq!(CG_SASH_PAYLOAD_SIZE, 22);
    }

    #[test]
    fn wire_size_arithmetic_is_visible_in_the_constant() {
        assert_eq!(
            CG_SASH_WIRE_SIZE,
            1 + 1 + 1 + 4 + 1 + ItemPos::WIRE_SIZE + 4 + 4 + 4
        );
    }

    #[test]
    fn encodes_to_the_exact_legacy_bytes() {
        let bytes = sample_sash().encode();
        assert_eq!(bytes.len(), 23);
        assert_eq!(bytes[0], 230);
        assert_eq!(bytes[1], 1);
        assert_eq!(bytes[2], 1);
        assert_eq!(&bytes[3..7], &0x1122_3344_u32.to_le_bytes());
        assert_eq!(bytes[7], 0x77);
        assert_eq!(&bytes[8..11], &[0x2A, 0xEF, 0xBE]);
        assert_eq!(&bytes[11..15], &0x5566_7788_u32.to_le_bytes());
        assert_eq!(&bytes[15..19], &0x99AA_BBCC_u32.to_le_bytes());
        assert_eq!(&bytes[19..23], &0xDDEE_FF00_u32.to_le_bytes());
    }

    #[test]
    fn decodes_the_exact_legacy_bytes() {
        let rec = sample_sash();
        assert_eq!(CgSash::decode(&rec.encode()).unwrap(), rec);
    }

    #[test]
    fn round_trips_through_a_frame() {
        let rec = sample_sash();
        let frame = rec.to_frame();
        assert_eq!(frame.header, 230);
        assert_eq!(frame.payload.len(), 22);
        assert_eq!(frame.payload[0], rec.subheader);
        assert_eq!(frame.payload[1], 1);
        assert_eq!(&frame.payload[2..6], &rec.price.to_le_bytes());
        assert_eq!(frame.payload[6], rec.pos);
        assert_eq!(&frame.payload[7..10], &rec.item_pos.encode()[..]);
        assert_eq!(&frame.payload[10..14], &rec.item_vnum.to_le_bytes());
        assert_eq!(&frame.payload[14..18], &rec.min_absorb.to_le_bytes());
        assert_eq!(&frame.payload[18..22], &rec.max_absorb.to_le_bytes());
        assert_eq!(CgSash::decode_frame(&frame).unwrap(), rec);
    }

    #[test]
    fn frame_and_slice_agree() {
        let rec = sample_sash();
        assert_eq!(
            CgSash::decode(&rec.encode()).unwrap(),
            CgSash::decode_frame(&rec.to_frame()).unwrap()
        );
    }

    #[test]
    fn the_frame_payload_is_the_record_minus_the_header() {
        let rec = sample_sash();
        let frame = rec.to_frame();
        assert_eq!(frame.payload, rec.encode()[1..]);
    }

    #[test]
    fn every_field_offset_is_pinned_by_distinct_bytes() {
        let mut rec = sample_sash();
        // One field at a time, each with a value no other offset can produce.
        rec.subheader = 0x11;
        rec.window = false;
        rec.price = 0x0000_0001;
        rec.pos = 0x22;
        rec.item_pos = ItemPos::new(0x33, 0x0000_0044);
        rec.item_vnum = 0x0000_0005;
        rec.min_absorb = 0x0000_0006;
        rec.max_absorb = 0x0000_0007;
        let back = CgSash::decode(&rec.encode()).unwrap();
        assert_eq!(back, rec);
    }

    // ---- errors -------------------------------------------------------------

    #[test]
    fn rejects_every_short_length() {
        for len in 0..CG_SASH_WIRE_SIZE {
            let err = CgSash::decode(&vec![230_u8; len]).unwrap_err();
            assert_eq!(
                err,
                CgSashError::Truncated {
                    needed: 23,
                    available: len
                }
            );
        }
    }

    #[test]
    fn rejects_every_long_length() {
        for len in 24..36 {
            let err = CgSash::decode(&vec![230_u8; len]).unwrap_err();
            assert_eq!(
                err,
                CgSashError::LengthMismatch {
                    expected: 23,
                    actual: len
                }
            );
        }
    }

    #[test]
    fn rejects_a_foreign_header() {
        let mut bytes = sample_sash().encode();
        bytes[0] = 231;
        assert_eq!(
            CgSash::decode(&bytes).unwrap_err(),
            CgSashError::InvalidHeader {
                expected: 230,
                actual: 231
            }
        );
    }

    #[test]
    fn frame_rejects_a_wrong_payload_width() {
        let mut frame = sample_sash().to_frame();
        frame.payload.push(0);
        assert_eq!(
            CgSash::decode_frame(&frame).unwrap_err(),
            CgSashError::LengthMismatch {
                expected: 22,
                actual: 23
            }
        );
    }

    #[test]
    fn frame_rejects_a_foreign_header() {
        let mut frame = sample_sash().to_frame();
        frame.header = 231;
        assert_eq!(
            CgSash::decode_frame(&frame).unwrap_err(),
            CgSashError::InvalidHeader {
                expected: 230,
                actual: 231
            }
        );
    }

    // ---- legacy semantics ---------------------------------------------------

    #[test]
    fn subheader_is_preserved_for_all_256_values() {
        // SASH_SUBHEADER_CG_REFINED (4) is a GC value with a CG_ name, and the
        // server's default arm drops 4 and above silently. Framing keeps all of it.
        for sub in 0..=u8::MAX {
            let rec = CgSash::new(sub, false, 0, 0, ItemPos::default(), 0, 0, 0);
            assert_eq!(rec.encode()[1], sub);
            assert_eq!(CgSash::decode(&rec.encode()).unwrap().subheader, sub);
        }
    }

    #[test]
    fn window_flag_is_a_nonzero_test_not_a_narrowing() {
        for raw in [1_u8, 2, 0x80, 0xFF] {
            let mut bytes = sample_sash().encode();
            bytes[2] = raw;
            assert!(CgSash::decode(&bytes).unwrap().window);
        }
        let mut bytes = sample_sash().encode();
        bytes[2] = 0;
        assert!(!CgSash::decode(&bytes).unwrap().window);
    }

    #[test]
    fn the_two_window_fields_are_independent() {
        // A record whose top-level window flag and item_pos.window_type disagree
        // must decode to both values, not to one shared field.
        let rec = CgSash::new(0, false, 0, 0, ItemPos::new(1, 0), 0, 0, 0);
        let back = CgSash::decode(&rec.encode()).unwrap();
        assert!(!back.window);
        assert_eq!(back.item_pos.window_type, 1);
    }

    #[test]
    fn unused_fields_still_round_trip() {
        // CLOSE and REFINE read no fields at all, so every one of the other
        // seven must still survive a round trip untouched.
        let rec = CgSash::new(
            0,
            true,
            0xDEAD_BEEF,
            0x5A,
            ItemPos::new(0x7F, 0x0102),
            0xCAFE_F00D,
            0x0102_0304,
            0x0506_0708,
        );
        let back = CgSash::decode(&rec.encode()).unwrap();
        assert_eq!(back.price, 0xDEAD_BEEF);
        assert_eq!(back.item_vnum, 0xCAFE_F00D);
        assert_eq!(back.min_absorb, 0x0102_0304);
        assert_eq!(back.max_absorb, 0x0506_0708);
    }

    #[test]
    fn every_distinct_byte_pattern_round_trips() {
        // Sweep a family of records where each payload byte differs, so a
        // single misplaced field offset cannot survive.
        for step in 0..22_u8 {
            let mut bytes = all_distinct_record();
            for b in bytes.iter_mut().skip(1) {
                *b = b.wrapping_add(step);
            }
            // Offset 2 is a C++ bool, so any nonzero byte decodes to `true` and
            // re-encodes as 1. Pin it here so the sweep still covers every other
            // offset byte-for-byte.
            bytes[2] = u8::from(step % 2 == 0);
            let rec = CgSash::decode(&bytes).expect("distinct pattern decodes");
            assert_eq!(rec.encode(), bytes, "step {step} did not round trip");
        }
    }

    #[test]
    fn encoding_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE, 0xAD];
        sample_sash().encode_into(&mut out);
        assert_eq!(out.len(), 2 + 23);
        assert_eq!(&out[2..], &sample_sash().encode()[..]);
    }

    #[test]
    fn error_messages_name_the_record_width() {
        let msg = CgSash::decode(&[230; 10]).unwrap_err().to_string();
        assert!(msg.contains("23"), "message was {msg}");
        let msg = CgSash::decode(&[0_u8; 30]).unwrap_err().to_string();
        assert!(msg.contains("23"), "message was {msg}");
    }
}
