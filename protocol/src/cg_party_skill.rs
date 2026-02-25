//! Explicit codec for the fixed legacy `TPacketCGPartyUseSkill` record.
//!
//! The active x86 declaration is packed as one header byte, one skill-index
//! byte, and one 32-bit target VID. The record is registered as a fixed
//! six-byte client frame. This module only validates and converts that wire
//! record; it does not look up a party, character, or VID, and it does not
//! execute the legacy skill handler.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::HEADER_CG_PARTY_USE_SKILL;
use crate::cg_wire::ClientFrame;

/// Complete packed wire size, including the one-byte header.
pub const CG_PARTY_USE_SKILL_WIRE_SIZE: usize = 6;

/// Source value for the party-heal action in `packet.h`.
pub const PARTY_SKILL_HEAL: u8 = 1;

/// Source value for the party-warp action in `packet.h`.
pub const PARTY_SKILL_WARP: u8 = 2;

/// One fixed client-to-game party-skill record.
///
/// `skill_index` is intentionally not restricted to [`PARTY_SKILL_HEAL`] or
/// [`PARTY_SKILL_WARP`]. The legacy handler's switch has no default arm, so an
/// unknown index is a source-level no-op rather than a wire-format error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgPartyUseSkill {
    /// One-byte `bySkillIndex` value.
    pub skill_index: u8,
    /// Target `DWORD` VID in explicit active-x86 little-endian order.
    pub target_vid: u32,
}

impl CgPartyUseSkill {
    /// Construct a record without interpreting its skill index.
    #[must_use]
    pub const fn new(skill_index: u8, target_vid: u32) -> Self {
        Self {
            skill_index,
            target_vid,
        }
    }

    /// Encode the exact six-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CG_PARTY_USE_SKILL_WIRE_SIZE);
        bytes.push(HEADER_CG_PARTY_USE_SKILL.value());
        bytes.push(self.skill_index);
        bytes.extend_from_slice(&self.target_vid.to_le_bytes());
        bytes
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        ClientFrame::new(
            HEADER_CG_PARTY_USE_SKILL.value(),
            [
                self.skill_index,
                self.target_vid.to_le_bytes()[0],
                self.target_vid.to_le_bytes()[1],
                self.target_vid.to_le_bytes()[2],
                self.target_vid.to_le_bytes()[3],
            ],
        )
    }

    /// Decode one exact complete six-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgPartyUseSkillError::Truncated`] for fewer than six bytes,
    /// [`CgPartyUseSkillError::LengthMismatch`] for more than six bytes, and
    /// [`CgPartyUseSkillError::InvalidHeader`] for a complete record with a
    /// header other than `0x4c`.
    pub fn decode(data: &[u8]) -> Result<Self, CgPartyUseSkillError> {
        check_exact(data)?;
        Self::decode_parts(data[0], &data[1..])
    }

    /// Decode a [`ClientFrame`] whose payload excludes the header byte.
    ///
    /// # Errors
    ///
    /// Returns a length error when the frame payload is not exactly five
    /// bytes, or [`CgPartyUseSkillError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgPartyUseSkillError> {
        if frame.payload.len() != CG_PARTY_USE_SKILL_WIRE_SIZE - 1 {
            let available = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
            return if frame.payload.len() < CG_PARTY_USE_SKILL_WIRE_SIZE - 1 {
                Err(CgPartyUseSkillError::Truncated {
                    needed: CG_PARTY_USE_SKILL_WIRE_SIZE,
                    available,
                })
            } else {
                Err(CgPartyUseSkillError::LengthMismatch {
                    expected: CG_PARTY_USE_SKILL_WIRE_SIZE,
                    actual: available,
                })
            };
        }
        Self::decode_parts(frame.header, &frame.payload)
    }

    fn decode_parts(header: u8, payload: &[u8]) -> Result<Self, CgPartyUseSkillError> {
        if header != HEADER_CG_PARTY_USE_SKILL.value() {
            return Err(CgPartyUseSkillError::InvalidHeader { actual: header });
        }
        // `decode` and `decode_frame` validate the exact five-byte payload
        // length before reaching this private parser.
        let skill_index = payload[0];
        let target_vid = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
        Ok(Self::new(skill_index, target_vid))
    }
}

/// A malformed fixed party-use-skill record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgPartyUseSkillError {
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
    /// The record used a header other than `HEADER_CG_PARTY_USE_SKILL`.
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for CgPartyUseSkillError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "party-use-skill record is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "party-use-skill record has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => write!(
                formatter,
                "expected party-use-skill header 0x4c, got 0x{actual:02x}"
            ),
        }
    }
}

impl Error for CgPartyUseSkillError {}

