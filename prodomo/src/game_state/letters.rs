//! The letters event: `/give_reward` and `/letters_open` (`G/cmd_general.cpp:3753-3806`).
//!
//! [`GameState::run_give_reward`] is `ACMD(give_reward)` (`:3761-3806`): the busy gates
//! (an open safebox, a browsed shop, a trade, the dead), the `letters_event` flag, and
//! the six letters (30216, 30213, 30219, 30214, 30217, 30210). A missing letter is only
//! a line: the trade's own count still runs, so a caller whose letters vanished between
//! the line and the trade takes what is there. The six go by `RemoveSpecifyItem`, and a
//! `GetMultiIndex` draw of group 55021 (`BOX_VNUM_L`, `G/LettersConfig.h:5`) lands by
//! `AutoGiveItem`. The cube and the personal shop are dead gates: no ported path opens
//! either. The caller answers the stored rows before the client hears of them.
//! [`GameState::run_letters_open`] is `ACMD(letters_open)` (`:3753-3760`): one
//! `letters_drop <vnum> <count> <index> <size>` line per group row, then `OpenLetterWindow`.
//! With no group (the owner's file holds none), the window opens alone, as the null arm does.

use std::time::{SystemTime, UNIX_EPOCH};

use common::item_slots::usable_inventory_cells;
use common::vid::Vid;

use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};

use world::character::{
    auto_give_item, count_specify_item, make_drop_item, remove_specify_item, AutoGiven, Character,
    DropAsk, DropMade, MoveDone, MoveKind, MoveRules,
};
use world::item::Item;

use crate::chat_line::Arg;
use crate::game_state::GameState;
use crate::item_move::Mover;

/// The box group `give_reward` draws from (`BOX_VNUM_L`, `G/LettersConfig.h:5`).
const BOX_VNUM: i32 = 55021;

/// The six letter vnums, in the order both counts and the removals name them
/// (`G/cmd_general.cpp:3772-3801`).
const LETTER_VNUMS: [u32; 6] = [30216, 30213, 30219, 30214, 30217, 30210];

/// What the trade stored: the box draw's vnum and count, or `None` when the draw landed
/// nothing (no group, no proto, or a keyword row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LettersTrade {
    /// The PID whose rows changed.
    pub pid: u32,
    /// The stored row changes, in order.
    pub changes: Vec<db::items::RowChange>,
    /// The records the caller hears, in order.
    pub records: Vec<Vec<u8>>,
}

