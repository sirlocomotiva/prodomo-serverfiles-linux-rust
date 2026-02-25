//! Explicit codec for the fixed legacy `TPacketCGAttack` record.
//!
//! `server/server/game/packet.h:13,531-538` defines `HEADER_CG_ATTACK = 2`
//! and a packed record containing one header byte, one opaque `BYTE bType`,
//! a little-endian `DWORD dwVID`, and two magic-cube CRC piece bytes.
//! `packet_info.cpp:110` registers the eight-byte record, and the checked-in
//! CG inventory already resolves header 2 as a fixed eight-byte frame.
//! The matching packed client record is `client/Client/UserInterface/Packet.h:461-468`,
//! where the same DWORD is named `dwVictimVID`, and
//! `client/Client/UserInterface/PythonNetworkStreamPhaseGame.cpp:3052-3077,3079-3093`
//! fills all eight client bytes: the caller supplies the type and victim VID,
//! and `SendSpecial` assigns both magic-cube pieces before sending.
//!
//! This module only validates and preserves the wire record. It does not
//! interpret `bType` as a skill, look up the target character, reject self or
//! NPC targets, assemble the magic-cube CRC, apply hit-rate limits, or invoke
//! the legacy melee `Attack` gameplay handler at `input_main.cpp:1966-2009`.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::HEADER_CG_ATTACK;
use crate::cg_wire::ClientFrame;

/// Complete packed wire size, including the one-byte header.
pub const CG_ATTACK_WIRE_SIZE: usize = 8;

/// Frame payload size, excluding the one-byte header.
pub const CG_ATTACK_PAYLOAD_SIZE: usize = CG_ATTACK_WIRE_SIZE - 1;

/// One fixed client-to-game melee attack record.
///
/// Every field stays opaque at this boundary. `target_vid` is a wire DWORD
/// and not a validated character reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgAttack {
    /// Opaque source `bType` value.
    pub attack_type: u8,
    /// Source `dwVID` value, encoded little-endian.
    pub target_vid: u32,
    /// Opaque source `bCRCMagicCubeProcPiece` value.
    pub magic_cube_proc_piece: u8,
    /// Opaque source `bCRCMagicCubeFilePiece` value.
    pub magic_cube_file_piece: u8,
}

impl CgAttack {
    /// Construct a record without applying gameplay or admission policy.
    #[must_use]
    pub const fn new(
        attack_type: u8,
        target_vid: u32,
        magic_cube_proc_piece: u8,
        magic_cube_file_piece: u8,
    ) -> Self {
        Self {
            attack_type,
            target_vid,
            magic_cube_proc_piece,
            magic_cube_file_piece,
        }
    }

    /// Encode the exact eight-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CG_ATTACK_WIRE_SIZE);
        bytes.push(HEADER_CG_ATTACK.value());
        bytes.push(self.attack_type);
        bytes.extend_from_slice(&self.target_vid.to_le_bytes());
        bytes.push(self.magic_cube_proc_piece);
        bytes.push(self.magic_cube_file_piece);
        bytes
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_ATTACK_PAYLOAD_SIZE);
        payload.push(self.attack_type);
        payload.extend_from_slice(&self.target_vid.to_le_bytes());
        payload.push(self.magic_cube_proc_piece);
        payload.push(self.magic_cube_file_piece);
        ClientFrame::new(HEADER_CG_ATTACK.value(), payload)
    }

    /// Decode one exact complete attack record.
    ///
    /// # Errors
    ///
    /// Returns [`CgAttackError::Truncated`] for fewer than eight bytes,
    /// [`CgAttackError::LengthMismatch`] for more than eight bytes, and
    /// [`CgAttackError::InvalidHeader`] for another header.
    pub fn decode(data: &[u8]) -> Result<Self, CgAttackError> {
        check_exact(data)?;
        decode_parts(data[0], &data[1..])
    }

    /// Decode a [`ClientFrame`] whose payload excludes the one-byte header.
    ///
    /// # Errors
    ///
    /// Returns a length error when the frame payload is not exactly seven
    /// bytes, or [`CgAttackError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgAttackError> {
        if frame.payload.len() != CG_ATTACK_PAYLOAD_SIZE {
            let actual = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
            return if frame.payload.len() < CG_ATTACK_PAYLOAD_SIZE {
                Err(CgAttackError::Truncated {
                    needed: CG_ATTACK_WIRE_SIZE,
                    available: actual,
                })
            } else {
                Err(CgAttackError::LengthMismatch {
                    expected: CG_ATTACK_WIRE_SIZE,
                    actual,
                })
            };
        }
        decode_parts(frame.header, &frame.payload)
    }
}

