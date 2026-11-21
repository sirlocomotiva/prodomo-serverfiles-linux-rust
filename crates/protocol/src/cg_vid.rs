//! The two 5-byte records that are a header plus a 32-bit virtual id.
//!
//! | record | header | wire | declaration |
//! |---|---|---|---|
//! | `OnClick` | 26 `0x1a` | 5 | `packet.h:708` |
//! | `TargetInfoLoad` | 59 `0x3b` | 5 | `packet.h:1834` |
//!
//! ```cpp
//! typedef struct command_on_click
//! {
//!     BYTE    header;
//!     DWORD    vid;
//! } TPacketCGOnClick;
//!
//! typedef struct packet_target_info_load
//! {
//!     BYTE header;
//!     DWORD dwVID;
//! } TPacketCGTargetInfoLoad;
//! ```
//!
//! # The `vid` is a `VID`, not an account id and not a player index
//!
//! The name is the trap. `CHARACTER_MANAGER::instance().Find(pinfo->vid)` at
//! `input_main.cpp:1306` takes a `DWORD` virtual id, and so does the target-info
//! path. The Rust field is named `vid` for that reason and **not** `player_id`,
//! because a virtual id is a runtime index into the character manager that a
//! client only ever learns by being told; it is not stable across sessions and
//! must not be cached or compared against an account. This is the same reasoning
//! that keeps `cg_item_drop`'s `gold` a `DWORD` and not a currency amount.
//!
//! # Neither handler validates the vid
//!
//! `CInputMain::OnClick` looks the id up and, if it misses, only logs on a test
//! server:
//!
//! ```cpp
//! if ((victim = CHARACTER_MANAGER::instance().Find(pinfo->vid)))
//!     victim->OnClick(ch);
//! else if (test_server)
//! {
//!     sys_err("CInputMain::OnClick %s.Click.NOT_EXIST_VID[%d]", ch->GetName(), pinfo->vid);
//! }
//! ```
//!
//! So a vid that names nothing is a **silent no-op in production**, not an
//! error and not a close. Range and existence checks belong to the session
//! layer that owns the character manager, not to a two-field codec, so `vid` is
//! left as an opaque `u32` and every value round-trips.
use crate::cg_inventory::CgHeader;
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGOnClick` record, header byte included.
pub const CG_ON_CLICK_WIRE_SIZE: usize = 1 + 4;
/// The framed payload of `TPacketCGOnClick`.
pub const CG_ON_CLICK_PAYLOAD_SIZE: usize = CG_ON_CLICK_WIRE_SIZE - 1;

/// The full legacy `TPacketCGTargetInfoLoad` record, header byte included.
pub const CG_TARGET_INFO_LOAD_WIRE_SIZE: usize = 1 + 4;
/// The framed payload of `TPacketCGTargetInfoLoad`.
pub const CG_TARGET_INFO_LOAD_PAYLOAD_SIZE: usize = CG_TARGET_INFO_LOAD_WIRE_SIZE - 1;

/// Every way the two virtual-id decoders can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgVidError {
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

/// The 5-byte `OnClick` record: a header then a little-endian `DWORD` virtual id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgOnClick {
    /// The legacy `vid`, little-endian. Opaque: the handler treats an unknown
    /// id as a silent no-op rather than an error.
    pub vid: u32,
}

/// The 5-byte `TargetInfoLoad` record: a header then a little-endian `DWORD`
/// virtual id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgTargetInfoLoad {
    /// The legacy `dwVID`, little-endian. Opaque for the same reason as
    /// [`CgOnClick::vid`].
    pub dw_vid: u32,
}

/// Reject anything that is not exactly `expected` bytes wide.
fn check_exact(actual: usize, expected: usize) -> Result<(), CgVidError> {
    if actual < expected {
        return Err(CgVidError::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(CgVidError::LengthMismatch { expected, actual });
    }
    Ok(())
}

/// Reject a header byte that is not `expected`.
fn check_header(actual: u8, expected: u8) -> Result<(), CgVidError> {
    if actual != expected {
        return Err(CgVidError::InvalidHeader { expected, actual });
    }
    Ok(())
}

/// Read the little-endian `u32` at `at`.
fn read_vid(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

impl CgOnClick {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        crate::cg_inventory::HEADER_CG_ON_CLICK
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_ON_CLICK_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_ON_CLICK_PAYLOAD_SIZE;

    /// Build the record from a virtual id.
    pub const fn new(vid: u32) -> Self {
        Self { vid }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.extend_from_slice(&self.vid.to_le_bytes());
    }

    /// Encode to a fresh 5-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 4-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame {
            header: Self::header().value(),
            payload: self.vid.to_le_bytes().to_vec(),
        }
    }

    /// # Errors
    ///
    /// [`CgVidError::Truncated`] below 5 bytes,
    /// [`CgVidError::LengthMismatch`] above,
    /// [`CgVidError::InvalidHeader`] for a full-length slice not starting with
    /// 26.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgVidError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        check_header(bytes[0], Self::header().value())?;
        Ok(Self {
            vid: read_vid(bytes, 1),
        })
    }

    /// # Errors
    ///
    /// As [`CgOnClick::decode`], except that the payload must be exactly 4
    /// bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgVidError> {
        check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
        check_header(frame.header, Self::header().value())?;
        Ok(Self {
            vid: read_vid(&frame.payload, 0),
        })
    }
}

impl CgTargetInfoLoad {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        crate::cg_inventory::HEADER_CG_TARGET_INFO_LOAD
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_TARGET_INFO_LOAD_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_TARGET_INFO_LOAD_PAYLOAD_SIZE;

    /// Build the record from a virtual id.
    pub const fn new(dw_vid: u32) -> Self {
        Self { dw_vid }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
    }

    /// Encode to a fresh 5-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 4-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame {
            header: Self::header().value(),
            payload: self.dw_vid.to_le_bytes().to_vec(),
        }
    }

    /// # Errors
    ///
    /// [`CgVidError::Truncated`] below 5 bytes,
    /// [`CgVidError::LengthMismatch`] above,
    /// [`CgVidError::InvalidHeader`] for a full-length slice not starting with
    /// 59.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgVidError> {
        check_exact(bytes.len(), Self::WIRE_SIZE)?;
        check_header(bytes[0], Self::header().value())?;
        Ok(Self {
            dw_vid: read_vid(bytes, 1),
        })
    }

    /// # Errors
    ///
    /// As [`CgTargetInfoLoad::decode`], except that the payload must be exactly
    /// 4 bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgVidError> {
        check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
        check_header(frame.header, Self::header().value())?;
        Ok(Self {
            dw_vid: read_vid(&frame.payload, 0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLICK: u8 = 26;
    const TINFO: u8 = 59;

    #[test]
    fn the_headers_are_26_and_59() {
        assert_eq!(CgOnClick::header().value(), CLICK);
        assert_eq!(CgTargetInfoLoad::header().value(), TINFO);
    }

    #[test]
    fn both_records_are_five_bytes() {
        assert_eq!(CgOnClick::WIRE_SIZE, 5);
        assert_eq!(CgTargetInfoLoad::WIRE_SIZE, 5);
        assert_eq!(CgOnClick::PAYLOAD_SIZE, 4);
        assert_eq!(CgTargetInfoLoad::PAYLOAD_SIZE, 4);
        assert_eq!(CgOnClick::WIRE_SIZE, 1 + 4);
    }

    #[test]
    fn on_click_encodes_to_the_exact_legacy_bytes() {
        assert_eq!(CgOnClick::new(0).encode(), vec![CLICK, 0, 0, 0, 0]);
        assert_eq!(
            CgOnClick::new(0x1122_3344).encode(),
            vec![CLICK, 0x44, 0x33, 0x22, 0x11]
        );
        assert_eq!(
            CgOnClick::new(u32::MAX).encode(),
            vec![CLICK, 0xFF, 0xFF, 0xFF, 0xFF]
        );
    }

    #[test]
    fn target_info_load_encodes_to_the_exact_legacy_bytes() {
        assert_eq!(CgTargetInfoLoad::new(0).encode(), vec![TINFO, 0, 0, 0, 0]);
        assert_eq!(
            CgTargetInfoLoad::new(0xAABB_CCDD).encode(),
            vec![TINFO, 0xDD, 0xCC, 0xBB, 0xAA]
        );
    }

    #[test]
    fn the_vid_is_little_endian_not_big_endian() {
        // 0x01020304 written little-endian is 04 03 02 01. If a future edit
        // reaches for to_be_bytes this assertion inverts.
        let bytes = CgOnClick::new(0x0102_0304).encode();
        assert_eq!(&bytes[1..], &[0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn both_round_trip() {
        for vid in [0, 1, 0x7FFF_FFFF, 0x8000_0000, u32::MAX] {
            let a = CgOnClick::new(vid);
            assert_eq!(CgOnClick::decode(&a.encode()).unwrap(), a);
            let b = CgTargetInfoLoad::new(vid);
            assert_eq!(CgTargetInfoLoad::decode(&b.encode()).unwrap(), b);
        }
    }

    #[test]
    fn both_round_trip_through_a_frame() {
        let a = CgOnClick::new(0xDEAD_BEEF);
        let fa = a.to_frame();
        assert_eq!(fa.header, CLICK);
        assert_eq!(fa.payload.len(), 4);
        assert_eq!(CgOnClick::decode_frame(&fa).unwrap(), a);

        let b = CgTargetInfoLoad::new(0x0BAD_F00D);
        let fb = b.to_frame();
        assert_eq!(fb.header, TINFO);
        assert_eq!(fb.payload.len(), 4);
        assert_eq!(CgTargetInfoLoad::decode_frame(&fb).unwrap(), b);
    }

    #[test]
    fn the_two_records_are_not_interchangeable() {
        assert!(CgTargetInfoLoad::decode(&CgOnClick::new(1).encode()).is_err());
        assert!(CgOnClick::decode(&CgTargetInfoLoad::new(1).encode()).is_err());
    }

    #[test]
    fn rejects_every_short_length_for_both() {
        for len in 0..5 {
            let mut a = CgOnClick::new(1).encode();
            a.truncate(len);
            assert_eq!(
                CgOnClick::decode(&a).unwrap_err(),
                CgVidError::Truncated {
                    needed: 5,
                    available: len
                },
                "click len {len}"
            );
            let mut b = CgTargetInfoLoad::new(1).encode();
            b.truncate(len);
            assert_eq!(
                CgTargetInfoLoad::decode(&b).unwrap_err(),
                CgVidError::Truncated {
                    needed: 5,
                    available: len
                },
                "tinfo len {len}"
            );
        }
    }

    #[test]
    fn rejects_every_long_length_for_both() {
        for extra in 1..=3 {
            let mut a = CgOnClick::new(1).encode();
            a.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgOnClick::decode(&a).unwrap_err(),
                CgVidError::LengthMismatch {
                    expected: 5,
                    actual: 5 + extra
                }
            );
            let mut b = CgTargetInfoLoad::new(1).encode();
            b.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgTargetInfoLoad::decode(&b).unwrap_err(),
                CgVidError::LengthMismatch {
                    expected: 5,
                    actual: 5 + extra
                }
            );
        }
    }

    #[test]
    fn rejects_every_wrong_frame_length() {
        for len in 0..4 {
            let f = ClientFrame {
                header: CLICK,
                payload: vec![0; len],
            };
            assert!(CgOnClick::decode_frame(&f).is_err(), "len {len}");
            let f = ClientFrame {
                header: TINFO,
                payload: vec![0; len],
            };
            assert!(CgTargetInfoLoad::decode_frame(&f).is_err(), "len {len}");
        }
    }

    #[test]
    fn rejects_a_wrong_frame_header() {
        let f = ClientFrame {
            header: 200,
            payload: vec![1, 2, 3, 4],
        };
        assert_eq!(
            CgOnClick::decode_frame(&f).unwrap_err(),
            CgVidError::InvalidHeader {
                expected: 26,
                actual: 200
            }
        );
    }

    #[test]
    fn an_unknown_vid_is_representable_not_an_error() {
        // The legacy handler treats a vid that names nobody as a silent no-op
        // outside test builds, so a missing character is not a decode failure.
        for vid in [0, u32::MAX] {
            let a = CgOnClick::new(vid);
            assert_eq!(CgOnClick::decode(&a.encode()).unwrap().vid, vid);
        }
    }

    #[test]
    fn every_vid_byte_is_read() {
        let base = CgOnClick::new(0);
        let mut changed = 0;
        for i in 0..4 {
            let mut bytes = base.encode();
            bytes[1 + i] = 0xFF;
            if CgOnClick::decode(&bytes).unwrap() != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 4);
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE, 0xAD];
        CgOnClick::new(1).encode_into(&mut out);
        assert_eq!(out.len(), 2 + 5);
        assert_eq!(&out[2..], &CgOnClick::new(1).encode()[..]);
    }
}
