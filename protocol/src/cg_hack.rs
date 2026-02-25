//! The 257-byte `Hack` record: a header and a raw 256-byte anti-cheat report.
//!
//! ```c
//! typedef struct SPacketCGHack
//! {
//!     BYTE bHeader;
//!     char szBuf[255+1];
//! } TPacketCGHack;
//! ```
//!
//! The declaration is **byte-identical** in both trees and has no `#ifdef`
//! anywhere inside it: `server/server/game/packet.h:2328-2332` and
//! `client/Client/UserInterface/Packet.h:919-923`. So
//! `1 + 256 = 257` closes against
//! `Set(HEADER_CG_HACK, sizeof(TPacketCGHack), "Hack")` at `packet_info.cpp:162`,
//! and there is **no feature profile to choose**. This is the easy one of the two
//! records over 16 bytes; `LOGIN3` is the other, and it is not.
//!
//! # The payload is plain text the server only logs
//!
//! `CInputMain::Hack` at `input_main.cpp:3219-3230` is the whole handler:
//!
//! ```c
//! void CInputMain::Hack(LPCHARACTER ch, const char * c_pData)
//! {
//!     TPacketCGHack * p = (TPacketCGHack *) c_pData;
//!     char buf[sizeof(p->szBuf)];
//!     strlcpy(buf, p->szBuf, sizeof(buf));
//!     sys_err("HACK_DETECT: %s %s", ch->GetName(), buf);
//!     ch->GetDesc()->SetPhase(PHASE_CLOSE);
//! }
//! ```
//!
//! It reads exactly one field, copies it through a **bounded** `strlcpy` into a
//! 256-byte local, logs it with the character name, and closes the character.
//! It does not decode, classify, count, or accumulate anything, and no other
//! reader of `szBuf` exists. So the record's only real structure is one boundary,
//! after the header byte -- and even that is generous.
//!
//! # The buffer is raw storage, and no terminator is required
//!
//! `szBuf` stays `[u8; 256]`. There is deliberately no `&str`, no `CStr`, and no
//! `to_str`:
//!
//! - The client declares `TPacketCGHack kPacketHack;` default-initialised
//!   rather than zeroed, and `strncpy` fills at most 255 of the 256 bytes. Byte
//!   255 and everything past the terminator is **indeterminate stack**.
//! - The server copes with a bounded `strlcpy` instead of requiring a
//!   terminator.
//!
//! A NUL-termination rule in the codec would therefore reject records the legacy
//! server accepts, and a UTF-8 rule would reject nearly all of them. Both would
//! be silent protocol changes. The same rule already governs the sash, guild,
//! and quest-text fixed `char` fields.
//!
//! # Reachable in two phases, and one of them is a no-op
//!
//! | site | source | behaviour |
//! |---|---|---|
//! | `CInputLogin::Analyze` | `input_login.cpp:1239-1240` | `case HEADER_CG_HACK: break;`, a deliberate no-op |
//! | `CInputMain::Analyze` | `input_main.cpp:3844-3846` | `Hack(ch, c_pData);` |
//! | `CInputDead::Analyze` | `input_main.cpp:4122-4124` | `Hack(ch, c_pData);`, a second copy in the dead phase |
//!
//! The login-phase arm is a **consume-and-drop**: the framing loop consumes the
//! 257 bytes and discards them. So `HACK` is reachable in two phases and only
//! one of them does anything. That is a session fact, and the codec must not
//! assume the main-phase behaviour.
//!
//! # No cross-direction collision
//!
//! Header 105 is not shared with any `GC` or `GG` record. The only other 105 in
//! the tree is `HEADER_DG_SET_EVENT_FLAG` at `common/tables.h:242`, which is
//! DB-peer framed with its own handle and length and imposes no second size.

use crate::cg_inventory::{CgHeader, HEADER_CG_HACK};
use crate::cg_wire::ClientFrame;

/// The legacy `char szBuf[255 + 1]` field width.
pub const CG_HACK_BUFFER_BYTES: usize = 255 + 1;

/// The full legacy `TPacketCGHack` record, header byte included.
pub const CG_HACK_WIRE_SIZE: usize = 1 + CG_HACK_BUFFER_BYTES;
/// The framed payload of `TPacketCGHack`.
pub const CG_HACK_PAYLOAD_SIZE: usize = CG_HACK_BUFFER_BYTES;

/// Every way the hack-report decoder can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgHackError {
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

impl core::fmt::Display for CgHackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated hack report: need {needed} bytes, got {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "hack report length mismatch: expected {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(
                    f,
                    "invalid hack report header: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for CgHackError {}

/// The 257-byte `Hack` record: a header and 256 raw report bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgHack {
    /// The legacy `char szBuf[256]`, as raw bytes. No terminator is required and
    /// none is invented.
    pub sz_buf: [u8; CG_HACK_BUFFER_BYTES],
}

