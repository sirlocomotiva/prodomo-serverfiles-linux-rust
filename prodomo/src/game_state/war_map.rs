//! The war maps (`G/war_map.cpp`): the war types, the two maps a guild war can be fought
//! on, and the private copy a war takes of the map it is fought on.
//!
//! Legacy keeps the war map information in the manager's own table
//! (`CWarMapManager::LoadWarMapInfo`, `G/war_map.cpp:974-1008`): two maps, three start
//! positions each, and a type. The war types are a table of their own,
//! `KOR_aGuildWarInfo` (`G/constants.cpp:1132-1145`), and each names the map it is fought
//! on — 0 for the field war, which is fought where the players already stand.
//!
//! # Divergences
//!
//! - **The map index and the Channel's map set.** Legacy checks the map twice: the declare
//!   asks the map index (`GuildWar_IsWarMap`, `G/guild_war.cpp:260-271`) and the wait-start
//!   asks the Channel's `MAP_ALLOW` (`map_allow_find`, `G/guild_war.cpp:488-493`). The
//!   Rewrite keeps the two checks apart: the atlas's indices are the map index, and the
//!   hosted maps are the `MAP_ALLOW`.
//! - **The flag's rule.** Legacy's flag mob runs its state through the character state
//!   machine (`CHARACTER::StateFlag`, `G/char_state.cpp:1185-1216`) and the base's through
//!   its own (`CHARACTER::StateFlagBase`, `G/char_state.cpp:1218-1222`); the Rewrite runs
//!   the same two rules from the war map's tick, because the flag's characters carry no
//!   state of their own. The wire is the same; the owner of the rule is not.

use std::collections::{BTreeSet, HashMap, HashSet};

use common::vid::Vid;

use crate::save::{ExitLocation, PASSES_PER_SEC};

use super::guild::GUILD_WAR_TYPE_MAX_NUM;

/// `TGuildWarInfo` (`G/constants.h:165-174`): one row of `KOR_aGuildWarInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WarTypeInfo {
    /// `lMapIndex`: the map the type is fought on. 0 is the field war, which has no map.
    pub map_index: i32,
    /// `iWarPrice`: what each guild pays to declare the type.
    pub price: i32,
    /// `iWinnerPotionRewardPctToWinner`: the winner's share of the loser's potion money.
    pub winner_potion_pct: i32,
    /// `iLoserPotionRewardPctToWinner`: the loser's share of the winner's potion money.
    pub loser_potion_pct: i32,
    /// `iInitialScore`: the score each side starts with.
    pub initial_score: i32,
    /// `iEndScore`: the score that ends the war.
    pub end_score: i32,
}

/// `KOR_aGuildWarInfo` (`G/constants.cpp:1132-1145`): the three war types, in the order of
/// `EGuildWarType` (`common/length.h:768-773`).
pub(super) const WAR_TYPES: [WarTypeInfo; GUILD_WAR_TYPE_MAX_NUM as usize] = [
    // The field war: no map, no price, no score.
    WarTypeInfo {
        map_index: 0,
        price: 0,
        winner_potion_pct: 0,
        loser_potion_pct: 0,
        initial_score: 0,
        end_score: 0,
    },
    // The battle map: the war ends at a hundred points.
    WarTypeInfo {
        map_index: 110,
        price: 0,
        winner_potion_pct: 100,
        loser_potion_pct: 50,
        initial_score: 0,
        end_score: 100,
    },
    // The flag map: the war ends at ten points.
    WarTypeInfo {
        map_index: 111,
        price: 0,
        winner_potion_pct: 100,
        loser_potion_pct: 50,
        initial_score: 0,
        end_score: 10,
    },
];

/// `EWarMapTypes` (`G/war_map.h:7-11`): the normal war map and the flag war map.
pub(super) const WAR_MAP_TYPE_NORMAL: u8 = 0;
pub(super) const WAR_MAP_TYPE_FLAG: u8 = 1;

/// `TWarMapInfo` (`G/war_map.h:13-18`): a war map's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WarMapInfo {
    /// `lMapIndex`: the map the war map is a copy of.
    pub map_index: i32,
    /// `bType`: `WAR_MAP_TYPE_NORMAL` or `WAR_MAP_TYPE_FLAG`.
    pub map_type: u8,
    /// `posStart[3]`: where the two teams and the observers stand
    /// (`CWarMapManager::GetStartPosition`, `G/war_map.cpp:1019-1040`). The first is the
    /// team of the lower guild id, the second the other team's, the third the observers'.
    pub start: [(i32, i32); 3],
}

