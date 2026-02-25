//! Legacy DB record codecs for the read-only login path.
//!
//! The legacy game/database peers exchange packed C++ records.  The wire
//! layout in this module is therefore defined by explicit little-endian
//! reads and writes, rather than by the alignment or size of a Rust value in
//! memory.  The constants below describe the active x86 build in
//! `server/server/common/prodomodefines.h` (32-bit `long` and `time_t`).
//!
//! The module covers the payloads used by these exchanges:
//!
//! * `HEADER_GD_LOGIN_BY_KEY` request and its eventual account outcomes
//!   (`HEADER_DG_LOGIN_SUCCESS`, `HEADER_DG_LOGIN_NOT_EXIST`,
//!   `HEADER_DG_LOGIN_WRONG_PASSWD`, or `HEADER_DG_LOGIN_ALREADY`);
//! * `HEADER_GD_PLAYER_LOAD` request and `HEADER_DG_PLAYER_LOAD_SUCCESS`
//!   player result; and
//! * `HEADER_DG_QUEST_LOAD`, whose payload is a `u32` count followed by that
//!   many `TQuestTable` records; and
//! * the optional boot `TMarketItemPrice` record, packed as `DWORD`, signed
//!   64-bit gold, and `DWORD` cheque;
//! * the active x86 boot `TItemTable` record, packed field by field as its
//!   204-byte six-socket, 64-bit-price profile; and
//! * the optional boot `TEventTable` record, packed as `DWORD`, a 64-byte
//!   type name, two x86 `long` values, two x86 `int` values, and one C++
//!   `bool` byte;
//! * the unconditional boot `TSkillTable` record, encoded field by field as
//!   its exact 1,475 packed x86 bytes;
//! * the unconditional boot `TMobTable` record, encoded field by field as its
//!   exact 255-byte packed x86 layout; and
//! * the unconditional boot `TRefineTable` record, packed as `DWORD`, `BYTE`,
//!   two signed 32-bit values, and five 8-byte material records;
//! * the active x86 boot `TItemAttrTable` record, packed as a 33-byte apply
//!   name, two `DWORD` values, five signed 32-bit values, and ten `BYTE`
//!   set-level values;
//! * the active x86 base `TShopTable` record and its fixed `TShopItemTable`
//!   slots, encoded field by field with the raw shop-name tail; and
//! * the active x86 boot `building::TLand` record, encoded as `DWORD`, five
//!   signed 32-bit values, `DWORD`, `BYTE`, three zero padding bytes, and
//!   `DWORD`;
//! * the active x86 boot `building::TObjectProto` record, encoded field by
//!   field as 96 little-endian bytes, including its five material slots and
//!   derived NPC coordinates; and
//! * the active x86 boot `building::TObject` record, encoded as three
//!   `DWORD` values, three signed 32-bit values, three IEEE-754 `float`
//!   values, and one signed 32-bit value; and
//! * the horse-name request/response pair: a four-byte player ID request and
//!   a packed `DWORD` plus 25-byte name acknowledgement record; and
//! * the active move-channel pair: an eight-byte map/channel request and a
//!   six-byte address/port result; and
//! * the 25-byte `TPacketGDAddAffect` request payload, including its
//!   21-byte `TPacketAffectElement`; and
//! * the nine-byte `TPacketGDRemoveAffect` request payload; and
//! * the variable-size `HEADER_DG_AFFECT_LOAD` response payload, containing a
//!   player ID, row count, and that many packed affect elements.
//!
//! Decoders reject short and overlong fixed records. Count-prefixed decoders
//! validate their exact payload size before reserving or allocating elements.

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;
use std::io;

/// Maximum number of fixed-width name bytes, including the NUL terminator.
pub const LEGACY_LOGIN_BYTES: usize = 31;
/// Maximum number of fixed-width password bytes, including the NUL terminator.
pub const LEGACY_PASSWORD_BYTES: usize = 17;
/// Maximum number of fixed-width social-ID bytes, including the NUL terminator.
pub const LEGACY_SOCIAL_ID_BYTES: usize = 19;
/// Maximum number of fixed-width account-status bytes, including the NUL terminator.
pub const LEGACY_ACCOUNT_STATUS_BYTES: usize = 9;
/// Maximum number of fixed-width character-name bytes, including the NUL terminator.
pub const LEGACY_CHARACTER_NAME_BYTES: usize = 25;
/// Maximum number of fixed-width IP/host bytes, including the NUL terminator.
pub const LEGACY_IP_BYTES: usize = 16;
/// Number of character summaries in an account result.
pub const LEGACY_PLAYER_PER_ACCOUNT: usize = 4;
/// Number of skills in a player result.
pub const LEGACY_SKILL_MAX_NUM: usize = 255;
/// Number of quickslots in a player result.
pub const LEGACY_QUICKSLOT_MAX_NUM: usize = 36;
/// Number of equipment-part words in a player result.
///
/// This is the value of the C++ `PART_MAX_NUM` sentinel in `EParts`.
/// `PART_WEAPON_SUB` follows the sentinel and is not serialized.
pub const LEGACY_PART_MAX_NUM: usize = 6;
/// Number of premium counters in a player result.
pub const LEGACY_PREMIUM_MAX_NUM: usize = 9;
/// Number of fish-event slots in a player result.
pub const LEGACY_FISH_EVENT_SLOTS_NUM: usize = 24;
/// Number of battle-pass records in a player result.
pub const LEGACY_BATTLEPASS_MISSIONS_PER_PLAYER: usize = 10;
/// Maximum number of quest-name bytes, including the NUL terminator.
pub const LEGACY_QUEST_NAME_BYTES: usize = 33;
/// Maximum number of quest-state bytes, including the NUL terminator.
pub const LEGACY_QUEST_STATE_BYTES: usize = 65;

/// Packed x86 size of `TSimplePlayer` in the active legacy build.
pub const SIMPLE_PLAYER_WIRE_SIZE: usize = 70;
/// Packed x86 size of `TAccountTable` in the active legacy build.
pub const LOGIN_ACCOUNT_WIRE_SIZE: usize = 362;
/// Packed x86 size of `TPacketDGLoginAlready`.
pub const LOGIN_ALREADY_WIRE_SIZE: usize = LEGACY_LOGIN_BYTES;
/// Packed x86 size of `TPacketGDLoginByKey` (the GD request payload).
pub const LOGIN_BY_KEY_REQUEST_WIRE_SIZE: usize = 67;
/// Packed x86 size of `TPlayerLoadPacket` (the GD request payload).
pub const PLAYER_LOAD_REQUEST_WIRE_SIZE: usize = 9;
/// Packed x86 size of `TPlayerTable` in the active legacy build.
pub const PLAYER_RESULT_WIRE_SIZE: usize = 2007;
/// Packed x86 size of one `TQuestTable` record.
pub const QUEST_RECORD_WIRE_SIZE: usize = 106;
/// Number of bytes in the `QUEST_LOAD` count prefix.
pub const QUEST_COUNT_WIRE_SIZE: usize = 4;
/// Packed x86 size of `TMarketItemPrice` in the active legacy build.
pub const MARKET_ITEM_PRICE_WIRE_SIZE: usize = 16;
/// Number of bytes in `TEventTable::szType` (the C++ array has no extra NUL).
pub const EVENT_TYPE_BYTES: usize = 64;
/// Packed x86 size of `TEventTable` in the active legacy build.
pub const EVENT_TABLE_WIRE_SIZE: usize = 85;
/// Number of raw bytes in `TSkillTable::szName`.
pub const SKILL_NAME_BYTES: usize = 33;
/// Number of raw bytes in each `TSkillTable::szPointOn*` field.
pub const SKILL_POINT_ON_BYTES: usize = 64;
/// Number of raw bytes in each 101-byte `TSkillTable` polynomial field.
pub const SKILL_POLY_EXPR_BYTES: usize = 101;
/// Exact packed x86 wire size of `TSkillTable` in the active legacy build.
///
/// `server/server/common/tables.h` applies `#pragma pack(1)` before
/// `TSkillTable`. Natural Rust alignment must therefore not derive this size.
pub const SKILL_TABLE_RECORD_WIRE_SIZE: usize = 1_475;
/// Byte offset of `TSkillTable::dwVnum`.
pub const SKILL_TABLE_VNUM_OFFSET: usize = 0;
/// Byte offset of `TSkillTable::szName`.
pub const SKILL_TABLE_NAME_OFFSET: usize = SKILL_TABLE_VNUM_OFFSET + 4;
/// Byte offset of `TSkillTable::bType`.
pub const SKILL_TABLE_SKILL_TYPE_OFFSET: usize = SKILL_TABLE_NAME_OFFSET + SKILL_NAME_BYTES;
/// Byte offset of `TSkillTable::bMaxLevel`.
pub const SKILL_TABLE_MAX_LEVEL_OFFSET: usize = SKILL_TABLE_SKILL_TYPE_OFFSET + 1;
/// Byte offset of `TSkillTable::dwSplashRange`.
pub const SKILL_TABLE_SPLASH_RANGE_OFFSET: usize = SKILL_TABLE_MAX_LEVEL_OFFSET + 1;
/// Byte offset of `TSkillTable::szPointOn`.
pub const SKILL_TABLE_POINT_ON_OFFSET: usize = SKILL_TABLE_SPLASH_RANGE_OFFSET + 4;
/// Byte offset of `TSkillTable::szPointPoly`.
pub const SKILL_TABLE_POINT_POLY_OFFSET: usize = SKILL_TABLE_POINT_ON_OFFSET + SKILL_POINT_ON_BYTES;
/// Byte offset of `TSkillTable::szSPCostPoly`.
pub const SKILL_TABLE_SP_COST_POLY_OFFSET: usize =
    SKILL_TABLE_POINT_POLY_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::szDurationPoly`.
pub const SKILL_TABLE_DURATION_POLY_OFFSET: usize =
    SKILL_TABLE_SP_COST_POLY_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::szDurationSPCostPoly`.
pub const SKILL_TABLE_DURATION_SP_COST_POLY_OFFSET: usize =
    SKILL_TABLE_DURATION_POLY_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::szCooldownPoly`.
pub const SKILL_TABLE_COOLDOWN_POLY_OFFSET: usize =
    SKILL_TABLE_DURATION_SP_COST_POLY_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::szMasterBonusPoly`.
pub const SKILL_TABLE_MASTER_BONUS_POLY_OFFSET: usize =
    SKILL_TABLE_COOLDOWN_POLY_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::szGrandMasterAddSPCostPoly`.
pub const SKILL_TABLE_GRAND_MASTER_ADD_SP_COST_POLY_OFFSET: usize =
    SKILL_TABLE_MASTER_BONUS_POLY_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::dwFlag`.
pub const SKILL_TABLE_FLAG_OFFSET: usize =
    SKILL_TABLE_GRAND_MASTER_ADD_SP_COST_POLY_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::dwAffectFlag`.
pub const SKILL_TABLE_AFFECT_FLAG_OFFSET: usize = SKILL_TABLE_FLAG_OFFSET + 4;
/// Byte offset of `TSkillTable::szPointOn2`.
pub const SKILL_TABLE_POINT_ON2_OFFSET: usize = SKILL_TABLE_AFFECT_FLAG_OFFSET + 4;
/// Byte offset of `TSkillTable::szPointPoly2`.
pub const SKILL_TABLE_POINT_POLY2_OFFSET: usize =
    SKILL_TABLE_POINT_ON2_OFFSET + SKILL_POINT_ON_BYTES;
/// Byte offset of `TSkillTable::szDurationPoly2`.
pub const SKILL_TABLE_DURATION_POLY2_OFFSET: usize =
    SKILL_TABLE_POINT_POLY2_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::dwAffectFlag2`.
pub const SKILL_TABLE_AFFECT_FLAG2_OFFSET: usize =
    SKILL_TABLE_DURATION_POLY2_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::szPointOn3`.
pub const SKILL_TABLE_POINT_ON3_OFFSET: usize = SKILL_TABLE_AFFECT_FLAG2_OFFSET + 4;
/// Byte offset of `TSkillTable::szPointPoly3`.
pub const SKILL_TABLE_POINT_POLY3_OFFSET: usize =
    SKILL_TABLE_POINT_ON3_OFFSET + SKILL_POINT_ON_BYTES;
/// Byte offset of `TSkillTable::szDurationPoly3`.
pub const SKILL_TABLE_DURATION_POLY3_OFFSET: usize =
    SKILL_TABLE_POINT_POLY3_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::bLevelStep`.
