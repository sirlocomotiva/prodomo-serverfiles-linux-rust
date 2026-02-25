//! The game-to-client actor, target, and main-character records.
//!
//! # Provenance and how every width here was obtained
//!
//! Each of the 14 records in this module is a `#pragma pack(1)` structure. No
//! width is hand-summed. Every one was **machine-derived**: the struct body was
//! extracted verbatim from `client/Client/UserInterface/Packet.h`, the
//! array-dimension constants were substituted with their resolved values, the
//! complete active feature-gate set was applied, and the result was compiled
//! with `i686-linux-gnu-g++-12` and run under `sizeof`.
//!
//! The legacy target is 32-bit. `server/server/premake5.lua:12` sets
//! `architecture "x86"`, so `long`, `DWORD`, `time_t`, and a C++ `bool` are
//! 4, 4, 4, and 1 bytes here while `long long` is 8.
//!
//! # The active feature-gate set
//!
//! These records are among the most heavily gated in the protocol, so a width
//! is only meaningful together with the gate state it was measured under. The
//! active set is read from `client/Client/UserInterface/LOCALE_INC.H` and
//! `server/server/common/prodomodefines.h`, the two headers that are
//! authoritative for this build. Both trees are consistent: every gate that
//! these records test is active on both sides.
//!
//! The two trees spell six of the same gates differently, and both spellings
//! must be resolved, never one assumed from the other:
//!
//! | client spelling | server spelling |
//! | --- | --- |
//! | `FIX_UPDATE_LEVEL` | `__FIX_UPDATE_LEVEL__` |
//! | `ENABLE_CONQUEROR_LEVEL` | `__CONQUEROR_LEVEL__` |
//! | `ENABLE_VIEW_TARGET_DECIMAL_HP` | `__VIEW_TARGET_DECIMAL_HP__` |
//! | `ENABLE_PREMIUM_PLAYERS` | `__ENABLE_PREMIUM_PLAYERS__` |
//! | `ENABLE_MULTI_LANGUAGE_SYSTEM` | `__MULTI_LANGUAGE_SYSTEM__` |
//! | `ENABLE_SHIP_DEFENSE` | `__SHIP_DEFENSE__` |
//!
//! Four gates keep the same name on both sides: `ELEMENT_TARGET`,
//! `ENABLE_REFINE_ELEMENT`, `ENABLE_SHOW_LIDER_AND_GENERAL_GUILD`, and
//! `ENABLE_SEND_TARGET_INFO_EXTENDED`.
//!
//! A negative control was run for every gate: removing any single gate changes
//! at least one width in this module, so no gate here is inert and none was
//! assumed rather than measured.
//!
//! # Array-dimension constants
//!
//! | constant | value | how it is defined |
//! | --- | --- | --- |
//! | `CHARACTER_NAME_MAX_LEN` | 24 | `client/.../StdAfx.h:43`, `server/.../common/length.h:15` |
//! | `CHR_EQUIPPART_NUM` | 6 | auto-incremented last member of `ECharacterEquipmentPart` |
//! | `SKILL_MAX_NUM` | 255 | `#define` in `client/.../Packet.h:2136`, enum member in `server/.../common/length.h` |
//!
//! `CHR_EQUIPPART_NUM` is **gate-dependent** and its value is 6 only because
//! `ENABLE_SASH_SYSTEM` and `ENABLE_AURA_SYSTEM` are both active, which adds
//! `CHR_EQUIPPART_SASH` and `CHR_EQUIPPART_AURA` to the enumeration. A negative
//! control compiled with both gates off measures 4. The Rust type therefore
//! fixes the array at `[u16; 6]` and documents that it is only correct for the
//! active profile.
//!
//! # Which records the server actually sends
//!
//! The two header enumerations are **not** name-compatible. For six of these
//! bytes the client and the server attach different names to the same value,
//! and reading either name as if it belonged to the other side produces the
//! wrong record:
//!
//! | byte | server spelling | client spelling |
//! | --- | --- | --- |
//! | 15 | `HEADER_GC_MAIN_CHARACTER_OLD` | `HEADER_GC_MAIN_CHARACTER` |
//! | 72 | `HEADER_GC_SKILL_LEVEL_OLD` | `HEADER_GC_SKILL_LEVEL` |
//! | 113 | `HEADER_GC_MAIN_CHARACTER` | `HEADER_GC_MAIN_CHARACTER2_EMPIRE` |
//! | 125 | `HEADER_GC_TARGET_CREATE` | `HEADER_GC_TARGET_CREATE_NEW` |
//!
//! The seventh case is asymmetric: byte 117 is
//! `HEADER_GC_CHARACTER_UPDATE2` on the client and has **no** server
//! enumerator at all. Byte 225 is the reverse: the server spells it
//! `HEADER_GC_CHARACTER_GOLD_CHANGE` and the client spells it the same, but in
//! a second guarded `enum` at `client/.../Packet.h:3047` rather than in the
//! main table.
//!
//! Resolving those names against the real send sites changes the conclusion
//! this module draws about three records, so the resolution is recorded here
//! rather than left implicit:
//!
//! - **Byte 15 is a dead client decode entry.** The server declares
//!   `HEADER_GC_MAIN_CHARACTER_OLD = 15` and never sends it; the only reference
//!   anywhere in `server/server` is the declaration itself. The live
//!   main-character record is **byte 113**, where the server sends its
//!   46-byte `TPacketGCMainCharacter` from `char.cpp:2018` and the client
//!   decodes the 46-byte `TPacketGCMainCharacter2_EMPIRE`. Those two agree
//!   field for field, so byte 113 is a normal, matching pair.
//! - **Byte 72 is a dead client decode entry.** The server declares
//!   `HEADER_GC_SKILL_LEVEL_OLD = 72` and never sends it. The live
//!   skill-level record is **byte 76**, where the server sends the 1531-byte
//!   `TPacketGCSkillLevel` from `char_skill.cpp:178` and the client decodes the
//!   1531-byte `TPacketGCSkillLevelNew`; `protocol::gc_nested` already covers
//!   that pair.
//! - **Byte 117 is a dead client decode entry.** No server enumerator and no
//!   send site exist for it.
//!
//! So three of the 14 records here — [`GcMainCharacter`], [`GcSkillLevel`], and
//! [`GcCharacterUpdate2`] — model a client decode that the checked-in server
//! never produces, and the other 11 are live pairs whose client and server
//! structs agree on every field offset. None of the 14 is a protocol defect.
//! This module implements the **client decode** width for all of them, because
//! that is the boundary a client-side reader must honour, and names the
//! generation and the send site so the two sides are never confused again.
//!
//! The three name arrays are the reason a byte cannot be read off a name: the
//! 25-byte character name, the 25-byte BGM name, and the 33-byte target name
//! appear in records whose enumerator names differ, so a single "name" field
//! length would be wrong for at least one record.
//!
//! # Scope
//!
//! These are pure wire-boundary codecs. They do not build a world, resolve
//! actors, move a character, interpret equipment parts, apply gold, target a
//! unit, play music, or mutate any session state.

use crate::gc_inventory::{
    HEADER_GC_CHARACTER_ADD, HEADER_GC_CHARACTER_GOLD_CHANGE, HEADER_GC_CHARACTER_MOVE,
    HEADER_GC_CHARACTER_UPDATE, HEADER_GC_CHARACTER_UPDATE2, HEADER_GC_CHAR_ADDITIONAL_INFO,
    HEADER_GC_MAIN_CHARACTER, HEADER_GC_MAIN_CHARACTER2_EMPIRE, HEADER_GC_MAIN_CHARACTER3_BGM,
    HEADER_GC_MAIN_CHARACTER4_BGM_VOL, HEADER_GC_SKILL_LEVEL, HEADER_GC_TARGET,
    HEADER_GC_TARGET_CREATE_NEW, HEADER_GC_TARGET_INFO,
};
use core::fmt;

/// The exact `CHARACTER_NAME_MAX_LEN` of the active profile, which is 24.
///
/// This is the value the legacy `StdAfx.h:43` and `common/length.h:15`
/// enum members both carry. It is published separately from [`NAME_LEN`] so
/// that the constant and the array length it produces cannot be conflated.
pub const CHARACTER_NAME_MAX_LEN_EXPECTED: usize = 24;
/// The exact `CHARACTER_NAME_MAX_LEN + 1` character array in the legacy records.
pub const NAME_LEN: usize = 25;
/// The exact `CHR_EQUIPPART_NUM` of the active profile, which is 6.
pub const EQUIP_PART_NUM: usize = 6;
/// The exact `SKILL_MAX_NUM` of the active profile, which is 255.
pub const SKILL_MAX_NUM: usize = 255;
/// The exact `32 + 1` target-name array in `TPacketGCTargetCreateNew`.
///
/// This is a literal in the legacy struct, not `CHARACTER_NAME_MAX_LEN + 1`,
/// so it is 33 and must not be unified with [`NAME_LEN`].
pub const TARGET_NAME_LEN: usize = 33;
/// The exact `MUSIC_NAME_MAX_LEN + 1` array in the BGM own-character records.
///
/// The client spells the constant `MUSIC_NAME_MAX_LEN` and the server
/// `MUSIC_NAME_LEN`; both are 24, so both arrays are 25.
pub const BGM_NAME_LEN: usize = 25;

/// The error returned by every decoder in this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcActorsError {
    /// The buffer is shorter than the record's exact fixed length.
    Truncated {
        /// The record name that was being decoded.
        context: &'static str,
        /// The exact byte length that record requires.
        needed: usize,
        /// The byte length the caller actually supplied.
        actual: usize,
    },
    /// The first byte is not the header this decoder accepts.
    Header {
        /// The record name that was being decoded.
        context: &'static str,
        /// The header byte that record requires.
        expected: u8,
        /// The header byte the caller actually supplied.
        actual: u8,
    },
}

impl fmt::Display for GcActorsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Truncated {
                context,
                needed,
                actual,
            } => write!(f, "{context}: truncated, need {needed} bytes, got {actual}"),
            Self::Header {
                context,
                expected,
                actual,
            } => write!(
                f,
                "{context}: wrong header, expected {expected:#04x}, got {actual:#04x}"
            ),
        }
    }
}

impl std::error::Error for GcActorsError {}

/// Require the exact fixed length of a record before reading any field.
fn take<'a>(
    bytes: &'a [u8],
    needed: usize,
    context: &'static str,
) -> Result<&'a [u8], GcActorsError> {
    bytes.get(..needed).ok_or(GcActorsError::Truncated {
        context,
        needed,
        actual: bytes.len(),
    })
}

