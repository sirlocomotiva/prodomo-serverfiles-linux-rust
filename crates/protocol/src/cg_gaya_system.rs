//! The 6-byte `GayaSystemSend` record -- and the one place where the legacy
//! framing table and the legacy dispatch switch **disagree about the same
//! header byte**.
//!
//! ```c
//! typedef struct packet_gaya_system
//! {
//!     BYTE header;
//!     BYTE subheader;
//!     int  pos;
//! } TPacketCGGayaSystem;
//! ```
//!
//! # Width
//!
//! `1 + 1 + 4 = 6`, which matches the declared `sizeof` in
//! `Set(HEADER_CG_GAYA_SYSTEM, sizeof(TPacketCGGayaSystem), "GayaSystemSend")` at
//! `packet_info.cpp:175`. The struct tag is `packet_gaya_system` and the typedef
//! name is `TPacketCGGayaSystem`; the declaration is byte-identical in
//! `server/server/game/packet.h` and `client/Client/UserInterface/Packet.h`, both
//! under `#ifdef ENABLE_GAYA_SYSTEM`.
//!
//! `ENABLE_GAYA_SYSTEM` is `#define`d in **both** trees --
//! `server/server/common/prodomodefines.h:52` and
//! `client/Client/UserInterface/LOCALE_INC.H:94` -- and never undefined, so the
//! record is in the active build of both.
//!
//! # `pos` is a signed `int`
//!
//! The legacy field is `int`, not `DWORD` and not `BYTE`, so it is modelled as
//! little-endian **`i32`**. The handler copies it into a local `int pos` and
//! passes it to `CraftGayaItems(pos)` / `MarketGayaItems(pos)` at
//! `input_main.cpp:3196-3214`. Nothing rejects a negative value on the wire, and
//! nothing here does either: the full `i32` range round-trips.
//!
//! ## Header 241 is registered twice, and the first registration wins
//!
//! `HEADER_CG_GAYA_SYSTEM` is `241` (`packet.h:92`). So is
//! `HEADER_CG_CLIENT_VERSION2` (`packet.h:94`). In `CPacketInfoCG`:
//!
//! ```c
//! // packet_info.cpp:167
//! Set(HEADER_CG_CLIENT_VERSION2, sizeof(TPacketCGClientVersion2), "Version");
//! ...
//! // packet_info.cpp:175
//! Set(HEADER_CG_GAYA_SYSTEM,    sizeof(TPacketCGGayaSystem),    "GayaSystemSend");
//! ```
//!
//! and `CPacketInfo::Set` is **first-wins**:
//!
//! ```c
//! void CPacketInfo::Set(int header, int iSize, const char * c_pszName)
//! {
//!     if (m_pPacketMap.find(header) != m_pPacketMap.end())
//!         return;
//!     ...
//! }
//! ```
//!
//! **So the second call is silently discarded.** Header 241 is framed as
//! **67 bytes** (`TPacketCGClientVersion2`), and the 6-byte Gaya registration
//! never enters the map. The client sends 6 bytes at
//! `client/Client/UserInterface/PythonNetworkStreamPhaseGame.cpp:5054`, `:5074`
//! and `:5094`.
//!
//! ## What that means at run time
//!
//! The framing size and the dispatch header are looked up from **two different
//! places**, and they disagree:
//!
//! - In the **main phase**, `input_main.cpp:3881` has
//!   `case HEADER_CG_GAYA_SYSTEM:` and calls `GayaSystemSend(ch, c_pData)`. The
//!   handler is reached and reads its 6 bytes correctly -- but the framing loop
//!   has already consumed 67, so the 61 surplus bytes are re-parsed as the next CG
//!   frame and close the session.
//! - In the **login phase**, `input_login.cpp:1250` has
//!   `case HEADER_CG_CLIENT_VERSION2:` and calls `Version(d->GetCharacter(),
//!   c_pData)`, so a Gaya record sent during login is handled as a client-version
//!   report and the Gaya handler is never reached.
//!
//! This is a **third live client/server incompatibility**, alongside
//! `CG_CHANGE_LANGUAGE` (server 238, client 245) and `LOGIN3` (server 66, client
//! 69). It is the mirror image of `LOGIN3`: there the record is too *short* in the
//! server's registration, here it is too *long*.
//!
//! ## What this codec does about it
//!
//! It decodes the 6-byte source record, because that is what the source declares
//! and what the handler reads. It does **not** pretend the server frames 6 bytes,
//! and it does **not** change the 67-byte `0xf1` record in
//! [`crate::cg_client_version`], which *is* the live framing. A live adapter must
//! resolve the 241 collision explicitly; the codec layer is where the two shapes
//! meet, not where the conflict is silently settled.
//!
//! # The sub-header is a shared enum with **no** `default:`
//!
//! `GAYA_SYSTEM_SUB_HEADER_CRAFT`, `_MARKET`, `_REFRESH` at `packet.h:2877-2881`.
//! `CInputMain::GayaSystemSend` `switch`es on them with no `default:` arm, so an
//! unrecognised value does nothing at all. Same rule as the sash and `DailyGift`
//! sub-headers: preserve all 256, police none.

