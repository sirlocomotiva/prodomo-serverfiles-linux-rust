//! `HEADER_GC_PRIVATE_SHOP` (175): a personal shop's stand, its panel, its window, and its sale.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:1330-1335` declares the record every private-shop send
//! starts with:
//!
//! ```ignore
//! typedef struct SPacketGCPrivateShop
//! {
//!     BYTE    bHeader;
//!     WORD    wSize;
//!     BYTE    bSubHeader;
//! } TPacketGCPrivateShop;
//! ```
//!
//! The subheaders are `EPrivateShopGCSubheader` at `packet.h:1101-1129`, numbered from 0
//! with `__PREMIUM_PRIVATE_SHOP__` on.
//!
//! The build, browse and buy subheaders are encoded here: `LOAD` (3), which
//! `CHARACTER::SetPrivateShopTable` (`server/server/game/char.cpp:12160-12178`) sends to the
//! owner after the store accepts the build; `ADD_ENTITY` (0), which
//! `CPrivateShop::EncodeInsertPacket` (`server/server/game/private_shop.cpp:853-880`) sends to
//! each viewer around the stand; the bare-header `OPEN_PANEL` (7) and `CLOSE` (9), which
//! `OpenPrivateShopPanel`, `ClosePrivateShopPanel` and `ClosePrivateShop`
//! (`char.cpp:12182-12219`) send; `START` (11) and `END` (12), which `AddShopViewer` and
//! `RemoveShopViewer` (`private_shop.cpp:739-811`) send to the viewer; `REMOVE_ITEM` (13) and
//! `REMOVE_MY_ITEM` (14); `SET_SALE` (5), which `SetPrivateShopSale` (`char.cpp:12295-12311`)
//! sends to the owner; and `BALANCE_UPDATE` (6), which `SaleUpdate` (`char.cpp:12524-12534`)
//! sends with it. The edit records are encoded here: `SET_ITEM` (4), which
//! `SetPrivateShopItem` (`char.cpp:12242-12257`) sends to the owner at checkin;
//! `ADD_ITEM` (14), which `CPrivateShop::ItemCheckin` (`private_shop.cpp:559-595`)
//! broadcasts to the viewers; and the price and the move, which both the owner
//! and the stand send (`char.cpp:12317-12373`, `private_shop.cpp:597-671`). The
//! rename records are encoded here: `TITLE_CHANGE` (19), which
//! `ChangePrivateShopTitle` (`char.cpp:12375-12390`) sends to the owner, and
//! `TITLE` (2), which `CPrivateShop::ChangeTitle` (`private_shop.cpp:674-697`)
//! sends around the stand, and `UNLOCKED_SLOTS_CHANGE` (20), which
//! `CPrivateShop::SetUnlockedSlots` (`private_shop.cpp:721-737`) broadcasts to
//! the viewers. The rest are later slices.
//!
//! # Width
//!
//! `LOAD`'s body is `TPacketGCPrivateShopLoad` (`packet.h:1337-1348`): a 33-byte title
//! (`SHOP_SIGN_MAX_LEN + 1`, but the copy's source is the 33-byte `TPrivateShop.szTitle`,
//! so all 33 carry), the gold, the cheque, the long X and Y (4 bytes each on i686), and
//! three bytes (channel, state, page count): 33 + 8 + 4 + 4 + 4 + 3 = 56 packed bytes,
//! because `packet.h` is inside `#pragma pack(1)`.
//!
//! `ADD_ENTITY`'s body is `TPacketGCPrivateShopAddEntity` (`packet.h:1233-1242`): three
//! longs, the VID, the vnum, a 25-byte name, the title-type byte, and the 33-byte title:
//! 12 + 4 + 4 + 25 + 1 + 33 = 79 packed bytes.
//!
//! `START`'s body is `TPacketGCPrivateShopOpen` (`packet.h:1348-1355`): the 33-byte title,
//! the state, the page count, the unlocked slots, the VID, and 128
//! `TPrivateShopItemData` rows (`packet.h:1171-1191`): each is the vnum (4), the price
//! (8 + 4), the checkin (4, `time_t` is a `long`), the count (4), the slot (2), six
//! sockets (24, `ENABLE_EXTENDED_SOCKETS` is on) and seven attributes (7 x (1 + 2) = 21),
//! all packed: 71 bytes. `SET_SALE`'s body (`packet.h:1193-1198`) is the 25-byte customer
//! name, the sale time (4), and one item row (71): 100 bytes. `BALANCE_UPDATE`'s body
//! (`packet.h:1375-1378`) is the price (12). `REMOVE_ITEM` and `REMOVE_MY_ITEM` carry one
//! word each.

use crate::gc_inventory::HEADER_GC_PRIVATE_SHOP;

/// `sizeof(TPacketGCPrivateShop)`: header, size and subheader.
pub const GC_PRIVATE_SHOP_WIRE_SIZE: usize = 4;

/// `sizeof(TPacketGCPrivateShopLoad)`.
pub const GC_PRIVATE_SHOP_LOAD_WIRE_SIZE: usize = 56;

