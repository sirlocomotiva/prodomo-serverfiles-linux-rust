//! Quickslots: `CHARACTER::SetQuickslot`, `DelQuickslot` and `SwapQuickslot`
//! (`G/char_quickslot.cpp:45-136`), the item check `CInputMain::QuickslotAdd` makes first
//! (`G/input_main.cpp:1083-1113`), and `SyncQuickslot` and `ChainQuickslotItem`
//! (`G/char_quickslot.cpp:12-34`, `:138-155`), which make an item slot follow its item.
//!
//! A character has 36 slots. Each is empty, or names an inventory cell, a skill or a command.
//! Every change answers the records legacy sends for it, in its order.
//!
//! An item step cannot reach the slots, so it leaves a [`QuickslotSync`] among its records
//! where legacy syncs, and [`sync_quickslots`] runs each one on the character's slots in its
//! place.

use common::enums::EQuickSlotType;
use common::item_slots::{
    EWindows, CUSTOM_INVENTORY_SLOT_END, CUSTOM_INVENTORY_SLOT_START, INVENTORY_MAX_NUM,
};
use gamedata::item_kind::{ITEM_USE, USE_ABILITY_UP, USE_POTION};
use gamedata::item_proto::{ItemProto, ItemProtos};
use protocol::item_pos::ItemPos;

use super::inventory::{is_belt_inventory_position, is_default_inventory_position};
use super::item_move::{MoveDone, MoveRecord};
use super::items::CharacterItems;

/// `QUICKSLOT_MAX_NUM` (`common/length.h:51`).
pub const QUICKSLOT_MAX_NUM: usize = 36;

/// `SKILL_MAX_NUM` (`common/length.h`): a skill slot names a skill below it.
const SKILL_MAX_NUM: u8 = 255;

/// One slot: `TQuickslot { BYTE type; BYTE pos; }` (`common/tables.h:452-456`). The default is
/// the empty slot, the zeroed record `DelQuickslot` writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Quickslot {
    /// `type`: an [`EQuickSlotType`] byte.
    pub kind: u8,
    /// `pos`: the inventory cell, the skill or the command.
    pub pos: u8,
}

impl Quickslot {
    /// Whether the slot names nothing.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.kind == EQuickSlotType::None as u8
    }
}

/// A record a quickslot change sends to the character's own client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickslotRecord {
    /// `GC_QUICKSLOT_ADD` (28): the slot now holds this.
    Add {
        /// The slot's index.
        slot: u8,
        /// What it holds.
        quickslot: Quickslot,
    },
    /// `GC_QUICKSLOT_DEL` (29): the slot is empty.
    Del {
        /// The slot's index.
        slot: u8,
    },
    /// `GC_QUICKSLOT_SWAP` (30): two slots traded places.
    Swap {
        /// The first slot.
        slot: u8,
        /// The second slot.
        with: u8,
    },
}

/// A character's slots: `CHARACTER::m_quickslot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quickslots([Quickslot; QUICKSLOT_MAX_NUM]);

impl Default for Quickslots {
    fn default() -> Self {
        Self([Quickslot::default(); QUICKSLOT_MAX_NUM])
    }
}

impl Quickslots {
    /// The slot at `slot`, or `None` past the last one (`GetQuickslot`).
    #[must_use]
    pub fn get(&self, slot: u8) -> Option<Quickslot> {
        self.0.get(usize::from(slot)).copied()
    }

