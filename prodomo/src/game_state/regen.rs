//! The regen event, the ore vein's end and a non-player's `Destroy`: `regen_event`
//! (`G/regen.cpp:582-599`), `kill_ore_load_event` (`G/char.cpp:2447-2462`) and
//! `CHARACTER::Destroy` (`G/char.cpp:572-790`) for a character with no desc.
//!
//! # What legacy does
//!
//! `regen_load` gives each entry with a regen time a `regen_event`, first due `number(0, 16)`
//! plus its time seconds after boot (`G/regen.cpp:679-689`). Each time it runs, the event calls
//! `regen_spawn(regen, false)`, which spawns `max_count - count` more units as boot did, each
//! holding the regen, and asks to run again `time` seconds on. Without a death the count stays
//! at what stood, so the event spawns only what failed to stand before: a box whose sixteen
//! points were all refused, a group whose leader did not stand, an anywhere spawn whose 2000
//! cells were.
//!
//! `SetProto` gives an ore vein a `kill_ore_load_event` of `number(420, 900)` seconds
//! (`G/char.cpp:2563-2570`), which destroys it. Its regen then spawns another at its next run.
//!
//! `M2_DESTROY_CHARACTER` of a non-player runs `CHARACTER::Destroy`:
//!
//! 1. the regen it holds counts one fewer (`:572-583`);
//! 2. `ClearSync` tells every player that sees it that it has no owner, a `GC_OWNERSHIP` with
//!    owner 0, unless `AIFLAG_NOMOVE` holds it, as it holds an ore vein (`:653`,
//!    `G/char.cpp:5469-5575`);
//! 3. `ClearTarget` sends each character targeting it VID 0 (`:654`);
//! 4. its party: a group's master deletes the party, so its members follow no one; a member
//!    quits it (`:661-676`);
//! 5. its events end: the ore vein's, and the stun, dead and recovery events of a fight
//!    (`:713-741`);
//! 6. `CEntity::Destroy` sends every player that sees it its removal, and the sectree lets it
//!    go (`:786-789`).
//!
//! The fight's events ride the same schedule (see [`super::fight`]).
//!
//! # Divergences
//!
//! - **The order of one Pulse's events.** Legacy runs the events due on one Pulse in its event
//!   heap's order. The Rewrite runs them by due Pulse, then an ore vein's end before a regen
//!   before a stun before a death before a recovery, then by VID or by map and entry (V16,
//!   after V1).
//! - **The list a map keeps.** Legacy keeps its characters by VID in `m_map_pkChrByVID`; the
//!   Rewrite keeps each map's in a list, and a destroyed character's place is taken by the last
//!   one, which only the Rewrite's own index sees (V16).
//!
//! # Not ported
//!
//! - The dungeon's regen (`dungeon_regen_event`), its `IsValidRegen` check and `DeadCharacter`:
//!   there are no dungeons yet.
//! - Every other part of `Destroy`: a non-player has no shop, exchange, safebox, horse, pet,
//!   affect or skill event yet, and no stone or victim.
//! - Every other way a non-player ends but its ore vein's end and a monster's death: the
//!   quests', the dungeons' and the events' purges wait for their systems.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use common::vid::Vid;
use gamedata::map_atlas::MapRegion;
use gamedata::server_attr::ATTR_BANPK;
use world::cells::{HostedCells, MapCells};
use world::npc::{MapNpcs, Npc, Respawner};

use tracing::warn;

use super::view::EntityKey;
use super::{warp_npc, GameState};
use crate::save::PASSES_PER_SEC;

/// An event due on a Pulse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Due {
    /// `kill_ore_load_event` of the ore vein under this VID.
    Vein(u32),
    /// `kill_campfire_event` of the campfire under this VID (`G/char_item.cpp:2047-2066`).
    /// It sorts after [`Due::Vein`]: at an equal Pulse a vein's end runs first (a
    /// Divergence — legacy's heap order at equal priority is heap-shape dependent, as
    /// the other arms note).
    Campfire(u32),
    /// `regen_event` of the regen at this place in its map's list.
    Regen {
        /// The Channel.
        channel: u8,
        /// The map index.
        map: i32,
        /// Its place in the map's [`world::npc::MapNpcs::regens`].
        at: usize,
    },
    /// `battle_arena_event` (`G/BattleArena.cpp:87-233`): the weekly event's state.
    /// It sorts after [`Due::SantaSpawn`]: at an equal Pulse Santa's return runs first
    /// (a Divergence — legacy's heap order at equal priority is heap-shape dependent,
    /// as the other arms note).
    BattleArena,
    /// The world-boss timers (`worldboss_cooldown_event`, `worldboss_combat_event`,
    /// `worldboss_spawn_event`, `G/worldboss.cpp:46-130`): the run's own arm tells
    /// which one fires. It sorts after [`Due::BattleArena`]: at an equal Pulse the
    /// arena runs first (a Divergence — legacy's heap order at equal priority is
    /// heap-shape dependent, as the other arms note).
    WorldBoss,
    /// `spawn_santa_event` (`G/xmas_event.cpp:115-141`): Santa's 50 min respawn on this map.
    /// It sorts first here: at an equal Pulse it fires before any other event (a Divergence
    /// — legacy's heap order at equal priority is heap-shape dependent, as the other arms
    /// note).
    SantaSpawn {
        /// The map index Santa returns to.
        map: i32,
    },
    /// `StunEvent` of the non-player under this VID. See [`super::fight`].
    Stun(u32),
    /// `dead_event` of the non-player under this VID.
    Dead(u32),
    /// `recovery_event` of the non-player under this VID.
    /// `recovery_event` of the non-player under this VID.
    Recovery(u32),
    /// `mob_skill_hit_event` of the non-player under this VID at this map key (the
    /// splash vec index). It sorts after [`Due::Recovery`]: at an equal Pulse the
    /// skill fires after the recovery event (a Divergence — legacy's heap order at
    /// equal priority is heap-shape dependent).
    MobSkill {
        /// The owner's VID.
        vid: u32,
        /// The map key.
        key: u32,
    },
    /// `poison_event` of the non-player under this VID, with the ticks it has left. It
    /// sorts after [`Due::MobSkill`]: at an equal Pulse the tick runs after the skill
    /// event (a Divergence — legacy's heap order at equal priority is heap-shape
    /// dependent).
    Poison {
        /// The victim's VID.
        vid: u32,
        /// The ticks left, counting the one this run fires.
        count: u32,
    },
    /// `fire_event` of the non-player under this VID, with the ticks it has left and the
    /// per-tick amount. It sorts after [`Due::Poison`]: at an equal Pulse the tick runs
    /// after the poison event (a Divergence — legacy's heap order at equal priority is
    /// heap-shape dependent).
    Fire {
        /// The victim's VID.
        vid: u32,
        /// The ticks left, counting the one this run fires.
        count: u32,
        /// The per-tick amount.
        amount: i32,
    },
    /// `StunEvent` of the player under this VID. See [`super::death`].
    PcStun(u32),
    /// `dead_event` of the player under this VID.
    PcDead(u32),
    /// `recovery_event` of the player under this VID.
    PcRecovery(u32),
    /// `poison_event` of the player under this VID, with the ticks it has left. It sorts
    /// after [`Due::PcRecovery`]: at an equal Pulse the tick runs after the recovery
    /// event (a Divergence — legacy's heap order at equal priority is heap-shape
    /// dependent).
    PcPoison {
        /// The victim's VID.
        vid: u32,
        /// The ticks left, counting the one this run fires.
        count: u32,
        /// The killer the poison carries (`info->attacker_pid`, `G/char_resist.cpp:211`),
        /// 0 for a monster.
        killer: u32,
    },
}

/// The regen events and the ore veins' ends, with what a regen spawns from.
#[derive(Debug, Default)]
pub(super) struct Regens {
    /// The spawner's tables and the VIDs boot left, once boot hands them over.
    respawner: Option<Respawner>,
    /// The cells of every hosted map, which a spawn checks its point against.
    cells: Arc<HostedCells>,
    /// The region of each map boot stood characters up on, keyed as the maps are.
    regions: BTreeMap<(u8, i32), MapRegion>,
    /// The events, by the Pulse each is due on.
    pub(super) due: BTreeSet<(u64, Due)>,
}

impl Regens {
    /// The events due on or before `pulse`, taken off the schedule, in order.
    fn take_due(&mut self, pulse: u64) -> Vec<Due> {
        let mut due = Vec::new();
        while self.due.first().is_some_and(|&(when, _)| when <= pulse) {
            if let Some((_, what)) = self.due.pop_first() {
                due.push(what);
            }
        }
        due
    }

    /// `event_create`: `what` is due on `when`, which `slot` keeps for its cancel.
    pub(super) fn schedule(&mut self, slot: &mut Option<u64>, when: u64, what: Due) {
        *slot = Some(when);
        self.due.insert((when, what));
    }

