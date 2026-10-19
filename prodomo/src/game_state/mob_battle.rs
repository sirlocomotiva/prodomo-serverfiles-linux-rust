//! A monster's Battle: its victim, the chase and the give-up (`StateBattle`,
//! `G/char_state.cpp:930-1183`; `CHARACTER::Follow`, `G/char.cpp:6848-7034`). The swing it ends
//! in is [`super::mob_strike`]. A guard's Battle on a monster is the same code with a
//! monster victim (`IsGuardNPC`, `G/char.cpp:7664-7667`).
//!
//! # What legacy does
//!
//! A monster begins to fight when a player hits it (`CanBeginFight` then `BeginFight`,
//! `G/char_battle.cpp:299-305`) or when its aggressive Idle finds a victim
//! (`FindVictim`, `G/char_state.cpp:668-686`). `BeginFight` stores the victim, takes
//! `POS_FIGHTING` (which asks for Battle) and asks for the next Pulse. Battle chases a victim
//! past its reach (`GotoNearTarget` then `Follow`), swings every 2 seconds once the attack timer
//! runs out (`Attack`, `G/char_battle.cpp:220-328`), drops one 4000 or more away, and searches
//! again after a dead one. A chase 15 seconds past its last hit and 5000 from where it was hit
//! gives up and returns there (`Return`).
//!
//! A guard searches its sectrees in Idle (`FuncFindGuardVictim`, `G/char_state.cpp:170-217`)
//! and fights a monster it finds: Battle drops a stunned or dead one and `Return()`s home,
//! chases one past the reach, and swings through the guard arm of `Damage`, which stuns
//! the monster instead of dealing hit points (`G/char_battle.cpp:2215-2224`).
//!
//! # Divergences
//!
//! - **RANGE and MAGIC fight (V19a, ported in 229s).** They begin, chase to eight tenths
//!   of the range, and shoot through [`super::mob_strike`]'s `FlyTarget` and `Shoot` arms.
//!   `SPECIAL` begins and chases but its `Attack` is `BATTLE_NONE`, as legacy's switch
//!   answers it.
//! - **The aggro steers a begun fight (V19b, ported in 229r).** `UpdateAggrPoint`,
//!   `UpdateAggrPointEx`, `ChangeVictimByAggro` and the chase's bleed live in
//!   [`super::fight`]; the party share lives in [`super::mob_party`], and only `Normal`
//!   reaches the multipliers. With no held victim there is no switch (ledger 229r.3).
//! - **A master's party fights with it (V19c).** The spawn group is read at Battle entry
//!   ([`super::mob_party`]); `SetLastAttacked` still stamps the monster itself, and the
//!   re-search has no party or leader arm.
//! - **The guards fight (V19d, ported in 229w).** A guard (`IsGuardNPC`) searches in Idle
//!   past the `noguard` event flag and fights a monster victim in Battle, with the
//!   stunned-loss arm, the `Return` on loss, and the swing's stun instead of hit points.
//!   The victim's invisibility affects are unread (no monster holds one), and the dead
//!   victim's `FindVictim` re-search never finds a monster (port's choice: it would
//!   arm a second victim slot no other path fills).
//! - **The mob skills (V19e, ledger 229u).** The Battle scan casts a hosted skill before
//!   the swing ([`super::mob_skill`]); no summoner's call or god speed is used, and races
//!   2191 and 2493 and the Meley map are not ported.
//! - **The swing waits 50 Pulses (V2).** No monster clip is loaded, so the clip's own wait is
//!   never read.
//! - **The search order (V19g).** `FindVictim` walks the sectree's `around` order with a strict
//!   nearer-than test, so the first one met wins a tie.
//! - **No [TS] lines and no chaser run (V19h).**
//! - **The chase moves at the walk (V19h).** Its `Goto` never sets the run.
//! - **The stun event runs first (V19k).** Legacy runs its events before the character updates,
//!   and the swing due on the same Pulse as a victim's stun event sees the dead victim.
//!
//! Everything here runs on the game thread only (ADR-0002).

use common::cfloat::{f32_to_i32, i32_to_f32, u32_to_f32};
use common::point_slot as point;
use common::vid::Vid;
use gamedata::mob_proto::{
    AIFLAG_BERSERK, AIFLAG_COWARD, AIFLAG_NOATTCHUNJO, AIFLAG_NOATTJINNO, AIFLAG_NOATTSHINSU,
    AIFLAG_NOMOVE, BATTLE_TYPE_MAGIC, BATTLE_TYPE_MELEE, BATTLE_TYPE_POWER, BATTLE_TYPE_RANGE,
    BATTLE_TYPE_SUPER_POWER, BATTLE_TYPE_SUPER_TANKER, BATTLE_TYPE_TANKER, CHAR_TYPE_MONSTER,
    CHAR_TYPE_NPC, MOB_RANK_BOSS,
};
use protocol::gc_actors::GcCharacterMove;
use world::character::number;
use world::npc::Npc;

use super::mob_ai::{delta_by_degree, pulses, Victim};
use super::motion::{calculate_duration, distance_sqrt, get_move_speed, rotation_to_xy};
use super::view::EntityKey;
use super::GameState;
use crate::movement::FUNC_ATTACK;
use crate::save::PASSES_PER_SEC;
use crate::sync_position::distance_approx;

/// `fDist >= 4000.0f`: the victim Battle drops (`G/char_state.cpp:1072-1098`).
const VICTIM_DROP: f32 = 4000.0;
/// `now - m_dwLastAttackedTime >= 15000`: a chase this old may give up (`:6921`).
const GIVE_UP_MS: u32 = 15_000;
/// `5000 < DISTANCE_APPROX(m_posLastAttacked - me)`: the give-up's distance (`:6922`).
const GIVE_UP_DISTANCE: i32 = 5000;
/// `IsChangeAttackPosition`'s wait within range (`G/char.cpp:7459-7471`).
const CHANGE_POSITION_NEAR_MS: u32 = 10_000;
/// `IsChangeAttackPosition`'s wait past 100 plus the range.
const CHANGE_POSITION_FAR_MS: u32 = 1000;
/// `DISTANCE_APPROX(me - target) > 100 + GetMobAttackRange()`: the far test.
const CHANGE_POSITION_DISTANCE: i32 = 100;
/// `retry = 16; while (--retry)`: 15 tries (`:6996`).
const CHANGE_POSITION_TRIES: usize = 15;
/// `fDist = number(400,1500)`: `KILL_AND_GO`'s distance (`:1004`).
const KILL_AND_GO: (i32, i32) = (400, 1500);
/// `CalculateDuration(GetLimitPoint(POINT_ATT_SPEED), 2000)`: the swing's wait (`:1119`).
const ATTACK_MS: i32 = 2000;
/// `PASSES_PER_SEC(2)`: the wait after a swing, with no clip.
const SWING_PULSES: u64 = 50;

/// The hold after a swing (`G/char_state.cpp:1179-1181`): `(DWORD)(fDuration *
/// 25)`, the product in `float`, then the cast truncates; a zero or missing
/// duration holds 50 (`PASSES_PER_SEC(2)`).
fn swing_hold(seconds: Option<f32>) -> u64 {
    seconds.map_or(SWING_PULSES, |seconds| {
        if seconds == 0.0 {
            return SWING_PULSES;
        }
        let pulses = f32_to_i32(seconds * 25.0).max(0);
        u64::try_from(pulses).unwrap_or(u64::MAX)
    })
}
/// `passes_per_sec / 2`: the wait after a failed `Attack` (`:1173`).
const FAILED_ATTACK_PULSES: u64 = 12;
/// `passes_per_sec / 2`: the wait when a guard's Idle finds a monster (`:532`). Shared
/// with the Idle arm, which sets it before `BeginFight` runs.
pub(super) const GUARD_IDLE_PULSES: u64 = 12;
/// `FuncFindGuardVictim(this, 50000)` (`G/char_state.cpp:524-529`): a guard searches this
/// far for a monster.
const GUARD_SIGHT: i32 = 50_000;
/// `AFF_REVIVE_INVISIBLE` (`G/affect.h:242`): bit 28, which `FindVictim` skips.
const AFF_REVIVE_INVISIBLE_WORD: u32 = 1 << 28;
/// The words `GetNearestVictim` skips (`G/char_battle.cpp:3394-3397`): `EUNHYUNG` (bit
/// 19), `INVISIBILITY` (bit 2), `REVIVE_INVISIBLE` (bit 28), all of word 0.
const NEAREST_SKIPPED_WORD: u32 = (1 << 19) | (1 << 2) | (1 << 28);

/// What Battle and Follow read of the monster, copied once per update. `attack_speed` is
/// `GetLimitPoint(POINT_ATT_SPEED)`, clamped to 0..250; `move_speed` is `GetMoveSpeed()` with
/// the 300 fall-back (V2).
#[derive(Clone, Copy, Debug)]
struct Fighter {
    map: i32,
    at: (i32, i32),
    ai_flags: u32,
    aggressive: bool,
    range: u16,
    sight: u16,
    attack_speed: i32,
    move_speed: f32,
    hp_pct: i32,
    berserk_point: u8,
    battle_type: u8,
}

/// What they read of the victim. `moving` is `body.motion.moving`, which leaving Move at the
/// first monster hit clears; `move_speed` is the player's `GetMoveSpeed()`. A monster
/// victim is read through [`GameState::npc_victim_state`], which yields `None` for a
/// stunned victim — `StateBattle` loses it the same way it loses a gone one
/// (`G/char_state.cpp:967`) — so no fourth flag is needed.
#[derive(Clone, Copy, Debug)]
pub(super) struct VictimState {
    pub(super) at: (i32, i32),
    dead: bool,
    banpk: bool,
    pub(super) moving: bool,
    rotation: f32,
    move_speed: f32,
}

/// Every battle type on a monster begins and chases (V19a, ported in 229s): the melee
/// family, RANGE and MAGIC, and SPECIAL. SPECIAL's swing blanks in [`super::mob_strike`].
pub(super) fn fights_back(npc: &world::npc::Npc) -> bool {
    npc.char_type == CHAR_TYPE_MONSTER
}

/// `KILL_AND_GO`'s switch (`:990-1022`): the five attack-melee types. `SPECIAL` begins and
/// chases through [`fights_back`] but its dead end goes Idle, as legacy's switch answers
/// no arm for it.
fn kill_goes(battle_type: u8) -> bool {
    matches!(
        battle_type,
        BATTLE_TYPE_MELEE
            | BATTLE_TYPE_POWER
            | BATTLE_TYPE_TANKER
            | BATTLE_TYPE_SUPER_POWER
            | BATTLE_TYPE_SUPER_TANKER
    )
}

/// `GetLimitPoint` of `POINT_ATT_SPEED` or `POINT_MOV_SPEED` for a non-player: 0..250
/// (`G/char.cpp:3745-3761`). Dormant: the 65 hosted protos above 250 are all STONE.
pub(super) fn npc_limit_speed(speed: i32) -> i32 {
    speed.clamp(0, 250)
}

/// `FindVictim`'s empire skip (`G/trigger.cpp:114-131`): the proto text's meaning (B30). Bit 3
/// spares empire 1, bit 4 (`NOATTCHUNJO`) empire 2 and bit 5 (`NOATTJINNO`) empire 3.
fn spares_empire(ai_flags: u32, empire: u8) -> bool {
    (ai_flags & AIFLAG_NOATTSHINSU != 0 && empire == 1)
        || (ai_flags & AIFLAG_NOATTCHUNJO != 0 && empire == 2)
        || (ai_flags & AIFLAG_NOATTJINNO != 0 && empire == 3)
}

