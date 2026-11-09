//! Transport-free codec for the legacy `HEADER_CG_PRIVATE_SHOP` (236) build, browse and buy records.
//!
//! # Wire layout
//!
//! `server/server/game/packet.h:1226-1230` declares the two-byte prefix the
//! framer sizes, and `packet.h:1256-1263` the build's fixed part:
//!
//! ```cpp
//! typedef struct SPacketCGPrivateShop
//! {
//!     BYTE    bHeader;
//!     BYTE    bSubHeader;
//! } TPacketCGPrivateShop;
//!
//! typedef struct SPacketCGPrivateShopBuild
//! {
//!     char    szTitle[TITLE_MAX_LEN + 1];  // 33 bytes
//!     DWORD   dwPolyVnum;
//!     BYTE    bTitleType;
//!     BYTE    bPageCount;
//!     WORD    wItemCount;
//! } TPacketCGPrivateShopBuild;
//!
//! typedef struct SPrivateShopItem
//! {
//!     TItemPos    TPos;       // window_type u8, cell u16
//!     TItemPrice  TPrice;     // llGold i64, dwCheque u32
//!     WORD        wDisplayPos;
//! } TPrivateShopItem;
//! ```
//!
//! `TItemPrice` is at `server/server/common/tables.h:1917-1921`. The framing in
//! [`crate::cg_variable`] already sizes each frame the same way: the 2-byte
//! prefix, the 41-byte fixed part, and `wItemCount` 17-byte tables, checked in
//! `PrivateShopBuild` (`server/server/game/input_main.cpp:4136-4149`) before a
//! single field is read.
//!
//! The build, `START`, `BUY`, price-change, move, checkin, checkout, title-change,
//! slot-unlock and search subheaders are decoded here; the other eleven are later slices. The
//! subheader byte is validated by the dispatch, not by this codec: a wrong subheader
//! through any decoder is a header error, as a wrong header is.
//!
//! The search body is `TPacketCGPrivateShopSearch` (`packet.h:1303-1307`): the
//! 84-byte `TPrivateShopSearchFilter` (`packet.h:1276-1301`) and the `bool`, all
//! packed: 4 (vnum) + 25 (owner) + 4 + 4 (type, subtype) + 4 + 4 (job, gender) +
//! 2 + 2 (stack) + 1 + 1 (refine) + 4 + 4 (level) + 21 (seven attrs) + 2 (sash) +
//! 1 + 1 (alchemy) + 1 (use-filter) = 85.
//!
//! # What this codec deliberately does not do
//!
//! The title stays raw bytes: no NUL or UTF-8 requirement. `CHARACTER::BuildPrivateShop`
//! (`server/server/game/char.cpp:11989-12127`) banword-checks and length-checks it, and the
//! shop truncates at 32 bytes. The poly vnum, the title type, the page count, the positions,
//! the prices and the display slots are opaque; the bounds on them belong to the shop.

use crate::cg_inventory::{CgHeader, HEADER_CG_PRIVATE_SHOP};
use crate::cg_variable::{
    PRIVATE_SHOP_SUBHEADER_BUILD, PRIVATE_SHOP_SUBHEADER_BUY, PRIVATE_SHOP_SUBHEADER_ITEM_CHECKIN,
    PRIVATE_SHOP_SUBHEADER_ITEM_CHECKOUT, PRIVATE_SHOP_SUBHEADER_ITEM_MOVE,
    PRIVATE_SHOP_SUBHEADER_ITEM_PRICE_CHANGE, PRIVATE_SHOP_SUBHEADER_SEARCH,
    PRIVATE_SHOP_SUBHEADER_SLOT_UNLOCK_REQUEST, PRIVATE_SHOP_SUBHEADER_START,
    PRIVATE_SHOP_SUBHEADER_TITLE_CHANGE,
};
use crate::cg_wire::ClientFrame;
use crate::item_pos::ItemPos;

/// One decoded private-shop build: the title, the decoration, and the priced tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgPrivateShopBuild {
    /// The raw `char[TITLE_MAX_LEN + 1]` title field.
    pub title: [u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE],
    /// The polymorph vnum the shop stands as.
    pub poly_vnum: u32,
    /// The title's type byte.
    pub title_type: u8,
    /// The page count.
    pub page_count: u8,
    /// The priced tables, in wire order.
    pub tables: Vec<CgPrivateShopItem>,
}

/// One decoded `TPrivateShopItem`: the held item's place, its price, and the
/// display slot the client chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgPrivateShopItem {
    /// The held item's window and cell.
    pub pos: ItemPos,
    /// The asking price in yang.
    pub gold: i64,
    /// The asking price in cheques.
    pub cheque: u32,
    /// The shop slot the client chose.
    pub display_pos: u16,
}

/// Width of the title field: `TITLE_MAX_LEN + 1` (`C/length.h:146`).
pub const CG_PRIVATE_SHOP_TITLE_FIELD_SIZE: usize = 33;