/// `sizeof(TPacketGCPrivateShopAddEntity)`.
pub const GC_PRIVATE_SHOP_ADD_ENTITY_WIRE_SIZE: usize = 79;

/// `sizeof(TPrivateShopItemData)`: vnum, price, checkin, count, slot, sockets, attrs.
pub const GC_PRIVATE_SHOP_ITEM_DATA_WIRE_SIZE: usize = 71;

/// `sizeof(TPacketGCPrivateShopOpen)`: title, state, pages, unlocked slots, VID, 128 rows.
pub const GC_PRIVATE_SHOP_OPEN_WIRE_SIZE: usize =
    33 + 1 + 1 + 2 + 4 + 128 * GC_PRIVATE_SHOP_ITEM_DATA_WIRE_SIZE;

/// `sizeof(TPrivateShopSaleData)`: customer, time, and one item row.
pub const GC_PRIVATE_SHOP_SALE_WIRE_SIZE: usize =
    PRIVATE_SHOP_OWNER_FIELD_SIZE + 4 + GC_PRIVATE_SHOP_ITEM_DATA_WIRE_SIZE;

/// `sizeof(TPacketGCPrivateShopBalanceUpdate)`: one `TItemPrice`.
pub const GC_PRIVATE_SHOP_BALANCE_WIRE_SIZE: usize = 12;

/// `sizeof(TPacketGCPrivateShopItemPriceChange)`: the word and the price.
pub const GC_PRIVATE_SHOP_PRICE_CHANGE_WIRE_SIZE: usize = 14;

/// `sizeof(TPacketGCPrivateShopItemMove)`: the two words.
pub const GC_PRIVATE_SHOP_MOVE_WIRE_SIZE: usize = 4;

/// `sizeof(TPacketGCPrivateShopTitle)`: the VID, the title and the type.
pub const GC_PRIVATE_SHOP_TITLE_WIRE_SIZE: usize = 4 + 33 + 1;

/// `sizeof(WORD)`: what `REMOVE_ITEM` and `REMOVE_MY_ITEM` carry.
pub const GC_PRIVATE_SHOP_REMOVE_WIRE_SIZE: usize = 2;

/// The whole `LOAD` record: the header and the load body.
pub const GC_PRIVATE_SHOP_LOAD_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_LOAD_WIRE_SIZE;

/// The whole `ADD_ENTITY` record: the header and the entity body.
pub const GC_PRIVATE_SHOP_ADD_ENTITY_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_ADD_ENTITY_WIRE_SIZE;

/// The whole `START` record: the header and the open body.
pub const GC_PRIVATE_SHOP_START_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_OPEN_WIRE_SIZE;

/// The whole `SET_SALE` record: the header and the sale body.
pub const GC_PRIVATE_SHOP_SALE_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_SALE_WIRE_SIZE;

/// The whole `BALANCE_UPDATE` record: the header and the price.
pub const GC_PRIVATE_SHOP_BALANCE_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_BALANCE_WIRE_SIZE;

/// The whole `REMOVE_ITEM` / `REMOVE_MY_ITEM` record: the header and the word.
pub const GC_PRIVATE_SHOP_REMOVE_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_REMOVE_WIRE_SIZE;

/// The whole `SET_ITEM` / `ADD_ITEM` record: the header and the item row.
pub const GC_PRIVATE_SHOP_SET_ITEM_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_ITEM_DATA_WIRE_SIZE;

/// The whole `ITEM_PRICE_CHANGE` record: the header, the word and the price.
pub const GC_PRIVATE_SHOP_PRICE_CHANGE_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_PRICE_CHANGE_WIRE_SIZE;

/// The whole `ITEM_MOVE` record: the header and the two words.
pub const GC_PRIVATE_SHOP_MOVE_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_MOVE_WIRE_SIZE;

/// The whole `TITLE` record: the header, the VID, the title and the type.
pub const GC_PRIVATE_SHOP_TITLE_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_TITLE_WIRE_SIZE;

/// The title field both bodies carry: `TITLE_MAX_LEN + 1` and `SHOP_SIGN_MAX_LEN + 1`
/// are both 33.
pub const PRIVATE_SHOP_TITLE_FIELD_SIZE: usize = 33;

/// The owner-name field of `ADD_ENTITY`: `CHARACTER_NAME_MAX_LEN + 1`.
pub const PRIVATE_SHOP_OWNER_FIELD_SIZE: usize = 25;

