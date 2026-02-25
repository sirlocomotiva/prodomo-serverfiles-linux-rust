//! Explicit codec for the fixed legacy `TPacketCGUseSkill` record.
//!
//! `server/server/game/packet.h:43,1788-1793` defines
//! `HEADER_CG_USE_SKILL = 52` and a packed record containing one header byte
//! followed by a skill `DWORD dwVnum` and a target `DWORD dwVID`, for a
//! complete nine-byte packet. `packet_info.cpp:141` registers that exact size,
//! and the client sends the same shape at
//! `PythonNetworkStreamPhaseGame.cpp:1295-1310`.
//!
//! This module only validates and preserves the wire record. It does not
//! resolve a skill, look up a target, authorize a skill level, apply range or
//! player-versus-player rules, or invoke the legacy `CHARACTER::UseSkill`
//! gameplay path.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::HEADER_CG_USE_SKILL;
use crate::cg_wire::ClientFrame;

/// Complete packed wire size, including the one-byte header.
pub const CG_USE_SKILL_WIRE_SIZE: usize = 9;

/// Fixed client-to-game payload size, excluding the one-byte header.
pub const CG_USE_SKILL_PAYLOAD_SIZE: usize = 8;

/// One fixed client-to-game skill-use record.
///
/// Both source `DWORD` fields are preserved verbatim. The legacy handler
/// interprets `skill_vnum` through the skill table and resolves
/// `target_vid` through the character manager; neither lookup, nor any range
/// or authorization rule, belongs at this protocol boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgUseSkill {
    /// Opaque source `dwVnum` skill identifier.
    pub skill_vnum: u32,
    /// Opaque source `dwVID` target identifier.
    pub target_vid: u32,
}

impl CgUseSkill {
    /// Construct a record without interpreting either source field.
    #[must_use]
    pub const fn new(skill_vnum: u32, target_vid: u32) -> Self {
        Self {
            skill_vnum,
            target_vid,
        }
    }

    /// Encode the exact nine-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut record = Vec::with_capacity(CG_USE_SKILL_WIRE_SIZE);
        record.push(HEADER_CG_USE_SKILL.value());
        record.extend_from_slice(&self.skill_vnum.to_le_bytes());
        record.extend_from_slice(&self.target_vid.to_le_bytes());
        record
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        let mut payload = [0u8; CG_USE_SKILL_PAYLOAD_SIZE];
        payload[..4].copy_from_slice(&self.skill_vnum.to_le_bytes());
        payload[4..].copy_from_slice(&self.target_vid.to_le_bytes());
        ClientFrame::new(HEADER_CG_USE_SKILL.value(), payload)
    }

    /// Decode one exact complete skill-use record.
    ///
    /// The exact length is checked before any field is read and before the
    /// header is validated.
    ///
    /// # Errors
    ///
    /// Returns [`CgUseSkillError::Truncated`] for fewer than nine bytes,
    /// [`CgUseSkillError::LengthMismatch`] for more than nine bytes, and
    /// [`CgUseSkillError::InvalidHeader`] for another header.
    pub fn decode(data: &[u8]) -> Result<Self, CgUseSkillError> {
        check_exact(data)?;
        decode_parts(
            data[0],
            u32::from_le_bytes([data[1], data[2], data[3], data[4]]),
            u32::from_le_bytes([data[5], data[6], data[7], data[8]]),
        )
    }

    /// Decode a [`ClientFrame`] whose payload excludes the header byte.
    ///
    /// # Errors
    ///
    /// Returns a length error when the payload is not exactly eight bytes, or
    /// [`CgUseSkillError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgUseSkillError> {
        match frame.payload.len().cmp(&CG_USE_SKILL_PAYLOAD_SIZE) {
            std::cmp::Ordering::Less => {
                let available = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
                Err(CgUseSkillError::Truncated {
                    needed: CG_USE_SKILL_WIRE_SIZE,
                    available,
                })
            }
            std::cmp::Ordering::Greater => {
                let actual = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
                Err(CgUseSkillError::LengthMismatch {
                    expected: CG_USE_SKILL_WIRE_SIZE,
                    actual,
                })
            }
            std::cmp::Ordering::Equal => {
                let bytes = frame.payload.as_slice();
                decode_parts(
                    frame.header,
                    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                    u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
                )
            }
        }
    }
}

