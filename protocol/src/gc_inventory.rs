//! Machine-readable inventory of the legacy game-to-client packet headers.
//!
//! The registration set is the client decode table in
//! `client/Client/UserInterface/PythonNetworkStream.cpp`, which calls
//! `CNetworkPacketHeaderMap::Set` for 134 game-to-client records: 115 fixed
//! width and 19 variable length. The game server's own
//! `server/server/game/packet_info.cpp` registers only inbound client and
//! peer packets, so it is the wrong table for this direction and must not be
//! used to measure game-to-client coverage.
//!
//! Each row pairs a client registration name with the server enumerator name
//! for the same wire byte. The pairing is by byte value and never by name,
//! because the two trees renamed the same four record families in opposite
//! directions: the server suffixes the legacy generation with `_OLD` while the
//! client suffixes the modern generation with `_NEW` or a numeric suffix. A
//! name-keyed comparison of the two enums therefore reports four differences
//! that do not exist on the wire.
//!
//! The client's `Set` is last-wins, the opposite of the game server's
//! first-wins `CPacketInfo::Set`, but no byte is registered twice in this
//! table, so the two policies do not currently disagree.
//!
//! This module records header values, the legacy struct name, and whether the
//! client decodes a fixed or variable frame. It deliberately does not publish
//! a size column: the game-to-client widths are heavily feature gated, and an
//! automatically derived packed width was found to be wrong for records whose
//! constants are enum aliases. Widths belong with the individual codec
//! modules, where each is hand-verified against the struct source.
/// A one-byte legacy game-to-client packet header.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GcHeader(u8);

