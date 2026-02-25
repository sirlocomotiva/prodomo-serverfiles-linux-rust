//! The game-to-DB `HEADER_GD_SETUP` record: shared fields, exact offsets, and
//! the encoder the game server needs.
//!
//! This module owns the record itself, because **both** binaries need it. The
//! game server builds a setup request; the DB server decodes one. Before this
//! module the record lived only in `db-server/src/setup.rs`, which made it
//! unreachable from the game server without giving the game binary a dependency
//! on the DB binary's crate.
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
//! * `server/server/game/desc_client.cpp:147-223` writes both records into one
//!   `TEMP_BUFFER` and sends the result as a single `HEADER_GD_SETUP` frame
//!   with handle 0.
//!
//! The decoder, the feature profile, the record cap, and the
//! "auth-mode must be base only" receive policy stay in
//! `db-server/src/setup.rs`. They are DB-peer receive policy, not wire layout.
//! This module holds only the layout both directions agree on, plus the
//! encoder.
//!
//! No packed Rust type is used. Every field is copied and encoded explicitly so
//! the active x86 wire profile does not depend on the Rust target's alignment,
//! endianness, or native `long` representation.

use std::error::Error;
use std::fmt;

/// One active-x86 `TPacketGDSetup` base record.
///
/// Every character array is raw storage. `auth_server` is a raw byte with no
/// interpretation at this layer: choosing the auth branch is a policy decision
/// for the DB side, never a credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupBase {
    /// Raw 16-byte `szPublicIP` field.
    pub public_ip: [u8; 16],
    /// Raw `bChannel` field.
    pub channel: u8,
    /// Little-endian `wListenPort` field.
    pub listen_port: u16,
    /// Little-endian `wP2PPort` field.
    pub p2p_port: u16,
    /// Thirty-two little-endian signed 32-bit `alMaps` values.
    pub maps: [i32; SETUP_MAP_LIMIT],
    /// Unsigned little-endian `dwLoginCount` field.
    pub login_count: u32,
    /// Raw `bAuthServer` byte. No service-level interpretation is performed.
    pub auth_server: u8,
}

/// One-byte legacy header for a game-to-DB setup request.
///
/// This is the game-to-DB direction. `HEADER_DG_P2P` also equals `0xff` but
/// belongs to the opposite direction; the two must never be merged.
pub const HEADER_GD_SETUP: u8 = 0xff;

/// Exact packed active-x86 width of `TPacketGDSetup`.
pub const SETUP_BASE_WIRE_SIZE: usize = 154;

/// Exact packed active-x86 width of `TPacketLoginOnSetup`.
pub const LOGIN_ON_SETUP_WIRE_SIZE: usize = 100;

/// Number of `long` map slots in the active legacy setup packet.
///
/// `server/server/common/prodomodefines.h:5` fixes this at 32. Legacy
/// `map_allow_copy` writes `MAP_ALLOW_LIMIT` entries, and a 33rd entry would
/// overwrite `dwLoginCount` in the packed record.
pub const SETUP_MAP_LIMIT: usize = 32;

/// One active-x86 `TPacketLoginOnSetup` record.
///
/// All character arrays preserve every byte from the record, including
/// embedded NULs and high bytes. The final byte stays a `u8`, not a Rust
/// `bool`, so even a non-canonical legacy boolean byte is retained exactly.
///
/// All character arrays preserve every byte from the record. The final byte
/// remains a `u8`, rather than a Rust `bool`, so even a non-canonical legacy
/// boolean byte is retained exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginOnSetup {
    /// Unsigned little-endian `dwID` field.
    pub id: u32,
    /// Raw 31-byte `szLogin` field.
    pub login: [u8; 31],
    /// Raw 19-byte `szSocialID` field.
    pub social_id: [u8; 19],
    /// Raw 16-byte `szHost` field.
    pub host: [u8; 16],
    /// Unsigned little-endian `dwLoginKey` field.
    pub login_key: u32,
    /// Four unsigned little-endian `adwClientKey` values.
    pub client_keys: [u32; 4],
    /// Raw `bLanguage` field from the enabled language feature.
    pub language: u8,
    /// Unsigned little-endian `dwPID` field.
    pub player_id: u32,
    /// Unsigned little-endian `dwHandle` field.
    pub player_handle: u32,
    /// Raw one-byte `bHasPrivateShop` value.
    pub has_private_shop_raw: u8,
}

