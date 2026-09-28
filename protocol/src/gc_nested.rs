//! The game-to-client records that nest another record or carry a fixed array.
//!
//! This is the second batch of named-field game-to-client codecs, after
//! `gc_fields`. It covers the 12 records that either embed a nested struct or
//! hold an array of elements, and it adds the three nested types they need.
//!
//! # Provenance and how every width here was obtained
//!
//! No width in this module is hand-summed. Each struct body was extracted
//! verbatim from `client/Client/UserInterface/Packet.h` and
//! `server/server/game/packet.h`, the array dimensions were substituted with
//! resolved constants, and the result was compiled with `i686-linux-gnu-g++-12`
//! and run under `sizeof`. See ledger section 167 for the transcript.
//!
//! **The probe caught two errors in my own hand sums**, which is the reason for
//! running it rather than trusting arithmetic:
//!
//! - `TPacketGCDigMotion` is **10** bytes, not the 14 I first added up. The
//!   body is a header, two `DWORD`s, and a `BYTE`.
//! - `TPlayerSkill` is **6** bytes only because `time_t` is 4 on this target.
//!   My first probe typedef'd `time_t` as `int64_t` and measured 10, which made
//!   `TPacketGCSkillLevelNew` 2551 instead of 1531.
//!
//! A control run confirmed `time_t` at 4 bytes and a packed two-`long` struct at
//! 8 bytes, so the probe really is 32-bit.
//!
//! # Two records that the byte value, not the name, has to pair
//!
//! The skill-level record is registered twice on the client and the two rows
//! have **different bodies**:
//!
//! | byte | client name | server name | body | width |
//! | --- | --- | --- | --- | --- |
//! | 76 | `HEADER_GC_SKILL_LEVEL_NEW` | `HEADER_GC_SKILL_LEVEL` | `TPlayerSkill skills[255]` | 1531 |
//! | 72 | `HEADER_GC_SKILL_LEVEL` | `HEADER_GC_SKILL_LEVEL_OLD` | `BYTE abSkillLevels[255]` | 256 |
//!
//! `char_skill.cpp:184` sends byte 76 with `sizeof(TPacketGCSkillLevel)`, and the
//! server's byte-72 enumerator at `packet.h:161` is declared and never used. So
//! byte 72 is dead on both sides, and **the live record is byte 76 at 1531
//! bytes**. This is the `_OLD`/`_NEW` generation-suffix pattern recorded in
//! ledger section 163, and it is why [`GcSkillLevelNew`] is keyed to
//! [`HEADER_GC_SKILL_LEVEL_NEW`] and never to the byte-72 name.
//!
//! # A field name that lies about its own width
//!
//! `TPacketGCPointChange` declares `int header`, not `BYTE header`, so its
//! first field is **4** bytes and the record is 25 rather than 22. The Rust
//! field is `header: i32` for the same reason.
//!
//! # A record whose client and server disagree in spelling but not in layout
//!
//! `TPacketGCPartyUpdate` declares `short affects[PARTY_AFFECT_SLOT_MAX_NUM]`
//! on the client and `short affects[7]` on the server. `PARTY_AFFECT_SLOT_MAX_NUM`
//! is 7 (`client/Client/UserInterface/Packet.h:265`) and has no server
//! definition at all, so the two agree by coincidence rather than by a shared
//! constant. The probe measured 21 from both bodies.
//!
//! The same shape appears in `TPacketGCFlyTargeting`, where the client writes
//! `long lX; long lY;` on two lines and the server writes `long x, y;` on one.
//! Both are two 4-byte fields, so both measure 17.
//!
//! # A nested record whose `time_t` matters
//!
//! `TPlayerSkill` is `{ BYTE bMasterType; BYTE bLevel; time_t tNextRead; }`.
//! `time_t` is **4** bytes here: `server/server/premake5.lua:12` sets
//! `architecture "x86"` and there is no `_TIME_BITS=64` anywhere in the tree.
//! The field is signed, so it is `i32`.
//!
//! # Records excluded from this batch, and why
//!
//! These are pending static-size game-to-client records that this module
//! deliberately does not implement:
//!
//! - `TPacketGCExchange` (42) has an array dimension of
//!   `ITEM_ATTRIBUTE_SLOT_MAX_NUM`, one of the five constants that are genuinely
//!   absent from the tree, and it sits behind `WJ_ENABLE_TRADABLE_ICON`. The
//!   server sends it as a plain `struct packet_exchange` (`exchange.cpp:36`).
//! - `TPacketGCMount` (61) has an enumerator in `packet.h:150` and **no send
//!   site anywhere in the server tree**.
//! - `TPacketGCTarget`, `TPacketGCTargetInfo`, and `TPacketGCWhisper` sit
//!   behind profile gates whose macro **names differ between the two trees**:
//!   `ENABLE_VIEW_TARGET_DECIMAL_HP` against `__VIEW_TARGET_DECIMAL_HP__`, and
//!   `ENABLE_SHIP_DEFENSE` against `__SHIP_DEFENSE__`.
//! - `TPacketGCBiologist` has 24 fields and is left for a dedicated batch.
//!
//! # Scope
//!
//! Record codecs only. Nothing here applies an affect, casts a skill, warps a
//! character, moves a party member, or mutates a session. The `u8` and `i32`
//! fields keep every value, and the fixed `char` arrays stay raw bytes with no
//! text decoding and no NUL validation.

use std::fmt;

use crate::gc::{
    HEADER_GC_ADD_FLY_TARGETING, HEADER_GC_AFFECT_ADD, HEADER_GC_CUBE_RENEWAL,
    HEADER_GC_DAILY_GIFT, HEADER_GC_DAMAGE_INFO, HEADER_GC_DIG_MOTION, HEADER_GC_FLY_TARGETING,
    HEADER_GC_PARTY_UPDATE, HEADER_GC_PLAYER_POINT_CHANGE, HEADER_GC_PREMIUM_PLAYERS,
    HEADER_GC_SKILL_LEVEL_NEW, HEADER_GC_WARP,
};

/// `SKILL_MAX_NUM`, from `server/server/common/length.h:72` and
/// `client/Client/UserInterface/Packet.h:2135`.
pub const GC_SKILL_SLOT_COUNT: usize = 255;

/// `DAILY_GIFT_WEEK_DAYS`, from `server/server/common/length.h:1190` and
/// `client/Client/UserInterface/GameType.h:147`.
pub const GC_DAILY_GIFT_DAY_COUNT: usize = 7;

/// `PARTY_AFFECT_SLOT_MAX_NUM`, from
/// `client/Client/UserInterface/Packet.h:265`. The server has no definition and
/// writes the literal `7` in its own body.
pub const GC_PARTY_AFFECT_SLOT_COUNT: usize = 7;

/// Packed width of [`GcSkill`]: two `BYTE`s and a 4-byte `time_t`.
pub const GC_SKILL_WIRE_SIZE: usize = 1 + 1 + 4;

/// Packed width of [`GcAffectElement`].
pub const GC_AFFECT_ELEMENT_WIRE_SIZE: usize = 4 + 1 + 4 + 4 + 4 + 4;

/// Packed width of [`GcAffectAdd`]: header plus one nested element.
pub const GC_AFFECT_ADD_WIRE_SIZE: usize = 1 + GC_AFFECT_ELEMENT_WIRE_SIZE;

/// Packed width of [`GcPointChange`]: an `int` header, `DWORD`, `BYTE`, two
/// `long long`s.
pub const GC_POINT_CHANGE_WIRE_SIZE: usize = 4 + 4 + 1 + 8 + 8;

/// The header value of [`GcPointChange`], as the signed word the record stores.
///
/// The legacy field is `int header`, so the byte is widened to a signed 32-bit
/// word rather than kept as a `u8`.
pub const GC_POINT_CHANGE_HEADER: i32 = HEADER_GC_PLAYER_POINT_CHANGE as i32;

/// Packed width of [`GcDigMotion`]: header, two `DWORD`s, and a `BYTE`.
pub const GC_DIG_MOTION_WIRE_SIZE: usize = 1 + 4 + 4 + 1;

/// Packed width of [`GcDamageInfo`]: header, `DWORD`, `BYTE`, and `int`.
pub const GC_DAMAGE_INFO_WIRE_SIZE: usize = 1 + 4 + 1 + 4;

/// Packed width of [`GcPremiumPlayers`]: three `BYTE`s and a 25-byte name.
pub const GC_PREMIUM_PLAYERS_WIRE_SIZE: usize = 1 + 1 + 1 + 25;

/// Packed width of the shared [`GcFlyTargeting`] records.
pub const GC_FLY_TARGETING_WIRE_SIZE: usize = 1 + 4 + 4 + 4 + 4;

/// The two headers the shared [`GcFlyTargeting`] body carries.
pub const GC_FLY_TARGETING_HEADERS: [u8; 2] =
    [HEADER_GC_ADD_FLY_TARGETING, HEADER_GC_FLY_TARGETING];

/// Packed width of [`GcWarp`]: header, three `long`s, and a `WORD`.
pub const GC_WARP_WIRE_SIZE: usize = 1 + 4 + 4 + 4 + 2;

/// Packed width of [`GcSkillLevelNew`]: header plus 255 nested skills.
pub const GC_SKILL_LEVEL_NEW_WIRE_SIZE: usize = 1 + GC_SKILL_SLOT_COUNT * GC_SKILL_WIRE_SIZE;

/// Packed width of the dead byte-72 generation: header plus 255 raw levels.
pub const GC_SKILL_LEVEL_OLD_WIRE_SIZE: usize = 1 + 255;

/// Packed width of [`GcDailyGift`].
pub const GC_DAILY_GIFT_WIRE_SIZE: usize =
    1 + 4 + 1 + GC_DAILY_GIFT_DAY_COUNT * (4 + 4 + 4 + 2 + 1);

/// Packed width of [`GcPartyUpdate`]: header, `DWORD`, two `BYTE`s, seven
/// `short`s.
pub const GC_PARTY_UPDATE_WIRE_SIZE: usize = 1 + 4 + 1 + 1 + 2 * GC_PARTY_AFFECT_SLOT_COUNT;

