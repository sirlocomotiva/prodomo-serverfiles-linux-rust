//! A melee monster's hit on a player.
//!
//! This is `battle_hit` (`G/battle.cpp:760-817`), the `CalcMeleeDamage` it calls (`:489-666`)
//! with the arms of `CalcAttBonus` a non-player attacker and a player victim run (`:299-319`,
//! `:371-393`, `:417-441`), the draws of `NormalAttackAffect` (`:734-758`), and the
//! `DAMAGE_TYPE_NORMAL` path of `CHARACTER::Damage` (`G/char_battle.cpp:1744-2436`) down to the
//! damage shown. Every random draw is taken from the `Dice` in legacy's order; what the hit
//! shows comes back as effect bytes in the order legacy sends them, and how it ended tells the
//! caller what to run next. The caller delivers the records and owns the hit points: the down
//! verdict and the clamp to the hit points live in `damage_pc`, which also runs the stun, the
//! killer and the target broadcast.

//! `strike_npc_by_npc` is the same `battle_hit` for a guard swinging at a monster: the
//! NPC-arm `CalcMeleeDamage`, the affect draws, the chances and the bonuses, then the
//! guard arm, which stuns with no reward and takes no hit points.
//!
//! What the Rewrite leaves out, because nothing it hosts can reach it:
//! * the victim holds no affect yet (except a skill's 263 slow on `MOV_SPEED` only), so
//!   `AFF_JEONGWIHON`, `AFF_TERROR`, `AFF_HOSIN` and `AFF_MANASHIELD` never scale the damage,
//!   and the flags gate the three draws open;
//! * a monster's proto leaves the steal, recovery, mana-burn, `MALL_ATTBONUS` and marriage
//!   points at 0, so those arms draw nothing and multiply by one;
//! * the `SKILL_RESIST_PENETRATE` poly is 0 for a victim with no skill;
//! * reflect, the monarch and `BlueDragon_Damage` wait for later work;
//! * `attMul` and `damMul` are 1.0, `cannot_dead` is false, and the user damage rate is 100;
//! * the functor's per-victim `k`-poly draws (`PENETRATE`, `IGNORE_TARGET_RATING`) and all
//!   arms past the CRUSH slide are dead for hosted rows, so the functor's `point2` is
//!   never read.
//!
//! What it draws and returns but the caller applies: the poison, stun and slow of
//! `NormalAttackAffect` land on `PcStrike::affect` (V19i, 229v).

use common::cfloat::{f32_to_i32, i32_to_f32};
use common::point_slot as point;
use gamedata::mob_proto::{AIFLAG_DEATHBLOW, CHAR_TYPE_MONSTER};

use super::melee::{
    AttackAffect, AFF_POISON_WORD, AFF_SLOW_WORD, AFF_STUN_WORD, DAMAGE_CRITICAL, DAMAGE_NORMAL,
    DAMAGE_PENETRATE, SE_CRITICAL, SE_PENETRATE,
};
use super::{attack_rating, AttackRatingInput};
use crate::character::{number, race_to_job, Dice, Points};
use crate::npc::Npc;

/// The monster that swings, and whether `SetBerserk(true)` holds.
#[derive(Debug, Clone, Copy)]
pub struct MobStriker<'a> {
    /// The monster: `GetMobDamageMin/Max` and its points read its `fight`.
    pub npc: &'a Npc,
    /// Whether berserk holds, which doubles `GetMobDamageMultiply` (`G/char.cpp:2605-2612`).
    pub berserk: bool,
}

/// The player it swings at, as `Damage` reads it.
#[derive(Debug, Clone, Copy)]
pub struct PcVictim<'a> {
    /// Its points.
    pub points: &'a Points,
    /// `IsAffectFlag`'s two words, which gate the poison, stun and slow draws.
    pub affect_flags: [u32; 2],
    /// `IsStun() || IsDead()`: the down check comes before the death blow.
    pub down: bool,
    /// `m_pointsInstant.dwImmuneFlag`, which refuses the stun and the slow.
    pub immune_flag: u32,
}

/// What the hit showed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcStrike {
    /// `SE_CRITICAL`, `SE_PENETRATE`, in the order legacy sends them.
    pub effects: Vec<u8>,
    /// How the hit ended.
    pub end: PcHitEnd,
    /// What `NormalAttackAffect` landed: the poison, the stun and the slow.
    pub affect: AttackAffect,
    /// The pre-bonus damage `Damage` reflects a share of (`:1981-1983`):
    /// `CalcMeleeDamage` through `attMul`, before block. `None` on the
    /// paths that never reflect (shot, skill).
    pub reflected_base: Option<i32>,
}

/// `GetPoisonDamageRate` of a player (`G/char_resist.cpp:16-29`): 50 per mille, less
/// `POINT_POISON_REDUCE`, floored at 0.
#[must_use]
pub fn pc_poison_rate(points: &Points) -> i32 {
    50_i32
        .wrapping_sub(points.get_point(point::POINT_POISON_REDUCE))
        .max(0)
}

/// How the hit ended, which decides what the caller runs next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcHitEnd {
    /// `POINT_BLOCK` held: `GC_DAMAGE_INFO{victim, DAMAGE_BLOCK, 0}` alone.
    Blocked,
    /// The player was stunned or dead: `Dead(mob)` or nothing; no damage info.
    Down,
    /// `SendDamagePacket(mob, damage, flag)`, then the hit points.
    Landed {
        /// The damage shown.
        damage: i32,
        /// `DAMAGE_NORMAL`, with `DAMAGE_CRITICAL` and `DAMAGE_PENETRATE` when they hit.
        flag: u8,
    },
}

/// `(int)(GetMobAttackRange() * 1.15f)`: a monster's melee reach (`G/battle.cpp:188`), with
/// no floor and no hydra term, which only a player attacker gets.
#[must_use]
pub fn mob_reach(npc: &Npc) -> i32 {
    f32_to_i32(i32_to_f32(i32::from(super::melee::mob_attack_range(npc))) * 1.15)
}

/// `GetHPPct`: the share of the maximum left, 0 when there is no maximum
/// (`G/char.cpp:8439-8444`).
#[must_use]
pub fn pc_hp_pct(points: &Points) -> i32 {
    if points.max_hp() <= 0 {
        0
    } else {
        points.hp().wrapping_mul(100).wrapping_div(points.max_hp())
    }
}

/// `IMMUNE_REFLECT` (`G/length.h:721`): the attacker's immune bit that thirds a
/// reflected hit.
pub const IMMUNE_REFLECT: u32 = 1 << 6;

/// The reflect arm of `Damage` (`G/char_battle.cpp:1980-1988`): the victim's
/// `POINT_REFLECT_MELEE` share of the pre-bonus `damage`, thirded when the attacker
/// is immune to reflect (`int(reflect / 3.0f + 0.5f)`: the float divide, the
/// round-half-up, the truncating cast). 0 when the victim holds no reflect point.
/// No hosted mob names `REFLECT` in its `IMMUNE_FLAG`, so the third never fires on
/// a hosted row; it is live for fixtures.
#[must_use]
pub fn reflect_damage(reflect: i32, immune: bool, damage: i32) -> i32 {
    if reflect == 0 {
        return 0;
    }
    let reflected = damage.wrapping_mul(reflect) / 100;
    if immune {
        f32_to_i32(i32_to_f32(reflected) / 3.0 + 0.5)
    } else {
        reflected
    }
}

/// The monster a guard swings at, as `Damage` reads it. The hit takes no hit points —
/// the guard arm preempts the change — so the borrow is shared.
#[derive(Debug, Clone, Copy)]
pub struct NpcVictim<'a> {
    /// The victim.
    pub npc: &'a Npc,
}

/// `battle_hit` of a guard on a monster, from `CalcMeleeDamage` to the guard arm: the
/// NPC-arm `CalcMeleeDamage` (NPC attacker, NPC victim), `NormalAttackAffect`'s draws
/// (a guard's `POINT_POISON_PCT`, `POINT_STUN_PCT` and `POINT_SLOW_PCT` are 0, so
/// nothing lands and the outcomes are dropped — V19i keeps the monster-victim side),
/// the `+0.5` of `attMul` 1.0, the critical and penetrate chances, and the bonuses.
/// Then the guard arm (`SET_BIT(NO_REWARD); Stun(); return true`,
/// `G/char_battle.cpp:2218-2223`): it preempts everything after the bonuses — the
/// BANPK refusal, the chat, the stun and dead checks, the stoneskin, the death blow,
/// the damage packet and the hit-point change — so the hit always ends
/// `StunnedByGuard` and takes no hit points. The caller sets the no-reward flag and
/// runs `Stun`; Battle drops a stunned, dead or banned victim before swinging, so the
/// preempted checks are unreachable as well as unrun.
pub fn strike_npc_by_npc(
    dice: &mut dyn Dice,
    striker: MobStriker<'_>,
    victim: NpcVictim<'_>,
) -> super::melee::Strike {
    use super::melee::{Strike, Struck};
    let mut events = Vec::new();
    let damage = guard_melee_damage(dice, striker, victim.npc);
    guard_attack_affect(dice, striker.npc);
    // `iDam = attMul * (float)iDam + 0.5f`, with `attMul` 1.0, which multiplies exactly.
    let damage = f32_to_i32(i32_to_f32(damage) + 0.5);
    let (damage, flag) = npc_chances(dice, striker.npc, victim.npc, damage, &mut events);
    let damage = npc_bonuses_guard(striker.npc, victim.npc, damage);
    let _ = (damage, flag);
    Strike {
        events,
        struck: Struck::StunnedByGuard,
        affect: super::melee::AttackAffect::default(),
        // A guard's swing stuns with no hit-point change and no steal arm.
        stolen_gold: 0,
    }
}