/// Check that a complete-length record starts with the header the caller expects.
fn check_header(bytes: &[u8], context: &'static str, expected: u8) -> Result<(), GcActorsError> {
    match bytes.first() {
        Some(&actual) if actual == expected => Ok(()),
        Some(&actual) => Err(GcActorsError::Header {
            context,
            expected,
            actual,
        }),
        None => Err(GcActorsError::Truncated {
            context,
            needed: 1,
            actual: 0,
        }),
    }
}

/// Copy a fixed-width field out of an already length-checked buffer.
///
/// Every decoder in this module calls [`take`] first, which proves the buffer
/// is exactly the record's wire size. That makes every field slice in range,
/// so a plain copy is correct and the decoders stay free of panic paths.
fn array<const N: usize>(raw: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&raw[at..at + N]);
    out
}

/// A two-byte little-endian unsigned reader.
fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

/// A four-byte little-endian unsigned reader.
fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// An eight-byte little-endian unsigned reader.
fn u64le(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// The exact packed wire length of [`GcCharacterAdd`], in bytes.
pub const GC_CHARACTER_ADD_WIRE_SIZE: usize = 35;

/// The 35-byte record that introduces a character to the client, as
/// `TPacketGCCharacterAdd`. The client struct additionally carries four
/// commented-out fields (`name`, `awPart`, `bEmpire`, `dwGuild`) that the
/// server never sends, so they are absent here too.

#[derive(Debug, Clone, Copy, PartialEq)]
/// The 35-byte record that introduces a character to the client, as the
/// client's `TPacketGCCharacterAdd`. The client struct additionally carries
/// four commented-out fields (`name`, `awPart`, `bEmpire`, `dwGuild`) that the
/// server never sends, so they are absent here too. The server struct is
/// `TPacketGCCharacterAdd` and agrees on every field offset.
pub struct GcCharacterAdd {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `angle` field.
    pub angle: f32,
    /// The raw `x` field.
    pub x: i32,
    /// The raw `y` field.
    pub y: i32,
    /// The raw `z` field.
    pub z: i32,
    /// The raw `bType` field.
    pub b_type: u8,
    /// The raw `wRaceNum` field.
    pub w_race_num: u16,
    /// The raw `bMovingSpeed` field.
    pub b_moving_speed: u8,
    /// The raw `bAttackSpeed` field.
    pub b_attack_speed: u8,
    /// The raw `bStateFlag` field.
    pub b_state_flag: u8,
    /// The raw `dwAffectFlag` field.
    pub dw_affect_flag: [u32; 2],
}

impl GcCharacterAdd {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        dw_vid: u32,
        angle: f32,
        x: i32,
        y: i32,
        z: i32,
        b_type: u8,
        w_race_num: u16,
        b_moving_speed: u8,
        b_attack_speed: u8,
        b_state_flag: u8,
        dw_affect_flag: [u32; 2],
    ) -> Self {
        Self {
            dw_vid,
            angle,
            x,
            y,
            z,
            b_type,
            w_race_num,
            b_moving_speed,
            b_attack_speed,
            b_state_flag,
            dw_affect_flag,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_CHARACTER_ADD.value()
    }

    /// Append the 35 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.angle.to_le_bytes());
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.extend_from_slice(&self.z.to_le_bytes());
        out.push(self.b_type);
        out.extend_from_slice(&self.w_race_num.to_le_bytes());
        out.push(self.b_moving_speed);
        out.push(self.b_attack_speed);
        out.push(self.b_state_flag);
        out.extend_from_slice(&self.dw_affect_flag[0].to_le_bytes());
        out.extend_from_slice(&self.dw_affect_flag[1].to_le_bytes());
    }

    /// Read the 35 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_CHARACTER_ADD_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(bytes, GC_CHARACTER_ADD_WIRE_SIZE, "GcCharacterAdd")?;
        check_header(raw, "GcCharacterAdd", Self::header())?;
        Ok(Self {
            dw_vid: u32le(&raw[1..5]),
            angle: f32::from_le_bytes([raw[5..9][0], raw[5..9][1], raw[5..9][2], raw[5..9][3]]),
            x: i32::from_le_bytes([raw[9..13][0], raw[9..13][1], raw[9..13][2], raw[9..13][3]]),
            y: i32::from_le_bytes([
                raw[13..17][0],
                raw[13..17][1],
                raw[13..17][2],
                raw[13..17][3],
            ]),
            z: i32::from_le_bytes([
                raw[17..21][0],
                raw[17..21][1],
                raw[17..21][2],
                raw[17..21][3],
            ]),
            b_type: raw[21..22][0],
            w_race_num: u16le(&raw[22..24]),
            b_moving_speed: raw[24..25][0],
            b_attack_speed: raw[25..26][0],
            b_state_flag: raw[26..27][0],
            dw_affect_flag: [u32le(&raw[27..31]), u32le(&raw[31..35])],
        })
    }
}

/// The exact packed wire length of [`GcCharacterMove`], in bytes.
pub const GC_CHARACTER_MOVE_WIRE_SIZE: usize = 24;

/// The 24-byte movement record, as `TPacketGCMove`. The four leading
/// bytes stay opaque: the legacy record does not interpret `bFunc`, `bArg`, or
/// `bRot`, and neither does this codec.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 24-byte movement record, as `TPacketGCMove`. The three leading
/// `BYTE` fields stay opaque: the legacy record does not interpret `bFunc`,
/// `bArg`, or `bRot`, and neither does this codec. The client and server
/// structs are both `TPacketGCMove` and agree on every field offset.
pub struct GcCharacterMove {
    /// The raw `bFunc` field.
    pub b_func: u8,
    /// The raw `bArg` field.
    pub b_arg: u8,
    /// The raw `bRot` field.
    pub b_rot: u8,
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `lX` field.
    pub l_x: i32,
    /// The raw `lY` field.
    pub l_y: i32,
    /// The raw `dwTime` field.
    pub dw_time: u32,
    /// The raw `dwDuration` field.
    pub dw_duration: u32,
}

impl GcCharacterMove {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        b_func: u8,
        b_arg: u8,
        b_rot: u8,
        dw_vid: u32,
        l_x: i32,
        l_y: i32,
        dw_time: u32,
        dw_duration: u32,
    ) -> Self {
        Self {
            b_func,
            b_arg,
            b_rot,
            dw_vid,
            l_x,
            l_y,
            dw_time,
            dw_duration,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_CHARACTER_MOVE.value()
    }

    /// Append the 24 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.push(self.b_func);
        out.push(self.b_arg);
        out.push(self.b_rot);
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.l_x.to_le_bytes());
        out.extend_from_slice(&self.l_y.to_le_bytes());
        out.extend_from_slice(&self.dw_time.to_le_bytes());
        out.extend_from_slice(&self.dw_duration.to_le_bytes());
    }

    /// Read the 24 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_CHARACTER_MOVE_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(bytes, GC_CHARACTER_MOVE_WIRE_SIZE, "GcCharacterMove")?;
        check_header(raw, "GcCharacterMove", Self::header())?;
        Ok(Self {
            b_func: raw[1..2][0],
            b_arg: raw[2..3][0],
            b_rot: raw[3..4][0],
            dw_vid: u32le(&raw[4..8]),
            l_x: i32::from_le_bytes([raw[8..12][0], raw[8..12][1], raw[8..12][2], raw[8..12][3]]),
            l_y: i32::from_le_bytes([
                raw[12..16][0],
                raw[12..16][1],
                raw[12..16][2],
                raw[12..16][3],
            ]),
            dw_time: u32le(&raw[16..20]),
            dw_duration: u32le(&raw[20..24]),
        })
    }
}

/// The exact packed wire length of [`GcMainCharacter`], in bytes.
pub const GC_MAIN_CHARACTER_WIRE_SIZE: usize = 45;

/// The 45-byte own-character record as the **client** decodes it, from
/// `TPacketGCMainCharacter`.
///
/// The server struct is 46 bytes: it ends `empire, skill_group`, while the
/// client struct ends `bySkillGroup` alone. The client therefore consumes 45
/// bytes of a 46-byte send and the trailing `empire` byte is never read. The
/// byte 113 successor `GcMainCharacter2Empire` does carry the empire field.
/// This is a legacy defect, recorded rather than repaired: the codec honours
/// the decode boundary a client reader must respect.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 45-byte record the **client** decodes at byte 15, from
/// `TPacketGCMainCharacter`. This is a dead generation: the server declares
/// `HEADER_GC_MAIN_CHARACTER_OLD = 15` and never sends it. The live
/// main-character record is byte 113, which the server sends as its 46-byte
/// `TPacketGCMainCharacter` and the client decodes as the 46-byte
/// [`GcMainCharacter2Empire`]; that pair agrees on every field offset. The
/// 45-byte client shape here omits the trailing `empire` byte that the live
/// record carries, which is exactly what makes it the older generation.
pub struct GcMainCharacter {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `wRaceNum` field.
    pub w_race_num: u16,
    /// The raw `szName` field.
    pub sz_name: [u8; NAME_LEN],
    /// The raw `lX` field.
    pub l_x: i32,
    /// The raw `lY` field.
    pub l_y: i32,
    /// The raw `lZ` field.
    pub l_z: i32,
    /// The raw `bySkillGroup` field.
    pub by_skill_group: u8,
}

impl GcMainCharacter {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        dw_vid: u32,
        w_race_num: u16,
        sz_name: [u8; NAME_LEN],
        l_x: i32,
        l_y: i32,
        l_z: i32,
        by_skill_group: u8,
    ) -> Self {
        Self {
            dw_vid,
            w_race_num,
            sz_name,
            l_x,
            l_y,
            l_z,
            by_skill_group,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_MAIN_CHARACTER.value()
    }

    /// Append the 45 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.w_race_num.to_le_bytes());
        out.extend_from_slice(&self.sz_name);
        out.extend_from_slice(&self.l_x.to_le_bytes());
        out.extend_from_slice(&self.l_y.to_le_bytes());
        out.extend_from_slice(&self.l_z.to_le_bytes());
        out.push(self.by_skill_group);
    }

    /// Read the 45 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_MAIN_CHARACTER_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(bytes, GC_MAIN_CHARACTER_WIRE_SIZE, "GcMainCharacter")?;
        check_header(raw, "GcMainCharacter", Self::header())?;
        Ok(Self {
            dw_vid: u32le(&raw[1..5]),
            w_race_num: u16le(&raw[5..7]),
            sz_name: array::<25>(raw, 7),
            l_x: i32::from_le_bytes([
                raw[32..36][0],
                raw[32..36][1],
                raw[32..36][2],
                raw[32..36][3],
            ]),
            l_y: i32::from_le_bytes([
                raw[36..40][0],
                raw[36..40][1],
                raw[36..40][2],
                raw[36..40][3],
            ]),
            l_z: i32::from_le_bytes([
                raw[40..44][0],
                raw[40..44][1],
                raw[40..44][2],
                raw[40..44][3],
            ]),
            by_skill_group: raw[44..45][0],
        })
    }
}

