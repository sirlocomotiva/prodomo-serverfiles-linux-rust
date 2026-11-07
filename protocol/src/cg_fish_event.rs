//! Transport-free codec for the legacy `HEADER_CG_FISH_EVENT_SEND` (219) record.
//!
//! # Wire layout
//!
//! `server/server/game/packet.h:2871-2881` declares the two bytes every fish-event
//! request starts with, and `packet.h:2862-2869` numbers the subheaders:
//!
//! ```cpp
//! typedef struct SPacketGCFishEvent {
//!     BYTE    bHeader;
//!     BYTE    bSubheader;
//! } TPacketCGFishEvent;
//! ```
//!
//! `CInputMain::FishEvent` (`server/server/game/input_main.cpp:1319-1366`) reads the
//! rest by subheader, and the framing in [`crate::cg_variable`] already sizes each
//! frame the same way:
//!
//! ```text
//! [219][0][window: u8][cell: u16]   BOX_USE: `TItemPos(bWindow, wCell)`
//! [219][1][pos: u8]                 SHAPE_ADD
//! [219][sub]                        any other subheader
//! ```
//!
//! An unknown subheader is logged by the server (`sys_err`, `:1359`) and consumes
//! nothing past the prefix; the connection stays open.
//!
//! # What this codec deliberately does not do
//!
//! The window, cell and position are opaque. The bounds on them belong to the fish
//! board, not to the record.

use crate::cg_inventory::{CgHeader, HEADER_CG_FISH_EVENT_SEND};
use crate::cg_variable::{FISH_EVENT_SUBHEADER_BOX_USE, FISH_EVENT_SUBHEADER_SHAPE_ADD};
use crate::cg_wire::ClientFrame;

/// One decoded fish-event request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgFishEvent {
    /// `FISH_EVENT_SUBHEADER_BOX_USE`: use the box at the position.
    BoxUse {
        /// `bWindow`, the window byte of the `TItemPos` the server builds.
        window: u8,
        /// `wCell`, the cell of the `TItemPos` the server builds.
        cell: u16,
    },
    /// `FISH_EVENT_SUBHEADER_SHAPE_ADD`: add the attached shape at the cell.
    ShapeAdd {
        /// `shapePos`, the board cell.
        pos: u8,
    },
    /// Any other subheader, which the server logs and ignores.
    Unknown {
        /// The subheader that arrived.
        subheader: u8,
    },
}

/// Every way [`CgFishEvent::decode_frame`] can refuse a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgFishEventError {
    /// The frame's header is not 219.
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

impl std::fmt::Display for CgFishEventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeader { actual } => write!(f, "fish-event header {actual} is not 219"),
            Self::LengthMismatch {
                subheader: Some(sub),
                expected,
                actual,
            } => write!(
                f,
                "fish-event subheader {sub} needs {expected} payload bytes, got {actual}"
            ),
            Self::LengthMismatch {
                subheader: None,
                actual,
                ..
            } => write!(f, "fish-event payload has no subheader, got {actual} bytes"),
        }
    }
}

impl std::error::Error for CgFishEventError {}

impl CgFishEvent {
    /// The fixed header byte.
    #[must_use]
    pub const fn header() -> CgHeader {
        HEADER_CG_FISH_EVENT_SEND
    }