/// `battle_hit` of a melee monster on a player, from `CalcMeleeDamage` through `Damage`'s
/// flag: the damage roll, the `+0.5` of `attMul` 1.0, the block, critical and penetrate
/// chances, the bonuses, the down check, the death blow, and the `+0.5` of `damMul` 1.0.
/// `mob_rate` is `GetMobDamageRate`, read by the caller.
pub fn strike_pc(
    dice: &mut dyn Dice,
    striker: MobStriker<'_>,
    victim: PcVictim<'_>,
    mob_rate: i32,
) -> PcStrike {
    let mut effects = Vec::new();
    let damage = mob_melee_damage(dice, striker, victim.points, mob_rate);
    let affect = mob_attack_affect(dice, striker.npc, &victim);
    // `iDam = attMul * (float)iDam + 0.5f`, with `attMul` 1.0, which multiplies exactly.
    let damage = f32_to_i32(i32_to_f32(damage) + 0.5);
    let reflected_base = Some(damage);
    if pc_blocks(dice, victim.points) {
        return PcStrike {
            effects,
            end: PcHitEnd::Blocked,
            affect,
            reflected_base,
        };
    }
    let (damage, flag) = pc_chances(dice, striker.npc, victim.points, damage, &mut effects);
    let damage = pc_bonuses(striker.npc, victim.points, damage);
    if victim.down {
        return PcStrike {
            effects,
            end: PcHitEnd::Down,
            affect,
            reflected_base,
        };
    }
    let damage = death_blow(dice, striker.npc, victim.points, damage);
    // `dam = (float)dam * damMul + 0.5f`, with `damMul` 1.0, which multiplies exactly.
    let damage = f32_to_i32(i32_to_f32(damage) + 0.5);
    PcStrike {
        effects,
        end: PcHitEnd::Landed { damage, flag },
        affect,
        reflected_base,
    }
}
/// `FuncSplashDamage`'s melee arm on a player (`G/char_skill.cpp:1485-1572`) and `Damage`'s
/// `DAMAGE_TYPE_MELEE` path (`G/char_battle.cpp:1858-1946`, `:2145-2156`), down to the damage
/// shown: the skill's pre-mitigation `atk` minus the defence grade (unclamped, so a negative
/// flows through, `:1572` — skipped when the functor's PENETRATE roll won), the halved
/// critical and penetrating chances (no marriage, a zero `RESIST_PENETRATE` poly), the skill
/// bonuses, the down check, the death blow, the `+0.5` of `damMul` 1.0, and the clamp to 0
/// (229s's choice for a negative: legacy would heal, and the Rewrite never ports a
/// heal-by-damage). No block, no dodge, no reflect, no steal draws: those are the NORMAL
/// path's, which a skill never takes. The dead/stunned predicate is read by the caller.
#[must_use]
pub fn skill_damage_pc(
    dice: &mut dyn Dice,
    mob: &Npc,
    victim: &PcVictim<'_>,
    attack: i32,
    ignore_defence: bool,
) -> PcStrike {
    let mut effects = Vec::new();
    let mut damage = if ignore_defence {
        attack
    } else {
        attack.wrapping_sub(victim.points.get_point(point::POINT_DEF_GRADE))
    };
    let mut flag = DAMAGE_NORMAL;
    let critical_pct = mob.fight.point(point::POINT_CRITICAL_PCT);
    if critical_pct != 0 {
        let critical_pct = if critical_pct >= 10 {
            5 + (critical_pct - 10) / 4
        } else {
            critical_pct / 2
        }
        .wrapping_sub(victim.points.get_point(point::POINT_RESIST_CRITICAL));
        if number(dice, 1, 100) <= critical_pct {
            flag |= DAMAGE_CRITICAL;
            damage = damage.wrapping_mul(2);
            effects.push(SE_CRITICAL);
        }
    }
    let penetrate_pct = mob.fight.point(point::POINT_PENETRATE_PCT);
    if penetrate_pct != 0 {
        let penetrate_pct = if penetrate_pct >= 10 {
            5 + (penetrate_pct - 10) / 4
        } else {
            penetrate_pct / 2
        }
        .wrapping_sub(victim.points.get_point(point::POINT_RESIST_PENETRATE));
        if number(dice, 1, 100) <= penetrate_pct {
            flag |= DAMAGE_PENETRATE;
            let defence = victim
                .points
                .get_point(point::POINT_DEF_GRADE)
                .wrapping_mul(100_i32.wrapping_add(victim.points.get_point(point::POINT_DEF_BONUS)))
                .wrapping_div(100);
            damage = damage.wrapping_add(defence);
            effects.push(SE_PENETRATE);
        }
    }
    let skill = mob.fight.point(point::POINT_SKILL_DAMAGE_BONUS);
    if skill != 0 {
        damage = damage
            .wrapping_mul(100_i32.wrapping_add(skill))
            .wrapping_div(100);
    }
    let defend = victim
        .points
        .get_point(point::POINT_SKILL_DEFEND_BONUS)
        .min(99);
    let combat = if pc_hp_pct(victim.points) >= 90 {
        victim.points.get_point(point::POINT_RESIST_COMBAT).min(99)
    } else {
        0
    };
    damage = damage
        .wrapping_mul(100_i32.wrapping_sub(defend).wrapping_sub(combat))
        .wrapping_div(100);
    let mall = victim.points.get_point(point::POINT_MALL_DEFBONUS);
    if mall > 0 {
        damage = damage.wrapping_sub((damage.wrapping_mul(mall).wrapping_div(100)).min(200));
    }
    if victim.down {
        return PcStrike {
            effects,
            end: PcHitEnd::Down,
            affect: AttackAffect::default(),
            reflected_base: None,
        };
    }
    let damage = death_blow(dice, mob, victim.points, damage);
    let damage = f32_to_i32(i32_to_f32(damage) + 0.5);
    PcStrike {
        effects,
        end: PcHitEnd::Landed {
            damage: damage.max(0),
            flag,
        },
        affect: AttackAffect::default(),
        reflected_base: None,
    }
}

/// `CFuncShoot` of a `RANGE` or `MAGIC` monster on a player (`G/char_battle.cpp:3070-3154`):
/// `CalcMeleeDamage`, the same draws, then the bow or magic resist, and the
/// `NORMAL_RANGE` or `MAGIC` path of `Damage`. `magic` picks the arm. A `Down` end on the
/// magic arm takes the blocked record, as legacy's `Dead(pAttacker)` does.
pub fn shoot_pc(
    dice: &mut dyn Dice,
    striker: MobStriker<'_>,
    victim: PcVictim<'_>,
    magic: bool,
    mob_rate: i32,
) -> PcStrike {
    let mut effects = Vec::new();
    let damage = mob_melee_damage(dice, striker, victim.points, mob_rate);
    let affect = mob_attack_affect(dice, striker.npc, &victim);
    let resist = if magic {
        victim.points.get_point(point::POINT_RESIST_MAGIC)
    } else {
        victim.points.get_point(point::POINT_RESIST_BOW)
    };
    let damage = damage
        .wrapping_mul(100_i32.wrapping_sub(resist))
        .wrapping_div(100);
    // `battle_hit`'s `attMul` step (`G/battle.cpp:807-809`) never runs on a shot — no shot
    // calls it — but 229l's `strike_pc` rounds the same `+0.5` on an `attMul` of 1.0, and the
    // RANGE arm keeps the step so the two agree (`:3092-3110` has no such line of its own);
    // the MAGIC arm has no rounding (`:3119-3142`).
    let damage = if magic {
        damage
    } else {
        f32_to_i32(i32_to_f32(damage) + 0.5)
    };
    if magic {
        return magic_damage(dice, striker.npc, &victim, damage, effects, affect);
    }
    if pc_dodges(dice, victim.points) {
        return PcStrike {
            effects,
            end: PcHitEnd::Blocked,
            affect,
            reflected_base: None,
        };
    }
    let (damage, flag) = pc_chances(dice, striker.npc, victim.points, damage, &mut effects);
    let damage = pc_bonuses(striker.npc, victim.points, damage);
    finish_shot(dice, striker.npc, &victim, damage, flag, effects, affect)
}

/// `Damage`'s `DAMAGE_TYPE_MAGIC` path on a player (`G/char_battle.cpp:1773-1776`,
/// `:1858-1946`, `:2145-2156`): the attacker's magic percent, the halved critical and
/// penetrating chances (with no marriage and no `RESIST_PENETRATE` skill), and the skill
/// bonuses. The terror gate draws nothing (no terror), the manashield, reflect, steal,
/// marriage, monarch, `BlueDragon` and empire arms never fire, and `cannot_dead` is false.
/// A `Down` end takes the blocked record, as `Dead(pAttacker)` shows.
fn magic_damage(
    dice: &mut dyn Dice,
    mob: &Npc,
    shot: &PcVictim<'_>,
    damage: i32,
    mut effects: Vec<u8>,
    affect: AttackAffect,
) -> PcStrike {
    let victim = shot.points;
    // `(int)((float)dam * (100 + per + melee) / 100.f + 0.5f)`: the multiply, then the
    // divide, then the `+0.5`, each in `float`, and the cast truncates.
    let percent = mob
        .fight
        .point(point::POINT_MAGIC_ATT_BONUS_PER)
        .wrapping_add(mob.fight.point(point::POINT_MELEE_MAGIC_ATT_BONUS_PER));
    let mut damage = f32_to_i32(i32_to_f32(damage) * i32_to_f32(100 + percent) / 100.0 + 0.5);
    let mut flag = DAMAGE_NORMAL;
    let critical_pct = mob.fight.point(point::POINT_CRITICAL_PCT);
    if critical_pct != 0 {
        let critical_pct = if critical_pct >= 10 {
            5 + (critical_pct - 10) / 4
        } else {
            critical_pct / 2
        }
        .wrapping_sub(victim.get_point(point::POINT_RESIST_CRITICAL));
        if number(dice, 1, 100) <= critical_pct {
            flag |= DAMAGE_CRITICAL;
            damage = damage.wrapping_mul(2);
            effects.push(SE_CRITICAL);
        }
    }
    let penetrate_pct = mob.fight.point(point::POINT_PENETRATE_PCT);
    if penetrate_pct != 0 {
        // The `SKILL_RESIST_PENETRATE` poly is 0 for a victim with no skill, so the gate
        // is the point itself, as in `pc_chances`.
        let penetrate_pct = if penetrate_pct >= 10 {
            5 + (penetrate_pct - 10) / 4
        } else {
            penetrate_pct / 2
        }
        .wrapping_sub(victim.get_point(point::POINT_RESIST_PENETRATE));
        if number(dice, 1, 100) <= penetrate_pct {
            flag |= DAMAGE_PENETRATE;
            let defence = victim
                .get_point(point::POINT_DEF_GRADE)
                .wrapping_mul(100_i32.wrapping_add(victim.get_point(point::POINT_DEF_BONUS)))
                .wrapping_div(100);
            damage = damage.wrapping_add(defence);
            effects.push(SE_PENETRATE);
        }
    }
    let skill = mob.fight.point(point::POINT_SKILL_DAMAGE_BONUS);
    if skill != 0 {
        damage = damage
            .wrapping_mul(100_i32.wrapping_add(skill))
            .wrapping_div(100);
    }
    let defend = victim.get_point(point::POINT_SKILL_DEFEND_BONUS).min(99);
    let combat = if pc_hp_pct(victim) >= 90 {
        victim.get_point(point::POINT_RESIST_COMBAT).min(99)
    } else {
        0
    };
    let mut damage = damage
        .wrapping_mul(100_i32.wrapping_sub(defend).wrapping_sub(combat))
        .wrapping_div(100);
    let mall = victim.get_point(point::POINT_MALL_DEFBONUS);
    if mall > 0 {
        damage = damage.wrapping_sub((damage.wrapping_mul(mall).wrapping_div(100)).min(200));
    }
    let damage = damage.max(0);
    finish_shot(dice, mob, shot, damage, flag, effects, affect)
}

/// `Damage`'s dodge: `POINT_DODGE && number(1, 100) <= DODGE` (`:1953-1967`).
fn pc_dodges(dice: &mut dyn Dice, victim: &Points) -> bool {
    let dodge = victim.get_point(point::POINT_DODGE);
    dodge != 0 && number(dice, 1, 100) <= dodge
}