fn decode_parts(
    header: u8,
    skill_vnum: u32,
    target_vid: u32,
) -> Result<CgUseSkill, CgUseSkillError> {
    if header != HEADER_CG_USE_SKILL.value() {
        return Err(CgUseSkillError::InvalidHeader { actual: header });
    }
    Ok(CgUseSkill::new(skill_vnum, target_vid))
}

/// A malformed fixed skill-use record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgUseSkillError {
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
    /// The record used a header other than `52`.
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for CgUseSkillError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "skill-use record is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "skill-use record has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => {
                write!(formatter, "expected skill-use header 52, got {actual}")
            }
        }
    }
}

impl Error for CgUseSkillError {}

fn check_exact(data: &[u8]) -> Result<(), CgUseSkillError> {
    match data.len().cmp(&CG_USE_SKILL_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(CgUseSkillError::Truncated {
            needed: CG_USE_SKILL_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(CgUseSkillError::LengthMismatch {
            expected: CG_USE_SKILL_WIRE_SIZE,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_account::CgEnterGame;
    use crate::cg_inventory::HEADER_CG_USE_SKILL as USE_SKILL_HEADER;
    use crate::cg_shoot::CgShoot;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};

    #[test]
    fn inventory_resolves_the_fixed_nine_byte_size() {
        assert_eq!(USE_SKILL_HEADER.value(), 52);
        assert_eq!(CG_USE_SKILL_WIRE_SIZE, 9);
        assert_eq!(CG_USE_SKILL_PAYLOAD_SIZE, 8);
        assert_eq!(
            resolve_client_frame_size(52).unwrap(),
            ClientFrameSize::Fixed(9)
        );
    }

    #[test]
    fn golden_record_is_header_plus_two_explicit_little_endian_words() {
        // Header 52, dwVnum 0x0001_0203 at offsets 1..5, dwVID 0x0405_0607 at 5..9.
        let packet = CgUseSkill::new(0x0001_0203, 0x0405_0607);
        assert_eq!(
            packet.encode(),
            vec![0x34, 0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x05, 0x04]
        );
        assert_eq!(CgUseSkill::decode(&packet.encode()).unwrap(), packet);
        let frame = packet.to_frame();
        assert_eq!(frame.header, 0x34);
        assert_eq!(frame.payload.len(), 8);
        assert_eq!(
            frame.payload,
            vec![0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x05, 0x04]
        );
        assert_eq!(CgUseSkill::decode_frame(&frame).unwrap(), packet);
    }

    #[test]
    fn every_u32_value_round_trips_including_client_default_and_extremes() {
        // Zero is the client's documented default target and a valid value for
        // both words. No skill table or character lookup exists at this layer.
        let values = [0, 1, 52, 0x0000_00ff, 0x0000_ffff, u32::MAX - 1, u32::MAX];
        for skill_vnum in values {
            for target_vid in values {
                let packet = CgUseSkill::new(skill_vnum, target_vid);
                assert_eq!(packet.encode().len(), CG_USE_SKILL_WIRE_SIZE);
                assert_eq!(CgUseSkill::decode(&packet.encode()).unwrap(), packet);
                assert_eq!(
                    CgUseSkill::decode_frame(&packet.to_frame()).unwrap(),
                    packet
                );
            }
        }
    }

    #[test]
    fn every_raw_short_and_long_length_is_rejected_before_the_header() {
        let complete = CgUseSkill::new(1, 2).encode();
        for len in 0..CG_USE_SKILL_WIRE_SIZE {
            let prefix = &complete[..len];
            assert_eq!(
                CgUseSkill::decode(prefix),
                Err(CgUseSkillError::Truncated {
                    needed: 9,
                    available: len,
                }),
                "raw length {len} must be truncated"
            );
        }
        for extra in [1usize, 2, 3, 64, 255] {
            let mut long = complete.clone();
            long.extend(std::iter::repeat(0u8).take(extra));
            assert_eq!(
                CgUseSkill::decode(&long),
                Err(CgUseSkillError::LengthMismatch {
                    expected: 9,
                    actual: 9 + extra,
                }),
                "raw length {} must mismatch",
                9 + extra
            );
        }
        // A wrong header at an exact length is a header error, but every wrong
        // header at a wrong length is still a length error.
        for header in 0u8..=u8::MAX {
            let mut record = complete.clone();
            record[0] = header;
            let expected = if header == 52 {
                Ok(CgUseSkill::new(1, 2))
            } else {
                Err(CgUseSkillError::InvalidHeader { actual: header })
            };
            assert_eq!(CgUseSkill::decode(&record), expected, "header {header}");
        }
        assert_eq!(
            CgUseSkill::decode(&[0x35, 0, 0, 0, 0, 0, 0, 0]),
            Err(CgUseSkillError::Truncated {
                needed: 9,
                available: 8,
            })
        );
    }

    #[test]
    fn every_framed_payload_length_and_header_is_checked() {
        let packet = CgUseSkill::new(0x1122_3344, 0x5566_7788);
        assert_eq!(
            CgUseSkill::decode_frame(&packet.to_frame()).unwrap(),
            packet
        );
        for len in 0..CG_USE_SKILL_PAYLOAD_SIZE {
            for header in [0x34u8, 0x35, 0x00, 0xff] {
                let frame = ClientFrame::new(header, vec![0xabu8; len]);
                let expected = if len < CG_USE_SKILL_PAYLOAD_SIZE {
                    CgUseSkillError::Truncated {
                        needed: 9,
                        available: len + 1,
                    }
                } else if header != 0x34 {
                    CgUseSkillError::InvalidHeader { actual: header }
                } else {
                    panic!("unreachable");
                };
                assert_eq!(
                    CgUseSkill::decode_frame(&frame),
                    Err(expected),
                    "payload {len} header {header}"
                );
            }
        }
        for extra in [1usize, 2, 3, 64, 255] {
            let mut payload = packet.to_frame().payload;
            payload.extend(std::iter::repeat(0u8).take(extra));
            assert_eq!(
                CgUseSkill::decode_frame(&ClientFrame::new(0x34, payload)),
                Err(CgUseSkillError::LengthMismatch {
                    expected: 9,
                    actual: 9 + extra,
                })
            );
        }
        for header in 0u8..=u8::MAX {
            if header == 52 {
                continue;
            }
            let frame = ClientFrame::new(header, packet.to_frame().payload);
            assert_eq!(
                CgUseSkill::decode_frame(&frame),
                Err(CgUseSkillError::InvalidHeader { actual: header }),
                "framed header {header}"
            );
        }
    }

    #[test]
    fn fragmented_and_coalesced_streaming_uses_the_shared_decoder() {
        let raw = CgUseSkill::new(0, 0).encode();
        let mut decoder = ClientFrameDecoder::new();
        decoder.feed(&raw[..1]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&raw[1..4]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&raw[4..8]).unwrap();
        assert_eq!(decoder.buffered_len(), 8);
        assert!(decoder.try_decode().unwrap().is_none());

        let mut coalesced = raw[8..].to_vec();
        coalesced.extend_from_slice(&CgUseSkill::new(u32::MAX, 0x8000_0000).encode());
        coalesced.extend_from_slice(&CgShoot::new(0x07).encode());
        coalesced.push(CgEnterGame::new().encode()[0]);
        decoder.feed(&coalesced).unwrap();

        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgUseSkill::decode_frame(&frame).unwrap(),
            CgUseSkill::new(0, 0)
        );
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgUseSkill::decode_frame(&frame).unwrap(),
            CgUseSkill::new(u32::MAX, 0x8000_0000)
        );
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgShoot::decode_frame(&frame).unwrap(), CgShoot::new(0x07));
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgEnterGame::decode_frame(&frame).unwrap(), CgEnterGame);
        assert!(decoder.is_empty());
    }
}
