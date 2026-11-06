//! The dungeons (`CDungeonManager`, `G/dungeon.cpp`): the private map a party jumps
//! into, its flags, its warp spot, its notices, its exits and its spawns.
//!
//! Legacy keeps one manager per game core: the dungeons, keyed by id and by private
//! map index, and the first free private index a create takes
//! (`SECTREE_MANAGER::CreatePrivateMap`, `G/sectree_manager.cpp:943-1026`). The one
//! process keeps them here, on the game thread, keyed by private index: the names
//! the scripts call (`d.select`, `d.getf_from_map_index`) name the dungeon by its
//! index, never by its id.
//!
//! A dungeon is a [`Dungeon`]: its base map, its flags (`m_mapFlag`), and the Pulse
//! its dead event fires on, if one is armed. The world hosts the private map beside
//! it (`host_map`, copied from the base map's grid, as `CreatePrivateMap` copies the
//! sectree map), destroys it when the dungeon goes (`DestroyPrivateMap`), and stamps
//! membership (`m_pkDungeon`) on the characters that enter: what `pc.in_dungeon`
//! answers, and what `d.get_map_index` reads when a character runs. A character
//! whose map is a dungeon's index is its member; a character that leaves reverts to
//! none, as `SetDungeon(NULL)` does on logout, death's revive and unparty
//! (`G/char.cpp:588`, `:5604-5606`).
//!
//! The dead event (`dungeon_dead_event`, `G/dungeon.cpp:192-211`) fires 300 seconds
//! after the last member left: it destroys the dungeon, its map and every server
//! timer keyed by its index (`CancelServerTimers`, `G/dungeon.cpp:448`).
//!
//! # Divergences
//!
//! - **The private index cursor.** Legacy walks the indices round-robin from a
//!   per-map cursor (`next_private_index_map_`, `G/sectree_manager.cpp:956-984`).
//!   The Rewrite takes the lowest free one, as the guild war port did before it,
//!   because the order is unobservable: every consumer names the index it was given.
//! - **The map copy.** Legacy copies the whole `SECTREE_MAP` with the characters on
//!   it (`M2_NEW SECTREE_MAP(*pkMapSectree)`, `G/sectree_manager.cpp:1022`) — the
//!   copy's trees still list entities standing on the base map. The Rewrite hosts a
//!   fresh grid copy with no entities, which is what the copy is for: a map with no
//!   one on it yet.

use std::collections::BTreeMap;

use common::vid::Vid;

/// The private map index base (`G/sectree_manager.cpp:957`).
pub(super) const PRIVATE_MAP_BASE: i32 = 10_000;

/// The Pulses between the last member's leave and the dungeon's destruction
/// (`PASSES_PER_SEC(300)`, `G/dungeon.cpp:237`).
pub(super) const DEAD_EVENT_PULSES: u64 = 300 * 25;

/// One dungeon: `CDungeon` narrowed to what the ported names read and write.
#[derive(Clone, Debug, Default)]
pub(super) struct Dungeon {
    /// `m_lOrigMapIndex`: the map the private map was copied from.
    pub base: i32,
    /// `m_mapFlag`: the flags `d.setf` writes and `d.getf` reads, by name.
    pub flags: BTreeMap<Vec<u8>, i32>,
    /// The Pulse the dead event fires on, armed when the last member left.
    pub dead_due: Option<u64>,
    /// The leader PID of the party the dungeon jumped in (`m_pParty`,
    /// `G/dungeon.cpp:410-426`): a second party can never jump into the same
    /// dungeon. Legacy leaves the dungeon unbound when the jumping party already
    /// stands in another one; the Rewrite binds it, because which dungeon a party
    /// stands in is read from the members' maps.
    pub party: Option<u32>,
    /// `m_set_pkChr` narrowed to players: the VIDs on the map, which `d.exit_all`
    /// sends out and `d.notice` reaches through their bodies.
    pub present: std::collections::HashSet<Vid>,
    /// `d.exit_all`'s destination per member: the place the jump remembered
    /// (`SaveExitLocation`, `G/char.cpp:6679-6683`), overwritten for the members
    /// present by `d.set_warp_location` (`CHARACTER::SetWarpLocation`,
    /// `G/char.cpp:6672-6677`). Legacy's `ExitToSavedLocation` warps to the
    /// pending Warp (`m_posWarp`), which the jump's `WarpSet` left holding the
    /// dungeon's own spot — but the reconnect on the new address re-seeds it from
    /// the row's exit columns, which the join wrote, so the member lands where it
    /// came from, which is what this names.
    pub exits: std::collections::HashMap<Vid, crate::save::ExitLocation>,
}

