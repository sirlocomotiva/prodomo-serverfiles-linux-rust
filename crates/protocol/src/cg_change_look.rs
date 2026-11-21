//! The 10-byte `ChangeLook` record: a header, a sub-header, a cost, a slot, and
//! the shared 3-byte position.
//!
//! ```c
//! typedef struct SPacketChangeLook
//! {
//!     BYTE    header;
//!     BYTE    subheader;
//!     DWORD    dwCost;
//!     BYTE    bPos;
//!     TItemPos    tPos;
//! } TPacketChangeLook;
//! ```
//!
//! # Width
//!
//! `1 + 1 + 4 + 1 + 3 = 10`, which closes against
//! `Set(HEADER_CG_CL, sizeof(TPacketChangeLook), "ChangeLook")` at
//! `packet_info.cpp:181`. The declaration is **byte-identical** in both trees --
//! `server/server/game/packet.h:2837-2859` and
//! `client/Client/UserInterface/Packet.h`, both under the same header enum --
//! and it has no `#ifdef` inside.
//!
//! `__CHANGELOOK_SYSTEM__` is `#define`d at
//! `server/server/common/prodomodefines.h:16` and `ENABLE_CHANGELOOK_SYSTEM` at
//! `client/Client/UserInterface/LOCALE_INC.H:26`, so the record is in the active
//! build of both. Dispatch is live at `input_main.cpp:3857-3861`.
//!
//! **Packing is load-bearing here.** Unpacked, the `DWORD dwCost` at offset 2
//! wants 4-byte alignment and the struct becomes 12 bytes. The workspace
//! `#pragma pack(1)` is what makes it 10. Stated rather than assumed, because the
//! same reasoning is *not* valid for `TAttr67AddData`.
//!
//! # This is the fourth consumer of the shared `TItemPos`
//!
//! `tPos` is `TItemPos`, so it is [`ItemPos`] from [`crate::item_pos`] -- the same
//! 3-byte packed `BYTE window_type` + little-endian `WORD cell` as `cg_sash`,
//! `cg_dragon_soul`, and `cg_exchange`. It is a fourth independent witness for
//! that width: `cg_sash` at 23 bytes, `cg_dragon_soul` at 47, and here at 10 all
//! close only if `TItemPos` is 3.
//!
//! # `bPos` and `tPos` are different positions
//!
//! `AddClMaterial(sPacket->tPos, sPacket->bPos)` passes **both**, in that order.
//! `bPos` is a `BYTE` and `tPos` is a `TItemPos`; they are not two views of one
//! index, and the codec keeps them as separate fields rather than unifying them.
//! What either one *means* is inventory policy: `sPacket->subheader` selects
//! between them, and `CL_SUBHEADER_REMOVE` uses only `bPos`.
//!
//! # The sub-header is a shared enum with a `default:`
//!
//! `CL_SUBHEADER_OPEN`, `CL_SUBHEADER_CLOSE`, `CL_SUBHEADER_ADD`,
//! `CL_SUBHEADER_REMOVE`, `CL_SUBHEADER_REFINE` at `packet.h:2838-2842`. The
//! handler's `switch` has a `default:` arm, so unknown values are absorbed. The
//! field stays an opaque `u8`.

use crate::cg_inventory::{CgHeader, HEADER_CG_CL};
use crate::cg_wire::ClientFrame;
use crate::item_pos::ItemPos;

/// The full legacy `TPacketChangeLook` record, header byte included.
pub const CG_CHANGE_LOOK_WIRE_SIZE: usize = 1 + 1 + 4 + 1 + ItemPos::WIRE_SIZE;
/// The framed payload of `TPacketChangeLook`.
pub const CG_CHANGE_LOOK_PAYLOAD_SIZE: usize = CG_CHANGE_LOOK_WIRE_SIZE - 1;

/// Every way the change-look decoder can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgChangeLookError {
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

impl core::fmt::Display for CgChangeLookError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated change-look: need {needed} bytes, got {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "change-look length mismatch: expected {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(
                    f,
                    "invalid change-look header: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for CgChangeLookError {}

/// The 10-byte `ChangeLook` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgChangeLook {
    /// The legacy `BYTE subheader`, a shared-enum value.
    pub subheader: u8,
    /// The legacy `DWORD dwCost`, little-endian.
    pub dw_cost: u32,
    /// The legacy `BYTE bPos`, a second position distinct from `tpos`.
    pub b_pos: u8,
    /// The legacy `TItemPos tPos`, the shared packed 3-byte position.
    pub t_pos: ItemPos,
}

impl CgChangeLook {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_CL
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_CHANGE_LOOK_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_CHANGE_LOOK_PAYLOAD_SIZE;

