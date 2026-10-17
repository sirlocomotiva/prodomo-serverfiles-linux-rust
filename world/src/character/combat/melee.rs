//! A player's melee hit on a monster.
//!
//! This is `battle_hit` (`G/battle.cpp:760-817`), the `CalcMeleeDamage` it calls (`:489-666`)
//! with the arms of `CalcAttBonus` a player attacker and a non-player victim run (`:299-415`),
//! and the `DAMAGE_TYPE_NORMAL` path of `CHARACTER::Damage` (`G/char_battle.cpp:1744-2436`) up to
//! the hit points the victim loses. Every random draw is taken from the `Dice` in legacy's order;
//! what the hit shows comes back as [`HitEvent`]s in the order legacy sends them. The caller
//! delivers them and runs what follows a hit, which needs the world: `StartRecoveryEvent`, `Stun`
//! and the killer's PID when the hit points reach 0, and `Dead` after a hit on a stunned victim.
//!
//! What the Rewrite leaves out, because nothing it hosts can reach it:
//! * the attacker is never polymorphed and holds no marriage bonus, so `GetPolymorphPoint` is
//!   `GetPoint` and every `GetMarriageBonus` is 0;
//! * a non-player holds no affect and no skill, so `AFF_JEONGWIHON`, `AFF_TERROR`, `AFF_HOSIN`
//!   and `AFF_MANASHIELD` never apply, the `SKILL_RESIST_PENETRATE` poly is 0, and the victim of
//!   `NormalAttackAffect` is never already poisoned, stunned or slowed;
//! * no proto column or apply gives a non-player `POINT_BLOCK` or `POINT_REFLECT_MELEE`, so there
//!   is no block draw and no reflected damage;
//! * the quest-set multipliers `attMul` and `damMul` are 1.0, `cannot_dead` is false, the user
//!   damage rate is 100 and `BlueDragon_Damage` is the identity for every race the Rewrite stands
//!   up.
//!
//! What it draws but does not yet apply: the poison, stun and slow of `NormalAttackAffect` wait
//! for the affect work, and the stolen gold for `POINT_GOLD`.

use std::ops::RangeInclusive;

use common::cfloat::{f32_to_i32, f64_to_i32, i32_to_f32};
use common::point_slot as point;
use gamedata::item_kind::{
    ITEM_WEAPON, WEAPON_BELL, WEAPON_BOW, WEAPON_DAGGER, WEAPON_FAN, WEAPON_MOUNT_SPEAR,
    WEAPON_SWORD, WEAPON_TWO_HANDED,
};
use gamedata::item_proto::ItemProto;
use gamedata::mob_proto::{AIFLAG_STONESKIN, CHAR_TYPE_MONSTER, CHAR_TYPE_STONE};

use super::{attack_rating, AttackRatingInput};
use crate::character::{number, race_to_job, Dice, PointRecord, Points};
use crate::npc::{map_empire, MobFight, Npc};

/// `SE_CRITICAL` (`C/length.h:856`): the effect a critical hit shows on its victim.
pub const SE_CRITICAL: u8 = 5;
/// `SE_PENETRATE`: the effect a penetrating hit shows, sent because `ENABLE_EFFECT_PENETRATE`
/// is defined (`G/char_battle.cpp:69`).
pub const SE_PENETRATE: u8 = 6;
/// `FLY_SP_SMALL` (`G/char.h:94`): the small blue orb of a stolen or recovered spell point.
pub const FLY_SP_SMALL: u8 = 4;
/// `FLY_HP_SMALL` (`G/char.h:105`): the small red orb of a stolen or recovered hit point.
pub const FLY_HP_SMALL: u8 = 15;
/// `DAMAGE_NORMAL` (`G/char.h:131`).
pub const DAMAGE_NORMAL: u8 = 1 << 0;
/// `DAMAGE_PENETRATE`.
pub const DAMAGE_PENETRATE: u8 = 1 << 4;
/// `DAMAGE_CRITICAL`.
pub const DAMAGE_CRITICAL: u8 = 1 << 5;

/// `MONSTER_CHAT_ATTACKED` (`G/constants.h:11`): the chat a monster may say when it is hit.
const MONSTER_CHAT_ATTACKED: i32 = 3;
/// `MOB_RANK_BOSS`: from this rank up `POINT_ATTBONUS_BOSS` applies.
const MOB_RANK_BOSS: u8 = 4;
/// `BATTLE_TYPE_MELEE`: the battle type whose reach follows its attack range.
const BATTLE_TYPE_MELEE: u8 = 0;
/// `HYDRA1` to `HYDRA3` (`G/ShipDefense.h:41-43`): `IsHydra`, which a player reaches from
/// 600 further.
const HYDRAS: [u32; 3] = [3960, 3961, 3962];
/// `IsMinion` (`G/ShipDefense.cpp:1925-1934`): the ship-defence monsters whose melee range is
/// 300 longer.
const MINIONS: [RangeInclusive<u32>; 2] = [3401..=3605, 3950..=3959];

/// The weapon sub-types `CalcMeleeDamage` swings; any other weapon deals 0.
const MELEE_WEAPONS: [i32; 6] = [
    WEAPON_SWORD,
    WEAPON_DAGGER,
    WEAPON_TWO_HANDED,
    WEAPON_BELL,
    WEAPON_FAN,
    WEAPON_MOUNT_SPEAR,
];

/// `CalcAttBonus`'s race arms, in its order: the victim's `RACE_FLAG_*` bit and the attacker's
/// point that adds its share.
const RACE_BONUSES: [(u32, usize); 17] = [
    (1 << 0, point::POINT_ATTBONUS_ANIMAL),
    (1 << 1, point::POINT_ATTBONUS_UNDEAD),
    (1 << 2, point::POINT_ATTBONUS_DEVIL),
    (1 << 3, point::POINT_ATTBONUS_HUMAN),
    (1 << 4, point::POINT_ATTBONUS_ORC),
    (1 << 5, point::POINT_ATTBONUS_MILGYO),
    (1 << 6, point::POINT_ATTBONUS_INSECT),
    (1 << 7, point::POINT_ATTBONUS_FIRE),
    (1 << 8, point::POINT_ATTBONUS_ICE),
    (1 << 9, point::POINT_ATTBONUS_DESERT),
    (1 << 10, point::POINT_ATTBONUS_TREE),
    (1 << 11, point::POINT_ENCHANT_ELECT),
    (1 << 12, point::POINT_ENCHANT_FIRE),
    (1 << 13, point::POINT_ENCHANT_ICE),
    (1 << 14, point::POINT_ENCHANT_WIND),
    (1 << 15, point::POINT_ENCHANT_EARTH),
    (1 << 16, point::POINT_ENCHANT_DARK),
];

/// The victim's resist to each job, by `GetJob`: the share `CalcAttBonus` takes back last.
const JOB_RESISTS: [usize; 4] = [
    point::POINT_RESIST_WARRIOR,
    point::POINT_RESIST_ASSASSIN,
    point::POINT_RESIST_SURA,
    point::POINT_RESIST_SHAMAN,
];

/// The player who swings.
#[derive(Debug)]
pub struct Striker<'a> {
    /// Its points, which a stolen or recovered point and a drained spell point change.
    pub points: &'a mut Points,
    /// The proto of what it wears in `WEAR_WEAPON`.
    pub weapon: Option<&'a ItemProto>,
    /// `GetEmpire`.
    pub empire: u8,
    /// `GetMapIndex`.
    pub map: i32,
}

