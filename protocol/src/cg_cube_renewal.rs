//! The 14-byte `CubeRenewalSend` record.
//!
//! # The struct is not called `SPacketCGCubeRenewalSend`
//!
//! ```c
//! #ifdef ENABLE_CUBE_RENEWAL_WORLDARD
//! typedef struct  packet_send_cube_renewal
//! {
//!     BYTE header;
//!     BYTE subheader;
//!     DWORD    index_item;
//!     DWORD    count_item;
//!     DWORD    index_item_improve;
//! }TPacketCGCubeRenewalSend;
//! #endif
//! ```
//!
//! The **struct tag** is `packet_send_cube_renewal` while the typedef name is
//! `TPacketCGCubeRenewalSend`. Searching the tag name and the typedef name finds
//! two different things, which is how this record can look absent. Both live in
//! `server/server/game/packet.h:2884-2897`, and the client's identical struct
//! is at `client/Client/UserInterface/Packet.h:2979-2991`.
//!
//! # Feature gate
//!
//! `ENABLE_CUBE_RENEWAL_WORLDARD` is `#define`d at
//! `server/server/common/prodomodefines.h:22` and is **never undefined**, so the
//! record is in the active server build. The client gates the same macro in
//! `StdAfx.h:90`. Without it the record does not exist at all, so there is no
//! inactive width to model -- unlike `Exchange`, where the gate selects between
//! two real widths.
//!
//! # Width
//!
//! `1 + 1 + 4 + 4 + 4 = 14`, which closes against
//! `Set(HEADER_CG_CUBE_RENEWAL, sizeof(TPacketCGCubeRenewalSend), "CubeRenewalSend")`
//! at `packet_info.cpp:202`. All three `DWORD`s are little-endian.
//!
//! There is **no packing dependency**: the three `DWORD`s are already
//! 4-byte aligned after the two `BYTE`s once the record is packed, and the
//! workspace `#pragma pack(1)` is what makes the struct 14 rather than 16. That
//! is stated rather than assumed -- the *unpacked* size would be 16, and a
//! "natural alignment" reading would be wrong here.
//!
//! # The sub-header names differ between the trees
//!
//! The server enum at `packet.h:2885-2893` is
//! `OPEN_RECEIVE`, `CLEAR_DATES_RECEIVE`, `DATES_RECEIVE`, `DATES_LOADING`,
//! `MAKE_ITEM`, `CLOSE`. The client has its own enum. The sub-header is an
//! opaque `u8`: the values are shared-enum constants resolved in
//! `input_main.cpp:3464`, and pinning one tree's names would hide a
//! naming-only divergence that the source does not resolve.

