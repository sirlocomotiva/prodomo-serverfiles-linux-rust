//! The game-to-client records that carry one 32-bit value.
//!
//! # Provenance
//!
//! Every record here is a `#pragma pack(1)` structure of 5 to 7 bytes. Widths
//! were hand-summed from the struct source on **both** sides and cross-checked
//! against the client's decode table in
//! `client/Client/UserInterface/PythonNetworkStream.cpp` and a server producer.
//! None of these widths comes from the rejected automated width calculator
//! described in ledger section 163.7.
//!
//! # Widths that depend on the legacy target
//!
//! `server/server/premake5.lua:12` sets `architecture "x86"`, so the legacy
//! target is 32-bit. That fixes the two non-fixed-width field types used here:
//! C++ `long` is 4 bytes and `time_t` is 4 bytes, with no `_TIME_BITS=64`
//! definition anywhere in the tree. Both therefore occupy the same 4 bytes as a
//! `DWORD`, and the records that use them are 5 bytes rather than 9.
//!
//! # Two feature profiles that both resolve to `DWORD`
//!
//! `TPacketGCItemDel`, which carries both `HEADER_GC_SAFEBOX_DEL` (86) and
//! `HEADER_GC_MALL_DEL` (129), is
//! `#if defined(__EXTENDED_SAFEBOX__)` a `DWORD pos`, otherwise a `BYTE pos`.
//! The macro **is** defined on both sides, at
//! `server/server/common/prodomodefines.h:61` and
//! `client/Client/UserInterface/LOCALE_INC.H:35`, so the active profile is the
//! 5-byte `DWORD` form. The 2-byte form is unreachable and is not implemented.
//!
//! That client macro lives in the one uppercase-extension header in the tree.
//! It is invisible to `grep --include=*.h`, which is the defect recorded in
//! ledger section 163.1.
//!
//! # Scope
//!
//! Record codecs only. Nothing here installs a key, touches a socket, resolves
//! a party, moves a character, changes an affect, refines an item, or mutates
//! a session. The 32-bit values stay opaque: the legacy code assigns them
//! meanings at each use site, and the `switch` statements on the client silently
//! ignore values they do not know.

use std::fmt;

use crate::item_pos::{ItemPos, ItemPosError, ITEM_POS_WIRE_SIZE};

/// Packed width of a record that is a header plus one 32-bit value.
pub const GC_HEADER_AND_DWORD_WIRE_SIZE: usize = 1 + 4;

/// Packed width of a record that is a header, one 32-bit value, and one `BYTE`.
pub const GC_HEADER_AND_DWORD_AND_BYTE_WIRE_SIZE: usize = 1 + 4 + 1;

/// Packed width of a record that is a header, one `BYTE`, and one 32-bit value.
pub const GC_HEADER_AND_BYTE_AND_DWORD_WIRE_SIZE: usize = 1 + 1 + 4;

/// Packed width of a record that is a header, a subheader, a 32-bit value, and a
/// `BYTE`.
pub const GC_FISHING_WIRE_SIZE: usize = 1 + 1 + 4 + 1;

/// Packed width of a record that is a header, a 32-bit value, and a `WORD`.
pub const GC_CHANGE_SPEED_WIRE_SIZE: usize = 1 + 4 + 2;

/// Packed width of a record that is a header followed by three `WORD`s.
pub const GC_EVENT_KW_SCORE_WIRE_SIZE: usize = 1 + 2 * 3;

/// Packed width of a record that is a header, two `WORD`s, and a `BYTE`.
pub const GC_REFINE_ELEMENT_WIRE_SIZE: usize = 1 + 2 + 2 + 1;

/// Packed width of [`GcDragonSoulRefine`], which nests the shared 3-byte slot.
pub const GC_DRAGON_SOUL_REFINE_WIRE_SIZE: usize = 1 + 1 + ITEM_POS_WIRE_SIZE;

/// A game-to-client record that is a header plus one opaque 32-bit value.
///
/// Eleven legacy records share this exact 5-byte shape. The 32-bit field is a
/// `DWORD`, a `long`, or a `time_t` depending on the record, and on this 32-bit
/// target all three occupy 4 bytes. The value's meaning is assigned by the
/// producer, so it stays a raw `u32`.
///
/// | byte | record | 32-bit field | meaning |
/// |---|---|---|---|
/// | 2 | `HEADER_GC_CHARACTER_DEL` | `dwVID` / `id` | departing character's VID |
/// | 13 | `HEADER_GC_STUN` | `vid` | stunned character's VID |
/// | 14 | `HEADER_GC_DEAD` | `vid` | dead character's VID |
/// | 27 | `HEADER_GC_ITEM_GROUND_DEL` | `vid` | actor VID |
/// | 77 | `HEADER_GC_PARTY_INVITE` | `leader_pid` / `leader_vid` | inviting party leader |
/// | 80 | `HEADER_GC_PARTY_REMOVE` | `pid` | removed party member |
/// | 84 | `HEADER_GC_SAFEBOX_MONEY_CHANGE` | `dwMoney` / `lMoney` | new safebox balance |
/// | 86 | `HEADER_GC_SAFEBOX_DEL` | `pos` | safebox cell, `DWORD` under `__EXTENDED_SAFEBOX__` |
/// | 106 | `HEADER_GC_TIME` | `time` | server `time_t` |
/// | 124 | `HEADER_GC_TARGET_DELETE` | `lID` | deleted target VID |
/// | 129 | `HEADER_GC_MALL_DEL` | `pos` | same struct as `HEADER_GC_SAFEBOX_DEL` |
///
/// `HEADER_GC_SAFEBOX_MONEY_CHANGE` has no server producer: a whole-token search
/// over `server/server` returns zero hits and the server has no enumerator for
/// byte 84. It is still a registered client decode entry, so its framing is real.
///
/// Bytes 86 and 129 are sent from a single ternary at
/// `server/server/game/safebox.cpp:119`, so one struct covers both and the
/// header is the only thing separating them. The two shapes are not
/// interchangeable: byte 86 and byte 129 are different wire bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcHeaderAndDword {
    /// The record's own header byte.
    pub header: u8,
    /// The single opaque 32-bit value, preserved verbatim.
    pub value: u32,
}

impl GcHeaderAndDword {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_HEADER_AND_DWORD_WIRE_SIZE;

