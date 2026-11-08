//! Fishing: `CG_FISHING` (`CInputMain::Fishing`, `G/input_main.cpp:3166-3172`), the fishing
//! event (`G/fishing.cpp:464-537`), the take (`Take`, `:596-745`), the bait
//! (`case USE_BAIT`, `G/char_item.cpp:6800-6825`) and the campfire (`case ITEM_CAMPFIRE`,
//! `:2891-2947`, `kill_campfire_event`, `:2047-2066`).
//!
//! A `CG_FISHING` turns the character to `dir * 5` degrees, then runs `CHARACTER::fishing`
//! (`G/char.cpp:5248-5298`). While the character's fishing event runs the record is the
//! take (`fishing_take`, `:5300-5320`): with a rod still in the weapon cell `Take` runs, else
//! `[LS;819]`, and the event ends either way. Otherwise a blocked cell answers `[LS;1051]`,
//! a weapon cell without a rod `[LS;817]`, a rod without bait `[LS;818]`, and else the cast
//! starts: `GC_FISHING` `START` reaches the viewers and the character with its direction,
//! and the event fires `number(10, 40)` seconds later.
//!
//! The event's first firing is the bite: it stamps the hang time, picks the fish
//! (`DetermineFish`, `:361-401`), sends `REACT` around, and, to a character who knows the
//! fish (`PredictFish`, `:451-462`: the fish mind pill's affect or a fish mind unique item;
//! the premium is not ported, so it never holds), sends `FISH` with the fish's vnum to its
//! own client. It fires again 6 seconds later: the fish got away, `FAIL` goes around, the
//! bait is gone (socket 2 of the rod is 0) and the event ends. A firing that finds no rod
//! in the weapon cell ends the event silently.
//!
//! The take before the bite sends `STOP` around. After the bite it rolls `Compute`
//! (`:549-576`) with the time since the bite and the fishing level (the bait's level in
//! socket 2 plus the rod's `value0`): a miss sends `FAIL` around; a catch sends `SUCCESS`
//! around, gives the fish event boxes while the `enable_fish_event` flag is set
//! (`ENABLE_FISH_EVENT` is defined in `C/prodomodefines.h:24`), sends `FISH` to the
//! character, gives the fish (`AutoGiveItem(vnum, 1, -1, false)`, no line) and writes its
//! length to socket 0 (`GetFishLength`, `:578-594`). Every take after the bite then trains
//! the rod (`FishingPractice`, `:430-449`) and empties its bait socket.
//!
//! # Not ported
//!
//! - The `fishevent` highscore (`:666-684`), which waits on the highscore boards
//!   (`lua.highscore.register`).
//! - `LogManager::FishLog`, the log database this build does not have.
//!
//! # Defects not reproduced
//!
//! - Every record but `START` leaves `dir` uninitialized (`TPacketGCFishing p;` on the
//!   stack, `:405-409` and the others); the Rewrite sends 0.
//! - A position with no sectree crashes the block check (`pkSectreeMap->Find(x, y)` is
//!   dereferenced unchecked, `G/char.cpp:5265-5266`); the Rewrite reads it as not blocked.
//! - A draw past the last accumulated probability makes `std::lower_bound` answer the end,
//!   and `fish_info[MAX_FISH]` is then read past the table (`:389-396`, `:502`); the
//!   Rewrite answers fish `MAX_FISH`, which `Compute` refuses as legacy's does, and a
//!   `FISH` record of vnum 0.
//!
//! A campfire used on map 113 answers `Forbidden` and one within 60 s of the last fire
//! answers `Once at 60 seconds you can use the fire`; past a missing sectree or a water
//! cell the lines are `[LS;1217]` and `[LS;1228]`. Else one fire is used up, `kamp.ates`
//! takes the time, and the campfire (12000) stands at the point 100 units ahead facing a
//! drawn heading, for 40 s.