    /// Every slot that names something, with its index, in slot order.
    pub fn set_slots(&self) -> impl Iterator<Item = (u8, Quickslot)> + '_ {
        self.0
            .iter()
            .zip(0_u8..)
            .filter(|(quickslot, _)| !quickslot.is_empty())
            .map(|(quickslot, slot)| (slot, *quickslot))
    }

    /// `SetQuickslot`: put `quickslot` in `slot`, answering whether it did.
    ///
    /// Any slot that already holds the same thing, `slot` itself included, is emptied first
    /// with its `GC_QUICKSLOT_DEL`. An item slot must name a base inventory or belt cell, and a
    /// skill slot a skill below `SKILL_MAX_NUM`; the empty kind is refused. The belt starts past
    /// 255 in this build, so a `BYTE` cell never reaches it, and the base inventory is 0 to 179.
    ///
    /// Legacy empties the twins before it checks what `quickslot` names. No slot can hold what
    /// the check refuses, so a refused slot never has a twin, and the order shows nowhere.
    pub fn set(&mut self, slot: u8, quickslot: Quickslot, out: &mut Vec<QuickslotRecord>) -> bool {
        if usize::from(slot) >= QUICKSLOT_MAX_NUM || quickslot.kind >= EQuickSlotType::MaxNum as u8
        {
            return false;
        }
        if !quickslot.is_empty() {
            let twins: Vec<u8> = self
                .set_slots()
                .filter(|(_, held)| *held == quickslot)
                .map(|(twin, _)| twin)
                .collect();
            for twin in twins {
                let _deleted = self.delete(twin, out);
            }
        }
        let cell = ItemPos {
            window_type: EWindows::Inventory as u8,
            cell: u16::from(quickslot.pos),
        };
        let legal = match quickslot.kind {
            kind if kind == EQuickSlotType::Item as u8 => {
                is_default_inventory_position(cell) || is_belt_inventory_position(cell)
            }
            kind if kind == EQuickSlotType::Skill as u8 => quickslot.pos < SKILL_MAX_NUM,
            kind => kind == EQuickSlotType::Command as u8,
        };
        if !legal {
            return false;
        }
        self.0[usize::from(slot)] = quickslot;
        out.push(QuickslotRecord::Add { slot, quickslot });
        true
    }

    /// `DelQuickslot`: empty `slot`, answering whether it was one. An empty slot is emptied
    /// again and told again.
    pub fn delete(&mut self, slot: u8, out: &mut Vec<QuickslotRecord>) -> bool {
        let Some(held) = self.0.get_mut(usize::from(slot)) else {
            return false;
        };
        *held = Quickslot::default();
        out.push(QuickslotRecord::Del { slot });
        true
    }

    /// `SwapQuickslot`: trade `slot` and `with`, answering whether both are slots. A slot swapped
    /// with itself is told too.
    pub fn swap(&mut self, slot: u8, with: u8, out: &mut Vec<QuickslotRecord>) -> bool {
        let (a, b) = (usize::from(slot), usize::from(with));
        if a >= QUICKSLOT_MAX_NUM || b >= QUICKSLOT_MAX_NUM {
            return false;
        }
        self.0.swap(a, b);
        out.push(QuickslotRecord::Swap { slot, with });
        true
    }
}

/// What an item slot that names a cell an item left does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTo {
    /// `SyncQuickslot(QUICKSLOT_TYPE_ITEM, from, cell)`: the item moved to this cell.
    Cell(u16),
    /// `SyncQuickslot(QUICKSLOT_TYPE_ITEM, from, 255)`: the item is gone.
    Delete,
    /// `CItem::SetCount(0)` of an item [`chains_when_used_up`] names: the slot follows the
    /// first other item of this vnum (`FindSpecifyItem`, then `ChainQuickslotItem`), and is
    /// deleted when there is none.
    Chain(u32),
}

/// A sync an item step left among its records, for [`sync_quickslots`] to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickslotSync {
    /// The cell the item left.
    pub from: u16,
    /// What the slots that named it do.
    pub to: SyncTo,
}

/// Whether `SetCount(0)` of an item of this prototype chains its slots to another item of the
/// same vnum, rather than deleting them (`G/item.cpp:307`).
///
/// Legacy tests the sub type alone, whatever the type, so any item whose sub type is 0 or 7
/// chains too, and so does vnum 70020.
#[must_use]
pub fn chains_when_used_up(proto: &ItemProto) -> bool {
    proto.sub_type == USE_ABILITY_UP || proto.sub_type == USE_POTION || proto.vnum == 70_020
}