fn finish_shot(
    dice: &mut dyn Dice,
    mob: &Npc,
    victim: &PcVictim<'_>,
    damage: i32,
    flag: u8,
    effects: Vec<u8>,
    affect: AttackAffect,
) -> PcStrike {
    if victim.down {
        return PcStrike {
            effects,
            end: PcHitEnd::Down,
            affect,
            reflected_base: None,
        };
    }
    let damage = death_blow(dice, mob, victim.points, damage);
    // `dam = (float)dam * damMul + 0.5f`, with `damMul` 1.0, as `strike_pc` rounds it.
    let damage = f32_to_i32(i32_to_f32(damage) + 0.5);
    PcStrike {
        effects,
        end: PcHitEnd::Landed { damage, flag },
        affect,
        reflected_base: None,
    }
}

/// `CalcMeleeDamage` of a monster on a player, through `CalcBattleDamage`'s `<3` floor.
fn mob_melee_damage(
    dice: &mut dyn Dice,
    striker: MobStriker<'_>,
    victim: &Points,
    mob_rate: i32,
) -> i32 {
    let mob = striker.npc;
    let attack = mob_calc_attack(dice, mob, Some(victim), striker.berserk, false, mob_rate);
    let defence = victim
        .get_point(point::POINT_DEF_GRADE)
        .wrapping_mul(100_i32.wrapping_add(victim.get_point(point::POINT_DEF_BONUS)))
        .wrapping_div(100);
    let damage = attack.wrapping_sub(defence).max(0);
    super::melee::battle_damage_floor(dice, damage)
}
/// `CalcMeleeDamage` of a monster up to its damage multiply (`G/battle.cpp:489-600`): the
/// attack-rating (for the skill's per-victim call, with or without the evasion term), the
/// doubled damage roll, the grade, the party and percent bonuses, `CalcAttBonus` for
/// `victim`, and the berserk-doubled damage multiply — everything before `MAX(0, iAtk -
/// iDef)`. `victim` is `Some` for a player (the HUMAN and job shares, then the B27
/// element arm), `None` for the pre-check's self (`CalcMeleeDamage(self, self, true,
/// false)`: the NPC arms of `:321-369` — the race and metin/boss shares — with the B27
/// scope decision, which was recorded for the player victim and provisionally governs
/// the self the same way). A monster draws no weapon roll and keeps no weapon; its
/// stone/berserk (`StoneSkinPoint`, the `ATTBONUS` halves of `Damage`) and its marriage
/// bonuses are 0, so those arms are identities. Berserk doubles the multiply
/// (`G/char.cpp:2605-2612`). `mob_rate` is `GetMobDamageRate` (`:311-318`), read by the
/// caller: it applies to a player victim only, never to the pre-check's self.
#[must_use]
pub fn mob_calc_attack(
    dice: &mut dyn Dice,
    mob: &Npc,
    victim: Option<&Points>,
    berserk: bool,
    ignore_target_rating: bool,
    mob_rate: i32,
) -> i32 {
    let rating = attack_rating(AttackRatingInput {
        attacker_dexterity: mob.fight.point(point::POINT_DX),
        attacker_level: i32::from(mob.fight.level),
        victim_dexterity: victim.map_or(mob.fight.point(point::POINT_DX), |victim| {
            victim.get_point(point::POINT_DX)
        }),
        ignore_target_rating,
    });
    let rolled = number(
        dice,
        i32::from_ne_bytes(mob.fight.damage_range[0].to_ne_bytes()),
        i32::from_ne_bytes(mob.fight.damage_range[1].to_ne_bytes()),
    )
    .wrapping_mul(2);
    let level_twice = i32::from(mob.fight.level).wrapping_mul(2);
    let attack = mob
        .fight
        .point(point::POINT_ATT_GRADE)
        .wrapping_add(rolled)
        .wrapping_sub(level_twice);
    let mut attack = f32_to_i32(i32_to_f32(attack) * rating).wrapping_add(level_twice);
    // No weapon arm: a monster holds no weapon (`pWeapon` is NULL), so the refine roll
    // never draws.
    attack = attack
        .wrapping_add(mob.fight.point(point::POINT_PARTY_ATTACKER_BONUS))
        .wrapping_mul(
            100_i32
                .wrapping_add(mob.fight.point(point::POINT_ATT_BONUS))
                .wrapping_add(mob.fight.point(point::POINT_MELEE_MAGIC_ATT_BONUS_PER)),
        )
        .wrapping_div(100);
    let attack = match victim {
        Some(victim) => mob_att_bonus(attack, mob, victim, mob_rate),
        None => mob_att_bonus_self(attack, mob),
    };
    let multiply = mob.fight.dam_multiply();
    let multiply = if berserk { multiply * 2.0 } else { multiply };
    f32_to_i32(i32_to_f32(attack) * multiply)
}

/// `CalcAttBonus` of a monster on itself (`G/battle.cpp:299-319`, `:321-369`): the
/// marriage attack bonus (0, no marriage ported) and the marriage transfer (0) are
/// identities, and the mob damage rate never applies to a non-player victim (`:311`:
/// `pkAttacker->IsNPC() && pkVictim->IsPC()`); then the NPC-victim `:321-369` arms —
/// the race-flag shares, the `ENCHANT_*` shares, the `ATTBONUS_MONSTER_PCT` arm
/// (`BONUS_PCT` is defined) in `melee.rs`'s rounding, and the metin/boss share — and the
/// B27 element arm on the caster's own resists.
#[must_use]
pub fn mob_att_bonus_self(attack: i32, mob: &Npc) -> i32 {
    const SHARES: [(u32, usize); 11] = [
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
    ];
    const ENCHANTS: [(u32, usize); 6] = [
        (1 << 11, point::POINT_ENCHANT_ELECT),
        (1 << 12, point::POINT_ENCHANT_FIRE),
        (1 << 13, point::POINT_ENCHANT_ICE),
        (1 << 14, point::POINT_ENCHANT_WIND),
        (1 << 15, point::POINT_ENCHANT_EARTH),
        (1 << 16, point::POINT_ENCHANT_DARK),
    ];
    // `GetMarriageBonus(ATTACK_BONUS)`: 0. The mob damage rate never applies to a
    // non-player victim (`:311`), so no rate multiplies here.
    let race = mob.race_flag;
    let mut attack = attack;
    for (bit, slot) in SHARES {
        if race & bit != 0 {
            attack = attack.wrapping_add(attack.wrapping_mul(mob.fight.point(slot)) / 100);
        }
    }
    for (bit, slot) in ENCHANTS {
        if race & bit != 0 {
            attack = attack.wrapping_add(attack.wrapping_mul(mob.fight.point(slot)) / 100);
        }
    }
    attack = super::melee::monster_share(
        attack,
        mob.fight.point(point::POINT_ATTBONUS_MONSTER),
        mob.fight.point(point::POINT_ATTBONUS_MONSTER_PCT),
    );
    let attack = if mob.char_type == gamedata::mob_proto::CHAR_TYPE_STONE {
        attack.wrapping_add(attack.wrapping_mul(mob.fight.point(point::POINT_ATTBONUS_METIN)) / 100)
    } else if mob.fight.rank >= gamedata::mob_proto::MOB_RANK_BOSS {
        attack.wrapping_add(attack.wrapping_mul(mob.fight.point(point::POINT_ATTBONUS_BOSS)) / 100)
    } else {
        attack
    };
    let resists = [
        (
            mob.fight.point(point::POINT_RESIST_ELEC),
            mob.fight.point(point::POINT_RESIST_ELECT_PCT),
        ),
        (
            mob.fight.point(point::POINT_RESIST_FIRE),
            mob.fight.point(point::POINT_RESIST_FIRE_PCT),
        ),
        (
            mob.fight.point(point::POINT_RESIST_ICE),
            mob.fight.point(point::POINT_RESIST_ICE_PCT),
        ),
        (
            mob.fight.point(point::POINT_RESIST_WIND),
            mob.fight.point(point::POINT_RESIST_WIND_PCT),
        ),
        (
            mob.fight.point(point::POINT_RESIST_EARTH),
            mob.fight.point(point::POINT_RESIST_EARTH_PCT),
        ),
        (
            mob.fight.point(point::POINT_RESIST_DARK),
            mob.fight.point(point::POINT_RESIST_DARK_PCT),
        ),
    ];
    element_resist_pairs(attack, race, &resists)
}

/// `CalcAttBonus` for a non-player attacker and a player victim (`G/battle.cpp:299-319`,
/// `:371-393`, `:417-441`): the marriage transfer (0, no marriage ported) is an identity
/// and `mob_rate` is `GetMobDamageRate` (`:311-318`), 100 while no quest set `mob_dam` —
/// `BONUS_PTC` is a typo legacy never defines, so the rate is a plain `int`; then the
/// attacker's `ATTBONUS_HUMAN` and job share (`:371-393`), and the B27 element arm. The
/// attacker's own race cuts nothing for a non-player (`:395-415` is a player attacker
/// only).
fn mob_att_bonus(attack: i32, mob: &Npc, victim: &Points, mob_rate: i32) -> i32 {
    // `GetMarriageBonus(TRANSFER_DAMAGE)`: 0, no marriage ported.
    let attack = attack.wrapping_mul(mob_rate) / 100;
    let share = |attack: i32, slot: usize| attack.wrapping_mul(mob.fight.point(slot)) / 100;
    let attack = attack.wrapping_add(share(attack, point::POINT_ATTBONUS_HUMAN));
    let job_slots = [
        point::POINT_ATTBONUS_WARRIOR,
        point::POINT_ATTBONUS_ASSASSIN,
        point::POINT_ATTBONUS_SURA,
        point::POINT_ATTBONUS_SHAMAN,
    ];
    let attack = attack.wrapping_add(share(
        attack,
        job_slots[usize::from(race_to_job(victim.race()))],
    ));
    element_resist(attack, mob.race_flag, victim)
}

/// The B27 element arm: the `#else` chain's first matching race bit, each with its
/// `RESIST_*` and `RESIST_*_PCT` point, in the live ELEC line's rounding
/// (`G/battle.cpp:424`).
fn element_resist(attack: i32, race_flag: u32, victim: &Points) -> i32 {
    const ARMS: [(u32, usize, usize); 6] = [
        (
            1 << 11,
            point::POINT_RESIST_ELEC,
            point::POINT_RESIST_ELECT_PCT,
        ),
        (
            1 << 12,
            point::POINT_RESIST_FIRE,
            point::POINT_RESIST_FIRE_PCT,
        ),
        (
            1 << 13,
            point::POINT_RESIST_ICE,
            point::POINT_RESIST_ICE_PCT,
        ),
        (
            1 << 14,
            point::POINT_RESIST_WIND,
            point::POINT_RESIST_WIND_PCT,
        ),
        (
            1 << 15,
            point::POINT_RESIST_EARTH,
            point::POINT_RESIST_EARTH_PCT,
        ),
        (
            1 << 16,
            point::POINT_RESIST_DARK,
            point::POINT_RESIST_DARK_PCT,
        ),
    ];
    let pairs: [(i32, i32); 6] =
        ARMS.map(|(_, resist, pct)| (victim.get_point(resist), victim.get_point(pct)));
    element_resist_pairs(attack, race_flag, &pairs)
}

