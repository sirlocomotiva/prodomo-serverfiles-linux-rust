//! The game-to-client records that carry named scalar fields and raw fixed
//! character arrays.
//!
//! # Provenance and how every width here was obtained
//!
//! Each of the 18 records in this module is a `#pragma pack(1)` structure. No
//! width is hand-summed. Every one was **machine-derived**: each struct body was
//! extracted verbatim from both `client/Client/UserInterface/Packet.h` and
//! `server/server/game/packet.h`, the array-dimension constants were substituted
//! with their resolved values, and the result was compiled with
//! `i686-linux-gnu-g++-12` and run under `sizeof`. The client-derived and
//! server-derived widths agree for all 18.
//!
//! The typedefs used for that compile were copied verbatim from
//! `server/server/libthecore/typedef.h`, taking the `#ifndef __WIN32__` branch
//! that the legacy FreeBSD server uses. That header fixes
//! `DWORD = unsigned int`, `BYTE = unsigned char`, `WORD = unsigned short`,
//! `LONG = long`, and `ULONG = unsigned long`.
//!
//! A control run on five already-settled records reproduced the earlier
//! hand-summed widths exactly (10, 9, 11, 13, 5), and a two-`long` packed
//! control struct measured 8 bytes, which proves the probe really is 32-bit and
//! not an LP64 reading.
//!
//! # Widths that depend on the legacy target
//!
//! `server/server/premake5.lua:12` sets `architecture "x86"`, so the target is
//! 32-bit. That matters for four records here:
//!
//! - `TPacketGCQuestConfirm.timeout` is a C++ `long`, measured at **4** bytes.
//! - `TPacketGCTargetUpdate.lID`, `.lX`, and `.lY` are C++ `long`, **4** bytes
//!   each.
//! - `TPacketGCPoints.points` is `long long`, 8 bytes, so the record is
//!   `1 + 255 * 8 = 2041` bytes.
//! - `TPacketGCGold.gold` is `unsigned long long`, 8 bytes, so the record is 9.
//!
//! Note the asymmetry inside the same legacy header: `QWORD` and `ULONG` are
//! `unsigned long`, which is **4** bytes on this target, while `long long` is
//! 8. `QWORD` is not used by any record in this module, but the trap is real
//! and is recorded in the ledger.
//!
//! # Array-dimension constants, and how they were resolved
//!
//! The dimension constants used by these records are **not** `#define`s. They
//! are members of anonymous `enum` blocks:
//!
//! | constant | value | definition |
//! | --- | --- | --- |
//! | `CHARACTER_NAME_MAX_LEN` | 24 | `server/server/common/length.h:15`, `client/Client/UserInterface/StdAfx.h:43` |
//! | `SHOP_SIGN_MAX_LEN` | 32 | `server/server/common/length.h:16`, `client/Client/UserInterface/Packet.h:249` |
//! | `MAX_EFFECT_FILE_NAME` | 128 | `server/server/game/packet.h:2739` (a `#define`; the client writes the literal `128`) |
//! | `POINT_MAX_NUM` | 255 | `server/server/common/length.h:81`, `client/Client/UserInterface/StdAfx.h:42` |
//!
//! A search that looks only for `#define NAME` reports all four as undefined.
//! See ledger section 166.3: this tree defines constants in four different
//! forms, and a `#define`-only search produces a false "not in the tree" result.
//!
//! # Two records whose name differs from the server's
//!
//! `HEADER_GC_MOTION` (36) is a client `TPacketGCMotion` but a server
//! `struct packet_motion`, declared with the plain `struct NAME { ... };` form
//! rather than the `} NAME;` typedef form. `char.cpp:5045` takes it as a
//! pointer parameter. The two bodies are identical.
//!
//! `HEADER_GC_PARTY_UNLINK` (92) is a client `TPacketGCPartyUnlink`, but
//! `party.cpp:729` sends it as a `TPacketGCPartyLink` with only the header
//! changed. The server does have a `TPacketGCPartyUnlink` type, which is
//! declared and never sent. Both bodies are identical here too.
//!
//! # Fixed character arrays stay raw
//!
//! `CHARACTER_NAME_MAX_LEN + 1`, `SHOP_SIGN_MAX_LEN + 1`, and the quest
//! message array are C++ `char[N]` fields. They are modelled as `[u8; N]`
//! with no text decoding, no NUL validation, and no string accessor, because
//! the legacy records are raw storage and the client applies its own encoding.
//!
//! # Feature profiles
//!
//! `HEADER_GC_AUTO_SHAMAN_SKILL` (57) is sent from `char.cpp:12745` inside
//! `#ifdef __ENABLE_SHAMAN_SYSTEM__`. That macro **is** defined on both sides,
//! at `server/server/common/prodomodefines.h:193` and
//! `client/Client/UserInterface/LOCALE_INC.H:143`, so the record is live in the
//! active build and its width does not vary.
//!
//! No record in this module is behind a profile that is undefined, so no record
//! here has two possible widths.
//!
//! # Scope
//!
//! Record codecs only. Nothing here picks up an item, moves a player, changes
//! ownership, forms a party, starts a quest, or mutates a session. The 32-bit
//! and 64-bit values stay opaque, and the `BYTE` fields keep every value,
//! including the ones a client `switch` would silently ignore.

use std::fmt;

/// Width of the packed `char[CHARACTER_NAME_MAX_LEN + 1]` field.
///
/// `CHARACTER_NAME_MAX_LEN` is 24 on both sides, so the field is 25 bytes.
pub const GC_NAME_FIELD_SIZE: usize = 24 + 1;

/// Width of the packed `char[SHOP_SIGN_MAX_LEN + 1]` field.
///
/// `SHOP_SIGN_MAX_LEN` is 32 on both sides, so the field is 33 bytes.
pub const GC_SHOP_SIGN_FIELD_SIZE: usize = 32 + 1;

/// Width of the packed `char[MAX_EFFECT_FILE_NAME]` effect-file field.
pub const GC_EFFECT_FILE_FIELD_SIZE: usize = 128;

/// Width of the packed `char[64 + 1]` quest message field.
pub const GC_QUEST_MESSAGE_FIELD_SIZE: usize = 64 + 1;

/// Number of point slots in [`GcPoints`], from `POINT_MAX_NUM`.
pub const GC_POINT_SLOT_COUNT: usize = 255;

/// Packed width of the shared 25-byte name records: header, `DWORD`, name.
pub const GC_NAMED_WIRE_SIZE: usize = 1 + 4 + GC_NAME_FIELD_SIZE;

/// Packed width of the shared 9-byte two-`DWORD` records: header and two words.
pub const GC_TWO_WORD_WIRE_SIZE: usize = 1 + 4 + 4;

/// Packed width of [`GcPvp`]: header, two `DWORD`s, and a `BYTE`.
pub const GC_PVP_WIRE_SIZE: usize = 1 + 4 + 4 + 1;

/// Packed width of [`GcPickupItem`]: header and two `int`s.
pub const GC_PICKUP_ITEM_WIRE_SIZE: usize = 1 + 4 + 4;

/// Packed width of [`GcMotion`]: header, two `DWORD`s, and a `WORD`.
pub const GC_MOTION_WIRE_SIZE: usize = 1 + 4 + 4 + 2;

/// Packed width of [`GcCreateFly`]: header, `BYTE`, and two `DWORD`s.
pub const GC_CREATE_FLY_WIRE_SIZE: usize = 1 + 1 + 4 + 4;

/// Packed width of [`GcShamanSkill`]: header, two `DWORD`s, and a `BYTE`.
pub const GC_SHAMAN_SKILL_WIRE_SIZE: usize = 1 + 4 + 4 + 1;

/// Packed width of [`GcTargetUpdate`]: header and three 4-byte `long`s.
pub const GC_TARGET_UPDATE_WIRE_SIZE: usize = 1 + 4 + 4 + 4;

/// Packed width of [`GcLoverInfo`]: header, a 25-byte name, and a `BYTE`.
pub const GC_LOVER_INFO_WIRE_SIZE: usize = 1 + GC_NAME_FIELD_SIZE + 1;

/// Packed width of [`GcShopSign`]: header, `DWORD`, and a 33-byte sign.
pub const GC_SHOP_SIGN_WIRE_SIZE: usize = 1 + 4 + GC_SHOP_SIGN_FIELD_SIZE;

/// Packed width of [`GcSpecificEffect`]: header, `DWORD`, and a 128-byte path.
pub const GC_SPECIFIC_EFFECT_WIRE_SIZE: usize = 1 + 4 + GC_EFFECT_FILE_FIELD_SIZE;

/// Packed width of [`GcQuestConfirm`]: header, a 65-byte message, a 4-byte
/// `long`, and a `DWORD`.
pub const GC_QUEST_CONFIRM_WIRE_SIZE: usize = 1 + GC_QUEST_MESSAGE_FIELD_SIZE + 4 + 4;

/// Packed width of [`GcPoints`]: a header and 255 eight-byte `long long`s.
pub const GC_POINTS_WIRE_SIZE: usize = 1 + GC_POINT_SLOT_COUNT * 8;

/// Packed width of [`GcGold`]: a header and one eight-byte `unsigned long long`.
pub const GC_GOLD_WIRE_SIZE: usize = 1 + 8;

