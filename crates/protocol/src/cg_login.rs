//! Transport-free codecs for the three CG records that the handshake and login
//! phases dispatch.
//!
//! These three are grouped by **phase boundary**, not by domain, and not by
//! width. That grouping is deliberate: the record that makes the boundary
//! interesting is [`CgPong`], which is the only CG record in the workspace that
//! is dispatched in *every* client phase -- handshake, auth, login and game --
//! so it belongs to the boundary module even though it is a pure keepalive.
//!
//! The four-phase claim was originally written as "three different phases" and
//! was wrong: the auth phase was missing from the search set. See ledger 160.2
//! and 160.3.
//!
//! | record | header | wire | payload | declaration | registration |
//! |---|---|---|---|---|---|
//! | `TPacketCGStateCheck` | 206 `0xce` | 1 | 0 | `packet_info.cpp:173` | `packet_info.cpp:173` |
//! | `TPacketCGEmpire` | 90 `0x5a` | 2 | 1 | `packet.h:2109-2113` | `packet_info.cpp` |
//! | *(no struct)* `Pong` | 254 `0xfe` | 1 | 0 | — | `sizeof(BYTE)` |
//!
//! The two one-byte records are registered as `sizeof(BYTE)` rather than
//! `sizeof(TSomething)`, which is why the table has no declaration column for
//! them; there is no `TPacketCGPong` and no `TPacketCGStateCheck` struct
//! anywhere in the server tree. Both are genuinely header-only, and both are
//! confirmed by their handlers never reading `c_pData`.
//!
//! # `CgStateChecker` is dispatched by an `else if`, not a `case`
//!
//! This record was previously recorded as having no live dispatch. That was
//! wrong, and the reason is worth stating in the module itself, because the
//! mistake is easy to repeat.
//!
//! The game and login phases dispatch with `switch`/`case`:
//! `CInputMain::Analyze` and `CInputLogin::Analyze`. The **handshake** phase
//! dispatches with an `else if` chain in `CInputHandshake::Analyze`
//! (`input.cpp:209`). `HEADER_CG_STATE_CHECKER` is handled at `input.cpp:227` in
//! that chain, so a search for `case HEADER_CG_STATE_CHECKER` finds only the
//! dead occurrence inside the `/* ... */` block at `input_udp.cpp:106-114` and
//! reports the record as dead.
//!
//! The live handler:
//!
//! ```text
//! else if (bHeader == HEADER_CG_STATE_CHECKER)
//! {
//!     if (d->isChannelStatusRequested()) {
//!         return 0;
//!     }
//!     d->SetChannelStatusRequested(true);
//!     db_clientdesc->DBPacket(HEADER_GD_REQUEST_CHANNELSTATUS, d->GetHandle(), NULL, 0);
//! }
//! ```
//!
//! It is a one-shot: the `isChannelStatusRequested()` flag at `desc.h:178-179`
//! means only the first such record per descriptor reaches the DB, and every
//! later one returns 0 immediately. **That is session policy, not framing**, and
//! it is not modelled here — this module decodes a byte and stops.
//!
//! # What the other two handlers do, and why none of it is framing
//!
//! - `CInputProcessor::Pong` at `input.cpp:126` is `d->SetPong(true);` and
//!   nothing else. It takes only the descriptor — no `c_pData` parameter at
//!   all — so the record cannot carry a payload. It is dispatched from six
//!   sites across four files: `input.cpp:107` (a log line), `input.cpp:235`,
//!   `input_auth.cpp:211`, `input_login.cpp:1169`, and `input_main.cpp:3661` and
//!   `:4102` for the living and dead game paths. The auth-phase arm is
//!   `CInputAuth::Analyze`, which is itself gated on `g_bAuthServer`; see
//!   ledger 160.4.
//! - `CInputLogin::Empire` at `input_login.cpp:963` reads `p->bEmpire` and then
//!   applies **session policy**: it closes the descriptor when
//!   `EMPIRE_MAX_NUM <= p->bEmpire`, and it refuses to re-select an empire for
//!   an account that already has characters. The range check is exactly the
//!   kind of rule that must not leak into a codec, so [`CgEmpire::empire`] is
//!   opaque across all 256 values and the close-on-out-of-range behaviour stays
//!   documented here rather than encoded.
//!
//! The log line at `input.cpp:107-110` is worth a note because it looks like an
//! out-of-bounds read and is not:
//!
//! ```cpp
//! sys_log(0, "PONG! %u", *(BYTE*)(c_pData + iPacketLen - sizeof(BYTE)));
//! ```
//!
//! `c_pData` points at the *header* byte (`input.cpp:78` reads the header from
//! it), and `iPacketLen` at that point is the **total record length** from
//! `CPacketInfo::Get`, which is 1 for a `sizeof(BYTE)` record. So the expression
//! is `c_pData + 1 - 1` — the header byte itself, in bounds. The line logs the
//! header value. If `iPacketLen` were a payload length this would read one byte
//! past the record; that it does not is an accident of `iPacketLen` meaning
//! something different in this function than the `iExtraLen` of the login and
//! game analyzers, and it is recorded so the idiom is not misread as safe in
//! general.

