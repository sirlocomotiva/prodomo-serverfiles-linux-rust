//! Explicit codecs for the fixed client-to-game account-phase records.
//!
//! `server/server/game/packet.h:465-521,594-597` declares
//! `TPacketCGLogin`, `TPacketCGLogin2`, `TPacketCGPlayerSelect`,
//! `TPacketCGPlayerDelete`, `TPacketCGPlayerCreate`, and `TPacketCGEnterGame`
//! under the active packed x86 profile. The `packet_info.cpp:107-116`
//! registrations give the exact 49-, 52-, 2-, 10-, 34-, and 1-byte complete
//! records, respectively. Login, character names, private-code, and stat
//! fields are retained as opaque bytes: the legacy handlers perform
//! normalization, narrowing, and bounded copies later, and this module does
//! not choose an encoding or locale.
//!
//! These types only validate the one-byte-header wire shape and preserve field
//! order and bytes. They do not perform admission checks, install keys, talk to
//! the DB peer, update lifecycle state, or invoke gameplay handlers.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::cg_inventory::{
    HEADER_CG_CHARACTER_CREATE, HEADER_CG_CHARACTER_DELETE, HEADER_CG_CHARACTER_SELECT,
    HEADER_CG_ENTERGAME, HEADER_CG_LOGIN, HEADER_CG_LOGIN2,
};
use crate::cg_wire::ClientFrame;

/// Number of bytes in the source `login[LOGIN_MAX_LEN + 1]` field.
pub const CG_LOGIN_FIELD_BYTES: usize = 31;

/// Number of bytes in the source `passwd[PASSWD_MAX_LEN + 1]` field.
pub const CG_PASSWORD_FIELD_BYTES: usize = 17;

/// Number of 32-bit words in the source `adwClientKey` field.
pub const CG_CLIENT_KEY_WORDS: usize = 4;

/// Complete packed size of `TPacketCGLogin`, including its header.
pub const CG_LOGIN_WIRE_SIZE: usize = 1 + CG_LOGIN_FIELD_BYTES + CG_PASSWORD_FIELD_BYTES;

/// Complete packed size of `TPacketCGLogin2`, including its header.
pub const CG_LOGIN_BY_KEY_WIRE_SIZE: usize =
    1 + CG_LOGIN_FIELD_BYTES + 4 + (CG_CLIENT_KEY_WORDS * 4);

/// Complete packed size of `TPacketCGPlayerSelect`, including its header.
pub const CG_PLAYER_SELECT_WIRE_SIZE: usize = 2;

/// Number of raw bytes in the source character-deletion private-code field.
pub const CG_PRIVATE_CODE_FIELD_BYTES: usize = 8;

/// Complete packed size of `TPacketCGPlayerDelete`, including its header.
pub const CG_PLAYER_DELETE_WIRE_SIZE: usize = 1 + 1 + CG_PRIVATE_CODE_FIELD_BYTES;

/// Number of raw bytes in the source character-name field.
pub const CG_PLAYER_CREATE_NAME_FIELD_BYTES: usize = 25;

/// Payload size of `TPacketCGPlayerCreate`, excluding its one-byte header.
pub const CG_PLAYER_CREATE_PAYLOAD_SIZE: usize = 1 + CG_PLAYER_CREATE_NAME_FIELD_BYTES + 2 + 5;

/// Complete packed size of `TPacketCGPlayerCreate`, including its header.
pub const CG_PLAYER_CREATE_WIRE_SIZE: usize = 1 + CG_PLAYER_CREATE_PAYLOAD_SIZE;

/// Complete packed size of `TPacketCGEnterGame`, including its header.
pub const CG_ENTER_GAME_WIRE_SIZE: usize = 1;

/// A password login record (`TPacketCGLogin`).
///
/// The `login` and `password` arrays are opaque source buffers. A NUL byte is
/// not required for wire decoding; the later legacy normalization and
/// `strlcpy`-style projection remain caller responsibilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgLogin {
    /// Raw `login[LOGIN_MAX_LEN + 1]` bytes.
    pub login: [u8; CG_LOGIN_FIELD_BYTES],
    /// Raw `passwd[PASSWD_MAX_LEN + 1]` bytes.
    pub password: [u8; CG_PASSWORD_FIELD_BYTES],
}

impl CgLogin {
    /// Construct a password-login record without interpreting either buffer.
    #[must_use]
    pub const fn new(
        login: [u8; CG_LOGIN_FIELD_BYTES],
        password: [u8; CG_PASSWORD_FIELD_BYTES],
    ) -> Self {
        Self { login, password }
    }

    /// Encode the exact 49-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CG_LOGIN_WIRE_SIZE);
        bytes.push(HEADER_CG_LOGIN.value());
        bytes.extend_from_slice(&self.login);
        bytes.extend_from_slice(&self.password);
        bytes
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_LOGIN_WIRE_SIZE - 1);
        payload.extend_from_slice(&self.login);
        payload.extend_from_slice(&self.password);
        ClientFrame::new(HEADER_CG_LOGIN.value(), payload)
    }

    /// Decode one exact complete password-login record.
    ///
    /// # Errors
    ///
    /// Returns [`CgAccountError::Truncated`] for fewer than 49 bytes,
    /// [`CgAccountError::LengthMismatch`] for more than 49 bytes, and
    /// [`CgAccountError::InvalidHeader`] for a complete record with another
    /// header.
    pub fn decode(data: &[u8]) -> Result<Self, CgAccountError> {
        check_exact(data, CG_LOGIN_WIRE_SIZE)?;
        Self::decode_parts(data[0], &data[1..])
    }

    /// Decode a [`ClientFrame`] whose payload excludes the one-byte header.
    ///
    /// # Errors
    ///
    /// Returns a length error when the frame payload is not exactly 48 bytes,
    /// or [`CgAccountError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgAccountError> {
        check_frame_payload(frame, CG_LOGIN_WIRE_SIZE - 1, CG_LOGIN_WIRE_SIZE)?;
        Self::decode_parts(frame.header, &frame.payload)
    }

    fn decode_parts(header: u8, payload: &[u8]) -> Result<Self, CgAccountError> {
        if header != HEADER_CG_LOGIN.value() {
            return Err(CgAccountError::InvalidHeader { actual: header });
        }
        debug_assert_eq!(payload.len(), CG_LOGIN_WIRE_SIZE - 1);
        let login = payload[..CG_LOGIN_FIELD_BYTES]
            .try_into()
            .expect("length checked before account-login field copy");
        let password = payload[CG_LOGIN_FIELD_BYTES..]
            .try_into()
            .expect("length checked before password-field copy");
        Ok(Self::new(login, password))
    }
}

