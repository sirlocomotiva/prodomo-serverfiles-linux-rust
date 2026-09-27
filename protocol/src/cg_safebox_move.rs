//! Explicit codec for the fixed legacy `TPacketCGItemMove` record as it is
//! reused for the safebox and mall in-container move, header 77.
//!
//! # Why this is a separate record and not a second `CgItemMove` header
//!
//! `server/server/game/packet_info.cpp:157` registers
//! `HEADER_CG_SAFEBOX_ITEM_MOVE` as `sizeof(TPacketCGItemMove)`, the very same
//! C++ type that `packet_info.cpp:121` registers for header 13. The two
//! records are therefore **byte-identical apart from byte 0**, and this module
//! deliberately does not fold them together. They are not one record with two
//! spellings:
//!
//! * Header 13 is handled by `CInputMain::ItemMove` at
//!   `server/server/game/input_main.cpp:1060-1066`, which calls
//!   `ch->MoveItem(pinfo->Cell, pinfo->CellTo, pinfo->count)`. It passes both
//!   **whole** `TItemPos` values, so the server resolves the destination
//!   window. The signature is `CHARACTER::MoveItem(TItemPos pos, TItemPos
//!   change_pos, WORD num)` at `server/server/game/char.h:1209`.
//! * Header 77 is handled by `CInputMain::SafeboxItemMove` at
//!   `input_main.cpp:2462-2473`, whose entire body is two guards and one call:
//!   `ch->GetSafebox()->MoveItem(pinfo->Cell.cell, pinfo->CellTo.cell,
//!   pinfo->count)`. It reads **only** the two `.cell` halves; the two
//!   `window_type` bytes are never read on this path at all.
//!
//! # Why a separate type, stated correctly
//!
//! An earlier draft of this module justified the split by claiming that merging
//! would have to pick one meaning for the window bytes. **That reasoning was
//! wrong and is withdrawn.** The window bytes are opaque at this boundary, and a
//! merged type would simply have kept both `TItemPos` values whole and never
//! interpreted them, so there was no meaning to pick and nothing to arbitrate.
//! The split is right, but for a different and weaker reason:
//!
//! * **It is an API-level guarantee, not a wire-level one.** With two distinct
//!   types, handing a header-77 record to a header-13 call site is a **compile
//!   error**. With a single type carrying a `kind` field, the same mistake
//!   compiles cleanly and fails only on the wire, at runtime, in whichever
//!   direction the bug points. Separate types move the mistake from production
//!   back to compilation.
//! * **Header 13 is already closed.** `crate::cg_item_move::CgItemMove` is
//!   receipted on the contract that it stores no header field and always
//!   encodes 13, and `protocol/src/cg_item_use.rs` and
//!   `protocol/src/cg_item_use_to_item.rs` set the same precedent for a shared
//!   layout.
//!
//! Against that, the honest costs are real: this duplicates roughly the layout
//! half of a nine-byte codec, and a future change to `TItemPos` would have to
//! be made in two places. That duplication is accepted deliberately, and the
//! shared `CgItemPos` is what keeps the *packed layout* from actually drifting.
//!
//! # Cross-direction collision
//!
//! Header 77 is **not** free in the opposite direction: it equals
//! `HEADER_GC_PARTY_INVITE` at `server/server/game/packet.h:164` and
//! `client/Client/UserInterface/Packet.h:161`, an inbound game-to-client party
//! invite of **five** packed bytes, registered at
//! `client/Client/UserInterface/PythonNetworkStream.cpp:108` and dispatched at
//! `PythonNetworkStreamPhaseGame.cpp:472-474`. Header 13 likewise equals
//! `HEADER_GC_STUN` at `packet.h:113`. One value therefore needs **two
//! different sizes** depending on direction, so the two directions must never
//! share a header table. This is the same fact already recorded for
//! `crate::cg_safebox` and `crate::cg_item_give`, and it is restated here
//! because a record that collides is not safe to treat as a globally unique
//! number.
//!
//! # Layout
//!
//! `server/server/game/packet.h:633-639` declares `BYTE header`, a packed
//! `TItemPos Cell`, a packed `TItemPos CellTo`, and a `WORD count`. `TItemPos`
//! is three packed bytes, `BYTE window_type` then `WORD cell`, at
//! `server/server/common/length.h:956-960`. The complete wire record is
//! therefore **nine bytes**, and the fixed client framing payload is eight.
//! `protocol/src/cg_inventory.rs` already records this as `base_size: 9`.
//!
//! # Field names differ between the two trees
//!
//! The client declares the same four fields at
//! `client/Client/UserInterface/Packet.h:529-535` as `header`, `pos`,
//! `change_pos`, and `WORD num`. The server calls the last three `Cell`,
//! `CellTo`, and `count`. The layout is shared; the names are not. No field
//! here borrows a name from only one tree as though the trees agreed.
//!
//! # Both window bytes stay opaque, and the client sends `INVENTORY`
//!
//! The legacy client builder at
//! `client/Client/UserInterface/PythonNetworkStreamPhaseGameItem.cpp:77` and
//! `:79` hardcodes **both** window bytes to `INVENTORY`, and the server never
//! reads them, so a well-behaved client always transmits the same window value
//! here regardless of which container it believes it is addressing. This codec
//! preserves whatever byte arrives and never normalises, range-checks, or
//! defaults it.
//!
//! # A confirmed `WORD`-to-`BYTE` truncation in the caller, not reproduced here
//!
//! `TPacketCGItemMove::count` is a `WORD` at `packet.h:638`, but
//! `CSafebox::MoveItem`'s third parameter is a `BYTE` in **both** of its
//! profiles, at `server/server/game/safebox.cpp:179` and `:181`, and the call
//! at `input_main.cpp:2472` narrows sixteen bits to eight with no cast and no
//! check. That is consequential because `safebox.cpp:211-212` treats a count of
//! zero as "move the whole stack", so a transmitted `count` of 256 truncates to
//! zero and moves everything, and the guard at `safebox.cpp:197` is applied to
//! the already-truncated value. The header-13 path has no such truncation,
//! because `char.h:1209` takes a `WORD`. The stock client cannot trigger it,
//! since its builder parameter is already a `BYTE` at
//! `PythonNetworkStreamPhaseGameItem.cpp:68`, so the field can never exceed 255.
//! This is a caller-layer defect, recorded and deliberately **not** reproduced:
//! `count` is a `u16` here and is never truncated. In the inactive
//! non-extended profile the same call site is worse still, because
//! `safebox.cpp:181` narrows **both** `cell` words as well as the count into
//! `BYTE` parameters. That profile is unreachable here, since
//! `__EXTENDED_SAFEBOX__` is unconditionally defined at
//! `server/server/common/prodomodefines.h:61`, but it is recorded so the
//! difference between the two arms is not mistaken for one that only ever
//! affected the count.
//!
//! # Scope
//!
//! This module only validates and preserves the wire record. It does not move
//! an item, address a container grid, split or merge a stack, bound a cell, or
//! apply any of the gameplay rules that live above this boundary. Every
//! behaviour of `CSafebox::MoveItem` at `safebox.cpp:183-249` is out of scope,
//! including its two discarded `Remove` and `Add` results at `:244-245`, its
//! silently reduced merge count at `:214`, and the source-equals-destination
//! case: `:203` is only the self-merge guard `item != item2`, and the rejection
//! itself is the `IsEmpty` test at `:226-227`.