/// The dungeons (`CDungeonManager`), keyed by private map index.
#[derive(Debug, Default)]
pub(super) struct Dungeons {
    /// `m_map_pkMapDungeon`, by private index.
    by_index: BTreeMap<i32, Dungeon>,
    /// The server timers, by (name, argument): `m_mapServerTimer`
    /// (`G/questmanager.cpp:1967-1976`), with the Pulse each fires on.
    timers: BTreeMap<(Vec<u8>, u32), u64>,
}

impl Dungeons {
    /// Every living dungeon's flags, by private index: the quest step's snapshot,
    /// so `d.getf` answers what the world would judge while `d.setf` queues.
    pub(super) fn flags_snapshot(&self) -> BTreeMap<i32, BTreeMap<Vec<u8>, i32>> {
        self.by_index
            .iter()
            .map(|(index, dungeon)| (*index, dungeon.flags.clone()))
            .collect()
    }
    /// Whether a dungeon lives under `index` (`CDungeonManager::FindByMapIndex`,
    /// `G/dungeon.cpp:461-469`): what `d.select` and `d.find` ask.
    pub(super) fn lives(&self, index: u32) -> bool {
        i32::try_from(index)
            .ok()
            .is_some_and(|index| self.by_index.contains_key(&index))
    }

    /// `CDungeonManager::Create(map)` (`G/dungeon.cpp:470-501`): the first free
    /// private index under `map`, or `None` when `map` is not a base map, when the
    /// base map is not hosted, or when every instance is taken. The map copy is
    /// hosted by the caller, which reads the base map's grid.
    pub(super) fn create(
        &mut self,
        map: i32,
        hosted: impl Fn(i32) -> bool,
        taken: &[i32],
    ) -> Option<i32> {
        if map <= 0 || map >= PRIVATE_MAP_BASE || !hosted(map) {
            return None;
        }
        let cap = if matches!(map, 107..=109) { 51 } else { 10_000 };
        let base = map * PRIVATE_MAP_BASE;
        let index = (0..cap)
            .map(|offset| base + offset)
            .find(|index| !self.by_index.contains_key(index) && !taken.contains(index))?;
        self.by_index.insert(
            index,
            Dungeon {
                base: map,
                ..Dungeon::default()
            },
        );
        Some(index)
    }

    /// `CDungeonManager::Destroy` (`G/dungeon.cpp:434-453`): the dungeon under
    /// `index` goes, with every server timer keyed by it. Answers whether one lived.
    pub(super) fn destroy(&mut self, index: i32) -> bool {
        if self.by_index.remove(&index).is_none() {
            return false;
        }
        let arg = u32::try_from(index).unwrap_or(0);
        self.timers.retain(|(_, timer_arg), _| *timer_arg != arg);
        true
    }

    /// The dungeon under `index`, when one lives.
    pub(super) fn get(&self, index: i32) -> Option<&Dungeon> {
        self.by_index.get(&index)
    }

    /// `d.setf` (`G/questlua_dungeon.cpp:106-130`): `name` of the dungeon under
    /// `index` takes `value`. Quiet with none.
    pub(super) fn set_flag(&mut self, index: i32, name: Vec<u8>, value: i32) {
        if let Some(dungeon) = self.by_index.get_mut(&index) {
            dungeon.flags.insert(name, value);
        }
    }

    /// `CDungeon::IncMember` (`G/dungeon.cpp:213-219`): a member entered the
    /// dungeon under `index`, so its dead event is cancelled. Quiet with none.
    pub(super) fn member_entered(&mut self, index: i32) {
        if let Some(dungeon) = self.by_index.get_mut(&index) {
            dungeon.dead_due = None;
        }
    }

    /// `CDungeon::DecMember` (`G/dungeon.cpp:221-239`): the last member left the
    /// dungeon under `index` on `pulse`, so its dead event fires 300 seconds on.
    /// Quiet with none. The caller counts the members; this arms the event.
    pub(super) fn last_member_left(&mut self, index: i32, pulse: u64) {
        if let Some(dungeon) = self.by_index.get_mut(&index) {
            dungeon.dead_due = Some(pulse.saturating_add(DEAD_EVENT_PULSES));
        }
    }

    /// The dead events due on `pulse` (`dungeon_dead_event`, `G/dungeon.cpp:192-211`):
    /// the private indices whose dungeons go, in index order.
    pub(super) fn dead_due(&self, pulse: u64) -> Vec<i32> {
        self.by_index
            .iter()
            .filter(|(_, dungeon)| dungeon.dead_due.is_some_and(|due| due <= pulse))
            .map(|(index, _)| *index)
            .collect()
    }

    /// `CQuestManager::AddServerTimer` (`G/questmanager.cpp:1967-1976`): the timer
    /// (`name`, `arg`) fires `pulses` Pulses on. Quiet when one is already armed.
    pub(super) fn add_timer(&mut self, name: Vec<u8>, arg: u32, due: u64) {
        if !self.timers.contains_key(&(name.clone(), arg)) {
            self.timers.insert((name, arg), due);
        }
    }