use crate::cg_inventory::{CgHeader, HEADER_CG_GAYA_SYSTEM};
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGGayaSystem` record, header byte included.
pub const CG_GAYA_SYSTEM_WIRE_SIZE: usize = 1 + 1 + 4;
/// The framed payload of `TPacketCGGayaSystem`.
pub const CG_GAYA_SYSTEM_PAYLOAD_SIZE: usize = CG_GAYA_SYSTEM_WIRE_SIZE - 1;

/// The width the server's framing table actually uses for header 241.
///
/// `CPacketInfo::Set` is first-wins and `HEADER_CG_CLIENT_VERSION2` registers 241
/// first, so this 6-byte record is **not** the size the framing loop consumes.
/// Recorded as a named constant so the collision is visible in code.
pub const CG_GAYA_SYSTEM_SHADOWED_BY: &str = "TPacketCGClientVersion2";

/// Every way the Gaya-system decoder can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgGayaSystemError {
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

impl core::fmt::Display for CgGayaSystemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated gaya-system: need {needed} bytes, got {available}"
                )
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "gaya-system length mismatch: expected {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(
                    f,
                    "invalid gaya-system header: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for CgGayaSystemError {}

/// The 6-byte `GayaSystemSend` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgGayaSystem {
    /// The legacy `BYTE subheader`, a shared-enum value with no `default:` arm.
    pub subheader: u8,
    /// The legacy **`int pos`**, little-endian and **signed**.
    pub pos: i32,
}

impl CgGayaSystem {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_GAYA_SYSTEM
    }
    /// The full legacy record width, header byte included.
    pub const WIRE_SIZE: usize = CG_GAYA_SYSTEM_WIRE_SIZE;
    /// The framed payload width, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_GAYA_SYSTEM_PAYLOAD_SIZE;

    /// Build the record.
    pub const fn new(subheader: u8, pos: i32) -> Self {
        Self { subheader, pos }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.subheader);
        out.extend_from_slice(&self.pos.to_le_bytes());
    }

    /// Encode to a fresh 6-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 5-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame {
            header: Self::header().value(),
            payload: {
                let mut p = Vec::with_capacity(Self::PAYLOAD_SIZE);
                p.push(self.subheader);
                p.extend_from_slice(&self.pos.to_le_bytes());
                p
            },
        }
    }

    /// # Errors
    ///
    /// [`CgGayaSystemError::Truncated`] below 6 bytes,
    /// [`CgGayaSystemError::LengthMismatch`] above,
    /// [`CgGayaSystemError::InvalidHeader`] for a full-length slice not starting
    /// with 241.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgGayaSystemError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(CgGayaSystemError::Truncated {
                needed: Self::WIRE_SIZE,
                available: bytes.len(),
            });
        }
        if bytes.len() > Self::WIRE_SIZE {
            return Err(CgGayaSystemError::LengthMismatch {
                expected: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header().value() {
            return Err(CgGayaSystemError::InvalidHeader {
                expected: Self::header().value(),
                actual: bytes[0],
            });
        }
        Ok(Self {
            subheader: bytes[1],
            pos: i32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
        })
    }

    /// # Errors
    ///
    /// As [`CgGayaSystem::decode`], except that the payload must be exactly 5
    /// bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgGayaSystemError> {
        if frame.payload.len() < Self::PAYLOAD_SIZE {
            return Err(CgGayaSystemError::Truncated {
                needed: Self::PAYLOAD_SIZE,
                available: frame.payload.len(),
            });
        }
        if frame.payload.len() > Self::PAYLOAD_SIZE {
            return Err(CgGayaSystemError::LengthMismatch {
                expected: Self::PAYLOAD_SIZE,
                actual: frame.payload.len(),
            });
        }
        if frame.header != Self::header().value() {
            return Err(CgGayaSystemError::InvalidHeader {
                expected: Self::header().value(),
                actual: frame.header,
            });
        }
        Ok(Self {
            subheader: frame.payload[0],
            pos: i32::from_le_bytes([
                frame.payload[1],
                frame.payload[2],
                frame.payload[3],
                frame.payload[4],
            ]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u8 = 241;

    #[test]
    fn the_header_is_241() {
        assert_eq!(CgGayaSystem::header().value(), H);
        assert_eq!(H, 0xf1);
    }

    #[test]
    fn the_record_is_six_bytes() {
        assert_eq!(1 + 1 + 4, 6);
        assert_eq!(CG_GAYA_SYSTEM_WIRE_SIZE, 6);
        assert_eq!(CgGayaSystem::WIRE_SIZE, 6);
        assert_eq!(CgGayaSystem::PAYLOAD_SIZE, 5);
    }

    #[test]
    fn the_shadowing_record_is_named_in_code() {
        // The 241 collision is a code-visible fact, not only prose.
        assert_eq!(CG_GAYA_SYSTEM_SHADOWED_BY, "TPacketCGClientVersion2");
    }

    #[test]
    fn encodes_to_the_exact_legacy_bytes() {
        assert_eq!(CgGayaSystem::new(0, 0).encode(), vec![0xf1, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn the_position_is_a_little_endian_signed_int() {
        let r = CgGayaSystem::new(1, 0x0102_0304);
        assert_eq!(&r.encode()[2..6], &[0x04, 0x03, 0x02, 0x01]);
        // A negative value is the two's-complement the legacy `int` would hold.
        let n = CgGayaSystem::new(0, -1);
        assert_eq!(&n.encode()[2..6], &[0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn the_whole_signed_range_round_trips() {
        for v in [0i32, 1, -1, i32::MAX, i32::MIN, 0x7FFF, -0x8000] {
            let r = CgGayaSystem::new(0, v);
            assert_eq!(CgGayaSystem::decode(&r.encode()).unwrap().pos, v, "pos {v}");
        }
    }

    #[test]
    fn a_negative_position_is_not_rejected() {
        // Nothing on the wire path checks the sign, so nothing here does.
        let r = CgGayaSystem::new(0, -12345);
        assert_eq!(CgGayaSystem::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn every_payload_byte_is_read() {
        let base = CgGayaSystem::default();
        let mut changed = 0;
        for i in 1..6 {
            let mut b = base.encode();
            b[i] = b[i].wrapping_add(0x11);
            if CgGayaSystem::decode(&b).unwrap() != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 5);
    }

    #[test]
    fn the_subheader_and_position_are_independent() {
        let a = CgGayaSystem::new(0, 1);
        let b = CgGayaSystem::new(1, 0);
        assert_ne!(a, b);
        assert_eq!(CgGayaSystem::decode(&a.encode()).unwrap(), a);
        assert_eq!(CgGayaSystem::decode(&b.encode()).unwrap(), b);
    }

    #[test]
    fn every_subheader_round_trips() {
        // No default: arm means unknown values are silently ignored, so all 256
        // are representable.
        for v in 0..=255_u8 {
            let r = CgGayaSystem::new(v, 0);
            assert_eq!(CgGayaSystem::decode(&r.encode()).unwrap().subheader, v);
        }
    }

    #[test]
    fn round_trips() {
        let r = CgGayaSystem::new(2, -999);
        assert_eq!(CgGayaSystem::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn round_trips_through_a_frame() {
        let r = CgGayaSystem::new(2, 0x7FFF_FFFF);
        let f = r.to_frame();
        assert_eq!(f.header, H);
        assert_eq!(f.payload.len(), 5);
        assert_eq!(CgGayaSystem::decode_frame(&f).unwrap(), r);
        assert_eq!(&f.payload[..], &r.encode()[1..]);
    }

    #[test]
    fn rejects_every_short_length() {
        for len in 0..6 {
            let mut b = CgGayaSystem::new(1, 2).encode();
            b.truncate(len);
            assert_eq!(
                CgGayaSystem::decode(&b).unwrap_err(),
                CgGayaSystemError::Truncated {
                    needed: 6,
                    available: len
                },
                "len {len}"
            );
        }
    }

    #[test]
    fn rejects_every_long_length() {
        for extra in 1..=3 {
            let mut b = CgGayaSystem::new(1, 2).encode();
            b.extend(std::iter::repeat_n(0u8, extra));
            assert_eq!(
                CgGayaSystem::decode(&b).unwrap_err(),
                CgGayaSystemError::LengthMismatch {
                    expected: 6,
                    actual: 6 + extra
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
            let mut b = CgGayaSystem::new(1, 2).encode();
            b[0] = v;
            assert_eq!(
                CgGayaSystem::decode(&b).unwrap_err(),
                CgGayaSystemError::InvalidHeader {
                    expected: 241,
                    actual: v
                },
                "header {v}"
            );
        }
    }

    #[test]
    fn rejects_every_wrong_frame_length() {
        for len in 0..5 {
            let f = ClientFrame {
                header: H,
                payload: vec![0; len],
            };
            assert!(CgGayaSystem::decode_frame(&f).is_err(), "len {len}");
        }
        let mut f = CgGayaSystem::new(1, 2).to_frame();
        f.payload.push(0);
        assert!(CgGayaSystem::decode_frame(&f).is_err());
    }

    #[test]
    fn the_error_type_displays() {
        let rendered = |e: CgGayaSystemError| e.to_string();
        assert!(rendered(CgGayaSystemError::Truncated {
            needed: 6,
            available: 0
        })
        .contains('6'));
        assert!(rendered(CgGayaSystemError::LengthMismatch {
            expected: 6,
            actual: 8
        })
        .contains('8'));
        assert!(rendered(CgGayaSystemError::InvalidHeader {
            expected: 241,
            actual: 1
        })
        .contains("241"));
        let _: &dyn std::error::Error = &CgGayaSystemError::Truncated {
            needed: 6,
            available: 0,
        };
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE];
        CgGayaSystem::new(1, 2).encode_into(&mut out);
        assert_eq!(out.len(), 1 + 6);
        assert_eq!(out[1], 0xf1);
    }
}