/// End offset of `szPublicIP`, the raw 16-byte public-IP field.
pub const PUBLIC_IP_END: usize = 16;
/// End offset of `bChannel`, the raw channel byte.
pub const CHANNEL_END: usize = PUBLIC_IP_END + 1;
/// End offset of `wListenPort`, the little-endian client listen port.
pub const LISTEN_PORT_END: usize = CHANNEL_END + 2;
/// End offset of `wP2PPort`, the little-endian P2P port.
pub const P2P_PORT_END: usize = LISTEN_PORT_END + 2;
/// End offset of `alMaps`, the 32 signed little-endian map indices.
pub const MAPS_END: usize = P2P_PORT_END + SETUP_MAP_LIMIT * 4;
/// End offset of `dwLoginCount`. The next byte is `bAuthServer`.
pub const LOGIN_COUNT_END: usize = MAPS_END + 4;

/// Offsets inside the 100-byte `TPacketLoginOnSetup` record.
/// End offset of `dwID`, the account id.
pub const ID_END: usize = 4;
/// End offset of `szLogin`, the raw 31-byte login name.
pub const LOGIN_END: usize = ID_END + 31;
/// End offset of `szSocialID`, the raw 19-byte social id.
pub const SOCIAL_ID_END: usize = LOGIN_END + 19;
/// End offset of `szHost`, the raw 16-byte host string.
pub const HOST_END: usize = SOCIAL_ID_END + 16;
/// End offset of `dwLoginKey`.
pub const LOGIN_KEY_END: usize = HOST_END + 4;
/// End offset of `adwClientKey`, the four little-endian client-key words.
pub const CLIENT_KEYS_END: usize = LOGIN_KEY_END + 4 * 4;
/// End offset of `bLanguage`.
pub const LANGUAGE_END: usize = CLIENT_KEYS_END + 1;
/// End offset of `dwPID`, the player id.
pub const PLAYER_ID_END: usize = LANGUAGE_END + 4;
/// End offset of `dwHandle`, the descriptor handle.
pub const PLAYER_HANDLE_END: usize = PLAYER_ID_END + 4;
/// End offset of `bHasPrivateShop`, the final raw byte.
pub const HAS_PRIVATE_SHOP_END: usize = PLAYER_HANDLE_END + 1;
impl LoginOnSetup {
    /// Interpret the preserved legacy byte using the C++ truth test.
    ///
    /// The raw byte is still available as
    /// [`LoginOnSetup::has_private_shop_raw`], so a non-canonical legacy value
    /// such as `2` is not lost by reading the record through this method.
    #[must_use]
    pub const fn has_private_shop(&self) -> bool {
        self.has_private_shop_raw != 0
    }
}

/// A game-to-DB setup payload encoding failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupEncodeError {
    /// The login record count does not fit the 32-bit `dwLoginCount` field.
    ///
    /// The count is derived from the record slice, so this is only reachable
    /// with a slice longer than 4,294,967,295 records. It stays a typed error
    /// rather than a silent truncation, matching the decoder's strictness.
    LoginCountOverflow {
        /// Number of records the caller supplied.
        login_count: usize,
    },
    /// The encoded payload does not fit the 32-bit DB frame length field.
    PayloadTooLarge {
        /// Byte length that was refused.
        payload_len: usize,
        /// Largest payload the length field can describe.
        max_payload_len: usize,
    },
    /// An auth-mode setup may not carry login records.
    ///
    /// Legacy writes a base-only record in the auth branch. Encoding a
    /// non-empty record list with a nonzero auth byte would produce a payload
    /// that the DB-side decoder rejects.
    AuthServerMustBeBaseOnly {
        /// Number of records the caller supplied.
        login_count: usize,
    },
}