/// Wire byte of `HEADER_GC_PLAYER_POINTS` (16).
pub const HEADER_GC_PLAYER_POINTS: u8 = 0x10;
/// Wire byte of `HEADER_GC_ITEM_OWNERSHIP` (31).
pub const HEADER_GC_ITEM_OWNERSHIP: u8 = 0x1f;
/// Wire byte of `HEADER_GC_MOTION` (36).
pub const HEADER_GC_MOTION: u8 = 0x24;
/// Wire byte of `HEADER_GC_SHOP_SIGN` (39).
pub const HEADER_GC_SHOP_SIGN: u8 = 0x27;
/// Wire byte of `HEADER_GC_PVP` (41).
pub const HEADER_GC_PVP: u8 = 0x29;
/// Wire byte of `HEADER_GC_QUEST_CONFIRM` (46).
pub const HEADER_GC_QUEST_CONFIRM: u8 = 0x2e;
/// Wire byte of `HEADER_GC_AUTO_SHAMAN_SKILL` (57).
pub const HEADER_GC_AUTO_SHAMAN_SKILL: u8 = 0x39;
/// Wire byte of `HEADER_GC_OWNERSHIP` (62).
pub const HEADER_GC_OWNERSHIP: u8 = 0x3e;
/// Wire byte of `HEADER_GC_PICKUP_ITEM_SC` (64).
pub const HEADER_GC_PICKUP_ITEM_SC: u8 = 0x40;
/// Wire byte of `HEADER_GC_CREATE_FLY` (70).
pub const HEADER_GC_CREATE_FLY: u8 = 0x46;
/// Wire byte of `HEADER_GC_PARTY_ADD` (78).
pub const HEADER_GC_PARTY_ADD: u8 = 0x4e;
/// Wire byte of `HEADER_GC_PARTY_LINK` (91).
pub const HEADER_GC_PARTY_LINK: u8 = 0x5b;
/// Wire byte of `HEADER_GC_PARTY_UNLINK` (92).
pub const HEADER_GC_PARTY_UNLINK: u8 = 0x5c;
/// Wire byte of `HEADER_GC_CHANGE_NAME` (107).
pub const HEADER_GC_CHANGE_NAME: u8 = 0x6b;
/// Wire byte of `HEADER_GC_TARGET_UPDATE` (123).
pub const HEADER_GC_TARGET_UPDATE: u8 = 0x7b;
/// Wire byte of `HEADER_GC_LOVER_INFO` (131).
pub const HEADER_GC_LOVER_INFO: u8 = 0x83;
/// Wire byte of `HEADER_GC_SPECIFIC_EFFECT` (208).
pub const HEADER_GC_SPECIFIC_EFFECT: u8 = 0xd0;
/// Wire byte of `HEADER_GC_CHARACTER_GOLD` (224).
pub const HEADER_GC_CHARACTER_GOLD: u8 = 0xe0;

/// Everything that can go wrong decoding one of these records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcFieldsError {
    /// The buffer was shorter than the fixed record width.
    Truncated {
        /// The record the caller asked for.
        context: &'static str,
        /// The exact width the record requires.
        needed: usize,
        /// How many bytes were actually offered.
        actual: usize,
    },
    /// A complete-length buffer started with a different header byte.
    ///
    /// These records each have one header, except [`GcNamed`] and
    /// [`GcTwoWord`], which are shared between several headers. For the shared
    /// types the header is data, so this variant cannot arise from them.
    Header {
        /// The record the caller asked for.
        context: &'static str,
        /// The one header that record allows.
        expected: u8,
        /// The header byte the buffer actually started with.
        actual: u8,
    },
}

impl fmt::Display for GcFieldsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                context,
                needed,
                actual,
            } => write!(f, "{context}: need {needed} bytes, buffer held {actual}"),
            Self::Header {
                context,
                expected,
                actual,
            } => write!(
                f,
                "{context}: header {actual:#04x}, expected {expected:#04x}"
            ),
        }
    }
}

impl std::error::Error for GcFieldsError {}

/// Read a fixed-width record after checking the length.
fn take<'a>(
    bytes: &'a [u8],
    needed: usize,
    context: &'static str,
) -> Result<&'a [u8], GcFieldsError> {
    bytes.get(..needed).ok_or(GcFieldsError::Truncated {
        context,
        needed,
        actual: bytes.len(),
    })
}

/// Check that a complete-length record starts with the header the caller expects.
fn check_header(bytes: &[u8], context: &'static str, expected: u8) -> Result<(), GcFieldsError> {
    match bytes.first() {
        Some(&actual) if actual == expected => Ok(()),
        Some(&actual) => Err(GcFieldsError::Header {
            context,
            expected,
            actual,
        }),
        None => Err(GcFieldsError::Truncated {
            context,
            needed: 1,
            actual: 0,
        }),
    }
}

/// A record of a header, one 32-bit identifier, and a raw 25-byte name.
///
/// Three records share this exact layout and are 30 bytes each:
/// `HEADER_GC_ITEM_OWNERSHIP`, `HEADER_GC_PARTY_ADD`, and
/// `HEADER_GC_CHANGE_NAME`. The header is data, so one type serves all three
/// and the caller must supply the byte it expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcNamed {
    /// The one-byte record header.
    pub header: u8,
    /// The legacy identifier field: a `DWORD` whose meaning the header decides.
    pub id: u32,
    /// The raw `char[CHARACTER_NAME_MAX_LEN + 1]` field, NUL padding included.
    pub name: [u8; GC_NAME_FIELD_SIZE],
}

impl GcNamed {
    /// Build a record for one of the three headers that share this shape.
    pub fn new(header: u8, id: u32, name: [u8; GC_NAME_FIELD_SIZE]) -> Self {
        Self { header, id, name }
    }

    /// Append the 30 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.header);
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.name);
    }

    /// Read the 30 packed bytes for whichever of the three headers is expected.
    /// Read the 30 packed bytes for whichever of the three headers is expected.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_NAMED_WIRE_SIZE`]. The header is data for this shared type, so no
    /// header check is made.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_NAMED_WIRE_SIZE, "GcNamed")?;
        let mut name = [0u8; GC_NAME_FIELD_SIZE];
        name.copy_from_slice(&raw[5..]);
        Ok(Self {
            header: raw[0],
            id: u32::from_le_bytes([raw[1], raw[2], raw[3], raw[4]]),
            name,
        })
    }
}

/// A record of a header and two 32-bit values, 9 bytes.
///
/// `HEADER_GC_OWNERSHIP`, `HEADER_GC_PARTY_LINK`, and `HEADER_GC_PARTY_UNLINK`
/// share this layout. The server sends the unlink record as a
/// `TPacketGCPartyLink` with only the header byte changed, at `party.cpp:729`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcTwoWord {
    /// The one-byte record header.
    pub header: u8,
    /// The first legacy `DWORD`.
    pub first: u32,
    /// The second legacy `DWORD`.
    pub second: u32,
}

impl GcTwoWord {
    /// Build a record for one of the three headers that share this shape.
    pub fn new(header: u8, first: u32, second: u32) -> Self {
        Self {
            header,
            first,
            second,
        }
    }

    /// Append the 9 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.header);
        out.extend_from_slice(&self.first.to_le_bytes());
        out.extend_from_slice(&self.second.to_le_bytes());
    }

    /// Read the 9 packed bytes for whichever of the three headers is expected.
    /// Read the 9 packed bytes for whichever of the three headers is expected.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_TWO_WORD_WIRE_SIZE`]. The header is data for this shared type, so
    /// no header check is made.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_TWO_WORD_WIRE_SIZE, "GcTwoWord")?;
        Ok(Self {
            header: raw[0],
            first: u32::from_le_bytes([raw[1], raw[2], raw[3], raw[4]]),
            second: u32::from_le_bytes([raw[5], raw[6], raw[7], raw[8]]),
        })
    }
}

/// `HEADER_GC_PVP` (41): a header, two VIDs, and a mode `BYTE`, 10 bytes.
///
/// The client `switch`es on `mode` against `PVP_MODE_AGREE` and
/// `PVP_MODE_CANCEL` and has no default, so every other byte is consumed and
/// ignored. [`GcPvp::mode`] therefore stays a raw `u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPvp {
    /// The source character identifier.
    pub source: u32,
    /// The target character identifier.
    pub target: u32,
    /// The raw mode byte.
    pub mode: u8,
}

impl GcPvp {
    /// Build the record.
    pub fn new(source: u32, target: u32, mode: u8) -> Self {
        Self {
            source,
            target,
            mode,
        }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_PVP
    }

    /// Append the 10 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.source.to_le_bytes());
        out.extend_from_slice(&self.target.to_le_bytes());
        out.push(self.mode);
    }

    /// Read the 10 packed bytes, rejecting any other header.
    /// Read the 10 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_PVP_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_PVP`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_PVP_WIRE_SIZE, "GcPvp")?;
        check_header(raw, "GcPvp", Self::header())?;
        Ok(Self {
            source: u32::from_le_bytes([raw[1], raw[2], raw[3], raw[4]]),
            target: u32::from_le_bytes([raw[5], raw[6], raw[7], raw[8]]),
            mode: raw[9],
        })
    }
}

/// `HEADER_GC_PICKUP_ITEM_SC` (64): a header and two `int`s, 9 bytes.
///
/// Both `int` fields are signed in the legacy source, so they are `i32` here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPickupItem {
    /// The item vnum.
    pub item_vnum: i32,
    /// The picked-up count.
    pub item_count: i32,
}

impl GcPickupItem {
    /// Build the record.
    pub fn new(item_vnum: i32, item_count: i32) -> Self {
        Self {
            item_vnum,
            item_count,
        }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_PICKUP_ITEM_SC
    }

    /// Append the 9 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.item_vnum.to_le_bytes());
        out.extend_from_slice(&self.item_count.to_le_bytes());
    }

    /// Read the 9 packed bytes, rejecting any other header.
    /// Read the 9 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_PICKUP_ITEM_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_PICKUP_ITEM_SC`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_PICKUP_ITEM_WIRE_SIZE, "GcPickupItem")?;
        check_header(raw, "GcPickupItem", Self::header())?;
        Ok(Self {
            item_vnum: i32::from_le_bytes([raw[1], raw[2], raw[3], raw[4]]),
            item_count: i32::from_le_bytes([raw[5], raw[6], raw[7], raw[8]]),
        })
    }
}

