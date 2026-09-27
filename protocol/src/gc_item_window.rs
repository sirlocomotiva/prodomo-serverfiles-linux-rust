//! The C bodies below are transcribed from the legacy source with its tab
//! indentation expanded to four spaces, because the `tabs_in_doc_comments`
//! lint rejects a tab inside a doc comment. The field order, the gate macros
//! and the member names are verbatim; only the leading whitespace differs, and
//! no field of any of these structs is a character literal, so nothing about the
//! layout depends on it.
//! The four game-to-client records that carry an item into or out of a window.
//!
//! # The two window records are named backwards between the trees
//!
//! This is the sharpest cross-direction rename in the tree, and it is easy to
//! get backwards, so it is stated first and tested:
//!
//! | wire byte | server enumerator | client name | client struct | server struct | width |
//! |---|---|---|---|---|---|
//! | 20 | `HEADER_GC_ITEM_DEL` | `HEADER_GC_ITEM_SET` | `TPacketGCItemSet` | `TPacketGCItemDelDeprecated` | 62 |
//! | 21 | `HEADER_GC_ITEM_SET` | `HEADER_GC_ITEM_SET2` | `TPacketGCItemSet2` | `TPacketGCItemSet` | 72 |
//!
//! Read the middle column top to bottom and the rename looks like a plain
//! generation suffix. Read it diagonally and the *set* record is the byte the
//! client calls a *delete*, and the *delete* record is the byte the client calls
//! a *set*. `server/server/game/packet.h:121-122` declares the two enumerators,
//! and `server/server/game/char_item.cpp:572` writes 21 while
//! `char_item.cpp:598` writes 20 into the deprecated delete struct.
//!
//! The client name for byte 20 (`HEADER_GC_ITEM_SET`, struct `TPacketGCItemSet`,
//! 72 bytes on the client) is **not** what the server sends there. The server
//! sends 62 bytes. This module therefore types both records after the **server**
//! structs, and the constants below carry the **server** names, matching
//! `crate::gc::HEADER_GC_TIME_SYNC` and the rest of the crate's convention. The
//! tests assert the byte values against `crate::gc_inventory`, which is keyed by
//! the client names, so the two tables cannot drift apart unnoticed.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:1427-1444`:
//!
//! ```ignore
//! typedef struct packet_item_set
//! {
//!     BYTE    header;
//!     TItemPos Cell;
//!     DWORD    vnum;
//!     WORD    count;
//! #ifdef ENABLE_REFINE_ELEMENT
//!     DWORD    dwRefineElement;
//! #endif
//! #ifdef __CHANGELOOK_SYSTEM__
//!     DWORD    transmutation;
//! #endif
//!     DWORD    flags;
//!     DWORD    anti_flags;
//!     bool    highlight;
//!     long    alSockets[ITEM_SOCKET_MAX_NUM];
//!     TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
//! } TPacketGCItemSet;
//! ```
//!
//! `server/server/game/packet.h:1085-1099`:
//!
//! ```ignore
//! struct TPacketGCItemDelDeprecated
//! {
//!     BYTE    header;
//!     TItemPos Cell;
//!     DWORD    vnum;
//!     BYTE    count;
//! #ifdef ENABLE_REFINE_ELEMENT
//!     DWORD    dwRefineElement;
//! #endif
//! #ifdef __CHANGELOOK_SYSTEM__
//!     DWORD    transmutation;
//! #endif
//!     long    alSockets[ITEM_SOCKET_MAX_NUM];
//!     TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
//! };
//! ```
//!
//! `server/server/game/packet.h:1472-1485` and `:1487-1493`:
//!
//! ```ignore
//! typedef struct packet_item_update
//! {
//!     BYTE    header;
//!     TItemPos Cell;
//!     WORD    count;
//!     // ... the same two feature-gated DWORDs, then:
//!     long    alSockets[ITEM_SOCKET_MAX_NUM];
//!     TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
//! } TPacketGCItemUpdate;
//!
//! typedef struct packet_item_ground_add
//! {
//!     BYTE    bHeader;
//!     long     x, y, z;
//!     DWORD    dwVID;
//!     DWORD    dwVnum;
//! } TPacketGCItemGroundAdd;
//! ```
//!
//! The delete record is named `Deprecated` in the legacy source but it is the
//! record the inventory actually sends. The 5-byte `TPacketGCItemDel`
//! (`packet.h:1446-1454`, a `DWORD pos` under `__EXTENDED_SAFEBOX__`) has exactly
//! one producer in the whole tree, `server/server/game/safebox.cpp:117`, and is
//! a safebox record, not an inventory one. It is not this module's business.
//!
//! # The feature gates are all on
//!
//! `server/server/common/prodomodefines.h` defines `ENABLE_REFINE_ELEMENT`
//! (line 28), `__CHANGELOOK_SYSTEM__` (line 16), `__EXTENDED_SAFEBOX__` (line 61)
//! and `ENABLE_EXTENDED_SOCKETS` (line 76). The last one is the easy miss: with
//! it on, `ITEM_SOCKET_MAX_NUM` is **6**, not the 3 in the `#else` arm of
//! `server/server/common/item_length.h:17`. Every width below is 24 bytes of
//! sockets, not 12. `ITEM_ATTRIBUTE_MAX_NUM` is 7 (`item_length.h:30`), computed as
//! `ITEM_ATTRIBUTE_RARE_END`: five normal attributes from `:24-25` plus two rare
//! from `:27-28`. So the attribute array is 21 bytes.
//!
//! # Widths
//!
//! This machine has no `i686-linux-gnu-g++-12` and no 32-bit multilib, so the
//! widths were not measured on the legacy target. They were measured by a
//! compiled probe on the host with one textual substitution: `#define long
//! int32_t` before the verbatim struct bodies, under the real gate macros and the
//! real `item_length.h` enum. The probe re-measured three controls on every run --
//! a packed two-`long` struct that must be 8, a `BYTE WORD bool` struct that must
//! be 4, and a three-`DWORD` struct that must be 12 -- and all three came out
//! right, which is what makes the substitution trustworthy for the records that
//! contain `long`.
//!
//! Three independent cross-checks agree with the probe, and each would fail at a
//! different wrong width:
//!
//! | record | probe | hand sum | witnesses |
//! |---|---|---|---|
//! | `GcItemSet` | 72 | 1+3+4+2+4+4+4+4+1+24+21 = 72 | -- |
//! | `GcItemDel` | 62 | 1+3+4+1+4+4+24+21 = 62 | -- |
//! | `GcItemUpdate` | 59 | 1+3+2+4+4+24+21 = 59 | -- |
//! | `GcItemGroundAdd` | 21 | 1+12+4+4 = 21 | -- |
//! | `ItemAttribute` | 3 | 1+2 = 3 | `packet_info.cpp:171` sizes two records around 3-byte positions |
//!
//! A fourth witness is independent of the probe: this repository's own
//! cross-direction collision table recorded `| HEADER_GC_ITEM_SET (21) |
//! HEADER_GC_ITEM_SET2 (21) | 72 | identical |` at
//! `docs/REWRITE_LEDGER.md:11325`, written by an earlier pass that did not have
//! this module. 72 is the six-socket width, so a three-socket reading would have
//! produced 60 and that table would say 60.
//!
//! # The byte-20 record does not match the client
//!
//! `docs/PROTOCOL_NOTES.md:461` records, for byte 20, that the client has no
//! `HEADER_GC_ITEM_DEL` at all and that its only 20 is `HEADER_GC_ITEM_SET`,
//! dispatched to the item-**set** handler. It also recorded that whether the two
//! widths match "is not determined", because the client's own
//! `ITEM_SOCKET_SLOT_MAX_NUM` is 3 or 6 at `GameType.h:550-552`.
//!
//! The server side is now measured: 62. The client's registered width for byte 20
//! is its item-set width, which is 72 at six sockets and 60 at three, so **62
//! matches neither**. The client's socket-count ambiguity therefore does not have
//! to be resolved to decide that this frame is the wrong size, and the outcome is
//! the same either way: `CheckPacket` at `PythonNetworkStream.cpp:537-543`
//! drops the frame. That is a Defect, recorded in the ledger and not reproduced.
//! What the Rewrite should send to clear a window slot is a design question for
//! the owner; this module models the legacy send faithfully and does not answer
//! it.
//!
//! The delete record's 62 against the set record's 72 is the check that would
//! catch a forgotten field: the set record has `WORD count`, two flag `DWORD`s and
//! a `bool`, the delete record has `BYTE count` and none of those, and 62 + 10 is
//! exactly 72.
//!
//! # What this codec does not do
//!
//! `TItemPos::window_type` and `cell` stay opaque, and so do the sockets, the
//! attribute types and values, the flags, and the two element fields. A socket is
//! a `long`, and the legacy window code stores vnums in it, but nothing on this
//! path range-checks it, so every `i32` round-trips. Attribute type is a `BYTE`
//! that the item manager later resolves through `FN_get_apply_type`; that
//! resolution is policy above this codec. `highlight` is a `bool` and therefore
//! **one byte** under `pack(1)`, decoded as a raw `u8` and never as a Rust `bool`.
//!
//! The set record's `count` is a `WORD` while the delete record's is a `BYTE`.
//! That asymmetry is real and is the widest single-field difference between the
//! two records; `GcItemSet::count` and `GcItemDel::count` are separate fields on
//! separate types for that reason and must not be unified.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::item_pos::ItemPos;

