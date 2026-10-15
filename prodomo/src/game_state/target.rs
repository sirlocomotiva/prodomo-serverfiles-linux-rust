//! The target: `CInputMain::Target` (`G/input_main.cpp:2249-2269`), `CHARACTER::SetTarget`,
//! `ClearTarget` and `BroadcastTargetPacket` (`G/char.cpp:6419-6661`).
//!
//! # What legacy does
//!
//! A click on a character sends `CG_TARGET` with its VID. `CInputMain::Target` first asks the
//! building manager for an object of that VID, and answers an object with a `GC_TARGET` that
//! names it and carries nothing else. Otherwise it calls `SetTarget` with whatever character
//! `CHARACTER_MANAGER::Find` finds, `NULL` for a VID that names none:
//!
//! 1. the same target as before changes nothing and sends nothing;
//! 2. on a castle map (181 to 183) a character that is not a GM changes nothing;
//! 3. the old target forgets the character, the new one remembers it in its targeted-by set, and
//!    the character is sent `GC_TARGET` with the new target's VID and hit points: its percentage
//!    clamped to 0..100, its hit points and its maximum (`__VIEW_TARGET_PLAYER_HP__` and
//!    `__VIEW_TARGET_DECIMAL_HP__`), 0 for each when the maximum is not positive, and 100 for each
//!    for a horse (races 20101 to 20109) with no rider. A monster whose race flags reach
//!    `RACE_FLAG_ATT_ELEC` carries an element (`ELEMENT_TARGET`). The alliance fields of
//!    `__SHIP_DEFENSE__` are false and 0. No target sends VID 0 and zeros.
//!
//! `PointChange(POINT_HP)` calls `BroadcastTargetPacket` after it sets the hit points, whatever the
//! amount, so each character targeting one whose hit points change is sent its new percentage
//! (`G/char.cpp:4259`). `CHARACTER::Destroy` calls `ClearTarget` (`G/char.cpp:654`): the
//! character's own target forgets it, and each character targeting it is sent VID 0 and loses
//! its target.
//!
//! # Divergences
//!
//! - **Where a target is found.** `CHARACTER_MANAGER::Find` finds a VID on any map, so a modified
//!   client can target a character on another map. The Rewrite finds a player standing on the
//!   targeting character's Channel and map, or an NPC standing there, as it does for a click, a
//!   shop and a trade (a Defect not reproduced).
//! - **The order of the targeters.** Legacy walks the targeted-by set by pointer; the Rewrite
//!   walks it by VID (V1).
//! - **The element of a broadcast and a clear.** `BroadcastTargetPacket` and `ClearTarget` never
//!   set `bElement`, so the byte is whatever the stack held (a Defect not reproduced). The Rewrite
//!   sends 0.
//! - **The percentage.** Legacy multiplies the hit points by 100 in an `int`. The Rewrite does it
//!   in 64 bits; no maximum in the owner's data comes near the bound.
//!
//! # Not ported
//!
//! - The building objects: the Rewrite has none (V6), so no VID names one.
//! - `CheckTarget`, which the player recovery event runs every three seconds and which drops a
//!   target 4800 or more away (`G/char.cpp:3250`, `:6663-6669`): that event is not ported, so a
//!   target is kept at any distance until it leaves the world.
//! - The killer's target, which a death clears (`G/char_battle.cpp:1275-1276`), for a player's
//!   death: no player dies yet. A monster's death clears it (see [`super::fight`]).
//! - The horse's rider (`GetVictim`): no horse has one, so a horse shows 100.
//! - The polymorph: no character is polymorphed, so none shows as the horse it was polymorphed into.
//! - The GM levels: every character is `GM_PLAYER`, so none targets anything on a castle map.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use common::vid::Vid;
use gamedata::mob_proto::CHAR_TYPE_MONSTER;
use protocol::gc_actors::GcTarget;
use world::npc::Npc;

use super::GameState;

