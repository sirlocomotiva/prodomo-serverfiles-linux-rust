//! `HEADER_GC_AURA` (214): the aura refine window's answers.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:3049-3055` declares the head every aura send writes:
//!
//! ```ignore
//! typedef struct SPacketGCAura
//! {
//!     SPacketGCAura() : bHeader(HEADER_GC_AURA) {}
//!     BYTE bHeader;
//!     WORD wSize;
//!     BYTE bSubHeader;
//! } TPacketGCAura;
//! ```
//!
//! The subheaders are the enum at `packet.h:2981-2989`: `AURA_SUBHEADER_GC_OPEN` (0),
//! `CLOSE` (1), `SET_ITEM` (2), `CLEAR_SLOT` (3), `CLEAR_ALL` (4), `CLEAR_RIGHT` (5) and
//! `REFINE_INFO` (6). `CHARACTER::OpenAuraRefineWindow`
//! (`server/server/game/char_aura.cpp:14-71`) sends the open: `wSize` counts the whole
//! record, the subheader is `OPEN`, and the one body byte is the window type
//! (`EAuraWindowType`, `C/length.h:1087-1093`: absorb 0, growth 1, evolve 2).
//! `AURA_WINDOW_TYPE_MAX` (3) returns without sending.
//!
//! # Width
//!
//! The open is 1 + 2 + 1 + 1 = 5 bytes. The close (`AuraRefineWindowClose`,
//! `:64-98`) is the same width, with subheader `CLOSE` and the body byte
//! `AURA_WINDOW_TYPE_MAX` (`C/length.h:1088-1094`: absorb 0, growth 1, evolve 2,
//! max 3). The check-out answers `CLEAR_SLOT` with the slot byte, or `CLEAR_ALL`
//! with no body, and the accept answers `CLEAR_ALL` too (`:693-1071`).
//! `CLEAR_RIGHT` (5) has no send site: only the enum names it. `REFINE_INFO` waits
//! for the growth and evolve systems, its only callers.

use crate::gc_item_window::{ItemAttributes, ItemSockets};
use crate::item_pos::ItemPos;

/// `HEADER_GC_AURA`.
pub const HEADER: u8 = 214;

/// `AURA_SUBHEADER_GC_OPEN`.
pub const SUB_OPEN: u8 = 0;

/// `AURA_SUBHEADER_GC_CLOSE`.
pub const SUB_CLOSE: u8 = 1;

/// `AURA_SUBHEADER_GC_SET_ITEM`.
pub const SUB_SET_ITEM: u8 = 2;

/// `AURA_SUBHEADER_GC_CLEAR_SLOT`.
pub const SUB_CLEAR_SLOT: u8 = 3;

/// `AURA_SUBHEADER_GC_CLEAR_ALL`.
pub const SUB_CLEAR_ALL: u8 = 4;

/// `AURA_WINDOW_TYPE_ABSORB`.
pub const WINDOW_ABSORB: u8 = 0;

/// `AURA_WINDOW_TYPE_GROWTH`.
pub const WINDOW_GROWTH: u8 = 1;

/// `AURA_WINDOW_TYPE_EVOLVE`.
pub const WINDOW_EVOLVE: u8 = 2;

/// `AURA_WINDOW_TYPE_MAX`: the close's body byte.
pub const WINDOW_MAX: u8 = 3;

/// The open's width: the head and the one window-type byte.
pub const OPEN_LEN: usize = 5;

/// The close's width: the head and the one window-type byte.
pub const CLOSE_LEN: usize = 5;

/// The clear-slot's width: the head and the one slot byte.
pub const CLEAR_SLOT_LEN: usize = 5;

/// The clear-all's width: the bare head.
pub const CLEAR_ALL_LEN: usize = 4;

/// The check-in's width: the head, the two cells, and the item copy —
/// 4 + 3 + 3 + (4 + 2 + 4 + 4 + 4 + 4 + 6 × 4 + 7 × 3) = 77.
pub const SET_ITEM_LEN: usize = 77;

