//! A player's melee hit on a player.
//!
//! This is `battle_hit` (`G/battle.cpp:760-818`) with a player victim, and the
//! `DAMAGE_TYPE_NORMAL` path of `CHARACTER::Damage` (`G/char_battle.cpp:1939-2267`) on a
//! player victim, plus the `DAMAGE_TYPE_SPECIAL` path a victim's reflect runs back at
//! its attacker (`:1977-1988`).
//!
//! Every random draw is taken from the `Dice` in legacy's order; what the hit does comes
//! back as [`PvAction`]s in the order legacy does it. The caller applies them — the point
//! changes, the gold, the fly and the effect records — and the hit points the damage
//! takes, which the caller alone can move.
//!
//! What the Rewrite leaves out, because nothing it hosts can reach it:
//! * the victim is a player and the attacker holds no marriage bonus, so no
//!   `GetMarriageBonus` share is added to the critical or the penetrate chance;
//! * the victim's `AFF_JEONGWIHON`, `AFF_TERROR` and `AFF_HOSIN` affects and
//!   `AFF_MANASHIELD` never apply, the `SKILL_RESIST_PENETRATE` poly is 0, and the
//!   monarch's power-up and defence-up never apply;
//! * `cannot_dead` is false, the quest-set multipliers `attMul` and `damMul` are 1.0, and
//!   `BlueDragon_Damage` is the identity for every race the Rewrite stands up.

use common::point_slot as point;

use super::melee::{
    AttackAffect, AFF_POISON_WORD, AFF_SLOW_WORD, AFF_STUN_WORD, DAMAGE_CRITICAL, DAMAGE_NORMAL,
    DAMAGE_PENETRATE, FLY_HP_SMALL, FLY_SP_SMALL, SE_CRITICAL, SE_PENETRATE,
};
use super::mob_melee::{pc_hp_pct, reflect_damage, IMMUNE_REFLECT};
use crate::character::{number, Dice, Points};
use crate::npc::map_empire;

/// The attacker of a player's hit on a player (`G/battle.cpp:154-230`).
#[derive(Debug, Clone, Copy)]
pub struct PvAttacker<'a> {
    /// The attacker's points, which a stolen or recovered point changes.
    pub points: &'a Points,
    /// `GetEmpire`.
    pub empire: u8,
    /// `GetMapIndex`.
    pub map: i32,
    /// `CHARACTER_MANAGER::GetUserDamageRate` (`G/char_manager.cpp:1060-1070`): 100, or the
    /// `user_dam` event flag while a quest set it.
    pub user_rate: i32,
}

/// The victim of a player's hit on a player.
#[derive(Debug, Clone, Copy)]
pub struct PvVictim<'a> {
    /// The victim's points.
    pub points: &'a Points,
    /// The victim's first affect flag word (`IsAffectFlag`).
    pub affect_flags: u32,
}

/// The kind of damage a player takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvKind {
    /// `DAMAGE_TYPE_NORMAL`: the arm of a melee hit.
    Normal,
    /// `DAMAGE_TYPE_SPECIAL`: the arm a reflect runs back at its attacker. It skips the
    /// normal arm's block, critical, penetrate, steals and recoveries, and the type
    /// switch's bonuses.
    Special,
}

/// How a player's hit on a player ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvEnd {
    /// `POINT_BLOCK` held (`G/char_battle.cpp:1940-1955`): the damage packet shows 0 with
    /// `DAMAGE_BLOCK` and nothing else runs.
    Blocked,
    /// The hit took `damage` hit points, shown with `flag`.
    Landed {
        /// The hit points the victim loses.
        damage: i32,
        /// `DAMAGE_NORMAL`, with `DAMAGE_CRITICAL` and `DAMAGE_PENETRATE` when they hit.
        flag: u8,
    },
}