/// What a hit shows, in the order legacy sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitEvent {
    /// `EffectPacket`: a `GC_SPECIAL_EFFECT` of this type on the victim, to its view.
    Effect(u8),
    /// `CreateFly`: a `GC_CREATE_FLY` of this type from the victim to the attacker, to the
    /// victim's view.
    Fly(u8),
    /// The attacker's own `PointChange` records.
    Attacker(Vec<PointRecord>),
    /// The victim's hit points changed: its `BroadcastTargetPacket`.
    VictimHp,
    /// `MonsterChat(MONSTER_CHAT_ATTACKED)` chose this line of the race's `monster_chat` table.
    Chat(i32),
    /// `SendDamagePacket(attacker, damage, flag)`, before the damage is held to the hit points.
    DamageInfo {
        /// The damage shown.
        damage: i32,
        /// `DAMAGE_NORMAL`, with `DAMAGE_CRITICAL` and `DAMAGE_PENETRATE` when they hit.
        flag: u8,
    },
}

/// How a hit ended, which decides what the caller runs next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Struck {
    /// `CalcMeleeDamage` gave 0, which only a weapon it does not swing does: `battle_hit`
    /// returns `BATTLE_DAMAGE` without calling `Damage`.
    Whiffed,
    /// The victim stands on a `ATTR_BANPK` cell or off the sectree: `Damage` returns false
    /// before the hit points.
    Refused,
    /// The victim was stunned: `Damage` calls `Dead(attacker)` and the hit kills.
    Finished,
    /// The victim was already dead.
    AlreadyDead,
    /// The hit took `damage` hit points: `StartRecoveryEvent` follows when it is above 0, and
    /// `Stun` when the victim's hit points reached 0.
    Landed {
        /// The hit points taken.
        damage: i32,
    },
}

/// One hit: what it showed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strike {
    /// What it showed, in order.
    pub events: Vec<HitEvent>,
    /// How it ended.
    pub struck: Struck,
}

/// `battle_melee_attack`'s reach for a player hitting `victim`: 300, or for a `MELEE` monster
/// its attack range plus 15 % but at least 350, and 600 more for a hydra (`G/battle.cpp:174-201`).
#[must_use]
pub fn melee_reach(victim: &Npc) -> i32 {
    let mut reach = 300;
    if victim.fight.battle_type == BATTLE_TYPE_MELEE {
        let range = i32_to_f32(i32::from(mob_attack_range(victim)));
        reach = f32_to_i32(range * 1.15).max(350);
    }
    if HYDRAS.contains(&victim.vnum) {
        reach += 600;
    }
    reach
}

/// `GetMobAttackRange` of a monster that is neither `RANGE` nor `MAGIC` (`G/char.cpp:2656-2676`):
/// its proto's range, 300 longer for a ship-defence minion, kept in a `WORD`.
#[must_use]
pub fn mob_attack_range(victim: &Npc) -> u16 {
    let range = victim.fight.attack_range;
    if MINIONS.iter().any(|minions| minions.contains(&victim.vnum)) {
        range.wrapping_add(300)
    } else {
        range
    }
}

/// `battle_hit` of a player on a non-player `victim` that `banned` says stands on an
/// `ATTR_BANPK` cell or off the sectree.
pub fn strike_npc(
    dice: &mut dyn Dice,
    striker: &mut Striker<'_>,
    victim: &mut Npc,
    banned: bool,
) -> Strike {
    let mut events = Vec::new();
    let damage = calc_melee_damage(dice, striker.points, striker.weapon, victim);
    if damage <= 0 {
        return Strike {
            events,
            struck: Struck::Whiffed,
        };
    }
    normal_attack_affect(dice, striker.points);
    let damage = weapon_resist(damage, striker.weapon, &victim.fight);
    // `iDam = attMul * tempIDam + 0.5f`, with `attMul` 1.0, which multiplies exactly.
    let damage = f32_to_i32(i32_to_f32(damage) + 0.5);
    let struck = damage_normal(dice, striker, victim, damage, banned, &mut events);
    Strike { events, struck }
}

/// `CalcMeleeDamage` of a player who is not polymorphed on a non-player.
pub fn calc_melee_damage(
    dice: &mut dyn Dice,
    attacker: &Points,
    weapon: Option<&ItemProto>,
    victim: &Npc,
) -> i32 {
    if let Some(weapon) = weapon {
        if weapon.item_type != ITEM_WEAPON || !MELEE_WEAPONS.contains(&weapon.sub_type) {
            return 0;
        }
    }
    let level = i32::from(attacker.level());
    let rating = attack_rating(AttackRatingInput {
        attacker_dexterity: attacker.get_point(point::POINT_DX),
        attacker_level: level,
        victim_dexterity: victim.fight.point(point::POINT_DX),
        ignore_target_rating: false,
    });
    // `Item_GetDamage`: the weapon's values 3 and 4; an unarmed player rolls 0 to 0.
    let (low, high) = weapon.map_or((0, 0), |weapon| (weapon.values[3], weapon.values[4]));
    let rolled = number(dice, low, high).wrapping_mul(2);
    let refine = weapon.map_or(0, |weapon| weapon.values[5]);
    let percent = attacker
        .get_point(point::POINT_ATT_BONUS)
        .wrapping_add(attacker.get_point(point::POINT_MELEE_MAGIC_ATT_BONUS_PER));
    let attack = melee_attack(&MeleeAttack {
        level,
        attack_grade: attacker.get_point(point::POINT_ATT_GRADE),
        rolled,
        rating,
        refine,
        party_bonus: attacker.get_point(point::POINT_PARTY_ATTACKER_BONUS),
        percent,
    });
    let attack = att_bonus_on_npc(attack, attacker, victim);
    let defence = victim
        .fight
        .point(point::POINT_DEF_GRADE)
        .wrapping_mul(100_i32.wrapping_add(victim.fight.point(point::POINT_DEF_BONUS)))
        .wrapping_div(100);
    let mut damage = attack.wrapping_sub(defence).max(0);
    if i32::from(attacker.will().strength) > attacker.get_point(point::POINT_SUNGMA_STR)
        && damage > 0
    {
        damage /= 2;
    }
    battle_damage_floor(dice, damage)
}

/// `CalcBattleDamage` without the level delta (`G/battle.cpp:244-251`): below 3, `number(1, 5)`.
/// A monster victim's level delta never applies here: the player attacker is never polymorphed.
pub(crate) fn battle_damage_floor(dice: &mut dyn Dice, damage: i32) -> i32 {
    if damage < 3 {
        number(dice, 1, 5)
    } else {
        damage
    }
}

/// The attack `CalcMeleeDamage` builds before `CalcAttBonus`.
#[derive(Debug, Clone, Copy)]
struct MeleeAttack {
    level: i32,
    attack_grade: i32,
    /// The weapon's roll, already doubled.
    rolled: i32,
    rating: f32,
    /// The weapon's value 5, which is added twice.
    refine: i32,
    party_bonus: i32,
    /// `POINT_ATT_BONUS` plus `POINT_MELEE_MAGIC_ATT_BONUS_PER`.
    percent: i32,
}

/// `CalcMeleeDamage` from the attack grade to the attack-bonus percent, with C's `int` and
/// `float` order: the level is taken out before the rating and put back after it.
fn melee_attack(input: &MeleeAttack) -> i32 {
    let level_twice = input.level.wrapping_mul(2);
    let attack = input
        .attack_grade
        .wrapping_add(input.rolled)
        .wrapping_sub(level_twice);
    let attack = f32_to_i32(i32_to_f32(attack) * input.rating);
    attack
        .wrapping_add(level_twice)
        .wrapping_add(input.refine.wrapping_mul(2))
        .wrapping_add(input.party_bonus)
        .wrapping_mul(100_i32.wrapping_add(input.percent))
        .wrapping_div(100)
}