    /// `CQuestManager::ClearServerTimer` (`G/questmanager.cpp:1983-1992`): the timer
    /// (`name`, `arg`) never fires.
    pub(super) fn clear_timer(&mut self, name: &[u8], arg: u32) {
        self.timers.remove(&(name.to_vec(), arg));
    }

    /// The timers due on `pulse`, in (name, argument) order, each forgotten as
    /// `ClearServerTimerNotCancel` forgets the one-shot (`G/questevent.cpp:38-43`).
    pub(super) fn timers_due(&mut self, pulse: u64) -> Vec<(Vec<u8>, u32)> {
        let due: Vec<(Vec<u8>, u32)> = self
            .timers
            .iter()
            .filter(|(_, fire)| **fire <= pulse)
            .map(|(key, _)| key.clone())
            .collect();
        for key in &due {
            self.timers.remove(key);
        }
        due
    }
}

impl super::GameState {
    /// `d.new_jump_party(map, x, y)` (`G/questlua_dungeon.cpp:407-439`): a dungeon of
    /// `map` is created on the reader's Channel, and the reader's party jumps in at
    /// (`x`, `y`), answering the created index. The create stands even when nobody
    /// jumps: legacy checks the party after the create and only logs `cannot go to
    /// dungeon alone`.
    ///
    /// A dungeon of another party refuses the jump (`G/dungeon.cpp:410-426`). The
    /// jumpers are the party's members on the reader's map
    /// (`ForEachOnMapMember`); each remembers the place it came from and is
    /// ordered the warp, which re-seeds the row's exit columns on the reconnect.
    /// `x` and `y` are the map's own units (`JumpParty` multiplies by 100,
    /// `G/dungeon.cpp:396`).
    pub(super) fn dungeon_jump_party(
        &mut self,
        reader: Vid,
        map: i32,
        x: i32,
        y: i32,
    ) -> Option<i32> {
        let (channel, from) = self.place_of(reader.raw())?;
        let hosted = |base: i32| self.recall.hosts(channel, base);
        let taken: Vec<i32> = self.maps.keys().map(|&(_, map)| map).collect();
        let index = self.dungeons.create(map, hosted, &taken)?;
        let Some(grid) = self
            .recall
            .atlas()
            .and_then(|atlas| atlas.region(map))
            .and_then(gamedata::server_attr::SectreeGrid::of)
        else {
            self.dungeons.destroy(index);
            return None;
        };
        self.host_map(channel, index, grid);
        let Ok(character) = self.characters.find_by_vid(reader) else {
            return Some(index);
        };
        let pid = character.player_id();
        let Some(leader) = self.parties.leader_of(pid) else {
            return Some(index);
        };
        let bound = self.dungeons.get(index).and_then(|dungeon| dungeon.party);
        if bound.is_some_and(|bound| bound != leader) {
            return Some(index);
        }
        let Some(members) = self.parties.on_map_members(leader, from, &self.bodies) else {
            return Some(index);
        };
        if let Some(dungeon) = self.dungeons.by_index.get_mut(&index) {
            dungeon.party = Some(leader);
        }
        for member in members {
            let Ok(character) = self.characters.find_by_pid(member) else {
                continue;
            };
            self.dungeon_jump_member(index, channel, character.vid(), x, y);
        }
        self.dungeons.member_entered(index);
        Some(index)
    }

    /// One member's jump into the dungeon under `index`: the place it stands is
    /// remembered, and its client is ordered the warp to (`x`, `y`) on the private
    /// map, on the Channel that hosts it.
    fn dungeon_jump_member(&mut self, index: i32, channel: u8, vid: Vid, x: i32, y: i32) {
        self.dungeon_jump_member_scaled(index, channel, vid, x, y, true);
    }

