//! A monster's `Attack` on a player (`G/char_battle.cpp:220-328`): the reach, the swing,
//! the claim through [`GameState::claim_pc`], the player's `BeginFight`, and the stamps.
//!
//! A guard's `Attack` on a monster is the same shape with a monster victim
//! ([`GameState::guard_attack`]): the reach, the swing's stun with no reward, the NPC
//! `SetSyncOwner`, the victim's `BeginFight`, and the attack stamp.
//! Everything here runs on the game thread only (ADR-0002).

use std::time::Duration;

use common::point_slot as point;
use common::vid::Vid;
use gamedata::mob_proto::{BATTLE_TYPE_MAGIC, BATTLE_TYPE_RANGE, BATTLE_TYPE_SPECIAL};
use protocol::gc::HEADER_GC_FLY_TARGETING;
use protocol::gc_nested::GcFlyTargeting;
use protocol::gc_vid::GcSpecialEffect;
use tracing::warn;
use world::character::{
    mob_reach, shoot_pc, strike_npc_by_npc, strike_pc, AttackAffect, MobStriker, NpcVictim,
    PcHitEnd, PcStrike, PcVictim, DAMAGE_DODGE,
};

use super::affect::Affect;
use super::death::Attacker;
use super::regen::Due;
use super::view::EntityKey;
use super::GameState;
use crate::loading_phase::{
    AFFECT_POISON, AFFECT_SLOW, AFFECT_STUN, AFF_POISON, AFF_SLOW, AFF_STUN, POISON_TICKS,
    SLOW_MOV_SPEED, SLOW_TICKS, STUN_TICKS,
};
use crate::sync_position::{
    distance_approx, judge_sync_ownership, SyncOwnershipOutcome, SyncOwnershipState,
    SyncPositionVictim,
};

/// `AttackedByPoison`'s `info->count = 10`: the poison event's ticks
/// (`G/char_resist.cpp:207`).
pub(super) const POISON_COUNT: u32 = 10;

/// `AFF_STUN` (`G/affect.h:216`) as a word-0 bit: flag 7 is bit 6.
const AFF_STUN_WORD: u32 = 1 << 6;

impl GameState {
    /// `CHARACTER::Attack` of the monster under `mob` on the player `victim`: whether it hit.
    pub(super) fn mob_attack(&mut self, mob: u32, victim: Vid) -> bool {
        if !self.npc_can_attack_pc(mob, victim) {
            return false;
        }
        // The chat is drawn before the reach test (`:310-316`).
        self.monster_chat_attack(mob);
        if self.mob_is_special(mob) {
            return false;
        }
        let Some(target) = self.victim_state(victim) else {
            return false;
        };
        let Some(npc) = self.npc_by_vid(mob) else {
            return false;
        };
        let reach = mob_reach(npc);
        let distance = distance_approx(
            target.at.0.saturating_sub(npc.x),
            target.at.1.saturating_sub(npc.y),
        );
        if distance > reach {
            return false;
        }
        let berserk = self.minds.get(mob).is_some_and(|mind| mind.berserk);
        let affects = self.affect_flags_of(victim);
        let down = self.is_down(victim.raw());
        // `Attack`'s RANGE and MAGIC arms (`:267-277`): the shot past the same reach, with
        // the fly record before the damage.
        let armed = self.shot_type(mob);
        let shot = armed.is_some();
        let mob_rate = self.mob_damage_rate();
        if shot {
            self.show_fly_target(mob, victim, target.at);
        }
        // Borrow fields, not methods, as `strike` does: the dice, the monster and the points
        // are disjoint.
        let Some(&(channel, map, at)) = self.npc_of.get(&mob) else {
            return false;
        };
        let (channel, map, at) = (channel, map, at);
        let characters = &self.characters;
        let Ok(character) = characters.find_by_vid(victim) else {
            return false;
        };
        let Some(points) = character.points() else {
            return false;
        };
        let dice = &mut self.dice;
        let Some(list) = self.npcs.get(&(channel, map)) else {
            return false;
        };
        let Some(npc) = list.npcs.get(at) else {
            return false;
        };
        let magic = armed.is_some_and(|magic| magic);
        let strike = if shot {
            shoot_pc(
                dice,
                MobStriker { npc, berserk },
                PcVictim {
                    points,
                    affect_flags: affects,
                    down,
                    immune_flag: points.immune_flag(),
                },
                magic,
                mob_rate,
            )
        } else {
            strike_pc(
                dice,
                MobStriker { npc, berserk },
                PcVictim {
                    points,
                    affect_flags: affects,
                    down,
                    immune_flag: points.immune_flag(),
                },
                mob_rate,
            )
        };
        self.show_mob_strike(mob, victim, &strike, shot);
        // The claim, `SetSyncOwner(mob)` on the player (`:299-302`). The NOMOVE arm reads the
        // victim's AI flags, and a player has none, so it never refuses.
        let attackable = self.npc_can_attack_pc(mob, victim);
        let actor_at = self.npc_spot(mob).unwrap_or((0, 0));
        let mark = SyncPositionVictim {
            vid: victim,
            kind: crate::sync_position::SyncPositionVictimKind::Player,
            x: target.at.0,
            y: target.at.1,
        };
        let now_span = self.clock.elapsed();
        let _claimed = self.claim_pc(Vid::new(mob), &mark, actor_at, attackable, now_span);
        // The player's `BeginFight` (`:303-304`).
        if self.can_pc_begin_fight(victim) {
            if let Some(body) = self.bodies.get_mut(&victim) {
                body.fighting = true;
            }
            self.leave_move(victim.raw());
        }
        // The stamps: the monster's `OnMove(true)` and the victim's `OnMove()` (`:317-318`).
        let now = self.clock.now();
        if let Some(combat) = self.npc_combat.get_mut(&mob) {
            combat.last_attack_ms = Some(now);
        }
        if let Some(body) = self.bodies.get_mut(&victim) {
            body.last_move = now;
        }
        true
    }

    /// Whether the monster under `mob` is `SPECIAL`, which `Attack`'s switch never lists:
    /// the `default` logs and answers `BATTLE_NONE`, past the chat and before any reach
    /// test (`:259-279`).
    fn mob_is_special(&self, mob: u32) -> bool {
        if self
            .npc_by_vid(mob)
            .is_some_and(|npc| npc.fight.battle_type == BATTLE_TYPE_SPECIAL)
        {
            warn!(mob, "unhandled battle type 3");
            return true;
        }
        false
    }

    /// `Attack`'s shot arm of the monster under `mob`: `Some(magic)` for RANGE and MAGIC,
    /// `None` for the melee family and SPECIAL (which never reaches here).
    fn shot_type(&self, mob: u32) -> Option<bool> {
        let battle_type = self.npc_by_vid(mob)?.fight.battle_type;
        match battle_type {
            BATTLE_TYPE_RANGE => Some(false),
            BATTLE_TYPE_MAGIC => Some(true),
            _ => None,
        }
    }

