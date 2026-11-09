//! The stack attributes: a kill or a use feeds the worn costume skins
//! (`ITEM_STACK_ATTRIBUTE`, `G/item_stack_attribute.cpp:108-205`).
//!
//! Each table entry names a worn item by vnum and up to three bonus rows. A feed runs
//! `StackAttributeByWearIndex`: the costume weapon, then the costume pet
//! (`WEAR_COSTUME_WEAPON`, `WEAR_COSTUME_PET`, `C/length.h:234,242`). Each worn piece in
//! the table is fed row by row (`AddStackAttribute`, `:126-155`): a row whose feed vnum is
//! 0 or the fed vnum runs `SetStackAttributeIndex`, which stamps the unequip flood, then
//! feeds the one row its index names. The feed adds one to the row's odd slot (1, 3, 5),
//! recomputes the row's even slot (0, 2, 4) as the odd over the progress, with the row's
//! apply type, and `ApplyPoint`s the apply by 1 when the even slot changed — but only
//! while the odd slot sits below the ceiling.
//!
//! The three call sites are the kill (`G/char_battle.cpp:1524-1549`), which feeds monster,
//! stone or boss at the race when the killer is a PC near the victim's level, and the two
//! `UseItem` tails (`G/char_item.cpp:7375,7398`), which feed the used vnum as an item feed
//! when `UseItemEx` returned true. The flood gate
//! (`G/char_item.cpp:7180-7185`) refuses a use within a second of a feed with legacy's
//! line; the stamp lives on the body, as `m_dwUseItemStackAttrFlood` lives on the
//! character (`G/char.h:2415-2419`).
//!
//! The fish type (`STACK_ATTRIBUTE_BY_FISH`) has no call site: nothing in the checked-in
//! server feeds it. The row's semantics are the feed's, shared with every other type, so a
//! fish row would stack if one fed it; none does.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::enums::EWearPositions;
use common::vid::Vid;
use gamedata::mob_proto::{CHAR_TYPE_MONSTER, CHAR_TYPE_STONE, MOB_RANK_BOSS};
use gamedata::stack_attribute::{STACK_BY_BOSS, STACK_BY_ITEM, STACK_BY_MONSTER, STACK_BY_STONE};
use protocol::gc_chat::CHAT_TYPE_INFO;
use world::character::PointRecord;
use world::character::{force_worn_attribute, wear_pos, MoveDone, MoveKind, MoveRecord};
use world::item::Item;

use super::GameState;
use crate::item_move::{MovedItems, Mover};
use crate::save::PASSES_PER_SEC;

/// The fed cells, in the order `StackAttributeByWearIndex` feeds them
/// (`G/item_stack_attribute.cpp:114-117`).
const FED_CELLS: [EWearPositions; 2] = [EWearPositions::CostumeWeapon, EWearPositions::CostumePet];

/// The kill's level window (`G/char_battle.cpp:1527`): the killer's level within 15 of the
/// victim's, both ways strict.
const KILL_LEVEL_WINDOW: i32 = 15;

/// The minimum race a kill feeds (`G/char_battle.cpp:1525`): the race above 100.
const KILL_MIN_RACE: u32 = 100;

/// The line the flood gate chats (`G/char_item.cpp:7183`).
pub const STACK_FLOOD_LINE: &[u8] = b"You need to wait 1 second to do that.";

impl GameState {
    /// Share the stack-attribute table boot read. Without it a kill or a use feeds
    /// nothing: every worn skin keeps its attributes.
    #[must_use]
    pub fn with_stack_attributes(
        mut self,
        table: std::sync::Arc<gamedata::stack_attribute::StackAttributes>,
    ) -> Self {
        self.stack_attributes = table;
        self
    }

    /// Whether a `CG_ITEM_USE` may start: the flood gate (`G/char_item.cpp:7180-7185`).
    /// A use within a second of a feed is refused, and the stamp is the feed's own.
    pub(super) fn stack_use_allowed(&self, vid: Vid) -> bool {
        let Some(body) = self.bodies.get(&vid) else {
            return true;
        };
        let Some(stamped) = body.stack_flood else {
            return true;
        };
        stamped.saturating_add(u64::from(PASSES_PER_SEC)) <= self.last_pulse
    }

    /// Chat the flood gate's line (`G/char_item.cpp:7183`), changing nothing.
    pub(super) fn tell_stack_flood(&self, vid: Vid, mover: Mover) {
        self.tell(vid, mover, CHAT_TYPE_INFO, STACK_FLOOD_LINE, &[]);
    }

