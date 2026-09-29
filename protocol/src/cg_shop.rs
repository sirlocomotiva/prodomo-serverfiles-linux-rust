//! Transport-free codec for the legacy `HEADER_CG_SHOP` (50) record.
//!
//! # Wire layout
//!
//! `server/server/game/packet.h:702-706` declares the two bytes every shop
//! request starts with, and `packet.h:683-689` numbers the subheaders:
//!
//! ```cpp
//! typedef struct command_shop
//! {
//!     BYTE    header;
//!     BYTE    subheader;
//! } TPacketCGShop;
//! ```
//!
//! `CInputMain::Shop` (`server/server/game/input_main.cpp:1241-1303`) reads the
//! rest by subheader, and the framing in [`crate::cg_variable`] already sizes
//! each frame the same way:
//!
//! ```text
//! [50][0]                                  END
//! [50][1][count: u8][pos: u8]              BUY: `bPos = *(c_pData + 1)`
//! [50][2][cell: u8]                        SELL
//! [50][3][slot: u8][pad: u8][count: u16]   SELL2: `TfckOFF`
//! [50][sub]                                any other subheader
//! ```
//!
//! BUY's first byte is `TPacketCGShopBuy::count`, which the server reads past
//! and never uses. `TfckOFF` (`input_main.cpp:1235-1239`) is declared outside
//! `packet.h`'s `#pragma pack(1)`, so its `WORD` is aligned and a padding byte
//! the client never initializes sits between the slot and the count. The codec
//! keeps both unused bytes as they arrived.
//!
//! An unknown subheader is logged by the server and consumes nothing past the
//! prefix; the connection stays open.
//!
//! # What this codec deliberately does not do
//!
//! The position, cell, slot and count are opaque. The bounds on them belong to
//! the shop, not to the record.

use crate::cg_inventory::{CgHeader, HEADER_CG_SHOP};
use crate::cg_variable::{
    SHOP_SUBHEADER_BUY, SHOP_SUBHEADER_END, SHOP_SUBHEADER_SELL, SHOP_SUBHEADER_SELL2,
};
use crate::cg_wire::ClientFrame;

/// One decoded shop request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgShop {
    /// `SHOP_SUBHEADER_CG_END`: close the window.
    End,
    /// `SHOP_SUBHEADER_CG_BUY`: buy the slot at `pos`.
    Buy {
        /// `TPacketCGShopBuy::count`, which the server never reads.
        count: u8,
        /// The window slot.
        pos: u8,
    },
    /// `SHOP_SUBHEADER_CG_SELL`: sell a whole inventory stack.
    Sell {
        /// The inventory cell.
        cell: u8,
    },
    /// `SHOP_SUBHEADER_CG_SELL2`: sell `count` of an inventory stack.
    Sell2 {
        /// `bySlot`: the inventory cell.
        slot: u8,
        /// The alignment byte before the `WORD`, as it arrived.
        padding: u8,
        /// `byCount`: how many to sell.
        count: u16,
    },
    /// Any other subheader, which the server logs and ignores.
    Unknown {
        /// The subheader that arrived.
        subheader: u8,
    },
}

/// Every way [`CgShop::decode_frame`] can refuse a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgShopError {
    /// The frame's header is not 50.
    InvalidHeader {
        /// The header byte that was present.
        actual: u8,
    },
    /// The payload is not the width its subheader needs.
    LengthMismatch {
        /// The subheader, or `None` for an empty payload.
        subheader: Option<u8>,
        /// The payload width the subheader needs.
        expected: usize,
        /// The payload width that was offered.
        actual: usize,
    },
}

impl std::fmt::Display for CgShopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeader { actual } => write!(f, "shop header {actual} is not 50"),
            Self::LengthMismatch {
                subheader: Some(sub),
                expected,
                actual,
            } => write!(
                f,
                "shop subheader {sub} needs {expected} payload bytes, got {actual}"
            ),
            Self::LengthMismatch {
                subheader: None,
                actual,
                ..
            } => write!(f, "shop payload has no subheader, got {actual} bytes"),
        }
    }
}