    /// `CHARACTER::Attack` of the guard under `mob` on the monster `victim`: whether it
    /// hit. `CanMove` holds (guards walk the state machine), the castle map is not hosted
    /// (`@fixme131` above the chat), and the chat is drawn before the reach test
    /// (`:310-316`). Guards are MELEE, so no shot arm runs. The victim's
    /// `SetSyncOwner` and `BeginFight` follow a hit, with the monster's `OnMove(true)`;
    /// the victim's `OnMove()` is a shop cancel and a mining cancel, both empty on a
    /// monster.
    pub(super) fn guard_attack(&mut self, mob: u32, victim: u32) -> bool {
        if !self.npc_can_attack_npc(mob, victim) {
            return false;
        }
        self.monster_chat_attack(mob);
        let Some(target) = self.npc_victim_state(victim) else {
            return false;
        };
        let Some(npc) = self.npc_by_vid(mob) else {
            return false;
        };
        let reach = mob_reach(npc);
        let (at, berserk) = (
            (npc.x, npc.y),
            self.minds.get(mob).is_some_and(|mind| mind.berserk),
        );
        let distance = distance_approx(
            target.at.0.saturating_sub(at.0),
            target.at.1.saturating_sub(at.1),
        );
        if distance > reach {
            return false;
        }
        let Some(&(channel, map, striker_at)) = self.npc_of.get(&mob) else {
            return false;
        };
        let (channel, map, striker_at) = (channel, map, striker_at);
        let Some(&(victim_channel, victim_map, victim_at)) = self.npc_of.get(&victim) else {
            return false;
        };
        let same_list = channel == victim_channel && map == victim_map;
        // The swing reads the striker and the victim from the same list (both live on the
        // guard's map, which `npc_can_attack_npc` checked) or from two lists when the
        // victim moved between the reach test and the swing.
        let strike = if same_list {
            let dice = &mut self.dice;
            let Some(list) = self.npcs.get(&(channel, map)) else {
                return false;
            };
            let (Some(striker), Some(victim_npc)) =
                (list.npcs.get(striker_at), list.npcs.get(victim_at))
            else {
                return false;
            };
            strike_npc_by_npc(
                dice,
                MobStriker {
                    npc: striker,
                    berserk,
                },
                NpcVictim { npc: victim_npc },
            )
        } else {
            return false;
        };
        self.show_guard_strike(victim, &strike);
        debug_assert!(matches!(
            strike.struck,
            world::character::Struck::StunnedByGuard
        ));
        // The no-reward flag (`SET_BIT(NO_REWARD)`, `:2221`), set before `Stun` runs.
        self.npc_combat.entry(victim).or_default().no_reward = true;
        self.stun_npc(victim);
        // `iRet != BATTLE_NONE`: the victim's `SetSyncOwner(mob)`, then its `BeginFight`
        // past `CanBeginFight` (`:299-304`). The victim is a monster, so its BeginFight
        // guard is the same `fights_back` the player's hit checks.
        let now_span = self.clock.elapsed();
        let mark = SyncPositionVictim {
            vid: Vid::new(victim),
            kind: crate::sync_position::SyncPositionVictimKind::Monster,
            x: target.at.0,
            y: target.at.1,
        };
        let _claimed = self.set_npc_sync_owner(Vid::new(mob), at, &mark, now_span);
        if self
            .npc_by_vid(victim)
            .is_some_and(super::mob_battle::fights_back)
            && self.can_begin_fight(victim)
        {
            self.begin_fight(victim, Vid::new(mob));
        }
        // The stamp: the guard's `OnMove(true)` (`:317`). `m_dwLastAttackTime` is the
        // same field the swing timer reads.
        let now = self.clock.now();
        if let Some(combat) = self.npc_combat.get_mut(&mob) {
            combat.last_attack_ms = Some(now);
        }
        true
    }

    /// What the guard's swing showed: the critical and penetrate effects around the
    /// victim, in order. No fly, no chat, no damage record: the guard arm preempts them.
    fn show_guard_strike(&mut self, victim: u32, strike: &world::character::Strike) {
        let around = EntityKey::Npc(victim);
        for event in &strike.events {
            let world::character::HitEvent::Effect(effect_type) = event else {
                continue;
            };
            let mut record = Vec::new();
            GcSpecialEffect {
                effect_type: *effect_type,
                vid: victim,
            }
            .encode_into(&mut record);
            self.packet_around(around, &record, None);
        }
    }

    /// `battle_is_attackable` of the guard under `mob` on the monster `victim`
    /// (`G/battle.cpp:99-152`): alive victim, neither cell BANPK, the guard neither
    /// stunned nor dead, and `CanAttack` lets a guard hit a monster (`IsGuardNPC`
    /// passes `:391`, the NPC arm answers true at `:417`). No castle, arena or
    /// ship-defence map is hosted, and the victim is no fake hydra (a guard never
    /// searches one).
    fn npc_can_attack_npc(&self, mob: u32, victim: u32) -> bool {
        let Some(npc) = self.npc_by_vid(mob) else {
            return false;
        };
        if npc.stunned || npc.dead {
            return false;
        }
        let Some(target) = self.npc_by_vid(victim) else {
            return false;
        };
        if target.dead {
            return false;
        }
        let Some(&(_, map, _)) = self.npc_of.get(&mob) else {
            return false;
        };
        let Some(&(_, victim_map, _)) = self.npc_of.get(&victim) else {
            return false;
        };
        if map != victim_map {
            return false;
        }
        !self.is_banpk(map, npc.x, npc.y) && !self.is_banpk(map, target.x, target.y)
    }

    /// `battle_is_attackable` of the monster under `mob` on the player `victim`
    /// (`G/battle.cpp:99-152`): neither dead, neither on BANPK, and the monster neither
    /// stunned nor dead. The rest passes for a monster on a player (D-OBS).
    fn npc_can_attack_pc(&self, mob: u32, victim: Vid) -> bool {
        let Some(npc) = self.npc_by_vid(mob) else {
            return false;
        };
        if npc.stunned || npc.dead {
            return false;
        }
        // `Attack`'s `CannotMoveByAffect` (`G/char_battle.cpp:225`): no swing under `AFF_STUN`.
        if self.npc_affect_words(mob)[0] & AFF_STUN_WORD != 0 {
            return false;
        }
        if self.is_dead(victim.raw()) {
            return false;
        }
        let Some((_, map)) = self.place_of(victim.raw()) else {
            return false;
        };
        let Some(spot) = self.spot_of(victim.raw()) else {
            return false;
        };
        if self.is_banpk(map, spot.x, spot.y) {
            return false;
        }
        let Some(&(_, mob_map, _)) = self.npc_of.get(&mob) else {
            return false;
        };
        if mob_map != map {
            return false;
        }
        !self.is_banpk(map, npc.x, npc.y)
    }

    /// What the monster's swing showed: each effect around the player, then what
    /// `NormalAttackAffect` landed (`battle_hit` runs it before the damage), then the hit.
    /// A shot shows a dodge on a `Blocked` end, where a swing shows a block. A `Landed`
    /// or `Down` hit reflects off the player's `POINT_REFLECT_MELEE` between the damage
    /// packet and the hit-point change (`damage_pc_head`, the reflect, `damage_pc_tail`);
    /// a `Blocked` hit never reflects (`Damage` returns before `:1979`).
    fn show_mob_strike(&mut self, mob: u32, victim: Vid, strike: &PcStrike, shot: bool) {
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
        self.apply_pc_affect(victim, strike.affect, None);
        let attacker = Attacker {
            vid: Vid::new(mob),
            pc: false,
        };
        match strike.end {
            PcHitEnd::Blocked if shot => self.send_pc_damage_info(victim, DAMAGE_DODGE, 0),
            PcHitEnd::Blocked => self.send_pc_damage_info(victim, super::fight::DAMAGE_BLOCK, 0),
            PcHitEnd::Down => {
                self.reflect_mob_swing(mob, victim, strike.reflected_base);
                let _hit = self.damage_pc(Some(attacker), victim, 0, None);
            }
            PcHitEnd::Landed { damage, flag } => {
                let head = self.damage_pc_head(Some(attacker), victim, damage, Some(flag), false);
                match head {
                    super::death::PcDamageHead::Open { hp, damage } => {
                        self.reflect_mob_swing(mob, victim, strike.reflected_base);
                        self.damage_pc_tail(Some(attacker), victim, hp, damage);
                    }
                    super::death::PcDamageHead::Gone
                    | super::death::PcDamageHead::Died
                    | super::death::PcDamageHead::Dead => {}
                }
            }
        }
    }

