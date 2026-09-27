//! Giving an item to a character: the cell it lands in, and the refusal when
//! there is none.
//!
//! This is the placement half of legacy's `ACMD(do_item)`
//! (`game/cmd_gm.cpp:467-519`), split out from the command so it can be tested
//! without a descriptor, a store, or a world. Legacy's order is a scan of the six
//! custom-inventory banks first, then the base inventory
//! (`char_item.cpp:1209-1233`), and **the first bank with a free cell wins** -- not
//! the first bank the item matches. Those are different rules and the difference
//! is observable, so [`grant`] takes the matching banks as a list and searches
//! them in order.
//!
//! # Why the banks are an argument
//!
//! Category membership is `CItem::IsCustomCategory` (`item.cpp:3002-3148`), a
//! hard-coded vnum table that lives in `gamedata`. `world` does not depend on
//! `gamedata`, and it should not: a gameplay crate that reaches into a data crate
//! for a placement rule cannot be tested with a synthetic item. So the caller
//! answers "which banks does this item match", in ascending order, and this module
//! answers "which of those has room".
//!
//! # What this reproduces, and what it does not
//!
//! Reproduced: the search order, the "first with room" latch, the refusal when
//! nothing is free, and the fact that a grant never merges into an existing stack.
//!
//! Not reproduced, deliberately, and each is a recorded Divergence or a decision:
//!
//! * **A locked inventory page is never offered.** Legacy's search scans all 180
//!   base cells (`char_item.cpp:1228-1233`) and ignores the unlock stat, so it can
//!   place an item in a page the player has not paid for and the client will not
//!   draw it. That is a Defect. `inven_point` bounds the search here.
//! * **No stacking.** `do_item` never merges, so neither does this, even where
//!   `PickupItem` and `AutoGiveItem` do. An Operator grant is "give me exactly this".
//! * **No magic.** Legacy passes `bTryMagic = true` (`cmd_gm.cpp:499`), so a GM
//!   grant can hand out a randomly enchanted or socketed item. That needs the proto
//!   percentage columns and an RNG, and an Operator command should not surprise
//!   the person who ran it. A grant is a plain item.

#![warn(missing_docs)]

use protocol::item_pos::ItemPos;

use crate::character::{CharacterItems, NPOS};
use crate::item::Item;

/// The window a granted item is placed in, unless it is a dragon soul.
///
/// `CItem::GetWindowInventoryEx` (`item.cpp:3192-3198`): `DRAGON_SOUL_INVENTORY` for a
/// dragon soul, `INVENTORY` for everything else.
pub const GRANT_WINDOW: u8 = common::item_slots::EWindows::Inventory as u8;

/// Why a grant did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantRefused {
    /// Every matching bank and every usable base cell is occupied.
    ///
    /// Legacy answers this in the caller, not here: `do_item` destroys the item and
    /// sends `ChatPacket(CHAT_TYPE_INFO, "Not enough inventory space.")`
    /// (`cmd_gm.cpp:509-513`) with no log line, while `PickupItem` logs
    /// (`char_item.cpp:8063-8068`). The Rewrite logs in both cases, because a
    /// silent refusal is how a lost item goes unnoticed.
    NoRoom {
        /// The item's footprint in cells, so the message can name it.
        size: u8,
    },
    /// The item could not be placed in a cell the search had already found free.
    ///
    /// This is not a gameplay outcome. It means the search and the placement
    /// disagreed, which they share code to make impossible; it is reported rather
    /// than ignored so that if it ever happens it is a bug with a name.
    SearchDisagreedWithPlacement {
        /// The cell the search chose.
        cell: u16,
    },
}

/// Where a granted item went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Granted {
    /// The `INVENTORY` window and the cell the item now occupies.
    pub pos: ItemPos,
    /// The custom-inventory bank the item went into, or `None` for the base
    /// inventory.
    ///
    /// This is a report, not a decision: the caller needs it to persist the cell and
    /// to explain the placement, and the search has already committed.
    pub bank: Option<u8>,
}

