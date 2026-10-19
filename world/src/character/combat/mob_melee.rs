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
//!
//! What the Rewrite leaves out, because nothing it hosts can reach it:
//! * the victim holds no affect yet, so `AFF_JEONGWIHON`, `AFF_TERROR`, `AFF_HOSIN` and
//!   `AFF_MANASHIELD` never scale the damage, and the flags gate the three draws open;
//! * a monster's proto leaves the steal, recovery, mana-burn, `MALL_ATTBONUS` and marriage
//!   points at 0, so those arms draw nothing and multiply by one;
//! * the `SKILL_RESIST_PENETRATE` poly is 0 for a victim with no skill;
//! * reflect, the guard attacker, the monarch and `BlueDragon_Damage` wait for later work;
//! * `attMul` and `damMul` are 1.0, `cannot_dead` is false, and the user damage rate is 100.
//!
//! What it draws but does not yet apply: the poison, stun and slow of `NormalAttackAffect`
//! wait for the affect work (V19i), as 229j did for a player's hit.

use common::cfloat::{f32_to_i32, i32_to_f32};
use common::point_slot as point;
use gamedata::mob_proto::{AIFLAG_DEATHBLOW, CHAR_TYPE_MONSTER};

use super::melee::{DAMAGE_CRITICAL, DAMAGE_NORMAL, DAMAGE_PENETRATE, SE_CRITICAL, SE_PENETRATE};
use super::{attack_rating, AttackRatingInput};
use crate::character::{number, race_to_job, Dice, Points};
use crate::npc::Npc;

/// `AFF_POISON` (`G/affect.h:214`): bit 4, the first word's `0x08`.
const AFF_POISON_WORD: u32 = 1 << 3;
/// `AFF_SLOW` (`G/affect.h:215`): bit 5.
const AFF_SLOW_WORD: u32 = 1 << 4;
/// `AFF_STUN` (`G/affect.h:216`): bit 6.
const AFF_STUN_WORD: u32 = 1 << 5;

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
}

/// What the hit showed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcStrike {
    /// `SE_CRITICAL`, `SE_PENETRATE`, in the order legacy sends them.
    pub effects: Vec<u8>,
    /// How the hit ended.
    pub end: PcHitEnd,
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

/// `battle_hit` of a melee monster on a player, from `CalcMeleeDamage` through `Damage`'s
/// flag: the damage roll, the `+0.5` of `attMul` 1.0, the block, critical and penetrate
/// chances, the bonuses, the down check, the death blow, and the `+0.5` of `damMul` 1.0.
pub fn strike_pc(dice: &mut dyn Dice, striker: MobStriker<'_>, victim: PcVictim<'_>) -> PcStrike {
    let mut effects = Vec::new();
    let damage = mob_melee_damage(dice, striker, victim.points);
    mob_attack_affect(dice, striker.npc, &victim);
    // `iDam = attMul * (float)iDam + 0.5f`, with `attMul` 1.0, which multiplies exactly.
    let damage = f32_to_i32(i32_to_f32(damage) + 0.5);
    if pc_blocks(dice, victim.points) {
        return PcStrike {
            effects,
            end: PcHitEnd::Blocked,
        };
    }
    let (damage, flag) = pc_chances(dice, striker.npc, victim.points, damage, &mut effects);
    let damage = pc_bonuses(striker.npc, victim.points, damage);
    if victim.down {
        return PcStrike {
            effects,
            end: PcHitEnd::Down,
        };
    }
    let damage = death_blow(dice, striker.npc, victim.points, damage);
    // `dam = (float)dam * damMul + 0.5f`, with `damMul` 1.0, which multiplies exactly.
    let damage = f32_to_i32(i32_to_f32(damage) + 0.5);
    PcStrike {
        effects,
        end: PcHitEnd::Landed { damage, flag },
    }
}