/// Packed width of the 100-byte `category` field of [`GcCubeRenewalDate`].
pub const GC_CUBE_CATEGORY_SIZE: usize = 100;

/// Packed width of [`GcCubeRenewalDate`].
pub const GC_CUBE_RENEWAL_DATE_WIRE_SIZE: usize =
    4 + 4 + 4 + 4 + 1 + 5 * (4 + 4) + 8 + 4 + GC_CUBE_CATEGORY_SIZE;

/// Packed width of [`GcCubeRenewal`]: two `BYTE`s plus the nested date record.
pub const GC_CUBE_RENEWAL_WIRE_SIZE: usize = 1 + 1 + GC_CUBE_RENEWAL_DATE_WIRE_SIZE;

/// A fixed-width decode failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcNestedError {
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
    Header {
        /// The record the caller asked for.
        context: &'static str,
        /// The one header that record allows.
        expected: u8,
        /// The header byte the buffer actually started with.
        actual: u8,
    },
}

impl fmt::Display for GcNestedError {
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

impl std::error::Error for GcNestedError {}

/// Read a fixed-width record after checking the length.
fn take<'a>(
    bytes: &'a [u8],
    needed: usize,
    context: &'static str,
) -> Result<&'a [u8], GcNestedError> {
    bytes.get(..needed).ok_or(GcNestedError::Truncated {
        context,
        needed,
        actual: bytes.len(),
    })
}

/// Reject a buffer whose header byte is not the one this record carries.
fn check_header(bytes: &[u8], context: &'static str, expected: u8) -> Result<(), GcNestedError> {
    match bytes.first() {
        Some(&actual) if actual == expected => Ok(()),
        Some(&actual) => Err(GcNestedError::Header {
            context,
            expected,
            actual,
        }),
        None => Err(GcNestedError::Truncated {
            context,
            needed: 1,
            actual: 0,
        }),
    }
}

/// Read a little-endian `u32` at `at`.
fn u32_at(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]])
}

/// Read a little-endian `i32` at `at`.
fn i32_at(raw: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]])
}

/// Read a little-endian `i64` at `at`.
fn i64_at(raw: &[u8], at: usize) -> i64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&raw[at..at + 8]);
    i64::from_le_bytes(word)
}

/// One entry of a skill table, the nested body of [`GcSkillLevelNew`].
///
/// Legacy `TPlayerSkill` is `{ BYTE bMasterType; BYTE bLevel; time_t
/// tNextRead; }`. `time_t` measures 4 bytes on the legacy 32-bit target, so the
/// record is 6 bytes, not 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcSkill {
    /// The legacy `bMasterType` byte.
    pub master_type: u8,
    /// The legacy `bLevel` byte.
    pub level: u8,
    /// The legacy `tNextRead` field, a signed 4-byte `time_t`.
    pub next_read: i32,
}

impl GcSkill {
    /// Append the fixed 6 packed bytes for one skill entry to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.master_type);
        out.push(self.level);
        out.extend_from_slice(&self.next_read.to_le_bytes());
    }

    /// Read one 6-byte skill entry from the front of `bytes`.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when `bytes` is shorter than
    /// [`GC_SKILL_WIRE_SIZE`]. A skill entry carries no header, so no header
    /// check is made.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_SKILL_WIRE_SIZE, "GcSkill")?;
        Ok(Self {
            master_type: raw[0],
            level: raw[1],
            next_read: i32_at(raw, 2),
        })
    }
}

/// The 21-byte nested body of [`GcAffectAdd`].
///
/// Legacy `TPacketAffectElement` is
/// `{ DWORD dwType; BYTE bPointIdxApplyOn; long lApplyValue; DWORD dwFlag;
/// long lDuration; long lSPCost; }`. The client calls the `BYTE` field
/// `bPointIdxApplyOn` and the server calls it `bApplyOn`; the wire is one byte
/// either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcAffectElement {
    /// The legacy `dwType` word.
    pub affect_type: u32,
    /// The legacy `bPointIdxApplyOn` byte, called `bApplyOn` by the server.
    pub apply_on: u8,
    /// The legacy `lApplyValue` field.
    pub apply_value: i32,
    /// The legacy `dwFlag` word.
    pub flag: u32,
    /// The legacy `lDuration` field.
    pub duration: i32,
    /// The legacy `lSPCost` field.
    pub sp_cost: i32,
}

impl GcAffectElement {
    /// Append the fixed 21 packed bytes for one affect element to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.affect_type.to_le_bytes());
        out.push(self.apply_on);
        out.extend_from_slice(&self.apply_value.to_le_bytes());
        out.extend_from_slice(&self.flag.to_le_bytes());
        out.extend_from_slice(&self.duration.to_le_bytes());
        out.extend_from_slice(&self.sp_cost.to_le_bytes());
    }

    /// Read one 21-byte affect element from the front of `bytes`.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when `bytes` is shorter than
    /// [`GC_AFFECT_ELEMENT_WIRE_SIZE`]. An affect element carries no header, so
    /// no header check is made.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_AFFECT_ELEMENT_WIRE_SIZE, "GcAffectElement")?;
        Ok(Self {
            affect_type: u32_at(raw, 0),
            apply_on: raw[4],
            apply_value: i32_at(raw, 5),
            flag: u32_at(raw, 9),
            duration: i32_at(raw, 13),
            sp_cost: i32_at(raw, 17),
        })
    }
}

/// `HEADER_GC_AFFECT_ADD` (126), 22 packed bytes.
///
/// A header and one nested [`GcAffectElement`]. The record does not apply the
/// affect; it only carries the bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcAffectAdd {
    /// The one-byte record header.
    pub header: u8,
    /// The single nested affect element.
    pub element: GcAffectElement,
}

impl GcAffectAdd {
    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_AFFECT_ADD
    }

    /// Append the 22 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        self.element.encode_into(out);
    }

    /// Read the 22 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_AFFECT_ADD_WIRE_SIZE`], and [`GcNestedError::Header`] when a
    /// complete-length buffer starts with any byte other than
    /// [`GcAffectAdd::header`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_AFFECT_ADD_WIRE_SIZE, "GcAffectAdd")?;
        check_header(bytes, "GcAffectAdd", Self::header())?;
        Ok(Self {
            header: raw[0],
            element: GcAffectElement::decode(&raw[1..])?,
        })
    }
}

/// `HEADER_GC_PLAYER_POINT_CHANGE` (17), 25 packed bytes.
///
/// The first field is a C++ **`int`**, not a `BYTE`, so the record is 25 bytes
/// and not 22. The server names the byte `HEADER_GC_CHARACTER_POINT_CHANGE`
/// (`server/server/game/packet.h:117`) and the struct `TPacketGCPointChange`
/// (`packet.h:1064-1071`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPointChange {
    /// The legacy `int header` field, a full 4-byte signed word.
    pub header: i32,
    /// The legacy `dwVID` word.
    pub vid: u32,
    /// The legacy `Type` byte, called `type` by the server.
    pub change_type: u8,
    /// The legacy `amount` field, a `long long`.
    pub amount: i64,
    /// The legacy `value` field, a `long long`.
    pub value: i64,
}

impl GcPointChange {
    /// Append the 25 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.header.to_le_bytes());
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.push(self.change_type);
        out.extend_from_slice(&self.amount.to_le_bytes());
        out.extend_from_slice(&self.value.to_le_bytes());
    }

    /// Read the 25 packed bytes.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_POINT_CHANGE_WIRE_SIZE`]. The header is a full signed word rather
    /// than a single byte, so there is no single-byte header check to make and
    /// no [`GcNestedError::Header`] variant can arise here.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_POINT_CHANGE_WIRE_SIZE, "GcPointChange")?;
        Ok(Self {
            header: i32_at(raw, 0),
            vid: u32_at(raw, 4),
            change_type: raw[8],
            amount: i64_at(raw, 9),
            value: i64_at(raw, 17),
        })
    }
}

/// `HEADER_GC_DIG_MOTION` (134), 10 packed bytes.
///
/// The body is a header, two `DWORD`s, and one `BYTE`. A hand sum of these five
/// values is 14, and the i686 probe measured 10, so this width comes from the
/// measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcDigMotion {
    /// The one-byte record header.
    pub header: u8,
    /// The legacy `vid` word for the character digging.
    pub vid: u32,
    /// The legacy `target_vid` word.
    pub target_vid: u32,
    /// The legacy `count` byte.
    pub count: u8,
}

impl GcDigMotion {
    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_DIG_MOTION
    }

    /// Append the 10 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.extend_from_slice(&self.target_vid.to_le_bytes());
        out.push(self.count);
    }

    /// Read the 10 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_DIG_MOTION_WIRE_SIZE`], and [`GcNestedError::Header`] when a
    /// complete-length buffer starts with any other byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_DIG_MOTION_WIRE_SIZE, "GcDigMotion")?;
        check_header(bytes, "GcDigMotion", Self::header())?;
        Ok(Self {
            header: raw[0],
            vid: u32_at(raw, 1),
            target_vid: u32_at(raw, 5),
            count: raw[9],
        })
    }
}

/// `HEADER_GC_DAMAGE_INFO` (135), 10 packed bytes.
///
/// The client writes `int  damage;` with two spaces, which is the same `int` the
/// server writes with one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcDamageInfo {
    /// The one-byte record header.
    pub header: u8,
    /// The legacy `dwVID` word.
    pub vid: u32,
    /// The legacy `flag` byte.
    pub flag: u8,
    /// The legacy `damage` field, a C++ `int`.
    pub damage: i32,
}

impl GcDamageInfo {
    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_DAMAGE_INFO
    }

    /// Append the 10 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.push(self.flag);
        out.extend_from_slice(&self.damage.to_le_bytes());
    }

    /// Read the 10 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_DAMAGE_INFO_WIRE_SIZE`], and [`GcNestedError::Header`] when a
    /// complete-length buffer starts with any other byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_DAMAGE_INFO_WIRE_SIZE, "GcDamageInfo")?;
        check_header(bytes, "GcDamageInfo", Self::header())?;
        Ok(Self {
            header: raw[0],
            vid: u32_at(raw, 1),
            flag: raw[5],
            damage: i32_at(raw, 6),
        })
    }
}

