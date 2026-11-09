//! `CHARACTER::AutoGiveItemNew` (`G/char_item.cpp:10725-10829`), the storage half of the
//! direct pick-up a kill's drop takes when its owner holds `AFFECT_PICKUP_ENABLE`
//! (`RENEWAL_PICKUP_AFFECT`, `G/char_battle.cpp:874`, `:969`).
//!
//! A stackable item (`IsStackable`: the item's `STACKABLE` flag) that is not `ITEM_BLEND`
//! tops up every stack of its vnum whose six sockets equal its own, in cell order over the
//! inventory, the equipment and the belt cells (`0..INVENTORY_AND_EQUIP_SLOT_MAX`), up to the
//! stack limit. Taken whole that way, the give ends. Otherwise what is left goes to the first
//! empty cell: a dragon soul stone into its box of the dragon soul inventory
//! (`GetEmptyDragonSoulInventory`, only for a qualified character), anything else into the
//! unlocked base inventory (`GetEmptyInventory`, which never reads the banks). The item is set
//! highlighted, as a drop was never held. With no cell it is answered unplaced, for the
//! ground. No quickslot is filled, unlike `AutoGiveItem`.

use common::item_slots::{EWindows, INVENTORY_AND_EQUIP_SLOT_MAX};
use gamedata::item_kind::{ITEM_BLEND, ITEM_DS};
use gamedata::item_proto::ItemProtos;
use protocol::item_pos::ItemPos;

use super::item_move::{ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, MoveRules};
use super::items::CharacterItems;
use crate::item::{Item, ITEM_FLAG_STACKABLE};

/// `DRAGON_SOUL_BOX_SIZE`: the cells of one dragon soul box.
const DRAGON_SOUL_BOX_SIZE: u16 = 32;

/// `DRAGON_SOUL_GRADE_MAX` with `ENABLE_DRAGONSOUL_ALCHEMY_PLUS`: the boxes of one type.
const DRAGON_SOUL_GRADE_MAX: u16 = 6;

/// Where [`auto_give_item_new`] left the item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GivenNew {
    /// The whole count went onto the stacks; the item is gone.
    Merged,
    /// The item, with what the stacks left, is in this cell.
    Placed(ItemPos),
    /// No cell took it: the item, with what the stacks left, goes to the ground.
    Unplaced(Item),
}

/// `AutoGiveItemNew(item)` for the character's storage. `ds_qualified` is
/// `DragonSoul_IsQualified()`. Answers where the item went and the records and rows.
pub fn auto_give_item_new(
    items: &mut CharacterItems,
    rules: &MoveRules,
    protos: &ItemProtos,
    mut item: Item,
    ds_qualified: bool,
) -> (GivenNew, MoveDone) {
    let mut done = MoveDone {
        kind: MoveKind::Given,
        records: Vec::new(),
        changes: Vec::new(),
    };
    let proto = protos.get(item.vnum);
    let blend = proto.is_some_and(|proto| proto.item_type == ITEM_BLEND);
    if item.flags & ITEM_FLAG_STACKABLE != 0 && !blend {
        for cell in 0..INVENTORY_AND_EQUIP_SLOT_MAX {
            let pos = ItemPos::new(EWindows::Inventory as u8, cell);
            let Some(stack) = items.item_at(pos) else {
                continue;
            };
            if stack.vnum != item.vnum || stack.sockets != item.sockets {
                continue;
            }
            let (id, held) = (stack.id, stack.count);
            let taken = rules.count_limit.saturating_sub(held).min(item.count);
            if taken == 0 {
                continue;
            }
            let Ok(count) = items.set_count(id, u32::from(held + taken)) else {
                continue;
            };
            item.count -= taken;
            if let Some(stack) = items.item(id) {
                done.records
                    .push(MoveRecord::Item(ItemRecord::Update(stack.gc_item_update())));
            }
            done.changes.push(ItemChange::Count { id, count });
            if item.count == 0 {
                return (GivenNew::Merged, done);
            }
        }
    }
    let dragon_soul = proto.is_some_and(|proto| proto.item_type == ITEM_DS);
    let pos = if dragon_soul {
        dragon_soul_buy_cell(
            items,
            &item,
            proto.map_or(0, |proto| proto.sub_type),
            ds_qualified,
        )
    } else {
        items
            .find_free_inventory_cell(rules.usable_cells, item.size)
            .map(|cell| ItemPos::new(EWindows::Inventory as u8, cell))
    };
    let Some(pos) = pos else {
        return (GivenNew::Unplaced(item), done);
    };
    item.pos = pos;
    if items.set(pos, &item).is_err() {
        return (GivenNew::Unplaced(item), done);
    }
    // A drop was never held, so `AddToCharacter` highlights its cell.
    done.records
        .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 1))));
    done.changes.push(ItemChange::Created(item));
    (GivenNew::Placed(pos), done)
}