/// What a player's hit on a player does, in legacy's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvAction {
    /// `EffectPacket`: a `GC_SPECIAL_EFFECT` of this type on the victim, around it.
    Effect(u8),
    /// `CreateFly`: a `GC_CREATE_FLY` of this type from the victim to the attacker, around
    /// them.
    Fly(u8),
    /// `pAttacker->PointChange(kind, amount)`: the attacker's own point change.
    Attacker {
        /// The point slot.
        kind: usize,
        /// The signed change.
        amount: i32,
    },
    /// `PointChange(kind, amount)` on the victim.
    Victim {
        /// The point slot.
        kind: usize,
        /// The signed change.
        amount: i32,
    },
    /// `ChangeGold`: the attacker's purse gains this much gold.
    Gold(i32),
}

/// One player's hit on a player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PvStrike {
    /// How the hit ended.
    pub end: PvEnd,
    /// What the hit does, in legacy's order.
    pub actions: Vec<PvAction>,
    /// The damage the victim's reflect runs back at the attacker. `None` when the victim
    /// holds no reflect share: the legacy arm runs only on a share, so a victim without
    /// one never answers the hit. `Some(0)` is a share that rounds the damage away: the
    /// arm still runs, and the attacker still sees the record.
    pub reflected: Option<i32>,
}