/// `CalcMeleeDamage` of a monster on a player, through `CalcBattleDamage`'s `<3` floor.
fn mob_melee_damage(dice: &mut dyn Dice, striker: MobStriker<'_>, victim: &Points) -> i32 {
    let mob = striker.npc;
    let rating = attack_rating(AttackRatingInput {
        attacker_dexterity: mob.fight.point(point::POINT_DX),
        attacker_level: i32::from(mob.fight.level),
        victim_dexterity: victim.get_point(point::POINT_DX),
        ignore_target_rating: false,
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
    attack = attack
        .wrapping_add(mob.fight.point(point::POINT_PARTY_ATTACKER_BONUS))
        .wrapping_mul(
            100_i32
                .wrapping_add(mob.fight.point(point::POINT_ATT_BONUS))
                .wrapping_add(mob.fight.point(point::POINT_MELEE_MAGIC_ATT_BONUS_PER)),
        )
        .wrapping_div(100);
    attack = mob_att_bonus(attack, mob, victim);
    let multiply = mob.fight.dam_multiply();
    let multiply = if striker.berserk {
        multiply * 2.0
    } else {
        multiply
    };
    attack = f32_to_i32(i32_to_f32(attack) * multiply);
    let defence = victim
        .get_point(point::POINT_DEF_GRADE)
        .wrapping_mul(100_i32.wrapping_add(victim.get_point(point::POINT_DEF_BONUS)))
        .wrapping_div(100);
    let damage = attack.wrapping_sub(defence).max(0);
    super::melee::battle_damage_floor(dice, damage)
}

/// `CalcAttBonus` for a non-player attacker and a player victim: the marriage transfer, the
/// mob damage rate, the attacker's `ATTBONUS_HUMAN` and job share, and the B27 element arm.
fn mob_att_bonus(attack: i32, mob: &Npc, victim: &Points) -> i32 {
    // `GetMarriageBonus(TRANSFER_DAMAGE)` and `GetMobDamageRate` are 0 and 100: no marriage
    // and no `mob_dam` event flag are ported.
    let attack = attack.wrapping_mul(100) / 100;
    let attack = attack.wrapping_mul(100) / 100;
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
    for (bit, resist, pct) in ARMS {
        if race_flag & bit != 0 {
            let product = attack
                .wrapping_mul(30)
                .wrapping_mul(victim.get_point(resist));
            // `iAtk -= (product) * GetPointPct(pct, 1) / 10000`: an `int` times a `float`
            // is a `float`, divided by an `int` is a `float`, and `iAtk -= float`
            // truncates toward 0.
            let percent = super::melee::point_pct(victim.get_point(pct));
            let take = f32_to_i32(i32_to_f32(product) * percent / 10_000.0);
            return attack.wrapping_sub(take);
        }
    }
    attack
}

/// `NormalAttackAffect`'s draws: the poison, then the stun and the slow. Each draws only when
/// the monster has the point and the player lacks the affect (V19i: drawn, not applied).
fn mob_attack_affect(dice: &mut dyn Dice, mob: &Npc, victim: &PcVictim<'_>) {
    let poison = mob.fight.point(point::POINT_POISON_PCT);
    if poison != 0 && victim.affect_flags[0] & AFF_POISON_WORD == 0 {
        // `BYTE bPoisonPct`: the low byte; the SungMa halving never fires for a monster.
        let percent = poison.to_le_bytes()[0];
        if number(dice, 1, 100) <= i32::from(percent)
            && i32::from(mob.fight.level) < i32::from(victim.points.level())
        {
            const ADJUST: [i32; 9] = [100, 90, 80, 70, 50, 30, 10, 5, 0];
            let delta = i32::from(victim.points.level())
                .wrapping_sub(i32::from(mob.fight.level))
                .min(8);
            let _resisted = number(dice, 1, 100) > ADJUST[usize::try_from(delta).unwrap_or(0)];
        }
    }
    for (slot, held) in [
        (point::POINT_STUN_PCT, AFF_STUN_WORD),
        (point::POINT_SLOW_PCT, AFF_SLOW_WORD),
    ] {
        let chance = mob.fight.point(slot);
        if chance != 0 && victim.affect_flags[0] & held == 0 {
            let _affected = number(dice, 1, 100) <= chance;
        }
    }
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
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
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
            },
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
            },
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
            },
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
                damage: 50,
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
                damage: 52,
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
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
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
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
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
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
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
            },
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
}
