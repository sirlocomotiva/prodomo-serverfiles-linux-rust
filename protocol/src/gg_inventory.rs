//! Machine-readable inventory of the legacy game-to-game packet headers.
//!
//! The byte values come from the global `HEADER_*` enum in
//! `server/server/game/packet.h`, and the registration set mirrors the
//! `CPacketInfoGG` table built in `server/server/game/packet_info.cpp`.
//! The DB server never sees these records: `server/server/db` contains no
//! `HEADER_GG_` enumerator, no `TPacketGG` type, and does not include the
//! game packet header at all, so the Rust `db-server` needs no GG decoder.
//!
//! Legacy game-to-game framing is one header byte followed by the registered
//! body, with no length prefix, no handle, and no padding, because the shared
//! `CInputProcessor` reads a single header byte and looks the total length up
//! in the table. That is a different socket and a different shape from the DB
//! peer's `[u8 header][u32 handle][u32 size]`, so the two tables must never be
//! shared.
//!
//! Every size in this table is a hand-summed packed x86 `sizeof` under the
//! legacy `#pragma pack(1)`, verified against the struct source. No size here
//! was produced by an automated layout guess.
/// A one-byte legacy game-to-game packet header.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GgHeader(u8);

impl GgHeader {
    /// Return the header's one-byte wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

impl From<GgHeader> for u8 {
    fn from(header: GgHeader) -> Self {
        header.0
    }
}
/// `HEADER_GG_LOGIN` from the legacy game-to-game protocol tables.
pub const HEADER_GG_LOGIN: GgHeader = GgHeader(0x01);
/// `HEADER_GG_LOGOUT` from the legacy game-to-game protocol tables.
pub const HEADER_GG_LOGOUT: GgHeader = GgHeader(0x02);
/// `HEADER_GG_RELAY` from the legacy game-to-game protocol tables.
pub const HEADER_GG_RELAY: GgHeader = GgHeader(0x03);
/// `HEADER_GG_NOTICE` from the legacy game-to-game protocol tables.
pub const HEADER_GG_NOTICE: GgHeader = GgHeader(0x04);
/// `HEADER_GG_SHUTDOWN` from the legacy game-to-game protocol tables.
pub const HEADER_GG_SHUTDOWN: GgHeader = GgHeader(0x05);
/// `HEADER_GG_GUILD` from the legacy game-to-game protocol tables.
pub const HEADER_GG_GUILD: GgHeader = GgHeader(0x06);
/// `HEADER_GG_DISCONNECT` from the legacy game-to-game protocol tables.
pub const HEADER_GG_DISCONNECT: GgHeader = GgHeader(0x07);
/// `HEADER_GG_SHOUT` from the legacy game-to-game protocol tables.
pub const HEADER_GG_SHOUT: GgHeader = GgHeader(0x08);
/// `HEADER_GG_SETUP` from the legacy game-to-game protocol tables.
pub const HEADER_GG_SETUP: GgHeader = GgHeader(0x09);
/// `HEADER_GG_MESSENGER_ADD` from the legacy game-to-game protocol tables.
pub const HEADER_GG_MESSENGER_ADD: GgHeader = GgHeader(0x0a);
/// `HEADER_GG_MESSENGER_REMOVE` from the legacy game-to-game protocol tables.
pub const HEADER_GG_MESSENGER_REMOVE: GgHeader = GgHeader(0x0b);
/// `HEADER_GG_FIND_POSITION` from the legacy game-to-game protocol tables.
pub const HEADER_GG_FIND_POSITION: GgHeader = GgHeader(0x0c);
/// `HEADER_GG_WARP_CHARACTER` from the legacy game-to-game protocol tables.
pub const HEADER_GG_WARP_CHARACTER: GgHeader = GgHeader(0x0d);
/// `HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX` from the legacy game-to-game protocol tables.
pub const HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX: GgHeader = GgHeader(0x0f);
/// `HEADER_GG_TRANSFER` from the legacy game-to-game protocol tables.
pub const HEADER_GG_TRANSFER: GgHeader = GgHeader(0x10);
/// `HEADER_GG_XMAS_WARP_SANTA` from the legacy game-to-game protocol tables.
pub const HEADER_GG_XMAS_WARP_SANTA: GgHeader = GgHeader(0x11);
/// `HEADER_GG_XMAS_WARP_SANTA_REPLY` from the legacy game-to-game protocol tables.
pub const HEADER_GG_XMAS_WARP_SANTA_REPLY: GgHeader = GgHeader(0x12);
/// `HEADER_GG_RELOAD_CRC_LIST` from the legacy game-to-game protocol tables.
pub const HEADER_GG_RELOAD_CRC_LIST: GgHeader = GgHeader(0x13);
/// `HEADER_GG_LOGIN_PING` from the legacy game-to-game protocol tables.
pub const HEADER_GG_LOGIN_PING: GgHeader = GgHeader(0x14);
/// `HEADER_GG_CHECK_CLIENT_VERSION` from the legacy game-to-game protocol tables.
pub const HEADER_GG_CHECK_CLIENT_VERSION: GgHeader = GgHeader(0x15);
/// `HEADER_GG_BLOCK_CHAT` from the legacy game-to-game protocol tables.
pub const HEADER_GG_BLOCK_CHAT: GgHeader = GgHeader(0x16);
/// `HEADER_GG_SIEGE` from the legacy game-to-game protocol tables.
pub const HEADER_GG_SIEGE: GgHeader = GgHeader(0x19);
/// `HEADER_GG_MONARCH_NOTICE` from the legacy game-to-game protocol tables.
pub const HEADER_GG_MONARCH_NOTICE: GgHeader = GgHeader(0x1a);
/// `HEADER_GG_MONARCH_TRANSFER` from the legacy game-to-game protocol tables.
pub const HEADER_GG_MONARCH_TRANSFER: GgHeader = GgHeader(0x1b);
/// `HEADER_GG_CHECK_AWAKENESS` from the legacy game-to-game protocol tables.
pub const HEADER_GG_CHECK_AWAKENESS: GgHeader = GgHeader(0x1d);
/// `HEADER_GG_BIG_NOTICE` from the legacy game-to-game protocol tables.
pub const HEADER_GG_BIG_NOTICE: GgHeader = GgHeader(0x1e);
/// `HEADER_GG_SWITCHBOT` from the legacy game-to-game protocol tables.
pub const HEADER_GG_SWITCHBOT: GgHeader = GgHeader(0x1f);
/// `HEADER_GG_EVENT_RELOAD` from the legacy game-to-game protocol tables.
pub const HEADER_GG_EVENT_RELOAD: GgHeader = GgHeader(0x20);
/// `HEADER_GG_EVENT` from the legacy game-to-game protocol tables.
pub const HEADER_GG_EVENT: GgHeader = GgHeader(0x21);
/// `HEADER_GG_EVENT_HIDE_AND_SEEK` from the legacy game-to-game protocol tables.
pub const HEADER_GG_EVENT_HIDE_AND_SEEK: GgHeader = GgHeader(0x22);
/// `HEADER_GG_REWARD_INFO` from the legacy game-to-game protocol tables.
pub const HEADER_GG_REWARD_INFO: GgHeader = GgHeader(0x2d);
/// `HEADER_GG_MULTI_FARM` from the legacy game-to-game protocol tables.
pub const HEADER_GG_MULTI_FARM: GgHeader = GgHeader(0x2e);
/// `HEADER_GG_LOCALE_NOTICE` from the legacy game-to-game protocol tables.
pub const HEADER_GG_LOCALE_NOTICE: GgHeader = GgHeader(0x2f);
/// `HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_RESULT` from the legacy game-to-game protocol tables.
pub const HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_RESULT: GgHeader = GgHeader(0x30);
/// `HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH` from the legacy game-to-game protocol tables.
pub const HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH: GgHeader = GgHeader(0x31);
/// `HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_UPDATE` from the legacy game-to-game protocol tables.
pub const HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_UPDATE: GgHeader = GgHeader(0x32);
/// How a legacy game-to-game record is shaped on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GgRecordShape {
    /// Every member is a scalar or a plain fixed char array, so the
    /// registered size is the whole frame.
    FixedWidth,
    /// The record embeds a nested legacy table struct.
    NestedStruct,
    /// The registered size is a fixed prefix and the frame continues with a
    /// tail whose length the prefix itself describes. The legacy `Analyze`
    /// step returns a non-zero extra length that the input processor adds to
    /// the table length, so the registered size is a minimum, not the frame.
    VariableLength,
}

