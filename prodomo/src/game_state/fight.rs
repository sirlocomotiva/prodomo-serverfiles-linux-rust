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
//! - **The fight's other halves (V17b-h, V19).** A hit melee monster runs `BeginFight`
//!   after the claim ([`super::mob_battle`]), and the aggro steers its fight (229r); a RANGE or
//!   MAGIC monster's fight waits for later work. `Dead` pays no reward; skills and the shoot
//!   and fly frames are consumed; the attacker's victim is not kept; a dead monster with a
//!   living reviver in its party stands back up, and a hit or stunned member tells its party
//!   to begin ([`super::mob_party`]). `NormalAttackAffect` lands on the victim (229z) and
//!   the gold steal pays the purse (230a).
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

use common::cfloat::{f32_to_i32, i32_to_f32};
use common::enums::EWearPositions;
use common::point_slot as point;
use common::vid::Vid;
use gamedata::mob_proto::{AIFLAG_AGGRESSIVE, AIFLAG_NOMOVE, CHAR_TYPE_MONSTER, CHAR_TYPE_STONE};
use protocol::cg_attack::CgAttack;
use protocol::gc_actors::GcCharacterGoldChange;
use protocol::gc_fields::GcCreateFly;
use protocol::gc_inventory::{HEADER_GC_DEAD, HEADER_GC_STUN};
use protocol::gc_nested::GcDamageInfo;
use protocol::gc_vid::{GcHeaderAndDword, GcSpecialEffect};
use world::character::{
    damage_special, melee_reach, strike_npc, Equipment, HitEvent, NpcAttackAffect, Strike, Striker,
    Struck, DAMAGE_NORMAL, DAMAGE_POISON, GOLD_MAX_MAX,
};
use world::npc::Npc;

use super::affect::Affect;
use super::motion::rotation_to_xy;
use super::regen::{pulses, Due};
use super::view::EntityKey;
use super::{GameState, MarkUploadAnswer};
use crate::game_loop_messages::{AttackAnswer, GameCommand, MARK_PIXELS};
use crate::loading_phase::{
    point_changes, AFFECT_POISON, AFFECT_SLOW, AFFECT_STUN, AFF_POISON, AFF_SLOW, AFF_STUN,
    POISON_DAMAGE_RATE, POISON_TICKS, SLOW_MOV_SPEED, SLOW_TICKS, STUN_TICKS_NPC,
};
use crate::sync_position::{
    distance_approx, judge_sync_ownership, ownership_record, SyncOwnershipOutcome,
    SyncOwnershipState, SyncPositionVictim, SyncPositionVictimKind,
};

/// `DAMAGE_BLOCK` (`G/char.h:134`): the flag `SetSyncOwner` shows an owner it refuses.
pub(super) const DAMAGE_BLOCK: u8 = 1 << 3;

/// `PASSES_PER_SEC(3)`: the stun before `Dead` (`G/char_battle.cpp:458`), a player's too.
pub(super) const STUN_SECONDS: u32 = 3;

/// `PASSES_PER_SEC(1)`: a dead monster with no reviver waits this long to go (`:1607`).
const DEAD_SECONDS: u32 = 1;

/// `PASSES_PER_SEC(3)`: the wait when a living reviver stands in its party (`:1601`).
const REVIVE_DEAD_SECONDS: u32 = 3;

/// The race whose hit drops gold (`G/char_battle.cpp:1776-1832`), refused until gold is ported.
/// `pub(super)` for the guard's search, which skips it too.
pub(super) const GOLD_RACE: u32 = 5001;

/// `AFF_STUN` (`G/affect.h:216`) as a word-0 bit: flag 7 is bit 6.
pub(super) const AFF_STUN_WORD: u32 = 1 << 6;

/// `AFF_POISON` (`G/affect.h:214`) as a word-0 bit: flag 5 is bit 4.
const AFF_POISON_WORD: u32 = 1 << 4;

/// `IsFakeHydra` (`G/ShipDefense.cpp:1914-1923`): `HYDRA_LEFT` and `HYDRA_RIGHT`, which
/// `battle_is_attackable` refuses to every player.
const FAKE_HYDRAS: [u32; 2] = [3963, 3964];

/// `m_posLastAttacked` and `m_dwLastAttackedTime`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct LastAttacked {
    /// `m_dwLastAttackedTime`.
    pub(super) ms: u32,
    /// `m_posLastAttacked`.
    pub(super) at: (i32, i32),
}

/// One `m_map_kDamage` row (`TBattleInfo`, `G/char.h:1334-1340`): the damage an attacker
/// VID dealt and its aggro.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct NpcDamage {
    /// `iTotalDamage`: the damage dealt, which the exp split and the owner list read.
    pub(super) total: i32,
    /// `iAggro`: the grudge, which `ChangeVictimByAggro` reads.
    pub(super) aggro: i32,
}
/// The `EDamageType` arms `UpdateAggrPointEx` multiplies (`G/char_battle.cpp:3791-3808`).
/// Only `Normal` reaches them yet (melee hits and the chase tick); the rest are built by
/// the tests, and their callers wait for skills and bows. `DAMAGE_TYPE_MELEE` has no arm
/// in the switch, so it feeds ×1 with a party share, like `Normal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(super) enum AggroDamage {
    /// `DAMAGE_TYPE_NORMAL` (and every unlisted type): ×1.
    Normal,
    /// `DAMAGE_TYPE_NORMAL_RANGE`: ×1.2.
    NormalRange,
    /// `DAMAGE_TYPE_RANGE`: ×1.5.
    Range,
    /// `DAMAGE_TYPE_MAGIC`: ×1.2.
    Magic,
    /// `DAMAGE_TYPE_SPECIAL`: ×1, and it spreads no party share (the arm's own type).
    Special,
    /// `DAMAGE_TYPE_MELEE`: no switch arm, so ×1, and the party share spreads (the only
    /// kind excluded is `SPECIAL`, `:3818-3820`).
    Melee,
}

/// What legacy's `CHARACTER` keeps of a non-player's fight beyond its [`Npc`]. The default
/// is a lazy `created`: the same -100 aggro and an unstamped victim-set time, with the
/// last-attacked point at the origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NpcCombat {
    /// `m_dwKillerPID`: the player whose hit stunned it, which `Dead` reads and resets.
    killer_pid: u32,
    /// `INSTANT_FLAG_NO_REWARD`, which a guard's swing sets before the stun: `Dead`
    /// skips the whole `Reward` past it (`G/char_battle.cpp:1475`, `:2221`).
    pub(super) no_reward: bool,
    /// `CMobInstance::m_IsRevive` (`G/mob_manager.cpp:40`): set by the stand-up, read by the
    /// dead wait and `dead_event`. False at creation.
    pub(super) revived: bool,
    /// `GetMaxSP` of a stone: `__StateIdle_Stone`'s ladder floor (`G/char_state.cpp:404-494`).
    /// 0 at creation, as `ComputePoints` sets no SP for a non-player. Unread on monsters.
    pub(super) max_sp: i32,
    /// `m_map_kDamage` (`G/char.h:1441`): each attacker VID's damage and aggro, in VID order.
    pub(super) damage: std::collections::BTreeMap<u32, NpcDamage>,
    /// `m_iMaxAggro` (`G/char.cpp:362`): -100 at creation.
    pub(super) max_aggro: i32,
    /// `m_dwLastVictimSetTime`; `None` is legacy's creation - 3000, which always passes
    /// the 3 s switch window. Stamped by `begin_fight` and `set_victim(Some)` through
    /// `SetVictim` (`G/char_battle.cpp:3427`); the NULL arm leaves it.
    pub(super) last_victim_set_ms: Option<u32>,
    /// `m_posLastAttacked` and `m_dwLastAttackedTime`.
    pub(super) last_attacked: LastAttacked,
    /// `m_dwLastAttackTime`; `None` is legacy's creation - 20000, which always passes the
    /// first test (`CalculateDuration(.., 2000)` is at most 4000).
    pub(super) last_attack_ms: Option<u32>,
    /// `m_dwLastChangeAttackPositionTime`; `None` is creation - 10000, which always passes
    /// (the wait is at most 10000).
    pub(super) last_change_position_ms: Option<u32>,
    /// `m_pkChrSyncOwner` and `m_fSyncTime`.
    pub(super) sync: SyncOwnershipState,
    /// `m_dwLastSyncTime`, which a new owner resets.
    pub(super) last_sync: Option<Duration>,
    /// The Pulse `m_pkStunEvent` is due on.
    /// The Pulse `m_pkStunEvent` is due on.
    stun_due: Option<u64>,
    /// The Pulse `m_pkDeadEvent` is due on.
    dead_due: Option<u64>,
    /// The Pulse `m_pkRecoveryEvent` is due on.
    recovery_due: Option<u64>,
    /// `m_pkPoisonEvent`: the Pulse the poison tick is due on and the ticks it has left.
    pub(super) poison_due: Option<(u64, u32)>,
    /// `m_bHasPoisoned`: a monster is poisoned once per life (`G/char_resist.cpp:183`).
    pub(super) poisoned_once: bool,
    /// The `poison_event`'s attacker PID: the player's VID, which `FindByPID` maps while
    /// the player is online (`G/char_resist.cpp:44`). `None` while no event runs.
    pub(super) poison_attacker: Option<u32>,
    /// `m_pkFireEvent`: the Pulse the fire tick is due on, the ticks it has left, and the
    /// per-tick amount (`G/char_resist.cpp:95-128`). `None` while no event runs.
    pub(super) fire_due: Option<(u64, u32, i32)>,
    /// The `fire_event`'s attacker PID (`G/char_resist.cpp:173`). `None` while no event runs.
    pub(super) fire_attacker: Option<u32>,
    /// `m_adwMobSkillCooltime[i]` (`G/char.cpp:384`): the ms a slot's skill may next
    /// cast at; 0 at creation, so the first Battle update may cast.
    pub(super) skill_cool_ms: [u32; 5],
    /// The live `mob_skill_hit_event`s by their map key (the splash vec index, NOT
    /// the skill slot): the due Pulse and the captured skill vnum, level and point.
    pub(super) skill_events: std::collections::BTreeMap<u32, SkillEvent>,
    /// `m_iChainLightingIndex` (`G/char.h:1556`): 0 at creation; the splash functor reads
    /// it per victim and increments it, and only the CHAIN skill resets it (unhosted, so
    /// no reset is ported).
    pub(super) chain: u8,
}

/// A live `mob_skill_hit_event` (`G/char_skill.cpp:3389-3412`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SkillEvent {
    /// The Pulse it fires on.
    pub(super) due: u64,
    /// The cast skill's vnum.
    pub(super) vnum: u32,
    /// The slot's level at the cast.
    pub(super) level: u8,
    /// The captured splash point: `self + GetDeltaByDegree(skill-face, dist)`.
    pub(super) at: (i32, i32),
}

impl NpcCombat {
    /// A fight as `Show` leaves it: the last-attacked point is the standing point, its time
    /// is now, and both timers are long ago (`G/mob_manager.cpp:39-46`, `G/char.cpp:1877-1882`).
    pub(super) fn created(now: u32, at: (i32, i32)) -> Self {
        Self {
            killer_pid: 0,
            no_reward: false,
            revived: false,
            max_sp: 0,
            damage: std::collections::BTreeMap::new(),
            max_aggro: -100,
            last_victim_set_ms: None,
            last_attacked: LastAttacked { ms: now, at },
            last_attack_ms: None,
            last_change_position_ms: None,
            sync: SyncOwnershipState {
                owner: None,
                claimed_at: None,
            },
            last_sync: None,
            stun_due: None,
            dead_due: None,
            recovery_due: None,
            poison_due: None,
            poisoned_once: false,
            poison_attacker: None,
            fire_due: None,
            fire_attacker: None,
            skill_cool_ms: [0; 5],
            skill_events: std::collections::BTreeMap::new(),
            chain: 0,
        }
    }
}