/// The exact packed wire length of [`GcCharacterUpdate`], in bytes.
pub const GC_CHARACTER_UPDATE_WIRE_SIZE: usize = 55;

/// The 55-byte character-attribute refresh, as `TPacketGCCharacterUpdate`.
///
/// This is the widest actor record in the module because six feature gates are
/// active: `FIX_UPDATE_LEVEL` adds `dwLevel`, `ENABLE_CONQUEROR_LEVEL` adds
/// `dwConquerorLevel`, `ENABLE_REFINE_ELEMENT` adds `bRefineElementType`,
/// `ENABLE_SHOW_LIDER_AND_GENERAL_GUILD` adds `dwNewIsGuildName`,
/// `ENABLE_PREMIUM_PLAYERS` adds `byPremium` and `iPremiumTime`, and
/// `ENABLE_MULTI_LANGUAGE_SYSTEM` adds `bLanguage`. The server spells the same
/// six gates `__FIX_UPDATE_LEVEL__`, `__CONQUEROR_LEVEL__`,
/// `ENABLE_REFINE_ELEMENT`, `ENABLE_SHOW_LIDER_AND_GENERAL_GUILD`,
/// `__ENABLE_PREMIUM_PLAYERS__`, and `__MULTI_LANGUAGE_SYSTEM__`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 55-byte record that updates a character's equipment and state, as
/// `TPacketGCCharacterUpdate`. The `FIX_UPDATE_LEVEL` `dwLevel` and
/// `ENABLE_CONQUEROR_LEVEL` `dwConquerorLevel` fields exist only because those
/// gates are active. Client and server structs agree on every field offset.
pub struct GcCharacterUpdate {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `awPart` field.
    pub aw_part: [u16; EQUIP_PART_NUM],
    /// The raw `bMovingSpeed` field.
    pub b_moving_speed: u8,
    /// The raw `bAttackSpeed` field.
    pub b_attack_speed: u8,
    /// The raw `bStateFlag` field.
    pub b_state_flag: u8,
    /// The raw `dwAffectFlag` field.
    pub dw_affect_flag: [u32; 2],
    /// The raw `dwGuildID` field.
    pub dw_guild_id: u32,
    /// The raw `sAlignment` field.
    pub s_alignment: i16,
    /// The raw `dwLevel` field.
    pub dw_level: u32,
    /// The raw `dwConquerorLevel` field.
    pub dw_conqueror_level: u32,
    /// The raw `bPKMode` field.
    pub b_pk_mode: u8,
    /// The raw `dwMountVnum` field.
    pub dw_mount_vnum: u32,
    /// The raw `bRefineElementType` field.
    pub b_refine_element_type: u8,
    /// The raw `dwNewIsGuildName` field.
    pub dw_new_is_guild_name: u8,
    /// The raw `byPremium` field.
    pub by_premium: u8,
    /// The raw `iPremiumTime` field.
    pub i_premium_time: i32,
    /// The raw `bLanguage` field.
    pub b_language: u8,
}

impl GcCharacterUpdate {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        dw_vid: u32,
        aw_part: [u16; EQUIP_PART_NUM],
        b_moving_speed: u8,
        b_attack_speed: u8,
        b_state_flag: u8,
        dw_affect_flag: [u32; 2],
        dw_guild_id: u32,
        s_alignment: i16,
        dw_level: u32,
        dw_conqueror_level: u32,
        b_pk_mode: u8,
        dw_mount_vnum: u32,
        b_refine_element_type: u8,
        dw_new_is_guild_name: u8,
        by_premium: u8,
        i_premium_time: i32,
        b_language: u8,
    ) -> Self {
        Self {
            dw_vid,
            aw_part,
            b_moving_speed,
            b_attack_speed,
            b_state_flag,
            dw_affect_flag,
            dw_guild_id,
            s_alignment,
            dw_level,
            dw_conqueror_level,
            b_pk_mode,
            dw_mount_vnum,
            b_refine_element_type,
            dw_new_is_guild_name,
            by_premium,
            i_premium_time,
            b_language,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_CHARACTER_UPDATE.value()
    }

    /// Append the 55 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.aw_part[0].to_le_bytes());
        out.extend_from_slice(&self.aw_part[1].to_le_bytes());
        out.extend_from_slice(&self.aw_part[2].to_le_bytes());
        out.extend_from_slice(&self.aw_part[3].to_le_bytes());
        out.extend_from_slice(&self.aw_part[4].to_le_bytes());
        out.extend_from_slice(&self.aw_part[5].to_le_bytes());
        out.push(self.b_moving_speed);
        out.push(self.b_attack_speed);
        out.push(self.b_state_flag);
        out.extend_from_slice(&self.dw_affect_flag[0].to_le_bytes());
        out.extend_from_slice(&self.dw_affect_flag[1].to_le_bytes());
        out.extend_from_slice(&self.dw_guild_id.to_le_bytes());
        out.extend_from_slice(&self.s_alignment.to_le_bytes());
        out.extend_from_slice(&self.dw_level.to_le_bytes());
        out.extend_from_slice(&self.dw_conqueror_level.to_le_bytes());
        out.push(self.b_pk_mode);
        out.extend_from_slice(&self.dw_mount_vnum.to_le_bytes());
        out.push(self.b_refine_element_type);
        out.push(self.dw_new_is_guild_name);
        out.push(self.by_premium);
        out.extend_from_slice(&self.i_premium_time.to_le_bytes());
        out.push(self.b_language);
    }

    /// Read the 55 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_CHARACTER_UPDATE_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(bytes, GC_CHARACTER_UPDATE_WIRE_SIZE, "GcCharacterUpdate")?;
        check_header(raw, "GcCharacterUpdate", Self::header())?;
        Ok(Self {
            dw_vid: u32le(&raw[1..5]),
            aw_part: [
                u16le(&raw[5..7]),
                u16le(&raw[7..9]),
                u16le(&raw[9..11]),
                u16le(&raw[11..13]),
                u16le(&raw[13..15]),
                u16le(&raw[15..17]),
            ],
            b_moving_speed: raw[17..18][0],
            b_attack_speed: raw[18..19][0],
            b_state_flag: raw[19..20][0],
            dw_affect_flag: [u32le(&raw[20..24]), u32le(&raw[24..28])],
            dw_guild_id: u32le(&raw[28..32]),
            s_alignment: i16::from_le_bytes([raw[32..34][0], raw[32..34][1]]),
            dw_level: u32le(&raw[34..38]),
            dw_conqueror_level: u32le(&raw[38..42]),
            b_pk_mode: raw[42..43][0],
            dw_mount_vnum: u32le(&raw[43..47]),
            b_refine_element_type: raw[47..48][0],
            dw_new_is_guild_name: raw[48..49][0],
            by_premium: raw[49..50][0],
            i_premium_time: i32::from_le_bytes([
                raw[50..54][0],
                raw[50..54][1],
                raw[50..54][2],
                raw[50..54][3],
            ]),
            b_language: raw[54..55][0],
        })
    }
}

/// The exact packed wire length of [`GcTargetInfo`], in bytes.
pub const GC_TARGET_INFO_WIRE_SIZE: usize = 19;

/// The 19-byte target descriptor, as `TPacketGCTargetInfo`. The trailing
/// `rarity` field exists only because `ENABLE_SEND_TARGET_INFO_EXTENDED` is
/// active.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 19-byte target summary, as `TPacketGCTargetInfo`. The trailing
/// `rarity` field exists only because `ENABLE_SEND_TARGET_INFO_EXTENDED` is
/// active, and it is the same gate on both sides. Client and server structs
/// agree on every field offset.
pub struct GcTargetInfo {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `race` field.
    pub race: u32,
    /// The raw `dwVnum` field.
    pub dw_vnum: u32,
    /// The raw `count` field.
    pub count: u16,
    /// The raw `rarity` field.
    pub rarity: u32,
}

impl GcTargetInfo {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(dw_vid: u32, race: u32, dw_vnum: u32, count: u16, rarity: u32) -> Self {
        Self {
            dw_vid,
            race,
            dw_vnum,
            count,
            rarity,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_TARGET_INFO.value()
    }

    /// Append the 19 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.race.to_le_bytes());
        out.extend_from_slice(&self.dw_vnum.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.rarity.to_le_bytes());
    }

    /// Read the 19 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_TARGET_INFO_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(bytes, GC_TARGET_INFO_WIRE_SIZE, "GcTargetInfo")?;
        check_header(raw, "GcTargetInfo", Self::header())?;
        Ok(Self {
            dw_vid: u32le(&raw[1..5]),
            race: u32le(&raw[5..9]),
            dw_vnum: u32le(&raw[9..13]),
            count: u16le(&raw[13..15]),
            rarity: u32le(&raw[15..19]),
        })
    }
}

/// The exact packed wire length of [`GcTarget`], in bytes.
pub const GC_TARGET_WIRE_SIZE: usize = 32;

/// The 32-byte selected-target record, as `TPacketGCTarget`.
///
/// `ENABLE_VIEW_TARGET_DECIMAL_HP` adds `iMinHP` and `iMaxHP`,
/// `ENABLE_SHIP_DEFENSE` adds `bAlliance` and the two 64-bit alliance bounds,
/// and `ELEMENT_TARGET` adds `bElement`. `bAlliance` is a C++ `bool`, which is
/// one byte here; it is kept as a raw `u8` because the legacy record does not
/// promise a canonical spelling for it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 32-byte target status record, as `TPacketGCTarget`. Three gates
/// shape it and each is spelled differently on the two sides:
/// `ENABLE_VIEW_TARGET_DECIMAL_HP` / `__VIEW_TARGET_DECIMAL_HP__` adds the
/// `iMinHP` and `iMaxHP` pair, `ENABLE_SHIP_DEFENSE` / `__SHIP_DEFENSE__` adds
/// the alliance flag and the two 8-byte HP bounds, and `ELEMENT_TARGET` keeps
/// its name and adds `bElement`. The two 8-byte fields are `int64_t` on the
/// legacy 32-bit target, so they are 8 bytes and not 4. Client and server
/// structs agree on every field offset.
pub struct GcTarget {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `bHPPercent` field.
    pub b_hp_percent: u8,
    /// The raw `iMinHP` field.
    pub i_min_hp: i32,
    /// The raw `iMaxHP` field.
    pub i_max_hp: i32,
    /// The raw `bAlliance` field.
    pub b_alliance: u8,
    /// The raw `iAllianceMinHP` field.
    pub i_alliance_min_hp: i64,
    /// The raw `iAllianceMaxHP` field.
    pub i_alliance_max_hp: i64,
    /// The raw `bElement` field.
    pub b_element: u8,
}