/// Width of one item table on the wire: `TItemPos` (3), `TItemPrice` (12),
/// `wDisplayPos` (2).
pub const CG_PRIVATE_SHOP_ITEM_WIRE_SIZE: usize = 17;

/// Width of the build's fixed part after the prefix: the title, the poly vnum,
/// the title type, the page count, and the item count.
pub const CG_PRIVATE_SHOP_BUILD_FIXED_SIZE: usize = 41;

/// Width of the whole prefix before the tables: the two framer bytes and the
/// build's fixed part.
pub const CG_PRIVATE_SHOP_BUILD_PREFIX_SIZE: usize = 2 + CG_PRIVATE_SHOP_BUILD_FIXED_SIZE;

/// What was wrong with a private-shop build record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgPrivateShopBuildError {
    /// The frame's header byte is not `HEADER_CG_PRIVATE_SHOP`, or its
    /// subheader is not the build's.
    Header {
        /// The header byte that arrived.
        header: u8,
        /// The subheader byte that arrived.
        subheader: u8,
    },
    /// The payload holds fewer bytes than the prefix, or fewer tables than the
    /// count names.
    Truncated {
        /// How many bytes the record needs.
        needed: usize,
        /// How many arrived.
        available: usize,
    },
}

impl std::fmt::Display for CgPrivateShopBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Header { header, subheader } => {
                write!(f, "not a private-shop build: {header}/{subheader}")
            }
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "a private-shop build needs {needed} bytes, {available} arrived"
                )
            }
        }
    }
}

impl std::error::Error for CgPrivateShopBuildError {}

/// One decoded `START`: the VID of the stand the client clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgPrivateShopStart {
    /// The clicked stand's VID.
    pub vid: u32,
}

/// One decoded `ITEM_PRICE_CHANGE`: the shop slot and its new price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgPrivateShopPriceChange {
    /// The shop slot.
    pub pos: u16,
    /// The new price in yang.
    pub gold: i64,
    /// The new price in cheques.
    pub cheque: u32,
}

impl CgPrivateShopPriceChange {
    /// Decode the subheader, the word and the price (`PrivateShopItemPriceChange`,
    /// `G/input_main.cpp:4481-4569`).
    ///
    /// # Errors
    ///
    /// As [`CgPrivateShopBuildError`]: a wrong header or subheader, or a frame
    /// short of the word and the price.
    ///
    /// # Panics
    ///
    /// Never: the slice is length-checked first.
    pub fn decode_price_change(frame: &ClientFrame) -> Result<Self, CgPrivateShopBuildError> {
        let bytes = &frame.payload;
        let subheader = bytes.first().copied().unwrap_or(0);
        if frame.header != HEADER_CG_PRIVATE_SHOP.value()
            || subheader != PRIVATE_SHOP_SUBHEADER_ITEM_PRICE_CHANGE
        {
            return Err(CgPrivateShopBuildError::Header {
                header: frame.header,
                subheader,
            });
        }
        if bytes.len() < 1 + 2 + 12 {
            return Err(CgPrivateShopBuildError::Truncated {
                needed: 1 + 2 + 12,
                available: bytes.len(),
            });
        }
        Ok(Self {
            pos: u16::from_le_bytes(bytes[1..3].try_into().expect("two bytes")),
            gold: i64::from_le_bytes(bytes[3..11].try_into().expect("eight bytes")),
            cheque: u32::from_le_bytes(bytes[11..15].try_into().expect("four bytes")),
        })
    }
}

/// One decoded `ITEM_MOVE`: the ware's slot and where it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgPrivateShopMove {
    /// The ware's shop slot.
    pub pos: u16,
    /// The shop slot it moves to.
    pub change_pos: u16,
}

impl CgPrivateShopMove {
    /// Decode the subheader and the two words (`PrivateShopItemMove`,
    /// `G/input_main.cpp:4570-4649`).
    ///
    /// # Errors
    ///
    /// As [`CgPrivateShopBuildError`]: a wrong header or subheader, or a frame
    /// short of the two words.
    ///
    /// # Panics
    ///
    /// Never: the slice is length-checked first.
    pub fn decode_move(frame: &ClientFrame) -> Result<Self, CgPrivateShopBuildError> {
        let bytes = &frame.payload;
        let subheader = bytes.first().copied().unwrap_or(0);
        if frame.header != HEADER_CG_PRIVATE_SHOP.value()
            || subheader != PRIVATE_SHOP_SUBHEADER_ITEM_MOVE
        {
            return Err(CgPrivateShopBuildError::Header {
                header: frame.header,
                subheader,
            });
        }
        if bytes.len() < 1 + 4 {
            return Err(CgPrivateShopBuildError::Truncated {
                needed: 1 + 4,
                available: bytes.len(),
            });
        }
        Ok(Self {
            pos: u16::from_le_bytes(bytes[1..3].try_into().expect("two bytes")),
            change_pos: u16::from_le_bytes(bytes[3..5].try_into().expect("two bytes")),
        })
    }
}

