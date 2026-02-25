//! Explicit codec for the fixed legacy `TPacketCGQuickslotAdd` record.
//!
//! `server/server/game/packet.h:25` sets `HEADER_CG_QUICKSLOT_ADD = 16` and
//! `packet.h:664-669` declares `command_quickslot_add` as a `BYTE header`, a
//! `BYTE pos`, and a trailing `TQuickslot slot`. The record sits under the
//! `#pragma pack(1)` that `packet.h:274` opens and does not close until
//! `:3540`, and `TQuickslot` is itself packed, so the record is exactly **four
//! bytes** and its framed payload **three**. Nothing in the declaration is
//! conditional, so the width is not profile-dependent.
//!
//! `TQuickslot` is `SQuickslot { BYTE type; BYTE pos; }` at
//! `server/server/common/tables.h:452-456`, inside the `#pragma pack(1)` opened
//! at `common/tables.h:345` and closed at `:2291`. It is therefore exactly two
//! bytes, which is what makes the containing record four.
//!
//! The record is registered exactly once, at
//! `server/server/game/packet_info.cpp:125`, as
//! `Set(HEADER_CG_QUICKSLOT_ADD, sizeof(TPacketCGQuickslotAdd), "QuickslotAdd")`
//! inside `CPacketInfoCG`'s constructor. The four-byte size there agrees with
//! the hand-derived width, and the existing `protocol/src/cg_inventory.rs` row
//! already carried `base_size: 4` with
//! `cpp_type: Some("TPacketCGQuickslotAdd")`, so no inventory change was
//! needed.
//!
//! # Two different fields are both called `pos`
//!
//! The record has a bare `pos` at wire offset 1 and a `slot.pos` at offset 3,
//! and they mean different things. The bare `pos` is the **quickbar index**:
//! it is passed to `CHARACTER::SetQuickslot(BYTE pos, ...)` at
//! `server/server/game/input_main.cpp:1112` and indexes
//! `CHARACTER::m_quickslot[QUICKSLOT_MAX_NUM]` at `char.h:1043`. The
//! `slot.pos` is a **payload** position, and when the slot type is an item it is
//! used as an inventory cell, as `TItemPos srcCell(INVENTORY, pinfo->slot.pos)`
//! at `input_main.cpp:1093`. Neither is derivable from the other, so the struct
//! names them `pos` and `slot` to keep the two apart.
//!
//! # Every bound on these fields lives above the codec
//!
//! The dispatch at `server/server/game/input_main.cpp:3742-3744` is inside
//! `CInputMain::Analyze` and leaves `iExtraLen` at zero, so header 16 is not a
//! variable header. `CInputMain::QuickslotAdd` is declared `void` at
//! `input.h:94` and defined at `input_main.cpp:1083-1113`; it checks only
//! `if (!ch) return;` and then `if (pinfo->slot.type == QUICKSLOT_TYPE_ITEM)`.
//! Every real bound is one layer further up, in
//! `CHARACTER::SetQuickslot` at `char_quickslot.cpp:45`:
//!
//! - `if (pos >= QUICKSLOT_MAX_NUM) return false;`, where `QUICKSLOT_MAX_NUM` is
//!   36 at `server/server/common/length.h:51`.
//! - `if (rSlot.type >= QUICKSLOT_TYPE_MAX_NUM) return false;`, where the enum
//!   at `common/length.h:374-381` is `QUICKSLOT_TYPE_NONE` 0, `_ITEM` 1,
//!   `_SKILL` 2, `_COMMAND` 3, `_MAX_NUM` 4.
//! - a second switch that requires an item slot to name a default or belt
//!   inventory position, a skill slot to name a position below `SKILL_MAX_NUM`,
//!   and whose `default:` arm returns false.
//!
//! So the wire `pos` and `slot.type` are **opaque** `u8` values here. This
//! boundary does not apply `QUICKSLOT_MAX_NUM`, does not apply
//! `QUICKSLOT_TYPE_MAX_NUM`, does not require the type to be a known enumerator,
//! and does not resolve the slot payload. Adding any of those rules would
//! silently reject wire values the legacy framer accepts, and would put gameplay
//! policy inside a record codec.
//!
//! # The dispatch has no observer guard, unlike its neighbours
//!
//! The `case HEADER_CG_QUICKSLOT_ADD:` arm at `input_main.cpp:3742-3744` calls
//! `QuickslotAdd(ch, c_pData)` with no `IsObserverMode()` test, while the
//! neighbouring `HEADER_CG_ITEM_PICKUP` arm at `:3715-3718` and the
//! `HEADER_CG_USE_SKILL` arm at `:3739-3741` both do. This is recorded as a
//! legacy observation about the dispatch layer. It is not reproduced, and it is
//! not a reason to add anything to the codec.
//!
//! The `if (!ch)` test inside the handler is also redundant with `Analyze`, which
//! closes the phase when `d->GetCharacter()` is null. Unlike the pickup handler,
//! this one does get that redundant test; the two siblings
//! `QuickslotDelete` at `input_main.cpp:1116-1120` and `QuickslotSwap` at
//! `:1122-1126` do **not**, and dereference `ch` directly. That asymmetry is
//! recorded here because those two records are the natural next slices and the
//! difference will otherwise look like an oversight.

