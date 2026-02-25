//! Data structures ported from `server/server/common/tables.h`.
//!
//! The table declarations use `#[repr(C, packed)]` only as a transitional
//! in-memory audit surface. Their explicit field types and offsets model the
//! active legacy x86 C++ profile (`#pragma pack(1)`, 32-bit `long` and
//! `time_t`, and one-byte C++ `bool`). This is not the final wire
//! representation; codecs must serialize fields explicitly.
//!
//! The priority layouts assume the feature set enabled by
//! `server/server/common/prodomodefines.h`. Changing those feature macros
//! changes the conditional fields and therefore their offsets. Missing field
//! docs are suppressed in this generated port.
#![allow(missing_docs)]

// ---------------------------------------------------------------------------
// Size constants (from length.h / item_length.h / prodomodefines.h)
// ---------------------------------------------------------------------------

pub const CHARACTER_NAME_MAX_LEN: usize = 24;
pub const LOGIN_MAX_LEN: usize = 30;
pub const PASSWD_MAX_LEN: usize = 16;
pub const SOCIAL_ID_MAX_LEN: usize = 18;
pub const ACCOUNT_STATUS_MAX_LEN: usize = 8;
pub const IP_ADDRESS_LENGTH: usize = 15;
pub const MAX_HOST_LENGTH: usize = 15;
pub const PLAYER_PER_ACCOUNT: usize = 4;
pub const SKILL_MAX_NUM: usize = 255;
pub const QUICKSLOT_MAX_NUM: usize = 36;
pub const PART_MAX_NUM: usize = 6;
pub const PREMIUM_MAX_NUM: usize = 9;
pub const ITEM_SOCKET_MAX_NUM: usize = 6;
pub const ITEM_ATTRIBUTE_MAX_NUM: usize = 7;
pub const SHOP_HOST_ITEM_MAX_NUM: usize = 40;
pub const QUEST_NAME_MAX_LEN: usize = 32;
pub const QUEST_STATE_MAX_LEN: usize = 64;
pub const MOB_ENCHANTS_MAX_NUM: usize = 6;
pub const MOB_RESISTS_MAX_NUM: usize = 11;
pub const MOB_SKILL_MAX_NUM: usize = 5;
pub const ITEM_NAME_MAX_LEN: usize = 36;
pub const ITEM_LIMIT_MAX_NUM: usize = 2;
pub const ITEM_APPLY_MAX_NUM: usize = 3;
pub const ITEM_VALUES_MAX_NUM: usize = 6;
pub const REFINE_MATERIAL_MAX_NUM: usize = 5;
pub const BANWORD_MAX_LEN: usize = 24;
pub const EVENT_FLAG_NAME_MAX_LEN: usize = 32;
pub const APPLY_NAME_MAX_LEN: usize = 32;
pub const ITEM_ATTRIBUTE_MAX_LEVEL: usize = 5;
pub const ATTRIBUTE_SET_MAX_NUM: usize = 10;
pub const SHOP_PRICELIST_MAX_NUM: usize = 40;
pub const MAP_ALLOW_LIMIT: usize = 32;
pub const SAFEBOX_PASSWORD_MAX_LEN: usize = 6;
pub const GUILD_NAME_MAX_LEN: usize = 12;
pub const SHOP_SIGN_MAX_LEN: usize = 32;
pub const TITLE_MAX_LEN: usize = 32;
pub const FISH_EVENT_SLOTS_NUM: usize = 24;
pub const BATTLEPASS_MISSIONS_PER_PLAYER: usize = 10;

// ---------------------------------------------------------------------------
// TSimplePlayer – compact player summary for character selection
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TSimplePlayer {
    pub id: u32,
    pub name: [i8; CHARACTER_NAME_MAX_LEN + 1],
    pub job: u8,
    pub level: u8,
    pub play_minutes: u32,
    pub st: u8,
    pub ht: u8,
    pub dx: u8,
    pub iq: u8,
    pub main_part: u16,
    pub change_name: u8,
    pub hair_part: u16,
    pub sash_part: u16,
    pub dummy: [u8; 4],
    pub x: i32,
    pub y: i32,
    pub addr: i32,
    pub port: u16,
    pub skill_group: u8,
    pub conqueror_level: u8,
    pub sungma_str: u8,
    pub sungma_hp: u8,
    pub sungma_move: u8,
    pub sungma_immune: u8,
}

