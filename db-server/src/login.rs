//! SQL-free response framing for the primary login-by-key outcome.
//!
//! Legacy evidence used by this module:
//!
//! * `server/server/common/tables.h:183-186` assigns response headers 30
//!   (`LOGIN_SUCCESS`), 31 (`LOGIN_NOT_EXIST`), 33 (`LOGIN_WRONG_PASSWD`), and
//!   34 (`LOGIN_ALREADY`). Only 30, 31, and 34 are primary login-by-key
//!   outcomes. Header 33 belongs to the password-login path.
//! * `server/server/common/tables.h:380-418` defines the packed active-build
//!   `TSimplePlayer` and `TAccountTable`. The existing
//!   [`LoginAccountRecord`] codec is the 362-byte success payload.
//! * `server/server/common/tables.h:1076-1079` defines the packed
//!   `TPacketDGLoginAlready` byte array. `LOGIN_MAX_LEN + 1` is 31.
//! * `server/server/common/tables.h:1291-1297` defines the 67-byte
//!   `TPacketGDLoginByKey` request. `server/server/game/input_login.cpp:153-218`
//!   fills it and passes the game descriptor handle to
//!   `HEADER_GD_LOGIN_BY_KEY`.
//! * `server/server/db/ClientManagerLogin.cpp:89-124` emits header 31 for a
//!   missing key, a login mismatch, or a client-key mismatch. Lines 98-104 emit
//!   header 34 plus a 31-byte login buffer when the resolved account is already
//!   logged in. Lines 151-264 continue into SQL work only after those checks.
//! * `server/server/db/ClientManagerLogin.cpp:569-570` emits header 30 and the
//!   packed `TAccountTable` for success.
//!
//! The legacy already-logged-in sender declares a stack record and calls
//! `strlcpy` without first clearing the record. Its bytes after the string NUL
//! are therefore indeterminate. [`LoginAlreadyRecord`] intentionally accepts
//! and preserves the caller's exact 31 raw bytes instead of inventing a
//! zero-filled tail.
//!
//! This module does not execute SQL, classify unresolved input, mutate the
//! logon-account map, create `player_index` rows, insert a resolved account, or
//! emit supplementary login frames. A caller must first resolve one of the
//! three outcomes represented by [`LoginByKeyPrimaryOutcome`].

use std::error::Error;
use std::fmt;

use protocol::db_records::{
    DbRecordError, LoginAccountRecord, LoginAlreadyRecord, HEADER_DG_LOGIN_ALREADY,
    HEADER_DG_LOGIN_NOT_EXIST, HEADER_DG_LOGIN_SUCCESS, LOGIN_ACCOUNT_WIRE_SIZE,
    LOGIN_ALREADY_WIRE_SIZE,
};
use protocol::db_wire::DbFrame;

/// An already-resolved primary login-by-key result.
///
/// This type has no `Unresolved`, SQL-error, service-unavailable, or
/// wrong-password variant. Callers must not turn an uncertain lookup into
/// [`LoginByKeyPrimaryOutcome::Missing`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum LoginByKeyPrimaryOutcome {
    /// A source-resolved `TAccountTable` for a successful login.
    SuccessAccount(LoginAccountRecord),
    /// A source-resolved header-31 result: missing key, mismatched identity,
    /// client-key mismatch, or a later source-defined SQL failure.
    Missing,
    /// A source-resolved already-logged-in result and its exact raw record.
    AlreadyLoggedIn(LoginAlreadyRecord),
}

/// A validated primary response with its original correlation handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginByKeyDecodedResponse {
    /// Handle copied from the decoded DB peer frame.
    pub handle: u32,
    /// Exact typed primary outcome carried by the frame.
    pub outcome: LoginByKeyPrimaryOutcome,
}

/// A pure adapter for the three source-defined primary login-by-key responses.
///
/// The adapter copies the supplied request handle into every frame. It emits
/// exactly one response frame and never dispatches the frame to a peer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LoginByKeyPrimaryResponseAdapter;

