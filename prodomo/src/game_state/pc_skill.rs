//! A player's skills: `UseSkill`'s entry gates, SP cost and cooldown stamp, and
//! the melee-skill damage arm on monsters (ledger 230d).
//!
//! A melee skill lands in two client steps (`G/input_main.cpp:1918-2009`,
//! `G/char_skill.cpp:2896-3135`). `CG_USE_SKILL` runs [`GameState::use_skill`],
//! which checks `CanUseSkill`, the stun gate (`CanMove` on the stun affect
//! only — no shop, riding, or polymorph is ported), the row, the level,
//! charges the SP (`kSPCostPoly` on `POINT_SP`; no owner PC row sets
//! `USE_HP_AS_COST`) and stamps the cooldown (`TSkillUseInfo::UseSkill`).
//! `CG_ATTACK` with `bType > 0` runs [`GameState::skill_attack`], which
//! checks `CanUseSkill` again, refuses the 14 shoot-list skills, checks
//! `CheckSkillHitCount` and the 1500 ms skill-term gate, then runs
//! `ComputeSkill`'s `FuncSplashDamage` arm on the monster victim.
//!
//! Clocks: legacy stamps wall ms (`get_dword_time()`). The Rewrite stamps
//! Pulses: the cooldown is `ComputeCooltime(cool*1000)` ms counted in Pulses
//! (rounding up, 40 ms a Pulse), and the 1500 ms term gate fires past 37
//! Pulses (`1500*25/1000 = 37.5`, so `delta > 37`). A Divergence under V17(f).
//!
//! Only the 230d melee rows cast through `skill_attack` (see
//! [`gamedata::pc_skill_data::PcSkill::is_castable_melee`]); the shoot path, the
//! SPLASH walk, the RANGE/MAGIC pipes, skill-up, the buffs and the grandmaster
//! wire wait.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use common::cfloat::{f64_to_f32, f64_to_i32};
use common::enums::EWearPositions;
use common::point_slot as point;
use common::vid::Vid;
use gamedata::mob_skill_data::{SKILL_FLAG_IGNORE_TARGET_RATING, SKILL_FLAG_PENETRATE};
use gamedata::pc_skill_data::SHOOT_LIST;
use world::character::{
    attack_rating, battle_damage_floor, calc_melee_skill_atk, damage_melee, number,
    skill_melee_attr, AttackRatingInput, Equipment, Points, Striker, Struck,
};
use world::npc::Npc;

use super::fight::{is_struck, AggroDamage, AFF_STUN_WORD};

use super::motion::{calculate_duration, distance_sqrt, rotation_to_xy};
use super::GameState;
use crate::sync_position::{SyncPositionVictim, SyncPositionVictimKind};

/// Wall ms per Pulse: the cooldown stamp's clock (`PASSES_PER_SEC(1)`).
const MS_PER_PULSE: u64 = 40;

/// The skill-term gate in Pulses: `1500ms * 25/1000 = 37.5`, and the `>` fires
/// past 37 (`G/char_battle.cpp:297-304`).
const SKILL_TERM_PULSES: u64 = 37;

/// `SKILL_FLAG_USE_HP_AS_COST` (`G/skill.h:14`): bit 5, matching the schema's
/// 6th `setFlag` member.
const SKILL_FLAG_USE_HP_AS_COST: u32 = 1 << 5;

/// `TSkillUseInfo` (`G/char.h:509-533`): one skill's cooldown stamp and hit
/// counts. Pulses, not wall ms (see the module docs).
#[derive(Debug, Clone, Default)]
pub(super) struct SkillUse {
    /// `bUsed`.
    used: bool,
    /// `dwNextSkillUsableTime`, in Pulses; `None` past a zero cooldown.
    next_usable: Option<u64>,
    /// `iHitCount`.
    hit_count: i32,
    /// `dwVID`: the main target's VID.
    main_vid: u32,
    /// `TargetVIDMap`: how many `CG_ATTACK`s of this skill each victim's VID
    /// has seen since the cast.
    hits_per_victim: HashMap<u32, u32>,
}

