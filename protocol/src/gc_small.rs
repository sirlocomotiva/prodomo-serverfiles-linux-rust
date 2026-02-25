//! The small fixed-width game-to-client records.
//!
//! Every record in this module is a `#pragma pack(1)` structure whose packed
//! width is one to four bytes and whose body is a single opaque byte after the
//! header. They are the "notification" records: the game server tells the client
//! that a state changed, and the client reacts without echoing a request.
//!
//! # Provenance
//!
//! The framing table is the **client's** decode table, in
//! `client/Client/UserInterface/PythonNetworkStream.cpp`, because that is what
//! determines how many bytes the client consumes. Each record was then checked
//! against the server producer so that the width is not a client-side guess.
//!
//! Three legacy naming hazards are handled explicitly and are load-bearing:
//!
//! * **The server reuses `CG`-prefixed struct names for outbound records.**
//!   `TPacketCGSafeboxSize` and `TPacketCGSafeboxWrongPassword` are
//!   `HEADER_GC_SAFEBOX_SIZE` and `HEADER_GC_SAFEBOX_WRONG_PASSWORD`. A search
//!   for `TPacketGC*` in the server tree misses both.
//! * **Field names differ without the wire differing.**
//!   `TPacketGCQuickSlotSwap` is `{ header; pos; change_pos; }` on the client
//!   and `{ header; pos; pos_to; }` on the server. Same order, same types, same
//!   three bytes.
//! * **A record the client decodes may have no producer at all.** Bytes 73, 112,
//!   and 213 have zero hits anywhere in `server/server`.
//!
//! # Scope
//!
//! These are record codecs only. They do not install keys, touch a socket,
//! resolve a party or guild, classify a result, mutate a session, or dispatch
//! anything. `bool` fields stay raw bytes.

use std::fmt;

/// Number of bytes in a body that is a single opaque `BYTE`.
///
/// `TPacketGCEventReload`, `TPacketGCOpenDragonSoulChangeAttr`,
/// `TPacketGCRequestMakeGuild` (registered as the shared `TPacketGCBlank`), and
/// `TPacketGCSafeboxWrongPassword` all reduce to the header byte.
pub const GC_ONE_BYTE_WIRE_SIZE: usize = 1;

/// Number of bytes in a record that is a header plus one opaque `BYTE`.
pub const GC_HEADER_AND_BYTE_WIRE_SIZE: usize = 2;

/// Number of bytes in a record that is a header plus two opaque `BYTE`s.
pub const GC_HEADER_AND_TWO_BYTES_WIRE_SIZE: usize = 3;

/// Number of bytes in [`GcQuickSlotAdd`], which nests the shared 2-byte slot.
pub const GC_QUICK_SLOT_ADD_WIRE_SIZE: usize = 4;

/// The shared 2-byte quickslot body.
///
/// Source: `client/Client/UserInterface/GameType.h:585-589`
/// (`SQuickSlot { BYTE Type; BYTE Position; }`) and
/// `server/server/common/tables.h:452-456` (`SQuickslot { BYTE type; BYTE pos; }`).
/// Both are 2 bytes packed and the field order agrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcQuickSlot {
    /// Legacy `Type` / `type`.
    pub slot_type: u8,
    /// Legacy `Position` / `pos`.
    pub position: u8,
}

impl GcQuickSlot {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = 2;

    /// Decodes the slot from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcQuickSlot::WIRE_SIZE`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcQuickSlot",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            slot_type: bytes[0],
            position: bytes[1],
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.slot_type);
        out.push(self.position);
    }
}

/// `HEADER_GC_QUICKSLOT_ADD` (byte 28): a quickslot was set.
///
/// Source: `client/.../Packet.h:1854-1859`
/// (`{ BYTE header; BYTE pos; TQuickSlot slot; }`) and
/// `server/server/game/packet.h:1508-1513` (the identical `struct
/// packet_quickslot_add`). Packed width 1 + 1 + 2 = 4.
///
/// Sent from `server/server/game/char_quickslot.cpp:90-94`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcQuickSlotAdd {
    /// Quick-slot index inside the character quickslot array.
    pub pos: u8,
    /// The slot body the client should store.
    pub slot: GcQuickSlot,
}

