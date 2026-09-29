//! Quickslots: `CHARACTER::SetQuickslot`, `DelQuickslot` and `SwapQuickslot`
//! (`G/char_quickslot.cpp:45-136`), and the item check `CInputMain::QuickslotAdd` makes first
//! (`G/input_main.cpp:1083-1113`).
//!
//! A character has 36 slots. Each is empty, or names an inventory cell, a skill or a command.
//! Every change answers the records legacy sends for it, in its order.
//!
//! Not ported, and each is a later ledger's: `SyncQuickslot` and `ChainQuickslotItem`, which
//! follow an item that moves, is used up or leaves the inventory.

use common::enums::EQuickSlotType;
use common::item_slots::EWindows;
use gamedata::item_kind::ITEM_USE;
use gamedata::item_proto::ItemProtos;
use protocol::item_pos::ItemPos;

use super::inventory::{is_belt_inventory_position, is_default_inventory_position};
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
}
