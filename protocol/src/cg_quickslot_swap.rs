//! Transport-free codec for the legacy `TPacketCGQuickslotSwap` record.
//!
//! # Wire layout
//!
//! The server declares the record at `server/server/game/packet.h:677-682`:
//!
//! ```cpp
//! typedef struct command_quickslot_swap
//! {
//!     BYTE    header;
//!     BYTE    pos;
//!     BYTE    change_pos;
//! } TPacketCGQuickslotSwap;
//! ```
//!
//! `packet.h:27` sets `HEADER_CG_QUICKSLOT_SWAP = 18`. The declaration is
//! strictly inside the `#pragma pack(1)` that `packet.h:274` opens and does not
//! close until `:3540`, and no `#ifdef` touches it, so the width is fixed.
//!
//! ```text
//! [0x12][pos: u8][change_pos: u8]   3 bytes total, 2 after the header
//! ```
//!
//! The client spells the same record `TPacketCGQuickSlotSwap` at
//! `client/Client/UserInterface/Packet.h:562-567`, with the same struct tag
//! `command_quickslot_swap`, the same three members including the same
//! `change_pos` field name, and the same 3-byte width, and its enumerator at
//! `Packet.h:27` also says 18.
//!
//! The two indices are interchangeable in meaning but not in position: `pos` is
//! the source quickbar slot and `change_pos` is the destination. Nothing in the
//! wire form says which is which, and neither value is ordered or validated, so
//! a swap of equal indices is a legal encoding.
//!
//! # Naming note
//!
//! The *outbound* mirror of this record is a different record at
//! `HEADER_GC_QUICKSLOT_SWAP` (30), declared as the bare struct
//! `packet_quickslot_swap` at `packet.h:1521-1526`, and it calls its second
//! field **`pos_to`**, not `change_pos`. The client agrees with the server on
//! that one: `TPacketGCQuickSlotSwap` at `client/Client/UserInterface/Packet.h`
//! also uses `change_pos` for the GC record. So the `pos_to` spelling is the
//! server's outbound-side outlier. This codec uses the CG spelling
//! `change_pos`, because that is the spelling both trees use for *this* record.
//!
//! # What this codec deliberately does not do
//!
//! Both fields are **opaque** `u8` quickbar indices. The bounds on them,
//! `a >= QUICKSLOT_MAX_NUM || b >= QUICKSLOT_MAX_NUM` (36), live in
//! `CHARACTER::SwapQuickslot` at `server/server/game/char_quickslot.cpp:121`,
//! which is gameplay policy above the record boundary. This codec does not
//! apply it, does not require the two indices to differ, does not mask either
//! value, and does not reject `0` or any other byte.
//!
//! It does not swap any state, does not send the `HEADER_GC_QUICKSLOT_SWAP`
//! reply that `SwapQuickslot` builds, and does not model that method's `bool`
//! return the way `CInputMain::QuickslotSwap` discards it.

use crate::cg_inventory::HEADER_CG_QUICKSLOT_SWAP;
use crate::cg_wire::ClientFrame;

/// The full legacy record, header byte included.
pub const CG_QUICKSLOT_SWAP_WIRE_SIZE: usize = 3;

/// The framed payload, everything after the header byte.
pub const CG_QUICKSLOT_SWAP_PAYLOAD_SIZE: usize = 2;

/// The transport-free `TPacketCGQuickslotSwap` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgQuickslotSwap {
    /// The source quickbar index. Opaque: the `QUICKSLOT_MAX_NUM` bound is
    /// applied by `CHARACTER::SwapQuickslot`, not here.
    pub pos: u8,
    /// The destination quickbar index. Opaque on the same terms as `pos`.
    pub change_pos: u8,
}

/// Every way [`CgQuickslotSwap::decode`] can refuse a byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgQuickslotSwapError {
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
    /// The correct number of bytes, but the header byte is not 18.
    InvalidHeader {
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl std::fmt::Display for CgQuickslotSwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "quickslot-swap needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => write!(
                f,
                "quickslot-swap must be exactly {expected} bytes, got {actual}"
            ),
            Self::InvalidHeader { actual } => {
                write!(f, "quickslot-swap header {actual} is not 18")
            }
        }
    }
}