/// `GetEmptyDragonSoulInventory` (`G/char_item.cpp:1270-1288`): the first empty cell of the
/// stone's box, `GetBasePosition` (`G/DragonSoul.cpp:130-147`) being the sub-type's column of
/// six boxes and the grade (`(vnum / 1000) % 10`) its row. A grade past 6 has no box.
/// The give and the shop buy share it.
pub fn dragon_soul_buy_cell(
    items: &CharacterItems,
    item: &Item,
    sub_type: i32,
    qualified: bool,
) -> Option<ItemPos> {
    if !qualified {
        return None;
    }
    let grade = u16::try_from((item.vnum / 1000) % 10).ok()?;
    if grade > DRAGON_SOUL_GRADE_MAX {
        return None;
    }
    let column = u16::try_from(sub_type).ok()?;
    let base = column
        .checked_mul(DRAGON_SOUL_GRADE_MAX * DRAGON_SOUL_BOX_SIZE)?
        .checked_add(grade * DRAGON_SOUL_BOX_SIZE)?;
    (base..base + DRAGON_SOUL_BOX_SIZE)
        .map(|cell| ItemPos::new(EWindows::DragonSoulInventory as u8, cell))
        .find(|pos| items.item_at(*pos).is_none())
}

#[cfg(test)]
mod tests {
    use gamedata::item_proto::ItemProto;

    use super::*;

    const POTION: u32 = 27_001;
    /// A grade-2 stone (`(vnum / 1000) % 10`) of sub-type 1.
    const STONE: u32 = 112_200;

    const RULES: MoveRules = MoveRules {
        count_limit: 200,
        usable_cells: 2,
        belt_grade: None,
        questing: false,
        secured: false,
    };

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    fn protos() -> ItemProtos {
        ItemProtos::from_rows(vec![
            ItemProto::for_category_rule(POTION, 3, 0),
            ItemProto::for_category_rule(STONE, ITEM_DS, 1),
        ])
    }

    fn potions(id: u32, count: u16) -> Item {
        let mut item = Item::new(id, POTION);
        item.count = count;
        item.flags = ITEM_FLAG_STACKABLE;
        item
    }

    /// The stacks of the vnum take what they can in cell order; with every cell of the
    /// unlocked inventory taken, the rest is answered for the ground, its count what the
    /// stacks left, and the rows name only the stacks.
    #[test]
    fn what_the_stacks_leave_with_no_free_cell_is_unplaced() {
        let mut items = CharacterItems::new();
        items.set(inv(0), &potions(1, 150)).expect("placed");
        items.set(inv(1), &potions(2, 190)).expect("placed");
        let (given, done) =
            auto_give_item_new(&mut items, &RULES, &protos(), potions(9, 70), false);
        let GivenNew::Unplaced(left) = given else {
            panic!("no cell is free: {given:?}");
        };
        assert_eq!(left.count, 10);
        assert_eq!(
            done.changes,
            [
                ItemChange::Count { id: 1, count: 200 },
                ItemChange::Count { id: 2, count: 200 },
            ]
        );
    }

    /// A stone goes to the first empty cell of its box, sub-type 1's column and grade 2's
    /// row, only for a qualified character; an unqualified one has no box.
    #[test]
    fn a_stone_goes_to_its_box_only_when_qualified() {
        let mut items = CharacterItems::new();
        let base = 6 * 32 + 2 * 32;
        let first = ItemPos::new(EWindows::DragonSoulInventory as u8, base);
        items.set(first, &Item::new(1, STONE)).expect("placed");
        let stone = Item::new(9, STONE);
        let (given, _) = auto_give_item_new(&mut items, &RULES, &protos(), stone.clone(), false);
        assert_eq!(given, GivenNew::Unplaced(stone.clone()));
        let (given, done) = auto_give_item_new(&mut items, &RULES, &protos(), stone, true);
        let second = ItemPos::new(EWindows::DragonSoulInventory as u8, base + 1);
        assert_eq!(given, GivenNew::Placed(second));
        assert_eq!(items.item_at(second).map(|item| item.id), Some(9));
        assert_eq!(done.records.len(), 1);
    }
}
