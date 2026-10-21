//! The war maps (`G/war_map.cpp`): the war types, the two maps a guild war can be fought
//! on, and the private copy a war takes of the map it is fought on.
//!
//! Legacy keeps the war map information in the manager's own table
//! (`CWarMapManager::LoadWarMapInfo`, `G/war_map.cpp:974-1008`): two maps, three start
//! positions each, and a type. The war types are a table of their own,
//! `KOR_aGuildWarInfo` (`G/constants.cpp:1132-1145`), and each names the map it is fought
//! on — 0 for the field war, which is fought where the players already stand.
//!
//! # What is not here yet
//!
//! The start positions (the war's members are warped to them when they join, which is the
//! `guild_war_join` quest's `guild.war_enter`), the war's members, the observers, the flags
//! (`warmap::WAR_FLAG_VNUM0` and the base, `G/war_map.h:24-32`), the score the map keeps
//! and the map's own end (its timeout, its draw and its exit) are ledger 237's. This module
//! holds the map the war is fought on.
//!
//! # Divergences
//!
//! - **The map index and the Channel's map set.** Legacy checks the map twice: the declare
//!   asks the map index (`GuildWar_IsWarMap`, `G/guild_war.cpp:260-271`) and the wait-start
//!   asks the Channel's `MAP_ALLOW` (`map_allow_find`, `G/guild_war.cpp:488-493`). The
//!   Rewrite keeps the two checks apart: the atlas's indices are the map index, and the
//!   hosted maps are the `MAP_ALLOW`.

use std::collections::{BTreeSet, HashSet};

use common::vid::Vid;

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
    /// `m_iObserverCount`: the players on the map that watch instead of fight.
    pub observers: i32,
    /// `m_set_pkChr`: the players on the map.
    pub present: HashSet<Vid>,
}

impl WarMap {
    /// `CWarMap`'s constructor (`G/war_map.cpp:120-198`): the private map the war is
    /// fought on, the two guilds in the order `GetTeamIndex` sorts them in, and the
    /// score each side starts with.
    pub(super) fn new(
        map_index: i32,
        base_map: i32,
        map_type: u8,
        guilds: [u32; 2],
        initial_score: i32,
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
        Self {
            map_index,
            base_map,
            map_type,
            guilds,
            teams,
            observers: 0,
            present: HashSet::new(),
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
            self.observers += 1;
        }
        self.present.insert(vid);
        member
    }

    /// `CWarMap::DecMember` (`G/war_map.cpp:400-451`): the player leaves the map. A
    /// member of a team falls out of the count; a watcher falls out of the observer's.
    pub(super) fn dec_member(&mut self, vid: Vid, guild: u32, is_observer: bool) {
        if is_observer {
            self.observers -= 1;
        } else if let Some(index) = self.team_of(guild) {
            let team = &mut self.teams[index];
            team.members -= 1;
        }
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
            self.observers,
        )
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
        let mut map = WarMap::new(1_100_000, 110, 0, [7, 3], 0);
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
        assert_eq!(map.observers, 0);
        let watcher = common::vid::Vid::new(12);
        assert!(!map.inc_member(watcher, 12, 3, false));
        assert_eq!(map.teams[0].members, 1);
        assert_eq!(map.observers, 1);
        // `UpdateUserCount` (`G/war_map.cpp:330-340`): the two guilds with the players
        // that joined them, and the watchers.
        assert_eq!(map.user_count(), (3, 1, 7, 0, 1));
        // `DecMember` (`G/war_map.cpp:400-451`): a member falls out of its team, and a
        // watcher out of the watchers.
        map.dec_member(fighter, 3, false);
        assert_eq!(map.teams[0].members, 0);
        assert_eq!(map.observers, 1);
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