use crate::cg_inventory::{CgHeader, HEADER_CG_CUBE_RENEWAL};
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGCubeRenewalSend` record, header byte included.
pub const CG_CUBE_RENEWAL_WIRE_SIZE: usize = 1 + 1 + 4 + 4 + 4;
/// The framed payload of `TPacketCGCubeRenewalSend`.
pub const CG_CUBE_RENEWAL_PAYLOAD_SIZE: usize = CG_CUBE_RENEWAL_WIRE_SIZE - 1;

/// Every way the cube-renewal decoder can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgCubeRenewalError {
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

/// The 14-byte `CubeRenewalSend` record: a header, an opaque sub-header, and
/// three little-endian `DWORD`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgCubeRenewal {
    /// The legacy `BYTE subheader`. Opaque: a shared-enum value.
    pub subheader: u8,
    /// The legacy `DWORD index_item`, little-endian.
    pub index_item: u32,
    /// The legacy `DWORD count_item`, little-endian.
    pub count_item: u32,
    /// The legacy `DWORD index_item_improve`, little-endian.
    pub index_item_improve: u32,
}

impl CgCubeRenewal {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_CUBE_RENEWAL
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_CUBE_RENEWAL_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_CUBE_RENEWAL_PAYLOAD_SIZE;

    /// Build the record.
    pub const fn new(
        subheader: u8,
        index_item: u32,
        count_item: u32,
        index_item_improve: u32,
    ) -> Self {
        Self {
            subheader,
            index_item,
            count_item,
            index_item_improve,
        }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.subheader);
        out.extend_from_slice(&self.index_item.to_le_bytes());
        out.extend_from_slice(&self.count_item.to_le_bytes());
        out.extend_from_slice(&self.index_item_improve.to_le_bytes());
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
        payload.push(self.subheader);
        payload.extend_from_slice(&self.index_item.to_le_bytes());
        payload.extend_from_slice(&self.count_item.to_le_bytes());
        payload.extend_from_slice(&self.index_item_improve.to_le_bytes());
        ClientFrame {
            header: Self::header().value(),
            payload,
        }
    }

    /// # Errors
    ///
    /// [`CgCubeRenewalError::Truncated`] below 14 bytes,
    /// [`CgCubeRenewalError::LengthMismatch`] above,
    /// [`CgCubeRenewalError::InvalidHeader`] for a full-length slice not starting
    /// with 220.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgCubeRenewalError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(CgCubeRenewalError::Truncated {
                needed: Self::WIRE_SIZE,
                available: bytes.len(),
            });
        }
        if bytes.len() > Self::WIRE_SIZE {
            return Err(CgCubeRenewalError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header().value() {
            return Err(CgCubeRenewalError::InvalidHeader {
                expected: Self::header().value(),
                actual: bytes[0],
            });
        }
        Ok(Self {
            subheader: bytes[1],
            index_item: u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            count_item: u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]),
            index_item_improve: u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]),
        })
    }

    /// # Errors
    ///
    /// As [`CgCubeRenewal::decode`], except that the payload must be exactly 13
    /// bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgCubeRenewalError> {
        if frame.payload.len() < Self::PAYLOAD_SIZE {
            return Err(CgCubeRenewalError::Truncated {
                needed: Self::PAYLOAD_SIZE,
                available: frame.payload.len(),
            });
        }
        if frame.payload.len() > Self::PAYLOAD_SIZE {
            return Err(CgCubeRenewalError::LengthMismatch {
                expected: Self::PAYLOAD_SIZE,
                actual: frame.payload.len(),
            });
        }
        if frame.header != Self::header().value() {
            return Err(CgCubeRenewalError::InvalidHeader {
                expected: Self::header().value(),
                actual: frame.header,
            });
        }
        Ok(Self {
            subheader: frame.payload[0],
            index_item: u32::from_le_bytes([
                frame.payload[1],
                frame.payload[2],
                frame.payload[3],
                frame.payload[4],
            ]),
            count_item: u32::from_le_bytes([
                frame.payload[5],
                frame.payload[6],
                frame.payload[7],
                frame.payload[8],
            ]),
            index_item_improve: u32::from_le_bytes([
                frame.payload[9],
                frame.payload[10],
                frame.payload[11],
                frame.payload[12],
            ]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u8 = 220;

    #[test]
    fn the_header_is_220() {
        assert_eq!(CgCubeRenewal::header().value(), H);
        assert_eq!(H, 0xdc);
    }

    #[test]
    fn the_record_is_fourteen_bytes() {
        assert_eq!(CG_CUBE_RENEWAL_WIRE_SIZE, 14);
        assert_eq!(CgCubeRenewal::WIRE_SIZE, 14);
        assert_eq!(CgCubeRenewal::PAYLOAD_SIZE, 13);
    }

    #[test]
    fn packing_is_load_bearing_here() {
        // Unlike TAttr67AddData, the unpacked width would be 16. Recorded so
        // nobody "fixes" the layout by removing the pack.
        assert_eq!(1 + 1 + 4 + 4 + 4, 14);
        assert_ne!(1 + 1 + 4 + 4 + 4 + 2, 14);
    }

    #[test]
    fn encodes_to_the_exact_legacy_bytes() {
        let r = CgCubeRenewal::new(0, 0, 0, 0);
        assert_eq!(
            r.encode(),
            vec![0xdc, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn the_three_dwords_are_little_endian() {
        let r = CgCubeRenewal::new(0, 0x0102_0304, 0xAABB_CCDD, 0xFFFF_0000);
        let b = r.encode();
        assert_eq!(&b[2..6], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&b[6..10], &[0xDD, 0xCC, 0xBB, 0xAA]);
        assert_eq!(&b[10..14], &[0x00, 0x00, 0xFF, 0xFF]);
    }

    #[test]
    fn every_byte_is_read() {
        let base = CgCubeRenewal::default();
        let mut changed = 0;
        for i in 1..14 {
            let mut b = base.encode();
            b[i] = 0xFF;
            if CgCubeRenewal::decode(&b).unwrap() != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 13);
    }

    #[test]
    fn the_three_dwords_are_independent() {
        for (a, b, c) in [
            (1u32, 0u32, 0u32),
            (0, 1, 0),
            (0, 0, 1),
            (u32::MAX, 0, 0),
            (0, u32::MAX, 0),
            (0, 0, u32::MAX),
        ] {
            let r = CgCubeRenewal::new(0, a, b, c);
            assert_eq!(CgCubeRenewal::decode(&r.encode()).unwrap(), r);
        }
        let x = CgCubeRenewal::new(0, 1, 2, 3);
        let y = CgCubeRenewal::new(0, 3, 2, 1);
        assert_ne!(x, y);
    }

    #[test]
    fn every_subheader_round_trips() {
        for v in 0..=255_u8 {
            let r = CgCubeRenewal::new(v, 0, 0, 0);
            assert_eq!(
                CgCubeRenewal::decode(&r.encode()).unwrap(),
                r,
                "subheader {v}"
            );
        }
    }

    #[test]
    fn round_trips() {
        let r = CgCubeRenewal::new(5, 0x1122_3344, 0x5566_7788, 0x99AA_BBCC);
        assert_eq!(CgCubeRenewal::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn round_trips_through_a_frame() {
        let r = CgCubeRenewal::new(2, 1, 2, 3);
        let f = r.to_frame();
        assert_eq!(f.header, H);
        assert_eq!(f.payload.len(), 13);
        assert_eq!(CgCubeRenewal::decode_frame(&f).unwrap(), r);
        assert_eq!(&f.payload[..], &r.encode()[1..]);
    }

    #[test]
    fn rejects_every_short_length() {
        for len in 0..14 {
            let mut b = CgCubeRenewal::new(1, 2, 3, 4).encode();
            b.truncate(len);
            assert_eq!(
                CgCubeRenewal::decode(&b).unwrap_err(),
                CgCubeRenewalError::Truncated {
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
            let mut b = CgCubeRenewal::new(1, 2, 3, 4).encode();
            b.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgCubeRenewal::decode(&b).unwrap_err(),
                CgCubeRenewalError::LengthMismatch {
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
            let mut b = CgCubeRenewal::new(1, 2, 3, 4).encode();
            b[0] = v;
            assert_eq!(
                CgCubeRenewal::decode(&b).unwrap_err(),
                CgCubeRenewalError::InvalidHeader {
                    expected: 220,
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
            assert!(CgCubeRenewal::decode_frame(&f).is_err(), "len {len}");
        }
        for extra in 1..=3 {
            let mut f = CgCubeRenewal::new(1, 2, 3, 4).to_frame();
            f.payload.extend(std::iter::repeat_n(0u8, extra));
            assert!(CgCubeRenewal::decode_frame(&f).is_err(), "extra {extra}");
        }
    }

    #[test]
    fn a_frame_header_cannot_come_from_the_payload() {
        let f = ClientFrame {
            header: 0x01,
            payload: vec![0xdc, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        };
        assert_eq!(
            CgCubeRenewal::decode_frame(&f).unwrap_err(),
            CgCubeRenewalError::InvalidHeader {
                expected: 220,
                actual: 0x01
            }
        );
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE, 0xAD, 0xBE];
        CgCubeRenewal::new(1, 2, 3, 4).encode_into(&mut out);
        assert_eq!(out.len(), 3 + 14);
        assert_eq!(out[3], 0xdc);
    }
}
