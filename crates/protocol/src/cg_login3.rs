//! The 66-byte `Login3` record -- and the one live client/server incompatibility
//! found in the whole CG inventory.
//!
//! ```c
//! // server/server/game/packet.h:480-489
//! typedef struct SPacketCGLogin3
//! {
//!     BYTE header;
//!     char login[LOGIN_MAX_LEN+1];
//!     char passwd[PASSWD_MAX_LEN+1];
//!     DWORD adwClientKey[4];
//! #ifdef __MULTI_LANGUAGE_SYSTEM__
//!     BYTE bLanguage;
//! #endif
//! } TPacketCGLogin3;
//! ```
//!
//! # Width
//!
//! `LOGIN_MAX_LEN` is 30 and `PASSWD_MAX_LEN` is 16 at
//! `server/server/common/length.h:11-12`, so the fields are 31 and 17. That
//! gives `1 + 31 + 17 + 16 + 1 = 66`, which closes against
//! `Set(HEADER_CG_LOGIN3, sizeof(TPacketCGLogin3), "Login3")` at
//! `packet_info.cpp:109`.
//!
//! # The client sends 69 bytes and the server consumes 66
//!
//! The client's record at `client/Client/UserInterface/Packet.h:436-445` is the
//! same **except `bLanguage` is a `DWORD`** under `#ifdef
//! ENABLE_MULTI_LANGUAGE_SYSTEM`, giving `1 + 31 + 17 + 16 + 4 = 69`.
//!
//! Both language features are on, and neither is ever undefined:
//!
//! - server `common/prodomodefines.h:133` `#define __MULTI_LANGUAGE_SYSTEM__`.
//!   The file's only unbalanced `#endif` at :139 closes the include guard opened
//!   at :1, so the define is live. It is reached through
//!   `server/server/game/stdafx.h:16`.
//! - client `LOCALE_INC.H:110` `#define ENABLE_MULTI_LANGUAGE_SYSTEM`, inside
//!   `#ifdef PRODOMO_VERSION_5` at :83..:151, with `PRODOMO_VERSION_5` defined at
//!   :8, reached through `StdAfx.h:33`.
//!
//! No `#undef` for either macro exists anywhere in either tree.
//!
//! `CAccountConnector::__AuthState_RecvPhase` at
//! `client/Client/UserInterface/AccountConnector.cpp:180-202` builds the record
//! and calls `Send(sizeof(LoginPacket), &LoginPacket)`, and that `sizeof` is 69.
//! The language value lands in the low byte, which the server's `BYTE` reads
//! correctly, and **the 3 surplus bytes stay in the stream**. `input.cpp:92-93`
//! only requires `m_iBufferLeft >= iPacketLen` (66) and `input.cpp:112-113`
//! advances exactly 66, so the 3 leftover bytes are parsed as the next frames.
//! They are the upper bytes of a language below 256, so each is a zero, and
//! `CInputProcessor::Process` takes header 0 as a one-byte frame it consumes
//! without analysing (`input.cpp:81-82`). The Rewrite's framing does the same
//! ([`crate::cg_wire::resolve_client_frame_size`]), so the client's record
//! costs nothing but three skipped bytes.
//!
//! An earlier note here said the surplus bytes closed the session. That was
//! wrong: it missed the header-0 arm, which is checked before the packet-info
//! lookup (ledger 184).
//!
//! ## What this codec does about it
//!
//! It models the **server's active 66-byte profile** and nothing else, because
//! that is what the server consumes: 66 bytes, then three zero headers. A
//! 69-byte record never reaches the decoder as one frame. [`CG_LOGIN3_CLIENT_WIDTH`]
//! records the 69-byte client width as a named constant so the difference is a
//! visible fact in code, and [`CG_LOGIN3_PROFILE_DIVERGENCE`] records it.
//!
//! # The language field is read
//!
//! `CInputAuth::Login` copies the whole record into the `ReturnQuery` data, and
//! the `QID_AUTH_LOGIN` result reads `pinfo->bLanguage` twice under
//! `__MULTI_LANGUAGE_SYSTEM__`: a value at or above `LOCALE_MAX_NUM` (12) is
//! refused with `INVLANG` and zero with `NOLANG`, and an accepted value is
//! written to `account.language` (`db.cpp`, `QID_AUTH_LOGIN`). An earlier note
//! here said the field was never read (ledger 184 corrects it).
//!
//! # `login` and `passwd` are raw storage
//!
//! Both stay `[u8; N]`. The client writes terminators at
//! `AccountConnector.cpp:185-186`, but a hostile client need not, and the server
//! reads through bounded calls rather than a length field. A NUL rule would
//! reject records the legacy server accepts, so there is no `&str` and no
//! `CStr`.
//!
//! The two fields are declared with `#pragma pack(1)` like everything else here,
//! but the layout needs no packing: the `DWORD[4]` is already 4-byte aligned
//! after 1 + 31 + 17 = 49 bytes, and the natural alignment of the whole struct
//! is 4 with size 68. The pack is what makes it 66. Stated rather than assumed.
//!
//! # Header 111 collides with a live `GC` record of a different size
//!
//! `packet.h:77` has `HEADER_CG_LOGIN3 = 111` and `packet.h:186` has
//! `HEADER_GC_WALK_MODE = 111` **in the same file**; the client mirrors both at
//! `Packet.h:75` and `Packet.h:188`. The GC record is `BYTE header`,
//! `DWORD vid`, `BYTE mode` = 6 bytes packed (`packet.h:2383-2388`), and it is
//! **live**: sent at `char.cpp:1217`, `:1230` and `:7362`, registered inbound as a
//! fixed 6-byte static packet at `PythonNetworkStream.cpp:123`, and dispatched to
//! `RecvWalkModePacket` at `PythonNetworkStreamPhaseGame.cpp:532-533`.
//!
//! So 111 is 66 bytes one way and 6 the other. **Only the direction
//! disambiguates them**, which is why the two directions must never share a
//! header-keyed dispatch table.
//!
//! # Where it is dispatched
//!
//! Exactly one place: `input_auth.cpp:215-217` inside the `switch (bHeader)` at
//! :209 in `CInputAuth::Analyze`. The other six `::Analyze` files have no hits. A
//! `Login3` arriving in the LOGIN phase falls to `default:` at
//! `input_login.cpp:1260-1263`, which `sys_err`s and returns 0 -- with the
//! `SetPhase(PHASE_CLOSE)` on :1262 already commented out.