impl std::error::Error for CgQuickslotSwapError {}

impl CgQuickslotSwap {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> crate::cg_inventory::CgHeader {
        HEADER_CG_QUICKSLOT_SWAP
    }

    /// Build a record from a source and a destination index. No value is
    /// rejected, and the two are not required to differ.
    pub const fn new(pos: u8, change_pos: u8) -> Self {
        Self { pos, change_pos }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.pos);
        out.push(self.change_pos);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_QUICKSLOT_SWAP_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, header byte and payload.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_QUICKSLOT_SWAP_PAYLOAD_SIZE);
        payload.push(self.pos);
        payload.push(self.change_pos);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgQuickslotSwapError::Truncated`] for a short input,
    /// [`CgQuickslotSwapError::LengthMismatch`] for a long input, and
    /// [`CgQuickslotSwapError::InvalidHeader`] when the header is not 18. The
    /// length is checked before the header, and the header is checked before
    /// any payload byte is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgQuickslotSwapError> {
        check_exact(bytes.len())?;
        check_header(bytes[0])?;
        Ok(Self {
            pos: bytes[1],
            change_pos: bytes[2],
        })
    }

    /// # Errors
    ///
    /// Returns [`CgQuickslotSwapError::Truncated`] for a short input,
    /// [`CgQuickslotSwapError::LengthMismatch`] for a long input, and
    /// [`CgQuickslotSwapError::InvalidHeader`] when the header is not 18. The
    /// the header-less payload length is checked before the frame header, and the header is checked before
    /// any payload byte is read.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgQuickslotSwapError> {
        // The frame payload excludes the header byte, so this checks the
        // payload width, not the full record width.
        check_exact_payload(frame.payload.len())?;
        check_header(frame.header)?;
        Ok(Self {
            pos: frame.payload[0],
            change_pos: frame.payload[1],
        })
    }
}

fn check_exact(len: usize) -> Result<(), CgQuickslotSwapError> {
    if len < CG_QUICKSLOT_SWAP_WIRE_SIZE {
        return Err(CgQuickslotSwapError::Truncated {
            needed: CG_QUICKSLOT_SWAP_WIRE_SIZE,
            available: len,
        });
    }
    if len > CG_QUICKSLOT_SWAP_WIRE_SIZE {
        return Err(CgQuickslotSwapError::LengthMismatch {
            expected: CG_QUICKSLOT_SWAP_WIRE_SIZE,
            actual: len,
        });
    }
    Ok(())
}

fn check_exact_payload(len: usize) -> Result<(), CgQuickslotSwapError> {
    if len < CG_QUICKSLOT_SWAP_PAYLOAD_SIZE {
        return Err(CgQuickslotSwapError::Truncated {
            needed: CG_QUICKSLOT_SWAP_PAYLOAD_SIZE,
            available: len,
        });
    }
    if len > CG_QUICKSLOT_SWAP_PAYLOAD_SIZE {
        return Err(CgQuickslotSwapError::LengthMismatch {
            expected: CG_QUICKSLOT_SWAP_PAYLOAD_SIZE,
            actual: len,
        });
    }
    Ok(())
}

fn check_header(actual: u8) -> Result<(), CgQuickslotSwapError> {
    if actual != HEADER_CG_QUICKSLOT_SWAP.value() {
        return Err(CgQuickslotSwapError::InvalidHeader { actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CgQuickslotSwap, CgQuickslotSwapError, CG_QUICKSLOT_SWAP_PAYLOAD_SIZE,
        CG_QUICKSLOT_SWAP_WIRE_SIZE,
    };
    use crate::cg_inventory::HEADER_CG_QUICKSLOT_SWAP;
    use crate::cg_wire::ClientFrame;

    fn wire(pos: u8, change_pos: u8) -> Vec<u8> {
        vec![18, pos, change_pos]
    }

    #[test]
    fn constants_match_the_source() {
        assert_eq!(HEADER_CG_QUICKSLOT_SWAP.value(), 18);
        assert_eq!(CgQuickslotSwap::header().value(), 18);
        // packet.h:677-682 -> BYTE header, BYTE pos, BYTE change_pos.
        assert_eq!(CG_QUICKSLOT_SWAP_WIRE_SIZE, 3);
        assert_eq!(CG_QUICKSLOT_SWAP_PAYLOAD_SIZE, 2);
        assert_eq!(
            CG_QUICKSLOT_SWAP_WIRE_SIZE,
            1 + CG_QUICKSLOT_SWAP_PAYLOAD_SIZE
        );
    }

    #[test]
    fn golden_bytes_for_a_known_pair() {
        assert_eq!(CgQuickslotSwap::new(2, 7).encode(), vec![18, 2, 7]);
        assert_eq!(wire(2, 7), CgQuickslotSwap::new(2, 7).encode());
    }

    /// A swap of a slot with itself is a legal encoding; the legacy bounds do
    /// not require the two indices to differ.
    #[test]
    fn equal_indices_are_not_rejected() {
        for v in [0u8, 1, 35, 255] {
            let rec = CgQuickslotSwap::new(v, v);
            assert_eq!(rec.encode(), vec![18, v, v]);
            assert_eq!(CgQuickslotSwap::decode(&rec.encode()).expect("d"), rec);
        }
    }

    /// Source and destination are distinct fields and must never be
    /// transposed. This pins the exact wire order.
    #[test]
    fn pos_precedes_change_pos() {
        assert_eq!(CgQuickslotSwap::new(4, 9).encode(), vec![18, 4, 9]);
        assert_ne!(
            CgQuickslotSwap::new(4, 9).encode(),
            CgQuickslotSwap::new(9, 4).encode()
        );
    }

    /// `QUICKSLOT_MAX_NUM` is 36, and that bound is applied by
    /// `CHARACTER::SwapQuickslot`, not here, so every byte must round-trip.
    #[test]
    fn no_pos_value_is_rejected() {
        for a in 0u8..=255 {
            for b in [0u8, 1, 35, 36, 128, 254, 255] {
                let rec = CgQuickslotSwap::new(a, b);
                let bytes = rec.encode();
                assert_eq!(bytes.len(), CG_QUICKSLOT_SWAP_WIRE_SIZE);
                assert_eq!(bytes[0], 18);
                assert_eq!(bytes[1], a, "a {a}");
                assert_eq!(bytes[2], b, "b {b}");
                assert_eq!(CgQuickslotSwap::decode(&bytes).expect("d"), rec);
                assert_eq!(
                    CgQuickslotSwap::decode_frame(&rec.to_frame()).expect("f"),
                    rec
                );
            }
        }
    }

    #[test]
    fn a_dense_sweep_round_trips() {
        for v in 0u32..4096 {
            let rec = CgQuickslotSwap::new((v & 0xff) as u8, ((v >> 4) & 0xff) as u8);
            assert_eq!(CgQuickslotSwap::decode(&rec.encode()).expect("d"), rec);
        }
    }

    #[test]
    fn length_is_checked_before_the_header() {
        for first in [0x00u8, 0x12, 0xff] {
            for len in 0..CG_QUICKSLOT_SWAP_WIRE_SIZE {
                let mut buf = vec![first];
                buf.resize(len, 0xcc);
                assert_eq!(
                    CgQuickslotSwap::decode(&buf),
                    Err(CgQuickslotSwapError::Truncated {
                        needed: CG_QUICKSLOT_SWAP_WIRE_SIZE,
                        available: len,
                    }),
                    "first {first:#x} len {len}"
                );
            }
        }
    }

    #[test]
    fn a_long_input_is_a_length_mismatch() {
        for len in (CG_QUICKSLOT_SWAP_WIRE_SIZE + 1)..(CG_QUICKSLOT_SWAP_WIRE_SIZE + 6) {
            let mut buf = wire(1, 1);
            buf.resize(len, 0xdd);
            assert_eq!(
                CgQuickslotSwap::decode(&buf),
                Err(CgQuickslotSwapError::LengthMismatch {
                    expected: CG_QUICKSLOT_SWAP_WIRE_SIZE,
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
            let buf = vec![first, 4, 9];
            match CgQuickslotSwap::decode(&buf) {
                Ok(rec) => {
                    assert_eq!(first, 18);
                    assert_eq!(rec.pos, 4);
                    assert_eq!(rec.change_pos, 9);
                    accepted += 1;
                }
                Err(CgQuickslotSwapError::InvalidHeader { actual }) => assert_eq!(actual, first),
                Err(other) => panic!("header {first:#x} gave {other:?}"),
            }
        }
        assert_eq!(accepted, 1);
    }

    #[test]
    fn a_wrong_header_with_an_exact_length_is_rejected() {
        for bad in [0x00u8, 0x11, 0x13, 0x14, 0xff] {
            let mut buf = wire(0, 0);
            buf[0] = bad;
            assert_eq!(
                CgQuickslotSwap::decode(&buf),
                Err(CgQuickslotSwapError::InvalidHeader { actual: bad }),
                "header {bad:#x}"
            );
        }
    }

    #[test]
    fn the_framed_payload_length_is_checked_before_the_frame_header() {
        let frame = ClientFrame::new(0xff, vec![1]);
        assert_eq!(
            CgQuickslotSwap::decode_frame(&frame),
            Err(CgQuickslotSwapError::Truncated {
                needed: CG_QUICKSLOT_SWAP_PAYLOAD_SIZE,
                available: 1,
            })
        );
        let frame = ClientFrame::new(0xff, vec![1, 2, 3]);
        assert_eq!(
            CgQuickslotSwap::decode_frame(&frame),
            Err(CgQuickslotSwapError::LengthMismatch {
                expected: CG_QUICKSLOT_SWAP_PAYLOAD_SIZE,
                actual: 3,
            })
        );
        let frame = ClientFrame::new(0xff, vec![1, 2]);
        assert_eq!(
            CgQuickslotSwap::decode_frame(&frame),
            Err(CgQuickslotSwapError::InvalidHeader { actual: 0xff })
        );
        let frame = ClientFrame::new(0xff, Vec::new());
        assert_eq!(
            CgQuickslotSwap::decode_frame(&frame),
            Err(CgQuickslotSwapError::Truncated {
                needed: CG_QUICKSLOT_SWAP_PAYLOAD_SIZE,
                available: 0,
            })
        );
    }

    #[test]
    fn the_header_is_fixed_and_never_stored() {
        for (a, b) in [(0u8, 0u8), (1, 1), (255, 255)] {
            let rec = CgQuickslotSwap::new(a, b);
            assert_eq!(rec.encode()[0], 18);
            assert_eq!(rec.to_frame().header, 18);
        }
    }

    #[test]
    fn encode_into_appends_rather_than_replaces() {
        let mut out = vec![0xde, 0xad];
        CgQuickslotSwap::new(2, 7).encode_into(&mut out);
        assert_eq!(out, vec![0xde, 0xad, 18, 2, 7]);
    }

    #[test]
    fn errors_display_without_panicking() {
        let cases = [
            CgQuickslotSwapError::Truncated {
                needed: 3,
                available: 2,
            },
            CgQuickslotSwapError::LengthMismatch {
                expected: 3,
                actual: 9,
            },
            CgQuickslotSwapError::InvalidHeader { actual: 0x11 },
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
        let cases = [(0u8, 0u8), (1, 2), (35, 36), (255, 255)];
        let mut stream = Vec::new();
        for (a, b) in cases {
            stream.extend_from_slice(&wire(a, b));
        }
        let mut dec = ClientFrameDecoder::new();
        dec.feed(&stream).expect("feed");
        let mut got = Vec::new();
        while let Some(frame) = dec.try_decode().expect("decode") {
            assert_eq!(frame.header, 18);
            got.push(CgQuickslotSwap::decode_frame(&frame).expect("codec"));
        }
        let want: Vec<_> = cases
            .iter()
            .map(|(a, b)| CgQuickslotSwap::new(*a, *b))
            .collect();
        assert_eq!(got, want);
        assert!(dec.is_empty());
    }

    /// Headers 16, 17, and 18 are adjacent quickslot records with widths 4, 2,
    /// and 3. They must not be confusable.
    #[test]
    fn the_quickslot_siblings_do_not_share_this_header() {
        assert_ne!(HEADER_CG_QUICKSLOT_SWAP.value(), 16);
        assert_ne!(HEADER_CG_QUICKSLOT_SWAP.value(), 17);
        assert_eq!(CG_QUICKSLOT_SWAP_WIRE_SIZE, 3);
    }
}
