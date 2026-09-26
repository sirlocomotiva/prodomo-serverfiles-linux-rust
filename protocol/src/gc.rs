//! Source-verified game-to-client authentication, roster, player-creation and
//! player-deletion results, handshake, time-sync, phase, ping, and UDP-grant
//! packets.
//!
//! The legacy server sends these packed records in
//! `server/server/game/desc.cpp`: `SendHandshake` writes
//! `TPacketGCHandshake`, `SetPhase` writes `TPacketGCPhase`, the heartbeat path
//! writes `TPacketGCPing`, and the dormant UDP path writes
//! `TPacketGCBindUDP`. The active x86 build has fixed `BYTE`, `WORD`, `DWORD`,
//! and `long` widths, so every wire field is encoded explicitly. `GcBindUdp`
//! stores the raw x86 `sockaddr_in` field words; little-endian conversion of
//! those words reproduces the network-order bytes copied by the C++ packet.
//!
//! This module is deliberately a record boundary. It does not open a socket,
//! advance a descriptor, negotiate TEA/key agreement, grant UDP access, or
//! claim that a live client session is ready.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

/// Header for `TPacketGCHandshake` (`HEADER_GC_HANDSHAKE`).
pub const HEADER_GC_HANDSHAKE: u8 = 0xff;
/// Header for `TPacketGCPhase` (`HEADER_GC_PHASE`).
pub const HEADER_GC_PHASE: u8 = 0xfd;
/// Header for `TPacketGCPing` (`HEADER_GC_PING`).
pub const HEADER_GC_PING: u8 = 44;
/// Header for `TPacketGCBindUDP` (`HEADER_GC_BINDUDP`).
pub const HEADER_GC_BINDUDP: u8 = 0xfe;
/// Header for the one-byte `TPacketGCBlank` time-sync acknowledgement
/// (`HEADER_GC_TIME_SYNC`, named `HEADER_GC_HANDSHAKE_OK` by the client).
pub const HEADER_GC_TIME_SYNC: u8 = 0xfc;

/// Header for the fixed `TPacketGCAuthSuccess` authentication-result record.
pub const HEADER_GC_AUTH_SUCCESS: u8 = 0x96;
/// Header for the fixed `TPacketGCLoginFailure` record.
pub const HEADER_GC_LOGIN_FAILURE: u8 = 7;
/// Header for the legacy three-slot `TPacketGCLoginSuccess3` client shape.
///
/// The checked-in server does not emit this header. It is retained as a
/// source constant so the active header-32 codec is not confused with the
/// client's compatibility dispatch.
pub const HEADER_GC_LOGIN_SUCCESS: u8 = 6;
/// Header for the active four-slot `TPacketGCLoginSuccess` server record.
pub const HEADER_GC_LOGIN_SUCCESS_NEWSLOT: u8 = 32;
/// Header for the fixed `TPacketGCLoginKey` record.
pub const HEADER_GC_LOGIN_KEY: u8 = 0x76;
/// Client header name for the fixed `TPacketGCPlayerCreateSuccess` record.
pub const HEADER_GC_PLAYER_CREATE_SUCCESS: u8 = 8;
/// Server header name for the fixed `TPacketGCPlayerCreateSuccess` record.
pub const HEADER_GC_CHARACTER_CREATE_SUCCESS: u8 = HEADER_GC_PLAYER_CREATE_SUCCESS;
/// Client header name for the fixed two-byte `TPacketGCCreateFailure` record.
pub const HEADER_GC_PLAYER_CREATE_FAILURE: u8 = 9;
/// Server header name for the fixed two-byte `TPacketGCCreateFailure` record.
pub const HEADER_GC_CHARACTER_CREATE_FAILURE: u8 = HEADER_GC_PLAYER_CREATE_FAILURE;
/// Client header name for the fixed `TPacketGCDestroyCharacterSuccess` record.
pub const HEADER_GC_PLAYER_DELETE_SUCCESS: u8 = 10;
/// Server header name for the fixed player-delete-success record.
pub const HEADER_GC_CHARACTER_DELETE_SUCCESS: u8 = HEADER_GC_PLAYER_DELETE_SUCCESS;
/// Client header name for the header-only wrong-social-ID delete result.
pub const HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID: u8 = 11;
/// Server header name for the header-only wrong-social-ID delete result.
pub const HEADER_GC_CHARACTER_DELETE_WRONG_SOCIAL_ID: u8 = HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID;
/// `HEADER_GC_WARP` (65). `TPacketGCWarp`, 15 packed bytes.
pub const HEADER_GC_WARP: u8 = 65;
/// `HEADER_GC_ADD_FLY_TARGETING` (69). `TPacketGCFlyTargeting`, 17 packed bytes.
pub const HEADER_GC_ADD_FLY_TARGETING: u8 = 69;
/// `HEADER_GC_FLY_TARGETING` (71). `TPacketGCFlyTargeting`, 17 packed bytes.
pub const HEADER_GC_FLY_TARGETING: u8 = 71;
/// The live skill-level record, byte 76.
///
/// The client's `HEADER_GC_SKILL_LEVEL_NEW` is 76 and the server's
/// `HEADER_GC_SKILL_LEVEL` is 76. The client's `HEADER_GC_SKILL_LEVEL` is 72
/// and pairs with the server's `HEADER_GC_SKILL_LEVEL_OLD`, which is declared
/// at `packet.h:161` and never sent. Both live rows carry `TPlayerSkill[255]`.
pub const HEADER_GC_SKILL_LEVEL_NEW: u8 = 76;
/// The dead skill-level generation, byte 72. Declared by both trees, sent by
/// neither. The client's `TPacketGCSkillLevel` is a `BYTE[255]` body, so its
/// 256-byte width does not describe the live 1531-byte record.
pub const HEADER_GC_SKILL_LEVEL_OLD: u8 = 72;
/// `HEADER_GC_PARTY_UPDATE` (79). `TPacketGCPartyUpdate`, 21 packed bytes.
pub const HEADER_GC_PARTY_UPDATE: u8 = 79;
/// `HEADER_GC_PLAYER_POINT_CHANGE` (17). `TPacketGCPointChange`, 25 packed bytes.
///
/// The client names byte 17 `HEADER_GC_PLAYER_POINT_CHANGE` and the server
/// names the same byte `HEADER_GC_CHARACTER_POINT_CHANGE`
/// (`server/server/game/packet.h:117`). The Rust name follows the client,
/// because the inventory is keyed by the client's registration table. This is
/// a naming difference across trees, not a second record.
pub const HEADER_GC_PLAYER_POINT_CHANGE: u8 = 17;
/// Server header name for the byte-17 character point-change record.
///
/// `server/server/game/packet.h:117` declares byte 17 as
/// `HEADER_GC_CHARACTER_POINT_CHANGE`. It is the same wire byte as
/// [`HEADER_GC_PLAYER_POINT_CHANGE`], and this alias exists so the two trees
/// can be compared by name without a false second record appearing.
pub const HEADER_GC_CHARACTER_POINT_CHANGE: u8 = HEADER_GC_PLAYER_POINT_CHANGE;
/// Server header name for the live byte-76 skill-level record.
///
/// `server/server/game/packet.h:161` declares byte 76 as
/// `HEADER_GC_SKILL_LEVEL` and byte 72 as `HEADER_GC_SKILL_LEVEL_OLD`. The
/// live body is [`HEADER_GC_SKILL_LEVEL_NEW`] and the dead body is
/// [`HEADER_GC_SKILL_LEVEL_OLD`]; the names are swapped between trees.
pub const HEADER_GC_SKILL_LEVEL_SERVER: u8 = HEADER_GC_SKILL_LEVEL_NEW;
/// `HEADER_GC_AFFECT_ADD` (126). `TPacketGCAffectAdd`, 22 packed bytes.
pub const HEADER_GC_AFFECT_ADD: u8 = 126;
/// `HEADER_GC_DIG_MOTION` (134). `TPacketGCDigMotion`, 10 packed bytes.
pub const HEADER_GC_DIG_MOTION: u8 = 134;
/// `HEADER_GC_DAMAGE_INFO` (135). `TPacketGCDamageInfo`, 10 packed bytes.
pub const HEADER_GC_DAMAGE_INFO: u8 = 135;
/// `HEADER_GC_PREMIUM_PLAYERS` (141). `TPacketGCPremiumPlayers`, 28 packed bytes.
pub const HEADER_GC_PREMIUM_PLAYERS: u8 = 141;
/// `HEADER_GC_DAILY_GIFT` (180). `TPacketGCDailyGift`, 111 packed bytes.
pub const HEADER_GC_DAILY_GIFT: u8 = 180;
/// `HEADER_GC_CUBE_RENEWAL` (221). `TPacketGCCubeRenewalReceive`, 171 packed bytes.
pub const HEADER_GC_CUBE_RENEWAL: u8 = 221;
/// Offset of the opaque create-failure type in the declared record.
pub const GC_CREATE_FAILURE_TYPE_OFFSET: usize = 1;
/// Packed size of the declared `TPacketGCCreateFailure` record.
pub const GC_CREATE_FAILURE_WIRE_SIZE: usize = 1 + GC_CREATE_FAILURE_TYPE_OFFSET;
/// Offset of the opaque account index in the player-delete-success record.
pub const GC_PLAYER_DELETE_SUCCESS_INDEX_OFFSET: usize = 1;
/// Packed size of the `TPacketGCDestroyCharacterSuccess` record.
pub const GC_PLAYER_DELETE_SUCCESS_WIRE_SIZE: usize = 1 + GC_PLAYER_DELETE_SUCCESS_INDEX_OFFSET;
/// Packed size of the header-only wrong-social-ID delete result.
pub const GC_PLAYER_DELETE_WRONG_SOCIAL_ID_WIRE_SIZE: usize = 1;