impl LoginByKeyPrimaryResponseAdapter {
    /// Construct the stateless response adapter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Compose one primary response frame from an already-resolved outcome.
    ///
    /// `SuccessAccount` writes header 30 and the existing 362-byte
    /// `LoginAccountRecord` encoding. `Missing` writes header 31 and an empty
    /// payload. `AlreadyLoggedIn` writes header 34 and exactly 31 raw bytes.
    #[must_use]
    pub fn response_for(&self, handle: u32, outcome: &LoginByKeyPrimaryOutcome) -> DbFrame {
        match outcome {
            LoginByKeyPrimaryOutcome::SuccessAccount(record) => {
                let payload = record.encode();
                debug_assert_eq!(payload.len(), LOGIN_ACCOUNT_WIRE_SIZE);
                DbFrame::new(HEADER_DG_LOGIN_SUCCESS, handle, payload)
            }
            LoginByKeyPrimaryOutcome::Missing => {
                DbFrame::new(HEADER_DG_LOGIN_NOT_EXIST, handle, Vec::new())
            }
            LoginByKeyPrimaryOutcome::AlreadyLoggedIn(record) => {
                let payload = record.encode();
                debug_assert_eq!(payload.len(), LOGIN_ALREADY_WIRE_SIZE);
                DbFrame::new(HEADER_DG_LOGIN_ALREADY, handle, payload)
            }
        }
    }
}

/// C++-style semantic alias for the primary outcome.
pub type LoginByKeyOutcome = LoginByKeyPrimaryOutcome;

/// An error raised while validating a primary login-by-key response frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginByKeyResponseError {
    /// The header is not one of the three supported primary outcomes.
    ///
    /// In particular, password-login header 33 and a GD request header are not
    /// converted into a missing login.
    UnsupportedHeader {
        /// Unsupported one-byte legacy protocol header.
        header: u8,
    },
    /// A success header did not contain an exact `TAccountTable` record.
    MalformedSuccessAccount {
        /// Exact-length codec failure.
        source: DbRecordError,
    },
    /// A missing-result header contained bytes instead of the legacy empty
    /// payload.
    MalformedMissingPayload {
        /// Number of unexpected payload bytes.
        actual_len: usize,
    },
    /// An already-logged-in header did not contain exactly 31 bytes.
    MalformedAlreadyLoggedIn {
        /// Exact-length codec failure.
        source: DbRecordError,
    },
}

impl fmt::Display for LoginByKeyResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedHeader { header } => {
                write!(
                    formatter,
                    "unsupported login-by-key response header {header}"
                )
            }
            Self::MalformedSuccessAccount { .. } => {
                formatter.write_str("malformed login success account payload")
            }
            Self::MalformedMissingPayload { actual_len } => write!(
                formatter,
                "login missing response has {actual_len} bytes; expected 0"
            ),
            Self::MalformedAlreadyLoggedIn { .. } => {
                formatter.write_str("malformed already-logged-in payload")
            }
        }
    }
}

impl Error for LoginByKeyResponseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MalformedSuccessAccount { source }
            | Self::MalformedAlreadyLoggedIn { source } => Some(source),
            Self::UnsupportedHeader { .. } | Self::MalformedMissingPayload { .. } => None,
        }
    }
}