/// `CWarMapManager::LoadWarMapInfo` (`G/war_map.cpp:974-1008`): the two war maps. The
/// manager's table is built in the code, not read from a file, so the numbers are the
/// code's.
pub(super) const WAR_MAPS: [WarMapInfo; 2] = [
    // Map 110, the normal war map.
    WarMapInfo {
        map_index: 110,
        map_type: WAR_MAP_TYPE_NORMAL,
        // 48 * 100 + 32000, 52 * 100 + 0, and so on: the code's own arithmetic
        // (`G/war_map.cpp:990-995`).
        start: [(36_800, 5_200), (50_300, 20_600), (46_100, 11_700)],
    },
    // Map 111, the flag war map.
    WarMapInfo {
        map_index: 111,
        map_type: WAR_MAP_TYPE_FLAG,
        // 68 * 100 + 57600, 69 * 100 + 0, and so on (`G/war_map.cpp:1003-1008`).
        start: [(64_400, 6_900), (74_700, 18_200), (69_800, 13_100)],
    },
];

/// The number of the instances of one map a private map may take
/// (`SECTREE_MANAGER::CreatePrivateMap`, `G/sectree_manager.cpp:957-962`): the index cap
/// is 10,000 for every map but 107, 108 and 109, which take 51.
const PRIVATE_INDEX_CAP: i32 = 10_000;

/// The Pulses between the war's end and the map's destruction (`CWarMap::SetEnded`,
/// `G/war_map.cpp:777` and `war_end_event`, `G/war_map.cpp:44-68`): the war's end event
/// runs ten seconds after the war ends and sends the players out, and the map goes five
/// seconds after that.
const WAR_MAP_END_DELAY: u64 = (10 + 5) * PASSES_PER_SEC as u64;

/// The Pulses between the war's end and the map's end event (`CWarMap::SetEnded`,
/// `G/war_map.cpp:777`): ten seconds, the delay the event waits before it sends the
/// players out.
const WAR_MAP_EXIT_DELAY: u64 = 10 * PASSES_PER_SEC as u64;

/// The Pulses between the flag's reset and the flags' return (`war_reset_flag_event`,
/// `G/war_map.cpp:962`): ten seconds.
const FLAG_RESET_DELAY: u64 = 10 * PASSES_PER_SEC as u64;

/// The Pulses between two runs of the flag mob's state (`CHARACTER::StateFlag`,
/// `G/char_state.cpp:1186`): `PASSES_PER_SEC(0.5)` is twelve Pulses, so the flag is
/// looked for twice a second.
const FLAG_STATE_PULSES: u64 = 12;

/// The reach of the flag and the base (`FuncFindChrForFlag` and
/// `FuncFindChrForFlagBase`, `G/char_state.cpp:56` and `:122`): `DISTANCE_APPROX` of
/// five hundred units.
pub(super) const FLAG_REACH: i32 = 500;

/// One side's flag: where it is, or who carries it.
///
/// Legacy keeps the flag as a mob (`CWarMap::STeamData`'s `pkChrFlag`,
/// `G/war_map.h:114-133`): the flag stands on the map until a player takes it, and the
/// mob is gone while a player carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WarFlag {
    /// The flag stands on the map at this point.
    OnMap(i32, i32),
    /// A player carries it, under the flag's affect (`AFFECT_WAR_FLAG`).
    Carried(Vid),
}

/// The war's flags (`CWarMap`'s flag mobs, `G/war_map.cpp:831-893`): the flag war's map
/// owns two flags and two bases, and only a flag map has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WarFlags {
    /// The two flags, one for each team: the flag's own guild is the team's, and a flag
    /// that is not on the map is carried or waits for the reset.
    pub flags: [Option<WarFlag>; 2],
    /// The two bases (`pkChrFlagBase`), at the map's start positions.
    pub bases: [(i32, i32); 2],
    /// `m_pkResetFlagEvent`: the Pulse at which the flags come back
    /// (`war_reset_flag_event`, `G/war_map.cpp:913-930`).
    pub reset_pulse: Option<u64>,
    /// `pkChrFlag`: the flag characters' VIDs. A flag that is carried, away or gone has
    /// none (`CWarMap::RemoveFlag`, `G/war_map.cpp:881-893`).
    pub flag_vids: [Option<u32>; 2],
    /// `pkChrFlagBase`: the base characters' VIDs, which stand until the war ends
    /// (`CWarMap::SetEnded`, `G/war_map.cpp:745-768`).
    pub base_vids: [Option<u32>; 2],
}