use common::vid::Vid;
use db::items::RowChange;
use gamedata::fishing::{Fish, FISHING_TIME, MAX_FISH, MAX_PROB};
use gamedata::item_kind::ITEM_ROD;
use gamedata::server_attr::{ATTR_BLOCK, ATTR_WATER};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_vid::{GcFishing, GcFishingSubheader};
use protocol::item_pos::ItemPos;
use world::character::{
    is_campfire, number, set_item_socket, use_bait, use_campfire, AutoGiven, Baited,
    CampfireGround, Dice, Fired, MoveDone, MoveKind, MoveRecord, CAMPFIRE_MOB, ROD_BAIT_SOCKET,
    WEAPON_CELL,
};
use world::item::Item;

use super::view::EntityKey;
use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::item_move::{MoveItemRefused, MovedItems, Mover, StoreOrder};
use crate::save::PASSES_PER_SEC;

/// `kill_campfire_event`'s delay (`G/char_item.cpp:2933`): 40 s.
const CAMPFIRE_LIFETIME: u32 = 40;
/// `AFFECT_FISH_MIND_PILL` (`G/affect.h:31`): 208, the eighth after `AFFECT_MOV_SPEED` (200).
const AFFECT_FISH_MIND_PILL: u32 = 208;
/// `UNIQUE_GROUP_FISH_MIND` (`G/unique_item.h:10`).
const UNIQUE_GROUP_FISH_MIND: i32 = 10_009;
/// `UNIQUE_ITEM_FISH_MIND` (`G/unique_item.h:47`).
const UNIQUE_ITEM_FISH_MIND: u32 = 71_008;
/// `ITEM_FISH_EVENT_BOX` (`C/length.h:111`).
const ITEM_FISH_EVENT_BOX: u32 = 25_106;
/// `ITEM_FISH_EVENT_BOX_SPECIAL` (`C/length.h:112`).
const ITEM_FISH_EVENT_BOX_SPECIAL: u32 = 25_107;
/// The seconds between the bite and the fish getting away (`PASSES_PER_SEC(6)`, `:505`).
const BITE_SECS: u64 = 6;
/// `MAX_FISHING_TIME_COUNT - 1` (`G/fishing.cpp:64`): the last time step.
const LAST_TIME_STEP: u32 = 30;
/// The time past which `Compute` refuses (`:560`).
const MAX_MS: u32 = 6000;
/// The fish `DetermineFish` never answers (`:396`): fish that are refused as nothing.
const NEVER_FISHED: [u32; 3] = [50_008, 50_009, 80_008];

/// One character's fishing event (`fishing_event_info`, `G/fishing.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FishingEvent {
    /// `step`: 0 before the bite, 1 after it, more once the fish got away.
    step: u8,
    /// `hang_time`: `get_dword_time()` at the bite.
    hang_time: u32,
    /// `fish_id`: the fish's place in the table.
    fish_id: usize,
    /// The Pulse the event fires on next.
    due: u64,
}

/// `GetProbIndexByMapIndex` (`G/fishing.cpp:338-358`): the probability column of a map.
const fn prob_index(map: i32) -> Option<usize> {
    match map {
        1 | 21 | 41 | 133 => Some(0),
        3 | 23 | 43 => Some(1),
        _ => None,
    }
}

/// `Compute` (`G/fishing.cpp:549-576`): the vnum caught, or the code of the miss: -2 for
/// fish 0 and a fish past the table, -1 for a time past 6 s or a missed time roll, -3 for a
/// missed difficulty roll. A fish the table has no row for is refused as one past it.
pub(super) fn compute(
    dice: &mut dyn Dice,
    fish: &[Fish],
    fish_id: usize,
    ms: u32,
    level: i32,
) -> Result<u32, i8> {
    if fish_id == 0 || fish_id >= MAX_FISH {
        return Err(-2);
    }
    let Some(one) = fish.get(fish_id) else {
        return Err(-2);
    };
    if ms > MAX_MS {
        return Err(-1);
    }
    let step = (ms.saturating_add(99) / 200).min(LAST_TIME_STEP);
    let chance = usize::try_from(one.time_type)
        .ok()
        .and_then(|row| FISHING_TIME.get(row))
        .and_then(|row| row.get(usize::try_from(step).ok()?))
        .copied()
        .unwrap_or(0);
    if number(dice, 1, 100) > chance {
        return Err(-1);
    }
    if number(dice, 1, one.difficulty) > level {
        return Err(-3);
    }
    Ok(one.vnum)
}

