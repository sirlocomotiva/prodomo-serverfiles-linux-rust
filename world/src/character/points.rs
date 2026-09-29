//! The points of a player character: the two point arrays, the three pools, and the legacy
//! rules that change them.
//!
//! Legacy `CHARACTER` keeps two arrays of 255 `int` slots. The *real* array
//! (`m_points.points`) holds what the character is: its four attributes, its base maximum hit
//! and spell points, its stat and skill points. The *instant* array (`m_pointsInstant.points`)
//! holds what the character has right now, with every bonus added, and it is the array
//! `GetPoint` reads and `GC_CHARACTER_POINTS` sends. The current hit points, spell points and
//! stamina, and their maxima, live beside the arrays rather than in them.
//!
//! This module ports the rules that fill and change them:
//!
//! | legacy | here |
//! |---|---|
//! | `CHARACTER::PointChange` (`char.cpp:3872-4822`) | [`Points::point_change`] |
//! | `CHARACTER::ComputeBattlePoints` (`char.cpp:2686-2860`) | [`Points::compute_battle_points`] |
//! | `CHARACTER::ComputePoints` (`char.cpp:2862-3160`) | [`Points::compute_points_with`] |
//! | `CHARACTER::ApplyPoint` (`char.cpp:4826-5043`) | [`Points::apply_point`] |
//! | `CHARACTER::CheckMaximumPoints` (`char.cpp:3863-3870`) | [`Points::check_maximum_points`] |
//! | `CHARACTER::GetLimitPoint` (`char.cpp:3730-3799`) | [`Points::limit_point`] |
//! | `CHARACTER::GetMaxHP` (`char.cpp:8430-8436`) | [`Points::max_hp`] |
//! | `CHARACTER::GetSungMaWill` (`char.cpp:11717-11743`) | [`Points::will`] |
//! | the points half of `CHARACTER::SetPlayerProto` (`char.cpp:2233-2420`) | [`Points::load`] |
//!
//! # The records a change sends
//!
//! Every `PointChange` that reaches its end writes one `GC_CHARACTER_POINT_CHANGE` (17) to the
//! character's descriptor. A change that makes another change first (`POINT_DEF_GRADE` also
//! changes `POINT_CLIENT_DEF_GRADE`, an attribute recomputes the battle points) writes the
//! inner record before its own, because the inner call returns before the outer one reaches
//! its send. [`PointRecord`] is one such record, and every method here returns them in the
//! order legacy writes them. The caller owns the descriptor and the VID, so it encodes them.
//!
//! # Where the Rewrite differs from legacy, and why
//!
//! - **One record per change, not two.** `PointChange` writes the record, and then calls
//!   `UpdatePointsPacket`, which writes the same record a second time (`char.cpp:4803-4822`
//!   and `2089-2118`). The second copy differs from the first only for `POINT_MOV_SPEED`,
//!   which it halves when the map's movement will exceeds the character's. So under that
//!   rule the client is first told a speed it does not have. That is a Defect. The Rewrite
//!   writes one record, carrying the value the client ends on in legacy: the second one.
//! - **The base maxima are always stored.** `ComputePoints` stores the base maximum hit points
//!   in the real array only `if (iMaxHP != GetMaxHP())` (`char.cpp:3050-3062`), comparing
//!   the new base with the old *total*. When the old total happens to equal the new base, the
//!   stale base from the previous computation is kept, and every bonus is then added to the
//!   wrong base. That is a Defect. The Rewrite always stores the new base, for spell points
//!   too.
//! - **Arithmetic does not wrap.** Legacy computes in `int` and would wrap past
//!   `i32::MAX`. The Rewrite computes in 64 bits and saturates on the narrowing, which no
//!   value a client can reach comes near.
//!
//! # Not ported yet
//!
//! Each arm below either needs a system the Rewrite does not have, or changes state the
//! Rewrite does not store. [`Points::point_change`] refuses them with
//! [`PointChangeRefused::NotPorted`], changing nothing and writing no record, so a caller can
//! never believe a level-up happened.
//!
//! - `POINT_LEVEL`, `POINT_EXP`, `POINT_LEVEL_STEP`, `POINT_CONQUEROR_LEVEL`,
//!   `POINT_CONQUEROR_EXP` and `POINT_CONQUEROR_LEVEL_STEP`: the level-up chain, with its
//!   quest trigger, guild and party updates, and save.
//! - `POINT_GOLD`, `POINT_GAYA` and `POINT_INVEN`: the currencies and the inventory unlocks.
//! - `POINT_POLYMORPH`, `POINT_MOUNT`, `POINT_ENERGY`, `POINT_COSTUME_ATTR_BONUS`, the three
//!   biologist arms and the two protected-inventory arms: each drives its own system.
//!
//! The random hit and spell points a level-up adds (`iRandomHP` and `iRandomSP`) are 0: they
//! grow only at a level-up, and the store gains their columns when the level-up is ported.
//!
//! Also not ported: the `IsDead() || IsStun()` guard on the three pools (the Rewrite has no
//! death yet), the target and party broadcasts after a hit-point change, the walking switch
//! after a stamina change, the polymorph and mount branches of the battle points, and the
//! parts of `ComputePoints` that read affects, skills, the horse, the dragon-soul deck and the
//! attribute buffs (`BuffOnAttr`). The passive-skill bonuses are an input
//! ([`Points::set_passive_bonuses`]) so the skill system can supply them; today they are zero,
//! which is exact for a character with no skill level. The worn items are computed: see
//! [`Points::compute_points_with`].
//!
//! `ComputePoints` also ORs each worn item's `dwImmuneFlag` into the character's immunity bits
//! (`char.cpp:3078`). The two use different bit orders: the proto's are the `IMMUNE` column's
//! names in `ProtoReader.cpp` order (paralysis, curse, stun, sleep, slow, poison, terror), the
//! character's are `IMMUNE_STUN`, `IMMUNE_SLOW`, `IMMUNE_FALL` and so on (`length.h:713-722`),
//! so an item immune to stun makes its wearer immune to falling. That is a Defect. No item in
//! the owner's proto sets the column, and the item load refuses one that does, so the
//! computation here never meets one and ORs nothing.

use std::collections::BTreeMap;

use common::cfloat::{f32_to_i32, i32_to_f32};
use common::enums::EParts;
use common::levels::{self, JobInitialPoints};
use common::point_slot as point;

use super::apply::ApplyArm;
use super::equipment::{Equipment, PARTS};

/// The slots in each point array (`POINT_MAX_NUM`).
const SLOTS: usize = point::POINT_MAX_NUM;

/// `IMMUNE_STUN`, `IMMUNE_SLOW` and `IMMUNE_FALL` (`length.h:715-717`).
pub const IMMUNE_STUN: u32 = 1 << 0;
/// See [`IMMUNE_STUN`].
pub const IMMUNE_SLOW: u32 = 1 << 1;
/// See [`IMMUNE_STUN`].
pub const IMMUNE_FALL: u32 = 1 << 2;

/// The four conqueror wills a map demands (`SSungMaWill`, `constants.h:202-205`).
///
/// A character whose own sungma point is below a map's will is weakened on that map: half its
/// maximum hit points for `hp`, half its movement speed for `movement`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SungmaWill {
    /// `str`, compared with `POINT_SUNGMA_STR`.
    pub strength: u8,
    /// `hp`, compared with `POINT_SUNGMA_HP`.
    pub hp: u8,
    /// `move`, compared with `POINT_SUNGMA_MOVE`.
    pub movement: u8,
    /// `immune`, compared with `POINT_SUNGMA_IMMUNE`.
    pub immune: u8,
}

/// `SungMaWillMap` (`constants.cpp:1351-1372`): the maps that demand a will, by map index.
pub const SUNGMA_WILL_MAPS: [(i32, SungmaWill); 7] = [
    (374, will(0, 0, 1, 0)),
    (373, will(15, 10, 15, 20)),
    (376, will(20, 10, 20, 25)),
    (377, will(25, 15, 25, 30)),
    (382, will(25, 15, 20, 25)),
    (383, will(55, 35, 45, 45)),
    (384, will(75, 60, 65, 65)),
];

const fn will(strength: u8, hp: u8, movement: u8, immune: u8) -> SungmaWill {
    SungmaWill {
        strength,
        hp,
        movement,
        immune,
    }
}

/// The will a map demands, or no will at all for a map `SungMaWillMap` does not list.
#[must_use]
pub fn sungma_will(map_index: i32) -> SungmaWill {
    SUNGMA_WILL_MAPS
        .iter()
        .find(|(index, _)| *index == map_index)
        .map_or_else(SungmaWill::default, |(_, will)| *will)
}

/// The job of a race, as `RaceToJob` (`input_login.cpp:354-395`) and `GetJob` answer it: the
/// eight main races map onto the four jobs, and anything else is `JOB_WARRIOR`.
#[must_use]
pub fn race_to_job(race: u8) -> u8 {
    if race < 8 {
        race % 4
    } else {
        0
    }
}

/// One `GC_CHARACTER_POINT_CHANGE` (17) a change writes (`packet_point_change`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointRecord {
    /// The point slot, `type`.
    pub kind: u8,
    /// `amount`: the change, when the caller asked to show it, else 0.
    pub amount: i64,
    /// `value`: the slot's value after the change.
    pub value: i64,
    /// Whether legacy writes it with `PacketAround` (to every character in view, this one
    /// included) rather than to this character alone.
    pub broadcast: bool,
}

/// Why [`Points::point_change`] changed nothing. Either way no state changed and no record
/// was written, which is also what legacy's unknown-type arm does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointChangeRefused {
    /// Legacy's `switch` has no arm for the slot; it logs and returns.
    Unknown(usize),
    /// The slot has an arm whose side effects the Rewrite has not ported. See the module
    /// note for the list.
    NotPorted(usize),
}

/// The evaluated `kPointPoly` of the four passive skills `ComputePoints` reads.
///
/// Legacy evaluates each skill's formula with `k` set to the character's skill power
/// (`char.cpp:2956-3010`). The formulas are data (`skill_proto.szPointPoly`) and the power
/// comes from the skill level, so the skill system supplies the results. Every power at level
/// 0 is 0 (`SKILL_POWER_BY_LEVEL` starts with 0 in `legacy/sql/gamedata/common.sql`), and
/// each formula is `k` times a constant, so a character with no skill level has all four at 0,
/// the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassiveBonuses {
    /// `SKILL_ADD_HP` (141), added to the base maximum hit points.
    pub max_hp: i32,
    /// `SKILL_MONSTER_BONUS` (164), added to `POINT_ATTBONUS_MONSTER`.
    pub monster: i32,
    /// `SKILL_STONE_BONUS` (165), added to `POINT_ATTBONUS_METIN`.
    pub stone: i32,
    /// `SKILL_BOSS_BONUS` (166), added to `POINT_ATTBONUS_BOSS`.
    pub boss: i32,
}

/// What `SetPlayerProto` reads into the points from a stored character.
///
/// The columns the Rewrite does not store yet are absent and read as 0, as a fresh legacy
/// row has them: the random hit and spell points, the stat, skill and horse-skill points,
/// the stat-reset count, the level step, the conqueror point and level step, the gaya, and
/// the private-shop slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointsRow {
    /// The race, 0 to 7.
    pub race: u8,
    /// The level.
    pub level: u8,
    /// The conqueror level; 0 turns every sungma will off.
    pub conqueror_level: u8,
    /// Strength.
    pub st: u8,
    /// Vitality.
    pub ht: u8,
    /// Dexterity.
    pub dx: u8,
    /// Intelligence.
    pub iq: u8,
    /// The four stored sungma points: strength, hit points, movement, immunity.
    pub sungma: [u8; 4],
    /// The stored hit points.
    pub hp: i32,
    /// The stored spell points.
    pub sp: i32,
    /// The stored stamina.
    pub stamina: i32,
    /// `Inven_Point`, the inventory pages unlocked.
    pub inven_point: u16,
    /// The map the character stands on, which picks the sungma will.
    pub map_index: i32,
    /// `part_base`, the body shape the main part falls back to when no armour is worn.
    pub part_base: u8,
    /// The stored hair part, which a hair costume replaces.
    pub hair_part: u16,
    /// The stored sash part, which a sash replaces.
    pub sash_part: u16,
}