/// An aura window open: `OpenAuraRefineWindow`'s record for window `window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcAuraOpen {
    /// The window type: 0 absorb, 1 growth, 2 evolve.
    pub window: u8,
}

impl GcAuraOpen {
    /// Encode the 5-byte open: the header, the size word counting the whole record,
    /// the `OPEN` subheader, and the window type.
    ///
    /// # Panics
    ///
    /// Never: the open is 5 bytes, so the size word fits.
    #[must_use]
    pub fn encode(self) -> [u8; OPEN_LEN] {
        let size = u16::try_from(OPEN_LEN).expect("the open is short");
        let [lo, hi] = size.to_le_bytes();
        [HEADER, lo, hi, SUB_OPEN, self.window]
    }
}

/// An aura window close: `AuraRefineWindowClose`'s record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcAuraClose;

impl GcAuraClose {
    /// Encode the 5-byte close: the header, the size word, the `CLOSE`
    /// subheader, and `AURA_WINDOW_TYPE_MAX`.
    ///
    /// # Panics
    ///
    /// Never: the close is 5 bytes, so the size word fits.
    #[must_use]
    pub fn encode(self) -> [u8; CLOSE_LEN] {
        let size = u16::try_from(CLOSE_LEN).expect("the close is short");
        let [lo, hi] = size.to_le_bytes();
        [HEADER, lo, hi, SUB_CLOSE, WINDOW_MAX]
    }
}

/// An aura slot clear: `AuraRefineWindowCheckOut`'s record for one slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcAuraClearSlot {
    /// The cleared slot: 0 main, 1 sub.
    pub slot: u8,
}

impl GcAuraClearSlot {
    /// Encode the 5-byte clear: the header, the size word, the `CLEAR_SLOT`
    /// subheader, and the slot.
    ///
    /// # Panics
    ///
    /// Never: the clear is 5 bytes, so the size word fits.
    #[must_use]
    pub fn encode(self) -> [u8; CLEAR_SLOT_LEN] {
        let size = u16::try_from(CLEAR_SLOT_LEN).expect("the clear is short");
        let [lo, hi] = size.to_le_bytes();
        [HEADER, lo, hi, SUB_CLEAR_SLOT, self.slot]
    }
}

/// An aura full clear: the check-out and accept record with no body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcAuraClearAll;

impl GcAuraClearAll {
    /// Encode the bare 4-byte head with the `CLEAR_ALL` subheader.
    ///
    /// # Panics
    ///
    /// Never: the clear is 4 bytes, so the size word fits.
    #[must_use]
    pub fn encode(self) -> [u8; CLEAR_ALL_LEN] {
        let size = u16::try_from(CLEAR_ALL_LEN).expect("the clear is short");
        let [lo, hi] = size.to_le_bytes();
        [HEADER, lo, hi, SUB_CLEAR_ALL]
    }
}

/// A checked-in material: `AuraRefineWindowCheckIn`'s `SET_ITEM` record. The item
/// copy is the `TItemData` field order (`packet.h:2973-2985`): vnum, count,
/// refine element, transmutation, flags, anti-flags, sockets, attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcAuraSetItem {
    /// The material's own window and cell.
    pub cell: ItemPos,
    /// The aura slot it fills: `AURA_REFINE` and 0 main, 1 sub, 2 result.
    pub aura_cell: ItemPos,
    /// The item vnum. Opaque here; the proto lives above this codec.
    pub vnum: u32,
    /// The opaque `WORD` stack count.
    pub count: u16,
    /// `ENABLE_REFINE_ELEMENT`: the opaque refine-element id.
    pub refine_element: u32,
    /// `__CHANGELOOK_SYSTEM__`: the opaque transmutation value.
    pub transmutation: u32,
    /// The opaque legacy `ITEM_FLAG_*` word.
    pub flags: u32,
    /// The opaque legacy `ITEM_ANTIFLAG_*` word.
    pub anti_flags: u32,
    /// The six opaque socket words.
    pub sockets: ItemSockets,
    /// The seven opaque attribute slots.
    pub attributes: ItemAttributes,
}

