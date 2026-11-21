//! `HEADER_GC_EXCHANGE` (42): the trade window between two players.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:1627-1651`:
//!
//! ```ignore
//! struct packet_exchange
//! {
//!     BYTE    header;
//!     BYTE    sub_header;
//!     BYTE    is_me;
//! #ifdef ENABLE_REMOVE_LIMIT_GOLD
//!     unsigned long long    arg1;    // vnum
//! #else
//!     DWORD    arg1;    // vnum
//! #endif
//!     TItemPos    arg2;    // cell
//!     DWORD    arg3;    // count
//! #ifdef WJ_ENABLE_TRADABLE_ICON
//!     TItemPos    arg4;    // srccell
//! #endif
//!     long    alSockets[ITEM_SOCKET_MAX_NUM];
//!     TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
//! #ifdef ENABLE_REFINE_ELEMENT
//!     DWORD    dwRefineElement;
//! #endif
//! #ifdef __CHANGELOOK_SYSTEM__
//!     DWORD    dwTransmutation;
//! #endif
//! };
//! ```
//!
//! The subheaders are `EPacketTradeSubHeaders` at `packet.h:1653-1663`, numbered from 0.
//!
//! `exchange_packet` (`server/server/game/exchange.cpp:25-74`) is the only writer. It sets the
//! header, the subheader, `is_me`, `arg1`, `arg2` and `arg3` from its arguments. For
//! `EXCHANGE_SUBHEADER_GC_ITEM_ADD` with an item it copies the item's window and cell into
//! `arg4` and its sockets, attributes, refine element and transmutation into the rest; for
//! every other send it writes `arg4 = TItemPos(RESERVED_WINDOW, 0)` and zeroes the rest, which
//! is [`GcExchange::new`].
//!
//! # Width
//!
//! With `ENABLE_REMOVE_LIMIT_GOLD`, `WJ_ENABLE_TRADABLE_ICON`, `ENABLE_REFINE_ELEMENT` and
//! `__CHANGELOOK_SYSTEM__` on (`server/server/common/prodomodefines.h`), and `packet.h` inside
//! `#pragma pack(1)`, the record is 1 + 1 + 1 + 8 + 3 + 4 + 3 + 6 × 4 + 7 × 3 + 4 + 4 = 74
//! bytes. `long` is four bytes on the 32-bit target the game builds for.

use std::fmt;

use crate::gc_inventory::HEADER_GC_EXCHANGE;
use crate::gc_item_window::{
    ItemAttribute, ItemAttributes, ItemSockets, ITEM_ATTRIBUTE_MAX_NUM, ITEM_SOCKET_MAX_NUM,
};
use crate::item_pos::ItemPos;

/// `sizeof(packet_exchange)`.
pub const GC_EXCHANGE_WIRE_SIZE: usize = 74;

/// `EXCHANGE_SUBHEADER_GC_START` (0): the window opens; `arg1` is the other player's VID.
pub const EXCHANGE_SUBHEADER_GC_START: u8 = 0;
/// `EXCHANGE_SUBHEADER_GC_ITEM_ADD` (1): an item is offered.
pub const EXCHANGE_SUBHEADER_GC_ITEM_ADD: u8 = 1;
/// `EXCHANGE_SUBHEADER_GC_ITEM_DEL` (2): an offered item is taken back.
pub const EXCHANGE_SUBHEADER_GC_ITEM_DEL: u8 = 2;
/// `EXCHANGE_SUBHEADER_GC_GOLD_ADD` (3): gold is offered; `arg1` is the amount.
pub const EXCHANGE_SUBHEADER_GC_GOLD_ADD: u8 = 3;
/// `EXCHANGE_SUBHEADER_GC_ACCEPT` (4): `arg1` is whether the side accepts.
pub const EXCHANGE_SUBHEADER_GC_ACCEPT: u8 = 4;
/// `EXCHANGE_SUBHEADER_GC_END` (5): the window closes.
pub const EXCHANGE_SUBHEADER_GC_END: u8 = 5;
/// `EXCHANGE_SUBHEADER_GC_ALREADY` (6): the other player is already trading.
pub const EXCHANGE_SUBHEADER_GC_ALREADY: u8 = 6;
/// `EXCHANGE_SUBHEADER_GC_LESS_GOLD` (7): the gold offered is more than the player holds.
pub const EXCHANGE_SUBHEADER_GC_LESS_GOLD: u8 = 7;

/// `TItemPos(RESERVED_WINDOW, 0)`: the `arg4` of every record but an offered item.
pub const NO_SOURCE_CELL: ItemPos = ItemPos::new(0, 0);

/// Every way an exchange record can be refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcExchangeError {
    /// Fewer than 74 bytes.
    Truncated {
        /// The bytes offered.
        actual: usize,
    },
    /// The leading byte is not 42.
    Header {
        /// The byte that was present.
        actual: u8,
    },
}

