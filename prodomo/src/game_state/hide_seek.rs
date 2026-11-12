//! The hide-and-seek event (`CEventManager`'s 10, `G/event_manager.cpp:1204-1320`):
//! the Old Box (20359) hides on one of four maps, a click on it pays five 50109 gift
//! boxes and moves it, and five rounds end the event.
//!
//! The start raises `hide_seek_event`, sets the round to 5 (`HIDE_AND_SEEK_ROUNDS`),
//! chats the start and hides the box (`SetHideAndSeekEvent`, `:1204-1225`). The box
//! stands through `HideAndSeekNPC`: the old box is destroyed first, a negative
//! position only removes it, round 0 or less ends the event through `SetEventState`,
//! else the round falls by one and the box spawns on the position's map at one of
//! its four spots (`SpawnMob`, base plus the spot times 100), its VID stamped to
//! `hide_seek_vid` and the spot's hint noticed. A position whose map no Channel
//! hosts stands nothing (`map_allow_find`, `:1292-1293`), and the P2P fan to the
//! other cores has no equivalent: the one process stands or removes the box itself.
//!
//! A click on the box's VID while the event runs pays the reward first
//! (`CHARACTER::OnClick`, `G/char.cpp:6297-6329`): five 50109 through `AutoGiveItem`,
//! and only when the give lands does the box move to a new drawn position with the
//! current round. The click runs before the quest click, as legacy's arm runs before
//! `CQuestManager::Click`. The five the call asks for land as one: the gift box is not
//! stackable, so `CreateItem` clamps the count to 1 (`G/item_manager.cpp:261-262`).
//!
//! The end chats the end, zeroes `hide_seek_vid`, clears the round and removes the
//! box (`:1227-1242`). The four maps (61-64) are all hosted on Channel 1, and the
//! spawn's own Channel gate (`HIDE_AND_SEEK_CHANNEL`, `G/input_p2p.cpp:784-792`)
//! answers Channel 1's map, as the GG fan is retired (ADR-0001).

use common::vid::Vid;

use super::GameState;
use crate::item_move::Mover;

/// The Old Box (`HIDE_AND_SEEK_NPC`, `G/event_manager.h:48`).
pub(super) const HIDE_BOX: u32 = 20_359;
/// The reward (`HIDE_AND_SEEK_REWARD_VNUM`, `:49`): five gift boxes.
const HIDE_REWARD: u32 = 50_109;
/// How many rewards a found box pays (`HIDE_AND_SEEK_REWARD_COUNT`, `:50`).
const HIDE_REWARD_COUNT_U16: u16 = 5;
/// The rounds a start sets (`HIDE_AND_SEEK_ROUNDS`, `:46`).
const HIDE_ROUNDS: i32 = 5;
/// One hiding map's index, its four spots in metres from the base, and their hints.
type HideMap<'a> = (i32, [(i32, i32); 4], [&'a str; 4]);

/// The box's spots and hints, by position (`aNPCPosition`, `:26-60`).
const HIDE_SPOTS: [HideMap<'_>; 4] = [
    (
        61,
        [(238, 1427), (819, 121), (1425, 1011), (1043, 693)],
        [
            "Mount Sohan",
            "Mount Sohan 2",
            "Mount Sohan 3",
            "Mount Sohan 4",
        ],
    ),
    (
        62,
        [(1044, 980), (798, 806), (1032, 668), (805, 1168)],
        ["Fireland", "Fireland 2", "Fireland 3", "Fireland 4"],
    ),
    (
        63,
        [(821, 787), (435, 703), (698, 428), (1485, 692)],
        [
            "Sahara Desert",
            "Sahara Desert 2",
            "Sahara Desert 3",
            "Sahara Desert 4",
        ],
    ),
    (
        64,
        [(184, 121), (1225, 279), (130, 1219), (1438, 1208)],
        [
            "Valley of Seungryong",
            "Valley of Seungryong 2",
            "Valley of Seungryong 3",
            "Valley of Seungryong 4",
        ],
    ),
];

/// The event's kept box: `m_pHideAndSeekNPC` and `m_iCurrentHideAndSeekRound`
/// (`G/event_manager.h:150-151`).
#[derive(Debug, Default)]
pub(super) struct HideAndSeek {
    /// The standing box's VID, if any.
    boxed: Option<u32>,
}