impl GameState {
    /// `ACMD(give_reward)`: the gates, the six letters, and the box draw. Answers the
    /// trade's stored rows and records, or `None` for a refusal the caller already heard.
    pub(super) fn run_give_reward(&mut self, vid: Vid, mover: Mover) -> Option<LettersTrade> {
        if self.safebox_open(vid)
            || self.browsing.contains_key(&vid)
            || self.trading.contains_key(&vid)
            || self.is_dead(vid.raw())
        {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"@@(cmd_general)tradus:You cannot do that",
                &[],
            );
            return None;
        }
        let flag = self
            .quests
            .as_ref()
            .is_some_and(|quests| quests.manager().event_flag(b"letters_event") != 0);
        if !flag {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"@@(cmd_general)tradus:The event is not available",
                &[],
            );
            return None;
        }
        let held = self
            .characters
            .find_by_vid(vid)
            .map(|character| LETTER_VNUMS.map(|vnum| Self::count_letter(character, vnum)))
            .unwrap_or([0; 6]);
        if held.iter().any(|&count| count <= 0) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"@@(cmd_general)tradus:You do not have all the necessary letters",
                &[],
            );
        }
        if held.iter().any(|&count| count < 1) {
            return None;
        }
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, Character::player_id);
        let questing = self.quest_running(vid);
        let mut packed = MoveDone {
            kind: MoveKind::Removed,
            records: Vec::new(),
            changes: Vec::new(),
        };
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let usable = usable_inventory_cells(character.inven_point());
            for vnum in LETTER_VNUMS {
                let protos = self.protos.clone();
                let (items, _, slots) = character.items_points_and_quickslots_mut();
                let done = remove_specify_item(items, slots, &protos, usable, (vnum, 1));
                packed.records.extend(done.records);
                packed.changes.extend(done.changes);
            }
        }
        if let Some(given) = self.give_box_draw(vid, questing) {
            packed.records.extend(given.records);
            packed.changes.extend(given.changes);
        }
        let packed = crate::item_move::MovedItems::new(
            pid,
            vid.raw(),
            MoveDone {
                kind: MoveKind::Removed,
                records: packed.records,
                changes: packed.changes,
            },
            mover,
            &self.protos,
            &self.locale,
        );
        Some(LettersTrade {
            pid,
            changes: packed.changes,
            records: packed.records,
        })
    }

    /// `CountSpecifyItem` of one letter vnum for the trade's gates.
    fn count_letter(character: &Character, vnum: u32) -> i32 {
        let usable = usable_inventory_cells(character.inven_point());
        count_specify_item(character.items(), usable, vnum)
    }

    /// The box draw: a `GetMultiIndex` roll of group 55021, given by `AutoGiveItem`
    /// (`GiveItemFromSpecialItemGroup`, `G/char_item.cpp:9575-9690`). Only the default
    /// arm lands: the six keyword rows (gold, exp, mob, slow, drain, poison, mob group)
    /// wait for their systems, and a `PCT` bag's rows each roll their own weight, as
    /// `GetMultiIndex` does.
    fn give_box_draw(&mut self, vid: Vid, questing: bool) -> Option<MoveDone> {
        let group = self.item_groups.special.get(&BOX_VNUM)?.clone();
        if group.group_size() == 0 {
            return None;
        }
        let rolls: Vec<i32> = (0..group.group_size())
            .map(|_| world::character::number(&mut self.dice, 1, 100))
            .collect();
        // A `PCT` bag answers one index per passed roll; any other bag answers one.
        // The owner's data has no group, so the loop below runs nowhere yet.
        let mut out: Option<MoveDone> = None;
        for index in group.multi_index(&rolls) {
            let (Some(vnum), Some(count)) = (group.vnum(index), group.count(index)) else {
                continue;
            };
            if vnum <= 7 {
                // A keyword row (`EGiveType`): its system is not ported, so the draw
                // lands nothing, as a refused give does.
                continue;
            }
            let rare_pct = group.rare_pct(index).unwrap_or(0);
            let count = u16::try_from(count.max(0)).unwrap_or(u16::MAX);
            let Some((outcome, done)) = self.auto_give(vid, (vnum, count), rare_pct, questing)
            else {
                break;
            };
            match outcome {
                AutoGiven::NoProto | AutoGiven::NotMade => continue,
                AutoGiven::Merged(_) | AutoGiven::Placed { .. } => {}
                AutoGiven::Unplaced(item) => self.lay_auto_given(vid, item),
            }
            match out.as_mut() {
                Some(out) => {
                    out.records.extend(done.records);
                    out.changes.extend(done.changes);
                }
                None => out = Some(done),
            }
        }
        out
    }

    /// `CHARACTER::AutoGiveItem(vnum, count)` (`G/char_item.cpp:9081-9223`) on the
    /// character `vid`: the stacks, a free cell, or an unplaced item the caller lays with
    /// [`Self::lay_auto_given`]. `CreateItem` is the drop's with `rare_pct`. `None` when
    /// the character is gone or no item IDs are installed, so nothing could be made.
    pub(super) fn auto_give(
        &mut self,
        vid: Vid,
        (vnum, count): (u32, u16),
        rare_pct: i32,
        questing: bool,
    ) -> Option<(AutoGiven, MoveDone)> {
        let character = self.characters.find_by_vid_mut(vid).ok()?;
        let usable = usable_inventory_cells(character.inven_point());
        let rules = MoveRules {
            count_limit: self.item_count_limit,
            usable_cells: usable,
            belt_grade: None,
            questing,
        };
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            });
        let attrs = std::sync::Arc::clone(&self.item_attrs);
        let masks = std::sync::Arc::clone(&self.item_masks);
        let protos = std::sync::Arc::clone(&self.protos);
        let made = DropMade {
            attrs: &attrs,
            masks: masks.pairs(),
            now_secs,
            count_limit: self.item_count_limit,
        };
        let Some(ids) = self.item_ids.as_mut() else {
            tracing::warn!(vnum, "an item was given; no item ids are installed");
            return None;
        };
        let dice = &mut self.dice;
        let gauss = &mut self.gauss;
        let create = |left: u16| {
            let ask = DropAsk {
                count: i32::from(left),
                rare_pct,
            };
            match make_drop_item(dice, gauss, ids, &protos, &made, vnum, ask) {
                Ok(item) => Some(item),
                Err(refused) => {
                    tracing::warn!(vnum, ?refused, "cannot create item by vnum");
                    None
                }
            }
        };
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        Some(auto_give_item(
            items,
            slots,
            &rules,
            &protos,
            (vnum, count),
            create,
        ))
    }

    /// An item `AutoGiveItem` could not place lies at the caller's feet, owned for 300
    /// seconds when it cannot be dropped and 60 otherwise (`G/char_item.cpp:9214-9217`). It
    /// has no row until it is picked up, as a monster's drop has none.
    pub(super) fn lay_auto_given(&mut self, vid: Vid, item: Item) {
        let secs = if item.anti_flags & world::item::ITEM_ANTIFLAG_DROP != 0 {
            300
        } else {
            60
        };
        self.lay_owned(vid, item, secs);
    }

    /// `AddToGround` at the character's feet, owned by it for `secs` seconds.
    pub(super) fn lay_owned(&mut self, vid: Vid, item: Item, secs: u16) {
        let Some((channel, map)) = self.place_of(vid.raw()) else {
            return;
        };
        let spot = self.spot_of(vid.raw());
        let (x, y) = spot.map_or((0, 0), |spot| (spot.x, spot.y));
        let z = spot.map_or(0, |spot| spot.z);
        let Some(ground) = self.lay_drop_hit(channel, map, (x, y, z), item) else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let Some(body) = self.bodies.get(&vid) else {
            return;
        };
        let name = crate::loading_phase::name_field(&body.card.name);
        self.set_ground_ownership(ground, Some((pid, name)), i32::from(secs));
    }

    /// `ACMD(letters_open)`: the box table's lines, then the window.
    pub(super) fn run_letters_open(&mut self, vid: Vid, mover: Mover) {
        if let Some(group) = self.item_groups.special.get(&BOX_VNUM) {
            let size = group.group_size();
            for index in 0..size {
                let (Some(vnum), Some(count)) = (group.vnum(index), group.count(index)) else {
                    continue;
                };
                let index = i64::try_from(index).unwrap_or(i64::MAX);
                let size = i64::try_from(size).unwrap_or(i64::MAX);
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_COMMAND,
                    b"letters_drop %d %d %d %d",
                    &[
                        Arg::Int(i64::from(vnum)),
                        Arg::Int(i64::from(count)),
                        Arg::Int(index),
                        Arg::Int(size),
                    ],
                );
            }
        }
        self.tell(vid, mover, CHAT_TYPE_COMMAND, b"OpenLetterWindow", &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::{BOX_VNUM, LETTER_VNUMS};
    use crate::game_state::fixtures::{a_world, enter, enter_seeing_with, TestClock, PLACE};
    use common::item_slots::EWindows;
    use common::vid::Vid;
    use gamedata::mob_locale_names::MobNamesByLanguage;
    use protocol::item_pos::ItemPos;
    use world::item::Item;

    const VID: u32 = 0x6001;

    fn mover() -> crate::item_move::Mover {
        crate::item_move::Mover {
            recently_fought: false,
            empire: 1,
            language: 0,
            pk_mode: 4,
            affect_flags: [0; 2],
            dead: false,
            alignment: 0,
            view_guild: (0, 0),
        }
    }

    fn text_of(line: &[u8]) -> Vec<u8> {
        let size = usize::from(u16::from_le_bytes([line[1], line[2]]));
        assert_eq!(size, line.len());
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    fn with_quests(world: &mut crate::game_state::GameState) {
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(crate::game_state::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            MobNamesByLanguage::default(),
        ));
    }

    fn letters() -> Vec<(ItemPos, Item)> {
        LETTER_VNUMS
            .iter()
            .enumerate()
            .map(|(cell, &vnum)| {
                let cell = u16::try_from(cell).expect("six letters fit a cell");
                let mut letter = Item::new(100 + u32::from(cell), vnum);
                letter.set_size(1).expect("a positive footprint");
                (ItemPos::new(EWindows::Inventory as u8, cell), letter)
            })
            .collect()
    }

    /// Without the event flag the trade refuses and the letters stay.
    #[test]
    fn without_the_flag_the_reward_is_not_available() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let held = letters();
        let (mut inbox, _shown) = enter_seeing_with(&mut world, PLACE, VID, (5, 5), 10_000, &held);
        with_quests(&mut world);
        let trade = world.run_give_reward(Vid::new(VID), mover());
        assert!(trade.is_none(), "no flag, no trade");
        assert_eq!(
            text_of(&inbox.try_recv().expect("the refusal")),
            b"@@(cmd_general)tradus:The event is not available"
        );
        assert!(inbox.try_recv().is_err(), "nothing else");
        let character = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("the caller stays");
        for &vnum in &LETTER_VNUMS {
            assert_eq!(
                super::GameState::count_letter(character, vnum),
                1,
                "letter {vnum} stays"
            );
        }
    }

    /// With the flag but a missing letter the caller hears the letter line and the
    /// letters it holds stay.
    #[test]
    fn without_every_letter_the_caller_hears_the_letter_line() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut held = letters();
        held.pop();
        let (mut inbox, _shown) = enter_seeing_with(&mut world, PLACE, VID, (5, 5), 10_000, &held);
        with_quests(&mut world);
        world
            .quests
            .as_mut()
            .expect("quests loaded")
            .manager_mut()
            .set_event_flag(b"letters_event", 1);
        let trade = world.run_give_reward(Vid::new(VID), mover());
        assert!(trade.is_none(), "a missing letter, no trade");
        assert_eq!(
            text_of(&inbox.try_recv().expect("the letter line")),
            b"@@(cmd_general)tradus:You do not have all the necessary letters"
        );
        assert!(inbox.try_recv().is_err(), "nothing else");
    }

    /// With the flag and every letter the six go, the rows answer for the store, and
    /// the draw lands nothing: the owner's group file holds no 55021 group.
    #[test]
    fn with_every_letter_the_six_go_and_the_draw_lands_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let held = letters();
        let (mut inbox, _shown) = enter_seeing_with(&mut world, PLACE, VID, (5, 5), 10_000, &held);
        with_quests(&mut world);
        world
            .quests
            .as_mut()
            .expect("quests loaded")
            .manager_mut()
            .set_event_flag(b"letters_event", 1);
        let trade = world
            .run_give_reward(Vid::new(VID), mover())
            .expect("the trade runs");
        assert_eq!(trade.changes.len(), 6, "one row change per letter");
        assert_eq!(trade.records.len(), 6, "one record per letter");
        assert!(inbox.try_recv().is_err(), "the trade itself is silent");
        let character = world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("the caller stays");
        for &vnum in &LETTER_VNUMS {
            assert_eq!(
                super::GameState::count_letter(character, vnum),
                0,
                "letter {vnum} goes"
            );
        }
    }

    /// A caller with its safebox open hears the busy line and the letters stay.
    #[test]
    fn a_busy_caller_hears_the_busy_line() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let held = letters();
        let (mut inbox, _shown) = enter_seeing_with(&mut world, PLACE, VID, (5, 5), 10_000, &held);
        with_quests(&mut world);
        world
            .quests
            .as_mut()
            .expect("quests loaded")
            .manager_mut()
            .set_event_flag(b"letters_event", 1);
        let (safebox, _skipped) = world::character::Safebox::open(
            protocol::gc_safebox::StoreWindow::Safebox,
            1,
            7,
            Vec::new(),
        );
        world
            .storages
            .entry(Vid::new(VID))
            .or_default()
            .set_safebox_for_test(safebox);
        let trade = world.run_give_reward(Vid::new(VID), mover());
        assert!(trade.is_none(), "busy, no trade");
        assert_eq!(
            text_of(&inbox.try_recv().expect("the busy line")),
            b"@@(cmd_general)tradus:You cannot do that"
        );
        assert!(inbox.try_recv().is_err(), "nothing else");
    }

    /// With no group the window opens alone, as the null arm does.
    #[test]
    fn without_a_group_the_window_opens_alone() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (5, 5), 10_000);
        assert!(
            !world.item_groups.special.contains_key(&BOX_VNUM),
            "the default table holds no box"
        );
        world.run_letters_open(Vid::new(VID), mover());
        assert_eq!(
            text_of(&inbox.try_recv().expect("the window")),
            b"OpenLetterWindow"
        );
        assert!(inbox.try_recv().is_err(), "no table, no lines");
    }

    /// With a group the caller hears one `letters_drop` line per row, then the window.
    #[test]
    fn with_a_group_the_table_lists_before_the_window() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (5, 5), 10_000);
        let mut group = gamedata::special_item_group::SpecialItemGroup::default();
        group.add_item(100, 2, 50, 0);
        group.add_item(200, 1, 50, 0);
        let groups = gamedata::special_item_group::SpecialItemGroups {
            special: [(BOX_VNUM, group)].into_iter().collect(),
            ..Default::default()
        };
        world.item_groups = std::sync::Arc::new(groups);
        world.run_letters_open(Vid::new(VID), mover());
        assert_eq!(
            text_of(&inbox.try_recv().expect("the first row")),
            b"letters_drop 100 2 0 2"
        );
        assert_eq!(
            text_of(&inbox.try_recv().expect("the second row")),
            b"letters_drop 200 1 1 2"
        );
        assert_eq!(
            text_of(&inbox.try_recv().expect("the window")),
            b"OpenLetterWindow"
        );
        assert!(inbox.try_recv().is_err(), "nothing else");
    }
}
