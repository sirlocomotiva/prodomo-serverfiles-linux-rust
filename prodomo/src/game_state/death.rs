//! A player's death and restart: the victim's side of `CHARACTER::Damage`
//! (`G/char_battle.cpp:2248-2433`), `Stun` and `StunEvent` (`:402-459`), the player arms of `Dead`
//! (`:1238-1696`) and `dead_event` (`:473-525`), `DeathPenalty` (`:330-392`), the restart
//! commands `do_restart_now` and `do_restart` (`G/cmd_general.cpp:471-706`), the player's
//! `recovery_event` with `CheckTarget` (`G/char.cpp:3245-3314`, `:6663-6670`), and a player's own
//! `ClearSync` (`G/char.cpp:5549-5575`).
//!
//! # What legacy does
//!
//! A hit that leaves a player no hit points stuns it: its potion pools empty, its recovery ends,
//! `GC_STUN` goes to its view, and `StunEvent` is due 3 seconds later. The event runs `Dead`, as a
//! second hit on the stunned player does at once:
//!
//! 1. the killer is the hitter, or else the player whose PID it kept, and the PID is reset; a
//!    player killer whose target it is drops it;
//! 2. the death penalty flag is set for a non-player killer and cleared otherwise;
//! 3. `ClearSync` tells the view the player has no owner, and releases what it owns;
//! 4. the session enters `PHASE_DEAD` (`GC_PHASE` 6), `GC_DEAD` goes to the view, and the running
//!    affects are sent again;
//! 5. `dead_event` is due 180 seconds later, and the trade, the shop and the safebox close.
//!
//! While dead, the hit points, spell points and stamina do not change, a move shows the body
//! back where it lies, and a newcomer sees it lying dead. The player leaves the state by a
//! restart command or by `dead_event`:
//!
//! - `/restart_here`, from 9.04 seconds after the death: back in game where it lies, with 50 hit
//!   points, the death penalty and a new revive-invisible affect;
//! - `/restart_town`, from 6.04 seconds: warped to its map's town with 50 hit points;
//! - `/restart_now`, at once, for 300 000 gold: where it lies, at full hit points, with the
//!   penalty and the revive;
//! - `dead_event`, 180 seconds after the death: warped to town with half its hit points.
//!
//! A restart command from a living player only closes the client's restart window and starts
//! its recovery. The death penalty takes a level's share of the next level's cost, at most
//! 800 000 (see [`world::character::death_penalty_loss`]), from a player of level 10 or more,
//! one time in three.
//!
//! # Divergences and Defects fixed
//!
//! See V18 and Defects B5, B8, B11, B12, B15, B24, B25, D2, D6, D7, D9 and D10 in
//! `docs/STATUS.md`. In short: the town arms write their records before the warp; the penalty
//! tests its flag before it rolls, and the town restarts take none; a recall point no Channel
//! serves restarts in place; a disconnect while down stands the player with 50 hit points and
//! the penalty; a stunned player uses no item; and a dead one uses, drops and picks up none,
//! starts no trade or click, and cannot sit up.
//!
//! # Not yet
//!
//! - The quest `die` trigger, `ItemDropPenalty`, the player-against-player, guild war, duel, arena
//!   and siege hooks, the party, the horse and the mount.
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use common::enums::EWearPositions;
use common::levels;
use common::point_slot as point;
use common::vid::Vid;
use gamedata::map_atlas::MapAtlas;
use protocol::gc_actors::GcCharacterGoldChange;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use protocol::gc_inventory::{HEADER_GC_DEAD, HEADER_GC_PHASE};
use protocol::gc_nested::GcDamageInfo;
use protocol::gc_small::GcHeaderAndByte;
use tracing::{debug, warn};
use world::character::{
    death_penalty_loss, number, pc_poison_rate, Equipment, PointRecord, Points, DAMAGE_POISON,
};

use super::affect::{affect_add_frame, affect_remove_frame, Affect};
use super::fight::{header_and_vid, DAMAGE_BLOCK};
use super::pc_heal::pc_heal;
use super::regen::{pulses, Due};
use super::view::EntityKey;
use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::client_registry::ClientOrder;
use crate::game_loop_messages::GroundPlace;
use crate::game_loop_messages::{Departure, RestartAnswer, RestartKind};
use crate::item_move::{gold_delta, Mover};
use crate::loading_phase::{point_changes, PHASE_DEAD, PHASE_GAME, PK_MODE_PEACE, PK_MODE_PROTECT};
use crate::save::PASSES_PER_SEC;
use crate::sync_position::{distance_approx, ownership_record};
use crate::warp;

/// `PASSES_PER_SEC(180)`: the wait before `dead_event` restarts a dead player in town (`:1594`).
const DEAD_SECONDS: u32 = 180;

/// `PASSES_PER_SEC(3)`: a player's recovery event runs every 3 seconds (`G/char.cpp:3311-3313`).
const RECOVERY_SECONDS: u32 = 3;

/// `PASSES_PER_SEC(3)`: a player's poison event ticks every 3 seconds
/// (`G/char_resist.cpp:71`).
pub(super) const POISON_TICK_SECONDS: u32 = 3;

/// `AFF_POISON` (`G/affect.h:214`) as a word-0 bit: flag 5 is bit 4.
const AFF_POISON_WORD: u32 = 1 << 4;

/// `eFRS_HERESEC`: `/restart_here` is refused while more seconds than this are left of the dead
/// event (`G/cmd_general.cpp:552`).
const HERE_SECONDS: u64 = 170;

/// `eFRS_TOWNSEC`: `/restart_town` is refused while more seconds than this are left
/// (`G/cmd_general.cpp:572`).
const TOWN_SECONDS: u64 = 173;

/// What `/restart_now` costs (`G/cmd_general.cpp:480`, `:519`).
const RESTART_NOW_GOLD: u64 = 300_000;

/// `CheckTarget` drops a target this far away or farther (`G/char.cpp:6668`).
const TARGET_REACH: i32 = 4_800;

/// The battle arena's maps, where a death costs nothing (`G/BattleArena.h:2`,
/// `G/char_battle.cpp:342-345`).
const ARENA_MAPS: [i32; 3] = [190, 191, 192];

/// `DeathPenalty` spares a player below this level (`G/char_battle.cpp:347`).
const PENALTY_LEVEL: u8 = 10;

/// `UNIQUE_ITEM_TEARDROP_OF_GODNESS` (`G/unique_item.h:18`): worn, it halves the loss.
const TEARDROP_OF_GODNESS: u32 = 70_012;

/// The hit points a here or town restart leaves (`G/cmd_general.cpp:691`, `:698`).
const RESTART_HP: i32 = 50;

/// `"CloseRestartWindow"`, the command line every restart and `dead_event` sends.
const CLOSE_RESTART_WINDOW: &[u8] = b"CloseRestartWindow";

/// `[LS;901]`: the death cost no experience.
const NO_PENALTY_NOTICE: &[u8] = b"[LS;901]";

/// `[LS;516;%d]`: the seconds a restart still waits.
const RESTART_WAIT_NOTICE: &[u8] = b"[LS;516;%d]";

/// What `/restart_now` says to a player short of its price (`G/cmd_general.cpp:482`).
const RESTART_NOW_PRICE_NOTICE: &[u8] = b"@@(cmd_general)tradus:You have to pay 300000 Gold";

/// What legacy's `CHARACTER` keeps of a player's fight beyond its points.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PcCombat {
    /// `INSTANT_FLAG_STUN`: stunned, waiting for `Dead`.
    stunned: bool,
    /// `POS_DEAD` and `ADD_CHARACTER_STATE_DEAD`.
    pub(super) dead: bool,
    /// `m_dwKillerPID`: the player whose hit stunned it, which `Dead` reads and resets.
    killer_pid: u32,
    /// `INSTANT_FLAG_DEATH_PENALTY`: the next `DeathPenalty` may take experience.
    pub(super) death_penalty: bool,
    /// The Pulse `m_pkStunEvent` is due on.
    stun_due: Option<u64>,
    /// The Pulse `m_pkDeadEvent` is due on.
    dead_due: Option<u64>,
    /// The Pulse `m_pkRecoveryEvent` is due on.
    recovery_due: Option<u64>,
    /// The Pulse `m_pkPoisonEvent` is due on, with the ticks it has left.
    pub(super) poison_due: Option<(u64, u32)>,
    /// `m_dwLastDeadTime` (`G/char.cpp:1126`), stamped by `Dead`: `CanSummon` reads it.
    /// `None` is the constructor seed (`now - 180000`), which fails the +180 check the
    /// same way an old stamp does.
    pub(super) last_dead_ms: Option<u32>,
    /// `m_dwUnderGuildWarInfoMessageTime` (`G/char.h:1147`): the boot-millisecond clock
    /// before which the war-reward line stays silent. `None` is the constructor 0, which
    /// speaks at once.
    pub(super) war_reward_mute_until_ms: Option<u32>,
}

impl PcCombat {
    /// `IsStun() || IsDead()`.
    const fn down(&self) -> bool {
        self.stunned || self.dead
    }
}

/// Who struck a player: legacy's `LPCHARACTER pkKiller`, of which `Dead` reads whether it is a
/// player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Attacker {
    /// The attacker's VID, which is a player's PID.
    pub(super) vid: Vid,
    /// Whether the attacker is a player.
    pub(super) pc: bool,
}

/// What [`GameState::damage_pc_head`] decided: the victim is gone (no combat row or no
/// points), died by `Dead`, was already dead, or is open for the hit-point change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PcDamageHead {
    /// No combat row or no points: nothing ran.
    Gone,
    /// Stunned: `Dead` ran.
    Died,
    /// Already dead: nothing ran.
    Dead,
    /// Live: the head sent the damage packet; the tail changes `hp` by `damage`.
    Open {
        /// Hit points the head read.
        hp: i32,
        /// The poison-clamped damage the head showed.
        damage: i32,
    },
}

/// Where a recall may send a player: the atlas, and the maps each Channel and the Shared Channel
/// serve, which `CMapLocation::Get` tests a destination against.
#[derive(Debug, Default)]
pub(super) struct Recall {
    atlas: Option<Arc<MapAtlas>>,
    channel_maps: HashMap<u8, HashSet<u32>>,
    shared: HashSet<u32>,
}

impl Recall {
    /// Whether a client on `channel` can be sent to map `index`: its Channel serves it, or the
    /// Shared Channel does (`MapLocations::get`).
    fn serves(&self, channel: u8, index: i32) -> bool {
        let Some(index) = u32::try_from(index).ok().filter(|&index| index != 0) else {
            return false;
        };
        self.channel_maps
            .get(&channel)
            .is_some_and(|maps| maps.contains(&index))
            || self.shared.contains(&index)
    }
}

impl GameState {
    /// Share the map atlas and the maps each Channel serves, where a town restart and
    /// `dead_event` find the player's town. Without them every recall restarts in place (B12).
    #[must_use]
    pub fn with_recall(
        mut self,
        atlas: Arc<MapAtlas>,
        channel_maps: &HashMap<u8, Vec<u32>>,
        shared: &[u32],
    ) -> Self {
        self.recall = Recall {
            atlas: Some(atlas),
            channel_maps: channel_maps
                .iter()
                .map(|(&channel, maps)| (channel, maps.iter().copied().collect()))
                .collect(),
            shared: shared.iter().copied().collect(),
        };
        self
    }

    /// Set `gPlayerMaxLevel`, the configured `game.max_level`, which the experience arms read.
    #[must_use]
    pub const fn with_max_level(mut self, max_level: i32) -> Self {
        self.exp_limits.max_level = max_level;
        self
    }

    /// Set `g_iStatusPointGetLevelLimit`, the configured
    /// `game.status_point_get_level_limit`, which the experience gain reads.
    #[must_use]
    pub const fn with_status_point_get_level_limit(mut self, limit: i32) -> Self {
        self.exp_limits.status_point_get_level_limit = limit;
        self
    }

    /// Set `PK_PROTECT_LEVEL`, below which a character is in the protect PK mode.
    ///
    /// The configured `game.pk_protect_level`. Until this is called the level is 15.
    #[must_use]
    pub const fn with_protect_level(mut self, level: u8) -> Self {
        self.protect_level = level;
        self
    }

    /// `IsStun() || IsDead()` of the player under `vid`.
    pub(super) fn is_down(&self, vid: u32) -> bool {
        self.pc_combat.get(&vid).is_some_and(PcCombat::down)
    }

    /// `IsDead()` of the player under `vid`.
    pub(super) fn is_dead(&self, vid: u32) -> bool {
        self.pc_combat.get(&vid).is_some_and(|combat| combat.dead)
    }

    /// `IsStun()` of the player under `vid`.
    pub(super) fn is_stunned(&self, vid: u32) -> bool {
        self.pc_combat
            .get(&vid)
            .is_some_and(|combat| combat.stunned)
    }

    /// The points of the player under `vid`, to change.
    pub(super) fn pc_points_mut(&mut self, vid: u32) -> Option<&mut Points> {
        self.characters
            .find_by_vid_mut(Vid::new(vid))
            .ok()?
            .items_and_points_mut()
            .1
    }

    /// `GiveExp`'s `PointChange(POINT_EXP, amount, true)` on the player under `vid`
    /// (`G/char_battle.cpp:2675`, the live grant without the conqueror gate): the records go to
    /// the viewers a broadcast record names, each level-up raises the quest `levelup` event
    /// (`G/char.cpp:4051`), and a level that reaches the protect level switches the body's PK
    /// mode from `PK_MODE_PROTECT` to `PK_MODE_PEACE` with an update (`SetLevel`,
    /// `G/char.cpp:2221`).
    pub(super) fn gain_pc_exp(&mut self, vid: Vid, amount: i64) {
        let limits = self.exp_limits;
        let Some(skill_group) = self.bodies.get(&vid).map(|body| body.card.skill_group) else {
            return;
        };
        let Self {
            characters, dice, ..
        } = self;
        let Ok(character) = characters.find_by_vid_mut(vid) else {
            return;
        };
        let Some(points) = character.items_and_points_mut().1 else {
            return;
        };
        let outcome = points.gain_exp(amount, skill_group, limits, dice);
        self.send_point_records(vid, &outcome.records);
        for _ in 0..outcome.level_ups {
            self.raise_quest_levelup(vid);
        }
        let crossed = outcome.level_before < self.protect_level
            && outcome.level_after >= self.protect_level
            && self
                .bodies
                .get(&vid)
                .is_some_and(|body| body.card.pk_mode == PK_MODE_PROTECT);
        if crossed {
            if let Some(body) = self.bodies.get_mut(&vid) {
                body.card.pk_mode = PK_MODE_PEACE;
            }
            let flags = self.affect_flags_of(vid);
            self.send_affect_update(vid, flags);
        }
        if outcome.level_after != outcome.level_before {
            if let Some(body) = self.bodies.get_mut(&vid) {
                body.card.level = outcome.level_after;
            }
        }
    }

    /// One quest `levelup` raise for the player under `vid`, which just leveled up: its
    /// scripts' records go to its own client.
    fn raise_quest_levelup(&mut self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let Some((channel, map)) = self.place_of(vid.raw()) else {
            return;
        };
        let (x, y) = self
            .spot_of(vid.raw())
            .map_or((0, 0), |spot| (spot.x, spot.y));
        let place = GroundPlace { channel, map, x, y };
        for record in self.quest_levelup(vid, place, mover) {
            let _sent = self.write_to_client(vid, record);
        }
    }

    /// Writes each point change to the player's own client, and a broadcast record to every
    /// character that sees it (`UpdatePointsPacket`'s `PacketAround`, `G/char.cpp:2112-2115`).
    pub(super) fn send_point_records(&self, vid: Vid, records: &[PointRecord]) {
        for (record, frame) in records.iter().zip(point_changes(records, vid.raw())) {
            if record.broadcast {
                self.packet_around(EntityKey::Character(vid.raw()), &frame, None);
            } else {
                let _sent = self.write_to_client(vid, frame);
            }
        }
    }

    /// `PointChange(kind, amount, show_amount)` on the player under `vid`, its records to its
    /// client.
    pub(super) fn change_pc_point(
        &mut self,
        vid: Vid,
        kind: usize,
        amount: i32,
        show_amount: bool,
    ) {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let records = points
            .point_change(kind, amount, show_amount, false)
            .unwrap_or_default();
        self.send_point_records(vid, &records);
    }

    /// `PointChange(POINT_HP, hp - GetHP())`: sets the hit points of the player under `vid`.
    fn set_pc_hp(&mut self, vid: Vid, hp: i32, show_amount: bool) {
        let Some(current) = self.pc_points_mut(vid.raw()).map(|points| points.hp()) else {
            return;
        };
        self.change_pc_point(
            vid,
            point::POINT_HP,
            hp.saturating_sub(current),
            show_amount,
        );
    }

