//! The state machine of the non-players: `CFSM` (`G/FSM.cpp`), the Idle and Move states legacy
//! gives a character (`G/char_state.cpp`), and the state set `CHARACTER_MANAGER::Update` runs
//! every Pulse (`m_set_pkChrState`, `G/char_manager.cpp:716-726`).
//!
//! # The state set
//!
//! An NPC runs only while a player stands near it. The sectrees ask for each start and stop as
//! a [`Wake`] (see [`super::view`]): a player's view taking an NPC in starts it
//! (`G/entity_view.cpp:110-118`), an NPC entering a sectree a player stands near starts it
//! (`G/sectree.cpp:150-155`), and a sectree left with no player near stops every NPC in it
//! (`G/sectree.cpp:88-117`). A warp or goto NPC is never started. A start puts the NPC in the set
//! with its next update one Pulse after the Pulse it was asked on (`StartStateMachine`,
//! `G/char.cpp:5322-5331`); an NPC already in the set keeps its next update. The world applies
//! the wakes at the start of each Pulse and again just before the update, so a wake asked
//! between two Pulses counts from the earlier one, as legacy's heart pulse does.
//!
//! # The update
//!
//! Each NPC in the set, in VID order (V1), whose next update is due runs `CFSM::Update`: a state
//! asked for since its last update becomes its state, then its state runs, and its next update
//! is `m_dwStateDuration` Pulses later (`UpdateStateMachine`, `G/char.cpp:5338-5348`). A new
//! character asks for Idle and waits 1 Pulse (`CHARACTER::Initialize`, `G/char.cpp:263-264`).
//!
//! - **Idle** (`StateIdle`, `G/char_state.cpp:362-387`). A stone waits 1 second. A monster runs
//!   `__StateIdle_Monster` (`:626-736`): a coward runs away (`CowardEscape`, `:282-321`);
//!   otherwise it waits 1 to 3 seconds when aggressive and 3 to 5 when not, and one time in 7
//!   turns to a random heading and walks 300 to 700 units, when neither `no_wander` nor
//!   `AIFLAG_NOMOVE` holds it and both the point and the half-way point are movable; a monster
//!   that does not walk may speak (`MonsterChat(MONSTER_CHAT_WAIT)`).
//!   Any other non-player runs `__StateIdle_NPC` (`:507-624`): it may speak, waits 5 seconds,
//!   and, unless it is a guard or `AIFLAG_NOMOVE` holds it, one time in 7 walks 200 to 400 units
//!   at a walk, which tells its view the first time.
//! - **Move** (`StateMove`, `:782-928`). `Goto` set the destination, the move's duration and an
//!   update every 4 Pulses (`G/char.cpp:3471-3508`), and told the view with a `FUNC_WAIT`
//!   (`SendMovePacket`, `:3624-3636`). Each update moves the NPC to the point the elapsed share
//!   of the duration reaches, and at the end asks for Idle with a wait of 1 to 3 seconds.
//!
//! `MonsterChat` (`G/char.cpp:7555-7600`) speaks one time in 61 for a monster, a line of its
//! race's waiting three, and one time in 7 for a guard and 31 for any other non-player, any line
//! of its race. The lines are `locale.monster_chat` of the quest libraries; a world with no
//! quests has none. The line is a `GC_CHAT` the speaker says to its view, with its VID, as a
//! player's talk is, and not a server line.
//!
//! A player's hit can stun a monster and kill it (see [`super::fight`]). A dead NPC's update
//! does nothing, and neither does a stunned monster's Idle (`G/char_state.cpp:628-629`), while
//! its Move goes on, as `StateMove` has no such guard. The line a hit monster speaks
//! (`MonsterChat(MONSTER_CHAT_ATTACKED)`) is drawn with the hit.
//!
//! # What is not here yet
//!
//! - **The fight's missing arms (V19).** The guard's victim search, the aggro of a
//!   chasing monster, the mob skills, the summoners and god speed, and a RANGE or MAGIC
//!   monster's fight wait for 229m. A ranging or magic monster near a player therefore
//!   wanders as a peaceful one does, and the melee family fights ([`super::mob_battle`],
//!   [`super::mob_strike`]).
//! - **The protector.** `GetProtege` (a group's leader, a stone's spawn) and `Follow` wait for the
//!   group's party and follow.
//! - **A stone's spawns** at each tenth of its hit points wait for a hit on a stone, which is
//!   refused until they are ported.
//! - **Santa, the war flags and the horses** (`StateHorse`) wait for their systems; no spawn file
//!   stands a horse race.
//! - **The monster clips.** Every NPC moves at the fall-back motion speed of 300 (V2), as no
//!   monster `.msa` is loaded.
//!
//! The berserk and god-speed resets wait for the fight too: 148 mob protos carry `BERSERK` and 46
//! `GODSPEED`, but only Battle turns either on (`G/char_state.cpp:1125-1133`), so Idle finds
//! nothing to reset (`:643-649`). No affect stuns an NPC and none keeps a shop, so `CanMove`
//! always holds. The `test_server` walk toggle (`:740-746`), which sets a wandering monster
//! running 60 times in 101, waits for the `test_server` audit (owner question 5): legacy's
//! `test_server` defaults to 1 (`G/config.cpp:72`).
//!
//! # Divergences
//!
//! - **A step outside every sectree.** Legacy's `Sync` kills an NPC whose step lands where no
//!   sectree is (`Dead()`, `G/char.cpp:3403-3407`). The Rewrite leaves it standing where it was
//!   (a Defect, fixed).
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use common::cfloat::{f32_to_i32, i32_to_f32};
use common::vid::Vid;
use gamedata::mob_proto::{
    AIFLAG_COWARD, AIFLAG_NOMOVE, CHAR_TYPE_GOTO, CHAR_TYPE_HORSE, CHAR_TYPE_MONSTER,
    CHAR_TYPE_STONE, CHAR_TYPE_WARP, MOB_RANK_BOSS,
};
use protocol::gc_actors::GcCharacterMove;
use protocol::gc_chat::{GcChat, CHAT_TYPE_TALKING};
use world::character::number;
use world::npc::Npc;

use super::motion::{move_duration, rotation_to_xy, Motion, MOTION_SPEED_FALLBACK};
use super::view::{EntityKey, Wake};
use super::view_encode::walk_mode;
use super::GameState;
use crate::movement::FUNC_WAIT;
use crate::save::PASSES_PER_SEC;

/// `WALKMODE_WALK` (`G/packet.h:2377-2381`).
pub(super) const WALKMODE_WALK: u8 = 1;

/// `m_dwStateDuration` after a `Goto` (`G/char.cpp:3482`, `:3493`), in Pulses.
pub(super) const MOVE_STEP_PULSES: u64 = 4;

/// `MONSTER_CHAT_WAIT` (`G/constants.h:8`): the chat a waiting character says.
const MONSTER_CHAT_WAIT: i32 = 0;

/// `MONSTER_CHAT_ATTACK` (`G/constants.h:9`): the chat an attacking monster says.
const MONSTER_CHAT_ATTACK: i32 = 1;

/// The guards' races (`IsGuardNPC`, `G/char.cpp:7664-7667`).
const GUARD_RACES: [u16; 3] = [11000, 11002, 11004];

/// The races legacy gives `StateHorse` whatever their type (`G/char.cpp:2545-2560`).
const HORSE_RACES: std::ops::RangeInclusive<u16> = 20101..=20109;

/// `CowardEscape`'s distances (`G/char_state.cpp:284`): a try at index `i` walks from
/// `COWARD_DISTANCES[i]` to `COWARD_DISTANCES[i + 1]`.
const COWARD_DISTANCES: [i32; 4] = [500, 1000, 3000, 5000];

/// `PASSES_PER_SEC(seconds)`.
pub(super) fn pulses(seconds: i32) -> u64 {
    u64::try_from(seconds)
        .unwrap_or(0)
        .saturating_mul(u64::from(PASSES_PER_SEC))
}

/// A state of `CFSM`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Fsm {
    /// `m_stateInitial`, which does nothing.
    Initial,
    /// `m_stateIdle`.
    Idle,
    /// `m_stateMove`.
    Move,
    /// `m_stateBattle` (see [`super::mob_battle`]).
    Battle,
}

/// `m_kVIDVictim`. A player's VID comes back at its next login, so a victim that leaves is
/// `Gone`: legacy's handle that no longer finds its character, which every reader takes as
/// `!victim` without the stand that `SetVictim(NULL)` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Victim {
    /// The victim is online under this VID.
    Here(Vid),
    /// The victim left; its VID must not be attacked if it comes back.
    Gone,
}