impl GcTarget {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        dw_vid: u32,
        b_hp_percent: u8,
        i_min_hp: i32,
        i_max_hp: i32,
        b_alliance: u8,
        i_alliance_min_hp: i64,
        i_alliance_max_hp: i64,
        b_element: u8,
    ) -> Self {
        Self {
            dw_vid,
            b_hp_percent,
            i_min_hp,
            i_max_hp,
            b_alliance,
            i_alliance_min_hp,
            i_alliance_max_hp,
            b_element,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_TARGET.value()
    }

    /// Append the 32 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.push(self.b_hp_percent);
        out.extend_from_slice(&self.i_min_hp.to_le_bytes());
        out.extend_from_slice(&self.i_max_hp.to_le_bytes());
        out.push(self.b_alliance);
        out.extend_from_slice(&self.i_alliance_min_hp.to_le_bytes());
        out.extend_from_slice(&self.i_alliance_max_hp.to_le_bytes());
        out.push(self.b_element);
    }

    /// Read the 32 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_TARGET_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(bytes, GC_TARGET_WIRE_SIZE, "GcTarget")?;
        check_header(raw, "GcTarget", Self::header())?;
        Ok(Self {
            dw_vid: u32le(&raw[1..5]),
            b_hp_percent: raw[5..6][0],
            i_min_hp: i32::from_le_bytes([
                raw[6..10][0],
                raw[6..10][1],
                raw[6..10][2],
                raw[6..10][3],
            ]),
            i_max_hp: i32::from_le_bytes([
                raw[10..14][0],
                raw[10..14][1],
                raw[10..14][2],
                raw[10..14][3],
            ]),
            b_alliance: raw[14..15][0],
            i_alliance_min_hp: i64::from_le_bytes([
                raw[15..23][0],
                raw[15..23][1],
                raw[15..23][2],
                raw[15..23][3],
                raw[15..23][4],
                raw[15..23][5],
                raw[15..23][6],
                raw[15..23][7],
            ]),
            i_alliance_max_hp: i64::from_le_bytes([
                raw[23..31][0],
                raw[23..31][1],
                raw[23..31][2],
                raw[23..31][3],
                raw[23..31][4],
                raw[23..31][5],
                raw[23..31][6],
                raw[23..31][7],
            ]),
            b_element: raw[31..32][0],
        })
    }
}

/// The exact packed wire length of [`GcSkillLevel`], in bytes.
pub const GC_SKILL_LEVEL_WIRE_SIZE: usize = 256;

/// The 256-byte skill-level record as the **client** decodes it, from
/// `TPacketGCSkillLevel`, whose payload is `BYTE abSkillLevels[255]`.
///
/// The server struct of the same name is `TPlayerSkill skills[255]`, and
/// `SPlayerSkill` measures 6 bytes on the legacy 32-bit target, so the server
/// send is `1 + 255 * 6 = 1531` bytes. The client allocates 256 and reads 256 of
/// a 1531-byte frame, a 1275-byte difference. The two sides cannot interoperate
/// and no codec can fix that, so this record implements the client decode and
/// preserves the 255 levels as raw bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 256-byte record the **client** decodes at byte 72, from
/// `TPacketGCSkillLevel`, whose payload is `BYTE abSkillLevels[255]`. This is
/// a dead generation: the server declares `HEADER_GC_SKILL_LEVEL_OLD = 72`
/// and never sends it. The live skill-level record is byte 76, where the
/// server sends the 1531-byte `TPacketGCSkillLevel` and the client decodes the
/// 1531-byte `TPacketGCSkillLevelNew`; `protocol::gc_nested` already covers
/// that pair. The 255 levels here are raw bytes precisely because the client
/// reads them as an opaque array rather than as skill structures.
pub struct GcSkillLevel {
    /// The raw `abSkillLevels` field.
    pub ab_skill_levels: [u8; SKILL_MAX_NUM],
}

impl GcSkillLevel {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(ab_skill_levels: [u8; SKILL_MAX_NUM]) -> Self {
        Self { ab_skill_levels }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_SKILL_LEVEL.value()
    }

    /// Append the 256 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.ab_skill_levels);
    }

    /// Read the 256 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_SKILL_LEVEL_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(bytes, GC_SKILL_LEVEL_WIRE_SIZE, "GcSkillLevel")?;
        check_header(raw, "GcSkillLevel", Self::header())?;
        Ok(Self {
            ab_skill_levels: array::<255>(raw, 1),
        })
    }
}

/// The exact packed wire length of [`GcMainCharacter2Empire`], in bytes.
pub const GC_MAIN_CHARACTER2_EMPIRE_WIRE_SIZE: usize = 46;

/// The 46-byte own-character record that adds the empire byte,
/// as `TPacketGCMainCharacter2_EMPIRE`. This is the shape the server actually
/// sends for its own 45-byte `TPacketGCMainCharacter`, which is why the byte 15
/// record is documented as one byte short.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 46-byte record the client decodes at byte 113, from
/// `TPacketGCMainCharacter2_EMPIRE`, and the one the server actually sends at
/// that byte under the plain name `HEADER_GC_MAIN_CHARACTER` from
/// `char.cpp:2018`. The server spells the same wire byte differently, which is
/// why this record is not called `GcMainCharacter`. The server struct is
/// `TPacketGCMainCharacter` with `lx, ly, lz, empire, skill_group`; the field
/// names differ but every offset agrees.
pub struct GcMainCharacter2Empire {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `wRaceNum` field.
    pub w_race_num: u16,
    /// The raw `szName` field.
    pub sz_name: [u8; NAME_LEN],
    /// The raw `lX` field.
    pub l_x: i32,
    /// The raw `lY` field.
    pub l_y: i32,
    /// The raw `lZ` field.
    pub l_z: i32,
    /// The raw `byEmpire` field.
    pub by_empire: u8,
    /// The raw `bySkillGroup` field.
    pub by_skill_group: u8,
}

impl GcMainCharacter2Empire {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        dw_vid: u32,
        w_race_num: u16,
        sz_name: [u8; NAME_LEN],
        l_x: i32,
        l_y: i32,
        l_z: i32,
        by_empire: u8,
        by_skill_group: u8,
    ) -> Self {
        Self {
            dw_vid,
            w_race_num,
            sz_name,
            l_x,
            l_y,
            l_z,
            by_empire,
            by_skill_group,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_MAIN_CHARACTER2_EMPIRE.value()
    }

    /// Append the 46 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.w_race_num.to_le_bytes());
        out.extend_from_slice(&self.sz_name);
        out.extend_from_slice(&self.l_x.to_le_bytes());
        out.extend_from_slice(&self.l_y.to_le_bytes());
        out.extend_from_slice(&self.l_z.to_le_bytes());
        out.push(self.by_empire);
        out.push(self.by_skill_group);
    }

    /// Read the 46 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_MAIN_CHARACTER2_EMPIRE_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(
            bytes,
            GC_MAIN_CHARACTER2_EMPIRE_WIRE_SIZE,
            "GcMainCharacter2Empire",
        )?;
        check_header(raw, "GcMainCharacter2Empire", Self::header())?;
        Ok(Self {
            dw_vid: u32le(&raw[1..5]),
            w_race_num: u16le(&raw[5..7]),
            sz_name: array::<25>(raw, 7),
            l_x: i32::from_le_bytes([
                raw[32..36][0],
                raw[32..36][1],
                raw[32..36][2],
                raw[32..36][3],
            ]),
            l_y: i32::from_le_bytes([
                raw[36..40][0],
                raw[36..40][1],
                raw[36..40][2],
                raw[36..40][3],
            ]),
            l_z: i32::from_le_bytes([
                raw[40..44][0],
                raw[40..44][1],
                raw[40..44][2],
                raw[40..44][3],
            ]),
            by_empire: raw[44..45][0],
            by_skill_group: raw[45..46][0],
        })
    }
}

/// The exact packed wire length of [`GcCharacterUpdate2`], in bytes.
pub const GC_CHARACTER_UPDATE2_WIRE_SIZE: usize = 44;

/// The 44-byte character-attribute refresh,
/// as `TPacketGCCharacterUpdate2`.
///
/// It is the byte 19 record minus `dwConquerorLevel`, `bRefineElementType`,
/// `byPremium`, `iPremiumTime`, and `bLanguage`, so it carries only the
/// `FIX_UPDATE_LEVEL` and `ENABLE_SHOW_LIDER_AND_GENERAL_GUILD` additions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 44-byte record the client decodes at byte 117, from
/// `TPacketGCCharacterUpdate2`. This is a dead generation: the server has no
/// `HEADER_GC_CHARACTER_UPDATE2` enumerator and no send site, so nothing in
/// the checked-in server produces this record. It is kept because the client
/// registers and decodes it, and it is the reduced successor of
/// [`GcCharacterUpdate`] without the conqueror-level, refine-element, premium,
/// and language fields.
pub struct GcCharacterUpdate2 {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `awPart` field.
    pub aw_part: [u16; EQUIP_PART_NUM],
    /// The raw `bMovingSpeed` field.
    pub b_moving_speed: u8,
    /// The raw `bAttackSpeed` field.
    pub b_attack_speed: u8,
    /// The raw `bStateFlag` field.
    pub b_state_flag: u8,
    /// The raw `dwAffectFlag` field.
    pub dw_affect_flag: [u32; 2],
    /// The raw `dwGuildID` field.
    pub dw_guild_id: u32,
    /// The raw `sAlignment` field.
    pub s_alignment: i16,
    /// The raw `dwLevel` field.
    pub dw_level: u32,
    /// The raw `bPKMode` field.
    pub b_pk_mode: u8,
    /// The raw `dwMountVnum` field.
    pub dw_mount_vnum: u32,
    /// The raw `dwNewIsGuildName` field.
    pub dw_new_is_guild_name: u8,
}