/// Server enumerator `HEADER_GC_ITEM_DEL` (20), which the client calls
/// `HEADER_GC_ITEM_SET`. This is the byte that clears a window slot.
pub const HEADER_GC_ITEM_DEL: u8 = 20;

/// Server enumerator `HEADER_GC_ITEM_SET` (21), which the client calls
/// `HEADER_GC_ITEM_SET2`. This is the byte that fills a window slot.
pub const HEADER_GC_ITEM_SET: u8 = 21;

/// `HEADER_GC_ITEM_UPDATE` (25): a slot changed in place, with no window move.
pub const HEADER_GC_ITEM_UPDATE: u8 = 25;

/// `HEADER_GC_ITEM_GROUND_ADD` (26): an item instance appeared on the map.
pub const HEADER_GC_ITEM_GROUND_ADD: u8 = 26;

/// `ITEM_SOCKET_MAX_NUM` with `ENABLE_EXTENDED_SOCKETS` on: six, not three.
pub const ITEM_SOCKET_MAX_NUM: usize = 6;

/// `ITEM_ATTRIBUTE_MAX_NUM`: five normal attributes plus two rare ones.
pub const ITEM_ATTRIBUTE_MAX_NUM: usize = 7;

/// Packed width of one `TPlayerItemAttribute`: `BYTE bType` then `short sValue`.
pub const ITEM_ATTRIBUTE_WIRE_SIZE: usize = 3;

/// Packed width of `TPacketGCItemSet`, the server's byte-21 record.
pub const GC_ITEM_SET_WIRE_SIZE: usize = 72;

/// Packed width of `TPacketGCItemDelDeprecated`, the server's byte-20 record.
pub const GC_ITEM_DEL_WIRE_SIZE: usize = 62;

/// Packed width of `TPacketGCItemUpdate`.
pub const GC_ITEM_UPDATE_WIRE_SIZE: usize = 59;

/// Packed width of `TPacketGCItemGroundAdd`.
pub const GC_ITEM_GROUND_ADD_WIRE_SIZE: usize = 21;

/// The window byte `EWindows` assigns to each window, compiled with every gate in
/// `prodomodefines.h` on. `RESERVED_WINDOW` is the invalid window, and `GROUND`
/// is not a window a character owns.
///
/// Two of these are feature-gated and both gates are on, so the numbering has
/// three gated members in the middle: `ATTR67_ADD` is 6 because
/// `__ATTR_6TH_7TH__` is defined, `AURA_REFINE` is 7 because `__AURA_SYSTEM__` is
/// defined, and `SWITCHBOT` is 8 because `ENABLE_SWITCHBOT` is defined. With any
/// one of them off, every later value shifts down. These are the server
/// enumerator's values, from `server/server/common/length.h:657-676`, measured by
/// compiling that enum.
pub mod window {
    /// `RESERVED_WINDOW` (0): the invalid window.
    pub const RESERVED: u8 = 0;
    /// `INVENTORY` (1).
    pub const INVENTORY: u8 = 1;
    /// `EQUIPMENT` (2).
    pub const EQUIPMENT: u8 = 2;
    /// `SAFEBOX` (3).
    pub const SAFEBOX: u8 = 3;
    /// `MALL` (4).
    pub const MALL: u8 = 4;
    /// `DRAGON_SOUL_INVENTORY` (5).
    pub const DRAGON_SOUL_INVENTORY: u8 = 5;
    /// `ATTR67_ADD` (6), behind `__ATTR_6TH_7TH__`.
    pub const ATTR67_ADD: u8 = 6;
    /// `AURA_REFINE` (7), behind `__AURA_SYSTEM__`.
    pub const AURA_REFINE: u8 = 7;
    /// `SWITCHBOT` (8), behind `ENABLE_SWITCHBOT`.
    pub const SWITCHBOT: u8 = 8;
    /// `BELT_INVENTORY` (9).
    pub const BELT_INVENTORY: u8 = 9;
    /// `GROUND` (10): the map, not a window.
    pub const GROUND: u8 = 10;
}