use crate::cg_inventory::{CgHeader, HEADER_CG_EMPIRE, HEADER_CG_PONG, HEADER_CG_STATE_CHECKER};
use crate::cg_wire::ClientFrame;

/// The full legacy `Pong` record. It is a header and nothing else.
pub const CG_PONG_WIRE_SIZE: usize = 1;
/// The framed payload of `Pong`, which is empty by construction.
pub const CG_PONG_PAYLOAD_SIZE: usize = 0;
/// The full legacy `ServerStateCheck` record. It is a header and nothing else.
pub const CG_STATE_CHECKER_WIRE_SIZE: usize = 1;
/// The framed payload of `ServerStateCheck`, which is empty by construction.
pub const CG_STATE_CHECKER_PAYLOAD_SIZE: usize = 0;
/// The full legacy `TPacketCGEmpire` record, header byte included.
pub const CG_EMPIRE_WIRE_SIZE: usize = 2;
/// The framed payload of `TPacketCGEmpire`, everything after the header.
pub const CG_EMPIRE_PAYLOAD_SIZE: usize = 1;

/// Every way one of these three decoders can refuse a byte slice.
///
/// The three failure modes are identical across all three records, so they
/// share one error type. `InvalidHeader` carries the expected value so the
/// message still names the right record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgLoginError {
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
    /// The right number of bytes, but the header byte is not this record's.
    InvalidHeader {
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl std::fmt::Display for CgLoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "CG record needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(
                    f,
                    "CG record must be exactly {expected} bytes, got {actual}"
                )
            }
            Self::InvalidHeader { expected, actual } => {
                write!(f, "CG header {actual} is not {expected}")
            }
        }
    }
}

impl std::error::Error for CgLoginError {}

/// Check a complete record width, then its header.
fn decode_parts(bytes: &[u8], wire_size: usize, expected: u8) -> Result<(), CgLoginError> {
    check_exact(bytes.len(), wire_size)?;
    check_header(bytes[0], expected)
}

/// Check a header-less frame payload width, then its header.
///
/// A zero-width payload is the normal case for two of the three records here,
/// so the empty check runs before the header is read.
fn decode_frame_parts(
    frame: &ClientFrame,
    payload_size: usize,
    expected: u8,
) -> Result<(), CgLoginError> {
    check_exact(frame.payload.len(), payload_size)?;
    check_header(frame.header, expected)
}

