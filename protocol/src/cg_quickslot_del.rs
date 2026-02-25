//! Transport-free codec for the legacy `TPacketCGQuickslotDel` record.
//!
//! # Wire layout
//!
//! The server declares the record at `server/server/game/packet.h:671-675`:
//!
//! ```cpp
//! typedef struct command_quickslot_del
//! {
//!     BYTE    header;
//!     BYTE    pos;
//! } TPacketCGQuickslotDel;
//! ```
//!
//! `packet.h:26` sets `HEADER_CG_QUICKSLOT_DEL = 17`. The declaration is
//! strictly inside the `#pragma pack(1)` that `packet.h:274` opens and does not
//! close until `:3540`, and no `#ifdef` touches it, so the width is fixed.
//!
//! ```text
//! [0x11][pos: u8]   2 bytes total, 1 after the header
//! ```
//!
//! The client spells the same record `TPacketCGQuickSlotDel` at
//! `client/Client/UserInterface/Packet.h:556-560`, with the same struct tag
//! `command_quickslot_del`, the same two members, and the same 2-byte width, and
//! its enumerator at `Packet.h:26` also says 17. The capital `S` in
//! `QuickSlot` is the only difference.
//!
//! # What this codec deliberately does not do
//!
//! `pos` is an **opaque** `u8` quickbar index. The only bound on it,
//! `pos >= QUICKSLOT_MAX_NUM` (36), lives in `CHARACTER::DelQuickslot` at
//! `server/server/game/char_quickslot.cpp:104`, which is gameplay policy above
//! the record boundary. This codec does not apply it, does not mask the value,
//! and does not reject `0` or any other byte.
//!
//! It does not resolve the quickslot state, does not send the
//! `HEADER_GC_QUICKSLOT_DEL` reply that `DelQuickslot` builds, and does not
//! discard or model that method's `bool` return the way
//! `CInputMain::QuickslotDelete` does.

use crate::cg_inventory::HEADER_CG_QUICKSLOT_DEL;
use crate::cg_wire::ClientFrame;

/// The full legacy record, header byte included.
pub const CG_QUICKSLOT_DEL_WIRE_SIZE: usize = 2;

/// The framed payload, everything after the header byte.
pub const CG_QUICKSLOT_DEL_PAYLOAD_SIZE: usize = 1;

/// The transport-free `TPacketCGQuickslotDel` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgQuickslotDel {
    /// The quickbar index. Opaque: the `QUICKSLOT_MAX_NUM` bound is applied by
    /// `CHARACTER::DelQuickslot`, not here.
    pub pos: u8,
}

/// Every way [`CgQuickslotDel::decode`] can refuse a byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgQuickslotDelError {
    /// Fewer bytes than the fixed record needs.
    Truncated {
        /// The fixed width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// A complete-length slice that is not the fixed width.
    LengthMismatch {
        /// The one width this record has.
        expected: usize,
        /// The width that was offered.
        actual: usize,
    },
    /// The correct number of bytes, but the header byte is not 17.
    InvalidHeader {
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl std::fmt::Display for CgQuickslotDelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "quickslot-del needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => write!(
                f,
                "quickslot-del must be exactly {expected} bytes, got {actual}"
            ),
            Self::InvalidHeader { actual } => {
                write!(f, "quickslot-del header {actual} is not 17")
            }
        }
    }
}

impl std::error::Error for CgQuickslotDelError {}

impl CgQuickslotDel {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> crate::cg_inventory::CgHeader {
        HEADER_CG_QUICKSLOT_DEL
    }

