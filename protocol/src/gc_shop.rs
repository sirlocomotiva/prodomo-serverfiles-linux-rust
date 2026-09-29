//! `HEADER_GC_SHOP` (38): an NPC shop's window, and the answers to a buy.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:1622-1627` declares the record every shop send
//! starts with:
//!
//! ```ignore
//! typedef struct packet_shop
//! {
//!     BYTE        header;
//!     WORD        size;
//!     BYTE        subheader;
//! } TPacketGCShop;
//! ```
//!
//! The subheaders are `EPacketShopSubHeaders` at `packet.h:1536-1554`, numbered
//! from 0 with `ENABLE_RENEWAL_SHOPEX` on, so the two last ones exist.
//!
//! `CShop::AddGuest` (`server/server/game/shop.cpp:813-905`) opens the window. It
//! writes the four header bytes with `size = sizeof(pack) + sizeof(pack2)` and
//! `subheader = SHOP_SUBHEADER_GC_START`, then `TPacketGCShopStart`
//! (`packet.h:1584-1591`): the keeper's VID and forty `packet_shop_item`s. It
//! zeroes the whole body first, then sets `vnum`, `price`, `count` and
//! `price_type = SHOPEX_GOLD` in every one of the forty slots, empty or not.
//!
//! Every other send is the four header bytes alone, with `size` 4:
//! `CShopManager::StopShopping` sends `SHOP_SUBHEADER_GC_END`, and
//! `CShopManager::Buy` sends the refusal `CShop::Buy` returned. A buy that
//! succeeds sends no shop record.
//!
//! # Width
//!
//! `packet_shop_item` (`packet.h:1556-1582`) with `ENABLE_REMOVE_LIMIT_GOLD`,
//! `ENABLE_REFINE_ELEMENT`, `__CHANGELOOK_SYSTEM__` and `ENABLE_RENEWAL_SHOPEX`
//! on (`server/server/common/prodomodefines.h`) is
//! 4 + 8 + 2 + 1 + 4 + 6 × 4 + 7 × 3 + 4 + 1 + 4 = 73 packed bytes, because
//! `packet.h` is inside `#pragma pack(1)`. `ENABLE_DISTANCE_SHOPPING` is off, so
//! the start body has no trailing flag and the whole window is
//! 4 + 4 + 40 × 73 = 2928 bytes.

use std::fmt;

use crate::gc_inventory::HEADER_GC_SHOP;
use crate::gc_item_window::{
    ItemAttribute, ItemAttributes, ItemSockets, ITEM_ATTRIBUTE_MAX_NUM, ITEM_SOCKET_MAX_NUM,
};

/// `sizeof(TPacketGCShop)`: header, size and subheader.
pub const GC_SHOP_WIRE_SIZE: usize = 4;

/// `sizeof(packet_shop_item)`.
pub const GC_SHOP_ITEM_WIRE_SIZE: usize = 73;

/// `SHOP_HOST_ITEM_MAX_NUM`: the slots a shop window shows.
pub const SHOP_HOST_ITEM_MAX_NUM: usize = 40;

/// The whole start record: the header, the keeper's VID and forty items.
pub const GC_SHOP_START_WIRE_SIZE: usize =
    GC_SHOP_WIRE_SIZE + 4 + SHOP_HOST_ITEM_MAX_NUM * GC_SHOP_ITEM_WIRE_SIZE;