/// A login-by-key record (`TPacketCGLogin2`).
///
/// The login buffer is opaque. `login_key` and each `client_key` word are
/// encoded explicitly little-endian, matching the active x86 wire profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgLoginByKey {
    /// Raw `login[LOGIN_MAX_LEN + 1]` bytes.
    pub login: [u8; CG_LOGIN_FIELD_BYTES],
    /// Source `dwLoginKey` value.
    pub login_key: u32,
    /// Four source `adwClientKey` values in declaration order.
    pub client_key: [u32; CG_CLIENT_KEY_WORDS],
}

impl CgLoginByKey {
    /// Construct a login-by-key record without interpreting the login buffer.
    #[must_use]
    pub const fn new(
        login: [u8; CG_LOGIN_FIELD_BYTES],
        login_key: u32,
        client_key: [u32; CG_CLIENT_KEY_WORDS],
    ) -> Self {
        Self {
            login,
            login_key,
            client_key,
        }
    }

    /// Encode the exact 52-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CG_LOGIN_BY_KEY_WIRE_SIZE);
        bytes.push(HEADER_CG_LOGIN2.value());
        bytes.extend_from_slice(&self.login);
        bytes.extend_from_slice(&self.login_key.to_le_bytes());
        for word in self.client_key {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_LOGIN_BY_KEY_WIRE_SIZE - 1);
        payload.extend_from_slice(&self.login);
        payload.extend_from_slice(&self.login_key.to_le_bytes());
        for word in self.client_key {
            payload.extend_from_slice(&word.to_le_bytes());
        }
        ClientFrame::new(HEADER_CG_LOGIN2.value(), payload)
    }

    /// Decode one exact complete login-by-key record.
    ///
    /// # Errors
    ///
    /// Returns [`CgAccountError::Truncated`] for fewer than 52 bytes,
    /// [`CgAccountError::LengthMismatch`] for more than 52 bytes, and
    /// [`CgAccountError::InvalidHeader`] for a complete record with another
    /// header.
    pub fn decode(data: &[u8]) -> Result<Self, CgAccountError> {
        check_exact(data, CG_LOGIN_BY_KEY_WIRE_SIZE)?;
        Self::decode_parts(data[0], &data[1..])
    }

    /// Decode a [`ClientFrame`] whose payload excludes the one-byte header.
    ///
    /// # Errors
    ///
    /// Returns a length error when the frame payload is not exactly 51 bytes,
    /// or [`CgAccountError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgAccountError> {
        check_frame_payload(
            frame,
            CG_LOGIN_BY_KEY_WIRE_SIZE - 1,
            CG_LOGIN_BY_KEY_WIRE_SIZE,
        )?;
        Self::decode_parts(frame.header, &frame.payload)
    }

    fn decode_parts(header: u8, payload: &[u8]) -> Result<Self, CgAccountError> {
        if header != HEADER_CG_LOGIN2.value() {
            return Err(CgAccountError::InvalidHeader { actual: header });
        }
        debug_assert_eq!(payload.len(), CG_LOGIN_BY_KEY_WIRE_SIZE - 1);
        let login = payload[..CG_LOGIN_FIELD_BYTES]
            .try_into()
            .expect("length checked before login-by-key field copy");
        let key_start = CG_LOGIN_FIELD_BYTES;
        let login_key = u32::from_le_bytes(
            payload[key_start..key_start + 4]
                .try_into()
                .expect("length checked before login-key copy"),
        );
        let mut client_key = [0_u32; CG_CLIENT_KEY_WORDS];
        for (index, word) in client_key.iter_mut().enumerate() {
            let start = key_start + 4 + (index * 4);
            *word = u32::from_le_bytes(
                payload[start..start + 4]
                    .try_into()
                    .expect("length checked before client-key copy"),
            );
        }
        Ok(Self::new(login, login_key, client_key))
    }
}

/// A character-selection record (`TPacketCGPlayerSelect`).
///
/// The codec preserves the one-byte index. It does not apply the four-slot
/// bound; the account/player reducer performs that check when it handles the
/// decoded value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgPlayerSelect {
    /// Source `index` byte.
    pub index: u8,
}

impl CgPlayerSelect {
    /// Construct a character-selection record without validating its index.
    #[must_use]
    pub const fn new(index: u8) -> Self {
        Self { index }
    }

    /// Encode the exact two-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        vec![HEADER_CG_CHARACTER_SELECT.value(), self.index]
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        ClientFrame::new(HEADER_CG_CHARACTER_SELECT.value(), [self.index])
    }

    /// Decode one exact complete character-selection record.
    ///
    /// # Errors
    ///
    /// Returns [`CgAccountError::Truncated`] for fewer than two bytes,
    /// [`CgAccountError::LengthMismatch`] for more than two bytes, and
    /// [`CgAccountError::InvalidHeader`] for a complete record with another
    /// header.
    pub fn decode(data: &[u8]) -> Result<Self, CgAccountError> {
        check_exact(data, CG_PLAYER_SELECT_WIRE_SIZE)?;
        Self::decode_parts(data[0], data[1])
    }

    /// Decode a [`ClientFrame`] whose payload excludes the one-byte header.
    ///
    /// # Errors
    ///
    /// Returns a length error when the frame payload is not exactly one byte,
    /// or [`CgAccountError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgAccountError> {
        check_frame_payload(frame, 1, CG_PLAYER_SELECT_WIRE_SIZE)?;
        Self::decode_parts(frame.header, frame.payload[0])
    }

    fn decode_parts(header: u8, index: u8) -> Result<Self, CgAccountError> {
        if header != HEADER_CG_CHARACTER_SELECT.value() {
            return Err(CgAccountError::InvalidHeader { actual: header });
        }
        Ok(Self::new(index))
    }
}

/// A character-deletion request (`TPacketCGPlayerDelete`).
///
/// The selected-slot index and all eight private-code bytes remain opaque at
/// this wire boundary. In particular, the codec does not require a NUL or
/// reproduce the legacy sender's possible uninitialized final byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgPlayerDelete {
    /// Source account-character `index` byte.
    pub index: u8,
    /// Raw source `private_code[8]` bytes.
    pub private_code: [u8; CG_PRIVATE_CODE_FIELD_BYTES],
}

impl CgPlayerDelete {
    /// Construct a deletion request without validating its index or code.
    #[must_use]
    pub const fn new(index: u8, private_code: [u8; CG_PRIVATE_CODE_FIELD_BYTES]) -> Self {
        Self {
            index,
            private_code,
        }
    }