impl fmt::Display for SetupEncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LoginCountOverflow { login_count } => write!(
                formatter,
                "setup login count {login_count} does not fit the 32-bit dwLoginCount field"
            ),
            Self::PayloadTooLarge {
                payload_len,
                max_payload_len,
            } => write!(
                formatter,
                "setup payload of {payload_len} bytes exceeds the {max_payload_len}-byte frame length field"
            ),
            Self::AuthServerMustBeBaseOnly { login_count } => write!(
                formatter,
                "an auth-mode setup must be base only, but {login_count} login records were supplied"
            ),
        }
    }
}

impl Error for SetupEncodeError {}

/// Return the exact payload width for `login_count` login records.
///
/// The DB frame length field is 32 bits, so the encoder refuses a payload it
/// could not describe. The decoder needs no such limit: it reads a length that
/// already arrived.
fn base_payload_len(login_count: usize) -> Result<usize, SetupEncodeError> {
    let max_payload_len = u32::MAX as usize;
    let record_bytes = login_count.checked_mul(LOGIN_ON_SETUP_WIRE_SIZE).ok_or(
        SetupEncodeError::PayloadTooLarge {
            payload_len: usize::MAX,
            max_payload_len,
        },
    )?;
    SETUP_BASE_WIRE_SIZE
        .checked_add(record_bytes)
        .filter(|length| *length <= max_payload_len)
        .ok_or(SetupEncodeError::PayloadTooLarge {
            payload_len: usize::MAX,
            max_payload_len,
        })
}

/// Encode one 154-byte `TPacketGDSetup` base record.
///
/// The offsets are the same named constants the DB-side decoder uses, so the
/// two directions cannot drift apart silently.
///
/// `login_count` is written exactly as supplied. Prefer
/// [`encode_setup_payload`], which derives the count from the records it was
/// given, because a caller that supplies a count disagreeing with its own
/// record list produces a payload the peer will reject.
///
/// # Errors
///
/// Returns [`SetupEncodeError::AuthServerMustBeBaseOnly`] when a nonzero auth
/// byte is combined with a nonzero count.
pub fn encode_setup_base(
    base: &SetupBase,
    login_count: u32,
) -> Result<[u8; SETUP_BASE_WIRE_SIZE], SetupEncodeError> {
    if base.auth_server != 0 && login_count != 0 {
        return Err(SetupEncodeError::AuthServerMustBeBaseOnly {
            login_count: usize::try_from(login_count).unwrap_or(usize::MAX),
        });
    }

    let mut bytes = [0_u8; SETUP_BASE_WIRE_SIZE];
    bytes[..PUBLIC_IP_END].copy_from_slice(&base.public_ip);
    bytes[PUBLIC_IP_END..CHANNEL_END].copy_from_slice(&base.channel.to_le_bytes());
    bytes[CHANNEL_END..LISTEN_PORT_END].copy_from_slice(&base.listen_port.to_le_bytes());
    bytes[LISTEN_PORT_END..P2P_PORT_END].copy_from_slice(&base.p2p_port.to_le_bytes());
    for (index, map) in base.maps.iter().enumerate() {
        let start = P2P_PORT_END + index * 4;
        bytes[start..start + 4].copy_from_slice(&map.to_le_bytes());
    }
    bytes[MAPS_END..LOGIN_COUNT_END].copy_from_slice(&login_count.to_le_bytes());
    bytes[LOGIN_COUNT_END] = base.auth_server;
    Ok(bytes)
}