#![warn(missing_docs)]

use std::fmt;

use crate::cg_inventory::{CgHeader, HEADER_CG_SAFEBOX_ITEM_MOVE};
use crate::cg_item_move::CgItemPos;
use crate::cg_wire::ClientFrame;

/// The exact wire size of one packed legacy `TItemPos`, re-exported so a caller
/// of this record does not have to reach into the item-move module for it.
pub use crate::cg_item_move::CG_ITEM_POS_SIZE;

/// Fixed client-to-game payload size, excluding the one-byte header.
pub const CG_SAFEBOX_MOVE_PAYLOAD_SIZE: usize = 8;

/// Complete packed wire size, including the one-byte header.
pub const CG_SAFEBOX_MOVE_WIRE_SIZE: usize = 9;

fn check_exact(len: usize) -> Result<(), CgSafeboxMoveError> {
    use core::cmp::Ordering;
    match len.cmp(&CG_SAFEBOX_MOVE_WIRE_SIZE) {
        Ordering::Less => Err(CgSafeboxMoveError::Truncated {
            needed: CG_SAFEBOX_MOVE_WIRE_SIZE,
            available: len,
        }),
        Ordering::Greater => Err(CgSafeboxMoveError::LengthMismatch {
            expected: CG_SAFEBOX_MOVE_WIRE_SIZE,
            actual: len,
        }),
        Ordering::Equal => Ok(()),
    }
}