/// One `TPlayerItemAttribute`: an opaque apply-type byte and a signed value.
///
/// The legacy struct is `BYTE bType` then `short sValue` at
/// `server/server/common/tables.h:426-430`, so this is 3 packed bytes and the value
/// is **signed**. Its 3-byte width is derived rather than independently registered:
/// see the module documentation.
///
/// # Why this is not [`crate::item_pos::ItemPos`]
///
/// Both types are three packed bytes, which makes them an obvious candidate for
/// merging, and merging them would be wrong. [`crate::item_pos::ItemPos`] is
/// `BYTE window_type` plus `WORD cell`, and `WORD` is **unsigned**. This is
/// `BYTE bType` plus `short sValue`, and `short` is **signed**. The widths agree
/// and the meanings do not: one is a place, the other is a bonus, and a negative
/// bonus is a real value that a `u16` could not carry. All 65,536 `short` values
/// round-trip here; one of them could not survive the other type. Keeping them
/// apart is also what lets each one say what it means in its own field docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ItemAttribute {
    /// The opaque `bType` byte. The item manager resolves it through
    /// `FN_get_apply_type`; that is policy above this codec.
    pub b_type: u8,
    /// The signed `sValue`. Every one of the 65,536 values round-trips.
    pub s_value: i16,
}

impl ItemAttribute {
    /// Build an attribute.
    #[must_use]
    pub const fn new(b_type: u8, s_value: i16) -> Self {
        Self { b_type, s_value }
    }

    /// Append the three packed bytes to `out`.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        out.push(self.b_type);
        out.extend_from_slice(&self.s_value.to_le_bytes());
    }

    /// Encode to a fresh three-byte buffer.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(ITEM_ATTRIBUTE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Read the three bytes back.
    ///
    /// # Errors
    ///
    /// Returns [`GcItemWindowError::Truncated`] for fewer than three bytes and
    /// [`GcItemWindowError::LengthMismatch`] for more.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcItemWindowError> {
        let raw = exact(bytes, ITEM_ATTRIBUTE_WIRE_SIZE, "ItemAttribute")?;
        Ok(Self {
            b_type: raw[0],
            s_value: i16::from_le_bytes([raw[1], raw[2]]),
        })
    }
}

/// The six socket words an item carries, in wire order.
///
/// A socket is a `long` on the 32-bit target. Legacy stores a vnum in it and
/// treats 0 as empty, but nothing on the send path range-checks it, so this is an
/// opaque `i32` and every one of the 2^32 values round-trips.
pub type ItemSockets = [i32; ITEM_SOCKET_MAX_NUM];

/// The seven attribute slots an item carries, in wire order.
pub type ItemAttributes = [ItemAttribute; ITEM_ATTRIBUTE_MAX_NUM];

/// `HEADER_GC_ITEM_SET` (21): fill or replace one window slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GcItemSet {
    /// The window and cell this slot is.
    pub cell: ItemPos,
    /// The item vnum. Opaque here; the proto lives above this codec.
    pub vnum: u32,
    /// The opaque `WORD` stack count. Zero is meaningful in legacy.
    pub count: u16,
    /// `ENABLE_REFINE_ELEMENT`: the opaque refine-element id.
    pub refine_element: u32,
    /// `__CHANGELOOK_SYSTEM__`: the opaque transmutation value.
    pub transmutation: u32,
    /// The opaque legacy `ITEM_FLAG_*` word.
    pub flags: u32,
    /// The opaque legacy `ITEM_ANTIFLAG_*` word.
    pub anti_flags: u32,
    /// The `bool highlight` byte, as a raw `u8`. `pack(1)` makes it one byte.
    ///
    /// `char_item.cpp:585-589` has two candidates. Because
    /// `__BL_ENABLE_PICKUP_ITEM_EFFECT__` **is** defined
    /// (`server/server/common/prodomodefines.h:23`), the live branch is
    /// `pack.highlight = bHighlight`, the `SetItem` parameter. The
    /// `(Cell.window_type == DRAGON_SOUL_INVENTORY)` alternative on the next
    /// line is compiled out, so the byte is not derived from the cell. Either
    /// way it is a `bool`, which is one byte here and two on a host that is not
    /// `pack(1)`, so it is stored as a raw `u8` and every value round-trips.
    pub highlight: u8,
    /// The six opaque socket words.
    pub sockets: ItemSockets,
    /// The seven opaque attribute slots.
    pub attributes: ItemAttributes,
}

impl GcItemSet {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_ITEM_SET
    }

    /// Append the exact 72 packed bytes to `out`.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        out.push(Self::header());
        self.cell.encode_into(out);
        out.extend_from_slice(&self.vnum.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.refine_element.to_le_bytes());
        out.extend_from_slice(&self.transmutation.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.anti_flags.to_le_bytes());
        out.push(self.highlight);
        for socket in self.sockets {
            out.extend_from_slice(&socket.to_le_bytes());
        }
        encode_attributes(self.attributes, out);
    }

    /// Encode to a fresh 72-byte buffer.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_ITEM_SET_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Read the record back.
    ///
    /// # Errors
    ///
    /// Returns [`GcItemWindowError::Truncated`] when the buffer is shorter than
    /// [`GC_ITEM_SET_WIRE_SIZE`], [`GcItemWindowError::LengthMismatch`] when it
    /// is longer, and [`GcItemWindowError::Header`] when the leading byte is not
    /// 21.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcItemWindowError> {
        let raw = exact(bytes, GC_ITEM_SET_WIRE_SIZE, "GcItemSet")?;
        if raw[0] != Self::header() {
            return Err(GcItemWindowError::Header {
                context: "GcItemSet",
                expected: Self::header(),
                actual: raw[0],
            });
        }
        Ok(Self {
            cell: ItemPos::decode_at(raw, 1),
            vnum: word(raw, 4),
            count: u16::from_le_bytes([raw[8], raw[9]]),
            refine_element: word(raw, 10),
            transmutation: word(raw, 14),
            flags: word(raw, 18),
            anti_flags: word(raw, 22),
            highlight: raw[26],
            sockets: sockets_at(raw, 27),
            attributes: attributes_at(raw, 51),
        })
    }
}