/// `SUBHEADER_GC_PRIVATE_SHOP_ADD_ENTITY` (0): the stand appears.
pub const SUBHEADER_GC_PRIVATE_SHOP_ADD_ENTITY: u8 = 0;
/// `SUBHEADER_GC_PRIVATE_SHOP_LOAD` (3): the build is stored.
pub const SUBHEADER_GC_PRIVATE_SHOP_LOAD: u8 = 3;
/// `SUBHEADER_GC_PRIVATE_SHOP_OPEN_PANEL` (7): the panel opens.
pub const SUBHEADER_GC_PRIVATE_SHOP_OPEN_PANEL: u8 = 7;
/// `SUBHEADER_GC_PRIVATE_SHOP_CLOSE_PANEL` (8): the panel closes.
pub const SUBHEADER_GC_PRIVATE_SHOP_CLOSE_PANEL: u8 = 8;
/// `SUBHEADER_GC_PRIVATE_SHOP_CLOSE` (9): the shop closed.
pub const SUBHEADER_GC_PRIVATE_SHOP_CLOSE: u8 = 9;
/// `SUBHEADER_GC_PRIVATE_SHOP_STATE_UPDATE` (15): the state flipped.
pub const SUBHEADER_GC_PRIVATE_SHOP_STATE_UPDATE: u8 = 15;
/// `SUBHEADER_GC_PRIVATE_SHOP_WITHDRAW` (16): the stash emptied.
pub const SUBHEADER_GC_PRIVATE_SHOP_WITHDRAW: u8 = 16;
/// `SUBHEADER_GC_PRIVATE_SHOP_SET_SALE` (5): the owner hears the sale.
pub const SUBHEADER_GC_PRIVATE_SHOP_SET_SALE: u8 = 5;
/// `SUBHEADER_GC_PRIVATE_SHOP_BALANCE_UPDATE` (6): the stash grew.
pub const SUBHEADER_GC_PRIVATE_SHOP_BALANCE_UPDATE: u8 = 6;
/// `SUBHEADER_GC_PRIVATE_SHOP_SET_ITEM` (4): the ware lands, as the owner hears it.
pub const SUBHEADER_GC_PRIVATE_SHOP_SET_ITEM: u8 = 4;
/// `SUBHEADER_GC_PRIVATE_SHOP_ADD_ITEM` (14): the ware lands, as the viewers hear it.
pub const SUBHEADER_GC_PRIVATE_SHOP_ADD_ITEM: u8 = 14;
/// `SUBHEADER_GC_PRIVATE_SHOP_ITEM_PRICE_CHANGE` (17): the new price.
pub const SUBHEADER_GC_PRIVATE_SHOP_ITEM_PRICE_CHANGE: u8 = 17;
/// `SUBHEADER_GC_PRIVATE_SHOP_ITEM_MOVE` (18): the ware's new slot.
pub const SUBHEADER_GC_PRIVATE_SHOP_ITEM_MOVE: u8 = 18;
/// `SUBHEADER_GC_PRIVATE_SHOP_TITLE` (2): the new title, as the viewers hear it.
pub const SUBHEADER_GC_PRIVATE_SHOP_TITLE: u8 = 2;
/// `SUBHEADER_GC_PRIVATE_SHOP_TITLE_CHANGE` (19): the new title, as the owner hears it.
pub const SUBHEADER_GC_PRIVATE_SHOP_TITLE_CHANGE: u8 = 19;
/// `SUBHEADER_GC_PRIVATE_SHOP_UNLOCKED_SLOTS_CHANGE` (20): the unlocked count.
pub const SUBHEADER_GC_PRIVATE_SHOP_UNLOCKED_SLOTS_CHANGE: u8 = 20;
/// `SUBHEADER_GC_PRIVATE_SHOP_START` (11): the window opens for the viewer.
pub const SUBHEADER_GC_PRIVATE_SHOP_START: u8 = 11;
/// `SUBHEADER_GC_PRIVATE_SHOP_END` (12): the window closes for the viewer.
pub const SUBHEADER_GC_PRIVATE_SHOP_END: u8 = 12;
/// `SUBHEADER_GC_PRIVATE_SHOP_REMOVE_ITEM` (13): the slot clears for the viewers.
pub const SUBHEADER_GC_PRIVATE_SHOP_REMOVE_ITEM: u8 = 13;
/// `SUBHEADER_GC_PRIVATE_SHOP_REMOVE_MY_ITEM` (14): the slot clears for the owner.
pub const SUBHEADER_GC_PRIVATE_SHOP_REMOVE_MY_ITEM: u8 = 14;

/// `STATE_OPEN` (2): the shop takes customers.
pub const PRIVATE_SHOP_STATE_OPEN: u8 = 2;

/// `TPacketGCPrivateShop` alone: the panel opening, or the shop closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShop {
    /// The subheader byte.
    pub subheader: u8,
}

impl GcPrivateShop {
    /// Build a bare-header record.
    #[must_use]
    pub const fn new(subheader: u8) -> Self {
        Self { subheader }
    }

    /// Encode the four header bytes, with `size` 4.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_WIRE_SIZE);
        push_header(&mut out, GC_PRIVATE_SHOP_WIRE_SIZE, self.subheader);
        out
    }
}

