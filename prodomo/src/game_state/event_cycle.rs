//! `sys.event.manager`'s map-230 arms (events 3-6, `SetBossEvent`, `SetMetinEvent`,
//! `SetMiningEvent`, `SetGoldFrogEvent`, `G/event_manager.cpp:986-1154`) and their walks
//! (`dynamic_spawn_cycle_event`, `static_spawn_cycle_event`, `:220-336`).
//!
//! The boss and metin walks are the dynamic one: rounds 1-5 spawn the first file,
//! 6-10 the second, the map clears at rounds 1 and 6, each round fires 5 min out,
//! and past round 10 the walk stops. The frog walk is the static one: rounds 1-3
//! spawn the frog file, the map clears at round 1 (no Tanaka exemption off the
//! Tanaka map), each round fires 8 min out, and past round 3 it stops. Mining is
//! no walk: the start frees the map's regens and loads the vein file (its spawn
//! now, its timer at `number(0, 16) + time`), as `regen_load` does
//! (`G/regen.cpp:601-692`); the end frees them again.
//!
//! Every arm gates on a hosted map 230 (`EVENT_MAP_INDEX`, `G/event_manager.h:38`),
//! which the owner's index does not list: there the starts set their flag and arm
//! nothing, and the ends clear their flag and chat with no warp, no telling and
//! no kill. The ends' order is legacy's: the flag falls and the end chats before
//! the gate, then the home warp
//! queues (60 s for the boss, 15 s for the rest), the map hears the teleport
//! line, and the monsters, stones and veins die. The frog's end keeps its
//! missing space (`has ended.Thank you`).
//!
//! One `Due::MapCycle` fires per round for every place hosting 230, where legacy
//! runs one event per core: the outcome is the same (each place spawns once per
//! round) with one timer. The walks spawn `m` lines through `SpawnMobRange`'s 16
//! tries, aggressive (`regen_load_in_file` forces it, `:538`); points spawn at
//! their point, not aggressive, as legacy's point path takes none. `g` lines
//! spawn their group the same way, and `s` lines draw anywhere; the other kinds
//! wait, as no event file holds them.

use gamedata::regen::{RegenEntry, RegenKind};

use super::regen::{pulses, Due};
use super::siege_war::TELEPORT_LINE;

/// The event map (`EVENT_MAP_INDEX`).
pub(super) const EVENT_MAP: i32 = 230;
/// The dynamic walk's first stage (`DYNAMIC_CYCLE_STAGE1`, `:30`).
const DYNAMIC_STAGE1: u8 = 5;
/// The dynamic walk's last round (`DYNAMIC_CYCLE_STAGE2`, `:31`).
const DYNAMIC_STAGE2: u8 = 10;
/// The frog walk's last round (`STATIC_CYCLE_MAX_ROUND`, `:33`).
const STATIC_ROUNDS: u8 = 3;
/// The round clearing the map a second time (`CLEAR_ENTITY_STAGE2_ROUND`, `:28`).
const CLEAR_STAGE2: u8 = 6;
/// The boss end's home warp, in seconds (`:1018`).
const BOSS_WARP_SECS: u64 = 60;
/// The metin, mining and frog ends' home warp, in seconds (`:1059,1101,1145`).
const WARP_SECS: u64 = 15;

/// The map-230 walk a `Due::MapCycle` fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum CycleStage {
    /// The frog's static walk.
    Frog,
    /// The boss's dynamic walk.
    Boss,
    /// The metin's dynamic walk.
    Metin,
}

/// The map-230 walks' placed entries, by load order.
#[derive(Debug, Clone, Default)]
pub struct EventCycleFiles {
    /// The frog file's entries.
    pub frog: Vec<RegenEntry>,
    /// The boss files' entries, first and second stage.
    pub boss: [Vec<RegenEntry>; 2],
    /// The metin files' entries, first and second stage.
    pub metin: [Vec<RegenEntry>; 2],
    /// The mining file's entries.
    pub mining: Vec<RegenEntry>,
}

impl super::GameState {
    /// `SetBossEvent(true)` (`:986-1002`): the flag rises, and on a hosted 230
    /// the dynamic walk arms at +1 s and the start chats.
    pub(super) fn start_boss_event(&mut self) {
        self.set_event_flag(b"boss_event", 1);
        if self.places_hosting(EVENT_MAP).is_empty() {
            return;
        }
        let when = self.last_pulse.saturating_add(pulses(1));
        self.regens.due.insert((
            when,
            Due::MapCycle {
                stage: CycleStage::Boss,
                step: 0,
            },
        ));
        self.broadcast_notice_text(b"Boss Hunt Event has started. Open the Quest Scroll to join.");
    }

