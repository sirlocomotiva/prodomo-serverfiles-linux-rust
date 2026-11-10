//! `TempleOchao::CMgr` (`G/TempleOchao.h`, `G/TempleOchao.cpp`): the En-Tai guardian's
//! eleven rooms on map 209, the guardian's VID, the portal's VID, and the two timed
//! arms. One per Channel world, as each legacy core held its own.
//!
//! `Prepare` arms the rooms from the map's base and picks a room; `ClearPrepare`
//! spawns the guardian in another room and destroys the portal. The activity arm
//! runs every 30 s: past 180 s idle (600 s once attacked) the guardian moves rooms,
//! unless it holds a victim on 209. The kill arm spawns the portal where the killer
//! stands and, 60 s later, clears the prepare. The attack and kill hooks live at the
//! fight's `BeginFight` and the monster's `Dead` (`G/char_battle.cpp:309`,
//! `:1655`), both under `__VERSION_162__`, which the owner's build defines.

use super::GameState;

/// `TEMPLE_OCHAO_MAP_INDEX`.
pub(super) const TEMPLE_MAP: i32 = 209;
/// `TEMPLE_OCHAO_ROOMS`.
const ROOM_COUNT: u8 = 11;
/// `TEMPLE_OCHAO_GUARDIAN`.
pub(super) const GUARDIAN_VNUM: u32 = 6400;
/// `TEMPLE_OCHAO_PORTAL`.
const PORTAL_VNUM: u32 = 20415;
/// `TEMPLE_OCHAO_NO_ACTIVITY`, in seconds.
const NO_ACTIVITY_SECS: u64 = 180;
/// `TEMPLE_OCHAO_ATTACKED`, in seconds.
const ATTACKED_SECS: u64 = 600;
/// `TEMPLE_OCHAO_CHECK_ACTIVITY`, in seconds.
const CHECK_ACTIVITY_SECS: u64 = 30;
/// `TEMPLE_OCHAO_PORTAL_SHOW`, in seconds.
const PORTAL_SHOW_SECS: u64 = 60;
/// The room offsets in map units (`Prepare`'s `pos`, each `* 100`).
const ROOM_OFFSETS: [(i32, i32); 11] = [
    (193, 145),
    (123, 216),
    (224, 383),
    (348, 708),
    (375, 608),
    (430, 516),
    (444, 382),
    (388, 195),
    (446, 247),
    (592, 139),
    (646, 152),
];

/// The temple's rooms and VIDs: `m_rooms`, `i_room`, `dw_GuardianVID` and
/// `dw_PortalVID`, with the two event arms as Pulses.
#[derive(Debug, Default)]
pub(super) struct TempleOchao {
    /// The rooms' absolute points, empty until `Prepare` arms them.
    rooms: Vec<(i32, i32)>,
    /// `i_room`, 1-based once armed.
    room: u8,
    /// The guardian's VID, 0 with none.
    guardian: u32,
    /// The portal's VID, 0 with none.
    portal: u32,
    /// The Pulse the activity arm next runs on, `None` while disarmed.
    activity_due: Option<u64>,
    /// The Pulse the idle guardian moves rooms at.
    next_change: u64,
    /// Whether the guardian was attacked since it spawned.
    attacked: bool,
    /// The Pulse the portal clears at, `None` with no portal.
    portal_due: Option<u64>,
}

fn pulses(secs: u64) -> u64 {
    secs.saturating_mul(u64::from(crate::save::PASSES_PER_SEC))
}

impl GameState {
    /// `CMgr::Prepare`: with no rooms and a base for 209 on this world, the rooms arm
    /// from the base, a room is picked, the VIDs clear, and the prepare clears (the
    /// guardian spawns). A second call is quiet, as `m_rooms` non-empty is.
    pub(super) fn prepare_temple_ochao(&mut self) {
        if !self.temple_ochao.rooms.is_empty() {
            return;
        }
        let Some(place) = self.places_hosting(TEMPLE_MAP).first().copied() else {
            return;
        };
        let Some((bx, by)) = self.map_base(place) else {
            return;
        };
        let rooms: Vec<(i32, i32)> = ROOM_OFFSETS
            .iter()
            .map(|(dx, dy)| (bx + dx * 100, by + dy * 100))
            .collect();
        self.temple_ochao.rooms = rooms;
        let room = world::character::number(&mut self.dice, 1, i32::from(ROOM_COUNT));
        self.temple_ochao.room = u8::try_from(room).unwrap_or(1).clamp(1, ROOM_COUNT);
        self.temple_ochao.guardian = 0;
        self.temple_ochao.portal = 0;
        self.clear_temple_prepare(place);
    }

