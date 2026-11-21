//! Transport-free codecs for the three CG records that carry fixed `char[N+1]`
//! storage.
//!
//! | record | header | wire | payload | declaration | registration |
//! |---|---|---|---|---|---|
//! | `SPacketCGChangeName` | 106 `0x6a` | 27 | 26 | `packet.h:2454-2458` | `packet_info.cpp:165` |
//! | `SPacketCGInventoryProtected` | 144 `0x90` | 17 | 16 | `packet.h:3300-3309` | `packet_info.cpp:123` |
//! | `SPacketCGWhisperDetails` | 239 `0xef` | 26 | 25 | `packet.h:3259-3263` | `packet_info.cpp:176` |
//!
//! These are grouped by **wire shape**, not by header value and not by width, and
//! not by phase: `CgChangeName` is dispatched in the login phase while the other
//! two are dispatched in the game phase. The shape is the honest grouping, because
//! all three end in a fixed C-string buffer whose width comes from a shared
//! constant.
//!
//! # The name buffer is raw storage, and must stay that way
//!
//! Both name-carrying records use `char name[CHARACTER_NAME_MAX_LEN + 1]`, which
//! is `char[25]` because `CHARACTER_NAME_MAX_LEN = 24` at `common/length.h:15`.
//! The field is **not** a C string, and this module deliberately offers no way to
//! read it as one:
//!
//! - `CInputLogin::ChangeName` (`input_login.cpp:221`) calls `check_name(p->name)`.
//!   `check_name` is a function pointer, `extern int (*check_name)(const char *)`
//!   at `game/config.h:85`, assigned per locale in `locale_service.cpp`. The
//!   simplest implementation, `check_name_alphabet` at `locale_service.cpp:317`,
//!   opens with `if (strlen(str) < 2) return 0;` and then walks `for (tmp = str;
//!   *tmp; ++tmp)`.
//! - `CInputMain::WhisperDetails` (`input_main.cpp`) tests `if (!*name) return;`
//!   and then passes the same pointer to `CHARACTER_MANAGER::FindPC` and
//!   `P2P_MANAGER::Find`.
//!
//! All of those read until a NUL with no length bound of their own, so a client
//! that fills all 25 bytes with non-NUL data makes the legacy server read past the
//! end of the record. That is a **C++ robustness follow-up, not a codec rule**.
//! Adding a NUL requirement here would reject records the legacy server accepts,
//! which would make this codec *stricter* than the oracle it is derived from.
//!
//! So the name fields are exposed as `[u8; N]` and nothing more. Any NUL-aware
//! behaviour belongs in the handler layer above this module.
//!
//! Two related non-rules, both worth stating because they look like omissions:
//!
//! - `CInputLogin::ChangeName` range-checks `p->index` against
//!   `PLAYER_PER_ACCOUNT` and disconnects on overflow. That is session policy, so
//!   [`CgChangeName::index`] preserves **all 256** `u8` values.
//! - `CInputMain::WhisperDetails` does *not* call `check_name` at all. The rewrite
//!   must not add a `check_name` equivalent here.
//!
//! # A sub-header byte is not an opaque byte
//!
//! [`CgInventoryProtected::by_sub_header`] is a **named** enumeration in
//! `packet.h:3294-3295` — `SUBHEADER_CG_INVENTORY_PROTECTED_ACTIVATE = 0` and
//! `SUBHEADER_CG_INVENTORY_PROTECTED_PASSWORD_CHANGE = 1` — and
//! `CInputMain::RecvActivateProtectedSystem` switches on it immediately. The
//! value is therefore *sub-typed* rather than opaque.
//!
//! This is the opposite of `CgEmpire::empire` in [`crate::cg_login`], which is
//! also a `u8` at offset 1 and is *not* switched on: its handler compares
//! against `EMPIRE_MAX_NUM` later and can legitimately receive an out-of-range
//! value. **The struct cannot tell you which kind of byte you have; only the
//! handler can.** Neither kind is validated here, though, and all 256 values
//! round-trip: `RecvActivateProtectedSystem`'s `default: break;` discards every
//! other value silently, with no log and no error, and that legacy behaviour is
//! preserved rather than hardened.
//!
//! The two sub-arms also read different field sets, so a declared field is not
//! necessarily a meaningful one:
//!
//! | sub-header | reads | ignores |
//! |---|---|---|
//! | `ACTIVATE` (0) | `current_password`, `activated` | `replacement_password` |
//! | `PASSWORD_CHANGE` (1) | `current_password`, `replacement_password` | `activated` |