/// Encode one 100-byte `TPacketLoginOnSetup` record.
///
/// The offsets are the shared decoder constants, and every character array is
/// copied byte for byte, including embedded NULs and high bytes.
#[must_use]
pub fn encode_login_on_setup(record: &LoginOnSetup) -> [u8; LOGIN_ON_SETUP_WIRE_SIZE] {
    let mut bytes = [0_u8; LOGIN_ON_SETUP_WIRE_SIZE];
    bytes[..ID_END].copy_from_slice(&record.id.to_le_bytes());
    bytes[ID_END..LOGIN_END].copy_from_slice(&record.login);
    bytes[LOGIN_END..SOCIAL_ID_END].copy_from_slice(&record.social_id);
    bytes[SOCIAL_ID_END..HOST_END].copy_from_slice(&record.host);
    bytes[HOST_END..LOGIN_KEY_END].copy_from_slice(&record.login_key.to_le_bytes());
    for (index, key) in record.client_keys.iter().enumerate() {
        let start = LOGIN_KEY_END + index * 4;
        bytes[start..start + 4].copy_from_slice(&key.to_le_bytes());
    }
    bytes[CLIENT_KEYS_END..LANGUAGE_END].copy_from_slice(&[record.language]);
    bytes[LANGUAGE_END..PLAYER_ID_END].copy_from_slice(&record.player_id.to_le_bytes());
    bytes[PLAYER_ID_END..PLAYER_HANDLE_END].copy_from_slice(&record.player_handle.to_le_bytes());
    bytes[PLAYER_HANDLE_END..HAS_PRIVATE_SHOP_END].copy_from_slice(&[record.has_private_shop_raw]);
    bytes
}