    /// Encode the exact ten-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CG_PLAYER_DELETE_WIRE_SIZE);
        bytes.push(HEADER_CG_CHARACTER_DELETE.value());
        bytes.push(self.index);
        bytes.extend_from_slice(&self.private_code);
        bytes
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        let mut payload = [0_u8; CG_PLAYER_DELETE_WIRE_SIZE - 1];
        payload[0] = self.index;
        payload[1..].copy_from_slice(&self.private_code);
        ClientFrame::new(HEADER_CG_CHARACTER_DELETE.value(), payload)
    }

    /// Decode one exact complete character-deletion request.
    ///
    /// # Errors
    ///
    /// Returns [`CgAccountError::Truncated`] for fewer than ten bytes,
    /// [`CgAccountError::LengthMismatch`] for more than ten bytes, and
    /// [`CgAccountError::InvalidHeader`] for a complete record with another
    /// header.
    pub fn decode(data: &[u8]) -> Result<Self, CgAccountError> {
        check_exact(data, CG_PLAYER_DELETE_WIRE_SIZE)?;
        Self::decode_parts(data[0], &data[1..])
    }

    /// Decode a [`ClientFrame`] whose payload excludes the one-byte header.
    ///
    /// # Errors
    ///
    /// Returns a length error when the frame payload is not exactly nine bytes,
    /// or [`CgAccountError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgAccountError> {
        check_frame_payload(
            frame,
            CG_PLAYER_DELETE_WIRE_SIZE - 1,
            CG_PLAYER_DELETE_WIRE_SIZE,
        )?;
        Self::decode_parts(frame.header, &frame.payload)
    }

    fn decode_parts(header: u8, payload: &[u8]) -> Result<Self, CgAccountError> {
        if header != HEADER_CG_CHARACTER_DELETE.value() {
            return Err(CgAccountError::InvalidHeader { actual: header });
        }
        debug_assert_eq!(payload.len(), CG_PLAYER_DELETE_WIRE_SIZE - 1);
        let private_code = payload[1..]
            .try_into()
            .expect("length checked before private-code field copy");
        Ok(Self::new(payload[0], private_code))
    }
}

/// A character-creation request (`TPacketCGPlayerCreate`).
///
/// The index, fixed raw name field, job word, shape, and four stat bytes stay
/// opaque at this wire boundary. The codec does not require a NUL in the name,
/// choose an encoding, validate a slot, narrow or validate a job/race, or
/// derive statistics from the submitted values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgPlayerCreate {
    /// Source account-character `index` byte.
    pub index: u8,
    /// Raw source `name[CHARACTER_NAME_MAX_LEN + 1]` bytes.
    pub name: [u8; CG_PLAYER_CREATE_NAME_FIELD_BYTES],
    /// Source `job` word, encoded little-endian on the active x86 wire.
    pub job: u16,
    /// Source `shape` byte.
    pub shape: u8,
    /// Source `Con` byte.
    pub con: u8,
    /// Source `Int` byte.
    pub int_: u8,
    /// Source `Str` byte.
    pub str_: u8,
    /// Source `Dex` byte.
    pub dex: u8,
}

impl CgPlayerCreate {
    /// Construct a creation request without applying admission or name policy.
    ///
    /// The argument list mirrors the exact source field order of
    /// `TPacketCGPlayerCreate`, so this constructor keeps the record's own
    /// eight-argument shape instead of grouping fields into a new type.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        index: u8,
        name: [u8; CG_PLAYER_CREATE_NAME_FIELD_BYTES],
        job: u16,
        shape: u8,
        con: u8,
        int_: u8,
        str_: u8,
        dex: u8,
    ) -> Self {
        Self {
            index,
            name,
            job,
            shape,
            con,
            int_,
            str_,
            dex,
        }
    }

    /// Encode the exact 34-byte packed record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CG_PLAYER_CREATE_WIRE_SIZE);
        bytes.push(HEADER_CG_CHARACTER_CREATE.value());
        bytes.push(self.index);
        bytes.extend_from_slice(&self.name);
        bytes.extend_from_slice(&self.job.to_le_bytes());
        bytes.extend_from_slice(&[self.shape, self.con, self.int_, self.str_, self.dex]);
        bytes
    }

    /// Build a [`ClientFrame`] whose payload excludes the one-byte header.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_PLAYER_CREATE_PAYLOAD_SIZE);
        payload.push(self.index);
        payload.extend_from_slice(&self.name);
        payload.extend_from_slice(&self.job.to_le_bytes());
        payload.extend_from_slice(&[self.shape, self.con, self.int_, self.str_, self.dex]);
        ClientFrame::new(HEADER_CG_CHARACTER_CREATE.value(), payload)
    }

    /// Decode one exact complete character-creation request.
    ///
    /// # Errors
    ///
    /// Returns [`CgAccountError::Truncated`] for fewer than 34 bytes,
    /// [`CgAccountError::LengthMismatch`] for more than 34 bytes, and
    /// [`CgAccountError::InvalidHeader`] for a complete record with another
    /// header.
    pub fn decode(data: &[u8]) -> Result<Self, CgAccountError> {
        check_exact(data, CG_PLAYER_CREATE_WIRE_SIZE)?;
        Self::decode_parts(data[0], &data[1..])
    }

    /// Decode a [`ClientFrame`] whose payload excludes the one-byte header.
    ///
    /// # Errors
    ///
    /// Returns a length error when the frame payload is not exactly 33 bytes,
    /// or [`CgAccountError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgAccountError> {
        check_frame_payload(
            frame,
            CG_PLAYER_CREATE_PAYLOAD_SIZE,
            CG_PLAYER_CREATE_WIRE_SIZE,
        )?;
        Self::decode_parts(frame.header, &frame.payload)
    }

    fn decode_parts(header: u8, payload: &[u8]) -> Result<Self, CgAccountError> {
        if header != HEADER_CG_CHARACTER_CREATE.value() {
            return Err(CgAccountError::InvalidHeader { actual: header });
        }
        debug_assert_eq!(payload.len(), CG_PLAYER_CREATE_PAYLOAD_SIZE);
        let name_start = 1;
        let name_end = name_start + CG_PLAYER_CREATE_NAME_FIELD_BYTES;
        let name = payload[name_start..name_end]
            .try_into()
            .expect("length checked before character-name field copy");
        let job = u16::from_le_bytes(
            payload[name_end..name_end + 2]
                .try_into()
                .expect("length checked before job-field copy"),
        );
        let stats_start = name_end + 2;
        Ok(Self::new(
            payload[0],
            name,
            job,
            payload[stats_start],
            payload[stats_start + 1],
            payload[stats_start + 2],
            payload[stats_start + 3],
            payload[stats_start + 4],
        ))
    }
}