impl GameState {
    /// `SetHideAndSeekEvent(true)` (`:1204-1225`): the flag rises, the round takes 5,
    /// the start chats and the box hides at a drawn position.
    pub(super) fn start_hide_and_seek(&mut self) {
        self.set_event_flag(b"hide_seek_event", 1);
        self.set_event_flag(b"hide_seek_round", HIDE_ROUNDS);
        self.broadcast_notice_text(
            b"Hide and Seek Event has started. Search for the Old Box to receive a reward!",
        );
        let position = world::character::number(&mut self.dice, 0, 3);
        self.hide_box(position, HIDE_ROUNDS);
    }

    /// `SetHideAndSeekEvent(false)` (`:1227-1242`): the flag falls first
    /// (`UpdateGameFlag` at the top of the arm), then the end chats, `hide_seek_vid`
    /// zeroes, the round clears and the box is removed.
    pub(super) fn end_hide_and_seek(&mut self) {
        self.set_event_flag(b"hide_seek_event", 0);
        self.broadcast_notice_text(b"Hide and Seek Event has ended. Thank you for participating!");
        self.set_event_flag(b"hide_seek_vid", 0);
        self.set_event_flag(b"hide_seek_round", 0);
        self.hide_box(-1, 0);
    }

    /// `HideAndSeekNPC(position, round)` (`:1247-1320`): the old box is destroyed,
    /// a negative position removes only, round 0 or less ends the event, else the
    /// round falls and the box spawns on Channel 1's map at a drawn spot, its VID
    /// stamped and its hint noticed. Answers whether the box stands or the call
    /// only removed or ended it.
    pub(super) fn hide_box(&mut self, position: i32, round: i32) -> bool {
        if let Some(boxed) = self.hide_and_seek.boxed.take() {
            self.destroy_npc(boxed);
        }
        if position < 0 {
            return true;
        }
        if round <= 0 {
            self.end_hide_and_seek_inner();
            return true;
        }
        self.set_event_flag(b"hide_seek_round", round - 1);
        let Some((map, spots, hints)) = usize::try_from(position)
            .ok()
            .and_then(|slot| HIDE_SPOTS.get(slot))
        else {
            return false;
        };
        let spot = world::character::number(&mut self.dice, 0, 3);
        let drawn = usize::try_from(spot).unwrap_or(0);
        let (Some(at), Some(hint)) = (spots.get(drawn), hints.get(drawn)) else {
            return false;
        };
        let Some((bx, by)) = self.map_base((1, *map)) else {
            return false;
        };
        let stood = self.spawn_event_mob(
            (1, *map),
            HIDE_BOX,
            (bx + at.0 * 100, by + at.1 * 100),
            0,
            false,
        );
        let Some(vid) = stood else {
            return false;
        };
        self.hide_and_seek.boxed = Some(vid);
        // `RequestSetEventFlag("hide_seek_vid", vid)`: the DWORD wraps into the
        // flag's `int`, as NPC VIDs start past `i32::MAX`.
        self.set_event_flag(b"hide_seek_vid", i32::from_ne_bytes(vid.to_ne_bytes()));
        self.broadcast_notice_text(hint.as_bytes());
        true
    }

    /// The event's end from inside `hide_box` (`SetEventState`, `:1270-1276`):
    /// the flag and the round clear without the end chat or the box removal, as
    /// the caller already removed the box.
    fn end_hide_and_seek_inner(&mut self) {
        self.set_event_flag(b"hide_seek_event", 0);
        self.set_event_flag(b"hide_seek_round", 0);
    }