/// `GetFishLength` (`G/fishing.cpp:578-594`): 99 in 100 fish measure inside the normal
/// range, and the rest between its top and the extra top.
fn fish_length(dice: &mut dyn Dice, fish: &Fish) -> i32 {
    let [low, high, extra] = fish.length_range;
    if number(dice, 0, 99) != 0 {
        let draws = (0..5).fold(0i32, |sum, _| sum.wrapping_add(number(dice, 0, 2000)));
        return low.wrapping_add(high.wrapping_sub(low).wrapping_mul(draws) / 10_000);
    }
    let ratio = f64::from(number(dice, 0, 10_000)) / 10_000.0;
    let span = f64::from(extra.wrapping_sub(high).wrapping_mul(2));
    // `(int)` of the `double` sum.
    common::cfloat::f64_to_i32(f64::from(high) + span * ratio.asin() / std::f64::consts::PI)
}

/// A `GC_FISHING` record.
fn fishing_record(subheader: GcFishingSubheader, info: u32, dir: u8) -> Vec<u8> {
    let mut record = Vec::with_capacity(GcFishing::WIRE_SIZE);
    GcFishing {
        subheader,
        info,
        dir,
    }
    .encode_into(&mut record);
    record
}

/// The rod in the weapon cell: its id, sockets and the proto values the take reads.
struct Rod {
    id: world::item::ItemId,
    sockets: [i32; world::item::SOCKETS],
    values: [i32; 6],
    refined_vnum: u32,
}

