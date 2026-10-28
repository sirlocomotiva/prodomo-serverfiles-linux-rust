//! The quest targets (`CTargetManager`, `G/target.cpp`): the characters a quest marks on its
//! character's client with `target.vid`, each the character's own, which the client hears of as
//! `GC_TARGET_CREATE` (125) and `GC_TARGET_DELETE` (124).
//!
//! A target's ID comes from one counter for the whole process (`m_iID`), so the IDs one
//! character sees are consecutive only while no one else is marked. A target that replaces one
//! of the same quest and name keeps its place and takes a new ID; a character that leaves the
//! world drops its targets and its client hears nothing (`CTargetManager::Logout`, run from
//! `CHARACTER::Disconnect`, `G/char.cpp:1815`).
//!
//! Each target has an event (`target_event`, `G/target.cpp:57-132`) that first runs a second
//! after the target is made and then again at a pace set by the distance. It tells the client
//! where the marked character stands whenever that changes (`GC_TARGET_UPDATE`, 123), runs the
//! quest's `arrive` script within 500 of it, and, once no character holds the VID, runs the
//! `die` script and ends the target. A replaced target keeps its event and its pace. A click on a
//! marked character asks the quest first ([`quest::manager::Manager::click`]). Only
//! `TARGET_TYPE_VID` targets exist, because `target.pos` is not ported.
//!
//! # Divergences
//!
//! - **Long names match.** Legacy keeps the first 32 bytes of a name (`szTargetName[32+1]`) but
//!   compares the whole name a script passes, so a target named with more than 32 bytes never
//!   matches itself: each `target.vid` adds another, and `target.delete` and `target.id` miss
//!   it (a Defect). The Rewrite compares the kept 32 bytes with the first 32 of the name.
//! - **The name field is zero-filled.** `SendTargetCreatePacket` fills a stack record whose
//!   `szName` bytes past the `strlcpy` terminator are never written (a Defect); the Rewrite
//!   sends zeros there.
//! - **The VID is a `DWORD`.** Legacy stores `(int)lua_tonumber` in `iArg1`, which cannot hold
//!   the Rewrite's NPC VIDs from 2^31 up ([`quest::manager`]'s `target.vid`).
//! - **The event finds the VID in the Channel.** `CHARACTER_MANAGER::Find` searches one core,
//!   so legacy's `die` depended on which maps shared the marked character's core; the Rewrite
//!   finds a character anywhere in the marking character's Channel, as on a one-core Channel.

use std::collections::BTreeMap;

use common::vid::Vid;
use protocol::gc_actors::{GcTargetCreateNew, TARGET_NAME_LEN};
use protocol::gc_fields::GcTargetUpdate;
use protocol::gc_inventory::HEADER_GC_TARGET_DELETE;
use protocol::gc_vid::GcHeaderAndDword;
use quest::manager::{Game, Manager, Target};
use world::character::Character;

use super::GameState;
use crate::game_loop_messages::GroundPlace;
use crate::save::PASSES_PER_SEC;
use crate::sync_position::distance_approx;

/// `TARGET_TYPE_VID` (`G/target.h:7`): a target on a character.
const TARGET_TYPE_VID: u8 = 1 << 1;

/// What `strlcpy` keeps of a name in `szTargetName[32+1]` and `szTargetDesc[32+1]`.
const KEPT: usize = TARGET_NAME_LEN - 1;

/// `target_event`'s distance before it finds the marked character (`iDist`, `G/target.cpp:75`).
const UNSEEN_DISTANCE: i32 = 5000;

/// The distance within which `target_event` reports `arrive` (`G/target.cpp:103-104`).
const ARRIVE_DISTANCE: i32 = 500;

/// What `target_event`'s `CHARACTER_MANAGER::Find` finds under a mark's VID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marked {
    /// No character of the Channel holds the VID: the mark's `die`, then its end.
    Gone,
    /// The character stands on another map, so its spot is not read.
    Elsewhere,
    /// The character stands on the event's map, at this spot.
    At(i32, i32),
}

