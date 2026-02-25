//! Feature flags documentation ported from C++ prodomodefines.h
//!
//! These were originally #define preprocessor flags that enabled/disabled
//! code sections. Documented here for reference when implementing features.

// ============================================================================
// General Features
// ============================================================================

/// Guild features enabled
pub const ENABLE_D_NJGUILD: bool = true;

/// New stuff from martysama
pub const ENABLE_NEWSTUFF: bool = true;

/// 3306 port security
pub const ENABLE_PORT_SECURITY: bool = true;

/// Refactored belt inventory
pub const ENABLE_BELT_INVENTORY_EX: bool = true;

/// Sash / Acce system
pub const SASH_SYSTEM: bool = true;

/// Transmutation / Change look system
pub const CHANGELOOK_SYSTEM: bool = true;

/// Quest renewal
pub const QUEST_RENEWAL: bool = true;

/// Cube renewal
pub const ENABLE_CUBE_RENEWAL_WORLDARD: bool = true;

/// Pickup slot effect
pub const BL_ENABLE_PICKUP_ITEM_EFFECT: bool = true;

/// Fish event (Jigsaw event)
pub const ENABLE_FISH_EVENT: bool = true;

/// Tradable icon
pub const WJ_ENABLE_TRADABLE_ICON: bool = true;

/// New exchange window
pub const NEW_EXCHANGE_WINDOW: bool = true;

/// Dragon soul mythic alchemy set bonus
pub const ENABLE_DRAGONSOUL_ALCHEMY_PLUS: bool = true;

/// Refine element (fire/ice/lightning/darkness/earth/wind)
pub const ENABLE_REFINE_ELEMENT: bool = true;

/// Pet costume slot
pub const ENABLE_PET_COSTUME_SYSTEM: bool = true;

/// Aura system
pub const AURA_SYSTEM: bool = true;

/// 6th and 7th attribute slots
pub const ATTR_6TH_7TH: bool = true;

/// Locale string renewal & multi-language adaptation
pub const LOCALE_STRING_RENEWAL: bool = true;

/// New set bonus (costume/sash/aura/item)
pub const NEW_SET_BONUS: bool = true;

/// New bonus system (metin/boss)
pub const NEW_BONUS: bool = true;

/// Proto new bonuses (percentage-based)
pub const BONUS_PCT: bool = true;

/// Ship defense (Hydra dungeon)
pub const SHIP_DEFENSE: bool = true;

/// Version 1.6.2 features
pub const VERSION_162: bool = true;

/// Healing skill vnum (for version 162)
pub const HEALING_SKILL_VNUM: u32 = 265;

/// Dungeon for guild (Meley Lair)
pub const DUNGEON_FOR_GUILD: bool = true;

/// Meley Lair dungeon
pub const MELEY_LAIR_DUNGEON: bool = true;

/// Destroy infinite statues GM command
pub const DESTROY_INFINITE_STATUES_GM: bool = true;

/// Laser effect at 75% HP
pub const LASER_EFFECT_ON_75HP: bool = true;

/// Laser effect at 50% HP
pub const LASER_EFFECT_ON_50HP: bool = true;

/// Gaya currency system
pub const ENABLE_GAYA_SYSTEM: bool = true;

/// Conqueror level system (level/map/attr/items/bonuses)
pub const CONQUEROR_LEVEL: bool = true;

/// Glove & Yohara system
pub const ENABLE_GLOVE_SYSTEM: bool = true;

/// Item attribute table for gloves
pub const ENABLE_GLOVE_ITEM_ATTR: bool = true;

/// Element on target (17.5 feature)
pub const ELEMENT_TARGET: bool = true;

// ============================================================================
// Inventory Features
// ============================================================================

/// Extended inventory system
pub const ENABLE_EXTEND_INVEN_SYSTEM: bool = true;

/// Extended safebox
pub const EXTENDED_SAFEBOX: bool = true;

/// Custom inventory system
pub const ENABLE_CUSTOM_INVENTORY: bool = true;

