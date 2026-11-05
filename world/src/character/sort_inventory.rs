//! `CHARACTER::SortInventoryItems` (`G/char.cpp:10790-10866`, `__SORT_INVENTORY_ITEMS__`),
//! the storage half of `/sort_inventory`.
//!
//! Every item in the base inventory (cells `0..INVENTORY_MAX_NUM`) is lifted in cell order:
//! its cell is cleared (`RemoveFromCharacter`) and the quickslots naming the cell are deleted
//! (`SyncQuickslot(QUICKSLOT_TYPE_ITEM, i, 255)`). The banks and the equipment stay. The
//! lifted items are sorted by their locale name (`SortItems` compares `std::string`s, which
//! order bytes as `unsigned char`), and each is given back in that order:
//!
//! - A `STACKABLE` item that is not `ITEM_BLEND`, not one of the twelve automatic potions and
//!   not a summon item goes back by vnum, `AutoGiveItem(vnum, count, -1, false)`: it tops up
//!   the stacks of its vnum whose sockets are the proto's, in cell order, and what is left
//!   goes to the first free cell of its banks or of the unlocked base inventory, highlighted
//!   as a fresh item, with a potion filling the first empty quickslot.
//! - Anything else goes back whole, `AutoGiveItem(item)`: the first free cell of its banks or
//!   of the unlocked base inventory, not highlighted (the character held it last), with the
//!   same potion quickslot.
//!
//! An item no cell takes lies at the character's feet: owned for 60 seconds by the whole
//! give, and for 300 or 60 by the vnum give as its `ANTIFLAG_DROP` says. With no `bMsg`, no
//! pickup line rides.
//!
//! Three legacy Defects are not reproduced:
//!
//! - The vnum give runs the `MAKECOUNT` arm at every stack it tops up, which raises the count
//!   to the proto's `value1`: sorting a stack of a `MAKECOUNT` item could make items. Here the
//!   count given is the count lifted.
//! - The vnum give destroys the lifted item and makes a fresh one with the proto's sockets, so
//!   a stackable item whose sockets or attributes differ from a fresh one's would lose them.
//!   Here such an item goes back whole; one equal to a fresh item goes by vnum as legacy's,
//!   keeping its id for what is left after the top-ups.
//! - An item whose vnum has no proto is lifted and never given back
//!   (`if (!itemTable) continue;`). Here it goes back whole.

use common::item_slots::{
    EWindows, CUSTOM_INVENTORY_SLOT_END, CUSTOM_INVENTORY_SLOT_START, INVENTORY_MAX_NUM,
};
use gamedata::item_custom_category::{is_custom_category, CATEGORY_NUM};
use gamedata::item_kind::{ITEM_BLEND, ITEM_USE, USE_POTION};
use gamedata::item_proto::{ItemProto, ItemProtos};
use protocol::gc_item_window::ItemAttribute;
use protocol::item_pos::ItemPos;

use super::item_move::{ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, MoveRules};
use super::items::CharacterItems;
use super::quickslot::{Quickslot, QuickslotSync, Quickslots, SyncTo, QUICKSLOT_MAX_NUM};
use super::NPOS;
use crate::item::{gc_item_clear, Item, ATTRIBUTES, ITEM_ANTIFLAG_DROP, ITEM_FLAG_STACKABLE};

/// The automatic potions the sort gives back whole although they stack (`excepList`):
/// `ITEM_AUTO_HP_RECOVERY_S` to `_X`, `ITEM_AUTO_SP_RECOVERY_S` to `_X` (`unique_item.h:87-95`)
/// and the four reward-box ones (`:100-104`).
pub const SORT_KEPT_WHOLE: [u32; 12] = [
    72723, 72724, 72725, 72726, 72727, 72728, 72729, 72730, 76004, 76005, 76021, 76022,
];

/// `IS_SUMMON_ITEM` (`G/char_item.cpp:82-95`): the horse whistles and the marriage ring.
pub const SUMMON_ITEMS: [u32; 5] = [22000, 22010, 22011, 22020, 70302];

/// The ownership of an item the whole give lays down: `AutoGiveItem(item, false)`.
const WHOLE_OWNERSHIP_SECS: u16 = 60;

/// What [`sort_inventory`] did: the records and rows, and the items no cell took, each with
/// the seconds its ownership lasts on the ground.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sorted {
    /// The records in order, and the row changes.
    pub done: MoveDone,
    /// The items for the ground, with their ownership seconds. Their rows are destroyed in
    /// `done`; a ground item has none until it is picked up.
    pub unplaced: Vec<(Item, u16)>,
}