    /// The reflect arm of the monster's swing (`G/char_battle.cpp:1979-1988`): the
    /// player's `POINT_REFLECT_MELEE` share of the pre-bonus `base`, thirded when the
    /// monster is immune to reflect, runs back at it as `DAMAGE_TYPE_SPECIAL`. `None`
    /// (a shot or skill, never NORMAL) or a zero share reflects nothing — but a zero
    /// `reflect_damage` still calls `Damage` in legacy, so the runner runs even at 0
    /// (the chat draw, the checks, the shown 0). A negative share is fixture-only and
    /// clamps at the hit-point change, as 229s clamps a negative skill hit.
    fn reflect_mob_swing(&mut self, mob: u32, victim: Vid, base: Option<i32>) {
        let Some(base) = base else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(victim) else {
            return;
        };
        let Some(points) = character.points() else {
            return;
        };
        let reflect = points.get_point(point::POINT_REFLECT_MELEE);
        if reflect == 0 {
            return;
        }
        let immune = self
            .npc_by_vid(mob)
            .is_some_and(|npc| npc.immune_flag & world::character::IMMUNE_REFLECT != 0);
        let damage = world::character::reflect_damage(reflect, immune, base);
        self.reflect_on_npc(victim, mob, damage);
    }

    /// What `NormalAttackAffect` landed on the player `victim`: the poison, the stun and
    /// the slow, in legacy's arm order (`G/battle.cpp:734-758`). The poison's killer is
    /// the attacker, which a monster has no PID for (`G/char_resist.cpp:211`).
    pub(super) fn apply_pc_affect(
        &mut self,
        victim: Vid,
        landed: AttackAffect,
        attacker: Option<Vid>,
    ) {
        if landed.poisoned {
            self.poison_pc(victim, attacker);
        }
        if landed.stunned {
            self.add_pc_affect(
                victim,
                Affect {
                    kind: AFFECT_STUN,
                    apply_on: u8::try_from(point::POINT_NONE).unwrap(),
                    value: 0,
                    flag: AFF_STUN,
                    remaining: STUN_TICKS,
                    sp_cost: 0,
                },
                true,
            );
        }
        if landed.slowed {
            self.add_pc_affect(
                victim,
                Affect {
                    kind: AFFECT_SLOW,
                    apply_on: u8::try_from(point::POINT_MOV_SPEED).unwrap(),
                    value: SLOW_MOV_SPEED,
                    flag: AFF_SLOW,
                    remaining: SLOW_TICKS,
                    sp_cost: 0,
                },
                true,
            );
        }
    }

    /// `AttackedByPoison` on the player under `victim` (`G/char_resist.cpp:178-220`), past
    /// the draws the strike already passed: nothing while a poison event runs; else the
    /// poison affect, and the poison event with 10 ticks, due 1 Pulse on. The
    /// poisoned-once arm is monster-victim-only, and the commented immunity arm never
    /// runs. The poison carries its attacker (`info->attacker_pid`, `:211`), which a
    /// monster has no PID for.
    pub(super) fn poison_pc(&mut self, victim: Vid, attacker: Option<Vid>) {
        let raw = victim.raw();
        if self
            .pc_combat
            .get(&raw)
            .is_some_and(|combat| combat.poison_due.is_some())
        {
            return;
        }
        self.add_pc_affect(
            victim,
            Affect {
                kind: AFFECT_POISON,
                apply_on: u8::try_from(point::POINT_NONE).unwrap(),
                value: 0,
                flag: AFF_POISON,
                remaining: POISON_TICKS,
                sp_cost: 0,
            },
            true,
        );
        let when = self.last_pulse.saturating_add(1);
        let killer = attacker.map_or(0, Vid::raw);
        let combat = self.pc_combat.entry(raw).or_default();
        let old = combat.poison_due.replace((when, POISON_COUNT, killer));
        if let Some((at, count, killer)) = old {
            self.regens.due.remove(&(
                at,
                Due::PcPoison {
                    vid: raw,
                    count,
                    killer,
                },
            ));
        }
        self.regens.due.insert((
            when,
            Due::PcPoison {
                vid: raw,
                count: POISON_COUNT,
                killer,
            },
        ));
    }

    /// `FlyTarget` of `mob` on the player `victim` at `at` (`G/char_battle.cpp:3342-3371`):
    /// the `GC_FLY_TARGETING` around the monster. The victim is online past
    /// `npc_can_attack_pc`, so the record always names it, never `(0, x, y)`.
    fn show_fly_target(&mut self, mob: u32, victim: Vid, at: (i32, i32)) {
        let mut record = Vec::new();
        GcFlyTargeting::new(HEADER_GC_FLY_TARGETING, mob, victim.raw(), at.0, at.1)
            .encode_into(&mut record);
        let me = EntityKey::Npc(mob);
        self.packet_around(me, &record, Some(me));
    }

    /// `SetSyncOwner(actor)` on the player `victim`, past the NOMOVE arm (a player has no AI
    /// flags): `!attackable` runs the `DAMAGE_BLOCK` and refuses; else `judge_sync_ownership`
    /// decides. The client sync passes `!(dead victim || down actor)`, the monster
    /// `npc_can_attack_pc`.
    pub(super) fn claim_pc(
        &mut self,
        actor: Vid,
        victim: &SyncPositionVictim,
        actor_at: (i32, i32),
        attackable: bool,
        now: Duration,
    ) -> bool {
        if !attackable {
            self.block_pc_claim(victim.vid, actor);
            return false;
        }
        let state = self
            .bodies
            .get(&victim.vid)
            .map(|body| body.sync)
            .unwrap_or_default();
        match judge_sync_ownership(actor, victim, state, actor_at.0, actor_at.1, now) {
            SyncOwnershipOutcome::Refused => false,
            // The owner past the limit keeps the claim as it was and sends nothing.
            SyncOwnershipOutcome::Kept => true,
            SyncOwnershipOutcome::Accepted {
                owner_changed,
                record,
            } => {
                if let Some(body) = self.bodies.get_mut(&victim.vid) {
                    if owner_changed {
                        body.last_sync = None;
                    }
                    body.sync = SyncOwnershipState {
                        owner: Some(actor),
                        claimed_at: Some(now),
                    };
                }
                let key = EntityKey::Character(victim.vid.raw());
                self.packet_around(key, &record, None);
                true
            }
        }
    }

    /// `CanBeginFight` of the player `victim` (`G/char_battle.cpp:108-115`): standing (not
    /// `fighting`), neither dead nor stunned, and `CanMove` (no stun affect and no own
    /// shop, of which only the stun is ported).
    fn can_pc_begin_fight(&self, victim: Vid) -> bool {
        self.bodies.get(&victim).is_some_and(|body| !body.fighting)
            && !self.is_down(victim.raw())
            && self.affect_flags_of(victim)[0] & AFF_STUN_WORD == 0
    }

