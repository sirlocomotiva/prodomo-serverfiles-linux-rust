//! The 14-byte `AnswerMakeGuild` record: a header and a raw 13-byte guild name.
//!
//! ```c
//! typedef struct command_guild_answer_make_guild
//! {
//!     BYTE header;
//!     char guild_name[GUILD_NAME_MAX_LEN+1];
//! } TPacketCGAnswerMakeGuild;
//! ```
//!
//! # Width
//!
//! `GUILD_NAME_MAX_LEN` is `12` at `server/server/common/length.h:42`, so the
//! field is `12 + 1 = 13` and the record is `1 + 13 = 14`, which closes against
//! `Set(HEADER_CG_ANSWER_MAKE_GUILD, sizeof(TPacketCGAnswerMakeGuild), "AnswerMakeGuild")`.
//! The `+ 1` is the legacy room for a NUL terminator.
//!
//! # The name is raw storage, not text
//!
//! The field is `char[13]`, and the codec keeps it as `[u8; 13]`:
//!
//! - The client writes the terminator, at
//!   `client/Client/UserInterface/PythonNetworkStreamPhaseGame.cpp` in
//!   `SendAnswerMakeGuild`.
//! - The server does not require one. `CInputMain::AnswerMakeGuild` hands the
//!   whole record to `CGuildManager::Instance().Add` after its own checks, and
//!   nothing on the wire path establishes that byte 12 is NUL.
//!
//! So a NUL rule would **reject records the legacy server accepts**. There is
//! deliberately no `&str`, no `CStr`, and no `to_str` convenience. A caller that
//! wants text converts explicitly and handles the no-terminator case itself.
//! This is the same rule already applied to the sash, guild, and quest-text
//! fixed `char` fields.
//!
//! `GUILD_NAME_MAX_LEN` is duplicated here as a local constant rather than
//! imported. The `protocol` crate deliberately does **not** depend on `common`,
//! so the width is stated at the point of use with its source line, the same way
//! every other legacy length in this crate is stated.

use crate::cg_inventory::{CgHeader, HEADER_CG_ANSWER_MAKE_GUILD};
use crate::cg_wire::ClientFrame;

/// The legacy `GUILD_NAME_MAX_LEN`, from `server/server/common/length.h:42`.
pub const GUILD_NAME_MAX_LEN: usize = 12;

/// The legacy `char guild_name[GUILD_NAME_MAX_LEN + 1]` field width.
pub const GUILD_NAME_FIELD_SIZE: usize = GUILD_NAME_MAX_LEN + 1;

/// The full legacy `TPacketCGAnswerMakeGuild` record, header byte included.
pub const CG_ANSWER_MAKE_GUILD_WIRE_SIZE: usize = 1 + GUILD_NAME_FIELD_SIZE;
/// The framed payload of `TPacketCGAnswerMakeGuild`.
pub const CG_ANSWER_MAKE_GUILD_PAYLOAD_SIZE: usize = GUILD_NAME_FIELD_SIZE;

/// Every way the make-guild decoder can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgAnswerMakeGuildError {
    /// Fewer bytes than the fixed record needs.
    Truncated {
        /// The fixed width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// A complete-length slice that is not the fixed width.
    LengthMismatch {
        /// The fixed width the decoder requires.
        expected: usize,
        /// How many bytes were actually offered.
        actual: usize,
    },
    /// The right number of bytes, but the header byte is not this record's.
    InvalidHeader {
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was actually present.
        actual: u8,
    },
}

/// The 14-byte `AnswerMakeGuild` record: a header and 13 raw name bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgAnswerMakeGuild {
    /// The legacy `char guild_name[13]`, as raw bytes.
    pub guild_name: [u8; GUILD_NAME_FIELD_SIZE],
}

impl CgAnswerMakeGuild {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_ANSWER_MAKE_GUILD
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_ANSWER_MAKE_GUILD_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_ANSWER_MAKE_GUILD_PAYLOAD_SIZE;

    /// Build the record from its raw name bytes.
    pub const fn new(guild_name: [u8; GUILD_NAME_FIELD_SIZE]) -> Self {
        Self { guild_name }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.extend_from_slice(&self.guild_name);
    }

