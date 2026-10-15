//! A player's hit on a monster and the monster's death: `CInputMain::Attack`
//! (`G/input_main.cpp:1918-2008`), `CHARACTER::Attack` (`G/char_battle.cpp:220-328`),
//! `battle_is_attackable` and `battle_melee_attack` (`G/battle.cpp:99-228`), then `Stun`,
//! `StunEvent`, `Dead` and `dead_event` (`G/char_battle.cpp:402-548`, `:1238-1696`), the
//! non-player's recovery event (`G/char.cpp:3172-3314`) and `SetSyncOwner` with `ClearSync`
//! (`G/char.cpp:5469-5575`).
//!
//! # What legacy does
//!
//! A `CG_ATTACK` of type 0 names its victim by VID. `CHARACTER::Attack` goes on only while
//! `battle_is_attackable` holds: the victim is alive, neither stands on an `ATTR_BANPK` cell, the
//! victim is no fake hydra, and `CPVPManager::CanAttack` lets a player hit any non-player but an
//! NPC, a warp or a goto. `battle_melee_attack` measures `DISTANCE_APPROX` against the reach
//! ([`melee_reach`]); within it the attacker turns to the victim and `battle_hit` runs
//! ([`strike_npc`]): the damage, the victim's `Damage` and what they show. The melee having run,
//! the victim's `SetSyncOwner` makes the attacker its owner, and `OnMove(true)` stamps the attack
//! and ends the attacker's revive-invisible affect.
//!
//! A hit that takes hit points starts the victim's recovery, which every `bRegenCycle` seconds
//! gives back `bRegenPercent` of the maximum, at least 1, until it is full. A hit that leaves
//! none stuns it: its recovery ends, `GC_STUN` goes to its view, and `StunEvent` is due 3 seconds
//! later, with the attacker's PID as its killer. The event runs `Dead`, as a hit on the stunned
//! monster does at once:
//!
//! 1. the killer is the hitter, or else the player whose PID it kept, and the PID is reset;
//! 2. the killer's target is cleared when it is this monster;
//! 3. `ClearSync` tells the view the monster has no owner, and the stun event ends;
//! 4. `GC_DEAD` goes to the view, and `dead_event` is due a second later, which destroys the
//!    monster ([`GameState::destroy_npc`]).
//!
//! A player that leaves releases every character it owns, and each one's view is told
//! (`ClearSync`'s `FuncClearSync`).
//!
//! # Divergences
//!
//! - **The victims (V17a).** Only a monster on the attacker's own Channel and map is hit.
//!   Legacy finds the VID on any map (a Defect, fixed). A player, stone, door, building or horse
//!   and race 5001 are refused with nothing sent until their systems are ported.
//! - **The fight's other halves (V17b-h).** The hit monster does not fight back (`BeginFight`,
//!   aggro, the damage map); `Dead` pays no reward; `NormalAttackAffect` and the gold steal draw
//!   but change nothing; skills and the shoot and fly frames are consumed; the attacker's
//!   `POS_FIGHTING` and victim are not kept; no party revives the monster or is told of it.
//! - **The order of one Pulse's events (V16).** The stun, dead and recovery events ride the
//!   regen schedule ([`super::regen`]).
//! - **The owners a leaving player releases** go in VID order (V1); legacy walks its list in the
//!   order it took them, after the leaving player's own release ([`super::death`]).
//! - **The recovery's share** is computed in 64 bits; legacy's `int` product of the maximum and
//!   the percentage wraps past `INT_MAX` (a Defect not reproduced).
//! - **The world boss's damage map** (`SetAccumulateDamageByVID`), which legacy grows for every
//!   victim a player hits, is not kept (a Defect not reproduced).
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::sync::Arc;
use std::time::Duration;

use common::enums::EWearPositions;
use common::vid::Vid;
use gamedata::mob_proto::{AIFLAG_NOMOVE, CHAR_TYPE_MONSTER};
use protocol::cg_attack::CgAttack;
use protocol::gc_fields::GcCreateFly;
use protocol::gc_inventory::{HEADER_GC_DEAD, HEADER_GC_STUN};
use protocol::gc_nested::GcDamageInfo;
use protocol::gc_vid::{GcHeaderAndDword, GcSpecialEffect};
use world::character::{melee_reach, strike_npc, Equipment, HitEvent, Strike, Striker, Struck};
use world::npc::Npc;

use super::motion::rotation_to_xy;
use super::regen::{pulses, Due};
use super::view::EntityKey;
use super::GameState;
use crate::game_loop_messages::GameCommand;
use crate::loading_phase::point_changes;
use crate::sync_position::{
    distance_approx, judge_sync_ownership, ownership_record, SyncOwnershipOutcome,
    SyncOwnershipState, SyncPositionVictim, SyncPositionVictimKind,
};

/// `DAMAGE_BLOCK` (`G/char.h:134`): the flag `SetSyncOwner` shows an owner it refuses.
pub(super) const DAMAGE_BLOCK: u8 = 1 << 3;

/// `PASSES_PER_SEC(3)`: the stun before `Dead` (`G/char_battle.cpp:458`), a player's too.
pub(super) const STUN_SECONDS: u32 = 3;

/// `PASSES_PER_SEC(1)`: a dead monster no party revives waits this long to go (`:1607`).
const DEAD_SECONDS: u32 = 1;

/// The race whose hit drops gold (`G/char_battle.cpp:1776-1832`), refused until gold is ported.
const GOLD_RACE: u32 = 5001;

/// `IsFakeHydra` (`G/ShipDefense.cpp:1914-1923`): `HYDRA_LEFT` and `HYDRA_RIGHT`, which
/// `battle_is_attackable` refuses to every player.
const FAKE_HYDRAS: [u32; 2] = [3963, 3964];

/// What legacy's `CHARACTER` keeps of a non-player's fight beyond its [`Npc`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct NpcCombat {
    /// `m_dwKillerPID`: the player whose hit stunned it, which `Dead` reads and resets.
    killer_pid: u32,
    /// `m_pkChrSyncOwner` and `m_fSyncTime`.
    pub(super) sync: SyncOwnershipState,
    /// `m_dwLastSyncTime`, which a new owner resets.
    pub(super) last_sync: Option<Duration>,
    /// The Pulse `m_pkStunEvent` is due on.
    stun_due: Option<u64>,
    /// The Pulse `m_pkDeadEvent` is due on.
    dead_due: Option<u64>,
    /// The Pulse `m_pkRecoveryEvent` is due on.
    recovery_due: Option<u64>,
}

/// Whether a player's hit reaches `npc`: a monster but race 5001 (V17a).
fn is_struck(npc: &Npc) -> bool {
    npc.char_type == CHAR_TYPE_MONSTER && npc.vnum != GOLD_RACE
}

/// A record of a header and a VID, as `GC_STUN` and `GC_DEAD` are.
pub(super) fn header_and_vid(header: u8, vid: u32) -> Vec<u8> {
    let mut record = Vec::with_capacity(GcHeaderAndDword::WIRE_SIZE);
    GcHeaderAndDword::new(header, vid).encode_into(&mut record);
    record
}

/// The Pulses between two recoveries of `npc`: `PASSES_PER_SEC(MAX(1, bRegenCycle))`.
fn recovery_pulses(npc: &Npc) -> u64 {
    pulses(u32::from(npc.fight.regen_cycle.max(1)))
}

impl GameState {
    /// Runs one `Target` or `Attack` command; its `settled` is dropped once its records are
    /// queued.
    pub(super) fn apply_pick(&mut self, command: GameCommand) {
        match command {
            GameCommand::Target {
                vid,
                target,
                settled,
            } => {
                self.target(vid, target);
                drop(settled);
            }
            GameCommand::Attack {
                vid,
                record,
                settled,
            } => {
                self.attack(vid, record);
                drop(settled);
            }
            _ => debug_assert!(false, "only target and attack commands reach apply_pick"),
        }
    }

