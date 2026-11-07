//! Meley's Lair (`MeleyLair::CMgr`, `G/MeleyLair.cpp`): the guild dungeon's lobby —
//! the register and enter gates, the leave and check warps, the participant rows,
//! and the dungeon triple each guild carries (`dungeon_ch`, `dungeon_map`,
//! `dungeon_cooldown`, `G/guild.h:433-438`).
//!
//! Legacy keeps one manager per game core: the registered guilds by id
//! (`m_RegGuilds`), the dungeon's centre and the sub map's warp spot (read once
//! from the hosted sectree settings, lazily on the first register), and the fight
//! engine (the steps, spawns and rewards, `sys.guild.dungeon.fight`) behind them.
//! The one process keeps the registrations on the game thread, keyed by guild id,
//! and reads the private map through the dungeon manager: `CreatePrivateMap` and
//! `DestroyPrivateMap` are the same calls the party dungeons use.
//!
//! A registration holds what the lobby names: the private index and the participant
//! PIDs (`CMgrMap::v_Partecipants`). The fight engine adds the step, the spawns and
//! the channel beside them; the lobby halves below stay as written.
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

/// The sub map warp spot in map units (`Register`, `G/MeleyLair.cpp:1063-1065`):
/// the sub map's base plus (87, 853).
pub(super) const SUB_SPOT: (i32, i32) = (87, 853);

/// One registered guild: the `CMgrMap` narrowed to what the lobby names. The fight
/// engine adds the step, the spawns and the channel beside them
/// (`sys.guild.dungeon.fight`).
#[derive(Clone, Debug, Default)]
pub(super) struct MeleyRun {
    /// The private index `CreatePrivateMap` gave (`CMgrMap::map_index`).
    pub index: i32,
    /// The participant PIDs (`CMgrMap::v_Partecipants`).
    pub participants: BTreeSet<u32>,
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
                participants: BTreeSet::new(),
            },
        );
    }
}

use common::vid::Vid;

use super::guild::{GuildQuestStore, MeleyGuildRow};
use super::GameState;

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
        calls: (u32, u32, u32, u32),
        now: u32,
    ) -> Vec<GuildQuestStore> {
        let mut stores = Vec::new();
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
        stores
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
        self.tell_ladder(id, ladder);
        Some(GuildQuestStore::Ladder {
            guild_id: id,
            ladder,
        })
    }

    /// Judge one `MeleyLair.Enter` (`Enter`, `G/MeleyLair.cpp:1075-1154`): the
    /// cross-Channel refusal, the late create, the full, finished and PK gates,
    /// then the participant row, the exit save and the ordered warp. The step and
    /// the recall read what the fight engine will own; the finished arm waits for
    /// it (`sys.guild.dungeon.fight`).
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
        }
        let Some(run) = self.meley.of(id) else {
            return;
        };
        let index = run.index;
        if run.participants.len() >= PARTECIPANTS_LIMIT {
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
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::fixtures::{a_world, enter, TestClock};
    use super::*;
    use crate::client_registry::{ChannelClients, ClientEntry, ClientOrder, Lease};
    use protocol::gc_guild::GUILD_SKILL_COUNT;

    const MASTER_VID: u32 = 100;
    const GUILD_ID: u32 = 7;
    const NOW: u32 = 10_000_000;

    /// A guilded master with a ladder of 600 and level 3, a client lease on its
    /// map, and an atlas naming maps 212 and 62.
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
}
