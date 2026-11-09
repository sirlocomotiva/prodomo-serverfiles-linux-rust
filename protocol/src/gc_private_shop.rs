//! `HEADER_GC_PRIVATE_SHOP` (175): a personal shop's stand, its panel, and its panel's answers.
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
//! Only the build slice's three subheaders are encoded here: `LOAD` (3), which
//! `CHARACTER::SetPrivateShopTable` (`server/server/game/char.cpp:12160-12178`) sends to the
//! owner after the store accepts the build; `ADD_ENTITY` (0), which
//! `CPrivateShop::EncodeInsertPacket` (`server/server/game/private_shop.cpp:853-880`) sends to
//! each viewer around the stand; and the bare-header `OPEN_PANEL` (7) and `CLOSE` (9), which
//! `OpenPrivateShopPanel`, `ClosePrivateShopPanel` and `ClosePrivateShop`
//! (`char.cpp:12182-12219`) send. The rest are later slices.
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

use crate::gc_inventory::HEADER_GC_PRIVATE_SHOP;

/// `sizeof(TPacketGCPrivateShop)`: header, size and subheader.
pub const GC_PRIVATE_SHOP_WIRE_SIZE: usize = 4;

/// `sizeof(TPacketGCPrivateShopLoad)`.
pub const GC_PRIVATE_SHOP_LOAD_WIRE_SIZE: usize = 56;

/// `sizeof(TPacketGCPrivateShopAddEntity)`.
pub const GC_PRIVATE_SHOP_ADD_ENTITY_WIRE_SIZE: usize = 79;

/// The whole `LOAD` record: the header and the load body.
pub const GC_PRIVATE_SHOP_LOAD_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_LOAD_WIRE_SIZE;

/// The whole `ADD_ENTITY` record: the header and the entity body.
pub const GC_PRIVATE_SHOP_ADD_ENTITY_RECORD_SIZE: usize =
    GC_PRIVATE_SHOP_WIRE_SIZE + GC_PRIVATE_SHOP_ADD_ENTITY_WIRE_SIZE;

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