/// `CalcAttBonus` for a player attacker and a non-player victim.
fn att_bonus_on_npc(attack: i32, attacker: &Points, victim: &Npc) -> i32 {
    let share = |attack: i32, slot: usize| attack.wrapping_mul(attacker.get_point(slot)) / 100;
    let mut attack = attack;
    for (flag, slot) in RACE_BONUSES {
        if victim.race_flag & flag != 0 {
            attack = attack.wrapping_add(share(attack, slot));
        }
    }
    // `BONUS_PCT`: the `int` product times the `float` percent, then divided by the `double`
    // 100.0, is added to the attack as a `double` and truncated back.
    let monster = attack.wrapping_mul(attacker.get_point(point::POINT_ATTBONUS_MONSTER));
    let percent = point_pct(attacker.get_point(point::POINT_ATTBONUS_MONSTER_PCT));
    let monster = f64::from(i32_to_f32(monster) * percent) / 100.0;
    attack = f64_to_i32(f64::from(attack) + monster);
    if victim.char_type == CHAR_TYPE_STONE {
        attack = attack.wrapping_add(share(attack, point::POINT_ATTBONUS_METIN));
    } else if victim.fight.rank >= MOB_RANK_BOSS {
        attack = attack.wrapping_add(share(attack, point::POINT_ATTBONUS_BOSS));
    }
    let resist = JOB_RESISTS[usize::from(race_to_job(attacker.race()))];
    attack.wrapping_sub(attack.wrapping_mul(victim.fight.point(resist)) / 100)
}

/// `GetPointPct(type, 1)` (`G/char.cpp:3651-3663`): `(100.0 + point) / 100.0`, computed in
/// `double` and stored in a `float`.
///
/// Dividing in `float` gives the same `float`: the sum is exact in both while it stays below
/// 2^24, and a quotient rounded to `double` and then to `float` is the quotient rounded to
/// `float` once, because 53 bits are at least twice 24 plus 2 (Figueroa, 1995).
pub(crate) fn point_pct(value: i32) -> f32 {
    i32_to_f32(value.wrapping_add(100)) / 100.0
}

/// `NormalAttackAffect`'s draws: the poison, then the stun and the slow of `AttackAffect`. Each
/// draws only when the attacker has the point; the victim is never already affected.
fn normal_attack_affect(dice: &mut dyn Dice, attacker: &Points) {
    let poison = attacker.get_point(point::POINT_POISON_PCT);
    if poison != 0 {
        // `BYTE bPoisonPct`: the low byte, halved when the map's immune will exceeds the
        // attacker's.
        let mut percent = poison.to_le_bytes()[0];
        if i32::from(attacker.will().immune) > attacker.get_point(point::POINT_SUNGMA_IMMUNE) {
            percent /= 2;
        }
        let _poisoned = number(dice, 1, 100) <= i32::from(percent);
    }
    for slot in [point::POINT_STUN_PCT, point::POINT_SLOW_PCT] {
        let chance = attacker.get_point(slot);
        if chance != 0 {
            let _affected = number(dice, 1, 100) <= chance;
        }
    }
}

/// `battle_hit`'s resist to the weapon's sub-type. A mounted spear has none.
fn weapon_resist(damage: i32, weapon: Option<&ItemProto>, victim: &MobFight) -> i32 {
    let Some(weapon) = weapon else {
        return damage;
    };
    let slot = match weapon.sub_type {
        WEAPON_SWORD => point::POINT_RESIST_SWORD,
        WEAPON_TWO_HANDED => point::POINT_RESIST_TWOHAND,
        WEAPON_DAGGER => point::POINT_RESIST_DAGGER,
        WEAPON_BELL => point::POINT_RESIST_BELL,
        WEAPON_FAN => point::POINT_RESIST_FAN,
        WEAPON_BOW => point::POINT_RESIST_BOW,
        _ => return damage,
    };
    damage
        .wrapping_mul(100_i32.wrapping_sub(victim.point(slot)))
        .wrapping_div(100)
}

/// `GetHPPct`: the share of the maximum left, 0 when there is no maximum.
pub(crate) fn hp_pct(victim: &Npc) -> i32 {
    if victim.max_hp <= 0 {
        0
    } else {
        victim.hp.wrapping_mul(100) / victim.max_hp
    }
}

/// A non-player's `PointChange(POINT_HP, amount)`: nothing while it is dead or stunned,
/// otherwise held to the maximum and broadcast to its targeters.
fn npc_hp_change(victim: &mut Npc, amount: i32, events: &mut Vec<HitEvent>) {
    if victim.dead || victim.stunned {
        return;
    }
    let amount = amount.min(victim.max_hp.wrapping_sub(victim.hp));
    victim.hp = victim.hp.wrapping_add(amount);
    events.push(HitEvent::VictimHp);
}

/// The attacker's `PointChange(kind, amount)`: shown without the amount, to itself alone.
fn attacker_change(points: &mut Points, kind: usize, amount: i32, events: &mut Vec<HitEvent>) {
    let records = points
        .point_change(kind, amount, false, false)
        .unwrap_or_default();
    if !records.is_empty() {
        events.push(HitEvent::Attacker(records));
    }
}

/// `CHARACTER::Damage(attacker, dam, DAMAGE_TYPE_NORMAL)` of a non-player.
fn damage_normal(
    dice: &mut dyn Dice,
    striker: &mut Striker<'_>,
    victim: &mut Npc,
    damage: i32,
    banned: bool,
    events: &mut Vec<HitEvent>,
) -> Struck {
    let current_hp = victim.hp;
    let (damage, flag) = damage_chances(dice, striker.points, victim, damage, events);
    damage_steals(dice, striker.points, victim, damage, current_hp, events);
    let mut damage = damage_bonuses(striker, victim, damage, events);
    if banned {
        return Struck::Refused;
    }
    if victim.char_type == CHAR_TYPE_MONSTER && number(dice, 0, 60) == 0 {
        let line = MONSTER_CHAT_ATTACKED * 3 + number(dice, 1, 3);
        events.push(HitEvent::Chat(line));
    }
    if victim.stunned {
        return Struck::Finished;
    }
    if victim.dead {
        return Struck::AlreadyDead;
    }
    if victim.char_type == CHAR_TYPE_MONSTER
        && victim.ai_flags & AIFLAG_STONESKIN != 0
        && hp_pct(victim) < i32::from(victim.fight.stone_skin_point)
    {
        damage /= 2;
    }
    // `dam = tempDam * damMul + 0.5f`, with `damMul` 1.0, which multiplies exactly.
    let damage = f32_to_i32(i32_to_f32(damage) + 0.5);
    events.push(HitEvent::DamageInfo { damage, flag });
    let damage = if victim.hp.wrapping_sub(damage) <= 0 {
        victim.hp
    } else {
        damage
    };
    npc_hp_change(victim, -damage, events);
    Struck::Landed { damage }
}