/// The Pulses until a target's next check (`G/target.cpp:78,128-131`): a second when its
/// `arrive` found no script, else `MINMAX(passes_per_sec / 2, iDist / (1500 / passes_per_sec),
/// passes_per_sec * 5)`.
fn next_check(distance: i32, taken: bool) -> u64 {
    let second = i64::from(PASSES_PER_SEC);
    let pulses = if taken {
        (i64::from(distance) / (1500 / second)).clamp(second / 2, second * 5)
    } else {
        second
    };
    u64::try_from(pulses).unwrap_or(0)
}

/// One target (`TargetInfo`, `G/target.h:13-51`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct QuestTarget {
    /// `iID`.
    id: i32,
    /// `dwQuestIndex`.
    quest: u32,
    /// `szTargetName`: the quest's name for it, its first 32 bytes.
    name: Vec<u8>,
    /// `szTargetDesc`: the name the client shows, its first 32 bytes.
    desc: Vec<u8>,
    /// `iArg1`: the VID it points at.
    vid: u32,
    /// `bSendToClient`.
    send: bool,
    /// The event's own key, which a replace keeps (legacy keeps the `LPEVENT`).
    event: u64,
    /// `iMapIndex`: the map the target was made on, which a replace keeps.
    map: i32,
    /// The Pulse of the event's next check, which a replace keeps.
    due: u64,
    /// `iOldX` and `iOldY`: where the client last heard the marked character stands.
    old: (i32, i32),
}

impl QuestTarget {
    /// `SendTargetCreatePacket` (`G/target.cpp:15-28`): nothing unless the client hears of it.
    fn create_record(&self) -> Option<Vec<u8>> {
        if !self.send {
            return None;
        }
        let mut name = [0; TARGET_NAME_LEN];
        name[..self.desc.len()].copy_from_slice(&self.desc);
        let mut out = Vec::new();
        GcTargetCreateNew::new(self.id, name, self.vid, TARGET_TYPE_VID).encode_into(&mut out);
        Some(out)
    }

    /// `SendTargetDeletePacket` (`G/target.cpp:41-47`), when the client heard of it.
    fn delete_record(&self) -> Option<Vec<u8>> {
        if !self.send {
            return None;
        }
        let id = u32::from_le_bytes(self.id.to_le_bytes());
        let mut out = Vec::new();
        GcHeaderAndDword::new(HEADER_GC_TARGET_DELETE.value(), id).encode_into(&mut out);
        Some(out)
    }
}

/// `strlcpy` into 33 bytes: the text up to its first zero, at most 32 bytes of it.
fn kept(text: &[u8]) -> Vec<u8> {
    let end = text
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(text.len());
    text[..end.min(KEPT)].to_vec()
}

/// Every character's quest targets (`m_map_kListEvent`), the last ID given (`m_iID`) and the
/// last event key.
#[derive(Debug, Default)]
pub(super) struct QuestTargets {
    last_id: i32,
    last_event: u64,
    by_pid: BTreeMap<u32, Vec<QuestTarget>>,
}

impl QuestTargets {
    /// `CTargetManager::CreateTarget` (`G/target.cpp:134-238`) with `TARGET_TYPE_VID` for
    /// character `pid`, standing on map `map` at Pulse `pulse`: a target of the same quest and
    /// name is replaced in place, the client hearing of the old one's end first, and keeps its
    /// event; else a new one is added, whose event first runs a second later. Answers the
    /// records for the character's client.
    pub(super) fn create(
        &mut self,
        pid: u32,
        target: &Target,
        (map, pulse): (i32, u64),
    ) -> Vec<Vec<u8>> {
        self.last_id = self.last_id.wrapping_add(1);
        let mut made = QuestTarget {
            id: self.last_id,
            quest: target.quest,
            name: kept(&target.name),
            desc: target.desc.as_deref().map(kept).unwrap_or_default(),
            vid: target.vid,
            send: target.send,
            event: 0,
            map,
            due: pulse + u64::from(PASSES_PER_SEC),
            old: (0, 0),
        };
        let mut records = Vec::new();
        let targets = self.by_pid.entry(pid).or_default();
        let same = targets
            .iter_mut()
            .find(|old| old.quest == made.quest && old.name == made.name);
        if let Some(old) = same {
            records.extend(old.delete_record());
            made.event = old.event;
            made.map = old.map;
            made.due = old.due;
            *old = made;
            records.extend(old.create_record());
        } else {
            self.last_event += 1;
            made.event = self.last_event;
            records.extend(made.create_record());
            targets.push(made);
        }
        records
    }