/// `NormalAttackAffect` (`G/battle.cpp:734-758`) on a player victim: the poison, then the
/// stun and the slow. The poison's byte and the sung-ma halving are the attacker's; the
/// victim's flags and its immunity decide the arms.
///
/// The poison's own gates — a running poison event, and the poisoned-once gate that only
/// a non-player has — are the caller's.
#[must_use]
pub fn normal_attack_affect_on_player(
    dice: &mut dyn Dice,
    attacker: &Points,
    victim: &PvVictim<'_>,
) -> AttackAffect {
    let mut landed = AttackAffect::default();
    let poison = attacker.get_point(point::POINT_POISON_PCT);
    if poison != 0 && victim.affect_flags & AFF_POISON_WORD == 0 {
        // `BYTE bPoisonPct`: the low byte, halved when the map's immune will exceeds the
        // attacker's.
        let mut percent = poison.to_le_bytes()[0];
        if i32::from(attacker.will().immune) > attacker.get_point(point::POINT_SUNGMA_IMMUNE) {
            percent /= 2;
        }
        if number(dice, 1, 100) <= i32::from(percent) {
            // `AttackedByPoison`'s level arm (`G/char_resist.cpp:191-197`): the attacker's
            // level below the victim's, the adjust table by the difference.
            let over = i32::from(victim.points.level()).wrapping_sub(i32::from(attacker.level()));
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
    // arm, so a failed roll spends exactly one draw.
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
        let chance = attacker.get_point(slot);
        if chance != 0
            && victim.affect_flags & held == 0
            && number(dice, 1, 100) <= chance
            && victim.points.immune_flag() & immune == 0
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

/// A player's hit on a player: the victim's `Damage` (`G/char_battle.cpp:1939-2267`).
///
/// `damage` is the damage `battle_hit` computed: `CalcMeleeDamage` with the wielded
/// weapon's kind resist against the victim (`G/battle.cpp:762-800`).
#[must_use]
pub fn strike_pc_by_pc(
    dice: &mut dyn Dice,
    attacker: &PvAttacker<'_>,
    victim: &PvVictim<'_>,
    damage: i32,
    kind: PvKind,
) -> PvStrike {
    let mut actions = Vec::new();
    let mut damage = damage;
    let mut flag = DAMAGE_NORMAL;
    let mut reflected = None;
    if kind == PvKind::Normal {
        // The block (`G/char_battle.cpp:1940-1955`): the victim's own point.
        let block = victim.points.get_point(point::POINT_BLOCK);
        if block != 0 && number(dice, 1, 100) <= block {
            return PvStrike {
                end: PvEnd::Blocked,
                actions,
                reflected,
            };
        }
        // The reflect (`G/char_battle.cpp:1977-1988`): the victim's share of the damage
        // before the bonuses, run back at the attacker as a special hit. The arm runs on
        // the share, not on the damage it shares: a share that rounds the damage away
        // still answers the hit.
        let reflect = victim.points.get_point(point::POINT_REFLECT_MELEE);
        if reflect != 0 {
            reflected = Some(reflect_damage(
                reflect,
                attacker.points.immune_flag() & IMMUNE_REFLECT != 0,
                damage,
            ));
        }
        // The critical (`G/char_battle.cpp:1990-2007`): the attacker's chance less the
        // victim's resist, doubling the damage.
        let critical_pct = attacker.points.get_point(point::POINT_CRITICAL_PCT);
        if critical_pct != 0 {
            let critical_pct =
                critical_pct.wrapping_sub(victim.points.get_point(point::POINT_RESIST_CRITICAL));
            if number(dice, 1, 100) <= critical_pct {
                flag |= DAMAGE_CRITICAL;
                damage = damage.wrapping_mul(2);
                actions.push(PvAction::Effect(SE_CRITICAL));
            }
        }
        // The penetrate (`G/char_battle.cpp:2008-2040`): the attacker's chance less the
        // victim's resist, adding the victim's defence to the damage.
        let penetrate_pct = attacker.points.get_point(point::POINT_PENETRATE_PCT);
        if penetrate_pct != 0 {
            let penetrate_pct =
                penetrate_pct.wrapping_sub(victim.points.get_point(point::POINT_RESIST_PENETRATE));
            if number(dice, 1, 100) <= penetrate_pct {
                flag |= DAMAGE_PENETRATE;
                damage = damage.wrapping_add(
                    victim
                        .points
                        .get_point(point::POINT_DEF_GRADE)
                        .wrapping_mul(
                            100_i32.wrapping_add(victim.points.get_point(point::POINT_DEF_BONUS)),
                        )
                        / 100,
                );
                actions.push(PvAction::Effect(SE_PENETRATE));
            }
        }
        // The steals, the recoveries and the mana burn (`G/char_battle.cpp:2038-2122`).
        steals(dice, attacker, victim, damage, &mut actions);
    }
    // The type switch (`G/char_battle.cpp:2124-2139`): the attacker's hit bonus and the
    // victim's defend bonus with its combat resist. A special hit takes neither.
    if kind == PvKind::Normal {
        let bonus = attacker
            .points
            .get_point(point::POINT_NORMAL_HIT_DAMAGE_BONUS);
        if bonus != 0 {
            damage = damage.wrapping_mul(100_i32.wrapping_add(bonus)) / 100;
        }
        let defend = victim
            .points
            .get_point(point::POINT_NORMAL_HIT_DEFEND_BONUS)
            .min(99);
        let combat = if pc_hp_pct(victim.points) >= 90 {
            victim.points.get_point(point::POINT_RESIST_COMBAT).min(99)
        } else {
            0
        };
        damage = damage.wrapping_mul(100_i32.wrapping_sub(defend).wrapping_sub(combat)) / 100;
    }
    // The shared region (`G/char_battle.cpp:2165-2267`): the mall defence of the victim,
    // the mall attack bonus of the attacker, the empire tenth and the user damage rate.
    let mall_defence = victim.points.get_point(point::POINT_MALL_DEFBONUS);
    if mall_defence > 0 {
        damage = damage.wrapping_sub((damage.wrapping_mul(mall_defence) / 100).min(200));
    }
    let mall_attack = attacker.points.get_point(point::POINT_MALL_ATTBONUS);
    if mall_attack > 0 {
        let mall_attack = attacker.points.limit_point(point::POINT_MALL_ATTBONUS);
        damage = damage.wrapping_add((damage.wrapping_mul(mall_attack) / 100).min(300));
    }
    let map_empire = map_empire(attacker.map);
    if attacker.empire != 0 && map_empire != 0 && attacker.empire != map_empire {
        damage = damage.wrapping_mul(9) / 10;
    }
    PvStrike {
        end: PvEnd::Landed {
            damage: damage.wrapping_mul(attacker.user_rate) / 100,
            flag,
        },
        actions,
        reflected,
    }
}

/// The steals, the recoveries and the mana burn of `Damage`
/// (`G/char_battle.cpp:2038-2122`), which read the damage before the bonuses and the hit
/// points the victim had before the hit.
fn steals(
    dice: &mut dyn Dice,
    attacker: &PvAttacker<'_>,
    victim: &PvVictim<'_>,
    damage: i32,
    actions: &mut Vec<PvAction>,
) {
    let hit_points = victim.points.hp();
    let spell_points = victim.points.sp();
    // The steal of hit points (`G/char_battle.cpp:2038-2054`).
    let life = attacker.points.get_point(point::POINT_STEAL_HP);
    if life != 0 && number(dice, 1, 10) <= 1 {
        let stolen = damage.min(hit_points.max(0)).wrapping_mul(life) / 100;
        if stolen > 0 && hit_points >= stolen {
            actions.push(PvAction::Fly(FLY_HP_SMALL));
            actions.push(PvAction::Attacker {
                kind: point::POINT_HP,
                amount: stolen,
            });
            actions.push(PvAction::Victim {
                kind: point::POINT_HP,
                amount: -stolen,
            });
        }
    }
    // The steal of spell points (`G/char_battle.cpp:2055-2079`): a player's spell points.
    let mana = attacker.points.get_point(point::POINT_STEAL_SP);
    if mana != 0 && number(dice, 1, 10) <= 1 {
        let stolen = damage.min(spell_points.max(0)).wrapping_mul(mana) / 100;
        if stolen > 0 && spell_points >= stolen {
            actions.push(PvAction::Fly(FLY_SP_SMALL));
            actions.push(PvAction::Attacker {
                kind: point::POINT_SP,
                amount: stolen,
            });
            actions.push(PvAction::Victim {
                kind: point::POINT_SP,
                amount: -stolen,
            });
        }
    }
    // The steal of gold (`G/char_battle.cpp:2080-2092`): the amount is drawn from the
    // victim's level, and the purse it lands in is the attacker's.
    let gold = attacker.points.get_point(point::POINT_STEAL_GOLD);
    if gold != 0 && number(dice, 1, 100) <= gold {
        let amount = number(dice, 1, i32::from(victim.points.level()));
        actions.push(PvAction::Gold(amount));
    }
    // The recoveries (`G/char_battle.cpp:2095-2119`): the attacker's own hit points and
    // spell points, four chances in five.
    let recovered = if hit_points >= 0 {
        damage.min(hit_points)
    } else {
        damage
    };
    for (slot, kind, fly) in [
        (point::POINT_HIT_HP_RECOVERY, point::POINT_HP, FLY_HP_SMALL),
        (point::POINT_HIT_SP_RECOVERY, point::POINT_SP, FLY_SP_SMALL),
    ] {
        let share = attacker.points.get_point(slot);
        if share != 0 && number(dice, 0, 4) > 0 {
            let amount = recovered.wrapping_mul(share) / 100;
            if amount != 0 {
                actions.push(PvAction::Fly(fly));
                actions.push(PvAction::Attacker { kind, amount });
            }
        }
    }
    // The mana burn (`G/char_battle.cpp:2120-2122`): the victim loses 50 spell points.
    let burn = attacker.points.get_point(point::POINT_MANA_BURN_PCT);
    if burn != 0 && number(dice, 1, 100) <= burn {
        actions.push(PvAction::Victim {
            kind: point::POINT_SP,
            amount: -50,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use common::point_slot as point;

    use super::*;
    use crate::character::legacy_source::{legacy, span, strip_comments};
    use crate::character::PointsRow;

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

    /// A player with the given level and hit points, and no point but the ones a test sets.
    fn player(level: u8, hp: i32) -> Points {
        Points::load(&PointsRow {
            race: 3,
            level,
            exp: 0,
            conqueror_level: 0,
            st: 17,
            ht: 18,
            dx: 19,
            iq: 20,
            sungma: [0; 4],
            hp,
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
        })
    }

    /// The attacker of a test: a player on map 1 of empire 1, with the user rate at 100.
    fn attacker_of(points: &Points) -> PvAttacker<'_> {
        PvAttacker {
            points,
            empire: 1,
            map: 1,
            user_rate: 100,
        }
    }

    /// The victim of a test: a player with no affect.
    fn victim_of(points: &Points) -> PvVictim<'_> {
        PvVictim {
            points,
            affect_flags: 0,
        }
    }

    /// A victim's block holds the hit: the damage packet shows 0 and nothing else runs.
    #[test]
    fn a_blocked_victim_takes_nothing() {
        let attacker = player(20, 100);
        let mut victim = player(20, 100);
        victim.set_point(point::POINT_BLOCK, 100);
        let mut dice = Script::new(&[0; 8]);
        let strike = strike_pc_by_pc(
            &mut dice,
            &attacker_of(&attacker),
            &victim_of(&victim),
            40,
            PvKind::Normal,
        );
        assert_eq!(strike.end, PvEnd::Blocked);
        assert!(strike.actions.is_empty());
        assert_eq!(strike.reflected, None);
    }

    /// The critical doubles the damage and shows its effect (`G/char_battle.cpp:1990-2007`).
    #[test]
    fn a_critical_hit_doubles_the_damage() {
        let mut attacker = player(20, 100);
        let victim = player(20, 100);
        attacker.set_point(point::POINT_CRITICAL_PCT, 100);
        let mut dice = Script::new(&[0; 8]);
        let strike = strike_pc_by_pc(
            &mut dice,
            &attacker_of(&attacker),
            &victim_of(&victim),
            40,
            PvKind::Normal,
        );
        assert_eq!(
            strike.end,
            PvEnd::Landed {
                damage: 80,
                flag: DAMAGE_NORMAL | DAMAGE_CRITICAL
            }
        );
        assert_eq!(strike.actions, vec![PvAction::Effect(SE_CRITICAL)]);
    }

    /// The penetrate adds the victim's defence to the damage
    /// (`G/char_battle.cpp:2008-2040`).
    #[test]
    fn a_penetrating_hit_adds_the_victims_defence() {
        let mut attacker = player(20, 100);
        let mut victim = player(20, 100);
        attacker.set_point(point::POINT_PENETRATE_PCT, 100);
        victim.set_point(point::POINT_DEF_GRADE, 12);
        victim.set_point(point::POINT_DEF_BONUS, 50);
        let mut dice = Script::new(&[0; 8]);
        let strike = strike_pc_by_pc(
            &mut dice,
            &attacker_of(&attacker),
            &victim_of(&victim),
            40,
            PvKind::Normal,
        );
        assert_eq!(
            strike.end,
            PvEnd::Landed {
                damage: 58,
                flag: DAMAGE_NORMAL | DAMAGE_PENETRATE
            }
        );
        assert_eq!(strike.actions, vec![PvAction::Effect(SE_PENETRATE)]);
    }

    /// The gold steal draws its amount from the victim's level
    /// (`G/char_battle.cpp:2080-2092`).
    #[test]
    fn the_gold_steal_draws_the_victims_level() {
        let mut attacker = player(20, 100);
        let victim = player(7, 100);
        attacker.set_point(point::POINT_STEAL_GOLD, 100);
        let mut dice = Script::new(&[0, 3]);
        let strike = strike_pc_by_pc(
            &mut dice,
            &attacker_of(&attacker),
            &victim_of(&victim),
            40,
            PvKind::Normal,
        );
        // `number(1, 100)` takes the first draw, `number(1, GetLevel())` the second: 3 % 7 + 1.
        assert_eq!(strike.actions, vec![PvAction::Gold(4)]);
    }

    /// The victim's reflect runs back at the attacker as a special hit
    /// (`G/char_battle.cpp:1977-1988`).
    #[test]
    fn the_victims_reflect_runs_back_at_the_attacker() {
        let attacker = player(20, 100);
        let mut victim = player(20, 100);
        victim.set_point(point::POINT_REFLECT_MELEE, 25);
        let mut dice = Script::new(&[0; 8]);
        let strike = strike_pc_by_pc(
            &mut dice,
            &attacker_of(&attacker),
            &victim_of(&victim),
            40,
            PvKind::Normal,
        );
        assert_eq!(strike.reflected, Some(10));
        // A special hit takes neither the normal arm's bonuses nor its steals.
        let mut dice = Script::new(&[0; 8]);
        let special = strike_pc_by_pc(
            &mut dice,
            &PvAttacker {
                points: &victim,
                empire: 1,
                map: 1,
                user_rate: 100,
            },
            &victim_of(&attacker),
            strike.reflected.expect("the reflect shares the damage"),
            PvKind::Special,
        );
        assert_eq!(
            special.end,
            PvEnd::Landed {
                damage: 10,
                flag: DAMAGE_NORMAL
            }
        );
        assert!(special.actions.is_empty());
    }

    /// The stun lands on a player victim (`G/battle.cpp:734-758`).
    #[test]
    fn the_stun_lands_on_a_player_victim() {
        let mut attacker = player(20, 100);
        let victim = player(20, 100);
        attacker.set_point(point::POINT_STUN_PCT, 100);
        let mut dice = Script::new(&[0; 8]);
        let landed = normal_attack_affect_on_player(&mut dice, &attacker, &victim_of(&victim));
        assert!(landed.stunned);
        assert!(!landed.poisoned);
        assert!(!landed.slowed);
    }

    /// The arms of `Damage` run in legacy's order: the block, then the reflect, the
    /// critical and the penetrate, then the steals, then the bonuses.
    #[test]
    fn the_arms_of_damage_run_in_legacys_order() {
        let text = strip_comments(&legacy("game/char_battle.cpp"));
        let body = span(&text, "bool CHARACTER::Damage(", "}");
        let at = |from: usize, needle: &str| {
            from + body[from..]
                .iter()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle}"))
        };
        // The normal arm: the melee arm above it holds the same critical and penetrate
        // lines, and the order below is the normal arm's.
        let normal = at(
            0,
            "else if (type == DAMAGE_TYPE_NORMAL || type == DAMAGE_TYPE_NORMAL_RANGE)",
        );
        let block = at(
            normal,
            "GetPoint(POINT_BLOCK) && number(1, 100) <= GetPoint(POINT_BLOCK)",
        );
        let reflect = at(
            normal,
            "int reflectDamage = dam * GetPoint(POINT_REFLECT_MELEE) / 100;",
        );
        let critical = at(
            normal,
            "iCriticalPct = pAttacker->GetPoint(POINT_CRITICAL_PCT);",
        );
        let penetrate = at(
            normal,
            "iPenetratePct = pAttacker->GetPoint(POINT_PENETRATE_PCT);",
        );
        let steal_hit_points = at(normal, "if (pAttacker->GetPoint(POINT_STEAL_HP))");
        let steal_spell_points = at(normal, "if (pAttacker->GetPoint(POINT_STEAL_SP))");
        let steal_gold = at(normal, "if (pAttacker->GetPoint(POINT_STEAL_GOLD))");
        let hit_bonus = at(
            normal,
            "if (pAttacker->GetPoint(POINT_NORMAL_HIT_DAMAGE_BONUS))",
        );
        let mall = at(normal, "if (GetPoint(POINT_MALL_DEFBONUS) > 0)");
        let empire = at(
            normal,
            "int iMapEmpire = SECTREE_MANAGER::instance().GetEmpireFromMapIndex",
        );
        let user_rate = at(
            normal,
            "int iDmgPct = CHARACTER_MANAGER::instance().GetUserDamageRate",
        );
        assert!(block < reflect, "the block before the reflect");
        assert!(reflect < critical, "the reflect before the critical");
        assert!(critical < penetrate, "the critical before the penetrate");
        assert!(
            penetrate < steal_hit_points,
            "the penetrate before the steals"
        );
        assert!(
            steal_hit_points < steal_spell_points,
            "the hit point steal before the spell point steal"
        );
        assert!(
            steal_spell_points < steal_gold,
            "the spell point steal before the gold"
        );
        assert!(steal_gold < hit_bonus, "the steals before the bonuses");
        assert!(hit_bonus < mall, "the bonuses before the mall");
        assert!(mall < empire, "the mall before the empire tenth");
        assert!(empire < user_rate, "the empire tenth before the user rate");
    }
}
