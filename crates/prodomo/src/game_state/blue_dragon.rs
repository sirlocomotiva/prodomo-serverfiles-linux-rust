//! Blue Dragon (`2493`) battle AI, skills and stone-scaled recovery (`G/BlueDragon.cpp`,
//! `G/BlueDragon_Skill.h`, `G/char.cpp:3195-3241`).
//!
//! `CDragonLairManager` (`G/DragonLair.cpp`) is entered only through `DragonLair.startRaid`
//! (`lua.DragonLair.startRaid` is `unused`), whereas `BlueDragon_StateBattle` and the
//! `2493` HP recovery arms run whenever vnum 2493 stands in the world.

use common::cfloat::{f32_to_i32, i32_to_f32};
use common::dragon::{
    BlueDragonSetting, MinMax, BLUE_DRAGON_VNUM, DRAGON_STONE_REGEN_PCT, DRAGON_STONE_REGEN_TIME,
    SE_PERCENT_DAMAGE1, SE_PERCENT_DAMAGE2, SE_PERCENT_DAMAGE3,
};
use common::point_slot as point;
use common::vid::Vid;
use protocol::gc_actors::GcCharacterMove;
use protocol::gc_inventory::HEADER_GC_SYNC_POSITION;
use protocol::gc_position::GcSyncPositionElement;
use protocol::gc_vid::GcSpecialEffect;
use world::character::{is_female, number, race_to_job};
use world::npc::Npc;

use super::affect::Affect;
use super::fight::AFF_STUN_WORD;
use super::mob_ai::pulses;
use super::mob_skill::rotation_byte;
use super::motion::move_duration;
use super::pc_skill::IMMUNE_STUN;
use super::view::EntityKey;
use super::GameState;
use crate::loading_phase::{AFFECT_STUN, AFF_REVIVE_INVISIBLE, AFF_STUN};
use crate::movement::FUNC_MOB_SKILL;
use crate::save::PASSES_PER_SEC;
use crate::sync_position::distance_approx;

/// `AFF_REVIVE_INVISIBLE` bit in word 0 (`G/affect.h`).
const REVIVE_INVISIBLE_WORD: u32 = 1 << (AFF_REVIVE_INVISIBLE - 1);

/// `nFlyDistance` (`G/BlueDragon_Skill.h:288`): knockback distance of `EarthQuake`.
const EARTHQUAKE_FLY_DISTANCE: f32 = 1000.0;

impl GameState {
    /// `BlueDragon_StateBattle` (`G/BlueDragon.cpp:83-131`): selects skill priority by
    /// the dragon's HP percent (`> 98` waits 1 s; `> 76` tries `[1, 0, 2]`; `> 31` tries
    /// `[0, 1, 2]`; else `[0, 2, 1]`), runs the first off-cooldown skill, emits
    /// `FUNC_MOB_SKILL` around the dragon, and returns the hold in Pulses.
    pub(super) fn blue_dragon_state_battle(&mut self, mob: u32) -> u64 {
        let Some(hp_pct) = self.npc_by_vid(mob).map(npc_hp_pct) else {
            return u64::from(PASSES_PER_SEC);
        };
        if hp_pct > 98 {
            return u64::from(PASSES_PER_SEC);
        }
        let priority = if hp_pct > 76 {
            [1_usize, 0, 2]
        } else if hp_pct > 31 {
            [0, 1, 2]
        } else {
            [0, 2, 1]
        };
        let now = self.clock.now();
        for idx in priority {
            if self.blue_dragon_skill_due[idx].is_some_and(|due| due >= now) {
                continue;
            }
            let duration_s = self
                .npc_by_vid(mob)
                .and_then(|npc| npc.skills.get(idx))
                .and_then(world::npc::MobSkillSlots::duration_s)
                .map_or(0, |secs| f32_to_i32(secs).max(0));
            let next_s = self.use_blue_dragon_skill(mob, idx, hp_pct);
            let due = now
                .wrapping_add(next_s.saturating_mul(1000))
                .wrapping_add(u32::try_from(duration_s).unwrap_or(0))
                .wrapping_add(3000);
            self.blue_dragon_skill_due[idx] = Some(due);
            if let Some(spot) = self.npc_spot(mob) {
                let record = GcCharacterMove::new(
                    FUNC_MOB_SKILL,
                    u8::try_from(idx).unwrap_or(0),
                    rotation_byte(self.minds.get(mob).map_or(0.0, |mind| mind.rotation)),
                    mob,
                    spot.0,
                    spot.1,
                    now,
                    0,
                )
                .encode();
                let me = EntityKey::Npc(mob);
                self.packet_around(me, &record, Some(me));
            }
            return if duration_s == 0 {
                u64::from(PASSES_PER_SEC)
            } else {
                pulses(duration_s)
            };
        }
        u64::from(PASSES_PER_SEC)
    }

