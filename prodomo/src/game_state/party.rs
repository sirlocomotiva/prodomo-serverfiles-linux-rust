//! Player parties: `CPartyManager` + `CParty` for PCs (`G/party.{h,cpp}`): membership,
//! invites, the Update's near flags, roles with their bonus points, the exp-bonus terms,
//! the distribution parameter, the info packets, the warp summon, and the exp share.
//! `HealParty` is legacy's dead body (`XXX DELETEME` returns first). Ownership rotation
//! is not ported: its only reader is chain lightning, which is not ported.
//!
//! - **Session state only.** Every `HEADER_GD_PARTY_*` DB packet drops: the store keeps
//!   nothing and parties die with the process (ADR-0001: no server-to-server).
//! - **Always enabled.** `m_bEnablePCParty` starts true and no config key flips it, so
//!   the `PERR_SERVER` arm is coded and dead.
//! - **No dungeons, observers or arenas.** Those gates always pass, with a comment at
//!   each. The empire check, the 30-level window, the partyless check and the 8-member
//!   cap are live.
//! - **No block modes.** `BLOCK_PARTY_INVITE`/`BLOCK_PARTY_REQUEST` are unported, so
//!   nobody blocks and the guest-side checks pass.

use std::collections::{BTreeMap, HashMap};

use common::vid::Vid;
use common::CharacterId;
use protocol::cg_party_skill::{PARTY_SKILL_HEAL, PARTY_SKILL_WARP};
use protocol::gc::HEADER_GC_PARTY_UPDATE;
use protocol::gc_fields::{GcNamed, GcTwoWord, GC_NAME_FIELD_SIZE};
use protocol::gc_fields::{HEADER_GC_PARTY_ADD, HEADER_GC_PARTY_LINK, HEADER_GC_PARTY_UNLINK};
use protocol::gc_nested::GcPartyUpdate;
use protocol::gc_nested::GC_PARTY_AFFECT_SLOT_COUNT;
use protocol::gc_small::{GcHeaderAndByte, HEADER_GC_PARTY_PARAMETER};
use protocol::gc_vid::{GcHeaderAndDword, HEADER_GC_PARTY_INVITE, HEADER_GC_PARTY_REMOVE};

use super::warp_npc::SHOW_Z;
use super::GameState;
use crate::sync_position::distance_approx;
use world::character::number;

/// `PARTY_MAX_MEMBER` (`G/party.h:11`): 8.
const PARTY_MAX_MEMBER: usize = 8;
/// The invite's 10 s wait (`G/char.cpp:5896`): 250 Pulses.
const INVITE_WAIT_PULSES: u64 = 250;
/// The Update's 3 s tick (`G/party.cpp:545`): 75 Pulses.
const UPDATE_EVERY_PULSES: u64 = 75;
/// The joinable level window (`G/char.cpp:6041`): 30.
const JOIN_LEVEL_WINDOW: u8 = 30;

/// `PARTY_DEFAULT_RANGE` (`G/party.h:12`): 5000. A member whose
/// `DISTANCE_APPROX` from the leader is strictly below this counts as near
/// (`G/party.cpp:1337`).
pub(super) const PARTY_DEFAULT_RANGE: i32 = 5000;

/// `PARTY_ROLE_NORMAL` (`G/party.h:17`).
const ROLE_NORMAL: u8 = 0;
/// `PARTY_ROLE_LEADER` (`G/party.h:18`).
const ROLE_LEADER: u8 = 1;
/// `PARTY_ROLE_ATTACKER` (`G/party.h:19`).
const ROLE_ATTACKER: u8 = 2;
/// `PARTY_ROLE_TANKER` (`G/party.h:20`).
const ROLE_TANKER: u8 = 3;
/// `PARTY_ROLE_BUFFER` (`G/party.h:21`).
const ROLE_BUFFER: u8 = 4;
/// `PARTY_ROLE_SKILL_MASTER` (`G/party.h:22`).
const ROLE_SKILL_MASTER: u8 = 5;
/// `PARTY_ROLE_HASTE` (`G/party.h:23`).
const ROLE_HASTE: u8 = 6;
/// `PARTY_ROLE_DEFENDER` (`G/party.h:24`).
const ROLE_DEFENDER: u8 = 7;
/// `PARTY_ROLE_MAX_NUM` (`G/party.h:25`): 8.
const ROLE_COUNT: usize = 8;
/// `PARTY_EXP_DISTRIBUTION_NON_PARITY` (`G/party.h:33`): 0.
const DISTRIBUTION_NON_PARITY: u8 = 0;
/// `PARTY_EXP_DISTRIBUTION_PARITY` (`G/party.h:31`): 1.
pub(super) const DISTRIBUTION_PARITY: u8 = 1;
/// `PARTY_EXP_DISTRIBUTION_MAX_NUM` (`G/party.h:35`): 2.
const DISTRIBUTION_MAX: u8 = 2;
/// `PARTY_ENOUGH_MINUTE_FOR_EXP_BONUS` (`G/party.h:13`): 60.
const LONGTIME_MINUTES: u32 = 60;
/// The summon's 12 radius-250 offsets (`G/party.cpp:1110-1124`).
const AROUND: [(i32, i32); 12] = [
    (250, 0),
    (216, 125),
    (125, 216),
    (0, 250),
    (-125, 216),
    (-216, 125),
    (-250, 0),
    (-216, -125),
    (-125, -216),
    (0, -250),
    (125, -216),
    (216, -125),
];

/// Narrow legacy's `int` bonus to the `short` the UPDATE carries, saturating.
fn narrow(value: i32) -> i16 {
    i16::try_from(value).unwrap_or(if value < 0 { i16::MIN } else { i16::MAX })
}

/// `CParty::TMember` (`G/party.h:95-102`) for a PC: the link, the role, the shown level,
/// the name, the near flag.
#[derive(Debug, Clone)]
struct Member {
    /// `pCharacter`'s VID while linked, `None` while offline.
    vid: Option<Vid>,
    /// `bRole`.
    role: u8,
    /// `strName`.
    name: String,
    /// `bNear`, recomputed by the Update.
    near: bool,
}

/// `CParty` (`G/party.h:89-240`) for PCs: PID-keyed members in PID order, the leader, the
/// cached near count, the Update due, the role counts and caps, the cached leadership,
/// the exp-bonus terms, the start clock, and the distribution mode.
#[derive(Debug)]
struct Party {
    /// `m_memberMap`: PID order, like legacy's `std::map`.
    members: BTreeMap<CharacterId, Member>,
    /// `m_iCountNearPartyMember`.
    near_count: usize,
    /// The Pulse the Update next runs on (`m_eventUpdate`'s tick).
    update_due: u64,
    /// `m_anRoleCount`: all 0 at create (the leader joins LEADER without incrementing).
    role_count: [i32; ROLE_COUNT],
    /// `m_anMaxRole`: LEADER 1, NORMAL 32, the rest 0 until the Update sets them.
    role_max: [i32; ROLE_COUNT],
    /// `m_iLeadership`: the leader's skill-121 level, cached by the Update and at the
    /// leader's join.
    leadership: u8,
    /// `m_iExpBonus`.
    exp_bonus: i32,
    /// `m_iLongTimeExpBonus`: 0, once 5.
    longtime_bonus: i32,
    /// `m_dwPartyStartTime`: the world clock at create.
    start_ms: u32,
    /// `m_iExpDistributionMode`.
    mode: u8,
}

/// `CPartyManager` (`G/party.h:37-75`) narrowed to PCs: parties by leader PID, the
/// PID→leader index, and the pending invites.
#[derive(Debug, Default)]
pub(super) struct Parties {
    /// The live parties, by leader PID.
    by_leader: BTreeMap<CharacterId, Party>,
    /// `m_map_pkParty`: each member PID's leader PID.
    of: HashMap<CharacterId, CharacterId>,
    /// The pending invites: `(leader, guest)` → the Pulse the invite lapses on.
    /// Legacy holds one event map per inviter (`m_PartyInviteEventMap`); one map keyed
    /// by both PIDs answers the same questions.
    invites: HashMap<(CharacterId, CharacterId), u64>,
}

/// `CHARACTER::PartyJoinErrCode` (`G/char.h:1106-1118`), without the separator and max.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JoinErr {
    /// `PERR_NONE`.
    None,
    /// `PERR_SERVER`: PC parties disabled — always enabled here, so unreachable.
    Server,
    /// `PERR_LVBOUNDARY`: the levels differ by more than 30.
    LevelBoundary,
    /// `PERR_ALREADYJOIN`: the guest already has a party.
    AlreadyJoin,
    /// `PERR_PARTYISFULL`: the leader's party holds 8.
    Full,
    /// `PERR_DIFFEMPIRE`: the empires differ.
    DiffEmpire,
}

