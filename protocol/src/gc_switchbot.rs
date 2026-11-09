//! `HEADER_GC_SWITCHBOT` (171): the switchbot's answers.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:3415-3422` declares the head every switchbot send
//! writes:
//!
//! ```ignore
//! struct TPacketGCSwitchbot
//! {
//!     BYTE header;
//!     int size;
//!     BYTE subheader;
//!     BYTE slot;
//! };
//! ```
//!
//! The subheaders are the enum at `packet.h:3409-3413`: `UPDATE` (0), `UPDATE_ITEM`
//! (1) and `SEND_ATTRIBUTE_INFORMATION` (2). `SendSwitchbotUpdate`
//! (`G/refactorized_switchbot.cpp:712-742`) sends the update: `size` counts the whole
//! record and the body is the `TSwitchbotTable`. `SendItemUpdate` (`:435-458`) sends
//! the item copy after each change, and `SendItemAttributeInformations` (`:667-710`)
//! sends the enter-game attribute table.
//!
//! # Width
//!
//! The head is 1 + 4 + 1 + 1 = 7 bytes. The update body is the
//! `TSwitchbotTable` (`C/tables.h:2218-2237`): `player_id` (4), `active` (5),
//! `finished` (5), `items` (5 x 4 = 20) and the alternatives (5 slots x 2 x 5
//! attributes x 3 bytes = 150), 184 bytes, so the update is 191. The item body is
//! `TSwitchbotUpdateItem` (`G/packet.h:3423-3431`): the slot, the vnum as one byte,
//! the count, the six sockets (`ENABLE_EXTENDED_SOCKETS` is on,
//! `C/prodomodefines.h:76`) and the seven attributes, 1 + 1 + 1 + 24 + 21 = 48, so
//! the item record is 55. Each attribute entry is `TSwitchbottAttributeTable`
//! (`C/tables.h:2239-2244`): the set byte, the apply number and the max value,
//! 1 + 4 + 4 = 9; the enter-game record is the head plus one entry per apply the set
//! table allows, or the bare head when none is allowed.

use crate::gc_item_window::{ItemAttribute, ItemAttributes, ItemSockets};

/// `HEADER_GC_SWITCHBOT`.
pub const HEADER: u8 = 171;

/// `SUBHEADER_GC_SWITCHBOT_UPDATE`.
pub const SUB_UPDATE: u8 = 0;

/// `SUBHEADER_GC_SWITCHBOT_UPDATE_ITEM`.
pub const SUB_UPDATE_ITEM: u8 = 1;

/// `SUBHEADER_GC_SWITCHBOT_SEND_ATTRIBUTE_INFORMATION`.
pub const SUB_ATTR_INFO: u8 = 2;

/// The update's width: the head and the `TSwitchbotTable`.
pub const UPDATE_LEN: usize = 191;

/// The item record's width: the head and the `TSwitchbotUpdateItem`.
pub const UPDATE_ITEM_LEN: usize = 55;

/// The bare head's width.
pub const HEAD_LEN: usize = 7;

/// One attribute entry's width: the set, the apply number, the max value.
pub const ATTR_ENTRY_LEN: usize = 9;

/// The switchbot head: the header, the whole-record size, the subheader, the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcSwitchbotHead {
    /// Legacy `subheader`.
    pub subheader: u8,
    /// Legacy `slot`, always written, always zero on the update and attr-info sends.
    pub slot: u8,
}

impl GcSwitchbotHead {
    /// Append the head for a record of `size` bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>, size: u32) {
        out.push(HEADER);
        out.extend_from_slice(&size.to_le_bytes());
        out.push(self.subheader);
        out.push(self.slot);
    }
}

/// One switchbot table, as `SendSwitchbotUpdate` writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcSwitchbotTable {
    /// Legacy `player_id`.
    pub player_id: u32,
    /// Legacy `active`, one byte per slot.
    pub active: [bool; 5],
    /// Legacy `finished`, one byte per slot.
    pub finished: [bool; 5],
    /// Legacy `items`, one item id per slot.
    pub items: [u32; 5],
    /// Legacy `alternatives`: per slot, per alternative, the five wanted attributes.
    pub alternatives: [[[ItemAttribute; 5]; 2]; 5],
}

/// The update record: the head and the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcSwitchbotUpdate {
    /// The table the record carries.
    pub table: GcSwitchbotTable,
}

impl GcSwitchbotUpdate {
    /// Encode to a fresh 191-byte buffer.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(UPDATE_LEN);
        GcSwitchbotHead {
            subheader: SUB_UPDATE,
            slot: 0,
        }
        .encode_into(&mut out, u32::try_from(UPDATE_LEN).unwrap_or(u32::MAX));
        out.extend_from_slice(&self.table.player_id.to_le_bytes());
        for active in self.table.active {
            out.push(u8::from(active));
        }
        for finished in self.table.finished {
            out.push(u8::from(finished));
        }
        for item in self.table.items {
            out.extend_from_slice(&item.to_le_bytes());
        }
        for slot in &self.table.alternatives {
            for alternative in slot {
                for attribute in alternative {
                    out.push(attribute.b_type);
                    out.extend_from_slice(&attribute.s_value.to_le_bytes());
                }
            }
        }
        out
    }
}

/// The item record: the head and the changed item's copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcSwitchbotItem {
    /// Legacy `slot`, carried in the head too.
    pub slot: u8,
    /// Legacy `vnum`, one byte: the item vnum's low byte.
    pub vnum: u8,
    /// Legacy `count`.
    pub count: u8,
    /// Legacy `alSockets`.
    pub sockets: ItemSockets,
    /// Legacy `aAttr`.
    pub attributes: ItemAttributes,
}

