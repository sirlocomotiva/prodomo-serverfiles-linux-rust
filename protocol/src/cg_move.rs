//! Explicit codec for the fixed legacy `TPacketCGMove` record.
//!
//! `server/server/game/packet.h:18,553-562` and
//! `client/Client/UserInterface/Packet.h:637-646` define the packed record
//! with header `0x07`, three opaque bytes, signed x86 coordinates, and a
//! `DWORD` client timestamp. `packet_info.cpp:136` registers the complete
//! record as a 16-byte fixed client frame.
//!
//! This module only converts that wire record. The legacy movement handler
//! applies character, phase, distance, speed, combat, and map policy; none of
//! those decisions belong in this transport-free payload boundary.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::HEADER_CG_MOVE;
use crate::cg_wire::ClientFrame;

/// Complete packed wire size, including the one-byte header.
pub const CG_MOVE_WIRE_SIZE: usize = 16;

/// One fixed client-to-game character-movement record.
///
/// The three `BYTE` fields remain opaque at this boundary. The legacy handler
/// interprets them only after character and movement policy checks; this
/// codec preserves every source byte value without making that decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgMove {
    /// Source `bFunc` byte.
    pub function: u8,
    /// Source `bArg` byte.
    pub argument: u8,
    /// Source `bRot` byte.
    pub rotation: u8,
    /// Signed source `lX` coordinate in active x86 units.
    pub x: i32,
    /// Signed source `lY` coordinate in active x86 units.
    pub y: i32,
    /// Opaque source `dwTime` client timestamp.
    pub time: u32,
}

impl CgMove {
    /// Construct a movement record without applying movement policy.
    #[must_use]
    pub const fn new(function: u8, argument: u8, rotation: u8, x: i32, y: i32, time: u32) -> Self {
        Self {
            function,
            argument,
            rotation,
            x,
            y,
            time,
        }
    }

    /// Encode the exact 16-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CG_MOVE_WIRE_SIZE);
        bytes.push(HEADER_CG_MOVE.value());
        bytes.push(self.function);
        bytes.push(self.argument);
        bytes.push(self.rotation);
        bytes.extend_from_slice(&self.x.to_le_bytes());
        bytes.extend_from_slice(&self.y.to_le_bytes());
        bytes.extend_from_slice(&self.time.to_le_bytes());
        bytes
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        let mut payload = [0_u8; CG_MOVE_WIRE_SIZE - 1];
        payload[0] = self.function;
        payload[1] = self.argument;
        payload[2] = self.rotation;
        payload[3..7].copy_from_slice(&self.x.to_le_bytes());
        payload[7..11].copy_from_slice(&self.y.to_le_bytes());
        payload[11..].copy_from_slice(&self.time.to_le_bytes());
        ClientFrame::new(HEADER_CG_MOVE.value(), payload)
    }

    /// Decode one exact complete movement record.
    ///
    /// # Errors
    ///
    /// Returns [`CgMoveError::Truncated`] for fewer than 16 bytes,
    /// [`CgMoveError::LengthMismatch`] for more than 16 bytes, and
    /// [`CgMoveError::InvalidHeader`] for another header.
    pub fn decode(data: &[u8]) -> Result<Self, CgMoveError> {
        check_exact(data)?;
        Self::decode_parts(data[0], &data[1..])
    }

    /// Decode a [`ClientFrame`] whose payload excludes the header byte.
    ///
    /// # Errors
    ///
    /// Returns a length error when the payload is not exactly 15 bytes, or
    /// [`CgMoveError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgMoveError> {
        let expected_payload = CG_MOVE_WIRE_SIZE - 1;
        if frame.payload.len() != expected_payload {
            let available = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
            return if frame.payload.len() < expected_payload {
                Err(CgMoveError::Truncated {
                    needed: CG_MOVE_WIRE_SIZE,
                    available,
                })
            } else {
                Err(CgMoveError::LengthMismatch {
                    expected: CG_MOVE_WIRE_SIZE,
                    actual: available,
                })
            };
        }
        Self::decode_parts(frame.header, &frame.payload)
    }

    fn decode_parts(header: u8, payload: &[u8]) -> Result<Self, CgMoveError> {
        if header != HEADER_CG_MOVE.value() {
            return Err(CgMoveError::InvalidHeader { actual: header });
        }
        // Both public decoders validate the exact payload length first.
        Ok(Self::new(
            payload[0],
            payload[1],
            payload[2],
            i32::from_le_bytes([payload[3], payload[4], payload[5], payload[6]]),
            i32::from_le_bytes([payload[7], payload[8], payload[9], payload[10]]),
            u32::from_le_bytes([payload[11], payload[12], payload[13], payload[14]]),
        ))
    }
}

/// A malformed fixed movement record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgMoveError {
    /// The complete record was shorter than its packed wire size.
    Truncated {
        /// Required complete wire size, including the header.
        needed: usize,
        /// Bytes supplied by the caller.
        available: usize,
    },
    /// The complete record had bytes beyond its packed wire size.
    LengthMismatch {
        /// Exact complete wire size, including the header.
        expected: usize,
        /// Supplied complete frame size.
        actual: usize,
    },
    /// The record used a header other than `0x07`.
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for CgMoveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "movement record is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "movement record has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => {
                write!(
                    formatter,
                    "expected movement header 0x07, got 0x{actual:02x}"
                )
            }
        }
    }
}