    /// `CTargetManager::DeleteTarget` (`G/target.cpp:240-282`): character `pid`'s targets of
    /// quest `quest` named `name`, or all of them when `name` is `None`, end in the order they
    /// were made, each the client heard of with a delete record.
    pub(super) fn delete(&mut self, pid: u32, quest: u32, name: Option<&[u8]>) -> Vec<Vec<u8>> {
        let Some(targets) = self.by_pid.get_mut(&pid) else {
            return Vec::new();
        };
        let name = name.map(kept);
        let mut records = Vec::new();
        targets.retain(|target| {
            let ends =
                target.quest == quest && name.as_ref().is_none_or(|name| target.name == *name);
            if ends {
                records.extend(target.delete_record());
            }
            !ends
        });
        if targets.is_empty() {
            self.by_pid.remove(&pid);
        }
        records
    }

    /// `GetTargetEvent` (`G/target.cpp:284-315`) as `target_id` reads it: the VID character
    /// `pid`'s target of quest `quest` named `name` points at, and 0 without one.
    pub(super) fn vid(&self, pid: u32, quest: u32, name: &[u8]) -> u32 {
        let name = kept(name);
        self.by_pid
            .get(&pid)
            .and_then(|targets| {
                targets
                    .iter()
                    .find(|target| target.quest == quest && target.name == name)
            })
            .map_or(0, |target| target.vid)
    }

    /// `GetTargetInfo(pid, TARGET_TYPE_VID, vid)` (`G/target.cpp:317-348`): the quest and name
    /// of character `pid`'s first target that points at `vid`.
    pub(super) fn at(&self, pid: u32, vid: u32) -> Option<(u32, Vec<u8>)> {
        self.by_pid
            .get(&pid)?
            .iter()
            .find(|target| target.vid == vid)
            .map(|target| (target.quest, target.name.clone()))
    }

    /// `CTargetManager::Logout` (`G/target.cpp:350-363`): character `pid`'s targets end
    /// with no record.
    pub(super) fn logout(&mut self, pid: u32) {
        self.by_pid.remove(&pid);
    }

    /// The events due at `pulse`: by character, each character's in the order its targets
    /// were made.
    fn due(&self, pulse: u64) -> Vec<(u32, u64)> {
        self.by_pid
            .iter()
            .flat_map(|(pid, targets)| {
                targets
                    .iter()
                    .filter(|target| target.due <= pulse)
                    .map(|target| (*pid, target.event))
            })
            .collect()
    }

    /// Character `pid`'s target whose event is `event`, while it lasts.
    fn event_mut(&mut self, pid: u32, event: u64) -> Option<&mut QuestTarget> {
        self.by_pid
            .get_mut(&pid)?
            .iter_mut()
            .find(|target| target.event == event)
    }
}

impl GameState {
    /// `target_event` (`G/target.cpp:57-132`) of every quest target due at `pulse`.
    pub(super) fn run_quest_targets(&mut self, pulse: u64) {
        for (pid, event) in self.quest_targets.due(pulse) {
            self.run_quest_target(pid, event, pulse);
        }
    }