    /// `event_cancel`: the event whose due Pulse `slot` keeps is taken off the schedule.
    pub(super) fn cancel(&mut self, slot: &mut Option<u64>, what: Due) {
        if let Some(when) = slot.take() {
            self.due.remove(&(when, what));
        }
    }

    /// Whether `what` is on the schedule, due on any Pulse.
    #[cfg(test)]
    pub(super) fn holds(&self, what: Due) -> bool {
        self.due.iter().any(|&(_, due)| due == what)
    }

    /// The cells of the map `index`: the sectrees it builds and the attributes its
    /// file gave them.
    ///
    /// A private map has no cells of its own: it is a copy of the map it was copied
    /// from (`SECTREE_MANAGER::CreatePrivateMap`, `G/sectree_manager.cpp:943-1026`), so
    /// a character on it is checked against the base map's cells.
    pub(super) fn cells_of(&self, index: i32) -> Option<MapCells> {
        self.cells.get(index).cloned()
    }

    /// The region of the map boot stood up on `channel`: its edges and its base
    /// point (`iBaseX`/`iBaseY`), which the `/state` coordinate line reads.
    pub(super) fn region_of(&self, channel: u8, map: i32) -> Option<MapRegion> {
        self.regions.get(&(channel, map)).cloned()
    }

    /// The spawner the regens spawn from, once boot has handed it over.
    pub(super) fn respawner_mut(&mut self) -> Option<&mut Respawner> {
        self.respawner.as_mut()
    }

    /// The spawner's mob prototypes, once boot has handed them over.
    pub(super) fn respawner(&self) -> Option<&Respawner> {
        self.respawner.as_ref()
    }
}

/// `PASSES_PER_SEC(seconds)`.
pub(super) fn pulses(seconds: u32) -> u64 {
    u64::from(seconds).saturating_mul(u64::from(PASSES_PER_SEC))
}

impl GameState {
    /// Hand the world what a regen spawns from once boot is over: the spawner's tables and the
    /// VIDs boot left, and the hosted maps' cells. Until then a regen event that runs spawns
    /// nothing.
    pub fn install_respawner(&mut self, respawner: Respawner, cells: Arc<HostedCells>) {
        self.regens.respawner = Some(respawner);
        self.regens.cells = cells;
    }

    /// `SECTREE_MANAGER::IsMovablePosition` (`G/sectree_manager.cpp:779-787`) on the cells boot
    /// handed over: false on a map it handed none of.
    pub(super) fn is_movable(&self, map: i32, x: i32, y: i32) -> bool {
        self.regens
            .cells
            .get(map)
            .is_some_and(|cells| cells.is_movable(x, y))
    }

    /// `SECTREE_MANAGER::GetMovablePosition` (`G/sectree_manager.cpp:789-813`) on the cells boot
    /// handed over: `None` on a map it handed none of, or where none of the 161 is movable.
    pub(super) fn movable_position(&self, map: i32, x: i32, y: i32) -> Option<(i32, i32)> {
        self.regens.cells.get(map)?.movable_position(x, y)
    }

    /// `SECTREE::IsAttr(x, y, ATTR_BANPK)` on the cells boot handed over: false on a map it
    /// handed none of.
    pub(super) fn is_banpk(&self, map: i32, x: i32, y: i32) -> bool {
        self.is_attr(map, (x, y), ATTR_BANPK)
    }

    /// Whether any cell in the rect sets any of `flags`, walking the cells the rect
    /// covers (`ForAttrRegion` in `ATTR_REGION_MODE_CHECK`, `G/sectree_manager.cpp`),
    /// on the cells boot handed over: false on a map it handed none of.
    pub(super) fn attr_in_rect(
        &self,
        map: i32,
        footprint: (i32, i32, i32, i32),
        flags: u32,
    ) -> bool {
        use gamedata::server_attr::CELL_SIZE;
        let Some(cells) = self.regens.cells.get(map) else {
            return false;
        };
        let mut y = footprint.1 - footprint.1 % CELL_SIZE;
        while y <= footprint.3 {
            let mut x = footprint.0 - footprint.0 % CELL_SIZE;
            while x <= footprint.2 {
                if cells.is_attr(x, y, flags) {
                    return true;
                }
                x = x.saturating_add(CELL_SIZE);
            }
            y = y.saturating_add(CELL_SIZE);
        }
        false
    }

    /// `SECTREE::IsAttr(x, y, flags)` on the cells boot handed over: false on a map it handed
    /// none of, and where the map has no sectree.
    pub(super) fn is_attr(&self, map: i32, (x, y): (i32, i32), flags: u32) -> bool {
        self.regens
            .cells
            .get(map)
            .is_some_and(|cells| cells.is_attr(x, y, flags))
    }

    /// Schedules the events of a map boot stood up on `pulse`: each regen's first run and each
    /// ore vein's end. The events of the characters it replaced are dropped.
    pub(super) fn schedule_map(&mut self, (channel, map): (u8, i32), region: &MapRegion) {
        let pulse = self.last_pulse;
        self.regens.regions.insert((channel, map), region.clone());
        self.regens.due.retain(|&(_, what)| {
            !matches!(what, Due::Regen { channel: c, map: m, .. } if (c, m) == (channel, map))
        });
        let Some(list) = self.npcs.get(&(channel, map)) else {
            return;
        };
        for (at, regen) in list.regens.iter().enumerate() {
            let when = pulse.saturating_add(pulses(regen.first_fire));
            self.regens
                .due
                .insert((when, Due::Regen { channel, map, at }));
        }
        let veins: Vec<(u32, u32)> = list
            .npcs
            .iter()
            .filter_map(|npc| npc.ore_lifetime.map(|lifetime| (npc.vid, lifetime)))
            .collect();
        for (vid, lifetime) in veins {
            self.schedule_vein(vid, lifetime, pulse);
        }
    }

    /// `kill_ore_load_event`, `lifetime` seconds after `pulse`.
    fn schedule_vein(&mut self, vid: u32, lifetime: u32, pulse: u64) {
        let when = pulse.saturating_add(pulses(lifetime));
        self.regens.due.insert((when, Due::Vein(vid)));
    }

    /// Runs every event due on or before `pulse`: the regens, the ore veins' ends and the stun,
    /// dead and recovery events of the non-players and the players.
    pub(super) fn run_regens(&mut self, pulse: u64) {
        for what in self.regens.take_due(pulse) {
            match what {
                // The VID of a non-player already destroyed names nothing, as no such VID is
                // reused. A player's VID is its PID, which comes back at its next login, so a
                // player's event runs only while its own slot still holds it.
                Due::Vein(vid) | Due::Campfire(vid) => self.destroy_npc(vid),
                Due::SantaSpawn { map } => self.fire_santa_spawn(map, pulse),
                Due::BattleArena => {}
                Due::WorldBoss => self.run_world_boss_tick(pulse),
                Due::Regen { channel, map, at } => self.fire_regen((channel, map), at, pulse),
                Due::Stun(vid) => self.stun_event(vid),
                Due::Dead(vid) => self.dead_event(vid),
                Due::Recovery(vid) => self.recovery_event(vid),
                Due::MobSkill { vid, key } => self.mob_skill_event(vid, key),
                Due::Poison { vid, count } => self.npc_poison_event(vid, count),
                Due::Fire { vid, count, amount } => self.npc_fire_event(vid, count, amount),
                Due::PcStun(vid) => self.pc_stun_event(vid),
                Due::PcDead(vid) => self.pc_dead_event(vid),
                Due::PcRecovery(vid) => self.pc_recovery_event(vid),
                Due::PcPoison { vid, count, killer } => self.pc_poison_event(vid, count, killer),
            }
        }
    }

    /// `xmas::SpawnSanta` (`G/xmas_event.cpp:143-156`): Santa returns to `map` after
    /// `gap` seconds (50 min from the hop or the GG relay), 50 s on the test server
    /// (`test_server` divides the gap by 60, and the world is one).
    pub(super) fn spawn_santa(&mut self, map: i32, gap: i32, pulse: u64) {
        let gap = gap / if self.test_server { 60 } else { 1 };
        let gap = u32::try_from(gap).unwrap_or(0);
        let when = pulse.saturating_add(pulses(gap));
        self.regens.due.insert((when, Due::SantaSpawn { map }));
    }