    /// Encode to a fresh 14-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 13-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(Self::PAYLOAD_SIZE);
        payload.extend_from_slice(&self.guild_name);
        ClientFrame {
            header: Self::header().value(),
            payload,
        }
    }

    /// # Errors
    ///
    /// [`CgAnswerMakeGuildError::Truncated`] below 14 bytes,
    /// [`CgAnswerMakeGuildError::LengthMismatch`] above,
    /// [`CgAnswerMakeGuildError::InvalidHeader`] for a full-length slice not
    /// starting with 81.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgAnswerMakeGuildError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(CgAnswerMakeGuildError::Truncated {
                needed: Self::WIRE_SIZE,
                available: bytes.len(),
            });
        }
        if bytes.len() > Self::WIRE_SIZE {
            return Err(CgAnswerMakeGuildError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header().value() {
            return Err(CgAnswerMakeGuildError::InvalidHeader {
                expected: Self::header().value(),
                actual: bytes[0],
            });
        }
        let mut guild_name = [0u8; GUILD_NAME_FIELD_SIZE];
        guild_name.copy_from_slice(&bytes[1..]);
        Ok(Self { guild_name })
    }

    /// # Errors
    ///
    /// As [`CgAnswerMakeGuild::decode`], except that the payload must be
    /// exactly 13 bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgAnswerMakeGuildError> {
        if frame.payload.len() < Self::PAYLOAD_SIZE {
            return Err(CgAnswerMakeGuildError::Truncated {
                needed: Self::PAYLOAD_SIZE,
                available: frame.payload.len(),
            });
        }
        if frame.payload.len() > Self::PAYLOAD_SIZE {
            return Err(CgAnswerMakeGuildError::LengthMismatch {
                expected: Self::PAYLOAD_SIZE,
                actual: frame.payload.len(),
            });
        }
        if frame.header != Self::header().value() {
            return Err(CgAnswerMakeGuildError::InvalidHeader {
                expected: Self::header().value(),
                actual: frame.header,
            });
        }
        let mut guild_name = [0u8; GUILD_NAME_FIELD_SIZE];
        guild_name.copy_from_slice(&frame.payload);
        Ok(Self { guild_name })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u8 = 81;

    /// A 12-character name with its terminator, as the client writes it.
    const SAMPLE: [u8; 13] = *b"Warriors\0\0\0\0\0";

    #[test]
    fn the_header_is_81() {
        assert_eq!(CgAnswerMakeGuild::header().value(), H);
        assert_eq!(H, 0x51);
    }

    #[test]
    fn the_name_field_is_13_bytes() {
        assert_eq!(GUILD_NAME_MAX_LEN, 12);
        assert_eq!(GUILD_NAME_FIELD_SIZE, 13);
        assert_eq!(GUILD_NAME_MAX_LEN + 1, GUILD_NAME_FIELD_SIZE);
    }

    #[test]
    fn the_record_is_fourteen_bytes() {
        assert_eq!(CG_ANSWER_MAKE_GUILD_WIRE_SIZE, 14);
        assert_eq!(CgAnswerMakeGuild::WIRE_SIZE, 14);
        assert_eq!(CgAnswerMakeGuild::PAYLOAD_SIZE, 13);
    }

    #[test]
    fn a_full_twelve_character_name_uses_the_whole_field() {
        let mut name = [0u8; 13];
        name[..12].copy_from_slice(b"ABCDEFGHIJKL");
        let r = CgAnswerMakeGuild::new(name);
        assert_eq!(r.encode().len(), 14);
        // 12 name bytes plus the one NUL the client writes.
        assert_eq!(&r.encode()[1..13], b"ABCDEFGHIJKL");
        assert_eq!(&r.encode()[13..], &[0]);
    }

    #[test]
    fn encodes_to_the_exact_legacy_bytes() {
        let r = CgAnswerMakeGuild::new(SAMPLE);
        let mut want = vec![H];
        want.extend_from_slice(&SAMPLE);
        assert_eq!(r.encode(), want);
    }

    #[test]
    fn every_name_byte_round_trips() {
        for i in 0..13 {
            let mut name = SAMPLE;
            name[i] = 0xFF;
            let r = CgAnswerMakeGuild::new(name);
            assert_eq!(
                CgAnswerMakeGuild::decode(&r.encode()).unwrap().guild_name,
                name,
                "byte {i}"
            );
        }
    }

    #[test]
    fn all_256_byte_values_are_accepted_in_every_position() {
        // No byte value is reserved and none is rejected.
        for v in 0..=255_u8 {
            for i in [0_usize, 6, 12] {
                let mut name = [0u8; 13];
                name[i] = v;
                let r = CgAnswerMakeGuild::new(name);
                assert_eq!(
                    CgAnswerMakeGuild::decode(&r.encode()).unwrap().guild_name,
                    name,
                    "value {v} at {i}"
                );
            }
        }
    }

    #[test]
    fn a_name_with_no_terminator_is_accepted() {
        // The legacy server does not require one, so the codec must not either.
        let name = [b'A'; 13];
        let r = CgAnswerMakeGuild::new(name);
        assert_eq!(CgAnswerMakeGuild::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn high_bytes_are_accepted() {
        let name = [0xFF; 13];
        let r = CgAnswerMakeGuild::new(name);
        assert_eq!(CgAnswerMakeGuild::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn round_trips() {
        let r = CgAnswerMakeGuild::new(SAMPLE);
        assert_eq!(CgAnswerMakeGuild::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn round_trips_through_a_frame() {
        let r = CgAnswerMakeGuild::new(SAMPLE);
        let f = r.to_frame();
        assert_eq!(f.header, H);
        assert_eq!(f.payload.len(), 13);
        assert_eq!(CgAnswerMakeGuild::decode_frame(&f).unwrap(), r);
        assert_eq!(&f.payload[..], &r.encode()[1..]);
    }

    #[test]
    fn the_header_cannot_come_from_the_name() {
        let f = ClientFrame {
            header: 0x01,
            payload: {
                let mut p = SAMPLE.to_vec();
                p[0] = H;
                p
            },
        };
        assert_eq!(
            CgAnswerMakeGuild::decode_frame(&f).unwrap_err(),
            CgAnswerMakeGuildError::InvalidHeader {
                expected: 81,
                actual: 0x01
            }
        );
    }

    #[test]
    fn rejects_every_short_length() {
        for len in 0..14 {
            let mut b = CgAnswerMakeGuild::new(SAMPLE).encode();
            b.truncate(len);
            assert_eq!(
                CgAnswerMakeGuild::decode(&b).unwrap_err(),
                CgAnswerMakeGuildError::Truncated {
                    needed: 14,
                    available: len
                },
                "len {len}"
            );
        }
    }

    #[test]
    fn rejects_every_long_length() {
        for extra in 1..=4 {
            let mut b = CgAnswerMakeGuild::new(SAMPLE).encode();
            b.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgAnswerMakeGuild::decode(&b).unwrap_err(),
                CgAnswerMakeGuildError::LengthMismatch {
                    expected: 14,
                    actual: 14 + extra
                }
            );
        }
    }

    #[test]
    fn rejects_every_wrong_header() {
        for v in 0..=255_u8 {
            if v == H {
                continue;
            }
            let mut b = CgAnswerMakeGuild::new(SAMPLE).encode();
            b[0] = v;
            assert_eq!(
                CgAnswerMakeGuild::decode(&b).unwrap_err(),
                CgAnswerMakeGuildError::InvalidHeader {
                    expected: 81,
                    actual: v
                },
                "header {v}"
            );
        }
    }

    #[test]
    fn rejects_every_wrong_frame_length() {
        for len in 0..13 {
            let f = ClientFrame {
                header: H,
                payload: vec![0; len],
            };
            assert!(CgAnswerMakeGuild::decode_frame(&f).is_err(), "len {len}");
        }
        let mut f = CgAnswerMakeGuild::new(SAMPLE).to_frame();
        f.payload.push(0);
        assert!(CgAnswerMakeGuild::decode_frame(&f).is_err());
    }

    #[test]
    fn distinct_names_stay_distinct() {
        let mut na = [0u8; 13];
        na[..3].copy_from_slice(b"aaa");
        let mut nb = [0u8; 13];
        nb[..3].copy_from_slice(b"aab");
        let a = CgAnswerMakeGuild::new(na);
        let b = CgAnswerMakeGuild::new(nb);
        assert_ne!(a, b);
        assert_eq!(CgAnswerMakeGuild::decode(&a.encode()).unwrap(), a);
        assert_eq!(CgAnswerMakeGuild::decode(&b.encode()).unwrap(), b);
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE];
        CgAnswerMakeGuild::new(SAMPLE).encode_into(&mut out);
        assert_eq!(out.len(), 1 + 14);
        assert_eq!(out[1], H);
    }
}