use crate::cg_account::{CG_CLIENT_KEY_WORDS, CG_LOGIN_FIELD_BYTES, CG_PASSWORD_FIELD_BYTES};
use crate::cg_inventory::{CgHeader, HEADER_CG_LOGIN3};
use crate::cg_wire::ClientFrame;

/// The full legacy `TPacketCGLogin3` record on the **server**, header included.
///
/// `1 + 31 + 17 + 4 * 4 + 1`.
pub const CG_LOGIN3_WIRE_SIZE: usize =
    1 + CG_LOGIN_FIELD_BYTES + CG_PASSWORD_FIELD_BYTES + (CG_CLIENT_KEY_WORDS * 4) + 1;

/// The framed payload of `TPacketCGLogin3` on the server.
pub const CG_LOGIN3_PAYLOAD_SIZE: usize = CG_LOGIN3_WIRE_SIZE - 1;

/// The width the **client** puts on the wire for the same record.
///
/// Recorded, not decoded: the client's `bLanguage` is a `DWORD`, so its
/// `sizeof(TPacketCGLogin3)` is 69. Accepting this width on a server socket would
/// desynchronise framing, because the server's registration is 66.
pub const CG_LOGIN3_CLIENT_WIDTH: usize =
    1 + CG_LOGIN_FIELD_BYTES + CG_PASSWORD_FIELD_BYTES + (CG_CLIENT_KEY_WORDS * 4) + 4;

/// How many bytes the client sends that the server does not consume.
pub const CG_LOGIN3_PROFILE_DIVERGENCE: usize = CG_LOGIN3_CLIENT_WIDTH - CG_LOGIN3_WIRE_SIZE;