impl CgHack {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_HACK
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_HACK_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_HACK_PAYLOAD_SIZE;

    /// Build the record from its raw report bytes.
    pub const fn new(sz_buf: [u8; CG_HACK_BUFFER_BYTES]) -> Self {
        Self { sz_buf }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.extend_from_slice(&self.sz_buf);
    }

    /// Encode to a fresh 257-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 256-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(Self::PAYLOAD_SIZE);
        payload.extend_from_slice(&self.sz_buf);
        ClientFrame {
            header: Self::header().value(),
            payload,
        }
    }

    /// # Errors
    ///
    /// [`CgHackError::Truncated`] below 257 bytes,
    /// [`CgHackError::LengthMismatch`] above,
    /// [`CgHackError::InvalidHeader`] for a full-length slice not starting with
    /// 105.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgHackError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(CgHackError::Truncated {
                needed: Self::WIRE_SIZE,
                available: bytes.len(),
            });
        }
        if bytes.len() > Self::WIRE_SIZE {
            return Err(CgHackError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header().value() {
            return Err(CgHackError::InvalidHeader {
                expected: Self::header().value(),
                actual: bytes[0],
            });
        }
        // `copy_from_slice` would panic on a length mismatch, and the three
        // checks above have already proved there are exactly 256 left. Building
        // the array through `try_from().expect()` instead would add a second
        // panic point that clippy then demands a `# Panics` section for.
        let mut sz_buf = [0u8; CG_HACK_BUFFER_BYTES];
        sz_buf.copy_from_slice(&bytes[1..]);
        Ok(Self { sz_buf })
    }

    /// # Errors
    ///
    /// As [`CgHack::decode`], except that the payload must be exactly 256
    /// bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgHackError> {
        if frame.payload.len() < Self::PAYLOAD_SIZE {
            return Err(CgHackError::Truncated {
                needed: Self::PAYLOAD_SIZE,
                available: frame.payload.len(),
            });
        }
        if frame.payload.len() > Self::PAYLOAD_SIZE {
            return Err(CgHackError::LengthMismatch {
                expected: Self::PAYLOAD_SIZE,
                actual: frame.payload.len(),
            });
        }
        if frame.header != Self::header().value() {
            return Err(CgHackError::InvalidHeader {
                expected: Self::header().value(),
                actual: frame.header,
            });
        }
        let mut sz_buf = [0u8; CG_HACK_BUFFER_BYTES];
        sz_buf.copy_from_slice(frame.payload.as_slice());
        Ok(Self { sz_buf })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u8 = 105;

    /// A realistic report: short ASCII, a NUL, then leftover stack bytes.
    fn report() -> [u8; CG_HACK_BUFFER_BYTES] {
        let mut b = [0xAA; CG_HACK_BUFFER_BYTES];
        b[..9].copy_from_slice(b"speedhack");
        b[9] = 0;
        b
    }

    #[test]
    fn the_header_is_105() {
        assert_eq!(CgHack::header().value(), H);
        assert_eq!(H, 0x69);
    }

    #[test]
    fn the_record_is_257_bytes() {
        assert_eq!(CG_HACK_BUFFER_BYTES, 256);
        assert_eq!(CG_HACK_WIRE_SIZE, 257);
        assert_eq!(CgHack::WIRE_SIZE, 257);
        assert_eq!(CgHack::PAYLOAD_SIZE, 256);
    }

    #[test]
    fn encodes_to_the_exact_legacy_bytes() {
        let r = CgHack::new(report());
        let b = r.encode();
        assert_eq!(b.len(), 257);
        assert_eq!(b[0], H);
        assert_eq!(&b[1..10], b"speedhack");
        assert_eq!(b[10], 0);
        assert_eq!(&b[11..], &[0xAA; 246]);
    }

    #[test]
    fn an_all_zero_buffer_is_valid() {
        let r = CgHack::new([0; CG_HACK_BUFFER_BYTES]);
        assert_eq!(CgHack::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn a_buffer_with_no_terminator_is_accepted() {
        // The legacy server uses a bounded strlcpy, so it does not require one.
        let r = CgHack::new([b'X'; CG_HACK_BUFFER_BYTES]);
        assert_eq!(CgHack::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn a_buffer_terminated_at_the_last_byte_is_accepted() {
        let mut b = [b'X'; CG_HACK_BUFFER_BYTES];
        b[CG_HACK_BUFFER_BYTES - 1] = 0;
        let r = CgHack::new(b);
        assert_eq!(CgHack::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn every_byte_value_is_accepted_in_every_position() {
        for v in 0..=255_u8 {
            for i in [0_usize, 1, 128, 254, 255] {
                let mut b = [0u8; CG_HACK_BUFFER_BYTES];
                b[i] = v;
                let r = CgHack::new(b);
                assert_eq!(
                    CgHack::decode(&r.encode()).unwrap().sz_buf,
                    b,
                    "value {v} at {i}"
                );
            }
        }
    }

    #[test]
    fn every_payload_byte_is_read() {
        let base = CgHack::new([0; CG_HACK_BUFFER_BYTES]);
        let mut changed = 0;
        for i in 0..CG_HACK_BUFFER_BYTES {
            let mut b = base.sz_buf;
            b[i] = 0xFF;
            if CgHack::new(b) != base {
                changed += 1;
            }
        }
        assert_eq!(changed, CG_HACK_BUFFER_BYTES);
    }

    #[test]
    fn round_trips() {
        let r = CgHack::new(report());
        assert_eq!(CgHack::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn round_trips_through_a_frame() {
        let r = CgHack::new(report());
        let f = r.to_frame();
        assert_eq!(f.header, H);
        assert_eq!(f.payload.len(), 256);
        assert_eq!(CgHack::decode_frame(&f).unwrap(), r);
        assert_eq!(&f.payload[..], &r.encode()[1..]);
    }

    #[test]
    fn distinct_reports_stay_distinct() {
        let mut a = [0u8; CG_HACK_BUFFER_BYTES];
        a[0] = 1;
        let mut b = [0u8; CG_HACK_BUFFER_BYTES];
        b[255] = 1;
        assert_ne!(CgHack::new(a), CgHack::new(b));
        assert_eq!(
            CgHack::decode(&CgHack::new(a).encode()).unwrap(),
            CgHack::new(a)
        );
        assert_eq!(
            CgHack::decode(&CgHack::new(b).encode()).unwrap(),
            CgHack::new(b)
        );
    }

    #[test]
    fn the_header_cannot_come_from_the_buffer() {
        let mut b = [0u8; CG_HACK_BUFFER_BYTES];
        b[0] = H;
        let f = ClientFrame {
            header: 0x01,
            payload: b.to_vec(),
        };
        assert_eq!(
            CgHack::decode_frame(&f).unwrap_err(),
            CgHackError::InvalidHeader {
                expected: 105,
                actual: 0x01
            }
        );
    }

    #[test]
    fn rejects_every_short_length() {
        for len in 0..257 {
            let mut b = CgHack::new(report()).encode();
            b.truncate(len);
            assert_eq!(
                CgHack::decode(&b).unwrap_err(),
                CgHackError::Truncated {
                    needed: 257,
                    available: len
                },
                "len {len}"
            );
        }
    }

    #[test]
    fn rejects_every_long_length() {
        for extra in [1_usize, 3, 4, 64, 256] {
            let mut b = CgHack::new(report()).encode();
            b.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgHack::decode(&b).unwrap_err(),
                CgHackError::LengthMismatch {
                    expected: 257,
                    actual: 257 + extra
                },
                "extra {extra}"
            );
        }
    }

    #[test]
    fn rejects_every_wrong_header() {
        for v in 0..=255_u8 {
            if v == H {
                continue;
            }
            let mut b = CgHack::new(report()).encode();
            b[0] = v;
            assert_eq!(
                CgHack::decode(&b).unwrap_err(),
                CgHackError::InvalidHeader {
                    expected: 105,
                    actual: v
                },
                "header {v}"
            );
        }
    }

    #[test]
    fn rejects_every_wrong_frame_length() {
        for len in [0_usize, 1, 255, 257, 512] {
            let f = ClientFrame {
                header: H,
                payload: vec![0; len],
            };
            assert!(CgHack::decode_frame(&f).is_err(), "len {len}");
        }
    }

    #[test]
    fn the_error_type_displays() {
        assert!(CgHackError::Truncated {
            needed: 257,
            available: 0
        }
        .to_string()
        .contains("257"));
        assert!(CgHackError::LengthMismatch {
            expected: 257,
            actual: 258
        }
        .to_string()
        .contains("258"));
        assert!(CgHackError::InvalidHeader {
            expected: 105,
            actual: 1
        }
        .to_string()
        .contains("105"));
        let _: &dyn std::error::Error = &CgHackError::Truncated {
            needed: 257,
            available: 0,
        };
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE, 0xAD];
        CgHack::new(report()).encode_into(&mut out);
        assert_eq!(out.len(), 2 + 257);
        assert_eq!(out[2], H);
    }
}