/// `LOAD`: the stored shop's title, earnings, stand, and state, as the owner hears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcPrivateShopLoad {
    /// The raw 33-byte title field.
    pub title: [u8; PRIVATE_SHOP_TITLE_FIELD_SIZE],
    /// The earnings in yang.
    pub gold: i64,
    /// The earnings in cheques.
    pub cheque: u32,
    /// The stand's X.
    pub x: i32,
    /// The stand's Y.
    pub y: i32,
    /// The Channel the stand is on.
    pub channel: u8,
    /// The shop's state.
    pub state: u8,
    /// The page count.
    pub page_count: u8,
}

impl GcPrivateShopLoad {
    /// Encode the header (`size` 60) and the load body in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_LOAD_RECORD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_LOAD_RECORD_SIZE,
            SUBHEADER_GC_PRIVATE_SHOP_LOAD,
        );
        out.extend_from_slice(&self.title);
        out.extend_from_slice(&self.gold.to_le_bytes());
        out.extend_from_slice(&self.cheque.to_le_bytes());
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.push(self.channel);
        out.push(self.state);
        out.push(self.page_count);
        out
    }
}

/// `ADD_ENTITY`: the stand as the viewers around it hear it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcPrivateShopAddEntity {
    /// The stand's X.
    pub x: i32,
    /// The stand's Y.
    pub y: i32,
    /// The stand's Z.
    pub z: i32,
    /// The stand's VID.
    pub vid: u32,
    /// The polymorph vnum the stand wears.
    pub vnum: u32,
    /// The raw 25-byte owner-name field.
    pub owner: [u8; PRIVATE_SHOP_OWNER_FIELD_SIZE],
    /// The title's type byte.
    pub title_type: u8,
    /// The raw 33-byte title field.
    pub title: [u8; PRIVATE_SHOP_TITLE_FIELD_SIZE],
}

impl GcPrivateShopAddEntity {
    /// Encode the header (`size` 83) and the entity body in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_ADD_ENTITY_RECORD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_ADD_ENTITY_RECORD_SIZE,
            SUBHEADER_GC_PRIVATE_SHOP_ADD_ENTITY,
        );
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.extend_from_slice(&self.z.to_le_bytes());
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.extend_from_slice(&self.vnum.to_le_bytes());
        out.extend_from_slice(&self.owner);
        out.push(self.title_type);
        out.extend_from_slice(&self.title);
        out
    }
}

/// One `TPrivateShopItemData` row: the ware as the window and the sale show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopItemData {
    /// The ware's vnum.
    pub vnum: u32,
    /// The asking price in yang.
    pub gold: i64,
    /// The asking price in cheques.
    pub cheque: u32,
    /// The checkin time.
    pub checkin: i32,
    /// The ware's count.
    pub count: u32,
    /// The shop slot.
    pub pos: u16,
    /// The six sockets.
    pub sockets: [i32; 6],
    /// The seven attributes: type and value.
    pub attrs: [(u8, i16); 7],
}

impl GcPrivateShopItemData {
    /// An empty row, as `memset` leaves the unsold slots of `START`.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            vnum: 0,
            gold: 0,
            cheque: 0,
            checkin: 0,
            count: 0,
            pos: 0,
            sockets: [0; 6],
            attrs: [(0, 0); 7],
        }
    }

    /// Append the 71 packed bytes in field order.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.vnum.to_le_bytes());
        out.extend_from_slice(&self.gold.to_le_bytes());
        out.extend_from_slice(&self.cheque.to_le_bytes());
        out.extend_from_slice(&self.checkin.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.pos.to_le_bytes());
        for socket in self.sockets {
            out.extend_from_slice(&socket.to_le_bytes());
        }
        for (kind, value) in self.attrs {
            out.push(kind);
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

/// `START`: the window as the viewer hears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcPrivateShopStart {
    /// The raw 33-byte title field.
    pub title: [u8; PRIVATE_SHOP_TITLE_FIELD_SIZE],
    /// The shop's state.
    pub state: u8,
    /// The page count.
    pub page_count: u8,
    /// The unlocked slots.
    pub unlocked_slots: u16,
    /// The stand's VID.
    pub vid: u32,
    /// The 128 window rows, in slot order.
    pub items: [GcPrivateShopItemData; 128],
}

impl GcPrivateShopStart {
    /// Encode the header and the open body in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_START_RECORD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_START_RECORD_SIZE,
            SUBHEADER_GC_PRIVATE_SHOP_START,
        );
        out.extend_from_slice(&self.title);
        out.push(self.state);
        out.push(self.page_count);
        out.extend_from_slice(&self.unlocked_slots.to_le_bytes());
        out.extend_from_slice(&self.vid.to_le_bytes());
        for row in self.items {
            row.encode_into(&mut out);
        }
        out
    }
}

/// `SET_SALE`: the sold ware as the owner hears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopSale {
    /// The raw 25-byte customer-name field.
    pub customer: [u8; PRIVATE_SHOP_OWNER_FIELD_SIZE],
    /// The sale time.
    pub time: i32,
    /// The sold ware's row.
    pub item: GcPrivateShopItemData,
}