/// The lead of Follow (`G/char.cpp:6936-6980`): where a moving target is met. Skipped for a
/// target that is not moving. Both speeds are `GetMoveSpeed()` floats; the cosine is `f64`
/// with pi; the doubles land in floats as C stores them. Module level: it reads no
/// `GameState`.
fn lead_point(fighter: Fighter, target: VictimState) -> (i32, i32) {
    if !target.moving {
        return target.at;
    }
    let heading = rotation_to_xy(
        target.at.0.saturating_sub(fighter.at.0),
        target.at.1.saturating_sub(fighter.at.1),
    );
    let mut delta = target.rotation - heading;
    while delta > 180.0 {
        delta -= 360.0;
    }
    while delta < -180.0 {
        delta += 360.0;
    }
    // `fFollowSpeed = mySpeed - yourSpeed * cos(rot_delta * M_PI / 180)`, a double in a float.
    let radians = f64::from(delta) * std::f64::consts::PI / 180.0;
    let follow_speed = common::cfloat::f64_to_f32(
        f64::from(fighter.move_speed) - f64::from(target.move_speed) * radians.cos(),
    );
    if follow_speed < 0.1 {
        return target.at;
    }
    let dist = distance_sqrt(
        target.at.0.saturating_sub(fighter.at.0),
        target.at.1.saturating_sub(fighter.at.1),
    );
    let meet_time = dist / follow_speed;
    let ahead = meet_time * target.move_speed;
    // `fMeetTime * yourSpeed <= 100000.0f`.
    let (mut x, mut y) = (target.at.0, target.at.1);
    if ahead <= 100_000.0 {
        let (fx, fy) = delta_by_degree(target.rotation, ahead);
        x = target.at.0.saturating_add(f32_to_i32(fx));
        y = target.at.1.saturating_add(f32_to_i32(fy));
    }
    // `fDist < fDistNew`: pulled back along the line to the monster (float maths).
    let dist_new = common::cfloat::f64_to_f32(
        ((f64::from(x.saturating_sub(fighter.at.0))).powi(2)
            + (f64::from(y.saturating_sub(fighter.at.1))).powi(2))
        .sqrt(),
    );
    if dist < dist_new && dist_new > 0.0 {
        let ratio = dist / dist_new;
        x = fighter.at.0.saturating_add(f32_to_i32(
            i32_to_f32(x.saturating_sub(fighter.at.0)) * ratio,
        ));
        y = fighter.at.1.saturating_add(f32_to_i32(
            i32_to_f32(y.saturating_sub(fighter.at.1)) * ratio,
        ));
    }
    (x, y)
}

impl GameState {
    /// `CanBeginFight` of the monster under `mob` (`G/char_battle.cpp:108-115`): neither
    /// stunned nor dead, no `AFF_STUN` (`CannotMoveByAffect`), and standing. A missing
    /// Mind is standing. A victim stunned by the very hit that asks does not begin.
    pub(super) fn can_begin_fight(&self, mob: u32) -> bool {
        let Some(npc) = self.npc_by_vid(mob) else {
            return false;
        };
        if npc.stunned || npc.dead {
            return false;
        }
        if self.npc_affect_words(mob)[0] & super::fight::AFF_STUN_WORD != 0 {
            return false;
        }
        self.minds.get(mob).is_none_or(|mind| !mind.fighting)
    }

    /// `BeginFight` of the monster under `mob` on `victim` (`:116-122`): the victim is held
    /// through `SetVictim` (which stamps the switch window's time), Battle is asked, and
    /// the next update is the next Pulse.
    pub(super) fn begin_fight(&mut self, mob: u32, victim: Vid) {
        let Some(npc) = self.npc_by_vid(mob) else {
            return;
        };
        let fresh = super::mob_ai::Mind::new(npc);
        let mind = self.minds.ensure(mob, fresh);
        mind.victim = Some(Victim::Here(victim));
        mind.fighting = true;
        mind.asked = Some(super::mob_ai::Fsm::Battle);
        mind.next_pulse = self.last_pulse.saturating_add(1);
        let now = self.clock.now();
        self.npc_combat.entry(mob).or_default().last_victim_set_ms = Some(now);
    }

    /// `BeginFight` of the guard under `mob` on the monster `victim`: the twin of
    /// [`GameState::begin_fight`] on the NPC slot.
    pub(super) fn begin_fight_npc(&mut self, mob: u32, victim: u32) {
        let Some(npc) = self.npc_by_vid(mob) else {
            return;
        };
        let fresh = super::mob_ai::Mind::new(npc);
        let mind = self.minds.ensure(mob, fresh);
        mind.victim_npc = Some(victim);
        mind.fighting = true;
        mind.asked = Some(super::mob_ai::Fsm::Battle);
        mind.next_pulse = self.last_pulse.saturating_add(1);
        let now = self.clock.now();
        self.npc_combat.entry(mob).or_default().last_victim_set_ms = Some(now);
    }

    /// `SetVictim` (`G/char_battle.cpp:3411-3428`): `None` stands a fighting monster down to
    /// Idle (`battle_end`), clearing both victim slots, and leaves the switch window's
    /// time; `Some` stores the victim and stamps it (`:3427`).
    pub(super) fn set_victim(&mut self, mob: u32, victim: Option<Vid>) {
        let Some(mind) = self.minds.get_mut(mob) else {
            return;
        };
        match victim {
            None => {
                mind.victim = None;
                mind.victim_npc = None;
                if mind.fighting {
                    mind.fighting = false;
                    mind.asked = Some(super::mob_ai::Fsm::Idle);
                }
            }
            Some(found) => {
                mind.victim = Some(Victim::Here(found));
            }
        }
        if victim.is_some() {
            let now = self.clock.now();
            self.npc_combat.entry(mob).or_default().last_victim_set_ms = Some(now);
        }
    }

    /// `GetVictim`: the held victim while it is online. `Gone`, no victim, or a body that is
    /// gone reads as `NULL`.
    pub(super) fn victim_of(&self, mob: u32) -> Option<Vid> {
        let Victim::Here(victim) = self.minds.get(mob)?.victim? else {
            return None;
        };
        self.bodies.contains_key(&victim).then_some(victim)
    }

    /// What Battle reads of `victim`, on its own Channel and map.
    pub(super) fn victim_state(&self, victim: Vid) -> Option<VictimState> {
        let body = self.bodies.get(&victim)?;
        let spot = self.spot_of(victim.raw())?;
        let (_, map) = self.place_of(victim.raw())?;
        Some(VictimState {
            at: (spot.x, spot.y),
            dead: self.is_dead(victim.raw()),
            banpk: self.is_banpk(map, spot.x, spot.y),
            moving: body.motion.moving,
            rotation: body.rotation,
            move_speed: get_move_speed(
                self.motion_speed_of(victim.raw()),
                self.move_speed_of(victim.raw()),
            ),
        })
    }

    /// `IsGuardNPC` of the NPC under `mob` (`G/char.cpp:7664-7667`).
    pub(super) fn is_guard(&self, mob: u32) -> bool {
        self.npc_by_vid(mob).is_some_and(|npc| {
            npc.char_type == CHAR_TYPE_NPC && super::mob_ai::GUARD_RACES.contains(&npc.race)
        })
    }

    /// What Battle reads of the monster under `victim`, on the guard's map. A stunned
    /// victim reads as gone (`None`): Battle drops it before engaging, as legacy's
    /// victim-loss arm drops a stunned victim of a guard (`G/char_state.cpp:967`).
    pub(super) fn npc_victim_state(&self, victim: u32) -> Option<VictimState> {
        let &(_, map, _) = self.npc_of.get(&victim)?;
        let npc = self.npc_by_vid(victim)?;
        if npc.stunned {
            return None;
        }
        let moving = self
            .minds
            .get(victim)
            .is_some_and(|mind| mind.state == super::mob_ai::Fsm::Move);
        Some(VictimState {
            at: (npc.x, npc.y),
            dead: npc.dead,
            banpk: self.is_banpk(map, npc.x, npc.y),
            moving,
            rotation: self.minds.get(victim).map_or(0.0, |mind| mind.rotation),
            move_speed: get_move_speed(
                super::motion::MOTION_SPEED_FALLBACK,
                npc_limit_speed(npc.fight.point(point::POINT_MOV_SPEED)),
            ),
        })
    }

    /// `FuncFindGuardVictim` of the guard under `mob` (`G/char_state.cpp:170-217`): the
    /// nearest monster around on its map within 50000, past PCs, non-monsters, the dead
    /// and race 5001. The EUNHYUNG, INVISIBILITY and REVIVE skips are vacuous: no monster
    /// holds an invisibility affect.
    pub(super) fn find_guard_victim(&self, mob: u32) -> Option<u32> {
        let fighter = self.fighter(mob)?;
        let index = self.maps.get(&(self.npc_of.get(&mob)?.0, fighter.map))?;
        let mut best: Option<(u32, i32)> = None;
        for key in index.around_of(EntityKey::Npc(mob)) {
            let EntityKey::Npc(raw) = key else {
                continue;
            };
            if raw == mob {
                continue;
            }
            if self.npc_of.get(&raw).map(|(_, map, _)| *map) != Some(fighter.map) {
                continue;
            }
            let Some(npc) = self.npc_by_vid(raw) else {
                continue;
            };
            if npc.char_type != CHAR_TYPE_MONSTER || npc.dead || npc.vnum == super::fight::GOLD_RACE
            {
                continue;
            }
            let distance = distance_approx(
                npc.x.saturating_sub(fighter.at.0),
                npc.y.saturating_sub(fighter.at.1),
            );
            let nearer = best.is_none_or(|(_, held)| distance < held);
            if nearer && distance <= GUARD_SIGHT {
                best = Some((raw, distance));
            }
        }
        best.map(|(victim, _)| victim)
    }

    /// What Battle reads of the monster under `mob`.
    fn fighter(&self, mob: u32) -> Option<Fighter> {
        let &(_, map, _) = self.npc_of.get(&mob)?;
        let npc = self.npc_by_vid(mob)?;
        let hp_pct = if npc.max_hp > 0 {
            npc.hp.saturating_mul(100) / npc.max_hp
        } else {
            0
        };
        Some(Fighter {
            map,
            at: (npc.x, npc.y),
            ai_flags: npc.ai_flags,
            aggressive: npc.is_aggressive(),
            range: world::character::mob_attack_range(npc),
            sight: npc.fight.aggressive_sight,
            attack_speed: npc_limit_speed(npc.fight.point(point::POINT_ATT_SPEED)),
            move_speed: get_move_speed(
                super::motion::MOTION_SPEED_FALLBACK,
                npc_limit_speed(npc.fight.point(point::POINT_MOV_SPEED)),
            ),
            hp_pct,
            berserk_point: npc.fight.berserk_point,
            battle_type: npc.fight.battle_type,
        })
    }