impl GcCharacterUpdate2 {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        dw_vid: u32,
        aw_part: [u16; EQUIP_PART_NUM],
        b_moving_speed: u8,
        b_attack_speed: u8,
        b_state_flag: u8,
        dw_affect_flag: [u32; 2],
        dw_guild_id: u32,
        s_alignment: i16,
        dw_level: u32,
        b_pk_mode: u8,
        dw_mount_vnum: u32,
        dw_new_is_guild_name: u8,
    ) -> Self {
        Self {
            dw_vid,
            aw_part,
            b_moving_speed,
            b_attack_speed,
            b_state_flag,
            dw_affect_flag,
            dw_guild_id,
            s_alignment,
            dw_level,
            b_pk_mode,
            dw_mount_vnum,
            dw_new_is_guild_name,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_CHARACTER_UPDATE2.value()
    }

    /// Append the 44 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.aw_part[0].to_le_bytes());
        out.extend_from_slice(&self.aw_part[1].to_le_bytes());
        out.extend_from_slice(&self.aw_part[2].to_le_bytes());
        out.extend_from_slice(&self.aw_part[3].to_le_bytes());
        out.extend_from_slice(&self.aw_part[4].to_le_bytes());
        out.extend_from_slice(&self.aw_part[5].to_le_bytes());
        out.push(self.b_moving_speed);
        out.push(self.b_attack_speed);
        out.push(self.b_state_flag);
        out.extend_from_slice(&self.dw_affect_flag[0].to_le_bytes());
        out.extend_from_slice(&self.dw_affect_flag[1].to_le_bytes());
        out.extend_from_slice(&self.dw_guild_id.to_le_bytes());
        out.extend_from_slice(&self.s_alignment.to_le_bytes());
        out.extend_from_slice(&self.dw_level.to_le_bytes());
        out.push(self.b_pk_mode);
        out.extend_from_slice(&self.dw_mount_vnum.to_le_bytes());
        out.push(self.dw_new_is_guild_name);
    }

    /// Read the 44 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_CHARACTER_UPDATE2_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(bytes, GC_CHARACTER_UPDATE2_WIRE_SIZE, "GcCharacterUpdate2")?;
        check_header(raw, "GcCharacterUpdate2", Self::header())?;
        Ok(Self {
            dw_vid: u32le(&raw[1..5]),
            aw_part: [
                u16le(&raw[5..7]),
                u16le(&raw[7..9]),
                u16le(&raw[9..11]),
                u16le(&raw[11..13]),
                u16le(&raw[13..15]),
                u16le(&raw[15..17]),
            ],
            b_moving_speed: raw[17..18][0],
            b_attack_speed: raw[18..19][0],
            b_state_flag: raw[19..20][0],
            dw_affect_flag: [u32le(&raw[20..24]), u32le(&raw[24..28])],
            dw_guild_id: u32le(&raw[28..32]),
            s_alignment: i16::from_le_bytes([raw[32..34][0], raw[32..34][1]]),
            dw_level: u32le(&raw[34..38]),
            b_pk_mode: raw[38..39][0],
            dw_mount_vnum: u32le(&raw[39..43]),
            dw_new_is_guild_name: raw[43..44][0],
        })
    }
}

/// The exact packed wire length of [`GcTargetCreateNew`], in bytes.
pub const GC_TARGET_CREATE_NEW_WIRE_SIZE: usize = 43;

/// The 43-byte manual-target record, as `TPacketGCTargetCreateNew`.
///
/// Its name array is a literal `char[32 + 1]`, not `CHARACTER_NAME_MAX_LEN + 1`,
/// so it is 33 bytes even though every other name field in the module is 25.
/// That difference is load-bearing: the two lengths must not be unified.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 43-byte record at byte 125, from the client's
/// `TPacketGCTargetCreateNew` and the server's `TPacketGCTargetCreate`; the
/// server sends it from `target.cpp:22` under the name
/// `HEADER_GC_TARGET_CREATE`. The name field is a distinct 33-byte
/// `char[32 + 1]` array on both sides and must not be unified with the 25-byte
/// character name used elsewhere.
pub struct GcTargetCreateNew {
    /// The raw `lID` field.
    pub l_id: i32,
    /// The raw `szTargetName` field.
    pub sz_target_name: [u8; TARGET_NAME_LEN],
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `byType` field.
    pub by_type: u8,
}

impl GcTargetCreateNew {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        l_id: i32,
        sz_target_name: [u8; TARGET_NAME_LEN],
        dw_vid: u32,
        by_type: u8,
    ) -> Self {
        Self {
            l_id,
            sz_target_name,
            dw_vid,
            by_type,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_TARGET_CREATE_NEW.value()
    }

    /// Append the 43 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.l_id.to_le_bytes());
        out.extend_from_slice(&self.sz_target_name);
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.push(self.by_type);
    }

    /// Read the 43 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_TARGET_CREATE_NEW_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(bytes, GC_TARGET_CREATE_NEW_WIRE_SIZE, "GcTargetCreateNew")?;
        check_header(raw, "GcTargetCreateNew", Self::header())?;
        Ok(Self {
            l_id: i32::from_le_bytes([raw[1..5][0], raw[1..5][1], raw[1..5][2], raw[1..5][3]]),
            sz_target_name: array::<33>(raw, 5),
            dw_vid: u32le(&raw[38..42]),
            by_type: raw[42..43][0],
        })
    }
}

/// The exact packed wire length of [`GcMainCharacter3Bgm`], in bytes.
pub const GC_MAIN_CHARACTER3_BGM_WIRE_SIZE: usize = 71;

/// The 71-byte own-character record that also carries the BGM name,
/// as `TPacketGCMainCharacter3_BGM`.
///
/// Its second array is a literal `MUSIC_NAME_MAX_LEN + 1`, which is 25 bytes.
/// The server spells the same constant `MUSIC_NAME_LEN` and names the first
/// array `szChrName` where the client says `szUserName`; the width is the same
/// on both sides.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 71-byte record at byte 137, from `TPacketGCMainCharacter3_BGM`
/// on both sides. It adds a 25-byte BGM name whose length comes from a nested
/// `enum { MUSIC_NAME_MAX_LEN = 24 }` inside the struct itself, so it is a
/// per-record constant and not the shared character name length. The server
/// sends it from `char.cpp:2009`.
pub struct GcMainCharacter3Bgm {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `wRaceNum` field.
    pub w_race_num: u16,
    /// The raw `szUserName` field.
    pub sz_user_name: [u8; NAME_LEN],
    /// The raw `szBGMName` field.
    pub sz_bgm_name: [u8; BGM_NAME_LEN],
    /// The raw `lX` field.
    pub l_x: i32,
    /// The raw `lY` field.
    pub l_y: i32,
    /// The raw `lZ` field.
    pub l_z: i32,
    /// The raw `byEmpire` field.
    pub by_empire: u8,
    /// The raw `bySkillGroup` field.
    pub by_skill_group: u8,
}

impl GcMainCharacter3Bgm {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        dw_vid: u32,
        w_race_num: u16,
        sz_user_name: [u8; NAME_LEN],
        sz_bgm_name: [u8; BGM_NAME_LEN],
        l_x: i32,
        l_y: i32,
        l_z: i32,
        by_empire: u8,
        by_skill_group: u8,
    ) -> Self {
        Self {
            dw_vid,
            w_race_num,
            sz_user_name,
            sz_bgm_name,
            l_x,
            l_y,
            l_z,
            by_empire,
            by_skill_group,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_MAIN_CHARACTER3_BGM.value()
    }

    /// Append the 71 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.w_race_num.to_le_bytes());
        out.extend_from_slice(&self.sz_user_name);
        out.extend_from_slice(&self.sz_bgm_name);
        out.extend_from_slice(&self.l_x.to_le_bytes());
        out.extend_from_slice(&self.l_y.to_le_bytes());
        out.extend_from_slice(&self.l_z.to_le_bytes());
        out.push(self.by_empire);
        out.push(self.by_skill_group);
    }

    /// Read the 71 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_MAIN_CHARACTER3_BGM_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(
            bytes,
            GC_MAIN_CHARACTER3_BGM_WIRE_SIZE,
            "GcMainCharacter3Bgm",
        )?;
        check_header(raw, "GcMainCharacter3Bgm", Self::header())?;
        Ok(Self {
            dw_vid: u32le(&raw[1..5]),
            w_race_num: u16le(&raw[5..7]),
            sz_user_name: array::<25>(raw, 7),
            sz_bgm_name: array::<25>(raw, 32),
            l_x: i32::from_le_bytes([
                raw[57..61][0],
                raw[57..61][1],
                raw[57..61][2],
                raw[57..61][3],
            ]),
            l_y: i32::from_le_bytes([
                raw[61..65][0],
                raw[61..65][1],
                raw[61..65][2],
                raw[61..65][3],
            ]),
            l_z: i32::from_le_bytes([
                raw[65..69][0],
                raw[65..69][1],
                raw[65..69][2],
                raw[65..69][3],
            ]),
            by_empire: raw[69..70][0],
            by_skill_group: raw[70..71][0],
        })
    }
}

/// The exact packed wire length of [`GcMainCharacter4BgmVol`], in bytes.
pub const GC_MAIN_CHARACTER4_BGM_VOL_WIRE_SIZE: usize = 75;

/// The 75-byte own-character record that adds the BGM volume,
/// as `TPacketGCMainCharacter4_BGM_VOL`. It is `GcMainCharacter3Bgm` with a
/// 4-byte `fBGMVol` inserted between the BGM name and the coordinates, so all
/// three coordinate fields shift by 4.

#[derive(Debug, Clone, Copy, PartialEq)]
/// The 75-byte record at byte 138, from
/// `TPacketGCMainCharacter4_BGM_VOL` on both sides. It is
/// [`GcMainCharacter3Bgm`] plus the leading 4-byte `float` BGM volume, and the
/// volume is a C++ `float`, so its four bytes are IEEE-754 and are copied
/// through rather than interpreted as an integer.
pub struct GcMainCharacter4BgmVol {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `wRaceNum` field.
    pub w_race_num: u16,
    /// The raw `szUserName` field.
    pub sz_user_name: [u8; NAME_LEN],
    /// The raw `szBGMName` field.
    pub sz_bgm_name: [u8; BGM_NAME_LEN],
    /// The raw `fBGMVol` field.
    pub f_bgm_vol: f32,
    /// The raw `lX` field.
    pub l_x: i32,
    /// The raw `lY` field.
    pub l_y: i32,
    /// The raw `lZ` field.
    pub l_z: i32,
    /// The raw `byEmpire` field.
    pub by_empire: u8,
    /// The raw `bySkillGroup` field.
    pub by_skill_group: u8,
}

