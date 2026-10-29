//! Counting and taking away the items of one vnum: `CHARACTER::CountSpecifyItem` and
//! `CHARACTER::RemoveSpecifyItem` (`G/char_item.cpp:8791-8940`), which `pc.count_item` and
//! `pc.remove_item` call; and `CItem::SetSocket` of one held item, which `item.set_socket` calls.
//!
//! Both read the base inventory up to `Inventory_Size()`, the unlocked cells
//! (`ENABLE_EXTEND_INVEN_SYSTEM`), and then the custom banks (`ENABLE_CUSTOM_INVENTORY`), in
//! cell order. Legacy skips an item its owner sells in a personal shop or a premium private
//! shop; the Rewrite has neither, so nothing is skipped. The gold-bar log of a removal is not
//! written.

use common::item_slots::{EWindows, CUSTOM_INVENTORY_SLOT_END, CUSTOM_INVENTORY_SLOT_START};
use gamedata::item_proto::ItemProtos;
use protocol::item_pos::ItemPos;

use super::item_move::{ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord};
use super::items::CharacterItems;
use super::quickslot::{chains_when_used_up, QuickslotSync, Quickslots, SyncTo};
use crate::item::{gc_item_clear, ItemId};

/// The cells both scans read, in their order: the unlocked base cells, then the custom banks.
fn scanned_cells(usable_cells: u16) -> impl Iterator<Item = ItemPos> {
    (0..usable_cells)
        .chain(CUSTOM_INVENTORY_SLOT_START..CUSTOM_INVENTORY_SLOT_END)
        .map(|cell| ItemPos::new(EWindows::Inventory as u8, cell))
}

/// `CountSpecifyItem(vnum)` (`G/char_item.cpp:8791-8846`): the summed count of every item of
/// `vnum` in the scanned cells.
#[must_use]
pub fn count_specify_item(items: &CharacterItems, usable_cells: u16, vnum: u32) -> i32 {
    scanned_cells(usable_cells)
        .filter_map(|pos| items.item_at(pos))
        .filter(|item| item.vnum == vnum)
        .map(|item| i32::from(item.count))
        .sum()
}

/// `RemoveSpecifyItem(vnum, count)` (`G/char_item.cpp:8848-8940`): `count` of `vnum` taken
/// from the scanned cells in order. Each stack the rest covers is used up, and the first it
/// does not cover loses the rest. A count of 0 takes nothing, and a count larger than the
/// character holds takes everything it holds.
///
/// A used-up stack is `SetCount(0)` (`G/item.cpp:303-330`): one that chains
/// ([`chains_when_used_up`]) is cleared first and its slots then follow the first other item
/// of its vnum, which is found at once, before the next stack is taken; any other has its
/// slots deleted before it is cleared. A stack that keeps some is updated (`UpdatePacket`).
pub fn remove_specify_item(
    items: &mut CharacterItems,
    slots: &mut Quickslots,
    protos: &ItemProtos,
    usable_cells: u16,
    (vnum, count): (u32, u32),
) -> MoveDone {
    let mut done = MoveDone {
        kind: MoveKind::Removed,
        records: Vec::new(),
        changes: Vec::new(),
    };
    let chains = protos.get(vnum).is_some_and(chains_when_used_up);
    let mut left = count;
    for pos in scanned_cells(usable_cells) {
        if left == 0 {
            break;
        }
        let Some((id, held)) = items
            .item_at(pos)
            .filter(|item| item.vnum == vnum)
            .map(|item| (item.id, item.count))
        else {
            continue;
        };
        if left >= u32::from(held) {
            left -= u32::from(held);
            use_up(items, slots, &mut done, (id, pos), chains);
        } else {
            let keeps = held - u16::try_from(left).unwrap_or(held);
            left = 0;
            if items.set_count(id, u32::from(keeps)).is_err() {
                continue;
            }
            if let Some(item) = items.item(id) {
                let update = ItemRecord::Update(item.gc_item_update());
                done.records.push(MoveRecord::Item(update));
            }
            done.changes.push(ItemChange::Count { id, count: keeps });
        }
    }
    done
}

/// `CItem::SetSocket(index, value)` (`G/item.cpp:1643-1656`) of the held item `id`: the socket
/// takes the value, the client's cell is updated (`UpdatePacket`), and the row is saved. An
/// item the character does not hold, or an index past the sockets, changes and sends nothing.
#[must_use]
pub fn set_item_socket(
    items: &mut CharacterItems,
    id: ItemId,
    (index, value): (usize, i32),
) -> MoveDone {
    let mut done = MoveDone {
        kind: MoveKind::Socketed,
        records: Vec::new(),
        changes: Vec::new(),
    };
    let Ok(sockets) = items.set_socket(id, index, value) else {
        return done;
    };
    if let Some(item) = items.item(id) {
        let update = ItemRecord::Update(item.gc_item_update());
        done.records.push(MoveRecord::Item(update));
    }
    done.changes.push(ItemChange::Sockets { id, sockets });
    done
}