impl std::error::Error for CgShopError {}

impl CgShop {
    /// The fixed header byte.
    #[must_use]
    pub const fn header() -> CgHeader {
        HEADER_CG_SHOP
    }

    /// The payload after the header byte: the subheader and its body.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match *self {
            Self::End => vec![SHOP_SUBHEADER_END],
            Self::Buy { count, pos } => vec![SHOP_SUBHEADER_BUY, count, pos],
            Self::Sell { cell } => vec![SHOP_SUBHEADER_SELL, cell],
            Self::Sell2 {
                slot,
                padding,
                count,
            } => {
                let [low, high] = count.to_le_bytes();
                vec![SHOP_SUBHEADER_SELL2, slot, padding, low, high]
            }
            Self::Unknown { subheader } => vec![subheader],
        }
    }

    /// Encode the whole record, header byte included.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![Self::header().value()];
        out.extend(self.payload());
        out
    }

    /// Project to the client frame, header byte and payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), self.payload())
    }

    /// # Errors
    ///
    /// Returns [`CgShopError::InvalidHeader`] when the header is not 50, and
    /// [`CgShopError::LengthMismatch`] when the payload is empty or is not the
    /// width its subheader needs. The header is checked first.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgShopError> {
        if frame.header != HEADER_CG_SHOP.value() {
            return Err(CgShopError::InvalidHeader {
                actual: frame.header,
            });
        }
        let payload = frame.payload.as_slice();
        let Some(&subheader) = payload.first() else {
            return Err(CgShopError::LengthMismatch {
                subheader: None,
                expected: 1,
                actual: 0,
            });
        };
        let expected = match subheader {
            SHOP_SUBHEADER_BUY => 3,
            SHOP_SUBHEADER_SELL => 2,
            SHOP_SUBHEADER_SELL2 => 5,
            _ => 1,
        };
        if payload.len() != expected {
            return Err(CgShopError::LengthMismatch {
                subheader: Some(subheader),
                expected,
                actual: payload.len(),
            });
        }
        Ok(match subheader {
            SHOP_SUBHEADER_END => Self::End,
            SHOP_SUBHEADER_BUY => Self::Buy {
                count: payload[1],
                pos: payload[2],
            },
            SHOP_SUBHEADER_SELL => Self::Sell { cell: payload[1] },
            SHOP_SUBHEADER_SELL2 => Self::Sell2 {
                slot: payload[1],
                padding: payload[2],
                count: u16::from_le_bytes([payload[3], payload[4]]),
            },
            _ => Self::Unknown { subheader },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CgShop, CgShopError};
    use crate::cg_variable::{
        resolve_variable_client_frame_size, SHOP_SUBHEADER_BUY, SHOP_SUBHEADER_END,
        SHOP_SUBHEADER_SELL, SHOP_SUBHEADER_SELL2,
    };
    use crate::cg_wire::ClientFrame;

    #[test]
    fn the_header_and_subheaders_match_the_source() {
        assert_eq!(CgShop::header().value(), 50);
        assert_eq!(
            [
                SHOP_SUBHEADER_END,
                SHOP_SUBHEADER_BUY,
                SHOP_SUBHEADER_SELL,
                SHOP_SUBHEADER_SELL2
            ],
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn each_request_is_written_as_the_server_reads_it() {
        assert_eq!(CgShop::End.encode(), vec![50, 0]);
        assert_eq!(
            CgShop::Buy { count: 7, pos: 39 }.encode(),
            vec![50, 1, 7, 39]
        );
        assert_eq!(CgShop::Sell { cell: 44 }.encode(), vec![50, 2, 44]);
        let sell2 = CgShop::Sell2 {
            slot: 5,
            padding: 0xcc,
            count: 0x0102,
        };
        assert_eq!(sell2.encode(), vec![50, 3, 5, 0xcc, 2, 1]);
        assert_eq!(CgShop::Unknown { subheader: 9 }.encode(), vec![50, 9]);
    }

    #[test]
    fn each_field_is_read_from_its_own_byte() {
        // Every field differs from the others, so a field read from the wrong byte is caught.
        let frame = |payload: &[u8]| ClientFrame::new(50, payload);
        assert_eq!(
            CgShop::decode_frame(&frame(&[1, 7, 39])),
            Ok(CgShop::Buy { count: 7, pos: 39 })
        );
        assert_eq!(
            CgShop::decode_frame(&frame(&[2, 44])),
            Ok(CgShop::Sell { cell: 44 })
        );
        assert_eq!(
            CgShop::decode_frame(&frame(&[3, 5, 0xcc, 2, 1])),
            Ok(CgShop::Sell2 {
                slot: 5,
                padding: 0xcc,
                count: 0x0102
            })
        );
    }

    #[test]
    fn each_request_round_trips_through_its_frame() {
        for request in [
            CgShop::End,
            CgShop::Buy { count: 0, pos: 0 },
            CgShop::Buy {
                count: 255,
                pos: 255,
            },
            CgShop::Sell { cell: 0 },
            CgShop::Sell { cell: 255 },
            CgShop::Sell2 {
                slot: 180,
                padding: 0,
                count: u16::MAX,
            },
            CgShop::Unknown { subheader: 4 },
            CgShop::Unknown { subheader: 255 },
        ] {
            let frame = request.to_frame();
            assert_eq!(frame.header, 50);
            assert_eq!(CgShop::decode_frame(&frame), Ok(request));
        }
    }

    #[test]
    fn the_framing_and_the_decoder_agree_on_every_width() {
        for request in [
            CgShop::End,
            CgShop::Buy { count: 1, pos: 2 },
            CgShop::Sell { cell: 3 },
            CgShop::Sell2 {
                slot: 4,
                padding: 5,
                count: 6,
            },
            CgShop::Unknown { subheader: 200 },
        ] {
            let bytes = request.encode();
            let framed = resolve_variable_client_frame_size(50, &bytes)
                .expect("a shop frame")
                .expect("complete");
            assert_eq!(framed, bytes.len(), "{request:?}");
        }
    }

    #[test]
    fn a_frame_of_the_wrong_width_or_header_is_refused() {
        let frame = |header, payload: &[u8]| ClientFrame::new(header, payload);
        assert_eq!(
            CgShop::decode_frame(&frame(51, &[0])),
            Err(CgShopError::InvalidHeader { actual: 51 })
        );
        assert_eq!(
            CgShop::decode_frame(&frame(50, &[])),
            Err(CgShopError::LengthMismatch {
                subheader: None,
                expected: 1,
                actual: 0
            })
        );
        for (payload, expected) in [
            (&[0u8, 0][..], 1),
            (&[1, 0][..], 3),
            (&[1, 0, 0, 0][..], 3),
            (&[2][..], 2),
            (&[2, 0, 0][..], 2),
            (&[3, 0, 0, 0][..], 5),
            (&[3, 0, 0, 0, 0, 0][..], 5),
            (&[7, 0][..], 1),
        ] {
            assert_eq!(
                CgShop::decode_frame(&frame(50, payload)),
                Err(CgShopError::LengthMismatch {
                    subheader: Some(payload[0]),
                    expected,
                    actual: payload.len()
                }),
                "{payload:?}"
            );
        }
    }

    #[test]
    fn the_errors_say_what_was_wrong() {
        assert_eq!(
            CgShopError::InvalidHeader { actual: 3 }.to_string(),
            "shop header 3 is not 50"
        );
        assert_eq!(
            CgShopError::LengthMismatch {
                subheader: Some(1),
                expected: 3,
                actual: 2
            }
            .to_string(),
            "shop subheader 1 needs 3 payload bytes, got 2"
        );
        assert_eq!(
            CgShopError::LengthMismatch {
                subheader: None,
                expected: 1,
                actual: 0
            }
            .to_string(),
            "shop payload has no subheader, got 0 bytes"
        );
    }
}
