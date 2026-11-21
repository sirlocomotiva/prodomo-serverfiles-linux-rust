//! Transport-free codec for the legacy `HEADER_CG_MYSHOP` (55) record.
//!
//! # Wire layout
//!
//! `server/server/game/packet.h:2361-2369` declares the bundle-build prefix, and
//! `server/server/common/tables.h:796-818` declares each item table
//! (`ENABLE_RENEWAL_SHOPEX` and `ENABLE_REMOVE_LIMIT_GOLD` are defined):
//!
//! ```cpp
//! typedef struct SPacketCGMyShop
//! {
//!     BYTE    bHeader;
//!     char    szSign[SHOP_SIGN_MAX_LEN + 1];  // 33 bytes
//!     BYTE    bCount;
//! } TPacketCGMyShop;
//!
//! typedef struct SShopItemTable
//! {
//!     DWORD   vnum;
//!     WORD    count;
//!     TItemPos pos;             // window_type u8, cell u16
//!     unsigned long long price;
//!     BYTE    display_pos;
//!     long    alSockets[ITEM_SOCKET_MAX_NUM];      // 6 x 4
//!     TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];  // 7 x 3
//!     BYTE    price_type;
//!     DWORD   price_vnum;
//! } TShopItemTable;
//! ```
//!
//! The framing in [`crate::cg_variable`] already sizes each frame the same way:
//! the 35-byte prefix plus `bCount` 68-byte tables.
//!
//! `CInputMain::MyShop` (`server/server/game/input_main.cpp:3255-3314`) reads only
//! the sign and the count from the prefix; `CHARACTER::OpenMyShop`
//! (`server/server/game/char.cpp:805-975`) reads only `pos` and `price` from each
//! table, and `CShop::SetShopItems` (`server/server/game/shop.cpp:90-197`) reads
//! only `display_pos` more for a PC shop. The SHOPEX tail (sockets, attributes,
//! `price_type`, `price_vnum`) is never read on this path, so it is consumed and
//! kept as it arrived.
//!
//! # What this codec deliberately does not do
//!
//! The sign stays raw bytes: no NUL or UTF-8 requirement. The shop truncates it
//! at 32 bytes and refuses an empty one. The count, the positions, the prices and
//! the display slots are opaque; the bounds on them belong to the shop.

use crate::cg_inventory::{CgHeader, HEADER_CG_MYSHOP};
use crate::cg_wire::ClientFrame;
use crate::item_pos::ItemPos;

/// One decoded bundle-build request: the sign and the item tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgMyShop {
    /// The raw `char[SHOP_SIGN_MAX_LEN + 1]` sign field.
    pub sign: [u8; CG_MYSHOP_SIGN_FIELD_SIZE],
    /// The tables, in wire order.
    pub tables: Vec<CgMyShopTable>,
}

/// One decoded `TShopItemTable`: the fields the bundle build reads, plus the
/// unread SHOPEX tail as it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgMyShopTable {
    /// `vnum`, read only for NPC shops.
    pub vnum: u32,
    /// `count`, read only for NPC shops.
    pub count: u16,
    /// The held item's window and cell.
    pub pos: ItemPos,
    /// The asking price in yang.
    pub price: u64,
    /// The window slot the client chose.
    pub display_pos: u8,
    /// The unread tail: 6 sockets, 7 attributes, `price_type`, `price_vnum`.
    pub tail: [u8; CG_MYSHOP_TABLE_TAIL_SIZE],
}

/// Width of the sign field: `SHOP_SIGN_MAX_LEN + 1` (`C/length.h:16`).
pub const CG_MYSHOP_SIGN_FIELD_SIZE: usize = 33;

/// Width of one item table on the wire.
pub const CG_MYSHOP_TABLE_WIRE_SIZE: usize = 68;

/// Width of the table head this codec decodes: vnum, count, pos, price,
/// display slot.
pub const CG_MYSHOP_TABLE_HEAD_SIZE: usize = 4 + 2 + 3 + 8 + 1;

/// Width of the unread SHOPEX tail: 6 sockets, 7 attributes, `price_type`,
/// `price_vnum`.
pub const CG_MYSHOP_TABLE_TAIL_SIZE: usize = CG_MYSHOP_TABLE_WIRE_SIZE - CG_MYSHOP_TABLE_HEAD_SIZE;

/// Width of the framed prefix: the sign field and the count.
pub const CG_MYSHOP_PREFIX_SIZE: usize = CG_MYSHOP_SIGN_FIELD_SIZE + 1;

/// What was wrong with a bundle-build record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgMyShopError {
    /// The frame's header byte is not `HEADER_CG_MYSHOP`.
    Header {
        /// The header byte that arrived.
        actual: u8,
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

impl std::fmt::Display for CgMyShopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Header { actual } => {
                write!(f, "not a bundle build: header {actual}")
            }
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "a bundle build needs {needed} bytes, {available} arrived"
                )
            }
        }
    }
}

impl std::error::Error for CgMyShopError {}

impl CgMyShop {
    /// The legacy header byte.
    #[must_use]
    pub const fn header() -> CgHeader {
        HEADER_CG_MYSHOP
    }