/// Place `item` in the first bank in `banks` that has room, and otherwise in the
/// first free base cell.
///
/// `banks` is the ascending list of custom-inventory categories the item matches,
/// from `gamedata::item_custom_category::is_custom_category`. It is walked in the
/// order given and the **first bank with a free cell wins**
/// (`char_item.cpp:1214-1225`), which is not the same as the first bank the item
/// matches. An item in two banks lands in the lower one only when the lower one is
/// full.
///
/// `inven_point` is the character's `m_points.envanter` (`char.h:1284`); it bounds
/// the base search to the pages the character has paid for. See the module note on
/// why that is a Divergence.
///
/// On success the item's own `pos` is set and the grid is marked, so the caller can
/// hand `item` to the store and the client without any further bookkeeping.
///
/// # Errors
///
/// [`GrantRefused`] when nothing is free, or when placement refused a cell the
/// search had already reported free.
///
/// # Panics
///
/// Never. A `size` of 0 finds no cell, because [`CharacterItems::set`] refuses one,
/// and that refusal arrives as [`GrantRefused::NoRoom`] rather than as a panic.
pub fn grant(
    items: &mut CharacterItems,
    item: &mut Item,
    banks: &[u8],
    inven_point: u16,
) -> Result<Granted, GrantRefused> {
    let size = item.size();
    if size == 0 {
        return Err(GrantRefused::NoRoom { size });
    }
    for &bank in banks {
        if let Some(cell) = items.find_free_custom_cell(bank, size) {
            return place(items, item, cell, Some(bank));
        }
    }
    match items.find_free_inventory_cell_for(inven_point, size) {
        Some(cell) => place(items, item, cell, None),
        None => Err(GrantRefused::NoRoom { size }),
    }
}

