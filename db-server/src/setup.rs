//! Strict, SQL-free decoding of one legacy `HEADER_GD_SETUP` payload.
//!
//! This module is deliberately a decoder only. It does not retain peers or
//! logins, touch private shops, broadcast data, query SQL, or build a reply.
//! The transport must supply one complete DB payload after removing the
//! one-byte header and four-byte fields of the enclosing DB frame.
//!
//! Source evidence from the active legacy build:
//!
//! * `server/server/common/tables.h:345` starts `#pragma pack(1)` for DB
//!   records. `TPacketGDSetup` at lines 1008-1017 is 154 bytes on the active
//!   x86 profile: 16-byte `char`, `BYTE`, two `WORD`s, 32 four-byte `long`s,
//!   `DWORD`, and `BYTE`.
//! * `server/server/common/length.h:9,11,40` fixes the host and login/social
//!   byte arrays at 16, 31, and 19 bytes. `TPacketLoginOnSetup` at
//!   `tables.h:1404-1420` is therefore 100 bytes with the active feature
//!   macros. `server/server/common/prodomodefines.h:5,133,176` fixes
//!   `MAP_ALLOW_LIMIT` to 32 and enables both the language and premium private
//!   shop fields.
//! * `server/server/libthecore/typedef.h:15-18` defines `DWORD`, `BYTE`, and
//!   `WORD`. The legacy x86 `long` is four bytes. The active server writes
//!   these structures directly to its little-endian DB stream in
//!   `server/server/game/desc_client.cpp:147-223`; the receiver consumes the
//!   base and then exactly `dwLoginCount` records in
//!   `server/server/db/ClientManager.cpp:1284-1287,1434-1437`. The auth branch
//!   at `desc_client.cpp:217-220` writes only the zero-count base, so this
//!   decoder rejects login records on a nonzero `bAuthServer` request. That
//!   pairing check is deliberate Rust hardening: the legacy receiver returns
//!   from its auth branch before inspecting the declared count or tail.
//!
//! No packed Rust type is used. Each field is copied and decoded explicitly so
//! the active x86 wire profile does not depend on the Rust target's alignment,
//! endianness, or native `long` representation. The default decoder limit is
//! independent of any transport's outer frame cap; callers must enforce both
//! boundaries deliberately.

use std::error::Error;
use std::fmt;

pub use protocol::db_setup::{
    encode_login_on_setup, encode_setup_base, encode_setup_payload, LoginOnSetup, SetupBase,
    SetupEncodeError, CHANNEL_END, CLIENT_KEYS_END, HAS_PRIVATE_SHOP_END, HEADER_GD_SETUP,
    HOST_END, ID_END, LANGUAGE_END, LISTEN_PORT_END, LOGIN_COUNT_END, LOGIN_END, LOGIN_KEY_END,
    LOGIN_ON_SETUP_WIRE_SIZE, MAPS_END, P2P_PORT_END, PLAYER_HANDLE_END, PLAYER_ID_END,
    PUBLIC_IP_END, SETUP_BASE_WIRE_SIZE, SETUP_MAP_LIMIT, SOCIAL_ID_END,
};

/// Default cap on records allocated by [`decode_setup_request`].
///
/// At the fixed legacy width this bounds the decoded login vector and its
/// equivalent payload to about 6.25 MiB. Callers may choose a lower cap. This
/// is a receive-side policy, so it belongs here rather than in
/// `protocol::db_setup`, which holds only layout and the encoder.
pub const DEFAULT_MAX_LOGIN_RECORDS: usize = 65_536;

/// Largest complete payload width allowed by [`DEFAULT_MAX_LOGIN_RECORDS`].
pub const DEFAULT_MAX_SETUP_PAYLOAD_SIZE: usize =
    SETUP_BASE_WIRE_SIZE + DEFAULT_MAX_LOGIN_RECORDS * LOGIN_ON_SETUP_WIRE_SIZE;

/// Explicit feature and compiler profile for setup wire decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupFeatureProfile {
    /// The repository's active x86 build with language and premium private
    /// shop fields enabled, giving the 154-byte and 100-byte packet widths.
    ActiveX86,
}

/// Caller-selected resource limits applied before the login vector is filled.
///
/// The input slice is already owned by the transport. This limit bounds the
/// decoder's additional record allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupDecodeLimits {
    /// Maximum number of login records accepted and allocated.
    pub max_login_records: usize,
}