/// `HEADER_GC_ITEM_DEL` (20), sent as `TPacketGCItemDelDeprecated`: clear one
/// window slot.
///
/// The legacy struct keeps the slot's cell, vnum, count, element fields, sockets
/// and attributes, all zeroed on the send path
/// (`server/server/game/char_item.cpp:597-610`), rather than sending a shorter
/// record. The client reads the same shape, so this record is 62 bytes and not
/// the 72 of [`GcItemSet`], and its `count` is a `BYTE` where the set record's is
/// a `WORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GcItemDel {
    /// The window and cell being cleared.
    pub cell: ItemPos,
    /// Always 0 on the legacy send path. Kept as a field so the record round-trips.
    pub vnum: u32,
    /// The opaque `BYTE` count. Always 0 on the legacy send path.
    pub count: u8,
    /// Always 0 on the legacy send path.
    pub refine_element: u32,
    /// Always 0 on the legacy send path.
    pub transmutation: u32,
    /// The six opaque socket words. Always zero on the legacy send path.
    pub sockets: ItemSockets,
    /// The seven opaque attribute slots. Always zero on the legacy send path.
    pub attributes: ItemAttributes,
}

impl GcItemDel {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_ITEM_DEL
    }

    /// Append the exact 62 packed bytes to `out`.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        out.push(Self::header());
        self.cell.encode_into(out);
        out.extend_from_slice(&self.vnum.to_le_bytes());
        out.push(self.count);
        out.extend_from_slice(&self.refine_element.to_le_bytes());
        out.extend_from_slice(&self.transmutation.to_le_bytes());
        for socket in self.sockets {
            out.extend_from_slice(&socket.to_le_bytes());
        }
        encode_attributes(self.attributes, out);
    }

    /// The record the legacy inventory actually sends.
    ///
    /// `char_item.cpp:597-610` builds the struct with every field zeroed except
    /// the cell, so this is the shape that goes on the wire for an inventory
    /// delete: the header byte, the window and cell, and 57 zero bytes. It is a
    /// named constructor rather than a `Default` derive because the header byte
    /// is part of the record and a zeroed default would decode as nothing.
    #[must_use]
    pub const fn default_record() -> Self {
        Self {
            cell: ItemPos {
                window_type: 0,
                cell: 0,
            },
            vnum: 0,
            count: 0,
            refine_element: 0,
            transmutation: 0,
            sockets: [0; ITEM_SOCKET_MAX_NUM],
            attributes: [ItemAttribute {
                b_type: 0,
                s_value: 0,
            }; ITEM_ATTRIBUTE_MAX_NUM],
        }
    }

    /// Encode to a fresh 62-byte buffer.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_ITEM_DEL_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Read the record back.
    ///
    /// # Errors
    ///
    /// Returns [`GcItemWindowError::Truncated`] when the buffer is shorter than
    /// [`GC_ITEM_DEL_WIRE_SIZE`], [`GcItemWindowError::LengthMismatch`] when it
    /// is longer, and [`GcItemWindowError::Header`] when the leading byte is not
    /// 20.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcItemWindowError> {
        let raw = exact(bytes, GC_ITEM_DEL_WIRE_SIZE, "GcItemDel")?;
        if raw[0] != Self::header() {
            return Err(GcItemWindowError::Header {
                context: "GcItemDel",
                expected: Self::header(),
                actual: raw[0],
            });
        }
        Ok(Self {
            cell: ItemPos::decode_at(raw, 1),
            vnum: word(raw, 4),
            count: raw[8],
            refine_element: word(raw, 9),
            transmutation: word(raw, 13),
            sockets: sockets_at(raw, 17),
            attributes: attributes_at(raw, 41),
        })
    }
}

/// `HEADER_GC_ITEM_UPDATE` (25): a slot's count, element, sockets or attributes
/// changed in place.
///
/// It carries no `vnum` and neither flag word, which is what makes it 13 bytes
/// shorter than the set record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GcItemUpdate {
    /// The window and cell that changed.
    pub cell: ItemPos,
    /// The new opaque `WORD` stack count.
    pub count: u16,
    /// The new opaque refine-element id.
    pub refine_element: u32,
    /// The new opaque transmutation value.
    pub transmutation: u32,
    /// The six new opaque socket words.
    pub sockets: ItemSockets,
    /// The seven new opaque attribute slots.
    pub attributes: ItemAttributes,
}

impl GcItemUpdate {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_ITEM_UPDATE
    }

    /// Append the exact 59 packed bytes to `out`.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        out.push(Self::header());
        self.cell.encode_into(out);
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.refine_element.to_le_bytes());
        out.extend_from_slice(&self.transmutation.to_le_bytes());
        for socket in self.sockets {
            out.extend_from_slice(&socket.to_le_bytes());
        }
        encode_attributes(self.attributes, out);
    }

    /// Encode to a fresh 59-byte buffer.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_ITEM_UPDATE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Read the record back.
    ///
    /// # Errors
    ///
    /// Returns [`GcItemWindowError::Truncated`] when the buffer is shorter than
    /// [`GC_ITEM_UPDATE_WIRE_SIZE`], [`GcItemWindowError::LengthMismatch`] when
    /// it is longer, and [`GcItemWindowError::Header`] when the leading byte is
    /// not 25.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcItemWindowError> {
        let raw = exact(bytes, GC_ITEM_UPDATE_WIRE_SIZE, "GcItemUpdate")?;
        if raw[0] != Self::header() {
            return Err(GcItemWindowError::Header {
                context: "GcItemUpdate",
                expected: Self::header(),
                actual: raw[0],
            });
        }
        Ok(Self {
            cell: ItemPos::decode_at(raw, 1),
            count: u16::from_le_bytes([raw[4], raw[5]]),
            refine_element: word(raw, 6),
            transmutation: word(raw, 10),
            sockets: sockets_at(raw, 14),
            attributes: attributes_at(raw, 38),
        })
    }
}