/// The B27 arm over explicit `(resist, pct)` pairs, ELEC first: what [`element_resist`]
/// reads off a player's points, and what the pre-check's self reads off its own fight.
/// `race_flag` is tested bit by bit, 11 to 16.
fn element_resist_pairs(attack: i32, race_flag: u32, pairs: &[(i32, i32); 6]) -> i32 {
    for (arm, (resist, pct)) in pairs.iter().enumerate() {
        let bit = 1_u32 << (u32::try_from(arm).unwrap_or(u32::MAX) + 11);
        if race_flag & bit != 0 {
            let product = attack.wrapping_mul(30).wrapping_mul(*resist);
            // `iAtk -= (product) * GetPointPct(pct, 1) / 10000`: an `int` times a `float`
            // is a `float`, divided by an `int` is a `float`, and the compound assignment
            // truncates the difference, `(int)((float)iAtk - q)`, as `melee.rs` truncates
            // the sibling sum. Subtracting the truncated take is off by one when both
            // are fractional (attack 100, ELEC 7, PCT 0: `q = 2.1`, legacy 97, not 98).
            let percent = super::melee::point_pct(*pct);
            let take = i32_to_f32(product) * percent / 10_000.0;
            return f32_to_i32(i32_to_f32(attack) - take);
        }
    }
    attack
}

/// `NormalAttackAffect` (`G/battle.cpp:734-758`) on the player: the poison, then the stun
/// and the slow. Each draws only when the monster has the point and the player lacks the
/// affect; the stun and slow then need no immunity bit, and the poison needs the level
/// draw. The caller applies what lands, and starts the poison event when the poison does.
fn mob_attack_affect(dice: &mut dyn Dice, mob: &Npc, victim: &PcVictim<'_>) -> AttackAffect {
    let mut landed = AttackAffect::default();
    let poison = mob.fight.point(point::POINT_POISON_PCT);
    if poison != 0 && victim.affect_flags[0] & AFF_POISON_WORD == 0 {
        // `BYTE bPoisonPct`: the low byte; the SungMa halving never fires for a monster.
        let percent = poison.to_le_bytes()[0];
        if number(dice, 1, 100) <= i32::from(percent) {
            // `AttackedByPoison`'s level arm, past the running-event and poisoned-once
            // gates, which the caller owns (`G/char_resist.cpp:178-197`). The commented
            // immunity arm never runs.
            let level = i32::from(mob.fight.level);
            let over = i32::from(victim.points.level()).wrapping_sub(level);
            if over <= 0 {
                landed.poisoned = true;
            } else {
                const ADJUST: [i32; 9] = [100, 90, 80, 70, 50, 30, 10, 5, 0];
                let delta = over.min(8);
                if number(dice, 1, 100) <= ADJUST[usize::try_from(delta).unwrap_or(8)] {
                    landed.poisoned = true;
                }
            }
        }
    }
    // `AttackAffect` (`G/battle.h:35-62`): the immune check is inside the draw's success
    // arm, so a failed roll spends exactly one draw. With `__FIX_BUG_IMMUNE__` the check
    // itself draws nothing (`G/char_resist.cpp:250-283`).
    for (slot, held, immune) in [
        (
            point::POINT_STUN_PCT,
            AFF_STUN_WORD,
            crate::character::IMMUNE_STUN,
        ),
        (
            point::POINT_SLOW_PCT,
            AFF_SLOW_WORD,
            crate::character::IMMUNE_SLOW,
        ),
    ] {
        let chance = mob.fight.point(slot);
        if chance != 0
            && victim.affect_flags[0] & held == 0
            && number(dice, 1, 100) <= chance
            && victim.immune_flag & immune == 0
        {
            if immune == crate::character::IMMUNE_STUN {
                landed.stunned = true;
            } else {
                landed.slowed = true;
            }
        }
    }
    landed
}

/// `Damage`'s block: `POINT_BLOCK && number(1, 100) <= BLOCK`.
fn pc_blocks(dice: &mut dyn Dice, victim: &Points) -> bool {
    let block = victim.get_point(point::POINT_BLOCK);
    block != 0 && number(dice, 1, 100) <= block
}

/// `Damage`'s critical and penetrating chances: the damage they leave and the flag they make.
fn pc_chances(
    dice: &mut dyn Dice,
    mob: &Npc,
    victim: &Points,
    damage: i32,
    effects: &mut Vec<u8>,
) -> (i32, u8) {
    let mut damage = damage;
    let mut flag = DAMAGE_NORMAL;
    let critical_pct = mob.fight.point(point::POINT_CRITICAL_PCT);
    if critical_pct != 0 {
        let critical_pct =
            critical_pct.wrapping_sub(victim.get_point(point::POINT_RESIST_CRITICAL));
        if number(dice, 1, 100) <= critical_pct {
            flag |= DAMAGE_CRITICAL;
            damage = damage.wrapping_mul(2);
            effects.push(SE_CRITICAL);
        }
    }
    // The `SKILL_RESIST_PENETRATE` poly gives 0 for a victim with no skill, so the gate is
    // the point itself, and a negative net percent still draws.
    let penetrate_pct = mob.fight.point(point::POINT_PENETRATE_PCT);
    if penetrate_pct != 0 {
        let penetrate_pct =
            penetrate_pct.wrapping_sub(victim.get_point(point::POINT_RESIST_PENETRATE));
        if number(dice, 1, 100) <= penetrate_pct {
            flag |= DAMAGE_PENETRATE;
            let defence = victim
                .get_point(point::POINT_DEF_GRADE)
                .wrapping_mul(100_i32.wrapping_add(victim.get_point(point::POINT_DEF_BONUS)))
                .wrapping_div(100);
            damage = damage.wrapping_add(defence);
            effects.push(SE_PENETRATE);
        }
    }
    (damage, flag)
}
/// `CalcMeleeDamage` of a guard on a monster, through `CalcBattleDamage`'s `<3` floor
/// (`G/battle.cpp:489-666`, NPC attacker and NPC victim). The attack-rating reads the
/// victim's `POINT_DX` with the target term (`CalcAttackRating(..., false)`, `:552`);
/// the doubled `GetMobDamageMin/Max` roll; the grade, the party and percent bonuses;
/// `CalcAttBonus`'s NPC-victim arms (`:299-369`); the victim's `DEF_GRADE` with the
/// victim's `DEF_BONUS` (`:597-602`); the berserk-doubled damage multiply
/// (`fDamMultiply` is 0.0 on the owner's guard protos, so a guard's swing falls to the
/// `<3` floor, `number(1, 5)`).
fn guard_melee_damage(dice: &mut dyn Dice, striker: MobStriker<'_>, victim: &Npc) -> i32 {
    let mob = striker.npc;
    let rating = attack_rating(AttackRatingInput {
        attacker_dexterity: mob.fight.point(point::POINT_DX),
        attacker_level: i32::from(mob.fight.level),
        victim_dexterity: victim.fight.point(point::POINT_DX),
        ignore_target_rating: false,
    });
    let rolled = number(
        dice,
        i32::from_ne_bytes(mob.fight.damage_range[0].to_ne_bytes()),
        i32::from_ne_bytes(mob.fight.damage_range[1].to_ne_bytes()),
    )
    .wrapping_mul(2);
    let level = i32::from(mob.fight.level);
    let level_twice = level.wrapping_mul(2);
    let attack = mob
        .fight
        .point(point::POINT_ATT_GRADE)
        .wrapping_add(rolled)
        .wrapping_sub(level_twice);
    let mut attack = f32_to_i32(i32_to_f32(attack) * rating).wrapping_add(level_twice);
    attack = attack
        .wrapping_add(mob.fight.point(point::POINT_PARTY_ATTACKER_BONUS))
        .wrapping_mul(
            100_i32
                .wrapping_add(mob.fight.point(point::POINT_ATT_BONUS))
                .wrapping_add(mob.fight.point(point::POINT_MELEE_MAGIC_ATT_BONUS_PER)),
        )
        .wrapping_div(100);
    let attack = npc_att_bonus(attack, mob, victim);
    // `:597-602`: the victim's grade with the victim's bonus (the melee-range line that
    // reads the attacker's is a different function).
    let defence = victim
        .fight
        .point(point::POINT_DEF_GRADE)
        .wrapping_mul(100_i32.wrapping_add(victim.fight.point(point::POINT_DEF_BONUS)))
        .wrapping_div(100);
    let multiply = mob.fight.dam_multiply();
    let multiply = if striker.berserk {
        multiply * 2.0
    } else {
        multiply
    };
    let attack = f32_to_i32(i32_to_f32(attack) * multiply);
    let damage = attack.wrapping_sub(defence).max(0);
    // `__CONQUEROR_LEVEL__` (defined): a guard's will is 0, so the halving never fires.
    super::melee::battle_damage_floor(dice, damage)
}

/// `CalcAttBonus` of a non-player attacker on a non-player victim (`G/battle.cpp:299-369`):
/// the attacker's marriage attack bonus (0), the victim's marriage transfer (0), the
/// victim's race-flag shares, the `ENCHANT_*` shares, the `ATTBONUS_MONSTER_PCT` arm
/// (`BONUS_PCT` is defined) and the metin/boss share. No `PC` arm runs.
fn npc_att_bonus(attack: i32, mob: &Npc, victim: &Npc) -> i32 {
    const SHARES: [(u32, usize); 11] = [
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
    ];
    const ENCHANTS: [(u32, usize); 6] = [
        (1 << 11, point::POINT_ENCHANT_ELECT),
        (1 << 12, point::POINT_ENCHANT_FIRE),
        (1 << 13, point::POINT_ENCHANT_ICE),
        (1 << 14, point::POINT_ENCHANT_WIND),
        (1 << 15, point::POINT_ENCHANT_EARTH),
        (1 << 16, point::POINT_ENCHANT_DARK),
    ];
    // The marriage attack bonus and transfer are 0: no marriage is ported.
    let race = victim.race_flag;
    let mut attack = attack;
    for (bit, slot) in SHARES {
        if race & bit != 0 {
            attack = attack.wrapping_add(attack.wrapping_mul(mob.fight.point(slot)) / 100);
        }
    }
    for (bit, slot) in ENCHANTS {
        if race & bit != 0 {
            attack = attack.wrapping_add(attack.wrapping_mul(mob.fight.point(slot)) / 100);
        }
    }
    attack = super::melee::monster_share(
        attack,
        mob.fight.point(point::POINT_ATTBONUS_MONSTER),
        mob.fight.point(point::POINT_ATTBONUS_MONSTER_PCT),
    );
    if victim.char_type == gamedata::mob_proto::CHAR_TYPE_STONE {
        attack.wrapping_add(attack.wrapping_mul(mob.fight.point(point::POINT_ATTBONUS_METIN)) / 100)
    } else if victim.fight.rank >= gamedata::mob_proto::MOB_RANK_BOSS {
        attack.wrapping_add(attack.wrapping_mul(mob.fight.point(point::POINT_ATTBONUS_BOSS)) / 100)
    } else {
        attack
    }
}