/// Packed size of `TPacketGCHandshake` in the active x86 profile.
pub const GC_HANDSHAKE_WIRE_SIZE: usize = 13;
/// Packed size of `TPacketGCPhase` in the active x86 profile.
pub const GC_PHASE_WIRE_SIZE: usize = 2;
/// Packed size of `TPacketGCPing` in the active x86 profile.
pub const GC_PING_WIRE_SIZE: usize = 1;
/// Packed size of `TPacketGCBindUDP` in the active x86 profile.
pub const GC_BINDUDP_WIRE_SIZE: usize = 7;
/// Packed size of the one-byte `TPacketGCBlank` time-sync acknowledgement.
pub const GC_TIME_SYNC_WIRE_SIZE: usize = 1;
/// Packed size of `TPacketGCAuthSuccess` in the active x86 profile.
pub const GC_AUTH_SUCCESS_WIRE_SIZE: usize = 6;
/// Packed size of `TPacketGCLoginFailure` in the active x86 profile.
pub const GC_LOGIN_FAILURE_WIRE_SIZE: usize = 10;
/// Packed size of `TPacketGCLoginKey` in the active x86 profile.
pub const GC_LOGIN_KEY_WIRE_SIZE: usize = 5;
/// Offset of the opaque account-character index in the create-success record.
pub const GC_PLAYER_CREATE_SUCCESS_INDEX_OFFSET: usize = 1;
/// Offset of the nested active-profile player in the create-success record.
pub const GC_PLAYER_CREATE_SUCCESS_PLAYER_OFFSET: usize = 2;
/// Packed size of the nested active-profile player summary.
pub const GC_PLAYER_CREATE_SUCCESS_PLAYER_WIRE_SIZE: usize =
    crate::simple_player::SIMPLE_PLAYER_WIRE_SIZE;
/// Packed size of the active-profile `TPacketGCPlayerCreateSuccess` record.
pub const GC_PLAYER_CREATE_SUCCESS_WIRE_SIZE: usize =
    1 + GC_PLAYER_CREATE_SUCCESS_INDEX_OFFSET + GC_PLAYER_CREATE_SUCCESS_PLAYER_WIRE_SIZE;
/// Number of player slots in the active `TPacketGCLoginSuccess` profile.
pub const GC_LOGIN_SUCCESS_PLAYER_COUNT: usize = 4;
/// Packed size of one active-profile `TSimplePlayer` summary.
pub const GC_LOGIN_SUCCESS_PLAYER_WIRE_SIZE: usize = crate::simple_player::SIMPLE_PLAYER_WIRE_SIZE;
/// Raw storage width of one guild-name field, including its NUL capacity.
pub const GC_LOGIN_SUCCESS_GUILD_NAME_BYTES: usize = crate::GUILD_NAME_MAX_LEN + 1;
/// Packed size of the active header-32 `TPacketGCLoginSuccess` record.
pub const GC_LOGIN_SUCCESS_WIRE_SIZE: usize = 1
    + GC_LOGIN_SUCCESS_PLAYER_COUNT * GC_LOGIN_SUCCESS_PLAYER_WIRE_SIZE
    + GC_LOGIN_SUCCESS_PLAYER_COUNT * 4
    + GC_LOGIN_SUCCESS_PLAYER_COUNT * GC_LOGIN_SUCCESS_GUILD_NAME_BYTES
    + 4
    + 4;

/// A complete game-to-client handshake frame.
///
/// `TPacketGCHandshake` is a one-byte header followed by `dwHandshake`,
/// `dwTime`, and signed x86 `long lDelta` (`packet.h:782-788`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcHandshake {
    /// Opaque server handshake token echoed by the client.
    pub handshake: u32,
    /// Server timestamp used by the legacy time synchronization.
    pub time: u32,
    /// Signed correction supplied by the descriptor.
    pub delta: i32,
}

impl GcHandshake {
    /// Construct a handshake record without selecting a wire profile.
    #[must_use]
    pub const fn new(handshake: u32, time: u32, delta: i32) -> Self {
        Self {
            handshake,
            time,
            delta,
        }
    }

    /// Encode the complete one-byte-header wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GC_HANDSHAKE_WIRE_SIZE);
        bytes.push(HEADER_GC_HANDSHAKE);
        bytes.extend_from_slice(&self.handshake.to_le_bytes());
        bytes.extend_from_slice(&self.time.to_le_bytes());
        bytes.extend_from_slice(&self.delta.to_le_bytes());
        bytes
    }

    /// Decode one exact complete handshake frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact("TPacketGCHandshake", data, GC_HANDSHAKE_WIRE_SIZE)?;
        check_header(data, HEADER_GC_HANDSHAKE)?;
        Ok(Self {
            handshake: u32::from_le_bytes([data[1], data[2], data[3], data[4]]),
            time: u32::from_le_bytes([data[5], data[6], data[7], data[8]]),
            delta: i32::from_le_bytes([data[9], data[10], data[11], data[12]]),
        })
    }
}

/// A complete game-to-client phase-change frame.
///
/// `TPacketGCPhase` contains only the header and the one-byte `EPhase` value
/// (`packet.h:790-810`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPhase {
    /// Numeric legacy `EPhase` value.
    pub phase: u8,
}

impl GcPhase {
    /// Construct a phase frame.
    #[must_use]
    pub const fn new(phase: u8) -> Self {
        Self { phase }
    }

    /// Encode the complete one-byte-header wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        vec![HEADER_GC_PHASE, self.phase]
    }

    /// Decode one exact complete phase frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact("TPacketGCPhase", data, GC_PHASE_WIRE_SIZE)?;
        check_header(data, HEADER_GC_PHASE)?;
        Ok(Self { phase: data[1] })
    }
}

/// A complete game-to-client ping frame.
///
/// The legacy heartbeat sends only `HEADER_GC_PING` (`desc.cpp:188-190`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GcPing;

impl GcPing {
    /// Encode the complete one-byte wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        vec![HEADER_GC_PING]
    }

    /// Decode one exact complete ping frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact("TPacketGCPing", data, GC_PING_WIRE_SIZE)?;
        check_header(data, HEADER_GC_PING)?;
        Ok(Self)
    }
}

/// The one-byte game-to-client time-sync acknowledgement.
///
/// The server emits this record as `HEADER_GC_TIME_SYNC` (`0xfc`); the
/// client names the same numeric header `HEADER_GC_HANDSHAKE_OK` and receives
/// it through the one-byte `TPacketGCBlank` shape. It carries no timestamp or
/// other payload.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GcTimeSync;

impl GcTimeSync {
    /// Construct the acknowledgement value.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Encode the complete one-byte wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        vec![HEADER_GC_TIME_SYNC]
    }

    /// Decode one exact complete acknowledgement frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact("TPacketGCBlank", data, GC_TIME_SYNC_WIRE_SIZE)?;
        check_header(data, HEADER_GC_TIME_SYNC)?;
        Ok(Self)
    }
}

/// A complete game-to-client authentication-result record.
///
/// `TPacketGCAuthSuccess` is the packed six-byte record
/// `[0x96][login_key: u32 little-endian][result: u8]`. Both fields are
/// preserved as opaque wire values. This type does not validate a result or
/// login key, install a key, or perform authentication or session setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcAuthSuccess {
    /// Opaque `dwLoginKey` field from the legacy record.
    pub login_key: u32,
    /// Opaque `bResult` field from the legacy record.
    pub result: u8,
}

impl GcAuthSuccess {
    /// Construct a record from the two opaque wire fields.
    #[must_use]
    pub const fn new(login_key: u32, result: u8) -> Self {
        Self { login_key, result }
    }

    /// Encode the complete one-byte-header wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GC_AUTH_SUCCESS_WIRE_SIZE);
        bytes.push(HEADER_GC_AUTH_SUCCESS);
        bytes.extend_from_slice(&self.login_key.to_le_bytes());
        bytes.push(self.result);
        bytes
    }

    /// Decode one exact complete authentication-result frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact("TPacketGCAuthSuccess", data, GC_AUTH_SUCCESS_WIRE_SIZE)?;
        check_header(data, HEADER_GC_AUTH_SUCCESS)?;
        Ok(Self {
            login_key: u32::from_le_bytes([data[1], data[2], data[3], data[4]]),
            result: data[5],
        })
    }
}

/// A complete game-to-client login-key record.
///
/// `TPacketGCLoginKey` is the packed five-byte record
/// `[0x76][login_key: u32 little-endian]`. The login key is kept as an
/// opaque wire value; this type does not generate, validate, install, or
/// otherwise use a session key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcLoginKey {
    /// Opaque `dwLoginKey` field from the legacy record.
    pub login_key: u32,
}

impl GcLoginKey {
    /// Construct a record from the opaque wire field.
    #[must_use]
    pub const fn new(login_key: u32) -> Self {
        Self { login_key }
    }