impl GcPrivateShopSale {
    /// Encode the header and the sale body in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_SALE_RECORD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_SALE_RECORD_SIZE,
            SUBHEADER_GC_PRIVATE_SHOP_SET_SALE,
        );
        out.extend_from_slice(&self.customer);
        out.extend_from_slice(&self.time.to_le_bytes());
        self.item.encode_into(&mut out);
        out
    }
}

/// `BALANCE_UPDATE`: the stash's growth as the owner hears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopBalance {
    /// The sold ware's price in yang.
    pub gold: i64,
    /// The sold ware's price in cheques.
    pub cheque: u32,
}

impl GcPrivateShopBalance {
    /// Encode the header and the price in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_BALANCE_RECORD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_BALANCE_RECORD_SIZE,
            SUBHEADER_GC_PRIVATE_SHOP_BALANCE_UPDATE,
        );
        out.extend_from_slice(&self.gold.to_le_bytes());
        out.extend_from_slice(&self.cheque.to_le_bytes());
        out
    }
}

/// `STATE_UPDATE`: the flipped state, and whether it is the viewer's own shop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopState {
    /// The shop's new state.
    pub state: u8,
    /// Whether the shop is the viewer's own.
    pub own: bool,
}

impl GcPrivateShopState {
    /// Encode the header (`size` 6) and the two bytes in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_WIRE_SIZE + 2);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_WIRE_SIZE + 2,
            SUBHEADER_GC_PRIVATE_SHOP_STATE_UPDATE,
        );
        out.push(self.state);
        out.push(u8::from(self.own));
        out
    }
}

/// `REMOVE_ITEM` / `REMOVE_MY_ITEM`: the cleared slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopRemove {
    /// The subheader: 13 for the viewers, 14 for the owner.
    pub subheader: u8,
    /// The cleared shop slot.
    pub pos: u16,
}

impl GcPrivateShopRemove {
    /// Encode the header and the word.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_REMOVE_RECORD_SIZE);
        push_header(&mut out, GC_PRIVATE_SHOP_REMOVE_RECORD_SIZE, self.subheader);
        out.extend_from_slice(&self.pos.to_le_bytes());
        out
    }
}

/// `SET_ITEM` / `ADD_ITEM`: the checked-in ware's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopSetItem {
    /// The subheader: 4 for the owner, 14 for the viewers.
    pub subheader: u8,
    /// The checked-in ware's row.
    pub item: GcPrivateShopItemData,
}

impl GcPrivateShopSetItem {
    /// Encode the header and the row in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_SET_ITEM_RECORD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_SET_ITEM_RECORD_SIZE,
            self.subheader,
        );
        self.item.encode_into(&mut out);
        out
    }
}

/// `ITEM_PRICE_CHANGE`: the ware's slot and its new price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopPriceChange {
    /// The ware's shop slot.
    pub pos: u16,
    /// The new price in yang.
    pub gold: i64,
    /// The new price in cheques.
    pub cheque: u32,
}

impl GcPrivateShopPriceChange {
    /// Encode the header, the word and the price in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_PRICE_CHANGE_RECORD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_PRICE_CHANGE_RECORD_SIZE,
            SUBHEADER_GC_PRIVATE_SHOP_ITEM_PRICE_CHANGE,
        );
        out.extend_from_slice(&self.pos.to_le_bytes());
        out.extend_from_slice(&self.gold.to_le_bytes());
        out.extend_from_slice(&self.cheque.to_le_bytes());
        out
    }
}

/// `ITEM_MOVE`: the ware's slot and where it went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopMove {
    /// The ware's old shop slot.
    pub pos: u16,
    /// The ware's new shop slot.
    pub change_pos: u16,
}

impl GcPrivateShopMove {
    /// Encode the header and the two words in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_MOVE_RECORD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_MOVE_RECORD_SIZE,
            SUBHEADER_GC_PRIVATE_SHOP_ITEM_MOVE,
        );
        out.extend_from_slice(&self.pos.to_le_bytes());
        out.extend_from_slice(&self.change_pos.to_le_bytes());
        out
    }
}

/// `TITLE_CHANGE`: the new title, as the owner hears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopTitleChange {
    /// The raw 33-byte title field.
    pub title: [u8; PRIVATE_SHOP_TITLE_FIELD_SIZE],
}

impl GcPrivateShopTitleChange {
    /// Encode the header and the title in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_WIRE_SIZE + PRIVATE_SHOP_TITLE_FIELD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_WIRE_SIZE + PRIVATE_SHOP_TITLE_FIELD_SIZE,
            SUBHEADER_GC_PRIVATE_SHOP_TITLE_CHANGE,
        );
        out.extend_from_slice(&self.title);
        out
    }
}

/// `TITLE`: the stand's VID and new title, as the viewers hear it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopTitle {
    /// The stand's VID.
    pub vid: u32,
    /// The raw 33-byte title field.
    pub title: [u8; PRIVATE_SHOP_TITLE_FIELD_SIZE],
    /// The title's type byte.
    pub title_type: u8,
}