/// `RACE_FLAG_ATT_ELEC` (`C/length.h:354`), the first element flag.
const RACE_FLAG_ATT_ELEC: u32 = 1 << 11;

/// `RACE_FLAG_ATT_DARK` (`C/length.h:359`), the last element flag.
const RACE_FLAG_ATT_DARK: u32 = 1 << 16;

/// How far from an element flag a monster's race flags may lie and still name it
/// (`abs(diff) <= 1024`).
const ELEMENT_REACH: i64 = 1024;

/// The horses `SetTarget` shows by their rider's horse health (`G/char.cpp:6510`).
const HORSE_RACES: std::ops::RangeInclusive<u32> = 20_101..=20_109;

/// `IS_CASTLE_MAP` (`G/castle.h:22`).
const CASTLE_MAPS: std::ops::RangeInclusive<i32> = 181..=183;

/// Who targets whom: `m_pkChrTarget` and `m_set_pkChrTargetedBy`, kept as one relation.
#[derive(Debug, Default)]
pub(super) struct Targets {
    /// Each targeting character's target, by the targeting VID.
    of: HashMap<u32, u32>,
    /// The characters targeting each target, by the target's VID, in VID order.
    by: BTreeMap<u32, BTreeSet<u32>>,
}

impl Targets {
    /// The target of `vid`, if it has one.
    pub(super) fn of(&self, vid: u32) -> Option<u32> {
        self.of.get(&vid).copied()
    }

    /// The characters targeting `target`, in VID order.
    pub(super) fn targeting(&self, target: u32) -> Vec<u32> {
        self.by
            .get(&target)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Makes `target` the target of `vid`, forgetting its old one.
    fn set(&mut self, vid: u32, target: Option<u32>) {
        if let Some(old) = self.of.remove(&vid) {
            self.forget(old, vid);
        }
        if let Some(target) = target {
            self.of.insert(vid, target);
            self.by.entry(target).or_default().insert(vid);
        }
    }

    /// Takes `vid` out of the targeted-by set of `target`.
    fn forget(&mut self, target: u32, vid: u32) {
        if let Some(set) = self.by.get_mut(&target) {
            set.remove(&vid);
            if set.is_empty() {
                self.by.remove(&target);
            }
        }
    }

    /// Takes away every target of a character that leaves, and the target of every character
    /// targeting it; answers the latter, in VID order.
    fn clear(&mut self, vid: u32) -> Vec<u32> {
        self.set(vid, None);
        let targeting: Vec<u32> = self
            .by
            .remove(&vid)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();
        for other in &targeting {
            self.of.remove(other);
        }
        targeting
    }
}

/// What `GC_TARGET` says of a target's hit points: the percentage, the hit points and the
/// maximum.
type HitPoints = (u8, i32, i32);

/// The hit points legacy shows for `hp` of `max_hp`: 0 for each when the maximum is not
/// positive, else `MINMAX(0, hp * 100 / max, 100)` with the two values.
fn shown_hit_points(hp: i32, max_hp: i32) -> HitPoints {
    if max_hp <= 0 {
        return (0, 0, 0);
    }
    let percent = (i64::from(hp) * 100 / i64::from(max_hp)).clamp(0, 100);
    (u8::try_from(percent).unwrap_or(0), hp, max_hp)
}

/// `bElement` for a monster's race flags (`G/char.cpp:6575-6592`): the first element flag the
/// flags lie within 1024 of, counted from 1 for `ATT_ELEC` to 6 for `ATT_DARK`, and 6 as well when
/// none is, where the loop runs out.
fn element_of(race_flag: u32) -> u8 {
    let mut element = 0;
    let mut flag = RACE_FLAG_ATT_ELEC;
    while flag <= RACE_FLAG_ATT_DARK {
        element += 1;
        if (i64::from(race_flag) - i64::from(flag)).abs() <= ELEMENT_REACH {
            break;
        }
        flag *= 2;
    }
    element
}

/// The hit points and element `SetTarget` sends for an NPC: a horse shows 100 for each, having
/// no rider, and a monster whose race flags reach `RACE_FLAG_ATT_ELEC` carries its element.
fn npc_target(npc: &Npc) -> (HitPoints, u8) {
    let shown = if npc.max_hp <= 0 {
        (0, 0, 0)
    } else if HORSE_RACES.contains(&npc.vnum) {
        (100, 100, 100)
    } else {
        shown_hit_points(npc.hp, npc.max_hp)
    };
    let element = if npc.char_type == CHAR_TYPE_MONSTER && npc.race_flag >= RACE_FLAG_ATT_ELEC {
        element_of(npc.race_flag)
    } else {
        0
    };
    (shown, element)
}

/// One `GC_TARGET`, with the alliance fields of `__SHIP_DEFENSE__` false and 0.
pub(super) fn target_record(vid: u32, (percent, hp, max_hp): HitPoints, element: u8) -> Vec<u8> {
    let mut record = Vec::new();
    GcTarget::new(vid, percent, hp, max_hp, 0, 0, 0, element).encode_into(&mut record);
    record
}

impl GameState {
    /// `CInputMain::Target` for the player under `vid`, which asked for `wanted`: no building
    /// object has a VID, so it is `SetTarget` with the character `wanted` names where the player
    /// stands, or with none.
    pub(super) fn target(&mut self, vid: Vid, wanted: u32) {
        let found = self.targeted(vid, wanted);
        let current = self.targets.of(vid.raw());
        if current == found {
            return;
        }
        // `IS_CASTLE_MAP(GetMapIndex()) && !IsGM()`: every character is `GM_PLAYER`.
        if self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| CASTLE_MAPS.contains(&map))
        {
            return;
        }
        self.targets.set(vid.raw(), found);
        let record = match found {
            Some(target) => {
                let (shown, element) = self.target_hit_points(target);
                target_record(target, shown, element)
            }
            None => target_record(0, (0, 0, 0), 0),
        };
        let _sent = self.write_to_client(vid, record);
    }