/// What legacy's `CHARACTER` keeps for an NPC's state machine and its walk.
#[derive(Clone, Copy, Debug)]
pub(super) struct Mind {
    /// `m_pCurrentState`.
    pub(super) state: Fsm,
    /// `m_pNewState`: the state asked for, which the next update enters.
    pub(super) asked: Option<Fsm>,
    /// `m_dwNextStatePulse`.
    pub(super) next_pulse: u64,
    /// `m_dwStateDuration`, in Pulses.
    pub(super) duration: u64,
    /// `GetRotation()` in degrees.
    pub(super) rotation: f32,
    /// `m_posStart`, `m_posDest`, `m_dwMoveStartTime` and `m_dwMoveDuration`. `moving` is unused:
    /// the state says whether the NPC moves.
    pub(super) motion: Motion,
    /// `m_bNowWalking`.
    pub(super) walking: bool,
    /// `m_kVIDVictim`.
    pub(super) victim: Option<Victim>,
    /// `POS_FIGHTING`.
    pub(super) fighting: bool,
    /// `m_isBerserk`.
    pub(super) berserk: bool,
}

impl Mind {
    /// An NPC as `Initialize` and `Show` leave it: Idle asked for, a duration of 1, standing at
    /// its point with its spawn's heading.
    pub(super) fn new(npc: &Npc) -> Self {
        Self {
            state: Fsm::Initial,
            asked: Some(Fsm::Idle),
            next_pulse: 0,
            duration: 1,
            rotation: f32::from(npc.rotation),
            motion: Motion::standing((npc.x, npc.y)),
            walking: false,
            victim: None,
            fighting: false,
            berserk: false,
        }
    }
}

/// Every NPC's state machine, and the state set.
#[derive(Debug, Default)]
pub(super) struct Minds {
    /// The state machine of each NPC that has ever been started, by VID.
    machines: HashMap<u32, Mind>,
    /// `m_set_pkChrState`, in VID order (V1).
    running: BTreeSet<u32>,
    /// `no_wander`: no monster wanders or looks for a victim.
    no_wander: bool,
}

impl Minds {
    /// The state machine of the NPC under `vid`, once started.
    pub(super) fn get(&self, vid: u32) -> Option<&Mind> {
        self.machines.get(&vid)
    }

    /// Whether the NPC under `vid` is in the state set.
    #[cfg(test)]
    pub(super) fn is_running(&self, vid: u32) -> bool {
        self.running.contains(&vid)
    }

    /// Forgets a destroyed NPC.
    pub(super) fn forget(&mut self, vid: u32) {
        self.machines.remove(&vid);
        self.running.remove(&vid);
    }

    /// The state machine of the NPC under `vid`, to change.
    pub(super) fn get_mut(&mut self, vid: u32) -> Option<&mut Mind> {
        self.machines.get_mut(&vid)
    }

    /// `AddToStateList` plus the Mind: the monster runs, and its machine is `fresh` unless it
    /// has one.
    pub(super) fn ensure(&mut self, vid: u32, fresh: Mind) -> &mut Mind {
        self.running.insert(vid);
        self.machines.entry(vid).or_insert(fresh)
    }

    /// Every state machine, to change.
    pub(super) fn minds_mut(&mut self) -> impl Iterator<Item = (u32, &mut Mind)> {
        self.machines.iter_mut().map(|(vid, mind)| (*vid, mind))
    }

    /// Whether `no_wander` holds: no monster wanders or looks for a victim.
    pub(super) const fn wanders_off(&self) -> bool {
        self.no_wander
    }
}

/// What the idle states read of an NPC.
#[derive(Clone, Copy, Debug)]
pub(super) struct Who {
    pub(super) char_type: u8,
    pub(super) race: u16,
    pub(super) ai_flags: u32,
    pub(super) map: i32,
    pub(super) aggressive: bool,
    /// `IsStun()`: a player's hit left it no hit points (see [`super::fight`]).
    pub(super) stunned: bool,
}

/// `GetDeltaByDegree` (`G/vector.cpp:54-60`): the step of `distance` toward `degree`, clockwise
/// from north, in `float`. `sin` and `cos` of a `float` are the `float` overloads, as `<cmath>`
/// resolves them.
pub(super) fn delta_by_degree(degree: f32, distance: f32) -> (f32, f32) {
    // `DegreeToRadian`: `degree * (_PI / 180.0f)`, with `_PI` the `float` of pi.
    let radian = degree * (std::f32::consts::PI / 180.0);
    (distance * radian.sin(), distance * radian.cos())
}

impl GameState {
    /// Set `no_wander` (`G/config.cpp:1281-1283`): no monster wanders, and none looks for a
    /// victim.
    #[must_use]
    pub fn with_no_wander(mut self, no_wander: bool) -> Self {
        self.minds.no_wander = no_wander;
        self
    }

    /// Applies the starts and stops every map asked for since the last time, each start
    /// counting from the Pulse in effect now.
    pub(super) fn wake_minds(&mut self) {
        let mut wakes = Vec::new();
        for index in self.maps.values_mut() {
            wakes.extend(index.take_wakes());
        }
        for wake in wakes {
            match wake {
                Wake::Start(vid) => self.start_state_machine(vid),
                Wake::Stop(vid) => {
                    self.minds.running.remove(&vid);
                }
            }
        }
    }

    /// `StartStateMachine(1)` (`G/char.cpp:5322-5331`) of the NPC under `vid`, unless it is a
    /// warp or goto (`G/entity_view.cpp:115`, `G/sectree.cpp:150`) or already in the set.
    fn start_state_machine(&mut self, vid: u32) {
        let Some(npc) = self.npc_by_vid(vid) else {
            return;
        };
        if matches!(npc.char_type, CHAR_TYPE_WARP | CHAR_TYPE_GOTO) {
            return;
        }
        let fresh = Mind::new(npc);
        if !self.minds.running.insert(vid) {
            return;
        }
        let next_pulse = self.last_pulse.saturating_add(1);
        self.minds.machines.entry(vid).or_insert(fresh).next_pulse = next_pulse;
    }

    /// The state set's part of `CHARACTER_MANAGER::Update` (`G/char_manager.cpp:716-726`): each
    /// NPC in the set when the update starts runs `UpdateStateMachine`, in VID order (V1).
    pub(super) fn run_minds(&mut self, pulse: u64) {
        self.wake_minds();
        let running: Vec<u32> = self.minds.running.iter().copied().collect();
        for vid in running {
            self.update_state_machine(vid, pulse);
        }
    }

    /// `UpdateStateMachine` (`G/char.cpp:5338-5348`).
    fn update_state_machine(&mut self, vid: u32, pulse: u64) {
        let Some(dead) = self.npc_by_vid(vid).map(|npc| npc.dead) else {
            self.minds.forget(vid);
            return;
        };
        let Some(mind) = self.minds.machines.get_mut(&vid) else {
            return;
        };
        if pulse < mind.next_pulse {
            return;
        }
        // `if (IsDead()) return;`: the next update stays where it was.
        if dead {
            return;
        }
        // `CFSM::Update` (`G/FSM.cpp:22-38`); every Begin and End is empty.
        if let Some(asked) = mind.asked.take() {
            mind.state = asked;
        }
        match mind.state {
            Fsm::Initial => {}
            Fsm::Idle => self.state_idle(vid),
            Fsm::Move => self.state_move(vid),
            Fsm::Battle => self.state_battle(vid),
        }
        if let Some(mind) = self.minds.machines.get_mut(&vid) {
            mind.next_pulse = pulse.saturating_add(mind.duration);
        }
    }

    /// What the idle states read of the NPC under `vid`.
    pub(super) fn who(&self, vid: u32) -> Option<Who> {
        let &(_, map, _) = self.npc_of.get(&vid)?;
        let npc = self.npc_by_vid(vid)?;
        Some(Who {
            char_type: npc.char_type,
            race: npc.race,
            ai_flags: npc.ai_flags,
            map,
            aggressive: npc.is_aggressive(),
            stunned: npc.stunned,
        })
    }

    pub(super) fn set_duration(&mut self, vid: u32, duration: u64) {
        if let Some(mind) = self.minds.machines.get_mut(&vid) {
            mind.duration = duration;
        }
    }

    /// `StateIdle` (`G/char_state.cpp:362-387`).
    fn state_idle(&mut self, vid: u32) {
        let Some(who) = self.who(vid) else {
            return;
        };
        if who.char_type == CHAR_TYPE_HORSE || HORSE_RACES.contains(&who.race) {
            // `StateHorse` is not ported.
            return;
        }
        match who.char_type {
            // `__StateIdle_Stone` (`:389-505`): the spawns at each tenth wait for damage, and a
            // stone at full hit points spawns nothing.
            CHAR_TYPE_STONE => self.set_duration(vid, pulses(1)),
            // A warp or goto would wait 60 seconds (`:369-374`), but none is ever started.
            CHAR_TYPE_MONSTER => self.idle_monster(vid, who),
            _ => self.idle_npc(vid, who),
        }
    }