/// `CWarMap::STeamData` (`G/war_map.h:114-133`): one side of a war.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct WarTeam {
    /// `iMemberCount`: the players of the team on the map now.
    pub members: i32,
    /// `iScore`: the team's points.
    pub score: i32,
    /// `iUsePotionPrice`: what the team's potions have cost.
    pub potion_price: i32,
    /// `set_pidJoiner`: every player that joined the team, and the count the map
    /// reports (`GetAccumulatedJoinerCount`, `G/war_map.cpp:289-292`). A player that
    /// leaves stays in the set: only the count of the players on the map falls
    /// (`RemoveMember`, `G/war_map.cpp:305-309`).
    pub joiners: BTreeSet<u32>,
}

/// `CWarMap` (`G/war_map.h:48-143`): the private map a war is fought on, the two teams
/// that fight on it, and the players that watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WarMap {
    /// `lMapIndex`: the private map index the war is fought on.
    pub map_index: i32,
    /// `m_kMapInfo.lMapIndex`: the map the private map was copied from.
    pub base_map: i32,
    /// `m_kMapInfo.bType`: the war map's type.
    pub map_type: u8,
    /// `m_TeamData[0].dwID` and `m_TeamData[1].dwID`: the two guilds, the lower id
    /// first, which is the order `GetTeamIndex` (`G/war_map.cpp:199-215`) sorts them in.
    pub guilds: [u32; 2],
    /// `m_TeamData`: the two teams.
    pub teams: [WarTeam; 2],
    /// `m_iObserverCount`: the players on the map that watch instead of fight. The
    /// legacy keeps the count and the observer mode on the character
    /// (`SetObserverMode`, `G/war_map.cpp:384`); the Rewrite keeps the players
    /// themselves, so a player that watched the war is known to have watched it.
    pub watchers: BTreeSet<u32>,
    /// `m_set_pkChr`: the players on the map.
    pub present: HashSet<Vid>,
    /// `m_bEnded`: the war is over and the map is closing.
    pub ended: bool,
    /// Whether the map has already sent its players out (`war_end_event`,
    /// `G/war_map.cpp:44-68`): the event's first step runs once.
    pub exited: bool,
    /// The Pulse at which the map sends its players out (`war_end_event`,
    /// `G/war_map.cpp:44-68`): ten seconds after the war ended.
    pub exit_pulse: Option<u64>,
    /// The Pulse at which the map is destroyed (`CWarMap::SetEnded`,
    /// `G/war_map.cpp:739-779`): the war's end event runs ten seconds later and sends
    /// the players out, and the map goes five seconds after that.
    pub destroy_pulse: Option<u64>,
    /// The place each member came from (`SaveExitLocation`, `G/char.cpp:6679-6683`):
    /// the map's end sends its members back there (`ExitToSavedLocation`,
    /// `G/char.cpp:6687-6692`).
    pub exits: HashMap<Vid, ExitLocation>,
    /// `m_dwStartTime`: the Pulse at which the map was created. A score may end the
    /// war only after it has run for thirty seconds (`CWarMap::CheckScore`,
    /// `G/war_map.cpp:695-697`).
    pub started_pulse: u64,
    /// `m_pkBeginEvent`: the Pulse at which the map asks whether the war is over
    /// (`war_begin_event`, `G/war_map.cpp:28-41`). The first ask comes a minute after
    /// the map was created, and then every ten seconds.
    pub check_pulse: Option<u64>,
    /// `m_pkTimeoutEvent`: the Pulse at which the war ends because one of the two
    /// sides has no members (`war_timeout_event`, `G/war_map.cpp:70-84`). The map
    /// asks for it a minute after it says so.
    pub timeout_pulse: Option<u64>,
    /// `m_bTimeout`: the map has asked for a timeout and the war is waiting for it
    /// (`G/war_map.cpp:477-479`). The map says so once.
    pub timed_out: bool,
    /// The map's flags: the two flags and the two bases of a flag map, and nothing for
    /// the other war maps (`CWarMap`'s constructor, `G/war_map.cpp:126-131`).
    pub flags: Option<WarFlags>,
}