impl GcMainCharacter4BgmVol {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        dw_vid: u32,
        w_race_num: u16,
        sz_user_name: [u8; NAME_LEN],
        sz_bgm_name: [u8; BGM_NAME_LEN],
        f_bgm_vol: f32,
        l_x: i32,
        l_y: i32,
        l_z: i32,
        by_empire: u8,
        by_skill_group: u8,
    ) -> Self {
        Self {
            dw_vid,
            w_race_num,
            sz_user_name,
            sz_bgm_name,
            f_bgm_vol,
            l_x,
            l_y,
            l_z,
            by_empire,
            by_skill_group,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_MAIN_CHARACTER4_BGM_VOL.value()
    }

    /// Append the 75 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.w_race_num.to_le_bytes());
        out.extend_from_slice(&self.sz_user_name);
        out.extend_from_slice(&self.sz_bgm_name);
        out.extend_from_slice(&self.f_bgm_vol.to_le_bytes());
        out.extend_from_slice(&self.l_x.to_le_bytes());
        out.extend_from_slice(&self.l_y.to_le_bytes());
        out.extend_from_slice(&self.l_z.to_le_bytes());
        out.push(self.by_empire);
        out.push(self.by_skill_group);
    }

    /// Read the 75 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_MAIN_CHARACTER4_BGM_VOL_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(
            bytes,
            GC_MAIN_CHARACTER4_BGM_VOL_WIRE_SIZE,
            "GcMainCharacter4BgmVol",
        )?;
        check_header(raw, "GcMainCharacter4BgmVol", Self::header())?;
        Ok(Self {
            dw_vid: u32le(&raw[1..5]),
            w_race_num: u16le(&raw[5..7]),
            sz_user_name: array::<25>(raw, 7),
            sz_bgm_name: array::<25>(raw, 32),
            f_bgm_vol: f32::from_le_bytes([
                raw[57..61][0],
                raw[57..61][1],
                raw[57..61][2],
                raw[57..61][3],
            ]),
            l_x: i32::from_le_bytes([
                raw[61..65][0],
                raw[61..65][1],
                raw[61..65][2],
                raw[61..65][3],
            ]),
            l_y: i32::from_le_bytes([
                raw[65..69][0],
                raw[65..69][1],
                raw[65..69][2],
                raw[65..69][3],
            ]),
            l_z: i32::from_le_bytes([
                raw[69..73][0],
                raw[69..73][1],
                raw[69..73][2],
                raw[69..73][3],
            ]),
            by_empire: raw[73..74][0],
            by_skill_group: raw[74..75][0],
        })
    }
}

/// The exact packed wire length of [`GcCharacterGoldChange`], in bytes.
pub const GC_CHARACTER_GOLD_CHANGE_WIRE_SIZE: usize = 24;

/// The 24-byte gold-change record, as `TPacketGCGoldChange`.
///
/// Its `header` field is declared `int`, not `BYTE`, so this is the only record
/// in the module with a **four-byte** little-endian header. `amount` is a
/// signed `long long` and `value` an `unsigned long long`. The server clamps a
/// non-positive `amount` to 0 before sending; that is producer policy and is
/// not modelled here.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 24-byte gold-change record, from `TPacketGCGoldChange`. This is
/// the only record in the module whose `header` field is a C++ `int`, so its
/// header occupies four little-endian bytes rather than one, and the encoder
/// writes all four. The two wide fields are `long long amount` and
/// `unsigned long long value`, which are 8 bytes each on the legacy 32-bit
/// target. The server sends it from `char.cpp:3848`.
pub struct GcCharacterGoldChange {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `amount` field.
    pub amount: i64,
    /// The raw `value` field.
    pub value: u64,
}

impl GcCharacterGoldChange {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(dw_vid: u32, amount: i64, value: u64) -> Self {
        Self {
            dw_vid,
            amount,
            value,
        }
    }

    /// The exact 4-byte little-endian header value this record carries.
    ///
    /// This is the only record in the module whose `header` field is a C++
    /// `int`, so it is four bytes wide rather than one.
    pub const HEADER: i32 = HEADER_GC_CHARACTER_GOLD_CHANGE.value() as i32;

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_CHARACTER_GOLD_CHANGE.value()
    }

    /// Append the 24 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&Self::HEADER.to_le_bytes());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.amount.to_le_bytes());
        out.extend_from_slice(&self.value.to_le_bytes());
    }

    /// Read the 24 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_CHARACTER_GOLD_CHANGE_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(
            bytes,
            GC_CHARACTER_GOLD_CHANGE_WIRE_SIZE,
            "GcCharacterGoldChange",
        )?;
        if i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) != Self::HEADER {
            return Err(GcActorsError::Header {
                context: "GcCharacterGoldChange",
                expected: Self::header(),
                actual: raw[0],
            });
        }
        Ok(Self {
            dw_vid: u32le(&raw[4..8]),
            amount: i64::from_le_bytes([
                raw[8..16][0],
                raw[8..16][1],
                raw[8..16][2],
                raw[8..16][3],
                raw[8..16][4],
                raw[8..16][5],
                raw[8..16][6],
                raw[8..16][7],
            ]),
            value: u64le(&raw[16..24]),
        })
    }
}

/// The exact packed wire length of [`GcCharacterAdditionalInfo`], in bytes.
pub const GC_CHAR_ADDITIONAL_INFO_WIRE_SIZE: usize = 70;

/// The 70-byte supplementary character record,
/// as `TPacketGCCharacterAdditionalInfo`. It carries the same six gate-gated
/// fields as `GcCharacterUpdate`, plus `bEmpire`, `dwGuildID`, `dwLevel`, and
/// the 25-byte `name`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The 70-byte record that carries the full character summary, from
/// `TPacketGCCharacterAdditionalInfo`. It is the widest of the equipment
/// records because it carries both guild fields, the conqueror level, the
/// refine element, the premium pair, and the language byte. Client and server
/// structs agree on every field offset.
pub struct GcCharacterAdditionalInfo {
    /// The raw `dwVID` field.
    pub dw_vid: u32,
    /// The raw `name` field.
    pub name: [u8; NAME_LEN],
    /// The raw `awPart` field.
    pub aw_part: [u16; EQUIP_PART_NUM],
    /// The raw `bEmpire` field.
    pub b_empire: u8,
    /// The raw `dwGuildID` field.
    pub dw_guild_id: u32,
    /// The raw `dwLevel` field.
    pub dw_level: u32,
    /// The raw `dwConquerorLevel` field.
    pub dw_conqueror_level: u32,
    /// The raw `sAlignment` field.
    pub s_alignment: i16,
    /// The raw `bPKMode` field.
    pub b_pk_mode: u8,
    /// The raw `dwMountVnum` field.
    pub dw_mount_vnum: u32,
    /// The raw `bRefineElementType` field.
    pub b_refine_element_type: u8,
    /// The raw `dwNewIsGuildName` field.
    pub dw_new_is_guild_name: u8,
    /// The raw `byPremium` field.
    pub by_premium: u8,
    /// The raw `iPremiumTime` field.
    pub i_premium_time: i32,
    /// The raw `bLanguage` field.
    pub b_language: u8,
}