    /// `__StateIdle_NPC` (`G/char_state.cpp:507-624`). No pet, mount or shaman is ported, the
    /// guard's victim search waits for the fight, and no protector is ported.
    fn idle_npc(&mut self, vid: u32, who: Who) {
        self.monster_chat_wait(vid, who);
        self.set_duration(vid, pulses(5));
        if GUARD_RACES.contains(&who.race) || who.ai_flags & AIFLAG_NOMOVE != 0 {
            return;
        }
        if number(&mut self.dice, 0, 6) != 0 {
            return;
        }
        self.turn_at_random(vid);
        let distance = i32_to_f32(number(&mut self.dice, 200, 400));
        let Some((x, y)) = self.wander_target(vid, who.map, distance) else {
            return;
        };
        self.set_now_walking(vid, true);
        if self.npc_goto(vid, x, y) {
            self.send_move_wait(vid);
        }
    }

    /// `__StateIdle_Monster` (`G/char_state.cpp:626-757`).
    fn idle_monster(&mut self, vid: u32, who: Who) {
        // `if (IsStun()) return;` (`:628-629`): the wait stays as it was.
        if who.stunned {
            return;
        }
        if who.ai_flags & AIFLAG_COWARD != 0 {
            self.coward_escape(vid, who.map);
            return;
        }
        // Berserk off (`:643-644`); god speed is dormant (no `POINT_GODSPEED`).
        if let Some(mind) = self.minds.get_mut(vid) {
            mind.berserk = false;
        }
        // Steps 4-6: the held victim is dropped when it is gone, dead or on BANPK, and an
        // aggressive wanderer looks for one.
        let victim = self.idle_victim(vid, who);
        // Step 7: a live victim starts the fight at once; `BeginFight`'s next Pulse is
        // overwritten by the post-Execute line, so Battle starts 25 Pulses later (2.2).
        if let Some(found) = victim {
            if !self.is_dead(found.raw()) && self.can_begin_fight(vid) {
                self.begin_fight(vid, found);
            }
            return;
        }
        // The wait: aggressive and victimless below, `IsAggressive() && !victim` always holds.
        let wait = if who.aggressive {
            number(&mut self.dice, 1, 3)
        } else {
            number(&mut self.dice, 3, 5)
        };
        self.set_duration(vid, pulses(wait));
        if !self.minds.no_wander
            && who.ai_flags & AIFLAG_NOMOVE == 0
            && number(&mut self.dice, 0, 6) == 0
        {
            self.turn_at_random(vid);
            let distance = i32_to_f32(number(&mut self.dice, 300, 700));
            let Some((x, y)) = self.wander_target(vid, who.map, distance) else {
                return;
            };
            if self.npc_goto(vid, x, y) {
                self.send_move_wait(vid);
            }
            return;
        }
        self.monster_chat_wait(vid, who);
    }

    /// Idle steps 4-6 (`G/char_state.cpp:653-686`): the victim held, or found for an
    /// aggressive wanderer. The `!victim` arm's second is set by the caller asking for nothing.
    fn idle_victim(&mut self, vid: u32, who: Who) -> Option<Vid> {
        let held = self.victim_of(vid);
        let lost = match held {
            None => true,
            Some(found) => {
                self.is_dead(found.raw())
                    || self
                        .place_of(found.raw())
                        .and_then(|(_, map)| self.spot_of(found.raw()).map(|spot| (map, spot)))
                        .is_some_and(|(map, spot)| self.is_banpk(map, spot.x, spot.y))
            }
        };
        if lost {
            self.set_victim(vid, None);
            self.set_duration(vid, pulses(1));
        } else if held.is_some() {
            return held;
        }
        // The Meley 40000 retry applies to a map that is not hosted; map 61's `xmas_tree`
        // flag reads 0 here. RANGE and MAGIC never search (V19a).
        if !self.minds.no_wander
            && who.aggressive
            && self
                .npc_by_vid(vid)
                .is_some_and(super::mob_battle::fights_back)
        {
            let sight = self
                .npc_by_vid(vid)
                .map_or(0, |npc| npc.fight.aggressive_sight);
            if let Some(found) = self.find_victim(vid, sight) {
                return Some(found);
            }
        }
        if lost {
            return None;
        }
        held
    }

    /// The idle walk's point `distance` ahead of the NPC's heading, when both it and the
    /// half-way point are movable (`G/char_state.cpp:592-598`, `:730-735`).
    fn wander_target(&self, vid: u32, map: i32, distance: f32) -> Option<(i32, i32)> {
        let spot = self.npc_spot(vid)?;
        let rotation = self.minds.get(vid)?.rotation;
        let (fx, fy) = delta_by_degree(rotation, distance);
        let (dx, dy) = (f32_to_i32(fx), f32_to_i32(fy));
        let target = (spot.0.saturating_add(dx), spot.1.saturating_add(dy));
        let half = (spot.0.saturating_add(dx / 2), spot.1.saturating_add(dy / 2));
        (self.is_movable(map, target.0, target.1) && self.is_movable(map, half.0, half.1))
            .then_some(target)
    }

    /// `CowardEscape` (`G/char_state.cpp:282-321`): up to eight headings at each of three
    /// distances, farthest first, the first whose every hundredth of the way is movable.
    pub(super) fn coward_escape(&mut self, vid: u32, map: i32) {
        for index in (0..3).rev() {
            for _ in 0..8 {
                self.turn_at_random(vid);
                let distance = i32_to_f32(number(
                    &mut self.dice,
                    COWARD_DISTANCES[index],
                    COWARD_DISTANCES[index + 1],
                ));
                let (Some(spot), Some(mind)) = (self.npc_spot(vid), self.minds.get(vid)) else {
                    return;
                };
                let (fx, fy) = delta_by_degree(mind.rotation, distance);
                let (dx, dy) = (f32_to_i32(fx), f32_to_i32(fy));
                // `(int) fx * j / 100`: the truncated step, times j, divided as an `int`.
                let blocked = (1..=100).any(|j| {
                    !self.is_movable(
                        map,
                        spot.0.saturating_add(dx.saturating_mul(j) / 100),
                        spot.1.saturating_add(dy.saturating_mul(j) / 100),
                    )
                });
                if blocked {
                    continue;
                }
                // The second this sets (`:309`) is replaced at once by the Goto's 4 Pulses, as an
                // idle NPC is never in Move.
                let target = (spot.0.saturating_add(dx), spot.1.saturating_add(dy));
                if self.npc_goto(vid, target.0, target.1) {
                    self.send_move_wait(vid);
                }
                return;
            }
        }
    }

    /// `SetRotation(number(0, 359))`.
    fn turn_at_random(&mut self, vid: u32) {
        let degrees = number(&mut self.dice, 0, 359);
        if let Some(mind) = self.minds.machines.get_mut(&vid) {
            mind.rotation = i32_to_f32(degrees);
        }
    }

    /// Where the NPC under `vid` stands.
    pub(super) fn npc_spot(&self, vid: u32) -> Option<(i32, i32)> {
        let npc = self.npc_by_vid(vid)?;
        Some((npc.x, npc.y))
    }

    /// `SetNowWalking` (`G/char.cpp:7345-7380`): a change tells the view.
    fn set_now_walking(&mut self, vid: u32, walking: bool) {
        let Some(mind) = self.minds.machines.get_mut(&vid) else {
            return;
        };
        if mind.walking == walking {
            return;
        }
        mind.walking = walking;
        let mode = if walking { WALKMODE_WALK } else { 0 };
        let me = EntityKey::Npc(vid);
        self.packet_around(me, &walk_mode(vid, mode), None);
    }

    /// `CHARACTER::Goto` (`G/char.cpp:3471-3508`) for an NPC. Returns true when a new
    /// destination was set. Entering Move with a live victim says
    /// `MonsterChat(MONSTER_CHAT_ATTACK)` first.
    pub(super) fn npc_goto(&mut self, vid: u32, x: i32, y: i32) -> bool {
        let Some(spot) = self.npc_spot(vid) else {
            return false;
        };
        let move_speed = self
            .npc_by_vid(vid)
            .map_or(0, |npc| i32::from(npc.moving_speed));
        let now = self.clock.now();
        let Some(mind) = self.minds.machines.get_mut(&vid) else {
            return false;
        };
        if spot == (x, y) {
            return false;
        }
        if mind.motion.dest == (x, y) {
            if mind.state != Fsm::Move {
                mind.duration = MOVE_STEP_PULSES;
                mind.asked = Some(Fsm::Move);
            }
            return false;
        }
        mind.motion.dest = (x, y);
        // `CalculateMoveDuration` (`:3585-3603`); a non-player's clip is always the run's, and no
        // monster clip is loaded (V2).
        mind.motion.start = spot;
        mind.motion.duration_ms = move_duration(spot, (x, y), MOTION_SPEED_FALLBACK, move_speed);
        mind.motion.start_ms = now;
        mind.duration = MOVE_STEP_PULSES;
        let chats = mind.state != Fsm::Move;
        mind.asked = Some(Fsm::Move);
        // Entering Move with a held victim that is still online chats first (`:3500-3503`).
        // `Gone` gives `NULL`, so there is no chat.
        if chats && self.victim_of(vid).is_some() {
            self.monster_chat_attack(vid);
        }
        true
    }