    /// `FindVictim` of the monster under `mob` within `sight` (`G/trigger.cpp:59-174`): the
    /// nearest live player around on its map, past the BANPK, affect and empire skips. Players
    /// have no EUNHYUNG, INVISIBILITY, TERROR or buildings (V19g).
    pub(super) fn find_victim(&self, mob: u32, sight: u16) -> Option<Vid> {
        let fighter = self.fighter(mob)?;
        let index = self.maps.get(&(self.npc_of.get(&mob)?.0, fighter.map))?;
        let mut best: Option<(Vid, i32)> = None;
        for key in index.around_of(EntityKey::Npc(mob)) {
            let EntityKey::Character(raw) = key else {
                continue;
            };
            let victim = Vid::new(raw);
            if self.place_of(raw).map(|(_, map)| map) != Some(fighter.map) {
                continue;
            }
            if self.is_dead(raw) {
                continue;
            }
            let Some(spot) = self.spot_of(raw) else {
                continue;
            };
            if self.is_banpk(fighter.map, spot.x, spot.y) {
                continue;
            }
            if self.affect_flags_of(victim)[0] & AFF_REVIVE_INVISIBLE_WORD != 0 {
                continue;
            }
            let empire = self.bodies.get(&victim)?.card.empire;
            if spares_empire(fighter.ai_flags, empire) {
                continue;
            }
            let distance = distance_approx(
                spot.x.saturating_sub(fighter.at.0),
                spot.y.saturating_sub(fighter.at.1),
            );
            let nearer = best.is_none_or(|(_, held)| distance < held);
            if nearer && distance <= i32::from(sight) {
                best = Some((victim, distance));
            }
        }
        best.map(|(victim, _)| victim)
    }

    /// `GetNearestVictim(stone)` (`G/char_battle.cpp:3373-3408`): over the stone's damage
    /// map in VID order, skipping offline and hidden attackers, the nearest by
    /// `DISTANCE_APPROX` to the stone; ties to the first met. No range cap.
    pub(super) fn nearest_victim(&self, stone: u32) -> Option<Vid> {
        let &(channel, map, _) = self.npc_of.get(&stone)?;
        let (x, y) = self.npc_spot(stone)?;
        let damage = self.npc_combat.get(&stone)?;
        let mut best: Option<(Vid, i32)> = None;
        // The map iterates in VID order (`std::map`); ties keep the first.
        for &raw in damage.damage.keys() {
            let attacker = Vid::new(raw);
            let Some(body) = self.bodies.get(&attacker) else {
                continue;
            };
            if (body.channel, body.map) != (channel, map) {
                continue;
            }
            if self.affect_flags_of(attacker)[0] & NEAREST_SKIPPED_WORD != 0 {
                continue;
            }
            let Some(spot) = self.spot_of(raw) else {
                continue;
            };
            let distance = distance_approx(spot.x.saturating_sub(x), spot.y.saturating_sub(y));
            if best.is_none_or(|(_, held)| distance < held) {
                best = Some((attacker, distance));
            }
        }
        best.map(|(victim, _)| victim)
    }

    /// `StateBattle` (`G/char_state.cpp:930-1183`) of the monster under `mob`, or the guard
    /// on its monster victim: a held NPC slot dispatches to [`GameState::state_battle_npc`].
    pub(super) fn state_battle(&mut self, mob: u32) {
        if self
            .minds
            .get(mob)
            .is_some_and(|mind| mind.victim_npc.is_some())
        {
            self.state_battle_npc(mob);
            return;
        }
        let Some(fighter) = self.fighter(mob) else {
            return;
        };
        // Stones and players never reach Battle; `!CanMove()` always holds and `IsStun()`
        // returns with the duration as it was. The instant arm stays first: both arms
        // return alike, so the order is unobservable (`:941-944`).
        if self
            .npc_by_vid(mob)
            .is_none_or(|npc| npc.char_type != CHAR_TYPE_MONSTER || npc.stunned)
        {
            return;
        }
        if self.npc_affect_words(mob)[0] & super::fight::AFF_STUN_WORD != 0 {
            return;
        }
        // Coward (`:949-965`): dead returns; else the victim is dropped, and one time in
        // 50 the coward runs.
        if fighter.ai_flags & AIFLAG_COWARD != 0 {
            if self.npc_by_vid(mob).is_some_and(|npc| npc.dead) {
                return;
            }
            self.set_victim(mob, None);
            if number(&mut self.dice, 1, 50) == 1 {
                self.coward_escape(mob, fighter.map);
            } else if let Some(mind) = self.minds.get_mut(mob) {
                mind.asked = Some(super::mob_ai::Fsm::Idle);
                mind.duration = 1;
            }
            return;
        }
        let state = self
            .victim_of(mob)
            .and_then(|victim| self.victim_state(victim).map(|state| (victim, state)));
        if self.battle_lost(mob, fighter, state) {
            return;
        }
        let Some((victim, target)) = state else {
            return;
        };
        if self.battle_engage(mob, fighter, victim, target) {
            return;
        }
        self.battle_swing(mob, fighter, victim, target);
    }

    /// `StateBattle` of the guard under `mob` on its monster victim: the head guards are
    /// dead (a guard is no stone, no player, `CanMove` holds, and the stun returns), the
    /// coward and summoner arms are empty (no guard proto carries the flag or a summon
    /// vnum), and the loss, engage and swing run on the NPC slot.
    fn state_battle_npc(&mut self, mob: u32) {
        let Some(fighter) = self.fighter(mob) else {
            return;
        };
        if self.npc_by_vid(mob).is_some_and(|npc| npc.stunned) {
            return;
        }
        let victim = self.minds.get(mob).and_then(|mind| mind.victim_npc);
        let state =
            victim.and_then(|victim| self.npc_victim_state(victim).map(|state| (victim, state)));
        if self.battle_lost_npc(mob, state) {
            return;
        }
        let Some((victim, target)) = state else {
            return;
        };
        if self.battle_engage_npc(mob, fighter, victim, target) {
            return;
        }
        self.battle_swing_npc(mob, fighter, victim, target);
    }

    /// Battle's victim loss on a monster victim (`:967-1036`): a stunned, dead, BANPK or
    /// gone victim is dropped, the guard `Return()`s home, and the duration is 1 second.
    /// No re-search (the research arm needs `IsDead` or BANPK *and* `IsAggressive`, and
    /// the guard protos are not aggressive) and no `KILL_AND_GO` (the guard is NOMOVE,
    /// and legacy's `Goto` would refuse).
    fn battle_lost_npc(&mut self, mob: u32, state: Option<(u32, VictimState)>) -> bool {
        let lost = match state {
            None => true,
            Some((_, target)) => target.dead || target.banpk,
        };
        if !lost {
            return false;
        }
        self.set_victim(mob, None);
        let _ = self.npc_return(mob);
        self.set_duration(mob, pulses(1));
        true
    }

    /// Battle's range arms on a monster victim (`:1067-1104`): a victim 4000 or more
    /// away is dropped with the duration as it was, with the protege follow; one past
    /// the reach is chased, which a NOMOVE guard refuses. True when either arm ran.
    fn battle_engage_npc(
        &mut self,
        mob: u32,
        fighter: Fighter,
        victim: u32,
        target: VictimState,
    ) -> bool {
        let dist = i32_to_f32(distance_approx(
            target.at.0.saturating_sub(fighter.at.0),
            target.at.1.saturating_sub(fighter.at.1),
        ));
        if dist >= VICTIM_DROP {
            self.set_victim(mob, None);
            // The protege follow (`:1090-1096`): past 1000 of its group's leader, a
            // dead-ended member walks back to within `number(150, 400)` of it.
            if let Some(leader) = self.protege(mob) {
                if let Some(spot) = self.npc_spot(leader) {
                    let away = distance_approx(
                        spot.0.saturating_sub(fighter.at.0),
                        spot.1.saturating_sub(fighter.at.1),
                    );
                    if away > 1000 {
                        let min = i32_to_f32(number(&mut self.dice, 150, 400));
                        let _ = self.follow_npc(mob, leader, min);
                    }
                }
            }
            return true;
        }
        let past_reach = f64::from(dist) >= f64::from(u32_to_f32(u32::from(fighter.range))) * 1.15;
        if past_reach {
            let _ = victim;
            // `__CHARACTER_GotoNearTarget` (`G/char_state.cpp:759-780`): NOMOVE refuses
            // before `Follow` runs, and the guard is always NOMOVE, so the chase ends
            // here and re-runs each update, as legacy's failed `Goto` does.
            debug_assert!(fighter.ai_flags & AIFLAG_NOMOVE != 0);
            return true;
        }
        false
    }

    /// Battle's swing on a monster victim (`:1106-1183`): `PM_ATTACKED_BY` first (a
    /// guard has no spawn group, so the send is vacuous), then the timer, then berserk
    /// on (a guard's berserk point is 0), then `Attack` on the monster.
    fn battle_swing_npc(&mut self, mob: u32, fighter: Fighter, victim: u32, target: VictimState) {
        self.party_attacked_by(mob);
        let now = self.clock.now();
        let wait = calculate_duration(fighter.attack_speed, ATTACK_MS);
        if let Some(last) = self
            .npc_combat
            .get(&mob)
            .and_then(|combat| combat.last_attack_ms)
        {
            let elapsed = now.wrapping_sub(last);
            let wait = u32::try_from(wait).unwrap_or(u32::MAX);
            if elapsed < wait {
                let duration =
                    (u64::from(PASSES_PER_SEC) * u64::from(wait - elapsed) / 1000).max(1);
                self.set_duration(mob, duration);
                return;
            }
        }
        // Berserk on (`:1125-1133`); god speed is dormant.
        if fighter.ai_flags & AIFLAG_BERSERK != 0
            && fighter.hp_pct < i32::from(fighter.berserk_point)
            && self.minds.get(mob).is_none_or(|mind| !mind.berserk)
        {
            if let Some(mind) = self.minds.get_mut(mob) {
                mind.berserk = true;
            }
        }
        if !self.guard_attack(mob, victim) {
            self.set_duration(mob, FAILED_ATTACK_PULSES);
            return;
        }
        if let Some(mind) = self.minds.get_mut(mob) {
            mind.rotation = rotation_to_xy(
                target.at.0.saturating_sub(fighter.at.0),
                target.at.1.saturating_sub(fighter.at.1),
            );
        }
        let record = GcCharacterMove::new(
            FUNC_ATTACK,
            0,
            rotation_byte(self.minds.get(mob).map_or(0.0, |mind| mind.rotation)),
            mob,
            fighter.at.0,
            fighter.at.1,
            now,
            0,
        )
        .encode();
        let me = EntityKey::Npc(mob);
        self.packet_around(me, &record, Some(me));
        let seconds = self.npc_by_vid(mob).and_then(Npc::attack_s);
        self.set_duration(mob, swing_hold(seconds));
    }

    /// Battle's victim loss (`:967-1036`): true when it handled one. A dead or BANPK victim
    /// of an aggressive wanderer is searched again; otherwise the victim is dropped for 25
    /// Pulses.
    fn battle_lost(
        &mut self,
        mob: u32,
        fighter: Fighter,
        state: Option<(Vid, VictimState)>,
    ) -> bool {
        let lost = match state {
            None => true,
            Some((_, target)) => target.dead || target.banpk,
        };
        if !lost {
            return false;
        }
        let research = matches!(state, Some((_, target)) if target.dead || target.banpk)
            && !self.minds.wanders_off()
            && fighter.aggressive;
        if research {
            let found = self.find_victim(mob, fighter.sight);
            let dead = state.map(|(_, target)| target).expect("a loss was found");
            self.set_victim(mob, found);
            self.set_duration(mob, pulses(1));
            if found.is_none()
                && self
                    .npc_by_vid(mob)
                    .is_some_and(|npc| kill_goes(npc.fight.battle_type))
            {
                self.kill_and_go(mob, fighter, dead);
            }
            return true;
        }
        self.set_victim(mob, None);
        self.set_duration(mob, pulses(1));
        true
    }