impl SkillUse {
    /// `TSkillUseInfo::UseSkill` (`G/char_skill.cpp:3229-3258`): false past an
    /// unexpired stamp (zeroing the hit count), else stamped with `cool`
    /// (`None` past a zero cooldown) and `hit_count`.
    fn stamp(&mut self, now: u64, main_vid: u32, cool: Option<u64>, hit_count: i32) -> bool {
        if self.used && self.next_usable.is_some_and(|due| due > now) {
            self.hit_count = 0;
            return false;
        }
        self.used = true;
        self.next_usable = cool.map(|pulses| now.saturating_add(pulses));
        self.hit_count = hit_count;
        self.main_vid = main_vid;
        true
    }
    /// `HitOnce` (`G/char_skill.cpp:3216-3227`): false unused, false past an
    /// expired stamp (the `MUYEONG`/`HORSE_WILDATTACK` exception waits — neither
    /// casts here), true on `-1`, else decremented while open.
    fn hit_once(&mut self, now: u64) -> bool {
        if !self.used {
            return false;
        }
        if self.next_usable.is_some_and(|due| due < now) {
            return false;
        }
        if self.hit_count == -1 {
            return true;
        }
        if self.hit_count > 0 {
            self.hit_count -= 1;
            return true;
        }
        false
    }
}

/// What a player keeps of its skills: its levels and its use stamps.
/// The fly targets, the Eunhyung level and the combo index wait (230e/230f/230h).
#[derive(Debug, Clone)]
pub(super) struct PcSkillState {
    /// `m_pSkillLevels[].bLevel` (`G/char.cpp:2275`): all 0 until a skill-up
    /// lands (the store blob, ledger 230f).
    pub(super) levels: [u8; 255],
    /// `m_SkillUseInfo`, by skill vnum.
    uses: HashMap<u32, SkillUse>,
    /// `m_dwLastSkillTime`, in Pulses.
    last_skill: Option<u64>,
}

impl Default for PcSkillState {
    fn default() -> Self {
        Self {
            levels: [0; 255],
            uses: HashMap::new(),
            last_skill: None,
        }
    }
}

impl GameState {
    /// Share the hosted PC skill rows boot loaded. Without them every cast refuses
    /// (as an unknown vnum does).
    #[must_use]
    pub fn with_pc_skills(mut self, skills: Arc<gamedata::pc_skill_data::PcSkills>) -> Self {
        self.pc_skill_tables = skills;
        self
    }

    /// The skill state of the player under `vid`, creating it.
    fn pc_skill_state(&mut self, vid: u32) -> &mut PcSkillState {
        self.pc_skills.entry(vid).or_default()
    }

    /// The skill level of the player under `vid` for `vnum`
    /// (`GetSkillLevel`, `G/char_skill.cpp:3198-3210`): 0 past vnum 254,
    /// clamped to 40.
    pub(super) fn pc_skill_level(&self, vid: u32, vnum: u32) -> u8 {
        if vnum >= 255 {
            return 0;
        }
        self.pc_skills
            .get(&vid)
            .map_or(0, |state| state.levels[vnum as usize].min(40))
    }

    /// Set a skill level in memory (tests only, until skill-up lands).
    #[cfg(test)]
    pub(super) fn set_pc_skill_level(&mut self, vid: u32, vnum: u32, level: u8) {
        if vnum < 255 {
            self.pc_skill_state(vid).levels[vnum as usize] = level.min(40);
        }
    }

    /// `k`: `1.0 * GetSkillPower(vnum, level) * bMaxLevel / 100`
    /// (`G/char_skill.cpp:2523`), as `f64`.
    fn pc_skill_k(&self, job: u8, group: u8, vnum: u32, level: u8) -> f64 {
        let power = self.pc_skill_tables.power(job, group, level, false);
        let max = self
            .pc_skill_tables
            .get(vnum)
            .map_or(0, |skill| skill.record.max_level);
        f64::from(power) * f64::from(max) / 100.0
    }

