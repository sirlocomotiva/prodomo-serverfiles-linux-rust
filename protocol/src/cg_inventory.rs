//! Machine-readable inventory of the legacy client-to-game packet headers.
//!
//! The byte values come from `server/server/game/packet.h`. The inventory
//! mirrors the `CPacketInfoCG` registrations in `packet_info.cpp`, including
//! the input processor's special case for a zero byte. Sizes are packed x86
//! `sizeof` values from the legacy `#pragma pack(1)` structures.
//!
//! A resolved size is only the first-pass `CPacketInfo` size. The legacy
//! `Analyze` step may append payload bytes for variable-length packets such as
//! chat, whisper, shop, guild, messenger, and private-shop messages.

/// A one-byte legacy client-to-game packet header.
///
/// This is a newtype rather than a Rust `enum` because the legacy tables give
/// both `HEADER_CG_CLIENT_VERSION2` and `HEADER_CG_GAYA_SYSTEM` the value
/// `0xf1`. The constants retain that wire-level alias.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CgHeader(u8);

impl CgHeader {
    /// Return the header's one-byte wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

impl From<CgHeader> for u8 {
    fn from(header: CgHeader) -> Self {
        header.0
    }
}

/// The zero byte consumed as a one-byte keepalive by the legacy input loop.
pub const CG_KEEP_ALIVE: CgHeader = CgHeader(0x00);

/// `HEADER_CG_HANDSHAKE` from the legacy client protocol tables.
pub const HEADER_CG_HANDSHAKE: CgHeader = CgHeader(0xff);

/// `HEADER_CG_PONG` from the legacy client protocol tables.
pub const HEADER_CG_PONG: CgHeader = CgHeader(0xfe);

/// `HEADER_CG_TIME_SYNC` from the legacy client protocol tables.
pub const HEADER_CG_TIME_SYNC: CgHeader = CgHeader(0xfc);

/// `HEADER_CG_KEY_AGREEMENT` from the legacy client protocol tables.
pub const HEADER_CG_KEY_AGREEMENT: CgHeader = CgHeader(0xfb);

/// `HEADER_CG_LOGIN` from the legacy client protocol tables.
pub const HEADER_CG_LOGIN: CgHeader = CgHeader(0x01);

/// `HEADER_CG_ATTACK` from the legacy client protocol tables.
pub const HEADER_CG_ATTACK: CgHeader = CgHeader(0x02);

/// `HEADER_CG_CHAT` from the legacy client protocol tables.
pub const HEADER_CG_CHAT: CgHeader = CgHeader(0x03);

/// `HEADER_CG_CHARACTER_CREATE` from the legacy client protocol tables.
pub const HEADER_CG_CHARACTER_CREATE: CgHeader = CgHeader(0x04);

/// `HEADER_CG_CHARACTER_DELETE` from the legacy client protocol tables.
pub const HEADER_CG_CHARACTER_DELETE: CgHeader = CgHeader(0x05);

/// `HEADER_CG_CHARACTER_SELECT` from the legacy client protocol tables.
pub const HEADER_CG_CHARACTER_SELECT: CgHeader = CgHeader(0x06);

/// `HEADER_CG_MOVE` from the legacy client protocol tables.
pub const HEADER_CG_MOVE: CgHeader = CgHeader(0x07);

/// `HEADER_CG_SYNC_POSITION` from the legacy client protocol tables.
pub const HEADER_CG_SYNC_POSITION: CgHeader = CgHeader(0x08);

/// `HEADER_CG_ENTERGAME` from the legacy client protocol tables.
pub const HEADER_CG_ENTERGAME: CgHeader = CgHeader(0x0a);

/// `HEADER_CG_ITEM_USE` from the legacy client protocol tables.
pub const HEADER_CG_ITEM_USE: CgHeader = CgHeader(0x0b);

/// `HEADER_CG_ITEM_DROP` from the legacy client protocol tables.
pub const HEADER_CG_ITEM_DROP: CgHeader = CgHeader(0x0c);

/// `HEADER_CG_ITEM_MOVE` from the legacy client protocol tables.
pub const HEADER_CG_ITEM_MOVE: CgHeader = CgHeader(0x0d);

/// `HEADER_CG_ITEM_PICKUP` from the legacy client protocol tables.
pub const HEADER_CG_ITEM_PICKUP: CgHeader = CgHeader(0x0f);

/// `HEADER_CG_QUICKSLOT_ADD` from the legacy client protocol tables.
pub const HEADER_CG_QUICKSLOT_ADD: CgHeader = CgHeader(0x10);

/// `HEADER_CG_QUICKSLOT_DEL` from the legacy client protocol tables.
pub const HEADER_CG_QUICKSLOT_DEL: CgHeader = CgHeader(0x11);

/// `HEADER_CG_QUICKSLOT_SWAP` from the legacy client protocol tables.
pub const HEADER_CG_QUICKSLOT_SWAP: CgHeader = CgHeader(0x12);

/// `HEADER_CG_WHISPER` from the legacy client protocol tables.
pub const HEADER_CG_WHISPER: CgHeader = CgHeader(0x13);

/// `HEADER_CG_ITEM_DROP2` from the legacy client protocol tables.
pub const HEADER_CG_ITEM_DROP2: CgHeader = CgHeader(0x14);

/// `HEADER_CG_ITEM_DESTROY` from the legacy client protocol tables.
pub const HEADER_CG_ITEM_DESTROY: CgHeader = CgHeader(0x15);

/// `HEADER_CG_ITEM_SELL` from the legacy client protocol tables.
pub const HEADER_CG_ITEM_SELL: CgHeader = CgHeader(0x16);

/// `HEADER_CG_ON_CLICK` from the legacy client protocol tables.
pub const HEADER_CG_ON_CLICK: CgHeader = CgHeader(0x1a);

/// `HEADER_CG_EXCHANGE` from the legacy client protocol tables.
pub const HEADER_CG_EXCHANGE: CgHeader = CgHeader(0x1b);

/// `HEADER_CG_CHARACTER_POSITION` from the legacy client protocol tables.
pub const HEADER_CG_CHARACTER_POSITION: CgHeader = CgHeader(0x1c);

/// `HEADER_CG_SCRIPT_ANSWER` from the legacy client protocol tables.
pub const HEADER_CG_SCRIPT_ANSWER: CgHeader = CgHeader(0x1d);

/// `HEADER_CG_QUEST_INPUT_STRING` from the legacy client protocol tables.
pub const HEADER_CG_QUEST_INPUT_STRING: CgHeader = CgHeader(0x1e);

/// `HEADER_CG_QUEST_CONFIRM` from the legacy client protocol tables.
pub const HEADER_CG_QUEST_CONFIRM: CgHeader = CgHeader(0x1f);

/// `HEADER_CG_REQUEST_EVENT_QUEST` from the legacy client protocol tables.
pub const HEADER_CG_REQUEST_EVENT_QUEST: CgHeader = CgHeader(0x20);

/// `HEADER_CG_SHOP` from the legacy client protocol tables.
pub const HEADER_CG_SHOP: CgHeader = CgHeader(0x32);

/// `HEADER_CG_FLY_TARGETING` from the legacy client protocol tables.
pub const HEADER_CG_FLY_TARGETING: CgHeader = CgHeader(0x33);

/// `HEADER_CG_USE_SKILL` from the legacy client protocol tables.
pub const HEADER_CG_USE_SKILL: CgHeader = CgHeader(0x34);

/// `HEADER_CG_ADD_FLY_TARGETING` from the legacy client protocol tables.
pub const HEADER_CG_ADD_FLY_TARGETING: CgHeader = CgHeader(0x35);

/// `HEADER_CG_SHOOT` from the legacy client protocol tables.
pub const HEADER_CG_SHOOT: CgHeader = CgHeader(0x36);

/// `HEADER_CG_MYSHOP` from the legacy client protocol tables.
pub const HEADER_CG_MYSHOP: CgHeader = CgHeader(0x37);

/// `HEADER_CG_ITEM_USE_TO_ITEM` from the legacy client protocol tables.
pub const HEADER_CG_ITEM_USE_TO_ITEM: CgHeader = CgHeader(0x3c);

/// `HEADER_CG_TARGET` from the legacy client protocol tables.
pub const HEADER_CG_TARGET: CgHeader = CgHeader(0x3d);

/// `HEADER_CG_TEXT` from the legacy client protocol tables.
pub const HEADER_CG_TEXT: CgHeader = CgHeader(0x40);

/// `HEADER_CG_WARP` from the legacy client protocol tables.
pub const HEADER_CG_WARP: CgHeader = CgHeader(0x41);

/// `HEADER_CG_SCRIPT_BUTTON` from the legacy client protocol tables.
pub const HEADER_CG_SCRIPT_BUTTON: CgHeader = CgHeader(0x42);

/// `HEADER_CG_MESSENGER` from the legacy client protocol tables.
pub const HEADER_CG_MESSENGER: CgHeader = CgHeader(0x43);

/// `HEADER_CG_MALL_CHECKOUT` from the legacy client protocol tables.
pub const HEADER_CG_MALL_CHECKOUT: CgHeader = CgHeader(0x45);

/// `HEADER_CG_SAFEBOX_CHECKIN` from the legacy client protocol tables.
pub const HEADER_CG_SAFEBOX_CHECKIN: CgHeader = CgHeader(0x46);

/// `HEADER_CG_SAFEBOX_CHECKOUT` from the legacy client protocol tables.
pub const HEADER_CG_SAFEBOX_CHECKOUT: CgHeader = CgHeader(0x47);

/// `HEADER_CG_PARTY_INVITE` from the legacy client protocol tables.
pub const HEADER_CG_PARTY_INVITE: CgHeader = CgHeader(0x48);

/// `HEADER_CG_PARTY_INVITE_ANSWER` from the legacy client protocol tables.
pub const HEADER_CG_PARTY_INVITE_ANSWER: CgHeader = CgHeader(0x49);

/// `HEADER_CG_PARTY_REMOVE` from the legacy client protocol tables.
pub const HEADER_CG_PARTY_REMOVE: CgHeader = CgHeader(0x4a);

/// `HEADER_CG_PARTY_SET_STATE` from the legacy client protocol tables.
pub const HEADER_CG_PARTY_SET_STATE: CgHeader = CgHeader(0x4b);

/// `HEADER_CG_PARTY_USE_SKILL` from the legacy client protocol tables.
pub const HEADER_CG_PARTY_USE_SKILL: CgHeader = CgHeader(0x4c);

/// `HEADER_CG_SAFEBOX_ITEM_MOVE` from the legacy client protocol tables.
pub const HEADER_CG_SAFEBOX_ITEM_MOVE: CgHeader = CgHeader(0x4d);

/// `HEADER_CG_PARTY_PARAMETER` from the legacy client protocol tables.
pub const HEADER_CG_PARTY_PARAMETER: CgHeader = CgHeader(0x4e);

/// `HEADER_CG_GUILD` from the legacy client protocol tables.
pub const HEADER_CG_GUILD: CgHeader = CgHeader(0x50);

/// `HEADER_CG_ANSWER_MAKE_GUILD` from the legacy client protocol tables.
pub const HEADER_CG_ANSWER_MAKE_GUILD: CgHeader = CgHeader(0x51);

/// `HEADER_CG_FISHING` from the legacy client protocol tables.
pub const HEADER_CG_FISHING: CgHeader = CgHeader(0x52);

/// `HEADER_CG_ITEM_GIVE` from the legacy client protocol tables.
pub const HEADER_CG_ITEM_GIVE: CgHeader = CgHeader(0x53);

/// `HEADER_CG_EMPIRE` from the legacy client protocol tables.
pub const HEADER_CG_EMPIRE: CgHeader = CgHeader(0x5a);

/// `HEADER_CG_REFINE` from the legacy client protocol tables.
pub const HEADER_CG_REFINE: CgHeader = CgHeader(0x60);

/// `HEADER_CG_MARK_LOGIN` from the legacy client protocol tables.
pub const HEADER_CG_MARK_LOGIN: CgHeader = CgHeader(0x64);

/// `HEADER_CG_MARK_CRCLIST` from the legacy client protocol tables.
pub const HEADER_CG_MARK_CRCLIST: CgHeader = CgHeader(0x65);

/// `HEADER_CG_MARK_UPLOAD` from the legacy client protocol tables.
pub const HEADER_CG_MARK_UPLOAD: CgHeader = CgHeader(0x66);

/// `HEADER_CG_MARK_IDXLIST` from the legacy client protocol tables.
pub const HEADER_CG_MARK_IDXLIST: CgHeader = CgHeader(0x68);

/// `HEADER_CG_HACK` from the legacy client protocol tables.
pub const HEADER_CG_HACK: CgHeader = CgHeader(0x69);

/// `HEADER_CG_CHANGE_NAME` from the legacy client protocol tables.
pub const HEADER_CG_CHANGE_NAME: CgHeader = CgHeader(0x6a);

/// `HEADER_CG_LOGIN2` from the legacy client protocol tables.
pub const HEADER_CG_LOGIN2: CgHeader = CgHeader(0x6d);

/// `HEADER_CG_DUNGEON` from the legacy client protocol tables.
pub const HEADER_CG_DUNGEON: CgHeader = CgHeader(0x6e);

/// `HEADER_CG_LOGIN3` from the legacy client protocol tables.
pub const HEADER_CG_LOGIN3: CgHeader = CgHeader(0x6f);

/// `HEADER_CG_GUILD_SYMBOL_UPLOAD` from the legacy client protocol tables.
pub const HEADER_CG_GUILD_SYMBOL_UPLOAD: CgHeader = CgHeader(0x70);

/// `HEADER_CG_SYMBOL_CRC` from the legacy client protocol tables.
pub const HEADER_CG_SYMBOL_CRC: CgHeader = CgHeader(0x71);

/// `HEADER_CG_SCRIPT_SELECT_ITEM` from the legacy client protocol tables.
pub const HEADER_CG_SCRIPT_SELECT_ITEM: CgHeader = CgHeader(0x72);

/// `HEADER_CG_REQUEST_EVENT_DATA` from the legacy client protocol tables.
pub const HEADER_CG_REQUEST_EVENT_DATA: CgHeader = CgHeader(0x75);

/// `HEADER_CG_SWITCHBOT` from the legacy client protocol tables.
pub const HEADER_CG_SWITCHBOT: CgHeader = CgHeader(0xab);

/// `HEADER_CG_AURA` from the legacy client protocol tables.
pub const HEADER_CG_AURA: CgHeader = CgHeader(0xac);

/// `HEADER_CG_DAILY_GIFT` from the legacy client protocol tables.
pub const HEADER_CG_DAILY_GIFT: CgHeader = CgHeader(0xb4);

/// `HEADER_CG_DRAGON_SOUL_REFINE` from the legacy client protocol tables.
pub const HEADER_CG_DRAGON_SOUL_REFINE: CgHeader = CgHeader(0xcd);

/// `HEADER_CG_STATE_CHECKER` from the legacy client protocol tables.
pub const HEADER_CG_STATE_CHECKER: CgHeader = CgHeader(0xce);

/// `HEADER_CG_CUBE_RENEWAL` from the legacy client protocol tables.
pub const HEADER_CG_CUBE_RENEWAL: CgHeader = CgHeader(0xdc);

/// `HEADER_CG_PRIVATE_SHOP` from the legacy client protocol tables.
pub const HEADER_CG_PRIVATE_SHOP: CgHeader = CgHeader(0xec);

/// `HEADER_CG_CHANGE_LANGUAGE` from the legacy client protocol tables.
pub const HEADER_CG_CHANGE_LANGUAGE: CgHeader = CgHeader(0xee);

/// `HEADER_CG_WHISPER_DETAILS` from the legacy client protocol tables.
pub const HEADER_CG_WHISPER_DETAILS: CgHeader = CgHeader(0xef);

/// `HEADER_CG_GAYA_SYSTEM` from the legacy client protocol tables.
pub const HEADER_CG_GAYA_SYSTEM: CgHeader = CgHeader(0xf1);

/// `HEADER_CG_CLIENT_VERSION` from the legacy client protocol tables.
pub const HEADER_CG_CLIENT_VERSION: CgHeader = CgHeader(0xfd);

/// `HEADER_CG_CLIENT_VERSION2` from the legacy client protocol tables.
pub const HEADER_CG_CLIENT_VERSION2: CgHeader = CgHeader(0xf1);

/// `HEADER_CG_TARGET_INFO_LOAD` from the legacy client protocol tables.
pub const HEADER_CG_TARGET_INFO_LOAD: CgHeader = CgHeader(0x3b);

/// `HEADER_CG_SASH` from the legacy client protocol tables.
pub const HEADER_CG_SASH: CgHeader = CgHeader(0xe6);

/// `HEADER_CG_CL` from the legacy client protocol tables.
pub const HEADER_CG_CL: CgHeader = CgHeader(0xe9);

/// `HEADER_CG_FISH_EVENT_SEND` from the legacy client protocol tables.
pub const HEADER_CG_FISH_EVENT_SEND: CgHeader = CgHeader(0xdb);

/// `HEADER_CG_REFINE_ELEMENT` from the legacy client protocol tables.
pub const HEADER_CG_REFINE_ELEMENT: CgHeader = CgHeader(0xe3);

/// `HEADER_CG_ATTR67_ADD` from the legacy client protocol tables.
pub const HEADER_CG_ATTR67_ADD: CgHeader = CgHeader(0xa9);

/// `HEADER_CG_BIOLOGIST` from the legacy client protocol tables.
pub const HEADER_CG_BIOLOGIST: CgHeader = CgHeader(0xaf);

/// `HEADER_CG_PREMIUM_PLAYERS` from the legacy client protocol tables.
pub const HEADER_CG_PREMIUM_PLAYERS: CgHeader = CgHeader(0xb0);

/// `HEADER_CG_INVENTORY_PROTECTED` from the legacy client protocol tables.
pub const HEADER_CG_INVENTORY_PROTECTED: CgHeader = CgHeader(0x90);

/// `HEADER_CG_WORLD_BOSS` from the legacy client protocol tables.
pub const HEADER_CG_WORLD_BOSS: CgHeader = CgHeader(0x94);

/// The legacy non-packet keepalive byte handled by `CInputProcessor::Process`.
pub const ENVANTER_BLACK: CgHeader = CgHeader(0xe2);

/// How a legacy packet-info entry behaves in the default server build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgPacketRegistration {
    /// Header zero is handled directly by `CInputProcessor::Process`.
    InputSpecialCase,
    /// The entry is present in the default `CPacketInfoCG` map.
    Registered,
    /// A preceding `Set` call registered the same byte first.
    Shadowed,
    /// The registration is behind a disabled legacy build feature.
    Conditional,
}