    /// What a world step that no command carries says to the player under `vid`: its empire,
    /// language and PK mode from its body, and its affect flags.
    pub(super) fn own_mover(&self, vid: Vid) -> Option<Mover> {
        let body = self.bodies.get(&vid)?;
        Some(Mover {
            recently_fought: false,
            empire: body.card.empire,
            language: body.card.language,
            pk_mode: body.card.pk_mode,
            affect_flags: self.affect_flags_of(vid),
            dead: self.is_dead(vid.raw()),
            alignment: i16::try_from(body.card.alignment / 10).unwrap_or(i16::MAX),
            view_guild: self.view_guild_of_vid(vid.raw()),
        })
    }

    /// `ChatPacket(type, format, args...)` to the player under `vid`.
    pub(super) fn tell(
        &self,
        vid: Vid,
        mover: Mover,
        chat_type: u8,
        format: &[u8],
        args: &[Arg<'_>],
    ) {
        let line = chat_packet(mover.recipient(&self.locale), chat_type, format, args);
        let _sent = self.write_to_client(vid, line);
    }

    /// `ChatPacket(CHAT_TYPE_COMMAND, "CloseRestartWindow")`.
    fn close_restart_window(&self, vid: Vid, mover: Mover) {
        self.tell(vid, mover, CHAT_TYPE_COMMAND, CLOSE_RESTART_WINDOW, &[]);
    }

    /// `GetDesc()->SetPhase(phase)`: the `GC_PHASE` record, which the descriptor turns into its
    /// phase change where it reads it.
    fn write_phase(&self, vid: Vid, phase: u8) {
        let mut record = Vec::new();
        GcHeaderAndByte::new(HEADER_GC_PHASE.value(), phase).encode_into(&mut record);
        let _sent = self.write_to_client(vid, record);
    }

    /// The victim's side of `CHARACTER::Damage` on the player `victim`
    /// (`G/char_battle.cpp:2248-2433`): a hit on a stunned player kills it, a hit on a dead one
    /// does nothing, and a hit that leaves no hit points stuns it, with the attacker's PID as its
    /// killer when the attacker is a player. Returns whether the player died, as legacy's `Damage`
    /// does.
    ///
    /// `shown` is the strike's flag: `SendDamagePacket` goes to the player first, then the hit
    /// points change, then the targeters see them before `GC_STUN` (V17i).
    pub(super) fn damage_pc(
        &mut self,
        attacker: Option<Attacker>,
        victim: Vid,
        damage: i32,
        shown: Option<u8>,
    ) -> bool {
        self.damage_pc_typed(attacker, victim, damage, shown, false)
    }

    /// The `DAMAGE_TYPE_POISON` path of [`GameState::damage_pc`] (`:2256-2261`): before
    /// anything else, a tick that would leave no hit points deals all but one. Poison never
    /// kills by itself, and never stuns. A stunned victim still dies by `Dead` first.
    pub(super) fn damage_pc_poison(&mut self, victim: Vid, damage: i32) -> bool {
        self.damage_pc_typed(None, victim, damage, Some(DAMAGE_POISON), true)
    }

    /// [`GameState::damage_pc`], with `poison` picking the poison clamp.
    fn damage_pc_typed(
        &mut self,
        attacker: Option<Attacker>,
        victim: Vid,
        damage: i32,
        shown: Option<u8>,
        poison: bool,
    ) -> bool {
        match self.damage_pc_head(attacker, victim, damage, shown, poison) {
            PcDamageHead::Gone => false,
            PcDamageHead::Died | PcDamageHead::Dead => true,
            PcDamageHead::Open { hp, damage } => {
                self.damage_pc_tail(attacker, victim, hp, damage);
                false
            }
        }
    }

    /// What [`GameState::damage_pc`] sends before the hit points change: the stunned
    /// victim dies by `Dead`, a dead one takes nothing, and a live one gets its damage
    /// packet (poison-clamped first). The reflected hit runs between this and
    /// [`GameState::damage_pc_tail`], as legacy's reflect arm (`:1979-1988`) runs
    /// inside the victim's `Damage` before its hit-point change (`:2387`).
    pub(super) fn damage_pc_head(
        &mut self,
        attacker: Option<Attacker>,
        victim: Vid,
        damage: i32,
        shown: Option<u8>,
        poison: bool,
    ) -> PcDamageHead {
        let raw = victim.raw();
        let Some(combat) = self.pc_combat.get(&raw).copied() else {
            return PcDamageHead::Gone;
        };
        if combat.stunned {
            self.dead_pc(victim, attacker);
            return PcDamageHead::Died;
        }
        if combat.dead {
            return PcDamageHead::Dead;
        }
        let Some(hp) = self.pc_points_mut(raw).map(|points| points.hp()) else {
            return PcDamageHead::Gone;
        };
        // The poison clamp runs before the damage packet: a clamped tick shows the damage
        // it dealt, not the damage it rolled.
        let damage = if poison && hp.saturating_sub(damage) <= 0 {
            hp.saturating_sub(1)
        } else {
            damage
        };
        if let Some(flag) = shown {
            self.send_pc_damage_info(victim, flag, damage);
        }
        PcDamageHead::Open { hp, damage }
    }

    /// What [`GameState::damage_pc`] does from the hit-point change on: the lethal clamp,
    /// the change, the targeters' broadcast, and the stun with the attacker's PID as the
    /// killer when the attacker is a player.
    pub(super) fn damage_pc_tail(
        &mut self,
        attacker: Option<Attacker>,
        victim: Vid,
        hp: i32,
        damage: i32,
    ) {
        let raw = victim.raw();
        // `cannot_dead` is a GM's flag, which no player has.
        let damage = if hp.saturating_sub(damage) <= 0 {
            hp
        } else {
            damage
        };
        self.change_pc_point(victim, point::POINT_HP, -damage, false);
        self.broadcast_pc_target(victim);
        if self
            .pc_points_mut(raw)
            .is_some_and(|points| points.hp() <= 0)
        {
            self.stun_pc(victim);
            let killer = attacker.filter(|attacker| attacker.pc);
            self.pc_combat.entry(raw).or_default().killer_pid =
                killer.map_or(0, |attacker| attacker.vid.raw());
        }
    }

    /// `Stun` of the player under `vid` (`G/char_battle.cpp:422-459`): its potion pools empty,
    /// its recovery ends, `GC_STUN` goes to its view, its pools lock, and `StunEvent` is due 3
    /// seconds later. It has no private shop to close.
    pub(super) fn stun_pc(&mut self, vid: Vid) {
        let raw = vid.raw();
        if self.is_down(raw) {
            return;
        }
        for kind in [point::POINT_HP_RECOVERY, point::POINT_SP_RECOVERY] {
            let held = self
                .pc_points_mut(raw)
                .map_or(0, |points| points.get_point(kind));
            self.change_pc_point(vid, kind, -held, false);
        }
        let combat = self.pc_combat.entry(raw).or_default();
        self.regens
            .cancel(&mut combat.recovery_due, Due::PcRecovery(raw));
        self.packet_around(
            EntityKey::Character(raw),
            &header_and_vid(protocol::gc_inventory::HEADER_GC_STUN.value(), raw),
            None,
        );
        self.pc_combat.entry(raw).or_default().stunned = true;
        if let Some(points) = self.pc_points_mut(raw) {
            points.set_down(true);
        }
        let when = self
            .last_pulse
            .saturating_add(pulses(super::fight::STUN_SECONDS));
        let combat = self.pc_combat.entry(raw).or_default();
        if combat.stun_due.is_none() {
            self.regens
                .schedule(&mut combat.stun_due, when, Due::PcStun(raw));
        }
    }

    /// `StunEvent` of a player (`G/char_battle.cpp:402-420`): the stunned player dies, with the
    /// killer it kept.
    pub(super) fn pc_stun_event(&mut self, vid: u32) {
        let Some(combat) = self.pc_combat.get_mut(&vid) else {
            return;
        };
        if combat.stun_due.take().is_none() {
            return;
        }
        self.dead_pc(Vid::new(vid), None);
    }

    /// The player arms of `Dead` (`G/char_battle.cpp:1238-1696`), for the player under `vid`
    /// killed by `killer`, or else by the player whose PID it kept.
    fn dead_pc(&mut self, vid: Vid, killer: Option<Attacker>) {
        let raw = vid.raw();
        let Some(combat) = self.pc_combat.get_mut(&raw) else {
            return;
        };
        if combat.dead {
            return;
        }
        // `m_dwKillerPID = 0`, which legacy says never to drop (`:1261`). `FindByPID` finds the
        // player only while it is online.
        let pid = std::mem::take(&mut combat.killer_pid);
        let killer = killer.or_else(|| {
            let online = pid != 0 && self.characters.find_by_vid(Vid::new(pid)).is_ok();
            online.then_some(Attacker {
                vid: Vid::new(pid),
                pc: true,
            })
        });
        // A player killer drops its target when it is this player (`:1275-1276`). The PvP,
        // battle-pass and quest-kill steps wait for PvP; the guild step scores the war.
        if let Some(killer) = killer.filter(|killer| killer.pc) {
            if self.targets.of(killer.vid.raw()) == Some(raw) {
                self.target(killer.vid, 0);
            }
            // `CGuildManager::Kill` (`:1308-1311`): the war score, when the two guilds
            // fight. The alignment skip (`isUnderGuildWar`, `:1413`) waits with the PvP
            // slice's killer-alignment arm — the port's alignment runs only on monster
            // kills (`raise_alignment`), where legacy never checks the war.
            self.guild_war_kill(killer.vid, vid);
        }
        // `SetPosition(POS_DEAD)`. `ClearAffect(true)` uncomputes every affect but the
        // no-save and no-clear ones (`G/char_affect.cpp:259-337`): of the affects a player
        // can hold, the revive-invisible stays and the poison, stun, slow and 263's slow
        // go, each with its point taken back, its remove packet, and one flag update.
        let combat = self.pc_combat.entry(raw).or_default();
        combat.dead = true;
        combat.last_dead_ms = Some(self.clock.now());
        // `POS_DEAD` is not `POS_FIGHTING`.
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.fighting = false;
        }
        // `INSTANT_FLAG_DEATH_PENALTY`: set for a non-player killer, as no map is forked; cleared
        // for a player killer and for no killer (`:1372-1387`).
        combat.death_penalty = killer.is_some_and(|killer| !killer.pc);
        self.clear_affects_on_death(vid);
        // `ClearSync` (`:1461`), then the stun event ends (`:1464`). The poison event ends
        // with the victim: legacy cancels nothing, but the Rewrite's tick resolves the
        // slot, and a dead victim must tick no more (V18b).
        self.clear_sync(vid, true);
        let combat = self.pc_combat.entry(raw).or_default();
        self.regens.cancel(&mut combat.stun_due, Due::PcStun(raw));
        self.cancel_pc_poison(vid);
        // `GetDesc()->SetPhase(PHASE_DEAD)` (`:1470`).
        self.write_phase(vid, PHASE_DEAD);
        self.packet_around(
            EntityKey::Character(raw),
            &header_and_vid(HEADER_GC_DEAD.value(), raw),
            None,
        );
        self.pc_combat.entry(raw).or_default().stunned = false;
        // The client is sent its affects again (`:1561-1569`), with the time each has left.
        for affect in self.running_affects(vid) {
            let _sent = self.write_to_client(vid, affect_add_frame(&affect));
        }
        let when = self.last_pulse.saturating_add(pulses(DEAD_SECONDS));
        let combat = self.pc_combat.entry(raw).or_default();
        self.regens.cancel(&mut combat.dead_due, Due::PcDead(raw));
        self.regens
            .schedule(&mut combat.dead_due, when, Due::PcDead(raw));
        // The closes (`:1614-1648`). The cube, sash, change-look, aura and premium-shop windows
        // and the private shop are not ported; the mall stays open, as legacy closes only the
        // safebox.
        self.cancel_trade(vid);
        self.end_browsing(vid);
        if let Some(mover) = self.own_mover(vid) {
            self.close_safebox_on_death(vid, mover);
        }
    }

    /// `dead_event` of a player (`G/char_battle.cpp:500-524`): back in game, standing, warped
    /// to its town with half its hit points. A town no Channel serves restarts it in place (B12).
    pub(super) fn pc_dead_event(&mut self, vid: u32) {
        let Some(combat) = self.pc_combat.get_mut(&vid) else {
            return;
        };
        if combat.dead_due.take().is_none() {
            return;
        }
        let player = Vid::new(vid);
        let Some(mover) = self.own_mover(player) else {
            return;
        };
        self.write_phase(player, PHASE_GAME);
        self.stand(player);
        let recall = self.recall_destination(player);
        if recall.is_none() {
            warn!(
                vid,
                "a dead player has no town a Channel serves; it restarts in place"
            );
            self.restart_at_same_pos(player);
        }
        let half = self
            .pc_points_mut(vid)
            .map_or(0, |points| points.max_hp() / 2);
        self.set_pc_hp(player, half, true);
        // `DeathPenalty(0)` in legacy; the automatic restart takes the town's, which is none (B15).
        self.death_penalty(player, true, mover);
        self.start_pc_recovery(player);
        self.close_restart_window(player, mover);
        match recall {
            Some(point) => self.order_warp(player, point),
            None => self.revive_invisible(player),
        }
    }

    /// `do_restart_now` and `do_restart` (`G/cmd_general.cpp:471-706`) for the player under
    /// `vid`. The answer carries the gold `/restart_now` took, which the descriptor stores.
    pub(super) fn restart(&mut self, vid: Vid, kind: RestartKind, mover: Mover) -> RestartAnswer {
        let raw = vid.raw();
        let unpaid = RestartAnswer { gold: None };
        if self.characters.find_by_vid(vid).is_err() {
            return unpaid;
        }
        // A living player, a stunned one too: the window closes and the recovery starts.
        if !self.is_dead(raw) {
            self.close_restart_window(vid, mover);
            self.start_pc_recovery(vid);
            return unpaid;
        }
        let Some(dead_due) = self.pc_combat.get(&raw).and_then(|combat| combat.dead_due) else {
            return unpaid;
        };
        // `event_time(m_pkDeadEvent) / passes_per_sec`, truncated.
        let time_to_dead = dead_due.saturating_sub(self.last_pulse) / u64::from(PASSES_PER_SEC);
        let wait = |limit: u64| i64::try_from(time_to_dead.saturating_sub(limit)).unwrap_or(0);
        match kind {
            RestartKind::Now => return self.restart_now(vid, mover),
            RestartKind::Here => {
                // `IsHack` alone answers a refusal: legacy adds a wait line that does not say
                // why (B24). The test-server arm, which waits for nothing, is not taken.
                if let Some(line) = self.is_hack(vid, mover) {
                    let _sent = self.write_to_client(vid, line);
                    return unpaid;
                }
                if time_to_dead > HERE_SECONDS {
                    let left = [Arg::Int(wait(HERE_SECONDS))];
                    self.tell(vid, mover, CHAT_TYPE_INFO, RESTART_WAIT_NOTICE, &left);
                    return unpaid;
                }
                self.begin_revive(vid, mover);
                self.restart_at_same_pos(vid);
                self.set_pc_hp(vid, RESTART_HP, false);
                self.death_penalty(vid, false, mover);
                self.revive_invisible(vid);
            }
            RestartKind::Town => {
                if let Some(line) = self.is_hack(vid, mover) {
                    let _sent = self.write_to_client(vid, line);
                    return unpaid;
                }
                if time_to_dead > TOWN_SECONDS {
                    let left = [Arg::Int(wait(TOWN_SECONDS))];
                    self.tell(vid, mover, CHAT_TYPE_INFO, RESTART_WAIT_NOTICE, &left);
                    return unpaid;
                }
                self.begin_revive(vid, mover);
                let recall = self.recall_destination(vid);
                if recall.is_none() {
                    warn!(
                        vid = raw,
                        "a town restart has no town a Channel serves; in place"
                    );
                    self.restart_at_same_pos(vid);
                }
                self.set_pc_hp(vid, RESTART_HP, false);
                self.death_penalty(vid, true, mover);
                match recall {
                    Some(point) => self.order_warp(vid, point),
                    None => self.revive_invisible(vid),
                }
            }
        }
        unpaid
    }