    /// `SetBossEvent(false)` (`:1004-1025`): the flag falls and the end chats, and on a hosted 230
    /// the 60 s home warp queues, the map hears the teleport line, and its
    /// monsters, stones and veins die.
    pub(super) fn end_boss_event(&mut self) {
        self.set_event_flag(b"boss_event", 0);
        self.broadcast_notice_text(b"Boss Hunt Event has ended. Thank you for participating.");
        if self.places_hosting(EVENT_MAP).is_empty() {
            return;
        }
        self.queue_war_home(EVENT_MAP, BOSS_WARP_SECS, 0);
        self.tell_war_map(EVENT_MAP, TELEPORT_LINE, false);
        self.kill_war_map(EVENT_MAP);
    }

    /// `SetMetinEvent(true)` (`:1028-1044`): the flag rises, and on a hosted 230
    /// the dynamic walk arms at +1 s and the start chats.
    pub(super) fn start_metin_event(&mut self) {
        self.set_event_flag(b"metin_event", 1);
        if self.places_hosting(EVENT_MAP).is_empty() {
            return;
        }
        let when = self.last_pulse.saturating_add(pulses(1));
        self.regens.due.insert((
            when,
            Due::MapCycle {
                stage: CycleStage::Metin,
                step: 0,
            },
        ));
        self.broadcast_notice_text(
            b"Metin Shower Event has started. Open the Quest Scroll to join.",
        );
    }

    /// `SetMetinEvent(false)` (`:1046-1065`): the flag falls and the end chats, and on a hosted 230
    /// the 15 s home warp queues, the map hears the teleport line, and its
    /// monsters, stones and veins die.
    pub(super) fn end_metin_event(&mut self) {
        self.set_event_flag(b"metin_event", 0);
        self.broadcast_notice_text(b"Metin Shower Event has ended. Thank you for participating.");
        if self.places_hosting(EVENT_MAP).is_empty() {
            return;
        }
        self.queue_war_home(EVENT_MAP, WARP_SECS, 0);
        self.tell_war_map(EVENT_MAP, TELEPORT_LINE, false);
        self.kill_war_map(EVENT_MAP);
    }

    /// `SetMiningEvent(true)` (`:1070-1090`): the flag rises, and on a hosted 230
    /// the map's regens free, the vein file loads, and the start chats.
    pub(super) fn start_mining_event(&mut self) {
        self.set_event_flag(b"mining_event", 1);
        if self.places_hosting(EVENT_MAP).is_empty() {
            return;
        }
        self.free_event_regens();
        self.load_event_regens();
        self.broadcast_notice_text(b"Mining Event has started. Open the Quest Scroll to join.");
    }

    /// `SetMiningEvent(false)` (`:1092-1112`): the flag falls and the end chats, and on a hosted 230
    /// the 15 s home warp queues, the map hears the teleport line, the regens
    /// free, and its monsters, stones and veins die.
    pub(super) fn end_mining_event(&mut self) {
        self.set_event_flag(b"mining_event", 0);
        self.broadcast_notice_text(b"Mining Event has ended. Thank you for participating.");
        if self.places_hosting(EVENT_MAP).is_empty() {
            return;
        }
        self.queue_war_home(EVENT_MAP, WARP_SECS, 0);
        self.tell_war_map(EVENT_MAP, TELEPORT_LINE, false);
        self.free_event_regens();
        self.kill_war_map(EVENT_MAP);
    }

    /// `SetGoldFrogEvent(true)` (`:1114-1130`): the flag rises, and on a hosted
    /// 230 the static walk arms at +1 s and the start chats.
    pub(super) fn start_frog_event(&mut self) {
        self.set_event_flag(b"golden_frog_event", 1);
        if self.places_hosting(EVENT_MAP).is_empty() {
            return;
        }
        let when = self.last_pulse.saturating_add(pulses(1));
        self.regens.due.insert((
            when,
            Due::MapCycle {
                stage: CycleStage::Frog,
                step: 0,
            },
        ));
        self.broadcast_notice_text(
            b"Golden Frog Event has started. Open the Quest Scroll to join.",
        );
    }

