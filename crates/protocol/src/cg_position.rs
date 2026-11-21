//! `HEADER_CG_CHARACTER_POSITION` (28): the client saying it sat down or stood up.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:737-741`:
//!
//! ```ignore
//! typedef struct command_position
//! {
//!     BYTE header;
//!     BYTE position;
//! } TPacketCGPosition;
//! ```
//!
//! The whole record is 1 + 1 = 2 bytes, which `packet_info.cpp:131` registers as
//! `sizeof(TPacketCGPosition)`. The handler is `CInputMain::Position`
//! (`server/server/game/input_main.cpp:1530-1548`), which switches on `position`
//! and ignores every other value.
//!
//! # The byte is not sub-typed here
//!
//! `position` is a `switch` input, not an enum: the handler has three named arms and
//! a silent fall-through. All 256 values are therefore representable and round-trip.
//! Deciding what a byte means is `CInputMain::Position`'s job, and the Rewrite keeps
//! that switch above this codec so a client byte the legacy handler ignores stays
//! ignorable instead of becoming a decode error.
//!
//! # A legacy defect, not reproduced
//!
//! `CInputMain::Position` calls `Sitdown(0)` for `POSITION_SITTING_CHAIR` and
//! `Sitdown(1)` for `POSITION_SITTING_GROUND`, but `CHARACTER::Sitdown`
//! (`server/server/game/char.cpp:3338-3350`) ignores its argument and always puts
//! `POSITION_SITTING_GROUND` on the wire. Every client therefore sees a ground sit
//! for a chair request. The Rewrite honours the request.

#![warn(missing_docs)]

use std::fmt;

use crate::cg_inventory::HEADER_CG_CHARACTER_POSITION;
use crate::cg_wire::ClientFrame;

/// `sizeof(TPacketCGPosition)`, including the one-byte header.
pub const CG_CHARACTER_POSITION_WIRE_SIZE: usize = 2;

/// `POSITION_GENERAL` (`server/server/common/length.h`).
pub const POSITION_GENERAL: u8 = 0;

/// `POSITION_SITTING_CHAIR`.
pub const POSITION_SITTING_CHAIR: u8 = 1;

/// `POSITION_SITTING_GROUND`.
pub const POSITION_SITTING_GROUND: u8 = 2;

/// One fixed client-to-game pose record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgCharacterPosition {
    /// The pose byte, relayed unchanged for the handler to interpret.
    pub position: u8,
}

impl CgCharacterPosition {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_CG_CHARACTER_POSITION.value()
    }

    /// A pose record with the given byte.
    #[must_use]
    pub const fn new(position: u8) -> Self {
        Self { position }
    }

    /// The exact packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        vec![Self::header(), self.position]
    }

    /// The record as a complete client frame.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        ClientFrame {
            header: Self::header(),
            payload: vec![self.position],
        }
    }

    /// Read a record back from the frame payload, header excluded.
    ///
    /// # Errors
    ///
    /// Returns [`CgPositionError::Payload`] when the payload is not exactly
    /// [`CG_CHARACTER_POSITION_WIRE_SIZE`] - 1 bytes. The check is on the payload,
    /// because the frame excludes the header byte.
    pub fn decode(frame: &ClientFrame) -> Result<Self, CgPositionError> {
        Self::decode_payload(frame.header, &frame.payload)
    }

    /// Read a record back from a payload that still carries its header byte.
    ///
    /// # Errors
    ///
    /// Returns [`CgPositionError::Truncated`] when fewer than
    /// [`CG_CHARACTER_POSITION_WIRE_SIZE`] bytes are present,
    /// [`CgPositionError::Header`] when the leading byte is not 28, and
    /// [`CgPositionError::Payload`] when the record is longer than the legacy width.
    pub fn decode_bytes(bytes: &[u8]) -> Result<Self, CgPositionError> {
        if bytes.len() < CG_CHARACTER_POSITION_WIRE_SIZE {
            return Err(CgPositionError::Truncated {
                context: "CgCharacterPosition",
                needed: CG_CHARACTER_POSITION_WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(CgPositionError::Header {
                context: "CgCharacterPosition",
                expected: Self::header(),
                actual: bytes[0],
            });
        }
        if bytes.len() != CG_CHARACTER_POSITION_WIRE_SIZE {
            return Err(CgPositionError::Payload {
                context: "CgCharacterPosition",
                actual: bytes.len(),
            });
        }
        Ok(Self { position: bytes[1] })
    }

    fn decode_payload(header: u8, payload: &[u8]) -> Result<Self, CgPositionError> {
        if header != Self::header() {
            return Err(CgPositionError::Header {
                context: "CgCharacterPosition",
                expected: Self::header(),
                actual: header,
            });
        }
        if payload.len() != CG_CHARACTER_POSITION_WIRE_SIZE - 1 {
            return Err(CgPositionError::Payload {
                context: "CgCharacterPosition",
                actual: payload.len(),
            });
        }
        Ok(Self {
            position: payload[0],
        })
    }
}

/// What a pose-record decode can report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgPositionError {
    /// The buffer ended before the record did.
    Truncated {
        /// Which record was being decoded.
        context: &'static str,
        /// Bytes the record needs.
        needed: usize,
        /// Bytes the buffer actually had.
        actual: usize,
    },
    /// The leading byte is not this record's header.
    Header {
        /// Which record was being decoded.
        context: &'static str,
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was present.
        actual: u8,
    },
    /// The payload is not the legacy width.
    Payload {
        /// Which record was being decoded.
        context: &'static str,
        /// The payload length that was present, header excluded.
        actual: usize,
    },
}