impl Quickslots {
    /// `SyncQuickslot(QUICKSLOT_TYPE_ITEM, from, to)`: every item slot that names `from` is
    /// deleted when `to` is `None`, and set to `to` otherwise.
    ///
    /// Legacy passes both cells as a `BYTE`, so a cell past 255 would name another cell. Here
    /// a slot names `from` only when it is that very cell, and a slot moved to a cell past 255
    /// is refused as [`Self::set`] refuses any cell a slot cannot name.
    fn sync_item(&mut self, from: u16, to: Option<u16>, out: &mut Vec<QuickslotRecord>) {
        if to == Some(from) {
            return;
        }
        let Ok(old) = u8::try_from(from) else {
            return;
        };
        let named = Quickslot {
            kind: EQuickSlotType::Item as u8,
            pos: old,
        };
        for slot in (0_u8..).take(QUICKSLOT_MAX_NUM) {
            if self.get(slot) != Some(named) {
                continue;
            }
            match to.map(u8::try_from) {
                None => {
                    let _deleted = self.delete(slot, out);
                }
                Some(Ok(pos)) => {
                    let quickslot = Quickslot {
                        kind: EQuickSlotType::Item as u8,
                        pos,
                    };
                    let _set = self.set(slot, quickslot, out);
                }
                Some(Err(_)) => {}
            }
        }
    }

    /// `ChainQuickslotItem`: the first item slot that names `from` is set to `cell`.
    fn chain_item(&mut self, from: u16, cell: u16, out: &mut Vec<QuickslotRecord>) {
        let (Ok(old), Ok(pos)) = (u8::try_from(from), u8::try_from(cell)) else {
            return;
        };
        let kind = EQuickSlotType::Item as u8;
        let named = Quickslot { kind, pos: old };
        if let Some(slot) = (0_u8..)
            .take(QUICKSLOT_MAX_NUM)
            .find(|slot| self.get(*slot) == Some(named))
        {
            let _set = self.set(slot, Quickslot { kind, pos }, out);
        }
    }

    /// Run one sync on the slots, answering the records it sent.
    pub fn sync(&mut self, sync: QuickslotSync, items: &CharacterItems) -> Vec<QuickslotRecord> {
        let mut out = Vec::new();
        match sync.to {
            SyncTo::Cell(cell) => self.sync_item(sync.from, Some(cell), &mut out),
            SyncTo::Delete => self.sync_item(sync.from, None, &mut out),
            SyncTo::Chain(vnum) => match find_specify_item(items, vnum) {
                Some(cell) => self.chain_item(sync.from, cell, &mut out),
                None => self.sync_item(sync.from, None, &mut out),
            },
        }
        out
    }
}

/// `CHARACTER::FindSpecifyItem` (`G/char_item.cpp:8724-8742`): the cell of the first item of
/// `vnum`, in the base inventory and then in the custom banks, in cell order.
///
/// Legacy reads the base inventory up to `Inventory_Size()`, the unlocked cells. No item lies
/// past them, so reading all 180 finds the same item.
#[must_use]
pub fn find_specify_item(items: &CharacterItems, vnum: u32) -> Option<u16> {
    (0..INVENTORY_MAX_NUM)
        .chain(CUSTOM_INVENTORY_SLOT_START..CUSTOM_INVENTORY_SLOT_END)
        .find(|cell| {
            let at = ItemPos {
                window_type: EWindows::Inventory as u8,
                cell: *cell,
            };
            items.item_at(at).is_some_and(|item| item.vnum == vnum)
        })
}

/// Run every [`QuickslotSync`] an item step left in its records on `slots`, in its place: each
/// becomes the [`MoveRecord::Quickslot`] records it sent. A chain finds its item in `items`,
/// which is the storage after the step, as legacy's `FindSpecifyItem` runs once the used-up
/// item has left.
pub fn sync_quickslots(done: &mut MoveDone, slots: &mut Quickslots, items: &CharacterItems) {
    let records = core::mem::take(&mut done.records);
    for record in records {
        match record {
            MoveRecord::QuickslotSync(sync) => done.records.extend(
                slots
                    .sync(sync, items)
                    .into_iter()
                    .map(MoveRecord::Quickslot),
            ),
            record => done.records.push(record),
        }
    }
}

