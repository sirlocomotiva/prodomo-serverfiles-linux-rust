//! The 14-byte `Exchange` record.
//!
//! ```c
//! typedef struct command_exchange
//! {
//!     BYTE    header;
//!     BYTE    sub_header;
//! #ifdef ENABLE_REMOVE_LIMIT_GOLD
//!     unsigned long long    arg1;
//! #else
//!     DWORD    arg1;
//! #endif
//!     BYTE    arg2;
//!     TItemPos    Pos;
//! } TPacketCGExchange;
//! ```
//!
//! # The width comes from a real, active `#ifdef`
//!
//! `ENABLE_REMOVE_LIMIT_GOLD` is `#define`d in **both** trees and is never
//! undefined:
//!
//! - server `server/server/common/prodomodefines.h:157`, marked
//!   `// __Unsigned_Long_Long_Limit_Gold__`;
//! - client `client/Client/UserInterface/LOCALE_INC.H:75`.
//!
//! So `arg1` is an `unsigned long long` and the active record is
//! `1 + 1 + 8 + 1 + 3 = 14`, which closes against
//! `Set(HEADER_CG_EXCHANGE, sizeof(TPacketCGExchange), "Exchange")`.
//!
//! **The inactive profile is 10 bytes**, and the difference is exactly 4: a
//! `DWORD` `arg1`. This is the same shape as the sash and item-drop records, so
//! the active width is named explicitly here rather than left to look obvious.
//! The status is recorded, not modelled: a 10-byte profile does not exist in
//! either checked-in tree, and inventing one would claim a wire format no
//! source declares.
//!
//! # The sub-header shares an enum with the `GC` direction
//!
//! ```c
//! enum
//! {
//!     EXCHANGE_SUBHEADER_CG_START,    /* arg1 == vid of target character */
//!     EXCHANGE_SUBHEADER_CG_ITEM_ADD,    /* arg1 == position of item */
//!     EXCHANGE_SUBHEADER_CG_ITEM_DEL,    /* arg1 == position of item */
//!     EXCHANGE_SUBHEADER_CG_ELK_ADD,    /* arg1 == amount of gold */
//!     EXCHANGE_SUBHEADER_CG_ACCEPT,    /* arg1 == not used */
//!     EXCHANGE_SUBHEADER_CG_CANCEL,    /* arg1 == not used */
//! };
//! ```
//!
//! The comments already say what the legacy author knew: `arg1` means a VID, an
//! item position, or a gold amount depending on the sub-header, and it is
//! explicitly **not used** for `ACCEPT` and `CANCEL`. The codec therefore does
//! not rename `arg1` to a VID, a position, or an amount. It is one opaque
//! `u64` whose meaning the sub-header selects, and that is a session fact.
//!
//! The sub-header stays an opaque `u8`: the handler `switch`es it with a
//! `default:` arm, so unknown values are absorbed rather than fatal.
//!
//! # `Pos` is the shared 3-byte `TItemPos`
//!
//! `TItemPos` is `BYTE window_type` then little-endian `WORD cell`, packed to
//! [`ItemPos::WIRE_SIZE`] = 3. It is **not** re-declared here; it is the same
//! type as `cg_sash` and `cg_dragon_soul`, and a fourth copy would be the first
//! step toward a divergent one.

use crate::cg_inventory::{CgHeader, HEADER_CG_EXCHANGE};
use crate::cg_wire::ClientFrame;
use crate::item_pos::ItemPos;

/// The full legacy `TPacketCGExchange` record, header byte included.
pub const CG_EXCHANGE_WIRE_SIZE: usize = 1 + 1 + 8 + 1 + ItemPos::WIRE_SIZE;
/// The framed payload of `TPacketCGExchange`.
pub const CG_EXCHANGE_PAYLOAD_SIZE: usize = CG_EXCHANGE_WIRE_SIZE - 1;