    /// The member jump with (`d.new_jump_party`) or without (`d.new_jump`) `JumpParty`'s
    /// hundredfold scale: the exit save, then the warp order for the finished point.
    fn dungeon_jump_member_scaled(
        &mut self,
        index: i32,
        channel: u8,
        vid: Vid,
        x: i32,
        y: i32,
        map_units: bool,
    ) {
        let raw = vid.raw();
        let Some((_, from)) = self.place_of(raw) else {
            return;
        };
        // `SaveExitLocation` (`G/char.cpp:6679-6683`): the place the member came from,
        // which the reconnect re-seeds the pending Warp from and `d.exit_all` sends it
        // back to.
        let exit = self.spot_of(raw).map(|spot| crate::save::ExitLocation {
            x: spot.x,
            y: spot.y,
            map_index: from,
        });
        if let Some(exit) = exit {
            if let Some(dungeon) = self.dungeons.by_index.get_mut(&index) {
                dungeon.exits.insert(vid, exit);
                dungeon.present.insert(vid);
            }
        }
        let (x, y) = if map_units {
            (x.saturating_mul(100), y.saturating_mul(100))
        } else {
            (x, y)
        };
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        let ordered = clients
            .members_on_map(channel, from)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| {
                clients.order(
                    channel,
                    id,
                    crate::client_registry::ClientOrder::Warp {
                        x,
                        y,
                        private_map_index: index,
                        channel: Some(channel),
                        exit,
                        sorted_until: self.sort_lock(vid),
                    },
                )
            });
        if !ordered {
            tracing::warn!(vid = raw, "a dungeon jump found no client to order");
        }
    }

    /// `d.notice(text)` (`G/questlua_dungeon.cpp:52-89`): every player on the dungeon's
    /// map hears `text` as a notice, in VID order. Quiet with none.
    pub(super) fn dungeon_notice(&mut self, index: i32, text: &[u8]) {
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == index)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, protocol::gc_chat::CHAT_TYPE_NOTICE, text, &[]);
            }
        }
    }

    /// `d.set_warp_location(map, x, y)` (`G/questlua_dungeon.cpp:1603-1622`): every
    /// member present takes (`map`, `x`, `y`) as the place `d.exit_all` sends it
    /// (`CHARACTER::SetWarpLocation`, `G/char.cpp:6672-6677`), in the map's own
    /// units. Quiet with none.
    pub(super) fn dungeon_set_warp_location(&mut self, index: i32, map: i32, x: i32, y: i32) {
        let Some(dungeon) = self.dungeons.by_index.get(&index) else {
            return;
        };
        let mut vids: Vec<Vid> = dungeon.present.iter().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(dungeon) = self.dungeons.by_index.get_mut(&index) {
                dungeon.exits.insert(
                    vid,
                    crate::save::ExitLocation {
                        x: x.saturating_mul(100),
                        y: y.saturating_mul(100),
                        map_index: map,
                    },
                );
            }
        }
    }

    /// `d.exit_all()` (`G/questlua_dungeon.cpp:1257-1265`): every member present is
    /// ordered the warp back to the place it came from (`ExitToSavedLocation`,
    /// `G/char.cpp:6687-6692`), in VID order. Quiet with none.
    pub(super) fn dungeon_exit_all(&mut self, index: i32) {
        let Some(dungeon) = self.dungeons.by_index.get(&index) else {
            return;
        };
        let mut vids: Vec<Vid> = dungeon.present.iter().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let Some(exit) = self
                .dungeons
                .by_index
                .get(&index)
                .and_then(|dungeon| dungeon.exits.get(&vid).copied())
            else {
                continue;
            };
            self.order_dungeon_exit(vid, exit);
            if let Some(dungeon) = self.dungeons.by_index.get_mut(&index) {
                dungeon.present.remove(&vid);
            }
        }
    }

    /// One member's warp back to the place it came from: the ordered warp names the
    /// remembered exit, and the member leaves the dungeon's present set.
    fn order_dungeon_exit(&mut self, vid: Vid, exit: crate::save::ExitLocation) {
        let raw = vid.raw();
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        let Some((channel, map)) = self.place_of(raw) else {
            return;
        };
        let private = if exit.map_index >= PRIVATE_MAP_BASE {
            exit.map_index
        } else {
            0
        };
        let ordered = clients
            .members_on_map(channel, map)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| {
                clients.order(
                    channel,
                    id,
                    crate::client_registry::ClientOrder::Warp {
                        x: exit.x,
                        y: exit.y,
                        private_map_index: private,
                        channel: None,
                        exit: None,
                        sorted_until: self.sort_lock(vid),
                    },
                )
            });
        if !ordered {
            tracing::warn!(vid = raw, "a dungeon exit found no client to order");
        }
    }

    /// `d.spawn_mob(vnum, x, y)` (`G/questlua_dungeon.cpp:1048-1099`): the mob stands
    /// on the dungeon's map at the base point plus (`x`, `y`) in the map's own units
    /// (`CDungeon::SpawnMob`, `G/dungeon.cpp:762-781`), with a drawn facing. Quiet
    /// with no map, no cells, or no spawner.
    pub(super) fn dungeon_spawn_mob(&mut self, index: i32, vnum: u32, x: i32, y: i32) {
        let (Some(base), pulse) = (
            self.dungeons.get(index).map(|dungeon| dungeon.base),
            self.last_pulse,
        ) else {
            return;
        };
        let Some(channel) = self
            .maps
            .keys()
            .find(|(_, map)| *map == index)
            .map(|(channel, _)| *channel)
        else {
            return;
        };
        let place = (channel, index);
        let (Some(region), Some(cells)) = (
            self.regens.region_of(channel, base),
            self.regens.cells_of(index),
        ) else {
            return;
        };
        let Some(respawner) = self.regens.respawner_mut() else {
            return;
        };
        let (base_x, base_y) = self
            .maps
            .get(&place)
            .map_or((0, 0), super::view::MapIndex::base);
        let at = (
            base_x.saturating_add(x.saturating_mul(100)),
            base_y.saturating_add(y.saturating_mul(100)),
        );
        match respawner.spawn_directed((&region, &cells), vnum, at, -1, false, &mut self.dice) {
            Ok(Some(stood)) => self.stand_runtime(place, vec![stood], pulse),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(?place, %error, "a dungeon spawn outnumbered the VIDs left");
            }
        }
    }

    /// `CDungeon::DecMember` (`G/dungeon.cpp:221-239`): the player left the map of the
    /// dungeon under `map`, so it leaves the dungeon's present set. The last one out
    /// arms the dead event 300 seconds on; a member still in cancels nothing. Quiet
    /// with none.
    pub(super) fn leave_dungeon(&mut self, vid: Vid, map: i32) {
        let Some(dungeon) = self.dungeons.by_index.get_mut(&map) else {
            return;
        };
        dungeon.present.remove(&vid);
        if dungeon.present.is_empty() {
            let pulse = self.last_pulse;
            self.dungeons.last_member_left(map, pulse);
        }
    }

    /// `dungeon_dead_event` (`G/dungeon.cpp:192-211`) and `quest_server_timer_event`
    /// (`G/questevent.cpp:24-46`): the dungeons whose dead events are due go, with the
    /// server timers keyed by their indices, their maps unhosted and their players
    /// disconnected; then the due server timers fire. A one-shot is forgotten as it
    /// fires (`ClearServerTimerNotCancel`).
    pub(super) fn run_dungeon_ticks(&mut self, pulse: u64) {
        for index in self.dungeons.dead_due(pulse) {
            self.destroy_dungeon(index);
        }
        for (name, arg) in self.dungeons.timers_due(pulse) {
            self.fire_server_timer(&name, arg);
        }
    }

    /// `CQuestManager::ServerTimer` (`G/questmanager.cpp:635-643`): the timer `name`
    /// fires with `arg`, so the timer NPC's `server_timer` scripts run for PID 0,
    /// which stands nowhere and holds nothing. The step's queues apply as any
    /// step's do; its records go nowhere, because PID 0 has no client. The step
    /// runs on the dungeon's Channel, as legacy's runs on its core's
    /// (`g_bChannel`, which `pc.get_channel_id` reads), so a channel-suffixed
    /// event flag reads what the dungeon's scripts wrote.
    fn fire_server_timer(&mut self, name: &[u8], arg: u32) {
        let index = i32::try_from(arg).unwrap_or(0);
        let channel = self
            .maps
            .keys()
            .find(|(_, map)| *map == index)
            .map_or(0, |(channel, _)| *channel);
        let place = crate::game_loop_messages::GroundPlace {
            channel,
            map: 0,
            x: 0,
            y: 0,
        };
        let mover = crate::item_move::Mover {
            recently_fought: false,
            empire: 0,
            language: 0,
            pk_mode: 0,
            affect_flags: [0, 0],
            dead: false,
            alignment: 0,
            view_guild: (0, 0),
        };
        let name = name.to_vec();
        if let Some((_, _, _, _, take_sides, _)) = self.run_quests_giving(
            place,
            (mover, 0, 0),
            super::quests::Giving::Nothing,
            crate::game_loop_messages::QuestAccount::default(),
            |manager, game| manager.server_timer(&name, arg, game),
        ) {
            if !take_sides.is_empty() {
                tracing::warn!(
                    sides = take_sides.len(),
                    "a quest took blocked items in a timer step; their rows are dropped"
                );
            }
            debug_assert!(take_sides.is_empty(), "take rows in a timer step");
        }
    }

    /// `CDungeonManager::Destroy` (`G/dungeon.cpp:434-453`): the dungeon under `index`
    /// goes, with every server timer keyed by it. Its map is unhosted and every
    /// player on it is disconnected, as `DestroyPrivateMap` destroys the map's
    /// entities (`G/sectree_manager.cpp:1028-1052`).
    fn destroy_dungeon(&mut self, index: i32) {
        if !self.dungeons.destroy(index) {
            return;
        }
        let Some(channel) = self
            .maps
            .keys()
            .find(|(_, map)| *map == index)
            .map(|(channel, _)| *channel)
        else {
            return;
        };
        if let Some(clients) = self.clients.as_ref() {
            for (id, _) in clients.members_on_map(channel, index) {
                let _sent = clients.order(channel, id, crate::client_registry::ClientOrder::Close);
            }
        }
        self.maps.remove(&(channel, index));
    }

    /// `d.new_jump(map, x, y)` (`G/questlua_dungeon.cpp:318-348`): a dungeon of
    /// `map` is created on the reader's Channel, and the reader alone jumps in at
    /// (`x`, `y`), answering the created index. Unlike the party jump no party
    /// binds and no member list is read: the current character warps (`WarpSet`).
    ///
    /// Legacy dereferences the current character unchecked (`ch->WarpSet`), so a
    /// server timer's script calling `d.new_jump` crashes it; the Rewrite is quiet
    /// with no reader instead (a Defect, never reproduced). The create stands even
    /// then, as the party jump's does.
    pub(super) fn dungeon_jump_single(
        &mut self,
        reader: Vid,
        map: i32,
        x: i32,
        y: i32,
    ) -> Option<i32> {
        let (channel, _) = self.place_of(reader.raw())?;
        let hosted = |base: i32| self.recall.hosts(channel, base);
        let taken: Vec<i32> = self.maps.keys().map(|&(_, map)| map).collect();
        let index = self.dungeons.create(map, hosted, &taken)?;
        let Some(grid) = self
            .recall
            .atlas()
            .and_then(|atlas| atlas.region(map))
            .and_then(gamedata::server_attr::SectreeGrid::of)
        else {
            self.dungeons.destroy(index);
            return None;
        };
        self.host_map(channel, index, grid);
        if self.characters.find_by_vid(reader).is_err() {
            return Some(index);
        }
        self.dungeon_jump_member_raw(index, channel, reader, x, y);
        self.dungeons.member_entered(index);
        Some(index)
    }

    /// As [`GameState::dungeon_jump_member`], for `d.new_jump`, whose `WarpSet` takes the
    /// script's numbers as they are (`G/questlua_dungeon.cpp:345`): the solo caller already
    /// multiplied by 100 (`dungeonFunctions.lua:262`), while `JumpParty` multiplies its own
    /// (`G/dungeon.cpp:398-399`), so the shared member jump's scale would land the solo
    /// jumper a hundredfold past the entry.
    fn dungeon_jump_member_raw(&mut self, index: i32, channel: u8, vid: Vid, x: i32, y: i32) {
        self.dungeon_jump_member_scaled(index, channel, vid, x, y, false);
    }

    /// `d.clear_regen()` (`G/questlua_dungeon.cpp:235-243`): the dungeon's regens go.
    /// Quiet with none. No ported path creates a dungeon regen — `d.regen_file` and
    /// `d.set_regen_file` are `unused`, and `d.spawn_mob` stands one-shot — so there
    /// is nothing to cancel yet; the call exists so the name bridges.
    pub(super) fn dungeon_clear_regen(&mut self, index: i32) {
        if self.dungeons.get(index).is_some() {
            // Nothing to cancel: no ported path creates a dungeon regen.
        }
    }

    /// `d.kill_all()` (`G/questlua_dungeon.cpp:1491-1499`, `G/dungeon.cpp:996-1008`):
    /// every non-player on the dungeon's map dies (`Dead()`, no killer), in VID
    /// order. Players are spared structurally, as `FKillSectree` spares them; the
    /// pet, mount and auto-shaman gates are dead, as the purge's are (no ported
    /// path sets them). Quiet with none.
    pub(super) fn dungeon_kill_all(&mut self, index: i32) {
        let Some(channel) = self
            .maps
            .keys()
            .find(|(_, map)| *map == index)
            .map(|(channel, _)| *channel)
        else {
            return;
        };
        let mut vids: Vec<u32> = self
            .npc_of
            .iter()
            .filter(|(_, place)| place.0 == channel && place.1 == index)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_unstable();
        for vid in vids {
            self.dead_npc(vid, None);
        }
    }

    /// `d.set_damage_from_race(flag, vid)` (`G/questlua_dungeon.cpp:902-919`): the
    /// character under `vid` refuses damage from attackers of race `flag`
    /// (`SetNoDamageRaceFlag`, `G/char.cpp:100-105`). A flag at or past
    /// `MAIN_RACE_MAX_NUM` is refused, and a set bit stays set. Quiet with no
    /// such character, player or not.
    pub(super) fn dungeon_set_damage_race(&mut self, flag: u32, vid: u32) {
        if flag >= 8 {
            return;
        }
        if let Some(npc) = self.npc_mut(vid) {
            npc.ndr |= 1 << flag;
            return;
        }
        if self.bodies.contains_key(&Vid::new(vid)) {
            self.no_damage.entry(Vid::new(vid)).or_default().race |= 1 << flag;
        }
    }

    /// `d.set_damage_from_affect(flag, vid)` (`G/questlua_dungeon.cpp:921-938`): the
    /// character under `vid` takes damage only from attackers holding affect flag
    /// `flag` (`SetNoDamageAffectFlag`, `G/char.cpp:124-127`). Any flag inserts;
    /// the read bounds-checks (`TAffectFlag::IsSet`). Quiet with no such
    /// character, player or not.
    pub(super) fn dungeon_set_damage_affect(&mut self, flag: u32, vid: u32) {
        if let Some(npc) = self.npc_mut(vid) {
            npc.nda.insert(flag);
            return;
        }
        if self.bodies.contains_key(&Vid::new(vid)) {
            self.no_damage
                .entry(Vid::new(vid))
                .or_default()
                .affects
                .insert(flag);
        }
    }
}