impl WarMap {
    /// `CWarMap`'s constructor (`G/war_map.cpp:120-198`): the private map the war is
    /// fought on, the two guilds in the order `GetTeamIndex` sorts them in, and the
    /// score each side starts with.
    ///
    /// The constructor also sets the map's begin event (`war_begin_event`,
    /// `G/war_map.cpp:124-127`): the map asks whether the war is over a minute after
    /// it was created, and then every ten seconds.
    ///
    /// A flag map stands its flags and its bases up at once (`AddFlagBase` and
    /// `AddFlag`, `G/war_map.cpp:126-131`): each flag is on its own base.
    pub(super) fn new(
        map_index: i32,
        base_map: i32,
        map_type: u8,
        guilds: [u32; 2],
        initial_score: i32,
        pulse: u64,
    ) -> Self {
        let mut teams = [WarTeam::default(), WarTeam::default()];
        for team in &mut teams {
            team.score = initial_score;
        }
        // `CWarMap`'s constructor (`G/war_map.cpp:120-198`): the lower guild id takes
        // the first team, so the teams are sorted here.
        let guilds = if guilds[0] <= guilds[1] {
            [guilds[0], guilds[1]]
        } else {
            [guilds[1], guilds[0]]
        };
        // `CWarMap`'s constructor (`G/war_map.cpp:126-131`): only a flag map has flags,
        // and each stands on its own base.
        let flags = if map_type == WAR_MAP_TYPE_FLAG {
            war_map_of(base_map).map(|info| WarFlags {
                flags: [
                    Some(WarFlag::OnMap(info.start[0].0, info.start[0].1)),
                    Some(WarFlag::OnMap(info.start[1].0, info.start[1].1)),
                ],
                bases: [info.start[0], info.start[1]],
                reset_pulse: None,
                // The characters are stood up by the world, which owns the map's
                // characters (`CWarMap`'s constructor adds the bases and the flags,
                // `G/war_map.cpp:126-131`).
                flag_vids: [None, None],
                base_vids: [None, None],
            })
        } else {
            None
        };
        Self {
            map_index,
            base_map,
            map_type,
            guilds,
            teams,
            watchers: BTreeSet::new(),
            present: HashSet::new(),
            ended: false,
            exited: false,
            exit_pulse: None,
            destroy_pulse: None,
            exits: HashMap::new(),
            started_pulse: pulse,
            // The map's begin event (`G/war_map.cpp:124-127`): the first ask comes a
            // minute after the map was created.
            check_pulse: Some(pulse + 60 * u64::from(PASSES_PER_SEC)),
            timeout_pulse: None,
            timed_out: false,
            flags,
        }
    }

    /// The team of the guild `guild` (`CWarMap::GetTeamIndex`,
    /// `G/war_map.cpp:199-215`): the guild of the lower id is the first team.
    pub(super) fn team_of(&self, guild: u32) -> Option<usize> {
        self.guilds.iter().position(|&id| id == guild)
    }

    /// `CWarMap::IncMember` (`G/war_map.cpp:342-398`): the player joins the team of its
    /// guild when it carries the war member's flag, and watches otherwise.
    ///
    /// The flag is dropped when the player's guild is neither of the two: a player whose
    /// guild left the war is no longer a member of it.
    pub(super) fn inc_member(
        &mut self,
        vid: Vid,
        pid: u32,
        guild: u32,
        is_war_member: bool,
    ) -> bool {
        let member = is_war_member && self.team_of(guild).is_some();
        if member {
            let team = &mut self.teams[self.team_of(guild).unwrap_or(0)];
            team.joiners.insert(pid);
            team.members += 1;
        } else {
            self.watchers.insert(pid);
        }
        self.present.insert(vid);
        member
    }

    /// `CWarMap::DecMember`'s count (`G/war_map.cpp:400-451`): the player falls out of
    /// the count of its team, or of the watchers'.
    ///
    /// The player leaves the map's set of players after the map has asked whether the
    /// war is over (`leave_present`), as legacy's `DecMember` does: the player that
    /// left still hears the map's notice.
    pub(super) fn dec_member(&mut self, pid: u32, guild: u32, is_observer: bool) {
        if is_observer {
            self.watchers.remove(&pid);
        } else if let Some(index) = self.team_of(guild) {
            let team = &mut self.teams[index];
            team.members = team.members.saturating_sub(1);
        }
    }