/// An enter-game phase record (`TPacketCGEnterGame`).
///
/// The source record contains only its header. It does not imply that a
/// character, world, or game state is valid; lifecycle and reducer checks
/// remain separate boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgEnterGame;

impl CgEnterGame {
    /// Construct the header-only record.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Encode the exact one-byte record.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        vec![HEADER_CG_ENTERGAME.value()]
    }

    /// Build a [`ClientFrame`] with an empty payload.
    #[must_use]
    pub fn to_frame(self) -> ClientFrame {
        ClientFrame::new(HEADER_CG_ENTERGAME.value(), Vec::new())
    }

    /// Decode one exact complete enter-game record.
    ///
    /// # Errors
    ///
    /// Returns [`CgAccountError::Truncated`] for an empty complete buffer,
    /// [`CgAccountError::LengthMismatch`] for bytes beyond the header, and
    /// [`CgAccountError::InvalidHeader`] for another header.
    pub fn decode(data: &[u8]) -> Result<Self, CgAccountError> {
        check_exact(data, CG_ENTER_GAME_WIRE_SIZE)?;
        if data[0] != HEADER_CG_ENTERGAME.value() {
            return Err(CgAccountError::InvalidHeader { actual: data[0] });
        }
        Ok(Self)
    }

    /// Decode a [`ClientFrame`] whose payload excludes the one-byte header.
    ///
    /// # Errors
    ///
    /// Returns [`CgAccountError::LengthMismatch`] when any payload byte is
    /// present, or [`CgAccountError::InvalidHeader`] for another header.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgAccountError> {
        if !frame.payload.is_empty() {
            let actual = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
            return Err(CgAccountError::LengthMismatch {
                expected: CG_ENTER_GAME_WIRE_SIZE,
                actual,
            });
        }
        if frame.header != HEADER_CG_ENTERGAME.value() {
            return Err(CgAccountError::InvalidHeader {
                actual: frame.header,
            });
        }
        Ok(Self)
    }
}

impl Default for CgEnterGame {
    /// Construct the header-only record using [`CgEnterGame::new`].
    fn default() -> Self {
        Self::new()
    }
}

/// A malformed fixed account-phase record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgAccountError {
    /// The complete record was shorter than its packed wire size.
    Truncated {
        /// Required complete wire size, including the header.
        needed: usize,
        /// Bytes supplied by the caller.
        available: usize,
    },
    /// The complete record had bytes beyond its packed wire size.
    LengthMismatch {
        /// Exact complete wire size, including the header.
        expected: usize,
        /// Bytes supplied by the caller.
        actual: usize,
    },
    /// The record used a header other than the source-defined account header.
    InvalidHeader {
        /// Unsupported one-byte header supplied by the caller.
        actual: u8,
    },
}

impl fmt::Display for CgAccountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => write!(
                formatter,
                "account-phase record is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                formatter,
                "account-phase record has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { actual } => {
                write!(
                    formatter,
                    "account-phase record has unsupported header 0x{actual:02x}"
                )
            }
        }
    }
}

impl Error for CgAccountError {}