/// `CInputMain::QuickslotAdd` (`G/input_main.cpp:1083-1113`): an item slot must name a cell
/// that holds an `ITEM_USE` item, or the request is dropped without a record; then
/// [`Quickslots::set`].
pub fn add_from_client(
    slots: &mut Quickslots,
    items: &CharacterItems,
    protos: &ItemProtos,
    slot: u8,
    quickslot: Quickslot,
) -> Vec<QuickslotRecord> {
    let mut out = Vec::new();
    if quickslot.kind == EQuickSlotType::Item as u8 {
        let cell = ItemPos {
            window_type: EWindows::Inventory as u8,
            cell: u16::from(quickslot.pos),
        };
        let usable = items
            .item_at(cell)
            .and_then(|item| protos.get(item.vnum))
            .is_some_and(|proto| proto.item_type == ITEM_USE);
        if !usable {
            return out;
        }
    }
    let _set = slots.set(slot, quickslot, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use gamedata::item_proto::ItemProto;

    use super::*;
    use crate::item::Item;

    const ITEM: u8 = EQuickSlotType::Item as u8;
    const SKILL: u8 = EQuickSlotType::Skill as u8;
    const COMMAND: u8 = EQuickSlotType::Command as u8;

    fn slot(kind: u8, pos: u8) -> Quickslot {
        Quickslot { kind, pos }
    }

    fn set(slots: &mut Quickslots, at: u8, quickslot: Quickslot) -> (bool, Vec<QuickslotRecord>) {
        let mut out = Vec::new();
        let done = slots.set(at, quickslot, &mut out);
        (done, out)
    }

    #[test]
    fn a_set_slot_is_held_and_told_and_every_kind_has_its_own_bound() {
        let mut slots = Quickslots::default();
        assert_eq!(
            set(&mut slots, 3, slot(ITEM, 89)),
            (
                true,
                vec![QuickslotRecord::Add {
                    slot: 3,
                    quickslot: slot(ITEM, 89)
                }]
            )
        );
        assert_eq!(slots.get(3), Some(slot(ITEM, 89)));
        // A worn cell is neither the base inventory nor the belt.
        assert_eq!(set(&mut slots, 4, slot(ITEM, 180)), (false, vec![]));
        assert!(set(&mut slots, 4, slot(ITEM, 179)).0);
        assert!(set(&mut slots, 5, slot(SKILL, 254)).0);
        assert_eq!(set(&mut slots, 6, slot(SKILL, 255)), (false, vec![]));
        assert!(set(&mut slots, 6, slot(COMMAND, 255)).0);
        assert_eq!(set(&mut slots, 7, slot(0, 0)), (false, vec![]));
        assert_eq!(set(&mut slots, 7, slot(4, 0)), (false, vec![]));
        assert_eq!(set(&mut slots, 36, slot(COMMAND, 1)), (false, vec![]));
        assert!(set(&mut slots, 35, slot(COMMAND, 1)).0);
        assert_eq!(slots.get(7), Some(Quickslot::default()));
        assert_eq!(slots.get(36), None);
        assert_eq!(
            slots.set_slots().collect::<Vec<_>>(),
            vec![
                (3, slot(ITEM, 89)),
                (4, slot(ITEM, 179)),
                (5, slot(SKILL, 254)),
                (6, slot(COMMAND, 255)),
                (35, slot(COMMAND, 1)),
            ]
        );
    }

    #[test]
    fn a_slot_set_again_empties_its_twins_first() {
        let mut slots = Quickslots::default();
        let _ = set(&mut slots, 0, slot(SKILL, 9));
        let _ = set(&mut slots, 1, slot(ITEM, 5));
        // The same skill in another slot: the old one is emptied and told first.
        assert_eq!(
            set(&mut slots, 2, slot(SKILL, 9)).1,
            vec![
                QuickslotRecord::Del { slot: 0 },
                QuickslotRecord::Add {
                    slot: 2,
                    quickslot: slot(SKILL, 9)
                },
            ]
        );
        // Set again in its own slot, it is emptied there and set there.
        assert_eq!(
            set(&mut slots, 2, slot(SKILL, 9)).1,
            vec![
                QuickslotRecord::Del { slot: 2 },
                QuickslotRecord::Add {
                    slot: 2,
                    quickslot: slot(SKILL, 9)
                },
            ]
        );
        assert_eq!(slots.get(0), Some(Quickslot::default()));
        assert_eq!(slots.get(1), Some(slot(ITEM, 5)));
    }

    #[test]
    fn a_delete_and_a_swap_are_told_even_when_nothing_changes() {
        let mut slots = Quickslots::default();
        let _ = set(&mut slots, 0, slot(COMMAND, 1));
        let _ = set(&mut slots, 1, slot(COMMAND, 2));
        let mut out = Vec::new();
        assert!(slots.swap(0, 35, &mut out));
        assert!(slots.swap(1, 1, &mut out));
        assert!(!slots.swap(36, 0, &mut out));
        assert!(!slots.swap(0, 36, &mut out));
        assert_eq!(slots.get(35), Some(slot(COMMAND, 1)));
        assert_eq!(slots.get(0), Some(Quickslot::default()));
        assert!(slots.delete(1, &mut out));
        assert!(slots.delete(1, &mut out));
        assert!(!slots.delete(36, &mut out));
        assert_eq!(slots.get(1), Some(Quickslot::default()));
        assert_eq!(
            out,
            vec![
                QuickslotRecord::Swap { slot: 0, with: 35 },
                QuickslotRecord::Swap { slot: 1, with: 1 },
                QuickslotRecord::Del { slot: 1 },
                QuickslotRecord::Del { slot: 1 },
            ]
        );
    }

    #[test]
    fn a_client_item_slot_needs_a_used_item_in_that_cell() {
        let mut potion = ItemProto::for_category_rule(27_001, ITEM_USE, 0);
        potion.values = [300, 0, 0, 0, 0, 0];
        let sword = ItemProto::for_category_rule(19, gamedata::item_kind::ITEM_WEAPON, 0);
        let protos = ItemProtos::from_rows(vec![potion, sword]);
        let mut items = CharacterItems::new();
        for (cell, vnum) in [(2, 27_001), (4, 19), (6, 999)] {
            let at = ItemPos {
                window_type: EWindows::Inventory as u8,
                cell,
            };
            items
                .set(at, &Item::new(u32::from(cell) + 1, vnum))
                .expect("the fixture places");
        }
        let mut slots = Quickslots::default();
        // An empty cell, a weapon and an item with no prototype are dropped without a record.
        for pos in [3, 4, 6] {
            let out = add_from_client(&mut slots, &items, &protos, 0, slot(ITEM, pos));
            assert!(out.is_empty(), "cell {pos}");
        }
        assert_eq!(slots, Quickslots::default());
        assert_eq!(
            add_from_client(&mut slots, &items, &protos, 0, slot(ITEM, 2)),
            vec![QuickslotRecord::Add {
                slot: 0,
                quickslot: slot(ITEM, 2)
            }]
        );
        // A skill slot is not checked against the items.
        assert_eq!(
            add_from_client(&mut slots, &items, &protos, 1, slot(SKILL, 4)),
            vec![QuickslotRecord::Add {
                slot: 1,
                quickslot: slot(SKILL, 4)
            }]
        );
    }

    fn placed(cells: &[(u16, u32)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (cell, vnum) in cells {
            let at = ItemPos {
                window_type: EWindows::Inventory as u8,
                cell: *cell,
            };
            items
                .set(at, &Item::new(u32::from(*cell) + 1, *vnum))
                .expect("the fixture places");
        }
        items
    }

    fn sync(slots: &mut Quickslots, from: u16, to: SyncTo) -> Vec<QuickslotRecord> {
        slots.sync(QuickslotSync { from, to }, &CharacterItems::new())
    }

    #[test]
    fn a_sync_moves_or_deletes_every_item_slot_on_the_old_cell() {
        let mut slots = Quickslots::default();
        let _ = set(&mut slots, 0, slot(ITEM, 3));
        let _ = set(&mut slots, 1, slot(SKILL, 3));
        let _ = set(&mut slots, 2, slot(ITEM, 9));
        // The slot on the new cell is a twin, and is emptied before the move lands.
        assert_eq!(
            sync(&mut slots, 3, SyncTo::Cell(9)),
            vec![
                QuickslotRecord::Del { slot: 2 },
                QuickslotRecord::Add {
                    slot: 0,
                    quickslot: slot(ITEM, 9)
                },
            ]
        );
        assert_eq!(slots.get(1), Some(slot(SKILL, 3)), "a skill is not an item");
        // The same cell, a cell no slot names and a cell past a `BYTE` change nothing.
        assert_eq!(sync(&mut slots, 9, SyncTo::Cell(9)), vec![]);
        assert_eq!(sync(&mut slots, 4, SyncTo::Cell(5)), vec![]);
        assert_eq!(sync(&mut slots, 9 + 256, SyncTo::Cell(5)), vec![]);
        // A cell a slot cannot name leaves the slot where it was.
        assert_eq!(sync(&mut slots, 9, SyncTo::Cell(9 + 256)), vec![]);
        assert_eq!(sync(&mut slots, 9, SyncTo::Cell(180)), vec![]);
        assert_eq!(slots.get(0), Some(slot(ITEM, 9)));
        assert_eq!(
            sync(&mut slots, 9, SyncTo::Delete),
            vec![QuickslotRecord::Del { slot: 0 }]
        );
        assert_eq!(
            slots.set_slots().collect::<Vec<_>>(),
            vec![(1, slot(SKILL, 3))]
        );
    }

    #[test]
    fn a_chain_follows_the_first_item_of_the_vnum_or_deletes_the_slot() {
        let mut slots = Quickslots::default();
        let _ = set(&mut slots, 4, slot(ITEM, 3));
        let bank = CUSTOM_INVENTORY_SLOT_START;
        let items = placed(&[(bank, 27_001), (40, 27_001), (12, 27_002), (20, 27_001)]);
        assert_eq!(find_specify_item(&items, 27_001), Some(20));
        assert_eq!(
            find_specify_item(&placed(&[(bank, 27_001)]), 27_001),
            Some(bank)
        );
        assert_eq!(find_specify_item(&items, 27_003), None);
        let chain = QuickslotSync {
            from: 3,
            to: SyncTo::Chain(27_001),
        };
        assert_eq!(
            slots.sync(chain, &items),
            vec![QuickslotRecord::Add {
                slot: 4,
                quickslot: slot(ITEM, 20)
            }]
        );
        // Only in the banks, past a `BYTE`: the slot stays.
        let chain = QuickslotSync {
            from: 20,
            to: SyncTo::Chain(27_001),
        };
        assert_eq!(slots.sync(chain, &placed(&[(bank, 27_001)])), vec![]);
        assert_eq!(
            slots.sync(chain, &CharacterItems::new()),
            vec![QuickslotRecord::Del { slot: 4 }]
        );
    }

    #[test]
    fn the_syncs_become_their_records_in_their_place() {
        let mut slots = Quickslots::default();
        let _ = set(&mut slots, 4, slot(ITEM, 3));
        let mut done = MoveDone {
            kind: super::super::item_move::MoveKind::Moved,
            records: vec![
                MoveRecord::Notice("first"),
                MoveRecord::QuickslotSync(QuickslotSync {
                    from: 3,
                    to: SyncTo::Cell(7),
                }),
                MoveRecord::QuickslotSync(QuickslotSync {
                    from: 50,
                    to: SyncTo::Delete,
                }),
                MoveRecord::Notice("last"),
            ],
            changes: Vec::new(),
        };
        sync_quickslots(&mut done, &mut slots, &CharacterItems::new());
        assert_eq!(
            done.records,
            vec![
                MoveRecord::Notice("first"),
                MoveRecord::Quickslot(QuickslotRecord::Add {
                    slot: 4,
                    quickslot: slot(ITEM, 7)
                }),
                MoveRecord::Notice("last"),
            ]
        );
    }

    #[test]
    fn a_used_up_item_chains_by_its_sub_type_whatever_its_type() {
        let proto =
            |vnum, item_type, sub_type| ItemProto::for_category_rule(vnum, item_type, sub_type);
        assert!(chains_when_used_up(&proto(27_001, ITEM_USE, USE_POTION)));
        assert!(chains_when_used_up(&proto(
            50_801,
            ITEM_USE,
            USE_ABILITY_UP
        )));
        assert!(chains_when_used_up(&proto(
            19,
            gamedata::item_kind::ITEM_WEAPON,
            0
        )));
        assert!(chains_when_used_up(&proto(70_020, ITEM_USE, 3)));
        let nodelay = gamedata::item_kind::USE_POTION_NODELAY;
        assert!(!chains_when_used_up(&proto(27_101, ITEM_USE, nodelay)));
    }
}