/// One machine-readable legacy game-to-game packet-info entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyGgPacket {
    /// One-byte game-to-game protocol header.
    pub header: GgHeader,
    /// Legacy `HEADER_GG_*` enumerator name.
    pub legacy_name: &'static str,
    /// Packed C++ type whose `sizeof` supplies the base size, if any.
    ///
    /// `None` marks a record registered as `sizeof(BYTE)` whose handler never
    /// dereferences the body, so the single header byte is the whole frame.
    pub cpp_type: Option<&'static str>,
    /// Hand-verified packed x86 size used by the first `CPacketInfo` lookup.
    pub base_size: usize,
    /// How the record continues past `base_size`.
    pub shape: GgRecordShape,
    /// Whether the default legacy build registers and dispatches this record.
    pub active_in_default_build: bool,
    /// Whether a Rust codec for this record exists in the `protocol` crate.
    pub implemented_in_rust: bool,
    /// Source-derived note that a future codec must respect.
    pub note: &'static str,
}
/// The legacy game-to-game packet-info table in ascending header order.
pub const LEGACY_GG_PACKET_INVENTORY: &[LegacyGgPacket] = &[
    LegacyGgPacket {
        header: HEADER_GG_LOGIN,
        legacy_name: "HEADER_GG_LOGIN",
        cpp_type: Some("TPacketGGLogin"),
        base_size: 37,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_LOGOUT,
        legacy_name: "HEADER_GG_LOGOUT",
        cpp_type: Some("TPacketGGLogout"),
        base_size: 26,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_RELAY,
        legacy_name: "HEADER_GG_RELAY",
        cpp_type: Some("TPacketGGRelay"),
        base_size: 30,
        shape: GgRecordShape::VariableLength,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "The tail is an opaque game-to-client blob. Only a tail whose first byte is `HEADER_GC_WHISPER` is cast to a whisper record.",
    },
    LegacyGgPacket {
        header: HEADER_GG_NOTICE,
        legacy_name: "HEADER_GG_NOTICE",
        cpp_type: Some("TPacketGGNotice"),
        base_size: 5,
        shape: GgRecordShape::VariableLength,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "Producers set the size to `strlen + 1`, so the tail includes the NUL byte.",
    },
    LegacyGgPacket {
        header: HEADER_GG_SHUTDOWN,
        legacy_name: "HEADER_GG_SHUTDOWN",
        cpp_type: Some("TPacketGGShutdown"),
        base_size: 1,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_GUILD,
        legacy_name: "HEADER_GG_GUILD",
        cpp_type: Some("TPacketGGGuild"),
        base_size: 6,
        shape: GgRecordShape::VariableLength,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "The tail length is chosen by `bSubHeader`: sub-header 0 appends a 519-byte nested guild-chat record, sub-header 1 appends a bare 4-byte `int`, and any other value ends the frame at 6 bytes.",
    },
    LegacyGgPacket {
        header: HEADER_GG_DISCONNECT,
        legacy_name: "HEADER_GG_DISCONNECT",
        cpp_type: Some("TPacketGGDisconnect"),
        base_size: 32,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_SHOUT,
        legacy_name: "HEADER_GG_SHOUT",
        cpp_type: Some("TPacketGGShout"),
        base_size: 515,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_SETUP,
        legacy_name: "HEADER_GG_SETUP",
        cpp_type: Some("TPacketGGSetup"),
        base_size: 4,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: true,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_MESSENGER_ADD,
        legacy_name: "HEADER_GG_MESSENGER_ADD",
        cpp_type: Some("TPacketGGMessenger"),
        base_size: 51,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "Shares one body with header 11, so a Rust codec needs an explicit header field and no default.",
    },
    LegacyGgPacket {
        header: HEADER_GG_MESSENGER_REMOVE,
        legacy_name: "HEADER_GG_MESSENGER_REMOVE",
        cpp_type: Some("TPacketGGMessenger"),
        base_size: 51,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "Shares one body with header 10, so a Rust codec needs an explicit header field and no default.",
    },
    LegacyGgPacket {
        header: HEADER_GG_FIND_POSITION,
        legacy_name: "HEADER_GG_FIND_POSITION",
        cpp_type: Some("TPacketGGFindPosition"),
        base_size: 9,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: true,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_WARP_CHARACTER,
        legacy_name: "HEADER_GG_WARP_CHARACTER",
        cpp_type: Some("TPacketGGWarpCharacter"),
        base_size: 13,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: true,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX,
        legacy_name: "HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX",
        cpp_type: Some("TPacketGGGuildWarMapIndex"),
        base_size: 13,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: true,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_TRANSFER,
        legacy_name: "HEADER_GG_TRANSFER",
        cpp_type: Some("TPacketGGTransfer"),
        base_size: 34,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_XMAS_WARP_SANTA,
        legacy_name: "HEADER_GG_XMAS_WARP_SANTA",
        cpp_type: Some("TPacketGGXmasWarpSanta"),
        base_size: 6,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_XMAS_WARP_SANTA_REPLY,
        legacy_name: "HEADER_GG_XMAS_WARP_SANTA_REPLY",
        cpp_type: Some("TPacketGGXmasWarpSantaReply"),
        base_size: 2,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_RELOAD_CRC_LIST,
        legacy_name: "HEADER_GG_RELOAD_CRC_LIST",
        cpp_type: None,
        base_size: 1,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "Receive-only in this tree. The handler never reads the body, so the header byte is the whole frame, and no producer exists.",
    },
    LegacyGgPacket {
        header: HEADER_GG_LOGIN_PING,
        legacy_name: "HEADER_GG_LOGIN_PING",
        cpp_type: Some("TPacketGGLoginPing"),
        base_size: 32,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_CHECK_CLIENT_VERSION,
        legacy_name: "HEADER_GG_CHECK_CLIENT_VERSION",
        cpp_type: None,
        base_size: 1,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "Receive-only in this tree. The handler never reads the body, so the header byte is the whole frame, and no producer exists.",
    },
    LegacyGgPacket {
        header: HEADER_GG_BLOCK_CHAT,
        legacy_name: "HEADER_GG_BLOCK_CHAT",
        cpp_type: Some("TPacketGGBlockChat"),
        base_size: 34,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_SIEGE,
        legacy_name: "HEADER_GG_SIEGE",
        cpp_type: Some("TPacketGGSiege"),
        base_size: 3,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "The struct tag is `tag_GGSiege`, which is inverted relative to every other record in the family.",
    },
    LegacyGgPacket {
        header: HEADER_GG_MONARCH_NOTICE,
        legacy_name: "HEADER_GG_MONARCH_NOTICE",
        cpp_type: Some("TPacketGGMonarchNotice"),
        base_size: 6,
        shape: GgRecordShape::VariableLength,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "Variable record whose length word is a 32-bit `long`, unlike the private-shop search result.",
    },
    LegacyGgPacket {
        header: HEADER_GG_MONARCH_TRANSFER,
        legacy_name: "HEADER_GG_MONARCH_TRANSFER",
        cpp_type: Some("TPacketMonarchGGTransfer"),
        base_size: 13,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "The typedef name and the struct tag name are inverted relative to every other record in the family.",
    },
    LegacyGgPacket {
        header: HEADER_GG_CHECK_AWAKENESS,
        legacy_name: "HEADER_GG_CHECK_AWAKENESS",
        cpp_type: Some("TPacketGGCheckAwakeness"),
        base_size: 1,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "Declared, registered, and dispatched, but never written anywhere in the server tree.",
    },
    LegacyGgPacket {
        header: HEADER_GG_BIG_NOTICE,
        legacy_name: "HEADER_GG_BIG_NOTICE",
        cpp_type: Some("TPacketGGNotice"),
        base_size: 5,
        shape: GgRecordShape::VariableLength,
        active_in_default_build: false,
        implemented_in_rust: false,
        note: "Dead end to end. The enumerator, the registration, the dispatch case, and both producers are all behind `ENABLE_FULL_NOTICE`, which is defined nowhere in the server or client tree.",
    },
    LegacyGgPacket {
        header: HEADER_GG_SWITCHBOT,
        legacy_name: "HEADER_GG_SWITCHBOT",
        cpp_type: Some("TPacketGGSwitchbot"),
        base_size: 187,
        shape: GgRecordShape::NestedStruct,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "The deepest record in the family: the nested table is three levels deep and pulls in the typed item-attribute record. The header byte is set by the legacy constructor, not at the send site.",
    },
    LegacyGgPacket {
        header: HEADER_GG_EVENT_RELOAD,
        legacy_name: "HEADER_GG_EVENT_RELOAD",
        cpp_type: Some("TPacketGGReloadEvent"),
        base_size: 1,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_EVENT,
        legacy_name: "HEADER_GG_EVENT",
        cpp_type: Some("TPacketGGEvent"),
        base_size: 87,
        shape: GgRecordShape::NestedStruct,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "The nested `TEventTable` is also the DB boot event payload element, so one Rust table type can serve both.",
    },
    LegacyGgPacket {
        header: HEADER_GG_EVENT_HIDE_AND_SEEK,
        legacy_name: "HEADER_GG_EVENT_HIDE_AND_SEEK",
        cpp_type: Some("TPacketGGEventHideAndSeek"),
        base_size: 9,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_REWARD_INFO,
        legacy_name: "HEADER_GG_REWARD_INFO",
        cpp_type: Some("TPacketGGRewardInfo"),
        base_size: 2,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
    LegacyGgPacket {
        header: HEADER_GG_MULTI_FARM,
        legacy_name: "HEADER_GG_MULTI_FARM",
        cpp_type: Some("TPacketGGMultiFarm"),
        base_size: 57,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "The `size` field is a dead wire field that no producer or consumer reads; preserve the four bytes anyway. `subHeader` is a typed sub-header, but keep it an opaque `u8`.",
    },
    LegacyGgPacket {
        header: HEADER_GG_LOCALE_NOTICE,
        legacy_name: "HEADER_GG_LOCALE_NOTICE",
        cpp_type: Some("TPacketGGLocaleNotice"),
        base_size: 3080,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "The notice field is a two-dimensional fixed char array with no NUL-aware accessor in the legacy handler.",
    },
    LegacyGgPacket {
        header: HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_RESULT,
        legacy_name: "HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_RESULT",
        cpp_type: Some("TPacketGGPrivateShopItemSearchResult"),
        base_size: 7,
        shape: GgRecordShape::VariableLength,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "The length word is 16-bit here, unlike the other variable game-to-game records. A Rust codec must not use a 32-bit size for this record.",
    },
    LegacyGgPacket {
        header: HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH,
        legacy_name: "HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH",
        cpp_type: Some("TPacketGGPrivateShopItemSearch"),
        base_size: 94,
        shape: GgRecordShape::NestedStruct,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "The `bUseFilter` flag precedes the filter; the client-to-game twin places the filter first and has no header byte. Do not merge the two records.",
    },
    LegacyGgPacket {
        header: HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_UPDATE,
        legacy_name: "HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_UPDATE",
        cpp_type: Some("TPacketGGPrivateShopItemSearchUpdate"),
        base_size: 10,
        shape: GgRecordShape::FixedWidth,
        active_in_default_build: true,
        implemented_in_rust: false,
        note: "",
    },
];
/// Resolve a raw game-to-game header to its legacy packet-info entry.
///
/// Records that the default legacy build does not register return `None`.
#[must_use]
pub fn resolve_gg_packet(header: u8) -> Option<&'static LegacyGgPacket> {
    LEGACY_GG_PACKET_INVENTORY
        .iter()
        .find(|entry| entry.header.value() == header && entry.active_in_default_build)
}