    /// Decode a whole frame: the sign, the count, and the count's tables.
    ///
    /// # Errors
    ///
    /// [`CgMyShopError::Header`] on a wrong header byte,
    /// [`CgMyShopError::Truncated`] when the payload is short.
    pub fn decode(frame: &ClientFrame) -> Result<Self, CgMyShopError> {
        if frame.header != Self::header().value() {
            return Err(CgMyShopError::Header {
                actual: frame.header,
            });
        }
        if frame.payload.len() < CG_MYSHOP_PREFIX_SIZE {
            return Err(CgMyShopError::Truncated {
                needed: CG_MYSHOP_PREFIX_SIZE,
                available: frame.payload.len(),
            });
        }
        let mut sign = [0u8; CG_MYSHOP_SIGN_FIELD_SIZE];
        sign.copy_from_slice(&frame.payload[..CG_MYSHOP_SIGN_FIELD_SIZE]);
        let count = usize::from(frame.payload[CG_MYSHOP_SIGN_FIELD_SIZE]);
        let needed = CG_MYSHOP_PREFIX_SIZE + count * CG_MYSHOP_TABLE_WIRE_SIZE;
        if frame.payload.len() < needed {
            return Err(CgMyShopError::Truncated {
                needed,
                available: frame.payload.len(),
            });
        }
        let mut tables = Vec::with_capacity(count);
        for index in 0..count {
            let at = CG_MYSHOP_PREFIX_SIZE + index * CG_MYSHOP_TABLE_WIRE_SIZE;
            tables.push(CgMyShopTable::decode(
                &frame.payload[at..at + CG_MYSHOP_TABLE_WIRE_SIZE],
            ));
        }
        Ok(Self { sign, tables })
    }
}

impl CgMyShopTable {
    /// Decode one 68-byte table. The caller proves the length.
    fn decode(bytes: &[u8]) -> Self {
        let vnum = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let count = u16::from_le_bytes([bytes[4], bytes[5]]);
        let pos = ItemPos::new(bytes[6], u16::from_le_bytes([bytes[7], bytes[8]]));
        let price = u64::from_le_bytes([
            bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], bytes[16],
        ]);
        let mut tail = [0u8; CG_MYSHOP_TABLE_TAIL_SIZE];
        tail.copy_from_slice(&bytes[CG_MYSHOP_TABLE_HEAD_SIZE..]);
        Self {
            vnum,
            count,
            pos,
            price,
            display_pos: bytes[17],
            tail,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The golden frame: sign `Swords`, one table selling the sword at
    /// `(INVENTORY, 0)` for 500 yang in window slot 3, tail filled with
    /// distinct bytes.
    fn golden() -> Vec<u8> {
        let mut frame = vec![HEADER_CG_MYSHOP.value()];
        let mut sign = [0u8; CG_MYSHOP_SIGN_FIELD_SIZE];
        sign[..6].copy_from_slice(b"Swords");
        frame.extend_from_slice(&sign);
        frame.push(1);
        frame.extend_from_slice(&19u32.to_le_bytes());
        frame.extend_from_slice(&1u16.to_le_bytes());
        frame.push(1);
        frame.extend_from_slice(&0u16.to_le_bytes());
        frame.extend_from_slice(&500u64.to_le_bytes());
        frame.push(3);
        for byte in 1..=u8::try_from(CG_MYSHOP_TABLE_TAIL_SIZE).unwrap_or(u8::MAX) {
            frame.push(byte);
        }
        frame
    }

    #[test]
    fn the_bundle_build_decodes_its_sign_and_its_tables() {
        let frame = golden();
        let record = CgMyShop::decode(&ClientFrame {
            header: frame[0],
            payload: frame[1..].to_vec(),
        })
        .expect("the golden frame decodes");
        assert_eq!(&record.sign[..7], b"Swords\x00");
        assert_eq!(record.tables.len(), 1);
        let table = &record.tables[0];
        assert_eq!(
            (table.vnum, table.count, table.price, table.display_pos),
            (19, 1, 500, 3)
        );
        assert_eq!(table.pos, ItemPos::new(1, 0));
        assert_eq!(table.tail[0], 1);
        assert_eq!(table.tail[CG_MYSHOP_TABLE_TAIL_SIZE - 1], 50);
    }

    #[test]
    fn a_short_frame_names_what_it_needed() {
        let frame = golden();
        let short = ClientFrame {
            header: frame[0],
            payload: frame[1..20].to_vec(),
        };
        assert_eq!(
            CgMyShop::decode(&short),
            Err(CgMyShopError::Truncated {
                needed: CG_MYSHOP_PREFIX_SIZE,
                available: 19,
            })
        );
        let short_tables = ClientFrame {
            header: frame[0],
            payload: frame[1..50].to_vec(),
        };
        assert_eq!(
            CgMyShop::decode(&short_tables),
            Err(CgMyShopError::Truncated {
                needed: CG_MYSHOP_PREFIX_SIZE + CG_MYSHOP_TABLE_WIRE_SIZE,
                available: 49,
            })
        );
        let wrong = ClientFrame {
            header: 0x38,
            payload: frame[1..].to_vec(),
        };
        assert_eq!(
            CgMyShop::decode(&wrong),
            Err(CgMyShopError::Header { actual: 0x38 })
        );
    }
}
