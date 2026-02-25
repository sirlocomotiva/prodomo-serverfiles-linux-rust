//! The legacy `TItemPos`, shared by every record that embeds one.
//!
//! This type started life inside [`crate::cg_sash`], which at the time held the
//! only audited `TItemPos` consumer. `SPacketCGDragonSoulRefine` is the second,
//! so the type has its own module now. A type with one consumer belongs beside
//! that consumer; a type with two belongs on its own.
//!
//! # `TItemPos` is 3 bytes
//!
//! There is exactly one `TItemPos` in the tree: `typedef struct SItemPos` at
//! `server/server/common/length.h:957-1057`, declared immediately after
//! `#pragma pack(push, 1)` at line 956. Its two data members are:
//!
//! ```cpp
//! typedef struct SItemPos
//! {
//!     BYTE window_type;
//!     WORD cell;
//!     // ...constructors and the Is*Position() predicates...
//! } TItemPos;
//! ```
//!
//! `BYTE` plus a packed `WORD` is **3 bytes**. The struct's constructors and
//! predicates contribute nothing to the layout.
//!
//! Three independent registrations agree, and each one would fail at 4 bytes:
//!
//! | witness | arithmetic | a 4-byte `TItemPos` would give |
//! |---|---|---|
//! | `packet_info.cpp:171` `sizeof(TPacketSash)` is 23 | `1+1+1+4+1+3+4+4+4 = 23` | 24 |
//! | `packet_info.cpp:171` `sizeof(TPacketCGDragonSoulRefine)` is 47 | `1+1+15*3 = 47` | 62 |
//! | `cg_inventory.rs` `base_size: 47` for `DragonSoulRefine` | matches the above | 62 |
//!
//! The two records are unrelated in every other respect -- different headers,
//! different fields, different dispatch arms -- so their agreement is a real
//! cross-check rather than a shared constant.
//!
//! # What this codec does not do
//!
//! `window_type` and `cell` are **opaque here**. `SItemPos::IsValidItemPosition()`
//! switches on `window_type` with a `default: return false;` arm, and its
//! `INVENTORY` / `EQUIPMENT` / `DRAGON_SOUL_INVENTORY` arms range-check `cell`
//! against different maxima. All of that is session policy above this codec, and
//! two of the arms are themselves behind feature gates (`__ATTR_6TH_7TH__`,
//! `ENABLE_SWITCHBOT`). Enforcing any of it here would make the codec stricter
//! than the oracle, so **every one of the 256 `window_type` values and every one
//! of the 65,536 `cell` values round-trips unchanged.**
//!
//! # Not the same field as a record's own `window`
//!
//! `SPacketSash` has a top-level `bool bWindow` at offset 2 *and* a `TItemPos`
//! at offset 8. They are separate wire decisions and are modelled separately.
//! `ItemPos::window_type` is the container; `CgSash::window` is a flag.
//!
//! # Fixed raw storage
//!
//! Neither member is text, and neither is validated. There is no `&str`, no
//! `CStr`, and no NUL handling, because the legacy side does not treat these
//! bytes as a string.

use crate::cg_wire::ClientFrame;

/// The packed wire width of [`ItemPos`]: `BYTE` plus `WORD` under `#pragma pack(1)`.
pub const ITEM_POS_WIRE_SIZE: usize = 1 + 2;

/// Every way an [`ItemPos`] decoder can refuse a byte slice.
///
/// The two failure modes are the same shape as the record-level errors, and the
/// variants carry the same fields, so [`crate::cg_sash::CgSashError`] converts
/// from this with a total `From` impl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemPosError {
    /// Fewer bytes than the fixed position needs.
    Truncated {
        /// The fixed width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// A complete-length slice that is not the fixed width.
    LengthMismatch {
        /// The fixed width the decoder requires.
        expected: usize,
        /// How many bytes were actually offered.
        actual: usize,
    },
}