    /// Encode the complete one-byte-header wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GC_LOGIN_KEY_WIRE_SIZE);
        bytes.push(HEADER_GC_LOGIN_KEY);
        bytes.extend_from_slice(&self.login_key.to_le_bytes());
        bytes
    }

    /// Decode one exact complete login-key frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact("TPacketGCLoginKey", data, GC_LOGIN_KEY_WIRE_SIZE)?;
        check_header(data, HEADER_GC_LOGIN_KEY)?;
        Ok(Self {
            login_key: u32::from_le_bytes([data[1], data[2], data[3], data[4]]),
        })
    }
}

/// Active-profile player summary embedded in [`GcLoginSuccess`].
///
/// This is the same packed 70-byte `TSimplePlayer` representation already
/// used by [`GcPlayerCreateSuccess`]. It is a type alias so the two records
/// cannot silently acquire different wire layouts.
pub type GcLoginPlayer = crate::simple_player::SimplePlayerRecord;

/// A complete active-profile game-to-client login-success roster record.
///
/// The checked-in server sends the packed header-32
/// `TPacketGCLoginSuccess` record. Its four active-profile player summaries
/// are followed by four guild IDs, four raw 13-byte guild-name fields, an
/// opaque `handle`, and an opaque `random_key`. The record is a transport-free
/// wire representation: it does not authenticate an account, resolve a
/// guild, select a player, or mutate a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcLoginSuccess {
    /// Four active-profile `TSimplePlayer` summaries.
    pub players: [GcLoginPlayer; GC_LOGIN_SUCCESS_PLAYER_COUNT],
    /// Four opaque guild IDs, serialized little-endian.
    pub guild_ids: [u32; GC_LOGIN_SUCCESS_PLAYER_COUNT],
    /// Four raw guild-name storage fields, each 13 bytes wide.
    pub guild_names: [[u8; GC_LOGIN_SUCCESS_GUILD_NAME_BYTES]; GC_LOGIN_SUCCESS_PLAYER_COUNT],
    /// Opaque descriptor handle from the legacy record.
    pub handle: u32,
    /// Opaque mark-auth random key from the legacy record.
    pub random_key: u32,
}

impl GcLoginSuccess {
    /// Construct an active header-32 login-success record.
    #[must_use]
    pub const fn new(
        players: [GcLoginPlayer; GC_LOGIN_SUCCESS_PLAYER_COUNT],
        guild_ids: [u32; GC_LOGIN_SUCCESS_PLAYER_COUNT],
        guild_names: [[u8; GC_LOGIN_SUCCESS_GUILD_NAME_BYTES]; GC_LOGIN_SUCCESS_PLAYER_COUNT],
        handle: u32,
        random_key: u32,
    ) -> Self {
        Self {
            players,
            guild_ids,
            guild_names,
            handle,
            random_key,
        }
    }

    /// Encode the exact active header-32 wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GC_LOGIN_SUCCESS_WIRE_SIZE);
        bytes.push(HEADER_GC_LOGIN_SUCCESS_NEWSLOT);
        for player in &self.players {
            bytes.extend_from_slice(&player.encode());
        }
        for guild_id in &self.guild_ids {
            bytes.extend_from_slice(&guild_id.to_le_bytes());
        }
        for guild_name in &self.guild_names {
            bytes.extend_from_slice(guild_name);
        }
        bytes.extend_from_slice(&self.handle.to_le_bytes());
        bytes.extend_from_slice(&self.random_key.to_le_bytes());
        bytes
    }

    /// Decode one exact active header-32 login-success frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact("TPacketGCLoginSuccess", data, GC_LOGIN_SUCCESS_WIRE_SIZE)?;
        check_header(data, HEADER_GC_LOGIN_SUCCESS_NEWSLOT)?;
        let players = [
            decode_gc_simple_player(data, 1),
            decode_gc_simple_player(data, 71),
            decode_gc_simple_player(data, 141),
            decode_gc_simple_player(data, 211),
        ];
        let guild_ids = [
            read_u32(data, 281),
            read_u32(data, 285),
            read_u32(data, 289),
            read_u32(data, 293),
        ];
        let guild_names = [
            read_array::<GC_LOGIN_SUCCESS_GUILD_NAME_BYTES>(data, 297),
            read_array::<GC_LOGIN_SUCCESS_GUILD_NAME_BYTES>(data, 310),
            read_array::<GC_LOGIN_SUCCESS_GUILD_NAME_BYTES>(data, 323),
            read_array::<GC_LOGIN_SUCCESS_GUILD_NAME_BYTES>(data, 336),
        ];
        Ok(Self {
            players,
            guild_ids,
            guild_names,
            handle: read_u32(data, 349),
            random_key: read_u32(data, 353),
        })
    }
}

fn read_array<const N: usize>(data: &[u8], offset: usize) -> [u8; N] {
    let mut value = [0; N];
    value.copy_from_slice(&data[offset..offset + N]);
    value
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(read_array::<2>(data, offset))
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(read_array::<4>(data, offset))
}

fn read_i32(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(read_array::<4>(data, offset))
}

fn decode_gc_simple_player(data: &[u8], offset: usize) -> GcLoginPlayer {
    GcLoginPlayer {
        id: read_u32(data, offset),
        name: read_array::<{ crate::simple_player::CHARACTER_NAME_BYTES }>(data, offset + 4),
        job: data[offset + 29],
        level: data[offset + 30],
        play_minutes: read_u32(data, offset + 31),
        st: data[offset + 35],
        ht: data[offset + 36],
        dx: data[offset + 37],
        iq: data[offset + 38],
        main_part: read_u16(data, offset + 39),
        change_name: data[offset + 41],
        hair_part: read_u16(data, offset + 42),
        sash_part: read_u16(data, offset + 44),
        dummy: read_array::<4>(data, offset + 46),
        x: read_i32(data, offset + 50),
        y: read_i32(data, offset + 54),
        addr: read_i32(data, offset + 58),
        port: read_u16(data, offset + 62),
        skill_group: data[offset + 64],
        conqueror_level: data[offset + 65],
        sungma_str: data[offset + 66],
        sungma_hp: data[offset + 67],
        sungma_move: data[offset + 68],
        sungma_immune: data[offset + 69],
    }
}

/// A complete game-to-client login-failure record.
///
/// `TPacketGCLoginFailure` is the packed ten-byte record
/// `[0x07][status bytes][NUL]`. The legacy `szStatus` field has eight logical
/// bytes and a ninth terminator byte. `status` contains the raw bytes before
/// the first NUL; it is not assumed to be UTF-8 or to carry a known message.
///
/// The encoder copies at most eight bytes and zero-fills its unused suffix.
/// The decoder requires a NUL somewhere in the nine status bytes and ignores
/// bytes after the first one, matching the source's observable C-string use
/// without reproducing its missing-terminator over-read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcLoginFailure {
    /// Raw logical status bytes before the first wire NUL terminator.
    pub status: Vec<u8>,
}

impl GcLoginFailure {
    /// Construct a record from raw status bytes.
    ///
    /// Copying stops at the first NUL or after eight bytes, whichever comes
    /// first. The remaining wire capacity is filled with zero bytes by
    /// [`Self::encode`].
    #[must_use]
    pub fn new(status: &[u8]) -> Self {
        let end = status
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(status.len())
            .min(8);
        Self {
            status: status[..end].to_vec(),
        }
    }

    /// Encode the complete one-byte-header wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = vec![0; GC_LOGIN_FAILURE_WIRE_SIZE];
        bytes[0] = HEADER_GC_LOGIN_FAILURE;
        let end = self
            .status
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(self.status.len())
            .min(8);
        bytes[1..=end].copy_from_slice(&self.status[..end]);
        bytes
    }

    /// Decode one exact complete login-failure frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame, a different header,
    /// or a status field without a NUL terminator.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact("TPacketGCLoginFailure", data, GC_LOGIN_FAILURE_WIRE_SIZE)?;
        check_header(data, HEADER_GC_LOGIN_FAILURE)?;
        let end =
            data[1..]
                .iter()
                .position(|byte| *byte == 0)
                .ok_or(GcPacketError::MissingNul {
                    record: "TPacketGCLoginFailure",
                })?;
        Ok(Self {
            status: data[1..=end].to_vec(),
        })
    }
}

/// A complete active-profile game-to-client player-create-success record.
///
/// `TPacketGCPlayerCreateSuccess` is the packed 72-byte record containing a
/// header, an opaque account-character index, and one active-profile
/// `TSimplePlayer`. The legacy producer and client separately range-check the
/// index; this transport-free codec preserves every `u8` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPlayerCreateSuccess {
    /// Opaque account-character index from the legacy record.
    pub account_character_index: u8,
    /// Raw active-profile player summary copied into the success record.
    pub player: crate::simple_player::SimplePlayerRecord,
}

impl GcPlayerCreateSuccess {
    /// Construct a player-create-success record without producer-side policy.
    #[must_use]
    pub const fn new(
        account_character_index: u8,
        player: crate::simple_player::SimplePlayerRecord,
    ) -> Self {
        Self {
            account_character_index,
            player,
        }
    }

    /// Encode the exact active-profile wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GC_PLAYER_CREATE_SUCCESS_WIRE_SIZE);
        bytes.push(HEADER_GC_PLAYER_CREATE_SUCCESS);
        bytes.push(self.account_character_index);
        bytes.extend_from_slice(&self.player.encode());
        bytes
    }

    /// Decode one exact complete player-create-success frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact(
            "TPacketGCPlayerCreateSuccess",
            data,
            GC_PLAYER_CREATE_SUCCESS_WIRE_SIZE,
        )?;
        check_header(data, HEADER_GC_PLAYER_CREATE_SUCCESS)?;
        let player = decode_gc_simple_player(data, GC_PLAYER_CREATE_SUCCESS_PLAYER_OFFSET);
        Ok(Self {
            account_character_index: data[GC_PLAYER_CREATE_SUCCESS_INDEX_OFFSET],
            player,
        })
    }
}

