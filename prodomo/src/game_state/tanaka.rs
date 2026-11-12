//! `sys.event.manager`'s Tanaka arm (event 12, `SetTanakaEvent`,
//! `G/event_manager.cpp:1318-1359`) and its static spawn cycle
//! (`static_spawn_cycle_event`, `:287-336`).
//!
//! On the Shared Channel legacy sets the flag and chats; on the Channel hosting
//! map 63 it arms the cycle instead (the GG relay is retired, so the one process
//! does both). The cycle fires 1 s after the start, then every 8 min for 3 rounds;
//! each round spawns every `m` line of `data/event/tanaka_event_regen_type_0.txt`
//! once, aggressive, through `SpawnMobRange`'s 16 tries. The round-1 entity clear
//! is exempt for the Tanaka map, and the end never cancels the cycle (legacy
//! `SetTanakaEvent(false)` has no cancel off the Shared Channel). Group, group-group
//! and anywhere lines wait: every event regen file on the owner's data holds `m`
//! lines only.

use gamedata::regen::RegenEntry;

use super::regen::{pulses, Due};

/// The Tanaka map (`TANAKA_EVENT_MAP`, `G/event_manager.h:35`).
pub(super) const TANAKA_MAP: i32 = 63;
/// The rounds a cycle runs (`STATIC_CYCLE_MAX_ROUND`, `:33`).
const TANAKA_ROUNDS: u8 = 3;

impl super::GameState {
    /// `SetTanakaEvent(true)` (`:1318-1359`): the flag rises, the start chats, and
    /// the cycle arms at +1 s on the place hosting map 63, when one hosts it.
    pub(super) fn start_tanaka(&mut self) {
        self.set_event_flag(b"tanaka_event", 1);
        self.broadcast_notice_text(
            b"Pirate Tanaka Event has started. Tanakas took over the desert, hunt them down!",
        );
        if self.places_hosting(TANAKA_MAP).is_empty() {
            return;
        }
        let when = self.last_pulse.saturating_add(pulses(1));
        self.regens.due.insert((when, Due::TanakaCycle { step: 0 }));
    }

    /// `SetTanakaEvent(false)` (`:1335-1336`): the flag falls and the end chats.
    /// The cycle runs on: legacy never cancels it.
    pub(super) fn end_tanaka(&mut self) {
        self.set_event_flag(b"tanaka_event", 0);
        self.broadcast_notice_text(b"Pirate Tanaka Event has ended. Thank you for participating!");
    }

    /// `static_spawn_cycle_event` (`:287-336`) for the Tanaka map: the step rises,
    /// no entity clears (the `!= TANAKA_EVENT_MAP` exemption), rounds 1-3 spawn
    /// every `m` line once, and past round 3 the cycle stops. Answers whether a
    /// round ran.
    pub(super) fn fire_tanaka_cycle(&mut self, step: u8) -> bool {
        let Some(place) = self.places_hosting(TANAKA_MAP).into_iter().next() else {
            return false;
        };
        let step = step.saturating_add(1);
        if step > TANAKA_ROUNDS {
            return false;
        }
        for entry in self.tanaka.clone() {
            self.spawn_cycle_mob(place, &entry);
        }
        let when = self.last_pulse.saturating_add(pulses(8 * 60));
        self.regens.due.insert((when, Due::TanakaCycle { step }));
        true
    }