fn check_exact(data: &[u8]) -> Result<(), CgPartyUseSkillError> {
    match data.len().cmp(&CG_PARTY_USE_SKILL_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(CgPartyUseSkillError::Truncated {
            needed: CG_PARTY_USE_SKILL_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(CgPartyUseSkillError::LengthMismatch {
            expected: CG_PARTY_USE_SKILL_WIRE_SIZE,
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
    fn golden_bytes_use_header_skill_and_little_endian_vid() {
        let packet = CgPartyUseSkill::new(PARTY_SKILL_HEAL, 0x1234_5678);
        assert_eq!(packet.encode(), vec![0x4c, 0x01, 0x78, 0x56, 0x34, 0x12]);
        assert_eq!(CgPartyUseSkill::decode(&packet.encode()).unwrap(), packet);
    }

    #[test]
    fn skill_extremes_unknown_indices_and_vid_extremes_round_trip() {
        for packet in [
            CgPartyUseSkill::new(0, 0),
            CgPartyUseSkill::new(PARTY_SKILL_WARP, u32::MAX),
            CgPartyUseSkill::new(u8::MAX, 1),
        ] {
            assert_eq!(CgPartyUseSkill::decode(&packet.encode()).unwrap(), packet);
            assert_eq!(
                CgPartyUseSkill::decode_frame(&packet.to_frame()).unwrap(),
                packet
            );
        }
    }

    #[test]
    fn every_truncated_length_is_rejected() {
        let complete = CgPartyUseSkill::new(1, 2).encode();
        for available in 0..CG_PARTY_USE_SKILL_WIRE_SIZE {
            assert_eq!(
                CgPartyUseSkill::decode(&complete[..available]),
                Err(CgPartyUseSkillError::Truncated {
                    needed: CG_PARTY_USE_SKILL_WIRE_SIZE,
                    available,
                })
            );
        }
    }

    #[test]
    fn trailing_bytes_and_wrong_headers_are_rejected() {
        let mut long = CgPartyUseSkill::new(1, 2).encode();
        long.push(0);
        assert_eq!(
            CgPartyUseSkill::decode(&long),
            Err(CgPartyUseSkillError::LengthMismatch {
                expected: CG_PARTY_USE_SKILL_WIRE_SIZE,
                actual: 7,
            })
        );

        let mut wrong = vec![0x4d; CG_PARTY_USE_SKILL_WIRE_SIZE];
        wrong[0] = 0x4d;
        assert_eq!(
            CgPartyUseSkill::decode(&wrong),
            Err(CgPartyUseSkillError::InvalidHeader { actual: 0x4d })
        );
    }

    #[test]
    fn frame_payload_length_is_checked_without_consuming_extra_bytes() {
        let packet = CgPartyUseSkill::new(PARTY_SKILL_HEAL, 7);
        let frame = packet.to_frame();
        assert_eq!(CgPartyUseSkill::decode_frame(&frame).unwrap(), packet);

        let short = ClientFrame::new(0x4c, [1, 0, 0, 0]);
        assert_eq!(
            CgPartyUseSkill::decode_frame(&short),
            Err(CgPartyUseSkillError::Truncated {
                needed: CG_PARTY_USE_SKILL_WIRE_SIZE,
                available: 5,
            })
        );

        let long = ClientFrame::new(0x4c, [1, 0, 0, 0, 0, 0]);
        assert_eq!(
            CgPartyUseSkill::decode_frame(&long),
            Err(CgPartyUseSkillError::LengthMismatch {
                expected: CG_PARTY_USE_SKILL_WIRE_SIZE,
                actual: 7,
            })
        );

        let wrong = ClientFrame::new(0x4d, [1, 0, 0, 0, 0]);
        assert_eq!(
            CgPartyUseSkill::decode_frame(&wrong),
            Err(CgPartyUseSkillError::InvalidHeader { actual: 0x4d })
        );
    }

    #[test]
    fn fixed_decoder_handles_fragmented_and_coalesced_party_skill_frames() {
        let first = CgPartyUseSkill::new(PARTY_SKILL_WARP, 0x0102_0304).encode();
        let second = CgPartyUseSkill::new(PARTY_SKILL_HEAL, 0xaabb_ccdd).encode();
        let mut decoder = ClientFrameDecoder::new();

        decoder.feed(&first[..2]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&first[2..]).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgPartyUseSkill::decode_frame(&frame).unwrap(),
            CgPartyUseSkill::new(PARTY_SKILL_WARP, 0x0102_0304)
        );

        let mut coalesced = second;
        coalesced.push(0x00);
        decoder.feed(&coalesced).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgPartyUseSkill::decode_frame(&frame).unwrap(),
            CgPartyUseSkill::new(PARTY_SKILL_HEAL, 0xaabb_ccdd)
        );
        assert_eq!(decoder.try_decode().unwrap().unwrap().header, 0x00);
    }
}
