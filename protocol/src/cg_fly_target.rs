//! The 13-byte fly-targeting record, which arrives under **two** headers.
//!
//! | record | header | wire | declaration | dispatch |
//! |---|---|---|---|---|
//! | `FlyTarget` | 51 `0x33` | 13 | `packet.h:1745` | `input_main.cpp:3732` |
//! | `AddFlyTarget` | 53 `0x35` | 13 | `packet.h:1745` | `input_main.cpp:3731` |
//!
//! Both register **the same C++ type**, `sizeof(TPacketCGFlyTargeting)`, and the
//! two `case` arms fall through to one call:
//!
//! ```cpp
//! case HEADER_CG_ADD_FLY_TARGETING:
//! case HEADER_CG_FLY_TARGETING:
//!     FlyTarget(ch, c_pData, bHeader);
//!     break;
//! ```
//!
//! ```cpp
//! void CInputMain::FlyTarget(LPCLIENTDESC pcData, const BYTE bHeader)
//! {
//!     TPacketCGFlyTargeting * p = (TPacketCGFlyTargeting *) pcData;
//!     ch->FlyTarget(p->dwTargetVID, p->x, p->y, bHeader);
//! }
//! ```
//!
//! # The header byte is a payload-equivalent field, not a framing prefix
//!
//! This is the consequence that matters, and it is why this record is not
//! modelled as "a struct with a validated header". **Two identical 12-byte
//! bodies mean different actions, and the remaining bytes cannot say which one
//! arrived.** The handler recovers the distinction only because the dispatcher
//! hands `bHeader` down alongside the data pointer.
//!
//! A decoder that checks the header and then drops it would destroy exactly the
//! information the server uses. So [`CgFlyTarget`] **keeps** its header as
//! `header: CgFlyTargetHeader`, and neither variant is the default: a caller
//! cannot accidentally treat "add" and "move" as the same action.
//!
//! This is the fifth distinct reason a CG header byte may not be
//! checked-and-discarded, after the sash sub-header, the dragon-soul sub-header,
//! the GC/CG direction splits, and the `CG_CHANGE_LANGUAGE` profile divergence.
//!
//! # Field names differ between the trees, bytes do not
//!
//! ```cpp
//! // server/server/game/packet.h:1745
//! typedef struct command_fly_targeting
//! {
//!     BYTE  bHeader;
//!     DWORD dwTargetVID;
//!     long  x, y;
//! } TPacketCGFlyTargeting;
//!
//! // client/Client/UserInterface/Packet.h:661
//! typedef struct command_fly_targeting
//! {
//!     BYTE  bHeader;
//!     DWORD dwTargetVID;
//!     long  lX;
//!     long  lY;
//! } TPacketCGFlyTargeting;
//! ```
//!
//! Same tag, same widths, same order, four differently spelled names. The Rust
//! field is `x` and `y` because that is the server's spelling and the server is
//! the dispatch oracle; the client's `lX`/`lY` is noted here only so the
//! difference is not rediscovered later as a discrepancy.
//!
//! Both `long` fields are **signed** 32-bit little-endian. The client sends
//! map coordinates, which are negative west and south of the origin, so a
//! `u32` would be wrong even though the width matches.
use crate::cg_inventory::{CgHeader, HEADER_CG_ADD_FLY_TARGETING, HEADER_CG_FLY_TARGETING};
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGFlyTargeting` record, header byte included.
pub const CG_FLY_TARGETING_WIRE_SIZE: usize = 1 + 4 + 4 + 4;
/// The framed payload of `TPacketCGFlyTargeting`.
pub const CG_FLY_TARGETING_PAYLOAD_SIZE: usize = CG_FLY_TARGETING_WIRE_SIZE - 1;

/// Which of the two registrations sent this record.
///
/// This is a two-valued enum rather than a `bool` on purpose. A `bool` named
/// `is_add` invites `if rec.is_add`, and the legacy code has no such flag to
/// mirror -- it passes the raw header byte. Naming the values after the two
/// registrations keeps the wire fact and the semantic fact in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgFlyTargetHeader {
    /// `HEADER_CG_FLY_TARGETING`, 51 `0x33`.
    FlyTarget,
    /// `HEADER_CG_ADD_FLY_TARGETING`, 53 `0x35`.
    AddFlyTarget,
}

impl CgFlyTargetHeader {
    /// Every header byte this record can legitimately arrive under.
    pub const ALL: [Self; 2] = [Self::FlyTarget, Self::AddFlyTarget];

    /// The legacy header byte.
    #[must_use]
    pub const fn header(self) -> CgHeader {
        match self {
            Self::FlyTarget => HEADER_CG_FLY_TARGETING,
            Self::AddFlyTarget => HEADER_CG_ADD_FLY_TARGETING,
        }
    }

    /// The header byte as a raw `u8`.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.header().value()
    }

    /// Map a raw header byte to a variant.
    #[must_use]
    pub const fn from_value(value: u8) -> Option<Self> {
        match value {
            0x33 => Some(Self::FlyTarget),
            0x35 => Some(Self::AddFlyTarget),
            _ => None,
        }
    }
}

/// Every way the fly-targeting decoder can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgFlyTargetError {
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
    /// The right number of bytes, but not either registration's header.
    InvalidHeader {
        /// The two header bytes this record requires.
        expected: [u8; 2],
        /// The header byte that was actually present.
        actual: u8,
    },
}

/// The 13-byte fly-targeting record.
///
/// The header is a **field**. Two otherwise identical bodies mean different
/// actions, so it is preserved rather than validated away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgFlyTarget {
    /// Which registration sent this. Load-bearing: the server dispatches on it.
    pub header: CgFlyTargetHeader,
    /// The legacy `dwTargetVID`, little-endian. Opaque.
    pub dw_target_vid: u32,
    /// The legacy `x`, signed 32-bit little-endian.
    pub x: i32,
    /// The legacy `y`, signed 32-bit little-endian.
    pub y: i32,
}

/// Reject anything that is not exactly `expected` bytes wide.
fn check_exact(actual: usize, expected: usize) -> Result<(), CgFlyTargetError> {
    if actual < expected {
        return Err(CgFlyTargetError::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(CgFlyTargetError::LengthMismatch { expected, actual });
    }
    Ok(())
}

/// Reject a header byte that is neither registration.
fn check_header(actual: u8) -> Result<CgFlyTargetHeader, CgFlyTargetError> {
    CgFlyTargetHeader::from_value(actual).ok_or(CgFlyTargetError::InvalidHeader {
        expected: [0x33, 0x35],
        actual,
    })
}

/// Read a signed little-endian `i32` at `at`.
fn read_i32(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

impl CgFlyTarget {
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_FLY_TARGETING_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_FLY_TARGETING_PAYLOAD_SIZE;

    /// Build the record. The header must be stated; there is no default variant.
    pub const fn new(header: CgFlyTargetHeader, dw_target_vid: u32, x: i32, y: i32) -> Self {
        Self {
            header,
            dw_target_vid,
            x,
            y,
        }
    }

    /// The header byte this instance encodes.
    #[must_use]
    pub const fn header_byte(&self) -> u8 {
        self.header.value()
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.header.value());
        out.extend_from_slice(&self.dw_target_vid.to_le_bytes());
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
    }

    /// Encode to a fresh 13-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 12-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(Self::PAYLOAD_SIZE);
        payload.extend_from_slice(&self.dw_target_vid.to_le_bytes());
        payload.extend_from_slice(&self.x.to_le_bytes());
        payload.extend_from_slice(&self.y.to_le_bytes());
        ClientFrame {
            header: self.header.value(),
            payload,
        }
    }

    /// # Errors
    ///
    /// [`CgFlyTargetError::Truncated`] below 13 bytes,
    /// [`CgFlyTargetError::LengthMismatch`] above,
    /// [`CgFlyTargetError::InvalidHeader`] for a full-length slice whose first
    /// byte is neither 51 nor 53.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgFlyTargetError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        let header = check_header(bytes[0])?;
        Ok(Self {
            header,
            dw_target_vid: u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
            x: read_i32(bytes, 5),
            y: read_i32(bytes, 9),
        })
    }

    /// # Errors
    ///
    /// As [`CgFlyTarget::decode`], except that the payload must be exactly 12
    /// bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgFlyTargetError> {
        check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
        let header = check_header(frame.header)?;
        Ok(Self {
            header,
            dw_target_vid: u32::from_le_bytes([
                frame.payload[0],
                frame.payload[1],
                frame.payload[2],
                frame.payload[3],
            ]),
            x: read_i32(&frame.payload, 4),
            y: read_i32(&frame.payload, 8),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_is_thirteen_bytes() {
        assert_eq!(CG_FLY_TARGETING_WIRE_SIZE, 13);
        assert_eq!(CgFlyTarget::WIRE_SIZE, 13);
        assert_eq!(CgFlyTarget::PAYLOAD_SIZE, 12);
        assert_eq!(CG_FLY_TARGETING_WIRE_SIZE, 1 + 4 + 4 + 4);
    }

    #[test]
    fn the_two_headers_are_51_and_53() {
        assert_eq!(CgFlyTargetHeader::FlyTarget.value(), 0x33);
        assert_eq!(CgFlyTargetHeader::AddFlyTarget.value(), 0x35);
        assert_eq!(CgFlyTargetHeader::FlyTarget.header().value(), 51);
        assert_eq!(CgFlyTargetHeader::AddFlyTarget.header().value(), 53);
    }

    #[test]
    fn from_value_maps_exactly_the_two_headers() {
        assert_eq!(
            CgFlyTargetHeader::from_value(0x33),
            Some(CgFlyTargetHeader::FlyTarget)
        );
        assert_eq!(
            CgFlyTargetHeader::from_value(0x35),
            Some(CgFlyTargetHeader::AddFlyTarget)
        );
        for v in 0..=255_u8 {
            let expected = if v == 0x33 || v == 0x35 {
                Some(CgFlyTargetHeader::from_value(v).expect("mapped"))
            } else {
                None
            };
            assert_eq!(CgFlyTargetHeader::from_value(v), expected, "value {v}");
        }
    }

    #[test]
    fn both_variants_encode_with_their_own_header() {
        let body = CgFlyTarget::new(CgFlyTargetHeader::FlyTarget, 1, 2, 3);
        let add = CgFlyTarget::new(CgFlyTargetHeader::AddFlyTarget, 1, 2, 3);
        assert_eq!(body.encode()[0], 0x33);
        assert_eq!(add.encode()[0], 0x35);
        // Identical bodies apart from the header byte.
        assert_eq!(&body.encode()[1..], &add.encode()[1..]);
    }

    #[test]
    fn the_two_variants_are_distinguishable_after_decoding() {
        let body = CgFlyTarget::new(CgFlyTargetHeader::FlyTarget, 0x1111, 2, 3);
        let add = CgFlyTarget::new(CgFlyTargetHeader::AddFlyTarget, 0x1111, 2, 3);
        let b = CgFlyTarget::decode(&body.encode()).unwrap();
        let a = CgFlyTarget::decode(&add.encode()).unwrap();
        assert_eq!(b.header, CgFlyTargetHeader::FlyTarget);
        assert_eq!(a.header, CgFlyTargetHeader::AddFlyTarget);
        // The only difference between the two records is the header field.
        assert_eq!(b.dw_target_vid, a.dw_target_vid);
        assert_eq!(b.x, a.x);
        assert_eq!(b.y, a.y);
        assert_ne!(b, a);
    }

    #[test]
    fn encodes_to_the_exact_legacy_bytes() {
        let rec = CgFlyTarget::new(CgFlyTargetHeader::FlyTarget, 0x0102_0304, 5, -1);
        assert_eq!(
            rec.encode(),
            vec![0x33, 0x04, 0x03, 0x02, 0x01, 5, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]
        );
    }

    #[test]
    fn the_coordinates_are_signed_little_endian() {
        let rec = CgFlyTarget::new(CgFlyTargetHeader::AddFlyTarget, 0, -2, 0x0102_0304);
        let bytes = rec.encode();
        assert_eq!(&bytes[5..9], &[0xFE, 0xFF, 0xFF, 0xFF]);
        assert_eq!(&bytes[9..13], &[0x04, 0x03, 0x02, 0x01]);
        let back = CgFlyTarget::decode(&bytes).unwrap();
        assert_eq!(back.x, -2);
        assert_eq!(back.y, 0x0102_0304);
    }

    #[test]
    fn both_round_trip() {
        for header in CgFlyTargetHeader::ALL {
            for (vid, x, y) in [
                (0, 0, 0),
                (u32::MAX, i32::MIN, i32::MAX),
                (1, -1, 1),
                (0x8000_0000, 0x7FFF_FFFF, -0x8000_0000),
            ] {
                let rec = CgFlyTarget::new(header, vid, x, y);
                assert_eq!(CgFlyTarget::decode(&rec.encode()).unwrap(), rec);
            }
        }
    }

    #[test]
    fn both_round_trip_through_a_frame() {
        let rec = CgFlyTarget::new(CgFlyTargetHeader::AddFlyTarget, 7, -7, 8);
        let f = rec.to_frame();
        assert_eq!(f.header, 0x35);
        assert_eq!(f.payload.len(), 12);
        assert_eq!(CgFlyTarget::decode_frame(&f).unwrap(), rec);
    }

    #[test]
    fn the_frame_payload_is_the_record_minus_the_header() {
        let rec = CgFlyTarget::new(CgFlyTargetHeader::FlyTarget, 9, 10, 11);
        assert_eq!(&rec.to_frame().payload[..], &rec.encode()[1..]);
    }

    #[test]
    fn rejects_every_short_length() {
        for len in 0..13 {
            let mut b = CgFlyTarget::new(CgFlyTargetHeader::FlyTarget, 1, 2, 3).encode();
            b.truncate(len);
            assert_eq!(
                CgFlyTarget::decode(&b).unwrap_err(),
                CgFlyTargetError::Truncated {
                    needed: 13,
                    available: len
                },
                "len {len}"
            );
        }
    }

    #[test]
    fn rejects_every_long_length() {
        for extra in 1..=3 {
            let mut b = CgFlyTarget::new(CgFlyTargetHeader::FlyTarget, 1, 2, 3).encode();
            b.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgFlyTarget::decode(&b).unwrap_err(),
                CgFlyTargetError::LengthMismatch {
                    expected: 13,
                    actual: 13 + extra
                }
            );
        }
    }

    #[test]
    fn rejects_every_other_header_byte() {
        for v in 0..=255_u8 {
            if v == 0x33 || v == 0x35 {
                continue;
            }
            let mut b = CgFlyTarget::new(CgFlyTargetHeader::FlyTarget, 1, 2, 3).encode();
            b[0] = v;
            assert_eq!(
                CgFlyTarget::decode(&b).unwrap_err(),
                CgFlyTargetError::InvalidHeader {
                    expected: [0x33, 0x35],
                    actual: v
                },
                "header {v}"
            );
        }
    }

    #[test]
    fn rejects_every_wrong_frame_length() {
        for len in 0..12 {
            let f = ClientFrame {
                header: 0x33,
                payload: vec![0; len],
            };
            assert!(CgFlyTarget::decode_frame(&f).is_err(), "len {len}");
        }
    }

    #[test]
    fn rejects_a_wrong_frame_header() {
        let f = ClientFrame {
            header: 0x34,
            payload: vec![0; 12],
        };
        assert_eq!(
            CgFlyTarget::decode_frame(&f).unwrap_err(),
            CgFlyTargetError::InvalidHeader {
                expected: [0x33, 0x35],
                actual: 0x34
            }
        );
    }

    #[test]
    fn every_payload_byte_is_read() {
        let base = CgFlyTarget::new(CgFlyTargetHeader::FlyTarget, 0, 0, 0);
        let mut changed = 0;
        for i in 1..13 {
            let mut b = base.encode();
            b[i] = 0xFF;
            if CgFlyTarget::decode(&b).unwrap() != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 12);
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE];
        CgFlyTarget::new(CgFlyTargetHeader::AddFlyTarget, 1, 2, 3).encode_into(&mut out);
        assert_eq!(out.len(), 1 + 13);
        assert_eq!(out[1], 0x35);
    }

    #[test]
    fn the_all_list_covers_both_variants_exactly_once() {
        assert_eq!(CgFlyTargetHeader::ALL.len(), 2);
        assert_ne!(CgFlyTargetHeader::ALL[0], CgFlyTargetHeader::ALL[1]);
    }
}