    /// `CWarMap::DecMember`'s last step (`G/war_map.cpp:445`): the player leaves the
    /// map's set of players, which the map's notices and records go to.
    pub(super) fn leave_present(&mut self, vid: Vid) {
        self.present.remove(&vid);
    }

    /// The counts the map tells its players (`CWarMap::UpdateUserCount`,
    /// `G/war_map.cpp:330-340`): the two guilds with the players that joined them, and
    /// the watchers.
    pub(super) fn user_count(&self) -> (u32, usize, u32, usize, i32) {
        (
            self.guilds[0],
            self.teams[0].joiners.len(),
            self.guilds[1],
            self.teams[1].joiners.len(),
            i32::try_from(self.watchers.len()).unwrap_or(i32::MAX),
        )
    }

    /// `CWarMap::SetEnded` (`G/war_map.cpp:739-779`): the war is over, so the map closes.
    ///
    /// Legacy destroys the flag mobs, cancels the flag reset and then waits: the war's
    /// end event runs ten seconds later and sends the players out, and the map goes five
    /// seconds after that. The Rewrite has no flag war, so the first two steps have
    /// nothing to do; the wait is the same.
    ///
    /// A map that is already closing is not closed twice.
    pub(super) fn end(&mut self, pulse: u64) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.exit_pulse = Some(pulse + WAR_MAP_EXIT_DELAY);
        self.destroy_pulse = Some(pulse + WAR_MAP_END_DELAY);
    }

    /// Whether the map is due to send its players out (`war_end_event`,
    /// `G/war_map.cpp:44-68`): the event's first step, ten seconds after the war ended.
    /// The event's step runs once.
    pub(super) fn exit_due(&self, pulse: u64) -> bool {
        !self.exited && self.exit_pulse.is_some_and(|due| due <= pulse)
    }

    /// The map has sent its players out (`war_end_event`, `G/war_map.cpp:57-60`): the
    /// event's first step, which runs once.
    pub(super) fn mark_exited(&mut self) {
        self.exited = true;
    }

    /// Whether the map is due to be destroyed (`war_end_event`, `G/war_map.cpp:44-68`):
    /// the event's second step, five seconds after it sent the players out.
    pub(super) fn destroy_due(&self, pulse: u64) -> bool {
        self.destroy_pulse.is_some_and(|due| due <= pulse)
    }

    /// `CWarMap::UpdateScore` (`G/war_map.cpp:660-681`): the side's own score changes,
    /// and the map tells its players. Reports whether the score changed, as legacy's
    /// comparison does.
    pub(super) fn set_score(&mut self, guild: u32, score: i32) -> bool {
        let Some(index) = self.team_of(guild) else {
            return false;
        };
        if self.teams[index].score == score {
            return false;
        }
        self.teams[index].score = score;
        true
    }

    /// `CWarMap::CheckScore`'s first rule (`G/war_map.cpp:695-697`): the war must have
    /// run for thirty seconds before a score can end it.
    pub(super) fn score_can_end(&self, pulse: u64) -> bool {
        pulse.saturating_sub(self.started_pulse) >= 30 * u64::from(PASSES_PER_SEC)
    }

    /// Whether the map is due to ask whether the war is over (`war_begin_event`,
    /// `G/war_map.cpp:28-41`): the event runs a minute after the map was created and
    /// then every ten seconds.
    pub(super) fn check_due(&self, pulse: u64) -> bool {
        self.check_pulse.is_some_and(|due| due <= pulse)
    }

    /// The next ask (`war_begin_event` returns `PASSES_PER_SEC(10)`,
    /// `G/war_map.cpp:40`): ten seconds after the one that ran.
    pub(super) fn mark_checked(&mut self) {
        if let Some(due) = self.check_pulse {
            self.check_pulse = Some(due + 10 * u64::from(PASSES_PER_SEC));
        }
    }

    /// `CWarMap::CheckWarEnd`'s timeout arm (`G/war_map.cpp:474-491`): a side with no
    /// members ends the war a minute later (`war_timeout_event`,
    /// `G/war_map.cpp:70-84`).
    pub(super) fn ask_timeout(&mut self, pulse: u64) {
        self.timeout_pulse = Some(pulse + 60 * u64::from(PASSES_PER_SEC));
    }

    /// Whether the timeout is due (`war_timeout_event`, `G/war_map.cpp:70-84`).
    pub(super) fn timeout_due(&self, pulse: u64) -> bool {
        self.timeout_pulse.is_some_and(|due| due <= pulse)
    }

    /// `IncMember` cancels the timeout the map asked for (`event_cancel(&m_pkTimeoutEvent)`,
    /// `G/war_map.cpp:374`): a member that joins the map ends the wait for the side
    /// that never came, and the map may ask for another one.
    pub(super) fn cancel_timeout(&mut self) {
        self.timeout_pulse = None;
    }

    /// The timeout has run (`CWarMap::Timeout`, `G/war_map.cpp:576`): the war is over,
    /// and the map says so once.
    pub(super) fn mark_timed_out(&mut self) {
        self.timed_out = true;
        self.timeout_pulse = None;
    }

    /// The flag of the team `index`, and where it is (`CWarMap::STeamData`'s
    /// `pkChrFlag`, `G/war_map.h:114-133`).
    pub(super) fn flag_of(&self, index: usize) -> Option<WarFlag> {
        self.flags
            .as_ref()
            .and_then(|flags| flags.flags.get(index).copied().flatten())
    }

    /// The base of the team `index` (`pkChrFlagBase`, `G/war_map.h:114-133`): the point
    /// the team's flag stands on, and the point the base's character stands on.
    pub(super) fn base_of(&self, index: usize) -> Option<(i32, i32)> {
        self.flags.as_ref()?.bases.get(index).copied()
    }

    /// `CWarMap::IsFlagOnBase` (`G/war_map.cpp:898-911`): the flag stands on its base.
    ///
    /// The flag starts on its base (`AddFlag`, `G/war_map.cpp:855-879`, spawns it at the
    /// map's start point, which is where `AddFlagBase` spawns the base), and a flag that
    /// is carried is nowhere near it.
    pub(super) fn is_flag_on_base(&self, index: usize) -> bool {
        let Some(flags) = self.flags.as_ref() else {
            return false;
        };
        let Some(base) = flags.bases.get(index) else {
            return false;
        };
        matches!(flags.flags.get(index), Some(Some(WarFlag::OnMap(x, y))) if *x == base.0 && *y == base.1)
    }

    /// `CWarMap::RemoveFlag` (`G/war_map.cpp:881-893`) and `StateFlag`'s last step
    /// (`G/char_state.cpp:1211`): the flag leaves the map, and a player carries it.
    ///
    /// Reports whether the flag was on the map: a flag that is already carried or away
    /// is not taken twice.
    pub(super) fn carry_flag(&mut self, index: usize, carrier: Vid) -> bool {
        let Some(flags) = self.flags.as_mut() else {
            return false;
        };
        let Some(slot) = flags.flags.get_mut(index) else {
            return false;
        };
        if !matches!(slot, Some(WarFlag::OnMap(_, _))) {
            return false;
        }
        *slot = Some(WarFlag::Carried(carrier));
        true
    }

    /// `CWarMap::OnKill`'s flag arm (`G/war_map.cpp:811-823`): the flag lies where its
    /// carrier fell.
    ///
    /// Reports whether the flag was carried: a flag that is on the map stays where it
    /// is.
    pub(super) fn drop_flag(&mut self, index: usize, at: (i32, i32)) -> bool {
        let Some(flags) = self.flags.as_mut() else {
            return false;
        };
        let Some(slot) = flags.flags.get_mut(index) else {
            return false;
        };
        if !matches!(slot, Some(WarFlag::Carried(_))) {
            return false;
        }
        *slot = Some(WarFlag::OnMap(at.0, at.1));
        true
    }

    /// `CWarMap::ResetFlag` (`G/war_map.cpp:941-964`): the flags leave the map and come
    /// back ten seconds later.
    ///
    /// Reports whether the reset happened: a reset that already waits does nothing
    /// (`if (m_pkResetFlagEvent) return;`), and a map that has ended keeps its flags
    /// (`if (m_bEnded) return;`).
    pub(super) fn reset_flags(&mut self, pulse: u64) -> bool {
        if self.ended {
            return false;
        }
        let Some(flags) = self.flags.as_mut() else {
            return false;
        };
        if flags.reset_pulse.is_some() {
            return false;
        }
        flags.flags = [None, None];
        flags.reset_pulse = Some(pulse + FLAG_RESET_DELAY);
        true
    }

    /// Whether the reset is due (`war_reset_flag_event`, `G/war_map.cpp:913-930`).
    pub(super) fn reset_due(&self, pulse: u64) -> bool {
        self.flags
            .as_ref()
            .is_some_and(|flags| flags.reset_pulse.is_some_and(|due| due <= pulse))
    }

    /// `war_reset_flag_event` (`G/war_map.cpp:913-930`): the flags are back on their
    /// bases, and the reset waits for nothing.
    pub(super) fn flags_back(&mut self) {
        let Some(flags) = self.flags.as_mut() else {
            return;
        };
        flags.flags = [
            Some(WarFlag::OnMap(flags.bases[0].0, flags.bases[0].1)),
            Some(WarFlag::OnMap(flags.bases[1].0, flags.bases[1].1)),
        ];
        flags.reset_pulse = None;
    }

    /// Whether the flag mob's state is due (`CHARACTER::StateFlag`,
    /// `G/char_state.cpp:1186`): the state runs every half a second, so the flag is
    /// looked for twice a second.
    pub(super) fn flag_state_due(&self, pulse: u64) -> bool {
        let since = pulse.saturating_sub(self.started_pulse);
        since >= FLAG_STATE_PULSES && since % FLAG_STATE_PULSES == 0
    }
}

