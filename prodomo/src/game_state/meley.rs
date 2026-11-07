//! Meley's Lair (`MeleyLair::CMgr`, `G/MeleyLair.cpp`): the guild dungeon — the
//! register and enter gates, the leave and check warps, the participant rows, the
//! fight's steps, spawns, seals and rewards, and the dungeon triple each guild
//! carries (`dungeon_ch`, `dungeon_map`, `dungeon_cooldown`, `G/guild.h:433-438`).
//!
//! Legacy keeps one manager per game core: the registered guilds by id
//! (`m_RegGuilds`), the dungeon's centre and the sub map's warp spot (read once
//! from the hosted sectree settings, lazily on the first register). The one
//! process keeps the registrations on the game thread, keyed by guild id, and
//! reads the private map through the dungeon manager: `CreatePrivateMap` and
//! `DestroyPrivateMap` are the same calls the party dungeons use.
//!
//! A registration holds the `CMgrMap`: the private index, the participant PIDs
//! (`v_Partecipants`), the step, the statue/boss/chest VIDs, the kill counters,
//! the reward and stone clocks, the sealers and the rewarded, and the tick dues.
//! The fight's mobs stand on the run's map through the NPC store, as the ship's
//! deck does; the run keeps only the VIDs legacy's pointers name.
//!
//! The dungeon triple is memory-only and starts at zero, as a fresh guild's does:
//! legacy receives it from the DB peer (`RecvDungeon`, `RecvDungeonCD`), which the
//! Rewrite retired (ADR-0001), and the ported game never reads it from a row. The
//! ladder debit the register takes is the call's own store half.

use std::collections::{BTreeMap, BTreeSet};

/// The dungeon map (`MeleyLair.h:16`).
pub(super) const MAP: i32 = 212;

/// The sub map the shelter's NPC stands on (`MeleyLair.h:17`).
pub(super) const SUBMAP: i32 = 62;

/// The minimum guild level `MeleyLair.Register` takes (`MeleyLair.h:18`).
pub(super) const MIN_LEVEL: u8 = 3;

/// The most participants a run takes (`MeleyLair.h:19`).
pub(super) const PARTECIPANTS_LIMIT: usize = 16;

/// The ladder the register takes (`MeleyLair.h:20`) and a failed start returns
/// (`EndDungeon`, `G/MeleyLair.cpp:605-713`).
pub(super) const LADDER_COST: i32 = 600;

/// The ladder a win returns (`MeleyLair.h:21`); the timeout returns it too.
pub(super) const LADDER_RETURN: i32 = 300;

/// The cooldown the end stamps (`MeleyLair.h:22`): 10800 seconds plus the call's 5.
pub(super) const COOLDOWN_DUNGEON: u32 = 10_800;

/// The fight's hour (`MeleyLair.h:30`).
pub(super) const TIME_LIMIT_DUNGEON: u32 = 3_600;

/// The shelter's NPC (`MeleyLair.h:23`).
pub(super) const NPC_VNUM: u32 = 20419;

/// The NPC's cell (`Start`, `G/MeleyLair.cpp:561`).
pub(super) const NPC_CELL: (i32, i32) = (135, 179);

/// The gate whose click starts the fight (`MeleyLair.h:24`).
pub(super) const GATE_VNUM: u32 = 20388;

/// Meley herself (`MeleyLair.h:25`).
pub(super) const BOSS_VNUM: u32 = 6193;

/// The four statues (`MeleyLair.h:26`).
pub(super) const STATUE_VNUM: u32 = 6118;

/// The reward chest (`MeleyLair.h:27`).
pub(super) const CHEST_VNUM: u32 = 20420;

/// The seal that breaks a step-4 statue (`MeleyLair.h:31`).
pub(super) const SEAL_VNUM: u32 = 30341;

/// The ten seconds to break the statues (`MeleyLair.h:32`).
pub(super) const TIME_LIMIT_TO_KILL_STATUE: u32 = 10;

/// The common respawns' vnums (`MeleyLair.h:34`, `:38`, `:43`).
pub(super) const COMMON_VNUMS: [u32; 3] = [6112, 6113, 6115];

/// The common respawns' reloads in seconds (`MeleyLair.h:33`, `:37`, `:42`).
pub(super) const COMMON_RELOADS: [u32; 3] = [5, 30, 30];

/// The common respawns' count (`MeleyLair.h:35`).
pub(super) const COMMON_COUNT: usize = 12;

/// The step-2/3 stones (`MeleyLair.h:39-40`).
pub(super) const STONE_VNUM: u32 = 20422;

/// How many stones stand (`MeleyLair.h:40`).
pub(super) const STONE_COUNT: usize = 4;

/// The step-3 bosses (`MeleyLair.h:44-46`).
pub(super) const BOSS3_VNUM: u32 = 6116;

/// The step-3 sub-bosses, two per stone spot (`MeleyLair.h:45`).
pub(super) const SUBBOSS3_VNUM: u32 = 6117;

/// How many step-3 boss kills open the statues (`MeleyLair.h:46`).
pub(super) const BOSS3_KILLS: u32 = 4;

/// The chest's two rewards, by the `Reward` argument (`MeleyLair.h:28-29`): 1
/// takes the first, anything else the second.
pub(super) const REWARD_VNUMS: [u32; 2] = [50270, 50271];

/// The sub map warp spot in map units (`Register`, `G/MeleyLair.cpp:1063-1065`):
/// the sub map's base plus (87, 853).
pub(super) const SUB_SPOT: (i32, i32) = (87, 853);

/// The dungeon's centre in map units (`Register`, `G/MeleyLair.cpp:1053-1055`):
/// the dungeon base plus (130, 130).
pub(super) const CENTRE_SPOT: (i32, i32) = (130, 130);

/// The gate, boss and statue cells (`Start`, `G/MeleyLair.cpp:558-562`): the NPC
/// (20419) at (135, 179), the gate at (129, 175), the boss at (130, 77), and the
/// statues at (123, 137), (123, 124), (136, 123) and (137, 137).
pub(super) const GATE_CELL: (i32, i32) = (129, 175);
pub(super) const BOSS_CELL: (i32, i32) = (130, 77);
pub(super) const STATUE_CELLS: [(i32, i32); 4] = [(123, 137), (123, 124), (136, 123), (137, 137)];

/// The stone spots (`stoneSpawnPos`, `G/MeleyLair.cpp:22-28`).
pub(super) const STONE_CELLS: [(i32, i32); 4] = [(140, 131), (130, 122), (121, 128), (128, 140)];

/// The common spots (`monsterSpawnPos`, `G/MeleyLair.cpp:30-44`).
pub(super) const MONSTER_CELLS: [(i32, i32); 12] = [
    (140, 131),
    (130, 122),
    (121, 128),
    (128, 140),
    (128, 142),
    (128, 131),
    (135, 130),
    (141, 126),
    (128, 122),
    (117, 127),
    (118, 136),
    (126, 142),
];

/// One registered guild: the `CMgrMap`. The lobby reads the index and the
/// participants; the fight reads the rest.
#[derive(Clone, Debug, Default)]
pub(super) struct MeleyRun {
    /// The private index `CreatePrivateMap` gave (`CMgrMap::map_index`).
    pub index: i32,
    /// The participant PIDs (`CMgrMap::v_Partecipants`).
    pub participants: BTreeSet<u32>,
    /// The dungeon step (`CMgrMap::dungeon_step`): 0 before the gate click.
    pub step: u8,
    /// The shelter NPC's VID (`CMgrMap::pkMainNPC`).
    pub main_npc: Option<u32>,
    /// The gate's VID (`CMgrMap::pkGate`), `None` once the fight starts.
    pub gate: Option<u32>,
    /// The boss's VID (`CMgrMap::pkBoss`).
    pub boss: Option<u32>,
    /// The four statues' VIDs (`CMgrMap::pkStatue1..4`).
    pub statues: [Option<u32>; 4],
    /// The reward chest's VID, once the statues fall.
    pub chest: Option<u32>,
    /// The kills the statues count (`CMgrMap::kill_stonesCount`).
    pub stones_killed: u32,
    /// The kills the bosses count (`CMgrMap::kill_bossesCount`).
    pub bosses_killed: u32,
    /// The epoch second the stones may stand again
    /// (`CMgrMap::last_stoneKilled`).
    pub stones_at: u32,
    /// The epoch second the chest opens (`CMgrMap::reward`).
    pub reward_at: u32,
    /// The PIDs that broke a statue (`CMgrMap::v_Already`), in break order.
    pub sealers: Vec<u32>,
    /// The PIDs that took a reward (`CMgrMap::v_Rewards`), in take order.
    pub rewarded: Vec<u32>,
    /// The fight's start in epoch seconds (`CMgrMap::time_start`).
    pub started_at: u32,
    /// The hour's due Pulse (`e_pEndEvent`), or the warp's once the end runs.
    pub end_due: Option<(bool, u64)>,
    /// The spawn tick's due Pulse (`e_SpawnEvent`): the step and its first run.
    pub spawn_due: Option<(u8, bool, u64)>,
    /// The effect tick's due Pulse (`e_SEffectEvent`): the step and the pass.
    pub effect_due: Option<(u8, u8, u64)>,
    /// The statue kill tick's due Pulse (`e_DestroyStatues`): its first run.
    pub statues_due: Option<(bool, u64)>,
}

/// The registered guilds (`MeleyLair::CMgr::m_RegGuilds`), by guild id.
#[derive(Clone, Debug, Default)]
pub(super) struct MeleyRuns {
    /// The live registrations.
    by_guild: BTreeMap<u32, MeleyRun>,
}

/// Whether `map` is Meley's map: 212 itself, or its 2120000..2130000 private
/// copies (`CMgr::IsMeleyMap`, `G/MeleyLair.cpp:1215-1222`). The logout and login
/// flows, the PK fold and the quest read share it.
pub(super) fn is_meley_map(map: i32) -> bool {
    map == MAP || (2_120_000..=2_130_000).contains(&map)
}

/// Whether `CreatePrivateMap` could copy `base`: the atlas names it, which is what
/// `SECTREE_MANAGER::CreatePrivateMap` checks first (`G/sectree_manager.cpp:943-956`).
pub(super) fn base_hosted(
    atlas: Option<&gamedata::map_atlas::MapAtlas>,
    channel: u8,
    base: i32,
) -> bool {
    let _ = channel;
    atlas.and_then(|atlas| atlas.region(base)).is_some()
}

/// `GetRecallPositionByEmpire(212, 0)` (`G/sectree_manager.cpp:493-526`): the map's
/// own town spawn, already absolute — the atlas builds it from the base plus the
/// `Town.txt` offset (`LoadMapRegion`, `G/sectree_manager.cpp:357-360`), so no second
/// scaling. `None` with no region, as the empire-0 `false` is. Empire 0 never takes
/// the per-empire points.
pub(super) fn recall_spot(
    atlas: Option<&gamedata::map_atlas::MapAtlas>,
    map: i32,
) -> Option<(i32, i32)> {
    atlas
        .and_then(|atlas| atlas.region(map))
        .map(|region| region.spawn)
}

impl MeleyRuns {
    /// The registration of `guild`, when one lives.
    pub(super) fn of(&self, guild: u32) -> Option<&MeleyRun> {
        self.by_guild.get(&guild)
    }

    /// The registration of `guild`, mutably, when one lives.
    pub(super) fn of_mut(&mut self, guild: u32) -> Option<&mut MeleyRun> {
        self.by_guild.get_mut(&guild)
    }