    /// `CInputMain::Attack` of the player under `vid` (`G/input_main.cpp:1918-2008`) and its
    /// `CHARACTER::Attack` with type 0 (`G/char_battle.cpp:220-328`). `CanMove` holds for every
    /// player yet, no castle map is hosted, and the speed-hack and combo gates read keys no
    /// configuration sets.
    pub(super) fn attack(&mut self, vid: Vid, record: CgAttack) {
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return;
        };
        // A skill (`bType > 0`) waits for the skills (V17f).
        if record.attack_type != 0 {
            return;
        }
        let Some(npc) = self
            .npc_on(record.target_vid, place)
            .filter(|npc| is_struck(npc))
        else {
            return;
        };
        // `battle_is_attackable` (`G/battle.cpp:165`), then `battle_melee_attack`'s reach
        // (`:172-210`): beyond it the attack is `BATTLE_NONE` and sends nothing.
        let distance = distance_approx(spot.x.saturating_sub(npc.x), spot.y.saturating_sub(npc.y));
        if !self.is_attackable(vid, (place.1, spot.x, spot.y), npc) || distance > melee_reach(npc) {
            return;
        }
        let victim = SyncPositionVictim {
            vid: Vid::new(npc.vid),
            kind: SyncPositionVictimKind::Monster,
            x: npc.x,
            y: npc.y,
        };
        // `SetRotationToXY` (`:219-223`); `POS_FIGHTING` and the victim are not kept (V17g).
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.rotation = rotation_to_xy(
                victim.x.saturating_sub(spot.x),
                victim.y.saturating_sub(spot.y),
            );
        }
        let target = victim.vid.raw();
        if let Some(strike) = self.strike(vid, target) {
            self.show_strike(vid, target, strike.events);
            match strike.struck {
                Struck::Landed { damage } => self.after_damage(vid, target, damage),
                Struck::Finished => self.dead_npc(target, Some(vid)),
                Struck::Whiffed | Struck::Refused | Struck::AlreadyDead => {}
            }
        }
        // `iRet != BATTLE_NONE`: `SetSyncOwner(this)`; `BeginFight` waits for 229l (V17b).
        let now = self.clock.elapsed();
        let _owner = self.set_npc_sync_owner(vid, (spot.x, spot.y), &victim, now);
        // `BATTLE_DAMAGE` or `BATTLE_DEAD`: `OnMove(true)` (`G/char.cpp:6148-6179`). The victim's
        // `OnMove` holds nothing for a non-player, and no Eunhyung or mining is ported.
        let now = self.clock.now();
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.last_attack = Some(now);
        }
        self.remove_revive_invisible(vid);
    }

    /// The non-player `vid` names on `place`, a Channel and map.
    fn npc_on(&self, vid: u32, place: (u8, i32)) -> Option<&Npc> {
        let &(channel, map, _) = self.npc_of.get(&vid)?;
        if (channel, map) != place {
            return None;
        }
        self.npc_by_vid(vid)
    }

    /// The non-player under `vid`, to change.
    pub(super) fn npc_mut(&mut self, vid: u32) -> Option<&mut Npc> {
        let &(channel, map, at) = self.npc_of.get(&vid)?;
        Arc::make_mut(self.npcs.get_mut(&(channel, map))?)
            .npcs
            .get_mut(at)
    }

    /// `battle_is_attackable` of the player `attacker` standing at `(map, x, y)` on `npc`
    /// (`G/battle.cpp:99-152`): the victim is alive, neither cell is `ATTR_BANPK`, the attacker
    /// is neither stunned nor dead (`:122`), and the victim is no fake hydra. No castle, arena
    /// or ship-defence map is hosted, and `CPVPManager::CanAttack` lets a player hit every
    /// non-player that reaches here.
    fn is_attackable(&self, attacker: Vid, (map, x, y): (i32, i32, i32), npc: &Npc) -> bool {
        !npc.dead
            && !self.is_banpk(map, x, y)
            && !self.is_banpk(map, npc.x, npc.y)
            && !self.is_down(attacker.raw())
            && !FAKE_HYDRAS.contains(&npc.vnum)
    }

    /// `battle_hit` of the player under `vid` on the non-player under `target`.
    fn strike(&mut self, vid: Vid, target: u32) -> Option<Strike> {
        let &(channel, map, at) = self.npc_of.get(&target)?;
        let empire = self.bodies.get(&vid)?.card.empire;
        let character = self.characters.find_by_vid_mut(vid).ok()?;
        let (items, points) = character.items_and_points_mut();
        let points = points?;
        let weapon = Equipment::of(items, &self.protos)
            .wear(EWearPositions::Weapon as u16)
            .map(|worn| worn.proto);
        let npc = Arc::make_mut(self.npcs.get_mut(&(channel, map))?)
            .npcs
            .get_mut(at)?;
        let mut striker = Striker {
            points,
            weapon,
            empire,
            map,
        };
        // `battle_is_attackable` refused a victim on an `ATTR_BANPK` cell, and a standing
        // non-player has a sectree, so `Damage` never refuses it here.
        Some(strike_npc(&mut self.dice, &mut striker, npc, false))
    }

    /// Sends what a hit on the non-player under `victim` showed, in order.
    fn show_strike(&mut self, attacker: Vid, victim: u32, events: Vec<HitEvent>) {
        let around = EntityKey::Npc(victim);
        for event in events {
            match event {
                HitEvent::Effect(effect_type) => {
                    let mut record = Vec::new();
                    GcSpecialEffect {
                        effect_type,
                        vid: victim,
                    }
                    .encode_into(&mut record);
                    self.packet_around(around, &record, None);
                }
                HitEvent::Fly(fly_type) => {
                    let mut record = Vec::new();
                    GcCreateFly::new(fly_type, victim, attacker.raw()).encode_into(&mut record);
                    self.packet_around(around, &record, None);
                }
                HitEvent::Attacker(records) => {
                    for frame in point_changes(&records, attacker.raw()) {
                        let _sent = self.write_to_client(attacker, frame);
                    }
                }
                HitEvent::VictimHp => self.broadcast_npc_target(victim),
                HitEvent::Chat(index) => self.monster_say(victim, index),
                HitEvent::DamageInfo { damage, flag } => {
                    self.send_damage_packet(victim, attacker, damage, flag);
                }
            }
        }
    }

    /// `SendDamagePacket` of the non-player under `victim` (`G/char_battle.cpp:1720-1742`): it
    /// has no desc, so only a player attacker whose target it is gets the record.
    fn send_damage_packet(&self, victim: u32, attacker: Vid, damage: i32, flag: u8) {
        if self.targets.of(attacker.raw()) != Some(victim) {
            return;
        }
        let mut record = Vec::new();
        GcDamageInfo {
            header: GcDamageInfo::header(),
            vid: victim,
            flag,
            damage,
        }
        .encode_into(&mut record);
        let _sent = self.write_to_client(attacker, record);
    }

    /// The end of `Damage` once the hit points are taken (`G/char_battle.cpp:2407-2433`): a hit
    /// that took any starts the recovery, and one that left none stuns the non-player with the
    /// attacker's PID, which is a player's VID, as its killer. The damage map and the aggro wait
    /// for 229l.
    fn after_damage(&mut self, attacker: Vid, victim: u32, damage: i32) {
        if damage > 0 {
            self.start_recovery(victim);
        }
        if self.npc_by_vid(victim).is_some_and(|npc| npc.hp <= 0) {
            self.stun_npc(victim);
            self.npc_combat.entry(victim).or_default().killer_pid = attacker.raw();
        }
    }

    /// `Stun` of the non-player under `vid` (`G/char_battle.cpp:422-459`). No party is told
    /// (V17h); a non-player has no recovery points to zero and no shop to close.
    fn stun_npc(&mut self, vid: u32) {
        if self
            .npc_by_vid(vid)
            .is_none_or(|npc| npc.stunned || npc.dead)
        {
            return;
        }
        let combat = self.npc_combat.entry(vid).or_default();
        self.regens
            .cancel(&mut combat.recovery_due, Due::Recovery(vid));
        self.packet_around(
            EntityKey::Npc(vid),
            &header_and_vid(HEADER_GC_STUN.value(), vid),
            None,
        );
        if let Some(npc) = self.npc_mut(vid) {
            npc.stunned = true;
        }
        let when = self.last_pulse.saturating_add(pulses(STUN_SECONDS));
        let combat = self.npc_combat.entry(vid).or_default();
        if combat.stun_due.is_none() {
            self.regens
                .schedule(&mut combat.stun_due, when, Due::Stun(vid));
        }
    }

    /// `StunEvent` (`G/char_battle.cpp:402-420`): the stunned non-player dies.
    pub(super) fn stun_event(&mut self, vid: u32) {
        if let Some(combat) = self.npc_combat.get_mut(&vid) {
            combat.stun_due = None;
        }
        self.dead_npc(vid, None);
    }

    /// `Dead` of the non-player under `vid` (`G/char_battle.cpp:1238-1696`), killed by `killer`
    /// or else by the player whose PID it kept. The battle pass, the reward missions and the
    /// duel, arena and siege hooks hold nothing for a monster a player kills; `Reward` waits for
    /// drops and exp (V17c).
    fn dead_npc(&mut self, vid: u32, killer: Option<Vid>) {
        if self.npc_by_vid(vid).is_none_or(|npc| npc.dead) {
            return;
        }
        // `m_dwKillerPID = 0`, which legacy says never to drop (`:1261`).
        let pid = self
            .npc_combat
            .get_mut(&vid)
            .map_or(0, |combat| std::mem::take(&mut combat.killer_pid));
        // `FindByPID` finds the player only while it is online, and a player that left has no
        // target left to clear.
        let killer = killer.or((pid != 0).then_some(Vid::new(pid)));
        if let Some(killer) = killer {
            if self.targets.of(killer.raw()) == Some(vid) {
                self.target(killer, 0);
            }
        }
        // `SetPosition(POS_DEAD)`; `ClearAffect` finds nothing on a non-player.
        if let Some(npc) = self.npc_mut(vid) {
            npc.dead = true;
        }
        // `ClearSync` (`:1461`), then the stun event ends (`:1464`).
        self.release_npc_sync(vid);
        let combat = self.npc_combat.entry(vid).or_default();
        self.regens.cancel(&mut combat.stun_due, Due::Stun(vid));
        self.packet_around(
            EntityKey::Npc(vid),
            &header_and_vid(HEADER_GC_DEAD.value(), vid),
            None,
        );
        if let Some(npc) = self.npc_mut(vid) {
            npc.stunned = false;
        }
        let when = self.last_pulse.saturating_add(pulses(DEAD_SECONDS));
        let combat = self.npc_combat.entry(vid).or_default();
        self.regens.cancel(&mut combat.dead_due, Due::Dead(vid));
        self.regens
            .schedule(&mut combat.dead_due, when, Due::Dead(vid));
    }

    /// `dead_event` of a monster (`G/char_battle.cpp:473-548`): no party revives it (V17h), so
    /// `M2_DESTROY_CHARACTER`.
    pub(super) fn dead_event(&mut self, vid: u32) {
        if let Some(combat) = self.npc_combat.get_mut(&vid) {
            combat.dead_due = None;
        }
        self.destroy_npc(vid);
    }

    /// `StartRecoveryEvent` of the non-player under `vid` (`G/char.cpp:3287-3314`): none while
    /// one runs, it is dead or stunned, or its hit points are full. The Meley statue and the
    /// ship's mast stand on no hosted map.
    fn start_recovery(&mut self, vid: u32) {
        let Some(npc) = self.npc_by_vid(vid) else {
            return;
        };
        if npc.dead || npc.stunned || npc.hp >= npc.max_hp {
            return;
        }
        let when = self.last_pulse.saturating_add(recovery_pulses(npc));
        let combat = self.npc_combat.entry(vid).or_default();
        if combat.recovery_due.is_none() {
            self.regens
                .schedule(&mut combat.recovery_due, when, Due::Recovery(vid));
        }
    }

    /// `recovery_event` of a non-player (`G/char.cpp:3191-3240`): `bRegenPercent` of its
    /// maximum, at least 1 and held to the maximum, shown to its targeters; again `bRegenCycle`
    /// seconds on until it is full. No affect poisons a non-player, no regen stands race 2493,
    /// and no door is hit.
    pub(super) fn recovery_event(&mut self, vid: u32) {
        if let Some(combat) = self.npc_combat.get_mut(&vid) {
            combat.recovery_due = None;
        }
        let Some(npc) = self.npc_mut(vid) else {
            return;
        };
        // `PointChange(POINT_HP)` changes nothing on a dead or stunned character
        // (`G/char.cpp:4250-4251`).
        let changed = !npc.dead && !npc.stunned;
        if changed {
            let share = i64::from(npc.max_hp) * i64::from(npc.fight.regen_percent) / 100;
            let room = i64::from(npc.max_hp) - i64::from(npc.hp);
            let amount = share.max(1).min(room);
            npc.hp = i32::try_from(i64::from(npc.hp) + amount).unwrap_or(npc.max_hp);
        }
        let full = npc.hp >= npc.max_hp;
        let cycle = recovery_pulses(npc);
        if changed {
            self.broadcast_npc_target(vid);
        }
        if full {
            return;
        }
        let when = self.last_pulse.saturating_add(cycle);
        let combat = self.npc_combat.entry(vid).or_default();
        self.regens
            .schedule(&mut combat.recovery_due, when, Due::Recovery(vid));
    }

    /// `SetSyncOwner(owner)` of the non-player `victim` (`G/char.cpp:5469-5557`), its owner
    /// standing at `owner_at`: refused under `AIFLAG_NOMOVE`, refused with a `DAMAGE_BLOCK` when
    /// `battle_is_attackable` does not hold, then judged by [`judge_sync_ownership`]. An
    /// accepted claim is told to the victim's view.
    pub(super) fn set_npc_sync_owner(
        &mut self,
        owner: Vid,
        owner_at: (i32, i32),
        victim: &SyncPositionVictim,
        now: Duration,
    ) -> bool {
        let raw = victim.vid.raw();
        let Some(&(_, map, _)) = self.npc_of.get(&raw) else {
            return false;
        };
        let Some(npc) = self.npc_by_vid(raw) else {
            return false;
        };
        if npc.ai_flags & AIFLAG_NOMOVE != 0 {
            return false;
        }
        if !self.is_attackable(owner, (map, owner_at.0, owner_at.1), npc) {
            self.send_damage_packet(raw, owner, 0, DAMAGE_BLOCK);
            return false;
        }
        let state = self
            .npc_combat
            .get(&raw)
            .map_or_else(SyncOwnershipState::default, |combat| combat.sync);
        match judge_sync_ownership(owner, victim, state, owner_at.0, owner_at.1, now) {
            SyncOwnershipOutcome::Refused => false,
            SyncOwnershipOutcome::Kept => true,
            SyncOwnershipOutcome::Accepted {
                owner_changed,
                record,
            } => {
                let combat = self.npc_combat.entry(raw).or_default();
                if owner_changed {
                    combat.last_sync = None;
                }
                combat.sync = SyncOwnershipState {
                    owner: Some(owner),
                    claimed_at: Some(now),
                };
                self.packet_around(EntityKey::Npc(raw), &record, None);
                true
            }
        }
    }

    /// `ClearSync` of the non-player under `vid` (`G/char.cpp:5568-5575`): `SetSyncOwner(NULL)`
    /// tells its view it has no owner, unless `AIFLAG_NOMOVE` holds it; the claim's time stays.
    /// A non-player owns no one.
    pub(super) fn release_npc_sync(&mut self, vid: u32) {
        if self
            .npc_by_vid(vid)
            .is_none_or(|npc| npc.ai_flags & AIFLAG_NOMOVE != 0)
        {
            return;
        }
        if let Some(combat) = self.npc_combat.get_mut(&vid) {
            combat.sync.owner = None;
        }
        let record = ownership_record(Vid::new(0), Vid::new(vid));
        self.packet_around(EntityKey::Npc(vid), &record, None);
    }

    /// `ClearSync`'s `FuncClearSync` for the player under `vid` (`G/char.cpp:5559-5575`): each
    /// character it owns has no owner, which that one's view is told, in VID order (V1). The
    /// player's own release is [`GameState::clear_sync`]'s.
    pub(super) fn release_owned(&mut self, vid: Vid) {
        let mut owned: Vec<EntityKey> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.sync.owner == Some(vid))
            .map(|(victim, _)| EntityKey::Character(victim.raw()))
            .chain(
                self.npc_combat
                    .iter()
                    .filter(|(_, combat)| combat.sync.owner == Some(vid))
                    .map(|(&victim, _)| EntityKey::Npc(victim)),
            )
            .collect();
        owned.sort_unstable();
        for key in owned {
            let victim = match key {
                EntityKey::Character(victim) => {
                    if let Some(body) = self.bodies.get_mut(&Vid::new(victim)) {
                        body.sync.owner = None;
                    }
                    victim
                }
                EntityKey::Npc(victim) => {
                    if let Some(combat) = self.npc_combat.get_mut(&victim) {
                        combat.sync.owner = None;
                    }
                    victim
                }
                EntityKey::Ground(_) => continue,
            };
            let record = ownership_record(Vid::new(0), Vid::new(victim));
            self.packet_around(key, &record, None);
        }
    }

    /// What a destroyed non-player's fight leaves: its stun, dead and recovery events end
    /// (`G/char.cpp:713-718`), and nothing of it is kept.
    pub(super) fn forget_npc_fight(&mut self, vid: u32) {
        let Some(mut combat) = self.npc_combat.remove(&vid) else {
            return;
        };
        self.regens.cancel(&mut combat.stun_due, Due::Stun(vid));
        self.regens.cancel(&mut combat.dead_due, Due::Dead(vid));
        self.regens
            .cancel(&mut combat.recovery_due, Due::Recovery(vid));
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;
    use std::sync::Arc;

    use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
    use common::point_slot as point;
    use gamedata::item_kind::{ITEM_WEAPON, WEAPON_BOW};
    use gamedata::mob_group::MobGroups;
    use gamedata::mob_locale_names::{MobLocaleNames, MobNamesByLanguage};
    use gamedata::mob_proto::{
        MobProtos, CHAR_TYPE_GOTO, CHAR_TYPE_HORSE, CHAR_TYPE_NPC, CHAR_TYPE_STONE, CHAR_TYPE_WARP,
    };
    use gamedata::records::MobTableRecord;
    use gamedata::server_attr::{
        CellBlock, SectreeGrid, ServerAttr, ATTR_BANPK, CELL_SIZE, SECTREE_CELLS,
    };
    use protocol::cg_variable::{SyncPositionElement, SyncPositionPacket};
    use protocol::gc_actors::GcCharacterAdd;
    use protocol::gc_chat::{GcChat, CHAT_TYPE_TALKING};
    use protocol::item_pos::ItemPos;
    use quest::manager::Manager;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::cells::{HostedCells, MapCells};
    use world::character::{
        calc_melee_damage, Pcg32, Points, DAMAGE_CRITICAL, DAMAGE_NORMAL, FLY_HP_SMALL, SE_CRITICAL,
    };
    use world::item::Item;
    use world::npc::{MapNpcs, MobFight, NpcVids, Respawner, FIRST_NPC_VID};

    use super::super::fixtures::{
        a_world, enter, enter_seeing, enter_seeing_with, open_cells, region, TestClock, PLACE,
    };
    use super::super::quests::Quests;
    use super::super::target::target_record;
    use super::super::view_encode::remove_record;
    use super::*;
    use crate::game_loop::PulseProcessor;
    use crate::sync_position::SYNC_POSITION_PREFIX_SIZE;

    /// The monster every test hits.
    const ME: u32 = FIRST_NPC_VID;

    /// Where it stands.
    const SPOT: (i32, i32) = (3200, 3200);

    /// 100 west of it: within the reach and the claim's 250.
    const NEAR: (i32, i32) = (3100, 3200);

    /// 100 east of it, where a second player stands.
    const EAST: (i32, i32) = (3300, 3200);

    /// When the tests' first Pulse runs, in milliseconds.
    const T0: u32 = 1_000;

    /// The tests' first Pulse, which every due counts from.
    const P0: u64 = 1;

    /// The monster's maximum hit points.
    const MAX_HP: i32 = 1_000;

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// A monster of race 101 (no chat lines) at [`SPOT`] with `hp` of [`MAX_HP`], giving back
    /// 1 % every 6 seconds.
    fn a_monster(hp: i32) -> Npc {
        let mut fight = MobFight::default();
        fight.regen_cycle = 6;
        fight.regen_percent = 1;
        Npc {
            vid: ME,
            vnum: 101,
            race: 101,
            char_type: CHAR_TYPE_MONSTER,
            x: SPOT.0,
            y: SPOT.1,
            hp,
            max_hp: MAX_HP,
            fight,
            ..Npc::default()
        }
    }

    /// A world on `clock` with `npcs` on [`PLACE`] over `cells`, where no monster wanders.
    fn a_field_on(clock: &TestClock, npcs: Vec<Npc>, cells: Arc<HostedCells>) -> GameState {
        let mut world = a_world(clock).with_no_wander(true);
        let respawner = Respawner::new(
            Arc::new(MobProtos::from_rows(Vec::new())),
            Arc::new(MobGroups::default()),
            Arc::new(MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap()),
            NpcVids::starting_at(FIRST_NPC_VID + 100),
        );
        world.install_respawner(respawner, cells);
        world.stand_npcs(
            PLACE.0,
            &region(),
            MapNpcs {
                npcs,
                positions: Vec::new(),
                regens: Vec::new(),
            },
        );
        world
    }

    /// `npc` on open cells and the player 1 at `at`, after Pulse [`P0`] at [`T0`]; the inbox
    /// is the player's, drained. The player is stored above its maximum stamina, so no refill
    /// reaches it.
    fn a_fight(npc: Npc, at: (i32, i32)) -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        clock.set(T0);
        let mut world = a_field_on(&clock, vec![npc], open_cells());
        let mut inbox = enter(&mut world, 1, at, 12_345);
        world.process_pulse(P0);
        drained(&mut inbox);
        (clock, world, inbox)
    }

    /// A map hosted at `place`, a Channel and map, with a monster of 1 hit point under `vid` at
    /// [`SPOT`].
    fn a_monster_on(world: &mut GameState, place: (u8, i32), vid: u32) {
        let mut map = region();
        map.index = place.1;
        world.host_map(
            place.0,
            place.1,
            SectreeGrid {
                x: 0,
                y: 0,
                columns: 10,
                rows: 10,
            },
        );
        let mut npc = a_monster(1);
        npc.vid = vid;
        world.stand_npcs(
            place.0,
            &map,
            MapNpcs {
                npcs: vec![npc],
                positions: Vec::new(),
                regens: Vec::new(),
            },
        );
    }

    /// The player `vid` entered at `at`, its inbox drained.
    fn a_player(world: &mut GameState, vid: u32, at: (i32, i32)) -> UnboundedReceiver<Vec<u8>> {
        let mut inbox = enter(world, vid, at, 12_345);
        drained(&mut inbox);
        inbox
    }

    /// The player under `vid` hits `target` with a type 0 `CG_ATTACK`.
    fn hit(world: &mut GameState, vid: u32, target: u32) {
        world.attack(Vid::new(vid), CgAttack::new(0, target, 0, 0));
    }

    fn points_of(world: &mut GameState, vid: u32) -> &mut Points {
        world
            .characters
            .find_by_vid_mut(Vid::new(vid))
            .unwrap()
            .items_and_points_mut()
            .1
            .unwrap()
    }

    /// The damage the unarmed player under `vid` deals [`ME`] with the world's next draws:
    /// `CalcMeleeDamage`, then 9/10, as the fixture's empire 1 stands on map 41 of empire 3
    /// (`G/char_battle.cpp:2196-2203`).
    fn expected_damage(world: &mut GameState, vid: u32) -> i32 {
        let mut dice = world.dice.clone();
        let points = points_of(world, vid).clone();
        let npc = world.npc_by_vid(ME).unwrap();
        calc_melee_damage(&mut dice, &points, None, npc) * 9 / 10
    }

    fn damage_info(damage: i32, flag: u8) -> Vec<u8> {
        let mut record = Vec::new();
        GcDamageInfo {
            header: GcDamageInfo::header(),
            vid: ME,
            flag,
            damage,
        }
        .encode_into(&mut record);
        record
    }

    fn stun_record() -> Vec<u8> {
        header_and_vid(HEADER_GC_STUN.value(), ME)
    }

    fn dead_record() -> Vec<u8> {
        header_and_vid(HEADER_GC_DEAD.value(), ME)
    }

    /// `GC_OWNERSHIP` of [`ME`] to the player under `owner`, or to none for 0.
    fn owner(owner: u32) -> Vec<u8> {
        ownership_record(Vid::new(owner), Vid::new(ME))
    }

    /// `GC_TARGET` of [`ME`] at `hp` of [`MAX_HP`].
    fn target_at(hp: i32) -> Vec<u8> {
        let percent = u8::try_from(hp * 100 / MAX_HP).unwrap();
        target_record(ME, (percent, hp, MAX_HP), 0)
    }

    fn no_target() -> Vec<u8> {
        target_record(0, (0, 0, 0), 0)
    }

    fn nothing() -> Vec<Vec<u8>> {
        Vec::new()
    }

    /// One `CG_SYNC_POSITION` element moving `vid` to `at`.
    fn sync_of(vid: u32, at: (i32, i32)) -> SyncPositionPacket {
        SyncPositionPacket {
            declared_size: SYNC_POSITION_PREFIX_SIZE + 12,
            elements: vec![SyncPositionElement {
                vid,
                x: at.0,
                y: at.1,
            }],
        }
    }

    /// A lethal hit by a targeting attacker: the damage before the clamp, the new hit points,
    /// the stun to the view, then the claim (`G/char_battle.cpp:2338-2433`,
    /// `G/char.cpp:5549-5555`). The PID is kept for `Dead`, the stun ends 3 seconds on, and the
    /// recovery the hit started is cancelled.
    #[test]
    fn a_lethal_hit_stuns_the_monster() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(1), NEAR);
        world.target(Vid::new(1), ME);
        assert_eq!(drained(&mut alpha), [target_at(1)]);
        let damage = expected_damage(&mut world, 1);
        assert!(damage > 1, "the damage is shown before the clamp");
        hit(&mut world, 1, ME);
        assert_eq!(
            drained(&mut alpha),
            [
                damage_info(damage, DAMAGE_NORMAL),
                target_at(0),
                stun_record(),
                owner(1)
            ]
        );
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!((npc.hp, npc.stunned, npc.dead), (0, true, false));
        let combat = world.npc_combat[&ME];
        assert_eq!(combat.killer_pid, 1);
        assert_eq!(combat.stun_due, Some(P0 + 75));
        assert_eq!((combat.recovery_due, combat.dead_due), (None, None));
    }

    /// `StunEvent` at 3 seconds runs `Dead`: the killer's target is cleared, the view is told
    /// the monster has no owner and is dead, and the PID is reset. A second later `dead_event`
    /// destroys it: no owner again, then its removal (`G/char_battle.cpp:402-548`, `:1238-1696`,
    /// `G/char.cpp:653-789`).
    #[test]
    fn the_stun_ends_in_death_and_the_death_in_the_monsters_end() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(1), NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        assert_eq!(drained(&mut beta), [stun_record(), owner(1)]);
        world.process_pulse(P0 + 74);
        assert_eq!(drained(&mut alpha), nothing());
        world.process_pulse(P0 + 75);
        assert_eq!(drained(&mut alpha), [no_target(), owner(0), dead_record()]);
        assert_eq!(drained(&mut beta), [owner(0), dead_record()]);
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!((npc.stunned, npc.dead), (false, true));
        assert_eq!(world.targets.of(1), None);
        let combat = world.npc_combat[&ME];
        assert_eq!(combat.killer_pid, 0);
        assert_eq!(combat.sync.owner, None);
        assert_eq!((combat.stun_due, combat.dead_due), (None, Some(P0 + 100)));
        // A dead monster is no victim: nothing is sent and nothing drawn.
        let dice = world.dice.clone();
        hit(&mut world, 1, ME);
        assert_eq!(world.dice, dice);
        assert_eq!(drained(&mut alpha), nothing());
        world.process_pulse(P0 + 99);
        assert_eq!(drained(&mut alpha), nothing());
        world.process_pulse(P0 + 100);
        assert_eq!(drained(&mut alpha), [owner(0), remove_record(ME)]);
        assert_eq!(drained(&mut beta), [owner(0), remove_record(ME)]);
        assert!(world.npc_by_vid(ME).is_none());
        assert!(world.minds.get(ME).is_none());
        assert!(!world.npc_combat.contains_key(&ME));
    }

    /// A hit on the stunned monster runs `Dead` at once (`G/char_battle.cpp:2248-2252`): no
    /// damage is shown, and the dead monster refuses the claim with nothing sent, as `Dead`
    /// cleared the target its `DAMAGE_BLOCK` would go to. The stun's own death never comes.
    #[test]
    fn a_hit_on_a_stunned_monster_kills_it_at_once() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(1), NEAR);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        assert_eq!(drained(&mut alpha), [no_target(), owner(0), dead_record()]);
        let combat = world.npc_combat[&ME];
        assert_eq!(combat.killer_pid, 0);
        assert_eq!((combat.stun_due, combat.dead_due), (None, Some(P0 + 25)));
        world.process_pulse(P0 + 25);
        assert_eq!(drained(&mut alpha), [owner(0), remove_record(ME)]);
        world.process_pulse(P0 + 75);
        assert_eq!(drained(&mut alpha), nothing());
    }

    /// The damage goes to an attacker that targets the victim and to no one else; the hit
    /// points to every player that targets it; the stun, the death and the claims to its whole
    /// view (`G/char_battle.cpp:1720-1742`, `G/char.cpp:6600-6661`). The killer that does not
    /// target it has no target cleared; a targeter is cleared as the monster goes.
    #[test]
    fn each_record_of_a_hit_goes_to_its_own_receivers() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(1), NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        let mut gamma = a_player(&mut world, 3, (3200, 3300));
        world.target(Vid::new(2), ME);
        drained(&mut alpha);
        drained(&mut beta);
        drained(&mut gamma);
        hit(&mut world, 1, ME);
        assert_eq!(drained(&mut alpha), [stun_record(), owner(1)]);
        assert_eq!(drained(&mut beta), [target_at(0), stun_record(), owner(1)]);
        assert_eq!(drained(&mut gamma), [stun_record(), owner(1)]);
        world.process_pulse(P0 + 75);
        assert_eq!(drained(&mut alpha), [owner(0), dead_record()]);
        assert_eq!(drained(&mut beta), [owner(0), dead_record()]);
        assert_eq!(drained(&mut gamma), [owner(0), dead_record()]);
        world.process_pulse(P0 + 100);
        assert_eq!(drained(&mut alpha), [owner(0), remove_record(ME)]);
        assert_eq!(
            drained(&mut beta),
            [owner(0), no_target(), remove_record(ME)]
        );
        assert_eq!(drained(&mut gamma), [owner(0), remove_record(ME)]);
    }

    /// The killer's PID finds the player only while it is online (`FindByPID`): one that left
    /// before the stun ended has nothing cleared, and the view is still told.
    #[test]
    fn a_killer_that_left_is_not_found() {
        let (_clock, mut world, _alpha) = a_fight(a_monster(1), NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        let _kept = world.leave_world(Vid::new(1));
        assert_eq!(
            drained(&mut beta),
            [
                stun_record(),
                owner(1),
                ownership_record(Vid::new(0), Vid::new(1)),
                owner(0),
                remove_record(1)
            ]
        );
        world.process_pulse(P0 + 75);
        assert_eq!(drained(&mut beta), [owner(0), dead_record()]);
        assert_eq!(world.npc_combat[&ME].killer_pid, 0);
    }

    /// A hit that takes hit points starts the recovery: 1 % of the maximum every 6 seconds,
    /// shown to the targeters, until the hit points are full (`G/char.cpp:3191-3240`,
    /// `:3287-3314`).
    #[test]
    fn a_hit_monster_recovers_until_it_is_full() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        let damage = expected_damage(&mut world, 1);
        hit(&mut world, 1, ME);
        assert_eq!(
            drained(&mut alpha),
            [
                damage_info(damage, DAMAGE_NORMAL),
                target_at(MAX_HP - damage),
                owner(1)
            ]
        );
        assert_eq!(world.npc_combat[&ME].recovery_due, Some(P0 + 150));
        let mut hp = MAX_HP - damage;
        let mut pulse = P0;
        while hp < MAX_HP {
            world.process_pulse(pulse + 149);
            assert_eq!(drained(&mut alpha), nothing());
            pulse += 150;
            world.process_pulse(pulse);
            hp = (hp + 10).min(MAX_HP);
            assert_eq!(drained(&mut alpha), [target_at(hp)]);
            assert_eq!(world.npc_by_vid(ME).unwrap().hp, hp);
        }
        assert_eq!(world.npc_combat[&ME].recovery_due, None);
        world.process_pulse(pulse + 150);
        assert_eq!(drained(&mut alpha), nothing());
    }

    /// A share below 1 gives back 1 (`MAX(1, ...)`), and a cycle of 0 waits a second
    /// (`MAX(1, bRegenCycle)`).
    #[test]
    fn a_recovery_gives_back_at_least_one_every_second_at_least() {
        let mut npc = a_monster(MAX_HP);
        npc.max_hp = 99;
        npc.hp = 99;
        npc.fight.regen_cycle = 0;
        let (_clock, mut world, _alpha) = a_fight(npc, NEAR);
        hit(&mut world, 1, ME);
        let hp = world.npc_by_vid(ME).unwrap().hp;
        assert!(hp < 98, "the hit leaves room for two");
        assert_eq!(world.npc_combat[&ME].recovery_due, Some(P0 + 25));
        world.process_pulse(P0 + 25);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, hp + 1);
        assert_eq!(world.npc_combat[&ME].recovery_due, Some(P0 + 50));
    }

    /// `Stun` cancels the recovery a hit started (`G/char_battle.cpp:440`), and nothing gives
    /// a stunned monster its hit points back.
    #[test]
    fn a_stun_ends_the_recovery() {
        let (_clock, mut world, _alpha) = a_fight(a_monster(MAX_HP), NEAR);
        hit(&mut world, 1, ME);
        assert!(world.npc_combat[&ME].recovery_due.is_some());
        world.npc_mut(ME).unwrap().hp = 1;
        hit(&mut world, 1, ME);
        assert_eq!(world.npc_combat[&ME].recovery_due, None);
        assert!(!world.regens.holds(Due::Recovery(ME)));
        world.process_pulse(P0 + 150);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, 0);
    }

    /// A hit that takes a single hit point starts the recovery too (`dam > 0`,
    /// `G/char_battle.cpp:2407-2421`). A monster whose defence outweighs the punch takes the
    /// low roll (`CalcBattleDamage`, `G/battle.cpp:244-251`), here a 2, which the foreign map's
    /// 9/10 makes 1.
    #[test]
    fn a_hit_of_one_point_starts_the_recovery() {
        let mut npc = a_monster(MAX_HP);
        npc.fight = MobFight::of(&MobTableRecord {
            def: u16::MAX,
            regen_cycle: 6,
            regen_percent: 1,
            ..MobTableRecord::default()
        });
        let (_clock, mut world, _alpha) = a_fight(npc, NEAR);
        let seed = (0..10_000)
            .find(|&seed| {
                world.dice = Pcg32::new(seed, 0);
                expected_damage(&mut world, 1) == 1
            })
            .expect("a seed in 10,000");
        world.dice = Pcg32::new(seed, 0);
        hit(&mut world, 1, ME);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, MAX_HP - 1);
        assert!(world.npc_combat[&ME].recovery_due.is_some());
    }

    /// `Dead` takes the hitter for its killer before the player whose PID the stun kept
    /// (`G/char_battle.cpp:1265-1276`): beta's hit on the monster alpha stunned clears beta's
    /// target, and alpha keeps its own.
    #[test]
    fn the_hitter_of_a_stunned_monster_is_its_killer() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(1), NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        world.target(Vid::new(1), ME);
        world.target(Vid::new(2), ME);
        hit(&mut world, 1, ME);
        assert_eq!(world.npc_combat[&ME].killer_pid, 1);
        drained(&mut alpha);
        drained(&mut beta);
        hit(&mut world, 2, ME);
        assert_eq!(drained(&mut beta), [no_target(), owner(0), dead_record()]);
        assert_eq!(drained(&mut alpha), [owner(0), dead_record()]);
        assert_eq!(world.targets.of(1), Some(ME));
        assert_eq!(world.targets.of(2), None);
    }

    /// A critical hit shows `SE_CRITICAL` to the whole view and doubles the damage, and a
    /// monster that drains spell points takes them from the attacker, which alone is shown
    /// its points (`G/char_battle.cpp:1990-2010`, `:2305-2316`).
    #[test]
    fn a_critical_hit_and_a_drain_show_what_they_did() {
        let mut npc = a_monster(MAX_HP);
        npc.fight.drain_sp = 5;
        let (_clock, mut world, mut alpha) = a_fight(npc, NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        // `APPLY_CRITICAL_PCT`.
        let _records = points_of(&mut world, 1).apply_point(15, 100);
        assert_eq!(
            points_of(&mut world, 1).get_point(point::POINT_CRITICAL_PCT),
            100
        );
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        let damage = expected_damage(&mut world, 1) * 2;
        let mut drained_points = points_of(&mut world, 1).clone();
        let drain = drained_points
            .point_change(point::POINT_SP, -5, false, false)
            .unwrap();
        let mut effect = Vec::new();
        GcSpecialEffect {
            effect_type: SE_CRITICAL,
            vid: ME,
        }
        .encode_into(&mut effect);
        hit(&mut world, 1, ME);
        let mut expected = vec![effect.clone()];
        expected.extend(point_changes(&drain, 1));
        expected.extend([
            damage_info(damage, DAMAGE_NORMAL | DAMAGE_CRITICAL),
            target_at(MAX_HP - damage),
            owner(1),
        ]);
        assert_eq!(drained(&mut alpha), expected);
        assert_eq!(drained(&mut beta), [effect, owner(1)]);
        assert_eq!(
            points_of(&mut world, 1).sp(),
            drained_points.sp(),
            "the drain is taken"
        );
    }

    /// What a hit shows goes where legacy sends it: an effect and a fly to the victim's view, a
    /// point record to the attacker alone, and a line the monster says to its view as a talk.
    #[test]
    fn what_a_hit_shows_goes_where_legacy_sends_it() {
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        let quests = Quests::new(manager, Arc::default(), MobNamesByLanguage::default());
        let mut npc = a_monster(MAX_HP);
        npc.race = 301;
        let clock = TestClock::default();
        clock.set(T0);
        let mut world = a_field_on(&clock, vec![npc], open_cells()).with_quests(quests);
        let mut alpha = a_player(&mut world, 1, NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        drained(&mut alpha);
        let records = points_of(&mut world, 1)
            .point_change(point::POINT_SP, -5, false, false)
            .unwrap();
        let mine = point_changes(&records, 1);
        let mut line = world
            .quests
            .as_ref()
            .unwrap()
            .manager()
            .host()
            .monster_chat_line(301, 1);
        assert!(!line.is_empty());
        line.push(0);
        let said = GcChat {
            chat_type: CHAT_TYPE_TALKING,
            id: ME,
            empire: 0,
            can_format: true,
            text: line,
        }
        .encode()
        .unwrap();
        let mut fly = Vec::new();
        GcCreateFly::new(FLY_HP_SMALL, ME, 1).encode_into(&mut fly);
        world.show_strike(
            Vid::new(1),
            ME,
            vec![
                HitEvent::Fly(FLY_HP_SMALL),
                HitEvent::Attacker(records),
                HitEvent::Chat(1),
            ],
        );
        let mut expected = vec![fly.clone()];
        expected.extend(mine);
        expected.push(said.clone());
        assert_eq!(drained(&mut alpha), expected);
        assert_eq!(drained(&mut beta), [fly, said]);
    }

    /// Only a monster standing on the attacker's Channel and map is hit (V17a): any other
    /// kind of non-player, race 5001, a fake hydra (`G/battle.cpp:134-139`), a player, the
    /// attacker itself, a monster on another Channel or on another map of this one, a VID that
    /// names nothing and a skill's type are refused before any draw, with nothing sent.
    #[test]
    fn only_a_monster_is_hit() {
        let refused = [
            (CHAR_TYPE_NPC, 101),
            (CHAR_TYPE_STONE, 8001),
            (CHAR_TYPE_WARP, 101),
            // `CHAR_TYPE_DOOR` and `CHAR_TYPE_BUILDING`.
            (4, 101),
            (5, 101),
            (CHAR_TYPE_HORSE, 101),
            (CHAR_TYPE_GOTO, 101),
            (CHAR_TYPE_MONSTER, 5001),
            (CHAR_TYPE_MONSTER, 3963),
            (CHAR_TYPE_MONSTER, 3964),
        ];
        for (char_type, vnum) in refused {
            let mut npc = a_monster(1);
            npc.char_type = char_type;
            npc.vnum = vnum;
            let (_clock, mut world, mut alpha) = a_fight(npc, NEAR);
            world.target(Vid::new(1), ME);
            drained(&mut alpha);
            let dice = world.dice.clone();
            hit(&mut world, 1, ME);
            assert_eq!(
                drained(&mut alpha),
                nothing(),
                "type {char_type}, vnum {vnum}"
            );
            assert_eq!(world.dice, dice, "type {char_type}, vnum {vnum}");
            assert_eq!(world.npc_by_vid(ME).unwrap().hp, 1);
        }
        // Control: race 3962 is a hydra, not a fake one, and is hit.
        let mut npc = a_monster(1);
        npc.vnum = 3962;
        let (_clock, mut world, _alpha) = a_fight(npc, NEAR);
        hit(&mut world, 1, ME);
        assert!(world.npc_by_vid(ME).unwrap().stunned);

        let (_clock, mut world, mut alpha) = a_fight(a_monster(1), NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        a_monster_on(&mut world, (2, PLACE.1), ME + 1);
        // The same Channel's next map, with a monster at the same point.
        a_monster_on(&mut world, (PLACE.0, PLACE.1 + 1), ME + 2);
        drained(&mut alpha);
        for (attack_type, target) in [
            (0, 1),
            (0, 2),
            (0, ME + 1),
            (0, ME + 2),
            (0, ME + 50),
            (1, ME),
            (255, ME),
        ] {
            let dice = world.dice.clone();
            world.attack(Vid::new(1), CgAttack::new(attack_type, target, 0, 0));
            assert_eq!(world.dice, dice, "type {attack_type} on {target}");
            assert_eq!(
                drained(&mut alpha),
                nothing(),
                "type {attack_type} on {target}"
            );
            assert_eq!(
                drained(&mut beta),
                nothing(),
                "type {attack_type} on {target}"
            );
        }
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, 1);
        assert_eq!(world.npc_by_vid(ME + 1).unwrap().hp, 1);
        assert_eq!(world.npc_by_vid(ME + 2).unwrap().hp, 1);
    }

    /// The reach (`G/battle.cpp:172-210`) and the claim's range (`G/char.cpp:5508-5519`) are
    /// both `DISTANCE_APPROX`: a hit lands up to the reach and draws nothing past it, and only
    /// an attacker within 250 becomes the owner.
    #[test]
    fn the_reach_and_the_claims_range_are_approximate_distances() {
        assert_eq!(distance_approx(365, 0), 350);
        assert_eq!(distance_approx(366, 0), 351);
        assert_eq!(distance_approx(261, 0), 250);
        assert_eq!(distance_approx(262, 0), 251);
        assert_eq!(melee_reach(&a_monster(1)), 350);
        for (west, lands, owns) in [
            (366, false, false),
            (365, true, false),
            (262, true, false),
            (261, true, true),
        ] {
            let (_clock, mut world, mut alpha) =
                a_fight(a_monster(MAX_HP), (SPOT.0 - west, SPOT.1));
            world.target(Vid::new(1), ME);
            drained(&mut alpha);
            let dice = world.dice.clone();
            let damage = expected_damage(&mut world, 1);
            hit(&mut world, 1, ME);
            let mut expected = Vec::new();
            if lands {
                expected.extend([
                    damage_info(damage, DAMAGE_NORMAL),
                    target_at(MAX_HP - damage),
                ]);
            }
            if owns {
                expected.push(owner(1));
            }
            assert_eq!(drained(&mut alpha), expected, "{west} west");
            assert_eq!(world.dice == dice, !lands, "{west} west");
        }
    }

    /// A monster `AIFLAG_NOMOVE` holds has no owner (`G/char.cpp:5471-5472`): no claim is sent
    /// as it is hit, nor a release as it dies and goes.
    #[test]
    fn a_monster_that_never_moves_is_never_owned() {
        let mut npc = a_monster(1);
        npc.ai_flags = AIFLAG_NOMOVE;
        let (_clock, mut world, mut alpha) = a_fight(npc, NEAR);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        let damage = expected_damage(&mut world, 1);
        hit(&mut world, 1, ME);
        assert_eq!(
            drained(&mut alpha),
            [
                damage_info(damage, DAMAGE_NORMAL),
                target_at(0),
                stun_record()
            ]
        );
        world.process_pulse(P0 + 75);
        assert_eq!(drained(&mut alpha), [no_target(), dead_record()]);
        world.process_pulse(P0 + 100);
        assert_eq!(drained(&mut alpha), [remove_record(ME)]);
    }

    /// A claim holds for 100 ms against any other attacker (`IsSyncOwner`, `G/char.cpp:5450-5467`),
    /// and the sync of the same victim reads the same claim and stamp: refused while the claim
    /// is fresh, accepted after, and too soon 100 ms within the last accepted sync.
    #[test]
    fn a_claim_is_one_for_the_attack_and_the_sync() {
        let (clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        assert_eq!(drained(&mut alpha), [owner(1)]);
        assert_eq!(drained(&mut beta), [owner(1)]);
        clock.set(T0 + 99);
        hit(&mut world, 2, ME);
        assert_eq!(drained(&mut beta), nothing());
        let result = world
            .sync_positions(Vid::new(2), &sync_of(ME, SPOT))
            .unwrap();
        assert!(result.accepted_elements.is_empty());
        assert_eq!(drained(&mut beta), nothing());
        clock.set(T0 + 100);
        let result = world
            .sync_positions(Vid::new(2), &sync_of(ME, SPOT))
            .unwrap();
        assert_eq!(result.accepted_elements.len(), 1);
        assert_eq!(drained(&mut alpha)[0], owner(2));
        assert_eq!(world.npc_combat[&ME].sync.owner, Some(Vid::new(2)));
        assert_eq!(
            world.npc_combat[&ME].last_sync,
            Some(Duration::from_millis(u64::from(T0 + 100)))
        );
        // The owner's next sync 50 ms on is too soon, and the refusal is counted.
        clock.set(T0 + 150);
        let result = world
            .sync_positions(Vid::new(2), &sync_of(ME, SPOT))
            .unwrap();
        assert!(result.accepted_elements.is_empty());
        assert_eq!(world.bodies[&Vid::new(2)].sync_hack_count, 1);
        // The first attacker cannot take it back while it is fresh.
        hit(&mut world, 1, ME);
        assert_eq!(world.npc_combat[&ME].sync.owner, Some(Vid::new(2)));
    }

    /// The owner's `CG_SYNC_POSITION` of the monster moves it there (`victim->Sync`,
    /// `G/input_main.cpp:2149-2152`), and the owner's view, the owner aside, is sent the point
    /// after the claim.
    #[test]
    fn the_owners_sync_moves_the_monster() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        drained(&mut beta);
        let to = (3250, 3260);
        let result = world.sync_positions(Vid::new(1), &sync_of(ME, to)).unwrap();
        assert_eq!(result.accepted_elements.len(), 1);
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!((npc.x, npc.y), to);
        assert_eq!(
            world.maps[&PLACE]
                .spot(EntityKey::Npc(ME))
                .map(|spot| (spot.x, spot.y)),
            Some(to)
        );
        // The same owner's claim is sent again (`G/char.cpp:5546-5555`).
        assert_eq!(drained(&mut alpha), [owner(1)]);
        let mut relayed = vec![5, 15, 0];
        for word in [ME.to_le_bytes(), to.0.to_le_bytes(), to.1.to_le_bytes()] {
            relayed.extend_from_slice(&word);
        }
        assert_eq!(drained(&mut beta), [owner(1), relayed]);
    }

    /// A claim on a dead monster fails `battle_is_attackable`, and its targeting claimer is
    /// shown a `DAMAGE_BLOCK` of 0 (`G/char.cpp:5474-5481`).
    #[test]
    fn a_claim_on_a_dead_monster_is_blocked() {
        let (clock, mut world, mut alpha) = a_fight(a_monster(1), NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        hit(&mut world, 1, ME);
        hit(&mut world, 1, ME);
        assert!(world.npc_by_vid(ME).unwrap().dead);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        drained(&mut beta);
        clock.set(T0 + 500);
        let result = world
            .sync_positions(Vid::new(1), &sync_of(ME, SPOT))
            .unwrap();
        assert!(result.accepted_elements.is_empty());
        assert_eq!(drained(&mut alpha), [damage_info(0, DAMAGE_BLOCK)]);
        assert_eq!(drained(&mut beta), nothing());
        // A claimer that does not target it is shown nothing.
        let _ = world
            .sync_positions(Vid::new(2), &sync_of(ME, SPOT))
            .unwrap();
        assert_eq!(drained(&mut beta), nothing());
    }

    /// The cells of [`PLACE`], `ATTR_BANPK` in the columns of `band`.
    fn banpk_cells(band: Range<i32>) -> Arc<HostedCells> {
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
        hosted.insert(PLACE.1, MapCells::new(grid, Arc::new(attr)));
        Arc::new(hosted)
    }

    /// An `ATTR_BANPK` cell under the attacker or under the victim refuses the hit before any
    /// draw (`G/battle.cpp:115-131`).
    #[test]
    fn a_banpk_cell_under_either_refuses_the_hit() {
        for (band, refused) in [(3050..3150, true), (3150..3250, true), (1000..1100, false)] {
            let clock = TestClock::default();
            clock.set(T0);
            let mut world = a_field_on(&clock, vec![a_monster(MAX_HP)], banpk_cells(band.clone()));
            let mut alpha = a_player(&mut world, 1, NEAR);
            world.process_pulse(P0);
            drained(&mut alpha);
            let dice = world.dice.clone();
            hit(&mut world, 1, ME);
            assert_eq!(world.dice == dice, refused, "{band:?}");
            assert_eq!(drained(&mut alpha).is_empty(), refused, "{band:?}");
            assert_eq!(
                world.npc_by_vid(ME).unwrap().hp == MAX_HP,
                refused,
                "{band:?}"
            );
        }
    }

    /// `battle_is_attackable`'s attacker arm (`G/battle.cpp:122`): a stunned or a dead player
    /// hits nothing, before any draw, and claims nothing; a standing one hits.
    #[test]
    fn a_dead_or_stunned_player_cannot_hit_a_monster() {
        for lethal_hits in [0, 1, 2] {
            let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
            world.start_pc_recovery(Vid::new(1));
            let hp = points_of(&mut world, 1).hp();
            for _ in 0..lethal_hits {
                let _died = world.damage_pc(None, Vid::new(1), hp);
            }
            let refused = lethal_hits > 0;
            assert_eq!(world.is_down(1), refused);
            assert_eq!(world.is_dead(1), lethal_hits == 2);
            drained(&mut alpha);
            let dice = world.dice.clone();
            hit(&mut world, 1, ME);
            assert_eq!(world.dice == dice, refused, "{lethal_hits} hits");
            assert_eq!(
                drained(&mut alpha).is_empty(),
                refused,
                "{lethal_hits} hits"
            );
            assert_eq!(world.npc_by_vid(ME).unwrap().hp == MAX_HP, refused);
            let claimed = world
                .npc_combat
                .get(&ME)
                .is_some_and(|combat| combat.sync.owner.is_some());
            assert_eq!(claimed, !refused, "{lethal_hits} hits");
        }
    }

    /// `SetSyncOwner` asks `battle_is_attackable` of the claimer (`G/char.cpp:5474-5481`): a
    /// stunned or a dead player's sync of a monster claims nothing, and its targeting claimer
    /// is shown a `DAMAGE_BLOCK` of 0; a standing one's claims it.
    #[test]
    fn a_dead_or_stunned_players_sync_claims_no_monster() {
        for lethal_hits in [0, 1, 2] {
            let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
            world.start_pc_recovery(Vid::new(1));
            let hp = points_of(&mut world, 1).hp();
            for _ in 0..lethal_hits {
                let _died = world.damage_pc(None, Vid::new(1), hp);
            }
            let refused = lethal_hits > 0;
            assert_eq!(world.is_down(1), refused);
            world.target(Vid::new(1), ME);
            drained(&mut alpha);
            let result = world
                .sync_positions(Vid::new(1), &sync_of(ME, SPOT))
                .unwrap();
            assert_eq!(
                result.accepted_elements.is_empty(),
                refused,
                "{lethal_hits} hits"
            );
            let held = world
                .npc_combat
                .get(&ME)
                .and_then(|combat| combat.sync.owner);
            assert_eq!(
                held,
                (!refused).then_some(Vid::new(1)),
                "{lethal_hits} hits"
            );
            let shown = if refused {
                vec![damage_info(0, DAMAGE_BLOCK)]
            } else {
                vec![owner(1)]
            };
            assert_eq!(drained(&mut alpha), shown, "{lethal_hits} hits");
        }
    }

    /// `Destroy` ends the stun, dead and recovery events (`G/char.cpp:713-718`): a monster gone
    /// while stunned never dies again.
    #[test]
    fn a_destroyed_monster_has_no_event_left() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        hit(&mut world, 1, ME);
        world.npc_mut(ME).unwrap().hp = 1;
        hit(&mut world, 1, ME);
        assert!(world.npc_by_vid(ME).unwrap().stunned);
        world.destroy_npc(ME);
        assert_eq!(drained(&mut alpha).last(), Some(&remove_record(ME)));
        assert!(!world.npc_combat.contains_key(&ME));
        for due in [Due::Stun(ME), Due::Dead(ME), Due::Recovery(ME)] {
            assert!(!world.regens.holds(due), "{due:?}");
        }
        for pulse in [P0 + 75, P0 + 100, P0 + 150] {
            world.process_pulse(pulse);
            assert_eq!(drained(&mut alpha), nothing(), "Pulse {pulse}");
        }
    }

    /// `OnMove(true)` after the claim ends the attacker's revive invisibility
    /// (`G/char.cpp:6156`): the update goes to its view and itself, the removal to itself.
    #[test]
    fn a_hit_ends_the_attackers_revive_invisibility_after_the_claim() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        let hidden = world.affect_flags_of(Vid::new(1));
        let _update = world.enter_revive_invisible(Vid::new(1));
        assert_ne!(world.affect_flags_of(Vid::new(1)), hidden);
        drained(&mut alpha);
        drained(&mut beta);
        hit(&mut world, 1, ME);
        let update = world.update_frame(Vid::new(1), hidden).unwrap();
        let mine = drained(&mut alpha);
        assert_eq!(mine.len(), 3);
        assert_eq!(mine[..2], [owner(1), update.clone()]);
        assert_eq!(drained(&mut beta), [owner(1), update]);
        assert_eq!(world.affect_flags_of(Vid::new(1)), hidden);
        assert!(world.bodies[&Vid::new(1)].last_attack.is_some());
        // A second hit has nothing left to end.
        hit(&mut world, 1, ME);
        assert_eq!(drained(&mut alpha), [owner(1)]);
    }

    /// A bow in the weapon cell deals nothing and draws nothing (`G/battle.cpp:489-497`), yet
    /// the attack goes on: the claim, and the revive invisibility ends.
    #[test]
    fn a_bow_strikes_nothing_but_claims() {
        let clock = TestClock::default();
        clock.set(T0);
        let mut world = a_field_on(&clock, vec![a_monster(MAX_HP)], open_cells());
        let bow = world
            .protos
            .rows()
            .iter()
            .find(|proto| proto.item_type == ITEM_WEAPON && proto.sub_type == WEAPON_BOW)
            .unwrap()
            .vnum;
        let mut item = Item::new(11, bow);
        item.set_size(1).unwrap();
        let cell = INVENTORY_MAX_NUM + EWearPositions::Weapon as u16;
        let items = [(ItemPos::new(EWindows::Inventory as u8, cell), item)];
        let (mut alpha, _shown) = enter_seeing_with(&mut world, PLACE, 1, NEAR, 12_345, &items);
        world.process_pulse(P0);
        world.target(Vid::new(1), ME);
        let _update = world.enter_revive_invisible(Vid::new(1));
        drained(&mut alpha);
        let dice = world.dice.clone();
        hit(&mut world, 1, ME);
        assert_eq!(world.dice, dice);
        let mine = drained(&mut alpha);
        assert_eq!((mine.len(), &mine[0]), (3, &owner(1)));
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, MAX_HP);
        assert_eq!(world.npc_combat[&ME].recovery_due, None);
    }

    /// A player that enters while the monster lies dead is shown it with
    /// `ADD_CHARACTER_STATE_DEAD` (`G/char.cpp:1446-1447`); a stunned one is not dead yet.
    #[test]
    fn an_entrant_sees_the_dead_monster_dead() {
        let (_clock, mut world, _alpha) = a_fight(a_monster(1), NEAR);
        let state_of = |shown: &[Vec<u8>]| {
            let add = shown
                .iter()
                .find(|record| {
                    record[0] == GcCharacterAdd::header() && record[1..5] == ME.to_le_bytes()
                })
                .expect("the monster is shown");
            add[26]
        };
        hit(&mut world, 1, ME);
        let (_beta, shown) = enter_seeing(&mut world, PLACE, 2, EAST, 12_345);
        assert_eq!(state_of(&shown), 0, "stunned");
        world.process_pulse(P0 + 75);
        let (_gamma, shown) = enter_seeing(&mut world, PLACE, 3, EAST, 12_345);
        assert_eq!(state_of(&shown), 1, "dead");
    }

    /// A player that leaves has no owner, which its view is told first, and releases what it
    /// owns, each victim's view told it has no owner, players before non-players and each by VID
    /// (`G/char.cpp:653`, `:5559-5575`; V1). The claim's time stays.
    #[test]
    fn a_leaving_owner_releases_what_it_owns() {
        let (_clock, mut world, _alpha) = a_fight(a_monster(MAX_HP), NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        let _gamma = a_player(&mut world, 3, (3150, 3200));
        hit(&mut world, 1, ME);
        let claimed = SyncOwnershipState {
            owner: Some(Vid::new(1)),
            claimed_at: Some(Duration::from_millis(u64::from(T0))),
        };
        assert_eq!(world.npc_combat[&ME].sync, claimed);
        world.bodies.get_mut(&Vid::new(3)).unwrap().sync = claimed;
        drained(&mut beta);
        let _kept = world.leave_world(Vid::new(1));
        assert_eq!(
            drained(&mut beta),
            [
                ownership_record(Vid::new(0), Vid::new(1)),
                ownership_record(Vid::new(0), Vid::new(3)),
                owner(0),
                remove_record(1)
            ]
        );
        let released = SyncOwnershipState {
            owner: None,
            ..claimed
        };
        assert_eq!(world.npc_combat[&ME].sync, released);
        assert_eq!(world.bodies[&Vid::new(3)].sync, released);
    }
}