    /// The payload after the header byte: the subheader and its body.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match *self {
            Self::BoxUse { window, cell } => {
                let [low, high] = cell.to_le_bytes();
                vec![FISH_EVENT_SUBHEADER_BOX_USE, window, low, high]
            }
            Self::ShapeAdd { pos } => vec![FISH_EVENT_SUBHEADER_SHAPE_ADD, pos],
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
    /// Returns [`CgFishEventError::InvalidHeader`] when the header is not 219, and
    /// [`CgFishEventError::LengthMismatch`] when the payload is empty or is not the
    /// width its subheader needs. The header is checked first.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgFishEventError> {
        if frame.header != HEADER_CG_FISH_EVENT_SEND.value() {
            return Err(CgFishEventError::InvalidHeader {
                actual: frame.header,
            });
        }
        let payload = frame.payload.as_slice();
        let Some(&subheader) = payload.first() else {
            return Err(CgFishEventError::LengthMismatch {
                subheader: None,
                expected: 1,
                actual: 0,
            });
        };
        let expected = match subheader {
            FISH_EVENT_SUBHEADER_BOX_USE => 4,
            FISH_EVENT_SUBHEADER_SHAPE_ADD => 2,
            _ => 1,
        };
        if payload.len() != expected {
            return Err(CgFishEventError::LengthMismatch {
                subheader: Some(subheader),
                expected,
                actual: payload.len(),
            });
        }
        Ok(match subheader {
            FISH_EVENT_SUBHEADER_BOX_USE => Self::BoxUse {
                window: payload[1],
                cell: u16::from_le_bytes([payload[2], payload[3]]),
            },
            FISH_EVENT_SUBHEADER_SHAPE_ADD => Self::ShapeAdd { pos: payload[1] },
            _ => Self::Unknown { subheader },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CgFishEvent, CgFishEventError};
    use crate::cg_variable::{
        resolve_variable_client_frame_size, FISH_EVENT_SUBHEADER_BOX_USE,
        FISH_EVENT_SUBHEADER_SHAPE_ADD,
    };
    use crate::cg_wire::ClientFrame;

    #[test]
    fn the_header_and_subheaders_match_the_source() {
        assert_eq!(CgFishEvent::header().value(), 219);
        assert_eq!(
            [FISH_EVENT_SUBHEADER_BOX_USE, FISH_EVENT_SUBHEADER_SHAPE_ADD],
            [0, 1]
        );
    }

    #[test]
    fn each_request_is_written_as_the_server_reads_it() {
        assert_eq!(
            CgFishEvent::BoxUse {
                window: 1,
                cell: 0x0203
            }
            .encode(),
            vec![219, 0, 1, 3, 2]
        );
        assert_eq!(CgFishEvent::ShapeAdd { pos: 17 }.encode(), vec![219, 1, 17]);
        assert_eq!(CgFishEvent::Unknown { subheader: 9 }.encode(), vec![219, 9]);
    }

    #[test]
    fn each_field_is_read_from_its_own_byte() {
        // Every field differs from the others, so a field read from the wrong byte is caught.
        let frame = |payload: &[u8]| ClientFrame::new(219, payload);
        assert_eq!(
            CgFishEvent::decode_frame(&frame(&[0, 7, 0x34, 0x12])),
            Ok(CgFishEvent::BoxUse {
                window: 7,
                cell: 0x1234
            })
        );
        assert_eq!(
            CgFishEvent::decode_frame(&frame(&[1, 9])),
            Ok(CgFishEvent::ShapeAdd { pos: 9 })
        );
        assert_eq!(
            CgFishEvent::decode_frame(&frame(&[4])),
            Ok(CgFishEvent::Unknown { subheader: 4 })
        );
    }

    #[test]
    fn the_framing_sizes_match_the_decoded_widths() {
        for encoded in [
            CgFishEvent::BoxUse { window: 1, cell: 3 }.encode(),
            CgFishEvent::ShapeAdd { pos: 3 }.encode(),
            CgFishEvent::Unknown { subheader: 2 }.encode(),
        ] {
            let sized = resolve_variable_client_frame_size(encoded[0], &encoded)
                .expect("fish-event resolves")
                .expect("prefix is long enough");
            assert_eq!(sized, encoded.len());
        }
    }

    #[test]
    fn a_wrong_header_an_empty_payload_and_a_short_body_are_refused() {
        assert_eq!(
            CgFishEvent::decode_frame(&ClientFrame::new(50, [0, 1, 0, 0])),
            Err(CgFishEventError::InvalidHeader { actual: 50 })
        );
        assert_eq!(
            CgFishEvent::decode_frame(&ClientFrame::new(219, [])),
            Err(CgFishEventError::LengthMismatch {
                subheader: None,
                expected: 1,
                actual: 0,
            })
        );
        assert_eq!(
            CgFishEvent::decode_frame(&ClientFrame::new(219, [0, 1, 3])),
            Err(CgFishEventError::LengthMismatch {
                subheader: Some(0),
                expected: 4,
                actual: 3,
            })
        );
        assert_eq!(
            CgFishEvent::decode_frame(&ClientFrame::new(219, [1])),
            Err(CgFishEventError::LengthMismatch {
                subheader: Some(1),
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(
            CgFishEvent::decode_frame(&ClientFrame::new(219, [2, 9])),
            Err(CgFishEventError::LengthMismatch {
                subheader: Some(2),
                expected: 1,
                actual: 2,
            })
        );
    }

    #[test]
    fn the_error_display_names_the_numbers() {
        assert_eq!(
            CgFishEventError::InvalidHeader { actual: 50 }.to_string(),
            "fish-event header 50 is not 219"
        );
        assert_eq!(
            CgFishEventError::LengthMismatch {
                subheader: Some(0),
                expected: 4,
                actual: 3,
            }
            .to_string(),
            "fish-event subheader 0 needs 4 payload bytes, got 3"
        );
        assert_eq!(
            CgFishEventError::LengthMismatch {
                subheader: None,
                expected: 1,
                actual: 0,
            }
            .to_string(),
            "fish-event payload has no subheader, got 0 bytes"
        );
    }
}