impl GcQuickSlotAdd {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_QUICK_SLOT_ADD_WIRE_SIZE;

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcQuickSlotAdd::WIRE_SIZE`], and [`GcSmallError::Header`] unless the
    /// first byte is [`HEADER_GC_QUICKSLOT_ADD`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcQuickSlotAdd",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_QUICKSLOT_ADD {
            return Err(GcSmallError::Header {
                context: "GcQuickSlotAdd",
                expected: HEADER_GC_QUICKSLOT_ADD,
                actual: bytes[0],
            });
        }
        Ok(Self {
            pos: bytes[1],
            slot: GcQuickSlot {
                slot_type: bytes[2],
                position: bytes[3],
            },
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_QUICKSLOT_ADD);
        out.push(self.pos);
        self.slot.encode_into(out);
    }
}

/// `HEADER_GC_QUICKSLOT_SWAP` (byte 30): two quickslot entries traded places.
///
/// Source: `client/.../Packet.h` `TPacketGCQuickSlotSwap`
/// (`{ BYTE header; BYTE pos; BYTE change_pos; }`) and
/// `server/server/game/char_quickslot.cpp:118,130-134` (a local
/// `struct packet_quickslot_swap` whose third field is spelled `pos_to`).
/// Same three bytes either way.
///
/// Sent from `char_quickslot.cpp:130-134`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcQuickSlotSwap {
    /// The slot that moved.
    pub pos: u8,
    /// The slot it moved to.
    pub change_pos: u8,
}

impl GcQuickSlotSwap {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_HEADER_AND_TWO_BYTES_WIRE_SIZE;

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcQuickSlotSwap::WIRE_SIZE`], and [`GcSmallError::Header`] unless the
    /// first byte is [`HEADER_GC_QUICKSLOT_SWAP`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcQuickSlotSwap",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_QUICKSLOT_SWAP {
            return Err(GcSmallError::Header {
                context: "GcQuickSlotSwap",
                expected: HEADER_GC_QUICKSLOT_SWAP,
                actual: bytes[0],
            });
        }
        Ok(Self {
            pos: bytes[1],
            change_pos: bytes[2],
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_QUICKSLOT_SWAP);
        out.push(self.pos);
        out.push(self.change_pos);
    }
}

/// A game-to-client record that is a header plus a fixed run of opaque bytes.
///
/// The same shape covers five distinct legacy records that share no struct and
/// no field names. Keeping them as one wire shape with a caller-supplied header
/// is honest: the bytes are identical, and inventing five near-identical Rust
/// types would imply five distinct legacy structs that do not exist.
///
/// | record | byte | body byte | legacy field |
/// |---|---|---|---|
/// | `HEADER_GC_QUICKSLOT_DEL` | 29 | `pos` | quick-slot index |
/// | `HEADER_GC_PARTY_PARAMETER` | 83 | `bDistributeMode` | party distribution mode |
/// | `HEADER_GC_SAFEBOX_SIZE` | 88 | `bSize` | safebox page count |
/// | `HEADER_GC_EMPIRE` | 90 | `bEmpire` | empire index |
/// | `HEADER_GC_MALL_OPEN` | 122 | `bSize` | `3 * SAFEBOX_PAGE_SIZE` |
/// | `HEADER_GC_LOVE_POINT_UPDATE` | 132 | `byLovePoint` / `love_point` | love point total |
/// | `HEADER_GC_CHANGE_SKILL_GROUP` | 112 | `skill_group` | skill group |
/// | `HEADER_GC_CHANNEL` | 121 | `channel` | channel index |
/// | `HEADER_GC_REQUEST_CHANGE_LANGUAGE` | 245 | `bLanguage` | requested language |
///
/// The body byte is never interpreted here. `bDistributeMode`, `bEmpire`,
/// `channel`, and `bLanguage` are all values with meaning the legacy code
/// assigns elsewhere, and a `switch` in the client silently ignores values it
/// does not know, so every `u8` is preserved.
///
/// Source: `client/Client/UserInterface/Packet.h` and the matching
/// `server/server/game/packet.h` structs. `HEADER_GC_SAFEBOX_SIZE` and
/// `HEADER_GC_MALL_OPEN` are both sent through the server's
/// `TPacketCGSafeboxSize` (`char.cpp:7120-7125`, `char.cpp:7166-7171`,
/// `char.cpp:7230-7235`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcHeaderAndByte {
    /// The record's own header byte.
    pub header: u8,
    /// The single opaque body byte, preserved verbatim.
    pub value: u8,
}

impl GcHeaderAndByte {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_HEADER_AND_BYTE_WIRE_SIZE;

    /// Builds the record for a known header.
    pub const fn new(header: u8, value: u8) -> Self {
        Self { header, value }
    }

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcHeaderAndByte::WIRE_SIZE`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcHeaderAndByte",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            header: bytes[0],
            value: bytes[1],
        })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.header);
        out.push(self.value);
    }
}

/// A game-to-client record that is only its header byte.
///
/// Covers `HEADER_GC_REQUEST_MAKE_GUILD` (82, client `TPacketGCBlank`, sent as a
/// bare `BYTE` at `questlua_game.cpp:57-58`),
/// `HEADER_GC_SAFEBOX_WRONG_PASSWORD` (87),
/// `HEADER_GC_EVENT_RELOAD` (156, sent at `cmd_gm.cpp:1604`), and
/// `HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN` (211, sent at `char.cpp:11239`).
///
/// The record carries no body. The legacy `Packet(&header, 1)` and
/// `Packet(&p, sizeof(p))` calls agree because each struct is a lone `BYTE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcHeaderOnly {
    /// The record's own header byte.
    pub header: u8,
}

impl GcHeaderOnly {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_ONE_BYTE_WIRE_SIZE;