/// Every way the `Login3` decoder can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgLogin3Error {
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
    /// The slice was exactly the **client's** 69-byte width.
    ///
    /// Reported separately from [`CgLogin3Error::LengthMismatch`] because it is
    /// a known, named incompatibility rather than a random wrong size, and a
    /// caller instrumenting a live socket wants to see it distinctly.
    ClientWidth,
}

impl core::fmt::Display for CgLogin3Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "truncated login3: need {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "login3 length mismatch: expected {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(
                    f,
                    "invalid login3 header: expected {expected}, got {actual}"
                )
            }
            Self::ClientWidth => write!(
                f,
                "login3 arrived at the {CG_LOGIN3_CLIENT_WIDTH} byte client width, but the \
                 server registration is {CG_LOGIN3_WIRE_SIZE}; the surplus desynchronises framing"
            ),
        }
    }
}

impl std::error::Error for CgLogin3Error {}

/// The 66-byte server `Login3` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgLogin3 {
    /// The legacy `char login[31]`, as raw bytes. No terminator is required.
    pub login: [u8; CG_LOGIN_FIELD_BYTES],
    /// The legacy `char passwd[17]`, as raw bytes. No terminator is required.
    pub passwd: [u8; CG_PASSWORD_FIELD_BYTES],
    /// The legacy `DWORD adwClientKey[4]`, four little-endian words in order.
    pub adw_client_key: [u32; CG_CLIENT_KEY_WORDS],
    /// The legacy `BYTE bLanguage`: the low byte of the client's `DWORD`. The
    /// auth result refuses 0 and anything from 12 up, and stores the rest as the
    /// account's language.
    pub b_language: u8,
}