fn check_exact(actual: usize, expected: usize) -> Result<(), CgLoginError> {
    if actual < expected {
        return Err(CgLoginError::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(CgLoginError::LengthMismatch { expected, actual });
    }
    Ok(())
}

fn check_header(actual: u8, expected: u8) -> Result<(), CgLoginError> {
    if actual == expected {
        return Ok(());
    }
    Err(CgLoginError::InvalidHeader { expected, actual })
}

/// The legacy keepalive. A one-byte header with no payload, dispatched in the
/// handshake, login, and game phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgPong;

impl CgPong {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_PONG
    }

    /// Build the record. It carries no field.
    pub const fn new() -> Self {
        Self
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_PONG_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, which has an empty payload.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), [0_u8; 0])
    }

    /// # Errors
    ///
    /// Returns [`CgLoginError::Truncated`] for an empty input,
    /// [`CgLoginError::LengthMismatch`] for an input longer than one byte, and
    /// [`CgLoginError::InvalidHeader`] when the header is not 254.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgLoginError> {
        decode_parts(bytes, CG_PONG_WIRE_SIZE, Self::header().value())?;
        Ok(Self)
    }

    /// # Errors
    ///
    /// As [`CgPong::decode`], except that the frame payload must be **empty**.
    /// A one-byte payload is a [`CgLoginError::LengthMismatch`], not an accepted
    /// record, because `CInputProcessor::Pong` has no `c_pData` parameter and
    /// cannot read one.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgLoginError> {
        decode_frame_parts(frame, CG_PONG_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self)
    }
}

/// The handshake-phase channel-status probe. A one-byte header with no payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CgStateChecker;

impl CgStateChecker {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_STATE_CHECKER
    }

    /// Build the record. It carries no field.
    pub const fn new() -> Self {
        Self
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_STATE_CHECKER_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, which has an empty payload.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), [0_u8; 0])
    }

    /// # Errors
    ///
    /// Returns [`CgLoginError::Truncated`] for an empty input,
    /// [`CgLoginError::LengthMismatch`] for an input longer than one byte, and
    /// [`CgLoginError::InvalidHeader`] when the header is not 206.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgLoginError> {
        decode_parts(bytes, CG_STATE_CHECKER_WIRE_SIZE, Self::header().value())?;
        Ok(Self)
    }

    /// # Errors
    ///
    /// As [`CgStateChecker::decode`], except that the frame payload must be
    /// **empty**. The legacy handler at `input.cpp:227` never touches
    /// `c_pData`, so a one-byte payload is a
    /// [`CgLoginError::LengthMismatch`] rather than an accepted record.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgLoginError> {
        decode_frame_parts(frame, CG_STATE_CHECKER_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self)
    }
}

/// The login-phase empire selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgEmpire {
    /// The opaque empire byte. Legacy range-checks it in session policy.
    pub empire: u8,
}

impl CgEmpire {
    /// The fixed header byte.
    pub const fn header() -> CgHeader {
        HEADER_CG_EMPIRE
    }