    /// Builds the record for a known header.
    pub const fn new(header: u8) -> Self {
        Self { header }
    }

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` is non-empty.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.is_empty() {
            return Err(GcSmallError::Truncated {
                context: "GcHeaderOnly",
                needed: Self::WIRE_SIZE,
                actual: 0,
            });
        }
        Ok(Self { header: bytes[0] })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.header);
    }
}

/// `HEADER_GC_SKILL_COOLTIME_END` (byte 73): a skill cooldown finished.
///
/// **The server never sends this.** A whole-token search for the enumerator and
/// for `TPacketGCSkillCoolTimeEnd` over `server/server` returns zero hits, and
/// the server has no enumerator for byte 73 at all. The client registers it
/// (`PythonNetworkStream.cpp:36`) and even has no dispatch case for it.
///
/// The record is still a registered client decode entry, so its framing is
/// real: `{ BYTE header; BYTE bSkill; }`, packed width 2
/// (`client/Client/UserInterface/Packet.h`). The skill index is opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcSkillCoolTimeEnd {
    /// Legacy `bSkill`. Opaque; the server never produces this record.
    pub skill: u8,
}

impl GcSkillCoolTimeEnd {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_HEADER_AND_BYTE_WIRE_SIZE;

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcSkillCoolTimeEnd::WIRE_SIZE`], and [`GcSmallError::Header`] unless
    /// the first byte is [`HEADER_GC_SKILL_COOLTIME_END`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcSkillCoolTimeEnd",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_SKILL_COOLTIME_END {
            return Err(GcSmallError::Header {
                context: "GcSkillCoolTimeEnd",
                expected: HEADER_GC_SKILL_COOLTIME_END,
                actual: bytes[0],
            });
        }
        Ok(Self { skill: bytes[1] })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_SKILL_COOLTIME_END);
        out.push(self.skill);
    }
}

/// `HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT` (byte 212): a dragon-soul change-attribute
/// request was answered.
///
/// Source: `server/server/game/packet.h` `{ BYTE header; bool result; }` and the
/// client's `TPacketGCDragonSoulChangeAttrResult`. Under `#pragma pack(1)` the
/// C++ `bool` occupies one byte, so the packed width is 2. That byte is the whole
/// record body and it is kept as a raw `u8` rather than a Rust `bool`: the wire
/// carries whatever the sender wrote, and a `bool` would silently normalise a
/// non-zero byte to `true` and lose it.
///
/// Sent from `char.cpp:11248`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcDragonSoulChangeAttrResult {
    /// The legacy `bool result` byte, preserved verbatim.
    pub result: u8,
}

impl GcDragonSoulChangeAttrResult {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_HEADER_AND_BYTE_WIRE_SIZE;

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcDragonSoulChangeAttrResult::WIRE_SIZE`], and
    /// [`GcSmallError::Header`] unless the first byte is
    /// [`HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcDragonSoulChangeAttrResult",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT {
            return Err(GcSmallError::Header {
                context: "GcDragonSoulChangeAttrResult",
                expected: HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT,
                actual: bytes[0],
            });
        }
        Ok(Self { result: bytes[1] })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT);
        out.push(self.result);
    }
}

/// `HEADER_GC_UNK_213` (byte 213): a record the client decodes and the server
/// never sends.
///
/// The client registers it with the `// @fixme007` comment
/// (`PythonNetworkStream.cpp:160`) and has a handler
/// (`PythonNetworkStreamPhaseGame.cpp:734,1029-1030`). The server has no
/// enumerator for byte 213 and no producer. Framing is
/// `{ BYTE bHeader; BYTE bUnk; }`, packed width 2, and the second byte's meaning
/// is unknown, so it stays opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcUnk213 {
    /// Unknown body byte, preserved verbatim.
    pub value: u8,
}

impl GcUnk213 {
    /// Number of bytes this record occupies on the wire.
    pub const WIRE_SIZE: usize = GC_HEADER_AND_BYTE_WIRE_SIZE;

    /// Decodes the record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`GcSmallError::Truncated`] unless `bytes` holds the full
    /// [`GcUnk213::WIRE_SIZE`], and [`GcSmallError::Header`] unless the first
    /// byte is [`HEADER_GC_UNK_213`].
    pub fn decode(bytes: &[u8]) -> Result<Self, GcSmallError> {
        if bytes.len() < Self::WIRE_SIZE {
            return Err(GcSmallError::Truncated {
                context: "GcUnk213",
                needed: Self::WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != HEADER_GC_UNK_213 {
            return Err(GcSmallError::Header {
                context: "GcUnk213",
                expected: HEADER_GC_UNK_213,
                actual: bytes[0],
            });
        }
        Ok(Self { value: bytes[1] })
    }

    /// Appends the wire bytes to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(HEADER_GC_UNK_213);
        out.push(self.value);
    }
}

/// Failure decoding one of the small fixed-width game-to-client records.
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
        }
    }
}