impl GcSwitchbotItem {
    /// Encode to a fresh 43-byte buffer.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(UPDATE_ITEM_LEN);
        GcSwitchbotHead {
            subheader: SUB_UPDATE_ITEM,
            slot: self.slot,
        }
        .encode_into(&mut out, u32::try_from(UPDATE_ITEM_LEN).unwrap_or(u32::MAX));
        out.push(self.slot);
        out.push(self.vnum);
        out.push(self.count);
        for socket in self.sockets {
            out.extend_from_slice(&socket.to_le_bytes());
        }
        for attribute in &self.attributes {
            out.push(attribute.b_type);
            out.extend_from_slice(&attribute.s_value.to_le_bytes());
        }
        out
    }
}

/// One attribute entry: the set, the apply number, the max value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcSwitchbotAttrEntry {
    /// Legacy `attribute_set`.
    pub attribute_set: u8,
    /// Legacy `apply_num`.
    pub apply_num: i32,
    /// Legacy `max_value`.
    pub max_value: i32,
}

impl GcSwitchbotAttrEntry {
    /// Append the entry to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.attribute_set);
        out.extend_from_slice(&self.apply_num.to_le_bytes());
        out.extend_from_slice(&self.max_value.to_le_bytes());
    }
}

/// The attribute-information record: the head and the entries, or the bare head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcSwitchbotAttrInfo {
    /// The entries, in set-then-apply order.
    pub entries: Vec<GcSwitchbotAttrEntry>,
}

impl GcSwitchbotAttrInfo {
    /// Encode to a fresh buffer.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let size = HEAD_LEN + self.entries.len() * ATTR_ENTRY_LEN;
        let mut out = Vec::with_capacity(size);
        GcSwitchbotHead {
            subheader: SUB_ATTR_INFO,
            slot: 0,
        }
        .encode_into(&mut out, u32::try_from(size).unwrap_or(u32::MAX));
        for entry in &self.entries {
            entry.encode_into(&mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden bytes in legacy field order: the head (header, size, sub, slot), the
    /// player id, the actives, the finisheds, the items, and the first wanted
    /// attribute's distinct halves.
    #[test]
    fn the_update_encodes_in_legacy_field_order() {
        let mut alternatives = [[[ItemAttribute::new(0, 0); 5]; 2]; 5];
        alternatives[0][0][0] = ItemAttribute::new(0x12, 0x3456);
        let record = GcSwitchbotUpdate {
            table: GcSwitchbotTable {
                player_id: 0x0102_0304,
                active: [true, false, false, false, false],
                finished: [false, true, false, false, false],
                items: [0x1111_1111, 0, 0, 0, 0],
                alternatives,
            },
        }
        .encode();
        assert_eq!(record.len(), UPDATE_LEN);
        assert_eq!(&record[0..7], &[171, 191, 0, 0, 0, 0, 0]);
        assert_eq!(&record[7..11], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&record[11..16], &[1, 0, 0, 0, 0]);
        assert_eq!(&record[16..21], &[0, 1, 0, 0, 0]);
        assert_eq!(&record[21..25], &[0x11, 0x11, 0x11, 0x11]);
        assert_eq!(&record[41..44], &[0x12, 0x56, 0x34]);
    }

    /// Golden bytes: the head's slot echoes the body's, the vnum is the low byte,
    /// and the sockets and attributes follow in order.
    #[test]
    fn the_item_record_echoes_the_slot_and_truncates_the_vnum() {
        let mut attributes = [ItemAttribute::new(0, 0); 7];
        attributes[0] = ItemAttribute::new(0x21, 0x6543);
        let record = GcSwitchbotItem {
            slot: 3,
            vnum: 0x9au8,
            count: 2,
            sockets: [0x0102_0304, 0, 0, 0, 0, 0],
            attributes,
        }
        .encode();
        assert_eq!(record.len(), UPDATE_ITEM_LEN);
        assert_eq!(&record[0..7], &[171, 55, 0, 0, 0, 1, 3]);
        assert_eq!(&record[7..10], &[3, 0x9a, 2]);
        assert_eq!(&record[10..14], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&record[34..37], &[0x21, 0x43, 0x65]);
    }

    /// Golden bytes: the size counts the entries, each set/apply/max in order, and
    /// the empty table is the bare head.
    #[test]
    fn the_attr_info_counts_its_entries() {
        let record = GcSwitchbotAttrInfo {
            entries: vec![
                GcSwitchbotAttrEntry {
                    attribute_set: 2,
                    apply_num: 0x0102_0304,
                    max_value: 0x1122_3344,
                },
                GcSwitchbotAttrEntry {
                    attribute_set: 5,
                    apply_num: 1,
                    max_value: 2,
                },
            ],
        }
        .encode();
        assert_eq!(record.len(), HEAD_LEN + 2 * ATTR_ENTRY_LEN);
        assert_eq!(&record[0..7], &[171, 25, 0, 0, 0, 2, 0]);
        assert_eq!(
            &record[7..16],
            &[2, 0x04, 0x03, 0x02, 0x01, 0x44, 0x33, 0x22, 0x11]
        );
        assert_eq!(&record[16..25], &[5, 1, 0, 0, 0, 2, 0, 0, 0]);
        let bare = GcSwitchbotAttrInfo { entries: vec![] }.encode();
        assert_eq!(bare, vec![171, 7, 0, 0, 0, 2, 0]);
    }
}