    /// `SetGoldFrogEvent(false)` (`:1132-1151`): the flag falls and the end chats (keeping its
    /// missing space), and on a hosted 230 the 15 s home warp queues, the map
    /// hears the teleport line, and its monsters, stones and veins die.
    pub(super) fn end_frog_event(&mut self) {
        self.set_event_flag(b"golden_frog_event", 0);
        self.broadcast_notice_text(b"Golden Frog Event has ended.Thank you for participating.");
        if self.places_hosting(EVENT_MAP).is_empty() {
            return;
        }
        self.queue_war_home(EVENT_MAP, WARP_SECS, 0);
        self.tell_war_map(EVENT_MAP, TELEPORT_LINE, false);
        self.kill_war_map(EVENT_MAP);
    }

    /// `dynamic_spawn_cycle_event` and the frog's `static_spawn_cycle_event`
    /// (`:220-336`): the step rises, round 1 clears (and round 6 for the
    /// dynamic walk), the round's file spawns on every place hosting 230, and
    /// the next round arms (+5 min dynamic, +8 min frog). Past the last round
    /// the walk stops. Answers whether a round ran.
    pub(super) fn fire_event_cycle(&mut self, stage: CycleStage, step: u8) -> bool {
        let places = self.places_hosting(EVENT_MAP);
        if places.is_empty() {
            return false;
        }
        let step = step.saturating_add(1);
        let last = if stage == CycleStage::Frog {
            STATIC_ROUNDS
        } else {
            DYNAMIC_STAGE2
        };
        if step > last {
            return false;
        }
        if step == 1 || (stage != CycleStage::Frog && step == CLEAR_STAGE2) {
            self.kill_war_map(EVENT_MAP);
        }
        let file: &[RegenEntry] = match stage {
            CycleStage::Frog => &self.event_cycles.frog,
            CycleStage::Boss => {
                if step <= DYNAMIC_STAGE1 {
                    &self.event_cycles.boss[0]
                } else {
                    &self.event_cycles.boss[1]
                }
            }
            CycleStage::Metin => {
                if step <= DYNAMIC_STAGE1 {
                    &self.event_cycles.metin[0]
                } else {
                    &self.event_cycles.metin[1]
                }
            }
        };
        let file = file.to_vec();
        for place in &places {
            for entry in &file {
                self.spawn_cycle_line(*place, entry);
            }
        }
        let gap = if stage == CycleStage::Frog {
            pulses(8 * 60)
        } else {
            pulses(5 * 60)
        };
        let when = self.last_pulse.saturating_add(gap);
        self.regens
            .due
            .insert((when, Due::MapCycle { stage, step }));
        true
    }

    /// One walk line's `regen_spawn(regen, true)` (`G/regen.cpp:325-386`): a
    /// point spawns at its point, a box draws through `SpawnMobRange`'s 16
    /// tries, aggressive; a group line spawns its group the same way, and an
    /// anywhere line draws anywhere. Other kinds wait (no event file holds them).
    fn spawn_cycle_line(&mut self, place: (u8, i32), entry: &RegenEntry) {
        if entry.kind == RegenKind::Anywhere {
            let Ok(vnum) = u32::try_from(entry.vnum) else {
                return;
            };
            if vnum == 0 {
                return;
            }
            self.spawn_event_mob_randomly(place, vnum);
            return;
        }
        if matches!(entry.kind, RegenKind::Group { .. }) {
            let Ok(vnum) = u32::try_from(entry.vnum) else {
                return;
            };
            if vnum == 0 {
                return;
            }
            self.spawn_event_group_aggressive(
                place,
                vnum,
                (entry.sx, entry.sy, entry.ex, entry.ey),
            );
            return;
        }
        if entry.kind != RegenKind::Mob {
            return;
        }
        let Ok(vnum) = u32::try_from(entry.vnum) else {
            return;
        };
        if vnum == 0 {
            return;
        }
        if entry.sx == entry.ex && entry.sy == entry.ey {
            let dir = if entry.direction == 0 {
                world::character::number(&mut self.dice, 0, 7) * 45
            } else {
                i32::from(entry.direction.saturating_sub(1)) * 45
            };
            let _ = self.spawn_event_mob(place, vnum, (entry.sx, entry.sy), dir, false);
            return;
        }
        let _ = self.spawn_event_mob_range_aggressive(
            place,
            vnum,
            (entry.sx, entry.sy, entry.ex, entry.ey),
        );
    }