    /// `CInputMain::UseSkill` + `CHARACTER::UseSkill`'s entry half
    /// (`G/input_main.cpp:2173-2178`, `G/char_skill.cpp:2896-3112`): the
    /// `CanUseSkill`, stun, row and level gates, the revive-invisible removal,
    /// the SP charge, and the cooldown stamp. The TOGGLE-off, charge and combo
    /// arms wait (no 230d row sets TOGGLE; 5's row is refused at load).
    ///
    /// ATTACK rows stop here (their `CG_ATTACK` lands the damage); non-ATTACK
    /// rows return true with nothing further (their buffs wait). A refused
    /// cast sends nothing, as `CInputMain::UseSkill` ignores the answer.
    pub(super) fn use_skill(&mut self, vid: Vid, skill_vnum: u32, target_vid: u32) -> bool {
        let (job, group) = match self.bodies.get(&vid) {
            Some(body) => (body.card.job, body.card.skill_group),
            None => return false,
        };
        if !gamedata::pc_skill_data::can_use(job, group, false, skill_vnum) {
            return false;
        }
        // `g_bSkillDisable`, the observer mode, `CanMove`, the polymorph: only
        // the stun arm of `CanMove` is ported (no shop, riding, polymorph,
        // or observer mode).
        if self.affect_flags_of(vid)[0] & AFF_STUN_WORD != 0 {
            return false;
        }
        if self.pc_skill_tables.get(skill_vnum).is_none() {
            return false;
        }
        let level = self.pc_skill_level(vid.raw(), skill_vnum);
        if level == 0 {
            return false;
        }
        // The rod/pick refusal (`:2971-2973`): no mining is ported, so no
        // equipped rod or pick can refuse. The TOGGLE-off arm (`:3006-3009`)
        // waits: no 230d row sets TOGGLE.
        self.remove_revive_invisible(vid);
        self.cast_skill(vid, job, group, skill_vnum, target_vid, level)
    }