// ---------------------------------------------------------------------------
// TAccountTable – full account data sent on login
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TAccountTable {
    pub id: u32,
    pub login: [i8; LOGIN_MAX_LEN + 1],
    pub passwd: [i8; PASSWD_MAX_LEN + 1],
    pub social_id: [i8; SOCIAL_ID_MAX_LEN + 1],
    pub status: [i8; ACCOUNT_STATUS_MAX_LEN + 1],
    pub empire: u8,
    pub players: [TSimplePlayer; PLAYER_PER_ACCOUNT],
    pub language: u8,
}

// ---------------------------------------------------------------------------
// TPlayerItemAttribute – single item attribute (type + value)
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPlayerItemAttribute {
    pub btype: u8,
    pub svalue: i16,
}

// ---------------------------------------------------------------------------
// TPlayerItem – player inventory / equipment item
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPlayerItem {
    pub id: u32,
    pub window: u8,
    pub pos: u16,
    pub count: u32,
    pub vnum: u32,
    pub sockets: [i32; ITEM_SOCKET_MAX_NUM],
    pub attrs: [TPlayerItemAttribute; ITEM_ATTRIBUTE_MAX_NUM],
    pub owner: u32,
    pub refine_element: u32,
    pub transmutation: u32,
}

// ---------------------------------------------------------------------------
// TQuickslot – quick-bar slot
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TQuickslot {
    pub slot_type: u8,
    pub pos: u8,
}

// ---------------------------------------------------------------------------
// TPlayerSkill – per-skill data stored on the player
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPlayerSkill {
    pub master_type: u8,
    pub level: u8,
    // C++ `time_t tNextRead` is four bytes in the audited x86 profile.
    pub next_read: i32,
}

// ---------------------------------------------------------------------------
// THorseInfo – mounted horse state
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct THorseInfo {
    pub level: u8,
    pub riding: u8,
    pub stamina: i16,
    pub health: i16,
    pub health_drop_time: u32,
}

// ---------------------------------------------------------------------------
// TPlayerBattlePass – one battle-pass mission slot
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPlayerBattlePass {
    pub mission_id: u16,
    pub progress: u32,
    pub battle_pass_type: u8,
    pub end_time: u32,
}

// ---------------------------------------------------------------------------
// TPlayerTable – full player record (largest struct)
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPlayerTable {
    pub id: u32,
    pub name: [i8; CHARACTER_NAME_MAX_LEN + 1],
    pub ip: [i8; IP_ADDRESS_LENGTH + 1],
    pub job: u16,
    pub voice: u8,
    pub level: u8,
    pub level_step: u8,
    pub st: i16,
    pub ht: i16,
    pub dx: i16,
    pub iq: i16,
    pub exp: u32,
    // C++ `unsigned long long gold` is active via `ENABLE_REMOVE_LIMIT_GOLD`.
    pub gold: u64,
    pub gaya: i32,
    pub dir: u8,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub map_index: i32,
    pub exit_x: i32,
    pub exit_y: i32,
    pub exit_map_index: i32,
    pub hp: i32,
    pub sp: i32,
    pub random_hp: i16,
    pub random_sp: i16,
    pub playtime: i32,
    pub stat_point: i16,
    pub skill_point: i16,
    pub sub_skill_point: i16,
    pub horse_skill_point: i16,
    pub skills: [TPlayerSkill; SKILL_MAX_NUM],
    pub quickslot: [TQuickslot; QUICKSLOT_MAX_NUM],
    pub part_base: u8,
    pub parts: [u16; PART_MAX_NUM],
    pub stamina: i16,
    pub skill_group: u8,
    pub alignment: i32,
    pub stat_reset_count: i16,
    pub horse: THorseInfo,
    pub logoff_interval: u32,
    pub premium_times: [i32; PREMIUM_MAX_NUM],
    pub envanter: i32,
    pub fish_event_use_count: u32,
    pub fish_slots: [TPlayerFishEventSlot; FISH_EVENT_SLOTS_NUM],
    pub premium: u8,
    pub premium_time: i32,
    pub secured: u8,
    pub secured_password: i32,
    pub biologist_state: u32,
    pub biologist_items_taken: u32,
    pub biologist_completed: u32,
    pub conqueror_level: u8,
    pub conqueror_level_step: u8,
    pub sungma_str: i16,
    pub sungma_hp: i16,
    pub sungma_move: i16,
    pub sungma_immune: i16,
    pub conqueror_exp: u32,
    pub conqueror_point: i16,
    pub battle_pass: [TPlayerBattlePass; BATTLEPASS_MISSIONS_PER_PLAYER],
    pub private_shop_unlocked_slot: u16,
}