    /// `SendDamagePacket` to the player under `victim` (`G/char_battle.cpp:1720-1742`): its
    /// own `GC_DAMAGE_INFO`. A monster's swing has no desc to send a second copy to.
    pub(super) fn send_pc_damage_info(&self, victim: Vid, flag: u8, damage: i32) {
        self.send_pc_damage_info_to(victim, None, flag, damage);
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;
    use std::time::Duration;

    use common::point_slot as point;
    use common::vid::Vid;
    use gamedata::mob_proto::BATTLE_TYPE_MELEE;
    use gamedata::mob_proto::BATTLE_TYPE_SPECIAL;
    use gamedata::server_attr::{
        CellBlock, SectreeGrid, ServerAttr, ATTR_BANPK, CELL_SIZE, SECTREE_CELLS,
    };
    use protocol::gc_actors::GcCharacterMove;
    use protocol::gc_inventory::{
        HEADER_GC_CHARACTER_MOVE, HEADER_GC_DAMAGE_INFO, HEADER_GC_DEAD, HEADER_GC_OWNERSHIP,
        HEADER_GC_PHASE, HEADER_GC_PLAYER_POINT_CHANGE, HEADER_GC_STUN, HEADER_GC_TARGET,
    };
    use protocol::gc_nested::{GcDamageInfo, GcPointChange};
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::cells::{HostedCells, MapCells};
    use world::character::{number, Pcg32};
    use world::npc::FIRST_NPC_VID;

    use super::super::fixtures::{
        a_fighter, enter, hit, open_cells, region, stand_field, TestClock, PLACE,
    };
    use super::super::mob_ai::Fsm;
    use super::*;
    use crate::game_loop::PulseProcessor;
    use crate::movement::FUNC_ATTACK;
    use crate::sync_position::{
        ownership_record, SyncPositionVictim, SyncPositionVictimKind, SYNC_CLAIM_LIFETIME,
    };

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
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let _ = no_wander;
        let mut inbox = enter(&mut world, 1, at, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut inbox);
        (clock, world, inbox)
    }

    /// The first byte of each record, with a point change's word read as its low byte.
    fn headers(records: &[Vec<u8>]) -> Vec<u8> {
        records
            .iter()
            .map(|record| record.first().copied().unwrap_or(0))
            .collect()
    }

    /// `GC_DAMAGE_INFO{victim, flag, damage}`.
    fn damage_info(vid: u32, flag: u8, damage: i32) -> Vec<u8> {
        let mut record = Vec::new();
        GcDamageInfo {
            header: GcDamageInfo::header(),
            vid,
            flag,
            damage,
        }
        .encode_into(&mut record);
        record
    }

    /// The cells of [`PLACE`], `ATTR_BANPK` in the columns of `band`.
    fn banpk_cells(band: Range<i32>) -> std::sync::Arc<HostedCells> {
        let side = SECTREE_CELLS;
        let cells: Vec<u32> = (0..side * side)
            .map(|at| {
                let x = i32::try_from(at % side).unwrap() * CELL_SIZE;
                if band.contains(&x) {
                    ATTR_BANPK
                } else {
                    0
                }
            })
            .collect();
        let attr = ServerAttr::from_blocks(1, 1, vec![CellBlock::from_cells(&cells)]).unwrap();
        let grid = SectreeGrid::of(&region()).unwrap();
        let mut hosted = HostedCells::default();
        hosted.insert(PLACE.1, MapCells::new(grid, std::sync::Arc::new(attr)));
        std::sync::Arc::new(hosted)
    }

    /// A seed whose first `number(0, 60)` is not 0, so no swing chats.
    fn quiet_seed() -> u64 {
        for seed in 0..100u64 {
            let mut dice = Pcg32::new(seed, 0);
            if number(&mut dice, 0, 60) != 0 {
                return seed;
            }
        }
        unreachable!("one seed in a hundred chats")
    }

    /// The hit points of the player 1.
    fn hp_of(world: &mut GameState) -> i32 {
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .unwrap()
            .items_and_points_mut()
            .1
            .unwrap()
            .hp()
    }

    /// A monster in reach swings every 50 Pulses in legacy's record order (G6).
    #[test]
    fn a_monster_in_reach_swings_every_50_pulses_in_legacys_record_order() {
        let (clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        let hp = hp_of(&mut world);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        let seed = quiet_seed();
        world.dice = Pcg32::new(seed, 0);
        world.process_pulse(P0 + 1);
        let heard = drained(&mut alpha);
        assert_eq!(
            headers(&heard),
            [
                HEADER_GC_DAMAGE_INFO.value(),
                HEADER_GC_PLAYER_POINT_CHANGE.value(),
                HEADER_GC_OWNERSHIP.value(),
                HEADER_GC_CHARACTER_MOVE.value(),
            ]
        );
        let hit = GcDamageInfo::decode(&heard[0]).expect("a damage info");
        assert_eq!((hit.vid, hit.flag), (1, 1));
        assert!(
            (1..=5).contains(&hit.damage),
            "the <3 floor: {}",
            hit.damage
        );
        let changed = GcPointChange::decode(&heard[1]).expect("a point change");
        assert_eq!(
            (changed.vid, changed.change_type, changed.value),
            (
                1,
                u8::try_from(point::POINT_HP).unwrap(),
                i64::from(hp - hit.damage)
            )
        );
        assert_eq!(heard[2], ownership_record(Vid::new(ME), Vid::new(1)));
        let moved = GcCharacterMove::decode(&heard[3]).expect("a move");
        assert_eq!((moved.b_func, moved.dw_vid), (FUNC_ATTACK, ME));
        // The draw order: the chat, the damage roll, the `<3` floor's `number(1, 5)`.
        let mut expect = Pcg32::new(seed, 0);
        let _chat = number(&mut expect, 0, 60);
        let _roll = number(&mut expect, 0, 0);
        let _floor = number(&mut expect, 1, 5);
        assert_eq!(world.dice, expect, "three draws a swing");
        let mind = world.minds.get(ME).unwrap();
        assert_eq!((mind.next_pulse, mind.duration), (52, 50));
        // The next swing, 50 Pulses later: the 4000 ms wait at attack speed 0 is over.
        clock.set(T0 + 4100);
        let mut expect = world.dice.clone();
        let _chat = number(&mut expect, 0, 60);
        let _roll = number(&mut expect, 0, 0);
        let _floor = number(&mut expect, 1, 5);
        world.process_pulse(52);
        let heard = drained(&mut alpha);
        assert_eq!(heard[0][0], HEADER_GC_DAMAGE_INFO.value());
        assert_eq!(world.dice, expect, "three draws a swing again");
    }

    /// The attack timer waits out the remainder (G7).
    #[test]
    fn the_attack_timer_waits_out_the_remainder() {
        let (clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world
            .npc_mut(ME)
            .unwrap()
            .fight
            .add(point::POINT_ATT_SPEED, 100);
        world.dice = Pcg32::new(quiet_seed(), 0);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        assert_eq!(drained(&mut alpha)[0][0], HEADER_GC_DAMAGE_INFO.value());
        clock.set(T0 + 1500);
        world.process_pulse(P0 + 52);
        let mind = world.minds.get(ME).unwrap();
        assert_eq!((mind.next_pulse, mind.duration), (P0 + 64, 12));
        assert!(
            drained(&mut alpha).is_empty(),
            "no swing before the wait is over"
        );
    }

    /// An `AFF_STUN` attacker never swings (`:225`): the Battle update sends nothing, the
    /// chat draw never runs, and the victim's hit points stay.
    #[test]
    fn a_flag_stunned_attacker_never_swings() {
        use crate::game_state::affect::Affect;
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world.dice = Pcg32::new(quiet_seed(), 0);
        world.add_npc_affect(
            ME,
            Affect {
                kind: AFFECT_STUN,
                apply_on: 0,
                value: 0,
                flag: AFF_STUN,
                remaining: STUN_TICKS,
                sp_cost: 0,
            },
        );
        drained(&mut alpha);
        let expect = Pcg32::new(quiet_seed(), 0);
        let hp = hp_of(&mut world);
        assert!(!world.mob_attack(ME, Vid::new(1)));
        assert_eq!(drained(&mut alpha), Vec::<Vec<u8>>::new(), "no swing");
        assert_eq!(world.dice, expect, "no chat draw");
        assert_eq!(hp_of(&mut world), hp, "untouched");
    }

    /// A monster on a BANPK cell cannot swing and waits 12 (G8).
    #[test]
    fn a_monster_on_a_banpk_cell_cannot_swing_and_waits_12() {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], banpk_cells(3150..3250), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut alpha);
        world.begin_fight(ME, Vid::new(1));
        world.process_pulse(P0 + 1);
        let mind = world.minds.get(ME).unwrap();
        assert_eq!((mind.state, mind.duration), (Fsm::Battle, 12));
        assert!(
            !drained(&mut alpha)
                .iter()
                .any(|record| record[0] == HEADER_GC_DAMAGE_INFO.value()),
            "no swing from a BANPK cell"
        );
    }

