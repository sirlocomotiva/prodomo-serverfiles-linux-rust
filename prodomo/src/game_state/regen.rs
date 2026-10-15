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
use world::cells::HostedCells;
use world::npc::{Npc, Respawner};

use tracing::warn;

use super::view::EntityKey;
use super::{warp_npc, GameState};
use crate::save::PASSES_PER_SEC;

/// An event due on a Pulse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Due {
    /// `kill_ore_load_event` of the ore vein under this VID.
    Vein(u32),
    /// `regen_event` of the regen at this place in its map's list.
    Regen {
        /// The Channel.
        channel: u8,
        /// The map index.
        map: i32,
        /// Its place in the map's [`world::npc::MapNpcs::regens`].
        at: usize,
    },
    /// `StunEvent` of the non-player under this VID. See [`super::fight`].
    Stun(u32),
    /// `dead_event` of the non-player under this VID.
    Dead(u32),
    /// `recovery_event` of the non-player under this VID.
    Recovery(u32),
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
    due: BTreeSet<(u64, Due)>,
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

    /// `SECTREE::IsAttr(x, y, ATTR_BANPK)` on the cells boot handed over: false on a map it
    /// handed none of.
    pub(super) fn is_banpk(&self, map: i32, x: i32, y: i32) -> bool {
        self.regens
            .cells
            .get(map)
            .is_some_and(|cells| cells.is_attr(x, y, ATTR_BANPK))
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

    /// Runs every event due on or before `pulse`: the regens, the ore veins' ends and the
    /// non-players' stun, dead and recovery events.
    pub(super) fn run_regens(&mut self, pulse: u64) {
        for what in self.regens.take_due(pulse) {
            match what {
                // The VID of a character already destroyed names nothing, as no VID is reused.
                Due::Vein(vid) => self.destroy_npc(vid),
                Due::Regen { channel, map, at } => self.fire_regen((channel, map), at, pulse),
                Due::Stun(vid) => self.stun_event(vid),
                Due::Dead(vid) => self.dead_event(vid),
                Due::Recovery(vid) => self.recovery_event(vid),
            }
        }
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
        let first = list.npcs.len();
        list.npcs.extend(stood);
        let mut effects = Vec::new();
        let mut veins = Vec::new();
        for (offset, npc) in list.npcs[first..].iter().enumerate() {
            self.npc_of
                .insert(npc.vid, (key.0, key.1, first.saturating_add(offset)));
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

    /// `M2_DESTROY_CHARACTER` of the non-player under `vid`. A VID that names none is
    /// refused, as `DestroyCharacter` refuses one it cannot find (`G/char_manager.cpp:131-136`).
    pub(super) fn destroy_npc(&mut self, vid: u32) {
        let Some(&(channel, map, at)) = self.npc_of.get(&vid) else {
            return;
        };
        let key = (channel, map);
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
        // A master's party is deleted, which leaves each member following no one; a member
        // that quits changes nothing the Rewrite keeps.
        for npc in &mut list.npcs {
            if npc.leader == Some(vid) {
                npc.leader = None;
            }
        }
        // `ClearSync` (`:653`) tells the view it has no owner, then `ClearTarget` (`:654`).
        self.release_npc_sync(vid);
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
}