    /// `SendMovePacket(FUNC_WAIT, 0, 0, 0, 0)` (`G/char.cpp:3624-3636`): the destination and the
    /// move's duration, the heading as `(int) GetRotation() / 5`, to the view.
    pub(super) fn send_move_wait(&mut self, vid: u32) {
        let Some(mind) = self.minds.get(vid) else {
            return;
        };
        let rotation = (f32_to_i32(mind.rotation) / 5).to_le_bytes()[0];
        let record = GcCharacterMove::new(
            FUNC_WAIT,
            0,
            rotation,
            vid,
            mind.motion.dest.0,
            mind.motion.dest.1,
            self.clock.now(),
            mind.motion.duration_ms,
        )
        .encode();
        let me = EntityKey::Npc(vid);
        self.packet_around(me, &record, Some(me));
    }

    /// `StateMove` (`G/char_state.cpp:782-928`) for an NPC.
    fn state_move(&mut self, vid: u32) {
        let Some(mind) = self.minds.get(vid) else {
            return;
        };
        let step = mind.motion.at(self.clock.now());
        if let Some((point, _)) = step {
            // `Move(x, y)` (`:793`); a not-a-number rate skips only this (D1).
            self.npc_move(vid, point.0, point.1);
        }
        // A chasing boss refollows one time in four (`:880-885`); race 2191 is not spawned.
        if let Some(victim) = self.victim_of(vid) {
            let boss = self
                .npc_by_vid(vid)
                .is_some_and(|npc| npc.fight.rank >= MOB_RANK_BOSS);
            if boss && number(&mut self.dice, 0, 3) == 0 && self.goto_near_target(vid, victim) {
                return;
            }
        }
        if let Some((_, true)) = step {
            // With a victim that is not a coward's, Battle after one Pulse; only the
            // `MonsterLog` sits under legacy's `if (!IsState(m_stateBattle))` (`:907-918`).
            let to_battle = self.victim_of(vid).is_some()
                && self
                    .npc_by_vid(vid)
                    .is_none_or(|npc| npc.ai_flags & AIFLAG_COWARD == 0);
            if to_battle {
                if let Some(mind) = self.minds.machines.get_mut(&vid) {
                    mind.asked = Some(Fsm::Battle);
                    mind.duration = 1;
                }
            } else {
                let wait = number(&mut self.dice, 1, 3);
                if let Some(mind) = self.minds.machines.get_mut(&vid) {
                    mind.asked = Some(Fsm::Idle);
                    mind.duration = pulses(wait);
                }
            }
        }
    }

    /// `CHARACTER::Move` and `Sync` (`G/char.cpp:3384-3458`, `:3610-3622`) for an NPC: the same
    /// point changes nothing; any other turns it toward the point and stands it there with z 0,
    /// changing its sectree but not its view. A point no sectree holds leaves it where it was.
    pub(super) fn npc_move(&mut self, vid: u32, x: i32, y: i32) {
        let Some(&(channel, map, at)) = self.npc_of.get(&vid) else {
            return;
        };
        let Some(spot) = self.npc_spot(vid) else {
            return;
        };
        if spot == (x, y) {
            return;
        }
        let radius = self.view_radius;
        // An NPC always stands in a sectree, so its move fills no view: `move_body` fills one
        // only for a body that stood in none.
        let mut effects = Vec::new();
        let Some(index) = self.maps.get_mut(&(channel, map)) else {
            return;
        };
        if !index.move_body(EntityKey::Npc(vid), x, y, radius, &mut effects) {
            return;
        }
        if let Some(mind) = self.minds.machines.get_mut(&vid) {
            mind.rotation = rotation_to_xy(x.saturating_sub(spot.0), y.saturating_sub(spot.1));
        }
        if let Some(list) = self.npcs.get_mut(&(channel, map)) {
            if let Some(npc) = Arc::make_mut(list).npcs.get_mut(at) {
                npc.x = x;
                npc.y = y;
                npc.z = 0;
            }
        }
    }

    /// `MonsterChat(MONSTER_CHAT_WAIT)` (`G/char.cpp:7555-7600`).
    /// `MonsterChat(MONSTER_CHAT_ATTACK)` (`G/char_battle.cpp:310-316`): one time in 61 the
    /// monster says one of its race's three attack lines, drawn as `MONSTER_CHAT_ATTACK * 3 +
    /// number(1, 3)`.
    pub(super) fn monster_chat_attack(&mut self, vid: u32) {
        if number(&mut self.dice, 0, 60) != 0 {
            return;
        }
        let line = number(&mut self.dice, 1, 3);
        self.monster_say(vid, MONSTER_CHAT_ATTACK * 3 + line);
    }

    fn monster_chat_wait(&mut self, vid: u32, who: Who) {
        let index = if who.char_type == CHAR_TYPE_MONSTER {
            if number(&mut self.dice, 0, 60) != 0 {
                return;
            }
            MONSTER_CHAT_WAIT * 3 + number(&mut self.dice, 1, 3)
        } else {
            let odds = if GUARD_RACES.contains(&who.race) {
                6
            } else {
                30
            };
            if number(&mut self.dice, 0, odds) != 0 {
                return;
            }
            let race = u32::from(who.race);
            let Some(count) = self
                .quests
                .as_ref()
                .and_then(|quests| quests.manager().host().monster_chat_count(race))
            else {
                return;
            };
            number(&mut self.dice, 1, count)
        };
        self.monster_say(vid, index);
    }