    /// `CMgr::ClearPrepare`: the guardian spawns and the old portal on 209 is
    /// destroyed. The portal arm disarms: its event ended when it fired.
    fn clear_temple_prepare(&mut self, place: (u8, i32)) {
        self.temple_ochao.portal_due = None;
        self.spawn_temple_guardian(place);
        let portal = self.temple_ochao.portal;
        if portal != 0 && self.npc_map_of(portal) == Some(TEMPLE_MAP) {
            self.destroy_npc(portal);
        }
        self.temple_ochao.portal = 0;
    }

    /// `CMgr::Spawn` and `RandomRoom`: one draw picks another room (a draw of the
    /// current room keeps it), the guardian spawns there, the old guardian on 209 is
    /// destroyed, and the activity arm starts.
    fn spawn_temple_guardian(&mut self, place: (u8, i32)) {
        let drawn = world::character::number(&mut self.dice, 1, i32::from(ROOM_COUNT));
        if let Ok(drawn) = u8::try_from(drawn) {
            if drawn != self.temple_ochao.room {
                self.temple_ochao.room = drawn.clamp(1, ROOM_COUNT);
            }
        }
        let at = usize::from(self.temple_ochao.room.saturating_sub(1));
        let Some(&point) = self.temple_ochao.rooms.get(at) else {
            return;
        };
        let stood = self.spawn_event_mob(place, GUARDIAN_VNUM, point, -1, true);
        let old = self.temple_ochao.guardian;
        if old != 0 && self.npc_map_of(old) == Some(TEMPLE_MAP) {
            self.destroy_npc(old);
        }
        // A failed spawn clears the VID, as legacy's null return skips only the store.
        self.temple_ochao.guardian = stood.unwrap_or(0);
        let now = self.last_pulse;
        self.temple_ochao.activity_due = Some(now.saturating_add(pulses(CHECK_ACTIVITY_SECS)));
        self.temple_ochao.next_change = now.saturating_add(pulses(NO_ACTIVITY_SECS));
        self.temple_ochao.attacked = false;
    }

    /// The map the NPC under `vid` stands on, when one does.
    fn npc_map_of(&self, vid: u32) -> Option<i32> {
        self.npc_of.get(&vid).map(|&(_, map, _)| map)
    }

    /// `CMgr::GuardianAttacked`: the first attack since the spawn pushes the move 600 s
    /// out. Quiet with the activity arm disarmed.
    pub(super) fn temple_guardian_attacked(&mut self, victim: u32) {
        if victim != self.temple_ochao.guardian || self.temple_ochao.activity_due.is_none() {
            return;
        }
        if !self.temple_ochao.attacked {
            self.temple_ochao.next_change = self.last_pulse.saturating_add(pulses(ATTACKED_SECS));
            self.temple_ochao.attacked = true;
        }
    }

    /// `CMgr::OnGuardianKilled`: the portal spawns where the killer stands and the
    /// prepare clears 60 s later.
    pub(super) fn temple_guardian_killed(&mut self, place: (u8, i32), at: (i32, i32)) {
        let stood = self.spawn_event_mob(place, PORTAL_VNUM, at, -1, true);
        self.temple_ochao.portal = stood.unwrap_or(0);
        self.temple_ochao.portal_due =
            Some(self.last_pulse.saturating_add(pulses(PORTAL_SHOW_SECS)));
    }

    /// The temple's Pulse arms: the portal clears at its Pulse, and the activity arm
    /// moves an idle guardian rooms, unless it holds a victim on 209. Both re-arm as
    /// legacy's events do.
    pub(super) fn run_temple_ochao(&mut self, pulse: u64) {
        if self.temple_ochao.rooms.is_empty() {
            return;
        }
        if self.temple_ochao.portal_due.is_some_and(|due| pulse >= due) {
            if let Some(place) = self.places_hosting(TEMPLE_MAP).first().copied() {
                self.clear_temple_prepare(place);
            } else {
                self.temple_ochao.portal_due = None;
            }
        }
        let Some(due) = self.temple_ochao.activity_due else {
            return;
        };
        if pulse < due {
            return;
        }
        self.temple_ochao.activity_due = Some(pulse.saturating_add(pulses(CHECK_ACTIVITY_SECS)));
        if self.last_pulse < self.temple_ochao.next_change {
            return;
        }
        self.temple_ochao.next_change = self.last_pulse.saturating_add(pulses(NO_ACTIVITY_SECS));
        self.change_temple_room();
    }