/// `SHOP_SUBHEADER_GC_START` (0): the window opens.
pub const SHOP_SUBHEADER_GC_START: u8 = 0;
/// `SHOP_SUBHEADER_GC_END` (1): the window closes.
pub const SHOP_SUBHEADER_GC_END: u8 = 1;
/// `SHOP_SUBHEADER_GC_UPDATE_ITEM` (2).
pub const SHOP_SUBHEADER_GC_UPDATE_ITEM: u8 = 2;
/// `SHOP_SUBHEADER_GC_UPDATE_PRICE` (3).
pub const SHOP_SUBHEADER_GC_UPDATE_PRICE: u8 = 3;
/// `SHOP_SUBHEADER_GC_OK` (4).
pub const SHOP_SUBHEADER_GC_OK: u8 = 4;
/// `SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY` (5).
pub const SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY: u8 = 5;
/// `SHOP_SUBHEADER_GC_SOLDOUT` (6).
pub const SHOP_SUBHEADER_GC_SOLDOUT: u8 = 6;
/// `SHOP_SUBHEADER_GC_INVENTORY_FULL` (7).
pub const SHOP_SUBHEADER_GC_INVENTORY_FULL: u8 = 7;
/// `SHOP_SUBHEADER_GC_INVALID_POS` (8).
pub const SHOP_SUBHEADER_GC_INVALID_POS: u8 = 8;
/// `SHOP_SUBHEADER_GC_SOLD_OUT` (9).
pub const SHOP_SUBHEADER_GC_SOLD_OUT: u8 = 9;
/// `SHOP_SUBHEADER_GC_START_EX` (10).
pub const SHOP_SUBHEADER_GC_START_EX: u8 = 10;
/// `SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY_EX` (11).
pub const SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY_EX: u8 = 11;
/// `SHOP_SUBHEADER_GC_NOT_ENOUGH_ITEM` (12), behind `ENABLE_RENEWAL_SHOPEX`.
pub const SHOP_SUBHEADER_GC_NOT_ENOUGH_ITEM: u8 = 12;
/// `SHOP_SUBHEADER_GC_NOT_ENOUGH_EXP` (13), behind `ENABLE_RENEWAL_SHOPEX`.
pub const SHOP_SUBHEADER_GC_NOT_ENOUGH_EXP: u8 = 13;

/// `SHOPEX_GOLD` (1): a price in gold, which every NPC shop slot carries.
pub const SHOPEX_GOLD: u8 = 1;

/// Every way a shop record can be refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcShopError {
    /// Fewer bytes than the record needs.
    Truncated {
        /// The record that was being read.
        context: &'static str,
        /// The bytes it needs.
        needed: usize,
        /// The bytes offered.
        actual: usize,
    },
    /// The leading byte is not 38.
    Header {
        /// The byte that was present.
        actual: u8,
    },
    /// The `size` field is not the record's own width.
    Size {
        /// The width the record has.
        expected: usize,
        /// The `size` the bytes declare.
        actual: usize,
    },
    /// A start record whose subheader is not 0.
    Subheader {
        /// The subheader that was present.
        actual: u8,
    },
}

impl fmt::Display for GcShopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                context,
                needed,
                actual,
            } => write!(f, "{context} needs {needed} bytes, got {actual}"),
            Self::Header { actual } => write!(f, "shop header {actual} is not 38"),
            Self::Size { expected, actual } => {
                write!(f, "shop size {actual} is not {expected}")
            }
            Self::Subheader { actual } => {
                write!(f, "shop start subheader {actual} is not 0")
            }
        }
    }
}

impl std::error::Error for GcShopError {}

/// `TPacketGCShop` alone: the window closing, or the answer to a buy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcShop {
    /// One of the `SHOP_SUBHEADER_GC_*` values, opaque to the codec.
    pub subheader: u8,
}

impl GcShop {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_SHOP.value()
    }

    /// Build the record for one subheader.
    #[must_use]
    pub const fn new(subheader: u8) -> Self {
        Self { subheader }
    }

    /// Appends the four packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        push_header(out, GC_SHOP_WIRE_SIZE, self.subheader);
    }

    /// Encode to a fresh buffer.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_SHOP_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Reads the four packed bytes from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcShopError::Truncated`] for fewer than four bytes,
    /// [`GcShopError::Header`] when the leading byte is not 38, and
    /// [`GcShopError::Size`] when `size` is not 4.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcShopError> {
        let subheader = read_header(bytes, GC_SHOP_WIRE_SIZE, "GcShop")?;
        Ok(Self { subheader })
    }
}

