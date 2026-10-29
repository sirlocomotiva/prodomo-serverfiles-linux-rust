//! A world for the game state's own tests: a clock the test sets, one hosted map, and players
//! entered with loaded points and a body on that map.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use common::vid::Vid;
use gamedata::item_proto::ItemProtos;
use gamedata::map_atlas::MapRegion;
#[cfg(test)]
use gamedata::mob_group::MobGroups;
#[cfg(test)]
use gamedata::mob_locale_names::MobLocaleNames;
#[cfg(test)]
use gamedata::mob_proto::MobProtos;

use gamedata::server_attr::{CellBlock, SectreeGrid, ServerAttr, SECTREE_CELLS};
use protocol::item_pos::ItemPos;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use world::cells::{HostedCells, MapCells};
use world::character::{Points, PointsRow};
use world::item::Item;
#[cfg(test)]
use world::npc::{MapNpcs, NpcVids, Respawner, FIRST_NPC_VID};

use super::GameState;
use crate::client_live::LiveClock;
use crate::client_registry::ClientOutbox;
use crate::game_loop_messages::EnterPlace;
use crate::loading_phase::PcCard;

/// A clock the test reads and sets, shared with the world it is given to.
#[derive(Clone, Debug, Default)]
pub(super) struct TestClock(Arc<AtomicU32>);