/// The four sungma slots, in the order of [`PointsRow::sungma`].
const SUNGMA_SLOTS: [usize; 4] = [
    point::POINT_SUNGMA_STR,
    point::POINT_SUNGMA_HP,
    point::POINT_SUNGMA_MOVE,
    point::POINT_SUNGMA_IMMUNE,
];

/// The slots `PointChange` simply adds to (`char.cpp:4448-4595`), in source order, with the
/// four sungma slots, whose arm has the same body.
///
/// `ACCEDRAIN_RATE` and `RESIST_MAGIC_REDUCTION` sit in the same list behind switches this
/// tree does not define, so neither is a slot here. Five attack bonuses are **not** in the
/// list: `POINT_ATTBONUS_INSECT`, `_FIRE`, `_ICE`, `_DESERT` and `_TREE` fall to the
/// unknown-type arm.
const PLAIN: [usize; 119] = [
    point::POINT_ST,
    point::POINT_HT,
    point::POINT_DX,
    point::POINT_IQ,
    point::POINT_HP_REGEN,
    point::POINT_SP_REGEN,
    point::POINT_ATT_SPEED,
    point::POINT_ATT_GRADE,
    point::POINT_MOV_SPEED,
    point::POINT_CASTING_SPEED,
    point::POINT_MAGIC_ATT_GRADE,
    point::POINT_MAGIC_DEF_GRADE,
    point::POINT_BOW_DISTANCE,
    point::POINT_HP_RECOVERY,
    point::POINT_SP_RECOVERY,
    point::POINT_ATTBONUS_HUMAN,
    point::POINT_ATTBONUS_ANIMAL,
    point::POINT_ATTBONUS_ORC,
    point::POINT_ATTBONUS_MILGYO,
    point::POINT_ATTBONUS_UNDEAD,
    point::POINT_ATTBONUS_DEVIL,
    point::POINT_ATTBONUS_MONSTER,
    point::POINT_ATTBONUS_SURA,
    point::POINT_ATTBONUS_ASSASSIN,
    point::POINT_ATTBONUS_WARRIOR,
    point::POINT_ATTBONUS_SHAMAN,
    point::POINT_POISON_PCT,
    point::POINT_STUN_PCT,
    point::POINT_SLOW_PCT,
    point::POINT_BLOCK,
    point::POINT_DODGE,
    point::POINT_CRITICAL_PCT,
    point::POINT_RESIST_CRITICAL,
    point::POINT_PENETRATE_PCT,
    point::POINT_RESIST_PENETRATE,
    point::POINT_ATTBONUS_METIN,
    point::POINT_ATTBONUS_BOSS,
    point::POINT_CURSE_PCT,
    point::POINT_STEAL_HP,
    point::POINT_STEAL_SP,
    point::POINT_MANA_BURN_PCT,
    point::POINT_DAMAGE_SP_RECOVER,
    point::POINT_RESIST_NORMAL_DAMAGE,
    point::POINT_RESIST_SWORD,
    point::POINT_RESIST_TWOHAND,
    point::POINT_RESIST_DAGGER,
    point::POINT_RESIST_BELL,
    point::POINT_RESIST_FAN,
    point::POINT_RESIST_BOW,
    point::POINT_RESIST_FIRE,
    point::POINT_RESIST_ELEC,
    point::POINT_RESIST_MAGIC,
    point::POINT_RESIST_WIND,
    point::POINT_RESIST_ICE,
    point::POINT_RESIST_EARTH,
    point::POINT_RESIST_DARK,
    point::POINT_REFLECT_MELEE,
    point::POINT_REFLECT_CURSE,
    point::POINT_POISON_REDUCE,
    point::POINT_KILL_SP_RECOVER,
    point::POINT_KILL_HP_RECOVERY,
    point::POINT_HIT_HP_RECOVERY,
    point::POINT_HIT_SP_RECOVERY,
    point::POINT_MANASHIELD,
    point::POINT_ATT_BONUS,
    point::POINT_DEF_BONUS,
    point::POINT_SKILL_DAMAGE_BONUS,
    point::POINT_NORMAL_HIT_DAMAGE_BONUS,
    point::POINT_ENCHANT_ELECT,
    point::POINT_ENCHANT_FIRE,
    point::POINT_ENCHANT_ICE,
    point::POINT_ENCHANT_WIND,
    point::POINT_ENCHANT_EARTH,
    point::POINT_ENCHANT_DARK,
    point::POINT_ATTBONUS_ANIMAL_PCT,
    point::POINT_ATTBONUS_UNDEAD_PCT,
    point::POINT_ATTBONUS_DEVIL_PCT,
    point::POINT_ATTBONUS_ORC_PCT,
    point::POINT_ATTBONUS_MILGYO_PCT,
    point::POINT_ATTBONUS_DESERT_PCT,
    point::POINT_ATTBONUS_INSECT_PCT,
    point::POINT_ATTBONUS_TREE_PCT,
    point::POINT_ATTBONUS_BOSS_PCT,
    point::POINT_ATTBONUS_METIN_PCT,
    point::POINT_ATTBONUS_CZ_PCT,
    point::POINT_ATTBONUS_HUMAN_PCT,
    point::POINT_ATTBONUS_MONSTER_PCT,
    point::POINT_ENCHANT_ELECT_PCT,
    point::POINT_ENCHANT_FIRE_PCT,
    point::POINT_ENCHANT_ICE_PCT,
    point::POINT_ENCHANT_WIND_PCT,
    point::POINT_ENCHANT_EARTH_PCT,
    point::POINT_ENCHANT_DARK_PCT,
    point::POINT_RESIST_ELECT_PCT,
    point::POINT_RESIST_FIRE_PCT,
    point::POINT_RESIST_ICE_PCT,
    point::POINT_RESIST_WIND_PCT,
    point::POINT_RESIST_EARTH_PCT,
    point::POINT_RESIST_DARK_PCT,
    point::POINT_RESIST_HUMAN_PCT,
    point::POINT_RESIST_FALL,
    point::POINT_RESIST_COMBAT,
    point::POINT_PRIVATE_SHOP_UNLOCKED_SLOT,
    point::POINT_SKILL_DEFEND_BONUS,
    point::POINT_NORMAL_HIT_DEFEND_BONUS,
    point::POINT_PARTY_ATTACKER_BONUS,
    point::POINT_PARTY_TANKER_BONUS,
    point::POINT_PARTY_BUFFER_BONUS,
    point::POINT_PARTY_SKILL_MASTER_BONUS,
    point::POINT_PARTY_HASTE_BONUS,
    point::POINT_PARTY_DEFENDER_BONUS,
    point::POINT_RESIST_WARRIOR,
    point::POINT_RESIST_ASSASSIN,
    point::POINT_RESIST_SURA,
    point::POINT_RESIST_SHAMAN,
    point::POINT_SUNGMA_STR,
    point::POINT_SUNGMA_HP,
    point::POINT_SUNGMA_MOVE,
    point::POINT_SUNGMA_IMMUNE,
];

/// The slots `PointChange` caps at 100 (`char.cpp:4597-4632`).
const CAPPED: [usize; 10] = [
    point::POINT_MALL_ATTBONUS,
    point::POINT_MALL_DEFBONUS,
    point::POINT_MALL_EXPBONUS,
    point::POINT_MALL_ITEMBONUS,
    point::POINT_MALL_GOLDBONUS,
    point::POINT_MELEE_MAGIC_ATT_BONUS_PER,
    point::POINT_EXP_DOUBLE_BONUS,
    point::POINT_GOLD_DOUBLE_BONUS,
    point::POINT_ITEM_DROP_BONUS,
    point::POINT_POTION_BONUS,
];

/// The counters whose change is written to both arrays (`char.cpp:4425-4434`).
const COUNTERS: [usize; 5] = [
    point::POINT_SKILL,
    point::POINT_STAT,
    point::POINT_SUB_SKILL,
    point::POINT_STAT_RESET_COUNT,
    point::POINT_HORSE_SKILL,
];

/// The arms whose side effects are not ported. See the module note.
const NOT_PORTED: [usize; 18] = [
    point::POINT_CONQUEROR_LEVEL,
    point::POINT_CONQUEROR_EXP,
    point::POINT_CONQUEROR_LEVEL_STEP,
    point::POINT_LEVEL,
    point::POINT_EXP,
    point::POINT_LEVEL_STEP,
    point::POINT_INVEN,
    point::POINT_GAYA,
    point::POINT_GOLD,
    point::POINT_POLYMORPH,
    point::POINT_MOUNT,
    point::POINT_ENERGY,
    point::POINT_COSTUME_ATTR_BONUS,
    point::POINT_BIOLOGIST_STATE,
    point::POINT_BIOLOGIST_ITEMS_TAKEN,
    point::POINT_BIOLOGIST_COMPLETED,
    point::POINT_SECURED_STATE,
    point::POINT_SECURED_PASSWORD,
];

/// The slots whose change recomputes the battle points (`char.cpp:4778-4797`).
const RECOMPUTES_BATTLE: [usize; 10] = [
    point::POINT_LEVEL,
    point::POINT_ST,
    point::POINT_DX,
    point::POINT_IQ,
    point::POINT_HT,
    point::POINT_CONQUEROR_LEVEL,
    point::POINT_SUNGMA_STR,
    point::POINT_SUNGMA_HP,
    point::POINT_SUNGMA_MOVE,
    point::POINT_SUNGMA_IMMUNE,
];

/// The two slots whose record carries the next level's cost and never the change.
const NEXT_EXP_SLOTS: [usize; 2] = [point::POINT_NEXT_EXP, point::POINT_CONQUEROR_NEXT_EXP];

/// What one `PointChange` arm did.
enum Applied {
    /// The arm ran; the record carries this amount and value.
    Changed { amount: i32, value: i64 },
    /// The arm ran and legacy returns before writing a record.
    Silent,
    /// The slot has no arm in this group.
    NotPool,
}

/// The points of one player character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Points {
    race: u8,
    level: u8,
    conqueror_level: u8,
    real: [i32; SLOTS],
    instant: [i32; SLOTS],
    hp: i32,
    sp: i32,
    stamina: i32,
    max_hp: i32,
    max_sp: i32,
    max_stamina: i32,
    random_hp: i32,
    random_sp: i32,
    inven_point: i32,
    immune_flag: u32,
    map_will: SungmaWill,
    armour: i32,
    passive: PassiveBonuses,
    skill_damage_bonus: BTreeMap<u8, i32>,
    part_base: u8,
    parts: [u16; PARTS],
}

impl Points {
    /// The points of a stored character, as `SetPlayerProto` leaves them.
    ///
    /// Legacy copies the attributes and the sungma points into both arrays, runs
    /// `ComputePoints`, and only then copies the stored hit points, spell points and stamina
    /// in, unclamped (`char.cpp:2296-2356`). So a stored value above the new maximum
    /// survives the load and is clamped later, by the item load's `CheckMaximumPoints`.
    ///
    /// `ComputePoints` writes point-change records here too, while the descriptor is still in
    /// the select phase. They are not returned, which is a recorded Divergence: what the
    /// client's select-phase reader of byte 17 does with them was not recorded (ledger 151.6),
    /// and every slot they set is set again, to its final value, by the loading burst's points
    /// record. The play test calibrates it.
    #[must_use]
    pub fn load(row: &PointsRow) -> Self {
        let mut points = Self {
            race: row.race,
            level: row.level,
            conqueror_level: row.conqueror_level,
            real: [0; SLOTS],
            instant: [0; SLOTS],
            hp: 0,
            sp: 0,
            stamina: 0,
            max_hp: 0,
            max_sp: 0,
            max_stamina: 0,
            random_hp: 0,
            random_sp: 0,
            inven_point: i32::from(row.inven_point),
            immune_flag: 0,
            map_will: sungma_will(row.map_index),
            armour: 0,
            passive: PassiveBonuses::default(),
            skill_damage_bonus: BTreeMap::new(),
            part_base: row.part_base,
            parts: [0; PARTS],
        };
        // `SetPlayerProto` sets the base part, the hair and the sash from the row before it
        // computes (`char.cpp:2258-2265`); the computation then puts the base part in the
        // main part. The aura part is not stored, so it starts at 0.
        points.parts[EParts::Hair as usize] = row.hair_part;
        points.parts[EParts::Sash as usize] = row.sash_part;
        for (slot, value) in [
            (point::POINT_ST, row.st),
            (point::POINT_HT, row.ht),
            (point::POINT_DX, row.dx),
            (point::POINT_IQ, row.iq),
        ] {
            points.real[slot] = i32::from(value);
            points.instant[slot] = i32::from(value);
        }
        for (slot, value) in SUNGMA_SLOTS.into_iter().zip(row.sungma) {
            points.real[slot] = i32::from(value);
            points.instant[slot] = i32::from(value);
        }
        let _discarded = points.compute_points();
        points.hp = row.hp;
        points.sp = row.sp;
        points.stamina = row.stamina;
        points
    }