/// One `server_timer` or `clear_server_timer` call, in call order with the other
/// kind: legacy executes the calls as they come, so an arm and a clear of the same
/// timer in one step leave it armed or cleared according to which came last.
pub(super) enum ServerTimerOp {
    /// `server_timer`'s name, seconds and argument.
    Set {
        /// The timer's name.
        name: Vec<u8>,
        /// The seconds the timer fires in.
        secs: f64,
        /// The timer's argument.
        arg: u32,
    },
    /// `clear_server_timer`'s name and argument.
    Clear {
        /// The timer's name.
        name: Vec<u8>,
        /// The timer's argument.
        arg: u32,
    },
}

/// A step's dungeon queues, for [`super::quests::GameState::apply_dungeon_moves`].
/// Named apart from [`super::quests::WorldMoves`] so that struct's destructure stays
/// readable.
pub(super) struct DungeonMoves {
    /// Every `d.setf` call's index, name and value, in order.
    pub sets: Vec<(i32, Vec<u8>, i32)>,
    /// Every `d.new_jump_party` call's staged index and its (map, x, y), in
    /// order.
    pub jumps: Vec<(i32, i32, i32, i32)>,
    /// Every `d.notice` call's index and text, in order.
    pub notices: Vec<(i32, Vec<u8>)>,
    /// Every `d.set_warp_location` call's index and (map, x, y), in order.
    pub warps: Vec<(i32, i32, i32, i32)>,
    /// Every `d.exit_all` call's index, in order.
    pub exits: Vec<i32>,
    /// Every `d.spawn_mob` call's index, vnum and (x, y), in order.
    pub spawns: Vec<(i32, u32, i32, i32)>,
    /// Every `d.new_jump` call's staged index and its (map, x, y), in order.
    pub singles: Vec<(i32, i32, i32, i32)>,
    /// Every `d.clear_regen` call's index, in order.
    pub clears: Vec<i32>,
    /// Every `d.kill_all` call's index, in order.
    pub kills: Vec<i32>,
    /// Every `d.set_damage_from_race` call's flag and VID, in order.
    pub damage_race: Vec<(u32, u32)>,
    /// Every `d.set_damage_from_affect` call's flag and VID, in order.
    pub damage_affect: Vec<(u32, u32)>,
    /// Every `server_timer` and `clear_server_timer` call, in call order.
    pub timer_ops: Vec<ServerTimerOp>,
    /// Every `game.set_event_flag` call's name and value, in order.
    pub event_sets: Vec<(Vec<u8>, i32)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosted(_: i32) -> bool {
        true
    }

