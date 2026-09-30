//! `HEADER_GC_SAFEBOX_SET` (85) and `HEADER_GC_MALL_SET` (128): an item shown in the safebox or
//! the item mall.
//!
//! # Provenance
//!
//! Both records are the 72-byte `TPacketGCItemSet` of [`GcItemSet`] under another header.
//! `server/server/game/safebox.cpp:71-86` (`CSafebox::Add`) is the only writer:
//!
//! ```ignore
//! TPacketGCItemSet pack;
//!
//! pack.header = m_bWindowMode == SAFEBOX ? HEADER_GC_SAFEBOX_SET : HEADER_GC_MALL_SET;
//! pack.Cell   = TItemPos(m_bWindowMode, dwPos);
//! pack.vnum   = pkItem->GetVnum();
//! pack.count  = pkItem->GetCount();
//! ```
//!
//! then the refine element, the transmutation, the flags, the anti-flags, the sockets and the
//! attributes. It never writes `highlight`, so legacy sends whatever the stack held there (a
//! Defect); the Rewrite sends 0. The cell's window is always the record's own window, `SAFEBOX`
//! (3) under byte 85 and `MALL` (4) under byte 128, and the two are kept together here so a
//! record cannot carry one header and the other window.
//!
//! The delete, size, open and wrong-password records that go with them are the small shapes of
//! [`crate::gc_vid::GcHeaderAndDword`], [`crate::gc_small::GcHeaderAndByte`] and
//! [`crate::gc_small::GcHeaderOnly`].

use crate::gc_inventory::{HEADER_GC_MALL_SET, HEADER_GC_SAFEBOX_SET};
use crate::gc_item_window::{window, GcItemSet, GcItemWindowError, GC_ITEM_SET_WIRE_SIZE};

/// The window a safebox record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoreWindow {
    /// `SAFEBOX` (3): the account's password-locked storage.
    Safebox,
    /// `MALL` (4): the item mall's delivery window.
    Mall,
}

impl StoreWindow {
    /// The window byte of the record's cell.
    #[must_use]
    pub const fn window_type(self) -> u8 {
        match self {
            Self::Safebox => window::SAFEBOX,
            Self::Mall => window::MALL,
        }
    }

    /// The header of this window's set record.
    #[must_use]
    pub const fn set_header(self) -> u8 {
        match self {
            Self::Safebox => HEADER_GC_SAFEBOX_SET.value(),
            Self::Mall => HEADER_GC_MALL_SET.value(),
        }
    }

    /// The header of this window's delete record.
    #[must_use]
    pub const fn del_header(self) -> u8 {
        match self {
            Self::Safebox => crate::gc_inventory::HEADER_GC_SAFEBOX_DEL.value(),
            Self::Mall => crate::gc_inventory::HEADER_GC_MALL_DEL.value(),
        }
    }
}

/// `HEADER_GC_SAFEBOX_SET` or `HEADER_GC_MALL_SET`: one item in a safebox cell.
///
/// `item` carries every field of the 72-byte record except the header, which
/// [`GcStoreItemSet::header`] takes from the window. The window of `item.cell` is written as it
/// stands; [`GcStoreItemSet::decode`] refuses a record whose cell names another window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcStoreItemSet {
    /// The window, which picks the header.
    pub window: StoreWindow,
    /// The record's fields after the header.
    pub item: GcItemSet,
}

impl GcStoreItemSet {
    /// The record's header byte.
    #[must_use]
    pub const fn header(&self) -> u8 {
        self.window.set_header()
    }