    /// `UseBlueDragonSkill` (`G/BlueDragon.cpp:19-81`): dispatches Breath (0), `WeakBreath` (1),
    /// or `EarthQuake` (2) across every living non-revive-invisible PC on the dragon's map,
    /// then scales the rolled cooldown by `BlueDragonSetting.hp_period`.
    fn use_blue_dragon_skill(&mut self, mob: u32, idx: usize, hp_pct: i32) -> u32 {
        let Some(&(channel, map, _)) = self.npc_of.get(&mob) else {
            return 0;
        };
        let Some(origin) = self.npc_spot(mob) else {
            return 0;
        };
        let setting = (*self.blue_dragon).clone();
        let targets = self.blue_dragon_targets(channel, map);
        let base_period = match idx {
            0 => {
                self.blue_dragon_breath(mob, origin, hp_pct, &setting, &targets);
                self.roll_min_max(setting.skill0.period)
            }
            1 => {
                self.blue_dragon_weak_breath(mob, origin, hp_pct, &setting, &targets);
                self.roll_min_max(setting.skill1.period)
            }
            2 => {
                self.blue_dragon_earthquake(mob, map, origin, hp_pct, &setting, &targets);
                self.roll_min_max(setting.skill2.period)
            }
            _ => return 0,
        };
        let add_pct =
            i32::try_from(BlueDragonSetting::range_factor(&setting.hp_period, hp_pct)).unwrap_or(0);
        let scaled = base_period.saturating_add(base_period.saturating_mul(add_pct) / 100);
        u32::try_from(scaled.max(0)).unwrap_or(0)
    }