    /// The six parts the client draws, in `EParts` order, as the last computation left them.
    #[must_use]
    pub fn parts(&self) -> [u16; PARTS] {
        self.parts
    }

    /// The level the battle points are computed from.
    #[must_use]
    pub fn level(&self) -> u8 {
        self.level
    }

    /// `GetPoint`: the instant slot, or 0 for a slot past the array, as legacy answers it.
    #[must_use]
    pub fn get_point(&self, kind: usize) -> i32 {
        self.instant.get(kind).copied().unwrap_or(0)
    }

    /// `GetRealPoint`: the real slot, or 0 for a slot past the array.
    #[must_use]
    pub fn real_point(&self, kind: usize) -> i32 {
        self.real.get(kind).copied().unwrap_or(0)
    }

    /// `GetLimitPoint` for a player character (`char.cpp:3725-3790`).
    ///
    /// Attack speed is held to 0..170 and movement speed to 0..200; life and mana steal to at
    /// most 50; the item-shop attack and defence bonuses to at most 20. Every other slot is
    /// held above `-INT_MAX` only. Movement speed is halved first when the map's movement will
    /// exceeds the character's.
    #[must_use]
    pub fn limit_point(&self, kind: usize) -> i32 {
        let Some(&value) = self.instant.get(kind) else {
            return 0;
        };
        let (low, high) = match kind {
            point::POINT_ATT_SPEED => (0, 170),
            point::POINT_MOV_SPEED => (0, 200),
            point::POINT_STEAL_HP | point::POINT_STEAL_SP => (-i32::MAX, 50),
            point::POINT_MALL_ATTBONUS | point::POINT_MALL_DEFBONUS => (-i32::MAX, 20),
            _ => (-i32::MAX, i32::MAX),
        };
        let value = if kind == point::POINT_MOV_SPEED && self.movement_is_weakened() {
            value / 2
        } else {
            value
        };
        value.clamp(low, high)
    }

    /// The current hit points.
    #[must_use]
    pub fn hp(&self) -> i32 {
        self.hp
    }

    /// The current spell points.
    #[must_use]
    pub fn sp(&self) -> i32 {
        self.sp
    }

    /// The current stamina.
    #[must_use]
    pub fn stamina(&self) -> i32 {
        self.stamina
    }

    /// `GetMaxHP`: the maximum hit points, halved when the map's hit-point will exceeds the
    /// character's and the maximum is positive (`char.cpp:8430-8437`).
    #[must_use]
    pub fn max_hp(&self) -> i32 {
        let will = i32::from(self.will().hp);
        if will > self.get_point(point::POINT_SUNGMA_HP) && self.max_hp > 0 {
            self.max_hp / 2
        } else {
            self.max_hp
        }
    }

    /// `GetMaxSP`: the maximum spell points.
    #[must_use]
    pub fn max_sp(&self) -> i32 {
        self.max_sp
    }

    /// `GetMaxStamina`: the maximum stamina.
    #[must_use]
    pub fn max_stamina(&self) -> i32 {
        self.max_stamina
    }

    /// `Inven_Point`: the inventory pages unlocked.
    #[must_use]
    pub fn inven_point(&self) -> i32 {
        self.inven_point
    }

    /// The `IMMUNE_*` bits the immunity slots have set.
    #[must_use]
    pub fn immune_flag(&self) -> u32 {
        self.immune_flag
    }

    /// `GetSungMaWill`: the will the character's map demands, or none for a character with
    /// no conqueror level (`char.cpp:11717-11743`).
    #[must_use]
    pub fn will(&self) -> SungmaWill {
        if self.conqueror_level == 0 {
            SungmaWill::default()
        } else {
            self.map_will
        }
    }

    /// Move the character to another map, which changes the will it is measured against.
    pub fn set_map_index(&mut self, map_index: i32) {
        self.map_will = sungma_will(map_index);
    }

    /// `m_SkillDamageBonus`: the sum of the `APPLY_SKILL` changes the worn items and set
    /// bonuses make to one skill, or 0 for a skill none of them names.
    #[must_use]
    pub fn skill_damage_bonus(&self, skill: u8) -> i32 {
        self.skill_damage_bonus.get(&skill).copied().unwrap_or(0)
    }

    /// The passive-skill bonuses, which take effect at the next [`Points::compute_points`].
    pub fn set_passive_bonuses(&mut self, passive: PassiveBonuses) {
        self.passive = passive;
    }

    /// `PointChange(type, amount, bAmount, bBroadcast)`.
    ///
    /// `show_amount` is `bAmount`: whether the record carries the change or 0. `broadcast`
    /// is `bBroadcast`. The records come back in the order legacy writes them.
    ///
    /// # Errors
    ///
    /// Returns [`PointChangeRefused`] for a slot legacy has no arm for, and for an arm the
    /// Rewrite has not ported. Either way nothing changed.
    pub fn point_change(
        &mut self,
        kind: usize,
        amount: i32,
        show_amount: bool,
        broadcast: bool,
    ) -> Result<Vec<PointRecord>, PointChangeRefused> {
        let mut records = Vec::new();
        match self.change(kind, amount, show_amount, broadcast, &mut records) {
            Some(refused) => Err(refused),
            None => Ok(records),
        }
    }

    /// `ComputeBattlePoints` for a player character that is neither polymorphed nor mounted.
    ///
    /// The attack grade is twice the level plus the job's attribute term; the defence grade
    /// is the level plus four fifths of the vitality plus the armour; the grade the client
    /// shows is the level plus the whole vitality plus the armour; the magic grades follow
    /// the intelligence (`char.cpp:2709-2847`).
    pub fn compute_battle_points(&mut self) -> Vec<PointRecord> {
        let mut records = Vec::new();
        self.battle_points(&mut records);
        records
    }

    /// [`Points::compute_points_with`] for a character that wears nothing.
    pub fn compute_points(&mut self) -> Vec<PointRecord> {
        self.compute_points_with(&Equipment::default())
    }

    /// `ComputePoints` for a player character, as far as the Rewrite has the systems it reads.
    ///
    /// Clears the instant array and the skill-damage bonuses, keeping the counters, the party
    /// bonuses and the recovery slots; recomputes the base maxima and speeds, the
    /// passive-skill bonuses and the battle points with the worn armour; applies every worn
    /// item's bonuses in wear-cell order, then the set bonuses; and clamps the hit and spell
    /// points to the new maxima.
    pub fn compute_points_with(&mut self, equipment: &Equipment<'_>) -> Vec<PointRecord> {
        let mut records = Vec::new();
        self.recompute(equipment, &mut records);
        records
    }

    /// The item load's computation: `ComputePoints` with the equipment the load placed, keeping
    /// the pools the character was saved with.
    ///
    /// Legacy's `EquipTo` applies each worn item's bonuses as the load places it, and then
    /// computes the points or the battle points again (`item.cpp:1458-1505`); after the last
    /// item that is one computation with everything worn. Its `ApplyPoint` keeps each pool's
    /// share of a maximum an item raises, but the stored pool was saved with the item already
    /// worn, so the share is taken against the maximum without it: a character saved at 600 of
    /// 1500, 500 of the maximum from an item, holds 600 of 1000 until the item is worn and 900
    /// of 1500 after. That relog heal is a Defect. Here the stored pools come back after the computation, and the load's
    /// `CheckMaximumPoints` clamps them next. The computation's records are not returned: every
    /// slot they set is sent again, at its final value, by the load's points record.
    pub fn compute_loaded(&mut self, equipment: &Equipment<'_>) {
        let (hp, sp, stamina) = (self.hp, self.sp, self.stamina);
        let _discarded = self.compute_points_with(equipment);
        self.hp = hp;
        self.sp = sp;
        self.stamina = stamina;
    }

    /// `ApplyPoint(type, value)`: carry out one bonus of an item or a set.
    ///
    /// `APPLY_CON` and `APPLY_INT` change the attribute and the maxima it gives;
    /// `APPLY_SKILL` changes one skill's damage bonus; the maximum hit and spell point types
    /// keep the current pool's share of its maximum; every other type changes its one point
    /// slot. A type whose slot's `PointChange` arm is not ported changes nothing
    /// ([`apply_is_ported`]).
    pub fn apply_point(&mut self, apply: u8, value: i32) -> Vec<PointRecord> {
        let mut records = Vec::new();
        self.apply(apply, value, &mut records);
        records
    }

    /// `CheckMaximumPoints`: lower the hit and spell points to their maxima.
    pub fn check_maximum_points(&mut self) -> Vec<PointRecord> {
        let mut records = Vec::new();
        if self.max_hp() < self.hp {
            let _ = self.change(
                point::POINT_HP,
                self.max_hp().saturating_sub(self.hp),
                false,
                false,
                &mut records,
            );
        }
        if self.max_sp < self.sp {
            let _ = self.change(
                point::POINT_SP,
                self.max_sp.saturating_sub(self.sp),
                false,
                false,
                &mut records,
            );
        }
        records
    }

    fn movement_is_weakened(&self) -> bool {
        i32::from(self.will().movement) > self.get_point(point::POINT_SUNGMA_MOVE)
    }