/// `SortInventoryItems` past its gates: lift, sort, and give back. See the module doc.
pub fn sort_inventory(
    items: &mut CharacterItems,
    slots: &mut Quickslots,
    rules: &MoveRules,
    protos: &ItemProtos,
) -> Sorted {
    let mut sorted = Sorted {
        done: MoveDone {
            kind: MoveKind::Moved,
            records: Vec::new(),
            changes: Vec::new(),
        },
        unplaced: Vec::new(),
    };
    let mut lifted = Vec::new();
    for cell in 0..INVENTORY_MAX_NUM {
        let pos = ItemPos::new(EWindows::Inventory as u8, cell);
        let Some(held) = items.item_at(pos) else {
            continue;
        };
        let item = held.clone();
        if items.release(item.id).is_err() {
            continue;
        }
        let done = &mut sorted.done;
        done.records
            .push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
        let sync = QuickslotSync {
            from: cell,
            to: SyncTo::Delete,
        };
        done.records.extend(
            slots
                .sync(sync, items)
                .into_iter()
                .map(MoveRecord::Quickslot),
        );
        lifted.push(item);
    }
    let name = |item: &Item| protos.get(item.vnum).map(|proto| proto.locale_name.clone());
    lifted.sort_by_cached_key(|item| name(item).unwrap_or_default());
    for item in lifted {
        let Some(proto) = protos.get(item.vnum) else {
            give_whole(items, slots, rules, None, item, &mut sorted);
            continue;
        };
        if goes_by_vnum(proto, &item) {
            give_by_vnum(items, slots, rules, proto, item, &mut sorted);
        } else {
            give_whole(items, slots, rules, Some(proto), item, &mut sorted);
        }
    }
    sorted
}

/// Whether legacy gives `item` back by vnum, and the Rewrite can without losing anything: the
/// stackable arm's test, with the item equal to a fresh one of its vnum.
fn goes_by_vnum(proto: &ItemProto, item: &Item) -> bool {
    proto.flags & ITEM_FLAG_STACKABLE != 0
        && proto.item_type != ITEM_BLEND
        && !SORT_KEPT_WHOLE.contains(&item.vnum)
        && !SUMMON_ITEMS.contains(&item.vnum)
        && item.sockets == proto.sockets
        && item.attributes == [ItemAttribute::new(0, 0); ATTRIBUTES]
}

/// `AutoGiveItem(vnum, count, -1, false)` for a lifted stack: the top-ups, then what is left in
/// a free cell, or on the ground.
fn give_by_vnum(
    items: &mut CharacterItems,
    slots: &mut Quickslots,
    rules: &MoveRules,
    proto: &ItemProto,
    mut item: Item,
    sorted: &mut Sorted,
) {
    let done = &mut sorted.done;
    let mut left = item.count;
    let cells =
        (0..INVENTORY_MAX_NUM).chain(CUSTOM_INVENTORY_SLOT_START..CUSTOM_INVENTORY_SLOT_END);
    for cell in cells {
        let pos = ItemPos::new(EWindows::Inventory as u8, cell);
        let Some(stack) = items.item_at(pos) else {
            continue;
        };
        if stack.vnum != item.vnum || stack.sockets != proto.sockets {
            continue;
        }
        let (id, held) = (stack.id, stack.count);
        let taken = rules.count_limit.saturating_sub(held).min(left);
        let Ok(count) = items.set_count(id, u32::from(held + taken)) else {
            continue;
        };
        left -= taken;
        if let Some(stack) = items.item(id) {
            done.records
                .push(MoveRecord::Item(ItemRecord::Update(stack.gc_item_update())));
        }
        done.changes.push(ItemChange::Count { id, count });
        if left == 0 {
            done.changes.push(ItemChange::Destroyed { id: item.id });
            return;
        }
    }
    if left != item.count {
        item.count = left;
        done.changes.push(ItemChange::Count {
            id: item.id,
            count: left,
        });
    }
    let secs = if item.anti_flags & ITEM_ANTIFLAG_DROP != 0 {
        300
    } else {
        60
    };
    // The made item was never held, so `AddToCharacter` highlights its cell.
    place(items, slots, rules, Some(proto), item, (1, secs), sorted);
}