impl GameState {
    /// One `CG_FISHING` of the character online under `vid`: the cast, or the take while its
    /// event runs. Every record reaches the client through the world, in legacy's order; the
    /// answer carries the rows to store.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when the world does not hold the character.
    pub fn fish(&mut self, vid: Vid, dir: u8, mover: Mover) -> Result<MovedItems, MoveItemRefused> {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?
            .player_id();
        // `SetRotation(p->dir * 5)`: the float of an `int`.
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.rotation = f32::from(dir) * 5.0;
        }
        // `ENABLE_MULTI_FARM_BLOCK` (`G/char.cpp:5251-5254`): unfarming fishes nothing,
        // not even the take.
        if !self.multi_status_of(vid) {
            return Ok(MovedItems::new(
                pid,
                vid.raw(),
                MoveDone {
                    kind: MoveKind::Socketed,
                    records: Vec::new(),
                    changes: Vec::new(),
                },
                mover,
                &self.protos,
                &self.locale,
            ));
        }
        let changes = if self.fishing_events.contains_key(&vid) {
            self.fishing_take(vid, pid, mover)
        } else {
            self.cast(vid, mover);
            Vec::new()
        };
        let mut answer = MovedItems::new(
            pid,
            vid.raw(),
            MoveDone {
                kind: MoveKind::Socketed,
                records: Vec::new(),
                changes: Vec::new(),
            },
            mover,
            &self.protos,
            &self.locale,
        );
        answer.changes = changes;
        Ok(answer)
    }

    /// `CHARACTER::fishing` without a running event: the refusals, then
    /// `CreateFishingEvent` (`G/fishing.cpp:520-537`).
    fn cast(&mut self, vid: Vid, mover: Mover) {
        let blocked = self
            .place_of(vid.raw())
            .zip(self.spot_of(vid.raw()))
            .is_some_and(|((_, map), spot)| self.is_attr(map, (spot.x, spot.y), ATTR_BLOCK));
        if blocked {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1051]", &[]);
            return;
        }
        let Some(rod) = self.worn_rod(vid) else {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;817]", &[]);
            return;
        };
        if rod.sockets[ROD_BAIT_SOCKET] == 0 {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;818]", &[]);
            return;
        }
        let secs = number(&mut self.dice, 10, 40);
        let rotation = self.bodies.get(&vid).map_or(0.0, |body| body.rotation);
        // `(BYTE)(GetRotation() / 5)`: the rotation is `dir * 5`, so this is `dir` again.
        let dir = u8::try_from(common::cfloat::f32_to_i32(rotation / 5.0)).unwrap_or(0);
        let start = fishing_record(GcFishingSubheader::Start, vid.raw(), dir);
        self.packet_around(EntityKey::Character(vid.raw()), &start, None);
        let due = self.last_pulse + u64::try_from(secs).unwrap_or(10) * u64::from(PASSES_PER_SEC);
        let _cast = self.fishing_events.insert(
            vid,
            FishingEvent {
                step: 0,
                hang_time: 0,
                fish_id: 0,
                due,
            },
        );
    }

    /// `fishing_take` (`G/char.cpp:5300-5320`): `Take` with a rod in the weapon cell, else
    /// `[LS;819]`; the event ends either way. Answers the rows the take changed.
    fn fishing_take(&mut self, vid: Vid, pid: u32, mover: Mover) -> Vec<RowChange> {
        let Some(event) = self.fishing_events.remove(&vid) else {
            return Vec::new();
        };
        let Some(rod) = self.worn_rod(vid) else {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;819]", &[]);
            return Vec::new();
        };
        let me = EntityKey::Character(vid.raw());
        let mut changes = Vec::new();
        match event.step {
            0 => {
                let stop = fishing_record(GcFishingSubheader::Stop, vid.raw(), 0);
                self.packet_around(me, &stop, None);
                return changes;
            }
            1 => {
                let ms = self.clock.now().wrapping_sub(event.hang_time);
                let level = rod.sockets[ROD_BAIT_SOCKET].wrapping_add(rod.values[0]);
                let fishing = std::sync::Arc::clone(&self.fishing);
                match compute(&mut self.dice, fishing.fish(), event.fish_id, ms, level) {
                    Ok(vnum) if vnum != 0 => {
                        let fish = fishing.fish().get(event.fish_id);
                        changes = self.land(vid, pid, (vnum, fish), mover);
                    }
                    _ => {
                        let fail = fishing_record(GcFishingSubheader::Fail, vid.raw(), 0);
                        self.packet_around(me, &fail, None);
                    }
                }
            }
            _ => {
                let fail = fishing_record(GcFishingSubheader::Fail, vid.raw(), 0);
                self.packet_around(me, &fail, None);
            }
        }
        changes.extend(self.practice(vid, pid, mover));
        changes
    }

    /// The catch of `Take` (`G/fishing.cpp:627-700`): `SUCCESS` around, the fish event
    /// boxes, `FISH` to the character, then the fish with its length in socket 0.
    fn land(
        &mut self,
        vid: Vid,
        pid: u32,
        (vnum, fish): (u32, Option<&Fish>),
        mover: Mover,
    ) -> Vec<RowChange> {
        let me = EntityKey::Character(vid.raw());
        let success = fishing_record(GcFishingSubheader::Success, vid.raw(), 0);
        self.packet_around(me, &success, None);
        let mut changes = Vec::new();
        if self.event_rate(b"enable_fish_event") != 0 {
            let special = self.wears_unique(vid, UNIQUE_ITEM_FISH_MIND)
                && number(&mut self.dice, 1, 100) <= 10;
            let boxes = if special {
                (ITEM_FISH_EVENT_BOX_SPECIAL, 1)
            } else {
                (ITEM_FISH_EVENT_BOX, 5)
            };
            changes.extend(self.give_quietly(vid, pid, boxes, mover, None));
        }
        let _sent = self.write_to_client(vid, fishing_record(GcFishingSubheader::Fish, vnum, 0));
        let length = fish.map(|fish| fish_length(&mut self.dice, fish));
        changes.extend(self.give_quietly(vid, pid, (vnum, 1), mover, length));
        changes
    }

    /// `AutoGiveItem(vnum, count, -1, false)`: the give without its name line, its records
    /// sent at once. With `socket0`, the given item then takes it (`SetSocket(0, ...)`),
    /// which an item laid on the ground does unseen.
    fn give_quietly(
        &mut self,
        vid: Vid,
        pid: u32,
        (vnum, count): (u32, u16),
        mover: Mover,
        socket0: Option<i32>,
    ) -> Vec<RowChange> {
        let questing = self.quest_running(vid);
        let Some((given, mut done)) = self.auto_give(vid, (vnum, count), -1, questing) else {
            return Vec::new();
        };
        done.records
            .retain(|record| !matches!(record, MoveRecord::PickedUp { .. }));
        let id = match given {
            AutoGiven::Merged(id) | AutoGiven::Placed { id, .. } => Some(id),
            AutoGiven::Unplaced(mut item) => {
                if let Some(length) = socket0 {
                    let _written = item.set_socket(0, length);
                }
                self.lay_auto_given(vid, item);
                None
            }
            AutoGiven::NoProto | AutoGiven::NotMade => None,
        };
        let mut changes = self.send_step(vid, pid, done, mover);
        if let (Some(id), Some(length)) = (id, socket0) {
            changes.extend(self.write_held_socket(vid, pid, (id, 0, length), mover));
        }
        changes
    }

    /// `FishingPractice` (`G/fishing.cpp:430-449`): a rod below its training top that can
    /// still be refined trains one step on a `number(1, value1)` of 1, and says so; then its
    /// bait is gone.
    fn practice(&mut self, vid: Vid, pid: u32, mover: Mover) -> Vec<RowChange> {
        let Some(rod) = self.worn_rod(vid) else {
            return Vec::new();
        };
        let mut changes = Vec::new();
        let top = rod.values[2];
        if rod.refined_vnum > 0
            && rod.sockets[0] < top
            && number(&mut self.dice, 1, rod.values[1]) == 1
        {
            let trained = rod.sockets[0].wrapping_add(1);
            changes.extend(self.write_held_socket(vid, pid, (rod.id, 0, trained), mover));
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"The training level of the fishing rod has increased! (%d/%d)",
                &[Arg::Int(i64::from(trained)), Arg::Int(i64::from(top))],
            );
            if trained == top {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"The fishing rod has reached its maximum training level.",
                    &[],
                );
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"You can upgrade your fishing rod to the next level through the fisherman.",
                    &[],
                );
            }
        }
        changes.extend(self.write_held_socket(vid, pid, (rod.id, ROD_BAIT_SOCKET, 0), mover));
        changes
    }

    /// The fishing events due at `pulse` (`fishing_event`, `G/fishing.cpp:464-518`): the
    /// bite, or the fish getting away. The rows of an emptied bait socket go to the
    /// descriptor, which writes them.
    pub(super) fn run_fishing_events(&mut self, pulse: u64) {
        let due: Vec<Vid> = self
            .fishing_events
            .iter()
            .filter_map(|(vid, event)| (pulse >= event.due).then_some(*vid))
            .collect();
        for vid in due {
            let Some(rod) = self.worn_rod(vid) else {
                let _ended = self.fishing_events.remove(&vid);
                continue;
            };
            let Some(event) = self.fishing_events.get(&vid).copied() else {
                continue;
            };
            if event.step == 0 {
                self.bite(vid, pulse);
                continue;
            }
            let _ended = self.fishing_events.remove(&vid);
            let fail = fishing_record(GcFishingSubheader::Fail, vid.raw(), 0);
            self.packet_around(EntityKey::Character(vid.raw()), &fail, None);
            let (Some(mover), Ok(character)) =
                (self.own_mover(vid), self.characters.find_by_vid(vid))
            else {
                continue;
            };
            let pid = character.player_id();
            let changes = self.write_held_socket(vid, pid, (rod.id, ROD_BAIT_SOCKET, 0), mover);
            if let Some((channel, map)) = self.place_of(vid.raw()) {
                let _ordered =
                    self.order_the_item_rows(vid, channel, map, changes, StoreOrder::default());
            }
        }
    }

    /// The event's first firing: the hang time, the fish, `REACT` around, `FISH` to a
    /// character who knows the fish, and the next firing 6 seconds out.
    fn bite(&mut self, vid: Vid, pulse: u64) {
        let fish_id = self.determine_fish(vid);
        let hang_time = self.clock.now();
        if let Some(event) = self.fishing_events.get_mut(&vid) {
            event.step = 1;
            event.hang_time = hang_time;
            event.fish_id = fish_id;
            event.due = pulse + BITE_SECS * u64::from(PASSES_PER_SEC);
        }
        let react = fishing_record(GcFishingSubheader::React, vid.raw(), 0);
        self.packet_around(EntityKey::Character(vid.raw()), &react, None);
        if self.predicts_fish(vid) {
            let vnum = self.fishing.fish().get(fish_id).map_or(0, |fish| fish.vnum);
            let _sent =
                self.write_to_client(vid, fishing_record(GcFishingSubheader::Fish, vnum, 0));
        }
    }

    /// `DetermineFish` (`G/fishing.cpp:361-401`): the fish of the map's column, of the fish
    /// mind's columns for a character who wears one (2, or 3 while the `manwoo` flag is
    /// set), with the `fish_miss_pct` flag's shift of the draw. 0 off the fishing maps.
    fn determine_fish(&mut self, vid: Vid) -> usize {
        let Some(mut prob) = self
            .place_of(vid.raw())
            .and_then(|(_, map)| prob_index(map))
        else {
            return 0;
        };
        if self.wears_unique_group(vid, UNIQUE_GROUP_FISH_MIND) {
            prob = if self.event_rate(b"manwoo") != 0 {
                3
            } else {
                2
            };
        }
        debug_assert!(prob < MAX_PROB);
        let fishing = std::sync::Arc::clone(&self.fishing);
        let miss = self.event_rate(b"fish_miss_pct");
        let adjust = if miss == 0 {
            0
        } else {
            let first = fishing.fish().first().map_or(0, |fish| fish.prob[prob]);
            (100 - miss.clamp(0, 200)).wrapping_mul(first) / 100
        };
        let (Some(table), Some(sum)) = (fishing.accumulate(prob), fishing.sum(prob)) else {
            return 0;
        };
        let rv = number(&mut self.dice, adjust.wrapping_add(1), sum);
        let fish_id = table
            .iter()
            .position(|&bound| bound >= rv)
            .unwrap_or(MAX_FISH);
        let vnum = fishing.fish().get(fish_id).map_or(0, |fish| fish.vnum);
        if NEVER_FISHED.contains(&vnum) {
            return 0;
        }
        fish_id
    }

    /// `PredictFish` (`G/fishing.cpp:451-462`): the fish mind pill's affect or a worn fish
    /// mind unique item. The premium fish mind is not ported.
    fn predicts_fish(&self, vid: Vid) -> bool {
        let pill = self.affect_events.get(&vid).is_some_and(|event| {
            event
                .affects
                .iter()
                .any(|affect| affect.kind == AFFECT_FISH_MIND_PILL)
        });
        pill || self.wears_unique_group(vid, UNIQUE_GROUP_FISH_MIND)
    }

    /// `IsEquipUniqueGroup` (`G/char_item.cpp:9543-9560`): a unique cell holds an item of the
    /// special item group `group`.
    fn wears_unique_group(&self, vid: Vid, group: i32) -> bool {
        use common::enums::EWearPositions;
        use world::character::Equipment;
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let equipment = Equipment::of(character.items(), &self.protos);
        [EWearPositions::Unique1, EWearPositions::Unique2]
            .into_iter()
            .filter_map(|wear| equipment.wear(wear as u16))
            .any(|worn| self.item_groups.unique.get(&worn.item.vnum) == Some(&group))
    }

    /// The rod in the weapon cell of the character under `vid`, if one is there.
    fn worn_rod(&self, vid: Vid) -> Option<Rod> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let item: &Item = character.items().item_at(ItemPos::new(
            common::item_slots::EWindows::Inventory as u8,
            WEAPON_CELL,
        ))?;
        let proto = self.protos.get(item.vnum)?;
        (proto.item_type == ITEM_ROD).then_some(Rod {
            id: item.id,
            sockets: item.sockets,
            values: proto.values,
            refined_vnum: proto.refined_vnum,
        })
    }

    /// `SetSocket(index, value)` on the held item `id`: its update reaches the client at
    /// once, and its rows are answered.
    pub(super) fn write_held_socket(
        &mut self,
        vid: Vid,
        pid: u32,
        (id, index, value): (world::item::ItemId, usize, i32),
        mover: Mover,
    ) -> Vec<RowChange> {
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return Vec::new();
        };
        let done = set_item_socket(character.items_mut(), id, (index, value));
        self.send_step(vid, pid, done, mover)
    }

    /// A step's records sent to the client at once; answers its rows.
    fn send_step(&self, vid: Vid, pid: u32, done: MoveDone, mover: Mover) -> Vec<RowChange> {
        let packed = MovedItems::new(pid, vid.raw(), done, mover, &self.protos, &self.locale);
        for record in packed.records {
            let _sent = self.write_to_client(vid, record);
        }
        packed.changes
    }

    /// Whether the item at `at` of the character under `vid` is a bait.
    pub(super) fn bait_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .and_then(|item| self.protos.get(item.vnum))
            .is_some_and(world::character::is_bait)
    }

    /// Whether the item at `at` of the character under `vid` is a campfire.
    pub(super) fn campfire_at(&self, vid: Vid, at: ItemPos) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .and_then(|item| self.protos.get(item.vnum))
            .is_some_and(is_campfire)
    }

    /// `CG_ITEM_USE` of a campfire (`case ITEM_CAMPFIRE`, `G/char_item.cpp:2891-2947`):
    /// the use checks, then the map, the 60 s cool, the tree and the water, each with
    /// its line first. A lit fire spends one, stamps `kamp.ates`, and stands the
    /// campfire at the point 100 ahead facing `number(0, 359)`, for 40 s
    /// (`kill_campfire_event`, `:2047-2066`).
    pub(super) fn use_campfire_item(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let Some(place) = self.place_of(vid.raw()) else {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        };
        let Some(spot) = self.spot_of(vid.raw()) else {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        };
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let last = self
            .quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, b"kamp.ates"))
            .map_or(0, |flag| u32::try_from(flag).unwrap_or(0));
        let now = Self::global_time();
        let rotation = self.bodies.get(&vid).map_or(0.0, |body| body.rotation);
        let (fx, fy) = super::mob_ai::delta_by_degree(rotation, 100.0);
        let point = (
            spot.x.saturating_add(common::cfloat::f32_to_i32(fx)),
            spot.y.saturating_add(common::cfloat::f32_to_i32(fy)),
        );
        let ground = self
            .maps
            .get(&place)
            .and_then(|index| index.tree_at(point.0, point.1))
            .map(|_| (point, self.is_attr(place.1, point, ATTR_WATER)));
        let mut outcome = Fired::NoTree;
        let mut lit_at: Option<(i32, i32)> = None;
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, _protos| {
            let around = CampfireGround {
                map: place.1,
                last,
                now,
                ground,
            };
            use_campfire(items, at, rules, gear, around).map(|(done, fired)| {
                outcome = fired;
                if fired == Fired::Lit {
                    lit_at = ground.map(|(at, _)| at);
                }
                done
            })
        })?;
        let line = match outcome {
            Fired::Barred => Some(b"Forbidden".as_slice()),
            Fired::Cooling => Some(b"Once at 60 seconds you can use the fire".as_slice()),
            Fired::NoTree => Some(b"[LS;1217]".as_slice()),
            Fired::Water => Some(b"[LS;1228]".as_slice()),
            Fired::Lit => None,
        };
        if let Some(text) = line {
            let to = actor.recipient(&self.locale);
            moved
                .records
                .insert(0, chat_packet(to, CHAT_TYPE_INFO, text, &[]));
            return Ok(moved);
        }
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_flag(
                pid,
                b"kamp.ates",
                i32::try_from(now).unwrap_or(i32::MAX),
            );
        }
        if let Some(at) = lit_at {
            let dir = number(&mut self.dice, 0, 359);
            if let Some(stood) = self.spawn_event_mob(place, CAMPFIRE_MOB, at, dir, false) {
                let when = self
                    .last_pulse
                    .saturating_add(super::regen::pulses(CAMPFIRE_LIFETIME));
                self.regens
                    .due
                    .insert((when, super::regen::Due::Campfire(stood)));
            }
        }
        Ok(moved)
    }

    /// `CG_ITEM_USE` of a bait (`case USE_BAIT`, `G/char_item.cpp:6800-6825`): the line
    /// goes first, then the rod's and the bait's records.
    pub(super) fn use_bait(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let fishing = self.fishing_events.contains_key(&vid);
        let mut outcome = (Baited::NoRod, 0);
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, _protos| {
            let vnum = items.item_at(at).map_or(0, |item| item.vnum);
            use_bait(items, at, rules, gear, fishing).map(|(done, baited)| {
                outcome = (baited, vnum);
                done
            })
        })?;
        let (baited, vnum) = outcome;
        let to = actor.recipient(&self.locale);
        let name = self.item_get_name(vnum);
        let line = match baited {
            Baited::NoRod => return Ok(moved),
            Baited::WhileFishing => chat_packet(to, CHAT_TYPE_INFO, b"[LS;418]", &[]),
            Baited::Replaced => {
                chat_packet(to, CHAT_TYPE_INFO, b"[LS;419;%s]", &[Arg::Text(&name)])
            }
            Baited::Put => chat_packet(to, CHAT_TYPE_INFO, b"[LS;420;%s]", &[Arg::Text(&name)]),
        };
        moved.records.insert(0, line);
        Ok(moved)
    }
}