pub const SKILL_TABLE_LEVEL_STEP_OFFSET: usize =
    SKILL_TABLE_DURATION_POLY3_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::bLevelLimit`.
pub const SKILL_TABLE_LEVEL_LIMIT_OFFSET: usize = SKILL_TABLE_LEVEL_STEP_OFFSET + 1;
/// Byte offset of `TSkillTable::preSkillVnum`.
pub const SKILL_TABLE_PRE_SKILL_VNUM_OFFSET: usize = SKILL_TABLE_LEVEL_LIMIT_OFFSET + 1;
/// Byte offset of `TSkillTable::preSkillLevel`.
pub const SKILL_TABLE_PRE_SKILL_LEVEL_OFFSET: usize = SKILL_TABLE_PRE_SKILL_VNUM_OFFSET + 4;
/// Byte offset of x86 `TSkillTable::lMaxHit`.
pub const SKILL_TABLE_MAX_HIT_OFFSET: usize = SKILL_TABLE_PRE_SKILL_LEVEL_OFFSET + 1;
/// Byte offset of `TSkillTable::szSplashAroundDamageAdjustPoly`.
pub const SKILL_TABLE_SPLASH_AROUND_DAMAGE_ADJUST_POLY_OFFSET: usize =
    SKILL_TABLE_MAX_HIT_OFFSET + 4;
/// Byte offset of `TSkillTable::bSkillAttrType`.
pub const SKILL_TABLE_SKILL_ATTR_TYPE_OFFSET: usize =
    SKILL_TABLE_SPLASH_AROUND_DAMAGE_ADJUST_POLY_OFFSET + SKILL_POLY_EXPR_BYTES;
/// Byte offset of `TSkillTable::dwTargetRange`.
pub const SKILL_TABLE_TARGET_RANGE_OFFSET: usize = SKILL_TABLE_SKILL_ATTR_TYPE_OFFSET + 1;

const _: () = assert!(SKILL_TABLE_TARGET_RANGE_OFFSET + 4 == SKILL_TABLE_RECORD_WIRE_SIZE);

/// Maximum raw `TMobTable::szName` bytes, excluding the terminating NUL.
pub const MOB_NAME_MAX_LEN: usize = LEGACY_CHARACTER_NAME_BYTES - 1;
/// Number of raw bytes in `TMobTable::szName`, including its terminating NUL.
pub const MOB_NAME_BYTES: usize = MOB_NAME_MAX_LEN + 1;
/// Maximum raw `TMobTable::szLocaleName` bytes, excluding the terminating NUL.
pub const MOB_LOCALE_NAME_MAX_LEN: usize = MOB_NAME_MAX_LEN;
/// Number of raw bytes in `TMobTable::szLocaleName`.
pub const MOB_LOCALE_NAME_BYTES: usize = MOB_LOCALE_NAME_MAX_LEN + 1;
/// Maximum raw `TMobTable::szFolder` bytes, excluding the terminating NUL.
pub const MOB_FOLDER_MAX_LEN: usize = 64;
/// Number of raw bytes in `TMobTable::szFolder`, including its terminating NUL.
pub const MOB_FOLDER_BYTES: usize = MOB_FOLDER_MAX_LEN + 1;
/// Number of signed `char` slots in `TMobTable::cEnchants`.
pub const MOB_ENCHANTS_MAX_NUM: usize = 6;
/// Number of signed `char` slots in `TMobTable::cResists`.
pub const MOB_RESISTS_MAX_NUM: usize = 11;
/// Number of fixed skill slots in `TMobTable::Skills`.
pub const MOB_SKILL_MAX_NUM: usize = 5;
/// Number of `u32` entries in `TMobTable::dwDamageRange`.
pub const MOB_DAMAGE_RANGE_COUNT: usize = 2;
/// Packed x86 size of one `TMobSkillLevel` record.
pub const MOB_SKILL_RECORD_WIRE_SIZE: usize = 4 + 1;
/// Compatibility alias for [`MOB_SKILL_RECORD_WIRE_SIZE`].
pub const MOB_SKILL_WIRE_SIZE: usize = MOB_SKILL_RECORD_WIRE_SIZE;
/// Packed x86 size of `TMobTable` in the active legacy build.
pub const MOB_TABLE_RECORD_WIRE_SIZE: usize = 255;
/// Compatibility alias for [`MOB_TABLE_RECORD_WIRE_SIZE`].
pub const MOB_TABLE_WIRE_SIZE: usize = MOB_TABLE_RECORD_WIRE_SIZE;
/// Byte offset of the inherited `TMobTable::dwVnum` field.
pub const MOB_TABLE_VNUM_OFFSET: usize = 0;
/// Byte offset of `TMobTable::szName`.
pub const MOB_TABLE_NAME_OFFSET: usize = MOB_TABLE_VNUM_OFFSET + 4;
/// Byte offset of `TMobTable::szLocaleName`.
pub const MOB_TABLE_LOCALE_NAME_OFFSET: usize = MOB_TABLE_NAME_OFFSET + MOB_NAME_BYTES;
/// Byte offset of `TMobTable::bType`.
pub const MOB_TABLE_MOB_TYPE_OFFSET: usize = MOB_TABLE_LOCALE_NAME_OFFSET + MOB_LOCALE_NAME_BYTES;
/// Compatibility offset alias for `TMobTable::bType`.
pub const MOB_TABLE_TYPE_OFFSET: usize = MOB_TABLE_MOB_TYPE_OFFSET;
/// Byte offset of `TMobTable::bRank`.
pub const MOB_TABLE_RANK_OFFSET: usize = MOB_TABLE_MOB_TYPE_OFFSET + 1;
/// Byte offset of `TMobTable::bBattleType`.
pub const MOB_TABLE_BATTLE_TYPE_OFFSET: usize = MOB_TABLE_RANK_OFFSET + 1;
/// Byte offset of `TMobTable::bLevel`.
pub const MOB_TABLE_LEVEL_OFFSET: usize = MOB_TABLE_BATTLE_TYPE_OFFSET + 1;
/// Byte offset of `TMobTable::bSize`.
pub const MOB_TABLE_SIZE_OFFSET: usize = MOB_TABLE_LEVEL_OFFSET + 1;
/// Byte offset of `TMobTable::dwGoldMin`.
pub const MOB_TABLE_GOLD_MIN_OFFSET: usize = MOB_TABLE_SIZE_OFFSET + 1;
/// Byte offset of `TMobTable::dwGoldMax`.
pub const MOB_TABLE_GOLD_MAX_OFFSET: usize = MOB_TABLE_GOLD_MIN_OFFSET + 4;
/// Byte offset of `TMobTable::dwExp`.
pub const MOB_TABLE_EXP_OFFSET: usize = MOB_TABLE_GOLD_MAX_OFFSET + 4;
/// Byte offset of `TMobTable::dwMaxHP`.
pub const MOB_TABLE_MAX_HP_OFFSET: usize = MOB_TABLE_EXP_OFFSET + 4;
/// Byte offset of `TMobTable::bRegenCycle`.
pub const MOB_TABLE_REGEN_CYCLE_OFFSET: usize = MOB_TABLE_MAX_HP_OFFSET + 4;
/// Byte offset of `TMobTable::bRegenPercent`.
pub const MOB_TABLE_REGEN_PERCENT_OFFSET: usize = MOB_TABLE_REGEN_CYCLE_OFFSET + 1;
/// Byte offset of `TMobTable::wDef`.
pub const MOB_TABLE_DEF_OFFSET: usize = MOB_TABLE_REGEN_PERCENT_OFFSET + 1;
/// Byte offset of `TMobTable::dwAIFlag`.
pub const MOB_TABLE_AI_FLAG_OFFSET: usize = MOB_TABLE_DEF_OFFSET + 2;
/// Byte offset of `TMobTable::dwRaceFlag`.
pub const MOB_TABLE_RACE_FLAG_OFFSET: usize = MOB_TABLE_AI_FLAG_OFFSET + 4;
/// Byte offset of `TMobTable::dwImmuneFlag`.
pub const MOB_TABLE_IMMUNE_FLAG_OFFSET: usize = MOB_TABLE_RACE_FLAG_OFFSET + 4;
/// Byte offset of `TMobTable::bStr`.
pub const MOB_TABLE_STR_OFFSET: usize = MOB_TABLE_IMMUNE_FLAG_OFFSET + 4;
/// Byte offset of `TMobTable::bDex`.
pub const MOB_TABLE_DEX_OFFSET: usize = MOB_TABLE_STR_OFFSET + 1;
/// Byte offset of `TMobTable::bCon`.
pub const MOB_TABLE_CON_OFFSET: usize = MOB_TABLE_DEX_OFFSET + 1;
/// Byte offset of `TMobTable::bInt`.
pub const MOB_TABLE_INT_OFFSET: usize = MOB_TABLE_CON_OFFSET + 1;
/// Byte offset of `TMobTable::dwDamageRange`.
pub const MOB_TABLE_DAMAGE_RANGE_OFFSET: usize = MOB_TABLE_INT_OFFSET + 1;
/// Byte offset of `TMobTable::sAttackSpeed`.
pub const MOB_TABLE_ATTACK_SPEED_OFFSET: usize =
    MOB_TABLE_DAMAGE_RANGE_OFFSET + MOB_DAMAGE_RANGE_COUNT * 4;
/// Byte offset of `TMobTable::sMovingSpeed`.
pub const MOB_TABLE_MOVING_SPEED_OFFSET: usize = MOB_TABLE_ATTACK_SPEED_OFFSET + 2;
/// Byte offset of `TMobTable::bAggresiveHPPct`.
pub const MOB_TABLE_AGGRESSIVE_HP_PCT_OFFSET: usize = MOB_TABLE_MOVING_SPEED_OFFSET + 2;
/// Byte offset of `TMobTable::wAggressiveSight`.
pub const MOB_TABLE_AGGRESSIVE_SIGHT_OFFSET: usize = MOB_TABLE_AGGRESSIVE_HP_PCT_OFFSET + 1;
/// Byte offset of `TMobTable::wAttackRange`.
pub const MOB_TABLE_ATTACK_RANGE_OFFSET: usize = MOB_TABLE_AGGRESSIVE_SIGHT_OFFSET + 2;
/// Byte offset of `TMobTable::cEnchants`.
pub const MOB_TABLE_ENCHANTS_OFFSET: usize = MOB_TABLE_ATTACK_RANGE_OFFSET + 2;
/// Byte offset of `TMobTable::cResists`.
pub const MOB_TABLE_RESISTS_OFFSET: usize = MOB_TABLE_ENCHANTS_OFFSET + MOB_ENCHANTS_MAX_NUM;
/// Byte offset of `TMobTable::dwResurrectionVnum`.
pub const MOB_TABLE_RESURRECTION_VNUM_OFFSET: usize =
    MOB_TABLE_RESISTS_OFFSET + MOB_RESISTS_MAX_NUM;
/// Byte offset of `TMobTable::dwDropItemVnum`.
pub const MOB_TABLE_DROP_ITEM_VNUM_OFFSET: usize = MOB_TABLE_RESURRECTION_VNUM_OFFSET + 4;
/// Byte offset of `TMobTable::bMountCapacity`.
pub const MOB_TABLE_MOUNT_CAPACITY_OFFSET: usize = MOB_TABLE_DROP_ITEM_VNUM_OFFSET + 4;
/// Byte offset of `TMobTable::bOnClickType`.
pub const MOB_TABLE_ON_CLICK_TYPE_OFFSET: usize = MOB_TABLE_MOUNT_CAPACITY_OFFSET + 1;
/// Byte offset of `TMobTable::bEmpire`.
pub const MOB_TABLE_EMPIRE_OFFSET: usize = MOB_TABLE_ON_CLICK_TYPE_OFFSET + 1;
/// Byte offset of `TMobTable::szFolder`.
pub const MOB_TABLE_FOLDER_OFFSET: usize = MOB_TABLE_EMPIRE_OFFSET + 1;
/// Byte offset of `TMobTable::fDamMultiply`.
pub const MOB_TABLE_DAM_MULTIPLY_OFFSET: usize = MOB_TABLE_FOLDER_OFFSET + MOB_FOLDER_BYTES;
/// Byte offset of `TMobTable::dwSummonVnum`.
pub const MOB_TABLE_SUMMON_VNUM_OFFSET: usize = MOB_TABLE_DAM_MULTIPLY_OFFSET + 4;
/// Byte offset of `TMobTable::dwDrainSP`.
pub const MOB_TABLE_DRAIN_SP_OFFSET: usize = MOB_TABLE_SUMMON_VNUM_OFFSET + 4;
/// Byte offset of `TMobTable::dwMobColor`.
pub const MOB_TABLE_MOB_COLOR_OFFSET: usize = MOB_TABLE_DRAIN_SP_OFFSET + 4;
/// Byte offset of `TMobTable::dwPolymorphItemVnum`.
pub const MOB_TABLE_POLYMORPH_ITEM_VNUM_OFFSET: usize = MOB_TABLE_MOB_COLOR_OFFSET + 4;
/// Byte offset of `TMobTable::Skills`.
pub const MOB_TABLE_SKILLS_OFFSET: usize = MOB_TABLE_POLYMORPH_ITEM_VNUM_OFFSET + 4;
/// Byte offset of `TMobTable::bBerserkPoint`.
pub const MOB_TABLE_BERSERK_POINT_OFFSET: usize =
    MOB_TABLE_SKILLS_OFFSET + MOB_SKILL_MAX_NUM * MOB_SKILL_RECORD_WIRE_SIZE;
/// Byte offset of `TMobTable::bStoneSkinPoint`.
pub const MOB_TABLE_STONE_SKIN_POINT_OFFSET: usize = MOB_TABLE_BERSERK_POINT_OFFSET + 1;
/// Byte offset of `TMobTable::bGodSpeedPoint`.
pub const MOB_TABLE_GOD_SPEED_POINT_OFFSET: usize = MOB_TABLE_STONE_SKIN_POINT_OFFSET + 1;
/// Byte offset of `TMobTable::bDeathBlowPoint`.
pub const MOB_TABLE_DEATH_BLOW_POINT_OFFSET: usize = MOB_TABLE_GOD_SPEED_POINT_OFFSET + 1;
/// Byte offset of `TMobTable::bRevivePoint`.
pub const MOB_TABLE_REVIVE_POINT_OFFSET: usize = MOB_TABLE_DEATH_BLOW_POINT_OFFSET + 1;

const _: () = assert!(MOB_TABLE_REVIVE_POINT_OFFSET + 1 == MOB_TABLE_RECORD_WIRE_SIZE);
const _: () = assert!(MOB_SKILL_RECORD_WIRE_SIZE == 5);
const _: () = assert!(MOB_NAME_BYTES == 25);
const _: () = assert!(MOB_LOCALE_NAME_BYTES == 25);
const _: () = assert!(MOB_FOLDER_BYTES == 65);

/// Number of material slots in the active legacy `TRefineTable`.
pub const REFINE_MATERIAL_MAX_NUM: usize = 5;
/// Packed x86 size of `TRefineMaterial` in the active legacy build.
pub const REFINE_MATERIAL_WIRE_SIZE: usize = 8;
/// Packed x86 size of `TRefineTable` in the active legacy build.
pub const REFINE_TABLE_WIRE_SIZE: usize = 53;
/// Number of raw bytes in each active `TItemTable` name field, including its
/// terminating NUL.
pub const ITEM_NAME_BYTES: usize = 37;
/// Maximum item-name bytes in the C++ source, excluding the terminator.
pub const ITEM_NAME_MAX_LEN: usize = ITEM_NAME_BYTES - 1;
/// Number of item limit slots in the active legacy build.
pub const ITEM_LIMIT_MAX_NUM: usize = 2;
/// Number of item apply slots in the active legacy build.
pub const ITEM_APPLY_MAX_NUM: usize = 3;
/// Number of signed item value slots in the active legacy build.
pub const ITEM_VALUES_MAX_NUM: usize = 6;
/// Number of item socket slots in the active extended-socket build.
pub const ITEM_SOCKET_MAX_NUM: usize = crate::ITEM_SOCKET_MAX_NUM;
/// Packed x86 size of one `TItemLimit` entry (`BYTE` plus x86 `long`).
pub const ITEM_LIMIT_RECORD_WIRE_SIZE: usize = 1 + 4;
/// Packed x86 size of one `TItemApply` entry (`BYTE` plus x86 `long`).
pub const ITEM_APPLY_RECORD_WIRE_SIZE: usize = 1 + 4;
/// Packed x86 size of `TItemTable` in the active build.
pub const ITEM_TABLE_RECORD_WIRE_SIZE: usize = 204;
/// Byte offset of `TItemTable::dwVnum` in the inherited `SEntityTable` prefix.
pub const ITEM_TABLE_VNUM_OFFSET: usize = 0;
/// Byte offset of `TItemTable::dwVnumRange`.
pub const ITEM_TABLE_VNUM_RANGE_OFFSET: usize = ITEM_TABLE_VNUM_OFFSET + 4;
/// Byte offset of `TItemTable::szName`.
pub const ITEM_TABLE_NAME_OFFSET: usize = ITEM_TABLE_VNUM_RANGE_OFFSET + 4;
/// Byte offset of `TItemTable::szLocaleName`.
pub const ITEM_TABLE_LOCALE_NAME_OFFSET: usize = ITEM_TABLE_NAME_OFFSET + ITEM_NAME_BYTES;
/// Byte offset of `TItemTable::bType`.
pub const ITEM_TABLE_ITEM_TYPE_OFFSET: usize = ITEM_TABLE_LOCALE_NAME_OFFSET + ITEM_NAME_BYTES;
/// Byte offset of `TItemTable::bSubType`.
pub const ITEM_TABLE_SUB_TYPE_OFFSET: usize = ITEM_TABLE_ITEM_TYPE_OFFSET + 1;
/// Byte offset of `TItemTable::bWeight`.
pub const ITEM_TABLE_WEIGHT_OFFSET: usize = ITEM_TABLE_SUB_TYPE_OFFSET + 1;
/// Byte offset of `TItemTable::bSize`.
pub const ITEM_TABLE_SIZE_OFFSET: usize = ITEM_TABLE_WEIGHT_OFFSET + 1;
/// Byte offset of `TItemTable::dwAntiFlags`.
pub const ITEM_TABLE_ANTI_FLAGS_OFFSET: usize = ITEM_TABLE_SIZE_OFFSET + 1;
/// Byte offset of `TItemTable::dwFlags`.
pub const ITEM_TABLE_FLAGS_OFFSET: usize = ITEM_TABLE_ANTI_FLAGS_OFFSET + 4;
/// Byte offset of `TItemTable::dwWearFlags`.
pub const ITEM_TABLE_WEAR_FLAGS_OFFSET: usize = ITEM_TABLE_FLAGS_OFFSET + 4;
/// Byte offset of `TItemTable::dwImmuneFlag`.
pub const ITEM_TABLE_IMMUNE_FLAG_OFFSET: usize = ITEM_TABLE_WEAR_FLAGS_OFFSET + 4;
/// Byte offset of the active 64-bit `TItemTable::dwGold` field.
pub const ITEM_TABLE_GOLD_OFFSET: usize = ITEM_TABLE_IMMUNE_FLAG_OFFSET + 4;
/// Byte offset of the active 64-bit `TItemTable::dwShopBuyPrice` field.
pub const ITEM_TABLE_SHOP_BUY_PRICE_OFFSET: usize = ITEM_TABLE_GOLD_OFFSET + 8;
/// Byte offset of `TItemTable::aLimits`.
pub const ITEM_TABLE_LIMITS_OFFSET: usize = ITEM_TABLE_SHOP_BUY_PRICE_OFFSET + 8;
/// Byte offset of `TItemTable::aApplies`.
pub const ITEM_TABLE_APPLIES_OFFSET: usize =
    ITEM_TABLE_LIMITS_OFFSET + ITEM_LIMIT_MAX_NUM * ITEM_LIMIT_RECORD_WIRE_SIZE;
/// Byte offset of `TItemTable::alValues`.
pub const ITEM_TABLE_VALUES_OFFSET: usize =
    ITEM_TABLE_APPLIES_OFFSET + ITEM_APPLY_MAX_NUM * ITEM_APPLY_RECORD_WIRE_SIZE;
/// Byte offset of `TItemTable::alSockets`.
pub const ITEM_TABLE_SOCKETS_OFFSET: usize = ITEM_TABLE_VALUES_OFFSET + ITEM_VALUES_MAX_NUM * 4;
/// Byte offset of `TItemTable::dwRefinedVnum`.
pub const ITEM_TABLE_REFINED_VNUM_OFFSET: usize =
    ITEM_TABLE_SOCKETS_OFFSET + ITEM_SOCKET_MAX_NUM * 4;
/// Byte offset of `TItemTable::wRefineSet`.
pub const ITEM_TABLE_REFINE_SET_OFFSET: usize = ITEM_TABLE_REFINED_VNUM_OFFSET + 4;
/// Byte offset of `TItemTable::bAlterToMagicItemPct`.
pub const ITEM_TABLE_ALTER_TO_MAGIC_ITEM_PCT_OFFSET: usize = ITEM_TABLE_REFINE_SET_OFFSET + 2;
/// Byte offset of `TItemTable::bSpecular`.
pub const ITEM_TABLE_SPECULAR_OFFSET: usize = ITEM_TABLE_ALTER_TO_MAGIC_ITEM_PCT_OFFSET + 1;
/// Byte offset of `TItemTable::bGainSocketPct`.
pub const ITEM_TABLE_GAIN_SOCKET_PCT_OFFSET: usize = ITEM_TABLE_SPECULAR_OFFSET + 1;
/// Byte offset of `TItemTable::sAddonType`.
pub const ITEM_TABLE_ADDON_TYPE_OFFSET: usize = ITEM_TABLE_GAIN_SOCKET_PCT_OFFSET + 1;
/// Byte offset of `TItemTable::cLimitRealTimeFirstUseIndex`.
pub const ITEM_TABLE_LIMIT_REAL_TIME_FIRST_USE_INDEX_OFFSET: usize =
    ITEM_TABLE_ADDON_TYPE_OFFSET + 2;
/// Byte offset of `TItemTable::cLimitTimerBasedOnWearIndex`.
pub const ITEM_TABLE_LIMIT_TIMER_BASED_ON_WEAR_INDEX_OFFSET: usize =
    ITEM_TABLE_LIMIT_REAL_TIME_FIRST_USE_INDEX_OFFSET + 1;

const _: () =
    assert!(ITEM_TABLE_LIMIT_TIMER_BASED_ON_WEAR_INDEX_OFFSET + 1 == ITEM_TABLE_RECORD_WIRE_SIZE);
const _: () = assert!(ITEM_NAME_BYTES == 37);
const _: () = assert!(ITEM_LIMIT_MAX_NUM == 2);
const _: () = assert!(ITEM_APPLY_MAX_NUM == 3);
const _: () = assert!(ITEM_VALUES_MAX_NUM == 6);
const _: () = assert!(ITEM_SOCKET_MAX_NUM == 6);
const _: () = assert!(ITEM_LIMIT_RECORD_WIRE_SIZE == 5);
const _: () = assert!(ITEM_APPLY_RECORD_WIRE_SIZE == 5);
/// Maximum apply-name bytes in the C++ source, excluding the terminator.
pub const APPLY_NAME_MAX_LEN: usize = 32;
/// Number of signed level values in `TItemAttrTable::lValues`.
pub const ITEM_ATTRIBUTE_MAX_LEVEL: usize = 5;
/// Number of equipment-set slots in the active glove-enabled build.
pub const ATTRIBUTE_SET_MAX_NUM: usize = 10;
/// Packed x86 size of `TItemAttrTable` in the active legacy build.
pub const ITEM_ATTR_RECORD_WIRE_SIZE: usize =
    APPLY_NAME_MAX_LEN + 1 + 4 + 4 + ITEM_ATTRIBUTE_MAX_LEVEL * 4 + ATTRIBUTE_SET_MAX_NUM;
/// Number of fixed item slots in the active base `TShopTable` profile.
pub const SHOP_HOST_ITEM_MAX_NUM: usize = crate::SHOP_HOST_ITEM_MAX_NUM;
/// Number of item sockets in the active `TShopItemTable` profile.
pub const SHOP_ITEM_SOCKET_MAX_NUM: usize = crate::ITEM_SOCKET_MAX_NUM;
/// Number of item attributes in the active `TShopItemTable` profile.
pub const SHOP_ITEM_ATTRIBUTE_MAX_NUM: usize = crate::ITEM_ATTRIBUTE_MAX_NUM;
/// Fixed packed size of the active `TItemPos` value.
pub const SHOP_ITEM_POSITION_WIRE_SIZE: usize = 3;
/// Fixed packed size of one `TPlayerItemAttribute` value.
pub const SHOP_ITEM_ATTRIBUTE_WIRE_SIZE: usize = 3;
/// Maximum raw shop-sign bytes, excluding the NUL terminator.
pub const SHOP_SIGN_MAX_LEN: usize = 32;
/// Number of raw bytes in the active packed `TShopTable::szShopName` field.
pub const SHOP_SIGN_BYTES: usize = SHOP_SIGN_MAX_LEN + 1;
/// Packed x86 size of one active-profile `TShopItemTable` record.
pub const SHOP_ITEM_RECORD_WIRE_SIZE: usize = 68;
/// Packed x86 size of one active-profile `TShopTable` record.
pub const SHOP_TABLE_RECORD_WIRE_SIZE: usize = 2762;
/// Compatibility alias for [`SHOP_ITEM_RECORD_WIRE_SIZE`].
pub const SHOP_ITEM_WIRE_SIZE: usize = SHOP_ITEM_RECORD_WIRE_SIZE;
/// Compatibility alias for [`SHOP_TABLE_RECORD_WIRE_SIZE`].
pub const SHOP_TABLE_WIRE_SIZE: usize = SHOP_TABLE_RECORD_WIRE_SIZE;
/// Compatibility alias using the legacy `TShopItemTable` name.
pub const SHOP_ITEM_TABLE_WIRE_SIZE: usize = SHOP_ITEM_RECORD_WIRE_SIZE;
/// Byte offset of `TShopItemTable::vnum`.
pub const SHOP_ITEM_VNUM_OFFSET: usize = 0;
/// Byte offset of `TShopItemTable::count`.
pub const SHOP_ITEM_COUNT_OFFSET: usize = SHOP_ITEM_VNUM_OFFSET + 4;
/// Byte offset of `TShopItemTable::pos`.
pub const SHOP_ITEM_POSITION_OFFSET: usize = SHOP_ITEM_COUNT_OFFSET + 2;
/// Byte offset of `TShopItemTable::price` in the active 64-bit-price profile.
pub const SHOP_ITEM_PRICE_OFFSET: usize = SHOP_ITEM_POSITION_OFFSET + SHOP_ITEM_POSITION_WIRE_SIZE;
/// Byte offset of `TShopItemTable::display_pos`.
pub const SHOP_ITEM_DISPLAY_POS_OFFSET: usize = SHOP_ITEM_PRICE_OFFSET + 8;
/// Byte offset of `TShopItemTable::alSockets`.
pub const SHOP_ITEM_SOCKETS_OFFSET: usize = SHOP_ITEM_DISPLAY_POS_OFFSET + 1;
/// Byte offset of `TShopItemTable::aAttr`.
pub const SHOP_ITEM_ATTRS_OFFSET: usize = SHOP_ITEM_SOCKETS_OFFSET + SHOP_ITEM_SOCKET_MAX_NUM * 4;
/// Byte offset of `TShopItemTable::price_type`.
pub const SHOP_ITEM_PRICE_TYPE_OFFSET: usize =
    SHOP_ITEM_ATTRS_OFFSET + SHOP_ITEM_ATTRIBUTE_MAX_NUM * SHOP_ITEM_ATTRIBUTE_WIRE_SIZE;
/// Byte offset of `TShopItemTable::price_vnum`.
pub const SHOP_ITEM_PRICE_VNUM_OFFSET: usize = SHOP_ITEM_PRICE_TYPE_OFFSET + 1;
/// Byte offset of `TShopTable::dwVnum`.
pub const SHOP_TABLE_VNUM_OFFSET: usize = 0;
/// Byte offset of `TShopTable::dwNPCVnum`.
pub const SHOP_TABLE_NPC_VNUM_OFFSET: usize = SHOP_TABLE_VNUM_OFFSET + 4;
/// Byte offset of `TShopTable::byItemCount`.
pub const SHOP_TABLE_ITEM_COUNT_OFFSET: usize = SHOP_TABLE_NPC_VNUM_OFFSET + 4;
/// Byte offset of `TShopTable::items`.
pub const SHOP_TABLE_ITEMS_OFFSET: usize = SHOP_TABLE_ITEM_COUNT_OFFSET + 1;
/// Byte offset of `TShopTable::szShopName`.
pub const SHOP_TABLE_SHOP_NAME_OFFSET: usize =
    SHOP_TABLE_ITEMS_OFFSET + SHOP_HOST_ITEM_MAX_NUM * SHOP_ITEM_RECORD_WIRE_SIZE;
/// `SHOPEX_GOLD`, the legacy default price selector for a shop item.
pub const SHOP_PRICE_TYPE_GOLD: u8 = 1;
/// The active `EWindows::INVENTORY` value used by `TItemPos`'s default ctor.
pub const SHOP_ITEM_DEFAULT_WINDOW_TYPE: u8 = 1;
/// The active `TItemPos` default cell (`WORD_MAX`).
pub const SHOP_ITEM_DEFAULT_CELL: u16 = u16::MAX;

const _: () = assert!(SHOP_ITEM_PRICE_VNUM_OFFSET + 4 == SHOP_ITEM_RECORD_WIRE_SIZE);
const _: () = assert!(SHOP_TABLE_SHOP_NAME_OFFSET + SHOP_SIGN_BYTES == SHOP_TABLE_RECORD_WIRE_SIZE);

/// Fixed 36-byte x86 wire size of `building::TLand` in the active legacy build.
pub const LAND_RECORD_WIRE_SIZE: usize = 36;
/// Number of material slots in the active legacy `building::TObjectProto`.
pub const OBJECT_MATERIAL_MAX_NUM: usize = 5;
/// Fixed 8-byte x86 wire size of `building::TObjectMaterial`.
pub const OBJECT_MATERIAL_WIRE_SIZE: usize = 8;
/// Number of signed region values in `building::TObjectProto`.
pub const OBJECT_PROTO_REGION_COUNT: usize = 4;
/// Fixed 96-byte x86 wire size of `building::TObjectProto`.
pub const OBJECT_PROTO_RECORD_WIRE_SIZE: usize = 96;
/// Compatibility alias for [`OBJECT_PROTO_RECORD_WIRE_SIZE`].
pub const OBJECT_PROTO_WIRE_SIZE: usize = OBJECT_PROTO_RECORD_WIRE_SIZE;
/// Fixed 40-byte x86 wire size of `building::TObject` in the active legacy build.
pub const OBJECT_RECORD_WIRE_SIZE: usize = 40;
/// Packed x86 size of the `HEADER_GD_REQ_HORSE_NAME` request payload.
pub const HORSE_NAME_REQUEST_WIRE_SIZE: usize = 4;
/// Packed x86 size of the `HEADER_DG_ACK_HORSE_NAME` response payload.
pub const HORSE_NAME_RECORD_WIRE_SIZE: usize = 29;
/// Packed x86 size of the `HEADER_GD_FIND_CHANNEL` request payload.
pub const CHANNEL_CHANGE_REQUEST_WIRE_SIZE: usize = 8;
/// Packed x86 size of the `HEADER_DG_CHANNEL_RESULT` response payload.
pub const CHANNEL_RESULT_WIRE_SIZE: usize = 6;
/// Packed x86 size of `TPacketAffectElement`.
pub const AFFECT_ELEMENT_WIRE_SIZE: usize = 21;
/// Bytes in the `HEADER_DG_AFFECT_LOAD` player-ID and row-count prefix.
pub const AFFECT_LOAD_PREFIX_WIRE_SIZE: usize = 8;
/// Packed x86 size of the `HEADER_GD_ADD_AFFECT` request payload.
pub const ADD_AFFECT_REQUEST_WIRE_SIZE: usize = 25;
/// Packed x86 size of the `HEADER_GD_REMOVE_AFFECT` request payload.
pub const REMOVE_AFFECT_REQUEST_WIRE_SIZE: usize = 9;

const _: () = assert!(ADD_AFFECT_REQUEST_WIRE_SIZE == 4 + AFFECT_ELEMENT_WIRE_SIZE);

/// Safe default bound for a `QUEST_LOAD` vector.
///
/// The C++ query can technically return more rows, but an untrusted count
/// must not cause an unbounded allocation in the Rust peer.  Applications
/// with a different DB policy can use `decode_quest_load_with_limit`.
pub const MAX_QUEST_RECORDS: usize = 65_535;

/// GD/DG headers involved in the first login-path records.
pub const HEADER_GD_LOGIN_BY_KEY: u8 = 101;
/// Header carrying a successful account result.
pub const HEADER_DG_LOGIN_SUCCESS: u8 = 30;
/// Header emitted when a login key/account lookup has no usable account.
pub const HEADER_DG_LOGIN_NOT_EXIST: u8 = 31;
/// Header emitted for the legacy wrong-password path.
pub const HEADER_DG_LOGIN_WRONG_PASSWD: u8 = 33;
/// Header carrying an already-logged-in account name.
pub const HEADER_DG_LOGIN_ALREADY: u8 = 34;
/// Header carrying a successful player-load result.
pub const HEADER_DG_PLAYER_LOAD_SUCCESS: u8 = 35;
/// Header emitted when a player-load result cannot be produced.
pub const HEADER_DG_PLAYER_LOAD_FAILED: u8 = 36;
/// GD header that requests a player load.
pub const HEADER_GD_PLAYER_LOAD: u8 = 3;
/// DB header that carries the count-prefixed quest-load payload.
pub const HEADER_DG_QUEST_LOAD: u8 = 44;
/// GD header that requests a horse name for one player ID.
pub const HEADER_GD_REQ_HORSE_NAME: u8 = 132;
/// DG header that acknowledges a horse-name lookup.
pub const HEADER_DG_ACK_HORSE_NAME: u8 = 176;
/// GD header that requests an alternate game channel.
pub const HEADER_GD_FIND_CHANNEL: u8 = 135;
/// DG header that returns the alternate channel endpoint.
pub const HEADER_DG_CHANNEL_RESULT: u8 = 185;
/// GD header that requests addition of one player affect.
pub const HEADER_GD_ADD_AFFECT: u8 = 32;
/// GD header that requests removal of one player affect.
pub const HEADER_GD_REMOVE_AFFECT: u8 = 33;
/// DB header that carries a player's affect elements.
pub const HEADER_DG_AFFECT_LOAD: u8 = 50;

/// Compatibility aliases for callers that use the C++ type names.
pub const T_SIMPLE_PLAYER_SIZE: usize = SIMPLE_PLAYER_WIRE_SIZE;
/// Packed x86 size of `TAccountTable`.
pub const T_ACCOUNT_TABLE_SIZE: usize = LOGIN_ACCOUNT_WIRE_SIZE;
/// Packed x86 size of `TPlayerTable`.
pub const T_PLAYER_TABLE_SIZE: usize = PLAYER_RESULT_WIRE_SIZE;
/// Packed x86 size of `TQuestTable`.
pub const T_QUEST_TABLE_SIZE: usize = QUEST_RECORD_WIRE_SIZE;
/// Packed x86 size of `TMarketItemPrice`.
pub const T_MARKET_ITEM_PRICE_SIZE: usize = MARKET_ITEM_PRICE_WIRE_SIZE;
/// Packed x86 size of `TEventTable`.
pub const T_EVENT_TABLE_SIZE: usize = EVENT_TABLE_WIRE_SIZE;
/// Packed x86 size of `TSkillTable`.
pub const T_SKILL_TABLE_SIZE: usize = SKILL_TABLE_RECORD_WIRE_SIZE;
/// Packed x86 size of `TMobSkillLevel`.
pub const T_MOB_SKILL_LEVEL_SIZE: usize = MOB_SKILL_RECORD_WIRE_SIZE;
/// Compatibility alias for the packed x86 size of `TMobTable`.
pub const T_MOB_TABLE_SIZE: usize = MOB_TABLE_RECORD_WIRE_SIZE;
/// Packed x86 size of `TRefineMaterial`.
pub const T_REFINE_MATERIAL_SIZE: usize = REFINE_MATERIAL_WIRE_SIZE;
/// Packed x86 size of `TRefineTable`.
pub const T_REFINE_TABLE_SIZE: usize = REFINE_TABLE_WIRE_SIZE;
/// Packed x86 size of `TItemAttrTable`.
pub const T_ITEM_ATTR_TABLE_SIZE: usize = ITEM_ATTR_RECORD_WIRE_SIZE;
/// Compatibility alias for the packed x86 size of `TItemTable`.
pub const T_ITEM_TABLE_SIZE: usize = ITEM_TABLE_RECORD_WIRE_SIZE;
/// Packed x86 size of `TShopItemTable`.
pub const T_SHOP_ITEM_TABLE_SIZE: usize = SHOP_ITEM_RECORD_WIRE_SIZE;
/// Packed x86 size of `TShopTable`.
pub const T_SHOP_TABLE_SIZE: usize = SHOP_TABLE_RECORD_WIRE_SIZE;
/// Packed x86 size of `TObjectMaterial`.
pub const T_OBJECT_MATERIAL_SIZE: usize = OBJECT_MATERIAL_WIRE_SIZE;
/// Packed x86 size of `building::TObjectProto`.
pub const T_OBJECT_PROTO_SIZE: usize = OBJECT_PROTO_RECORD_WIRE_SIZE;
/// Packed x86 size of the horse-name request payload.
pub const T_HORSE_NAME_REQUEST_SIZE: usize = HORSE_NAME_REQUEST_WIRE_SIZE;
/// Packed x86 size of the horse-name response record.
pub const T_HORSE_NAME_RECORD_SIZE: usize = HORSE_NAME_RECORD_WIRE_SIZE;
/// Packed x86 size of `TPacketChangeChannel`.
pub const T_PACKET_CHANGE_CHANNEL_SIZE: usize = CHANNEL_CHANGE_REQUEST_WIRE_SIZE;
/// Packed x86 size of `TPacketReturnChannel`.
pub const T_PACKET_RETURN_CHANNEL_SIZE: usize = CHANNEL_RESULT_WIRE_SIZE;
/// Packed x86 size of `TPacketAffectElement`.
pub const T_PACKET_AFFECT_ELEMENT_SIZE: usize = AFFECT_ELEMENT_WIRE_SIZE;
/// Packed x86 size of `TPacketGDAddAffect`.
pub const T_PACKET_GD_ADD_AFFECT_SIZE: usize = ADD_AFFECT_REQUEST_WIRE_SIZE;
/// Packed x86 size of `TPacketGDRemoveAffect`.
pub const T_PACKET_REMOVE_AFFECT_SIZE: usize = REMOVE_AFFECT_REQUEST_WIRE_SIZE;

/// An error returned while decoding or encoding a DB record payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbRecordError {
    /// The input ended before a required field or the complete record.
    Truncated {
        /// Logical record being decoded.
        record: &'static str,
        /// Number of bytes required by the record/field.
        needed: usize,
        /// Number of bytes actually available.
        available: usize,
    },
    /// A fixed-width record had bytes beyond its exact wire size.
    LengthMismatch {
        /// Logical record being decoded.
        record: &'static str,
        /// Exact wire size required.
        expected: usize,
        /// Size supplied by the caller.
        actual: usize,
    },
    /// An affect count cannot be represented by the platform or wire integer.
    AffectCountOverflow {
        /// Unrepresentable row count.
        count: u128,
    },
    /// The exact affect payload size overflows the platform size type.
    AffectPayloadSizeOverflow {
        /// Row count whose size cannot be represented.
        count: usize,
    },
    /// An affect count exceeds the caller-selected safety bound.
    TooManyAffectRecords {
        /// Count supplied or read from the payload.
        count: u32,
        /// Maximum accepted by the caller.
        maximum: usize,
    },
    /// Storage for an affect payload or its element vector could not be reserved.
    AffectAllocationFailed {
        /// Number of affect elements whose storage was requested.
        count: usize,
    },
    /// A quest count exceeds the configured safety bound.
    TooManyQuestRecords {
        /// Count read from the payload.
        count: u32,
        /// Maximum accepted by the decoder.
        maximum: usize,
    },
}

impl fmt::Display for DbRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                record,
                needed,
                available,
            } => write!(
                f,
                "{record} is truncated: need {needed} bytes, have {available}"
            ),
            Self::LengthMismatch {
                record,
                expected,
                actual,
            } => write!(
                f,
                "{record} has {actual} bytes; expected exactly {expected}"
            ),
            Self::AffectCountOverflow { count } => write!(
                f,
                "AFFECT_LOAD count {count} cannot be represented by the required integer type"
            ),
            Self::AffectPayloadSizeOverflow { count } => write!(
                f,
                "AFFECT_LOAD payload size overflows usize for {count} elements"
            ),
            Self::TooManyAffectRecords { count, maximum } => write!(
                f,
                "AFFECT_LOAD count {count} exceeds the safety bound {maximum}"
            ),
            Self::AffectAllocationFailed { count } => {
                write!(f, "cannot reserve storage for {count} AFFECT_LOAD elements")
            }
            Self::TooManyQuestRecords { count, maximum } => write!(
                f,
                "QUEST_LOAD count {count} exceeds the safety bound {maximum}"
            ),
        }
    }
}

impl Error for DbRecordError {}

impl From<DbRecordError> for io::Error {
    fn from(error: DbRecordError) -> Self {
        let kind = match error {
            DbRecordError::Truncated { .. } => io::ErrorKind::UnexpectedEof,
            DbRecordError::LengthMismatch { .. }
            | DbRecordError::AffectCountOverflow { .. }
            | DbRecordError::AffectPayloadSizeOverflow { .. }
            | DbRecordError::TooManyAffectRecords { .. }
            | DbRecordError::TooManyQuestRecords { .. } => io::ErrorKind::InvalidData,
            DbRecordError::AffectAllocationFailed { .. } => io::ErrorKind::OutOfMemory,
        };
        Self::new(kind, error)
    }
}

/// Result type used by the record codecs.
pub type DbRecordResult<T> = Result<T, DbRecordError>;

/// A small manual byte reader used by every decoder in this module.
struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
    record: &'static str,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], record: &'static str) -> Self {
        Self {
            data,
            offset: 0,
            record,
        }
    }

    fn take(&mut self, length: usize) -> DbRecordResult<&'a [u8]> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(DbRecordError::Truncated {
                record: self.record,
                needed: usize::MAX,
                available: self.data.len(),
            })?;
        if end > self.data.len() {
            return Err(DbRecordError::Truncated {
                record: self.record,
                needed: end,
                available: self.data.len(),
            });
        }
        let result = &self.data[self.offset..end];
        self.offset = end;
        Ok(result)
    }

    fn u8(&mut self) -> DbRecordResult<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> DbRecordResult<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn i16(&mut self) -> DbRecordResult<i16> {
        let bytes = self.take(2)?;
        Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> DbRecordResult<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i32(&mut self) -> DbRecordResult<i32> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i64(&mut self) -> DbRecordResult<i64> {
        let bytes = self.take(8)?;
        Ok(i64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn u64(&mut self) -> DbRecordResult<u64> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn array<const N: usize>(&mut self) -> DbRecordResult<[u8; N]> {
        let bytes = self.take(N)?;
        let mut result = [0_u8; N];
        result.copy_from_slice(bytes);
        Ok(result)
    }
}

/// A small manual byte writer.  Every integer is written little endian.
#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i16(&mut self, value: i16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }
}

fn check_exact(record: &'static str, data: &[u8], expected: usize) -> DbRecordResult<()> {
    match data.len().cmp(&expected) {
        Ordering::Less => Err(DbRecordError::Truncated {
            record,
            needed: expected,
            available: data.len(),
        }),
        Ordering::Greater => Err(DbRecordError::LengthMismatch {
            record,
            expected,
            actual: data.len(),
        }),
        Ordering::Equal => Ok(()),
    }
}

fn affect_count_for_error(count: usize) -> u128 {
    u64::try_from(count).map_or(u128::MAX, u128::from)
}

fn affect_wire_count(count: usize) -> DbRecordResult<u32> {
    u32::try_from(count).map_err(|_| DbRecordError::AffectCountOverflow {
        count: affect_count_for_error(count),
    })
}

fn checked_affect_payload_size(count: usize) -> DbRecordResult<usize> {
    let elements_size = count
        .checked_mul(AffectElementRecord::WIRE_SIZE)
        .ok_or(DbRecordError::AffectPayloadSizeOverflow { count })?;
    AFFECT_LOAD_PREFIX_WIRE_SIZE
        .checked_add(elements_size)
        .ok_or(DbRecordError::AffectPayloadSizeOverflow { count })
}

fn try_allocate_affect<T>(capacity: usize, count: usize) -> DbRecordResult<Vec<T>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| DbRecordError::AffectAllocationFailed { count })?;
    Ok(values)
}

/// Common API implemented by fixed-width records in this module.
pub trait RecordCodec: Sized {
    /// Exact packed x86 wire size.
    const WIRE_SIZE: usize;

    /// Encode this record field by field.
    fn encode(&self) -> Vec<u8>;

    /// Decode one exact-size record.
    ///
    /// # Errors
    ///
    /// Returns [`DbRecordError::Truncated`] for a short input or
    /// [`DbRecordError::LengthMismatch`] when trailing bytes are present.
    fn decode(data: &[u8]) -> DbRecordResult<Self>;
}

/// `TSimplePlayer`: a character summary in `TAccountTable`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SimplePlayerRecord {
    /// `dwID`.
    pub id: u32,
    /// `szName[CHARACTER_NAME_MAX_LEN + 1]`.
    pub name: [u8; LEGACY_CHARACTER_NAME_BYTES],
    /// `byJob`.
    pub job: u8,
    /// `byLevel`.
    pub level: u8,
    /// `dwPlayMinutes`.
    pub play_minutes: u32,
    /// `byST`.
    pub st: u8,
    /// `byHT`.
    pub ht: u8,
    /// `byDX`.
    pub dx: u8,
    /// `byIQ`.
    pub iq: u8,
    /// `wMainPart`.
    pub main_part: u16,
    /// `bChangeName`.
    pub change_name: u8,
    /// `wHairPart`.
    pub hair_part: u16,
    /// `wSashPart` (the active legacy build enables the sash system).
    pub sash_part: u16,
    /// `bDummy[4]`.
    pub dummy: [u8; 4],
    /// `x` (x86 `long`).
    pub x: i32,
    /// `y` (x86 `long`).
    pub y: i32,
    /// `lAddr` (x86 `long`).
    pub addr: i32,
    /// `wPort`.
    pub port: u16,
    /// `skill_group`.
    pub skill_group: u8,
    /// `byConquerorLevel`.
    pub conqueror_level: u8,
    /// `bySungmaStr`.
    pub sungma_str: u8,
    /// `bySungmaHp`.
    pub sungma_hp: u8,
    /// `bySungmaMove`.
    pub sungma_move: u8,
    /// `bySungmaImmune`.
    pub sungma_immune: u8,
}

fn encode_simple_player(record: &SimplePlayerRecord) -> Vec<u8> {
    let mut out = Writer::with_capacity(SIMPLE_PLAYER_WIRE_SIZE);
    out.u32(record.id);
    out.bytes(&record.name);
    out.u8(record.job);
    out.u8(record.level);
    out.u32(record.play_minutes);
    out.u8(record.st);
    out.u8(record.ht);
    out.u8(record.dx);
    out.u8(record.iq);
    out.u16(record.main_part);
    out.u8(record.change_name);
    out.u16(record.hair_part);
    out.u16(record.sash_part);
    out.bytes(&record.dummy);
    out.i32(record.x);
    out.i32(record.y);
    out.i32(record.addr);
    out.u16(record.port);
    out.u8(record.skill_group);
    out.u8(record.conqueror_level);
    out.u8(record.sungma_str);
    out.u8(record.sungma_hp);
    out.u8(record.sungma_move);
    out.u8(record.sungma_immune);
    out.bytes
}

fn decode_simple_player(reader: &mut Reader<'_>) -> DbRecordResult<SimplePlayerRecord> {
    Ok(SimplePlayerRecord {
        id: reader.u32()?,
        name: reader.array()?,
        job: reader.u8()?,
        level: reader.u8()?,
        play_minutes: reader.u32()?,
        st: reader.u8()?,
        ht: reader.u8()?,
        dx: reader.u8()?,
        iq: reader.u8()?,
        main_part: reader.u16()?,
        change_name: reader.u8()?,
        hair_part: reader.u16()?,
        sash_part: reader.u16()?,
        dummy: reader.array()?,
        x: reader.i32()?,
        y: reader.i32()?,
        addr: reader.i32()?,
        port: reader.u16()?,
        skill_group: reader.u8()?,
        conqueror_level: reader.u8()?,
        sungma_str: reader.u8()?,
        sungma_hp: reader.u8()?,
        sungma_move: reader.u8()?,
        sungma_immune: reader.u8()?,
    })
}

impl SimplePlayerRecord {
    /// Exact packed x86 wire size (`sizeof(TSimplePlayer)`).
    pub const WIRE_SIZE: usize = SIMPLE_PLAYER_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        encode_simple_player(self)
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TSimplePlayer", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TSimplePlayer");
        decode_simple_player(&mut reader)
    }
}

macro_rules! impl_fixed_codec {
    ($ty:ty, $label:literal, $size:expr) => {
        impl RecordCodec for $ty {
            const WIRE_SIZE: usize = $size;

            fn encode(&self) -> Vec<u8> {
                <$ty>::encode_wire(self)
            }

            fn decode(data: &[u8]) -> DbRecordResult<Self> {
                <$ty>::decode_wire(data)
            }
        }

        impl $ty {
            /// Encode this record using the legacy packed x86 field order.
            pub fn encode(&self) -> Vec<u8> {
                <$ty>::encode_wire(self)
            }

            /// Decode exactly one legacy record.
            ///
            /// # Errors
            ///
            /// Returns [`DbRecordError::Truncated`] for a short input or
            /// [`DbRecordError::LengthMismatch`] when trailing bytes are present.
            pub fn decode(data: &[u8]) -> DbRecordResult<Self> {
                <$ty>::decode_wire(data)
            }

            /// Alias for [`RecordCodec::encode`].
            pub fn to_bytes(&self) -> Vec<u8> {
                <$ty>::encode_wire(self)
            }

            /// Alias for [`RecordCodec::decode`].
            ///
            /// # Errors
            ///
            /// Returns [`DbRecordError::Truncated`] for a short input or
            /// [`DbRecordError::LengthMismatch`] when trailing bytes are present.
            pub fn from_bytes(data: &[u8]) -> DbRecordResult<Self> {
                <$ty>::decode_wire(data)
            }

            /// Return the packed x86 wire size.
            pub const fn packed_size() -> usize {
                $size
            }
        }

        impl crate::PacketSerialize for $ty {
            fn to_bytes(&self) -> Vec<u8> {
                <$ty>::encode_wire(self)
            }

            fn from_bytes(data: &[u8]) -> io::Result<Self> {
                <$ty>::decode_wire(data).map_err(Into::into)
            }

            fn packed_size() -> usize {
                $size
            }
        }
    };
}

impl_fixed_codec!(SimplePlayerRecord, "TSimplePlayer", SIMPLE_PLAYER_WIRE_SIZE);

/// `TAccountTable`: the account result sent for a successful login.
///
/// A `LOGIN_BY_KEY` request is followed by a database lookup.  The eventual
/// `HEADER_DG_LOGIN_SUCCESS` payload is this record, not the GD request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoginAccountRecord {
    /// `id`.
    pub id: u32,
    /// `login[LOGIN_MAX_LEN + 1]`.
    pub login: [u8; LEGACY_LOGIN_BYTES],
    /// `passwd[PASSWD_MAX_LEN + 1]`.
    pub passwd: [u8; LEGACY_PASSWORD_BYTES],
    /// `social_id[SOCIAL_ID_MAX_LEN + 1]`.
    pub social_id: [u8; LEGACY_SOCIAL_ID_BYTES],
    /// `status[ACCOUNT_STATUS_MAX_LEN + 1]`.
    pub status: [u8; LEGACY_ACCOUNT_STATUS_BYTES],
    /// `bEmpire`.
    pub empire: u8,
    /// `players[PLAYER_PER_ACCOUNT]`.
    pub players: [SimplePlayerRecord; LEGACY_PLAYER_PER_ACCOUNT],
    /// `bLanguage` (the active legacy build enables multi-language support).
    pub language: u8,
}

impl LoginAccountRecord {
    /// Exact packed x86 wire size (`sizeof(TAccountTable)`).
    pub const WIRE_SIZE: usize = LOGIN_ACCOUNT_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.id);
        out.bytes(&self.login);
        out.bytes(&self.passwd);
        out.bytes(&self.social_id);
        out.bytes(&self.status);
        out.u8(self.empire);
        for player in &self.players {
            let encoded = encode_simple_player(player);
            debug_assert_eq!(encoded.len(), SimplePlayerRecord::WIRE_SIZE);
            out.bytes(&encoded);
        }
        out.u8(self.language);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TAccountTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TAccountTable");
        let id = reader.u32()?;
        let login = reader.array()?;
        let passwd = reader.array()?;
        let social_id = reader.array()?;
        let status = reader.array()?;
        let empire = reader.u8()?;
        let mut players = [SimplePlayerRecord::default(); LEGACY_PLAYER_PER_ACCOUNT];
        for player in &mut players {
            *player = decode_simple_player(&mut reader)?;
        }
        let language = reader.u8()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            id,
            login,
            passwd,
            social_id,
            status,
            empire,
            players,
            language,
        })
    }
}

impl_fixed_codec!(LoginAccountRecord, "TAccountTable", LOGIN_ACCOUNT_WIRE_SIZE);

/// `TPacketDGLoginAlready`: the account name returned when login is already active.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoginAlreadyRecord {
    /// Raw `szLogin[LOGIN_MAX_LEN + 1]` bytes, including any source tail.
    pub login: [u8; LEGACY_LOGIN_BYTES],
}

impl LoginAlreadyRecord {
    /// Exact packed x86 wire size (`sizeof(TPacketDGLoginAlready)`).
    pub const WIRE_SIZE: usize = LOGIN_ALREADY_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        self.login.to_vec()
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPacketDGLoginAlready", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPacketDGLoginAlready");
        let login = reader.array()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self { login })
    }
}

impl_fixed_codec!(
    LoginAlreadyRecord,
    "TPacketDGLoginAlready",
    LOGIN_ALREADY_WIRE_SIZE
);

/// `TPacketGDLoginByKey`: the GD request that starts the key-based login.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoginByKeyRequest {
    /// `szLogin[LOGIN_MAX_LEN + 1]`.
    pub login: [u8; LEGACY_LOGIN_BYTES],
    /// `dwLoginKey`.
    pub login_key: u32,
    /// `adwClientKey[4]`.
    pub client_key: [u32; 4],
    /// `szIP[MAX_HOST_LENGTH + 1]`.
    pub ip: [u8; LEGACY_IP_BYTES],
}

impl LoginByKeyRequest {
    /// Exact packed x86 wire size (`sizeof(TPacketGDLoginByKey)`).
    pub const WIRE_SIZE: usize = LOGIN_BY_KEY_REQUEST_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.bytes(&self.login);
        out.u32(self.login_key);
        for key in self.client_key {
            out.u32(key);
        }
        out.bytes(&self.ip);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPacketGDLoginByKey", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPacketGDLoginByKey");
        let login = reader.array()?;
        let login_key = reader.u32()?;
        let client_key = [reader.u32()?, reader.u32()?, reader.u32()?, reader.u32()?];
        let ip = reader.array()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            login,
            login_key,
            client_key,
            ip,
        })
    }
}

impl_fixed_codec!(
    LoginByKeyRequest,
    "TPacketGDLoginByKey",
    LOGIN_BY_KEY_REQUEST_WIRE_SIZE
);

/// `TPlayerLoadPacket`: the GD request for one character.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerLoadRequest {
    /// `account_id`.
    pub account_id: u32,
    /// `player_id`.
    pub player_id: u32,
    /// `account_index`.
    pub account_index: u8,
}

impl PlayerLoadRequest {
    /// Exact packed x86 wire size (`sizeof(TPlayerLoadPacket)`).
    pub const WIRE_SIZE: usize = PLAYER_LOAD_REQUEST_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.account_id);
        out.u32(self.player_id);
        out.u8(self.account_index);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPlayerLoadPacket", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPlayerLoadPacket");
        let account_id = reader.u32()?;
        let player_id = reader.u32()?;
        let account_index = reader.u8()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            account_id,
            player_id,
            account_index,
        })
    }
}

impl_fixed_codec!(
    PlayerLoadRequest,
    "TPlayerLoadPacket",
    PLAYER_LOAD_REQUEST_WIRE_SIZE
);

/// The four-byte payload of `HEADER_GD_REQ_HORSE_NAME`.
///
/// The active legacy router reads one packed `DWORD` from this request and
/// passes it to `AckHorseName`; it does not send a named C++ packet struct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HorseNameRequest {
    /// Player ID whose horse name should be looked up.
    pub player_id: u32,
}

impl HorseNameRequest {
    /// Construct a request for one player ID.
    pub const fn new(player_id: u32) -> Self {
        Self { player_id }
    }

    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = HORSE_NAME_REQUEST_WIRE_SIZE;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.player_id);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("horse-name request", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "horse-name request");
        Ok(Self {
            player_id: reader.u32()?,
        })
    }
}

impl_fixed_codec!(
    HorseNameRequest,
    "horse-name request",
    HORSE_NAME_REQUEST_WIRE_SIZE
);

/// The packed `TPacketUpdateHorseName` horse-name acknowledgement record.
///
/// The legacy DB response uses this 29-byte `DWORD` plus `char[25]` layout.
/// Name bytes are retained exactly; callers that load a C string should
/// provide a deterministic 25-byte array rather than relying on the legacy
/// stack's uninitialized tail bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HorseNameRecord {
    /// Player ID copied from the request.
    pub player_id: u32,
    /// Raw `CHARACTER_NAME_MAX_LEN + 1` name bytes.
    pub name: [u8; LEGACY_CHARACTER_NAME_BYTES],
}

impl HorseNameRecord {
    /// Construct a response record from an exact raw name buffer.
    pub const fn new(player_id: u32, name: [u8; LEGACY_CHARACTER_NAME_BYTES]) -> Self {
        Self { player_id, name }
    }

    /// Construct a missing-row response with all name bytes zeroed.
    pub const fn missing(player_id: u32) -> Self {
        Self {
            player_id,
            name: [0; LEGACY_CHARACTER_NAME_BYTES],
        }
    }

    /// Exact packed x86 wire size (`sizeof(TPacketUpdateHorseName)`).
    pub const WIRE_SIZE: usize = HORSE_NAME_RECORD_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.player_id);
        out.bytes(&self.name);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPacketUpdateHorseName", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPacketUpdateHorseName");
        Ok(Self {
            player_id: reader.u32()?,
            name: reader.array()?,
        })
    }
}

impl_fixed_codec!(
    HorseNameRecord,
    "TPacketUpdateHorseName",
    HORSE_NAME_RECORD_WIRE_SIZE
);

/// `TPacketChangeChannel`: the request used to find an alternate game channel.
///
/// The active x86 build packs an x86 `long` map index followed by a signed
/// 32-bit channel number. The legacy handler treats either field being exactly
/// zero as an instruction to return without sending a response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelChangeRequest {
    /// `lMapIndex` (signed x86 `long`).
    pub map_index: i32,
    /// `iChannel`.
    pub channel: i32,
}

impl ChannelChangeRequest {
    /// Construct a channel-change request.
    pub const fn new(map_index: i32, channel: i32) -> Self {
        Self { map_index, channel }
    }

    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = CHANNEL_CHANGE_REQUEST_WIRE_SIZE;

    /// Return whether the legacy handler would reject this request without a
    /// response because its map index or channel is zero.
    pub const fn is_valid(&self) -> bool {
        self.map_index != 0 && self.channel != 0
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.i32(self.map_index);
        out.i32(self.channel);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPacketChangeChannel", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPacketChangeChannel");
        let request = Self {
            map_index: reader.i32()?,
            channel: reader.i32()?,
        };
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(request)
    }
}

impl_fixed_codec!(
    ChannelChangeRequest,
    "TPacketChangeChannel",
    CHANNEL_CHANGE_REQUEST_WIRE_SIZE
);

/// `TPacketReturnChannel`: the result returned for a channel lookup.
///
/// `lAddr` is represented as signed `i32` because the active legacy C++ struct
/// uses an x86 `long`. A resolver should provide the integer result of
/// `inet_addr` on that target; the codec writes the raw four bytes explicitly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelResultRecord {
    /// `lAddr` (signed x86 `long` containing the legacy `inet_addr` result).
    pub address: i32,
    /// `wPort` (unsigned 16-bit listen port).
    pub port: u16,
}

impl ChannelResultRecord {
    /// Construct a channel endpoint result.
    pub const fn new(address: i32, port: u16) -> Self {
        Self { address, port }
    }

    /// Construct the all-zero result used when a valid lookup has no match.
    pub const fn missing() -> Self {
        Self {
            address: 0,
            port: 0,
        }
    }

    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = CHANNEL_RESULT_WIRE_SIZE;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.i32(self.address);
        out.u16(self.port);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPacketReturnChannel", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPacketReturnChannel");
        let record = Self {
            address: reader.i32()?,
            port: reader.u16()?,
        };
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(record)
    }
}

impl_fixed_codec!(
    ChannelResultRecord,
    "TPacketReturnChannel",
    CHANNEL_RESULT_WIRE_SIZE
);

/// `TPacketAffectElement`: the packed affect element carried by an add request.
///
/// The active x86 record contains one `DWORD`, one `BYTE`, and four signed or
/// unsigned 32-bit fields. It is encoded field by field in source order; its
/// Rust layout is not used as the wire representation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AffectElementRecord {
    /// `dwType`.
    pub affect_type: u32,
    /// `bApplyOn`.
    pub apply_on: u8,
    /// `lApplyValue` (x86 `long`).
    pub apply_value: i32,
    /// `dwFlag`.
    pub flag: u32,
    /// `lDuration` (x86 `long`).
    pub duration: i32,
    /// `lSPCost` (x86 `long`).
    pub sp_cost: i32,
}

impl AffectElementRecord {
    /// Construct an affect element.
    #[must_use]
    pub const fn new(
        affect_type: u32,
        apply_on: u8,
        apply_value: i32,
        flag: u32,
        duration: i32,
        sp_cost: i32,
    ) -> Self {
        Self {
            affect_type,
            apply_on,
            apply_value,
            flag,
            duration,
            sp_cost,
        }
    }

    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = AFFECT_ELEMENT_WIRE_SIZE;

    fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.affect_type.to_le_bytes());
        out.push(self.apply_on);
        out.extend_from_slice(&self.apply_value.to_le_bytes());
        out.extend_from_slice(&self.flag.to_le_bytes());
        out.extend_from_slice(&self.duration.to_le_bytes());
        out.extend_from_slice(&self.sp_cost.to_le_bytes());
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        self.encode_into(&mut out.bytes);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPacketAffectElement", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPacketAffectElement");
        let record = Self {
            affect_type: reader.u32()?,
            apply_on: reader.u8()?,
            apply_value: reader.i32()?,
            flag: reader.u32()?,
            duration: reader.i32()?,
            sp_cost: reader.i32()?,
        };
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(record)
    }
}

impl_fixed_codec!(
    AffectElementRecord,
    "TPacketAffectElement",
    AFFECT_ELEMENT_WIRE_SIZE
);

/// Payload returned for `HEADER_DG_AFFECT_LOAD`.
///
/// The variable-size payload is a little-endian player ID, a little-endian row
/// count, and exactly `count` packed [`AffectElementRecord`] values. The player
/// ID is preserved exactly as supplied, including for an empty element list.
/// This type is only a payload codec; it does not load, schedule, route, or
/// apply affects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectLoadResponse {
    /// Player ID stored in the response prefix.
    pub player_id: u32,
    /// Affect elements in legacy row order.
    pub elements: Vec<AffectElementRecord>,
}

impl AffectLoadResponse {
    /// Bytes in the player-ID and row-count prefix.
    pub const PREFIX_WIRE_SIZE: usize = AFFECT_LOAD_PREFIX_WIRE_SIZE;

    /// Construct an affect-load response without changing or validating rows.
    #[must_use]
    pub fn new(player_id: u32, elements: Vec<AffectElementRecord>) -> Self {
        Self {
            player_id,
            elements,
        }
    }

    /// Calculate the exact payload size with checked arithmetic.
    ///
    /// # Errors
    ///
    /// Returns [`DbRecordError::AffectCountOverflow`] when the row count cannot
    /// be represented by the wire `u32`, or
    /// [`DbRecordError::AffectPayloadSizeOverflow`] when the byte size cannot
    /// be represented by `usize`.
    pub fn encoded_len(&self) -> DbRecordResult<usize> {
        affect_wire_count(self.elements.len())?;
        checked_affect_payload_size(self.elements.len())
    }

    /// Encode this response without an invented source row-count cap.
    ///
    /// # Errors
    ///
    /// Returns a distinct count-conversion, size-overflow, or allocation error.
    pub fn encode(&self) -> DbRecordResult<Vec<u8>> {
        self.encode_with_limit(usize::MAX)
    }

    /// Encode this response with an explicit caller-selected row limit.
    ///
    /// The limit is checked before count conversion, sizing, or allocation.
    ///
    /// # Errors
    ///
    /// Returns [`DbRecordError::TooManyAffectRecords`] when the input exceeds
    /// `maximum`, or a distinct count, size, or allocation error.
    pub fn encode_with_limit(&self, maximum: usize) -> DbRecordResult<Vec<u8>> {
        let count = self.elements.len();
        if count > maximum {
            return Err(DbRecordError::TooManyAffectRecords {
                count: u32::try_from(count).unwrap_or(u32::MAX),
                maximum,
            });
        }
        let wire_count = affect_wire_count(count)?;
        let payload_size = checked_affect_payload_size(count)?;
        let mut out = try_allocate_affect::<u8>(payload_size, count)?;
        out.extend_from_slice(&self.player_id.to_le_bytes());
        out.extend_from_slice(&wire_count.to_le_bytes());
        for element in &self.elements {
            element.encode_into(&mut out);
        }
        debug_assert_eq!(out.len(), payload_size);
        Ok(out)
    }

    /// Decode this response without an invented source row-count cap.
    ///
    /// The decoder computes and checks the exact `8 + 21 * count` length before
    /// reserving or allocating the element vector.
    ///
    /// # Errors
    ///
    /// Returns [`DbRecordError::Truncated`], [`DbRecordError::LengthMismatch`],
    /// or a distinct count-conversion, size-overflow, or allocation error.
    pub fn decode(data: &[u8]) -> DbRecordResult<Self> {
        Self::decode_with_limit(data, usize::MAX)
    }

    /// Decode this response with an explicit caller-selected row limit.
    ///
    /// The limit, count conversion, checked byte size, and exact input length
    /// are all validated before storage is reserved.
    ///
    /// # Errors
    ///
    /// Returns [`DbRecordError::TooManyAffectRecords`] when the declared count
    /// exceeds `maximum`, [`DbRecordError::Truncated`] or
    /// [`DbRecordError::LengthMismatch`] for an invalid exact length, or a
    /// distinct count-conversion, size-overflow, or allocation error.
    pub fn decode_with_limit(data: &[u8], maximum: usize) -> DbRecordResult<Self> {
        const RECORD: &str = "HEADER_DG_AFFECT_LOAD";

        if data.len() < Self::PREFIX_WIRE_SIZE {
            return Err(DbRecordError::Truncated {
                record: RECORD,
                needed: Self::PREFIX_WIRE_SIZE,
                available: data.len(),
            });
        }
        let player_id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let wire_count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let count =
            usize::try_from(wire_count).map_err(|_| DbRecordError::AffectCountOverflow {
                count: u128::from(wire_count),
            })?;
        if count > maximum {
            return Err(DbRecordError::TooManyAffectRecords {
                count: wire_count,
                maximum,
            });
        }
        let expected = checked_affect_payload_size(count)?;
        match data.len().cmp(&expected) {
            Ordering::Less => {
                return Err(DbRecordError::Truncated {
                    record: RECORD,
                    needed: expected,
                    available: data.len(),
                });
            }
            Ordering::Greater => {
                return Err(DbRecordError::LengthMismatch {
                    record: RECORD,
                    expected,
                    actual: data.len(),
                });
            }
            Ordering::Equal => {}
        }

        let mut elements = try_allocate_affect::<AffectElementRecord>(count, count)?;
        let element_bytes = &data[Self::PREFIX_WIRE_SIZE..];
        for encoded_element in element_bytes.chunks_exact(AffectElementRecord::WIRE_SIZE) {
            elements.push(AffectElementRecord::decode(encoded_element)?);
        }
        debug_assert_eq!(elements.len(), count);
        Ok(Self {
            player_id,
            elements,
        })
    }

    /// Alias for [`AffectLoadResponse::encode`].
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`AffectLoadResponse::encode`].
    pub fn to_bytes(&self) -> DbRecordResult<Vec<u8>> {
        self.encode()
    }

    /// Alias for [`AffectLoadResponse::decode`].
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`AffectLoadResponse::decode`].
    pub fn from_bytes(data: &[u8]) -> DbRecordResult<Self> {
        Self::decode(data)
    }
}

/// `TPacketGDAddAffect`: the request that adds one player affect.
///
/// The packed active x86 payload is `DWORD dwPID` followed by
/// `TPacketAffectElement elem`, for 25 bytes total. The surrounding DB peer
/// frame carries the header, handle, and length; this codec covers only the
/// payload. The legacy DB handler performs asynchronous persistence and sends
/// no response, so this record deliberately has no SQL or response adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AddAffectRequest {
    /// `dwPID`, the affected player ID.
    pub pid: u32,
    /// `elem`, the packed affect element.
    pub elem: AffectElementRecord,
}

impl AddAffectRequest {
    /// Construct an add-affect request.
    #[must_use]
    pub const fn new(pid: u32, elem: AffectElementRecord) -> Self {
        Self { pid, elem }
    }

    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = ADD_AFFECT_REQUEST_WIRE_SIZE;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.pid);
        let elem = self.elem.encode();
        out.bytes(&elem);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPacketGDAddAffect", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPacketGDAddAffect");
        let pid = reader.u32()?;
        let elem = AffectElementRecord::decode(&data[4..])?;
        debug_assert_eq!(reader.offset, 4);
        Ok(Self { pid, elem })
    }
}

impl_fixed_codec!(
    AddAffectRequest,
    "TPacketGDAddAffect",
    ADD_AFFECT_REQUEST_WIRE_SIZE
);

/// `TPacketGDRemoveAffect`: the request that removes one player affect.
///
/// The packed active x86 payload is `DWORD dwPID`, `DWORD dwType`, and
/// `BYTE bApplyOn`. The header is carried by the surrounding DB peer frame;
/// this codec covers the nine-byte payload only. The legacy DB router issues
/// an asynchronous DELETE and sends no response, so this record deliberately
/// has no response or SQL adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RemoveAffectRequest {
    /// `dwPID`, the affected player ID.
    pub player_id: u32,
    /// `dwType`, the affect type.
    pub affect_type: u32,
    /// `bApplyOn`, the source application selector.
    pub apply_on: u8,
}

impl RemoveAffectRequest {
    /// Construct a remove-affect request.
    pub const fn new(player_id: u32, affect_type: u32, apply_on: u8) -> Self {
        Self {
            player_id,
            affect_type,
            apply_on,
        }
    }

    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = REMOVE_AFFECT_REQUEST_WIRE_SIZE;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.player_id);
        out.u32(self.affect_type);
        out.u8(self.apply_on);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPacketGDRemoveAffect", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPacketGDRemoveAffect");
        let request = Self {
            player_id: reader.u32()?,
            affect_type: reader.u32()?,
            apply_on: reader.u8()?,
        };
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(request)
    }
}

impl_fixed_codec!(
    RemoveAffectRequest,
    "TPacketGDRemoveAffect",
    REMOVE_AFFECT_REQUEST_WIRE_SIZE
);

/// `TPlayerSkill`: one packed x86 skill record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerSkillRecord {
    /// `bMasterType`.
    pub master_type: u8,
    /// `bLevel`.
    pub level: u8,
    /// `tNextRead` (x86 `time_t` is four bytes).
    pub next_read: i32,
}

impl PlayerSkillRecord {
    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = 6;

    const EMPTY: Self = Self {
        master_type: 0,
        level: 0,
        next_read: 0,
    };

    // Keep the common codec hook reference-based for all records.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u8(self.master_type);
        out.u8(self.level);
        out.i32(self.next_read);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPlayerSkill", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPlayerSkill");
        Ok(Self {
            master_type: reader.u8()?,
            level: reader.u8()?,
            next_read: reader.i32()?,
        })
    }
}

impl_fixed_codec!(PlayerSkillRecord, "TPlayerSkill", 6);

/// `TQuickslot`: one packed quickslot record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuickslotRecord {
    /// `type`.
    pub slot_type: u8,
    /// `pos`.
    pub pos: u8,
}

impl QuickslotRecord {
    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = 2;

    const EMPTY: Self = Self {
        slot_type: 0,
        pos: 0,
    };

    // Keep the common codec hook reference-based for all records.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        vec![self.slot_type, self.pos]
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TQuickslot", data, Self::WIRE_SIZE)?;
        Ok(Self {
            slot_type: data[0],
            pos: data[1],
        })
    }
}

impl_fixed_codec!(QuickslotRecord, "TQuickslot", 2);

/// `TPlayerFishEventSlot`: one packed fish-event slot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FishEventSlotRecord {
    /// Raw one-byte C++ `bool` representation.
    pub is_main: u8,
    /// `bShape`.
    pub shape: u8,
}

impl FishEventSlotRecord {
    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = 2;

    const EMPTY: Self = Self {
        is_main: 0,
        shape: 0,
    };

    // Keep the common codec hook reference-based for all records.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        vec![self.is_main, self.shape]
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPlayerFishEventSlot", data, Self::WIRE_SIZE)?;
        Ok(Self {
            is_main: data[0],
            shape: data[1],
        })
    }
}

impl_fixed_codec!(FishEventSlotRecord, "TPlayerFishEventSlot", 2);

/// `THorseInfo`: one packed horse record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HorseInfoRecord {
    /// `bLevel`.
    pub level: u8,
    /// `bRiding`.
    pub riding: u8,
    /// `sStamina`.
    pub stamina: i16,
    /// `sHealth`.
    pub health: i16,
    /// `dwHorseHealthDropTime`.
    pub health_drop_time: u32,
}

impl HorseInfoRecord {
    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = 10;

    const EMPTY: Self = Self {
        level: 0,
        riding: 0,
        stamina: 0,
        health: 0,
        health_drop_time: 0,
    };

    // Keep the common codec hook reference-based for all records.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u8(self.level);
        out.u8(self.riding);
        out.i16(self.stamina);
        out.i16(self.health);
        out.u32(self.health_drop_time);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("THorseInfo", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "THorseInfo");
        Ok(Self {
            level: reader.u8()?,
            riding: reader.u8()?,
            stamina: reader.i16()?,
            health: reader.i16()?,
            health_drop_time: reader.u32()?,
        })
    }
}

impl_fixed_codec!(HorseInfoRecord, "THorseInfo", 10);

/// `TPlayerBattlePass`: one packed battle-pass record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BattlePassRecord {
    /// `missionID`.
    pub mission_id: u16,
    /// `progress`.
    pub progress: u32,
    /// `type`.
    pub battle_pass_type: u8,
    /// `endTime`.
    pub end_time: u32,
}

impl BattlePassRecord {
    /// Exact packed x86 wire size.
    pub const WIRE_SIZE: usize = 11;

    const EMPTY: Self = Self {
        mission_id: 0,
        progress: 0,
        battle_pass_type: 0,
        end_time: 0,
    };

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u16(self.mission_id);
        out.u32(self.progress);
        out.u8(self.battle_pass_type);
        out.u32(self.end_time);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPlayerBattlePass", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPlayerBattlePass");
        Ok(Self {
            mission_id: reader.u16()?,
            progress: reader.u32()?,
            battle_pass_type: reader.u8()?,
            end_time: reader.u32()?,
        })
    }
}

impl_fixed_codec!(BattlePassRecord, "TPlayerBattlePass", 11);

/// `TPlayerTable`: the `PLAYER_LOAD_SUCCESS` result.
///
/// The active production feature set is encoded here, including the x86
/// `long`/`time_t` widths, the ten battle-pass records, and the private-shop
/// unlocked-slot word.  No host-language struct layout is assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerResultRecord {
    /// `id`.
    pub id: u32,
    /// `name[CHARACTER_NAME_MAX_LEN + 1]`.
    pub name: [u8; LEGACY_CHARACTER_NAME_BYTES],
    /// `ip[IP_ADDRESS_LENGTH + 1]`.
    pub ip: [u8; LEGACY_IP_BYTES],
    /// `job`.
    pub job: u16,
    /// `voice`.
    pub voice: u8,
    /// `level`.
    pub level: u8,
    /// `level_step`.
    pub level_step: u8,
    /// `st`.
    pub st: i16,
    /// `ht`.
    pub ht: i16,
    /// `dx`.
    pub dx: i16,
    /// `iq`.
    pub iq: i16,
    /// `exp`.
    pub exp: u32,
    /// `gold` (`unsigned long long` is enabled in the active build).
    pub gold: u64,
    /// `gaya` (`ENABLE_GAYA_SYSTEM`).
    pub gaya: i32,
    /// `dir`.
    pub dir: u8,
    /// `x`.
    pub x: i32,
    /// `y`.
    pub y: i32,
    /// `z`.
    pub z: i32,
    /// `lMapIndex`.
    pub map_index: i32,
    /// `lExitX`.
    pub exit_x: i32,
    /// `lExitY`.
    pub exit_y: i32,
    /// `lExitMapIndex`.
    pub exit_map_index: i32,
    /// `hp`.
    pub hp: i32,
    /// `sp`.
    pub sp: i32,
    /// `sRandomHP`.
    pub random_hp: i16,
    /// `sRandomSP`.
    pub random_sp: i16,
    /// `playtime`.
    pub playtime: i32,
    /// `stat_point`.
    pub stat_point: i16,
    /// `skill_point`.
    pub skill_point: i16,
    /// `sub_skill_point`.
    pub sub_skill_point: i16,
    /// `horse_skill_point`.
    pub horse_skill_point: i16,
    /// `skills[SKILL_MAX_NUM]`.
    pub skills: [PlayerSkillRecord; LEGACY_SKILL_MAX_NUM],
    /// `quickslot[QUICKSLOT_MAX_NUM]`.
    pub quickslot: [QuickslotRecord; LEGACY_QUICKSLOT_MAX_NUM],
    /// `part_base`.
    pub part_base: u8,
    /// `parts[PART_MAX_NUM]`.
    pub parts: [u16; LEGACY_PART_MAX_NUM],
    /// `stamina`.
    pub stamina: i16,
    /// `skill_group`.
    pub skill_group: u8,
    /// `lAlignment` (x86 `long`).
    pub alignment: i32,
    /// `stat_reset_count`.
    pub stat_reset_count: i16,
    /// `horse`.
    pub horse: HorseInfoRecord,
    /// `logoff_interval`.
    pub logoff_interval: u32,
    /// `aiPremiumTimes[PREMIUM_MAX_NUM]`.
    pub premium_times: [i32; LEGACY_PREMIUM_MAX_NUM],
    /// `envanter` (`ENABLE_EXTEND_INVEN_SYSTEM`).
    pub envanter: i32,
    /// `fishEventUseCount`.
    pub fish_event_use_count: u32,
    /// `fishSlots[FISH_EVENT_SLOTS_NUM]`.
    pub fish_slots: [FishEventSlotRecord; LEGACY_FISH_EVENT_SLOTS_NUM],
    /// `premium`.
    pub premium: u8,
    /// `premium_time` (x86 `long`).
    pub premium_time: i32,
    /// `secured`.
    pub secured: u8,
    /// `secured_password`.
    pub secured_password: i32,
    /// `biologist_state`.
    pub biologist_state: u32,
    /// `biologist_items_taken`.
    pub biologist_items_taken: u32,
    /// `biologist_completed`.
    pub biologist_completed: u32,
    /// `conqueror_level`.
    pub conqueror_level: u8,
    /// `conqueror_level_step`.
    pub conqueror_level_step: u8,
    /// `sungma_str`.
    pub sungma_str: i16,
    /// `sungma_hp`.
    pub sungma_hp: i16,
    /// `sungma_move`.
    pub sungma_move: i16,
    /// `sungma_immune`.
    pub sungma_immune: i16,
    /// `conqueror_exp`.
    pub conqueror_exp: u32,
    /// `conqueror_point`.
    pub conqueror_point: i16,
    /// `battlePass[BATTLEPASS_MISSIONS_PER_PLAYER]`.
    pub battle_pass: [BattlePassRecord; LEGACY_BATTLEPASS_MISSIONS_PER_PLAYER],
    /// `wPrivateShopUnlockedSlot`.
    pub private_shop_unlocked_slot: u16,
}

impl Default for PlayerResultRecord {
    fn default() -> Self {
        Self {
            id: 0,
            name: [0; LEGACY_CHARACTER_NAME_BYTES],
            ip: [0; LEGACY_IP_BYTES],
            job: 0,
            voice: 0,
            level: 0,
            level_step: 0,
            st: 0,
            ht: 0,
            dx: 0,
            iq: 0,
            exp: 0,
            gold: 0,
            gaya: 0,
            dir: 0,
            x: 0,
            y: 0,
            z: 0,
            map_index: 0,
            exit_x: 0,
            exit_y: 0,
            exit_map_index: 0,
            hp: 0,
            sp: 0,
            random_hp: 0,
            random_sp: 0,
            playtime: 0,
            stat_point: 0,
            skill_point: 0,
            sub_skill_point: 0,
            horse_skill_point: 0,
            skills: [PlayerSkillRecord::EMPTY; LEGACY_SKILL_MAX_NUM],
            quickslot: [QuickslotRecord::EMPTY; LEGACY_QUICKSLOT_MAX_NUM],
            part_base: 0,
            parts: [0; LEGACY_PART_MAX_NUM],
            stamina: 0,
            skill_group: 0,
            alignment: 0,
            stat_reset_count: 0,
            horse: HorseInfoRecord::EMPTY,
            logoff_interval: 0,
            premium_times: [0; LEGACY_PREMIUM_MAX_NUM],
            envanter: 0,
            fish_event_use_count: 0,
            fish_slots: [FishEventSlotRecord::EMPTY; LEGACY_FISH_EVENT_SLOTS_NUM],
            premium: 0,
            premium_time: 0,
            secured: 0,
            secured_password: 0,
            biologist_state: 0,
            biologist_items_taken: 0,
            biologist_completed: 0,
            conqueror_level: 0,
            conqueror_level_step: 0,
            sungma_str: 0,
            sungma_hp: 0,
            sungma_move: 0,
            sungma_immune: 0,
            conqueror_exp: 0,
            conqueror_point: 0,
            battle_pass: [BattlePassRecord::EMPTY; LEGACY_BATTLEPASS_MISSIONS_PER_PLAYER],
            private_shop_unlocked_slot: 0,
        }
    }
}

impl PlayerResultRecord {
    /// Exact packed x86 wire size (`sizeof(TPlayerTable)`).
    pub const WIRE_SIZE: usize = PLAYER_RESULT_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.id);
        out.bytes(&self.name);
        out.bytes(&self.ip);
        out.u16(self.job);
        out.u8(self.voice);
        out.u8(self.level);
        out.u8(self.level_step);
        out.i16(self.st);
        out.i16(self.ht);
        out.i16(self.dx);
        out.i16(self.iq);
        out.u32(self.exp);
        out.u64(self.gold);
        out.i32(self.gaya);
        out.u8(self.dir);
        out.i32(self.x);
        out.i32(self.y);
        out.i32(self.z);
        out.i32(self.map_index);
        out.i32(self.exit_x);
        out.i32(self.exit_y);
        out.i32(self.exit_map_index);
        out.i32(self.hp);
        out.i32(self.sp);
        out.i16(self.random_hp);
        out.i16(self.random_sp);
        out.i32(self.playtime);
        out.i16(self.stat_point);
        out.i16(self.skill_point);
        out.i16(self.sub_skill_point);
        out.i16(self.horse_skill_point);
        for skill in &self.skills {
            out.u8(skill.master_type);
            out.u8(skill.level);
            out.i32(skill.next_read);
        }
        for slot in &self.quickslot {
            out.u8(slot.slot_type);
            out.u8(slot.pos);
        }
        out.u8(self.part_base);
        for part in self.parts {
            out.u16(part);
        }
        out.i16(self.stamina);
        out.u8(self.skill_group);
        out.i32(self.alignment);
        out.i16(self.stat_reset_count);
        out.u8(self.horse.level);
        out.u8(self.horse.riding);
        out.i16(self.horse.stamina);
        out.i16(self.horse.health);
        out.u32(self.horse.health_drop_time);
        out.u32(self.logoff_interval);
        for premium in self.premium_times {
            out.i32(premium);
        }
        out.i32(self.envanter);
        out.u32(self.fish_event_use_count);
        for slot in &self.fish_slots {
            out.u8(slot.is_main);
            out.u8(slot.shape);
        }
        out.u8(self.premium);
        out.i32(self.premium_time);
        out.u8(self.secured);
        out.i32(self.secured_password);
        out.u32(self.biologist_state);
        out.u32(self.biologist_items_taken);
        out.u32(self.biologist_completed);
        out.u8(self.conqueror_level);
        out.u8(self.conqueror_level_step);
        out.i16(self.sungma_str);
        out.i16(self.sungma_hp);
        out.i16(self.sungma_move);
        out.i16(self.sungma_immune);
        out.u32(self.conqueror_exp);
        out.i16(self.conqueror_point);
        for mission in &self.battle_pass {
            out.u16(mission.mission_id);
            out.u32(mission.progress);
            out.u8(mission.battle_pass_type);
            out.u32(mission.end_time);
        }
        out.u16(self.private_shop_unlocked_slot);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    // This decoder intentionally mirrors the packed legacy field order.
    #[allow(clippy::too_many_lines)]
    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPlayerTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPlayerTable");
        let id = reader.u32()?;
        let name = reader.array()?;
        let ip = reader.array()?;
        let job = reader.u16()?;
        let voice = reader.u8()?;
        let level = reader.u8()?;
        let level_step = reader.u8()?;
        let st = reader.i16()?;
        let ht = reader.i16()?;
        let dx = reader.i16()?;
        let iq = reader.i16()?;
        let exp = reader.u32()?;
        let gold = reader.u64()?;
        let gaya = reader.i32()?;
        let dir = reader.u8()?;
        let x = reader.i32()?;
        let y = reader.i32()?;
        let z = reader.i32()?;
        let map_index = reader.i32()?;
        let exit_x = reader.i32()?;
        let exit_y = reader.i32()?;
        let exit_map_index = reader.i32()?;
        let hp = reader.i32()?;
        let sp = reader.i32()?;
        let hp_variance = reader.i16()?;
        let sp_variance = reader.i16()?;
        let playtime = reader.i32()?;
        let stat_point = reader.i16()?;
        let skill_point = reader.i16()?;
        let sub_skill_point = reader.i16()?;
        let horse_skill_point = reader.i16()?;

        let mut skills = [PlayerSkillRecord::default(); LEGACY_SKILL_MAX_NUM];
        for skill in &mut skills {
            *skill = PlayerSkillRecord {
                master_type: reader.u8()?,
                level: reader.u8()?,
                next_read: reader.i32()?,
            };
        }
        let mut quickslot = [QuickslotRecord::default(); LEGACY_QUICKSLOT_MAX_NUM];
        for slot in &mut quickslot {
            *slot = QuickslotRecord {
                slot_type: reader.u8()?,
                pos: reader.u8()?,
            };
        }
        let part_base = reader.u8()?;
        let mut parts = [0_u16; LEGACY_PART_MAX_NUM];
        for part in &mut parts {
            *part = reader.u16()?;
        }
        let stamina = reader.i16()?;
        let skill_group = reader.u8()?;
        let alignment = reader.i32()?;
        let stat_reset_count = reader.i16()?;
        let horse = HorseInfoRecord {
            level: reader.u8()?,
            riding: reader.u8()?,
            stamina: reader.i16()?,
            health: reader.i16()?,
            health_drop_time: reader.u32()?,
        };
        let logoff_interval = reader.u32()?;
        let mut premium_times = [0_i32; LEGACY_PREMIUM_MAX_NUM];
        for premium in &mut premium_times {
            *premium = reader.i32()?;
        }
        let envanter = reader.i32()?;
        let fish_event_use_count = reader.u32()?;
        let mut fish_slots = [FishEventSlotRecord::default(); LEGACY_FISH_EVENT_SLOTS_NUM];
        for slot in &mut fish_slots {
            *slot = FishEventSlotRecord {
                is_main: reader.u8()?,
                shape: reader.u8()?,
            };
        }
        let premium = reader.u8()?;
        let premium_time = reader.i32()?;
        let secured = reader.u8()?;
        let secured_password = reader.i32()?;
        let biologist_state = reader.u32()?;
        let biologist_items_taken = reader.u32()?;
        let biologist_completed = reader.u32()?;
        let conqueror_level = reader.u8()?;
        let conqueror_level_step = reader.u8()?;
        let sungma_str = reader.i16()?;
        let sungma_hp = reader.i16()?;
        let sungma_move = reader.i16()?;
        let sungma_immune = reader.i16()?;
        let conqueror_exp = reader.u32()?;
        let conqueror_point = reader.i16()?;
        let mut battle_pass = [BattlePassRecord::default(); LEGACY_BATTLEPASS_MISSIONS_PER_PLAYER];
        for mission in &mut battle_pass {
            *mission = BattlePassRecord {
                mission_id: reader.u16()?,
                progress: reader.u32()?,
                battle_pass_type: reader.u8()?,
                end_time: reader.u32()?,
            };
        }
        let private_shop_unlocked_slot = reader.u16()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            id,
            name,
            ip,
            job,
            voice,
            level,
            level_step,
            st,
            ht,
            dx,
            iq,
            exp,
            gold,
            gaya,
            dir,
            x,
            y,
            z,
            map_index,
            exit_x,
            exit_y,
            exit_map_index,
            hp,
            sp,
            random_hp: hp_variance,
            random_sp: sp_variance,
            playtime,
            stat_point,
            skill_point,
            sub_skill_point,
            horse_skill_point,
            skills,
            quickslot,
            part_base,
            parts,
            stamina,
            skill_group,
            alignment,
            stat_reset_count,
            horse,
            logoff_interval,
            premium_times,
            envanter,
            fish_event_use_count,
            fish_slots,
            premium,
            premium_time,
            secured,
            secured_password,
            biologist_state,
            biologist_items_taken,
            biologist_completed,
            conqueror_level,
            conqueror_level_step,
            sungma_str,
            sungma_hp,
            sungma_move,
            sungma_immune,
            conqueror_exp,
            conqueror_point,
            battle_pass,
            private_shop_unlocked_slot,
        })
    }
}

impl_fixed_codec!(PlayerResultRecord, "TPlayerTable", PLAYER_RESULT_WIRE_SIZE);

/// `TQuestTable`: one packed quest state record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestRecord {
    /// `dwPID`.
    pub pid: u32,
    /// `szName[QUEST_NAME_MAX_LEN + 1]`.
    pub name: [u8; LEGACY_QUEST_NAME_BYTES],
    /// `szState[QUEST_STATE_MAX_LEN + 1]`.
    pub state: [u8; LEGACY_QUEST_STATE_BYTES],
    /// `lValue` (x86 `long`).
    pub value: i32,
}

impl Default for QuestRecord {
    fn default() -> Self {
        Self {
            pid: 0,
            name: [0; LEGACY_QUEST_NAME_BYTES],
            state: [0; LEGACY_QUEST_STATE_BYTES],
            value: 0,
        }
    }
}

impl QuestRecord {
    /// Exact packed x86 wire size (`sizeof(TQuestTable)`).
    pub const WIRE_SIZE: usize = QUEST_RECORD_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.pid);
        out.bytes(&self.name);
        out.bytes(&self.state);
        out.i32(self.value);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TQuestTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TQuestTable");
        Ok(Self {
            pid: reader.u32()?,
            name: reader.array()?,
            state: reader.array()?,
            value: reader.i32()?,
        })
    }
}

impl_fixed_codec!(QuestRecord, "TQuestTable", QUEST_RECORD_WIRE_SIZE);

/// `building::TLand`: one land record in the active x86 boot stream.
///
/// The fixed x86 wire record is 36 bytes. Three bytes between `guild_level_limit`
/// and `price` are padding and are always written as zero. They are skipped
/// while decoding because they are not part of the record's logical value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LandRecord {
    /// Land identifier (`dwID`).
    pub id: u32,
    /// Map identifier (`lMapIndex`, x86 `long`).
    pub map_index: i32,
    /// X coordinate (`x`, x86 `long`).
    pub x: i32,
    /// Y coordinate (`y`, x86 `long`).
    pub y: i32,
    /// Land width in map units (`width`, x86 `long`).
    pub width: i32,
    /// Land height in map units (`height`, x86 `long`).
    pub height: i32,
    /// Owning guild identifier, or zero for unowned land (`dwGuildID`).
    pub guild_id: u32,
    /// Minimum guild level allowed to own the land (`bGuildLevelLimit`).
    pub guild_level_limit: u8,
    /// Land price (`dwPrice`).
    pub price: u32,
}

impl LandRecord {
    /// Exact fixed x86 wire size (`sizeof(building::TLand)`).
    pub const WIRE_SIZE: usize = LAND_RECORD_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.id);
        out.i32(self.map_index);
        out.i32(self.x);
        out.i32(self.y);
        out.i32(self.width);
        out.i32(self.height);
        out.u32(self.guild_id);
        out.u8(self.guild_level_limit);
        out.bytes(&[0; 3]);
        out.u32(self.price);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("building::TLand", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "building::TLand");
        let id = reader.u32()?;
        let map_index = reader.i32()?;
        let x = reader.i32()?;
        let y = reader.i32()?;
        let width = reader.i32()?;
        let height = reader.i32()?;
        let guild_id = reader.u32()?;
        let guild_level_limit = reader.u8()?;
        let _padding = reader.array::<3>()?;
        let price = reader.u32()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            id,
            map_index,
            x,
            y,
            width,
            height,
            guild_id,
            guild_level_limit,
            price,
        })
    }
}

impl_fixed_codec!(LandRecord, "building::TLand", LAND_RECORD_WIRE_SIZE);

/// `building::TObjectMaterial`: one fixed material slot in an object prototype.
///
/// The active x86 record contains two unsigned 32-bit values. Its Rust field
/// layout is not used as a wire representation; [`RecordCodec`] writes both
/// values explicitly in little-endian order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObjectMaterial {
    /// Required item virtual number (`dwItemVnum`).
    pub item_vnum: u32,
    /// Required item count (`dwCount`).
    pub count: u32,
}

impl ObjectMaterial {
    /// Exact fixed x86 wire size (`sizeof(building::TObjectMaterial)`).
    pub const WIRE_SIZE: usize = OBJECT_MATERIAL_WIRE_SIZE;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.item_vnum);
        out.u32(self.count);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("building::TObjectMaterial", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "building::TObjectMaterial");
        let material = Self {
            item_vnum: reader.u32()?,
            count: reader.u32()?,
        };
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(material)
    }
}

impl_fixed_codec!(
    ObjectMaterial,
    "building::TObjectMaterial",
    OBJECT_MATERIAL_WIRE_SIZE
);

/// `building::TObjectProto`: one building-object prototype boot record.
///
/// The active x86 record is exactly 96 bytes. It contains two `DWORD` values,
/// five fixed two-`DWORD` material slots, upgrade and life values, four signed
/// x86 `long` regions, the NPC vnum and two signed x86 coordinates, and two
/// group vnums. The boot loader derives `npc_x` as zero and `npc_y` as
/// `max(regions[1], regions[3]) + 300` before sending the record. This codec
/// preserves the transmitted coordinate values and performs no host-layout
/// casts, pointer arithmetic, or C++ `long` assumptions beyond the active
/// little-endian x86 wire width.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObjectProtoRecord {
    /// Object prototype virtual number (`dwVnum`).
    pub vnum: u32,
    /// Building price (`dwPrice`).
    pub price: u32,
    /// Fixed material slots in legacy order (`kMaterials`).
    pub materials: [ObjectMaterial; OBJECT_MATERIAL_MAX_NUM],
    /// Upgrade prototype virtual number (`dwUpgradeVnum`).
    pub upgrade_vnum: u32,
    /// Upgrade time limit (`dwUpgradeLimitTime`).
    pub upgrade_limit_time: u32,
    /// Remaining life (`lLife`, active x86 `long`).
    pub life: i32,
    /// Signed region boundaries (`lRegion`, active x86 `long` values).
    pub regions: [i32; OBJECT_PROTO_REGION_COUNT],
    /// NPC virtual number (`dwNPCVnum`).
    pub npc_vnum: u32,
    /// Derived NPC X coordinate (`lNPCX`, active x86 `long`).
    pub npc_x: i32,
    /// Derived NPC Y coordinate (`lNPCY`, active x86 `long`).
    pub npc_y: i32,
    /// Object group virtual number (`dwGroupVnum`).
    pub group_vnum: u32,
    /// Dependent object group virtual number (`dwDependOnGroupVnum`).
    pub dependent_group_vnum: u32,
}

impl ObjectProtoRecord {
    /// Exact fixed x86 wire size (`sizeof(building::TObjectProto)`).
    pub const WIRE_SIZE: usize = OBJECT_PROTO_RECORD_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.vnum);
        out.u32(self.price);
        for material in &self.materials {
            out.u32(material.item_vnum);
            out.u32(material.count);
        }
        out.u32(self.upgrade_vnum);
        out.u32(self.upgrade_limit_time);
        out.i32(self.life);
        for region in self.regions {
            out.i32(region);
        }
        out.u32(self.npc_vnum);
        out.i32(self.npc_x);
        out.i32(self.npc_y);
        out.u32(self.group_vnum);
        out.u32(self.dependent_group_vnum);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("building::TObjectProto", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "building::TObjectProto");
        let vnum = reader.u32()?;
        let price = reader.u32()?;
        let mut materials = [ObjectMaterial::default(); OBJECT_MATERIAL_MAX_NUM];
        for material in &mut materials {
            material.item_vnum = reader.u32()?;
            material.count = reader.u32()?;
        }
        let upgrade_vnum = reader.u32()?;
        let upgrade_limit_time = reader.u32()?;
        let life = reader.i32()?;
        let mut regions = [0_i32; OBJECT_PROTO_REGION_COUNT];
        for region in &mut regions {
            *region = reader.i32()?;
        }
        let npc_vnum = reader.u32()?;
        let npc_x = reader.i32()?;
        let npc_y = reader.i32()?;
        let group_vnum = reader.u32()?;
        let dependent_group_vnum = reader.u32()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            vnum,
            price,
            materials,
            upgrade_vnum,
            upgrade_limit_time,
            life,
            regions,
            npc_vnum,
            npc_x,
            npc_y,
            group_vnum,
            dependent_group_vnum,
        })
    }
}

impl_fixed_codec!(
    ObjectProtoRecord,
    "building::TObjectProto",
    OBJECT_PROTO_RECORD_WIRE_SIZE
);

/// `building::TObject`: one object record in the active x86 boot stream.
///
/// The legacy object record is a packed 40-byte sequence of three `DWORD`
/// identifiers, three signed x86 `long` values, three IEEE-754 `float`
/// rotations, and a final signed x86 `long`. Rust field layout is deliberately
/// not used; the codec writes each value with an explicit little-endian
/// representation. Float payloads are transferred as raw IEEE-754 bits, so
/// NaN payloads and signed zero are preserved.
#[derive(Debug, Clone, Copy, Default)]
pub struct ObjectRecord {
    /// Object identifier (`dwID`).
    pub id: u32,
    /// Owning land identifier (`dwLandID`).
    pub land_id: u32,
    /// Object virtual number (`dwVnum`).
    pub vnum: u32,
    /// Map index (`lMapIndex`, x86 `long`).
    pub map_index: i32,
    /// X coordinate (`x`, x86 `long`).
    pub x: i32,
    /// Y coordinate (`y`, x86 `long`).
    pub y: i32,
    /// X rotation (`xRot`, x86 `float`).
    pub x_rot: f32,
    /// Y rotation (`yRot`, x86 `float`).
    pub y_rot: f32,
    /// Z rotation (`zRot`, x86 `float`).
    pub z_rot: f32,
    /// Remaining life (`lLife`, x86 `long`).
    pub life: i32,
}

impl PartialEq for ObjectRecord {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.land_id == other.land_id
            && self.vnum == other.vnum
            && self.map_index == other.map_index
            && self.x == other.x
            && self.y == other.y
            && self.x_rot.to_bits() == other.x_rot.to_bits()
            && self.y_rot.to_bits() == other.y_rot.to_bits()
            && self.z_rot.to_bits() == other.z_rot.to_bits()
            && self.life == other.life
    }
}

impl ObjectRecord {
    /// Exact fixed x86 wire size (`sizeof(building::TObject)`).
    pub const WIRE_SIZE: usize = OBJECT_RECORD_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.id);
        out.u32(self.land_id);
        out.u32(self.vnum);
        out.i32(self.map_index);
        out.i32(self.x);
        out.i32(self.y);
        // Encode the bit pattern rather than relying on a host float layout.
        out.u32(self.x_rot.to_bits());
        out.u32(self.y_rot.to_bits());
        out.u32(self.z_rot.to_bits());
        out.i32(self.life);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("building::TObject", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "building::TObject");
        let id = reader.u32()?;
        let land_id = reader.u32()?;
        let vnum = reader.u32()?;
        let map_index = reader.i32()?;
        let x = reader.i32()?;
        let y = reader.i32()?;
        let x_rot = f32::from_bits(reader.u32()?);
        let y_rot = f32::from_bits(reader.u32()?);
        let z_rot = f32::from_bits(reader.u32()?);
        let life = reader.i32()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            id,
            land_id,
            vnum,
            map_index,
            x,
            y,
            x_rot,
            y_rot,
            z_rot,
            life,
        })
    }
}

impl_fixed_codec!(ObjectRecord, "building::TObject", OBJECT_RECORD_WIRE_SIZE);

/// The three-byte packed x86 `TItemPos` embedded in a shop item row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopItemPosition {
    /// Inventory window type (`window_type`).
    pub window_type: u8,
    /// Inventory cell (`cell`).
    pub cell: u16,
}

impl Default for ShopItemPosition {
    /// Match the legacy `TItemPos()` constructor: inventory window and the
    /// invalid/empty `WORD_MAX` cell, rather than a packed zero position.
    fn default() -> Self {
        Self::new(SHOP_ITEM_DEFAULT_WINDOW_TYPE, SHOP_ITEM_DEFAULT_CELL)
    }
}

impl ShopItemPosition {
    /// Exact packed x86 wire size (`sizeof(TItemPos)`).
    pub const WIRE_SIZE: usize = SHOP_ITEM_POSITION_WIRE_SIZE;

    /// Construct a position from its two legacy fields.
    #[must_use]
    pub const fn new(window_type: u8, cell: u16) -> Self {
        Self { window_type, cell }
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u8(self.window_type);
        out.u16(self.cell);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TItemPos", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TItemPos");
        let position = Self {
            window_type: reader.u8()?,
            cell: reader.u16()?,
        };
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(position)
    }
}

impl_fixed_codec!(ShopItemPosition, "TItemPos", SHOP_ITEM_POSITION_WIRE_SIZE);

/// The three-byte packed x86 `TPlayerItemAttribute` embedded in a shop row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShopItemAttribute {
    /// Attribute selector (`bType`).
    pub attr_type: u8,
    /// Signed attribute value (`sValue`, x86 `short`).
    pub value: i16,
}

impl ShopItemAttribute {
    /// Exact packed x86 wire size (`sizeof(TPlayerItemAttribute)`).
    pub const WIRE_SIZE: usize = SHOP_ITEM_ATTRIBUTE_WIRE_SIZE;

    /// Construct an attribute from its two legacy fields.
    #[must_use]
    pub const fn new(attr_type: u8, value: i16) -> Self {
        Self { attr_type, value }
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u8(self.attr_type);
        out.i16(self.value);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TPlayerItemAttribute", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TPlayerItemAttribute");
        let attribute = Self {
            attr_type: reader.u8()?,
            value: reader.i16()?,
        };
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(attribute)
    }
}

impl_fixed_codec!(
    ShopItemAttribute,
    "TPlayerItemAttribute",
    SHOP_ITEM_ATTRIBUTE_WIRE_SIZE
);

/// `TShopItemTable`: one fixed item slot in the active base shop profile.
///
/// The active x86 profile has the 64-bit `price` field enabled by
/// `ENABLE_REMOVE_LIMIT_GOLD` and the renewal-shop socket/attribute fields.
/// The base and optional renewal-shop sections share this wire layout; their
/// SQL projection and initialization policies are separate. Every integer is
/// encoded explicitly in little-endian order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopItemRecord {
    /// Item virtual number (`vnum`).
    pub vnum: u32,
    /// Item count (`count`).
    pub count: u16,
    /// Packed inventory position (`pos`).
    pub pos: ShopItemPosition,
    /// Item price (`price`, active unsigned 64-bit field).
    pub price: u64,
    /// Client display position (`display_pos`).
    pub display_pos: u8,
    /// Fixed socket values (`alSockets`).
    pub sockets: [i32; SHOP_ITEM_SOCKET_MAX_NUM],
    /// Fixed item attributes (`aAttr`).
    pub attrs: [ShopItemAttribute; SHOP_ITEM_ATTRIBUTE_MAX_NUM],
    /// Price selector (`price_type`).
    pub price_type: u8,
    /// Price-item virtual number (`price_vnum`).
    pub price_vnum: u32,
}

impl ShopItemRecord {
    /// Exact packed x86 wire size (`sizeof(TShopItemTable)`).
    pub const WIRE_SIZE: usize = SHOP_ITEM_RECORD_WIRE_SIZE;

    /// Construct a source-shaped, deterministic shop-item slot.
    ///
    /// The active legacy loader uses `TShopTable{}` value-initialization before
    /// the `TShopItemTable` constructor runs. Scalar fields are therefore
    /// deterministic zeros, while the embedded `TItemPos` default is retained
    /// (`INVENTORY`, `WORD_MAX`) and `price_type` is `SHOPEX_GOLD` as set by
    /// the legacy constructor.
    #[must_use]
    pub const fn zeroed() -> Self {
        Self {
            vnum: 0,
            count: 0,
            pos: ShopItemPosition {
                window_type: SHOP_ITEM_DEFAULT_WINDOW_TYPE,
                cell: SHOP_ITEM_DEFAULT_CELL,
            },
            price: 0,
            display_pos: 0,
            sockets: [0; SHOP_ITEM_SOCKET_MAX_NUM],
            attrs: [ShopItemAttribute {
                attr_type: 0,
                value: 0,
            }; SHOP_ITEM_ATTRIBUTE_MAX_NUM],
            price_type: SHOP_PRICE_TYPE_GOLD,
            price_vnum: 0,
        }
    }

    /// Construct the all-zero item-slot state used after the renewal
    /// `InitializeShopEXTable` `memset` on a newly created `TShopTable`.
    ///
    /// This intentionally differs from [`ShopItemRecord::zeroed`], which
    /// models the `TShopItemTable` constructor defaults for the base loader.
    #[must_use]
    pub const fn zeroed_renewal_slot() -> Self {
        Self {
            vnum: 0,
            count: 0,
            pos: ShopItemPosition::new(0, 0),
            price: 0,
            display_pos: 0,
            sockets: [0; SHOP_ITEM_SOCKET_MAX_NUM],
            attrs: [ShopItemAttribute {
                attr_type: 0,
                value: 0,
            }; SHOP_ITEM_ATTRIBUTE_MAX_NUM],
            price_type: 0,
            price_vnum: 0,
        }
    }

    /// Construct a basic item slot, retaining the default price selector.
    #[must_use]
    pub const fn new(vnum: u32, count: u16, price: u64) -> Self {
        Self {
            vnum,
            count,
            price,
            ..Self::zeroed()
        }
    }

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.vnum);
        out.u16(self.count);
        out.u8(self.pos.window_type);
        out.u16(self.pos.cell);
        out.u64(self.price);
        out.u8(self.display_pos);
        for socket in self.sockets {
            out.i32(socket);
        }
        for attribute in &self.attrs {
            out.u8(attribute.attr_type);
            out.i16(attribute.value);
        }
        out.u8(self.price_type);
        out.u32(self.price_vnum);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TShopItemTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TShopItemTable");
        let vnum = reader.u32()?;
        let count = reader.u16()?;
        let pos = ShopItemPosition {
            window_type: reader.u8()?,
            cell: reader.u16()?,
        };
        let price = reader.u64()?;
        let display_pos = reader.u8()?;
        let mut sockets = [0_i32; SHOP_ITEM_SOCKET_MAX_NUM];
        for socket in &mut sockets {
            *socket = reader.i32()?;
        }
        let mut attrs = [ShopItemAttribute::default(); SHOP_ITEM_ATTRIBUTE_MAX_NUM];
        for attribute in &mut attrs {
            attribute.attr_type = reader.u8()?;
            attribute.value = reader.i16()?;
        }
        let price_type = reader.u8()?;
        let price_vnum = reader.u32()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            vnum,
            count,
            pos,
            price,
            display_pos,
            sockets,
            attrs,
            price_type,
            price_vnum,
        })
    }
}

impl Default for ShopItemRecord {
    fn default() -> Self {
        Self::zeroed()
    }
}

impl_fixed_codec!(ShopItemRecord, "TShopItemTable", SHOP_ITEM_RECORD_WIRE_SIZE);

/// `TShopTable`: one fixed base shop record in the boot stream.
///
/// The field order is `dwVnum`, `dwNPCVnum`, `byItemCount`, forty fixed
/// `TShopItemTable` slots, and thirty-three raw shop-name bytes. The Rust
/// fields are domain values only; all wire bytes are written explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopTableRecord {
    /// Shop virtual number (`dwVnum`).
    pub vnum: u32,
    /// NPC virtual number (`dwNPCVnum`).
    pub npc_vnum: u32,
    /// Number of populated item slots (`byItemCount`).
    pub item_count: u8,
    /// Fixed item slots (`items[SHOP_HOST_ITEM_MAX_NUM]`).
    pub items: [ShopItemRecord; SHOP_HOST_ITEM_MAX_NUM],
    /// Raw shop-name bytes (`szShopName[SHOP_SIGN_MAX_LEN + 1]`).
    pub shop_name: [u8; SHOP_SIGN_BYTES],
}

impl ShopTableRecord {
    /// Exact packed x86 wire size (`sizeof(TShopTable)`).
    pub const WIRE_SIZE: usize = SHOP_TABLE_RECORD_WIRE_SIZE;

    /// Construct a deterministic base shop with source-shaped default item
    /// slots. The active `TShopTable{}` value-initialization makes scalar
    /// fields zero while keeping the `TItemPos` and `SHOPEX_GOLD` constructor
    /// defaults.
    #[must_use]
    pub const fn zeroed_base_shop() -> Self {
        Self {
            vnum: 0,
            npc_vnum: 0,
            item_count: 0,
            items: [ShopItemRecord::zeroed(); SHOP_HOST_ITEM_MAX_NUM],
            shop_name: [0; SHOP_SIGN_BYTES],
        }
    }

    /// Construct the all-zero record state used by the renewal-shop loader.
    ///
    /// `InitializeShopEXTable` allocates and `memset`s a `TShopTable` before
    /// copying any row, so renewal-shop slots do not inherit the base
    /// `TShopItemTable` constructor defaults.
    #[must_use]
    pub const fn zeroed_renewal_shop() -> Self {
        Self {
            vnum: 0,
            npc_vnum: 0,
            item_count: 0,
            items: [ShopItemRecord::zeroed_renewal_slot(); SHOP_HOST_ITEM_MAX_NUM],
            shop_name: [0; SHOP_SIGN_BYTES],
        }
    }

    /// Construct a shop from all legacy fields and raw name bytes.
    #[must_use]
    pub const fn new(
        vnum: u32,
        npc_vnum: u32,
        item_count: u8,
        items: [ShopItemRecord; SHOP_HOST_ITEM_MAX_NUM],
        shop_name: [u8; SHOP_SIGN_BYTES],
    ) -> Self {
        Self {
            vnum,
            npc_vnum,
            item_count,
            items,
            shop_name,
        }
    }

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.vnum);
        out.u32(self.npc_vnum);
        out.u8(self.item_count);
        for item in &self.items {
            let encoded = item.encode_wire();
            debug_assert_eq!(encoded.len(), ShopItemRecord::WIRE_SIZE);
            out.bytes(&encoded);
        }
        out.bytes(&self.shop_name);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TShopTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TShopTable");
        let vnum = reader.u32()?;
        let npc_vnum = reader.u32()?;
        let item_count = reader.u8()?;
        let mut items = [ShopItemRecord::zeroed(); SHOP_HOST_ITEM_MAX_NUM];
        for item in &mut items {
            *item = ShopItemRecord::decode_wire(reader.take(ShopItemRecord::WIRE_SIZE)?)?;
        }
        let shop_name = reader.array()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            vnum,
            npc_vnum,
            item_count,
            items,
            shop_name,
        })
    }
}

impl Default for ShopTableRecord {
    fn default() -> Self {
        Self::zeroed_base_shop()
    }
}

impl_fixed_codec!(ShopTableRecord, "TShopTable", SHOP_TABLE_RECORD_WIRE_SIZE);

/// Compatibility aliases for the nested legacy shop types.
pub type ShopItemPositionRecord = ShopItemPosition;
/// Compatibility alias for the nested legacy shop attribute type.
pub type ShopItemAttributeRecord = ShopItemAttribute;
/// C++-style alias for `TItemPos` as used by `TShopItemTable`.
pub type TItemPos = ShopItemPosition;
/// C++-style alias for `TPlayerItemAttribute` as used by `TShopItemTable`.
pub type TPlayerItemAttribute = ShopItemAttribute;
/// C++-style alias for `TShopItemTable`.
pub type TShopItemTable = ShopItemRecord;
/// C++-style alias for `TShopTable`.
pub type TShopTable = ShopTableRecord;
/// Compatibility alias for the renewal-shop item record; it shares the
/// source-fixed 68-byte x86 wire layout.
pub type RenewalShopItemRecord = ShopItemRecord;
/// Compatibility alias for the renewal-shop table record; it shares the
/// source-fixed 2,762-byte x86 wire layout.
pub type RenewalShopTableRecord = ShopTableRecord;

/// `TMarketItemPrice`: one row in the optional premium-market boot section.
///
/// The active x86 build packs `DWORD dwVnum`, `TItemPrice` (`long long
/// llGold`, `DWORD dwCheque`), and no padding into 16 bytes. The field order
/// is written explicitly because Rust alignment must not affect the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MarketItemPriceRecord {
    /// Item virtual number.
    pub vnum: u32,
    /// Gold amount from `TItemPrice::llGold`.
    pub gold: i64,
    /// Cheque amount from `TItemPrice::dwCheque`.
    pub cheque: u32,
}