/// One `packet_shop_item`: a slot of the shop window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcShopItem {
    /// `vnum`: the item's prototype, or 0 for an empty slot.
    pub vnum: u32,
    /// `price`: the gold one buy costs.
    pub price: u64,
    /// `count`: the stack one buy makes.
    pub count: u16,
    /// `display_pos`: zero for an NPC shop.
    pub display_pos: u8,
    /// `dwRefineElement`.
    pub refine_element: u32,
    /// `alSockets`.
    pub sockets: ItemSockets,
    /// `aAttr`.
    pub attrs: ItemAttributes,
    /// `transmutation`.
    pub transmutation: u32,
    /// `price_type`: [`SHOPEX_GOLD`] in every NPC shop slot.
    pub price_type: u8,
    /// `price_vnum`.
    pub price_vnum: u32,
}

impl Default for GcShopItem {
    /// An empty slot as `CShop::AddGuest` sends it: zeroed, with a gold price.
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}

impl GcShopItem {
    /// Number of bytes one slot occupies on the wire.
    pub const WIRE_SIZE: usize = GC_SHOP_ITEM_WIRE_SIZE;

    /// A slot as an NPC shop fills it: the three fields `CShop::AddGuest`
    /// sets, a gold price type, and zero everywhere else.
    #[must_use]
    pub const fn new(vnum: u32, price: u64, count: u16) -> Self {
        Self {
            vnum,
            price,
            count,
            display_pos: 0,
            refine_element: 0,
            sockets: [0; ITEM_SOCKET_MAX_NUM],
            attrs: [ItemAttribute::new(0, 0); ITEM_ATTRIBUTE_MAX_NUM],
            transmutation: 0,
            price_type: SHOPEX_GOLD,
            price_vnum: 0,
        }
    }

    /// Appends the 73 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.vnum.to_le_bytes());
        out.extend_from_slice(&self.price.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.push(self.display_pos);
        out.extend_from_slice(&self.refine_element.to_le_bytes());
        for socket in self.sockets {
            out.extend_from_slice(&socket.to_le_bytes());
        }
        for attr in self.attrs {
            attr.encode_into(out);
        }
        out.extend_from_slice(&self.transmutation.to_le_bytes());
        out.push(self.price_type);
        out.extend_from_slice(&self.price_vnum.to_le_bytes());
    }

    /// Reads the 73 packed bytes from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcShopError::Truncated`] for fewer than 73 bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcShopError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcShopError::Truncated {
                context: "GcShopItem",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        let mut at = Reader { bytes, at: 0 };
        let vnum = at.u32();
        let price = u64::from_le_bytes(at.take());
        let count = u16::from_le_bytes(at.take());
        let [display_pos] = at.take();
        let refine_element = at.u32();
        let mut sockets = [0; ITEM_SOCKET_MAX_NUM];
        for socket in &mut sockets {
            *socket = i32::from_le_bytes(at.take());
        }
        let mut attrs = [ItemAttribute::default(); ITEM_ATTRIBUTE_MAX_NUM];
        for attr in &mut attrs {
            let [b_type, low, high] = at.take();
            *attr = ItemAttribute::new(b_type, i16::from_le_bytes([low, high]));
        }
        let transmutation = at.u32();
        let [price_type] = at.take();
        let price_vnum = at.u32();
        Ok(Self {
            vnum,
            price,
            count,
            display_pos,
            refine_element,
            sockets,
            attrs,
            transmutation,
            price_type,
            price_vnum,
        })
    }
}

/// `TPacketGCShop` with `SHOP_SUBHEADER_GC_START`, then `TPacketGCShopStart`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcShopStart {
    /// `owner_vid`: the keeper the window belongs to.
    pub owner_vid: u32,
    /// The forty slots, in window order.
    pub items: [GcShopItem; SHOP_HOST_ITEM_MAX_NUM],
}