// ============================================================================
// Bug Fixes & Improvements
// ============================================================================

/// Flood protection
pub const ENABLE_FLOOD_PRETECTION: bool = true;

/// Fix change sex without relog
pub const FIX_CHANGE_SEX_WITHOUT_RELOG: bool = true;

/// Fly fix
pub const ENABLE_FLY_FIX: bool = true;

/// Extended reload commands
pub const ENABLE_EXTENDED_RELOAD: bool = true;

/// Boss kick into walls fix
pub const ENABLE_BOSS_KICK_INTO_WALLS_FIX: bool = true;

/// Kick multi IP in OX event
pub const ENABLE_KICK_MULTI_IP_OX: bool = true;

/// Header 100 fix
pub const ENABLE_HEADER_100_FIX: bool = true;

/// Update level fix
pub const FIX_UPDATE_LEVEL: bool = true;

/// Extended sockets (6 sockets)
pub const ENABLE_EXTENDED_SOCKETS: bool = true;

/// Update alignment fix
pub const FIX_UPDATE_ALIGNMENT: bool = true;

/// Costume over normal costume fix
pub const FIX_COSTUM_NUNTA_PESTE_COSTUM_NORMAL: bool = true;

/// Block mob in safezone fix
pub const FIX_BLOCK_MOB_SAFEZONE: bool = true;

/// Breasla la schimbare regat fix
pub const FIX_BREASLA_LA_SCHIMBARE_REGAT: bool = true;

/// Update playtime and items fix
pub const FIX_UPDATE_PLAYTIME_AND_ITEMS: bool = true;

/// Delete friend refresh fix
pub const FIX_DELETE_FRIEND_REFRESH: bool = true;

/// Info refine dragonsoul fix
pub const FIX_INFO_REFINE_DRAGONSOUL: bool = true;

/// Dungeon party fix
pub const FIX_DUNGEON_PARTY: bool = true;

/// Read etc drop item file by vnum fix
pub const ENABLE_FIX_READ_ETC_DROP_ITEM_FILE_BY_VNUM: bool = true;

/// Exploit quest fix
pub const FIX_EXPLOIT_QUEST: bool = true;

/// Flush at shutdown
pub const FLUSH_AT_SHUTDOWN: bool = true;

/// Select empire phase fix
pub const FIX_SELECT_EMPIRE_PHASE: bool = true;

/// EXP group fix
pub const FIX_EXP_GRUP: bool = true;

/// Items type 33 fix
pub const FIX_ITEMS_TYPE_33: bool = true;

/// Kick hack fix
pub const FIX_KICK_HACK: bool = true;

/// Count monster fix
pub const ENABLE_COUNT_MONSTER_FIX: bool = true;

/// Dungeon notice fix
pub const ENABLE_DUNGEON_NOTICE_FIX: bool = true;

/// Load mobs with mount fix
pub const FIX_LOAD_MOBS_WITH_MOUNT: bool = true;

/// Nivel cal fix
pub const FIX_NIVEL_CAL: bool = true;

/// PC select quest fix
pub const FIX_PC_SELECT_QUEST: bool = true;

/// Quick slot fix
pub const ENABLE_FIX_QUICK_SLOT: bool = true;

/// Secondary skill fix
pub const FIX_SECONDARY_SKILL: bool = true;

/// Timer event fix
pub const FIX_TIMER_EVENT: bool = true;

/// Destroy guild if war is active fix
pub const ENABLE_DESTROY_GUILD_IF_WAR_IS_ACTIVE_FIX: bool = true;

/// Aura tais fara arma fix
pub const FIX_BUG_AURA_TAIS_FARA_ARMA: bool = true;

/// HP group fix
pub const FIX_HP_GROUP: bool = true;

/// Comanda razboi fix
pub const FIX_COMANDA_RAZBOI: bool = true;

/// Campfire fix
pub const FIX_CAMPFIRE: bool = true;