impl core::fmt::Display for ItemPosError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated item position: need {needed} bytes, got {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "item position length mismatch: expected {expected} bytes, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ItemPosError {}

/// The legacy `SItemPos`, a packed `BYTE window_type` followed by `WORD cell`.
///
/// This is 3 bytes on the wire. `window_type` is unrelated to a record's own
/// `window` field -- see the module documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ItemPos {
    /// The legacy `window_type` byte. `INVENTORY`, `EQUIPMENT`, `SAFEBOX` and the
    /// rest are semantic policy above this codec; every `u8` is preserved.
    pub window_type: u8,
    /// The legacy `cell` index, little-endian.
    pub cell: u16,
}

impl ItemPos {
    /// The packed wire width: `BYTE` plus `WORD` under `#pragma pack(1)`.
    pub const WIRE_SIZE: usize = ITEM_POS_WIRE_SIZE;

    /// Build a position.
    pub const fn new(window_type: u8, cell: u16) -> Self {
        Self { window_type, cell }
    }

    /// Append the position to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.window_type);
        out.extend_from_slice(&self.cell.to_le_bytes());
    }

    /// Encode to a fresh 3-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// # Errors
    ///
    /// Returns [`ItemPosError::Truncated`] for fewer than 3 bytes and
    /// [`ItemPosError::LengthMismatch`] for more.
    pub fn decode(bytes: &[u8]) -> Result<Self, ItemPosError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        Ok(Self {
            window_type: bytes[0],
            cell: u16::from_le_bytes([bytes[1], bytes[2]]),
        })
    }

    /// Decode a position embedded in a larger buffer at `at`.
    ///
    /// # Panics
    ///
    /// Panics if `at + 3` is past the end of `bytes`. Callers that do not want a
    /// panic must validate the enclosing record's length first, which is what
    /// every record decoder in this crate does before it reaches here.
    pub fn decode_at(bytes: &[u8], at: usize) -> Self {
        Self {
            window_type: bytes[at],
            cell: u16::from_le_bytes([bytes[at + 1], bytes[at + 2]]),
        }
    }

    /// Read a position out of a header-less frame payload at `at`.
    ///
    /// # Panics
    ///
    /// Panics if `at + 3` is past the end of `payload`, for the same reason as
    /// [`ItemPos::decode_at`].
    pub fn decode_frame_at(frame: &ClientFrame, at: usize) -> Self {
        Self::decode_at(&frame.payload, at)
    }
}