impl MarketItemPriceRecord {
    /// Exact packed x86 wire size (`sizeof(TMarketItemPrice)`).
    pub const WIRE_SIZE: usize = MARKET_ITEM_PRICE_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.vnum);
        out.i64(self.gold);
        out.u32(self.cheque);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TMarketItemPrice", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TMarketItemPrice");
        Ok(Self {
            vnum: reader.u32()?,
            gold: reader.i64()?,
            cheque: reader.u32()?,
        })
    }
}

impl_fixed_codec!(
    MarketItemPriceRecord,
    "TMarketItemPrice",
    MARKET_ITEM_PRICE_WIRE_SIZE
);

/// `TEventTable`: one row in the optional event-manager boot section.
///
/// The active x86 build packs `DWORD dwID`, `char szType[64]`, two x86
/// `long` timestamps, two x86 `int` values, and the one-byte C++ `bool`
/// `bCompleted` into 85 bytes. The boolean is kept as `u8` here because the
/// legacy sender copies the packed object and a raw byte is the lossless wire
/// representation; callers can interpret it as a boolean only after applying
/// their own validation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventTableRecord {
    /// Event identifier (`dwID`).
    pub id: u32,
    /// Event type string bytes (`szType[64]`).
    pub event_type: [u8; EVENT_TYPE_BYTES],
    /// Start timestamp (`startTime`, x86 `long`).
    pub start_time: i32,
    /// End timestamp (`endTime`, x86 `long`).
    pub end_time: i32,
    /// First event value (`iValue0`).
    pub value0: i32,
    /// Second event value (`iValue1`).
    pub value1: i32,
    /// Raw C++ `bool` byte (`bCompleted`).
    pub completed: u8,
}