impl fmt::Display for GcExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { actual } => {
                write!(
                    f,
                    "GcExchange needs {GC_EXCHANGE_WIRE_SIZE} bytes, got {actual}"
                )
            }
            Self::Header { actual } => write!(f, "exchange header {actual} is not 42"),
        }
    }
}

impl std::error::Error for GcExchangeError {}

/// `packet_exchange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcExchange {
    /// One of the `EXCHANGE_SUBHEADER_GC_*` values, opaque to the codec.
    pub sub_header: u8,
    /// `is_me`: 1 when the record is about the receiver's own side.
    pub is_me: u8,
    /// `arg1`: a VID, a vnum, a slot, an amount or a flag, as the subheader says.
    pub arg1: u64,
    /// `arg2`: the display cell of an offered item, or a position the subheader chooses.
    pub arg2: ItemPos,
    /// `arg3`: the count of an offered item.
    pub arg3: u32,
    /// `arg4`: where an offered item sits in its owner's inventory.
    pub arg4: ItemPos,
    /// `alSockets`.
    pub sockets: ItemSockets,
    /// `aAttr`.
    pub attrs: ItemAttributes,
    /// `dwRefineElement`.
    pub refine_element: u32,
    /// `dwTransmutation`.
    pub transmutation: u32,
}