/// Campfire fix timeout (seconds)
pub const CAMPFIRE_FIX_SEC: u32 = 60;

/// Bug immune fix
pub const FIX_BUG_IMMUNE: bool = true;

/// Change skill visual bug fix
pub const FIX_CHANGE_SKILL_VISUAL_BUG: bool = true;

/// Dungeon music fix
pub const FIX_DUNGEON_MUSIC: bool = true;

/// Permanent potions
pub const POTIUNI_PERMANENTE_RELUCRATE: bool = true;

/// Sticla cunoasterii fix
pub const FIX_STICLA_CUNOASTERII: bool = true;

/// Aura bug visual fix
pub const FIX_AURA_BUG_VIZUAL: bool = true;

/// Emotie fix
pub const PRODOMO_EMOTIE_FIX: bool = true;

/// Piatra fix
pub const PRODOMO_PIATRA_FIX: bool = true;

/// Some fix
pub const PRODOMO_SOME_FIX: bool = true;

/// Essex prodomo library
pub const LIB_ESSEX_PRODOMO: bool = true;

/// Wedding fix
pub const ENABLE_WEDDING_FIX: bool = true;

/// Clear old guilds lands by inactivity
pub const ENABLE_CLEAR_OLD_GUILDS_LANDS_BY_INACTIVITY: bool = true;

/// Update lastplay real time
pub const ENABLE_UPDATE_LASTPLAY_REAL_TIME: bool = true;

// ============================================================================
// Top System
// ============================================================================

/// Top players visual effect
pub const ENABLE_TOP_PLAYERS_EFFECT: bool = true;

/// Top player max level for effect
pub const TOP_PLAYER_MAX_LEVEL: u32 = 120;

/// Show leader and general guild
pub const ENABLE_SHOW_LIDER_AND_GENERAL_GUILD: bool = true;

/// Premium players system
pub const ENABLE_PREMIUM_PLAYERS: bool = true;

// ============================================================================
// Multi-Language Systems
// ============================================================================

/// Multi-language system (11 languages)
pub const MULTI_LANGUAGE_SYSTEM: bool = true;

/// Extended whisper details for multi-language
pub const EXTENDED_WHISPER_DETAILS: bool = true;

// ============================================================================
// Additional Features
// ============================================================================

/// Move channel system
pub const ENABLE_MOVE_CHANNEL: bool = true;

/// Send target info
pub const SEND_TARGET_INFO: bool = true;

/// Sort inventory items
pub const SORT_INVENTORY_ITEMS: bool = true;

/// View target player HP
pub const VIEW_TARGET_PLAYER_HP: bool = true;

/// View target decimal HP
pub const VIEW_TARGET_DECIMAL_HP: bool = true;

/// Send target info extended
pub const ENABLE_SEND_TARGET_INFO_EXTENDED: bool = true;

/// Daily gift system
pub const DAILY_GIFT_SYSTEM: bool = true;

/// Biologist renewal system
pub const ENABLE_BIOLOGIST_RENEWAL_SYSTEM: bool = true;

/// Hide costume system
pub const PRODOMO_HIDE_COSTUME: bool = true;

/// Multi farm block (HWID)
pub const ENABLE_MULTI_FARM_BLOCK: bool = true;

/// Renewal pickup affect (instant)
pub const RENEWAL_PICKUP_AFFECT: bool = true;

/// GM name display
pub const GM_PE_N: bool = true;

/// In-game item shop
pub const ENABLE_ITEMSHOP: bool = true;

/// Item shop to inventory
pub const ENABLE_ITEMSHOP_TO_INVENTORY: bool = true;

/// Messenger team system
pub const ENABLE_MESSENGER_TEAM: bool = true;

/// Renewed shop ex
pub const ENABLE_RENEWAL_SHOPEX: bool = true;

/// Remove gold limit (unsigned long long)
pub const ENABLE_REMOVE_LIMIT_GOLD: bool = true;

/// Affect renewal
pub const ENABLE_AFFECT_RENEWAL: bool = true;