    /// `regen_free_map(230)` (`G/regen.cpp:759-774`) on every place hosting it:
    /// the regens drop and their timers cancel. The standing characters stay,
    /// as legacy's free touches no character.
    fn free_event_regens(&mut self) {
        let places = self.places_hosting(EVENT_MAP);
        for place in &places {
            if let Some(list) = self.npcs.get_mut(place) {
                std::sync::Arc::make_mut(list).regens.clear();
            }
        }
        self.regens
            .due
            .retain(|&(_, what)| !matches!(what, Due::Regen { map, .. } if map == EVENT_MAP));
    }

    /// `regen_load` of the mining file (`G/regen.cpp:601-692`) on every place
    /// hosting 230: each line places a regen that spawns now (when its time is
    /// nonzero) and fires first at `number(0, 16) + time`. Idle lines (time 0)
    /// place nothing, as at boot.
    fn load_event_regens(&mut self) {
        let places = self.places_hosting(EVENT_MAP);
        for place in places {
            let entries = self.event_cycles.mining.clone();
            for entry in entries {
                self.plant_event_regen(place, &entry);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CycleStage, EVENT_MAP};
    use crate::client_registry::ChannelClients;
    use crate::game_state::fixtures::{a_world, TestClock};
    use gamedata::map_atlas::MapRegion;
    use gamedata::mob_proto::{MobProto, MobProtos};
    use gamedata::regen::{RegenEntry, RegenKind};
    use gamedata::server_attr::{CellBlock, SectreeGrid, ServerAttr, SECTREE_CELLS};
    use std::sync::Arc;
    use world::cells::{HostedCells, MapCells};
    use world::npc::{MapNpcs, NpcVids, Respawner, FIRST_NPC_VID};

    /// A world hosting map 230 on Channel 1 with the walks' protos and `files`
    /// as their own.
    fn event_world(
        clock: &TestClock,
        files: super::EventCycleFiles,
    ) -> crate::game_state::GameState {
        let world = a_world(clock)
            .with_clients(Arc::new(ChannelClients::new()))
            .with_quests(quests())
            .with_event_cycles(files);
        let region = MapRegion {
            index: EVENT_MAP,
            name: b"test".to_vec(),
            sx: 0,
            sy: 0,
            ex: 24 * 6_400,
            ey: 24 * 6_400,
            spawn: (0, 0),
            empire_spawns: None,
        };
        let grid = SectreeGrid::of(&region).expect("the region is whole sectrees");
        let mut world = world;
        world.host_map(1, EVENT_MAP, grid);
        let blocks = (0..576)
            .map(|_| CellBlock::from_cells(&vec![0; SECTREE_CELLS * SECTREE_CELLS]))
            .collect();
        let attr = ServerAttr::from_blocks(24, 24, blocks).expect("576 blocks");
        let mut hosted = HostedCells::default();
        hosted.insert(EVENT_MAP, MapCells::new(grid, Arc::new(attr)));
        let rows = [693, 2598, 8009, 8051, 11_505, 20_047]
            .into_iter()
            .map(|vnum| MobProto {
                line: 2,
                table: gamedata::records::MobTableRecord {
                    vnum,
                    max_hp: 100,
                    ..gamedata::records::MobTableRecord::default()
                },
            })
            .collect();
        let groups = gamedata::mob_group::MobGroups::parse(
            b"Group vein\n{\n\tVnum\t20047\n\tLeader\tx\t20047\n}\n",
            b"",
        )
        .expect("the vein group parses");
        let respawner = Respawner::new(
            Arc::new(MobProtos::from_rows(rows)),
            Arc::new(groups),
            Arc::new(gamedata::mob_locale_names::MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap()),
            NpcVids::starting_at(FIRST_NPC_VID + 100),
        );
        world.install_respawner(respawner, Arc::new(hosted));
        world.stand_npcs(1, &region, MapNpcs::default());
        world
    }

    /// The quest runtime over the legacy locale.
    fn quests() -> crate::game_state::quests::Quests {
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        crate::game_state::quests::Quests::new(
            manager,
            Arc::default(),
            gamedata::mob_locale_names::MobNamesByLanguage::default(),
        )
    }

    /// One mob box naming `vnum`, as the walk files hold it.
    fn mob_box(vnum: i32) -> RegenEntry {
        RegenEntry {
            kind: RegenKind::Mob,
            sx: 1_000,
            sy: 1_000,
            ex: 1_200,
            ey: 1_200,
            z_section: 0,
            direction: 0,
            time: 60,
            max_count: 1,
            vnum,
        }
    }

    /// One vein-group box, as the mining file holds it.
    fn vein_box() -> RegenEntry {
        RegenEntry {
            kind: RegenKind::Group { aggressive: false },
            sx: 1_000,
            sy: 1_000,
            ex: 1_200,
            ey: 1_200,
            z_section: 0,
            direction: 0,
            time: 60,
            max_count: 1,
            vnum: 20_047,
        }
    }

    /// The live mobs of `race` on map 230.
    fn raced(world: &crate::game_state::GameState, race: u32) -> Vec<&world::npc::Npc> {
        world.npcs[&(1, EVENT_MAP)]
            .npcs
            .iter()
            .filter(|npc| u32::from(npc.race) == race && !npc.dead)
            .collect()
    }

    /// Whether a walk of `stage` at `step` waits on the schedule.
    fn armed(world: &crate::game_state::GameState, stage: CycleStage, step: u8) -> bool {
        world.regens.due.iter().any(|(_, what)| {
            matches!(
                what,
                super::super::regen::Due::MapCycle {
                    stage: at,
                    step: now
                } if *at == stage && *now == step
            )
        })
    }

    /// The boss walk runs ten rounds: five of the first file, five of the
    /// second, and the eleventh call stops with nothing scheduled.
    #[test]
    fn ten_rounds_walk_the_two_boss_files_and_the_eleventh_stops() {
        let clock = TestClock::default();
        let files = super::EventCycleFiles {
            boss: [vec![mob_box(693)], vec![mob_box(2598)]],
            ..super::EventCycleFiles::default()
        };
        let mut world = event_world(&clock, files);
        world.start_boss_event();
        assert_eq!(world.event_rate(b"boss_event"), 1);
        assert!(armed(&world, CycleStage::Boss, 0), "the walk arms at +1 s");
        for round in 1..=5u8 {
            assert!(world.fire_event_cycle(CycleStage::Boss, round - 1));
            assert_eq!(raced(&world, 693).len(), usize::from(round));
            assert!(armed(&world, CycleStage::Boss, round));
        }
        for round in 6..=10u8 {
            assert!(world.fire_event_cycle(CycleStage::Boss, round - 1));
            assert_eq!(raced(&world, 2598).len(), usize::from(round - 5));
            assert!(raced(&world, 693).is_empty(), "round 6 clears the map");
            assert!(armed(&world, CycleStage::Boss, round));
        }
        assert!(
            !world.fire_event_cycle(CycleStage::Boss, 10),
            "past round 10 it stops"
        );
        world.end_boss_event();
        assert_eq!(world.event_rate(b"boss_event"), 0);
    }

    /// The frog walk runs three rounds of its file and the fourth call stops;
    /// round 1 clears the map, where the Tanaka map is exempt.
    #[test]
    fn three_rounds_walk_the_frog_file_and_the_fourth_stops() {
        let clock = TestClock::default();
        let files = super::EventCycleFiles {
            frog: vec![mob_box(11_505)],
            ..super::EventCycleFiles::default()
        };
        let mut world = event_world(&clock, files);
        world.start_frog_event();
        assert_eq!(world.event_rate(b"golden_frog_event"), 1);
        assert!(armed(&world, CycleStage::Frog, 0), "the walk arms at +1 s");
        let victim = world
            .spawn_event_mob((1, EVENT_MAP), 693, (2_000, 2_000), 0, false)
            .expect("a victim stands first");
        let _ = victim;
        assert!(world.fire_event_cycle(CycleStage::Frog, 0), "round 1 runs");
        assert!(raced(&world, 693).is_empty(), "round 1 clears the map");
        assert_eq!(raced(&world, 11_505).len(), 1, "and stands its frog");
        for round in 2..=3u8 {
            assert!(world.fire_event_cycle(CycleStage::Frog, round - 1));
            assert_eq!(
                raced(&world, 11_505).len(),
                usize::from(round),
                "round {round} adds one"
            );
        }
        assert!(
            raced(&world, 11_505).iter().all(|npc| npc.is_aggressive()),
            "every frog is aggressive"
        );
        assert!(
            !world.fire_event_cycle(CycleStage::Frog, 3),
            "past round 3 it stops"
        );
        world.end_frog_event();
        assert_eq!(world.event_rate(b"golden_frog_event"), 0);
        assert!(raced(&world, 11_505).is_empty(), "the end kills the frogs");
        assert_eq!(world.siege_war.warps.len(), 1, "and queues the home warp");
    }

    /// The mining start plants its vein regen, which stands its vein now and
    /// schedules its first fire; the end frees the regen and kills the vein.
    #[test]
    fn the_mining_start_plants_its_vein_and_the_end_frees_it() {
        let clock = TestClock::default();
        let files = super::EventCycleFiles {
            mining: vec![vein_box()],
            ..super::EventCycleFiles::default()
        };
        let mut world = event_world(&clock, files);
        world.start_mining_event();
        assert_eq!(world.event_rate(b"mining_event"), 1);
        assert_eq!(
            world.npcs[&(1, EVENT_MAP)].regens.len(),
            1,
            "one regen plants"
        );
        assert_eq!(raced(&world, 20_047).len(), 1, "its vein stands now");
        assert!(
            world.regens.due.iter().any(|(_, what)| matches!(
                what,
                super::super::regen::Due::Regen { map, .. } if *map == EVENT_MAP
            )),
            "its first fire schedules"
        );
        world.end_mining_event();
        assert_eq!(world.event_rate(b"mining_event"), 0);
        assert!(
            world.npcs[&(1, EVENT_MAP)].regens.is_empty(),
            "the end frees"
        );
        assert!(raced(&world, 20_047).is_empty(), "and kills the vein");
    }

    /// Without map 230 the starts set their flags and arm nothing; the ends
    /// chat (the scenario hears them) with no warp queued, no telling and no
    /// kill.
    #[test]
    fn without_the_map_the_starts_arm_nothing_and_the_ends_queue_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock)
            .with_clients(Arc::new(ChannelClients::new()))
            .with_quests(quests())
            .with_event_cycles(super::EventCycleFiles::default());
        world.start_boss_event();
        world.start_metin_event();
        world.start_mining_event();
        world.start_frog_event();
        for flag in [
            b"boss_event".as_slice(),
            b"metin_event",
            b"mining_event",
            b"golden_frog_event",
        ] {
            assert_eq!(world.event_rate(flag), 1, "the flag rises");
        }
        assert!(
            !world
                .regens
                .due
                .iter()
                .any(|(_, what)| matches!(what, super::super::regen::Due::MapCycle { .. })),
            "no walk arms"
        );
        assert!(
            world.npcs.values().all(|list| list.regens.is_empty()),
            "no regen plants"
        );
        world.end_boss_event();
        world.end_metin_event();
        world.end_mining_event();
        world.end_frog_event();
        assert!(world.siege_war.warps.is_empty(), "no home warp queues");
    }