    /// One `target_event`: where the marked character stands, its `arrive` within 500, its
    /// `die` and the target's end when no character holds the VID, the client's
    /// `GC_TARGET_UPDATE` when the spot changed, and the next check.
    fn run_quest_target(&mut self, pid: u32, event: u64, pulse: u64) {
        // A script of an earlier event this Pulse may have ended it (`is_force_to_end`).
        let Some(target) = self.quest_targets.event_mut(pid, event).cloned() else {
            return;
        };
        let Some(vid) = self.characters.find_by_pid(pid).ok().map(Character::vid) else {
            // Legacy's `FindByPID` miss ends the event and leaves the target listed; a
            // character's targets end when it leaves, so none outlives it here.
            self.quest_targets.logout(pid);
            return;
        };
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return;
        };
        if place.1 != target.map {
            if let Some(waiting) = self.quest_targets.event_mut(pid, event) {
                waiting.due = pulse + next_check(UNSEEN_DISTANCE, true);
            }
            return;
        }
        let found = self.marked_spot(target.vid, place);
        let ((x, y), distance) = match found {
            Marked::At(x, y) => ((x, y), distance_approx(spot.x - x, spot.y - y)),
            Marked::Elsewhere | Marked::Gone => ((0, 0), UNSEEN_DISTANCE),
        };
        let mark = (target.quest, target.name.as_slice());
        let mut taken = true;
        if distance <= ARRIVE_DISTANCE {
            taken = self.run_quest_event(vid, |manager, game| {
                manager.target(pid, mark, b"arrive", game)
            });
        }
        if found == Marked::Gone {
            let _ran =
                self.run_quest_event(vid, |manager, game| manager.target(pid, mark, b"die", game));
            for record in self.quest_targets.delete(pid, mark.0, Some(mark.1)) {
                let _sent = self.write_to_client(vid, record);
            }
            return;
        }
        let Some(target) = self.quest_targets.event_mut(pid, event) else {
            return;
        };
        let mut update = None;
        if (x, y) != target.old {
            if target.send {
                let mut out = Vec::new();
                GcTargetUpdate::new(target.id, x, y).encode_into(&mut out);
                update = Some(out);
            }
            target.old = (x, y);
        }
        target.due = pulse + next_check(distance, taken);
        if let Some(record) = update {
            let _sent = self.write_to_client(vid, record);
        }
    }

    /// `CHARACTER_MANAGER::Find(vid)` for a target event in Channel `channel`, and where on
    /// `map` the character it finds stands.
    fn marked_spot(&self, vid: u32, (channel, map): (u8, i32)) -> Marked {
        if let Some(&(at_channel, at_map, _)) = self.npc_of.get(&vid) {
            return match self.npc_by_vid(vid) {
                Some(npc) if at_channel == channel && at_map == map => Marked::At(npc.x, npc.y),
                Some(_) if at_channel == channel => Marked::Elsewhere,
                _ => Marked::Gone,
            };
        }
        match self.place_of(vid) {
            Some((at_channel, at_map)) if at_channel == channel => {
                match self.spot_of(vid).filter(|_| at_map == map) {
                    Some(spot) => Marked::At(spot.x, spot.y),
                    None => Marked::Elsewhere,
                }
            }
            _ => Marked::Gone,
        }
    }

    /// Runs `op` for the character under `vid` outside any step of its own, as the login and
    /// a target's event do: the records its scripts sent go to its client, the block byte
    /// they set becomes its own, and their GRAND climbs and guild calls are dropped.
    pub(super) fn run_quest_event<R: Default>(
        &mut self,
        vid: Vid,
        op: impl FnOnce(&mut Manager, &mut dyn Game) -> R,
    ) -> R {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return R::default();
        };
        let pid = character.player_id();
        let level = character
            .points()
            .map_or(1, world::character::Points::level);
        let Some(mover) = self.own_mover(vid) else {
            return R::default();
        };
        let Some((channel, map)) = self.place_of(vid.raw()) else {
            return R::default();
        };
        let (x, y) = self
            .spot_of(vid.raw())
            .map_or((0, 0), |spot| (spot.x, spot.y));
        let place = GroundPlace { channel, map, x, y };
        let Some((answer, (records, _, _, block_mode))) =
            self.run_quests(place, mover, level, pid, op)
        else {
            return R::default();
        };
        if let Some(mode) = block_mode {
            self.block_modes.insert(vid, mode);
        }
        for record in records {
            let _sent = self.write_to_client(vid, record);
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `create` on map 1 at Pulse 0.
    fn make(targets: &mut QuestTargets, pid: u32, target: &Target) -> Vec<Vec<u8>> {
        targets.create(pid, target, (1, 0))
    }

    fn target(quest: u32, name: &[u8], vid: u32, desc: Option<&[u8]>, send: bool) -> Target {
        Target {
            quest,
            name: name.to_vec(),
            vid,
            desc: desc.map(<[u8]>::to_vec),
            send,
        }
    }

    /// `TPacketGCTargetCreate` (`G/packet.h:2523-2530`) in its field order: the header, `lID`,
    /// the 33-byte `szName`, `dwVID` and `bType`.
    fn create(id: i32, name: &[u8], vid: u32) -> Vec<u8> {
        let mut out = vec![125];
        out.extend_from_slice(&id.to_le_bytes());
        let mut field = [0; 33];
        field[..name.len()].copy_from_slice(name);
        out.extend_from_slice(&field);
        out.extend_from_slice(&vid.to_le_bytes());
        out.push(2);
        out
    }

    fn delete(id: i32) -> Vec<u8> {
        let mut out = vec![124];
        out.extend_from_slice(&id.to_le_bytes());
        out
    }

    /// A new target takes the next ID of the one counter and the client hears of it; the same
    /// quest and name replace it in place with a new ID, the delete first; another quest or
    /// another name is another target.
    #[test]
    fn a_target_of_the_same_quest_and_name_is_replaced() {
        let mut targets = QuestTargets::default();
        let first = make(
            &mut targets,
            7,
            &target(44, b"theowahdan", 0x8000_0102, Some(b"Th"), true),
        );
        assert_eq!(first, [create(1, b"Th", 0x8000_0102)]);
        assert_eq!(
            hex(&first[0]),
            "7d01000000\
             546800000000000000000000000000000000000000000000000000000000000000\
             0201008002"
        );
        let other = make(
            &mut targets,
            9,
            &target(44, b"theowahdan", 0x0304_0506, None, true),
        );
        assert_eq!(other, [create(2, b"", 0x0304_0506)], "another character's");
        let replaced = make(
            &mut targets,
            7,
            &target(44, b"theowahdan", 0x8000_0103, None, true),
        );
        assert_eq!(replaced, [delete(1), create(3, b"", 0x8000_0103)]);
        assert_eq!(targets.vid(7, 44, b"theowahdan"), 0x8000_0103);
        let quiet = make(
            &mut targets,
            7,
            &target(45, b"theowahdan", 0x0a0b_0c0d, None, false),
        );
        assert!(quiet.is_empty(), "a target the client does not hear of");
        assert_eq!(targets.vid(7, 45, b"theowahdan"), 0x0a0b_0c0d);
        let heard = make(
            &mut targets,
            7,
            &target(45, b"theowahdan", 0x0a0b_0c0e, None, true),
        );
        assert_eq!(
            heard,
            [create(5, b"", 0x0a0b_0c0e)],
            "the quiet one ends quietly"
        );
        assert_eq!(
            targets.vid(7, 44, b"Theowahdan"),
            0,
            "the name is case-sensitive"
        );
        assert_eq!(targets.by_pid[&7].len(), 2);
    }

    /// `strlcpy` keeps 32 bytes of the name and of the description, and both stop at a zero; a
    /// long name matches its own target (a Divergence from legacy's full-name compare).
    #[test]
    fn names_keep_32_bytes_and_long_names_match() {
        let long = [b'n'; 40];
        let desc = [b'd'; 40];
        let mut targets = QuestTargets::default();
        let made = make(&mut targets, 7, &target(1, &long, 5, Some(&desc), true));
        assert_eq!(made, [create(1, &desc[..32], 5)]);
        assert_eq!(made[0].len(), 43);
        assert_eq!(made[0][37], 0, "the 33rd byte is the terminator");
        let replaced = make(
            &mut targets,
            7,
            &target(1, &long[..33], 6, Some(b"a\0b"), true),
        );
        assert_eq!(replaced, [delete(1), create(2, b"a", 6)]);
        assert_eq!(targets.vid(7, 1, &long), 6);
        assert_eq!(targets.vid(7, 1, &long[..31]), 0);
        assert_eq!(targets.delete(7, 1, Some(&long[..32])), [delete(2)]);
        assert!(targets.by_pid.is_empty());
    }

    /// A named delete ends the quest's targets of that name; a clear ends every target of the
    /// quest, in the order they were made; another quest's and another character's stay. A
    /// target the client never heard of ends with no record, and a logout ends all with none.
    #[test]
    fn a_delete_ends_the_quests_targets_in_order() {
        let mut targets = QuestTargets::default();
        make(&mut targets, 7, &target(1, b"a", 10, None, true));
        make(&mut targets, 7, &target(1, b"b", 11, None, false));
        make(&mut targets, 7, &target(2, b"a", 12, None, true));
        make(&mut targets, 7, &target(1, b"c", 13, None, true));
        make(&mut targets, 8, &target(1, b"a", 14, None, true));
        assert!(targets.delete(7, 1, Some(b"z")).is_empty());
        assert!(targets.delete(6, 1, None).is_empty());
        assert_eq!(targets.delete(7, 1, Some(b"a")), [delete(1)]);
        assert_eq!(targets.vid(7, 1, b"a"), 0);
        assert_eq!(targets.vid(7, 2, b"a"), 12);
        assert_eq!(targets.delete(7, 1, None), [delete(4)], "b was never sent");
        assert_eq!(targets.vid(7, 1, b"b"), 0);
        assert_eq!(targets.vid(8, 1, b"a"), 14);
        targets.logout(7);
        assert_eq!(targets.vid(7, 2, b"a"), 0);
        assert!(targets.delete(7, 2, None).is_empty());
        assert_eq!(targets.vid(8, 1, b"a"), 14);
        let next = make(&mut targets, 8, &target(1, b"x", 15, None, true));
        assert_eq!(next, [create(6, b"", 15)], "the counter never goes back");
    }

    /// `target_event`'s `CHARACTER_MANAGER::Find` and its map check (`G/target.cpp:89-98`): a
    /// VID on the event's map is found where it stands, an NPC's or a character's; one on
    /// another map is found without its spot; one of another Channel, or of nobody, is gone,
    /// which runs `die` and ends the mark.
    #[test]
    fn the_find_tells_a_spot_from_another_map_and_from_nobody() {
        use crate::game_state::fixtures::{enter, open_cells, stand_field, TestClock, PLACE};

        let clock = TestClock::default();
        let theowahdan = world::npc::Npc {
            vid: world::npc::FIRST_NPC_VID,
            vnum: 60_003,
            race: 60_003,
            x: 600,
            y: 700,
            ..world::npc::Npc::default()
        };
        let mut world = stand_field(&clock, vec![theowahdan], open_cells(), true);
        let _inbox = enter(&mut world, 1, (500, 400), 0);
        let elsewhere = (PLACE.0, PLACE.1 + 1);
        let other_channel = (PLACE.0 + 1, PLACE.1);
        let npc = world::npc::FIRST_NPC_VID;
        assert_eq!(world.marked_spot(npc, PLACE), Marked::At(600, 700));
        assert_eq!(world.marked_spot(npc, elsewhere), Marked::Elsewhere);
        assert_eq!(world.marked_spot(npc, other_channel), Marked::Gone);
        assert_eq!(world.marked_spot(1, PLACE), Marked::At(500, 400));
        assert_eq!(world.marked_spot(1, elsewhere), Marked::Elsewhere);
        assert_eq!(world.marked_spot(1, other_channel), Marked::Gone);
        assert_eq!(world.marked_spot(2, PLACE), Marked::Gone, "nobody");
        assert_eq!(world.marked_spot(npc + 1, PLACE), Marked::Gone, "no NPC");
    }

    /// A mark on a character of another map (`G/target.cpp:89-98`): the find holds it, so no
    /// `die` runs and the mark stays. Its spot reads (0, 0), the old spot a new mark starts
    /// from (`:226-227`), so the client hears nothing, and the checks come
    /// `MINMAX(12, 5000 / 60, 125)` = 83 Pulses apart until it stands on the mark's map.
    #[test]
    fn a_mark_on_a_character_of_another_map_stays_and_waits() {
        use crate::game_loop::PulseProcessor;
        use crate::game_state::fixtures::{a_world, enter, enter_on, region, TestClock, PLACE};
        use gamedata::map_atlas::MapRegion;
        use gamedata::server_attr::SectreeGrid;

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let other = MapRegion {
            index: PLACE.1 + 1,
            ..region()
        };
        world.host_map(
            PLACE.0,
            other.index,
            SectreeGrid::of(&other).expect("a grid"),
        );
        let mut inbox = enter(&mut world, 1, (500, 500), 0);
        let _away = enter_on(&mut world, (PLACE.0, other.index), 2, (600, 600), 0);
        while inbox.try_recv().is_ok() {}
        let made =
            world
                .quest_targets
                .create(1, &target(44, b"friend", 2, None, true), (PLACE.1, 100));
        assert_eq!(made, [create(1, b"", 2)]);
        let mut heard = |world: &mut GameState, pulse: u64| -> Vec<Vec<u8>> {
            world.process_pulse(pulse);
            std::iter::from_fn(|| inbox.try_recv().ok())
                .filter(|record| matches!(record.first(), Some(123 | 124)))
                .collect()
        };
        assert!(heard(&mut world, 125).is_empty(), "found, without a spot");
        assert_eq!(
            world.quest_targets.vid(1, 44, b"friend"),
            2,
            "the mark stays"
        );
        let away = world.place_of(2).expect("a place");
        let mut effects = Vec::new();
        world
            .maps
            .get_mut(&away)
            .expect("its map")
            .remove(super::super::view::EntityKey::Character(2), &mut effects);
        world.bodies.get_mut(&Vid::new(2)).expect("a body").map = PLACE.1;
        assert!(world.show_body(2, (600, 600, 0), None));
        assert!(heard(&mut world, 207).is_empty());
        let mut update = vec![123, 1, 0, 0, 0];
        update.extend_from_slice(&600_i32.to_le_bytes());
        update.extend_from_slice(&600_i32.to_le_bytes());
        assert_eq!(
            heard(&mut world, 208),
            [update],
            "83 Pulses after the first"
        );
    }

    /// `MINMAX(12, iDist / 60, 125)` when the check stands, a second when an `arrive` found
    /// no script.
    #[test]
    fn the_next_check_follows_the_distance() {
        assert_eq!(next_check(0, true), 12);
        assert_eq!(next_check(779, true), 12);
        assert_eq!(next_check(780, true), 13);
        assert_eq!(next_check(UNSEEN_DISTANCE, true), 83);
        assert_eq!(next_check(7499, true), 124);
        assert_eq!(next_check(7560, true), 125);
        assert_eq!(next_check(i32::MAX, true), 125);
        assert_eq!(next_check(100, false), 25);
        assert_eq!(next_check(9000, false), 25);
    }

    /// A new target's event first runs a second after it is made, on the map it was made on.
    /// A replace keeps the event, its map and its next check, and the client hears the spot
    /// again (`iOldX` and `iOldY` back to 0). `GetTargetInfo` answers the first target that
    /// points at the VID.
    #[test]
    fn a_replace_keeps_the_event_and_its_pace() {
        let mut targets = QuestTargets::default();
        targets.create(7, &target(1, b"a", 10, None, true), (41, 100));
        targets.create(7, &target(1, b"b", 11, None, true), (41, 110));
        targets.create(7, &target(2, b"c", 11, None, true), (41, 110));
        let first = &targets.by_pid[&7][0];
        assert_eq!(
            (first.event, first.map, first.due, first.old),
            (1, 41, 125, (0, 0))
        );
        targets.by_pid.get_mut(&7).expect("7's")[0].old = (5, 6);
        targets.create(7, &target(1, b"a", 12, None, true), (21, 300));
        let replaced = &targets.by_pid[&7][0];
        assert_eq!(
            (replaced.id, replaced.vid, replaced.event),
            (4, 12, 1),
            "a new ID, the same event"
        );
        assert_eq!(
            (replaced.map, replaced.due, replaced.old),
            (41, 125, (0, 0))
        );
        assert_eq!(targets.by_pid[&7][1].event, 2);
        assert!(targets.due(124).is_empty());
        assert_eq!(targets.due(125), [(7, 1)]);
        assert_eq!(targets.due(135), [(7, 1), (7, 2), (7, 3)]);
        assert_eq!(targets.at(7, 12), Some((1, b"a".to_vec())));
        assert_eq!(targets.at(7, 11), Some((1, b"b".to_vec())), "the first");
        assert_eq!(targets.at(7, 10), None);
        assert_eq!(targets.at(8, 12), None);
    }

    /// `++m_iID` past `INT_MAX` wraps (legacy's signed overflow); the record carries the bits.
    #[test]
    fn the_id_wraps_at_the_top_of_int() {
        let mut targets = QuestTargets {
            last_id: i32::MAX,
            ..QuestTargets::default()
        };
        let made = make(&mut targets, 7, &target(1, b"a", 1, None, true));
        assert_eq!(made, [create(i32::MIN, b"", 1)]);
        assert_eq!(targets.delete(7, 1, None), [delete(i32::MIN)]);
        assert_eq!(hex(&delete(i32::MIN)), "7c00000080");
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;

        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
        }
        output
    }
}