    /// The kill's stack feed (`G/char_battle.cpp:1524-1549`): a PC killer near the
    /// victim's level feeds monster, stone or boss at the race, in legacy's order. The
    /// feeds' updates reach the killer's client at once, and the rows go to the
    /// descriptor that holds the killer, as a `SetForceAttribute` sends and saves each.
    pub(super) fn stack_on_kill(&mut self, vid: u32, killer: Option<Vid>) {
        let Some(killer) = killer else {
            return;
        };
        let Some(npc) = self.npc_by_vid(vid) else {
            return;
        };
        let race = u32::from(npc.race);
        if race <= KILL_MIN_RACE {
            return;
        }
        let (char_type, rank, level) = (npc.char_type, npc.fight.rank, npc.fight.level);
        let killer_level = self
            .characters
            .find_by_vid(killer)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, |points| i32::from(points.level()));
        let victim_level = i32::from(level);
        if killer_level + KILL_LEVEL_WINDOW <= victim_level
            || killer_level - KILL_LEVEL_WINDOW >= victim_level
        {
            return;
        }
        let feed = if char_type == CHAR_TYPE_MONSTER && rank < MOB_RANK_BOSS {
            STACK_BY_MONSTER
        } else if char_type == CHAR_TYPE_STONE {
            STACK_BY_STONE
        } else if rank >= MOB_RANK_BOSS {
            STACK_BY_BOSS
        } else {
            return;
        };
        let fed_vnum = i32::try_from(race).unwrap_or(i32::MAX);
        self.stack_feed(killer, feed, fed_vnum);
    }

    /// The used item's stack feed (`G/char_item.cpp:7375,7398`): the used vnum feeds as
    /// an item feed, after a `UseItemEx` that returned true. The feeds' records and rows
    /// extend `moved`, after the step's own.
    pub(super) fn stack_on_use(
        &mut self,
        vid: Vid,
        mover: Mover,
        used_vnum: u32,
        trail: &mut MovedItems,
    ) {
        let fed_vnum = i32::try_from(used_vnum).unwrap_or(i32::MAX);
        let done = self.stack_moves(vid, STACK_BY_ITEM, fed_vnum);
        if done.records.is_empty() && done.changes.is_empty() {
            return;
        }
        let owner = trail.owner_id;
        let packed = MovedItems::new(owner, vid.raw(), done, mover, &self.protos, &self.locale);
        trail.records.extend(packed.records);
        trail.around.extend(packed.around);
        trail.changes.extend(packed.changes);
        if let Ok(character) = self.characters.find_by_vid(vid) {
            trail.points = character.points().cloned();
        }
    }

    /// `StackAttributeByWearIndex` (`G/item_stack_attribute.cpp:108-119`) on a kill: the
    /// costume weapon, then the costume pet, each fed `AddStackAttribute`'s rows. The
    /// updates reach the killer's client at once, and the rows are ordered to the
    /// descriptor that holds the killer.
    fn stack_feed(&mut self, killer: Vid, feed: i32, fed_vnum: i32) {
        let done = self.stack_moves(killer, feed, fed_vnum);
        if done.records.is_empty() && done.changes.is_empty() {
            return;
        }
        let Some(mover) = self.own_mover(killer) else {
            return;
        };
        let owner = self
            .characters
            .find_by_vid(killer)
            .map_or(0, world::character::Character::player_id);
        let packed = MovedItems::new(owner, killer.raw(), done, mover, &self.protos, &self.locale);
        for frame in packed.records {
            let _sent = self.write_to_client(killer, frame);
        }
        if packed.changes.is_empty() {
            return;
        }
        let Some((channel, map)) = self.place_of(killer.raw()) else {
            return;
        };
        let order = crate::item_move::StoreOrder::default();
        let _ordered = self.order_the_item_rows(killer, channel, map, packed.changes, order);
    }

    /// The feeds' `MoveDone`: `AddStackAttribute` (`G/item_stack_attribute.cpp:126-155`)
    /// of each fed cell, in order. Each row whose feed vnum is 0 or the fed vnum runs
    /// `SetStackAttributeIndex`; a piece no group names feeds nothing.
    fn stack_moves(&mut self, vid: Vid, feed: i32, fed_vnum: i32) -> MoveDone {
        let mut done = MoveDone {
            kind: MoveKind::Attributed,
            records: Vec::new(),
            changes: Vec::new(),
        };
        for cell in FED_CELLS {
            self.stack_feed_cell(vid, cell as u16, feed, fed_vnum, &mut done);
        }
        done
    }

    /// One cell's rows: each row whose feed vnum is 0 or the fed vnum runs
    /// `SetStackAttributeIndex` (`G/item_stack_attribute.cpp:156-205`).
    fn stack_feed_cell(
        &mut self,
        vid: Vid,
        cell: u16,
        feed: i32,
        fed_vnum: i32,
        done: &mut MoveDone,
    ) {
        let worn = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(wear_pos(cell)).cloned());
        let Some(worn) = worn else {
            return;
        };
        let rows: Vec<(i32, i32, i32, i32, i32)> = self
            .stack_attributes
            .get(i32::try_from(worn.vnum).unwrap_or(i32::MAX))
            .map(|entry| {
                entry
                    .rows
                    .iter()
                    .map(|row| {
                        (
                            row.feed_vnum,
                            row.stack_type,
                            row.apply,
                            row.progress,
                            row.ceiling,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (index, (feed_vnum, ..)) in rows.iter().enumerate() {
            if *feed_vnum != 0 && *feed_vnum != fed_vnum {
                continue;
            }
            self.stack_feed_row(vid, &worn, index, feed, done);
        }
    }

    /// `SetStackAttributeIndex`'s one row: the flood is stamped, and the row feeds only
    /// when its type matches and its odd slot (1, 3, 5) sits below its ceiling. The feed
    /// adds 1 to the odd slot, recomputes the even slot (0, 2, 4) as the odd over the
    /// progress with the row's apply type, and `ApplyPoint`s that apply by 1 when the
    /// even slot changed. Each `SetForceAttribute` sends and saves, in legacy's order.
    fn stack_feed_row(
        &mut self,
        vid: Vid,
        worn: &Item,
        index: usize,
        feed: i32,
        done: &mut MoveDone,
    ) {
        let row = self
            .stack_attributes
            .get(i32::try_from(worn.vnum).unwrap_or(i32::MAX))
            .and_then(|entry| entry.rows.get(index).copied());
        let Some(row) = row else {
            return;
        };
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.stack_flood = Some(self.last_pulse);
        }
        if row.stack_type != feed {
            return;
        }
        let (even, odd) = (2 * index, 2 * index + 1);
        let counter = i32::from(worn.attributes[odd].s_value);
        if counter >= row.ceiling {
            return;
        }
        let forced = counter.saturating_add(1);
        let next = if row.progress == 0 {
            0
        } else {
            forced / row.progress
        };
        let before = worn.attributes[even].s_value;
        let after = i16::try_from(next).unwrap_or(i16::MAX);
        let apply = u8::try_from(row.apply).unwrap_or(u8::MAX);
        let mut fed_rows = MoveDone {
            kind: MoveKind::Attributed,
            records: Vec::new(),
            changes: Vec::new(),
        };
        let mut points: Vec<PointRecord> = Vec::new();
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let (items, point_state) = character.items_and_points_mut();
            let step = force_worn_attribute(
                items,
                worn.id,
                (odd, 0, i16::try_from(forced).unwrap_or(i16::MAX)),
            );
            fed_rows.records.extend(step.records);
            fed_rows.changes.extend(step.changes);
            let step = force_worn_attribute(items, worn.id, (even, apply, after));
            fed_rows.records.extend(step.records);
            fed_rows.changes.extend(step.changes);
            if before != after {
                if let Some(point_state) = point_state {
                    points = point_state.apply_point(apply, 1);
                }
            }
        }
        for record in points {
            fed_rows.records.push(MoveRecord::Point(record));
        }
        done.records.extend(fed_rows.records);
        done.changes.extend(fed_rows.changes);
    }
}

#[cfg(test)]
mod tests {
    use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
    use protocol::gc_item_window::ItemAttribute;
    use protocol::item_pos::ItemPos;
    use world::character::MoveKind as Kind;
    use world::item::Item;

    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock};

    /// The wearer.
    const VID: u32 = 0x6001;

    /// The owner's skin: `Skin-Aquarian-Pumnal`, vnum 40130.
    const SKIN: u32 = 40_130;

    /// The owner's file, read as the table: one group, the monster, stone and boss rows.
    const OWNER_FILE: &[u8] = b"Group\tSkin-Aquarian-Pumnal\n{\n\tvnum\t40130\n\t1\t0\t0\t63\t25\t2500\n\t2\t0\t1\t61\t20\t2000\n\t3\t591\t2\t63\t15\t1500\n}\n";

    fn cell(wear: EWearPositions) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, INVENTORY_MAX_NUM + wear as u16)
    }

    /// A world with the owner's table, the wearer entered with the skin on the costume
    /// weapon cell holding `attrs`, and a potion in cell 0 for the use paths.
    fn skinned(attrs: [ItemAttribute; world::item::ATTRIBUTES]) -> GameState {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let table = gamedata::stack_attribute::read(OWNER_FILE).expect("the table reads");
        world.stack_attributes = std::sync::Arc::new(table);
        let mut skin = Item::new(7, SKIN);
        skin.attributes = attrs;
        let potion = Item::new(9, 27_001);
        let _ = enter_seeing_with(
            &mut world,
            crate::game_state::fixtures::PLACE,
            VID,
            (100, 100),
            12_000,
            &[
                (cell(EWearPositions::CostumeWeapon), skin),
                (ItemPos::new(EWindows::Inventory as u8, 0), potion),
            ],
        );
        world
    }

    fn attrs_of(world: &GameState) -> [ItemAttribute; world::item::ATTRIBUTES] {
        world
            .characters
            .find_by_vid(Vid::new(VID))
            .expect("the wearer")
            .items()
            .item_at(cell(EWearPositions::CostumeWeapon))
            .expect("the skin")
            .attributes
    }

    /// A monster feed adds one to slot 1 and writes slot 0 as the counter over the
    /// progress: 24 feeds to 25, so slot 0 turns from 0 to 1 with apply 63, and the
    /// point moves. A 25th feed moves the counter to 26 with slot 0 unchanged and no
    /// point. Slot 5's stone row and slot 3's boss row keep their zeros throughout.
    #[test]
    fn a_monster_feed_counts_and_grants_at_the_progress() {
        let mut attrs = [ItemAttribute::new(0, 0); world::item::ATTRIBUTES];
        attrs[1] = ItemAttribute::new(0, 24);
        let mut world = skinned(attrs);
        let done = world.stack_moves(Vid::new(VID), STACK_BY_MONSTER, 101);
        assert_eq!(done.kind, Kind::Attributed);
        let attrs = attrs_of(&world);
        assert_eq!(
            [
                (attrs[0].b_type, attrs[0].s_value),
                (attrs[1].b_type, attrs[1].s_value)
            ],
            [(63, 1), (0, 25)],
            "slot 0 grants apply 63 at 25 over 25"
        );
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Point(_))),
            "the 24th feed moves the point: {done:?}"
        );
        let done = world.stack_moves(Vid::new(VID), STACK_BY_MONSTER, 101);
        let attrs = attrs_of(&world);
        assert_eq!(attrs[1].s_value, 26);
        assert_eq!(attrs[0].s_value, 1);
        assert!(
            done.records
                .iter()
                .all(|record| !matches!(record, MoveRecord::Point(_))),
            "the 25th feed moves no point: {done:?}"
        );
    }

    /// The boss row feeds only at vnum 591: a boss feed at the dog's race stamps the
    /// flood and feeds nothing, while the same feed at 591 counts slot 5.
    #[test]
    fn a_boss_feed_needs_its_vnum() {
        let world = &mut skinned([ItemAttribute::new(0, 0); world::item::ATTRIBUTES]);
        let done = world.stack_moves(Vid::new(VID), STACK_BY_BOSS, 101);
        assert!(done.records.is_empty(), "no row runs: {done:?}");
        assert!(done.changes.is_empty());
        assert!(
            !world.stack_use_allowed(Vid::new(VID)),
            "the stamp lands even when the type misses"
        );
        let done = world.stack_moves(Vid::new(VID), STACK_BY_BOSS, 591);
        assert_eq!(attrs_of(world)[5].s_value, 15 - 14, "one feed counts one");
        assert_eq!(done.changes.len(), 2, "the counter and the bonus save");
    }

    /// The ceiling holds: at 2500 the monster row feeds nothing more, and the flood
    /// gate refuses a use within the second, then lets one through a second out.
    #[test]
    fn the_ceiling_holds_and_the_flood_refuses_a_use() {
        let mut attrs = [ItemAttribute::new(0, 0); world::item::ATTRIBUTES];
        attrs[1] = ItemAttribute::new(0, 2500);
        let mut world = skinned(attrs);
        let done = world.stack_moves(Vid::new(VID), STACK_BY_MONSTER, 101);
        assert!(
            done.records.is_empty(),
            "the ceiling feeds nothing: {done:?}"
        );
        assert!(
            !world.stack_use_allowed(Vid::new(VID)),
            "the stamp lands even at the ceiling"
        );
        world.last_pulse += u64::from(PASSES_PER_SEC) - 1;
        assert!(
            !world.stack_use_allowed(Vid::new(VID)),
            "24 Pulses on, still shut"
        );
        world.last_pulse += 1;
        assert!(world.stack_use_allowed(Vid::new(VID)), "a second out, open");
    }
}