/// One decoded `ITEM_CHECKIN`: the held item, its price, and where it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgPrivateShopCheckin {
    /// The held item's window and cell.
    pub src: ItemPos,
    /// The asking price in yang.
    pub gold: i64,
    /// The asking price in cheques.
    pub cheque: u32,
    /// The shop slot the client chose, or a negative pick for the first free one.
    pub dst_pos: i32,
}

impl CgPrivateShopCheckin {
    /// Decode the subheader, the place, the price and the picked slot
    /// (`PrivateShopItemCheckin`, `G/input_main.cpp:4651-4779`).
    ///
    /// # Errors
    ///
    /// As [`CgPrivateShopBuildError`]: a wrong header or subheader, or a frame
    /// short of the place, the price and the pick.
    ///
    /// # Panics
    ///
    /// Never: the slice is length-checked first.
    pub fn decode_checkin(frame: &ClientFrame) -> Result<Self, CgPrivateShopBuildError> {
        let bytes = &frame.payload;
        let subheader = bytes.first().copied().unwrap_or(0);
        if frame.header != HEADER_CG_PRIVATE_SHOP.value()
            || subheader != PRIVATE_SHOP_SUBHEADER_ITEM_CHECKIN
        {
            return Err(CgPrivateShopBuildError::Header {
                header: frame.header,
                subheader,
            });
        }
        if bytes.len() < 1 + 3 + 12 + 4 {
            return Err(CgPrivateShopBuildError::Truncated {
                needed: 1 + 3 + 12 + 4,
                available: bytes.len(),
            });
        }
        Ok(Self {
            src: ItemPos::decode_at(bytes, 1),
            gold: i64::from_le_bytes(bytes[4..12].try_into().expect("eight bytes")),
            cheque: u32::from_le_bytes(bytes[12..16].try_into().expect("four bytes")),
            dst_pos: i32::from_le_bytes(bytes[16..20].try_into().expect("four bytes")),
        })
    }
}

/// One decoded `ITEM_CHECKOUT`: the ware's slot and where it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgPrivateShopCheckout {
    /// The ware's shop slot.
    pub src_pos: u16,
    /// The picked inventory cell, or a negative pick for the first free one.
    pub dst_pos: i32,
}

impl CgPrivateShopCheckout {
    /// Decode the subheader, the word and the picked cell (`PrivateShopItemCheckout`,
    /// `G/input_main.cpp:4781-4961`).
    ///
    /// # Errors
    ///
    /// As [`CgPrivateShopBuildError`]: a wrong header or subheader, or a frame
    /// short of the word and the pick.
    ///
    /// # Panics
    ///
    /// Never: the slice is length-checked first.
    pub fn decode_checkout(frame: &ClientFrame) -> Result<Self, CgPrivateShopBuildError> {
        let bytes = &frame.payload;
        let subheader = bytes.first().copied().unwrap_or(0);
        if frame.header != HEADER_CG_PRIVATE_SHOP.value()
            || subheader != PRIVATE_SHOP_SUBHEADER_ITEM_CHECKOUT
        {
            return Err(CgPrivateShopBuildError::Header {
                header: frame.header,
                subheader,
            });
        }
        if bytes.len() < 1 + 2 + 4 {
            return Err(CgPrivateShopBuildError::Truncated {
                needed: 1 + 2 + 4,
                available: bytes.len(),
            });
        }
        Ok(Self {
            src_pos: u16::from_le_bytes(bytes[1..3].try_into().expect("two bytes")),
            dst_pos: i32::from_le_bytes(bytes[3..7].try_into().expect("four bytes")),
        })
    }
}

/// One decoded `TITLE_CHANGE`: the raw 33-byte title field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgPrivateShopTitleChange {
    /// The raw `char[TITLE_MAX_LEN + 1]` title field.
    pub title: [u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE],
}

impl CgPrivateShopTitleChange {
    /// Decode the subheader and the 33 title bytes (`PrivateShopTitleChange`,
    /// `G/input_main.cpp:4961-5005`).
    ///
    /// # Errors
    ///
    /// As [`CgPrivateShopBuildError`]: a wrong header or subheader, or a frame
    /// short of the title.
    ///
    /// # Panics
    ///
    /// Never: the slice is length-checked first.
    pub fn decode_title_change(frame: &ClientFrame) -> Result<Self, CgPrivateShopBuildError> {
        let bytes = &frame.payload;
        let subheader = bytes.first().copied().unwrap_or(0);
        if frame.header != HEADER_CG_PRIVATE_SHOP.value()
            || subheader != PRIVATE_SHOP_SUBHEADER_TITLE_CHANGE
        {
            return Err(CgPrivateShopBuildError::Header {
                header: frame.header,
                subheader,
            });
        }
        if bytes.len() < 1 + CG_PRIVATE_SHOP_TITLE_FIELD_SIZE {
            return Err(CgPrivateShopBuildError::Truncated {
                needed: 1 + CG_PRIVATE_SHOP_TITLE_FIELD_SIZE,
                available: bytes.len(),
            });
        }
        let mut title = [0u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE];
        title.copy_from_slice(&bytes[1..=CG_PRIVATE_SHOP_TITLE_FIELD_SIZE]);
        Ok(Self { title })
    }
}