    /// `spawn_santa_event` (`G/xmas_event.cpp:115-141`): silent when `xmas_santa` is 0 or a
    /// Santa stands anywhere (any Channel world — each legacy core kept its own map and
    /// the check was process-wide); else Santa returns to the map on some Channel world
    /// hosting it, first one in map order. A map none hosts re-arms 5 s on, as legacy's
    /// failed spawn does.
    fn fire_santa_spawn(&mut self, map: i32, pulse: u64) {
        let flag = self
            .quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(b"xmas_santa"));
        if flag == 0 || !self.vids_of_race(None, super::event_flag::SANTA).is_empty() {
            return;
        }
        let Some(&place) = self.places_hosting(map).iter().min().copied().as_ref() else {
            let when = pulse.saturating_add(pulses(5));
            self.regens.due.insert((when, Due::SantaSpawn { map }));
            return;
        };
        self.spawn_event_mob_randomly(place, super::event_flag::SANTA);
    }

    /// `regen_event`: `regen_spawn(regen, false)`, then the next run `time` seconds on.
    fn fire_regen(&mut self, key: (u8, i32), at: usize, pulse: u64) {
        let Some(regen) = self
            .npcs
            .get(&key)
            .and_then(|list| list.regens.get(at))
            .copied()
        else {
            return;
        };
        let next = pulse.saturating_add(pulses(regen.entry.time));
        let (channel, map) = key;
        self.regens
            .due
            .insert((next, Due::Regen { channel, map, at }));
        if regen.missing() == 0 {
            return;
        }
        let (Some(respawner), Some(region), Some(cells)) = (
            self.regens.respawner.as_mut(),
            self.regens.regions.get(&key),
            self.regens.cells.get(map),
        ) else {
            return;
        };
        match respawner.respawn((region, cells), &regen, at, &mut self.dice) {
            Ok((stood, count)) => self.stand_more(key, at, (stood, count), pulse),
            Err(error) => warn!(channel, map, at, %error, "a regen could not spawn"),
        }
    }

    /// Stands the characters a regen spawned on its map after the others: each enters the
    /// map's view, which sends its insert to every player that now sees it; an ore vein's end
    /// is scheduled; a warp or goto NPC starts its event; and the regen counts the units.
    fn stand_more(
        &mut self,
        key: (u8, i32),
        at: usize,
        (stood, count): (Vec<Npc>, u32),
        pulse: u64,
    ) {
        let Some(list) = self.npcs.get_mut(&key) else {
            return;
        };
        let list = Arc::make_mut(list);
        if let Some(regen) = list.regens.get_mut(at) {
            regen.count = regen.count.saturating_add(count);
        }
        self.stand_runtime(key, stood, pulse);
    }

    /// Stands the sons a stone's `SpawnGroup` spawned on its map, after the others.
    /// No regen counts them: the stone holds none (`pkRegen` never reaches them).
    fn stand_stone_group(&mut self, key: (u8, i32), stood: Vec<Npc>, pulse: u64) {
        if stood.is_empty() {
            return;
        }
        self.stand_runtime(key, stood, pulse);
    }

    /// One `SpawnGroup` of the stone under `stone` in the box of half-`width` around
    /// `(x, y)`, stood on its map (`G/char_state.cpp:406-410`). Empty when the group is 0
    /// or unknown, or the map has no respawner data.
    pub(super) fn spawn_stone_group(
        &mut self,
        key: (u8, i32),
        group: u32,
        stone: u32,
        (x, y): (i32, i32),
        width: i32,
    ) {
        let (Some(respawner), Some(region), Some(cells)) = (
            self.regens.respawner.as_mut(),
            self.regens.regions.get(&key),
            self.regens.cells.get(key.1),
        ) else {
            return;
        };
        let area = (
            x.saturating_sub(width),
            y.saturating_sub(width),
            x.saturating_add(width),
            y.saturating_add(width),
        );
        let stood = respawner
            .spawn_stone_group((region, cells), group, stone, area, &mut self.dice)
            .unwrap_or_default();
        let pulse = self.last_pulse;
        self.stand_stone_group(key, stood, pulse);
    }

    /// Stands runtime-spawned characters on their map after the others: each enters the
    /// map's view, which sends its insert to every player that now sees it; an ore vein's
    /// end is scheduled; and a warp or goto NPC starts its event.
    ///
    /// A map with no regen characters holds none yet, so its list is made here: a war
    /// map is a private copy of a map, and it holds only what the war stands up.
    pub(super) fn stand_runtime(&mut self, key: (u8, i32), stood: Vec<Npc>, pulse: u64) {
        let list = self
            .npcs
            .entry(key)
            .or_insert_with(|| Arc::new(MapNpcs::default()));
        let list = Arc::make_mut(list);
        let first = list.npcs.len();
        list.npcs.extend(stood);
        let now = self.clock.now();
        let mut effects = Vec::new();
        let mut veins = Vec::new();
        for (offset, npc) in list.npcs[first..].iter().enumerate() {
            self.npc_of
                .insert(npc.vid, (key.0, key.1, first.saturating_add(offset)));
            self.npc_combat.insert(
                npc.vid,
                super::fight::NpcCombat::created(now, (npc.x, npc.y)),
            );
            if let Some(index) = self.maps.get_mut(&key) {
                // `SpawnMob` places a character only where a sectree stands.
                let _shown = index.show(
                    EntityKey::Npc(npc.vid),
                    (npc.x, npc.y, npc.z),
                    self.view_radius,
                    &mut effects,
                );
            }
            if let Some(lifetime) = npc.ore_lifetime {
                veins.push((npc.vid, lifetime));
            }
        }
        if let Some(region) = self.regens.regions.get(&key) {
            let warps = warp_npc::warp_npcs(&list.npcs[first..], region);
            self.warp_npcs.entry(key).or_default().extend(warps);
        }
        for (vid, lifetime) in veins {
            self.schedule_vein(vid, lifetime, pulse);
        }
        self.deliver(&effects, None);
    }

    /// `ACMD(do_purge)` and `ACMD(do_weaken)` (`G/cmd_gm.cpp:816-868`,
    /// `:2824-2861`): every NPC of the sectrees around the caller is destroyed
    /// (`purge`) or set to 10 hit points (`weaken`), `all` or within
    /// `DISTANCE_APPROX` 1000. An NPC is `char_type != CHAR_TYPE_PC`; `purge`
    /// spares a summoned horse that has its rider (`GetRider() == NULL`), a mount
    /// follower (`IsMount`) and a pet (`IsPet`); the auto-shaman gate is dead (no
    /// ported path sets it).
    /// With no sectree legacy logs and destroys nothing (`:857-861`).
    /// `weaken`'s set is `PointChange(POINT_HP)`: dead or stunned refuses,
    /// the change caps at the maximum, and the targeters hear it.
    pub(super) fn purge_around(&mut self, vid: Vid, all: bool, weaken: bool) {
        let Some(place) = self.place_of(vid.raw()) else {
            return;
        };
        let Some(spot) = self.spot_of(vid.raw()) else {
            return;
        };
        if spot.tree.is_none() {
            return;
        }
        let Some(index) = self.maps.get(&place) else {
            return;
        };
        let around = index.around_of(EntityKey::Character(vid.raw()));
        let mut swept: Vec<u32> = Vec::new();
        for key in around {
            let EntityKey::Npc(npc) = key else {
                continue;
            };
            let Some(found) = self.npc_by_vid(npc) else {
                continue;
            };
            if found.char_type == gamedata::mob_proto::CHAR_TYPE_PC {
                continue;
            };
            if !all
                && crate::sync_position::distance_approx(found.x - spot.x, found.y - spot.y) >= 1000
            {
                continue;
            }
            if !weaken && (self.has_rider(npc) || self.is_mount_npc(npc) || self.is_pet_npc(npc)) {
                continue;
            }
            swept.push(npc);
        }
        for npc in swept {
            if weaken {
                self.weaken_npc(npc);
            } else {
                self.destroy_npc(npc);
            }
        }
    }

    /// `regen_reset(x, y)` (`G/regen.cpp:737-757`): every regen with an event
    /// fires one Pulse on - those holding the point when one is named, every
    /// regen when it is not. The caller's own Channel is the walk, as
    /// `/user` walks it: legacy's list is one core's.
    ///
    /// A regen with no due event is skipped, as one with no `event` is. A
    /// regen whose rectangle holds the caller fires even when its map is not
    /// the caller's, because the legacy test is only the rectangle.
    pub(super) fn respawn_around(&mut self, vid: Vid, all: bool) {
        let when = self.last_pulse.saturating_add(1);
        if all {
            let due: Vec<(u64, Due)> = self
                .regens
                .due
                .iter()
                .copied()
                .filter(|&(_, what)| matches!(what, Due::Regen { .. }))
                .collect();
            for (old, what) in due {
                self.regens.due.remove(&(old, what));
                self.regens.due.insert((when, what));
            }
            return;
        }
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return;
        };
        let due: Vec<(u64, Due)> = self
            .regens
            .due
            .iter()
            .copied()
            .filter(|&(_, what)| match what {
                Due::Regen { channel, map, at } => {
                    channel == place.0
                        && self
                            .npcs
                            .get(&(channel, map))
                            .and_then(|list| list.regens.get(at))
                            .is_some_and(|regen| {
                                spot.x >= regen.entry.sx
                                    && spot.x <= regen.entry.ex
                                    && spot.y >= regen.entry.sy
                                    && spot.y <= regen.entry.ey
                            })
                }
                _ => false,
            })
            .collect();
        for (old, what) in due {
            self.regens.due.remove(&(old, what));
            self.regens.due.insert((when, what));
        }
    }

    /// `do_mob` (`G/cmd_gm.cpp:699-758`): `count` of `vnum` stand around the
    /// caller, each in a box `number(200, 750)` past the caller on every
    /// side, or the usage or the miss answers. The proto answers with the
    /// spawn motion when it is a stone; the count caps at 40 on the test
    /// server, 20 elsewhere.
    ///
    /// The vnum-or-name split is legacy's: a leading digit reads a vnum, any
    /// other first byte reads a locale-name prefix. A vnum with no row, or a
    /// prefix no locale name starts with, is the miss.
    pub(super) fn mob_around(&mut self, vid: Vid, rest: &[u8]) {
        self.mob_around_inner(vid, rest);
    }

    /// `do_group` (`G/cmd_gm.cpp:537-555`): the group in the 500-box around
    /// the caller, or the usage answers. The word is a bare `str_to_number`.
    /// A vnum with no group stands nothing and stays silent, as legacy's
    /// null return does.
    pub(super) fn group_around(&mut self, vid: Vid, rest: &[u8]) {
        use protocol::gc_chat::CHAT_TYPE_INFO;
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let (first, _) = crate::command::one_argument(rest);
        if first.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage: group <group vnum>",
                &[],
            );
            return;
        }
        let vnum = Self::mob_vnum(&first);
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return;
        };
        let pulse = self.last_pulse;
        let (Some(region), Some(cells), Some(respawner)) = (
            self.regens.regions.get(&place).cloned(),
            self.regens.cells.get(place.1).cloned(),
            self.regens.respawner.as_mut(),
        ) else {
            return;
        };
        let area = (
            spot.x.saturating_sub(500),
            spot.y.saturating_sub(500),
            spot.x.saturating_add(500),
            spot.y.saturating_add(500),
        );
        match respawner.spawn_group_line((&region, &cells), vnum, area, &mut self.dice) {
            Ok(stood) => self.stand_runtime(place, stood, pulse),
            Err(error) => {
                warn!(?place, %error, "a group line outnumbered the VIDs left");
            }
        }
    }

    /// `do_group_random` (`G/cmd_gm.cpp:521-536`): `/group`'s spawn with one
    /// group drawn uniformly from the group-group, or the usage answers. A
    /// vnum with no group-group stands nothing and stays silent.
    pub(super) fn group_random_around(&mut self, vid: Vid, rest: &[u8]) {
        use protocol::gc_chat::CHAT_TYPE_INFO;
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let (first, _) = crate::command::one_argument(rest);
        if first.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage: grrandom <group vnum>",
                &[],
            );
            return;
        }
        let vnum = Self::mob_vnum(&first);
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return;
        };
        let pulse = self.last_pulse;
        let (Some(region), Some(cells), Some(respawner)) = (
            self.regens.regions.get(&place).cloned(),
            self.regens.cells.get(place.1).cloned(),
            self.regens.respawner.as_mut(),
        ) else {
            return;
        };
        let area = (
            spot.x.saturating_sub(500),
            spot.y.saturating_sub(500),
            spot.x.saturating_add(500),
            spot.y.saturating_add(500),
        );
        match respawner.spawn_group_random_line((&region, &cells), vnum, area, &mut self.dice) {
            Ok(stood) => self.stand_runtime(place, stood, pulse),
            Err(error) => {
                warn!(?place, %error, "a grrandom line outnumbered the VIDs left");
            }
        }
    }

    /// `do_mob_map` (`G/cmd_gm.cpp:618-637`): the numbered mob at a random
    /// point of the caller's map, or the syntax, the stood line, or the
    /// failure answers. The word is a bare `str_to_number`: no name reads,
    /// so a name is 0 and fails. A vnum with no proto, a gated vnum, or 2000
    /// tries with no cell is `Failed`; the stood line names the mob's locale
    /// name and its centimetres (`%s spawned in %dx%d`).
    pub(super) fn mob_somewhere(&mut self, vid: Vid, rest: &[u8]) {
        use protocol::gc_chat::CHAT_TYPE_INFO;
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let (first, _) = crate::command::one_argument(rest);
        if first.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Syntax: mm <vnum>", &[]);
            return;
        }
        let vnum = Self::mob_vnum(&first);
        let Some(place) = self.place_of(vid.raw()) else {
            return;
        };
        let pulse = self.last_pulse;
        let (Some(region), Some(cells), Some(respawner)) = (
            self.regens.regions.get(&place).cloned(),
            self.regens.cells.get(place.1).cloned(),
            self.regens.respawner.as_mut(),
        ) else {
            return;
        };
        match respawner.spawn_random_line((&region, &cells), vnum, &mut self.dice) {
            Ok(Some(stood)) => {
                let name = String::from_utf8_lossy(&stood.name).into_owned();
                let line = format!("{name} spawned in {}x{}", stood.x, stood.y);
                self.stand_runtime(place, vec![stood], pulse);
                self.tell(vid, mover, CHAT_TYPE_INFO, line.as_bytes(), &[]);
            }
            Ok(None) => {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"Failed", &[]);
            }
            Err(error) => {
                warn!(?place, %error, "a mob_map line outnumbered the VIDs left");
            }
        }
    }

    /// The places of `map` this process hosts (`map_allow_find`), one per Channel world, in
    /// Channel order. Legacy runs an event arm on every core that hosts the map; a Channel
    /// world is one core's worth of maps here, so the arm runs once per place.
    pub(super) fn places_hosting(&self, map: i32) -> Vec<(u8, i32)> {
        self.regens
            .regions
            .keys()
            .filter(|&&(_, hosted)| hosted == map)
            .copied()
            .collect()
    }

    /// Whether the process hosts `channel` at all.
    pub(super) fn channel_hosts(&self, channel: u8) -> bool {
        self.regens
            .regions
            .keys()
            .any(|&(hosted, _)| hosted == channel)
    }

    /// `GetMapBasePositionByMapIndex`: the map's top-left corner in centimetres.
    pub(super) fn map_base(&self, place: (u8, i32)) -> Option<(i32, i32)> {
        self.regens
            .regions
            .get(&place)
            .map(|region| (region.sx, region.sy))
    }

    /// `SpawnMob(vnum, map, x, y, 0, spawn_motion, dir)` on `place` for an event arm: the
    /// point is absolute, `-1` draws the facing. Answers the stood VID, or nothing when
    /// a point no sectree holds, a cell that refuses, or a missing proto stands nothing,
    /// as legacy's null return does.
    pub(super) fn spawn_event_mob(
        &mut self,
        place: (u8, i32),
        vnum: u32,
        at: (i32, i32),
        dir: i32,
        spawn_motion: bool,
    ) -> Option<u32> {
        let pulse = self.last_pulse;
        let (Some(region), Some(cells), Some(respawner)) = (
            self.regens.regions.get(&place).cloned(),
            self.regens.cells.get(place.1).cloned(),
            self.regens.respawner.as_mut(),
        ) else {
            return None;
        };
        match respawner.spawn_directed(
            (&region, &cells),
            vnum,
            at,
            dir,
            spawn_motion,
            &mut self.dice,
        ) {
            Ok(Some(stood)) => {
                let vid = stood.vid;
                self.stand_runtime(place, vec![stood], pulse);
                Some(vid)
            }
            Ok(None) => None,
            Err(error) => {
                warn!(?place, %error, "an event spawn outnumbered the VIDs left");
                None
            }
        }
    }

    /// `SpawnMob(race, map, x, y, z, false, dir, false)` on `place` for the summoned horse:
    /// the NPC, not yet shown.
    pub(super) fn spawn_horse(
        &mut self,
        place: (u8, i32),
        race: u32,
        at: (i32, i32),
        dir: i32,
    ) -> Option<world::npc::Npc> {
        let (Some(region), Some(cells), Some(respawner)) = (
            self.regens.regions.get(&place).cloned(),
            self.regens.cells.get(place.1).cloned(),
            self.regens.respawner.as_mut(),
        ) else {
            return None;
        };
        match respawner.spawn_directed((&region, &cells), race, at, dir, false, &mut self.dice) {
            Ok(stood) => stood,
            Err(error) => {
                tracing::warn!(?place, %error, "a horse outnumbered the VIDs left");
                None
            }
        }
    }

    /// `SpawnMobRandomPosition(vnum, map)` on `place` for an event arm.
    pub(super) fn spawn_event_mob_randomly(&mut self, place: (u8, i32), vnum: u32) {
        let pulse = self.last_pulse;
        let (Some(region), Some(cells), Some(respawner)) = (
            self.regens.regions.get(&place).cloned(),
            self.regens.cells.get(place.1).cloned(),
            self.regens.respawner.as_mut(),
        ) else {
            return;
        };
        match respawner.spawn_random_line((&region, &cells), vnum, &mut self.dice) {
            Ok(Some(stood)) => self.stand_runtime(place, vec![stood], pulse),
            Ok(None) => {}
            Err(error) => warn!(?place, %error, "an event spawn outnumbered the VIDs left"),
        }
    }
    /// `do_mob_ld` (`G/cmd_gm.cpp:760-815`): the named mob at the named
    /// metres on the caller's map, facing `dir`, or the usage or the miss
    /// answers. The parse is `/mob`'s: a leading digit reads a vnum, any
    /// other first byte a locale-name prefix. The metres scale to
    /// centimetres; the missing words are 0, 0 and 1. A point no sectree
    /// holds, or a cell that refuses, stands nothing and stays silent, as
    /// legacy's `SpawnMob` returning null does.
    pub(super) fn mob_ld_at(&mut self, vid: Vid, rest: &[u8]) {
        use protocol::gc_chat::CHAT_TYPE_INFO;
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let (first, back) = crate::command::one_argument(rest);
        let (second, back) = crate::command::one_argument(back);
        let (third, back) = crate::command::one_argument(back);
        let (fourth, _) = crate::command::one_argument(back);
        if first.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Usage: mob <mob vnum>", &[]);
            return;
        }
        let Some(respawner) = self.regens.respawner.as_ref() else {
            return;
        };
        let (vnum, stone) = if first[0].is_ascii_digit() {
            let vnum = Self::mob_vnum(&first);
            let Some(row) = respawner.mob(vnum) else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"No such mob(%s) by that vnum",
                    &[crate::chat_line::Arg::Text(&first)],
                );
                return;
            };
            (vnum, row.mob_type == gamedata::mob_proto::CHAR_TYPE_STONE)
        } else {
            let Some(row) = respawner.mob_by_name(&first) else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"No such mob(%s) by that vnum",
                    &[crate::chat_line::Arg::Text(&first)],
                );
                return;
            };
            (
                row.vnum,
                row.mob_type == gamedata::mob_proto::CHAR_TYPE_STONE,
            )
        };
        let x = if second.is_empty() {
            0
        } else {
            Self::mob_count(&second).saturating_mul(100)
        };
        let y = if third.is_empty() {
            0
        } else {
            Self::mob_count(&third).saturating_mul(100)
        };
        let dir = if fourth.is_empty() {
            1
        } else {
            Self::mob_count(&fourth)
        };
        let Some(place) = self.place_of(vid.raw()) else {
            return;
        };
        let pulse = self.last_pulse;
        let (Some(region), Some(cells), Some(respawner)) = (
            self.regens.regions.get(&place).cloned(),
            self.regens.cells.get(place.1).cloned(),
            self.regens.respawner.as_mut(),
        ) else {
            return;
        };
        match respawner.spawn_directed((&region, &cells), vnum, (x, y), dir, stone, &mut self.dice)
        {
            Ok(Some(stood)) => self.stand_runtime(place, vec![stood], pulse),
            Ok(None) => {}
            Err(error) => {
                warn!(?place, %error, "a mob_ld line outnumbered the VIDs left");
            }
        }
    }

    /// `str_to_number` on a `DWORD` (`common/utils.h:44-50`): `strtoul` base 10
    /// cast to `u32`, garbage answering 0. The wrap matches `parse_war_type`'s.
    fn mob_vnum(word: &[u8]) -> u32 {
        let Some(offset) = word.iter().position(|byte| !byte.is_ascii_whitespace()) else {
            return 0;
        };
        let rest = &word[offset..];
        let (digits, negative) = match rest.first() {
            Some(b'-') => (&rest[1..], true),
            Some(b'+') => (&rest[1..], false),
            _ => (rest, false),
        };
        let mut value: u32 = 0;
        for byte in digits {
            if !byte.is_ascii_digit() {
                break;
            }
            value = value.wrapping_mul(10).wrapping_add(u32::from(byte - b'0'));
        }
        if negative {
            value.wrapping_neg()
        } else {
            value
        }
    }

    /// `str_to_number` on an `int` (`common/utils.h:38-43`): `strtol` base 10
    /// cast to `i32`, garbage answering 0.
    fn mob_count(word: &[u8]) -> i32 {
        let Some(offset) = word.iter().position(|byte| !byte.is_ascii_whitespace()) else {
            return 0;
        };
        let rest = &word[offset..];
        let (digits, negative) = match rest.first() {
            Some(b'-') => (&rest[1..], true),
            Some(b'+') => (&rest[1..], false),
            _ => (rest, false),
        };
        let mut value: i32 = 0;
        for byte in digits {
            if !byte.is_ascii_digit() {
                break;
            }
            value = value.wrapping_mul(10).wrapping_add(i32::from(byte - b'0'));
        }
        if negative {
            value.wrapping_neg()
        } else {
            value
        }
    }

    fn mob_around_inner(&mut self, vid: Vid, rest: &[u8]) {
        use protocol::gc_chat::CHAT_TYPE_INFO;
        use world::character::number;
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let (first, back) = crate::command::one_argument(rest);
        let (second, _) = crate::command::one_argument(back);
        if first.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Usage: mob <mob vnum>", &[]);
            return;
        }
        let Some(respawner) = self.regens.respawner.as_ref() else {
            return;
        };
        let (vnum, stone) = if first[0].is_ascii_digit() {
            let vnum = Self::mob_vnum(&first);
            let Some(row) = respawner.mob(vnum) else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"No such mob(%s) by that vnum",
                    &[crate::chat_line::Arg::Text(&first)],
                );
                return;
            };
            (vnum, row.mob_type == gamedata::mob_proto::CHAR_TYPE_STONE)
        } else {
            let Some(row) = respawner.mob_by_name(&first) else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"No such mob(%s) by that vnum",
                    &[crate::chat_line::Arg::Text(&first)],
                );
                return;
            };
            (
                row.vnum,
                row.mob_type == gamedata::mob_proto::CHAR_TYPE_STONE,
            )
        };
        let mut count = if second.is_empty() {
            1
        } else {
            Self::mob_count(&second)
        };
        count = count.min(if self.test_server { 40 } else { 20 });
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return;
        };
        let pulse = self.last_pulse;
        while count > 0 {
            count -= 1;
            let (low_x, low_y) = (
                spot.x.saturating_sub(number(&mut self.dice, 200, 750)),
                spot.y.saturating_sub(number(&mut self.dice, 200, 750)),
            );
            let (high_x, high_y) = (
                spot.x.saturating_add(number(&mut self.dice, 200, 750)),
                spot.y.saturating_add(number(&mut self.dice, 200, 750)),
            );
            let (Some(region), Some(cells), Some(respawner)) = (
                self.regens.regions.get(&place).cloned(),
                self.regens.cells.get(place.1).cloned(),
                self.regens.respawner.as_mut(),
            ) else {
                return;
            };
            match respawner.spawn_mob_line(
                (&region, &cells),
                vnum,
                (low_x, low_y, high_x, high_y),
                stone,
                &mut self.dice,
            ) {
                Ok(Some(stood)) => self.stand_runtime(place, vec![stood], pulse),
                Ok(None) => {}
                Err(error) => {
                    warn!(?place, %error, "a mob line outnumbered the VIDs left");
                    return;
                }
            }
        }
    }

    /// `do_mob_aggresive` and `do_mob_coward` (`G/cmd_gm.cpp:557-617`,
    /// `:639-698`): `/mob`'s spawn with the aggressive or the coward bit set
    /// on each (`SetAggressive`, `SetCoward`, `G/char_state.cpp:242-255`).
    /// The counts cap at 20 on either server, with no test-server 40: the
    /// lines never learned `/mob`'s branch. The usages differ (`Usage: mob
    /// <mob vnum>` for the aggressive, `Usage: mc <vnum>` for the coward);
    /// the miss is `/mob`'s.
    pub(super) fn mob_flagged(&mut self, vid: Vid, rest: &[u8], coward: bool) {
        use protocol::gc_chat::CHAT_TYPE_INFO;
        use world::character::number;
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let (first, back) = crate::command::one_argument(rest);
        let (second, _) = crate::command::one_argument(back);
        if first.is_empty() {
            let usage = if coward {
                b"Usage: mc <vnum>".as_slice()
            } else {
                b"Usage: mob <mob vnum>".as_slice()
            };
            self.tell(vid, mover, CHAT_TYPE_INFO, usage, &[]);
            return;
        }
        let Some(respawner) = self.regens.respawner.as_ref() else {
            return;
        };
        let (vnum, stone) = if first[0].is_ascii_digit() {
            let vnum = Self::mob_vnum(&first);
            let Some(row) = respawner.mob(vnum) else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"No such mob(%s) by that vnum",
                    &[crate::chat_line::Arg::Text(&first)],
                );
                return;
            };
            (vnum, row.mob_type == gamedata::mob_proto::CHAR_TYPE_STONE)
        } else {
            let Some(row) = respawner.mob_by_name(&first) else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"No such mob(%s) by that vnum",
                    &[crate::chat_line::Arg::Text(&first)],
                );
                return;
            };
            (
                row.vnum,
                row.mob_type == gamedata::mob_proto::CHAR_TYPE_STONE,
            )
        };
        let mut count = if second.is_empty() {
            1
        } else {
            Self::mob_count(&second)
        };
        count = count.min(20);
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return;
        };
        let pulse = self.last_pulse;
        let bit = if coward {
            gamedata::mob_proto::AIFLAG_COWARD
        } else {
            gamedata::mob_proto::AIFLAG_AGGRESSIVE
        };
        while count > 0 {
            count -= 1;
            let (low_x, low_y) = (
                spot.x.saturating_sub(number(&mut self.dice, 200, 750)),
                spot.y.saturating_sub(number(&mut self.dice, 200, 750)),
            );
            let (high_x, high_y) = (
                spot.x.saturating_add(number(&mut self.dice, 200, 750)),
                spot.y.saturating_add(number(&mut self.dice, 200, 750)),
            );
            let (Some(region), Some(cells), Some(respawner)) = (
                self.regens.regions.get(&place).cloned(),
                self.regens.cells.get(place.1).cloned(),
                self.regens.respawner.as_mut(),
            ) else {
                return;
            };
            match respawner.spawn_mob_line(
                (&region, &cells),
                vnum,
                (low_x, low_y, high_x, high_y),
                stone,
                &mut self.dice,
            ) {
                Ok(Some(mut stood)) => {
                    stood.ai_flags |= bit;
                    self.stand_runtime(place, vec![stood], pulse);
                }
                Ok(None) => {}
                Err(error) => {
                    warn!(?place, %error, "a flagged mob line outnumbered the VIDs left");
                    return;
                }
            }
        }
    }

    /// `FuncWeaken`'s set (`G/cmd_gm.cpp:2845-2846`): the NPC's hit points
    /// become 10 through `PointChange(POINT_HP)`.
    fn weaken_npc(&mut self, vid: u32) {
        let Some(npc) = self.npc_by_vid(vid) else {
            return;
        };
        if npc.dead || npc.stunned {
            return;
        }
        let amount = 10i32
            .saturating_sub(npc.hp)
            .min(npc.max_hp.saturating_sub(npc.hp));
        let Some(npc) = self.npc_mut(vid) else {
            return;
        };
        npc.hp = npc.hp.saturating_add(amount);
        self.broadcast_npc_target(vid);
    }

    /// `M2_DESTROY_CHARACTER` of the non-player under `vid`. A VID that names none is
    /// refused, as `DestroyCharacter` refuses one it cannot find (`G/char_manager.cpp:131-136`).
    pub(super) fn destroy_npc(&mut self, vid: u32) {
        let Some(&(channel, map, at)) = self.npc_of.get(&vid) else {
            return;
        };
        let key = (channel, map);
        // `HorseSummon(false)` and `GetRider()->ClearHorseInfo()` (`G/char.cpp:632-635`).
        self.forget_horse_npc(vid);
        // `DeleteMount` of a mount follower (`G/MountSystem.cpp:391-404`).
        self.forget_mount_npc(vid);
        // `DeletePet` of a pet (`G/PetSystem.cpp:404-421`).
        self.forget_pet_npc(vid);
        let Some(list) = self.npcs.get_mut(&key) else {
            return;
        };
        let list = Arc::make_mut(list);
        // `--m_pkRegen->count`.
        if let Some(regen) = list.npcs.get(at).and_then(|npc| npc.regen) {
            if let Some(regen) = list.regens.get_mut(regen) {
                regen.count = regen.count.saturating_sub(1);
            }
        }
        // `ClearStone` (`:652`): a stone's sons die with it, in map order; a son's own
        // row goes with it, which drops it from the derived set.
        self.kill_stone_sons(vid);
        let Some(list) = self.npcs.get_mut(&key) else {
            return;
        };
        let list = Arc::make_mut(list);
        // A master's party is deleted, which leaves each member following no one; a member
        // that quits changes nothing the Rewrite keeps.
        for npc in &mut list.npcs {
            if npc.leader == Some(vid) {
                npc.leader = None;
            }
        }
        // `ClearSync` (`:653`) tells the view it has no owner and releases what it owns, then
        // `ClearTarget` (`:654`).
        self.release_npc_sync(vid);
        self.release_owned(Vid::new(vid));
        self.clear_target(Vid::new(vid));
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&key) {
            index.remove(EntityKey::Npc(vid), &mut effects);
        }
        self.deliver(&effects, None);
        self.npc_of.remove(&vid);
        self.minds.forget(vid);
        // Its stun, dead and recovery events end with it (`:713-718`).
        self.forget_npc_fight(vid);
        if let Some(list) = self.npcs.get_mut(&key) {
            let list = Arc::make_mut(list);
            if at < list.npcs.len() {
                list.npcs.swap_remove(at);
                if let Some(moved) = list.npcs.get(at) {
                    self.npc_of.insert(moved.vid, (channel, map, at));
                }
            }
        }
        if let Some(warps) = self.warp_npcs.get_mut(&key) {
            warps.retain(|warp| warp.vid() != vid);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gamedata::mob_group::MobGroups;
    use gamedata::mob_locale_names::MobLocaleNames;
    use gamedata::mob_proto::{MobProto, MobProtos, CHAR_TYPE_MONSTER, CHAR_TYPE_NPC};
    use gamedata::records::MobTableRecord;
    use gamedata::regen::{RegenEntry, RegenKind};
    use protocol::cg_attack::CgAttack;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::npc::{MapNpcs, Npc, NpcVids, Regen, Respawner, FIRST_NPC_VID};

    use super::super::fixtures::{a_world, enter, open_cells, region, TestClock, PLACE};
    use super::super::view_encode::remove_record;
    use super::*;
    use crate::game_loop::PulseProcessor;

    /// `GC_CHARACTER_ADD`'s header.
    const CHARACTER_ADD: u8 = 0x01;

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The VIDs of the characters an inbox was sent the insert of.
    fn added(records: &[Vec<u8>]) -> Vec<u32> {
        records
            .iter()
            .filter(|record| record.first() == Some(&CHARACTER_ADD))
            .map(|record| u32::from_le_bytes(record[1..5].try_into().unwrap()))
            .collect()
    }

    /// Monster 101, ore vein 20047, and warp 20018 to (10000, 20000), which `AIFLAG_NOMOVE`
    /// keeps from walking into an inbox once they respawn.
    fn respawner(first: u32) -> Respawner {
        let row = |vnum, mob_type| MobProto {
            line: 2,
            table: MobTableRecord {
                vnum,
                mob_type,
                ai_flag: gamedata::mob_proto::AIFLAG_NOMOVE,
                max_hp: 100,
                ..MobTableRecord::default()
            },
        };
        let protos = MobProtos::from_rows(vec![
            row(101, CHAR_TYPE_MONSTER),
            row(20_047, CHAR_TYPE_NPC),
            row(20_018, gamedata::mob_proto::CHAR_TYPE_WARP),
        ]);
        let names = MobLocaleNames::parse(b"VNUM\tNAME\n20018\tx 100 200\n").unwrap();
        Respawner::new(
            Arc::new(protos),
            Arc::new(MobGroups::default()),
            Arc::new(names),
            NpcVids::starting_at(first),
        )
    }

    /// An entry for `vnum` at (500, 500), facing its first direction, every `time` seconds.
    fn at_a_point(vnum: i32, max_count: i32, time: u32) -> RegenEntry {
        RegenEntry {
            kind: RegenKind::Mob,
            sx: 500,
            sy: 500,
            ex: 500,
            ey: 500,
            z_section: 0,
            direction: 1,
            time,
            max_count,
            vnum,
        }
    }

    /// A monster at (500, 500), which `AIFLAG_NOMOVE` keeps from walking into the inbox.
    fn a_character(vid: u32, vnum: u32, regen: Option<usize>) -> Npc {
        Npc {
            vid,
            vnum,
            race: u16::try_from(vnum).unwrap(),
            char_type: CHAR_TYPE_MONSTER,
            x: 500,
            y: 500,
            hp: 100,
            max_hp: 100,
            regen,
            ai_flags: gamedata::mob_proto::AIFLAG_NOMOVE,
            ..Npc::default()
        }
    }

    /// A world with `npcs` and `regens` stood up on [`PLACE`] at Pulse 0, its respawner
    /// installed, and a player near them whose inbox is drained. The player is stored above its
    /// maximum stamina, so no refill reaches its inbox.
    fn a_map(npcs: Vec<Npc>, regens: Vec<Regen>) -> (GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world.install_respawner(respawner(FIRST_NPC_VID + 100), open_cells());
        world.stand_npcs(
            PLACE.0,
            &region(),
            MapNpcs {
                npcs,
                positions: Vec::new(),
                regens,
            },
        );
        let mut inbox = enter(&mut world, 1, (600, 600), 12_345);
        drained(&mut inbox);
        (world, inbox)
    }

    fn pulses_to(world: &mut GameState, from: u64, to: u64) {
        for pulse in from..=to {
            world.process_pulse(pulse);
        }
    }

    fn list(world: &GameState) -> Arc<MapNpcs> {
        world.npcs_on(PLACE.0, PLACE.1)
    }

    /// A regen first runs at its drawn second and then every `time` seconds, and spawns only
    /// what its count lacks: here two of three, each indexed at its own place, and then none.
    #[test]
    fn a_regen_first_runs_at_its_draw_and_then_spawns_only_what_it_lacks() {
        let regen = Regen {
            entry: at_a_point(101, 3, 2),
            count: 1,
            first_fire: 3,
        };
        let (mut world, mut inbox) =
            a_map(vec![a_character(FIRST_NPC_VID, 101, Some(0))], vec![regen]);
        pulses_to(&mut world, 1, 74);
        assert_eq!(
            drained(&mut inbox),
            Vec::<Vec<u8>>::new(),
            "nothing before three seconds"
        );
        assert_eq!(list(&world).npcs.len(), 1);
        world.process_pulse(75);
        let (first, second) = (FIRST_NPC_VID + 100, FIRST_NPC_VID + 101);
        assert_eq!(added(&drained(&mut inbox)), [first, second]);
        let after = list(&world);
        assert_eq!(after.npcs.len(), 3);
        assert_eq!(after.regens[0].count, 3);
        assert_eq!(
            (after.npcs[1].regen, after.npcs[2].regen),
            (Some(0), Some(0))
        );
        assert_eq!(world.npc_of[&first], (PLACE.0, PLACE.1, 1));
        assert_eq!(world.npc_of[&second], (PLACE.0, PLACE.1, 2));
        let index = &world.maps[&PLACE];
        assert!(index.spot(EntityKey::Npc(second)).is_some());
        // Two seconds on, not the three of the first run: 125, then 175.
        pulses_to(&mut world, 76, 130);
        assert_eq!(
            drained(&mut inbox),
            Vec::<Vec<u8>>::new(),
            "a full regen spawns nothing"
        );
        assert_eq!(list(&world).npcs.len(), 3);
        assert_eq!(
            world.regens.due.iter().next(),
            Some(&(
                175,
                Due::Regen {
                    channel: PLACE.0,
                    map: PLACE.1,
                    at: 0
                }
            )),
            "75, then every 50 Pulses"
        );
    }

    /// An ore vein ends at its time: the regen counts one fewer, each player that saw it is sent
    /// its removal, each that targeted it VID 0, and the regen's next run stands another vein,
    /// whose own end is then due.
    #[test]
    fn an_ore_vein_ends_at_its_time_and_its_regen_stands_another() {
        let mut vein = a_character(FIRST_NPC_VID, 20_047, Some(0));
        vein.char_type = CHAR_TYPE_NPC;
        vein.ore_lifetime = Some(2);
        let regen = Regen {
            entry: at_a_point(20_047, 1, 3),
            count: 1,
            first_fire: 4,
        };
        let (mut world, mut inbox) = a_map(vec![vein], vec![regen]);
        world.target(Vid::new(1), FIRST_NPC_VID);
        drained(&mut inbox);
        pulses_to(&mut world, 1, 49);
        assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new());
        world.process_pulse(50);
        assert_eq!(
            drained(&mut inbox),
            [
                super::super::target::target_record(0, (0, 0, 0), 0),
                remove_record(FIRST_NPC_VID)
            ]
        );
        assert!(list(&world).npcs.is_empty());
        assert_eq!(list(&world).regens[0].count, 0);
        assert!(!world.npc_of.contains_key(&FIRST_NPC_VID));
        assert!(world.maps[&PLACE]
            .spot(EntityKey::Npc(FIRST_NPC_VID))
            .is_none());
        assert_eq!(world.targets.of(1), None);
        pulses_to(&mut world, 51, 99);
        assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new());
        world.process_pulse(100);
        assert_eq!(added(&drained(&mut inbox)), [FIRST_NPC_VID + 100]);
        let after = list(&world);
        assert_eq!(after.regens[0].count, 1);
        let lifetime = after.npcs[0]
            .ore_lifetime
            .expect("a vein draws its lifetime");
        assert!((420..=900).contains(&lifetime));
        assert!(world.regens.due.contains(&(
            100 + u64::from(lifetime) * 25,
            Due::Vein(FIRST_NPC_VID + 100)
        )));
    }

    /// A monster a player kills goes as an ore vein goes: a second after its death its regen
    /// counts one fewer, and the regen's next run stands another (`G/char_battle.cpp:473-548`,
    /// `G/char.cpp:653-789`).
    #[test]
    fn a_killed_monster_gives_its_regen_room_for_another() {
        let mut monster = a_character(FIRST_NPC_VID, 101, Some(0));
        monster.hp = 1;
        let regen = Regen {
            entry: at_a_point(101, 1, 3),
            count: 1,
            first_fire: 5,
        };
        let (mut world, mut inbox) = a_map(vec![monster], vec![regen]);
        world.attack(Vid::new(1), CgAttack::new(0, FIRST_NPC_VID, 0, 0));
        assert!(world.npc_by_vid(FIRST_NPC_VID).unwrap().stunned);
        pulses_to(&mut world, 1, 99);
        assert_eq!(list(&world).regens[0].count, 1);
        assert!(world.npc_by_vid(FIRST_NPC_VID).unwrap().dead);
        world.process_pulse(100);
        assert_eq!(list(&world).regens[0].count, 0);
        assert!(list(&world).npcs.is_empty());
        assert_eq!(
            drained(&mut inbox).last(),
            Some(&remove_record(FIRST_NPC_VID))
        );
        pulses_to(&mut world, 101, 124);
        assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new());
        world.process_pulse(125);
        assert_eq!(added(&drained(&mut inbox)), [FIRST_NPC_VID + 100]);
        assert_eq!(list(&world).regens[0].count, 1);
    }

    /// Destroying a character moves the last one into its place, which the VID index follows,
    /// and leaves its followers following no one and the warp list without it.
    #[test]
    fn a_destroyed_character_gives_its_place_to_the_last_and_frees_its_followers() {
        let leader = a_character(FIRST_NPC_VID, 101, None);
        let mut follower = a_character(FIRST_NPC_VID + 1, 101, None);
        follower.leader = Some(FIRST_NPC_VID);
        let last = a_character(FIRST_NPC_VID + 2, 101, None);
        let (mut world, mut inbox) = a_map(vec![leader, follower, last], Vec::new());
        world.destroy_npc(FIRST_NPC_VID);
        assert_eq!(drained(&mut inbox), [remove_record(FIRST_NPC_VID)]);
        let after = list(&world);
        let vids: Vec<u32> = after.npcs.iter().map(|n| n.vid).collect();
        assert_eq!(vids, [FIRST_NPC_VID + 2, FIRST_NPC_VID + 1]);
        assert_eq!(after.npcs[1].leader, None);
        assert_eq!(world.npc_of[&(FIRST_NPC_VID + 2)], (PLACE.0, PLACE.1, 0));
        assert_eq!(
            world.npc_by_vid(FIRST_NPC_VID + 2).unwrap().vid,
            FIRST_NPC_VID + 2
        );
        world.destroy_npc(FIRST_NPC_VID + 1);
        assert_eq!(world.npc_of[&(FIRST_NPC_VID + 2)], (PLACE.0, PLACE.1, 0));
        world.destroy_npc(FIRST_NPC_VID + 2);
        assert!(list(&world).npcs.is_empty());
        drained(&mut inbox);
        // A VID that names no character is refused, as `DestroyCharacter` refuses one.
        world.destroy_npc(FIRST_NPC_VID);
        assert_eq!(drained(&mut inbox), Vec::<Vec<u8>>::new());
    }

    /// A destroyed warp NPC leaves the warp list; the others stay.
    #[test]
    fn a_destroyed_warp_npc_leaves_the_warp_list() {
        let warp = |vid| Npc {
            char_type: gamedata::mob_proto::CHAR_TYPE_WARP,
            name: b"x 41 100 100".to_vec(),
            ..a_character(vid, 20_018, None)
        };
        let (mut world, _inbox) = a_map(
            vec![warp(FIRST_NPC_VID), warp(FIRST_NPC_VID + 1)],
            Vec::new(),
        );
        let warps = |world: &GameState| -> Vec<u32> {
            world.warp_npcs[&PLACE]
                .iter()
                .map(warp_npc::WarpNpc::vid)
                .collect()
        };
        assert_eq!(warps(&world), [FIRST_NPC_VID, FIRST_NPC_VID + 1]);
        world.destroy_npc(FIRST_NPC_VID);
        assert_eq!(warps(&world), [FIRST_NPC_VID + 1]);
    }

    /// A warp NPC a regen stands up after boot starts its event with the others.
    #[test]
    fn a_respawned_warp_npc_joins_the_warp_list() {
        let regen = Regen {
            entry: at_a_point(20_018, 1, 1),
            count: 0,
            first_fire: 1,
        };
        let (mut world, mut inbox) = a_map(Vec::new(), vec![regen]);
        assert!(world.warp_npcs[&PLACE].is_empty());
        pulses_to(&mut world, 1, 25);
        assert_eq!(added(&drained(&mut inbox)), [FIRST_NPC_VID + 100]);
        let warps: Vec<u32> = world.warp_npcs[&PLACE]
            .iter()
            .map(warp_npc::WarpNpc::vid)
            .collect();
        assert_eq!(warps, [FIRST_NPC_VID + 100]);
        assert_eq!(
            world.npc_by_vid(FIRST_NPC_VID + 100).unwrap().name,
            b"x 100 200"
        );
    }

    /// Without a respawner a regen only keeps its schedule; a descriptor that holds a map's list
    /// keeps the list it was given when a destroy changes it.
    #[test]
    fn a_regen_without_a_respawner_spawns_nothing_and_a_held_list_is_kept() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let regen = Regen {
            entry: at_a_point(101, 1, 1),
            count: 0,
            first_fire: 1,
        };
        world.stand_npcs(
            PLACE.0,
            &region(),
            MapNpcs {
                npcs: vec![a_character(FIRST_NPC_VID, 101, None)],
                positions: Vec::new(),
                regens: vec![regen],
            },
        );
        let held = list(&world);
        pulses_to(&mut world, 1, 25);
        assert_eq!(list(&world).npcs.len(), 1);
        assert!(world.regens.due.contains(&(
            50,
            Due::Regen {
                channel: PLACE.0,
                map: PLACE.1,
                at: 0
            }
        )));
        world.destroy_npc(FIRST_NPC_VID);
        assert_eq!(held.npcs.len(), 1, "the descriptor's list");
        assert!(list(&world).npcs.is_empty());
    }

    /// Standing a map up again drops its old regens' runs; the new ones are due from the
    /// Pulse it was stood up on.
    #[test]
    fn a_replaced_map_drops_its_old_regen_runs() {
        let (mut world, _inbox) = a_map(
            Vec::new(),
            vec![
                Regen {
                    entry: at_a_point(101, 1, 9),
                    count: 1,
                    first_fire: 5,
                },
                Regen {
                    entry: at_a_point(101, 1, 9),
                    count: 1,
                    first_fire: 6,
                },
            ],
        );
        world.last_pulse = 10;
        world.stand_npcs(
            PLACE.0,
            &region(),
            MapNpcs {
                npcs: Vec::new(),
                positions: Vec::new(),
                regens: vec![Regen {
                    entry: at_a_point(101, 1, 9),
                    count: 1,
                    first_fire: 2,
                }],
            },
        );
        let due: Vec<(u64, Due)> = world.regens.due.iter().copied().collect();
        assert_eq!(
            due,
            [(
                60,
                Due::Regen {
                    channel: PLACE.0,
                    map: PLACE.1,
                    at: 0
                }
            )]
        );
    }

    /// The events due on a Pulse are taken in order, an ore vein's before a regen's, and the
    /// later ones stay.
    #[test]
    fn the_due_events_are_taken_in_order_and_the_later_ones_stay() {
        let mut regens = Regens::default();
        let regen = Due::Regen {
            channel: 1,
            map: 1,
            at: 0,
        };
        regens.due.insert((5, regen));
        regens.due.insert((5, Due::Vein(9)));
        regens.due.insert((4, Due::Vein(10)));
        regens.due.insert((6, Due::Vein(1)));
        assert!(regens.take_due(3).is_empty());
        assert_eq!(regens.take_due(5), [Due::Vein(10), Due::Vein(9), regen]);
        assert_eq!(regens.take_due(u64::MAX), [Due::Vein(1)]);
        assert!(regens.due.is_empty());
        assert_eq!(pulses(3), 75);
        assert_eq!(pulses(u32::MAX), u64::from(u32::MAX) * 25);
    }

    /// A player's events due on one Pulse are taken after the non-players', its stun before its
    /// death before its recovery, whatever their VIDs.
    #[test]
    fn a_players_stun_dead_and_recovery_events_are_taken_in_that_order() {
        let mut regens = Regens::default();
        let due = [
            Due::PcRecovery(1),
            Due::PcDead(2),
            Due::PcStun(3),
            Due::Recovery(4),
        ];
        for what in due {
            regens.due.insert((5, what));
        }
        assert_eq!(
            regens.take_due(5),
            [
                Due::Recovery(4),
                Due::PcStun(3),
                Due::PcDead(2),
                Due::PcRecovery(1)
            ]
        );
    }

    /// A purge destroys the near monster and leaves the far one: the
    /// removal rides, the regen counts one fewer, and the list drops it.
    #[test]
    fn a_purge_destroys_the_near_monster_and_keeps_the_far_one() {
        let near = a_character(FIRST_NPC_VID, 101, Some(0));
        let mut far = a_character(FIRST_NPC_VID + 1, 101, None);
        far.x = 5000;
        far.y = 5000;
        let regen = Regen {
            entry: at_a_point(101, 2, 60),
            count: 1,
            first_fire: 3600,
        };
        let (mut world, mut inbox) = a_map(vec![near, far], vec![regen]);
        world.purge_around(Vid::new(1), false, false);
        assert_eq!(
            drained(&mut inbox),
            [remove_record(FIRST_NPC_VID)],
            "the near one's removal rides"
        );
        assert!(world.npc_by_vid(FIRST_NPC_VID).is_none());
        assert!(world.npc_by_vid(FIRST_NPC_VID + 1).is_some());
        assert_eq!(
            list(&world).regens[0].count,
            0,
            "the held regen counts down"
        );
    }

    /// `all` destroys the far one too; a player-typed row is never an NPC.
    #[test]
    fn a_purge_all_destroys_the_far_one_but_no_player_row() {
        let mut far = a_character(FIRST_NPC_VID, 101, None);
        far.x = 5000;
        far.y = 5000;
        let mut pc = a_character(FIRST_NPC_VID + 1, 101, None);
        pc.char_type = gamedata::mob_proto::CHAR_TYPE_PC;
        let (mut world, mut inbox) = a_map(vec![far, pc], vec![]);
        world.purge_around(Vid::new(1), true, false);
        assert_eq!(
            drained(&mut inbox),
            [remove_record(FIRST_NPC_VID)],
            "only the far monster's removal rides"
        );
        assert!(world.npc_by_vid(FIRST_NPC_VID).is_none());
        assert!(world.npc_by_vid(FIRST_NPC_VID + 1).is_some());
    }

    /// A weaken sets the near monster to 10 hit points; the far one keeps 100.
    #[test]
    fn a_weaken_sets_the_near_monster_to_ten() {
        let near = a_character(FIRST_NPC_VID, 101, None);
        let mut far = a_character(FIRST_NPC_VID + 1, 101, None);
        far.x = 5000;
        far.y = 5000;
        let (mut world, _inbox) = a_map(vec![near, far], vec![]);
        world.purge_around(Vid::new(1), false, true);
        assert_eq!(world.npc_by_vid(FIRST_NPC_VID).unwrap().hp, 10);
        assert_eq!(world.npc_by_vid(FIRST_NPC_VID + 1).unwrap().hp, 100);
    }

    /// A respawn fires the holding regen one Pulse on; the far regen keeps
    /// its due, and `all` moves both.
    #[test]
    fn a_respawn_moves_only_the_holding_regen_unless_all() {
        let held = Regen {
            entry: RegenEntry {
                sx: 0,
                sy: 0,
                ex: 1000,
                ey: 1000,
                ..at_a_point(101, 3, 1000)
            },
            count: 3,
            first_fire: 1000,
        };
        let mut far_entry = at_a_point(101, 3, 1000);
        far_entry.sx = 50_000;
        far_entry.ex = 51_000;
        far_entry.sy = 50_000;
        far_entry.ey = 51_000;
        let far = Regen {
            entry: far_entry,
            count: 3,
            first_fire: 1000,
        };
        let (mut world, _inbox) = a_map(Vec::new(), vec![held, far]);
        world.process_pulse(1);
        let held_due = Due::Regen {
            channel: PLACE.0,
            map: PLACE.1,
            at: 0,
        };
        let far_due = Due::Regen {
            channel: PLACE.0,
            map: PLACE.1,
            at: 1,
        };
        world.respawn_around(Vid::new(1), false);
        assert!(
            world.regens.due.contains(&(2, held_due)),
            "the holding regen fires one Pulse on"
        );
        assert!(
            !world.regens.due.contains(&(2, far_due)),
            "the far regen keeps its due"
        );
        world.respawn_around(Vid::new(1), true);
        assert!(
            world.regens.due.contains(&(2, far_due)),
            "`all` moves the far regen too"
        );
    }
}