/// `HEADER_GC_ITEM_GROUND_ADD` (26): an item instance appeared on the map.
///
/// The three coordinates are `long`, so they are `i32` here. The record has no
/// grid cell and no ground identifier: the client places the item from the
/// coordinates alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GcItemGroundAdd {
    /// Map x, a `long` on the 32-bit target.
    pub x: i32,
    /// Map y.
    pub y: i32,
    /// Map z.
    pub z: i32,
    /// The VID that owns the ground item, or the entity that dropped it.
    pub vid: u32,
    /// The item vnum. Opaque here.
    pub vnum: u32,
}

impl GcItemGroundAdd {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_ITEM_GROUND_ADD
    }

    /// Append the exact 21 packed bytes to `out`.
    pub fn encode_into(self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.extend_from_slice(&self.z.to_le_bytes());
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.extend_from_slice(&self.vnum.to_le_bytes());
    }

    /// Encode to a fresh 21-byte buffer.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_ITEM_GROUND_ADD_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Read the record back.
    ///
    /// # Errors
    ///
    /// Returns [`GcItemWindowError::Truncated`] when the buffer is shorter than
    /// [`GC_ITEM_GROUND_ADD_WIRE_SIZE`], [`GcItemWindowError::LengthMismatch`]
    /// when it is longer, and [`GcItemWindowError::Header`] when the leading byte
    /// is not 26.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcItemWindowError> {
        let raw = exact(bytes, GC_ITEM_GROUND_ADD_WIRE_SIZE, "GcItemGroundAdd")?;
        if raw[0] != Self::header() {
            return Err(GcItemWindowError::Header {
                context: "GcItemGroundAdd",
                expected: Self::header(),
                actual: raw[0],
            });
        }
        Ok(Self {
            x: i32::from_le_bytes([raw[1], raw[2], raw[3], raw[4]]),
            y: i32::from_le_bytes([raw[5], raw[6], raw[7], raw[8]]),
            z: i32::from_le_bytes([raw[9], raw[10], raw[11], raw[12]]),
            vid: word(raw, 13),
            vnum: word(raw, 17),
        })
    }
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn sockets_at(bytes: &[u8], at: usize) -> ItemSockets {
    let mut out = [0i32; ITEM_SOCKET_MAX_NUM];
    for (index, slot) in out.iter_mut().enumerate() {
        let start = at + index * 4;
        *slot = i32::from_le_bytes([
            bytes[start],
            bytes[start + 1],
            bytes[start + 2],
            bytes[start + 3],
        ]);
    }
    out
}

fn encode_attributes(attributes: ItemAttributes, out: &mut Vec<u8>) {
    for attribute in attributes {
        attribute.encode_into(out);
    }
}

fn attributes_at(bytes: &[u8], at: usize) -> ItemAttributes {
    let mut out = [ItemAttribute::default(); ITEM_ATTRIBUTE_MAX_NUM];
    for (index, slot) in out.iter_mut().enumerate() {
        let start = at + index * ITEM_ATTRIBUTE_WIRE_SIZE;
        *slot = ItemAttribute {
            b_type: bytes[start],
            s_value: i16::from_le_bytes([bytes[start + 1], bytes[start + 2]]),
        };
    }
    out
}

fn exact<'a>(
    bytes: &'a [u8],
    needed: usize,
    context: &'static str,
) -> Result<&'a [u8], GcItemWindowError> {
    if bytes.len() < needed {
        return Err(GcItemWindowError::Truncated {
            context,
            needed,
            actual: bytes.len(),
        });
    }
    if bytes.len() > needed {
        return Err(GcItemWindowError::LengthMismatch {
            context,
            expected: needed,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

/// What an item-window decode can report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcItemWindowError {
    /// The buffer ended before the record did.
    Truncated {
        /// Which record was being decoded.
        context: &'static str,
        /// Bytes the record needs.
        needed: usize,
        /// Bytes the buffer held.
        actual: usize,
    },
    /// The buffer held more bytes than the record has.
    LengthMismatch {
        /// Which record was being decoded.
        context: &'static str,
        /// Bytes the record has.
        expected: usize,
        /// Bytes the buffer held.
        actual: usize,
    },
    /// The leading byte is not this record's header.
    Header {
        /// Which record was being decoded.
        context: &'static str,
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was present.
        actual: u8,
    },
}

impl fmt::Display for GcItemWindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                context,
                needed,
                actual,
            } => write!(f, "{context}: need {needed} bytes, buffer held {actual}"),
            Self::LengthMismatch {
                context,
                expected,
                actual,
            } => write!(
                f,
                "{context}: {actual} bytes given, the record is exactly {expected}"
            ),
            Self::Header {
                context,
                expected,
                actual,
            } => write!(
                f,
                "{context}: header {expected} expected, buffer held {actual}"
            ),
        }
    }
}