    /// One cycle line's `regen_spawn(regen, true)` (`G/regen.cpp:325-386`) for a mob:
    /// a point spawns at its point, a box draws through `SpawnMobRange`'s 16 tries,
    /// aggressive. Other kinds wait (no event file holds them).
    fn spawn_cycle_mob(&mut self, place: (u8, i32), entry: &RegenEntry) {
        if entry.kind != gamedata::regen::RegenKind::Mob {
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
}

/// The Tanaka cycle's placed entries, by load order.
pub(super) type TanakaEntries = Vec<RegenEntry>;

#[cfg(test)]
mod tests {
    use super::TANAKA_MAP;
    use crate::client_registry::ChannelClients;
    use crate::game_state::fixtures::{a_world, TestClock};
    use gamedata::map_atlas::MapRegion;
    use gamedata::mob_proto::{MobProto, MobProtos};
    use gamedata::regen::{RegenEntry, RegenKind};
    use gamedata::server_attr::{CellBlock, SectreeGrid, ServerAttr, SECTREE_CELLS};
    use std::sync::Arc;
    use world::cells::{HostedCells, MapCells};
    use world::npc::{MapNpcs, NpcVids, Respawner, FIRST_NPC_VID};

    /// A world hosting map 63 on Channel 1 with the Tanaka pirate's proto and
    /// `entries` as the cycle's file.
    fn tanaka_world(clock: &TestClock, entries: Vec<RegenEntry>) -> crate::game_state::GameState {
        let world = a_world(clock)
            .with_clients(Arc::new(ChannelClients::new()))
            .with_quests(quests())
            .with_tanaka(entries);
        let region = MapRegion {
            index: TANAKA_MAP,
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
        world.host_map(1, TANAKA_MAP, grid);
        let blocks = (0..576)
            .map(|_| CellBlock::from_cells(&vec![0; SECTREE_CELLS * SECTREE_CELLS]))
            .collect();
        let attr = ServerAttr::from_blocks(24, 24, blocks).expect("576 blocks");
        let mut hosted = HostedCells::default();
        hosted.insert(TANAKA_MAP, MapCells::new(grid, Arc::new(attr)));
        let rows = [MobProto {
            line: 2,
            table: gamedata::records::MobTableRecord {
                vnum: 5001,
                max_hp: 100,
                ..gamedata::records::MobTableRecord::default()
            },
        }]
        .into_iter()
        .collect();
        let respawner = Respawner::new(
            Arc::new(MobProtos::from_rows(rows)),
            Arc::default(),
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

    /// One mob box, as the file holds it.
    fn mob_box() -> RegenEntry {
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
            vnum: 5001,
        }
    }

    /// The live pirates of race 5001 on map 63.
    fn pirates(world: &crate::game_state::GameState) -> Vec<&world::npc::Npc> {
        world.npcs[&(1, TANAKA_MAP)]
            .npcs
            .iter()
            .filter(|npc| u32::from(npc.race) == 5001 && !npc.dead)
            .collect()
    }

    /// The start arms the cycle at +1 s; three rounds each stand the file's mob
    /// aggressive, and the fourth call stops with nothing scheduled.
    #[test]
    fn three_rounds_stand_aggressive_pirates_and_the_fourth_stops() {
        let clock = TestClock::default();
        let mut world = tanaka_world(&clock, vec![mob_box()]);
        world.start_tanaka();
        assert_eq!(world.event_rate(b"tanaka_event"), 1);
        assert!(
            world
                .regens
                .due
                .iter()
                .any(|(_, what)| matches!(what, super::super::regen::Due::TanakaCycle { step: 0 })),
            "the cycle arms at +1 s"
        );
        for round in 1..=3u8 {
            assert!(world.fire_tanaka_cycle(round - 1), "round {round} runs");
            assert_eq!(
                pirates(&world).len(),
                usize::from(round),
                "round {round} stands one"
            );
            assert!(
                pirates(&world).iter().all(|npc| npc.is_aggressive()),
                "every pirate is aggressive"
            );
            assert!(
                world.regens.due.iter().any(|(_, what)| matches!(
                    what,
                    super::super::regen::Due::TanakaCycle { step } if *step == round
                )),
                "round {round} arms the next"
            );
        }
        assert!(!world.fire_tanaka_cycle(3), "past round 3 the cycle stops");
        world.end_tanaka();
        assert_eq!(world.event_rate(b"tanaka_event"), 0);
    }
    /// No host, no cycle: the start arms nothing and the fire answers false,
    /// sparing the mapless deployment (`GetMap`'s null branch).
    #[test]
    fn without_a_host_the_start_arms_nothing_and_the_fire_runs_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock)
            .with_clients(Arc::new(ChannelClients::new()))
            .with_quests(quests())
            .with_tanaka(vec![mob_box()]);
        world.start_tanaka();
        assert_eq!(world.event_rate(b"tanaka_event"), 1, "the flag still rises");
        assert!(
            !world
                .regens
                .due
                .iter()
                .any(|(_, what)| matches!(what, super::super::regen::Due::TanakaCycle { .. })),
            "nothing arms"
        );
        assert!(!world.fire_tanaka_cycle(0), "the mapless fire runs nothing");
    }

    /// The end never cancels the cycle (`SetTanakaEvent(false)` has no cancel):
    /// a round armed before the end still fires after it.
    #[test]
    fn the_end_leaves_an_armed_round_firing() {
        let clock = TestClock::default();
        let mut world = tanaka_world(&clock, vec![mob_box()]);
        world.start_tanaka();
        world.end_tanaka();
        assert_eq!(world.event_rate(b"tanaka_event"), 0);
        assert!(world.fire_tanaka_cycle(0), "the armed round still runs");
        assert_eq!(pirates(&world).len(), 1, "its pirate still stands");
    }

    /// A point line spawns at its point, with the legacy rotation (`direction - 1`
    /// times 45) — and not aggressive, as legacy's point path passes none.
    #[test]
    fn a_point_line_stands_at_its_point() {
        let clock = TestClock::default();
        let mut entry = mob_box();
        entry.ex = entry.sx;
        entry.ey = entry.sy;
        entry.direction = 2;
        let at = (entry.sx, entry.sy);
        let mut world = tanaka_world(&clock, vec![entry]);
        assert!(world.fire_tanaka_cycle(0), "the round runs");
        let stood = pirates(&world);
        assert_eq!(stood.len(), 1, "the point stands one");
        assert_eq!((stood[0].x, stood[0].y), at, "it stands at its point");
        assert!(
            !stood[0].is_aggressive(),
            "and legacy's point takes no aggressive"
        );
    }

    #[test]
    fn the_owner_file_holds_mob_lines_only() {
        let bytes =
            include_bytes!("../../../legacy/gamedata/data/event/tanaka_event_regen_type_0.txt");
        let entries = gamedata::regen::parse(bytes).expect("the file parses");
        assert_eq!(entries.len(), 210, "two hundred ten lines");
        assert!(
            entries
                .iter()
                .all(|entry| entry.kind == gamedata::regen::RegenKind::Mob),
            "every line is a mob"
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.vnum == 5001 || entry.vnum == 5004),
            "every line names a Tanaka pirate"
        );
    }

    #[test]
    fn the_owner_file_places_onto_map_63() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe/map");
        let atlas =
            gamedata::map_atlas::MapAtlas::load(&dir).expect("the legacy map directory parses");
        let region = atlas.region(super::TANAKA_MAP).expect("map 63 loads");
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata")
            .join("data/event/tanaka_event_regen_type_0.txt");
        let entries =
            gamedata::regen::load_file(&file, region.sx, region.sy).expect("the file places");
        assert_eq!(entries.len(), 210, "every line places");
        assert!(
            entries
                .iter()
                .all(|entry| entry.sx >= region.sx && entry.sy >= region.sy),
            "every box sits past the base"
        );
    }
}