/// The exact declared two-byte game-to-client character-create failure.
///
/// `TPacketGCCreateFailure` contains a header and one raw `bType` byte. Normal
/// legacy producers use types 0 and 1, but the C++ field is a plain `BYTE`, so
/// this record preserves every `u8` value. Separate checked-in server paths
/// accidentally send one-byte and ten-byte header-9 frames; those malformed
/// outcomes are deliberately not accepted as this declared record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcCreateFailure {
    /// Opaque legacy create-failure type.
    pub failure_type: u8,
}

impl GcCreateFailure {
    /// Construct a declared create-failure record without classifying its type.
    #[must_use]
    pub const fn new(failure_type: u8) -> Self {
        Self { failure_type }
    }

    /// Encode the exact declared two-byte frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        vec![HEADER_GC_PLAYER_CREATE_FAILURE, self.failure_type]
    }

    /// Decode one exact complete declared create-failure frame.
    ///
    /// The one-byte and ten-byte malformed sends present in the legacy server
    /// are framing defects, not alternate profiles, and are rejected by the
    /// exact-size check.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact("TPacketGCCreateFailure", data, GC_CREATE_FAILURE_WIRE_SIZE)?;
        check_header(data, HEADER_GC_PLAYER_CREATE_FAILURE)?;
        Ok(Self {
            failure_type: data[GC_CREATE_FAILURE_TYPE_OFFSET],
        })
    }
}

/// The exact two-byte game-to-client player-delete-success record.
///
/// The checked-in server composes `[0x0a][account_index]` with a buffered
/// header followed by a one-byte `Packet` call. The matching client receiver
/// consumes the packed `TPacketGCDestroyCharacterSuccess` fields. The account
/// index remains an opaque `u8` here; roster and producer range policy belongs
/// outside this transport-free record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPlayerDeleteSuccess {
    /// Opaque account-character index copied from the delete result.
    pub account_index: u8,
}

impl GcPlayerDeleteSuccess {
    /// Construct a delete-success record without imposing slot policy.
    #[must_use]
    pub const fn new(account_index: u8) -> Self {
        Self { account_index }
    }

    /// Encode the exact two-byte wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        vec![HEADER_GC_PLAYER_DELETE_SUCCESS, self.account_index]
    }

    /// Decode one exact complete player-delete-success frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact(
            "TPacketGCDestroyCharacterSuccess",
            data,
            GC_PLAYER_DELETE_SUCCESS_WIRE_SIZE,
        )?;
        check_header(data, HEADER_GC_PLAYER_DELETE_SUCCESS)?;
        Ok(Self {
            account_index: data[GC_PLAYER_DELETE_SUCCESS_INDEX_OFFSET],
        })
    }
}

/// The exact header-only game-to-client wrong-social-ID delete result.
///
/// The legacy name is retained for wire compatibility, but checked-in server
/// paths use this header for several DB deletion failures. This record carries
/// no account index or failure reason and does not infer outcome semantics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GcPlayerDeleteWrongSocialId;

impl GcPlayerDeleteWrongSocialId {
    /// Encode the exact one-byte wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        vec![HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID]
    }

    /// Decode one exact complete header-only delete result.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact(
            "TPacketGCBlank",
            data,
            GC_PLAYER_DELETE_WRONG_SOCIAL_ID_WIRE_SIZE,
        )?;
        check_header(data, HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID)?;
        Ok(Self)
    }
}

/// A complete game-to-client UDP endpoint record.
///
/// `addr` and `port` are the raw x86 `DWORD` and `WORD` field values copied
/// from `sockaddr_in` by `DESC::UDPGrant`. The little-endian host encoding of
/// those raw words reproduces the network-order address and port bytes in the
/// packed legacy record. The values are intentionally opaque; this record does
/// not convert a socket address or perform endpoint validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcBindUdp {
    /// Raw x86 `sockaddr_in::sin_addr.s_addr` field value.
    pub addr: u32,
    /// Raw x86 `sockaddr_in::sin_port` field value.
    pub port: u16,
}

impl GcBindUdp {
    /// Construct a record from the raw x86 field values.
    #[must_use]
    pub const fn new(addr: u32, port: u16) -> Self {
        Self { addr, port }
    }

    /// Encode the complete one-byte-header wire frame.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GC_BINDUDP_WIRE_SIZE);
        bytes.push(HEADER_GC_BINDUDP);
        bytes.extend_from_slice(&self.addr.to_le_bytes());
        bytes.extend_from_slice(&self.port.to_le_bytes());
        bytes
    }

    /// Decode one exact complete UDP endpoint frame.
    ///
    /// # Errors
    ///
    /// Returns [`GcPacketError`] for a short/long frame or a different header.
    pub fn decode(data: &[u8]) -> Result<Self, GcPacketError> {
        check_exact("TPacketGCBindUDP", data, GC_BINDUDP_WIRE_SIZE)?;
        check_header(data, HEADER_GC_BINDUDP)?;
        Ok(Self {
            addr: u32::from_le_bytes([data[1], data[2], data[3], data[4]]),
            port: u16::from_le_bytes([data[5], data[6]]),
        })
    }
}

/// Source-name alias for [`GcBindUdp`].
pub type GcBindUDP = GcBindUdp;

/// A malformed fixed game-to-client record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcPacketError {
    /// The complete record was shorter than its packed wire size.
    Truncated {
        /// C++ record name used in diagnostics.
        record: &'static str,
        /// Required complete wire size.
        needed: usize,
        /// Bytes supplied by the caller.
        available: usize,
    },
    /// The complete record had bytes beyond its packed wire size.
    LengthMismatch {
        /// C++ record name used in diagnostics.
        record: &'static str,
        /// Exact complete wire size.
        expected: usize,
        /// Bytes supplied by the caller.
        actual: usize,
    },
    /// The record header did not match the expected game-to-client header.
    InvalidHeader {
        /// Expected one-byte header.
        expected: u8,
        /// Header supplied by the caller.
        actual: u8,
    },
    /// A fixed C-string field did not contain a NUL terminator.
    MissingNul {
        /// C++ record name used in diagnostics.
        record: &'static str,
    },
}

impl fmt::Display for GcPacketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                record,
                needed,
                available,
            } => write!(
                formatter,
                "{record} is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch {
                record,
                expected,
                actual,
            } => write!(
                formatter,
                "{record} has {actual} bytes; expected exactly {expected}"
            ),
            Self::InvalidHeader { expected, actual } => write!(
                formatter,
                "expected game-to-client header 0x{expected:02x}, got 0x{actual:02x}"
            ),
            Self::MissingNul { record } => {
                write!(formatter, "{record} status field has no NUL terminator")
            }
        }
    }
}

impl Error for GcPacketError {}