    fn job_points(&self) -> &'static JobInitialPoints {
        &levels::JOB_INITIAL_POINTS[usize::from(race_to_job(self.race))]
    }

    /// The body of `ApplyPoint` (`char.cpp:4826-5043`).
    fn apply(&mut self, apply: u8, value: i32, records: &mut Vec<PointRecord>) {
        let job = self.job_points();
        let per = |factor: i32| narrow(i64::from(value) * i64::from(factor));
        match ApplyArm::of(apply) {
            ApplyArm::Nothing => {}
            ApplyArm::Con => {
                self.nested(point::POINT_HT, value, records);
                self.nested(point::POINT_MAX_HP, per(job.hp_per_ht), records);
                self.nested(point::POINT_MAX_STAMINA, per(job.stamina_per_con), records);
            }
            ApplyArm::Int => {
                self.nested(point::POINT_IQ, value, records);
                self.nested(point::POINT_MAX_SP, per(job.sp_per_iq), records);
            }
            ApplyArm::Skill => {
                // The top byte names the skill, bit 23 says whether the low 23 bits are added
                // or taken away.
                let [skill, ..] = value.to_be_bytes();
                let change = value & 0x007f_ffff;
                let change = if value & 0x0080_0000 == 0 {
                    -change
                } else {
                    change
                };
                let bonus = self.skill_damage_bonus.entry(skill).or_insert(0);
                *bonus = bonus.saturating_add(change);
            }
            ApplyArm::MaxHp(slot) => {
                let before = self.max_hp();
                if before == 0 {
                    return;
                }
                self.nested(slot, value, records);
                let hp = i32_to_f32(self.hp);
                let ratio = i32_to_f32(self.max_hp()) / i32_to_f32(before);
                self.nested(point::POINT_HP, f32_to_i32(hp * ratio - hp), records);
            }
            ApplyArm::MaxSp(slot) => {
                let before = self.max_sp;
                if before == 0 {
                    return;
                }
                self.nested(slot, value, records);
                let sp = i32_to_f32(self.sp);
                let ratio = i32_to_f32(self.max_sp) / i32_to_f32(before);
                self.nested(point::POINT_SP, f32_to_i32(sp * ratio - sp), records);
            }
            ApplyArm::Point(slot) => self.nested(slot, value, records),
        }
    }

    /// Add to an instant slot and answer the new value.
    fn add(&mut self, kind: usize, amount: i32) -> i32 {
        let value = narrow(i64::from(self.instant[kind]) + i64::from(amount));
        self.instant[kind] = value;
        value
    }

    /// A change legacy makes from inside another change or a computation. Its refusal, if
    /// any, is dropped, because legacy's `PointChange` returns nothing to drop it from.
    fn nested(&mut self, kind: usize, amount: i32, records: &mut Vec<PointRecord>) {
        let _ = self.change(kind, amount, false, false, records);
    }

    /// The body of `PointChange`. Returns the refusal, or `None` when the change ran (even
    /// if it wrote no record).
    fn change(
        &mut self,
        kind: usize,
        amount: i32,
        show_amount: bool,
        broadcast: bool,
        records: &mut Vec<PointRecord>,
    ) -> Option<PointChangeRefused> {
        let Ok(byte) = u8::try_from(kind) else {
            return Some(PointChangeRefused::Unknown(kind));
        };
        if NOT_PORTED.contains(&kind) {
            return Some(PointChangeRefused::NotPorted(kind));
        }
        let (amount, value) = match self.apply_pool(kind, amount, records) {
            Applied::Changed { amount, value } => (amount, value),
            Applied::Silent => return None,
            Applied::NotPool => match self.apply_slot(kind, amount, records) {
                Applied::Changed { amount, value } => (amount, value),
                Applied::Silent => return None,
                Applied::NotPool => return Some(PointChangeRefused::Unknown(kind)),
            },
        };
        let show_amount = show_amount && !NEXT_EXP_SLOTS.contains(&kind);
        if RECOMPUTES_BATTLE.contains(&kind) {
            self.battle_points(records);
        }
        if kind == point::POINT_HP && amount == 0 {
            return None;
        }
        self.emit(byte, amount, show_amount, broadcast, value, records);
        None
    }

    /// The arms for the three pools and their maxima.
    fn apply_pool(&mut self, kind: usize, amount: i32, records: &mut Vec<PointRecord>) -> Applied {
        let (amount, value) = match kind {
            point::POINT_NONE => return Applied::Silent,
            point::POINT_HP => {
                let amount = amount.min(self.max_hp().saturating_sub(self.hp));
                self.hp = self.hp.saturating_add(amount);
                (amount, self.hp)
            }
            point::POINT_SP => {
                let amount = amount.min(self.max_sp.saturating_sub(self.sp));
                self.sp = self.sp.saturating_add(amount);
                (amount, self.sp)
            }
            point::POINT_STAMINA => {
                let amount = amount.min(self.max_stamina.saturating_sub(self.stamina));
                self.stamina = self.stamina.saturating_add(amount);
                // "A decrease is not sent" (`char.cpp:4297-4298`), unless it emptied the pool.
                if amount < 0 && self.stamina != 0 {
                    return Applied::Silent;
                }
                (amount, self.stamina)
            }
            point::POINT_MAX_HP => {
                self.add(kind, amount);
                self.max_hp = self.total_maximum(
                    point::POINT_MAX_HP,
                    point::POINT_MAX_HP_PCT,
                    3500,
                    point::POINT_PARTY_TANKER_BONUS,
                );
                (amount, self.max_hp())
            }
            point::POINT_MAX_SP => {
                self.add(kind, amount);
                self.max_sp = self.total_maximum(
                    point::POINT_MAX_SP,
                    point::POINT_MAX_SP_PCT,
                    800,
                    point::POINT_PARTY_SKILL_MASTER_BONUS,
                );
                (amount, self.max_sp)
            }
            point::POINT_MAX_HP_PCT | point::POINT_MAX_SP_PCT => {
                let value = self.add(kind, amount);
                let pool = if kind == point::POINT_MAX_HP_PCT {
                    point::POINT_MAX_HP
                } else {
                    point::POINT_MAX_SP
                };
                self.nested(pool, 0, records);
                (amount, value)
            }
            point::POINT_MAX_STAMINA => {
                self.max_stamina = self.max_stamina.saturating_add(amount);
                (amount, self.max_stamina)
            }
            _ => return Applied::NotPool,
        };
        Applied::Changed {
            amount,
            value: i64::from(value),
        }
    }

    /// The arms for every other slot. `NotPool` here means legacy has no arm at all.
    fn apply_slot(&mut self, kind: usize, amount: i32, records: &mut Vec<PointRecord>) -> Applied {
        let mut amount = amount;
        let value = match kind {
            // The cost of the next level, never the change (`char.cpp:3897-3901` and
            // `4073-4076`).
            point::POINT_NEXT_EXP => levels::next_exp(self.level),
            point::POINT_CONQUEROR_NEXT_EXP => levels::conqueror_next_exp(self.conqueror_level),
            point::POINT_DEF_GRADE => {
                let value = self.add(kind, amount);
                self.nested(point::POINT_CLIENT_DEF_GRADE, amount, records);
                i64::from(value)
            }
            point::POINT_CLIENT_DEF_GRADE => i64::from(self.add(kind, amount)),
            point::POINT_CONQUEROR_POINT => {
                let value = self.add(kind, amount);
                self.real[kind] = value;
                i64::from(value)
            }
            point::POINT_RAMADAN_CANDY_BONUS_EXP => {
                self.instant[kind] = amount;
                i64::from(amount)
            }
            point::POINT_IMMUNE_STUN | point::POINT_IMMUNE_SLOW | point::POINT_IMMUNE_FALL => {
                i64::from(self.immunity(kind, amount))
            }
            point::POINT_ATT_GRADE_BONUS
            | point::POINT_DEF_GRADE_BONUS
            | point::POINT_MAGIC_ATT_GRADE_BONUS
            | point::POINT_MAGIC_DEF_GRADE_BONUS => {
                self.add(kind, amount);
                let grade = match kind {
                    point::POINT_ATT_GRADE_BONUS => point::POINT_ATT_GRADE,
                    point::POINT_DEF_GRADE_BONUS => point::POINT_DEF_GRADE,
                    point::POINT_MAGIC_ATT_GRADE_BONUS => point::POINT_MAGIC_ATT_GRADE,
                    _ => point::POINT_MAGIC_DEF_GRADE,
                };
                self.nested(grade, amount, records);
                i64::from(self.instant[kind])
            }
            // Legacy keeps the voice outside the array, so its slot always reads 0.
            point::POINT_VOICE | point::POINT_EMPIRE_POINT => i64::from(self.real[kind]),
            _ if COUNTERS.contains(&kind) => {
                let value = self.add(kind, amount);
                self.real[kind] = value;
                i64::from(value)
            }
            _ if CAPPED.contains(&kind) => {
                if i64::from(self.instant[kind]) + i64::from(amount) > 100 {
                    amount = 100 - self.instant[kind];
                }
                i64::from(self.add(kind, amount))
            }
            _ if PLAIN.contains(&kind) => i64::from(self.add(kind, amount)),
            _ => return Applied::NotPool,
        };
        Applied::Changed { amount, value }
    }

    /// An immunity slot: add, then set its `IMMUNE_*` bit while the slot is non-zero.
    fn immunity(&mut self, kind: usize, amount: i32) -> i32 {
        let value = self.add(kind, amount);
        let bit = match kind {
            point::POINT_IMMUNE_STUN => IMMUNE_STUN,
            point::POINT_IMMUNE_SLOW => IMMUNE_SLOW,
            _ => IMMUNE_FALL,
        };
        if value == 0 {
            self.immune_flag &= !bit;
        } else {
            self.immune_flag |= bit;
        }
        value
    }

    /// Write the one record the Rewrite keeps of legacy's two: the `UpdatePointsPacket` copy,
    /// whose movement speed is halved under the map's movement will.
    fn emit(
        &self,
        byte: u8,
        amount: i32,
        show_amount: bool,
        broadcast: bool,
        value: i64,
        records: &mut Vec<PointRecord>,
    ) {
        let value = if usize::from(byte) == point::POINT_MOV_SPEED && self.movement_is_weakened() {
            value / 2
        } else {
            value
        };
        records.push(PointRecord {
            kind: byte,
            amount: if show_amount { i64::from(amount) } else { 0 },
            value,
            broadcast,
        });
    }

    /// The total of a maximum pool: the base in the real array, plus a percentage of it capped
    /// at `cap`, plus the instant slot's flat bonus, plus a party bonus (`char.cpp:4303-4336`).
    fn total_maximum(&self, pool: usize, percent: usize, cap: i64, party: usize) -> i32 {
        let base = i64::from(self.real[pool]);
        let from_percent = (base * i64::from(self.instant[percent]) / 100).min(cap);
        narrow(base + from_percent + i64::from(self.instant[pool]) + i64::from(self.instant[party]))
    }

    fn battle_points(&mut self, records: &mut Vec<PointRecord>) {
        self.instant[point::POINT_ATT_GRADE] = 0;
        self.instant[point::POINT_DEF_GRADE] = 0;
        self.instant[point::POINT_CLIENT_DEF_GRADE] = 0;
        self.instant[point::POINT_MAGIC_ATT_GRADE] = 0;
        self.instant[point::POINT_MAGIC_DEF_GRADE] = 0;

        let level = i64::from(self.level);
        let st = i64::from(self.instant[point::POINT_ST]);
        let ht = i64::from(self.instant[point::POINT_HT]);
        let dx = i64::from(self.instant[point::POINT_DX]);
        let iq = i64::from(self.instant[point::POINT_IQ]);
        let stat_attack = match race_to_job(self.race) {
            1 => (4 * st + 2 * dx) / 3,
            3 => (4 * st + 2 * iq) / 3,
            _ => 2 * st,
        };
        let attack = 2 * level + stat_attack + self.bonus(point::POINT_ATT_GRADE_BONUS);
        self.nested(point::POINT_ATT_GRADE, narrow(attack), records);

        let shown_defence = level + ht;
        // `(int)(HT / 1.25)` is `HT * 4 / 5` truncated: the double quotient of an `int` by
        // 1.25 is never within rounding of the next integer.
        let defence = level + ht * 4 / 5;
        let mut armour = i64::from(self.armour)
            + self.bonus(point::POINT_DEF_GRADE_BONUS)
            + self.bonus(point::POINT_PARTY_DEFENDER_BONUS);
        armour += armour * self.bonus(point::POINT_MALL_DEFBONUS) / 100;
        self.nested(point::POINT_DEF_GRADE, narrow(defence + armour), records);
        let shown = shown_defence + armour - self.bonus(point::POINT_DEF_GRADE);
        self.nested(point::POINT_CLIENT_DEF_GRADE, narrow(shown), records);

        let magic_attack = 2 * level + 2 * iq + self.bonus(point::POINT_MAGIC_ATT_GRADE_BONUS);
        self.nested(point::POINT_MAGIC_ATT_GRADE, narrow(magic_attack), records);
        let magic_defence =
            level + (3 * iq + ht) / 3 + armour / 2 + self.bonus(point::POINT_MAGIC_DEF_GRADE_BONUS);
        self.nested(point::POINT_MAGIC_DEF_GRADE, narrow(magic_defence), records);
    }

    fn bonus(&self, kind: usize) -> i64 {
        i64::from(self.instant[kind])
    }

    fn recompute(&mut self, equipment: &Equipment<'_>, records: &mut Vec<PointRecord>) {
        const KEPT: [usize; 15] = [
            point::POINT_STAT,
            point::POINT_STAT_RESET_COUNT,
            point::POINT_SKILL,
            point::POINT_SUB_SKILL,
            point::POINT_HORSE_SKILL,
            point::POINT_LEVEL_STEP,
            point::POINT_PARTY_ATTACKER_BONUS,
            point::POINT_PARTY_TANKER_BONUS,
            point::POINT_PARTY_BUFFER_BONUS,
            point::POINT_PARTY_SKILL_MASTER_BONUS,
            point::POINT_PARTY_HASTE_BONUS,
            point::POINT_PARTY_DEFENDER_BONUS,
            point::POINT_HP_RECOVERY,
            point::POINT_SP_RECOVERY,
            point::POINT_PRIVATE_SHOP_UNLOCKED_SLOT,
        ];
        let kept = KEPT.map(|kind| self.instant[kind]);
        let conqueror_point = self.instant[point::POINT_CONQUEROR_POINT];
        let conqueror_step = self.instant[point::POINT_CONQUEROR_LEVEL_STEP];

        self.instant = [0; SLOTS];
        self.skill_damage_bonus.clear();
        for (kind, value) in KEPT.into_iter().zip(kept) {
            self.instant[kind] = value;
        }
        for kind in [
            point::POINT_ST,
            point::POINT_HT,
            point::POINT_DX,
            point::POINT_IQ,
        ]
        .into_iter()
        .chain(SUNGMA_SLOTS)
        {
            self.instant[kind] = self.real[kind];
        }
        self.instant[point::POINT_CONQUEROR_POINT] = conqueror_point;
        self.instant[point::POINT_CONQUEROR_LEVEL_STEP] = conqueror_step;
        // The parts go back to `GetOriginalPart` and the worn loop's parts switch sets them
        // (`char.cpp:2917-2926`); no other step reads them, so both happen here.
        self.parts = equipment.parts(u16::from(self.part_base), self.parts);
        self.instant[point::POINT_INVEN] = self.inven_point;

        let job = self.job_points();
        let ht = i64::from(self.instant[point::POINT_HT]);
        let iq = i64::from(self.instant[point::POINT_IQ]);
        let hp_base = narrow(
            i64::from(job.max_hp)
                + i64::from(self.random_hp)
                + ht * i64::from(job.hp_per_ht)
                + i64::from(self.passive.max_hp),
        );
        let sp_base = narrow(
            i64::from(job.max_sp) + i64::from(self.random_sp) + iq * i64::from(job.sp_per_iq),
        );
        let stamina_base = narrow(i64::from(job.max_stamina) + ht * i64::from(job.stamina_per_con));

        self.instant[point::POINT_MOV_SPEED] = 100;
        self.instant[point::POINT_ATT_SPEED] = 100;
        let haste = self.instant[point::POINT_PARTY_HASTE_BONUS];
        self.nested(point::POINT_ATT_SPEED, haste, records);
        self.instant[point::POINT_CASTING_SPEED] = 100;

        let passive = self.passive;
        self.nested(point::POINT_ATTBONUS_MONSTER, passive.monster, records);
        self.nested(point::POINT_ATTBONUS_METIN, passive.stone, records);
        self.nested(point::POINT_ATTBONUS_BOSS, passive.boss, records);

        self.armour = equipment.armour();
        self.battle_points(records);

        self.real[point::POINT_MAX_HP] = hp_base;
        self.nested(point::POINT_MAX_HP, 0, records);
        self.real[point::POINT_MAX_SP] = sp_base;
        self.nested(point::POINT_MAX_SP, 0, records);
        self.max_stamina = stamina_base;

        let hp_before = self.hp;
        let sp_before = self.sp;
        self.immune_flag = 0;

        for (_, applies) in equipment.modify_points() {
            for (apply, value) in applies {
                self.apply(apply, value, records);
            }
        }
        for (apply, value) in equipment.set_bonus_applies() {
            self.apply(apply, value, records);
        }

        if self.hp > self.max_hp() {
            self.nested(
                point::POINT_HP,
                self.max_hp().saturating_sub(self.hp),
                records,
            );
        }
        if self.sp > self.max_sp {
            self.nested(
                point::POINT_SP,
                self.max_sp.saturating_sub(self.sp),
                records,
            );
        }
        // `@fixme118`: legacy puts back whatever the affects took (`char.cpp:3150-3156`). The
        // Rewrite has no affects, so this only ever meets the clamp above, which it cannot
        // undo because a change is bounded by the maximum.
        if self.hp != hp_before {
            self.nested(point::POINT_HP, hp_before.saturating_sub(self.hp), records);
        }
        if self.sp != sp_before {
            self.nested(point::POINT_SP, sp_before.saturating_sub(self.sp), records);
        }
    }
}