/// One decoded `SEARCH`: the filter and whether it applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgPrivateShopSearch {
    /// The searched vnum, or 0 for any.
    pub vnum: u32,
    /// The raw `char[CHARACTER_NAME_MAX_LEN + 1]` owner field.
    pub owner: [u8; CG_PRIVATE_SHOP_SEARCH_OWNER_FIELD_SIZE],
    /// The item type, or a negative pick for any.
    pub item_type: i32,
    /// The item subtype, or a negative pick for any.
    pub item_subtype: i32,
    /// The job, or a negative pick for any.
    pub job: i32,
    /// The gender, or a negative pick for any.
    pub gender: i32,
    /// The smallest stack, or 0 for any.
    pub min_stack: u16,
    /// The largest stack, or 0 for any.
    pub max_stack: u16,
    /// The smallest refine, or 0 for any.
    pub min_refine: u8,
    /// The largest refine, or 0 for any.
    pub max_refine: u8,
    /// The smallest level, or 0 for any.
    pub min_level: u32,
    /// The largest level, or 0 for any.
    pub max_level: u32,
    /// The seven attributes, where a zero type asks for nothing.
    pub attrs: [(u8, i16); CG_PRIVATE_SHOP_SEARCH_ATTRS],
    /// The sash absorption.
    pub sash_absorption: u16,
    /// The alchemy level.
    pub alchemy_level: u8,
    /// The alchemy clarity.
    pub alchemy_clarity: u8,
    /// Whether the filter applies.
    pub use_filter: bool,
}

/// Width of the search's owner field: `CHARACTER_NAME_MAX_LEN + 1`.
pub const CG_PRIVATE_SHOP_SEARCH_OWNER_FIELD_SIZE: usize = 25;

/// The search filter's attributes: `ITEM_ATTRIBUTE_MAX_NUM`.
pub const CG_PRIVATE_SHOP_SEARCH_ATTRS: usize = 7;

/// Width of the search body after the prefix: the filter and the flag.
pub const CG_PRIVATE_SHOP_SEARCH_WIRE_SIZE: usize = 85;

impl CgPrivateShopSearch {
    /// Decode the subheader, the filter and the flag (`PrivateShopSearch`,
    /// `G/input_main.cpp:5140-5176`).
    ///
    /// # Errors
    ///
    /// As [`CgPrivateShopBuildError`]: a wrong header or subheader, or a frame
    /// short of the filter and the flag.
    ///
    /// # Panics
    ///
    /// Never: the slice is length-checked first.
    pub fn decode_search(frame: &ClientFrame) -> Result<Self, CgPrivateShopBuildError> {
        let bytes = &frame.payload;
        let subheader = bytes.first().copied().unwrap_or(0);
        if frame.header != HEADER_CG_PRIVATE_SHOP.value()
            || subheader != PRIVATE_SHOP_SUBHEADER_SEARCH
        {
            return Err(CgPrivateShopBuildError::Header {
                header: frame.header,
                subheader,
            });
        }
        if bytes.len() < 1 + CG_PRIVATE_SHOP_SEARCH_WIRE_SIZE {
            return Err(CgPrivateShopBuildError::Truncated {
                needed: 1 + CG_PRIVATE_SHOP_SEARCH_WIRE_SIZE,
                available: bytes.len(),
            });
        }
        let mut owner = [0u8; CG_PRIVATE_SHOP_SEARCH_OWNER_FIELD_SIZE];
        owner.copy_from_slice(&bytes[5..30]);
        let mut attrs = [(0u8, 0i16); CG_PRIVATE_SHOP_SEARCH_ATTRS];
        for (index, attr) in attrs.iter_mut().enumerate() {
            let at = 60 + index * 3;
            let value = i16::from_le_bytes(bytes[at + 1..at + 3].try_into().expect("two bytes"));
            *attr = (bytes[at], value);
        }
        Ok(Self {
            vnum: u32::from_le_bytes(bytes[1..5].try_into().expect("four bytes")),
            owner,
            item_type: i32::from_le_bytes(bytes[30..34].try_into().expect("four bytes")),
            item_subtype: i32::from_le_bytes(bytes[34..38].try_into().expect("four bytes")),
            job: i32::from_le_bytes(bytes[38..42].try_into().expect("four bytes")),
            gender: i32::from_le_bytes(bytes[42..46].try_into().expect("four bytes")),
            min_stack: u16::from_le_bytes(bytes[46..48].try_into().expect("two bytes")),
            max_stack: u16::from_le_bytes(bytes[48..50].try_into().expect("two bytes")),
            min_refine: bytes[50],
            max_refine: bytes[51],
            min_level: u32::from_le_bytes(bytes[52..56].try_into().expect("four bytes")),
            max_level: u32::from_le_bytes(bytes[56..60].try_into().expect("four bytes")),
            attrs,
            sash_absorption: u16::from_le_bytes(bytes[81..83].try_into().expect("two bytes")),
            alchemy_level: bytes[83],
            alchemy_clarity: bytes[84],
            use_filter: bytes[85] != 0,
        })
    }
}