use crate::cg_inventory::{CgHeader, HEADER_CG_QUICKSLOT_ADD};
use crate::cg_wire::ClientFrame;
use crate::TQuickslot;

/// The complete on-wire size of a `TPacketCGQuickslotAdd`, including the header.
pub const CG_QUICKSLOT_ADD_WIRE_SIZE: usize = 4;

/// The number of bytes after the one-byte header.
pub const CG_QUICKSLOT_ADD_PAYLOAD_SIZE: usize = 3;

/// Errors returned by the `TPacketCGQuickslotAdd` codec.
///
/// The length is always checked before the header byte, and the header byte is
/// always checked before any payload byte is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CgQuickslotAddError {
    /// The input ended before the complete record was available.
    Truncated {
        /// The exact wire size that was required.
        needed: usize,
        /// The number of bytes that were actually available.
        available: usize,
    },
    /// The input carried more bytes than the fixed record can hold.
    LengthMismatch {
        /// The exact wire size the record requires.
        expected: usize,
        /// The number of bytes that were actually supplied.
        actual: usize,
    },
    /// The leading byte was not [`HEADER_CG_QUICKSLOT_ADD`].
    InvalidHeader {
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl std::fmt::Display for CgQuickslotAddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated TPacketCGQuickslotAdd: need {needed} bytes, got {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "TPacketCGQuickslotAdd must be exactly {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { actual } => {
                write!(
                    f,
                    "expected header {HEADER_CG_QUICKSLOT_ADD:?}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for CgQuickslotAddError {}

/// A request to place one entry on a quickbar slot.
///
/// The header is fixed and is never stored as a field: it is always emitted as
/// byte 16 and is accepted only as 16. `pos` is the quickbar index and
/// `slot` is the entry to store; neither is range-checked here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgQuickslotAdd {
    /// The quickbar index, opaque and unchecked.
    pub pos: u8,
    /// The entry to store, opaque and unchecked.
    pub slot: TQuickslot,
}

impl CgQuickslotAdd {
    /// Build a request for the given quickbar index and entry.
    #[must_use]
    pub const fn new(pos: u8, slot: TQuickslot) -> Self {
        Self { pos, slot }
    }

    /// The one-byte header this record encodes.
    #[must_use]
    pub const fn header() -> CgHeader {
        HEADER_CG_QUICKSLOT_ADD
    }

    /// Encode this request as the exact four wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_QUICKSLOT_ADD_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode this request into an existing buffer.
    ///
    /// The output is exactly `[0x10][pos][slot type][slot pos]`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.pos);
        out.push(self.slot.b_type);
        out.push(self.slot.b_pos);
    }

    /// Project this request as a `ClientFrame` with an exact three-byte
    /// payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_QUICKSLOT_ADD_PAYLOAD_SIZE);
        payload.push(self.pos);
        payload.push(self.slot.b_type);
        payload.push(self.slot.b_pos);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// Decode one exact four-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgQuickslotAddError::Truncated`] for a short input,
    /// [`CgQuickslotAddError::LengthMismatch`] for a long input, and
    /// [`CgQuickslotAddError::InvalidHeader`] when the header is not 16. The
    /// length is checked before the header, and the header is checked before
    /// any payload byte is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgQuickslotAddError> {
        check_exact(bytes.len())?;
        check_header(bytes[0])?;
        Ok(Self {
            pos: bytes[1],
            slot: TQuickslot {
                b_type: bytes[2],
                b_pos: bytes[3],
            },
        })
    }

    /// Decode one framed request.
    ///
    /// # Errors
    ///
    /// Returns [`CgQuickslotAddError::Truncated`] for a payload shorter than
    /// three bytes, [`CgQuickslotAddError::LengthMismatch`] for a longer
    /// payload, and [`CgQuickslotAddError::InvalidHeader`] when the frame header
    /// is not 16. The payload length is checked before the header, and the
    /// header is checked before any payload byte is read.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgQuickslotAddError> {
        let len = frame.payload.len();
        match len.cmp(&CG_QUICKSLOT_ADD_PAYLOAD_SIZE) {
            core::cmp::Ordering::Less => {
                return Err(CgQuickslotAddError::Truncated {
                    needed: CG_QUICKSLOT_ADD_PAYLOAD_SIZE,
                    available: len,
                })
            }
            core::cmp::Ordering::Greater => {
                return Err(CgQuickslotAddError::LengthMismatch {
                    expected: CG_QUICKSLOT_ADD_PAYLOAD_SIZE,
                    actual: len,
                })
            }
            core::cmp::Ordering::Equal => {}
        }
        check_header(frame.header)?;
        Ok(Self {
            pos: frame.payload[0],
            slot: TQuickslot {
                b_type: frame.payload[1],
                b_pos: frame.payload[2],
            },
        })
    }
}

