//! Explicit codec for the fixed legacy `TPacketCGShoot` record.
//!
//! `server/server/game/packet.h:45,1760-1764` defines
//! `HEADER_CG_SHOOT = 0x36` and a packed record containing one header byte
//! followed by one opaque `BYTE bType`. `packet_info.cpp:140` registers the
//! two-byte record, and the client sends the same shape at
//! `PythonNetworkStreamPhaseGame.cpp:3174-3180`.
//!
//! This module only validates and preserves the wire record. It does not
//! interpret `bType`, look up a skill, target a character, or invoke the
//! legacy `Shoot` gameplay handler.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::HEADER_CG_SHOOT;
use crate::cg_wire::ClientFrame;

/// Complete packed wire size, including the one-byte header.
pub const CG_SHOOT_WIRE_SIZE: usize = 2;

/// One fixed client-to-game shoot record.
///
/// `shoot_type` is deliberately an opaque byte. The legacy handler passes it
/// to gameplay code, but this protocol boundary does not assign meaning to
/// any particular value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgShoot {
    /// Opaque source `bType` value.
    pub shoot_type: u8,
}

impl CgShoot {
    /// Construct a record without interpreting the source byte.
    #[must_use]
    pub const fn new(shoot_type: u8) -> Self {
        Self { shoot_type }
    }

    /// Encode the exact two-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        vec![HEADER_CG_SHOOT.value(), self.shoot_type]
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        ClientFrame::new(HEADER_CG_SHOOT.value(), [self.shoot_type])
    }

    /// Decode one exact complete shoot record.
    ///
    /// # Errors
    ///
    /// Returns [`CgShootError::Truncated`] for fewer than two bytes,
    /// [`CgShootError::LengthMismatch`] for more than two bytes, and
    /// [`CgShootError::InvalidHeader`] for another header.
    pub fn decode(data: &[u8]) -> Result<Self, CgShootError> {
        check_exact(data)?;
        decode_parts(data[0], data[1])
    }

    /// Decode a [`ClientFrame`] whose payload excludes the header byte.
    ///
    /// # Errors
    ///
    /// Returns a length error when the payload is not exactly one byte, or
    /// [`CgShootError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgShootError> {
        if frame.payload.len() != 1 {
            let available = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
            return if frame.payload.is_empty() {
                Err(CgShootError::Truncated {
                    needed: CG_SHOOT_WIRE_SIZE,
                    available,
                })
            } else {
                Err(CgShootError::LengthMismatch {
                    expected: CG_SHOOT_WIRE_SIZE,
                    actual: available,
                })
            };
        }
        decode_parts(frame.header, frame.payload[0])
    }
}

fn decode_parts(header: u8, shoot_type: u8) -> Result<CgShoot, CgShootError> {
    if header != HEADER_CG_SHOOT.value() {
        return Err(CgShootError::InvalidHeader { actual: header });
    }
    Ok(CgShoot::new(shoot_type))
}

/// A malformed fixed shoot record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgShootError {
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
        /// Bytes supplied by the caller.
        actual: usize,
    },
    /// The record used a header other than `0x36`.
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for CgShootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "shoot record is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "shoot record has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => {
                write!(formatter, "expected shoot header 0x36, got 0x{actual:02x}")
            }
        }
    }
}

impl Error for CgShootError {}

fn check_exact(data: &[u8]) -> Result<(), CgShootError> {
    match data.len().cmp(&CG_SHOOT_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(CgShootError::Truncated {
            needed: CG_SHOOT_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(CgShootError::LengthMismatch {
            expected: CG_SHOOT_WIRE_SIZE,
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
    fn golden_record_is_header_plus_opaque_type() {
        let packet = CgShoot::new(0x7f);
        assert_eq!(packet.encode(), vec![0x36, 0x7f]);
        assert_eq!(CgShoot::decode(&packet.encode()).unwrap(), packet);
    }

    #[test]
    fn all_byte_values_round_trip_without_gameplay_validation() {
        for shoot_type in [0, 1, 0x7f, 0x80, u8::MAX] {
            let packet = CgShoot::new(shoot_type);
            assert_eq!(CgShoot::decode(&packet.encode()).unwrap(), packet);
            assert_eq!(CgShoot::decode_frame(&packet.to_frame()).unwrap(), packet);
        }
    }

    #[test]
    fn truncation_trailing_bytes_and_wrong_headers_are_rejected() {
        let complete = CgShoot::new(1).encode();
        assert_eq!(
            CgShoot::decode(&[]),
            Err(CgShootError::Truncated {
                needed: 2,
                available: 0,
            })
        );
        assert_eq!(
            CgShoot::decode(&complete[..1]),
            Err(CgShootError::Truncated {
                needed: 2,
                available: 1,
            })
        );
        let mut long = complete.clone();
        long.push(0);
        assert_eq!(
            CgShoot::decode(&long),
            Err(CgShootError::LengthMismatch {
                expected: 2,
                actual: 3,
            })
        );
        assert_eq!(
            CgShoot::decode(&[0x37, 1]),
            Err(CgShootError::InvalidHeader { actual: 0x37 })
        );
    }

    #[test]
    fn frame_payload_lengths_and_headers_are_checked() {
        let packet = CgShoot::new(0x80);
        assert_eq!(CgShoot::decode_frame(&packet.to_frame()).unwrap(), packet);
        assert_eq!(
            CgShoot::decode_frame(&ClientFrame::new(0x36, [])),
            Err(CgShootError::Truncated {
                needed: 2,
                available: 1,
            })
        );
        assert_eq!(
            CgShoot::decode_frame(&ClientFrame::new(0x36, [1, 2])),
            Err(CgShootError::LengthMismatch {
                expected: 2,
                actual: 3,
            })
        );
        assert_eq!(
            CgShoot::decode_frame(&ClientFrame::new(0x37, [1])),
            Err(CgShootError::InvalidHeader { actual: 0x37 })
        );
    }

    #[test]
    fn fixed_decoder_handles_fragmented_and_coalesced_shoot_frames() {
        let first = CgShoot::new(0x11).encode();
        let second = CgShoot::new(0x22).encode();
        let mut decoder = ClientFrameDecoder::new();
        decoder.feed(&first[..1]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&first[1..]).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgShoot::decode_frame(&frame).unwrap(), CgShoot::new(0x11));

        let mut coalesced = second;
        coalesced.push(0);
        decoder.feed(&coalesced).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgShoot::decode_frame(&frame).unwrap(), CgShoot::new(0x22));
        assert_eq!(decoder.try_decode().unwrap().unwrap().header, 0);
    }
}
