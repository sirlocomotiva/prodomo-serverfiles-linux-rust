//! `HEADER_GC_VIEW_EQUIP` (0x63): another player's worn equipment.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:2490-2507`, inside `#pragma pack(1)`:
//!
//! ```ignore
//! typedef struct pakcet_view_equip
//! {
//!     BYTE    header;
//!     DWORD    vid;
//!     struct {
//!         DWORD    vnum;
//!         WORD    count;
//! #ifdef __CHANGELOOK_SYSTEM__
//!         DWORD    transmutation;
//! #endif
//!         DWORD   dwRefineElement;
//!         long    alSockets[ITEM_SOCKET_MAX_NUM];
//!         TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
//!     } equips[22];
//! } TPacketViewEquip;
//! ```
//!
//! `__CHANGELOOK_SYSTEM__` is on (`server/server/common/prodomodefines.h:16`),
//! and `ITEM_SOCKET_MAX_NUM` is 6 with `ENABLE_EXTENDED_SOCKETS` on (`:76`).
//! `CHARACTER::SendEquipment` (`server/server/game/char.cpp:7815-7846`) is the
//! only writer: the target's VID, then the 22 wear positions in the order
//! `WEAR_BODY, WEAR_HEAD, WEAR_FOOTS, WEAR_WRIST, WEAR_WEAPON, WEAR_NECK,
//! WEAR_EAR, WEAR_UNIQUE1, WEAR_UNIQUE2, WEAR_ARROW, WEAR_SHIELD,
//! WEAR_COSTUME_BODY, WEAR_COSTUME_HAIR, WEAR_COSTUME_MOUNT,
//! WEAR_COSTUME_WEAPON, WEAR_COSTUME_SASH, WEAR_COSTUME_AURA, WEAR_RING1,
//! WEAR_RING2, WEAR_BELT, WEAR_COSTUME_PET, WEAR_COSTUME_SASH_SKIN`. A position
//! with no item writes `vnum = 0` and leaves the rest of its cell untouched —
//! the struct is uninitialised stack, so the bytes are whatever was there.
//! The port writes zeroes: a viewer cannot tell a cleared cell from a fresh one.
//!
//! # Width
//!
//! One cell is 4 + 2 + 4 + 4 + 6 × 4 + 7 × 3 = 59 bytes, and the record is
//! 1 + 4 + 22 × 59 = 1303 bytes. `long` is four bytes on the 32-bit target
//! the game builds for.

use crate::gc_inventory::HEADER_GC_VIEW_EQUIP;
use crate::gc_item_window::{ItemAttributes, ItemSockets};

/// `sizeof(TPacketViewEquip)`.
pub const GC_VIEW_EQUIP_WIRE_SIZE: usize = 1303;

/// The wear positions `SendEquipment` walks, in its order.
pub const VIEW_EQUIP_SLOTS: usize = 22;

/// Packed width of one equipment cell.
pub const VIEW_EQUIP_CELL_WIRE_SIZE: usize = 59;

/// One worn item of a `HEADER_GC_VIEW_EQUIP` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewEquipCell {
    /// The item vnum, or 0 for an empty position.
    pub vnum: u32,
    /// The opaque `WORD` stack count.
    pub count: u16,
    /// `__CHANGELOOK_SYSTEM__`: the opaque transmutation value.
    pub transmutation: u32,
    /// `ENABLE_REFINE_ELEMENT`: the opaque refine-element id.
    pub refine_element: u32,
    /// The six opaque socket words.
    pub sockets: ItemSockets,
    /// The seven opaque attribute slots.
    pub attributes: ItemAttributes,
}

impl ViewEquipCell {
    /// Append the exact 59 packed bytes to `out`.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.vnum.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.transmutation.to_le_bytes());
        out.extend_from_slice(&self.refine_element.to_le_bytes());
        for socket in self.sockets {
            out.extend_from_slice(&socket.to_le_bytes());
        }
        for attribute in self.attributes {
            attribute.encode_into(out);
        }
    }
}

/// `HEADER_GC_VIEW_EQUIP`: the VID followed by its 22 worn cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcViewEquip {
    /// The viewed character's VID.
    pub vid: u32,
    /// The 22 cells in `SendEquipment`'s order.
    pub equips: [ViewEquipCell; VIEW_EQUIP_SLOTS],
}

impl GcViewEquip {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_VIEW_EQUIP.value()
    }

    /// Append the exact 1303 packed bytes to `out`.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.vid.to_le_bytes());
        for cell in self.equips {
            cell.encode_into(out);
        }
    }

    /// Encode to a fresh 1303-byte buffer.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_VIEW_EQUIP_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc_item_window::ItemAttribute;

    /// The golden record: the field order of `pakcet_view_equip`, with values
    /// whose halves differ.
    #[test]
    fn golden_layout_matches_the_struct_field_order() {
        let cell = ViewEquipCell {
            vnum: 0x1122_3344,
            count: 0x5566,
            transmutation: 0x7788_99AA,
            refine_element: 0xBBCC_DDEE,
            sockets: [0x0102_0304, -1, 0, 0, 0, 0],
            attributes: [
                ItemAttribute::new(0x11, 0x2233),
                ItemAttribute::default(),
                ItemAttribute::default(),
                ItemAttribute::default(),
                ItemAttribute::default(),
                ItemAttribute::default(),
                ItemAttribute::default(),
            ],
        };
        let mut equips = [ViewEquipCell::default(); VIEW_EQUIP_SLOTS];
        equips[0] = cell;
        let record = GcViewEquip {
            vid: 0x99AA_BBCC,
            equips,
        }
        .encode();
        assert_eq!(record.len(), GC_VIEW_EQUIP_WIRE_SIZE);
        assert_eq!(record[0], 0x63);
        assert_eq!(&record[1..5], &[0xCC, 0xBB, 0xAA, 0x99]);
        let first = &record[5..5 + VIEW_EQUIP_CELL_WIRE_SIZE];
        assert_eq!(&first[0..4], &[0x44, 0x33, 0x22, 0x11]);
        assert_eq!(&first[4..6], &[0x66, 0x55]);
        assert_eq!(&first[6..10], &[0xAA, 0x99, 0x88, 0x77]);
        assert_eq!(&first[10..14], &[0xEE, 0xDD, 0xCC, 0xBB]);
        assert_eq!(&first[14..18], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&first[18..22], &[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(&first[38..41], &[0x11, 0x33, 0x22]);
        assert!(record[5 + VIEW_EQUIP_CELL_WIRE_SIZE..]
            .iter()
            .all(|&byte| byte == 0));
    }
}
