//! Explicit codec for the fixed legacy client-version report.
//!
//! `server/server/game/packet.h:2465-2477` declares
//! `TPacketCGClientVersion` and `TPacketCGClientVersion2` as one header byte
//! followed by two 33-byte character arrays. `packet_info.cpp:166-167`
//! registers both records at 67 bytes. The second header (`0xf1`) is also used
//! by the later Gaya registration, but `CPacketInfo::Set` keeps the first
//! registration; this codec follows that source behavior and preserves the
//! two header variants without interpreting either field as text.
//!
//! This module is a wire boundary only. It does not update a descriptor's
//! client-version state, log a filename, open a socket, or perform handshake
//! or encryption work.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::{HEADER_CG_CLIENT_VERSION, HEADER_CG_CLIENT_VERSION2};
use crate::cg_wire::ClientFrame;

/// Size of each source `char` array in the client-version report.
pub const CG_CLIENT_VERSION_FIELD_SIZE: usize = 33;

/// Complete packed wire size, including the one-byte header.
pub const CG_CLIENT_VERSION_WIRE_SIZE: usize = 1 + (2 * CG_CLIENT_VERSION_FIELD_SIZE);

/// The two source-defined client-version headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgClientVersionHeader {
    /// `HEADER_CG_CLIENT_VERSION` (`0xfd`).
    Version,
    /// `HEADER_CG_CLIENT_VERSION2` (`0xf1`).
    ///
    /// The legacy packet-info map keeps this first registration when the
    /// later Gaya registration uses the same numeric value.
    Version2,
}

impl CgClientVersionHeader {
    /// Return the exact one-byte legacy header value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::Version => HEADER_CG_CLIENT_VERSION.value(),
            Self::Version2 => HEADER_CG_CLIENT_VERSION2.value(),
        }
    }
}

/// One complete client-to-game client-version report.
///
/// The fields are opaque bytes because the C++ record uses character arrays;
/// decoding does not require UTF-8, a NUL terminator, or a particular text
/// representation. Any source bytes, including zeroes and non-UTF-8 values,
/// round-trip unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgClientVersion {
    /// Source header variant.
    pub header: CgClientVersionHeader,
    /// Opaque 33-byte filename field.
    pub filename: [u8; CG_CLIENT_VERSION_FIELD_SIZE],
    /// Opaque 33-byte timestamp field.
    pub timestamp: [u8; CG_CLIENT_VERSION_FIELD_SIZE],
}

impl CgClientVersion {
    /// Construct a report without interpreting either opaque field.
    #[must_use]
    pub const fn new(
        header: CgClientVersionHeader,
        filename: [u8; CG_CLIENT_VERSION_FIELD_SIZE],
        timestamp: [u8; CG_CLIENT_VERSION_FIELD_SIZE],
    ) -> Self {
        Self {
            header,
            filename,
            timestamp,
        }
    }