/// The war map a map index belongs to (`CWarMapManager::GetWarMapInfo`,
/// `G/war_map.cpp:1069-1080`): a private map's index names the map it was copied from,
/// and the index itself names the map it is.
#[must_use]
pub(super) fn war_map_of(map_index: i32) -> Option<&'static WarMapInfo> {
    let base = if map_index >= 10_000 {
        map_index / 10_000
    } else {
        map_index
    };
    WAR_MAPS.iter().find(|info| info.map_index == base)
}

/// The start position of one side of a war (`CWarMapManager::GetStartPosition`,
/// `G/war_map.cpp:1019-1040`): the side 0 is the team of the lower guild id, the side 1
/// the other team's, and the side 2 the watchers'. The private map's index names the map
/// it was copied from, so the position is the base map's.
#[must_use]
pub(super) fn start_position(map_index: i32, side: usize) -> Option<(i32, i32)> {
    let info = war_map_of(map_index)?;
    info.start.get(side).copied()
}

/// The private map index a war takes of a map (`SECTREE_MANAGER::CreatePrivateMap`,
/// `G/sectree_manager.cpp:943-1026`): the map's own index times 10,000 plus the first
/// free instance under it. `taken` names the instances the process holds.
///
/// The index cap is 10,000 for every map but 107, 108 and 109, which legacy caps at 51
/// (or at 1 on a test server, which the Rewrite does not have).
#[must_use]
pub(super) fn private_index(map_index: i32, taken: &[i32]) -> Option<i32> {
    if map_index <= 0 || map_index >= 10_000 {
        return None;
    }
    let cap = if matches!(map_index, 107..=109) {
        51
    } else {
        PRIVATE_INDEX_CAP
    };
    let base = map_index * 10_000;
    (0..cap)
        .map(|offset| base + offset)
        .find(|index| !taken.contains(index))
}