/// One decoded `SLOT_UNLOCK_REQUEST`: the key's inventory cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgPrivateShopSlotUnlock {
    /// The key's inventory cell.
    pub pos: u16,
}

impl CgPrivateShopSlotUnlock {
    /// Decode the subheader and the bare `WORD` (`PrivateShopSlotUnlockRequest`,
    /// `G/input_main.cpp:5047-5135`).
    ///
    /// # Errors
    ///
    /// As [`CgPrivateShopBuildError`]: a wrong header or subheader, or a frame
    /// short of the two cell bytes.
    ///
    /// # Panics
    ///
    /// Never: the slice is length-checked first.
    pub fn decode_slot_unlock(frame: &ClientFrame) -> Result<Self, CgPrivateShopBuildError> {
        let bytes = &frame.payload;
        let subheader = bytes.first().copied().unwrap_or(0);
        if frame.header != HEADER_CG_PRIVATE_SHOP.value()
            || subheader != PRIVATE_SHOP_SUBHEADER_SLOT_UNLOCK_REQUEST
        {
            return Err(CgPrivateShopBuildError::Header {
                header: frame.header,
                subheader,
            });
        }
        if bytes.len() < 1 + 2 {
            return Err(CgPrivateShopBuildError::Truncated {
                needed: 1 + 2,
                available: bytes.len(),
            });
        }
        Ok(Self {
            pos: u16::from_le_bytes(bytes[1..3].try_into().expect("two bytes")),
        })
    }
}

impl CgPrivateShopStart {
    /// Decode the subheader and the bare `DWORD` (`PrivateShopStart`,
    /// `G/input_main.cpp:4271-4322`).
    ///
    /// # Errors
    ///
    /// As [`CgPrivateShopBuildError`]: a wrong header or subheader, or a frame
    /// short of the four VID bytes.
    ///
    /// # Panics
    ///
    /// Never: the slice is length-checked first.
    pub fn decode_start(frame: &ClientFrame) -> Result<Self, CgPrivateShopBuildError> {
        let bytes = &frame.payload;
        let subheader = bytes.first().copied().unwrap_or(0);
        if frame.header != HEADER_CG_PRIVATE_SHOP.value()
            || subheader != PRIVATE_SHOP_SUBHEADER_START
        {
            return Err(CgPrivateShopBuildError::Header {
                header: frame.header,
                subheader,
            });
        }
        if bytes.len() < 1 + 4 {
            return Err(CgPrivateShopBuildError::Truncated {
                needed: 1 + 4,
                available: bytes.len(),
            });
        }
        Ok(Self {
            vid: u32::from_le_bytes(bytes[1..5].try_into().expect("four bytes")),
        })
    }
}

/// One decoded `BUY`: the shop slot the client wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgPrivateShopBuy {
    /// The shop slot.
    pub pos: u16,
}

impl CgPrivateShopBuy {
    /// Decode the subheader and the bare `WORD` (`PrivateShopBuy`,
    /// `G/input_main.cpp:4332-4395`).
    ///
    /// # Errors
    ///
    /// As [`CgPrivateShopBuildError`]: a wrong header or subheader, or a frame
    /// short of the two slot bytes.
    ///
    /// # Panics
    ///
    /// Never: the slice is length-checked first.
    pub fn decode_buy(frame: &ClientFrame) -> Result<Self, CgPrivateShopBuildError> {
        let bytes = &frame.payload;
        let subheader = bytes.first().copied().unwrap_or(0);
        if frame.header != HEADER_CG_PRIVATE_SHOP.value() || subheader != PRIVATE_SHOP_SUBHEADER_BUY
        {
            return Err(CgPrivateShopBuildError::Header {
                header: frame.header,
                subheader,
            });
        }
        if bytes.len() < 1 + 2 {
            return Err(CgPrivateShopBuildError::Truncated {
                needed: 1 + 2,
                available: bytes.len(),
            });
        }
        Ok(Self {
            pos: u16::from_le_bytes(bytes[1..3].try_into().expect("two bytes")),
        })
    }
}

impl CgPrivateShopBuild {
    /// The legacy header byte.
    #[must_use]
    pub const fn header() -> CgHeader {
        HEADER_CG_PRIVATE_SHOP
    }