    /// Encode the exact complete 67-byte legacy record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CG_CLIENT_VERSION_WIRE_SIZE);
        bytes.push(self.header.value());
        bytes.extend_from_slice(&self.filename);
        bytes.extend_from_slice(&self.timestamp);
        bytes
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_CLIENT_VERSION_WIRE_SIZE - 1);
        payload.extend_from_slice(&self.filename);
        payload.extend_from_slice(&self.timestamp);
        ClientFrame::new(self.header.value(), payload)
    }

    /// Decode one exact complete client-version record.
    ///
    /// # Errors
    ///
    /// Returns [`CgClientVersionError::Truncated`] for fewer than 67 bytes,
    /// [`CgClientVersionError::LengthMismatch`] for more than 67 bytes, and
    /// [`CgClientVersionError::InvalidHeader`] for another header.
    pub fn decode(data: &[u8]) -> Result<Self, CgClientVersionError> {
        check_exact(data)?;
        Self::decode_parts(data[0], &data[1..])
    }

    /// Decode a [`ClientFrame`] whose payload excludes the header byte.
    ///
    /// # Errors
    ///
    /// Returns a length error when the frame payload is not exactly 66 bytes,
    /// or [`CgClientVersionError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgClientVersionError> {
        let expected_payload = CG_CLIENT_VERSION_WIRE_SIZE - 1;
        if frame.payload.len() != expected_payload {
            let available = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
            return if frame.payload.len() < expected_payload {
                Err(CgClientVersionError::Truncated {
                    needed: CG_CLIENT_VERSION_WIRE_SIZE,
                    available,
                })
            } else {
                Err(CgClientVersionError::LengthMismatch {
                    expected: CG_CLIENT_VERSION_WIRE_SIZE,
                    actual: available,
                })
            };
        }
        Self::decode_parts(frame.header, &frame.payload)
    }

    fn decode_parts(header: u8, payload: &[u8]) -> Result<Self, CgClientVersionError> {
        let header = match header {
            value if value == HEADER_CG_CLIENT_VERSION.value() => CgClientVersionHeader::Version,
            value if value == HEADER_CG_CLIENT_VERSION2.value() => CgClientVersionHeader::Version2,
            actual => return Err(CgClientVersionError::InvalidHeader { actual }),
        };

        let filename_start = 0;
        let timestamp_start = CG_CLIENT_VERSION_FIELD_SIZE;
        let mut filename = [0_u8; CG_CLIENT_VERSION_FIELD_SIZE];
        let mut timestamp = [0_u8; CG_CLIENT_VERSION_FIELD_SIZE];
        filename.copy_from_slice(&payload[filename_start..timestamp_start]);
        timestamp.copy_from_slice(
            &payload[timestamp_start..timestamp_start + CG_CLIENT_VERSION_FIELD_SIZE],
        );
        Ok(Self::new(header, filename, timestamp))
    }
}

/// A malformed fixed client-version record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgClientVersionError {
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
    /// The record used a header other than the two source-defined variants.
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for CgClientVersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "client-version record is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "client-version record has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => write!(
                formatter,
                "expected client-version header 0xfd or 0xf1, got 0x{actual:02x}"
            ),
        }
    }
}

impl Error for CgClientVersionError {}

