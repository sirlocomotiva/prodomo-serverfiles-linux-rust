//! A monster's skill cast (`G/char_skill.cpp:3322-3525`): the Battle scan, the
//! gates, `UseMobSkill`, the hit events, and `ComputeSkillAtPosition`'s splash arm
//! (ledger 229u).
//!
//! Everything here runs on the game thread only (ADR-0002).
//!
//! What is ported: the Battle scan (`G/char_state.cpp:1138-1170`), `HasMobSkill` and
//! `CanUseMobSkill` (`:3322-3368`), `UseMobSkill` (`:3429-3525`) with the 162 self-heal,
//! the hit event (`:3389-3412`), and the splash-damage arm of `ComputeSkillAtPosition`
//! (`:2115-2441`) through `FuncSplashDamage` (`:1360-2000`) down to `Damage` — for the
//! seven skill vnums the owner's known-skill mobs name (20, 256, 258, 263, 264, 265,
//! 266), plus vnum 270's refusal (no such row).
//!
//! What stays out, because no hosted cast reaches it: the splash-affect arm's other points
//! (only 263's `MOV_SPEED` slow fires); the `KNOCKBACK`/`REMOVE_GOOD`/`SLOW`/`STUN`/
//! `FIRE_CONT`/`POISON`/`HP_ABSORB`/`SP_ABSORB` arms (no hosted row sets them); the
//! PC-only `HitOnce`/main-target arms; grandmaster (an NPC's master type is 0); the
//! anti-skill and 7/8th-help arms (unhosted vnums).

use std::collections::BTreeMap;
use std::sync::Arc;

use common::cfloat::{f32_to_i32, f64_to_i32, i32_to_f32};
use common::point_slot as point;
use common::skill_poly::Poly;
use common::vid::Vid;
use gamedata::mob_proto::CHAR_TYPE_MONSTER;
use gamedata::mob_skill_data::{
    MobSkillClips, MobSkills, HEALING_SKILL_VNUM, SKILL_ATTR_MELEE, SKILL_ATTR_NORMAL,
    SKILL_FLAG_ATTACK, SKILL_FLAG_CRUSH, SKILL_FLAG_IGNORE_TARGET_RATING, SKILL_FLAG_PARTY,
    SKILL_FLAG_PENETRATE, SKILL_FLAG_SPLASH, SKILL_FLAG_USE_MAGIC_DAMAGE,
    SKILL_FLAG_USE_MELEE_DAMAGE,
};
use protocol::gc_actors::GcCharacterMove;
use protocol::gc_inventory::HEADER_GC_SYNC_POSITION;
use protocol::gc_position::GcSyncPositionElement;
use protocol::gc_vid::GcSpecialEffect;
use world::character::{
    attack_rating, mob_calc_attack, number, skill_damage_pc, AttackRatingInput, MobAttackAffect,
    MobStriker, PcHitEnd, PcStrike, PcVictim, SE_EFFECT_HEALER,
};
use world::npc::Npc;

use super::affect::Affect;
use super::death::Attacker;
use super::fight::SkillEvent;
use super::mob_ai::delta_by_degree;
use super::motion::{move_duration, rotation_to_xy};
use super::regen::Due;
use super::view::EntityKey;
use super::GameState;
use crate::movement::FUNC_MOB_SKILL;
use crate::save::PASSES_PER_SEC;
use crate::sync_position::distance_approx;

/// `PASSES_PER_SEC(2)`: the duration of a cast whose clip is missing
/// (`G/char_state.cpp:1160`).
const MISSING_CLIP_PULSES: u64 = 2 * (PASSES_PER_SEC as u64);

/// `IsMonster()`: the 162 heal arm's gate.
fn is_monster(npc: &Npc) -> bool {
    npc.char_type == CHAR_TYPE_MONSTER
}

/// `point_on` as `FindPointType` reads it (`G/skill.cpp:81-89`): the hosted rows name only
/// `NONE`, `HP` and `MOV_SPEED`.
fn point_on(field: &[u8]) -> usize {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    match &field[..end] {
        b"HP" => point::POINT_HP,
        b"MOV_SPEED" => point::POINT_MOV_SPEED,
        _ => point::POINT_NONE,
    }
}

/// The damage roll of the monster's range: the `u32` pair reinterpreted as `i32` (a
/// `DWORD` pair read as `int`, as the legacy roll passes them).
fn damage_roll_low_high(npc: &Npc) -> (i32, i32) {
    (
        i32::from_ne_bytes(npc.fight.damage_range[0].to_ne_bytes()),
        i32::from_ne_bytes(npc.fight.damage_range[1].to_ne_bytes()),
    )
}

/// `SendMovePacket`'s heading byte, as `send_move_wait` builds it
/// (`G/char_state.cpp:2021-2022`): `(int)rotation / 5`, unsigned.
fn rotation_byte(rotation: f32) -> u8 {
    (f32_to_i32(rotation) / 5).to_le_bytes()[0]
}

/// What `ComputeSkillAtPosition`'s splash arms carry from the cast.
#[derive(Debug, Clone, Copy)]
struct SplashCast {
    /// The casting monster.
    mob: u32,
    /// The skill vnum.
    vnum: u32,
    /// The slot's skill level.
    level: u8,
    /// The splash point.
    at: (i32, i32),
    /// The row's splash range.
    range: u32,
    /// The row's flag word.
    flag: u32,
    /// The row's attr type.
    attr: u8,
}

impl GameState {
    /// Share the hosted mob skill rows boot loaded. Without them every cast refuses
    /// (as an unknown vnum does).
    #[must_use]
    pub fn with_mob_skills(mut self, skills: Arc<MobSkills>) -> Self {
        self.mob_skills = skills;
        self
    }

    /// Share the skill clips boot loaded. Without them every slot is clip-less (the
    /// empty-vector refusal).
    #[must_use]
    pub fn with_mob_clips(mut self, clips: Arc<MobSkillClips>) -> Self {
        self.mob_clips = clips;
        self
    }

    /// `HasMobSkill` (`G/char_skill.cpp:3322-3340`): any of the 5 slots names a vnum.
    fn has_mob_skill(&self, mob: u32) -> bool {
        self.npc_by_vid(mob)
            .is_some_and(|npc| npc.skills.iter().any(|slot| slot.vnum != 0))
    }

    /// `CanUseMobSkill` (`:3343-3356`): the slot is set, its cooldown has passed, and
    /// `number(0, 1)` is 0.
    fn can_use_mob_skill(&mut self, mob: u32, slot: usize) -> bool {
        let Some(npc) = self.npc_by_vid(mob) else {
            return false;
        };
        let set = npc.skills.get(slot).is_some_and(|slot| slot.vnum != 0);
        if !set {
            return false;
        }
        let now = self.clock.now();
        if self
            .npc_combat
            .get(&mob)
            .and_then(|combat| combat.skill_cool_ms.get(slot))
            .is_some_and(|cool| *cool > now)
        {
            return false;
        }
        number(&mut self.dice, 0, 1) == 0
    }

    /// `GetSkillPowerByLevelFromType` row 0 in `float` (`G/skill_power.cpp:32-37`), times
    /// the row's max level over 100: the `k` of `UseMobSkill` and the cast
    /// (`G/char_skill.cpp:3448`, `:2153`).
    fn skill_k(&self, vnum: u32, level: u8) -> f32 {
        let power = self.mob_skills.power(level);
        let max = self
            .mob_skills
            .get(vnum)
            .map_or(0, |skill| i32::from(skill.record.max_level));
        i32_to_f32(power) * i32_to_f32(max) / 100.0
    }