// ---------------------------------------------------------------------------
// TPlayerFishEventSlot – fish event mini-game slot
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPlayerFishEventSlot {
    // C++ `bool bIsMain` is one byte. Keep the raw wire byte as `u8`; legacy
    // data may contain values that are not valid Rust `bool` values.
    pub is_main: u8,
    pub shape: u8,
}

// ---------------------------------------------------------------------------
// TMobSkillLevel – skill entry in mob proto
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TMobSkillLevel {
    pub vnum: u32,
    pub level: u8,
}

// ---------------------------------------------------------------------------
// TMobTable – monster / NPC proto (inherits TEntityTable::dwVnum)
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TMobTable {
    pub vnum: u32,
    pub name: [i8; CHARACTER_NAME_MAX_LEN + 1],
    pub locale_name: [i8; CHARACTER_NAME_MAX_LEN + 1],
    pub mob_type: u8,
    pub rank: u8,
    pub battle_type: u8,
    pub level: u8,
    pub size: u8,
    pub gold_min: u32,
    pub gold_max: u32,
    pub exp: u32,
    pub max_hp: u32,
    pub regen_cycle: u8,
    pub regen_percent: u8,
    pub def: u16,
    pub ai_flag: u32,
    pub race_flag: u32,
    pub immune_flag: u32,
    pub str: u8,
    pub dex: u8,
    pub con: u8,
    pub int_: u8,
    pub damage_range: [u32; 2],
    pub attack_speed: i16,
    pub moving_speed: i16,
    pub aggressive_hp_pct: u8,
    pub aggressive_sight: u16,
    pub attack_range: u16,
    pub enchants: [i8; MOB_ENCHANTS_MAX_NUM],
    pub resists: [i8; MOB_RESISTS_MAX_NUM],
    pub resurrection_vnum: u32,
    pub drop_item_vnum: u32,
    pub mount_capacity: u8,
    pub on_click_type: u8,
    pub empire: u8,
    pub folder: [i8; 65],
    pub dam_multiply: f32,
    pub summon_vnum: u32,
    pub drain_sp: u32,
    pub mob_color: u32,
    pub polymorph_item_vnum: u32,
    pub skills: [TMobSkillLevel; MOB_SKILL_MAX_NUM],
    pub berserk_point: u8,
    pub stone_skin_point: u8,
    pub god_speed_point: u8,
    pub death_blow_point: u8,
    pub revive_point: u8,
}

// ---------------------------------------------------------------------------
// TSkillTable – skill proto
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TSkillTable {
    pub vnum: u32,
    pub name: [i8; 33],
    pub skill_type: u8,
    pub max_level: u8,
    pub splash_range: u32,
    pub point_on: [i8; 64],
    pub point_poly: [i8; 101],
    pub sp_cost_poly: [i8; 101],
    pub duration_poly: [i8; 101],
    pub duration_sp_cost_poly: [i8; 101],
    pub cooldown_poly: [i8; 101],
    pub master_bonus_poly: [i8; 101],
    pub grand_master_add_sp_cost_poly: [i8; 101],
    pub flag: u32,
    pub affect_flag: u32,
    pub point_on2: [i8; 64],
    pub point_poly2: [i8; 101],
    pub duration_poly2: [i8; 101],
    pub affect_flag2: u32,
    pub point_on3: [i8; 64],
    pub point_poly3: [i8; 101],
    pub duration_poly3: [i8; 101],
    pub level_step: u8,
    pub level_limit: u8,
    pub pre_skill_vnum: u32,
    pub pre_skill_level: u8,
    pub max_hit: i32,
    pub splash_around_damage_adjust_poly: [i8; 101],
    pub skill_attr_type: u8,
    pub target_range: u32,
}