    /// The owner's walk files hold `m` lines only, and the mining file `g`
    /// lines only; every line places, by count.
    #[test]
    fn the_owner_files_hold_their_kinds_only() {
        let base =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/data/event");
        let walks = [
            ("boss_event_regen_type_1.txt", 88),
            ("boss_event_regen_type_2.txt", 96),
            ("metin_event_regen_type_1.txt", 149),
            ("metin_event_regen_type_2.txt", 155),
            ("golden_frog_event_regen_type_0.txt", 85),
        ];
        for (name, count) in walks {
            let bytes = std::fs::read(base.join(name)).expect("the walk file reads");
            let entries = gamedata::regen::parse(&bytes).expect("the walk file parses");
            assert_eq!(entries.len(), count, "{name} places every line");
            assert!(
                entries.iter().all(|entry| entry.kind == RegenKind::Mob),
                "{name} holds mobs only"
            );
        }
        let bytes = std::fs::read(base.join("mining_event_regen_type_0.txt"))
            .expect("the mining file reads");
        let entries = gamedata::regen::parse(&bytes).expect("the mining file parses");
        assert_eq!(entries.len(), 403, "mining places every line");
        assert!(
            entries
                .iter()
                .all(|entry| matches!(entry.kind, RegenKind::Group { .. })),
            "mining holds groups only"
        );
    }
}