/// `NormalAttackAffect`'s draws for a guard's swing (`G/battle.cpp:734-758`): the poison,
/// then the stun and the slow. Each draws only when the guard has the point (a guard's
/// are 0); the victim is never already affected and `AddAffect` on a monster is not
/// ported (V19i keeps the monster-victim side), so outcomes are dropped.
fn guard_attack_affect(dice: &mut dyn Dice, mob: &Npc) {
    let poison = mob.fight.point(point::POINT_POISON_PCT);
    if poison != 0 {
        // `BYTE bPoisonPct`: the low byte; the SungMa halving never fires for a monster.
        let percent = poison.to_le_bytes()[0];
        let _poisoned = number(dice, 1, 100) <= i32::from(percent);
    }
    for slot in [point::POINT_STUN_PCT, point::POINT_SLOW_PCT] {
        let chance = mob.fight.point(slot);
        if chance != 0 {
            let _affected = number(dice, 1, 100) <= chance;
        }
    }
}

/// `Damage`'s critical and penetrating chances on a monster victim: the damage they
/// leave, the flag they make, and the effect events. The `SKILL_RESIST_PENETRATE` poly
/// gives 0 for a victim with no skill, so the penetrate gate is the point itself; the
/// marriage critical/penetrate bonuses are 0.
fn npc_chances(
    dice: &mut dyn Dice,
    mob: &Npc,
    victim: &Npc,
    damage: i32,
    events: &mut Vec<super::melee::HitEvent>,
) -> (i32, u8) {
    use super::melee::HitEvent;
    let mut damage = damage;
    let mut flag = DAMAGE_NORMAL;
    let critical_pct = mob.fight.point(point::POINT_CRITICAL_PCT);
    if critical_pct != 0 {
        let critical_pct =
            critical_pct.wrapping_sub(victim.fight.point(point::POINT_RESIST_CRITICAL));
        if number(dice, 1, 100) <= critical_pct {
            flag |= DAMAGE_CRITICAL;
            damage = damage.wrapping_mul(2);
            events.push(HitEvent::Effect(SE_CRITICAL));
        }
    }
    let penetrate_pct = mob.fight.point(point::POINT_PENETRATE_PCT);
    if penetrate_pct != 0 {
        let penetrate_pct =
            penetrate_pct.wrapping_sub(victim.fight.point(point::POINT_RESIST_PENETRATE));
        if number(dice, 1, 100) <= penetrate_pct {
            flag |= DAMAGE_PENETRATE;
            let defence = victim
                .fight
                .point(point::POINT_DEF_GRADE)
                .wrapping_mul(100_i32.wrapping_add(victim.fight.point(point::POINT_DEF_BONUS)))
                .wrapping_div(100);
            damage = damage.wrapping_add(defence);
            events.push(HitEvent::Effect(SE_PENETRATE));
        }
    }
    (damage, flag)
}

/// `Damage`'s bonuses on a monster victim up to the guard arm
/// (`G/char_battle.cpp:2123-2218`): the attacker's hit bonus, the victim's defend bonus
/// with its combat resist at 90 % hit points, and the mall defence. The steals,
/// recoveries and mana burn draw nothing — a guard's `POINT_STEAL_*`,
/// `POINT_HIT_*_RECOVERY`, `POINT_MANA_BURN_PCT` and `POINT_MALL_ATTBONUS` are 0 — and
/// the empire tenth and the drain run for a player attacker only.
fn npc_bonuses_guard(mob: &Npc, victim: &Npc, damage: i32) -> i32 {
    let mut damage = damage;
    let bonus = mob.fight.point(point::POINT_NORMAL_HIT_DAMAGE_BONUS);
    if bonus != 0 {
        damage = damage.wrapping_mul(100_i32.wrapping_add(bonus)) / 100;
    }
    let defend = victim
        .fight
        .point(point::POINT_NORMAL_HIT_DEFEND_BONUS)
        .min(99);
    let combat = if super::melee::hp_pct(victim) >= 90 {
        victim.fight.point(point::POINT_RESIST_COMBAT).min(99)
    } else {
        0
    };
    damage = damage.wrapping_mul(100_i32.wrapping_sub(defend).wrapping_sub(combat)) / 100;
    let mall_defence = victim.fight.point(point::POINT_MALL_DEFBONUS);
    if mall_defence > 0 {
        damage = damage.wrapping_sub((damage.wrapping_mul(mall_defence) / 100).min(200));
    }
    damage
}
/// `Damage`'s bonuses on a player victim: the attacker's hit bonus, the victim's defend
/// bonus with its combat resist at 90 % hit points, and the mall defence. Held at 0 (B28).
fn pc_bonuses(mob: &Npc, victim: &Points, damage: i32) -> i32 {
    let mut damage = damage;
    // `ApplyPoint` gives a monster 0: only a worn item's `NORMAL_HIT_DAMAGE_BONUS` could
    // set it, and monsters wear nothing.
    let hit = mob.fight.point(point::POINT_NORMAL_HIT_DAMAGE_BONUS);
    if hit != 0 {
        damage = damage
            .wrapping_mul(100_i32.wrapping_add(hit))
            .wrapping_div(100);
    }
    let defend = victim
        .get_point(point::POINT_NORMAL_HIT_DEFEND_BONUS)
        .min(99);
    let combat = if pc_hp_pct(victim) >= 90 {
        victim.get_point(point::POINT_RESIST_COMBAT).min(99)
    } else {
        0
    };
    damage = damage
        .wrapping_mul(100_i32.wrapping_sub(defend).wrapping_sub(combat))
        .wrapping_div(100);
    let mall = victim.get_point(point::POINT_MALL_DEFBONUS);
    if mall > 0 {
        damage = damage.wrapping_sub((damage.wrapping_mul(mall).wrapping_div(100)).min(200));
    }
    damage.max(0)
}