// ---------------------------------------------------------------------------
// TItemPos – inventory position (window + cell)
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TItemPos {
    pub window_type: u8,
    pub cell: u16,
}

// ---------------------------------------------------------------------------
// TQuestTable – quest state record
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TQuestTable {
    pub pid: u32,
    pub name: [i8; QUEST_NAME_MAX_LEN + 1],
    pub state: [i8; QUEST_STATE_MAX_LEN + 1],
    pub value: i32,
}

// ---------------------------------------------------------------------------
// TItemLimit – item limit entry
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TItemLimit {
    pub limit_type: u8,
    pub value: i32,
}

// ---------------------------------------------------------------------------
// TItemApply – item apply entry
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TItemApply {
    pub apply_type: u8,
    pub value: i32,
}

// ---------------------------------------------------------------------------
// TItemTable – item proto (inherits TEntityTable::dwVnum)
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TItemTable {
    pub vnum: u32,
    pub vnum_range: u32,
    pub name: [i8; ITEM_NAME_MAX_LEN + 1],
    pub locale_name: [i8; ITEM_NAME_MAX_LEN + 1],
    pub item_type: u8,
    pub sub_type: u8,
    pub weight: u8,
    pub size: u8,
    pub anti_flags: u32,
    pub flags: u32,
    pub wear_flags: u32,
    pub immune_flag: u32,
    pub gold: u64,
    pub shop_buy_price: u64,
    pub limits: [TItemLimit; ITEM_LIMIT_MAX_NUM],
    pub applies: [TItemApply; ITEM_APPLY_MAX_NUM],
    pub values: [i32; ITEM_VALUES_MAX_NUM],
    pub sockets: [i32; ITEM_SOCKET_MAX_NUM],
    pub refined_vnum: u32,
    pub refine_set: u16,
    pub alter_to_magic_item_pct: u8,
    pub specular: u8,
    pub gain_socket_pct: u8,
    pub addon_type: i16,
    pub limit_real_time_first_use_index: i8,
    pub limit_timer_based_on_wear_index: i8,
}

// ---------------------------------------------------------------------------
// TItemAttrTable – item attribute bonus table
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TItemAttrTable {
    pub apply: [i8; APPLY_NAME_MAX_LEN + 1],
    pub apply_index: u32,
    pub prob: u32,
    pub values: [i32; ITEM_ATTRIBUTE_MAX_LEVEL],
    pub max_level_by_set: [u8; ATTRIBUTE_SET_MAX_NUM],
}

// ---------------------------------------------------------------------------
// TShopItemTable – shop item entry
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TShopItemTable {
    pub vnum: u32,
    pub count: u16,
    pub pos: TItemPos,
    pub price: u64,
    pub display_pos: u8,
    pub sockets: [i32; ITEM_SOCKET_MAX_NUM],
    pub attrs: [TPlayerItemAttribute; ITEM_ATTRIBUTE_MAX_NUM],
    pub price_type: u8,
    pub price_vnum: u32,
}

// ---------------------------------------------------------------------------
// TShopTable – NPC shop definition
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TShopTable {
    pub vnum: u32,
    pub npc_vnum: u32,
    pub item_count: u8,
    pub items: [TShopItemTable; SHOP_HOST_ITEM_MAX_NUM],
    pub shop_name: [i8; SHOP_SIGN_MAX_LEN + 1],
}