    /// The character `wanted` names on the Channel and map the player under `vid` stands on: a
    /// player with a body there, or an NPC standing there.
    fn targeted(&self, vid: Vid, wanted: u32) -> Option<u32> {
        // No character has VID 0, so it finds none.
        let place = self.place_of(vid.raw())?;
        if self.place_of(wanted) == Some(place) {
            return Some(wanted);
        }
        self.npc_of
            .get(&wanted)
            .filter(|&&(channel, map, _)| (channel, map) == place)
            .map(|_| wanted)
    }

    /// What `SetTarget` shows of `target`: a player's hit points, or an NPC's with its element.
    fn target_hit_points(&self, target: u32) -> (HitPoints, u8) {
        if let Some(npc) = self.npc_by_vid(target) {
            return npc_target(npc);
        }
        let shown = self
            .characters
            .find_by_vid(Vid::new(target))
            .ok()
            .and_then(world::character::Character::points)
            .map_or((0, 0, 0), |points| {
                shown_hit_points(points.hp(), points.max_hp())
            });
        (shown, 0)
    }

    /// `ClearTarget` for the character under `vid`, which leaves the world: its target forgets
    /// it, and each character targeting it is sent VID 0 and loses its target.
    pub(super) fn clear_target(&mut self, vid: Vid) {
        for other in self.targets.clear(vid.raw()) {
            let _sent = self.write_to_client(Vid::new(other), target_record(0, (0, 0, 0), 0));
        }
    }