fn check_exact(data: &[u8]) -> Result<(), CgClientVersionError> {
    match data.len().cmp(&CG_CLIENT_VERSION_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(CgClientVersionError::Truncated {
            needed: CG_CLIENT_VERSION_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(CgClientVersionError::LengthMismatch {
            expected: CG_CLIENT_VERSION_WIRE_SIZE,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_wire::ClientFrameDecoder;

    fn fields(
        seed: u8,
    ) -> (
        [u8; CG_CLIENT_VERSION_FIELD_SIZE],
        [u8; CG_CLIENT_VERSION_FIELD_SIZE],
    ) {
        let filename = [seed; CG_CLIENT_VERSION_FIELD_SIZE];
        let timestamp = [seed.wrapping_add(1); CG_CLIENT_VERSION_FIELD_SIZE];
        (filename, timestamp)
    }

    #[test]
    fn golden_headers_and_fixed_field_widths_round_trip() {
        let (filename, timestamp) = fields(0x41);
        for header in [
            CgClientVersionHeader::Version,
            CgClientVersionHeader::Version2,
        ] {
            let packet = CgClientVersion::new(header, filename, timestamp);
            let encoded = packet.encode();
            assert_eq!(encoded.len(), CG_CLIENT_VERSION_WIRE_SIZE);
            assert_eq!(encoded[0], header.value());
            assert_eq!(&encoded[1..34], &filename);
            assert_eq!(&encoded[34..], &timestamp);
            assert_eq!(CgClientVersion::decode(&encoded).unwrap(), packet);
            assert_eq!(
                CgClientVersion::decode_frame(&packet.to_frame()).unwrap(),
                packet
            );
        }
        assert_eq!(CgClientVersionHeader::Version.value(), 0xfd);
        assert_eq!(CgClientVersionHeader::Version2.value(), 0xf1);
    }

    #[test]
    fn opaque_non_utf8_and_embedded_zero_fields_are_preserved() {
        let filename = [0xff; CG_CLIENT_VERSION_FIELD_SIZE];
        let timestamp = {
            let mut value = [0_u8; CG_CLIENT_VERSION_FIELD_SIZE];
            value[0] = 0;
            value[1] = 0xfe;
            value[CG_CLIENT_VERSION_FIELD_SIZE - 1] = 0xff;
            value
        };
        let packet = CgClientVersion::new(CgClientVersionHeader::Version, filename, timestamp);
        assert_eq!(CgClientVersion::decode(&packet.encode()).unwrap(), packet);
    }

    #[test]
    fn every_truncated_length_is_rejected() {
        let complete = CgClientVersion::new(
            CgClientVersionHeader::Version,
            [0; CG_CLIENT_VERSION_FIELD_SIZE],
            [0; CG_CLIENT_VERSION_FIELD_SIZE],
        )
        .encode();
        for available in 0..CG_CLIENT_VERSION_WIRE_SIZE {
            assert_eq!(
                CgClientVersion::decode(&complete[..available]),
                Err(CgClientVersionError::Truncated {
                    needed: CG_CLIENT_VERSION_WIRE_SIZE,
                    available,
                })
            );
        }
    }

    #[test]
    fn trailing_bytes_and_wrong_headers_are_rejected() {
        let mut long = CgClientVersion::new(
            CgClientVersionHeader::Version2,
            [1; CG_CLIENT_VERSION_FIELD_SIZE],
            [2; CG_CLIENT_VERSION_FIELD_SIZE],
        )
        .encode();
        long.push(0);
        assert_eq!(
            CgClientVersion::decode(&long),
            Err(CgClientVersionError::LengthMismatch {
                expected: CG_CLIENT_VERSION_WIRE_SIZE,
                actual: CG_CLIENT_VERSION_WIRE_SIZE + 1,
            })
        );

        let mut wrong = vec![0x00; CG_CLIENT_VERSION_WIRE_SIZE];
        wrong[0] = 0xfc;
        assert_eq!(
            CgClientVersion::decode(&wrong),
            Err(CgClientVersionError::InvalidHeader { actual: 0xfc })
        );
    }

    #[test]
    fn frame_payload_lengths_and_headers_are_checked() {
        let packet = CgClientVersion::new(
            CgClientVersionHeader::Version,
            [3; CG_CLIENT_VERSION_FIELD_SIZE],
            [4; CG_CLIENT_VERSION_FIELD_SIZE],
        );
        assert_eq!(
            CgClientVersion::decode_frame(&packet.to_frame()).unwrap(),
            packet
        );

        let short = ClientFrame::new(0xfd, vec![0; CG_CLIENT_VERSION_WIRE_SIZE - 2]);
        assert_eq!(
            CgClientVersion::decode_frame(&short),
            Err(CgClientVersionError::Truncated {
                needed: CG_CLIENT_VERSION_WIRE_SIZE,
                available: CG_CLIENT_VERSION_WIRE_SIZE - 1,
            })
        );

        let long = ClientFrame::new(0xfd, vec![0; CG_CLIENT_VERSION_WIRE_SIZE]);
        assert_eq!(
            CgClientVersion::decode_frame(&long),
            Err(CgClientVersionError::LengthMismatch {
                expected: CG_CLIENT_VERSION_WIRE_SIZE,
                actual: CG_CLIENT_VERSION_WIRE_SIZE + 1,
            })
        );

        let wrong = ClientFrame::new(0xfc, vec![0; CG_CLIENT_VERSION_WIRE_SIZE - 1]);
        assert_eq!(
            CgClientVersion::decode_frame(&wrong),
            Err(CgClientVersionError::InvalidHeader { actual: 0xfc })
        );
    }

    #[test]
    fn fixed_decoder_handles_fragmented_and_coalesced_version_frames() {
        let first = CgClientVersion::new(
            CgClientVersionHeader::Version,
            [0x11; CG_CLIENT_VERSION_FIELD_SIZE],
            [0x22; CG_CLIENT_VERSION_FIELD_SIZE],
        )
        .encode();
        let second = CgClientVersion::new(
            CgClientVersionHeader::Version2,
            [0x33; CG_CLIENT_VERSION_FIELD_SIZE],
            [0x44; CG_CLIENT_VERSION_FIELD_SIZE],
        )
        .encode();
        let mut decoder = ClientFrameDecoder::new();

        decoder.feed(&first[..3]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&first[3..]).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgClientVersion::decode_frame(&frame).unwrap().header,
            CgClientVersionHeader::Version
        );

        let mut coalesced = second;
        coalesced.push(0x00);
        decoder.feed(&coalesced).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgClientVersion::decode_frame(&frame).unwrap().header,
            CgClientVersionHeader::Version2
        );
        assert_eq!(decoder.try_decode().unwrap().unwrap().header, 0x00);
    }
}