    /// All living PCs on `(channel, map)` without `AFF_REVIVE_INVISIBLE`, in VID order.
    fn blue_dragon_targets(&self, channel: u8, map: i32) -> Vec<Vid> {
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(vid, body)| {
                body.channel == channel
                    && body.map == map
                    && !self.is_down(vid.raw())
                    && self.affect_flags_of(**vid)[0] & REVIVE_INVISIBLE_WORD == 0
            })
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        vids
    }

    /// `FSkillBreath` (`G/BlueDragon_Skill.h:4-134`).
    fn blue_dragon_breath(
        &mut self,
        mob: u32,
        origin: (i32, i32),
        hp_pct: i32,
        setting: &BlueDragonSetting,
        targets: &[Vid],
    ) {
        let set1 = u8::try_from(number(&mut self.dice, 0, 3)).unwrap_or(0);
        let set2 = u8::try_from(number(&mut self.dice, 0, 3)).unwrap_or(0);
        let gender = u8::try_from(number(&mut self.dice, 0, 2)).unwrap_or(0);
        let add_pct =
            i32::try_from(BlueDragonSetting::range_factor(&setting.hp_damage, hp_pct)).unwrap_or(0);
        for &vid in targets {
            let Some(spot) = self.spot_of(vid.raw()) else {
                continue;
            };
            if distance_approx(
                origin.0.saturating_sub(spot.x),
                origin.1.saturating_sub(spot.y),
            ) > setting.skill0.damage_area
            {
                continue;
            }
            let Some((job, sex, max_hp)) = self.pc_combat_facts(vid) else {
                continue;
            };
            let mut pct = 0_i32;
            let mut overlap = 0_u8;
            if job == set1 {
                let rolled = self.roll_min_max(setting.skill0.job_damage[usize::from(job)]);
                pct = pct.saturating_add(rolled);
                if rolled > 0 {
                    overlap = overlap.saturating_add(1);
                }
            }
            if job == set2 {
                let rolled = self.roll_min_max(setting.skill0.job_damage[usize::from(job)]);
                pct = pct.saturating_add(rolled);
                if rolled > 0 {
                    overlap = overlap.saturating_add(1);
                }
            }
            if sex == gender {
                let rolled = self.roll_min_max(setting.skill0.gender_damage[usize::from(sex)]);
                pct = pct.saturating_add(rolled);
                if rolled > 0 {
                    overlap = overlap.saturating_add(1);
                }
            }
            let effect = match overlap {
                1 => Some(SE_PERCENT_DAMAGE1),
                2 => Some(SE_PERCENT_DAMAGE2),
                3 => Some(SE_PERCENT_DAMAGE3),
                _ => None,
            };
            if let Some(effect_type) = effect {
                let mut record = Vec::new();
                GcSpecialEffect {
                    effect_type,
                    vid: vid.raw(),
                }
                .encode_into(&mut record);
                self.packet_around(EntityKey::Character(vid.raw()), &record, None);
            }
            pct = pct.saturating_add(add_pct);
            let mut dam = self.roll_min_max(setting.skill0.default_damage);
            dam = dam.saturating_add(dam.saturating_mul(add_pct) / 100);
            dam = dam.saturating_add(max_hp.saturating_mul(pct) / 100);
            let strike = self.melee_arm_pc(mob, vid, dam, true);
            self.show_skill_strike(mob, vid, &strike);
        }
    }

    /// `FSkillWeakBreath` (`G/BlueDragon_Skill.h:136-176`).
    fn blue_dragon_weak_breath(
        &mut self,
        mob: u32,
        origin: (i32, i32),
        hp_pct: i32,
        setting: &BlueDragonSetting,
        targets: &[Vid],
    ) {
        let add_pct =
            i32::try_from(BlueDragonSetting::range_factor(&setting.hp_damage, hp_pct)).unwrap_or(0);
        for &vid in targets {
            let Some(spot) = self.spot_of(vid.raw()) else {
                continue;
            };
            if distance_approx(
                origin.0.saturating_sub(spot.x),
                origin.1.saturating_sub(spot.y),
            ) > setting.skill1.damage_area
            {
                continue;
            }
            let mut dam = self.roll_min_max(setting.skill1.default_damage);
            dam = dam.saturating_add(dam.saturating_mul(add_pct) / 100);
            let strike = self.melee_arm_pc(mob, vid, dam, true);
            self.show_skill_strike(mob, vid, &strike);
        }
    }

    /// `FSkillEarthQuake` (`G/BlueDragon_Skill.h:178-342`): deals ice damage, applies
    /// `BDRAGON_STUN` (`AFFECT_STUN`/`AFF_STUN` gated by `IMMUNE_STUN`), knocks each victim
    /// back along the normalized dragon-to-victim vector, and calls `BeginFight` on the
    /// farthest struck player.
    fn blue_dragon_earthquake(
        &mut self,
        mob: u32,
        map: i32,
        origin: (i32, i32),
        hp_pct: i32,
        setting: &BlueDragonSetting,
        targets: &[Vid],
    ) {
        let set1 = u8::try_from(number(&mut self.dice, 0, 3)).unwrap_or(0);
        let set2 = u8::try_from(number(&mut self.dice, 0, 3)).unwrap_or(0);
        let gender = u8::try_from(number(&mut self.dice, 0, 2)).unwrap_or(0);
        let add_pct =
            i32::try_from(BlueDragonSetting::range_factor(&setting.hp_damage, hp_pct)).unwrap_or(0);
        let mut farthest: Option<(i32, Vid)> = None;
        for &vid in targets {
            let Some(spot) = self.spot_of(vid.raw()) else {
                continue;
            };
            if distance_approx(
                origin.0.saturating_sub(spot.x),
                origin.1.saturating_sub(spot.y),
            ) > setting.skill2.damage_area
            {
                continue;
            }
            let Some((job, sex, _)) = self.pc_combat_facts(vid) else {
                continue;
            };
            let mut sec = self.roll_min_max(setting.skill2.stun_default);
            if job == set1 {
                sec = sec
                    .saturating_add(self.roll_min_max(setting.skill2.job_stun[usize::from(job)]));
            }
            if job == set2 {
                sec = sec
                    .saturating_add(self.roll_min_max(setting.skill2.job_stun[usize::from(job)]));
            }
            if sex == gender {
                sec = sec.saturating_add(
                    self.roll_min_max(setting.skill2.gender_stun[usize::from(sex)]),
                );
            }
            let mut dam = self.roll_min_max(setting.skill2.default_damage);
            dam = dam.saturating_add(dam.saturating_mul(add_pct) / 100);
            let strike = self.melee_arm_pc(mob, vid, dam, true);
            self.show_skill_strike(mob, vid, &strike);
            self.apply_blue_dragon_stun(vid, sec);
            let (tx, ty) = self.earthquake_knockback(map, origin, (spot.x, spot.y));
            self.slide_pc_to(vid, tx, ty);
            let dist = distance_approx(origin.0.saturating_sub(tx), origin.1.saturating_sub(ty));
            if farthest.is_none_or(|(best, _)| dist > best) {
                farthest = Some((dist, vid));
            }
        }
        if let Some((_, far_vid)) = farthest {
            self.begin_fight(mob, far_vid);
        }
    }

    /// `SkillAttackAffect(..., IMMUNE_STUN, AFFECT_STUN, POINT_NONE, 0, AFF_STUN, sec, ...)`
    /// (`G/BlueDragon_Skill.h:276`).
    fn apply_blue_dragon_stun(&mut self, vid: Vid, sec: i32) {
        if sec <= 0 || self.is_down(vid.raw()) {
            return;
        }
        let immune = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(world::character::Character::points)
            .is_some_and(|points| points.immune_flag() & IMMUNE_STUN != 0);
        if immune || self.affect_flags_of(vid)[0] & AFF_STUN_WORD != 0 {
            return;
        }
        self.add_pc_affect(
            vid,
            Affect {
                kind: AFFECT_STUN,
                apply_on: 0,
                value: 0,
                flag: AFF_STUN,
                remaining: sec,
                sp_cost: 0,
            },
            false,
        );
    }

    /// `FSkillEarthQuake` knockback target (`G/BlueDragon_Skill.h:281-323`).
    fn earthquake_knockback(
        &mut self,
        map: i32,
        origin: (i32, i32),
        spot: (i32, i32),
    ) -> (i32, i32) {
        let dx = i32_to_f32(origin.0.saturating_sub(spot.0));
        let dy = i32_to_f32(origin.1.saturating_sub(spot.1));
        let len = dx.hypot(dy);
        let (vx, vy) = if len > 0.0 {
            (dx / len, dy / len)
        } else {
            (0.0, 0.0)
        };
        let mut tx = spot
            .0
            .saturating_add(f32_to_i32(vx * EARTHQUAKE_FLY_DISTANCE));
        let mut ty = spot
            .1
            .saturating_add(f32_to_i32(vy * EARTHQUAKE_FLY_DISTANCE));
        for attempt in 0..5 {
            if self.is_movable(map, tx, ty) {
                break;
            }
            match attempt {
                0 => {
                    tx = spot
                        .0
                        .saturating_add(f32_to_i32(vx * -EARTHQUAKE_FLY_DISTANCE));
                    ty = spot
                        .1
                        .saturating_add(f32_to_i32(vy * -EARTHQUAKE_FLY_DISTANCE));
                }
                1 => {
                    tx = spot
                        .0
                        .saturating_add(f32_to_i32(vx * -EARTHQUAKE_FLY_DISTANCE));
                    ty = spot
                        .1
                        .saturating_add(f32_to_i32(vy * EARTHQUAKE_FLY_DISTANCE));
                }
                2 => {
                    tx = spot
                        .0
                        .saturating_add(f32_to_i32(vx * EARTHQUAKE_FLY_DISTANCE));
                    ty = spot
                        .1
                        .saturating_add(f32_to_i32(vy * -EARTHQUAKE_FLY_DISTANCE));
                }
                3 => {
                    let rx = i32_to_f32(number(&mut self.dice, 1, 100));
                    let ry = i32_to_f32(number(&mut self.dice, 1, 100));
                    tx = spot.0.saturating_add(f32_to_i32(vx * rx));
                    ty = spot.1.saturating_add(f32_to_i32(vy * ry));
                }
                _ => {
                    let rx = i32_to_f32(number(&mut self.dice, 1, 10));
                    let ry = i32_to_f32(number(&mut self.dice, 1, 10));
                    tx = spot.0.saturating_add(f32_to_i32(vx * rx));
                    ty = spot.1.saturating_add(f32_to_i32(vy * ry));
                }
            }
        }
        (tx, ty)
    }

    /// `Sync(tx, ty)` + `Goto(tx, ty)` + `CalculateMoveDuration()` + `SyncPacket()`
    /// (`G/BlueDragon_Skill.h:325-329`).
    fn slide_pc_to(&mut self, vid: Vid, tx: i32, ty: i32) {
        self.sync_body(vid.raw(), tx, ty);
        let now = self.clock.now();
        let move_speed = self.move_speed_of(vid.raw());
        let motion_speed = self.motion_speed_of(vid.raw());
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.motion.start = (tx, ty);
            body.motion.dest = (tx, ty);
            body.motion.duration_ms = move_duration((tx, ty), (tx, ty), motion_speed, move_speed);
            body.motion.start_ms = now;
            body.motion.moving = false;
        }
        let mut record = vec![HEADER_GC_SYNC_POSITION.value(), 0, 0];
        record.extend_from_slice(&GcSyncPositionElement::new(vid.raw(), tx, ty).encode());
        let size = u16::try_from(record.len()).unwrap_or(u16::MAX);
        record[1..3].copy_from_slice(&size.to_le_bytes());
        self.packet_around(EntityKey::Character(vid.raw()), &record, None);
    }

    /// `(job 0..3, sex 0=male/1=female, max_hp)` for `vid`.
    fn pc_combat_facts(&self, vid: Vid) -> Option<(u8, u8, i32)> {
        let points = self.characters.find_by_vid(vid).ok()?.points()?;
        let job = race_to_job(points.race());
        if job > 3 {
            return None;
        }
        let sex = u8::from(is_female(points.race()));
        let max_hp = points.get_point(point::POINT_MAX_HP);
        Some((job, sex, max_hp))
    }

    /// Inclusive roll over `range` (`number(min, max)`).
    fn roll_min_max(&mut self, range: MinMax) -> i32 {
        number(&mut self.dice, range.min, range.max)
    }

    /// `recovery_event`'s Blue Dragon arms (`G/char.cpp:3195-3241`): returns
    /// `(hp_gain, next_cycle_pulses)` for vnum 2493, or `None` for any other vnum.
    pub(super) fn blue_dragon_recovery_params(&self, vid: u32, npc: &Npc) -> Option<(i64, u64)> {
        if npc.vnum != BLUE_DRAGON_VNUM {
            return None;
        }
        let hp_pct = npc_hp_pct(npc);
        let (_, counts) = self.dragon_strike_facts(vid);
        let stones = &self.blue_dragon.stones;
        let mut regen_pct = i64::from(BlueDragonSetting::range_factor(
            &self.blue_dragon.hp_regen,
            hp_pct,
        )) + i64::from(npc.fight.regen_percent);
        for (idx, stone) in stones.iter().enumerate() {
            if stone.effect == DRAGON_STONE_REGEN_PCT {
                regen_pct += i64::from(stone.val) * i64::try_from(counts[idx]).unwrap_or(0);
                break;
            }
        }
        let gain = (i64::from(npc.max_hp) * regen_pct / 100).max(1);
        let mut cycle_s = i32::from(npc.fight.regen_cycle);
        for (idx, stone) in stones.iter().enumerate() {
            if stone.effect == DRAGON_STONE_REGEN_TIME {
                let cut =
                    i32::try_from(stone.val).unwrap_or(0) * i32::try_from(counts[idx]).unwrap_or(0);
                cycle_s = cycle_s.saturating_sub(cut);
                break;
            }
        }
        Some((gain, pulses(cycle_s.max(1))))
    }
}