impl GcPrivateShopTitle {
    /// Encode the header, the VID, the title and the type in field order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_TITLE_RECORD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_TITLE_RECORD_SIZE,
            SUBHEADER_GC_PRIVATE_SHOP_TITLE,
        );
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.extend_from_slice(&self.title);
        out.push(self.title_type);
        out
    }
}

/// `UNLOCKED_SLOTS_CHANGE`: the shop's unlocked count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPrivateShopUnlocked {
    /// The unlocked count.
    pub slots: u16,
}

impl GcPrivateShopUnlocked {
    /// Encode the header and the word.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_PRIVATE_SHOP_REMOVE_RECORD_SIZE);
        push_header(
            &mut out,
            GC_PRIVATE_SHOP_REMOVE_RECORD_SIZE,
            SUBHEADER_GC_PRIVATE_SHOP_UNLOCKED_SLOTS_CHANGE,
        );
        out.extend_from_slice(&self.slots.to_le_bytes());
        out
    }
}

/// Push the four header bytes: the header, the size word, and the subheader.
fn push_header(out: &mut Vec<u8>, size: usize, subheader: u8) {
    let size = u16::try_from(size).unwrap_or(u16::MAX);
    out.push(HEADER_GC_PRIVATE_SHOP.value());
    out.extend_from_slice(&size.to_le_bytes());
    out.push(subheader);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An item row with distinct byte halves, so an endianness swap shows.
    fn golden_row() -> GcPrivateShopItemData {
        GcPrivateShopItemData {
            vnum: 0x7531,
            gold: 0x0102_0304_0506_0708,
            cheque: 0x090a_0b0c,
            checkin: 0x0d0e_0f10,
            count: 0x1112_1314,
            pos: 0x1516,
            sockets: [0x0102, 0x0304, 0x0506, 0x0708, 0x090a, 0x0b0c],
            attrs: [
                (1, 0x0203),
                (4, 0x0506),
                (12, 0x0708),
                (17, 0x090a),
                (71, 0x0b0c),
                (72, 0x0d0e),
                (0, 0),
            ],
        }
    }

    #[test]
    fn the_edits_encode_in_field_order() {
        let set = GcPrivateShopSetItem {
            subheader: SUBHEADER_GC_PRIVATE_SHOP_SET_ITEM,
            item: golden_row(),
        }
        .encode();
        assert_eq!(set.len(), GC_PRIVATE_SHOP_SET_ITEM_RECORD_SIZE);
        assert_eq!(set[3], SUBHEADER_GC_PRIVATE_SHOP_SET_ITEM);
        assert_eq!(
            u32::from_le_bytes(set[4..8].try_into().expect("four")),
            0x7531
        );
        assert_eq!(
            i64::from_le_bytes(set[8..16].try_into().expect("eight")),
            0x0102_0304_0506_0708
        );
        assert_eq!(
            u16::from_le_bytes(set[28..30].try_into().expect("two")),
            0x1516
        );
        let add = GcPrivateShopSetItem {
            subheader: SUBHEADER_GC_PRIVATE_SHOP_ADD_ITEM,
            item: golden_row(),
        }
        .encode();
        assert_eq!(add.len(), GC_PRIVATE_SHOP_SET_ITEM_RECORD_SIZE);
        assert_eq!(add[3], SUBHEADER_GC_PRIVATE_SHOP_ADD_ITEM);
        let price = GcPrivateShopPriceChange {
            pos: 0x1011,
            gold: 0x0102_0304_0506_0708,
            cheque: 0x090a_0b0c,
        }
        .encode();
        assert_eq!(price.len(), GC_PRIVATE_SHOP_PRICE_CHANGE_RECORD_SIZE);
        assert_eq!(price[3], SUBHEADER_GC_PRIVATE_SHOP_ITEM_PRICE_CHANGE);
        assert_eq!(
            u16::from_le_bytes(price[4..6].try_into().expect("two")),
            0x1011
        );
        assert_eq!(
            i64::from_le_bytes(price[6..14].try_into().expect("eight")),
            0x0102_0304_0506_0708
        );
        assert_eq!(
            u32::from_le_bytes(price[14..18].try_into().expect("four")),
            0x090a_0b0c
        );
        let moved = GcPrivateShopMove {
            pos: 0x1011,
            change_pos: 0x1213,
        }
        .encode();
        assert_eq!(moved.len(), GC_PRIVATE_SHOP_MOVE_RECORD_SIZE);
        assert_eq!(moved[3], SUBHEADER_GC_PRIVATE_SHOP_ITEM_MOVE);
        assert_eq!(
            u16::from_le_bytes(moved[4..6].try_into().expect("two")),
            0x1011
        );
        assert_eq!(
            u16::from_le_bytes(moved[6..8].try_into().expect("two")),
            0x1213
        );
    }

    #[test]
    fn the_rename_encodes_in_field_order() {
        let mut title = [0u8; PRIVATE_SHOP_TITLE_FIELD_SIZE];
        title[..4].copy_from_slice(b"Axes");
        let changed = GcPrivateShopTitleChange { title }.encode();
        assert_eq!(
            changed.len(),
            GC_PRIVATE_SHOP_WIRE_SIZE + PRIVATE_SHOP_TITLE_FIELD_SIZE
        );
        assert_eq!(changed[3], SUBHEADER_GC_PRIVATE_SHOP_TITLE_CHANGE);
        assert_eq!(&changed[4..8], b"Axes");
        let titled = GcPrivateShopTitle {
            vid: 0x7531,
            title,
            title_type: 1,
        }
        .encode();
        assert_eq!(titled.len(), GC_PRIVATE_SHOP_TITLE_RECORD_SIZE);
        assert_eq!(titled[3], SUBHEADER_GC_PRIVATE_SHOP_TITLE);
        assert_eq!(
            u32::from_le_bytes(titled[4..8].try_into().expect("four")),
            0x7531
        );
        assert_eq!(&titled[8..12], b"Axes");
        assert_eq!(titled[titled.len() - 1], 1);
    }

    #[test]
    fn the_unlock_encodes_its_count() {
        let unlocked = GcPrivateShopUnlocked { slots: 0x1213 }.encode();
        assert_eq!(unlocked.len(), GC_PRIVATE_SHOP_REMOVE_RECORD_SIZE);
        assert_eq!(unlocked[3], SUBHEADER_GC_PRIVATE_SHOP_UNLOCKED_SLOTS_CHANGE);
        assert_eq!(
            u16::from_le_bytes(unlocked[4..6].try_into().expect("two")),
            0x1213
        );
    }

    #[test]
    fn a_load_encodes_in_field_order() {
        let mut title = [0u8; PRIVATE_SHOP_TITLE_FIELD_SIZE];
        title[..4].copy_from_slice(b"Axes");
        let record = GcPrivateShopLoad {
            title,
            gold: 0x0102_0304_0506_0708,
            cheque: 0x090a_0b0c,
            x: 0x0d0e_0f10,
            y: 0x1112_1314,
            channel: 1,
            state: PRIVATE_SHOP_STATE_OPEN,
            page_count: 2,
        }
        .encode();
        assert_eq!(record.len(), GC_PRIVATE_SHOP_LOAD_RECORD_SIZE);
        assert_eq!(record[0], HEADER_GC_PRIVATE_SHOP.value());
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 60);
        assert_eq!(record[3], SUBHEADER_GC_PRIVATE_SHOP_LOAD);
        assert_eq!(&record[4..8], b"Axes");
        assert_eq!(record[8], 0);
        assert_eq!(
            i64::from_le_bytes(record[37..45].try_into().expect("eight bytes")),
            0x0102_0304_0506_0708
        );
        assert_eq!(
            u32::from_le_bytes(record[45..49].try_into().expect("four bytes")),
            0x090a_0b0c
        );
        assert_eq!(
            i32::from_le_bytes(record[49..53].try_into().expect("four bytes")),
            0x0d0e_0f10
        );
        assert_eq!(
            i32::from_le_bytes(record[53..57].try_into().expect("four bytes")),
            0x1112_1314
        );
        assert_eq!(record[57], 1);
        assert_eq!(record[58], PRIVATE_SHOP_STATE_OPEN);
        assert_eq!(record[59], 2);
    }

    #[test]
    fn an_entity_encodes_in_field_order() {
        let mut owner = [0u8; PRIVATE_SHOP_OWNER_FIELD_SIZE];
        owner[..5].copy_from_slice(b"Alpha");
        let mut title = [0u8; PRIVATE_SHOP_TITLE_FIELD_SIZE];
        title[..4].copy_from_slice(b"Axes");
        let record = GcPrivateShopAddEntity {
            x: 0x0102_0304,
            y: 0x0506_0708,
            z: 0,
            vid: 0x090a_0b0c,
            vnum: 0x7531,
            owner,
            title_type: 1,
            title,
        }
        .encode();
        assert_eq!(record.len(), GC_PRIVATE_SHOP_ADD_ENTITY_RECORD_SIZE);
        assert_eq!(record[0], HEADER_GC_PRIVATE_SHOP.value());
        assert_eq!(u16::from_le_bytes([record[1], record[2]]), 83);
        assert_eq!(record[3], SUBHEADER_GC_PRIVATE_SHOP_ADD_ENTITY);
        assert_eq!(
            i32::from_le_bytes(record[4..8].try_into().expect("four bytes")),
            0x0102_0304
        );
        assert_eq!(
            i32::from_le_bytes(record[8..12].try_into().expect("four bytes")),
            0x0506_0708
        );
        assert_eq!(
            u32::from_le_bytes(record[16..20].try_into().expect("four bytes")),
            0x090a_0b0c
        );
        assert_eq!(
            u32::from_le_bytes(record[20..24].try_into().expect("four bytes")),
            0x7531
        );
        assert_eq!(&record[24..29], b"Alpha");
        assert_eq!(record[49], 1);
        assert_eq!(&record[50..54], b"Axes");
    }

    #[test]
    fn a_window_and_a_sale_encode_in_field_order() {
        let mut items = [GcPrivateShopItemData::empty(); 128];
        items[3] = GcPrivateShopItemData {
            vnum: 19,
            gold: 500,
            cheque: 0,
            checkin: 0x0102_0304,
            count: 1,
            pos: 3,
            sockets: [0; 6],
            attrs: [(0, 0); 7],
        };
        let mut title = [0u8; PRIVATE_SHOP_TITLE_FIELD_SIZE];
        title[..4].copy_from_slice(b"Axes");
        let record = GcPrivateShopStart {
            title,
            state: PRIVATE_SHOP_STATE_OPEN,
            page_count: 1,
            unlocked_slots: 0,
            vid: 0x7531,
            items,
        }
        .encode();
        assert_eq!(record.len(), GC_PRIVATE_SHOP_START_RECORD_SIZE);
        assert_eq!(record[0], HEADER_GC_PRIVATE_SHOP.value());
        assert_eq!(
            u16::from_le_bytes([record[1], record[2]]),
            u16::try_from(GC_PRIVATE_SHOP_START_RECORD_SIZE).expect("fits")
        );
        assert_eq!(record[3], SUBHEADER_GC_PRIVATE_SHOP_START);
        assert_eq!(&record[4..8], b"Axes");
        assert_eq!(record[37], PRIVATE_SHOP_STATE_OPEN);
        assert_eq!(record[38], 1);
        assert_eq!(u16::from_le_bytes([record[39], record[40]]), 0);
        assert_eq!(
            u32::from_le_bytes(record[41..45].try_into().expect("four bytes")),
            0x7531
        );
        // The ware's row at slot 3: vnum 19, gold 500, count 1, slot 3.
        let at = 45 + 3 * GC_PRIVATE_SHOP_ITEM_DATA_WIRE_SIZE;
        assert_eq!(
            u32::from_le_bytes(record[at..at + 4].try_into().expect("four bytes")),
            19
        );
        assert_eq!(
            i64::from_le_bytes(record[at + 4..at + 12].try_into().expect("eight bytes")),
            500
        );
        assert_eq!(
            i32::from_le_bytes(record[at + 16..at + 20].try_into().expect("four bytes")),
            0x0102_0304
        );
        assert_eq!(
            u32::from_le_bytes(record[at + 20..at + 24].try_into().expect("four bytes")),
            1
        );
        assert_eq!(
            u16::from_le_bytes(record[at + 24..at + 26].try_into().expect("two bytes")),
            3
        );
        // The sockets and attrs are empty.
        assert_eq!(&record[at + 26..at + 26 + 24], &[0u8; 24]);
        assert_eq!(&record[at + 50..at + 71], &[0u8; 21]);
        let mut customer = [0u8; PRIVATE_SHOP_OWNER_FIELD_SIZE];
        customer[..6].copy_from_slice(b"Yankee");
        let sale = GcPrivateShopSale {
            customer,
            time: 0x1112_1314,
            item: items[3],
        }
        .encode();
        assert_eq!(sale.len(), GC_PRIVATE_SHOP_SALE_RECORD_SIZE);
        assert_eq!(sale[3], SUBHEADER_GC_PRIVATE_SHOP_SET_SALE);
        assert_eq!(&sale[4..10], b"Yankee");
        assert_eq!(
            i32::from_le_bytes(sale[29..33].try_into().expect("four bytes")),
            0x1112_1314
        );
        assert_eq!(
            u32::from_le_bytes(sale[33..37].try_into().expect("four bytes")),
            19
        );
        let balance = GcPrivateShopBalance {
            gold: 500,
            cheque: 0,
        }
        .encode();
        assert_eq!(balance.len(), GC_PRIVATE_SHOP_BALANCE_RECORD_SIZE);
        assert_eq!(balance[3], SUBHEADER_GC_PRIVATE_SHOP_BALANCE_UPDATE);
        assert_eq!(
            i64::from_le_bytes(balance[4..12].try_into().expect("eight bytes")),
            500
        );
        let removed = GcPrivateShopRemove {
            subheader: SUBHEADER_GC_PRIVATE_SHOP_REMOVE_ITEM,
            pos: 3,
        }
        .encode();
        assert_eq!(
            removed,
            vec![HEADER_GC_PRIVATE_SHOP.value(), 6, 0, 13, 3, 0]
        );
    }

    #[test]
    fn a_state_update_encodes_its_two_bytes() {
        let record = GcPrivateShopState {
            state: 3,
            own: true,
        }
        .encode();
        assert_eq!(record, vec![HEADER_GC_PRIVATE_SHOP.value(), 6, 0, 15, 3, 1]);
    }

    #[test]
    fn a_bare_header_is_four_bytes() {
        assert_eq!(
            GcPrivateShop::new(SUBHEADER_GC_PRIVATE_SHOP_OPEN_PANEL).encode(),
            vec![
                HEADER_GC_PRIVATE_SHOP.value(),
                4,
                0,
                SUBHEADER_GC_PRIVATE_SHOP_OPEN_PANEL
            ]
        );
    }
}