    /// `KILL_AND_GO` (`:1000-1022`, B29): a dead-ended wanderer walks off toward a point past
    /// its dead victim. The heading is measured from the monster (B29's fix).
    fn kill_and_go(&mut self, mob: u32, fighter: Fighter, old: VictimState) {
        let distance = i32_to_f32(number(&mut self.dice, KILL_AND_GO.0, KILL_AND_GO.1));
        let (fx, fy) = delta_by_degree(i32_to_f32(number(&mut self.dice, 0, 359)), distance);
        let (dx, dy) = (f32_to_i32(fx), f32_to_i32(fy));
        let point = (old.at.0.saturating_add(dx), old.at.1.saturating_add(dy));
        let half = (
            old.at.0.saturating_add(dx / 2),
            old.at.1.saturating_add(dy / 2),
        );
        if !self.is_movable(fighter.map, point.0, point.1)
            || !self.is_movable(fighter.map, half.0, half.1)
        {
            return;
        }
        if let Some(mind) = self.minds.get_mut(mob) {
            mind.rotation = rotation_to_xy(
                point.0.saturating_sub(fighter.at.0),
                point.1.saturating_sub(fighter.at.1),
            );
        }
        if self.npc_goto(mob, point.0, point.1) {
            self.send_move_wait(mob);
        }
    }

    /// Battle's range arms (`:1067-1104`): a victim 4000 or more away is dropped with the
    /// duration as it was; one past the reach is chased. True when either arm ran.
    fn battle_engage(
        &mut self,
        mob: u32,
        fighter: Fighter,
        victim: Vid,
        target: VictimState,
    ) -> bool {
        let dist = i32_to_f32(distance_approx(
            target.at.0.saturating_sub(fighter.at.0),
            target.at.1.saturating_sub(fighter.at.1),
        ));
        if dist >= VICTIM_DROP {
            self.set_victim(mob, None);
            // The protege follow (`:1090-1096`): past 1000 of its group's leader, a
            // dead-ended member walks back to within `number(150, 400)` of it.
            if let Some(leader) = self.protege(mob) {
                if let Some(spot) = self.npc_spot(leader) {
                    let away = distance_approx(
                        spot.0.saturating_sub(fighter.at.0),
                        spot.1.saturating_sub(fighter.at.1),
                    );
                    if away > 1000 {
                        let min = i32_to_f32(number(&mut self.dice, 150, 400));
                        let _ = self.follow_npc(mob, leader, min);
                    }
                }
            }
            return true;
        }
        let reach = f64::from(dist) >= f64::from(u32_to_f32(u32::from(fighter.range))) * 1.15;
        if reach {
            let _chased = self.goto_near_target(mob, victim);
            return true;
        }
        false
    }

    /// Battle's swing (`:1106-1183`): `PM_ATTACKED_BY` first, then the timer, then berserk
    /// on, then `Attack`.
    fn battle_swing(&mut self, mob: u32, fighter: Fighter, victim: Vid, target: VictimState) {
        // `PM_ATTACKED_BY` (`:1106-1107`): each member that can begins on the held victim.
        self.party_attacked_by(mob);
        let now = self.clock.now();
        let wait = calculate_duration(fighter.attack_speed, ATTACK_MS);
        if let Some(last) = self
            .npc_combat
            .get(&mob)
            .and_then(|combat| combat.last_attack_ms)
        {
            let elapsed = now.wrapping_sub(last);
            let wait = u32::try_from(wait).unwrap_or(u32::MAX);
            if elapsed < wait {
                let duration =
                    (u64::from(PASSES_PER_SEC) * u64::from(wait - elapsed) / 1000).max(1);
                self.set_duration(mob, duration);
                return;
            }
        }
        // Berserk on (`:1125-1133`); god speed is dormant.
        if fighter.ai_flags & AIFLAG_BERSERK != 0
            && fighter.hp_pct < i32::from(fighter.berserk_point)
            && self.minds.get(mob).is_none_or(|mind| !mind.berserk)
        {
            if let Some(mind) = self.minds.get_mut(mob) {
                mind.berserk = true;
            }
        }
        // The skill scan (`:1138-1170`): a cast holds the state and skips the swing.
        if self.battle_skill(mob, victim) {
            return;
        }
        if !self.mob_attack(mob, victim) {
            self.set_duration(mob, FAILED_ATTACK_PULSES);
            return;
        }
        if let Some(mind) = self.minds.get_mut(mob) {
            mind.rotation = rotation_to_xy(
                target.at.0.saturating_sub(fighter.at.0),
                target.at.1.saturating_sub(fighter.at.1),
            );
        }
        let record = GcCharacterMove::new(
            FUNC_ATTACK,
            0,
            rotation_byte(self.minds.get(mob).map_or(0.0, |mind| mind.rotation)),
            mob,
            fighter.at.0,
            fighter.at.1,
            now,
            0,
        )
        .encode();
        let me = EntityKey::Npc(mob);
        self.packet_around(me, &record, Some(me));
        let seconds = self.npc_by_vid(mob).and_then(Npc::attack_s);
        self.set_duration(mob, swing_hold(seconds));
    }

    /// `__CHARACTER_GotoNearTarget` (`G/char_state.cpp:759-780`): NOMOVE refuses; RANGE and
    /// MAGIC follow to eight tenths of the range, the rest to nine tenths. The integer
    /// `WORD*int/int`.
    pub(super) fn goto_near_target(&mut self, mob: u32, victim: Vid) -> bool {
        let Some(fighter) = self.fighter(mob) else {
            return false;
        };
        if fighter.ai_flags & AIFLAG_NOMOVE != 0 {
            return false;
        }
        let tenths = if fighter.battle_type == BATTLE_TYPE_RANGE
            || fighter.battle_type == BATTLE_TYPE_MAGIC
        {
            8
        } else {
            9
        };
        let min = f32::from(fighter.range.saturating_mul(tenths) / 10);
        self.follow(mob, victim, min)
    }

    /// `GetProtege` (`G/char_battle.cpp:3436-3445`): a stone's son answers its live stone;
    /// a group's member answers its leader. A dead or missing stone reads as none (D-new:
    /// legacy would read freed memory).
    pub(super) fn protege(&self, mob: u32) -> Option<u32> {
        if let Some(stone) = self.npc_by_vid(mob).and_then(|npc| npc.stone) {
            if self.npc_by_vid(stone).is_some_and(|stone| !stone.dead) {
                return Some(stone);
            }
            return None;
        }
        self.party_leader(mob)
    }

    /// `Follow` (`G/char.cpp:6882-7034`) of `target` to `min_distance`.
    fn follow(&mut self, mob: u32, target: Vid, min_distance: f32) -> bool {
        let Some(fighter) = self.fighter(mob) else {
            return false;
        };
        let Some(state) = self.victim_state(target) else {
            return false;
        };
        // The target's point is captured first (`:6911-6912`).
        if self.follow_gives_up(mob, fighter, state) {
            return true;
        }
        let (x, y) = lead_point(fighter, state);
        if let Some(mind) = self.minds.get_mut(mob) {
            mind.rotation = rotation_to_xy(
                x.saturating_sub(fighter.at.0),
                y.saturating_sub(fighter.at.1),
            );
        }
        let dist = distance_sqrt(
            x.saturating_sub(fighter.at.0),
            y.saturating_sub(fighter.at.1),
        );
        if dist <= min_distance {
            return false;
        }
        let boss = self
            .npc_by_vid(mob)
            .is_some_and(|npc| npc.fight.rank >= MOB_RANK_BOSS);
        if !boss && self.is_change_attack_position(mob, fighter, state.at) {
            let now = self.clock.now();
            if let Some(combat) = self.npc_combat.get_mut(&mob) {
                combat.last_change_position_ms = Some(now);
            }
            let fresh = self.change_attack_position(fighter, (x, y), dist, min_distance);
            if !self.npc_goto(mob, fresh.0, fresh.1) {
                return false;
            }
        } else {
            let (fx, fy) = delta_by_degree(
                self.minds.get(mob).map_or(0.0, |mind| mind.rotation),
                dist - min_distance,
            );
            let step = (
                fighter.at.0.saturating_add(f32_to_i32(fx)),
                fighter.at.1.saturating_add(f32_to_i32(fy)),
            );
            if !self.npc_goto(mob, step.0, step.1) {
                return false;
            }
        }
        self.send_move_wait(mob);
        true
    }

    /// Follow's give-up (`:6914-6927`): 15 seconds past the last hit and 5000 from where it
    /// was hit returns home. True when `Return` moved.
    fn follow_gives_up(&mut self, mob: u32, fighter: Fighter, target: VictimState) -> bool {
        let Some(combat) = self.npc_combat.get(&mob) else {
            return false;
        };
        let now = self.clock.now();
        if now.wrapping_sub(combat.last_attacked.ms) < GIVE_UP_MS {
            return false;
        }
        let from_home = distance_approx(
            combat.last_attacked.at.0.saturating_sub(fighter.at.0),
            combat.last_attacked.at.1.saturating_sub(fighter.at.1),
        );
        if from_home <= GIVE_UP_DISTANCE {
            return false;
        }
        let _ = target;
        self.npc_return(mob)
    }

    /// `IsChangeAttackPosition` (`G/char.cpp:7459-7471`): past the wait, 10000 ms near or
    /// 1000 ms when more than 100 plus the range away. `None` always passes.
    fn is_change_attack_position(&self, mob: u32, fighter: Fighter, at: (i32, i32)) -> bool {
        let wait = if distance_approx(
            at.0.saturating_sub(fighter.at.0),
            at.1.saturating_sub(fighter.at.1),
        ) > CHANGE_POSITION_DISTANCE + i32::from(fighter.range)
        {
            CHANGE_POSITION_FAR_MS
        } else {
            CHANGE_POSITION_NEAR_MS
        };
        self.npc_combat
            .get(&mob)
            .and_then(|combat| combat.last_change_position_ms)
            .is_none_or(|last| self.clock.now().wrapping_sub(last) > wait)
    }

    /// The change of attack position (`:6988-7019`): 15 tries around the aim point, near with
    /// two turn draws, far with one. A missing sectree or a cell with neither BLOCK nor
    /// OBJECT ends the tries; after 15 blocked tries the last point is kept.
    fn change_attack_position(
        &mut self,
        fighter: Fighter,
        aim: (i32, i32),
        distance: f32,
        min_distance: f32,
    ) -> (i32, i32) {
        let mut dest = aim;
        for _ in 0..CHANGE_POSITION_TRIES {
            let degree = if distance < 500.0 {
                let heading = f32_to_i32(rotation_to_xy(
                    fighter.at.0.saturating_sub(aim.0),
                    fighter.at.1.saturating_sub(aim.1),
                ));
                (heading + number(&mut self.dice, -90, 90) + number(&mut self.dice, -90, 90)) % 360
            } else {
                number(&mut self.dice, 0, 359)
            };
            let (fx, fy) = delta_by_degree(i32_to_f32(degree), min_distance);
            dest = (
                aim.0.saturating_add(f32_to_i32(fx)),
                aim.1.saturating_add(f32_to_i32(fy)),
            );
            if self.is_movable(fighter.map, dest.0, dest.1) {
                break;
            }
        }
        dest
    }