#[cfg(test)]
mod tests {
    use gamedata::fishing::Fish;
    use world::character::FixedDraw;

    use super::{compute, fish_length, prob_index};

    fn fish(time_type: i32, difficulty: i32) -> Fish {
        Fish {
            name: b"fish".to_vec(),
            vnum: 27_803,
            dead_vnum: 0,
            grill_vnum: 0,
            prob: [0; 4],
            difficulty,
            time_type,
            length_range: [1000, 2500, 2800],
            used_table: [0; 10],
        }
    }

    #[test]
    fn only_the_seven_fishing_maps_have_a_column() {
        for (map, column) in [(1, 0), (21, 0), (41, 0), (133, 0), (3, 1), (23, 1), (43, 1)] {
            assert_eq!(prob_index(map), Some(column), "map {map}");
        }
        for map in [0, 2, 4, 22, 42, 132, 134, 135] {
            assert_eq!(prob_index(map), None, "map {map}");
        }
    }

    #[test]
    fn compute_refuses_fish_zero_past_the_table_and_a_late_take() {
        let table = vec![fish(3, 1); 37];
        let mut dice = FixedDraw::new(1);
        assert_eq!(compute(&mut dice, &table, 0, 0, 100), Err(-2));
        assert_eq!(compute(&mut dice, &table, 37, 0, 100), Err(-2));
        assert_eq!(compute(&mut dice, &table, 1, 6001, 100), Err(-1));
        // `FISHING_TIME_ALL` always passes the time roll; difficulty 1 always passes level 1.
        assert_eq!(compute(&mut dice, &table, 1, 6000, 1), Ok(27_803));
    }