    /// `BroadcastTargetPacket` for every player whose hit points changed since the last call:
    /// each change is sent, oldest first, to every character targeting the player.
    pub(super) fn broadcast_targets(&mut self) {
        let online: Vec<Vid> = self.outboxes.keys().copied().collect();
        for vid in online {
            let Some(points) = self
                .characters
                .find_by_vid_mut(vid)
                .ok()
                .and_then(|character| character.items_and_points_mut().1)
            else {
                continue;
            };
            let changes = points.take_target_broadcasts();
            if changes.is_empty() {
                continue;
            }
            let targeting = self.targets.targeting(vid.raw());
            for (hp, max_hp) in changes {
                let record = target_record(vid.raw(), shown_hit_points(hp, max_hp), 0);
                for &other in &targeting {
                    let _sent = self.write_to_client(Vid::new(other), record.clone());
                }
            }
        }
    }

    /// `BroadcastTargetPacket` of the non-player under `vid`, whose hit points have just changed:
    /// each character targeting it is sent its hit points, with no element.
    pub(super) fn broadcast_npc_target(&self, vid: u32) {
        let Some(npc) = self.npc_by_vid(vid) else {
            return;
        };
        let record = target_record(vid, shown_hit_points(npc.hp, npc.max_hp), 0);
        for other in self.targets.targeting(vid) {
            let _sent = self.write_to_client(Vid::new(other), record.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use common::point_slot as point;
    use gamedata::server_attr::SectreeGrid;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::npc::MapNpcs;

    use super::super::fixtures::{a_world, enter, enter_on, TestClock, PLACE};
    use super::*;
    use crate::game_loop::PulseProcessor;
    use crate::game_loop_messages::GameCommand;

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The `GC_TARGET` records among those an inbox was sent.
    fn targets_sent(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        drained(inbox)
            .into_iter()
            .filter(|record| record.first() == Some(&GcTarget::header()))
            .collect()
    }

    /// A monster of 400 hit points with the ice flag (element 3), standing on [`PLACE`].
    fn a_monster(vid: u32) -> Npc {
        Npc {
            vid,
            vnum: 101,
            race: 101,
            char_type: CHAR_TYPE_MONSTER,
            x: 500,
            y: 500,
            hp: 300,
            max_hp: 400,
            race_flag: 1 << 13,
            ..Npc::default()
        }
    }

    /// Stands `npcs` on [`PLACE`], as boot does.
    fn stand(world: &mut GameState, npcs: Vec<Npc>) {
        for (at, npc) in npcs.iter().enumerate() {
            world.npc_of.insert(npc.vid, (PLACE.0, PLACE.1, at));
        }
        world.npcs.insert(
            PLACE,
            Arc::new(MapNpcs {
                npcs,
                positions: Vec::new(),
                regens: Vec::new(),
            }),
        );
    }

    /// What `SetTarget` sends for the player under `vid`, read from its points.
    fn player_record(world: &GameState, vid: u32) -> Vec<u8> {
        let points = world
            .characters
            .find_by_vid(Vid::new(vid))
            .unwrap()
            .points()
            .unwrap();
        target_record(vid, shown_hit_points(points.hp(), points.max_hp()), 0)
    }

    /// Changes the hit points of the player under `vid` by `amount`, as `PointChange` does.
    fn change_hp(world: &mut GameState, vid: u32, amount: i32) {
        world
            .characters
            .find_by_vid_mut(Vid::new(vid))
            .unwrap()
            .items_and_points_mut()
            .1
            .unwrap()
            .point_change(point::POINT_HP, amount, false, false)
            .unwrap();
    }

    /// A world with the players 1 and 2 near each other on [`PLACE`] and the monster
    /// `0x8000_0001`, their inboxes drained.
    fn two_players() -> (
        GameState,
        UnboundedReceiver<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        stand(&mut world, vec![a_monster(0x8000_0001)]);
        let mut one = enter(&mut world, 1, (100, 100), 800);
        let mut two = enter(&mut world, 2, (200, 200), 800);
        world.broadcast_targets();
        drained(&mut one);
        drained(&mut two);
        (world, one, two)
    }

    #[test]
    fn a_player_targets_a_player_and_is_sent_its_hit_points_once() {
        let (mut world, mut one, mut two) = two_players();
        world.target(Vid::new(1), 2);
        assert_eq!(drained(&mut one), [player_record(&world, 2)]);
        assert!(drained(&mut two).is_empty());
        assert_eq!(world.targets.of(1), Some(2));
        assert_eq!(world.targets.targeting(2), [1]);
        // The same target again changes nothing and sends nothing.
        world.target(Vid::new(1), 2);
        assert!(drained(&mut one).is_empty());
    }

    #[test]
    fn a_monster_target_carries_its_hit_points_and_element() {
        let (mut world, mut one, _two) = two_players();
        world.target(Vid::new(1), 0x8000_0001);
        assert_eq!(
            drained(&mut one),
            [target_record(0x8000_0001, (75, 300, 400), 3)]
        );
        assert_eq!(world.targets.targeting(0x8000_0001), [1]);
    }

    #[test]
    fn a_vid_naming_nothing_clears_a_target_and_changes_nothing_without_one() {
        let (mut world, mut one, _two) = two_players();
        world.target(Vid::new(1), 999);
        assert!(drained(&mut one).is_empty());
        assert_eq!(world.targets.of(1), None);
        world.target(Vid::new(1), 2);
        drained(&mut one);
        world.target(Vid::new(1), 999);
        assert_eq!(drained(&mut one), [target_record(0, (0, 0, 0), 0)]);
        assert_eq!(world.targets.of(1), None);
        assert!(world.targets.targeting(2).is_empty());
        world.target(Vid::new(1), 2);
        drained(&mut one);
        world.target(Vid::new(1), 0);
        assert_eq!(drained(&mut one), [target_record(0, (0, 0, 0), 0)]);
    }

    #[test]
    fn a_character_on_another_map_or_channel_is_not_found() {
        let (mut world, mut one, _two) = two_players();
        let grid = SectreeGrid {
            x: 0,
            y: 0,
            columns: 10,
            rows: 10,
        };
        world.host_map(PLACE.0, 42, grid);
        world.host_map(99, PLACE.1, grid);
        let _three = enter_on(&mut world, (PLACE.0, 42), 3, (100, 100), 800);
        let _four = enter_on(&mut world, (99, PLACE.1), 4, (100, 100), 800);
        world.target(Vid::new(1), 3);
        world.target(Vid::new(1), 4);
        assert!(drained(&mut one).is_empty());
        assert_eq!(world.targets.of(1), None);
        // Nor is a monster of this map found from the other.
        world.target(Vid::new(3), 0x8000_0001);
        assert_eq!(world.targets.of(3), None);
    }

    #[test]
    fn nobody_targets_on_a_castle_map() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let grid = SectreeGrid {
            x: 0,
            y: 0,
            columns: 10,
            rows: 10,
        };
        for map in [180, 181, 183, 184] {
            world.host_map(PLACE.0, map, grid);
            let vid = u32::try_from(map).unwrap() * 10;
            let mut first = enter_on(&mut world, (PLACE.0, map), vid, (100, 100), 800);
            let _second = enter_on(&mut world, (PLACE.0, map), vid + 1, (200, 200), 800);
            drained(&mut first);
            world.target(Vid::new(vid), vid + 1);
            let castle = (181..=183).contains(&map);
            assert_eq!(drained(&mut first).is_empty(), castle, "map {map}");
            assert_eq!(world.targets.of(vid).is_none(), castle, "map {map}");
        }
    }

    #[test]
    fn a_leaver_is_forgotten_by_its_target_and_its_targeters_are_sent_vid_0() {
        let (mut world, mut one, mut two) = two_players();
        let mut three = enter(&mut world, 3, (300, 300), 800);
        world.target(Vid::new(1), 2);
        world.target(Vid::new(3), 1);
        world.target(Vid::new(2), 1);
        drained(&mut one);
        drained(&mut two);
        drained(&mut three);
        world.leave_world(Vid::new(1));
        assert_eq!(targets_sent(&mut two), [target_record(0, (0, 0, 0), 0)]);
        assert_eq!(targets_sent(&mut three), [target_record(0, (0, 0, 0), 0)]);
        assert!(world.targets.targeting(2).is_empty());
        assert_eq!(world.targets.of(2), None);
        assert_eq!(world.targets.of(3), None);
        // Targeting it again finds nobody.
        world.target(Vid::new(3), 1);
        assert!(drained(&mut three).is_empty());
    }

    #[test]
    fn each_hit_point_change_is_broadcast_to_the_targeters_in_order() {
        let (mut world, mut one, mut two) = two_players();
        let mut three = enter(&mut world, 3, (300, 300), 800);
        let mut four = enter(&mut world, 4, (400, 400), 800);
        world.broadcast_targets();
        world.target(Vid::new(3), 2);
        world.target(Vid::new(1), 2);
        drained(&mut one);
        drained(&mut three);
        drained(&mut four);
        change_hp(&mut world, 2, -10);
        let first = player_record(&world, 2);
        // `BroadcastTargetPacket` runs for a change of nothing too.
        change_hp(&mut world, 2, 0);
        change_hp(&mut world, 2, -15);
        let last = player_record(&world, 2);
        world.broadcast_targets();
        let expected = [first.clone(), first, last];
        assert_eq!(drained(&mut one), expected);
        assert_eq!(drained(&mut three), expected);
        assert!(targets_sent(&mut two).is_empty());
        assert!(targets_sent(&mut four).is_empty());
        // Each change goes once.
        world.broadcast_targets();
        assert!(drained(&mut one).is_empty());
    }

    #[test]
    fn a_pulse_ends_by_broadcasting_what_its_events_changed() {
        let (mut world, mut one, _two) = two_players();
        world.target(Vid::new(1), 2);
        drained(&mut one);
        change_hp(&mut world, 2, -10);
        world.process_pulse(1);
        assert_eq!(targets_sent(&mut one), [player_record(&world, 2)]);
    }

    #[test]
    fn the_target_command_answers_and_settles_and_a_command_broadcasts() {
        let (mut world, mut one, _two) = two_players();
        let (settled, mut settling) = tokio::sync::oneshot::channel();
        world.apply(GameCommand::Target {
            vid: Vid::new(2),
            target: 1,
            settled,
        });
        assert!(matches!(
            settling.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
        assert!(drained(&mut one).is_empty());
        change_hp(&mut world, 1, -5);
        let (settled, _settling) = tokio::sync::oneshot::channel();
        world.apply(GameCommand::Target {
            vid: Vid::new(1),
            target: 0x8000_0001,
            settled,
        });
        assert_eq!(
            drained(&mut one),
            [target_record(0x8000_0001, (75, 300, 400), 3)]
        );
    }

    #[test]
    fn the_shown_hit_points_clamp_the_percentage_and_zero_a_maximum_that_is_not_positive() {
        assert_eq!(shown_hit_points(50, 200), (25, 50, 200));
        assert_eq!(shown_hit_points(199, 200), (99, 199, 200));
        assert_eq!(shown_hit_points(400, 200), (100, 400, 200));
        assert_eq!(shown_hit_points(-5, 200), (0, -5, 200));
        assert_eq!(shown_hit_points(50, 0), (0, 0, 0));
        assert_eq!(shown_hit_points(50, -1), (0, 0, 0));
        // An `int` product would wrap here; the Rewrite's does not.
        assert_eq!(
            shown_hit_points(30_000_000, 30_000_000),
            (100, 30_000_000, 30_000_000)
        );
    }

    #[test]
    fn the_element_is_the_first_flag_within_1024_and_six_past_them_all() {
        // `RACE_FLAG_ANIMAL` to `RACE_FLAG_DESERT`, 1023 together.
        let kinds = (1 << 10) - 1;
        for (at, flag) in [11, 12, 13, 14, 15, 16].into_iter().enumerate() {
            let expected = u8::try_from(at + 1).unwrap();
            assert_eq!(element_of(1 << flag), expected, "flag {flag}");
            assert_eq!(element_of((1 << flag) | kinds), expected, "flag {flag}");
        }
        // `RACE_FLAG_TREE` with `ATT_ELEC` lies 1025 past it and 1023 short of `ATT_FIRE`.
        assert_eq!(element_of((1 << 11) | (1 << 10) | 1), 2);
        // Two element flags lie within 1024 of neither, and a boss flag carries any flag past
        // them all: legacy's loop ends and leaves 6.
        assert_eq!(element_of((1 << 11) | (1 << 12)), 6);
        assert_eq!(element_of((1 << 14) | (1 << 18)), 6);
        // 1024 below the next flag counts, 1025 does not.
        assert_eq!(element_of((1 << 12) - 1024), 1);
        assert_eq!(element_of((1 << 12) + 1024), 2);
        assert_eq!(element_of((1 << 12) + 1025), 6);
    }

    #[test]
    fn an_npc_target_shows_its_hit_points_a_horse_100_and_only_a_monster_an_element() {
        let mut npc = Npc {
            vnum: 101,
            char_type: CHAR_TYPE_MONSTER,
            hp: 30,
            max_hp: 120,
            race_flag: 1 << 13,
            ..Npc::default()
        };
        assert_eq!(npc_target(&npc), ((25, 30, 120), 3));
        npc.race_flag = (1 << 11) - 1;
        assert_eq!(npc_target(&npc), ((25, 30, 120), 0));
        // `RACE_FLAG_ATT_ELEC` itself reaches it.
        npc.race_flag = 1 << 11;
        assert_eq!(npc_target(&npc), ((25, 30, 120), 1));
        npc.char_type = gamedata::mob_proto::CHAR_TYPE_STONE;
        npc.race_flag = 1 << 13;
        assert_eq!(npc_target(&npc), ((25, 30, 120), 0));
        npc.vnum = 20_101;
        assert_eq!(npc_target(&npc), ((100, 100, 100), 0));
        npc.vnum = 20_110;
        assert_eq!(npc_target(&npc), ((25, 30, 120), 0));
        npc.vnum = 20_100;
        assert_eq!(npc_target(&npc), ((25, 30, 120), 0));
        npc.vnum = 20_109;
        assert_eq!(npc_target(&npc), ((100, 100, 100), 0));
        npc.max_hp = 0;
        assert_eq!(npc_target(&npc), ((0, 0, 0), 0));
    }

    #[test]
    fn the_relation_keeps_both_sides_and_a_leaver_clears_them() {
        let mut targets = Targets::default();
        targets.set(1, Some(9));
        targets.set(3, Some(9));
        targets.set(2, Some(1));
        assert_eq!(targets.targeting(9), [1, 3]);
        targets.set(1, Some(7));
        assert_eq!(targets.targeting(9), [3]);
        assert_eq!(targets.targeting(7), [1]);
        assert_eq!(targets.clear(1), [2]);
        assert_eq!(targets.of(1), None);
        assert_eq!(targets.of(2), None);
        assert!(targets.targeting(7).is_empty());
        assert_eq!(targets.of(3), Some(9));
        targets.set(3, None);
        assert!(targets.by.is_empty());
        assert!(targets.of.is_empty());
    }

    #[test]
    fn a_target_record_is_the_32_byte_gc_target() {
        let record = target_record(0x0102_0304, (25, 0x0506_0708, -2), 3);
        assert_eq!(record.len(), protocol::gc_actors::GC_TARGET_WIRE_SIZE);
        assert_eq!(record[0], GcTarget::header());
        assert_eq!(&record[1..5], &[4, 3, 2, 1]);
        assert_eq!(record[5], 25);
        assert_eq!(&record[6..10], &[8, 7, 6, 5]);
        assert_eq!(&record[10..14], &(-2_i32).to_le_bytes());
        assert_eq!(&record[14..31], &[0; 17]);
        assert_eq!(record[31], 3);
    }
}