    /// `Return()` (`G/char.cpp:6848-6880`): the victim is dropped and the monster walks home
    /// to where it was last hit, then `PM_RETURN` scatters the party home.
    pub(super) fn npc_return(&mut self, mob: u32) -> bool {
        let Some(fighter) = self.fighter(mob) else {
            return false;
        };
        let home = self
            .npc_combat
            .get(&mob)
            .map_or(fighter.at, |combat| combat.last_attacked.at);
        self.set_victim(mob, None);
        if let Some(mind) = self.minds.get_mut(mob) {
            mind.rotation = rotation_to_xy(
                home.0.saturating_sub(fighter.at.0),
                home.1.saturating_sub(fighter.at.1),
            );
        }
        if !self.npc_goto(mob, home.0, home.1) {
            return false;
        }
        self.send_move_wait(mob);
        // Past the walk (`:6876-6878`): each member drops its victim and walks to a point
        // within 500 of the returner's home.
        self.party_return(mob, home);
        true
    }

    /// `SetLastAttacked` (`G/char_battle.cpp:1698-1718`): the hit point and time, on the
    /// monster itself (V19c).
    pub(super) fn set_last_attacked(&mut self, mob: u32) {
        let now = self.clock.now();
        let Some(spot) = self.npc_spot(mob) else {
            return;
        };
        let combat = self.npc_combat.entry(mob).or_default();
        combat.last_attacked.ms = now;
        combat.last_attacked.at = spot;
    }

    /// `ForgetMyAttacker` of `vid` (`G/char_battle.cpp:3624-3642`): every monster around
    /// whose victim is this player drops it, then the player hides for 5 seconds. A stunned
    /// or dead monster is skipped (B26). The dropped victim's damage row goes too: with no
    /// row the next hit re-inserts `(dam, 0)` instead of adding to a stale grudge.
    ///
    /// No live caller yet: the white flag 70008 waits for `USE_SPECIAL`, and `/forgetme` for
    /// the GM levels.
    #[cfg(test)]
    pub(super) fn forget_my_attacker(&mut self, vid: Vid) {
        let Some(place) = self.place_of(vid.raw()) else {
            return;
        };
        let Some(index) = self.maps.get(&place) else {
            return;
        };
        let calmed: Vec<u32> = index
            .around_of(EntityKey::Character(vid.raw()))
            .into_iter()
            .filter_map(|key| match key {
                EntityKey::Npc(raw) => Some(raw),
                EntityKey::Character(_) | EntityKey::Ground(_) => None,
            })
            .filter(|mob| {
                self.minds
                    .get(*mob)
                    .and_then(|mind| mind.victim)
                    .is_some_and(|victim| victim == Victim::Here(vid))
                    && self
                        .npc_by_vid(*mob)
                        .is_none_or(|npc| !npc.stunned && !npc.dead)
            })
            .collect();
        for mob in calmed {
            self.set_victim(mob, None);
            if let Some(combat) = self.npc_combat.get_mut(&mob) {
                combat.damage.remove(&vid.raw());
            }
        }
        self.revive_invisible(vid);
    }

    /// Every Mind whose victim is `vid` now holds `Gone`, and every damage row of `vid`
    /// goes: the player left (`leave_fight`). The scan skips no row whose owner is gone,
    /// so a stale grudge never wins a victim back at the next login.
    pub(super) fn lose_pc_victim(&mut self, vid: Vid) {
        for (_, mind) in self.minds.minds_mut() {
            if mind.victim == Some(Victim::Here(vid)) {
                mind.victim = Some(Victim::Gone);
            }
        }
        for combat in self.npc_combat.values_mut() {
            combat.damage.remove(&vid.raw());
        }
    }
}

/// `SendMovePacket`'s heading byte, as `send_move_wait` builds it.
fn rotation_byte(rotation: f32) -> u8 {
    (f32_to_i32(rotation) / 5).to_le_bytes()[0]
}

#[cfg(test)]
mod tests {
    use common::vid::Vid;
    use gamedata::mob_proto::{
        AIFLAG_AGGRESSIVE, AIFLAG_COWARD, AIFLAG_NOATTCHUNJO, AIFLAG_NOATTJINNO,
        AIFLAG_NOATTSHINSU, BATTLE_TYPE_MAGIC, BATTLE_TYPE_MELEE, BATTLE_TYPE_RANGE,
        BATTLE_TYPE_SPECIAL, MOB_RANK_BOSS,
    };
    use protocol::gc_actors::GcCharacterMove;
    use protocol::gc_inventory::{HEADER_GC_CHARACTER_MOVE, HEADER_GC_FLY_TARGETING};
    use protocol::gc_nested::GcFlyTargeting;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::npc::FIRST_NPC_VID;

    use super::super::fixtures::{
        a_fighter, enter, hit, open_cells, stand_field, TestClock, PLACE,
    };
    use super::super::mob_ai::{pulses, Fsm, Victim};
    use super::*;
    use crate::game_loop::PulseProcessor;
    use crate::movement::{FUNC_ATTACK, FUNC_WAIT};

    /// The monster every test fights.
    const ME: u32 = FIRST_NPC_VID;

    /// Where it stands.
    const SPOT: (i32, i32) = (3200, 3200);

    /// 100 west of it: within every reach used here.
    const NEAR: (i32, i32) = (3100, 3200);

    /// When the tests' first Pulse runs, in milliseconds.
    const T0: u32 = 1_000;