/// Whether [`Points::apply_point`] carries out `apply` as legacy does: every type but one
/// whose point slot's `PointChange` arm is not ported ([`PointChangeRefused::NotPorted`]),
/// which the apply leaves unchanged.
///
/// `APPLY_MAGIC_ATTBONUS_PER` (83) counts as ported: its slot, `POINT_MAGIC_ATT_BONUS_PER`,
/// has no `PointChange` arm in legacy, whose `default` logs and returns
/// (`char.cpp:4773-4775`), so it changes nothing there and nothing here.
#[must_use]
pub fn apply_is_ported(apply: u8) -> bool {
    match ApplyArm::of(apply) {
        ApplyArm::Point(slot) => !NOT_PORTED.contains(&slot),
        _ => true,
    }
}

/// Narrow a 64-bit total to the `int` legacy keeps it in, saturating rather than wrapping.
fn narrow(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

#[cfg(test)]
mod tests {
    use common::enums::EWearPositions;
    use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
    use gamedata::item_kind::{ARMOR_BODY, ITEM_ARMOR, ITEM_WEAPON};
    use gamedata::item_proto::{ItemProto, ItemProtos, ItemValue};
    use protocol::item_pos::ItemPos;

    use super::*;
    use crate::character::apply::{
        APPLY_ATTBONUS_BOSS, APPLY_ATTBONUS_HUMAN, APPLY_ATTBONUS_METIN, APPLY_CON,
        APPLY_COSTUME_ATTR_BONUS, APPLY_ENERGY, APPLY_INT, APPLY_MAX_HP, APPLY_MAX_HP_PCT,
        APPLY_MAX_SP, APPLY_NONE, APPLY_SKILL, MAX_APPLY_NUM,
    };

    /// `APPLY_MAGIC_ATTBONUS_PER`, whose slot `POINT_MAGIC_ATT_BONUS_PER` has no
    /// `PointChange` arm in legacy.
    const APPLY_MAGIC_ATTBONUS_PER: u8 = 83;
    use crate::character::CharacterItems;
    use crate::item::Item;

    const BODY: u16 = EWearPositions::Body as u16;
    const WEAPON: u16 = EWearPositions::Weapon as u16;

    fn proto(vnum: u32, item_type: i32, sub_type: i32) -> ItemProto {
        ItemProto::for_category_rule(vnum, item_type, sub_type)
    }

    fn valued(mut proto: ItemProto, values: [i32; 6]) -> ItemProto {
        proto.values = values;
        proto
    }

    fn applying(mut proto: ItemProto, applies: [(u8, i32); 3]) -> ItemProto {
        proto.applies = applies.map(|(kind, value)| ItemValue {
            kind: i32::from(kind),
            value,
        });
        proto
    }

    fn wearing(worn: &[(u16, &Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (wear, item) in worn {
            let pos = ItemPos::new(EWindows::Inventory as u8, INVENTORY_MAX_NUM + wear);
            items.set(pos, item).expect("the wear cell is free");
        }
        items
    }

    /// An `APPLY_SKILL` value: the skill in the top byte, bit 23 set to add.
    fn skill_apply(skill: u8, add: bool, change: i32) -> i32 {
        let top = i32::from_ne_bytes(u32::from(skill).wrapping_shl(24).to_ne_bytes());
        top | if add { 0x0080_0000 } else { 0 } | change
    }

    /// A fresh level-1 character of a race, with its job's starting attributes, full pools,
    /// and no conqueror level, on map 1.
    fn fresh(race: u8) -> PointsRow {
        let job = &levels::JOB_INITIAL_POINTS[usize::from(race_to_job(race))];
        PointsRow {
            race,
            level: 1,
            conqueror_level: 0,
            st: job.st,
            ht: job.ht,
            dx: job.dx,
            iq: job.iq,
            sungma: [0; 4],
            hp: 0,
            sp: 0,
            stamina: 0,
            inven_point: 0,
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
        }
    }

    fn warrior() -> Points {
        let mut row = fresh(0);
        row.hp = 760;
        row.sp = 260;
        row.stamina = 820;
        Points::load(&row)
    }

    fn record(kind: usize, amount: i64, value: i64) -> PointRecord {
        PointRecord {
            kind: u8::try_from(kind).unwrap(),
            amount,
            value,
            broadcast: false,
        }
    }

    fn grades(points: &Points) -> [i32; 5] {
        [
            point::POINT_ATT_GRADE,
            point::POINT_DEF_GRADE,
            point::POINT_CLIENT_DEF_GRADE,
            point::POINT_MAGIC_ATT_GRADE,
            point::POINT_MAGIC_DEF_GRADE,
        ]
        .map(|kind| points.get_point(kind))
    }

    /// The battle records of the fresh warrior, in legacy's order: the client defence grade
    /// that `POINT_DEF_GRADE` changes comes before the defence grade itself.
    fn warrior_battle_records() -> Vec<PointRecord> {
        vec![
            record(point::POINT_ATT_GRADE, 0, 14),
            record(point::POINT_CLIENT_DEF_GRADE, 0, 4),
            record(point::POINT_DEF_GRADE, 0, 4),
            record(point::POINT_CLIENT_DEF_GRADE, 0, 5),
            record(point::POINT_MAGIC_ATT_GRADE, 0, 8),
            record(point::POINT_MAGIC_DEF_GRADE, 0, 5),
        ]
    }

    #[test]
    fn a_fresh_character_of_each_job_has_the_grades_the_formulas_give() {
        // ATT = 2 lvl + the job term; DEF = lvl + 4 HT / 5; shown DEF = lvl + HT;
        // MATT = 2 lvl + 2 IQ; MDEF = lvl + (3 IQ + HT) / 3.
        for (race, expected, max_hp, max_sp) in [
            (0, [14, 4, 5, 8, 5], 760, 260),
            (1, [11, 3, 4, 8, 5], 770, 260),
            (2, [12, 3, 4, 12, 7], 770, 300),
            (3, [10, 4, 5, 14, 8], 860, 320),
            (4, [14, 4, 5, 8, 5], 760, 260),
            (5, [11, 3, 4, 8, 5], 770, 260),
            (6, [12, 3, 4, 12, 7], 770, 300),
            (7, [10, 4, 5, 14, 8], 860, 320),
        ] {
            let points = Points::load(&fresh(race));
            assert_eq!(grades(&points), expected, "race {race}");
            assert_eq!(points.max_hp(), max_hp, "race {race}");
            assert_eq!(points.max_sp(), max_sp, "race {race}");
            assert_eq!(points.max_stamina(), 800 + 5 * i32::from(fresh(race).ht));
            assert_eq!(points.real_point(point::POINT_MAX_HP), max_hp);
            assert_eq!(points.real_point(point::POINT_MAX_SP), max_sp);
        }
    }

    #[test]
    fn a_high_level_shaman_has_the_grades_the_parity_fixture_expects() {
        let row = PointsRow {
            race: 3,
            level: 154,
            st: 17,
            ht: 18,
            dx: 19,
            iq: 20,
            ..fresh(3)
        };
        assert_eq!(grades(&Points::load(&row)), [344, 168, 172, 348, 180]);
    }

    #[test]
    fn the_assassin_and_shaman_terms_use_dexterity_and_intelligence() {
        let base = PointsRow {
            level: 10,
            st: 30,
            ht: 10,
            dx: 60,
            iq: 90,
            ..fresh(0)
        };
        let attack = |race| Points::load(&PointsRow { race, ..base }).get_point(18);
        assert_eq!(attack(0), 20 + 60);
        assert_eq!(attack(2), 20 + 60);
        assert_eq!(attack(1), 20 + (120 + 120) / 3);
        assert_eq!(attack(3), 20 + (120 + 180) / 3);
        assert_eq!(attack(8), 20 + 60, "a race past the eight is a warrior");
    }

    #[test]
    fn races_map_onto_the_four_jobs_and_anything_else_is_a_warrior() {
        for race in 0..8 {
            assert_eq!(race_to_job(race), race % 4);
        }
        for race in [8, 9, 100, 255] {
            assert_eq!(race_to_job(race), 0);
        }
    }

    #[test]
    fn the_battle_records_come_in_legacy_order() {
        let mut points = warrior();
        assert_eq!(points.compute_battle_points(), warrior_battle_records());
        assert_eq!(grades(&points), [14, 4, 5, 8, 5]);
    }

    #[test]
    fn compute_points_writes_the_speed_passives_battle_and_maxima_records_in_order() {
        let mut points = warrior();
        let mut expected = vec![
            record(point::POINT_ATT_SPEED, 0, 100),
            record(point::POINT_ATTBONUS_MONSTER, 0, 0),
            record(point::POINT_ATTBONUS_METIN, 0, 0),
            record(point::POINT_ATTBONUS_BOSS, 0, 0),
        ];
        expected.extend(warrior_battle_records());
        expected.push(record(point::POINT_MAX_HP, 0, 760));
        expected.push(record(point::POINT_MAX_SP, 0, 260));
        assert_eq!(points.compute_points(), expected);
        assert_eq!(points.get_point(point::POINT_MOV_SPEED), 100);
        assert_eq!(points.get_point(point::POINT_CASTING_SPEED), 100);
    }

    #[test]
    fn a_stored_pool_above_its_maximum_survives_the_load_and_the_maximum_check_lowers_it() {
        let mut row = fresh(0);
        row.hp = 900;
        row.sp = 300;
        let mut points = Points::load(&row);
        assert_eq!((points.hp(), points.sp()), (900, 300));
        assert_eq!(
            points.check_maximum_points(),
            [
                record(point::POINT_HP, 0, 760),
                record(point::POINT_SP, 0, 260)
            ]
        );
        assert_eq!((points.hp(), points.sp()), (760, 260));
        assert!(points.check_maximum_points().is_empty());
    }

    #[test]
    fn the_maximum_check_touches_only_the_pool_that_is_over() {
        let mut row = fresh(0);
        row.hp = 100;
        row.sp = 261;
        let mut points = Points::load(&row);
        assert_eq!(
            points.check_maximum_points(),
            [record(point::POINT_SP, 0, 260)]
        );
        assert_eq!(points.hp(), 100);
        row.sp = 260;
        row.hp = 761;
        let mut points = Points::load(&row);
        assert_eq!(
            points.check_maximum_points(),
            [record(point::POINT_HP, 0, 760)]
        );
    }

    #[test]
    fn compute_points_clamps_the_pools_and_repeats_the_spell_point_record() {
        let mut points = warrior();
        points.hp = 900;
        points.sp = 300;
        let records = points.compute_points();
        // The clamp writes each pool once; `@fixme118` then asks to give the difference back,
        // which the maximum bounds to 0. A zero hit-point change writes nothing, but a zero
        // spell-point change writes its record again.
        assert_eq!(
            records[records.len() - 3..],
            [
                record(point::POINT_HP, 0, 760),
                record(point::POINT_SP, 0, 260),
                record(point::POINT_SP, 0, 260),
            ]
        );
        assert_eq!(
            records[records.len() - 4],
            record(point::POINT_MAX_SP, 0, 260)
        );
        assert_eq!((points.hp(), points.sp()), (760, 260));
    }

    #[test]
    fn a_hit_point_change_is_bounded_by_the_maximum_and_a_zero_change_writes_nothing() {
        let mut points = warrior();
        assert_eq!(
            points.point_change(point::POINT_HP, 100, true, false),
            Ok(vec![])
        );
        assert_eq!(points.hp(), 760);
        assert_eq!(
            points.point_change(point::POINT_HP, -300, true, true),
            Ok(vec![PointRecord {
                kind: 5,
                amount: -300,
                value: 460,
                broadcast: true,
            }])
        );
        assert_eq!(
            points.point_change(point::POINT_HP, 1000, true, false),
            Ok(vec![record(point::POINT_HP, 300, 760)])
        );
    }

    #[test]
    fn a_spell_point_change_at_the_maximum_still_writes_its_record() {
        let mut points = warrior();
        assert_eq!(
            points.point_change(point::POINT_SP, 50, true, false),
            Ok(vec![record(point::POINT_SP, 0, 260)])
        );
        assert_eq!(
            points.point_change(point::POINT_SP, -60, false, false),
            Ok(vec![record(point::POINT_SP, 0, 200)])
        );
    }

    #[test]
    fn a_stamina_decrease_is_not_written_unless_it_empties_the_pool() {
        let mut points = warrior();
        assert_eq!(
            points.point_change(point::POINT_STAMINA, -20, true, false),
            Ok(vec![])
        );
        assert_eq!(points.stamina(), 800);
        assert_eq!(
            points.point_change(point::POINT_STAMINA, 50, true, false),
            Ok(vec![record(point::POINT_STAMINA, 20, 820)])
        );
        assert_eq!(
            points.point_change(point::POINT_STAMINA, -820, true, false),
            Ok(vec![record(point::POINT_STAMINA, -820, 0)])
        );
        assert_eq!(points.stamina(), 0);
    }

    #[test]
    fn the_maximum_hit_points_add_a_capped_percentage_a_flat_bonus_and_the_party_bonus() {
        let mut points = warrior();
        assert_eq!(
            points.point_change(point::POINT_MAX_HP_PCT, 10, true, false),
            Ok(vec![
                record(point::POINT_MAX_HP, 0, 836),
                record(point::POINT_MAX_HP_PCT, 10, 10),
            ])
        );
        assert_eq!(
            points.point_change(point::POINT_MAX_HP, 40, true, false),
            Ok(vec![record(point::POINT_MAX_HP, 40, 876)])
        );
        points
            .point_change(point::POINT_PARTY_TANKER_BONUS, 100, false, false)
            .unwrap();
        assert_eq!(
            points.point_change(point::POINT_MAX_HP, 0, false, false),
            Ok(vec![record(point::POINT_MAX_HP, 0, 976)])
        );
        // 760 * 1000 / 100 is 7600, capped at 3500.
        points
            .point_change(point::POINT_MAX_HP_PCT, 990, false, false)
            .unwrap();
        assert_eq!(points.max_hp(), 760 + 3500 + 40 + 100);
    }

    #[test]
    fn the_maximum_spell_points_cap_the_percentage_at_800() {
        let mut points = warrior();
        points
            .point_change(point::POINT_MAX_SP_PCT, 50, false, false)
            .unwrap();
        assert_eq!(points.max_sp(), 260 + 130);
        points
            .point_change(point::POINT_PARTY_SKILL_MASTER_BONUS, 7, false, false)
            .unwrap();
        points
            .point_change(point::POINT_MAX_SP_PCT, 950, false, false)
            .unwrap();
        assert_eq!(points.max_sp(), 260 + 800 + 7);
        assert_eq!(
            points.point_change(point::POINT_MAX_STAMINA, 5, true, false),
            Ok(vec![record(point::POINT_MAX_STAMINA, 5, 825)])
        );
    }

    #[test]
    fn a_new_base_equal_to_the_old_total_is_still_stored() {
        // Legacy stores the base only when it differs from the old total (a Defect): here the
        // old total is 760 + 40 = 800 and the new base is 760 + 40 from the passive skill.
        let mut points = warrior();
        points
            .point_change(point::POINT_MAX_HP, 40, false, false)
            .unwrap();
        assert_eq!(points.max_hp(), 800);
        points.set_passive_bonuses(PassiveBonuses {
            max_hp: 40,
            ..PassiveBonuses::default()
        });
        points.compute_points();
        assert_eq!(points.real_point(point::POINT_MAX_HP), 800);
        assert_eq!(points.max_hp(), 800);
    }

    #[test]
    fn the_passive_bonuses_reach_their_slots() {
        let mut points = warrior();
        points.set_passive_bonuses(PassiveBonuses {
            max_hp: 1333,
            monster: 10,
            stone: 20,
            boss: 30,
        });
        let records = points.compute_points();
        assert_eq!(records[1], record(point::POINT_ATTBONUS_MONSTER, 0, 10));
        assert_eq!(records[2], record(point::POINT_ATTBONUS_METIN, 0, 20));
        assert_eq!(records[3], record(point::POINT_ATTBONUS_BOSS, 0, 30));
        assert_eq!(points.max_hp(), 760 + 1333);
    }

    #[test]
    fn the_defence_grade_also_changes_the_client_grade_first() {
        let mut points = warrior();
        assert_eq!(
            points.point_change(point::POINT_DEF_GRADE, 3, true, false),
            Ok(vec![
                record(point::POINT_CLIENT_DEF_GRADE, 0, 8),
                record(point::POINT_DEF_GRADE, 3, 7),
            ])
        );
        assert_eq!(
            points.point_change(point::POINT_CLIENT_DEF_GRADE, 2, true, false),
            Ok(vec![record(point::POINT_CLIENT_DEF_GRADE, 2, 10)])
        );
    }

    #[test]
    fn a_grade_bonus_changes_its_grade_before_its_own_record() {
        let mut points = warrior();
        assert_eq!(
            points.point_change(point::POINT_DEF_GRADE_BONUS, 10, true, false),
            Ok(vec![
                record(point::POINT_CLIENT_DEF_GRADE, 0, 15),
                record(point::POINT_DEF_GRADE, 0, 14),
                record(point::POINT_DEF_GRADE_BONUS, 10, 10),
            ])
        );
        for (bonus, grade, value) in [
            (point::POINT_ATT_GRADE_BONUS, point::POINT_ATT_GRADE, 19),
            (
                point::POINT_MAGIC_ATT_GRADE_BONUS,
                point::POINT_MAGIC_ATT_GRADE,
                13,
            ),
            (
                point::POINT_MAGIC_DEF_GRADE_BONUS,
                point::POINT_MAGIC_DEF_GRADE,
                10,
            ),
        ] {
            assert_eq!(
                points.point_change(bonus, 5, false, false),
                Ok(vec![record(grade, 0, value), record(bonus, 0, 5)])
            );
        }
        // The bonuses stay in the next battle computation: the armour term takes the defence
        // bonus, and half of it reaches the magic defence.
        points.compute_battle_points();
        assert_eq!(grades(&points), [19, 14, 15, 13, 15]);
    }

    #[test]
    fn the_armour_and_the_item_shop_defence_bonus_raise_every_defence() {
        let mut points = warrior();
        let armour = valued(proto(11_210, ITEM_ARMOR, ARMOR_BODY), [0, 40, 0, 0, 0, 0]);
        let protos = ItemProtos::from_rows(vec![armour]);
        let body = Item::new(1, 11_210);
        let worn = wearing(&[(BODY, &body)]);
        points.compute_points_with(&Equipment::of(&worn, &protos));
        points
            .point_change(point::POINT_MALL_DEFBONUS, 50, false, false)
            .unwrap();
        points.compute_battle_points();
        // armour = 40 + 40 * 50 / 100 = 60.
        assert_eq!(grades(&points), [14, 64, 65, 8, 35]);
    }

    #[test]
    fn an_apply_of_vitality_or_intelligence_raises_the_maxima_it_gives() {
        let mut points = warrior();
        let records = points.apply_point(APPLY_CON, 2);
        assert_eq!(points.get_point(point::POINT_HT), 6);
        assert_eq!(points.max_hp(), 760 + 2 * 40);
        assert_eq!(points.max_stamina(), 820 + 2 * 5);
        // Vitality does not keep the hit points' share: the pool stays where it was.
        assert_eq!(points.hp(), 760);
        let tail: Vec<(u8, i64)> = records[records.len() - 3..]
            .iter()
            .map(|record| (record.kind, record.value))
            .collect();
        assert_eq!(
            tail,
            [
                (u8::try_from(point::POINT_HT).unwrap(), 6),
                (u8::try_from(point::POINT_MAX_HP).unwrap(), 840),
                (u8::try_from(point::POINT_MAX_STAMINA).unwrap(), 830),
            ]
        );

        points.apply_point(APPLY_INT, 3);
        assert_eq!(points.get_point(point::POINT_IQ), 6);
        assert_eq!(points.max_sp(), 260 + 3 * 20);
        assert_eq!(points.sp(), 260);
    }

    #[test]
    fn an_apply_of_a_skill_adds_or_takes_away_its_damage_bonus() {
        let mut points = warrior();
        assert!(points
            .apply_point(APPLY_SKILL, skill_apply(5, true, 10))
            .is_empty());
        assert!(points
            .apply_point(APPLY_SKILL, skill_apply(5, false, 4))
            .is_empty());
        assert_eq!(points.skill_damage_bonus(5), 6);
        // A skill above 127 sets the sign bit, which must not reach the skill or the change.
        points.apply_point(APPLY_SKILL, skill_apply(200, true, 0x0012_3456));
        assert_eq!(points.skill_damage_bonus(200), 0x0012_3456);
        points.apply_point(APPLY_SKILL, skill_apply(200, false, 0x0012_3450));
        assert_eq!(points.skill_damage_bonus(200), 6);
        assert_eq!(points.skill_damage_bonus(6), 0);
        assert_eq!(points.get_point(point::POINT_SKILL_DAMAGE_BONUS), 0);
    }

    #[test]
    fn an_apply_of_a_maximum_keeps_the_pools_share_in_single_precision() {
        let mut points = warrior();
        points
            .point_change(point::POINT_HP, -380, false, false)
            .unwrap();
        points.apply_point(APPLY_MAX_HP, 240);
        assert_eq!((points.hp(), points.max_hp()), (500, 1000));

        // 7777 / 7777 of 8017 is 8017 exactly, but `(float)8017 / 7777` is a little short of
        // the true ratio, so the full pool ends one point below its maximum, as in legacy.
        let mut points = warrior();
        points
            .point_change(point::POINT_MAX_HP, 7017, false, false)
            .unwrap();
        points
            .point_change(point::POINT_HP, 7017, false, false)
            .unwrap();
        points.apply_point(APPLY_MAX_HP, 240);
        assert_eq!((points.hp(), points.max_hp()), (7776 + 240, 8017));

        // And here single precision lands on the integer that the true share falls short of.
        let mut points = warrior();
        points
            .point_change(point::POINT_MAX_HP, 7017, false, false)
            .unwrap();
        points
            .point_change(point::POINT_HP, 2502 - 760, false, false)
            .unwrap();
        points.apply_point(APPLY_MAX_HP, 9011 - 7777);
        assert_eq!((points.hp(), points.max_hp()), (2502 + 397, 9011));

        let mut points = warrior();
        points.apply_point(APPLY_MAX_HP_PCT, 10);
        assert_eq!((points.hp(), points.max_hp()), (836, 836));
        assert_eq!(points.get_point(point::POINT_MAX_HP_PCT), 10);

        let mut points = warrior();
        points
            .point_change(point::POINT_SP, -130, false, false)
            .unwrap();
        points.apply_point(APPLY_MAX_SP, 40);
        assert_eq!((points.sp(), points.max_sp()), (150, 300));
    }

    #[test]
    fn an_apply_of_a_maximum_does_nothing_while_the_maximum_is_zero() {
        let mut points = warrior();
        points.apply_point(APPLY_MAX_HP, -760);
        assert_eq!((points.hp(), points.max_hp()), (0, 0));
        assert!(points.apply_point(APPLY_MAX_HP, 100).is_empty());
        assert_eq!((points.hp(), points.max_hp()), (0, 0));
    }

    #[test]
    fn an_apply_of_any_other_type_changes_its_one_slot() {
        let mut points = warrior();
        assert!(points.apply_point(APPLY_NONE, 99).is_empty());
        let records = points.apply_point(APPLY_ATTBONUS_HUMAN, 7);
        assert_eq!(records, [record(point::POINT_ATTBONUS_HUMAN, 0, 7)]);
        // Each boss and metin type raises the point of its own name (see `apply.rs`).
        points.apply_point(APPLY_ATTBONUS_BOSS, 3);
        points.apply_point(APPLY_ATTBONUS_METIN, 5);
        assert_eq!(points.get_point(point::POINT_ATTBONUS_BOSS), 3);
        assert_eq!(points.get_point(point::POINT_ATTBONUS_METIN), 5);
    }

    #[test]
    fn an_apply_is_ported_unless_its_slot_is_not() {
        let mut unported = Vec::new();
        let mut armless = Vec::new();
        for apply in 0..=u8::MAX {
            let ported = apply_is_ported(apply);
            if !ported {
                unported.push(apply);
            }
            let ApplyArm::Point(slot) = ApplyArm::of(apply) else {
                assert!(ported, "{apply}");
                continue;
            };
            assert!(usize::from(apply) < MAX_APPLY_NUM, "{apply}");
            let mut points = warrior();
            let refused = match points.point_change(slot, 0, false, false) {
                Ok(_) => {
                    assert!(ported, "{apply}");
                    continue;
                }
                Err(PointChangeRefused::NotPorted(refused)) => {
                    assert!(!ported, "{apply}");
                    refused
                }
                // Legacy's `PointChange` has no arm for the slot either: its `default` logs and
                // returns, so the apply changes nothing there and here.
                Err(PointChangeRefused::Unknown(refused)) => {
                    assert!(ported, "{apply}");
                    armless.push(apply);
                    refused
                }
            };
            assert_eq!(refused, slot, "{apply}");
            let before = points.clone();
            assert!(points.apply_point(apply, 5).is_empty(), "{apply}");
            assert_eq!(points, before, "{apply}");
        }
        assert_eq!(unported, [APPLY_ENERGY, APPLY_COSTUME_ATTR_BONUS]);
        assert_eq!(armless, [APPLY_MAGIC_ATTBONUS_PER]);
    }

    #[test]
    fn the_worn_items_apply_in_the_computation_and_the_hit_points_are_put_back() {
        let armour = applying(
            valued(proto(11_210, ITEM_ARMOR, ARMOR_BODY), [0, 40, 0, 0, 0, 0]),
            [
                (APPLY_CON, 2),
                (APPLY_MAX_HP, 240),
                (APPLY_SKILL, skill_apply(5, true, 10)),
            ],
        );
        let protos = ItemProtos::from_rows(vec![armour]);
        let body = Item::new(1, 11_210);
        let worn = wearing(&[(BODY, &body)]);
        let equipment = Equipment::of(&worn, &protos);

        let mut points = warrior();
        points.compute_points_with(&equipment);
        assert_eq!(points.get_point(point::POINT_HT), 6);
        assert_eq!(points.max_hp(), 760 + 80 + 240);
        assert_eq!(points.max_stamina(), 830);
        // `APPLY_MAX_HP` raised the pool to keep its share, and the `@fixme118` restore put the
        // difference back.
        assert_eq!(points.hp(), 760);
        assert_eq!(points.skill_damage_bonus(5), 10);
        // The level, four fifths of the vitality of 6, and the armour.
        assert_eq!(points.get_point(point::POINT_DEF_GRADE), 1 + 4 + 40);

        // A second computation starts again from nothing.
        points.compute_points_with(&equipment);
        assert_eq!(points.max_hp(), 1080);
        assert_eq!(points.skill_damage_bonus(5), 10);

        // Taking the armour off lowers the maximum and clamps the pool to it.
        points
            .point_change(point::POINT_HP, 300, false, false)
            .unwrap();
        assert_eq!(points.hp(), 1060);
        points.compute_points();
        assert_eq!((points.hp(), points.max_hp()), (760, 760));
        assert_eq!(points.skill_damage_bonus(5), 0);
        assert_eq!(points.get_point(point::POINT_DEF_GRADE), 1 + 3);
    }

    #[test]
    fn two_pieces_of_a_set_give_its_first_bonus() {
        // "SetBonus - 1": the weapon 19 and the body 11209, each in its own cell.
        let protos = ItemProtos::from_rows(vec![
            proto(19, ITEM_WEAPON, 0),
            applying(
                proto(11_209, ITEM_ARMOR, ARMOR_BODY),
                [(APPLY_MAX_HP, 40), (APPLY_NONE, 0), (APPLY_NONE, 0)],
            ),
        ]);
        let weapon = Item::new(1, 19);
        let body = Item::new(2, 11_209);
        let worn = wearing(&[(WEAPON, &weapon), (BODY, &body)]);
        let mut points = warrior();
        points.compute_points_with(&Equipment::of(&worn, &protos));
        assert_eq!(points.max_hp(), 760 + 40 + 500);
        assert_eq!(points.hp(), 760);

        // One piece alone gives nothing.
        let worn = wearing(&[(BODY, &body)]);
        points.compute_points_with(&Equipment::of(&worn, &protos));
        assert_eq!(points.max_hp(), 760 + 40);
    }

    #[test]
    fn an_attribute_change_recomputes_the_battle_points_before_its_record() {
        let mut points = warrior();
        let mut expected = vec![
            record(point::POINT_ATT_GRADE, 0, 20),
            record(point::POINT_CLIENT_DEF_GRADE, 0, 4),
            record(point::POINT_DEF_GRADE, 0, 4),
            record(point::POINT_CLIENT_DEF_GRADE, 0, 5),
            record(point::POINT_MAGIC_ATT_GRADE, 0, 8),
            record(point::POINT_MAGIC_DEF_GRADE, 0, 5),
        ];
        expected.push(record(point::POINT_ST, 3, 9));
        assert_eq!(
            points.point_change(point::POINT_ST, 3, true, false),
            Ok(expected)
        );
        for kind in [
            point::POINT_HT,
            point::POINT_DX,
            point::POINT_IQ,
            point::POINT_SUNGMA_STR,
            point::POINT_SUNGMA_HP,
            point::POINT_SUNGMA_MOVE,
            point::POINT_SUNGMA_IMMUNE,
        ] {
            let records = points.point_change(kind, 1, false, false).unwrap();
            assert_eq!(records.len(), 7, "{kind}");
            assert_eq!(records[6].kind, u8::try_from(kind).unwrap());
        }
        let records = points
            .point_change(point::POINT_HP_REGEN, 1, false, false)
            .unwrap();
        assert_eq!(records, [record(point::POINT_HP_REGEN, 0, 1)]);
    }

    #[test]
    fn the_item_shop_and_double_bonuses_cap_at_100() {
        for kind in CAPPED {
            let mut points = warrior();
            assert_eq!(
                points.point_change(kind, 70, true, false),
                Ok(vec![record(kind, 70, 70)])
            );
            assert_eq!(
                points.point_change(kind, 50, true, false),
                Ok(vec![record(kind, 30, 100)])
            );
            assert_eq!(
                points.point_change(kind, -20, true, false),
                Ok(vec![record(kind, -20, 80)])
            );
        }
        assert_eq!(
            CAPPED.map(|kind| u8::try_from(kind).unwrap()),
            [114, 115, 116, 117, 118, 132, 83, 84, 85, 86]
        );
    }

    #[test]
    fn the_ramadan_bonus_is_set_not_added() {
        let mut points = warrior();
        let kind = point::POINT_RAMADAN_CANDY_BONUS_EXP;
        points.point_change(kind, 30, false, false).unwrap();
        assert_eq!(
            points.point_change(kind, 20, true, false),
            Ok(vec![record(kind, 20, 20)])
        );
    }

    #[test]
    fn an_immunity_slot_sets_its_bit_while_it_is_not_zero() {
        let mut points = warrior();
        for (kind, bit) in [
            (point::POINT_IMMUNE_STUN, IMMUNE_STUN),
            (point::POINT_IMMUNE_SLOW, IMMUNE_SLOW),
            (point::POINT_IMMUNE_FALL, IMMUNE_FALL),
        ] {
            points.point_change(kind, 1, false, false).unwrap();
            assert_eq!(points.immune_flag() & bit, bit);
            points.point_change(kind, 1, false, false).unwrap();
            points.point_change(kind, -1, false, false).unwrap();
            assert_eq!(points.immune_flag() & bit, bit, "still 1");
            points.point_change(kind, -1, false, false).unwrap();
            assert_eq!(points.immune_flag() & bit, 0);
        }
        points
            .point_change(point::POINT_IMMUNE_SLOW, 1, false, false)
            .unwrap();
        assert_eq!(points.immune_flag(), IMMUNE_SLOW);
        points.compute_points();
        assert_eq!(points.immune_flag(), 0);
        assert_eq!((IMMUNE_STUN, IMMUNE_SLOW, IMMUNE_FALL), (1, 2, 4));
    }

    #[test]
    fn the_counters_are_kept_in_both_arrays_and_survive_a_recomputation() {
        let mut points = warrior();
        for kind in COUNTERS {
            assert_eq!(
                points.point_change(kind, 4, true, false),
                Ok(vec![record(kind, 4, 4)])
            );
            assert_eq!(points.real_point(kind), 4);
        }
        points
            .point_change(point::POINT_PARTY_HASTE_BONUS, 10, false, false)
            .unwrap();
        points
            .point_change(point::POINT_HP_RECOVERY, 6, false, false)
            .unwrap();
        points
            .point_change(point::POINT_ATTBONUS_HUMAN, 9, false, false)
            .unwrap();
        let records = points.compute_points();
        assert_eq!(records[0], record(point::POINT_ATT_SPEED, 0, 110));
        for kind in COUNTERS {
            assert_eq!(points.get_point(kind), 4);
        }
        assert_eq!(points.get_point(point::POINT_HP_RECOVERY), 6);
        assert_eq!(points.get_point(point::POINT_ATTBONUS_HUMAN), 0);
    }

    #[test]
    fn the_conqueror_point_is_kept_in_both_arrays_and_survives_a_recomputation() {
        let mut points = warrior();
        let kind = point::POINT_CONQUEROR_POINT;
        assert_eq!(
            points.point_change(kind, 3, true, false),
            Ok(vec![record(kind, 3, 3)])
        );
        assert_eq!(points.real_point(kind), 3);
        points.compute_points();
        assert_eq!(points.get_point(kind), 3);
    }

    #[test]
    fn the_next_level_cost_is_written_without_the_change() {
        let mut points = warrior();
        assert_eq!(
            points.point_change(point::POINT_NEXT_EXP, 123, true, false),
            Ok(vec![record(point::POINT_NEXT_EXP, 0, levels::next_exp(1))])
        );
        assert_eq!(
            points.point_change(point::POINT_CONQUEROR_NEXT_EXP, 9, true, true),
            Ok(vec![PointRecord {
                kind: 176,
                amount: 0,
                value: levels::conqueror_next_exp(0),
                broadcast: true,
            }])
        );
    }

    #[test]
    fn the_voice_and_empire_slots_cannot_be_changed() {
        let mut points = warrior();
        for kind in [point::POINT_VOICE, point::POINT_EMPIRE_POINT] {
            assert_eq!(
                points.point_change(kind, 5, true, false),
                Ok(vec![record(kind, 5, 0)])
            );
            assert_eq!(points.get_point(kind), 0);
        }
    }

    #[test]
    fn a_slot_legacy_has_no_arm_for_changes_nothing() {
        let before = warrior();
        for kind in [
            point::POINT_ATTBONUS_INSECT,
            point::POINT_ATTBONUS_FIRE,
            point::POINT_ATTBONUS_ICE,
            point::POINT_ATTBONUS_DESERT,
            point::POINT_ATTBONUS_TREE,
            point::POINT_MAGIC_ATT_BONUS_PER,
            point::POINT_PLAYTIME,
            point::POINT_MAX_NUM,
            256,
            usize::MAX,
        ] {
            let mut points = before.clone();
            assert_eq!(
                points.point_change(kind, 5, true, false),
                Err(PointChangeRefused::Unknown(kind))
            );
            assert_eq!(points, before);
        }
    }

    #[test]
    fn an_arm_the_rewrite_has_not_ported_changes_nothing() {
        let before = warrior();
        for kind in NOT_PORTED {
            let mut points = before.clone();
            assert_eq!(
                points.point_change(kind, 5, true, false),
                Err(PointChangeRefused::NotPorted(kind))
            );
            assert_eq!(points, before);
        }
        assert!(NOT_PORTED.contains(&point::POINT_LEVEL));
        assert!(NOT_PORTED.contains(&point::POINT_GOLD));
    }

    #[test]
    fn the_none_slot_changes_nothing_and_writes_nothing() {
        let before = warrior();
        let mut points = before.clone();
        assert_eq!(
            points.point_change(point::POINT_NONE, 5, true, false),
            Ok(vec![])
        );
        assert_eq!(points, before);
    }

    #[test]
    fn every_plain_slot_adds_and_writes_its_value() {
        for kind in PLAIN {
            if RECOMPUTES_BATTLE.contains(&kind) {
                continue;
            }
            let mut points = warrior();
            let start = points.get_point(kind);
            let records = points.point_change(kind, 7, true, false).unwrap();
            let expected = i64::from(start + 7);
            let written = records.last().unwrap();
            assert_eq!(
                (written.kind, written.amount),
                (u8::try_from(kind).unwrap(), 7)
            );
            assert_eq!(written.value, expected, "{kind}");
            assert_eq!(points.get_point(kind), start + 7);
        }
        assert_eq!(PLAIN.len(), 119);
    }

    #[test]
    fn the_arm_groups_do_not_overlap() {
        let groups: [&[usize]; 4] = [&PLAIN, &CAPPED, &COUNTERS, &NOT_PORTED];
        for (i, left) in groups.iter().enumerate() {
            for right in &groups[i + 1..] {
                assert!(left.iter().all(|kind| !right.contains(kind)));
            }
        }
    }

    #[test]
    fn a_slot_past_the_array_reads_zero() {
        let points = warrior();
        for kind in [point::POINT_MAX_NUM, 256, usize::MAX] {
            assert_eq!(points.get_point(kind), 0);
            assert_eq!(points.real_point(kind), 0);
            assert_eq!(points.limit_point(kind), 0);
        }
    }

    #[test]
    fn the_limits_hold_the_speeds_steals_and_item_shop_bonuses() {
        let mut points = warrior();
        points
            .point_change(point::POINT_ATT_SPEED, 100, false, false)
            .unwrap();
        assert_eq!(points.limit_point(point::POINT_ATT_SPEED), 170);
        points
            .point_change(point::POINT_ATT_SPEED, -300, false, false)
            .unwrap();
        assert_eq!(points.limit_point(point::POINT_ATT_SPEED), 0);
        points
            .point_change(point::POINT_MOV_SPEED, 150, false, false)
            .unwrap();
        assert_eq!(points.limit_point(point::POINT_MOV_SPEED), 200);
        for (kind, limit) in [
            (point::POINT_STEAL_HP, 50),
            (point::POINT_STEAL_SP, 50),
            (point::POINT_MALL_ATTBONUS, 20),
            (point::POINT_MALL_DEFBONUS, 20),
        ] {
            points.point_change(kind, 60, false, false).unwrap();
            assert_eq!(points.get_point(kind), 60);
            assert_eq!(points.limit_point(kind), limit);
        }
        points
            .point_change(point::POINT_ATTBONUS_HUMAN, i32::MIN, false, false)
            .unwrap();
        assert_eq!(points.get_point(point::POINT_ATTBONUS_HUMAN), i32::MIN);
        assert_eq!(points.limit_point(point::POINT_ATTBONUS_HUMAN), -i32::MAX);
        assert_eq!(points.limit_point(point::POINT_ATT_GRADE), 14);
    }

    #[test]
    fn a_sum_saturates_instead_of_wrapping() {
        let mut points = warrior();
        let kind = point::POINT_ATTBONUS_HUMAN;
        points.point_change(kind, i32::MAX, false, false).unwrap();
        points.point_change(kind, i32::MAX, false, false).unwrap();
        assert_eq!(points.get_point(kind), i32::MAX);
        assert_eq!(narrow(i64::MIN), i32::MIN);
        assert_eq!(narrow(-5), -5);
    }

    fn conqueror_on(map_index: i32, sungma: [u8; 4]) -> Points {
        let mut row = fresh(0);
        row.conqueror_level = 1;
        row.sungma = sungma;
        row.map_index = map_index;
        Points::load(&row)
    }

    #[test]
    fn the_will_table_is_legacy_s() {
        assert_eq!(sungma_will(373), will(15, 10, 15, 20));
        assert_eq!(sungma_will(374), will(0, 0, 1, 0));
        assert_eq!(sungma_will(384), will(75, 60, 65, 65));
        for index in [0, 1, 41, 372, 375, 378, 385] {
            assert_eq!(sungma_will(index), SungmaWill::default());
        }
        let maps = SUNGMA_WILL_MAPS.map(|(index, _)| index);
        assert_eq!(maps, [374, 373, 376, 377, 382, 383, 384]);
    }

    #[test]
    fn a_weak_conqueror_has_half_the_hit_points_and_speed_on_a_will_map() {
        let points = conqueror_on(373, [0; 4]);
        assert_eq!(points.will(), will(15, 10, 15, 20));
        assert_eq!(points.max_hp(), 380);
        assert_eq!(points.real_point(point::POINT_MAX_HP), 760);
        assert_eq!(points.limit_point(point::POINT_MOV_SPEED), 50);
        assert_eq!(points.get_point(point::POINT_MOV_SPEED), 100);
        // The maximum spell points and the attack speed are never halved.
        assert_eq!(points.max_sp(), 260);
        assert_eq!(points.limit_point(point::POINT_ATT_SPEED), 100);
    }

    #[test]
    fn the_will_needs_a_conqueror_level_and_a_point_below_it() {
        let mut row = fresh(0);
        row.map_index = 373;
        let points = Points::load(&row);
        assert_eq!(points.will(), SungmaWill::default());
        assert_eq!(points.max_hp(), 760);
        assert_eq!(points.limit_point(point::POINT_MOV_SPEED), 100);

        // Equal to the will is enough: legacy halves only when the will is greater.
        let points = conqueror_on(373, [0, 10, 15, 0]);
        assert_eq!(points.max_hp(), 760);
        assert_eq!(points.limit_point(point::POINT_MOV_SPEED), 100);
        let points = conqueror_on(373, [0, 9, 14, 0]);
        assert_eq!(points.max_hp(), 380);
        assert_eq!(points.limit_point(point::POINT_MOV_SPEED), 50);

        let points = conqueror_on(1, [0; 4]);
        assert_eq!(points.max_hp(), 760);
    }

    #[test]
    fn the_movement_speed_is_halved_before_it_is_limited_and_in_its_record() {
        let mut points = conqueror_on(373, [0; 4]);
        points
            .point_change(point::POINT_MOV_SPEED, 200, false, false)
            .unwrap();
        assert_eq!(points.limit_point(point::POINT_MOV_SPEED), 150);
        assert_eq!(
            points.point_change(point::POINT_MOV_SPEED, 1, true, false),
            Ok(vec![record(point::POINT_MOV_SPEED, 1, 150)])
        );
        points.set_map_index(1);
        assert_eq!(
            points.point_change(point::POINT_MOV_SPEED, 1, true, false),
            Ok(vec![record(point::POINT_MOV_SPEED, 1, 302)])
        );
        assert_eq!(points.limit_point(point::POINT_MOV_SPEED), 200);
    }

    #[test]
    fn a_weak_conqueror_s_hit_point_record_carries_the_halved_maximum() {
        let mut points = conqueror_on(376, [0; 4]);
        assert_eq!(
            points.point_change(point::POINT_MAX_HP, 0, false, false),
            Ok(vec![record(point::POINT_MAX_HP, 0, 380)])
        );
        points.hp = 700;
        assert_eq!(
            points.check_maximum_points(),
            [record(point::POINT_HP, 0, 380)]
        );
        // The halving needs a positive maximum.
        points
            .point_change(point::POINT_MAX_HP, -760, false, false)
            .unwrap();
        assert_eq!(points.max_hp(), 0);
        points
            .point_change(point::POINT_MAX_HP, -1, false, false)
            .unwrap();
        assert_eq!(points.max_hp(), -1);
    }

    #[test]
    fn a_weak_conqueror_s_heal_stops_at_the_halved_maximum() {
        let mut points = conqueror_on(376, [0; 4]);
        points.hp = 300;
        assert_eq!(
            points.point_change(point::POINT_HP, 200, true, false),
            Ok(vec![record(point::POINT_HP, 80, 380)])
        );
        assert_eq!(points.hp(), 380);
        // At the halved maximum a heal changes nothing and writes nothing.
        assert_eq!(
            points.point_change(point::POINT_HP, 1, true, false),
            Ok(vec![])
        );
        assert_eq!(points.hp(), 380);
    }

    #[test]
    fn the_load_keeps_the_sungma_points_and_the_inventory_pages() {
        let mut row = fresh(0);
        row.sungma = [34, 35, 36, 37];
        row.inven_point = 3;
        let points = Points::load(&row);
        for (kind, value) in SUNGMA_SLOTS.into_iter().zip([34, 35, 36, 37]) {
            assert_eq!(points.get_point(kind), value);
            assert_eq!(points.real_point(kind), value);
        }
        assert_eq!(points.get_point(point::POINT_INVEN), 3);
        assert_eq!(points.inven_point(), 3);
        assert_eq!(points.level(), 1);
        assert_eq!(points.real_point(point::POINT_ST), 6);
    }
}