/// The width the same record would have with `arg1` as a `DWORD`.
///
/// Recorded so the four-byte difference is explicit. No such profile exists in
/// either checked-in tree, so nothing decodes it.
pub const CG_EXCHANGE_DWORD_PROFILE_SIZE: usize = 1 + 1 + 4 + 1 + ItemPos::WIRE_SIZE;

/// Every way the exchange decoder can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgExchangeError {
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

impl core::fmt::Display for CgExchangeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated exchange: need {needed} bytes, got {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "exchange length mismatch: expected {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(
                    f,
                    "invalid exchange header: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for CgExchangeError {}

/// The 14-byte `Exchange` record: a header, an opaque sub-header, an opaque
/// 8-byte `arg1`, an opaque `arg2`, and the shared 3-byte position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgExchange {
    /// The legacy `BYTE sub_header`. A shared-enum value.
    pub sub_header: u8,
    /// The legacy `unsigned long long arg1`, little-endian. Its meaning depends
    /// on `sub_header`, and the source marks it unused for `ACCEPT`/`CANCEL`.
    pub arg1: u64,
    /// The legacy `BYTE arg2`. Opaque.
    pub arg2: u8,
    /// The legacy `TItemPos Pos`, the shared packed 3-byte position.
    pub pos: ItemPos,
}

impl CgExchange {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_EXCHANGE
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_EXCHANGE_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_EXCHANGE_PAYLOAD_SIZE;