/// Resolve the legacy `CPacketInfo` base size for a raw game-to-game header.
///
/// For a [`GgRecordShape::VariableLength`] record this is a fixed prefix, not
/// a complete frame length.
#[must_use]
pub fn resolve_gg_base_size(header: u8) -> Option<usize> {
    resolve_gg_packet(header).map(|entry| entry.base_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn table_matches_the_legacy_registration_set() {
        assert_eq!(LEGACY_GG_PACKET_INVENTORY.len(), 36);
        let names: HashSet<&str> = LEGACY_GG_PACKET_INVENTORY
            .iter()
            .map(|entry| entry.legacy_name)
            .collect();
        assert_eq!(names.len(), LEGACY_GG_PACKET_INVENTORY.len());
        for expected in [
            "HEADER_GG_LOGIN",
            "HEADER_GG_RELAY",
            "HEADER_GG_GUILD",
            "HEADER_GG_SETUP",
            "HEADER_GG_SWITCHBOT",
            "HEADER_GG_LOCALE_NOTICE",
            "HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_UPDATE",
        ] {
            assert!(names.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn no_two_game_to_game_headers_share_a_byte() {
        let mut seen = HashSet::new();
        for entry in LEGACY_GG_PACKET_INVENTORY {
            assert!(
                seen.insert(entry.header.value()),
                "duplicate byte for {}",
                entry.legacy_name
            );
        }
    }

    #[test]
    fn the_table_is_sorted_by_header_byte() {
        let bytes: Vec<u8> = LEGACY_GG_PACKET_INVENTORY
            .iter()
            .map(|entry| entry.header.value())
            .collect();
        let mut sorted = bytes.clone();
        sorted.sort_unstable();
        assert_eq!(bytes, sorted);
    }

    #[test]
    fn resolves_hand_verified_prefix_sizes() {
        assert_eq!(resolve_gg_base_size(HEADER_GG_LOGIN.value()), Some(37));
        assert_eq!(resolve_gg_base_size(HEADER_GG_SHUTDOWN.value()), Some(1));
        assert_eq!(resolve_gg_base_size(HEADER_GG_SHOUT.value()), Some(515));
        assert_eq!(resolve_gg_base_size(HEADER_GG_SETUP.value()), Some(4));
        assert_eq!(
            resolve_gg_base_size(HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX.value()),
            Some(13)
        );
        assert_eq!(
            resolve_gg_base_size(HEADER_GG_LOCALE_NOTICE.value()),
            Some(3080)
        );
        assert_eq!(resolve_gg_base_size(HEADER_GG_SWITCHBOT.value()), Some(187));
    }

    #[test]
    fn variable_records_expose_only_their_fixed_prefix() {
        for name in ["HEADER_GG_RELAY", "HEADER_GG_NOTICE", "HEADER_GG_GUILD"] {
            let entry = LEGACY_GG_PACKET_INVENTORY
                .iter()
                .find(|entry| entry.legacy_name == name)
                .expect("record present");
            assert_eq!(entry.shape, GgRecordShape::VariableLength);
        }
        assert_eq!(resolve_gg_base_size(HEADER_GG_RELAY.value()), Some(30));
        assert_eq!(resolve_gg_base_size(HEADER_GG_NOTICE.value()), Some(5));
        assert_eq!(resolve_gg_base_size(HEADER_GG_GUILD.value()), Some(6));
        assert_eq!(
            resolve_gg_base_size(HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH_RESULT.value()),
            Some(7)
        );
    }

    #[test]
    fn bodyless_records_carry_no_cpp_type() {
        for name in [
            "HEADER_GG_RELOAD_CRC_LIST",
            "HEADER_GG_CHECK_CLIENT_VERSION",
        ] {
            let entry = LEGACY_GG_PACKET_INVENTORY
                .iter()
                .find(|entry| entry.legacy_name == name)
                .expect("record present");
            assert_eq!(entry.cpp_type, None);
            assert_eq!(entry.base_size, 1);
        }
    }

    #[test]
    fn the_dead_big_notice_record_does_not_resolve() {
        assert_eq!(HEADER_GG_BIG_NOTICE.value(), 30);
        assert_eq!(resolve_gg_base_size(HEADER_GG_BIG_NOTICE.value()), None);
        let entry = LEGACY_GG_PACKET_INVENTORY
            .iter()
            .find(|entry| entry.header == HEADER_GG_BIG_NOTICE)
            .expect("record is documented even though it is inactive");
        assert!(!entry.active_in_default_build);
    }

    #[test]
    fn unassigned_header_bytes_do_not_resolve() {
        // 14, 23, 24, 28 and 51 are not registered in the legacy table.
        for byte in [0u8, 14, 23, 24, 28, 51, 255] {
            assert_eq!(resolve_gg_base_size(byte), None, "byte {byte}");
        }
    }

    #[test]
    fn implementation_status_matches_the_ported_codecs() {
        let done: Vec<&str> = LEGACY_GG_PACKET_INVENTORY
            .iter()
            .filter(|entry| entry.implemented_in_rust)
            .map(|entry| entry.legacy_name)
            .collect();
        assert_eq!(
            done,
            vec![
                "HEADER_GG_SETUP",
                "HEADER_GG_FIND_POSITION",
                "HEADER_GG_WARP_CHARACTER",
                "HEADER_GG_GUILD_WAR_ZONE_MAP_INDEX",
            ]
        );
    }
}