/// Decode and validate a source-defined primary response without dispatch.
///
/// This helper checks only the response header and exact payload shape. The
/// returned wrapper retains `frame.handle`, but the helper does not verify
/// handle ownership, login state, account contents, or any condition that led
/// to the outcome. Unknown or uncertain response headers are returned as
/// [`LoginByKeyResponseError::UnsupportedHeader`].
///
/// # Errors
///
/// Returns an error for an unsupported header or a malformed supported
/// payload.
pub fn decode_primary_response(
    frame: &DbFrame,
) -> Result<LoginByKeyDecodedResponse, LoginByKeyResponseError> {
    let outcome = match frame.header {
        HEADER_DG_LOGIN_SUCCESS => LoginAccountRecord::decode(&frame.payload)
            .map(LoginByKeyPrimaryOutcome::SuccessAccount)
            .map_err(|source| LoginByKeyResponseError::MalformedSuccessAccount { source }),
        HEADER_DG_LOGIN_NOT_EXIST if frame.payload.is_empty() => {
            Ok(LoginByKeyPrimaryOutcome::Missing)
        }
        HEADER_DG_LOGIN_NOT_EXIST => Err(LoginByKeyResponseError::MalformedMissingPayload {
            actual_len: frame.payload.len(),
        }),
        HEADER_DG_LOGIN_ALREADY => LoginAlreadyRecord::decode(&frame.payload)
            .map(LoginByKeyPrimaryOutcome::AlreadyLoggedIn)
            .map_err(|source| LoginByKeyResponseError::MalformedAlreadyLoggedIn { source }),
        header => Err(LoginByKeyResponseError::UnsupportedHeader { header }),
    }?;
    Ok(LoginByKeyDecodedResponse {
        handle: frame.handle,
        outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_records::{
        LoginByKeyRequest, HEADER_DG_LOGIN_WRONG_PASSWD, HEADER_GD_LOGIN_BY_KEY,
    };
    use protocol::db_wire::{DbFrameDecoder, DB_PEER_HEADER_SIZE};

    fn c_array<const N: usize>(value: &str) -> [u8; N] {
        let mut bytes = [0_u8; N];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        bytes
    }

    fn account() -> LoginAccountRecord {
        LoginAccountRecord {
            id: 0x1020_3040,
            login: c_array("alice"),
            passwd: c_array("secret"),
            social_id: c_array("social"),
            status: c_array("OK"),
            empire: 3,
            ..LoginAccountRecord::default()
        }
    }

    fn already_record() -> LoginAlreadyRecord {
        // Bytes after the NUL model a valid source-produced indeterminate tail.
        // They must be preserved, not replaced with zeros.
        let mut login = [0xa5; LOGIN_ALREADY_WIRE_SIZE];
        login[..5].copy_from_slice(b"alice");
        login[5] = 0;
        LoginAlreadyRecord { login }
    }

    fn assert_outer_header(frame: &DbFrame, payload_len: usize) {
        let encoded = frame.encode().unwrap();
        assert_eq!(encoded.len(), DB_PEER_HEADER_SIZE + payload_len);
        let expected_len = u32::try_from(payload_len).unwrap();
        let mut expected = Vec::with_capacity(DB_PEER_HEADER_SIZE);
        expected.push(frame.header);
        expected.extend_from_slice(&frame.handle.to_le_bytes());
        expected.extend_from_slice(&expected_len.to_le_bytes());
        assert_eq!(&encoded[..DB_PEER_HEADER_SIZE], expected);
    }

    fn wire_round_trip(frame: &DbFrame) -> DbFrame {
        let mut decoder = DbFrameDecoder::new();
        decoder.feed(&frame.encode().unwrap()).unwrap();
        let round_tripped = decoder.try_decode().unwrap().unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        round_tripped
    }

    #[test]
    fn success_account_has_exact_header_size_codec_and_handle() {
        let handle = 0x0102_0304;
        let outcome = LoginByKeyPrimaryOutcome::SuccessAccount(account());
        let frame = LoginByKeyPrimaryResponseAdapter::new().response_for(handle, &outcome);

        assert_eq!(frame.header, HEADER_DG_LOGIN_SUCCESS);
        assert_eq!(frame.handle, handle);
        assert_eq!(frame.payload.len(), LOGIN_ACCOUNT_WIRE_SIZE);
        assert_eq!(frame.payload.len(), 362);
        assert_outer_header(&frame, 362);
        assert_eq!(
            LoginAccountRecord::decode(&frame.payload).unwrap(),
            account()
        );
        let wire_frame = wire_round_trip(&frame);
        assert_eq!(wire_frame, frame);
        assert_eq!(
            decode_primary_response(&wire_frame).unwrap(),
            LoginByKeyDecodedResponse { handle, outcome }
        );
    }

    #[test]
    fn missing_has_exact_empty_header_and_preserves_zero_handle() {
        let handle = 0;
        let outcome = LoginByKeyPrimaryOutcome::Missing;
        let frame = LoginByKeyPrimaryResponseAdapter::new().response_for(handle, &outcome);

        assert_eq!(frame.header, HEADER_DG_LOGIN_NOT_EXIST);
        assert_eq!(frame.header, 31);
        assert_eq!(frame.handle, handle);
        assert!(frame.payload.is_empty());
        assert_outer_header(&frame, 0);
        assert_eq!(frame.encode().unwrap(), vec![31, 0, 0, 0, 0, 0, 0, 0, 0]);
        let wire_frame = wire_round_trip(&frame);
        assert_eq!(wire_frame, frame);
        assert_eq!(
            decode_primary_response(&wire_frame).unwrap(),
            LoginByKeyDecodedResponse { handle, outcome }
        );
    }

    #[test]
    fn already_logged_in_preserves_exact_31_raw_bytes_and_max_handle() {
        let handle = u32::MAX;
        let record = already_record();
        let outcome = LoginByKeyPrimaryOutcome::AlreadyLoggedIn(record);
        let frame = LoginByKeyPrimaryResponseAdapter::new().response_for(handle, &outcome);

        assert_eq!(frame.header, HEADER_DG_LOGIN_ALREADY);
        assert_eq!(frame.header, 34);
        assert_eq!(frame.handle, handle);
        assert_eq!(frame.payload.len(), LOGIN_ALREADY_WIRE_SIZE);
        assert_eq!(frame.payload.len(), 31);
        assert_eq!(frame.payload, record.login);
        assert_eq!(frame.payload[6..], [0xa5; 25]);
        assert_outer_header(&frame, 31);
        assert_eq!(LoginAlreadyRecord::decode(&frame.payload).unwrap(), record);
        let wire_frame = wire_round_trip(&frame);
        assert_eq!(wire_frame, frame);
        assert_eq!(
            decode_primary_response(&wire_frame).unwrap(),
            LoginByKeyDecodedResponse { handle, outcome }
        );
    }

    #[test]
    fn exact_sizes_and_source_headers_are_fixed() {
        assert_eq!(HEADER_DG_LOGIN_SUCCESS, 30);
        assert_eq!(HEADER_DG_LOGIN_NOT_EXIST, 31);
        assert_eq!(HEADER_DG_LOGIN_ALREADY, 34);
        assert_eq!(LoginByKeyRequest::WIRE_SIZE, 67);
        assert_eq!(LoginAccountRecord::WIRE_SIZE, 362);
        assert_eq!(LoginAlreadyRecord::WIRE_SIZE, 31);
        assert_eq!(LoginAlreadyRecord::packed_size(), 31);
        assert_eq!(HEADER_GD_LOGIN_BY_KEY, 101);
    }

    #[test]
    fn malformed_success_payload_lengths_are_rejected() {
        for length in [LOGIN_ACCOUNT_WIRE_SIZE - 1, LOGIN_ACCOUNT_WIRE_SIZE + 1] {
            let frame = DbFrame::new(HEADER_DG_LOGIN_SUCCESS, 7, vec![0; length]);
            assert!(matches!(
                decode_primary_response(&frame),
                Err(LoginByKeyResponseError::MalformedSuccessAccount { .. })
            ));
        }
    }

    #[test]
    fn malformed_missing_and_already_payloads_are_rejected() {
        let missing = DbFrame::new(HEADER_DG_LOGIN_NOT_EXIST, 8, vec![0]);
        assert_eq!(
            decode_primary_response(&missing),
            Err(LoginByKeyResponseError::MalformedMissingPayload { actual_len: 1 })
        );

        for length in [LOGIN_ALREADY_WIRE_SIZE - 1, LOGIN_ALREADY_WIRE_SIZE + 1] {
            let already = DbFrame::new(HEADER_DG_LOGIN_ALREADY, 9, vec![0; length]);
            assert!(matches!(
                decode_primary_response(&already),
                Err(LoginByKeyResponseError::MalformedAlreadyLoggedIn { .. })
            ));
        }
    }

    #[test]
    fn unsupported_or_uncertain_headers_are_not_inferred_as_missing() {
        for header in [
            0,
            HEADER_DG_LOGIN_WRONG_PASSWD,
            HEADER_GD_LOGIN_BY_KEY,
            0xab,
        ] {
            let frame = DbFrame::new(header, 10, Vec::new());
            assert_eq!(
                decode_primary_response(&frame),
                Err(LoginByKeyResponseError::UnsupportedHeader { header })
            );
        }
    }
}