    /// `do_restart_now`'s dead arm (`G/cmd_general.cpp:480-524`): the price, `IsHack`, then the
    /// revive where the player lies at full hit points, paid with 300 000 gold.
    fn restart_now(&mut self, vid: Vid, mover: Mover) -> RestartAnswer {
        let unpaid = RestartAnswer { gold: None };
        let Some(gold) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .map(world::character::Character::gold)
        else {
            return unpaid;
        };
        if gold < RESTART_NOW_GOLD {
            self.tell(vid, mover, CHAT_TYPE_INFO, RESTART_NOW_PRICE_NOTICE, &[]);
            return unpaid;
        }
        // `IsHack` refuses with its own line; legacy's "You paid" line is not sent (B1).
        if let Some(line) = self.is_hack(vid, mover) {
            let _sent = self.write_to_client(vid, line);
            return unpaid;
        }
        self.begin_revive(vid, mover);
        self.restart_at_same_pos(vid);
        let full = self
            .pc_points_mut(vid.raw())
            .map_or(0, |points| points.max_hp());
        self.set_pc_hp(vid, full, false);
        self.death_penalty(vid, false, mover);
        self.revive_invisible(vid);
        // `ChangeGold(-300000)` (`G/char.cpp:3823-3850`): the record shows no amount for a loss.
        let left = gold - RESTART_NOW_GOLD;
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            character.set_gold(left);
        }
        let mut record = Vec::new();
        GcCharacterGoldChange::new(vid.raw(), 0, left).encode_into(&mut record);
        let _sent = self.write_to_client(vid, record);
        RestartAnswer {
            gold: Some(gold_delta(gold, left)),
        }
    }

    /// What every accepted restart does first (`G/cmd_general.cpp:579-582`, `:506-509`): the
    /// window closes, the session is back in game, the player stands, and its recovery starts.
    fn begin_revive(&mut self, vid: Vid, mover: Mover) {
        self.close_restart_window(vid, mover);
        self.write_phase(vid, PHASE_GAME);
        self.stand(vid);
        self.start_pc_recovery(vid);
    }

    /// `SetPosition(POS_STANDING)` of a player (`G/char.cpp:1436-1444`): not stunned and not
    /// dead, its dead and stun events end, and its pools unlock. The PID of a killer it kept is
    /// dropped too, so a stale one blames no one at a later death (D6).
    fn stand(&mut self, vid: Vid) {
        let raw = vid.raw();
        let combat = self.pc_combat.entry(raw).or_default();
        combat.stunned = false;
        combat.dead = false;
        combat.killer_pid = 0;
        // `POS_STANDING`.
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.fighting = false;
        }
        self.regens.cancel(&mut combat.dead_due, Due::PcDead(raw));
        self.regens.cancel(&mut combat.stun_due, Due::PcStun(raw));
        if let Some(points) = self.pc_points_mut(raw) {
            points.set_down(false);
        }
    }

    /// `RestartAtSamePos` (`G/char.cpp:1021-1056`): the body is shown again where it stands,
    /// which removes and re-inserts it for itself and its view.
    fn restart_at_same_pos(&mut self, vid: Vid) {
        let raw = vid.raw();
        let Some(spot) = self.spot_of(raw) else {
            return;
        };
        let _shown = self.show_body(raw, (spot.x, spot.y, spot.z), None);
    }

    /// `GetRecallPositionByEmpire`, else the empire start (`G/char_battle.cpp:508-516`,
    /// `G/cmd_general.cpp:687-690`), kept only when the atlas names its map and a Channel the
    /// player's client can reach serves it, as `WarpSet` requires.
    fn recall_destination(&self, vid: Vid) -> Option<(i32, i32)> {
        let body = self.bodies.get(&vid)?;
        let atlas = self.recall.atlas.as_deref()?;
        let empire = body.card.empire;
        let (x, y) =
            warp::recall_position(atlas, body.map, empire).or_else(|| warp::go_home(empire))?;
        let index = atlas.index_at(x, y)?;
        self.recall.serves(body.channel, index).then_some((x, y))
    }

    /// `WarpSet(x, y)` for the player under `vid`, which its descriptor runs.
    pub(super) fn order_warp(&self, vid: Vid, (x, y): (i32, i32)) {
        let raw = vid.raw();
        let (Some(clients), Some((channel, map))) = (self.clients.as_ref(), self.place_of(raw))
        else {
            warn!(vid = raw, "a restart warp has no client registry to reach");
            return;
        };
        let ordered = clients
            .members_on_map(channel, map)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| clients.order(channel, id, ClientOrder::Warp { x, y }));
        if !ordered {
            warn!(vid = raw, x, y, "a restart warp found no client to order");
        }
    }

    /// `DeathPenalty(bTown)` (`G/char_battle.cpp:330-392`), with the flag tested first (D2) and
    /// no roll in town (B11). The cube, sash and change-look windows are not ported, and no
    /// affect 511 (`AFFECT_NO_DEATH_PENALTY`) is ever added.
    fn death_penalty(&mut self, vid: Vid, town: bool, mover: Mover) {
        let raw = vid.raw();
        if self
            .place_of(raw)
            .is_some_and(|(_, map)| ARENA_MAPS.contains(&map))
        {
            return;
        }
        let flagged = self
            .pc_combat
            .get_mut(&raw)
            .is_some_and(|combat| std::mem::take(&mut combat.death_penalty));
        if !flagged || town {
            return;
        }
        let Some(level) = self.pc_points_mut(raw).map(|points| points.level()) else {
            return;
        };
        if level < PENALTY_LEVEL || number(&mut self.dice, 0, 2) != 0 {
            self.tell(vid, mover, CHAT_TYPE_INFO, NO_PENALTY_NOTICE, &[]);
            return;
        }
        let teardrop = self.wears_teardrop(vid);
        let loss = death_penalty_loss(level, levels::next_exp(level), teardrop);
        let max_level = self.exp_limits.max_level;
        let record = self
            .pc_points_mut(raw)
            .and_then(|points| points.lose_exp(loss, max_level));
        if let Some(record) = record {
            self.send_point_records(vid, &[record]);
        }
    }

    /// `IsEquipUniqueItem(UNIQUE_ITEM_TEARDROP_OF_GODNESS)` (`G/char_item.cpp:9519-9540`).
    fn wears_teardrop(&self, vid: Vid) -> bool {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let equipment = Equipment::of(character.items(), &self.protos);
        [EWearPositions::Unique1, EWearPositions::Unique2]
            .into_iter()
            .filter_map(|wear| equipment.wear(wear as u16))
            .any(|worn| worn.item.vnum == TEARDROP_OF_GODNESS)
    }

    /// `ReviveInvisible(5)`, `AddAffect(215, POINT_NONE, 0, 28, 5, 0, true)`
    /// (`G/char_affect.cpp:668-760`): a running revive is reset in place and its removal sent,
    /// otherwise one is added; then the update goes to the view, the player included, the event
    /// starts, and the client is sent the affect.
    pub(super) fn revive_invisible(&mut self, vid: Vid) {
        self.add_pc_affect(vid, Affect::revive_invisible(), true);
    }

    /// The affects running on the player under `vid`, in the order they were added.
    fn running_affects(&self, vid: Vid) -> Vec<Affect> {
        self.affect_events
            .get(&vid)
            .map(|event| event.affects.clone())
            .unwrap_or_default()
    }

    /// `StartRecoveryEvent` of a player (`G/char.cpp:3287-3314`): none while one runs or the
    /// player is down; otherwise due 3 seconds from now.
    pub(super) fn start_pc_recovery(&mut self, vid: Vid) {
        let raw = vid.raw();
        let when = self.last_pulse.saturating_add(pulses(RECOVERY_SECONDS));
        let combat = self.pc_combat.entry(raw).or_default();
        if combat.recovery_due.is_some() || combat.down() {
            return;
        }
        self.regens
            .schedule(&mut combat.recovery_due, when, Due::PcRecovery(raw));
    }

    /// A player's `recovery_event` (`G/char.cpp:3245-3284`): `CheckTarget`, then the heal,
    /// then again 3 seconds on. `UpdateKillerMode` is not ported (no PK decay).
    pub(super) fn pc_recovery_event(&mut self, vid: u32) {
        let Some(combat) = self.pc_combat.get_mut(&vid) else {
            return;
        };
        if combat.recovery_due.take().is_none() {
            return;
        }
        let player = Vid::new(vid);
        self.check_target(player);
        let now = self.clock.now();
        let Some(body) = self.bodies.get(&player) else {
            let when = self.last_pulse.saturating_add(pulses(RECOVERY_SECONDS));
            let combat = self.pc_combat.entry(vid).or_default();
            self.regens
                .schedule(&mut combat.recovery_due, when, Due::PcRecovery(vid));
            return;
        };
        // `char_battle.cpp:146-147`: within 3 seconds of the attack or the move.
        let attacking = body
            .last_attack
            .is_some_and(|at| now.wrapping_sub(at) < 3000);
        let moving = now.wrapping_sub(body.last_move) < 3000;
        let idle_ms = now.wrapping_sub(body.last_move);
        let skill_group = body.card.skill_group;
        // `IsAffectFlag(AFF_POISON)`: a poisoned player neither heals nor distributes
        // (`G/char.cpp:3254-3260`).
        let poisoned = self.affect_flags_of(player)[0] & AFF_POISON_WORD != 0;
        let Some(points) = self.pc_points_mut(vid) else {
            let when = self.last_pulse.saturating_add(pulses(RECOVERY_SECONDS));
            let combat = self.pc_combat.entry(vid).or_default();
            self.regens
                .schedule(&mut combat.recovery_due, when, Due::PcRecovery(vid));
            return;
        };
        let records = pc_heal(points, skill_group, attacking, moving, idle_ms, poisoned);
        self.send_point_records(player, &records);
        let when = self.last_pulse.saturating_add(pulses(RECOVERY_SECONDS));
        let combat = self.pc_combat.entry(vid).or_default();
        self.regens
            .schedule(&mut combat.recovery_due, when, Due::PcRecovery(vid));
    }

    /// `CheckTarget` (`G/char.cpp:6663-6670`): a target 4800 or more away is dropped.
    fn check_target(&mut self, vid: Vid) {
        let Some(target) = self.targets.of(vid.raw()) else {
            return;
        };
        let Some(spot) = self.spot_of(vid.raw()) else {
            return;
        };
        let at = match self.npc_by_vid(target) {
            Some(npc) => (npc.x, npc.y),
            None => match self.spot_of(target) {
                Some(other) => (other.x, other.y),
                None => return,
            },
        };
        let distance = distance_approx(spot.x.saturating_sub(at.0), spot.y.saturating_sub(at.1));
        if distance >= TARGET_REACH {
            self.target(vid, 0);
        }
    }

    /// `ClearSync` of the player under `vid` (`G/char.cpp:5568-5575`). With `own`, it first has
    /// no owner, which its view is told; the claim's time stays. Then each character it owns has
    /// none ([`GameState::release_owned`]).
    pub(super) fn clear_sync(&mut self, vid: Vid, own: bool) {
        if own {
            if let Some(body) = self.bodies.get_mut(&vid) {
                body.sync.owner = None;
            }
            let record = ownership_record(Vid::new(0), vid);
            self.packet_around(EntityKey::Character(vid.raw()), &record, None);
        }
        self.release_owned(vid);
    }

    /// The `bSave` half of `ClearAffect(true)` on the player under `vid`
    /// (`G/char_affect.cpp:259-337`): every affect but the no-clear and no-save ones is
    /// uncomputed, erased, and refused its next run, with its remove packet to the player.
    /// Of the affects a player can hold, the revive-invisible stays (no-save, `:43-55`) and
    /// the poison, stun, slow and 263's slow go. Then the flags update when they or either
    /// speed moved. `CheckMaximumPoints` is dead here: no affect here touches a maximum.
    fn clear_affects_on_death(&mut self, vid: Vid) {
        let flags_before = self.affect_flags_of(vid);
        let mov_before = self.move_speed_of(vid.raw());
        let Some(event) = self.affect_events.get_mut(&vid) else {
            return;
        };
        let mut cleared = Vec::new();
        event.affects.retain(|affect| {
            if affect.kind == crate::loading_phase::AFFECT_REVIVE_INVISIBLE {
                return true;
            }
            cleared.push(*affect);
            false
        });
        if cleared.is_empty() {
            return;
        }
        for affect in &cleared {
            self.change_pc_point(vid, usize::from(affect.apply_on), -affect.value, false);
            let frame = affect_remove_frame(affect);
            let _cleared = self.write_to_client(vid, frame);
        }
        let flags_after = self.affect_flags_of(vid);
        if flags_after != flags_before || mov_before != self.move_speed_of(vid.raw()) {
            self.send_affect_update(vid, flags_after);
        }
    }

    /// `RemovePoison` of the player under `vid` (`G/char_resist.cpp:233-237`): the poison
    /// affect goes and its event ends. The cure calls it (the heal 109); `Dead`
    /// routes through the death clearing.
    pub(super) fn remove_pc_poison(&mut self, vid: Vid) {
        self.cancel_pc_poison(vid);
    }

    /// Ends the poison event of the player under `vid`, if one runs.
    fn cancel_pc_poison(&mut self, vid: Vid) {
        let raw = vid.raw();
        let Some(combat) = self.pc_combat.get_mut(&raw) else {
            return;
        };
        let Some((when, count)) = combat.poison_due.take() else {
            return;
        };
        self.regens
            .due
            .remove(&(when, Due::PcPoison { vid: raw, count }));
    }

    /// `poison_event` of the player under `vid` (`G/char_resist.cpp:42-76`): the victim's
    /// own slot must still hold this tick, else a re-poison cancelled it. The tick deals
    /// `maxHP * rate / 1000` through the poison path; on death the event ends, else the
    /// count falls and while any is left the next tick is due 3 seconds on. The attacker
    /// is `None`: the poison's PID is 0 for a monster, which `FindByPID` never maps
    /// (`G/char_manager.cpp:109-119`).
    pub(super) fn pc_poison_event(&mut self, vid: u32, count: u32) {
        let held = self
            .pc_combat
            .get(&vid)
            .and_then(|combat| combat.poison_due);
        let Some((_, held_count)) = held else {
            return;
        };
        if held_count != count {
            return;
        }
        let points = self
            .characters
            .find_by_vid(Vid::new(vid))
            .ok()
            .and_then(|character| character.points().cloned());
        let Some(points) = points else {
            self.cancel_pc_poison(Vid::new(vid));
            return;
        };
        let damage = points
            .max_hp()
            .saturating_mul(pc_poison_rate(&points))
            .wrapping_div(1000);
        if self.damage_pc_poison(Vid::new(vid), damage) {
            self.cancel_pc_poison(Vid::new(vid));
            return;
        }
        if count <= 1 {
            self.cancel_pc_poison(Vid::new(vid));
            return;
        }
        let next = count.saturating_sub(1);
        let when = self.last_pulse.saturating_add(pulses(POISON_TICK_SECONDS));
        let old = self
            .pc_combat
            .get_mut(&vid)
            .and_then(|combat| combat.poison_due.replace((when, next)));
        if let Some((at, _)) = old {
            self.regens.due.remove(&(at, Due::PcPoison { vid, count }));
        }
        self.regens
            .due
            .insert((when, Due::PcPoison { vid, count: next }));
    }
    /// `SendDamagePacket(ch, 0, DAMAGE_BLOCK)` of the player `victim` claimed by `actor`
    /// (`G/char_battle.cpp:1720-1742`): a player victim and a player claimer are both told.
    pub(super) fn block_pc_claim(&self, victim: Vid, actor: Vid) {
        let mut record = Vec::new();
        GcDamageInfo {
            header: GcDamageInfo::header(),
            vid: victim.raw(),
            flag: DAMAGE_BLOCK,
            damage: 0,
        }
        .encode_into(&mut record);
        let _victim = self.write_to_client(victim, record.clone());
        let _actor = self.write_to_client(actor, record);
    }

    /// What a leaving player's fight leaves (`Disconnect`'s dead arm, `G/char.cpp:1779-1783`, and
    /// `Destroy`'s cancels, `:713-770`). A player that leaves while down stands with 50 hit points
    /// and pays the here penalty first, so it comes back alive where it fell (B5). Then its own
    /// claim is released when it disconnects (a warp leaves its sectree first, so its own release
    /// reaches nobody), what it owns is released, and its events end.
    pub(super) fn leave_fight(&mut self, vid: Vid, departure: Departure) {
        self.lose_pc_victim(vid);
        let raw = vid.raw();
        if self.is_down(raw) {
            if let Some(mover) = self.own_mover(vid) {
                self.death_penalty(vid, false, mover);
            }
            self.stand(vid);
            self.set_pc_hp(vid, RESTART_HP, false);
            debug!(
                vid = raw,
                "a player left while down and stands with 50 hit points"
            );
        }
        self.clear_sync(vid, departure == Departure::Disconnect);
        let Some(mut combat) = self.pc_combat.remove(&raw) else {
            return;
        };
        self.regens.cancel(&mut combat.stun_due, Due::PcStun(raw));
        self.regens.cancel(&mut combat.dead_due, Due::PcDead(raw));
        self.regens
            .cancel(&mut combat.recovery_due, Due::PcRecovery(raw));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
    use gamedata::map_atlas::MapRegion;
    use gamedata::mob_proto::CHAR_TYPE_MONSTER;
    use gamedata::server_attr::SectreeGrid;
    use protocol::cg_move::CgMove;
    use protocol::gc_actors::{GcCharacterAdd, GcCharacterUpdate};
    use protocol::gc_inventory::HEADER_GC_STUN;
    use protocol::gc_nested::GcAffectAdd;
    use protocol::item_pos::ItemPos;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::character::{Character, MoveRefused, MoveRequest, Pcg32, Trade};
    use world::item::{Item, ITEM_FLAG_STACKABLE};
    use world::npc::{MapNpcs, Npc};

    use super::super::fixtures::{a_world, points_at, region, TestClock, PLACE};
    use super::super::safebox::{SafeboxAnswer, SafeboxStep};
    use super::super::target::target_record;
    use super::super::trade::{TradeAnswer, TradeDeclined, TradeStep};
    use super::super::view_encode::remove_record;
    use super::*;
    use crate::client_registry::{ChannelClients, ClientEntry, Lease};
    use crate::game_loop::PulseProcessor;
    use crate::game_loop_messages::{EnterPlace, GameCommand, GroundPlace, Kept};
    use crate::item_move::MoveItemRefused;
    use crate::loading_phase::{PcCard, ADD_CHARACTER_STATE_DEAD, PK_MODE_PROTECT};
    use crate::movement::{MoveOutcome, MoveRefusal, FUNC_MOVE};
    use crate::sync_position::SyncOwnershipState;

    /// The player who dies.
    const ALPHA: Vid = Vid::new(1);
    /// A player beside it, who sees it.
    const BRAVO: Vid = Vid::new(2);
    /// A third player.
    const CHARLIE: Vid = Vid::new(3);
    /// Where Alpha stands.
    const HERE: (i32, i32) = (3200, 3200);
    /// 100 east of it, where Bravo stands.
    const BESIDE: (i32, i32) = (3300, 3200);
    /// The town spawn of [`PLACE`]'s map.
    const TOWN: (i32, i32) = (1000, 2000);
    /// The Pulse a field starts on; every player joins after it.
    const P0: u64 = 1;
    /// The experience every player holds.
    const EXP: i64 = 5_000;
    /// What a level 10 player loses: 4 % of the 33 000 its next level costs.
    const LOSS: i64 = 1_320;
    /// A monster's hit.
    const MONSTER: Attacker = Attacker {
        vid: Vid::new(0x8000_0001),
        pc: false,
    };
    /// Where a town restart sends a player of empire 1 on [`PLACE`].
    const TO_TOWN: ClientOrder = ClientOrder::Warp {
        x: TOWN.0,
        y: TOWN.1,
    };

    /// A world with its client registry, and each joined player's lease and inbox.
    struct Field {
        world: GameState,
        clients: Arc<ChannelClients>,
        players: BTreeMap<u32, (Lease, UnboundedReceiver<Vec<u8>>)>,
        clock: TestClock,
    }

    /// A world hosting [`PLACE`] whose town is [`TOWN`], served by Channel 1 when `served`,
    /// after Pulse [`P0`].
    fn a_field(served: bool) -> Field {
        let town = MapRegion {
            spawn: TOWN,
            ..region()
        };
        a_field_recalling(vec![town], served)
    }

    /// As [`a_field`], with an atlas of `regions`.
    fn a_field_recalling(regions: Vec<MapRegion>, served: bool) -> Field {
        let mut maps = HashMap::new();
        if served {
            let _served = maps.insert(PLACE.0, vec![u32::try_from(PLACE.1).unwrap()]);
        }
        a_field_serving(regions, &maps, &[])
    }

    /// As [`a_field`], with an atlas of `regions`, the maps each Channel of `maps` serves, and
    /// the `shared` maps the Shared Channel serves.
    fn a_field_serving(
        regions: Vec<MapRegion>,
        maps: &HashMap<u8, Vec<u32>>,
        shared: &[u32],
    ) -> Field {
        let clock = TestClock::default();
        let clients = Arc::new(ChannelClients::new());
        let atlas = Arc::new(MapAtlas::from_regions(regions));
        let mut world = a_world(&clock)
            .with_clients(Arc::clone(&clients))
            .with_recall(atlas, maps, shared);
        world.process_pulse(P0);
        Field {
            world,
            clients,
            players: BTreeMap::new(),
            clock: clock.clone(),
        }
    }

    impl Field {
        /// The player `vid` of `level` holding `items` joins Channel 1 at `at` on `map`, with
        /// [`EXP`], its recovery started as its entry starts it. The answer is what its client
        /// was shown.
        fn join_with(
            &mut self,
            vid: Vid,
            (map, at): (i32, (i32, i32)),
            level: u8,
            items: &[(ItemPos, Item)],
        ) -> Vec<Vec<u8>> {
            let name = format!("P{}", vid.raw());
            let mut lease = self.clients.join(ClientEntry {
                channel: PLACE.0,
                map,
                name: name.clone(),
                vid: vid.raw(),
                empire: 1,
                language: 0,
            });
            let inbox = lease.take_receiver().expect("the inbox is home");
            // The computation's own hit-point changes are no one's news.
            let mut points = points_at(level, 12_345, EXP);
            let _computed = points.take_target_broadcasts();
            self.world
                .enter_world_with_items(vid, vid.raw(), &name, items, lease.outbox())
                .expect("the player enters");
            self.world
                .characters
                .find_by_vid_mut(vid)
                .expect("the player is online")
                .set_points(Some(points));
            let card = PcCard {
                name,
                job: 0,
                empire: 1,
                level,
                conqueror_level: 0,
                language: 0,
                pk_mode: PK_MODE_PROTECT,
                skill_group: 0,
                alignment: 0,
            };
            let place = EnterPlace {
                channel: PLACE.0,
                map,
                x: at.0,
                y: at.1,
                z: 0,
            };
            let shown = self.world.place_body(vid, place, card);
            self.world.start_pc_recovery(vid);
            let _left = self.players.insert(vid.raw(), (lease, inbox));
            shown
        }

        /// A level 10 player `vid` holding nothing joins at `at` on [`PLACE`].
        fn join(&mut self, vid: Vid, at: (i32, i32)) -> Vec<Vec<u8>> {
            self.join_with(vid, (PLACE.1, at), 10, &[])
        }

        /// `vid` leaves the world by `departure`, and its client is gone.
        fn leave(&mut self, vid: Vid, departure: Departure) -> Kept {
            let kept = self
                .world
                .leave_world_by(vid, departure)
                .expect("the player was online");
            let _gone = self.players.remove(&vid.raw());
            kept
        }

        /// What `vid`'s client was sent since the last look.
        fn heard(&mut self, vid: Vid) -> Vec<Vec<u8>> {
            let (_, inbox) = self.players.get_mut(&vid.raw()).expect("the player joined");
            std::iter::from_fn(|| inbox.try_recv().ok()).collect()
        }

        /// Forgets what every client was sent so far.
        fn quiet(&mut self) {
            for (_, inbox) in self.players.values_mut() {
                while inbox.try_recv().is_ok() {}
            }
        }

        /// The orders `vid`'s descriptor was handed since the last look.
        fn ordered(&mut self, vid: Vid) -> Vec<ClientOrder> {
            let (lease, _) = self.players.get_mut(&vid.raw()).expect("the player joined");
            let mut orders = lease.take_orders().expect("the queue is home");
            let got = std::iter::from_fn(|| orders.try_recv().ok()).collect();
            lease.put_orders(orders);
            got
        }

        fn character(&self, vid: Vid) -> &Character {
            self.world
                .characters
                .find_by_vid(vid)
                .expect("the player is online")
        }

        fn points(&self, vid: Vid) -> Points {
            self.character(vid)
                .points()
                .cloned()
                .expect("the player has points")
        }

        fn combat(&self, vid: Vid) -> PcCombat {
            self.world
                .pc_combat
                .get(&vid.raw())
                .copied()
                .unwrap_or_default()
        }

        /// Sets `vid`'s pools, forgetting the computation's records.
        fn set_hp_sp(&mut self, vid: Vid, hp: i32, sp: i32) {
            let points = self
                .world
                .characters
                .find_by_vid_mut(vid)
                .expect("the player is online")
                .items_and_points_mut()
                .1
                .expect("the player has points");
            let hp_delta = hp - points.hp();
            let _hp = points.point_change(point::POINT_HP, hp_delta, false, false);
            let sp_delta = sp - points.sp();
            let _sp = points.point_change(point::POINT_SP, sp_delta, false, false);
            let _own = points.take_target_broadcasts();
        }

        fn set_gold(&mut self, vid: Vid, gold: u64) {
            self.world
                .characters
                .find_by_vid_mut(vid)
                .expect("the player is online")
                .set_gold(gold);
        }

        fn pulse(&mut self, pulse: u64) {
            self.world.process_pulse(pulse);
        }

        /// Runs every Pulse up to `to`, so each affect tick runs on its own Pulse.
        fn walk(&mut self, to: u64) {
            for pulse in self.world.last_pulse + 1..=to {
                self.pulse(pulse);
            }
        }

        /// A monster's hit takes all of `vid`'s hit points: it is stunned.
        fn struck_down(&mut self, vid: Vid) {
            let hp = self.points(vid).hp();
            assert!(
                !self.world.damage_pc(Some(MONSTER), vid, hp, None),
                "a stun is no death"
            );
            assert!(self.world.is_stunned(vid.raw()));
        }

        /// A monster stuns `vid`, what the stun sent is forgotten, and its next hit kills `vid`
        /// at once with the death penalty's flag set.
        fn killed(&mut self, vid: Vid) {
            self.struck_down(vid);
            self.quiet();
            assert!(self.world.damage_pc(Some(MONSTER), vid, 1, None));
            assert!(self.world.is_dead(vid.raw()));
        }

        /// A monster stuns `vid` now, and the stun event kills it 75 Pulses later with no
        /// killer. Answers the Pulse it died on.
        fn stunned_to_death(&mut self, vid: Vid) -> u64 {
            self.struck_down(vid);
            let died = self.world.last_pulse + 75;
            self.walk(died);
            assert!(self.world.is_dead(vid.raw()));
            died
        }

        /// Alpha's restart of `kind`.
        fn restart(&mut self, kind: RestartKind) -> RestartAnswer {
            self.world.restart(ALPHA, kind, mover())
        }

        /// Alpha, killed by a monster when `flagged` and else by the stun event, restarts here
        /// 226 Pulses on with `dice`. Answers what Alpha heard of the restart.
        fn dies_and_restarts_here(&mut self, flagged: bool, dice: Pcg32) -> Vec<Vec<u8>> {
            let died = if flagged {
                self.killed(ALPHA);
                self.world.last_pulse
            } else {
                self.stunned_to_death(ALPHA)
            };
            assert_eq!(self.combat(ALPHA).death_penalty, flagged);
            self.world.dice = dice;
            self.quiet();
            self.pulse(died + 226);
            assert_eq!(self.restart(RestartKind::Here), RestartAnswer::default());
            self.heard(ALPHA)
        }
    }

    /// What a step no command carries says to a player of a field.
    const fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 0,
            pk_mode: PK_MODE_PROTECT,
            affect_flags: [0; 2],
            dead: false,
            alignment: 0,
            view_guild: (0, 0),
        }
    }

    /// A seed whose first `number(0, 2)` is 0 when `zero`, and not 0 otherwise.
    fn a_seed(zero: bool) -> u64 {
        (0..10_000)
            .find(|&seed| (number(&mut Pcg32::new(seed, 0), 0, 2) == 0) == zero)
            .expect("a seed")
    }

    fn line(world: &GameState, chat_type: u8, text: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
        chat_packet(mover().recipient(&world.locale), chat_type, text, args)
    }

    fn close(world: &GameState) -> Vec<u8> {
        line(world, CHAT_TYPE_COMMAND, CLOSE_RESTART_WINDOW, &[])
    }

    fn wait(world: &GameState, seconds: i64) -> Vec<u8> {
        line(
            world,
            CHAT_TYPE_INFO,
            RESTART_WAIT_NOTICE,
            &[Arg::Int(seconds)],
        )
    }

    fn phase(phase: u8) -> Vec<u8> {
        let mut record = Vec::new();
        GcHeaderAndByte::new(HEADER_GC_PHASE.value(), phase).encode_into(&mut record);
        record
    }

    /// Alpha's `GC_POINT_CHANGE` of `kind`.
    fn change(kind: usize, amount: i64, value: i64) -> Vec<u8> {
        let record = PointRecord {
            kind: u8::try_from(kind).unwrap(),
            amount,
            value,
            broadcast: false,
        };
        point_changes(&[record], ALPHA.raw()).concat()
    }

    fn hp(value: i32) -> Vec<u8> {
        change(point::POINT_HP, 0, i64::from(value))
    }

    /// A `PointChange(kind, amount, false)` record: the heal hides the amount (`:3270`,`:3282`).
    fn healed(kind: usize, after: i32) -> Vec<u8> {
        change(kind, 0, i64::from(after))
    }

    fn hp_change(after: i32) -> Vec<u8> {
        healed(point::POINT_HP, after)
    }

    fn sp_change(after: i32) -> Vec<u8> {
        healed(point::POINT_SP, after)
    }

    fn stun(vid: Vid) -> Vec<u8> {
        header_and_vid(HEADER_GC_STUN.value(), vid.raw())
    }

    fn dead(vid: Vid) -> Vec<u8> {
        header_and_vid(HEADER_GC_DEAD.value(), vid.raw())
    }

    fn unowned(vid: Vid) -> Vec<u8> {
        ownership_record(Vid::new(0), vid)
    }

    fn revive() -> Vec<u8> {
        affect_add_frame(&Affect::revive_invisible())
    }

    /// Alpha's `GC_CHARACTER_UPDATE` with the affect flags it has now.
    fn update(world: &GameState) -> Vec<u8> {
        world
            .update_frame(ALPHA, world.affect_flags_of(ALPHA))
            .expect("Alpha is online")
    }

    fn gold_left(left: u64) -> Vec<u8> {
        let mut record = Vec::new();
        GcCharacterGoldChange::new(ALPHA.raw(), 0, left).encode_into(&mut record);
        record
    }

    /// What the restart arms send Alpha first: the window closes and the game phase is back,
    /// then the body's removal.
    fn revived(world: &GameState) -> [Vec<u8>; 3] {
        [close(world), phase(PHASE_GAME), remove_record(ALPHA.raw())]
    }

    /// Asserts `records` open with `first` and end with `last`, Alpha shown standing between.
    fn reinserted(records: &[Vec<u8>], first: &[Vec<u8>], last: &[Vec<u8>]) {
        assert!(records.len() > first.len() + last.len(), "{records:?}");
        assert_eq!(&records[..first.len()], first);
        let end = records.len() - last.len();
        assert_eq!(&records[end..], last);
        let shown = records[first.len()..end]
            .iter()
            .find_map(|record| GcCharacterAdd::decode(record).ok())
            .expect("Alpha is shown again");
        assert_eq!(
            (shown.dw_vid, shown.x, shown.y, shown.b_state_flag),
            (ALPHA.raw(), HERE.0, HERE.1, 0)
        );
    }

    /// The state flag of Alpha's `GC_CHARACTER_ADD` among `shown`.
    fn alpha_state(shown: &[Vec<u8>]) -> Option<u8> {
        shown.iter().find_map(|record| {
            GcCharacterAdd::decode(record)
                .ok()
                .filter(|add| add.dw_vid == ALPHA.raw())
                .map(|add| add.b_state_flag)
        })
    }

    /// Alpha's spot on Channel 1's map 41 at `HERE`.
    const fn here() -> GroundPlace {
        GroundPlace {
            channel: PLACE.0,
            map: PLACE.1,
            x: HERE.0,
            y: HERE.1,
        }
    }

    /// Table A: the hit leaves no hit points, empties the potion pools, stuns the player in its
    /// view, locks its pools, ends its recovery and makes `StunEvent` due 75 Pulses on; the
    /// players targeting it are told at the next broadcast.
    #[test]
    fn a_lethal_hit_on_a_player_stuns_it_and_zeroes_its_recovery_pools() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        let pools = [point::POINT_HP_RECOVERY, point::POINT_SP_RECOVERY];
        // Each pool holds something for the stun to empty.
        for kind in pools {
            field.world.change_pc_point(ALPHA, kind, 6, false);
        }
        field.world.target(BRAVO, ALPHA.raw());
        field.quiet();
        let max = field.points(ALPHA).max_hp();

        field.struck_down(ALPHA);
        let mut expected = vec![hp(0)];
        expected.extend(pools.map(|kind| change(kind, 0, 0)));
        expected.push(stun(ALPHA));
        assert_eq!(field.heard(ALPHA), expected);
        assert_eq!(
            pools.map(|kind| field.points(ALPHA).get_point(kind)),
            [0, 0]
        );
        assert_eq!(
            field.heard(BRAVO),
            vec![target_record(ALPHA.raw(), (0, 0, max), 0), stun(ALPHA)]
        );
        field.world.broadcast_targets();
        assert_eq!(field.heard(BRAVO), Vec::<Vec<u8>>::new());
        let combat = field.combat(ALPHA);
        assert!(combat.stunned && !combat.dead);
        assert_eq!(
            (combat.stun_due, combat.recovery_due),
            (Some(P0 + 75), None)
        );
        assert!(!field.world.regens.holds(Due::PcRecovery(ALPHA.raw())));
        assert!(field.world.regens.holds(Due::PcStun(ALPHA.raw())));
        // The pools are locked: a heal changes nothing and sends nothing.
        field
            .world
            .change_pc_point(ALPHA, point::POINT_HP, 100, false);
        assert_eq!(field.points(ALPHA).hp(), 0);
        assert!(field.heard(ALPHA).is_empty());
        // A second stun changes nothing either.
        field.world.stun_pc(ALPHA);
        assert!(field.heard(BRAVO).is_empty());
        assert_eq!(field.combat(ALPHA).stun_due, Some(P0 + 75));
    }

    /// `Damage` stops a hit at the hit points (`G/char_battle.cpp:2391-2392`): a larger one
    /// leaves 0, which the record shows, and never less.
    #[test]
    fn a_hit_larger_than_a_players_hit_points_leaves_it_with_zero() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.quiet();
        let full = field.points(ALPHA).hp();

        assert!(!field
            .world
            .damage_pc(Some(MONSTER), ALPHA, full + 1_000, None));
        assert_eq!(field.points(ALPHA).hp(), 0);
        assert_eq!(field.heard(ALPHA).first(), Some(&hp(0)));
        assert!(field.world.is_stunned(ALPHA.raw()));
    }

    /// Table B, rows 2, 4 and 5: no killer, so no flag, and `dead_event` is due 4500 Pulses on.
    /// The phase record carries legacy's byte for `PHASE_DEAD`, 6 (`EPhase`,
    /// `G/packet.h:790-798`), which the client reads as the dead phase.
    #[test]
    fn a_player_stunned_by_a_monster_dies_75_pulses_later_with_no_killer() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        field.struck_down(ALPHA);
        field.quiet();

        field.pulse(P0 + 74);
        assert!(field.heard(ALPHA).is_empty());
        assert!(field.heard(BRAVO).is_empty());
        assert!(field.world.is_stunned(ALPHA.raw()));
        field.pulse(P0 + 75);
        let heard = field.heard(ALPHA);
        assert_eq!(heard, vec![unowned(ALPHA), phase(PHASE_DEAD), dead(ALPHA)]);
        assert_eq!(heard[1], [HEADER_GC_PHASE.value(), 6]);
        assert_eq!(field.heard(BRAVO), vec![unowned(ALPHA), dead(ALPHA)]);
        let combat = field.combat(ALPHA);
        assert!(combat.dead && !combat.stunned && !combat.death_penalty);
        assert_eq!(combat.stun_due, None);
        assert_eq!(combat.dead_due, Some(P0 + 75 + 4_500));
        assert!(field.world.regens.holds(Due::PcDead(ALPHA.raw())));
        assert!(!field.world.regens.holds(Due::PcStun(ALPHA.raw())));
    }

    /// `Damage` on a stunned player runs `Dead` with the hitter, a monster, which sets the flag;
    /// the stun event is cancelled, and a hit on the dead player does nothing.
    #[test]
    fn a_second_hit_on_a_stunned_player_kills_it_at_once_and_blames_the_monster() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        field.struck_down(ALPHA);
        field.pulse(10);
        field.quiet();

        assert!(field.world.damage_pc(Some(MONSTER), ALPHA, 1, None));
        assert_eq!(
            field.heard(ALPHA),
            vec![unowned(ALPHA), phase(PHASE_DEAD), dead(ALPHA)]
        );
        assert_eq!(field.heard(BRAVO), vec![unowned(ALPHA), dead(ALPHA)]);
        let combat = field.combat(ALPHA);
        assert!(combat.dead && combat.death_penalty);
        assert_eq!((combat.stun_due, combat.dead_due), (None, Some(10 + 4_500)));
        field.pulse(P0 + 75);
        assert!(
            field.heard(ALPHA).is_empty(),
            "the stun event was cancelled"
        );
        assert!(field.world.damage_pc(Some(MONSTER), ALPHA, 1, None));
        assert!(field.heard(ALPHA).is_empty());
        assert!(field.heard(BRAVO).is_empty());
        assert_eq!(field.combat(ALPHA).dead_due, Some(10 + 4_500));
    }

    /// The stun keeps a player hitter's PID, which `Dead` reads and resets: that killer drops
    /// its target when it is the dead player (Table B row 1), and the flag stays clear.
    #[test]
    fn a_player_killed_by_a_player_carries_no_penalty_and_its_killer_drops_its_target() {
        for targets in [true, false] {
            let mut field = a_field(true);
            let _alpha = field.join(ALPHA, HERE);
            let _bravo = field.join(BRAVO, BESIDE);
            if targets {
                field.world.target(BRAVO, ALPHA.raw());
            }
            let hp = field.points(ALPHA).hp();
            let bravo = Attacker {
                vid: BRAVO,
                pc: true,
            };
            assert!(!field.world.damage_pc(Some(bravo), ALPHA, hp, None));
            assert_eq!(field.combat(ALPHA).killer_pid, BRAVO.raw());
            field.pulse(P0 + 74);
            field.quiet();

            field.pulse(P0 + 75);
            let mut expected = vec![unowned(ALPHA), dead(ALPHA)];
            if targets {
                expected.insert(0, target_record(0, (0, 0, 0), 0));
            }
            assert_eq!(field.heard(BRAVO), expected, "targets: {targets}");
            assert_eq!(field.world.targets.of(BRAVO.raw()), None);
            let combat = field.combat(ALPHA);
            assert!(combat.dead && !combat.death_penalty);
            assert_eq!(combat.killer_pid, 0, "the PID is reset");
        }
    }

    /// `Dead` looks up the kept PID only when no hitter is given (`G/char_battle.cpp:1257-1258`):
    /// a monster's hit on a player that a player stunned blames the monster, so the flag is set
    /// and the player that stunned it keeps its target. The PID is reset all the same.
    #[test]
    fn the_hitter_of_a_stunned_player_is_its_killer_over_the_player_that_stunned_it() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        field.world.target(BRAVO, ALPHA.raw());
        let full = field.points(ALPHA).hp();
        let bravo = Attacker {
            vid: BRAVO,
            pc: true,
        };
        assert!(!field.world.damage_pc(Some(bravo), ALPHA, full, None));
        assert_eq!(field.combat(ALPHA).killer_pid, BRAVO.raw());
        field.quiet();

        assert!(field.world.damage_pc(Some(MONSTER), ALPHA, 1, None));
        assert_eq!(field.heard(BRAVO), vec![unowned(ALPHA), dead(ALPHA)]);
        assert_eq!(field.world.targets.of(BRAVO.raw()), Some(ALPHA.raw()));
        let combat = field.combat(ALPHA);
        assert!(combat.dead && combat.death_penalty);
        assert_eq!(combat.killer_pid, 0);
    }

    /// Table B in order, inside the entry revive: the player's own claim and what it owns are
    /// released, then its phase, its death, and its revive with the 2 ticks it has left. The
    /// claim keeps its time.
    #[test]
    fn the_death_sends_ownership_then_phase_then_dead_then_the_running_revive() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        let _update = field.world.enter_revive_invisible(ALPHA);
        let claimed = Some(Duration::from_millis(500));
        for (victim, owner) in [(ALPHA, BRAVO), (BRAVO, ALPHA)] {
            field.world.bodies.get_mut(&victim).unwrap().sync = SyncOwnershipState {
                owner: Some(owner),
                claimed_at: claimed,
            };
        }
        field.struck_down(ALPHA);
        field.walk(P0 + 74);
        field.quiet();

        field.pulse(P0 + 75);
        let running = Affect {
            remaining: 2,
            ..Affect::revive_invisible()
        };
        let heard = field.heard(ALPHA);
        assert_eq!(
            heard,
            vec![
                unowned(ALPHA),
                unowned(BRAVO),
                phase(PHASE_DEAD),
                dead(ALPHA),
                affect_add_frame(&running)
            ]
        );
        // Read back from the bytes, which pins the record's own field: 2 of the 5 seconds left.
        let resent = GcAffectAdd::decode(&heard[4]).expect("the revive is sent again");
        assert_eq!(resent.element.duration, 2);
        assert_eq!(
            field.heard(BRAVO),
            vec![unowned(ALPHA), unowned(BRAVO), dead(ALPHA)]
        );
        let unclaimed = SyncOwnershipState {
            owner: None,
            claimed_at: claimed,
        };
        assert_eq!(field.world.bodies[&ALPHA].sync, unclaimed);
        assert_eq!(field.world.bodies[&BRAVO].sync.owner, None);
    }

    /// Table B rows 7 and 9: the trade ends for both sides, and the safebox closes with its
    /// command while the mall stays open.
    #[test]
    fn a_dying_player_ends_its_trade_and_closes_its_safebox_but_not_its_mall() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        let start = TradeStep::Start {
            target: BRAVO.raw(),
        };
        let started = field.world.trade(ALPHA, start, here(), mover());
        assert!(matches!(started, Ok(TradeAnswer::Sent(_))), "{started:?}");
        field.killed(ALPHA);
        let end = Trade::end_record().encode();
        let died = [unowned(ALPHA), phase(PHASE_DEAD), dead(ALPHA)];
        assert_eq!(
            field.heard(ALPHA),
            [died.as_slice(), &[end.clone()]].concat()
        );
        assert_eq!(field.heard(BRAVO), vec![unowned(ALPHA), dead(ALPHA), end]);
        assert!(field.world.trading.is_empty());

        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let opens = [
            (
                SafeboxStep::BeginOpen,
                SafeboxStep::Open {
                    account: 3,
                    items: Vec::new(),
                },
            ),
            (
                SafeboxStep::BeginMall,
                SafeboxStep::OpenMall {
                    account: 3,
                    items: Vec::new(),
                },
            ),
        ];
        for (begin, open) in opens {
            let begun = field.world.safebox(ALPHA, begin, mover());
            assert!(matches!(begun, Ok(SafeboxAnswer::Proceed)), "{begun:?}");
            let opened = field.world.safebox(ALPHA, open, mover());
            assert!(matches!(opened, Ok(SafeboxAnswer::Sent(_))), "{opened:?}");
        }
        field.killed(ALPHA);
        let closed = line(&field.world, CHAT_TYPE_COMMAND, b"CloseSafebox", &[]);
        assert_eq!(field.heard(ALPHA), [died.as_slice(), &[closed]].concat());
        assert!(!field.world.safebox_open(ALPHA));
        let mall = field.world.safebox(ALPHA, SafeboxStep::CloseMall, mover());
        let closed = line(&field.world, CHAT_TYPE_COMMAND, b"CloseMall", &[]);
        assert!(
            matches!(&mall, Ok(SafeboxAnswer::Sent(records)) if *records == vec![closed]),
            "the mall stayed open: {mall:?}"
        );
    }

    /// The insert and the update carry the dead state flag while the player is dead, and not
    /// after its restart.
    #[test]
    fn a_newcomer_sees_a_dead_player_lying_dead_and_a_restarted_one_standing() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.set_gold(ALPHA, 300_000);
        field.killed(ALPHA);
        let state_of_update = |world: &GameState| {
            let frame = world.update_frame(ALPHA, [0; 2]).expect("Alpha is online");
            GcCharacterUpdate::decode(&frame)
                .expect("an update")
                .b_state_flag
        };

        let shown = field.join(BRAVO, BESIDE);
        assert_eq!(alpha_state(&shown), Some(ADD_CHARACTER_STATE_DEAD));
        assert_eq!(state_of_update(&field.world), ADD_CHARACTER_STATE_DEAD);
        field.pulse(P0 + 1);
        assert_eq!(field.restart(RestartKind::Now).gold, Some(-300_000));
        let shown = field.join(CHARLIE, BESIDE);
        assert_eq!(alpha_state(&shown), Some(0));
        assert_eq!(state_of_update(&field.world), 0);
    }

    /// Only `SetPosition(POS_DEAD)` sets the dead state (`G/char.cpp:1446-1447`); `Stun` does
    /// not, so a stunned player is shown standing.
    #[test]
    fn a_newcomer_sees_a_stunned_player_standing() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.struck_down(ALPHA);

        let shown = field.join(BRAVO, BESIDE);
        assert_eq!(alpha_state(&shown), Some(0));
    }

    /// An item step's look carries the dead bit (`UpdatePacket`'s `bStateFlag`,
    /// `G/char.cpp:1310`): a dead player that wields a sword is still shown lying dead.
    #[test]
    fn a_dead_player_wielding_a_sword_is_still_shown_dead() {
        let mut field = a_field(true);
        let cell = ItemPos::new(EWindows::Inventory as u8, 0);
        let mut sword = Item::new(11, 10);
        sword.set_size(2).unwrap();
        let _alpha = field.join_with(ALPHA, (PLACE.1, HERE), 10, &[(cell, sword)]);
        field.killed(ALPHA);

        let weapon = INVENTORY_MAX_NUM + EWearPositions::Weapon as u16;
        let wield = MoveRequest {
            from: cell,
            to: ItemPos::new(EWindows::Inventory as u8, weapon),
            count: 0,
        };
        let worn = field
            .world
            .move_item(ALPHA, wield, mover())
            .expect("the sword is worn");
        let look = worn
            .records
            .iter()
            .find_map(|record| GcCharacterUpdate::decode(record).ok())
            .expect("a GC_CHARACTER_UPDATE");
        assert_eq!(look.b_state_flag, ADD_CHARACTER_STATE_DEAD);
    }

    /// A stunned player's move is refused with nothing sent (D9); a dead player's shows its body
    /// back where it lies, still dead, and stops it.
    #[test]
    fn a_stunned_player_cannot_move_and_a_dead_one_is_shown_back_where_it_lies() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        field.struck_down(ALPHA);
        field.quiet();
        let step = CgMove::new(FUNC_MOVE, 0, 18, HERE.0 + 50, HERE.1, 77);

        let refused = field.world.run_move(ALPHA, &step);
        assert!(
            matches!(refused, Some(MoveOutcome::Refuse(MoveRefusal::CannotMove))),
            "{refused:?}"
        );
        assert!(field.heard(ALPHA).is_empty());
        assert!(field.heard(BRAVO).is_empty());
        assert!(field.world.damage_pc(Some(MONSTER), ALPHA, 1, None));
        field.quiet();
        let refused = field.world.run_move(ALPHA, &step);
        assert!(
            matches!(refused, Some(MoveOutcome::Refuse(MoveRefusal::Dead))),
            "{refused:?}"
        );
        let seen = field.heard(BRAVO);
        assert_eq!(seen.first(), Some(&remove_record(ALPHA.raw())));
        assert_eq!(alpha_state(&seen), Some(ADD_CHARACTER_STATE_DEAD));
        assert_eq!(
            field.heard(ALPHA).first(),
            Some(&remove_record(ALPHA.raw()))
        );
        let spot = field.world.spot_of(ALPHA.raw()).expect("Alpha stands");
        assert_eq!((spot.x, spot.y), HERE);
    }

    /// D7 and D10: a stunned player uses no item but still drops one, picks one up and starts a
    /// trade, as legacy lets it; a dead player uses, drops and picks up nothing, starts no trade
    /// and is asked for none.
    #[test]
    fn a_dead_player_cannot_use_drop_or_pick_up_items_or_trade() {
        let mut field = a_field(true);
        let cell = ItemPos::new(EWindows::Inventory as u8, 0);
        let mut potions = Item::new(1, 27_001);
        potions.set_size(1).unwrap();
        potions.flags = ITEM_FLAG_STACKABLE;
        potions.count = 3;
        let _alpha = field.join_with(ALPHA, (PLACE.1, HERE), 10, &[(cell, potions)]);
        let _bravo = field.join(BRAVO, BESIDE);
        let refused = |reason| Err(MoveItemRefused::Refused(reason));

        field.struck_down(ALPHA);
        assert_eq!(
            field.world.use_item(ALPHA, cell, mover()),
            refused(MoveRefused::UsedWhileDown)
        );
        let dropped = field.world.drop_item(ALPHA, cell, 1, here(), mover());
        assert_ne!(dropped, refused(MoveRefused::DroppedWhileDead));
        // The pickup passes the dead guard and finds nothing under the VID.
        assert_eq!(
            field.world.pickup_item(ALPHA, 77, here(), mover()),
            refused(MoveRefused::NotOnGround)
        );
        let asked = TradeStep::Start {
            target: BRAVO.raw(),
        };
        let started = field.world.trade(ALPHA, asked, here(), mover());
        assert!(matches!(started, Ok(TradeAnswer::Sent(_))), "{started:?}");
        assert!(field.world.damage_pc(Some(MONSTER), ALPHA, 1, None));
        assert!(field.world.trading.is_empty(), "the death ends the trade");
        assert_eq!(
            field.world.use_item(ALPHA, cell, mover()),
            refused(MoveRefused::UsedWhileDown)
        );
        assert_eq!(
            field.world.drop_item(ALPHA, cell, 1, here(), mover()),
            refused(MoveRefused::DroppedWhileDead)
        );
        assert_eq!(
            field.world.pickup_item(ALPHA, 77, here(), mover()),
            refused(MoveRefused::PickedUpWhileDead)
        );
        let asked = TradeStep::Start {
            target: BRAVO.raw(),
        };
        let started = field.world.trade(ALPHA, asked, here(), mover());
        assert!(
            matches!(
                started,
                Ok(TradeAnswer::Declined {
                    reason: TradeDeclined::Dead,
                    ..
                })
            ),
            "{started:?}"
        );
        let asked = TradeStep::Start {
            target: ALPHA.raw(),
        };
        let beside = GroundPlace {
            x: BESIDE.0,
            ..here()
        };
        let started = field.world.trade(BRAVO, asked, beside, mover());
        assert!(
            matches!(
                started,
                Ok(TradeAnswer::Declined {
                    reason: TradeDeclined::PartnerDead,
                    ..
                })
            ),
            "{started:?}"
        );
        assert!(field.world.trading.is_empty());
    }

    /// Table F at 4500 Pulses, not 4499: the game phase, half the hit points shown, the window
    /// closed, then the warp. The flag is spent with no draw (B11, B15).
    #[test]
    fn the_automatic_restart_comes_4500_pulses_after_the_death_and_warps_to_town_with_half_hp() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        field.killed(ALPHA);
        assert!(field.combat(ALPHA).death_penalty);
        field.quiet();
        let dice = field.world.dice.clone();

        field.pulse(P0 + 4_499);
        assert!(field.heard(ALPHA).is_empty());
        assert!(field.ordered(ALPHA).is_empty());
        field.pulse(P0 + 4_500);
        let half = field.points(ALPHA).max_hp() / 2;
        assert!(half > 50, "half is not the restart's 50");
        assert_eq!(
            field.heard(ALPHA),
            vec![
                phase(PHASE_GAME),
                change(point::POINT_HP, i64::from(half), i64::from(half)),
                close(&field.world)
            ]
        );
        assert!(field.heard(BRAVO).is_empty());
        assert_eq!(field.ordered(ALPHA), vec![TO_TOWN]);
        assert_eq!(field.world.dice, dice);
        let combat = field.combat(ALPHA);
        assert!(!combat.dead && !combat.stunned && !combat.death_penalty);
        assert_eq!(field.points(ALPHA).hp(), half);
        assert_eq!(field.points(ALPHA).exp(), EXP);
        assert!(field.world.regens.holds(Due::PcRecovery(ALPHA.raw())));
    }

    /// G30: the first recovery heals the spirit then the hit points (`G/char.cpp:3261-3283`).
    /// Idled under 3 seconds, a level 10 warrior at 300/760 HP and 100/260 SP gains 4 SP
    /// (2 + max/100, idle with HP below max) then 22 HP (15 + max * 1 %).
    #[test]
    fn the_first_recovery_heals_sp_then_hp() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.set_hp_sp(ALPHA, 300, 100);
        field.quiet();
        field.pulse(P0 + 75);
        assert_eq!(field.heard(ALPHA), vec![sp_change(104), hp_change(322)]);
        assert_eq!(
            (field.points(ALPHA).hp(), field.points(ALPHA).sp()),
            (322, 104)
        );
    }

    /// G31: an attack within 3 seconds narrows the spirit gain to 3 (2 + max/200), while a
    /// full-HP idle player gains 11 (9 + max/100): the three `DistributeSP` arms.
    #[test]
    fn a_recent_attack_narrows_the_spirit_gain() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.set_hp_sp(ALPHA, 300, 100);
        let now = field.world.clock.now();
        field
            .world
            .bodies
            .get_mut(&ALPHA)
            .expect("Alpha stands")
            .last_attack = Some(now);
        field.quiet();
        field.pulse(P0 + 75);
        assert_eq!(field.points(ALPHA).sp(), 103);
        assert_eq!(field.heard(ALPHA), vec![sp_change(103), hp_change(322)]);
    }

    /// G31b: an idle player at full hit points gains 11 spirit (9 + max/100) and sends no
    /// HP record. The joins tick the clock, so the body is re-idled 10 seconds before pulsing.
    #[test]
    fn full_hp_widens_the_spirit_gain_and_sends_no_hp() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.set_hp_sp(ALPHA, 760, 100);
        field.clock.set(10_000);
        field
            .world
            .bodies
            .get_mut(&ALPHA)
            .expect("Alpha stands")
            .last_move = 0;
        field.quiet();
        field.pulse(P0 + 75);
        assert_eq!(field.heard(ALPHA), vec![sp_change(111)]);
        assert_eq!(
            (field.points(ALPHA).hp(), field.points(ALPHA).sp()),
            (760, 111)
        );
    }

    /// G32: both pools clamp at their maxima, and a full player hears nothing.
    #[test]
    fn the_heal_clamps_at_both_maxima() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.set_hp_sp(ALPHA, 755, 259);
        field.quiet();
        field.pulse(P0 + 75);
        assert_eq!(field.heard(ALPHA), vec![sp_change(260), hp_change(760)]);
        assert_eq!(
            (field.points(ALPHA).hp(), field.points(ALPHA).sp()),
            (760, 260)
        );
        field.quiet();
        field.pulse(P0 + 150);
        assert!(field.heard(ALPHA).is_empty());
    }

    /// Table C at 226 Pulses, refused at 225 with one second left; after it, the dead event's
    /// Pulse brings no second restart.
    #[test]
    fn a_dead_player_restarting_here_stands_with_50_hp_and_a_new_revive() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        let died = field.stunned_to_death(ALPHA);
        field.quiet();
        let dice = field.world.dice.clone();

        field.pulse(died + 225);
        assert_eq!(field.restart(RestartKind::Here), RestartAnswer::default());
        assert_eq!(field.heard(ALPHA), vec![wait(&field.world, 1)]);
        assert!(field.world.is_dead(ALPHA.raw()));
        field.pulse(died + 226);
        assert_eq!(field.restart(RestartKind::Here), RestartAnswer::default());
        let after = [hp(50), update(&field.world), revive()];
        reinserted(&field.heard(ALPHA), &revived(&field.world), &after);
        let seen = field.heard(BRAVO);
        reinserted(
            &seen,
            &[remove_record(ALPHA.raw())],
            &[update(&field.world)],
        );
        assert_eq!(field.world.dice, dice, "a flagless death draws nothing");
        assert!(!field.world.is_down(ALPHA.raw()));
        assert_eq!(field.points(ALPHA).hp(), 50);
        assert!(field.world.regens.holds(Due::PcRecovery(ALPHA.raw())));

        // The heal is real now: one recovery runs in the wait, idled under 3 seconds.
        field.pulse(died + 4_500);
        let later = field.heard(ALPHA);
        assert!(
            !later
                .iter()
                .any(|record| record.first() == Some(&HEADER_GC_PHASE.value())),
            "{later:?}"
        );
        assert!(field.ordered(ALPHA).is_empty());
        // The wait healed 15 + max * 1 % once.
        let max = field.points(ALPHA).max_hp();
        assert_eq!(field.points(ALPHA).hp(), 50 + 15 + max / 100);
    }

    /// `ReviveInvisible` over a running revive resets it in place and sends its removal before
    /// the update and the fresh affect: one revive runs after.
    #[test]
    fn a_restart_inside_the_revive_window_ends_the_old_revive_first() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _update = field.world.enter_revive_invisible(ALPHA);
        field.set_gold(ALPHA, 300_000);
        let died = field.stunned_to_death(ALPHA);
        let old = Affect {
            remaining: 2,
            ..Affect::revive_invisible()
        };
        assert_eq!(field.world.running_affects(ALPHA), vec![old]);
        field.quiet();

        field.pulse(died + 1);
        let _paid = field.restart(RestartKind::Now);
        let full = field.points(ALPHA).max_hp();
        let after = [
            hp(full),
            affect_remove_frame(&old),
            update(&field.world),
            revive(),
            gold_left(0),
        ];
        reinserted(&field.heard(ALPHA), &revived(&field.world), &after);
        assert_eq!(
            field.world.running_affects(ALPHA),
            vec![Affect::revive_invisible()]
        );
    }

    /// Table E at 151 Pulses, refused at 150 with one second left: no penalty in town (B11).
    #[test]
    fn a_dead_player_restarting_in_town_waits_151_pulses_then_warps_with_50_hp() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.killed(ALPHA);
        field.quiet();
        let dice = field.world.dice.clone();

        field.pulse(P0 + 150);
        assert_eq!(field.restart(RestartKind::Town), RestartAnswer::default());
        assert_eq!(field.heard(ALPHA), vec![wait(&field.world, 1)]);
        assert!(field.ordered(ALPHA).is_empty());
        field.pulse(P0 + 151);
        assert_eq!(field.restart(RestartKind::Town), RestartAnswer::default());
        assert_eq!(
            field.heard(ALPHA),
            vec![close(&field.world), phase(PHASE_GAME), hp(50)]
        );
        assert_eq!(field.ordered(ALPHA), vec![TO_TOWN]);
        assert_eq!(field.world.dice, dice);
        assert!(!field.combat(ALPHA).death_penalty, "the flag is spent");
        assert_eq!(field.points(ALPHA).exp(), EXP);
    }

    /// B24 and B1: until 250 Pulses after a trade every restart is held back with `IsHack`'s
    /// line alone: before the town and here waits, which still run 100 Pulses after the death
    /// and are over 226 Pulses after it, and after `/restart_now`'s price, which is paid. Then
    /// the town restart passes.
    #[test]
    fn a_restart_refused_by_a_recent_trade_hears_only_that_refusal() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        field.set_gold(ALPHA, 300_000);
        let start = TradeStep::Start {
            target: BRAVO.raw(),
        };
        let started = field.world.trade(ALPHA, start, here(), mover());
        assert!(matches!(started, Ok(TradeAnswer::Sent(_))), "{started:?}");
        field.killed(ALPHA);
        field.quiet();
        let dice = field.world.dice.clone();

        let recent = line(
            &field.world,
            CHAT_TYPE_INFO,
            b"[LS;852;%d]",
            &[Arg::Int(10)],
        );
        for pulse in [P0 + 100, P0 + 226] {
            field.pulse(pulse);
            assert_eq!(field.world.is_hack(ALPHA, mover()), Some(recent.clone()));
            for kind in [RestartKind::Here, RestartKind::Town, RestartKind::Now] {
                assert_eq!(field.restart(kind), RestartAnswer::default(), "{kind:?}");
                assert_eq!(
                    field.heard(ALPHA),
                    vec![recent.clone()],
                    "{kind:?} at {pulse}"
                );
            }
        }
        assert!(field.world.is_dead(ALPHA.raw()));
        assert!(field.ordered(ALPHA).is_empty());
        assert_eq!(field.character(ALPHA).gold(), 300_000);
        assert_eq!(field.world.dice, dice);
        assert!(field.combat(ALPHA).death_penalty, "no penalty ran");
        field.pulse(P0 + 250);
        assert_eq!(field.restart(RestartKind::Town), RestartAnswer::default());
        assert_eq!(field.ordered(ALPHA), vec![TO_TOWN]);
    }

    /// The safebox a death closes keeps its load time (`CloseSafebox`'s `SetSafeboxLoadTime`,
    /// `G/char.cpp:7195`), so `IsHack` (`:8234-8241`) holds `/restart_here` back until 250
    /// Pulses after the death, though the here wait is over at 226; the load of the opening,
    /// 100 Pulses before the death, is long past by then.
    #[test]
    fn a_safebox_closed_by_the_death_holds_the_restart_back_250_pulses() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let begun = field.world.safebox(ALPHA, SafeboxStep::BeginOpen, mover());
        assert!(matches!(begun, Ok(SafeboxAnswer::Proceed)), "{begun:?}");
        let open = SafeboxStep::Open {
            account: 3,
            items: Vec::new(),
        };
        let opened = field.world.safebox(ALPHA, open, mover());
        assert!(matches!(opened, Ok(SafeboxAnswer::Sent(_))), "{opened:?}");
        field.pulse(P0 + 25);
        let died = field.stunned_to_death(ALPHA);
        assert_eq!(died, P0 + 100);
        field.quiet();

        let refused = line(
            &field.world,
            CHAT_TYPE_INFO,
            b"[LS;850;%d]",
            &[Arg::Int(10)],
        );
        for pulse in [died + 226, died + 249] {
            field.pulse(pulse);
            assert_eq!(field.restart(RestartKind::Here), RestartAnswer::default());
            assert_eq!(field.heard(ALPHA), vec![refused.clone()], "at {pulse}");
            assert!(field.world.is_dead(ALPHA.raw()));
        }
        field.pulse(died + 250);
        assert_eq!(field.restart(RestartKind::Here), RestartAnswer::default());
        let after = [hp(50), update(&field.world), revive()];
        reinserted(&field.heard(ALPHA), &revived(&field.world), &after);
        assert!(!field.world.is_down(ALPHA.raw()));
    }

    /// Table D one Pulse after the death: full hit points, the revive, then the gold left.
    #[test]
    fn a_dead_player_paying_300000_gold_restarts_at_once_where_it_lies_at_full_hp() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.set_gold(ALPHA, 300_000);
        let died = field.stunned_to_death(ALPHA);
        field.quiet();

        field.pulse(died + 1);
        let answer = field.restart(RestartKind::Now);
        assert_eq!(answer.gold, Some(-300_000));
        let full = field.points(ALPHA).max_hp();
        let after = [hp(full), update(&field.world), revive(), gold_left(0)];
        reinserted(&field.heard(ALPHA), &revived(&field.world), &after);
        assert_eq!(field.character(ALPHA).gold(), 0);
        assert_eq!(field.points(ALPHA).hp(), full);
        assert!(!field.world.is_down(ALPHA.raw()));
        assert!(field.ordered(ALPHA).is_empty());
    }

    /// A player a monster killed is sent its revive update on `/restart_now`: every earlier
    /// restart test died by the stun event, with no killer.
    #[test]
    fn a_player_killed_by_a_monster_is_sent_its_revive_update_on_restart_now() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.set_gold(ALPHA, 300_000);
        field.killed(ALPHA);
        field.quiet();

        field.pulse(field.world.last_pulse + 1);
        let answer = field.restart(RestartKind::Now);
        assert_eq!(answer.gold, Some(-300_000));
        let heard = field.heard(ALPHA);
        let update = update(&field.world);
        assert!(
            heard.contains(&update),
            "the revive update is sent: {:?}",
            heard.iter().map(|r| (r[0], r.len())).collect::<Vec<_>>()
        );
    }

    /// The world's `Restart` arm answers its reply with what the restart took: the descriptor
    /// stores the 300 000 gold `/restart_now` cost.
    #[test]
    fn a_restart_command_answers_the_gold_restart_now_took() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.set_gold(ALPHA, 300_000);
        let died = field.stunned_to_death(ALPHA);
        field.pulse(died + 1);

        let (reply, answer) = tokio::sync::oneshot::channel();
        field.world.apply(GameCommand::Restart {
            vid: ALPHA,
            kind: RestartKind::Now,
            mover: mover(),
            reply,
        });
        let answer = answer.blocking_recv().expect("the world answered");
        assert_eq!(answer.gold, Some(-300_000));
        assert!(!field.world.is_down(ALPHA.raw()));
        assert_eq!(field.character(ALPHA).gold(), 0);
    }

    /// `/restart_now` pays the here penalty (`G/cmd_general.cpp:512`): a flagged death rolling 0
    /// loses the level share after the full hit points, and the flag is spent.
    #[test]
    fn a_flagged_death_restarted_now_rolling_zero_takes_the_level_share_of_the_exp() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.set_gold(ALPHA, 300_000);
        field.killed(ALPHA);
        field.world.dice = Pcg32::new(a_seed(true), 0);
        field.quiet();

        field.pulse(P0 + 1);
        assert_eq!(field.restart(RestartKind::Now).gold, Some(-300_000));
        let full = field.points(ALPHA).max_hp();
        let taken = change(point::POINT_EXP, -LOSS, EXP - LOSS);
        let after = [
            hp(full),
            taken,
            update(&field.world),
            revive(),
            gold_left(0),
        ];
        reinserted(&field.heard(ALPHA), &revived(&field.world), &after);
        assert_eq!(field.points(ALPHA).exp(), EXP - LOSS);
        assert!(!field.combat(ALPHA).death_penalty, "the flag is spent");
    }

    #[test]
    fn a_dead_player_short_of_300000_gold_hears_the_price_and_stays_dead() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.set_gold(ALPHA, 299_999);
        let died = field.stunned_to_death(ALPHA);
        field.quiet();

        field.pulse(died + 1);
        assert_eq!(field.restart(RestartKind::Now), RestartAnswer::default());
        let price = line(&field.world, CHAT_TYPE_INFO, RESTART_NOW_PRICE_NOTICE, &[]);
        assert_eq!(field.heard(ALPHA), vec![price]);
        assert!(field.world.is_dead(ALPHA.raw()));
        assert_eq!(field.character(ALPHA).gold(), 299_999);
    }

    /// Table G: whatever the gold, a living or stunned player's restart only closes the window;
    /// a stunned one starts no recovery.
    #[test]
    fn a_living_or_stunned_player_restarting_is_only_told_to_close_the_restart_window() {
        let kinds = [RestartKind::Here, RestartKind::Town, RestartKind::Now];
        for gold in [299_999, 400_000] {
            let mut field = a_field(true);
            let _alpha = field.join(ALPHA, HERE);
            field.set_gold(ALPHA, gold);
            field.quiet();
            for kind in kinds {
                assert_eq!(field.restart(kind), RestartAnswer::default());
                assert_eq!(field.heard(ALPHA), vec![close(&field.world)], "{kind:?}");
            }
            field.struck_down(ALPHA);
            field.quiet();
            for kind in kinds {
                assert_eq!(field.restart(kind), RestartAnswer::default());
                assert_eq!(field.heard(ALPHA), vec![close(&field.world)], "{kind:?}");
                assert!(!field.world.regens.holds(Due::PcRecovery(ALPHA.raw())));
            }
            assert!(field.world.is_stunned(ALPHA.raw()));
            assert_eq!(field.character(ALPHA).gold(), gold);
            assert!(field.ordered(ALPHA).is_empty());
        }
    }

    /// Alpha of `level`, killed by a monster when `flagged` and else by the stun event, restarts
    /// here 226 Pulses on with `dice`. Answers the field and what Alpha heard of the restart.
    fn restarted_here(level: u8, flagged: bool, dice: Pcg32) -> (Field, Vec<Vec<u8>>) {
        let mut field = a_field(true);
        let _alpha = field.join_with(ALPHA, (PLACE.1, HERE), level, &[]);
        let heard = field.dies_and_restarts_here(flagged, dice);
        (field, heard)
    }

    /// D2: the flag is tested before the level and the roll, so a flagless death draws
    /// nothing, and below level 10 it hears no `[LS;901]` either.
    #[test]
    fn a_flagless_death_draws_nothing_and_takes_nothing() {
        for level in [9, 10] {
            let dice = Pcg32::new(a_seed(true), 0);
            let (field, heard) = restarted_here(level, false, dice.clone());
            let after = [hp(50), update(&field.world), revive()];
            reinserted(&heard, &revived(&field.world), &after);
            assert_eq!(field.world.dice, dice, "level {level}");
            assert_eq!(field.points(ALPHA).exp(), EXP);
        }
    }

    /// A roll of 0 takes 4 % of the next level's cost from a level 10 player, with one draw.
    #[test]
    fn a_flagged_death_rolling_zero_takes_the_level_share_of_the_exp() {
        assert_eq!(death_penalty_loss(10, levels::next_exp(10), false), LOSS);
        let seed = a_seed(true);
        let (field, heard) = restarted_here(10, true, Pcg32::new(seed, 0));
        let taken = change(point::POINT_EXP, -LOSS, EXP - LOSS);
        let after = [hp(50), taken, update(&field.world), revive()];
        reinserted(&heard, &revived(&field.world), &after);
        let mut rolled = Pcg32::new(seed, 0);
        let _roll = number(&mut rolled, 0, 2);
        assert_eq!(field.world.dice, rolled, "one draw");
        assert_eq!(field.points(ALPHA).exp(), EXP - LOSS);
        assert!(!field.combat(ALPHA).death_penalty, "the flag is spent");
    }

    /// Any other roll says `[LS;901]` and takes nothing; the flag is spent all the same.
    #[test]
    fn a_flagged_death_rolling_more_than_zero_hears_901_and_keeps_its_exp() {
        let seed = a_seed(false);
        let (field, heard) = restarted_here(10, true, Pcg32::new(seed, 0));
        let spared = line(&field.world, CHAT_TYPE_INFO, NO_PENALTY_NOTICE, &[]);
        let after = [hp(50), spared, update(&field.world), revive()];
        reinserted(&heard, &revived(&field.world), &after);
        let mut rolled = Pcg32::new(seed, 0);
        let _roll = number(&mut rolled, 0, 2);
        assert_eq!(field.world.dice, rolled);
        assert_eq!(field.points(ALPHA).exp(), EXP);
        assert!(!field.combat(ALPHA).death_penalty);
    }

    /// Below level 10 the penalty says `[LS;901]` before any draw.
    #[test]
    fn a_player_below_level_10_hears_901_and_keeps_its_exp() {
        let dice = Pcg32::new(a_seed(true), 0);
        let (field, heard) = restarted_here(9, true, dice.clone());
        let spared = line(&field.world, CHAT_TYPE_INFO, NO_PENALTY_NOTICE, &[]);
        let after = [hp(50), spared, update(&field.world), revive()];
        reinserted(&heard, &revived(&field.world), &after);
        assert_eq!(field.world.dice, dice, "no draw at level 9");
        assert_eq!(field.points(ALPHA).exp(), EXP);
        assert!(!field.combat(ALPHA).death_penalty, "the flag is spent");
    }

    /// A worn Teardrop of Godness halves the loss (`G/char_battle.cpp:385-386`) in either
    /// unique cell `IsEquipUniqueItem` reads (`G/char_item.cpp:9519-9533`).
    #[test]
    fn a_player_wearing_the_teardrop_of_godness_loses_half_the_level_share() {
        let half = death_penalty_loss(10, levels::next_exp(10), true);
        assert_eq!(half, LOSS / 2);
        for wear in [EWearPositions::Unique1, EWearPositions::Unique2] {
            let cell = INVENTORY_MAX_NUM + wear as u16;
            let teardrop = Item::new(1, TEARDROP_OF_GODNESS);
            let worn = [(ItemPos::new(EWindows::Inventory as u8, cell), teardrop)];
            let mut field = a_field(true);
            let _alpha = field.join_with(ALPHA, (PLACE.1, HERE), 10, &worn);
            let heard = field.dies_and_restarts_here(true, Pcg32::new(a_seed(true), 0));
            let taken = change(point::POINT_EXP, -half, EXP - half);
            let after = [hp(50), taken, update(&field.world), revive()];
            reinserted(&heard, &revived(&field.world), &after);
            assert_eq!(field.points(ALPHA).exp(), EXP - half, "{wear:?}");
        }
    }

    /// `PointChange(POINT_EXP)` takes nothing from a player at `game.max_level` holding a
    /// quarter of its next level's cost (`G/char.cpp:4113-4116`), and no record is sent; one
    /// level below the configured highest, the same player loses the level share.
    #[test]
    fn a_player_at_the_configured_max_level_holding_a_quarter_of_its_next_level_loses_nothing() {
        let quarter = levels::next_exp(10) / 4;
        for (max_level, lost) in [(10, 0), (11, LOSS)] {
            let mut field = a_field(true);
            field.world = field.world.with_max_level(max_level);
            let _alpha = field.join(ALPHA, HERE);
            let mut points = points_at(10, 12_345, quarter);
            let _computed = points.take_target_broadcasts();
            field
                .world
                .characters
                .find_by_vid_mut(ALPHA)
                .expect("Alpha is online")
                .set_points(Some(points));
            let heard = field.dies_and_restarts_here(true, Pcg32::new(a_seed(true), 0));
            let mut after = vec![hp(50), update(&field.world), revive()];
            if lost > 0 {
                after.insert(1, change(point::POINT_EXP, -lost, quarter - lost));
            }
            reinserted(&heard, &revived(&field.world), &after);
            assert_eq!(field.points(ALPHA).exp(), quarter - lost, "max {max_level}");
            assert!(!field.combat(ALPHA).death_penalty, "the flag is spent");
        }
    }

    /// The battle arena's maps take nothing, draw nothing, and leave the flag as it was.
    #[test]
    fn no_death_penalty_on_the_arena_maps() {
        for map in ARENA_MAPS {
            let mut field = a_field(true);
            let grid = SectreeGrid {
                x: 0,
                y: 0,
                columns: 10,
                rows: 10,
            };
            field.world.host_map(PLACE.0, map, grid);
            let _alpha = field.join_with(ALPHA, (map, HERE), 10, &[]);
            field.killed(ALPHA);
            field.world.dice = Pcg32::new(a_seed(true), 0);
            let dice = field.world.dice.clone();
            field.quiet();

            field.pulse(P0 + 226);
            assert_eq!(field.restart(RestartKind::Here), RestartAnswer::default());
            let after = [hp(50), update(&field.world), revive()];
            reinserted(&field.heard(ALPHA), &revived(&field.world), &after);
            assert_eq!(field.world.dice, dice, "map {map}");
            assert_eq!(field.points(ALPHA).exp(), EXP);
            assert!(
                field.combat(ALPHA).death_penalty,
                "map {map} leaves the flag"
            );
        }
    }

    /// B5: a player that disconnects dead pays the here penalty and is kept standing with 50
    /// hit points; its events end with it, so after a relog only a new death's event fires.
    #[test]
    fn a_disconnect_while_dead_saves_50_hp_and_pays_the_here_penalty() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, BESIDE);
        field.killed(ALPHA);
        field.world.dice = Pcg32::new(a_seed(true), 0);
        field.pulse(10);

        let kept = field.leave(ALPHA, Departure::Disconnect);
        let points = kept.points.expect("the points are kept");
        assert_eq!((points.hp(), points.exp()), (50, EXP - LOSS));
        assert!(!field.world.pc_combat.contains_key(&ALPHA.raw()));
        for due in [Due::PcStun(1), Due::PcDead(1), Due::PcRecovery(1)] {
            assert!(!field.world.regens.holds(due), "{due:?}");
        }
        let _alpha = field.join(ALPHA, HERE);
        field.pulse(20);
        field.killed(ALPHA);
        field.quiet();
        field.pulse(P0 + 4_500);
        assert!(
            field.heard(ALPHA).is_empty(),
            "the first death's event ended"
        );
        assert!(field.ordered(ALPHA).is_empty());
        field.pulse(20 + 4_500);
        assert_eq!(field.ordered(ALPHA), vec![TO_TOWN]);
    }

    /// B5 for a stunned player: 50 hit points, and with no flag no draw.
    #[test]
    fn a_disconnect_while_stunned_saves_50_hp() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.struck_down(ALPHA);
        let dice = field.world.dice.clone();

        let kept = field.leave(ALPHA, Departure::Disconnect);
        let points = kept.points.expect("the points are kept");
        assert_eq!((points.hp(), points.exp()), (50, EXP));
        assert_eq!(field.world.dice, dice);
        assert!(!field.world.regens.holds(Due::PcStun(ALPHA.raw())));
    }

    /// `Destroy`'s `ClearSync`: a disconnect tells the view the player has no owner, then
    /// releases what it owns; a warp, which left its sectree first, releases only what it owns.
    #[test]
    fn a_leaving_player_releases_its_own_ownership_before_what_it_owns() {
        for departure in [Departure::Disconnect, Departure::Warp] {
            let mut field = a_field(true);
            let _alpha = field.join(ALPHA, HERE);
            let _bravo = field.join(BRAVO, BESIDE);
            let claimed = Some(Duration::from_millis(500));
            for (victim, owner) in [(ALPHA, BRAVO), (BRAVO, ALPHA)] {
                field.world.bodies.get_mut(&victim).unwrap().sync = SyncOwnershipState {
                    owner: Some(owner),
                    claimed_at: claimed,
                };
            }
            field.quiet();

            let _kept = field.leave(ALPHA, departure);
            let own = (departure == Departure::Disconnect).then(|| unowned(ALPHA));
            let expected: Vec<Vec<u8>> = own
                .into_iter()
                .chain([unowned(BRAVO), remove_record(ALPHA.raw())])
                .collect();
            assert_eq!(field.heard(BRAVO), expected, "{departure:?}");
            assert_eq!(field.world.bodies[&BRAVO].sync.owner, None);
        }
    }

    /// `Destroy` cancels the recovery event (`G/char.cpp:714`): a living player leaves none
    /// behind, so after a relog only the new one runs.
    #[test]
    fn a_living_player_that_leaves_leaves_no_recovery_event_behind() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        assert_eq!(field.combat(ALPHA).recovery_due, Some(P0 + 75));

        let _kept = field.leave(ALPHA, Departure::Disconnect);
        assert!(!field.world.regens.holds(Due::PcRecovery(ALPHA.raw())));
        field.pulse(20);
        let _alpha = field.join(ALPHA, HERE);
        field.pulse(P0 + 75);
        assert_eq!(field.combat(ALPHA).recovery_due, Some(20 + 75));
    }

    /// `CheckTarget` in the recovery event, every 75 Pulses: 4799 away is kept, 4800 dropped.
    #[test]
    fn a_player_target_4800_away_is_dropped_and_one_4799_away_is_kept() {
        assert_eq!(distance_approx(4_995, 0), 4_799);
        assert_eq!(distance_approx(4_996, 0), 4_800);
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, (HERE.0 + 4_995, HERE.1));
        let _charlie = field.join(CHARLIE, (HERE.0 + 4_996, HERE.1));
        field.world.target(ALPHA, BRAVO.raw());
        field.quiet();

        field.pulse(P0 + 75);
        assert!(field.heard(ALPHA).is_empty());
        assert_eq!(field.world.targets.of(ALPHA.raw()), Some(BRAVO.raw()));
        field.world.target(ALPHA, CHARLIE.raw());
        field.quiet();
        field.pulse(P0 + 149);
        assert!(field.heard(ALPHA).is_empty(), "the event waits 75 Pulses");
        field.pulse(P0 + 150);
        assert_eq!(field.heard(ALPHA), vec![target_record(0, (0, 0, 0), 0)]);
        assert_eq!(field.world.targets.of(ALPHA.raw()), None);
    }

    /// The same reach for a non-player target.
    #[test]
    fn a_monster_target_4800_away_is_dropped_and_one_4799_away_is_kept() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let monster = |vid: u32, dx: i32| Npc {
            vid,
            vnum: 101,
            race: 101,
            char_type: CHAR_TYPE_MONSTER,
            x: HERE.0 + dx,
            y: HERE.1,
            hp: 300,
            max_hp: 400,
            ..Npc::default()
        };
        let (near, far) = (FIRST_MONSTER, FIRST_MONSTER + 1);
        let npcs = vec![monster(near, 4_995), monster(far, 4_996)];
        for (at, npc) in npcs.iter().enumerate() {
            let _placed = field.world.npc_of.insert(npc.vid, (PLACE.0, PLACE.1, at));
        }
        let map = MapNpcs {
            npcs,
            positions: Vec::new(),
            regens: Vec::new(),
        };
        let _stood = field.world.npcs.insert(PLACE, Arc::new(map));
        field.world.target(ALPHA, near);
        field.quiet();

        field.pulse(P0 + 75);
        assert!(field.heard(ALPHA).is_empty());
        assert_eq!(field.world.targets.of(ALPHA.raw()), Some(near));
        field.world.target(ALPHA, far);
        field.quiet();
        field.pulse(P0 + 150);
        assert_eq!(field.heard(ALPHA), vec![target_record(0, (0, 0, 0), 0)]);
        assert_eq!(field.world.targets.of(ALPHA.raw()), None);
    }

    /// The first non-player VID the monster target test stands.
    const FIRST_MONSTER: u32 = 0x8000_0001;

    /// `CheckTarget` measures both coordinates: 4000 off on each axis is 5437 away and dropped,
    /// while 4000 off on one axis alone is 3843 away and kept.
    #[test]
    fn a_target_4000_off_on_both_axes_is_dropped_and_one_4000_off_on_one_axis_is_kept() {
        assert_eq!(distance_approx(4_000, 0), 3_843);
        assert_eq!(distance_approx(4_000, 4_000), 5_437);
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _bravo = field.join(BRAVO, (HERE.0 + 4_000, HERE.1));
        let _charlie = field.join(CHARLIE, (HERE.0 + 4_000, HERE.1 + 4_000));
        field.world.target(ALPHA, BRAVO.raw());
        field.quiet();

        field.pulse(P0 + 75);
        assert!(field.heard(ALPHA).is_empty());
        assert_eq!(field.world.targets.of(ALPHA.raw()), Some(BRAVO.raw()));
        field.world.target(ALPHA, CHARLIE.raw());
        field.quiet();
        field.pulse(P0 + 150);
        assert_eq!(field.heard(ALPHA), vec![target_record(0, (0, 0, 0), 0)]);
        assert_eq!(field.world.targets.of(ALPHA.raw()), None);
    }

    /// The stun ends the recovery, a dead player starts none, and a restart starts it again 75
    /// Pulses on.
    #[test]
    fn a_stun_stops_the_recovery_and_a_restart_starts_it_again() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        assert_eq!(field.combat(ALPHA).recovery_due, Some(P0 + 75));

        let died = field.stunned_to_death(ALPHA);
        field.world.start_pc_recovery(ALPHA);
        assert_eq!(field.combat(ALPHA).recovery_due, None);
        assert!(!field.world.regens.holds(Due::PcRecovery(ALPHA.raw())));
        field.pulse(died + 226);
        let _answer = field.restart(RestartKind::Here);
        assert_eq!(field.combat(ALPHA).recovery_due, Some(died + 226 + 75));
        field.pulse(died + 301);
        assert_eq!(field.combat(ALPHA).recovery_due, Some(died + 376));
    }

    /// `StartRecoveryEvent` returns while one runs (`G/char.cpp:3289-3290`): a living player's
    /// restart keeps the Pulse its running recovery is due on.
    #[test]
    fn a_living_players_restart_keeps_its_running_recovery_due_where_it_was() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        field.pulse(P0 + 10);

        let _answer = field.restart(RestartKind::Here);
        assert_eq!(field.combat(ALPHA).recovery_due, Some(P0 + 75));
    }

    /// A recovery event whose slot is empty, as a stunned player's is, does nothing: it drops
    /// no target, however far, and is not due again.
    #[test]
    fn a_recovery_event_with_no_slot_checks_no_target_and_is_not_due_again() {
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let _charlie = field.join(CHARLIE, (HERE.0 + 4_996, HERE.1));
        field.world.target(ALPHA, CHARLIE.raw());
        field.struck_down(ALPHA);
        field.quiet();

        field.world.pc_recovery_event(ALPHA.raw());
        assert!(field.heard(ALPHA).is_empty());
        assert_eq!(field.world.targets.of(ALPHA.raw()), Some(CHARLIE.raw()));
        assert_eq!(field.combat(ALPHA).recovery_due, None);
        assert!(!field.world.regens.holds(Due::PcRecovery(ALPHA.raw())));
    }

    /// B12: a town no Channel serves, or no region the atlas names, restarts the player in
    /// place with the here records and no warp.
    #[test]
    fn a_town_restart_with_no_served_recall_point_restarts_in_place() {
        let elsewhere = MapRegion {
            index: 99,
            ..region()
        };
        for field in [a_field(false), a_field_recalling(vec![elsewhere], true)] {
            let mut field = field;
            let _alpha = field.join(ALPHA, HERE);
            field.killed(ALPHA);
            field.quiet();

            field.pulse(P0 + 151);
            assert_eq!(field.restart(RestartKind::Town), RestartAnswer::default());
            let after = [hp(50), update(&field.world), revive()];
            reinserted(&field.heard(ALPHA), &revived(&field.world), &after);
            assert!(field.ordered(ALPHA).is_empty());
            assert!(!field.world.is_down(ALPHA.raw()));
        }
    }

    /// `CMapLocation::Get` finds a map the Shared Channel serves too (`G/map_location.cpp:17-41`):
    /// a town map that only it serves, and not the player's own Channel, is still warped to.
    #[test]
    fn a_town_restart_warps_to_a_town_only_the_shared_channel_serves() {
        let town = MapRegion {
            spawn: TOWN,
            ..region()
        };
        let own = HashMap::from([(PLACE.0, vec![1])]);
        let shared = [u32::try_from(PLACE.1).unwrap()];
        let mut field = a_field_serving(vec![town], &own, &shared);
        let _alpha = field.join(ALPHA, HERE);
        field.killed(ALPHA);
        field.quiet();

        field.pulse(P0 + 151);
        assert_eq!(field.restart(RestartKind::Town), RestartAnswer::default());
        assert_eq!(
            field.heard(ALPHA),
            vec![close(&field.world), phase(PHASE_GAME), hp(50)]
        );
        assert_eq!(field.ordered(ALPHA), vec![TO_TOWN]);
    }

    /// A map the atlas has no region for has no recall position, so the town restart warps to
    /// the empire's start (`G/cmd_general.cpp:687-690`), kept because the atlas puts that start
    /// on map 1, which Channel 1 serves.
    #[test]
    fn a_town_restart_on_a_map_the_atlas_does_not_name_warps_to_the_empire_start() {
        let (x, y) = warp::go_home(1).expect("empire 1 has a start");
        let start = MapRegion {
            index: 1,
            sx: x - 100,
            sy: y - 100,
            ex: x + 100,
            ey: y + 100,
            ..region()
        };
        let own = HashMap::from([(PLACE.0, vec![1])]);
        let mut field = a_field_serving(vec![start], &own, &[]);
        let _alpha = field.join(ALPHA, HERE);
        field.killed(ALPHA);
        field.quiet();

        field.pulse(P0 + 151);
        assert_eq!(field.restart(RestartKind::Town), RestartAnswer::default());
        assert_eq!(field.ordered(ALPHA), vec![ClientOrder::Warp { x, y }]);
        assert!(!field.world.is_down(ALPHA.raw()));
    }

    /// `GiveExp`'s grant on a player: the experience banks, each record goes to the player
    /// alone, a broadcast record goes to the view, the level-up raises the quest `levelup`
    /// event, and the body's card follows the level (`G/char_battle.cpp:2675`).
    #[test]
    fn an_exp_gain_banks_sends_and_raises_the_quest_levelup() {
        use quest::manager::Manager;

        use super::super::quests::Quests;
        let mut field = a_field(true);
        field.join_with(ALPHA, (PLACE.1, HERE), 39, &[]);
        let _bravo = field.join(BRAVO, BESIDE);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        field.world.quests = Some(Quests::new(
            manager,
            std::sync::Arc::default(),
            gamedata::mob_locale_names::MobNamesByLanguage::default(),
        ));
        field.world.start_a_quest(ALPHA);
        field.world.end_the_quest(ALPHA);
        field.quiet();
        // Level 39 holds 5,000 of 6,264,000; the grant crosses the cost onto 40.
        field.world.dice = Pcg32::new(0, 0);
        field.world.gain_pc_exp(ALPHA, 6_264_000 - 5_000);
        assert_eq!(field.points(ALPHA).level(), 40);
        assert_eq!(field.points(ALPHA).exp(), 0);
        // The quest `levelup` ran: sash's `set_state("information")` moved its status out of
        // `start`'s 0, and it sends nothing.
        let quests = field.world.quests.as_ref().expect("quests loaded");
        assert_eq!(
            quests.manager().quest_flag(1, b"sash_mission.__status"),
            Some(695_801_987)
        );
        let heard = field.heard(ALPHA);
        let exp = point_changes(
            &[PointRecord {
                kind: u8::try_from(point::POINT_EXP).unwrap(),
                amount: 6_264_000 - 5_000,
                value: 0,
                broadcast: false,
            }],
            ALPHA.raw(),
        );
        assert!(heard.iter().any(|record| *record == exp[0]));
        let seen = field.heard(BRAVO);
        let level = point_changes(
            &[PointRecord {
                kind: u8::try_from(point::POINT_LEVEL).unwrap(),
                amount: 0,
                value: 40,
                broadcast: true,
            }],
            ALPHA.raw(),
        );
        assert!(seen.iter().any(|record| *record == level[0]));
    }

    /// A gain that carries the level onto the protect level switches the body's PK mode from
    /// `PROTECT` to `PEACE` with an update (`SetLevel`, `G/char.cpp:2221`); a gain that stays
    /// below it changes nothing.
    #[test]
    fn a_level_up_onto_the_protect_level_leaves_protect_mode() {
        use crate::loading_phase::PK_MODE_PEACE;
        let mut field = a_field(true);
        field.join_with(ALPHA, (PLACE.1, HERE), 14, &[]);
        field.quiet();
        // Level 14 holds 5,000 of 100,000; 95,000 crosses the cost onto 15.
        field.world.dice = Pcg32::new(0, 0);
        field.world.gain_pc_exp(ALPHA, 95_000);
        assert_eq!(field.points(ALPHA).level(), 15);
        let body = field.world.bodies.get(&ALPHA).expect("Alpha has a body");
        assert_eq!(body.card.pk_mode, PK_MODE_PEACE);
        assert_eq!(body.card.level, 15);
        assert!(field
            .heard(ALPHA)
            .iter()
            .any(|record| *record == update(&field.world)));
        // Below the protect level the mode stays `PROTECT`.
        let mut low = a_field(true);
        low.join_with(ALPHA, (PLACE.1, HERE), 10, &[]);
        low.quiet();
        low.world.dice = Pcg32::new(0, 0);
        low.world.gain_pc_exp(ALPHA, 28_000);
        assert_eq!(low.points(ALPHA).level(), 11);
        let body = low.world.bodies.get(&ALPHA).expect("Alpha has a body");
        assert_eq!(body.card.pk_mode, PK_MODE_PROTECT);
    }

    /// B12 for `dead_event`: in place with half the hit points, the window closed, the revive.
    #[test]
    fn the_automatic_restart_with_no_served_town_restarts_in_place_with_half_hp() {
        let mut field = a_field(false);
        let _alpha = field.join(ALPHA, HERE);
        field.killed(ALPHA);
        field.quiet();

        field.pulse(P0 + 4_500);
        let half = i64::from(field.points(ALPHA).max_hp() / 2);
        let after = [
            change(point::POINT_HP, half, half),
            close(&field.world),
            update(&field.world),
            revive(),
        ];
        let first = [phase(PHASE_GAME), remove_record(ALPHA.raw())];
        reinserted(&field.heard(ALPHA), &first, &after);
        assert!(field.ordered(ALPHA).is_empty());
    }

    /// A poisoned player ticks: `maxHP * 50 / 1000` through the poison path, ten ticks
    /// three seconds apart, and the HP floor of 1 it never kills past.
    #[test]
    fn the_poison_event_ticks_ten_times_and_never_kills() {
        use protocol::gc_nested::GcDamageInfo;
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let max = field.points(ALPHA).max_hp();
        field.set_hp_sp(ALPHA, max, 100);
        field.quiet();
        field.world.poison_pc(ALPHA);
        let tick = max * 50 / 1000;
        let mut pulse = P0 + 1;
        for left in (1..=10).rev() {
            field.pulse(pulse);
            let heard = field.heard(ALPHA);
            let infos: Vec<(u8, i32)> = heard
                .iter()
                .filter_map(|record| GcDamageInfo::decode(record).ok())
                .map(|info| (info.flag, info.damage))
                .collect();
            assert_eq!(infos, [(DAMAGE_POISON, tick)], "tick with {left} left");
            assert!(!field.world.is_dead(ALPHA.raw()));
            pulse += 75;
        }
        assert!(field
            .world
            .pc_combat
            .get(&ALPHA.raw())
            .is_some_and(|combat| combat.poison_due.is_none()));
        // The floor: one hit point left, a full tick deals nothing and still lives.
        field.set_hp_sp(ALPHA, 1, 100);
        field.world.poison_pc(ALPHA);
        field.pulse(field.world.last_pulse + 1);
        let heard = field.heard(ALPHA);
        let infos: Vec<(u8, i32)> = heard
            .iter()
            .filter_map(|record| GcDamageInfo::decode(record).ok())
            .map(|info| (info.flag, info.damage))
            .collect();
        assert_eq!(infos, [(DAMAGE_POISON, 0)]);
        assert_eq!(field.points(ALPHA).hp(), 1);
        assert!(!field.world.is_dead(ALPHA.raw()));
    }

    /// Poison blocks the heal: a poisoned player's recovery sends no point changes.
    #[test]
    fn a_poisoned_player_heals_nothing() {
        use protocol::gc_inventory::HEADER_GC_PLAYER_POINT_CHANGE;
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let max = field.points(ALPHA).max_hp();
        field.set_hp_sp(ALPHA, max - 50, 50);
        field.quiet();
        field.world.poison_pc(ALPHA);
        field.quiet();
        field.world.pc_recovery_event(ALPHA.raw());
        let heard = field.heard(ALPHA);
        assert!(
            heard
                .iter()
                .all(|record| record.first() != Some(&HEADER_GC_PLAYER_POINT_CHANGE.value())),
            "no heal while poisoned: {heard:?}"
        );
    }

    /// Death clears the poison, stun and slow with their removes and the +30, keeps the
    /// revive, and ends the poison event.
    #[test]
    fn death_clears_the_hit_affects_and_ends_the_poison() {
        use crate::loading_phase::{
            AFFECT_POISON, AFFECT_SLOW, AFFECT_STUN, AFF_POISON, AFF_SLOW, AFF_STUN, POISON_TICKS,
            SLOW_MOV_SPEED, SLOW_TICKS, STUN_TICKS,
        };
        use protocol::gc_inventory::HEADER_GC_AFFECT_REMOVE;
        use protocol::gc_vid::GcHeaderAndDwordAndByte;
        let mut field = a_field(true);
        let _alpha = field.join(ALPHA, HERE);
        let before = field.world.move_speed_of(ALPHA.raw());
        field.world.add_pc_affect(
            ALPHA,
            Affect {
                kind: AFFECT_POISON,
                apply_on: 0,
                value: 0,
                flag: AFF_POISON,
                remaining: POISON_TICKS,
                sp_cost: 0,
            },
            true,
        );
        field.world.add_pc_affect(
            ALPHA,
            Affect {
                kind: AFFECT_STUN,
                apply_on: 0,
                value: 0,
                flag: AFF_STUN,
                remaining: STUN_TICKS,
                sp_cost: 0,
            },
            true,
        );
        field.world.add_pc_affect(
            ALPHA,
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
        assert_eq!(field.world.move_speed_of(ALPHA.raw()), (before - 30).max(0));
        field.killed(ALPHA);
        let heard = field.heard(ALPHA);
        let removes: Vec<(u32, u8)> = heard
            .iter()
            .filter(|record| record.first() == Some(&HEADER_GC_AFFECT_REMOVE.value()))
            .map(|record| {
                let frame = GcHeaderAndDwordAndByte::decode(record).unwrap();
                (frame.value, frame.flag)
            })
            .collect();
        assert!(removes.contains(&(AFFECT_POISON, 0)));
        assert!(removes.contains(&(AFFECT_STUN, 0)));
        assert!(removes.contains(&(AFFECT_SLOW, u8::try_from(point::POINT_MOV_SPEED).unwrap())));
        assert_eq!(field.world.move_speed_of(ALPHA.raw()), before);
        assert_eq!(
            field.world.affect_flags_of(ALPHA),
            [0, 0],
            "no flags stay: no revive was running"
        );
        assert!(field
            .world
            .pc_combat
            .get(&ALPHA.raw())
            .is_some_and(|combat| combat.poison_due.is_none()));
    }
}