    /// Builds the record for a known header.
    pub const fn new(header: u8, value: u32) -> Self {
        Self { header, value }
    }

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcHeaderAndDword::WIRE_SIZE`]. The header is accepted as any byte,
    /// because this shape is shared by eleven records and the caller knows which
    /// one it expects.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcHeaderAndDword",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            header: bytes[0],
            value: u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.header);
        out.extend_from_slice(&self.value.to_le_bytes());
    }
}

/// A game-to-client record that is a header, a 32-bit value, and one `BYTE`.
///
/// Three records share this 6-byte shape:
///
/// | byte | record | `DWORD` field | `BYTE` field |
/// |---|---|---|---|
/// | 43 | `HEADER_GC_CHARACTER_POSITION` | `vid` | `position` |
/// | 111 | `HEADER_GC_WALK_MODE` | `vid` | `mode` |
/// | 127 | `HEADER_GC_AFFECT_REMOVE` | `dwType` | `bApplyOn` |
///
/// Note that the 32-bit field is **not** always a VID: `HEADER_GC_AFFECT_REMOVE`
/// carries an affect type, and `bApplyOn` is a `BYTE` rather than a flag set.
/// Every value is preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcHeaderAndDwordAndByte {
    /// The record's own header byte.
    pub header: u8,
    /// The 32-bit field, preserved verbatim.
    pub value: u32,
    /// The trailing `BYTE` field, preserved verbatim.
    pub flag: u8,
}

impl GcHeaderAndDwordAndByte {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_HEADER_AND_DWORD_AND_BYTE_WIRE_SIZE;

    /// Builds the record for a known header.
    pub const fn new(header: u8, value: u32, flag: u8) -> Self {
        Self {
            header,
            value,
            flag,
        }
    }

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcHeaderAndDwordAndByte::WIRE_SIZE`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcHeaderAndDwordAndByte",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            header: bytes[0],
            value: u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
            flag: bytes[5],
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.header);
        out.extend_from_slice(&self.value.to_le_bytes());
        out.push(self.flag);
    }
}

/// `HEADER_GC_SEPCIAL_EFFECT` (byte 114).
///
/// Source: `{ BYTE header; BYTE type; DWORD vid; }` in both
/// `client/.../Packet.h` and `server/server/game/packet.h`. Packed width 6.
///
/// This record gets its own type even though it is also 6 bytes, because its
/// field **order** differs from [`GcHeaderAndDwordAndByte`]: the `BYTE` comes
/// first and the `DWORD` second. A shared type would silently transpose them.
///
/// The name is spelled `SEPCIAL` in both trees, which is a typo in the legacy
/// source. It is preserved because the header enumerator is the wire contract.
/// Sent from `char.cpp:7537`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcSpecialEffect {
    /// Legacy `type`.
    pub effect_type: u8,
    /// Legacy `vid`, the character the effect applies to.
    pub vid: u32,
}

impl GcSpecialEffect {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_HEADER_AND_BYTE_AND_DWORD_WIRE_SIZE;

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcSpecialEffect::WIRE_SIZE`], and [`GcSmallError::Header`] unless the
    /// first byte is [`HEADER_GC_SEPCIAL_EFFECT`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcSpecialEffect",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_SEPCIAL_EFFECT {
            return Err(GcSmallError::Header {
                context: "GcSpecialEffect",
                expected: HEADER_GC_SEPCIAL_EFFECT,
                actual: bytes[0],
            });
        }
        Ok(Self {
            effect_type: bytes[1],
            vid: u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_SEPCIAL_EFFECT);
        out.push(self.effect_type);
        out.extend_from_slice(&self.vid.to_le_bytes());
    }
}

/// `HEADER_GC_REFINE_ELEMENT` (byte 228).
///
/// Source: `{ BYTE bHeader; WORD wSrcCell; WORD wDstCell; BYTE bType; }` in
/// both trees. Packed width 1 + 2 + 2 + 1 = 6. Both `WORD`s are little-endian.
///
/// Sent from `char_item.cpp:10445`. The cell indices and the type byte are
/// preserved as raw words; the module does not resolve an element table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcRefineElement {
    /// Legacy `wSrcCell`.
    pub src_cell: u16,
    /// Legacy `wDstCell`.
    pub dst_cell: u16,
    /// Legacy `bType`.
    pub element_type: u8,
}

impl GcRefineElement {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_REFINE_ELEMENT_WIRE_SIZE;

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcRefineElement::WIRE_SIZE`], and [`GcSmallError::Header`] unless the
    /// first byte is [`HEADER_GC_REFINE_ELEMENT`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcRefineElement",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_REFINE_ELEMENT {
            return Err(GcSmallError::Header {
                context: "GcRefineElement",
                expected: HEADER_GC_REFINE_ELEMENT,
                actual: bytes[0],
            });
        }
        Ok(Self {
            src_cell: u16::from_le_bytes([bytes[1], bytes[2]]),
            dst_cell: u16::from_le_bytes([bytes[3], bytes[4]]),
            element_type: bytes[5],
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_REFINE_ELEMENT);
        out.extend_from_slice(&self.src_cell.to_le_bytes());
        out.extend_from_slice(&self.dst_cell.to_le_bytes());
        out.push(self.element_type);
    }
}

/// `HEADER_GC_CHANGE_SPEED` (byte 18).
///
/// Source: `{ BYTE header; DWORD vid; WORD moving_speed; }` in both trees.
/// Packed width 1 + 4 + 2 = 7.
///
/// The server has the enumerator and the struct but **no producer**: a
/// whole-token search finds the header only at its own definition in
/// `server/server/game/packet.h`. The client registers and decodes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcChangeSpeed {
    /// Legacy `vid`.
    pub vid: u32,
    /// Legacy `moving_speed`.
    pub moving_speed: u16,
}