impl GcHeader {
    /// Return the header's one-byte wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

impl From<GcHeader> for u8 {
    fn from(header: GcHeader) -> Self {
        header.0
    }
}
/// `HEADER_GC_EMPIRE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_EMPIRE: GcHeader = GcHeader(0x5a);
/// `HEADER_GC_WARP` from the legacy game-to-client protocol tables.
pub const HEADER_GC_WARP: GcHeader = GcHeader(0x41);
/// `HEADER_GC_SKILL_COOLTIME_END` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SKILL_COOLTIME_END: GcHeader = GcHeader(0x49);
/// `HEADER_GC_QUEST_INFO` from the legacy game-to-client protocol tables.
pub const HEADER_GC_QUEST_INFO: GcHeader = GcHeader(0x51);
/// `HEADER_GC_REQUEST_MAKE_GUILD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_REQUEST_MAKE_GUILD: GcHeader = GcHeader(0x52);
/// `HEADER_GC_PVP` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PVP: GcHeader = GcHeader(0x29);
/// `HEADER_GC_DUEL_START` from the legacy game-to-client protocol tables.
pub const HEADER_GC_DUEL_START: GcHeader = GcHeader(0x28);
/// `HEADER_GC_CHARACTER_ADD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHARACTER_ADD: GcHeader = GcHeader(0x01);
/// `HEADER_GC_CHAR_ADDITIONAL_INFO` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHAR_ADDITIONAL_INFO: GcHeader = GcHeader(0x88);
/// `HEADER_GC_CHARACTER_ADD2` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHARACTER_ADD2: GcHeader = GcHeader(0x78);
/// `HEADER_GC_CHARACTER_UPDATE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHARACTER_UPDATE: GcHeader = GcHeader(0x13);
/// `HEADER_GC_CHARACTER_UPDATE2` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHARACTER_UPDATE2: GcHeader = GcHeader(0x75);
/// `HEADER_GC_CHARACTER_DEL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHARACTER_DEL: GcHeader = GcHeader(0x02);
/// `HEADER_GC_CHARACTER_MOVE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHARACTER_MOVE: GcHeader = GcHeader(0x03);
/// `HEADER_GC_CHAT` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHAT: GcHeader = GcHeader(0x04);
/// `HEADER_GC_SYNC_POSITION` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SYNC_POSITION: GcHeader = GcHeader(0x05);
/// `HEADER_GC_LOGIN_SUCCESS3` from the legacy game-to-client protocol tables.
pub const HEADER_GC_LOGIN_SUCCESS3: GcHeader = GcHeader(0x06);
/// `HEADER_GC_LOGIN_SUCCESS4` from the legacy game-to-client protocol tables.
pub const HEADER_GC_LOGIN_SUCCESS4: GcHeader = GcHeader(0x20);
/// `HEADER_GC_LOGIN_FAILURE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_LOGIN_FAILURE: GcHeader = GcHeader(0x07);
/// `HEADER_GC_AURA` from the legacy game-to-client protocol tables.
pub const HEADER_GC_AURA: GcHeader = GcHeader(0xd6);
/// `HEADER_GC_PICKUP_ITEM_SC` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PICKUP_ITEM_SC: GcHeader = GcHeader(0x40);
/// `HEADER_GC_PREMIUM_PLAYERS` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PREMIUM_PLAYERS: GcHeader = GcHeader(0x8d);
/// `HEADER_GC_BIOLOGIST` from the legacy game-to-client protocol tables.
pub const HEADER_GC_BIOLOGIST: GcHeader = GcHeader(0x8c);
/// `HEADER_GC_PRIVATE_SHOP` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PRIVATE_SHOP: GcHeader = GcHeader(0xaf);
/// `HEADER_GC_PLAYER_CREATE_SUCCESS` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PLAYER_CREATE_SUCCESS: GcHeader = GcHeader(0x08);
/// `HEADER_GC_PLAYER_CREATE_FAILURE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PLAYER_CREATE_FAILURE: GcHeader = GcHeader(0x09);
/// `HEADER_GC_PLAYER_DELETE_SUCCESS` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PLAYER_DELETE_SUCCESS: GcHeader = GcHeader(0x0a);
/// `HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID: GcHeader = GcHeader(0x0b);
/// `HEADER_GC_STUN` from the legacy game-to-client protocol tables.
pub const HEADER_GC_STUN: GcHeader = GcHeader(0x0d);
/// `HEADER_GC_DEAD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_DEAD: GcHeader = GcHeader(0x0e);
/// `HEADER_GC_MAIN_CHARACTER` from the legacy game-to-client protocol tables.
pub const HEADER_GC_MAIN_CHARACTER: GcHeader = GcHeader(0x0f);
/// `HEADER_GC_MAIN_CHARACTER2_EMPIRE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_MAIN_CHARACTER2_EMPIRE: GcHeader = GcHeader(0x71);
/// `HEADER_GC_MAIN_CHARACTER3_BGM` from the legacy game-to-client protocol tables.
pub const HEADER_GC_MAIN_CHARACTER3_BGM: GcHeader = GcHeader(0x89);
/// `HEADER_GC_MAIN_CHARACTER4_BGM_VOL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_MAIN_CHARACTER4_BGM_VOL: GcHeader = GcHeader(0x8a);
/// `HEADER_GC_PLAYER_POINTS` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PLAYER_POINTS: GcHeader = GcHeader(0x10);
/// `HEADER_GC_PLAYER_POINT_CHANGE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PLAYER_POINT_CHANGE: GcHeader = GcHeader(0x11);
/// `HEADER_GC_ITEM_SET` from the legacy game-to-client protocol tables.
pub const HEADER_GC_ITEM_SET: GcHeader = GcHeader(0x14);
/// `HEADER_GC_ITEM_SET2` from the legacy game-to-client protocol tables.
pub const HEADER_GC_ITEM_SET2: GcHeader = GcHeader(0x15);
/// `HEADER_GC_ITEM_USE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_ITEM_USE: GcHeader = GcHeader(0x16);
/// `HEADER_GC_ITEM_UPDATE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_ITEM_UPDATE: GcHeader = GcHeader(0x19);
/// `HEADER_GC_ITEM_GROUND_ADD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_ITEM_GROUND_ADD: GcHeader = GcHeader(0x1a);
/// `HEADER_GC_ITEM_GROUND_DEL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_ITEM_GROUND_DEL: GcHeader = GcHeader(0x1b);
/// `HEADER_GC_ITEM_OWNERSHIP` from the legacy game-to-client protocol tables.
pub const HEADER_GC_ITEM_OWNERSHIP: GcHeader = GcHeader(0x1f);
/// `HEADER_GC_QUICKSLOT_ADD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_QUICKSLOT_ADD: GcHeader = GcHeader(0x1c);
/// `HEADER_GC_QUICKSLOT_DEL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_QUICKSLOT_DEL: GcHeader = GcHeader(0x1d);
/// `HEADER_GC_QUICKSLOT_SWAP` from the legacy game-to-client protocol tables.
pub const HEADER_GC_QUICKSLOT_SWAP: GcHeader = GcHeader(0x1e);
/// `HEADER_GC_WHISPER` from the legacy game-to-client protocol tables.
pub const HEADER_GC_WHISPER: GcHeader = GcHeader(0x22);
/// `HEADER_GC_CHARACTER_POSITION` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHARACTER_POSITION: GcHeader = GcHeader(0x2b);
/// `HEADER_GC_MOTION` from the legacy game-to-client protocol tables.
pub const HEADER_GC_MOTION: GcHeader = GcHeader(0x24);
/// `HEADER_GC_SHOP` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SHOP: GcHeader = GcHeader(0x26);
/// `HEADER_GC_SHOP_SIGN` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SHOP_SIGN: GcHeader = GcHeader(0x27);
/// `HEADER_GC_EXCHANGE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_EXCHANGE: GcHeader = GcHeader(0x2a);
/// `HEADER_GC_PING` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PING: GcHeader = GcHeader(0x2c);
/// `HEADER_GC_SCRIPT` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SCRIPT: GcHeader = GcHeader(0x2d);
/// `HEADER_GC_QUEST_CONFIRM` from the legacy game-to-client protocol tables.
pub const HEADER_GC_QUEST_CONFIRM: GcHeader = GcHeader(0x2e);
/// `HEADER_GC_TARGET` from the legacy game-to-client protocol tables.
pub const HEADER_GC_TARGET: GcHeader = GcHeader(0x3f);
/// `HEADER_GC_TARGET_INFO` from the legacy game-to-client protocol tables.
pub const HEADER_GC_TARGET_INFO: GcHeader = GcHeader(0x3a);
/// `HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN` from the legacy game-to-client protocol tables.
pub const HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN: GcHeader = GcHeader(0xd3);
/// `HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT` from the legacy game-to-client protocol tables.
pub const HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT: GcHeader = GcHeader(0xd4);
/// `HEADER_GC_MOUNT` from the legacy game-to-client protocol tables.
pub const HEADER_GC_MOUNT: GcHeader = GcHeader(0x3d);
/// `HEADER_GC_CHANGE_SPEED` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHANGE_SPEED: GcHeader = GcHeader(0x12);
/// `HEADER_GC_HANDSHAKE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_HANDSHAKE: GcHeader = GcHeader(0xff);
/// `HEADER_GC_HANDSHAKE_OK` from the legacy game-to-client protocol tables.
pub const HEADER_GC_HANDSHAKE_OK: GcHeader = GcHeader(0xfc);
/// `HEADER_GC_BINDUDP` from the legacy game-to-client protocol tables.
pub const HEADER_GC_BINDUDP: GcHeader = GcHeader(0xfe);
/// `HEADER_GC_OWNERSHIP` from the legacy game-to-client protocol tables.
pub const HEADER_GC_OWNERSHIP: GcHeader = GcHeader(0x3e);
/// `HEADER_GC_CREATE_FLY` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CREATE_FLY: GcHeader = GcHeader(0x46);
/// `HEADER_GC_ADD_FLY_TARGETING` from the legacy game-to-client protocol tables.
pub const HEADER_GC_ADD_FLY_TARGETING: GcHeader = GcHeader(0x45);
/// `HEADER_GC_FLY_TARGETING` from the legacy game-to-client protocol tables.
pub const HEADER_GC_FLY_TARGETING: GcHeader = GcHeader(0x47);
/// `HEADER_GC_PHASE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PHASE: GcHeader = GcHeader(0xfd);
/// `HEADER_GC_SKILL_LEVEL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SKILL_LEVEL: GcHeader = GcHeader(0x48);
/// `HEADER_GC_SKILL_LEVEL_NEW` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SKILL_LEVEL_NEW: GcHeader = GcHeader(0x4c);
/// `HEADER_GC_MESSENGER` from the legacy game-to-client protocol tables.
pub const HEADER_GC_MESSENGER: GcHeader = GcHeader(0x4a);
/// `HEADER_GC_GUILD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_GUILD: GcHeader = GcHeader(0x4b);
/// `HEADER_GC_PARTY_INVITE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PARTY_INVITE: GcHeader = GcHeader(0x4d);
/// `HEADER_GC_PARTY_ADD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PARTY_ADD: GcHeader = GcHeader(0x4e);
/// `HEADER_GC_PARTY_UPDATE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PARTY_UPDATE: GcHeader = GcHeader(0x4f);
/// `HEADER_GC_PARTY_REMOVE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PARTY_REMOVE: GcHeader = GcHeader(0x50);
/// `HEADER_GC_PARTY_LINK` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PARTY_LINK: GcHeader = GcHeader(0x5b);
/// `HEADER_GC_PARTY_UNLINK` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PARTY_UNLINK: GcHeader = GcHeader(0x5c);
/// `HEADER_GC_PARTY_PARAMETER` from the legacy game-to-client protocol tables.
pub const HEADER_GC_PARTY_PARAMETER: GcHeader = GcHeader(0x53);
/// `HEADER_GC_SAFEBOX_SET` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SAFEBOX_SET: GcHeader = GcHeader(0x55);
/// `HEADER_GC_SAFEBOX_DEL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SAFEBOX_DEL: GcHeader = GcHeader(0x56);
/// `HEADER_GC_SAFEBOX_WRONG_PASSWORD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SAFEBOX_WRONG_PASSWORD: GcHeader = GcHeader(0x57);
/// `HEADER_GC_SAFEBOX_SIZE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SAFEBOX_SIZE: GcHeader = GcHeader(0x58);
/// `HEADER_GC_SAFEBOX_MONEY_CHANGE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SAFEBOX_MONEY_CHANGE: GcHeader = GcHeader(0x54);
/// `HEADER_GC_FISHING` from the legacy game-to-client protocol tables.
pub const HEADER_GC_FISHING: GcHeader = GcHeader(0x59);
/// `HEADER_GC_DUNGEON` from the legacy game-to-client protocol tables.
pub const HEADER_GC_DUNGEON: GcHeader = GcHeader(0x6e);
/// `HEADER_GC_TIME` from the legacy game-to-client protocol tables.
pub const HEADER_GC_TIME: GcHeader = GcHeader(0x6a);
/// `HEADER_GC_WALK_MODE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_WALK_MODE: GcHeader = GcHeader(0x6f);
/// `HEADER_GC_CHANGE_SKILL_GROUP` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHANGE_SKILL_GROUP: GcHeader = GcHeader(0x70);
/// `HEADER_GC_REFINE_INFORMATION` from the legacy game-to-client protocol tables.
pub const HEADER_GC_REFINE_INFORMATION: GcHeader = GcHeader(0x5f);
/// `HEADER_GC_REFINE_INFORMATION_NEW` from the legacy game-to-client protocol tables.
pub const HEADER_GC_REFINE_INFORMATION_NEW: GcHeader = GcHeader(0x77);
/// `HEADER_GC_SEPCIAL_EFFECT` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SEPCIAL_EFFECT: GcHeader = GcHeader(0x72);
/// `HEADER_GC_NPC_POSITION` from the legacy game-to-client protocol tables.
pub const HEADER_GC_NPC_POSITION: GcHeader = GcHeader(0x73);
/// `HEADER_GC_CHANGE_NAME` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHANGE_NAME: GcHeader = GcHeader(0x6b);
/// `HEADER_GC_LOGIN_KEY` from the legacy game-to-client protocol tables.
pub const HEADER_GC_LOGIN_KEY: GcHeader = GcHeader(0x76);
/// `HEADER_GC_AUTH_SUCCESS` from the legacy game-to-client protocol tables.
pub const HEADER_GC_AUTH_SUCCESS: GcHeader = GcHeader(0x96);
/// `HEADER_GC_CHANNEL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHANNEL: GcHeader = GcHeader(0x79);
/// `HEADER_GC_VIEW_EQUIP` from the legacy game-to-client protocol tables.
pub const HEADER_GC_VIEW_EQUIP: GcHeader = GcHeader(0x63);
/// `HEADER_GC_LAND_LIST` from the legacy game-to-client protocol tables.
pub const HEADER_GC_LAND_LIST: GcHeader = GcHeader(0x82);
/// `HEADER_GC_TARGET_UPDATE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_TARGET_UPDATE: GcHeader = GcHeader(0x7b);
/// `HEADER_GC_TARGET_DELETE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_TARGET_DELETE: GcHeader = GcHeader(0x7c);
/// `HEADER_GC_TARGET_CREATE_NEW` from the legacy game-to-client protocol tables.
pub const HEADER_GC_TARGET_CREATE_NEW: GcHeader = GcHeader(0x7d);
/// `HEADER_GC_AFFECT_ADD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_AFFECT_ADD: GcHeader = GcHeader(0x7e);
/// `HEADER_GC_AFFECT_REMOVE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_AFFECT_REMOVE: GcHeader = GcHeader(0x7f);
/// `HEADER_GC_MALL_OPEN` from the legacy game-to-client protocol tables.
pub const HEADER_GC_MALL_OPEN: GcHeader = GcHeader(0x7a);
/// `HEADER_GC_MALL_SET` from the legacy game-to-client protocol tables.
pub const HEADER_GC_MALL_SET: GcHeader = GcHeader(0x80);
/// `HEADER_GC_MALL_DEL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_MALL_DEL: GcHeader = GcHeader(0x81);
/// `HEADER_GC_LOVER_INFO` from the legacy game-to-client protocol tables.
pub const HEADER_GC_LOVER_INFO: GcHeader = GcHeader(0x83);
/// `HEADER_GC_LOVE_POINT_UPDATE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_LOVE_POINT_UPDATE: GcHeader = GcHeader(0x84);
/// `HEADER_GC_DIG_MOTION` from the legacy game-to-client protocol tables.
pub const HEADER_GC_DIG_MOTION: GcHeader = GcHeader(0x86);
/// `HEADER_GC_DAMAGE_INFO` from the legacy game-to-client protocol tables.
pub const HEADER_GC_DAMAGE_INFO: GcHeader = GcHeader(0x87);
/// `HEADER_GC_HYBRIDCRYPT_KEYS` from the legacy game-to-client protocol tables.
pub const HEADER_GC_HYBRIDCRYPT_KEYS: GcHeader = GcHeader(0x98);
/// `HEADER_GC_HYBRIDCRYPT_SDB` from the legacy game-to-client protocol tables.
pub const HEADER_GC_HYBRIDCRYPT_SDB: GcHeader = GcHeader(0x99);
/// `HEADER_GC_SPECIFIC_EFFECT` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SPECIFIC_EFFECT: GcHeader = GcHeader(0xd0);
/// `HEADER_GC_DRAGON_SOUL_REFINE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_DRAGON_SOUL_REFINE: GcHeader = GcHeader(0xd1);
/// `HEADER_GC_AUTO_SHAMAN_SKILL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_AUTO_SHAMAN_SKILL: GcHeader = GcHeader(0x39);
/// `HEADER_GC_EVENT_INFO` from the legacy game-to-client protocol tables.
pub const HEADER_GC_EVENT_INFO: GcHeader = GcHeader(0x9b);
/// `HEADER_GC_EVENT_RELOAD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_EVENT_RELOAD: GcHeader = GcHeader(0x9c);
/// `HEADER_GC_EVENT_KW_SCORE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_EVENT_KW_SCORE: GcHeader = GcHeader(0x9d);
/// `HEADER_GC_SWITCHBOT` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SWITCHBOT: GcHeader = GcHeader(0xab);
/// `HEADER_GC_REQUEST_CHANGE_LANGUAGE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_REQUEST_CHANGE_LANGUAGE: GcHeader = GcHeader(0xf5);
/// `HEADER_GC_WHISPER_DETAILS` from the legacy game-to-client protocol tables.
pub const HEADER_GC_WHISPER_DETAILS: GcHeader = GcHeader(0xf6);
/// `HEADER_GC_DAILY_GIFT` from the legacy game-to-client protocol tables.
pub const HEADER_GC_DAILY_GIFT: GcHeader = GcHeader(0xb4);
/// `HEADER_GC_FISH_EVENT_INFO` from the legacy game-to-client protocol tables.
pub const HEADER_GC_FISH_EVENT_INFO: GcHeader = GcHeader(0xdf);
/// `HEADER_GC_UNK_213` from the legacy game-to-client protocol tables.
pub const HEADER_GC_UNK_213: GcHeader = GcHeader(0xd5);
/// `HEADER_GC_SASH` from the legacy game-to-client protocol tables.
pub const HEADER_GC_SASH: GcHeader = GcHeader(0xe7);
/// `HEADER_GC_CL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CL: GcHeader = GcHeader(0xea);
/// `HEADER_GC_CUBE_RENEWAL` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CUBE_RENEWAL: GcHeader = GcHeader(0xdd);
/// `HEADER_GC_CHARACTER_GOLD` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHARACTER_GOLD: GcHeader = GcHeader(0xe0);
/// `HEADER_GC_CHARACTER_GOLD_CHANGE` from the legacy game-to-client protocol tables.
pub const HEADER_GC_CHARACTER_GOLD_CHANGE: GcHeader = GcHeader(0xe1);
/// `HEADER_GC_REFINE_ELEMENT` from the legacy game-to-client protocol tables.
pub const HEADER_GC_REFINE_ELEMENT: GcHeader = GcHeader(0xe4);
/// `HEADER_GC_ENTITY` from the legacy game-to-client protocol tables.
pub const HEADER_GC_ENTITY: GcHeader = GcHeader(0xf9);
/// `HEADER_GC_WORLD_BOSS` from the legacy game-to-client protocol tables.
pub const HEADER_GC_WORLD_BOSS: GcHeader = GcHeader(0x94);
/// How the legacy client decodes a game-to-client frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcFraming {
    /// The client reads a fixed number of bytes for the record.
    StaticSize,
    /// The client reads a length from the record and then consumes that many
    /// further bytes.
    DynamicSize,
}