    /// Append the exact 72 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        let start = out.len();
        self.item.encode_into(out);
        out[start] = self.header();
    }

    /// Encode to a fresh 72-byte buffer.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_ITEM_SET_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Read the record back.
    ///
    /// # Errors
    ///
    /// Returns [`GcItemWindowError::Truncated`] or [`GcItemWindowError::LengthMismatch`] unless
    /// the buffer is exactly 72 bytes, and [`GcItemWindowError::Header`] when the leading byte is
    /// neither 85 nor 128, or when the cell's window is not the one the header names (the
    /// `expected` byte is then that window).
    pub fn decode(bytes: &[u8]) -> Result<Self, GcItemWindowError> {
        let Some(&first) = bytes.first() else {
            return Err(GcItemWindowError::Truncated {
                context: "GcStoreItemSet",
                needed: GC_ITEM_SET_WIRE_SIZE,
                actual: 0,
            });
        };
        let window = if first == HEADER_GC_SAFEBOX_SET.value() {
            StoreWindow::Safebox
        } else if first == HEADER_GC_MALL_SET.value() {
            StoreWindow::Mall
        } else {
            return Err(GcItemWindowError::Header {
                context: "GcStoreItemSet",
                expected: HEADER_GC_SAFEBOX_SET.value(),
                actual: first,
            });
        };
        let mut body = bytes.to_vec();
        body[0] = GcItemSet::header();
        let item = GcItemSet::decode(&body).map_err(|error| match error {
            GcItemWindowError::Truncated { needed, actual, .. } => GcItemWindowError::Truncated {
                context: "GcStoreItemSet",
                needed,
                actual,
            },
            GcItemWindowError::LengthMismatch {
                expected, actual, ..
            } => GcItemWindowError::LengthMismatch {
                context: "GcStoreItemSet",
                expected,
                actual,
            },
            other @ GcItemWindowError::Header { .. } => other,
        })?;
        if item.cell.window_type != window.window_type() {
            return Err(GcItemWindowError::Header {
                context: "GcStoreItemSet window",
                expected: window.window_type(),
                actual: item.cell.window_type,
            });
        }
        Ok(Self { window, item })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc_inventory::{resolve_gc_packet, HEADER_GC_MALL_DEL, HEADER_GC_SAFEBOX_DEL};
    use crate::gc_item_window::{ItemAttribute, ItemAttributes, ItemSockets};
    use crate::item_pos::ItemPos;

    /// Distinct byte halves on every multi-byte field, so a big-endian read cannot pass by byte
    /// symmetry.
    const SOCKETS: ItemSockets = [
        0x0102_0304,
        0x1122_3344,
        0x5566_7788,
        -0x0102_0304,
        i32::from_le_bytes([0xcc, 0xbb, 0xaa, 0x99]),
        i32::MIN,
    ];
    const ATTRIBUTES: ItemAttributes = [
        ItemAttribute::new(0x1a, 0x2b3c),
        ItemAttribute::new(0x2b, -0x3c4d),
        ItemAttribute::new(0x3c, 0x4d5e),
        ItemAttribute::new(0x4d, i16::MIN),
        ItemAttribute::new(0x5e, i16::MAX),
        ItemAttribute::new(0x6f, 0),
        ItemAttribute::new(0x70, 1),
    ];

    fn a_record(window: StoreWindow) -> GcStoreItemSet {
        GcStoreItemSet {
            window,
            item: GcItemSet {
                cell: ItemPos::new(window.window_type(), 0x2b1a),
                vnum: 0x5f4e_3d2c,
                count: 0x6b5a,
                refine_element: 0x7e6d_5c4b,
                transmutation: 0x8f7e_6d5c,
                flags: 0x9a8b_7c6d,
                anti_flags: 0xab9c_8d7e,
                highlight: 0,
                sockets: SOCKETS,
                attributes: ATTRIBUTES,
            },
        }
    }

    #[test]
    fn the_headers_are_the_inventory_bytes() {
        assert_eq!(StoreWindow::Safebox.set_header(), 85);
        assert_eq!(StoreWindow::Mall.set_header(), 128);
        assert_eq!(StoreWindow::Safebox.del_header(), 86);
        assert_eq!(StoreWindow::Mall.del_header(), 129);
        assert_eq!(
            StoreWindow::Safebox.del_header(),
            HEADER_GC_SAFEBOX_DEL.value()
        );
        assert_eq!(StoreWindow::Mall.del_header(), HEADER_GC_MALL_DEL.value());
        assert_eq!(StoreWindow::Safebox.window_type(), 3);
        assert_eq!(StoreWindow::Mall.window_type(), 4);
        for byte in [85, 128] {
            let row = resolve_gc_packet(byte).expect("registered");
            assert_eq!(row.cpp_type, "TPacketGCItemSet");
            assert!(row.implemented_in_rust, "byte {byte}");
        }
    }

    /// The whole 72-byte safebox record pinned against the legacy field order.
    #[test]
    fn the_safebox_record_is_pinned_byte_for_byte() {
        let wire = a_record(StoreWindow::Safebox).encode();
        assert_eq!(wire.len(), 72);
        assert_eq!(&wire[0..1], &[85]);
        assert_eq!(&wire[1..4], &[3, 26, 43]);
        assert_eq!(&wire[4..8], &[44, 61, 78, 95]);
        assert_eq!(&wire[8..10], &[90, 107]);
        assert_eq!(&wire[10..14], &[75, 92, 109, 126]);
        assert_eq!(&wire[14..18], &[92, 109, 126, 143]);
        assert_eq!(&wire[18..22], &[109, 124, 139, 154]);
        assert_eq!(&wire[22..26], &[126, 141, 156, 171]);
        assert_eq!(&wire[26..27], &[0]);
        assert_eq!(&wire[27..31], &[4, 3, 2, 1]);
        assert_eq!(&wire[31..35], &[68, 51, 34, 17]);
        assert_eq!(&wire[39..43], &[252, 252, 253, 254]);
        assert_eq!(&wire[51..54], &[26, 60, 43]);
        assert_eq!(&wire[54..57], &[43, 179, 195]);
        assert_eq!(&wire[69..72], &[112, 1, 0]);
    }

    /// The mall record differs from the safebox record in exactly the header and the window.
    #[test]
    fn the_mall_record_differs_only_in_the_header_and_the_window() {
        let safebox = a_record(StoreWindow::Safebox).encode();
        let mall = a_record(StoreWindow::Mall).encode();
        assert_eq!(&mall[0..2], &[128, 4]);
        assert_eq!(&mall[2..], &safebox[2..]);
    }

    #[test]
    fn both_records_round_trip() {
        for window in [StoreWindow::Safebox, StoreWindow::Mall] {
            let original = a_record(window);
            let mut out = vec![0xee];
            original.encode_into(&mut out);
            assert_eq!(out[0], 0xee, "encode_into appends");
            assert_eq!(GcStoreItemSet::decode(&out[1..]).unwrap(), original);
        }
    }

    #[test]
    fn a_wrong_header_a_wrong_window_and_a_wrong_length_are_refused() {
        let mut wire = a_record(StoreWindow::Safebox).encode();
        wire[0] = GcItemSet::header();
        assert_eq!(
            GcStoreItemSet::decode(&wire).unwrap_err(),
            GcItemWindowError::Header {
                context: "GcStoreItemSet",
                expected: 85,
                actual: 21,
            }
        );
        wire[0] = 128;
        assert_eq!(
            GcStoreItemSet::decode(&wire).unwrap_err(),
            GcItemWindowError::Header {
                context: "GcStoreItemSet window",
                expected: 4,
                actual: 3,
            }
        );
        wire[0] = 85;
        assert_eq!(
            GcStoreItemSet::decode(&wire[..71]).unwrap_err(),
            GcItemWindowError::Truncated {
                context: "GcStoreItemSet",
                needed: 72,
                actual: 71,
            }
        );
        assert_eq!(
            GcStoreItemSet::decode(&[]).unwrap_err(),
            GcItemWindowError::Truncated {
                context: "GcStoreItemSet",
                needed: 72,
                actual: 0,
            }
        );
        let mut long = wire.clone();
        long.push(0);
        assert_eq!(
            GcStoreItemSet::decode(&long).unwrap_err(),
            GcItemWindowError::LengthMismatch {
                context: "GcStoreItemSet",
                expected: 72,
                actual: 73,
            }
        );
    }

    /// A plain inventory set record must not decode as a safebox record, and a safebox record
    /// must not decode as an inventory one.
    #[test]
    fn the_inventory_and_safebox_set_records_reject_each_other() {
        let inventory = GcItemSet {
            cell: ItemPos::new(window::SAFEBOX, 1),
            ..a_record(StoreWindow::Safebox).item
        };
        assert!(GcStoreItemSet::decode(&inventory.encode()).is_err());
        assert!(GcItemSet::decode(&a_record(StoreWindow::Safebox).encode()).is_err());
    }
}