    /// The non-player under `vid` says line `index` of its race's `locale.monster_chat`
    /// (`G/char.cpp:7568-7600`): a talking `GC_CHAT` with its VID, to its view. A world with no
    /// quests, and a line that is missing or empty, say nothing.
    pub(super) fn monster_say(&self, vid: u32, index: i32) {
        let (Some(npc), Some(quests)) = (self.npc_by_vid(vid), &self.quests) else {
            return;
        };
        let line = quests
            .manager()
            .host()
            .monster_chat_line(u32::from(npc.race), index);
        if line.is_empty() {
            return;
        }
        // `text.size() + 1`: the line goes with its terminator.
        let mut text = line;
        text.push(0);
        let chat = GcChat {
            chat_type: CHAT_TYPE_TALKING,
            id: vid,
            empire: 0,
            can_format: true,
            text,
        };
        let Ok(record) = chat.encode() else {
            return;
        };
        self.packet_around(EntityKey::Npc(vid), &record, None);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use common::vid::Vid;
    use gamedata::mob_group::MobGroups;
    use gamedata::mob_locale_names::{MobLocaleNames, MobNamesByLanguage};
    use gamedata::mob_proto::{MobProtos, AIFLAG_AGGRESSIVE, CHAR_TYPE_NPC};
    use gamedata::server_attr::{
        CellBlock, SectreeGrid, ServerAttr, ATTR_BLOCK, CELL_SIZE, SECTREE_CELLS,
    };
    use quest::manager::Manager;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::cells::{HostedCells, MapCells};
    use world::character::Pcg32;
    use world::npc::{MapNpcs, NpcVids, Respawner, FIRST_NPC_VID};

    use super::super::fixtures::{
        a_world, enter, enter_seeing, open_cells, region, TestClock, PLACE,
    };
    use super::super::quests::Quests;
    use super::super::view_encode::encode_npc_insert;
    use super::*;
    use crate::game_loop::PulseProcessor;
    use crate::movement::FUNC_MOVE;

    /// The NPC every test watches.
    const ME: u32 = FIRST_NPC_VID;

    /// Where it stands: the middle of [`PLACE`]'s open cells, so any idle walk is movable.
    const SPOT: (i32, i32) = (3200, 3200);

    /// When the tests' Pulse 1 runs, in milliseconds.
    const T0: u32 = 1_000;

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// An NPC of `char_type` and race `race` at [`SPOT`] with `ai_flags`, at a moving speed
    /// of 100.
    fn an_npc(vid: u32, char_type: u8, race: u16, ai_flags: u32) -> Npc {
        Npc {
            vid,
            vnum: u32::from(race),
            race,
            char_type,
            x: SPOT.0,
            y: SPOT.1,
            z: 7,
            rotation: 90,
            moving_speed: 100,
            ai_flags,
            hp: 100,
            max_hp: 100,
            ..Npc::default()
        }
    }

    /// A world on `clock` with `npcs` stood on [`PLACE`] and its cells open, nobody on it yet.
    fn a_map(clock: &TestClock, npcs: Vec<Npc>) -> GameState {
        a_map_on(clock, npcs, open_cells())
    }

    /// The x of the band [`banded_cells`] blocks.
    const BAND: std::ops::Range<i32> = 3450..3550;

    /// [`PLACE`]'s cells, open but for the columns of [`BAND`], which block every row.
    fn banded_cells() -> Arc<HostedCells> {
        let side = SECTREE_CELLS;
        let cells: Vec<u32> = (0..side * side)
            .map(|at| {
                let x = i32::try_from(at % side).unwrap() * CELL_SIZE;
                if BAND.contains(&x) {
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
        Arc::new(hosted)
    }

    /// Whether [`banded_cells`] hold `(x, y)` movable.
    fn banded_movable(x: i32, y: i32) -> bool {
        (0..6400).contains(&x) && (0..6400).contains(&y) && !BAND.contains(&x)
    }

    /// As [`a_map`], on `cells`.
    fn a_map_on(clock: &TestClock, npcs: Vec<Npc>, cells: Arc<HostedCells>) -> GameState {
        let mut world = a_world(clock);
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

    /// The player 1 entered near [`SPOT`], its inbox drained. It is stored above its maximum
    /// stamina, so no refill reaches its inbox.
    fn a_player(world: &mut GameState) -> UnboundedReceiver<Vec<u8>> {
        let mut inbox = enter(world, 1, (3000, 3000), 12_345);
        drained(&mut inbox);
        inbox
    }

    /// `npc` alone near a player, with the world's dice seeded with `seed`, after Pulse 1 at
    /// [`T0`]. The dice returned are as they were before Pulse 1.
    fn first_idle(npc: Npc, seed: u64) -> (GameState, UnboundedReceiver<Vec<u8>>, Pcg32) {
        first_idle_on(npc, seed, open_cells())
    }

    /// As [`first_idle`], on `cells`.
    fn first_idle_on(
        npc: Npc,
        seed: u64,
        cells: Arc<HostedCells>,
    ) -> (GameState, UnboundedReceiver<Vec<u8>>, Pcg32) {
        let clock = TestClock::default();
        clock.set(T0);
        let mut world = a_map_on(&clock, vec![npc], cells);
        let inbox = a_player(&mut world);
        world.dice = Pcg32::new(seed, 0);
        let dice = world.dice.clone();
        world.process_pulse(1);
        (world, inbox, dice)
    }

    /// The first seed whose dice `holds`.
    fn a_seed(holds: impl Fn(&mut Pcg32) -> bool) -> u64 {
        (0..10_000)
            .find(|&seed| holds(&mut Pcg32::new(seed, 0)))
            .expect("a seed in 10,000")
    }

    fn mind(world: &GameState) -> Mind {
        *world.minds.get(ME).expect("a state machine")
    }

    fn quests() -> Quests {
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        Quests::new(manager, Arc::default(), MobNamesByLanguage::default())
    }

    /// The `FUNC_WAIT` an NPC's `Goto` tells its view.
    fn move_wait(mind: &Mind, now: u32) -> Vec<u8> {
        let rotation = (f32_to_i32(mind.rotation) / 5).to_le_bytes()[0];
        GcCharacterMove::new(
            FUNC_WAIT,
            0,
            rotation,
            ME,
            mind.motion.dest.0,
            mind.motion.dest.1,
            now,
            mind.motion.duration_ms,
        )
        .encode()
    }

    /// The point an idle walk of `distance` toward `degrees` reaches from [`SPOT`].
    fn ahead(degrees: i32, distance: i32) -> (i32, i32) {
        let (fx, fy) = delta_by_degree(i32_to_f32(degrees), i32_to_f32(distance));
        (SPOT.0 + f32_to_i32(fx), SPOT.1 + f32_to_i32(fy))
    }

    /// `CowardEscape` as legacy writes it (`G/char_state.cpp:282-321`), from [`SPOT`] over
    /// `movable`: where it runs, or `None`.
    fn coward_model(dice: &mut Pcg32, movable: impl Fn(i32, i32) -> bool) -> Option<(i32, i32)> {
        for (low, high) in [(3000, 5000), (1000, 3000), (500, 1000)] {
            for _ in 0..8 {
                let degrees = number(dice, 0, 359);
                let (dx, dy) = {
                    let distance = number(dice, low, high);
                    let (fx, fy) = delta_by_degree(i32_to_f32(degrees), i32_to_f32(distance));
                    (f32_to_i32(fx), f32_to_i32(fy))
                };
                if (1..=100).all(|j| movable(SPOT.0 + dx * j / 100, SPOT.1 + dy * j / 100)) {
                    return Some((SPOT.0 + dx, SPOT.1 + dy));
                }
            }
        }
        None
    }

    /// Whether the dice of a walking idle hold a walk: `draws` draws first, then 1 in 7.
    fn walks_after(dice: &mut Pcg32, draws: &[(i32, i32)]) -> bool {
        for &(low, high) in draws {
            let _ = number(dice, low, high);
        }
        number(dice, 0, 6) == 0
    }

    /// `StartStateMachine`: an NPC a player's view takes in starts on the Pulse after, and a
    /// warp never; an NPC already running keeps its next update; a sectree left with no player
    /// stops it, keeping its state machine, and a player seeing it again starts it once more.
    #[test]
    fn a_seen_npc_starts_on_the_next_pulse_and_stops_when_no_player_is_near() {
        let clock = TestClock::default();
        let stone = an_npc(ME, CHAR_TYPE_STONE, 8001, 0);
        let warp = an_npc(ME + 1, CHAR_TYPE_WARP, 20_018, 0);
        let mut world = a_map(&clock, vec![stone, warp]);
        for pulse in 1..=10 {
            world.process_pulse(pulse);
        }
        assert!(
            world.minds.get(ME).is_none(),
            "no player near: never started"
        );
        let _inbox = a_player(&mut world);
        assert!(
            world.minds.get(ME).is_none(),
            "the start waits for the next Pulse"
        );
        world.process_pulse(11);
        assert!(world.minds.is_running(ME));
        assert!(!world.minds.is_running(ME + 1));
        assert!(world.minds.get(ME + 1).is_none(), "a warp is never started");
        // Pulse 11 was the first update: Idle entered and run, a stone's second waited.
        assert_eq!(mind(&world).state, Fsm::Idle);
        assert_eq!(mind(&world).asked, None);
        assert_eq!(mind(&world).next_pulse, 11 + 25);
        assert_eq!(
            mind(&world).rotation.to_bits(),
            90_f32.to_bits(),
            "its spawn's heading"
        );
        let mut second = enter(&mut world, 2, (3100, 3100), 12_345);
        drained(&mut second);
        world.process_pulse(12);
        assert_eq!(
            mind(&world).next_pulse,
            36,
            "a running NPC keeps its next update"
        );
        let _ = world.leave_world(Vid::new(1));
        let _ = world.leave_world(Vid::new(2));
        world.process_pulse(13);
        assert!(!world.minds.is_running(ME));
        assert_eq!(
            mind(&world).next_pulse,
            36,
            "a stopped NPC keeps its state machine"
        );
        for pulse in 14..=40 {
            world.process_pulse(pulse);
        }
        assert_eq!(mind(&world).next_pulse, 36, "a stopped NPC is not updated");
        let _again = a_player(&mut world);
        world.process_pulse(41);
        assert!(world.minds.is_running(ME));
        assert_eq!(mind(&world).next_pulse, 41 + 25);
    }

    /// A stop asked inside a Pulse, before the state set's turn, keeps the NPC from that turn, as
    /// legacy's `DecreasePC` takes it out of the set at once; a start asked inside it counts from
    /// it.
    #[test]
    fn a_wake_asked_inside_a_pulse_counts_before_the_state_sets_turn() {
        let stone = an_npc(ME, CHAR_TYPE_STONE, 8001, 0);
        let (mut world, _, _) = first_idle(stone, 1);
        assert_eq!(mind(&world).next_pulse, 26);
        for pulse in 2..=25 {
            world.process_pulse(pulse);
        }
        let _ = world.leave_world(Vid::new(1));
        world.run_minds(26);
        assert!(!world.minds.is_running(ME));
        assert_eq!(mind(&world).next_pulse, 26, "stopped before its update");
        world.last_pulse = 26;
        let _again = a_player(&mut world);
        world.run_minds(26);
        assert_eq!(
            mind(&world).next_pulse,
            27,
            "started on Pulse 26, due on 27"
        );
        assert_eq!(mind(&world).state, Fsm::Idle);
    }

    /// `Goto` to where the NPC stands sets nothing; to the destination it already has, it asks
    /// for Move again with 4 Pulses unless it is moving, and tells nobody.
    #[test]
    fn a_goto_to_its_own_point_or_its_destination_sets_no_new_way() {
        let stone = an_npc(ME, CHAR_TYPE_STONE, 8001, 0);
        let (mut world, _, _) = first_idle(stone, 1);
        let before = mind(&world);
        assert!(!world.npc_goto(ME, SPOT.0, SPOT.1));
        assert_eq!(mind(&world).asked, before.asked);
        assert_eq!(mind(&world).duration, before.duration);
        assert!(world.npc_goto(ME, 3500, 3200));
        let set = mind(&world);
        assert_eq!(
            (set.asked, set.duration, set.motion.dest),
            (Some(Fsm::Move), 4, (3500, 3200))
        );
        let machine = world.minds.machines.get_mut(&ME).unwrap();
        machine.asked = None;
        machine.duration = 25;
        assert!(!world.npc_goto(ME, 3500, 3200));
        assert_eq!(
            (mind(&world).asked, mind(&world).duration),
            (Some(Fsm::Move), 4)
        );
        let machine = world.minds.machines.get_mut(&ME).unwrap();
        machine.state = Fsm::Move;
        machine.asked = None;
        machine.duration = 25;
        assert!(!world.npc_goto(ME, 3500, 3200));
        assert_eq!(
            (mind(&world).asked, mind(&world).duration),
            (None, 25),
            "already moving"
        );
        assert_eq!(mind(&world).motion, set.motion);
    }

    /// `UpdateStateMachine` runs a state only on its due Pulse: a stone's Idle every second.
    #[test]
    fn a_state_runs_only_on_its_due_pulse() {
        let stone = an_npc(ME, CHAR_TYPE_STONE, 8001, 0);
        let (mut world, mut inbox, _) = first_idle(stone, 1);
        for pulse in 2..=26 {
            world.process_pulse(pulse);
            let due = if pulse < 26 { 26 } else { 51 };
            assert_eq!(mind(&world).next_pulse, due, "after Pulse {pulse}");
        }
        assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new());
    }

    /// The idle waits: a non-player 5 seconds, a guard and a held NPC never walking even on dice
    /// that walk, a monster 1 to 3 seconds when aggressive and 3 to 5 when not, a held one never
    /// walking, and a horse race running nothing; each draws only what legacy draws.
    #[test]
    fn each_kind_of_npc_waits_its_own_idle_time() {
        // A non-player's draws: its chat's, a miss with no quests, then its walk's.
        let walking = a_seed(|dice| walks_after(dice, &[(0, 30)]));
        for (race, flags) in [
            (20_016, AIFLAG_NOMOVE),
            (11_000, 0),
            (11_002, 0),
            (11_004, 0),
        ] {
            let npc = an_npc(ME, CHAR_TYPE_NPC, race, flags);
            let (world, mut inbox, mut dice) = first_idle(npc, walking);
            assert_eq!(mind(&world).next_pulse, 1 + 125, "race {race}");
            assert_eq!(mind(&world).asked, None, "race {race} never walks");
            let _chat = number(&mut dice, 0, 30);
            assert_eq!(world.dice, dice, "race {race} draws its chat's odds alone");
            assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new());
        }
        let (world, _, dice) = first_idle(an_npc(ME, CHAR_TYPE_NPC, 20_105, 0), walking);
        assert_eq!(
            mind(&world).next_pulse,
            2,
            "a horse keeps the first duration"
        );
        assert_eq!(world.dice, dice, "a horse draws nothing");
        // A horse by its type, whatever its race.
        let (world, _, dice) = first_idle(an_npc(ME, CHAR_TYPE_HORSE, 101, 0), walking);
        assert_eq!(mind(&world).next_pulse, 2);
        assert_eq!(world.dice, dice, "a horse by type draws nothing");
        for (flags, low, high) in [
            (AIFLAG_NOMOVE | AIFLAG_AGGRESSIVE, 1, 3),
            (AIFLAG_NOMOVE, 3, 5),
        ] {
            // A held monster's draws: its wait, then its chat's odds, which a walk would take.
            let would_walk = a_seed(|dice| walks_after(dice, &[(low, high)]));
            for seed in (0..12).chain([would_walk]) {
                let monster = an_npc(ME, CHAR_TYPE_MONSTER, 101, flags);
                let (world, _, mut dice) = first_idle(monster, seed);
                let wait = number(&mut dice, low, high);
                assert_eq!(mind(&world).next_pulse, 1 + pulses(wait));
                assert_eq!(mind(&world).asked, None, "a held monster never walks");
                if number(&mut dice, 0, 60) == 0 {
                    let _line = number(&mut dice, 1, 3);
                }
                assert_eq!(world.dice, dice);
            }
        }
    }

    /// A monster's idle walk: one time in 7 it turns to a drawn heading, walks a drawn 300 to
    /// 700 units at the run, which its view is told as a `FUNC_WAIT`, steps every 4 Pulses to
    /// the share of the way the time reaches, and at the end waits 1 to 3 seconds.
    #[test]
    fn a_monster_wanders_one_time_in_seven_and_steps_to_its_point() {
        let seed = a_seed(|dice| {
            let _wait = number(dice, 3, 5);
            number(dice, 0, 6) == 0
        });
        let monster = an_npc(ME, CHAR_TYPE_MONSTER, 101, 0);
        let (mut world, mut inbox, mut dice) = first_idle(monster, seed);
        let wait = number(&mut dice, 3, 5);
        assert_eq!(number(&mut dice, 0, 6), 0);
        let degrees = number(&mut dice, 0, 359);
        let target = ahead(degrees, number(&mut dice, 300, 700));
        assert_eq!(world.dice, dice, "a walk draws no chat");
        let walk = mind(&world);
        assert_eq!(walk.rotation.to_bits(), i32_to_f32(degrees).to_bits());
        assert_eq!(walk.motion.start, SPOT);
        assert_eq!(walk.motion.dest, target);
        assert_eq!(walk.motion.start_ms, T0);
        assert_eq!(
            walk.motion.duration_ms,
            move_duration(SPOT, target, MOTION_SPEED_FALLBACK, 100)
        );
        assert_eq!((walk.state, walk.asked), (Fsm::Idle, Some(Fsm::Move)));
        assert_eq!(
            walk.next_pulse,
            1 + MOVE_STEP_PULSES,
            "not the drawn {wait} seconds"
        );
        assert!(!walk.walking, "a monster runs");
        assert_eq!(drained(&mut inbox), [move_wait(&walk, T0)]);
        let clock = TestClock::default();
        world.clock = super::super::motion::WorldClock::new(Box::new(clock.clone()));
        let half = T0 + walk.motion.duration_ms / 2;
        clock.set(half);
        for pulse in 2..=5 {
            world.process_pulse(pulse);
        }
        let (step, arrived) = walk.motion.at(half).unwrap();
        assert!(!arrived);
        assert_ne!(step, SPOT);
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!((npc.x, npc.y, npc.z), (step.0, step.1, 0));
        assert_eq!(
            world.maps[&PLACE]
                .spot(EntityKey::Npc(ME))
                .map(|s| (s.x, s.y)),
            Some(step)
        );
        let stepping = mind(&world);
        assert_eq!(stepping.state, Fsm::Move);
        assert_eq!(stepping.next_pulse, 9);
        assert_eq!(
            stepping.rotation.to_bits(),
            rotation_to_xy(step.0 - SPOT.0, step.1 - SPOT.1).to_bits()
        );
        clock.set(T0 + walk.motion.duration_ms);
        for pulse in 6..=9 {
            world.process_pulse(pulse);
        }
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!((npc.x, npc.y), target);
        let rest = mind(&world);
        assert_eq!((rest.state, rest.asked), (Fsm::Move, Some(Fsm::Idle)));
        assert_eq!(rest.next_pulse, 9 + pulses(number(&mut dice, 1, 3)));
        assert_eq!(
            drained(&mut inbox),
            Vec::<Vec<u8>>::new(),
            "a step tells nobody"
        );
        // The next way starts where this one ended, not at the spawn point.
        let back = (SPOT.0 + 100, SPOT.1);
        assert!(world.npc_goto(ME, back.0, back.1));
        let next = mind(&world);
        assert_eq!((next.motion.start, next.motion.dest), (target, back));
        assert_eq!(
            next.motion.duration_ms,
            move_duration(target, back, MOTION_SPEED_FALLBACK, 100)
        );
    }

    /// A stunned monster's Idle returns before any draw and keeps the wait it had
    /// (`G/char_state.cpp:628-629`); the same monster unstunned draws its wait.
    #[test]
    fn a_stunned_monster_idles_with_no_draw() {
        let mut stunned = an_npc(ME, CHAR_TYPE_MONSTER, 101, 0);
        stunned.stunned = true;
        let (world, mut inbox, dice) = first_idle(stunned, 1);
        assert_eq!(world.dice, dice);
        assert_eq!(mind(&world).next_pulse, 2, "the first duration stays");
        assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new());
        let (world, _, mut dice) = first_idle(an_npc(ME, CHAR_TYPE_MONSTER, 101, 0), 1);
        let wait = number(&mut dice, 3, 5);
        assert_eq!(mind(&world).next_pulse, 1 + pulses(wait));
        assert_ne!(world.dice, Pcg32::new(1, 0));
    }

    /// A dead NPC's `UpdateStateMachine` returns before the state runs, and its next update
    /// stays on the Pulse it was (`G/char.cpp:5343-5344`).
    #[test]
    fn a_dead_npc_runs_no_state() {
        let mut dead = an_npc(ME, CHAR_TYPE_MONSTER, 101, 0);
        dead.dead = true;
        let (mut world, mut inbox, dice) = first_idle(dead, 1);
        for pulse in 2..=30 {
            world.process_pulse(pulse);
        }
        assert_eq!(world.dice, dice);
        let mind = mind(&world);
        assert_eq!((mind.next_pulse, mind.state), (1, Fsm::Initial));
        assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new());
    }

    /// `StateMove` has no stun guard: a monster stunned mid-walk still reaches its point, and
    /// then idles with no draw.
    #[test]
    fn a_monster_stunned_mid_walk_still_arrives() {
        let seed = a_seed(|dice| {
            let _wait = number(dice, 3, 5);
            number(dice, 0, 6) == 0
        });
        let monster = an_npc(ME, CHAR_TYPE_MONSTER, 101, 0);
        let (mut world, _, _) = first_idle(monster, seed);
        let walk = mind(&world);
        assert_eq!(walk.asked, Some(Fsm::Move));
        world.npc_mut(ME).unwrap().stunned = true;
        let clock = TestClock::default();
        world.clock = super::super::motion::WorldClock::new(Box::new(clock.clone()));
        clock.set(T0 + walk.motion.duration_ms);
        for pulse in 2..=5 {
            world.process_pulse(pulse);
        }
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!((npc.x, npc.y), walk.motion.dest);
        let rest = mind(&world);
        assert_eq!((rest.state, rest.asked), (Fsm::Move, Some(Fsm::Idle)));
        let dice = world.dice.clone();
        world.process_pulse(rest.next_pulse);
        assert_eq!(world.dice, dice, "the stunned Idle draws nothing");
        assert_eq!(mind(&world).state, Fsm::Idle);
    }

    /// A player shown an NPC mid-walk is sent it where it stands, with its move and its walk
    /// mode (D2), and once it has arrived only where it stands.
    #[test]
    fn a_player_shown_a_walking_npc_is_sent_its_move() {
        let seed = a_seed(|dice| {
            let _wait = number(dice, 3, 5);
            number(dice, 0, 6) == 0
        });
        let monster = an_npc(ME, CHAR_TYPE_MONSTER, 101, 0);
        let (world, _, _) = first_idle(monster, seed);
        let mut world = world;
        let walk = mind(&world);
        let now = T0 + 40;
        let clock = TestClock::default();
        clock.set(now);
        world.clock = super::super::motion::WorldClock::new(Box::new(clock.clone()));
        let (_, shown) = enter_seeing(&mut world, PLACE, 2, (3100, 3100), 12_345);
        let insert = encode_npc_insert(world.npc_by_vid(ME).unwrap(), Some(&walk), now);
        assert_eq!(insert.len(), 3, "the add, the move and the walk mode");
        let added = protocol::gc_actors::GcCharacterAdd::decode(&insert[0]).expect("an add");
        assert_eq!(
            added.angle.to_bits(),
            walk.rotation.to_bits(),
            "its walk's heading"
        );
        assert_ne!(walk.rotation.to_bits(), 90_f32.to_bits(), "not the spawn's");
        let moving = GcCharacterMove::decode(&insert[1]).expect("a move");
        assert_eq!(
            (moving.b_func, moving.dw_vid, (moving.l_x, moving.l_y)),
            (FUNC_MOVE, ME, walk.motion.dest)
        );
        assert_eq!(insert[2], walk_mode(ME, 0), "a monster runs");
        assert!(
            shown.windows(3).any(|window| window == insert.as_slice()),
            "the insert is sent as encoded"
        );
        let (_, shown_late) = {
            clock.set(T0 + walk.motion.duration_ms + 1);
            enter_seeing(&mut world, PLACE, 3, (3100, 3100), 12_345)
        };
        let late = encode_npc_insert(
            world.npc_by_vid(ME).unwrap(),
            Some(&walk),
            T0 + walk.motion.duration_ms + 1,
        );
        assert_eq!(late.len(), 1, "an ended move sends the add alone");
        let added = protocol::gc_actors::GcCharacterAdd::decode(&late[0]).expect("an add");
        assert_eq!(
            (added.x, added.y),
            walk.motion.dest,
            "at its destination, though its last step is still to come"
        );
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!((npc.x, npc.y), SPOT);
        assert!(shown_late.contains(&late[0]));
    }

    /// A non-player's idle walk: 200 to 400 units at a walk, whose first sets the walk mode its
    /// view is told once, before the `FUNC_WAIT`; `no_wander` does not hold it.
    #[test]
    fn a_non_player_walks_and_tells_its_view_the_walk_mode_once() {
        let seed = a_seed(|dice| {
            // The chat's draw, a miss with no quests, then the walk's.
            let _chat = number(dice, 0, 30);
            number(dice, 0, 6) == 0
        });
        let npc = an_npc(ME, CHAR_TYPE_NPC, 20_016, 0);
        let clock = TestClock::default();
        clock.set(T0);
        let mut world = a_map(&clock, vec![npc]).with_no_wander(true);
        let mut inbox = a_player(&mut world);
        world.dice = Pcg32::new(seed, 0);
        let mut dice = world.dice.clone();
        world.process_pulse(1);
        let _chat = number(&mut dice, 0, 30);
        let _walks = number(&mut dice, 0, 6);
        let degrees = number(&mut dice, 0, 359);
        let target = ahead(degrees, number(&mut dice, 200, 400));
        let walk = mind(&world);
        assert_eq!(walk.motion.dest, target);
        assert!(walk.walking);
        assert_eq!(
            drained(&mut inbox),
            [walk_mode(ME, WALKMODE_WALK), move_wait(&walk, T0)]
        );
        world.set_now_walking(ME, true);
        assert_eq!(
            drained(&mut inbox),
            Vec::<Vec<u8>>::new(),
            "no change, no record"
        );
        world.set_now_walking(ME, false);
        assert_eq!(drained(&mut inbox), [walk_mode(ME, 0)]);
    }

    /// `no_wander` keeps every monster where it stands, and a walk whose point or half-way point
    /// is not movable is not taken.
    #[test]
    fn no_wander_and_a_blocked_point_keep_a_monster_still() {
        let seed = a_seed(|dice| {
            let _wait = number(dice, 3, 5);
            number(dice, 0, 6) == 0
        });
        let clock = TestClock::default();
        let monster = an_npc(ME, CHAR_TYPE_MONSTER, 101, 0);
        let mut world = a_map(&clock, vec![monster.clone()]).with_no_wander(true);
        let mut inbox = a_player(&mut world);
        world.dice = Pcg32::new(seed, 0);
        world.process_pulse(1);
        assert_eq!(mind(&world).asked, None);
        assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new());
        // Far outside the open cells, though inside the sectrees: nothing is movable.
        let mut far = monster;
        far.x = 30_000;
        far.y = 30_000;
        let mut world = a_map(&clock, vec![far]);
        let mut inbox = enter(&mut world, 1, (30_100, 30_100), 12_345);
        drained(&mut inbox);
        world.dice = Pcg32::new(seed, 0);
        world.process_pulse(1);
        let still = mind(&world);
        assert_eq!(still.asked, None);
        assert_eq!(still.motion.dest, (30_000, 30_000));
        assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new());
    }

    /// `CowardEscape`: a coward tries eight headings at 3,000 to 5,000 units, then at 1,000 to
    /// 3,000, then at 500 to 1,000, and runs to the first whose every hundredth of the way is
    /// movable, telling its view; with no such way it stays.
    #[test]
    fn a_coward_runs_along_a_movable_way_or_stays() {
        let open = |x: i32, y: i32| (0..6400).contains(&x) && (0..6400).contains(&y);
        let reach = |target: Option<(i32, i32)>| {
            target.map(|(x, y)| {
                let length = f64::from(x - SPOT.0).hypot(f64::from(y - SPOT.1));
                [999.0, 2999.0]
                    .iter()
                    .filter(|&&floor| length > floor)
                    .count()
            })
        };
        // A seed for each distance a way is found at on the banded cells. From the middle of the
        // cells some way 500 to 1,000 long is always open; the stuck coward below has none.
        let seeds: Vec<u64> = [Some(0), Some(1), Some(2)]
            .into_iter()
            .map(|wanted| a_seed(|dice| reach(coward_model(dice, banded_movable)) == wanted))
            .chain(0..8)
            .collect();
        let mut distances = BTreeSet::new();
        for seed in seeds {
            for banded in [false, true] {
                let coward = an_npc(ME, CHAR_TYPE_MONSTER, 101, AIFLAG_COWARD);
                let cells = if banded { banded_cells() } else { open_cells() };
                let (world, mut inbox, mut dice) = first_idle_on(coward, seed, cells);
                let expected = if banded {
                    coward_model(&mut dice, banded_movable)
                } else {
                    coward_model(&mut dice, open)
                };
                assert_eq!(world.dice, dice, "seed {seed}");
                let run = mind(&world);
                let Some(target) = expected else {
                    assert_eq!(run.asked, None, "seed {seed}: no way");
                    continue;
                };
                assert_eq!(run.motion.dest, target, "seed {seed}, banded {banded}");
                assert_eq!(
                    (run.asked, run.duration),
                    (Some(Fsm::Move), MOVE_STEP_PULSES)
                );
                assert_eq!(drained(&mut inbox), [move_wait(&run, T0)]);
                distances.insert(reach(Some(target)));
            }
        }
        assert_eq!(
            distances.len(),
            3,
            "the seeds reach each distance: {distances:?}"
        );
        let clock = TestClock::default();
        let mut stuck = an_npc(ME, CHAR_TYPE_MONSTER, 101, AIFLAG_COWARD);
        stuck.x = 30_000;
        stuck.y = 30_000;
        let mut world = a_map(&clock, vec![stuck]);
        let _inbox = enter(&mut world, 1, (30_100, 30_100), 12_345);
        world.process_pulse(1);
        let stays = mind(&world);
        assert_eq!(stays.asked, None);
        assert_eq!(stays.next_pulse, 2, "no way: the first duration stays");
    }

    /// An idle walk is taken only when both its point and its half-way point are movable.
    #[test]
    fn an_idle_walk_needs_its_point_and_its_half_way_point_movable() {
        let stone = an_npc(ME, CHAR_TYPE_STONE, 8001, 0);
        let (mut world, _, _) = first_idle_on(stone, 1, banded_cells());
        let mut toward = |degrees: f32, distance: f32| {
            world.minds.machines.get_mut(&ME).unwrap().rotation = degrees;
            world.wander_target(ME, PLACE.1, distance)
        };
        // East: the half-way point at 3500 is in the band, the point at 3800 is not.
        assert_eq!(toward(90.0, 600.0), None);
        // The point at 3500 is in the band, the half-way point at 3350 is not.
        assert_eq!(toward(90.0, 300.0), None);
        assert_eq!(toward(90.0, 200.0), Some((3400, 3200)));
        assert_eq!(toward(270.0, 600.0), Some((2600, 3200)));
        assert_eq!(toward(90.0, 4000.0), None, "off the cells");
    }

    /// `MonsterChat(MONSTER_CHAT_WAIT)`: a monster says one of its race's three waiting lines,
    /// a non-player any of its lines, as a talk with its VID to its view.
    #[test]
    fn an_idle_npc_says_a_line_of_its_race() {
        let quests = quests();
        let host = quests.manager().host();
        assert_eq!(host.monster_chat_count(301), Some(5));
        let seed = a_seed(|dice| {
            let _wait = number(dice, 3, 5);
            number(dice, 0, 60) == 0
        });
        let monster = an_npc(ME, CHAR_TYPE_MONSTER, 301, AIFLAG_NOMOVE);
        let clock = TestClock::default();
        clock.set(T0);
        let mut world = a_map(&clock, vec![monster]).with_quests(quests);
        let mut inbox = a_player(&mut world);
        world.dice = Pcg32::new(seed, 0);
        let mut dice = world.dice.clone();
        world.process_pulse(1);
        let _wait = number(&mut dice, 3, 5);
        let _speaks = number(&mut dice, 0, 60);
        let index = number(&mut dice, 1, 3);
        let line = world
            .quests
            .as_ref()
            .unwrap()
            .manager()
            .host()
            .monster_chat_line(301, index);
        assert!(!line.is_empty());
        let mut text = line;
        text.push(0);
        let said = GcChat {
            chat_type: CHAT_TYPE_TALKING,
            id: ME,
            empire: 0,
            can_format: true,
            text,
        }
        .encode()
        .unwrap();
        assert_eq!(drained(&mut inbox), [said]);

        // Race 101 has no table: the expression gives `''`, and an empty line is not said
        // (`G/char.cpp:7591-7592`), though the index was drawn.
        let monster = an_npc(ME, CHAR_TYPE_MONSTER, 101, AIFLAG_NOMOVE);
        let mut world = a_map(&clock, vec![monster]).with_quests(world.quests.take().unwrap());
        let mut inbox = a_player(&mut world);
        world.dice = Pcg32::new(seed, 0);
        let mut dice = world.dice.clone();
        world.process_pulse(1);
        let _wait = number(&mut dice, 3, 5);
        assert_eq!(number(&mut dice, 0, 60), 0, "it would speak");
        let _index = number(&mut dice, 1, 3);
        assert_eq!(world.dice, dice);
        assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new(), "nothing to say");

        let seed = a_seed(|dice| number(dice, 0, 30) == 0);
        let npc = an_npc(ME, CHAR_TYPE_NPC, 20_018, AIFLAG_NOMOVE);
        let mut world = a_map(&clock, vec![npc]).with_quests(world.quests.take().unwrap());
        let mut inbox = a_player(&mut world);
        world.dice = Pcg32::new(seed, 0);
        let mut dice = world.dice.clone();
        world.process_pulse(1);
        let _speaks = number(&mut dice, 0, 30);
        let index = number(&mut dice, 1, 5);
        let host = world.quests.as_ref().unwrap().manager().host();
        let mut text = host.monster_chat_line(20_018, index);
        assert!(!text.is_empty());
        text.push(0);
        let records = drained(&mut inbox);
        assert_eq!(records.len(), 1);
        assert_eq!(
            GcChat::decode(&records[0]).map(|chat| (chat.id, chat.text)),
            Ok((ME, text))
        );

        // A guard speaks one time in 7: here on a draw where one in 31 would keep it silent.
        let seed = a_seed(|dice| {
            let mut odds_of_others = dice.clone();
            number(dice, 0, 6) == 0 && number(&mut odds_of_others, 0, 30) != 0
        });
        let guard = an_npc(ME, CHAR_TYPE_NPC, 11_000, 0);
        let mut world = a_map(&clock, vec![guard]).with_quests(world.quests.take().unwrap());
        let mut inbox = a_player(&mut world);
        world.dice = Pcg32::new(seed, 0);
        let mut dice = world.dice.clone();
        world.process_pulse(1);
        let _speaks = number(&mut dice, 0, 6);
        let host = world.quests.as_ref().unwrap().manager().host();
        let count = host.monster_chat_count(11_000).unwrap();
        let mut text = host.monster_chat_line(11_000, number(&mut dice, 1, count));
        assert!(!text.is_empty());
        text.push(0);
        let records = drained(&mut inbox);
        assert_eq!(records.len(), 1, "a guard's one in 7");
        assert_eq!(
            GcChat::decode(&records[0]).map(|chat| (chat.id, chat.text)),
            Ok((ME, text))
        );
        assert_eq!(world.dice, dice, "a guard never walks");
    }

    /// A destroyed NPC and one its map is stood again over are forgotten, and a step where no
    /// sectree stands leaves an NPC where it was (a Defect fixed).
    #[test]
    fn a_destroyed_npc_is_forgotten_and_a_step_off_the_sectrees_is_not_taken() {
        let monster = an_npc(ME, CHAR_TYPE_MONSTER, 101, AIFLAG_NOMOVE);
        let (mut world, _, _) = first_idle(monster, 1);
        world.npc_move(ME, 70_000, 70_000);
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!((npc.x, npc.y), SPOT);
        world.npc_move(ME, 3300, 3100);
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!((npc.x, npc.y), (3300, 3100));
        world.destroy_npc(ME);
        assert!(world.minds.get(ME).is_none());
        assert!(!world.minds.is_running(ME));

        // A map stood again forgets the characters it replaced.
        let monster = an_npc(ME, CHAR_TYPE_MONSTER, 101, AIFLAG_NOMOVE);
        let (mut world, _, _) = first_idle(monster, 1);
        assert!(world.minds.is_running(ME));
        world.stand_npcs(PLACE.0, &region(), MapNpcs::default());
        assert!(world.minds.get(ME).is_none());
        assert!(!world.minds.is_running(ME));
    }

    /// `GetDeltaByDegree`: north is +y, east is +x, in `float`.
    #[test]
    fn the_delta_by_degree_is_clockwise_from_north() {
        let (x, y) = delta_by_degree(0.0, 500.0);
        assert_eq!((f32_to_i32(x), f32_to_i32(y)), (0, 500));
        let (x, y) = delta_by_degree(90.0, 500.0);
        assert_eq!((f32_to_i32(x), f32_to_i32(y)), (500, 0));
        let (x, y) = delta_by_degree(225.0, 400.0);
        assert_eq!((f32_to_i32(x), f32_to_i32(y)), (-282, -282));
        assert_eq!(pulses(3), 75);
        assert_eq!(pulses(-1), 0);
    }
}