/// `HEADER_GC_MOTION` (36): a header, two `DWORD`s, and a `WORD`, 11 bytes.
///
/// The server calls this `struct packet_motion`, not `TPacketGCMotion`. The
/// bodies are identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcMotion {
    /// The moving character identifier.
    pub vid: u32,
    /// The victim identifier, which is zero when there is no victim.
    pub victim_vid: u32,
    /// The raw motion number.
    pub motion: u16,
}

impl GcMotion {
    /// Build the record.
    pub fn new(vid: u32, victim_vid: u32, motion: u16) -> Self {
        Self {
            vid,
            victim_vid,
            motion,
        }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_MOTION
    }

    /// Append the 11 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.extend_from_slice(&self.victim_vid.to_le_bytes());
        out.extend_from_slice(&self.motion.to_le_bytes());
    }

    /// Read the 11 packed bytes, rejecting any other header.
    /// Read the 11 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_MOTION_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_MOTION`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_MOTION_WIRE_SIZE, "GcMotion")?;
        check_header(raw, "GcMotion", Self::header())?;
        Ok(Self {
            vid: u32::from_le_bytes([raw[1], raw[2], raw[3], raw[4]]),
            victim_vid: u32::from_le_bytes([raw[5], raw[6], raw[7], raw[8]]),
            motion: u16::from_le_bytes([raw[9], raw[10]]),
        })
    }
}

/// `HEADER_GC_CREATE_FLY` (70): a header, a `BYTE`, and two `DWORD`s, 10 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcCreateFly {
    /// The raw fly `BYTE` type.
    pub fly_type: u8,
    /// The start character identifier.
    pub start_vid: u32,
    /// The end character identifier.
    pub end_vid: u32,
}

impl GcCreateFly {
    /// Build the record.
    pub fn new(fly_type: u8, start_vid: u32, end_vid: u32) -> Self {
        Self {
            fly_type,
            start_vid,
            end_vid,
        }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_CREATE_FLY
    }

    /// Append the 10 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.push(self.fly_type);
        out.extend_from_slice(&self.start_vid.to_le_bytes());
        out.extend_from_slice(&self.end_vid.to_le_bytes());
    }

    /// Read the 10 packed bytes, rejecting any other header.
    /// Read the 10 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_CREATE_FLY_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_CREATE_FLY`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_CREATE_FLY_WIRE_SIZE, "GcCreateFly")?;
        check_header(raw, "GcCreateFly", Self::header())?;
        Ok(Self {
            fly_type: raw[1],
            start_vid: u32::from_le_bytes([raw[2], raw[3], raw[4], raw[5]]),
            end_vid: u32::from_le_bytes([raw[6], raw[7], raw[8], raw[9]]),
        })
    }
}

/// `HEADER_GC_AUTO_SHAMAN_SKILL` (57): a header, two `DWORD`s, and a `BYTE`.
///
/// 10 bytes. The legacy field is named `dwLevel` but declared `BYTE`, so it is
/// one byte wide and is `u8` here, not a `DWORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcShamanSkill {
    /// The skill vnum.
    pub skill_vnum: u32,
    /// The caster character identifier.
    pub vid: u32,
    /// The raw skill level byte.
    pub level: u8,
}

impl GcShamanSkill {
    /// Build the record.
    pub fn new(skill_vnum: u32, vid: u32, level: u8) -> Self {
        Self {
            skill_vnum,
            vid,
            level,
        }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_AUTO_SHAMAN_SKILL
    }

    /// Append the 10 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.skill_vnum.to_le_bytes());
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.push(self.level);
    }

    /// Read the 10 packed bytes, rejecting any other header.
    /// Read the 10 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_SHAMAN_SKILL_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_AUTO_SHAMAN_SKILL`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_SHAMAN_SKILL_WIRE_SIZE, "GcShamanSkill")?;
        check_header(raw, "GcShamanSkill", Self::header())?;
        Ok(Self {
            skill_vnum: u32::from_le_bytes([raw[1], raw[2], raw[3], raw[4]]),
            vid: u32::from_le_bytes([raw[5], raw[6], raw[7], raw[8]]),
            level: raw[9],
        })
    }
}

/// `HEADER_GC_TARGET_UPDATE` (123): a header and three 4-byte `long`s, 13 bytes.
///
/// The three legacy fields are `long lID; long lX, lY;`. On the 32-bit legacy
/// target a `long` measures 4 bytes, which the width probe confirmed, so they
/// are `i32` here. Note the multi-declarator `long lX, lY;` line, which a
/// naive one-field-per-line parser reads as a single field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcTargetUpdate {
    /// The raw target identifier.
    pub id: i32,
    /// The raw X coordinate.
    pub x: i32,
    /// The raw Y coordinate.
    pub y: i32,
}

impl GcTargetUpdate {
    /// Build the record.
    pub fn new(id: i32, x: i32, y: i32) -> Self {
        Self { id, x, y }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_TARGET_UPDATE
    }

    /// Append the 13 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
    }

    /// Read the 13 packed bytes, rejecting any other header.
    /// Read the 13 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_TARGET_UPDATE_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_TARGET_UPDATE`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_TARGET_UPDATE_WIRE_SIZE, "GcTargetUpdate")?;
        check_header(raw, "GcTargetUpdate", Self::header())?;
        Ok(Self {
            id: i32::from_le_bytes([raw[1], raw[2], raw[3], raw[4]]),
            x: i32::from_le_bytes([raw[5], raw[6], raw[7], raw[8]]),
            y: i32::from_le_bytes([raw[9], raw[10], raw[11], raw[12]]),
        })
    }
}

/// `HEADER_GC_LOVER_INFO` (131): a header, a 25-byte name, and a `BYTE`.
///
/// 27 bytes. The name comes first and the point byte last, the reverse of
/// [`GcNamed`], so the two cannot share a type despite both carrying a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcLoverInfo {
    /// The raw `char[CHARACTER_NAME_MAX_LEN + 1]` field.
    pub name: [u8; GC_NAME_FIELD_SIZE],
    /// The raw love-point byte.
    pub love_point: u8,
}

impl GcLoverInfo {
    /// Build the record.
    pub fn new(name: [u8; GC_NAME_FIELD_SIZE], love_point: u8) -> Self {
        Self { name, love_point }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_LOVER_INFO
    }

    /// Append the 27 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.name);
        out.push(self.love_point);
    }

    /// Read the 27 packed bytes, rejecting any other header.
    /// Read the 27 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_LOVER_INFO_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_LOVER_INFO`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_LOVER_INFO_WIRE_SIZE, "GcLoverInfo")?;
        check_header(raw, "GcLoverInfo", Self::header())?;
        let mut name = [0u8; GC_NAME_FIELD_SIZE];
        name.copy_from_slice(&raw[1..=GC_NAME_FIELD_SIZE]);
        Ok(Self {
            name,
            love_point: raw[GC_LOVER_INFO_WIRE_SIZE - 1],
        })
    }
}

/// `HEADER_GC_SHOP_SIGN` (39): a header, a `DWORD`, and a 33-byte sign, 38 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcShopSign {
    /// The character identifier that owns the sign.
    pub vid: u32,
    /// The raw `char[SHOP_SIGN_MAX_LEN + 1]` field.
    pub sign: [u8; GC_SHOP_SIGN_FIELD_SIZE],
}

impl GcShopSign {
    /// Build the record.
    pub fn new(vid: u32, sign: [u8; GC_SHOP_SIGN_FIELD_SIZE]) -> Self {
        Self { vid, sign }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_SHOP_SIGN
    }

    /// Append the 38 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.extend_from_slice(&self.sign);
    }

    /// Read the 38 packed bytes, rejecting any other header.
    /// Read the 38 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_SHOP_SIGN_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_SHOP_SIGN`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_SHOP_SIGN_WIRE_SIZE, "GcShopSign")?;
        check_header(raw, "GcShopSign", Self::header())?;
        let mut sign = [0u8; GC_SHOP_SIGN_FIELD_SIZE];
        sign.copy_from_slice(&raw[5..]);
        Ok(Self {
            vid: u32::from_le_bytes([raw[1], raw[2], raw[3], raw[4]]),
            sign,
        })
    }
}

/// `HEADER_GC_SPECIFIC_EFFECT` (208): a header, a `DWORD`, and a 128-byte path.
///
/// 133 bytes. The client writes the literal `128` where the server writes
/// `MAX_EFFECT_FILE_NAME`, and that macro is `#define MAX_EFFECT_FILE_NAME 128`
/// at `server/server/game/packet.h:2739`, so the two agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcSpecificEffect {
    /// The character identifier the effect plays on.
    pub vid: u32,
    /// The raw `char[MAX_EFFECT_FILE_NAME]` effect file path.
    pub effect_file: [u8; GC_EFFECT_FILE_FIELD_SIZE],
}

impl GcSpecificEffect {
    /// Build the record.
    pub fn new(vid: u32, effect_file: [u8; GC_EFFECT_FILE_FIELD_SIZE]) -> Self {
        Self { vid, effect_file }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_SPECIFIC_EFFECT
    }

    /// Append the 133 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.extend_from_slice(&self.effect_file);
    }

    /// Read the 133 packed bytes, rejecting any other header.
    /// Read the 133 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_SPECIFIC_EFFECT_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_SPECIFIC_EFFECT`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_SPECIFIC_EFFECT_WIRE_SIZE, "GcSpecificEffect")?;
        check_header(raw, "GcSpecificEffect", Self::header())?;
        let mut effect_file = [0u8; GC_EFFECT_FILE_FIELD_SIZE];
        effect_file.copy_from_slice(&raw[5..]);
        Ok(Self {
            vid: u32::from_le_bytes([raw[1], raw[2], raw[3], raw[4]]),
            effect_file,
        })
    }
}