    /// `CMgr::ChangeRoom`: an idle guardian with no victim on 209 moves rooms. One
    /// holding a victim stays: the activity arm already re-armed.
    fn change_temple_room(&mut self) {
        let guardian = self.temple_ochao.guardian;
        if guardian != 0
            && self.npc_map_of(guardian) == Some(TEMPLE_MAP)
            && self
                .minds
                .get(guardian)
                .is_some_and(|mind| mind.victim.is_some())
        {
            return;
        }
        self.temple_ochao.activity_due = None;
        if let Some(place) = self.places_hosting(TEMPLE_MAP).first().copied() {
            self.spawn_temple_guardian(place);
        }
    }

    /// The kill arm's read: a killer standing on 209 raises the portal at its spot.
    pub(super) fn raise_temple_portal(&mut self, killer: Option<common::vid::Vid>) {
        let Some(killer) = killer else {
            return;
        };
        let Some(body) = self.bodies.get(&killer) else {
            return;
        };
        if body.map != TEMPLE_MAP {
            return;
        }
        let place = (body.channel, body.map);
        let Some(spot) = self.spot_of(killer.raw()) else {
            return;
        };
        self.temple_guardian_killed(place, (spot.x, spot.y));
    }
}

#[cfg(test)]
mod tests {
    use super::{GUARDIAN_VNUM, PORTAL_VNUM, TEMPLE_MAP};
    use crate::game_state::fixtures::{a_fighter, a_world, enter_on, hit, points_at, TestClock};
    use common::vid::Vid;
    use gamedata::map_atlas::MapRegion;
    use gamedata::mob_proto::{MobProto, MobProtos};
    use gamedata::server_attr::{CellBlock, SectreeGrid, ServerAttr, SECTREE_CELLS};
    use std::sync::Arc;
    use world::cells::{HostedCells, MapCells};
    use world::npc::{MapNpcs, NpcVids, Respawner, FIRST_NPC_VID};

    /// The owner's map 209 base (`Setting.txt`).
    const BASE: (i32, i32) = (844_800, 1_408_000);
    /// The bodyguard whose kill drops the temple's table (`temple_of_the_ochao.lua:7`).
    const BODYGUARD_VNUM: u32 = 6311;