    /// Register `guild` under `index` (`Register`, `G/MeleyLair.cpp:1072`).
    pub(super) fn insert(&mut self, guild: u32, index: i32) {
        self.by_guild.insert(
            guild,
            MeleyRun {
                index,
                ..MeleyRun::default()
            },
        );
    }

    /// Whether no guild is registered.
    pub(super) fn is_empty(&self) -> bool {
        self.by_guild.is_empty()
    }

    /// Every registered guild id, in id order.
    pub(super) fn guilds(&self) -> Vec<u32> {
        self.by_guild.keys().copied().collect()
    }
}

use common::vid::Vid;

use super::guild::{GuildQuestStore, MeleyGuildRow};
use super::GameState;
use crate::item_move::{MovedItems, Mover};

impl GameState {
    /// Judge the step's `MeleyLair.Register`, `MeleyLair.Enter`, `MeleyLair.Check`
    /// and `MeleyLair.Leave` calls, in legacy call order per name (`Register`,
    /// `Enter`, `Check`, `LeaveRequest`, `G/MeleyLair.cpp:992-1214`): the register
    /// stands the run and debits the ladder, the enter warps the reader in, the
    /// check and the leave warp a stranger out. The ladder debits land for the
    /// descriptor to drain; the `[LS;1001]` lines ride the reader's own burst.
    /// Each judge reads the world the one before it left, as legacy's in-call
    /// runs do.
    pub(super) fn judge_meley_calls(
        &mut self,
        reader: Vid,
        calls: (u32, u32, u32, u32, Vec<u8>),
        now: u32,
    ) -> (Vec<GuildQuestStore>, Vec<MovedItems>) {
        let mut stores = Vec::new();
        let mut rewards = Vec::new();
        for _ in 0..calls.0 {
            stores.extend(self.judge_meley_register(reader, now));
        }
        for _ in 0..calls.1 {
            self.judge_meley_enter(reader);
        }
        for _ in 0..calls.2 {
            self.judge_meley_check(reader);
        }
        for _ in 0..calls.3 {
            self.judge_meley_leave(reader);
        }
        for arg in calls.4 {
            if let Some((moved, _)) = self.meley_reward(reader, arg, now) {
                rewards.push(moved);
            }
        }
        (stores, rewards)
    }

    /// The reader's guild, when it holds one.
    fn meley_guild_of(&self, reader: Vid) -> Option<u32> {
        let pid = self.characters.find_by_vid(reader).ok()?.player_id();
        self.guild_of(pid)
    }

    /// The register's reads of guild `id`: its master, level, ladder and dungeon
    /// triple.
    fn meley_guild_row(&self, id: u32) -> Option<MeleyGuildRow> {
        self.guilds.shadow_meley_guild(id)
    }

    /// Write the ladder and the dungeon triple of guild `id`.
    fn set_meley_guild_row(&mut self, id: u32, ladder: i32, dungeon: (u8, i32, u32)) {
        self.guilds.set_meley_row(id, ladder, dungeon);
    }

    /// Judge one `MeleyLair.Register` (`Register`, `G/MeleyLair.cpp:992-1073`):
    /// the gates in legacy order, then the map, the double write, the triple and
    /// the run. The ladder store half lands; the `[LS;1001]` line is the reader's
    /// own burst, as `SetLadderPoint` chats it. Answers the pair the shadow
    /// already pushed, for the record the test reads.
    fn judge_meley_register(&mut self, reader: Vid, now: u32) -> Option<GuildQuestStore> {
        let pid = self.characters.find_by_vid(reader).ok()?.player_id();
        let id = self.guild_of(pid)?;
        let (master, level, ladder, (dungeon_ch, _, cooldown)) = self.meley_guild_row(id)?;
        if master != pid {
            return None;
        }
        if self.meley_registered(id) {
            return None;
        }
        if dungeon_ch != 0 {
            return None;
        }
        if level < MIN_LEVEL {
            return None;
        }
        if ladder < LADDER_COST {
            return None;
        }
        if cooldown > now {
            return None;
        }
        let channel = self.place_of(reader.raw())?.0;
        let index = self.create_meley_map(channel)?;
        let ladder = ladder.saturating_sub(LADDER_COST).max(0);
        self.set_meley_guild_row(id, ladder, (channel, index, 0));
        self.meley.insert(id, index);
        self.stand_meley_start(id);
        self.tell_ladder(id, ladder);
        Some(GuildQuestStore::Ladder {
            guild_id: id,
            ladder,
        })
    }

    /// Judge one `MeleyLair.Enter` (`Enter`, `G/MeleyLair.cpp:1075-1154`): the
    /// cross-Channel refusal, the late create, the full, finished (`limit` 3 at
    /// step 4) and PK gates, then the participant row, the exit save and the
    /// ordered warp.
    fn judge_meley_enter(&mut self, reader: Vid) {
        let Some(id) = self.meley_guild_of(reader) else {
            return;
        };
        let channel = self.place_of(reader.raw()).map_or(0, |place| place.0);
        if self.meley.of(id).is_none() {
            let (_, _, ladder, (dungeon_ch, _, cooldown)) =
                self.meley_guild_row(id).unwrap_or((0, 0, 0, (0, 0, 0)));
            if dungeon_ch != 0 && dungeon_ch != channel {
                return;
            }
            if channel != dungeon_ch {
                return;
            }
            let Some(index) = self.create_meley_map(channel) else {
                let ladder = ladder.saturating_add(LADDER_COST).max(0);
                self.set_meley_guild_row(id, ladder, (dungeon_ch, 0, cooldown));
                self.tell_ladder(id, ladder);
                return;
            };
            self.set_meley_guild_row(id, ladder, (channel, index, cooldown));
            self.meley.insert(id, index);
            self.stand_meley_start(id);
        }
        let Some(run) = self.meley.of(id) else {
            return;
        };
        let index = run.index;
        if run.participants.len() >= PARTECIPANTS_LIMIT {
            return;
        }
        // The finished gate (`limit` 3) reads the live step, as `Enter` does.
        if run.step >= 4 {
            return;
        }
        let pk = self.bodies.get(&reader).map_or(0, |body| body.card.pk_mode);
        if pk != crate::loading_phase::PK_MODE_GUILD {
            return;
        }
        let Some((x, y)) = self
            .recall
            .atlas()
            .and_then(|atlas| recall_spot(Some(atlas), MAP))
        else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(reader) else {
            return;
        };
        let pid = character.player_id();
        if let Some(run) = self.meley.of_mut(id) {
            run.participants.insert(pid);
        }
        self.order_meley_enter(reader, index, x, y);
    }

    /// Judge one `MeleyLair.Check` (`Check`, `G/MeleyLair.cpp:1224-1248`): a
    /// stranger on a live run's map is warped out.
    fn judge_meley_check(&mut self, reader: Vid) {
        let Some(id) = self.meley_guild_of(reader) else {
            self.order_meley_out(reader);
            return;
        };
        let Some(run) = self.meley.of(id) else {
            self.order_meley_out(reader);
            return;
        };
        let map = self.place_of(reader.raw()).map_or(0, |place| place.1);
        if map != run.index {
            return;
        }
        let pid = self
            .characters
            .find_by_vid(reader)
            .map_or(0, world::character::Character::player_id);
        if !run.participants.contains(&pid) {
            self.order_meley_out(reader);
        }
    }

    /// Judge one `MeleyLair.Leave` (`LeaveRequest`, `G/MeleyLair.cpp:1187-1213`):
    /// the reader drops from the run and is warped out.
    fn judge_meley_leave(&mut self, reader: Vid) {
        if let Some(id) = self.meley_guild_of(reader) {
            let pid = self
                .characters
                .find_by_vid(reader)
                .map_or(0, world::character::Character::player_id);
            if let Some(run) = self.meley.of_mut(id) {
                run.participants.remove(&pid);
            }
        }
        self.order_meley_out(reader);
    }

    /// `Leave(guild, char, false)` (`G/MeleyLair.cpp:1156-1185`): the player under
    /// `vid` left Meley's map, so it leaves its run's participants. Quiet with no
    /// run, no guild, or no map match, as the chain's returns are.
    pub(super) fn leave_meley(&mut self, vid: Vid, map: i32) {
        if !is_meley_map(map) {
            return;
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let Some(id) = self.guild_of(pid) else {
            return;
        };
        let remove = self.meley.of(id).is_some_and(|run| run.index == map);
        if remove {
            if let Some(run) = self.meley.of_mut(id) {
                run.participants.remove(&pid);
            }
        }
    }

    /// `Leave(guild, ch, true)` (`G/MeleyLair.cpp:1156-1185`): the login flow's arm —
    /// a stranger on Meley's map is warped out. A participant stays.
    pub(super) fn send_out_meley_stranger(&mut self, vid: Vid) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let stays = self
            .guild_of(pid)
            .and_then(|id| self.meley.of(id))
            .is_some_and(|run| run.participants.contains(&pid));
        if !stays {
            self.order_meley_out(vid);
        }
    }

    /// `GuildRemoved` (`G/MeleyLair.cpp:1439-1452`): the guild's run ends — every
    /// member on its map is warped out, and the map and the registration go, as
    /// `EndDungeonWarp` drops them (`G/MeleyLair.cpp:714-727`).
    pub(super) fn end_meley_run(&mut self, id: u32) {
        let Some(run) = self.meley.of(id) else {
            return;
        };
        let index = run.index;
        let vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        for vid in vids {
            self.order_meley_out(vid);
        }
        self.dungeons.destroy(index);
        let channels: Vec<u8> = self
            .maps
            .keys()
            .filter(|(_, map)| *map == index)
            .map(|(channel, _)| *channel)
            .collect();
        for channel in channels {
            self.maps.remove(&(channel, index));
        }
        self.meley.by_guild.remove(&id);
    }

    /// `CMgr::isRegistered` (`G/MeleyLair.cpp:976-990`): the triple's range arm.
    fn meley_registered(&self, id: u32) -> bool {
        let Some((_, _, _, (_, map, _))) = self.meley_guild_row(id) else {
            return false;
        };
        (MAP * 10_000..=MAP * 10_000 + 10_000).contains(&map)
    }

    /// Host Meley's private map on `channel`: `CreatePrivateMap(212)` through the
    /// dungeon manager, which destroys the index again when the base grid is
    /// missing, as the create's `None` arm does.
    fn create_meley_map(&mut self, channel: u8) -> Option<i32> {
        let taken: Vec<i32> = self.maps.keys().map(|&(_, map)| map).collect();
        let index = self
            .dungeons
            .create(MAP, |base| self.recall.hosts(channel, base), &taken)?;
        let Some(grid) = self
            .recall
            .atlas()
            .and_then(|atlas| atlas.region(MAP))
            .and_then(gamedata::server_attr::SectreeGrid::of)
        else {
            self.dungeons.destroy(index);
            return None;
        };
        self.host_map(channel, index, grid);
        Some(index)
    }