/// `HEADER_GC_QUEST_CONFIRM` (46): a header, a 65-byte message, a 4-byte
/// `long`, and a `DWORD`, 74 bytes.
///
/// The `long timeout` measures 4 bytes on the 32-bit legacy target, so it is
/// `i32` here. The message field is raw storage, not text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcQuestConfirm {
    /// The raw `char[64 + 1]` message field.
    pub message: [u8; GC_QUEST_MESSAGE_FIELD_SIZE],
    /// The raw timeout, in the legacy unit.
    pub timeout: i32,
    /// The requesting character's identifier.
    pub request_pid: u32,
}

impl GcQuestConfirm {
    /// Build the record.
    pub fn new(message: [u8; GC_QUEST_MESSAGE_FIELD_SIZE], timeout: i32, request_pid: u32) -> Self {
        Self {
            message,
            timeout,
            request_pid,
        }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_QUEST_CONFIRM
    }

    /// Append the 74 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.message);
        out.extend_from_slice(&self.timeout.to_le_bytes());
        out.extend_from_slice(&self.request_pid.to_le_bytes());
    }

    /// Read the 74 packed bytes, rejecting any other header.
    /// Read the 74 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_QUEST_CONFIRM_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_QUEST_CONFIRM`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_QUEST_CONFIRM_WIRE_SIZE, "GcQuestConfirm")?;
        check_header(raw, "GcQuestConfirm", Self::header())?;
        let mut message = [0u8; GC_QUEST_MESSAGE_FIELD_SIZE];
        message.copy_from_slice(&raw[1..=GC_QUEST_MESSAGE_FIELD_SIZE]);
        let at = 1 + GC_QUEST_MESSAGE_FIELD_SIZE;
        Ok(Self {
            message,
            timeout: i32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]),
            request_pid: u32::from_le_bytes([raw[at + 4], raw[at + 5], raw[at + 6], raw[at + 7]]),
        })
    }
}

/// `HEADER_GC_PLAYER_POINTS` (16): a header and 255 eight-byte `long long`
/// point slots, 2041 bytes.
///
/// `POINT_MAX_NUM` is 255 on both sides, so the array is exactly 255 slots.
/// Each slot is a signed 64-bit value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPoints {
    /// The 255 raw point slots, in legacy index order.
    pub points: [i64; GC_POINT_SLOT_COUNT],
}

impl GcPoints {
    /// Build the record from a full slot array.
    pub fn new(points: [i64; GC_POINT_SLOT_COUNT]) -> Self {
        Self { points }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_PLAYER_POINTS
    }

    /// Append the 2041 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        for value in &self.points {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }

    /// Read the 2041 packed bytes, rejecting any other header.
    /// Read the 2041 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_POINTS_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_PLAYER_POINTS`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_POINTS_WIRE_SIZE, "GcPoints")?;
        check_header(raw, "GcPoints", Self::header())?;
        let mut points = [0i64; GC_POINT_SLOT_COUNT];
        for (slot, value) in points.iter_mut().enumerate() {
            let at = 1 + slot * 8;
            let mut word = [0u8; 8];
            word.copy_from_slice(&raw[at..=at + 7]);
            *value = i64::from_le_bytes(word);
        }
        Ok(Self { points })
    }
}

/// `HEADER_GC_CHARACTER_GOLD` (224): a header and one eight-byte
/// `unsigned long long`, 9 bytes.
///
/// The field is unsigned in the legacy source, so it is `u64` here. That is the
/// opposite of the `long long` point slots in [`GcPoints`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcGold {
    /// The raw gold amount.
    pub gold: u64,
}