impl Default for NpcCombat {
    /// `created` at time 0 at the origin: -100 aggro, an unstamped victim-set time.
    fn default() -> Self {
        Self::created(0, (0, 0))
    }
}
/// Whether a player's hit reaches `npc`: a monster or a stone, but not race 5001 (V17a
/// narrows: a stone is hit, and only the empire arm refuses it).
pub(super) fn is_struck(npc: &Npc) -> bool {
    (npc.char_type == CHAR_TYPE_MONSTER || npc.char_type == CHAR_TYPE_STONE)
        && npc.vnum != GOLD_RACE
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
    /// Runs one `Target`, `UseSkill` or `Attack` command: a target's `settled` is dropped
    /// once its records are queued; a cast's `settled` once the cast has run; an attack
    /// answers the gold it stole.
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
            command @ (GameCommand::PartyInvite { .. }
            | GameCommand::PartyInviteAnswer { .. }
            | GameCommand::PartyRemove { .. }
            | GameCommand::PartySetState { .. }
            | GameCommand::PartyParameter { .. }
            | GameCommand::PartyUseSkill { .. }
            | GameCommand::PartyOf { .. }) => self.apply_party_pick(command),
            GameCommand::GuildOf { vid, reply } => {
                let _heard = reply.send(self.holds_guild(vid));
            }
            GameCommand::MarkUpload {
                guild_id,
                image,
                min_level,
                reply,
            } => {
                self.apply_mark_upload(guild_id, &image, min_level, reply);
            }
            GameCommand::MarkIdxList { reply } => {
                self.apply_mark_idx_list(reply);
            }
            GameCommand::MarkCrcList {
                img_idx,
                crc_list,
                reply,
            } => {
                self.apply_mark_crc_list(img_idx, crc_list.as_ref(), reply);
            }
            GameCommand::SymbolUpload {
                guild_id,
                data,
                reply,
            } => {
                self.apply_symbol_upload(guild_id, &data, reply);
            }
            GameCommand::SymbolCrc {
                guild_id,
                crc,
                size,
                reply,
            } => {
                self.apply_symbol_crc(guild_id, crc, size, reply);
            }
            GameCommand::UseSkill {
                vid,
                record,
                settled,
            } => {
                self.use_skill(vid, record.skill_vnum, record.target_vid);
                drop(settled);
            }
            GameCommand::Attack { vid, record, reply } => {
                let answer = self.attack(vid, record);
                let _heard = reply.send(answer);
            }
            GameCommand::FlyTarget {
                vid,
                record,
                settled,
            } => {
                self.fly_target(
                    vid,
                    record.header == protocol::cg_fly_target::CgFlyTargetHeader::FlyTarget,
                    record.dw_target_vid,
                    record.x,
                    record.y,
                );
                drop(settled);
            }
            GameCommand::Shoot { vid, record, reply } => {
                let answer = self.shoot(vid, record.shoot_type);
                let _heard = reply.send(answer);
            }
            _ => debug_assert!(
                false,
                "only target, skill, attack, fly, shoot, party, guild-read and mark commands reach apply_pick"
            ),
        }
    }

    /// Runs one party command: each handler's `settled` is dropped once its records
    /// are queued.
    fn apply_party_pick(&mut self, command: GameCommand) {
        match command {
            GameCommand::PartyInvite {
                vid,
                guest,
                settled,
            } => {
                self.party_invite(vid, common::vid::Vid::new(guest));
                drop(settled);
            }
            GameCommand::PartyInviteAnswer {
                vid,
                inviter,
                accept,
                settled,
            } => {
                self.party_invite_answer(vid, common::vid::Vid::new(inviter), accept);
                drop(settled);
            }
            GameCommand::PartyRemove { vid, pid, settled } => {
                self.party_remove(vid, pid);
                drop(settled);
            }
            GameCommand::PartySetState {
                vid,
                pid,
                role,
                flag,
                settled,
            } => {
                self.party_set_state(vid, pid, role, flag);
                drop(settled);
            }
            GameCommand::PartyParameter { vid, mode, settled } => {
                self.party_parameter(vid, mode);
                drop(settled);
            }
            GameCommand::PartyUseSkill {
                vid,
                skill,
                target,
                settled,
            } => {
                self.party_use_skill(vid, skill, target);
                drop(settled);
            }
            GameCommand::PartyOf { vid, reply } => {
                let _heard = reply.send(self.holds_party(vid));
            }
            _ => debug_assert!(false, "only party commands reach apply_party_pick"),
        }
    }

    /// Answer one judged `MARK_UPLOAD` (`GuildMarkUpload`, `G/input_login.cpp:1059-1090`).
    fn apply_mark_upload(
        &mut self,
        guild_id: u32,
        image: &[u32; MARK_PIXELS],
        min_level: u8,
        reply: tokio::sync::oneshot::Sender<MarkUploadAnswer>,
    ) {
        let answer = self.judge_mark_upload(guild_id, image, min_level);
        let _heard = reply.send(answer);
    }

    /// Answer one `MARK_IDXLIST` (`GuildMarkIDXList`, `G/input_login.cpp:1092-1114`).
    fn apply_mark_idx_list(&self, reply: tokio::sync::oneshot::Sender<Vec<(u16, u16)>>) {
        let _heard = reply.send(self.marks.idx_pairs());
    }

    /// Answer one `MARK_CRCLIST` (`GuildMarkCRCList`, `G/input_login.cpp:1116-1162`).
    fn apply_mark_crc_list(
        &mut self,
        img_idx: u8,
        crc_list: &[u32],
        reply: tokio::sync::oneshot::Sender<Vec<(u8, Vec<u8>)>>,
    ) {
        let diff = self
            .marks
            .diff_blocks(u32::from(img_idx), crc_list)
            .into_iter()
            .map(|(pos, bytes)| (pos, bytes.to_vec()))
            .collect();
        let _heard = reply.send(diff);
    }

    /// Commit one `GUILD_SYMBOL_UPLOAD` (`G/input_login.cpp:1025-1026`).
    fn apply_symbol_upload(
        &mut self,
        guild_id: u32,
        data: &[u8],
        reply: tokio::sync::oneshot::Sender<bool>,
    ) {
        let _heard = reply.send(self.commit_symbol_upload(guild_id, data));
    }

    /// Answer one `SYMBOL_CRC` (`GuildSymbolCRC`, `G/input_login.cpp:1030-1057`).
    fn apply_symbol_crc(
        &self,
        guild_id: u32,
        crc: u32,
        size: u32,
        reply: tokio::sync::oneshot::Sender<Option<Vec<u8>>>,
    ) {
        let _heard = reply.send(self.symbol_mismatch(guild_id, crc, size));
    }

    /// `CInputMain::Attack` of the player under `vid` (`G/input_main.cpp:1918-2008`) and its
    /// `CHARACTER::Attack` (`G/char_battle.cpp:220-328`): type 0 swings the weapon (the
    /// stun, reach and attackable gates below); a type above 0 runs the skill cast
    /// (`CanUseSkill`, the shoot-list refusal, the hit count, the term gate, and the
    /// MELEE damage arm — see `game_state::pc_skill`). `CanMove` fails on the stun
    /// affect only (no shop is ported); no castle map is hosted, and the speed-hack
    /// and combo gates read keys no configuration sets. A refused attack sends nothing, as
    /// `CInputMain::Attack` ignores `Attack`'s answer (`:2004`), and steals no gold.
    pub(super) fn attack(&mut self, vid: Vid, record: CgAttack) -> AttackAnswer {
        // `!CanMove()` (`:226`): a stunned player refuses its own attack.
        if self.affect_flags_of(vid)[0] & AFF_STUN_WORD != 0 {
            return AttackAnswer::default();
        }
        if record.attack_type != 0 {
            self.skill_attack(vid, record.attack_type, record.target_vid);
            return AttackAnswer::default();
        }
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return AttackAnswer::default();
        };
        let Some(npc) = self
            .npc_on(record.target_vid, place)
            .filter(|npc| is_struck(npc))
        else {
            return AttackAnswer::default();
        };
        // `battle_is_attackable` (`G/battle.cpp:165`), then `battle_melee_attack`'s reach
        // (`:172-210`): beyond it the attack is `BATTLE_NONE` and sends nothing.
        let distance = distance_approx(spot.x.saturating_sub(npc.x), spot.y.saturating_sub(npc.y));
        if !self.is_attackable(vid, (place.1, spot.x, spot.y), npc) || distance > melee_reach(npc) {
            return AttackAnswer::default();
        }
        let victim = SyncPositionVictim {
            vid: Vid::new(npc.vid),
            kind: SyncPositionVictimKind::Monster,
            x: npc.x,
            y: npc.y,
        };
        // `SetPosition(POS_FIGHTING)` and `SetRotationToXY` (`:219-223`).
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.fighting = true;
            body.rotation = rotation_to_xy(
                victim.x.saturating_sub(spot.x),
                victim.y.saturating_sub(spot.y),
            );
        }
        let target = victim.vid.raw();
        let mut stolen = None;
        if let Some(strike) = self.strike(vid, target) {
            stolen = self.steal_gold(vid, strike.stolen_gold);
            self.apply_npc_affect(vid, target, strike.affect);
            self.show_strike(vid, target, strike.events);
            match strike.struck {
                Struck::Landed { damage } => {
                    self.set_last_attacked(target);
                    self.after_damage(vid, target, damage);
                }
                Struck::Finished => {
                    self.set_last_attacked(target);
                    // `BATTLE_DEAD && IsPC()` stands the hitter down (`battle_end`).
                    if let Some(body) = self.bodies.get_mut(&vid) {
                        body.fighting = false;
                    }
                    self.dead_npc(target, Some(vid));
                }
                Struck::Whiffed
                | Struck::Refused
                | Struck::AlreadyDead
                | Struck::StunnedByGuard => {}
            }
        }
        // `iRet != BATTLE_NONE`: `SetSyncOwner(this)`, then the victim's `BeginFight`.
        let now = self.clock.elapsed();
        let _owner = self.set_npc_sync_owner(vid, (spot.x, spot.y), &victim, now);
        if self
            .npc_by_vid(target)
            .is_some_and(super::mob_battle::fights_back)
            && self.can_begin_fight(target)
        {
            self.begin_fight(target, vid);
        }
        // `BATTLE_DAMAGE` or `BATTLE_DEAD`: `OnMove(true)` (`G/char.cpp:6148-6179`). The victim's
        // `OnMove` holds nothing for a non-player, and no Eunhyung or mining is ported.
        let now = self.clock.now();
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.last_move = now;
            body.last_attack = Some(now);
        }
        self.remove_revive_invisible(vid);
        AttackAnswer { gold: stolen }
    }

    /// The non-player `vid` names on `place`, a Channel and map.
    pub(super) fn npc_on(&self, vid: u32, place: (u8, i32)) -> Option<&Npc> {
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
    pub(super) fn is_attackable(
        &self,
        attacker: Vid,
        (map, x, y): (i32, i32, i32),
        npc: &Npc,
    ) -> bool {
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
        let user_rate = self.user_damage_rate();
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
            user_rate,
        };
        // `battle_is_attackable` refused a victim on an `ATTR_BANPK` cell, and a standing
        // non-player has a sectree, so `Damage` never refuses it here.
        Some(strike_npc(&mut self.dice, &mut striker, npc, false))
    }

    /// `ChangeGold` for the `amount` the hit stole (`G/char.cpp:3823-3850`): nothing when
    /// the steal drew nothing; at `GOLD_MAX_MAX` the purse refuses with nothing sent, as
    /// the `OVERFLOW_GOLD` arm returns without changing or sending; else the purse gains
    /// the amount and the attacker hears `GC_CHARACTER_GOLD_CHANGE`. Returns the signed
    /// delta the store applies, or `None` when nothing moved. Runs before the damage
    /// packet and the hit-point change, inside `Damage` (`:2080-2092`); `SendMoneyLog`
    /// is log-only. Ahead of `show_strike` here rather than mid-pipeline: the gold
    /// record lands before the affect updates where legacy's steal lands after them.
    fn steal_gold(&mut self, vid: Vid, amount: i32) -> Option<i64> {
        if amount <= 0 {
            return None;
        }
        let held = self.characters.find_by_vid(vid).ok()?.gold();
        let paid = held
            .checked_add(u64::try_from(amount).unwrap_or(u64::MAX))
            .filter(|paid| *paid < GOLD_MAX_MAX)?;
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            character.set_gold(paid);
        }
        let mut record = Vec::new();
        GcCharacterGoldChange::new(vid.raw(), i64::from(amount), paid).encode_into(&mut record);
        let _sent = self.write_to_client(vid, record);
        Some(crate::item_move::gold_delta(held, paid))
    }

    /// The reflect arm's `DAMAGE_TYPE_SPECIAL` hit on the monster under `mob`
    /// (`G/char_battle.cpp:1987`, `:2179-2433`): the player's `POINT_REFLECT_MELEE`
    /// share `damage` of the pre-bonus hit runs the SPECIAL tail (no chance or bonus
    /// arm), shows through the player-hit's pipeline, fills the damage map with the
    /// SPECIAL kind (no party share), and kills rewarded — the killer is the player.
    /// `Finished` dies at once by `Dead(player)` (`:2248-2252`), as a hit on a stunned
    /// monster does; `AlreadyDead` takes nothing. No claim, no `BeginFight`: the reflect
    /// is inside `Damage`, not an `Attack`, and the monster already fights the player.
    pub(super) fn reflect_on_npc(&mut self, victim: Vid, mob: u32, damage: i32) {
        let Some(&(channel, map, at)) = self.npc_of.get(&mob) else {
            return;
        };
        let (channel, map, at) = (channel, map, at);
        let Some(empire) = self.bodies.get(&victim).map(|body| body.card.empire) else {
            return;
        };
        let user_rate = self.user_damage_rate();
        let Ok(character) = self.characters.find_by_vid_mut(victim) else {
            return;
        };
        let (items, points) = character.items_and_points_mut();
        let Some(points) = points else {
            return;
        };
        let weapon = Equipment::of(items, &self.protos)
            .wear(EWearPositions::Weapon as u16)
            .map(|worn| worn.proto);
        let Some(list) = self.npcs.get_mut(&(channel, map)) else {
            return;
        };
        let Some(npc) = Arc::make_mut(list).npcs.get_mut(at) else {
            return;
        };
        let mut striker = Striker {
            points,
            weapon,
            empire,
            map,
            user_rate,
        };
        let (struck, events) = damage_special(&mut self.dice, &mut striker, npc, damage);
        self.show_strike(victim, mob, events);
        match struck {
            Struck::Landed { damage } => {
                self.set_last_attacked(mob);
                self.after_damage_typed(victim, mob, damage, AggroDamage::Special);
            }
            Struck::Finished => {
                self.set_last_attacked(mob);
                self.dead_npc(mob, Some(victim));
            }
            Struck::Whiffed | Struck::Refused | Struck::AlreadyDead | Struck::StunnedByGuard => {}
        }
    }

    /// Sends what a hit on the non-player under `victim` showed, in order.
    pub(super) fn show_strike(&mut self, attacker: Vid, victim: u32, events: Vec<HitEvent>) {
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

    /// What `NormalAttackAffect` landed on the non-player `victim`: the poison, the stun
    /// and the slow, in legacy's arm order (`G/battle.cpp:734-758`). Runs before the
    /// damage packet and the hit-point change, as `battle_hit` runs the affects before
    /// the resist arm and `Damage` — so a victim stunned by this very hit still takes
    /// the hit. A stone takes them too (its Idle guards nothing, ported as found).
    pub(super) fn apply_npc_affect(&mut self, attacker: Vid, victim: u32, landed: NpcAttackAffect) {
        if landed.poisoned {
            self.poison_npc(attacker, victim);
        }
        if landed.stunned {
            self.add_npc_affect(
                victim,
                Affect {
                    kind: AFFECT_STUN,
                    apply_on: u8::try_from(point::POINT_NONE).unwrap_or(u8::MAX),
                    value: 0,
                    flag: AFF_STUN,
                    remaining: STUN_TICKS_NPC,
                    sp_cost: 0,
                },
            );
        }
        if landed.slowed {
            self.add_npc_affect(
                victim,
                Affect {
                    kind: AFFECT_SLOW,
                    apply_on: u8::try_from(point::POINT_MOV_SPEED).unwrap_or(u8::MAX),
                    value: SLOW_MOV_SPEED,
                    flag: AFF_SLOW,
                    remaining: SLOW_TICKS,
                    sp_cost: 0,
                },
            );
        }
    }

    /// `AttackedByPoison` on the non-player under `victim` (`G/char_resist.cpp:178-220`),
    /// past the draws the strike already passed: nothing while a poison event runs or the
    /// victim has ever been poisoned — a monster is poisoned once per life — else the
    /// flag is set, the poison affect lands, and the poison event with 10 ticks is due
    /// 1 Pulse on. The commented immunity arm never runs. The attacker PID is the
    /// player's VID, which `FindByPID` maps while the player is online.
    pub(super) fn poison_npc(&mut self, attacker: Vid, victim: u32) {
        if self
            .npc_combat
            .get(&victim)
            .is_some_and(|combat| combat.poison_due.is_some() || combat.poisoned_once)
        {
            return;
        }
        self.add_npc_affect(
            victim,
            Affect {
                kind: AFFECT_POISON,
                apply_on: u8::try_from(point::POINT_NONE).unwrap_or(u8::MAX),
                value: 0,
                flag: AFF_POISON,
                remaining: POISON_TICKS,
                sp_cost: 0,
            },
        );
        let when = self.last_pulse.saturating_add(1);
        let combat = self.npc_combat.entry(victim).or_default();
        combat.poisoned_once = true;
        combat.poison_attacker = Some(attacker.raw());
        let old = combat
            .poison_due
            .replace((when, super::mob_strike::POISON_COUNT));
        if let Some((at, count)) = old {
            self.regens
                .due
                .remove(&(at, Due::Poison { vid: victim, count }));
        }
        self.regens.due.insert((
            when,
            Due::Poison {
                vid: victim,
                count: super::mob_strike::POISON_COUNT,
            },
        ));
    }

    /// `GetPoisonDamageRate` of the non-player under `victim` (`G/char_resist.cpp:11-20`):
    /// the rank table's per-mille less `POINT_POISON_REDUCE`, floored at 0. A rank at or
    /// past the table's end reads the last per-mille: legacy would read out of bounds
    /// (a Defect), and there is no game rank past KING.
    fn npc_poison_rate(&self, victim: u32) -> i32 {
        let Some(npc) = self.npc_by_vid(victim) else {
            return 0;
        };
        let rank = usize::from(npc.fight.rank).min(POISON_DAMAGE_RATE.len() - 1);
        POISON_DAMAGE_RATE[rank]
            .wrapping_sub(npc.fight.point(point::POINT_POISON_REDUCE))
            .max(0)
    }

    /// Cancels the poison event the slot of the non-player under `vid` keeps, if any.
    fn cancel_npc_poison(&mut self, vid: u32) {
        let Some(combat) = self.npc_combat.get_mut(&vid) else {
            return;
        };
        let Some((when, count)) = combat.poison_due.take() else {
            return;
        };
        combat.poison_attacker = None;
        self.regens.due.remove(&(when, Due::Poison { vid, count }));
    }

    /// `poison_event` of the non-player under `vid` (`G/char_resist.cpp:42-76`): the
    /// victim's own slot must still hold this tick, else a re-poison cancelled it. A
    /// gone, dead or stunned victim ends the event (`Damage` true). Else the tick deals
    /// `maxHP * rate / 1000` through the poison floor — a tick that would leave no hit
    /// points deals all but one, so poison never kills and the event never stuns — with
    /// the target broadcast and the `DAMAGE_POISON` damage packet. Past 0 damage and an
    /// online attacker the damage map row fills, the recovery starts and `Ex` feeds
    /// NORMAL aggro (`:2416-2430`'s non-null arm). Then the count falls; while any is
    /// left the next tick is 3 seconds on, else the event ends.
    pub(super) fn npc_poison_event(&mut self, vid: u32, count: u32) {
        let held = self
            .npc_combat
            .get(&vid)
            .and_then(|combat| combat.poison_due);
        let Some((_, held_count)) = held else {
            return;
        };
        if held_count != count {
            return;
        }
        let Some(npc) = self.npc_by_vid(vid) else {
            self.cancel_npc_poison(vid);
            return;
        };
        if npc.dead || npc.stunned {
            self.cancel_npc_poison(vid);
            return;
        }
        let max_hp = npc.max_hp;
        let hp = npc.hp;
        let mut damage = max_hp
            .saturating_mul(self.npc_poison_rate(vid))
            .wrapping_div(1000);
        // `GetHP() - dam <= 0 → dam = GetHP() - 1` (`G/char_battle.cpp:2256-2261`).
        if hp.saturating_sub(damage) <= 0 {
            damage = hp.saturating_sub(1).max(0);
        }
        if let Some(npc) = self.npc_mut(vid) {
            npc.hp = npc.hp.saturating_sub(damage);
        }
        self.broadcast_npc_target(vid);
        let attacker = self
            .npc_combat
            .get(&vid)
            .and_then(|combat| combat.poison_attacker)
            .filter(|attacker| self.is_online(Vid::new(*attacker)));
        if let Some(attacker) = attacker {
            self.send_damage_packet(vid, Vid::new(attacker), damage, DAMAGE_POISON);
        }
        if damage > 0 {
            if let Some(attacker) = attacker {
                self.after_damage(Vid::new(attacker), vid, damage);
            }
        }
        if count <= 1 {
            self.cancel_npc_poison(vid);
            return;
        }
        let next = count.saturating_sub(1);
        let when = self
            .last_pulse
            .saturating_add(pulses(super::death::POISON_TICK_SECONDS));
        let old = self
            .npc_combat
            .get_mut(&vid)
            .and_then(|combat| combat.poison_due.replace((when, next)));
        if let Some((at, _)) = old {
            self.regens.due.remove(&(at, Due::Poison { vid, count }));
        }
        self.regens
            .due
            .insert((when, Due::Poison { vid, count: next }));
    }

    /// Cancels the fire event the slot of the non-player under `vid` keeps, if any.
    fn cancel_npc_fire(&mut self, vid: u32) {
        let Some(combat) = self.npc_combat.get_mut(&vid) else {
            return;
        };
        let Some((when, count, amount)) = combat.fire_due.take() else {
            return;
        };
        combat.fire_attacker = None;
        self.regens
            .due
            .remove(&(when, Due::Fire { vid, count, amount }));
    }

    /// `fire_event` of the non-player under `vid` (`G/char_resist.cpp:95-128`): the
    /// victim's own slot must still hold this tick. A gone, dead or stunned victim
    /// ends the event (`Damage` true). Else the tick deals the stored amount with
    /// the target broadcast and the `DAMAGE_NORMAL` damage packet; past an online
    /// attacker the damage map row fills, the recovery starts and `Ex` feeds NORMAL
    /// aggro (`:2416-2430`'s non-null arm — FIRE carries no aggro kind of its own).
    /// A tick that leaves no hit points stuns with the attacker's PID as killer
    /// (`after_damage`'s second arm). Then the count falls; while any is left the
    /// next tick is 3 seconds on, else the event ends.
    pub(super) fn npc_fire_event(&mut self, vid: u32, count: u32, amount: i32) {
        let held = self.npc_combat.get(&vid).and_then(|combat| combat.fire_due);
        let Some((_, held_count, held_amount)) = held else {
            return;
        };
        if held_count != count || held_amount != amount {
            return;
        }
        let Some(npc) = self.npc_by_vid(vid) else {
            self.cancel_npc_fire(vid);
            return;
        };
        if npc.dead || npc.stunned {
            self.cancel_npc_fire(vid);
            return;
        }
        let damage = amount.max(0).min(npc.hp);
        if let Some(npc) = self.npc_mut(vid) {
            npc.hp = npc.hp.saturating_sub(damage);
        }
        self.broadcast_npc_target(vid);
        let attacker = self
            .npc_combat
            .get(&vid)
            .and_then(|combat| combat.fire_attacker)
            .filter(|attacker| self.is_online(Vid::new(*attacker)));
        if let Some(attacker) = attacker {
            self.send_damage_packet(vid, Vid::new(attacker), damage, DAMAGE_NORMAL);
            if damage > 0 {
                self.after_damage(Vid::new(attacker), vid, damage);
            }
        } else if damage > 0 {
            // `Damage` with a null attacker still stuns at 0 (`:2280-2290`): the row
            // and aggro need an attacker, the stun does not.
            if self.npc_by_vid(vid).is_some_and(|npc| npc.hp <= 0) {
                self.stun_npc(vid);
            }
        }
        if count <= 1 {
            self.cancel_npc_fire(vid);
            return;
        }
        let next = count.saturating_sub(1);
        let when = self
            .last_pulse
            .saturating_add(pulses(super::death::POISON_TICK_SECONDS));
        let old = self
            .npc_combat
            .get_mut(&vid)
            .and_then(|combat| combat.fire_due.replace((when, next, amount)));
        if let Some((at, _, _)) = old {
            self.regens
                .due
                .remove(&(at, Due::Fire { vid, count, amount }));
        }
        self.regens.due.insert((
            when,
            Due::Fire {
                vid,
                count: next,
                amount,
            },
        ));
    }

    /// The end of `Damage` once the hit points are taken (`G/char_battle.cpp:2407-2433`): a hit
    /// that took any fills the damage map, starts the recovery and feeds the aggro, and one
    /// that left none stuns the non-player with the attacker's PID, which is a player's VID,
    /// as its killer.
    fn after_damage(&mut self, attacker: Vid, victim: u32, damage: i32) {
        self.after_damage_typed(attacker, victim, damage, AggroDamage::Normal);
    }

    /// [`GameState::after_damage`], with the aggro kind of the hit: a reflected hit
    /// fills the same row but feeds `SPECIAL` aggro (×1, no party share, `:3818-3820`).
    pub(super) fn after_damage_typed(
        &mut self,
        attacker: Vid,
        victim: u32,
        damage: i32,
        kind: AggroDamage,
    ) {
        if damage > 0 {
            // `m_map_kDamage` (`:2407-2423`): a new row holds `(dam, 0)`, else the total
            // grows; then the recovery, then `UpdateAggrPointEx`.
            self.npc_combat
                .entry(victim)
                .or_default()
                .damage
                .entry(attacker.raw())
                .and_modify(|row| row.total = row.total.saturating_add(damage))
                .or_insert(NpcDamage {
                    total: damage,
                    aggro: 0,
                });
            self.start_recovery(victim);
            self.update_aggr_point_ex(victim, attacker, kind, damage);
        }
        if self.npc_by_vid(victim).is_some_and(|npc| npc.hp <= 0) {
            self.stun_npc(victim);
            self.npc_combat.entry(victim).or_default().killer_pid = attacker.raw();
        }
    }

    /// `(int)(dam * f)` (`G/char_battle.cpp:3795-3805`): to `f32` as C converts, the
    /// product in `f32`, then the fraction dropped toward zero. Damage stays far below
    /// 2^24, so both steps are exact and the saturation of huge values is unreachable.
    fn aggro_scaled(dam: i32, factor: f32) -> i32 {
        f32_to_i32(i32_to_f32(dam) * factor)
    }

    /// `UpdateAggrPointEx` (`G/char_battle.cpp:3789-3836`) of the monster under `mob` for
    /// `attacker`'s `dam` of `kind`: the type multiplier, the victim's ×1.2, the add
    /// floored at 0, the party's share past `PM_AGGRO_INCREASE`, then
    /// `ChangeVictimByAggro`.
    fn update_aggr_point_ex(&mut self, mob: u32, attacker: Vid, kind: AggroDamage, dam: i32) {
        let mut dam = match kind {
            AggroDamage::Normal | AggroDamage::Special | AggroDamage::Melee => dam,
            AggroDamage::NormalRange | AggroDamage::Magic => Self::aggro_scaled(dam, 1.2),
            AggroDamage::Range => Self::aggro_scaled(dam, 1.5),
        };
        if self.victim_of(mob) == Some(attacker) {
            dam = Self::aggro_scaled(dam, 1.2);
        }
        let aggro = self
            .npc_combat
            .entry(mob)
            .or_default()
            .damage
            .entry(attacker.raw())
            .or_default();
        aggro.aggro = aggro.aggro.saturating_add(dam).max(0);
        let aggro = aggro.aggro;
        // The party's share (`:3820-3831`): past 0, never `SPECIAL`, a half for the leader
        // and a third for a member.
        if dam > 0 && !matches!(kind, AggroDamage::Special) {
            let party = self.party_of(mob);
            if !party.is_empty() {
                let share = if self.party_leader(mob) == Some(mob) {
                    dam / 2
                } else {
                    dam / 3
                };
                self.party_aggro_increase(mob, &party, attacker, share);
            }
        }
        self.change_victim_by_aggro(mob, aggro, attacker);
    }

    /// `UpdateAggrPoint` (`G/char_battle.cpp:3838-3852`): dead or stunned returns; a missing
    /// row inserts `(0, dam)` — the damage in the aggro — then `UpdateAggrPointEx`.
    pub(super) fn update_aggr_point(
        &mut self,
        mob: u32,
        attacker: Vid,
        kind: AggroDamage,
        dam: i32,
    ) {
        if self
            .npc_by_vid(mob)
            .is_none_or(|npc| npc.dead || npc.stunned)
        {
            return;
        }
        self.npc_combat
            .entry(mob)
            .or_default()
            .damage
            .entry(attacker.raw())
            .or_insert(NpcDamage {
                total: 0,
                aggro: dam,
            });
        self.update_aggr_point_ex(mob, attacker, kind, dam);
    }

    /// `ChangeVictimByAggro` (`G/char_battle.cpp:3854-3900`): past the 3 s window, the
    /// victim with the most aggro wins. Same victim: raise the max, else scan for a
    /// strictly greater row whose owner is online, alive and within 5000 — last one wins
    /// (ties keep the greater VID, as `std::map` order does). Other victim: switch only
    /// past the max. A switch re-stamps `SetVictim`'s time and asks for the next Pulse.
    fn change_victim_by_aggro(&mut self, mob: u32, aggro: i32, attacker: Vid) {
        let now = self.clock.now();
        if let Some(set) = self
            .npc_combat
            .get(&mob)
            .and_then(|combat| combat.last_victim_set_ms)
        {
            if now.wrapping_sub(set) < 3000 {
                return;
            }
        }
        let held = self.victim_of(mob);
        // The aggro steers a fight already begun: with no held victim — stunned, dead,
        // never begun — there is no victim to keep or switch.
        let Some(held) = held else {
            return;
        };
        if held == attacker {
            let max = self.npc_combat.entry(mob).or_default().max_aggro;
            if max < aggro {
                self.npc_combat.entry(mob).or_default().max_aggro = aggro;
                return;
            }
            let found = self.aggro_successor(mob, aggro);
            if let Some((vid, aggro)) = found {
                self.npc_combat.entry(mob).or_default().max_aggro = aggro;
                self.set_victim(mob, Some(vid));
                self.set_duration(mob, 1);
            }
            return;
        }
        let max = self.npc_combat.entry(mob).or_default().max_aggro;
        if max < aggro {
            self.npc_combat.entry(mob).or_default().max_aggro = aggro;
            self.set_victim(mob, Some(attacker));
            self.set_duration(mob, 1);
        }
    }

    /// The same-victim scan (`:3870-3885`): the last row past `aggro` whose owner is
    /// online, alive and within 5000 of the monster under `mob`, in VID order.
    fn aggro_successor(&self, mob: u32, aggro: i32) -> Option<(Vid, i32)> {
        let npc = self.npc_by_vid(mob)?;
        let combat = self.npc_combat.get(&mob)?;
        let mut found: Option<(Vid, i32)> = None;
        let mut best = aggro;
        for (&vid, row) in &combat.damage {
            if row.aggro <= best {
                continue;
            }
            let vid = Vid::new(vid);
            // `Find` reads the online (`characters`), and `IsDead` the dead.
            if self.characters.find_by_vid(vid).is_err() || self.is_dead(vid.raw()) {
                continue;
            }
            let Some(spot) = self.spot_of(vid.raw()) else {
                continue;
            };
            if distance_approx(npc.x.saturating_sub(spot.x), npc.y.saturating_sub(spot.y)) >= 5000 {
                continue;
            }
            best = row.aggro;
            found = Some((vid, row.aggro));
        }
        found
    }

    /// `Stun` of the non-player under `vid` (`G/char_battle.cpp:422-459`):
    /// `PM_ATTACKED_BY` first (`:431-433`), then the recovery ends and the stun shows. A
    /// non-player has no recovery points to zero and no shop to close.
    pub(super) fn stun_npc(&mut self, vid: u32) {
        if self
            .npc_by_vid(vid)
            .is_none_or(|npc| npc.stunned || npc.dead)
        {
            return;
        }
        self.party_attacked_by(vid);
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
    /// duel, arena and siege hooks hold nothing for a monster a player kills; `Reward` runs
    /// between the stun cancel and `GC_DEAD`: exp, then gold, then the item draws, whose
    /// hits wait for D5 (V17c narrows). The `BOSS_KILL_LOG` and `STACK_ATTR` hooks are
    /// log-only and item-system work (hooks).
    pub(super) fn dead_npc(&mut self, vid: u32, killer: Option<Vid>) {
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
        // `SetPosition(POS_DEAD)`, then `ClearAffect(true)` (`:1370`): the poison, stun
        // and slow go — none is in the no-clear tables — each computed off, with no
        // update packet, before `GC_DEAD`. The poison event's next run would find the
        // victim dead and end, but the slot cancels so it never blocks a re-stood VID.
        if let Some(npc) = self.npc_mut(vid) {
            npc.dead = true;
        }
        self.clear_npc_affects(vid, false);
        self.cancel_npc_poison(vid);
        // `ClearSync` (`:1461`): its own release, then what it owns, then the stun event
        // ends (`:1464`).
        self.release_npc_sync(vid);
        self.release_owned(Vid::new(vid));
        let combat = self.npc_combat.entry(vid).or_default();
        self.regens.cancel(&mut combat.stun_due, Due::Stun(vid));
        // `Reward` (`:1476-1498`): no field war (no guilds) and no resurrection — only vnum
        // 8479 carries one (4305), and neither spawns on a hosted map — so the exp, gold
        // and item halves run, unless a guard's swing set `NO_REWARD` (`:1475`), which
        // skips them all.
        let no_reward = self
            .npc_combat
            .get(&vid)
            .is_some_and(|combat| combat.no_reward);
        if !no_reward {
            if let Some(attacker) = self.reward_exp(vid) {
                self.reward_gold(vid, attacker);
                self.reward_drops(vid, attacker);
            }
        }
        self.packet_around(
            EntityKey::Npc(vid),
            &header_and_vid(HEADER_GC_DEAD.value(), vid),
            None,
        );
        // `ClearStone` (`:1579-1580`): a stone's sons die with it — `Dead(NULL)` each,
        // which pays their damage maps and feeds the stone (`distribute_exp` reads the
        // link while it still stands). A son unlinks via its own `Dead` arm below.
        self.kill_stone_sons(vid);
        if let Some(npc) = self.npc_mut(vid) {
            npc.stunned = false;
        }
        let revived = self
            .npc_combat
            .get(&vid)
            .is_some_and(|combat| combat.revived);
        // `m_pkDeadEvent` (`:1599-1610`): 3 s past a living reviver, 1 s else.
        let wait = if !revived && self.has_reviver_in_party(vid) {
            REVIVE_DEAD_SECONDS
        } else {
            DEAD_SECONDS
        };
        let when = self.last_pulse.saturating_add(pulses(wait));
        let combat = self.npc_combat.entry(vid).or_default();
        self.regens.cancel(&mut combat.dead_due, Due::Dead(vid));
        self.regens
            .schedule(&mut combat.dead_due, when, Due::Dead(vid));
    }

    /// `dead_event` of a monster (`G/char_battle.cpp:473-548`): a monster its party revives
    /// stands back up; else `M2_DESTROY_CHARACTER`.
    pub(super) fn dead_event(&mut self, vid: u32) {
        if let Some(combat) = self.npc_combat.get_mut(&vid) {
            combat.dead_due = None;
        }
        // The stand-up (`:529-541`): a monster only, once, past a living reviver.
        let revived = self
            .npc_combat
            .get(&vid)
            .is_some_and(|combat| combat.revived);
        let monster = self
            .npc_by_vid(vid)
            .is_some_and(|npc| npc.char_type == CHAR_TYPE_MONSTER);
        if monster && !revived && self.has_reviver_in_party(vid) {
            // `SetPosition(POS_STANDING)` clears the dead flag; `SetHP(GetMaxHP())`
            // refills; `SetAggressive` sets the bit (`G/char_state.cpp:242-245`).
            if let Some(npc) = self.npc_mut(vid) {
                npc.dead = false;
                npc.hp = npc.max_hp;
                npc.ai_flags |= AIFLAG_AGGRESSIVE;
            }
            if let Some(combat) = self.npc_combat.get_mut(&vid) {
                combat.revived = true;
            }
            self.reencode_npc(vid);
            return;
        }
        self.destroy_npc(vid);
    }

    /// `ViewReencode` of the NPC under `vid` (`G/entity_view.cpp:23-45`): a same-spot `show`
    /// removes and re-inserts it for itself and its view.
    fn reencode_npc(&mut self, vid: u32) {
        let (Some((channel, map, _)), Some(spot)) =
            (self.npc_of.get(&vid).copied(), self.npc_spot(vid))
        else {
            return;
        };
        let z = self.npc_by_vid(vid).map_or(0, |npc| npc.z);
        let radius = self.view_radius;
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&(channel, map)) {
            let _shown = index.show(
                EntityKey::Npc(vid),
                (spot.0, spot.1, z),
                radius,
                &mut effects,
            );
        }
        self.deliver(&effects, None);
    }

    /// `ClearStone`'s stone arm (`G/char.cpp:6402-6410`) in `Dead`: every live son of the
    /// stone under `vid` dies — `Dead(NULL)` pays its damage map — and unlinks. Nothing
    /// when `vid` is no stone. Sons in map order (V1: legacy iterates a pointer set).
    pub(super) fn kill_stone_sons(&mut self, vid: u32) {
        let is_stone = self
            .npc_by_vid(vid)
            .is_some_and(|npc| npc.char_type == CHAR_TYPE_STONE);
        if !is_stone {
            return;
        }
        let Some(&(channel, map, _)) = self.npc_of.get(&vid) else {
            return;
        };
        let sons: Vec<u32> = self
            .npcs
            .get(&(channel, map))
            .map(|list| {
                list.npcs
                    .iter()
                    .filter(|npc| npc.stone == Some(vid) && !npc.dead)
                    .map(|npc| npc.vid)
                    .collect()
            })
            .unwrap_or_default();
        for son in sons {
            self.dead_npc(son, None);
            if let Some(son) = self.npc_mut(son) {
                son.stone = None;
            }
        }
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

    /// `recovery_event` of a non-player (`G/char.cpp:3187-3240`): a poisoned one heals
    /// nothing and reschedules (`:3187-3194`); else `bRegenPercent` of its maximum, at
    /// least 1 and held to the maximum, shown to its targeters; again `bRegenCycle`
    /// seconds on until it is full. No regen stands race 2493, and no door is hit.
    pub(super) fn recovery_event(&mut self, vid: u32) {
        if let Some(combat) = self.npc_combat.get_mut(&vid) {
            combat.recovery_due = None;
        }
        if self.npc_affect_words(vid)[0] & AFF_POISON_WORD != 0 {
            let cycle = self.npc_by_vid(vid).map_or(0, recovery_pulses);
            let when = self.last_pulse.saturating_add(cycle);
            let combat = self.npc_combat.entry(vid).or_default();
            self.regens
                .schedule(&mut combat.recovery_due, when, Due::Recovery(vid));
            return;
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
    /// What it owns is released by the caller ([`GameState::release_owned`]).
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

    /// What a destroyed non-player's fight leaves: its stun, dead, recovery and poison
    /// events end (`G/char.cpp:713-720`), its affects clear without packets (`:768`), and
    /// nothing of it is kept.
    pub(super) fn forget_npc_fight(&mut self, vid: u32) {
        let Some(mut combat) = self.npc_combat.remove(&vid) else {
            return;
        };
        self.regens.cancel(&mut combat.stun_due, Due::Stun(vid));
        self.regens.cancel(&mut combat.dead_due, Due::Dead(vid));
        self.regens
            .cancel(&mut combat.recovery_due, Due::Recovery(vid));
        if let Some((when, count)) = combat.poison_due.take() {
            self.regens.due.remove(&(when, Due::Poison { vid, count }));
        }
        self.clear_npc_affects(vid, false);
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
        calc_melee_damage, Pcg32, PointRecord, Points, DAMAGE_CRITICAL, DAMAGE_NORMAL, FLY_EXP,
        FLY_HP_SMALL, SE_CRITICAL,
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
        fight.level = 1;
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

    /// `npc` on open cells, entered and pulsed as [`a_fight`], with the monster's experience
    /// set to `exp` and its level to `level`: the spawn path copies both from the proto.
    fn a_reward_fight(
        npc: Npc,
        exp: u32,
        level: u8,
        at: (i32, i32),
    ) -> (TestClock, GameState, UnboundedReceiver<Vec<u8>>) {
        let (clock, mut world, inbox) = a_fight(npc, at);
        let npc = world.npc_mut(ME).expect("the monster stands");
        npc.exp = exp;
        npc.fight.level = level;
        (clock, world, inbox)
    }

    /// The exp orb from [`ME`] to `to`.
    fn fly(to: u32) -> Vec<u8> {
        let mut orb = Vec::new();
        GcCreateFly::new(FLY_EXP, ME, to).encode_into(&mut orb);
        orb
    }

    /// `(int)(dam * f)` truncates toward zero in `f32` (`G/char_battle.cpp:3795-3805`):
    /// the victim's ×1.2 and the RANGE ×1.5, pinned both signs and at zero.
    #[test]
    fn the_aggro_multipliers_truncate_toward_zero_in_f32() {
        // ×1.2: 10 -> 12, 7 -> 8 (8.4 truncates), -7 -> -8, 0 -> 0.
        assert_eq!(GameState::aggro_scaled(10, 1.2), 12);
        assert_eq!(GameState::aggro_scaled(7, 1.2), 8);
        assert_eq!(GameState::aggro_scaled(-7, 1.2), -8);
        assert_eq!(GameState::aggro_scaled(0, 1.2), 0);
        // ×1.5: 7 -> 10 (10.5 truncates), -7 -> -10.
        assert_eq!(GameState::aggro_scaled(7, 1.5), 10);
        assert_eq!(GameState::aggro_scaled(-7, 1.5), -10);
        assert_eq!(GameState::aggro_scaled(0, 1.5), 0);
        // The type arms route: NORMAL_RANGE and MAGIC ×1.2, RANGE ×1.5.
        let kinds = [
            (AggroDamage::NormalRange, 7, 8),
            (AggroDamage::Magic, 7, 8),
            (AggroDamage::Range, 7, 10),
        ];
        for (kind, dam, want) in kinds {
            let scaled = match kind {
                AggroDamage::Normal | AggroDamage::Special | AggroDamage::Melee => dam,
                AggroDamage::NormalRange | AggroDamage::Magic => GameState::aggro_scaled(dam, 1.2),
                AggroDamage::Range => GameState::aggro_scaled(dam, 1.5),
            };
            assert_eq!(scaled, want, "the {kind:?} arm multiplies");
        }
    }

    /// A damage map row of `total` dealt and no aggro.
    fn row(total: i32) -> NpcDamage {
        NpcDamage { total, aggro: 0 }
    }

    /// The exp point change `to`'s client is sent for a gain of `amount` onto `value`.
    fn exp_change(to: u32, amount: i64, value: i64) -> Vec<u8> {
        point_changes(
            &[PointRecord {
                kind: u8::try_from(point::POINT_EXP).unwrap(),
                amount,
                value,
                broadcast: false,
            }],
            to,
        )
        .concat()
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
        let combat = world.npc_combat[&ME].clone();
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
        let combat = world.npc_combat[&ME].clone();
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

    /// A dead monster with a passing reviver waits 3 s, not 1 s (`G/char_battle.cpp:1601`).
    #[test]
    fn a_dead_monster_with_a_reviver_waits_three_seconds() {
        use gamedata::mob_proto::AIFLAG_REVIVE;
        let clock = TestClock::default();
        clock.set(T0);
        let master = a_monster(1);
        let mut follower = a_monster(MAX_HP);
        follower.vid = ME + 1;
        follower.x = SPOT.0 + 100;
        follower.leader = Some(ME);
        follower.ai_flags |= AIFLAG_REVIVE;
        follower.fight.revive_point = 100;
        let mut world = a_field_on(&clock, vec![master, follower], open_cells());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut inbox);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut inbox);
        world.process_pulse(P0 + 75);
        drained(&mut inbox);
        assert!(
            world.npc_by_vid(ME).is_some_and(|npc| npc.dead),
            "the stun ends in death"
        );
        assert_eq!(world.npc_combat[&ME].dead_due, Some(P0 + 150));
    }

    /// `dead_event` with a passing reviver stands the monster up: dead cleared, full hit
    /// points, the aggressive bit, `revived` set, and the remove+insert pair — and the
    /// monster is not destroyed (`G/char_battle.cpp:529-541`).
    #[test]
    fn dead_event_with_a_reviver_stands_the_monster_back_up() {
        use gamedata::mob_proto::{AIFLAG_AGGRESSIVE, AIFLAG_REVIVE};
        let clock = TestClock::default();
        clock.set(T0);
        let mut follower = a_monster(MAX_HP);
        follower.vid = ME + 1;
        follower.x = SPOT.0 + 100;
        follower.leader = Some(ME);
        follower.ai_flags |= AIFLAG_REVIVE;
        follower.fight.revive_point = 100;
        let mut world = a_field_on(&clock, vec![a_monster(1), follower], open_cells());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut inbox);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut inbox);
        world.process_pulse(P0 + 75);
        drained(&mut inbox);
        world.process_pulse(P0 + 150);
        let heard = drained(&mut inbox);
        assert_eq!(heard.len(), 2, "the remove and the insert: {heard:02x?}");
        assert_eq!(heard[0], remove_record(ME));
        assert_eq!(
            heard[1][0],
            protocol::gc_inventory::HEADER_GC_CHARACTER_ADD.value()
        );
        let npc = world.npc_by_vid(ME).expect("the monster stands");
        assert_eq!((npc.dead, npc.hp), (false, MAX_HP));
        assert_ne!(npc.ai_flags & AIFLAG_AGGRESSIVE, 0);
        assert!(world.npc_combat[&ME].revived);
    }

    /// A revived monster dies for good: the second `dead_event` finds `revived` set and
    /// destroys it.
    #[test]
    fn a_revived_monster_dies_for_good_the_second_time() {
        use gamedata::mob_proto::AIFLAG_REVIVE;
        let clock = TestClock::default();
        clock.set(T0);
        let mut follower = a_monster(MAX_HP);
        follower.vid = ME + 1;
        follower.x = SPOT.0 + 100;
        follower.leader = Some(ME);
        follower.ai_flags |= AIFLAG_REVIVE;
        follower.fight.revive_point = 100;
        let mut world = a_field_on(&clock, vec![a_monster(1), follower], open_cells());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut inbox);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut inbox);
        world.process_pulse(P0 + 75);
        drained(&mut inbox);
        world.process_pulse(P0 + 150);
        drained(&mut inbox);
        assert!(world.npc_combat[&ME].revived, "the stand-up ran");
        world.npc_mut(ME).expect("the monster stands").hp = 1;
        hit(&mut world, 1, ME);
        drained(&mut inbox);
        world.process_pulse(P0 + 225);
        drained(&mut inbox);
        assert!(
            world.npc_by_vid(ME).is_some_and(|npc| npc.dead),
            "the stun ends in death again"
        );
        // Already revived, so the wait is back to 1 s even past the reviver.
        assert_eq!(world.npc_combat[&ME].dead_due, Some(P0 + 250));
        world.process_pulse(P0 + 250);
        assert!(world.npc_by_vid(ME).is_none(), "the second death destroys");
    }
    /// (`m_map_kDamage`, `G/char_battle.cpp:2407-2423`); a blocked hit writes nothing.
    #[test]
    fn a_hit_fills_the_damage_map_and_a_blocked_hit_writes_nothing() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        world.target(Vid::new(1), ME);
        let damage = expected_damage(&mut world, 1);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        let combat = world.npc_combat[&ME].clone();
        assert_eq!(
            combat.damage.get(&1).map(|row| row.total),
            Some(damage.min(MAX_HP))
        );
        // A blocked hit is damage 0: the map keeps the old total.
        let npc = world.npc_by_vid(ME).unwrap();
        let left = npc.hp;
        assert!(left > 0, "the monster stands");
        let _ = left;
    }

    /// A solo kill of a level-10 monster worth 75 exp: the killer's client is sent the exp
    /// record, the view sees the exp orb, then `GC_DEAD`; the map is cleared and the
    /// monster's experience consumed (`DistributeExp`, `G/char_battle.cpp:2818-2985`).
    #[test]
    fn a_solo_kill_pays_the_whole_exp_with_an_orb_before_gc_dead() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 75, 10, NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        drained(&mut beta);
        hit(&mut world, 1, ME);
        // Level 10 vs 10 takes the whole 75 (delta row 100), capped well above. The sync
        // release runs in `Dead` before `Reward`, so its `owner(0)` leads.
        assert_eq!(
            drained(&mut alpha),
            [
                no_target(),
                owner(0),
                exp_change(1, 75, 75),
                fly(1),
                dead_record()
            ]
        );
        assert_eq!(drained(&mut beta), [owner(0), fly(1), dead_record()]);
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!(npc.exp, 0);
        let combat = world.npc_combat[&ME].clone();
        assert!(combat.damage.is_empty());
        assert_eq!(world.reward_exp(ME), None, "a second Reward finds no exp");
    }

    /// A solo kill of a KING worth gold: the level-10 killer's 100% chance (rank 100 × delta
    /// 100) pays piles the view sees as ground adds before `GC_DEAD`, each with the fixed
    /// 300 s destroy event, and the piles are picked up for the purse (`RewardGold`,
    /// `G/char_battle.cpp:558-749`).
    #[test]
    fn a_solo_kill_of_a_king_pays_gold_piles_before_gc_dead() {
        use common::enums::EMobRank;
        let mut npc = a_monster(1);
        npc.fight.rank = EMobRank::King as u8;
        npc.fight.gold_range = [100, 100];
        let (_clock, mut world, mut alpha) = a_reward_fight(npc, 75, 10, NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        world.target(Vid::new(1), ME);
        // KING at equal levels: 100 × 100/100 = 100, so every seed pays.
        hit(&mut world, 1, ME);
        let _ = drained(&mut alpha);
        let _ = drained(&mut beta);
        world.process_pulse(P0 + 75);
        let dead = drained(&mut alpha);
        assert!(
            dead.iter().any(|record| *record == dead_record()),
            "GC_DEAD: {dead:02x?}"
        );
        assert!(!world.ground.is_empty(), "the kill laid piles");
        for lying in world.ground.values() {
            assert_eq!(lying.expires, P0 + 75 + 300 * 25, "the fixed 300 s");
            assert!(lying.owner.is_none(), "monster gold is unowned");
        }
        let beta_seen = drained(&mut beta);
        assert!(
            beta_seen.iter().any(|record| *record == dead_record()),
            "beta hears GC_DEAD"
        );
        // The piles pay the purse when picked up.
        let pile = *world.ground.keys().next().expect("a pile");
        let spot = world.ground[&pile].ground.clone();
        let (before, _) = {
            let character = world.characters.find_by_vid(Vid::new(1)).expect("alpha");
            (character.gold(), character.player_id())
        };
        world
            .pickup_item(
                Vid::new(1),
                pile,
                crate::game_loop_messages::GroundPlace {
                    channel: PLACE.0,
                    map: PLACE.1,
                    x: spot.x,
                    y: spot.y,
                },
                crate::item_move::Mover {
                    recently_fought: false,
                    empire: 1,
                    language: 1,
                    pk_mode: crate::loading_phase::PK_MODE_PEACE,
                    affect_flags: [0; 2],
                    dead: false,
                    alignment: 0,
                    view_guild: (0, 0),
                },
            )
            .expect("the pile pays");
        let after = world
            .characters
            .find_by_vid(Vid::new(1))
            .expect("alpha")
            .gold();
        assert!(after > before, "the purse grew by the pile");
    }

    /// A solo kill of a KING at equal levels takes the boss-gold arm: rank 100 x delta 100
    /// always pays, and the split of 25-35 piles (each `number(100, 100)` = 100) lands them
    /// all in view as ground adds (`RewardGold`'s boss arm, `G/char_battle.cpp:631-676`).
    #[test]
    fn a_solo_kill_of_a_king_takes_the_boss_gold_arm_with_its_split() {
        use common::enums::EMobRank;
        let mut npc = a_monster(1);
        npc.fight.rank = EMobRank::King as u8;
        npc.fight.gold_range = [100, 100];
        let (_clock, mut world, mut alpha) = a_reward_fight(npc, 75, 10, NEAR);
        // The killing hit draws `number(0, 0)` (unarmed) and the chat's `number(0, 60)`
        // (misses), then the kill recovery's `number(0, 9)` (misses), gold's `number(1,
        // 100)` (always pays at chance 100), the multiplier pair (both miss), and the
        // boss's `number(1, 100)` (misses the x10): seed 0's prefix draws exactly that,
        // with a split of 29.
        world.dice = Pcg32::new(0, 0);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        let _ = drained(&mut alpha);
        // 29 piles of 100/29 = 3: seed 0's split is 29.
        assert_eq!(world.ground.len(), 29, "the boss split lays 29 piles");
        for lying in world.ground.values() {
            assert_eq!(lying.ground.item.count, 3, "100 / 27 truncates to 3");
            assert_eq!(lying.expires, P0 + 300 * 25, "the fixed 300 s");
        }
    }

    /// A PAWN's gold chance is its rank percent through the delta table: level 10 vs 10 is
    /// 20 x 100/100 = 20, and seed 7's `number(1, 100)` draw of 83 is above it: no piles
    /// fall, and the kill still ends in `GC_DEAD` (`RewardGold`'s chance,
    /// `G/char_battle.cpp:573-603`).
    #[test]
    fn a_pawn_kill_below_its_chance_lays_no_gold() {
        let mut npc = a_monster(1);
        npc.fight.gold_range = [100, 100];
        let (_clock, mut world, mut alpha) = a_reward_fight(npc, 75, 10, NEAR);
        world.dice = Pcg32::new(7, 0);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        let heard = drained(&mut alpha);
        assert!(
            heard.iter().any(|record| *record == dead_record()),
            "GC_DEAD still sent: {heard:02x?}"
        );
        assert!(world.ground.is_empty(), "83 > 20 pays nothing");
    }

    /// The mall gold bonus raises the chance before the delta share: seed 9's `number(1,
    /// 100)` draw of 22 misses the PAWN's 20, but the bonus capped at 100 turns 20 into 40
    /// and the same draw pays (`POINT_MALL_GOLDBONUS`, `:583-584`).
    #[test]
    fn the_mall_gold_bonus_raises_the_chance_before_the_delta_share() {
        let mut npc = a_monster(1);
        npc.fight.gold_range = [100, 100];
        let (_clock, mut world, mut alpha) = a_reward_fight(npc, 75, 10, NEAR);
        // `APPLY_MALL_GOLDBONUS` caps the point at 100 (legacy's `CAPPED` list):
        // 20 + 20*100/100 = 40 >= 22.
        let _records = points_of(&mut world, 1).apply_point(68, 400);
        assert_eq!(
            points_of(&mut world, 1).get_point(point::POINT_MALL_GOLDBONUS),
            100
        );
        world.dice = Pcg32::new(9, 0);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        let _ = drained(&mut alpha);
        assert!(!world.ground.is_empty(), "22 <= 40 pays");
    }

    /// A kill runs `CreateDropItem`'s draws and lays nothing: every table is empty and every
    /// event flag is 0, so each `number()` misses -- and the dice advanced past the gold
    /// arm proves the quest chain's 13 draws ran (`CreateQuestDropItem`,
    /// `G/item_manager.cpp:1595-1910`).
    #[test]
    fn a_kill_runs_the_drop_draws_and_lays_no_item() {
        let mut npc = a_monster(1);
        npc.fight.gold_range = [0, 0];
        let (_clock, mut world, mut alpha) = a_reward_fight(npc, 75, 10, NEAR);
        world.dice = Pcg32::new(0, 0);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        let _ = drained(&mut alpha);
        assert!(world.ground.is_empty(), "no gold, no items");
        // The hit drew 2 (unarmed + chat), the recovery 1, the gold chance 1 (seed 0's
        // draw pays at 20 even for gold 0/0), the multiplier pair 2 (both miss), the
        // common arm's ten-draw 1 (misses) and its gold draw 1 (0, so no piles),
        // `GetDropPct` 2 (the else-chain: both miss), the lotto 1, and the quest chain 12
        // (2007, newyear x2, valentine, icecream, halloween, ramadan, easter, football,
        // whiteday, kids; `medal_part` needs mob 30+, `three_skill` mob 40+ BOSS, `mars`
        // killer 15+, xmas/moon/hex flag-gated, `hc_drop` needs killer 15+, and
        // `dragon_boat_festival_drop` is drawn last -- the count below pins the order):
        // 2 + 1 + 1 + 2 + 1 + 1 + 2 + 1 + 10 = 21 draws after the reseed.
        let mut dice = Pcg32::new(0, 0);
        for _ in 0..21 {
            let _ = world::character::number(&mut dice, 0, 0);
        }
        // Advance both one more draw and compare: equal states draw equal numbers.
        let mine = world::character::number(&mut world.dice, 0, 1000);
        let probe = world::character::number(&mut dice, 0, 1000);
        assert_eq!(mine, probe, "the chain drew exactly 21");
    }

    /// The several-item owner list keeps every online damager while `damage * 10 >=
    /// total`, greatest damage first, ties by lower VID (`Reward`'s queue,
    /// `G/char_battle.cpp:892-923`; the queue's pointer tie-break is a Defect, so the Rewrite
    /// uses VID order like the exp half).
    #[test]
    fn the_drop_owner_list_keeps_qualifiers_in_damage_order_with_vid_ties() {
        let (_clock, mut world, _alpha) = a_fight(a_monster(MAX_HP), NEAR);
        let _beta = a_player(&mut world, 2, EAST);
        let _gamma = a_player(&mut world, 3, (3150, 3200));
        let _delta = a_player(&mut world, 4, (3250, 3200));
        // Total 100: 1 and 3 tie at 40 (VID order), 2 at 15 qualifies (150 >= 100), 4 at 5
        // does not (50 < 100). VID 99 dealt damage but left: not online, skipped.
        world
            .npc_combat
            .entry(ME)
            .or_default()
            .damage
            .insert(1, row(40));
        world
            .npc_combat
            .entry(ME)
            .or_default()
            .damage
            .insert(2, row(15));
        world
            .npc_combat
            .entry(ME)
            .or_default()
            .damage
            .insert(3, row(40));
        world
            .npc_combat
            .entry(ME)
            .or_default()
            .damage
            .insert(4, row(5));
        world
            .npc_combat
            .entry(ME)
            .or_default()
            .damage
            .insert(99, row(40));
        let owners = world.drop_owner_list(ME);
        assert_eq!(
            owners,
            [Vid::new(1), Vid::new(3), Vid::new(2)],
            "ties by VID, the offline skipped, the small cut"
        );
    }

    /// A 2493 killed at level 75 lays its limit group's two hits: the made items lie on
    /// the ground with the row's vnum and count, and the several-hit round-robin names
    /// the killer (the only qualifier) on both (`CreateItem` + the placement,
    /// `G/item_manager.cpp:160-455`, `G/char_battle.cpp:892-989`).
    #[test]
    fn a_2493_kill_at_limit_lays_its_hits_with_the_killers_ownership() {
        use gamedata::drop::{DropItem, DropTables, LevelGroup};
        use world::item::ItemIdRange;
        let mut npc = a_monster(1);
        npc.race = 2493;
        npc.fight.gold_range = [0, 0];
        let (_clock, mut world, mut alpha) = a_reward_fight(npc, 75, 10, NEAR);
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 1).expect("a usable range"))
            .expect("the first install");
        let mut tables = DropTables::default();
        tables.mob.limit.insert(
            2493,
            LevelGroup {
                level_limit: 75,
                items: vec![
                    DropItem {
                        vnum: 30077,
                        pct: 15 * 1000,
                        count: 15,
                    },
                    DropItem {
                        vnum: 15370,
                        pct: 60,
                        count: 1,
                    },
                ],
            },
        );
        world.drops = Arc::new(tables);
        // The killer at the limit: the fixture enters at 10.
        let stamina = points_of(&mut world, 1).get_point(point::POINT_STAMINA);
        let points = super::super::fixtures::points_at(75, stamina, points_of(&mut world, 1).exp());
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("the killer stands")
            .set_points(Some(points));
        // Seed 896849 (found offline): the first hit stuns (its unarmed roll and a
        // missing chat), the second finishes (its unarmed roll and a missing chat),
        // then the kill recovery misses, the gold chance misses at chance 0, and
        // `drop_pct`'s two legs miss — 6 draws before the limit arm. Row 1's draw hits
        // (11307 <= 15000), its creation's magic gate draws once and misses, then row
        // 2's draw hits (52 <= 60).
        world.dice = Pcg32::new(896_849, 0);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        let _ = drained(&mut alpha);
        let mut ground: Vec<_> = world.ground.values().collect();
        ground.sort_by_key(|lying| lying.ground.item.vnum);
        assert_eq!(ground.len(), 2, "both rows hit and laid");
        assert_eq!(ground[0].ground.item.vnum, 15370);
        assert_eq!(ground[1].ground.item.vnum, 30077);
        assert_eq!(ground[1].ground.item.count(), 15);
        for lying in &ground {
            assert_eq!(
                lying.owner.as_ref().map(|owner| owner.pid),
                Some(1),
                "the killer owns every hit of a multi-hit kill"
            );
        }
    }

    /// A 2493 killed at level 75 with one hitting row lays one item, and the one-hit arm
    /// names the killer directly (`G/char_battle.cpp:870-890`). Seed 20 (found offline):
    /// both hits' chats miss, the kill recovery misses, the gold chance misses at chance
    /// 0, `drop_pct`'s legs miss, and the row's draw hits (9401 <= 15000).
    #[test]
    fn a_single_limit_hit_lays_one_item_owned_by_the_killer() {
        use gamedata::drop::{DropItem, DropTables, LevelGroup};
        use world::item::ItemIdRange;
        let mut npc = a_monster(1);
        npc.race = 2493;
        npc.fight.gold_range = [0, 0];
        let (_clock, mut world, mut alpha) = a_reward_fight(npc, 75, 10, NEAR);
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 1).expect("a usable range"))
            .expect("the first install");
        let mut tables = DropTables::default();
        tables.mob.limit.insert(
            2493,
            LevelGroup {
                level_limit: 75,
                items: vec![DropItem {
                    vnum: 30077,
                    pct: 15 * 1000,
                    count: 15,
                }],
            },
        );
        world.drops = Arc::new(tables);
        let stamina = points_of(&mut world, 1).get_point(point::POINT_STAMINA);
        let points = super::super::fixtures::points_at(75, stamina, points_of(&mut world, 1).exp());
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("the killer stands")
            .set_points(Some(points));
        world.dice = Pcg32::new(20, 0);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        let _ = drained(&mut alpha);
        let ground: Vec<_> = world.ground.values().collect();
        assert_eq!(ground.len(), 1, "the one row hit and laid");
        assert_eq!(ground[0].ground.item.vnum, 30077);
        assert_eq!(ground[0].ground.item.count(), 15);
        assert_eq!(
            ground[0].owner.as_ref().map(|owner| owner.pid),
            Some(1),
            "the one-hit arm names the killer"
        );
        assert!(
            world.npc_combat[&ME].damage.is_empty(),
            "the placement clears the damage map"
        );
    }

    /// A hitting row whose vnum takes an unported creation arm lays nothing: seed 0's
    /// chats miss, the kill recovery misses, `drop_pct`'s legs miss, and the row always
    /// hits (pct 1000000), but 72723 is a socketed vnum, so creation refuses with a warn
    /// and the placement's empty arm clears the damage map.
    #[test]
    fn an_unported_limit_hit_lays_nothing_and_clears_the_damage_map() {
        use gamedata::drop::{DropItem, DropTables, LevelGroup};
        use world::item::ItemIdRange;
        let mut npc = a_monster(1);
        npc.race = 2493;
        npc.fight.gold_range = [0, 0];
        let (_clock, mut world, mut alpha) = a_reward_fight(npc, 75, 10, NEAR);
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 1).expect("a usable range"))
            .expect("the first install");
        let mut tables = DropTables::default();
        tables.mob.limit.insert(
            2493,
            LevelGroup {
                level_limit: 75,
                items: vec![DropItem {
                    vnum: 72723,
                    pct: 1_000_000,
                    count: 1,
                }],
            },
        );
        world.drops = Arc::new(tables);
        let stamina = points_of(&mut world, 1).get_point(point::POINT_STAMINA);
        let points = super::super::fixtures::points_at(75, stamina, points_of(&mut world, 1).exp());
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("the killer stands")
            .set_points(Some(points));
        world.dice = Pcg32::new(0, 0);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        let _ = drained(&mut alpha);
        assert!(world.ground.is_empty(), "the refused hit lays nothing");
        assert!(
            world.npc_combat[&ME].damage.is_empty(),
            "the empty arm still clears the damage map"
        );
    }

    /// Clearing an owned pile sends the empty-name record and frees it; clearing an unowned
    /// pile sends nothing (`SetOwnership(NULL)`, `G/item.cpp:1735-1749`).
    #[test]
    fn clearing_a_ground_owner_sends_the_empty_name_once() {
        use crate::game_state::view_encode;
        let mut npc = a_monster(1);
        npc.fight.rank = common::enums::EMobRank::King as u8;
        npc.fight.gold_range = [100, 100];
        // Lay a pile through the gold half directly: KING at equal levels always pays.
        let (_clock, mut world, _alpha) = a_reward_fight(npc, 75, 10, NEAR);
        world.dice = Pcg32::new(0, 0);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        hit(&mut world, 1, ME);
        let pile = *world.ground.keys().next().expect("a pile");
        let name = crate::loading_phase::name_field("Alpha");
        world.set_ground_ownership(pile, Some((1, name)), 10);
        assert!(world.ground[&pile].owner.is_some(), "owned");
        // Keep-first: a second owner is refused silently.
        let other = crate::loading_phase::name_field("Beta");
        world.set_ground_ownership(pile, Some((2, other)), 10);
        assert_eq!(world.ground[&pile].owner.as_ref().unwrap().pid, 1);
        world.set_ground_ownership(pile, None, 10);
        assert!(world.ground[&pile].owner.is_none(), "cleared");
        let empty = view_encode::ground_ownership(pile, [0; 25]);
        assert_eq!(empty.len(), 1 + 4 + 25, "header + VID + empty name");
        // Clearing again sends nothing and changes nothing.
        world.set_ground_ownership(pile, None, 10);
        assert!(world.ground[&pile].owner.is_none());
    }

    /// Two attackers split the experience by damage: the top takes 20% plus its share of the
    /// rest, the other its share (`DistributeExp`'s two walks, `G/char_battle.cpp:2920-2979`).
    /// The most-attacked is the returned attacker, whatever the VIDs.
    #[test]
    fn two_attackers_split_by_damage_with_the_top_bonus_to_the_first_max() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 10, NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        drained(&mut beta);
        // Alpha's stunning hit is in the map; Beta's earlier 30 joins it after the drains,
        // so the first maximum is Beta's whatever the VIDs.
        let dealt = world.npc_combat[&ME].damage[&1].total;
        world
            .npc_combat
            .entry(ME)
            .or_default()
            .damage
            .insert(2, row(30));
        hit(&mut world, 1, ME);
        // Top is Beta with 30 of (30 + dealt): 20 + its share; Alpha takes its share.
        let total = 30 + dealt;
        let beta_share = 20 + common::cfloat::f64_to_i32(80.0 * (30.0 / f64::from(total)));
        let alpha_share = common::cfloat::f64_to_i32(80.0 * (f64::from(dealt) / f64::from(total)));
        // Each grant sends its exp record then its orb, top first.
        assert_eq!(
            drained(&mut alpha),
            [
                no_target(),
                owner(0),
                fly(2),
                exp_change(1, i64::from(alpha_share), i64::from(alpha_share)),
                fly(1),
                dead_record()
            ]
        );
        assert_eq!(
            drained(&mut beta),
            [
                owner(0),
                exp_change(2, i64::from(beta_share), i64::from(beta_share)),
                fly(2),
                fly(1),
                dead_record()
            ]
        );
        assert_eq!(world.reward_exp(ME), None, "a second Reward finds no exp");
    }
    /// An attacker beyond 5000 earns nothing; the in-range attacker still takes the whole
    /// experience.
    #[test]
    fn attackers_beyond_range_earn_nothing() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 10, NEAR);
        // Beta stands 5400 east: `DISTANCE_APPROX` reads 5189, past the 5000 gate (its
        // under-read needs a true ~5208).
        let mut beta = enter(&mut world, 2, (SPOT.0 + 5400, SPOT.1), 12_345);
        drained(&mut beta);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        drained(&mut beta);
        let dealt = world.npc_combat[&ME].damage[&1].total;
        world
            .npc_combat
            .entry(ME)
            .or_default()
            .damage
            .insert(2, row(10 * dealt));
        hit(&mut world, 1, ME);
        assert_eq!(
            drained(&mut alpha),
            [
                no_target(),
                owner(0),
                exp_change(1, 100, 100),
                fly(1),
                dead_record()
            ]
        );
        // Beta sees the view's records but no exp record of its own.
        assert_eq!(drained(&mut beta), [owner(0), fly(1), dead_record()]);
    }
    /// An attacker on another map earns nothing even at the same coordinates (D2: legacy
    /// checks only the distance, which would grant across maps).
    #[test]
    fn attackers_on_another_map_earn_nothing() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 10, NEAR);
        world.host_map(
            PLACE.0,
            PLACE.1 + 1,
            SectreeGrid {
                x: 0,
                y: 0,
                columns: 10,
                rows: 10,
            },
        );
        let mut beta = enter_seeing(&mut world, (PLACE.0, PLACE.1 + 1), 2, NEAR, 12_345).0;
        drained(&mut beta);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        drained(&mut beta);
        let dealt = world.npc_combat[&ME].damage[&1].total;
        world
            .npc_combat
            .entry(ME)
            .or_default()
            .damage
            .insert(2, row(10 * dealt));
        hit(&mut world, 1, ME);
        assert_eq!(
            drained(&mut alpha),
            [
                no_target(),
                owner(0),
                exp_change(1, 100, 100),
                fly(1),
                dead_record()
            ]
        );
        assert!(
            drained(&mut beta).is_empty(),
            "Beta earns nothing cross-map"
        );
    }
    /// A monster worth no experience distributes nothing: no record, no orb, and the damage
    /// map is left for the destroy (`DistributeExp`'s first return, `:2822`).
    #[test]
    fn a_monster_worth_no_exp_distributes_nothing() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 0, 10, NEAR);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        assert_eq!(drained(&mut alpha), [no_target(), owner(0), dead_record()]);
        assert!(!world.npc_combat[&ME].damage.is_empty(), "the map is kept");
        assert_eq!(world.reward_exp(ME), None);
    }

    /// A monster whose attackers all left distributes nothing but consumes its experience
    /// (`SetExp(0)` runs before the empty return, `:2893-2897`).
    #[test]
    fn a_kill_with_no_attacker_in_range_consumes_the_exp() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 10, NEAR);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        // Alpha's damage stays in the map, but Alpha leaves the world before the stun ends.
        world.leave_world_by(
            Vid::new(1),
            crate::game_loop_messages::Departure::Disconnect,
        );
        world.process_pulse(P0 + 75);
        assert_eq!(world.npc_by_vid(ME).unwrap().exp, 0);
        assert_eq!(world.reward_exp(ME), None);
    }
    /// A level-10 killer of a level-25 monster takes the 180% row; of a level-1 monster the
    /// 70% row; of a level-9 one the 100% row (`CALCULATE_VALUE_LVDELTA`).
    #[test]
    fn the_level_delta_scales_the_share() {
        for (monster_level, percent) in [(25_u8, 180_i64), (1_u8, 70_i64), (9_u8, 100_i64)] {
            let (_clock, mut world, mut alpha) =
                a_reward_fight(a_monster(1), 100, monster_level, NEAR);
            world.target(Vid::new(1), ME);
            hit(&mut world, 1, ME);
            drained(&mut alpha);
            hit(&mut world, 1, ME);
            let grant = 100 * percent / 100;
            assert_eq!(
                drained(&mut alpha),
                [
                    no_target(),
                    owner(0),
                    exp_change(1, grant, grant),
                    fly(1),
                    dead_record()
                ],
                "monster level {monster_level}"
            );
        }
    }

    /// A grant past a tenth of the next level's cost is capped there (`MIN(GetNextExp()/10)`).
    #[test]
    fn a_grant_past_a_tenth_of_the_next_cost_is_capped() {
        // Level 10 needs 33,000: the tenth is 3,300, and the monster pays 100,000.
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100_000, 10, NEAR);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        assert_eq!(
            drained(&mut alpha),
            [
                no_target(),
                owner(0),
                exp_change(1, 3_300, 3_300),
                fly(1),
                dead_record()
            ]
        );
    }
    /// The ramadan and mall bonuses stack in order: a 100 share with ramadan 30 and mall
    /// 50 grants 195 (`exp += exp * pct / 100` per term, `:2685-2686`).
    #[test]
    fn the_ramadan_and_mall_bonuses_stack_in_order() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 10, NEAR);
        points_of(&mut world, 1)
            .point_change(point::POINT_RAMADAN_CANDY_BONUS_EXP, 30, false, false)
            .unwrap();
        points_of(&mut world, 1)
            .point_change(point::POINT_MALL_EXPBONUS, 50, false, false)
            .unwrap();
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        assert_eq!(
            drained(&mut alpha),
            [
                no_target(),
                owner(0),
                exp_change(1, 195, 195),
                fly(1),
                dead_record()
            ]
        );
    }
    /// A tenth of kills heals the attacker: with `POINT_KILL_HP_RECOVERY` at 10 and the
    /// 1-in-10 draw landing 0, the killer takes a tenth of its max HP with a small-HP orb
    /// after the exp orb (`DistributeHP`, `:2438-2442`, in `Reward`'s tail, `:789-792`).
    /// The seed scan replays the killing hit on trial worlds until the draw lands 0.
    #[test]
    fn a_tenth_of_kills_heals_the_attacker() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 10, NEAR);
        points_of(&mut world, 1)
            .point_change(point::POINT_KILL_HP_RECOVERY, 10, false, false)
            .unwrap();
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        // The killing hit draws the chat line's `number(0, 60)` before the stun check (a
        // second `number(1, 3)` follows only when the chat fires), and `POINT_EXP_DOUBLE_BONUS`
        // is 0, so the recovery draw is the seed's second draw when the first misses 0.
        // The killing hit draws `number(0, 0)` (unarmed roll; the `number(1, 5)` floor is
        // skipped at 28 damage) and the chat line's `number(0, 60)` (no second draw when
        // the chat misses), then the recovery's `number(0, 9)` (no other draw between:
        // `exp_double` is 0, no level-up, no quests, no steals).
        let seed = (0..100_000)
            .find(|&seed| {
                let mut dice = Pcg32::new(seed, 0);
                let _ = world::character::number(&mut dice, 0, 0);
                world::character::number(&mut dice, 0, 60) != 0
                    && world::character::number(&mut dice, 0, 9) == 0
            })
            .expect("a seed in 100,000");
        world.dice = Pcg32::new(seed, 0);
        let max_hp = points_of(&mut world, 1).max_hp();
        // Alpha stands hurt: the heal lands below max and writes its record.
        points_of(&mut world, 1)
            .point_change(point::POINT_HP, -500, false, false)
            .unwrap();
        hit(&mut world, 1, ME);
        let heal = max_hp * 10 / 100;
        let mut orbs = Vec::new();
        GcCreateFly::new(FLY_HP_SMALL, ME, 1).encode_into(&mut orbs);
        assert_eq!(
            drained(&mut alpha),
            [
                no_target(),
                owner(0),
                exp_change(1, 100, 100),
                fly(1),
                // The heal hides the amount and carries the value.
                point_changes(
                    &[PointRecord {
                        kind: u8::try_from(point::POINT_HP).unwrap(),
                        amount: 0,
                        value: i64::from(max_hp - 500 + heal),
                        broadcast: false,
                    }],
                    1,
                )
                .concat(),
                orbs,
                dead_record()
            ]
        );
    }
    /// A kill of a monster within 10 levels raises a non-negative alignment by 2, and the
    /// /10 crossing sends the killer's update (`UpdateAlignment`, `:3477-3494`); a kill
    /// without a crossing sends no update. The update goes around the killer, who gets its
    /// own copy, after the exp record and orb and before `GC_DEAD`.
    #[test]
    fn a_kill_raises_alignment_and_a_tenth_crossing_sends_the_update() {
        for (start, crossed) in [(8_i32, true), (0_i32, false)] {
            let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 10, NEAR);
            world.bodies.get_mut(&Vid::new(1)).unwrap().card.alignment = start;
            world.target(Vid::new(1), ME);
            hit(&mut world, 1, ME);
            drained(&mut alpha);
            hit(&mut world, 1, ME);
            assert_eq!(
                world.bodies[&Vid::new(1)].card.alignment,
                start + 2,
                "start {start}"
            );
            let mut expected = vec![no_target(), owner(0), exp_change(1, 100, 100), fly(1)];
            if crossed {
                expected.push(
                    world
                        .update_frame(Vid::new(1), world.affect_flags_of(Vid::new(1)))
                        .expect("Alpha is online"),
                );
            }
            expected.push(dead_record());
            assert_eq!(drained(&mut alpha), expected, "start {start}");
        }
    }

    /// A kill of a monster more than 10 levels below leaves the alignment alone.
    #[test]
    fn a_kill_of_a_far_lower_monster_leaves_alignment_alone() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 1, NEAR);
        // Level 10 vs level 1: 1 - 10 = -9, within the gate; a level-21 killer stands
        // 20 below... instead the monster's level 1 against a level-12-plus killer
        // misses by 11. Set the body's level directly.
        world.bodies.get_mut(&Vid::new(1)).unwrap().card.level = 12;
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        assert_eq!(world.bodies[&Vid::new(1)].card.alignment, 0);
        assert_eq!(
            drained(&mut alpha),
            [
                no_target(),
                owner(0),
                exp_change(1, 70, 70),
                fly(1),
                dead_record()
            ]
        );
    }
    /// A kill with a negative alignment raises it by 7, or by 14 with the faster-alignment
    /// unique (70051) worn (`:796-801`).
    #[test]
    fn a_kill_with_a_negative_alignment_rises_by_7_or_14_with_the_unique() {
        for worn in [false, true] {
            let clock = TestClock::default();
            clock.set(T0);
            let mut monster = a_monster(1);
            monster.exp = 100;
            monster.fight.level = 10;
            let mut world = a_field_on(&clock, vec![monster], open_cells());
            let mut items = Vec::new();
            if worn {
                let mut medal = Item::new(11, 70_051);
                medal.set_size(1).unwrap();
                let cell = INVENTORY_MAX_NUM + EWearPositions::Unique1 as u16;
                items.push((ItemPos::new(EWindows::Inventory as u8, cell), medal));
            }
            let (mut alpha, _shown) = enter_seeing_with(&mut world, PLACE, 1, NEAR, 12_345, &items);
            world.process_pulse(P0);
            world.bodies.get_mut(&Vid::new(1)).unwrap().card.alignment = -8;
            world.target(Vid::new(1), ME);
            hit(&mut world, 1, ME);
            drained(&mut alpha);
            hit(&mut world, 1, ME);
            let gain = if worn { 14 } else { 7 };
            assert_eq!(
                world.bodies[&Vid::new(1)].card.alignment,
                -8 + gain,
                "worn {worn}"
            );
            // -8/10 = 0 truncates toward zero on both sides; -1 and 6 share the quotient,
            // so neither kill sends the update.
            assert_eq!(
                drained(&mut alpha),
                [
                    no_target(),
                    owner(0),
                    exp_change(1, 100, 100),
                    fly(1),
                    dead_record()
                ],
                "worn {worn}"
            );
        }
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
        let combat = world.npc_combat[&ME].clone();
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
    /// `:3287-3314`). The hitter steps out of the 4000 drop so the chase sends nothing (G33).
    #[test]
    fn a_hit_monster_recovers_until_it_is_full() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        let damage = expected_damage(&mut world, 1);
        hit(&mut world, 1, ME);
        // The hit starts a fight; the hitter steps out of the 4000 drop, so the chase sends
        // nothing and the recovery assertions stay as they are.
        let spot = world.spot_of(1).unwrap();
        assert!(world.move_to(1, spot.x + 4400, spot.y));
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

    /// Only a monster or a stone standing on the attacker's Channel and map is hit (V17a):
    /// any other kind of non-player, race 5001, a fake hydra (`G/battle.cpp:134-139`), a
    /// player, the attacker itself, a monster on another Channel or on another map of this
    /// one, a VID that names nothing and a skill's type are refused before any draw, with
    /// nothing sent.
    #[test]
    fn only_a_monster_is_hit() {
        let refused = [
            (CHAR_TYPE_NPC, 101),
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
        // Control: a stone is struck (it fights nothing back).
        let mut npc = a_monster(1);
        npc.char_type = CHAR_TYPE_STONE;
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
                let _died = world.damage_pc(None, Vid::new(1), hp, None);
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
                let _died = world.damage_pc(None, Vid::new(1), hp, None);
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
    // appended temporarily to fight.rs tests

    /// A stunned player's own attack is refused: nothing is sent and the monster's pools
    /// hold (`CanMove` on the stun).
    #[test]
    fn a_stunned_players_own_attack_is_refused() {
        use super::super::affect::Affect;
        use crate::loading_phase::{AFFECT_STUN, AFF_STUN, STUN_TICKS};
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        let before = world.npc_by_vid(ME).unwrap().hp;
        world.add_pc_affect(
            Vid::new(1),
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
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        assert!(drained(&mut alpha).is_empty());
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, before);
    }

    /// A stunned player cannot `BeginFight`: a monster's hit changes nothing of its Move
    /// or its flag.
    #[test]
    fn a_stunned_player_cannot_begin_fight() {
        use super::super::affect::Affect;
        use crate::loading_phase::{AFFECT_STUN, AFF_STUN, STUN_TICKS};
        let (_clock, mut world, _alpha) = a_fight(a_monster(MAX_HP), NEAR);
        world.add_pc_affect(
            Vid::new(1),
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
        world.goto(1, NEAR.0 + 2000, NEAR.1);
        world.npc_mut(ME).unwrap().fight.attack_range = 175;
        assert!(world.mob_attack(ME, Vid::new(1)));
        let body = world.bodies.get(&Vid::new(1)).unwrap();
        assert!(body.motion.moving);
        assert!(!body.fighting);
    }

    /// A stone is hit: its HP falls, the damage map fills, the recovery starts, and it
    /// holds no victim and asks no Battle.
    #[test]
    fn a_stone_is_hit_but_fights_nothing() {
        let mut stone = a_monster(MAX_HP);
        stone.char_type = CHAR_TYPE_STONE;
        stone.vnum = 8001;
        stone.empire = 0;
        let (_clock, mut world, mut alpha) = a_fight(stone, NEAR);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        let npc = world.npc_by_vid(ME).expect("the stone stands");
        assert!(npc.hp < MAX_HP);
        let combat = world.npc_combat.get(&ME).expect("the stone fights");
        assert!(combat.damage.contains_key(&1));
        assert!(combat.recovery_due.is_some());
        assert!(world.minds.get(ME).is_none_or(|mind| !mind.fighting));
        assert!(world.victim_of(ME).is_none());
        assert!(!drained(&mut alpha).is_empty());
    }

    /// A same-empire attacker on a nonzero-empire stone is blocked: 0 damage, HP
    /// unchanged, and the targeting attacker is shown `DAMAGE_BLOCK`.
    #[test]
    fn a_same_empire_stone_hit_is_blocked() {
        let mut stone = a_monster(MAX_HP);
        stone.char_type = CHAR_TYPE_STONE;
        stone.vnum = 8001;
        stone.empire = 1;
        let (_clock, mut world, mut alpha) = a_fight(stone, NEAR);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        let dice = world.dice.clone();
        hit(&mut world, 1, ME);
        assert_ne!(world.dice, dice, "the rating draws ran");
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, MAX_HP);
        assert_eq!(
            drained(&mut alpha),
            [damage_info(0, DAMAGE_BLOCK), owner(1)]
        );
    }

    /// A son's death feeds half its exp to its live stone before the split: the stone's
    /// exp grows by `exp >> 1`, and the killer takes the rest's share.
    #[test]
    fn a_sons_death_feeds_half_its_exp_to_the_stone() {
        let mut son = a_monster(1);
        son.stone = Some(ME + 1);
        let clock = TestClock::default();
        clock.set(T0);
        let mut stone = a_monster(MAX_HP);
        stone.vid = ME + 1;
        stone.char_type = CHAR_TYPE_STONE;
        stone.vnum = 8001;
        stone.empire = 0;
        stone.exp = 1000;
        let mut world = a_field_on(&clock, vec![son, stone], open_cells());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut inbox);
        world.npc_mut(ME).expect("the son stands").exp = 100;
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        hit(&mut world, 1, ME);
        assert!(world.npc_by_vid(ME).unwrap().dead);
        assert_eq!(world.npc_by_vid(ME + 1).unwrap().exp, 1000 + 50);
        assert_eq!(world.npc_by_vid(ME).unwrap().exp, 0);
    }

    /// A stone's death kills its sons in map order: each is dead, each sent `GC_DEAD`
    /// after the stone's, and each unlinked.
    #[test]
    fn a_stones_death_kills_its_sons() {
        let clock = TestClock::default();
        clock.set(T0);
        let mut stone = a_monster(1);
        stone.char_type = CHAR_TYPE_STONE;
        stone.vnum = 8001;
        stone.empire = 0;
        let mut son_a = a_monster(MAX_HP);
        son_a.vid = ME + 1;
        son_a.stone = Some(ME);
        let mut son_b = a_monster(MAX_HP);
        son_b.vid = ME + 2;
        son_b.stone = Some(ME);
        let mut world = a_field_on(&clock, vec![stone, son_a, son_b], open_cells());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut inbox);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        hit(&mut world, 1, ME);
        assert!(world.npc_by_vid(ME).unwrap().dead);
        assert!(world.npc_by_vid(ME + 1).unwrap().dead);
        assert!(world.npc_by_vid(ME + 2).unwrap().dead);
        assert!(world.npc_by_vid(ME + 1).unwrap().stone.is_none());
        assert!(world.npc_by_vid(ME + 2).unwrap().stone.is_none());
        let frames = drained(&mut inbox);
        let dead_of = |vid| header_and_vid(HEADER_GC_DEAD.value(), vid);
        let at = |vid: u32| frames.iter().position(|f| *f == dead_of(vid));
        assert!(at(ME) < at(ME + 1), "the stone dies first");
        assert!(at(ME + 1) < at(ME + 2), "sons in map order");
    }

    /// Destroying a stone dead-ends its sons the same; destroying a son drops it from
    /// the stone's derived set, and the stone still idles.
    #[test]
    fn destroying_a_stone_kills_its_sons() {
        let clock = TestClock::default();
        clock.set(T0);
        let mut stone = a_monster(MAX_HP);
        stone.char_type = CHAR_TYPE_STONE;
        stone.vnum = 8001;
        let mut son = a_monster(MAX_HP);
        son.vid = ME + 1;
        son.stone = Some(ME);
        let mut world = a_field_on(&clock, vec![stone, son], open_cells());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut inbox);
        world.destroy_npc(ME);
        assert!(world.npc_by_vid(ME).is_none());
        assert!(world.npc_by_vid(ME + 1).unwrap().dead);
        assert!(world.npc_by_vid(ME + 1).unwrap().stone.is_none());
        // The son's destroy drops it from the set; the stone idles on.
        let clock = TestClock::default();
        clock.set(T0);
        let mut stone = a_monster(MAX_HP);
        stone.char_type = CHAR_TYPE_STONE;
        stone.vnum = 8001;
        let mut son = a_monster(MAX_HP);
        son.vid = ME + 1;
        son.stone = Some(ME);
        let mut world = a_field_on(&clock, vec![stone, son], open_cells());
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.process_pulse(P0);
        drained(&mut inbox);
        world.destroy_npc(ME + 1);
        assert!(world.npc_by_vid(ME + 1).is_none());
        world.npc_mut(ME).expect("the stone stands").hp = 40;
        world.process_pulse(P0 + 1);
        assert!(world.npc_by_vid(ME).is_some());
    }

    /// Seeds 100 % poison, stun and slow chances on the player 1.
    fn all_chances(world: &mut GameState) {
        let _records = points_of(world, 1).apply_point(12, 100);
        let _records = points_of(world, 1).apply_point(13, 100);
        let _records = points_of(world, 1).apply_point(14, 100);
    }

    /// A hit with every chance lands the poison affect and schedules its event one Pulse
    /// on, stuns and slows the victim, and the neighbourhood hears the updates.
    #[test]
    fn a_hit_with_every_chance_lands_the_poison_stun_and_slow() {
        use protocol::gc_actors::GcCharacterUpdate;
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        all_chances(&mut world);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        hit(&mut world, 1, ME);
        let words = world.npc_affect_words(ME);
        assert_eq!(
            words[0] & (AFF_POISON_WORD | AFF_STUN_WORD),
            AFF_POISON_WORD | AFF_STUN_WORD
        );
        assert_ne!(words[0] & (1 << 5), 0, "the slow word");
        let combat = world.npc_combat[&ME].clone();
        assert!(combat.poisoned_once);
        assert_eq!(combat.poison_due, Some((P0 + 1, 10)));
        assert_eq!(combat.poison_attacker, Some(1));
        assert_eq!(
            world
                .npc_by_vid(ME)
                .unwrap()
                .fight
                .point(point::POINT_MOV_SPEED),
            -30
        );
        let heard: Vec<Vec<u8>> = drained(&mut alpha);
        let updates: Vec<GcCharacterUpdate> = heard
            .iter()
            .filter_map(|record| GcCharacterUpdate::decode(record).ok())
            .collect();
        assert!(!updates.is_empty(), "an update went out: {heard:02x?}");
        assert!(
            !world.can_begin_fight(ME),
            "stunned by the hit, it does not begin"
        );
    }

    /// A hit with `POINT_STEAL_GOLD` pays the purse `number(1, victim level)`: the dog is
    /// level 1, so the purse gains 1, the attacker hears `GC_CHARACTER_GOLD_CHANGE`, and
    /// the answer carries the delta. The steal lands before the damage packet and the
    /// hit-point change, inside `Damage` (`G/char_battle.cpp:2080-2092`).
    #[test]
    fn a_hit_with_steal_gold_pays_the_purse_and_answers_the_delta() {
        use protocol::gc_actors::GcCharacterGoldChange;
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        let _records =
            points_of(&mut world, 1).point_change(point::POINT_STEAL_GOLD, 100, false, false);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        let held = world.characters.find_by_vid(Vid::new(1)).unwrap().gold();
        let answer = world.attack(Vid::new(1), CgAttack::new(0, ME, 0, 0));
        assert_eq!(answer.gold, Some(1));
        let paid = world.characters.find_by_vid(Vid::new(1)).unwrap().gold();
        assert_eq!(paid, held + 1);
        let heard: Vec<Vec<u8>> = drained(&mut alpha);
        let mut gold = Vec::new();
        GcCharacterGoldChange::new(1, 1, paid).encode_into(&mut gold);
        assert!(
            heard.iter().any(|record| *record == gold),
            "the gold record went out: {heard:02x?}"
        );
        let at = heard
            .iter()
            .position(|record| *record == gold)
            .expect("the gold record");
        let damage_at = heard.iter().position(|record| {
            record.first() == Some(&protocol::gc_nested::GcDamageInfo::header())
        });
        if let Some(damage_at) = damage_at {
            assert!(at < damage_at, "gold before the damage packet");
        }
    }

    /// At the cap the steal refuses with nothing changed, nothing sent and no delta:
    /// `ChangeGold`'s `OVERFLOW_GOLD` arm returns (`G/char.cpp:3827-3832`).
    #[test]
    fn a_steal_at_the_gold_cap_refuses_silently() {
        use protocol::gc_inventory::HEADER_GC_CHARACTER_GOLD_CHANGE;
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        let _records =
            points_of(&mut world, 1).point_change(point::POINT_STEAL_GOLD, 100, false, false);
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .unwrap()
            .set_gold(GOLD_MAX_MAX - 1);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        let answer = world.attack(Vid::new(1), CgAttack::new(0, ME, 0, 0));
        assert_eq!(answer.gold, None);
        assert_eq!(
            world.characters.find_by_vid(Vid::new(1)).unwrap().gold(),
            GOLD_MAX_MAX - 1
        );
        let heard: Vec<Vec<u8>> = drained(&mut alpha);
        assert!(
            heard
                .iter()
                .all(|record| record.first() != Some(&HEADER_GC_CHARACTER_GOLD_CHANGE.value())),
            "no gold record: {heard:02x?}"
        );
    }

    /// The rate reads the rank table less `POISON_REDUCE`, floored at 0, and a rank past
    /// the table's end reads the last per-mille (legacy would read out of bounds).
    #[test]
    fn the_poison_rate_follows_the_rank_and_the_reduce_with_a_clamp() {
        let mut npc = a_monster(MAX_HP);
        npc.fight.rank = 5;
        let (_clock, mut world, _alpha) = a_fight(npc, NEAR);
        assert_eq!(world.npc_poison_rate(ME), 1, "KING's per-mille");
        world
            .npc_mut(ME)
            .unwrap()
            .fight
            .add(point::POINT_POISON_REDUCE, 1);
        assert_eq!(world.npc_poison_rate(ME), 0, "the floor");
        world.npc_mut(ME).unwrap().fight.rank = 200;
        assert_eq!(world.npc_poison_rate(ME), 0, "the clamp, still reduced");
        world
            .npc_mut(ME)
            .unwrap()
            .fight
            .add(point::POINT_POISON_REDUCE, -1);
        assert_eq!(world.npc_poison_rate(ME), 1, "the clamp reads KING");
        world.npc_mut(ME).unwrap().fight.rank = 1;
        assert_eq!(world.npc_poison_rate(ME), 50, "PAWN's per-mille");
    }

    /// The poison tick deals `maxHP * rate / 1000`, ten ticks three seconds apart, feeds
    /// the damage map and the aggro, and shows `DAMAGE_POISON` to the targeter.
    #[test]
    fn the_npc_poison_event_ticks_ten_times_with_the_rank_rate() {
        use protocol::gc_nested::GcDamageInfo;
        let mut npc = a_monster(MAX_HP);
        npc.fight.rank = 0;
        let (_clock, mut world, mut alpha) = a_fight(npc, NEAR);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        world.poison_npc(Vid::new(1), ME);
        let tick = MAX_HP * 80 / 1000;
        let mut pulse = P0 + 1;
        for left in (1..=10).rev() {
            world.process_pulse(pulse);
            let heard = drained(&mut alpha);
            let infos: Vec<(u8, i32)> = heard
                .iter()
                .filter_map(|record| GcDamageInfo::decode(record).ok())
                .map(|info| (info.flag, info.damage))
                .collect();
            assert_eq!(infos, [(DAMAGE_POISON, tick)], "tick with {left} left");
            assert!(!world.npc_by_vid(ME).unwrap().dead);
            pulse += 75;
        }
        assert!(world.npc_combat[&ME].poison_due.is_none());
        assert_eq!(
            world.npc_combat[&ME].damage.get(&1).map(|row| row.total),
            Some(tick * 10)
        );
        assert!(world.npc_combat[&ME].damage[&1].aggro > 0);
    }

    /// The HP-1 floor: a tick on 1 hit point deals 0, the event continues, and a
    /// re-poison is gated by the running event and then by the once flag.
    #[test]
    fn the_poison_floor_never_kills_and_a_re_poison_is_gated() {
        let mut npc = a_monster(MAX_HP);
        npc.hp = 1;
        let (_clock, mut world, mut alpha) = a_fight(npc, NEAR);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        world.poison_npc(Vid::new(1), ME);
        world.process_pulse(P0 + 1);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, 1);
        assert_eq!(world.npc_combat[&ME].poison_due, Some((P0 + 1 + 75, 9)));
        // A re-poison while the event runs is gated by it.
        world.poison_npc(Vid::new(1), ME);
        assert_eq!(world.npc_combat[&ME].poison_due, Some((P0 + 1 + 75, 9)));
        // The event runs out; the once flag gates the next.
        for n in 1..10 {
            world.process_pulse(P0 + 1 + 75 * n);
        }
        assert!(world.npc_combat[&ME].poison_due.is_none());
        world.poison_npc(Vid::new(1), ME);
        assert!(world.npc_combat[&ME].poison_due.is_none());
        assert!(
            world.npc_affects.get(&ME).is_none_or(|event| {
                event
                    .affects
                    .iter()
                    .all(|affect| affect.kind != AFFECT_POISON)
            }) || world.npc_affect_words(ME)[0] & AFF_POISON_WORD != 0
        );
    }

    /// A poisoned monster's recovery heals nothing and reschedules; death clears the
    /// affects before `GC_DEAD` with no update packet.
    #[test]
    fn a_poisoned_monster_recovers_nothing_and_death_clears_silently() {
        use protocol::gc_actors::GcCharacterUpdate;
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        world.poison_npc(Vid::new(1), ME);
        drained(&mut alpha);
        world.npc_mut(ME).unwrap().hp = 500;
        world.recovery_event(ME);
        assert_eq!(world.npc_by_vid(ME).unwrap().hp, 500);
        assert!(world.npc_combat[&ME].recovery_due.is_some());
        assert!(drained(&mut alpha).is_empty(), "no target broadcast");
        // Death clears without an update: only `GC_DEAD` and the sync release go out.
        world.npc_mut(ME).unwrap().hp = 0;
        world.dead_npc(ME, Some(Vid::new(1)));
        assert!(!world.npc_affects.contains_key(&ME));
        let heard = drained(&mut alpha);
        assert!(
            heard
                .iter()
                .all(|record| GcCharacterUpdate::decode(record).is_err()),
            "no update for the clear: {heard:02x?}"
        );
        assert!(
            heard
                .iter()
                .any(|record| record[0] == HEADER_GC_DEAD.value()),
            "GC_DEAD went out: {heard:02x?}"
        );
    }

    /// The affect tick expires the stun at 4 ticks and the slow at 20: the slow's −30
    /// comes back and the words clear.
    #[test]
    fn the_npc_affect_tick_expires_the_stun_and_the_slow() {
        let (_clock, mut world, mut alpha) = a_fight(a_monster(MAX_HP), NEAR);
        world.target(Vid::new(1), ME);
        drained(&mut alpha);
        world.add_npc_affect(
            ME,
            crate::game_state::affect::Affect {
                kind: AFFECT_STUN,
                apply_on: u8::try_from(point::POINT_NONE).unwrap(),
                value: 0,
                flag: AFF_STUN,
                remaining: STUN_TICKS_NPC,
                sp_cost: 0,
            },
        );
        world.add_npc_affect(
            ME,
            crate::game_state::affect::Affect {
                kind: AFFECT_SLOW,
                apply_on: u8::try_from(point::POINT_MOV_SPEED).unwrap(),
                value: SLOW_MOV_SPEED,
                flag: AFF_SLOW,
                remaining: SLOW_TICKS,
                sp_cost: 0,
            },
        );
        assert!(!world.can_begin_fight(ME));
        for n in 1..=4 {
            world.process_pulse(P0 + 25 * n);
        }
        assert!(world.can_begin_fight(ME), "the stun expired");
        assert_eq!(
            world
                .npc_by_vid(ME)
                .unwrap()
                .fight
                .point(point::POINT_MOV_SPEED),
            -30
        );
        for n in 5..=20 {
            world.process_pulse(P0 + 25 * n);
        }
        assert_eq!(
            world
                .npc_by_vid(ME)
                .unwrap()
                .fight
                .point(point::POINT_MOV_SPEED),
            0
        );
        assert_eq!(world.npc_affect_words(ME), [0, 0]);
        assert!(!world.npc_affects.contains_key(&ME));
    }
    /// Two party members in non-parity split by the NP table: both level 10 (NP 40 each),
    /// so each takes half of the row's grant after the level-delta row (100%).
    #[test]
    fn a_party_row_splits_non_parity_by_the_np_table() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 10, NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        world.party_invite(Vid::new(1), Vid::new(2));
        world.party_invite_answer(Vid::new(2), Vid::new(1), 1);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        drained(&mut beta);
        hit(&mut world, 1, ME);
        // The single party row takes 20 + 80; the two NP-40 halves are 50 each.
        let alpha_frames = drained(&mut alpha);
        let beta_frames = drained(&mut beta);
        assert!(alpha_frames.contains(&exp_change(1, 50, 50)));
        assert!(beta_frames.contains(&exp_change(2, 50, 50)));
        let npc = world.npc_by_vid(ME).unwrap();
        assert_eq!(npc.exp, 0);
    }

    /// Two party members in parity split evenly: `exp / count` each.
    #[test]
    fn a_party_row_splits_parity_evenly() {
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 10, NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        world.party_invite(Vid::new(1), Vid::new(2));
        world.party_invite_answer(Vid::new(2), Vid::new(1), 1);
        world.party_parameter(Vid::new(1), 1);
        drained(&mut alpha);
        drained(&mut beta);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        drained(&mut beta);
        hit(&mut world, 1, ME);
        let alpha_frames = drained(&mut alpha);
        let beta_frames = drained(&mut beta);
        assert!(alpha_frames.contains(&exp_change(1, 50, 50)));
        assert!(beta_frames.contains(&exp_change(2, 50, 50)));
    }

    /// The party bonus multiplier runs only when the victim stands near the leader: two
    /// near members (bonus 12) with the victim beside them take 112% of the row; the
    /// same pair with the victim 6000 from the leader takes 100%.
    #[test]
    fn the_party_multiplier_needs_the_victim_near_the_leader() {
        // Near: the victim at SPOT, leader and member beside it.
        let (_clock, mut world, mut alpha) = a_reward_fight(a_monster(1), 100, 10, NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        world.party_invite(Vid::new(1), Vid::new(2));
        world.party_invite_answer(Vid::new(2), Vid::new(1), 1);
        world.process_pulse(76);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        drained(&mut beta);
        hit(&mut world, 1, ME);
        // 100 * 112 / 100 = 112, halved by the NP split.
        assert!(drained(&mut alpha).contains(&exp_change(1, 56, 56)));
        assert!(drained(&mut beta).contains(&exp_change(2, 56, 56)));
        // Far: Beta leads at EAST, Alpha rows beside the victim 6000 east of the
        // leader. The victim is out of leader range, so no multiplier; Alpha is out of
        // leader range too, so the Update zeroes the nears — but the multiplier reads
        // the victim-near-leader check, which is what this arm pins. To keep the bonus
        // nonzero AND the victim far is impossible in one setup (the bonus needs two
        // nears, the victim near the leader): so this arm pins the geometry — victim
        // far from the leader takes 100% — with the bonus at 0.
        let mut far = a_monster(1);
        far.x = SPOT.0 + 6000;
        far.y = SPOT.1;
        let beside = (SPOT.0 + 5900, SPOT.1);
        let (_clock, mut world, mut alpha) = a_reward_fight(far, 100, 10, beside);
        let mut beta = a_player(&mut world, 2, EAST);
        world.party_invite(Vid::new(2), Vid::new(1));
        world.party_invite_answer(Vid::new(1), Vid::new(2), 1);
        world.process_pulse(76);
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        drained(&mut beta);
        hit(&mut world, 1, ME);
        // No multiplier: the row's 100 splits by NP — but only Alpha stands in victim
        // range, so Alpha takes the whole 100 and Beta nothing.
        assert!(drained(&mut alpha).contains(&exp_change(1, 100, 100)));
        assert!(!drained(&mut beta).contains(&exp_change(2, 100, 100)));
    }
    #[test]
    fn a_party_kill_runs_on_the_leaders_pc_not_the_killers() {
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        let quests = Quests::new(manager, Arc::default(), MobNamesByLanguage::default());
        let mut npc = a_monster(1);
        npc.race = 691;
        npc.exp = 100;
        let clock = TestClock::default();
        clock.set(T0);
        let mut world = a_field_on(&clock, vec![npc], open_cells()).with_quests(quests);
        let mut alpha = a_player(&mut world, 1, NEAR);
        let mut beta = a_player(&mut world, 2, EAST);
        // Beta leads; Alpha rows. Only Alpha clicks, so only Alpha has a quest PC.
        world.party_invite(Vid::new(2), Vid::new(1));
        world.party_invite_answer(Vid::new(1), Vid::new(2), 1);
        world.process_pulse(76);
        world.start_a_quest(Vid::new(1));
        world.end_the_quest(Vid::new(1));
        assert_eq!(world.quest_kill_leader(Vid::new(1)), Some(2));
        world.target(Vid::new(1), ME);
        hit(&mut world, 1, ME);
        drained(&mut alpha);
        drained(&mut beta);
        hit(&mut world, 1, ME);
        assert_eq!(
            world.npc_by_vid(ME).map(|npc| npc.exp),
            Some(0),
            "the kill paid"
        );
        drained(&mut alpha);
        drained(&mut beta);
        // Alpha's kill ran `party_kill` on Beta's PC (`GetPCForce` made it):
        // `guild_create`'s script sits in `run`, so nothing moved — but Beta's PC
        // now exists, which a killer-side `party_kill` would never create.
        let quests = world.quests.as_ref().expect("quests loaded");
        assert_eq!(
            quests.manager().quest_flag(1, b"guild_create.__status"),
            None
        );
        assert_eq!(
            quests.manager().quest_flag(2, b"guild_create.__status"),
            None
        );
        assert!(quests.manager().has_seen(2), "the leader's PC was made");
        // The leader offline: no leader, no `party_kill`.
        world.leave_world(Vid::new(2));
        assert_eq!(world.quest_kill_leader(Vid::new(1)), None);
    }
}