    /// `d.kill_all` (`G/dungeon.cpp:996-1008`): every non-player on the dungeon's
    /// map dies, while the player standing beside it lives.
    #[test]
    fn a_kill_all_kills_the_dungeons_monsters_and_spares_its_players() {
        use crate::game_state::fixtures::{a_world, enter, region, TestClock};

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut map = region();
        map.index = 2_100_000;
        world.host_map(
            1,
            2_100_000,
            gamedata::server_attr::SectreeGrid {
                x: 0,
                y: 0,
                columns: 10,
                rows: 10,
            },
        );
        let npc = world::npc::Npc {
            vid: 1001,
            vnum: 101,
            race: 101,
            hp: 100,
            max_hp: 100,
            ..world::npc::Npc::default()
        };
        world.stand_npcs(
            1,
            &map,
            world::npc::MapNpcs {
                npcs: vec![npc],
                positions: Vec::new(),
                regens: Vec::new(),
            },
        );
        let mut inbox = enter(&mut world, 1, (100, 100), 12_345);
        while inbox.try_recv().is_ok() {}
        world.dungeon_kill_all(2_100_000);
        assert!(world.npc_by_vid(1001).unwrap().dead);
        assert!(!world.is_dead(1));
        assert!(world.npc_by_vid(1001).is_some());
    }