impl SetupDecodeLimits {
    /// Construct limits with the given maximum record count.
    #[must_use]
    pub const fn new(max_login_records: usize) -> Self {
        Self { max_login_records }
    }
}

impl Default for SetupDecodeLimits {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_LOGIN_RECORDS)
    }
}

/// A fully width-checked setup payload and all of its declared login records.
///
/// The decoder is the only constructor. Keeping the record collection and base
/// fields private prevents callers from manufacturing a typed request whose
/// declared count, vector length, and wire length disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupRequest {
    base: SetupBase,
    logins: Vec<LoginOnSetup>,
    payload_len: usize,
}

impl SetupRequest {
    /// Borrow the decoded `TPacketGDSetup` base record.
    #[must_use]
    pub const fn base(&self) -> &SetupBase {
        &self.base
    }

    /// Borrow exactly the declared `TPacketLoginOnSetup` records.
    #[must_use]
    pub fn logins(&self) -> &[LoginOnSetup] {
        &self.logins
    }

    /// Return the exact payload width represented by this validated request.
    #[must_use]
    pub const fn wire_len(&self) -> usize {
        self.payload_len
    }
}

/// A strict setup payload decoding failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupDecodeError {
    /// The payload has fewer bytes than its base and declared records require.
    InvalidWidth {
        /// Required complete payload width.
        expected: usize,
        /// Supplied payload width.
        actual: usize,
    },
    /// The payload has bytes beyond the width declared by `dwLoginCount`.
    TrailingBytes {
        /// Required complete payload width.
        expected: usize,
        /// Supplied payload width.
        actual: usize,
    },
    /// A nonzero auth-server flag was paired with an impossible login count.
    AuthServerMustBeBaseOnly {
        /// Untrusted count read from the base record.
        login_count: u32,
    },
    /// Conversion or checked size arithmetic could not represent the payload.
    SizeOverflow,
    /// The declared login count exceeds the caller-selected allocation cap.
    LoginCountLimitExceeded {
        /// Untrusted count read from the base record.
        login_count: u32,
        /// Caller-selected maximum.
        max_login_records: usize,
    },
    /// The bounded vector reservation failed.
    AllocationFailed {
        /// Number of records the decoder attempted to reserve.
        requested_login_records: usize,
    },
}

impl fmt::Display for SetupDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWidth { expected, actual } => write!(
                formatter,
                "setup payload has {actual} bytes; expected exactly {expected}"
            ),
            Self::TrailingBytes { expected, actual } => write!(
                formatter,
                "setup payload has {actual} bytes, more than expected width {expected}"
            ),
            Self::AuthServerMustBeBaseOnly { login_count } => write!(
                formatter,
                "auth-server setup must be base-only; declared {login_count} login records"
            ),
            Self::SizeOverflow => formatter.write_str("setup payload size arithmetic overflowed"),
            Self::LoginCountLimitExceeded {
                login_count,
                max_login_records,
            } => write!(
                formatter,
                "setup declares {login_count} login records; limit is {max_login_records}"
            ),
            Self::AllocationFailed {
                requested_login_records,
            } => write!(
                formatter,
                "could not allocate {requested_login_records} setup login records"
            ),
        }
    }
}

impl Error for SetupDecodeError {}

const _: () = assert!(PUBLIC_IP_END == 16);
const _: () = assert!(CHANNEL_END == 17);
const _: () = assert!(LISTEN_PORT_END == 19);
const _: () = assert!(P2P_PORT_END == 21);
const _: () = assert!(MAPS_END == 149);
const _: () = assert!(LOGIN_COUNT_END == 153);
const _: () = assert!(LOGIN_COUNT_END + 1 == SETUP_BASE_WIRE_SIZE);
const _: () = assert!(ID_END == 4);
const _: () = assert!(LOGIN_END == 35);
const _: () = assert!(SOCIAL_ID_END == 54);
const _: () = assert!(HOST_END == 70);
const _: () = assert!(LOGIN_KEY_END == 74);
const _: () = assert!(CLIENT_KEYS_END == 90);
const _: () = assert!(LANGUAGE_END == 91);
const _: () = assert!(PLAYER_ID_END == 95);
const _: () = assert!(PLAYER_HANDLE_END == 99);
const _: () = assert!(HAS_PRIVATE_SHOP_END == LOGIN_ON_SETUP_WIRE_SIZE);