    /// The enter warp: the exit save, then the ordered warp to the recall spot on
    /// the run's index (`Enter`, `G/MeleyLair.cpp:1136-1152`).
    fn order_meley_enter(&mut self, reader: Vid, index: i32, x: i32, y: i32) {
        let raw = reader.raw();
        let Some((channel_now, map)) = self.place_of(raw) else {
            return;
        };
        let exit = self.spot_of(raw).map(|spot| crate::save::ExitLocation {
            x: spot.x,
            y: spot.y,
            map_index: map,
        });
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        let ordered = clients
            .members_on_map(channel_now, map)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| {
                clients.order(
                    channel_now,
                    id,
                    crate::client_registry::ClientOrder::Warp {
                        x,
                        y,
                        private_map_index: index,
                        channel: Some(channel_now),
                        exit,
                        sorted_until: self.sort_lock(reader),
                    },
                )
            });
        if !ordered {
            tracing::warn!(vid = raw, "a meley enter found no client to order");
        }
    }

    /// `WarpOut` (`G/MeleyLair.cpp:1250-1260`): the ordered warp to the sub map's
    /// spot, or the empire start with no sub base.
    pub(super) fn order_meley_out(&mut self, reader: Vid) {
        let Some(mover) = self.own_mover(reader) else {
            return;
        };
        let at = self
            .recall
            .atlas()
            .and_then(|atlas| atlas.region(SUBMAP))
            .map(|region| (region.sx + SUB_SPOT.0 * 100, region.sy + SUB_SPOT.1 * 100));
        let Some((x, y)) = at else {
            self.order_village_warp(reader, mover);
            return;
        };
        self.order_quest_warp(reader, x, y, 0, mover);
    }

    /// `SetLadderPoint`'s chat (`G/guild_war.cpp:732-751`): the `[LS;1001]` line to
    /// every online member, in member order. The line carries the judged ladder.
    fn tell_ladder(&self, id: u32, ladder: i32) {
        use protocol::gc_chat::CHAT_TYPE_INFO;
        for vid in self.guilds.online_vids(id) {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"[LS;1001;%d]",
                    &[crate::chat_line::Arg::Int(i64::from(ladder))],
                );
            }
        }
    }

    /// `CGuild::Chat` (`G/guild.cpp:1760-1774`): the formatted guild line to every
    /// online member, in member order.
    fn tell_meley_guild(&self, id: u32, format: &[u8], args: &[crate::chat_line::Arg<'_>]) {
        use protocol::gc_chat::CHAT_TYPE_GUILD;
        for vid in self.guilds.online_vids(id) {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_GUILD, format, args);
            }
        }
    }

    /// `number(from, to)` on the game dice.
    fn meley_number(&mut self, from: i32, to: i32) -> i32 {
        world::character::number(&mut self.dice, from, to)
    }

    /// `GetMonsterCountInMap(index, vnum, force)` (`G/sectree_manager.cpp:1673-1700`):
    /// the standing non-players of `vnum` on every Channel's copy of `index`. With
    /// `force` every non-player counts, else only stones (`FCountSpecifiedMonster`),
    /// which is all a dead NPC sweep needs since only the dead stand out.
    fn meley_count(&self, index: i32, vnum: u32, force: bool) -> usize {
        use gamedata::mob_proto::CHAR_TYPE_STONE;
        self.npcs
            .values()
            .flat_map(|list| list.npcs.iter())
            .filter(|npc| {
                self.npc_of.get(&npc.vid).is_some_and(|(_, map, _)| {
                    *map == index && npc.vnum == vnum && (force || npc.char_type == CHAR_TYPE_STONE)
                })
            })
            .count()
    }

    /// Stand one fight spawn (`CMgrMap::Spawn`, `G/MeleyLair.cpp:407-419`): the vnum
    /// at the map-unit cell on the run's index, facing `(dir - 1) * 45` degrees.
    /// Nothing with no proto, as `SpawnMob` stands none (`G/char_manager.cpp:382-471`).
    fn meley_spawn(
        &mut self,
        channel: u8,
        index: i32,
        vnum: u32,
        cell: (i32, i32),
        dir: i32,
    ) -> Option<u32> {
        let place = (channel, index);
        let (base_x, base_y) = self
            .maps
            .get(&place)
            .map_or((0, 0), super::view::MapIndex::base);
        let at = (
            base_x.saturating_add(cell.0.saturating_mul(100)),
            base_y.saturating_add(cell.1.saturating_mul(100)),
        );
        let (Some(region), Some(cells)) = (
            self.regens.region_of(channel, MAP),
            self.regens.cells_of(index),
        ) else {
            return None;
        };
        let respawner = self.regens.respawner_mut()?;
        let facing = if dir == 0 { -1 } else { (dir - 1) * 45 };
        let stood = match respawner.spawn_directed(
            (&region, &cells),
            vnum,
            at,
            facing,
            false,
            &mut self.dice,
        ) {
            Ok(stood) => stood?,
            Err(error) => {
                tracing::warn!(?place, %error, "a meley spawn outnumbered the VIDs left");
                return None;
            }
        };
        let vid = stood.vid;
        let pulse = self.last_pulse;
        self.stand_runtime(place, vec![stood], pulse);
        Some(vid)
    }

    /// The Channel hosting `index`, or 0 when none does.
    fn meley_channel(&self, index: i32) -> u8 {
        self.maps
            .keys()
            .find(|(_, map)| *map == index)
            .map_or(0, |(channel, _)| *channel)
    }

    /// `Start` (`G/MeleyLair.cpp:558-562`): the NPC, the gate, the boss and the four
    /// statues stand on the run's index. Runs at the register and the late create,
    /// as the `CMgrMap` constructor does.
    fn stand_meley_start(&mut self, id: u32) {
        let Some(index) = self.meley.of(id).map(|run| run.index) else {
            return;
        };
        let channel = self.meley_channel(index);
        let npc = self.meley_spawn(channel, index, NPC_VNUM, NPC_CELL, 8);
        let gate = self.meley_spawn(channel, index, GATE_VNUM, GATE_CELL, 5);
        let boss = self.meley_spawn(channel, index, BOSS_VNUM, BOSS_CELL, 1);
        let mut statues = [None; 4];
        for (slot, cell) in STATUE_CELLS.iter().enumerate() {
            statues[slot] = self.meley_spawn(channel, index, STATUE_VNUM, *cell, 5);
        }
        if let Some(run) = self.meley.of_mut(id) {
            run.main_npc = npc;
            run.gate = gate;
            run.boss = boss;
            run.statues = statues;
        }
    }

    /// `StartDungeon` (`G/MeleyLair.cpp:564-603`): the gate click starts the fight.
    /// A stranger's click is quiet; a second start, or a start with a piece
    /// missing, ends the dungeon failed. Else the two guild lines chat, the gate
    /// dies, step 1 sets and the hour's end arms.
    fn meley_start_dungeon(&mut self, id: u32, reader: Vid, now: u32) -> Vec<GuildQuestStore> {
        let pid = self
            .characters
            .find_by_vid(reader)
            .map_or(0, world::character::Character::player_id);
        let Some(run) = self.meley.of(id) else {
            return Vec::new();
        };
        if !run.participants.contains(&pid) {
            return Vec::new();
        }
        if run.end_due.is_some()
            || run.main_npc.is_none()
            || run.gate.is_none()
            || run.boss.is_none()
            || run.statues.iter().any(Option::is_none)
        {
            return self.meley_end(id, false, true, now);
        }
        let boss = run.boss.expect("checked");
        let name = self
            .npc_by_vid(boss)
            .map_or(Vec::new(), |npc| npc.name.clone());
        self.tell_meley_guild(
            id,
            b"<Guild> The fight against %s just start.",
            &[crate::chat_line::Arg::Text(&name)],
        );
        self.tell_meley_guild(
            id,
            b"<Guild> The shelter's doors are now opened. The time limit is %d hour(s).",
            &[crate::chat_line::Arg::Int(i64::from(
                TIME_LIMIT_DUNGEON / 3_600,
            ))],
        );
        if let Some(manager) = self.quests.as_mut() {
            manager.manager_mut().set_quest_npc(0);
        }
        let gate = self.meley.of(id).and_then(|run| run.gate).expect("checked");
        self.dead_npc(gate, None);
        if let Some(run) = self.meley.of_mut(id) {
            run.gate = None;
        }
        self.meley_set_step(id, 1, now);
        let due = self.last_pulse.saturating_add(super::mob_ai::pulses(
            i32::try_from(TIME_LIMIT_DUNGEON).unwrap_or(0),
        ));
        if let Some(run) = self.meley.of_mut(id) {
            run.end_due = Some((false, due));
        }
        Vec::new()
    }

    /// `SetDungeonStep` (`G/MeleyLair.cpp:421-516`): the step sets, the spawn tick
    /// cancels, and the step's timer arms — step 1 the common tick, steps 2 and 3
    /// the effect ticks, step 4 the statue kill tick with the armada and the
    /// notice. A step past 2 or 3 with a piece missing ends the dungeon failed
    /// first, but the step still sets, as legacy falls through.
    fn meley_set_step(&mut self, id: u32, step: u8, now: u32) {
        if self.meley.of(id).is_some_and(|run| run.step == step) {
            return;
        }
        if let Some(run) = self.meley.of_mut(id) {
            run.step = step;
            run.spawn_due = None;
        }
        if step == 1 {
            if let Some(run) = self.meley.of_mut(id) {
                run.started_at = now;
                run.stones_killed = 0;
            }
            let due = self.last_pulse.saturating_add(super::mob_ai::pulses(1));
            if let Some(run) = self.meley.of_mut(id) {
                run.spawn_due = Some((1, false, due));
            }
            return;
        }
        let whole = self
            .meley
            .of(id)
            .is_some_and(|run| run.boss.is_some() && run.statues.iter().all(Option::is_some));
        if !whole {
            let _stores = self.meley_end(id, false, true, now);
        }
        if step == 2 || step == 3 {
            if let Some(run) = self.meley.of_mut(id) {
                run.effect_due = None;
            }
            let due = self.last_pulse.saturating_add(super::mob_ai::pulses(5));
            let effect = if step == 2 { 1 } else { 2 };
            if let Some(run) = self.meley.of_mut(id) {
                run.effect_due = Some((effect, 1, due));
            }
            return;
        }
        if step == 4 {
            if let Some(run) = self.meley.of_mut(id) {
                run.effect_due = None;
                run.statues_due = None;
                run.sealers.clear();
            }
            let statues = self.meley_statues(id);
            for statue in statues.into_iter().flatten() {
                self.meley_armada(statue);
            }
            self.meley_notice(
                id,
                b"You have %d second(s) to destroy the statues, hurry up!",
                &[crate::chat_line::Arg::Int(i64::from(
                    TIME_LIMIT_TO_KILL_STATUE,
                ))],
            );
            let due = self.last_pulse.saturating_add(super::mob_ai::pulses(3));
            if let Some(run) = self.meley.of_mut(id) {
                run.statues_due = Some((false, due));
            }
        }
    }

    /// `SetArmada` (`G/char.h:2103`): the statue under `vid` cannot die.
    fn meley_armada(&mut self, vid: u32) {
        self.meley_cannot_dead.insert(vid);
    }

    /// `FNotice` (`G/MeleyLair.cpp:46-59`): the line to every PC on the run's map.
    fn meley_notice(&self, id: u32, format: &[u8], args: &[crate::chat_line::Arg<'_>]) {
        use protocol::gc_chat::CHAT_TYPE_NOTICE;
        let Some(index) = self.meley.of(id).map(|run| run.index) else {
            return;
        };
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_NOTICE, format, args);
            }
        }
    }

    /// `EndDungeon` (`G/MeleyLair.cpp:605-713`): the win, failed-start and timeout
    /// lines chat, the ladder returns, the triple clears and the end or warp
    /// timer arms. A win stamps the cooldown and warps in 60 s; a failed start
    /// warps in 10 s with no cooldown; a timeout stamps the cooldown and warps
    /// in 10 s. The `MeleyLog` row is a log write, which the Rewrite drops with
    /// the other dungeon logs.
    fn meley_end(
        &mut self,
        id: u32,
        success: bool,
        give_back: bool,
        now: u32,
    ) -> Vec<GuildQuestStore> {
        let mut stores = Vec::new();
        let (warp_secs, back) = self.meley_end_lines(id, success, give_back, now);
        let hosted = self.meley.of(id).is_some();
        if hosted {
            if warp_secs == 60 {
                self.tell_meley_guild(
                    id,
                    b"<Guild> For your safety, you will be teleported out of the refuge in %d second(s).",
                    &[crate::chat_line::Arg::Int(i64::from(warp_secs))],
                );
            } else {
                self.tell_meley_guild(
                    id,
                    b"<Guild> All the partecipants will be teleported out in %d seconds.",
                    &[crate::chat_line::Arg::Int(i64::from(warp_secs))],
                );
            }
        }
        if back != 0 {
            let ladder = self
                .meley_guild_row(id)
                .map_or(0, |(_, _, ladder, _)| ladder)
                .saturating_add(back)
                .max(0);
            // The failed start's triple keeps its index until the warp drops the
            // run; the win and timeout already cleared it above.
            let triple = if give_back && !success {
                self.meley_guild_row(id)
                    .map_or((0, 0, 0), |(_, _, _, triple)| triple)
            } else {
                (0, 0, now.saturating_add(COOLDOWN_DUNGEON).saturating_add(5))
            };
            self.set_meley_guild_row(id, ladder, triple);
            self.tell_ladder(id, ladder);
            stores.push(GuildQuestStore::Ladder {
                guild_id: id,
                ladder,
            });
        }
        if !give_back || success {
            let cooldown = now.saturating_add(COOLDOWN_DUNGEON).saturating_add(5);
            let ladder = self
                .meley_guild_row(id)
                .map_or(0, |(_, _, ladder, _)| ladder);
            self.set_meley_guild_row(id, ladder, (0, 0, cooldown));
        }
        let due = self
            .last_pulse
            .saturating_add(super::mob_ai::pulses(warp_secs));
        if let Some(run) = self.meley.of_mut(id) {
            run.end_due = Some((true, due));
        }
        stores
    }

    /// `EndDungeon`'s lines (`G/MeleyLair.cpp:605-660`): the win, failed-start
    /// or timeout lines chat and stamp the cooldown, answering the warp seconds
    /// and the ladder return. Split out so `meley_end` stays under the line lint.
    fn meley_end_lines(&mut self, id: u32, success: bool, give_back: bool, now: u32) -> (i32, i32) {
        if !give_back {
            let name = self
                .meley
                .of(id)
                .and_then(|run| run.boss)
                .and_then(|boss| self.npc_by_vid(boss))
                .map_or(Vec::new(), |npc| npc.name.clone());
            self.tell_meley_guild(
                id,
                b"<Guild> The time limit to fight %s has ended.",
                &[crate::chat_line::Arg::Text(&name)],
            );
            let cooldown = now.saturating_add(COOLDOWN_DUNGEON).saturating_add(5);
            let ladder = self
                .meley_guild_row(id)
                .map_or(0, |(_, _, ladder, _)| ladder);
            self.set_meley_guild_row(id, ladder, (0, 0, cooldown));
            return (10, LADDER_RETURN);
        }
        if !success {
            self.tell_meley_guild(
                id,
                b"<Guild> The dungeon can't be started, please contact our staff.",
                &[],
            );
            self.tell_meley_guild(
                id,
                b"<Guild> %d ladder points will be returned.",
                &[crate::chat_line::Arg::Int(i64::from(LADDER_COST))],
            );
            return (10, LADDER_COST);
        }
        let (count, started) = self
            .meley
            .of(id)
            .map_or((0, now), |run| (run.participants.len(), run.started_at));
        let need = now.saturating_sub(started);
        let mut time = Vec::new();
        let mins = (need / 60) % 60;
        let secs = need % 60;
        if mins > 0 {
            time.extend_from_slice(mins.to_string().as_bytes());
            time.extend_from_slice(b" minute(s)");
        }
        if secs > 0 {
            if mins > 0 {
                time.extend_from_slice(b", ");
            }
            time.extend_from_slice(secs.to_string().as_bytes());
            time.extend_from_slice(b" second(s)");
        }
        let _ = count;
        self.tell_meley_guild(id, b"<Guild> Compliments, you defeat Meley.", &[]);
        self.tell_meley_guild(
            id,
            b"<Guild> Necessary time: %s.",
            &[crate::chat_line::Arg::Text(&time)],
        );
        let cooldown = now.saturating_add(COOLDOWN_DUNGEON).saturating_add(5);
        let ladder = self
            .meley_guild_row(id)
            .map_or(0, |(_, _, ladder, _)| ladder);
        self.set_meley_guild_row(id, ladder, (0, 0, cooldown));
        (60, LADDER_COST)
    }

    /// `EndDungeonWarp` (`G/MeleyLair.cpp:714-727`): every PC on the run's map
    /// warps out, and the map and the registration go.
    fn meley_end_warp(&mut self, id: u32) {
        let Some(index) = self.meley.of(id).map(|run| run.index) else {
            return;
        };
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            self.order_meley_out(vid);
        }
        self.dungeons.destroy(index);
        let channels: Vec<u8> = self
            .maps
            .keys()
            .filter(|(_, map)| *map == index)
            .map(|(channel, _)| *channel)
            .collect();
        for channel in channels {
            self.maps.remove(&(channel, index));
        }
        if let Some(statues) = self.meley.of(id).map(|run| run.statues) {
            for statue in statues.into_iter().flatten() {
                self.meley_cannot_dead.remove(&statue);
            }
        }
        self.meley.by_guild.remove(&id);
    }

    /// The Meley ticks (`run_ship_ticks`'s neighbour): every run's due timers fire
    /// in guild order — the end, spawn, effect and statue timers — as the legacy
    /// events fire each second (`G/MeleyLair.cpp:84-334`). Quiet with no run.
    pub(super) fn run_meley_ticks(&mut self, pulse: u64) {
        if self.meley.is_empty() {
            return;
        }
        for id in self.meley.guilds() {
            let now = Self::global_time();
            self.run_meley_end(id, pulse, now);
            self.run_meley_spawn(id, pulse, now);
            self.run_meley_effect(id, pulse);
            self.run_meley_statues(id, pulse, now);
        }
    }

    /// `r_meleylimit_event` (`G/MeleyLair.cpp:115-135`): the hour ends the dungeon
    /// timed out, the warp drops the run.
    fn run_meley_end(&mut self, id: u32, pulse: u64, now: u32) {
        let Some((warp, due)) = self.meley.of(id).and_then(|run| run.end_due) else {
            return;
        };
        if pulse < due {
            return;
        }
        if let Some(run) = self.meley.of_mut(id) {
            run.end_due = None;
        }
        if warp {
            self.meley_end_warp(id);
        } else {
            let _stores = self.meley_end(id, false, false, now);
        }
    }

    /// `r_meleyspawn_event` (`G/MeleyLair.cpp:141-232`): the common row refills to
    /// 12, steps 2 and 3 stand the four stones past the stone clock (re-flagging
    /// the statues) and step 3 stands the bosses once, at the first run.
    fn run_meley_spawn(&mut self, id: u32, pulse: u64, now: u32) {
        let Some((step, first, due)) = self.meley.of(id).and_then(|run| run.spawn_due) else {
            return;
        };
        if pulse < due {
            return;
        }
        let Some(index) = self.meley.of(id).map(|run| run.index) else {
            return;
        };
        let channel = self.meley_channel(index);
        let slot = usize::from(step.saturating_sub(1)).min(2);
        let vnum = COMMON_VNUMS[slot];
        let have = self.meley_count(index, vnum, true);
        for at in have..COMMON_COUNT {
            let cell = MONSTER_CELLS[at % MONSTER_CELLS.len()];
            let dy = self.meley_number(3, 10);
            self.meley_spawn(channel, index, vnum, (cell.0, cell.1.saturating_add(dy)), 0);
        }
        if step == 2 || step == 3 {
            let stones = self.meley_count(index, STONE_VNUM, false);
            if stones == 0 {
                let past = self.meley.of(id).is_some_and(|run| now >= run.stones_at);
                if past {
                    for cell in STONE_CELLS {
                        let dy = self.meley_number(3, 10);
                        self.meley_spawn(
                            channel,
                            index,
                            STONE_VNUM,
                            (cell.0, cell.1.saturating_add(dy)),
                            0,
                        );
                    }
                    self.meley_reflag_statues(id, step);
                }
            }
            if step == 3 && first {
                for cell in STONE_CELLS {
                    let dy = self.meley_number(3, 10);
                    let at = (cell.0, cell.1.saturating_add(dy));
                    self.meley_spawn(channel, index, BOSS3_VNUM, at, 0);
                    self.meley_spawn(channel, index, SUBBOSS3_VNUM, at, 0);
                    self.meley_spawn(channel, index, SUBBOSS3_VNUM, at, 0);
                }
                if let Some(run) = self.meley.of_mut(id) {
                    run.spawn_due = Some((step, false, pulse));
                }
            }
        }
        let reload = COMMON_RELOADS[slot];
        let due = pulse.saturating_add(super::mob_ai::pulses(i32::try_from(reload).unwrap_or(0)));
        let first = self
            .meley
            .of(id)
            .and_then(|run| run.spawn_due)
            .map_or(first, |(_, first, _)| first);
        if let Some(run) = self.meley.of_mut(id) {
            run.spawn_due = Some((step, first, due));
        }
    }

    /// The spawn tick's statue re-flag: step 3 flags the statues back to
    /// `AFF_STATUE1` unless one already carries `AFF_STATUE3`, and zeroes the
    /// stone kills; step 2 unless one already carries `AFF_STATUE2`.
    fn meley_reflag_statues(&mut self, id: u32, step: u8) {
        use crate::loading_phase::{AFFECT_STATUE, AFF_STATUE1, AFF_STATUE2, AFF_STATUE3};
        let statues = self.meley_statues(id);
        let keep = if step == 3 { AFF_STATUE3 } else { AFF_STATUE2 };
        for statue in statues.into_iter().flatten() {
            if !self.meley_npc_flag(statue, keep) {
                self.meley_flag_statue(statue, AFFECT_STATUE, AFF_STATUE1);
            }
        }
        if step == 3 {
            if let Some(run) = self.meley.of_mut(id) {
                run.stones_killed = 0;
            }
        }
    }

    /// Whether the non-player under `vid` carries affect flag `flag`
    /// (`IsAffectFlag`, `G/affect_flag.h:27-33`), numbered from 1.
    fn meley_npc_flag(&self, vid: u32, flag: u32) -> bool {
        let Some(bit) = flag.checked_sub(1) else {
            return false;
        };
        if flag >= crate::loading_phase::AFF_BITS_MAX {
            return false;
        }
        let words = self.npc_affect_words(vid);
        usize::try_from(bit / 32)
            .ok()
            .and_then(|word| words.get(word))
            .is_some_and(|word| word & (1 << (bit % 32)) != 0)
    }

    /// `AddAffect(AFFECT_STATUE, POINT_NONE, 0, flag, 3600, 0, true)` on the
    /// non-player under `vid`.
    fn meley_flag_statue(&mut self, vid: u32, kind: u32, flag: u32) {
        self.add_npc_affect(vid, super::affect::Affect::buff(kind, 0, 0, flag, 3_600, 0));
    }

    /// `r_meleyeffect_event` (`G/MeleyLair.cpp:238-334`): the first pass turns the
    /// statues to the boss and casts their skill; the second moves the step on
    /// (1's effect to step 2, 2's to step 3).
    fn run_meley_effect(&mut self, id: u32, pulse: u64) {
        let Some((step, pass, due)) = self.meley.of(id).and_then(|run| run.effect_due) else {
            return;
        };
        if pulse < due {
            return;
        }
        let Some((boss, statues)) = self
            .meley
            .of(id)
            .and_then(|run| run.boss.map(|boss| (boss, run.statues)))
        else {
            return;
        };
        if pass == 1 {
            let Some(boss_at) = self.npc_spot(boss) else {
                return;
            };
            for statue in statues.into_iter().flatten() {
                if let Some(at) = self.npc_spot(statue) {
                    if let Some(mind) = self.minds.get_mut(statue) {
                        mind.rotation = super::motion::rotation_to_xy(
                            boss_at.0.saturating_sub(at.0),
                            boss_at.1.saturating_sub(at.1),
                        );
                    }
                    let record = protocol::gc_actors::GcCharacterMove::new(
                        crate::movement::FUNC_MOB_SKILL,
                        0,
                        super::mob_skill::rotation_byte(
                            self.minds.get(statue).map_or(0.0, |mind| mind.rotation),
                        ),
                        statue,
                        at.0,
                        at.1,
                        self.clock.now(),
                        0,
                    )
                    .encode();
                    let me = super::view::EntityKey::Npc(statue);
                    self.packet_around(me, &record, Some(me));
                }
            }
            let due = pulse.saturating_add(super::mob_ai::pulses(5));
            if let Some(run) = self.meley.of_mut(id) {
                run.effect_due = Some((step, 2, due));
            }
            return;
        }
        if let Some(run) = self.meley.of_mut(id) {
            run.effect_due = None;
        }
        let now = Self::global_time();
        self.meley_start_step(id, step.saturating_add(1), now);
    }

    /// `StartDungeonStep` (`G/MeleyLair.cpp:517-557`): the spawn and effect ticks
    /// cancel, and step 2 or 3 arms its common tick — step 3 with its first run
    /// (the one-shot bosses) and zeroed kill counts.
    fn meley_start_step(&mut self, id: u32, step: u8, _now: u32) {
        if let Some(run) = self.meley.of_mut(id) {
            run.spawn_due = None;
            run.effect_due = None;
        }
        if step != 2 && step != 3 {
            return;
        }
        if let Some(run) = self.meley.of_mut(id) {
            run.stones_at = 0;
            if step == 3 {
                run.stones_killed = 0;
                run.bosses_killed = 0;
            }
        }
        // The step the spawn tick reads follows the arming, as `SetDungeonStep`
        // would set it; the effect's second pass stands in for it.
        if let Some(run) = self.meley.of_mut(id) {
            run.step = step;
        }
        let due = self.last_pulse.saturating_add(super::mob_ai::pulses(1));
        if let Some(run) = self.meley.of_mut(id) {
            run.spawn_due = Some((step, step == 3, due));
        }
    }

    /// `r_meleystatues_event` (`G/MeleyLair.cpp:84-108`): the first run re-arms for
    /// the kill window, the second judges the statues.
    fn run_meley_statues(&mut self, id: u32, pulse: u64, now: u32) {
        let Some((first, due)) = self.meley.of(id).and_then(|run| run.statues_due) else {
            return;
        };
        if pulse < due {
            return;
        }
        if !first {
            let due = pulse.saturating_add(super::mob_ai::pulses(
                i32::try_from(TIME_LIMIT_TO_KILL_STATUE).unwrap_or(0),
            ));
            if let Some(run) = self.meley.of_mut(id) {
                run.statues_due = Some((true, due));
            }
            return;
        }
        if let Some(run) = self.meley.of_mut(id) {
            run.statues_due = None;
        }
        self.meley_dungeon_result(id, now);
    }

    /// `DungeonResult` (`G/MeleyLair.cpp:907-936`): without four `AFF_STATUE4`
    /// statues the dungeon times out; else the rewards clear, the chest's clock
    /// stamps 10 s out, the statues and the boss die, the chest stands and the
    /// dungeon ends won.
    fn meley_dungeon_result(&mut self, id: u32, now: u32) {
        use crate::loading_phase::AFF_STATUE4;
        if let Some(run) = self.meley.of_mut(id) {
            run.statues_due = None;
        }
        let statues = self.meley_statues(id);
        let won = statues
            .into_iter()
            .flatten()
            .all(|statue| self.meley_npc_flag(statue, AFF_STATUE4))
            && statues.iter().all(Option::is_some);
        if !won {
            let _stores = self.meley_end(id, false, false, now);
            return;
        }
        if let Some(run) = self.meley.of_mut(id) {
            run.rewarded.clear();
            run.reward_at = now.saturating_add(10);
        }
        for statue in statues.into_iter().flatten() {
            self.meley_cannot_dead.remove(&statue);
            self.dead_npc(statue, None);
        }
        if let Some(boss) = self.meley.of(id).and_then(|run| run.boss) {
            self.dead_npc(boss, None);
        }
        let index = self.meley.of(id).map_or(0, |run| run.index);
        let channel = self.meley_channel(index);
        let chest = self.meley_spawn(channel, index, CHEST_VNUM, CENTRE_SPOT, 1);
        if let Some(run) = self.meley.of_mut(id) {
            run.chest = chest;
        }
        let _stores = self.meley_end(id, true, true, now);
    }

    /// `CMgrMap::Damage` (`G/MeleyLair.cpp:729-783`): true when the hit on the
    /// statue under `vid` lands. Step 0, a non-statue, or a flagged statue
    /// blocks. At or under the step's HP line (75%, 50%, 1%) the statue heals
    /// back to the line, takes the step's flag, and moves the step on when all
    /// four carry it — and the hit blocks.
    pub(super) fn meley_statue_damage(&mut self, id: u32, vid: u32) -> bool {
        use crate::loading_phase::{
            AFFECT_STATUE, AFF_STATUE1, AFF_STATUE2, AFF_STATUE3, AFF_STATUE4,
        };
        let Some(run) = self.meley.of(id) else {
            return false;
        };
        let step = run.step;
        if step == 0 || !run.statues.contains(&Some(vid)) {
            return false;
        }
        if [AFF_STATUE1, AFF_STATUE2, AFF_STATUE3, AFF_STATUE4]
            .into_iter()
            .any(|flag| self.meley_npc_flag(vid, flag))
        {
            return false;
        }
        let Some(npc) = self.npc_by_vid(vid) else {
            return false;
        };
        let line = match step {
            1 => npc.max_hp.saturating_mul(75) / 100,
            2 => npc.max_hp.saturating_mul(50) / 100,
            3 => npc.max_hp.saturating_mul(1) / 100,
            _ => return true,
        };
        if npc.hp > line {
            return true;
        }
        let heal = line.saturating_sub(npc.hp);
        if let Some(npc) = self.npc_mut(vid) {
            npc.hp = npc.hp.saturating_add(heal);
        }
        self.broadcast_npc_target(vid);
        let flag = match step {
            1 => AFF_STATUE1,
            2 => AFF_STATUE2,
            _ => AFF_STATUE3,
        };
        self.meley_flag_statue(vid, AFFECT_STATUE, flag);
        let statues = self.meley_statues(id);
        let next = statues
            .into_iter()
            .flatten()
            .all(|statue| self.meley_npc_flag(statue, flag));
        if next {
            let now = Self::global_time();
            self.meley_set_step(id, step.saturating_add(1), now);
        }
        false
    }

    /// `CMgrMap::OnKill` (`G/MeleyLair.cpp:785-862`): steps 2 and 3 count stone
    /// kills, step 3 boss kills; at the limit the stone clock rolls 2-4 minutes
    /// out and the statues lose their flags (step 3 needs both limits met).
    fn meley_on_kill(&mut self, id: u32, vnum: u32, now: u32) {
        let Some(run) = self.meley.of(id) else {
            return;
        };
        let step = run.step;
        if step == 0 || run.statues.iter().any(Option::is_none) {
            return;
        }
        if (step == 2 || step == 3) && vnum == STONE_VNUM {
            let limit = u32::try_from(STONE_COUNT.saturating_sub(1)).unwrap_or(u32::MAX);
            let count = self.meley.of(id).map_or(0, |run| run.stones_killed);
            if count < limit {
                if let Some(run) = self.meley.of_mut(id) {
                    run.stones_killed = count.saturating_add(1);
                }
                return;
            }
            if step == 2 {
                if let Some(run) = self.meley.of_mut(id) {
                    run.stones_killed = 0;
                }
            } else if let Some(run) = self.meley.of_mut(id) {
                run.stones_killed = count.saturating_add(1);
            }
            let mins = self.meley_number(2, 4);
            let at = now.saturating_add(u32::try_from(mins).unwrap_or(0).saturating_mul(60));
            if let Some(run) = self.meley.of_mut(id) {
                run.stones_at = at;
            }
            let mut strip = true;
            if step == 3 {
                let bosses = self.meley.of(id).map_or(0, |run| run.bosses_killed);
                if bosses >= BOSS3_KILLS {
                    if let Some(run) = self.meley.of_mut(id) {
                        run.stones_killed = 0;
                    }
                } else {
                    strip = false;
                }
            }
            if strip {
                self.meley_strip_statues(id, step);
            }
            return;
        }
        if step == 3 && vnum == BOSS3_VNUM {
            let limit = BOSS3_KILLS.saturating_sub(1);
            let count = self.meley.of(id).map_or(0, |run| run.bosses_killed);
            if count < limit {
                if let Some(run) = self.meley.of_mut(id) {
                    run.bosses_killed = count.saturating_add(1);
                }
                return;
            }
            if let Some(run) = self.meley.of_mut(id) {
                run.bosses_killed = count.saturating_add(1);
            }
            let stones = self.meley.of(id).map_or(0, |run| run.stones_killed);
            if stones >= u32::try_from(STONE_COUNT).unwrap_or(u32::MAX) {
                if let Some(run) = self.meley.of_mut(id) {
                    run.stones_killed = 0;
                }
                self.meley_strip_statues(id, step);
            }
        }
    }

    /// The run's four statue VIDs, or four empty slots with no run.
    fn meley_statues(&self, id: u32) -> [Option<u32>; 4] {
        self.meley.of(id).map_or([None; 4], |run| run.statues)
    }

    /// The kill's statue strip: step 2 loses `AFF_STATUE1`, step 3 `AFF_STATUE1`
    /// and `AFF_STATUE2`.
    fn meley_strip_statues(&mut self, id: u32, step: u8) {
        use crate::loading_phase::{AFFECT_STATUE, AFF_STATUE1, AFF_STATUE2};
        let statues = self.meley_statues(id);
        for statue in statues.into_iter().flatten() {
            let strip = self.meley_npc_flag(statue, AFF_STATUE1)
                || (step == 3 && self.meley_npc_flag(statue, AFF_STATUE2));
            if strip {
                self.remove_npc_affect(statue, AFFECT_STATUE);
            }
        }
    }

    /// `CMgr::OnKillStatue` into `CMgrMap::OnKillStatue`
    /// `ReceiveItem`'s statue arm (`G/char_item.cpp:9443-9448`): a seal breaks
    /// the statue, anything else is quiet. Off Meley's map the statue never
    /// answers. Split out so `give_item` stays under the line lint.
    pub(super) fn meley_gift(
        &mut self,
        giver: Vid,
        target: u32,
        place: crate::game_loop_messages::GroundPlace,
        (item_id, vnum): (u32, u32),
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        if vnum == SEAL_VNUM && is_meley_map(place.map) {
            self.meley_break_statue(giver, target, item_id, actor, moved);
        }
    }

    /// (`G/MeleyLair.cpp:865-906`, `:1305-1330`): the seal `item_id` the player
    /// `giver` holds breaks the statue under `vid` at step 4. The vnum, the
    /// participant row, the four `AFF_STATUE3/4` statues and the one-seal rule
    /// (GMs break any number, `__DESTROY_INFINITE_STATUES_GM__`) gate it; then
    /// the seal is spent, the statue takes `AFF_STATUE4`, and four of those end
    /// the dungeon. Returns whether the seal was spent.
    pub(super) fn meley_break_statue(
        &mut self,
        giver: Vid,
        vid: u32,
        item_id: u32,
        actor: Mover,
        moved: &mut MovedItems,
    ) -> bool {
        use crate::loading_phase::{AFFECT_STATUE, AFF_STATUE3, AFF_STATUE4};
        let pid = self
            .characters
            .find_by_vid(giver)
            .map_or(0, world::character::Character::player_id);
        let Some(id) = self.guild_of(pid) else {
            return false;
        };
        let Some(run) = self.meley.of(id) else {
            return false;
        };
        if run.step != 4 || !run.participants.contains(&pid) {
            return false;
        }
        let statues = run.statues;
        if !statues.contains(&Some(vid)) {
            return false;
        }
        let ready = statues.into_iter().flatten().all(|statue| {
            self.meley_npc_flag(statue, AFF_STATUE3) || self.meley_npc_flag(statue, AFF_STATUE4)
        });
        if !ready || self.meley_npc_flag(vid, AFF_STATUE4) {
            return false;
        }
        let sealed = self
            .meley
            .of(id)
            .is_some_and(|run| run.sealers.contains(&pid));
        let gm = self
            .characters
            .find_by_vid(giver)
            .ok()
            .and_then(|giver| giver.points())
            .map_or(
                crate::command::GM_PLAYER,
                world::character::Points::gm_level,
            );
        if sealed && gm == crate::command::GM_PLAYER {
            return false;
        }
        if !sealed {
            if let Some(run) = self.meley.of_mut(id) {
                run.sealers.push(pid);
            }
        }
        self.spend_gift(giver, item_id, actor, moved);
        self.remove_npc_affect(vid, AFFECT_STATUE);
        self.meley_flag_statue(vid, AFFECT_STATUE, AFF_STATUE4);
        let statues = self.meley_statues(id);
        let done = statues
            .into_iter()
            .flatten()
            .all(|statue| self.meley_npc_flag(statue, AFF_STATUE4));
        if done {
            let now = Self::global_time();
            self.meley_dungeon_result(id, now);
        }
        true
    }

    /// `CMgr::CanGetReward` (`G/MeleyLair.cpp:1357-1379`): the reader's guild runs
    /// step 4, the reader is a participant past the chest's clock who took no
    /// reward yet.
    pub(super) fn meley_can_reward(&self, reader: Vid, now: u32) -> bool {
        let pid = self
            .characters
            .find_by_vid(reader)
            .map_or(0, world::character::Character::player_id);
        let Some(id) = self.guild_of(pid) else {
            return false;
        };
        let Some(run) = self.meley.of(id) else {
            return false;
        };
        run.step == 4
            && run.participants.contains(&pid)
            && run.reward_at <= now
            && !run.rewarded.contains(&pid)
    }

    /// `CMgr::Reward` into `GiveReward` (`G/MeleyLair.cpp:1381-1394`, `:939-951`):
    /// a participant's first take rows the PID and gives chest 1 for argument 1,
    /// chest 2 else — but only past the chest's clock, as the port's `meley_reward`
    /// re-judges the step-4 clock the shadow read before each take. The give's
    /// records ride the caller, as the dialog step's do.
    pub(super) fn meley_reward(
        &mut self,
        reader: Vid,
        arg: u8,
        now: u32,
    ) -> Option<(MovedItems, Vec<GuildQuestStore>)> {
        let pid = self
            .characters
            .find_by_vid(reader)
            .map_or(0, world::character::Character::player_id);
        let id = self.guild_of(pid)?;
        if !self.meley_can_reward(reader, now) {
            return None;
        }
        if let Some(run) = self.meley.of_mut(id) {
            run.rewarded.push(pid);
        }
        let vnum = if arg == 1 {
            REWARD_VNUMS[0]
        } else {
            REWARD_VNUMS[1]
        };
        let actor = self.own_mover(reader)?;
        let (given, done) = self.auto_give(reader, (vnum, 1), 0, true)?;
        if let world::character::AutoGiven::Unplaced(item) = given {
            self.lay_auto_given(reader, item);
        }
        let moved = MovedItems::new(pid, reader.raw(), done, actor, &self.protos, &self.locale);
        Some((moved, Vec::new()))
    }

    /// `OnClick`'s Meley arm (`G/char.cpp:6192-6197`): true when `target` is the
    /// gate on Meley's map, whose click starts the clicker's fight. The dead
    /// gate is gone, as its `Dead` cleared the run's row.
    pub(super) fn meley_gate_click(
        &mut self,
        reader: Vid,
        target: u32,
        place: crate::game_loop_messages::GroundPlace,
    ) -> bool {
        if !is_meley_map(place.map) {
            return false;
        }
        let Some(npc) = self.npc_by_vid(target) else {
            return false;
        };
        if npc.vnum != GATE_VNUM || npc.dead {
            return false;
        }
        let pid = self
            .characters
            .find_by_vid(reader)
            .map_or(0, world::character::Character::player_id);
        let Some(id) = self.guild_of(pid) else {
            return false;
        };
        if self.meley.of(id).is_none() {
            return false;
        }
        let now = Self::global_time();
        let _stores = self.meley_start_dungeon(id, reader, now);
        true
    }

    /// `Damage`'s Meley arms (`G/char_battle.cpp:1746-1759`): a statue's hit runs
    /// the step's gate, the boss's blocks. Returns false when the hit lands, true
    /// when `DAMAGE_BLOCK` shows and the hit ends. Nothing with no guild run.
    pub(super) fn meley_blocks_hit(&mut self, attacker: Vid, victim: u32) -> bool {
        let Some(npc) = self.npc_by_vid(victim) else {
            return false;
        };
        if npc.vnum != STATUE_VNUM && npc.vnum != BOSS_VNUM {
            return false;
        }
        let map = self.place_of(attacker.raw()).map_or(0, |(_, map)| map);
        if !is_meley_map(map) {
            return false;
        }
        let pid = self
            .characters
            .find_by_vid(attacker)
            .map_or(0, world::character::Character::player_id);
        let Some(id) = self.guild_of(pid) else {
            // No guild: `CMgr::Damage` finds no map and refuses (`:1280-1289`).
            return true;
        };
        if self.meley.of(id).is_none() {
            return true;
        }
        if npc.vnum == BOSS_VNUM {
            return true;
        }
        !self.meley_statue_damage(id, victim)
    }

    /// `Dead`'s Meley arm (`G/char_battle.cpp:1678-1688`): a stone's or a step-3
    /// boss's death counts its kill; any other monster's death past step 3 may
    /// drop a seal at its feet, owned by the killer for a minute
    /// (`OnKillCommon`, `G/MeleyLair.cpp:1332-1355`).
    pub(super) fn meley_on_npc_kill(&mut self, vid: u32, killer: Option<Vid>) {
        use gamedata::mob_proto::{CHAR_TYPE_MONSTER, CHAR_TYPE_STONE};
        let Some(npc) = self.npc_by_vid(vid) else {
            return;
        };
        let (char_type, vnum, at) = (npc.char_type, npc.vnum, (npc.x, npc.y));
        if char_type != CHAR_TYPE_STONE && char_type != CHAR_TYPE_MONSTER {
            return;
        }
        let Some(killer) = killer else {
            return;
        };
        let map = self.place_of(killer.raw()).map_or(0, |(_, map)| map);
        if !is_meley_map(map) {
            return;
        }
        let pid = self
            .characters
            .find_by_vid(killer)
            .map_or(0, world::character::Character::player_id);
        let Some(id) = self.guild_of(pid) else {
            return;
        };
        let Some(run) = self.meley.of(id) else {
            return;
        };
        if !run.participants.contains(&pid) {
            return;
        }
        let now = Self::global_time();
        if (vnum == STONE_VNUM || vnum == BOSS3_VNUM) && run.step != 0 {
            self.meley_on_kill(id, vnum, now);
            return;
        }
        if run.step < 3 {
            return;
        }
        if self.meley_number(1, 100) <= 70 {
            return;
        }
        self.meley_lay_seal(killer, at);
    }

    /// Lay a seal at the dead monster's feet on the killer's map, owned by the
    /// killer for 60 s (`OnKillCommon`, `G/MeleyLair.cpp:1332-1355`).
    fn meley_lay_seal(&mut self, killer: Vid, (x, y): (i32, i32)) {
        use world::character::{DropAsk, DropMade};
        let Some((channel, map)) = self.place_of(killer.raw()) else {
            return;
        };
        let Some(spot) = self.spot_of(killer.raw()) else {
            return;
        };
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            });
        let attrs = std::sync::Arc::clone(&self.item_attrs);
        let masks = std::sync::Arc::clone(&self.item_masks);
        let protos = std::sync::Arc::clone(&self.protos);
        let made = DropMade {
            attrs: &attrs,
            masks: masks.pairs(),
            now_secs,
            count_limit: self.item_count_limit,
        };
        let Some(ids) = self.item_ids.as_mut() else {
            tracing::warn!("a meley seal fell; no item ids are installed");
            return;
        };
        let dice = &mut self.dice;
        let gauss = &mut self.gauss;
        let ask = DropAsk {
            count: 1,
            rare_pct: 0,
        };
        let Ok(item) =
            world::character::make_drop_item(dice, gauss, ids, &protos, &made, SEAL_VNUM, ask)
        else {
            return;
        };
        let Some(ground) = self.lay_drop_hit(channel, map, (x, y, spot.z), item) else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(killer) else {
            return;
        };
        let pid = character.player_id();
        let Some(body) = self.bodies.get(&killer) else {
            return;
        };
        let name = crate::loading_phase::name_field(&body.card.name);
        self.set_ground_ownership(ground, Some((pid, name)), 60);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::fixtures::{a_world, enter, TestClock};
    use super::*;
    use crate::client_registry::{ChannelClients, ClientEntry, ClientOrder, Lease};
    use gamedata::mob_proto::{CHAR_TYPE_MONSTER, CHAR_TYPE_STONE};
    use protocol::gc_guild::GUILD_SKILL_COUNT;
    use world::npc::{MapNpcs, Npc, NpcVids, Respawner, FIRST_NPC_VID};

    const MASTER_VID: u32 = 100;
    const GUILD_ID: u32 = 7;
    const NOW: u32 = 10_000_000;

    /// A guilded master with a ladder of 600 and level 3, a client lease on its
    /// map, and an atlas naming maps 212 and 62 — with the fight's protos and
    /// open cells on 212, so the register's start pieces stand.
    fn a_meley_world() -> (
        GameState,
        tokio::sync::mpsc::UnboundedReceiver<ClientOrder>,
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        Lease,
    ) {
        let clock = TestClock::default();
        clock.set(1_000);
        let regions = vec![
            gamedata::map_atlas::MapRegion {
                index: MAP,
                name: b"meley".to_vec(),
                sx: 100_000,
                sy: 200_000,
                ex: 200_000,
                ey: 300_000,
                spawn: (113_000, 213_000),
                empire_spawns: None,
            },
            gamedata::map_atlas::MapRegion {
                index: SUBMAP,
                name: b"flame".to_vec(),
                sx: 300_000,
                sy: 400_000,
                ex: 400_000,
                ey: 500_000,
                spawn: (375_000, 475_000),
                empire_spawns: None,
            },
        ];
        let atlas = Arc::new(gamedata::map_atlas::MapAtlas::from_regions(regions));
        let clients = Arc::new(ChannelClients::new());
        let mut maps = std::collections::HashMap::new();
        maps.insert(1u8, vec![41u32, MAP as u32, SUBMAP as u32]);
        let mut world = a_world(&clock)
            .with_clients(Arc::clone(&clients))
            .with_recall(atlas, &maps, &[]);
        install_meley_spawns(&mut world);
        world.host_map(
            1,
            SUBMAP,
            gamedata::server_attr::SectreeGrid {
                x: 0,
                y: 0,
                columns: 10,
                rows: 10,
            },
        );
        let inbox = enter(&mut world, MASTER_VID, (3200, 3200), 0);
        let grades: Vec<db::guilds::StoredGrade> = (1..=15)
            .map(|grade| db::guilds::StoredGrade {
                guild_id: GUILD_ID,
                grade,
                name: if grade == 1 {
                    "Leader".to_owned()
                } else {
                    "...".to_owned()
                },
                auth: if grade == 1 { 15 } else { 0 },
            })
            .collect();
        world.install_guilds(
            &db::guilds::StoredGuilds {
                guilds: vec![db::guilds::StoredGuild {
                    id: GUILD_ID,
                    name: "Knights".to_owned(),
                    master_pid: MASTER_VID,
                    sp: 400,
                    level: 3,
                    exp: 0,
                    skill_point: 0,
                    skills: vec![0; GUILD_SKILL_COUNT],
                    ladder_point: 600,
                    win: 0,
                    draw: 0,
                    loss: 0,
                    gold: 0,
                }],
                grades,
                members: vec![db::guilds::StoredMember {
                    pid: MASTER_VID,
                    guild_id: GUILD_ID,
                    grade: 1,
                    is_general: false,
                    offer: 0,
                    level: 10,
                    job: 0,
                    name: format!("P{MASTER_VID}"),
                }],
            },
            &[],
            &[],
        );
        world.link_guild(Vid::new(MASTER_VID));
        let mut lease = clients.join(ClientEntry {
            channel: 1,
            map: 41,
            name: format!("P{MASTER_VID}"),
            vid: MASTER_VID,
            empire: 1,
            language: 0,
        });
        let orders = lease.take_orders().expect("the queue is home");
        (world, orders, inbox, lease)
    }

    /// What `inbox` holds since the last look.
    fn heard(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The register's gates in legacy order: a guest's quiet (0, 0), the low
    /// level's 3, the poor ladder's 4, the cooldown's 7 with its seconds, and the
    /// pass, which stands the run, debits the ladder and names the triple.
    #[test]
    fn the_register_runs_its_gates_then_stands_the_run() {
        let (mut world, _orders, _inbox, _lease) = a_meley_world();
        // The guest holds no guild, so its register is quiet.
        let guest = enter(&mut world, 101, (3300, 3200), 0);
        let _ = guest;
        assert!(world.judge_meley_register(Vid::new(101), NOW).is_none());
        assert!(world.meley.of(GUILD_ID).is_none());
        // A level-2 guild reads 3 in the shadow; the world refuses quietly.
        world.guilds.set_meley_row(GUILD_ID, 600, (0, 0, 0));
        world.guilds.set_meley_level(GUILD_ID, 2);
        assert!(world
            .judge_meley_register(Vid::new(MASTER_VID), NOW)
            .is_none());
        // A ladder of 599 refuses.
        world.guilds.set_meley_level(GUILD_ID, 3);
        world.guilds.set_meley_row(GUILD_ID, 599, (0, 0, 0));
        assert!(world
            .judge_meley_register(Vid::new(MASTER_VID), NOW)
            .is_none());
        // A live cooldown refuses.
        world.guilds.set_meley_row(GUILD_ID, 600, (0, 0, NOW + 60));
        assert!(world
            .judge_meley_register(Vid::new(MASTER_VID), NOW)
            .is_none());
        // The pass: the run stands, the ladder is 0, the triple names the index.
        world.guilds.set_meley_row(GUILD_ID, 600, (0, 0, 0));
        let store = world
            .judge_meley_register(Vid::new(MASTER_VID), NOW)
            .expect("the pass stores");
        assert_eq!(
            store,
            GuildQuestStore::Ladder {
                guild_id: GUILD_ID,
                ladder: 0
            }
        );
        let run = world.meley.of(GUILD_ID).expect("the run stands");
        assert_eq!(run.index, 2_120_000);
        assert!(run.participants.is_empty());
        let (_, _, ladder, triple) = world
            .guilds
            .shadow_meley_guild(GUILD_ID)
            .expect("installed");
        assert_eq!(ladder, 0);
        assert_eq!(triple, (1, 2_120_000, 0));
        // A second register refuses: the guild is registered.
        assert!(world
            .judge_meley_register(Vid::new(MASTER_VID), NOW)
            .is_none());
    }

    /// The enter's gates: a stranger's PK refusal, then the pass, which rows the
    /// participant and orders the warp to the recall spot on the run's index.
    /// The check keeps a participant and outs a stranger; the leave drops the
    /// row and outs the reader.
    #[test]
    fn the_enter_warps_a_guild_mode_member_in_and_the_check_and_leave_send_out() {
        let (mut world, mut orders, mut inbox, _lease) = a_meley_world();
        let _ = heard(&mut inbox);
        let store = world
            .judge_meley_register(Vid::new(MASTER_VID), NOW)
            .expect("the pass stores");
        assert_eq!(
            store,
            GuildQuestStore::Ladder {
                guild_id: GUILD_ID,
                ladder: 0
            }
        );
        // The `[LS;1001]` line rides the master's burst.
        let burst = heard(&mut inbox);
        assert!(
            burst.iter().any(|line| line
                .windows(b"[LS;1001;0]".len())
                .any(|window| window == b"[LS;1001;0]")),
            "the ladder line chats: {burst:02x?}"
        );
        // The fixture's PK mode is PROTECT, so the enter refuses quietly.
        world.judge_meley_enter(Vid::new(MASTER_VID));
        assert!(world
            .meley
            .of(GUILD_ID)
            .expect("the run stands")
            .participants
            .is_empty());
        assert!(orders.try_recv().is_err(), "no warp while refused");
        // In guild mode the member enters: the row lands and the warp is ordered
        // to the recall spot on the run's index.
        world
            .bodies
            .get_mut(&Vid::new(MASTER_VID))
            .expect("the body stands")
            .card
            .pk_mode = crate::loading_phase::PK_MODE_GUILD;
        world.judge_meley_enter(Vid::new(MASTER_VID));
        assert!(world
            .meley
            .of(GUILD_ID)
            .expect("the run stands")
            .participants
            .contains(&MASTER_VID));
        let warp = orders.try_recv().expect("the warp is ordered");
        assert!(
            matches!(
                warp,
                ClientOrder::Warp {
                    x: 113_000,
                    y: 213_000,
                    private_map_index: 2_120_000,
                    ..
                }
            ),
            "the recall spot on the run: {warp:?}"
        );
        // The check keeps a participant: no order.
        world.judge_meley_check(Vid::new(MASTER_VID));
        assert!(orders.try_recv().is_err(), "a participant stays");
        // The leave drops the row and outs the reader to the sub spot.
        world.judge_meley_leave(Vid::new(MASTER_VID));
        assert!(world
            .meley
            .of(GUILD_ID)
            .expect("the run stands")
            .participants
            .is_empty());
        let out = orders.try_recv().expect("the leave warps out");
        assert!(
            matches!(
                out,
                ClientOrder::Warp {
                    x: 308_700,
                    y: 485_300,
                    ..
                }
            ),
            "the sub spot: {out:?}"
        );
    }

    /// A standing run with the master a participant: the register passed, the
    /// enter warped it in guild mode, and the gate click started the fight, so
    /// step 1 runs and the start pieces stand.
    fn a_fighting_world() -> (
        GameState,
        tokio::sync::mpsc::UnboundedReceiver<ClientOrder>,
        tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        Lease,
    ) {
        let (mut world, orders, inbox, lease) = a_meley_world();
        world
            .judge_meley_register(Vid::new(MASTER_VID), NOW)
            .expect("the pass stores");
        world
            .bodies
            .get_mut(&Vid::new(MASTER_VID))
            .expect("the body stands")
            .card
            .pk_mode = crate::loading_phase::PK_MODE_GUILD;
        world.judge_meley_enter(Vid::new(MASTER_VID));
        let gate = world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.gate)
            .expect("the gate stands");
        world.meley_gate_click(
            Vid::new(MASTER_VID),
            gate,
            crate::game_loop_messages::GroundPlace {
                channel: 1,
                map: MAP,
                x: 0,
                y: 0,
            },
        );
        (world, orders, inbox, lease)
    }

    /// The spawn tick: step 1 refills the common row to 12 and re-arms 5 s on.
    #[test]
    fn the_spawn_tick_refills_the_common_row() {
        let (mut world, _orders, _inbox, _lease) = a_fighting_world();
        let (_, _, due) = world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.spawn_due)
            .expect("step 1 arms");
        world.run_meley_spawn(GUILD_ID, due, NOW);
        let index = world
            .meley
            .of(GUILD_ID)
            .map(|run| run.index)
            .expect("the run stands");
        assert_eq!(
            world.meley_count(index, COMMON_VNUMS[0], true),
            COMMON_COUNT,
            "the row refills"
        );
        let (_, _, rearm) = world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.spawn_due)
            .expect("re-armed");
        assert_eq!(rearm, due.saturating_add(super::super::mob_ai::pulses(5)));
    }

    /// The effect tick: the first pass re-arms the second 5 s on, the second
    /// moves the step on to 2.
    #[test]
    fn the_effect_tick_arms_then_moves_the_step_on() {
        let (mut world, _orders, _inbox, _lease) = a_fighting_world();
        world.meley_set_step(GUILD_ID, 2, NOW);
        let (_, _, due) = world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.effect_due)
            .expect("step 2 arms");
        world.run_meley_effect(GUILD_ID, due);
        let (_, pass, rearm) = world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.effect_due)
            .expect("the second pass arms");
        assert_eq!(pass, 2);
        assert_eq!(rearm, due.saturating_add(super::super::mob_ai::pulses(5)));
        world.run_meley_effect(GUILD_ID, rearm);
        assert!(world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.effect_due)
            .is_none());
        // `meley_start_step` refuses a no-op step: the effect's second pass is
        // what moves 2 on, so the spawn tick for it stands armed.
        assert!(world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.spawn_due)
            .is_some());
    }

    /// The statue tick: the first run re-arms 10 s on, the second judges — with
    /// no `AFF_STATUE4` statues the dungeon times out and the warp arms.
    #[test]
    fn the_statue_tick_rearms_then_judges_the_timeout() {
        let (mut world, _orders, _inbox, _lease) = a_fighting_world();
        world.meley_set_step(GUILD_ID, 4, NOW);
        let (_, due) = world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.statues_due)
            .expect("step 4 arms");
        world.run_meley_statues(GUILD_ID, due, NOW);
        let (first, rearm) = world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.statues_due)
            .expect("the kill window arms");
        assert!(first);
        assert_eq!(
            rearm,
            due.saturating_add(super::super::mob_ai::pulses(10)),
            "the 10-second kill window"
        );
        world.run_meley_statues(GUILD_ID, rearm, NOW);
        let (warp, _) = world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.end_due)
            .expect("the timeout warps");
        assert!(warp, "the warp arm, not the hour");
    }

    /// The end tick: the hour ends the dungeon timed out, and the warp drops
    /// the run.
    #[test]
    fn the_hour_ends_the_dungeon_and_the_warp_drops_the_run() {
        let (mut world, _orders, _inbox, _lease) = a_fighting_world();
        let (_, due) = world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.end_due)
            .expect("the hour arms");
        world.run_meley_end(GUILD_ID, due, NOW);
        let (warp, warp_due) = world
            .meley
            .of(GUILD_ID)
            .and_then(|run| run.end_due)
            .expect("the timeout warps");
        assert!(warp);
        world.run_meley_end(GUILD_ID, warp_due, NOW);
        assert!(world.meley.of(GUILD_ID).is_none(), "the run drops");
    }

    /// Install the fight's spawner and stand the empty base region, so the
    /// register's start pieces stand. Split out so `a_meley_world` stays under
    /// the line lint.
    fn install_meley_spawns(world: &mut GameState) {
        let rows = [
            NPC_VNUM,
            GATE_VNUM,
            BOSS_VNUM,
            STATUE_VNUM,
            CHEST_VNUM,
            STONE_VNUM,
            BOSS3_VNUM,
            SUBBOSS3_VNUM,
            COMMON_VNUMS[0],
            COMMON_VNUMS[1],
            COMMON_VNUMS[2],
        ]
        .into_iter()
        .map(|vnum| gamedata::mob_proto::MobProto {
            line: 2,
            table: gamedata::records::MobTableRecord {
                vnum,
                mob_type: CHAR_TYPE_MONSTER,
                max_hp: 10_000,
                ..gamedata::records::MobTableRecord::default()
            },
        });
        let respawner = Respawner::new(
            Arc::new(gamedata::mob_proto::MobProtos::from_rows(rows.collect())),
            Arc::new(gamedata::mob_group::MobGroups::default()),
            Arc::new(gamedata::mob_locale_names::MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap()),
            NpcVids::starting_at(FIRST_NPC_VID + 100),
        );
        world.install_respawner(respawner, meley_cells());
        world.stand_npcs(
            1,
            &gamedata::map_atlas::MapRegion {
                index: MAP,
                name: b"meley".to_vec(),
                sx: 100_000,
                sy: 200_000,
                ex: 200_000,
                ey: 300_000,
                spawn: (113_000, 213_000),
                empire_spawns: None,
            },
            MapNpcs::default(),
        );
    }

    /// The fight's open cells: [`open_cells`] plus a 32 by 32 open grid for the
    /// Meley base, which the start spawns land on.
    fn meley_cells() -> Arc<world::cells::HostedCells> {
        use super::super::fixtures::open_cells;
        let blocks = (0..1024)
            .map(|_| {
                gamedata::server_attr::CellBlock::from_cells(&vec![
                    0;
                    gamedata::server_attr::SECTREE_CELLS
                        * gamedata::server_attr::SECTREE_CELLS
                ])
            })
            .collect();
        let attr = Arc::new(
            gamedata::server_attr::ServerAttr::from_blocks(32, 32, blocks).expect("1024 blocks"),
        );
        let mut hosted = (*open_cells()).clone();
        // The grid starts at the region's own first sectree (15, 31): the spawn
        // cells (17..19, 33..36) must fall inside it, as the ship's 32-wide grid
        // from (0, 0) covers its spawns.
        let grid = gamedata::server_attr::SectreeGrid {
            x: 15,
            y: 31,
            columns: 32,
            rows: 32,
        };
        hosted.insert(MAP, world::cells::MapCells::new(grid, Arc::clone(&attr)));
        Arc::new(hosted)
    }

    /// A stone of `vnum` under `vid` at the run's index.
    fn a_stone(vid: u32, vnum: u32) -> Npc {
        Npc {
            vid,
            vnum,
            race: u16::try_from(vnum).unwrap_or(0),
            char_type: CHAR_TYPE_STONE,
            hp: 100,
            max_hp: 100,
            ..Npc::default()
        }
    }

    /// Stand `npcs` on the run's index on channel 1.
    fn stand_on_run(world: &mut GameState, npcs: Vec<Npc>) {
        let index = world
            .meley
            .of(GUILD_ID)
            .map(|run| run.index)
            .expect("the run stands");
        let region = gamedata::map_atlas::MapRegion {
            index,
            name: Vec::new(),
            sx: 100_000,
            sy: 200_000,
            ex: 200_000,
            ey: 300_000,
            spawn: (113_000, 213_000),
            empire_spawns: None,
        };
        world.stand_npcs(
            1,
            &region,
            MapNpcs {
                npcs,
                positions: Vec::new(),
                regens: Vec::new(),
            },
        );
    }

    /// The gate click starts the fight: step 1 runs, the gate dies, the hour
    /// arms, and a statue's hit above its line lands while the same hit at the
    /// line flags it and moves on when all four carry the step's flag.
    #[test]
    fn the_gate_click_starts_the_fight_and_the_statues_flag_at_their_line() {
        use crate::loading_phase::AFF_STATUE1;
        let (mut world, _orders, _inbox, _lease) = a_fighting_world();
        let run = world.meley.of(GUILD_ID).expect("the run stands");
        assert_eq!(run.step, 1);
        assert!(run.gate.is_none());
        assert!(run.end_due.is_some());
        let statues: Vec<u32> = world
            .meley
            .of(GUILD_ID)
            .map(|run| run.statues.into_iter().flatten().collect())
            .expect("the run stands");
        assert_eq!(statues.len(), 4);
        // Above the 75% line the hit lands and flags nothing.
        let first = statues[0];
        assert!(world.meley_statue_damage(GUILD_ID, first));
        assert!(!world.meley_npc_flag(first, AFF_STATUE1));
        // At the line the statue heals to it, takes the step's flag, and the hit
        // blocks.
        if let Some(npc) = world.npc_mut(first) {
            npc.hp = 7_500;
        }
        assert!(!world.meley_statue_damage(GUILD_ID, first));
        assert!(world.meley_npc_flag(first, AFF_STATUE1));
        assert_eq!(world.npc_by_vid(first).map(|npc| npc.hp), Some(7_500));
        // The other three at the line move the step to 2.
        for statue in &statues[1..] {
            if let Some(npc) = world.npc_mut(*statue) {
                npc.hp = 7_500;
            }
            assert!(!world.meley_statue_damage(GUILD_ID, *statue));
        }
        assert_eq!(world.meley.of(GUILD_ID).map(|run| run.step), Some(2));
        // A flagged statue blocks without touching the step.
        assert!(!world.meley_statue_damage(GUILD_ID, first));
        assert_eq!(world.meley.of(GUILD_ID).map(|run| run.step), Some(2));
    }

    /// The kill hook: a step-2 stone's death counts it, and the fourth stone's
    /// strips the statues' step-2 flags; before step 2 nothing counts.
    #[test]
    fn the_stone_kills_strip_the_statues_at_the_limit() {
        use crate::loading_phase::{AFFECT_STATUE, AFF_STATUE1, AFF_STATUE2};
        let (mut world, _orders, _inbox, _lease) = a_fighting_world();
        // The enter only ordered the warp; the master stands on the run now.
        let index = world
            .meley
            .of(GUILD_ID)
            .map(|run| run.index)
            .expect("the run stands");
        if let Some(body) = world.bodies.get_mut(&Vid::new(MASTER_VID)) {
            body.map = index;
        }
        // Step 1: a stone's death counts nothing.
        stand_on_run(&mut world, vec![a_stone(9001, STONE_VNUM)]);
        world.dead_npc(9001, Some(Vid::new(MASTER_VID)));
        assert_eq!(
            world.meley.of(GUILD_ID).map(|run| run.stones_killed),
            Some(0)
        );
        // Step 2 with the step-1 flags stripped and all four carrying the
        // step's flag: each stone's death counts, and the fourth strips them.
        let now = GameState::global_time();
        world.meley_set_step(GUILD_ID, 2, now);
        let statues: Vec<u32> = world
            .meley
            .of(GUILD_ID)
            .map(|run| run.statues.into_iter().flatten().collect())
            .expect("the run stands");
        for statue in &statues {
            world.remove_npc_affect(*statue, AFFECT_STATUE);
            world.meley_flag_statue(*statue, AFFECT_STATUE, AFF_STATUE1);
        }
        for (slot, vid) in [9010, 9011, 9012, 9013].into_iter().enumerate() {
            stand_on_run(&mut world, vec![a_stone(vid, STONE_VNUM)]);
            world.dead_npc(vid, Some(Vid::new(MASTER_VID)));
            let count = world
                .meley
                .of(GUILD_ID)
                .map(|run| run.stones_killed)
                .expect("the run stands");
            // The first three count up; the fourth resets to 0 with the strip.
            let want = if slot < 3 { slot + 1 } else { 0 };
            assert_eq!(count, u32::try_from(want).unwrap_or(0));
        }
        for statue in &statues {
            assert!(!world.meley_npc_flag(*statue, AFF_STATUE1));
        }
        let _ = AFF_STATUE2;
    }

    /// The reward: before the chest's clock nothing answers, past it the first
    /// take gives chest 1 for argument 1 and chest 2 else, and a second take is
    /// quiet.
    #[test]
    fn the_reward_gives_one_chest_past_the_clock() {
        use world::item::ItemIdRange;
        let (mut world, _orders, _inbox, _lease) = a_fighting_world();
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 1).expect("a usable range"))
            .expect("the first install");
        world.protos = Arc::new(crate::game_state::ItemProtos::from_rows(vec![
            gamedata::item_proto::ItemProto::for_category_rule(REWARD_VNUMS[0], 0, 0),
            gamedata::item_proto::ItemProto::for_category_rule(REWARD_VNUMS[1], 0, 0),
        ]));
        let now = GameState::global_time();
        if let Some(run) = world.meley.of_mut(GUILD_ID) {
            run.step = 4;
            run.reward_at = now.saturating_add(60);
        }
        assert!(!world.meley_can_reward(Vid::new(MASTER_VID), now));
        assert!(world.meley_reward(Vid::new(MASTER_VID), 1, now).is_none());
        if let Some(run) = world.meley.of_mut(GUILD_ID) {
            run.reward_at = now;
        }
        assert!(world.meley_can_reward(Vid::new(MASTER_VID), now));
        let (moved, _) = world
            .meley_reward(Vid::new(MASTER_VID), 1, now)
            .expect("the first take gives");
        assert_eq!(moved.changes.len(), 1);
        assert!(!world.meley_can_reward(Vid::new(MASTER_VID), now));
        assert!(world.meley_reward(Vid::new(MASTER_VID), 2, now).is_none());
    }
}