    /// `UseSkill`'s charge-and-stamp half (`:3021-3130`): the SP charge, the
    /// cooldown stamp, and the last-skill stamp. 64's pre-charge cooling refusal,
    /// the grandmaster cost and the dispatch tail wait (ATTACK rows only).
    fn cast_skill(
        &mut self,
        vid: Vid,
        job: u8,
        group: u8,
        skill_vnum: u32,
        target_vid: u32,
        level: u8,
    ) -> bool {
        let k = self.pc_skill_k(job, group, skill_vnum, level);
        let skill = self
            .pc_skill_tables
            .get(skill_vnum)
            .expect("the row was read above");
        let cool_secs = f64_to_i32(skill.cooldown.eval(&BTreeMap::from([("k", k)])));
        let hit_count = if skill.record.max_hit == 0 {
            -1
        } else {
            skill.record.max_hit
        };
        let hp_cost = skill.record.flag & SKILL_FLAG_USE_HP_AS_COST != 0;
        let sp_cost = skill.sp_cost.clone();
        // No owner PC row sets `USE_HP_AS_COST`, so the HP arm is refused
        // rather than charged (its rows never reach here — but the flag test
        // keeps the shape for them).
        if hp_cost {
            return false;
        }
        let casting = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, |points| points.get_point(point::POINT_CASTING_SPEED));
        let records = {
            let Some(points) = self.pc_points_mut(vid.raw()) else {
                return false;
            };
            let vars = BTreeMap::from([
                ("k", k),
                ("maxhp", f64::from(points.max_hp())),
                ("maxv", f64::from(points.max_sp())),
                ("v", f64::from(points.sp())),
            ]);
            let needed = f64_to_i32(sp_cost.eval(&vars));
            if points.sp() < needed {
                return false;
            }
            match points.point_change(point::POINT_SP, -needed, false, false) {
                Ok(records) => records.clone(),
                Err(_) => return false,
            }
        };
        self.send_point_records(vid, &records);
        let cool_millis = calculate_duration(casting, cool_secs.saturating_mul(1000));
        let cool = (cool_millis > 0).then(|| ms_to_pulses(cool_millis));
        let now = self.last_pulse;
        let stamped = self
            .pc_skill_state(vid.raw())
            .uses
            .entry(skill_vnum)
            .or_default()
            .stamp(now, target_vid, cool, hit_count);
        if !stamped {
            return false;
        }
        self.pc_skill_state(vid.raw()).last_skill = Some(now);
        // The dispatch tail (`:3119-3130`) waits: no 230d row computes here
        // (non-ATTACK rows return true above with nothing further).
        true
    }

    /// `CInputMain::Attack` with `bType > 0` on a non-player (`G/input_main.cpp:1928-2007`)
    /// and its `CHARACTER::Attack(bType)` (`G/char_battle.cpp:284-326`): `CanUseSkill`, the
    /// shoot-list refusal, `CheckSkillHitCount`, the skill-term gate, and the
    /// `ComputeSkill` damage arm on the 230d melee rows (`FuncSplashDamage` on the one
    /// victim, `DAMAGE_TYPE_MELEE`). A refused attack sends nothing, as
    /// `CInputMain::Attack` ignores `Attack`'s answer, and steals no gold.
    pub(super) fn skill_attack(&mut self, vid: Vid, skill_vnum: u8, target_vid: u32) -> bool {
        let (job, group) = match self.bodies.get(&vid) {
            Some(body) => (body.card.job, body.card.skill_group),
            None => return false,
        };
        let skill_vnum = u32::from(skill_vnum);
        if !gamedata::pc_skill_data::can_use(job, group, false, skill_vnum) {
            return false;
        }
        if SHOOT_LIST.contains(&skill_vnum) {
            return false;
        }
        let row = match self.pc_skill_tables.get(skill_vnum) {
            Some(row) if row.is_castable_melee() => row.clone(),
            _ => return false,
        };
        let Some(place) = self.place_of(vid.raw()) else {
            return false;
        };
        let Some(spot) = self.spot_of(vid.raw()) else {
            return false;
        };
        let npc_alive = self
            .npc_on(target_vid, place)
            .is_some_and(|npc| is_struck(npc) && !npc.dead);
        if !npc_alive {
            return false;
        }
        let npc_snapshot = self
            .npc_by_vid(target_vid)
            .expect("the victim stood above")
            .clone();
        // `battle_is_attackable` (`G/battle.cpp:165`): beyond it the attack is `BATTLE_NONE`
        // and sends nothing. There is no melee reach here: `ComputeSkill`'s target-range
        // arm (`+ 50`) below is the only distance gate.
        if !self.is_attackable(vid, (place.1, spot.x, spot.y), &npc_snapshot) {
            return false;
        }
        // `CheckSkillHitCount` (`G/char_skill.cpp:4184-4254`): an unused skill refuses;
        // YONGKWON 62/HWAYEOMPOK 77/DAEJINGAK 18/PAERYONG 93 refuse the attack record;
        // then each victim's count is capped (3 for SAMYEON=1/CHARYUN=33; the 230d rows
        // keep the default 1 — the YEONSA-7/HORSE_WILDATTACK_RANGE-5/HORSE_ESCAPE-10
        // arms have no hosted caster and stay out).
        if matches!(skill_vnum, 62 | 77 | 18 | 93) {
            return false;
        }
        let max_hits = u32::from(match skill_vnum {
            1 | 33 => 3_u8,
            _ => 1,
        });
        let seen = self
            .pc_skills
            .get(&vid.raw())
            .and_then(|state| state.uses.get(&skill_vnum))
            .filter(|info| info.used)
            .and_then(|info| info.hits_per_victim.get(&target_vid).copied())
            .unwrap_or(0);
        if seen >= max_hits {
            return false;
        }
        let count = self
            .pc_skill_state(vid.raw())
            .uses
            .entry(skill_vnum)
            .or_default();
        if !count.used {
            return false;
        }
        count
            .hits_per_victim
            .insert(target_vid, seen.saturating_add(1));
        // `Attack(bType)`'s skill-term gate (`G/char_battle.cpp:297-304`): past 1500 ms
        // after `m_dwLastSkillTime` the cast is refused. `use_skill` stamps the pulse,
        // so the `>` fires past 37 Pulses.
        let now = self.last_pulse;
        let term = self
            .pc_skills
            .get(&vid.raw())
            .and_then(|state| state.last_skill)
            .is_some_and(|last| now.saturating_sub(last) > SKILL_TERM_PULSES);
        if term {
            return false;
        }
        self.compute_skill_melee(vid, &row, target_vid);
        // The `ComputeSkill` `OnMove(true)` for a non-NORMAL attr
        // (`G/char_skill.cpp:2576-2577`): every 230d row is MELEE-attr.
        let now_ms = self.clock.now();
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.fighting = true;
            body.last_move = now_ms;
            body.last_attack = Some(now_ms);
        }
        // `SetPosition(POS_FIGHTING)` and `SetRotationToXY` (`G/char_battle.cpp:219-223`).
        let (nx, ny) = self
            .npc_by_vid(target_vid)
            .map(|npc| (npc.x, npc.y))
            .expect("the victim stood above");
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.rotation = rotation_to_xy(nx.saturating_sub(spot.x), ny.saturating_sub(spot.y));
        }
        true
    }

    /// `ComputeSkill`'s damage arm (`G/char_skill.cpp:2443-2732`) on the 230d rows: the
    /// riding and polymorph gates, the row, the horse-type arms, the SELFONLY victim
    /// (no 230d row sets it), the target-range arm (`DISTANCE_SQRT >= range + 50`
    /// refuses), the level, and the functor on the one victim — no SPLASH walk
    /// (no 230d row sets SPLASH). Then `Damage(pc, iDam, MELEE)`: the MELEE pipe, the
    /// damage map with MELEE aggro, the stun at 0, the kill of a stunned victim, and
    /// the victim's `BeginFight`.
    fn compute_skill_melee(
        &mut self,
        vid: Vid,
        row: &gamedata::pc_skill_data::PcSkill,
        target_vid: u32,
    ) {
        let spot = self.spot_of(vid.raw()).expect("the caster stood above");
        let Some((nx, ny)) = self.skill_range_spot(row, target_vid, spot) else {
            return;
        };
        let level = self.pc_skill_level(vid.raw(), row.record.vnum);
        if level == 0 {
            return;
        }
        // No riding and no polymorph is ported, so both gates pass; no hosted cast is a
        // horse row. `AFF_PABEOP` waits for the buffs.
        let Some(body) = self.bodies.get(&vid) else {
            return;
        };
        let (job, group) = (body.card.job, body.card.skill_group);
        let k = self.pc_skill_k(job, group, row.record.vnum, level);
        // The functor's per-victim draws (`G/char_skill.cpp:1412-1446`): PENETRATE's and
        // IGNORE_TARGET_RATING's `kPointPoly2` rolls, then `atk` and `ar`. No 230d row
        // sets IGNORE_TARGET_RATING; only vnum 61 sets PENETRATE, and its point2 poly
        // (`1+k*9`) reads a plain level-1 `k`, so it can draw.
        let point2_vars = BTreeMap::from([("k", k)]);
        let point2 = f64_to_i32(row.point2.eval(&point2_vars));
        let flag = row.record.flag;
        let ignore_defence =
            flag & SKILL_FLAG_PENETRATE != 0 && number(&mut self.dice, 1, 100) <= point2;
        let ignore_rating =
            flag & SKILL_FLAG_IGNORE_TARGET_RATING != 0 && number(&mut self.dice, 1, 100) <= point2;
        let points_snapshot = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned())
            .expect("the caster stood above");
        let victim_snapshot = self
            .npc_by_vid(target_vid)
            .expect("the victim stood above")
            .clone();
        let ar = attack_rating(AttackRatingInput {
            attacker_dexterity: points_snapshot.get_point(point::POINT_DX),
            attacker_level: i32::from(points_snapshot.level()),
            victim_dexterity: victim_snapshot.fight.point(point::POINT_DX),
            ignore_target_rating: ignore_rating,
        });
        let weapon = self.wielded_weapon(vid);
        let atk = calc_melee_skill_atk(
            &mut self.dice,
            &points_snapshot,
            weapon.as_ref(),
            &victim_snapshot,
        );
        let vars = skill_vars(&points_snapshot, &victim_snapshot, k, atk, ar);
        let mut amount = -f64_to_i32(row.point.eval(&vars));
        // No hosted cast rises past NORMAL (`SetSkillLevel` maps level < 20 there), so
        // the grandmaster `kMasterBonusPoly` arm never fires.
        amount = battle_damage_floor(&mut self.dice, amount);
        // The non-main splash adjust: a no-SPLASH row hits only the main victim
        // (`m_x`/`m_y` are the victim's spot), so no adjust multiplies.
        let damage = skill_melee_attr(
            amount,
            weapon.as_ref(),
            victim_snapshot.fight.point(point::POINT_DEF_GRADE),
            &victim_snapshot.fight,
            ignore_defence,
        );
        // `HitOnce` (`:1393-1400`): unused refuses (unreachable — the hit-count gate
        // above already demanded `used`), an expired stamp refuses, `-1` is
        // infinite, else the count falls by one.
        let now = self.last_pulse;
        let once = self
            .pc_skill_state(vid.raw())
            .uses
            .entry(row.record.vnum)
            .or_default()
            .hit_once(now);
        if !once {
            return;
        }
        let (struck, events) = self.skill_damage_melee(vid, target_vid, damage, weapon.as_ref());
        self.show_strike(vid, target_vid, events);
        match struck {
            Struck::Landed { damage } => {
                self.set_last_attacked(target_vid);
                self.after_damage_typed(vid, target_vid, damage, AggroDamage::Melee);
            }
            Struck::Finished => {
                self.set_last_attacked(target_vid);
                if let Some(body) = self.bodies.get_mut(&vid) {
                    body.fighting = false;
                }
                self.dead_npc(target_vid, Some(vid));
            }
            Struck::Whiffed | Struck::Refused | Struck::AlreadyDead | Struck::StunnedByGuard => {}
        }
        // `iRet != BATTLE_NONE`: `SetSyncOwner(this)`, then the victim's `BeginFight`.
        let victim_pos = SyncPositionVictim {
            vid: Vid::new(target_vid),
            kind: SyncPositionVictimKind::Monster,
            x: nx,
            y: ny,
        };
        let now_ms = self.clock.elapsed();
        let _owner = self.set_npc_sync_owner(vid, (spot.x, spot.y), &victim_pos, now_ms);
        if self
            .npc_by_vid(target_vid)
            .is_some_and(super::mob_battle::fights_back)
            && self.can_begin_fight(target_vid)
        {
            self.begin_fight(target_vid, vid);
        }
    }

    /// `ComputeSkill`'s target-range arm (`G/char_skill.cpp:2505-2517`): the
    /// victim's spot, or `None` past `dwTargetRange + 50` (`0` reaches everywhere).
    fn skill_range_spot(
        &self,
        row: &gamedata::pc_skill_data::PcSkill,
        target_vid: u32,
        spot: super::view::Spot,
    ) -> Option<(i32, i32)> {
        // A SELFONLY centre is the caster, not a non-player: the walk starts at the
        // caster's own spot (no target-range measure — the caster is 0 away).
        let (nx, ny) = match self.npc_by_vid(target_vid) {
            Some(npc) => (npc.x, npc.y),
            None => match self.spot_of(target_vid) {
                Some(at) => (at.x, at.y),
                None => return None,
            },
        };
        // `pkSk->dwTargetRange && DISTANCE_SQRT(...) >= range + 50`.
        let ok = row.record.target_range == 0
            || distance_sqrt(nx - spot.x, ny - spot.y)
                < f64_to_f32(f64::from(row.record.target_range)) + 50.0;
        ok.then_some((nx, ny))
    }

    /// `Damage(pc, iDam, MELEE)` on the monster under `target_vid`: the MELEE
    /// pipe, split from [`GameState::compute_skill_melee`] so the borrow of the
    /// caster's points ends before the hit is shown.
    fn skill_damage_melee(
        &mut self,
        vid: Vid,
        target_vid: u32,
        damage: i32,
        weapon: Option<&gamedata::item_proto::ItemProto>,
    ) -> (Struck, Vec<world::character::HitEvent>) {
        let empire = self
            .bodies
            .get(&vid)
            .expect("the caster stood above")
            .card
            .empire;
        let map = self.place_of(vid.raw()).expect("the caster stood above").1;
        let user_rate = self.user_damage_rate();
        let &(channel, nmap, at) = self
            .npc_of
            .get(&target_vid)
            .expect("the victim stood above");
        let character = self
            .characters
            .find_by_vid_mut(vid)
            .expect("the caster stood above");
        let points_mut = character
            .items_and_points_mut()
            .1
            .expect("the caster stood above");
        let mut striker = Striker {
            points: points_mut,
            weapon,
            empire,
            map,
            user_rate,
        };
        let npcs = self
            .npcs
            .get_mut(&(channel, nmap))
            .expect("the victim stood above");
        let npc = Arc::make_mut(npcs)
            .npcs
            .get_mut(at)
            .expect("the victim stood above");
        damage_melee(&mut self.dice, &mut striker, npc, damage)
    }

    /// The proto of what the player under `vid` wears in `WEAR_WEAPON`, cloned so the
    /// skill pipe can hold it across `&mut self` draws.
    fn wielded_weapon(&self, vid: Vid) -> Option<gamedata::item_proto::ItemProto> {
        let character = self.characters.find_by_vid(vid).ok()?;
        Equipment::of(character.items(), &self.protos)
            .wear(EWearPositions::Weapon as u16)
            .map(|worn| worn.proto.clone())
    }
}