impl Error for CgMoveError {}

fn check_exact(data: &[u8]) -> Result<(), CgMoveError> {
    match data.len().cmp(&CG_MOVE_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(CgMoveError::Truncated {
            needed: CG_MOVE_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(CgMoveError::LengthMismatch {
            expected: CG_MOVE_WIRE_SIZE,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_wire::ClientFrameDecoder;

    #[test]
    fn golden_record_has_explicit_signed_little_endian_fields() {
        let packet = CgMove::new(1, 2, 3, -123_456, 0x0102_0304, 0xdead_beef);
        assert_eq!(packet.encode().len(), CG_MOVE_WIRE_SIZE);
        assert_eq!(
            packet.encode(),
            vec![
                0x07, 0x01, 0x02, 0x03, 0xc0, 0x1d, 0xfe, 0xff, 0x04, 0x03, 0x02, 0x01, 0xef, 0xbe,
                0xad, 0xde,
            ]
        );
        assert_eq!(CgMove::decode(&packet.encode()).unwrap(), packet);
    }

    #[test]
    fn opaque_bytes_signed_extrema_and_timestamp_extrema_round_trip() {
        for packet in [
            CgMove::new(0, 0, 0, 0, 0, 0),
            CgMove::new(0xff, 0x80, 0x7f, i32::MIN, i32::MAX, u32::MAX),
            CgMove::new(0x80, 0xff, 0xff, -1, 1, 1),
        ] {
            assert_eq!(CgMove::decode(&packet.encode()).unwrap(), packet);
            assert_eq!(CgMove::decode_frame(&packet.to_frame()).unwrap(), packet);
        }
    }

    #[test]
    fn every_truncated_length_is_rejected() {
        let complete = CgMove::new(1, 2, 3, 4, 5, 6).encode();
        for available in 0..CG_MOVE_WIRE_SIZE {
            assert_eq!(
                CgMove::decode(&complete[..available]),
                Err(CgMoveError::Truncated {
                    needed: CG_MOVE_WIRE_SIZE,
                    available,
                })
            );
        }
    }

    #[test]
    fn trailing_bytes_and_wrong_headers_are_rejected() {
        let mut long = CgMove::new(1, 2, 3, 4, 5, 6).encode();
        long.push(0);
        assert_eq!(
            CgMove::decode(&long),
            Err(CgMoveError::LengthMismatch {
                expected: CG_MOVE_WIRE_SIZE,
                actual: CG_MOVE_WIRE_SIZE + 1,
            })
        );

        let mut wrong = vec![0_u8; CG_MOVE_WIRE_SIZE];
        wrong[0] = 0x08;
        assert_eq!(
            CgMove::decode(&wrong),
            Err(CgMoveError::InvalidHeader { actual: 0x08 })
        );
    }

    #[test]
    fn frame_payload_lengths_and_headers_are_checked() {
        let packet = CgMove::new(0xff, 0x80, 0x7f, -1, 1, 7);
        assert_eq!(CgMove::decode_frame(&packet.to_frame()).unwrap(), packet);

        let short = ClientFrame::new(0x07, vec![0; CG_MOVE_WIRE_SIZE - 2]);
        assert_eq!(
            CgMove::decode_frame(&short),
            Err(CgMoveError::Truncated {
                needed: CG_MOVE_WIRE_SIZE,
                available: CG_MOVE_WIRE_SIZE - 1,
            })
        );
        let long = ClientFrame::new(0x07, vec![0; CG_MOVE_WIRE_SIZE]);
        assert_eq!(
            CgMove::decode_frame(&long),
            Err(CgMoveError::LengthMismatch {
                expected: CG_MOVE_WIRE_SIZE,
                actual: CG_MOVE_WIRE_SIZE + 1,
            })
        );
        let wrong = ClientFrame::new(0x08, vec![0; CG_MOVE_WIRE_SIZE - 1]);
        assert_eq!(
            CgMove::decode_frame(&wrong),
            Err(CgMoveError::InvalidHeader { actual: 0x08 })
        );
    }

    #[test]
    fn fixed_decoder_handles_fragmented_and_coalesced_move_frames() {
        let first = CgMove::new(1, 2, 3, -10, 20, 30).encode();
        let second = CgMove::new(4, 5, 6, 40, 50, 60).encode();
        let mut decoder = ClientFrameDecoder::new();

        decoder.feed(&first[..5]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&first[5..]).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgMove::decode_frame(&frame).unwrap(),
            CgMove::new(1, 2, 3, -10, 20, 30)
        );

        let mut coalesced = second;
        coalesced.push(0);
        decoder.feed(&coalesced).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgMove::decode_frame(&frame).unwrap(),
            CgMove::new(4, 5, 6, 40, 50, 60)
        );
        assert_eq!(decoder.try_decode().unwrap().unwrap().header, 0);
    }
}