use crate::cg_inventory::{
    CgHeader, HEADER_CG_CHANGE_NAME, HEADER_CG_INVENTORY_PROTECTED, HEADER_CG_WHISPER_DETAILS,
};
use crate::cg_wire::ClientFrame;

/// The width of every `char name[CHARACTER_NAME_MAX_LEN + 1]` field in this
/// family: `CHARACTER_NAME_MAX_LEN` is 24, so the buffer is 25 bytes.
///
/// Shared by [`CgChangeName`] and [`CgWhisperDetails`] because both are declared
/// as `CHARACTER_NAME_MAX_LEN + 1` in both the server's `packet.h` and the
/// client's `Packet.h`, not as a literal 25.
pub const CG_CHARACTER_NAME_FIELD_SIZE: usize = 25;
/// The width of each password buffer in [`CgInventoryProtected`]:
/// `INVENTORY_PROTECTED_PASSWORD_MAX_LEN` is 6, so the buffer is 7 bytes.
pub const CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE: usize = 7;
/// The full legacy `SPacketCGChangeName` record, header byte included.
pub const CG_CHANGE_NAME_WIRE_SIZE: usize = 1 + 1 + CG_CHARACTER_NAME_FIELD_SIZE;
/// The framed payload of `SPacketCGChangeName`, everything after the header.
pub const CG_CHANGE_NAME_PAYLOAD_SIZE: usize = CG_CHANGE_NAME_WIRE_SIZE - 1;
/// The full legacy `SPacketCGWhisperDetails` record, header byte included.
pub const CG_WHISPER_DETAILS_WIRE_SIZE: usize = 1 + CG_CHARACTER_NAME_FIELD_SIZE;
/// The framed payload of `SPacketCGWhisperDetails`, everything after the header.
pub const CG_WHISPER_DETAILS_PAYLOAD_SIZE: usize = CG_WHISPER_DETAILS_WIRE_SIZE - 1;
/// The full legacy `SPacketCGInventoryProtected` record, header byte included.
pub const CG_INVENTORY_PROTECTED_WIRE_SIZE: usize =
    1 + 1 + 2 * CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE + 1;
/// The framed payload of `SPacketCGInventoryProtected`, after the header.
pub const CG_INVENTORY_PROTECTED_PAYLOAD_SIZE: usize = CG_INVENTORY_PROTECTED_WIRE_SIZE - 1;

/// Every way one of these three decoders can refuse a byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgNameError {
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

impl std::fmt::Display for CgNameError {
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

impl std::error::Error for CgNameError {}

/// Check a complete record width, then its header.
fn decode_parts(bytes: &[u8], wire_size: usize, expected: u8) -> Result<(), CgNameError> {
    check_exact(bytes.len(), wire_size)?;
    check_header(bytes[0], expected)
}

/// Check a header-less frame payload width, then its header.
fn decode_frame_parts(
    frame: &ClientFrame,
    payload_size: usize,
    expected: u8,
) -> Result<(), CgNameError> {
    check_exact(frame.payload.len(), payload_size)?;
    check_header(frame.header, expected)
}

fn check_exact(actual: usize, expected: usize) -> Result<(), CgNameError> {
    if actual < expected {
        return Err(CgNameError::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(CgNameError::LengthMismatch { expected, actual });
    }
    Ok(())
}

fn check_header(actual: u8, expected: u8) -> Result<(), CgNameError> {
    if actual == expected {
        return Ok(());
    }
    Err(CgNameError::InvalidHeader { expected, actual })
}

/// The login-phase request to rename a character slot.
///
/// Field order is `index: u8` at offset 1, then `name: [u8; 25]` at offset 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgChangeName {
    /// The account-character slot. `CInputLogin::ChangeName` rejects values at
    /// or above `PLAYER_PER_ACCOUNT`, but that check is session policy and every
    /// `u8` is preserved here.
    pub index: u8,
    /// The 25 raw name bytes. No NUL is required and none is implied.
    pub name: [u8; CG_CHARACTER_NAME_FIELD_SIZE],
}

impl CgChangeName {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_CHANGE_NAME
    }