fn check_exact(len: usize) -> Result<(), CgQuickslotAddError> {
    if len < CG_QUICKSLOT_ADD_WIRE_SIZE {
        return Err(CgQuickslotAddError::Truncated {
            needed: CG_QUICKSLOT_ADD_WIRE_SIZE,
            available: len,
        });
    }
    if len > CG_QUICKSLOT_ADD_WIRE_SIZE {
        return Err(CgQuickslotAddError::LengthMismatch {
            expected: CG_QUICKSLOT_ADD_WIRE_SIZE,
            actual: len,
        });
    }
    Ok(())
}

fn check_header(actual: u8) -> Result<(), CgQuickslotAddError> {
    if actual != HEADER_CG_QUICKSLOT_ADD.value() {
        return Err(CgQuickslotAddError::InvalidHeader { actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CgQuickslotAdd, CgQuickslotAddError, CG_QUICKSLOT_ADD_PAYLOAD_SIZE,
        CG_QUICKSLOT_ADD_WIRE_SIZE,
    };
    use crate::cg_inventory::HEADER_CG_QUICKSLOT_ADD;
    use crate::cg_wire::ClientFrame;
    use crate::TQuickslot;

    fn slot(b_type: u8, b_pos: u8) -> TQuickslot {
        TQuickslot { b_type, b_pos }
    }

    /// The five wire bytes the legacy server would receive.
    fn wire(pos: u8, b_type: u8, b_pos: u8) -> Vec<u8> {
        vec![16, pos, b_type, b_pos]
    }

    #[test]
    fn constants_match_the_source() {
        assert_eq!(HEADER_CG_QUICKSLOT_ADD.value(), 16);
        assert_eq!(CgQuickslotAdd::header().value(), 16);
        // packet.h:664-669 -> BYTE header, BYTE pos, TQuickslot slot.
        assert_eq!(CG_QUICKSLOT_ADD_WIRE_SIZE, 4);
        assert_eq!(CG_QUICKSLOT_ADD_PAYLOAD_SIZE, 3);
        assert_eq!(
            CG_QUICKSLOT_ADD_WIRE_SIZE,
            1 + CG_QUICKSLOT_ADD_PAYLOAD_SIZE
        );
        // TQuickslot is the two-byte SQuickslot at common/tables.h:452-456.
        assert_eq!(<TQuickslot as crate::PacketSerialize>::packed_size(), 2);
    }

    #[test]
    fn golden_bytes_for_a_known_slot() {
        // Quickbar index 3, an item slot pointing at inventory cell 12.
        let rec = CgQuickslotAdd::new(3, slot(1, 12));
        assert_eq!(rec.encode(), vec![16, 3, 1, 12]);
        assert_eq!(wire(3, 1, 12), rec.encode());
    }

    /// The two `pos` fields must not be swapped or merged. This pins the exact
    /// order: quickbar index first, then the slot's own position.
    #[test]
    fn the_bare_pos_precedes_the_slot_pos() {
        assert_eq!(
            CgQuickslotAdd::new(7, slot(9, 2)).encode(),
            vec![16, 7, 9, 2]
        );
        // Swapping them must produce different bytes.
        assert_ne!(
            CgQuickslotAdd::new(7, slot(9, 2)).encode(),
            CgQuickslotAdd::new(2, slot(9, 7)).encode()
        );
    }

    /// Both fields are opaque `u8`. The bounds `QUICKSLOT_MAX_NUM` (36) and
    /// `QUICKSLOT_TYPE_MAX_NUM` (4) live in `SetQuickslot`, one layer above, so
    /// every value must round-trip here.
    #[test]
    fn no_pos_or_type_value_is_rejected() {
        let edges = [0u8, 1, 3, 4, 5, 34, 35, 36, 37, 200, 254, 255];
        for pos in edges {
            for b_type in edges {
                let rec = CgQuickslotAdd::new(pos, slot(b_type, 255));
                let bytes = rec.encode();
                assert_eq!(bytes.len(), CG_QUICKSLOT_ADD_WIRE_SIZE);
                assert_eq!(bytes[0], 16);
                assert_eq!(bytes[1], pos, "pos {pos}");
                assert_eq!(bytes[2], b_type, "type {b_type}");
                assert_eq!(CgQuickslotAdd::decode(&bytes).expect("d"), rec);
                assert_eq!(
                    CgQuickslotAdd::decode_frame(&rec.to_frame()).expect("f"),
                    rec
                );
            }
        }
    }

    #[test]
    fn a_dense_sweep_round_trips() {
        for v in 0u32..1024 {
            let pos = (v & 0xff) as u8;
            let b_type = ((v >> 2) & 0xff) as u8;
            let b_pos = ((v >> 4) & 0x0f) as u8;
            let rec = CgQuickslotAdd::new(pos, slot(b_type, b_pos));
            assert_eq!(CgQuickslotAdd::decode(&rec.encode()).expect("d"), rec);
        }
    }

    #[test]
    fn length_is_checked_before_the_header() {
        for first in [0x00u8, 0x10, 0xff] {
            for len in 0..CG_QUICKSLOT_ADD_WIRE_SIZE {
                let mut buf = vec![first];
                buf.resize(len, 0xcc);
                assert_eq!(
                    CgQuickslotAdd::decode(&buf),
                    Err(CgQuickslotAddError::Truncated {
                        needed: CG_QUICKSLOT_ADD_WIRE_SIZE,
                        available: len,
                    }),
                    "first {first:#x} len {len}"
                );
            }
        }
    }

    #[test]
    fn a_long_input_is_a_length_mismatch() {
        for len in (CG_QUICKSLOT_ADD_WIRE_SIZE + 1)..(CG_QUICKSLOT_ADD_WIRE_SIZE + 6) {
            let mut buf = wire(1, 1, 1);
            buf.resize(len, 0xdd);
            assert_eq!(
                CgQuickslotAdd::decode(&buf),
                Err(CgQuickslotAddError::LengthMismatch {
                    expected: CG_QUICKSLOT_ADD_WIRE_SIZE,
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
            let buf = vec![first, 1, 2, 3];
            match CgQuickslotAdd::decode(&buf) {
                Ok(rec) => {
                    assert_eq!(first, 16);
                    assert_eq!(rec.pos, 1);
                    assert_eq!(rec.slot.b_type, 2);
                    assert_eq!(rec.slot.b_pos, 3);
                    accepted += 1;
                }
                Err(CgQuickslotAddError::InvalidHeader { actual }) => assert_eq!(actual, first),
                Err(other) => panic!("header {first:#x} gave {other:?}"),
            }
        }
        assert_eq!(accepted, 1);
    }

    #[test]
    fn a_wrong_header_with_an_exact_length_is_rejected() {
        for bad in [0x00u8, 0x0f, 0x11, 0x14, 0xff] {
            let mut buf = wire(0, 0, 0);
            buf[0] = bad;
            assert_eq!(
                CgQuickslotAdd::decode(&buf),
                Err(CgQuickslotAddError::InvalidHeader { actual: bad }),
                "header {bad:#x}"
            );
        }
    }

    #[test]
    fn the_framed_payload_length_is_checked_before_the_frame_header() {
        let frame = ClientFrame::new(0xff, vec![1, 2]);
        assert_eq!(
            CgQuickslotAdd::decode_frame(&frame),
            Err(CgQuickslotAddError::Truncated {
                needed: CG_QUICKSLOT_ADD_PAYLOAD_SIZE,
                available: 2,
            })
        );
        let frame = ClientFrame::new(0xff, vec![1, 2, 3, 4]);
        assert_eq!(
            CgQuickslotAdd::decode_frame(&frame),
            Err(CgQuickslotAddError::LengthMismatch {
                expected: CG_QUICKSLOT_ADD_PAYLOAD_SIZE,
                actual: 4,
            })
        );
        let frame = ClientFrame::new(0xff, vec![1, 2, 3]);
        assert_eq!(
            CgQuickslotAdd::decode_frame(&frame),
            Err(CgQuickslotAddError::InvalidHeader { actual: 0xff })
        );
        let frame = ClientFrame::new(0xff, Vec::new());
        assert_eq!(
            CgQuickslotAdd::decode_frame(&frame),
            Err(CgQuickslotAddError::Truncated {
                needed: CG_QUICKSLOT_ADD_PAYLOAD_SIZE,
                available: 0,
            })
        );
    }

    #[test]
    fn the_header_is_fixed_and_never_stored() {
        for (pos, b_type, b_pos) in [(0u8, 0u8, 0u8), (1, 1, 1), (255, 255, 255)] {
            let rec = CgQuickslotAdd::new(pos, slot(b_type, b_pos));
            assert_eq!(rec.encode()[0], 16);
            assert_eq!(rec.to_frame().header, 16);
        }
    }

    #[test]
    fn encode_into_appends_rather_than_replaces() {
        let mut out = vec![0xde, 0xad];
        CgQuickslotAdd::new(3, slot(1, 12)).encode_into(&mut out);
        assert_eq!(out, vec![0xde, 0xad, 16, 3, 1, 12]);
    }

    /// The shared two-byte primitive must agree with this record about where
    /// its bytes land.
    #[test]
    fn the_slot_primitive_agrees_with_the_record() {
        let s = slot(0xab, 0xcd);
        let rec = CgQuickslotAdd::new(0x12, s);
        let bytes = rec.encode();
        let own = <TQuickslot as crate::PacketSerialize>::to_bytes(&s);
        assert_eq!(own, vec![0xab, 0xcd]);
        assert_eq!(&bytes[2..], own.as_slice());
        // And the same slice decodes back through the primitive.
        let back = <TQuickslot as crate::PacketSerialize>::from_bytes(&bytes[2..]).expect("d");
        assert_eq!(back, s);
    }

    #[test]
    fn errors_display_without_panicking() {
        let cases = [
            CgQuickslotAddError::Truncated {
                needed: 4,
                available: 1,
            },
            CgQuickslotAddError::LengthMismatch {
                expected: 4,
                actual: 8,
            },
            CgQuickslotAddError::InvalidHeader { actual: 0x11 },
        ];
        for e in cases {
            let s = e.to_string();
            assert!(!s.is_empty());
            let dyn_err: &dyn std::error::Error = &e;
            assert!(!dyn_err.to_string().is_empty());
        }
    }

    #[test]
    fn it_decodes_through_the_fixed_frame_decoder() {
        use crate::cg_wire::ClientFrameDecoder;
        let cases = [(0u8, 0u8, 0u8), (1, 1, 1), (36, 4, 255), (255, 255, 255)];
        let mut stream = Vec::new();
        for (p, t, sp) in cases {
            stream.extend_from_slice(&wire(p, t, sp));
        }
        let mut dec = ClientFrameDecoder::new();
        dec.feed(&stream).expect("feed");
        let mut got = Vec::new();
        while let Some(frame) = dec.try_decode().expect("decode") {
            assert_eq!(frame.header, 16);
            got.push(CgQuickslotAdd::decode_frame(&frame).expect("codec"));
        }
        let want: Vec<_> = cases
            .iter()
            .map(|(p, t, sp)| CgQuickslotAdd::new(*p, slot(*t, *sp)))
            .collect();
        assert_eq!(got, want);
        assert!(dec.is_empty());
    }

    /// The three quickslot records sit at consecutive headers with widths 4, 2
    /// and 3. They must not be confusable.
    #[test]
    fn the_quickslot_siblings_do_not_share_this_header() {
        assert_ne!(HEADER_CG_QUICKSLOT_ADD.value(), 17);
        assert_ne!(HEADER_CG_QUICKSLOT_ADD.value(), 18);
        assert_eq!(CG_QUICKSLOT_ADD_WIRE_SIZE, 4);
    }
}
