//! Explicit client-to-game handshake and time-sync record codec.
//!
//! The legacy x86 `TPacketCGHandshake` is a packed 13-byte record: one
//! header byte followed by a `u32` token, a `u32` time, and a signed x86
//! `long` delta. Headers `0xff` (`HEADER_CG_HANDSHAKE`) and `0xfc`
//! (`HEADER_CG_TIME_SYNC`) both use this exact layout. This module is only a
//! record boundary; it does not frame TCP input or perform descriptor state
//! changes.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::{HEADER_CG_HANDSHAKE, HEADER_CG_TIME_SYNC};
use crate::cg_wire::ClientFrame;

/// Complete packed wire size, including the one-byte header.
pub const CG_HANDSHAKE_WIRE_SIZE: usize = 13;

/// Wire bytes after the one-byte header.
pub const CG_HANDSHAKE_BODY_SIZE: usize = 12;

/// The two source-defined client-to-game handshake headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgHandshakeHeader {
    /// Initial handshake response (`HEADER_CG_HANDSHAKE`, `0xff`).
    Handshake,
    /// Post-handshake time synchronization (`HEADER_CG_TIME_SYNC`, `0xfc`).
    TimeSync,
}

impl CgHandshakeHeader {
    /// Return the exact one-byte legacy header value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::Handshake => HEADER_CG_HANDSHAKE.value(),
            Self::TimeSync => HEADER_CG_TIME_SYNC.value(),
        }
    }
}

/// One complete inbound client handshake or time-sync record.
///
/// Every field is encoded and decoded explicitly in the active x86 wire
/// order. The Rust type has no packed-memory-layout contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgInboundHandshake {
    /// Source header, preserved without normalizing `0xfc` to `0xff`.
    pub header: CgHandshakeHeader,
    /// Opaque descriptor token echoed by the client.
    pub token: u32,
    /// Client timestamp used by the legacy time calculation.
    pub time: u32,
    /// Signed time correction.
    pub delta: i32,
}

impl CgInboundHandshake {
    /// Construct a typed record without choosing a wire byte order.
    #[must_use]
    pub const fn new(header: CgHandshakeHeader, token: u32, time: u32, delta: i32) -> Self {
        Self {
            header,
            token,
            time,
            delta,
        }
    }