/// Encode a complete game-to-DB setup payload.
///
/// `dwLoginCount` is derived from `logins`, never taken from a caller-supplied
/// field, so the declared count and the encoded length cannot disagree. The
/// base record's own `login_count` field is ignored: the count written is the
/// length of `logins`.
///
/// Legacy `desc_client.cpp:167-215` counts the connected descriptors with a
/// non-zero account id, writes that count, then appends exactly that many
/// records. Deriving the count from the records reproduces the shape without
/// the possibility of the two disagreeing.
///
/// # Errors
///
/// Returns [`SetupEncodeError::LoginCountOverflow`] when the record count does
/// not fit `u32`, [`SetupEncodeError::AuthServerMustBeBaseOnly`] for a
/// non-empty record list with a nonzero auth byte, and
/// [`SetupEncodeError::PayloadTooLarge`] when the payload does not fit the
/// 32-bit DB frame length field.
pub fn encode_setup_payload(
    base: &SetupBase,
    logins: &[LoginOnSetup],
) -> Result<Vec<u8>, SetupEncodeError> {
    let login_count =
        u32::try_from(logins.len()).map_err(|_| SetupEncodeError::LoginCountOverflow {
            login_count: logins.len(),
        })?;
    if base.auth_server != 0 && !logins.is_empty() {
        return Err(SetupEncodeError::AuthServerMustBeBaseOnly {
            login_count: logins.len(),
        });
    }

    let payload_len = base_payload_len(logins.len())?;
    let mut payload = Vec::with_capacity(payload_len);
    payload.extend_from_slice(&encode_setup_base(base, login_count)?);
    for record in logins {
        payload.extend_from_slice(&encode_login_on_setup(record));
    }
    debug_assert_eq!(payload.len(), payload_len);
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_wire::DbFrameDecoder;

    /// Fill a byte array with a distinct value per position.
    ///
    /// Distinct values per byte catch a field-order mistake that a repeated
    /// value would hide.
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

    /// The 154 base bytes, written field by field from the legacy declaration
    /// order in `tables.h:1008-1017`.
    ///
    /// This deliberately does not call the encoder, so agreement between the
    /// two is evidence about the offsets rather than self-consistency.
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

    /// The 100 record bytes, written field by field from `tables.h:1404-1420`.
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

    #[test]
    fn the_base_encoder_matches_the_hand_written_legacy_field_order() {
        for (login_count, auth_server) in [(0_u32, 0_u8), (0, 1), (3, 0)] {
            let base = expected_base(login_count, auth_server);
            assert_eq!(
                encode_setup_base(&base, login_count).expect("a valid base encodes"),
                base_bytes(login_count, auth_server).as_slice(),
                "login_count={login_count} auth_server={auth_server}"
            );
        }
    }

    #[test]
    fn the_login_encoder_matches_the_hand_written_legacy_field_order() {
        for seed in [0_u8, 1, 0x7f, 0x80, 0xff] {
            let record = expected_login_record(seed);
            assert_eq!(
                encode_login_on_setup(&record).as_slice(),
                login_bytes(seed).as_slice(),
                "seed={seed}"
            );
        }
    }

    #[test]
    fn the_login_encoder_preserves_high_bytes_and_embedded_nuls() {
        // Each of these bytes is a valid raw character-array element in the
        // legacy records. A text round trip would corrupt all of them.
        let mut record = expected_login_record(0);
        record.login = [
            0xa1, 0x00, 0xff, 0x00, 0x80, 0x7f, 0xc3, 0x9c, 0xed, 0x00, 0xfe, 0x01, 0x81, 0xbf,
            0xf4, 0x8f, 0xbe, 0x80, 0xbd, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xaa, 0xbb,
        ];
        record.social_id = [0x00; 19];
        record.social_id[18] = 0xfe;
        record.host = [b'2'; 16];
        let bytes = encode_login_on_setup(&record);
        assert_eq!(&bytes[4..35], &record.login, "every login byte survives");
        assert_eq!(&bytes[35..54], &record.social_id);
        assert_eq!(&bytes[54..70], &record.host);
    }

    #[test]
    fn the_payload_encoder_derives_the_count_instead_of_trusting_the_base_field() {
        // The base's own `login_count` says 9 while three records follow. The
        // encoder must write 3: a count that disagreed with the record list
        // would produce a payload the DB-side decoder rejects on width.
        let base = expected_base(9, 0);
        let logins: Vec<LoginOnSetup> = (0..3)
            .map(|index| expected_login_record(u8::try_from(index).expect("small login index")))
            .collect();
        let payload = encode_setup_payload(&base, &logins).expect("composition succeeds");
        assert_eq!(
            payload,
            setup_payload(3),
            "the encoded bytes match the hand-built legacy payload"
        );
        assert_eq!(
            &payload[MAPS_END..LOGIN_COUNT_END],
            &3_u32.to_le_bytes(),
            "the wire count is the record count, not the base field"
        );
    }

    #[test]
    fn an_auth_mode_payload_with_login_records_is_refused_by_the_encoder() {
        let base = expected_base(1, 1);
        let logins = [expected_login_record(0)];
        assert_eq!(
            encode_setup_payload(&base, &logins),
            Err(SetupEncodeError::AuthServerMustBeBaseOnly { login_count: 1 })
        );
        assert_eq!(
            encode_setup_base(&base, 1),
            Err(SetupEncodeError::AuthServerMustBeBaseOnly { login_count: 1 })
        );
        // A base-only auth payload is exactly what the legacy auth branch sends
        // at desc_client.cpp:217-220.
        assert_eq!(
            encode_setup_payload(&base, &[]).expect("base only is valid"),
            base_bytes(0, 1)
        );
    }

    #[test]
    fn an_empty_login_list_still_encodes_the_whole_base_record() {
        let payload = encode_setup_payload(&expected_base(0, 0), &[])
            .expect("a base-only game setup encodes");
        assert_eq!(payload.len(), SETUP_BASE_WIRE_SIZE);
        assert_eq!(&payload[MAPS_END..LOGIN_COUNT_END], &0_u32.to_le_bytes());
        assert_eq!(payload[LOGIN_COUNT_END], 0, "bAuthServer is 0");
    }

    #[test]
    fn a_fresh_game_setup_never_claims_a_login_count() {
        // The Rust game server has no client descriptors yet, so the count must
        // be zero. A nonzero count with no records is exactly the disagreement
        // the decoder rejects, so this is a real assertion, not a formality.
        let payload =
            encode_setup_payload(&expected_base(0, 0), &[]).expect("a fresh setup encodes");
        let count = u32::from_le_bytes([
            payload[MAPS_END],
            payload[MAPS_END + 1],
            payload[MAPS_END + 2],
            payload[MAPS_END + 3],
        ]);
        assert_eq!(count, 0, "no records means no declared records");
    }

    #[test]
    fn the_payload_length_limit_uses_the_frame_length_field() {
        assert_eq!(
            base_payload_len(0).expect("a base fits"),
            SETUP_BASE_WIRE_SIZE
        );
        assert_eq!(
            base_payload_len(1).expect("one record fits"),
            SETUP_BASE_WIRE_SIZE + LOGIN_ON_SETUP_WIRE_SIZE
        );
        // A record count whose byte width overflows `usize` is refused without
        // allocating anything.
        assert_eq!(
            base_payload_len(usize::MAX),
            Err(SetupEncodeError::PayloadTooLarge {
                payload_len: usize::MAX,
                max_payload_len: u32::MAX as usize,
            })
        );
    }

    #[test]
    fn an_encoded_setup_payload_survives_a_real_db_frame() {
        // The game does not write bare payloads; it writes DB frames. Prove the
        // encoded bytes travel through the production frame codec and come back
        // identical, so the game-side encoder and the frame layer agree on a
        // real wire image rather than on a shared buffer.
        let payload =
            encode_setup_payload(&expected_base(0, 0), &[]).expect("composition succeeds");
        let mut frame = Vec::new();
        frame.push(HEADER_GD_SETUP);
        frame.extend_from_slice(&0_u32.to_le_bytes());
        frame.extend_from_slice(
            &u32::try_from(payload.len())
                .expect("a small payload fits the length field")
                .to_le_bytes(),
        );
        frame.extend_from_slice(&payload);

        // Feed the envelope through the production frame decoder in the two
        // halves a socket actually delivers.
        let mut decoder = DbFrameDecoder::new();
        assert_eq!(
            decoder
                .try_decode()
                .expect("a partial frame is not an error"),
            None,
            "a five-byte header prefix is not yet a frame"
        );
        decoder
            .feed(&frame[0..5])
            .expect("the header prefix is accepted");
        decoder.feed(&frame[5..]).expect("the payload is accepted");
        let recovered = decoder
            .try_decode()
            .expect("the frame codec accepts the envelope")
            .expect("a complete frame is produced");
        assert_eq!(recovered.header, HEADER_GD_SETUP);
        assert_eq!(recovered.handle, 0, "legacy sends SETUP with handle 0");
        assert_eq!(recovered.payload, payload);
        assert_eq!(frame.len(), 9 + SETUP_BASE_WIRE_SIZE);
    }

    #[test]
    fn the_map_field_is_signed_and_little_endian() {
        // A negative map index is how legacy encodes "no map", and it must
        // survive as two's complement, not as a truncated unsigned value.
        let mut base = expected_base(0, 0);
        base.maps[0] = -1;
        base.maps[1] = -2;
        base.maps[2] = 0;
        base.maps[3] = 1;
        let bytes = encode_setup_base(&base, 0).expect("a base encodes");
        let start = P2P_PORT_END;
        assert_eq!(&bytes[start..start + 4], &(-1_i32).to_le_bytes());
        assert_eq!(&bytes[start + 4..start + 8], &(-2_i32).to_le_bytes());
        assert_eq!(&bytes[start + 8..start + 12], &0_i32.to_le_bytes());
        assert_eq!(&bytes[start + 12..start + 16], &1_i32.to_le_bytes());
    }
}