impl Default for EventTableRecord {
    fn default() -> Self {
        Self {
            id: 0,
            event_type: [0; EVENT_TYPE_BYTES],
            start_time: 0,
            end_time: 0,
            value0: 0,
            value1: 0,
            completed: 0,
        }
    }
}

impl EventTableRecord {
    /// Exact packed x86 wire size (`sizeof(TEventTable)`).
    pub const WIRE_SIZE: usize = EVENT_TABLE_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.id);
        out.bytes(&self.event_type);
        out.i32(self.start_time);
        out.i32(self.end_time);
        out.i32(self.value0);
        out.i32(self.value1);
        out.u8(self.completed);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TEventTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TEventTable");
        Ok(Self {
            id: reader.u32()?,
            event_type: reader.array()?,
            start_time: reader.i32()?,
            end_time: reader.i32()?,
            value0: reader.i32()?,
            value1: reader.i32()?,
            completed: reader.u8()?,
        })
    }
}

impl_fixed_codec!(EventTableRecord, "TEventTable", EVENT_TABLE_WIRE_SIZE);

/// `TSkillTable`: one packed skill definition from the boot table.
///
/// The active x86 source applies `#pragma pack(1)` before this structure.
/// Every `char` array is retained as raw fixed-width bytes, including bytes
/// after a NUL terminator. The Rust field layout is not a wire format.
/// Integers are encoded and decoded explicitly in little-endian order, and
/// x86 `long lMaxHit` is represented as a signed 32-bit value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillTableRecord {
    /// `dwVnum`.
    pub vnum: u32,
    /// `szName[32 + 1]`.
    pub name: [u8; SKILL_NAME_BYTES],
    /// `bType`.
    pub skill_type: u8,
    /// `bMaxLevel`.
    pub max_level: u8,
    /// `dwSplashRange`.
    pub splash_range: u32,
    /// `szPointOn[64]`.
    pub point_on: [u8; SKILL_POINT_ON_BYTES],
    /// `szPointPoly[100 + 1]`.
    pub point_poly: [u8; SKILL_POLY_EXPR_BYTES],
    /// `szSPCostPoly[100 + 1]`.
    pub sp_cost_poly: [u8; SKILL_POLY_EXPR_BYTES],
    /// `szDurationPoly[100 + 1]`.
    pub duration_poly: [u8; SKILL_POLY_EXPR_BYTES],
    /// `szDurationSPCostPoly[100 + 1]`.
    pub duration_sp_cost_poly: [u8; SKILL_POLY_EXPR_BYTES],
    /// `szCooldownPoly[100 + 1]`.
    pub cooldown_poly: [u8; SKILL_POLY_EXPR_BYTES],
    /// `szMasterBonusPoly[100 + 1]`.
    pub master_bonus_poly: [u8; SKILL_POLY_EXPR_BYTES],
    /// `szGrandMasterAddSPCostPoly[100 + 1]`.
    pub grand_master_add_sp_cost_poly: [u8; SKILL_POLY_EXPR_BYTES],
    /// `dwFlag`.
    pub flag: u32,
    /// `dwAffectFlag`.
    pub affect_flag: u32,
    /// `szPointOn2[64]`.
    pub point_on2: [u8; SKILL_POINT_ON_BYTES],
    /// `szPointPoly2[100 + 1]`.
    pub point_poly2: [u8; SKILL_POLY_EXPR_BYTES],
    /// `szDurationPoly2[100 + 1]`.
    pub duration_poly2: [u8; SKILL_POLY_EXPR_BYTES],
    /// `dwAffectFlag2`.
    pub affect_flag2: u32,
    /// `szPointOn3[64]`.
    pub point_on3: [u8; SKILL_POINT_ON_BYTES],
    /// `szPointPoly3[100 + 1]`.
    pub point_poly3: [u8; SKILL_POLY_EXPR_BYTES],
    /// `szDurationPoly3[100 + 1]`.
    pub duration_poly3: [u8; SKILL_POLY_EXPR_BYTES],
    /// `bLevelStep`.
    pub level_step: u8,
    /// `bLevelLimit`.
    pub level_limit: u8,
    /// `preSkillVnum`.
    pub pre_skill_vnum: u32,
    /// `preSkillLevel`.
    pub pre_skill_level: u8,
    /// `lMaxHit` (signed x86 `long`).
    pub max_hit: i32,
    /// `szSplashAroundDamageAdjustPoly[100 + 1]`.
    pub splash_around_damage_adjust_poly: [u8; SKILL_POLY_EXPR_BYTES],
    /// `bSkillAttrType`.
    pub skill_attr_type: u8,
    /// `dwTargetRange`.
    pub target_range: u32,
}