impl GcAuraSetItem {
    /// Encode the 77-byte check-in: the header, the size word counting the whole
    /// record, the `SET_ITEM` subheader, the two cells, and the item copy.
    ///
    /// # Panics
    ///
    /// Never: the record is 77 bytes, so the size word fits.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let size = u16::try_from(SET_ITEM_LEN).expect("the set is short");
        let mut out = Vec::with_capacity(SET_ITEM_LEN);
        out.push(HEADER);
        out.extend_from_slice(&size.to_le_bytes());
        out.push(SUB_SET_ITEM);
        self.cell.encode_into(&mut out);
        self.aura_cell.encode_into(&mut out);
        out.extend_from_slice(&self.vnum.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.refine_element.to_le_bytes());
        out.extend_from_slice(&self.transmutation.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.anti_flags.to_le_bytes());
        for socket in self.sockets {
            out.extend_from_slice(&socket.to_le_bytes());
        }
        for attribute in self.attributes {
            attribute.encode_into(&mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The golden open: header 214, size 5, subheader 0, growth 1.
    #[test]
    fn the_open_counts_itself() {
        assert_eq!(
            GcAuraOpen {
                window: WINDOW_GROWTH
            }
            .encode(),
            [214, 5, 0, 0, 1]
        );
        assert_eq!(
            GcAuraOpen {
                window: WINDOW_EVOLVE
            }
            .encode()[4],
            2
        );
    }

    /// The golden close and clears: the subheaders and the body bytes.
    #[test]
    fn the_close_and_the_clears_count_themselves() {
        assert_eq!(GcAuraClose.encode(), [214, 5, 0, 1, 3]);
        assert_eq!(GcAuraClearSlot { slot: 1 }.encode(), [214, 5, 0, 3, 1]);
        assert_eq!(GcAuraClearAll.encode(), [214, 4, 0, 4]);
    }

    /// The golden check-in: distinct halves in every word, so an endianness bug
    /// shows.
    #[test]
    fn the_check_in_copies_the_item_in_field_order() {
        use crate::gc_item_window::ItemAttribute;
        let record = GcAuraSetItem {
            cell: ItemPos::new(1, 0x0203),
            aura_cell: ItemPos::new(7, 0x0405),
            vnum: 0x0607_0809,
            count: 0x0a0b,
            refine_element: 0x0c0d_0e0f,
            transmutation: 0x1011_1213,
            flags: 0x1415_1617,
            anti_flags: 0x1819_1a1b,
            sockets: [0x1c1d_1e1f, 0, 0, 0, 0, 0],
            attributes: [
                ItemAttribute::new(0x20, 0x2221),
                ItemAttribute::new(0, 0),
                ItemAttribute::new(0, 0),
                ItemAttribute::new(0, 0),
                ItemAttribute::new(0, 0),
                ItemAttribute::new(0, 0),
                ItemAttribute::new(0, 0),
            ],
        }
        .encode();
        assert_eq!(record.len(), SET_ITEM_LEN);
        assert_eq!(
            &record[..16],
            &[214, 77, 0, 2, 1, 3, 2, 7, 5, 4, 9, 8, 7, 6, 11, 10]
        );
        assert_eq!(&record[16..24], &[15, 14, 13, 12, 19, 18, 17, 16]);
        assert_eq!(&record[24..32], &[23, 22, 21, 20, 27, 26, 25, 24]);
        assert_eq!(&record[32..36], &[31, 30, 29, 28]);
        assert_eq!(&record[36..56], &[0; 20]);
        assert_eq!(&record[56..59], &[0x20, 0x21, 0x22]);
        assert_eq!(&record[59..77], &[0; 18][..]);
    }
}