    /// `CalcMeleeDamage` of the monster `mob` on the player `victim`, ignoring the defence
    /// (`G/battle.cpp:489-600` with `bIgnoreDefense` true): the skill's per-victim `atk`
    /// (`G/char_skill.cpp:1432`). The shared calc draws the doubled damage roll and runs to
    /// the multiply; the skill skips the defence subtraction and the `<3` floor, which it
    /// applies later (the floor in `CalcBattleDamage`, the defence in the melee arm) —
    /// but the `MAX(0, iAtk - 0)` clamp lands first, so a negative `atk` becomes 0
    /// (`G/battle.cpp:596-600`, past the unhosted conqueror halve).
    fn skill_melee_attack(&mut self, mob: u32, victim: Vid, ignore_target_rating: bool) -> i32 {
        let Some(npc) = self.npc_by_vid(mob).cloned() else {
            return 0;
        };
        let Ok(character) = self.characters.find_by_vid(victim) else {
            return 0;
        };
        let Some(target) = character.points().cloned() else {
            return 0;
        };
        let berserk = self.minds.get(mob).is_some_and(|mind| mind.berserk);
        mob_calc_attack(
            &mut self.dice,
            &npc,
            Some(&target),
            berserk,
            ignore_target_rating,
        )
        .max(0)
    }