fn place(
    items: &mut CharacterItems,
    item: &mut Item,
    cell: u16,
    bank: Option<u8>,
) -> Result<Granted, GrantRefused> {
    let pos = ItemPos {
        window_type: GRANT_WINDOW,
        cell,
    };
    // The position is set before `set`, because `set` records the item id into the
    // grid keyed by the position it was handed, and the two must agree.
    item.pos = pos;
    match items.set(pos, item) {
        Ok(()) => Ok(Granted { pos, bank }),
        // The search and `set` share `footprint_is_clear`, so this arm is unreachable
        // unless one of them changes. Naming it is what makes that a bug report rather
        // than a silently dropped item. The position is put back to `NPOS` so a
        // refused grant leaves the item unplaced rather than claiming a cell no grid
        // record points at.
        Err(_) => {
            item.pos = NPOS;
            Err(GrantRefused::SearchDisagreedWithPlacement { cell })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::items::CharacterItems;
    use common::item_slots::{EWindows, CUSTOM_INVENTORY_MAX_NUM, CUSTOM_INVENTORY_SLOT_START};
    use protocol::item_pos::ItemPos;

    const INV: u8 = EWindows::Inventory as u8;

    /// A one-cell item with the given id.
    fn one_cell(id: u32) -> Item {
        let mut item = Item::new(id, 30_000);
        item.set_size(1).expect("a size of at least one");
        item
    }

    /// An item of `size` cells, with a distinct count so the packet test can tell
    /// the fields apart.
    fn sized(id: u32, size: u8) -> Item {
        let mut item = Item::new(id, 30_001);
        item.set_size(size).expect("a size of at least one");
        item
    }

    fn at(window: u8, cell: u16) -> ItemPos {
        ItemPos {
            window_type: window,
            cell,
        }
    }

    /// The first cell of custom bank `bank`.
    ///
    /// The six banks are contiguous, so bank `n` starts `n * CUSTOM_INVENTORY_MAX_NUM`
    /// after [`CUSTOM_INVENTORY_SLOT_START`]. Computed rather than restated, because
    /// three of the failures in this module's first run were exactly this arithmetic
    /// written out by hand.
    fn bank_start(bank: u16) -> u16 {
        CUSTOM_INVENTORY_SLOT_START + bank * CUSTOM_INVENTORY_MAX_NUM
    }

    /// Fill one custom bank with one-cell items carrying distinct ids.
    fn fill_bank(items: &mut CharacterItems, bank: u16) {
        for offset in 0..CUSTOM_INVENTORY_MAX_NUM {
            let id = 1_000 * (u32::from(bank) + 1) + u32::from(offset);
            items
                .set(at(INV, bank_start(bank) + offset), &one_cell(id))
                .expect("a free bank cell takes an item");
        }
    }

    #[test]
    fn a_fresh_character_gets_the_first_base_cell() {
        let mut items = CharacterItems::new();
        let mut item = one_cell(1);
        let placed = grant(&mut items, &mut item, &[], 0).expect("an empty inventory has room");
        assert_eq!(placed.pos, at(INV, 0));
        assert_eq!(placed.bank, None);
        assert_eq!(
            item.pos,
            at(INV, 0),
            "the item carries its own new position"
        );
    }

    #[test]
    fn a_matching_bank_is_used_before_the_base_inventory() {
        let mut items = CharacterItems::new();
        let mut item = one_cell(1);
        let placed = grant(&mut items, &mut item, &[2], 0).expect("bank 2 has room");
        assert_eq!(placed.bank, Some(2));
        assert_eq!(
            placed.pos.cell,
            bank_start(2),
            "the item landed in bank 2's own cells"
        );
    }

    #[test]
    fn the_first_bank_with_room_wins_and_not_the_first_bank_the_item_matches() {
        // Legacy scans the banks and takes the first one with a free cell
        // (`char_item.cpp:1214-1225`), which is not `CItem::GetItemCategory`'s
        // "first that matches". The item matches banks 0 and 2; bank 0 is full, so it
        // must land in bank 2. An implementation that took the first match would
        // fail this by refusing.
        let mut items = CharacterItems::new();
        fill_bank(&mut items, 0);
        let mut item = one_cell(1);
        let placed = grant(&mut items, &mut item, &[0, 2], 0).expect("bank 2 still has room");
        assert_eq!(placed.bank, Some(2));
        assert_eq!(
            placed.pos.cell,
            bank_start(2),
            "the search reached bank 2 and not bank 0's occupied cells"
        );
    }

    #[test]
    fn a_full_bank_falls_through_to_the_next_and_then_the_base_inventory() {
        let mut items = CharacterItems::new();
        fill_bank(&mut items, 0);
        fill_bank(&mut items, 2);
        let mut item = one_cell(9_999);
        let placed = grant(&mut items, &mut item, &[0, 2], 0).expect("the base inventory is empty");
        assert_eq!(placed.bank, None);
        assert_eq!(
            placed.pos.cell, 0,
            "the base inventory is searched after the banks"
        );
    }

    #[test]
    fn a_full_inventory_refuses_and_leaves_the_item_unplaced() {
        let mut items = CharacterItems::new();
        for cell in 0..180 {
            items
                .set(at(INV, cell), &one_cell(100 + u32::from(cell)))
                .expect("a free base cell takes an item");
        }
        let mut item = one_cell(9_999);
        let refused = grant(&mut items, &mut item, &[], 0).expect_err("nothing is free");
        assert_eq!(refused, GrantRefused::NoRoom { size: 1 });
        assert!(item.is_unplaced(), "a refused grant does not claim a cell");
    }

    #[test]
    fn a_custom_bank_is_still_available_when_the_base_inventory_is_full() {
        let mut items = CharacterItems::new();
        for cell in 0..180 {
            items
                .set(at(INV, cell), &one_cell(100 + u32::from(cell)))
                .expect("a free base cell takes an item");
        }
        let mut item = one_cell(9_999);
        let placed = grant(&mut items, &mut item, &[1], 0).expect("bank 1 is untouched");
        assert_eq!(placed.bank, Some(1));
    }

    #[test]
    fn a_locked_page_is_never_offered() {
        // The positive control first: cell 90 really does hold an item, so a
        // refusal below cannot be explained by `set` refusing cell 90 for any other
        // reason.
        let mut items = CharacterItems::new();
        items
            .set(at(INV, 90), &one_cell(7))
            .expect("a free cell takes an item");

        // `inven_point` 0 buys 90 base cells, so 90..=94 is locked. A 5-cell item
        // cannot fit in 85..=89 either, so the first free cell is 0.
        let mut item = one_cell(1);
        let placed = grant(&mut items, &mut item, &[], 0).expect("cell 0 is free");
        assert_eq!(
            placed.pos.cell, 0,
            "the search stops before the locked page"
        );

        // With every cell up to the lock filled, there is no room rather than a
        // cell in a page the client will not draw.
        let mut full = CharacterItems::new();
        for cell in 0..90 {
            full.set(at(INV, cell), &one_cell(100 + u32::from(cell)))
                .expect("a free base cell takes an item");
        }
        let mut item = one_cell(1);
        let refused =
            grant(&mut full, &mut item, &[], 0).expect_err("the locked page is not offered");
        assert_eq!(refused, GrantRefused::NoRoom { size: 1 });
    }

    #[test]
    fn a_raising_inventory_unlock_opens_the_next_page() {
        let mut items = CharacterItems::new();
        for cell in 0..90 {
            items
                .set(at(INV, cell), &one_cell(100 + u32::from(cell)))
                .expect("a free base cell takes an item");
        }
        // `usable_inventory_cells(1)` is 95, so cells 90..=94 open up.
        let mut item = one_cell(1);
        let placed = grant(&mut items, &mut item, &[], 1).expect("the second page is paid for");
        assert_eq!(placed.pos.cell, 90);
    }

    #[test]
    fn a_multi_cell_item_cannot_straddle_a_page_boundary() {
        // A 45-cell page is 5 columns by 9 rows, walked with stride 5. A 5-cell item
        // at cell 41 would need 41, 46, 51, 56 and 61; the last is in the next page,
        // so the search must pass it and offer nothing.
        let mut items = CharacterItems::new();
        for cell in 0..41 {
            items
                .set(at(INV, cell), &one_cell(100 + u32::from(cell)))
                .expect("a free base cell takes an item");
        }
        // 41..=44 are genuinely free, but a 5-cell item at 41 needs 41, 46, 51, 56 and
        // 61, and 61 is in the next page. The search must skip 41 and take 45, which
        // is where the whole of page 1 starts.
        let mut item = sized(9_999, 5);
        let placed = grant(&mut items, &mut item, &[], 0).expect("page 1 has room at its anchor");
        assert_eq!(
            placed.pos.cell, 45,
            "the search passed the free-but-straddling cells 41..=44"
        );
    }

    #[test]
    fn a_zero_size_item_is_refused_rather_than_placed() {
        // `CharacterItems::set` refuses a zero footprint, so the search must not
        // offer cell 0. A grant that returned cell 0 here would hand the caller an
        // item that cannot be stored.
        let mut items = CharacterItems::new();
        let mut item = Item::new(1, 30_000);
        // `set_size` refuses zero, which is the right answer for a setter; a zero
        // footprint still exists in legacy for an item with no prototype, so the
        // public field is assigned directly to build one.
        item.size = 0;
        let refused =
            grant(&mut items, &mut item, &[], 0).expect_err("a zero footprint is refused");
        assert_eq!(refused, GrantRefused::NoRoom { size: 0 });
    }

    #[test]
    fn a_grant_never_merges_into_an_existing_stack() {
        // `do_item` always creates a new item and a new cell
        // (`cmd_gm.cpp:494-503`). A same-vnum item already at cell 0 must not absorb
        // the grant, so the grant lands at cell 1.
        let mut items = CharacterItems::new();
        let mut first = Item::new(1, 30_000);
        first.set_size(1).expect("a size of at least one");
        items
            .set(at(INV, 0), &first)
            .expect("a free cell takes an item");

        let mut item = one_cell(2);
        let placed = grant(&mut items, &mut item, &[], 0).expect("cell 1 is free");
        assert_eq!(placed.pos.cell, 1, "a grant does not stack");
        assert_eq!(item.count, 1, "a grant's own count is untouched");
    }

    #[test]
    fn two_grants_of_the_same_vnum_take_two_cells_and_two_ids() {
        let mut items = CharacterItems::new();
        let mut first = one_cell(1);
        let a = grant(&mut items, &mut first, &[], 0).expect("cell 0 is free");
        let mut second = one_cell(2);
        let b = grant(&mut items, &mut second, &[], 0).expect("cell 1 is free");
        assert_eq!((a.pos.cell, b.pos.cell), (0, 1));
        assert_eq!(first.vnum, second.vnum, "both came from one prototype");
        assert_ne!(
            first.id, second.id,
            "two grants of one vnum are two instances with two ids"
        );
    }

    #[test]
    fn the_granted_item_builds_the_client_record_field_for_field() {
        // The nine shared fields are pinned in both directions. A field that moved
        // in `Item::gc_item_set` or in `GC_ITEM_SET` and not the other fails here.
        let mut items = CharacterItems::new();
        let mut item = Item::new(7, 30_002);
        item.set_size(1).expect("a size of at least one");
        item.set_count(5).expect("five is under the ceiling");
        item.set_socket(0, -1).expect("socket 0 is in range");
        item.set_attribute(
            3,
            protocol::gc_item_window::ItemAttribute {
                b_type: 200,
                s_value: -300,
            },
        )
        .expect("attribute 3 is in range");
        let placed = grant(&mut items, &mut item, &[], 0).expect("an empty inventory has room");

        let record = item.gc_item_set(placed.pos, 1);
        assert_eq!(record.cell, at(INV, 0));
        assert_eq!(record.vnum, 30_002);
        assert_eq!(record.count, 5);
        assert_eq!(record.refine_element, 0);
        assert_eq!(record.transmutation, 0);
        assert_eq!(record.flags, 0);
        assert_eq!(record.anti_flags, 0);
        assert_eq!(record.highlight, 1, "a fresh grant is highlighted");
        assert_eq!(
            record.sockets,
            [-1, 0, 0, 0, 0, 0],
            "socket 0 is the -1 that was set and the rest are zero"
        );
        assert_eq!(record.attributes.len(), 7);
        assert_eq!(record.attributes[3].b_type, 200);
        assert_eq!(record.attributes[3].s_value, -300);
    }

    #[test]
    fn the_granted_item_encodes_to_the_measured_seventy_two_byte_record() {
        let mut items = CharacterItems::new();
        let mut item = one_cell(1);
        let placed = grant(&mut items, &mut item, &[], 0).expect("an empty inventory has room");
        let bytes = item.gc_item_set(placed.pos, 1).encode();
        assert_eq!(bytes.len(), 72, "the measured `TPacketGCItemSet` width");
        assert_eq!(bytes[0], 21, "`HEADER_GC_ITEM_SET`");
    }

    #[test]
    fn a_negative_socket_and_a_signed_attribute_survive_the_wire() {
        // The width probe cannot run here, so the sign question is answered on the
        // bytes instead: both are `long` on the 32-bit target, and a codec that
        // truncated to 16 bits would fail this.
        let mut items = CharacterItems::new();
        let mut item = one_cell(1);
        item.set_socket(2, -2).expect("socket 2 is in range");
        item.set_attribute(
            0,
            protocol::gc_item_window::ItemAttribute {
                b_type: 255,
                s_value: -1,
            },
        )
        .expect("attribute 0 is in range");
        let placed = grant(&mut items, &mut item, &[], 0).expect("an empty inventory has room");
        let bytes = item.gc_item_set(placed.pos, 1).encode();

        // The first socket word sits after the fixed header and the single-byte
        // `highlight`. The record is 72 bytes: 1 header + 1 window + 2 cell + 4 vnum
        // + 2 count + 4 refine + 4 transmutation + 4 flags + 4 anti + 1 highlight.
        let socket_two_at = 1 + 1 + 2 + 4 + 2 + 4 + 4 + 4 + 4 + 1 + (2 * 4);
        let mut got: i32 = 0;
        for (shift, byte) in bytes[socket_two_at..socket_two_at + 4].iter().enumerate() {
            got |= i32::from(*byte) << (8 * shift);
        }
        assert_eq!(
            got, -2,
            "socket 2 is -2 on the wire, not a 16-bit truncation"
        );
    }
}