impl CgLogin3 {
    /// The legacy header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_LOGIN3
    }
    /// The full legacy record width on the **server**, header included.
    pub const WIRE_SIZE: usize = CG_LOGIN3_WIRE_SIZE;
    /// The framed payload width on the server, everything after the header.
    pub const PAYLOAD_SIZE: usize = CG_LOGIN3_PAYLOAD_SIZE;
    /// The width the **client** sends, recorded not decoded.
    pub const CLIENT_WIDTH: usize = CG_LOGIN3_CLIENT_WIDTH;

    /// Build the record.
    pub const fn new(
        login: [u8; CG_LOGIN_FIELD_BYTES],
        passwd: [u8; CG_PASSWORD_FIELD_BYTES],
        adw_client_key: [u32; CG_CLIENT_KEY_WORDS],
        b_language: u8,
    ) -> Self {
        Self {
            login,
            passwd,
            adw_client_key,
            b_language,
        }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.extend_from_slice(&self.login);
        out.extend_from_slice(&self.passwd);
        for word in self.adw_client_key {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.push(self.b_language);
    }

    /// Encode to a fresh 66-byte buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode the header-less payload to a fresh 65-byte buffer.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(Self::PAYLOAD_SIZE);
        payload.extend_from_slice(&self.login);
        payload.extend_from_slice(&self.passwd);
        for word in self.adw_client_key {
            payload.extend_from_slice(&word.to_le_bytes());
        }
        payload.push(self.b_language);
        ClientFrame {
            header: Self::header().value(),
            payload,
        }
    }

    /// # Errors
    ///
    /// [`CgLogin3Error::Truncated`] below 66 bytes,
    /// [`CgLogin3Error::LengthMismatch`] above,
    /// [`CgLogin3Error::ClientWidth`] at exactly the client's 69 bytes,
    /// [`CgLogin3Error::InvalidHeader`] for a full-length slice not starting with
    /// 111.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgLogin3Error> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(CgLogin3Error::Truncated {
                needed: Self::WIRE_SIZE,
                available: bytes.len(),
            });
        }
        if bytes.len() != Self::WIRE_SIZE {
            return if bytes.len() == Self::CLIENT_WIDTH {
                Err(CgLogin3Error::ClientWidth)
            } else {
                Err(CgLogin3Error::LengthMismatch {
                    expected: Self::WIRE_SIZE,
                    actual: bytes.len(),
                })
            };
        }
        if bytes[0] != Self::header().value() {
            return Err(CgLogin3Error::InvalidHeader {
                expected: Self::header().value(),
                actual: bytes[0],
            });
        }
        let mut login = [0u8; CG_LOGIN_FIELD_BYTES];
        login.copy_from_slice(&bytes[1..=CG_LOGIN_FIELD_BYTES]);
        let p0 = 1 + CG_LOGIN_FIELD_BYTES;
        let mut passwd = [0u8; CG_PASSWORD_FIELD_BYTES];
        passwd.copy_from_slice(&bytes[p0..p0 + CG_PASSWORD_FIELD_BYTES]);
        let k0 = p0 + CG_PASSWORD_FIELD_BYTES;
        let mut adw_client_key = [0u32; CG_CLIENT_KEY_WORDS];
        for (i, word) in adw_client_key.iter_mut().enumerate() {
            let a = k0 + i * 4;
            *word = u32::from_le_bytes([bytes[a], bytes[a + 1], bytes[a + 2], bytes[a + 3]]);
        }
        Ok(Self {
            login,
            passwd,
            adw_client_key,
            b_language: bytes[k0 + 16],
        })
    }

    /// # Errors
    ///
    /// As [`CgLogin3::decode`], except that the payload must be exactly 65
    /// bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgLogin3Error> {
        if frame.payload.len() < Self::PAYLOAD_SIZE {
            return Err(CgLogin3Error::Truncated {
                needed: Self::PAYLOAD_SIZE,
                available: frame.payload.len(),
            });
        }
        if frame.payload.len() != Self::PAYLOAD_SIZE {
            return if frame.payload.len() == Self::CLIENT_WIDTH - 1 {
                Err(CgLogin3Error::ClientWidth)
            } else {
                Err(CgLogin3Error::LengthMismatch {
                    expected: Self::PAYLOAD_SIZE,
                    actual: frame.payload.len(),
                })
            };
        }
        if frame.header != Self::header().value() {
            return Err(CgLogin3Error::InvalidHeader {
                expected: Self::header().value(),
                actual: frame.header,
            });
        }
        let mut login = [0u8; CG_LOGIN_FIELD_BYTES];
        login.copy_from_slice(&frame.payload[..CG_LOGIN_FIELD_BYTES]);
        let p0 = CG_LOGIN_FIELD_BYTES;
        let mut passwd = [0u8; CG_PASSWORD_FIELD_BYTES];
        passwd.copy_from_slice(&frame.payload[p0..p0 + CG_PASSWORD_FIELD_BYTES]);
        let k0 = p0 + CG_PASSWORD_FIELD_BYTES;
        let mut adw_client_key = [0u32; CG_CLIENT_KEY_WORDS];
        for (i, word) in adw_client_key.iter_mut().enumerate() {
            let a = k0 + i * 4;
            *word = u32::from_le_bytes([
                frame.payload[a],
                frame.payload[a + 1],
                frame.payload[a + 2],
                frame.payload[a + 3],
            ]);
        }
        Ok(Self {
            login,
            passwd,
            adw_client_key,
            b_language: frame.payload[k0 + 16],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u8 = 111;

    /// A realistic record: 30 name bytes, 16 password bytes, 4 key words.
    fn rec() -> CgLogin3 {
        let mut login = [0u8; CG_LOGIN_FIELD_BYTES];
        login[..10].copy_from_slice(b"someplayer");
        let mut passwd = [0u8; CG_PASSWORD_FIELD_BYTES];
        passwd[..7].copy_from_slice(b"hunter2");
        CgLogin3::new(
            login,
            passwd,
            [0x1122_3344, 0x5566_7788, 0x99AA_BBCC, 0xDDEE_FF00],
            7,
        )
    }

    #[test]
    fn the_header_is_111() {
        assert_eq!(CgLogin3::header().value(), H);
        assert_eq!(H, 0x6f);
    }

    #[test]
    fn the_server_record_is_66_bytes() {
        assert_eq!(CG_LOGIN_FIELD_BYTES, 31);
        assert_eq!(CG_PASSWORD_FIELD_BYTES, 17);
        assert_eq!(CG_CLIENT_KEY_WORDS, 4);
        assert_eq!(1 + 31 + 17 + 16 + 1, 66);
        assert_eq!(CG_LOGIN3_WIRE_SIZE, 66);
        assert_eq!(CgLogin3::WIRE_SIZE, 66);
        assert_eq!(CgLogin3::PAYLOAD_SIZE, 65);
    }

    #[test]
    fn the_client_record_is_69_bytes_and_the_gap_is_three() {
        assert_eq!(CG_LOGIN3_CLIENT_WIDTH, 69);
        assert_eq!(CgLogin3::CLIENT_WIDTH, 69);
        assert_eq!(CG_LOGIN3_PROFILE_DIVERGENCE, 3);
        // The gap is the `DWORD` language field minus the `BYTE` one.
        assert_eq!(CG_LOGIN3_PROFILE_DIVERGENCE, 4 - 1);
    }

    #[test]
    fn the_client_width_is_rejected_distinctly() {
        // Accepting it would desynchronise framing, so it is not a plain
        // "wrong size": it is the named incompatibility.
        let mut b = rec().encode();
        b.extend_from_slice(&[0, 0, 0]);
        assert_eq!(b.len(), 69);
        assert_eq!(
            CgLogin3::decode(&b).unwrap_err(),
            CgLogin3Error::ClientWidth
        );
    }

    #[test]
    fn the_client_width_is_rejected_distinctly_through_a_frame() {
        let mut f = rec().to_frame();
        f.payload.extend_from_slice(&[0, 0, 0]);
        assert_eq!(
            CgLogin3::decode_frame(&f).unwrap_err(),
            CgLogin3Error::ClientWidth
        );
    }

    #[test]
    fn encoding_the_record_never_produces_the_client_width() {
        for v in 0..=255_u8 {
            let r = CgLogin3::new(
                [0; CG_LOGIN_FIELD_BYTES],
                [0; CG_PASSWORD_FIELD_BYTES],
                [0; 4],
                v,
            );
            assert_eq!(r.encode().len(), 66, "language {v}");
        }
    }

    #[test]
    fn encodes_to_the_exact_legacy_bytes() {
        let b = rec().encode();
        assert_eq!(b.len(), 66);
        assert_eq!(b[0], H);
        assert_eq!(&b[1..11], b"someplayer");
        assert_eq!(&b[32..39], b"hunter2");
        // The key words start at 1 + 31 + 17 = 49.
        assert_eq!(&b[49..53], &[0x44, 0x33, 0x22, 0x11]);
        assert_eq!(&b[53..57], &[0x88, 0x77, 0x66, 0x55]);
        assert_eq!(&b[57..61], &[0xCC, 0xBB, 0xAA, 0x99]);
        assert_eq!(&b[61..65], &[0x00, 0xFF, 0xEE, 0xDD]);
        // And the language byte is the last one.
        assert_eq!(b[65], 7);
    }

    #[test]
    fn the_key_words_are_little_endian_at_49() {
        let r = CgLogin3::new([0; 31], [0; 17], [1, 2, 3, 4], 0);
        let b = r.encode();
        assert_eq!(&b[49..53], &[1, 0, 0, 0]);
        assert_eq!(&b[61..65], &[4, 0, 0, 0]);
    }

    #[test]
    fn the_full_key_word_range_round_trips() {
        for w in [0u32, 1, 0xFFFF, 0x1_0000, u32::MAX] {
            let r = CgLogin3::new([0; 31], [0; 17], [w; 4], 0);
            assert_eq!(
                CgLogin3::decode(&r.encode()).unwrap().adw_client_key,
                [w; 4]
            );
        }
    }

    #[test]
    fn the_key_words_are_independent() {
        let r = CgLogin3::new([0; 31], [0; 17], [1, 2, 3, 4], 0);
        let got = CgLogin3::decode(&r.encode()).unwrap();
        assert_eq!(got.adw_client_key, [1, 2, 3, 4]);
    }

    #[test]
    fn every_language_byte_round_trips() {
        for v in 0..=255_u8 {
            let r = CgLogin3::new([0; 31], [0; 17], [0; 4], v);
            assert_eq!(CgLogin3::decode(&r.encode()).unwrap().b_language, v);
        }
    }

    #[test]
    fn every_credential_byte_is_accepted() {
        // No byte value is reserved, and no terminator is required.
        for v in 0..=255_u8 {
            let mut login = [0u8; 31];
            login[30] = v;
            let mut passwd = [0u8; 17];
            passwd[16] = v;
            let r = CgLogin3::new(login, passwd, [0; 4], 0);
            let got = CgLogin3::decode(&r.encode()).unwrap();
            assert_eq!(got.login, login, "login byte {v}");
            assert_eq!(got.passwd, passwd, "passwd byte {v}");
        }
    }

    #[test]
    fn an_unterminated_login_is_accepted() {
        let login = [b'A'; 31];
        let r = CgLogin3::new(login, [0; 17], [0; 4], 0);
        assert_eq!(CgLogin3::decode(&r.encode()).unwrap().login, login);
    }

    #[test]
    fn every_payload_byte_is_read() {
        let base = CgLogin3::default();
        let mut changed = 0;
        for i in 1..66 {
            let mut b = base.encode();
            b[i] = b[i].wrapping_add(0x11);
            if CgLogin3::decode(&b).unwrap() != base {
                changed += 1;
            }
        }
        assert_eq!(changed, 65);
    }

    #[test]
    fn round_trips() {
        let r = rec();
        assert_eq!(CgLogin3::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn round_trips_through_a_frame() {
        let r = rec();
        let f = r.to_frame();
        assert_eq!(f.header, H);
        assert_eq!(f.payload.len(), 65);
        assert_eq!(CgLogin3::decode_frame(&f).unwrap(), r);
        assert_eq!(&f.payload[..], &r.encode()[1..]);
    }

    #[test]
    fn rejects_every_short_length() {
        for len in 0..66 {
            let mut b = rec().encode();
            b.truncate(len);
            assert_eq!(
                CgLogin3::decode(&b).unwrap_err(),
                CgLogin3Error::Truncated {
                    needed: 66,
                    available: len
                },
                "len {len}"
            );
        }
    }

    #[test]
    fn rejects_every_long_length_except_the_named_client_one() {
        for len in 67..=80 {
            // Pad, not truncate: the encoding is 66 bytes, so truncating to 67
            // or more would leave it untouched and decode successfully.
            let mut b = rec().encode();
            b.resize(len, 0);
            assert_eq!(b.len(), len);
            if len == 69 {
                assert_eq!(
                    CgLogin3::decode(&b).unwrap_err(),
                    CgLogin3Error::ClientWidth
                );
            } else {
                assert_eq!(
                    CgLogin3::decode(&b).unwrap_err(),
                    CgLogin3Error::LengthMismatch {
                        expected: 66,
                        actual: len
                    },
                    "len {len}"
                );
            }
        }
    }

    #[test]
    fn rejects_every_wrong_header() {
        for v in 0..=255_u8 {
            if v == H {
                continue;
            }
            let mut b = rec().encode();
            b[0] = v;
            assert_eq!(
                CgLogin3::decode(&b).unwrap_err(),
                CgLogin3Error::InvalidHeader {
                    expected: 111,
                    actual: v
                },
                "header {v}"
            );
        }
    }

    #[test]
    fn the_error_type_displays() {
        assert!(CgLogin3Error::Truncated {
            needed: 66,
            available: 0
        }
        .to_string()
        .contains("66"));
        assert!(CgLogin3Error::LengthMismatch {
            expected: 66,
            actual: 70
        }
        .to_string()
        .contains("70"));
        assert!(CgLogin3Error::InvalidHeader {
            expected: 111,
            actual: 1
        }
        .to_string()
        .contains("111"));
        // The named divergence must name both widths in its message.
        let s = CgLogin3Error::ClientWidth.to_string();
        assert!(s.contains("69") && s.contains("66"), "{s}");
        let _: &dyn std::error::Error = &CgLogin3Error::ClientWidth;
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE];
        rec().encode_into(&mut out);
        assert_eq!(out.len(), 1 + 66);
        assert_eq!(out[1], H);
    }
}