impl Default for SkillTableRecord {
    fn default() -> Self {
        Self {
            vnum: 0,
            name: [0; SKILL_NAME_BYTES],
            skill_type: 0,
            max_level: 0,
            splash_range: 0,
            point_on: [0; SKILL_POINT_ON_BYTES],
            point_poly: [0; SKILL_POLY_EXPR_BYTES],
            sp_cost_poly: [0; SKILL_POLY_EXPR_BYTES],
            duration_poly: [0; SKILL_POLY_EXPR_BYTES],
            duration_sp_cost_poly: [0; SKILL_POLY_EXPR_BYTES],
            cooldown_poly: [0; SKILL_POLY_EXPR_BYTES],
            master_bonus_poly: [0; SKILL_POLY_EXPR_BYTES],
            grand_master_add_sp_cost_poly: [0; SKILL_POLY_EXPR_BYTES],
            flag: 0,
            affect_flag: 0,
            point_on2: [0; SKILL_POINT_ON_BYTES],
            point_poly2: [0; SKILL_POLY_EXPR_BYTES],
            duration_poly2: [0; SKILL_POLY_EXPR_BYTES],
            affect_flag2: 0,
            point_on3: [0; SKILL_POINT_ON_BYTES],
            point_poly3: [0; SKILL_POLY_EXPR_BYTES],
            duration_poly3: [0; SKILL_POLY_EXPR_BYTES],
            level_step: 0,
            level_limit: 0,
            pre_skill_vnum: 0,
            pre_skill_level: 0,
            max_hit: 0,
            splash_around_damage_adjust_poly: [0; SKILL_POLY_EXPR_BYTES],
            skill_attr_type: 0,
            target_range: 0,
        }
    }
}

impl SkillTableRecord {
    /// Exact packed x86 wire size (`sizeof(TSkillTable)`).
    pub const WIRE_SIZE: usize = SKILL_TABLE_RECORD_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.vnum);
        out.bytes(&self.name);
        out.u8(self.skill_type);
        out.u8(self.max_level);
        out.u32(self.splash_range);
        out.bytes(&self.point_on);
        out.bytes(&self.point_poly);
        out.bytes(&self.sp_cost_poly);
        out.bytes(&self.duration_poly);
        out.bytes(&self.duration_sp_cost_poly);
        out.bytes(&self.cooldown_poly);
        out.bytes(&self.master_bonus_poly);
        out.bytes(&self.grand_master_add_sp_cost_poly);
        out.u32(self.flag);
        out.u32(self.affect_flag);
        out.bytes(&self.point_on2);
        out.bytes(&self.point_poly2);
        out.bytes(&self.duration_poly2);
        out.u32(self.affect_flag2);
        out.bytes(&self.point_on3);
        out.bytes(&self.point_poly3);
        out.bytes(&self.duration_poly3);
        out.u8(self.level_step);
        out.u8(self.level_limit);
        out.u32(self.pre_skill_vnum);
        out.u8(self.pre_skill_level);
        out.i32(self.max_hit);
        out.bytes(&self.splash_around_damage_adjust_poly);
        out.u8(self.skill_attr_type);
        out.u32(self.target_range);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TSkillTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TSkillTable");
        let vnum = reader.u32()?;
        let name = reader.array()?;
        let skill_type = reader.u8()?;
        let max_level = reader.u8()?;
        let splash_range = reader.u32()?;
        let point_on = reader.array()?;
        let point_poly = reader.array()?;
        let sp_cost_poly = reader.array()?;
        let duration_poly = reader.array()?;
        let duration_sp_cost_poly = reader.array()?;
        let cooldown_poly = reader.array()?;
        let master_bonus_poly = reader.array()?;
        let grand_master_add_sp_cost_poly = reader.array()?;
        let flag = reader.u32()?;
        let affect_flag = reader.u32()?;
        let point_on2 = reader.array()?;
        let point_poly2 = reader.array()?;
        let duration_poly2 = reader.array()?;
        let affect_flag2 = reader.u32()?;
        let point_on3 = reader.array()?;
        let point_poly3 = reader.array()?;
        let duration_poly3 = reader.array()?;
        let level_step = reader.u8()?;
        let level_limit = reader.u8()?;
        let pre_skill_vnum = reader.u32()?;
        let pre_skill_level = reader.u8()?;
        let max_hit = reader.i32()?;
        let splash_around_damage_adjust_poly = reader.array()?;
        let skill_attr_type = reader.u8()?;
        let target_range = reader.u32()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            vnum,
            name,
            skill_type,
            max_level,
            splash_range,
            point_on,
            point_poly,
            sp_cost_poly,
            duration_poly,
            duration_sp_cost_poly,
            cooldown_poly,
            master_bonus_poly,
            grand_master_add_sp_cost_poly,
            flag,
            affect_flag,
            point_on2,
            point_poly2,
            duration_poly2,
            affect_flag2,
            point_on3,
            point_poly3,
            duration_poly3,
            level_step,
            level_limit,
            pre_skill_vnum,
            pre_skill_level,
            max_hit,
            splash_around_damage_adjust_poly,
            skill_attr_type,
            target_range,
        })
    }
}

impl_fixed_codec!(
    SkillTableRecord,
    "TSkillTable",
    SKILL_TABLE_RECORD_WIRE_SIZE
);

/// `TMobSkillLevel`: one fixed skill slot in a `TMobTable` row.
///
/// The packed x86 record consists of a little-endian `DWORD dwVnum` followed
/// by one `BYTE bLevel`. No range or enum interpretation is performed here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MobSkillRecord {
    /// Skill virtual number (`dwVnum`).
    pub vnum: u32,
    /// Skill level (`bLevel`).
    pub level: u8,
}

impl MobSkillRecord {
    /// Exact packed x86 wire size (`sizeof(TMobSkillLevel)`).
    pub const WIRE_SIZE: usize = MOB_SKILL_RECORD_WIRE_SIZE;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.vnum);
        out.u8(self.level);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TMobSkillLevel", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TMobSkillLevel");
        let vnum = reader.u32()?;
        let level = reader.u8()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self { vnum, level })
    }
}

impl_fixed_codec!(MobSkillRecord, "TMobSkillLevel", MOB_SKILL_RECORD_WIRE_SIZE);

/// `TMobTable`: one packed monster or NPC prototype from the boot table.
///
/// The active x86 source applies `#pragma pack(1)` around the inherited
/// `SEntityTable::dwVnum` prefix and this record. The resulting wire size is
/// exactly [`MOB_TABLE_RECORD_WIRE_SIZE`] bytes. Names and folder bytes are
/// retained without string validation or normalization. The C++ signed
/// `char` enchant/resist arrays are represented as `i8`, while speed fields
/// are signed 16-bit values. The float is encoded and compared by its exact
/// IEEE-754 bit pattern.
#[derive(Debug, Clone, Copy)]
pub struct MobTableRecord {
    /// Inherited `SEntityTable::dwVnum`.
    pub vnum: u32,
    /// Raw `szName[CHARACTER_NAME_MAX_LEN + 1]` bytes.
    pub name: [u8; MOB_NAME_BYTES],
    /// Raw `szLocaleName[CHARACTER_NAME_MAX_LEN + 1]` bytes.
    pub locale_name: [u8; MOB_LOCALE_NAME_BYTES],
    /// Raw `bType` selector.
    pub mob_type: u8,
    /// Raw `bRank` selector.
    pub rank: u8,
    /// Raw `bBattleType` selector.
    pub battle_type: u8,
    /// Raw `bLevel` value.
    pub level: u8,
    /// Raw `bSize` value.
    pub size: u8,
    /// `dwGoldMin`.
    pub gold_min: u32,
    /// `dwGoldMax`.
    pub gold_max: u32,
    /// `dwExp`.
    pub exp: u32,
    /// `dwMaxHP`.
    pub max_hp: u32,
    /// Raw `bRegenCycle` value.
    pub regen_cycle: u8,
    /// Raw `bRegenPercent` value.
    pub regen_percent: u8,
    /// Unsigned x86 `wDef` value.
    pub def: u16,
    /// Raw `dwAIFlag` bits.
    pub ai_flag: u32,
    /// Raw `dwRaceFlag` bits.
    pub race_flag: u32,
    /// Raw `dwImmuneFlag` bits.
    pub immune_flag: u32,
    /// Raw `bStr` value.
    pub str: u8,
    /// Raw `bDex` value.
    pub dex: u8,
    /// Raw `bCon` value.
    pub con: u8,
    /// Raw `bInt` value.
    pub int_: u8,
    /// Raw `dwDamageRange[2]` values.
    pub damage_range: [u32; MOB_DAMAGE_RANGE_COUNT],
    /// Signed x86 `sAttackSpeed` value.
    pub attack_speed: i16,
    /// Signed x86 `sMovingSpeed` value.
    pub moving_speed: i16,
    /// Raw `bAggresiveHPPct` value (spelling follows the C++ field).
    pub aggressive_hp_pct: u8,
    /// Raw `wAggressiveSight` value.
    pub aggressive_sight: u16,
    /// Raw `wAttackRange` value.
    pub attack_range: u16,
    /// Signed C++ `char cEnchants[MOB_ENCHANTS_MAX_NUM]` values.
    pub enchants: [i8; MOB_ENCHANTS_MAX_NUM],
    /// Signed C++ `char cResists[MOB_RESISTS_MAX_NUM]` values.
    pub resists: [i8; MOB_RESISTS_MAX_NUM],
    /// Raw `dwResurrectionVnum` value.
    pub resurrection_vnum: u32,
    /// Raw `dwDropItemVnum` value.
    pub drop_item_vnum: u32,
    /// Raw `bMountCapacity` value.
    pub mount_capacity: u8,
    /// Raw `bOnClickType` selector.
    pub on_click_type: u8,
    /// Raw `bEmpire` value.
    pub empire: u8,
    /// Raw `szFolder[64 + 1]` bytes.
    pub folder: [u8; MOB_FOLDER_BYTES],
    /// IEEE-754 `fDamMultiply` bits.
    pub dam_multiply: f32,
    /// Raw `dwSummonVnum` value.
    pub summon_vnum: u32,
    /// Raw `dwDrainSP` value.
    pub drain_sp: u32,
    /// Raw `dwMobColor` value.
    pub mob_color: u32,
    /// Raw `dwPolymorphItemVnum` value.
    pub polymorph_item_vnum: u32,
    /// Fixed skill slots in legacy order.
    pub skills: [MobSkillRecord; MOB_SKILL_MAX_NUM],
    /// Raw `bBerserkPoint` value.
    pub berserk_point: u8,
    /// Raw `bStoneSkinPoint` value.
    pub stone_skin_point: u8,
    /// Raw `bGodSpeedPoint` value.
    pub god_speed_point: u8,
    /// Raw `bDeathBlowPoint` value.
    pub death_blow_point: u8,
    /// Raw `bRevivePoint` value.
    pub revive_point: u8,
}

impl Default for MobTableRecord {
    fn default() -> Self {
        Self {
            vnum: 0,
            name: [0; MOB_NAME_BYTES],
            locale_name: [0; MOB_LOCALE_NAME_BYTES],
            mob_type: 0,
            rank: 0,
            battle_type: 0,
            level: 0,
            size: 0,
            gold_min: 0,
            gold_max: 0,
            exp: 0,
            max_hp: 0,
            regen_cycle: 0,
            regen_percent: 0,
            def: 0,
            ai_flag: 0,
            race_flag: 0,
            immune_flag: 0,
            str: 0,
            dex: 0,
            con: 0,
            int_: 0,
            damage_range: [0; MOB_DAMAGE_RANGE_COUNT],
            attack_speed: 0,
            moving_speed: 0,
            aggressive_hp_pct: 0,
            aggressive_sight: 0,
            attack_range: 0,
            enchants: [0; MOB_ENCHANTS_MAX_NUM],
            resists: [0; MOB_RESISTS_MAX_NUM],
            resurrection_vnum: 0,
            drop_item_vnum: 0,
            mount_capacity: 0,
            on_click_type: 0,
            empire: 0,
            folder: [0; MOB_FOLDER_BYTES],
            dam_multiply: 0.0,
            summon_vnum: 0,
            drain_sp: 0,
            mob_color: 0,
            polymorph_item_vnum: 0,
            skills: [MobSkillRecord::default(); MOB_SKILL_MAX_NUM],
            berserk_point: 0,
            stone_skin_point: 0,
            god_speed_point: 0,
            death_blow_point: 0,
            revive_point: 0,
        }
    }
}

impl PartialEq for MobTableRecord {
    fn eq(&self, other: &Self) -> bool {
        self.vnum == other.vnum
            && self.name == other.name
            && self.locale_name == other.locale_name
            && self.mob_type == other.mob_type
            && self.rank == other.rank
            && self.battle_type == other.battle_type
            && self.level == other.level
            && self.size == other.size
            && self.gold_min == other.gold_min
            && self.gold_max == other.gold_max
            && self.exp == other.exp
            && self.max_hp == other.max_hp
            && self.regen_cycle == other.regen_cycle
            && self.regen_percent == other.regen_percent
            && self.def == other.def
            && self.ai_flag == other.ai_flag
            && self.race_flag == other.race_flag
            && self.immune_flag == other.immune_flag
            && self.str == other.str
            && self.dex == other.dex
            && self.con == other.con
            && self.int_ == other.int_
            && self.damage_range == other.damage_range
            && self.attack_speed == other.attack_speed
            && self.moving_speed == other.moving_speed
            && self.aggressive_hp_pct == other.aggressive_hp_pct
            && self.aggressive_sight == other.aggressive_sight
            && self.attack_range == other.attack_range
            && self.enchants == other.enchants
            && self.resists == other.resists
            && self.resurrection_vnum == other.resurrection_vnum
            && self.drop_item_vnum == other.drop_item_vnum
            && self.mount_capacity == other.mount_capacity
            && self.on_click_type == other.on_click_type
            && self.empire == other.empire
            && self.folder == other.folder
            && self.dam_multiply.to_bits() == other.dam_multiply.to_bits()
            && self.summon_vnum == other.summon_vnum
            && self.drain_sp == other.drain_sp
            && self.mob_color == other.mob_color
            && self.polymorph_item_vnum == other.polymorph_item_vnum
            && self.skills == other.skills
            && self.berserk_point == other.berserk_point
            && self.stone_skin_point == other.stone_skin_point
            && self.god_speed_point == other.god_speed_point
            && self.death_blow_point == other.death_blow_point
            && self.revive_point == other.revive_point
    }
}

impl MobTableRecord {
    /// Exact packed x86 wire size (`sizeof(TMobTable)`).
    pub const WIRE_SIZE: usize = MOB_TABLE_RECORD_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.vnum);
        out.bytes(&self.name);
        out.bytes(&self.locale_name);
        out.u8(self.mob_type);
        out.u8(self.rank);
        out.u8(self.battle_type);
        out.u8(self.level);
        out.u8(self.size);
        out.u32(self.gold_min);
        out.u32(self.gold_max);
        out.u32(self.exp);
        out.u32(self.max_hp);
        out.u8(self.regen_cycle);
        out.u8(self.regen_percent);
        out.u16(self.def);
        out.u32(self.ai_flag);
        out.u32(self.race_flag);
        out.u32(self.immune_flag);
        out.u8(self.str);
        out.u8(self.dex);
        out.u8(self.con);
        out.u8(self.int_);
        for value in self.damage_range {
            out.u32(value);
        }
        out.i16(self.attack_speed);
        out.i16(self.moving_speed);
        out.u8(self.aggressive_hp_pct);
        out.u16(self.aggressive_sight);
        out.u16(self.attack_range);
        for value in self.enchants {
            out.bytes(&value.to_le_bytes());
        }
        for value in self.resists {
            out.bytes(&value.to_le_bytes());
        }
        out.u32(self.resurrection_vnum);
        out.u32(self.drop_item_vnum);
        out.u8(self.mount_capacity);
        out.u8(self.on_click_type);
        out.u8(self.empire);
        out.bytes(&self.folder);
        out.u32(self.dam_multiply.to_bits());
        out.u32(self.summon_vnum);
        out.u32(self.drain_sp);
        out.u32(self.mob_color);
        out.u32(self.polymorph_item_vnum);
        for skill in &self.skills {
            out.u32(skill.vnum);
            out.u8(skill.level);
        }
        out.u8(self.berserk_point);
        out.u8(self.stone_skin_point);
        out.u8(self.god_speed_point);
        out.u8(self.death_blow_point);
        out.u8(self.revive_point);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    #[allow(clippy::similar_names, clippy::too_many_lines)]
    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TMobTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TMobTable");
        let vnum = reader.u32()?;
        let name = reader.array()?;
        let locale_name = reader.array()?;
        let mob_type = reader.u8()?;
        let rank = reader.u8()?;
        let battle_type = reader.u8()?;
        let level = reader.u8()?;
        let size = reader.u8()?;
        let gold_min = reader.u32()?;
        let gold_max = reader.u32()?;
        let exp = reader.u32()?;
        let max_hp = reader.u32()?;
        let regen_cycle = reader.u8()?;
        let regen_percent = reader.u8()?;
        let def = reader.u16()?;
        let ai_flag = reader.u32()?;
        let race_flag = reader.u32()?;
        let immune_flag = reader.u32()?;
        let str = reader.u8()?;
        let dex = reader.u8()?;
        let con = reader.u8()?;
        let int_ = reader.u8()?;
        let mut damage_range = [0_u32; MOB_DAMAGE_RANGE_COUNT];
        for value in &mut damage_range {
            *value = reader.u32()?;
        }
        let attack_speed = reader.i16()?;
        let moving_speed = reader.i16()?;
        let aggressive_hp_pct = reader.u8()?;
        let aggressive_sight = reader.u16()?;
        let attack_range = reader.u16()?;
        let mut enchants = [0_i8; MOB_ENCHANTS_MAX_NUM];
        for value in &mut enchants {
            *value = i8::from_le_bytes([reader.u8()?]);
        }
        let mut resists = [0_i8; MOB_RESISTS_MAX_NUM];
        for value in &mut resists {
            *value = i8::from_le_bytes([reader.u8()?]);
        }
        let resurrection_vnum = reader.u32()?;
        let drop_item_vnum = reader.u32()?;
        let mount_capacity = reader.u8()?;
        let on_click_type = reader.u8()?;
        let empire = reader.u8()?;
        let folder = reader.array()?;
        let dam_multiply = f32::from_bits(reader.u32()?);
        let summon_vnum = reader.u32()?;
        let drain_sp = reader.u32()?;
        let mob_color = reader.u32()?;
        let polymorph_item_vnum = reader.u32()?;
        let mut skills = [MobSkillRecord::default(); MOB_SKILL_MAX_NUM];
        for skill in &mut skills {
            skill.vnum = reader.u32()?;
            skill.level = reader.u8()?;
        }
        let berserk_point = reader.u8()?;
        let stone_skin_point = reader.u8()?;
        let god_speed_point = reader.u8()?;
        let death_blow_point = reader.u8()?;
        let revive_point = reader.u8()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            vnum,
            name,
            locale_name,
            mob_type,
            rank,
            battle_type,
            level,
            size,
            gold_min,
            gold_max,
            exp,
            max_hp,
            regen_cycle,
            regen_percent,
            def,
            ai_flag,
            race_flag,
            immune_flag,
            str,
            dex,
            con,
            int_,
            damage_range,
            attack_speed,
            moving_speed,
            aggressive_hp_pct,
            aggressive_sight,
            attack_range,
            enchants,
            resists,
            resurrection_vnum,
            drop_item_vnum,
            mount_capacity,
            on_click_type,
            empire,
            folder,
            dam_multiply,
            summon_vnum,
            drain_sp,
            mob_color,
            polymorph_item_vnum,
            skills,
            berserk_point,
            stone_skin_point,
            god_speed_point,
            death_blow_point,
            revive_point,
        })
    }
}

impl_fixed_codec!(MobTableRecord, "TMobTable", MOB_TABLE_RECORD_WIRE_SIZE);

/// `TRefineMaterial`: one material slot in a refine-table row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefineMaterialRecord {
    /// Material item virtual number (`vnum`).
    pub vnum: u32,
    /// Required material count (`count`).
    pub count: i32,
}

impl RefineMaterialRecord {
    /// Exact packed x86 wire size (`sizeof(TRefineMaterial)`).
    pub const WIRE_SIZE: usize = REFINE_MATERIAL_WIRE_SIZE;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.vnum);
        out.i32(self.count);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TRefineMaterial", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TRefineMaterial");
        Ok(Self {
            vnum: reader.u32()?,
            count: reader.i32()?,
        })
    }
}

impl_fixed_codec!(
    RefineMaterialRecord,
    "TRefineMaterial",
    REFINE_MATERIAL_WIRE_SIZE
);

/// `TRefineTable`: one row in the unconditional refine boot section.
///
/// The active x86 build packs `DWORD id`, `BYTE material_count`, signed
/// 32-bit `cost` and `prob`, and five `TRefineMaterial` values. The nested
/// materials are fixed slots, not a variable-length list; `material_count` is
/// the legacy loader's number of nonzero material vnums.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefineTableRecord {
    /// Refine recipe identifier (`id`).
    pub id: u32,
    /// Number of nonzero material vnums (`material_count`).
    pub material_count: u8,
    /// Refine cost (`cost`).
    pub cost: i32,
    /// Refine probability (`prob`).
    pub prob: i32,
    /// Fixed material slots in legacy order.
    pub materials: [RefineMaterialRecord; REFINE_MATERIAL_MAX_NUM],
}

impl RefineTableRecord {
    /// Exact packed x86 wire size (`sizeof(TRefineTable)`).
    pub const WIRE_SIZE: usize = REFINE_TABLE_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.id);
        out.u8(self.material_count);
        out.i32(self.cost);
        out.i32(self.prob);
        for material in &self.materials {
            out.u32(material.vnum);
            out.i32(material.count);
        }
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TRefineTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TRefineTable");
        let id = reader.u32()?;
        let material_count = reader.u8()?;
        let cost = reader.i32()?;
        let prob = reader.i32()?;
        let mut materials = [RefineMaterialRecord::default(); REFINE_MATERIAL_MAX_NUM];
        for material in &mut materials {
            material.vnum = reader.u32()?;
            material.count = reader.i32()?;
        }
        Ok(Self {
            id,
            material_count,
            cost,
            prob,
            materials,
        })
    }
}

impl_fixed_codec!(RefineTableRecord, "TRefineTable", REFINE_TABLE_WIRE_SIZE);

/// `TItemLimit`: one fixed item limit entry.
///
/// The active x86 `long` value is signed and occupies four bytes. The codec
/// preserves the raw type byte and does not assign enum meaning to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemLimitRecord {
    /// `bType` limit selector.
    pub limit_type: u8,
    /// `lValue` signed x86 limit value.
    pub value: i32,
}

impl ItemLimitRecord {
    /// Exact packed x86 wire size (`sizeof(TItemLimit)`).
    pub const WIRE_SIZE: usize = ITEM_LIMIT_RECORD_WIRE_SIZE;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u8(self.limit_type);
        out.i32(self.value);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TItemLimit", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TItemLimit");
        let record = Self {
            limit_type: reader.u8()?,
            value: reader.i32()?,
        };
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(record)
    }
}

impl_fixed_codec!(ItemLimitRecord, "TItemLimit", ITEM_LIMIT_RECORD_WIRE_SIZE);

/// `TItemApply`: one fixed item apply entry.
///
/// The active x86 `long` value is signed and occupies four bytes. The codec
/// preserves the raw type byte and does not assign enum meaning to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemApplyRecord {
    /// `bType` apply selector.
    pub apply_type: u8,
    /// `lValue` signed x86 apply value.
    pub value: i32,
}

impl ItemApplyRecord {
    /// Exact packed x86 wire size (`sizeof(TItemApply)`).
    pub const WIRE_SIZE: usize = ITEM_APPLY_RECORD_WIRE_SIZE;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u8(self.apply_type);
        out.i32(self.value);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TItemApply", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TItemApply");
        let record = Self {
            apply_type: reader.u8()?,
            value: reader.i32()?,
        };
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(record)
    }
}

impl_fixed_codec!(ItemApplyRecord, "TItemApply", ITEM_APPLY_RECORD_WIRE_SIZE);

/// `TItemTable`: one active x86 item-prototype record.
///
/// The active build uses `#pragma pack(1)`, six extended item sockets, and the
/// 64-bit `ENABLE_REMOVE_LIMIT_GOLD` price fields. The resulting record is
/// exactly [`ITEM_TABLE_RECORD_WIRE_SIZE`] bytes. Names are raw byte arrays:
/// embedded NUL bytes, non-ASCII bytes, and bytes after an early NUL are all
/// retained. This type is a wire codec boundary, not a loader or a semantic
/// validation layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemTableRecord {
    /// `SEntityTable::dwVnum`.
    pub vnum: u32,
    /// `dwVnumRange`.
    pub vnum_range: u32,
    /// Raw `szName[ITEM_NAME_MAX_LEN + 1]` bytes.
    pub name: [u8; ITEM_NAME_BYTES],
    /// Raw `szLocaleName[ITEM_NAME_MAX_LEN + 1]` bytes.
    pub locale_name: [u8; ITEM_NAME_BYTES],
    /// `bType`.
    pub item_type: u8,
    /// `bSubType`.
    pub sub_type: u8,
    /// `bWeight`.
    pub weight: u8,
    /// `bSize`.
    pub size: u8,
    /// `dwAntiFlags`.
    pub anti_flags: u32,
    /// `dwFlags`.
    pub flags: u32,
    /// `dwWearFlags`.
    pub wear_flags: u32,
    /// `dwImmuneFlag`.
    pub immune_flag: u32,
    /// Active-profile 64-bit `dwGold`.
    pub gold: u64,
    /// Active-profile 64-bit `dwShopBuyPrice`.
    pub shop_buy_price: u64,
    /// Fixed `aLimits` slots in legacy order.
    pub limits: [ItemLimitRecord; ITEM_LIMIT_MAX_NUM],
    /// Fixed `aApplies` slots in legacy order.
    pub applies: [ItemApplyRecord; ITEM_APPLY_MAX_NUM],
    /// Signed x86 `alValues` slots in legacy order.
    pub values: [i32; ITEM_VALUES_MAX_NUM],
    /// Signed x86 `alSockets` slots in legacy order.
    pub sockets: [i32; ITEM_SOCKET_MAX_NUM],
    /// `dwRefinedVnum`.
    pub refined_vnum: u32,
    /// `wRefineSet`.
    pub refine_set: u16,
    /// `bAlterToMagicItemPct`.
    pub alter_to_magic_item_pct: u8,
    /// `bSpecular`.
    pub specular: u8,
    /// `bGainSocketPct`.
    pub gain_socket_pct: u8,
    /// `sAddonType`.
    pub addon_type: i16,
    /// `cLimitRealTimeFirstUseIndex` represented as signed raw byte.
    pub limit_real_time_first_use_index: i8,
    /// `cLimitTimerBasedOnWearIndex` represented as signed raw byte.
    pub limit_timer_based_on_wear_index: i8,
}

impl Default for ItemTableRecord {
    fn default() -> Self {
        Self {
            vnum: 0,
            vnum_range: 0,
            name: [0; ITEM_NAME_BYTES],
            locale_name: [0; ITEM_NAME_BYTES],
            item_type: 0,
            sub_type: 0,
            weight: 0,
            size: 0,
            anti_flags: 0,
            flags: 0,
            wear_flags: 0,
            immune_flag: 0,
            gold: 0,
            shop_buy_price: 0,
            limits: [ItemLimitRecord::default(); ITEM_LIMIT_MAX_NUM],
            applies: [ItemApplyRecord::default(); ITEM_APPLY_MAX_NUM],
            values: [0; ITEM_VALUES_MAX_NUM],
            sockets: [0; ITEM_SOCKET_MAX_NUM],
            refined_vnum: 0,
            refine_set: 0,
            alter_to_magic_item_pct: 0,
            specular: 0,
            gain_socket_pct: 0,
            addon_type: 0,
            limit_real_time_first_use_index: 0,
            limit_timer_based_on_wear_index: 0,
        }
    }
}

impl ItemTableRecord {
    /// Exact packed x86 wire size (`sizeof(TItemTable)`).
    pub const WIRE_SIZE: usize = ITEM_TABLE_RECORD_WIRE_SIZE;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.u32(self.vnum);
        out.u32(self.vnum_range);
        out.bytes(&self.name);
        out.bytes(&self.locale_name);
        out.u8(self.item_type);
        out.u8(self.sub_type);
        out.u8(self.weight);
        out.u8(self.size);
        out.u32(self.anti_flags);
        out.u32(self.flags);
        out.u32(self.wear_flags);
        out.u32(self.immune_flag);
        out.u64(self.gold);
        out.u64(self.shop_buy_price);
        for limit in &self.limits {
            out.u8(limit.limit_type);
            out.i32(limit.value);
        }
        for apply in &self.applies {
            out.u8(apply.apply_type);
            out.i32(apply.value);
        }
        for value in self.values {
            out.i32(value);
        }
        for socket in self.sockets {
            out.i32(socket);
        }
        out.u32(self.refined_vnum);
        out.u16(self.refine_set);
        out.u8(self.alter_to_magic_item_pct);
        out.u8(self.specular);
        out.u8(self.gain_socket_pct);
        out.i16(self.addon_type);
        out.bytes(&self.limit_real_time_first_use_index.to_le_bytes());
        out.bytes(&self.limit_timer_based_on_wear_index.to_le_bytes());
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TItemTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TItemTable");
        let vnum = reader.u32()?;
        let vnum_range = reader.u32()?;
        let name = reader.array()?;
        let locale_name = reader.array()?;
        let item_type = reader.u8()?;
        let sub_type = reader.u8()?;
        let weight = reader.u8()?;
        let size = reader.u8()?;
        let anti_flags = reader.u32()?;
        let flags = reader.u32()?;
        let wear_flags = reader.u32()?;
        let immune_flag = reader.u32()?;
        let gold = reader.u64()?;
        let shop_buy_price = reader.u64()?;
        let mut limits = [ItemLimitRecord::default(); ITEM_LIMIT_MAX_NUM];
        for limit in &mut limits {
            limit.limit_type = reader.u8()?;
            limit.value = reader.i32()?;
        }
        let mut applies = [ItemApplyRecord::default(); ITEM_APPLY_MAX_NUM];
        for apply in &mut applies {
            apply.apply_type = reader.u8()?;
            apply.value = reader.i32()?;
        }
        let mut values = [0_i32; ITEM_VALUES_MAX_NUM];
        for value in &mut values {
            *value = reader.i32()?;
        }
        let mut sockets = [0_i32; ITEM_SOCKET_MAX_NUM];
        for socket in &mut sockets {
            *socket = reader.i32()?;
        }
        let refined_vnum = reader.u32()?;
        let refine_set = reader.u16()?;
        let alter_to_magic_item_pct = reader.u8()?;
        let specular = reader.u8()?;
        let gain_socket_pct = reader.u8()?;
        let addon_type = reader.i16()?;
        let limit_real_time_first_use_index = i8::from_le_bytes([reader.u8()?]);
        let limit_timer_based_on_wear_index = i8::from_le_bytes([reader.u8()?]);
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            vnum,
            vnum_range,
            name,
            locale_name,
            item_type,
            sub_type,
            weight,
            size,
            anti_flags,
            flags,
            wear_flags,
            immune_flag,
            gold,
            shop_buy_price,
            limits,
            applies,
            values,
            sockets,
            refined_vnum,
            refine_set,
            alter_to_magic_item_pct,
            specular,
            gain_socket_pct,
            addon_type,
            limit_real_time_first_use_index,
            limit_timer_based_on_wear_index,
        })
    }
}

impl_fixed_codec!(ItemTableRecord, "TItemTable", ITEM_TABLE_RECORD_WIRE_SIZE);

/// `TItemAttrTable`: one packed item-attribute rule.
///
/// The active build enables glove attributes, so `bMaxLevelBySet` has ten
/// slots. `szApply` is preserved as raw bytes, including any bytes after the
/// first NUL. `long` values are decoded as signed little-endian `i32` values
/// for the active x86 target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemAttrRecord {
    /// `szApply[APPLY_NAME_MAX_LEN + 1]`.
    pub apply: [u8; APPLY_NAME_MAX_LEN + 1],
    /// `dwApplyIndex`.
    pub apply_index: u32,
    /// `dwProb`.
    pub prob: u32,
    /// `lValues[ITEM_ATTRIBUTE_MAX_LEVEL]` (x86 `long`).
    pub values: [i32; ITEM_ATTRIBUTE_MAX_LEVEL],
    /// `bMaxLevelBySet[ATTRIBUTE_SET_MAX_NUM]`, in legacy set order.
    pub max_level_by_set: [u8; ATTRIBUTE_SET_MAX_NUM],
}

impl Default for ItemAttrRecord {
    fn default() -> Self {
        Self {
            apply: [0; APPLY_NAME_MAX_LEN + 1],
            apply_index: 0,
            prob: 0,
            values: [0; ITEM_ATTRIBUTE_MAX_LEVEL],
            max_level_by_set: [0; ATTRIBUTE_SET_MAX_NUM],
        }
    }
}

impl ItemAttrRecord {
    /// Exact packed x86 wire size (`sizeof(TItemAttrTable)`).
    pub const WIRE_SIZE: usize = ITEM_ATTR_RECORD_WIRE_SIZE;

    fn encode_wire(&self) -> Vec<u8> {
        let mut out = Writer::with_capacity(Self::WIRE_SIZE);
        out.bytes(&self.apply);
        out.u32(self.apply_index);
        out.u32(self.prob);
        for value in self.values {
            out.i32(value);
        }
        out.bytes(&self.max_level_by_set);
        debug_assert_eq!(out.bytes.len(), Self::WIRE_SIZE);
        out.bytes
    }

    fn decode_wire(data: &[u8]) -> DbRecordResult<Self> {
        check_exact("TItemAttrTable", data, Self::WIRE_SIZE)?;
        let mut reader = Reader::new(data, "TItemAttrTable");
        let apply = reader.array()?;
        let apply_index = reader.u32()?;
        let prob = reader.u32()?;
        let mut values = [0_i32; ITEM_ATTRIBUTE_MAX_LEVEL];
        for value in &mut values {
            *value = reader.i32()?;
        }
        let max_level_by_set = reader.array()?;
        debug_assert_eq!(reader.offset, Self::WIRE_SIZE);
        Ok(Self {
            apply,
            apply_index,
            prob,
            values,
            max_level_by_set,
        })
    }
}

