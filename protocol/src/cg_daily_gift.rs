//! Transport-free codec for the legacy `HEADER_CG_DAILY_GIFT` (180) record.
//!
//! # Wire layout
//!
//! `server/server/game/packet.h` declares the three bytes every daily-gift
//! request carries:
//!
//! ```cpp
//! typedef struct SPacketCGDailyGift
//! {
//!     BYTE bHeader;
//!     BYTE bAction;
//!     BYTE bSlotIndex;
//! } TPacketCGDailyGift;
//! ```
//!
//! ```text
//! [180][action: u8][slot: u8]
//! ```
//!
//! `CInputMain::DailyGift` (`server/server/game/input_main.cpp:3512-3545`) switches
//! on the action and ends silently on any other byte; the slot is the 0-based day
//! the collect arms add one to. The framing is fixed-size: the whole record is the
//! frame.
//!
//! # What this codec deliberately does not do
//!
//! The action and slot are opaque. Whether the action names an arm, and whether
//! the slot names a day, belongs to the gift week, not to the record.

use crate::cg_inventory::{CgHeader, HEADER_CG_DAILY_GIFT};
use crate::cg_wire::ClientFrame;

/// One decoded daily-gift request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgDailyGift {
    /// `bAction`: close, open, collect or collect-with-ticket, or anything else.
    pub action: u8,
    /// `bSlotIndex`: the 0-based day the collect arms play.
    pub slot: u8,
}

/// Every way [`CgDailyGift::decode_frame`] can refuse a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgDailyGiftError {
    /// The frame's header is not 180.
    InvalidHeader {
        /// The header byte that was present.
        actual: u8,
    },
    /// The payload is not the two bytes the record carries.
    LengthMismatch {
        /// The payload width that was offered.
        actual: usize,
    },
}

impl std::fmt::Display for CgDailyGiftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeader { actual } => write!(f, "daily-gift header {actual} is not 180"),
            Self::LengthMismatch { actual } => {
                write!(f, "daily-gift needs 2 payload bytes, got {actual}")
            }
        }
    }
}

impl std::error::Error for CgDailyGiftError {}

impl CgDailyGift {
    /// The fixed header byte.
    #[must_use]
    pub const fn header() -> CgHeader {
        HEADER_CG_DAILY_GIFT
    }

    /// The payload after the header byte: the action and the slot.
    #[must_use]
    pub const fn payload(&self) -> [u8; 2] {
        [self.action, self.slot]
    }

    /// The full record: the header and the payload.
    #[must_use]
    pub fn encode(&self) -> [u8; 3] {
        [Self::header().value(), self.action, self.slot]
    }

    /// Decode one framed record.
    ///
    /// # Errors
    ///
    /// [`CgDailyGiftError::InvalidHeader`] when the frame starts with any other
    /// byte, [`CgDailyGiftError::LengthMismatch`] when the payload is not the two
    /// bytes the record carries.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgDailyGiftError> {
        if frame.header != HEADER_CG_DAILY_GIFT.value() {
            return Err(CgDailyGiftError::InvalidHeader {
                actual: frame.header,
            });
        }
        let payload = frame.payload.as_slice();
        if payload.len() != 2 {
            return Err(CgDailyGiftError::LengthMismatch {
                actual: payload.len(),
            });
        }
        Ok(Self {
            action: payload[0],
            slot: payload[1],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CgDailyGift, CgDailyGiftError};
    use crate::cg_wire::ClientFrame;

    #[test]
    fn the_open_encodes_to_its_three_bytes() {
        let open = CgDailyGift { action: 1, slot: 0 };
        assert_eq!(open.encode(), [180, 1, 0]);
        assert_eq!(open.payload(), [1, 0]);
    }

    #[test]
    fn the_collect_round_trips_distinct_halves() {
        // `0xA5` and `0x3C` differ in both nibbles, so a swap or an
        // endianness slip shows.
        let frame = ClientFrame::new(180, [0xa5, 0x3c]);
        let decoded = CgDailyGift::decode_frame(&frame).expect("two bytes decode");
        assert_eq!(
            decoded,
            CgDailyGift {
                action: 0xa5,
                slot: 0x3c
            }
        );
        assert_eq!(decoded.encode(), [180, 0xa5, 0x3c]);
    }

    #[test]
    fn a_foreign_header_is_refused_before_the_payload() {
        let frame = ClientFrame::new(181, [1, 0]);
        assert_eq!(
            CgDailyGift::decode_frame(&frame),
            Err(CgDailyGiftError::InvalidHeader { actual: 181 })
        );
    }

    #[test]
    fn a_short_or_long_payload_is_refused() {
        for payload in [&[1u8][..], &[1u8, 0, 0][..]] {
            let frame = ClientFrame::new(180, payload);
            assert_eq!(
                CgDailyGift::decode_frame(&frame),
                Err(CgDailyGiftError::LengthMismatch {
                    actual: payload.len()
                })
            );
        }
    }
}