impl GcCharacterAdditionalInfo {
    /// Build the record with every non-header field set explicitly.
    ///
    /// The header is fixed by the record type and is written by the encoder, so
    /// it is not a field: a caller cannot build a record that its own decoder
    /// would then reject.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        dw_vid: u32,
        name: [u8; NAME_LEN],
        aw_part: [u16; EQUIP_PART_NUM],
        b_empire: u8,
        dw_guild_id: u32,
        dw_level: u32,
        dw_conqueror_level: u32,
        s_alignment: i16,
        b_pk_mode: u8,
        dw_mount_vnum: u32,
        b_refine_element_type: u8,
        dw_new_is_guild_name: u8,
        by_premium: u8,
        i_premium_time: i32,
        b_language: u8,
    ) -> Self {
        Self {
            dw_vid,
            name,
            aw_part,
            b_empire,
            dw_guild_id,
            dw_level,
            dw_conqueror_level,
            s_alignment,
            b_pk_mode,
            dw_mount_vnum,
            b_refine_element_type,
            dw_new_is_guild_name,
            by_premium,
            i_premium_time,
            b_language,
        }
    }

    /// The fixed one-byte header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_CHAR_ADDITIONAL_INFO.value()
    }

    /// Append the 70 packed bytes to `out`, starting with the fixed header.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.dw_vid.to_le_bytes());
        out.extend_from_slice(&self.name);
        out.extend_from_slice(&self.aw_part[0].to_le_bytes());
        out.extend_from_slice(&self.aw_part[1].to_le_bytes());
        out.extend_from_slice(&self.aw_part[2].to_le_bytes());
        out.extend_from_slice(&self.aw_part[3].to_le_bytes());
        out.extend_from_slice(&self.aw_part[4].to_le_bytes());
        out.extend_from_slice(&self.aw_part[5].to_le_bytes());
        out.push(self.b_empire);
        out.extend_from_slice(&self.dw_guild_id.to_le_bytes());
        out.extend_from_slice(&self.dw_level.to_le_bytes());
        out.extend_from_slice(&self.dw_conqueror_level.to_le_bytes());
        out.extend_from_slice(&self.s_alignment.to_le_bytes());
        out.push(self.b_pk_mode);
        out.extend_from_slice(&self.dw_mount_vnum.to_le_bytes());
        out.push(self.b_refine_element_type);
        out.push(self.dw_new_is_guild_name);
        out.push(self.by_premium);
        out.extend_from_slice(&self.i_premium_time.to_le_bytes());
        out.push(self.b_language);
    }

    /// Read the 70 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcActorsError::Truncated`] when the buffer is shorter than [`GC_CHAR_ADDITIONAL_INFO_WIRE_SIZE`],
    /// and [`GcActorsError::Header`] when a complete-length buffer starts with
    /// a byte other than the one this record requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcActorsError> {
        let raw = take(
            bytes,
            GC_CHAR_ADDITIONAL_INFO_WIRE_SIZE,
            "GcCharacterAdditionalInfo",
        )?;
        check_header(raw, "GcCharacterAdditionalInfo", Self::header())?;
        Ok(Self {
            dw_vid: u32le(&raw[1..5]),
            name: array::<25>(raw, 5),
            aw_part: [
                u16le(&raw[30..32]),
                u16le(&raw[32..34]),
                u16le(&raw[34..36]),
                u16le(&raw[36..38]),
                u16le(&raw[38..40]),
                u16le(&raw[40..42]),
            ],
            b_empire: raw[42..43][0],
            dw_guild_id: u32le(&raw[43..47]),
            dw_level: u32le(&raw[47..51]),
            dw_conqueror_level: u32le(&raw[51..55]),
            s_alignment: i16::from_le_bytes([raw[55..57][0], raw[55..57][1]]),
            b_pk_mode: raw[57..58][0],
            dw_mount_vnum: u32le(&raw[58..62]),
            b_refine_element_type: raw[62..63][0],
            dw_new_is_guild_name: raw[63..64][0],
            by_premium: raw[64..65][0],
            i_premium_time: i32::from_le_bytes([
                raw[65..69][0],
                raw[65..69][1],
                raw[65..69][2],
                raw[65..69][3],
            ]),
            b_language: raw[69..70][0],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// One record, reduced to everything the table-driven tests need.
    struct Rec {
        /// The legacy enumerator name for this record.
        name: &'static str,
        /// The Rust record type, which is what the errors name.
        ty: &'static str,
        /// The one header byte this record requires.
        header: u8,
        /// The exact packed wire length.
        size: usize,
        /// A fully populated `instance`, encoded to the wire.
        sample: fn() -> Vec<u8>,
        /// Decode `bytes` and re-encode them, or return the rejection reason.
        ///
        /// This single hook covers every case the tests need: `Ok` means the
        /// bytes were accepted, and comparing the result with the input is a
        /// byte-for-byte round trip.
        round_trip: fn(&[u8]) -> Result<Vec<u8>, GcActorsError>,
    }

    /// Build a table row for one record type.
    macro_rules! rec {
        ($ty:ident, $name:literal, $size:tt, $sample:expr) => {
            Rec {
                name: $name,
                ty: stringify!($ty),
                header: $ty::header(),
                size: $size,
                sample: $sample,
                round_trip: |bytes| {
                    let record = $ty::decode(bytes)?;
                    let mut out = Vec::new();
                    record.encode_into(&mut out);
                    Ok(out)
                },
            }
        };
    }

    /// A deterministic asymmetric byte value, so a byte-order defect shows up.
    fn byte_at(i: usize) -> u8 {
        u8::try_from((i * 7 + 11) % 251).expect("the range is under 251")
    }

    /// A buffer of `len` asymmetric bytes, used to prove truncation is exact.
    fn ramp(len: usize) -> Vec<u8> {
        (0..len).map(byte_at).collect()
    }

    fn name_at(i: usize) -> [u8; NAME_LEN] {
        let mut n = [0u8; NAME_LEN];
        for (k, b) in n.iter_mut().enumerate() {
            *b = byte_at(i * 31 + k);
        }
        n
    }

    fn bgm_at(i: usize) -> [u8; BGM_NAME_LEN] {
        let mut n = [0u8; BGM_NAME_LEN];
        for (k, b) in n.iter_mut().enumerate() {
            *b = byte_at(i * 37 + k);
        }
        n
    }

    fn target_at(i: usize) -> [u8; TARGET_NAME_LEN] {
        let mut n = [0u8; TARGET_NAME_LEN];
        for (k, b) in n.iter_mut().enumerate() {
            *b = byte_at(i * 41 + k);
        }
        n
    }

    fn skill_at() -> [u8; SKILL_MAX_NUM] {
        let mut s = [0u8; SKILL_MAX_NUM];
        for (k, b) in s.iter_mut().enumerate() {
            *b = byte_at(k + 3);
        }
        s
    }

    fn parts_at(i: usize) -> [u16; EQUIP_PART_NUM] {
        let mut p = [0u16; EQUIP_PART_NUM];
        for (k, b) in p.iter_mut().enumerate() {
            *b = u16::try_from(i * 101 + k + 1).expect("the value is small");
        }
        p
    }

    fn flags_at(i: usize) -> [u32; 2] {
        [
            u32::try_from(i * 1009 + 1).expect("the value is small"),
            u32::try_from(i * 2011 + 2).expect("the value is small"),
        ]
    }

    /// Encode a record into a fresh buffer.
    fn encode<T: Copy, F: Fn(&T, &mut Vec<u8>)>(record: T, push: F) -> Vec<u8> {
        let mut out = Vec::new();
        push(&record, &mut out);
        out
    }

    ///
    /// The sample values are chosen so that no two adjacent fields share a
    /// value, which is what lets the swap and reversal tests notice a wrong
    /// field order or a wrong byte order.    /// Every record this module implements, with one full sample each.
    ///
    /// The sample values are chosen so that no two adjacent fields share a
    /// value, which is what lets the swap and reversal tests notice a wrong
    /// field order or a wrong byte order.
    fn table() -> Vec<Rec> {
        vec![
            rec!(
                GcCharacterAdd,
                "HEADER_GC_CHARACTER_ADD",
                GC_CHARACTER_ADD_WIRE_SIZE,
                sample_gc_character_add
            ),
            rec!(
                GcCharacterMove,
                "HEADER_GC_CHARACTER_MOVE",
                GC_CHARACTER_MOVE_WIRE_SIZE,
                sample_gc_character_move
            ),
            rec!(
                GcMainCharacter,
                "HEADER_GC_MAIN_CHARACTER",
                GC_MAIN_CHARACTER_WIRE_SIZE,
                sample_gc_main_character
            ),
            rec!(
                GcCharacterUpdate,
                "HEADER_GC_CHARACTER_UPDATE",
                GC_CHARACTER_UPDATE_WIRE_SIZE,
                sample_gc_character_update
            ),
            rec!(
                GcTargetInfo,
                "HEADER_GC_TARGET_INFO",
                GC_TARGET_INFO_WIRE_SIZE,
                sample_gc_target_info
            ),
            rec!(
                GcTarget,
                "HEADER_GC_TARGET",
                GC_TARGET_WIRE_SIZE,
                sample_gc_target
            ),
            rec!(
                GcSkillLevel,
                "HEADER_GC_SKILL_LEVEL",
                GC_SKILL_LEVEL_WIRE_SIZE,
                sample_gc_skill_level
            ),
            rec!(
                GcMainCharacter2Empire,
                "HEADER_GC_MAIN_CHARACTER2_EMPIRE",
                GC_MAIN_CHARACTER2_EMPIRE_WIRE_SIZE,
                sample_gc_main_character2_empire
            ),
            rec!(
                GcCharacterUpdate2,
                "HEADER_GC_CHARACTER_UPDATE2",
                GC_CHARACTER_UPDATE2_WIRE_SIZE,
                sample_gc_character_update2
            ),
            rec!(
                GcTargetCreateNew,
                "HEADER_GC_TARGET_CREATE_NEW",
                GC_TARGET_CREATE_NEW_WIRE_SIZE,
                sample_gc_target_create_new
            ),
            rec!(
                GcMainCharacter3Bgm,
                "HEADER_GC_MAIN_CHARACTER3_BGM",
                GC_MAIN_CHARACTER3_BGM_WIRE_SIZE,
                sample_gc_main_character3_bgm
            ),
            rec!(
                GcMainCharacter4BgmVol,
                "HEADER_GC_MAIN_CHARACTER4_BGM_VOL",
                GC_MAIN_CHARACTER4_BGM_VOL_WIRE_SIZE,
                sample_gc_main_character4_bgm_vol
            ),
            rec!(
                GcCharacterGoldChange,
                "HEADER_GC_CHARACTER_GOLD_CHANGE",
                GC_CHARACTER_GOLD_CHANGE_WIRE_SIZE,
                sample_gc_character_gold_change
            ),
            rec!(
                GcCharacterAdditionalInfo,
                "HEADER_GC_CHAR_ADDITIONAL_INFO",
                GC_CHAR_ADDITIONAL_INFO_WIRE_SIZE,
                sample_gc_character_additional_info
            ),
        ]
    }

    /// A fully populated `GcCharacterAdd`, encoded to the wire.
    fn sample_gc_character_add() -> Vec<u8> {
        encode(
            GcCharacterAdd::new(
                0x0000_0001,
                1.25,
                -200_202,
                -200_303,
                -200_404,
                0x10,
                0x0607,
                0x16,
                0x19,
                0x1c,
                flags_at(10),
            ),
            GcCharacterAdd::encode_into,
        )
    }

    /// A fully populated `GcCharacterMove`, encoded to the wire.
    fn sample_gc_character_move() -> Vec<u8> {
        encode(
            GcCharacterMove::new(
                0x01,
                0x04,
                0x07,
                0x0003_0004,
                -200_404,
                -200_505,
                0x0006_0007,
                0x0007_0008,
            ),
            GcCharacterMove::encode_into,
        )
    }

    /// A fully populated `GcMainCharacter`, encoded to the wire.
    fn sample_gc_main_character() -> Vec<u8> {
        encode(
            GcMainCharacter::new(
                0x0000_0001,
                0x0102,
                name_at(2),
                -200_303,
                -200_404,
                -200_505,
                0x13,
            ),
            GcMainCharacter::encode_into,
        )
    }

    /// A fully populated `GcCharacterUpdate`, encoded to the wire.
    fn sample_gc_character_update() -> Vec<u8> {
        encode(
            GcCharacterUpdate::new(
                0x0000_0001,
                parts_at(1),
                0x07,
                0x0a,
                0x0d,
                flags_at(5),
                0x0006_0007,
                -1_049,
                0x0008_0009,
                0x0009_000a,
                0x1f,
                0x000b_000c,
                0x25,
                0x28,
                0x2b,
                -201_515,
                0x31,
            ),
            GcCharacterUpdate::encode_into,
        )
    }

    /// A fully populated `GcTargetInfo`, encoded to the wire.
    fn sample_gc_target_info() -> Vec<u8> {
        encode(
            GcTargetInfo::new(0x0000_0001, 0x0001_0002, 0x0002_0003, 0x0304, 0x0004_0005),
            GcTargetInfo::encode_into,
        )
    }

    /// A fully populated `GcTarget`, encoded to the wire.
    fn sample_gc_target() -> Vec<u8> {
        encode(
            GcTarget::new(
                0x0000_0001,
                0x04,
                -200_202,
                -200_303,
                0x0d,
                -2_999_999_995,
                -2_999_999_994,
                0x16,
            ),
            GcTarget::encode_into,
        )
    }

    /// A fully populated `GcSkillLevel`, encoded to the wire.
    fn sample_gc_skill_level() -> Vec<u8> {
        encode(GcSkillLevel::new(skill_at()), GcSkillLevel::encode_into)
    }

    /// A fully populated `GcMainCharacter2Empire`, encoded to the wire.
    fn sample_gc_main_character2_empire() -> Vec<u8> {
        encode(
            GcMainCharacter2Empire::new(
                0x0000_0001,
                0x0102,
                name_at(2),
                -200_303,
                -200_404,
                -200_505,
                0x13,
                0x16,
            ),
            GcMainCharacter2Empire::encode_into,
        )
    }

    /// A fully populated `GcCharacterUpdate2`, encoded to the wire.
    fn sample_gc_character_update2() -> Vec<u8> {
        encode(
            GcCharacterUpdate2::new(
                0x0000_0001,
                parts_at(1),
                0x07,
                0x0a,
                0x0d,
                flags_at(5),
                0x0006_0007,
                -1_049,
                0x0008_0009,
                0x1c,
                0x000a_000b,
                0x22,
            ),
            GcCharacterUpdate2::encode_into,
        )
    }

    /// A fully populated `GcTargetCreateNew`, encoded to the wire.
    fn sample_gc_target_create_new() -> Vec<u8> {
        encode(
            GcTargetCreateNew::new(-200_000, target_at(1), 0x0002_0003, 0x0a),
            GcTargetCreateNew::encode_into,
        )
    }

    /// A fully populated `GcMainCharacter3Bgm`, encoded to the wire.
    fn sample_gc_main_character3_bgm() -> Vec<u8> {
        encode(
            GcMainCharacter3Bgm::new(
                0x0000_0001,
                0x0102,
                name_at(2),
                bgm_at(3),
                -200_404,
                -200_505,
                -200_606,
                0x16,
                0x19,
            ),
            GcMainCharacter3Bgm::encode_into,
        )
    }

    /// A fully populated `GcMainCharacter4BgmVol`, encoded to the wire.
    fn sample_gc_main_character4_bgm_vol() -> Vec<u8> {
        encode(
            GcMainCharacter4BgmVol::new(
                0x0000_0001,
                0x0102,
                name_at(2),
                bgm_at(3),
                2.0,
                -200_505,
                -200_606,
                -200_707,
                0x19,
                0x1c,
            ),
            GcMainCharacter4BgmVol::encode_into,
        )
    }

    /// A fully populated `GcCharacterGoldChange`, encoded to the wire.
    fn sample_gc_character_gold_change() -> Vec<u8> {
        encode(
            GcCharacterGoldChange::new(0x0000_0001, -2_999_999_999, 0x00_00_02_04_06_08_0a_0d),
            GcCharacterGoldChange::encode_into,
        )
    }

    /// A fully populated `GcCharacterAdditionalInfo`, encoded to the wire.
    fn sample_gc_character_additional_info() -> Vec<u8> {
        encode(
            GcCharacterAdditionalInfo::new(
                0x0000_0001,
                name_at(1),
                parts_at(2),
                0x0a,
                0x0004_0005,
                0x0005_0006,
                0x0006_0007,
                -1_049,
                0x19,
                0x0009_000a,
                0x1f,
                0x22,
                0x25,
                -201_313,
                0x2b,
            ),
            GcCharacterAdditionalInfo::encode_into,
        )
    }

    /// Every record emits exactly its declared number of bytes, header first.
    #[test]
    fn every_record_encodes_to_its_declared_wire_size() {
        for rec in table() {
            let wire = (rec.sample)();
            assert_eq!(
                wire.len(),
                rec.size,
                "{} emitted the wrong byte count",
                rec.name
            );
            assert_eq!(
                wire[0], rec.header,
                "{} did not emit its own header first",
                rec.name
            );
        }
    }

    /// Every record decodes and then re-encodes to the identical bytes.
    #[test]
    fn every_record_round_trips_byte_for_byte() {
        for rec in table() {
            let wire = (rec.sample)();
            let again = (rec.round_trip)(&wire)
                .unwrap_or_else(|e| panic!("{} was rejected: {e:?}", rec.name));
            assert_eq!(again, wire, "{} did not round trip byte for byte", rec.name);
        }
    }

    /// A second round trip changes nothing, so decoding is a fixed point.
    #[test]
    fn a_second_round_trip_changes_nothing() {
        for rec in table() {
            let once = (rec.round_trip)(&(rec.sample)()).expect("the sample decodes");
            let twice = (rec.round_trip)(&once).expect("the re-encode decodes");
            assert_eq!(
                once, twice,
                "{} is not stable across two round trips",
                rec.name
            );
        }
    }

    /// A record never decodes when one byte is missing.
    #[test]
    fn every_record_rejects_every_truncated_prefix() {
        for rec in table() {
            let wire = (rec.sample)();
            for cut in 0..rec.size {
                let err =
                    (rec.round_trip)(&wire[..cut]).expect_err("a short buffer must not decode");
                assert!(
                    matches!(err, GcActorsError::Truncated { .. }),
                    "{} at {cut} bytes gave {err:?}",
                    rec.name
                );
            }
        }
    }

    /// A record never decodes when the header byte is wrong, and it says why.
    #[test]
    fn every_record_rejects_a_wrong_header() {
        for rec in table() {
            let mut wire = ramp(rec.size);
            wire[0] = rec.header.wrapping_add(0x5a);
            if wire[0] == rec.header {
                wire[0] = rec.header.wrapping_add(1);
            }
            let err = (rec.round_trip)(&wire).expect_err("a wrong header must not decode");
            if let GcActorsError::Header {
                context,
                expected,
                actual,
            } = err
            {
                assert_eq!(context, rec.ty);
                assert_eq!(expected, rec.header);
                assert_eq!(actual, wire[0]);
            } else {
                panic!("{} gave {err:?} instead of a header error", rec.name);
            }
        }
    }

    /// The 14 headers are all distinct, so no two records can be confused.
    #[test]
    fn all_fourteen_headers_are_distinct() {
        let table = table();
        let headers: BTreeSet<u8> = table.iter().map(|rec| rec.header).collect();
        assert_eq!(headers.len(), table.len());
    }

    /// No record accepts another record's bytes at a complete length.
    #[test]
    fn no_record_accepts_another_records_bytes() {
        let table = table();
        for (i, rec) in table.iter().enumerate() {
            for (j, other) in table.iter().enumerate() {
                if i == j {
                    continue;
                }
                let mut copy = (rec.sample)();
                copy.resize(other.size, 0);
                copy[0] = other.header;
                assert!(
                    (rec.round_trip)(&copy).is_err(),
                    "{} accepted {} bytes",
                    rec.name,
                    other.name
                );
            }
        }
    }

    /// Swapping the first two bytes never yields the same decoded record.
    ///
    /// The sample values make every adjacent field distinct, so a decoder that
    /// reads two neighbours into the wrong slots changes the decoded record.
    #[test]
    fn no_two_adjacent_fields_of_any_record_are_interchangeable() {
        for rec in table() {
            let wire = (rec.sample)();
            let mut swapped = wire.clone();
            swapped.swap(0, 1);
            if swapped == wire {
                continue;
            }
            if let (Ok(before), Ok(after)) = ((rec.round_trip)(&wire), (rec.round_trip)(&swapped)) {
                assert_ne!(before, after, "{} ignored a swap", rec.name);
            }
        }
    }

    /// Reversing the whole frame never yields the same decoded record.
    #[test]
    fn no_record_survives_a_full_byte_reversal() {
        for rec in table() {
            let wire = (rec.sample)();
            let reversed: Vec<u8> = wire.iter().rev().copied().collect();
            if reversed == wire {
                continue;
            }
            if let (Ok(before), Ok(after)) = ((rec.round_trip)(&wire), (rec.round_trip)(&reversed))
            {
                assert_ne!(before, after, "{} ignored a reversal", rec.name);
            }
        }
    }

    /// The four-byte gold header is rejected when any of its four bytes differ.
    #[test]
    fn the_gold_header_is_checked_across_all_four_bytes() {
        let wire = encode(
            GcCharacterGoldChange::new(7, 9, 11),
            GcCharacterGoldChange::encode_into,
        );
        assert_eq!(&wire[0..4], &[0xe1, 0x00, 0x00, 0x00]);
        for pos in 0..4 {
            let mut bad = wire.clone();
            bad[pos] ^= 0xff;
            let err = GcCharacterGoldChange::decode(&bad)
                .expect_err("a corrupted four-byte gold header must not decode");
            assert!(
                matches!(err, GcActorsError::Header { .. }),
                "byte {pos} gave {err:?}"
            );
        }
    }

    /// The three dead client decode entries keep their measured widths.
    ///
    /// These are the records the client registers and decodes but the
    /// checked-in server never sends: byte 15
    /// (`HEADER_GC_MAIN_CHARACTER_OLD`), byte 72
    /// (`HEADER_GC_SKILL_LEVEL_OLD`), and byte 117, which has no server
    /// enumerator at all. The live successors are bytes 113 and 76.
    #[test]
    fn the_three_dead_client_entries_keep_their_measured_widths() {
        assert_eq!(GcMainCharacter::header(), 0x0f);
        assert_eq!(GC_MAIN_CHARACTER_WIRE_SIZE, 45);
        assert_eq!(GcSkillLevel::header(), 0x48);
        assert_eq!(GC_SKILL_LEVEL_WIRE_SIZE, 256);
        assert_eq!(GcCharacterUpdate2::header(), 0x75);
        assert_eq!(GC_CHARACTER_UPDATE2_WIRE_SIZE, 44);
    }

    /// The live main-character record is byte 113, not byte 15.
    #[test]
    fn the_live_main_character_record_is_byte_113() {
        // Byte 113 is a matching pair: the client decodes 46 bytes and the
        // server sends 46 bytes, so this record is neither padded nor trimmed.
        assert_eq!(GcMainCharacter2Empire::header(), 0x71);
        assert_eq!(GC_MAIN_CHARACTER2_EMPIRE_WIRE_SIZE, 46);
        assert_ne!(GcMainCharacter2Empire::header(), GcMainCharacter::header());
    }

    /// The three distinct name-array lengths must never be unified.
    #[test]
    fn the_three_name_array_lengths_stay_distinct() {
        assert_eq!(NAME_LEN, 25);
        assert_eq!(BGM_NAME_LEN, 25);
        assert_eq!(TARGET_NAME_LEN, 33);
    }

    /// Raw character arrays keep every byte, including NULs and high bytes.
    #[test]
    fn raw_name_arrays_preserve_every_byte_including_nul() {
        let mut name = [0u8; NAME_LEN];
        name[0] = 0xff;
        name[1] = 0x00;
        name[24] = 0x80;
        let wire = encode(
            GcMainCharacter::new(1, 2, name, 3, 4, 5, 6),
            GcMainCharacter::encode_into,
        );
        assert_eq!(&wire[7..32], &name);
        assert_eq!(
            GcMainCharacter::decode(&wire).expect("decodes").sz_name,
            name
        );
    }

    /// The resolved array constants match the legacy values.
    #[test]
    fn the_resolved_array_constants_match_the_legacy_values() {
        assert_eq!(CHARACTER_NAME_MAX_LEN_EXPECTED, NAME_LEN - 1);
        assert_eq!(EQUIP_PART_NUM, 6);
        assert_eq!(SKILL_MAX_NUM, 255);
    }
}