impl_fixed_codec!(ItemAttrRecord, "TItemAttrTable", ITEM_ATTR_RECORD_WIRE_SIZE);

/// Encode the `HEADER_DG_QUEST_LOAD` payload: count followed by records.
///
/// # Errors
///
/// Returns [`DbRecordError::TooManyQuestRecords`] when the input exceeds
/// [`MAX_QUEST_RECORDS`].
pub fn encode_quest_load(records: &[QuestRecord]) -> DbRecordResult<Vec<u8>> {
    encode_quest_load_with_limit(records, MAX_QUEST_RECORDS)
}

/// Encode a quest-load payload with an explicit row-count safety bound.
///
/// # Errors
///
/// Returns [`DbRecordError::TooManyQuestRecords`] when the input exceeds
/// `maximum` or its count cannot be represented on the wire.
pub fn encode_quest_load_with_limit(
    records: &[QuestRecord],
    maximum: usize,
) -> DbRecordResult<Vec<u8>> {
    if records.len() > maximum {
        return Err(DbRecordError::TooManyQuestRecords {
            count: u32::try_from(records.len()).unwrap_or(u32::MAX),
            maximum,
        });
    }
    let count = u32::try_from(records.len()).map_err(|_| DbRecordError::TooManyQuestRecords {
        count: u32::MAX,
        maximum,
    })?;
    let payload_size = QUEST_COUNT_WIRE_SIZE
        .checked_add(
            records
                .len()
                .checked_mul(QuestRecord::WIRE_SIZE)
                .ok_or(DbRecordError::TooManyQuestRecords { count, maximum })?,
        )
        .ok_or(DbRecordError::TooManyQuestRecords { count, maximum })?;
    let mut out = Writer::with_capacity(payload_size);
    out.u32(count);
    for record in records {
        let encoded = record.encode();
        debug_assert_eq!(encoded.len(), QuestRecord::WIRE_SIZE);
        out.bytes(&encoded);
    }
    debug_assert_eq!(out.bytes.len(), payload_size);
    Ok(out.bytes)
}

/// Decode the `HEADER_DG_QUEST_LOAD` payload using [`MAX_QUEST_RECORDS`].
///
/// # Errors
///
/// Returns an error for a truncated or overlong payload, or when the declared
/// count exceeds [`MAX_QUEST_RECORDS`].
pub fn decode_quest_load(data: &[u8]) -> DbRecordResult<Vec<QuestRecord>> {
    decode_quest_load_with_limit(data, MAX_QUEST_RECORDS)
}

/// Decode a quest-load payload with an explicit row-count safety bound.
///
/// # Errors
///
/// Returns an error for a truncated or overlong payload, or when the declared
/// count exceeds `maximum`.
pub fn decode_quest_load_with_limit(
    data: &[u8],
    maximum: usize,
) -> DbRecordResult<Vec<QuestRecord>> {
    if data.len() < QUEST_COUNT_WIRE_SIZE {
        return Err(DbRecordError::Truncated {
            record: "HEADER_DG_QUEST_LOAD",
            needed: QUEST_COUNT_WIRE_SIZE,
            available: data.len(),
        });
    }
    let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if count as usize > maximum {
        return Err(DbRecordError::TooManyQuestRecords { count, maximum });
    }
    let records_size = (count as usize)
        .checked_mul(QuestRecord::WIRE_SIZE)
        .ok_or(DbRecordError::TooManyQuestRecords { count, maximum })?;
    let expected = QUEST_COUNT_WIRE_SIZE
        .checked_add(records_size)
        .ok_or(DbRecordError::TooManyQuestRecords { count, maximum })?;
    if data.len() < expected {
        return Err(DbRecordError::Truncated {
            record: "HEADER_DG_QUEST_LOAD",
            needed: expected,
            available: data.len(),
        });
    }
    if data.len() > expected {
        return Err(DbRecordError::LengthMismatch {
            record: "HEADER_DG_QUEST_LOAD",
            expected,
            actual: data.len(),
        });
    }

    let mut reader = Reader::new(&data[QUEST_COUNT_WIRE_SIZE..], "TQuestTable");
    let mut records = Vec::with_capacity(count as usize);
    for _ in 0..count {
        records.push(QuestRecord {
            pid: reader.u32()?,
            name: reader.array()?,
            state: reader.array()?,
            value: reader.i32()?,
        });
    }
    debug_assert_eq!(reader.offset, records_size);
    Ok(records)
}

/// Encode a login-by-key account result.
pub fn encode_login_account(record: &LoginAccountRecord) -> Vec<u8> {
    record.encode()
}

/// Decode a login-by-key account result.
///
/// # Errors
///
/// Returns [`DbRecordError::Truncated`] or
/// [`DbRecordError::LengthMismatch`] for an invalid fixed-width payload.
pub fn decode_login_account(data: &[u8]) -> DbRecordResult<LoginAccountRecord> {
    LoginAccountRecord::decode(data)
}

/// Encode a player-load result.
pub fn encode_player_result(record: &PlayerResultRecord) -> Vec<u8> {
    record.encode()
}

/// Decode a player-load result.
///
/// # Errors
///
/// Returns [`DbRecordError::Truncated`] or
/// [`DbRecordError::LengthMismatch`] for an invalid fixed-width payload.
pub fn decode_player_result(data: &[u8]) -> DbRecordResult<PlayerResultRecord> {
    PlayerResultRecord::decode(data)
}

/// Encode a `HEADER_DG_AFFECT_LOAD` response payload.
///
/// # Errors
///
/// Returns a distinct count-conversion, size-overflow, or allocation error.
pub fn encode_affect_load(response: &AffectLoadResponse) -> DbRecordResult<Vec<u8>> {
    response.encode()
}

/// Encode an affect-load response with an explicit caller-selected row limit.
///
/// # Errors
///
/// Returns [`DbRecordError::TooManyAffectRecords`] or a distinct count, size,
/// or allocation error.
pub fn encode_affect_load_with_limit(
    response: &AffectLoadResponse,
    maximum: usize,
) -> DbRecordResult<Vec<u8>> {
    response.encode_with_limit(maximum)
}

/// Decode a `HEADER_DG_AFFECT_LOAD` response payload without a source cap.
///
/// # Errors
///
/// Returns an exact-length, count-conversion, size-overflow, or allocation
/// error.
pub fn decode_affect_load(data: &[u8]) -> DbRecordResult<AffectLoadResponse> {
    AffectLoadResponse::decode(data)
}

/// Decode an affect-load response with an explicit caller-selected row limit.
///
/// # Errors
///
/// Returns [`DbRecordError::TooManyAffectRecords`], an exact-length error, or
/// a distinct count-conversion, size-overflow, or allocation error.
pub fn decode_affect_load_with_limit(
    data: &[u8],
    maximum: usize,
) -> DbRecordResult<AffectLoadResponse> {
    AffectLoadResponse::decode_with_limit(data, maximum)
}

/// Encode a `HEADER_GD_ADD_AFFECT` request payload.
pub fn encode_add_affect_request(record: &AddAffectRequest) -> Vec<u8> {
    record.encode()
}

/// Decode a `HEADER_GD_ADD_AFFECT` request payload.
///
/// # Errors
///
/// Returns [`DbRecordError::Truncated`] for a short input or
/// [`DbRecordError::LengthMismatch`] for an overlong input.
pub fn decode_add_affect_request(data: &[u8]) -> DbRecordResult<AddAffectRequest> {
    AddAffectRequest::decode(data)
}

/// Encode a `HEADER_GD_REMOVE_AFFECT` request payload.
pub fn encode_remove_affect_request(record: &RemoveAffectRequest) -> Vec<u8> {
    record.encode()
}

/// Decode a `HEADER_GD_REMOVE_AFFECT` request payload.
///
/// # Errors
///
/// Returns [`DbRecordError::Truncated`] or
/// [`DbRecordError::LengthMismatch`] for an invalid fixed-width payload.
pub fn decode_remove_affect_request(data: &[u8]) -> DbRecordResult<RemoveAffectRequest> {
    RemoveAffectRequest::decode(data)
}

/// Encode one quest record.
pub fn encode_quest_record(record: &QuestRecord) -> Vec<u8> {
    record.encode()
}

/// Decode one quest record.
///
/// # Errors
///
/// Returns [`DbRecordError::Truncated`] or
/// [`DbRecordError::LengthMismatch`] for an invalid fixed-width payload.
pub fn decode_quest_record(data: &[u8]) -> DbRecordResult<QuestRecord> {
    QuestRecord::decode(data)
}

/// C++-style aliases for the records represented by this module.
pub type TSimplePlayer = SimplePlayerRecord;
/// C++-style alias for `TAccountTable`.
pub type TAccountTable = LoginAccountRecord;
/// C++-style alias for `TPacketGDLoginByKey`.
pub type TPacketGDLoginByKey = LoginByKeyRequest;
/// C++-style alias for `TPacketDGLoginAlready`.
pub type TPacketDGLoginAlready = LoginAlreadyRecord;
/// C++-style alias for `TPlayerLoadPacket`.
pub type TPlayerLoadPacket = PlayerLoadRequest;
/// C++-style alias for `TPlayerSkill`.
pub type TPlayerSkill = PlayerSkillRecord;
/// C++-style alias for `TQuickslot`.
pub type TQuickslot = QuickslotRecord;
/// C++-style alias for `TPlayerFishEventSlot`.
pub type TPlayerFishEventSlot = FishEventSlotRecord;
/// C++-style alias for `THorseInfo`.
pub type THorseInfo = HorseInfoRecord;
/// C++-style alias for `TPlayerBattlePass`.
pub type TPlayerBattlePass = BattlePassRecord;
/// C++-style alias for `TPlayerTable`.
pub type TPlayerTable = PlayerResultRecord;
/// C++-style alias for `TQuestTable`.
pub type TQuestTable = QuestRecord;
/// C++-style alias for `TMarketItemPrice`.
pub type TMarketItemPrice = MarketItemPriceRecord;
/// C++-style alias for `TEventTable`.
pub type TEventTable = EventTableRecord;
/// Semantic alias for a `TSkillTable` row.
pub type SkillRecord = SkillTableRecord;
/// C++-style alias for `TSkillTable`.
pub type TSkillTable = SkillTableRecord;
/// C++-style alias for `TMobSkillLevel`.
pub type TMobSkillLevel = MobSkillRecord;
/// Semantic alias for a `TMobSkillLevel` row.
pub type MobSkillLevelRecord = MobSkillRecord;
/// C++-style alias for `TMobTable`.
pub type TMobTable = MobTableRecord;
/// C++-style alias for `TRefineMaterial`.
pub type TRefineMaterial = RefineMaterialRecord;
/// C++-style alias for `TRefineTable`.
pub type TRefineTable = RefineTableRecord;
/// C++-style alias for `TItemLimit`.
pub type TItemLimit = ItemLimitRecord;
/// C++-style alias for `TItemApply`.
pub type TItemApply = ItemApplyRecord;
/// C++-style alias for `TItemTable`.
pub type TItemTable = ItemTableRecord;
/// C++-style alias for `TItemAttrTable`.
pub type TItemAttrTable = ItemAttrRecord;
/// Record-oriented alias for [`ObjectMaterial`].
pub type ObjectMaterialRecord = ObjectMaterial;
/// Table-oriented alias for [`ObjectProtoRecord`].
pub type ObjectProtoTableRecord = ObjectProtoRecord;
/// C++-style alias for `building::TObjectMaterial`.
pub type TObjectMaterial = ObjectMaterial;
/// C++-style alias for `building::TObjectProto`.
pub type TObjectProto = ObjectProtoRecord;
/// C++-style alias for `building::TObject`.
pub type TObject = ObjectRecord;
/// C++-style alias for the horse-name acknowledgement packet.
pub type TPacketUpdateHorseName = HorseNameRecord;
/// Semantic alias for a horse-name acknowledgement record.
pub type HorseNameAck = HorseNameRecord;
/// C++-style alias for `TPacketAffectElement`.
pub type TPacketAffectElement = AffectElementRecord;
/// Semantic alias for the `HEADER_DG_AFFECT_LOAD` response.
pub type AffectLoadResult = AffectLoadResponse;
/// C++-style alias for `TPacketGDAddAffect`.
pub type TPacketGDAddAffect = AddAffectRequest;
/// C++-style alias for `TPacketChangeChannel`.
pub type TPacketChangeChannel = ChannelChangeRequest;
/// C++-style alias for `TPacketReturnChannel`.
pub type TPacketReturnChannel = ChannelResultRecord;
/// C++-style alias for `TPacketGDRemoveAffect`.
pub type TPacketGDRemoveAffect = RemoveAffectRequest;
/// Semantic alias for the channel-change request.
pub type FindChannelRequest = ChannelChangeRequest;
/// Semantic alias for the channel lookup result.
pub type ChannelResult = ChannelResultRecord;
/// Semantic alias for the `LOGIN_BY_KEY` account result.
pub type LoginByKeyResult = LoginAccountRecord;
/// Semantic alias for the `PLAYER_LOAD` result.
pub type PlayerLoadResult = PlayerResultRecord;

#[cfg(test)]
mod tests {
    use super::*;

    fn put_c_string<const N: usize>(target: &mut [u8; N], value: &str) {
        let bytes = value.as_bytes();
        assert!(bytes.len() < N);
        target[..bytes.len()].copy_from_slice(bytes);
    }

    #[allow(clippy::field_reassign_with_default)]
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    fn account_fixture() -> LoginAccountRecord {
        let mut account = LoginAccountRecord::default();
        account.id = 0x0102_0304;
        put_c_string(&mut account.login, "alice");
        put_c_string(&mut account.passwd, "secret");
        put_c_string(&mut account.social_id, "social-1");
        put_c_string(&mut account.status, "OK");
        account.empire = 2;
        account.language = 7;
        for (index, player) in account.players.iter_mut().enumerate() {
            player.id = 0x1000 + index as u32;
            put_c_string(&mut player.name, &format!("hero-{index}"));
            player.job = index as u8 + 1;
            player.level = 20 + index as u8;
            player.play_minutes = 300 + index as u32;
            player.st = 11 + index as u8;
            player.ht = 12 + index as u8;
            player.dx = 13 + index as u8;
            player.iq = 14 + index as u8;
            player.main_part = 100 + index as u16;
            player.change_name = 1;
            player.hair_part = 200 + index as u16;
            player.sash_part = 300 + index as u16;
            player.dummy = [0xa0 + index as u8; 4];
            player.x = -1000 + index as i32;
            player.y = 2000 + index as i32;
            player.addr = 0x7f00_0001 + index as i32;
            player.port = 7000 + index as u16;
            player.skill_group = index as u8;
            player.conqueror_level = 3 + index as u8;
            player.sungma_str = 4 + index as u8;
            player.sungma_hp = 5 + index as u8;
            player.sungma_move = 6 + index as u8;
            player.sungma_immune = 7 + index as u8;
        }
        account
    }