impl GcChangeSpeed {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_CHANGE_SPEED_WIRE_SIZE;

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcChangeSpeed::WIRE_SIZE`], and [`GcSmallError::Header`] unless the
    /// first byte is [`HEADER_GC_CHANGE_SPEED`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcChangeSpeed",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_CHANGE_SPEED {
            return Err(GcSmallError::Header {
                context: "GcChangeSpeed",
                expected: HEADER_GC_CHANGE_SPEED,
                actual: bytes[0],
            });
        }
        Ok(Self {
            vid: u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
            moving_speed: u16::from_le_bytes([bytes[5], bytes[6]]),
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_CHANGE_SPEED);
        out.extend_from_slice(&self.vid.to_le_bytes());
        out.extend_from_slice(&self.moving_speed.to_le_bytes());
    }
}

/// `HEADER_GC_EVENT_KW_SCORE` (byte 157).
///
/// Source: `{ BYTE bHeader; WORD wKingdomScores[3]; }` in both trees. Packed
/// width 1 + 2 * 3 = 7. The three kingdoms are in array order.
///
/// Sent from `event_manager.cpp:1737`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcEventKwScore {
    /// Legacy `wKingdomScores`, in kingdom order.
    pub kingdom_scores: [u16; 3],
}

impl GcEventKwScore {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_EVENT_KW_SCORE_WIRE_SIZE;

    /// Builds the record from three kingdom scores.
    pub const fn new(scores: [u16; 3]) -> Self {
        Self {
            kingdom_scores: scores,
        }
    }

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcEventKwScore::WIRE_SIZE`], and [`GcSmallError::Header`] unless the
    /// first byte is [`HEADER_GC_EVENT_KW_SCORE`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcEventKwScore",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_EVENT_KW_SCORE {
            return Err(GcSmallError::Header {
                context: "GcEventKwScore",
                expected: HEADER_GC_EVENT_KW_SCORE,
                actual: bytes[0],
            });
        }
        let mut kingdom_scores = [0u16; 3];
        for (index, slot) in kingdom_scores.iter_mut().enumerate() {
            let at = 1 + index * 2;
            *slot = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        }
        Ok(Self { kingdom_scores })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_EVENT_KW_SCORE);
        for score in self.kingdom_scores {
            out.extend_from_slice(&score.to_le_bytes());
        }
    }
}

/// `HEADER_GC_DRAGON_SOUL_REFINE` (byte 209).
///
/// Source: `SPacketGCDragonSoulRefine` in `server/server/game/packet.h` and the
/// client's `TPacketGCDragonSoulRefine`, both
/// `{ BYTE header; BYTE bSubType; TItemPos Pos; }`. Packed width
/// 1 + 1 + 3 = 5, because `TItemPos` is the shared packed 3-byte grid position
/// and this is its third consumer after `cg_sash` and `cg_dragon_soul`.
///
/// `SPacketGCDragonSoulRefine` is a C++ **class** with a user-supplied
/// constructor rather than a plain struct. That does not change the layout: the
/// constructor only initialises `header`, there is no base class, no virtual
/// member, and `pack(1)` applies.
///
/// Sent from `char_dragonsoul.cpp:146`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcDragonSoulRefine {
    /// Legacy `bSubType`.
    pub sub_type: u8,
    /// Legacy `Pos`, the shared 3-byte grid position.
    pub pos: ItemPos,
}

impl GcDragonSoulRefine {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_DRAGON_SOUL_REFINE_WIRE_SIZE;

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcDragonSoulRefine::WIRE_SIZE`], [`GcSmallError::Header`] unless the
    /// first byte is [`HEADER_GC_DRAGON_SOUL_REFINE`], and
    /// [`GcSmallError::ItemPos`] if the nested 3-byte grid position is not a
    /// valid `ItemPos`.
    ///
    /// The nested arm is currently unreachable: `ItemPos::decode` rejects only a
    /// wrong length, never a wrong value, and the length is checked above. The
    /// mapping is kept so that this record stays correct if `ItemPos` ever gains
    /// value validation, and it is exercised directly in the tests.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcDragonSoulRefine",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_DRAGON_SOUL_REFINE {
            return Err(GcSmallError::Header {
                context: "GcDragonSoulRefine",
                expected: HEADER_GC_DRAGON_SOUL_REFINE,
                actual: bytes[0],
            });
        }
        let pos = ItemPos::decode(&bytes[2..2 + ITEM_POS_WIRE_SIZE]).map_err(|source| {
            GcSmallError::ItemPos {
                context: "GcDragonSoulRefine",
                source,
            }
        })?;
        Ok(Self {
            sub_type: bytes[1],
            pos,
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_DRAGON_SOUL_REFINE);
        out.push(self.sub_type);
        self.pos.encode_into(out);
    }
}

/// The `FISHING_SUBHEADER_GC_*` sub-header of [`GcFishing`].
///
/// The legacy enum is anonymous and auto-increments from zero in both trees, in
/// the same order, so these values are the enumerator positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcFishingSubheader {
    /// `FISHING_SUBHEADER_GC_START` = 0.
    Start,
    /// `FISHING_SUBHEADER_GC_STOP` = 1.
    Stop,
    /// `FISHING_SUBHEADER_GC_REACT` = 2.
    React,
    /// `FISHING_SUBHEADER_GC_SUCCESS` = 3.
    Success,
    /// `FISHING_SUBHEADER_GC_FAIL` = 4.
    Fail,
    /// `FISHING_SUBHEADER_GC_FISH` = 5.
    Fish,
    /// A value the legacy enum does not define.
    ///
    /// The client's `switch` has no `default` arm, so an unknown sub-header is
    /// consumed and ignored rather than rejected. Preserving it as a raw value
    /// keeps that behaviour reachable instead of turning it into a decode error.
    Unknown(u8),
}

impl Default for GcFishingSubheader {
    /// The legacy enumerator starts at zero, so the zero value is
    /// [`GcFishingSubheader::Start`].
    fn default() -> Self {
        Self::Start
    }
}

impl GcFishingSubheader {
    /// The raw wire byte for this sub-header.
    pub const fn value(self) -> u8 {
        match self {
            Self::Start => 0,
            Self::Stop => 1,
            Self::React => 2,
            Self::Success => 3,
            Self::Fail => 4,
            Self::Fish => 5,
            Self::Unknown(raw) => raw,
        }
    }

    /// Maps a wire byte to a sub-header, with an explicit unknown arm.
    pub const fn from_value(raw: u8) -> Self {
        match raw {
            0 => Self::Start,
            1 => Self::Stop,
            2 => Self::React,
            3 => Self::Success,
            4 => Self::Fail,
            5 => Self::Fish,
            other => Self::Unknown(other),
        }
    }

    /// Whether this is one of the six enumerated sub-headers.
    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

/// `HEADER_GC_FISHING` (byte 89).
///
/// Source: `{ BYTE header; BYTE subheader; DWORD info; BYTE dir; }` in both
/// trees. Packed width 1 + 1 + 4 + 1 = 7, and the width is the same for every
/// sub-header, so the frame length does not depend on the sub-header value.
///
/// The `info` field is **not** one thing. For `Start`, `Stop`, `React`,
/// `Success`, and `Fail` the server writes the character's `GetVID()`; for
/// `Fish` it writes the fish item's `vnum` (`fishing.cpp:502`). The client
/// branches on the sub-header before interpreting it
/// (`PythonNetworkStreamPhaseGame.cpp:4399-4401`). This module therefore keeps
/// `info` as a raw `u32` and exposes the two readings as named accessors rather
/// than pretending there is a single field.
///
/// Sent from `fishing.cpp:406`, `:415`, `:424`, and `:501`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcFishing {
    /// Legacy `subheader`, the typed sub-header discriminator.
    pub subheader: GcFishingSubheader,
    /// Legacy `info`. A character VID for five sub-headers, a fish item vnum for
    /// `Fish`. Kept raw because the meaning depends on the sub-header.
    pub info: u32,
    /// Legacy `dir`, a signed direction multiplier the client scales by 5.0 for
    /// `Start`. Kept as a raw `u8` rather than an `i8` so the exact byte
    /// survives.
    pub dir: u8,
}

impl GcFishing {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_FISHING_WIRE_SIZE;

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcFishing::WIRE_SIZE`], and [`GcSmallError::Header`] unless the first
    /// byte is [`HEADER_GC_FISHING`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcFishing",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_FISHING {
            return Err(GcSmallError::Header {
                context: "GcFishing",
                expected: HEADER_GC_FISHING,
                actual: bytes[0],
            });
        }
        Ok(Self {
            subheader: GcFishingSubheader::from_value(bytes[1]),
            info: u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            dir: bytes[6],
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_FISHING);
        out.push(self.subheader.value());
        out.extend_from_slice(&self.info.to_le_bytes());
        out.push(self.dir);
    }

    /// Reads `info` as the character VID, for the five sub-headers that carry one.
    ///
    /// Returns `None` for [`GcFishingSubheader::Fish`], where the field holds a
    /// fish item vnum instead, and for an unknown sub-header.
    pub const fn actor_vid(&self) -> Option<u32> {
        match self.subheader {
            GcFishingSubheader::Fish | GcFishingSubheader::Unknown(_) => None,
            _ => Some(self.info),
        }
    }

    /// Reads `info` as the fish item vnum, for the `Fish` sub-header only.
    pub const fn fish_vnum(&self) -> Option<u32> {
        match self.subheader {
            GcFishingSubheader::Fish => Some(self.info),
            _ => None,
        }
    }
}