/// `HEADER_GC_PREMIUM_PLAYERS` (141), 28 packed bytes.
///
/// This is a C++ **class** with a user-supplied constructor, not a typedef
/// struct. The constructor only sets the header, so the wire layout is the four
/// members. The name field is a raw `char[CHARACTER_NAME_MAX_LEN + 1]`, so it
/// stays 25 bytes of storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPremiumPlayers {
    /// The one-byte record header.
    pub header: u8,
    /// The legacy `bySubHeader` byte.
    pub sub_header: u8,
    /// The legacy `byPos` byte.
    pub pos: u8,
    /// The raw `char[25]` name field. No text decoding, no NUL validation.
    pub name: [u8; 25],
}

impl GcPremiumPlayers {
    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_PREMIUM_PLAYERS
    }

    /// Append the 28 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.push(self.sub_header);
        out.push(self.pos);
        out.extend_from_slice(&self.name);
    }

    /// Read the 28 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_PREMIUM_PLAYERS_WIRE_SIZE`], and [`GcNestedError::Header`] when a
    /// complete-length buffer starts with any other byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_PREMIUM_PLAYERS_WIRE_SIZE, "GcPremiumPlayers")?;
        check_header(bytes, "GcPremiumPlayers", Self::header())?;
        let mut name = [0u8; 25];
        name.copy_from_slice(&raw[3..]);
        Ok(Self {
            header: raw[0],
            sub_header: raw[1],
            pos: raw[2],
            name,
        })
    }
}

/// The 17-byte record shared by bytes 69 and 71.
///
/// The client declares `long lX; long lY;` on two lines and the server declares
/// `long x, y;` on one. Both bodies are two 4-byte fields, so the probe
/// measured 17 from each. The header is the only thing separating
/// [`HEADER_GC_ADD_FLY_TARGETING`] from [`HEADER_GC_FLY_TARGETING`], so the
/// header is a field and the caller supplies the one it expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcFlyTargeting {
    /// The one-byte record header.
    pub header: u8,
    /// The legacy `dwShooterVID` word.
    pub shooter_vid: u32,
    /// The legacy `dwTargetVID` word.
    pub target_vid: u32,
    /// The legacy horizontal coordinate, a C++ `long`.
    pub x: i32,
    /// The legacy vertical coordinate, a C++ `long`.
    pub y: i32,
}

impl GcFlyTargeting {
    /// Build the record for either of the two headers.
    #[must_use]
    pub const fn new(header: u8, shooter_vid: u32, target_vid: u32, x: i32, y: i32) -> Self {
        Self {
            header,
            shooter_vid,
            target_vid,
            x,
            y,
        }
    }

    /// Append the 17 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.header);
        out.extend_from_slice(&self.shooter_vid.to_le_bytes());
        out.extend_from_slice(&self.target_vid.to_le_bytes());
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
    }

    /// Read the 17 packed bytes for whichever of the two headers is expected.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_FLY_TARGETING_WIRE_SIZE`]. The header is data for this shared type,
    /// so no header check is made and [`GcNestedError::Header`] cannot arise.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_FLY_TARGETING_WIRE_SIZE, "GcFlyTargeting")?;
        Ok(Self {
            header: raw[0],
            shooter_vid: u32_at(raw, 1),
            target_vid: u32_at(raw, 5),
            x: i32_at(raw, 9),
            y: i32_at(raw, 13),
        })
    }
}

/// `HEADER_GC_WARP` (65), 15 packed bytes.
///
/// Three C++ `long` fields and one `WORD`. The client spells the type `LONG` and
/// the server spells it `long`; both are 4 bytes here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcWarp {
    /// The one-byte record header.
    pub header: u8,
    /// The legacy `lX` field.
    pub x: i32,
    /// The legacy `lY` field.
    pub y: i32,
    /// The legacy `lAddr` field, a map or sector address.
    pub addr: i32,
    /// The legacy `wPort` word.
    pub port: u16,
}

impl GcWarp {
    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_WARP
    }

    /// Append the 15 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.extend_from_slice(&self.addr.to_le_bytes());
        out.extend_from_slice(&self.port.to_le_bytes());
    }

    /// Read the 15 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_WARP_WIRE_SIZE`], and [`GcNestedError::Header`] when a
    /// complete-length buffer starts with any other byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_WARP_WIRE_SIZE, "GcWarp")?;
        check_header(bytes, "GcWarp", Self::header())?;
        Ok(Self {
            header: raw[0],
            x: i32_at(raw, 1),
            y: i32_at(raw, 5),
            addr: i32_at(raw, 9),
            port: u16::from_le_bytes([raw[13], raw[14]]),
        })
    }
}

/// The live skill-level record, byte 76, 1531 packed bytes.
///
/// This is the client's `TPacketGCSkillLevelNew` and the server's
/// `TPacketGCSkillLevel`. Both declare `TPlayerSkill skills[255]`, so both
/// measure `1 + 255 * 6 = 1531`.
///
/// The client's byte-72 `TPacketGCSkillLevel` is a **different record** with a
/// `BYTE abSkillLevels[255]` body, 256 bytes wide. The server's byte-72
/// enumerator `HEADER_GC_SKILL_LEVEL_OLD` at `packet.h:161` is declared and
/// never sent, and `char_skill.cpp:184` sends byte 76. So the 256-byte shape is
/// dead on both sides and is not this record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcSkillLevelNew {
    /// The one-byte record header.
    pub header: u8,
    /// The 255 nested skill entries, in legacy slot order.
    pub skills: [GcSkill; GC_SKILL_SLOT_COUNT],
}

impl GcSkillLevelNew {
    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_SKILL_LEVEL_NEW
    }

    /// Append the 1531 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        for skill in &self.skills {
            skill.encode_into(out);
        }
    }

    /// Read the 1531 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_SKILL_LEVEL_NEW_WIRE_SIZE`], and [`GcNestedError::Header`] when a
    /// complete-length buffer starts with any other byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_SKILL_LEVEL_NEW_WIRE_SIZE, "GcSkillLevelNew")?;
        check_header(bytes, "GcSkillLevelNew", Self::header())?;
        let mut skills = [GcSkill {
            master_type: 0,
            level: 0,
            next_read: 0,
        }; GC_SKILL_SLOT_COUNT];
        for (index, slot) in skills.iter_mut().enumerate() {
            *slot = GcSkill::decode(&raw[1 + index * GC_SKILL_WIRE_SIZE..])?;
        }
        Ok(Self {
            header: raw[0],
            skills,
        })
    }
}

/// `HEADER_GC_DAILY_GIFT` (180), 111 packed bytes.
///
/// Five parallel arrays of `DAILY_GIFT_WEEK_DAYS` elements, which is 7. The
/// five arrays have different element widths, so the record is
/// `1 + 4 + 1 + 7 * (4 + 4 + 4 + 2 + 1) = 111`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcDailyGift {
    /// The one-byte record header.
    pub header: u8,
    /// The legacy `dwCash` word.
    pub cash: u32,
    /// The legacy `bWeek` byte.
    pub week: u8,
    /// The legacy `dwVnum[7]` array.
    pub vnum: [u32; GC_DAILY_GIFT_DAY_COUNT],
    /// The legacy `dwCount[7]` array.
    pub count: [u32; GC_DAILY_GIFT_DAY_COUNT],
    /// The legacy `dwCollectTime[7]` array.
    pub collect_time: [u32; GC_DAILY_GIFT_DAY_COUNT],
    /// The legacy `wCost[7]` array, whose elements are `WORD`, not `DWORD`.
    pub cost: [u16; GC_DAILY_GIFT_DAY_COUNT],
    /// The legacy `bStatus[7]` array.
    pub status: [u8; GC_DAILY_GIFT_DAY_COUNT],
}

impl GcDailyGift {
    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_DAILY_GIFT
    }

    /// Append the 111 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.cash.to_le_bytes());
        out.push(self.week);
        for value in &self.vnum {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in &self.count {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in &self.collect_time {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in &self.cost {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in &self.status {
            out.push(*value);
        }
    }

    /// Read the 111 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_DAILY_GIFT_WIRE_SIZE`], and [`GcNestedError::Header`] when a
    /// complete-length buffer starts with any other byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_DAILY_GIFT_WIRE_SIZE, "GcDailyGift")?;
        check_header(bytes, "GcDailyGift", Self::header())?;
        let mut vnum = [0u32; GC_DAILY_GIFT_DAY_COUNT];
        let mut count = [0u32; GC_DAILY_GIFT_DAY_COUNT];
        let mut collect_time = [0u32; GC_DAILY_GIFT_DAY_COUNT];
        let mut cost = [0u16; GC_DAILY_GIFT_DAY_COUNT];
        let mut status = [0u8; GC_DAILY_GIFT_DAY_COUNT];
        let n = GC_DAILY_GIFT_DAY_COUNT;
        for (index, slot) in vnum.iter_mut().enumerate() {
            *slot = u32_at(raw, 6 + index * 4);
        }
        for (index, slot) in count.iter_mut().enumerate() {
            *slot = u32_at(raw, 6 + n * 4 + index * 4);
        }
        for (index, slot) in collect_time.iter_mut().enumerate() {
            *slot = u32_at(raw, 6 + n * 8 + index * 4);
        }
        for (index, slot) in cost.iter_mut().enumerate() {
            *slot = u16::from_le_bytes([raw[6 + n * 12 + index * 2], raw[7 + n * 12 + index * 2]]);
        }
        for (index, slot) in status.iter_mut().enumerate() {
            *slot = raw[6 + n * 14 + index];
        }
        Ok(Self {
            header: raw[0],
            cash: u32_at(raw, 1),
            week: raw[5],
            vnum,
            count,
            collect_time,
            cost,
            status,
        })
    }
}

/// `HEADER_GC_PARTY_UPDATE` (79), 21 packed bytes.
///
/// The client declares `short affects[PARTY_AFFECT_SLOT_MAX_NUM]` and the server
/// declares `short affects[7]`. `PARTY_AFFECT_SLOT_MAX_NUM` is 7 and has no
/// server definition, so the two agree at 7 by coincidence rather than through
/// a shared constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPartyUpdate {
    /// The one-byte record header.
    pub header: u8,
    /// The legacy `pid` word, called `role` by the server.
    pub pid: u32,
    /// The legacy `state` byte, called `role` by the server.
    pub state: u8,
    /// The legacy `percent_hp` byte.
    pub percent_hp: u8,
    /// The legacy `affects` array of seven signed 16-bit values.
    pub affects: [i16; GC_PARTY_AFFECT_SLOT_COUNT],
}