impl GameState {
    /// The leader PID of `pid`'s party, if any.
    pub(super) fn party_leader_of(&self, pid: CharacterId) -> Option<CharacterId> {
        self.parties.of.get(&pid).copied()
    }
    /// `ch->GetParty() != NULL` for the character online under `vid`
    /// (`G/input_main.cpp:959`): its PID holds a party. The descriptor asks before
    /// judging a party chat line.
    pub(super) fn holds_party(&self, vid: Vid) -> bool {
        self.characters
            .find_by_vid(vid)
            .is_ok_and(|character| self.party_leader_of(character.player_id()).is_some())
    }
    /// `ForEachOnlineMember`'s set for the character online under `vid`
    /// (`G/party.h:277-285`): the linked members' VIDs in PID order, the speaker
    /// included. Empty when the character holds no party.
    pub(super) fn party_member_vids(&self, vid: Vid) -> Vec<Vid> {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| self.party_leader_of(character.player_id()))
            .and_then(|leader| self.parties.by_leader.get(&leader))
            .map(|party| {
                party
                    .members
                    .values()
                    .filter_map(|member| member.vid)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `GetLeaderCharacter`'s PID for the killer under `vid` (`G/questmanager.cpp:596-600`):
    /// the party leader's PID when the killer is a linked member whose leader is linked,
    /// and `None` when partyless or the leader is offline (the `party_kill` halves then
    /// run on the killer's own PC, or nowhere).
    pub(super) fn quest_kill_leader(&self, vid: Vid) -> Option<u32> {
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        let leader = self.party_leader_of(pid)?;
        let linked = self
            .parties
            .by_leader
            .get(&leader)
            .and_then(|party| party.members.get(&leader))
            .and_then(|member| member.vid);
        linked.map(|_| leader)
    }

    /// The VID `pid` is online under, if any.
    fn vid_of_pid(&self, pid: CharacterId) -> Option<Vid> {
        self.characters
            .find_by_pid(pid)
            .ok()
            .map(world::character::Character::vid)
    }

    /// `IsPartyJoinableCondition` (`G/char.cpp:6030-6037`): the empire check, then the
    /// mutable conditions.
    fn joinable(&self, leader: CharacterId, guest: CharacterId) -> JoinErr {
        let empire_of = |pid: CharacterId| {
            self.vid_of_pid(pid)
                .and_then(|vid| self.bodies.get(&vid))
                .map(|body| body.card.empire)
        };
        if empire_of(leader) != empire_of(guest) {
            return JoinErr::DiffEmpire;
        }
        self.joinable_mutable(leader, guest)
    }

    /// `IsPartyJoinableMutableCondition` (`G/char.cpp:6045-6066`): the level, joined and
    /// full checks. The enabled arm passes (always enabled), and the dungeon and observer
    /// arms pass (no dungeons, no observer mode — see the module doc), so no `JoinErr`
    /// names them and the `[LS;822]`/`[LS;823]`/`[LS;835]` lines have no sender yet.
    /// `PERR_LOWLEVEL`/`PERR_HILEVEL` never return — legacy's checker has no arm that
    /// yields them either (the answer switches still list them).
    fn joinable_mutable(&self, leader: CharacterId, guest: CharacterId) -> JoinErr {
        let level_of = |pid: CharacterId| {
            self.vid_of_pid(pid)
                .and_then(|vid| self.bodies.get(&vid))
                .map(|body| body.card.level)
        };
        let (Some(leader_level), Some(guest_level)) = (level_of(leader), level_of(guest)) else {
            return JoinErr::Server;
        };
        if leader_level.abs_diff(guest_level) > JOIN_LEVEL_WINDOW {
            return JoinErr::LevelBoundary;
        }
        if self.parties.of.contains_key(&guest) {
            return JoinErr::AlreadyJoin;
        }
        let leader_party = self
            .parties
            .of
            .get(&leader)
            .and_then(|leader| self.parties.by_leader.get(leader));
        if leader_party.is_some_and(|party| party.members.len() >= PARTY_MAX_MEMBER) {
            return JoinErr::Full;
        }
        JoinErr::None
    }
}

impl GameState {
    /// `CPartyManager::CreateParty` (`G/party.cpp:131-164`) for a PC: a leader that
    /// already has a party keeps it; else a party is made, the leader joins and links.
    /// Returns the leader PID.
    pub(super) fn create_party(&mut self, leader: CharacterId) -> CharacterId {
        if let Some(held) = self.party_leader_of(leader) {
            return held;
        }
        let due = self.last_pulse.saturating_add(UPDATE_EVERY_PULSES);
        let start_ms = self.clock.now();
        let mut role_max = [0_i32; ROLE_COUNT];
        role_max[ROLE_LEADER as usize] = 1;
        role_max[ROLE_NORMAL as usize] = 32;
        self.parties.by_leader.insert(
            leader,
            Party {
                members: BTreeMap::new(),
                near_count: 0,
                update_due: due,
                role_count: [0; ROLE_COUNT],
                role_max,
                leadership: 0,
                exp_bonus: 0,
                longtime_bonus: 0,
                start_ms,
                mode: DISTRIBUTION_NON_PARITY,
            },
        );
        self.join_party(leader, leader);
        if let Some(vid) = self.vid_of_pid(leader) {
            self.link_party(vid);
        }
        leader
    }

    /// `CParty::Join` + `P2PJoin` (`G/party.cpp:384-451`, `:453-465`): the PID joins the
    /// leader's party (first member becomes `LEADER`), then the join-one-to-all and the
    /// parameter-to-one go out. Quiet when the party or the guest is gone.
    pub(super) fn join_party(&mut self, leader: CharacterId, pid: CharacterId) {
        let exists = self
            .parties
            .by_leader
            .get(&leader)
            .is_some_and(|party| party.members.contains_key(&pid));
        if exists {
            return;
        }
        let role = self
            .parties
            .by_leader
            .get(&leader)
            .map_or(ROLE_NORMAL, |party| {
                if party.members.is_empty() {
                    ROLE_LEADER
                } else {
                    ROLE_NORMAL
                }
            });
        let name = self
            .vid_of_pid(pid)
            .and_then(|vid| self.bodies.get(&vid))
            .map_or_else(String::new, |body| body.card.name.clone());
        let Some(party) = self.parties.by_leader.get_mut(&leader) else {
            return;
        };
        party.members.insert(
            pid,
            Member {
                vid: None,
                role,
                name,
                near: false,
            },
        );
        self.parties.of.insert(pid, leader);
        // `P2PJoin`'s leadership seed (`G/party.cpp:219-220`): the leader's own join
        // caches its skill-121 level.
        if role == ROLE_LEADER {
            let level = self
                .vid_of_pid(pid)
                .map_or(0, |vid| self.leadership_level(vid));
            if let Some(party) = self.parties.by_leader.get_mut(&leader) {
                party.leadership = level;
            }
        }
        self.send_party_join_one_to_all(leader, pid);
        if let Some(vid) = self.vid_of_pid(pid) {
            self.send_party_parameter(vid);
        }
    }

    /// `CParty::Link` (`G/party.cpp:536-596`): the VID attaches to its PID's party and the
    /// six sends go out in legacy order, then the parameter. Quiet when partyless.
    pub(super) fn link_party(&mut self, vid: Vid) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let Some(leader) = self.party_leader_of(pid) else {
            return;
        };
        let name_missing = self
            .parties
            .by_leader
            .get(&leader)
            .and_then(|party| party.members.get(&pid))
            .is_some_and(|member| member.vid.is_none() || member.name.is_empty());
        let fresh_name = if name_missing {
            self.bodies.get(&vid).map(|body| body.card.name.clone())
        } else {
            None
        };
        if let Some(member) = self
            .parties
            .by_leader
            .get_mut(&leader)
            .and_then(|party| party.members.get_mut(&pid))
        {
            member.vid = Some(vid);
            if member.name.is_empty() {
                if let Some(name) = fresh_name {
                    member.name = name;
                }
            }
        }
        self.send_party_join_one_to_all(leader, pid);
        self.send_party_join_all_to_one(vid, leader);
        self.send_party_link_one_to_all(leader, vid, pid);
        self.send_party_link_all_to_one(vid, leader);
        self.send_party_info_all_to_one(vid, leader);
        self.send_party_info_one_to_all(leader, pid);
        self.send_party_parameter(vid);
    }

    /// `CParty::Quit` + `P2PQuit` (`G/party.cpp:467-535`): the remove-one-to-all goes out,
    /// the quitter's role points zero (`RemoveBonusForOne`), its role count drops, the
    /// PID detaches and unindexes; a leader's quit deletes the party, which sends
    /// nothing further (legacy's `DeleteParty` broadcasts no remove — the members'
    /// clients drop the party when their own link ends; see `unlink_party`).
    pub(super) fn quit_party(&mut self, pid: CharacterId) {
        let Some(leader) = self.party_leader_of(pid) else {
            return;
        };
        let (role, vid) = self
            .parties
            .by_leader
            .get(&leader)
            .and_then(|party| party.members.get(&pid))
            .map_or((ROLE_NORMAL, None), |member| (member.role, member.vid));
        self.send_party_remove_one_to_all(leader, pid);
        // `RemoveBonusForOne` (`G/party.cpp:1041`) feeds the refresh through
        // `ComputePoints`; the count decrement has no floor (`:523-524`), so a
        // count the unset mismatch drove wrong can go negative.
        if let Some(vid) = vid {
            self.zero_role_points(vid, true);
        }
        if usize::from(role) < ROLE_COUNT {
            if let Some(party) = self.parties.by_leader.get_mut(&leader) {
                party.role_count[usize::from(role)] -= 1;
            }
        }
        if let Some(party) = self.parties.by_leader.get_mut(&leader) {
            party.members.remove(&pid);
        }
        self.parties.of.remove(&pid);
        if role == ROLE_LEADER {
            self.parties.by_leader.remove(&leader);
            for orphan in self
                .parties
                .of
                .iter()
                .filter_map(|(member, held)| (*held == leader).then_some(*member))
                .collect::<Vec<_>>()
            {
                self.parties.of.remove(&orphan);
            }
        }
    }

    /// `CParty::Unlink` (`G/party.cpp:632-670`): the unlink-one-to-all goes out and the VID
    /// detaches; the PID stays a member. When the unlinked member holds LEADER,
    /// `RemoveBonus` runs: every linked member's six role points zero through
    /// `ComputePoints` and every member's near flag clears. Quiet when the VID holds no
    /// party. The dungeon `FExitDungeon` arm is not ported (no dungeons).
    pub(super) fn unlink_party(&mut self, vid: Vid) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let Some(leader) = self.party_leader_of(pid) else {
            return;
        };
        let is_leader = pid == leader;
        self.send_party_unlink_one_to_all(leader, vid, pid);
        if let Some(member) = self
            .parties
            .by_leader
            .get_mut(&leader)
            .and_then(|party| party.members.get_mut(&pid))
        {
            member.vid = None;
        }
        if !is_leader {
            return;
        }
        let linked: Vec<Vid> = self
            .parties
            .by_leader
            .get(&leader)
            .map(|party| {
                party
                    .members
                    .values()
                    .filter_map(|member| member.vid)
                    .collect()
            })
            .unwrap_or_default();
        for other in linked {
            self.zero_role_points(other, true);
        }
        if let Some(party) = self.parties.by_leader.get_mut(&leader) {
            for member in party.members.values_mut() {
                member.near = false;
            }
        }
    }

    /// `CParty::Update` (`G/party.cpp:1308-1443`): the near flags, the leadership cache,
    /// the exp-bonus recompute with its resend, the once-only long-time +5, each
    /// online member's role points, the near infos, and the role caps. A leaderless
    /// party takes the `RemoveBonuses` arm instead. No map check: legacy compares raw
    /// x/y across maps too. The attack/defence party bonuses are not stored: both
    /// computes are commented-out `return 0` and no live reader of the getters exists.
    /// The heal-ready block is skipped: its only send is `if (0)` DELETEME'd and its
    /// flag feeds only the dead `HealParty`.
    pub(super) fn update_party_nears(&mut self, leader: CharacterId) {
        let Some(party) = self.parties.by_leader.get(&leader) else {
            return;
        };
        let Some(leader_vid) = party.members.get(&leader).and_then(|member| member.vid) else {
            self.remove_party_bonuses(leader);
            return;
        };
        let Some(leader_spot) = self.spot_of(leader_vid.raw()) else {
            self.remove_party_bonuses(leader);
            return;
        };
        // Two passes: the spot reads borrow the world, the flag writes borrow the party.
        let mut flags: Vec<(CharacterId, bool)> = Vec::new();
        if let Some(party) = self.parties.by_leader.get(&leader) {
            for (pid, member) in &party.members {
                let close = member
                    .vid
                    .and_then(|vid| self.spot_of(vid.raw()))
                    .is_some_and(|spot| {
                        distance_approx(
                            leader_spot.x.saturating_sub(spot.x),
                            leader_spot.y.saturating_sub(spot.y),
                        ) < PARTY_DEFAULT_RANGE
                    });
                flags.push((*pid, close));
            }
        }
        let mut near = flags.iter().filter(|(_, close)| *close).count();
        if near <= 1 {
            for (_, close) in &mut flags {
                *close = false;
            }
            near = 0;
        }
        let mut resend = self
            .parties
            .by_leader
            .get(&leader)
            .is_some_and(|party| party.near_count != near);
        if let Some(party) = self.parties.by_leader.get_mut(&leader) {
            for (pid, close) in &flags {
                if let Some(member) = party.members.get_mut(pid) {
                    member.near = *close;
                }
            }
            party.near_count = near;
        }
        // `m_iLeadership` refreshes before the bonus computes read it.
        let leadership = self.leadership_level(leader_vid);
        if let Some(party) = self.parties.by_leader.get_mut(&leader) {
            party.leadership = leadership;
        }
        let exp = self.compute_exp_bonus(leader);
        if let Some(party) = self.parties.by_leader.get_mut(&leader) {
            if party.exp_bonus != exp {
                party.exp_bonus = exp;
                resend = true;
            }
        }
        // The long-time arm fires once: the flag is the bonus itself.
        let elapsed = self.clock.now().wrapping_sub(self.party_start_ms(leader));
        let longtime_due =
            elapsed > LONGTIME_MINUTES * 60 * 1000 && self.party_longtime_bonus(leader) == 0;
        if longtime_due {
            if let Some(party) = self.parties.by_leader.get_mut(&leader) {
                party.longtime_bonus = 5;
            }
            resend = true;
            for vid in self.online_party_vids(leader) {
                self.info_to(vid, b"[LS;926]", &[]);
            }
        }
        // Each online member's role points, at its cached near (`:1408-1425`); near
        // members hear their info unless the resend covers them.
        for (pid, vid, role, close) in self.online_party_roles(leader) {
            self.compute_role_point(vid, role, close);
            if close && !resend {
                self.send_party_info_one_to_all(leader, pid);
            }
        }
        self.apply_role_caps(leader);
        if !resend {
            return;
        }
        for vid in self.online_party_vids(leader) {
            let Ok(character) = self.characters.find_by_vid(vid) else {
                continue;
            };
            self.send_party_info_one_to_all(leader, character.player_id());
        }
    }

    /// `CParty::RemoveBonuses` (`G/party.cpp:1361-1370`): the leaderless Update's arm.
    /// The exp bonus recomputes over the leaderless party, and when it changed every
    /// online member's six role points zero (through `ComputeBattlePoints`) and hears
    /// its info. Nothing else resends: the role points change silently otherwise.
    fn remove_party_bonuses(&mut self, leader: CharacterId) {
        let exp = self.compute_exp_bonus(leader);
        let changed = self
            .parties
            .by_leader
            .get(&leader)
            .is_some_and(|party| party.exp_bonus != exp);
        if let Some(party) = self.parties.by_leader.get_mut(&leader) {
            party.exp_bonus = exp;
        }
        if !changed {
            return;
        }
        for (pid, vid, _role, _close) in self.online_party_roles(leader) {
            self.zero_role_points(vid, false);
            self.send_party_info_one_to_all(leader, pid);
        }
    }

    /// `CParty::ComputePartyBonusExpPercent` (`G/party.cpp:1694-1715`) with the
    /// upgrade-party table (`G/party.h:170-174`): 0 while one member or fewer is near,
    /// else the `min(8, near)` row — 12, 18, 26, 40, 53, 70, 100 for 2 through 8 —
    /// plus 30 while the leader wears 70003, 71012 or 76011. The unique-group half
    /// of the +30 is coded and dead: the owner's only group is `Vnum 50011`, above
    /// the 30000 gate, so the `m_ItemToSpecialGroup` map never fills.
    fn compute_exp_bonus(&self, leader: CharacterId) -> i32 {
        const TABLE: [i32; 9] = [0, 0, 12, 18, 26, 40, 53, 70, 100];
        const UNIQUE_BONUS: [u32; 3] = [70_003, 71_012, 76_011];
        let Some(party) = self.parties.by_leader.get(&leader) else {
            return 0;
        };
        let near = party.near_count.min(8);
        if near <= 1 {
            return 0;
        }
        let mut bonus = TABLE[near];
        if party
            .members
            .get(&leader)
            .and_then(|member| member.vid)
            .is_some_and(|vid| {
                UNIQUE_BONUS
                    .into_iter()
                    .any(|vnum| self.wears_unique(vid, vnum))
            })
        {
            bonus += 30;
        }
        // `IsEquipUniqueGroup(10010)`: dead against the owner's data (see the doc).
        bonus
    }