/// Why a safebox in-container move record could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgSafeboxMoveError {
    /// The input was shorter than the fixed nine bytes.
    Truncated {
        /// The fixed wire size that was required.
        needed: usize,
        /// How many bytes were actually available.
        available: usize,
    },
    /// The input was longer than the fixed nine bytes.
    LengthMismatch {
        /// The fixed wire size that was required.
        expected: usize,
        /// How many bytes were actually supplied.
        actual: usize,
    },
    /// The header byte was not 77.
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for CgSafeboxMoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "safebox move record needs {needed} bytes, got {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "safebox move record needs {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { actual: byte } => {
                write!(f, "header {byte:#04x} is not the safebox item-move header")
            }
        }
    }
}

impl std::error::Error for CgSafeboxMoveError {}

/// One fixed nine-byte client-to-game safebox in-container move record.
///
/// The record has a single encodable header, so no header field is stored, for
/// the same reason [`crate::cg_item_move::CgItemMove`] stores none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgSafeboxItemMove {
    /// Opaque source position.
    pub from: CgItemPos,
    /// Opaque destination position.
    pub to: CgItemPos,
    /// Opaque `WORD` count word, preserved at full width.
    pub count: u16,
}

impl CgSafeboxItemMove {
    /// Build a record without interpreting any field.
    #[must_use]
    pub const fn new(from: CgItemPos, to: CgItemPos, count: u16) -> Self {
        Self { from, to, count }
    }

    /// The one-byte header this record encodes.
    #[must_use]
    pub const fn header() -> CgHeader {
        HEADER_CG_SAFEBOX_ITEM_MOVE
    }