/// Failure decoding one of the 32-bit-value game-to-client records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcSmallError {
    /// The buffer ended before the record did.
    Truncated {
        /// Which record was being decoded.
        context: &'static str,
        /// Bytes the record needs.
        needed: usize,
        /// Bytes the buffer actually had.
        actual: usize,
    },
    /// The leading byte is not this record's header.
    Header {
        /// Which record was being decoded.
        context: &'static str,
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was present.
        actual: u8,
    },
    /// The nested shared record could not be decoded.
    ItemPos {
        /// Which record was being decoded.
        context: &'static str,
        /// The nested failure.
        source: ItemPosError,
    },
}

impl fmt::Display for GcSmallError {
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
            Self::ItemPos { context, source } => {
                write!(f, "{context}: nested grid position is invalid: {source}")
            }
        }
    }
}

impl std::error::Error for GcSmallError {}

/// `HEADER_GC_CHARACTER_DEL` (2).
pub const HEADER_GC_CHARACTER_DEL: u8 = 0x02;
/// `HEADER_GC_CHANGE_SPEED` (18).
pub const HEADER_GC_CHANGE_SPEED: u8 = 0x12;
/// `HEADER_GC_ITEM_GROUND_DEL` (27).
pub const HEADER_GC_ITEM_GROUND_DEL: u8 = 0x1b;
/// `HEADER_GC_CHARACTER_POSITION` (43).
pub const HEADER_GC_CHARACTER_POSITION: u8 = 0x2b;
/// `HEADER_GC_STUN` (13).
pub const HEADER_GC_STUN: u8 = 0x0d;
/// `HEADER_GC_DEAD` (14).
pub const HEADER_GC_DEAD: u8 = 0x0e;
/// `HEADER_GC_PARTY_INVITE` (77).
pub const HEADER_GC_PARTY_INVITE: u8 = 0x4d;
/// `HEADER_GC_PARTY_REMOVE` (80).
pub const HEADER_GC_PARTY_REMOVE: u8 = 0x50;
/// `HEADER_GC_SAFEBOX_MONEY_CHANGE` (84). The server never sends this.
pub const HEADER_GC_SAFEBOX_MONEY_CHANGE: u8 = 0x54;
/// `HEADER_GC_SAFEBOX_DEL` (86).
pub const HEADER_GC_SAFEBOX_DEL: u8 = 0x56;
/// `HEADER_GC_FISHING` (89).
pub const HEADER_GC_FISHING: u8 = 0x59;
/// `HEADER_GC_TIME` (106).
pub const HEADER_GC_TIME: u8 = 0x6a;
/// `HEADER_GC_WALK_MODE` (111).
pub const HEADER_GC_WALK_MODE: u8 = 0x6f;
/// `HEADER_GC_SEPCIAL_EFFECT` (114). The legacy name contains a typo.
pub const HEADER_GC_SEPCIAL_EFFECT: u8 = 0x72;
/// `HEADER_GC_AFFECT_REMOVE` (127).
pub const HEADER_GC_AFFECT_REMOVE: u8 = 0x7f;
/// `HEADER_GC_TARGET_DELETE` (124).
pub const HEADER_GC_TARGET_DELETE: u8 = 0x7c;
/// `HEADER_GC_MALL_DEL` (129).
pub const HEADER_GC_MALL_DEL: u8 = 0x81;
/// `HEADER_GC_EVENT_KW_SCORE` (157).
pub const HEADER_GC_EVENT_KW_SCORE: u8 = 0x9d;
/// `HEADER_GC_DRAGON_SOUL_REFINE` (209).
pub const HEADER_GC_DRAGON_SOUL_REFINE: u8 = 0xd1;
/// `HEADER_GC_REFINE_ELEMENT` (228).
pub const HEADER_GC_REFINE_ELEMENT: u8 = 0xe4;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The eleven records that share the 5-byte header-plus-DWORD shape.
    const DWORD_RECORDS: [(u8, &str); 11] = [
        (HEADER_GC_CHARACTER_DEL, "CHARACTER_DEL"),
        (HEADER_GC_STUN, "STUN"),
        (HEADER_GC_DEAD, "DEAD"),
        (HEADER_GC_ITEM_GROUND_DEL, "ITEM_GROUND_DEL"),
        (HEADER_GC_PARTY_INVITE, "PARTY_INVITE"),
        (HEADER_GC_PARTY_REMOVE, "PARTY_REMOVE"),
        (HEADER_GC_SAFEBOX_MONEY_CHANGE, "SAFEBOX_MONEY_CHANGE"),
        (HEADER_GC_SAFEBOX_DEL, "SAFEBOX_DEL"),
        (HEADER_GC_TIME, "TIME"),
        (HEADER_GC_TARGET_DELETE, "TARGET_DELETE"),
        (HEADER_GC_MALL_DEL, "MALL_DEL"),
    ];

    /// The three records that share the 6-byte header, DWORD, BYTE shape.
    const DWORD_BYTE_RECORDS: [(u8, &str); 3] = [
        (HEADER_GC_CHARACTER_POSITION, "CHARACTER_POSITION"),
        (HEADER_GC_WALK_MODE, "WALK_MODE"),
        (HEADER_GC_AFFECT_REMOVE, "AFFECT_REMOVE"),
    ];

    #[test]
    fn the_documented_header_values_match_the_legacy_tables() {
        assert_eq!(HEADER_GC_CHARACTER_DEL, 2);
        assert_eq!(HEADER_GC_STUN, 13);
        assert_eq!(HEADER_GC_DEAD, 14);
        assert_eq!(HEADER_GC_CHANGE_SPEED, 18);
        assert_eq!(HEADER_GC_ITEM_GROUND_DEL, 27);
        assert_eq!(HEADER_GC_CHARACTER_POSITION, 43);
        assert_eq!(HEADER_GC_PARTY_INVITE, 77);
        assert_eq!(HEADER_GC_PARTY_REMOVE, 80);
        assert_eq!(HEADER_GC_SAFEBOX_MONEY_CHANGE, 84);
        assert_eq!(HEADER_GC_SAFEBOX_DEL, 86);
        assert_eq!(HEADER_GC_FISHING, 89);
        assert_eq!(HEADER_GC_TIME, 106);
        assert_eq!(HEADER_GC_WALK_MODE, 111);
        assert_eq!(HEADER_GC_SEPCIAL_EFFECT, 114);
        assert_eq!(HEADER_GC_TARGET_DELETE, 124);
        assert_eq!(HEADER_GC_AFFECT_REMOVE, 127);
        assert_eq!(HEADER_GC_MALL_DEL, 129);
        assert_eq!(HEADER_GC_EVENT_KW_SCORE, 157);
        assert_eq!(HEADER_GC_DRAGON_SOUL_REFINE, 209);
        assert_eq!(HEADER_GC_REFINE_ELEMENT, 228);
    }

    #[test]
    fn the_shapes_capture_twenty_distinct_header_bytes() {
        let all: BTreeSet<u8> = DWORD_RECORDS
            .iter()
            .chain(DWORD_BYTE_RECORDS.iter())
            .map(|entry| entry.0)
            .chain([
                HEADER_GC_CHANGE_SPEED,
                HEADER_GC_SEPCIAL_EFFECT,
                HEADER_GC_REFINE_ELEMENT,
                HEADER_GC_EVENT_KW_SCORE,
                HEADER_GC_DRAGON_SOUL_REFINE,
                HEADER_GC_FISHING,
            ])
            .collect();
        assert_eq!(all.len(), 11 + 3 + 6);
        assert_eq!(all.len(), 20);
    }

    #[test]
    fn the_packed_widths_match_the_hand_summed_field_lists() {
        assert_eq!(GC_HEADER_AND_DWORD_WIRE_SIZE, 5);
        assert_eq!(GC_HEADER_AND_DWORD_AND_BYTE_WIRE_SIZE, 6);
        assert_eq!(GC_HEADER_AND_BYTE_AND_DWORD_WIRE_SIZE, 6);
        assert_eq!(GC_REFINE_ELEMENT_WIRE_SIZE, 6);
        assert_eq!(GC_DRAGON_SOUL_REFINE_WIRE_SIZE, 5);
        assert_eq!(GC_CHANGE_SPEED_WIRE_SIZE, 7);
        assert_eq!(GC_EVENT_KW_SCORE_WIRE_SIZE, 7);
        assert_eq!(GC_FISHING_WIRE_SIZE, 7);
        assert_eq!(ITEM_POS_WIRE_SIZE, 3);
    }

    #[test]
    fn the_header_and_dword_shape_round_trips_every_relevant_value() {
        for (header, name) in DWORD_RECORDS {
            for value in [0u32, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
                let record = GcHeaderAndDword::new(header, value);
                let mut wire = Vec::new();
                record.encode_into(&mut wire);
                assert_eq!(wire.len(), GC_HEADER_AND_DWORD_WIRE_SIZE, "{name}");
                assert_eq!(wire[0], header, "{name}");
                assert_eq!(&wire[1..], &value.to_le_bytes(), "{name}");
                assert_eq!(GcHeaderAndDword::decode(&wire), Ok(record), "{name}");
            }
        }
    }

    #[test]
    fn the_header_and_dword_shape_keeps_every_32_bit_pattern() {
        // The field is a raw u32, not a signed type, so no value is normalised.
        for value in [0x8000_0000u32, 0xffff_ffff, 0xdead_beef] {
            let record = GcHeaderAndDword::new(HEADER_GC_TIME, value);
            let mut wire = Vec::new();
            record.encode_into(&mut wire);
            assert_eq!(GcHeaderAndDword::decode(&wire).unwrap().value, value);
        }
    }

    #[test]
    fn the_header_and_dword_shape_rejects_every_short_buffer() {
        for len in 0..GC_HEADER_AND_DWORD_WIRE_SIZE {
            let buf = vec![HEADER_GC_TIME; len];
            assert_eq!(
                GcHeaderAndDword::decode(&buf),
                Err(GcSmallError::Truncated {
                    context: "GcHeaderAndDword",
                    needed: GC_HEADER_AND_DWORD_WIRE_SIZE,
                    actual: len,
                })
            );
        }
    }

    #[test]
    fn the_header_and_dword_shape_accepts_any_header_because_it_is_shared() {
        // The caller knows which of the eleven records it expects; the codec
        // must not silently claim a header is wrong.
        for header in [0x00u8, 0x02, 0x6a, 0xff] {
            let record = GcHeaderAndDword::new(header, 0);
            let mut wire = Vec::new();
            record.encode_into(&mut wire);
            assert_eq!(GcHeaderAndDword::decode(&wire).unwrap().header, header);
        }
    }

    #[test]
    fn safebox_del_and_mall_del_share_a_struct_but_not_a_header() {
        // safebox.cpp:119 sends one TPacketGCItemDel with either header.
        let safebox = GcHeaderAndDword::new(HEADER_GC_SAFEBOX_DEL, 100);
        let mall = GcHeaderAndDword::new(HEADER_GC_MALL_DEL, 100);
        assert_ne!(safebox.header, mall.header);
        assert_eq!(safebox.value, mall.value);
        let mut wire = Vec::new();
        safebox.encode_into(&mut wire);
        mall.encode_into(&mut wire);
        assert_eq!(
            wire,
            vec![0x56, 100, 0, 0, 0, 0x81, 100, 0, 0, 0],
            "the active __EXTENDED_SAFEBOX__ profile carries a DWORD pos"
        );
    }

    #[test]
    fn the_header_and_dword_and_byte_shape_round_trips() {
        for (header, name) in DWORD_BYTE_RECORDS {
            for flag in [0u8, 1, 0x7f, 0x80, 0xff] {
                let record = GcHeaderAndDwordAndByte::new(header, 0x0102_0304, flag);
                let mut wire = Vec::new();
                record.encode_into(&mut wire);
                assert_eq!(wire.len(), GC_HEADER_AND_DWORD_AND_BYTE_WIRE_SIZE, "{name}");
                assert_eq!(wire[0], header, "{name}");
                assert_eq!(
                    &wire[1..5],
                    &[0x04, 0x03, 0x02, 0x01],
                    "{name} is little endian"
                );
                assert_eq!(wire[5], flag, "{name}");
                assert_eq!(GcHeaderAndDwordAndByte::decode(&wire), Ok(record), "{name}");
            }
        }
    }

    #[test]
    fn the_header_and_dword_and_byte_shape_rejects_a_five_byte_buffer() {
        assert_eq!(
            GcHeaderAndDwordAndByte::decode(&[0x6f; 5]),
            Err(GcSmallError::Truncated {
                context: "GcHeaderAndDwordAndByte",
                needed: 6,
                actual: 5,
            })
        );
    }

    /// `TPacketGCAffectRemove` (`G/packet.h`): the header, the affect type, and `bApplyOn`
    /// last. The revive-invisible affect is type 215 with apply 0, so it is `7f d7 00 00 00 00`.
    #[test]
    fn the_affect_remove_record_is_the_type_then_the_apply_byte() {
        let record = GcHeaderAndDwordAndByte::new(HEADER_GC_AFFECT_REMOVE, 215, 0);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire, [0x7f, 0xd7, 0, 0, 0, 0]);
    }

    #[test]
    fn the_special_effect_record_puts_the_byte_before_the_dword() {
        // { BYTE header; BYTE type; DWORD vid; } -- the opposite order from
        // GcHeaderAndDwordAndByte, which is exactly why it gets its own type.
        let record = GcSpecialEffect {
            effect_type: 0x11,
            vid: 0x2233_4455,
        };
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(
            wire,
            vec![HEADER_GC_SEPCIAL_EFFECT, 0x11, 0x55, 0x44, 0x33, 0x22],
            "type precedes vid, so the little-endian vid starts at offset 2"
        );
        assert_eq!(GcSpecialEffect::decode(&wire), Ok(record));
    }

    #[test]
    fn the_special_effect_record_rejects_a_wrong_header() {
        let err = GcSpecialEffect::decode(&[HEADER_GC_WALK_MODE, 1, 2, 3, 4, 5]).unwrap_err();
        assert_eq!(
            err,
            GcSmallError::Header {
                context: "GcSpecialEffect",
                expected: HEADER_GC_SEPCIAL_EFFECT,
                actual: HEADER_GC_WALK_MODE,
            }
        );
    }

    #[test]
    fn the_refine_element_record_is_six_bytes_with_two_words_then_a_byte() {
        let record = GcRefineElement {
            src_cell: 0x0102,
            dst_cell: 0x0304,
            element_type: 0x05,
        };
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire, vec![0xe4, 0x02, 0x01, 0x04, 0x03, 0x05]);
        assert_eq!(wire.len(), GC_REFINE_ELEMENT_WIRE_SIZE);
        assert_eq!(GcRefineElement::decode(&wire), Ok(record));
    }

    #[test]
    fn the_refine_element_record_rejects_every_short_buffer_and_a_wrong_header() {
        for len in 0..GC_REFINE_ELEMENT_WIRE_SIZE {
            let buf = vec![HEADER_GC_REFINE_ELEMENT; len];
            assert_eq!(
                GcRefineElement::decode(&buf),
                Err(GcSmallError::Truncated {
                    context: "GcRefineElement",
                    needed: GC_REFINE_ELEMENT_WIRE_SIZE,
                    actual: len,
                })
            );
        }
        assert!(matches!(
            GcRefineElement::decode(&[HEADER_GC_CHANGE_SPEED, 0, 0, 0, 0, 0]),
            Err(GcSmallError::Header {
                expected: HEADER_GC_REFINE_ELEMENT,
                actual: HEADER_GC_CHANGE_SPEED,
                ..
            })
        ));
    }

    #[test]
    fn the_change_speed_record_is_seven_bytes_with_the_word_last() {
        let record = GcChangeSpeed {
            vid: 0x0a0b_0c0d,
            moving_speed: 0x0102,
        };
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(
            wire,
            vec![HEADER_GC_CHANGE_SPEED, 0x0d, 0x0c, 0x0b, 0x0a, 0x02, 0x01]
        );
        assert_eq!(wire.len(), GC_CHANGE_SPEED_WIRE_SIZE);
        assert_eq!(GcChangeSpeed::decode(&wire), Ok(record));
    }

    #[test]
    fn the_change_speed_record_keeps_the_full_unsigned_word_range() {
        // The legacy field is a WORD and the client scales it, so 0xffff must
        // survive as 65535 rather than wrapping to -1.
        let record = GcChangeSpeed {
            vid: 0,
            moving_speed: u16::MAX,
        };
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(GcChangeSpeed::decode(&wire).unwrap().moving_speed, 65535);
    }

    #[test]
    fn the_change_speed_record_rejects_a_wrong_header_and_a_short_buffer() {
        assert!(matches!(
            GcChangeSpeed::decode(&[HEADER_GC_SEPCIAL_EFFECT, 0, 0, 0, 0, 0, 0]),
            Err(GcSmallError::Header {
                expected: HEADER_GC_CHANGE_SPEED,
                actual: HEADER_GC_SEPCIAL_EFFECT,
                ..
            })
        ));
        assert_eq!(
            GcChangeSpeed::decode(&[HEADER_GC_CHANGE_SPEED; 6]),
            Err(GcSmallError::Truncated {
                context: "GcChangeSpeed",
                needed: 7,
                actual: 6,
            })
        );
    }

    #[test]
    fn the_event_kw_score_record_is_seven_bytes_in_kingdom_order() {
        let record = GcEventKwScore::new([0x0102, 0x0304, 0x0506]);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire, vec![0x9d, 0x02, 0x01, 0x04, 0x03, 0x06, 0x05]);
        assert_eq!(wire.len(), GC_EVENT_KW_SCORE_WIRE_SIZE);
        assert_eq!(GcEventKwScore::decode(&wire), Ok(record));
    }

    #[test]
    fn the_event_kw_score_record_keeps_the_kingdom_index_of_each_word() {
        let record = GcEventKwScore::new([0x1111, 0x2222, 0x3333]);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        let decoded = GcEventKwScore::decode(&wire).unwrap();
        assert_eq!(decoded.kingdom_scores, [0x1111, 0x2222, 0x3333]);
        assert_eq!(decoded.kingdom_scores[0], 0x1111);
        assert_eq!(decoded.kingdom_scores[2], 0x3333);
    }

    #[test]
    fn the_event_kw_score_record_rejects_a_wrong_header() {
        assert!(matches!(
            GcEventKwScore::decode(&[HEADER_GC_FISHING, 0, 0, 0, 0, 0, 0]),
            Err(GcSmallError::Header {
                expected: HEADER_GC_EVENT_KW_SCORE,
                actual: HEADER_GC_FISHING,
                ..
            })
        ));
    }

    #[test]
    fn the_dragon_soul_refine_record_nests_the_shared_three_byte_slot() {
        let record = GcDragonSoulRefine {
            sub_type: 0x02,
            pos: ItemPos::new(0x41, 0x0203),
        };
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire.len(), GC_DRAGON_SOUL_REFINE_WIRE_SIZE);
        assert_eq!(wire[0], HEADER_GC_DRAGON_SOUL_REFINE);
        assert_eq!(wire[1], 0x02);
        assert_eq!(
            &wire[2..],
            &[0x41, 0x03, 0x02],
            "the slot keeps its own order"
        );
        assert_eq!(GcDragonSoulRefine::decode(&wire), Ok(record));
    }

    #[test]
    fn the_dragon_soul_refine_record_accepts_its_minimum_length_and_any_slot_value() {
        // The nested ItemPos error arm is unreachable because ItemPos::decode
        // rejects only a wrong length and the parent checks the length first.
        // Pinned here so that fact is verified rather than assumed: if ItemPos
        // ever gains value validation this test is the one that must change.
        for slot in [[0u8, 0, 0], [0x41, 0, 0], [0xff, 0xff, 0xff]] {
            let wire = [
                HEADER_GC_DRAGON_SOUL_REFINE,
                0x02,
                slot[0],
                slot[1],
                slot[2],
            ];
            let decoded = GcDragonSoulRefine::decode(&wire).unwrap();
            assert_eq!(decoded.sub_type, 0x02);
            assert_eq!(ItemPos::decode(&slot), Ok(decoded.pos));
        }
    }

    #[test]
    fn a_bad_nested_slot_is_surfaced_as_data_rather_than_a_panic() {
        // Direct exercise of the mapping that GcDragonSoulRefine::decode uses.
        let source = ItemPos::decode(&[0u8, 0]).unwrap_err();
        let err = GcSmallError::ItemPos {
            context: "GcDragonSoulRefine",
            source,
        };
        assert!(err.to_string().contains("nested grid position is invalid"));
        assert!(err.to_string().contains("GcDragonSoulRefine"));
    }

    #[test]
    fn the_dragon_soul_refine_record_rejects_a_wrong_header() {
        let wire = [HEADER_GC_CHANGE_SPEED, 0, 0x41, 0x00, 0x00];
        assert!(matches!(
            GcDragonSoulRefine::decode(&wire),
            Err(GcSmallError::Header {
                expected: HEADER_GC_DRAGON_SOUL_REFINE,
                actual: HEADER_GC_CHANGE_SPEED,
                ..
            })
        ));
    }

    #[test]
    fn every_dragon_soul_refine_window_type_and_cell_round_trips() {
        for window_type in 0..=u8::MAX {
            for cell in [0u16, 1, 0x00ff, 0x0100, u16::MAX] {
                let record = GcDragonSoulRefine {
                    sub_type: window_type,
                    pos: ItemPos::new(window_type, cell),
                };
                let mut wire = Vec::new();
                record.encode_into(&mut wire);
                assert_eq!(GcDragonSoulRefine::decode(&wire), Ok(record));
            }
        }
    }

    #[test]
    fn the_fishing_subheader_values_are_the_enumerator_positions() {
        let expected = [
            (GcFishingSubheader::Start, 0u8),
            (GcFishingSubheader::Stop, 1),
            (GcFishingSubheader::React, 2),
            (GcFishingSubheader::Success, 3),
            (GcFishingSubheader::Fail, 4),
            (GcFishingSubheader::Fish, 5),
        ];
        for (variant, byte) in expected {
            assert_eq!(variant.value(), byte);
            assert_eq!(GcFishingSubheader::from_value(byte), variant);
            assert!(variant.is_known());
        }
    }

    #[test]
    fn an_undefined_fishing_subheader_is_preserved_rather_than_rejected() {
        for raw in [6u8, 7, 100, 255] {
            let variant = GcFishingSubheader::from_value(raw);
            assert_eq!(variant, GcFishingSubheader::Unknown(raw));
            assert_eq!(variant.value(), raw);
            assert!(!variant.is_known());
        }
    }

    #[test]
    fn the_fishing_record_is_seven_bytes_for_every_subheader() {
        for variant in [
            GcFishingSubheader::Start,
            GcFishingSubheader::Stop,
            GcFishingSubheader::React,
            GcFishingSubheader::Success,
            GcFishingSubheader::Fail,
            GcFishingSubheader::Fish,
            GcFishingSubheader::Unknown(200),
        ] {
            let record = GcFishing {
                subheader: variant,
                info: 0x0a0b_0c0d,
                dir: 0xff,
            };
            let mut wire = Vec::new();
            record.encode_into(&mut wire);
            assert_eq!(wire.len(), GC_FISHING_WIRE_SIZE, "{variant:?}");
            assert_eq!(wire[0], HEADER_GC_FISHING);
            assert_eq!(wire[1], variant.value());
            assert_eq!(&wire[2..6], &[0x0d, 0x0c, 0x0b, 0x0a]);
            assert_eq!(wire[6], 0xff);
            assert_eq!(GcFishing::decode(&wire), Ok(record), "{variant:?}");
        }
    }

    #[test]
    fn the_fishing_info_field_has_two_meanings_depending_on_the_subheader() {
        // fishing.cpp writes GetVID() for five sub-headers and a fish vnum for
        // Fish. The accessors must not claim one meaning for both.
        let as_vid = GcFishing {
            subheader: GcFishingSubheader::React,
            info: 4242,
            dir: 1,
        };
        assert_eq!(as_vid.actor_vid(), Some(4242));
        assert_eq!(as_vid.fish_vnum(), None);

        let as_fish = GcFishing {
            subheader: GcFishingSubheader::Fish,
            info: 50008,
            dir: 0,
        };
        assert_eq!(as_fish.actor_vid(), None);
        assert_eq!(as_fish.fish_vnum(), Some(50008));

        let unknown = GcFishing {
            subheader: GcFishingSubheader::Unknown(9),
            info: 1,
            dir: 0,
        };
        assert_eq!(unknown.actor_vid(), None);
        assert_eq!(unknown.fish_vnum(), None);
    }

    #[test]
    fn the_fishing_direction_stays_a_raw_byte() {
        // The client computes float(dir) * 5.0f. A Rust i8 would reinterpret the
        // exact same byte differently, so the byte is kept verbatim.
        for dir in [0u8, 1, 127, 128, 200, 255] {
            let record = GcFishing {
                subheader: GcFishingSubheader::Start,
                info: 1,
                dir,
            };
            let mut wire = Vec::new();
            record.encode_into(&mut wire);
            assert_eq!(GcFishing::decode(&wire).unwrap().dir, dir);
        }
    }

    #[test]
    fn the_fishing_record_rejects_a_wrong_header_and_a_short_buffer() {
        assert!(matches!(
            GcFishing::decode(&[HEADER_GC_SEPCIAL_EFFECT, 0, 0, 0, 0, 0, 0]),
            Err(GcSmallError::Header {
                expected: HEADER_GC_FISHING,
                actual: HEADER_GC_SEPCIAL_EFFECT,
                ..
            })
        ));
        assert_eq!(
            GcFishing::decode(&[HEADER_GC_FISHING; 6]),
            Err(GcSmallError::Truncated {
                context: "GcFishing",
                needed: 7,
                actual: 6,
            })
        );
    }

    #[test]
    fn every_record_ignores_bytes_past_its_own_width() {
        let mut buf = Vec::new();
        GcHeaderAndDword::new(HEADER_GC_TIME, 1).encode_into(&mut buf);
        GcHeaderAndDwordAndByte::new(HEADER_GC_WALK_MODE, 2, 3).encode_into(&mut buf);
        GcSpecialEffect {
            effect_type: 4,
            vid: 5,
        }
        .encode_into(&mut buf);
        GcFishing {
            subheader: GcFishingSubheader::Fail,
            info: 6,
            dir: 7,
        }
        .encode_into(&mut buf);

        let at = |n: usize| buf.get(n).copied();
        let _ = at;
        assert_eq!(
            GcHeaderAndDword::decode(&buf).unwrap().header,
            HEADER_GC_TIME
        );
        let rest = &buf[GC_HEADER_AND_DWORD_WIRE_SIZE..];
        assert_eq!(
            GcHeaderAndDwordAndByte::decode(rest).unwrap(),
            GcHeaderAndDwordAndByte::new(HEADER_GC_WALK_MODE, 2, 3)
        );
        let rest = &rest[GC_HEADER_AND_DWORD_AND_BYTE_WIRE_SIZE..];
        assert_eq!(
            GcSpecialEffect::decode(rest).unwrap(),
            GcSpecialEffect {
                effect_type: 4,
                vid: 5
            }
        );
        let rest = &rest[GC_HEADER_AND_BYTE_AND_DWORD_WIRE_SIZE..];
        assert_eq!(
            GcFishing::decode(rest).unwrap().subheader,
            GcFishingSubheader::Fail
        );
    }

    #[test]
    fn the_error_display_names_the_record_and_the_numbers() {
        assert_eq!(
            GcSmallError::Truncated {
                context: "GcFishing",
                needed: 7,
                actual: 3,
            }
            .to_string(),
            "GcFishing: need 7 bytes, buffer held 3"
        );
        assert_eq!(
            GcSmallError::Header {
                context: "GcChangeSpeed",
                expected: 18,
                actual: 114,
            }
            .to_string(),
            "GcChangeSpeed: header 0x72, expected 0x12"
        );
    }

    #[test]
    fn the_error_type_is_a_standard_error() {
        fn assert_error<E: std::error::Error>(_: &E) {}
        assert_error(&GcSmallError::Truncated {
            context: "x",
            needed: 1,
            actual: 0,
        });
    }
}