    #[allow(clippy::field_reassign_with_default)]
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    fn player_fixture() -> PlayerResultRecord {
        let mut player = PlayerResultRecord::default();
        player.id = 0x1122_3344;
        put_c_string(&mut player.name, "player-one");
        put_c_string(&mut player.ip, "127.0.0.1");
        player.job = 2;
        player.voice = 9;
        player.level = 42;
        player.level_step = 3;
        player.st = -1;
        player.ht = 2;
        player.dx = -3;
        player.iq = 4;
        player.exp = 0x1234_5678;
        player.gold = 0x0102_0304_0506_0708;
        player.gaya = -99;
        player.dir = 6;
        player.x = -10;
        player.y = 20;
        player.z = 30;
        player.map_index = 3;
        player.exit_x = 11;
        player.exit_y = 12;
        player.exit_map_index = 4;
        player.hp = 500;
        player.sp = 300;
        player.random_hp = -7;
        player.random_sp = 8;
        player.playtime = 1234;
        player.stat_point = 5;
        player.skill_point = 6;
        player.sub_skill_point = 7;
        player.horse_skill_point = 8;
        for (index, skill) in player.skills.iter_mut().enumerate() {
            *skill = PlayerSkillRecord {
                master_type: (index % 251) as u8,
                level: (index % 40) as u8,
                next_read: 1_700_000_000 + index as i32,
            };
        }
        for (index, slot) in player.quickslot.iter_mut().enumerate() {
            *slot = QuickslotRecord {
                slot_type: (index % 4) as u8,
                pos: (index % 36) as u8,
            };
        }
        player.part_base = 5;
        player.parts = [11, 22, 33, 44, 55, 66];
        player.stamina = 77;
        player.skill_group = 1;
        player.alignment = -88;
        player.stat_reset_count = 9;
        player.horse = HorseInfoRecord {
            level: 12,
            riding: 1,
            stamina: 13,
            health: 14,
            health_drop_time: 0x5566_7788,
        };
        player.logoff_interval = 999;
        player.premium_times = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        player.envanter = 10;
        player.fish_event_use_count = 11;
        for (index, slot) in player.fish_slots.iter_mut().enumerate() {
            *slot = FishEventSlotRecord {
                is_main: (index % 2) as u8,
                shape: (index % 24) as u8,
            };
        }
        player.premium = 12;
        player.premium_time = 13;
        player.secured = 1;
        player.secured_password = 123_456;
        player.biologist_state = 14;
        player.biologist_items_taken = 15;
        player.biologist_completed = 16;
        player.conqueror_level = 17;
        player.conqueror_level_step = 18;
        player.sungma_str = 19;
        player.sungma_hp = 20;
        player.sungma_move = 21;
        player.sungma_immune = 22;
        player.conqueror_exp = 23;
        player.conqueror_point = 24;
        for (index, mission) in player.battle_pass.iter_mut().enumerate() {
            *mission = BattlePassRecord {
                mission_id: 100 + index as u16,
                progress: 200 + index as u32,
                battle_pass_type: (index % 5) as u8,
                end_time: 300 + index as u32,
            };
        }
        player.private_shop_unlocked_slot = 0xabcd;
        player
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn shop_records_use_exact_packed_offsets_and_round_trip() {
        assert_eq!(SHOP_HOST_ITEM_MAX_NUM, 40);
        assert_eq!(SHOP_SIGN_MAX_LEN, 32);
        assert_eq!(SHOP_ITEM_RECORD_WIRE_SIZE, 68);
        assert_eq!(SHOP_TABLE_RECORD_WIRE_SIZE, 2_762);
        assert_eq!(ShopItemRecord::WIRE_SIZE, 68);
        assert_eq!(ShopTableRecord::WIRE_SIZE, 2_762);
        assert_eq!(ShopItemPosition::WIRE_SIZE, 3);
        assert_eq!(ShopItemAttribute::WIRE_SIZE, 3);
        assert_eq!(
            ShopItemPosition::default().window_type,
            SHOP_ITEM_DEFAULT_WINDOW_TYPE
        );
        assert_eq!(ShopItemPosition::default().cell, SHOP_ITEM_DEFAULT_CELL);
        let default_item = ShopItemRecord::zeroed();
        assert_eq!(default_item.pos.window_type, SHOP_ITEM_DEFAULT_WINDOW_TYPE);
        assert_eq!(default_item.pos.cell, SHOP_ITEM_DEFAULT_CELL);
        assert_eq!(default_item.price_type, SHOP_PRICE_TYPE_GOLD);

        let mut attributes = [ShopItemAttribute::default(); SHOP_ITEM_ATTRIBUTE_MAX_NUM];
        for (index, attribute) in attributes.iter_mut().enumerate() {
            *attribute = ShopItemAttribute::new(
                u8::try_from(index + 1).unwrap(),
                -i16::try_from(index).unwrap() - 1,
            );
        }
        let item = ShopItemRecord {
            vnum: 0x1122_3344,
            count: 0x5566,
            pos: ShopItemPosition::new(0x77, 0x8899),
            price: 0x0102_0304_0506_0708,
            display_pos: 0xaa,
            sockets: [i32::MIN, -2, -1, 0, 1, i32::MAX],
            attrs: attributes,
            price_type: SHOP_PRICE_TYPE_GOLD,
            price_vnum: 0xdead_beef,
        };
        let item_bytes = item.encode();
        assert_eq!(item_bytes.len(), 68);
        assert_eq!(
            &item_bytes[SHOP_ITEM_VNUM_OFFSET..SHOP_ITEM_VNUM_OFFSET + 4],
            &item.vnum.to_le_bytes()
        );
        assert_eq!(
            &item_bytes[SHOP_ITEM_COUNT_OFFSET..SHOP_ITEM_COUNT_OFFSET + 2],
            &item.count.to_le_bytes()
        );
        assert_eq!(item_bytes[SHOP_ITEM_POSITION_OFFSET], item.pos.window_type);
        assert_eq!(
            &item_bytes[SHOP_ITEM_POSITION_OFFSET + 1..SHOP_ITEM_POSITION_OFFSET + 3],
            &item.pos.cell.to_le_bytes()
        );
        assert_eq!(
            &item_bytes[SHOP_ITEM_PRICE_OFFSET..SHOP_ITEM_PRICE_OFFSET + 8],
            &item.price.to_le_bytes()
        );
        assert_eq!(item_bytes[SHOP_ITEM_DISPLAY_POS_OFFSET], item.display_pos);
        assert_eq!(
            &item_bytes[SHOP_ITEM_PRICE_TYPE_OFFSET..=SHOP_ITEM_PRICE_TYPE_OFFSET],
            &[SHOP_PRICE_TYPE_GOLD]
        );
        assert_eq!(
            &item_bytes[SHOP_ITEM_PRICE_VNUM_OFFSET..SHOP_ITEM_PRICE_VNUM_OFFSET + 4],
            &item.price_vnum.to_le_bytes()
        );
        assert_eq!(ShopItemRecord::decode(&item_bytes).unwrap(), item);

        let mut table = ShopTableRecord::zeroed_base_shop();
        table.vnum = 0x0102_0304;
        table.npc_vnum = 0xa1b2_c3d4;
        table.item_count = 1;
        table.items[0] = item;
        table.shop_name[..6].copy_from_slice(b"SHOPEX");
        let table_bytes = table.encode();
        assert_eq!(table_bytes.len(), 2_762);
        assert_eq!(
            &table_bytes[SHOP_TABLE_VNUM_OFFSET..SHOP_TABLE_VNUM_OFFSET + 4],
            &table.vnum.to_le_bytes()
        );
        assert_eq!(
            &table_bytes[SHOP_TABLE_NPC_VNUM_OFFSET..SHOP_TABLE_NPC_VNUM_OFFSET + 4],
            &table.npc_vnum.to_le_bytes()
        );
        assert_eq!(table_bytes[SHOP_TABLE_ITEM_COUNT_OFFSET], 1);
        assert_eq!(
            &table_bytes[SHOP_TABLE_ITEMS_OFFSET..SHOP_TABLE_ITEMS_OFFSET + 68],
            &item_bytes
        );
        assert_eq!(
            &table_bytes[SHOP_TABLE_SHOP_NAME_OFFSET..SHOP_TABLE_SHOP_NAME_OFFSET + 6],
            b"SHOPEX"
        );
        assert_eq!(ShopTableRecord::decode(&table_bytes).unwrap(), table);
        assert!(table.items.iter().skip(1).all(|slot| slot.price_type == 1));
        let renewal = ShopTableRecord::zeroed_renewal_shop();
        assert!(renewal.items.iter().all(|slot| {
            slot.pos == ShopItemPosition::new(0, 0)
                && slot.price_type == 0
                && slot.vnum == 0
                && slot.count == 0
                && slot.price == 0
                && slot.display_pos == 0
                && slot.price_vnum == 0
                && slot.sockets.iter().all(|value| *value == 0)
                && slot
                    .attrs
                    .iter()
                    .all(|attr| *attr == ShopItemAttribute::new(0, 0))
        }));
    }

    #[test]
    fn shop_records_reject_truncation_and_trailing_bytes() {
        let item_bytes = ShopItemRecord::zeroed().encode();
        for available in 0..SHOP_ITEM_RECORD_WIRE_SIZE {
            assert!(matches!(
                ShopItemRecord::decode(&item_bytes[..available]),
                Err(DbRecordError::Truncated { .. })
            ));
        }
        let mut item_trailing = item_bytes;
        item_trailing.push(0);
        assert!(matches!(
            ShopItemRecord::decode(&item_trailing),
            Err(DbRecordError::LengthMismatch { .. })
        ));

        let table_bytes = ShopTableRecord::default().encode();
        for available in 0..SHOP_TABLE_RECORD_WIRE_SIZE {
            assert!(matches!(
                ShopTableRecord::decode(&table_bytes[..available]),
                Err(DbRecordError::Truncated { .. })
            ));
        }
        let mut table_trailing = table_bytes;
        table_trailing.push(0);
        assert!(matches!(
            ShopTableRecord::decode(&table_trailing),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn exact_x86_wire_sizes_are_stable() {
        assert_eq!(SimplePlayerRecord::WIRE_SIZE, 70);
        assert_eq!(LoginAccountRecord::WIRE_SIZE, 362);
        assert_eq!(LoginAlreadyRecord::WIRE_SIZE, 31);
        assert_eq!(LoginByKeyRequest::WIRE_SIZE, 67);
        assert_eq!(HEADER_DG_LOGIN_SUCCESS, 30);
        assert_eq!(HEADER_DG_LOGIN_NOT_EXIST, 31);
        assert_eq!(HEADER_DG_LOGIN_WRONG_PASSWD, 33);
        assert_eq!(HEADER_DG_LOGIN_ALREADY, 34);
        assert_eq!(PlayerLoadRequest::WIRE_SIZE, 9);
        assert_eq!(HEADER_GD_PLAYER_LOAD, 3);
        assert_eq!(HEADER_DG_PLAYER_LOAD_SUCCESS, 35);
        assert_eq!(HEADER_DG_PLAYER_LOAD_FAILED, 36);
        assert_eq!(PlayerResultRecord::WIRE_SIZE, 2007);
        assert_eq!(QuestRecord::WIRE_SIZE, 106);
        assert_eq!(SkillTableRecord::WIRE_SIZE, SKILL_TABLE_RECORD_WIRE_SIZE);
        assert_eq!(SkillTableRecord::WIRE_SIZE, 1_475);
        assert_eq!(SkillTableRecord::default().encode().len(), 1_475);
        assert_eq!(MarketItemPriceRecord::WIRE_SIZE, 16);
        assert_eq!(EventTableRecord::WIRE_SIZE, 85);
        assert_eq!(RefineMaterialRecord::WIRE_SIZE, 8);
        assert_eq!(RefineTableRecord::WIRE_SIZE, 53);
        assert_eq!(ItemAttrRecord::WIRE_SIZE, ITEM_ATTR_RECORD_WIRE_SIZE);
        assert_eq!(ItemAttrRecord::WIRE_SIZE, 71);
        assert_eq!(ItemAttrRecord::default().encode().len(), 71);
        assert_eq!(LandRecord::WIRE_SIZE, LAND_RECORD_WIRE_SIZE);
        assert_eq!(LandRecord::WIRE_SIZE, 36);
        assert_eq!(LandRecord::default().encode().len(), 36);
        assert_eq!(ObjectMaterial::WIRE_SIZE, 8);
        assert_eq!(ObjectProtoRecord::WIRE_SIZE, 96);
        assert_eq!(ObjectProtoRecord::default().encode().len(), 96);
        assert_eq!(ObjectRecord::WIRE_SIZE, OBJECT_RECORD_WIRE_SIZE);
        assert_eq!(ObjectRecord::WIRE_SIZE, 40);
        assert_eq!(ObjectRecord::default().encode().len(), 40);
        assert_eq!(HorseNameRequest::WIRE_SIZE, 4);
        assert_eq!(HorseNameRecord::WIRE_SIZE, 29);
        assert_eq!(ChannelChangeRequest::WIRE_SIZE, 8);
        assert_eq!(ChannelResultRecord::WIRE_SIZE, 6);
        assert_eq!(account_fixture().encode().len(), 362);
        assert_eq!(player_fixture().encode().len(), 2007);
        assert_eq!(QuestRecord::default().encode().len(), 106);
    }

    #[test]
    fn land_record_round_trips_with_exact_packed_offsets_and_padding() {
        let record = LandRecord {
            id: 0x1122_3344,
            map_index: 2,
            x: 0x0102_0304,
            y: -3,
            width: 480,
            height: -1,
            guild_id: 0xa1b2_c3d4,
            guild_level_limit: 7,
            price: 0xdead_beef,
        };
        let encoded = record.encode();
        assert_eq!(
            encoded,
            vec![
                0x44, 0x33, 0x22, 0x11, // id
                0x02, 0x00, 0x00, 0x00, // map_index
                0x04, 0x03, 0x02, 0x01, // x
                0xfd, 0xff, 0xff, 0xff, // y
                0xe0, 0x01, 0x00, 0x00, // width
                0xff, 0xff, 0xff, 0xff, // height
                0xd4, 0xc3, 0xb2, 0xa1, // guild_id
                0x07, // guild_level_limit
                0x00, 0x00, 0x00, // padding
                0xef, 0xbe, 0xad, 0xde, // price
            ]
        );
        assert_eq!(&encoded[29..32], &[0; 3]);
        assert_eq!(LandRecord::decode(&encoded).unwrap(), record);

        let mut nonzero_padding = encoded.clone();
        nonzero_padding[29..32].copy_from_slice(&[0xaa, 0xbb, 0xcc]);
        assert_eq!(LandRecord::decode(&nonzero_padding).unwrap(), record);

        for available in 0..LAND_RECORD_WIRE_SIZE {
            assert!(matches!(
                LandRecord::decode(&encoded[..available]),
                Err(DbRecordError::Truncated { .. })
            ));
        }
        let mut long = encoded;
        long.push(0);
        assert!(matches!(
            LandRecord::decode(&long),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn object_material_round_trips_and_requires_exact_eight_bytes() {
        let material = ObjectMaterial {
            item_vnum: 0x1122_3344,
            count: 0xa1b2_c3d4,
        };
        let encoded = material.encode();
        assert_eq!(
            encoded,
            vec![0x44, 0x33, 0x22, 0x11, 0xd4, 0xc3, 0xb2, 0xa1]
        );
        assert_eq!(ObjectMaterial::decode(&encoded).unwrap(), material);
        for available in 0..OBJECT_MATERIAL_WIRE_SIZE {
            assert!(matches!(
                ObjectMaterial::decode(&encoded[..available]),
                Err(DbRecordError::Truncated { .. })
            ));
        }
        let mut trailing = encoded;
        trailing.push(0);
        assert!(matches!(
            ObjectMaterial::decode(&trailing),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn object_proto_round_trips_all_values_and_requires_exact_96_bytes() {
        let record = ObjectProtoRecord {
            vnum: 0x1122_3344,
            price: 0xa1b2_c3d4,
            materials: [
                ObjectMaterial {
                    item_vnum: 0x0102_0304,
                    count: 5,
                },
                ObjectMaterial {
                    item_vnum: u32::MAX,
                    count: 0,
                },
                ObjectMaterial {
                    item_vnum: 0,
                    count: u32::MAX,
                },
                ObjectMaterial {
                    item_vnum: 0xdead_beef,
                    count: 0x99aa_bbcc,
                },
                ObjectMaterial {
                    item_vnum: 7,
                    count: 11,
                },
            ],
            upgrade_vnum: 0x5566_7788,
            upgrade_limit_time: 0x0102_0304,
            life: i32::MIN,
            regions: [i32::MIN, -2, 0, i32::MAX],
            npc_vnum: 0x1357_9bdf,
            npc_x: 0,
            npc_y: i32::MAX,
            group_vnum: 0x0f0f_0f0f,
            dependent_group_vnum: u32::MAX,
        };

        let encoded = record.encode();
        assert_eq!(encoded.len(), OBJECT_PROTO_RECORD_WIRE_SIZE);
        let mut expected = Vec::with_capacity(OBJECT_PROTO_RECORD_WIRE_SIZE);
        expected.extend_from_slice(&record.vnum.to_le_bytes());
        expected.extend_from_slice(&record.price.to_le_bytes());
        for material in &record.materials {
            expected.extend_from_slice(&material.item_vnum.to_le_bytes());
            expected.extend_from_slice(&material.count.to_le_bytes());
        }
        expected.extend_from_slice(&record.upgrade_vnum.to_le_bytes());
        expected.extend_from_slice(&record.upgrade_limit_time.to_le_bytes());
        expected.extend_from_slice(&record.life.to_le_bytes());
        for region in record.regions {
            expected.extend_from_slice(&region.to_le_bytes());
        }
        expected.extend_from_slice(&record.npc_vnum.to_le_bytes());
        expected.extend_from_slice(&record.npc_x.to_le_bytes());
        expected.extend_from_slice(&record.npc_y.to_le_bytes());
        expected.extend_from_slice(&record.group_vnum.to_le_bytes());
        expected.extend_from_slice(&record.dependent_group_vnum.to_le_bytes());
        assert_eq!(encoded, expected);
        assert_eq!(ObjectProtoRecord::decode(&encoded).unwrap(), record);

        for available in 0..OBJECT_PROTO_RECORD_WIRE_SIZE {
            assert!(matches!(
                ObjectProtoRecord::decode(&encoded[..available]),
                Err(DbRecordError::Truncated { .. })
            ));
        }
        let mut trailing = encoded;
        trailing.push(0);
        assert!(matches!(
            ObjectProtoRecord::decode(&trailing),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn object_record_round_trips_with_exact_packed_offsets() {
        let record = ObjectRecord {
            id: 0x1122_3344,
            land_id: 0xa1b2_c3d4,
            vnum: 0x0102_0304,
            map_index: -2,
            x: 0x0102_0304,
            y: -3,
            x_rot: 1.5,
            y_rot: -2.25,
            z_rot: 0.0,
            life: -559_038_737,
        };
        let encoded = record.encode();
        assert_eq!(encoded.len(), OBJECT_RECORD_WIRE_SIZE);
        assert_eq!(
            encoded,
            vec![
                0x44, 0x33, 0x22, 0x11, // id
                0xd4, 0xc3, 0xb2, 0xa1, // land_id
                0x04, 0x03, 0x02, 0x01, // vnum
                0xfe, 0xff, 0xff, 0xff, // map_index
                0x04, 0x03, 0x02, 0x01, // x
                0xfd, 0xff, 0xff, 0xff, // y
                0x00, 0x00, 0xc0, 0x3f, // x_rot = 1.5
                0x00, 0x00, 0x10, 0xc0, // y_rot = -2.25
                0x00, 0x00, 0x00, 0x00, // z_rot = +0.0
                0xef, 0xbe, 0xad, 0xde, // life
            ]
        );
        assert_eq!(ObjectRecord::decode(&encoded).unwrap(), record);
    }

    #[test]
    fn object_record_rejects_truncation_and_trailing_bytes() {
        let encoded = ObjectRecord {
            id: 1,
            land_id: 2,
            vnum: 3,
            map_index: 4,
            x: 5,
            y: 6,
            x_rot: 7.0,
            y_rot: 8.0,
            z_rot: 9.0,
            life: 10,
        }
        .encode();
        for available in 0..OBJECT_RECORD_WIRE_SIZE {
            assert!(matches!(
                ObjectRecord::decode(&encoded[..available]),
                Err(DbRecordError::Truncated { .. })
            ));
        }
        let mut trailing = encoded;
        trailing.push(0);
        assert!(matches!(
            ObjectRecord::decode(&trailing),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn object_record_preserves_float_bits_and_signed_domains() {
        let x_rot = f32::from_bits(0x8000_0000); // negative zero
        let y_rot = f32::from_bits(0x7fc0_1234); // NaN payload
        let z_rot = f32::from_bits(0xff80_0000); // negative infinity
        let record = ObjectRecord {
            id: u32::MAX,
            land_id: 0,
            vnum: u32::MAX,
            map_index: i32::MIN,
            x: i32::MAX,
            y: i32::MIN,
            x_rot,
            y_rot,
            z_rot,
            life: i32::MAX,
        };
        let encoded = record.encode();
        let decoded = ObjectRecord::decode(&encoded).unwrap();
        assert_eq!(decoded.id, u32::MAX);
        assert_eq!(decoded.land_id, 0);
        assert_eq!(decoded.vnum, u32::MAX);
        assert_eq!(decoded.map_index, i32::MIN);
        assert_eq!(decoded.x, i32::MAX);
        assert_eq!(decoded.y, i32::MIN);
        assert_eq!(decoded.life, i32::MAX);
        assert_eq!(decoded.x_rot.to_bits(), x_rot.to_bits());
        assert_eq!(decoded.y_rot.to_bits(), y_rot.to_bits());
        assert_eq!(decoded.z_rot.to_bits(), z_rot.to_bits());
        assert_eq!(&encoded[24..28], &x_rot.to_bits().to_le_bytes());
        assert_eq!(&encoded[28..32], &y_rot.to_bits().to_le_bytes());
        assert_eq!(&encoded[32..36], &z_rot.to_bits().to_le_bytes());
    }

    #[test]
    fn market_item_price_round_trips_with_exact_packed_offsets() {
        let record = MarketItemPriceRecord {
            vnum: 0x1122_3344,
            gold: 0x0102_0304_0506_0708,
            cheque: 0xa1b2_c3d4,
        };
        let encoded = record.encode();
        assert_eq!(encoded.len(), MARKET_ITEM_PRICE_WIRE_SIZE);
        assert_eq!(
            encoded,
            vec![
                0x44, 0x33, 0x22, 0x11, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0xd4, 0xc3,
                0xb2, 0xa1,
            ]
        );
        assert_eq!(MarketItemPriceRecord::decode(&encoded).unwrap(), record);

        for record in [
            MarketItemPriceRecord {
                vnum: u32::MAX,
                gold: i64::MIN,
                cheque: u32::MAX,
            },
            MarketItemPriceRecord {
                vnum: 0,
                gold: i64::MAX,
                cheque: 0,
            },
        ] {
            assert_eq!(
                MarketItemPriceRecord::decode(&record.encode()).unwrap(),
                record
            );
        }

        let mut short = record.encode();
        short.pop();
        assert!(matches!(
            MarketItemPriceRecord::decode(&short),
            Err(DbRecordError::Truncated { .. })
        ));
        let mut long = record.encode();
        long.push(0);
        assert!(matches!(
            MarketItemPriceRecord::decode(&long),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn event_table_round_trips_with_exact_packed_offsets() {
        let mut event_type = [0_u8; EVENT_TYPE_BYTES];
        event_type[..5].copy_from_slice(b"event");
        let record = EventTableRecord {
            id: 0x1122_3344,
            event_type,
            start_time: -1_234_567,
            end_time: 1_234_567,
            value0: -42,
            value1: 42,
            completed: 1,
        };
        let encoded = record.encode();
        assert_eq!(encoded.len(), EVENT_TABLE_WIRE_SIZE);
        assert_eq!(&encoded[0..4], &[0x44, 0x33, 0x22, 0x11]);
        assert_eq!(&encoded[4..9], b"event");
        assert_eq!(&encoded[68..72], &(-1_234_567_i32).to_le_bytes());
        assert_eq!(&encoded[72..76], &1_234_567_i32.to_le_bytes());
        assert_eq!(&encoded[76..80], &(-42_i32).to_le_bytes());
        assert_eq!(&encoded[80..84], &42_i32.to_le_bytes());
        assert_eq!(encoded[84], 1);
        assert_eq!(EventTableRecord::decode(&encoded).unwrap(), record);

        let raw_bool = EventTableRecord {
            completed: 0xff,
            ..record
        };
        assert_eq!(
            EventTableRecord::decode(&raw_bool.encode())
                .unwrap()
                .completed,
            0xff
        );

        let mut short = encoded.clone();
        short.pop();
        assert!(matches!(
            EventTableRecord::decode(&short),
            Err(DbRecordError::Truncated { .. })
        ));
        let mut long = encoded;
        long.push(0);
        assert!(matches!(
            EventTableRecord::decode(&long),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn skill_table_round_trips_with_padding_free_offsets() {
        fn filled<const N: usize>(value: u8) -> [u8; N] {
            [value; N]
        }

        let mut name = filled(0x80);
        name[..6].copy_from_slice(b"SKILL\0");
        let record = SkillTableRecord {
            vnum: 0x1122_3344,
            name,
            skill_type: 0x12,
            max_level: 0x34,
            splash_range: 0x5566_7788,
            point_on: filled(0x21),
            point_poly: filled(0x22),
            sp_cost_poly: filled(0x23),
            duration_poly: filled(0x24),
            duration_sp_cost_poly: filled(0x25),
            cooldown_poly: filled(0x26),
            master_bonus_poly: filled(0x27),
            grand_master_add_sp_cost_poly: filled(0x28),
            flag: 0x99aa_bbcc,
            affect_flag: 0xddee_ff00,
            point_on2: filled(0x31),
            point_poly2: filled(0x32),
            duration_poly2: filled(0x33),
            affect_flag2: 0x0bad_f00d,
            point_on3: filled(0x41),
            point_poly3: filled(0x42),
            duration_poly3: filled(0x43),
            level_step: 0x5a,
            level_limit: 0xa5,
            pre_skill_vnum: 0x1234_5678,
            pre_skill_level: 0xef,
            max_hit: i32::MIN,
            splash_around_damage_adjust_poly: filled(0x51),
            skill_attr_type: 0x7e,
            target_range: 0xdead_beef,
        };

        assert_eq!(
            [
                SKILL_TABLE_VNUM_OFFSET,
                SKILL_TABLE_NAME_OFFSET,
                SKILL_TABLE_SKILL_TYPE_OFFSET,
                SKILL_TABLE_MAX_LEVEL_OFFSET,
                SKILL_TABLE_SPLASH_RANGE_OFFSET,
                SKILL_TABLE_POINT_ON_OFFSET,
                SKILL_TABLE_POINT_POLY_OFFSET,
                SKILL_TABLE_SP_COST_POLY_OFFSET,
                SKILL_TABLE_DURATION_POLY_OFFSET,
                SKILL_TABLE_DURATION_SP_COST_POLY_OFFSET,
                SKILL_TABLE_COOLDOWN_POLY_OFFSET,
                SKILL_TABLE_MASTER_BONUS_POLY_OFFSET,
                SKILL_TABLE_GRAND_MASTER_ADD_SP_COST_POLY_OFFSET,
                SKILL_TABLE_FLAG_OFFSET,
                SKILL_TABLE_AFFECT_FLAG_OFFSET,
                SKILL_TABLE_POINT_ON2_OFFSET,
                SKILL_TABLE_POINT_POLY2_OFFSET,
                SKILL_TABLE_DURATION_POLY2_OFFSET,
                SKILL_TABLE_AFFECT_FLAG2_OFFSET,
                SKILL_TABLE_POINT_ON3_OFFSET,
                SKILL_TABLE_POINT_POLY3_OFFSET,
                SKILL_TABLE_DURATION_POLY3_OFFSET,
                SKILL_TABLE_LEVEL_STEP_OFFSET,
                SKILL_TABLE_LEVEL_LIMIT_OFFSET,
                SKILL_TABLE_PRE_SKILL_VNUM_OFFSET,
                SKILL_TABLE_PRE_SKILL_LEVEL_OFFSET,
                SKILL_TABLE_MAX_HIT_OFFSET,
                SKILL_TABLE_SPLASH_AROUND_DAMAGE_ADJUST_POLY_OFFSET,
                SKILL_TABLE_SKILL_ATTR_TYPE_OFFSET,
                SKILL_TABLE_TARGET_RANGE_OFFSET,
            ],
            [
                0, 4, 37, 38, 39, 43, 107, 208, 309, 410, 511, 612, 713, 814, 818, 822, 886, 987,
                1_088, 1_092, 1_156, 1_257, 1_358, 1_359, 1_360, 1_364, 1_365, 1_369, 1_470, 1_471,
            ]
        );

        let encoded = record.encode();
        assert_eq!(encoded.len(), SKILL_TABLE_RECORD_WIRE_SIZE);
        assert_eq!(&encoded[0..4], &0x1122_3344_u32.to_le_bytes());
        assert_eq!(&encoded[4..37], &record.name);
        assert_eq!(encoded[37], record.skill_type);
        assert_eq!(encoded[38], record.max_level);
        assert_eq!(&encoded[39..43], &0x5566_7788_u32.to_le_bytes());
        assert_eq!(&encoded[43..107], &record.point_on);
        assert_eq!(&encoded[107..208], &record.point_poly);
        assert_eq!(&encoded[208..309], &record.sp_cost_poly);
        assert_eq!(&encoded[309..410], &record.duration_poly);
        assert_eq!(&encoded[410..511], &record.duration_sp_cost_poly);
        assert_eq!(&encoded[511..612], &record.cooldown_poly);
        assert_eq!(&encoded[612..713], &record.master_bonus_poly);
        assert_eq!(&encoded[713..814], &record.grand_master_add_sp_cost_poly);
        assert_eq!(&encoded[814..818], &0x99aa_bbcc_u32.to_le_bytes());
        assert_eq!(&encoded[818..822], &0xddee_ff00_u32.to_le_bytes());
        assert_eq!(&encoded[822..886], &record.point_on2);
        assert_eq!(&encoded[886..987], &record.point_poly2);
        assert_eq!(&encoded[987..1_088], &record.duration_poly2);
        assert_eq!(&encoded[1_088..1_092], &0x0bad_f00d_u32.to_le_bytes());
        assert_eq!(&encoded[1_092..1_156], &record.point_on3);
        assert_eq!(&encoded[1_156..1_257], &record.point_poly3);
        assert_eq!(&encoded[1_257..1_358], &record.duration_poly3);
        assert_eq!(encoded[1_358], record.level_step);
        assert_eq!(encoded[1_359], record.level_limit);
        assert_eq!(&encoded[1_360..1_364], &0x1234_5678_u32.to_le_bytes());
        assert_eq!(encoded[1_364], record.pre_skill_level);
        assert_eq!(&encoded[1_365..1_369], &i32::MIN.to_le_bytes());
        assert_eq!(
            &encoded[1_369..1_470],
            &record.splash_around_damage_adjust_poly
        );
        assert_eq!(encoded[1_470], record.skill_attr_type);
        assert_eq!(&encoded[1_471..1_475], &0xdead_beef_u32.to_le_bytes());
        assert_eq!(SkillTableRecord::decode(&encoded).unwrap(), record);
        assert_eq!(SkillTableRecord::from_bytes(&encoded).unwrap(), record);
        assert_eq!(record.to_bytes(), encoded);
    }

    #[test]
    fn skill_table_requires_exact_wire_size() {
        let encoded = SkillTableRecord::default().encode();
        assert!(matches!(
            SkillTableRecord::decode(&encoded[..SKILL_TABLE_RECORD_WIRE_SIZE - 1]),
            Err(DbRecordError::Truncated {
                record: "TSkillTable",
                needed: SKILL_TABLE_RECORD_WIRE_SIZE,
                available: 1_474,
            })
        ));
        let mut trailing = encoded;
        trailing.push(0);
        assert!(matches!(
            SkillTableRecord::decode(&trailing),
            Err(DbRecordError::LengthMismatch {
                record: "TSkillTable",
                expected: SKILL_TABLE_RECORD_WIRE_SIZE,
                actual: 1_476,
            })
        ));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn mob_table_default_has_source_literal_offsets_and_size() {
        let offsets = [
            MOB_TABLE_VNUM_OFFSET,
            MOB_TABLE_NAME_OFFSET,
            MOB_TABLE_LOCALE_NAME_OFFSET,
            MOB_TABLE_MOB_TYPE_OFFSET,
            MOB_TABLE_RANK_OFFSET,
            MOB_TABLE_BATTLE_TYPE_OFFSET,
            MOB_TABLE_LEVEL_OFFSET,
            MOB_TABLE_SIZE_OFFSET,
            MOB_TABLE_GOLD_MIN_OFFSET,
            MOB_TABLE_GOLD_MAX_OFFSET,
            MOB_TABLE_EXP_OFFSET,
            MOB_TABLE_MAX_HP_OFFSET,
            MOB_TABLE_REGEN_CYCLE_OFFSET,
            MOB_TABLE_REGEN_PERCENT_OFFSET,
            MOB_TABLE_DEF_OFFSET,
            MOB_TABLE_AI_FLAG_OFFSET,
            MOB_TABLE_RACE_FLAG_OFFSET,
            MOB_TABLE_IMMUNE_FLAG_OFFSET,
            MOB_TABLE_STR_OFFSET,
            MOB_TABLE_DEX_OFFSET,
            MOB_TABLE_CON_OFFSET,
            MOB_TABLE_INT_OFFSET,
            MOB_TABLE_DAMAGE_RANGE_OFFSET,
            MOB_TABLE_ATTACK_SPEED_OFFSET,
            MOB_TABLE_MOVING_SPEED_OFFSET,
            MOB_TABLE_AGGRESSIVE_HP_PCT_OFFSET,
            MOB_TABLE_AGGRESSIVE_SIGHT_OFFSET,
            MOB_TABLE_ATTACK_RANGE_OFFSET,
            MOB_TABLE_ENCHANTS_OFFSET,
            MOB_TABLE_RESISTS_OFFSET,
            MOB_TABLE_RESURRECTION_VNUM_OFFSET,
            MOB_TABLE_DROP_ITEM_VNUM_OFFSET,
            MOB_TABLE_MOUNT_CAPACITY_OFFSET,
            MOB_TABLE_ON_CLICK_TYPE_OFFSET,
            MOB_TABLE_EMPIRE_OFFSET,
            MOB_TABLE_FOLDER_OFFSET,
            MOB_TABLE_DAM_MULTIPLY_OFFSET,
            MOB_TABLE_SUMMON_VNUM_OFFSET,
            MOB_TABLE_DRAIN_SP_OFFSET,
            MOB_TABLE_MOB_COLOR_OFFSET,
            MOB_TABLE_POLYMORPH_ITEM_VNUM_OFFSET,
            MOB_TABLE_SKILLS_OFFSET,
            MOB_TABLE_BERSERK_POINT_OFFSET,
            MOB_TABLE_STONE_SKIN_POINT_OFFSET,
            MOB_TABLE_GOD_SPEED_POINT_OFFSET,
            MOB_TABLE_DEATH_BLOW_POINT_OFFSET,
            MOB_TABLE_REVIVE_POINT_OFFSET,
        ];
        assert_eq!(
            offsets,
            [
                0, 4, 29, 54, 55, 56, 57, 58, 59, 63, 67, 71, 75, 76, 77, 79, 83, 87, 91, 92, 93,
                94, 95, 103, 105, 107, 108, 110, 112, 118, 129, 133, 137, 138, 139, 140, 205, 209,
                213, 217, 221, 225, 250, 251, 252, 253, 254,
            ]
        );
        assert_eq!(MOB_TABLE_RECORD_WIRE_SIZE, 255);
        assert_eq!(MOB_TABLE_WIRE_SIZE, 255);
        assert_eq!(T_MOB_TABLE_SIZE, 255);
        assert_eq!(MobTableRecord::WIRE_SIZE, 255);
        assert_eq!(MobTableRecord::default().encode(), vec![0; 255]);

        assert_eq!(MOB_NAME_MAX_LEN, 24);
        assert_eq!(MOB_NAME_BYTES, 25);
        assert_eq!(MOB_LOCALE_NAME_MAX_LEN, 24);
        assert_eq!(MOB_LOCALE_NAME_BYTES, 25);
        assert_eq!(MOB_FOLDER_MAX_LEN, 64);
        assert_eq!(MOB_FOLDER_BYTES, 65);
        assert_eq!(MOB_ENCHANTS_MAX_NUM, 6);
        assert_eq!(MOB_RESISTS_MAX_NUM, 11);
        assert_eq!(MOB_SKILL_MAX_NUM, 5);
        assert_eq!(MOB_DAMAGE_RANGE_COUNT, 2);
        assert_eq!(MOB_SKILL_RECORD_WIRE_SIZE, 5);
        assert_eq!(MOB_SKILL_WIRE_SIZE, 5);
        assert_eq!(T_MOB_SKILL_LEVEL_SIZE, 5);

        let _: TMobTable = MobTableRecord::default();
        let _: TMobSkillLevel = MobSkillRecord::default();
    }

    #[test]
    #[allow(clippy::field_reassign_with_default, clippy::too_many_lines)]
    fn mob_table_round_trips_extrema_raw_bytes_and_float_bits() {
        let mut name = [0_u8; MOB_NAME_BYTES];
        name[..9].copy_from_slice(&[0xff, b'M', 0, 0xc3, 0xa9, 0, 0x80, b'Z', 0]);
        name[9..].fill(0xa5);
        let mut locale_name = [0x5a_u8; MOB_LOCALE_NAME_BYTES];
        locale_name[..8].copy_from_slice(&[0, 0x80, b'L', 0, 0xf0, 0x9f, 0x92, 0xa9]);
        locale_name[8..].fill(0x3c);
        let mut folder = [0x6b_u8; MOB_FOLDER_BYTES];
        folder[..10].copy_from_slice(&[b'f', 0, 0x80, 0xff, 0, b'/', b'\\', b'm', b'o', b'b']);
        folder[10..].fill(0xd4);
        let nan = f32::from_bits(0x7fc0_1234);
        let record = MobTableRecord {
            vnum: u32::MAX,
            name,
            locale_name,
            mob_type: u8::MAX,
            rank: 0,
            battle_type: 1,
            level: 2,
            size: 3,
            gold_min: 0,
            gold_max: u32::MAX,
            exp: 0x0102_0304,
            max_hp: 0x89ab_cdef,
            regen_cycle: 4,
            regen_percent: 5,
            def: u16::MAX,
            ai_flag: 0x1020_3040,
            race_flag: 0x5060_7080,
            immune_flag: u32::MAX,
            str: 6,
            dex: 7,
            con: 8,
            int_: 9,
            damage_range: [0, u32::MAX],
            attack_speed: i16::MIN,
            moving_speed: i16::MAX,
            aggressive_hp_pct: 10,
            aggressive_sight: 11,
            attack_range: 12,
            enchants: [i8::MIN, -1, 0, 1, i8::MAX, 42],
            resists: [i8::MIN, -100, -1, 0, 1, 100, i8::MAX, 2, 3, 4, 5],
            resurrection_vnum: 0x1122_3344,
            drop_item_vnum: 0x5566_7788,
            mount_capacity: 13,
            on_click_type: 14,
            empire: 15,
            folder,
            dam_multiply: nan,
            summon_vnum: 0x99aa_bbcc,
            drain_sp: 0xddee_ff00,
            mob_color: 0x0bad_f00d,
            polymorph_item_vnum: 0x1234_5678,
            skills: [
                MobSkillRecord {
                    vnum: 0x0102_0304,
                    level: 0x80,
                },
                MobSkillRecord {
                    vnum: 0x1112_1314,
                    level: 0x7f,
                },
                MobSkillRecord {
                    vnum: 0x2122_2324,
                    level: 1,
                },
                MobSkillRecord {
                    vnum: 0x3132_3334,
                    level: 2,
                },
                MobSkillRecord {
                    vnum: u32::MAX,
                    level: u8::MAX,
                },
            ],
            berserk_point: 16,
            stone_skin_point: 17,
            god_speed_point: 18,
            death_blow_point: 19,
            revive_point: 20,
        };

        let encoded = record.encode();
        assert_eq!(encoded.len(), MOB_TABLE_RECORD_WIRE_SIZE);
        assert_eq!(&encoded[0..4], &u32::MAX.to_le_bytes());
        assert_eq!(&encoded[4..29], &record.name);
        assert_eq!(&encoded[29..54], &record.locale_name);
        assert_eq!(&encoded[95..103], &[0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(&encoded[103..105], &i16::MIN.to_le_bytes());
        assert_eq!(&encoded[105..107], &i16::MAX.to_le_bytes());
        assert_eq!(&encoded[112..118], &[0x80, 0xff, 0, 1, 0x7f, 42]);
        assert_eq!(&encoded[140..205], &record.folder);
        assert_eq!(&encoded[205..209], &nan.to_bits().to_le_bytes());
        assert_eq!(&encoded[225..230], &[0x04, 0x03, 0x02, 0x01, 0x80]);

        let decoded = MobTableRecord::decode(&encoded).unwrap();
        assert_eq!(decoded, record);
        assert_eq!(decoded.dam_multiply.to_bits(), nan.to_bits());
        assert_eq!(decoded.enchants, record.enchants);
        assert_eq!(decoded.resists, record.resists);

        let mut negative_zero = record;
        negative_zero.dam_multiply = f32::from_bits(0x8000_0000);
        let decoded_zero = MobTableRecord::decode(&negative_zero.encode()).unwrap();
        assert_eq!(decoded_zero, negative_zero);
        assert_eq!(decoded_zero.dam_multiply.to_bits(), 0x8000_0000);
        let mut positive_zero = negative_zero;
        positive_zero.dam_multiply = 0.0;
        assert_ne!(positive_zero, negative_zero);
    }

    #[test]
    fn mob_table_and_skill_reject_every_short_prefix_and_trailing_bytes() {
        let table = MobTableRecord::default().encode();
        for available in 0..MOB_TABLE_RECORD_WIRE_SIZE {
            assert!(matches!(
                MobTableRecord::decode(&table[..available]),
                Err(DbRecordError::Truncated {
                    record: "TMobTable",
                    needed: MOB_TABLE_RECORD_WIRE_SIZE,
                    available: _,
                })
            ));
        }
        let mut table_trailing = table;
        table_trailing.push(0);
        assert!(matches!(
            MobTableRecord::decode(&table_trailing),
            Err(DbRecordError::LengthMismatch {
                record: "TMobTable",
                expected: MOB_TABLE_RECORD_WIRE_SIZE,
                actual: 256,
            })
        ));

        let skill = MobSkillRecord { vnum: 1, level: 2 }.encode();
        for available in 0..MOB_SKILL_RECORD_WIRE_SIZE {
            assert!(matches!(
                MobSkillRecord::decode(&skill[..available]),
                Err(DbRecordError::Truncated { .. })
            ));
        }
        let mut skill_trailing = skill;
        skill_trailing.push(0);
        assert!(matches!(
            MobSkillRecord::decode(&skill_trailing),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn mob_skill_uses_exact_five_byte_layout() {
        let skill = MobSkillRecord {
            vnum: 0x1122_3344,
            level: 0xa5,
        };
        let encoded = skill.encode();
        assert_eq!(encoded, vec![0x44, 0x33, 0x22, 0x11, 0xa5]);
        assert_eq!(encoded.len(), 5);
        assert_eq!(MobSkillRecord::decode(&encoded).unwrap(), skill);
        assert_eq!(MobSkillRecord::from_bytes(&encoded).unwrap(), skill);
        assert_eq!(skill.to_bytes(), encoded);
    }

    #[test]
    fn refine_table_round_trips_with_exact_packed_offsets() {
        let record = RefineTableRecord {
            id: 0x1122_3344,
            material_count: 3,
            cost: -12_345,
            prob: i32::MAX,
            materials: [
                RefineMaterialRecord {
                    vnum: 0x0102_0304,
                    count: -7,
                },
                RefineMaterialRecord {
                    vnum: 0x1112_1314,
                    count: 42,
                },
                RefineMaterialRecord {
                    vnum: 0x2122_2324,
                    count: i32::MIN,
                },
                RefineMaterialRecord::default(),
                RefineMaterialRecord {
                    vnum: u32::MAX,
                    count: i32::MAX,
                },
            ],
        };
        let encoded = record.encode();
        assert_eq!(encoded.len(), REFINE_TABLE_WIRE_SIZE);
        assert_eq!(&encoded[0..4], &[0x44, 0x33, 0x22, 0x11]);
        assert_eq!(encoded[4], 3);
        assert_eq!(&encoded[5..9], &(-12_345_i32).to_le_bytes());
        assert_eq!(&encoded[9..13], &i32::MAX.to_le_bytes());
        assert_eq!(&encoded[13..17], &0x0102_0304_u32.to_le_bytes());
        assert_eq!(&encoded[17..21], &(-7_i32).to_le_bytes());
        assert_eq!(&encoded[21..25], &0x1112_1314_u32.to_le_bytes());
        assert_eq!(&encoded[25..29], &42_i32.to_le_bytes());
        assert_eq!(&encoded[49..53], &i32::MAX.to_le_bytes());
        assert_eq!(RefineTableRecord::decode(&encoded).unwrap(), record);

        let mut short = encoded.clone();
        short.pop();
        assert!(matches!(
            RefineTableRecord::decode(&short),
            Err(DbRecordError::Truncated { .. })
        ));
        let mut long = encoded;
        long.push(0);
        assert!(matches!(
            RefineTableRecord::decode(&long),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn item_table_round_trips_with_exact_offsets_and_raw_names() {
        let mut name = [0x5a_u8; ITEM_NAME_BYTES];
        name[..9].copy_from_slice(&[0xff, b'I', 0, 0xc3, 0xa9, 0, 0x80, b'Z', 0]);
        name[9..].fill(0xa5);
        let mut locale_name = [0x6b_u8; ITEM_NAME_BYTES];
        locale_name[..8].copy_from_slice(&[0, 0x80, b'L', 0, 0xf0, 0x9f, 0x92, 0xa9]);
        locale_name[8..].fill(0x3c);

        let record = ItemTableRecord {
            vnum: 0x1122_3344,
            vnum_range: u32::MAX,
            name,
            locale_name,
            item_type: 0,
            sub_type: u8::MAX,
            weight: 1,
            size: 254,
            anti_flags: 0x0102_0304,
            flags: 0xa1b2_c3d4,
            wear_flags: 0x1020_3040,
            immune_flag: 0xdead_beef,
            gold: u64::MAX,
            shop_buy_price: 0x0123_4567_89ab_cdef,
            limits: [
                ItemLimitRecord {
                    limit_type: 0xff,
                    value: i32::MIN,
                },
                ItemLimitRecord {
                    limit_type: 0,
                    value: i32::MAX,
                },
            ],
            applies: [
                ItemApplyRecord {
                    apply_type: 0x80,
                    value: -1,
                },
                ItemApplyRecord {
                    apply_type: 7,
                    value: 0x0102_0304_i32,
                },
                ItemApplyRecord {
                    apply_type: u8::MAX,
                    value: i32::MIN,
                },
            ],
            values: [i32::MIN, -1, 0, 1, 0x5555_5555, i32::MAX],
            sockets: [i32::MAX, -2_000_000, -1, 0, 123, i32::MIN],
            refined_vnum: 0x9988_7766,
            refine_set: u16::MAX,
            alter_to_magic_item_pct: 0,
            specular: 1,
            gain_socket_pct: u8::MAX,
            addon_type: i16::MIN,
            limit_real_time_first_use_index: i8::MIN,
            limit_timer_based_on_wear_index: -1,
        };
        let encoded = record.encode();
        assert_eq!(encoded.len(), ITEM_TABLE_RECORD_WIRE_SIZE);
        assert_eq!(ITEM_TABLE_RECORD_WIRE_SIZE, 204);
        assert_eq!(T_ITEM_TABLE_SIZE, 204);
        assert_eq!(ItemTableRecord::WIRE_SIZE, 204);

        assert_eq!(
            &encoded[ITEM_TABLE_VNUM_OFFSET..ITEM_TABLE_VNUM_OFFSET + 4],
            &record.vnum.to_le_bytes()
        );
        assert_eq!(
            &encoded[ITEM_TABLE_VNUM_RANGE_OFFSET..ITEM_TABLE_VNUM_RANGE_OFFSET + 4],
            &record.vnum_range.to_le_bytes()
        );
        assert_eq!(
            &encoded[ITEM_TABLE_NAME_OFFSET..ITEM_TABLE_LOCALE_NAME_OFFSET],
            &record.name
        );
        assert_eq!(
            &encoded[ITEM_TABLE_LOCALE_NAME_OFFSET..ITEM_TABLE_ITEM_TYPE_OFFSET],
            &record.locale_name
        );
        assert_eq!(encoded[ITEM_TABLE_ITEM_TYPE_OFFSET], record.item_type);
        assert_eq!(encoded[ITEM_TABLE_SUB_TYPE_OFFSET], record.sub_type);
        assert_eq!(encoded[ITEM_TABLE_WEIGHT_OFFSET], record.weight);
        assert_eq!(encoded[ITEM_TABLE_SIZE_OFFSET], record.size);
        assert_eq!(
            &encoded[ITEM_TABLE_ANTI_FLAGS_OFFSET..ITEM_TABLE_FLAGS_OFFSET],
            &record.anti_flags.to_le_bytes()
        );
        assert_eq!(
            &encoded[ITEM_TABLE_FLAGS_OFFSET..ITEM_TABLE_WEAR_FLAGS_OFFSET],
            &record.flags.to_le_bytes()
        );
        assert_eq!(
            &encoded[ITEM_TABLE_WEAR_FLAGS_OFFSET..ITEM_TABLE_IMMUNE_FLAG_OFFSET],
            &record.wear_flags.to_le_bytes()
        );
        assert_eq!(
            &encoded[ITEM_TABLE_IMMUNE_FLAG_OFFSET..ITEM_TABLE_GOLD_OFFSET],
            &record.immune_flag.to_le_bytes()
        );
        assert_eq!(
            &encoded[ITEM_TABLE_GOLD_OFFSET..ITEM_TABLE_SHOP_BUY_PRICE_OFFSET],
            &record.gold.to_le_bytes()
        );
        assert_eq!(
            &encoded[ITEM_TABLE_SHOP_BUY_PRICE_OFFSET..ITEM_TABLE_LIMITS_OFFSET],
            &record.shop_buy_price.to_le_bytes()
        );
        assert_eq!(
            &encoded[ITEM_TABLE_LIMITS_OFFSET..ITEM_TABLE_APPLIES_OFFSET],
            &{
                let mut expected = Vec::new();
                for limit in &record.limits {
                    expected.push(limit.limit_type);
                    expected.extend_from_slice(&limit.value.to_le_bytes());
                }
                expected
            }
        );
        assert_eq!(
            &encoded[ITEM_TABLE_APPLIES_OFFSET..ITEM_TABLE_VALUES_OFFSET],
            &{
                let mut expected = Vec::new();
                for apply in &record.applies {
                    expected.push(apply.apply_type);
                    expected.extend_from_slice(&apply.value.to_le_bytes());
                }
                expected
            }
        );
        assert_eq!(
            &encoded[ITEM_TABLE_VALUES_OFFSET..ITEM_TABLE_SOCKETS_OFFSET],
            &{
                let mut expected = Vec::new();
                for value in record.values {
                    expected.extend_from_slice(&value.to_le_bytes());
                }
                expected
            }
        );
        assert_eq!(
            &encoded[ITEM_TABLE_SOCKETS_OFFSET..ITEM_TABLE_REFINED_VNUM_OFFSET],
            &{
                let mut expected = Vec::new();
                for socket in record.sockets {
                    expected.extend_from_slice(&socket.to_le_bytes());
                }
                expected
            }
        );
        assert_eq!(
            &encoded[ITEM_TABLE_REFINED_VNUM_OFFSET..ITEM_TABLE_REFINE_SET_OFFSET],
            &record.refined_vnum.to_le_bytes()
        );
        assert_eq!(
            &encoded[ITEM_TABLE_REFINE_SET_OFFSET..ITEM_TABLE_ALTER_TO_MAGIC_ITEM_PCT_OFFSET],
            &record.refine_set.to_le_bytes()
        );
        assert_eq!(
            encoded[ITEM_TABLE_ALTER_TO_MAGIC_ITEM_PCT_OFFSET],
            record.alter_to_magic_item_pct
        );
        assert_eq!(encoded[ITEM_TABLE_SPECULAR_OFFSET], record.specular);
        assert_eq!(
            encoded[ITEM_TABLE_GAIN_SOCKET_PCT_OFFSET],
            record.gain_socket_pct
        );
        assert_eq!(
            &encoded
                [ITEM_TABLE_ADDON_TYPE_OFFSET..ITEM_TABLE_LIMIT_REAL_TIME_FIRST_USE_INDEX_OFFSET],
            &record.addon_type.to_le_bytes()
        );
        assert_eq!(
            encoded[ITEM_TABLE_LIMIT_REAL_TIME_FIRST_USE_INDEX_OFFSET],
            record.limit_real_time_first_use_index.to_le_bytes()[0]
        );
        assert_eq!(
            encoded[ITEM_TABLE_LIMIT_TIMER_BASED_ON_WEAR_INDEX_OFFSET],
            record.limit_timer_based_on_wear_index.to_le_bytes()[0]
        );
        assert_eq!(
            &encoded[ITEM_TABLE_NAME_OFFSET..ITEM_TABLE_NAME_OFFSET + 9],
            &[0xff, b'I', 0, 0xc3, 0xa9, 0, 0x80, b'Z', 0]
        );

        assert_eq!(ItemTableRecord::decode(&encoded).unwrap(), record);
        assert_eq!(ItemTableRecord::from_bytes(&encoded).unwrap(), record);
        assert_eq!(record.to_bytes(), encoded);
    }

    #[test]
    fn item_limit_and_apply_records_use_five_byte_exact_codecs() {
        let limits = [
            ItemLimitRecord {
                limit_type: u8::MAX,
                value: i32::MIN,
            },
            ItemLimitRecord {
                limit_type: 0,
                value: i32::MAX,
            },
        ];
        for record in limits {
            let encoded = record.encode();
            assert_eq!(encoded.len(), ITEM_LIMIT_RECORD_WIRE_SIZE);
            assert_eq!(encoded[0], record.limit_type);
            assert_eq!(&encoded[1..5], &record.value.to_le_bytes());
            assert_eq!(ItemLimitRecord::decode(&encoded).unwrap(), record);
            let mut short = encoded.clone();
            short.pop();
            assert!(matches!(
                ItemLimitRecord::decode(&short),
                Err(DbRecordError::Truncated { .. })
            ));
            let mut long = encoded;
            long.push(0);
            assert!(matches!(
                ItemLimitRecord::decode(&long),
                Err(DbRecordError::LengthMismatch { .. })
            ));
        }

        let applies = [
            ItemApplyRecord {
                apply_type: 0x80,
                value: -1,
            },
            ItemApplyRecord {
                apply_type: 7,
                value: 123,
            },
            ItemApplyRecord {
                apply_type: u8::MAX,
                value: i32::MIN,
            },
        ];
        for record in applies {
            let encoded = record.encode();
            assert_eq!(encoded.len(), ITEM_APPLY_RECORD_WIRE_SIZE);
            assert_eq!(encoded[0], record.apply_type);
            assert_eq!(&encoded[1..5], &record.value.to_le_bytes());
            assert_eq!(ItemApplyRecord::decode(&encoded).unwrap(), record);
            let mut short = encoded.clone();
            short.pop();
            assert!(matches!(
                ItemApplyRecord::decode(&short),
                Err(DbRecordError::Truncated { .. })
            ));
            let mut long = encoded;
            long.push(0);
            assert!(matches!(
                ItemApplyRecord::decode(&long),
                Err(DbRecordError::LengthMismatch { .. })
            ));
        }
    }

    #[test]
    fn item_table_rejects_short_overlong_and_fragmented_input() {
        let record = ItemTableRecord {
            vnum: 7,
            gold: u64::MAX,
            ..ItemTableRecord::default()
        };
        let encoded = record.encode();
        for available in 0..ITEM_TABLE_RECORD_WIRE_SIZE {
            assert!(matches!(
                ItemTableRecord::decode(&encoded[..available]),
                Err(DbRecordError::Truncated {
                    record: "TItemTable",
                    needed: ITEM_TABLE_RECORD_WIRE_SIZE,
                    available: _,
                })
            ));
        }

        // The record codec is stateless, but callers may still assemble a
        // fragmented DB payload. Decode only after all exact-width bytes arrive.
        let mut assembled = Vec::with_capacity(ITEM_TABLE_RECORD_WIRE_SIZE);
        for byte in &encoded {
            assembled.push(*byte);
            if assembled.len() < ITEM_TABLE_RECORD_WIRE_SIZE {
                assert!(matches!(
                    ItemTableRecord::decode(&assembled),
                    Err(DbRecordError::Truncated { .. })
                ));
            }
        }
        assert_eq!(ItemTableRecord::decode(&assembled).unwrap(), record);

        let mut trailing = encoded;
        trailing.push(0);
        assert!(matches!(
            ItemTableRecord::decode(&trailing),
            Err(DbRecordError::LengthMismatch {
                record: "TItemTable",
                expected: ITEM_TABLE_RECORD_WIRE_SIZE,
                actual: 205,
            })
        ));
    }

    #[test]
    fn item_attr_round_trips_with_exact_packed_offsets() {
        let mut apply = [0_u8; APPLY_NAME_MAX_LEN + 1];
        apply[..5].copy_from_slice(b"ATK+1");
        apply[5..].fill(0x80);
        let record = ItemAttrRecord {
            apply,
            apply_index: 0x1122_3344,
            prob: 0xa1b2_c3d4,
            values: [i32::MIN, -12_345, 0, 7, i32::MAX],
            max_level_by_set: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        };
        let encoded = record.encode();
        assert_eq!(encoded.len(), ITEM_ATTR_RECORD_WIRE_SIZE);
        assert_eq!(&encoded[0..33], &record.apply);
        assert_eq!(&encoded[33..37], &0x1122_3344_u32.to_le_bytes());
        assert_eq!(&encoded[37..41], &0xa1b2_c3d4_u32.to_le_bytes());
        assert_eq!(&encoded[41..45], &i32::MIN.to_le_bytes());
        assert_eq!(&encoded[45..49], &(-12_345_i32).to_le_bytes());
        assert_eq!(&encoded[57..61], &i32::MAX.to_le_bytes());
        assert_eq!(&encoded[61..71], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(ItemAttrRecord::decode(&encoded).unwrap(), record);
        assert_eq!(ItemAttrRecord::from_bytes(&encoded).unwrap(), record);
        assert_eq!(record.to_bytes(), encoded);
    }

    #[test]
    fn item_attr_rejects_every_truncation_and_trailing_bytes() {
        let mut apply = [0_u8; APPLY_NAME_MAX_LEN + 1];
        apply[..4].copy_from_slice(b"ATTR");
        let record = ItemAttrRecord {
            apply,
            apply_index: 1,
            prob: 2,
            values: [3, 4, 5, 6, 7],
            max_level_by_set: [8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
        };
        let encoded = record.encode();
        for available in 0..ITEM_ATTR_RECORD_WIRE_SIZE {
            assert!(matches!(
                ItemAttrRecord::decode(&encoded[..available]),
                Err(DbRecordError::Truncated {
                    record: "TItemAttrTable",
                    needed: ITEM_ATTR_RECORD_WIRE_SIZE,
                    available: _,
                })
            ));
        }
        let mut trailing = encoded;
        trailing.push(0);
        assert!(matches!(
            ItemAttrRecord::decode(&trailing),
            Err(DbRecordError::LengthMismatch {
                record: "TItemAttrTable",
                expected: ITEM_ATTR_RECORD_WIRE_SIZE,
                actual: 72,
            })
        ));
    }

    #[test]
    fn horse_name_pair_round_trips_with_exact_packed_offsets() {
        let request = HorseNameRequest::new(0x1122_3344);
        let request_bytes = request.encode();
        assert_eq!(request_bytes, vec![0x44, 0x33, 0x22, 0x11]);
        assert_eq!(HorseNameRequest::decode(&request_bytes).unwrap(), request);

        let mut name = [0_u8; LEGACY_CHARACTER_NAME_BYTES];
        name[..9].copy_from_slice(b"windhorse");
        // Keep a deterministic non-zero tail in this wire fixture. The
        // service does not infer or normalize legacy uninitialized bytes.
        name[9..17].copy_from_slice(&[0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18]);
        let record = HorseNameRecord::new(request.player_id, name);
        let record_bytes = record.encode();
        assert_eq!(record_bytes.len(), HORSE_NAME_RECORD_WIRE_SIZE);
        assert_eq!(&record_bytes[0..4], &0x1122_3344_u32.to_le_bytes());
        assert_eq!(&record_bytes[4..13], b"windhorse");
        assert_eq!(
            &record_bytes[13..21],
            &[0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18]
        );
        assert_eq!(HorseNameRecord::decode(&record_bytes).unwrap(), record);

        let mut request_short = request_bytes.clone();
        request_short.pop();
        assert!(matches!(
            HorseNameRequest::decode(&request_short),
            Err(DbRecordError::Truncated { .. })
        ));
        let mut request_long = request_bytes;
        request_long.push(0);
        assert!(matches!(
            HorseNameRequest::decode(&request_long),
            Err(DbRecordError::LengthMismatch { .. })
        ));

        let mut record_short = record_bytes.clone();
        record_short.pop();
        assert!(matches!(
            HorseNameRecord::decode(&record_short),
            Err(DbRecordError::Truncated { .. })
        ));
        let mut record_long = record_bytes;
        record_long.push(0);
        assert!(matches!(
            HorseNameRecord::decode(&record_long),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn channel_pair_round_trips_with_exact_packed_offsets() {
        let request = ChannelChangeRequest::new(0x0102_0304, 2);
        let request_bytes = request.encode();
        assert_eq!(
            request_bytes,
            vec![0x04, 0x03, 0x02, 0x01, 0x02, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            ChannelChangeRequest::decode(&request_bytes).unwrap(),
            request
        );
        assert!(request.is_valid());
        assert!(ChannelChangeRequest::new(-1, -1).is_valid());
        assert!(!ChannelChangeRequest::new(0, -1).is_valid());
        assert!(!ChannelChangeRequest::new(-1, 0).is_valid());

        let result = ChannelResultRecord::new(0x0100_007f, 0x1234);
        let result_bytes = result.encode();
        assert_eq!(result_bytes, vec![0x7f, 0x00, 0x00, 0x01, 0x34, 0x12]);
        assert_eq!(ChannelResultRecord::decode(&result_bytes).unwrap(), result);

        // `lAddr` is an x86 signed long. Preserve a high-bit raw value rather
        // than reinterpreting it through a Rust IPv4 address type.
        let high_bit = ChannelResultRecord::new(-1, u16::MAX);
        assert_eq!(high_bit.encode(), vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(
            ChannelResultRecord::decode(&high_bit.encode()).unwrap(),
            high_bit
        );

        let mut request_short = request_bytes.clone();
        request_short.pop();
        assert!(matches!(
            ChannelChangeRequest::decode(&request_short),
            Err(DbRecordError::Truncated { .. })
        ));
        let mut request_long = request_bytes;
        request_long.push(0);
        assert!(matches!(
            ChannelChangeRequest::decode(&request_long),
            Err(DbRecordError::LengthMismatch { .. })
        ));

        let mut result_short = result_bytes.clone();
        result_short.pop();
        assert!(matches!(
            ChannelResultRecord::decode(&result_short),
            Err(DbRecordError::Truncated { .. })
        ));
        let mut result_long = result_bytes;
        result_long.push(0);
        assert!(matches!(
            ChannelResultRecord::decode(&result_long),
            Err(DbRecordError::LengthMismatch { .. })
        ));

        assert!(!ChannelChangeRequest::new(0, 2).is_valid());
        assert!(!ChannelChangeRequest::new(1, 0).is_valid());
        assert_eq!(ChannelResultRecord::missing().encode(), [0; 6]);
    }

    #[test]
    fn horse_name_missing_response_is_all_zero_after_the_player_id() {
        let record = HorseNameRecord::missing(7);
        let encoded = record.encode();
        let mut expected = vec![7, 0, 0, 0];
        expected.extend_from_slice(&[0; 25]);
        assert_eq!(encoded, expected);
        assert_eq!(HorseNameRecord::decode(&encoded).unwrap(), record);
    }

    #[test]
    fn account_result_round_trip_and_golden_offsets() {
        let account = account_fixture();
        let encoded = account.encode();
        assert_eq!(&encoded[0..4], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&encoded[4..10], b"alice\0");
        assert_eq!(&encoded[35..43], b"secret\0\0");
        assert_eq!(encoded[80], 2);
        assert_eq!(encoded[361], 7);
        assert_eq!(LoginAccountRecord::decode(&encoded).unwrap(), account);
    }

    #[test]
    fn login_already_result_preserves_all_name_bytes() {
        let record = LoginAlreadyRecord {
            login: std::array::from_fn(|index| u8::try_from(index + 1).unwrap()),
        };
        let encoded = record.encode();
        assert_eq!(encoded.len(), LOGIN_ALREADY_WIRE_SIZE);
        assert_eq!(encoded[0..5], [1, 2, 3, 4, 5]);
        assert_eq!(LoginAlreadyRecord::decode(&encoded).unwrap(), record);
    }

    #[test]
    fn player_result_round_trip_and_golden_offsets() {
        let player = player_fixture();
        let encoded = player.encode();
        assert_eq!(&encoded[0..4], &[0x44, 0x33, 0x22, 0x11]);
        assert_eq!(&encoded[4..16], b"player-one\0\0");
        assert_eq!(&encoded[29..40], b"127.0.0.1\0\0");
        assert_eq!(&encoded[45..47], &[0x02, 0x00]);
        assert_eq!(
            &encoded[62..70],
            &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
        // The first battle-pass record follows the 1895-byte fixed prefix.
        assert_eq!(&encoded[1895..1897], &[0x64, 0x00]);
        assert_eq!(&encoded[2005..2007], &[0xcd, 0xab]);
        assert_eq!(PlayerResultRecord::decode(&encoded).unwrap(), player);
    }

    #[test]
    fn request_records_round_trip() {
        let mut login_bytes = [0_u8; LEGACY_LOGIN_BYTES];
        login_bytes[..5].copy_from_slice(b"alice");
        let mut ip_bytes = [0_u8; LEGACY_IP_BYTES];
        ip_bytes[..9].copy_from_slice(b"127.0.0.1");
        let login = LoginByKeyRequest {
            login: login_bytes,
            login_key: 0x1020_3040,
            client_key: [1, 0x0102_0304, 0x0506_0708, 0x090a_0b0c],
            ip: ip_bytes,
        };
        let encoded_login = login.encode();
        assert_eq!(&encoded_login[31..35], &[0x40, 0x30, 0x20, 0x10]);
        assert_eq!(&encoded_login[51..60], b"127.0.0.1");
        assert!(encoded_login[60..67].iter().all(|byte| *byte == 0));
        assert_eq!(LoginByKeyRequest::decode(&encoded_login).unwrap(), login);

        let load = PlayerLoadRequest {
            account_id: 0x0102_0304,
            player_id: 0x1112_1314,
            account_index: 2,
        };
        let load_bytes = load.encode();
        assert_eq!(
            load_bytes,
            vec![0x04, 0x03, 0x02, 0x01, 0x14, 0x13, 0x12, 0x11, 0x02]
        );
        assert_eq!(PlayerLoadRequest::decode(&load_bytes).unwrap(), load);
    }

    #[test]
    fn affect_load_constants_and_alias_are_stable() {
        assert_eq!(HEADER_DG_AFFECT_LOAD, 50);
        assert_eq!(AFFECT_LOAD_PREFIX_WIRE_SIZE, 8);
        assert_eq!(AffectElementRecord::WIRE_SIZE, 21);
        assert_eq!(AffectLoadResponse::PREFIX_WIRE_SIZE, 8);

        let response = AffectLoadResponse::new(7, Vec::new());
        let alias: AffectLoadResult = response.clone();
        assert_eq!(alias, response);
    }

    #[test]
    fn affect_load_empty_and_multi_payloads_have_golden_little_endian_bytes() {
        let empty = AffectLoadResponse::new(0x0102_0304, Vec::new());
        assert_eq!(
            empty.encode().unwrap(),
            vec![0x04, 0x03, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(decode_affect_load(&empty.encode().unwrap()).unwrap(), empty);
        assert_eq!(encode_affect_load(&empty).unwrap(), empty.encode().unwrap());

        let first = AffectElementRecord::new(
            0x5566_7788,
            0x99,
            -0x0102_0304,
            0xa1b2_c3d4,
            -0x0506_0708,
            0x090a_0b0c,
        );
        let second = AffectElementRecord::new(1, 0, 2, 3, -2, -1);
        let multi = AffectLoadResponse::new(0x1122_3344, vec![first, second]);
        let encoded = multi.encode().unwrap();
        assert_eq!(
            encoded,
            vec![
                0x44, 0x33, 0x22, 0x11, 0x02, 0x00, 0x00, 0x00, 0x88, 0x77, 0x66, 0x55, 0x99, 0xfc,
                0xfc, 0xfd, 0xfe, 0xd4, 0xc3, 0xb2, 0xa1, 0xf8, 0xf8, 0xf9, 0xfa, 0x0c, 0x0b, 0x0a,
                0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00,
                0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            ]
        );
        assert_eq!(encoded.len(), 8 + 2 * AFFECT_ELEMENT_WIRE_SIZE);
        assert_eq!(multi.encoded_len().unwrap(), encoded.len());
        assert_eq!(decode_affect_load(&encoded).unwrap(), multi);
        assert_eq!(decode_affect_load_with_limit(&encoded, 2).unwrap(), multi);
    }

    #[test]
    fn affect_load_round_trips_ordered_values_and_numeric_extrema() {
        let low =
            AffectElementRecord::new(u32::MAX, u8::MAX, i32::MIN, u32::MAX, i32::MAX, i32::MIN);
        let high = AffectElementRecord::new(0, 0, i32::MAX, 0, i32::MIN, 0);
        let response = AffectLoadResponse::new(u32::MAX, vec![low, high]);
        let encoded = response.to_bytes().unwrap();

        assert_eq!(encoded.len(), 8 + 2 * AFFECT_ELEMENT_WIRE_SIZE);
        assert_eq!(AffectLoadResponse::from_bytes(&encoded).unwrap(), response);
        assert_eq!(AffectLoadResponse::decode(&encoded).unwrap(), response);
        assert_eq!(response.encode().unwrap(), encoded);
        assert_eq!(response.encode_with_limit(2).unwrap(), encoded);
        assert_eq!(
            response.elements,
            vec![
                AffectElementRecord::new(u32::MAX, u8::MAX, i32::MIN, u32::MAX, i32::MAX, i32::MIN),
                AffectElementRecord::new(0, 0, i32::MAX, 0, i32::MIN, 0),
            ]
        );
    }

    #[test]
    fn affect_load_rejects_every_truncation_and_wrong_exact_length() {
        let response = AffectLoadResponse::new(
            0x0102_0304,
            vec![
                AffectElementRecord::default(),
                AffectElementRecord::default(),
            ],
        );
        let complete = response.encode().unwrap();
        let expected = 8 + 2 * AFFECT_ELEMENT_WIRE_SIZE;

        for available in 0..AFFECT_LOAD_PREFIX_WIRE_SIZE {
            assert_eq!(
                decode_affect_load(&complete[..available]),
                Err(DbRecordError::Truncated {
                    record: "HEADER_DG_AFFECT_LOAD",
                    needed: AFFECT_LOAD_PREFIX_WIRE_SIZE,
                    available,
                })
            );
        }
        for available in AFFECT_LOAD_PREFIX_WIRE_SIZE..expected {
            assert_eq!(
                decode_affect_load(&complete[..available]),
                Err(DbRecordError::Truncated {
                    record: "HEADER_DG_AFFECT_LOAD",
                    needed: expected,
                    available,
                })
            );
        }

        let mut trailing = complete.clone();
        trailing.push(0xaa);
        assert_eq!(
            decode_affect_load(&trailing),
            Err(DbRecordError::LengthMismatch {
                record: "HEADER_DG_AFFECT_LOAD",
                expected,
                actual: expected + 1,
            })
        );

        let mut wrong_count = complete;
        wrong_count[4..8].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            decode_affect_load(&wrong_count),
            Err(DbRecordError::LengthMismatch {
                record: "HEADER_DG_AFFECT_LOAD",
                expected: 8 + AFFECT_ELEMENT_WIRE_SIZE,
                actual: expected,
            })
        );
    }

    #[test]
    fn affect_load_limits_overflow_and_allocation_failures_are_distinct() {
        let response = AffectLoadResponse::new(
            1,
            vec![
                AffectElementRecord::default(),
                AffectElementRecord::default(),
            ],
        );
        assert_eq!(
            encode_affect_load_with_limit(&response, 1),
            Err(DbRecordError::TooManyAffectRecords {
                count: 2,
                maximum: 1,
            })
        );
        assert_eq!(
            decode_affect_load_with_limit(&response.encode().unwrap()[..8], 1),
            Err(DbRecordError::TooManyAffectRecords {
                count: 2,
                maximum: 1,
            })
        );

        assert_eq!(
            affect_wire_count(usize::MAX),
            Err(DbRecordError::AffectCountOverflow {
                count: affect_count_for_error(usize::MAX),
            })
        );
        assert_eq!(
            checked_affect_payload_size(usize::MAX),
            Err(DbRecordError::AffectPayloadSizeOverflow { count: usize::MAX })
        );
        assert_eq!(
            try_allocate_affect::<u8>(usize::MAX, usize::MAX).unwrap_err(),
            DbRecordError::AffectAllocationFailed { count: usize::MAX }
        );

        let mut huge_count = [0_u8; AFFECT_LOAD_PREFIX_WIRE_SIZE];
        huge_count[4..].copy_from_slice(&u32::MAX.to_le_bytes());
        match decode_affect_load(&huge_count) {
            Err(DbRecordError::Truncated {
                needed,
                available: 8,
                ..
            }) => {
                assert!(needed > AFFECT_LOAD_PREFIX_WIRE_SIZE);
            }
            Err(DbRecordError::AffectPayloadSizeOverflow { count }) => {
                assert_eq!(count, u32::MAX as usize);
            }
            other => panic!("unexpected huge-count result: {other:?}"),
        }
    }

    #[test]
    fn affect_load_survives_fragmented_and_coalesced_db_peer_frames() {
        use crate::db_wire::{DbFrame, DbFrameDecoder};

        let response = AffectLoadResponse::new(
            0x0102_0304,
            vec![AffectElementRecord::new(1, 2, 3, 4, 5, 6)],
        );
        let empty = AffectLoadResponse::new(u32::MAX, Vec::new());
        let first_frame = DbFrame::new(
            HEADER_DG_AFFECT_LOAD,
            0x1122_3344,
            response.encode().unwrap(),
        )
        .encode()
        .unwrap();
        let second_frame = DbFrame::new(HEADER_DG_AFFECT_LOAD, 0, empty.encode().unwrap())
            .encode()
            .unwrap();
        assert_eq!(
            &first_frame[..9],
            &[50, 0x44, 0x33, 0x22, 0x11, 29, 0, 0, 0]
        );
        assert_eq!(&second_frame[..9], &[50, 0, 0, 0, 0, 8, 0, 0, 0]);

        let mut coalesced = first_frame.clone();
        coalesced.extend_from_slice(&second_frame);
        let mut decoder = DbFrameDecoder::new();
        decoder.feed(&coalesced).unwrap();
        assert_eq!(
            decoder.try_decode().unwrap(),
            Some(DbFrame::new(
                HEADER_DG_AFFECT_LOAD,
                0x1122_3344,
                response.encode().unwrap(),
            ))
        );
        assert_eq!(
            decoder.try_decode().unwrap(),
            Some(DbFrame::new(
                HEADER_DG_AFFECT_LOAD,
                0,
                empty.encode().unwrap(),
            ))
        );
        assert!(decoder.try_decode().unwrap().is_none());
        assert_eq!(decoder.buffered_len(), 0);

        let mut fragmented = DbFrameDecoder::new();
        fragmented.feed(&first_frame[..7]).unwrap();
        assert!(fragmented.try_decode().unwrap().is_none());
        for byte in &first_frame[7..] {
            fragmented.feed(&[*byte]).unwrap();
        }
        let frame = fragmented.try_decode().unwrap().unwrap();
        assert_eq!(frame.header, HEADER_DG_AFFECT_LOAD);
        assert_eq!(frame.handle, 0x1122_3344);
        assert_eq!(decode_affect_load(&frame.payload).unwrap(), response);
        assert!(fragmented.try_decode().unwrap().is_none());
    }

    #[test]
    fn add_affect_request_round_trips_with_exact_headerless_payload() {
        let element = AffectElementRecord::new(
            0x5566_7788,
            0x99,
            -0x0102_0304,
            0xa1b2_c3d4,
            -0x0506_0708,
            0x090a_0b0c,
        );
        let request = AddAffectRequest::new(0x1122_3344, element);
        let encoded = request.encode();
        assert_eq!(
            encoded,
            vec![
                0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55, 0x99, 0xfc, 0xfc, 0xfd, 0xfe, 0xd4,
                0xc3, 0xb2, 0xa1, 0xf8, 0xf8, 0xf9, 0xfa, 0x0c, 0x0b, 0x0a, 0x09,
            ]
        );
        assert_eq!(&encoded[0..4], &[0x44, 0x33, 0x22, 0x11]);
        assert_eq!(&encoded[4..8], &[0x88, 0x77, 0x66, 0x55]);
        assert_eq!(encoded[8], 0x99);
        assert_eq!(&encoded[9..13], &[0xfc, 0xfc, 0xfd, 0xfe]);
        assert_eq!(&encoded[13..17], &[0xd4, 0xc3, 0xb2, 0xa1]);
        assert_eq!(&encoded[17..21], &[0xf8, 0xf8, 0xf9, 0xfa]);
        assert_eq!(&encoded[21..25], &[0x0c, 0x0b, 0x0a, 0x09]);
        assert_eq!(AddAffectRequest::decode(&encoded).unwrap(), request);
        assert_eq!(decode_add_affect_request(&encoded).unwrap(), request);
        assert_eq!(encode_add_affect_request(&request), encoded);
        assert_eq!(AffectElementRecord::WIRE_SIZE, 21);
        assert_eq!(AddAffectRequest::WIRE_SIZE, 25);
        assert_eq!(T_PACKET_AFFECT_ELEMENT_SIZE, 21);
        assert_eq!(T_PACKET_GD_ADD_AFFECT_SIZE, 25);
    }

    #[test]
    fn add_affect_request_round_trips_unsigned_and_signed_boundaries() {
        let element =
            AffectElementRecord::new(u32::MAX, u8::MAX, i32::MIN, u32::MAX, i32::MAX, i32::MIN);
        let request = AddAffectRequest::new(u32::MAX, element);
        let encoded = request.encode();
        assert_eq!(AddAffectRequest::decode(&encoded).unwrap(), request);

        let zero_element = AffectElementRecord::default();
        let zero_request = AddAffectRequest::new(0, zero_element);
        assert_eq!(
            AddAffectRequest::decode(&zero_request.encode()).unwrap(),
            zero_request
        );
    }

    #[test]
    fn add_affect_request_rejects_all_short_lengths_trailing_bytes_and_checks_length_first() {
        let complete = AddAffectRequest::new(1, AffectElementRecord::default()).encode();
        for available in 0..ADD_AFFECT_REQUEST_WIRE_SIZE {
            assert_eq!(
                AddAffectRequest::decode(&complete[..available]),
                Err(DbRecordError::Truncated {
                    record: "TPacketGDAddAffect",
                    needed: ADD_AFFECT_REQUEST_WIRE_SIZE,
                    available,
                })
            );
        }

        let mut long = complete;
        long.push(0xaa);
        assert_eq!(
            AddAffectRequest::decode(&long),
            Err(DbRecordError::LengthMismatch {
                record: "TPacketGDAddAffect",
                expected: ADD_AFFECT_REQUEST_WIRE_SIZE,
                actual: ADD_AFFECT_REQUEST_WIRE_SIZE + 1,
            })
        );

        // The exact-width check runs before any field read.
        assert_eq!(
            AddAffectRequest::decode(&[0_u8; ADD_AFFECT_REQUEST_WIRE_SIZE + 1]),
            Err(DbRecordError::LengthMismatch {
                record: "TPacketGDAddAffect",
                expected: ADD_AFFECT_REQUEST_WIRE_SIZE,
                actual: ADD_AFFECT_REQUEST_WIRE_SIZE + 1,
            })
        );
    }

    #[test]
    fn remove_affect_request_round_trips_with_exact_headerless_payload() {
        let request = RemoveAffectRequest::new(0x1122_3344, 0x5566_7788, 0x99);
        let encoded = request.encode();
        assert_eq!(
            encoded,
            vec![0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55, 0x99]
        );
        assert_eq!(RemoveAffectRequest::decode(&encoded).unwrap(), request);
        assert_eq!(decode_remove_affect_request(&encoded).unwrap(), request);
        assert_eq!(T_PACKET_REMOVE_AFFECT_SIZE, REMOVE_AFFECT_REQUEST_WIRE_SIZE);
    }

    #[test]
    fn remove_affect_request_rejects_all_short_lengths_and_trailing_bytes() {
        let complete = RemoveAffectRequest::new(1, 2, 3).encode();
        for available in 0..REMOVE_AFFECT_REQUEST_WIRE_SIZE {
            assert!(matches!(
                RemoveAffectRequest::decode(&complete[..available]),
                Err(DbRecordError::Truncated { .. })
            ));
        }
        let mut long = complete;
        long.push(0);
        assert!(matches!(
            RemoveAffectRequest::decode(&long),
            Err(DbRecordError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn remove_affect_request_survives_fragmented_db_peer_framing() {
        use crate::db_wire::{DbFrame, DbFrameDecoder};

        let request = RemoveAffectRequest::new(0x0102_0304, 0xaabb_ccdd, 0x7f);
        let encoded_frame = DbFrame::new(HEADER_GD_REMOVE_AFFECT, 0, request.encode())
            .encode()
            .unwrap();
        assert_eq!(encoded_frame.len(), 9 + REMOVE_AFFECT_REQUEST_WIRE_SIZE);
        let mut decoder = DbFrameDecoder::new();
        for byte in &encoded_frame {
            decoder.feed(&[*byte]).unwrap();
        }
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(frame.header, HEADER_GD_REMOVE_AFFECT);
        assert_eq!(frame.handle, 0);
        assert_eq!(
            RemoveAffectRequest::decode(&frame.payload).unwrap(),
            request
        );
        assert!(decoder.try_decode().unwrap().is_none());
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn quest_payload_has_explicit_count_and_fixed_records() {
        let mut first = QuestRecord::default();
        first.pid = 0x0102_0304;
        put_c_string(&mut first.name, "first_quest");
        put_c_string(&mut first.state, "state_a");
        first.value = -1234;
        let second = QuestRecord {
            pid: 9,
            ..QuestRecord::default()
        };
        let encoded = encode_quest_load(&[first, second]).unwrap();
        assert_eq!(&encoded[0..4], &[0x02, 0x00, 0x00, 0x00]);
        assert_eq!(&encoded[4..8], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(
            &encoded[4 + QUEST_RECORD_WIRE_SIZE..4 + QUEST_RECORD_WIRE_SIZE + 4],
            &[9, 0, 0, 0]
        );
        assert_eq!(decode_quest_load(&encoded).unwrap(), vec![first, second]);
        assert_eq!(
            decode_quest_load(&[0, 0, 0, 0]).unwrap(),
            Vec::<QuestRecord>::new()
        );
    }

    #[test]
    fn fixed_decoders_reject_truncation_and_trailing_bytes() {
        let account = account_fixture();
        let player = player_fixture();
        let quest = QuestRecord::default();

        for (label, bytes, decode) in [
            (
                "account",
                account.encode(),
                (|data: &[u8]| LoginAccountRecord::decode(data).map(|_| ()))
                    as fn(&[u8]) -> DbRecordResult<()>,
            ),
            (
                "player",
                player.encode(),
                (|data: &[u8]| PlayerResultRecord::decode(data).map(|_| ()))
                    as fn(&[u8]) -> DbRecordResult<()>,
            ),
            (
                "quest",
                quest.encode(),
                (|data: &[u8]| QuestRecord::decode(data).map(|_| ()))
                    as fn(&[u8]) -> DbRecordResult<()>,
            ),
        ] {
            for length in 0..bytes.len() {
                assert!(
                    matches!(
                        decode(&bytes[..length]),
                        Err(DbRecordError::Truncated { .. })
                    ),
                    "{label} should reject truncation at {length}"
                );
            }
            let mut extra = bytes;
            extra.push(0);
            assert!(
                matches!(decode(&extra), Err(DbRecordError::LengthMismatch { .. })),
                "{label} should reject trailing bytes"
            );
        }
    }

    #[test]
    fn quest_decoder_checks_count_before_allocating_and_rejects_bad_lengths() {
        assert!(matches!(
            decode_quest_load(&u32::MAX.to_le_bytes()),
            Err(DbRecordError::TooManyQuestRecords { .. })
        ));
        assert!(matches!(
            decode_quest_load(&[1, 0, 0, 0]),
            Err(DbRecordError::Truncated { .. })
        ));
        assert!(matches!(
            decode_quest_load(&[0, 0, 0, 0, 0]),
            Err(DbRecordError::LengthMismatch { .. })
        ));
        assert!(matches!(
            decode_quest_load(
                &[1, 0, 0, 0]
                    .iter()
                    .copied()
                    .chain(std::iter::repeat(0).take(105))
                    .collect::<Vec<_>>()
            ),
            Err(DbRecordError::Truncated { .. })
        ));
        assert!(matches!(
            decode_quest_load_with_limit(&[1, 0, 0, 0], 0),
            Err(DbRecordError::TooManyQuestRecords {
                count: 1,
                maximum: 0
            })
        ));
    }
}