fn checked_payload_len(login_count: usize) -> Result<usize, SetupDecodeError> {
    let record_bytes = login_count
        .checked_mul(LOGIN_ON_SETUP_WIRE_SIZE)
        .ok_or(SetupDecodeError::SizeOverflow)?;
    SETUP_BASE_WIRE_SIZE
        .checked_add(record_bytes)
        .ok_or(SetupDecodeError::SizeOverflow)
}

fn copy_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut output = [0; N];
    output.copy_from_slice(bytes);
    output
}

fn read_u16_le(bytes: &[u8]) -> u16 {
    let bytes: [u8; 2] = copy_array(bytes);
    u16::from_le_bytes(bytes)
}

fn read_u32_le(bytes: &[u8]) -> u32 {
    let bytes: [u8; 4] = copy_array(bytes);
    u32::from_le_bytes(bytes)
}

fn read_i32_le(bytes: &[u8]) -> i32 {
    let bytes: [u8; 4] = copy_array(bytes);
    i32::from_le_bytes(bytes)
}

fn decode_base(payload: &[u8]) -> Result<SetupBase, SetupDecodeError> {
    if payload.len() < SETUP_BASE_WIRE_SIZE {
        return Err(SetupDecodeError::InvalidWidth {
            expected: SETUP_BASE_WIRE_SIZE,
            actual: payload.len(),
        });
    }

    let base = &payload[..SETUP_BASE_WIRE_SIZE];

    let mut maps = [0; SETUP_MAP_LIMIT];
    for (index, map) in maps.iter_mut().enumerate() {
        let start = P2P_PORT_END + index * 4;
        *map = read_i32_le(&base[start..start + 4]);
    }

    Ok(SetupBase {
        public_ip: copy_array(&base[..PUBLIC_IP_END]),
        channel: base[PUBLIC_IP_END],
        listen_port: read_u16_le(&base[CHANNEL_END..LISTEN_PORT_END]),
        p2p_port: read_u16_le(&base[LISTEN_PORT_END..P2P_PORT_END]),
        maps,
        login_count: read_u32_le(&base[MAPS_END..LOGIN_COUNT_END]),
        auth_server: base[LOGIN_COUNT_END],
    })
}

fn decode_login_record(bytes: &[u8]) -> LoginOnSetup {
    debug_assert_eq!(HAS_PRIVATE_SHOP_END, bytes.len());

    let mut client_keys = [0; 4];
    for (index, key) in client_keys.iter_mut().enumerate() {
        let start = LOGIN_KEY_END + index * 4;
        *key = read_u32_le(&bytes[start..start + 4]);
    }

    LoginOnSetup {
        id: read_u32_le(&bytes[..ID_END]),
        login: copy_array(&bytes[ID_END..LOGIN_END]),
        social_id: copy_array(&bytes[LOGIN_END..SOCIAL_ID_END]),
        host: copy_array(&bytes[SOCIAL_ID_END..HOST_END]),
        login_key: read_u32_le(&bytes[HOST_END..LOGIN_KEY_END]),
        client_keys,
        language: bytes[CLIENT_KEYS_END],
        player_id: read_u32_le(&bytes[LANGUAGE_END..PLAYER_ID_END]),
        player_handle: read_u32_le(&bytes[PLAYER_ID_END..PLAYER_HANDLE_END]),
        has_private_shop_raw: bytes[PLAYER_HANDLE_END],
    }
}