    /// Decode a whole frame: the prefix, the fixed part, and the count's tables.
    ///
    /// # Errors
    ///
    /// Returns [`CgPrivateShopBuildError::Header`] for a wrong header or
    /// subheader, and [`CgPrivateShopBuildError::Truncated`] when the frame is
    /// short.
    ///
    /// # Panics
    ///
    /// Never: every conversion reads bytes the length checks proved present.
    pub fn decode_build(frame: &ClientFrame) -> Result<Self, CgPrivateShopBuildError> {
        let bytes = &frame.payload;
        let subheader = bytes.first().copied().unwrap_or(0);
        if frame.header != HEADER_CG_PRIVATE_SHOP.value()
            || subheader != PRIVATE_SHOP_SUBHEADER_BUILD
        {
            return Err(CgPrivateShopBuildError::Header {
                header: frame.header,
                subheader,
            });
        }
        // The payload's first byte is the subheader; the fixed part follows it.
        if bytes.len() < 1 + CG_PRIVATE_SHOP_BUILD_FIXED_SIZE {
            return Err(CgPrivateShopBuildError::Truncated {
                needed: 1 + CG_PRIVATE_SHOP_BUILD_FIXED_SIZE,
                available: bytes.len(),
            });
        }
        let mut title = [0u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE];
        title.copy_from_slice(&bytes[1..=CG_PRIVATE_SHOP_TITLE_FIELD_SIZE]);
        let poly_vnum = u32::from_le_bytes(bytes[34..38].try_into().expect("four bytes"));
        let title_type = bytes[38];
        let page_count = bytes[39];
        let count = usize::from(u16::from_le_bytes(
            bytes[40..42].try_into().expect("two bytes"),
        ));
        let needed = 1 + CG_PRIVATE_SHOP_BUILD_FIXED_SIZE + count * CG_PRIVATE_SHOP_ITEM_WIRE_SIZE;
        if bytes.len() < needed {
            return Err(CgPrivateShopBuildError::Truncated {
                needed,
                available: bytes.len(),
            });
        }
        let mut tables = Vec::with_capacity(count);
        for index in 0..count {
            let at = 1 + CG_PRIVATE_SHOP_BUILD_FIXED_SIZE + index * CG_PRIVATE_SHOP_ITEM_WIRE_SIZE;
            let pos = ItemPos::decode_at(bytes, at);
            let gold = i64::from_le_bytes(bytes[at + 3..at + 11].try_into().expect("eight bytes"));
            let cheque =
                u32::from_le_bytes(bytes[at + 11..at + 15].try_into().expect("four bytes"));
            let display_pos =
                u16::from_le_bytes(bytes[at + 15..at + 17].try_into().expect("two bytes"));
            tables.push(CgPrivateShopItem {
                pos,
                gold,
                cheque,
                display_pos,
            });
        }
        Ok(Self {
            title,
            poly_vnum,
            title_type,
            page_count,
            tables,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A build frame with distinct byte halves, so an endianness swap shows.
    fn golden() -> ClientFrame {
        let mut payload = vec![PRIVATE_SHOP_SUBHEADER_BUILD];
        let mut title = [0u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE];
        title[..4].copy_from_slice(b"Axes");
        payload.extend_from_slice(&title);
        payload.extend_from_slice(&0x7531u32.to_le_bytes());
        payload.push(0x01);
        payload.push(0x02);
        payload.extend_from_slice(&2u16.to_le_bytes());
        // Table one: window 1, cell 0x0203, gold 0x0405_0607_0809_0a0b, cheque
        // 0x0c0d0e0f, display 0x1011.
        payload.extend_from_slice(&[1, 0x03, 0x02]);
        payload.extend_from_slice(&[0x0b, 0x0a, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04]);
        payload.extend_from_slice(&[0x0f, 0x0e, 0x0d, 0x0c]);
        payload.extend_from_slice(&[0x11, 0x10]);
        // Table two: window 9, cell 0x1213, no gold, one cheque, display 0x1415.
        payload.extend_from_slice(&[9, 0x13, 0x12]);
        payload.extend_from_slice(&0i64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&[0x15, 0x14]);
        ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload,
        }
    }

    #[test]
    fn a_build_decodes_its_title_and_both_tables() {
        let frame = golden();
        let build = CgPrivateShopBuild::decode_build(&frame).expect("decoded");
        assert_eq!(&build.title[..4], b"Axes");
        assert_eq!(build.title[4], 0);
        assert_eq!(build.poly_vnum, 0x7531);
        assert_eq!(build.title_type, 0x01);
        assert_eq!(build.page_count, 0x02);
        assert_eq!(
            build.tables,
            vec![
                CgPrivateShopItem {
                    pos: ItemPos::new(1, 0x0203),
                    gold: 0x0405_0607_0809_0a0b,
                    cheque: 0x0c0d_0e0f,
                    display_pos: 0x1011,
                },
                CgPrivateShopItem {
                    pos: ItemPos::new(9, 0x1213),
                    gold: 0,
                    cheque: 1,
                    display_pos: 0x1415,
                },
            ]
        );
    }

    #[test]
    fn a_wrong_subheader_is_a_header_error() {
        let mut frame = golden();
        frame.payload[0] = PRIVATE_SHOP_SUBHEADER_BUILD + 1;
        assert_eq!(
            CgPrivateShopBuild::decode_build(&frame),
            Err(CgPrivateShopBuildError::Header {
                header: HEADER_CG_PRIVATE_SHOP.value(),
                subheader: PRIVATE_SHOP_SUBHEADER_BUILD + 1,
            })
        );
    }

    #[test]
    fn a_start_decodes_its_vid_and_a_buy_its_slot() {
        let start = CgPrivateShopStart::decode_start(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_START, 0x31, 0x75, 0x00, 0x00],
        })
        .expect("decoded");
        assert_eq!(start.vid, 0x7531);
        let buy = CgPrivateShopBuy::decode_buy(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_BUY, 0x13, 0x12],
        })
        .expect("decoded");
        assert_eq!(buy.pos, 0x1213);
        // Each through the other's decoder is a header error.
        let crossed = CgPrivateShopBuy::decode_buy(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_START, 0x31, 0x75, 0x00, 0x00],
        });
        assert_eq!(
            crossed,
            Err(CgPrivateShopBuildError::Header {
                header: HEADER_CG_PRIVATE_SHOP.value(),
                subheader: PRIVATE_SHOP_SUBHEADER_START,
            })
        );
        let short = CgPrivateShopStart::decode_start(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_START, 0x31],
        });
        assert_eq!(
            short,
            Err(CgPrivateShopBuildError::Truncated {
                needed: 5,
                available: 2,
            })
        );
    }

    #[test]
    fn the_four_edits_decode_in_field_order() {
        let price = CgPrivateShopPriceChange::decode_price_change(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: {
                let mut payload = vec![PRIVATE_SHOP_SUBHEADER_ITEM_PRICE_CHANGE, 0x11, 0x10];
                payload.extend_from_slice(&[0x0b, 0x0a, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04]);
                payload.extend_from_slice(&[0x0f, 0x0e, 0x0d, 0x0c]);
                payload
            },
        })
        .expect("decoded");
        assert_eq!(price.pos, 0x1011);
        assert_eq!(price.gold, 0x0405_0607_0809_0a0b);
        assert_eq!(price.cheque, 0x0c0d_0e0f);
        let moved = CgPrivateShopMove::decode_move(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_ITEM_MOVE, 0x11, 0x10, 0x13, 0x12],
        })
        .expect("decoded");
        assert_eq!(moved.pos, 0x1011);
        assert_eq!(moved.change_pos, 0x1213);
        let checkin = CgPrivateShopCheckin::decode_checkin(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: {
                let mut payload = vec![PRIVATE_SHOP_SUBHEADER_ITEM_CHECKIN, 1, 0x03, 0x02];
                payload.extend_from_slice(&0x0405_0607_0809_0a0bi64.to_le_bytes());
                payload.extend_from_slice(&0x0c0d_0e0fu32.to_le_bytes());
                payload.extend_from_slice(&(-1i32).to_le_bytes());
                payload
            },
        })
        .expect("decoded");
        assert_eq!(checkin.src, ItemPos::new(1, 0x0203));
        assert_eq!(checkin.gold, 0x0405_0607_0809_0a0b);
        assert_eq!(checkin.cheque, 0x0c0d_0e0f);
        assert_eq!(checkin.dst_pos, -1);
        let checkout = CgPrivateShopCheckout::decode_checkout(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![
                PRIVATE_SHOP_SUBHEADER_ITEM_CHECKOUT,
                0x11,
                0x10,
                0x14,
                0x13,
                0x12,
                0x11,
            ],
        })
        .expect("decoded");
        assert_eq!(checkout.src_pos, 0x1011);
        assert_eq!(checkout.dst_pos, 0x1112_1314);
        // Each through the price decoder is a header error.
        let crossed = CgPrivateShopPriceChange::decode_price_change(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_ITEM_MOVE, 0x11, 0x10, 0x13, 0x12],
        });
        assert_eq!(
            crossed,
            Err(CgPrivateShopBuildError::Header {
                header: HEADER_CG_PRIVATE_SHOP.value(),
                subheader: PRIVATE_SHOP_SUBHEADER_ITEM_MOVE,
            })
        );
        let short = CgPrivateShopCheckout::decode_checkout(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_ITEM_CHECKOUT, 0x11],
        });
        assert_eq!(
            short,
            Err(CgPrivateShopBuildError::Truncated {
                needed: 7,
                available: 2,
            })
        );
    }

    #[test]
    fn the_title_and_the_unlock_decode_in_field_order() {
        let mut payload = vec![PRIVATE_SHOP_SUBHEADER_TITLE_CHANGE];
        let mut title = [0u8; CG_PRIVATE_SHOP_TITLE_FIELD_SIZE];
        title[..4].copy_from_slice(b"Axes");
        payload.extend_from_slice(&title);
        let renamed = CgPrivateShopTitleChange::decode_title_change(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload,
        })
        .expect("decoded");
        assert_eq!(&renamed.title[..4], b"Axes");
        assert_eq!(renamed.title[4], 0);
        let unlock = CgPrivateShopSlotUnlock::decode_slot_unlock(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_SLOT_UNLOCK_REQUEST, 0x13, 0x12],
        })
        .expect("decoded");
        assert_eq!(unlock.pos, 0x1213);
        let crossed = CgPrivateShopSlotUnlock::decode_slot_unlock(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_TITLE_CHANGE, 0x13, 0x12],
        });
        assert_eq!(
            crossed,
            Err(CgPrivateShopBuildError::Header {
                header: HEADER_CG_PRIVATE_SHOP.value(),
                subheader: PRIVATE_SHOP_SUBHEADER_TITLE_CHANGE,
            })
        );
        let short = CgPrivateShopTitleChange::decode_title_change(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_TITLE_CHANGE, b'A'],
        });
        assert_eq!(
            short,
            Err(CgPrivateShopBuildError::Truncated {
                needed: 34,
                available: 2,
            })
        );
    }

    #[test]
    fn the_search_decodes_in_field_order() {
        let mut payload = vec![PRIVATE_SHOP_SUBHEADER_SEARCH];
        payload.extend_from_slice(&0x7531u32.to_le_bytes());
        let mut owner = [0u8; CG_PRIVATE_SHOP_SEARCH_OWNER_FIELD_SIZE];
        owner[..6].copy_from_slice(b"Seller");
        payload.extend_from_slice(&owner);
        payload.extend_from_slice(&1i32.to_le_bytes());
        payload.extend_from_slice(&(-1i32).to_le_bytes());
        payload.extend_from_slice(&0i32.to_le_bytes());
        payload.extend_from_slice(&1i32.to_le_bytes());
        payload.extend_from_slice(&0x0203u16.to_le_bytes());
        payload.extend_from_slice(&0x0405u16.to_le_bytes());
        payload.push(3);
        payload.push(9);
        payload.extend_from_slice(&0x0a0bu32.to_le_bytes());
        payload.extend_from_slice(&0x0c0du32.to_le_bytes());
        payload.push(17);
        payload.extend_from_slice(&0x090ai16.to_le_bytes());
        payload.extend_from_slice(&[0u8; 18]);
        payload.extend_from_slice(&0x0e0fu16.to_le_bytes());
        payload.push(4);
        payload.push(5);
        payload.push(1);
        assert_eq!(payload.len(), 1 + CG_PRIVATE_SHOP_SEARCH_WIRE_SIZE);
        let search = CgPrivateShopSearch::decode_search(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: payload.clone(),
        })
        .expect("decoded");
        assert_eq!(search.vnum, 0x7531);
        assert_eq!(&search.owner[..6], b"Seller");
        assert_eq!(search.owner[6], 0);
        assert_eq!(search.item_type, 1);
        assert_eq!(search.item_subtype, -1);
        assert_eq!(search.job, 0);
        assert_eq!(search.gender, 1);
        assert_eq!(search.min_stack, 0x0203);
        assert_eq!(search.max_stack, 0x0405);
        assert_eq!(search.min_refine, 3);
        assert_eq!(search.max_refine, 9);
        assert_eq!(search.min_level, 0x0a0b);
        assert_eq!(search.max_level, 0x0c0d);
        assert_eq!(search.attrs[0], (17, 0x090a));
        assert_eq!(search.attrs[1], (0, 0));
        assert_eq!(search.sash_absorption, 0x0e0f);
        assert_eq!(search.alchemy_level, 4);
        assert_eq!(search.alchemy_clarity, 5);
        assert!(search.use_filter);
        let crossed = CgPrivateShopSearch::decode_search(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_BUY, 0x13, 0x12],
        });
        assert_eq!(
            crossed,
            Err(CgPrivateShopBuildError::Header {
                header: HEADER_CG_PRIVATE_SHOP.value(),
                subheader: PRIVATE_SHOP_SUBHEADER_BUY,
            })
        );
        let short = CgPrivateShopSearch::decode_search(&ClientFrame {
            header: HEADER_CG_PRIVATE_SHOP.value(),
            payload: vec![PRIVATE_SHOP_SUBHEADER_SEARCH, 0x31],
        });
        assert_eq!(
            short,
            Err(CgPrivateShopBuildError::Truncated {
                needed: 86,
                available: 2,
            })
        );
    }

    #[test]
    fn a_short_table_is_truncated() {
        let mut frame = golden();
        frame.payload.pop();
        let available = frame.payload.len();
        assert_eq!(
            CgPrivateShopBuild::decode_build(&frame),
            Err(CgPrivateShopBuildError::Truncated {
                needed: available + 1,
                available,
            })
        );
    }
}