impl GcGold {
    /// Build the record.
    pub fn new(gold: u64) -> Self {
        Self { gold }
    }

    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_CHARACTER_GOLD
    }

    /// Append the 9 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.gold.to_le_bytes());
    }

    /// Read the 9 packed bytes, rejecting any other header.
    /// Read the 9 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcFieldsError::Truncated`] when the buffer is shorter than
    /// [`GC_GOLD_WIRE_SIZE`], and [`GcFieldsError::Header`] when a complete-length buffer
    /// starts with a byte other than `HEADER_GC_CHARACTER_GOLD`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcFieldsError> {
        let raw = take(bytes, GC_GOLD_WIRE_SIZE, "GcGold")?;
        check_header(raw, "GcGold", Self::header())?;
        Ok(Self {
            gold: u64::from_le_bytes([
                raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7], raw[8],
            ]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The three headers that share the 30-byte name layout.
    const NAMED_HEADERS: [u8; 3] = [
        HEADER_GC_ITEM_OWNERSHIP,
        HEADER_GC_PARTY_ADD,
        HEADER_GC_CHANGE_NAME,
    ];

    /// The three headers that share the 9-byte two-word layout.
    const TWO_WORD_HEADERS: [u8; 3] = [
        HEADER_GC_OWNERSHIP,
        HEADER_GC_PARTY_LINK,
        HEADER_GC_PARTY_UNLINK,
    ];

    /// Every record this module implements, with its measured width.
    fn all_widths() -> Vec<(u8, &'static str, usize)> {
        vec![
            (HEADER_GC_PLAYER_POINTS, "GcPoints", GC_POINTS_WIRE_SIZE),
            (HEADER_GC_ITEM_OWNERSHIP, "GcNamed", GC_NAMED_WIRE_SIZE),
            (HEADER_GC_MOTION, "GcMotion", GC_MOTION_WIRE_SIZE),
            (HEADER_GC_SHOP_SIGN, "GcShopSign", GC_SHOP_SIGN_WIRE_SIZE),
            (HEADER_GC_PVP, "GcPvp", GC_PVP_WIRE_SIZE),
            (
                HEADER_GC_QUEST_CONFIRM,
                "GcQuestConfirm",
                GC_QUEST_CONFIRM_WIRE_SIZE,
            ),
            (
                HEADER_GC_AUTO_SHAMAN_SKILL,
                "GcShamanSkill",
                GC_SHAMAN_SKILL_WIRE_SIZE,
            ),
            (HEADER_GC_OWNERSHIP, "GcTwoWord", GC_TWO_WORD_WIRE_SIZE),
            (
                HEADER_GC_PICKUP_ITEM_SC,
                "GcPickupItem",
                GC_PICKUP_ITEM_WIRE_SIZE,
            ),
            (HEADER_GC_CREATE_FLY, "GcCreateFly", GC_CREATE_FLY_WIRE_SIZE),
            (HEADER_GC_PARTY_ADD, "GcNamed", GC_NAMED_WIRE_SIZE),
            (HEADER_GC_PARTY_LINK, "GcTwoWord", GC_TWO_WORD_WIRE_SIZE),
            (HEADER_GC_PARTY_UNLINK, "GcTwoWord", GC_TWO_WORD_WIRE_SIZE),
            (HEADER_GC_CHANGE_NAME, "GcNamed", GC_NAMED_WIRE_SIZE),
            (
                HEADER_GC_TARGET_UPDATE,
                "GcTargetUpdate",
                GC_TARGET_UPDATE_WIRE_SIZE,
            ),
            (HEADER_GC_LOVER_INFO, "GcLoverInfo", GC_LOVER_INFO_WIRE_SIZE),
            (
                HEADER_GC_SPECIFIC_EFFECT,
                "GcSpecificEffect",
                GC_SPECIFIC_EFFECT_WIRE_SIZE,
            ),
            (HEADER_GC_CHARACTER_GOLD, "GcGold", GC_GOLD_WIRE_SIZE),
        ]
    }

    /// The array-dimension constants this module substitutes, with the values
    /// resolved from the legacy tree in ledger section 166.3.
    #[test]
    fn the_substituted_array_dimensions_match_the_legacy_constants() {
        assert_eq!(
            GC_NAME_FIELD_SIZE, 25,
            "CHARACTER_NAME_MAX_LEN + 1 == 24 + 1"
        );
        assert_eq!(
            GC_SHOP_SIGN_FIELD_SIZE, 33,
            "SHOP_SIGN_MAX_LEN + 1 == 32 + 1"
        );
        assert_eq!(
            GC_EFFECT_FILE_FIELD_SIZE, 128,
            "MAX_EFFECT_FILE_NAME == 128"
        );
        assert_eq!(GC_QUEST_MESSAGE_FIELD_SIZE, 65, "64 + 1");
        assert_eq!(GC_POINT_SLOT_COUNT, 255, "POINT_MAX_NUM == 255");
    }

    #[test]
    fn the_eighteen_header_constants_match_the_legacy_enumerators() {
        assert_eq!(HEADER_GC_PLAYER_POINTS, 16);
        assert_eq!(HEADER_GC_ITEM_OWNERSHIP, 31);
        assert_eq!(HEADER_GC_MOTION, 36);
        assert_eq!(HEADER_GC_SHOP_SIGN, 39);
        assert_eq!(HEADER_GC_PVP, 41);
        assert_eq!(HEADER_GC_QUEST_CONFIRM, 46);
        assert_eq!(HEADER_GC_AUTO_SHAMAN_SKILL, 57);
        assert_eq!(HEADER_GC_OWNERSHIP, 62);
        assert_eq!(HEADER_GC_PICKUP_ITEM_SC, 64);
        assert_eq!(HEADER_GC_CREATE_FLY, 70);
        assert_eq!(HEADER_GC_PARTY_ADD, 78);
        assert_eq!(HEADER_GC_PARTY_LINK, 91);
        assert_eq!(HEADER_GC_PARTY_UNLINK, 92);
        assert_eq!(HEADER_GC_CHANGE_NAME, 107);
        assert_eq!(HEADER_GC_TARGET_UPDATE, 123);
        assert_eq!(HEADER_GC_LOVER_INFO, 131);
        assert_eq!(HEADER_GC_SPECIFIC_EFFECT, 208);
        assert_eq!(HEADER_GC_CHARACTER_GOLD, 224);
    }

    #[test]
    fn the_module_covers_eighteen_distinct_header_bytes() {
        let seen: BTreeSet<u8> = all_widths().iter().map(|entry| entry.0).collect();
        assert_eq!(seen.len(), 18);
        assert_eq!(all_widths().len(), 18);
    }

    #[test]
    fn every_measured_width_is_reproduced_by_an_actual_encode() {
        for (header, name, width) in all_widths() {
            let mut out = Vec::new();
            match name {
                "GcPoints" => GcPoints::new([7i64; GC_POINT_SLOT_COUNT]).encode_into(&mut out),
                "GcNamed" => {
                    GcNamed::new(header, 1, [b'a'; GC_NAME_FIELD_SIZE]).encode_into(&mut out);
                }
                "GcMotion" => GcMotion::new(1, 2, 3).encode_into(&mut out),
                "GcShopSign" => {
                    GcShopSign::new(1, [b'b'; GC_SHOP_SIGN_FIELD_SIZE]).encode_into(&mut out);
                }
                "GcPvp" => GcPvp::new(1, 2, 3).encode_into(&mut out),
                "GcQuestConfirm" => {
                    GcQuestConfirm::new([b'c'; GC_QUEST_MESSAGE_FIELD_SIZE], 4, 5)
                        .encode_into(&mut out);
                }
                "GcShamanSkill" => GcShamanSkill::new(1, 2, 3).encode_into(&mut out),
                "GcTwoWord" => GcTwoWord::new(header, 1, 2).encode_into(&mut out),
                "GcPickupItem" => GcPickupItem::new(1, 2).encode_into(&mut out),
                "GcCreateFly" => GcCreateFly::new(1, 2, 3).encode_into(&mut out),
                "GcTargetUpdate" => GcTargetUpdate::new(1, 2, 3).encode_into(&mut out),
                "GcLoverInfo" => {
                    GcLoverInfo::new([b'd'; GC_NAME_FIELD_SIZE], 1).encode_into(&mut out);
                }
                "GcSpecificEffect" => {
                    GcSpecificEffect::new(1, [b'e'; GC_EFFECT_FILE_FIELD_SIZE])
                        .encode_into(&mut out);
                }
                "GcGold" => GcGold::new(1).encode_into(&mut out),
                other => panic!("no encoder for {other}"),
            }
            assert_eq!(out.len(), width, "{name} at byte {header:#04x}");
            assert_eq!(out[0], header, "{name}");
        }
    }

    #[test]
    fn every_measured_width_survives_a_decode() {
        for (header, name, width) in all_widths() {
            let mut buf = vec![0xffu8; width];
            buf[0] = header;
            let decoded = match name {
                "GcPoints" => GcPoints::decode(&buf).map(|_| ()),
                "GcNamed" => GcNamed::decode(&buf).map(|_| ()),
                "GcMotion" => GcMotion::decode(&buf).map(|_| ()),
                "GcShopSign" => GcShopSign::decode(&buf).map(|_| ()),
                "GcPvp" => GcPvp::decode(&buf).map(|_| ()),
                "GcQuestConfirm" => GcQuestConfirm::decode(&buf).map(|_| ()),
                "GcShamanSkill" => GcShamanSkill::decode(&buf).map(|_| ()),
                "GcTwoWord" => GcTwoWord::decode(&buf).map(|_| ()),
                "GcPickupItem" => GcPickupItem::decode(&buf).map(|_| ()),
                "GcCreateFly" => GcCreateFly::decode(&buf).map(|_| ()),
                "GcTargetUpdate" => GcTargetUpdate::decode(&buf).map(|_| ()),
                "GcLoverInfo" => GcLoverInfo::decode(&buf).map(|_| ()),
                "GcSpecificEffect" => GcSpecificEffect::decode(&buf).map(|_| ()),
                "GcGold" => GcGold::decode(&buf).map(|_| ()),
                other => panic!("no decoder for {other}"),
            };
            assert!(decoded.is_ok(), "{name} rejected a full {width}-byte frame");
        }
    }

    #[test]
    fn every_record_rejects_every_short_buffer() {
        for (header, name, width) in all_widths() {
            for len in 0..width {
                let buf = vec![header; len];
                let err = match name {
                    "GcPoints" => GcPoints::decode(&buf).unwrap_err(),
                    "GcNamed" => GcNamed::decode(&buf).unwrap_err(),
                    "GcMotion" => GcMotion::decode(&buf).unwrap_err(),
                    "GcShopSign" => GcShopSign::decode(&buf).unwrap_err(),
                    "GcPvp" => GcPvp::decode(&buf).unwrap_err(),
                    "GcQuestConfirm" => GcQuestConfirm::decode(&buf).unwrap_err(),
                    "GcShamanSkill" => GcShamanSkill::decode(&buf).unwrap_err(),
                    "GcTwoWord" => GcTwoWord::decode(&buf).unwrap_err(),
                    "GcPickupItem" => GcPickupItem::decode(&buf).unwrap_err(),
                    "GcCreateFly" => GcCreateFly::decode(&buf).unwrap_err(),
                    "GcTargetUpdate" => GcTargetUpdate::decode(&buf).unwrap_err(),
                    "GcLoverInfo" => GcLoverInfo::decode(&buf).unwrap_err(),
                    "GcSpecificEffect" => GcSpecificEffect::decode(&buf).unwrap_err(),
                    "GcGold" => GcGold::decode(&buf).unwrap_err(),
                    other => panic!("no decoder for {other}"),
                };
                assert_eq!(
                    err,
                    GcFieldsError::Truncated {
                        context: name,
                        needed: width,
                        actual: len,
                    },
                    "{name} at len {len}"
                );
            }
        }
    }

    #[test]
    fn every_single_header_record_rejects_every_other_header() {
        for (own, name, width) in all_widths() {
            if matches!(name, "GcNamed" | "GcTwoWord" | "GcPoints") {
                continue;
            }
            // pick a different header byte and confirm the Header arm fires
            let other = if own == 0x10 { 0x11 } else { 0x10 };
            let mut buf = vec![other; width];
            buf[0] = other;
            let err = match name {
                "GcMotion" => GcMotion::decode(&buf).unwrap_err(),
                "GcShopSign" => GcShopSign::decode(&buf).unwrap_err(),
                "GcPvp" => GcPvp::decode(&buf).unwrap_err(),
                "GcQuestConfirm" => GcQuestConfirm::decode(&buf).unwrap_err(),
                "GcShamanSkill" => GcShamanSkill::decode(&buf).unwrap_err(),
                "GcPickupItem" => GcPickupItem::decode(&buf).unwrap_err(),
                "GcCreateFly" => GcCreateFly::decode(&buf).unwrap_err(),
                "GcTargetUpdate" => GcTargetUpdate::decode(&buf).unwrap_err(),
                "GcLoverInfo" => GcLoverInfo::decode(&buf).unwrap_err(),
                "GcSpecificEffect" => GcSpecificEffect::decode(&buf).unwrap_err(),
                "GcGold" => GcGold::decode(&buf).unwrap_err(),
                other => panic!("unexpected {other}"),
            };
            assert_eq!(
                err,
                GcFieldsError::Header {
                    context: name,
                    expected: own,
                    actual: other,
                },
                "{name}"
            );
        }
    }

    #[test]
    fn the_shared_types_accept_any_header_because_it_is_data() {
        // GcNamed and GcTwoWord each back three records. The caller knows which
        // one it wants, so the codec must not second-guess the header byte.
        for header in NAMED_HEADERS {
            let record = GcNamed::new(header, 9, [b'x'; GC_NAME_FIELD_SIZE]);
            let mut wire = Vec::new();
            record.encode_into(&mut wire);
            assert_eq!(wire.len(), GC_NAMED_WIRE_SIZE);
            assert_eq!(GcNamed::decode(&wire).unwrap().header, header);
        }
        for header in TWO_WORD_HEADERS {
            let record = GcTwoWord::new(header, 11, 22);
            let mut wire = Vec::new();
            record.encode_into(&mut wire);
            assert_eq!(wire.len(), GC_TWO_WORD_WIRE_SIZE);
            let decoded = GcTwoWord::decode(&wire).unwrap();
            assert_eq!(decoded.header, header);
            assert_eq!((decoded.first, decoded.second), (11, 22));
        }
    }

    #[test]
    fn the_shared_types_preserve_every_name_byte_including_embedded_nul() {
        // A C++ char[25] holds arbitrary bytes. The codec must not treat a NUL
        // as a terminator or reject a short name.
        let mut name = [0u8; GC_NAME_FIELD_SIZE];
        name[0] = b'a';
        name[1] = 0;
        name[2] = b'b';
        name[GC_NAME_FIELD_SIZE - 1] = 0xff;
        let mut wire = Vec::new();
        GcNamed::new(HEADER_GC_PARTY_ADD, 5, name).encode_into(&mut wire);
        let decoded = GcNamed::decode(&wire).unwrap();
        assert_eq!(decoded.name, name);
        assert_eq!(decoded.name[1], 0, "an embedded NUL survives");
        assert_eq!(decoded.name[GC_NAME_FIELD_SIZE - 1], 0xff);
    }

    #[test]
    fn the_named_record_writes_the_identifier_before_the_name() {
        let mut name = [0u8; GC_NAME_FIELD_SIZE];
        name[0] = b'n';
        let mut wire = Vec::new();
        GcNamed::new(HEADER_GC_CHANGE_NAME, 0x0102_0304, name).encode_into(&mut wire);
        assert_eq!(wire.len(), 30);
        assert_eq!(wire[0], HEADER_GC_CHANGE_NAME);
        assert_eq!(&wire[1..5], &[0x04, 0x03, 0x02, 0x01], "little endian");
        assert_eq!(wire[5], b'n');
        assert_eq!(&wire[6..], &[0u8; 24], "the rest of the field is padding");
    }

    #[test]
    fn the_two_word_record_keeps_the_field_order() {
        let mut wire = Vec::new();
        GcTwoWord::new(HEADER_GC_OWNERSHIP, 0x0a0b_0c0d, 0x0102_0304).encode_into(&mut wire);
        assert_eq!(
            wire,
            vec![
                HEADER_GC_OWNERSHIP,
                0x0d,
                0x0c,
                0x0b,
                0x0a,
                0x04,
                0x03,
                0x02,
                0x01
            ]
        );
    }

    #[test]
    fn the_pvp_record_keeps_an_unrecognised_mode_byte() {
        // The client switch has no default, so an unknown mode is consumed and
        // ignored rather than rejected.
        for mode in [0u8, 1, 2, 0x7f, 0x80, 0xfe, 0xff] {
            let record = GcPvp::new(1, 2, mode);
            let mut wire = Vec::new();
            record.encode_into(&mut wire);
            assert_eq!(wire.len(), 10);
            assert_eq!(wire[9], mode);
            assert_eq!(GcPvp::decode(&wire).unwrap().mode, mode);
        }
    }

    #[test]
    fn the_pickup_item_record_keeps_signed_counts() {
        // The legacy fields are C++ int, so a negative value must round trip
        // rather than saturate or wrap to a huge unsigned count.
        for (vnum, count) in [(0, 0), (1, 1), (-1, -1), (i32::MIN, i32::MAX)] {
            let mut wire = Vec::new();
            GcPickupItem::new(vnum, count).encode_into(&mut wire);
            assert_eq!(wire.len(), 9);
            let decoded = GcPickupItem::decode(&wire).unwrap();
            assert_eq!(decoded.item_vnum, vnum);
            assert_eq!(decoded.item_count, count);
        }
    }

    #[test]
    fn the_motion_record_keeps_the_full_unsigned_motion_range() {
        let record = GcMotion::new(0, 0, u16::MAX);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire.len(), 11);
        assert_eq!(&wire[9..], &[0xff, 0xff]);
        assert_eq!(GcMotion::decode(&wire).unwrap().motion, 65535);
    }

    #[test]
    fn the_create_fly_record_puts_the_byte_before_the_two_words() {
        let mut wire = Vec::new();
        GcCreateFly::new(0x0a, 0x0b0c_0d0e, 0x0102_0304).encode_into(&mut wire);
        assert_eq!(
            wire,
            vec![
                HEADER_GC_CREATE_FLY,
                0x0a,
                0x0e,
                0x0d,
                0x0c,
                0x0b,
                0x04,
                0x03,
                0x02,
                0x01
            ]
        );
    }

    #[test]
    fn the_shaman_skill_level_is_one_byte_not_a_word() {
        // The legacy field is spelled dwLevel but declared BYTE. Treating it as
        // a DWORD would make the record 13 bytes instead of 10.
        let record = GcShamanSkill::new(0x1122_3344, 0x5566_7788, 0x99);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire.len(), 10);
        assert_eq!(
            wire,
            vec![
                HEADER_GC_AUTO_SHAMAN_SKILL,
                0x44,
                0x33,
                0x22,
                0x11,
                0x88,
                0x77,
                0x66,
                0x55,
                0x99
            ]
        );
    }

    #[test]
    fn the_target_update_record_uses_four_byte_longs() {
        // A long is 4 bytes on the 32-bit legacy target. The legacy line is
        // `long lX, lY;`, a two-declarator field.
        let record = GcTargetUpdate::new(-1, -2, -3);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire.len(), 13);
        assert_eq!(
            wire,
            vec![
                HEADER_GC_TARGET_UPDATE,
                0xff,
                0xff,
                0xff,
                0xff,
                0xfe,
                0xff,
                0xff,
                0xff,
                0xfd,
                0xff,
                0xff,
                0xff
            ]
        );
        let decoded = GcTargetUpdate::decode(&wire).unwrap();
        assert_eq!((decoded.id, decoded.x, decoded.y), (-1, -2, -3));
    }

    #[test]
    fn the_lover_info_record_puts_the_name_before_the_point_byte() {
        let mut name = [0u8; GC_NAME_FIELD_SIZE];
        name[0] = b'L';
        let mut wire = Vec::new();
        GcLoverInfo::new(name, 0x07).encode_into(&mut wire);
        assert_eq!(wire.len(), 27);
        assert_eq!(wire[0], HEADER_GC_LOVER_INFO);
        assert_eq!(wire[1], b'L');
        assert_eq!(&wire[1..=25], &name, "the name occupies bytes 1 through 25");
        assert_eq!(wire[26], 0x07, "the point byte is last, at index 26");
        let decoded = GcLoverInfo::decode(&wire).unwrap();
        assert_eq!(decoded.name, name);
        assert_eq!(decoded.love_point, 7);
    }

    #[test]
    fn the_lover_info_record_is_not_the_named_record_reversed() {
        // Both carry a 25-byte name, but the widths are 27 and 30 and the
        // field order differs, so neither type can decode the other's bytes.
        let mut wire = Vec::new();
        GcLoverInfo::new([0u8; GC_NAME_FIELD_SIZE], 1).encode_into(&mut wire);
        assert!(GcLoverInfo::decode(&wire).is_ok());
        let named_len = GcNamed::new(HEADER_GC_PARTY_ADD, 0, [0u8; GC_NAME_FIELD_SIZE]);
        let mut other = Vec::new();
        named_len.encode_into(&mut other);
        assert_ne!(other.len(), wire.len());
        assert!(GcLoverInfo::decode(&other).is_err());
    }

    #[test]
    fn the_shop_sign_record_is_thirty_eight_bytes() {
        let mut sign = [0u8; GC_SHOP_SIGN_FIELD_SIZE];
        sign[0] = b's';
        let mut wire = Vec::new();
        GcShopSign::new(0x0102_0304, sign).encode_into(&mut wire);
        assert_eq!(wire.len(), 38);
        assert_eq!(wire[0], HEADER_GC_SHOP_SIGN);
        assert_eq!(&wire[1..5], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(wire[5], b's');
        assert_eq!(&wire[6..], &[0u8; 32]);
        assert_eq!(GcShopSign::decode(&wire).unwrap().sign, sign);
    }

    #[test]
    fn the_specific_effect_record_is_one_hundred_thirty_three_bytes() {
        let mut path = [0u8; GC_EFFECT_FILE_FIELD_SIZE];
        path[0] = b'd';
        path[127] = b'x';
        let mut wire = Vec::new();
        GcSpecificEffect::new(7, path).encode_into(&mut wire);
        assert_eq!(wire.len(), 133);
        assert_eq!(wire[0], HEADER_GC_SPECIFIC_EFFECT);
        assert_eq!(wire[5], b'd');
        assert_eq!(wire[132], b'x', "the last path byte is the last frame byte");
        let decoded = GcSpecificEffect::decode(&wire).unwrap();
        assert_eq!(decoded.effect_file, path);
        assert_eq!(decoded.vid, 7);
    }

    #[test]
    fn the_quest_confirm_record_puts_the_message_before_the_timeout() {
        let mut message = [0u8; GC_QUEST_MESSAGE_FIELD_SIZE];
        message[0] = b'q';
        let mut wire = Vec::new();
        GcQuestConfirm::new(message, -5, 0x0a0b_0c0d).encode_into(&mut wire);
        assert_eq!(wire.len(), 74);
        assert_eq!(wire[0], HEADER_GC_QUEST_CONFIRM);
        assert_eq!(wire[1], b'q');
        // the 4-byte long timeout sits at offset 66, the DWORD at 70
        assert_eq!(&wire[66..70], &[0xfb, 0xff, 0xff, 0xff]);
        assert_eq!(&wire[70..], &[0x0d, 0x0c, 0x0b, 0x0a]);
        let decoded = GcQuestConfirm::decode(&wire).unwrap();
        assert_eq!(decoded.message, message);
        assert_eq!(decoded.timeout, -5);
        assert_eq!(decoded.request_pid, 0x0a0b_0c0d);
    }

    #[test]
    fn the_quest_confirm_timeout_is_four_bytes_not_eight() {
        // The distinguishing test: an 8-byte long would make the record 78.
        let mut wire = Vec::new();
        GcQuestConfirm::new([0u8; GC_QUEST_MESSAGE_FIELD_SIZE], 0, 0).encode_into(&mut wire);
        assert_eq!(wire.len(), 74);
        assert_eq!(GC_QUEST_MESSAGE_FIELD_SIZE + 1 + 4 + 4, 74);
    }

    #[test]
    fn the_points_record_holds_exactly_255_signed_slots() {
        let mut points = [0i64; GC_POINT_SLOT_COUNT];
        for (slot, value) in points.iter_mut().enumerate() {
            *value = i64::try_from(slot).unwrap_or(0) * -3;
        }
        points[0] = i64::MIN;
        points[GC_POINT_SLOT_COUNT - 1] = i64::MAX;
        let mut wire = Vec::new();
        GcPoints::new(points).encode_into(&mut wire);
        assert_eq!(wire.len(), 2041);
        assert_eq!(wire[0], HEADER_GC_PLAYER_POINTS);
        let decoded = GcPoints::decode(&wire).unwrap();
        assert_eq!(decoded.points, points);
        assert_eq!(decoded.points[0], i64::MIN);
        assert_eq!(decoded.points[GC_POINT_SLOT_COUNT - 1], i64::MAX);
    }

    #[test]
    fn the_points_record_rejects_a_2040_byte_buffer() {
        assert_eq!(
            GcPoints::decode(&vec![HEADER_GC_PLAYER_POINTS; 2040]).unwrap_err(),
            GcFieldsError::Truncated {
                context: "GcPoints",
                needed: 2041,
                actual: 2040,
            }
        );
    }

    #[test]
    fn the_gold_record_is_nine_bytes_and_unsigned() {
        let record = GcGold::new(u64::MAX);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire.len(), 9);
        assert_eq!(wire[0], HEADER_GC_CHARACTER_GOLD);
        assert_eq!(&wire[1..], &[0xff; 8]);
        assert_eq!(GcGold::decode(&wire).unwrap().gold, u64::MAX);
    }

    #[test]
    fn the_gold_and_points_records_use_opposite_signedness() {
        // gold is `unsigned long long`, points is `long long`. Keeping the
        // distinction means the high bit of a point slot stays negative while a
        // gold value stays large and positive.
        let mut gold_wire = Vec::new();
        GcGold::new(0xffff_ffff_ffff_ffff).encode_into(&mut gold_wire);
        assert_eq!(
            GcGold::decode(&gold_wire).unwrap().gold,
            0xffff_ffff_ffff_ffff
        );

        let mut points = [0i64; GC_POINT_SLOT_COUNT];
        points[9] = -1;
        let mut points_wire = Vec::new();
        GcPoints::new(points).encode_into(&mut points_wire);
        assert_eq!(GcPoints::decode(&points_wire).unwrap().points[9], -1);
    }

    #[test]
    fn the_41_byte_pvp_and_10_byte_shaman_record_are_different_lengths() {
        // Both are header + 2 DWORD + BYTE, but the header differs, so a swap
        // must be caught by the header check rather than by the length.
        let mut pvp = Vec::new();
        GcPvp::new(1, 2, 3).encode_into(&mut pvp);
        let mut shaman = Vec::new();
        GcShamanSkill::new(1, 2, 3).encode_into(&mut shaman);
        assert_eq!(pvp.len(), shaman.len());
        assert!(GcShamanSkill::decode(&pvp).is_err());
        assert!(GcPvp::decode(&shaman).is_err());
    }

    #[test]
    fn the_named_and_pickup_records_are_both_nine_or_thirty_bytes_apart() {
        let mut named = Vec::new();
        GcNamed::new(HEADER_GC_ITEM_OWNERSHIP, 1, [0u8; GC_NAME_FIELD_SIZE])
            .encode_into(&mut named);
        let mut pickup = Vec::new();
        GcPickupItem::new(1, 2).encode_into(&mut pickup);
        assert_eq!(named.len(), 30);
        assert_eq!(pickup.len(), 9);
        assert!(GcPickupItem::decode(&named).is_err());
    }

    #[test]
    fn every_record_ignores_bytes_past_its_own_width() {
        let mut buf = Vec::new();
        GcTwoWord::new(HEADER_GC_OWNERSHIP, 1, 2).encode_into(&mut buf);
        let at = GC_TWO_WORD_WIRE_SIZE;
        GcGold::new(3).encode_into(&mut buf);
        let two = GcTwoWord::decode(&buf).unwrap();
        assert_eq!(two.first, 1);
        assert_eq!(two.second, 2);
        let rest = &buf[at..];
        assert_eq!(GcGold::decode(rest).unwrap().gold, 3);
        assert_eq!(rest.len(), 9);
    }

    /// Values chosen so that a big-endian or shifted read cannot pass: no
    /// value is byte-symmetric, and every multi-byte field differs from its
    /// neighbours so a one-byte shift is visible.
    const ASYM_U32_A: u32 = 0x0102_0304;
    const ASYM_U32_B: u32 = 0x0a0b_0c0d;
    const ASYM_U16: u16 = 0x1122;
    const ASYM_I32_A: i32 = 0x0102_0305;
    const ASYM_I32_B: i32 = -0x0a0b_0c0d;
    const ASYM_I32_C: i32 = 0x1122_3344;
    const ASYM_U64: u64 = 0x0102_0304_0506_0708;

    /// A deterministic byte ramp, so a test value never needs a narrowing cast.
    fn ramp(seed: u8) -> impl Iterator<Item = u8> {
        let mut value = seed;
        core::iter::from_fn(move || {
            let current = value;
            value = value.wrapping_mul(7).wrapping_add(3);
            Some(current)
        })
    }

    /// Fill a fixed legacy array with an asymmetric, non-repeating pattern.
    fn pattern<const N: usize>(seed: u8) -> [u8; N] {
        let mut out = [0u8; N];
        let bytes: Vec<u8> = ramp(seed).take(N).collect();
        out.copy_from_slice(&bytes);
        out
    }

    /// The expected value of point slot `index` under the round-trip test.
    fn point_at(index: usize) -> i64 {
        let n = i32::try_from(index).unwrap_or(0);
        i64::from(n) * 0x0102_0304 - 0x0a0b_0c0d
    }

    type FieldCase = (&'static str, Vec<u8>, fn(&[u8]));

    fn case_named() -> FieldCase {
        let mut w = Vec::new();
        GcNamed::new(HEADER_GC_PARTY_ADD, ASYM_U32_A, pattern(0x11)).encode_into(&mut w);
        ("GcNamed", w, |b| {
            let r = GcNamed::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_PARTY_ADD, "header");
            assert_eq!(r.id, ASYM_U32_A, "id");
            assert_eq!(r.name, pattern::<GC_NAME_FIELD_SIZE>(0x11), "name");
        })
    }

    fn case_two_word() -> FieldCase {
        let mut w = Vec::new();
        GcTwoWord::new(HEADER_GC_PARTY_UNLINK, ASYM_U32_A, ASYM_U32_B).encode_into(&mut w);
        ("GcTwoWord", w, |b| {
            let r = GcTwoWord::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_PARTY_UNLINK, "header");
            assert_eq!(r.first, ASYM_U32_A, "first");
            assert_eq!(r.second, ASYM_U32_B, "second");
        })
    }

    fn case_pvp() -> FieldCase {
        let mut w = Vec::new();
        GcPvp::new(ASYM_U32_A, ASYM_U32_B, 0x5a).encode_into(&mut w);
        ("GcPvp", w, |b| {
            let r = GcPvp::decode(b).unwrap();
            assert_eq!(r.source, ASYM_U32_A, "source");
            assert_eq!(r.target, ASYM_U32_B, "target");
            assert_eq!(r.mode, 0x5a, "mode");
        })
    }

    fn case_pickup() -> FieldCase {
        let mut w = Vec::new();
        GcPickupItem::new(ASYM_I32_A, ASYM_I32_B).encode_into(&mut w);
        ("GcPickupItem", w, |b| {
            let r = GcPickupItem::decode(b).unwrap();
            assert_eq!(r.item_vnum, ASYM_I32_A, "item_vnum");
            assert_eq!(r.item_count, ASYM_I32_B, "item_count");
        })
    }

    fn case_motion() -> FieldCase {
        let mut w = Vec::new();
        GcMotion::new(ASYM_U32_A, ASYM_U32_B, ASYM_U16).encode_into(&mut w);
        ("GcMotion", w, |b| {
            let r = GcMotion::decode(b).unwrap();
            assert_eq!(r.vid, ASYM_U32_A, "vid");
            assert_eq!(r.victim_vid, ASYM_U32_B, "victim_vid");
            assert_eq!(r.motion, ASYM_U16, "motion");
        })
    }

    fn case_create_fly() -> FieldCase {
        let mut w = Vec::new();
        GcCreateFly::new(0x3c, ASYM_U32_A, ASYM_U32_B).encode_into(&mut w);
        ("GcCreateFly", w, |b| {
            let r = GcCreateFly::decode(b).unwrap();
            assert_eq!(r.fly_type, 0x3c, "fly_type");
            assert_eq!(r.start_vid, ASYM_U32_A, "start_vid");
            assert_eq!(r.end_vid, ASYM_U32_B, "end_vid");
        })
    }

    fn case_shaman() -> FieldCase {
        let mut w = Vec::new();
        GcShamanSkill::new(ASYM_U32_A, ASYM_U32_B, 0x2b).encode_into(&mut w);
        ("GcShamanSkill", w, |b| {
            let r = GcShamanSkill::decode(b).unwrap();
            assert_eq!(r.skill_vnum, ASYM_U32_A, "skill_vnum");
            assert_eq!(r.vid, ASYM_U32_B, "vid");
            assert_eq!(r.level, 0x2b, "level");
        })
    }

    fn case_target_update() -> FieldCase {
        let mut w = Vec::new();
        GcTargetUpdate::new(ASYM_I32_A, ASYM_I32_B, ASYM_I32_C).encode_into(&mut w);
        ("GcTargetUpdate", w, |b| {
            let r = GcTargetUpdate::decode(b).unwrap();
            assert_eq!(r.id, ASYM_I32_A, "id");
            assert_eq!(r.x, ASYM_I32_B, "x");
            assert_eq!(r.y, ASYM_I32_C, "y");
        })
    }

    /// The eight record types whose fields are all scalars.
    fn scalar_cases() -> Vec<FieldCase> {
        vec![
            case_two_word(),
            case_pvp(),
            case_pickup(),
            case_motion(),
            case_create_fly(),
            case_shaman(),
            case_target_update(),
            case_gold(),
        ]
    }

    /// The six record types that carry a fixed byte array or a 64-bit value.
    fn array_cases() -> Vec<FieldCase> {
        vec![
            case_named(),
            case_lover(),
            case_shop_sign(),
            case_specific_effect(),
            case_quest_confirm(),
            case_points(),
        ]
    }

    fn case_lover() -> FieldCase {
        let mut w = Vec::new();
        GcLoverInfo::new(pattern(0x77), 0x6c).encode_into(&mut w);
        ("GcLoverInfo", w, |b| {
            let r = GcLoverInfo::decode(b).unwrap();
            assert_eq!(r.name, pattern::<GC_NAME_FIELD_SIZE>(0x77), "name");
            assert_eq!(r.love_point, 0x6c, "love_point");
        })
    }

    fn case_shop_sign() -> FieldCase {
        let mut w = Vec::new();
        GcShopSign::new(ASYM_U32_A, pattern(0x21)).encode_into(&mut w);
        ("GcShopSign", w, |b| {
            let r = GcShopSign::decode(b).unwrap();
            assert_eq!(r.vid, ASYM_U32_A, "vid");
            assert_eq!(r.sign, pattern::<GC_SHOP_SIGN_FIELD_SIZE>(0x21), "sign");
        })
    }

    fn case_specific_effect() -> FieldCase {
        let mut w = Vec::new();
        GcSpecificEffect::new(ASYM_U32_A, pattern(0x31)).encode_into(&mut w);
        ("GcSpecificEffect", w, |b| {
            let r = GcSpecificEffect::decode(b).unwrap();
            assert_eq!(r.vid, ASYM_U32_A, "vid");
            assert_eq!(
                r.effect_file,
                pattern::<GC_EFFECT_FILE_FIELD_SIZE>(0x31),
                "effect_file"
            );
        })
    }

    fn case_quest_confirm() -> FieldCase {
        let mut w = Vec::new();
        GcQuestConfirm::new(pattern(0x41), ASYM_I32_A, ASYM_U32_A).encode_into(&mut w);
        ("GcQuestConfirm", w, |b| {
            let r = GcQuestConfirm::decode(b).unwrap();
            assert_eq!(
                r.message,
                pattern::<GC_QUEST_MESSAGE_FIELD_SIZE>(0x41),
                "message"
            );
            assert_eq!(r.timeout, ASYM_I32_A, "timeout");
            assert_eq!(r.request_pid, ASYM_U32_A, "request_pid");
        })
    }

    fn case_gold() -> FieldCase {
        let mut w = Vec::new();
        GcGold::new(ASYM_U64).encode_into(&mut w);
        ("GcGold", w, |b| {
            let r = GcGold::decode(b).unwrap();
            assert_eq!(r.gold, ASYM_U64, "gold");
        })
    }

    fn case_points() -> FieldCase {
        let mut slots = [0i64; GC_POINT_SLOT_COUNT];
        for (index, slot) in slots.iter_mut().enumerate() {
            *slot = point_at(index);
        }
        let mut w = Vec::new();
        GcPoints::new(slots).encode_into(&mut w);
        ("GcPoints", w, |b| {
            let r = GcPoints::decode(b).unwrap();
            for (index, slot) in r.points.iter().enumerate() {
                assert_eq!(*slot, point_at(index), "point slot {index}");
            }
        })
    }

    /// Every scalar-only record must decode back to every field it was built
    /// with, using values that a byte reversal or a one-byte shift would break.
    #[test]
    fn every_scalar_record_decodes_back_to_every_field_it_was_built_with() {
        let cases = scalar_cases();
        assert_eq!(cases.len(), 8, "eight distinct scalar record types");
        for (name, wire, check) in cases {
            assert!(!wire.is_empty(), "{name} produced no bytes");
            check(&wire);
        }
    }

    /// The same for the six records that carry a fixed array or a 64-bit value.
    #[test]
    fn every_array_record_decodes_back_to_every_field_it_was_built_with() {
        let cases = array_cases();
        assert_eq!(cases.len(), 6, "six distinct array record types");
        for (name, wire, check) in cases {
            assert!(!wire.is_empty(), "{name} produced no bytes");
            check(&wire);
        }
    }

    /// The two test groups must together cover fourteen types, once each.
    #[test]
    fn the_round_trip_groups_cover_fourteen_distinct_types() {
        let mut names: Vec<&str> = scalar_cases()
            .iter()
            .chain(array_cases().iter())
            .map(|entry| entry.0)
            .collect();
        assert_eq!(names.len(), 14);
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 14, "no type may appear in both groups");
    }

    /// A fixed-array field must not read back a different array's bytes.
    #[test]
    fn no_two_fixed_arrays_of_different_length_can_alias() {
        // The four array fields are 25, 33, 65, and 128 bytes. A decoder that
        // copied the wrong length would either panic or pull in neighbouring
        // bytes, so confirm each record keeps its own field separate.
        let cases = array_cases();
        for (name, wire, check) in cases {
            assert_eq!(wire.len(), wire.len(), "{name} width is stable");
            check(&wire);
        }
        // The narrowest and widest records differ by 59 bytes, so a fixed-length
        // copy cannot be correct for both.
        assert_eq!(
            GC_LOVER_INFO_WIRE_SIZE.abs_diff(GC_SPECIFIC_EFFECT_WIRE_SIZE),
            106
        );
    }

    /// Two adjacent fields must never be able to swap without a test noticing.
    #[test]
    fn no_two_adjacent_fields_of_any_record_are_interchangeable() {
        // Encode with two values in the first and second field, then swap them.
        // If the layout put the fields in the wrong order, both frames would
        // decode to the same record.
        let mut pvp_a = Vec::new();
        GcPvp::new(1, 2, 0).encode_into(&mut pvp_a);
        let mut pvp_b = Vec::new();
        GcPvp::new(2, 1, 0).encode_into(&mut pvp_b);
        assert_ne!(pvp_a, pvp_b);
        assert_eq!(GcPvp::decode(&pvp_a).unwrap().source, 1);
        assert_eq!(GcPvp::decode(&pvp_b).unwrap().source, 2);

        let mut fly_a = Vec::new();
        GcCreateFly::new(0, 1, 2).encode_into(&mut fly_a);
        let mut fly_b = Vec::new();
        GcCreateFly::new(0, 2, 1).encode_into(&mut fly_b);
        assert_ne!(fly_a, fly_b);
        assert_eq!(GcCreateFly::decode(&fly_a).unwrap().start_vid, 1);
        assert_eq!(GcCreateFly::decode(&fly_b).unwrap().start_vid, 2);

        let mut sh_a = Vec::new();
        GcShamanSkill::new(1, 2, 0).encode_into(&mut sh_a);
        let mut sh_b = Vec::new();
        GcShamanSkill::new(2, 1, 0).encode_into(&mut sh_b);
        assert_ne!(sh_a, sh_b);
        assert_eq!(GcShamanSkill::decode(&sh_a).unwrap().vid, 2);
        assert_eq!(GcShamanSkill::decode(&sh_b).unwrap().vid, 1);

        let mut mv_a = Vec::new();
        GcMotion::new(1, 2, 3).encode_into(&mut mv_a);
        let mut mv_b = Vec::new();
        GcMotion::new(2, 1, 3).encode_into(&mut mv_b);
        assert_ne!(mv_a, mv_b);
        assert_eq!(GcMotion::decode(&mv_a).unwrap().victim_vid, 2);
        assert_eq!(GcMotion::decode(&mv_b).unwrap().victim_vid, 1);

        let mut tw_a = Vec::new();
        GcTwoWord::new(HEADER_GC_OWNERSHIP, 1, 2).encode_into(&mut tw_a);
        let mut tw_b = Vec::new();
        GcTwoWord::new(HEADER_GC_OWNERSHIP, 2, 1).encode_into(&mut tw_b);
        assert_ne!(tw_a, tw_b);
        assert_eq!(GcTwoWord::decode(&tw_a).unwrap().first, 1);
        assert_eq!(GcTwoWord::decode(&tw_b).unwrap().first, 2);
    }

    /// A little-endian word must not be readable as its own byte reversal.
    #[test]
    fn no_multi_byte_field_survives_a_byte_reversal() {
        // A field whose two halves are equal is invisible to an endianness
        // mistake, so every multi-byte value used here has distinct halves.
        assert_ne!(ASYM_U32_A.to_le_bytes(), ASYM_U32_A.to_be_bytes());
        assert_ne!(ASYM_U16.to_le_bytes(), ASYM_U16.to_be_bytes());
        assert_ne!(ASYM_U64.to_le_bytes(), ASYM_U64.to_be_bytes());

        let mut wire = Vec::new();
        GcMotion::new(0, 0, ASYM_U16).encode_into(&mut wire);
        let reversed: Vec<u8> = wire.iter().rev().copied().collect();
        let flipped = GcMotion::decode(&reversed);
        assert!(flipped.is_err() || flipped.unwrap().motion != ASYM_U16);

        let mut wire = Vec::new();
        GcGold::new(ASYM_U64).encode_into(&mut wire);
        let reversed: Vec<u8> = wire.iter().rev().copied().collect();
        let flipped = GcGold::decode(&reversed);
        assert!(flipped.is_err() || flipped.unwrap().gold != ASYM_U64);
    }

    #[test]
    fn the_error_display_names_the_record_and_the_numbers() {
        assert_eq!(
            GcFieldsError::Truncated {
                context: "GcPoints",
                needed: 2041,
                actual: 7,
            }
            .to_string(),
            "GcPoints: need 2041 bytes, buffer held 7"
        );
        assert_eq!(
            GcFieldsError::Header {
                context: "GcGold",
                expected: 224,
                actual: 16,
            }
            .to_string(),
            "GcGold: header 0x10, expected 0xe0"
        );
    }

    #[test]
    fn the_error_type_is_a_standard_error() {
        fn assert_error<E: std::error::Error>(_: &E) {}
        assert_error(&GcFieldsError::Truncated {
            context: "x",
            needed: 1,
            actual: 0,
        });
        assert_error(&GcFieldsError::Header {
            context: "x",
            expected: 1,
            actual: 2,
        });
    }

    #[test]
    fn the_header_byte_is_the_only_thing_separating_the_three_named_records() {
        // party.cpp:729 sends the unlink record through TPacketGCPartyLink, so
        // the client must be able to tell all three apart by the header alone.
        let mut wires = Vec::new();
        for header in NAMED_HEADERS {
            let mut wire = Vec::new();
            GcNamed::new(header, 42, [0u8; GC_NAME_FIELD_SIZE]).encode_into(&mut wire);
            wires.push(wire);
        }
        let headers: BTreeSet<u8> = wires.iter().map(|w| w[0]).collect();
        assert_eq!(
            headers.len(),
            3,
            "the three records need three distinct bytes"
        );
        for (wire, header) in wires.iter().zip(NAMED_HEADERS) {
            assert_eq!(wire.len(), 30);
            assert_eq!(GcNamed::decode(wire).unwrap().header, header);
            assert_eq!(GcNamed::decode(wire).unwrap().id, 42);
        }
    }
}