    /// The Battle skill scan (`G/char_state.cpp:1138-1170`): after berserk/godspeed, each
    /// slot in order rolls `CanUseMobSkill`; the first that passes faces the victim (no
    /// record) and casts. A cast sends `FUNC_MOB_SKILL` with the slot as `bArg`, holds the
    /// state for the clip's seconds × 25 (50 without a clip), and skips the swing. True
    /// when a slot cast.
    pub(super) fn battle_skill(&mut self, mob: u32, victim: Vid) -> bool {
        if !self.has_mob_skill(mob) {
            return false;
        }
        for slot in 0..5 {
            if !self.can_use_mob_skill(mob, slot) {
                continue;
            }
            let Some(target) = self.victim_state(victim) else {
                continue;
            };
            if let (Some(fighter), Some(mind)) = (self.npc_spot(mob), self.minds.get_mut(mob)) {
                mind.rotation = rotation_to_xy(
                    target.at.0.saturating_sub(fighter.0),
                    target.at.1.saturating_sub(fighter.1),
                );
            }
            if !self.use_mob_skill(mob, slot) {
                continue;
            }
            let now = self.clock.now();
            let spot = self.npc_spot(mob).unwrap_or((0, 0));
            let record = GcCharacterMove::new(
                FUNC_MOB_SKILL,
                u8::try_from(slot).unwrap_or(u8::MAX),
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
            let pulses = self
                .npc_by_vid(mob)
                .and_then(|npc| npc.skills.get(slot))
                .and_then(world::npc::MobSkillSlots::duration_s)
                .map_or(MISSING_CLIP_PULSES, |seconds| {
                    // `(DWORD)(fDuration * 25)`: the product in `float`, then the cast
                    // truncates. 3.6 gives 90 in `f32`, 89 in `f64`.
                    let pulses = f32_to_i32(seconds * 25.0).max(0);
                    u64::try_from(pulses).unwrap_or(u64::MAX)
                });
            self.set_duration(mob, pulses);
            return true;
        }
        false
    }

    /// `UseMobSkill` (`G/char_skill.cpp:3429-3525`): the cooldown stamps before the heal and
    /// the empty-vector arms (but an unknown vnum returns before the stamp); the 162 arm
    /// heals (the party test always fails in legacy, so it is always the self-heal); else
    /// each splash entry casts now or schedules its hit event at its vec-index key.
    fn use_mob_skill(&mut self, mob: u32, slot: usize) -> bool {
        let Some(npc) = self.npc_by_vid(mob) else {
            return false;
        };
        let Some(entry) = npc.skills.get(slot) else {
            return false;
        };
        if entry.vnum == 0 {
            return false;
        }
        let (vnum, level) = (entry.vnum, entry.level);
        let is_mob = is_monster(npc);
        let splash = entry.splash.clone();
        let Some(skill) = self.mob_skills.get(vnum) else {
            return false;
        };
        let cooldown_poly = skill.cooldown.clone();
        let flag = skill.record.flag;
        // The cooldown stamps before anything else (`:3450-3452`): `(int)(poly * 1000)` ms
        // in `f64` (the poly evaluates in `double`), truncated toward zero, then added to
        // the `DWORD` clock (the `u32` add wraps past its range).
        let k = f64::from(self.skill_k(vnum, level));
        let vars = BTreeMap::from([("k", k)]);
        let wait = cooldown_poly.eval(&vars) * 1000.0;
        let now = self.clock.now();
        if let Some(combat) = self.npc_combat.get_mut(&mob) {
            if let Some(cool) = combat.skill_cool_ms.get_mut(slot) {
                *cool = now.wrapping_add(u32::from_ne_bytes(f64_to_i32(wait).to_ne_bytes()));
            }
        }
        if is_mob && vnum == HEALING_SKILL_VNUM {
            // The party arm tests bit 27, which the decode never sets (bit 28): it is
            // coded and dead, as legacy's is.
            if flag & SKILL_FLAG_PARTY != 0 {
                let party = self.party_of(mob);
                for member in party {
                    self.heal_mob(member);
                }
            } else {
                self.heal_mob(mob);
            }
            return true;
        }
        if splash.is_empty() {
            return false;
        }
        let rotation = self.minds.get(mob).map_or(0.0, |mind| mind.rotation);
        let here = self.npc_spot(mob).unwrap_or((0, 0));
        for (key, entry) in splash.iter().enumerate() {
            let mut at = here;
            if entry.hit_distance != 0 {
                let reach = i32::try_from(entry.hit_distance).unwrap_or(i32::MAX);
                let (fx, fy) = delta_by_degree(rotation, i32_to_f32(reach));
                // `pos.x += (long)fx`: the C cast truncates toward zero, saturating at the
                // add (Rust cannot wrap like the `long` add of a 2^32 offset does — the
                // unhosted 278/280 clip — so that offset lands clamped, a Divergence).
                at = (
                    at.0.saturating_add(f32_to_i32(fx)),
                    at.1.saturating_add(f32_to_i32(fy)),
                );
            }
            let key = u32::try_from(key).unwrap_or(u32::MAX);
            if entry.timing_ms == 0 {
                self.compute_splash(mob, vnum, level, at);
                continue;
            }
            // A live event at the vec-index key is cancelled first (`<Factor>`).
            let old = self
                .npc_combat
                .get_mut(&mob)
                .and_then(|combat| combat.skill_events.remove(&key));
            if let Some(old) = old {
                let mut slot = Some(old.due);
                self.regens
                    .cancel(&mut slot, Due::MobSkill { vid: mob, key });
            }
            let when = self
                .last_pulse
                .saturating_add(u64::from(entry.timing_ms) * u64::from(PASSES_PER_SEC) / 1000);
            let mut slot = None;
            self.regens
                .schedule(&mut slot, when, Due::MobSkill { vid: mob, key });
            if let Some(combat) = self.npc_combat.get_mut(&mob) {
                combat.skill_events.insert(
                    key,
                    SkillEvent {
                        due: when,
                        vnum,
                        level,
                        at,
                    },
                );
            }
        }
        true
    }

    /// The 162 self-heal (`:3467-3474`): 15 % of the maximum (`maxHP / 100` first, integer),
    /// capped at the maximum, with `SE_EFFECT_HEALER` around it. No `PointChange`, no other
    /// record — and a corpse keeps its death with 15 % hit points (a legacy Defect, ported
    /// HP-only).
    fn heal_mob(&mut self, mob: u32) {
        let Some(npc) = self.npc_by_vid(mob) else {
            return;
        };
        let revive = npc.max_hp / 100 * 15;
        let hp = (npc.hp + revive).min(npc.max_hp);
        if let Some(npc) = self.npc_mut(mob) {
            npc.hp = hp;
        }
        let mut record = Vec::new();
        GcSpecialEffect {
            effect_type: SE_EFFECT_HEALER,
            vid: mob,
        }
        .encode_into(&mut record);
        self.packet_around(EntityKey::Npc(mob), &record, None);
    }

    /// `mob_skill_hit_event` (`G/char_skill.cpp:3389-3412`): a destroyed owner drops it;
    /// else the cast runs at the captured point, and the map key is erased.
    pub(super) fn mob_skill_event(&mut self, vid: u32, key: u32) {
        let Some(event) = self
            .npc_combat
            .get_mut(&vid)
            .and_then(|combat| combat.skill_events.remove(&key))
        else {
            return;
        };
        if self.npc_by_vid(vid).is_none() {
            return;
        }
        self.compute_splash(vid, event.vnum, event.level, event.at);
    }
    /// `ComputeSkillAtPosition`'s splash arm for a hosted cast (`:2115-2441`): the mount,
    /// poly, disable, unknown-vnum and non-`SPLASH` refusals are dead (a hosted cast passed
    /// them in `UseMobSkill`); level 0 fizzles (the demon/gorilla/dragon slot 0) before any
    /// draw; the pre-check gates `POINT_HP && iAmount < 0` (all hosted `ATTACK` rows pass;
    /// 266 takes the silent else); then the damage functor runs over the caster's view, and
    /// 263's second point runs the affect pass.
    fn compute_splash(&mut self, mob: u32, vnum: u32, level: u8, at: (i32, i32)) {
        if level == 0 {
            return;
        }
        let Some(skill) = self.mob_skills.get(vnum) else {
            return;
        };
        if skill.record.flag & SKILL_FLAG_SPLASH == 0 {
            return;
        }
        // The pre-check (`:2153-2234`): `k`, the caster's vars, `SetPolyVarForAttack`'s
        // `wep` roll, and the two amount evals. `OnMove(true)` for a non-NORMAL row stamps
        // the attack clock (which the battle timer reads next update); a monster holds no
        // revive invisibility, Eunhyung, mining or riding, so nothing else lands.
        let k = f64::from(self.skill_k(vnum, level));
        let mut vars = BTreeMap::from([("k", k)]);
        let Some(npc) = self.npc_by_vid(mob).cloned() else {
            return;
        };
        let flag = skill.record.flag;
        let attr = skill.record.skill_attr_type;
        let range = skill.record.splash_range;
        let on = point_on(&skill.record.point_on);
        let on2 = point_on(&skill.record.point_on2);
        let point = skill.point.clone();
        let point2 = skill.point2.clone();
        let duration2 = skill.duration2.clone();
        if on == point::POINT_MOV_SPEED {
            // Dead for hosted rows (no hosted first point is `MOV_SPEED`), but coded: the
            // pre-check sets `maxv` before the amounts eval.
            vars.insert("maxv", f64::from(npc.fight.point(point::POINT_MOV_SPEED)));
        }
        vars.insert("lv", f64::from(npc.fight.level));
        vars.insert("iq", f64::from(npc.fight.point(point::POINT_IQ)));
        vars.insert("str", f64::from(npc.fight.point(point::POINT_ST)));
        vars.insert("dex", f64::from(npc.fight.point(point::POINT_DX)));
        vars.insert("con", f64::from(npc.fight.point(point::POINT_HT)));
        vars.insert("maxhp", f64::from(npc.max_hp));
        // `GetMaxSP` of a monster is 0 (`ComputePoints`' else arm, `G/char.cpp:3016-3018`).
        vars.insert("maxsp", 0.0);
        vars.insert("chain", 0.0);
        vars.insert(
            "ar",
            f64::from(attack_rating(AttackRatingInput {
                attacker_dexterity: npc.fight.point(point::POINT_DX),
                attacker_level: i32::from(npc.fight.level),
                victim_dexterity: npc.fight.point(point::POINT_DX),
                ignore_target_rating: false,
            })),
        );
        vars.insert("def", f64::from(npc.fight.point(point::POINT_DEF_GRADE)));
        vars.insert(
            "odef",
            f64::from(
                npc.fight
                    .point(point::POINT_DEF_GRADE)
                    .wrapping_sub(npc.fight.point(point::POINT_DEF_GRADE_BONUS)),
            ),
        );
        vars.insert("horse_level", 0.0);
        let (low, high) = damage_roll_low_high(&npc);
        if flag & SKILL_FLAG_USE_MELEE_DAMAGE != 0 {
            // `CalcMeleeDamage(self, self, true, false)`: the shared calc with no victim
            // (the NPC arms) draws the roll, and `CalcBattleDamage` floors below 3.
            let berserk = self.minds.get(mob).is_some_and(|mind| mind.berserk);
            let atk = mob_calc_attack(&mut self.dice, &npc, None, berserk, false).max(0);
            vars.insert("atk", f64::from(atk));
        }
        // `USE_MAGIC_DAMAGE` and `USE_ARROW_DAMAGE` are dead for hosted rows (no row sets
        // them), so `atk` stays unset (0) without `USE_MELEE_DAMAGE`. A non-normal attr
        // calls `OnMove(true)` (`G/char.cpp:6148-6179`), but a monster holds no revive
        // invisibility, Eunhyung, mining or riding — only the attack and move stamps,
        // which a non-player has no fields for, so nothing lands.
        let _ = attr != SKILL_ATTR_NORMAL;
        let wep = number(&mut self.dice, low, high);
        vars.insert("wep", f64::from(wep));
        vars.insert("mwep", f64::from(wep));
        vars.insert("mtk", f64::from(wep));
        let amount = f64_to_i32(point.eval(&vars));
        let amount2 = f64_to_i32(point2.eval(&vars));
        if flag & (SKILL_FLAG_ATTACK | SKILL_FLAG_USE_MELEE_DAMAGE | SKILL_FLAG_USE_MAGIC_DAMAGE)
            == 0
        {
            // 266's silent else (`:2348-2440`): the empty duration poly evals 0, the
            // duration is not positive, and nothing lands. The two draws above still ran.
            return;
        }
        if !(on == point::POINT_HP && amount < 0) {
            // Unhosted: every hosted ATTACK row's poly evals negative here. The else runs
            // `FuncSplashAffect` when the duration poly evals positive — no hosted row
            // reaches it.
            return;
        }
        let cast = SplashCast {
            mob,
            vnum,
            level,
            at,
            range,
            flag,
            attr,
        };
        self.splash_damage(cast, &point);
        if on2 != point::POINT_NONE {
            let buffer = self
                .npc_by_vid(mob)
                .map_or(0, |npc| npc.fight.point(point::POINT_PARTY_BUFFER_BONUS));
            let duration = f64_to_i32(duration2.eval(&vars)).wrapping_add(buffer);
            if duration > 0 {
                self.splash_affect(cast, on2, amount2, duration);
            } else {
                // `iDur2 <= 0`: `PointChange(on2, amount2)` on the caster — dead for hosted
                // rows (263's duration is 3) and a no-op for `MOV_SPEED` on a monster
                // anyway, so nothing lands.
            }
        }
    }

    /// `FuncSplashDamage` (`G/char_skill.cpp:1360-2000`) for a hosted MELEE cast: every
    /// entity in the caster's view within `range` (`>` misses, so the rim hits) that
    /// `battle_is_attackable` passes takes the recompute — the ignore-defence and
    /// ignore-rating rolls, `ar` on the victim, `atk` ignoring the defence, the victim's
    /// limit speed (`maxv`, dead for hosted rows), `maxhp`/`maxsp`, a redrawn `wep` — the
    /// negate, the `<3` floor, the melee arm (`-DEF_GRADE`, unclamped, unless ignored),
    /// the victim's `BeginFight`, `Damage`, and the CRUSH slide of a hit that neither
    /// killed nor stunned. The cooldown and main-target arms are dead (an NPC's `m_pInfo`
    /// is NULL, so no `HitOnce` gate), and the damage functor carries no `MaxHit` cap.
    fn splash_damage(&mut self, cast: SplashCast, poly: &Poly) {
        if cast.attr != SKILL_ATTR_MELEE {
            return;
        }
        let Some(point2) = self
            .mob_skills
            .get(cast.vnum)
            .map(|skill| skill.point2.clone())
        else {
            return;
        };
        let Some(place) = self
            .npc_of
            .get(&cast.mob)
            .map(|&(channel, map, _)| (channel, map))
        else {
            return;
        };
        let Some(index) = self.maps.get(&place) else {
            return;
        };
        // `ForEachAround` collects the entities before it calls the functor on any. The
        // NPC walk stays coded: `splash_can_hit_npc` refuses every NPC victim (NPC on
        // NPC past guards), so the arm is dead — but the walk still tests each one.
        let around = index.around_of(EntityKey::Npc(cast.mob));
        let mut pcs = Vec::new();
        for key in around {
            match key {
                EntityKey::Character(raw) => {
                    if self.splash_can_hit_pc(cast.mob, Vid::new(raw), cast.at, cast.range) {
                        pcs.push(Vid::new(raw));
                    }
                }
                EntityKey::Npc(raw) => {
                    let _ = raw != cast.mob
                        && self.splash_can_hit_npc(cast.mob, raw, cast.at, cast.range);
                }
                EntityKey::Ground(_) => {}
            }
        }
        for victim in pcs {
            self.splash_hit_pc(cast, victim, poly, &point2);
        }
    }

    /// `battle_is_attackable` of the monster `mob` on the player `victim`, plus the
    /// splash's range test (`G/char_skill.cpp:1375-1391`).
    fn splash_can_hit_pc(&self, mob: u32, victim: Vid, at: (i32, i32), range: u32) -> bool {
        let Some(spot) = self.spot_of(victim.raw()) else {
            return false;
        };
        if self.place_of(victim.raw()).map(|(_, map)| map)
            != self.npc_of.get(&mob).map(|&(_, map, _)| map)
        {
            return false;
        }
        let range = i32::try_from(range).unwrap_or(i32::MAX);
        if distance_approx(spot.x.saturating_sub(at.0), spot.y.saturating_sub(at.1)) > range {
            return false;
        }
        // `battle_is_attackable` (`G/battle.cpp:99-152`): the victim is alive, neither cell
        // is BANPK, the caster is neither stunned nor dead, and `CanAttack` lets a
        // monster hit a player (no PC-vs-PC, observer, or mount arm applies).
        !self.is_dead(victim.raw())
            && !self
                .npc_by_vid(mob)
                .is_none_or(|caster| caster.stunned || caster.dead)
            && self.banpk_cells_clear(mob, victim.raw(), true)
    }

    /// `battle_is_attackable` of the monster `mob` on the non-player `victim`, plus the
    /// range test. `CanAttack` refuses NPC on NPC past guards (`G/pvp.cpp:391-392`, and
    /// no guard NPC is ported), so no hosted cast lands — but the walk still tests each
    /// one, and the refuse arms stay coded.
    fn splash_can_hit_npc(&self, mob: u32, victim: u32, at: (i32, i32), range: u32) -> bool {
        let Some(spot) = self.npc_spot(victim) else {
            return false;
        };
        if self.npc_of.get(&victim).map(|&(_, map, _)| map)
            != self.npc_of.get(&mob).map(|&(_, map, _)| map)
        {
            return false;
        }
        let range = i32::try_from(range).unwrap_or(i32::MAX);
        if distance_approx(spot.0.saturating_sub(at.0), spot.1.saturating_sub(at.1)) > range {
            return false;
        }
        false
    }

    /// The BANPK half of `battle_is_attackable`: neither the caster's nor the victim's
    /// cell may carry `ATTR_BANPK`. `pc` says which side `victim` is read from.
    fn banpk_cells_clear(&self, mob: u32, victim: u32, pc: bool) -> bool {
        let Some(&(_, map, _)) = self.npc_of.get(&mob) else {
            return false;
        };
        let Some(caster) = self.npc_by_vid(mob) else {
            return false;
        };
        let caster_at = (caster.x, caster.y);
        if self.is_banpk(map, caster_at.0, caster_at.1) {
            return false;
        }
        if pc {
            self.spot_of(victim)
                .is_some_and(|spot| !self.is_banpk(map, spot.x, spot.y))
        } else {
            self.npc_spot(victim)
                .is_some_and(|spot| !self.is_banpk(map, spot.0, spot.1))
        }
    }
    /// One player victim of the splash: the recompute (the ignore-defence and
    /// ignore-rating rolls off `point2`, `k`, `ar` on the victim, `atk` ignoring the
    /// defence, the victim's `maxhp`/`maxsp`, the chain index, `ek` 0, a redrawn `wep`),
    /// the negate, `CalcBattleDamage`, the melee arm through `Damage`'s MELEE path, the
    /// victim's `BeginFight`, and the CRUSH slide of a hit that neither killed nor
    /// stunned. A master-typed caster would take the master-bonus poly, but a monster
    /// holds no skill level, so `GetUsedSkillMasterType` is always normal.
    fn splash_hit_pc(&mut self, cast: SplashCast, victim: Vid, poly: &Poly, point2: &Poly) {
        let k = f64::from(self.skill_k(cast.vnum, cast.level));
        let mut vars = BTreeMap::from([("k", k)]);
        let Ok(target) = self.characters.find_by_vid(victim) else {
            return;
        };
        let Some(target_points) = target.points() else {
            return;
        };
        let health_cap = target_points.max_hp();
        let mana_cap = target_points.max_sp();
        let target_dx = target_points.get_point(point::POINT_DX);
        let Some(npc) = self.npc_by_vid(cast.mob).cloned() else {
            return;
        };
        let (low, high) = damage_roll_low_high(&npc);
        let chain = i32::from(self.chain_of(cast.mob));
        vars.insert("lv", f64::from(npc.fight.level));
        vars.insert("iq", f64::from(npc.fight.point(point::POINT_IQ)));
        vars.insert("str", f64::from(npc.fight.point(point::POINT_ST)));
        vars.insert("dex", f64::from(npc.fight.point(point::POINT_DX)));
        vars.insert("con", f64::from(npc.fight.point(point::POINT_HT)));
        vars.insert("def", f64::from(npc.fight.point(point::POINT_DEF_GRADE)));
        vars.insert(
            "odef",
            f64::from(
                npc.fight
                    .point(point::POINT_DEF_GRADE)
                    .wrapping_sub(npc.fight.point(point::POINT_DEF_GRADE_BONUS)),
            ),
        );
        vars.insert("horse_level", 0.0);
        let ignore_defence = if cast.flag & SKILL_FLAG_PENETRATE != 0 {
            let percent = f64_to_i32(point2.eval(&vars));
            number(&mut self.dice, 1, 100) <= percent
        } else {
            false
        };
        let ignore_rating = if cast.flag & SKILL_FLAG_IGNORE_TARGET_RATING != 0 {
            let percent = f64_to_i32(point2.eval(&vars));
            number(&mut self.dice, 1, 100) <= percent
        } else {
            false
        };
        vars.insert(
            "ar",
            f64::from(attack_rating(AttackRatingInput {
                attacker_dexterity: npc.fight.point(point::POINT_DX),
                attacker_level: i32::from(npc.fight.level),
                victim_dexterity: target_dx,
                ignore_target_rating: ignore_rating,
            })),
        );
        vars.insert("maxhp", f64::from(health_cap));
        vars.insert("maxsp", f64::from(mana_cap));
        vars.insert("chain", f64::from(chain));
        self.inc_chain(cast.mob);
        vars.insert("ek", 0.0);
        let atk = if cast.flag & SKILL_FLAG_USE_MELEE_DAMAGE != 0 {
            self.skill_melee_attack(cast.mob, victim, ignore_rating)
        } else {
            0
        };
        vars.insert("atk", f64::from(atk));
        let wep = number(&mut self.dice, low, high);
        vars.insert("wep", f64::from(wep));
        vars.insert("mwep", f64::from(wep));
        vars.insert("mtk", f64::from(wep));
        let amount = -f64_to_i32(poly.eval(&vars));
        // `CalcBattleDamage` (`G/battle.cpp:244-251`): below 3, `number(1, 5)`.
        let amount = if amount < 3 {
            number(&mut self.dice, 1, 5)
        } else {
            amount
        };
        drop(vars);
        let strike = self.melee_arm_pc(cast.mob, victim, amount, ignore_defence);
        self.show_skill_strike(cast.mob, victim, &strike);
        // `CanBeginFight → BeginFight` runs BEFORE `Damage` (`:1632-1633`), but the tree's
        // `damage_pc` already ran inside `show_skill_strike` — ordering the visible effects
        // as legacy does needs the pose before the damage records, so `show_skill_strike`
        // runs `begin_fight` first; this site keeps the legacy order in the code.
        let stunned = self.is_stunned(victim.raw());
        let dead = self.is_down(victim.raw());
        // `!Damage && !IsStun`: the CRUSH slide (`:1899-1961`) — a hit that neither killed
        // nor stunned, on a victim without `AIFLAG_NOMOVE` (a player has none).
        if !dead && !stunned && cast.flag & SKILL_FLAG_CRUSH != 0 {
            self.crush_slide(cast.mob, victim);
        }
        let _ = strike;
    }
    /// `GetChainLightningIndex`: 0 for a caster with no combat yet.
    fn chain_of(&self, mob: u32) -> u8 {
        self.npc_combat.get(&mob).map_or(0, |combat| combat.chain)
    }

    /// `IncChainLightningIndex`.
    fn inc_chain(&mut self, mob: u32) {
        if let Some(combat) = self.npc_combat.get_mut(&mob) {
            combat.chain = combat.chain.wrapping_add(1);
        }
    }
    /// The melee arm and `Damage`'s MELEE path on a player (`:1538-1572`,
    /// `G/char_battle.cpp:1858-1946`, `:2145-2156`): the skill's pre-mitigation `atk`
    /// minus the defence grade (unclamped, so a negative flows through; skipped when
    /// `ignore_defence` won the PENETRATE roll), the halved critical and penetrating
    /// chances, the skill bonuses, the down check, the death blow, and the `+0.5` of
    /// `damMul` 1.0. A dead victim refuses with a whiff: legacy would run the draws and
    /// then refuse in `Damage`, but the tree's `strike_pc` needs live points, so the
    /// refusal short-circuits before the dice (a Divergence in draw count only — the
    /// record stream is identical).
    fn melee_arm_pc(
        &mut self,
        mob: u32,
        victim: Vid,
        amount: i32,
        ignore_defence: bool,
    ) -> PcStrike {
        let Ok(character) = self.characters.find_by_vid(victim) else {
            return PcStrike {
                effects: Vec::new(),
                end: PcHitEnd::Blocked,
                affect: MobAttackAffect::default(),
            };
        };
        let Some(points) = character.points().cloned() else {
            return PcStrike {
                effects: Vec::new(),
                end: PcHitEnd::Blocked,
                affect: MobAttackAffect::default(),
            };
        };
        let Some(npc) = self.npc_by_vid(mob).cloned() else {
            return PcStrike {
                effects: Vec::new(),
                end: PcHitEnd::Blocked,
                affect: MobAttackAffect::default(),
            };
        };
        let berserk = self.minds.get(mob).is_some_and(|mind| mind.berserk);
        let affects = self.affect_flags_of(victim);
        let down = self.is_down(victim.raw());
        let striker = MobStriker { npc: &npc, berserk };
        let target = PcVictim {
            points: &points,
            affect_flags: affects,
            down,
            immune_flag: points.immune_flag(),
        };
        skill_damage_pc(&mut self.dice, striker.npc, &target, amount, ignore_defence)
    }

    /// What the skill's hit on a player showed: `BeginFight` first (legacy runs it before
    /// `Damage`), then each effect around the player, then the hit.
    fn show_skill_strike(&mut self, mob: u32, victim: Vid, strike: &PcStrike) {
        // `CanBeginFight → BeginFight` (`:1632-1633`): legacy sends no record for a
        // player's `BeginFight` (it sets the pose and the victim only).
        let can_begin = self.bodies.get(&victim).is_some_and(|body| !body.fighting)
            && !self.is_down(victim.raw());
        if can_begin {
            if let Some(body) = self.bodies.get_mut(&victim) {
                body.fighting = true;
            }
            self.leave_move(victim.raw());
        }
        let around = EntityKey::Character(victim.raw());
        for effect in &strike.effects {
            let mut record = Vec::new();
            GcSpecialEffect {
                effect_type: *effect,
                vid: victim.raw(),
            }
            .encode_into(&mut record);
            self.packet_around(around, &record, None);
        }
        let attacker = Attacker {
            vid: Vid::new(mob),
            pc: false,
        };
        match strike.end {
            PcHitEnd::Blocked => {}
            PcHitEnd::Down => {
                let _killed = self.damage_pc(Some(attacker), victim, 0, None);
            }
            PcHitEnd::Landed { damage, flag } => {
                let _killed = self.damage_pc(Some(attacker), victim, damage, Some(flag));
            }
        }
    }
    /// The CRUSH slide (`G/char_skill.cpp:1899-1961`): 400.0 (an NPC slides) from the
    /// caster through the victim, the `WALLS_FIX` loop stepping back 10 at a time off a
    /// `BLOCK`/`OBJECT` cell, then `Sync`, `Goto`, `CalculateMoveDuration` and `SyncPacket`.
    /// The PC-attacker main-target stun arm is dead (an NPC slides).
    fn crush_slide(&mut self, mob: u32, victim: Vid) {
        let Some(caster) = self.npc_spot(mob) else {
            return;
        };
        let Some(spot) = self.spot_of(victim.raw()) else {
            return;
        };
        let Some((_, map)) = self.place_of(victim.raw()) else {
            return;
        };
        let degree = rotation_to_xy(
            spot.x.saturating_sub(caster.0),
            spot.y.saturating_sub(caster.1),
        );
        let mut length = 400.0_f32;
        let (mut fx, mut fy) = delta_by_degree(degree, length);
        let (mut tx, mut ty) = (
            spot.x.saturating_add(f32_to_i32(fx)),
            spot.y.saturating_add(f32_to_i32(fy)),
        );
        while !self.is_movable(map, tx, ty) && length > 0.0 {
            if length >= 10.0 {
                length -= 10.0;
            } else {
                length = 0.0;
            }
            (fx, fy) = delta_by_degree(degree, length);
            tx = spot.x.saturating_add(f32_to_i32(fx));
            ty = spot.y.saturating_add(f32_to_i32(fy));
        }
        // `Sync(tx, ty)`: the victim turns toward the point and stands there.
        self.sync_body(victim.raw(), tx, ty);
        // `Goto(tx, ty)`: already there, so nothing moves — but `CalculateMoveDuration`
        // still runs with the slid point as start and dest.
        let now = self.clock.now();
        let move_speed = self.move_speed_of(victim.raw());
        let motion_speed = self.motion_speed_of(victim.raw());
        if let Some(body) = self.bodies.get_mut(&victim) {
            body.motion.start = (tx, ty);
            body.motion.dest = (tx, ty);
            body.motion.duration_ms = move_duration((tx, ty), (tx, ty), motion_speed, move_speed);
            body.motion.start_ms = now;
            body.motion.moving = false;
        }
        // `SyncPacket`: the one-element `GC_SYNC_POSITION` around the victim.
        let mut record = vec![HEADER_GC_SYNC_POSITION.value(), 0, 0];
        record.extend_from_slice(&GcSyncPositionElement::new(victim.raw(), tx, ty).encode());
        let size = u16::try_from(record.len()).unwrap_or(u16::MAX);
        record[1..3].copy_from_slice(&size.to_le_bytes());
        self.packet_around(EntityKey::Character(victim.raw()), &record, None);
    }

    /// `FuncSplashAffect` (`:2000-2060`) for 263's second point: every character in the
    /// caster's view strictly within `range` (`<`, so the rim misses — opposite of the
    /// damage functor's `>`) takes `AddAffect(263, MOV_SPEED, -70, flag 0, 3 s, 0,
    /// override)`. Uncapped (`maxHit` 0).
    fn splash_affect(&mut self, cast: SplashCast, on: usize, amount: i32, duration: i32) {
        let Some(place) = self
            .npc_of
            .get(&cast.mob)
            .map(|&(channel, map, _)| (channel, map))
        else {
            return;
        };
        let Some(index) = self.maps.get(&place) else {
            return;
        };
        let around = index.around_of(EntityKey::Npc(cast.mob));
        let mut victims = Vec::new();
        for key in around {
            let EntityKey::Character(raw) = key else {
                continue;
            };
            if self.place_of(raw).map(|(_, map)| map) != Some(place.1) {
                continue;
            }
            let Some(spot) = self.spot_of(raw) else {
                continue;
            };
            let range = i32::try_from(cast.range).unwrap_or(i32::MAX);
            if distance_approx(
                spot.x.saturating_sub(cast.at.0),
                spot.y.saturating_sub(cast.at.1),
            ) >= range
            {
                continue;
            }
            victims.push(Vid::new(raw));
        }
        for victim in victims {
            // `AddAffect(vnum, on, amount, flag 0, dur, 0, override)`.
            self.add_pc_affect(
                victim,
                Affect {
                    kind: cast.vnum,
                    apply_on: u8::try_from(on).unwrap_or(u8::MAX),
                    value: amount,
                    flag: 0,
                    remaining: duration,
                },
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use common::point_slot as point;
    use common::vid::Vid;
    use gamedata::mob_proto::{BATTLE_TYPE_MELEE, CHAR_TYPE_MONSTER};
    use std::sync::Arc;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::character::{number, Pcg32};
    use world::npc::{MobSkillSlots, MobSplash, FIRST_NPC_VID};

    use super::super::fixtures::{a_fighter, enter, open_cells, stand_field, TestClock, PLACE};
    use super::*;
    use crate::game_loop::PulseProcessor;

    /// The casting monster.
    const ME: u32 = FIRST_NPC_VID;
    /// Where it stands.
    const SPOT: (i32, i32) = (3200, 3200);
    /// 100 west: inside every hosted splash range (the cast's own CRUSH slide may move
    /// the player afterwards, so range probes must run before it).
    const NEAR: (i32, i32) = (3100, 3200);
    /// When the tests' first Pulse runs, in milliseconds.
    const T0: u32 = 1_000;
    /// The tests' first Pulse, which every due counts from.
    const P0: u64 = 1;

    fn dumps() -> Arc<MobSkills> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/sql/gamedata");
        let player = std::fs::read(root.join("player.sql")).unwrap();
        let common = std::fs::read(root.join("common.sql")).unwrap();
        Arc::new(MobSkills::from_dumps(&player, &common).unwrap())
    }

    fn slot(
        vnum: u32,
        level: u8,
        duration_s: Option<f32>,
        splash: Vec<MobSplash>,
    ) -> MobSkillSlots {
        MobSkillSlots {
            vnum,
            level,
            duration_bits: duration_s.map(f32::to_bits),
            splash,
        }
    }

    fn splash_now() -> Vec<MobSplash> {
        vec![MobSplash {
            timing_ms: 0,
            hit_distance: 0,
        }]
    }

    /// A monster with `slots` standing at [`SPOT`], and the player 1 [`NEAR`], after Pulse
    /// [`P0`] at [`T0`]; the inbox is the player's, drained.
    fn a_cast(
        slots: [MobSkillSlots; 5],
        hp: i32,
    ) -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(T0);
        let mut npc = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, hp);
        npc.char_type = CHAR_TYPE_MONSTER;
        npc.fight.damage_range = [10, 10];
        npc.fight.dam_multiply_bits = 1.0_f32.to_bits();
        npc.skills = slots;
        let world = stand_field(&clock, vec![npc], open_cells(), true).with_mob_skills(dumps());
        let mut world = world;
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        (clock, world, inbox)
    }

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    fn empty_slots() -> [MobSkillSlots; 5] {
        std::array::from_fn(|_| slot(0, 0, None, Vec::new()))
    }

    fn char_move_of(records: &[Vec<u8>]) -> Vec<GcCharacterMove> {
        records
            .iter()
            .filter_map(|record| {
                GcCharacterMove::decode(record)
                    .ok()
                    .filter(|_| record[0] == GcCharacterMove::header())
            })
            .collect()
    }

    /// The scan casts the first slot whose `number(0, 1)` draw is 0: seed 6 fails slot 0
    /// and passes slot 1, so the `FUNC_MOB_SKILL` carries `bArg` 1 (seed 6: 1, 0) — and the swing never
    /// runs (`G/char_state.cpp:1138-1170`).
    #[test]
    fn the_scan_casts_the_first_passing_slot_and_skips_the_swing() {
        let mut slots = empty_slots();
        slots[0] = slot(256, 20, Some(3.6), splash_now());
        slots[1] = slot(264, 20, Some(3.6), splash_now());
        let (_clock, mut world, mut inbox) = a_cast(slots, 10_000);
        world.dice = Pcg32::new(6, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        let moves = char_move_of(&drained(&mut inbox));
        assert_eq!(moves.len(), 1, "{moves:?}");
        assert_eq!(moves[0].b_func, FUNC_MOB_SKILL);
        assert_eq!(moves[0].b_arg, 1);
        assert_eq!(moves[0].dw_vid, ME);
        // 3.6 s in `f32` × 25 truncates to 90 (89 in `f64`).
        assert_eq!(world.minds.get(ME).map(|mind| mind.duration), Some(90));
        // Only slot 1's cooldown stamped.
        let combat = world.npc_combat.get(&ME).unwrap();
        assert_eq!(combat.skill_cool_ms[0], 0);
        assert_eq!(combat.skill_cool_ms[1], T0 + 12_000);
    }

    /// Both gates can refuse: slot 0 is unset, slot 1's cooldown is in the future, so the
    /// scan reaches slot 2; seed 4's first `number(0, 1)` is 1, so nothing casts and the
    /// swing runs instead.
    #[test]
    fn unset_and_cooling_slots_are_skipped_without_a_draw() {
        let mut slots = empty_slots();
        slots[0] = slot(0, 0, None, Vec::new());
        slots[1] = slot(256, 20, Some(3.6), splash_now());
        slots[2] = slot(256, 20, Some(3.6), splash_now());
        let (_clock, mut world, mut inbox) = a_cast(slots, 10_000);
        world.npc_combat.get_mut(&ME).unwrap().skill_cool_ms[1] = T0 + 1;
        world.dice = Pcg32::new(4, 0);
        assert!(!world.battle_skill(ME, Vid::new(1)));
        assert!(drained(&mut inbox).is_empty());
        // Only one draw ran (slot 2's refuse): the next `number(0, 1)` is seed 4's
        // second output, 1.
        assert_eq!(number(&mut world.dice, 0, 1), 1);
    }

    /// An unknown vnum (270) stamps no cooldown and keeps scanning: slot 1 casts with
    /// `bArg` 1 (`G/char_skill.cpp:3436-3444`).
    #[test]
    fn an_unknown_vnum_stamps_nothing_and_keeps_scanning() {
        let mut slots = empty_slots();
        slots[0] = slot(270, 20, Some(3.6), splash_now());
        slots[1] = slot(256, 20, Some(3.6), splash_now());
        let (_clock, mut world, mut inbox) = a_cast(slots, 10_000);
        world.dice = Pcg32::new(1, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        let moves = char_move_of(&drained(&mut inbox));
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].b_arg, 1);
        let combat = world.npc_combat.get(&ME).unwrap();
        assert_eq!(combat.skill_cool_ms[0], 0);
        assert_eq!(combat.skill_cool_ms[1], T0 + 4_000);
        // Both gate draws ran (slot 0's pass, slot 1's pass) before the splash draws.
    }

    /// A cast whose clip is missing holds the state 50 Pulses (`PASSES_PER_SEC(2)`).
    #[test]
    fn a_missing_clip_holds_the_state_fifty_pulses() {
        let mut slots = empty_slots();
        slots[0] = slot(256, 20, None, splash_now());
        let (_clock, mut world, _inbox) = a_cast(slots, 10_000);
        world.dice = Pcg32::new(0, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        assert_eq!(world.minds.get(ME).map(|mind| mind.duration), Some(50));
    }

    /// An empty splash vector stamps the cooldown, `sys_err`s, and refuses — the scan
    /// continues to the next slot (`:3487-3491`).
    #[test]
    fn an_empty_vector_stamps_and_refuses() {
        let mut slots = empty_slots();
        slots[0] = slot(256, 20, Some(3.6), Vec::new());
        slots[1] = slot(256, 20, Some(3.6), splash_now());
        let (_clock, mut world, mut inbox) = a_cast(slots, 10_000);
        world.dice = Pcg32::new(1, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        let moves = char_move_of(&drained(&mut inbox));
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].b_arg, 1);
        let combat = world.npc_combat.get(&ME).unwrap();
        assert_eq!(combat.skill_cool_ms[0], T0 + 4_000);
        assert_eq!(combat.skill_cool_ms[1], T0 + 4_000);
    }

    /// The 162 arm heals the monster 15 % (`maxHP / 100` first, integer) with no other
    /// record but the healer effect — and the row's bit 28 never passes the bit-27 party
    /// test, so it is always the self-heal (`:3454-3485`, plan §9).
    #[test]
    fn the_heal_takes_fifteen_percent_and_the_party_test_always_fails() {
        let mut slots = empty_slots();
        slots[0] = slot(265, 20, Some(1.0), splash_now());
        let (_clock, mut world, mut inbox) = a_cast(slots, 10_000);
        world.npc_mut(ME).unwrap().hp = 100;
        world.dice = Pcg32::new(0, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, 100 + 10_000 / 100 * 15);
        let combat = world.npc_combat.get(&ME).unwrap();
        assert_eq!(combat.skill_cool_ms[0], T0 + 5_000);
        let heard = drained(&mut inbox);
        assert_eq!(heard.len(), 2, "{heard:02x?}");
        let mut effect = Vec::new();
        GcSpecialEffect {
            effect_type: SE_EFFECT_HEALER,
            vid: ME,
        }
        .encode_into(&mut effect);
        assert!(heard.contains(&effect), "{heard:02x?}");
    }

    /// A `dwTiming != 0` entry schedules its hit event at vec-index key `t * 25 / 1000`
    /// Pulses out; a second cast cancels the live one first (`<Factor>`).
    #[test]
    fn splash_timing_schedules_and_a_second_cast_cancels_first() {
        let mut slots = empty_slots();
        slots[0] = slot(
            256,
            20,
            Some(3.6),
            vec![MobSplash {
                timing_ms: 521,
                hit_distance: 0,
            }],
        );
        let (_clock, mut world, _inbox) = a_cast(slots, 10_000);
        world.dice = Pcg32::new(0, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        let event = world
            .npc_combat
            .get(&ME)
            .unwrap()
            .skill_events
            .get(&0)
            .unwrap();
        assert_eq!(event.due, P0 + 521 * 25 / 1000);
        assert_eq!(event.vnum, 256);
        assert_eq!(event.at, SPOT);
        // A second cast at the same key cancels the first: the due moves with `last_pulse`.
        world.process_pulse(P0 + 1);
        world.dice = Pcg32::new(0, 0);
        world.npc_combat.get_mut(&ME).unwrap().skill_cool_ms[0] = 0;
        assert!(world.battle_skill(ME, Vid::new(1)));
        let event = world
            .npc_combat
            .get(&ME)
            .unwrap()
            .skill_events
            .get(&0)
            .unwrap();
        assert_eq!(event.due, P0 + 1 + 521 * 25 / 1000);
    }

    /// A level-0 slot spends its cooldown and fizzles at the cast (the demon/gorilla/
    /// dragon slot 0: no draw, no record, no damage).
    #[test]
    fn a_level_zero_slot_spends_its_cooldown_and_fizzles() {
        let mut slots = empty_slots();
        slots[0] = slot(258, 0, Some(3.6), splash_now());
        let (_clock, mut world, mut inbox) = a_cast(slots, 10_000);
        let hp = points_hp(&mut world, 1);
        world.dice = Pcg32::new(0, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        assert_eq!(
            world.npc_combat.get(&ME).unwrap().skill_cool_ms[0],
            T0 + 8_000
        );
        assert_eq!(points_hp(&mut world, 1), hp);
        // The move record only: the fizzle returns before the pre-check, so only the
        // gate drew. The next `number(0, 1)` is seed 0's second output, 1.
        assert_eq!(char_move_of(&drained(&mut inbox)).len(), 1);
        assert_eq!(number(&mut world.dice, 0, 1), 1, "seed 0's second draw");
    }

    /// 266's cast spends its pre-check draw and lands nothing: empty polys eval 0 (the
    /// stack under-read is a legacy Defect; the port reads 0). No `USE_MELEE_DAMAGE` arm
    /// runs, so there is no `CalcMeleeDamage` draw — only the `wep` roll.
    #[test]
    fn the_silent_row_spends_one_draw_and_lands_nothing() {
        let mut slots = empty_slots();
        slots[0] = slot(266, 20, Some(3.6), splash_now());
        let (_clock, mut world, mut inbox) = a_cast(slots, 10_000);
        let hp = points_hp(&mut world, 1);
        world.dice = Pcg32::new(0, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        assert_eq!(points_hp(&mut world, 1), hp);
        assert_eq!(char_move_of(&drained(&mut inbox)).len(), 1);
        // The gate and the `wep` roll: two draws.
        let mut dice = Pcg32::new(0, 0);
        for _ in 0..2 {
            let _ = dice.next_u32();
        }
        assert_eq!(world.dice.next_u32(), dice.next_u32());
    }

    /// 256's splash numbers at fixed dice: `atk` from the doubled roll, `-5*k*atk`
    /// negated, the floor, minus the defence grade — all in legacy's draw order.
    #[test]
    fn the_crush_splash_numbers_run_in_legacy_order() {
        let mut slots = empty_slots();
        slots[0] = slot(256, 20, Some(3.6), splash_now());
        let (_clock, mut world, _inbox) = a_cast(slots, 10_000);
        world.dice = Pcg32::new(0, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        // Seed 0: the gate draws 0 (pass). `mob_calc_attack` rolls `number(10, 10)` =
        // 10, doubled to 20; attack grade 0 + 20 − 2*level(0) = 20; the rating of dx 0
        // on dx 0 is (210/300) − ((5/95)*3/10) ≈ 0.684: 20*0.684 truncates to 13;
        // bonuses 0, `dam_multiply` 1.0 → 13; the floor keeps it; `wep` = 10 (unused by
        // `-5*k*atk`): pre-check amount −(5*0.5*13) = −32 < 0, so the functor runs.
        // Per victim: `atk` again (the victim's dx 3 moves the rating to ≈0.672 but 20
        // × 0.672 still truncates to 13), `wep` redrawn (10); amount −(−32) = 32 ≥ 3;
        // minus the warrior's DEF_GRADE (10 + 4*4/5 = 13) → 19; critical/penetrate
        // unhosted; the down check, the death blow (no draw: the fixture's
        // `death_blow_point` is 0), the `+0.5`. The 760-HP warrior takes 19.
        assert_eq!(points_hp(&mut world, 1), 760 - 19);
        // CRUSH slid it 400 west (no wall), and the view heard the move, the damage and
        // the slide's sync.
        assert_eq!(
            world.spot_of(1).map(|spot| (spot.x, spot.y)),
            Some((2700, 3200))
        );
    }

    fn points_hp(world: &mut GameState, vid: u32) -> i32 {
        world
            .characters
            .find_by_vid_mut(Vid::new(vid))
            .unwrap()
            .items_and_points_mut()
            .1
            .unwrap()
            .hp()
    }
    /// 264's poly reads the VICTIM's maximum HP: `-(maxhp*k)` negated is 760*0.5 = 380,
    /// minus 13 → 367 (seed 0's draws as in the crush test).
    #[test]
    fn the_thunder_poly_reads_the_victims_max_hp() {
        let mut slots = empty_slots();
        slots[0] = slot(264, 20, Some(3.6), splash_now());
        let (_clock, mut world, _inbox) = a_cast(slots, 10_000);
        world.dice = Pcg32::new(0, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        assert_eq!(points_hp(&mut world, 1), 760 - 367);
    }

    /// The damage rim hits (`>` misses): `DISTANCE_APPROX` reads 288 at true 300 (a hit)
    /// and 302 at true 315 (a miss).
    #[test]
    fn the_damage_rim_hits() {
        for (at, hit) in [((2900, 3200), true), ((2885, 3200), false)] {
            let mut slots = empty_slots();
            slots[0] = slot(256, 20, Some(3.6), splash_now());
            let clock = TestClock::default();
            clock.set(T0);
            let mut npc = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
            npc.fight.damage_range = [10, 10];
            npc.fight.dam_multiply_bits = 1.0_f32.to_bits();
            npc.skills = slots;
            let mut world =
                stand_field(&clock, vec![npc], open_cells(), true).with_mob_skills(dumps());
            let mut inbox = enter(&mut world, 1, at, 12_345);
            world.start_pc_recovery(Vid::new(1));
            world.process_pulse(P0);
            drained(&mut inbox);
            world.dice = Pcg32::new(0, 0);
            assert!(world.battle_skill(ME, Vid::new(1)));
            assert_eq!(points_hp(&mut world, 1) < 760, hit, "at {at:?}");
        }
    }

    /// 263's second point lands the slow: `MOV_SPEED` −70 for 3 s, kind 263, flag 0 — and
    /// the affect rim misses (`<`): on-rim takes no slow.
    #[test]
    fn the_slow_lands_with_kind_263_and_the_affect_rim_misses() {
        let mut slots = empty_slots();
        slots[0] = slot(263, 20, Some(3.6), splash_now());
        let (_clock, mut world, _inbox) = a_cast(slots, 10_000);
        let before = world.move_speed_of(1);
        world.dice = Pcg32::new(0, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        assert_eq!(world.move_speed_of(1), (before - 70).max(0));
        let event = world.affect_events.get(&Vid::new(1)).unwrap();
        let slow = event
            .affects
            .iter()
            .find(|affect| affect.kind == 263)
            .unwrap();
        assert_eq!(slow.apply_on, u8::try_from(point::POINT_MOV_SPEED).unwrap());
        assert_eq!(slow.flag, 0);
        assert_eq!(slow.remaining, 3);
    }

    /// A wall stops the CRUSH slide: a `BLOCK` band west of the victim holds it at the
    /// last movable point stepping back 10 at a time.
    #[test]
    fn a_wall_stops_the_crush_slide() {
        use super::super::fixtures::region;
        use gamedata::server_attr::{
            CellBlock, SectreeGrid, ServerAttr, ATTR_BLOCK, CELL_SIZE, SECTREE_CELLS,
        };
        use world::cells::{HostedCells, MapCells};
        let side = SECTREE_CELLS;
        let cells: Vec<u32> = (0..side * side)
            .map(|at| {
                let x = i32::try_from(at % side).unwrap() * CELL_SIZE;
                if (2600..2800).contains(&x) {
                    ATTR_BLOCK
                } else {
                    0
                }
            })
            .collect();
        let attr = ServerAttr::from_blocks(1, 1, vec![CellBlock::from_cells(&cells)]).unwrap();
        let grid = SectreeGrid::of(&region()).unwrap();
        let mut hosted = HostedCells::default();
        hosted.insert(PLACE.1, MapCells::new(grid, Arc::new(attr)));
        let cells = Arc::new(hosted);
        let mut slots = empty_slots();
        slots[0] = slot(256, 20, Some(3.6), splash_now());
        let clock = TestClock::default();
        clock.set(T0);
        let mut npc = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        npc.fight.damage_range = [10, 10];
        npc.fight.dam_multiply_bits = 1.0_f32.to_bits();
        npc.skills = slots;
        let mut world = stand_field(&clock, vec![npc], cells, true).with_mob_skills(dumps());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut inbox);
        world.dice = Pcg32::new(0, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        // 2700 is blocked: the loop steps back 2710, 2720... to the first movable point.
        let spot = world.spot_of(1).map(|spot| (spot.x, spot.y)).unwrap();
        assert!(spot.0 >= 2800, "stopped east of the band: {spot:?}");
        assert!(spot.0 < 3100, "moved west from {NEAR:?}: {spot:?}");
    }

    /// A destroyed owner drops its hit event: nothing lands when the Pulse comes.
    #[test]
    fn a_destroyed_owner_drops_its_hit_event() {
        let mut slots = empty_slots();
        slots[0] = slot(
            256,
            20,
            Some(3.6),
            vec![MobSplash {
                timing_ms: 521,
                hit_distance: 0,
            }],
        );
        let (_clock, mut world, _inbox) = a_cast(slots, 10_000);
        world.dice = Pcg32::new(0, 0);
        assert!(world.battle_skill(ME, Vid::new(1)));
        let due = world
            .npc_combat
            .get(&ME)
            .unwrap()
            .skill_events
            .get(&0)
            .unwrap()
            .due;
        world.mob_skill_event(ME + 999, 0);
        assert_eq!(points_hp(&mut world, 1), 760, "a gone owner casts nothing");
        // The live owner's event still lands at its due.
        world.process_pulse(due);
        assert!(points_hp(&mut world, 1) < 760, "the live event landed");
    }

    /// Slot 1's level-2 `k` is 0.06: `power(2)=6 × maxLevel/... ` — pin the `k` the cast
    /// uses for the demon/gorilla/dragon slot-1 shape (vnum 258, level 2).
    #[test]
    fn a_level_two_slot_casts_with_k_006() {
        let mut slots = empty_slots();
        slots[0] = slot(258, 2, Some(3.6), splash_now());
        let (_clock, world, _inbox) = a_cast(slots, 10_000);
        assert!((world.skill_k(258, 2) - 0.06).abs() < 1e-6);
    }
}