impl GcShopStart {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_SHOP.value()
    }

    /// Build the window for a keeper and its forty slots.
    #[must_use]
    pub const fn new(owner_vid: u32, items: [GcShopItem; SHOP_HOST_ITEM_MAX_NUM]) -> Self {
        Self { owner_vid, items }
    }

    /// Appends the 2928 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        push_header(out, GC_SHOP_START_WIRE_SIZE, SHOP_SUBHEADER_GC_START);
        out.extend_from_slice(&self.owner_vid.to_le_bytes());
        for item in &self.items {
            item.encode_into(out);
        }
    }

    /// Encode to a fresh buffer.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_SHOP_START_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Reads the whole window from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcShopError::Truncated`] for fewer than 2928 bytes,
    /// [`GcShopError::Header`] when the leading byte is not 38,
    /// [`GcShopError::Size`] when `size` is not 2928, and
    /// [`GcShopError::Subheader`] when the subheader is not 0.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcShopError> {
        let subheader = read_header(bytes, GC_SHOP_START_WIRE_SIZE, "GcShopStart")?;
        if subheader != SHOP_SUBHEADER_GC_START {
            return Err(GcShopError::Subheader { actual: subheader });
        }
        let body = &bytes[GC_SHOP_WIRE_SIZE..];
        let owner_vid = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        let mut items = [GcShopItem::default(); SHOP_HOST_ITEM_MAX_NUM];
        for (item, raw) in items
            .iter_mut()
            .zip(body[4..].chunks_exact(GcShopItem::WIRE_SIZE))
        {
            *item = GcShopItem::decode(raw)?;
        }
        Ok(Self { owner_vid, items })
    }
}

fn push_header(out: &mut Vec<u8>, size: usize, subheader: u8) {
    let size = u16::try_from(size).unwrap_or(u16::MAX);
    out.push(HEADER_GC_SHOP.value());
    out.extend_from_slice(&size.to_le_bytes());
    out.push(subheader);
}

fn read_header(bytes: &[u8], width: usize, context: &'static str) -> Result<u8, GcShopError> {
    if bytes.len() < width {
        return Err(GcShopError::Truncated {
            context,
            needed: width,
            actual: bytes.len(),
        });
    }
    if bytes[0] != HEADER_GC_SHOP.value() {
        return Err(GcShopError::Header { actual: bytes[0] });
    }
    let size = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
    if size != width {
        return Err(GcShopError::Size {
            expected: width,
            actual: size,
        });
    }
    Ok(bytes[3])
}