impl GcExchange {
    /// Number of bytes the record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_EXCHANGE_WIRE_SIZE;

    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_EXCHANGE.value()
    }

    /// A record as `exchange_packet` writes it without an item: `arg4` is
    /// [`NO_SOURCE_CELL`] and everything after it is zero.
    #[must_use]
    pub fn new(sub_header: u8, is_me: bool, arg1: u64, arg2: ItemPos, arg3: u32) -> Self {
        Self {
            sub_header,
            is_me: u8::from(is_me),
            arg1,
            arg2,
            arg3,
            arg4: NO_SOURCE_CELL,
            sockets: [0; ITEM_SOCKET_MAX_NUM],
            attrs: [ItemAttribute::new(0, 0); ITEM_ATTRIBUTE_MAX_NUM],
            refine_element: 0,
            transmutation: 0,
        }
    }

    /// Appends the 74 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_EXCHANGE.value());
        out.push(self.sub_header);
        out.push(self.is_me);
        out.extend_from_slice(&self.arg1.to_le_bytes());
        self.arg2.encode_into(out);
        out.extend_from_slice(&self.arg3.to_le_bytes());
        self.arg4.encode_into(out);
        for socket in self.sockets {
            out.extend_from_slice(&socket.to_le_bytes());
        }
        for attr in self.attrs {
            attr.encode_into(out);
        }
        out.extend_from_slice(&self.refine_element.to_le_bytes());
        out.extend_from_slice(&self.transmutation.to_le_bytes());
    }

    /// Encode to a fresh buffer.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(GC_EXCHANGE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Reads the 74 packed bytes from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcExchangeError::Truncated`] for fewer than 74 bytes and
    /// [`GcExchangeError::Header`] when the leading byte is not 42.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcExchangeError> {
        if bytes.len() < GC_EXCHANGE_WIRE_SIZE {
            return Err(GcExchangeError::Truncated {
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_EXCHANGE.value() {
            return Err(GcExchangeError::Header { actual: bytes[0] });
        }
        let word = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let mut arg1 = [0; 8];
        arg1.copy_from_slice(&bytes[3..11]);
        let mut sockets = [0; ITEM_SOCKET_MAX_NUM];
        for (index, socket) in sockets.iter_mut().enumerate() {
            *socket = i32::from_le_bytes(word(21 + index * 4).to_le_bytes());
        }
        let mut attrs = [ItemAttribute::default(); ITEM_ATTRIBUTE_MAX_NUM];
        for (index, attr) in attrs.iter_mut().enumerate() {
            let at = 45 + index * 3;
            *attr = ItemAttribute::new(
                bytes[at],
                i16::from_le_bytes([bytes[at + 1], bytes[at + 2]]),
            );
        }
        Ok(Self {
            sub_header: bytes[1],
            is_me: bytes[2],
            arg1: u64::from_le_bytes(arg1),
            arg2: ItemPos::decode_at(bytes, 11),
            arg3: word(14),
            arg4: ItemPos::decode_at(bytes, 18),
            sockets,
            attrs,
            refine_element: word(66),
            transmutation: word(70),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GcExchange, GcExchangeError, EXCHANGE_SUBHEADER_GC_ACCEPT, EXCHANGE_SUBHEADER_GC_ALREADY,
        EXCHANGE_SUBHEADER_GC_END, EXCHANGE_SUBHEADER_GC_GOLD_ADD, EXCHANGE_SUBHEADER_GC_ITEM_ADD,
        EXCHANGE_SUBHEADER_GC_ITEM_DEL, EXCHANGE_SUBHEADER_GC_LESS_GOLD,
        EXCHANGE_SUBHEADER_GC_START, GC_EXCHANGE_WIRE_SIZE, NO_SOURCE_CELL,
    };
    use crate::gc_inventory::HEADER_GC_EXCHANGE;
    use crate::gc_item_window::ItemAttribute;
    use crate::item_pos::ItemPos;

    /// A record whose every field holds a distinct value with distinct byte halves, so a
    /// swapped or misplaced field shows in the bytes.
    fn marked() -> GcExchange {
        let mut record = GcExchange::new(
            0x12,
            true,
            0x2122_2324_2526_2728,
            ItemPos::new(0x31, 0x3233),
            0x4142_4344,
        );
        record.is_me = 0x13;
        record.arg4 = ItemPos::new(0x51, 0x5253);
        record.sockets = [-2, 0x0102_0304, 3, 4, 5, i32::MIN];
        record.attrs = [
            ItemAttribute::new(1, -3),
            ItemAttribute::new(3, 0x0405),
            ItemAttribute::new(5, 6),
            ItemAttribute::new(7, 8),
            ItemAttribute::new(9, 10),
            ItemAttribute::new(11, 12),
            ItemAttribute::new(13, i16::MIN),
        ];
        record.refine_element = 0x6162_6364;
        record.transmutation = 0x7172_7374;
        record
    }

    /// The bytes of [`marked`], written from the source's field order.
    fn marked_bytes() -> Vec<u8> {
        let mut out = vec![42, 0x12, 0x13];
        out.extend([0x28, 0x27, 0x26, 0x25, 0x24, 0x23, 0x22, 0x21]);
        out.extend([0x31, 0x33, 0x32, 0x44, 0x43, 0x42, 0x41, 0x51, 0x53, 0x52]);
        out.extend([0xfe, 0xff, 0xff, 0xff, 4, 3, 2, 1, 3, 0, 0, 0]);
        out.extend([4, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0x80]);
        out.extend([1, 0xfd, 0xff, 3, 5, 4, 5, 6, 0, 7, 8, 0, 9, 10, 0]);
        out.extend([11, 12, 0, 13, 0, 0x80]);
        out.extend([0x64, 0x63, 0x62, 0x61, 0x74, 0x73, 0x72, 0x71]);
        out
    }

    #[test]
    fn the_width_and_subheaders_match_the_source() {
        assert_eq!(GcExchange::header(), 42);
        assert_eq!(HEADER_GC_EXCHANGE.value(), 0x2a);
        assert_eq!(
            GC_EXCHANGE_WIRE_SIZE,
            1 + 1 + 1 + 8 + 3 + 4 + 3 + 24 + 21 + 4 + 4
        );
        assert_eq!(GcExchange::WIRE_SIZE, 74);
        let subheaders = [
            EXCHANGE_SUBHEADER_GC_START,
            EXCHANGE_SUBHEADER_GC_ITEM_ADD,
            EXCHANGE_SUBHEADER_GC_ITEM_DEL,
            EXCHANGE_SUBHEADER_GC_GOLD_ADD,
            EXCHANGE_SUBHEADER_GC_ACCEPT,
            EXCHANGE_SUBHEADER_GC_END,
            EXCHANGE_SUBHEADER_GC_ALREADY,
            EXCHANGE_SUBHEADER_GC_LESS_GOLD,
        ];
        assert!(subheaders.iter().copied().eq(0..8));
        assert_eq!(NO_SOURCE_CELL, ItemPos::new(0, 0));
    }

    #[test]
    fn a_record_is_written_in_the_sources_field_order_and_round_trips() {
        let bytes = marked().encode();
        assert_eq!(bytes, marked_bytes());
        assert_eq!(bytes.len(), 74);
        assert_eq!(GcExchange::decode(&bytes), Ok(marked()));
    }

    #[test]
    fn a_record_without_an_item_has_the_reserved_source_and_zeros() {
        let bytes = GcExchange::new(
            EXCHANGE_SUBHEADER_GC_START,
            false,
            0x0102,
            ItemPos::new(0, 0xffff),
            0,
        )
        .encode();
        let mut expected = vec![42, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff];
        expected.resize(74, 0);
        assert_eq!(bytes, expected);
        assert_eq!(GcExchange::new(0, true, 0, NO_SOURCE_CELL, 0).is_me, 1);
    }

    #[test]
    fn a_record_is_refused_when_short_or_foreign() {
        let bytes = marked_bytes();
        assert_eq!(
            GcExchange::decode(&bytes[..73]),
            Err(GcExchangeError::Truncated { actual: 73 })
        );
        let mut foreign = bytes;
        foreign[0] = 41;
        assert_eq!(
            GcExchange::decode(&foreign),
            Err(GcExchangeError::Header { actual: 41 })
        );
        assert_eq!(
            GcExchangeError::Truncated { actual: 1 }.to_string(),
            "GcExchange needs 74 bytes, got 1"
        );
        assert_eq!(
            GcExchangeError::Header { actual: 9 }.to_string(),
            "exchange header 9 is not 42"
        );
    }
}