/// One machine-readable legacy game-to-client decode-table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyGcPacket {
    /// One-byte game-to-client protocol header.
    pub header: GcHeader,
    /// Client-side `HEADER_GC_*` name used in the decode table.
    pub client_name: &'static str,
    /// Game-server `HEADER_GC_*` name for the same wire byte, when the game
    /// server has an enumerator for it.
    pub server_name: Option<&'static str>,
    /// Packed C++ type whose `sizeof` the client registers.
    pub cpp_type: &'static str,
    /// Whether the client decodes a fixed or length-driven frame.
    pub framing: GcFraming,
    /// Whether a Rust codec for this record exists in the `protocol` crate.
    pub implemented_in_rust: bool,
}
/// The legacy game-to-client decode table in ascending header order.
pub const LEGACY_GC_PACKET_INVENTORY: &[LegacyGcPacket] = &[
    LegacyGcPacket {
        header: HEADER_GC_CHARACTER_ADD,
        client_name: "HEADER_GC_CHARACTER_ADD",
        server_name: Some("HEADER_GC_CHARACTER_ADD"),
        cpp_type: "TPacketGCCharacterAdd",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHARACTER_DEL,
        client_name: "HEADER_GC_CHARACTER_DEL",
        server_name: Some("HEADER_GC_CHARACTER_DEL"),
        cpp_type: "TPacketGCCharacterDelete",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHARACTER_MOVE,
        client_name: "HEADER_GC_CHARACTER_MOVE",
        server_name: Some("HEADER_GC_MOVE"),
        cpp_type: "TPacketGCMove",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHAT,
        client_name: "HEADER_GC_CHAT",
        server_name: Some("HEADER_GC_CHAT"),
        cpp_type: "TPacketGCChat",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SYNC_POSITION,
        client_name: "HEADER_GC_SYNC_POSITION",
        server_name: Some("HEADER_GC_SYNC_POSITION"),
        cpp_type: "TPacketGCC2C",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_LOGIN_SUCCESS3,
        client_name: "HEADER_GC_LOGIN_SUCCESS3",
        server_name: Some("HEADER_GC_LOGIN_SUCCESS"),
        cpp_type: "TPacketGCLoginSuccess3",
        framing: GcFraming::StaticSize,
        // The server names byte 6 and has the struct, but its only login-success
        // send site writes `HEADER_GC_LOGIN_SUCCESS_NEWSLOT` (32) into that same
        // struct (`server/server/game/desc.cpp:880`). A whole-word search for
        // `HEADER_GC_LOGIN_SUCCESS` over `server/server` returns the enumerator
        // at `packet.h:106` and nothing else, so byte 6 has no producer. The
        // Rewrite's `GcLoginSuccess` encodes byte 32, so this stays false.
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_LOGIN_FAILURE,
        client_name: "HEADER_GC_LOGIN_FAILURE",
        server_name: Some("HEADER_GC_LOGIN_FAILURE"),
        cpp_type: "TPacketGCLoginFailure",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PLAYER_CREATE_SUCCESS,
        client_name: "HEADER_GC_PLAYER_CREATE_SUCCESS",
        server_name: Some("HEADER_GC_CHARACTER_CREATE_SUCCESS"),
        cpp_type: "TPacketGCPlayerCreateSuccess",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PLAYER_CREATE_FAILURE,
        client_name: "HEADER_GC_PLAYER_CREATE_FAILURE",
        server_name: Some("HEADER_GC_CHARACTER_CREATE_FAILURE"),
        cpp_type: "TPacketGCCreateFailure",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PLAYER_DELETE_SUCCESS,
        client_name: "HEADER_GC_PLAYER_DELETE_SUCCESS",
        server_name: Some("HEADER_GC_CHARACTER_DELETE_SUCCESS"),
        cpp_type: "TPacketGCBlank",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID,
        client_name: "HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID",
        server_name: Some("HEADER_GC_CHARACTER_DELETE_WRONG_SOCIAL_ID"),
        cpp_type: "TPacketGCBlank",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_STUN,
        client_name: "HEADER_GC_STUN",
        server_name: Some("HEADER_GC_STUN"),
        cpp_type: "TPacketGCStun",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_DEAD,
        client_name: "HEADER_GC_DEAD",
        server_name: Some("HEADER_GC_DEAD"),
        cpp_type: "TPacketGCDead",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_MAIN_CHARACTER,
        client_name: "HEADER_GC_MAIN_CHARACTER",
        server_name: Some("HEADER_GC_MAIN_CHARACTER_OLD"),
        cpp_type: "TPacketGCMainCharacter",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PLAYER_POINTS,
        client_name: "HEADER_GC_PLAYER_POINTS",
        server_name: Some("HEADER_GC_CHARACTER_POINTS"),
        cpp_type: "TPacketGCPoints",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PLAYER_POINT_CHANGE,
        client_name: "HEADER_GC_PLAYER_POINT_CHANGE",
        server_name: Some("HEADER_GC_CHARACTER_POINT_CHANGE"),
        cpp_type: "TPacketGCPointChange",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHANGE_SPEED,
        client_name: "HEADER_GC_CHANGE_SPEED",
        server_name: Some("HEADER_GC_CHANGE_SPEED"),
        cpp_type: "TPacketGCChangeSpeed",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHARACTER_UPDATE,
        client_name: "HEADER_GC_CHARACTER_UPDATE",
        server_name: Some("HEADER_GC_CHARACTER_UPDATE"),
        cpp_type: "TPacketGCCharacterUpdate",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_ITEM_SET,
        client_name: "HEADER_GC_ITEM_SET",
        server_name: Some("HEADER_GC_ITEM_DEL"),
        cpp_type: "TPacketGCItemSet",
        framing: GcFraming::StaticSize,
        // `crate::gc_item_window::GcItemDel`, the 62-byte
        // `TPacketGCItemDelDeprecated` the server actually writes. The client's
        // own struct for byte 20 is 72 bytes, so the client name is misleading.
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_ITEM_SET2,
        client_name: "HEADER_GC_ITEM_SET2",
        server_name: Some("HEADER_GC_ITEM_SET"),
        cpp_type: "TPacketGCItemSet2",
        framing: GcFraming::StaticSize,
        // `crate::gc_item_window::GcItemSet`, the 72-byte `TPacketGCItemSet`.
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_ITEM_USE,
        client_name: "HEADER_GC_ITEM_USE",
        server_name: Some("HEADER_GC_ITEM_USE"),
        cpp_type: "TPacketGCItemUse",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_ITEM_UPDATE,
        client_name: "HEADER_GC_ITEM_UPDATE",
        server_name: Some("HEADER_GC_ITEM_UPDATE"),
        cpp_type: "TPacketGCItemUpdate",
        framing: GcFraming::StaticSize,
        // `crate::gc_item_window::GcItemUpdate`, 59 bytes.
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_ITEM_GROUND_ADD,
        client_name: "HEADER_GC_ITEM_GROUND_ADD",
        server_name: Some("HEADER_GC_ITEM_GROUND_ADD"),
        cpp_type: "TPacketGCItemGroundAdd",
        framing: GcFraming::StaticSize,
        // `crate::gc_item_window::GcItemGroundAdd`, 21 bytes.
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_ITEM_GROUND_DEL,
        client_name: "HEADER_GC_ITEM_GROUND_DEL",
        server_name: Some("HEADER_GC_ITEM_GROUND_DEL"),
        cpp_type: "TPacketGCItemGroundDel",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_QUICKSLOT_ADD,
        client_name: "HEADER_GC_QUICKSLOT_ADD",
        server_name: Some("HEADER_GC_QUICKSLOT_ADD"),
        cpp_type: "TPacketGCQuickSlotAdd",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_QUICKSLOT_DEL,
        client_name: "HEADER_GC_QUICKSLOT_DEL",
        server_name: Some("HEADER_GC_QUICKSLOT_DEL"),
        cpp_type: "TPacketGCQuickSlotDel",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_QUICKSLOT_SWAP,
        client_name: "HEADER_GC_QUICKSLOT_SWAP",
        server_name: Some("HEADER_GC_QUICKSLOT_SWAP"),
        cpp_type: "TPacketGCQuickSlotSwap",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_ITEM_OWNERSHIP,
        client_name: "HEADER_GC_ITEM_OWNERSHIP",
        server_name: Some("HEADER_GC_ITEM_OWNERSHIP"),
        cpp_type: "TPacketGCItemOwnership",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_LOGIN_SUCCESS4,
        client_name: "HEADER_GC_LOGIN_SUCCESS4",
        server_name: Some("HEADER_GC_LOGIN_SUCCESS_NEWSLOT"),
        cpp_type: "TPacketGCLoginSuccess4",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_WHISPER,
        client_name: "HEADER_GC_WHISPER",
        server_name: Some("HEADER_GC_WHISPER"),
        cpp_type: "TPacketGCWhisper",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_MOTION,
        client_name: "HEADER_GC_MOTION",
        server_name: Some("HEADER_GC_MOTION"),
        cpp_type: "TPacketGCMotion",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SHOP,
        client_name: "HEADER_GC_SHOP",
        server_name: Some("HEADER_GC_SHOP"),
        cpp_type: "TPacketGCShop",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SHOP_SIGN,
        client_name: "HEADER_GC_SHOP_SIGN",
        server_name: Some("HEADER_GC_SHOP_SIGN"),
        cpp_type: "TPacketGCShopSign",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_DUEL_START,
        client_name: "HEADER_GC_DUEL_START",
        server_name: Some("HEADER_GC_DUEL_START"),
        cpp_type: "TPacketGCDuelStart",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_PVP,
        client_name: "HEADER_GC_PVP",
        server_name: Some("HEADER_GC_PVP"),
        cpp_type: "TPacketGCPVP",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_EXCHANGE,
        client_name: "HEADER_GC_EXCHANGE",
        server_name: Some("HEADER_GC_EXCHANGE"),
        cpp_type: "TPacketGCExchange",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHARACTER_POSITION,
        client_name: "HEADER_GC_CHARACTER_POSITION",
        server_name: Some("HEADER_GC_CHARACTER_POSITION"),
        cpp_type: "TPacketGCPosition",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PING,
        client_name: "HEADER_GC_PING",
        server_name: Some("HEADER_GC_PING"),
        cpp_type: "TPacketGCPing",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SCRIPT,
        client_name: "HEADER_GC_SCRIPT",
        server_name: Some("HEADER_GC_SCRIPT"),
        cpp_type: "TPacketGCScript",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_QUEST_CONFIRM,
        client_name: "HEADER_GC_QUEST_CONFIRM",
        server_name: Some("HEADER_GC_QUEST_CONFIRM"),
        cpp_type: "TPacketGCQuestConfirm",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_AUTO_SHAMAN_SKILL,
        client_name: "HEADER_GC_AUTO_SHAMAN_SKILL",
        server_name: Some("HEADER_GC_AUTO_SHAMAN_SKILL"),
        cpp_type: "TPacketGCShamanUseSkill",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_TARGET_INFO,
        client_name: "HEADER_GC_TARGET_INFO",
        server_name: Some("HEADER_GC_TARGET_INFO"),
        cpp_type: "TPacketGCTargetInfo",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_MOUNT,
        client_name: "HEADER_GC_MOUNT",
        server_name: Some("HEADER_GC_MOUNT"),
        cpp_type: "TPacketGCMount",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_OWNERSHIP,
        client_name: "HEADER_GC_OWNERSHIP",
        server_name: Some("HEADER_GC_OWNERSHIP"),
        cpp_type: "TPacketGCOwnership",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_TARGET,
        client_name: "HEADER_GC_TARGET",
        server_name: Some("HEADER_GC_TARGET"),
        cpp_type: "TPacketGCTarget",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PICKUP_ITEM_SC,
        client_name: "HEADER_GC_PICKUP_ITEM_SC",
        server_name: Some("HEADER_GC_PICKUP_ITEM_SC"),
        cpp_type: "TPacketGCPickupItemSC",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_WARP,
        client_name: "HEADER_GC_WARP",
        server_name: Some("HEADER_GC_WARP"),
        cpp_type: "TPacketGCWarp",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_ADD_FLY_TARGETING,
        client_name: "HEADER_GC_ADD_FLY_TARGETING",
        server_name: Some("HEADER_GC_ADD_FLY_TARGETING"),
        cpp_type: "TPacketGCFlyTargeting",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CREATE_FLY,
        client_name: "HEADER_GC_CREATE_FLY",
        server_name: Some("HEADER_GC_CREATE_FLY"),
        cpp_type: "TPacketGCCreateFly",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_FLY_TARGETING,
        client_name: "HEADER_GC_FLY_TARGETING",
        server_name: Some("HEADER_GC_FLY_TARGETING"),
        cpp_type: "TPacketGCFlyTargeting",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SKILL_LEVEL,
        client_name: "HEADER_GC_SKILL_LEVEL",
        server_name: Some("HEADER_GC_SKILL_LEVEL_OLD"),
        cpp_type: "TPacketGCSkillLevel",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SKILL_COOLTIME_END,
        client_name: "HEADER_GC_SKILL_COOLTIME_END",
        server_name: None,
        cpp_type: "TPacketGCSkillCoolTimeEnd",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_MESSENGER,
        client_name: "HEADER_GC_MESSENGER",
        server_name: Some("HEADER_GC_MESSENGER"),
        cpp_type: "TPacketGCMessenger",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_GUILD,
        client_name: "HEADER_GC_GUILD",
        server_name: Some("HEADER_GC_GUILD"),
        cpp_type: "TPacketGCGuild",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_SKILL_LEVEL_NEW,
        client_name: "HEADER_GC_SKILL_LEVEL_NEW",
        server_name: Some("HEADER_GC_SKILL_LEVEL"),
        cpp_type: "TPacketGCSkillLevelNew",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PARTY_INVITE,
        client_name: "HEADER_GC_PARTY_INVITE",
        server_name: Some("HEADER_GC_PARTY_INVITE"),
        cpp_type: "TPacketGCPartyInvite",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PARTY_ADD,
        client_name: "HEADER_GC_PARTY_ADD",
        server_name: Some("HEADER_GC_PARTY_ADD"),
        cpp_type: "TPacketGCPartyAdd",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PARTY_UPDATE,
        client_name: "HEADER_GC_PARTY_UPDATE",
        server_name: Some("HEADER_GC_PARTY_UPDATE"),
        cpp_type: "TPacketGCPartyUpdate",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PARTY_REMOVE,
        client_name: "HEADER_GC_PARTY_REMOVE",
        server_name: Some("HEADER_GC_PARTY_REMOVE"),
        cpp_type: "TPacketGCPartyRemove",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_QUEST_INFO,
        client_name: "HEADER_GC_QUEST_INFO",
        server_name: Some("HEADER_GC_QUEST_INFO"),
        cpp_type: "TPacketGCQuestInfo",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_REQUEST_MAKE_GUILD,
        client_name: "HEADER_GC_REQUEST_MAKE_GUILD",
        server_name: Some("HEADER_GC_REQUEST_MAKE_GUILD"),
        cpp_type: "TPacketGCBlank",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PARTY_PARAMETER,
        client_name: "HEADER_GC_PARTY_PARAMETER",
        server_name: Some("HEADER_GC_PARTY_PARAMETER"),
        cpp_type: "TPacketGCPartyParameter",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SAFEBOX_MONEY_CHANGE,
        client_name: "HEADER_GC_SAFEBOX_MONEY_CHANGE",
        server_name: None,
        cpp_type: "TPacketGCSafeboxMoneyChange",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SAFEBOX_SET,
        client_name: "HEADER_GC_SAFEBOX_SET",
        server_name: Some("HEADER_GC_SAFEBOX_SET"),
        cpp_type: "TPacketGCItemSet",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_SAFEBOX_DEL,
        client_name: "HEADER_GC_SAFEBOX_DEL",
        server_name: Some("HEADER_GC_SAFEBOX_DEL"),
        cpp_type: "TPacketGCItemDel",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SAFEBOX_WRONG_PASSWORD,
        client_name: "HEADER_GC_SAFEBOX_WRONG_PASSWORD",
        server_name: Some("HEADER_GC_SAFEBOX_WRONG_PASSWORD"),
        cpp_type: "TPacketGCSafeboxWrongPassword",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SAFEBOX_SIZE,
        client_name: "HEADER_GC_SAFEBOX_SIZE",
        server_name: Some("HEADER_GC_SAFEBOX_SIZE"),
        cpp_type: "TPacketGCSafeboxSize",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_FISHING,
        client_name: "HEADER_GC_FISHING",
        server_name: Some("HEADER_GC_FISHING"),
        cpp_type: "TPacketGCFishing",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_EMPIRE,
        client_name: "HEADER_GC_EMPIRE",
        server_name: Some("HEADER_GC_EMPIRE"),
        cpp_type: "TPacketGCEmpire",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PARTY_LINK,
        client_name: "HEADER_GC_PARTY_LINK",
        server_name: Some("HEADER_GC_PARTY_LINK"),
        cpp_type: "TPacketGCPartyLink",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PARTY_UNLINK,
        client_name: "HEADER_GC_PARTY_UNLINK",
        server_name: Some("HEADER_GC_PARTY_UNLINK"),
        cpp_type: "TPacketGCPartyUnlink",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_REFINE_INFORMATION,
        client_name: "HEADER_GC_REFINE_INFORMATION",
        server_name: Some("HEADER_GC_REFINE_INFORMATION_OLD"),
        cpp_type: "TPacketGCRefineInformation",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_VIEW_EQUIP,
        client_name: "HEADER_GC_VIEW_EQUIP",
        server_name: Some("HEADER_GC_VIEW_EQUIP"),
        cpp_type: "TPacketGCViewEquip",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_TIME,
        client_name: "HEADER_GC_TIME",
        server_name: Some("HEADER_GC_TIME"),
        cpp_type: "TPacketGCTime",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHANGE_NAME,
        client_name: "HEADER_GC_CHANGE_NAME",
        server_name: Some("HEADER_GC_CHANGE_NAME"),
        cpp_type: "TPacketGCChangeName",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_DUNGEON,
        client_name: "HEADER_GC_DUNGEON",
        server_name: Some("HEADER_GC_DUNGEON"),
        cpp_type: "TPacketGCDungeon",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_WALK_MODE,
        client_name: "HEADER_GC_WALK_MODE",
        server_name: Some("HEADER_GC_WALK_MODE"),
        cpp_type: "TPacketGCWalkMode",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHANGE_SKILL_GROUP,
        client_name: "HEADER_GC_CHANGE_SKILL_GROUP",
        server_name: Some("HEADER_GC_SKILL_GROUP"),
        cpp_type: "TPacketGCChangeSkillGroup",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_MAIN_CHARACTER2_EMPIRE,
        client_name: "HEADER_GC_MAIN_CHARACTER2_EMPIRE",
        server_name: Some("HEADER_GC_MAIN_CHARACTER"),
        cpp_type: "TPacketGCMainCharacter2_EMPIRE",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SEPCIAL_EFFECT,
        client_name: "HEADER_GC_SEPCIAL_EFFECT",
        server_name: Some("HEADER_GC_SEPCIAL_EFFECT"),
        cpp_type: "TPacketGCSpecialEffect",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_NPC_POSITION,
        client_name: "HEADER_GC_NPC_POSITION",
        server_name: Some("HEADER_GC_NPC_POSITION"),
        cpp_type: "TPacketGCNPCPosition",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHARACTER_UPDATE2,
        client_name: "HEADER_GC_CHARACTER_UPDATE2",
        server_name: None,
        cpp_type: "TPacketGCCharacterUpdate2",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_LOGIN_KEY,
        client_name: "HEADER_GC_LOGIN_KEY",
        server_name: Some("HEADER_GC_LOGIN_KEY"),
        cpp_type: "TPacketGCLoginKey",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_REFINE_INFORMATION_NEW,
        client_name: "HEADER_GC_REFINE_INFORMATION_NEW",
        server_name: Some("HEADER_GC_REFINE_INFORMATION"),
        cpp_type: "TPacketGCRefineInformationNew",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHARACTER_ADD2,
        client_name: "HEADER_GC_CHARACTER_ADD2",
        server_name: None,
        cpp_type: "TPacketGCCharacterAdd2",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHANNEL,
        client_name: "HEADER_GC_CHANNEL",
        server_name: Some("HEADER_GC_CHANNEL"),
        cpp_type: "TPacketGCChannel",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_MALL_OPEN,
        client_name: "HEADER_GC_MALL_OPEN",
        server_name: Some("HEADER_GC_MALL_OPEN"),
        cpp_type: "TPacketGCMallOpen",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_TARGET_UPDATE,
        client_name: "HEADER_GC_TARGET_UPDATE",
        server_name: Some("HEADER_GC_TARGET_UPDATE"),
        cpp_type: "TPacketGCTargetUpdate",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_TARGET_DELETE,
        client_name: "HEADER_GC_TARGET_DELETE",
        server_name: Some("HEADER_GC_TARGET_DELETE"),
        cpp_type: "TPacketGCTargetDelete",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_TARGET_CREATE_NEW,
        client_name: "HEADER_GC_TARGET_CREATE_NEW",
        server_name: Some("HEADER_GC_TARGET_CREATE"),
        cpp_type: "TPacketGCTargetCreateNew",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_AFFECT_ADD,
        client_name: "HEADER_GC_AFFECT_ADD",
        server_name: Some("HEADER_GC_AFFECT_ADD"),
        cpp_type: "TPacketGCAffectAdd",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_AFFECT_REMOVE,
        client_name: "HEADER_GC_AFFECT_REMOVE",
        server_name: Some("HEADER_GC_AFFECT_REMOVE"),
        cpp_type: "TPacketGCAffectRemove",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_MALL_SET,
        client_name: "HEADER_GC_MALL_SET",
        server_name: Some("HEADER_GC_MALL_SET"),
        cpp_type: "TPacketGCItemSet",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_MALL_DEL,
        client_name: "HEADER_GC_MALL_DEL",
        server_name: Some("HEADER_GC_MALL_DEL"),
        cpp_type: "TPacketGCItemDel",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_LAND_LIST,
        client_name: "HEADER_GC_LAND_LIST",
        server_name: Some("HEADER_GC_LAND_LIST"),
        cpp_type: "TPacketGCLandList",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_LOVER_INFO,
        client_name: "HEADER_GC_LOVER_INFO",
        server_name: Some("HEADER_GC_LOVER_INFO"),
        cpp_type: "TPacketGCLoverInfo",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_LOVE_POINT_UPDATE,
        client_name: "HEADER_GC_LOVE_POINT_UPDATE",
        server_name: Some("HEADER_GC_LOVE_POINT_UPDATE"),
        cpp_type: "TPacketGCLovePointUpdate",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_DIG_MOTION,
        client_name: "HEADER_GC_DIG_MOTION",
        server_name: Some("HEADER_GC_DIG_MOTION"),
        cpp_type: "TPacketGCDigMotion",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_DAMAGE_INFO,
        client_name: "HEADER_GC_DAMAGE_INFO",
        server_name: Some("HEADER_GC_DAMAGE_INFO"),
        cpp_type: "TPacketGCDamageInfo",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHAR_ADDITIONAL_INFO,
        client_name: "HEADER_GC_CHAR_ADDITIONAL_INFO",
        server_name: Some("HEADER_GC_CHAR_ADDITIONAL_INFO"),
        cpp_type: "TPacketGCCharacterAdditionalInfo",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_MAIN_CHARACTER3_BGM,
        client_name: "HEADER_GC_MAIN_CHARACTER3_BGM",
        server_name: Some("HEADER_GC_MAIN_CHARACTER3_BGM"),
        cpp_type: "TPacketGCMainCharacter3_BGM",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_MAIN_CHARACTER4_BGM_VOL,
        client_name: "HEADER_GC_MAIN_CHARACTER4_BGM_VOL",
        server_name: Some("HEADER_GC_MAIN_CHARACTER4_BGM_VOL"),
        cpp_type: "TPacketGCMainCharacter4_BGM_VOL",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_BIOLOGIST,
        client_name: "HEADER_GC_BIOLOGIST",
        server_name: Some("HEADER_GC_BIOLOGIST"),
        cpp_type: "TPacketGCBiologist",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_PREMIUM_PLAYERS,
        client_name: "HEADER_GC_PREMIUM_PLAYERS",
        server_name: Some("HEADER_GC_PREMIUM_PLAYERS"),
        cpp_type: "TPacketGCPremiumPlayers",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_WORLD_BOSS,
        client_name: "HEADER_GC_WORLD_BOSS",
        server_name: Some("HEADER_GC_WORLD_BOSS"),
        cpp_type: "TPacketGCWorldBoss",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_AUTH_SUCCESS,
        client_name: "HEADER_GC_AUTH_SUCCESS",
        server_name: Some("HEADER_GC_AUTH_SUCCESS"),
        cpp_type: "TPacketGCAuthSuccess",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_HYBRIDCRYPT_KEYS,
        client_name: "HEADER_GC_HYBRIDCRYPT_KEYS",
        server_name: Some("HEADER_GC_HYBRIDCRYPT_KEYS"),
        cpp_type: "TPacketGCHybridCryptKeys",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_HYBRIDCRYPT_SDB,
        client_name: "HEADER_GC_HYBRIDCRYPT_SDB",
        server_name: Some("HEADER_GC_HYBRIDCRYPT_SDB"),
        cpp_type: "TPacketGCHybridSDB",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_EVENT_INFO,
        client_name: "HEADER_GC_EVENT_INFO",
        server_name: Some("HEADER_GC_EVENT_INFO"),
        cpp_type: "TPacketGCEventInfo",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_EVENT_RELOAD,
        client_name: "HEADER_GC_EVENT_RELOAD",
        server_name: Some("HEADER_GC_EVENT_RELOAD"),
        cpp_type: "TPacketGCEventReload",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_EVENT_KW_SCORE,
        client_name: "HEADER_GC_EVENT_KW_SCORE",
        server_name: Some("HEADER_GC_EVENT_KW_SCORE"),
        cpp_type: "TPacketGCEventKWScore",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SWITCHBOT,
        client_name: "HEADER_GC_SWITCHBOT",
        server_name: Some("HEADER_GC_SWITCHBOT"),
        cpp_type: "TPacketGCSwitchbot",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_PRIVATE_SHOP,
        client_name: "HEADER_GC_PRIVATE_SHOP",
        server_name: Some("HEADER_GC_PRIVATE_SHOP"),
        cpp_type: "TPacketGCPrivateShop",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_DAILY_GIFT,
        client_name: "HEADER_GC_DAILY_GIFT",
        server_name: Some("HEADER_GC_DAILY_GIFT"),
        cpp_type: "TPacketGCDailyGift",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SPECIFIC_EFFECT,
        client_name: "HEADER_GC_SPECIFIC_EFFECT",
        server_name: Some("HEADER_GC_SPECIFIC_EFFECT"),
        cpp_type: "TPacketGCSpecificEffect",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_DRAGON_SOUL_REFINE,
        client_name: "HEADER_GC_DRAGON_SOUL_REFINE",
        server_name: Some("HEADER_GC_DRAGON_SOUL_REFINE"),
        cpp_type: "TPacketGCDragonSoulRefine",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN,
        client_name: "HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN",
        server_name: Some("HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN"),
        cpp_type: "TPacketGCOpenDragonSoulChangeAttr",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT,
        client_name: "HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT",
        server_name: Some("HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT"),
        cpp_type: "TPacketGCDragonSoulChangeAttrResult",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_UNK_213,
        client_name: "HEADER_GC_UNK_213",
        server_name: None,
        cpp_type: "TPacketGCUnk213",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_AURA,
        client_name: "HEADER_GC_AURA",
        server_name: Some("HEADER_GC_AURA"),
        cpp_type: "TPacketGCAura",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_CUBE_RENEWAL,
        client_name: "HEADER_GC_CUBE_RENEWAL",
        server_name: Some("HEADER_GC_CUBE_RENEWAL"),
        cpp_type: "TPacketGCCubeRenewalReceive",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_FISH_EVENT_INFO,
        client_name: "HEADER_GC_FISH_EVENT_INFO",
        server_name: Some("HEADER_GC_FISH_EVENT_INFO"),
        cpp_type: "TPacketGCFishEventInfo",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHARACTER_GOLD,
        client_name: "HEADER_GC_CHARACTER_GOLD",
        server_name: Some("HEADER_GC_CHARACTER_GOLD"),
        cpp_type: "TPacketGCGold",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_CHARACTER_GOLD_CHANGE,
        client_name: "HEADER_GC_CHARACTER_GOLD_CHANGE",
        server_name: Some("HEADER_GC_CHARACTER_GOLD_CHANGE"),
        cpp_type: "TPacketGCGoldChange",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_REFINE_ELEMENT,
        client_name: "HEADER_GC_REFINE_ELEMENT",
        server_name: Some("HEADER_GC_REFINE_ELEMENT"),
        cpp_type: "TPacketGCRefineElement",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_SASH,
        client_name: "HEADER_GC_SASH",
        server_name: Some("HEADER_GC_SASH"),
        cpp_type: "TPacketSash",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_CL,
        client_name: "HEADER_GC_CL",
        server_name: Some("HEADER_GC_CL"),
        cpp_type: "TPacketChangeLook",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_REQUEST_CHANGE_LANGUAGE,
        client_name: "HEADER_GC_REQUEST_CHANGE_LANGUAGE",
        server_name: Some("HEADER_GC_REQUEST_CHANGE_LANGUAGE"),
        cpp_type: "TPacketChangeLanguage",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_WHISPER_DETAILS,
        client_name: "HEADER_GC_WHISPER_DETAILS",
        server_name: Some("HEADER_GC_WHISPER_DETAILS"),
        cpp_type: "TPacketGCWhisperDetails",
        framing: GcFraming::StaticSize,
        implemented_in_rust: false,
    },
    LegacyGcPacket {
        header: HEADER_GC_ENTITY,
        client_name: "HEADER_GC_ENTITY",
        server_name: Some("HEADER_GC_ENTITY"),
        cpp_type: "TPacketGCEntity",
        framing: GcFraming::DynamicSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_HANDSHAKE_OK,
        client_name: "HEADER_GC_HANDSHAKE_OK",
        server_name: Some("HEADER_GC_TIME_SYNC"),
        cpp_type: "TPacketGCBlank",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_PHASE,
        client_name: "HEADER_GC_PHASE",
        server_name: Some("HEADER_GC_PHASE"),
        cpp_type: "TPacketGCPhase",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_BINDUDP,
        client_name: "HEADER_GC_BINDUDP",
        server_name: Some("HEADER_GC_BINDUDP"),
        cpp_type: "TPacketGCBindUDP",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
    LegacyGcPacket {
        header: HEADER_GC_HANDSHAKE,
        client_name: "HEADER_GC_HANDSHAKE",
        server_name: Some("HEADER_GC_HANDSHAKE"),
        cpp_type: "TPacketGCHandshake",
        framing: GcFraming::StaticSize,
        implemented_in_rust: true,
    },
];
/// Resolve a raw game-to-client header to its legacy decode-table entry.
#[must_use]
pub fn resolve_gc_packet(header: u8) -> Option<&'static LegacyGcPacket> {
    LEGACY_GC_PACKET_INVENTORY
        .iter()
        .find(|entry| entry.header.value() == header)
}