impl fmt::Display for CgPositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                context,
                needed,
                actual,
            } => write!(f, "{context}: need {needed} bytes, buffer held {actual}"),
            Self::Header {
                context,
                expected,
                actual,
            } => write!(
                f,
                "{context}: header {expected} expected, buffer held {actual}"
            ),
            Self::Payload { context, actual } => write!(
                f,
                "{context}: payload was {actual} bytes, the legacy width is {}",
                CG_CHARACTER_POSITION_WIRE_SIZE - 1
            ),
        }
    }
}

impl std::error::Error for CgPositionError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct byte halves, so a big-endian read cannot pass by symmetry.
    const VIDLIKE: u8 = 0xa7;

    #[test]
    fn the_golden_bytes_are_the_legacy_field_order() {
        assert_eq!(
            CgCharacterPosition::new(POSITION_SITTING_GROUND).encode(),
            vec![28, 2],
            "header then the pose byte",
        );
    }

    #[test]
    fn the_header_is_twenty_eight() {
        assert_eq!(CgCharacterPosition::header(), 0x1c);
    }

    #[test]
    fn the_wire_size_is_two() {
        assert_eq!(CG_CHARACTER_POSITION_WIRE_SIZE, 2);
        assert_eq!(CgCharacterPosition::new(0).encode().len(), 2);
    }

    #[test]
    fn a_frame_holds_the_pose_byte_and_nothing_else() {
        let frame = CgCharacterPosition::new(POSITION_GENERAL).to_frame();
        assert_eq!(frame.header, 28);
        assert_eq!(frame.payload, vec![0]);
    }

    #[test]
    fn a_record_round_trips_through_its_frame() {
        for position in 0..=u8::MAX {
            let original = CgCharacterPosition::new(position);
            let decoded = CgCharacterPosition::decode(&original.to_frame()).expect("round trip");
            assert_eq!(decoded, original);
        }
    }

    #[test]
    fn a_record_round_trips_through_raw_bytes() {
        for position in 0..=u8::MAX {
            let original = CgCharacterPosition::new(position);
            let decoded =
                CgCharacterPosition::decode_bytes(&original.encode()).expect("round trip");
            assert_eq!(decoded, original);
        }
    }

    #[test]
    fn a_short_buffer_is_refused_rather_than_read_past() {
        let error = CgCharacterPosition::decode_bytes(&[28]).expect_err("one byte is short");
        assert!(matches!(
            error,
            CgPositionError::Truncated { actual: 1, .. }
        ));
    }

    #[test]
    fn a_long_buffer_is_refused_rather_than_truncated_silently() {
        let error =
            CgCharacterPosition::decode_bytes(&[28, 2, 9]).expect_err("three bytes is long");
        assert!(matches!(error, CgPositionError::Payload { actual: 3, .. }));
    }

    #[test]
    fn a_wrong_header_is_refused() {
        let error = CgCharacterPosition::decode_bytes(&[29, 2]).expect_err("29 is not 28");
        assert!(matches!(error, CgPositionError::Header { actual: 29, .. }));
    }

    #[test]
    fn a_wrong_frame_header_is_refused() {
        let frame = ClientFrame {
            header: 29,
            payload: vec![2],
        };
        let error = CgCharacterPosition::decode(&frame).expect_err("29 is not 28");
        assert!(matches!(error, CgPositionError::Header { actual: 29, .. }));
    }

    #[test]
    fn a_wide_frame_payload_is_refused() {
        let frame = ClientFrame {
            header: 28,
            payload: vec![2, 0],
        };
        let error = CgCharacterPosition::decode(&frame).expect_err("two payload bytes is wide");
        assert!(matches!(error, CgPositionError::Payload { actual: 2, .. }));
    }

    #[test]
    fn an_empty_frame_payload_is_refused() {
        let frame = ClientFrame {
            header: 28,
            payload: Vec::new(),
        };
        let error = CgCharacterPosition::decode(&frame).expect_err("no pose byte is not a record");
        assert!(matches!(error, CgPositionError::Payload { actual: 0, .. }));
    }

    #[test]
    fn an_unknown_pose_byte_is_kept_for_the_handler_to_ignore() {
        let decoded =
            CgCharacterPosition::decode_bytes(&[28, VIDLIKE]).expect("any byte is a record");
        assert_eq!(decoded.position, VIDLIKE);
    }

    #[test]
    fn the_error_text_names_the_record() {
        let error = CgCharacterPosition::decode_bytes(&[]).expect_err("empty");
        assert!(error.to_string().contains("CgCharacterPosition"), "{error}");
    }
}