    /// Encode the exact complete 13-byte legacy record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CG_HANDSHAKE_WIRE_SIZE);
        bytes.push(self.header.value());
        bytes.extend_from_slice(&self.token.to_le_bytes());
        bytes.extend_from_slice(&self.time.to_le_bytes());
        bytes.extend_from_slice(&self.delta.to_le_bytes());
        bytes
    }

    /// Decode one exact complete record.
    ///
    /// # Errors
    ///
    /// Returns [`CgHandshakeError::Truncated`] for fewer than 13 bytes,
    /// [`CgHandshakeError::LengthMismatch`] for more than 13 bytes, and
    /// [`CgHandshakeError::InvalidHeader`] for a complete record whose header
    /// is neither `0xff` nor `0xfc`.
    pub fn decode(data: &[u8]) -> Result<Self, CgHandshakeError> {
        check_exact(data)?;
        Self::decode_parts(data[0], &data[1..])
    }

    /// Decode a header and its 12-byte body without copying a complete wire
    /// record into a temporary vector.
    ///
    /// This validates the exact handshake body width and header, but does not
    /// consult the packet-info inventory. A framing adapter must resolve the
    /// inventory size separately before calling this method.
    ///
    /// # Errors
    ///
    /// Returns [`CgHandshakeError::Truncated`] or
    /// [`CgHandshakeError::LengthMismatch`] when the body is not exactly 12
    /// bytes, and [`CgHandshakeError::InvalidHeader`] for any other header.
    pub fn decode_parts(header: u8, body: &[u8]) -> Result<Self, CgHandshakeError> {
        check_body_exact(body)?;
        let header = match header {
            value if value == HEADER_CG_HANDSHAKE.value() => CgHandshakeHeader::Handshake,
            value if value == HEADER_CG_TIME_SYNC.value() => CgHandshakeHeader::TimeSync,
            actual => return Err(CgHandshakeError::InvalidHeader { actual }),
        };

        Ok(Self {
            header,
            token: u32::from_le_bytes([body[0], body[1], body[2], body[3]]),
            time: u32::from_le_bytes([body[4], body[5], body[6], body[7]]),
            delta: i32::from_le_bytes([body[8], body[9], body[10], body[11]]),
        })
    }

    /// Decode a validated fixed client frame without reconstructing its bytes.
    ///
    /// The frame's payload is borrowed and must contain exactly the 12 bytes
    /// following the one-byte header. Inventory resolution remains the caller's
    /// responsibility, as with [`Self::decode`].
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::decode_parts`].
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgHandshakeError> {
        Self::decode_parts(frame.header, &frame.payload)
    }
}

/// A malformed fixed client handshake record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgHandshakeError {
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
    /// The complete record used a header outside the handshake subset.
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for CgHandshakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "CG handshake is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "CG handshake has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => write!(
                formatter,
                "expected CG handshake header 0xff or 0xfc, got 0x{actual:02x}"
            ),
        }
    }
}

impl Error for CgHandshakeError {}

fn check_exact(data: &[u8]) -> Result<(), CgHandshakeError> {
    match data.len().cmp(&CG_HANDSHAKE_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(CgHandshakeError::Truncated {
            needed: CG_HANDSHAKE_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(CgHandshakeError::LengthMismatch {
            expected: CG_HANDSHAKE_WIRE_SIZE,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_body_exact(body: &[u8]) -> Result<(), CgHandshakeError> {
    match body.len().cmp(&CG_HANDSHAKE_BODY_SIZE) {
        std::cmp::Ordering::Less => Err(CgHandshakeError::Truncated {
            needed: CG_HANDSHAKE_WIRE_SIZE,
            available: body.len().saturating_add(1),
        }),
        std::cmp::Ordering::Greater => Err(CgHandshakeError::LengthMismatch {
            expected: CG_HANDSHAKE_WIRE_SIZE,
            actual: body.len().saturating_add(1),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_handshake_and_time_sync_use_exact_header_plus_12_byte_body() {
        let handshake = CgInboundHandshake::new(
            CgHandshakeHeader::Handshake,
            0x1234_5678,
            0xdead_beef,
            -123_456,
        );
        let time_sync = CgInboundHandshake::new(
            CgHandshakeHeader::TimeSync,
            0x1234_5678,
            0xdead_beef,
            -123_456,
        );

        let expected_body = [
            0x78, 0x56, 0x34, 0x12, 0xef, 0xbe, 0xad, 0xde, 0xc0, 0x1d, 0xfe, 0xff,
        ];
        assert_eq!(handshake.encode().len(), CG_HANDSHAKE_WIRE_SIZE);
        assert_eq!(
            handshake.encode(),
            [0xff].into_iter().chain(expected_body).collect::<Vec<_>>()
        );
        assert_eq!(
            time_sync.encode(),
            [0xfc].into_iter().chain(expected_body).collect::<Vec<_>>()
        );
        assert_eq!(
            CgInboundHandshake::decode(&handshake.encode()).unwrap(),
            handshake
        );
        assert_eq!(
            CgInboundHandshake::decode(&time_sync.encode()).unwrap(),
            time_sync
        );
    }

    #[test]
    fn borrowed_parts_and_frame_decode_match_exact_wire_bytes() {
        let packet = CgInboundHandshake::new(
            CgHandshakeHeader::TimeSync,
            0xdead_beef,
            0x1234_5678,
            -123_456,
        );
        let encoded = packet.encode();
        assert_eq!(
            CgInboundHandshake::decode_parts(encoded[0], &encoded[1..]),
            Ok(packet)
        );
        let frame = ClientFrame::new(encoded[0], &encoded[1..]);
        assert_eq!(CgInboundHandshake::decode_frame(&frame), Ok(packet));
        assert_eq!(
            CgInboundHandshake::decode_parts(0xff, &encoded[1..11]),
            Err(CgHandshakeError::Truncated {
                needed: CG_HANDSHAKE_WIRE_SIZE,
                available: 11,
            })
        );
    }

    #[test]
    fn every_truncated_length_is_rejected() {
        let complete = CgInboundHandshake::new(CgHandshakeHeader::Handshake, 1, 2, 3).encode();

        for available in 0..CG_HANDSHAKE_WIRE_SIZE {
            assert_eq!(
                CgInboundHandshake::decode(&complete[..available]),
                Err(CgHandshakeError::Truncated {
                    needed: CG_HANDSHAKE_WIRE_SIZE,
                    available,
                })
            );
        }
    }

    #[test]
    fn trailing_bytes_and_non_handshake_headers_are_rejected() {
        let mut long = CgInboundHandshake::new(CgHandshakeHeader::Handshake, 1, 2, 3).encode();
        long.push(0);
        assert_eq!(
            CgInboundHandshake::decode(&long),
            Err(CgHandshakeError::LengthMismatch {
                expected: CG_HANDSHAKE_WIRE_SIZE,
                actual: 14,
            })
        );

        let mut wrong_header = vec![0xfe; CG_HANDSHAKE_WIRE_SIZE];
        wrong_header[0] = 0xfe;
        assert_eq!(
            CgInboundHandshake::decode(&wrong_header),
            Err(CgHandshakeError::InvalidHeader { actual: 0xfe })
        );
    }

    #[test]
    fn unsigned_and_signed_field_extremes_round_trip() {
        for (token, time) in [(0, 0), (1, u32::MAX), (u32::MAX, 1)] {
            for delta in [i32::MIN, -1, 0, 1, i32::MAX] {
                for header in [CgHandshakeHeader::Handshake, CgHandshakeHeader::TimeSync] {
                    let packet = CgInboundHandshake::new(header, token, time, delta);
                    let encoded = packet.encode();
                    assert_eq!(encoded.len(), 1 + CG_HANDSHAKE_BODY_SIZE);
                    assert_eq!(CgInboundHandshake::decode(&encoded).unwrap(), packet);
                }
            }
        }
    }
}