    /// A `SPECIAL` monster's swing blanks: the chat is drawn, then `Attack`'s switch
    /// answers `BATTLE_NONE` with no claim, no hit and no stamps.
    #[test]
    fn a_special_monsters_swing_blanks() {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 406, BATTLE_TYPE_SPECIAL, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut alpha);
        let seed = quiet_seed();
        world.dice = Pcg32::new(seed, 0);
        world.begin_fight(ME, Vid::new(1));
        world.process_pulse(P0 + 1);
        assert!(
            drained(&mut alpha).is_empty(),
            "no record from a blank swing"
        );
        let mut expect = Pcg32::new(seed, 0);
        let _chat = number(&mut expect, 0, 60);
        assert_eq!(world.dice, expect, "the chat draw only");
        assert!(
            world
                .npc_combat
                .get(&ME)
                .and_then(|combat| combat.last_attack_ms)
                .is_none(),
            "no attack stamp"
        );
        let mind = world.minds.get(ME).unwrap();
        assert_eq!((mind.state, mind.duration), (Fsm::Battle, 12));
    }

    /// The swing on a stunned player kills it with the monster as killer (G24).
    #[test]
    fn the_swing_on_a_stunned_player_kills_it_with_the_monster_as_killer() {
        let (clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world.dice = Pcg32::new(quiet_seed(), 0);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        world.process_pulse(P0 + 1);
        drained(&mut alpha);
        // The stun, due at 77; the next swing at 52 lands first.
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(world.is_down(1));
        drained(&mut alpha);
        clock.set(T0 + 4100);
        world.process_pulse(52);
        let heard = drained(&mut alpha);
        assert_eq!(
            headers(&heard),
            [
                HEADER_GC_OWNERSHIP.value(),
                HEADER_GC_OWNERSHIP.value(),
                HEADER_GC_PHASE.value(),
                HEADER_GC_DEAD.value(),
                HEADER_GC_DAMAGE_INFO.value(),
                HEADER_GC_CHARACTER_MOVE.value(),
            ]
        );
        // `ClearSync`: the player's own release, then what it owned (the monster its
        // punch claimed).
        assert_eq!(heard[0], ownership_record(Vid::new(0), Vid::new(1)));
        assert_eq!(heard[1], ownership_record(Vid::new(0), Vid::new(ME)));
        assert_eq!(
            heard[4],
            damage_info(1, super::super::fight::DAMAGE_BLOCK, 0)
        );
        let moved = GcCharacterMove::decode(&heard[5]).expect("a move");
        assert_eq!((moved.b_func, moved.dw_vid), (FUNC_ATTACK, ME));
        // The death has a killer that is not a player: the penalty flag is set.
        assert!(world
            .pc_combat
            .get(&1)
            .is_some_and(|combat| combat.death_penalty));
        assert!(world.pc_combat.get(&1).is_some_and(|combat| combat.dead));
    }

    /// A stun event and a swing due on one Pulse: the event kills first (G25).
    #[test]
    fn a_stun_event_and_a_swing_due_on_one_pulse() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        // The stun, due at 76; the punch lands at 75, so the first Battle update
        // is due on the same Pulse.
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        for pulse in P0 + 1..76 {
            world.process_pulse(pulse);
        }
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        world.process_pulse(76);
        let heard = drained(&mut alpha);
        // The stun event killed without a killer; Battle then saw the victim dead.
        assert!(world.pc_combat.get(&1).is_some_and(|combat| combat.dead));
        assert!(!world
            .pc_combat
            .get(&1)
            .is_some_and(|combat| combat.death_penalty));
        let mind = world.minds.get(ME).unwrap();
        assert_eq!((mind.state, world.victim_of(ME)), (Fsm::Idle, None));
        assert!(
            !heard
                .iter()
                .any(|record| record[0] == HEADER_GC_DAMAGE_INFO.value()),
            "no swing on a dead victim"
        );
        assert!(
            !heard.iter().any(|record| {
                record[0] == HEADER_GC_CHARACTER_MOVE.value()
                    && GcCharacterMove::decode(record)
                        .is_ok_and(|moved| moved.b_func == FUNC_ATTACK)
            }),
            "no attack move on a dead victim"
        );
    }

    /// A dead or destroyed monster releases the players it owns (G26).
    #[test]
    fn a_dead_or_destroyed_monster_releases_the_players_it_owns() {
        // Death: the claim, then a punch that kills.
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world.dice = Pcg32::new(quiet_seed(), 0);
        hit(&mut world, 1, ME);
        world.process_pulse(P0 + 1);
        drained(&mut alpha);
        assert_eq!(
            world.bodies.get(&Vid::new(1)).unwrap().sync.owner,
            Some(Vid::new(ME))
        );
        world.npc_mut(ME).unwrap().hp = 1;
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        for pulse in P0 + 2..=77 {
            world.process_pulse(pulse);
        }
        let heard = drained(&mut alpha);
        let unowned = ownership_record(Vid::new(0), Vid::new(1));
        let freed = ownership_record(Vid::new(0), Vid::new(ME));
        let own_at = heard.iter().position(|record| *record == freed);
        let owned_at = heard.iter().position(|record| *record == unowned);
        assert!(
            own_at.is_some_and(|first| owned_at.is_some_and(|second| first < second)),
            "the monster's own release comes before what it owned: {heard:?}"
        );
        // Destroy: the claim, then the world takes the monster down.
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world.dice = Pcg32::new(quiet_seed(), 0);
        assert!(world.mob_attack(ME, Vid::new(1)));
        drained(&mut alpha);
        world.destroy_npc(ME);
        let heard = drained(&mut alpha);
        let own_at = heard.iter().position(|record| *record == freed);
        let owned_at = heard.iter().position(|record| *record == unowned);
        assert!(
            own_at.is_some_and(|first| owned_at.is_some_and(|second| first < second)),
            "destroy releases the owned after the own: {heard:?}"
        );
    }

    /// A monster's claim on a player is kept by its owner and refused to others past 250 (G28).
    #[test]
    fn a_monsters_claim_on_a_player_is_kept_by_its_owner_and_refused_to_others_beyond_250() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        let victim = SyncPositionVictim {
            vid: Vid::new(1),
            kind: SyncPositionVictimKind::Player,
            x: NEAR.0,
            y: NEAR.1,
        };
        let now = world.clock.elapsed();
        let me = Vid::new(ME);
        let other = Vid::new(ME + 1);
        // The first claim is accepted and shown.
        assert!(world.claim_pc(me, &victim, SPOT, true, now));
        assert_eq!(drained(&mut alpha), [ownership_record(me, Vid::new(1))]);
        assert_eq!(world.bodies.get(&Vid::new(1)).unwrap().sync.owner, Some(me));
        // The owner past 250 keeps the claim with no record and no new stamp.
        world.bodies.get_mut(&Vid::new(1)).unwrap().last_sync = Some(Duration::from_millis(5));
        assert!(world.claim_pc(me, &victim, (NEAR.0 + 300, NEAR.1), true, now));
        assert!(drained(&mut alpha).is_empty());
        assert_eq!(
            world.bodies.get(&Vid::new(1)).unwrap().last_sync,
            Some(Duration::from_millis(5))
        );
        // Another monster past 250 is refused.
        assert!(!world.claim_pc(other, &victim, (NEAR.0 + 300, NEAR.1), true, now));
        assert!(drained(&mut alpha).is_empty());
        assert_eq!(world.bodies.get(&Vid::new(1)).unwrap().sync.owner, Some(me));
        // An owner change within 250 resets `last_sync`.
        let later = now + SYNC_CLAIM_LIFETIME;
        assert!(world.claim_pc(other, &victim, NEAR, true, later));
        assert_eq!(world.bodies.get(&Vid::new(1)).unwrap().last_sync, None);
        assert_eq!(
            world.bodies.get(&Vid::new(1)).unwrap().sync.owner,
            Some(other)
        );
        // A dead player gets the `DAMAGE_BLOCK`.
        drained(&mut alpha);
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(world.is_dead(1));
        drained(&mut alpha);
        assert!(!world.claim_pc(me, &victim, SPOT, false, later));
        assert_eq!(
            drained(&mut alpha),
            [damage_info(1, super::super::fight::DAMAGE_BLOCK, 0)]
        );
    }

    /// The targeters see the player's hit points before its stun (G29).
    #[test]
    fn the_targeters_see_the_players_hit_points_before_its_stun() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        let mut bravo = enter(&mut world, 2, (3300, 3200), 12_346);
        world.target(Vid::new(2), 1);
        drained(&mut alpha);
        drained(&mut bravo);
        let hp = hp_of(&mut world);
        world.damage_pc(None, Vid::new(1), hp - 1, None);
        drained(&mut alpha);
        world.dice = Pcg32::new(quiet_seed(), 0);
        assert!(world.mob_attack(ME, Vid::new(1)));
        let heard = drained(&mut bravo);
        let target_at = heard
            .iter()
            .position(|record| record[0] == HEADER_GC_TARGET.value());
        let stun_at = heard
            .iter()
            .position(|record| record[0] == HEADER_GC_STUN.value());
        assert!(
            target_at.is_some_and(|first| stun_at.is_some_and(|second| first < second)),
            "the target flush comes before the stun: {heard:?}"
        );
    }

    /// A running player stops at the first monster hit until it moves again (G35).
    #[test]
    fn a_running_player_stops_at_the_first_monster_hit_until_it_moves_again() {
        const OTHER: u32 = FIRST_NPC_VID + 1;
        let clock = TestClock::default();
        clock.set(T0);
        let npcs = vec![
            a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000),
            a_fighter(OTHER, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000),
        ];
        let mut world = stand_field(&clock, npcs, open_cells(), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut alpha);
        world.dice = Pcg32::new(quiet_seed(), 0);
        assert!(world.goto(1, NEAR.0 + 2000, NEAR.1));
        clock.set(T0 + 500);
        world.process_pulse(P0 + 1);
        let frozen = world.spot_of(1).unwrap();
        assert_ne!((frozen.x, frozen.y), NEAR, "the player stepped");
        // The first landed hit freezes it: fighting, out of Move and out of the movers.
        assert!(world.mob_attack(ME, Vid::new(1)));
        assert!(world.bodies.get(&Vid::new(1)).unwrap().fighting);
        assert!(!world.bodies.get(&Vid::new(1)).unwrap().motion.moving);
        assert!(!world.movers.contains(&1));
        for pulse in (P0 + 2)..=(P0 + 5) {
            world.process_pulse(pulse);
        }
        let held = world.spot_of(1).unwrap();
        assert_eq!((held.x, held.y), (frozen.x, frozen.y));
        // The chase does not lead it: it is not moving.
        assert!(!world.victim_state(Vid::new(1)).unwrap().moving);
        // Its next move runs again; a second hit changes nothing.
        world.goto(1, NEAR.0 + 2000, NEAR.1);
        assert!(world.bodies.get(&Vid::new(1)).unwrap().motion.moving);
        assert!(world.mob_attack(ME, Vid::new(1)));
        assert!(world.bodies.get(&Vid::new(1)).unwrap().fighting);
        // Its own kill stands it down, and the next monster hit freezes it again.
        world.npc_mut(ME).unwrap().hp = 1;
        hit(&mut world, 1, ME);
        assert!(world.npc_by_vid(ME).is_some_and(|npc| npc.stunned));
        hit(&mut world, 1, ME);
        assert!(world.npc_by_vid(ME).is_some_and(|npc| npc.dead));
        assert!(!world.bodies.get(&Vid::new(1)).unwrap().fighting);
        assert!(world.mob_attack(OTHER, Vid::new(1)));
        assert!(world.bodies.get(&Vid::new(1)).unwrap().fighting);
        // Its death clears the flag too.
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        world.damage_pc(None, Vid::new(1), i32::MAX, None);
        assert!(world.is_dead(1));
        assert!(!world.bodies.get(&Vid::new(1)).unwrap().fighting);
    }

    /// A monster's hit lands the poison, stun and slow: three adds before the damage
    /// info, the move speed down 30, and the poison event due one Pulse on.
    #[test]
    fn a_hit_lands_the_poison_stun_and_slow_before_the_damage() {
        use protocol::gc_inventory::{HEADER_GC_AFFECT_ADD, HEADER_GC_CHARACTER_UPDATE};
        use protocol::gc_nested::GcAffectAdd;
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        world.dice = Pcg32::new(quiet_seed(), 0);
        let npc = world.npc_mut(ME).unwrap();
        npc.fight.add(point::POINT_POISON_PCT, 100);
        npc.fight.add(point::POINT_STUN_PCT, 100);
        npc.fight.add(point::POINT_SLOW_PCT, 100);
        npc.fight.level = 154;
        let before = world.move_speed_of(1);
        assert!(world.mob_attack(ME, Vid::new(1)));
        let heard = drained(&mut alpha);
        let kinds: Vec<u32> = heard
            .iter()
            .filter(|record| record.first() == Some(&HEADER_GC_AFFECT_ADD.value()))
            .map(|record| GcAffectAdd::decode(record).unwrap().element.affect_type)
            .collect();
        assert_eq!(kinds, [AFFECT_POISON, AFFECT_STUN, AFFECT_SLOW]);
        let damage_at = heard
            .iter()
            .position(|record| record.first() == Some(&HEADER_GC_DAMAGE_INFO.value()));
        let last_add = heard
            .iter()
            .rposition(|record| record.first() == Some(&HEADER_GC_AFFECT_ADD.value()));
        assert!(last_add.is_some_and(|add| damage_at.is_some_and(|hit| add < hit)));
        assert!(heard
            .iter()
            .any(|record| record.first() == Some(&HEADER_GC_CHARACTER_UPDATE.value())));
        assert_eq!(world.move_speed_of(1), (before - 30).max(0));
        let combat = world.pc_combat.get(&1).unwrap();
        assert!(combat
            .poison_due
            .is_some_and(|(when, count, killer)| when == P0 + 1 && count == 10 && killer == 0));
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
            char_type: gamedata::mob_proto::CHAR_TYPE_NPC,
            x: at.0,
            y: at.1,
            ai_flags: gamedata::mob_proto::AIFLAG_NOMOVE,
            hp: 6938,
            max_hp: 6938,
            fight,
            ..world::npc::Npc::default()
        }
    }

    /// A guard's swing stuns the monster: no hit points taken, the no-reward flag set,
    /// the stun shown, the guard claimed as sync owner, and no fight begun — the victim
    /// is stunned, so `CanBeginFight` refuses (`G/char_battle.cpp:303-304`). The player
    /// watches from `NEAR`, which is in the victim's view.
    #[test]
    fn a_guard_swing_stuns_the_monster() {
        use protocol::gc_inventory::HEADER_GC_OWNERSHIP;
        let clock = TestClock::default();
        clock.set(T0);
        let monster = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let guard = a_guard(ME + 1, NEAR);
        let mut world = stand_field(&clock, vec![monster, guard], open_cells(), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut alpha);
        assert!(world.guard_attack(ME + 1, ME));
        let victim = world.npc_by_vid(ME).unwrap();
        assert!(victim.stunned);
        assert_eq!(victim.hp, 10_000);
        assert!(world.npc_combat.get(&ME).unwrap().no_reward);
        let heard = drained(&mut alpha);
        assert!(heard
            .iter()
            .any(|record| record.first() == Some(&HEADER_GC_STUN.value())));
        assert!(heard
            .iter()
            .any(|record| record.first() == Some(&HEADER_GC_OWNERSHIP.value())));
        // Stunned, so `CanBeginFight` refuses: the victim holds no fight.
        let mind = world.minds.get(ME);
        assert!(mind.is_none_or(|mind| !mind.fighting && mind.victim.is_none()));
    }

    /// A guard's swing past the reach blanks: nothing stunned, nothing claimed, no
    /// records.
    #[test]
    fn a_guard_swing_past_the_reach_blanks() {
        let clock = TestClock::default();
        clock.set(T0);
        let monster = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let guard = a_guard(ME + 1, (SPOT.0 + 500, SPOT.1));
        let mut world = stand_field(&clock, vec![monster, guard], open_cells(), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut alpha);
        assert!(!world.guard_attack(ME + 1, ME));
        let victim = world.npc_by_vid(ME).unwrap();
        assert!(!victim.stunned);
        assert!(!world
            .npc_combat
            .get(&ME)
            .is_some_and(|combat| combat.no_reward));
        assert!(drained(&mut alpha).is_empty());
    }

    /// The tests' dice replay: the chat, the damage roll, the `<3` floor.
    fn swing_draws(seed: u64) -> (i32, i32, i32) {
        let mut dice = Pcg32::new(seed, 0);
        let chat = number(&mut dice, 0, 60);
        let roll = number(&mut dice, 0, 0);
        let floor = number(&mut dice, 1, 5);
        (chat, roll, floor)
    }

    /// The first seed whose swing chats nothing and floors at `floor`.
    fn swing_seed(floor: i32) -> u64 {
        (0..10_000u64)
            .find(|&seed| {
                let (chat, _, floored) = swing_draws(seed);
                chat != 0 && floored == floor
            })
            .expect("a seed in 10,000")
    }

    /// The player's `POINT_REFLECT_MELEE`.
    fn give_reflect(world: &mut GameState, amount: i32) {
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .unwrap()
            .items_and_points_mut()
            .1
            .unwrap()
            .point_change(point::POINT_REFLECT_MELEE, amount, false, false)
            .expect("reflect is ported");
    }

    /// A NORMAL swing at a reflecting player runs back at the monster (G1): the
    /// empire tenth of the full share, the damage map without the party share, and the
    /// damage info only while the player targets it.
    #[test]
    fn a_swing_at_a_reflecting_player_runs_back_at_the_monster() {
        let (clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        let _ = clock;
        give_reflect(&mut world, 100);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        let seed = swing_seed(4);
        world.dice = Pcg32::new(seed, 0);
        let before = world.npc_by_vid(ME).unwrap().hp;
        let row = world.npc_combat.get(&ME).unwrap().damage[&1].total;
        world.process_pulse(P0 + 1);
        let heard = drained(&mut alpha);
        // The incoming info, the reflected info, the target bar's refresh, then the
        // player's hit-point change: the reflect lands between the head and the tail
        // (G6's order, pinned here too).
        assert_eq!(
            headers(&heard)[..4],
            [
                HEADER_GC_DAMAGE_INFO.value(),
                HEADER_GC_DAMAGE_INFO.value(),
                HEADER_GC_TARGET.value(),
                HEADER_GC_PLAYER_POINT_CHANGE.value(),
            ]
        );
        let incoming = GcDamageInfo::decode(&heard[0]).expect("the incoming info");
        assert_eq!((incoming.vid, incoming.flag, incoming.damage), (1, 1, 4));
        let reflected = GcDamageInfo::decode(&heard[1]).expect("the reflected info");
        // The full share 4 takes the empire tenth (1 vs map 41's 3): 3.
        assert_eq!(
            (reflected.vid, reflected.flag, reflected.damage),
            (ME, 1, 3)
        );
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, before - 3);
        assert_eq!(world.npc_combat.get(&ME).unwrap().damage[&1].total, row + 3);
        let mut expect = Pcg32::new(seed, 0);
        let _chat = number(&mut expect, 0, 60);
        let _roll = number(&mut expect, 0, 0);
        let _floor = number(&mut expect, 1, 5);
        let _reflect_chat = number(&mut expect, 0, 60);
        assert_eq!(world.dice, expect, "the swing plus the reflect's chat draw");
    }

    /// The reflected hit spreads no party share (G1): the follower's row is untouched.
    #[test]
    fn a_reflected_hit_spreads_no_party_share() {
        let clock = TestClock::default();
        clock.set(T0);
        let master = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let mut follower = a_fighter(
            ME + 1,
            406,
            BATTLE_TYPE_MELEE,
            (175, 2000),
            0,
            (3300, 3200),
            10_000,
        );
        follower.leader = Some(ME);
        let mut world = stand_field(&clock, vec![master, follower], open_cells(), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut alpha);
        give_reflect(&mut world, 100);
        hit(&mut world, 1, ME);
        // The NORMAL hit shares half the grudge with the follower.
        let shared = world.npc_combat.get(&(ME + 1)).unwrap().damage[&1].aggro;
        assert!(shared > 0, "the NORMAL hit shares");
        world.dice = Pcg32::new(swing_seed(4), 0);
        world.process_pulse(P0 + 1);
        drained(&mut alpha);
        assert_eq!(
            world.npc_combat.get(&(ME + 1)).unwrap().damage[&1].aggro,
            shared,
            "the SPECIAL reflect shares nothing"
        );
        assert_eq!(world.npc_combat.get(&(ME + 1)).unwrap().damage[&1].total, 0);
    }

    /// A reflected kill pays (G2): the monster stuns at 0 with the player as its killer,
    /// and the stun event's `Dead` lays the gold.
    #[test]
    fn a_reflected_kill_pays_gold() {
        use common::enums::EMobRank;
        let clock = TestClock::default();
        clock.set(T0);
        let mut npc = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 1);
        npc.fight.rank = EMobRank::King as u8;
        npc.fight.gold_range = [100, 100];
        npc.exp = 75;
        npc.fight.level = 10;
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut alpha);
        // No player hit: it would kill the 1-HP monster first. The fight begins bare.
        give_reflect(&mut world, 100);
        world.target(Vid::new(1), ME);
        world.begin_fight(ME, Vid::new(1));
        world.dice = Pcg32::new(swing_seed(4), 0);
        world.process_pulse(P0 + 1);
        drained(&mut alpha);
        let stunned = world.npc_by_vid(ME).unwrap();
        assert_eq!((stunned.hp, stunned.stunned), (0, true));
        for pulse in P0 + 2..=P0 + 76 {
            world.process_pulse(pulse);
        }
        let dead = drained(&mut alpha);
        assert!(
            dead.iter()
                .any(|record| record[0] == HEADER_GC_DEAD.value()),
            "GC_DEAD: {dead:02x?}"
        );
        assert!(!world.ground.is_empty(), "the kill laid gold");
        assert_ne!(
            world.targets.of(1),
            Some(ME),
            "Dead read the killer and cleared its target"
        );
    }

    /// A blocked swing reflects nothing (G3): no hit points, no row, no chat draw.
    #[test]
    fn a_blocked_swing_reflects_nothing() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        give_reflect(&mut world, 100);
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .unwrap()
            .items_and_points_mut()
            .1
            .unwrap()
            .point_change(point::POINT_BLOCK, 100, false, false)
            .expect("block is ported");
        world.begin_fight(ME, Vid::new(1));
        drained(&mut alpha);
        let seed = swing_seed(4);
        world.dice = Pcg32::new(seed, 0);
        world.process_pulse(P0 + 1);
        let heard = drained(&mut alpha);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, 10_000);
        assert!(world
            .npc_combat
            .get(&ME)
            .is_none_or(|combat| !combat.damage.contains_key(&1)));
        // The chat, the roll, the floor, the block: the reflect's chat never draws.
        let mut expect = Pcg32::new(seed, 0);
        let _chat = number(&mut expect, 0, 60);
        let _roll = number(&mut expect, 0, 0);
        let _floor = number(&mut expect, 1, 5);
        let _block = number(&mut expect, 1, 100);
        assert_eq!(world.dice, expect, "no reflect draw on a block");
        assert_eq!(heard[0][0], HEADER_GC_DAMAGE_INFO.value());
    }

    /// A `Down` swing still reflects (G4): the monster takes the share, then the player
    /// dies by `Dead`.
    #[test]
    fn a_down_swing_still_reflects_before_the_player_dies() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        give_reflect(&mut world, 100);
        world.stun_pc(Vid::new(1));
        world.begin_fight(ME, Vid::new(1));
        drained(&mut alpha);
        world.dice = Pcg32::new(swing_seed(4), 0);
        world.process_pulse(P0 + 1);
        drained(&mut alpha);
        // The share 4 takes the tenth to 3, as in the `Landed` test.
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, 10_000 - 3);
        assert!(
            world.pc_combat.get(&1).is_some_and(|combat| combat.dead),
            "the stunned player dies by Dead"
        );
    }

    /// The head guard covers the shot too: an `AFF_STUN` RANGE monster never shoots.
    #[test]
    fn a_flag_stunned_range_monster_never_shoots() {
        use crate::game_state::affect::Affect;
        use gamedata::mob_proto::BATTLE_TYPE_RANGE;
        use protocol::gc::HEADER_GC_FLY_TARGETING;
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 406, BATTLE_TYPE_RANGE, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        world.add_npc_affect(
            ME,
            Affect {
                kind: AFFECT_STUN,
                apply_on: 0,
                value: 0,
                flag: AFF_STUN,
                remaining: STUN_TICKS,
                sp_cost: 0,
            },
        );
        drained(&mut alpha);
        world.begin_fight(ME, Vid::new(1));
        world.process_pulse(P0 + 1);
        let heard = drained(&mut alpha);
        assert!(
            heard
                .iter()
                .all(|record| record.first() != Some(&HEADER_GC_FLY_TARGETING)),
            "no shot: {heard:02x?}"
        );
    }

    /// A shot at a reflecting player reflects nothing (G5): only NORMAL reflects.
    #[test]
    fn a_shot_at_a_reflecting_player_reflects_nothing() {
        use gamedata::mob_proto::BATTLE_TYPE_RANGE;
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 406, BATTLE_TYPE_RANGE, (175, 2000), 0, SPOT, 10_000);
        let mut world = stand_field(&clock, vec![npc], open_cells(), true);
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut alpha);
        give_reflect(&mut world, 100);
        world.begin_fight(ME, Vid::new(1));
        world.dice = Pcg32::new(swing_seed(4), 0);
        world.process_pulse(P0 + 1);
        drained(&mut alpha);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, 10_000);
        assert!(world
            .npc_combat
            .get(&ME)
            .is_none_or(|combat| !combat.damage.contains_key(&1)));
    }

    /// The immune third (G8): an `IMMUNE_REFLECT` monster takes a third of the share.
    #[test]
    fn an_immune_monster_takes_a_third_of_the_share() {
        let (_clock, mut world, mut alpha) = a_fight(10_000, NEAR, true);
        give_reflect(&mut world, 100);
        world.npc_mut(ME).unwrap().immune_flag |= world::character::IMMUNE_REFLECT;
        world.begin_fight(ME, Vid::new(1));
        drained(&mut alpha);
        world.dice = Pcg32::new(swing_seed(4), 0);
        world.process_pulse(P0 + 1);
        drained(&mut alpha);
        // `int(4 / 3.0 + 0.5)` is 1; the tenth of 1 is 0 — the runner still runs.
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, 10_000);
        assert!(world
            .npc_combat
            .get(&ME)
            .is_none_or(|combat| !combat.damage.contains_key(&1)));
    }

    /// The damage rates read the quest flags live (G7): `user_dam` doubles the
    /// player's swing, `mob_dam` doubles the monster's.
    #[test]
    fn the_quest_flags_scale_both_sides_damage() {
        let clock = TestClock::default();
        clock.set(T0);
        let mut fighter = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        fighter.fight.damage_range = [30, 30];
        fighter.fight.dam_multiply_bits = 1.0f32.to_bits();
        let mut user_quests = quests();
        user_quests.manager_mut().set_event_flag(b"user_dam", 200);
        let mut mob_quests = quests();
        mob_quests.manager_mut().set_event_flag(b"mob_dam", 200);
        let plain = run_swings(&clock, fighter.clone(), None);
        let user = run_swings(&clock, fighter.clone(), Some(user_quests));
        let mob = run_swings(&clock, fighter.clone(), Some(mob_quests));
        // The player's swing draws only the damage roll: weaponless, no crit or
        // penetrate points, so `user_dam` multiplies the shown number exactly.
        assert_eq!(user.0, plain.0 * 2, "user_dam doubles the player's hit");
        assert_eq!(user.1, plain.1, "user_dam leaves the monster's swing");
        assert_eq!(mob.0, plain.0, "mob_dam leaves the player's hit");
        assert!(mob.1 > plain.1, "mob_dam grows the monster's swing");
    }

    /// The quest runtime over the legacy locale, for the flag tests.
    fn quests() -> super::super::quests::Quests {
        use gamedata::mob_locale_names::MobNamesByLanguage;
        use quest::manager::Manager;
        use std::sync::Arc;
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        super::super::quests::Quests::new(manager, Arc::default(), MobNamesByLanguage::default())
    }

    /// The player's shown hit and the monster's shown swing on a fresh field with
    /// `quests`: the player's hit starts the fight, the seed fixes the dice.
    fn run_swings(
        clock: &TestClock,
        fighter: world::npc::Npc,
        quests: Option<super::super::quests::Quests>,
    ) -> (i32, i32) {
        let mut world = stand_field(clock, vec![fighter], open_cells(), true);
        if let Some(quests) = quests {
            world = world.with_quests(quests);
        }
        let mut alpha = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        let struck = drained(&mut alpha);
        let shown = GcDamageInfo::decode(
            struck
                .iter()
                .find(|record| record[0] == HEADER_GC_DAMAGE_INFO.value())
                .expect("the player's info"),
        )
        .expect("the player's damage")
        .damage;
        world.dice = Pcg32::new(swing_seed(4), 0);
        world.process_pulse(P0 + 1);
        let swung = drained(&mut alpha);
        let back = GcDamageInfo::decode(
            swung
                .iter()
                .find(|record| record[0] == HEADER_GC_DAMAGE_INFO.value())
                .expect("the monster's info"),
        )
        .expect("the monster's damage")
        .damage;
        (shown, back)
    }
}