fn check_exact(record: &'static str, data: &[u8], expected: usize) -> Result<(), GcPacketError> {
    match data.len().cmp(&expected) {
        std::cmp::Ordering::Less => Err(GcPacketError::Truncated {
            record,
            needed: expected,
            available: data.len(),
        }),
        std::cmp::Ordering::Greater => Err(GcPacketError::LengthMismatch {
            record,
            expected,
            actual: data.len(),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_header(data: &[u8], expected: u8) -> Result<(), GcPacketError> {
    let actual = data.first().copied().unwrap_or_default();
    if actual == expected {
        Ok(())
    } else {
        Err(GcPacketError::InvalidHeader { expected, actual })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_login_player(id: u32, marker: u8) -> GcLoginPlayer {
        GcLoginPlayer {
            id,
            name: [marker; 25],
            job: marker,
            level: marker.wrapping_add(1),
            play_minutes: u32::from(marker).wrapping_mul(0x0101_0101),
            st: marker.wrapping_add(2),
            ht: marker.wrapping_add(3),
            dx: marker.wrapping_add(4),
            iq: marker.wrapping_add(5),
            main_part: u16::from(marker).wrapping_mul(0x0101),
            change_name: marker.wrapping_add(6),
            hair_part: u16::from(marker).wrapping_mul(0x0202),
            sash_part: u16::from(marker).wrapping_mul(0x0303),
            dummy: [marker.wrapping_add(7); 4],
            x: i32::from(marker).wrapping_mul(1000),
            y: i32::from(marker).wrapping_mul(2000).wrapping_neg(),
            addr: i32::from(marker).wrapping_mul(0x0100_0000),
            port: u16::from(marker).wrapping_mul(0x0404),
            skill_group: marker.wrapping_add(8),
            conqueror_level: marker.wrapping_add(9),
            sungma_str: marker.wrapping_add(10),
            sungma_hp: marker.wrapping_add(11),
            sungma_move: marker.wrapping_add(12),
            sungma_immune: marker.wrapping_add(13),
        }
    }

    #[test]
    fn handshake_has_source_exact_size_and_signed_little_endian_fields() {
        let packet = GcHandshake::new(0x1234_5678, 0xdead_beef, -123_456);
        assert_eq!(packet.encode().len(), GC_HANDSHAKE_WIRE_SIZE);
        assert_eq!(
            packet.encode(),
            vec![0xff, 0x78, 0x56, 0x34, 0x12, 0xef, 0xbe, 0xad, 0xde, 0xc0, 0x1d, 0xfe, 0xff,]
        );
        assert_eq!(GcHandshake::decode(&packet.encode()).unwrap(), packet);
    }

    #[test]
    fn phase_and_ping_match_legacy_one_byte_payloads() {
        let phase = GcPhase::new(5);
        assert_eq!(phase.encode(), vec![0xfd, 5]);
        assert_eq!(GcPhase::decode(&phase.encode()).unwrap(), phase);

        let ping = GcPing;
        assert_eq!(ping.encode(), vec![44]);
        assert_eq!(GcPing::decode(&ping.encode()).unwrap(), ping);
    }

    #[test]
    fn decoders_reject_wrong_lengths_and_headers() {
        assert!(matches!(
            GcHandshake::decode(&[HEADER_GC_HANDSHAKE]),
            Err(GcPacketError::Truncated {
                record: "TPacketGCHandshake",
                needed: 13,
                available: 1
            })
        ));
        let mut long = GcHandshake::new(1, 2, 3).encode();
        long.push(0);
        assert!(matches!(
            GcHandshake::decode(&long),
            Err(GcPacketError::LengthMismatch {
                record: "TPacketGCHandshake",
                expected: 13,
                actual: 14
            })
        ));
        assert_eq!(
            GcPhase::decode(&[HEADER_GC_HANDSHAKE, 5]),
            Err(GcPacketError::InvalidHeader {
                expected: HEADER_GC_PHASE,
                actual: HEADER_GC_HANDSHAKE
            })
        );
        assert_eq!(
            GcPing::decode(&[]),
            Err(GcPacketError::Truncated {
                record: "TPacketGCPing",
                needed: 1,
                available: 0
            })
        );
    }

    #[test]
    fn all_values_round_trip_without_host_layout_assumptions() {
        for value in [0, 1, u32::MAX] {
            let delta = i32::from_le_bytes(value.to_le_bytes());
            let packet = GcHandshake::new(value, value, delta);
            assert_eq!(GcHandshake::decode(&packet.encode()).unwrap(), packet);
        }
        for phase in [0, 1, 10, u8::MAX] {
            let packet = GcPhase::new(phase);
            assert_eq!(GcPhase::decode(&packet.encode()).unwrap(), packet);
        }
    }

    #[test]
    fn auth_success_has_source_exact_size_and_little_endian_fields() {
        assert_eq!(HEADER_GC_AUTH_SUCCESS, 0x96);
        assert_eq!(GC_AUTH_SUCCESS_WIRE_SIZE, 6);

        let packet = GcAuthSuccess::new(0x1234_5678, 0xfe);
        assert_eq!(packet.encode(), vec![0x96, 0x78, 0x56, 0x34, 0x12, 0xfe]);
        assert_eq!(GcAuthSuccess::decode(&packet.encode()).unwrap(), packet);
    }

    #[test]
    fn auth_success_round_trips_key_extrema_and_raw_result_bytes() {
        for login_key in [0, 1, 0x1234_5678, u32::MAX] {
            for result in [0, 1, 2, u8::MAX] {
                let packet = GcAuthSuccess::new(login_key, result);
                assert_eq!(packet.encode().len(), GC_AUTH_SUCCESS_WIRE_SIZE);
                assert_eq!(GcAuthSuccess::decode(&packet.encode()).unwrap(), packet);
            }
        }
    }

    #[test]
    fn auth_success_rejects_truncation_trailing_bytes_and_wrong_headers() {
        for available in 0..GC_AUTH_SUCCESS_WIRE_SIZE {
            assert!(matches!(
                GcAuthSuccess::decode(&vec![HEADER_GC_AUTH_SUCCESS; available]),
                Err(GcPacketError::Truncated {
                    record: "TPacketGCAuthSuccess",
                    needed: 6,
                    available
                }) if available < 6
            ));
        }

        assert!(matches!(
            GcAuthSuccess::decode(&[HEADER_GC_AUTH_SUCCESS, 0, 0, 0, 0, 0, 0]),
            Err(GcPacketError::LengthMismatch {
                record: "TPacketGCAuthSuccess",
                expected: 6,
                actual: 7
            })
        ));
        assert!(matches!(
            GcAuthSuccess::decode(&[0x95, 0, 0, 0, 0, 0]),
            Err(GcPacketError::InvalidHeader {
                expected: HEADER_GC_AUTH_SUCCESS,
                actual: 0x95
            })
        ));
    }

    #[test]
    fn login_key_has_source_exact_size_and_little_endian_field() {
        assert_eq!(HEADER_GC_LOGIN_KEY, 0x76);
        assert_eq!(GC_LOGIN_KEY_WIRE_SIZE, 5);

        let packet = GcLoginKey::new(0x1234_5678);
        assert_eq!(packet.encode(), vec![0x76, 0x78, 0x56, 0x34, 0x12]);
        assert_eq!(GcLoginKey::decode(&packet.encode()).unwrap(), packet);
    }

    #[test]
    fn login_key_round_trips_zero_high_bit_and_max_values() {
        for login_key in [0, 1, 0x1234_5678, 0x8000_0001, u32::MAX] {
            let packet = GcLoginKey::new(login_key);
            assert_eq!(packet.encode().len(), GC_LOGIN_KEY_WIRE_SIZE);
            assert_eq!(GcLoginKey::decode(&packet.encode()).unwrap(), packet);
        }
    }

    #[test]
    fn login_key_rejects_truncation_trailing_bytes_and_wrong_headers() {
        for available in 0..GC_LOGIN_KEY_WIRE_SIZE {
            assert!(matches!(
                GcLoginKey::decode(&vec![HEADER_GC_LOGIN_KEY; available]),
                Err(GcPacketError::Truncated {
                    record: "TPacketGCLoginKey",
                    needed: 5,
                    available
                }) if available < 5
            ));
        }

        assert!(matches!(
            GcLoginKey::decode(&[HEADER_GC_LOGIN_KEY, 0, 0, 0, 0, 0]),
            Err(GcPacketError::LengthMismatch {
                record: "TPacketGCLoginKey",
                expected: 5,
                actual: 6
            })
        ));
        assert!(matches!(
            GcLoginKey::decode(&[0x77, 0, 0, 0, 0]),
            Err(GcPacketError::InvalidHeader {
                expected: HEADER_GC_LOGIN_KEY,
                actual: 0x77
            })
        ));
    }

    #[test]
    fn login_failure_has_source_exact_size_and_known_vectors() {
        assert_eq!(HEADER_GC_LOGIN_FAILURE, 7);
        assert_eq!(GC_LOGIN_FAILURE_WIRE_SIZE, 10);

        let empty = GcLoginFailure::new(b"");
        assert_eq!(empty.status, Vec::<u8>::new());
        assert_eq!(empty.encode(), vec![7, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(GcLoginFailure::decode(&empty.encode()).unwrap(), empty);

        let noid = GcLoginFailure::new(b"NOID");
        assert_eq!(noid.status, b"NOID");
        assert_eq!(
            noid.encode(),
            vec![7, b'N', b'O', b'I', b'D', 0, 0, 0, 0, 0]
        );
        assert_eq!(GcLoginFailure::decode(&noid.encode()).unwrap(), noid);

        let shutdown = GcLoginFailure::new(b"SHUTDOWN");
        assert_eq!(shutdown.status, b"SHUTDOWN");
        assert_eq!(
            shutdown.encode(),
            vec![7, b'S', b'H', b'U', b'T', b'D', b'O', b'W', b'N', 0]
        );
        assert_eq!(
            GcLoginFailure::decode(&shutdown.encode()).unwrap(),
            shutdown
        );

        let truncated = GcLoginFailure::new(b"ABCDEFGHIJK");
        assert_eq!(truncated.status, b"ABCDEFGH");
        assert_eq!(
            truncated.encode(),
            vec![7, b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', 0]
        );
    }

    #[test]
    fn login_failure_preserves_raw_bytes_and_ignores_after_first_nul() {
        let raw = GcLoginFailure::new(&[0x80, 0xff, 0x00, b'X']);
        assert_eq!(raw.status, vec![0x80, 0xff]);
        assert_eq!(raw.encode()[0], HEADER_GC_LOGIN_FAILURE);
        assert_eq!(&raw.encode()[1..3], &[0x80, 0xff]);
        assert_eq!(GcLoginFailure::decode(&raw.encode()).unwrap(), raw);

        let ignored_tail = [7, b'N', b'O', b'I', b'D', 0, 0xaa, 0xbb, 0xcc, 0xdd];
        assert_eq!(
            GcLoginFailure::decode(&ignored_tail).unwrap().status,
            b"NOID"
        );

        let embedded = [7, b'A', 0, b'B', b'C', b'D', b'E', b'F', b'G', b'H'];
        assert_eq!(GcLoginFailure::decode(&embedded).unwrap().status, b"A");
    }

    #[test]
    fn login_failure_rejects_bad_lengths_headers_and_missing_nul() {
        for available in 0..GC_LOGIN_FAILURE_WIRE_SIZE {
            assert!(matches!(
                GcLoginFailure::decode(&vec![HEADER_GC_LOGIN_FAILURE; available]),
                Err(GcPacketError::Truncated {
                    record: "TPacketGCLoginFailure",
                    needed: 10,
                    available
                }) if available < 10
            ));
        }

        assert!(matches!(
            GcLoginFailure::decode(&[7; 11]),
            Err(GcPacketError::LengthMismatch {
                record: "TPacketGCLoginFailure",
                expected: 10,
                actual: 11
            })
        ));
        assert!(matches!(
            GcLoginFailure::decode(&[7; 10]),
            Err(GcPacketError::MissingNul {
                record: "TPacketGCLoginFailure"
            })
        ));
        assert!(matches!(
            GcLoginFailure::decode(&[8, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(GcPacketError::InvalidHeader {
                expected: HEADER_GC_LOGIN_FAILURE,
                actual: 8
            })
        ));
    }

    #[test]
    fn player_create_success_has_source_exact_size_and_golden_layout() {
        assert_eq!(HEADER_GC_PLAYER_CREATE_SUCCESS, 8);
        assert_eq!(HEADER_GC_CHARACTER_CREATE_SUCCESS, 8);
        assert_eq!(GC_PLAYER_CREATE_SUCCESS_INDEX_OFFSET, 1);
        assert_eq!(GC_PLAYER_CREATE_SUCCESS_PLAYER_OFFSET, 2);
        assert_eq!(GC_PLAYER_CREATE_SUCCESS_PLAYER_WIRE_SIZE, 70);
        assert_eq!(GC_PLAYER_CREATE_SUCCESS_WIRE_SIZE, 72);

        let mut player = sample_login_player(0x0102_0304, 0x11);
        player.name[..5].copy_from_slice(b"Crate");
        player.name[5] = 0;
        player.name[6] = 0x80;
        player.name[7] = 0xff;
        player.dummy = [0x00, 0x80, 0xff, 0xaa];
        player.x = -123_456;
        player.y = 654_321;
        player.addr = 0x0100_007f;
        player.port = 0xabcd;
        let packet = GcPlayerCreateSuccess::new(2, player);
        let encoded = packet.encode();

        // Independently transcribed source-order bytes for the outer fields
        // and every active-profile TSimplePlayer field.
        let expected: [u8; GC_PLAYER_CREATE_SUCCESS_WIRE_SIZE] = [
            0x08, 0x02, 0x04, 0x03, 0x02, 0x01, 0x43, 0x72, 0x61, 0x74, 0x65, 0x00, 0x80, 0xff,
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
            0x11, 0x11, 0x11, 0x11, 0x12, 0x11, 0x11, 0x11, 0x11, 0x13, 0x14, 0x15, 0x16, 0x11,
            0x11, 0x17, 0x22, 0x22, 0x33, 0x33, 0x00, 0x80, 0xff, 0xaa, 0xc0, 0x1d, 0xfe, 0xff,
            0xf1, 0xfb, 0x09, 0x00, 0x7f, 0x00, 0x00, 0x01, 0xcd, 0xab, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e,
        ];
        assert_eq!(encoded, expected);
        assert_eq!(GcPlayerCreateSuccess::decode(&encoded).unwrap(), packet);
    }

    #[test]
    fn player_create_success_round_trips_raw_fields_extrema_and_opaque_slots() {
        let mut low = sample_login_player(0, 0);
        low.name = [0; 25];
        low.job = 0;
        low.level = 0;
        low.play_minutes = 0;
        low.st = 0;
        low.ht = 0;
        low.dx = 0;
        low.iq = 0;
        low.main_part = 0;
        low.change_name = 0;
        low.hair_part = 0;
        low.sash_part = 0;
        low.dummy = [0; 4];
        low.x = i32::MIN;
        low.y = i32::MIN;
        low.addr = i32::MIN;
        low.port = 0;
        low.skill_group = 0;
        low.conqueror_level = 0;
        low.sungma_str = 0;
        low.sungma_hp = 0;
        low.sungma_move = 0;
        low.sungma_immune = 0;

        let mut high = sample_login_player(u32::MAX, u8::MAX);
        high.name = [u8::MAX; 25];
        high.job = u8::MAX;
        high.level = u8::MAX;
        high.play_minutes = u32::MAX;
        high.st = u8::MAX;
        high.ht = u8::MAX;
        high.dx = u8::MAX;
        high.iq = u8::MAX;
        high.main_part = u16::MAX;
        high.change_name = u8::MAX;
        high.hair_part = u16::MAX;
        high.sash_part = u16::MAX;
        high.dummy = [u8::MAX; 4];
        high.x = i32::MAX;
        high.y = i32::MAX;
        high.addr = i32::MAX;
        high.port = u16::MAX;
        high.skill_group = u8::MAX;
        high.conqueror_level = u8::MAX;
        high.sungma_str = u8::MAX;
        high.sungma_hp = u8::MAX;
        high.sungma_move = u8::MAX;
        high.sungma_immune = u8::MAX;

        for player in [low, high] {
            for account_character_index in [0, 3, 4, u8::MAX] {
                let packet = GcPlayerCreateSuccess::new(account_character_index, player);
                let encoded = packet.encode();
                assert_eq!(encoded.len(), GC_PLAYER_CREATE_SUCCESS_WIRE_SIZE);
                assert_eq!(GcPlayerCreateSuccess::decode(&encoded).unwrap(), packet);
            }
        }
    }

    #[test]
    fn player_create_success_rejects_every_bad_length_and_wrong_header() {
        for available in 0..GC_PLAYER_CREATE_SUCCESS_WIRE_SIZE {
            assert!(matches!(
                GcPlayerCreateSuccess::decode(&vec![HEADER_GC_CHARACTER_CREATE_SUCCESS; available]),
                Err(GcPacketError::Truncated {
                    record: "TPacketGCPlayerCreateSuccess",
                    needed: 72,
                    available: actual
                }) if actual == available
            ));
        }

        assert!(matches!(
            GcPlayerCreateSuccess::decode(&[9; 71]),
            Err(GcPacketError::Truncated {
                record: "TPacketGCPlayerCreateSuccess",
                needed: 72,
                available: 71
            })
        ));
        assert!(matches!(
            GcPlayerCreateSuccess::decode(
                &[HEADER_GC_PLAYER_CREATE_SUCCESS; GC_PLAYER_CREATE_SUCCESS_WIRE_SIZE + 1]
            ),
            Err(GcPacketError::LengthMismatch {
                record: "TPacketGCPlayerCreateSuccess",
                expected: 72,
                actual: 73
            })
        ));
        for actual in [7, 9] {
            let mut frame = vec![0; GC_PLAYER_CREATE_SUCCESS_WIRE_SIZE];
            frame[0] = actual;
            assert!(matches!(
                GcPlayerCreateSuccess::decode(&frame),
                Err(GcPacketError::InvalidHeader {
                    expected: HEADER_GC_PLAYER_CREATE_SUCCESS,
                    actual: header
                }) if header == actual
            ));
        }
    }

    #[test]
    fn create_failure_has_source_exact_layout_and_known_types() {
        assert_eq!(HEADER_GC_PLAYER_CREATE_FAILURE, 9);
        assert_eq!(HEADER_GC_CHARACTER_CREATE_FAILURE, 9);
        assert_eq!(GC_CREATE_FAILURE_TYPE_OFFSET, 1);
        assert_eq!(GC_CREATE_FAILURE_WIRE_SIZE, 2);

        for (failure_type, expected) in [
            (0, [0x09, 0x00]),
            (1, [0x09, 0x01]),
            (u8::MAX, [0x09, 0xff]),
        ] {
            let packet = GcCreateFailure::new(failure_type);
            assert_eq!(packet.encode(), expected);
            assert_eq!(GcCreateFailure::decode(&expected).unwrap(), packet);
        }
    }

    #[test]
    fn create_failure_round_trips_every_raw_type_and_rejects_malformed_source_sends() {
        for failure_type in u8::MIN..=u8::MAX {
            let packet = GcCreateFailure::new(failure_type);
            let encoded = packet.encode();
            assert_eq!(encoded, [0x09, failure_type]);
            assert_eq!(GcCreateFailure::decode(&encoded).unwrap(), packet);
        }

        // CInputDB sends this one-byte frame for an out-of-range DB index.
        assert!(matches!(
            GcCreateFailure::decode(&[0x09]),
            Err(GcPacketError::Truncated {
                record: "TPacketGCCreateFailure",
                needed: 2,
                available: 1
            })
        ));
        // CInputLogin sends this zero-filled, wrong-type 10-byte frame on
        // several creation-error paths. It is a source framing defect.
        let source_wrong_type_frame = [0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        for frame in [&[0x09; 10][..], source_wrong_type_frame.as_slice()] {
            assert!(matches!(
                GcCreateFailure::decode(frame),
                Err(GcPacketError::LengthMismatch {
                    record: "TPacketGCCreateFailure",
                    expected: 2,
                    actual: 10
                })
            ));
        }
    }

    #[test]
    fn create_failure_rejects_every_bad_length_and_wrong_header() {
        for available in 0..GC_CREATE_FAILURE_WIRE_SIZE {
            assert!(matches!(
                GcCreateFailure::decode(&vec![HEADER_GC_CHARACTER_CREATE_FAILURE; available]),
                Err(GcPacketError::Truncated {
                    record: "TPacketGCCreateFailure",
                    needed: 2,
                    available: actual
                }) if actual == available
            ));
        }

        // Complete length is checked before the header at every length.
        assert!(matches!(
            GcCreateFailure::decode(&[0x08]),
            Err(GcPacketError::Truncated {
                record: "TPacketGCCreateFailure",
                needed: 2,
                available: 1
            })
        ));
        assert!(matches!(
            GcCreateFailure::decode(&[0x08, 0, 0]),
            Err(GcPacketError::LengthMismatch {
                record: "TPacketGCCreateFailure",
                expected: 2,
                actual: 3
            })
        ));
        for actual in [0, 7, 8, 10, u8::MAX] {
            let mut frame = vec![0; GC_CREATE_FAILURE_WIRE_SIZE];
            frame[0] = actual;
            assert!(matches!(
                GcCreateFailure::decode(&frame),
                Err(GcPacketError::InvalidHeader {
                    expected: HEADER_GC_PLAYER_CREATE_FAILURE,
                    actual: header
                }) if header == actual
            ));
        }
    }

    #[test]
    fn player_delete_results_have_source_exact_headers_and_layouts() {
        assert_eq!(HEADER_GC_PLAYER_DELETE_SUCCESS, 10);
        assert_eq!(HEADER_GC_CHARACTER_DELETE_SUCCESS, 10);
        assert_eq!(HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID, 11);
        assert_eq!(HEADER_GC_CHARACTER_DELETE_WRONG_SOCIAL_ID, 11);
        assert_eq!(GC_PLAYER_DELETE_SUCCESS_INDEX_OFFSET, 1);
        assert_eq!(GC_PLAYER_DELETE_SUCCESS_WIRE_SIZE, 2);
        assert_eq!(GC_PLAYER_DELETE_WRONG_SOCIAL_ID_WIRE_SIZE, 1);

        let success = GcPlayerDeleteSuccess::new(0xa5);
        assert_eq!(success.encode(), [0x0a, 0xa5]);
        assert_eq!(
            GcPlayerDeleteSuccess::decode(&success.encode()),
            Ok(success)
        );

        let wrong_social_id = GcPlayerDeleteWrongSocialId;
        assert_eq!(wrong_social_id.encode(), [0x0b]);
        assert_eq!(
            GcPlayerDeleteWrongSocialId::decode(&wrong_social_id.encode()),
            Ok(wrong_social_id)
        );
    }

    #[test]
    fn player_delete_success_round_trips_every_opaque_account_index() {
        for account_index in 0..=u8::MAX {
            let packet = GcPlayerDeleteSuccess::new(account_index);
            let encoded = packet.encode();
            assert_eq!(encoded, [0x0a, account_index]);
            assert_eq!(GcPlayerDeleteSuccess::decode(&encoded), Ok(packet));
        }
    }

    #[test]
    fn player_delete_results_reject_bad_lengths_before_headers() {
        for available in 0..GC_PLAYER_DELETE_SUCCESS_WIRE_SIZE {
            assert!(matches!(
                GcPlayerDeleteSuccess::decode(&vec![0x0b; available]),
                Err(GcPacketError::Truncated {
                    record: "TPacketGCDestroyCharacterSuccess",
                    needed: 2,
                    available: actual
                }) if actual == available
            ));
        }
        for actual in [3, 8, 16, 255] {
            assert!(matches!(
                GcPlayerDeleteSuccess::decode(&vec![0x0b; actual]),
                Err(GcPacketError::LengthMismatch {
                    record: "TPacketGCDestroyCharacterSuccess",
                    expected: 2,
                    actual: observed
                }) if observed == actual
            ));
        }

        assert!(matches!(
            GcPlayerDeleteWrongSocialId::decode(&[]),
            Err(GcPacketError::Truncated {
                record: "TPacketGCBlank",
                needed: 1,
                available: 0
            })
        ));
        for actual in [2, 8, 16, 255] {
            assert!(matches!(
                GcPlayerDeleteWrongSocialId::decode(&vec![0x0a; actual]),
                Err(GcPacketError::LengthMismatch {
                    record: "TPacketGCBlank",
                    expected: 1,
                    actual: observed
                }) if observed == actual
            ));
        }

        // Complete length is checked before the header for both record types.
        assert!(matches!(
            GcPlayerDeleteSuccess::decode(&[0x0b]),
            Err(GcPacketError::Truncated {
                record: "TPacketGCDestroyCharacterSuccess",
                needed: 2,
                available: 1
            })
        ));
    }

    #[test]
    fn player_delete_results_reject_exact_wrong_headers_and_cross_shapes() {
        for actual in [0, 7, 9, 11, 12, u8::MAX] {
            assert!(matches!(
                GcPlayerDeleteSuccess::decode(&[actual, 0]),
                Err(GcPacketError::InvalidHeader {
                    expected: HEADER_GC_PLAYER_DELETE_SUCCESS,
                    actual: header
                }) if header == actual
            ));
        }
        for actual in [0, 7, 8, 10, 12, u8::MAX] {
            assert!(matches!(
                GcPlayerDeleteWrongSocialId::decode(&[actual]),
                Err(GcPacketError::InvalidHeader {
                    expected: HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID,
                    actual: header
                }) if header == actual
            ));
        }

        assert!(matches!(
            GcPlayerDeleteSuccess::decode(&[0x0a]),
            Err(GcPacketError::Truncated {
                record: "TPacketGCDestroyCharacterSuccess",
                needed: 2,
                available: 1
            })
        ));
        assert!(matches!(
            GcPlayerDeleteWrongSocialId::decode(&[0x0b, 0]),
            Err(GcPacketError::LengthMismatch {
                record: "TPacketGCBlank",
                expected: 1,
                actual: 2
            })
        ));
    }

    #[test]
    fn login_success_has_source_exact_size_and_active_header() {
        assert_eq!(HEADER_GC_LOGIN_SUCCESS, 6);
        assert_eq!(HEADER_GC_LOGIN_SUCCESS_NEWSLOT, 32);
        assert_eq!(GC_LOGIN_SUCCESS_PLAYER_COUNT, 4);
        assert_eq!(GC_LOGIN_SUCCESS_PLAYER_WIRE_SIZE, 70);
        assert_eq!(GC_LOGIN_SUCCESS_GUILD_NAME_BYTES, 13);
        assert_eq!(GC_LOGIN_SUCCESS_WIRE_SIZE, 357);

        let mut first = sample_login_player(0x0102_0304, 0x11);
        first.name[..5].copy_from_slice(b"First");
        first.name[5] = 0;
        first.x = -123_456;
        first.y = 654_321;
        first.addr = 0x0100_007f;
        let mut second = sample_login_player(0x1112_1314, 0x22);
        second.name[..6].copy_from_slice(b"Secnd ");
        let third = sample_login_player(0x2122_2324, 0x33);
        let fourth = sample_login_player(0x3132_3334, 0x44);
        let players = [first, second, third, fourth];
        let guild_ids = [0x0101_0101, 0x0202_0202, 0x0303_0303, 0x0404_0404];
        let guild_names = [[b'A'; 13], [b'B'; 13], [b'C'; 13], [b'D'; 13]];
        let packet = GcLoginSuccess::new(players, guild_ids, guild_names, 0xa1b2_c3d4, 0x0102_0304);
        let encoded = packet.encode();
        // Independently transcribed source-order bytes for every nested field.
        let expected: [u8; GC_LOGIN_SUCCESS_WIRE_SIZE] = [
            0x20, 0x04, 0x03, 0x02, 0x01, 0x46, 0x69, 0x72, 0x73, 0x74, 0x00, 0x11, 0x11, 0x11,
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
            0x11, 0x11, 0x11, 0x12, 0x11, 0x11, 0x11, 0x11, 0x13, 0x14, 0x15, 0x16, 0x11, 0x11,
            0x17, 0x22, 0x22, 0x33, 0x33, 0x18, 0x18, 0x18, 0x18, 0xc0, 0x1d, 0xfe, 0xff, 0xf1,
            0xfb, 0x09, 0x00, 0x7f, 0x00, 0x00, 0x01, 0x44, 0x44, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
            0x1e, 0x14, 0x13, 0x12, 0x11, 0x53, 0x65, 0x63, 0x6e, 0x64, 0x00, 0x22, 0x22, 0x22,
            0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
            0x22, 0x22, 0x22, 0x23, 0x22, 0x22, 0x22, 0x22, 0x24, 0x25, 0x26, 0x27, 0x22, 0x22,
            0x28, 0x44, 0x44, 0x66, 0x66, 0x29, 0x29, 0x29, 0x29, 0xd0, 0x84, 0x00, 0x00, 0x60,
            0xf6, 0xfe, 0xff, 0x00, 0x00, 0x00, 0x22, 0x88, 0x88, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e,
            0x2f, 0x24, 0x23, 0x22, 0x21, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33,
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33,
            0x33, 0x33, 0x33, 0x34, 0x33, 0x33, 0x33, 0x33, 0x35, 0x36, 0x37, 0x38, 0x33, 0x33,
            0x39, 0x66, 0x66, 0x99, 0x99, 0x3a, 0x3a, 0x3a, 0x3a, 0x38, 0xc7, 0x00, 0x00, 0x90,
            0x71, 0xfe, 0xff, 0x00, 0x00, 0x00, 0x33, 0xcc, 0xcc, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
            0x40, 0x34, 0x33, 0x32, 0x31, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44,
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44,
            0x44, 0x44, 0x44, 0x45, 0x44, 0x44, 0x44, 0x44, 0x46, 0x47, 0x48, 0x49, 0x44, 0x44,
            0x4a, 0x88, 0x88, 0xcc, 0xcc, 0x4b, 0x4b, 0x4b, 0x4b, 0xa0, 0x09, 0x01, 0x00, 0xc0,
            0xec, 0xfd, 0xff, 0x00, 0x00, 0x00, 0x44, 0x10, 0x11, 0x4c, 0x4d, 0x4e, 0x4f, 0x50,
            0x51, 0x01, 0x01, 0x01, 0x01, 0x02, 0x02, 0x02, 0x02, 0x03, 0x03, 0x03, 0x03, 0x04,
            0x04, 0x04, 0x04, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41,
            0x41, 0x41, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
            0x42, 0x43, 0x43, 0x43, 0x43, 0x43, 0x43, 0x43, 0x43, 0x43, 0x43, 0x43, 0x43, 0x43,
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0xd4,
            0xc3, 0xb2, 0xa1, 0x04, 0x03, 0x02, 0x01,
        ];
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), GC_LOGIN_SUCCESS_WIRE_SIZE);
        assert_eq!(encoded[0], HEADER_GC_LOGIN_SUCCESS_NEWSLOT);
        assert_eq!(&encoded[1..5], &0x0102_0304u32.to_le_bytes());
        assert_eq!(&encoded[1 + 4..1 + 4 + 25], &first.name);
        assert_eq!(&encoded[1 + 29], &first.job);
        assert_eq!(&encoded[(1 + 31)..=35], &first.play_minutes.to_le_bytes());
        assert_eq!(&encoded[(1 + 39)..=41], &first.main_part.to_le_bytes());
        assert_eq!(&encoded[(1 + 44)..=46], &first.sash_part.to_le_bytes());
        assert_eq!(&encoded[(1 + 50)..=54], &first.x.to_le_bytes());
        assert_eq!(&encoded[(1 + 58)..=62], &first.addr.to_le_bytes());
        assert_eq!(&encoded[281..285], &guild_ids[0].to_le_bytes());
        assert_eq!(&encoded[297..310], &guild_names[0]);
        assert_eq!(&encoded[349..353], &0xa1b2_c3d4u32.to_le_bytes());
        assert_eq!(&encoded[353..357], &0x0102_0304u32.to_le_bytes());
        assert_eq!(GcLoginSuccess::decode(&encoded).unwrap(), packet);
    }

    #[test]
    fn login_success_round_trips_raw_names_and_extrema() {
        let mut players = [sample_login_player(0, 0); GC_LOGIN_SUCCESS_PLAYER_COUNT];
        for (index, player) in players.iter_mut().enumerate() {
            player.id = u32::MAX - u32::try_from(index).unwrap();
            player.name = [0xff; 25];
            player.name[7] = 0;
            player.dummy = [0x80, 0x00, 0xff, 0x7f];
            player.x = i32::MIN;
            player.y = i32::MAX;
            player.addr = -1;
            player.port = u16::MAX;
        }
        let guild_ids = [0, 1, u32::MAX, 0x8000_0000];
        let guild_names = [[0xff; 13]; GC_LOGIN_SUCCESS_PLAYER_COUNT];
        let packet = GcLoginSuccess::new(players, guild_ids, guild_names, u32::MAX, 0x8000_0000);
        let encoded = packet.encode();
        assert_eq!(encoded.len(), GC_LOGIN_SUCCESS_WIRE_SIZE);
        assert_eq!(GcLoginSuccess::decode(&encoded).unwrap(), packet);
    }

    #[test]
    fn login_success_rejects_bad_lengths_headers_and_feature_profiles() {
        for available in 0..GC_LOGIN_SUCCESS_WIRE_SIZE {
            assert!(matches!(
                GcLoginSuccess::decode(&vec![HEADER_GC_LOGIN_SUCCESS_NEWSLOT; available]),
                Err(GcPacketError::Truncated {
                    record: "TPacketGCLoginSuccess",
                    needed: GC_LOGIN_SUCCESS_WIRE_SIZE,
                    available: _
                })
            ));
        }

        let packet = GcLoginSuccess::new(
            [sample_login_player(0, 1); GC_LOGIN_SUCCESS_PLAYER_COUNT],
            [0; GC_LOGIN_SUCCESS_PLAYER_COUNT],
            [[0; 13]; GC_LOGIN_SUCCESS_PLAYER_COUNT],
            0,
            0,
        );
        let mut long = packet.encode();
        long.push(0);
        assert!(matches!(
            GcLoginSuccess::decode(&long),
            Err(GcPacketError::LengthMismatch {
                record: "TPacketGCLoginSuccess",
                expected: GC_LOGIN_SUCCESS_WIRE_SIZE,
                actual: 358
            })
        ));

        let mut wrong_header = packet.encode();
        wrong_header[0] = HEADER_GC_LOGIN_SUCCESS;
        assert!(matches!(
            GcLoginSuccess::decode(&wrong_header),
            Err(GcPacketError::InvalidHeader {
                expected: HEADER_GC_LOGIN_SUCCESS_NEWSLOT,
                actual: HEADER_GC_LOGIN_SUCCESS
            })
        ));

        let no_feature_profile = vec![HEADER_GC_LOGIN_SUCCESS_NEWSLOT; 329];
        assert!(matches!(
            GcLoginSuccess::decode(&no_feature_profile),
            Err(GcPacketError::Truncated {
                record: "TPacketGCLoginSuccess",
                needed: GC_LOGIN_SUCCESS_WIRE_SIZE,
                available: 329
            })
        ));
    }

    #[test]
    fn time_sync_is_exact_one_byte_blank_acknowledgement() {
        assert_eq!(HEADER_GC_TIME_SYNC, 0xfc);
        assert_eq!(GC_TIME_SYNC_WIRE_SIZE, 1);

        let packet = GcTimeSync::new();
        assert_eq!(packet.encode(), vec![0xfc]);
        assert_eq!(GcTimeSync::decode(&packet.encode()), Ok(packet));
        assert_eq!(packet, GcTimeSync);
    }

    #[test]
    fn time_sync_rejects_truncation_trailing_bytes_and_wrong_headers() {
        assert_eq!(
            GcTimeSync::decode(&[]),
            Err(GcPacketError::Truncated {
                record: "TPacketGCBlank",
                needed: 1,
                available: 0,
            })
        );
        assert_eq!(
            GcTimeSync::decode(&[0xfc, 0]),
            Err(GcPacketError::LengthMismatch {
                record: "TPacketGCBlank",
                expected: 1,
                actual: 2,
            })
        );
        assert_eq!(
            GcTimeSync::decode(&[0xfe]),
            Err(GcPacketError::InvalidHeader {
                expected: HEADER_GC_TIME_SYNC,
                actual: 0xfe,
            })
        );
    }

    #[test]
    fn gc_time_sync_header_is_not_the_inbound_cg_time_sync_record() {
        assert_eq!(
            crate::cg_inventory::HEADER_CG_TIME_SYNC.value(),
            HEADER_GC_TIME_SYNC
        );
        assert_ne!(106, HEADER_GC_TIME_SYNC);
        let inbound = crate::cg_handshake::CgInboundHandshake::new(
            crate::cg_handshake::CgHandshakeHeader::TimeSync,
            1,
            2,
            3,
        );
        assert_eq!(
            inbound.encode().len(),
            crate::cg_handshake::CG_HANDSHAKE_WIRE_SIZE
        );
        assert_ne!(inbound.encode().len(), GC_TIME_SYNC_WIRE_SIZE);
        assert!(GcTimeSync::decode(&inbound.encode()).is_err());
        assert!(GcTimeSync::decode(&[106, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn bind_udp_constants_and_raw_network_bytes_are_stable() {
        assert_eq!(HEADER_GC_BINDUDP, 0xfe);
        assert_eq!(GC_BINDUDP_WIRE_SIZE, 7);

        let packet = GcBindUdp::new(0x0102_00c0, 0x901f);
        assert_eq!(
            packet.encode(),
            vec![0xfe, 0xc0, 0x00, 0x02, 0x01, 0x1f, 0x90]
        );
        assert_eq!(GcBindUdp::decode(&packet.encode()).unwrap(), packet);

        let port_1234 = GcBindUdp::new(0x0102_00c0, 0xd204);
        assert_eq!(
            port_1234.encode(),
            vec![0xfe, 0xc0, 0x00, 0x02, 0x01, 0x04, 0xd2]
        );
        let alias: GcBindUDP = port_1234;
        assert_eq!(GcBindUdp::decode(&alias.encode()).unwrap(), alias);
    }

    #[test]
    fn bind_udp_round_trips_raw_extrema_and_zero_values() {
        for addr in [0, 1, 0x0102_00c0, u32::MAX] {
            for port in [0, 1, 0x901f, u16::MAX] {
                let packet = GcBindUdp::new(addr, port);
                assert_eq!(packet.encode().len(), GC_BINDUDP_WIRE_SIZE);
                assert_eq!(GcBindUdp::decode(&packet.encode()).unwrap(), packet);
            }
        }
    }

    #[test]
    fn bind_udp_rejects_truncation_trailing_bytes_and_wrong_header() {
        for available in 0..GC_BINDUDP_WIRE_SIZE {
            assert!(matches!(
                GcBindUdp::decode(&vec![0xff; available]),
                Err(GcPacketError::Truncated {
                    record: "TPacketGCBindUDP",
                    needed: 7,
                    available: _
                }) if available < 7
            ));
        }

        let mut long = GcBindUdp::new(0, 0).encode();
        long.push(0);
        assert!(matches!(
            GcBindUdp::decode(&long),
            Err(GcPacketError::LengthMismatch {
                record: "TPacketGCBindUDP",
                expected: 7,
                actual: 8
            })
        ));

        assert_eq!(
            GcBindUdp::decode(&[0xff, 0, 0, 0, 0, 0, 0]),
            Err(GcPacketError::InvalidHeader {
                expected: HEADER_GC_BINDUDP,
                actual: 0xff
            })
        );
    }
}