/// Count the decode-table entries that have no Rust codec yet.
#[must_use]
pub fn gc_missing_codec_count() -> usize {
    LEGACY_GC_PACKET_INVENTORY
        .iter()
        .filter(|entry| !entry.implemented_in_rust)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn table_matches_the_client_decode_table() {
        assert_eq!(LEGACY_GC_PACKET_INVENTORY.len(), 134);
        let static_count = LEGACY_GC_PACKET_INVENTORY
            .iter()
            .filter(|entry| entry.framing == GcFraming::StaticSize)
            .count();
        let dynamic_count = LEGACY_GC_PACKET_INVENTORY
            .iter()
            .filter(|entry| entry.framing == GcFraming::DynamicSize)
            .count();
        assert_eq!(static_count, 115);
        assert_eq!(dynamic_count, 19);
    }

    #[test]
    fn no_two_game_to_client_headers_share_a_byte() {
        let mut seen = HashSet::new();
        for entry in LEGACY_GC_PACKET_INVENTORY {
            assert!(
                seen.insert(entry.header.value()),
                "duplicate byte for {}",
                entry.client_name
            );
        }
    }

    #[test]
    fn the_table_is_sorted_by_header_byte() {
        let bytes: Vec<u8> = LEGACY_GC_PACKET_INVENTORY
            .iter()
            .map(|entry| entry.header.value())
            .collect();
        let mut sorted = bytes.clone();
        sorted.sort_unstable();
        assert_eq!(bytes, sorted);
    }

    #[test]
    fn the_four_renamed_families_pair_by_byte_not_by_name() {
        // The server keeps the modern name and suffixes the legacy generation,
        // while the client does the opposite. These bytes still agree.
        for (client, server) in [
            ("HEADER_GC_SKILL_LEVEL_NEW", "HEADER_GC_SKILL_LEVEL"),
            (
                "HEADER_GC_MAIN_CHARACTER2_EMPIRE",
                "HEADER_GC_MAIN_CHARACTER",
            ),
            (
                "HEADER_GC_REFINE_INFORMATION_NEW",
                "HEADER_GC_REFINE_INFORMATION",
            ),
            ("HEADER_GC_ITEM_SET2", "HEADER_GC_ITEM_SET"),
        ] {
            let entry = LEGACY_GC_PACKET_INVENTORY
                .iter()
                .find(|entry| entry.client_name == client)
                .unwrap_or_else(|| panic!("missing {client}"));
            assert_eq!(entry.server_name, Some(server), "{client}");
        }
    }

    #[test]
    fn the_legacy_generations_are_separate_rows() {
        // The client's plain names land on the bytes the server marks `_OLD`.
        for (client, server) in [
            ("HEADER_GC_SKILL_LEVEL", "HEADER_GC_SKILL_LEVEL_OLD"),
            ("HEADER_GC_MAIN_CHARACTER", "HEADER_GC_MAIN_CHARACTER_OLD"),
            (
                "HEADER_GC_REFINE_INFORMATION",
                "HEADER_GC_REFINE_INFORMATION_OLD",
            ),
        ] {
            let entry = LEGACY_GC_PACKET_INVENTORY
                .iter()
                .find(|entry| entry.client_name == client)
                .unwrap_or_else(|| panic!("missing {client}"));
            assert_eq!(entry.server_name, Some(server), "{client}");
        }
    }

    #[test]
    fn header_bytes_match_the_known_legacy_values() {
        assert_eq!(HEADER_GC_LOGIN_SUCCESS3.value(), 6);
        assert_eq!(HEADER_GC_LOGIN_SUCCESS4.value(), 0x20);
        assert_eq!(HEADER_GC_PLAYER_CREATE_SUCCESS.value(), 8);
        assert_eq!(HEADER_GC_PLAYER_CREATE_FAILURE.value(), 9);
        assert_eq!(HEADER_GC_PLAYER_DELETE_SUCCESS.value(), 0x0a);
        assert_eq!(HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID.value(), 0x0b);
        assert_eq!(HEADER_GC_ITEM_SET2.value(), 21);
        assert_eq!(HEADER_GC_HANDSHAKE_OK.value(), 0xfc);
    }

    #[test]
    fn five_client_rows_have_no_server_enumerator() {
        let missing: Vec<&str> = LEGACY_GC_PACKET_INVENTORY
            .iter()
            .filter(|entry| entry.server_name.is_none())
            .map(|entry| entry.client_name)
            .collect();
        assert_eq!(missing.len(), 5);
        for name in [
            "HEADER_GC_SKILL_COOLTIME_END",
            "HEADER_GC_CHARACTER_ADD2",
            "HEADER_GC_CHARACTER_UPDATE2",
            "HEADER_GC_SAFEBOX_MONEY_CHANGE",
            "HEADER_GC_UNK_213",
        ] {
            assert!(missing.contains(&name), "expected {name} to be serverless");
        }
    }

    #[test]
    fn implementation_coverage_is_reported_honestly() {
        let done: Vec<&str> = LEGACY_GC_PACKET_INVENTORY
            .iter()
            .filter(|entry| entry.implemented_in_rust)
            .map(|entry| entry.client_name)
            .collect();
        assert_eq!(done.len(), 103);
        assert_eq!(gc_missing_codec_count(), 31);
        for name in [
            "HEADER_GC_AFFECT_ADD",
            "HEADER_GC_PLAYER_POINT_CHANGE",
            "HEADER_GC_DIG_MOTION",
            "HEADER_GC_DAMAGE_INFO",
            "HEADER_GC_PREMIUM_PLAYERS",
            "HEADER_GC_ADD_FLY_TARGETING",
            "HEADER_GC_FLY_TARGETING",
            "HEADER_GC_WARP",
            "HEADER_GC_SKILL_LEVEL_NEW",
            "HEADER_GC_DAILY_GIFT",
            "HEADER_GC_PARTY_UPDATE",
            "HEADER_GC_CUBE_RENEWAL",
            "HEADER_GC_LOGIN_FAILURE",
            "HEADER_GC_PLAYER_CREATE_SUCCESS",
            "HEADER_GC_PLAYER_CREATE_FAILURE",
            "HEADER_GC_PLAYER_DELETE_SUCCESS",
            "HEADER_GC_PLAYER_DELETE_WRONG_SOCIAL_ID",
            "HEADER_GC_PING",
            "HEADER_GC_HANDSHAKE",
            "HEADER_GC_BINDUDP",
            "HEADER_GC_PHASE",
            "HEADER_GC_LOGIN_KEY",
            "HEADER_GC_CHARACTER_ADD",
            "HEADER_GC_CHARACTER_MOVE",
            "HEADER_GC_MAIN_CHARACTER",
            "HEADER_GC_CHARACTER_UPDATE",
            "HEADER_GC_TARGET_INFO",
            "HEADER_GC_TARGET",
            "HEADER_GC_SKILL_LEVEL",
            "HEADER_GC_MAIN_CHARACTER2_EMPIRE",
            "HEADER_GC_CHARACTER_UPDATE2",
            "HEADER_GC_TARGET_CREATE_NEW",
            "HEADER_GC_CHAR_ADDITIONAL_INFO",
            "HEADER_GC_MAIN_CHARACTER3_BGM",
            "HEADER_GC_MAIN_CHARACTER4_BGM_VOL",
            "HEADER_GC_CHARACTER_GOLD_CHANGE",
            "HEADER_GC_AUTH_SUCCESS",
            "HEADER_GC_SHOP",
        ] {
            assert!(done.contains(&name), "expected {name} to be implemented");
        }
    }

    #[test]
    fn unregistered_header_bytes_do_not_resolve() {
        // 122 of the 256 byte values are not registered in the client table.
        for byte in [0x00u8, 0x0c, 0x17, 0x21, 0x23, 0x2f, 0x3b, 0x5d] {
            assert_eq!(resolve_gc_packet(byte), None, "byte {byte:#04x}");
        }
        assert!(resolve_gc_packet(HEADER_GC_HANDSHAKE_OK.value()).is_some());
    }

    /// Implementation status must be keyed by wire byte, not by client-side name.
    ///
    /// The two trees rename some record generations in opposite directions, so a record can be
    /// implemented under its server name while the client calls the same byte something else.
    /// Three rows exist only because of that: byte 6 (`HEADER_GC_LOGIN_SUCCESS3` on the client,
    /// `HEADER_GC_LOGIN_SUCCESS` on the server), byte 32 (`HEADER_GC_LOGIN_SUCCESS4` on the
    /// client, `HEADER_GC_LOGIN_SUCCESS_NEWSLOT` on the server), and byte 252
    /// (`HEADER_GC_HANDSHAKE_OK` on the client, `HEADER_GC_TIME_SYNC` on the server). The Rust
    /// constants use the server names, so a name-keyed scan would report all three as renamed.
    ///
    /// Two of the three are implemented; byte 6 is not, and that is the finding rather than an
    /// oversight. The server keeps the enumerator and the struct, but its only login-success send
    /// site writes byte 32 into that struct (`server/server/game/desc.cpp:880`), so a whole-word
    /// search for `HEADER_GC_LOGIN_SUCCESS` over `server/server` returns `packet.h:106` and
    /// nothing else. The Rewrite's `GcLoginSuccess` encodes byte 32, so byte 6 is unreachable and
    /// a codec for it would model a send that never happens.
    #[test]
    fn implementation_status_is_keyed_by_wire_byte_not_by_client_name() {
        // The server-side names the Rust constants actually use, with the bytes they declare.
        const SERVER_NAMED: [(&str, u8); 3] = [
            ("HEADER_GC_LOGIN_SUCCESS", 6),
            ("HEADER_GC_LOGIN_SUCCESS_NEWSLOT", 32),
            ("HEADER_GC_TIME_SYNC", 0xfc),
        ];
        for (server_name, byte) in SERVER_NAMED {
            let entry = resolve_gc_packet(byte)
                .unwrap_or_else(|| panic!("byte {byte:#04x} is not registered"));
            assert_ne!(
                entry.client_name, server_name,
                "byte {byte:#04x} is expected to be renamed between the trees"
            );
        }
        // Byte 32 and byte 252 are the renamed rows the crate actually decodes.
        for byte in [32u8, 0xfc] {
            let entry = resolve_gc_packet(byte).expect("row is registered");
            assert!(
                entry.implemented_in_rust,
                "byte {byte:#04x} is decoded as {} and must stay implemented",
                entry.client_name
            );
        }
        // Byte 6 is the third renamed row and has no producer, so it must stay unimplemented.
        let six = resolve_gc_packet(6).expect("byte 6 is registered");
        assert_eq!(six.client_name, "HEADER_GC_LOGIN_SUCCESS3");
        assert!(
            !six.implemented_in_rust,
            "byte 6 has no server producer and must not claim a codec"
        );
        assert_eq!(gc_missing_codec_count(), 31);
    }
}