    #[test]
    fn compute_misses_on_the_time_step_and_on_the_difficulty() {
        // `FISHING_TIME_NORMAL` is 0 for the first five steps: ms 0..=900.
        let table = vec![fish(0, 1); 37];
        assert_eq!(
            compute(&mut FixedDraw::new(1), &table, 1, 900, 100),
            Err(-1)
        );
        // Step 15 (ms 2901..=3100) is 80: a time draw of 80 passes, 81 misses.
        let hard = vec![fish(0, 50); 37];
        assert_eq!(
            compute(&mut FixedDraw::new(80), &hard, 1, 3000, 100),
            Ok(27_803)
        );
        assert_eq!(
            compute(&mut FixedDraw::new(81), &hard, 1, 3000, 100),
            Err(-1)
        );
        // The same source's `number(1, 50)` is 30: level 29 misses, 30 lands.
        assert_eq!(
            compute(&mut FixedDraw::new(80), &hard, 1, 3000, 29),
            Err(-3)
        );
        assert_eq!(
            compute(&mut FixedDraw::new(80), &hard, 1, 3000, 30),
            Ok(27_803)
        );
    }

    #[test]
    fn a_fish_measures_inside_its_range_or_its_extra_range() {
        let one = fish(0, 1);
        // A source of 1999: `number(0, 99)` is 99, the normal size, and each of the five
        // `number(0, 2000)` is 1999.
        assert_eq!(
            fish_length(&mut FixedDraw::new(2000), &one),
            1000 + 1500 * 9995 / 10_000
        );
        // A source of 0: `number(0, 99)` is 0, the extra size, and `asin(0)` keeps the
        // range's top.
        assert_eq!(fish_length(&mut FixedDraw::new(1), &one), 2500);
    }
}