    /// The tests' first Pulse.
    const P0: u64 = 1;

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// `a_fighter` 406 (MELEE, range 175, sight 2000) at [`SPOT`] with `hp`, and the player 1
    /// at `at`, after Pulse [`P0`]; the inbox is the player's, drained.
    fn a_fight(
        npc_hp: i32,
        at: (i32, i32),
        no_wander: bool,
    ) -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, npc_hp);
        let mut world = stand_field(&clock, vec![npc], open_cells(), no_wander);
        let mut inbox = enter(&mut world, 1, at, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut inbox);
        (clock, world, inbox)
    }

    /// A hit monster begins to fight its attacker on the next Pulse (G1).
    #[test]
    fn a_hit_monster_begins_to_fight_its_attacker_on_the_next_pulse() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        hit(&mut world, 1, ME);
        let mind = world.minds.get(ME).expect("BeginFight made the Mind");
        assert_eq!(mind.victim, Some(Victim::Here(Vid::new(1))));
        assert!(mind.fighting);
        assert_eq!(mind.asked, Some(Fsm::Battle));
        assert_eq!(mind.next_pulse, P0 + 1);
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        let mind = world.minds.get(ME).expect("the Mind runs");
        assert_eq!(mind.state, Fsm::Battle);
        assert!(
            !drained(&mut alpha).is_empty(),
            "the first Battle update swings"
        );
    }

    /// A stunned monster never begins to fight (G2).
    #[test]
    fn a_stunned_monster_never_begins_to_fight() {
        let (_clock, mut world, _alpha) = a_fight(1, NEAR, true);
        hit(&mut world, 1, ME);
        assert!(world.npc_by_vid(ME).unwrap().stunned, "the one punch stuns");
        assert_eq!(world.minds.get(ME).and_then(|mind| mind.victim), None);
    }

    /// The flag twin of G2: `AFF_STUN` through `AddAffect` gates `BeginFight` too.
    #[test]
    fn a_flag_stunned_monster_never_begins_to_fight() {
        use crate::game_state::affect::Affect;
        use crate::loading_phase::{AFFECT_STUN, AFF_STUN, STUN_TICKS_NPC};
        let (_clock, mut world, _alpha) = a_fight(10_000, NEAR, true);
        world.add_npc_affect(
            ME,
            Affect {
                kind: AFFECT_STUN,
                apply_on: 0,
                value: 0,
                flag: AFF_STUN,
                remaining: STUN_TICKS_NPC,
            },
        );
        hit(&mut world, 1, ME);
        assert_eq!(world.minds.get(ME).and_then(|mind| mind.victim), None);
    }

    /// `AFF_STUN` in Battle returns with the duration kept (`:941-943`): no swing, no
    /// record, the victim held.
    #[test]
    fn a_flag_stunned_battle_returns_with_its_duration_kept() {
        use crate::game_state::affect::Affect;
        use crate::loading_phase::{AFFECT_STUN, AFF_STUN, STUN_TICKS_NPC};
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        hit(&mut world, 1, ME);
        world.process_pulse(P0 + 1);
        assert_eq!(
            world.minds.get(ME).map(|mind| mind.state),
            Some(Fsm::Battle)
        );
        world.add_npc_affect(
            ME,
            Affect {
                kind: AFFECT_STUN,
                apply_on: 0,
                value: 0,
                flag: AFF_STUN,
                remaining: STUN_TICKS_NPC,
            },
        );
        drained(&mut alpha);
        let duration = world.minds.get(ME).map_or(0, |mind| mind.duration);
        let next = world.minds.get(ME).map_or(0, |mind| mind.next_pulse);
        world.process_pulse(next);
        let mind = world.minds.get(ME).expect("the Mind runs");
        assert_eq!(mind.duration, duration, "the duration stays");
        assert_eq!(
            mind.victim,
            Some(Victim::Here(Vid::new(1))),
            "the victim stays"
        );
        assert_eq!(drained(&mut alpha), Vec::<Vec<u8>>::new(), "no swing");
    }

    /// An aggressive idle monster finds the nearest player and fights 25 Pulses later (G3).
    #[test]
    fn an_aggressive_idle_monster_finds_the_nearest_player_and_fights_25_pulses_later() {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(
            ME,
            115,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            AIFLAG_AGGRESSIVE,
            SPOT,
            10_000,
        );
        let mut world = stand_field(&clock, vec![npc], open_cells(), false);
        let mut one = enter(&mut world, 1, (SPOT.0 + 900, SPOT.1), 12_345);
        let mut two = enter(&mut world, 2, (SPOT.0 + 1200, SPOT.1), 12_345);
        world.process_pulse(P0);
        drained(&mut one);
        drained(&mut two);
        // The wake may still be pending at Pulse 1; run until Idle searches.
        let mut searched = world.last_pulse;
        for pulse in P0 + 1..=P0 + 3 {
            if world.victim_of(ME).is_some() {
                break;
            }
            searched = pulse;
            world.process_pulse(pulse);
        }
        assert_eq!(world.victim_of(ME), Some(Vid::new(1)));
        let mind = world.minds.get(ME).unwrap();
        assert_eq!(mind.next_pulse, searched + pulses(1));
        drained(&mut one);
        world.process_pulse(searched + pulses(1));
        assert_eq!(world.minds.get(ME).unwrap().state, Fsm::Battle);
    }

    /// `FindVictim` skips the dead, BANPK and revive-invisible, but not the stunned (G4).
    #[test]
    fn find_victim_skips_the_dead_banpk_and_revive_invisible_but_not_the_stunned() {
        let (_clock, mut world, _alpha) = a_fight(10_000, NEAR, true);
        // The stunned player 1 is still found.
        let stunned = world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(!stunned, "a stun is no death");
        assert!(world.is_down(1));
        assert_eq!(world.find_victim(ME, 2000), Some(Vid::new(1)));
        // A dead player is skipped.
        let died = world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(died);
        assert_eq!(world.find_victim(ME, 2000), None);
    }

    /// `no_wander` stops the search but not a hit monster's fight (G5).
    #[test]
    fn no_wander_stops_the_search_but_not_a_hit_monsters_fight() {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(
            ME,
            115,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            AIFLAG_AGGRESSIVE,
            SPOT,
            10_000,
        );
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut one = enter(&mut world, 1, NEAR, 12_345);
        for pulse in P0..=P0 + 30 {
            world.process_pulse(pulse);
        }
        drained(&mut one);
        assert_eq!(world.victim_of(ME), None, "no search under no_wander");
        hit(&mut world, 1, ME);
        assert_eq!(world.victim_of(ME), Some(Vid::new(1)));
    }

    /// A victim beyond the reach is chased with a wait move to nine tenths (G9).
    #[test]
    fn a_victim_beyond_the_reach_is_chased_with_a_wait_move_to_nine_tenths() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        hit(&mut world, 1, ME);
        assert!(world.move_to(1, SPOT.0 + 1000, SPOT.1));
        drained(&mut alpha);
        // The change of position draws its turns, and the world's dice is seeded per
        // process: pin it so the chase lands in the same place every run.
        world.dice = world::character::Pcg32::new(0, 0);
        world.process_pulse(P0 + 1);
        let records = drained(&mut alpha);
        let chase = records
            .iter()
            .find(|record| {
                record.first() == Some(&HEADER_GC_CHARACTER_MOVE.value())
                    && record.get(1) == Some(&FUNC_WAIT)
            })
            .expect("the chase sends FUNC_WAIT");
        let moved = GcCharacterMove::decode(chase).expect("a move");
        assert_eq!(moved.dw_vid, ME);
        // Nine tenths of 175 is 157 from the victim.
        let got = distance_approx(
            moved.l_x.saturating_sub(SPOT.0 + 1000),
            moved.l_y.saturating_sub(SPOT.1),
        );
        assert!(got <= 160, "the chase stops at nine tenths, not {got} away");
        assert_eq!(world.minds.get(ME).unwrap().duration, 4);
    }

    /// A victim 4000 away is dropped with no record (G12).
    #[test]
    fn a_victim_4000_away_is_dropped_with_no_record() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        hit(&mut world, 1, ME);
        let spot = world.spot_of(1).unwrap();
        assert!(world.move_to(1, spot.x + 4400, spot.y));
        drained(&mut alpha);
        let duration = world.minds.get(ME).unwrap().duration;
        world.process_pulse(P0 + 1);
        assert_eq!(world.victim_of(ME), None);
        assert_eq!(drained(&mut alpha), Vec::<Vec<u8>>::new());
        assert_eq!(world.minds.get(ME).unwrap().duration, duration);
    }

    /// A dead-ended follower past 1000 of its leader walks back: the victim 4000 away
    /// is dropped, and the follower's Battle follows its leader with a `FUNC_WAIT` to
    /// the view. A lone monster's drop stays silent.
    #[test]
    fn a_dead_ended_follower_past_1000_walks_back_to_its_leader() {
        let clock = TestClock::default();
        clock.set(T0);
        let leader = a_fighter(
            FIRST_NPC_VID,
            406,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            0,
            SPOT,
            10_000,
        );
        let mut follower = a_fighter(
            FIRST_NPC_VID + 1,
            406,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            0,
            (SPOT.0 + 1500, SPOT.1),
            10_000,
        );
        follower.leader = Some(FIRST_NPC_VID);
        let mut world = stand_field(&clock, vec![leader, follower], open_cells(), true);
        let mut alpha = enter(&mut world, 1, (SPOT.0 + 1500, SPOT.1 + 100), 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut alpha);
        hit(&mut world, 1, FIRST_NPC_VID + 1);
        let spot = world.spot_of(1).unwrap();
        assert!(world.move_to(1, spot.x, spot.y + 4400));
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        assert_eq!(
            world.victim_of(FIRST_NPC_VID + 1),
            None,
            "the victim is dropped"
        );
        let mind = world
            .minds
            .get(FIRST_NPC_VID + 1)
            .expect("the follower walks back");
        assert_eq!(mind.asked, Some(Fsm::Move));
        assert!(
            mind.motion.dest.0 < SPOT.0 + 1500,
            "the walk closes on the leader: {:?}",
            mind.motion.dest
        );
        let records = drained(&mut alpha);
        assert!(
            records.iter().any(|record| {
                record.first() == Some(&HEADER_GC_CHARACTER_MOVE.value())
                    && record.get(1) == Some(&FUNC_WAIT)
            }),
            "the walk-back tells the view"
        );
    }

    /// A dead victim sends an aggressive wanderer to `KILL_AND_GO` (G13).
    #[test]
    fn a_dead_victim_sends_an_aggressive_wanderer_to_kill_and_go() {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(
            ME,
            115,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            AIFLAG_AGGRESSIVE,
            SPOT,
            10_000,
        );
        let mut world = stand_field(&clock, vec![npc], open_cells(), false);
        let mut one = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut one);
        hit(&mut world, 1, ME);
        let _stunned = world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(world.damage_pc(None, Vid::new(1), i32::MAX, None));
        world.process_pulse(P0 + 1);
        drained(&mut one);
        assert!(
            world.victim_of(ME).is_none() || world.victim_of(ME) == Some(Vid::new(1)),
            "the dead victim is lost or re-found dead"
        );
    }

    /// A `SPECIAL` monster's dead end goes Idle: no `KILL_AND_GO` walk. It begins and
    /// chases through `fights_back`, but legacy's switch answers no arm for it.
    #[test]
    fn a_special_monsters_dead_end_goes_idle_without_a_walk() {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(
            ME,
            115,
            BATTLE_TYPE_SPECIAL,
            (175, 2000),
            AIFLAG_AGGRESSIVE,
            SPOT,
            10_000,
        );
        let mut world = stand_field(&clock, vec![npc], open_cells(), false);
        let mut one = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut one);
        hit(&mut world, 1, ME);
        assert_eq!(world.victim_of(ME), Some(Vid::new(1)), "it begins");
        let _stunned = world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(world.damage_pc(None, Vid::new(1), i32::MAX, None));
        drained(&mut one);
        // `begin_fight` waits 25 Pulses, so Battle handles the loss on pulse 26: the
        // MELEE control emits one walk there (probed), while `SPECIAL` stays silent.
        for pulse in P0 + 1..=P0 + 26 {
            world.process_pulse(pulse);
        }
        assert_eq!(world.victim_of(ME), None, "the dead victim is lost");
        assert_eq!(drained(&mut one), Vec::<Vec<u8>>::new(), "no walk");
    }

    /// A passive monster whose victim died goes Idle without a record (G14).
    #[test]
    fn a_passive_monster_whose_victim_died_goes_idle_without_a_record() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        hit(&mut world, 1, ME);
        let _stunned = world.damage_pc(None, Vid::new(1), i32::MAX, None);
        let _died = world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(world.is_dead(1));
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        assert_eq!(world.victim_of(ME), None);
        assert_eq!(drained(&mut alpha), Vec::<Vec<u8>>::new());
    }

    /// A victim who leaves is gone, and a returning VID is not attacked (G15).
    #[test]
    fn a_victim_who_leaves_is_gone_and_a_returning_vid_is_not_attacked() {
        let (clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        hit(&mut world, 1, ME);
        let _ = clock;
        world.leave_fight(
            Vid::new(1),
            crate::game_loop_messages::Departure::Disconnect,
        );
        assert_eq!(world.minds.get(ME).unwrap().victim, Some(Victim::Gone));
        assert!(
            !world.npc_combat[&ME].damage.contains_key(&1),
            "the leaver's row goes too"
        );
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        assert_eq!(world.victim_of(ME), None);
        assert_eq!(drained(&mut alpha), Vec::<Vec<u8>>::new());
        world.process_pulse(P0 + 26);
        assert_eq!(world.minds.get(ME).unwrap().state, Fsm::Idle);
    }

    /// A player's hit moves the monster's last-attacked point and time (G17).
    #[test]
    fn a_players_hit_moves_the_monsters_last_attacked_point_and_time() {
        let (clock, mut world, _alpha) = a_fight(10_000, NEAR, true);
        clock.set(T0 + 5_000);
        hit(&mut world, 1, ME);
        let combat = world.npc_combat.get(&ME).unwrap();
        assert_eq!(combat.last_attacked.ms, T0 + 5_000);
        assert_eq!(combat.last_attacked.at, SPOT);
    }

    /// Arriving with a victim enters Battle after one Pulse (G19).
    #[test]
    fn arriving_with_a_victim_enters_battle_after_one_pulse() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        hit(&mut world, 1, ME);
        assert!(world.move_to(1, SPOT.0 + 1000, SPOT.1));
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        assert_eq!(world.minds.get(ME).unwrap().state, Fsm::Battle);
        drained(&mut alpha);
        // The chase asked Move; step until it arrives.
        for pulse in P0 + 2..=P0 + 40 {
            world.process_pulse(pulse);
            if world.minds.get(ME).unwrap().state == Fsm::Move {
                continue;
            }
            break;
        }
        let mind = world.minds.get(ME).unwrap();
        assert!(
            mind.state == Fsm::Battle || mind.asked == Some(Fsm::Battle),
            "the arrival asks Battle"
        );
    }

    /// Berserk turns on below the point and off in Idle (G21).
    #[test]
    fn berserk_turns_on_below_the_point_and_off_in_idle() {
        let clock = TestClock::default();
        clock.set(T0);
        let mut npc = a_fighter(
            ME,
            406,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            gamedata::mob_proto::AIFLAG_BERSERK,
            SPOT,
            10_000,
        );
        npc.fight.berserk_point = 50;
        npc.hp = 1_000;
        npc.max_hp = 10_000;
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut one = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut one);
        hit(&mut world, 1, ME);
        world.process_pulse(P0 + 1);
        assert!(world.minds.get(ME).unwrap().berserk);
        world.set_victim(ME, None);
        // Idle asked; run past the swing's wait until the Idle update runs.
        for pulse in P0 + 2..=P0 + 60 {
            world.process_pulse(pulse);
            let mind = world.minds.get(ME).unwrap();
            if mind.state == Fsm::Idle && !mind.berserk {
                break;
            }
        }
        assert!(!world.minds.get(ME).unwrap().berserk);
    }

    /// A coward in Battle drops its victim (G22).
    #[test]
    fn a_coward_in_battle_drops_its_victim() {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(
            ME,
            406,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            AIFLAG_COWARD,
            SPOT,
            10_000,
        );
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut one = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut one);
        // Idle cowards flee at once, so begin the fight directly.
        world.begin_fight(ME, Vid::new(1));
        world.process_pulse(P0 + 1);
        assert_eq!(world.victim_of(ME), None);
    }

    /// RANGE and MAGIC monsters fight like the melee family (G23, ported in 229s;
    /// `G/char_battle.cpp:267-277` for the arms, `G/char_state.cpp:759-778` for the
    /// eight-tenths chase): a hit begins the fight, Battle chases and swings, and the
    /// swing's fly record names the victim.
    #[test]
    fn range_and_magic_monsters_fight_like_the_melee_family() {
        for battle_type in [BATTLE_TYPE_RANGE, BATTLE_TYPE_MAGIC] {
            let clock = TestClock::default();
            clock.set(T0);
            let npc = a_fighter(
                ME,
                406,
                battle_type,
                (175, 2000),
                AIFLAG_AGGRESSIVE,
                SPOT,
                10_000,
            );
            let mut world = stand_field(&clock, vec![npc], open_cells(), false);
            let mut one = enter(&mut world, 1, NEAR, 12_345);
            world.process_pulse(P0);
            drained(&mut one);
            hit(&mut world, 1, ME);
            assert_eq!(
                world.victim_of(ME),
                Some(Vid::new(1)),
                "a hit begins the fight"
            );
            for pulse in P0 + 1..=P0 + 60 {
                world.process_pulse(pulse);
            }
            let records = drained(&mut one);
            let fly = records
                .iter()
                .find(|record| record.first() == Some(&HEADER_GC_FLY_TARGETING.value()));
            assert!(fly.is_some(), "the swing sends its fly record");
            let fly = GcFlyTargeting::decode(fly.unwrap()).expect("the fly record");
            assert_eq!(fly.shooter_vid, ME);
            assert_eq!(fly.target_vid, 1);
        }
    }

    /// The NOATT bits spare the empire their proto name says (G34, B30).
    #[test]
    fn the_noatt_bits_spare_the_empire_their_proto_name_says() {
        assert!(super::spares_empire(AIFLAG_NOATTSHINSU, 1));
        assert!(!super::spares_empire(AIFLAG_NOATTSHINSU, 2));
        assert!(super::spares_empire(AIFLAG_NOATTCHUNJO, 2));
        assert!(!super::spares_empire(AIFLAG_NOATTCHUNJO, 3));
        assert!(super::spares_empire(AIFLAG_NOATTJINNO, 3));
        assert!(!super::spares_empire(AIFLAG_NOATTJINNO, 1));
        assert!(!super::spares_empire(0, 1));
        assert!(!super::spares_empire(0, 2));
        assert!(!super::spares_empire(0, 3));
        let _ = MOB_RANK_BOSS;
    }

    /// The chase leads a moving target, and not a standing one (G10).
    #[test]
    fn the_chase_leads_a_moving_target() {
        let fighter = Fighter {
            map: PLACE.1,
            at: SPOT,
            ai_flags: 0,
            aggressive: true,
            range: 175,
            sight: 2000,
            attack_speed: 100,
            move_speed: 394.7368,
            hp_pct: 100,
            berserk_point: 0,
            battle_type: BATTLE_TYPE_MELEE,
        };
        let standing = VictimState {
            at: (SPOT.0 + 1000, SPOT.1),
            dead: false,
            banpk: false,
            moving: false,
            rotation: 90.0,
            move_speed: 300.0,
        };
        assert_eq!(super::lead_point(fighter, standing), standing.at);
        // The target runs north while east of the monster: the lead runs ahead north,
        // then the clamp to `fDist` pulls it back along the line to the monster.
        let running = VictimState {
            moving: true,
            rotation: 0.0,
            ..standing
        };
        let led = super::lead_point(fighter, running);
        assert!(led.1 > standing.at.1, "the lead runs ahead north: {led:?}");
        assert!(
            (3990..=4010).contains(&led.0),
            "the clamp pulls it back: {led:?}"
        );
    }

    /// The change of attack position draws two turns near and one far (G11).
    #[test]
    fn the_change_of_attack_position_draws_two_turns_near_and_one_far() {
        let (_clock, mut world, _alpha) = a_fight(10_000, NEAR, true);
        let fighter = world.fighter(ME).expect("the fighter");
        // Near: two draws per try on open cells, so one try.
        let dice = world.dice.clone();
        let _near = world.change_attack_position(fighter, (SPOT.0 + 100, SPOT.1), 400.0, 157.0);
        let mut expect = dice;
        let _first = number(&mut expect, -90, 90);
        let _second = number(&mut expect, -90, 90);
        assert_eq!(world.dice, expect, "two turn draws near");
        // Far: one draw per try.
        let dice = world.dice.clone();
        let _far = world.change_attack_position(fighter, (SPOT.0 + 600, SPOT.1), 600.0, 157.0);
        let mut expect = dice;
        let _only = number(&mut expect, 0, 359);
        assert_eq!(world.dice, expect, "one draw far");
    }

    /// A chaser gives up after 15 seconds 5000 from where it was hit (G16).
    #[test]
    fn a_chaser_gives_up_after_15_seconds_5000_from_where_it_was_hit() {
        let clock = TestClock::default();
        clock.set(T0);
        let home = (400, 3200);
        let npc = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, home, 10_000);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut alpha = enter(&mut world, 1, (500, 3200), 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        // The fight moves on: the clock 16 s past the hit, the monster dragged 5100 east
        // of where it was hit, the victim past its reach beside it.
        clock.set(T0 + 16_000);
        // 5300 east: `DISTANCE_APPROX` reads it above 5000 (its 123/128 slope).
        world.npc_move(ME, home.0 + 5300, home.1);
        assert!(world.move_to(1, home.0 + 5300 + 300, home.1));
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        assert_eq!(world.victim_of(ME), None, "Return drops the victim");
        let records = drained(&mut alpha);
        assert!(
            records.iter().any(|record| {
                record.first() == Some(&HEADER_GC_CHARACTER_MOVE.value())
                    && record.get(1) == Some(&FUNC_WAIT)
            }),
            "Return walks home"
        );
    }

    /// A boss-rank chaser refollows one time in four (G18).

    #[test]
    fn a_boss_rank_chaser_refollows_one_time_in_four() {
        let clock = TestClock::default();
        clock.set(T0);
        let mut npc = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        npc.fight.rank = MOB_RANK_BOSS;
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut one = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut one);
        hit(&mut world, 1, ME);
        assert!(world.move_to(1, SPOT.0 + 1000, SPOT.1));
        world.process_pulse(P0 + 1);
        assert_eq!(world.minds.get(ME).unwrap().state, Fsm::Battle);
        // The chase asked Move; find a seed where the boss refollows.
        let mut refollowed = false;
        for seed in 0..40u64 {
            let clock = TestClock::default();
            clock.set(T0);
            let mut npc = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
            npc.fight.rank = MOB_RANK_BOSS;
            let mut world = stand_field(&clock, vec![npc], open_cells(), true);
            let mut one = enter(&mut world, 1, NEAR, 12_345);
            world.start_pc_recovery(Vid::new(1));
            world.process_pulse(P0);
            drained(&mut one);
            hit(&mut world, 1, ME);
            assert!(world.move_to(1, SPOT.0 + 1000, SPOT.1));
            drained(&mut one);
            world.process_pulse(P0 + 1);
            world.dice = world::character::Pcg32::new(seed, 0);
            // The victim sidesteps: the refollow aims somewhere new, not at `motion.dest`,
            // which legacy's `Goto` would refuse (`G/char.cpp:3478-3487`).
            assert!(world.move_to(1, SPOT.0 + 1500, SPOT.1));
            drained(&mut one);
            world.process_pulse(P0 + 5);
            if world.minds.get(ME).unwrap().asked == Some(Fsm::Move) {
                refollowed = true;
                break;
            }
        }
        assert!(refollowed, "a boss refollows one time in four");
    }

    /// A Goto into Move with a victim draws the ATTACK chat (G20).
    #[test]
    fn a_goto_into_move_with_a_victim_draws_the_attack_chat() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        hit(&mut world, 1, ME);
        assert!(world.move_to(1, SPOT.0 + 1000, SPOT.1));
        drained(&mut alpha);
        let dice = world.dice.clone();
        world.process_pulse(P0 + 1);
        // The chase entered Move with a victim: the chat's draw ran.
        assert_ne!(world.dice, dice, "the ATTACK chat draws");
        let _ = alpha;
    }

    /// `forget_my_attacker` calms the monsters around on this player only (G27).
    #[test]
    fn forget_my_attacker_calms_the_monsters_around_on_this_player_only() {
        let clock = TestClock::default();
        clock.set(T0);
        let one = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let two = a_fighter(
            ME + 1,
            406,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            0,
            (SPOT.0 + 50, SPOT.1),
            10_000,
        );
        let mut world = stand_field(&clock, vec![one, two], open_cells(), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        let mut beta = enter(&mut world, 2, (SPOT.0 + 60, SPOT.1), 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.start_pc_recovery(Vid::new(2));
        world.process_pulse(P0);
        drained(&mut alpha);
        drained(&mut beta);
        world.begin_fight(ME, Vid::new(1));
        world.begin_fight(ME + 1, Vid::new(2));
        hit(&mut world, 1, ME);
        assert!(world.npc_combat[&ME].damage.contains_key(&1));
        world.forget_my_attacker(Vid::new(1));
        assert_eq!(world.minds.get(ME).unwrap().victim, None);
        assert!(
            !world.npc_combat[&ME].damage.contains_key(&1),
            "the dropped victim's row goes too"
        );
        assert_eq!(
            world.minds.get(ME + 1).unwrap().victim,
            Some(Victim::Here(Vid::new(2))),
            "the other victim is untouched"
        );
        assert_eq!(world.find_victim(ME, 2000), Some(Vid::new(2)));
    }

    /// Two attackers past the 3 s window: the second's greater aggro switches the
    /// victim, raises the max and asks for the next Pulse (`ChangeVictimByAggro`,
    /// `G/char_battle.cpp:3854-3900`).
    #[test]
    fn the_greater_aggro_past_the_window_switches_the_victim() {
        let (clock, mut world, _alpha) = a_fight(10_000, NEAR, true);
        let mut beta = enter(&mut world, 2, (SPOT.0 + 100, SPOT.1), 12_345);
        world.start_pc_recovery(Vid::new(2));
        world.process_pulse(P0);
        drained(&mut beta);
        // Alpha hits and begins: the window stamps at T0.
        hit(&mut world, 1, ME);
        assert_eq!(world.victim_of(ME), Some(Vid::new(1)));
        let first = world.npc_combat[&ME].damage[&1].aggro;
        assert!(first > 0, "the first hit feeds the aggro");
        // Inside the window Beta's hits only feed its row: no switch, whatever the max.
        // Enough hits land past Alpha's aggro: the dice vary, so up to ten are swung.
        clock.set(T0 + 2999);
        for _ in 0..10 {
            if world.npc_combat[&ME]
                .damage
                .get(&2)
                .map_or(0, |row| row.aggro)
                > first
            {
                break;
            }
            hit(&mut world, 2, ME);
        }
        assert_eq!(world.victim_of(ME), Some(Vid::new(1)));
        assert!(
            world.npc_combat[&ME].damage[&2].aggro > first,
            "Beta out-damages inside the window"
        );
        // Past the window Beta's next hit switches: the max rises, the stamp moves, the
        // duration asks for the next Pulse.
        clock.set(T0 + 3000);
        hit(&mut world, 2, ME);
        assert_eq!(world.victim_of(ME), Some(Vid::new(2)));
        assert_eq!(
            world.npc_combat[&ME].max_aggro,
            world.npc_combat[&ME].damage[&2].aggro
        );
        assert_eq!(world.npc_combat[&ME].last_victim_set_ms, Some(T0 + 3000));
        assert_eq!(world.minds.get(ME).unwrap().duration, 1);
    }

    /// The same victim's hit past the window with a greater third row scans to it: the
    /// last row in VID order wins the ties (`:3870-3885`).
    #[test]
    fn the_same_victims_hit_scans_to_the_greatest_row_with_vid_ties() {
        let (clock, mut world, _alpha) = a_fight(10_000, NEAR, true);
        let mut beta = enter(&mut world, 2, (SPOT.0 + 100, SPOT.1), 12_345);
        let mut gamma = enter(&mut world, 3, (SPOT.0 + 110, SPOT.1), 12_345);
        world.start_pc_recovery(Vid::new(2));
        world.start_pc_recovery(Vid::new(3));
        world.process_pulse(P0);
        drained(&mut beta);
        drained(&mut gamma);
        hit(&mut world, 1, ME);
        assert_eq!(world.victim_of(ME), Some(Vid::new(1)));
        // Beta and Gamma out-damage inside the window: their rows grow, the victim holds.
        clock.set(T0 + 1000);
        hit(&mut world, 2, ME);
        hit(&mut world, 3, ME);
        let beta = world.npc_combat[&ME].damage[&2].aggro;
        let gamma = world.npc_combat[&ME].damage[&3].aggro;
        assert!(beta > 0 && gamma > 0);
        // Gamma's row is lifted past Beta's and past anything Alpha's next hit feeds
        // its own row, and the max is lifted with it: the same-victim arm then scans
        // past both to Gamma instead of raising the max.
        let alpha = world.npc_combat[&ME].damage[&1].aggro;
        let raised = beta.max(gamma).max(alpha) + 1000;
        world.npc_combat.entry(ME).or_default().max_aggro = raised;
        world.npc_combat.entry(ME).or_default().damage.insert(
            3,
            super::super::fight::NpcDamage {
                total: raised,
                aggro: raised,
            },
        );
        clock.set(T0 + 3001);
        hit(&mut world, 1, ME);
        assert_eq!(
            world.victim_of(ME),
            Some(Vid::new(3)),
            "the scan keeps the last past row"
        );
    }

    /// The chase bleeds the victim's aggro by `level / 3 + 1` every `StateMove` update
    /// (`G/char_state.cpp:855-859`): the victim bonus widens it ×1.2, and the floor is 0.
    #[test]
    fn the_chase_bleeds_the_victims_aggro_every_move_update() {
        let (_clock, mut world, _alpha) = a_fight(10_000, NEAR, true);
        hit(&mut world, 1, ME);
        // The `begin_fight` asks Battle; NEAR is in reach (175 * 1.15), so Battle swings
        // and never chases. The victim steps out: Battle chases, and the Move update
        // bleeds 4 (level 10 bleeds 4, ×1.2 truncated back to 4).
        let first = world.npc_combat[&ME].damage[&1].aggro;
        assert!(first > 10, "the bleed needs room: {first}");
        assert!(world.move_to(1, SPOT.0 + 1000, SPOT.1));
        world.process_pulse(P0 + 1);
        assert_eq!(
            world.minds.get(ME).unwrap().state,
            Fsm::Battle,
            "the first update enters Battle"
        );
        assert_eq!(
            world.minds.get(ME).unwrap().asked,
            Some(Fsm::Move),
            "out of reach chases"
        );
        // The chase steps every `MOVE_STEP_PULSES`: the next update enters Move and bleeds.
        let step = super::super::mob_ai::MOVE_STEP_PULSES;
        world.process_pulse(P0 + 1 + step);
        assert_eq!(world.minds.get(ME).unwrap().state, Fsm::Move);
        let bled = world.npc_combat[&ME].damage[&1].aggro;
        assert_eq!(bled, first - 4, "one chase update bleeds 4");
    }

    /// The bleed's stun check is the instant flag only (`:3838-3840`): a flagged chase
    /// still bleeds, an instant-stunned attacker bleeds nothing.
    #[test]
    fn the_chase_bleed_reads_only_the_instant_stun() {
        use crate::game_state::affect::Affect;
        use crate::game_state::fight::AggroDamage;
        use crate::loading_phase::{AFFECT_STUN, AFF_STUN, STUN_TICKS_NPC};
        let (_clock, mut world, _alpha) = a_fight(10_000, NEAR, true);
        hit(&mut world, 1, ME);
        let first = world.npc_combat[&ME].damage[&1].aggro;
        world.add_npc_affect(
            ME,
            Affect {
                kind: AFFECT_STUN,
                apply_on: 0,
                value: 0,
                flag: AFF_STUN,
                remaining: STUN_TICKS_NPC,
            },
        );
        world.update_aggr_point(ME, Vid::new(1), AggroDamage::Normal, -4);
        assert_eq!(world.npc_combat[&ME].damage[&1].aggro, first - 4);
        world.npc_mut(ME).unwrap().stunned = true;
        world.update_aggr_point(ME, Vid::new(1), AggroDamage::Normal, -4);
        assert_eq!(
            world.npc_combat[&ME].damage[&1].aggro,
            first - 4,
            "no bleed"
        );
    }

    /// A 11000 guard at `at`: NOMOVE, MELEE, range 175, as the proto gives it.
    fn a_guard(vid: u32, at: (i32, i32)) -> world::npc::Npc {
        let mut fight = world::npc::MobFight::default();
        fight.battle_type = BATTLE_TYPE_MELEE;
        fight.attack_range = 175;
        fight.aggressive_sight = 2000;
        world::npc::Npc {
            vid,
            vnum: 11_000,
            race: 11_000,
            char_type: CHAR_TYPE_NPC,
            x: at.0,
            y: at.1,
            ai_flags: AIFLAG_NOMOVE,
            hp: 6938,
            max_hp: 6938,
            fight,
            ..world::npc::Npc::default()
        }
    }

    /// A guard, a monster in its reach, and the player 1 watching, after Pulse [`P0`];
    /// the inbox is the player's, drained. The guard already holds the monster.
    fn a_guard_fight() -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(T0);
        let monster = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let guard = a_guard(ME + 1, NEAR);
        let mut world = stand_field(&clock, vec![monster, guard], open_cells(), true);
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut inbox);
        world.begin_fight_npc(ME + 1, ME);
        (clock, world, inbox)
    }

    /// A guard's Battle swings and stuns: the victim keeps its hit points, carries the
    /// no-reward flag, and shows the stun, and the guard's `FUNC_ATTACK` goes out with
    /// a 50-Pulse wait.
    #[test]
    fn a_guards_battle_swings_and_stuns() {
        use protocol::gc_inventory::HEADER_GC_STUN;
        let (_clock, mut world, mut alpha) = a_guard_fight();
        world.process_pulse(P0 + 1);
        assert_eq!(world.minds.get(ME + 1).unwrap().state, Fsm::Battle);
        let victim = world.npc_by_vid(ME).unwrap();
        assert!(victim.stunned);
        assert_eq!(victim.hp, 10_000);
        assert!(world.npc_combat.get(&ME).unwrap().no_reward);
        let heard = drained(&mut alpha);
        assert!(heard
            .iter()
            .any(|record| record.first() == Some(&HEADER_GC_STUN.value())));
        assert!(heard.iter().any(|record| {
            record.first() == Some(&HEADER_GC_CHARACTER_MOVE.value())
                && GcCharacterMove::decode(record).is_ok_and(|moved| moved.b_func == FUNC_ATTACK)
        }));
        assert_eq!(world.minds.get(ME + 1).unwrap().duration, 50);
    }

    /// The guard arm reads the clip too: a guard stood with the dog's swing
    /// holds 23, not 50.
    #[test]
    fn a_guards_swing_holds_for_its_clip() {
        let (_clock, mut world, mut alpha) = a_guard_fight();
        world.npc_mut(ME + 1).unwrap().attack_bits = Some(0.933_333_f32.to_bits());
        world.process_pulse(P0 + 1);
        assert_eq!(world.minds.get(ME + 1).unwrap().duration, 23);
        drained(&mut alpha);
    }

    /// `swing_hold`'s arithmetic: the clip's seconds times 25, truncated in `f32`;
    /// a zero or missing duration holds 50 (`PASSES_PER_SEC(2)`).
    #[test]
    fn a_swings_hold_is_the_clips_seconds_times_25() {
        assert_eq!(swing_hold(None), 50);
        assert_eq!(swing_hold(Some(0.0)), 50);
        assert_eq!(swing_hold(Some(0.933_333)), 23);
        assert_eq!(swing_hold(Some(1.5)), 37);
        assert_eq!(swing_hold(Some(3.6)), 90, "f32 3.6*25 is 90, f64 89");
        assert_eq!(swing_hold(Some(4.333_333)), 108);
    }

    /// A swing holds Battle for the stood clip's seconds (the stray dog's
    /// 0.933333 s is 23 Pulses); without a clip it holds 50.
    #[test]
    fn a_swing_holds_battle_for_its_clip() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world.npc_mut(ME).unwrap().attack_bits = Some(0.933_333_f32.to_bits());
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        assert_eq!(world.minds.get(ME).unwrap().state, Fsm::Battle);
        assert_eq!(world.minds.get(ME).unwrap().duration, 23);
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        assert_eq!(world.npc_by_vid(ME).unwrap().attack_s(), None);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        assert_eq!(world.minds.get(ME).unwrap().duration, 50);
    }

    /// A guard's Battle drops a stunned victim: both slots clear, the duration is 1
    /// second, and the NOMOVE guard never walks home.
    #[test]
    fn a_guards_battle_drops_a_stunned_victim() {
        let (_clock, mut world, _alpha) = a_guard_fight();
        // Stunned before Battle's first update: the loss arm drops it at once, before any
        // swing runs.
        world.stun_npc(ME);
        world.process_pulse(P0 + 1);
        let mind = world.minds.get(ME + 1).unwrap();
        assert_eq!(mind.state, Fsm::Battle);
        assert!(mind.victim_npc.is_none());
        assert!(mind.victim.is_none());
        assert!(!mind.fighting);
        assert_eq!(mind.duration, pulses(1));
        assert_eq!(
            (
                world.npc_by_vid(ME + 1).unwrap().x,
                world.npc_by_vid(ME + 1).unwrap().y
            ),
            NEAR,
            "NOMOVE never walks"
        );
    }

    /// A guard's Battle drops a victim 4000 away with the duration as it was: no chase
    /// walks, since NOMOVE refuses. `DISTANCE_APPROX` reads 0.96 of a straight line,
    /// so the guard stands 4200 out to clear the 4000 bar.
    #[test]
    fn a_guards_battle_drops_a_distant_victim() {
        let clock = TestClock::default();
        clock.set(T0);
        let monster = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let guard = a_guard(ME + 1, (SPOT.0 + 4200, SPOT.1));
        let mut world = stand_field(&clock, vec![monster, guard], open_cells(), true);
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut inbox);
        world.begin_fight_npc(ME + 1, ME);
        world.process_pulse(P0 + 1);
        let mind = world.minds.get(ME + 1).unwrap();
        assert_eq!(mind.state, Fsm::Battle);
        assert!(mind.victim_npc.is_none());
        assert!(!mind.fighting);
    }

    /// A monster a guard stunned dies unrewarded: the stun event's `Dead` finds
    /// `NO_REWARD` and no damage row, so no exp share reaches the watcher and no
    /// gold or drop half runs — only `GC_DEAD` goes out.
    #[test]
    fn a_guard_stunned_monster_dies_unrewarded() {
        use protocol::gc_inventory::{HEADER_GC_DEAD, HEADER_GC_PLAYER_POINT_CHANGE};
        let (_clock, mut world, mut alpha) = a_guard_fight();
        world.npc_mut(ME).unwrap().exp = 802;
        world.process_pulse(P0 + 1);
        assert!(
            world.npc_by_vid(ME).unwrap().stunned,
            "the swing stuns first"
        );
        drained(&mut alpha);
        // The stun event fires 75 Pulses after the swing's Pulse: run it.
        world.process_pulse(P0 + 76);
        assert!(world.npc_by_vid(ME).unwrap().dead);
        let heard = drained(&mut alpha);
        assert!(heard
            .iter()
            .any(|record| record.first() == Some(&HEADER_GC_DEAD.value())));
        assert!(
            !heard
                .iter()
                .any(|record| record.first() == Some(&HEADER_GC_PLAYER_POINT_CHANGE.value())),
            "no exp share: {heard:?}"
        );
    }
}