    /// Build the record from the opaque empire byte.
    pub const fn new(empire: u8) -> Self {
        Self { empire }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.empire);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_EMPIRE_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame, which carries the one payload byte.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), [self.empire])
    }

    /// # Errors
    ///
    /// Returns [`CgLoginError::Truncated`] for a one-byte input,
    /// [`CgLoginError::LengthMismatch`] for an input longer than two bytes, and
    /// [`CgLoginError::InvalidHeader`] when the header is not 90.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgLoginError> {
        decode_parts(bytes, CG_EMPIRE_WIRE_SIZE, Self::header().value())?;
        Ok(Self { empire: bytes[1] })
    }

    /// # Errors
    ///
    /// As [`CgEmpire::decode`], except that the frame payload must be exactly
    /// one byte.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgLoginError> {
        decode_frame_parts(frame, CG_EMPIRE_PAYLOAD_SIZE, Self::header().value())?;
        Ok(Self {
            empire: frame.payload[0],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_inventory::{HEADER_CG_ENTERGAME, HEADER_CG_HANDSHAKE};

    const PONG: u8 = 254;
    const CHECKER: u8 = 206;
    const EMPIRE: u8 = 90;

    // ---- CgPong -------------------------------------------------------

    #[test]
    fn pong_header_is_254() {
        assert_eq!(CgPong::header().value(), PONG);
    }

    #[test]
    fn pong_encodes_to_one_byte() {
        assert_eq!(CgPong::new().encode(), vec![PONG]);
    }

    #[test]
    fn pong_wire_and_payload_sizes() {
        assert_eq!(CG_PONG_WIRE_SIZE, 1);
        assert_eq!(CG_PONG_PAYLOAD_SIZE, 0);
    }

    #[test]
    fn pong_frame_has_empty_payload() {
        let f = CgPong::new().to_frame();
        assert_eq!(f.header, PONG);
        assert_eq!(f.payload.len(), 0);
    }

    #[test]
    fn pong_roundtrips_through_a_slice() {
        let b = CgPong::new().encode();
        assert_eq!(CgPong::decode(&b), Ok(CgPong::new()));
    }

    #[test]
    fn pong_roundtrips_through_a_frame() {
        assert_eq!(
            CgPong::decode_frame(&CgPong::new().to_frame()),
            Ok(CgPong::new())
        );
    }

    #[test]
    fn pong_encode_into_appends_rather_than_replaces() {
        let mut out = vec![0xAA];
        CgPong::new().encode_into(&mut out);
        assert_eq!(out, vec![0xAA, PONG]);
    }

    #[test]
    fn pong_rejects_empty_input_as_truncated() {
        assert_eq!(
            CgPong::decode(&[]),
            Err(CgLoginError::Truncated {
                needed: 1,
                available: 0
            })
        );
    }

    #[test]
    fn pong_rejects_two_bytes_as_a_length_mismatch() {
        assert_eq!(
            CgPong::decode(&[PONG, 0x00]),
            Err(CgLoginError::LengthMismatch {
                expected: 1,
                actual: 2
            })
        );
    }

    #[test]
    fn pong_rejects_a_wrong_header_of_the_right_width() {
        assert_eq!(
            CgPong::decode(&[CHECKER]),
            Err(CgLoginError::InvalidHeader {
                expected: PONG,
                actual: CHECKER
            })
        );
    }

    #[test]
    fn pong_frame_rejects_a_non_empty_payload() {
        let f = ClientFrame::new(PONG, [0x01]);
        assert_eq!(
            CgPong::decode_frame(&f),
            Err(CgLoginError::LengthMismatch {
                expected: 0,
                actual: 1
            })
        );
    }

    #[test]
    fn pong_is_the_only_record_at_254() {
        assert_ne!(CgPong::header().value(), CgStateChecker::header().value());
        assert_ne!(CgPong::header().value(), CgEmpire::header().value());
    }

    // ---- CgStateChecker ------------------------------------------------

    #[test]
    fn state_checker_header_is_206() {
        assert_eq!(CgStateChecker::header().value(), CHECKER);
    }

    #[test]
    fn state_checker_encodes_to_one_byte() {
        assert_eq!(CgStateChecker::new().encode(), vec![CHECKER]);
    }

    #[test]
    fn state_checker_wire_and_payload_sizes() {
        assert_eq!(CG_STATE_CHECKER_WIRE_SIZE, 1);
        assert_eq!(CG_STATE_CHECKER_PAYLOAD_SIZE, 0);
    }

    #[test]
    fn state_checker_frame_has_empty_payload() {
        let f = CgStateChecker::new().to_frame();
        assert_eq!(f.header, CHECKER);
        assert_eq!(f.payload.len(), 0);
    }

    #[test]
    fn state_checker_roundtrips_through_a_slice() {
        let b = CgStateChecker::new().encode();
        assert_eq!(CgStateChecker::decode(&b), Ok(CgStateChecker::new()));
    }

    #[test]
    fn state_checker_roundtrips_through_a_frame() {
        assert_eq!(
            CgStateChecker::decode_frame(&CgStateChecker::new().to_frame()),
            Ok(CgStateChecker::new())
        );
    }

    #[test]
    fn state_checker_encode_into_appends_rather_than_replaces() {
        let mut out = vec![0xBB];
        CgStateChecker::new().encode_into(&mut out);
        assert_eq!(out, vec![0xBB, CHECKER]);
    }

    #[test]
    fn state_checker_rejects_empty_input_as_truncated() {
        assert_eq!(
            CgStateChecker::decode(&[]),
            Err(CgLoginError::Truncated {
                needed: 1,
                available: 0
            })
        );
    }

    #[test]
    fn state_checker_rejects_two_bytes_as_a_length_mismatch() {
        assert_eq!(
            CgStateChecker::decode(&[CHECKER, 0x00]),
            Err(CgLoginError::LengthMismatch {
                expected: 1,
                actual: 2
            })
        );
    }

    #[test]
    fn state_checker_rejects_a_wrong_header_of_the_right_width() {
        assert_eq!(
            CgStateChecker::decode(&[PONG]),
            Err(CgLoginError::InvalidHeader {
                expected: CHECKER,
                actual: PONG
            })
        );
    }

    #[test]
    fn state_checker_frame_rejects_a_non_empty_payload() {
        let f = ClientFrame::new(CHECKER, [0x7F]);
        assert_eq!(
            CgStateChecker::decode_frame(&f),
            Err(CgLoginError::LengthMismatch {
                expected: 0,
                actual: 1
            })
        );
    }

    // ---- CgEmpire -----------------------------------------------------

    #[test]
    fn empire_header_is_90() {
        assert_eq!(CgEmpire::header().value(), EMPIRE);
    }

    #[test]
    fn empire_wire_and_payload_sizes() {
        assert_eq!(CG_EMPIRE_WIRE_SIZE, 2);
        assert_eq!(CG_EMPIRE_PAYLOAD_SIZE, 1);
    }

    #[test]
    fn empire_encodes_header_then_byte() {
        assert_eq!(CgEmpire::new(3).encode(), vec![EMPIRE, 0x03]);
    }

    #[test]
    fn empire_frame_carries_the_payload_byte() {
        let f = CgEmpire::new(3).to_frame();
        assert_eq!(f.header, EMPIRE);
        assert_eq!(f.payload.as_slice(), &[0x03]);
    }

    #[test]
    fn empire_roundtrips_through_a_slice() {
        for v in [0_u8, 1, 2, 3, 4, 200, 255] {
            let b = CgEmpire::new(v).encode();
            assert_eq!(CgEmpire::decode(&b), Ok(CgEmpire::new(v)));
        }
    }

    #[test]
    fn empire_roundtrips_through_a_frame() {
        for v in [0_u8, 1, 2, 3, 4, 200, 255] {
            assert_eq!(
                CgEmpire::decode_frame(&CgEmpire::new(v).to_frame()),
                Ok(CgEmpire::new(v))
            );
        }
    }

    #[test]
    fn empire_keeps_every_byte_value_opaque() {
        // The legacy `EMPIRE_MAX_NUM` range check is session policy and must
        // not leak into the codec, so 0xFF has to round-trip unchanged.
        let b = CgEmpire::new(0xFF).encode();
        assert_eq!(CgEmpire::decode(&b), Ok(CgEmpire::new(0xFF)));
    }

    #[test]
    fn empire_encode_into_appends_both_bytes() {
        let mut out = vec![0xCC];
        CgEmpire::new(2).encode_into(&mut out);
        assert_eq!(out, vec![0xCC, EMPIRE, 0x02]);
    }

    #[test]
    fn empire_rejects_a_header_only_slice_as_truncated() {
        assert_eq!(
            CgEmpire::decode(&[EMPIRE]),
            Err(CgLoginError::Truncated {
                needed: 2,
                available: 1
            })
        );
    }

    #[test]
    fn empire_rejects_three_bytes_as_a_length_mismatch() {
        assert_eq!(
            CgEmpire::decode(&[EMPIRE, 0x01, 0x02]),
            Err(CgLoginError::LengthMismatch {
                expected: 2,
                actual: 3
            })
        );
    }

    #[test]
    fn empire_rejects_a_wrong_header_of_the_right_width() {
        assert_eq!(
            CgEmpire::decode(&[CHECKER, 0x01]),
            Err(CgLoginError::InvalidHeader {
                expected: EMPIRE,
                actual: CHECKER
            })
        );
    }

    #[test]
    fn empire_frame_rejects_an_empty_payload() {
        let f = ClientFrame::new(EMPIRE, [0_u8; 0]);
        assert_eq!(
            CgEmpire::decode_frame(&f),
            Err(CgLoginError::Truncated {
                needed: 1,
                available: 0
            })
        );
    }

    #[test]
    fn empire_frame_rejects_a_two_byte_payload() {
        let f = ClientFrame::new(EMPIRE, [0x01, 0x02]);
        assert_eq!(
            CgEmpire::decode_frame(&f),
            Err(CgLoginError::LengthMismatch {
                expected: 1,
                actual: 2
            })
        );
    }

    #[test]
    fn empire_reads_its_byte_at_offset_one() {
        // A self-consistent encoder/decoder pair would survive a reversed
        // layout, so the offset is pinned against hand-written bytes.
        assert_eq!(CgEmpire::decode(&[EMPIRE, 0xFF]), Ok(CgEmpire::new(0xFF)));
    }

    #[test]
    fn empire_rejects_a_record_whose_payload_is_the_header() {
        // `[0x5a, 0x5a]` is a plausible-looking "empire 90" framing that must
        // not be confused with a headerless payload.
        let f = ClientFrame::new(EMPIRE, [EMPIRE]);
        assert_eq!(CgEmpire::decode_frame(&f), Ok(CgEmpire::new(EMPIRE)));
    }

    // ---- cross-record and error surface --------------------------------

    #[test]
    fn the_two_header_only_records_reject_each_other() {
        assert!(CgPong::decode(&[CHECKER]).is_err());
        assert!(CgStateChecker::decode(&[PONG]).is_err());
    }

    #[test]
    fn a_two_byte_record_never_decodes_as_a_header_only_one() {
        // Same width as `CgPong`? No: this pins that the widths really differ,
        // so the 1-byte records report Truncated rather than InvalidHeader.
        assert_eq!(
            CgPong::decode(&[EMPIRE, 0x01]),
            Err(CgLoginError::LengthMismatch {
                expected: 1,
                actual: 2
            })
        );
    }

    #[test]
    fn every_record_rejects_the_login_handshake_header() {
        let h = HEADER_CG_HANDSHAKE.value();
        assert!(CgPong::decode(&[h]).is_err());
        assert!(CgStateChecker::decode(&[h]).is_err());
        assert!(CgEmpire::decode(&[h, 0x00]).is_err());
    }

    #[test]
    fn every_record_rejects_the_enter_game_header() {
        let h = HEADER_CG_ENTERGAME.value();
        assert!(CgPong::decode(&[h]).is_err());
        assert!(CgStateChecker::decode(&[h]).is_err());
        assert!(CgEmpire::decode(&[h, 0x00]).is_err());
    }

    #[test]
    fn the_three_headers_are_distinct() {
        let hs = [
            CgPong::header().value(),
            CgStateChecker::header().value(),
            CgEmpire::header().value(),
        ];
        let mut sorted = hs.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 3);
    }

    #[test]
    fn error_display_names_the_numbers() {
        assert_eq!(
            CgLoginError::Truncated {
                needed: 2,
                available: 1
            }
            .to_string(),
            "CG record needs 2 bytes, got 1"
        );
        assert_eq!(
            CgLoginError::LengthMismatch {
                expected: 1,
                actual: 3
            }
            .to_string(),
            "CG record must be exactly 1 bytes, got 3"
        );
        assert_eq!(
            CgLoginError::InvalidHeader {
                expected: 90,
                actual: 91
            }
            .to_string(),
            "CG header 91 is not 90"
        );
    }

    #[test]
    fn all_constructor_shapes_encode_to_the_declared_width() {
        assert_eq!(CgPong::new().encode().len(), CG_PONG_WIRE_SIZE);
        assert_eq!(
            CgStateChecker::new().encode().len(),
            CG_STATE_CHECKER_WIRE_SIZE
        );
        assert_eq!(CgEmpire::new(0).encode().len(), CG_EMPIRE_WIRE_SIZE);
    }

    #[test]
    fn all_frames_project_to_the_declared_payload_width() {
        assert_eq!(CgPong::new().to_frame().payload.len(), CG_PONG_PAYLOAD_SIZE);
        assert_eq!(
            CgStateChecker::new().to_frame().payload.len(),
            CG_STATE_CHECKER_PAYLOAD_SIZE
        );
        assert_eq!(
            CgEmpire::new(0).to_frame().payload.len(),
            CG_EMPIRE_PAYLOAD_SIZE
        );
    }
}