    /// Build the record.
    pub const fn new(subheader: u8, dw_cost: u32, b_pos: u8, t_pos: ItemPos) -> Self {
        Self {
            subheader,
            dw_cost,
            b_pos,
            t_pos,
        }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.subheader);
        out.extend_from_slice(&self.dw_cost.to_le_bytes());
        out.push(self.b_pos);
        self.t_pos.encode_into(out);
    }

    /// Encode to a fresh 10-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 9-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(Self::PAYLOAD_SIZE);
        payload.push(self.subheader);
        payload.extend_from_slice(&self.dw_cost.to_le_bytes());
        payload.push(self.b_pos);
        self.t_pos.encode_into(&mut payload);
        ClientFrame {
            header: Self::header().value(),
            payload,
        }
    }

    /// # Errors
    ///
    /// [`CgChangeLookError::Truncated`] below 10 bytes,
    /// [`CgChangeLookError::LengthMismatch`] above,
    /// [`CgChangeLookError::InvalidHeader`] for a full-length slice not starting
    /// with 233.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgChangeLookError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(CgChangeLookError::Truncated {
                needed: Self::WIRE_SIZE,
                available: bytes.len(),
            });
        }
        if bytes.len() > Self::WIRE_SIZE {
            return Err(CgChangeLookError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header().value() {
            return Err(CgChangeLookError::InvalidHeader {
                expected: Self::header().value(),
                actual: bytes[0],
            });
        }
        Ok(Self {
            subheader: bytes[1],
            dw_cost: u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            b_pos: bytes[6],
            t_pos: ItemPos::decode_at(bytes, 7),
        })
    }

    /// # Errors
    ///
    /// As [`CgChangeLook::decode`], except that the payload must be exactly 9
    /// bytes. The `TItemPos` bytes are read at payload offset 6.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgChangeLookError> {
        if frame.payload.len() < Self::PAYLOAD_SIZE {
            return Err(CgChangeLookError::Truncated {
                needed: Self::PAYLOAD_SIZE,
                available: frame.payload.len(),
            });
        }
        if frame.payload.len() > Self::PAYLOAD_SIZE {
            return Err(CgChangeLookError::LengthMismatch {
                expected: Self::PAYLOAD_SIZE,
                actual: frame.payload.len(),
            });
        }
        if frame.header != Self::header().value() {
            return Err(CgChangeLookError::InvalidHeader {
                expected: Self::header().value(),
                actual: frame.header,
            });
        }
        Ok(Self {
            subheader: frame.payload[0],
            dw_cost: u32::from_le_bytes([
                frame.payload[1],
                frame.payload[2],
                frame.payload[3],
                frame.payload[4],
            ]),
            b_pos: frame.payload[5],
            t_pos: ItemPos::decode_at(&frame.payload, 6),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u8 = 233;

    #[test]
    fn the_header_is_233() {
        assert_eq!(CgChangeLook::header().value(), H);
        assert_eq!(H, 0xe9);
    }

    #[test]
    fn the_record_is_ten_bytes() {
        assert_eq!(1 + 1 + 4 + 1 + 3, 10);
        assert_eq!(CG_CHANGE_LOOK_WIRE_SIZE, 10);
        assert_eq!(CgChangeLook::WIRE_SIZE, 10);
        assert_eq!(CgChangeLook::PAYLOAD_SIZE, 9);
    }

    #[test]
    fn packing_is_load_bearing_here() {
        // Unpacked: header@0, subheader@1, two pad bytes, dwCost@4..8, bPos@8,
        // tPos@9..12, then the struct rounds up to its alignment of 4.
        let unpacked = 4 + 4 + 1 + ItemPos::WIRE_SIZE;
        assert_eq!(unpacked, 12);
        assert_ne!(unpacked, CgChangeLook::WIRE_SIZE);
        // Packed, the two pad bytes and the trailing round-up both disappear.
        assert_eq!(CgChangeLook::WIRE_SIZE, 10);
    }

    #[test]
    fn encodes_to_the_exact_legacy_bytes() {
        let r = CgChangeLook::new(0, 0, 0, ItemPos::new(0, 0));
        assert_eq!(r.encode(), vec![0xe9, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn the_cost_is_a_little_endian_dword_at_offset_two() {
        let r = CgChangeLook::new(1, 0x0102_0304, 9, ItemPos::new(0xAB, 0xCDEF));
        let b = r.encode();
        assert_eq!(b.len(), 10);
        assert_eq!(b[0], H);
        assert_eq!(b[1], 1);
        assert_eq!(&b[2..6], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(b[6], 9);
        assert_eq!(&b[7..10], &[0xAB, 0xEF, 0xCD]);
    }

    #[test]
    fn the_full_cost_range_round_trips() {
        for v in [0u32, 1, 0xFFFF, 0x1_0000, u32::MAX] {
            let r = CgChangeLook::new(0, v, 0, ItemPos::new(0, 0));
            assert_eq!(CgChangeLook::decode(&r.encode()).unwrap().dw_cost, v);
        }
    }

    #[test]
    fn every_payload_byte_is_read() {
        let base = CgChangeLook::default();
        let mut changed = 0;
        for i in 1..10 {
            let mut b = base.encode();
            b[i] = b[i].wrapping_add(0x11);
            if CgChangeLook::decode(&b).unwrap() != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 9);
    }

    #[test]
    fn the_two_positions_stay_distinct() {
        // AddClMaterial(tPos, bPos) uses both, so they cannot be merged.
        let a = CgChangeLook::new(0, 0, 5, ItemPos::new(1, 2));
        let b = CgChangeLook::new(0, 0, 2, ItemPos::new(1, 5));
        assert_ne!(a, b);
        assert_eq!(CgChangeLook::decode(&a.encode()).unwrap(), a);
        assert_eq!(CgChangeLook::decode(&b.encode()).unwrap(), b);
    }

    #[test]
    fn every_subheader_round_trips() {
        for v in 0..=255_u8 {
            let r = CgChangeLook::new(v, 0, 0, ItemPos::new(0, 0));
            assert_eq!(CgChangeLook::decode(&r.encode()).unwrap().subheader, v);
        }
    }

    #[test]
    fn every_window_type_round_trips() {
        for w in 0..=255_u8 {
            let r = CgChangeLook::new(0, 0, 0, ItemPos::new(w, 0));
            assert_eq!(
                CgChangeLook::decode(&r.encode()).unwrap().t_pos.window_type,
                w
            );
        }
    }

    #[test]
    fn round_trips() {
        let r = CgChangeLook::new(3, 0xCAFE_BABE, 0x77, ItemPos::new(4, 5));
        assert_eq!(CgChangeLook::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn round_trips_through_a_frame() {
        let r = CgChangeLook::new(3, 1, 2, ItemPos::new(4, 5));
        let f = r.to_frame();
        assert_eq!(f.header, H);
        assert_eq!(f.payload.len(), 9);
        assert_eq!(CgChangeLook::decode_frame(&f).unwrap(), r);
        assert_eq!(&f.payload[..], &r.encode()[1..]);
    }

    #[test]
    fn rejects_every_short_length() {
        for len in 0..10 {
            let mut b = CgChangeLook::new(1, 2, 3, ItemPos::new(4, 5)).encode();
            b.truncate(len);
            assert_eq!(
                CgChangeLook::decode(&b).unwrap_err(),
                CgChangeLookError::Truncated {
                    needed: 10,
                    available: len
                },
                "len {len}"
            );
        }
    }

    #[test]
    fn rejects_every_long_length() {
        for extra in 1..=3 {
            let mut b = CgChangeLook::new(1, 2, 3, ItemPos::new(4, 5)).encode();
            b.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgChangeLook::decode(&b).unwrap_err(),
                CgChangeLookError::LengthMismatch {
                    expected: 10,
                    actual: 10 + extra
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
            let mut b = CgChangeLook::new(1, 2, 3, ItemPos::new(4, 5)).encode();
            b[0] = v;
            assert_eq!(
                CgChangeLook::decode(&b).unwrap_err(),
                CgChangeLookError::InvalidHeader {
                    expected: 233,
                    actual: v
                },
                "header {v}"
            );
        }
    }

    #[test]
    fn rejects_every_wrong_frame_length() {
        for len in 0..9 {
            let f = ClientFrame {
                header: H,
                payload: vec![0; len],
            };
            assert!(CgChangeLook::decode_frame(&f).is_err(), "len {len}");
        }
        let mut f = CgChangeLook::new(1, 2, 3, ItemPos::new(4, 5)).to_frame();
        f.payload.push(0);
        assert!(CgChangeLook::decode_frame(&f).is_err());
    }

    #[test]
    fn the_error_type_displays() {
        assert!(CgChangeLookError::Truncated {
            needed: 10,
            available: 0
        }
        .to_string()
        .contains("10"));
        assert!(CgChangeLookError::LengthMismatch {
            expected: 10,
            actual: 12
        }
        .to_string()
        .contains("12"));
        assert!(CgChangeLookError::InvalidHeader {
            expected: 233,
            actual: 1
        }
        .to_string()
        .contains("233"));
        let _: &dyn std::error::Error = &CgChangeLookError::Truncated {
            needed: 10,
            available: 0,
        };
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE];
        CgChangeLook::new(1, 2, 3, ItemPos::new(4, 5)).encode_into(&mut out);
        assert_eq!(out.len(), 1 + 10);
        assert_eq!(out[1], 0xe9);
    }
}