/// Decode one complete active-x86 game-to-DB setup payload.
///
/// The payload must have exactly
/// `154 + 100 * TPacketGDSetup::dwLoginCount` bytes. A nonzero auth-server
/// byte additionally requires a zero count, matching the active sender. The
/// base width, count conversion, record-byte multiplication, and final addition
/// are checked. The record limit is checked before a vector is reserved, and
/// the reservation is fallible. No trailing byte is ignored.
///
/// # Errors
///
/// Returns [`SetupDecodeError::InvalidWidth`] for a short base or missing
/// declared record bytes, [`SetupDecodeError::TrailingBytes`] for extra bytes,
/// [`SetupDecodeError::AuthServerMustBeBaseOnly`] for a nonzero auth flag with
/// declared logins, [`SetupDecodeError::SizeOverflow`] when size arithmetic
/// cannot be represented, [`SetupDecodeError::LoginCountLimitExceeded`] before
/// any login allocation, or [`SetupDecodeError::AllocationFailed`] if
/// reservation fails.
pub fn decode_setup_request(
    payload: &[u8],
    profile: SetupFeatureProfile,
    limits: SetupDecodeLimits,
) -> Result<SetupRequest, SetupDecodeError> {
    match profile {
        SetupFeatureProfile::ActiveX86 => {}
    }

    let base = decode_base(payload)?;
    let login_count =
        usize::try_from(base.login_count).map_err(|_| SetupDecodeError::SizeOverflow)?;
    let expected_len = checked_payload_len(login_count)?;

    if login_count > limits.max_login_records {
        return Err(SetupDecodeError::LoginCountLimitExceeded {
            login_count: base.login_count,
            max_login_records: limits.max_login_records,
        });
    }

    match payload.len().cmp(&expected_len) {
        std::cmp::Ordering::Less => {
            return Err(SetupDecodeError::InvalidWidth {
                expected: expected_len,
                actual: payload.len(),
            });
        }
        std::cmp::Ordering::Greater => {
            return Err(SetupDecodeError::TrailingBytes {
                expected: expected_len,
                actual: payload.len(),
            });
        }
        std::cmp::Ordering::Equal => {}
    }

    if base.auth_server != 0 && base.login_count != 0 {
        return Err(SetupDecodeError::AuthServerMustBeBaseOnly {
            login_count: base.login_count,
        });
    }

    let mut logins = Vec::new();
    logins
        .try_reserve_exact(login_count)
        .map_err(|_| SetupDecodeError::AllocationFailed {
            requested_login_records: login_count,
        })?;

    for bytes in payload[SETUP_BASE_WIRE_SIZE..].chunks_exact(LOGIN_ON_SETUP_WIRE_SIZE) {
        logins.push(decode_login_record(bytes));
    }
    debug_assert_eq!(logins.len(), login_count);

    Ok(SetupRequest {
        base,
        logins,
        payload_len: expected_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::db_wire::{DbFrame, DbFrameDecoder};

    fn filled<const N: usize>(start: u8) -> [u8; N] {
        std::array::from_fn(|index| {
            start.wrapping_add(u8::try_from(index).expect("small fixture index"))
        })
    }

    fn expected_base(login_count: u32, auth_server: u8) -> SetupBase {
        SetupBase {
            public_ip: filled(0x10),
            channel: 7,
            listen_port: 0x1234,
            p2p_port: 0x5678,
            maps: std::array::from_fn(|index| {
                -i32::try_from(index).expect("small map index") - 100
            }),
            login_count,
            auth_server,
        }
    }

    fn base_bytes(login_count: u32, auth_server: u8) -> Vec<u8> {
        let base = expected_base(login_count, auth_server);
        let mut bytes = Vec::with_capacity(SETUP_BASE_WIRE_SIZE);
        bytes.extend_from_slice(&base.public_ip);
        bytes.push(base.channel);
        bytes.extend_from_slice(&base.listen_port.to_le_bytes());
        bytes.extend_from_slice(&base.p2p_port.to_le_bytes());
        for map in base.maps {
            bytes.extend_from_slice(&map.to_le_bytes());
        }
        bytes.extend_from_slice(&base.login_count.to_le_bytes());
        bytes.push(base.auth_server);
        assert_eq!(bytes.len(), SETUP_BASE_WIRE_SIZE);
        bytes
    }

    fn expected_login_record(seed: u8) -> LoginOnSetup {
        LoginOnSetup {
            id: 0x1122_3344_u32.wrapping_add(u32::from(seed)),
            login: filled(0x40_u8.wrapping_add(seed)),
            social_id: filled(0x60_u8.wrapping_add(seed)),
            host: filled(0x80_u8.wrapping_add(seed)),
            login_key: 0xaabb_ccdd_u32.wrapping_add(u32::from(seed)),
            client_keys: [
                0x0102_0304,
                0x1112_1314_u32.wrapping_add(u32::from(seed)),
                0x2122_2324_u32.wrapping_add(u32::from(seed)),
                0x3132_3334_u32.wrapping_add(u32::from(seed)),
            ],
            language: 9,
            player_id: 0x5566_7788_u32.wrapping_add(u32::from(seed)),
            player_handle: 0x99aa_bbcc_u32.wrapping_add(u32::from(seed)),
            has_private_shop_raw: seed,
        }
    }

    fn login_bytes(seed: u8) -> Vec<u8> {
        let record = expected_login_record(seed);
        let mut bytes = Vec::with_capacity(LOGIN_ON_SETUP_WIRE_SIZE);
        bytes.extend_from_slice(&record.id.to_le_bytes());
        bytes.extend_from_slice(&record.login);
        bytes.extend_from_slice(&record.social_id);
        bytes.extend_from_slice(&record.host);
        bytes.extend_from_slice(&record.login_key.to_le_bytes());
        for key in record.client_keys {
            bytes.extend_from_slice(&key.to_le_bytes());
        }
        bytes.push(record.language);
        bytes.extend_from_slice(&record.player_id.to_le_bytes());
        bytes.extend_from_slice(&record.player_handle.to_le_bytes());
        bytes.push(record.has_private_shop_raw);
        assert_eq!(bytes.len(), LOGIN_ON_SETUP_WIRE_SIZE);
        bytes
    }

    fn setup_payload(login_count: u32) -> Vec<u8> {
        let mut payload = base_bytes(login_count, 0);
        for index in 0..login_count {
            payload.extend_from_slice(&login_bytes(
                u8::try_from(index).expect("small login index"),
            ));
        }
        payload
    }

    fn limits_for(count: usize) -> SetupDecodeLimits {
        SetupDecodeLimits::new(count)
    }

    #[test]
    fn decodes_zero_record_normal_and_auth_base_payloads() {
        for auth_server in [0, 1, 0xa5] {
            let payload = base_bytes(0, auth_server);
            let decoded =
                decode_setup_request(&payload, SetupFeatureProfile::ActiveX86, limits_for(0))
                    .expect("zero-record payload is valid");

            assert_eq!(
                decoded,
                SetupRequest {
                    base: expected_base(0, auth_server),
                    logins: Vec::new(),
                    payload_len: SETUP_BASE_WIRE_SIZE,
                }
            );
            assert_eq!(decoded.wire_len(), SETUP_BASE_WIRE_SIZE);
        }
    }

    #[test]
    fn rejects_login_records_on_an_auth_server_setup() {
        let mut payload = base_bytes(1, 1);
        payload.extend_from_slice(&login_bytes(0));

        assert_eq!(
            decode_setup_request(&payload, SetupFeatureProfile::ActiveX86, limits_for(1),),
            Err(SetupDecodeError::AuthServerMustBeBaseOnly { login_count: 1 })
        );
    }

    #[test]
    fn decodes_one_and_multiple_login_records() {
        let mut one = base_bytes(1, 0);
        one.extend_from_slice(&login_bytes(0x80));
        let decoded_one = decode_setup_request(&one, SetupFeatureProfile::ActiveX86, limits_for(1))
            .expect("one record is valid");
        assert_eq!(
            decoded_one,
            SetupRequest {
                base: expected_base(1, 0),
                logins: vec![expected_login_record(0x80)],
                payload_len: SETUP_BASE_WIRE_SIZE + LOGIN_ON_SETUP_WIRE_SIZE,
            }
        );
        assert!(decoded_one.logins[0].has_private_shop());

        let multiple = setup_payload(3);
        let decoded =
            decode_setup_request(&multiple, SetupFeatureProfile::ActiveX86, limits_for(3))
                .expect("three records are valid");
        assert_eq!(
            decoded.logins,
            vec![
                expected_login_record(0),
                expected_login_record(1),
                expected_login_record(2),
            ]
        );
        assert!(!decoded.logins[0].has_private_shop());
        assert!(decoded.logins[1].has_private_shop());
    }

    #[test]
    fn decodes_raw_fields_after_real_db_frame_fragmentation() {
        let direct = setup_payload(2);
        let expected = decode_setup_request(&direct, SetupFeatureProfile::ActiveX86, limits_for(2))
            .expect("direct payload is valid");
        let encoded = DbFrame::new(HEADER_GD_SETUP, 0, direct.clone())
            .encode()
            .expect("setup frame fits the DB header");

        let mut frame_decoder = DbFrameDecoder::with_max_payload_size(direct.len());
        for end in 1..encoded.len() {
            frame_decoder.feed(&encoded[end - 1..end]).unwrap();
            assert_eq!(frame_decoder.try_decode().unwrap(), None);
        }
        frame_decoder.feed(&encoded[encoded.len() - 1..]).unwrap();
        let frame = frame_decoder
            .try_decode()
            .unwrap()
            .expect("final byte completes the DB frame");
        assert_eq!(frame.header, HEADER_GD_SETUP);
        assert_eq!(frame.handle, 0);
        assert_eq!(frame_decoder.buffered_len(), 0);
        assert_eq!(
            decode_setup_request(
                &frame.payload,
                SetupFeatureProfile::ActiveX86,
                limits_for(2),
            )
            .expect("reassembled setup payload is valid"),
            expected
        );

        let mut prefixed = Vec::with_capacity(frame.payload.len() + 1);
        prefixed.push(0xa5);
        prefixed.extend_from_slice(&frame.payload);
        let decoded_from_unaligned = decode_setup_request(
            &prefixed[1..],
            SetupFeatureProfile::ActiveX86,
            limits_for(2),
        )
        .expect("raw fields do not require aligned storage");
        assert_eq!(decoded_from_unaligned, expected);
        assert_eq!(decoded_from_unaligned.logins[0].login, filled(0x40));
        assert_eq!(decoded_from_unaligned.logins[1].host, filled(0x81));
    }

    #[test]
    fn rejects_short_base_and_missing_record_widths() {
        let short_base = &base_bytes(0, 0)[..SETUP_BASE_WIRE_SIZE - 1];
        assert_eq!(
            decode_setup_request(
                short_base,
                SetupFeatureProfile::ActiveX86,
                SetupDecodeLimits::default(),
            ),
            Err(SetupDecodeError::InvalidWidth {
                expected: SETUP_BASE_WIRE_SIZE,
                actual: SETUP_BASE_WIRE_SIZE - 1,
            })
        );

        let missing_record = base_bytes(1, 0);
        assert_eq!(
            decode_setup_request(
                &missing_record,
                SetupFeatureProfile::ActiveX86,
                SetupDecodeLimits::default(),
            ),
            Err(SetupDecodeError::InvalidWidth {
                expected: SETUP_BASE_WIRE_SIZE + LOGIN_ON_SETUP_WIRE_SIZE,
                actual: SETUP_BASE_WIRE_SIZE,
            })
        );
    }

    #[test]
    fn rejects_trailing_bytes_after_base_and_records() {
        let mut after_base = base_bytes(0, 0);
        after_base.push(0x5a);
        assert_eq!(
            decode_setup_request(
                &after_base,
                SetupFeatureProfile::ActiveX86,
                SetupDecodeLimits::default(),
            ),
            Err(SetupDecodeError::TrailingBytes {
                expected: SETUP_BASE_WIRE_SIZE,
                actual: SETUP_BASE_WIRE_SIZE + 1,
            })
        );

        let mut after_record = setup_payload(1);
        after_record.extend_from_slice(&[0xa5, 0x5a]);
        assert_eq!(
            decode_setup_request(
                &after_record,
                SetupFeatureProfile::ActiveX86,
                SetupDecodeLimits::default(),
            ),
            Err(SetupDecodeError::TrailingBytes {
                expected: SETUP_BASE_WIRE_SIZE + LOGIN_ON_SETUP_WIRE_SIZE,
                actual: SETUP_BASE_WIRE_SIZE + LOGIN_ON_SETUP_WIRE_SIZE + 2,
            })
        );
    }

    #[test]
    fn checked_size_arithmetic_reports_overflow_without_allocating() {
        assert_eq!(
            checked_payload_len(usize::MAX),
            Err(SetupDecodeError::SizeOverflow)
        );
    }

    #[test]
    fn exact_record_limit_is_inclusive_and_rejects_one_more() {
        let zero = base_bytes(0, 0);
        assert!(decode_setup_request(&zero, SetupFeatureProfile::ActiveX86, limits_for(0)).is_ok());

        let one = setup_payload(1);
        assert_eq!(
            decode_setup_request(&one, SetupFeatureProfile::ActiveX86, limits_for(0)),
            Err(SetupDecodeError::LoginCountLimitExceeded {
                login_count: 1,
                max_login_records: 0,
            })
        );
        assert!(decode_setup_request(&one, SetupFeatureProfile::ActiveX86, limits_for(1)).is_ok());

        let two = setup_payload(2);
        assert_eq!(
            decode_setup_request(&two, SetupFeatureProfile::ActiveX86, limits_for(1)),
            Err(SetupDecodeError::LoginCountLimitExceeded {
                login_count: 2,
                max_login_records: 1,
            })
        );
        assert_eq!(DEFAULT_MAX_SETUP_PAYLOAD_SIZE, 6_553_754);
        assert_eq!(SetupDecodeLimits::default().max_login_records, 65_536);
    }
}