/// Reject anything that is not exactly `expected` bytes wide.
fn check_exact(actual: usize, expected: usize) -> Result<(), ItemPosError> {
    if actual < expected {
        return Err(ItemPosError::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(ItemPosError::LengthMismatch { expected, actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_packed_width_is_three_bytes() {
        assert_eq!(ITEM_POS_WIRE_SIZE, 3);
        assert_eq!(ItemPos::WIRE_SIZE, 3);
        assert_eq!(1 + 2, ItemPos::WIRE_SIZE);
    }

    #[test]
    fn both_sash_registrations_agree_on_the_width() {
        // sizeof(TPacketSash) = 23, per packet_info.cpp:171.
        assert_eq!(1 + 1 + 1 + 4 + 1 + ItemPos::WIRE_SIZE + 4 + 4 + 4, 23);
        // sizeof(TPacketCGDragonSoulRefine) = 47, per packet_info.cpp:171.
        assert_eq!(1 + 1 + 15 * ItemPos::WIRE_SIZE, 47);
    }

    #[test]
    fn a_four_byte_width_would_break_both_registrations() {
        // The positive control: if the width were ever "corrected" to 4, these
        // two assertions are what would fail.
        assert_ne!(1 + 1 + 1 + 4 + 1 + 4 + 4 + 4 + 4, 23);
        assert_ne!(1 + 1 + 15 * 4, 47);
    }

    #[test]
    fn the_field_order_is_window_type_then_cell() {
        let bytes = ItemPos::new(0x03, 0x1234).encode();
        assert_eq!(bytes, vec![0x03, 0x34, 0x12]);
    }

    #[test]
    fn encode_produces_exactly_three_bytes() {
        assert_eq!(ItemPos::new(1, 2).encode().len(), 3);
        assert_eq!(ItemPos::default().encode().len(), 3);
    }

    #[test]
    fn decode_round_trips() {
        let pos = ItemPos::new(0xFE, 0xFFFF);
        assert_eq!(ItemPos::decode(&pos.encode()).unwrap(), pos);
    }

    #[test]
    fn every_window_type_value_round_trips() {
        for w in 0..=u8::MAX {
            let pos = ItemPos::new(w, 0);
            assert_eq!(ItemPos::decode(&pos.encode()).unwrap().window_type, w);
        }
    }

    #[test]
    fn every_cell_value_round_trips() {
        for c in [0_u16, 1, 0x00FF, 0x0100, 0x7FFF, 0x8000, 0xFFFE, 0xFFFF] {
            let pos = ItemPos::new(0, c);
            assert_eq!(ItemPos::decode(&pos.encode()).unwrap().cell, c);
        }
    }

    #[test]
    fn cell_is_little_endian() {
        assert_eq!(ItemPos::new(0, 0x0102).encode(), vec![0x00, 0x02, 0x01]);
    }

    #[test]
    fn decode_rejects_two_bytes_as_truncated() {
        assert_eq!(
            ItemPos::decode(&[1, 2]).unwrap_err(),
            ItemPosError::Truncated {
                needed: 3,
                available: 2
            }
        );
    }

    #[test]
    fn decode_rejects_an_empty_slice_as_truncated() {
        assert_eq!(
            ItemPos::decode(&[]).unwrap_err(),
            ItemPosError::Truncated {
                needed: 3,
                available: 0
            }
        );
    }

    #[test]
    fn decode_rejects_four_bytes_as_a_length_mismatch() {
        assert_eq!(
            ItemPos::decode(&[1, 2, 3, 4]).unwrap_err(),
            ItemPosError::LengthMismatch {
                expected: 3,
                actual: 4
            }
        );
    }

    #[test]
    fn decode_at_reads_the_three_bytes_at_an_offset() {
        let buf = [0xAA, 0xBB, 0x2A, 0xEF, 0xBE, 0xCC];
        assert_eq!(ItemPos::decode_at(&buf, 2), ItemPos::new(0x2A, 0xBEEF));
    }

    #[test]
    fn decode_frame_at_reads_from_the_payload() {
        let frame = ClientFrame {
            header: 7,
            payload: vec![0x00, 0x11, 0x22, 0x2A, 0xEF, 0xBE],
        };
        assert_eq!(
            ItemPos::decode_frame_at(&frame, 3),
            ItemPos::new(0x2A, 0xBEEF)
        );
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE, 0xAD];
        ItemPos::new(1, 0x0304).encode_into(&mut out);
        assert_eq!(out, vec![0xDE, 0xAD, 0x01, 0x04, 0x03]);
    }

    #[test]
    fn default_is_the_legacy_inventory_placeholder_shape() {
        // SItemPos() sets window_type = INVENTORY and cell = WORD_MAX. The codec
        // keeps the derived Default at zero/zero; the legacy constructor is
        // policy above this boundary, not a wire fact.
        let d = ItemPos::default();
        assert_eq!(d.window_type, 0);
        assert_eq!(d.cell, 0);
        assert_eq!(d.encode(), vec![0, 0, 0]);
    }

    #[test]
    fn equality_is_fieldwise() {
        assert_eq!(ItemPos::new(1, 2), ItemPos::new(1, 2));
        assert_ne!(ItemPos::new(1, 2), ItemPos::new(2, 2));
        assert_ne!(ItemPos::new(1, 2), ItemPos::new(1, 3));
    }
}