fn ms_to_pulses(ms: i32) -> u64 {
    u64::try_from(ms).unwrap_or(0).div_ceil(MS_PER_PULSE)
}
/// The functor's variables for a no-SPLASH cast (`G/char_skill.cpp:1402-1446`,
/// `SetPolyVarForAttack`): `k`, the stat vars, `ar`, `atk`, the victim's maxima,
/// and `chain` 0 (no splash walk raises it). `def`/`odef` are the caster's; no
/// horse is ported, so `horse_level` is 0; `ek` is 0 (no Eunhyung affect is
/// ported). The weapon vars (`wep`/`mtk`/`mwep`) wait: no 230d point poly names
/// them, and an unset name reads 0 either way (see the `Poly` docs).
fn skill_vars(
    points: &Points,
    victim: &Npc,
    k: f64,
    atk: i32,
    ar: f32,
) -> BTreeMap<&'static str, f64> {
    BTreeMap::from([
        ("k", k),
        ("lv", f64::from(points.level())),
        ("iq", f64::from(points.get_point(point::POINT_IQ))),
        ("str", f64::from(points.get_point(point::POINT_ST))),
        ("dex", f64::from(points.get_point(point::POINT_DX))),
        ("con", f64::from(points.get_point(point::POINT_HT))),
        ("ar", f64::from(ar)),
        ("atk", f64::from(atk)),
        ("def", f64::from(points.get_point(point::POINT_DEF_GRADE))),
        (
            "odef",
            f64::from(
                points
                    .get_point(point::POINT_DEF_GRADE)
                    .wrapping_sub(points.get_point(point::POINT_DEF_GRADE_BONUS)),
            ),
        ),
        ("horse_level", 0.0),
        ("maxhp", f64::from(victim.max_hp)),
        // `GetMaxSP` of a monster is 0 (`ComputePoints`' else arm, `G/char.cpp:3016-3018`).
        ("maxsp", 0.0),
        ("chain", 0.0),
        ("ek", 0.0),
    ])
}
#[cfg(test)]
mod tests {
    use std::path::Path;

