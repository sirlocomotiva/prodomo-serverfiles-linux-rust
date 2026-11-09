//! Transport-free codec for the legacy `HEADER_CG_PRIVATE_SHOP` (236) build record.
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
//! Only the build subheader is decoded here; the other twenty are later slices.
//! The subheader byte is validated by the dispatch, not by this codec: a
//! non-build subheader through `decode_build` is a header error, as a wrong
//! header is.
//!
//! # What this codec deliberately does not do
//!
//! The title stays raw bytes: no NUL or UTF-8 requirement. `CHARACTER::BuildPrivateShop`
//! (`server/server/game/char.cpp:11989-12127`) banword-checks and length-checks it, and the
//! shop truncates at 32 bytes. The poly vnum, the title type, the page count, the positions,
//! the prices and the display slots are opaque; the bounds on them belong to the shop.

use crate::cg_inventory::{CgHeader, HEADER_CG_PRIVATE_SHOP};
use crate::cg_variable::PRIVATE_SHOP_SUBHEADER_BUILD;
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