fn check_exact(data: &[u8], expected: usize) -> Result<(), CgAccountError> {
    match data.len().cmp(&expected) {
        std::cmp::Ordering::Equal => Ok(()),
        std::cmp::Ordering::Less => Err(CgAccountError::Truncated {
            needed: expected,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(CgAccountError::LengthMismatch {
            expected,
            actual: data.len(),
        }),
    }
}

fn check_frame_payload(
    frame: &ClientFrame,
    expected_payload: usize,
    expected_complete: usize,
) -> Result<(), CgAccountError> {
    if frame.payload.len() == expected_payload {
        return Ok(());
    }
    let actual = frame.payload.len().checked_add(1).unwrap_or(usize::MAX);
    if frame.payload.len() < expected_payload {
        Err(CgAccountError::Truncated {
            needed: expected_complete,
            available: actual,
        })
    } else {
        Err(CgAccountError::LengthMismatch {
            expected: expected_complete,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_wire::{
        resolve_client_frame_size, ClientFrameDecoder, ClientFrameEncoder, ClientFrameSize,
    };

    fn sample_login() -> CgLogin {
        CgLogin::new(
            [0x41; CG_LOGIN_FIELD_BYTES],
            [0xff; CG_PASSWORD_FIELD_BYTES],
        )
    }

    fn sample_login_by_key() -> CgLoginByKey {
        CgLoginByKey::new(
            [0x42; CG_LOGIN_FIELD_BYTES],
            0x1234_5678,
            [0x0102_0304, 0x1112_1314, 0x2122_2324, u32::MAX],
        )
    }

    fn sample_player_delete() -> CgPlayerDelete {
        CgPlayerDelete::new(3, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88])
    }

    fn sample_player_create() -> CgPlayerCreate {
        let mut name = [0_u8; CG_PLAYER_CREATE_NAME_FIELD_BYTES];
        name[..5].copy_from_slice(b"Hero!");
        name[5] = 0;
        name[6] = 0xff;
        name[24] = 0x7f;
        CgPlayerCreate::new(3, name, 0x1234, 1, 0xfe, 0xfd, 0xfc, 0xfb)
    }

    #[test]
    fn source_sizes_and_headers_match_packet_info() {
        assert_eq!(HEADER_CG_LOGIN.value(), 0x01);
        assert_eq!(HEADER_CG_LOGIN2.value(), 0x6d);
        assert_eq!(HEADER_CG_CHARACTER_SELECT.value(), 0x06);
        assert_eq!(HEADER_CG_CHARACTER_DELETE.value(), 0x05);
        assert_eq!(HEADER_CG_CHARACTER_CREATE.value(), 0x04);
        assert_eq!(HEADER_CG_ENTERGAME.value(), 0x0a);
        assert_eq!(CG_LOGIN_WIRE_SIZE, 49);
        assert_eq!(CG_LOGIN_BY_KEY_WIRE_SIZE, 52);
        assert_eq!(CG_PLAYER_SELECT_WIRE_SIZE, 2);
        assert_eq!(CG_PRIVATE_CODE_FIELD_BYTES, 8);
        assert_eq!(CG_PLAYER_DELETE_WIRE_SIZE, 10);
        assert_eq!(CG_PLAYER_CREATE_NAME_FIELD_BYTES, 25);
        assert_eq!(CG_PLAYER_CREATE_PAYLOAD_SIZE, 33);
        assert_eq!(CG_PLAYER_CREATE_WIRE_SIZE, 34);
        assert_eq!(CG_ENTER_GAME_WIRE_SIZE, 1);
        assert_eq!(
            resolve_client_frame_size(HEADER_CG_CHARACTER_CREATE.value()).unwrap(),
            ClientFrameSize::Fixed(34)
        );
    }

    #[test]
    fn all_account_records_round_trip_through_raw_and_frame_forms() {
        let records = [
            ("login", sample_login().encode(), sample_login().to_frame()),
            (
                "login-by-key",
                sample_login_by_key().encode(),
                sample_login_by_key().to_frame(),
            ),
            (
                "select",
                CgPlayerSelect::new(3).encode(),
                CgPlayerSelect::new(3).to_frame(),
            ),
            (
                "delete",
                sample_player_delete().encode(),
                sample_player_delete().to_frame(),
            ),
            (
                "create",
                sample_player_create().encode(),
                sample_player_create().to_frame(),
            ),
            (
                "enter-game",
                CgEnterGame::new().encode(),
                CgEnterGame::new().to_frame(),
            ),
        ];
        for (name, raw, frame) in records {
            assert_eq!(
                ClientFrameEncoder::new().encode(&frame).unwrap(),
                raw,
                "{name}"
            );
            match name {
                "login" => assert_eq!(CgLogin::decode(&raw).unwrap(), sample_login()),
                "login-by-key" => {
                    assert_eq!(CgLoginByKey::decode(&raw).unwrap(), sample_login_by_key());
                }
                "select" => assert_eq!(
                    CgPlayerSelect::decode(&raw).unwrap(),
                    CgPlayerSelect::new(3)
                ),
                "delete" => assert_eq!(
                    CgPlayerDelete::decode(&raw).unwrap(),
                    sample_player_delete()
                ),
                "create" => assert_eq!(
                    CgPlayerCreate::decode(&raw).unwrap(),
                    sample_player_create()
                ),
                "enter-game" => assert_eq!(CgEnterGame::decode(&raw).unwrap(), CgEnterGame),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn login_by_key_preserves_field_order_and_explicit_little_endian_words() {
        let packet = sample_login_by_key();
        let raw = packet.encode();
        assert_eq!(&raw[..5], &[0x6d, 0x42, 0x42, 0x42, 0x42]);
        assert_eq!(&raw[32..36], &[0x78, 0x56, 0x34, 0x12]);
        assert_eq!(&raw[36..40], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&raw[40..44], &[0x14, 0x13, 0x12, 0x11]);
        assert_eq!(&raw[44..48], &[0x24, 0x23, 0x22, 0x21]);
        assert_eq!(&raw[48..], &[0xff, 0xff, 0xff, 0xff]);
        assert_eq!(
            CgLoginByKey::decode_frame(&packet.to_frame()).unwrap(),
            packet
        );
    }

    #[test]
    fn every_truncated_length_is_rejected_for_each_record() {
        let cases: [(&[u8], usize); 6] = [
            (&sample_login().encode(), CG_LOGIN_WIRE_SIZE),
            (&sample_login_by_key().encode(), CG_LOGIN_BY_KEY_WIRE_SIZE),
            (&CgPlayerSelect::new(2).encode(), CG_PLAYER_SELECT_WIRE_SIZE),
            (&sample_player_delete().encode(), CG_PLAYER_DELETE_WIRE_SIZE),
            (&sample_player_create().encode(), CG_PLAYER_CREATE_WIRE_SIZE),
            (&CgEnterGame::new().encode(), CG_ENTER_GAME_WIRE_SIZE),
        ];
        for (complete, expected) in cases {
            for available in 0..expected {
                let error = match expected {
                    CG_LOGIN_WIRE_SIZE => CgLogin::decode(&complete[..available]).unwrap_err(),
                    CG_LOGIN_BY_KEY_WIRE_SIZE => {
                        CgLoginByKey::decode(&complete[..available]).unwrap_err()
                    }
                    CG_PLAYER_SELECT_WIRE_SIZE => {
                        CgPlayerSelect::decode(&complete[..available]).unwrap_err()
                    }
                    CG_PLAYER_DELETE_WIRE_SIZE => {
                        CgPlayerDelete::decode(&complete[..available]).unwrap_err()
                    }
                    CG_PLAYER_CREATE_WIRE_SIZE => {
                        CgPlayerCreate::decode(&complete[..available]).unwrap_err()
                    }
                    CG_ENTER_GAME_WIRE_SIZE => {
                        CgEnterGame::decode(&complete[..available]).unwrap_err()
                    }
                    _ => unreachable!(),
                };
                assert_eq!(
                    error,
                    CgAccountError::Truncated {
                        needed: expected,
                        available
                    }
                );
            }
        }
    }

    #[test]
    fn trailing_bytes_and_wrong_headers_are_rejected() {
        let mut login = sample_login().encode();
        login.push(0);
        assert_eq!(
            CgLogin::decode(&login),
            Err(CgAccountError::LengthMismatch {
                expected: CG_LOGIN_WIRE_SIZE,
                actual: CG_LOGIN_WIRE_SIZE + 1,
            })
        );
        let mut wrong_login = sample_login().encode();
        wrong_login[0] = 0x02;
        assert_eq!(
            CgLogin::decode(&wrong_login),
            Err(CgAccountError::InvalidHeader { actual: 0x02 })
        );

        let mut key = sample_login_by_key().encode();
        key.push(0);
        assert_eq!(
            CgLoginByKey::decode(&key),
            Err(CgAccountError::LengthMismatch {
                expected: CG_LOGIN_BY_KEY_WIRE_SIZE,
                actual: CG_LOGIN_BY_KEY_WIRE_SIZE + 1,
            })
        );
        let mut wrong_key = sample_login_by_key().encode();
        wrong_key[0] = 0x6c;
        assert_eq!(
            CgLoginByKey::decode(&wrong_key),
            Err(CgAccountError::InvalidHeader { actual: 0x6c })
        );

        let mut select = CgPlayerSelect::new(0).encode();
        select.push(0);
        assert_eq!(
            CgPlayerSelect::decode(&select),
            Err(CgAccountError::LengthMismatch {
                expected: CG_PLAYER_SELECT_WIRE_SIZE,
                actual: CG_PLAYER_SELECT_WIRE_SIZE + 1,
            })
        );
        assert_eq!(
            CgPlayerSelect::decode(&[0x07, 0]),
            Err(CgAccountError::InvalidHeader { actual: 0x07 })
        );

        let mut delete = sample_player_delete().encode();
        delete.push(0);
        assert_eq!(
            CgPlayerDelete::decode(&delete),
            Err(CgAccountError::LengthMismatch {
                expected: CG_PLAYER_DELETE_WIRE_SIZE,
                actual: CG_PLAYER_DELETE_WIRE_SIZE + 1,
            })
        );
        assert_eq!(
            CgPlayerDelete::decode(&[0x06; CG_PLAYER_DELETE_WIRE_SIZE]),
            Err(CgAccountError::InvalidHeader { actual: 0x06 })
        );

        let mut create = sample_player_create().encode();
        create.push(0);
        assert_eq!(
            CgPlayerCreate::decode(&create),
            Err(CgAccountError::LengthMismatch {
                expected: CG_PLAYER_CREATE_WIRE_SIZE,
                actual: CG_PLAYER_CREATE_WIRE_SIZE + 1,
            })
        );
        assert_eq!(
            CgPlayerCreate::decode(&[0x05; CG_PLAYER_CREATE_WIRE_SIZE]),
            Err(CgAccountError::InvalidHeader { actual: 0x05 })
        );

        assert_eq!(
            CgEnterGame::decode(&[0x0a, 0]),
            Err(CgAccountError::LengthMismatch {
                expected: CG_ENTER_GAME_WIRE_SIZE,
                actual: 2,
            })
        );
        assert_eq!(
            CgEnterGame::decode(&[0x0b]),
            Err(CgAccountError::InvalidHeader { actual: 0x0b })
        );
    }

    #[test]
    fn frame_payload_lengths_and_headers_are_checked_without_consuming_extra_bytes() {
        assert_eq!(
            CgLogin::decode_frame(&ClientFrame::new(0x01, [0; CG_LOGIN_WIRE_SIZE - 2])),
            Err(CgAccountError::Truncated {
                needed: CG_LOGIN_WIRE_SIZE,
                available: CG_LOGIN_WIRE_SIZE - 1,
            })
        );
        assert_eq!(
            CgLoginByKey::decode_frame(&ClientFrame::new(0x6d, [0; CG_LOGIN_BY_KEY_WIRE_SIZE])),
            Err(CgAccountError::LengthMismatch {
                expected: CG_LOGIN_BY_KEY_WIRE_SIZE,
                actual: CG_LOGIN_BY_KEY_WIRE_SIZE + 1,
            })
        );
        assert_eq!(
            CgPlayerSelect::decode_frame(&ClientFrame::new(0x06, [])),
            Err(CgAccountError::Truncated {
                needed: CG_PLAYER_SELECT_WIRE_SIZE,
                available: 1,
            })
        );
        assert_eq!(
            CgPlayerDelete::decode_frame(&ClientFrame::new(
                0x05,
                [0; CG_PLAYER_DELETE_WIRE_SIZE - 2],
            )),
            Err(CgAccountError::Truncated {
                needed: CG_PLAYER_DELETE_WIRE_SIZE,
                available: CG_PLAYER_DELETE_WIRE_SIZE - 1,
            })
        );
        assert_eq!(
            CgPlayerDelete::decode_frame(&ClientFrame::new(0x06, [0; 9])),
            Err(CgAccountError::InvalidHeader { actual: 0x06 })
        );
        assert_eq!(
            CgPlayerCreate::decode_frame(&ClientFrame::new(
                0x04,
                [0; CG_PLAYER_CREATE_PAYLOAD_SIZE - 1],
            )),
            Err(CgAccountError::Truncated {
                needed: CG_PLAYER_CREATE_WIRE_SIZE,
                available: CG_PLAYER_CREATE_WIRE_SIZE - 1,
            })
        );
        assert_eq!(
            CgPlayerCreate::decode_frame(&ClientFrame::new(
                0x05,
                [0; CG_PLAYER_CREATE_PAYLOAD_SIZE]
            )),
            Err(CgAccountError::InvalidHeader { actual: 0x05 })
        );
        assert_eq!(
            CgEnterGame::decode_frame(&ClientFrame::new(0x0a, [0])),
            Err(CgAccountError::LengthMismatch {
                expected: CG_ENTER_GAME_WIRE_SIZE,
                actual: 2,
            })
        );
        assert_eq!(
            CgEnterGame::decode_frame(&ClientFrame::new(0x0b, [])),
            Err(CgAccountError::InvalidHeader { actual: 0x0b })
        );
    }

    #[test]
    fn fixed_decoder_handles_fragmented_and_coalesced_account_frames() {
        let first = sample_login().encode();
        let second = sample_login_by_key().encode();
        let mut decoder = ClientFrameDecoder::new();
        decoder.feed(&first[..7]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&first[7..]).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgLogin::decode_frame(&frame).unwrap(), sample_login());

        let mut coalesced = second;
        coalesced.extend_from_slice(&sample_player_delete().encode());
        coalesced.push(0x0a);
        decoder.feed(&coalesced).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgLoginByKey::decode_frame(&frame).unwrap(),
            sample_login_by_key()
        );
        let delete = decoder.try_decode().unwrap().unwrap();
        assert_eq!(
            CgPlayerDelete::decode_frame(&delete).unwrap(),
            sample_player_delete()
        );
        let enter = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgEnterGame::decode_frame(&enter).unwrap(), CgEnterGame);
    }

    #[test]
    fn cg_player_delete_round_trips_every_index_with_exact_layout() {
        let golden = [0x05, 0x03, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let decoded = CgPlayerDelete::decode(&golden).unwrap();
        assert_eq!(decoded.encode(), golden);
        assert_eq!(decoded.to_frame(), ClientFrame::new(0x05, &golden[1..]));
        assert_eq!(
            CgPlayerDelete::decode_frame(&ClientFrame::new(0x05, &golden[1..])).unwrap(),
            decoded
        );

        let private_code = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        for index in u8::MIN..=u8::MAX {
            let packet = CgPlayerDelete::new(index, private_code);
            let raw = packet.encode();
            assert_eq!(raw.len(), CG_PLAYER_DELETE_WIRE_SIZE);
            assert_eq!(&raw[..2], &[HEADER_CG_CHARACTER_DELETE.value(), index]);
            assert_eq!(&raw[2..], &private_code);
            assert_eq!(CgPlayerDelete::decode(&raw).unwrap(), packet);
            assert_eq!(
                CgPlayerDelete::decode_frame(&packet.to_frame()).unwrap(),
                packet
            );
        }
    }

    #[test]
    fn cg_player_delete_preserves_opaque_private_code_bytes() {
        let codes = [
            [0x00; CG_PRIVATE_CODE_FIELD_BYTES],
            [0xff; CG_PRIVATE_CODE_FIELD_BYTES],
            [b'a'; CG_PRIVATE_CODE_FIELD_BYTES],
            [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            [0xff, 0x00, 0xff, 0x00, 0xff, 0x00, 0xff, 0x00],
        ];
        for private_code in codes {
            let packet = CgPlayerDelete::new(0xff, private_code);
            let raw = packet.encode();
            assert_eq!(&raw[2..], &private_code);
            assert_eq!(CgPlayerDelete::decode(&raw).unwrap(), packet);
            assert_eq!(
                CgPlayerDelete::decode_frame(&packet.to_frame()).unwrap(),
                packet
            );
        }
    }

    #[test]
    fn cg_player_delete_rejects_all_wrong_headers_and_checks_length_first() {
        for header in u8::MIN..=u8::MAX {
            if header == HEADER_CG_CHARACTER_DELETE.value() {
                continue;
            }
            let mut raw = sample_player_delete().encode();
            raw[0] = header;
            assert_eq!(
                CgPlayerDelete::decode(&raw),
                Err(CgAccountError::InvalidHeader { actual: header })
            );
        }

        assert_eq!(
            CgPlayerDelete::decode(&[0x06]),
            Err(CgAccountError::Truncated {
                needed: CG_PLAYER_DELETE_WIRE_SIZE,
                available: 1,
            })
        );
        assert_eq!(
            CgPlayerDelete::decode(&[0x06; CG_PLAYER_DELETE_WIRE_SIZE + 1]),
            Err(CgAccountError::LengthMismatch {
                expected: CG_PLAYER_DELETE_WIRE_SIZE,
                actual: CG_PLAYER_DELETE_WIRE_SIZE + 1,
            })
        );
    }

    #[test]
    fn cg_player_delete_frame_boundaries_and_streaming_are_exact() {
        let packet = sample_player_delete();
        let frame = packet.to_frame();
        assert_eq!(frame.header, HEADER_CG_CHARACTER_DELETE.value());
        assert_eq!(frame.payload.len(), CG_PLAYER_DELETE_WIRE_SIZE - 1);
        assert_eq!(CgPlayerDelete::decode_frame(&frame).unwrap(), packet);

        for payload_len in 0..=(CG_PLAYER_DELETE_WIRE_SIZE - 2) {
            assert_eq!(
                CgPlayerDelete::decode_frame(&ClientFrame::new(0x05, vec![0; payload_len])),
                Err(CgAccountError::Truncated {
                    needed: CG_PLAYER_DELETE_WIRE_SIZE,
                    available: payload_len + 1,
                })
            );
        }
        assert_eq!(
            CgPlayerDelete::decode_frame(&ClientFrame::new(0x05, [0; 10])),
            Err(CgAccountError::LengthMismatch {
                expected: CG_PLAYER_DELETE_WIRE_SIZE,
                actual: CG_PLAYER_DELETE_WIRE_SIZE + 1,
            })
        );
        assert_eq!(
            CgPlayerDelete::decode_frame(&ClientFrame::new(0x06, [0; 9])),
            Err(CgAccountError::InvalidHeader { actual: 0x06 })
        );

        let mut decoder = ClientFrameDecoder::new();
        let raw = packet.encode();
        decoder.feed(&raw[..4]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        let mut coalesced = raw[4..].to_vec();
        coalesced.push(HEADER_CG_ENTERGAME.value());
        decoder.feed(&coalesced).unwrap();
        let delete = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgPlayerDelete::decode_frame(&delete).unwrap(), packet);
        let enter = decoder.try_decode().unwrap().unwrap();
        assert_eq!(CgEnterGame::decode_frame(&enter).unwrap(), CgEnterGame);
    }

    #[test]
    fn cg_player_create_matches_source_layout_and_literal_golden() {
        let golden = [
            0x04, 0x03, 0x48, 0x65, 0x72, 0x6f, 0x21, 0x00, 0xff, 0x80, 0x01, 0x00, 0x7f, 0xaa,
            0xbb, 0xcc, 0xdd, 0xee, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x7e, 0x7d, 0x34,
            0x12, 0x01, 0xfe, 0xfd, 0xfc, 0xfb,
        ];
        assert_eq!(golden.len(), CG_PLAYER_CREATE_WIRE_SIZE);
        assert_eq!(&golden[..2], &[0x04, 0x03]);
        assert_eq!(
            &golden[2..27],
            &[
                0x48, 0x65, 0x72, 0x6f, 0x21, 0x00, 0xff, 0x80, 0x01, 0x00, 0x7f, 0xaa, 0xbb, 0xcc,
                0xdd, 0xee, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x7e, 0x7d,
            ]
        );
        assert_eq!(&golden[27..29], &[0x34, 0x12]);
        assert_eq!(&golden[29..], &[0x01, 0xfe, 0xfd, 0xfc, 0xfb]);

        let packet = CgPlayerCreate::decode(&golden).unwrap();
        assert_eq!(packet.index, 3);
        assert_eq!(packet.job, 0x1234);
        assert_eq!(packet.shape, 1);
        assert_eq!(packet.con, 0xfe);
        assert_eq!(packet.int_, 0xfd);
        assert_eq!(packet.str_, 0xfc);
        assert_eq!(packet.dex, 0xfb);
        assert_eq!(packet.encode(), golden);
        let frame = packet.to_frame();
        assert_eq!(frame.header, HEADER_CG_CHARACTER_CREATE.value());
        assert_eq!(frame.payload, golden[1..].to_vec());
        assert_eq!(CgPlayerCreate::decode_frame(&frame).unwrap(), packet);
    }

    #[test]
    fn cg_player_create_preserves_every_index_and_opaque_create_fields() {
        let names = [
            [0x00; CG_PLAYER_CREATE_NAME_FIELD_BYTES],
            [0xff; CG_PLAYER_CREATE_NAME_FIELD_BYTES],
            [b'A'; CG_PLAYER_CREATE_NAME_FIELD_BYTES],
            [
                0x01, 0x02, 0x00, 0x80, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x7f,
            ],
        ];
        for index in u8::MIN..=u8::MAX {
            let packet = CgPlayerCreate::new(
                index,
                names[usize::from(index) % names.len()],
                u16::MAX,
                0xff,
                0x01,
                0x80,
                0xfe,
                0x7f,
            );
            let raw = packet.encode();
            assert_eq!(raw.len(), CG_PLAYER_CREATE_WIRE_SIZE);
            assert_eq!(&raw[..2], &[HEADER_CG_CHARACTER_CREATE.value(), index]);
            assert_eq!(&raw[2..27], &packet.name);
            assert_eq!(&raw[27..29], &[0xff, 0xff]);
            assert_eq!(&raw[29..], &[0xff, 0x01, 0x80, 0xfe, 0x7f]);
            assert_eq!(CgPlayerCreate::decode(&raw).unwrap(), packet);
            assert_eq!(
                CgPlayerCreate::decode_frame(&packet.to_frame()).unwrap(),
                packet
            );
        }

        for (job, shape, con, int_, str_, dex) in [
            (0_u16, 0_u8, 0_u8, 0_u8, 0_u8, 0_u8),
            (1, 1, 0xff, 0x80, 0x7f, 0xfe),
            (0x1234, 0xff, 0x01, 0x02, 0x03, 0x04),
            (u16::MAX, 0x7f, 0xff, 0xaa, 0x55, 0xcc),
        ] {
            let packet = CgPlayerCreate::new(
                0,
                [0x5a; CG_PLAYER_CREATE_NAME_FIELD_BYTES],
                job,
                shape,
                con,
                int_,
                str_,
                dex,
            );
            let raw = packet.encode();
            assert_eq!(&raw[27..29], &job.to_le_bytes());
            assert_eq!(&raw[29..], &[shape, con, int_, str_, dex]);
            assert_eq!(CgPlayerCreate::decode(&raw).unwrap(), packet);
        }
    }

    #[test]
    fn cg_player_create_rejects_all_headers_and_checks_length_before_fields() {
        for header in u8::MIN..=u8::MAX {
            if header == HEADER_CG_CHARACTER_CREATE.value() {
                continue;
            }
            let mut raw = sample_player_create().encode();
            raw[0] = header;
            assert_eq!(
                CgPlayerCreate::decode(&raw),
                Err(CgAccountError::InvalidHeader { actual: header })
            );
        }

        for available in 0..CG_PLAYER_CREATE_WIRE_SIZE {
            assert_eq!(
                CgPlayerCreate::decode(&sample_player_create().encode()[..available]),
                Err(CgAccountError::Truncated {
                    needed: CG_PLAYER_CREATE_WIRE_SIZE,
                    available,
                })
            );
        }

        for header in [0x00_u8, 0x05, 0x06, 0xff] {
            for available in 0..CG_PLAYER_CREATE_WIRE_SIZE {
                let raw = vec![header; available];
                assert_eq!(
                    CgPlayerCreate::decode(&raw),
                    Err(CgAccountError::Truncated {
                        needed: CG_PLAYER_CREATE_WIRE_SIZE,
                        available,
                    }),
                    "short raw record {available} with wrong header {header:#04x}"
                );
            }
            for actual in [
                CG_PLAYER_CREATE_WIRE_SIZE + 1,
                CG_PLAYER_CREATE_WIRE_SIZE + 7,
            ] {
                assert_eq!(
                    CgPlayerCreate::decode(&vec![header; actual]),
                    Err(CgAccountError::LengthMismatch {
                        expected: CG_PLAYER_CREATE_WIRE_SIZE,
                        actual,
                    }),
                    "long raw record {actual} with wrong header {header:#04x}"
                );
            }
        }

        for header in u8::MIN..=u8::MAX {
            if header == HEADER_CG_CHARACTER_CREATE.value() {
                continue;
            }
            assert_eq!(
                CgPlayerCreate::decode_frame(&ClientFrame::new(
                    header,
                    vec![0; CG_PLAYER_CREATE_PAYLOAD_SIZE],
                )),
                Err(CgAccountError::InvalidHeader { actual: header }),
                "exact frame with wrong header {header:#04x}"
            );
        }
        for payload_len in 0..CG_PLAYER_CREATE_PAYLOAD_SIZE {
            assert_eq!(
                CgPlayerCreate::decode_frame(&ClientFrame::new(0x05, vec![0; payload_len])),
                Err(CgAccountError::Truncated {
                    needed: CG_PLAYER_CREATE_WIRE_SIZE,
                    available: payload_len + 1,
                }),
                "short frame payload {payload_len} with wrong header"
            );
        }
        for actual in [
            CG_PLAYER_CREATE_WIRE_SIZE + 1,
            CG_PLAYER_CREATE_WIRE_SIZE + 7,
        ] {
            assert_eq!(
                CgPlayerCreate::decode_frame(&ClientFrame::new(0x05, vec![0; actual - 1],)),
                Err(CgAccountError::LengthMismatch {
                    expected: CG_PLAYER_CREATE_WIRE_SIZE,
                    actual,
                }),
                "long frame payload {actual} with wrong header"
            );
        }
    }

    #[test]
    fn cg_player_create_frame_lengths_and_stream_boundaries_are_exact() {
        let packet = sample_player_create();
        let frame = packet.to_frame();
        assert_eq!(frame.payload.len(), CG_PLAYER_CREATE_PAYLOAD_SIZE);
        assert_eq!(CgPlayerCreate::decode_frame(&frame).unwrap(), packet);

        for payload_len in 0..CG_PLAYER_CREATE_PAYLOAD_SIZE {
            assert_eq!(
                CgPlayerCreate::decode_frame(&ClientFrame::new(0x04, vec![0; payload_len])),
                Err(CgAccountError::Truncated {
                    needed: CG_PLAYER_CREATE_WIRE_SIZE,
                    available: payload_len + 1,
                })
            );
        }
        assert_eq!(
            CgPlayerCreate::decode_frame(&ClientFrame::new(
                0x04,
                vec![0; CG_PLAYER_CREATE_PAYLOAD_SIZE + 1],
            )),
            Err(CgAccountError::LengthMismatch {
                expected: CG_PLAYER_CREATE_WIRE_SIZE,
                actual: CG_PLAYER_CREATE_WIRE_SIZE + 1,
            })
        );
        assert_eq!(
            CgPlayerCreate::decode_frame(&ClientFrame::new(
                0x05,
                vec![0; CG_PLAYER_CREATE_PAYLOAD_SIZE]
            )),
            Err(CgAccountError::InvalidHeader { actual: 0x05 })
        );

        let mut decoder = ClientFrameDecoder::new();
        let raw = packet.encode();
        decoder.feed(&raw[..1]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&raw[1..10]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        let mut coalesced = raw[10..].to_vec();
        coalesced.extend_from_slice(&sample_player_delete().encode());
        coalesced.push(HEADER_CG_ENTERGAME.value());
        decoder.feed(&coalesced).unwrap();
        assert_eq!(
            CgPlayerCreate::decode_frame(&decoder.try_decode().unwrap().unwrap()).unwrap(),
            packet
        );
        assert_eq!(
            CgPlayerDelete::decode_frame(&decoder.try_decode().unwrap().unwrap()).unwrap(),
            sample_player_delete()
        );
        assert_eq!(
            CgEnterGame::decode_frame(&decoder.try_decode().unwrap().unwrap()).unwrap(),
            CgEnterGame
        );
    }
}