    /// `CHARACTER::OnClick`'s hide-and-seek arm (`G/char.cpp:6297-6329`): when the
    /// event runs and `target` is the box's VID, five 50109 through `AutoGiveItem`,
    /// and only when the give lands does the box move to a new drawn position with
    /// the current round. Answers the move when the click was the box's and the
    /// reward landed.
    pub(super) fn hide_box_click(
        &mut self,
        vid: Vid,
        target: u32,
        mover: Mover,
    ) -> Option<crate::item_move::MovedItems> {
        if self.manager_event_flag(b"hide_seek_event") == 0 {
            return None;
        }
        if self.manager_event_flag(b"hide_seek_vid") != i32::from_ne_bytes(target.to_ne_bytes()) {
            return None;
        }
        let questing = self.quest_running(vid);
        let (outcome, given) =
            self.auto_give(vid, (HIDE_REWARD, HIDE_REWARD_COUNT_U16), -1, questing)?;
        if let world::character::AutoGiven::Unplaced(item) = outcome {
            self.lay_auto_given(vid, item);
        }
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        let reward = crate::item_move::MovedItems::new(
            pid,
            vid.raw(),
            given,
            mover,
            &self.protos,
            &self.locale,
        );
        let round = self.manager_event_flag(b"hide_seek_round");
        let position = world::character::number(&mut self.dice, 0, 3);
        self.hide_box(position, round);
        Some(reward)
    }
}

#[cfg(test)]
mod tests {
    use super::{HIDE_BOX, HIDE_REWARD};
    use crate::client_registry::ChannelClients;
    use crate::game_state::fixtures::{a_world, enter_on, TestClock};
    use common::vid::Vid;
    use gamedata::map_atlas::MapRegion;
    use gamedata::mob_proto::{MobProto, MobProtos};
    use gamedata::server_attr::{CellBlock, SectreeGrid, ServerAttr, SECTREE_CELLS};
    use std::sync::Arc;
    use world::cells::{HostedCells, MapCells};
    use world::npc::{MapNpcs, NpcVids, Respawner, FIRST_NPC_VID};

    /// The owner's map 61 base (`Setting.txt`).
    const BASE: (i32, i32) = (0, 0);