impl TestClock {
    /// Sets `get_dword_time()` to `now`.
    pub(super) fn set(&self, now: u32) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl LiveClock for TestClock {
    fn now(&self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The Channel and map every fixture body stands on.
pub(super) const PLACE: (u8, i32) = (1, 41);

/// A world on `clock` with a 10,000 view range, hosting [`PLACE`] as a 10 by 10 sectree grid
/// from the origin.
pub(super) fn a_world(clock: &TestClock) -> GameState {
    let protos = ItemProtos::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto"),
    )
    .expect("the legacy item protos load");
    let mut world = GameState::new(protos)
        .with_clock(Box::new(clock.clone()))
        .with_view_range(10_000);
    let (channel, map) = PLACE;
    world.host_map(
        channel,
        map,
        SectreeGrid {
            x: 0,
            y: 0,
            columns: 10,
            rows: 10,
        },
    );
    world
}

/// [`PLACE`]'s region: one sectree from the origin.
pub(super) fn region() -> MapRegion {
    MapRegion {
        index: PLACE.1,
        name: b"test".to_vec(),
        sx: 0,
        sy: 0,
        ex: 6_400,
        ey: 6_400,
        spawn: (0, 0),
        empire_spawns: None,
    }
}

/// [`PLACE`]'s cells, all open.
pub(super) fn open_cells() -> Arc<HostedCells> {
    let block = CellBlock::from_cells(&vec![0; SECTREE_CELLS * SECTREE_CELLS]);
    let attr = ServerAttr::from_blocks(1, 1, vec![block]).unwrap();
    let grid = SectreeGrid::of(&region()).unwrap();
    let mut hosted = HostedCells::default();
    hosted.insert(PLACE.1, MapCells::new(grid, Arc::new(attr)));
    Arc::new(hosted)
}

/// A level 10 warrior's computed points on [`PLACE`], with `stamina`.
pub(super) fn points(stamina: i32) -> Points {
    points_at(10, stamina, 0)
}

/// A warrior's computed points on [`PLACE`] at `level`, with `stamina` and `exp`.
pub(super) fn points_at(level: u8, stamina: i32, exp: i64) -> Points {
    let mut points = Points::load(&PointsRow {
        race: 0,
        level,
        exp,
        conqueror_level: 0,
        st: 6,
        ht: 4,
        dx: 3,
        iq: 3,
        sungma: [0; 4],
        hp: 10_000,
        sp: 10_000,
        stamina,
        inven_point: 0,
        random_hp: 0,
        random_sp: 0,
        stat_point: 0,
        skill_point: 0,
        sub_skill_point: 0,
        level_step: 0,
        map_index: PLACE.1,
        part_base: 0,
        hair_part: 0,
        sash_part: 0,
        playtime: 0,
    });
    let _ = points.compute_points();
    points
}

/// Enters the player `vid` with `stamina` and shows its body at `(x, y)` on [`PLACE`]; the
/// receiver is its client's inbox.
pub(super) fn enter(
    world: &mut GameState,
    vid: u32,
    at: (i32, i32),
    stamina: i32,
) -> UnboundedReceiver<Vec<u8>> {
    enter_on(world, PLACE, vid, at, stamina)
}

/// As [`enter`], on `place`, a Channel and map the world hosts.
pub(super) fn enter_on(
    world: &mut GameState,
    place: (u8, i32),
    vid: u32,
    at: (i32, i32),
    stamina: i32,
) -> UnboundedReceiver<Vec<u8>> {
    enter_seeing(world, place, vid, at, stamina).0
}

/// As [`enter_on`], with the records the entrant's own client was sent as it was shown.
pub(super) fn enter_seeing(
    world: &mut GameState,
    place: (u8, i32),
    vid: u32,
    at: (i32, i32),
    stamina: i32,
) -> (UnboundedReceiver<Vec<u8>>, Vec<Vec<u8>>) {
    enter_seeing_with(world, place, vid, at, stamina, &[])
}

/// As [`enter_seeing`], with `items` the entrant holds, its worn items among them.
pub(super) fn enter_seeing_with(
    world: &mut GameState,
    (channel, map): (u8, i32),
    vid: u32,
    (x, y): (i32, i32),
    stamina: i32,
    items: &[(ItemPos, Item)],
) -> (UnboundedReceiver<Vec<u8>>, Vec<Vec<u8>>) {
    let (tx, mut inbox) = unbounded_channel();
    let name = format!("P{vid}");
    world
        .enter_world_with_items(Vid::new(vid), vid, &name, items, ClientOutbox::new(tx))
        .expect("the player enters");
    // The enter walk sent the WAR_LIST before the body was placed; drain it so the
    // tests read only what placing and their steps send.
    let _list = inbox.try_recv().expect("the enter WAR_LIST is queued");
    world
        .characters
        .find_by_vid_mut(Vid::new(vid))
        .expect("the player is in the world")
        .set_points(Some(points(stamina)));
    let card = PcCard {
        name,
        job: 0,
        empire: 1,
        level: 10,
        conqueror_level: 0,
        language: 0,
        pk_mode: crate::loading_phase::PK_MODE_PROTECT,
        skill_group: 0,
        alignment: 0,
    };
    let shown = world.place_body(
        Vid::new(vid),
        EnterPlace {
            channel,
            map,
            x,
            y,
            z: 0,
        },
        card,
    );
    (inbox, shown)
}

/// A monster with a real reach for the fight tests: race and battle type of `vnum`, stood at
/// `at` with `hp`, the given attack range, aggressive sight and AI flags.
#[cfg(test)]
pub(super) fn a_fighter(
    vid: u32,
    vnum: u32,
    battle_type: u8,
    reach: (u16, u16),
    ai_flags: u32,
    at: (i32, i32),
    hp: i32,
) -> world::npc::Npc {
    let mut fight = world::npc::MobFight::default();
    fight.battle_type = battle_type;
    fight.attack_range = reach.0;
    fight.aggressive_sight = reach.1;
    world::npc::Npc {
        vid,
        vnum,
        race: u16::try_from(vnum).unwrap_or(u16::MAX),
        char_type: gamedata::mob_proto::CHAR_TYPE_MONSTER,
        x: at.0,
        y: at.1,
        ai_flags,
        hp,
        max_hp: hp.max(1),
        fight,
        ..world::npc::Npc::default()
    }
}

/// The player's punch at `target`, as `fight`'s tests make it.
#[cfg(test)]
pub(super) fn hit(world: &mut GameState, vid: u32, target: u32) {
    world.attack(
        Vid::new(vid),
        protocol::cg_attack::CgAttack::new(0, target, 0, 0),
    );
}

/// A world on `clock` with `npcs` stood on [`PLACE`] over `cells`, where monsters wander
/// unless `no_wander` holds.
#[cfg(test)]
pub(super) fn stand_field(
    clock: &TestClock,
    npcs: Vec<world::npc::Npc>,
    cells: Arc<HostedCells>,
    no_wander: bool,
) -> GameState {
    let mut world = a_world(clock).with_no_wander(no_wander);
    let respawner = Respawner::new(
        Arc::new(MobProtos::from_rows(Vec::new())),
        Arc::new(MobGroups::default()),
        Arc::new(MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap()),
        NpcVids::starting_at(FIRST_NPC_VID + 100),
    );
    world.install_respawner(respawner, cells);
    world.stand_npcs(
        PLACE.0,
        &region(),
        MapNpcs {
            npcs,
            positions: Vec::new(),
            regens: Vec::new(),
        },
    );
    world
}