fn decode_parts(header: u8, payload: &[u8]) -> Result<CgAttack, CgAttackError> {
    if header != HEADER_CG_ATTACK.value() {
        return Err(CgAttackError::InvalidHeader { actual: header });
    }
    debug_assert_eq!(payload.len(), CG_ATTACK_PAYLOAD_SIZE);
    let target_vid = u32::from_le_bytes(
        payload[1..5]
            .try_into()
            .expect("length checked before target VID copy"),
    );
    Ok(CgAttack::new(
        payload[0], target_vid, payload[5], payload[6],
    ))
}

/// A malformed fixed attack record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgAttackError {
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
    /// The record used a header other than `0x02`.
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for CgAttackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "attack record is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "attack record has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => {
                write!(formatter, "expected attack header 0x02, got 0x{actual:02x}")
            }
        }
    }
}

impl Error for CgAttackError {}

fn check_exact(data: &[u8]) -> Result<(), CgAttackError> {
    match data.len().cmp(&CG_ATTACK_WIRE_SIZE) {
        std::cmp::Ordering::Less => Err(CgAttackError::Truncated {
            needed: CG_ATTACK_WIRE_SIZE,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(CgAttackError::LengthMismatch {
            expected: CG_ATTACK_WIRE_SIZE,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_account::CgEnterGame;
    use crate::cg_shoot::CgShoot;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};

    fn sample() -> CgAttack {
        CgAttack::new(0x0b, 0x0001_2345, 0x21, 0xfe)
    }

    #[test]
    fn source_inventory_resolves_header_two_as_a_fixed_eight_byte_frame() {
        assert_eq!(HEADER_CG_ATTACK.value(), 0x02);
        assert_eq!(CG_ATTACK_WIRE_SIZE, 8);
        assert_eq!(CG_ATTACK_PAYLOAD_SIZE, 7);
        assert_eq!(
            resolve_client_frame_size(HEADER_CG_ATTACK.value()).unwrap(),
            ClientFrameSize::Fixed(CG_ATTACK_WIRE_SIZE)
        );
    }

    #[test]
    fn golden_record_has_exact_source_offsets_and_little_endian_vid() {
        let golden = [0x02, 0x0b, 0x45, 0x23, 0x01, 0x00, 0x21, 0xfe];
        assert_eq!(golden.len(), CG_ATTACK_WIRE_SIZE);
        assert_eq!(&golden[..2], &[0x02, 0x0b]);
        assert_eq!(&golden[2..6], &0x0001_2345_u32.to_le_bytes());
        assert_eq!(&golden[6..], &[0x21, 0xfe]);

        let packet = CgAttack::decode(&golden).unwrap();
        assert_eq!(packet, sample());
        assert_eq!(packet.encode(), golden);

        let frame = packet.to_frame();
        assert_eq!(frame.header, HEADER_CG_ATTACK.value());
        assert_eq!(frame.payload, golden[1..].to_vec());
        assert_eq!(CgAttack::decode_frame(&frame).unwrap(), packet);
    }

    #[test]
    fn all_type_values_vid_extrema_and_piece_extrema_round_trip() {
        for attack_type in u8::MIN..=u8::MAX {
            for target_vid in [0_u32, 1, 0x0001_2345, 0x8000_0000, u32::MAX] {
                let packet = CgAttack::new(attack_type, target_vid, 0x00, 0xff);
                let raw = packet.encode();
                assert_eq!(raw.len(), CG_ATTACK_WIRE_SIZE);
                assert_eq!(&raw[..2], &[0x02, attack_type]);
                assert_eq!(&raw[2..6], &target_vid.to_le_bytes());
                assert_eq!(&raw[6..], &[0x00, 0xff]);
                assert_eq!(CgAttack::decode(&raw).unwrap(), packet);
                assert_eq!(CgAttack::decode_frame(&packet.to_frame()).unwrap(), packet);
            }
        }

        for (proc_piece, file_piece) in [
            (0_u8, 0_u8),
            (0xff, 0xff),
            (0x80, 0x01),
            (0x01, 0x80),
            (0x7f, 0xfe),
        ] {
            let packet = CgAttack::new(0, 0, proc_piece, file_piece);
            assert_eq!(&packet.encode()[6..], &[proc_piece, file_piece]);
            assert_eq!(CgAttack::decode(&packet.encode()).unwrap(), packet);
        }
    }

    #[test]
    fn every_short_and_long_raw_length_is_rejected() {
        let complete = sample().encode();
        for available in 0..CG_ATTACK_WIRE_SIZE {
            assert_eq!(
                CgAttack::decode(&complete[..available]),
                Err(CgAttackError::Truncated {
                    needed: CG_ATTACK_WIRE_SIZE,
                    available,
                })
            );
        }
        for actual in [CG_ATTACK_WIRE_SIZE + 1, CG_ATTACK_WIRE_SIZE + 7] {
            assert_eq!(
                CgAttack::decode(&vec![0x02; actual]),
                Err(CgAttackError::LengthMismatch {
                    expected: CG_ATTACK_WIRE_SIZE,
                    actual,
                })
            );
        }
    }

    #[test]
    fn every_wrong_header_and_length_precedence_are_checked_before_fields() {
        for header in u8::MIN..=u8::MAX {
            if header == HEADER_CG_ATTACK.value() {
                continue;
            }
            let mut raw = sample().encode();
            raw[0] = header;
            assert_eq!(
                CgAttack::decode(&raw),
                Err(CgAttackError::InvalidHeader { actual: header }),
                "exact raw record with header {header:#04x}"
            );
            assert_eq!(
                CgAttack::decode_frame(&ClientFrame::new(header, vec![0; CG_ATTACK_PAYLOAD_SIZE],)),
                Err(CgAttackError::InvalidHeader { actual: header }),
                "exact frame with header {header:#04x}"
            );
        }

        for header in [0x00_u8, 0x01, 0x03, 0xff] {
            for available in 0..CG_ATTACK_WIRE_SIZE {
                assert_eq!(
                    CgAttack::decode(&vec![header; available]),
                    Err(CgAttackError::Truncated {
                        needed: CG_ATTACK_WIRE_SIZE,
                        available,
                    }),
                    "short raw record {available} with header {header:#04x}"
                );
            }
            for actual in [CG_ATTACK_WIRE_SIZE + 1, CG_ATTACK_WIRE_SIZE + 7] {
                assert_eq!(
                    CgAttack::decode(&vec![header; actual]),
                    Err(CgAttackError::LengthMismatch {
                        expected: CG_ATTACK_WIRE_SIZE,
                        actual,
                    }),
                    "long raw record {actual} with header {header:#04x}"
                );
            }
        }
    }

    #[test]
    fn every_frame_payload_length_and_wrong_header_is_checked() {
        let packet = sample();
        let frame = packet.to_frame();
        assert_eq!(frame.payload.len(), CG_ATTACK_PAYLOAD_SIZE);
        assert_eq!(CgAttack::decode_frame(&frame).unwrap(), packet);

        for payload_len in 0..CG_ATTACK_PAYLOAD_SIZE {
            assert_eq!(
                CgAttack::decode_frame(&ClientFrame::new(0x02, vec![0; payload_len])),
                Err(CgAttackError::Truncated {
                    needed: CG_ATTACK_WIRE_SIZE,
                    available: payload_len + 1,
                }),
                "short frame payload {payload_len}"
            );
            assert_eq!(
                CgAttack::decode_frame(&ClientFrame::new(0x03, vec![0; payload_len])),
                Err(CgAttackError::Truncated {
                    needed: CG_ATTACK_WIRE_SIZE,
                    available: payload_len + 1,
                }),
                "short frame payload {payload_len} with wrong header"
            );
        }
        for actual in [CG_ATTACK_WIRE_SIZE + 1, CG_ATTACK_WIRE_SIZE + 7] {
            assert_eq!(
                CgAttack::decode_frame(&ClientFrame::new(0x02, vec![0; actual - 1])),
                Err(CgAttackError::LengthMismatch {
                    expected: CG_ATTACK_WIRE_SIZE,
                    actual,
                }),
                "long frame payload {actual}"
            );
        }
    }

    #[test]
    fn fixed_decoder_handles_fragmented_and_coalesced_attack_frames() {
        let raw = sample().encode();
        let mut decoder = ClientFrameDecoder::new();
        decoder.feed(&raw[..1]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&raw[1..4]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());

        let mut coalesced = raw[4..].to_vec();
        coalesced.extend_from_slice(&CgShoot::new(0x07).encode());
        coalesced.push(CgEnterGame::new().encode()[0]);
        decoder.feed(&coalesced).unwrap();

        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgAttack::decode_frame(&frame).unwrap(), sample());
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgShoot::decode_frame(&frame).unwrap(), CgShoot::new(0x07));
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgEnterGame::decode_frame(&frame).unwrap(), CgEnterGame);
    }
}