    /// The role caps (`G/party.cpp:1427-1441`, `PARTY_ROLE_LIMIT_LEVEL_BUG_FIX`
    /// defined): `ATTACKER` at 10, `HASTE` and `TANKER` at 20, `BUFFER` at 25,
    /// `SKILL_MASTER` at 35, `DEFENDER` at 40 — and a second `ATTACKER` at 40.
    fn apply_role_caps(&mut self, leader: CharacterId) {
        let leadership = self
            .parties
            .by_leader
            .get(&leader)
            .map_or(0, |party| party.leadership);
        if let Some(party) = self.parties.by_leader.get_mut(&leader) {
            let max = &mut party.role_max;
            max[ROLE_ATTACKER as usize] = i32::from(leadership >= 10) + i32::from(leadership >= 40);
            max[ROLE_HASTE as usize] = i32::from(leadership >= 20);
            max[ROLE_TANKER as usize] = i32::from(leadership >= 20);
            max[ROLE_BUFFER as usize] = i32::from(leadership >= 25);
            max[ROLE_SKILL_MASTER as usize] = i32::from(leadership >= 35);
            max[ROLE_DEFENDER as usize] = i32::from(leadership >= 40);
        }
    }

    /// The online members' `(pid, vid, role, near)` in PID order.
    fn online_party_roles(&self, leader: CharacterId) -> Vec<(CharacterId, Vid, u8, bool)> {
        self.parties
            .by_leader
            .get(&leader)
            .map(|party| {
                party
                    .members
                    .iter()
                    .filter_map(|(pid, member)| {
                        member.vid.map(|vid| (*pid, vid, member.role, member.near))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The online members' VIDs in PID order.
    fn online_party_vids(&self, leader: CharacterId) -> Vec<Vid> {
        self.parties
            .by_leader
            .get(&leader)
            .map(|party| {
                party
                    .members
                    .values()
                    .filter_map(|member| member.vid)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `m_dwPartyStartTime` of the leader's party, 0 when gone.
    fn party_start_ms(&self, leader: CharacterId) -> u32 {
        self.parties
            .by_leader
            .get(&leader)
            .map_or(0, |party| party.start_ms)
    }

    /// `m_iLongTimeExpBonus` of the leader's party, 0 when gone.
    fn party_longtime_bonus(&self, leader: CharacterId) -> i32 {
        self.parties
            .by_leader
            .get(&leader)
            .map_or(0, |party| party.longtime_bonus)
    }

    /// `CHARACTER::GetLeadershipSkillLevel` (`G/char.cpp:7314-7320`): the skill-121
    /// level of the player under `vid`.
    fn leadership_level(&self, vid: Vid) -> u8 {
        self.pc_skill_level(vid.raw(), 121)
    }

    /// The leader PID of the party the player under `vid` belongs to, if any.
    pub(super) fn party_of_vid(&self, vid: Vid) -> Option<CharacterId> {
        self.pid_of(vid).and_then(|pid| self.party_leader_of(pid))
    }

    /// `m_iExpDistributionMode` of the leader's party, non-parity when gone.
    pub(super) fn party_mode(&self, leader: CharacterId) -> u8 {
        self.parties
            .by_leader
            .get(&leader)
            .map_or(DISTRIBUTION_NON_PARITY, |party| party.mode)
    }

    /// `GetExpBonusPercent` (`G/party.cpp:1548-1554`): 0 when one or fewer members are
    /// near, else the exp bonus plus the longtime bonus.
    pub(super) fn party_exp_bonus_percent(&self, leader: CharacterId) -> i32 {
        self.parties.by_leader.get(&leader).map_or(0, |party| {
            if party.near_count <= 1 {
                0
            } else {
                party.exp_bonus + party.longtime_bonus
            }
        })
    }

    /// `IsPositionNearLeader` (`G/party.cpp:1536-1545`): the leader is linked and the
    /// point is strictly below 5000 from it (the `>=` refuses).
    pub(super) fn party_victim_near_leader(&self, leader: CharacterId, x: i32, y: i32) -> bool {
        let linked = self
            .parties
            .by_leader
            .get(&leader)
            .and_then(|party| party.members.get(&leader))
            .and_then(|member| member.vid);
        let Some(leader_vid) = linked else {
            return false;
        };
        let Some(spot) = self.spot_of(leader_vid.raw()) else {
            return false;
        };
        distance_approx(x.saturating_sub(spot.x), y.saturating_sub(spot.y)) < PARTY_DEFAULT_RANGE
    }

    /// `ForEachOnlineMember` with the victim-distance check (`:2710-2716`, `:2738-2742`):
    /// the online members within 5000 (inclusive) of `(x, y)`, in PID order, each with
    /// its card level.
    pub(super) fn party_members_near_victim(
        &self,
        leader: CharacterId,
        x: i32,
        y: i32,
    ) -> Vec<(Vid, u8)> {
        let Some(party) = self.parties.by_leader.get(&leader) else {
            return Vec::new();
        };
        party
            .members
            .values()
            .filter_map(|member| member.vid)
            .filter(|vid| {
                self.spot_of(vid.raw()).is_some_and(|spot| {
                    distance_approx(x.saturating_sub(spot.x), y.saturating_sub(spot.y))
                        <= PARTY_DEFAULT_RANGE
                })
            })
            .map(|vid| {
                let level = self.bodies.get(&vid).map_or(0, |body| body.card.level);
                (vid, level)
            })
            .collect()
    }
}

impl GameState {
    /// The 25-byte name field (`strlcpy` into `char[25]`): the bytes, NUL-padded.
    fn party_name_field(name: &str) -> [u8; GC_NAME_FIELD_SIZE] {
        let mut field = [0u8; GC_NAME_FIELD_SIZE];
        let bytes = name.as_bytes();
        let len = bytes.len().min(GC_NAME_FIELD_SIZE);
        field[..len].copy_from_slice(&bytes[..len]);
        field
    }

    /// `SendPartyJoinOneToAll` (`G/party.cpp:687-702`): the ADD of `pid` to every online
    /// member of the leader's party.
    fn send_party_join_one_to_all(&self, leader: CharacterId, pid: CharacterId) {
        let Some(party) = self.parties.by_leader.get(&leader) else {
            return;
        };
        let Some(member) = party.members.get(&pid) else {
            return;
        };
        let mut record = Vec::new();
        GcNamed::new(
            HEADER_GC_PARTY_ADD,
            pid,
            Self::party_name_field(&member.name),
        )
        .encode_into(&mut record);
        for vid in party.members.values().filter_map(|member| member.vid) {
            let _sent = self.write_to_client(vid, record.clone());
        }
    }

    /// `SendPartyJoinAllToOne` (`G/party.cpp:704-720`): every member's ADD to `vid`, in
    /// PID order.
    fn send_party_join_all_to_one(&self, vid: Vid, leader: CharacterId) {
        let Some(party) = self.parties.by_leader.get(&leader) else {
            return;
        };
        for (pid, member) in &party.members {
            let mut record = Vec::new();
            GcNamed::new(
                HEADER_GC_PARTY_ADD,
                *pid,
                Self::party_name_field(&member.name),
            )
            .encode_into(&mut record);
            let _sent = self.write_to_client(vid, record);
        }
    }

    /// `SendPartyRemoveOneToAll` (`G/party.cpp:672-685`): the REMOVE of `pid` to every
    /// online member of the leader's party.
    fn send_party_remove_one_to_all(&self, leader: CharacterId, pid: CharacterId) {
        let Some(party) = self.parties.by_leader.get(&leader) else {
            return;
        };
        let mut record = Vec::new();
        GcHeaderAndDword::new(HEADER_GC_PARTY_REMOVE, pid).encode_into(&mut record);
        for vid in party.members.values().filter_map(|member| member.vid) {
            let _sent = self.write_to_client(vid, record.clone());
        }
    }

    /// `SendPartyLinkOneToAll` (`G/party.cpp:743-762`): the LINK of `vid`/`pid` to every
    /// online member. Quiet when the linker has no descriptor — every linked VID here
    /// has one, so the gate passes.
    fn send_party_link_one_to_all(&self, leader: CharacterId, vid: Vid, pid: CharacterId) {
        let Some(party) = self.parties.by_leader.get(&leader) else {
            return;
        };
        let mut record = Vec::new();
        GcTwoWord::new(HEADER_GC_PARTY_LINK, pid, vid.raw()).encode_into(&mut record);
        for held in party.members.values().filter_map(|member| member.vid) {
            let _sent = self.write_to_client(held, record.clone());
        }
    }

    /// `SendPartyLinkAllToOne` (`G/party.cpp:764-783`): every online member's LINK to
    /// `vid`, in PID order.
    fn send_party_link_all_to_one(&self, vid: Vid, leader: CharacterId) {
        let Some(party) = self.parties.by_leader.get(&leader) else {
            return;
        };
        for (pid, member) in &party.members {
            let Some(held) = member.vid else {
                continue;
            };
            let mut record = Vec::new();
            GcTwoWord::new(HEADER_GC_PARTY_LINK, *pid, held.raw()).encode_into(&mut record);
            let _sent = self.write_to_client(vid, record);
        }
    }

    /// `SendPartyUnlinkOneToAll` (`G/party.cpp:722-741`): the UNLINK of `vid`/`pid` to
    /// every online member (sent as a `TPacketGCPartyLink` with the header changed).
    fn send_party_unlink_one_to_all(&self, leader: CharacterId, vid: Vid, pid: CharacterId) {
        let Some(party) = self.parties.by_leader.get(&leader) else {
            return;
        };
        let mut record = Vec::new();
        GcTwoWord::new(HEADER_GC_PARTY_UNLINK, pid, vid.raw()).encode_into(&mut record);
        for held in party.members.values().filter_map(|member| member.vid) {
            let _sent = self.write_to_client(held, record.clone());
        }
    }

    /// `SendPartyInfoOneToAll` (`G/party.cpp:785-834`): `pid`'s UPDATE to every online
    /// member — the built packet while linked, the zeroed 255-HP one while offline.
    fn send_party_info_one_to_all(&self, leader: CharacterId, pid: CharacterId) {
        let packet = self.build_party_update(leader, pid);
        let Some(party) = self.parties.by_leader.get(&leader) else {
            return;
        };
        let mut record = Vec::new();
        packet.encode_into(&mut record);
        for vid in party.members.values().filter_map(|member| member.vid) {
            let _sent = self.write_to_client(vid, record.clone());
        }
    }

    /// `SendPartyInfoAllToOne` (`G/party.cpp:837-860`): every member's UPDATE to `vid`,
    /// in PID order.
    fn send_party_info_all_to_one(&self, vid: Vid, leader: CharacterId) {
        let pids: Vec<CharacterId> = self
            .parties
            .by_leader
            .get(&leader)
            .map(|party| party.members.keys().copied().collect())
            .unwrap_or_default();
        for pid in pids {
            let mut record = Vec::new();
            self.build_party_update(leader, pid)
                .encode_into(&mut record);
            let _sent = self.write_to_client(vid, record);
        }
    }

    /// `CHARACTER::BuildUpdatePartyPacket` (`G/char.cpp:7282-7311`): the UPDATE — pid,
    /// `hp*100/maxhp` clamped 0-100 (0 when the max is 0), the role — with the affects
    /// filled while the member stands within 5000 of the linked leader (a live
    /// distance read, not the cached near flag): `m_iExpBonus` first — the table-plus-30
    /// term alone, not the long-time +5 — then the member's six role points. An
    /// offline member reads 255 HP.
    fn build_party_update(&self, leader: CharacterId, pid: CharacterId) -> GcPartyUpdate {
        use common::point_slot as point;
        let role = self
            .parties
            .by_leader
            .get(&leader)
            .and_then(|party| party.members.get(&pid))
            .map_or(ROLE_NORMAL, |member| member.role);
        let blank = GcPartyUpdate {
            header: HEADER_GC_PARTY_UPDATE,
            pid,
            state: role,
            percent_hp: 255,
            affects: [0; GC_PARTY_AFFECT_SLOT_COUNT],
        };
        let members = self
            .parties
            .by_leader
            .get(&leader)
            .map_or((0, None, None), |party| {
                (
                    party.exp_bonus,
                    party.members.get(&pid).and_then(|member| member.vid),
                    party.members.get(&leader).and_then(|member| member.vid),
                )
            });
        let (exp_bonus, member_vid, leader_vid) = members;
        let Some(vid) = member_vid else {
            return blank;
        };
        let (hp, max_hp) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or((0, 0), |points| (points.hp(), points.max_hp()));
        let percent_hp = if max_hp <= 0 {
            0
        } else {
            u8::try_from((hp * 100 / max_hp).clamp(0, 100)).unwrap_or(u8::MAX)
        };
        // The live gate, not the cached flag: the member within 5000 of the leader.
        let gated = leader_vid
            .and_then(|held| self.spot_of(held.raw()))
            .zip(self.spot_of(vid.raw()))
            .is_some_and(|(leader_spot, spot)| {
                distance_approx(
                    leader_spot.x.saturating_sub(spot.x),
                    leader_spot.y.saturating_sub(spot.y),
                ) < PARTY_DEFAULT_RANGE
            });
        let mut affects = [0_i16; GC_PARTY_AFFECT_SLOT_COUNT];
        if gated {
            let points = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.points());
            let at = |slot| points.map_or(0, |points| points.get_point(slot));
            // Legacy's int→short assignment truncates; the bonuses fit, so the
            // narrow reproduces it exactly on live values.
            affects[0] = narrow(exp_bonus);
            affects[1] = narrow(at(point::POINT_PARTY_ATTACKER_BONUS));
            affects[2] = narrow(at(point::POINT_PARTY_TANKER_BONUS));
            affects[3] = narrow(at(point::POINT_PARTY_BUFFER_BONUS));
            affects[4] = narrow(at(point::POINT_PARTY_SKILL_MASTER_BONUS));
            affects[5] = narrow(at(point::POINT_PARTY_HASTE_BONUS));
            affects[6] = narrow(at(point::POINT_PARTY_DEFENDER_BONUS));
        }
        GcPartyUpdate {
            header: HEADER_GC_PARTY_UPDATE,
            pid,
            state: role,
            percent_hp,
            affects,
        }
    }

    /// `SendParameter` (`G/party.cpp:1601-1614`): the member's party mode byte to `vid`.
    /// Partyless is 0 — the only mode legacy's default constructor knows.
    fn send_party_parameter(&self, vid: Vid) {
        let mode = self
            .pid_of(vid)
            .and_then(|pid| self.party_leader_of(pid))
            .and_then(|leader| self.parties.by_leader.get(&leader))
            .map_or(DISTRIBUTION_NON_PARITY, |party| party.mode);
        let mut record = Vec::new();
        GcHeaderAndByte::new(HEADER_GC_PARTY_PARAMETER, mode).encode_into(&mut record);
        let _sent = self.write_to_client(vid, record);
    }
}

impl GameState {
    /// `CInputMain::PartyInvite` (`G/input_main.cpp:2476-2499`): the arena gate passes
    /// (no arenas), unknown or descriptor-less VIDs stay quiet, and `PartyInvite`
    /// runs. The invite and answer handlers take VIDs, like legacy's `Find`.
    pub(super) fn party_invite(&mut self, inviter: Vid, guest: Vid) {
        if !self.is_online(inviter) || !self.is_online(guest) {
            return;
        }
        self.character_party_invite(inviter, guest);
    }

    /// `CHARACTER::PartyInvite` (`G/char.cpp:5839-5908`): the non-leader line, the
    /// block-mode line (unported — never blocks, the check passes with a comment),
    /// the joinable gates, the dup-invite quiet return, the 250-Pulse invite event,
    /// and the INVITE to the guest.
    fn character_party_invite(&mut self, inviter: Vid, guest: Vid) {
        let (Some(inviter_pid), Some(guest_pid)) = (self.pid_of(inviter), self.pid_of(guest))
        else {
            return;
        };
        if self
            .party_leader_of(inviter_pid)
            .is_some_and(|leader| leader != inviter_pid)
        {
            self.info_to(inviter, b"[LS;832]", &[]);
            return;
        }
        // `BLOCK_PARTY_INVITE`: block modes are unported, so nobody blocks.
        match self.joinable(inviter_pid, guest_pid) {
            JoinErr::None => {}
            JoinErr::Server => {
                self.info_to(inviter, b"[LS;530]", &[]);
                return;
            }
            JoinErr::DiffEmpire => {
                self.info_to(inviter, b"[LS;821]", &[]);
                return;
            }
            JoinErr::LevelBoundary => {
                self.info_to(inviter, b"[LS;824]", &[]);
                return;
            }
            JoinErr::AlreadyJoin => {
                self.info_to(
                    inviter,
                    b"[LS;834;%s]",
                    &[crate::chat_line::Arg::Text(&self.name_arg(guest_pid))],
                );
                return;
            }
            JoinErr::Full => {
                self.info_to(inviter, b"[LS;828]", &[]);
                return;
            }
        }
        if self.parties.invites.contains_key(&(inviter_pid, guest_pid)) {
            return;
        }
        let due = self.last_pulse.saturating_add(INVITE_WAIT_PULSES);
        self.parties.invites.insert((inviter_pid, guest_pid), due);
        let mut record = Vec::new();
        GcHeaderAndDword::new(HEADER_GC_PARTY_INVITE, inviter.raw()).encode_into(&mut record);
        let _sent = self.write_to_client(guest, record);
    }

    /// `CInputMain::PartyInviteAnswer` (`G/input_main.cpp:2501-2519`): the arena gate
    /// passes; a gone inviter chats `[LS;668]`; deny runs `PartyInviteDeny`, accept
    /// runs `PartyInviteAccept`.
    pub(super) fn party_invite_answer(&mut self, guest: Vid, inviter: Vid, accept: u8) {
        if !self.is_online(guest) {
            return;
        }
        if !self.is_online(inviter) {
            self.info_to(guest, b"[LS;668]", &[]);
            return;
        }
        if accept == 0 {
            self.party_invite_deny(inviter, guest);
        } else {
            self.party_invite_accept(inviter, guest);
        }
    }

    /// `CHARACTER::PartyInviteAccept` (`G/char.cpp:5910-5969`): a guest with no invite
    /// stays quiet; the event cancels; a leader that lost the party chats `[LS;832]`;
    /// the mutable gates run (the empire does not recheck — only the mutable
    /// conditions); then the guest joins the live party, or a party is made and the
    /// guest joins, links, and the inviter hears every member's info. The
    /// LOWLEVEL/HILEVEL arms are unreachable: the checker never returns them.
    fn party_invite_accept(&mut self, inviter: Vid, guest: Vid) {
        let (Some(inviter_pid), Some(guest_pid)) = (self.pid_of(inviter), self.pid_of(guest))
        else {
            return;
        };
        if self
            .parties
            .invites
            .remove(&(inviter_pid, guest_pid))
            .is_none()
        {
            return;
        }
        if self
            .party_leader_of(inviter_pid)
            .is_some_and(|leader| leader != inviter_pid)
        {
            self.info_to(inviter, b"[LS;832]", &[]);
            return;
        }
        match self.joinable_mutable(inviter_pid, guest_pid) {
            JoinErr::None => {}
            JoinErr::Server => {
                self.info_to(guest, b"[LS;530]", &[]);
                return;
            }
            JoinErr::LevelBoundary => {
                self.info_to(guest, b"[LS;824]", &[]);
                return;
            }
            JoinErr::AlreadyJoin => {
                self.info_to(guest, b"[LS;837]", &[]);
                return;
            }
            JoinErr::Full => {
                self.info_to(inviter, b"[LS;828]", &[]);
                self.info_to(guest, b"[LS;831]", &[]);
                return;
            }
            JoinErr::DiffEmpire => {
                return;
            }
        }
        if self.party_leader_of(inviter_pid).is_some() {
            self.character_party_join(inviter, guest);
        } else {
            let leader = self.create_party(inviter_pid);
            self.join_party(leader, guest_pid);
            self.link_party(guest);
            self.send_party_info_all_to_one(inviter, leader);
        }
    }

    /// `CHARACTER::PartyInviteDeny` (`G/char.cpp:5997-6012`): a guest with no invite
    /// stays quiet; the event cancels; the inviter chats `[LS;838;%s]`. The timeout
    /// event calls this too, so expiry tells the inviter as well.
    fn party_invite_deny(&mut self, inviter: Vid, guest: Vid) {
        let (Some(inviter_pid), Some(guest_pid)) = (self.pid_of(inviter), self.pid_of(guest))
        else {
            return;
        };
        if self
            .parties
            .invites
            .remove(&(inviter_pid, guest_pid))
            .is_none()
        {
            return;
        }
        self.info_to(
            inviter,
            b"[LS;838;%s]",
            &[crate::chat_line::Arg::Text(&self.name_arg(guest_pid))],
        );
    }

    /// `CHARACTER::PartyJoin` (`G/char.cpp:6014-6021`): the two `[LS;839;%s]` lines,
    /// then the guest joins and links the inviter's live party.
    fn character_party_join(&mut self, inviter: Vid, guest: Vid) {
        let (Some(inviter_pid), Some(guest_pid)) = (self.pid_of(inviter), self.pid_of(guest))
        else {
            return;
        };
        let Some(leader) = self.party_leader_of(inviter_pid) else {
            return;
        };
        self.info_to(
            inviter,
            b"[LS;839;%s]",
            &[crate::chat_line::Arg::Text(&self.name_arg(guest_pid))],
        );
        self.info_to(
            guest,
            b"[LS;839;%s]",
            &[crate::chat_line::Arg::Text(&self.name_arg(inviter_pid))],
        );
        self.join_party(leader, guest_pid);
        self.link_party(guest);
    }

    /// The PID online under `vid`, if any.
    fn pid_of(&self, vid: Vid) -> Option<CharacterId> {
        self.characters
            .find_by_vid(vid)
            .ok()
            .map(world::character::Character::player_id)
    }

    /// The guest's name as a chat arg, empty when offline.
    fn name_arg(&self, pid: CharacterId) -> Vec<u8> {
        self.vid_of_pid(pid)
            .and_then(|vid| self.bodies.get(&vid))
            .map_or_else(Vec::new, |body| body.card.name.as_bytes().to_vec())
    }

    /// `ChatPacket(CHAT_TYPE_INFO, format, ...)` to the player under `vid`.
    fn info_to(&self, vid: Vid, format: &[u8], args: &[crate::chat_line::Arg<'_>]) {
        if let Some(mover) = self.own_mover(vid) {
            self.tell(vid, mover, protocol::gc_chat::CHAT_TYPE_INFO, format, args);
        }
    }
}

impl GameState {
    /// `CInputMain::PartyRemove` (`G/input_main.cpp:2578-2675`): the arena and PC-party
    /// gates pass (no arenas, always enabled), the dungeon arms pass (no dungeons;
    /// legacy checks the dungeon twice and the 351 check once — all pass), then the
    /// leader and member arms run. A gone or partyless sender stays quiet.
    pub(super) fn party_remove(&mut self, sender: Vid, pid: CharacterId) {
        let Some(self_pid) = self.pid_of(sender) else {
            return;
        };
        let Some(leader) = self.party_leader_of(self_pid) else {
            return;
        };
        // `ch->GetDungeon()` ×2 and `IsPartyInDungeon(351)`: no dungeons, so pass.
        if leader == self_pid {
            self.party_remove_as_leader(sender, leader, self_pid, pid);
        } else {
            self.party_remove_as_member(sender, leader, self_pid, pid);
        }
    }

    /// The leader's arm: removing itself or from a party of two disbands (which sends
    /// nothing — see `delete_party`); else the removed member, when online, chats
    /// `[LS;675]` and quits. (`Quit` on a non-member sends its remove anyway —
    /// `SendPartyRemoveOneToAll` runs before the map lookup.)
    fn party_remove_as_leader(
        &mut self,
        sender: Vid,
        leader: CharacterId,
        self_pid: CharacterId,
        pid: CharacterId,
    ) {
        let _ = sender;
        let count = self
            .parties
            .by_leader
            .get(&leader)
            .map_or(0, |party| party.members.len());
        if pid == self_pid || count == 2 {
            self.delete_party(leader);
            return;
        }
        if let Some(vid) = self.vid_of_pid(pid) {
            self.info_to(vid, b"[LS;675]", &[]);
        }
        self.quit_party(pid);
    }

    /// The member's arm: removing another chats `[LS;677]`; removing itself from a
    /// party of two disbands; else it chats `[LS;532]` and quits.
    fn party_remove_as_member(
        &mut self,
        sender: Vid,
        leader: CharacterId,
        self_pid: CharacterId,
        pid: CharacterId,
    ) {
        if pid != self_pid {
            self.info_to(sender, b"[LS;677]", &[]);
            return;
        }
        let count = self
            .parties
            .by_leader
            .get(&leader)
            .map_or(0, |party| party.members.len());
        if count == 2 {
            self.delete_party(leader);
            return;
        }
        self.info_to(sender, b"[LS;532]", &[]);
        self.quit_party(self_pid);
    }

    /// `CPartyManager::DeleteParty` (`G/party.cpp:166-180`) for PCs: the DB packet
    /// drops with every other `HEADER_GD_PARTY_*` packet, and — the plan's
    /// DeleteParty-broadcast question, answered — legacy sends no member a packet:
    /// no remove, no unlink; the P2P destroy is commented out and no caller adds a
    /// companion broadcast. Members' clients keep showing a dead party until their
    /// own link ends. The `M2_DELETE` still runs `Destroy`'s `RemoveBonus`
    /// (`G/party.cpp:1043-1059`): every online member's six role points zero through
    /// `ComputePoints`. Ported as-is: zero, unindex every member, drop the party.
    fn delete_party(&mut self, leader: CharacterId) {
        let members: Vec<CharacterId> = self
            .parties
            .by_leader
            .get(&leader)
            .map(|party| party.members.keys().copied().collect())
            .unwrap_or_default();
        let vids: Vec<Vid> = self
            .parties
            .by_leader
            .get(&leader)
            .map(|party| {
                party
                    .members
                    .values()
                    .filter_map(|member| member.vid)
                    .collect()
            })
            .unwrap_or_default();
        for vid in vids {
            self.zero_role_points(vid, true);
        }
        for pid in members {
            self.parties.of.remove(&pid);
        }
        self.parties.by_leader.remove(&leader);
    }

    /// The 3 s Update tick plus the invite-timeout tick, called from `process_pulse`:
    /// every party whose `update_due` has come recomputes its near flags and waits
    /// another 75 Pulses; every invite whose due has come denies silently through
    /// `PartyInviteDeny` (so the inviter still hears `[LS;838;%s]`).
    pub(super) fn run_party_ticks(&mut self, pulse: u64) {
        let due_parties: Vec<CharacterId> = self
            .parties
            .by_leader
            .iter()
            .filter_map(|(leader, party)| (party.update_due <= pulse).then_some(*leader))
            .collect();
        for leader in due_parties {
            self.update_party_nears(leader);
            if let Some(party) = self.parties.by_leader.get_mut(&leader) {
                party.update_due = pulse.saturating_add(UPDATE_EVERY_PULSES);
            }
        }
        let expired: Vec<(CharacterId, CharacterId)> = self
            .parties
            .invites
            .iter()
            .filter_map(|(key, due)| (*due <= pulse).then_some(*key))
            .collect();
        for (leader, guest) in expired {
            let (Some(leader_vid), Some(guest_vid)) =
                (self.vid_of_pid(leader), self.vid_of_pid(guest))
            else {
                self.parties.invites.remove(&(leader, guest));
                continue;
            };
            self.party_invite_deny(leader_vid, guest_vid);
        }
    }
}

impl GameState {
    /// The party fan's read (`CParty::ForEachNearMember`, `G/party.h:288-296`): the
    /// near online members' VIDs in PID order, or `None` when `pid` is partyless or
    /// no member is near (`GetNearMemberCount()` zero takes the solo arm).
    pub(super) fn near_party_vids(&self, pid: CharacterId) -> Option<Vec<Vid>> {
        let leader = self.party_leader_of(pid)?;
        let party = self.parties.by_leader.get(&leader)?;
        if party.near_count == 0 {
            return None;
        }
        let vids: Vec<Vid> = party
            .members
            .values()
            .filter(|member| member.near)
            .filter_map(|member| member.vid)
            .collect();
        if vids.is_empty() {
            return None;
        }
        Some(vids)
    }
}

impl GameState {
    /// `CParty::SetRole` (`G/party.cpp:963-1023`): the cap refuses, the member must hold
    /// NORMAL, then the role stores, the linked member's points recompute (through
    /// `ComputePoints` when the leader is linked too), and the count rises. The
    /// `HEADER_GD_PARTY_STATE_CHANGE` packet drops (no DB peer). Success sends the
    /// member's info to all.
    fn set_party_role(&mut self, leader: CharacterId, pid: CharacterId, role: u8) -> bool {
        let index = usize::from(role);
        if index >= ROLE_COUNT {
            return false;
        }
        let (count, max) = self.parties.by_leader.get(&leader).map_or((0, 0), |party| {
            (party.role_count[index], party.role_max[index])
        });
        if count >= max {
            return false;
        }
        let (held, vid) = self
            .parties
            .by_leader
            .get(&leader)
            .and_then(|party| party.members.get(&pid))
            .map_or((ROLE_LEADER, None), |member| (member.role, member.vid));
        if held != ROLE_NORMAL {
            return false;
        }
        if let Some(party) = self.parties.by_leader.get_mut(&leader) {
            if let Some(member) = party.members.get_mut(&pid) {
                member.role = role;
            }
        }
        // The points recompute only when the member links and the leader's member row
        // exists (`GetLeader()` is the PID row, not the link).
        let leader_row = self
            .parties
            .by_leader
            .get(&leader)
            .is_some_and(|party| party.members.contains_key(&leader));
        if let Some(vid) = vid {
            if leader_row {
                self.compute_role_point(vid, role, true);
            }
            self.refresh_role_points(vid);
        }
        // The count rises after the points (`:986-994`); the `< MAX` check fails only
        // for 8+, which the input switch never passes.
        if index < ROLE_COUNT {
            if let Some(party) = self.parties.by_leader.get_mut(&leader) {
                party.role_count[index] += 1;
            }
        }
        self.send_party_info_one_to_all(leader, pid);
        true
    }

    /// `CParty::SetRole`'s unset arm: LEADER and NORMAL members refuse; else the role
    /// returns to NORMAL, all six points zero, and the PASSED role's count drops —
    /// not the member's old role (legacy decrements `m_anRoleCount[bRole]`, so
    /// unsetting a role the member never held drives that count negative).
    fn unset_party_role(&mut self, leader: CharacterId, pid: CharacterId, role: u8) -> bool {
        let held = self
            .parties
            .by_leader
            .get(&leader)
            .and_then(|party| party.members.get(&pid))
            .map_or(ROLE_NORMAL, |member| member.role);
        if held == ROLE_LEADER || held == ROLE_NORMAL {
            return false;
        }
        let vid = self
            .parties
            .by_leader
            .get(&leader)
            .and_then(|party| party.members.get(&pid))
            .and_then(|member| member.vid);
        if let Some(party) = self.parties.by_leader.get_mut(&leader) {
            if let Some(member) = party.members.get_mut(&pid) {
                member.role = ROLE_NORMAL;
            }
            if usize::from(role) < ROLE_COUNT {
                party.role_count[usize::from(role)] -= 1;
            }
        }
        if let Some(vid) = vid {
            self.zero_role_points(vid, false);
            self.refresh_role_points(vid);
        }
        self.send_party_info_one_to_all(leader, pid);
        true
    }

    /// `CParty::ComputeRolePoint(ch, role, true)` (`G/party.cpp:1215-1307`): the member's
    /// role point lands at its leadership formula. `k` is the member's own
    /// `(job, skill_group)` power row at `min(40, leadership)` over 100, in `f32`;
    /// `ATTACKER` reads `10 + 60k`, `TANKER` `50 + 1450k`, `BUFFER` `5 + 45k`,
    /// `SKILL_MASTER` `25 + 600k`, `HASTE` `1 + 5k`, `DEFENDER` `5 + 30k`, each
    /// truncated. `NORMAL`, `LEADER` and 8+ match nothing. The diff lands through
    /// `PointChange` (which emits a record per point) and the points refresh —
    /// except `BUFFER`, whose arm changes the point without a `ComputePoints`
    /// (ported as-is).
    fn compute_role_point(&mut self, vid: Vid, role: u8, near: bool) {
        if !near {
            self.zero_role_points(vid, false);
            return;
        }
        let (slot, base, scale) = match role {
            ROLE_ATTACKER => (
                common::point_slot::POINT_PARTY_ATTACKER_BONUS,
                10.0_f32,
                60.0_f32,
            ),
            ROLE_TANKER => (
                common::point_slot::POINT_PARTY_TANKER_BONUS,
                50.0_f32,
                1450.0_f32,
            ),
            ROLE_BUFFER => (
                common::point_slot::POINT_PARTY_BUFFER_BONUS,
                5.0_f32,
                45.0_f32,
            ),
            ROLE_SKILL_MASTER => (
                common::point_slot::POINT_PARTY_SKILL_MASTER_BONUS,
                25.0_f32,
                600.0_f32,
            ),
            ROLE_HASTE => (
                common::point_slot::POINT_PARTY_HASTE_BONUS,
                1.0_f32,
                5.0_f32,
            ),
            ROLE_DEFENDER => (
                common::point_slot::POINT_PARTY_DEFENDER_BONUS,
                5.0_f32,
                30.0_f32,
            ),
            _ => return,
        };
        let (job, group) = self
            .bodies
            .get(&vid)
            .map_or((0, 0), |body| (body.card.job, body.card.skill_group));
        let leadership = self
            .pid_of(vid)
            .and_then(|pid| self.party_leader_of(pid))
            .and_then(|leader| self.parties.by_leader.get(&leader))
            .map_or(0, |party| party.leadership);
        let power = self
            .pc_skill_tables
            .power(job, group, leadership.min(40), false);
        let k = f32::from(u16::try_from(power.max(0)).unwrap_or(u16::MAX)) / 100.0;
        #[allow(clippy::cast_possible_truncation)]
        let bonus = (base + scale * k) as i32;
        let current = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, |points| points.get_point(slot));
        if bonus == current {
            return;
        }
        self.change_pc_point(vid, slot, bonus - current, false);
        if role != ROLE_BUFFER {
            self.refresh_role_points(vid);
        }
    }

    /// `ComputeRolePoint`'s `bAdd=false` arm: all six role points zero through
    /// `PointChange`, then the refresh runs through `ComputePoints` when `full` —
    /// `ComputeBattlePoints` otherwise (the zeroing arms differ by caller).
    fn zero_role_points(&mut self, vid: Vid, full: bool) {
        use common::point_slot as point;
        for slot in [
            point::POINT_PARTY_ATTACKER_BONUS,
            point::POINT_PARTY_TANKER_BONUS,
            point::POINT_PARTY_BUFFER_BONUS,
            point::POINT_PARTY_SKILL_MASTER_BONUS,
            point::POINT_PARTY_HASTE_BONUS,
            point::POINT_PARTY_DEFENDER_BONUS,
        ] {
            let current = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.points())
                .map_or(0, |points| points.get_point(slot));
            if current != 0 {
                self.change_pc_point(vid, slot, -current, false);
            }
        }
        if full {
            self.refresh_role_points(vid);
        } else {
            self.refresh_battle_points(vid);
        }
    }

    /// `ComputePoints` on the player under `vid`, its records to its client.
    fn refresh_role_points(&mut self, vid: Vid) {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let records = points.compute_points();
        self.send_point_records(vid, &records);
    }

    /// `ComputeBattlePoints` on the player under `vid`, its records to its client.
    fn refresh_battle_points(&mut self, vid: Vid) {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let records = points.compute_battle_points();
        self.send_point_records(vid, &records);
    }

    /// `CParty::SetParameter` (`G/party.cpp:1617-1628`): modes 2 and up refuse quietly
    /// (legacy logs); else the mode stores and fans to every online member.
    fn set_party_parameter(&mut self, leader: CharacterId, mode: u8) {
        if mode >= DISTRIBUTION_MAX {
            return;
        }
        if let Some(party) = self.parties.by_leader.get_mut(&leader) {
            party.mode = mode;
        }
        for vid in self.online_party_vids(leader) {
            self.send_party_parameter(vid);
        }
    }

    /// `CInputMain::PartySetState` (`G/input_main.cpp:2522-2575`): the enabled gate
    /// passes (always enabled); a partyless or non-leader sender stays quiet or chats
    /// `[LS;671]`; a non-member PID chats `[LS;672]`; NORMAL is the legacy no-op;
    /// the six roles set or unset by the flag byte; any other role stays quiet like
    /// legacy's log-only default.
    pub(super) fn party_set_state(&mut self, sender: Vid, pid: CharacterId, role: u8, flag: u8) {
        let Some(self_pid) = self.pid_of(sender) else {
            return;
        };
        let Some(leader) = self.party_leader_of(self_pid) else {
            return;
        };
        if leader != self_pid {
            self.info_to(sender, b"[LS;671]", &[]);
            return;
        }
        let is_member = self
            .parties
            .by_leader
            .get(&leader)
            .is_some_and(|party| party.members.contains_key(&pid));
        if !is_member {
            self.info_to(sender, b"[LS;672]", &[]);
            return;
        }
        if role == ROLE_NORMAL {
            return;
        }
        // The input switch passes only 2-7 to `SetRole`; 8+ logs and stays quiet.
        if !(ROLE_ATTACKER..=ROLE_DEFENDER).contains(&role) {
            return;
        }
        if flag == 0 {
            self.unset_party_role(leader, pid, role);
        } else {
            self.set_party_role(leader, pid, role);
        }
    }

    /// `CInputMain::PartyParameter` (`G/input_main.cpp:2763-2772`): with
    /// `__FIX_EXP_GRUP__` defined (it is — `common/prodomodefines.h:89`), only the
    /// leader's party stores the mode; other senders stay quiet.
    pub(super) fn party_parameter(&mut self, sender: Vid, mode: u8) {
        let Some(self_pid) = self.pid_of(sender) else {
            return;
        };
        if self.party_leader_of(self_pid) == Some(self_pid) {
            self.set_party_parameter(self_pid, mode);
        }
    }

    /// `CInputMain::PartyUseSkill` (`G/input_main.cpp:2734-2761`): a partyless sender
    /// stays quiet; a non-leader chats `[LS;684]`; the leader's HEAL is legacy's dead
    /// body (`XXX DELETEME` returns first — ported as the same no-op; the dead body
    /// below it would refill every member within 5000 of the leader); WARP summons the
    /// target to the leader, and a gone VID chats the raw `LC_TEXT` line; any other index
    /// stays quiet like legacy's switch without a default.
    pub(super) fn party_use_skill(&mut self, sender: Vid, skill_index: u8, target: u32) {
        let Some(self_pid) = self.pid_of(sender) else {
            return;
        };
        let Some(leader) = self.party_leader_of(self_pid) else {
            return;
        };
        if leader != self_pid {
            self.info_to(sender, b"[LS;684]", &[]);
            return;
        }
        if skill_index == PARTY_SKILL_HEAL {
            return;
        }
        if skill_index != PARTY_SKILL_WARP {
            return;
        }
        let Some(pid) = self.pid_of(Vid::new(target)) else {
            self.info_to(sender, b"<Party> Cannot find target to summon.", &[]);
            return;
        };
        self.summon_to_leader(leader, pid);
    }

    /// `CParty::SummonToLeader` (`G/party.cpp:1107-1171`): the member `pid` names is
    /// pulled to a movable point around the leader and stopped. A non-member or offline
    /// PID chats `[LS;685]` to the leader, an unsummonable one `[LS;923]`, and no
    /// movable point `[LS;924]`. The leader is always linked here (its descriptor sent
    /// the skill), so a gone leader VID stays quiet.
    fn summon_to_leader(&mut self, leader: CharacterId, pid: CharacterId) {
        let leader_vid = self.vid_of_pid(leader);
        let Some(leader_vid) = leader_vid else {
            return;
        };
        let member = self
            .parties
            .by_leader
            .get(&leader)
            .and_then(|party| party.members.get(&pid));
        let offline = member.is_none() || member.is_some_and(|member| member.vid.is_none());
        if offline {
            self.info_to(leader_vid, b"[LS;685]", &[]);
            return;
        }
        let member_vid = member.and_then(|member| member.vid);
        let Some(member_vid) = member_vid else {
            return;
        };
        let leadership = self
            .parties
            .by_leader
            .get(&leader)
            .map_or(0, |party| party.leadership);
        if !self.can_summon(member_vid, leadership) {
            self.info_to(leader_vid, b"[LS;923]", &[]);
            return;
        }
        let Some(spot) = self.spot_of(leader_vid.raw()) else {
            return;
        };
        let (_, map) = self.place_of(leader_vid.raw()).unwrap_or((0, 0));
        let mut found: Vec<(i32, i32)> = Vec::new();
        for (dx, dy) in AROUND {
            let (Some(x), Some(y)) = (spot.x.checked_add(dx), spot.y.checked_add(dy)) else {
                continue;
            };
            if let Some(point) = self.movable_position(map, x, y) {
                found.push(point);
            }
        }
        if found.is_empty() {
            self.info_to(leader_vid, b"[LS;924]", &[]);
            return;
        }
        let n = i32::try_from(found.len()).unwrap_or(1).max(1);
        let at = found[usize::try_from(number(&mut self.dice, 0, n - 1).max(0)).unwrap_or(0)];
        self.summon_show(member_vid, leader_vid, at);
        self.stop(member_vid.raw());
    }

    /// The summon's `Show(leader_map, x, y)` (`:1168`): on the leader's map the member
    /// shows in place; on another map it leaves its own index first and joins the
    /// leader's, which is the same removes and inserts legacy's cross-map `Show` sends
    /// (no `GC_WARP` either way).
    fn summon_show(&mut self, member: Vid, leader: Vid, (x, y): (i32, i32)) {
        let leader_place = self.place_of(leader.raw());
        let member_place = self.place_of(member.raw());
        if member_place == leader_place {
            let _shown = self.show_body(member.raw(), (x, y, SHOW_Z), None);
            return;
        }
        let Some((channel, map)) = leader_place else {
            return;
        };
        let mut effects = Vec::new();
        if let Some((old_channel, old_map)) = member_place {
            if let Some(index) = self.maps.get_mut(&(old_channel, old_map)) {
                index.remove(
                    super::view::EntityKey::Character(member.raw()),
                    &mut effects,
                );
            }
        }
        if let Some(body) = self.bodies.get_mut(&member) {
            body.channel = channel;
            body.map = map;
        }
        self.deliver(&effects, None);
        let _shown = self.show_body(member.raw(), (x, y, SHOW_Z), None);
    }

    /// `CHARACTER::CanSummon` (`G/char.cpp:847-848`): leadership 20 and up always, 12-19
    /// within 180 of the member's death, below 12 never. `m_dwLastDeadTime` is a
    /// millisecond clock but the check adds 180, so the window is 180 MILLISECONDS
    /// (suspected unit mismatch: the constructor seeds `now - 180000`), ported
    /// as-written.
    fn can_summon(&self, member: Vid, leadership: u8) -> bool {
        if leadership >= 20 {
            return true;
        }
        if leadership < 12 {
            return false;
        }
        let Some(dead) = self
            .pc_combat
            .get(&member.raw())
            .and_then(|combat| combat.last_dead_ms)
        else {
            return false;
        };
        dead.wrapping_add(180) > self.clock.now()
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::super::fixtures::{a_world, enter, TestClock};
    use super::*;
    use crate::game_loop::PulseProcessor;
    use crate::game_loop_messages::RelayScope;

    /// The inviter, at the origin.
    const LEADER_VID: u32 = 100;
    /// The guest, 100 east.
    const GUEST_VID: u32 = 101;
    /// A third player, far away.
    const THIRD_VID: u32 = 102;

    const HERE: (i32, i32) = (3200, 3200);
    const EAST: (i32, i32) = (3300, 3200);
    const FAR: (i32, i32) = (9000, 9000);

    fn a_pair() -> (
        TestClock,
        GameState,
        UnboundedReceiver<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let clock = TestClock::default();
        clock.set(1_000);
        let mut world = a_world(&clock);
        let leader = enter(&mut world, LEADER_VID, HERE, 0);
        let guest = enter(&mut world, GUEST_VID, EAST, 0);
        (clock, world, leader, guest)
    }

    /// What `inbox` holds since the last look.
    fn heard(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The headers of `records`, in order.
    fn headers(records: &[Vec<u8>]) -> Vec<u8> {
        records.iter().map(|record| record[0]).collect()
    }

    #[test]
    fn invite_accept_joins_and_links_in_legacy_order() {
        let (_clock, mut world, mut leader, mut guest) = a_pair();
        heard(&mut leader);
        heard(&mut guest);
        world.party_invite(Vid::new(LEADER_VID), Vid::new(GUEST_VID));
        let invite = heard(&mut guest);
        assert_eq!(headers(&invite), [HEADER_GC_PARTY_INVITE]);
        assert!(heard(&mut leader).is_empty());

        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        // The leader hears: join-one-to-all (its own ADD at create) went out before;
        // at accept: guest ADD, the link sends, infos, parameter. The guest hears its
        // own ADD, every ADD, the links, every info, infos, parameter.
        let leader_frames = heard(&mut leader);
        let guest_frames = heard(&mut guest);
        let leader_headers = headers(&leader_frames);
        let guest_headers = headers(&guest_frames);
        // Both hear the guest's ADD (82), LINKs (76), UPDATEs (79) and PARAMETER (81);
        // exact order follows the six sends + parameter of `Link`.
        assert!(leader_headers.contains(&HEADER_GC_PARTY_ADD));
        assert!(leader_headers.contains(&HEADER_GC_PARTY_LINK));
        assert!(leader_headers.contains(&HEADER_GC_PARTY_UPDATE));
        assert!(guest_headers.contains(&HEADER_GC_PARTY_PARAMETER));
        assert_eq!(world.party_leader_of(LEADER_VID), Some(LEADER_VID));
        assert_eq!(world.party_leader_of(GUEST_VID), Some(LEADER_VID));
    }

    #[test]
    fn accept_without_invite_is_quiet() {
        let (_clock, mut world, mut leader, mut guest) = a_pair();
        heard(&mut leader);
        heard(&mut guest);
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        assert!(heard(&mut leader).is_empty());
        assert!(heard(&mut guest).is_empty());
        assert!(world.party_leader_of(GUEST_VID).is_none());
    }

    #[test]
    fn deny_cancels_so_a_later_accept_is_quiet() {
        let (_clock, mut world, mut leader, mut guest) = a_pair();
        heard(&mut leader);
        heard(&mut guest);
        world.party_invite(Vid::new(LEADER_VID), Vid::new(GUEST_VID));
        heard(&mut guest);
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 0);
        // The inviter hears the `[LS;838;%s]` line.
        assert_eq!(
            headers(&heard(&mut leader)),
            [protocol::gc_inventory::HEADER_GC_CHAT.value()]
        );
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        assert!(heard(&mut leader).is_empty());
        assert!(heard(&mut guest).is_empty());
        assert!(world.party_leader_of(GUEST_VID).is_none());
    }

    #[test]
    fn invite_expires_after_250_pulses() {
        let (_clock, mut world, mut leader, mut guest) = a_pair();
        heard(&mut leader);
        heard(&mut guest);
        world.party_invite(Vid::new(LEADER_VID), Vid::new(GUEST_VID));
        heard(&mut guest);
        for pulse in 1..=250_u64 {
            world.process_pulse(pulse);
        }
        // Expiry denies through `PartyInviteDeny`: the inviter hears the line (among the
        // recovery frames the 250 Pulses also send).
        let leader_headers = headers(&heard(&mut leader));
        assert!(leader_headers.contains(&protocol::gc_inventory::HEADER_GC_CHAT.value()));
        heard(&mut guest);
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        assert!(heard(&mut guest).is_empty());
        assert!(world.party_leader_of(GUEST_VID).is_none());
    }

    #[test]
    fn remove_disbands_a_party_of_two() {
        let (_clock, mut world, mut leader, mut guest) = a_pair();
        heard(&mut leader);
        heard(&mut guest);
        world.party_invite(Vid::new(LEADER_VID), Vid::new(GUEST_VID));
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        heard(&mut leader);
        heard(&mut guest);
        world.party_remove(Vid::new(LEADER_VID), GUEST_VID);
        // Disband sends no party record (legacy's `DeleteParty` has no member
        // broadcast) — only the point-change frames of `Destroy`'s `RemoveBonus`
        // zeroing the role points through `ComputePoints`.
        let leader_frames = heard(&mut leader);
        let guest_frames = heard(&mut guest);
        let point = protocol::gc_inventory::HEADER_GC_PLAYER_POINT_CHANGE.value();
        assert!(!leader_frames.is_empty() || !guest_frames.is_empty());
        for frame in leader_frames.iter().chain(guest_frames.iter()) {
            assert_eq!(frame[0], point);
        }
        assert!(world.party_leader_of(LEADER_VID).is_none());
        assert!(world.party_leader_of(GUEST_VID).is_none());
    }

    #[test]
    fn non_leader_self_leave_at_three_stays_and_tells() {
        let clock = TestClock::default();
        clock.set(1_000);
        let mut world = a_world(&clock);
        let mut leader = enter(&mut world, LEADER_VID, HERE, 0);
        let mut guest = enter(&mut world, GUEST_VID, EAST, 0);
        let mut third = enter(&mut world, THIRD_VID, EAST, 0);
        heard(&mut leader);
        heard(&mut guest);
        heard(&mut third);
        world.party_invite(Vid::new(LEADER_VID), Vid::new(GUEST_VID));
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        world.party_invite(Vid::new(LEADER_VID), Vid::new(THIRD_VID));
        world.party_invite_answer(Vid::new(THIRD_VID), Vid::new(LEADER_VID), 1);
        heard(&mut leader);
        heard(&mut guest);
        heard(&mut third);
        world.party_remove(Vid::new(GUEST_VID), GUEST_VID);
        // The leaver hears `[LS;532]` then its own REMOVE (`SendPartyRemoveOneToAll`
        // runs before the erase), then the `RemoveBonusForOne` point zeros; the rest
        // hear its REMOVE.
        let guest_headers = headers(&heard(&mut guest));
        let point = protocol::gc_inventory::HEADER_GC_PLAYER_POINT_CHANGE.value();
        assert_eq!(
            guest_headers[..2],
            [
                protocol::gc_inventory::HEADER_GC_CHAT.value(),
                HEADER_GC_PARTY_REMOVE
            ]
        );
        assert!(!guest_headers[2..].is_empty());
        for header in &guest_headers[2..] {
            assert_eq!(*header, point);
        }
        assert!(headers(&heard(&mut leader)).contains(&HEADER_GC_PARTY_REMOVE));
        assert!(headers(&heard(&mut third)).contains(&HEADER_GC_PARTY_REMOVE));
        assert!(world.party_leader_of(GUEST_VID).is_none());
        assert_eq!(world.party_leader_of(THIRD_VID), Some(LEADER_VID));
    }

    #[test]
    fn update_nears_wipes_and_resends() {
        let (_clock, mut world, mut leader, mut guest) = a_pair();
        heard(&mut leader);
        heard(&mut guest);
        world.party_invite(Vid::new(LEADER_VID), Vid::new(GUEST_VID));
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        heard(&mut leader);
        heard(&mut guest);
        // Both stand 100 apart: the first Update nears both and resends infos.
        world.process_pulse(75);
        let party = world
            .parties
            .by_leader
            .get(&LEADER_VID)
            .expect("the party stands");
        assert_eq!(party.near_count, 2);
        assert!(headers(&heard(&mut leader)).contains(&HEADER_GC_PARTY_UPDATE));
        // The guest walks 5800 away: the count changes and infos resend.
        world.leave_world(Vid::new(GUEST_VID));
        let mut guest = enter(&mut world, GUEST_VID, FAR, 0);
        heard(&mut guest);
        heard(&mut leader);
        world.process_pulse(150);
        let party = world
            .parties
            .by_leader
            .get(&LEADER_VID)
            .expect("the party stands");
        assert_eq!(party.near_count, 0);
        assert!(headers(&heard(&mut leader)).contains(&HEADER_GC_PARTY_UPDATE));
    }

    #[test]
    fn the_fan_buffs_the_near() {
        let (_clock, mut world, mut leader, mut guest) = a_pair();
        heard(&mut leader);
        heard(&mut guest);
        world.party_invite(Vid::new(LEADER_VID), Vid::new(GUEST_VID));
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        world.process_pulse(75);
        assert!(world.near_party_vids(LEADER_VID).is_some());
        // No hosted PC row is PARTY-flagged, so the fan has no live caller yet; this
        // pins the walk directly with vnum 3 (a hosted ATT_SPEED buff — the fan never
        // checks flags, and neither does legacy's functor).
        heard(&mut leader);
        heard(&mut guest);
        // `compute_skill_party` is private to `pc_skill`; reach it through the fan read.
        let vids = world.near_party_vids(GUEST_VID).expect("both near");
        assert_eq!(vids.len(), 2);
    }

    /// The member's six role points, in UPDATE order.
    fn six(world: &GameState, vid: u32) -> [i32; 6] {
        use common::point_slot as point;
        let character = world
            .characters
            .find_by_vid(Vid::new(vid))
            .expect("the member stands");
        let points = character.points().expect("the member has points");
        [
            points.get_point(point::POINT_PARTY_ATTACKER_BONUS),
            points.get_point(point::POINT_PARTY_TANKER_BONUS),
            points.get_point(point::POINT_PARTY_BUFFER_BONUS),
            points.get_point(point::POINT_PARTY_SKILL_MASTER_BONUS),
            points.get_point(point::POINT_PARTY_HASTE_BONUS),
            points.get_point(point::POINT_PARTY_DEFENDER_BONUS),
        ]
    }

    /// The last UPDATE for `pid` among `records`, decoded.
    fn last_update_for(records: &[Vec<u8>], pid: CharacterId) -> GcPartyUpdate {
        records
            .iter()
            .filter(|record| record[0] == HEADER_GC_PARTY_UPDATE)
            .filter_map(|record| GcPartyUpdate::decode(record).ok())
            .filter(|packet| packet.pid == pid)
            .next_back()
            .expect("an UPDATE for the member")
    }

    /// A pair partied and Updated at 100 apart: both near.
    fn a_partied_pair() -> (
        TestClock,
        GameState,
        UnboundedReceiver<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let (clock, mut world, mut leader, mut guest) = a_pair();
        world.party_invite(Vid::new(LEADER_VID), Vid::new(GUEST_VID));
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        world.process_pulse(75);
        heard(&mut leader);
        heard(&mut guest);
        (clock, world, leader, guest)
    }

    #[test]
    fn set_role_caps_follow_leadership() {
        let (_clock, mut world, mut leader, mut guest) = a_partied_pair();
        // Leadership 0: every cap is 0, so ATTACKER refuses.
        world.party_set_state(Vid::new(LEADER_VID), GUEST_VID, ROLE_ATTACKER, 1);
        assert_eq!(
            world
                .parties
                .by_leader
                .get(&LEADER_VID)
                .and_then(|party| party.members.get(&GUEST_VID))
                .map(|member| member.role),
            Some(ROLE_NORMAL)
        );
        assert!(heard(&mut leader).is_empty());
        assert!(heard(&mut guest).is_empty());
        // Leadership 10: the cap opens; one ATTACKER holds, a second refuses.
        world.set_pc_skill_level(LEADER_VID, 121, 10);
        world.process_pulse(150);
        heard(&mut leader);
        heard(&mut guest);
        world.party_set_state(Vid::new(LEADER_VID), GUEST_VID, ROLE_ATTACKER, 1);
        assert_eq!(
            world
                .parties
                .by_leader
                .get(&LEADER_VID)
                .and_then(|party| party.members.get(&GUEST_VID))
                .map(|member| member.role),
            Some(ROLE_ATTACKER)
        );
        let update = last_update_for(&heard(&mut guest), GUEST_VID);
        assert_eq!(update.state, ROLE_ATTACKER);
        world.party_set_state(Vid::new(LEADER_VID), LEADER_VID, ROLE_ATTACKER, 1);
        assert_eq!(
            world
                .parties
                .by_leader
                .get(&LEADER_VID)
                .and_then(|party| party.members.get(&LEADER_VID))
                .map(|member| member.role),
            Some(ROLE_LEADER)
        );
        // Leadership 40: the caps read 2/1/1/1/1/1.
        world.set_pc_skill_level(LEADER_VID, 121, 40);
        world.process_pulse(225);
        let party = world
            .parties
            .by_leader
            .get(&LEADER_VID)
            .expect("the party stands");
        assert_eq!(
            [
                party.role_max[ROLE_ATTACKER as usize],
                party.role_max[ROLE_TANKER as usize],
                party.role_max[ROLE_BUFFER as usize],
                party.role_max[ROLE_SKILL_MASTER as usize],
                party.role_max[ROLE_HASTE as usize],
                party.role_max[ROLE_DEFENDER as usize],
            ],
            [2, 1, 1, 1, 1, 1]
        );
    }

    #[test]
    fn attacker_bonus_lands_and_clears_with_near() {
        let (_clock, mut world, mut leader, mut guest) = a_partied_pair();
        world.set_pc_skill_level(LEADER_VID, 121, 10);
        world.process_pulse(150);
        heard(&mut leader);
        heard(&mut guest);
        // The fixture's tables are empty, so k is 0 and ATTACKER lands at 10.
        world.party_set_state(Vid::new(LEADER_VID), GUEST_VID, ROLE_ATTACKER, 1);
        assert_eq!(six(&world, GUEST_VID), [10, 0, 0, 0, 0, 0]);
        heard(&mut guest);
        // The guest walks away: the next Update zeroes the bonus (un-near).
        world.leave_world(Vid::new(GUEST_VID));
        let mut guest = enter(&mut world, GUEST_VID, FAR, 0);
        heard(&mut guest);
        world.process_pulse(225);
        assert_eq!(six(&world, GUEST_VID), [0, 0, 0, 0, 0, 0]);
        // The member's role stays ATTACKER: distance clears points, not roles.
        assert_eq!(
            world
                .parties
                .by_leader
                .get(&LEADER_VID)
                .and_then(|party| party.members.get(&GUEST_VID))
                .map(|member| member.role),
            Some(ROLE_ATTACKER)
        );
    }

    #[test]
    fn unset_zeroes_all_six_and_decrements_the_passed_role() {
        let (_clock, mut world, _leader, _guest) = a_partied_pair();
        world.set_pc_skill_level(LEADER_VID, 121, 20);
        world.process_pulse(150);
        world.party_set_state(Vid::new(LEADER_VID), GUEST_VID, ROLE_ATTACKER, 1);
        world.party_set_state(Vid::new(LEADER_VID), LEADER_VID, ROLE_HASTE, 1);
        assert_eq!(six(&world, GUEST_VID)[0], 10);
        // Unsetting TANKER on an ATTACKER member: the role returns to NORMAL, all
        // six zero, and the TANKER count — not the ATTACKER count — drops to -1.
        world.party_set_state(Vid::new(LEADER_VID), GUEST_VID, ROLE_TANKER, 0);
        assert_eq!(six(&world, GUEST_VID), [0, 0, 0, 0, 0, 0]);
        let party = world
            .parties
            .by_leader
            .get(&LEADER_VID)
            .expect("the party stands");
        assert_eq!(
            party.members.get(&GUEST_VID).map(|member| member.role),
            Some(ROLE_NORMAL)
        );
        assert_eq!(party.role_count[ROLE_TANKER as usize], -1);
        assert_eq!(party.role_count[ROLE_ATTACKER as usize], 1);
    }

    #[test]
    fn quit_removes_the_bonus_and_decrements() {
        let (_clock, mut world, _leader, _guest) = a_partied_pair();
        world.set_pc_skill_level(LEADER_VID, 121, 10);
        world.process_pulse(150);
        world.party_set_state(Vid::new(LEADER_VID), GUEST_VID, ROLE_ATTACKER, 1);
        assert_eq!(six(&world, GUEST_VID)[0], 10);
        world.party_remove(Vid::new(GUEST_VID), GUEST_VID);
        // The party of two disbands: the leaver's points zeroed via `RemoveBonus`.
        assert_eq!(six(&world, GUEST_VID)[0], 0);
        assert!(world.party_leader_of(GUEST_VID).is_none());
    }
    #[test]
    fn leader_unlink_zeroes_every_linked_members_role_points_and_nears() {
        let (_clock, mut world, mut leader, mut guest) = a_partied_pair();
        world.set_pc_skill_level(LEADER_VID, 121, 10);
        world.process_pulse(150);
        world.party_set_state(Vid::new(LEADER_VID), GUEST_VID, ROLE_ATTACKER, 1);
        // The leader's own role stays LEADER (`SetRole` refuses a held non-NORMAL
        // role): the guest's bonus is what the leader's unlink clears.
        assert_eq!(six(&world, GUEST_VID)[0], 10);
        assert!(world
            .parties
            .by_leader
            .get(&LEADER_VID)
            .is_some_and(|party| party.members.values().all(|member| member.near)));
        heard(&mut leader);
        heard(&mut guest);
        // The leader leaves the world: unlink, then `RemoveBonus` zeroes every linked
        // member's six (`G/party.cpp:649-654`) and clears every near flag.
        world.leave_world(Vid::new(LEADER_VID));
        assert_eq!(six(&world, GUEST_VID), [0, 0, 0, 0, 0, 0]);
        assert!(!heard(&mut guest).is_empty(), "the guest hears its zeroing");
        let party = world
            .parties
            .by_leader
            .get(&LEADER_VID)
            .expect("the party stands");
        assert!(party.members.values().all(|member| !member.near));
        assert!(party
            .members
            .get(&LEADER_VID)
            .is_some_and(|member| member.vid.is_none()));
    }

    #[test]
    fn member_unlink_keeps_the_role_points() {
        let (_clock, mut world, mut leader, _guest) = a_partied_pair();
        world.set_pc_skill_level(LEADER_VID, 121, 10);
        world.process_pulse(150);
        world.party_set_state(Vid::new(LEADER_VID), GUEST_VID, ROLE_ATTACKER, 1);
        assert_eq!(six(&world, GUEST_VID)[0], 10);
        heard(&mut leader);
        // The guest leaves the world: the unlink goes out first, with no point-change
        // (17) or character-update records after it — no `RemoveBonus` runs
        // (`G/party.cpp:632-670` reserves it for the leader). The later records are the
        // leave's own view effects.
        world.leave_world(Vid::new(GUEST_VID));
        let frames = heard(&mut leader);
        assert_eq!(
            frames.first().map(|record| record[0]),
            Some(HEADER_GC_PARTY_UNLINK)
        );
        assert!(!frames
            .iter()
            .skip(1)
            .any(|record| record[0] == protocol::gc::HEADER_GC_PLAYER_POINT_CHANGE));
        assert!(world.party_leader_of(GUEST_VID).is_some());
    }

    #[test]
    fn exp_bonus_table_and_longtime() {
        let (clock, mut world, leader, mut guest) = a_partied_pair();
        drop(leader);
        // Two near: the table reads 12, and the UPDATE carries it.
        // The leader pins 70003 on UNIQUE1: the bonus rises 30.
        let mut leader = {
            use common::enums::EWearPositions;
            use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
            use protocol::item_pos::ItemPos;
            use world::item::Item;
            let medal = Item::new(11, 70_003);
            let cell = INVENTORY_MAX_NUM + EWearPositions::Unique1 as u16;
            let items = [(ItemPos::new(EWindows::Inventory as u8, cell), medal)];
            world.leave_world(Vid::new(LEADER_VID));
            let (mut inbox, _) = super::super::fixtures::enter_seeing_with(
                &mut world,
                super::super::fixtures::PLACE,
                LEADER_VID,
                HERE,
                0,
                &items,
            );
            heard(&mut inbox);
            inbox
        };
        world.process_pulse(150);
        // The re-enter unlinked the leader, so the Update takes the leaderless
        // arm: no resend here. Relink and Update again.
        world.link_party(Vid::new(LEADER_VID));
        heard(&mut leader);
        heard(&mut guest);
        world.process_pulse(225);
        let update = last_update_for(&heard(&mut guest), GUEST_VID);
        assert_eq!(update.affects[0], 42);
        // An hour passes: the long-time +5 lands in the multiplier read, the
        // `[LS;926]` line goes out once, and affects[0] stays 42.
        clock.set(1_000 + 61 * 60 * 1000);
        heard(&mut leader);
        heard(&mut guest);
        world.process_pulse(300);
        let chat = protocol::gc_inventory::HEADER_GC_CHAT.value();
        let leader_headers = headers(&heard(&mut leader));
        let mut chats = 0;
        for header in &leader_headers {
            if *header == chat {
                chats += 1;
            }
        }
        assert_eq!(chats, 1);
        let update = last_update_for(&heard(&mut guest), GUEST_VID);
        assert_eq!(update.affects[0], 42);
        let party = world
            .parties
            .by_leader
            .get(&LEADER_VID)
            .expect("the party stands");
        assert_eq!(party.longtime_bonus, 5);
        // A further Update sends no second line: the flag is the bonus itself.
        heard(&mut leader);
        world.process_pulse(375);
        assert!(
            !headers(&heard(&mut leader)).contains(&chat),
            "the long-time line goes out once"
        );
    }

    #[test]
    fn parameter_stores_and_fans() {
        let (_clock, mut world, mut leader, mut guest) = a_partied_pair();
        world.party_parameter(Vid::new(LEADER_VID), 1);
        assert_eq!(headers(&heard(&mut leader)), [HEADER_GC_PARTY_PARAMETER]);
        assert_eq!(headers(&heard(&mut guest)), [HEADER_GC_PARTY_PARAMETER]);
        assert_eq!(
            world
                .parties
                .by_leader
                .get(&LEADER_VID)
                .map(|party| party.mode),
            Some(1)
        );
        // A member's set stays quiet; mode 2 refuses quietly.
        world.party_parameter(Vid::new(GUEST_VID), 0);
        world.party_parameter(Vid::new(LEADER_VID), 2);
        assert!(heard(&mut leader).is_empty());
        assert!(heard(&mut guest).is_empty());
        assert_eq!(
            world
                .parties
                .by_leader
                .get(&LEADER_VID)
                .map(|party| party.mode),
            Some(1)
        );
    }

    #[test]
    fn affects_fill_behind_the_live_gate() {
        let (_clock, mut world, mut leader, mut guest) = a_partied_pair();
        world.set_pc_skill_level(LEADER_VID, 121, 10);
        world.process_pulse(150);
        world.party_set_state(Vid::new(LEADER_VID), GUEST_VID, ROLE_ATTACKER, 1);
        // Near: the SetRole info fan's UPDATE carries [12, 10, 0, 0, 0, 0, 0].
        let update = last_update_for(&heard(&mut guest), GUEST_VID);
        assert_eq!(update.affects, [12, 10, 0, 0, 0, 0, 0]);
        heard(&mut leader);
        // The guest steps 5800 away; no Update runs. The next UPDATE (from the
        // SetRole info fan) reads the live gate, not the cached near: zeroed.
        world.leave_world(Vid::new(GUEST_VID));
        let mut guest = enter(&mut world, GUEST_VID, FAR, 0);
        heard(&mut guest);
        heard(&mut leader);
        world.party_set_state(Vid::new(LEADER_VID), GUEST_VID, ROLE_ATTACKER, 0);
        let update = last_update_for(&heard(&mut guest), GUEST_VID);
        assert_eq!(update.affects, [0, 0, 0, 0, 0, 0, 0]);
    }
    /// The summon's gates: a non-member PID and an offline member chat `[LS;685]`, an
    /// unsummonable member `[LS;923]`, a gone VID the raw `LC_TEXT` line, and HEAL or an
    /// unknown index stays quiet.
    #[test]
    fn summon_gates_chat_their_lines() {
        let (_clock, mut world, mut leader, _guest) = a_partied_pair();
        let mut third = enter(&mut world, THIRD_VID, FAR, 0);
        heard(&mut third);
        heard(&mut leader);
        let chat = protocol::gc_inventory::HEADER_GC_CHAT.value();
        // A non-member PID.
        world.party_use_skill(Vid::new(LEADER_VID), PARTY_SKILL_WARP, THIRD_VID);
        let frames = heard(&mut leader);
        assert_eq!(headers(&frames), [chat]);
        assert!(frames[0].windows(8).any(|w| w == b"[LS;685]"));
        // A gone VID.
        world.party_use_skill(Vid::new(LEADER_VID), PARTY_SKILL_WARP, 9999);
        let frames = heard(&mut leader);
        assert_eq!(headers(&frames), [chat]);
        assert!(frames[0]
            .windows(37)
            .any(|w| w == b"<Party> Cannot find target to summon."));
        // An unsummonable member: leadership 0.
        world.party_use_skill(Vid::new(LEADER_VID), PARTY_SKILL_WARP, GUEST_VID);
        let frames = heard(&mut leader);
        assert_eq!(headers(&frames), [chat]);
        assert!(frames[0].windows(8).any(|w| w == b"[LS;923]"));
        // HEAL and an unknown index stay quiet.
        world.party_use_skill(Vid::new(LEADER_VID), PARTY_SKILL_HEAL, GUEST_VID);
        world.party_use_skill(Vid::new(LEADER_VID), 9, GUEST_VID);
        assert!(heard(&mut leader).is_empty());
    }

    /// `CanSummon`'s windows: 20 and up always summons, 12-19 within 180ms of death
    /// summons and after refuses, below 12 refuses.
    #[test]
    fn can_summon_keeps_its_windows() {
        let (clock, mut world, mut leader, _guest) = a_partied_pair();
        let chat = protocol::gc_inventory::HEADER_GC_CHAT.value();
        let mut pulse = 150;
        let mut set_leadership = |world: &mut GameState, level: u8| {
            world.pc_skills.entry(LEADER_VID).or_default().levels[121] = level;
            world.process_pulse(pulse);
            pulse += 75;
        };
        // Leadership 20: the summon runs (no cells, so the [LS;924] line proves the
        // CanSummon gate passed).
        set_leadership(&mut world, 20);
        heard(&mut leader);
        world.party_use_skill(Vid::new(LEADER_VID), PARTY_SKILL_WARP, GUEST_VID);
        let frames = heard(&mut leader);
        assert_eq!(headers(&frames), [chat]);
        assert!(frames[0].windows(8).any(|w| w == b"[LS;924]"));
        // Leadership 11: refused.
        set_leadership(&mut world, 11);
        heard(&mut leader);
        world.party_use_skill(Vid::new(LEADER_VID), PARTY_SKILL_WARP, GUEST_VID);
        let frames = heard(&mut leader);
        assert_eq!(headers(&frames), [chat]);
        assert!(frames[0].windows(8).any(|w| w == b"[LS;923]"));
        // Leadership 12, never died: refused.
        set_leadership(&mut world, 12);
        heard(&mut leader);
        world.party_use_skill(Vid::new(LEADER_VID), PARTY_SKILL_WARP, GUEST_VID);
        let frames = heard(&mut leader);
        assert_eq!(headers(&frames), [chat]);
        assert!(frames[0].windows(8).any(|w| w == b"[LS;923]"));
        // Leadership 12, died at the clock: runs within 180ms, refuses after.
        world.pc_combat.entry(GUEST_VID).or_default().last_dead_ms = Some(1_000);
        heard(&mut leader);
        world.party_use_skill(Vid::new(LEADER_VID), PARTY_SKILL_WARP, GUEST_VID);
        let frames = heard(&mut leader);
        assert_eq!(headers(&frames), [chat]);
        assert!(frames[0].windows(8).any(|w| w == b"[LS;924]"));
        // Leadership 12, died 181ms ago: refused.
        clock.set(1_000 + 181);
        heard(&mut leader);
        world.party_use_skill(Vid::new(LEADER_VID), PARTY_SKILL_WARP, GUEST_VID);
        let frames = heard(&mut leader);
        assert_eq!(headers(&frames), [chat]);
        assert!(frames[0].windows(8).any(|w| w == b"[LS;923]"));
    }

    /// The summon moves the member beside the leader and stops it: the movers lose it,
    /// and its start and destination stand at its spot.
    #[test]
    fn summon_moves_and_stops_the_member() {
        use super::super::fixtures::{open_cells, stand_field};
        use world::npc::Npc;
        let clock = TestClock::default();
        clock.set(1_000);
        let mut world = stand_field(&clock, Vec::<Npc>::new(), open_cells(), true);
        let mut leader = enter(&mut world, LEADER_VID, HERE, 0);
        // The guest stands moving 100 east.
        let mut guest = enter(&mut world, GUEST_VID, EAST, 0);
        world.movers.insert(GUEST_VID);
        world.party_invite(Vid::new(LEADER_VID), Vid::new(GUEST_VID));
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        // Leadership 20: the summon runs.
        world.pc_skills.entry(LEADER_VID).or_default().levels[121] = 20;
        world.process_pulse(75);
        heard(&mut leader);
        heard(&mut guest);
        world.party_use_skill(Vid::new(LEADER_VID), PARTY_SKILL_WARP, GUEST_VID);
        assert!(!world.movers.contains(&GUEST_VID));
        let spot = world.spot_of(GUEST_VID).expect("the member stands");
        let body = world
            .bodies
            .get(&Vid::new(GUEST_VID))
            .expect("the body stands");
        assert_eq!(body.motion.start, (spot.x, spot.y));
        assert_eq!(body.motion.dest, (spot.x, spot.y));
        // Beside the leader: within 250 + the movable search of it.
        let leader_spot = world.spot_of(LEADER_VID).expect("the leader stands");
        assert!(spot.x.abs_diff(leader_spot.x) <= 500);
        assert!(spot.y.abs_diff(leader_spot.y) <= 500);
        assert!(heard(&mut leader)
            .iter()
            .all(|frame| { frame[0] != protocol::gc_inventory::HEADER_GC_CHAT.value() }));
    }
    /// A party line reaches every linked member in PID order, the speaker included;
    /// an unlinked member and an outsider hear nothing, and a partyless speaker's
    /// fan is quiet (`ForEachOnlineMember`, `G/party.h:277-285`).
    #[test]
    fn a_party_fan_reaches_the_linked_members_in_pid_order() {
        let clock = TestClock::default();
        clock.set(1_000);
        let mut world = a_world(&clock);
        let mut leader = enter(&mut world, LEADER_VID, HERE, 0);
        let mut guest = enter(&mut world, GUEST_VID, EAST, 0);
        let mut third = enter(&mut world, THIRD_VID, FAR, 0);
        world.party_invite(Vid::new(LEADER_VID), Vid::new(GUEST_VID));
        world.party_invite_answer(Vid::new(GUEST_VID), Vid::new(LEADER_VID), 1);
        assert!(world.holds_party(Vid::new(LEADER_VID)));
        assert!(world.holds_party(Vid::new(GUEST_VID)));
        assert!(!world.holds_party(Vid::new(THIRD_VID)));
        assert_eq!(
            world.party_member_vids(Vid::new(GUEST_VID)),
            vec![Vid::new(LEADER_VID), Vid::new(GUEST_VID)],
            "PID order, speaker included"
        );
        let _ = (heard(&mut leader), heard(&mut guest), heard(&mut third));
        world.relay(Vid::new(GUEST_VID), &[(RelayScope::Party, vec![7])]);
        assert_eq!(heard(&mut leader), vec![vec![7]]);
        assert_eq!(heard(&mut guest), vec![vec![7]]);
        assert!(heard(&mut third).is_empty(), "an outsider hears nothing");
        // The guest unlinks: still a member, but off the fan.
        world.unlink_party(Vid::new(GUEST_VID));
        assert!(
            world.holds_party(Vid::new(GUEST_VID)),
            "unlink keeps the PID"
        );
        let _ = (heard(&mut leader), heard(&mut guest));
        world.relay(Vid::new(LEADER_VID), &[(RelayScope::Party, vec![8])]);
        assert_eq!(heard(&mut leader), vec![vec![8]]);
        assert!(
            heard(&mut guest).is_empty(),
            "the unlinked member is skipped"
        );
        // A partyless speaker's fan is quiet.
        world.relay(Vid::new(THIRD_VID), &[(RelayScope::Party, vec![9])]);
        assert!(heard(&mut third).is_empty());
        assert!(heard(&mut leader).is_empty());
    }
}