/// Advance skill select
pub const ENABLE_ADVANCE_SKILL_SELECT: bool = true;

/// New attr reinforcements
pub const NEW_ATTR_RANFORSARI: bool = true;

/// Offline messages reworked
pub const OFFLINE_MESSAGE_REWORKED: bool = true;

/// Mount costume system
pub const ENABLE_MOUNT_COSTUME_SYSTEM: bool = true;

/// Weapon costume system
pub const ENABLE_WEAPON_COSTUME_SYSTEM: bool = true;

/// Guild bonuses
pub const GUILD_BONUSES: bool = true;

/// 7th and 8th skills
pub const AND8TH_SKILLS: bool = true;

/// New passive skill
pub const ENABLE_NEW_PASSIVE_SKILL: bool = true;

/// Expressing emotion
pub const ENABLE_EXPRESSING_EMOTION: bool = true;

/// Dungeon info
pub const DUNGEON_INFO: bool = true;

/// Inventory protected system
pub const ENABLE_INVENTORY_PROTECTED_SYSTEM: bool = true;

/// Global rank system
pub const ENABLE_GLOBAL_RANK: bool = true;

/// Global rank DB save time (seconds)
pub const RANKGLOBAL_DB_SAVE_TIME: u32 = 60 * 20;

/// Global rank DB flush time (seconds)
pub const RANKGLOBAL_DB_FLUSH_TIME: u32 = 10;

/// Global rank DB flush count
pub const RANKGLOBAL_DB_FLUSH_COUNT: u32 = 2000;

/// Premium private shop
pub const PREMIUM_PRIVATE_SHOP: bool = true;

/// Private shop premium time
pub const ENABLE_PRIVATE_SHOP_PREMIUM_TIME: bool = true;

/// Private shop locked slots
pub const ENABLE_PRIVATE_SHOP_LOCKED_SLOTS: bool = true;

/// Switchbot system
pub const ENABLE_SWITCHBOT: bool = true;

/// Event manager
pub const EVENT_MANAGER: bool = true;

/// Shaman system
pub const ENABLE_SHAMAN_SYSTEM: bool = true;

/// Pet system
pub const PET_SYSTEM: bool = true;

/// UDP block
pub const UDP_BLOCK: bool = true;

/// Casket preview
pub const CASKET_PREVIEW_ENABLE: bool = true;

/// Skin system
pub const SKIN_SYSTEM: bool = true;

/// Battle pass system
pub const ENABLE_BATTLE_PASS: bool = true;

/// Reward system
pub const ENABLE_REWARD_SYSTEM: bool = true;

/// World boss event
pub const WORLD_BOSS_EVENT: bool = true;

// ============================================================================
// Map Constants
// ============================================================================

/// Map allow limit
pub const MAP_ALLOW_LIMIT: u32 = 32;

// ============================================================================
// Poison/Affect Fix
// ============================================================================

/// Affect types that are fixed for poison risipa
/// This is a list of affect types that should not be affected by poison risipa
pub const POISON_RISIPA_FIXED_AFFECTS: &[u32] = &[
    0,  // AFFECT_MOV_SPEED
    1,  // AFFECT_ATT_SPEED
    2,  // AFFECT_STR
    3,  // AFFECT_DEX
    4,  // AFFECT_INT
    5,  // AFFECT_CON
    6,  // AFFECT_CHINA_FIREWORK
    10, // SKILL_JEONGWI
    11, // SKILL_GEOMKYUNG
    12, // SKILL_CHUNKEON
    13, // SKILL_EUNHYUNG
    14, // SKILL_GYEONGGONG
    15, // SKILL_GWIGEOM
    16, // SKILL_TERROR
    17, // SKILL_JUMAGAP
    18, // SKILL_MANASHILED
    19, // SKILL_HOSIN
    20, // SKILL_REFLECT
    21, // SKILL_KWAESOK
    22, // SKILL_JEUNGRYEOK
    23, // SKILL_GICHEON
];
