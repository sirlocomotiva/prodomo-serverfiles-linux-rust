//! A world for the game state's own tests: a clock the test sets, one hosted map, and players
//! entered with loaded points and a body on that map.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use common::vid::Vid;
use gamedata::item_proto::ItemProtos;
use gamedata::server_attr::SectreeGrid;
use protocol::item_pos::ItemPos;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use world::character::{Points, PointsRow};
use world::item::Item;

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

/// A level 10 warrior's computed points on [`PLACE`], with `stamina`.
pub(super) fn points(stamina: i32) -> Points {
    let mut points = Points::load(&PointsRow {
        race: 0,
        level: 10,
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
        map_index: PLACE.1,
        part_base: 0,
        hair_part: 0,
        sash_part: 0,
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
    let (tx, inbox) = unbounded_channel();
    let name = format!("P{vid}");
    world
        .enter_world_with_items(Vid::new(vid), vid, &name, items, ClientOutbox::new(tx))
        .expect("the player enters");
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