    /// A world hosting map 61 on Channel 1, whose respawner knows the box.
    fn box_world(clock: &TestClock) -> crate::game_state::GameState {
        let mut world = a_world(clock)
            .with_clients(Arc::new(ChannelClients::new()))
            .with_quests(quests());
        world
            .install_item_ids(world::item::ItemIdRange::new(1, 1_000_000, 1).expect("a range"))
            .expect("no ids yet");
        let region = MapRegion {
            index: 61,
            name: b"test".to_vec(),
            sx: BASE.0,
            sy: BASE.1,
            ex: BASE.0 + 24 * 6_400,
            ey: BASE.1 + 24 * 6_400,
            spawn: (0, 0),
            empire_spawns: None,
        };
        let grid = SectreeGrid::of(&region).expect("the region is whole sectrees");
        world.host_map(1, 61, grid);
        let blocks = (0..576)
            .map(|_| CellBlock::from_cells(&vec![0; SECTREE_CELLS * SECTREE_CELLS]))
            .collect();
        let attr = ServerAttr::from_blocks(24, 24, blocks).expect("576 blocks");
        let mut hosted = HostedCells::default();
        hosted.insert(61, MapCells::new(grid, Arc::new(attr)));
        let rows = [MobProto {
            line: 2,
            table: gamedata::records::MobTableRecord {
                vnum: HIDE_BOX,
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

    /// The standing boxes on map 61: their VIDs and spots.
    fn boxes(world: &crate::game_state::GameState) -> Vec<(u32, i32, i32)> {
        world.npcs[&(1, 61)]
            .npcs
            .iter()
            .filter(|npc| u32::from(npc.race) == HIDE_BOX && !npc.dead)
            .map(|npc| (npc.vid, npc.x, npc.y))
            .collect()
    }

    /// The text of a `GC_CHAT` line.
    fn text_of(line: &[u8]) -> Vec<u8> {
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    /// The start raises the flag, sets the round to 5, chats the start and hides
    /// the box on one of the four maps; the end falls the flag, chats the end,
    /// zeroes the VID, clears the round and removes the box.
    #[test]
    fn the_start_hides_the_box_and_the_end_removes_it() {
        let clock = TestClock::default();
        let mut world = box_world(&clock);
        let mut inbox = enter_on(&mut world, (1, 61), 0x6001, (6400, 6400), 0);
        while inbox.try_recv().is_ok() {}
        world.start_game_event(10, 0, 0);
        assert_eq!(world.event_rate(b"hide_seek_event"), 1);
        assert_eq!(world.event_rate(b"hide_seek_round"), 4);
        let stood = boxes(&world);
        assert!(stood.len() <= 1, "at most one box stands: {stood:?}");
        let mut frames = Vec::new();
        while let Ok(frame) = inbox.try_recv() {
            frames.push(frame);
        }
        let texts: Vec<Vec<u8>> = frames.iter().map(|frame| text_of(frame)).collect();
        assert_eq!(
            texts[0],
            b"Hide and Seek Event has started. Search for the Old Box to receive a reward!",
            "the start chats first: {texts:?}"
        );
        world.end_game_event(10);
        assert_eq!(world.event_rate(b"hide_seek_event"), 0);
        assert_eq!(world.event_rate(b"hide_seek_vid"), 0);
        assert_eq!(world.event_rate(b"hide_seek_round"), 0);
        assert!(boxes(&world).is_empty(), "the end removes the box");
        let _ = HIDE_REWARD;
    }

    /// `hide_box(0, 5)` stands the box on map 61 at one of the four spots, stamps
    /// its VID and notices the spot's hint; a second call moves it (the old box is
    /// gone); round 0 ends the event without the end chat.
    #[test]
    fn the_box_stands_moves_and_ends_on_round_zero() {
        let clock = TestClock::default();
        let mut world = box_world(&clock);
        let mut inbox = enter_on(&mut world, (1, 61), 0x6001, (6400, 6400), 0);
        while inbox.try_recv().is_ok() {}
        world.set_event_flag(b"hide_seek_event", 1);
        assert!(world.hide_box(0, 5), "position 0 stands on map 61");
        let stood = boxes(&world);
        assert_eq!(stood.len(), 1, "one box stands: {stood:?}");
        let (vid, x, y) = stood[0];
        assert_eq!(
            world.event_rate(b"hide_seek_vid"),
            i32::from_ne_bytes(vid.to_ne_bytes())
        );
        assert_eq!(world.event_rate(b"hide_seek_round"), 4);
        let spots = [
            (23_800, 142_700),
            (81_900, 12_100),
            (142_500, 101_100),
            (104_300, 69_300),
        ];
        assert!(spots.contains(&(x, y)), "a listed spot: ({x}, {y})");
        let hint = text_of(&inbox.try_recv().expect("the hint"));
        assert!(
            hint.starts_with(b"Mount Sohan"),
            "the spot's hint: {hint:?}"
        );
        assert!(!world.hide_box(1, 4), "map 62 stands nothing here");
        assert!(boxes(&world).is_empty(), "the old box is gone");
        assert_eq!(world.event_rate(b"hide_seek_round"), 3);
        world.hide_box(0, 0);
        assert_eq!(world.event_rate(b"hide_seek_event"), 0);
        assert_eq!(world.event_rate(b"hide_seek_round"), 0);
    }

    /// A click on the box's VID pays the 50109 (once: the gift box is not
    /// stackable) and moves the box with the current round; a click on another VID
    /// pays nothing and moves nothing.
    #[test]
    fn the_box_click_pays_five_and_moves_the_box() {
        let clock = TestClock::default();
        let mut world = box_world(&clock);
        let mut inbox = enter_on(&mut world, (1, 61), 0x6001, (6400, 6400), 0);
        while inbox.try_recv().is_ok() {}
        world.set_event_flag(b"hide_seek_event", 1);
        assert!(world.hide_box(0, 5), "the box stands");
        let (vid, _, _) = boxes(&world)[0];
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(0x6001)).expect("the body stands");
        assert!(
            world
                .hide_box_click(Vid::new(0x6001), vid + 1, mover)
                .is_none(),
            "another VID pays nothing"
        );
        assert_eq!(boxes(&world).len(), 1, "another VID moves nothing");
        let reward = world
            .hide_box_click(Vid::new(0x6001), vid, mover)
            .expect("the box pays");
        assert_eq!(
            world.count_specify_item(Vid::new(0x6001), HIDE_REWARD),
            1,
            "the unstackable gift box lands once"
        );
        assert!(!reward.records.is_empty(), "the give packs records");
        let _ = &mut inbox;
    }
}