/// Reads fixed-width fields in order. The caller checks the length first.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let mut out = [0; N];
        out.copy_from_slice(&self.bytes[self.at..self.at + N]);
        self.at += N;
        out
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GcShop, GcShopError, GcShopItem, GcShopStart, GC_SHOP_ITEM_WIRE_SIZE,
        GC_SHOP_START_WIRE_SIZE, GC_SHOP_WIRE_SIZE, SHOPEX_GOLD, SHOP_HOST_ITEM_MAX_NUM,
        SHOP_SUBHEADER_GC_END, SHOP_SUBHEADER_GC_INVALID_POS, SHOP_SUBHEADER_GC_INVENTORY_FULL,
        SHOP_SUBHEADER_GC_NOT_ENOUGH_EXP, SHOP_SUBHEADER_GC_NOT_ENOUGH_ITEM,
        SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY, SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY_EX,
        SHOP_SUBHEADER_GC_OK, SHOP_SUBHEADER_GC_SOLDOUT, SHOP_SUBHEADER_GC_SOLD_OUT,
        SHOP_SUBHEADER_GC_START, SHOP_SUBHEADER_GC_START_EX, SHOP_SUBHEADER_GC_UPDATE_ITEM,
        SHOP_SUBHEADER_GC_UPDATE_PRICE,
    };
    use crate::gc_item_window::ItemAttribute;

    /// A slot whose every field holds a distinct value, so a swapped or
    /// misplaced field shows in the bytes.
    fn marked() -> GcShopItem {
        let mut item = GcShopItem::new(0x0102_0304, 0x1112_1314_1516_1718, 0x2122);
        item.display_pos = 0x31;
        item.refine_element = 0x4142_4344;
        item.sockets = [-1, 2, 3, 4, 5, i32::MIN];
        item.attrs = [
            ItemAttribute::new(1, -2),
            ItemAttribute::new(3, 4),
            ItemAttribute::new(5, 6),
            ItemAttribute::new(7, 8),
            ItemAttribute::new(9, 10),
            ItemAttribute::new(11, 12),
            ItemAttribute::new(13, i16::MIN),
        ];
        item.transmutation = 0x5152_5354;
        item.price_type = 0x61;
        item.price_vnum = 0x7172_7374;
        item
    }

    /// The bytes of [`marked`], written from the source's field order.
    fn marked_bytes() -> Vec<u8> {
        let mut out = vec![4, 3, 2, 1, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11];
        out.extend([0x22, 0x21, 0x31, 0x44, 0x43, 0x42, 0x41]);
        out.extend([0xff, 0xff, 0xff, 0xff, 2, 0, 0, 0, 3, 0, 0, 0]);
        out.extend([4, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0x80]);
        out.extend([
            1, 0xfe, 0xff, 3, 4, 0, 5, 6, 0, 7, 8, 0, 9, 10, 0, 11, 12, 0,
        ]);
        out.extend([13, 0, 0x80]);
        out.extend([0x54, 0x53, 0x52, 0x51, 0x61, 0x74, 0x73, 0x72, 0x71]);
        out
    }

    #[test]
    fn the_widths_and_subheaders_match_the_source() {
        assert_eq!(GcShop::header(), 38);
        assert_eq!(GcShopStart::header(), 38);
        assert_eq!(GC_SHOP_WIRE_SIZE, 4);
        assert_eq!(
            GC_SHOP_ITEM_WIRE_SIZE,
            4 + 8 + 2 + 1 + 4 + 24 + 21 + 4 + 1 + 4
        );
        assert_eq!(GC_SHOP_ITEM_WIRE_SIZE, 73);
        assert_eq!(SHOP_HOST_ITEM_MAX_NUM, 40);
        assert_eq!(GC_SHOP_START_WIRE_SIZE, 2928);
        let subheaders = [
            SHOP_SUBHEADER_GC_START,
            SHOP_SUBHEADER_GC_END,
            SHOP_SUBHEADER_GC_UPDATE_ITEM,
            SHOP_SUBHEADER_GC_UPDATE_PRICE,
            SHOP_SUBHEADER_GC_OK,
            SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY,
            SHOP_SUBHEADER_GC_SOLDOUT,
            SHOP_SUBHEADER_GC_INVENTORY_FULL,
            SHOP_SUBHEADER_GC_INVALID_POS,
            SHOP_SUBHEADER_GC_SOLD_OUT,
            SHOP_SUBHEADER_GC_START_EX,
            SHOP_SUBHEADER_GC_NOT_ENOUGH_MONEY_EX,
            SHOP_SUBHEADER_GC_NOT_ENOUGH_ITEM,
            SHOP_SUBHEADER_GC_NOT_ENOUGH_EXP,
        ];
        assert!(subheaders.iter().copied().eq(0..14));
        assert_eq!(SHOPEX_GOLD, 1);
    }

    #[test]
    fn an_answer_is_four_bytes_and_round_trips() {
        for sub in [SHOP_SUBHEADER_GC_END, SHOP_SUBHEADER_GC_SOLD_OUT, 0xff] {
            let bytes = GcShop::new(sub).encode();
            assert_eq!(bytes, vec![38, 4, 0, sub]);
            assert_eq!(GcShop::decode(&bytes), Ok(GcShop::new(sub)));
        }
    }

    #[test]
    fn an_answer_is_refused_when_short_foreign_or_mis_sized() {
        assert_eq!(
            GcShop::decode(&[38, 4, 0]),
            Err(GcShopError::Truncated {
                context: "GcShop",
                needed: 4,
                actual: 3
            })
        );
        assert_eq!(
            GcShop::decode(&[39, 4, 0, 1]),
            Err(GcShopError::Header { actual: 39 })
        );
        assert_eq!(
            GcShop::decode(&[38, 5, 0, 1]),
            Err(GcShopError::Size {
                expected: 4,
                actual: 5
            })
        );
        assert_eq!(
            GcShop::decode(&[38, 4, 1, 1]),
            Err(GcShopError::Size {
                expected: 4,
                actual: 260
            })
        );
    }

    #[test]
    fn a_slot_is_written_in_the_sources_field_order() {
        let bytes = {
            let mut out = Vec::new();
            marked().encode_into(&mut out);
            out
        };
        assert_eq!(bytes, marked_bytes());
        assert_eq!(bytes.len(), 73);
        assert_eq!(GcShopItem::decode(&bytes), Ok(marked()));
        assert_eq!(
            GcShopItem::decode(&bytes[..72]),
            Err(GcShopError::Truncated {
                context: "GcShopItem",
                needed: 73,
                actual: 72
            })
        );
    }

    #[test]
    fn an_empty_slot_is_zero_with_a_gold_price() {
        let mut bytes = Vec::new();
        GcShopItem::default().encode_into(&mut bytes);
        let mut expected = vec![0; 73];
        expected[68] = SHOPEX_GOLD;
        assert_eq!(bytes, expected);
        assert_eq!(GcShopItem::new(27001, 10, 1).price_type, SHOPEX_GOLD);
    }

    #[test]
    fn the_window_is_the_header_the_keeper_and_forty_slots() {
        let mut items = [GcShopItem::default(); SHOP_HOST_ITEM_MAX_NUM];
        items[0] = GcShopItem::new(27001, 10, 1);
        items[39] = marked();
        let start = GcShopStart::new(0x0a0b_0c0d, items);
        let bytes = start.encode();
        assert_eq!(bytes.len(), 2928);
        assert_eq!(&bytes[..8], &[38, 0x70, 0x0b, 0, 0x0d, 0x0c, 0x0b, 0x0a]);
        assert_eq!(&bytes[8..12], &27001u32.to_le_bytes());
        assert_eq!(&bytes[12..20], &10u64.to_le_bytes());
        assert_eq!(&bytes[20..22], &[1, 0]);
        assert_eq!(&bytes[8 + 39 * 73..], marked_bytes().as_slice());
        assert_eq!(GcShopStart::decode(&bytes), Ok(start));
    }

    #[test]
    fn a_window_is_refused_when_short_foreign_mis_sized_or_not_a_start() {
        let bytes = GcShopStart::new(1, [GcShopItem::default(); 40]).encode();
        assert_eq!(
            GcShopStart::decode(&bytes[..2927]),
            Err(GcShopError::Truncated {
                context: "GcShopStart",
                needed: 2928,
                actual: 2927
            })
        );
        let mut foreign = bytes.clone();
        foreign[0] = 37;
        assert_eq!(
            GcShopStart::decode(&foreign),
            Err(GcShopError::Header { actual: 37 })
        );
        let mut sized = bytes.clone();
        sized[1] = 0x71;
        assert_eq!(
            GcShopStart::decode(&sized),
            Err(GcShopError::Size {
                expected: 2928,
                actual: 2929
            })
        );
        let mut ended = bytes;
        ended[3] = SHOP_SUBHEADER_GC_START_EX;
        assert_eq!(
            GcShopStart::decode(&ended),
            Err(GcShopError::Subheader { actual: 10 })
        );
        assert_eq!(
            GcShop::decode(&GcShop::new(SHOP_SUBHEADER_GC_END).encode()).map(|shop| shop.subheader),
            Ok(1)
        );
    }

    #[test]
    fn the_errors_say_what_was_wrong() {
        let cases = [
            (
                GcShopError::Truncated {
                    context: "GcShop",
                    needed: 4,
                    actual: 1,
                },
                "GcShop needs 4 bytes, got 1",
            ),
            (GcShopError::Header { actual: 9 }, "shop header 9 is not 38"),
            (
                GcShopError::Size {
                    expected: 4,
                    actual: 7,
                },
                "shop size 7 is not 4",
            ),
            (
                GcShopError::Subheader { actual: 3 },
                "shop start subheader 3 is not 0",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
        }
    }
}