    /// A world hosting map 209 on Channel 1 as twelve sectrees square from the owner's
    /// base, all cells open, whose respawner knows the guardian and the portal.
    fn temple_world(clock: &TestClock) -> crate::game_state::GameState {
        let mut world = a_world(clock);
        let region = MapRegion {
            index: TEMPLE_MAP,
            name: b"test".to_vec(),
            sx: BASE.0,
            sy: BASE.1,
            ex: BASE.0 + 12 * 6_400,
            ey: BASE.1 + 12 * 6_400,
            spawn: (0, 0),
            empire_spawns: None,
        };
        let grid = SectreeGrid::of(&region).expect("the region is whole sectrees");
        world.host_map(1, TEMPLE_MAP, grid);
        let blocks = (0..144)
            .map(|_| CellBlock::from_cells(&vec![0; SECTREE_CELLS * SECTREE_CELLS]))
            .collect();
        let attr = ServerAttr::from_blocks(12, 12, blocks).expect("144 blocks");
        let mut hosted = HostedCells::default();
        hosted.insert(TEMPLE_MAP, MapCells::new(grid, Arc::new(attr)));
        let rows = [GUARDIAN_VNUM, PORTAL_VNUM]
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

    /// The points of every standing character of `race` on map 209.
    fn standing(world: &crate::game_state::GameState, race: u32) -> Vec<(i32, i32)> {
        world.npcs[&(1, TEMPLE_MAP)]
            .npcs
            .iter()
            .filter(|npc| u32::from(npc.race) == race)
            .map(|npc| (npc.x, npc.y))
            .collect()
    }

    /// The prepare stands the guardian in a drawn room, records its VID and room, and
    /// arms the activity check 30 s out; a second prepare is quiet.
    #[test]
    fn the_prepare_stands_the_guardian_in_a_drawn_room() {
        let clock = TestClock::default();
        let mut world = temple_world(&clock);
        world.prepare_temple_ochao();
        let stood = standing(&world, GUARDIAN_VNUM);
        assert_eq!(stood.len(), 1);
        let rooms: Vec<(i32, i32)> = super::ROOM_OFFSETS
            .iter()
            .map(|(dx, dy)| (BASE.0 + dx * 100, BASE.1 + dy * 100))
            .collect();
        assert!(rooms.contains(&stood[0]), "{stood:?}");
        assert!((1..=11).contains(&world.temple_ochao.room));
        let guardian = world.temple_ochao.guardian;
        assert_ne!(guardian, 0);
        assert_eq!(
            world.temple_ochao.activity_due,
            Some(30 * u64::from(crate::save::PASSES_PER_SEC))
        );
        let guardian = world.temple_ochao.guardian;
        world.prepare_temple_ochao();
        assert_eq!(world.temple_ochao.guardian, guardian);
        assert_eq!(standing(&world, GUARDIAN_VNUM), stood);
    }

    /// The attack arm touches only the temple's guardian: its next change moves 600 s
    /// out, while another VID changes nothing.
    #[test]
    fn the_attack_arm_pushes_only_the_guardians_change() {
        let clock = TestClock::default();
        let mut world = temple_world(&clock);
        world.prepare_temple_ochao();
        let guardian = world.temple_ochao.guardian;
        world.temple_guardian_attacked(guardian + 1);
        assert!(!world.temple_ochao.attacked);
        world.temple_guardian_attacked(guardian);
        assert!(world.temple_ochao.attacked);
        assert_eq!(
            world.temple_ochao.next_change,
            600 * u64::from(crate::save::PASSES_PER_SEC)
        );
    }

    /// Past the activity check with no victim, the guardian is destroyed and a new one
    /// stands in another room; with a victim on 209 the room holds.
    #[test]
    fn the_idle_guardian_changes_room_and_the_busy_one_holds() {
        let clock = TestClock::default();
        let mut world = temple_world(&clock);
        world.prepare_temple_ochao();
        let guardian = world.temple_ochao.guardian;
        // The check fires every 30 s but moves only past the idle 180 s.
        let check = world.temple_ochao.activity_due.expect("armed");
        world.last_pulse = check;
        world.run_temple_ochao(check);
        assert_eq!(world.temple_ochao.guardian, guardian);
        let idle = world.temple_ochao.next_change;
        world.last_pulse = idle;
        world.run_temple_ochao(idle);
        assert_eq!(standing(&world, GUARDIAN_VNUM).len(), 1);
        assert_ne!(world.temple_ochao.guardian, guardian);
        assert_ne!(world.temple_ochao.guardian, 0);
        // A victim holds the room: the mind fights, so the check re-arms instead.
        let holder = world.temple_ochao.guardian;
        let fresh = crate::game_state::mob_ai::Mind::new(world.npc_by_vid(holder).expect("stood"));
        world.minds.ensure(holder, fresh);
        world.set_victim(holder, Some(Vid::new(0x6001)));
        let before = standing(&world, GUARDIAN_VNUM);
        let idle = world.temple_ochao.next_change;
        world.last_pulse = idle;
        world.run_temple_ochao(idle);
        assert_eq!(world.temple_ochao.guardian, holder);
        assert_eq!(standing(&world, GUARDIAN_VNUM), before);
    }

    /// The guardian's death at a killer standing on 209 raises the portal where the
    /// killer stands for 60 s; a killer elsewhere raises nothing.
    #[test]
    fn the_kill_raises_the_portal_where_the_killer_stands() {
        let clock = TestClock::default();
        let mut world = temple_world(&clock);
        world.prepare_temple_ochao();
        let guardian = world.temple_ochao.guardian;
        let place = (1, TEMPLE_MAP);
        let mut inbox = enter_on(
            &mut world,
            place,
            0x6001,
            (BASE.0 + 19_400, BASE.1 + 14_600),
            0,
        );
        world.dead_npc(guardian, Some(Vid::new(0x6001)));
        assert_eq!(
            standing(&world, PORTAL_VNUM),
            [(BASE.0 + 19_400, BASE.1 + 14_600)]
        );
        let due = world.temple_ochao.portal_due.expect("shown");
        assert_eq!(due, 60 * u64::from(crate::save::PASSES_PER_SEC));
        world.run_temple_ochao(due);
        assert!(standing(&world, PORTAL_VNUM).is_empty());
        while inbox.try_recv().is_ok() {}
    }

    /// A level-95 killer's punches stun and kill a 1-HP bodyguard stood beside it on
    /// 209: the kill's quest event runs the temple's table and lays what passes at
    /// the killer's feet, each owned by the killer for 300 s
    /// (`G/questlua_game.cpp:205-206`). Seed 5636 (found offline): both punches' chats miss,
    /// the ten script draws pass eight rows (not 21075's no-proto row or 70043's
    /// unported unique), and each laid row draws two scatter legs.
    #[test]
    fn a_bodyguard_kill_on_the_temple_lays_the_scripts_passing_rows() {
        use gamedata::mob_locale_names::MobNamesByLanguage;
        use quest::manager::Manager;
        use world::character::Pcg32;
        use world::item::ItemIdRange;
        use world::npc::Npc;
        const BODYGUARD: u32 = FIRST_NPC_VID;
        const KILLER: u32 = 0x6001;
        const AT: (i32, i32) = (BASE.0 + 19_400, BASE.1 + 14_600);
        let clock = TestClock::default();
        let mut world = temple_world(&clock);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        let quests = crate::game_state::quests::Quests::new(
            manager,
            Arc::default(),
            MobNamesByLanguage::default(),
        );
        world = world.with_quests(quests);
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 1).expect("a usable range"))
            .expect("the first install");
        let mut bodyguard: Npc = a_fighter(BODYGUARD, BODYGUARD_VNUM, 0, (175, 0), 0, AT, 1);
        bodyguard.exp = 10;
        bodyguard.fight.level = 108;
        let region = gamedata::map_atlas::MapRegion {
            index: TEMPLE_MAP,
            name: b"test".to_vec(),
            sx: BASE.0,
            sy: BASE.1,
            ex: BASE.0 + 12 * 6_400,
            ey: BASE.1 + 12 * 6_400,
            spawn: (0, 0),
            empire_spawns: None,
        };
        world.stand_npcs(
            1,
            &region,
            MapNpcs {
                npcs: vec![bodyguard],
                positions: Vec::new(),
                regens: Vec::new(),
            },
        );
        let mut inbox = enter_on(&mut world, (1, TEMPLE_MAP), KILLER, AT, 0);
        world
            .characters
            .find_by_vid_mut(Vid::new(KILLER))
            .expect("the killer stands")
            .set_points(Some(points_at(95, 0, 0)));
        world
            .quests
            .as_mut()
            .expect("quests loaded")
            .manager_mut()
            .load_flags(KILLER, Vec::new());
        world.dice = Pcg32::new(5636, 0);
        world.target(Vid::new(KILLER), BODYGUARD);
        hit(&mut world, KILLER, BODYGUARD);
        while inbox.try_recv().is_ok() {}
        hit(&mut world, KILLER, BODYGUARD);
        while inbox.try_recv().is_ok() {}
        let mut laid: Vec<(u32, u32)> = world
            .ground
            .values()
            .map(|lying| {
                (
                    lying.ground.item.vnum,
                    lying.owner.as_ref().map_or(0, |owner| owner.pid),
                )
            })
            .collect();
        laid.sort_unstable();
        assert_eq!(
            laid,
            [
                (2145, KILLER),
                (11295, KILLER),
                (11495, KILLER),
                (11695, KILLER),
                (11895, KILLER),
                (30523, KILLER),
                (30609, KILLER),
                (30612, KILLER),
            ],
            "eight rows pass; the no-proto and unported rows lay nothing"
        );
        for lying in world.ground.values() {
            assert_eq!(lying.ground.item.count(), 1, "one of each");
            assert_eq!(
                lying.owner.as_ref().map(|owner| owner.due),
                Some(300 * u64::from(crate::save::PASSES_PER_SEC)),
                "owned for 300 s"
            );
        }
    }
}
