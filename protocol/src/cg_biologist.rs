//! Transport-free codec for the legacy `HEADER_CG_BIOLOGIST` (175) record.
//!
//! # Wire layout
//!
//! `server/server/game/packet.h` declares the six bytes every biologist request carries:
//!
//! ```cpp
//! typedef struct SPacketCGBiologist
//! {
//!     BYTE byHeader;
//!     BYTE bySubHeader;
//!     BYTE byChosenAffect;
//!     BYTE byDecreaseTimeIndex;
//!     BYTE byIsElixirUse;
//!     BYTE byIsBookTimeUse;
//! } TPacketCGBiologist;
//! ```
//!
//! ```text
//! [175][sub-header: u8][chosen: u8][flower: u8][elixir: u8][book: u8]
//! ```
//!
//! `CInputMain::RecvBiologistPacket` (`server/server/game/input_main.cpp:5344-5363`)
//! switches on the sub-header and ends silently on any other byte; each arm reads only
//! the byte it names. The framing is fixed-size: the whole record is the frame
//! (`Set(HEADER_CG_BIOLOGIST, sizeof(TPacketCGBiologist))`,
//! `server/server/game/packet_info.cpp:156`).
//!
//! # What this codec deliberately does not do
//!
//! The sub-header and every byte are opaque. Whether the sub-header names an arm, and
//! whether a byte is in range, belongs to the biologist, not to the record.

use crate::cg_inventory::{CgHeader, HEADER_CG_BIOLOGIST};
use crate::cg_wire::ClientFrame;

/// One decoded biologist request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgBiologist {
    /// `bySubHeader`: open, provides, selective or time, or anything else.
    pub sub_header: u8,
    /// `byChosenAffect`: the bonus the selective arm takes.
    pub chosen: u8,
    /// `byDecreaseTimeIndex`: the flower the time arm spends.
    pub flower: u8,
    /// `byIsElixirUse`: whether the provide arm spends the elixir.
    pub elixir: u8,
    /// `byIsBookTimeUse`: whether the provide arm spends the forget book.
    pub book: u8,
}

/// Every way [`CgBiologist::decode_frame`] can refuse a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgBiologistError {
    /// The frame's header is not 175.
    InvalidHeader {
        /// The header byte that was present.
        actual: u8,
    },
    /// The payload is not the five bytes the record carries.
    LengthMismatch {
        /// The payload width that was offered.
        actual: usize,
    },
}

impl std::fmt::Display for CgBiologistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeader { actual } => write!(f, "biologist header {actual} is not 175"),
            Self::LengthMismatch { actual } => {
                write!(f, "biologist needs 5 payload bytes, got {actual}")
            }
        }
    }
}

impl std::error::Error for CgBiologistError {}

impl CgBiologist {
    /// The fixed header byte.
    #[must_use]
    pub const fn header() -> CgHeader {
        HEADER_CG_BIOLOGIST
    }

    /// The payload after the header byte: the sub-header and the four bytes.
    #[must_use]
    pub const fn payload(&self) -> [u8; 5] {
        [
            self.sub_header,
            self.chosen,
            self.flower,
            self.elixir,
            self.book,
        ]
    }

    /// The full record: the header and the payload.
    #[must_use]
    pub fn encode(&self) -> [u8; 6] {
        [
            Self::header().value(),
            self.sub_header,
            self.chosen,
            self.flower,
            self.elixir,
            self.book,
        ]
    }

    /// Decode one framed record.
    ///
    /// # Errors
    ///
    /// [`CgBiologistError::InvalidHeader`] when the frame starts with any other
    /// byte, [`CgBiologistError::LengthMismatch`] when the payload is not the five
    /// bytes the record carries.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgBiologistError> {
        if frame.header != HEADER_CG_BIOLOGIST.value() {
            return Err(CgBiologistError::InvalidHeader {
                actual: frame.header,
            });
        }
        let payload = frame.payload.as_slice();
        if payload.len() != 5 {
            return Err(CgBiologistError::LengthMismatch {
                actual: payload.len(),
            });
        }
        Ok(Self {
            sub_header: payload[0],
            chosen: payload[1],
            flower: payload[2],
            elixir: payload[3],
            book: payload[4],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CgBiologist, CgBiologistError};
    use crate::cg_wire::ClientFrame;

    #[test]
    fn the_open_encodes_to_its_six_bytes() {
        let open = CgBiologist {
            sub_header: 0,
            chosen: 0,
            flower: 0,
            elixir: 0,
            book: 0,
        };
        assert_eq!(open.encode(), [175, 0, 0, 0, 0, 0]);
        assert_eq!(open.payload(), [0, 0, 0, 0, 0]);
    }

    #[test]
    fn the_provide_round_trips_distinct_halves() {
        // No two bytes agree in either nibble, so a swap shows.
        let frame = ClientFrame::new(175, [0x02, 0x1a, 0x3c, 0x5e, 0x70]);
        let decoded = CgBiologist::decode_frame(&frame).expect("five bytes decode");
        assert_eq!(
            decoded,
            CgBiologist {
                sub_header: 0x02,
                chosen: 0x1a,
                flower: 0x3c,
                elixir: 0x5e,
                book: 0x70,
            }
        );
        assert_eq!(decoded.encode(), [175, 0x02, 0x1a, 0x3c, 0x5e, 0x70]);
    }

    #[test]
    fn a_foreign_header_is_refused_before_the_payload() {
        let frame = ClientFrame::new(174, [0, 0, 0, 0, 0]);
        assert_eq!(
            CgBiologist::decode_frame(&frame),
            Err(CgBiologistError::InvalidHeader { actual: 174 })
        );
    }

    #[test]
    fn a_short_or_long_payload_is_refused() {
        for payload in [&[0u8][..], &[0u8, 0, 0, 0, 0, 0][..]] {
            let frame = ClientFrame::new(175, payload);
            assert_eq!(
                CgBiologist::decode_frame(&frame),
                Err(CgBiologistError::LengthMismatch {
                    actual: payload.len()
                })
            );
        }
    }
}