    /// The dungeon damage flags ride the character: a warp keeps them, a disconnect
    /// drops them (`SetNoDamageRaceFlag` writes its fields, `G/char.cpp:100-127`, and
    /// the fields go with the character).
    #[test]
    fn the_damage_flags_survive_a_warp_and_die_on_disconnect() {
        use crate::game_loop_messages::Departure;
        use crate::game_state::fixtures::{a_world, enter, TestClock};

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, 1, (100, 100), 12_345);
        while inbox.try_recv().is_ok() {}
        world.dungeon_set_damage_race(3, 1);
        world.leave_world_by(Vid::new(1), Departure::Warp);
        world.leave_world_by(Vid::new(1), Departure::Disconnect);
        assert!(!world.no_damage.contains_key(&Vid::new(1)));
        let mut inbox = enter(&mut world, 1, (100, 100), 12_345);
        while inbox.try_recv().is_ok() {}
        world.dungeon_set_damage_race(3, 1);
        world.leave_world_by(Vid::new(1), Departure::Warp);
        assert_eq!(world.no_damage.get(&Vid::new(1)).unwrap().race, 1 << 3);
    }

    #[test]
    fn a_create_names_the_first_free_private_index() {
        // `CreatePrivateMap` (`G/sectree_manager.cpp:943-1026`): the base map times
        // 10,000 plus the first free instance.
        let mut dungeons = Dungeons::default();
        assert_eq!(dungeons.create(210, hosted, &[]), Some(2_100_000));
        assert_eq!(dungeons.create(210, hosted, &[]), Some(2_100_001));
        assert!(dungeons.lives(2_100_000));
        assert!(dungeons.lives(2_100_001));
        assert!(!dungeons.lives(2_100_002));
    }

    #[test]
    fn a_create_refuses_a_bad_map_and_a_full_set() {
        let mut dungeons = Dungeons::default();
        assert_eq!(dungeons.create(0, hosted, &[]), None);
        assert_eq!(dungeons.create(10_000, hosted, &[]), None);
        assert_eq!(dungeons.create(-3, hosted, &[]), None);
        assert_eq!(dungeons.create(210, |_| false, &[]), None);
        let taken: Vec<i32> = (0..51).map(|offset| 1_070_000 + offset).collect();
        assert_eq!(dungeons.create(107, hosted, &taken), None);
    }

    #[test]
    fn a_destroy_takes_the_timers_keyed_by_the_index() {
        // `Destroy` (`G/dungeon.cpp:434-453`): the map goes with the timers whose
        // argument is its index.
        let mut dungeons = Dungeons::default();
        let index = dungeons.create(210, hosted, &[]).expect("an index");
        let key = u32::from_ne_bytes(index.to_ne_bytes());
        dungeons.add_timer(b"frt_exit_timer".to_vec(), key, 10);
        dungeons.add_timer(b"frt_exit_timer".to_vec(), 7, 10);
        assert!(dungeons.destroy(index));
        assert!(!dungeons.destroy(index));
        assert!(!dungeons.lives(key));
        assert_eq!(dungeons.timers_due(10), [(b"frt_exit_timer".to_vec(), 7)]);
    }

    #[test]
    fn the_flags_default_to_zero() {
        // `GetFlag` (`G/dungeon.cpp`): 0 with no dungeon or no such flag. The step
        // answers its snapshot, so the snapshot shows the set and nothing else.
        let mut dungeons = Dungeons::default();
        assert!(dungeons.flags_snapshot().is_empty());
        let index = dungeons.create(210, hosted, &[]).expect("an index");
        assert!(dungeons.flags_snapshot()[&index].is_empty());
        dungeons.set_flag(index, b"floor".to_vec(), 3);
        assert_eq!(
            dungeons.flags_snapshot()[&index].get(b"floor".as_slice()),
            Some(&3)
        );
        dungeons.set_flag(9, b"floor".to_vec(), 5);
        assert!(!dungeons.flags_snapshot().contains_key(&9));
    }

    #[test]
    fn the_dead_event_fires_300_seconds_after_the_last_leave() {
        // `DecMember` (`G/dungeon.cpp:221-239`) arms it, `IncMember` (`:213-219`)
        // cancels it.
        let mut dungeons = Dungeons::default();
        let index = dungeons.create(210, hosted, &[]).expect("an index");
        dungeons.last_member_left(index, 100);
        assert!(dungeons.dead_due(100 + DEAD_EVENT_PULSES - 1).is_empty());
        assert_eq!(dungeons.dead_due(100 + DEAD_EVENT_PULSES), [index]);
        dungeons.member_entered(index);
        assert!(dungeons.dead_due(u64::MAX).is_empty());
    }

    #[test]
    fn a_timer_fires_once_and_a_second_add_is_quiet() {
        // `AddServerTimer` (`G/questmanager.cpp:1967-1976`) refuses a duplicate,
        // and the one-shot forgets itself when it fires (`G/questevent.cpp:38-43`).
        let mut dungeons = Dungeons::default();
        dungeons.add_timer(b"t".to_vec(), 1, 10);
        dungeons.add_timer(b"t".to_vec(), 1, 5);
        dungeons.clear_timer(b"nope", 1);
        assert!(dungeons.timers_due(9).is_empty());
        assert_eq!(dungeons.timers_due(10), [(b"t".to_vec(), 1)]);
        assert!(dungeons.timers_due(10).is_empty());
        dungeons.add_timer(b"u".to_vec(), 2, 20);
        dungeons.clear_timer(b"u", 2);
        assert!(dungeons.timers_due(20).is_empty());
    }
}