/// `Damage`'s critical and penetrating chances: the damage they leave and the flag they make.
fn damage_chances(
    dice: &mut dyn Dice,
    points: &Points,
    victim: &Npc,
    damage: i32,
    events: &mut Vec<HitEvent>,
) -> (i32, u8) {
    let mut damage = damage;
    let mut flag = DAMAGE_NORMAL;
    let critical_pct = points.get_point(point::POINT_CRITICAL_PCT);
    if critical_pct != 0 {
        let critical_pct =
            critical_pct.wrapping_sub(victim.fight.point(point::POINT_RESIST_CRITICAL));
        if number(dice, 1, 100) <= critical_pct {
            flag |= DAMAGE_CRITICAL;
            damage = damage.wrapping_mul(2);
            events.push(HitEvent::Effect(SE_CRITICAL));
        }
    }
    // The `SKILL_RESIST_PENETRATE` poly runs for every hit and gives 0 for a victim with no
    // skill, so a percent of 0 is kept and a negative one still draws.
    let penetrate_pct = points.get_point(point::POINT_PENETRATE_PCT);
    if penetrate_pct != 0 {
        let penetrate_pct =
            penetrate_pct.wrapping_sub(victim.fight.point(point::POINT_RESIST_PENETRATE));
        if number(dice, 1, 100) <= penetrate_pct {
            flag |= DAMAGE_PENETRATE;
            damage = damage.wrapping_add(defence(&victim.fight));
            events.push(HitEvent::Effect(SE_PENETRATE));
        }
    }
    (damage, flag)
}

/// `Damage`'s steals, recoveries and mana burn, which read the damage before its bonuses and
/// the hit points the victim had before the hit.
fn damage_steals(
    dice: &mut dyn Dice,
    points: &mut Points,
    victim: &mut Npc,
    damage: i32,
    current_hp: i32,
    events: &mut Vec<HitEvent>,
) {
    let life = points.get_point(point::POINT_STEAL_HP);
    if life != 0 && number(dice, 1, 10) <= 1 {
        let stolen = damage.min(current_hp.max(0)).wrapping_mul(life) / 100;
        if stolen > 0 && victim.hp >= stolen {
            events.push(HitEvent::Fly(FLY_HP_SMALL));
            attacker_change(points, point::POINT_HP, stolen, events);
            npc_hp_change(victim, -stolen, events);
        }
    }
    let mana = points.get_point(point::POINT_STEAL_SP);
    if mana != 0 && number(dice, 1, 10) <= 1 {
        // A non-player's spell points are read from its hit points.
        let stolen = damage.min(current_hp.max(0)).wrapping_mul(mana) / 100;
        if stolen > 0 && current_hp >= stolen {
            events.push(HitEvent::Fly(FLY_SP_SMALL));
            attacker_change(points, point::POINT_SP, stolen, events);
        }
    }
    let gold = points.get_point(point::POINT_STEAL_GOLD);
    if gold != 0 && number(dice, 1, 100) <= gold {
        let _gold = number(dice, 1, i32::from(victim.fight.level));
    }
    let recovered = if current_hp >= 0 {
        damage.min(current_hp)
    } else {
        damage
    };
    for (slot, kind, fly) in [
        (point::POINT_HIT_HP_RECOVERY, point::POINT_HP, FLY_HP_SMALL),
        (point::POINT_HIT_SP_RECOVERY, point::POINT_SP, FLY_SP_SMALL),
    ] {
        let share = points.get_point(slot);
        if share != 0 && number(dice, 0, 4) > 0 {
            let amount = recovered.wrapping_mul(share) / 100;
            if amount != 0 {
                events.push(HitEvent::Fly(fly));
                attacker_change(points, kind, amount, events);
            }
        }
    }
    // A non-player's `PointChange(POINT_SP, -50)` changes spell points nothing reads.
    let mana_burn = points.get_point(point::POINT_MANA_BURN_PCT);
    if mana_burn != 0 {
        let _burned = number(dice, 1, 100) <= mana_burn;
    }
}

/// `Damage`'s bonuses and defences, then the spell points a draining victim takes.
fn damage_bonuses(
    striker: &mut Striker<'_>,
    victim: &Npc,
    damage: i32,
    events: &mut Vec<HitEvent>,
) -> i32 {
    let points = &mut *striker.points;
    let mut damage = damage;
    let bonus = points.get_point(point::POINT_NORMAL_HIT_DAMAGE_BONUS);
    if bonus != 0 {
        damage = damage.wrapping_mul(100_i32.wrapping_add(bonus)) / 100;
    }
    let defend = victim
        .fight
        .point(point::POINT_NORMAL_HIT_DEFEND_BONUS)
        .min(99);
    let combat = if hp_pct(victim) >= 90 {
        victim.fight.point(point::POINT_RESIST_COMBAT).min(99)
    } else {
        0
    };
    damage = damage.wrapping_mul(100_i32.wrapping_sub(defend).wrapping_sub(combat)) / 100;
    let mall_defence = victim.fight.point(point::POINT_MALL_DEFBONUS);
    if mall_defence > 0 {
        damage = damage.wrapping_sub((damage.wrapping_mul(mall_defence) / 100).min(200));
    }
    if points.get_point(point::POINT_MALL_ATTBONUS) > 0 {
        let mall_attack = points.limit_point(point::POINT_MALL_ATTBONUS);
        damage = damage.wrapping_add((damage.wrapping_mul(mall_attack) / 100).min(300));
    }
    let map_empire = map_empire(striker.map);
    if striker.empire != 0 && map_empire != 0 && striker.empire != map_empire {
        damage = damage.wrapping_mul(9) / 10;
    }
    let drain = i32::try_from(victim.fight.drain_sp).unwrap_or(i32::MAX);
    if drain != 0 {
        let drained = drain.min(points.sp());
        attacker_change(points, point::POINT_SP, -drained, events);
    }
    damage
}