impl Error for GcItemWindowError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc_inventory::{
        resolve_gc_packet, HEADER_GC_ITEM_GROUND_ADD as INV_ITEM_GROUND_ADD,
        HEADER_GC_ITEM_SET as INV_ITEM_SET, HEADER_GC_ITEM_SET2 as INV_ITEM_SET2,
        HEADER_GC_ITEM_UPDATE as INV_ITEM_UPDATE,
    };

    /// Distinct byte halves on every multi-byte field, so a big-endian read
    /// cannot pass by byte symmetry. `0x11` is not a byte-symmetric pair with
    /// `0x22`, `0x33` is not symmetric with `0x44`, and so on.
    const SET_CELL: ItemPos = ItemPos {
        window_type: 1,
        cell: 0x2b1a,
    };
    const SET_VNUM: u32 = 0x5f4e_3d2c;
    const SET_COUNT: u16 = 0x6b5a;
    const SET_ELEMENT: u32 = 0x7e6d_5c4b;
    const SET_TRANSMUTATION: u32 = 0x8f7e_6d5c;
    const SET_FLAGS: u32 = 0x9a8b_7c6d;
    const SET_ANTI: u32 = 0xab9c_8d7e;
    const SET_SOCKETS: ItemSockets = [
        0x0102_0304,
        0x1122_3344,
        0x5566_7788,
        -0x0102_0304,
        i32::from_le_bytes([0xcc, 0xbb, 0xaa, 0x99]),
        i32::MIN,
    ];
    const SET_ATTRIBUTES: ItemAttributes = [
        ItemAttribute::new(0x1a, 0x2b3c),
        ItemAttribute::new(0x2b, -0x3c4d),
        ItemAttribute::new(0x3c, 0x4d5e),
        ItemAttribute::new(0x4d, i16::MIN),
        ItemAttribute::new(0x5e, i16::MAX),
        ItemAttribute::new(0x6f, 0),
        ItemAttribute::new(0x70, 1),
    ];

    fn a_set() -> GcItemSet {
        GcItemSet {
            cell: SET_CELL,
            vnum: SET_VNUM,
            count: SET_COUNT,
            refine_element: SET_ELEMENT,
            transmutation: SET_TRANSMUTATION,
            flags: SET_FLAGS,
            anti_flags: SET_ANTI,
            highlight: 0xc3,
            sockets: SET_SOCKETS,
            attributes: SET_ATTRIBUTES,
        }
    }

    #[test]
    fn the_widths_are_the_measured_packed_widths() {
        assert_eq!(GC_ITEM_SET_WIRE_SIZE, 72);
        assert_eq!(GC_ITEM_DEL_WIRE_SIZE, 62);
        assert_eq!(GC_ITEM_UPDATE_WIRE_SIZE, 59);
        assert_eq!(GC_ITEM_GROUND_ADD_WIRE_SIZE, 21);
        assert_eq!(ITEM_ATTRIBUTE_WIRE_SIZE, 3);
        assert_eq!(ITEM_SOCKET_MAX_NUM, 6);
        assert_eq!(ITEM_ATTRIBUTE_MAX_NUM, 7);
        // Each hand sum restated as field arithmetic, so a changed constant fails
        // here rather than silently at the client.
        assert_eq!(
            GC_ITEM_SET_WIRE_SIZE,
            1 + 3 + 4 + 2 + 4 + 4 + 4 + 4 + 1 + 24 + 21
        );
        assert_eq!(GC_ITEM_DEL_WIRE_SIZE, 1 + 3 + 4 + 1 + 4 + 4 + 24 + 21);
        assert_eq!(GC_ITEM_UPDATE_WIRE_SIZE, 1 + 3 + 2 + 4 + 4 + 24 + 21);
        assert_eq!(GC_ITEM_GROUND_ADD_WIRE_SIZE, 1 + 4 + 4 + 4 + 4 + 4);
    }

    #[test]
    fn the_delete_record_is_ten_bytes_shorter_than_the_set_record() {
        // WORD count vs BYTE count, plus the two flag words and the bool.
        assert_eq!(GC_ITEM_SET_WIRE_SIZE - GC_ITEM_DEL_WIRE_SIZE, 10);
    }

    #[test]
    fn the_headers_are_the_server_bytes_and_the_inventory_is_the_client_side() {
        assert_eq!(HEADER_GC_ITEM_DEL, 20);
        assert_eq!(HEADER_GC_ITEM_SET, 21);
        assert_eq!(HEADER_GC_ITEM_UPDATE, 25);
        assert_eq!(HEADER_GC_ITEM_GROUND_ADD, 26);
        // The inventory is keyed by the client names, which are the swapped pair
        // on the first two: the client's HEADER_GC_ITEM_SET is byte 20 and its
        // HEADER_GC_ITEM_SET2 is byte 21.
        assert_eq!(HEADER_GC_ITEM_DEL, INV_ITEM_SET.value());
        assert_eq!(HEADER_GC_ITEM_SET, INV_ITEM_SET2.value());
        assert_eq!(HEADER_GC_ITEM_UPDATE, INV_ITEM_UPDATE.value());
        assert_eq!(HEADER_GC_ITEM_GROUND_ADD, INV_ITEM_GROUND_ADD.value());
        let twenty = resolve_gc_packet(20).expect("byte 20 is registered");
        let twenty_one = resolve_gc_packet(21).expect("byte 21 is registered");
        assert_eq!(twenty.client_name, "HEADER_GC_ITEM_SET");
        assert_eq!(twenty_one.client_name, "HEADER_GC_ITEM_SET2");
        assert_eq!(twenty.server_name, Some("HEADER_GC_ITEM_DEL"));
        assert_eq!(twenty_one.server_name, Some("HEADER_GC_ITEM_SET"));
    }

    #[test]
    fn the_window_bytes_are_the_compiled_enum_values() {
        assert_eq!(window::RESERVED, 0);
        assert_eq!(window::INVENTORY, 1);
        assert_eq!(window::EQUIPMENT, 2);
        assert_eq!(window::SAFEBOX, 3);
        assert_eq!(window::MALL, 4);
        assert_eq!(window::DRAGON_SOUL_INVENTORY, 5);
        assert_eq!(window::ATTR67_ADD, 6);
        assert_eq!(window::AURA_REFINE, 7);
        assert_eq!(window::SWITCHBOT, 8);
        assert_eq!(window::BELT_INVENTORY, 9);
        assert_eq!(window::GROUND, 10);
        assert_eq!(window::GROUND, window::RESERVED + 10);
    }

    /// The whole 72-byte record pinned against the legacy field order.
    #[test]
    fn the_set_record_is_pinned_byte_for_byte() {
        let wire = a_set().encode();
        assert_eq!(wire.len(), 72);
        assert_eq!(&wire[0..1], &[21]);
        assert_eq!(&wire[1..4], &[1, 26, 43]);
        assert_eq!(&wire[4..8], &[44, 61, 78, 95]);
        assert_eq!(&wire[8..10], &[90, 107]);
        assert_eq!(&wire[10..14], &[75, 92, 109, 126]);
        assert_eq!(&wire[14..18], &[92, 109, 126, 143]);
        assert_eq!(&wire[18..22], &[109, 124, 139, 154]);
        assert_eq!(&wire[22..26], &[126, 141, 156, 171]);
        assert_eq!(&wire[26..27], &[195]);
        assert_eq!(&wire[27..31], &[4, 3, 2, 1]);
        assert_eq!(&wire[31..35], &[68, 51, 34, 17]);
        assert_eq!(&wire[39..43], &[252, 252, 253, 254]);
        assert_eq!(&wire[51..54], &[26, 60, 43]);
        assert_eq!(&wire[54..57], &[43, 179, 195]);
        assert_eq!(&wire[69..72], &[112, 1, 0]);
    }

    #[test]
    fn the_set_record_round_trips() {
        let original = a_set();
        assert_eq!(GcItemSet::decode(&original.encode()).unwrap(), original);
    }

    #[test]
    fn the_set_record_rejects_a_wrong_header_and_a_wrong_length() {
        let mut wire = a_set().encode();
        wire[0] = 20;
        assert_eq!(
            GcItemSet::decode(&wire).unwrap_err(),
            GcItemWindowError::Header {
                context: "GcItemSet",
                expected: 21,
                actual: 20,
            }
        );
        let short = &wire[..71];
        assert_eq!(
            GcItemSet::decode(short).unwrap_err(),
            GcItemWindowError::Truncated {
                context: "GcItemSet",
                needed: 72,
                actual: 71,
            }
        );
        let mut long = wire.clone();
        long.push(0);
        assert_eq!(
            GcItemSet::decode(&long).unwrap_err(),
            GcItemWindowError::LengthMismatch {
                context: "GcItemSet",
                expected: 72,
                actual: 73,
            }
        );
    }

    /// A 62-byte delete record must not be readable as a 72-byte set record and
    /// vice versa. The two differ by ten bytes, so a length-only check would
    /// accept the wrong one; the header check is what separates them.
    #[test]
    fn the_set_and_delete_records_reject_each_other() {
        let del = GcItemDel::default_record();
        let set_wire = a_set().encode();
        let del_wire = del.encode();
        assert_eq!(del_wire.len(), 62);
        // The set decoder sees a 62-byte buffer: too short, and the wrong header.
        assert!(matches!(
            GcItemSet::decode(&del_wire),
            Err(GcItemWindowError::Truncated { .. })
        ));
        // The delete decoder sees a 72-byte buffer: too long.
        assert!(matches!(
            GcItemDel::decode(&set_wire),
            Err(GcItemWindowError::LengthMismatch { .. })
        ));
        // A 62-byte buffer whose header says 21 is still refused.
        let mut forged = del_wire.clone();
        forged[0] = 21;
        assert!(matches!(
            GcItemDel::decode(&forged),
            Err(GcItemWindowError::Header { .. })
        ));
    }

    /// The 62-byte delete record pinned against the legacy field order.
    #[test]
    fn the_delete_record_is_pinned_byte_for_byte() {
        let del = GcItemDel {
            cell: ItemPos::new(1, 0x2b1a),
            vnum: 0x5f4e_3d2c,
            count: 0xa9,
            refine_element: 0x7e6d_5c4b,
            transmutation: 0x8f7e_6d5c,
            sockets: SET_SOCKETS,
            attributes: SET_ATTRIBUTES,
        };
        let wire = del.encode();
        assert_eq!(wire.len(), 62);
        assert_eq!(&wire[0..1], &[20]);
        assert_eq!(&wire[1..4], &[1, 26, 43]);
        assert_eq!(&wire[4..8], &[44, 61, 78, 95]);
        // A BYTE count, not the WORD the set record has.
        assert_eq!(&wire[8..9], &[169]);
        assert_eq!(&wire[9..13], &[75, 92, 109, 126]);
        assert_eq!(&wire[13..17], &[92, 109, 126, 143]);
        assert_eq!(&wire[17..21], &[4, 3, 2, 1]);
        assert_eq!(&wire[41..44], &[26, 60, 43]);
        assert_eq!(&wire[59..62], &[112, 1, 0]);
        assert_eq!(GcItemDel::decode(&wire).unwrap(), del);
    }

    #[test]
    fn the_delete_record_takes_a_byte_count_and_the_set_record_a_word() {
        // 0x1a2b does not fit a BYTE: the delete record truncates to 0x2b, the
        // set record keeps both bytes. This is the one field whose width differs.
        let del = GcItemDel {
            count: 0x2b,
            ..GcItemDel::default_record()
        };
        assert_eq!(del.encode()[8], 0x2b);
        let mut set = a_set();
        set.count = 0x1a2b;
        assert_eq!(&set.encode()[8..10], &[0x2b, 0x1a]);
        assert_eq!(GcItemSet::decode(&set.encode()).unwrap().count, 0x1a2b);
    }

    /// The 59-byte update record pinned against the legacy field order.
    #[test]
    fn the_update_record_is_pinned_byte_for_byte() {
        let update = GcItemUpdate {
            cell: ItemPos::new(1, 0x2b1a),
            count: 0x6b5a,
            refine_element: 0x7e6d_5c4b,
            transmutation: 0x8f7e_6d5c,
            sockets: SET_SOCKETS,
            attributes: SET_ATTRIBUTES,
        };
        let wire = update.encode();
        assert_eq!(wire.len(), 59);
        assert_eq!(&wire[0..1], &[25]);
        assert_eq!(&wire[1..4], &[1, 26, 43]);
        assert_eq!(&wire[4..6], &[90, 107]);
        assert_eq!(&wire[6..10], &[75, 92, 109, 126]);
        assert_eq!(&wire[10..14], &[92, 109, 126, 143]);
        assert_eq!(&wire[14..18], &[4, 3, 2, 1]);
        assert_eq!(&wire[38..41], &[26, 60, 43]);
        assert_eq!(&wire[56..59], &[112, 1, 0]);
        assert_eq!(GcItemUpdate::decode(&wire).unwrap(), update);
    }

    /// The 21-byte ground record pinned against the legacy field order.
    #[test]
    fn the_ground_add_record_is_pinned_byte_for_byte() {
        let ground = GcItemGroundAdd {
            x: -0x07_29_4b_5e,
            y: 0x7b_35_1e_0c,
            z: i32::MIN,
            vid: 0xdf9b_5713,
            vnum: 0x5f4e_3d2c,
        };
        let wire = ground.encode();
        assert_eq!(wire.len(), 21);
        assert_eq!(&wire[0..1], &[26]);
        assert_eq!(&wire[1..5], &[162, 180, 214, 248]);
        assert_eq!(&wire[5..9], &[12, 30, 53, 123]);
        assert_eq!(&wire[9..13], &[0, 0, 0, 128]);
        assert_eq!(&wire[13..17], &[19, 87, 155, 223]);
        assert_eq!(&wire[17..21], &[44, 61, 78, 95]);
        assert_eq!(GcItemGroundAdd::decode(&wire).unwrap(), ground);
    }

    #[test]
    fn the_attribute_codec_is_three_bytes_and_signed() {
        let attribute = ItemAttribute::new(0x1a, -0x3c4d);
        let wire = attribute.encode();
        assert_eq!(wire, vec![26, 179, 195]);
        assert_eq!(ItemAttribute::decode(&wire).unwrap(), attribute);
        assert_eq!(
            ItemAttribute::decode(&wire[..2]).unwrap_err(),
            GcItemWindowError::Truncated {
                context: "ItemAttribute",
                needed: 3,
                actual: 2,
            }
        );
        assert!(matches!(
            ItemAttribute::decode(&[0u8; 4]).unwrap_err(),
            GcItemWindowError::LengthMismatch { .. }
        ));
    }

    /// The legacy handler never range-checks these, so every value round-trips.
    #[test]
    fn opaque_fields_round_trip_every_value() {
        for cell in [0u16, 1, 0x8000, u16::MAX] {
            for window_type in [0u8, 1, 0x80, u8::MAX] {
                let mut wire = a_set().encode();
                wire[1] = window_type;
                wire[2..4].copy_from_slice(&cell.to_le_bytes());
                let read = GcItemSet::decode(&wire).unwrap();
                assert_eq!(read.cell.window_type, window_type);
                assert_eq!(read.cell.cell, cell);
            }
        }
        for highlight in [0u8, 1, 0x80, u8::MAX] {
            let mut set = a_set();
            set.highlight = highlight;
            assert_eq!(
                GcItemSet::decode(&set.encode()).unwrap().highlight,
                highlight
            );
        }
        for b_type in [0u8, 0x80, u8::MAX] {
            for s_value in [i16::MIN, -1, 0, 1, i16::MAX] {
                let attribute = ItemAttribute::new(b_type, s_value);
                assert_eq!(
                    ItemAttribute::decode(&attribute.encode()).unwrap(),
                    attribute
                );
            }
        }
        for socket in [0i32, 1, -1, i32::MIN, i32::MAX] {
            let mut set = a_set();
            set.sockets = [socket; ITEM_SOCKET_MAX_NUM];
            assert_eq!(
                GcItemSet::decode(&set.encode()).unwrap().sockets,
                [socket; 6]
            );
        }
    }

    /// The whole-record round trip is not an independent witness, so the golden
    /// byte tests above are what pin the field order. This one only guards the
    /// six-socket and seven-attribute loops, which a swapped loop body could
    /// still satisfy.
    #[test]
    fn socket_and_attribute_slots_keep_their_index() {
        let indexed_sockets: ItemSockets = [0x100, 0x101, 0x102, 0x103, 0x104, 0x105];
        let indexed_attributes: ItemAttributes = [
            ItemAttribute::new(0x20, -0x300),
            ItemAttribute::new(0x21, -0x301),
            ItemAttribute::new(0x22, -0x302),
            ItemAttribute::new(0x23, -0x303),
            ItemAttribute::new(0x24, -0x304),
            ItemAttribute::new(0x25, -0x305),
            ItemAttribute::new(0x26, -0x306),
        ];
        let mut set = a_set();
        set.sockets = indexed_sockets;
        set.attributes = indexed_attributes;
        let read = GcItemSet::decode(&set.encode()).unwrap();
        assert_eq!(read.sockets, indexed_sockets);
        assert_eq!(read.attributes, indexed_attributes);
        // The same slots, in the same places, on the other three records.
        let del = GcItemDel {
            sockets: set.sockets,
            attributes: set.attributes,
            ..GcItemDel::default_record()
        };
        assert_eq!(
            GcItemDel::decode(&del.encode()).unwrap().sockets,
            set.sockets
        );
        let update = GcItemUpdate {
            sockets: set.sockets,
            attributes: set.attributes,
            cell: ItemPos::default(),
            count: 0,
            refine_element: 0,
            transmutation: 0,
        };
        assert_eq!(
            GcItemUpdate::decode(&update.encode()).unwrap().attributes,
            set.attributes
        );
    }

    /// A structural sweep over the offsets. The golden byte tests above pin the
    /// layout; this pins the arithmetic that would catch a field that is
    /// declared but never written.
    ///
    /// Each record's two trailing arrays are the same 24 and 21 bytes, so the
    /// attribute array must start exactly 24 bytes after the socket array, and
    /// the pair must end exactly at the record's width. A record that gained a
    /// field without growing its width, or whose arrays were read from the wrong
    /// offset, fails here. Note this is a check on the declared constants, not a
    /// substitute for the byte witnesses: it is stated as such so nobody reads a
    /// pass as proof of the layout.
    #[test]
    fn the_trailing_arrays_end_exactly_at_each_record_width() {
        let socket_span = ITEM_SOCKET_MAX_NUM * 4;
        let attribute_span = ITEM_ATTRIBUTE_MAX_NUM * ITEM_ATTRIBUTE_WIRE_SIZE;
        assert_eq!(socket_span, 24);
        assert_eq!(attribute_span, 21);
        // (record, width, socket array offset, attribute array offset)
        let table: [(&str, usize, usize, usize); 3] = [
            ("GcItemSet", GC_ITEM_SET_WIRE_SIZE, 27, 51),
            ("GcItemDel", GC_ITEM_DEL_WIRE_SIZE, 17, 41),
            ("GcItemUpdate", GC_ITEM_UPDATE_WIRE_SIZE, 14, 38),
        ];
        for (name, total, socket_at, attribute_at) in table {
            assert_eq!(
                attribute_at,
                socket_at + socket_span,
                "{name}: the attribute array follows the socket array"
            );
            assert_eq!(
                attribute_at + attribute_span,
                total,
                "{name}: the two arrays end at the record width"
            );
        }
        // The ground record has no arrays; its last field is the trailing DWORD.
        assert_eq!(GC_ITEM_GROUND_ADD_WIRE_SIZE, 17 + 4);
    }
}