#[cfg(test)]
mod tests {
    use super::{private_index, war_map_of, GUILD_WAR_TYPE_MAX_NUM, WAR_MAPS, WAR_TYPES};
    use crate::game_state::guild::{
        GUILD_WAR_TYPE_BATTLE, GUILD_WAR_TYPE_FIELD, GUILD_WAR_TYPE_FLAG,
    };

    #[test]
    fn the_war_types_are_legacy_s() {
        // `KOR_aGuildWarInfo` (`G/constants.cpp:1132-1145`): the field war has no map and
        // no price, the battle map's war ends at a hundred points and the flag map's at ten.
        assert_eq!(WAR_TYPES.len(), usize::from(GUILD_WAR_TYPE_MAX_NUM));
        assert_eq!(WAR_TYPES[usize::from(GUILD_WAR_TYPE_FIELD)].map_index, 0);
        assert_eq!(WAR_TYPES[usize::from(GUILD_WAR_TYPE_FIELD)].price, 0);
        assert_eq!(WAR_TYPES[usize::from(GUILD_WAR_TYPE_FIELD)].end_score, 0);
        assert_eq!(WAR_TYPES[usize::from(GUILD_WAR_TYPE_BATTLE)].map_index, 110);
        assert_eq!(
            WAR_TYPES[usize::from(GUILD_WAR_TYPE_BATTLE)].winner_potion_pct,
            100
        );
        assert_eq!(
            WAR_TYPES[usize::from(GUILD_WAR_TYPE_BATTLE)].loser_potion_pct,
            50
        );
        assert_eq!(WAR_TYPES[usize::from(GUILD_WAR_TYPE_BATTLE)].end_score, 100);
        assert_eq!(WAR_TYPES[usize::from(GUILD_WAR_TYPE_FLAG)].map_index, 111);
        assert_eq!(WAR_TYPES[usize::from(GUILD_WAR_TYPE_FLAG)].end_score, 10);
    }