impl GcPartyUpdate {
    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_PARTY_UPDATE
    }

    /// Append the 21 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.extend_from_slice(&self.pid.to_le_bytes());
        out.push(self.state);
        out.push(self.percent_hp);
        for value in &self.affects {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }

    /// Read the 21 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_PARTY_UPDATE_WIRE_SIZE`], and [`GcNestedError::Header`] when a
    /// complete-length buffer starts with any other byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_PARTY_UPDATE_WIRE_SIZE, "GcPartyUpdate")?;
        check_header(bytes, "GcPartyUpdate", Self::header())?;
        let mut affects = [0i16; GC_PARTY_AFFECT_SLOT_COUNT];
        for (index, slot) in affects.iter_mut().enumerate() {
            let at = 7 + index * 2;
            *slot = i16::from_le_bytes([raw[at], raw[at + 1]]);
        }
        Ok(Self {
            header: raw[0],
            pid: u32_at(raw, 1),
            state: raw[5],
            percent_hp: raw[6],
            affects,
        })
    }
}

/// The 169-byte nested body of [`GcCubeRenewal`].
///
/// Legacy `TInfoDateCubeRenewal`. The `item_reward_stackable` field is a C++
/// `bool`, which is one byte under `pack(1)`, so it is a raw `u8` here and the
/// module has no `bool`. The `category` field is a raw `char[100]`.
///
/// **The five material pairs are interleaved on the wire.** The legacy body
/// reads `vnum_material_1; count_material_1; vnum_material_2; count_material_2;
/// ...`, so the Rust `vnum_material` and `count_material` arrays are *not*
/// adjacent blocks of five elements. Material `i` sits at byte `17 + 8 * i` and
/// its count at `21 + 8 * i`, which is why this record is 169 bytes and not the
/// 159 a two-block reading would give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcCubeRenewalDate {
    /// The legacy `npc_vnum` word.
    pub npc_vnum: u32,
    /// The legacy `index` word.
    pub index: u32,
    /// The legacy `vnum_reward` word.
    pub vnum_reward: u32,
    /// The legacy `count_reward` field.
    pub count_reward: i32,
    /// The legacy `item_reward_stackable` `bool`, carried as a raw byte.
    pub item_reward_stackable: u8,
    /// The five material vnum words, in legacy order.
    pub vnum_material: [u32; 5],
    /// The five material count fields, in legacy order.
    pub count_material: [i32; 5],
    /// The legacy `gold` field, an `unsigned long long`.
    pub gold: u64,
    /// The legacy `percent` field.
    pub percent: i32,
    /// The raw `char[100]` category field.
    pub category: [u8; GC_CUBE_CATEGORY_SIZE],
}

impl GcCubeRenewalDate {
    /// Append the fixed 169 packed bytes for one cube renewal date record.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.npc_vnum.to_le_bytes());
        out.extend_from_slice(&self.index.to_le_bytes());
        out.extend_from_slice(&self.vnum_reward.to_le_bytes());
        out.extend_from_slice(&self.count_reward.to_le_bytes());
        out.push(self.item_reward_stackable);
        // Interleaved, matching the legacy `vnum_material_N; count_material_N;`
        // declaration order. Writing two adjacent blocks would give 169 bytes
        // of the right length but the wrong contents.
        for index in 0..5 {
            out.extend_from_slice(&self.vnum_material[index].to_le_bytes());
            out.extend_from_slice(&self.count_material[index].to_le_bytes());
        }
        out.extend_from_slice(&self.gold.to_le_bytes());
        out.extend_from_slice(&self.percent.to_le_bytes());
        out.extend_from_slice(&self.category);
    }

    /// Read one 169-byte date record from the front of `bytes`.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when `bytes` is shorter than
    /// [`GC_CUBE_RENEWAL_DATE_WIRE_SIZE`]. A date record carries no header, so
    /// no header check is made.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_CUBE_RENEWAL_DATE_WIRE_SIZE, "GcCubeRenewalDate")?;
        // The five material pairs are INTERLEAVED on the wire: the legacy body
        // is `vnum_material_1; count_material_1; vnum_material_2; ...`, not two
        // separate arrays. Each pair therefore sits 8 bytes after the previous
        // one, and `gold` follows all five pairs.
        let mut vnum_material = [0u32; 5];
        let mut count_material = [0i32; 5];
        for index in 0..5 {
            vnum_material[index] = u32_at(raw, 17 + index * 8);
            count_material[index] = i32_at(raw, 21 + index * 8);
        }
        let mut category = [0u8; GC_CUBE_CATEGORY_SIZE];
        category.copy_from_slice(&raw[69..]);
        Ok(Self {
            npc_vnum: u32_at(raw, 0),
            index: u32_at(raw, 4),
            vnum_reward: u32_at(raw, 8),
            count_reward: i32_at(raw, 12),
            item_reward_stackable: raw[16],
            vnum_material,
            count_material,
            gold: u64::from_le_bytes([
                raw[57], raw[58], raw[59], raw[60], raw[61], raw[62], raw[63], raw[64],
            ]),
            percent: i32_at(raw, 65),
            category,
        })
    }
}

/// `HEADER_GC_CUBE_RENEWAL` (221), 171 packed bytes.
///
/// A C++ class with a user-supplied constructor whose body only sets the header.
/// The wire layout is a header, a sub-header byte, and one nested
/// [`GcCubeRenewalDate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcCubeRenewal {
    /// The one-byte record header.
    pub header: u8,
    /// The legacy `subheader` byte.
    pub subheader: u8,
    /// The nested date record.
    pub date: GcCubeRenewalDate,
}

impl GcCubeRenewal {
    /// The fixed header this record always carries.
    pub const fn header() -> u8 {
        HEADER_GC_CUBE_RENEWAL
    }