    /// Build the record.
    pub const fn new(sub_header: u8, arg1: u64, arg2: u8, pos: ItemPos) -> Self {
        Self {
            sub_header,
            arg1,
            arg2,
            pos,
        }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.sub_header);
        out.extend_from_slice(&self.arg1.to_le_bytes());
        out.push(self.arg2);
        self.pos.encode_into(out);
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
        payload.push(self.sub_header);
        payload.extend_from_slice(&self.arg1.to_le_bytes());
        payload.push(self.arg2);
        self.pos.encode_into(&mut payload);
        ClientFrame {
            header: Self::header().value(),
            payload,
        }
    }

    /// # Errors
    ///
    /// [`CgExchangeError::Truncated`] below 14 bytes,
    /// [`CgExchangeError::LengthMismatch`] above,
    /// [`CgExchangeError::InvalidHeader`] for a full-length slice not starting
    /// with 27.
    ///
    /// Those three are the only failures. `TItemPos` is read with
    /// [`ItemPos::decode_at`] rather than [`ItemPos::decode`], because the width
    /// check above has already proved the 3 bytes exist, and
    /// [`ItemPos::decode`] can fail on nothing else: it validates length, not
    /// `window_type`. An `ItemPos` error variant here would be unreachable code
    /// dressed up as a check.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgExchangeError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(CgExchangeError::Truncated {
                needed: Self::WIRE_SIZE,
                available: bytes.len(),
            });
        }
        if bytes.len() > Self::WIRE_SIZE {
            return Err(CgExchangeError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header().value() {
            return Err(CgExchangeError::InvalidHeader {
                expected: Self::header().value(),
                actual: bytes[0],
            });
        }
        let arg1 = u64::from_le_bytes([
            bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9],
        ]);
        // The 3 bytes at offset 11, never the remaining tail. The length above
        // already proved 14 bytes, which is the precondition `decode_at` needs.
        let pos = ItemPos::decode_at(bytes, 11);
        Ok(Self {
            sub_header: bytes[1],
            arg1,
            arg2: bytes[10],
            pos,
        })
    }

    /// # Errors
    ///
    /// As [`CgExchange::decode`], except that the payload must be exactly 13
    /// bytes. The `TItemPos` bytes are read at payload offset 10.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgExchangeError> {
        if frame.payload.len() < Self::PAYLOAD_SIZE {
            return Err(CgExchangeError::Truncated {
                needed: Self::PAYLOAD_SIZE,
                available: frame.payload.len(),
            });
        }
        if frame.payload.len() > Self::PAYLOAD_SIZE {
            return Err(CgExchangeError::LengthMismatch {
                expected: Self::PAYLOAD_SIZE,
                actual: frame.payload.len(),
            });
        }
        if frame.header != Self::header().value() {
            return Err(CgExchangeError::InvalidHeader {
                expected: Self::header().value(),
                actual: frame.header,
            });
        }
        let arg1 = u64::from_le_bytes([
            frame.payload[1],
            frame.payload[2],
            frame.payload[3],
            frame.payload[4],
            frame.payload[5],
            frame.payload[6],
            frame.payload[7],
            frame.payload[8],
        ]);
        // The 3 bytes at payload offset 10, which is byte 11 of the record.
        let pos = ItemPos::decode_at(&frame.payload, 10);
        Ok(Self {
            sub_header: frame.payload[0],
            arg1,
            arg2: frame.payload[9],
            pos,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u8 = 27;

    /// A position that is valid in every window the legacy `BYTE` allows.
    fn pos() -> ItemPos {
        ItemPos::new(0, 10)
    }

    #[test]
    fn the_header_is_27() {
        assert_eq!(CgExchange::header().value(), H);
        assert_eq!(H, 0x1b);
    }

    #[test]
    fn the_active_record_is_fourteen_bytes() {
        assert_eq!(CG_EXCHANGE_WIRE_SIZE, 14);
        assert_eq!(CgExchange::WIRE_SIZE, 14);
        assert_eq!(CgExchange::PAYLOAD_SIZE, 13);
    }

    #[test]
    fn the_dword_profile_is_four_bytes_shorter() {
        // The inactive ENABLE_REMOVE_LIMIT_GOLD-off reading, recorded not modelled.
        assert_eq!(CG_EXCHANGE_DWORD_PROFILE_SIZE, 10);
        assert_eq!(CG_EXCHANGE_WIRE_SIZE - CG_EXCHANGE_DWORD_PROFILE_SIZE, 4);
    }

    #[test]
    fn encodes_to_the_exact_legacy_bytes() {
        let r = CgExchange::new(0, 0, 0, pos());
        assert_eq!(
            r.encode(),
            vec![0x1b, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 0]
        );
    }

    #[test]
    fn arg1_is_a_little_endian_u64() {
        let r = CgExchange::new(0, 0x0102_0304_0506_0708, 0, pos());
        let b = r.encode();
        assert_eq!(&b[2..10], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn the_full_u64_range_round_trips() {
        for v in [0u64, 1, 0xFFFF, 0x1_0000, u64::from(u32::MAX), u64::MAX] {
            let r = CgExchange::new(0, v, 0, pos());
            assert_eq!(CgExchange::decode(&r.encode()).unwrap().arg1, v, "arg1 {v}");
        }
    }

    #[test]
    fn every_byte_is_read() {
        let base = CgExchange::new(0, 0, 0, pos());
        let mut changed = 0;
        for i in 1..14 {
            let mut b = base.encode();
            b[i] = b[i].wrapping_add(0x11);
            if CgExchange::decode(&b).unwrap() != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 13);
    }

    #[test]
    fn the_fields_are_independent() {
        let a = CgExchange::new(1, 2, 3, ItemPos::new(4, 5));
        let b = CgExchange::new(3, 2, 1, ItemPos::new(5, 4));
        assert_ne!(a, b);
        assert_eq!(CgExchange::decode(&a.encode()).unwrap(), a);
        assert_eq!(CgExchange::decode(&b.encode()).unwrap(), b);
    }

    #[test]
    fn every_sub_header_round_trips() {
        for v in 0..=255_u8 {
            let r = CgExchange::new(v, 0, 0, pos());
            assert_eq!(
                CgExchange::decode(&r.encode()).unwrap().sub_header,
                v,
                "sub_header {v}"
            );
        }
    }

    #[test]
    fn every_arg2_round_trips() {
        for v in 0..=255_u8 {
            let r = CgExchange::new(0, 0, v, pos());
            assert_eq!(CgExchange::decode(&r.encode()).unwrap().arg2, v, "arg2 {v}");
        }
    }

    #[test]
    fn the_nested_position_is_exactly_three_bytes() {
        // A nested fixed-width decoder must not be handed the remaining tail.
        let r = CgExchange::new(0, 0, 0, ItemPos::new(0xAB, 0xCDEF));
        let b = r.encode();
        assert_eq!(b.len(), 14);
        assert_eq!(&b[11..14], &[0xAB, 0xEF, 0xCD]);
        assert_eq!(
            CgExchange::decode(&b).unwrap().pos,
            ItemPos::new(0xAB, 0xCDEF)
        );
    }

    #[test]
    fn every_window_type_is_accepted_including_255() {
        // ItemPos validates length, not window_type, so no u8 is reserved. An
        // earlier version of this test assumed 0xFF was rejected; it is not, and
        // inventing an invalid window would have pinned a rule the legacy server
        // does not have.
        for w in 0..=255_u8 {
            let r = CgExchange::new(0, 0, 0, ItemPos::new(w, 0));
            assert_eq!(
                CgExchange::decode(&r.encode()).unwrap().pos.window_type,
                w,
                "window {w}"
            );
        }
    }

    #[test]
    fn round_trips() {
        let r = CgExchange::new(2, 0xDEAD_BEEF_CAFE_1234, 0x77, ItemPos::new(1, 99));
        assert_eq!(CgExchange::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn round_trips_through_a_frame() {
        let r = CgExchange::new(2, 1234, 5, ItemPos::new(1, 2));
        let f = r.to_frame();
        assert_eq!(f.header, H);
        assert_eq!(f.payload.len(), 13);
        assert_eq!(CgExchange::decode_frame(&f).unwrap(), r);
        assert_eq!(&f.payload[..], &r.encode()[1..]);
    }

    #[test]
    fn the_header_cannot_come_from_the_payload() {
        let mut f = CgExchange::new(0, 0, 0, pos()).to_frame();
        f.header = 0x99;
        assert_eq!(
            CgExchange::decode_frame(&f).unwrap_err(),
            CgExchangeError::InvalidHeader {
                expected: 27,
                actual: 0x99
            }
        );
    }

    #[test]
    fn rejects_every_short_length() {
        for len in 0..14 {
            let mut b = CgExchange::new(1, 2, 3, pos()).encode();
            b.truncate(len);
            assert_eq!(
                CgExchange::decode(&b).unwrap_err(),
                CgExchangeError::Truncated {
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
            let mut b = CgExchange::new(1, 2, 3, pos()).encode();
            b.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgExchange::decode(&b).unwrap_err(),
                CgExchangeError::LengthMismatch {
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
            let mut b = CgExchange::new(1, 2, 3, pos()).encode();
            b[0] = v;
            assert_eq!(
                CgExchange::decode(&b).unwrap_err(),
                CgExchangeError::InvalidHeader {
                    expected: 27,
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
            assert!(CgExchange::decode_frame(&f).is_err(), "len {len}");
        }
        let mut f = CgExchange::new(1, 2, 3, pos()).to_frame();
        f.payload.push(0);
        assert!(CgExchange::decode_frame(&f).is_err());
    }

    #[test]
    fn the_error_type_displays_and_converts() {
        let e = CgExchangeError::Truncated {
            needed: 14,
            available: 3,
        };
        assert!(e.to_string().contains("14"));
        let e = CgExchangeError::LengthMismatch {
            expected: 14,
            actual: 20,
        };
        assert!(e.to_string().contains("20"));
        let e = CgExchangeError::InvalidHeader {
            expected: 27,
            actual: 1,
        };
        assert!(e.to_string().contains("27"));
        // The `?` conversion from the nested error is the reason the variant
        // carries `#[from]`.
        let _: &dyn std::error::Error = &CgExchangeError::Truncated {
            needed: 14,
            available: 0,
        };
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE];
        CgExchange::new(1, 2, 3, pos()).encode_into(&mut out);
        assert_eq!(out.len(), 1 + 14);
        assert_eq!(out[1], 0x1b);
    }
}