// ---------------------------------------------------------------------------
// DB protocol packets
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGDLogin {
    pub login: [i8; LOGIN_MAX_LEN + 1],
    pub passwd: [i8; PASSWD_MAX_LEN + 1],
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPlayerLoadPacket {
    pub account_id: u32,
    pub player_id: u32,
    pub account_index: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPlayerCreatePacket {
    pub login: [i8; LOGIN_MAX_LEN + 1],
    pub passwd: [i8; PASSWD_MAX_LEN + 1],
    pub account_id: u32,
    pub account_index: u8,
    pub player_table: TPlayerTable,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPlayerDeletePacket {
    pub login: [i8; LOGIN_MAX_LEN + 1],
    pub player_id: u32,
    pub account_index: u8,
    pub private_code: [i8; 8],
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TLogoutPacket {
    pub login: [i8; LOGIN_MAX_LEN + 1],
    pub passwd: [i8; PASSWD_MAX_LEN + 1],
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPlayerCountPacket {
    pub count: u32,
}

// ---------------------------------------------------------------------------
// Guild packets
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGuildSkillUpdate {
    pub guild_id: u32,
    pub amount: i32,
    pub skill_levels: [u8; 12],
    pub skill_point: u8,
    pub save: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGuildExpUpdate {
    pub guild_id: u32,
    pub amount: i32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGuildChangeMemberData {
    pub guild_id: u32,
    pub pid: u32,
    pub offer: u32,
    pub level: u8,
    pub grade: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGuildWar {
    pub war_type: u8,
    pub war: u8,
    pub guild_from: u32,
    pub guild_to: u32,
    pub war_price: i32,
    pub initial_score: i32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGuildWarScore {
    pub guild_gain_point: u32,
    pub guild_opponent: u32,
    pub score: i32,
    pub bet_score: i32,
}

// ---------------------------------------------------------------------------
// Affect packets
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketAffectElement {
    pub affect_type: u32,
    pub apply_on: u8,
    pub apply_value: i32,
    pub flag: u32,
    pub duration: i32,
    pub sp_cost: i32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGDAddAffect {
    pub pid: u32,
    pub elem: TPacketAffectElement,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGDRemoveAffect {
    pub pid: u32,
    pub affect_type: u32,
    pub apply_on: u8,
}

// ---------------------------------------------------------------------------
// Party packets
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketPartyCreate {
    pub leader_pid: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketPartyDelete {
    pub leader_pid: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketPartyAdd {
    pub leader_pid: u32,
    pub pid: u32,
    pub state: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketPartyRemove {
    pub leader_pid: u32,
    pub pid: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketPartyStateChange {
    pub leader_pid: u32,
    pub pid: u32,
    pub role: u8,
    pub flag: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketPartySetMemberLevel {
    pub leader_pid: u32,
    pub pid: u32,
    pub level: u8,
}

// ---------------------------------------------------------------------------
// Setup / map packets
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGDSetup {
    pub public_ip: [i8; 16],
    pub channel: u8,
    pub listen_port: u16,
    pub p2p_port: u16,
    pub maps: [i32; MAP_ALLOW_LIMIT],
    pub login_count: u32,
    pub auth_server: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketDGMapLocations {
    pub count: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TMapLocation {
    pub maps: [i32; MAP_ALLOW_LIMIT],
    pub host: [i8; MAX_HOST_LENGTH + 1],
    pub port: u16,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketDGP2P {
    pub host: [i8; MAX_HOST_LENGTH + 1],
    pub port: u16,
    pub channel: u8,
}

// ---------------------------------------------------------------------------
// Auth / login packets
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGDAuthLogin {
    pub id: u32,
    pub login_key: u32,
    pub login: [i8; LOGIN_MAX_LEN + 1],
    pub social_id: [i8; SOCIAL_ID_MAX_LEN + 1],
    pub client_keys: [u32; 4],
    pub premium_times: [i32; PREMIUM_MAX_NUM],
    pub language: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGDLoginByKey {
    pub login: [i8; LOGIN_MAX_LEN + 1],
    pub login_key: u32,
    pub client_keys: [u32; 4],
    pub ip: [i8; MAX_HOST_LENGTH + 1],
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketDGLoginAlready {
    pub login: [i8; LOGIN_MAX_LEN + 1],
}

// ---------------------------------------------------------------------------
// Safebox packets
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TSafeboxTable {
    pub id: u32,
    pub size: u8,
    pub gold: u32,
    pub item_count: u16,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TSafeboxLoadPacket {
    pub id: u32,
    pub login: [i8; LOGIN_MAX_LEN + 1],
    pub password: [i8; SAFEBOX_PASSWORD_MAX_LEN + 1],
}

// ---------------------------------------------------------------------------
// Refine table
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TRefineMaterial {
    pub vnum: u32,
    pub count: i32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TRefineTable {
    pub id: u32,
    pub material_count: u8,
    pub cost: i32,
    pub prob: i32,
    pub materials: [TRefineMaterial; REFINE_MATERIAL_MAX_NUM],
}

// ---------------------------------------------------------------------------
// Guild ladder / reserve
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGuildLadder {
    pub guild: u32,
    pub ladder_point: i32,
    pub win: i32,
    pub draw: i32,
    pub loss: i32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TGuildWarReserve {
    pub id: u32,
    pub guild_from: u32,
    pub guild_to: u32,
    pub time: u32,
    pub war_type: u8,
    pub war_price: i32,
    pub initial_score: i32,
    pub started: u8,
    pub bet_from: u32,
    pub bet_to: u32,
    pub power_from: i32,
    pub power_to: i32,
    pub handicap: i32,
}

// ---------------------------------------------------------------------------
// Marriage packets
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketMarriageAdd {
    pub pid1: u32,
    pub pid2: u32,
    pub marry_time: i32,
    pub name1: [i8; CHARACTER_NAME_MAX_LEN + 1],
    pub name2: [i8; CHARACTER_NAME_MAX_LEN + 1],
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketMarriageUpdate {
    pub pid1: u32,
    pub pid2: u32,
    pub love_point: i32,
    pub married: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketMarriageRemove {
    pub pid1: u32,
    pub pid2: u32,
}

// ---------------------------------------------------------------------------
// Privilege packets
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGiveGuildPriv {
    pub priv_type: u8,
    pub value: i32,
    pub guild_id: u32,
    pub duration_sec: i32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGiveEmpirePriv {
    pub priv_type: u8,
    pub value: i32,
    pub empire: u8,
    pub duration_sec: i32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketGiveCharacterPriv {
    pub priv_type: u8,
    pub value: i32,
    pub pid: u32,
}

// ---------------------------------------------------------------------------
// Money log
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketMoneyLog {
    pub log_type: u8,
    pub vnum: u32,
    pub gold: i64,
}

// ---------------------------------------------------------------------------
// Event flag
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketSetEventFlag {
    pub flag_name: [i8; EVENT_FLAG_NAME_MAX_LEN + 1],
    pub value: i32,
}

// ---------------------------------------------------------------------------
// Item price info (myshop)
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TItemPriceInfo {
    pub vnum: u32,
    pub price: u64,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TItemPriceListTable {
    pub owner_id: u32,
    pub count: u8,
    pub price_info: [TItemPriceInfo; SHOP_PRICELIST_MAX_NUM],
}

// ---------------------------------------------------------------------------
// Admin info
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TAdminInfo {
    pub id: i32,
    pub account: [i8; 32],
    pub name: [i8; 32],
    pub contact_ip: [i8; 16],
    pub server_ip: [i8; 16],
    pub authority: i32,
}

// ---------------------------------------------------------------------------
// Monarch info
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TMonarchInfo {
    pub pid: [u32; 4],
    pub money: [i64; 4],
    pub name: [[i8; 32]; 4],
    pub date: [[i8; 32]; 4],
}

// ---------------------------------------------------------------------------
// Block / ban packets
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketBlockCountryIp {
    pub ip_from: u32,
    pub ip_to: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketBlockException {
    pub cmd: u8,
    pub login: [i8; LOGIN_MAX_LEN + 1],
}

// ---------------------------------------------------------------------------
// Guild master change
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TPacketChangeGuildMaster {
    pub guild_id: u32,
    pub id_from: u32,
    pub id_to: u32,
}

// ---------------------------------------------------------------------------
// Item ID range
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TItemIDRangeTable {
    pub min: u32,
    pub max: u32,
    pub usable_item_id_min: u32,
}

// ---------------------------------------------------------------------------
// Channel status
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TChannelStatus {
    pub port: i16,
    pub status: u8,
}

// ---------------------------------------------------------------------------
// Size verification tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn test_t_simple_player_size() {
        // Packed x86 C++ profile with the active sash and conqueror fields = 70.
        assert_eq!(size_of::<TSimplePlayer>(), 70);
    }

    #[test]
    fn test_t_account_table_size() {
        // Packed x86 C++ profile with four 70-byte TSimplePlayer values = 362.
        assert_eq!(size_of::<TAccountTable>(), 362);
    }

    #[test]
    fn test_t_player_item_attribute_size() {
        assert_eq!(size_of::<TPlayerItemAttribute>(), 3);
    }

    #[test]
    fn test_t_player_item_size() {
        // Packed x86 C++ profile with six sockets and seven attributes = 72.
        assert_eq!(size_of::<TPlayerItem>(), 72);
    }

    #[test]
    fn test_t_quickslot_size() {
        assert_eq!(size_of::<TQuickslot>(), 2);
    }

    #[test]
    fn test_t_player_skill_size() {
        // BYTE + BYTE + x86 time_t (4) with no trailing padding.
        assert_eq!(size_of::<TPlayerSkill>(), 6);
    }

    #[test]
    fn test_t_horse_info_size() {
        // Packed x86 C++ layout: BYTE + BYTE + short + short + DWORD.
        assert_eq!(size_of::<THorseInfo>(), 10);
    }

    #[test]
    fn test_t_player_fish_event_slot_size() {
        assert_eq!(size_of::<TPlayerFishEventSlot>(), 2);
    }

    #[test]
    fn test_t_player_table_size() {
        // Packed x86 C++ profile with the active feature set = 2007.
        assert_eq!(size_of::<TPlayerTable>(), 2007);
    }

    #[test]
    fn test_priority_field_offsets_match_x86_profile() {
        // These offsets make field omissions and accidental host-width
        // assumptions fail even when a total size happens to stay unchanged.
        assert_eq!(size_of::<bool>(), 1);
        assert_eq!(size_of::<i32>(), 4);
        assert_eq!(size_of::<u64>(), 8);

        assert_eq!(std::mem::offset_of!(TSimplePlayer, id), 0);
        assert_eq!(std::mem::offset_of!(TSimplePlayer, x), 50);
        assert_eq!(std::mem::offset_of!(TSimplePlayer, addr), 58);
        assert_eq!(std::mem::offset_of!(TSimplePlayer, conqueror_level), 65);
        assert_eq!(std::mem::offset_of!(TSimplePlayer, sungma_immune), 69);

        assert_eq!(std::mem::offset_of!(TAccountTable, login), 4);
        assert_eq!(std::mem::offset_of!(TAccountTable, players), 81);
        assert_eq!(std::mem::offset_of!(TAccountTable, language), 361);

        assert_eq!(std::mem::offset_of!(TPlayerItemAttribute, svalue), 1);

        assert_eq!(std::mem::offset_of!(TPlayerItem, sockets), 15);
        assert_eq!(std::mem::offset_of!(TPlayerItem, attrs), 39);
        assert_eq!(std::mem::offset_of!(TPlayerItem, refine_element), 64);
        assert_eq!(std::mem::offset_of!(TPlayerItem, transmutation), 68);

        assert_eq!(std::mem::offset_of!(TQuickslot, pos), 1);
        assert_eq!(std::mem::offset_of!(TPlayerSkill, next_read), 2);
        assert_eq!(std::mem::offset_of!(THorseInfo, health_drop_time), 6);
        assert_eq!(std::mem::offset_of!(TPlayerFishEventSlot, shape), 1);

        assert_eq!(std::mem::offset_of!(TPlayerTable, gold), 62);
        assert_eq!(std::mem::offset_of!(TPlayerTable, gaya), 70);
        assert_eq!(std::mem::offset_of!(TPlayerTable, skills), 127);
        assert_eq!(std::mem::offset_of!(TPlayerTable, quickslot), 1657);
        assert_eq!(std::mem::offset_of!(TPlayerTable, horse), 1751);
        assert_eq!(std::mem::offset_of!(TPlayerTable, fish_slots), 1809);
        assert_eq!(std::mem::offset_of!(TPlayerTable, battle_pass), 1895);
        assert_eq!(
            std::mem::offset_of!(TPlayerTable, private_shop_unlocked_slot),
            2005
        );
    }

    #[test]
    fn test_t_mob_skill_level_size() {
        assert_eq!(size_of::<TMobSkillLevel>(), 5);
    }

    #[test]
    fn test_t_mob_table_size() {
        // Packed x86 C++ profile with the inherited vnum = 255.
        assert_eq!(size_of::<TMobTable>(), 255);
    }

    #[test]
    fn test_t_skill_table_size() {
        // Packed x86 C++ profile with all three skill point arrays = 1475.
        assert_eq!(size_of::<TSkillTable>(), 1475);
    }

    #[test]
    fn test_t_item_pos_size() {
        // C++ sizeof(TItemPos) = 3 (BYTE + WORD)
        assert_eq!(size_of::<TItemPos>(), 3);
    }

    #[test]
    fn test_t_quest_table_size() {
        // DWORD + char[33] + char[65] + long = 4 + 33 + 65 + 4 = 106
        assert_eq!(size_of::<TQuestTable>(), 106);
    }

    #[test]
    fn test_t_item_limit_size() {
        assert_eq!(size_of::<TItemLimit>(), 5);
    }

    #[test]
    fn test_t_item_apply_size() {
        assert_eq!(size_of::<TItemApply>(), 5);
    }

    #[test]
    fn test_t_item_table_size() {
        // Packed x86 C++ profile with extended sockets and limit-gold fields = 204.
        assert_eq!(size_of::<TItemTable>(), 204);
    }

    #[test]
    fn test_t_item_attr_table_size() {
        // char[33] + DWORD + DWORD + long[5] + BYTE[10] = 33 + 4 + 4 + 20 + 10 = 71
        assert_eq!(size_of::<TItemAttrTable>(), 71);
    }

    #[test]
    fn test_t_shop_item_table_size() {
        // 4+2+3+8+1 + 24 + 21 + 1+4 = 68
        assert_eq!(size_of::<TShopItemTable>(), 68);
    }

    #[test]
    fn test_t_shop_table_size() {
        // 4+4+1 + 40*68 + 33 = 9 + 2720 + 33 = 2762
        assert_eq!(size_of::<TShopTable>(), 2762);
    }

    #[test]
    fn test_t_safebox_table_size() {
        // 4+1+4+2 = 11
        assert_eq!(size_of::<TSafeboxTable>(), 11);
    }

    #[test]
    fn test_t_refine_material_size() {
        assert_eq!(size_of::<TRefineMaterial>(), 8);
    }

    #[test]
    fn test_t_refine_table_size() {
        // 4+1+4+4 + 5*8 = 13 + 40 = 53
        assert_eq!(size_of::<TRefineTable>(), 53);
    }

    #[test]
    fn test_t_guild_war_reserve_size() {
        // Packed x86 C++ layout: 4+4+4+4+1+4+4+1+4+4+4+4+4 = 46.
        assert_eq!(size_of::<TGuildWarReserve>(), 46);
    }

    #[test]
    fn test_t_packet_affect_element_size() {
        // 4+1+4+4+4+4 = 21
        assert_eq!(size_of::<TPacketAffectElement>(), 21);
    }

    #[test]
    fn test_t_admin_info_size() {
        // 4 + 32 + 32 + 16 + 16 + 4 = 104
        assert_eq!(size_of::<TAdminInfo>(), 104);
    }

    #[test]
    fn test_t_channel_status_size() {
        assert_eq!(size_of::<TChannelStatus>(), 3);
    }

    #[test]
    fn test_t_monarch_info_size() {
        // 16 + 32 + 128 + 128 = 304
        assert_eq!(size_of::<TMonarchInfo>(), 304);
    }

    #[test]
    fn test_t_item_id_range_table_size() {
        assert_eq!(size_of::<TItemIDRangeTable>(), 12);
    }
}