    /// Append the 171 packed bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header());
        out.push(self.subheader);
        self.date.encode_into(out);
    }

    /// Read the 171 packed bytes, rejecting any other header.
    ///
    /// # Errors
    ///
    /// [`GcNestedError::Truncated`] when the buffer is shorter than
    /// [`GC_CUBE_RENEWAL_WIRE_SIZE`], and [`GcNestedError::Header`] when a
    /// complete-length buffer starts with any other byte.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcNestedError> {
        let raw = take(bytes, GC_CUBE_RENEWAL_WIRE_SIZE, "GcCubeRenewal")?;
        check_header(bytes, "GcCubeRenewal", Self::header())?;
        Ok(Self {
            header: raw[0],
            subheader: raw[1],
            date: GcCubeRenewalDate::decode(&raw[2..])?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A_U32: u32 = 0x0102_0304;
    const B_U32: u32 = 0x0a0b_0c0d;
    const A_I32: i32 = 0x0102_0305;
    const B_I32: i32 = -0x0a0b_0c0d;
    const C_I32: i32 = 0x1122_3344;
    const A_I64: i64 = 0x0102_0304_0506_0708;
    const B_I64: i64 = -0x0a0b_0c0d_0e0f_1011;
    const A_U64: u64 = 0x0102_0304_0506_0708;
    const A_U16: u16 = 0x1122;
    const B_U16: u16 = 0x3344;
    /// A `time_t` word that is not representable as a positive `i32`.
    const BIG_TIME_T: i32 = i32::MIN.wrapping_add(0x1122_3344);

    /// A deterministic byte ramp, so no test value can be byte-symmetric.
    fn ramp(seed: u8) -> impl Iterator<Item = u8> {
        let mut value = seed;
        core::iter::from_fn(move || {
            let current = value;
            value = value.wrapping_mul(7).wrapping_add(3);
            Some(current)
        })
    }

    /// Fill a fixed legacy array with an asymmetric pattern.
    fn pattern<const N: usize>(seed: u8) -> [u8; N] {
        let mut out = [0u8; N];
        let bytes: Vec<u8> = ramp(seed).take(N).collect();
        out.copy_from_slice(&bytes);
        out
    }

    /// A sample skill for `index`, built without any narrowing cast.
    fn sample_skill(index: usize) -> GcSkill {
        let n = u16::try_from(index % 512).unwrap_or(0);
        let low = u8::try_from(index % 256).unwrap_or(0);
        GcSkill {
            master_type: low.wrapping_mul(3).wrapping_add(1),
            level: low.wrapping_mul(5).wrapping_add(2),
            next_read: i32::from(n.wrapping_mul(7).wrapping_add(11)) - 3000,
        }
    }

    fn sample_skills() -> [GcSkill; GC_SKILL_SLOT_COUNT] {
        let mut out = [GcSkill {
            master_type: 0,
            level: 0,
            next_read: 0,
        }; GC_SKILL_SLOT_COUNT];
        for (index, slot) in out.iter_mut().enumerate() {
            *slot = sample_skill(index);
        }
        out
    }

    fn sample_affect() -> GcAffectElement {
        GcAffectElement {
            affect_type: A_U32,
            apply_on: 0x5a,
            apply_value: B_I32,
            flag: B_U32,
            duration: C_I32,
            sp_cost: A_I32,
        }
    }

    fn sample_date() -> GcCubeRenewalDate {
        GcCubeRenewalDate {
            npc_vnum: A_U32,
            index: B_U32,
            vnum_reward: C_I32 as u32,
            count_reward: B_I32,
            item_reward_stackable: 0x01,
            vnum_material: [A_U32, B_U32, 0x1111_1111, 0x2222_2222, 0x3333_3333],
            count_material: [A_I32, B_I32, C_I32, -1, -2],
            gold: A_U64,
            percent: B_I32,
            category: pattern(0x51),
        }
    }

    /// Encode a record, confirm its width, then decode it and run `body`.
    fn check(name: &'static str, wire: &[u8], expected: usize, body: fn(&[u8])) {
        assert_eq!(wire.len(), expected, "{name} encoded width");
        body(wire);
    }

    /// The two records that share a width of 10 bytes, kept apart by their headers.
    #[test]
    fn the_two_ten_byte_records_round_trip_to_every_field() {
        let mut w = Vec::new();
        GcDigMotion {
            header: GcDigMotion::header(),
            vid: A_U32,
            target_vid: B_U32,
            count: 0x3d,
        }
        .encode_into(&mut w);
        check("GcDigMotion", &w, GC_DIG_MOTION_WIRE_SIZE, |b| {
            let r = GcDigMotion::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_DIG_MOTION, "header");
            assert_eq!(r.vid, A_U32, "vid");
            assert_eq!(r.target_vid, B_U32, "target_vid");
            assert_eq!(r.count, 0x3d, "count");
        });

        let mut w = Vec::new();
        GcDamageInfo {
            header: GcDamageInfo::header(),
            vid: A_U32,
            flag: 0x2c,
            damage: B_I32,
        }
        .encode_into(&mut w);
        check("GcDamageInfo", &w, GC_DAMAGE_INFO_WIRE_SIZE, |b| {
            let r = GcDamageInfo::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_DAMAGE_INFO, "header");
            assert_eq!(r.vid, A_U32, "vid");
            assert_eq!(r.flag, 0x2c, "flag");
            assert_eq!(r.damage, B_I32, "damage");
        });
    }

    #[test]
    fn every_small_record_round_trips_to_every_field_it_was_built_with() {
        let mut w = Vec::new();
        GcAffectAdd {
            header: GcAffectAdd::header(),
            element: sample_affect(),
        }
        .encode_into(&mut w);
        check("GcAffectAdd", &w, GC_AFFECT_ADD_WIRE_SIZE, |b| {
            let r = GcAffectAdd::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_AFFECT_ADD, "header");
            assert_eq!(r.element, sample_affect(), "element");
        });

        let mut w = Vec::new();
        GcPointChange {
            header: GC_POINT_CHANGE_HEADER,
            vid: A_U32,
            change_type: 0x7b,
            amount: A_I64,
            value: B_I64,
        }
        .encode_into(&mut w);
        check("GcPointChange", &w, GC_POINT_CHANGE_WIRE_SIZE, |b| {
            let r = GcPointChange::decode(b).unwrap();
            assert_eq!(r.header, GC_POINT_CHANGE_HEADER, "header");
            assert_eq!(r.vid, A_U32, "vid");
            assert_eq!(r.change_type, 0x7b, "change_type");
            assert_eq!(r.amount, A_I64, "amount");
            assert_eq!(r.value, B_I64, "value");
        });

        let mut w = Vec::new();
        GcPremiumPlayers {
            header: GcPremiumPlayers::header(),
            sub_header: 0x11,
            pos: 0x22,
            name: pattern(0x31),
        }
        .encode_into(&mut w);
        check("GcPremiumPlayers", &w, GC_PREMIUM_PLAYERS_WIRE_SIZE, |b| {
            let r = GcPremiumPlayers::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_PREMIUM_PLAYERS, "header");
            assert_eq!(r.sub_header, 0x11, "sub_header");
            assert_eq!(r.pos, 0x22, "pos");
            assert_eq!(r.name, pattern::<25>(0x31), "name");
        });

        let mut w = Vec::new();
        GcFlyTargeting::new(HEADER_GC_FLY_TARGETING, A_U32, B_U32, A_I32, B_I32)
            .encode_into(&mut w);
        check("GcFlyTargeting", &w, GC_FLY_TARGETING_WIRE_SIZE, |b| {
            let r = GcFlyTargeting::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_FLY_TARGETING, "header");
            assert_eq!(r.shooter_vid, A_U32, "shooter_vid");
            assert_eq!(r.target_vid, B_U32, "target_vid");
            assert_eq!(r.x, A_I32, "x");
            assert_eq!(r.y, B_I32, "y");
        });

        let mut w = Vec::new();
        GcWarp {
            header: GcWarp::header(),
            x: A_I32,
            y: B_I32,
            addr: C_I32,
            port: A_U16,
        }
        .encode_into(&mut w);
        check("GcWarp", &w, GC_WARP_WIRE_SIZE, |b| {
            let r = GcWarp::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_WARP, "header");
            assert_eq!(r.x, A_I32, "x");
            assert_eq!(r.y, B_I32, "y");
            assert_eq!(r.addr, C_I32, "addr");
            assert_eq!(r.port, A_U16, "port");
        });
    }

    #[test]
    fn every_array_record_round_trips_to_every_field_it_was_built_with() {
        let mut w = Vec::new();
        GcSkillLevelNew {
            header: GcSkillLevelNew::header(),
            skills: sample_skills(),
        }
        .encode_into(&mut w);
        check("GcSkillLevelNew", &w, GC_SKILL_LEVEL_NEW_WIRE_SIZE, |b| {
            let r = GcSkillLevelNew::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_SKILL_LEVEL_NEW, "header");
            assert_eq!(r.skills, sample_skills(), "skills");
        });

        let mut w = Vec::new();
        GcDailyGift {
            header: GcDailyGift::header(),
            cash: A_U32,
            week: 0x04,
            vnum: [A_U32, B_U32, 1, 2, 3, 4, 5],
            count: [B_U32, A_U32, 6, 7, 8, 9, 10],
            collect_time: [0xdead_beef, 0x0bad_f00d, 11, 12, 13, 14, 15],
            cost: [A_U16, B_U16, 1, 2, 3, 4, 5],
            status: [0, 1, 2, 3, 4, 5, 6],
        }
        .encode_into(&mut w);
        check("GcDailyGift", &w, GC_DAILY_GIFT_WIRE_SIZE, |b| {
            let r = GcDailyGift::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_DAILY_GIFT, "header");
            assert_eq!(r.cash, A_U32, "cash");
            assert_eq!(r.week, 0x04, "week");
            assert_eq!(r.vnum, [A_U32, B_U32, 1, 2, 3, 4, 5], "vnum");
            assert_eq!(r.count, [B_U32, A_U32, 6, 7, 8, 9, 10], "count");
            assert_eq!(
                r.collect_time,
                [0xdead_beef, 0x0bad_f00d, 11, 12, 13, 14, 15],
                "collect_time"
            );
            assert_eq!(r.cost, [A_U16, B_U16, 1, 2, 3, 4, 5], "cost");
            assert_eq!(r.status, [0, 1, 2, 3, 4, 5, 6], "status");
        });

        let mut w = Vec::new();
        GcPartyUpdate {
            header: GcPartyUpdate::header(),
            pid: A_U32,
            state: 0x09,
            percent_hp: 0x64,
            affects: [1, -1, 300, -300, 32767, -32768, 7],
        }
        .encode_into(&mut w);
        check("GcPartyUpdate", &w, GC_PARTY_UPDATE_WIRE_SIZE, |b| {
            let r = GcPartyUpdate::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_PARTY_UPDATE, "header");
            assert_eq!(r.pid, A_U32, "pid");
            assert_eq!(r.state, 0x09, "state");
            assert_eq!(r.percent_hp, 0x64, "percent_hp");
            assert_eq!(r.affects, [1, -1, 300, -300, 32767, -32768, 7], "affects");
        });

        let mut w = Vec::new();
        GcCubeRenewal {
            header: GcCubeRenewal::header(),
            subheader: 0x06,
            date: sample_date(),
        }
        .encode_into(&mut w);
        check("GcCubeRenewal", &w, GC_CUBE_RENEWAL_WIRE_SIZE, |b| {
            let r = GcCubeRenewal::decode(b).unwrap();
            assert_eq!(r.header, HEADER_GC_CUBE_RENEWAL, "header");
            assert_eq!(r.subheader, 0x06, "subheader");
            assert_eq!(r.date, sample_date(), "date");
        });
    }

    #[test]
    fn the_nested_bodies_round_trip_on_their_own() {
        let mut w = Vec::new();
        sample_skill(42).encode_into(&mut w);
        assert_eq!(w.len(), GC_SKILL_WIRE_SIZE, "skill width");
        assert_eq!(GcSkill::decode(&w).unwrap(), sample_skill(42), "skill");

        let mut w = Vec::new();
        sample_affect().encode_into(&mut w);
        assert_eq!(w.len(), GC_AFFECT_ELEMENT_WIRE_SIZE, "affect width");
        assert_eq!(
            GcAffectElement::decode(&w).unwrap(),
            sample_affect(),
            "affect element"
        );

        let mut w = Vec::new();
        sample_date().encode_into(&mut w);
        assert_eq!(w.len(), GC_CUBE_RENEWAL_DATE_WIRE_SIZE, "date width");
        assert_eq!(
            GcCubeRenewalDate::decode(&w).unwrap(),
            sample_date(),
            "cube renewal date"
        );
    }

    /// A valid frame for each single-header record, for the negative tests.
    fn frame_for(name: &str) -> Vec<u8> {
        let mut w = Vec::new();
        match name {
            "GcAffectAdd" => GcAffectAdd {
                header: GcAffectAdd::header(),
                element: sample_affect(),
            }
            .encode_into(&mut w),
            "GcDigMotion" => GcDigMotion {
                header: GcDigMotion::header(),
                vid: A_U32,
                target_vid: B_U32,
                count: 1,
            }
            .encode_into(&mut w),
            "GcDamageInfo" => GcDamageInfo {
                header: GcDamageInfo::header(),
                vid: A_U32,
                flag: 1,
                damage: 2,
            }
            .encode_into(&mut w),
            "GcPremiumPlayers" => GcPremiumPlayers {
                header: GcPremiumPlayers::header(),
                sub_header: 1,
                pos: 2,
                name: [0; 25],
            }
            .encode_into(&mut w),
            "GcWarp" => GcWarp {
                header: GcWarp::header(),
                x: 1,
                y: 2,
                addr: 3,
                port: 4,
            }
            .encode_into(&mut w),
            "GcSkillLevelNew" => GcSkillLevelNew {
                header: GcSkillLevelNew::header(),
                skills: sample_skills(),
            }
            .encode_into(&mut w),
            "GcDailyGift" => GcDailyGift {
                header: GcDailyGift::header(),
                cash: 1,
                week: 2,
                vnum: [0; 7],
                count: [0; 7],
                collect_time: [0; 7],
                cost: [0; 7],
                status: [0; 7],
            }
            .encode_into(&mut w),
            "GcPartyUpdate" => GcPartyUpdate {
                header: GcPartyUpdate::header(),
                pid: 1,
                state: 2,
                percent_hp: 3,
                affects: [0; 7],
            }
            .encode_into(&mut w),
            "GcCubeRenewal" => GcCubeRenewal {
                header: GcCubeRenewal::header(),
                subheader: 1,
                date: sample_date(),
            }
            .encode_into(&mut w),
            other => panic!("no frame for {other}"),
        }
        w
    }

    /// The measured widths, spelled out so a changed constant fails here too.
    #[test]
    fn the_measured_widths_are_the_ones_this_module_publishes() {
        assert_eq!(GC_SKILL_WIRE_SIZE, 6, "TPlayerSkill with 4-byte time_t");
        assert_eq!(GC_AFFECT_ELEMENT_WIRE_SIZE, 21, "TPacketAffectElement");
        assert_eq!(GC_AFFECT_ADD_WIRE_SIZE, 22, "header plus one element");
        assert_eq!(GC_POINT_CHANGE_WIRE_SIZE, 25, "int header, not BYTE");
        assert_eq!(GC_DIG_MOTION_WIRE_SIZE, 10, "the probe measured 10, not 14");
        assert_eq!(GC_DAMAGE_INFO_WIRE_SIZE, 10, "header, DWORD, BYTE, int");
        assert_eq!(GC_PREMIUM_PLAYERS_WIRE_SIZE, 28, "three bytes and a name");
        assert_eq!(GC_FLY_TARGETING_WIRE_SIZE, 17, "two longs, not one line");
        assert_eq!(GC_WARP_WIRE_SIZE, 15, "three longs and a WORD");
        assert_eq!(GC_SKILL_SLOT_COUNT, 255, "SKILL_MAX_NUM");
        assert_eq!(
            GC_SKILL_LEVEL_NEW_WIRE_SIZE, 1531,
            "1 + 255 * 6 on the live byte 76"
        );
        assert_eq!(GC_SKILL_LEVEL_OLD_WIRE_SIZE, 256, "the dead byte-72 shape");
        assert_eq!(GC_DAILY_GIFT_DAY_COUNT, 7, "DAILY_GIFT_WEEK_DAYS");
        assert_eq!(GC_DAILY_GIFT_WIRE_SIZE, 111, "five parallel arrays");
        assert_eq!(GC_PARTY_AFFECT_SLOT_COUNT, 7, "PARTY_AFFECT_SLOT_MAX_NUM");
        assert_eq!(GC_PARTY_UPDATE_WIRE_SIZE, 21, "seven shorts");
        assert_eq!(GC_CUBE_CATEGORY_SIZE, 100, "raw char[100]");
        assert_eq!(GC_CUBE_RENEWAL_DATE_WIRE_SIZE, 169, "TInfoDateCubeRenewal");
        assert_eq!(GC_CUBE_RENEWAL_WIRE_SIZE, 171, "two bytes plus the date");
    }

    /// Every published width must be the length of a real frame.
    #[test]
    fn every_published_width_matches_a_real_frame() {
        for name in [
            "GcAffectAdd",
            "GcDigMotion",
            "GcDamageInfo",
            "GcPremiumPlayers",
            "GcWarp",
            "GcSkillLevelNew",
            "GcDailyGift",
            "GcPartyUpdate",
            "GcCubeRenewal",
        ] {
            let frame = frame_for(name);
            let expected = match name {
                "GcAffectAdd" => GC_AFFECT_ADD_WIRE_SIZE,
                "GcDigMotion" => GC_DIG_MOTION_WIRE_SIZE,
                "GcDamageInfo" => GC_DAMAGE_INFO_WIRE_SIZE,
                "GcPremiumPlayers" => GC_PREMIUM_PLAYERS_WIRE_SIZE,
                "GcWarp" => GC_WARP_WIRE_SIZE,
                "GcSkillLevelNew" => GC_SKILL_LEVEL_NEW_WIRE_SIZE,
                "GcDailyGift" => GC_DAILY_GIFT_WIRE_SIZE,
                "GcPartyUpdate" => GC_PARTY_UPDATE_WIRE_SIZE,
                _ => GC_CUBE_RENEWAL_WIRE_SIZE,
            };
            assert_eq!(frame.len(), expected, "{name} frame length");
        }
        let mut w = Vec::new();
        GcPointChange {
            header: GC_POINT_CHANGE_HEADER,
            vid: 1,
            change_type: 2,
            amount: 3,
            value: 4,
        }
        .encode_into(&mut w);
        assert_eq!(w.len(), GC_POINT_CHANGE_WIRE_SIZE, "GcPointChange");
        let mut w = Vec::new();
        GcFlyTargeting::new(HEADER_GC_FLY_TARGETING, 1, 2, 3, 4).encode_into(&mut w);
        assert_eq!(w.len(), GC_FLY_TARGETING_WIRE_SIZE, "GcFlyTargeting");
    }

    /// A short buffer must be rejected, never padded or partially read.
    #[test]
    fn every_record_rejects_every_truncation() {
        for name in [
            "GcAffectAdd",
            "GcDigMotion",
            "GcDamageInfo",
            "GcPremiumPlayers",
            "GcWarp",
            "GcSkillLevelNew",
            "GcDailyGift",
            "GcPartyUpdate",
            "GcCubeRenewal",
        ] {
            let frame = frame_for(name);
            for cut in [0, 1, 2, frame.len() / 2, frame.len() - 1] {
                let short = &frame[..cut];
                let outcome = match name {
                    "GcAffectAdd" => GcAffectAdd::decode(short).err(),
                    "GcDigMotion" => GcDigMotion::decode(short).err(),
                    "GcDamageInfo" => GcDamageInfo::decode(short).err(),
                    "GcPremiumPlayers" => GcPremiumPlayers::decode(short).err(),
                    "GcWarp" => GcWarp::decode(short).err(),
                    "GcSkillLevelNew" => GcSkillLevelNew::decode(short).err(),
                    "GcDailyGift" => GcDailyGift::decode(short).err(),
                    "GcPartyUpdate" => GcPartyUpdate::decode(short).err(),
                    _ => GcCubeRenewal::decode(short).err(),
                };
                match outcome {
                    Some(GcNestedError::Truncated {
                        context,
                        needed,
                        actual,
                    }) => {
                        assert_eq!(context, name, "{name} context");
                        assert_eq!(needed, frame.len(), "{name} needed at cut {cut}");
                        assert_eq!(actual, cut, "{name} actual at cut {cut}");
                    }
                    other => panic!("{name} at cut {cut} gave {other:?}"),
                }
            }
        }
    }

    /// A complete-length frame with the wrong header byte must be rejected.
    #[test]
    fn every_single_header_record_rejects_a_foreign_header() {
        for (name, frame) in [
            ("GcAffectAdd", frame_for("GcAffectAdd")),
            ("GcDigMotion", frame_for("GcDigMotion")),
            ("GcDamageInfo", frame_for("GcDamageInfo")),
            ("GcPremiumPlayers", frame_for("GcPremiumPlayers")),
            ("GcWarp", frame_for("GcWarp")),
            ("GcSkillLevelNew", frame_for("GcSkillLevelNew")),
            ("GcDailyGift", frame_for("GcDailyGift")),
            ("GcPartyUpdate", frame_for("GcPartyUpdate")),
            ("GcCubeRenewal", frame_for("GcCubeRenewal")),
        ] {
            let mut wrong = frame.clone();
            wrong[0] = wrong[0].wrapping_add(1);
            let outcome = match name {
                "GcAffectAdd" => GcAffectAdd::decode(&wrong).err(),
                "GcDigMotion" => GcDigMotion::decode(&wrong).err(),
                "GcDamageInfo" => GcDamageInfo::decode(&wrong).err(),
                "GcPremiumPlayers" => GcPremiumPlayers::decode(&wrong).err(),
                "GcWarp" => GcWarp::decode(&wrong).err(),
                "GcSkillLevelNew" => GcSkillLevelNew::decode(&wrong).err(),
                "GcDailyGift" => GcDailyGift::decode(&wrong).err(),
                "GcPartyUpdate" => GcPartyUpdate::decode(&wrong).err(),
                _ => GcCubeRenewal::decode(&wrong).err(),
            };
            match outcome {
                Some(GcNestedError::Header {
                    context,
                    expected,
                    actual,
                }) => {
                    assert_eq!(context, name, "{name} context");
                    assert_eq!(expected, wrong[0].wrapping_sub(1), "{name} expected");
                    assert_eq!(actual, wrong[0], "{name} actual");
                }
                other => panic!("{name} accepted a foreign header: {other:?}"),
            }
        }
    }

    /// The header byte is the only thing separating bytes 69 and 71.
    #[test]
    fn the_header_byte_is_the_only_thing_separating_the_two_fly_records() {
        assert_eq!(GC_FLY_TARGETING_HEADERS.len(), 2, "two headers");
        assert_ne!(
            GC_FLY_TARGETING_HEADERS[0], GC_FLY_TARGETING_HEADERS[1],
            "the two fly headers must differ"
        );
        let mut a = Vec::new();
        GcFlyTargeting::new(GC_FLY_TARGETING_HEADERS[0], A_U32, B_U32, A_I32, B_I32)
            .encode_into(&mut a);
        let mut b = Vec::new();
        GcFlyTargeting::new(GC_FLY_TARGETING_HEADERS[1], A_U32, B_U32, A_I32, B_I32)
            .encode_into(&mut b);
        assert_eq!(a.len(), b.len(), "the two frames have the same length");
        assert_ne!(a, b, "the two frames differ");
        assert_eq!(
            a[1..],
            b[1..],
            "everything after the header byte must be identical"
        );
        assert_eq!(GcFlyTargeting::decode(&a).unwrap().header, a[0], "header a");
        assert_eq!(GcFlyTargeting::decode(&b).unwrap().header, b[0], "header b");
    }

    /// Byte 76 is the live record and byte 72 is dead on both sides.
    #[test]
    fn the_skill_level_generations_keep_their_own_bodies() {
        assert_eq!(HEADER_GC_SKILL_LEVEL_NEW, 76, "the live record");
        assert_eq!(GcSkillLevelNew::header(), 76, "codec keyed to byte 76");
        // The two generations are not the same width and must never be confused.
        assert_eq!(GC_SKILL_LEVEL_NEW_WIRE_SIZE, 1531, "live width");
        assert_eq!(GC_SKILL_LEVEL_OLD_WIRE_SIZE, 256, "dead width");
        assert_ne!(
            GC_SKILL_LEVEL_NEW_WIRE_SIZE, GC_SKILL_LEVEL_OLD_WIRE_SIZE,
            "the generations have different widths"
        );
        // A 256-byte frame is a valid live record only if every slot decodes,
        // and it must not decode a byte-72 frame as live data.
        let mut w = Vec::new();
        GcSkillLevelNew {
            header: GcSkillLevelNew::header(),
            skills: sample_skills(),
        }
        .encode_into(&mut w);
        let rounded = GcSkillLevelNew::decode(&w[..GC_SKILL_LEVEL_OLD_WIRE_SIZE]);
        assert!(
            rounded.is_err(),
            "a 256-byte prefix is too short for the live record"
        );
    }

    /// `time_t` is 4 bytes here, so a skill entry is 6 and not 10.
    #[test]
    fn a_skill_entry_is_six_bytes_because_time_t_is_four() {
        let mut w = Vec::new();
        GcSkill {
            master_type: 0xab,
            level: 0xcd,
            next_read: BIG_TIME_T,
        }
        .encode_into(&mut w);
        assert_eq!(w.len(), 6, "two BYTEs and a 4-byte time_t");
        assert_eq!(w[..2], [0xab, 0xcd], "the two leading bytes");
        assert_eq!(&w[2..], &BIG_TIME_T.to_le_bytes(), "the time_t word");
        // A negative time_t must stay negative rather than wrap to a large u32.
        let mut w = Vec::new();
        GcSkill {
            master_type: 0,
            level: 0,
            next_read: -1,
        }
        .encode_into(&mut w);
        assert_eq!(GcSkill::decode(&w).unwrap().next_read, -1, "signed time_t");
    }

    /// The two `i32`s of the point-change record must not be able to swap.
    #[test]
    fn no_two_adjacent_fields_of_the_point_change_record_can_swap() {
        let mut a = Vec::new();
        GcPointChange {
            header: GC_POINT_CHANGE_HEADER,
            vid: 1,
            change_type: 2,
            amount: 3,
            value: 4,
        }
        .encode_into(&mut a);
        let mut b = Vec::new();
        GcPointChange {
            header: GC_POINT_CHANGE_HEADER,
            vid: 1,
            change_type: 2,
            amount: 4,
            value: 3,
        }
        .encode_into(&mut b);
        assert_ne!(a, b, "the two amount/value pairs differ");
        assert_eq!(GcPointChange::decode(&a).unwrap().amount, 3, "amount a");
        assert_eq!(GcPointChange::decode(&b).unwrap().amount, 4, "amount b");
        assert_eq!(GcPointChange::decode(&a).unwrap().value, 4, "value a");
        assert_eq!(GcPointChange::decode(&b).unwrap().value, 3, "value b");
    }

    /// The five parallel daily-gift arrays must stay in their own slots.
    #[test]
    fn the_daily_gift_arrays_cannot_be_read_from_each_others_slots() {
        let mut filled = [0u32; 7];
        for (index, slot) in filled.iter_mut().enumerate() {
            *slot = 0x1000_0000u32.wrapping_add(u32::try_from(index).unwrap_or(0));
        }
        let mut w = Vec::new();
        GcDailyGift {
            header: GcDailyGift::header(),
            cash: 1,
            week: 2,
            vnum: filled,
            count: [0; 7],
            collect_time: [0; 7],
            cost: [0; 7],
            status: [0; 7],
        }
        .encode_into(&mut w);
        let r = GcDailyGift::decode(&w).unwrap();
        assert_eq!(r.vnum, filled, "vnum keeps its own slots");
        assert_eq!(r.count, [0; 7], "count is a separate array");
        assert_eq!(r.collect_time, [0; 7], "collect_time is a separate array");
        assert_eq!(r.cost, [0; 7], "cost is a separate array");
        assert_eq!(r.status, [0; 7], "status is a separate array");
    }

    /// A multi-byte field must not be readable as its own byte reversal.
    #[test]
    fn no_multi_byte_field_survives_a_byte_reversal() {
        assert_ne!(A_U32.to_le_bytes(), A_U32.to_be_bytes(), "A_U32");
        assert_ne!(B_U32.to_le_bytes(), B_U32.to_be_bytes(), "B_U32");
        assert_ne!(A_I32.to_le_bytes(), A_I32.to_be_bytes(), "A_I32");
        assert_ne!(A_I64.to_le_bytes(), A_I64.to_be_bytes(), "A_I64");
        assert_ne!(A_U16.to_le_bytes(), A_U16.to_be_bytes(), "A_U16");

        for name in [
            "GcDigMotion",
            "GcDamageInfo",
            "GcWarp",
            "GcPremiumPlayers",
            "GcPartyUpdate",
            "GcDailyGift",
        ] {
            let frame = frame_for(name);
            let reversed: Vec<u8> = frame.iter().rev().copied().collect();
            let differs = match name {
                "GcDigMotion" => GcDigMotion::decode(&reversed)
                    .map(|r| r.vid != A_U32)
                    .unwrap_or(true),
                "GcDamageInfo" => GcDamageInfo::decode(&reversed)
                    .map(|r| r.vid != A_U32)
                    .unwrap_or(true),
                "GcWarp" => GcWarp::decode(&reversed).map(|r| r.x != 1).unwrap_or(true),
                "GcPremiumPlayers" => GcPremiumPlayers::decode(&reversed)
                    .map(|r| r.name != [0; 25])
                    .unwrap_or(true),
                "GcPartyUpdate" => GcPartyUpdate::decode(&reversed)
                    .map(|r| r.pid != 1)
                    .unwrap_or(true),
                _ => GcDailyGift::decode(&reversed)
                    .map(|r| r.cash != 1)
                    .unwrap_or(true),
            };
            assert!(differs, "{name} survived a whole-frame byte reversal");
        }
    }

    /// The nested affect element must not be readable at the wrong offset.
    #[test]
    fn the_nested_affect_element_must_not_be_read_at_a_shifted_offset() {
        let mut frame = frame_for("GcAffectAdd");
        // Two spare bytes so a shifted read is a value error, not a length error.
        frame.extend_from_slice(&[0xaa, 0xbb]);
        // The nested body starts at byte 1. A one-byte shift must not produce
        // the same element, because the first two bytes differ in value.
        let shifted = GcAffectElement::decode(&frame[2..]).unwrap();
        assert_ne!(
            shifted,
            sample_affect(),
            "a one-byte shift must not reproduce the element"
        );
        assert_eq!(
            GcAffectElement::decode(&frame[1..]).unwrap(),
            sample_affect(),
            "the element starts at byte 1"
        );
    }

    /// The cube-renewal date must not be read at a shifted offset.
    #[test]
    fn the_nested_cube_date_must_not_be_read_at_a_shifted_offset() {
        let mut frame = frame_for("GcCubeRenewal");
        frame.extend_from_slice(&[0xaa, 0xbb]);
        assert_eq!(
            GcCubeRenewalDate::decode(&frame[2..]).unwrap(),
            sample_date(),
            "the date starts at byte 2"
        );
        assert_ne!(
            GcCubeRenewalDate::decode(&frame[3..]).unwrap(),
            sample_date(),
            "a one-byte shift must not reproduce the date"
        );
    }

    /// The error type must name the record and the numbers.
    #[test]
    fn the_error_display_names_the_record_and_the_numbers() {
        let truncated = GcNestedError::Truncated {
            context: "GcWarp",
            needed: 15,
            actual: 3,
        };
        assert_eq!(
            truncated.to_string(),
            "GcWarp: need 15 bytes, buffer held 3"
        );
        let wrong_header = GcNestedError::Header {
            context: "GcDamageInfo",
            expected: 0x87,
            actual: 0x88,
        };
        assert_eq!(
            wrong_header.to_string(),
            "GcDamageInfo: header 0x88, expected 0x87"
        );
        let as_error: &dyn std::error::Error = &truncated;
        assert!(as_error.to_string().contains("GcWarp"), "Error impl");
    }

    /// Every `u8` and `i8` field must keep all 256 values.
    #[test]
    fn the_single_byte_fields_keep_every_one_of_the_256_values() {
        for value in 0u8..=255 {
            let mut w = Vec::new();
            GcDigMotion {
                header: GcDigMotion::header(),
                vid: 0,
                target_vid: 0,
                count: value,
            }
            .encode_into(&mut w);
            assert_eq!(
                GcDigMotion::decode(&w).unwrap().count,
                value,
                "count {value}"
            );

            let mut w = Vec::new();
            GcDamageInfo {
                header: GcDamageInfo::header(),
                vid: 0,
                flag: value,
                damage: 0,
            }
            .encode_into(&mut w);
            assert_eq!(
                GcDamageInfo::decode(&w).unwrap().flag,
                value,
                "flag {value}"
            );

            let mut w = Vec::new();
            GcAffectAdd {
                header: GcAffectAdd::header(),
                element: GcAffectElement {
                    affect_type: 0,
                    apply_on: value,
                    apply_value: 0,
                    flag: 0,
                    duration: 0,
                    sp_cost: 0,
                },
            }
            .encode_into(&mut w);
            assert_eq!(
                GcAffectAdd::decode(&w).unwrap().element.apply_on,
                value,
                "apply_on {value}"
            );
        }
    }

    /// The `bool` in the cube-renewal date is one byte and stays raw.
    #[test]
    fn the_cube_renewal_stackable_flag_is_one_raw_byte() {
        for value in 0u8..=255 {
            let mut date = sample_date();
            date.item_reward_stackable = value;
            let mut w = Vec::new();
            date.encode_into(&mut w);
            assert_eq!(w.len(), GC_CUBE_RENEWAL_DATE_WIRE_SIZE, "width is stable");
            assert_eq!(
                GcCubeRenewalDate::decode(&w).unwrap().item_reward_stackable,
                value,
                "flag {value}"
            );
        }
    }

    /// The `i16` party-affect slots must keep both extremes.
    #[test]
    fn the_party_affect_slots_keep_both_signed_extremes() {
        let affects = [i16::MIN, -1, 0, 1, i16::MAX, 7, -7];
        let mut w = Vec::new();
        GcPartyUpdate {
            header: GcPartyUpdate::header(),
            pid: 0,
            state: 0,
            percent_hp: 0,
            affects,
        }
        .encode_into(&mut w);
        let r = GcPartyUpdate::decode(&w).unwrap();
        for (index, want) in affects.iter().enumerate() {
            assert_eq!(r.affects[index], *want, "affects[{index}]");
        }
    }

    /// A golden byte test for the interleaved cube-renewal material pairs.
    ///
    /// The legacy body declares `vnum_material_N; count_material_N;` in pairs,
    /// not as two adjacent blocks. A two-block encoding is the same 169 bytes
    /// long and passes every width check, so only explicit bytes catch it.
    #[test]
    fn the_cube_material_pairs_are_interleaved_on_the_wire() {
        let mut date = GcCubeRenewalDate {
            npc_vnum: 0x0102_0304,
            index: 0x0a0b_0c0d,
            vnum_reward: 0x1122_3344,
            count_reward: 0x5566_7788,
            item_reward_stackable: 0x99,
            vnum_material: [0x11, 0x22, 0x33, 0x44, 0x55],
            count_material: [-1, -2, -3, -4, -5],
            gold: 0x0102_0304_0506_0708,
            percent: 0x090a_0b0c,
            category: [0; GC_CUBE_CATEGORY_SIZE],
        };
        let mut w = Vec::new();
        date.encode_into(&mut w);
        assert_eq!(w.len(), 169, "the date is 169 bytes");

        // The first four fields occupy bytes 0..17.
        assert_eq!(&w[0..4], &0x0102_0304u32.to_le_bytes(), "npc_vnum");
        assert_eq!(&w[4..8], &0x0a0b_0c0du32.to_le_bytes(), "index");
        assert_eq!(&w[8..12], &0x1122_3344u32.to_le_bytes(), "vnum_reward");
        assert_eq!(&w[12..16], &0x5566_7788i32.to_le_bytes(), "count_reward");
        assert_eq!(w[16], 0x99, "item_reward_stackable");

        // Then five interleaved 8-byte pairs starting at byte 17.
        let want: [(u32, i32); 5] = [(0x11, -1), (0x22, -2), (0x33, -3), (0x44, -4), (0x55, -5)];
        for (index, (vnum, count)) in want.iter().enumerate() {
            let at = 17 + index * 8;
            assert_eq!(
                &w[at..at + 4],
                &vnum.to_le_bytes(),
                "vnum_material[{index}] at byte {at}"
            );
            assert_eq!(
                &w[at + 4..at + 8],
                &count.to_le_bytes(),
                "count_material[{index}] at byte {}",
                at + 4
            );
        }

        // Then gold, percent, and the 100-byte category.
        assert_eq!(
            &w[57..65],
            &0x0102_0304_0506_0708u64.to_le_bytes(),
            "gold at byte 57"
        );
        assert_eq!(
            &w[65..69],
            &0x090a_0b0ci32.to_le_bytes(),
            "percent at byte 65"
        );
        assert_eq!(&w[69..], &[0u8; 100], "category at byte 69");

        // A two-block encoding would put 0x22 at byte 21, so pin that it is not.
        assert_eq!(
            w[21..25],
            (-1i32).to_le_bytes(),
            "byte 21 holds count_material[0], not vnum_material[1]"
        );
        assert_eq!(GcCubeRenewalDate::decode(&w).unwrap(), date, "round trip");

        // Changing only the counts must not disturb the vnums.
        date.count_material = [10, 20, 30, 40, 50];
        let mut w2 = Vec::new();
        date.encode_into(&mut w2);
        assert_eq!(w2.len(), 169, "the width does not depend on the values");
        assert_eq!(&w2[17..21], &w[17..21], "vnum_material[0] is unchanged");
        let r = GcCubeRenewalDate::decode(&w2).unwrap();
        assert_eq!(r.vnum_material, [0x11, 0x22, 0x33, 0x44, 0x55], "vnums");
        assert_eq!(r.count_material, [10, 20, 30, 40, 50], "counts");
    }

    /// A signed field must be read by reinterpreting its bytes, not by
    /// converting them. A `u32`-then-`as i32` read is bit-identical to a
    /// direct `i32` read, so it is a provable equivalent mutant; these
    /// boundaries are here so that any *converting* read still fails.
    #[test]
    fn the_signed_fields_keep_their_full_32_bit_range() {
        for value in [i32::MIN, -65_537, -256, -1, 0, 1, 255, 65_536, i32::MAX] {
            let mut w = Vec::new();
            GcAffectElement {
                affect_type: 0,
                apply_on: 0,
                apply_value: 0,
                flag: 0,
                duration: value,
                sp_cost: value,
            }
            .encode_into(&mut w);
            let r = GcAffectElement::decode(&w).unwrap();
            assert_eq!(r.duration, value, "affect duration {value}");
            assert_eq!(r.sp_cost, value, "affect sp_cost {value}");

            let mut w = Vec::new();
            GcDamageInfo {
                header: GcDamageInfo::header(),
                vid: 0,
                flag: 0,
                damage: value,
            }
            .encode_into(&mut w);
            assert_eq!(
                GcDamageInfo::decode(&w).unwrap().damage,
                value,
                "damage {value}"
            );

            let mut w = Vec::new();
            GcWarp {
                header: GcWarp::header(),
                x: value,
                y: 0,
                addr: 0,
                port: 0,
            }
            .encode_into(&mut w);
            assert_eq!(GcWarp::decode(&w).unwrap().x, value, "warp x {value}");
        }
    }

    /// The two `long long` fields of the point-change record span the full
    /// signed 64-bit range, so a truncating or converting read is visible.
    #[test]
    fn the_point_change_long_longs_keep_their_full_64_bit_range() {
        for value in [i64::MIN, -4_294_967_297, -1, 0, 1, 4_294_967_297, i64::MAX] {
            let mut w = Vec::new();
            GcPointChange {
                header: GC_POINT_CHANGE_HEADER,
                vid: 0,
                change_type: 0,
                amount: value,
                value: !value,
            }
            .encode_into(&mut w);
            assert_eq!(w.len(), GC_POINT_CHANGE_WIRE_SIZE, "the width is stable");
            let r = GcPointChange::decode(&w).unwrap();
            assert_eq!(r.amount, value, "amount {value}");
            assert_eq!(r.value, !value, "value {value}");
        }
    }

    /// The cube-renewal `count_material` fields are signed and must keep
    /// negative values, which a `u32` read followed by a saturating conversion
    /// would fold to zero.
    #[test]
    fn the_cube_material_counts_stay_signed() {
        for value in [i32::MIN, -65_537, -1, 0, 1, 65_536, i32::MAX] {
            let mut date = sample_date();
            for slot in &mut date.count_material {
                *slot = value;
            }
            let mut w = Vec::new();
            date.encode_into(&mut w);
            let r = GcCubeRenewalDate::decode(&w).unwrap();
            for (index, got) in r.count_material.iter().enumerate() {
                assert_eq!(*got, value, "count_material[{index}] at {value}");
            }
            assert_eq!(
                r.count_reward,
                sample_date().count_reward,
                "count_reward is untouched"
            );
        }
    }

    /// The `time_t` field is a signed 4-byte word, so the negative half of the
    /// range is reachable and must not fold to a large unsigned value.
    #[test]
    fn the_skill_time_t_keeps_its_signed_range() {
        for value in [i32::MIN, -65_537, -1, 0, 1, 65_536, i32::MAX] {
            let skill = GcSkill {
                master_type: 0,
                level: 0,
                next_read: value,
            };
            let mut w = Vec::new();
            skill.encode_into(&mut w);
            assert_eq!(w.len(), GC_SKILL_WIRE_SIZE, "the entry is 6 bytes");
            assert_eq!(GcSkill::decode(&w).unwrap().next_read, value, "{value}");
            // The whole 255-entry table must keep every value too.
            let mut table = [skill; GC_SKILL_SLOT_COUNT];
            for (index, slot) in table.iter_mut().enumerate() {
                slot.level = u8::try_from(index % 256).unwrap_or(0);
            }
            let mut w = Vec::new();
            GcSkillLevelNew {
                header: GcSkillLevelNew::header(),
                skills: table,
            }
            .encode_into(&mut w);
            assert_eq!(w.len(), GC_SKILL_LEVEL_NEW_WIRE_SIZE, "the table is 1531");
            let r = GcSkillLevelNew::decode(&w).unwrap();
            for (index, got) in r.skills.iter().enumerate() {
                assert_eq!(got.next_read, value, "skills[{index}].next_read");
            }
        }
    }
}