impl CgPacketRegistration {
    /// Return whether the default legacy input processor resolves this entry.
    #[must_use]
    pub const fn is_resolvable(self) -> bool {
        matches!(self, Self::InputSpecialCase | Self::Registered)
    }
}

/// One machine-readable legacy client packet-info entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCgPacket {
    /// One-byte client-to-game protocol header.
    pub header: CgHeader,
    /// Name passed to `CPacketInfo::Set` in the legacy source.
    pub legacy_name: &'static str,
    /// Packed C++ type whose `sizeof` supplies the base size, if any.
    pub cpp_type: Option<&'static str>,
    /// Packed x86 size used by the first `CPacketInfo` lookup.
    pub base_size: usize,
    /// Whether this source entry is active in the default legacy build.
    pub registration: CgPacketRegistration,
}

/// The legacy client packet-info table in source registration order.
///
/// The table includes the input-loop keepalive special case, the default
/// `CPacketInfoCG` entries, the shadowed Gaya entry, and the key-agreement
/// entry whose feature is disabled in the default build.
pub const LEGACY_CG_PACKET_INVENTORY: &[LegacyCgPacket] = &[
    LegacyCgPacket {
        header: CG_KEEP_ALIVE,
        legacy_name: "KeepAlive",
        cpp_type: None,
        base_size: 1,
        registration: CgPacketRegistration::InputSpecialCase,
    },
    LegacyCgPacket {
        header: HEADER_CG_TEXT,
        legacy_name: "Text",
        cpp_type: Some("TPacketCGText"),
        base_size: 1,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_HANDSHAKE,
        legacy_name: "Handshake",
        cpp_type: Some("TPacketCGHandshake"),
        base_size: 13,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_TIME_SYNC,
        legacy_name: "TimeSync",
        cpp_type: Some("TPacketCGHandshake"),
        base_size: 13,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_MARK_LOGIN,
        legacy_name: "MarkLogin",
        cpp_type: Some("TPacketCGMarkLogin"),
        base_size: 9,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_MARK_IDXLIST,
        legacy_name: "MarkIdxList",
        cpp_type: Some("TPacketCGMarkIDXList"),
        base_size: 1,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_MARK_CRCLIST,
        legacy_name: "MarkCrcList",
        cpp_type: Some("TPacketCGMarkCRCList"),
        base_size: 322,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_MARK_UPLOAD,
        legacy_name: "MarkUpload",
        cpp_type: Some("TPacketCGMarkUpload"),
        base_size: 773,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_GUILD_SYMBOL_UPLOAD,
        legacy_name: "SymbolUpload",
        cpp_type: Some("TPacketCGGuildSymbolUpload"),
        base_size: 7,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SYMBOL_CRC,
        legacy_name: "SymbolCRC",
        cpp_type: Some("TPacketCGSymbolCRC"),
        base_size: 13,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_LOGIN,
        legacy_name: "Login",
        cpp_type: Some("TPacketCGLogin"),
        base_size: 49,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_LOGIN2,
        legacy_name: "Login2",
        cpp_type: Some("TPacketCGLogin2"),
        base_size: 52,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_LOGIN3,
        legacy_name: "Login3",
        cpp_type: Some("TPacketCGLogin3"),
        base_size: 66,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ATTACK,
        legacy_name: "Attack",
        cpp_type: Some("TPacketCGAttack"),
        base_size: 8,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CHAT,
        legacy_name: "Chat",
        cpp_type: Some("TPacketCGChat"),
        base_size: 4,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_WHISPER,
        legacy_name: "Whisper",
        cpp_type: Some("TPacketCGWhisper"),
        base_size: 28,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CHARACTER_SELECT,
        legacy_name: "Select",
        cpp_type: Some("TPacketCGPlayerSelect"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CHARACTER_CREATE,
        legacy_name: "Create",
        cpp_type: Some("TPacketCGPlayerCreate"),
        base_size: 34,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CHARACTER_DELETE,
        legacy_name: "Delete",
        cpp_type: Some("TPacketCGPlayerDelete"),
        base_size: 10,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ENTERGAME,
        legacy_name: "EnterGame",
        cpp_type: Some("TPacketCGEnterGame"),
        base_size: 1,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ITEM_USE,
        legacy_name: "ItemUse",
        cpp_type: Some("TPacketCGItemUse"),
        base_size: 4,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ITEM_DROP,
        legacy_name: "ItemDrop",
        cpp_type: Some("TPacketCGItemDrop"),
        base_size: 8,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ITEM_DROP2,
        legacy_name: "ItemDrop2",
        cpp_type: Some("TPacketCGItemDrop2"),
        base_size: 10,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ITEM_DESTROY,
        legacy_name: "ItemDestroy",
        cpp_type: Some("TPacketCGItemDestroy"),
        base_size: 4,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ITEM_MOVE,
        legacy_name: "ItemMove",
        cpp_type: Some("TPacketCGItemMove"),
        base_size: 9,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: ENVANTER_BLACK,
        legacy_name: "InventoryExpansion",
        cpp_type: Some("TPacketCGEnvanter"),
        base_size: 1,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_INVENTORY_PROTECTED,
        legacy_name: "RecvActivateProtectedSystem",
        cpp_type: Some("TPacketCGInventoryProtected"),
        base_size: 17,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ITEM_PICKUP,
        legacy_name: "ItemPickup",
        cpp_type: Some("TPacketCGItemPickup"),
        base_size: 5,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_QUICKSLOT_ADD,
        legacy_name: "QuickslotAdd",
        cpp_type: Some("TPacketCGQuickslotAdd"),
        base_size: 4,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_QUICKSLOT_DEL,
        legacy_name: "QuickslotDel",
        cpp_type: Some("TPacketCGQuickslotDel"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_QUICKSLOT_SWAP,
        legacy_name: "QuickslotSwap",
        cpp_type: Some("TPacketCGQuickslotSwap"),
        base_size: 3,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SHOP,
        legacy_name: "Shop",
        cpp_type: Some("TPacketCGShop"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ON_CLICK,
        legacy_name: "OnClick",
        cpp_type: Some("TPacketCGOnClick"),
        base_size: 5,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_EXCHANGE,
        legacy_name: "Exchange",
        cpp_type: Some("TPacketCGExchange"),
        base_size: 14,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CHARACTER_POSITION,
        legacy_name: "Position",
        cpp_type: Some("TPacketCGPosition"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SCRIPT_ANSWER,
        legacy_name: "ScriptAnswer",
        cpp_type: Some("TPacketCGScriptAnswer"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SCRIPT_BUTTON,
        legacy_name: "ScriptButton",
        cpp_type: Some("TPacketCGScriptButton"),
        base_size: 5,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_QUEST_INPUT_STRING,
        legacy_name: "QuestInputString",
        cpp_type: Some("TPacketCGQuestInputString"),
        base_size: 66,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_QUEST_CONFIRM,
        legacy_name: "QuestConfirm",
        cpp_type: Some("TPacketCGQuestConfirm"),
        base_size: 6,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_MOVE,
        legacy_name: "Move",
        cpp_type: Some("TPacketCGMove"),
        base_size: 16,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SYNC_POSITION,
        legacy_name: "SyncPosition",
        cpp_type: Some("TPacketCGSyncPosition"),
        base_size: 3,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_FLY_TARGETING,
        legacy_name: "FlyTarget",
        cpp_type: Some("TPacketCGFlyTargeting"),
        base_size: 13,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ADD_FLY_TARGETING,
        legacy_name: "AddFlyTarget",
        cpp_type: Some("TPacketCGFlyTargeting"),
        base_size: 13,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SHOOT,
        legacy_name: "Shoot",
        cpp_type: Some("TPacketCGShoot"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_USE_SKILL,
        legacy_name: "UseSkill",
        cpp_type: Some("TPacketCGUseSkill"),
        base_size: 9,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ITEM_USE_TO_ITEM,
        legacy_name: "UseItemToItem",
        cpp_type: Some("TPacketCGItemUseToItem"),
        base_size: 7,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_TARGET,
        legacy_name: "Target",
        cpp_type: Some("TPacketCGTarget"),
        base_size: 5,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_WARP,
        legacy_name: "Warp",
        cpp_type: Some("TPacketCGWarp"),
        base_size: 1,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_MESSENGER,
        legacy_name: "Messenger",
        cpp_type: Some("TPacketCGMessenger"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_PARTY_REMOVE,
        legacy_name: "PartyRemove",
        cpp_type: Some("TPacketCGPartyRemove"),
        base_size: 5,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_PARTY_INVITE,
        legacy_name: "PartyInvite",
        cpp_type: Some("TPacketCGPartyInvite"),
        base_size: 5,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_PARTY_INVITE_ANSWER,
        legacy_name: "PartyInviteAnswer",
        cpp_type: Some("TPacketCGPartyInviteAnswer"),
        base_size: 6,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_PARTY_SET_STATE,
        legacy_name: "PartySetState",
        cpp_type: Some("TPacketCGPartySetState"),
        base_size: 7,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_PARTY_USE_SKILL,
        legacy_name: "PartyUseSkill",
        cpp_type: Some("TPacketCGPartyUseSkill"),
        base_size: 6,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_PARTY_PARAMETER,
        legacy_name: "PartyParam",
        cpp_type: Some("TPacketCGPartyParameter"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_EMPIRE,
        legacy_name: "Empire",
        cpp_type: Some("TPacketCGEmpire"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SAFEBOX_CHECKOUT,
        legacy_name: "SafeboxCheckout",
        cpp_type: Some("TPacketCGSafeboxCheckout"),
        base_size: 8,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SAFEBOX_CHECKIN,
        legacy_name: "SafeboxCheckin",
        cpp_type: Some("TPacketCGSafeboxCheckin"),
        base_size: 8,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_PREMIUM_PLAYERS,
        legacy_name: "RecvPremiumPlayersPacket",
        cpp_type: Some("TPacketCGPremiumPlayers"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_BIOLOGIST,
        legacy_name: "RecvBiologistPacket",
        cpp_type: Some("TPacketCGBiologist"),
        base_size: 6,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SAFEBOX_ITEM_MOVE,
        legacy_name: "SafeboxItemMove",
        cpp_type: Some("TPacketCGItemMove"),
        base_size: 9,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_GUILD,
        legacy_name: "Guild",
        cpp_type: Some("TPacketCGGuild"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ANSWER_MAKE_GUILD,
        legacy_name: "AnswerMakeGuild",
        cpp_type: Some("TPacketCGAnswerMakeGuild"),
        base_size: 14,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_FISHING,
        legacy_name: "Fishing",
        cpp_type: Some("TPacketCGFishing"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ITEM_GIVE,
        legacy_name: "ItemGive",
        cpp_type: Some("TPacketCGGiveItem"),
        base_size: 9,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_HACK,
        legacy_name: "Hack",
        cpp_type: Some("TPacketCGHack"),
        base_size: 257,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_MYSHOP,
        legacy_name: "MyShop",
        cpp_type: Some("TPacketCGMyShop"),
        base_size: 35,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_REFINE,
        legacy_name: "Refine",
        cpp_type: Some("TPacketCGRefine"),
        base_size: 3,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CHANGE_NAME,
        legacy_name: "ChangeName",
        cpp_type: Some("TPacketCGChangeName"),
        base_size: 27,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CLIENT_VERSION,
        legacy_name: "Version",
        cpp_type: Some("TPacketCGClientVersion"),
        base_size: 67,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CLIENT_VERSION2,
        legacy_name: "Version",
        cpp_type: Some("TPacketCGClientVersion2"),
        base_size: 67,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_PONG,
        legacy_name: "Pong",
        cpp_type: Some("BYTE"),
        base_size: 1,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_MALL_CHECKOUT,
        legacy_name: "MallCheckout",
        cpp_type: Some("TPacketCGSafeboxCheckout"),
        base_size: 8,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SCRIPT_SELECT_ITEM,
        legacy_name: "ScriptSelectItem",
        cpp_type: Some("TPacketCGScriptSelectItem"),
        base_size: 5,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_DRAGON_SOUL_REFINE,
        legacy_name: "DragonSoulRefine",
        cpp_type: Some("TPacketCGDragonSoulRefine"),
        base_size: 47,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SWITCHBOT,
        legacy_name: "Switchbot",
        cpp_type: Some("TPacketGCSwitchbot"),
        base_size: 7,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_STATE_CHECKER,
        legacy_name: "ServerStateCheck",
        cpp_type: Some("BYTE"),
        base_size: 1,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CHANGE_LANGUAGE,
        legacy_name: "ChangeLanguage",
        cpp_type: Some("TPacketChangeLanguage"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_GAYA_SYSTEM,
        legacy_name: "GayaSystemSend",
        cpp_type: Some("TPacketCGGayaSystem"),
        base_size: 6,
        registration: CgPacketRegistration::Shadowed,
    },
    LegacyCgPacket {
        header: HEADER_CG_WHISPER_DETAILS,
        legacy_name: "WhisperDetails",
        cpp_type: Some("TPacketCGWhisperDetails"),
        base_size: 26,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_DAILY_GIFT,
        legacy_name: "DailyGift",
        cpp_type: Some("TPacketCGDailyGift"),
        base_size: 3,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_ATTR67_ADD,
        legacy_name: "Attr67Add",
        cpp_type: Some("TPacketCGAttr67Add"),
        base_size: 8,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_FISH_EVENT_SEND,
        legacy_name: "FishEvent",
        cpp_type: Some("TPacketCGFishEvent"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CUBE_RENEWAL,
        legacy_name: "CubeRenewalSend",
        cpp_type: Some("TPacketCGCubeRenewalSend"),
        base_size: 14,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_CL,
        legacy_name: "ChangeLook",
        cpp_type: Some("TPacketChangeLook"),
        base_size: 10,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_SASH,
        legacy_name: "Sash",
        cpp_type: Some("TPacketSash"),
        base_size: 23,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_TARGET_INFO_LOAD,
        legacy_name: "TargetInfoLoad",
        cpp_type: Some("TPacketCGTargetInfoLoad"),
        base_size: 5,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_AURA,
        legacy_name: "Aura",
        cpp_type: Some("TPacketCGAura"),
        base_size: 4,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_REFINE_ELEMENT,
        legacy_name: "RefineElement",
        cpp_type: Some("TPacketCGRefineElement"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_PRIVATE_SHOP,
        legacy_name: "PrivateShop",
        cpp_type: Some("TPacketCGPrivateShop"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_REQUEST_EVENT_QUEST,
        legacy_name: "RequestEventQuest",
        cpp_type: Some("TPacketCGRequestEventQuest"),
        base_size: 66,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_REQUEST_EVENT_DATA,
        legacy_name: "EventRequest",
        cpp_type: Some("TPacketCGRequestEventData"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_WORLD_BOSS,
        legacy_name: "WorldBoss",
        cpp_type: Some("TPacketCGWorldBoss"),
        base_size: 2,
        registration: CgPacketRegistration::Registered,
    },
    LegacyCgPacket {
        header: HEADER_CG_KEY_AGREEMENT,
        legacy_name: "KeyAgreement",
        cpp_type: Some("TPacketKeyAgreement"),
        base_size: 261,
        registration: CgPacketRegistration::Conditional,
    },
];

/// Headers declared by `packet.h` but absent from `CPacketInfoCG`.
///
/// These values deliberately do not resolve. In particular, the dungeon
/// header has no `Set` call, and the optional item-sell declaration has no
/// `Set` call even when its declaration feature is enabled.
pub const LEGACY_CG_DECLARED_BUT_UNREGISTERED_HEADERS: &[CgHeader] =
    &[HEADER_CG_ITEM_SELL, HEADER_CG_DUNGEON];

/// Resolve a raw client header to its legacy packet-info entry.
///
/// Header zero resolves to its one-byte keepalive special case. Unknown
/// headers and entries disabled or shadowed in the default legacy build
/// return `None`.
#[must_use]
pub fn resolve_cg_packet(header: u8) -> Option<&'static LegacyCgPacket> {
    LEGACY_CG_PACKET_INVENTORY
        .iter()
        .find(|entry| entry.header.value() == header && entry.registration.is_resolvable())
}

/// Resolve the legacy `CPacketInfo` base size for a raw client header.
///
/// This is the size consumed before `CInputMain::Analyze` may add variable
/// payload bytes. It is therefore not necessarily a complete frame length.
#[must_use]
pub fn resolve_cg_base_size(header: u8) -> Option<usize> {
    resolve_cg_packet(header).map(|entry| entry.base_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn resolves_representative_legacy_sizes() {
        assert_eq!(resolve_cg_base_size(CG_KEEP_ALIVE.value()), Some(1));
        assert_eq!(resolve_cg_base_size(HEADER_CG_LOGIN.value()), Some(49));
        assert_eq!(resolve_cg_base_size(HEADER_CG_MOVE.value()), Some(16));
        assert_eq!(
            resolve_cg_base_size(HEADER_CG_MARK_UPLOAD.value()),
            Some(773)
        );
        assert_eq!(resolve_cg_base_size(HEADER_CG_HACK.value()), Some(257));
        assert_eq!(resolve_cg_base_size(HEADER_CG_HANDSHAKE.value()), Some(13));
    }

    #[test]
    fn variable_packets_resolve_their_legacy_prefix_size() {
        assert_eq!(resolve_cg_base_size(HEADER_CG_CHAT.value()), Some(4));
        assert_eq!(resolve_cg_base_size(HEADER_CG_WHISPER.value()), Some(28));
        assert_eq!(
            resolve_cg_base_size(HEADER_CG_SYNC_POSITION.value()),
            Some(3)
        );
        assert_eq!(resolve_cg_base_size(HEADER_CG_SHOP.value()), Some(2));
        assert_eq!(resolve_cg_base_size(HEADER_CG_MESSENGER.value()), Some(2));
        assert_eq!(resolve_cg_base_size(HEADER_CG_GUILD.value()), Some(2));
        assert_eq!(
            resolve_cg_base_size(HEADER_CG_PRIVATE_SHOP.value()),
            Some(2)
        );
    }

    #[test]
    fn unknown_and_unregistered_headers_do_not_resolve() {
        assert_eq!(resolve_cg_base_size(0x99), None);
        assert_eq!(resolve_cg_base_size(HEADER_CG_DUNGEON.value()), None);
        assert_eq!(resolve_cg_base_size(HEADER_CG_ITEM_SELL.value()), None);
        assert_eq!(LEGACY_CG_DECLARED_BUT_UNREGISTERED_HEADERS.len(), 2);
    }

    #[test]
    fn first_registration_wins_for_the_legacy_header_collision() {
        assert_eq!(HEADER_CG_CLIENT_VERSION2, HEADER_CG_GAYA_SYSTEM);
        let resolved = resolve_cg_packet(HEADER_CG_CLIENT_VERSION2.value()).unwrap();
        assert_eq!(resolved.legacy_name, "Version");
        assert_eq!(resolved.base_size, 67);

        let gaya = LEGACY_CG_PACKET_INVENTORY
            .iter()
            .find(|entry| {
                entry.header == HEADER_CG_GAYA_SYSTEM
                    && entry.registration == CgPacketRegistration::Shadowed
            })
            .unwrap();
        assert_eq!(gaya.legacy_name, "GayaSystemSend");
        assert_eq!(gaya.base_size, 6);
    }

    #[test]
    fn disabled_key_agreement_remains_in_the_source_inventory() {
        assert_eq!(resolve_cg_base_size(HEADER_CG_KEY_AGREEMENT.value()), None);
        let key_agreement = LEGACY_CG_PACKET_INVENTORY
            .iter()
            .find(|entry| entry.header == HEADER_CG_KEY_AGREEMENT)
            .unwrap();
        assert_eq!(
            key_agreement.registration,
            CgPacketRegistration::Conditional
        );
        assert_eq!(key_agreement.base_size, 261);
    }

    #[test]
    fn every_resolvable_inventory_header_is_unique_and_nonzero() {
        assert_eq!(LEGACY_CG_PACKET_INVENTORY.len(), 94);
        assert_eq!(
            LEGACY_CG_PACKET_INVENTORY
                .iter()
                .filter(|entry| entry.registration == CgPacketRegistration::Shadowed)
                .count(),
            1
        );
        assert_eq!(
            LEGACY_CG_PACKET_INVENTORY
                .iter()
                .filter(|entry| entry.registration == CgPacketRegistration::Conditional)
                .count(),
            1
        );

        let mut headers = HashSet::new();
        for entry in LEGACY_CG_PACKET_INVENTORY
            .iter()
            .filter(|entry| entry.registration.is_resolvable())
        {
            assert!(entry.base_size > 0);
            assert!(
                headers.insert(entry.header.value()),
                "duplicate resolvable header {:#04x}",
                entry.header.value()
            );
            assert_eq!(
                resolve_cg_base_size(entry.header.value()),
                Some(entry.base_size)
            );
        }
        assert_eq!(headers.len(), 92);
    }
}