/// `CHARACTER::GetHPPct` (`G/char.h`): `(hp * 100) / max_hp.max(1)`.
fn npc_hp_pct(npc: &Npc) -> i32 {
    if npc.max_hp <= 0 {
        return 0;
    }
    i32::try_from(i64::from(npc.hp) * 100 / i64::from(npc.max_hp)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use common::dragon::{
        BlueDragonSetting, BreathSkill, DragonStone, EarthQuakeSkill, MinMax, RangeFactor,
        WeakBreathSkill, BLUE_DRAGON_VNUM, DRAGON_STONE_REGEN_PCT, DRAGON_STONE_REGEN_TIME,
        SE_PERCENT_DAMAGE1, SE_PERCENT_DAMAGE2, SE_PERCENT_DAMAGE3,
    };
    use common::vid::Vid;
    use gamedata::mob_proto::CHAR_TYPE_MONSTER;
    use protocol::gc_actors::GcCharacterMove;
    use protocol::gc_inventory::{
        HEADER_GC_DAMAGE_INFO, HEADER_GC_SEPCIAL_EFFECT, HEADER_GC_SYNC_POSITION,
    };
    use world::npc::Npc;

    use super::super::fight::AFF_STUN_WORD;
    use super::super::fixtures::{enter, open_cells, stand_field, TestClock};
    use super::super::mob_ai::pulses;
    use crate::movement::FUNC_MOB_SKILL;

    fn sample_setting() -> BlueDragonSetting {
        BlueDragonSetting {
            stones: [
                DragonStone {
                    vnum: 8031,
                    effect: 1,
                    val: 25,
                    enemy: 20110,
                    enemy_val: 3,
                },
                DragonStone {
                    vnum: 8032,
                    effect: 2,
                    val: 25,
                    enemy: 20111,
                    enemy_val: 3,
                },
                DragonStone {
                    vnum: 8033,
                    effect: DRAGON_STONE_REGEN_PCT,
                    val: 20,
                    enemy: 20112,
                    enemy_val: 3,
                },
                DragonStone {
                    vnum: 8034,
                    effect: DRAGON_STONE_REGEN_TIME,
                    val: 10,
                    enemy: 20113,
                    enemy_val: 3,
                },
            ],
            hp_period: vec![
                RangeFactor {
                    min: 76,
                    max: 100,
                    pct: 0,
                },
                RangeFactor {
                    min: 31,
                    max: 75,
                    pct: 15,
                },
                RangeFactor {
                    min: 0,
                    max: 30,
                    pct: 30,
                },
            ],
            hp_damage: vec![
                RangeFactor {
                    min: 76,
                    max: 100,
                    pct: 0,
                },
                RangeFactor {
                    min: 31,
                    max: 75,
                    pct: 10,
                },
                RangeFactor {
                    min: 0,
                    max: 30,
                    pct: 20,
                },
            ],
            hp_regen: vec![
                RangeFactor {
                    min: 76,
                    max: 97,
                    pct: 4,
                },
                RangeFactor {
                    min: 31,
                    max: 75,
                    pct: 8,
                },
            ],
            skill0: BreathSkill {
                period: MinMax { min: 30, max: 30 },
                damage_area: 5000,
                default_damage: MinMax { min: 500, max: 500 },
                job_damage: [MinMax { min: 10, max: 10 }; 4],
                gender_damage: [MinMax { min: 10, max: 10 }; 2],
            },
            skill1: WeakBreathSkill {
                period: MinMax { min: 10, max: 10 },
                damage_area: 3500,
                default_damage: MinMax { min: 300, max: 300 },
            },
            skill2: EarthQuakeSkill {
                period: MinMax { min: 35, max: 35 },
                damage_area: 3000,
                default_damage: MinMax { min: 200, max: 200 },
                stun_default: MinMax { min: 2, max: 2 },
                job_stun: [MinMax { min: 1, max: 1 }; 4],
                gender_stun: [MinMax { min: 1, max: 1 }; 2],
            },
        }
    }

    fn dragon_npc(vid: u32, hp: i32, max_hp: i32, at: (i32, i32)) -> Npc {
        let mut npc = Npc {
            vid,
            vnum: BLUE_DRAGON_VNUM,
            char_type: CHAR_TYPE_MONSTER,
            hp,
            max_hp,
            x: at.0,
            y: at.1,
            ..Npc::default()
        };
        npc.fight.regen_cycle = 15;
        npc.fight.regen_percent = 5;
        npc
    }

    #[test]
    fn blue_dragon_state_battle_fires_priority_skills_breath_weak_breath_and_earthquake() {
        let clock = TestClock::default();
        let mut world = stand_field(
            &clock,
            vec![dragon_npc(200, 9900, 10_000, (5200, 5000))],
            open_cells(),
            true,
        )
        .with_blue_dragon(Arc::new(sample_setting()));
        let mut inbox = enter(&mut world, 10, (5000, 5000), 0);
        let vid = Vid::new(10);
        world.start_pc_recovery(vid);
        if let Ok(ch) = world.characters.find_by_vid_mut(vid) {
            if let (_, Some(points)) = ch.items_and_points_mut() {
                let _ = points.point_change(common::point_slot::POINT_MAX_HP, 50_000, false, false);
                let _ = points.point_change(common::point_slot::POINT_HP, 50_000, false, false);
            }
        }
        let _ = world.remove_pc_affect(vid, crate::loading_phase::AFFECT_REVIVE_INVISIBLE);
        while inbox.try_recv().is_ok() {}

        // > 98% HP: no skill fires.
        let hold = world.blue_dragon_state_battle(200);
        assert_eq!(hold, pulses(1));
        assert!(inbox.try_recv().is_err(), "silent above 98% HP");

        // 80% HP: priority [1, 0, 2] fires WeakBreath (1), then Breath (0), then EarthQuake (2).
        world.npc_mut(200).expect("dragon").hp = 8000;
        let _ = world.blue_dragon_state_battle(200);
        let mut first = Vec::new();
        while let Ok(pkt) = inbox.try_recv() {
            first.push(pkt);
        }
        assert!(
            first.iter().any(|p| p[0] == HEADER_GC_DAMAGE_INFO.value()),
            "WeakBreath damaged"
        );
        assert!(
            first.iter().any(|p| GcCharacterMove::decode(p)
                .is_ok_and(|m| m.b_func == FUNC_MOB_SKILL && m.b_arg == 1)),
            "WeakBreath sent FUNC_MOB_SKILL arg 1"
        );

        let _ = world.blue_dragon_state_battle(200);
        let mut second = Vec::new();
        while let Ok(pkt) = inbox.try_recv() {
            second.push(pkt);
        }
        assert!(
            second.iter().any(|p| p[0] == HEADER_GC_DAMAGE_INFO.value()),
            "Breath damaged"
        );
        assert!(
            second.iter().any(|p| GcCharacterMove::decode(p)
                .is_ok_and(|m| m.b_func == FUNC_MOB_SKILL && m.b_arg == 0)),
            "Breath sent FUNC_MOB_SKILL arg 0"
        );
        for p in &second {
            if p[0] == HEADER_GC_SEPCIAL_EFFECT.value() {
                assert!(matches!(
                    p[1],
                    SE_PERCENT_DAMAGE1 | SE_PERCENT_DAMAGE2 | SE_PERCENT_DAMAGE3
                ));
            }
        }

        let _ = world.blue_dragon_state_battle(200);
        let mut third = Vec::new();
        while let Ok(pkt) = inbox.try_recv() {
            third.push(pkt);
        }
        assert!(
            third.iter().any(|p| p[0] == HEADER_GC_DAMAGE_INFO.value()),
            "EarthQuake damaged"
        );
        assert!(
            third
                .iter()
                .any(|p| p[0] == HEADER_GC_SYNC_POSITION.value()),
            "EarthQuake synced knockback"
        );
        assert!(
            third.iter().any(|p| GcCharacterMove::decode(p)
                .is_ok_and(|m| m.b_func == FUNC_MOB_SKILL && m.b_arg == 2)),
            "EarthQuake sent FUNC_MOB_SKILL arg 2"
        );
        assert_ne!(
            world.affect_flags_of(vid)[0] & AFF_STUN_WORD,
            0,
            "EarthQuake stunned the victim"
        );
        assert_eq!(
            world.minds.get(200).and_then(|mind| mind.victim),
            Some(super::super::mob_ai::Victim::Here(Vid::new(10))),
            "EarthQuake targeted the farthest struck player"
        );
    }

    #[test]
    fn blue_dragon_recovery_scales_regen_percent_and_cycle_from_stones() {
        let clock = TestClock::default();
        let mut stones = vec![dragon_npc(200, 5000, 10_000, (5200, 5000))];
        for (vid, vnum) in [(201, 8033), (202, 8034)] {
            stones.push(Npc {
                vid,
                vnum,
                char_type: gamedata::mob_proto::CHAR_TYPE_STONE,
                hp: 1000,
                max_hp: 1000,
                x: 5300,
                y: 5000,
                ..Npc::default()
            });
        }
        let world = stand_field(&clock, stones, open_cells(), true)
            .with_blue_dragon(Arc::new(sample_setting()));
        let npc = world.npc_by_vid(200).expect("dragon").clone();
        // 50% HP -> hp_regen=8 + bRegenPercent=5 + 1*20 = 33% of 10_000 = 3300; cycle = 15 - 10 = 5 s.
        let (gain, cycle) = world
            .blue_dragon_recovery_params(200, &npc)
            .expect("blue dragon params");
        assert_eq!(gain, 3300);
        assert_eq!(cycle, pulses(5));
    }
}