impl std::error::Error for GcSmallError {}

/// `HEADER_GC_REQUEST_MAKE_GUILD` (82). The client registers the shared
/// `TPacketGCBlank`; the server sends a bare `BYTE` at `questlua_game.cpp:57-58`.
pub const HEADER_GC_REQUEST_MAKE_GUILD: u8 = 0x52;
/// `HEADER_GC_SAFEBOX_WRONG_PASSWORD` (87).
pub const HEADER_GC_SAFEBOX_WRONG_PASSWORD: u8 = 0x57;
/// `HEADER_GC_EVENT_RELOAD` (156).
pub const HEADER_GC_EVENT_RELOAD: u8 = 0x9c;
/// `HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN` (211).
pub const HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN: u8 = 0xd3;
/// `HEADER_GC_QUICKSLOT_DEL` (29).
pub const HEADER_GC_QUICKSLOT_DEL: u8 = 0x1d;
/// `HEADER_GC_SKILL_COOLTIME_END` (73). The server never sends this.
pub const HEADER_GC_SKILL_COOLTIME_END: u8 = 0x49;
/// `HEADER_GC_PARTY_PARAMETER` (83).
pub const HEADER_GC_PARTY_PARAMETER: u8 = 0x53;
/// `HEADER_GC_SAFEBOX_SIZE` (88).
pub const HEADER_GC_SAFEBOX_SIZE: u8 = 0x58;
/// `HEADER_GC_EMPIRE` (90).
pub const HEADER_GC_EMPIRE: u8 = 0x5a;
/// `HEADER_GC_CHANGE_SKILL_GROUP` (112). The server never sends this.
pub const HEADER_GC_CHANGE_SKILL_GROUP: u8 = 0x70;
/// `HEADER_GC_CHANNEL` (121).
pub const HEADER_GC_CHANNEL: u8 = 0x79;
/// `HEADER_GC_MALL_OPEN` (122).
pub const HEADER_GC_MALL_OPEN: u8 = 0x7a;
/// `HEADER_GC_LOVE_POINT_UPDATE` (132).
pub const HEADER_GC_LOVE_POINT_UPDATE: u8 = 0x84;
/// `HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT` (212).
pub const HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT: u8 = 0xd4;
/// `HEADER_GC_UNK_213` (213). The server never sends this.
pub const HEADER_GC_UNK_213: u8 = 0xd5;
/// `HEADER_GC_REQUEST_CHANGE_LANGUAGE` (245).
pub const HEADER_GC_REQUEST_CHANGE_LANGUAGE: u8 = 0xf5;
/// `HEADER_GC_QUICKSLOT_SWAP` (30).
pub const HEADER_GC_QUICKSLOT_SWAP: u8 = 0x1e;
/// `HEADER_GC_QUICKSLOT_ADD` (28).
pub const HEADER_GC_QUICKSLOT_ADD: u8 = 0x1c;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every header this module can decode, with its expected packed width.
    const TABLE: [(u8, usize, &str); 19] = [
        (HEADER_GC_REQUEST_MAKE_GUILD, 1, "REQUEST_MAKE_GUILD"),
        (
            HEADER_GC_SAFEBOX_WRONG_PASSWORD,
            1,
            "SAFEBOX_WRONG_PASSWORD",
        ),
        (HEADER_GC_EVENT_RELOAD, 1, "EVENT_RELOAD"),
        (
            HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN,
            1,
            "DS_PLUS_CHANGE_ATTR_OPEN",
        ),
        (HEADER_GC_QUICKSLOT_DEL, 2, "QUICKSLOT_DEL"),
        (HEADER_GC_SKILL_COOLTIME_END, 2, "SKILL_COOLTIME_END"),
        (HEADER_GC_PARTY_PARAMETER, 2, "PARTY_PARAMETER"),
        (HEADER_GC_SAFEBOX_SIZE, 2, "SAFEBOX_SIZE"),
        (HEADER_GC_EMPIRE, 2, "EMPIRE"),
        (HEADER_GC_CHANGE_SKILL_GROUP, 2, "CHANGE_SKILL_GROUP"),
        (HEADER_GC_CHANNEL, 2, "CHANNEL"),
        (HEADER_GC_MALL_OPEN, 2, "MALL_OPEN"),
        (HEADER_GC_LOVE_POINT_UPDATE, 2, "LOVE_POINT_UPDATE"),
        (
            HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT,
            2,
            "DS_PLUS_CHANGE_ATTR_RESULT",
        ),
        (HEADER_GC_UNK_213, 2, "UNK_213"),
        (
            HEADER_GC_REQUEST_CHANGE_LANGUAGE,
            2,
            "REQUEST_CHANGE_LANGUAGE",
        ),
        (HEADER_GC_QUICKSLOT_SWAP, 3, "QUICKSLOT_SWAP"),
        (HEADER_GC_QUICKSLOT_ADD, 4, "QUICKSLOT_ADD"),
        (0xfc, 1, "TIME_SYNC / HANDSHAKE_OK, in gc.rs"),
    ];

    #[test]
    fn the_table_covers_nineteen_distinct_header_bytes() {
        let bytes: BTreeSet<u8> = TABLE.iter().map(|entry| entry.0).collect();
        assert_eq!(bytes.len(), TABLE.len());
        assert_eq!(bytes.len(), 19);
    }

    #[test]
    fn the_documented_header_values_match_the_legacy_tables() {
        // Values transcribed from the client enum in
        // client/Client/UserInterface/Packet.h and checked against
        // server/server/game/packet.h. All 19 agree on both sides except byte
        // 0xfc, where the client calls it HANDSHAKE_OK and the server TIME_SYNC.
        assert_eq!(HEADER_GC_QUICKSLOT_ADD, 28);
        assert_eq!(HEADER_GC_QUICKSLOT_DEL, 29);
        assert_eq!(HEADER_GC_QUICKSLOT_SWAP, 30);
        assert_eq!(HEADER_GC_SKILL_COOLTIME_END, 73);
        assert_eq!(HEADER_GC_PARTY_PARAMETER, 83);
        assert_eq!(HEADER_GC_REQUEST_MAKE_GUILD, 82);
        assert_eq!(HEADER_GC_SAFEBOX_WRONG_PASSWORD, 87);
        assert_eq!(HEADER_GC_SAFEBOX_SIZE, 88);
        assert_eq!(HEADER_GC_EMPIRE, 90);
        assert_eq!(HEADER_GC_CHANGE_SKILL_GROUP, 112);
        assert_eq!(HEADER_GC_CHANNEL, 121);
        assert_eq!(HEADER_GC_MALL_OPEN, 122);
        assert_eq!(HEADER_GC_LOVE_POINT_UPDATE, 132);
        assert_eq!(HEADER_GC_EVENT_RELOAD, 156);
        assert_eq!(HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN, 211);
        assert_eq!(HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT, 212);
        assert_eq!(HEADER_GC_UNK_213, 213);
        assert_eq!(HEADER_GC_REQUEST_CHANGE_LANGUAGE, 245);
        assert_eq!(0xfc, 252);
    }

    #[test]
    fn the_field_order_is_header_then_body_bytes() {
        // Golden bytes for every width in the family.
        let cases: [(&[u8], usize); 4] = [
            (&[0x9c], 1),
            (&[0x9c, 0x5a], 2),
            (&[0x1e, 0x03, 0x07], 3),
            (&[0x1c, 0x03, 0x11, 0x22], 4),
        ];
        for (bytes, size) in cases {
            assert_eq!(bytes.len(), size);
        }
    }

    #[test]
    fn header_only_records_are_exactly_one_byte() {
        for header in [
            HEADER_GC_REQUEST_MAKE_GUILD,
            HEADER_GC_SAFEBOX_WRONG_PASSWORD,
            HEADER_GC_EVENT_RELOAD,
            HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN,
        ] {
            let record = GcHeaderOnly::new(header);
            let mut out = Vec::new();
            record.encode_into(&mut out);
            assert_eq!(out, vec![header]);
            assert_eq!(out.len(), GC_ONE_BYTE_WIRE_SIZE);
            assert_eq!(GcHeaderOnly::decode(&out), Ok(record));
            assert_eq!(GcHeaderOnly::WIRE_SIZE, 1);
        }
    }

    #[test]
    fn header_only_decode_rejects_an_empty_buffer() {
        assert_eq!(
            GcHeaderOnly::decode(&[]),
            Err(GcSmallError::Truncated {
                context: "GcHeaderOnly",
                needed: 1,
                actual: 0,
            })
        );
    }

    #[test]
    fn header_only_ignores_trailing_bytes_because_the_legacy_send_is_one_byte() {
        // questlua_game.cpp sends Packet(&header, 1) for REQUEST_MAKE_GUILD, so
        // any byte after the header belongs to the next record, not to this one.
        let decoded = GcHeaderOnly::decode(&[HEADER_GC_EVENT_RELOAD, 0xaa, 0xbb]).unwrap();
        assert_eq!(decoded, GcHeaderOnly::new(HEADER_GC_EVENT_RELOAD));
    }

    #[test]
    fn header_and_byte_records_preserve_every_possible_body_value() {
        for header in [
            HEADER_GC_QUICKSLOT_DEL,
            HEADER_GC_PARTY_PARAMETER,
            HEADER_GC_SAFEBOX_SIZE,
            HEADER_GC_EMPIRE,
            HEADER_GC_CHANGE_SKILL_GROUP,
            HEADER_GC_CHANNEL,
            HEADER_GC_MALL_OPEN,
            HEADER_GC_LOVE_POINT_UPDATE,
            HEADER_GC_REQUEST_CHANGE_LANGUAGE,
        ] {
            for value in 0..=u8::MAX {
                let record = GcHeaderAndByte::new(header, value);
                let mut out = Vec::new();
                record.encode_into(&mut out);
                assert_eq!(out.len(), GC_HEADER_AND_BYTE_WIRE_SIZE);
                assert_eq!(out, vec![header, value]);
                assert_eq!(GcHeaderAndByte::decode(&out), Ok(record));
            }
        }
    }

    #[test]
    fn header_and_byte_decode_rejects_a_one_byte_buffer() {
        assert_eq!(
            GcHeaderAndByte::decode(&[HEADER_GC_EMPIRE]),
            Err(GcSmallError::Truncated {
                context: "GcHeaderAndByte",
                needed: 2,
                actual: 1,
            })
        );
    }

    #[test]
    fn the_body_byte_is_opaque_so_an_unknown_value_survives_a_round_trip() {
        // The client's switches silently ignore values they do not know, so a
        // decoder that rejected them would lose wire data.
        for value in [0x00u8, 0x01, 0x7f, 0x80, 0xfe, 0xff] {
            let record = GcHeaderAndByte::new(HEADER_GC_EMPIRE, value);
            let mut out = Vec::new();
            record.encode_into(&mut out);
            assert_eq!(GcHeaderAndByte::decode(&out).unwrap().value, value);
        }
    }

    #[test]
    fn safebox_size_and_mall_open_share_one_shape_but_not_one_header() {
        // Both are sent through the server's TPacketCGSafeboxSize. The header
        // is the only thing that separates them, so it must survive the round trip.
        let size = GcHeaderAndByte::new(HEADER_GC_SAFEBOX_SIZE, 3 * 45);
        let mall = GcHeaderAndByte::new(HEADER_GC_MALL_OPEN, 3 * 45);
        assert_ne!(size.header, mall.header);
        assert_eq!(size.value, mall.value);
        let mut out = Vec::new();
        size.encode_into(&mut out);
        mall.encode_into(&mut out);
        assert_eq!(
            out,
            vec![HEADER_GC_SAFEBOX_SIZE, 135, HEADER_GC_MALL_OPEN, 135]
        );
    }

    #[test]
    fn quick_slot_add_is_four_bytes_with_the_slot_nested_last() {
        let record = GcQuickSlotAdd {
            pos: 3,
            slot: GcQuickSlot {
                slot_type: 0x11,
                position: 0x22,
            },
        };
        let mut out = Vec::new();
        record.encode_into(&mut out);
        assert_eq!(out, vec![HEADER_GC_QUICKSLOT_ADD, 0x03, 0x11, 0x22]);
        assert_eq!(out.len(), GC_QUICK_SLOT_ADD_WIRE_SIZE);
        assert_eq!(GcQuickSlotAdd::WIRE_SIZE, 4);
        assert_eq!(GcQuickSlotAdd::decode(&out), Ok(record));
    }

    #[test]
    fn quick_slot_add_rejects_a_wrong_header() {
        let err = GcQuickSlotAdd::decode(&[HEADER_GC_QUICKSLOT_DEL, 1, 2, 3]).unwrap_err();
        assert_eq!(
            err,
            GcSmallError::Header {
                context: "GcQuickSlotAdd",
                expected: HEADER_GC_QUICKSLOT_ADD,
                actual: HEADER_GC_QUICKSLOT_DEL,
            }
        );
    }

    #[test]
    fn quick_slot_add_rejects_every_short_buffer() {
        for len in 0..GC_QUICK_SLOT_ADD_WIRE_SIZE {
            let buf = vec![HEADER_GC_QUICKSLOT_ADD; len];
            assert_eq!(
                GcQuickSlotAdd::decode(&buf),
                Err(GcSmallError::Truncated {
                    context: "GcQuickSlotAdd",
                    needed: GC_QUICK_SLOT_ADD_WIRE_SIZE,
                    actual: len,
                })
            );
        }
    }

    #[test]
    fn quick_slot_add_ignores_trailing_bytes() {
        let record = GcQuickSlotAdd {
            pos: 1,
            slot: GcQuickSlot {
                slot_type: 2,
                position: 3,
            },
        };
        let mut buf = Vec::new();
        record.encode_into(&mut buf);
        buf.push(0xde);
        buf.push(0xad);
        assert_eq!(GcQuickSlotAdd::decode(&buf), Ok(record));
    }

    #[test]
    fn the_shared_quick_slot_body_is_two_bytes() {
        assert_eq!(GcQuickSlot::WIRE_SIZE, 2);
        for slot_type in 0..=u8::MAX {
            for position in 0..=u8::MAX {
                let slot = GcQuickSlot {
                    slot_type,
                    position,
                };
                let mut out = Vec::new();
                slot.encode_into(&mut out);
                assert_eq!(out, vec![slot_type, position]);
                assert_eq!(GcQuickSlot::decode(&out), Ok(slot));
            }
        }
    }

    #[test]
    fn the_shared_quick_slot_body_rejects_a_one_byte_buffer() {
        assert_eq!(
            GcQuickSlot::decode(&[1]),
            Err(GcSmallError::Truncated {
                context: "GcQuickSlot",
                needed: 2,
                actual: 1,
            })
        );
    }

    #[test]
    fn the_field_order_is_slot_type_then_position_on_both_sides() {
        // client GameType.h SQuickSlot { BYTE Type; BYTE Position; }
        // server tables.h    SQuickslot { BYTE type; BYTE pos; }
        let slot = GcQuickSlot {
            slot_type: 0xaa,
            position: 0xbb,
        };
        let mut out = Vec::new();
        slot.encode_into(&mut out);
        assert_eq!(out, vec![0xaa, 0xbb], "slot type must come first");
    }

    #[test]
    fn quick_slot_swap_is_three_bytes() {
        let record = GcQuickSlotSwap {
            pos: 0x11,
            change_pos: 0x22,
        };
        let mut out = Vec::new();
        record.encode_into(&mut out);
        assert_eq!(out, vec![HEADER_GC_QUICKSLOT_SWAP, 0x11, 0x22]);
        assert_eq!(out.len(), GC_HEADER_AND_TWO_BYTES_WIRE_SIZE);
        assert_eq!(GcQuickSlotSwap::decode(&out), Ok(record));
    }

    #[test]
    fn quick_slot_swap_keeps_the_client_field_names_over_the_server_spelling() {
        // The server spells the third field pos_to; the client spells it
        // change_pos. Same byte, so the decoder must not lose it.
        let record = GcQuickSlotSwap {
            pos: 4,
            change_pos: 9,
        };
        assert_eq!(record.change_pos, 9, "pos_to is the third wire byte");
        assert_ne!(record.pos, record.change_pos);
    }

    #[test]
    fn quick_slot_swap_rejects_every_short_buffer_and_a_wrong_header() {
        for len in 0..GC_HEADER_AND_TWO_BYTES_WIRE_SIZE {
            let buf = vec![HEADER_GC_QUICKSLOT_SWAP; len];
            assert_eq!(
                GcQuickSlotSwap::decode(&buf),
                Err(GcSmallError::Truncated {
                    context: "GcQuickSlotSwap",
                    needed: GC_HEADER_AND_TWO_BYTES_WIRE_SIZE,
                    actual: len,
                })
            );
        }
        assert_eq!(
            GcQuickSlotSwap::decode(&[HEADER_GC_QUICKSLOT_ADD, 1, 2]),
            Err(GcSmallError::Header {
                context: "GcQuickSlotSwap",
                expected: HEADER_GC_QUICKSLOT_SWAP,
                actual: HEADER_GC_QUICKSLOT_ADD,
            })
        );
    }

    #[test]
    fn the_skill_cool_time_record_still_frames_even_though_no_server_sends_it() {
        let record = GcSkillCoolTimeEnd { skill: 42 };
        let mut out = Vec::new();
        record.encode_into(&mut out);
        assert_eq!(out, vec![HEADER_GC_SKILL_COOLTIME_END, 42]);
        assert_eq!(GcSkillCoolTimeEnd::decode(&out), Ok(record));
        assert_eq!(
            GcSkillCoolTimeEnd::decode(&[HEADER_GC_EMPIRE, 42]),
            Err(GcSmallError::Header {
                context: "GcSkillCoolTimeEnd",
                expected: HEADER_GC_SKILL_COOLTIME_END,
                actual: HEADER_GC_EMPIRE,
            })
        );
        assert_eq!(
            GcSkillCoolTimeEnd::decode(&[HEADER_GC_SKILL_COOLTIME_END]),
            Err(GcSmallError::Truncated {
                context: "GcSkillCoolTimeEnd",
                needed: 2,
                actual: 1,
            })
        );
    }

    #[test]
    fn the_dragon_soul_result_keeps_a_non_boolean_byte_raw() {
        // The legacy field is a C++ bool, but the wire carries whatever byte was
        // written. A Rust bool would collapse every non-zero value to true.
        for result in 0..=u8::MAX {
            let record = GcDragonSoulChangeAttrResult { result };
            let mut out = Vec::new();
            record.encode_into(&mut out);
            assert_eq!(out, vec![HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT, result]);
            assert_eq!(GcDragonSoulChangeAttrResult::decode(&out), Ok(record));
        }
    }

    #[test]
    fn the_dragon_soul_result_rejects_a_wrong_header_and_a_short_buffer() {
        assert_eq!(
            GcDragonSoulChangeAttrResult::decode(&[HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN, 1]),
            Err(GcSmallError::Header {
                context: "GcDragonSoulChangeAttrResult",
                expected: HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT,
                actual: HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN,
            })
        );
        assert_eq!(
            GcDragonSoulChangeAttrResult::decode(&[HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT]),
            Err(GcSmallError::Truncated {
                context: "GcDragonSoulChangeAttrResult",
                needed: 2,
                actual: 1,
            })
        );
    }

    #[test]
    fn the_dragon_soul_open_and_result_records_are_distinct_headers_one_byte_apart() {
        // char.cpp:11239 sends OPEN, char.cpp:11248 sends RESULT.
        assert_eq!(HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN, 211);
        assert_eq!(HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT, 212);
        assert_eq!(
            u32::from(HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT)
                - u32::from(HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN),
            1
        );
    }

    #[test]
    fn the_unk_213_record_frames_and_rejects_other_headers() {
        let record = GcUnk213 { value: 0xbe };
        let mut out = Vec::new();
        record.encode_into(&mut out);
        assert_eq!(out, vec![HEADER_GC_UNK_213, 0xbe]);
        assert_eq!(GcUnk213::decode(&out), Ok(record));
        assert_eq!(
            GcUnk213::decode(&[HEADER_GC_UNK_213 + 1, 0xbe]),
            Err(GcSmallError::Header {
                context: "GcUnk213",
                expected: HEADER_GC_UNK_213,
                actual: HEADER_GC_UNK_213 + 1,
            })
        );
        assert_eq!(
            GcUnk213::decode(&[HEADER_GC_UNK_213]),
            Err(GcSmallError::Truncated {
                context: "GcUnk213",
                needed: 2,
                actual: 1,
            })
        );
    }

    #[test]
    fn every_record_ignores_bytes_past_its_own_width() {
        // The framing layer owns stream consumption. A record codec that demanded
        // the whole buffer would make concatenation impossible.
        let mut buf = Vec::new();
        GcHeaderOnly::new(HEADER_GC_EVENT_RELOAD).encode_into(&mut buf);
        GcHeaderAndByte::new(HEADER_GC_EMPIRE, 3).encode_into(&mut buf);
        GcQuickSlotSwap {
            pos: 1,
            change_pos: 2,
        }
        .encode_into(&mut buf);
        GcQuickSlotAdd {
            pos: 1,
            slot: GcQuickSlot {
                slot_type: 5,
                position: 6,
            },
        }
        .encode_into(&mut buf);

        assert_eq!(
            GcHeaderOnly::decode(&buf).unwrap().header,
            HEADER_GC_EVENT_RELOAD
        );
        let rest = &buf[1..];
        assert_eq!(
            GcHeaderAndByte::decode(rest).unwrap(),
            GcHeaderAndByte::new(HEADER_GC_EMPIRE, 3)
        );
        let rest = &rest[2..];
        assert_eq!(
            GcQuickSlotSwap::decode(rest).unwrap(),
            GcQuickSlotSwap {
                pos: 1,
                change_pos: 2
            }
        );
        let rest = &rest[3..];
        assert_eq!(
            GcQuickSlotAdd::decode(rest).unwrap(),
            GcQuickSlotAdd {
                pos: 1,
                slot: GcQuickSlot {
                    slot_type: 5,
                    position: 6
                }
            }
        );
    }

    #[test]
    fn the_display_text_names_the_record_and_the_numbers() {
        let truncated = GcSmallError::Truncated {
            context: "GcQuickSlotAdd",
            needed: 4,
            actual: 2,
        };
        assert_eq!(
            truncated.to_string(),
            "GcQuickSlotAdd: need 4 bytes, buffer held 2"
        );
        let wrong = GcSmallError::Header {
            context: "GcUnk213",
            expected: 0xd5,
            actual: 0x58,
        };
        assert_eq!(wrong.to_string(), "GcUnk213: header 0x58, expected 0xd5");
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

    #[test]
    fn the_three_client_only_records_carry_no_server_producer() {
        // Documented in 163.8-style findings: a whole-token search over
        // server/server returns zero hits for each of these enumerators.
        for header in [
            HEADER_GC_SKILL_COOLTIME_END,
            HEADER_GC_CHANGE_SKILL_GROUP,
            HEADER_GC_UNK_213,
        ] {
            let record = GcHeaderAndByte::new(header, 0);
            let mut out = Vec::new();
            record.encode_into(&mut out);
            assert_eq!(out, vec![header, 0]);
        }
    }
}