    /// Encode this record as the exact nine wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_SAFEBOX_MOVE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode this record into an existing buffer.
    ///
    /// The buffer must already have room for [`CG_SAFEBOX_MOVE_WIRE_SIZE`]
    /// bytes. The output is
    /// `[header][from window][from cell][to window][to cell][count little-endian]`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        self.from.encode_into(out);
        self.to.encode_into(out);
        out.extend_from_slice(&self.count.to_le_bytes());
    }

    /// Project this record as a `ClientFrame` with an exact eight-byte payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_SAFEBOX_MOVE_PAYLOAD_SIZE);
        self.from.encode_into(&mut payload);
        self.to.encode_into(&mut payload);
        payload.extend_from_slice(&self.count.to_le_bytes());
        ClientFrame::new(Self::header().value(), payload)
    }

    /// Decode one exact nine-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgSafeboxMoveError::Truncated`] for a short input,
    /// [`CgSafeboxMoveError::LengthMismatch`] for a long input, and
    /// [`CgSafeboxMoveError::InvalidHeader`] when the header is not 77. The
    /// length is checked before the header, and the header before any field.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgSafeboxMoveError> {
        check_exact(bytes.len())?;
        if bytes[0] != Self::header().value() {
            return Err(CgSafeboxMoveError::InvalidHeader { actual: bytes[0] });
        }
        Ok(Self {
            from: CgItemPos::decode_at(bytes, 1),
            to: CgItemPos::decode_at(bytes, 4),
            count: u16::from_le_bytes([bytes[7], bytes[8]]),
        })
    }

    /// Decode a `ClientFrame` with an exact eight-byte payload.
    ///
    /// # Errors
    ///
    /// The payload length is checked before the header, and the header before
    /// any field, so a malformed frame reports a length error rather than an
    /// invalid header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgSafeboxMoveError> {
        check_exact(frame.payload.len().saturating_add(1))?;
        if frame.header != Self::header().value() {
            return Err(CgSafeboxMoveError::InvalidHeader {
                actual: frame.header,
            });
        }
        Ok(Self {
            from: CgItemPos::decode_at(&frame.payload, 0),
            to: CgItemPos::decode_at(&frame.payload, 3),
            count: u16::from_le_bytes([frame.payload[6], frame.payload[7]]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};
    use std::error::Error;

    const FROM: CgItemPos = CgItemPos {
        window_type: 1,
        cell: 0,
    };
    const TO: CgItemPos = CgItemPos {
        window_type: 1,
        cell: 1,
    };

    fn rec(count: u16) -> CgSafeboxItemMove {
        CgSafeboxItemMove::new(FROM, TO, count)
    }

    /// The shared position size this record relies on must be exactly three
    /// bytes, and the re-exported constant must say so. Nothing else in the
    /// module would notice if either drifted.
    #[test]
    fn shared_position_size_is_three_bytes_on_both_sides() {
        assert_eq!(CG_ITEM_POS_SIZE, 3);
        assert_eq!(CG_ITEM_POS_SIZE, crate::cg_item_move::CG_ITEM_POS_SIZE);
    }

    /// The two public size constants are documentation, so a test has to pin
    /// them against real output. Without this, changing
    /// `CG_SAFEBOX_MOVE_PAYLOAD_SIZE` is invisible: it is only a capacity hint
    /// and a loop bound, so the whole workspace still passes.
    #[test]
    fn declared_sizes_match_the_bytes_actually_produced() {
        let r = rec(0);
        assert_eq!(r.encode().len(), CG_SAFEBOX_MOVE_WIRE_SIZE);
        assert_eq!(r.to_frame().payload.len(), CG_SAFEBOX_MOVE_PAYLOAD_SIZE);
        assert_eq!(r.encode().len() - 1, CG_SAFEBOX_MOVE_PAYLOAD_SIZE);
    }

    /// The stock client builder hardcodes both window bytes to `INVENTORY`, so
    /// this is the exact byte sequence a real safebox move produces.
    #[test]
    fn golden_bytes_match_the_stock_client_builder() {
        assert_eq!(
            rec(0).encode(),
            vec![0x4d, 0x01, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00]
        );
    }

    /// Every offset must be where the packed server struct puts it, and the
    /// record must be nine bytes in both trees.
    #[test]
    fn layout_matches_the_packed_server_and_client_structs() {
        let w = rec(0x0201).encode();
        assert_eq!(w.len(), CG_SAFEBOX_MOVE_WIRE_SIZE);
        assert_eq!(w[0], 0x4d, "header");
        assert_eq!(&w[1..4], &[0x01, 0x00, 0x00], "source window then cell");
        assert_eq!(
            &w[4..7],
            &[0x01, 0x01, 0x00],
            "destination window then cell"
        );
        assert_eq!(&w[7..9], &[0x01, 0x02], "count is little-endian");
    }

    /// This record reuses `sizeof(TPacketCGItemMove)`, so the live inventory
    /// must already agree that header 77 is a fixed nine-byte record.
    #[test]
    fn active_inventory_agrees_on_nine_bytes() {
        let size = resolve_client_frame_size(0x4d).expect("header 77 is registered");
        assert_eq!(size, ClientFrameSize::Fixed(CG_SAFEBOX_MOVE_WIRE_SIZE));
        assert_eq!(size, ClientFrameSize::Fixed(9));
        assert_eq!(
            resolve_client_frame_size(13).expect("header 13 is registered"),
            ClientFrameSize::Fixed(CG_SAFEBOX_MOVE_WIRE_SIZE),
            "header 13 shares the same struct and the same size"
        );
    }

    /// The count is a `WORD` on the wire. It must survive at full width,
    /// including the boundary values that a caller-side `BYTE` parameter
    /// would silently destroy.
    #[test]
    fn count_round_trips_at_full_word_width() {
        for count in [0u16, 1, 255, 256, 257, 511, 512, u16::MAX] {
            let r = rec(count);
            assert_eq!(CgSafeboxItemMove::decode(&r.encode()), Ok(r));
            assert_eq!(CgSafeboxItemMove::decode_frame(&r.to_frame()), Ok(r));
            assert_eq!(&r.encode()[7..9], &count.to_le_bytes());
        }
    }

    /// Both window bytes are opaque: no value may be rejected, and all 256
    /// must round-trip through both decoders. The server never reads them.
    #[test]
    fn both_window_bytes_stay_opaque() {
        for byte in 0u8..=255 {
            let r = CgSafeboxItemMove::new(
                CgItemPos {
                    window_type: byte,
                    cell: 0xffff,
                },
                CgItemPos {
                    window_type: byte,
                    cell: 0,
                },
                7,
            );
            let w = r.encode();
            assert_eq!(w[1], byte);
            assert_eq!(w[4], byte);
            assert_eq!(CgSafeboxItemMove::decode(&w), Ok(r));
            assert_eq!(CgSafeboxItemMove::decode_frame(&r.to_frame()), Ok(r));
        }
    }

    /// Position boundaries, including a zero cell and the legacy `0xffff`
    /// "no position" sentinel, must all survive.
    #[test]
    fn position_boundaries_round_trip() {
        for cell in [0u16, 1, 254, 255, 256, 269, 270, u16::MAX] {
            let r = CgSafeboxItemMove::new(
                CgItemPos {
                    window_type: 0,
                    cell,
                },
                CgItemPos {
                    window_type: 0xff,
                    cell,
                },
                1,
            );
            assert_eq!(CgSafeboxItemMove::decode(&r.encode()), Ok(r));
            assert_eq!(CgSafeboxItemMove::decode_frame(&r.to_frame()), Ok(r));
        }
    }

    /// Every single-byte header other than 77 must be rejected at the exact
    /// length. Testing only a short or long input would let a length-first
    /// decoder pass without ever reaching the header branch.
    #[test]
    fn wrong_headers_are_rejected_at_the_exact_length() {
        for byte in 0u8..=255 {
            if byte == 0x4d {
                continue;
            }
            let mut w = rec(0).encode();
            w[0] = byte;
            assert_eq!(
                CgSafeboxItemMove::decode(&w),
                Err(CgSafeboxMoveError::InvalidHeader { actual: byte })
            );
            let mut frame = rec(0).to_frame();
            frame.header = byte;
            assert_eq!(
                CgSafeboxItemMove::decode_frame(&frame),
                Err(CgSafeboxMoveError::InvalidHeader { actual: byte })
            );
        }
    }

    /// Headers 13 and 77 share a struct but are different records. Each codec
    /// must refuse the other's header, or the two would be interchangeable.
    #[test]
    fn header_13_and_header_77_are_not_interchangeable() {
        let thirteen = crate::cg_item_move::CgItemMove::new(FROM, TO, 1).encode();
        assert_eq!(thirteen[0], 13);
        assert_eq!(
            CgSafeboxItemMove::decode(&thirteen),
            Err(CgSafeboxMoveError::InvalidHeader { actual: 13 })
        );
        let seventy_seven = rec(1).encode();
        assert_eq!(
            crate::cg_item_move::CgItemMove::decode(&seventy_seven),
            Err(crate::cg_item_move::CgItemMoveError::InvalidHeader { actual: 0x4d })
        );
        assert_eq!(thirteen.len(), seventy_seven.len());
    }

    /// A short raw input is `Truncated` and a long one is
    /// `LengthMismatch`, both before the header is ever examined.
    #[test]
    fn raw_length_errors_take_precedence_over_the_header() {
        for len in 0..CG_SAFEBOX_MOVE_WIRE_SIZE {
            let w = vec![0xff; len];
            assert_eq!(
                CgSafeboxItemMove::decode(&w),
                Err(CgSafeboxMoveError::Truncated {
                    needed: CG_SAFEBOX_MOVE_WIRE_SIZE,
                    available: len
                }),
                "len {len} with a wrong header must still report Truncated"
            );
        }
        for extra in 1..5 {
            let mut w = rec(0).encode();
            w.extend(std::iter::repeat(0xaa).take(extra));
            let mut w2 = w.clone();
            w2[0] = 0xff;
            assert_eq!(
                CgSafeboxItemMove::decode(&w),
                Err(CgSafeboxMoveError::LengthMismatch {
                    expected: CG_SAFEBOX_MOVE_WIRE_SIZE,
                    actual: CG_SAFEBOX_MOVE_WIRE_SIZE + extra
                })
            );
            assert!(
                matches!(
                    CgSafeboxItemMove::decode(&w2),
                    Err(CgSafeboxMoveError::LengthMismatch { .. })
                ),
                "len {extra} too long with a wrong header must still report a length error"
            );
        }
    }

    /// The framed path checks the payload length before the header, and a
    /// short payload is `Truncated` while a long one is `LengthMismatch`.
    #[test]
    fn framed_length_errors_take_precedence_over_the_header() {
        for len in 0..CG_SAFEBOX_MOVE_PAYLOAD_SIZE {
            let frame = ClientFrame::new(0xff, vec![0xaa; len]);
            assert_eq!(
                CgSafeboxItemMove::decode_frame(&frame),
                Err(CgSafeboxMoveError::Truncated {
                    needed: CG_SAFEBOX_MOVE_WIRE_SIZE,
                    available: len + 1
                })
            );
        }
        for extra in 1..4 {
            let mut payload = rec(0).to_frame().payload;
            payload.extend_from_slice(&vec![0xaa; extra]);
            let frame = ClientFrame::new(0xff, payload);
            assert_eq!(
                CgSafeboxItemMove::decode_frame(&frame),
                Err(CgSafeboxMoveError::LengthMismatch {
                    expected: CG_SAFEBOX_MOVE_WIRE_SIZE,
                    actual: CG_SAFEBOX_MOVE_WIRE_SIZE + extra
                })
            );
        }
    }

    /// The header must be validated at the exact payload length, not skipped.
    #[test]
    fn framed_header_is_validated_at_the_exact_payload_length() {
        let mut frame = rec(0).to_frame();
        frame.header = 0x53;
        assert_eq!(
            CgSafeboxItemMove::decode_frame(&frame),
            Err(CgSafeboxMoveError::InvalidHeader { actual: 0x53 })
        );
        let mut frame = rec(0).to_frame();
        frame.header = 0x0d;
        assert_eq!(
            CgSafeboxItemMove::decode_frame(&frame),
            Err(CgSafeboxMoveError::InvalidHeader { actual: 0x0d })
        );
    }

    /// The header accessor and the encoders must agree, and must agree with
    /// the byte the record actually produces.
    #[test]
    fn header_accessor_matches_every_encoder_path() {
        let r = rec(0xbeef);
        assert_eq!(CgSafeboxItemMove::header(), HEADER_CG_SAFEBOX_ITEM_MOVE);
        assert_eq!(CgSafeboxItemMove::header().value(), 0x4d);
        assert_eq!(r.encode()[0], 0x4d);
        assert_eq!(r.to_frame().header, 0x4d);
        let mut buf = vec![0xcc];
        r.encode_into(&mut buf);
        assert_eq!(
            &buf,
            &[0xcc, 0x4d, 0x01, 0x00, 0x00, 0x01, 0x01, 0x00, 0xef, 0xbe]
        );
    }

    /// A record fed one byte at a time must decode identically to the same
    /// record fed in bulk, and a partial tail must be retained rather than
    /// dropped or reported early.
    #[test]
    fn fragmented_stream_retains_a_partial_tail() {
        let r = rec(0x0102);
        let wire = r.encode();
        let mut bulk = ClientFrameDecoder::new();
        bulk.feed(&wire).expect("bulk feed must fit");
        assert_eq!(bulk.try_decode(), Ok(Some(r.to_frame())));

        for split in 1..wire.len() {
            let mut d = ClientFrameDecoder::new();
            d.feed(&wire[..split]).expect("first half feeds");
            assert_eq!(d.try_decode(), Ok(None), "split {split} decoded early");
            d.feed(&wire[split..]).expect("second half feeds");
            assert_eq!(d.try_decode(), Ok(Some(r.to_frame())), "split {split}");
        }

        // Every strict prefix must be retained and eventually complete.
        let mut d = ClientFrameDecoder::new();
        for b in &wire[..wire.len() - 1] {
            d.feed(&[*b]).expect("single byte feeds");
            assert_eq!(d.try_decode(), Ok(None));
        }
        assert_eq!(d.buffered_len(), wire.len() - 1);
        d.feed(&[wire[wire.len() - 1]]).expect("final byte feeds");
        assert_eq!(d.try_decode(), Ok(Some(r.to_frame())));
        assert!(d.is_empty());
    }

    /// Two records back to back on one stream must both be recovered in
    /// order, so the codec cannot be quietly stateful.
    #[test]
    fn two_records_stream_back_to_back_in_order() {
        let a = rec(1);
        let b = CgSafeboxItemMove::new(
            CgItemPos {
                window_type: 9,
                cell: 44,
            },
            CgItemPos {
                window_type: 3,
                cell: 7,
            },
            256,
        );
        let mut wire = a.encode();
        wire.extend_from_slice(&b.encode());
        let mut d = ClientFrameDecoder::new();
        d.feed(&wire).expect("two-record feed must fit");
        assert_eq!(d.try_decode(), Ok(Some(a.to_frame())));
        assert_eq!(d.try_decode(), Ok(Some(b.to_frame())));
        assert_eq!(d.try_decode(), Ok(None));
    }

    /// The error type must stay usable, including as a `std::error::Error`.
    #[test]
    fn error_display_is_stable_and_the_type_is_an_error() {
        let cases = [
            (
                CgSafeboxMoveError::Truncated {
                    needed: 9,
                    available: 3,
                },
                "safebox move record needs 9 bytes, got 3",
            ),
            (
                CgSafeboxMoveError::LengthMismatch {
                    expected: 9,
                    actual: 11,
                },
                "safebox move record needs 9 bytes, got 11",
            ),
            (
                CgSafeboxMoveError::InvalidHeader { actual: 0x53 },
                "header 0x53 is not the safebox item-move header",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            let as_dyn: &dyn Error = &error;
            assert_eq!(as_dyn.to_string(), text);
        }
    }
}