    /// Build a record from a quickbar index. No value is rejected.
    pub const fn new(pos: u8) -> Self {
        Self { pos }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.pos);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_QUICKSLOT_DEL_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_QUICKSLOT_DEL_PAYLOAD_SIZE);
        payload.push(self.pos);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgQuickslotDelError::Truncated`] for a short input,
    /// [`CgQuickslotDelError::LengthMismatch`] for a long input, and
    /// [`CgQuickslotDelError::InvalidHeader`] when the header is not 17. The
    /// length is checked before the header, and the header is checked before
    /// any payload byte is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgQuickslotDelError> {
        check_exact(bytes.len())?;
        check_header(bytes[0])?;
        Ok(Self { pos: bytes[1] })
    }

    /// # Errors
    ///
    /// Returns [`CgQuickslotDelError::Truncated`] for a short input,
    /// [`CgQuickslotDelError::LengthMismatch`] for a long input, and
    /// [`CgQuickslotDelError::InvalidHeader`] when the header is not 17. The
    /// the header-less payload length is checked before the frame header, and the header is checked before
    /// any payload byte is read.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgQuickslotDelError> {
        // The frame payload excludes the header byte, so this checks the
        // payload width, not the full record width.
        check_exact_payload(frame.payload.len())?;
        check_header(frame.header)?;
        Ok(Self {
            pos: frame.payload[0],
        })
    }
}

fn check_exact(len: usize) -> Result<(), CgQuickslotDelError> {
    if len < CG_QUICKSLOT_DEL_WIRE_SIZE {
        return Err(CgQuickslotDelError::Truncated {
            needed: CG_QUICKSLOT_DEL_WIRE_SIZE,
            available: len,
        });
    }
    if len > CG_QUICKSLOT_DEL_WIRE_SIZE {
        return Err(CgQuickslotDelError::LengthMismatch {
            expected: CG_QUICKSLOT_DEL_WIRE_SIZE,
            actual: len,
        });
    }
    Ok(())
}

fn check_exact_payload(len: usize) -> Result<(), CgQuickslotDelError> {
    if len < CG_QUICKSLOT_DEL_PAYLOAD_SIZE {
        return Err(CgQuickslotDelError::Truncated {
            needed: CG_QUICKSLOT_DEL_PAYLOAD_SIZE,
            available: len,
        });
    }
    if len > CG_QUICKSLOT_DEL_PAYLOAD_SIZE {
        return Err(CgQuickslotDelError::LengthMismatch {
            expected: CG_QUICKSLOT_DEL_PAYLOAD_SIZE,
            actual: len,
        });
    }
    Ok(())
}

fn check_header(actual: u8) -> Result<(), CgQuickslotDelError> {
    if actual != HEADER_CG_QUICKSLOT_DEL.value() {
        return Err(CgQuickslotDelError::InvalidHeader { actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CgQuickslotDel, CgQuickslotDelError, CG_QUICKSLOT_DEL_PAYLOAD_SIZE,
        CG_QUICKSLOT_DEL_WIRE_SIZE,
    };
    use crate::cg_inventory::HEADER_CG_QUICKSLOT_DEL;
    use crate::cg_wire::ClientFrame;

    fn wire(pos: u8) -> Vec<u8> {
        vec![17, pos]
    }

    #[test]
    fn constants_match_the_source() {
        assert_eq!(HEADER_CG_QUICKSLOT_DEL.value(), 17);
        assert_eq!(CgQuickslotDel::header().value(), 17);
        // packet.h:671-675 -> BYTE header, BYTE pos.
        assert_eq!(CG_QUICKSLOT_DEL_WIRE_SIZE, 2);
        assert_eq!(CG_QUICKSLOT_DEL_PAYLOAD_SIZE, 1);
        assert_eq!(
            CG_QUICKSLOT_DEL_WIRE_SIZE,
            1 + CG_QUICKSLOT_DEL_PAYLOAD_SIZE
        );
    }

    #[test]
    fn golden_bytes_for_a_known_index() {
        assert_eq!(CgQuickslotDel::new(5).encode(), vec![17, 5]);
        assert_eq!(wire(5), CgQuickslotDel::new(5).encode());
    }

    /// `QUICKSLOT_MAX_NUM` is 36, and that bound is applied by
    /// `CHARACTER::DelQuickslot`, not here, so every byte must round-trip.
    #[test]
    fn no_pos_value_is_rejected() {
        for pos in 0u8..=255 {
            let rec = CgQuickslotDel::new(pos);
            let bytes = rec.encode();
            assert_eq!(bytes.len(), CG_QUICKSLOT_DEL_WIRE_SIZE);
            assert_eq!(bytes[0], 17);
            assert_eq!(bytes[1], pos, "pos {pos}");
            assert_eq!(CgQuickslotDel::decode(&bytes).expect("d"), rec);
            assert_eq!(
                CgQuickslotDel::decode_frame(&rec.to_frame()).expect("f"),
                rec
            );
        }
    }

    #[test]
    fn the_whole_u8_domain_is_covered() {
        let mut seen = std::collections::BTreeSet::new();
        for pos in 0u8..=255 {
            assert!(seen.insert(CgQuickslotDel::new(pos).encode()));
        }
        assert_eq!(seen.len(), 256);
    }

    #[test]
    fn length_is_checked_before_the_header() {
        for first in [0x00u8, 0x11, 0xff] {
            for len in 0..CG_QUICKSLOT_DEL_WIRE_SIZE {
                let mut buf = vec![first];
                buf.resize(len, 0xcc);
                assert_eq!(
                    CgQuickslotDel::decode(&buf),
                    Err(CgQuickslotDelError::Truncated {
                        needed: CG_QUICKSLOT_DEL_WIRE_SIZE,
                        available: len,
                    }),
                    "first {first:#x} len {len}"
                );
            }
        }
    }

    #[test]
    fn a_long_input_is_a_length_mismatch() {
        for len in (CG_QUICKSLOT_DEL_WIRE_SIZE + 1)..(CG_QUICKSLOT_DEL_WIRE_SIZE + 6) {
            let mut buf = wire(1);
            buf.resize(len, 0xdd);
            assert_eq!(
                CgQuickslotDel::decode(&buf),
                Err(CgQuickslotDelError::LengthMismatch {
                    expected: CG_QUICKSLOT_DEL_WIRE_SIZE,
                    actual: len,
                }),
                "len {len}"
            );
        }
    }

    #[test]
    fn exactly_one_leading_byte_is_accepted() {
        let mut accepted = 0;
        for first in 0u8..=255 {
            let buf = vec![first, 9];
            match CgQuickslotDel::decode(&buf) {
                Ok(rec) => {
                    assert_eq!(first, 17);
                    assert_eq!(rec.pos, 9);
                    accepted += 1;
                }
                Err(CgQuickslotDelError::InvalidHeader { actual }) => assert_eq!(actual, first),
                Err(other) => panic!("header {first:#x} gave {other:?}"),
            }
        }
        assert_eq!(accepted, 1);
    }

    #[test]
    fn a_wrong_header_with_an_exact_length_is_rejected() {
        for bad in [0x00u8, 0x10, 0x12, 0x18, 0xff] {
            let mut buf = wire(0);
            buf[0] = bad;
            assert_eq!(
                CgQuickslotDel::decode(&buf),
                Err(CgQuickslotDelError::InvalidHeader { actual: bad }),
                "header {bad:#x}"
            );
        }
    }

    #[test]
    fn the_framed_payload_length_is_checked_before_the_frame_header() {
        let frame = ClientFrame::new(0xff, Vec::new());
        assert_eq!(
            CgQuickslotDel::decode_frame(&frame),
            Err(CgQuickslotDelError::Truncated {
                needed: CG_QUICKSLOT_DEL_PAYLOAD_SIZE,
                available: 0,
            })
        );
        let frame = ClientFrame::new(0xff, vec![1, 2]);
        assert_eq!(
            CgQuickslotDel::decode_frame(&frame),
            Err(CgQuickslotDelError::LengthMismatch {
                expected: CG_QUICKSLOT_DEL_PAYLOAD_SIZE,
                actual: 2,
            })
        );
        let frame = ClientFrame::new(0xff, vec![1]);
        assert_eq!(
            CgQuickslotDel::decode_frame(&frame),
            Err(CgQuickslotDelError::InvalidHeader { actual: 0xff })
        );
    }

    #[test]
    fn the_header_is_fixed_and_never_stored() {
        for pos in [0u8, 1, 17, 255] {
            let rec = CgQuickslotDel::new(pos);
            assert_eq!(rec.encode()[0], 17);
            assert_eq!(rec.to_frame().header, 17);
        }
    }

    #[test]
    fn encode_into_appends_rather_than_replaces() {
        let mut out = vec![0xde, 0xad];
        CgQuickslotDel::new(3).encode_into(&mut out);
        assert_eq!(out, vec![0xde, 0xad, 17, 3]);
    }

    #[test]
    fn errors_display_without_panicking() {
        let cases = [
            CgQuickslotDelError::Truncated {
                needed: 2,
                available: 1,
            },
            CgQuickslotDelError::LengthMismatch {
                expected: 2,
                actual: 8,
            },
            CgQuickslotDelError::InvalidHeader { actual: 0x12 },
        ];
        for e in cases {
            assert!(!e.to_string().is_empty());
            let dyn_err: &dyn std::error::Error = &e;
            assert!(!dyn_err.to_string().is_empty());
        }
    }

    #[test]
    fn it_decodes_through_the_fixed_frame_decoder() {
        use crate::cg_wire::ClientFrameDecoder;
        let cases = [0u8, 1, 35, 36, 255];
        let mut stream = Vec::new();
        for pos in cases {
            stream.extend_from_slice(&wire(pos));
        }
        let mut dec = ClientFrameDecoder::new();
        dec.feed(&stream).expect("feed");
        let mut got = Vec::new();
        while let Some(frame) = dec.try_decode().expect("decode") {
            assert_eq!(frame.header, 17);
            got.push(CgQuickslotDel::decode_frame(&frame).expect("codec"));
        }
        let want: Vec<_> = cases.iter().map(|p| CgQuickslotDel::new(*p)).collect();
        assert_eq!(got, want);
        assert!(dec.is_empty());
    }

    /// Headers 17, 18, and 16 are adjacent quickslot records with widths 2, 3,
    /// and 4. They must not be confusable.
    #[test]
    fn the_quickslot_siblings_do_not_share_this_header() {
        assert_ne!(HEADER_CG_QUICKSLOT_DEL.value(), 16);
        assert_ne!(HEADER_CG_QUICKSLOT_DEL.value(), 18);
        assert_eq!(CG_QUICKSLOT_DEL_WIRE_SIZE, 2);
    }
}