/// `GetPoint(POINT_DEF_GRADE) * (100 + GetPoint(POINT_DEF_BONUS)) / 100`.
fn defence(fight: &MobFight) -> i32 {
    fight
        .point(point::POINT_DEF_GRADE)
        .wrapping_mul(100_i32.wrapping_add(fight.point(point::POINT_DEF_BONUS)))
        .wrapping_div(100)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::path::Path;

    use gamedata::item_kind::{ITEM_ARMOR, WEAPON_BOW};
    use gamedata::mob_proto::{MobProtos, RACE_FLAG};
    use gamedata::records::MobTableRecord;

    use super::*;
    use crate::character::legacy_source::{legacy, names, span, strip_comments};
    use crate::character::PointsRow;

    /// A dice that hands out the draws a test scripted and fails on one more.
    struct Script(VecDeque<u32>);

    impl Script {
        fn new(draws: &[u32]) -> Self {
            Self(draws.iter().copied().collect())
        }

        fn left(&self) -> usize {
            self.0.len()
        }
    }

    impl Dice for Script {
        fn random31(&mut self) -> u32 {
            self.0.pop_front().expect("a draw the test scripted")
        }
    }

    /// The parity scenarios' Alpha: a level-154 shaman with ST 17, HT 18, DX 19 and IQ 20 on
    /// map 1, at 100 hit points.
    fn alpha() -> Points {
        Points::load(&PointsRow {
            race: 3,
            level: 154,
            exp: 0,
            conqueror_level: 0,
            st: 17,
            ht: 18,
            dx: 19,
            iq: 20,
            sungma: [0; 4],
            hp: 100,
            sp: 100,
            stamina: 0,
            inven_point: 0,
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
        })
    }

    /// Vnum 101's proto, as the owner's `mob_proto.txt` gives it.
    fn dog_proto() -> MobTableRecord {
        MobTableRecord {
            vnum: 101,
            level: 1,
            str: 3,
            dex: 6,
            con: 5,
            int_: 2,
            def: 4,
            damage_range: [20, 24],
            max_hp: 126,
            regen_cycle: 6,
            regen_percent: 7,
            attack_range: 175,
            ..MobTableRecord::default()
        }
    }

    fn dog() -> Npc {
        let proto = dog_proto();
        Npc {
            vid: 7,
            vnum: 101,
            race: 101,
            hp: 126,
            max_hp: 126,
            fight: MobFight::of(&proto),
            ..Npc::default()
        }
    }

    fn weapon(item_type: i32, sub_type: i32, values: [i32; 6]) -> ItemProto {
        let mut proto = ItemProto::for_category_rule(1, item_type, sub_type);
        proto.values = values;
        proto
    }

    fn strike(dice: &mut Script, points: &mut Points, victim: &mut Npc) -> Strike {
        let mut striker = Striker {
            points,
            weapon: None,
            empire: 1,
            map: 1,
        };
        strike_npc(dice, &mut striker, victim, false)
    }

    #[test]
    fn the_dog_is_the_owners_vnum_101() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let protos = MobProtos::load(&root).expect("the owner's protos");
        let real = protos.get(101).expect("vnum 101");
        let ours = dog_proto();
        assert_eq!(
            (real.level, real.str, real.dex, real.con, real.int_, real.def),
            (ours.level, ours.str, ours.dex, ours.con, ours.int_, ours.def)
        );
        assert_eq!(real.damage_range, ours.damage_range);
        assert_eq!(
            (real.max_hp, real.attack_range),
            (ours.max_hp, ours.attack_range)
        );
        assert_eq!((real.regen_cycle, real.regen_percent), (6, 7));
        assert_eq!((real.battle_type, real.mob_type, real.drain_sp), (0, 0, 0));
        assert_eq!((real.enchants, real.resists), (ours.enchants, ours.resists));
        let fight = MobFight::of(real);
        // 2 * 1 + 2 * 3, and 1 + 5 + 4.
        assert_eq!(fight.point(point::POINT_ATT_GRADE), 8);
        assert_eq!(fight.point(point::POINT_DEF_GRADE), 10);
    }

    #[test]
    fn alpha_punches_the_dog_for_322_with_one_draw() {
        let points = alpha();
        assert_eq!(points.get_point(point::POINT_ATT_GRADE), 344);
        let mut dice = Script::new(&[12_345]);
        assert_eq!(calc_melee_damage(&mut dice, &points, None, &dog()), 322);
        assert_eq!(dice.left(), 0);
    }

    #[test]
    fn a_lethal_punch_shows_the_whole_damage_and_takes_what_is_left() {
        let mut points = alpha();
        let mut victim = dog();
        // The punch's 0-to-0 roll, then the monster chat's 1 in 61, which says nothing.
        let mut dice = Script::new(&[0, 1]);
        let hit = strike(&mut dice, &mut points, &mut victim);
        assert_eq!(dice.left(), 0);
        assert_eq!(
            hit.events,
            [
                HitEvent::DamageInfo {
                    damage: 322,
                    flag: DAMAGE_NORMAL
                },
                HitEvent::VictimHp
            ]
        );
        assert_eq!(hit.struck, Struck::Landed { damage: 126 });
        assert_eq!(victim.hp, 0);
    }

    #[test]
    fn a_light_hit_rolls_one_to_five_and_a_heavier_one_does_not() {
        let points = alpha();
        let mut victim = dog();
        victim.fight.add(point::POINT_DEF_GRADE, 330);
        // 332 - 340 is below 0, so 0, then the low roll: 4 % 5 + 1.
        let mut dice = Script::new(&[0, 4]);
        assert_eq!(calc_melee_damage(&mut dice, &points, None, &victim), 5);
        assert_eq!(dice.left(), 0);
        // 332 - 329 = 3 is not below 3.
        victim.fight.add(point::POINT_DEF_GRADE, -11);
        let mut dice = Script::new(&[0]);
        assert_eq!(calc_melee_damage(&mut dice, &points, None, &victim), 3);
        assert_eq!(dice.left(), 0);
        // 332 - 330 = 2 is.
        victim.fight.add(point::POINT_DEF_GRADE, 1);
        let mut dice = Script::new(&[0, 0]);
        assert_eq!(calc_melee_damage(&mut dice, &points, None, &victim), 1);
        assert_eq!(dice.left(), 0);
    }

    #[test]
    fn a_weapon_it_does_not_swing_deals_nothing_and_draws_nothing() {
        let points = alpha();
        for weapon in [
            weapon(ITEM_ARMOR, WEAPON_SWORD, [0, 0, 0, 10, 20, 0]),
            weapon(ITEM_WEAPON, WEAPON_BOW, [0, 0, 0, 10, 20, 0]),
            weapon(ITEM_WEAPON, 6, [0, 0, 0, 10, 20, 0]),
            weapon(ITEM_WEAPON, 8, [0, 0, 0, 10, 20, 0]),
        ] {
            let mut dice = Script::new(&[]);
            assert_eq!(
                calc_melee_damage(&mut dice, &points, Some(&weapon), &dog()),
                0
            );
        }
        let mut striker_points = alpha();
        let bow = weapon(ITEM_WEAPON, WEAPON_BOW, [0; 6]);
        let mut striker = Striker {
            points: &mut striker_points,
            weapon: Some(&bow),
            empire: 1,
            map: 1,
        };
        let mut victim = dog();
        let hit = strike_npc(&mut Script::new(&[]), &mut striker, &mut victim, false);
        assert_eq!(hit.struck, Struck::Whiffed);
        assert!(hit.events.is_empty());
        assert_eq!(victim.hp, 126);
    }

    #[test]
    fn a_weapon_rolls_its_values_and_adds_its_refine_twice() {
        let points = alpha();
        for sub_type in [
            WEAPON_SWORD,
            WEAPON_DAGGER,
            WEAPON_TWO_HANDED,
            WEAPON_BELL,
            WEAPON_FAN,
            WEAPON_MOUNT_SPEAR,
        ] {
            // A roll of 10 + 7 % 11 = 17 doubled; 344 + 34 - 308 = 70, times 274/300 less the
            // evasion is 47, plus 308 and twice the refine of 3 is 361, less 10.
            let sword = weapon(ITEM_WEAPON, sub_type, [0, 0, 0, 10, 20, 3]);
            let mut dice = Script::new(&[7]);
            assert_eq!(
                calc_melee_damage(&mut dice, &points, Some(&sword), &dog()),
                351,
                "{sub_type}"
            );
        }
    }

    #[test]
    fn the_victim_resists_the_weapons_kind() {
        let mut fight = MobFight::default();
        for (sub_type, slot) in [
            (WEAPON_SWORD, point::POINT_RESIST_SWORD),
            (WEAPON_TWO_HANDED, point::POINT_RESIST_TWOHAND),
            (WEAPON_DAGGER, point::POINT_RESIST_DAGGER),
            (WEAPON_BELL, point::POINT_RESIST_BELL),
            (WEAPON_FAN, point::POINT_RESIST_FAN),
            (WEAPON_BOW, point::POINT_RESIST_BOW),
        ] {
            fight.add(slot, 30);
            let held = weapon(ITEM_WEAPON, sub_type, [0; 6]);
            assert_eq!(weapon_resist(101, Some(&held), &fight), 70, "{sub_type}");
            fight.add(slot, -30);
        }
        fight.add(point::POINT_RESIST_SWORD, 30);
        let spear = weapon(ITEM_WEAPON, WEAPON_MOUNT_SPEAR, [0; 6]);
        assert_eq!(weapon_resist(101, Some(&spear), &fight), 101);
        assert_eq!(weapon_resist(101, None, &fight), 101);
        // A resist over 100 heals nothing: the damage goes below 0 and `Damage` takes none.
        fight.add(point::POINT_RESIST_SWORD, 100);
        let sword = weapon(ITEM_WEAPON, WEAPON_SWORD, [0; 6]);
        assert_eq!(weapon_resist(100, Some(&sword), &fight), -30);
    }

    #[test]
    fn the_attack_keeps_cs_order_and_wraps() {
        let attack = |attack_grade, percent| {
            melee_attack(&MeleeAttack {
                level: 10,
                attack_grade,
                rolled: 20,
                rating: 1.0,
                refine: 0,
                party_bonus: 0,
                percent,
            })
        };
        assert_eq!(attack(40, 0), 60);
        assert_eq!(attack(40, 100), 120);
        // 30_000_020 * 200 wraps in the 32-bit `int` before the division.
        assert_eq!(attack(30_000_000, 100), 17_050_367);
        let rated = melee_attack(&MeleeAttack {
            level: 10,
            attack_grade: 40,
            rolled: 20,
            rating: 0.5,
            refine: 3,
            party_bonus: 5,
            percent: 10,
        });
        // (40 + 20 - 20) * 0.5 = 20, + 20 + 6 + 5 = 51, * 110 / 100.
        assert_eq!(rated, 56);
    }

    /// `CalcAttBonus` runs on the attack before the defence comes off (`G/battle.cpp:580-595`):
    /// 10 % more against a monster makes Alpha's 332 into 365, and less the dog's 10 that is
    /// 355. The bonus on the 322 the defence leaves would make 354.
    #[test]
    fn the_attack_bonus_comes_before_the_defence() {
        let mut points = alpha();
        points.set_point(point::POINT_ATTBONUS_MONSTER, 10);
        let mut dice = Script::new(&[0]);
        assert_eq!(calc_melee_damage(&mut dice, &points, None, &dog()), 355);
        assert_eq!(dice.left(), 0);
    }

    #[test]
    fn the_race_bonuses_add_in_turn_before_the_monster_bonus() {
        let mut points = alpha();
        let mut victim = dog();
        assert_eq!(att_bonus_on_npc(1000, &points, &victim), 1000);
        victim.race_flag = 1 | (1 << 16);
        points.set_point(point::POINT_ATTBONUS_ANIMAL, 10);
        points.set_point(point::POINT_ENCHANT_DARK, 10);
        // 1000 + 100, then 1100 + 110.
        assert_eq!(att_bonus_on_npc(1000, &points, &victim), 1210);
        victim.race_flag = (1 << 17) | (1 << 18);
        assert_eq!(att_bonus_on_npc(1000, &points, &victim), 1000);
        for (bit, slot) in RACE_BONUSES {
            let mut points = alpha();
            points.set_point(slot, 50);
            victim.race_flag = bit;
            assert_eq!(att_bonus_on_npc(200, &points, &victim), 300, "{slot}");
            victim.race_flag = !bit;
            assert!(att_bonus_on_npc(200, &points, &victim) < 300, "{slot}");
        }
    }

    #[test]
    fn the_monster_bonus_runs_through_float_and_double() {
        let mut points = alpha();
        let victim = dog();
        points.set_point(point::POINT_ATTBONUS_MONSTER, 10);
        assert_eq!(att_bonus_on_npc(1000, &points, &victim), 1100);
        points.set_point(point::POINT_ATTBONUS_MONSTER_PCT, 50);
        // 1000 * 10 * 1.5 / 100.0 = 150.
        assert_eq!(att_bonus_on_npc(1000, &points, &victim), 1150);
        points.set_point(point::POINT_ATTBONUS_MONSTER_PCT, -150);
        // The sum is truncated, not the share: 999 + 999 * 10 * -0.5 / 100 = 949.05.
        assert_eq!(att_bonus_on_npc(999, &points, &victim), 949);
        assert_eq!(point_pct(0).to_bits(), 1.0_f32.to_bits());
        assert_eq!(point_pct(13).to_bits(), 1.13_f32.to_bits());
        assert_eq!(point_pct(-100).to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn a_stone_takes_the_metin_bonus_and_a_boss_the_boss_bonus() {
        let mut points = alpha();
        points.set_point(point::POINT_ATTBONUS_METIN, 20);
        points.set_point(point::POINT_ATTBONUS_BOSS, 40);
        let mut victim = dog();
        victim.fight.rank = MOB_RANK_BOSS - 1;
        assert_eq!(att_bonus_on_npc(1000, &points, &victim), 1000);
        victim.fight.rank = MOB_RANK_BOSS;
        assert_eq!(att_bonus_on_npc(1000, &points, &victim), 1400);
        victim.fight.rank = MOB_RANK_BOSS + 1;
        assert_eq!(att_bonus_on_npc(1000, &points, &victim), 1400);
        victim.char_type = CHAR_TYPE_STONE;
        assert_eq!(att_bonus_on_npc(1000, &points, &victim), 1200);
    }

    #[test]
    fn the_victim_resists_the_attackers_job() {
        let mut victim = dog();
        for (job, slot) in JOB_RESISTS.into_iter().enumerate() {
            victim.fight.add(slot, 25);
            for race in 0..8_u8 {
                let points = Points::load(&PointsRow {
                    race,
                    level: 154,
                    exp: 0,
                    conqueror_level: 0,
                    st: 17,
                    ht: 18,
                    dx: 19,
                    iq: 20,
                    sungma: [0; 4],
                    hp: 100,
                    sp: 100,
                    stamina: 0,
                    inven_point: 0,
                    map_index: 1,
                    part_base: 0,
                    hair_part: 0,
                    sash_part: 0,
                });
                let expected = if usize::from(race % 4) == job {
                    750
                } else {
                    1000
                };
                assert_eq!(att_bonus_on_npc(1000, &points, &victim), expected);
            }
            victim.fight.add(slot, -25);
        }
    }

    #[test]
    fn the_strength_will_halves_the_damage() {
        let mut points = Points::load(&PointsRow {
            race: 3,
            level: 154,
            exp: 0,
            conqueror_level: 1,
            st: 17,
            ht: 18,
            dx: 19,
            iq: 20,
            sungma: [10, 0, 0, 0],
            hp: 100,
            sp: 100,
            stamina: 0,
            inven_point: 0,
            map_index: 373,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
        });
        // Map 373 asks for 15 strength; 10 halves the 322.
        let mut dice = Script::new(&[0]);
        assert_eq!(calc_melee_damage(&mut dice, &points, None, &dog()), 161);
        points.set_point(point::POINT_SUNGMA_STR, 15);
        let mut dice = Script::new(&[0]);
        assert_eq!(calc_melee_damage(&mut dice, &points, None, &dog()), 322);
    }

    #[test]
    fn every_draw_comes_in_legacys_order() {
        let mut points = alpha();
        for (slot, value) in [
            (point::POINT_POISON_PCT, 1),
            (point::POINT_STUN_PCT, 1),
            (point::POINT_SLOW_PCT, 1),
            (point::POINT_CRITICAL_PCT, 1),
            (point::POINT_PENETRATE_PCT, 1),
            (point::POINT_STEAL_HP, 10),
            (point::POINT_STEAL_SP, 10),
            (point::POINT_STEAL_GOLD, 1),
            (point::POINT_HIT_HP_RECOVERY, 10),
            (point::POINT_HIT_SP_RECOVERY, 10),
            (point::POINT_MANA_BURN_PCT, 1),
        ] {
            points.set_point(slot, value);
        }
        let mut victim = dog();
        // The roll, poison, stun, slow, crit, penetrate, the HP and SP steals, the gold and its
        // amount, the HP and SP recoveries, the mana burn, then the chat and its line.
        let draws = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 1];
        let mut dice = Script::new(&draws);
        let hit = strike(&mut dice, &mut points, &mut victim);
        assert_eq!(dice.left(), 0);
        assert_eq!(hit.events.len(), 14);
        assert_eq!(hit.events[0], HitEvent::Effect(SE_CRITICAL));
        assert_eq!(hit.events[1], HitEvent::Effect(SE_PENETRATE));
        assert_eq!(hit.events[2], HitEvent::Fly(FLY_HP_SMALL));
        assert!(matches!(&hit.events[3], HitEvent::Attacker(r) if r[0].kind == 5));
        assert_eq!(hit.events[4], HitEvent::VictimHp);
        assert_eq!(hit.events[5], HitEvent::Fly(FLY_SP_SMALL));
        assert!(matches!(&hit.events[6], HitEvent::Attacker(r) if r[0].kind == 7));
        assert_eq!(hit.events[7], HitEvent::Fly(FLY_HP_SMALL));
        assert!(matches!(&hit.events[8], HitEvent::Attacker(r) if r[0].kind == 5));
        assert_eq!(hit.events[9], HitEvent::Fly(FLY_SP_SMALL));
        assert!(matches!(&hit.events[10], HitEvent::Attacker(r) if r[0].kind == 7));
        assert_eq!(hit.events[11], HitEvent::Chat(11));
        // 322 doubled, plus the defence of 10; the steal took 12 of 126 first.
        assert_eq!(
            hit.events[12],
            HitEvent::DamageInfo {
                damage: 654,
                flag: DAMAGE_NORMAL | DAMAGE_CRITICAL | DAMAGE_PENETRATE
            }
        );
        assert_eq!(hit.events[13], HitEvent::VictimHp);
        assert_eq!(hit.struck, Struck::Landed { damage: 114 });
        assert_eq!(victim.hp, 0);
        // 100, then 12 stolen and 12 recovered of each.
        assert_eq!((points.hp(), points.sp()), (124, 124));
    }

    #[test]
    fn a_missed_chance_draws_and_shows_nothing() {
        let mut points = alpha();
        for slot in [
            point::POINT_CRITICAL_PCT,
            point::POINT_PENETRATE_PCT,
            point::POINT_STEAL_GOLD,
            point::POINT_MANA_BURN_PCT,
        ] {
            points.set_point(slot, 1);
        }
        for slot in [
            point::POINT_STEAL_HP,
            point::POINT_STEAL_SP,
            point::POINT_HIT_HP_RECOVERY,
            point::POINT_HIT_SP_RECOVERY,
        ] {
            points.set_point(slot, 10);
        }
        let mut victim = dog();
        // Every chance draws its highest number, and the chat its 1.
        let draws = [0, 99, 99, 9, 9, 99, 0, 0, 99, 1];
        let mut dice = Script::new(&draws);
        let hit = strike(&mut dice, &mut points, &mut victim);
        assert_eq!(dice.left(), 0);
        assert_eq!(
            hit.events,
            [
                HitEvent::DamageInfo {
                    damage: 322,
                    flag: DAMAGE_NORMAL
                },
                HitEvent::VictimHp
            ]
        );
    }

    #[test]
    fn a_chance_of_0_draws_nothing_and_a_resisted_one_still_draws() {
        let mut points = alpha();
        let mut victim = dog();
        points.set_point(point::POINT_CRITICAL_PCT, 5);
        victim.fight.add(point::POINT_RESIST_CRITICAL, 5);
        points.set_point(point::POINT_PENETRATE_PCT, 5);
        victim.fight.add(point::POINT_RESIST_PENETRATE, 10);
        // Both draw 1, and neither 1 is at or below 0 or -5.
        let mut dice = Script::new(&[0, 0, 0, 1]);
        let hit = strike(&mut dice, &mut points, &mut victim);
        assert_eq!(dice.left(), 0);
        assert_eq!(hit.events.len(), 2);
    }

    #[test]
    fn the_poison_chance_is_its_low_byte_and_the_immune_will_halves_it() {
        let mut points = alpha();
        points.set_point(point::POINT_POISON_PCT, 256 + 3);
        // The draw happens; nothing is applied yet.
        let mut dice = Script::new(&[0, 50]);
        let damage = calc_melee_damage(&mut dice, &points, None, &dog());
        normal_attack_affect(&mut dice, &points);
        assert_eq!((damage, dice.left()), (322, 0));
        // The gate reads the whole point, so a chance whose low byte is 0 still draws.
        points.set_point(point::POINT_POISON_PCT, 256);
        let mut dice = Script::new(&[0]);
        normal_attack_affect(&mut dice, &points);
        assert_eq!(dice.left(), 0);
        points.set_point(point::POINT_POISON_PCT, 0);
        points.set_point(point::POINT_STUN_PCT, -1);
        points.set_point(point::POINT_SLOW_PCT, 1);
        let mut dice = Script::new(&[0, 0]);
        normal_attack_affect(&mut dice, &points);
        assert_eq!(dice.left(), 0);
    }

    #[test]
    fn the_defend_bonus_and_the_combat_resist_at_90_percent_and_up() {
        let mut points = alpha();
        let mut victim = dog();
        victim.fight.add(point::POINT_NORMAL_HIT_DEFEND_BONUS, 10);
        victim.fight.add(point::POINT_RESIST_COMBAT, 20);
        victim.hp = 113;
        victim.max_hp = 126;
        // 113 of 126 is 89 %: only the defend bonus, 322 * 90 / 100.
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 289,
                flag: 1
            }
        );
        victim.hp = 114;
        // 90 %: 322 * 70 / 100.
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 225,
                flag: 1
            }
        );
        victim.fight.add(point::POINT_NORMAL_HIT_DEFEND_BONUS, 200);
        victim.hp = 126;
        // Each is held to 99: 322 * (100 - 99 - 20) / 100 is -61, and -61 + 0.5 truncates to
        // -60. The hit points take a negative damage as a gain, held to the maximum.
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: -60,
                flag: 1
            }
        );
        assert_eq!(hit.struck, Struck::Landed { damage: -60 });
        assert_eq!(victim.hp, 126);
        // At 120 of 126 the gain is held to the 6 missing; below 90 % the resist is not taken.
        victim.hp = 120;
        let _hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(victim.hp, 126);
        victim.hp = 100;
        let _hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(victim.hp, 97);
    }

    #[test]
    fn the_normal_hit_and_mall_bonuses() {
        let mut points = alpha();
        let mut victim = dog();
        victim.max_hp = 100_000;
        victim.hp = 100_000;
        points.set_point(point::POINT_NORMAL_HIT_DAMAGE_BONUS, 50);
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 483,
                flag: 1
            }
        );
        points.set_point(point::POINT_NORMAL_HIT_DAMAGE_BONUS, 0);
        // The item-shop attack is held to 20 % and the extra to 300.
        points.set_point(point::POINT_MALL_ATTBONUS, 50);
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 386,
                flag: 1
            }
        );
        points.set_point(point::POINT_MALL_ATTBONUS, 0);
        victim.fight.add(point::POINT_MALL_DEFBONUS, 10);
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 290,
                flag: 1
            }
        );
        victim.fight.add(point::POINT_MALL_DEFBONUS, 90);
        // 322 off would be all of it; at most 200 comes off.
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 122,
                flag: 1
            }
        );
    }

    #[test]
    fn a_foreign_empires_map_takes_a_tenth() {
        let mut points = alpha();
        let mut victim = dog();
        for (empire, map, damage) in [(1, 21, 289), (2, 21, 322), (1, 1, 322), (0, 21, 322)] {
            victim.hp = 1000;
            victim.max_hp = 1000;
            let mut striker = Striker {
                points: &mut points,
                weapon: None,
                empire,
                map,
            };
            let hit = strike_npc(&mut Script::new(&[0, 1]), &mut striker, &mut victim, false);
            assert_eq!(hit.events[0], HitEvent::DamageInfo { damage, flag: 1 });
        }
        let mut striker = Striker {
            points: &mut points,
            weapon: None,
            empire: 1,
            map: 500,
        };
        victim.hp = 1000;
        let hit = strike_npc(&mut Script::new(&[0, 1]), &mut striker, &mut victim, false);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 322,
                flag: 1
            }
        );
    }

    #[test]
    fn a_draining_monster_takes_spell_points() {
        let mut points = alpha();
        let mut victim = dog();
        victim.fight.drain_sp = 30;
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert!(matches!(&hit.events[0], HitEvent::Attacker(r) if r[0].kind == 7));
        assert_eq!(points.sp(), 70);
        victim.hp = 126;
        victim.fight.drain_sp = 100;
        let _hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(points.sp(), 0);
    }

    #[test]
    fn a_stone_skinned_monster_takes_half_below_its_point() {
        let mut points = alpha();
        let mut victim = dog();
        victim.max_hp = 1000;
        victim.ai_flags = AIFLAG_STONESKIN;
        victim.fight.stone_skin_point = 50;
        victim.hp = 500;
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 322,
                flag: 1
            }
        );
        victim.hp = 499;
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 161,
                flag: 1
            }
        );
        victim.hp = 499;
        victim.ai_flags = 0;
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 322,
                flag: 1
            }
        );
        victim.hp = 499;
        victim.ai_flags = AIFLAG_STONESKIN;
        victim.char_type = 1;
        let hit = strike(&mut Script::new(&[0]), &mut points, &mut victim);
        assert_eq!(
            hit.events[0],
            HitEvent::DamageInfo {
                damage: 322,
                flag: 1
            }
        );
    }

    #[test]
    fn a_monster_speaks_one_hit_in_61() {
        let mut points = alpha();
        for (draws, line) in [([0, 0, 0], 10), ([0, 0, 2], 12)] {
            let mut victim = dog();
            let mut dice = Script::new(&draws);
            let hit = strike(&mut dice, &mut points, &mut victim);
            assert_eq!(dice.left(), 0);
            assert_eq!(hit.events[0], HitEvent::Chat(line));
        }
    }

    #[test]
    fn a_banned_victim_draws_the_hit_and_takes_nothing() {
        let mut points = alpha();
        let mut victim = dog();
        let mut striker = Striker {
            points: &mut points,
            weapon: None,
            empire: 1,
            map: 1,
        };
        let mut dice = Script::new(&[0]);
        let hit = strike_npc(&mut dice, &mut striker, &mut victim, true);
        assert_eq!(dice.left(), 0);
        assert_eq!(hit.struck, Struck::Refused);
        assert!(hit.events.is_empty());
        assert_eq!(victim.hp, 126);
    }

    #[test]
    fn a_stunned_victim_is_finished_and_a_dead_one_is_left() {
        let mut points = alpha();
        let mut victim = dog();
        victim.hp = 0;
        victim.stunned = true;
        // The chat still draws first.
        let mut dice = Script::new(&[0, 0, 1]);
        let hit = strike(&mut dice, &mut points, &mut victim);
        assert_eq!(dice.left(), 0);
        assert_eq!(hit.events, [HitEvent::Chat(11)]);
        assert_eq!(hit.struck, Struck::Finished);
        victim.stunned = false;
        victim.dead = true;
        let hit = strike(&mut Script::new(&[0, 1]), &mut points, &mut victim);
        assert_eq!(hit.struck, Struck::AlreadyDead);
        assert!(hit.events.is_empty());
    }

    #[test]
    fn the_reach_follows_a_melee_monsters_range() {
        let mut victim = dog();
        assert_eq!(melee_reach(&victim), 350);
        victim.fight.attack_range = 850;
        assert_eq!(melee_reach(&victim), 977);
        victim.fight.attack_range = 305;
        assert_eq!(melee_reach(&victim), 350);
        victim.fight.attack_range = 306;
        assert_eq!(melee_reach(&victim), 351);
        for battle_type in 1..8 {
            victim.fight.battle_type = battle_type;
            assert_eq!(melee_reach(&victim), 300);
        }
        for vnum in HYDRAS {
            victim.vnum = vnum;
            assert_eq!(melee_reach(&victim), 900);
        }
        victim.vnum = 3963;
        assert_eq!(melee_reach(&victim), 300);
        victim.fight.battle_type = BATTLE_TYPE_MELEE;
        victim.fight.attack_range = 100;
        for (vnum, reach) in [
            (3400, 350),
            (3401, 460),
            (3605, 460),
            (3606, 350),
            (3949, 350),
            (3950, 460),
            (3959, 460),
            (3960, 950),
        ] {
            victim.vnum = vnum;
            assert_eq!(melee_reach(&victim), reach, "{vnum}");
        }
        victim.fight.attack_range = u16::MAX;
        victim.vnum = 3401;
        assert_eq!(mob_attack_range(&victim), 299);
    }

    /// `POINT_*` name to slot, from `common::point_slot`, whose own tests pin it to `char.h`.
    fn point_slots() -> BTreeMap<String, usize> {
        let text = std::fs::read_to_string("../common/src/point_slot.rs").expect("point_slot.rs");
        text.lines()
            .filter_map(|line| line.trim().strip_prefix("pub const "))
            .filter_map(|rest| {
                let (name, value) = rest.split_once(": usize = ")?;
                let value = value.strip_suffix(';')?.parse().ok()?;
                Some((name.to_owned(), value))
            })
            .collect()
    }

    #[test]
    fn the_race_bonuses_and_job_resists_are_legacys() {
        let text = strip_comments(&legacy("game/battle.cpp"));
        let body = span(&text, "int CalcAttBonus(", "}");
        let at = |from: usize, needle: &str| {
            from + body[from..]
                .iter()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle}"))
        };
        let npc = at(0, "if (pkVictim->IsNPC())");
        let pc = at(npc, "else if (pkVictim->IsPC())");
        let job = at(pc, "if (pkAttacker->IsPC() == true)");
        let element = at(job, "if (pkAttacker->IsNPC() && pkVictim->IsPC())");
        let slots = point_slots();
        let mut pairs = Vec::new();
        let mut flag = None;
        for line in &body[npc..pc] {
            if let Some(race) = names(line, "RACE_FLAG_").first() {
                flag = Some(*race);
                continue;
            }
            if let (Some(race), Some(slot)) = (flag.take(), names(line, "POINT_").first()) {
                let bit = RACE_FLAG
                    .iter()
                    .position(|name| format!("RACE_FLAG_{name}") == race)
                    .unwrap_or_else(|| panic!("{race}"));
                pairs.push((1_u32 << bit, slots[*slot]));
            }
        }
        assert_eq!(pairs, RACE_BONUSES);
        let resists: Vec<usize> = body[job..element]
            .iter()
            .filter(|line| line.contains("pkVictim->GetPoint(POINT_RESIST_"))
            .flat_map(|line| names(line, "POINT_RESIST_"))
            .map(|name| slots[name])
            .collect();
        assert_eq!(resists, JOB_RESISTS);
    }
}