    /// Build the record.
    pub const fn new(index: u8, name: [u8; CG_CHARACTER_NAME_FIELD_SIZE]) -> Self {
        Self { index, name }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.index);
        out.extend_from_slice(&self.name);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_CHANGE_NAME_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_CHANGE_NAME_PAYLOAD_SIZE);
        payload.push(self.index);
        payload.extend_from_slice(&self.name);
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgNameError::Truncated`] for a short input,
    /// [`CgNameError::LengthMismatch`] for a long one, and
    /// [`CgNameError::InvalidHeader`] when the header is not 106.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgNameError> {
        decode_parts(bytes, CG_CHANGE_NAME_WIRE_SIZE, Self::header().value())?;
        let mut name = [0_u8; CG_CHARACTER_NAME_FIELD_SIZE];
        name.copy_from_slice(&bytes[2..]);
        Ok(Self {
            index: bytes[1],
            name,
        })
    }

    /// # Errors
    ///
    /// As [`CgChangeName::decode`], except that the frame payload must be
    /// exactly 26 bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgNameError> {
        decode_frame_parts(frame, CG_CHANGE_NAME_PAYLOAD_SIZE, Self::header().value())?;
        let mut name = [0_u8; CG_CHARACTER_NAME_FIELD_SIZE];
        name.copy_from_slice(&frame.payload[1..]);
        Ok(Self {
            index: frame.payload[0],
            name,
        })
    }
}

/// The game-phase request for a character's whisper details.
///
/// Field order is `name: [u8; 25]` at offset 1. There is no second byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgWhisperDetails {
    /// The 25 raw name bytes. No NUL is required and none is implied.
    pub name: [u8; CG_CHARACTER_NAME_FIELD_SIZE],
}

impl CgWhisperDetails {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_WHISPER_DETAILS
    }

    /// Build the record.
    pub const fn new(name: [u8; CG_CHARACTER_NAME_FIELD_SIZE]) -> Self {
        Self { name }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.extend_from_slice(&self.name);
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_WHISPER_DETAILS_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame.
    pub fn to_frame(&self) -> ClientFrame {
        ClientFrame::new(Self::header().value(), self.name)
    }

    /// # Errors
    ///
    /// Returns [`CgNameError::Truncated`] for a short input,
    /// [`CgNameError::LengthMismatch`] for a long one, and
    /// [`CgNameError::InvalidHeader`] when the header is not 239.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgNameError> {
        decode_parts(bytes, CG_WHISPER_DETAILS_WIRE_SIZE, Self::header().value())?;
        let mut name = [0_u8; CG_CHARACTER_NAME_FIELD_SIZE];
        name.copy_from_slice(&bytes[1..]);
        Ok(Self { name })
    }

    /// # Errors
    ///
    /// As [`CgWhisperDetails::decode`], except that the frame payload must be
    /// exactly 25 bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgNameError> {
        decode_frame_parts(
            frame,
            CG_WHISPER_DETAILS_PAYLOAD_SIZE,
            Self::header().value(),
        )?;
        let mut name = [0_u8; CG_CHARACTER_NAME_FIELD_SIZE];
        name.copy_from_slice(&frame.payload[..]);
        Ok(Self { name })
    }
}

/// The game-phase protected-inventory activation or password-change request.
///
/// Field order is `by_sub_header: u8` at offset 1, `current_password: [u8; 7]` at
/// offset 2, `replacement_password: [u8; 7]` at offset 9, `activated: bool` at offset 16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgInventoryProtected {
    /// Selects between the two named sub-commands. All 256 values are preserved;
    /// the legacy `default:` arm discards the rest silently.
    pub by_sub_header: u8,
    /// The 7 raw bytes of the legacy `szPasswordNow` field.
    pub current_password: [u8; CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE],
    /// The 7 raw bytes of the legacy `szPasswordNew` field. Only the
    /// `PASSWORD_CHANGE` arm reads this.
    pub replacement_password: [u8; CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE],
    /// The activation flag. Only the `ACTIVATE` arm reads this.
    pub activated: bool,
}

impl CgInventoryProtected {
    /// The fixed header byte. It is never stored on the struct.
    pub const fn header() -> CgHeader {
        HEADER_CG_INVENTORY_PROTECTED
    }

    /// Build the record.
    pub const fn new(
        by_sub_header: u8,
        current_password: [u8; CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE],
        replacement_password: [u8; CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE],
        activated: bool,
    ) -> Self {
        Self {
            by_sub_header,
            current_password,
            replacement_password,
            activated,
        }
    }

    /// Append the record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        out.push(self.by_sub_header);
        out.extend_from_slice(&self.current_password);
        out.extend_from_slice(&self.replacement_password);
        out.push(u8::from(self.activated));
    }

    /// Encode to a fresh buffer.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_INVENTORY_PROTECTED_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Project to the fixed client frame.
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_INVENTORY_PROTECTED_PAYLOAD_SIZE);
        payload.push(self.by_sub_header);
        payload.extend_from_slice(&self.current_password);
        payload.extend_from_slice(&self.replacement_password);
        payload.push(u8::from(self.activated));
        ClientFrame::new(Self::header().value(), payload)
    }

    /// # Errors
    ///
    /// Returns [`CgNameError::Truncated`] for a short input,
    /// [`CgNameError::LengthMismatch`] for a long one, and
    /// [`CgNameError::InvalidHeader`] when the header is not 144.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgNameError> {
        decode_parts(
            bytes,
            CG_INVENTORY_PROTECTED_WIRE_SIZE,
            Self::header().value(),
        )?;
        let mut current_password = [0_u8; CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE];
        current_password.copy_from_slice(&bytes[2..9]);
        let mut replacement_password = [0_u8; CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE];
        replacement_password.copy_from_slice(&bytes[9..16]);
        Ok(Self {
            by_sub_header: bytes[1],
            current_password,
            replacement_password,
            activated: bytes[16] != 0,
        })
    }

    /// # Errors
    ///
    /// As [`CgInventoryProtected::decode`], except that the frame payload must
    /// be exactly 16 bytes.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgNameError> {
        decode_frame_parts(
            frame,
            CG_INVENTORY_PROTECTED_PAYLOAD_SIZE,
            Self::header().value(),
        )?;
        let mut current_password = [0_u8; CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE];
        current_password.copy_from_slice(&frame.payload[1..8]);
        let mut replacement_password = [0_u8; CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE];
        replacement_password.copy_from_slice(&frame.payload[8..15]);
        Ok(Self {
            by_sub_header: frame.payload[0],
            current_password,
            replacement_password,
            activated: frame.payload[15] != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// A 25-byte buffer with distinct values in every position, so a field
    /// reorder or a one-byte shift cannot pass by accident.
    fn sample_name() -> [u8; CG_CHARACTER_NAME_FIELD_SIZE] {
        let mut n = [0_u8; CG_CHARACTER_NAME_FIELD_SIZE];
        for (i, b) in n.iter_mut().enumerate() {
            *b = u8::try_from(i + 1).expect("i + 1 fits in u8");
        }
        n
    }

    /// A 7-byte password buffer with distinct values in every position.
    fn sample_password() -> [u8; CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE] {
        let mut p = [0_u8; CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE];
        for (i, b) in p.iter_mut().enumerate() {
            *b = u8::try_from(0xA0 + i).expect("0xA0 + i fits in u8");
        }
        p
    }

    fn sample_change_name() -> CgChangeName {
        CgChangeName::new(3, sample_name())
    }

    fn sample_whisper() -> CgWhisperDetails {
        CgWhisperDetails::new(sample_name())
    }

    fn sample_inventory() -> CgInventoryProtected {
        CgInventoryProtected::new(1, sample_password(), [0x5A; 7], true)
    }

    // ---- shared invariants -------------------------------------------------

    #[test]
    fn headers_are_the_legacy_values() {
        assert_eq!(CgChangeName::header().value(), 106);
        assert_eq!(CgInventoryProtected::header().value(), 144);
        assert_eq!(CgWhisperDetails::header().value(), 239);
    }

    #[test]
    fn wire_sizes_match_the_registrations() {
        assert_eq!(CG_CHANGE_NAME_WIRE_SIZE, 27);
        assert_eq!(CG_WHISPER_DETAILS_WIRE_SIZE, 26);
        assert_eq!(CG_INVENTORY_PROTECTED_WIRE_SIZE, 17);
    }

    #[test]
    fn payload_sizes_are_the_wire_size_minus_the_header() {
        assert_eq!(CG_CHANGE_NAME_PAYLOAD_SIZE, CG_CHANGE_NAME_WIRE_SIZE - 1);
        assert_eq!(
            CG_WHISPER_DETAILS_PAYLOAD_SIZE,
            CG_WHISPER_DETAILS_WIRE_SIZE - 1
        );
        assert_eq!(
            CG_INVENTORY_PROTECTED_PAYLOAD_SIZE,
            CG_INVENTORY_PROTECTED_WIRE_SIZE - 1
        );
    }

    #[test]
    fn field_widths_come_from_the_legacy_length_constants() {
        // CHARACTER_NAME_MAX_LEN = 24 and INVENTORY_PROTECTED_PASSWORD_MAX_LEN = 6,
        // each stored as a +1 buffer.
        assert_eq!(CG_CHARACTER_NAME_FIELD_SIZE, 25);
        assert_eq!(CG_INVENTORY_PROTECTED_PASSWORD_FIELD_SIZE, 7);
    }

    // ---- CgChangeName -------------------------------------------------------

    #[test]
    fn change_name_encodes_to_the_exact_legacy_bytes() {
        let bytes = sample_change_name().encode();
        assert_eq!(bytes.len(), 27);
        assert_eq!(bytes[0], 106);
        assert_eq!(bytes[1], 3);
        for (i, b) in bytes[2..].iter().enumerate() {
            assert_eq!(*b, u8::try_from(i + 1).expect("fits"));
        }
    }

    #[test]
    fn change_name_decodes_the_exact_legacy_bytes() {
        let rec = sample_change_name();
        assert_eq!(CgChangeName::decode(&rec.encode()).unwrap(), rec);
    }

    #[test]
    fn change_name_round_trips_through_a_frame() {
        let rec = sample_change_name();
        let frame = rec.to_frame();
        assert_eq!(frame.header, 106);
        assert_eq!(frame.payload.len(), 26);
        assert_eq!(frame.payload[0], rec.index);
        assert_eq!(&frame.payload[1..], &rec.name[..]);
        assert_eq!(CgChangeName::decode_frame(&frame).unwrap(), rec);
    }

    #[test]
    fn change_name_frame_and_slice_agree() {
        let rec = sample_change_name();
        assert_eq!(
            CgChangeName::decode(&rec.encode()).unwrap(),
            CgChangeName::decode_frame(&rec.to_frame()).unwrap()
        );
    }

    #[test]
    fn change_name_rejects_every_short_length() {
        for len in 0..CG_CHANGE_NAME_WIRE_SIZE {
            let err = CgChangeName::decode(&vec![106_u8; len]).unwrap_err();
            assert_eq!(
                err,
                CgNameError::Truncated {
                    needed: 27,
                    available: len
                }
            );
        }
    }

    #[test]
    fn change_name_rejects_every_long_length() {
        for len in 28..40 {
            let err = CgChangeName::decode(&vec![106_u8; len]).unwrap_err();
            assert_eq!(
                err,
                CgNameError::LengthMismatch {
                    expected: 27,
                    actual: len
                }
            );
        }
    }

    #[test]
    fn change_name_rejects_a_foreign_header() {
        let mut bytes = sample_change_name().encode();
        bytes[0] = 107;
        assert_eq!(
            CgChangeName::decode(&bytes).unwrap_err(),
            CgNameError::InvalidHeader {
                expected: 106,
                actual: 107
            }
        );
    }

    #[test]
    fn change_name_frame_rejects_a_wrong_payload_width() {
        let rec = sample_change_name();
        let mut frame = rec.to_frame();
        frame.payload.push(0);
        assert_eq!(
            CgChangeName::decode_frame(&frame).unwrap_err(),
            CgNameError::LengthMismatch {
                expected: 26,
                actual: 27
            }
        );
    }

    #[test]
    fn change_name_index_is_opaque_across_all_256_values() {
        // CInputLogin::ChangeName range-checks against PLAYER_PER_ACCOUNT, but
        // that is session policy, so framing preserves every byte.
        for index in 0..=u8::MAX {
            let rec = CgChangeName::new(index, sample_name());
            assert_eq!(rec.encode()[1], index);
            assert_eq!(CgChangeName::decode(&rec.encode()).unwrap().index, index);
        }
    }

    #[test]
    fn change_name_preserves_a_name_with_no_nul() {
        let rec = CgChangeName::new(0, [0x41; CG_CHARACTER_NAME_FIELD_SIZE]);
        let back = CgChangeName::decode(&rec.encode()).unwrap();
        assert_eq!(back.name, [0x41; CG_CHARACTER_NAME_FIELD_SIZE]);
    }

    #[test]
    fn change_name_reads_the_index_at_offset_one() {
        let mut bytes = sample_change_name().encode();
        bytes[1] = 0xEE;
        assert_eq!(CgChangeName::decode(&bytes).unwrap().index, 0xEE);
    }

    // ---- CgWhisperDetails ---------------------------------------------------

    #[test]
    fn whisper_encodes_to_the_exact_legacy_bytes() {
        let bytes = sample_whisper().encode();
        assert_eq!(bytes.len(), 26);
        assert_eq!(bytes[0], 239);
        for (i, b) in bytes[1..].iter().enumerate() {
            assert_eq!(*b, u8::try_from(i + 1).expect("fits"));
        }
    }

    #[test]
    fn whisper_decodes_the_exact_legacy_bytes() {
        let rec = sample_whisper();
        assert_eq!(CgWhisperDetails::decode(&rec.encode()).unwrap(), rec);
    }

    #[test]
    fn whisper_round_trips_through_a_frame() {
        let rec = sample_whisper();
        let frame = rec.to_frame();
        assert_eq!(frame.header, 239);
        assert_eq!(frame.payload.len(), 25);
        assert_eq!(&frame.payload[..], &rec.name[..]);
        assert_eq!(CgWhisperDetails::decode_frame(&frame).unwrap(), rec);
    }

    #[test]
    fn whisper_frame_and_slice_agree() {
        let rec = sample_whisper();
        assert_eq!(
            CgWhisperDetails::decode(&rec.encode()).unwrap(),
            CgWhisperDetails::decode_frame(&rec.to_frame()).unwrap()
        );
    }

    #[test]
    fn whisper_rejects_every_short_length() {
        for len in 0..CG_WHISPER_DETAILS_WIRE_SIZE {
            let err = CgWhisperDetails::decode(&vec![239_u8; len]).unwrap_err();
            assert_eq!(
                err,
                CgNameError::Truncated {
                    needed: 26,
                    available: len
                }
            );
        }
    }

    #[test]
    fn whisper_rejects_every_long_length() {
        for len in 27..40 {
            let err = CgWhisperDetails::decode(&vec![239_u8; len]).unwrap_err();
            assert_eq!(
                err,
                CgNameError::LengthMismatch {
                    expected: 26,
                    actual: len
                }
            );
        }
    }

    #[test]
    fn whisper_rejects_a_foreign_header() {
        let mut bytes = sample_whisper().encode();
        bytes[0] = 238;
        assert_eq!(
            CgWhisperDetails::decode(&bytes).unwrap_err(),
            CgNameError::InvalidHeader {
                expected: 239,
                actual: 238
            }
        );
    }

    #[test]
    fn whisper_frame_rejects_a_wrong_payload_width() {
        let mut frame = sample_whisper().to_frame();
        frame.payload.pop();
        assert_eq!(
            CgWhisperDetails::decode_frame(&frame).unwrap_err(),
            CgNameError::Truncated {
                needed: 25,
                available: 24
            }
        );
        // Two pushes, because one would just undo the pop above.
        frame.payload.push(0);
        frame.payload.push(0);
        assert_eq!(
            CgWhisperDetails::decode_frame(&frame).unwrap_err(),
            CgNameError::LengthMismatch {
                expected: 25,
                actual: 26
            }
        );
    }

    #[test]
    fn whisper_preserves_an_all_nonzero_name() {
        // Every legacy consumer reads this field as a C string, so this record
        // proves the codec imposes no NUL requirement of its own.
        let rec = CgWhisperDetails::new([0x7A; CG_CHARACTER_NAME_FIELD_SIZE]);
        let back = CgWhisperDetails::decode(&rec.encode()).unwrap();
        assert_eq!(back.name, [0x7A; CG_CHARACTER_NAME_FIELD_SIZE]);
    }

    #[test]
    fn whisper_reads_the_name_from_offset_one() {
        let mut bytes = sample_whisper().encode();
        bytes[1] = 0xEE;
        assert_eq!(CgWhisperDetails::decode(&bytes).unwrap().name[0], 0xEE);
    }

    // ---- CgInventoryProtected -----------------------------------------------

    #[test]
    fn inventory_protected_encodes_to_the_exact_legacy_bytes() {
        let rec = sample_inventory();
        let bytes = rec.encode();
        assert_eq!(bytes.len(), 17);
        assert_eq!(bytes[0], 144);
        assert_eq!(bytes[1], 1);
        for (i, b) in bytes[2..9].iter().enumerate() {
            assert_eq!(*b, u8::try_from(0xA0 + i).expect("fits"));
        }
        assert_eq!(&bytes[9..16], &[0x5A; 7]);
        assert_eq!(bytes[16], 1);
    }

    #[test]
    fn inventory_protected_decodes_the_exact_legacy_bytes() {
        let rec = sample_inventory();
        assert_eq!(CgInventoryProtected::decode(&rec.encode()).unwrap(), rec);
    }

    #[test]
    fn inventory_protected_round_trips_through_a_frame() {
        let rec = sample_inventory();
        let frame = rec.to_frame();
        assert_eq!(frame.header, 144);
        assert_eq!(frame.payload.len(), 16);
        assert_eq!(frame.payload[0], rec.by_sub_header);
        assert_eq!(&frame.payload[1..8], &rec.current_password[..]);
        assert_eq!(&frame.payload[8..15], &rec.replacement_password[..]);
        assert_eq!(frame.payload[15], 1);
        assert_eq!(CgInventoryProtected::decode_frame(&frame).unwrap(), rec);
    }

    #[test]
    fn inventory_protected_frame_and_slice_agree() {
        let rec = sample_inventory();
        assert_eq!(
            CgInventoryProtected::decode(&rec.encode()).unwrap(),
            CgInventoryProtected::decode_frame(&rec.to_frame()).unwrap()
        );
    }

    #[test]
    fn inventory_protected_rejects_every_short_length() {
        for len in 0..CG_INVENTORY_PROTECTED_WIRE_SIZE {
            let err = CgInventoryProtected::decode(&vec![144_u8; len]).unwrap_err();
            assert_eq!(
                err,
                CgNameError::Truncated {
                    needed: 17,
                    available: len
                }
            );
        }
    }

    #[test]
    fn inventory_protected_rejects_every_long_length() {
        for len in 18..30 {
            let err = CgInventoryProtected::decode(&vec![144_u8; len]).unwrap_err();
            assert_eq!(
                err,
                CgNameError::LengthMismatch {
                    expected: 17,
                    actual: len
                }
            );
        }
    }

    #[test]
    fn inventory_protected_rejects_a_foreign_header() {
        let mut bytes = sample_inventory().encode();
        bytes[0] = 143;
        assert_eq!(
            CgInventoryProtected::decode(&bytes).unwrap_err(),
            CgNameError::InvalidHeader {
                expected: 144,
                actual: 143
            }
        );
    }

    #[test]
    fn inventory_protected_sub_header_is_preserved_for_all_256_values() {
        // The legacy default arm discards unknown sub-headers silently, so no
        // value is rejected here even though only 0 and 1 are named.
        for sub in 0..=u8::MAX {
            let rec = CgInventoryProtected::new(sub, sample_password(), [0x5A; 7], true);
            assert_eq!(rec.encode()[1], sub);
            assert_eq!(
                CgInventoryProtected::decode(&rec.encode())
                    .unwrap()
                    .by_sub_header,
                sub
            );
        }
    }

    #[test]
    fn inventory_protected_activated_is_a_nonzero_test_not_a_narrowing() {
        // The legacy field is a C++ bool, so any nonzero byte means true.
        for raw in [1_u8, 2, 0x80, 0xFF] {
            let mut bytes = sample_inventory().encode();
            bytes[16] = raw;
            assert!(CgInventoryProtected::decode(&bytes).unwrap().activated);
        }
        let mut bytes = sample_inventory().encode();
        bytes[16] = 0;
        assert!(!CgInventoryProtected::decode(&bytes).unwrap().activated);
    }

    #[test]
    fn inventory_protected_passwords_are_independent_fields() {
        // The PASSWORD_CHANGE arm reads both buffers, so a swap must be visible.
        let rec = CgInventoryProtected::new(1, [0x11; 7], [0x22; 7], false);
        let bytes = rec.encode();
        assert_eq!(&bytes[2..9], &[0x11; 7]);
        assert_eq!(&bytes[9..16], &[0x22; 7]);
        assert_eq!(bytes[16], 0);
    }

    #[test]
    fn inventory_protected_reads_the_sub_header_at_offset_one() {
        let mut bytes = sample_inventory().encode();
        bytes[1] = 0xEE;
        assert_eq!(
            CgInventoryProtected::decode(&bytes).unwrap().by_sub_header,
            0xEE
        );
    }

    #[test]
    fn inventory_protected_frame_reads_the_flag_at_the_last_payload_byte() {
        // The flag is the FINAL payload byte. Pin it there by zeroing every
        // earlier byte, including the tail of the replacement password: a
        // decoder that looked one byte earlier would read 0 and report false.
        //
        // This precondition is the whole point. The other frame tests use
        // `sample_inventory()`, whose replacement password ends in 0x5A, so a
        // decoder reading index 14 instead of 15 still sees a nonzero byte and
        // still returns true. Those tests cannot tell the two offsets apart.
        let mut frame = CgInventoryProtected::new(0, [0; 7], [0; 7], true).to_frame();
        assert_eq!(frame.payload.len(), 16);
        assert_eq!(frame.payload[15], 1, "the flag is encoded last");
        assert_eq!(
            frame.payload[14], 0,
            "precondition: the byte before the flag must be zero"
        );
        assert!(
            CgInventoryProtected::decode_frame(&frame)
                .unwrap()
                .activated
        );

        frame.payload[15] = 0;
        assert!(
            !CgInventoryProtected::decode_frame(&frame)
                .unwrap()
                .activated
        );

        // The C++ bool rule holds through the frame path too: any nonzero byte
        // is true, not just 1.
        for raw in [1_u8, 2, 0x80, 0xFF] {
            let mut probe = CgInventoryProtected::new(0, [0; 7], [0; 7], false).to_frame();
            probe.payload[15] = raw;
            assert_eq!(probe.payload[14], 0, "precondition: index 14 stays zero");
            assert!(
                CgInventoryProtected::decode_frame(&probe)
                    .unwrap()
                    .activated
            );
        }
    }

    // ---- shared error behaviour --------------------------------------------

    #[test]
    fn an_empty_input_reports_each_records_own_width() {
        // The three records are different widths, so a shared empty input must
        // produce three different `needed` values -- one per record.
        let empty: [u8; 0] = [];
        for (err, own) in [
            (
                CgChangeName::decode(&empty).unwrap_err(),
                CG_CHANGE_NAME_WIRE_SIZE,
            ),
            (
                CgInventoryProtected::decode(&empty).unwrap_err(),
                CG_INVENTORY_PROTECTED_WIRE_SIZE,
            ),
            (
                CgWhisperDetails::decode(&empty).unwrap_err(),
                CG_WHISPER_DETAILS_WIRE_SIZE,
            ),
        ] {
            assert_eq!(
                err,
                CgNameError::Truncated {
                    needed: own,
                    available: 0
                }
            );
        }
    }

    #[test]
    fn error_messages_name_the_record_width() {
        let rec = sample_change_name().encode();
        let msg = CgChangeName::decode(&rec[..10]).unwrap_err().to_string();
        assert!(msg.contains("27"), "message was {msg}");
        let msg = CgChangeName::decode(&[0_u8; 30]).unwrap_err().to_string();
        assert!(msg.contains("27"), "message was {msg}");
    }

    #[test]
    fn three_distinct_wire_sizes_are_recorded() {
        let sizes = [
            CG_CHANGE_NAME_WIRE_SIZE,
            CG_WHISPER_DETAILS_WIRE_SIZE,
            CG_INVENTORY_PROTECTED_WIRE_SIZE,
        ];
        let unique: BTreeSet<usize> = sizes.iter().copied().collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn every_record_rejects_the_other_records_header() {
        // 106, 144 and 239 are all distinct, so no record can decode another's
        // bytes even when the length happens to line up.
        let mut headers = BTreeSet::new();
        headers.insert(CgChangeName::header().value());
        headers.insert(CgInventoryProtected::header().value());
        headers.insert(CgWhisperDetails::header().value());
        assert_eq!(headers.len(), 3);
    }

    #[test]
    fn encoding_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE, 0xAD];
        sample_change_name().encode_into(&mut out);
        assert_eq!(out.len(), 2 + 27);
        assert_eq!(&out[2..], &sample_change_name().encode()[..]);
    }
}