    #[test]
    fn the_war_maps_are_legacy_s() {
        // `CWarMapManager::LoadWarMapInfo` (`G/war_map.cpp:974-1008`): two maps, the
        // normal one and the flag one.
        assert_eq!(WAR_MAPS.len(), 2);
        assert_eq!(WAR_MAPS[0].map_index, 110);
        assert_eq!(WAR_MAPS[1].map_index, 111);
    }

    #[test]
    fn a_private_map_names_the_map_it_was_copied_from() {
        // `GetWarMapInfo` (`G/war_map.cpp:1069-1080`): the index of a private map is its
        // base times 10,000 plus the instance.
        assert_eq!(war_map_of(110).map(|info| info.map_index), Some(110));
        assert_eq!(war_map_of(1_100_000).map(|info| info.map_index), Some(110));
        assert_eq!(war_map_of(1_110_003).map(|info| info.map_index), Some(111));
        assert!(war_map_of(1).is_none());
        assert!(war_map_of(0).is_none());
    }

    #[test]
    fn a_war_map_holds_two_teams_and_the_watchers() {
        use super::WarMap;
        let mut map = WarMap::new(1_100_000, 110, 0, [7, 3], 0, 0);
        // `GetTeamIndex` (`G/war_map.cpp:199-215`): the guild of the lower id is the
        // first team.
        assert_eq!(map.team_of(3), Some(0));
        assert_eq!(map.team_of(7), Some(1));
        assert_eq!(map.team_of(9), None);
        // `IncMember` (`G/war_map.cpp:342-398`): a player that carries the war member's
        // flag joins the team of its guild, a player without it watches.
        let fighter = common::vid::Vid::new(11);
        assert!(map.inc_member(fighter, 11, 3, true));
        assert_eq!(map.teams[0].members, 1);
        assert_eq!(map.watchers.len(), 0);
        let watcher = common::vid::Vid::new(12);
        assert!(!map.inc_member(watcher, 12, 3, false));
        assert_eq!(map.teams[0].members, 1);
        assert_eq!(map.watchers.len(), 1);
        // `UpdateUserCount` (`G/war_map.cpp:330-340`): the two guilds with the players
        // that joined them, and the watchers.
        assert_eq!(map.user_count(), (3, 1, 7, 0, 1));
        // `DecMember` (`G/war_map.cpp:400-451`): a member falls out of its team, and a
        // watcher out of the watchers.
        map.dec_member(11, 3, false);
        map.leave_present(fighter);
        assert_eq!(map.teams[0].members, 0);
        assert_eq!(map.watchers.len(), 1);
        map.dec_member(12, 3, true);
        map.leave_present(watcher);
        assert_eq!(map.teams[0].members, 0);
        assert_eq!(map.watchers.len(), 0);
    }

    #[test]
    fn a_private_map_takes_the_first_free_instance_of_its_base() {
        // `CreatePrivateMap` (`G/sectree_manager.cpp:943-1026`): the base is the map's own
        // index times 10,000, and the instance is the first free one under it.
        assert_eq!(private_index(110, &[]), Some(1_100_000));
        assert_eq!(private_index(110, &[1_100_000]), Some(1_100_001));
        assert_eq!(private_index(110, &[1_100_000, 1_100_001]), Some(1_100_002));
        // The cap is 51 for the three maps legacy caps, and 10,000 for the rest.
        let taken: Vec<i32> = (0..51).map(|offset| 1_070_000 + offset).collect();
        assert_eq!(private_index(107, &taken), None);
        assert_eq!(private_index(108, &[]), Some(1_080_000));
        // A map that is itself private, or the field war's 0, takes no instance.
        assert_eq!(private_index(0, &[]), None);
        assert_eq!(private_index(1_100_000, &[]), None);
    }
}