    use common::vid::Vid;
    use gamedata::mob_proto::BATTLE_TYPE_MELEE;
    use gamedata::pc_skill_data::PcSkills;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::npc::FIRST_NPC_VID;

    use super::super::fixtures::{a_fighter, enter, open_cells, stand_field, TestClock};
    use super::*;
    use crate::game_loop::PulseProcessor;

    /// The monster every test casts on.
    const ME: u32 = FIRST_NPC_VID;
    /// Where it stands.
    const SPOT: (i32, i32) = (3200, 3200);
    /// 100 west: inside every 230d target range arm (`+ 50`).
    const NEAR: (i32, i32) = (3100, 3200);
    /// When the tests' first Pulse runs, in milliseconds.
    const T0: u32 = 1_000;
    /// The tests' first Pulse, which every due counts from.
    const P0: u64 = 1;

    fn dumps() -> Arc<PcSkills> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/sql/gamedata");
        let player = std::fs::read(root.join("player.sql")).unwrap();
        let common = std::fs::read(root.join("common.sql")).unwrap();
        Arc::new(PcSkills::from_dumps(&player, &common).unwrap())
    }

    /// A dog at [`SPOT`] and the player 1 at [`NEAR`], after Pulse [`P0`] at [`T0`];
    /// the inbox is the player's, drained. The caster is a group-1 warrior, so vnum 1
    /// passes `CanUseSkill`.
    fn a_cast() -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 101, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true).with_pc_skills(dumps());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 1;
        world.process_pulse(P0);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        (clock, world, inbox)
    }

    fn sp_of(world: &GameState, vid: u32) -> i32 {
        world
            .characters
            .find_by_vid(Vid::new(vid))
            .unwrap()
            .points()
            .unwrap()
            .sp()
    }

    /// `CG_USE_SKILL` charges the SP (`40+100*k`, level-1 `k = 5/100`) and stamps the
    /// cooldown (`12 s → 300 Pulses`); a second cast inside the window is refused
    /// (the SP the second cast would charge stays).
    #[test]
    fn a_cast_charges_the_sp_and_stamps_the_cooldown() {
        let (_clock, mut world, mut inbox) = a_cast();
        world.set_pc_skill_level(1, 1, 1);
        let before = sp_of(&world, 1);
        assert!(world.use_skill(Vid::new(1), 1, ME));
        assert_eq!(sp_of(&world, 1), before - 45);
        let stamp = world.pc_skills.get(&1).unwrap().uses.get(&1).unwrap();
        assert!(stamp.used);
        assert_eq!(stamp.next_usable, Some(P0 + 300));
        assert_eq!(stamp.hit_count, 5);
        // The SP point change went out.
        assert!(!std::iter::from_fn(|| inbox.try_recv().ok())
            .collect::<Vec<_>>()
            .is_empty());
        // A second cast inside the 12 s window refuses — but the SP was already
        // charged before the stamp (`G/char_skill.cpp:3066-3112` charges, then
        // `UseSkill` stamps).
        assert!(!world.use_skill(Vid::new(1), 1, ME));
        assert_eq!(sp_of(&world, 1), before - 90);
    }

    /// An unlearned skill refuses: level 0 never reaches the SP charge.
    #[test]
    fn an_unlearned_skill_refuses() {
        let (_clock, mut world, mut inbox) = a_cast();
        let before = sp_of(&world, 1);
        assert!(!world.use_skill(Vid::new(1), 1, ME));
        assert_eq!(sp_of(&world, 1), before);
        assert!(inbox.try_recv().is_err());
    }

    /// A `CG_ATTACK` of a shoot-listed vnum is refused even after a cast stamps it:
    /// vnum 20 (`GEOMPUNG`) needs `CG_SHOOT`.
    #[test]
    fn a_shoot_listed_vnum_refuses_through_cg_attack() {
        let (_clock, mut world, mut inbox) = a_cast();
        // Vnum 20 sits in the warrior's group-2 list.
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.skill_group = 2;
        world.set_pc_skill_level(1, 20, 1);
        assert!(world.use_skill(Vid::new(1), 20, ME));
        let hp = world.npc_by_vid(ME).unwrap().hp;
        assert!(!world.skill_attack(Vid::new(1), 20, ME));
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, hp);
        std::iter::from_fn(|| inbox.try_recv().ok()).for_each(drop);
        assert!(inbox.try_recv().is_err());
    }

    /// A vnum-1 cast lands MELEE damage on the dog: the hit points fall, the damage
    /// map holds a MELEE row, gold is not stolen (NORMAL-only), and the dog begins
    /// the fight back.
    #[test]
    fn a_skill_hit_lands_melee_damage_and_the_dog_fights_back() {
        let (_clock, mut world, _inbox) = a_cast();
        world.set_pc_skill_level(1, 1, 1);
        world.dice = world::character::Pcg32::new(7, 0);
        let before = world.npc_by_vid(ME).unwrap().hp;
        assert!(world.use_skill(Vid::new(1), 1, ME));
        assert!(world.skill_attack(Vid::new(1), 1, ME));
        let after = world.npc_by_vid(ME).unwrap().hp;
        assert!(after < before, "the dog took damage: {before} -> {after}");
        let row = world.npc_combat.get(&ME).unwrap().damage.get(&1).unwrap();
        assert_eq!(row.total, before - after);
        assert!(row.aggro > 0);
        assert!(world.minds.get(ME).is_some());
    }

    /// Past 37 Pulses after the cast, the skill-term gate refuses the `CG_ATTACK`:
    /// the dog is whole.
    #[test]
    fn a_late_cg_attack_hits_the_skill_term_gate() {
        let (_clock, mut world, _inbox) = a_cast();
        world.set_pc_skill_level(1, 1, 1);
        assert!(world.use_skill(Vid::new(1), 1, ME));
        for pulse in P0 + 1..=P0 + 38 {
            world.process_pulse(pulse);
        }
        let hp = world.npc_by_vid(ME).unwrap().hp;
        assert!(!world.skill_attack(Vid::new(1), 1, ME));
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, hp);
    }
}