/// `SetCount(0)` of the item `id` at `pos`: the item leaves the storage and the client's cell,
/// and the slots that named its cell are synced in legacy's order.
fn use_up(
    items: &mut CharacterItems,
    slots: &mut Quickslots,
    done: &mut MoveDone,
    (id, pos): (u32, ItemPos),
    chains: bool,
) {
    let vnum = items.item(id).map_or(0, |item| item.vnum);
    let to = if chains {
        SyncTo::Chain(vnum)
    } else {
        SyncTo::Delete
    };
    let sync = QuickslotSync { from: pos.cell, to };
    if !chains {
        let deleted = slots.sync(sync, items);
        done.records
            .extend(deleted.into_iter().map(MoveRecord::Quickslot));
    }
    if items.release(id).is_err() {
        return;
    }
    done.records
        .push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
    done.changes.push(ItemChange::Destroyed { id });
    if chains {
        let chained = slots.sync(sync, items);
        done.records
            .extend(chained.into_iter().map(MoveRecord::Quickslot));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::{Quickslot, QuickslotRecord};
    use crate::item::{Item, ITEM_FLAG_STACKABLE};
    use common::enums::EQuickSlotType;
    use gamedata::item_kind::{ITEM_USE, USE_POTION};
    use gamedata::item_proto::ItemProto;

    const INV: u8 = EWindows::Inventory as u8;
    /// A stackable `ITEM_USE` `USE_POTION`, whose used-up stacks chain their slots.
    const POTION: u32 = 27_001;
    /// A stackable item that chains nothing.
    const ORE: u32 = 50_601;

    fn protos() -> ItemProtos {
        let mut potion = ItemProto::for_category_rule(POTION, ITEM_USE, USE_POTION);
        potion.flags = ITEM_FLAG_STACKABLE;
        let mut ore = ItemProto::for_category_rule(ORE, 1, 1);
        ore.flags = ITEM_FLAG_STACKABLE;
        ItemProtos::from_rows(vec![potion, ore])
    }

    fn stack(id: u32, vnum: u32, count: u16) -> Item {
        let mut item = Item::new(id, vnum);
        item.count = count;
        item.flags = ITEM_FLAG_STACKABLE;
        item
    }

    fn holding(placed: &[(u16, Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (cell, item) in placed {
            let pos = ItemPos::new(INV, *cell);
            items.set(pos, item).expect("the fixture places");
        }
        items
    }

    fn item_slot(pos: u8) -> Quickslot {
        Quickslot {
            kind: EQuickSlotType::Item as u8,
            pos,
        }
    }

    fn count_of(items: &CharacterItems, cell: u16) -> Option<u16> {
        items
            .item_at(ItemPos::new(INV, cell))
            .map(|item| item.count)
    }

    /// The base cells count up to the unlocked ones, and the custom banks after them; other
    /// vnums are not counted.
    #[test]
    fn the_count_sums_the_unlocked_cells_and_the_banks() {
        let items = holding(&[
            (0, stack(1, ORE, 3)),
            (4, stack(2, ORE, 0x0102)),
            (5, stack(3, POTION, 7)),
            (CUSTOM_INVENTORY_SLOT_START + 9, stack(4, ORE, 11)),
            (90, stack(5, ORE, 1000)),
        ]);
        assert_eq!(count_specify_item(&items, 45, ORE), 3 + 0x0102 + 11);
        assert_eq!(count_specify_item(&items, 91, ORE), 3 + 0x0102 + 11 + 1000);
        assert_eq!(count_specify_item(&items, 45, POTION), 7);
        assert_eq!(count_specify_item(&items, 45, 12_345), 0);
    }

    /// The stacks are taken in cell order: the first is used up, the next keeps the rest.
    #[test]
    fn a_removal_uses_up_stacks_in_cell_order_and_trims_the_last() {
        let mut items = holding(&[
            (2, stack(1, ORE, 3)),
            (7, stack(2, ORE, 10)),
            (CUSTOM_INVENTORY_SLOT_START, stack(3, ORE, 5)),
        ]);
        let mut slots = Quickslots::default();
        let done = remove_specify_item(&mut items, &mut slots, &protos(), 45, (ORE, 7));
        assert_eq!(done.kind, MoveKind::Removed);
        assert_eq!(count_of(&items, 2), None);
        assert_eq!(count_of(&items, 7), Some(6));
        assert_eq!(count_of(&items, CUSTOM_INVENTORY_SLOT_START), Some(5));
        assert_eq!(
            done.changes,
            [
                ItemChange::Destroyed { id: 1 },
                ItemChange::Count { id: 2, count: 6 }
            ]
        );
        let updated = items.item(2).expect("kept").gc_item_update();
        assert_eq!(
            done.records,
            [
                MoveRecord::Item(ItemRecord::Set(gc_item_clear(ItemPos::new(INV, 2)))),
                MoveRecord::Item(ItemRecord::Update(updated)),
            ]
        );
    }

    /// A count that covers every stack exactly ends at the last; a larger count takes all it
    /// finds, the banks included; a count of 0 takes nothing.
    #[test]
    fn a_removal_past_what_is_held_takes_everything_and_zero_takes_nothing() {
        let mut items = holding(&[
            (0, stack(1, ORE, 2)),
            (CUSTOM_INVENTORY_SLOT_START + 1, stack(2, ORE, 3)),
            (1, stack(3, POTION, 4)),
        ]);
        let mut slots = Quickslots::default();
        let none = remove_specify_item(&mut items, &mut slots, &protos(), 45, (ORE, 0));
        assert!(none.records.is_empty() && none.changes.is_empty());
        let all = remove_specify_item(&mut items, &mut slots, &protos(), 45, (ORE, 70_000));
        assert_eq!(
            all.changes,
            [
                ItemChange::Destroyed { id: 1 },
                ItemChange::Destroyed { id: 2 }
            ]
        );
        assert_eq!(count_specify_item(&items, 45, ORE), 0);
        assert_eq!(count_of(&items, 1), Some(4), "another vnum stays");
    }

    /// An item past the unlocked cells is neither counted nor taken.
    #[test]
    fn a_removal_skips_the_locked_cells() {
        let mut items = holding(&[(50, stack(1, ORE, 2)), (3, stack(2, ORE, 1))]);
        let mut slots = Quickslots::default();
        let done = remove_specify_item(&mut items, &mut slots, &protos(), 45, (ORE, 3));
        assert_eq!(done.changes, [ItemChange::Destroyed { id: 2 }]);
        assert_eq!(count_of(&items, 50), Some(2));
    }

    /// A used-up item that chains nothing has its slot deleted before its cell is cleared.
    #[test]
    fn a_used_up_stack_deletes_its_slot_before_the_clear() {
        let mut items = holding(&[(4, stack(1, ORE, 2))]);
        let mut slots = Quickslots::default();
        let _ = slots.set(3, item_slot(4), &mut Vec::new());
        let done = remove_specify_item(&mut items, &mut slots, &protos(), 45, (ORE, 2));
        assert_eq!(
            done.records,
            [
                MoveRecord::Quickslot(QuickslotRecord::Del { slot: 3 }),
                MoveRecord::Item(ItemRecord::Set(gc_item_clear(ItemPos::new(INV, 4)))),
            ]
        );
        assert_eq!(slots.get(3), Some(Quickslot::default()));
    }

    /// A used-up potion is cleared first and its slot then follows the next potion, found at
    /// once; when that one is used up in turn, nothing is left to follow and the slot goes.
    #[test]
    fn a_used_up_potion_chains_its_slot_to_the_next_then_deletes_it() {
        let mut items = holding(&[(4, stack(1, POTION, 2)), (9, stack(2, POTION, 1))]);
        let mut slots = Quickslots::default();
        let _ = slots.set(0, item_slot(4), &mut Vec::new());
        let done = remove_specify_item(&mut items, &mut slots, &protos(), 45, (POTION, 3));
        assert_eq!(
            done.records,
            [
                MoveRecord::Item(ItemRecord::Set(gc_item_clear(ItemPos::new(INV, 4)))),
                MoveRecord::Quickslot(QuickslotRecord::Add {
                    slot: 0,
                    quickslot: item_slot(9),
                }),
                MoveRecord::Item(ItemRecord::Set(gc_item_clear(ItemPos::new(INV, 9)))),
                MoveRecord::Quickslot(QuickslotRecord::Del { slot: 0 }),
            ]
        );
    }

    /// A socket write lands on the held item and updates its cell; another id or an index past
    /// the sockets changes and sends nothing.
    #[test]
    fn a_socket_write_updates_the_held_item_and_nothing_else() {
        let mut items = holding(&[(4, stack(9, ORE, 2))]);
        let done = set_item_socket(&mut items, 9, (1, 0x0102_0304));
        assert_eq!(done.kind, MoveKind::Socketed);
        let item = items.item(9).expect("held");
        assert_eq!(item.sockets[..3], [0, 0x0102_0304, 0]);
        assert_eq!(
            done.changes,
            [ItemChange::Sockets {
                id: 9,
                sockets: item.sockets
            }]
        );
        assert_eq!(
            done.records,
            [MoveRecord::Item(ItemRecord::Update(item.gc_item_update()))]
        );
        let last = crate::item::SOCKETS - 1;
        let done = set_item_socket(&mut items, 9, (last, -5));
        assert_eq!(items.item(9).expect("held").sockets[last], -5);
        assert_eq!(done.changes.len(), 1);
        for (id, index) in [(8, 0), (9, crate::item::SOCKETS)] {
            let done = set_item_socket(&mut items, id, (index, 7));
            assert!(done.records.is_empty() && done.changes.is_empty());
        }
        assert_eq!(items.item(9).expect("held").sockets[0], 0);
    }
}