/// `AutoGiveItem(item)` for a lifted item: a free cell, or the ground for 60 seconds.
fn give_whole(
    items: &mut CharacterItems,
    slots: &mut Quickslots,
    rules: &MoveRules,
    proto: Option<&ItemProto>,
    item: Item,
    sorted: &mut Sorted,
) {
    // The character held the item last, so `AddToCharacter` does not highlight it.
    place(
        items,
        slots,
        rules,
        proto,
        item,
        (0, WHOLE_OWNERSHIP_SECS),
        sorted,
    );
}

/// `GetEmptyInventoryEx` and `AddToCharacter`, or the ground: the item's banks first, then the
/// unlocked base inventory. A placed potion fills the first empty quickslot.
fn place(
    items: &mut CharacterItems,
    slots: &mut Quickslots,
    rules: &MoveRules,
    proto: Option<&ItemProto>,
    mut item: Item,
    (highlight, secs): (u8, u16),
    sorted: &mut Sorted,
) {
    let size = item.size;
    let cell = proto
        .and_then(|proto| {
            (0..CATEGORY_NUM)
                .filter(|bank| is_custom_category(proto, *bank))
                .find_map(|bank| items.find_free_custom_cell(bank, size))
        })
        .or_else(|| items.find_free_inventory_cell(rules.usable_cells, size));
    let pos = cell.map(|cell| ItemPos::new(EWindows::Inventory as u8, cell));
    item.pos = pos.unwrap_or(NPOS);
    let Some((cell, pos)) = cell.zip(pos).filter(|_| items.set(item.pos, &item).is_ok()) else {
        sorted
            .done
            .changes
            .push(ItemChange::Destroyed { id: item.id });
        item.pos = NPOS;
        sorted.unplaced.push((item, secs));
        return;
    };
    let done = &mut sorted.done;
    done.records.push(MoveRecord::Item(ItemRecord::Set(
        item.gc_item_set(pos, highlight),
    )));
    done.changes.push(ItemChange::Moved { id: item.id, pos });
    let potion =
        proto.is_some_and(|proto| proto.item_type == ITEM_USE && proto.sub_type == USE_POTION);
    // Legacy names the cell as a `BYTE`; a bank cell past 255 fills no slot.
    let (true, Ok(byte)) = (potion, u8::try_from(cell)) else {
        return;
    };
    let empty = (0_u8..)
        .take(QUICKSLOT_MAX_NUM)
        .find(|slot| slots.get(*slot).is_some_and(Quickslot::is_empty));
    if let Some(slot) = empty {
        let mut out = Vec::new();
        let quickslot = Quickslot {
            kind: common::enums::EQuickSlotType::Item as u8,
            pos: byte,
        };
        let _set = slots.set(slot, quickslot, &mut out);
        done.records
            .extend(out.into_iter().map(MoveRecord::Quickslot));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::drop_item::ITEM_FLAG_MAKECOUNT;
    use common::enums::EQuickSlotType;

    const SWORD: u32 = 19;
    const ARROW: u32 = 8_000;
    const POTION: u32 = 27_001;

    const RULES: MoveRules = MoveRules {
        count_limit: 200,
        usable_cells: 90,
        belt_grade: None,
        questing: false,
    };

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    fn proto(vnum: u32, kind: (i32, i32), name: &[u8], flags: u32) -> ItemProto {
        let mut proto = ItemProto::for_category_rule(vnum, kind.0, kind.1);
        proto.locale_name = name.to_vec();
        proto.flags = flags;
        proto
    }

    fn protos(arrow_flags: u32) -> ItemProtos {
        let mut arrow = proto(ARROW, (1, 0), b"Arrow", arrow_flags);
        arrow.values[1] = 100;
        ItemProtos::from_rows(vec![
            proto(SWORD, (1, 0), b"Sword", 0),
            arrow,
            proto(POTION, (ITEM_USE, USE_POTION), b"Red Potion", 0),
        ])
    }

    fn item(id: u32, vnum: u32, count: u16) -> Item {
        let mut item = Item::new(id, vnum);
        item.count = count;
        item
    }

    fn holding(placed: &[(u16, Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (cell, item) in placed {
            items.set(inv(*cell), item).expect("the fixture places");
        }
        items
    }

    fn at(items: &CharacterItems, cell: u16) -> Option<(u32, u32, u16)> {
        items
            .item_at(inv(cell))
            .map(|item| (item.id, item.vnum, item.count))
    }

    /// The lifted items go back by name: the two arrow stacks become one at cell 0, the
    /// potion takes cell 1 and the first quickslot its sync freed, and the sword cell 2.
    #[test]
    fn the_items_go_back_by_name_and_the_stacks_merge() {
        let mut items = holding(&[
            (0, item(1, SWORD, 1)),
            (1, item(2, ARROW, 50)),
            (3, item(3, ARROW, 30)),
            (5, item(4, POTION, 1)),
        ]);
        let mut slots = Quickslots::default();
        let named = Quickslot {
            kind: EQuickSlotType::Item as u8,
            pos: 5,
        };
        assert!(slots.set(0, named, &mut Vec::new()));
        let sorted = sort_inventory(&mut items, &mut slots, &RULES, &protos(ITEM_FLAG_STACKABLE));
        assert_eq!(at(&items, 0), Some((2, ARROW, 80)));
        assert_eq!(at(&items, 1), Some((4, POTION, 1)));
        assert_eq!(at(&items, 2), Some((1, SWORD, 1)));
        assert_eq!(at(&items, 3), None);
        assert_eq!(at(&items, 5), None);
        assert_eq!(
            slots.get(0),
            Some(Quickslot {
                kind: EQuickSlotType::Item as u8,
                pos: 1
            }),
            "the potion fills the slot its lift emptied"
        );
        assert!(sorted.unplaced.is_empty());
        assert_eq!(
            sorted.done.changes,
            [
                ItemChange::Moved { id: 2, pos: inv(0) },
                ItemChange::Count { id: 2, count: 80 },
                ItemChange::Destroyed { id: 3 },
                ItemChange::Moved { id: 4, pos: inv(1) },
                ItemChange::Moved { id: 1, pos: inv(2) },
            ]
        );
        let sets: Vec<(u16, u8)> = sorted
            .done
            .records
            .iter()
            .filter_map(|record| match record {
                MoveRecord::Item(ItemRecord::Set(set)) if set.vnum != 0 => {
                    Some((set.cell.cell, set.highlight))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            sets,
            [(0, 1), (1, 0), (2, 0)],
            "only the vnum give highlights"
        );
    }

    /// Legacy's top-up raises a `MAKECOUNT` stack to the proto's `value1` (a Defect); the
    /// sort gives back the count it lifted.
    #[test]
    fn a_makecount_stack_keeps_its_count() {
        let mut items = holding(&[(0, item(1, ARROW, 5)), (1, item(2, ARROW, 5))]);
        let flags = ITEM_FLAG_STACKABLE | ITEM_FLAG_MAKECOUNT;
        let _sorted = sort_inventory(
            &mut items,
            &mut Quickslots::default(),
            &RULES,
            &protos(flags),
        );
        assert_eq!(at(&items, 0), Some((1, ARROW, 10)));
        assert_eq!(at(&items, 1), None);
    }

    /// A stack whose sockets differ from a fresh one's goes back whole, keeping them, and
    /// takes no top-up (legacy would remake it with the proto's sockets).
    #[test]
    fn a_socketed_stack_goes_back_whole() {
        let mut marked = item(2, ARROW, 7);
        marked.sockets[0] = 9;
        let mut items = holding(&[(0, item(1, ARROW, 5)), (4, marked)]);
        let sorted = sort_inventory(
            &mut items,
            &mut Quickslots::default(),
            &RULES,
            &protos(ITEM_FLAG_STACKABLE),
        );
        assert_eq!(at(&items, 0), Some((1, ARROW, 5)));
        assert_eq!(at(&items, 1), Some((2, ARROW, 7)));
        assert_eq!(items.item(2).map(|item| item.sockets[0]), Some(9));
        assert!(sorted
            .done
            .changes
            .iter()
            .all(|change| !matches!(change, ItemChange::Destroyed { .. })));
    }

    /// With one usable cell the second item lies on the ground for 60 seconds, its row gone.
    #[test]
    fn what_no_cell_takes_lies_on_the_ground() {
        let mut items = holding(&[(0, item(1, SWORD, 1)), (1, item(2, POTION, 1))]);
        let rules = MoveRules {
            usable_cells: 1,
            ..RULES
        };
        let sorted = sort_inventory(
            &mut items,
            &mut Quickslots::default(),
            &rules,
            &protos(ITEM_FLAG_STACKABLE),
        );
        assert_eq!(
            at(&items, 0),
            Some((2, POTION, 1)),
            "the potion sorts first"
        );
        assert_eq!(sorted.unplaced.len(), 1);
        assert_eq!(sorted.unplaced[0].0.id, 1);
        assert_eq!(sorted.unplaced[0].1, 60);
        assert_eq!(
            sorted.done.changes.last(),
            Some(&ItemChange::Destroyed { id: 1 })
        );
    }
}