/// `Damage`'s death blow: a `MONSTER` with `AIFLAG_DEATHBLOW` draws `number(1, 100) <=
/// bDeathBlowPoint`, and then `number(JOB_WARRIOR, JOB_MAX_NUM - 1) == GetJob()` quadruples
/// the damage with no flag bit.
fn death_blow(dice: &mut dyn Dice, mob: &Npc, victim: &Points, damage: i32) -> i32 {
    if mob.char_type == CHAR_TYPE_MONSTER
        && mob.ai_flags & AIFLAG_DEATHBLOW != 0
        && number(dice, 1, 100) <= i32::from(mob.fight.death_blow_point)
        && number(dice, 0, 3) == i32::from(race_to_job(victim.race()))
    {
        damage.wrapping_mul(4)
    } else {
        damage
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use gamedata::records::MobTableRecord;

    use super::*;
    use crate::character::points::PointsRow;
    use crate::npc::MobFight;

    /// A dice that hands out the draws a test scripted and fails on one more.
    struct Script(VecDeque<u32>);

    impl Script {
        fn new(draws: &[u32]) -> Self {
            Self(draws.iter().copied().collect())
        }
    }

    impl Dice for Script {
        fn random31(&mut self) -> u32 {
            self.0.pop_front().expect("a draw the test scripted")
        }
    }

    /// The parity scenarios' Alpha: a level-154 shaman with ST 17, HT 18, DX 19 and IQ 20
    /// on map 1, at 100 hit points.
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
            random_hp: 0,
            random_sp: 0,
            stat_point: 0,
            skill_point: 0,
            sub_skill_point: 0,
            level_step: 0,
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
            playtime: 0,
        })
    }

    /// Vnum 191's proto, as the owner's `mob_proto.txt` gives it: a level-30 MELEE monster
    /// with stun 3, critical 5 and penetrate 20, and a damage multiplier of 2.0.
    fn boar_proto() -> MobTableRecord {
        MobTableRecord {
            vnum: 191,
            level: 30,
            str: 39,
            dex: 20,
            con: 30,
            int_: 10,
            def: 35,
            damage_range: [52, 96],
            attack_range: 175,
            aggressive_sight: 2000,
            enchants: [0, 0, 0, 3, 5, 20],
            dam_multiply: 2.0,
            ..MobTableRecord::default()
        }
    }

    /// Vnum 406's proto: a level-35 MELEE `HUMAN,ATT_WIND` monster with no enchants.
    fn wind_proto() -> MobTableRecord {
        MobTableRecord {
            vnum: 406,
            level: 35,
            str: 35,
            dex: 23,
            con: 42,
            int_: 12,
            def: 41,
            damage_range: [59, 110],
            attack_range: 175,
            aggressive_sight: 2000,
            race_flag: (1 << 3) | (1 << 14),
            dam_multiply: 1.0,
            ..MobTableRecord::default()
        }
    }

    fn monster(proto: &MobTableRecord, race_flag: u32, ai_flags: u32) -> Npc {
        Npc {
            vid: 7,
            vnum: proto.vnum,
            race: u16::try_from(proto.vnum).unwrap_or(0),
            char_type: CHAR_TYPE_MONSTER,
            race_flag,
            ai_flags,
            hp: 1000,
            max_hp: 1000,
            fight: MobFight::of(proto),
            ..Npc::default()
        }
    }

    fn shot(dice: &mut Script, mob: &Npc, points: &Points, down: bool, magic: bool) -> PcStrike {
        shoot_pc(
            dice,
            MobStriker {
                npc: mob,
                berserk: false,
            },
            PcVictim {
                points,
                affect_flags: [0; 2],
                down,
                immune_flag: 0,
            },
            magic,
            100,
        )
    }

    fn hit_with(
        dice: &mut Script,
        mob: &Npc,
        points: &Points,
        down: bool,
        mob_rate: i32,
    ) -> PcStrike {
        strike_pc(
            dice,
            MobStriker {
                npc: mob,
                berserk: false,
            },
            PcVictim {
                points,
                affect_flags: [0; 2],
                down,
                immune_flag: 0,
            },
            mob_rate,
        )
    }

    fn hit(dice: &mut Script, mob: &Npc, points: &Points, down: bool) -> PcStrike {
        strike_pc(
            dice,
            MobStriker {
                npc: mob,
                berserk: false,
            },
            PcVictim {
                points,
                affect_flags: [0; 2],
                down,
                immune_flag: 0,
            },
            100,
        )
    }

    #[test]
    fn a_mob_swing_draws_in_legacys_order() {
        let proto = boar_proto();
        let mob = monster(&proto, 1 << 0, 0);
        let points = alpha();
        // The damage roll, the stun draw, then the critical and penetrate draws: no block
        // draw at `POINT_BLOCK` 0. Each draw's raw value maps through `number`'s width.
        let mut dice = Script::new(&[18, 50, 50, 50]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.effects, Vec::<u8>::new());
        assert!(matches!(strike.end, PcHitEnd::Landed { flag: 1, .. }));
    }

    #[test]
    fn a_mob_swing_takes_cs_roundings_for_406_on_a_level_154_shaman() {
        let proto = wind_proto();
        // No element bit, so the element arm leaves the attack alone.
        let mob = monster(&proto, 1 << 3, 0);
        let points = alpha();
        assert_eq!(points.get_point(point::POINT_DEF_GRADE), 168);
        // `number(59, 110)`: the raw draw maps modulo 52.
        for (draw, damage) in [(0, 25), (51, 92), (25, 58)] {
            let mut dice = Script::new(&[draw]);
            let strike = hit(&mut dice, &mob, &points, false);
            assert!(dice.0.is_empty());
            assert_eq!(
                strike.end,
                PcHitEnd::Landed { damage, flag: 1 },
                "draw {draw}"
            );
        }
    }

    #[test]
    fn berserk_doubles_the_damage_multiplier() {
        let proto = wind_proto();
        let mob = monster(&proto, 1 << 3, 0);
        let points = alpha();
        let mut dice = Script::new(&[0]);
        let calm = strike_pc(
            &mut dice,
            MobStriker {
                npc: &mob,
                berserk: false,
            },
            PcVictim {
                points: &points,
                affect_flags: [0; 2],
                down: false,
                immune_flag: 0,
            },
            100,
        );
        let mut dice = Script::new(&[0]);
        let berserk = strike_pc(
            &mut dice,
            MobStriker {
                npc: &mob,
                berserk: true,
            },
            PcVictim {
                points: &points,
                affect_flags: [0; 2],
                down: false,
                immune_flag: 0,
            },
            100,
        );
        // Roll 59: calm `(int)(193 * 1.0) - 168 = 25`; berserk `(int)(193 * 2.0) - 168`.
        assert_eq!(
            calm.end,
            PcHitEnd::Landed {
                damage: 25,
                flag: 1
            }
        );
        assert_eq!(
            berserk.end,
            PcHitEnd::Landed {
                damage: 193 * 2 - 168,
                flag: 1
            }
        );
    }

    #[test]
    fn a_zero_multiplier_hits_one_to_five() {
        let mut proto = wind_proto();
        proto.dam_multiply = 0.0;
        let mob = monster(&proto, 1 << 3, 0);
        let points = alpha();
        // The damage roll, then the `<3` floor's `number(1, 5)`.
        let mut dice = Script::new(&[0, 3]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.end, PcHitEnd::Landed { damage: 4, flag: 1 });
    }

    #[test]
    fn the_element_resist_takes_the_first_matching_flag() {
        let proto = wind_proto();
        let points = alpha();
        let mut winded = alpha();
        winded.set_point(point::POINT_RESIST_WIND, 10);
        winded.set_point(point::POINT_RESIST_WIND_PCT, 20);
        // Roll 84: 226 before the arm; `q = (226 * 30 * 10) * 1.2 / 10000 = 8`.
        let mut dice = Script::new(&[25]);
        let plain = hit(&mut dice, &monster(&proto, 1 << 3, 0), &winded, false);
        assert_eq!(
            plain.end,
            PcHitEnd::Landed {
                damage: 58,
                flag: 1
            }
        );
        let mut dice = Script::new(&[25]);
        let struck = hit(
            &mut dice,
            &monster(&proto, (1 << 3) | (1 << 14), 0),
            &winded,
            false,
        );
        assert_eq!(
            struck.end,
            PcHitEnd::Landed {
                damage: 49,
                flag: 1
            }
        );
        // An ELEC bit first in the chain wins over WIND.
        let mut elec = alpha();
        elec.set_point(point::POINT_RESIST_ELEC, 10);
        let mut dice = Script::new(&[25]);
        let first = hit(
            &mut dice,
            &monster(&proto, (1 << 11) | (1 << 14), 0),
            &elec,
            false,
        );
        // `q = (226 * 30 * 10) * 1.0 / 10000 = 6`.
        assert_eq!(
            first.end,
            PcHitEnd::Landed {
                damage: 51,
                flag: 1
            }
        );
        let _ = points;
    }

    #[test]
    fn a_negative_damage_is_held_at_zero() {
        let proto = wind_proto();
        let mob = monster(&proto, 1 << 3, 0);
        let mut points = Points::load(&PointsRow {
            race: 3,
            level: 154,
            exp: 0,
            conqueror_level: 0,
            st: 17,
            ht: 18,
            dx: 19,
            iq: 20,
            sungma: [0; 4],
            hp: 1420,
            sp: 100,
            stamina: 0,
            inven_point: 0,
            random_hp: 0,
            random_sp: 0,
            stat_point: 0,
            skill_point: 0,
            sub_skill_point: 0,
            level_step: 0,
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
            playtime: 0,
        });
        points.set_point(point::POINT_NORMAL_HIT_DEFEND_BONUS, 60);
        points.set_point(point::POINT_RESIST_COMBAT, 60);
        assert!(pc_hp_pct(&points) >= 90);
        let mut dice = Script::new(&[51]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.end, PcHitEnd::Landed { damage: 0, flag: 1 });
    }

    #[test]
    fn a_block_ends_the_hit_before_critical_and_penetrate() {
        let proto = boar_proto();
        let mob = monster(&proto, 1 << 0, 0);
        let mut points = alpha();
        points.set_point(point::POINT_BLOCK, 50);
        // The damage roll, the stun draw, then the block draw, which hits: no critical or
        // penetrate draw follows.
        let mut dice = Script::new(&[18, 50, 10]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.end, PcHitEnd::Blocked);
        assert!(strike.effects.is_empty());
    }

    #[test]
    fn a_down_player_takes_no_death_blow_draw() {
        let mut proto = boar_proto();
        proto.death_blow_point = 30;
        let mob = monster(&proto, 1 << 0, AIFLAG_DEATHBLOW);
        let points = alpha();
        // The damage roll, the stun, critical and penetrate draws, then `Down`: the death
        // blow's draws are never taken.
        let mut dice = Script::new(&[18, 50, 50, 50]);
        let strike = hit(&mut dice, &mob, &points, true);
        assert!(dice.0.is_empty());
        assert_eq!(strike.end, PcHitEnd::Down);
    }

    #[test]
    fn the_death_blow_quadruples_only_on_a_job_match_and_sets_no_flag() {
        let mut proto = wind_proto();
        proto.death_blow_point = 30;
        let mob = monster(&proto, 1 << 3, AIFLAG_DEATHBLOW);
        // Alpha is a shaman: race 3 maps to job 3.
        let points = alpha();
        // Roll 59 gives 25; the death blow's `number(1, 100)` hits and `number(0, 3)`
        // rolls 3.
        let mut dice = Script::new(&[0, 10, 3]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 100,
                flag: 1
            }
        );
        // The same, but the job roll misses.
        let mut dice = Script::new(&[0, 10, 1]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 25,
                flag: 1
            }
        );
    }

    #[test]
    fn critical_and_penetrate_set_their_flags_and_effects_in_order() {
        let proto = boar_proto();
        let mob = monster(&proto, 1 << 0, 0);
        let points = alpha();
        // Roll 70: `(int)(201 * 2.0) - 168 = 234`. The stun draw misses; the critical
        // draw (`<= 5`) and the penetrate draw (`<= 20`) hit.
        let mut dice = Script::new(&[18, 50, 2, 10]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.effects, vec![SE_CRITICAL, SE_PENETRATE]);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 234 * 2 + 168,
                flag: DAMAGE_NORMAL | DAMAGE_CRITICAL | DAMAGE_PENETRATE,
            }
        );
    }

    #[test]
    fn only_a_monster_strikes_a_death_blow() {
        let mut proto = wind_proto();
        proto.death_blow_point = 100;
        let points = alpha();
        // A stone with the flag and a full point draws the death blow: the type gate
        // refuses it, so the job draw is never taken.
        let mut stone = monster(&proto, 1 << 3, AIFLAG_DEATHBLOW);
        stone.char_type = gamedata::mob_proto::CHAR_TYPE_STONE;
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &stone, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 25,
                flag: 1
            }
        );
    }

    #[test]
    fn the_poison_draw_hits_on_an_equal_roll() {
        let mut proto = wind_proto();
        // `ENCHANT_POISON` is the third enchant.
        proto.enchants = [0, 0, 1, 0, 0, 0];
        let mob = monster(&proto, 1 << 3, 0);
        let points = alpha();
        // The damage roll, then the poison draw at exactly the percent: a hit. The
        // monster is below Alpha's level, so the adjust draw follows.
        let mut dice = Script::new(&[0, 0, 50]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 25,
                flag: 1
            }
        );
    }

    #[test]
    fn resists_and_the_mall_cap_survive_their_mutants() {
        let proto = boar_proto();
        let mob = monster(&proto, 1 << 0, 0);
        // A resist above the chance turns the hit away: `5 - 6 < 1`.
        let mut points = alpha();
        points.set_point(point::POINT_RESIST_CRITICAL, 6);
        points.set_point(point::POINT_RESIST_PENETRATE, 21);
        let mut dice = Script::new(&[18, 50, 0, 0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.effects, Vec::<u8>::new());
        assert!(matches!(strike.end, PcHitEnd::Landed { flag: 1, .. }));
        // The mall defence takes at most 200: roll 70 on 191 gives 234, and 50 %
        // of it is 117, below the cap.
        let mut proto = wind_proto();
        proto.damage_range = [59, 59];
        let mob = monster(&proto, 1 << 3, 0);
        let mut points = alpha();
        points.set_point(point::POINT_MALL_DEFBONUS, 50);
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 25 - 12,
                flag: 1
            }
        );
        // At full hit points the combat resist joins the defend bonus.
        let mut points = Points::load(&PointsRow {
            race: 3,
            level: 154,
            exp: 0,
            conqueror_level: 0,
            st: 17,
            ht: 18,
            dx: 19,
            iq: 20,
            sungma: [0; 4],
            hp: 1420,
            sp: 100,
            stamina: 0,
            inven_point: 0,
            random_hp: 0,
            random_sp: 0,
            stat_point: 0,
            skill_point: 0,
            sub_skill_point: 0,
            level_step: 0,
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
            playtime: 0,
        });
        points.set_point(point::POINT_RESIST_COMBAT, 10);
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 25 * 90 / 100,
                flag: 1
            }
        );
        // Below 90 % the combat resist stays out.
        let mut points = alpha();
        points.set_point(point::POINT_RESIST_COMBAT, 90);
        assert!(pc_hp_pct(&points) < 90);
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 25,
                flag: 1
            }
        );
        // The mall defence never takes more than 200: roll 200 on 191 gives 574, and
        // half of it would be 287.
        let mut proto = boar_proto();
        proto.damage_range = [200, 200];
        proto.enchants = [0; 6];
        let mob = monster(&proto, 1 << 0, 0);
        let mut points = alpha();
        points.set_point(point::POINT_MALL_DEFBONUS, 50);
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 574 - 200,
                flag: 1
            }
        );
    }

    #[test]
    fn the_floor_mall_gate_and_defend_cap_hit_their_edges() {
        // A level-1 monster with a fixed roll of 1 against 2 defence: the pre-floor
        // damage is exactly 2, so the `<3` floor draws.
        let proto = MobTableRecord {
            vnum: 9000,
            level: 1,
            str: 1,
            dex: 1,
            con: 1,
            int_: 1,
            damage_range: [1, 1],
            attack_range: 175,
            dam_multiply: 1.0,
            ..MobTableRecord::default()
        };
        let mob = monster(&proto, 0, 0);
        let mut points = alpha();
        points.set_point(point::POINT_DEF_GRADE, 2);
        let mut dice = Script::new(&[0, 3]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.end, PcHitEnd::Landed { damage: 4, flag: 1 });
        // A negative mall defence is not a bonus: the `> 0` gate skips it.
        let proto = wind_proto();
        let mob = monster(&proto, 1 << 3, 0);
        let mut points = alpha();
        points.set_point(point::POINT_MALL_DEFBONUS, -50);
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 25,
                flag: 1
            }
        );
        // The defend bonus caps at 99.
        let mut points = alpha();
        points.set_point(point::POINT_NORMAL_HIT_DEFEND_BONUS, 150);
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert_eq!(strike.end, PcHitEnd::Landed { damage: 0, flag: 1 });
        // A pre-floor damage of exactly 3 draws no floor: roll 42 on 406 gives 3.
        let mut proto = wind_proto();
        proto.damage_range = [42, 42];
        let mob = monster(&proto, 1 << 3, 0);
        let points = alpha();
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.end, PcHitEnd::Landed { damage: 3, flag: 1 });
        // The combat resist caps at 99 too.
        let mut points = Points::load(&PointsRow {
            race: 3,
            level: 154,
            exp: 0,
            conqueror_level: 0,
            st: 17,
            ht: 18,
            dx: 19,
            iq: 20,
            sungma: [0; 4],
            hp: 1420,
            sp: 100,
            stamina: 0,
            inven_point: 0,
            random_hp: 0,
            random_sp: 0,
            stat_point: 0,
            skill_point: 0,
            sub_skill_point: 0,
            level_step: 0,
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
            playtime: 0,
        });
        points.set_point(point::POINT_RESIST_COMBAT, 60);
        let proto = wind_proto();
        let mob = monster(&proto, 1 << 3, 0);
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 25 * 40 / 100,
                flag: 1
            }
        );
    }

    #[test]
    fn a_held_affect_shuts_its_draw_and_bonuses_add_their_shares() {
        // A stunned player skips the stun draw: 191's script is the damage roll, then
        // the critical and penetrate draws.
        let proto = boar_proto();
        let mob = monster(&proto, 1 << 0, 0);
        let points = alpha();
        let mut dice = Script::new(&[18, 50, 50]);
        let strike = strike_pc(
            &mut dice,
            MobStriker {
                npc: &mob,
                berserk: false,
            },
            PcVictim {
                points: &points,
                affect_flags: [AFF_STUN_WORD, 0],
                down: false,
                immune_flag: 0,
            },
            100,
        );
        assert!(dice.0.is_empty());
        assert!(matches!(strike.end, PcHitEnd::Landed { flag: 1, .. }));
        // The attacker's `ATTBONUS_HUMAN` and job share add up: roll 59 on 406 gives
        // 193 before the bonus, 212 with 10 % human on a shaman victim whose own
        // warrior share stays out.
        let proto = wind_proto();
        let mut mob = monster(&proto, 1 << 3, 0);
        mob.fight.add(point::POINT_ATTBONUS_HUMAN, 10);
        mob.fight.add(point::POINT_ATTBONUS_WARRIOR, 50);
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 193 + 19 - 168,
                flag: 1
            }
        );
        // The party attacker bonus adds flat before the percent.
        let mut mob = monster(&proto, 1 << 3, 0);
        mob.fight.add(point::POINT_PARTY_ATTACKER_BONUS, 20);
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 193 + 20 - 168,
                flag: 1
            }
        );
    }

    #[test]
    fn penetrate_adds_defence_before_the_bonuses_scale_it() {
        // Roll 70 on 191 gives 234; the penetrate hit adds Alpha's 168 before the
        // defend bonus halves the sum, not the bare roll.
        let proto = boar_proto();
        let mob = monster(&proto, 1 << 0, 0);
        let mut points = alpha();
        points.set_point(point::POINT_NORMAL_HIT_DEFEND_BONUS, 50);
        let mut dice = Script::new(&[18, 50, 50, 10]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.effects, vec![SE_PENETRATE]);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: (234 + 168) * 50 / 100,
                flag: DAMAGE_NORMAL | DAMAGE_PENETRATE,
            }
        );
    }

    #[test]
    fn a_monsters_reach_is_its_range_times_1_15_with_no_floor() {
        let proto = wind_proto();
        for (range, reach) in [(100, 115), (175, 201), (50, 57)] {
            let mut proto = proto;
            proto.attack_range = range;
            let mob = monster(&proto, 1 << 3, 0);
            assert_eq!(mob_reach(&mob), reach, "range {range}");
        }
    }

    #[test]
    fn a_range_shot_dodges_draws_and_resists_the_bow() {
        // Roll 59 on 406 gives 25 past Alpha's 168 defence. A 20 % bow resist leaves 20,
        // and the dodge draw comes after the roll with no block draw.
        let proto = wind_proto();
        let mob = monster(&proto, 1 << 3, 0);
        let mut points = alpha();
        points.set_point(point::POINT_RESIST_BOW, 20);
        points.set_point(point::POINT_DODGE, 50);
        let mut dice = Script::new(&[0, 49]);
        let strike = shot(&mut dice, &mob, &points, false, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.end, PcHitEnd::Blocked);
        let mut dice = Script::new(&[0, 50]);
        let strike = shot(&mut dice, &mob, &points, false, false);
        assert!(dice.0.is_empty());
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 25 * 80 / 100,
                flag: 1
            }
        );
    }

    #[test]
    fn a_magic_shot_scales_resists_and_halves_its_chances() {
        // Roll 59 on 406 gives 25 past Alpha's 168 defence. A 10 % magic percent scales
        // to 28 (`+0.5` on 27.5), the 20 % magic resist leaves 22, and the 10 % critical
        // chance halves to 5: roll 5 lands doubled, roll 6 lands plain.
        let proto = wind_proto();
        let mut mob = monster(&proto, 1 << 3, 0);
        mob.fight.add(point::POINT_MAGIC_ATT_BONUS_PER, 10);
        mob.fight.add(point::POINT_CRITICAL_PCT, 10);
        let mut points = alpha();
        points.set_point(point::POINT_RESIST_MAGIC, 20);
        let mut dice = Script::new(&[0, 4]);
        let strike = shot(&mut dice, &mob, &points, false, true);
        assert!(dice.0.is_empty());
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 22 * 2,
                flag: DAMAGE_NORMAL | DAMAGE_CRITICAL,
            }
        );
        assert_eq!(strike.effects, [SE_CRITICAL]);
        let mut dice = Script::new(&[0, 5]);
        let strike = shot(&mut dice, &mob, &points, false, true);
        assert!(dice.0.is_empty());
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 22,
                flag: 1
            }
        );
    }

    #[test]
    fn a_magic_shot_takes_the_skill_bonuses_not_the_hit_ones() {
        // Roll 59 on 406 gives 25 past Alpha's 168 defence. The skill damage bonus
        // scales it, the skill defend bonus halves it, and the hit bonuses sit untouched.
        let proto = wind_proto();
        let mut mob = monster(&proto, 1 << 3, 0);
        mob.fight.add(point::POINT_SKILL_DAMAGE_BONUS, 10);
        mob.fight.add(point::POINT_NORMAL_HIT_DAMAGE_BONUS, 100);
        let mut points = alpha();
        points.set_point(point::POINT_SKILL_DEFEND_BONUS, 50);
        points.set_point(point::POINT_NORMAL_HIT_DEFEND_BONUS, 99);
        let mut dice = Script::new(&[0]);
        let strike = shot(&mut dice, &mob, &points, false, true);
        assert!(dice.0.is_empty());
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: (25 * 110 / 100) * 50 / 100,
                flag: 1
            }
        );
    }

    #[test]
    fn a_monsters_hit_bonus_scales_before_the_defend_bonus() {
        // Roll 59 on 406 gives 193, less Alpha's 168 defence; a 10 % hit bonus scales
        // the 25 first, then the defend bonus halves the scaled sum.
        let proto = wind_proto();
        let mut mob = monster(&proto, 1 << 3, 0);
        mob.fight.add(point::POINT_NORMAL_HIT_DAMAGE_BONUS, 10);
        let mut points = alpha();
        points.set_point(point::POINT_NORMAL_HIT_DEFEND_BONUS, 50);
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: (193 - 168) * 110 / 100 * 50 / 100,
                flag: 1
            }
        );
    }

    #[test]
    fn the_hit_lands_the_poison_stun_and_slow() {
        // Roll 59 on 406 gives 193; `number` maps each raw draw modulo 100. All three
        // draw 1, and the victim is level 154 past a level-35 monster, so the poison
        // needs its level draw too (`ADJUST[min(119, 8)] = 0`, which no draw passes —
        // this proto's poison misses on level, and the stun and slow land).
        let proto = wind_proto();
        let mut mob = monster(&proto, 1 << 3, 0);
        mob.fight.add(point::POINT_POISON_PCT, 100);
        mob.fight.add(point::POINT_STUN_PCT, 100);
        mob.fight.add(point::POINT_SLOW_PCT, 100);
        let points = alpha();
        let mut dice = Script::new(&[0, 0, 0, 0, 0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert!(!strike.affect.poisoned);
        assert!(strike.affect.stunned);
        assert!(strike.affect.slowed);
    }

    #[test]
    fn the_poison_lands_on_an_equal_level() {
        // The level arm draws only when the attacker is lower; on an equal level the
        // poison lands past its one draw.
        let proto = wind_proto();
        let mut mob = monster(&proto, 1 << 3, 0);
        mob.fight.add(point::POINT_POISON_PCT, 100);
        mob.fight.level = 154;
        let points = alpha();
        let mut dice = Script::new(&[0, 0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert!(strike.affect.poisoned);
    }

    #[test]
    fn a_held_affect_skips_its_draw() {
        // The stun is held, so the stun draws nothing; the slow's raw 99 maps to 100
        // and lands at 100. The dice prove the stun spent nothing.
        let proto = wind_proto();
        let mut mob = monster(&proto, 1 << 3, 0);
        mob.fight.add(point::POINT_STUN_PCT, 100);
        mob.fight.add(point::POINT_SLOW_PCT, 100);
        let points = alpha();
        let mut dice = Script::new(&[0, 99]);
        let strike = strike_pc(
            &mut dice,
            MobStriker {
                npc: &mob,
                berserk: false,
            },
            PcVictim {
                points: &points,
                affect_flags: [AFF_STUN_WORD, 0],
                down: false,
                immune_flag: 0,
            },
            100,
        );
        assert!(dice.0.is_empty());
        assert!(!strike.affect.stunned);
        assert!(strike.affect.slowed);
    }

    #[test]
    fn immunity_refuses_past_a_spent_draw() {
        // The stun draws 1 and passes, but `IMMUNE_STUN` refuses it; the dice prove the
        // draw was spent anyway. `IMMUNE_SLOW` refuses the slow the same way.
        let proto = wind_proto();
        let mut mob = monster(&proto, 1 << 3, 0);
        mob.fight.add(point::POINT_STUN_PCT, 100);
        mob.fight.add(point::POINT_SLOW_PCT, 100);
        let points = alpha();
        let mut dice = Script::new(&[0, 0, 0]);
        let strike = strike_pc(
            &mut dice,
            MobStriker {
                npc: &mob,
                berserk: false,
            },
            PcVictim {
                points: &points,
                affect_flags: [0; 2],
                down: false,
                immune_flag: crate::character::IMMUNE_STUN | crate::character::IMMUNE_SLOW,
            },
            100,
        );
        assert!(dice.0.is_empty());
        assert!(!strike.affect.stunned);
        assert!(!strike.affect.slowed);
    }

    #[test]
    fn a_failed_draw_spends_exactly_one_draw() {
        // The stun draws 100 and fails at 100 − 1; one draw spent, no immunity read.
        let proto = wind_proto();
        let mut mob = monster(&proto, 1 << 3, 0);
        mob.fight.add(point::POINT_STUN_PCT, 99);
        let points = alpha();
        let mut dice = Script::new(&[0, 99]);
        let strike = strike_pc(
            &mut dice,
            MobStriker {
                npc: &mob,
                berserk: false,
            },
            PcVictim {
                points: &points,
                affect_flags: [0; 2],
                down: false,
                immune_flag: u32::MAX,
            },
            100,
        );
        assert!(dice.0.is_empty());
        assert!(!strike.affect.stunned);
    }

    #[test]
    fn the_poison_rate_floors_at_zero() {
        let mut points = alpha();
        assert_eq!(pc_poison_rate(&points), 50);
        points.set_point(point::POINT_POISON_REDUCE, 60);
        assert_eq!(pc_poison_rate(&points), 0);
    }

    /// A stun held spends no draw: the dice holds what the swing before it scripted.
    #[test]
    fn a_held_stun_spends_no_draw() {
        let proto = boar_proto();
        let mut mob = monster(&proto, 1 << 0, 0);
        mob.fight.add(point::POINT_STUN_PCT, 100);
        let points = alpha();
        let mut dice = Script::new(&[18, 50, 50, 42]);
        let strike = strike_pc(
            &mut dice,
            MobStriker {
                npc: &mob,
                berserk: false,
            },
            PcVictim {
                points: &points,
                affect_flags: [AFF_STUN_WORD, 0],
                down: false,
                immune_flag: 0,
            },
            100,
        );
        assert!(!strike.affect.stunned);
        assert_eq!(dice.0.len(), 1, "the stun draw was never taken");
    }

    /// An immune stun spends the draw and lands nothing.
    #[test]
    fn an_immune_stun_spends_the_draw_and_lands_nothing() {
        let proto = boar_proto();
        let mut mob = monster(&proto, 1 << 0, 0);
        mob.fight.add(point::POINT_STUN_PCT, 100);
        let points = alpha();
        let mut dice = Script::new(&[18, 50, 50, 0]);
        let strike = strike_pc(
            &mut dice,
            MobStriker {
                npc: &mob,
                berserk: false,
            },
            PcVictim {
                points: &points,
                affect_flags: [0; 2],
                down: false,
                immune_flag: crate::character::IMMUNE_STUN,
            },
            100,
        );
        assert!(dice.0.is_empty());
        assert!(!strike.affect.stunned);
    }

    /// The poison's level arm: no draw at equal level, the table at 1 over, and dead
    /// at 8 over.
    #[test]
    fn the_poison_level_arm_draws_by_the_delta() {
        let poisoned = |mob_level: u8, draws: &[u32]| {
            let proto = wind_proto();
            let mut mob = monster(&proto, 1 << 0, 0);
            mob.fight.level = mob_level;
            mob.fight.add(point::POINT_POISON_PCT, 100);
            let points = alpha();
            let mut dice = Script::new(draws);
            let strike = strike_pc(
                &mut dice,
                MobStriker {
                    npc: &mob,
                    berserk: false,
                },
                PcVictim {
                    points: &points,
                    affect_flags: [0; 2],
                    down: false,
                    immune_flag: 0,
                },
                100,
            );
            (strike.affect.poisoned, dice.0.is_empty())
        };
        // The swing's dice come first, then the poison draw, then the level draw.
        assert_eq!(poisoned(154, &[0, 0]), (true, true));
        // One over: the level draw at 90 passes on a raw 0 (1), fails on a raw 90 (91).
        assert_eq!(poisoned(153, &[0, 0, 0]), (true, true));
        assert_eq!(poisoned(153, &[0, 0, 90]), (false, true));
        // Eight over: the table is 0, so even 0 fails.
        assert_eq!(poisoned(146, &[0, 0, 0]), (false, true));
    }

    /// Vnum 11000's proto, as the owner's `mob_proto.txt` gives it: a level-100 NOMOVE
    /// guard with 0 damage and a damage multiplier of 0.
    fn guard_proto() -> MobTableRecord {
        MobTableRecord {
            vnum: 11000,
            level: 100,
            def: 109,
            damage_range: [0, 0],
            attack_range: 175,
            aggressive_sight: 2000,
            dam_multiply: 0.0,
            ..MobTableRecord::default()
        }
    }

    fn guard(proto: &MobTableRecord) -> Npc {
        Npc {
            vid: 9,
            vnum: proto.vnum,
            race: u16::try_from(proto.vnum).unwrap_or(0),
            char_type: gamedata::mob_proto::CHAR_TYPE_NPC,
            hp: 6938,
            max_hp: 6938,
            fight: MobFight::of(proto),
            ..Npc::default()
        }
    }

    /// A guard's swing ends stunned: the NPC-arm `CalcMeleeDamage` rolls 0 to 0, the
    /// multiply is 0, the floor's `number(1, 5)` fires (raw 3 gives 4), and the guard
    /// arm preempts the rest — no effects, no hit-point change.
    #[test]
    fn a_guard_swing_stuns_and_takes_nothing() {
        let proto = guard_proto();
        let guard = guard(&proto);
        let boar = boar_proto();
        let victim = monster(&boar, 1 << 0, 0);
        let mut dice = Script::new(&[0, 3]);
        let strike = strike_npc_by_npc(
            &mut dice,
            MobStriker {
                npc: &guard,
                berserk: false,
            },
            NpcVictim { npc: &victim },
        );
        assert!(dice.0.is_empty());
        assert!(strike.events.is_empty());
        assert_eq!(strike.struck, crate::character::Struck::StunnedByGuard);
        assert_eq!(victim.hp, 1000);
    }

    /// A critical guard still stuns: the `POINT_CRITICAL_PCT` draw fires (raw 0 gives 1),
    /// the effect shows, and the doubled damage is dropped with it.
    #[test]
    fn a_critical_guard_still_stuns() {
        let proto = guard_proto();
        let mut guard = guard(&proto);
        guard.fight.add(point::POINT_CRITICAL_PCT, 100);
        let boar = boar_proto();
        let victim = monster(&boar, 1 << 0, 0);
        let mut dice = Script::new(&[0, 0, 0]);
        let strike = strike_npc_by_npc(
            &mut dice,
            MobStriker {
                npc: &guard,
                berserk: false,
            },
            NpcVictim { npc: &victim },
        );
        assert!(dice.0.is_empty());
        assert_eq!(
            strike.events,
            vec![crate::character::HitEvent::Effect(
                crate::character::SE_CRITICAL
            )]
        );
        assert_eq!(strike.struck, crate::character::Struck::StunnedByGuard);
        assert_eq!(victim.hp, 1000);
    }

    #[test]
    fn reflect_damage_shares_the_pre_bonus_hit_and_thirds_when_immune() {
        assert_eq!(reflect_damage(0, false, 1000), 0);
        assert_eq!(reflect_damage(10, false, 1000), 100);
        // A small base rounds down to nothing: the runner still runs.
        assert_eq!(reflect_damage(6, false, 1), 0);
        // `int(reflect / 3.0f + 0.5f)`: 99 thirds to 33, 100 to 33, 101 to 34.
        assert_eq!(reflect_damage(99, true, 100), 33);
        assert_eq!(reflect_damage(100, true, 100), 33);
        assert_eq!(reflect_damage(101, true, 100), 34);
        assert_eq!(reflect_damage(10, true, 1000), 33);
        assert_eq!(IMMUNE_REFLECT, 1 << 6);
    }

    #[test]
    fn a_mob_swing_reports_its_pre_bonus_damage_for_the_reflect() {
        let proto = wind_proto();
        let mob = monster(&proto, 1 << 3, 0);
        let points = alpha();
        // Draw 0: `number(59, 110)` rolls 59, doubled to 118; the grade 140 less
        // the doubled level 70 is 188; the rating 0.6563... takes it to 193; no
        // party, percent, HUMAN, job or element share moves it; less the defence
        // 168 is 25, and `attMul` rounds it to 25.
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.reflected_base, Some(25));
        assert_eq!(
            strike.end,
            PcHitEnd::Landed {
                damage: 25,
                flag: 1
            }
        );
    }

    #[test]
    fn a_mob_rate_of_200_doubles_the_pre_bonus_damage() {
        let proto = wind_proto();
        let mob = monster(&proto, 1 << 3, 0);
        let points = alpha();
        let mut dice = Script::new(&[0]);
        let strike = hit_with(&mut dice, &mob, &points, false, 200);
        assert!(dice.0.is_empty());
        // The rate multiplies the attack 193 to 386 before the defence: 386 - 168
        // is 218, and `attMul` rounds it to 218.
        assert_eq!(strike.reflected_base, Some(218));
    }

    #[test]
    fn a_blocked_or_down_swing_still_reports_its_pre_bonus_damage() {
        let proto = wind_proto();
        let mob = monster(&proto, 1 << 3, 0);
        let mut points = alpha();
        points.set_point(point::POINT_BLOCK, 100);
        // The damage roll, then the block draw (raw 0 gives 1, held).
        let mut dice = Script::new(&[0, 0]);
        let strike = hit(&mut dice, &mob, &points, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.end, PcHitEnd::Blocked);
        assert_eq!(strike.reflected_base, Some(25));
        let points = alpha();
        let mut dice = Script::new(&[0]);
        let strike = hit(&mut dice, &mob, &points, true);
        assert!(dice.0.is_empty());
        assert_eq!(strike.end, PcHitEnd::Down);
        assert_eq!(strike.reflected_base, Some(25));
    }

    #[test]
    fn a_shot_or_skill_reports_no_pre_bonus_damage() {
        let proto = wind_proto();
        let mob = monster(&proto, 1 << 3, 0);
        let points = alpha();
        let mut dice = Script::new(&[0]);
        let strike = shot(&mut dice, &mob, &points, false, false);
        assert!(dice.0.is_empty());
        assert_eq!(strike.reflected_base, None);
        let victim = PcVictim {
            points: &points,
            affect_flags: [0; 2],
            down: false,
            immune_flag: 0,
        };
        let mut dice = Script::new(&[]);
        let strike = skill_damage_pc(&mut dice, &mob, &victim, 200, false);
        assert_eq!(strike.reflected_base, None);
    }
}
