//! The item slot space of `server/server/common/length.h` and `item_length.h`.
//!
//! A character does not own one array per window. It owns **one flat array of
//! `INVENTORY_AND_EQUIP_SLOT_MAX` item slots** plus a second array of
//! `DRAGON_SOUL_INVENTORY_MAX_NUM` dragon soul slots (`char.h:458-461`), and the
//! window byte in a `TItemPos` selects *which array*, not *which array of arrays*.
//! The windows are laid out end to end in one address space, which is why the
//! `TItemPos` `cell` is a plain index with no per-window base.
//!
//! # The layout, with this tree's feature switches
//!
//! | range | constant | start | end |
//! |---|---|---|---|
//! | base inventory | `INVENTORY_MAX_NUM` | 0 | 180 |
//! | equipment | `WEAR_MAX_NUM` | 180 | 244 |
//! | dragon soul equipment | `DRAGON_SOUL_EQUIP_SLOT_START..END` | 244 | 256 |
//! | dragon soul reserved | `DRAGON_SOUL_EQUIP_RESERVED_SLOT_END` | 256 | 274 |
//! | belt inventory | `BELT_INVENTORY_SLOT_START..END` | 274 | 290 |
//! | custom inventory | `CUSTOM_INVENTORY_SLOT_START..END` | 290 | 1370 |
//!
//! The six ranges are contiguous: every start is the previous end, and
//! `INVENTORY_AND_EQUIP_SLOT_MAX` is 1370. The custom inventory is the last
//! range, so without `ENABLE_CUSTOM_INVENTORY` the space would end at
//! [`crate::item_slots::BELT_INVENTORY_SLOT_END`], 290. The tests check both.
//!
//! The custom inventory is addressed through the **inventory window byte**, not a
//! window of its own: [`crate::item_slots::EWindows::Inventory`] plus a cell at or above
//! [`crate::item_slots::CUSTOM_INVENTORY_SLOT_START`]. There are [`crate::item_slots::CUSTOM_INVENTORY_CATEGORY_NUM`]
//! categories of [`crate::item_slots::CUSTOM_INVENTORY_MAX_NUM`] cells each, laid out in category
//! order, so category `c` starts at `CUSTOM_INVENTORY_SLOT_START + c *
//! CUSTOM_INVENTORY_MAX_NUM` (`char_item.cpp:314`).
//!
//! # Two rustdoc notes, so the next edit does not undo them
//!
//! The links in this block are written as [`crate::item_slots::EWindows`] and so
//! on, not as a bare name. A module's inner `//!` documentation resolves its intra-doc links
//! in the **crate root** scope on this toolchain, not in its own module scope,
//! so a bare `EWindows` here fails `RUSTDOCFLAGS="-D warnings"` even though the
//! enum is declared 200 lines below. A link inside an *item's* doc comment in
//! this same file resolves normally, which is the asymmetry that makes it look
//! like a typo rather than a scope rule.
//!
//! # How the numbers were derived
//!
//! The arithmetic is spelled out as comments in the legacy source, and **those
//! comments are stale**. `length.h:917` reads
//! `DRAGON_SOUL_EQUIP_SLOT_START = INVENTORY_MAX_NUM + WEAR_MAX_NUM, // 180 + 32 ( 212 )`
//! but `WEAR_MAX_NUM` is 64 at `length.h:89`, so the enumerator starts at 244, not
//! 212. `length.h:25` likewise reads `INVENTORY_MAX_NUM = ... // 90 (default)`
//! where `ENABLE_EXTEND_INVEN_SYSTEM` makes the count 4, not the default 2, so it
//! is 180. The same trap closed ledger 192's `MAX_APPLY_NUM` table. Only the
//! compiler settles it.
//!
//! So the numbers here are **measured, not read**: the verbatim enum bodies from
//! `length.h` and `item_length.h` were compiled with the switches
//! `prodomodefines.h` defines in this snapshot -- [`crate::features::ENABLE_EXTEND_INVEN_SYSTEM`],
//! [`crate::features::ENABLE_CUSTOM_INVENTORY`], [`crate::features::ENABLE_DRAGONSOUL_ALCHEMY_PLUS`],
//! [`crate::features::EXTENDED_SAFEBOX`], [`crate::features::ATTR_6TH_7TH`],
//! [`crate::features::AURA_SYSTEM`], and [`crate::features::ENABLE_SWITCHBOT`] -- and the
//! compiler's own numbering was printed. The probe is
//! `.scratch/probe194/slot_space.cpp`; it re-checks every constant against an
//! independently written hand sum and against the contiguity of the ranges, so a
//! mistranscribed constant fails the build rather than quietly shifting every
//! later range. A deployment that changes a switch must regenerate this module,
//! because legacy would then relayout the array it indexes.
//!
//! # The two arrays are big and the safebox is not one of them
//!
//! On the 32-bit legacy target the four arrays in `char.h:458-461` come to
//! 15,132 bytes per character: `pItems` 5,480, `bItemGrid` 2,740, `pDSItems`
//! 4,608, and `wDSItemGrid` 2,304. The safebox is *not* in this space: its
//! [`crate::item_slots::SAFEBOX_MAX_NUM`] pages are a separate structure, and
//! `SItemPos::IsValidItemPosition` rejects a safebox position outright
//! (`length.h:985-987`).

/// The window byte of a `TItemPos`, and the `BYTE` a record's own `window` field
/// carries.
///
/// Legacy `enum EWindows` at `length.h:657-676`. The values were **measured** with
/// the compiler, not counted: three members sit behind
/// `__ATTR_6TH_7TH__`, `__AURA_SYSTEM__`, and `ENABLE_SWITCHBOT`, all three of
/// which are live in this snapshot, so the enumerator reaches
/// [`EWindows::Ground`] at 10. A build with fewer switches live would renumber
/// every member from [`EWindows::Attr67Add`] up, which is why the table is
/// measured and why the test below pins all eleven.
///
/// This enum replaces an earlier eight-member version that omitted
/// `ATTR67_ADD`, `AURA_REFINE`, and `SWITCHBOT` and therefore misnumbered
/// `BELT_INVENTORY` as 6 and `GROUND` as 7. The window byte travels on the wire
/// in well over ninety records, so those were live wrong bytes; see ledger 194.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EWindows {
    /// `RESERVED_WINDOW` 0. Never a valid item position.
    ReservedWindow = 0,
    /// `INVENTORY` 1. The base inventory, the equipment, the dragon soul and belt
    /// ranges, *and* the custom inventory all use this byte.
    Inventory = 1,
    /// `EQUIPMENT` 2. Shares the flat array with `INVENTORY`.
    Equipment = 2,
    /// `SAFEBOX` 3. Rejected by `IsValidItemPosition`; not part of the slot space.
    Safebox = 3,
    /// `MALL` 4. Rejected by `IsValidItemPosition`; not part of the slot space.
    Mall = 4,
    /// `DRAGON_SOUL_INVENTORY` 5. Selects the second array, `pDSItems`.
    DragonSoulInventory = 5,
    /// `ATTR67_ADD` 6, behind `__ATTR_6TH_7TH__`. The refinement-slot window.
    Attr67Add = 6,
    /// `AURA_REFINE` 7, behind `__AURA_SYSTEM__`. Never a valid item position.
    AuraRefine = 7,
    /// `SWITCHBOT` 8, behind `ENABLE_SWITCHBOT`. Never in the player's arrays.
    Switchbot = 8,
    /// `BELT_INVENTORY` 9. Lives in the flat array at
    /// [`BELT_INVENTORY_SLOT_START`], 274.
    BeltInventory = 9,
    /// `GROUND` 10. A dropped item, set at `item.cpp:578`. Never a valid item
    /// position on a character.
    Ground = 10,
}

/// A byte that is not an `EWindows` enumerator.
///
/// The legacy enum has eleven members numbered 0 to 10, so 245 of the 256 bytes a
/// client can send name no window. `IsValidItemPosition` reaches its
/// `default: return false` arm for all of them and so does [`cell_bound`], so
/// this is an ordinary outcome and not an error condition of its own; it exists
/// because a caller that wants to *name* a window has to be told it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownWindow {
    /// The byte that named no enumerator.
    pub byte: u8,
}

impl core::fmt::Display for UnknownWindow {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "byte {} is not an EWindows member", self.byte)
    }
}

impl std::error::Error for UnknownWindow {}

impl From<EWindows> for u8 {
    /// The wire byte this window travels as.
    fn from(window: EWindows) -> Self {
        window as u8
    }
}

impl TryFrom<u8> for EWindows {
    type Error = UnknownWindow;

    /// Name the window a `TItemPos` byte names.
    ///
    /// The whole byte range is covered, including the 245 values the enumerator
    /// does not use, so this never truncates and never guesses: the enum is
    /// dense from 0 to [`EWindows::Ground`] and each member is its own value.
    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        match byte {
            0 => Ok(Self::ReservedWindow),
            1 => Ok(Self::Inventory),
            2 => Ok(Self::Equipment),
            3 => Ok(Self::Safebox),
            4 => Ok(Self::Mall),
            5 => Ok(Self::DragonSoulInventory),
            6 => Ok(Self::Attr67Add),
            7 => Ok(Self::AuraRefine),
            8 => Ok(Self::Switchbot),
            9 => Ok(Self::BeltInventory),
            10 => Ok(Self::Ground),
            _ => Err(UnknownWindow { byte }),
        }
    }
}

/// `INVENTORY_PAGE_SIZE` = 45, the product of the 5 columns and 9 rows at
/// `length.h:17-19`. This is a packet-level page: `GC_ITEM_SET` carries
/// `INVENTORY_PAGE_SIZE` slots in its grid.
pub const INVENTORY_PAGE_SIZE: u16 = 45;

/// `INVENTORY_PAGE_COUNT` = 4, because `ENABLE_EXTEND_INVEN_SYSTEM` selects the
/// four-page inventory at `length.h:20-24`. The legacy comment's `2 (default)`
/// is the value the other arm would give.
pub const INVENTORY_PAGE_COUNT: u16 = 4;

/// `INVENTORY_MAX_NUM` = 180, the base inventory: `INVENTORY_PAGE_SIZE *
/// INVENTORY_PAGE_COUNT`. The legacy comment's `90 (default)` is the two-page
/// figure and is wrong for this build.
pub const INVENTORY_MAX_NUM: u16 = 180;

/// `INVENTORY_OPEN_PAGE_COUNT` = 2 (`length.h:278`): the pages a fresh character
/// may use with no inventory stat.
pub const INVENTORY_OPEN_PAGE_COUNT: u16 = 2;

/// `INVENTORY_OPEN_PAGE_SIZE` = 90 = `INVENTORY_OPEN_PAGE_COUNT *
/// INVENTORY_PAGE_SIZE` (`length.h:289`).
pub const INVENTORY_OPEN_PAGE_SIZE: u16 = 90;

/// `INVENTORY_WIDTH` = 5 (`length.h:285`).
pub const INVENTORY_WIDTH: u16 = 5;

/// `INVENTORY_HEIGHT` = 9 (`length.h:286`).
pub const INVENTORY_HEIGHT: u16 = 9;

/// The number of base-inventory cells a character may actually use:
/// `INVENTORY_OPEN_PAGE_SIZE + INVENTORY_WIDTH * Inven_Point()`.
///
/// This is `CHARACTER::Inventory_Size` at `char.h:1285`, and it is **not** the
/// array length. The array is always [`INVENTORY_MAX_NUM`] cells whether or not
/// the character has the stat, so a cell between the usable count and 180 is
/// inside the array, passes [`crate::item_slots::cell_bound`], and is still not
/// a cell the player's inventory has been extended to cover. The Rewrite needs
/// the two numbers apart: the bound is for memory safety and the usable count is
/// a gameplay rule. The legacy expression is a bare sum, so a stat above 18
/// would name cells past the base inventory; callers clamp it.
pub const fn usable_inventory_cells(inven_point: u16) -> u16 {
    INVENTORY_OPEN_PAGE_SIZE + INVENTORY_WIDTH * inven_point
}

/// `WEAR_MAX_NUM` = 64 (`length.h:89`). The equipment window. The comment at
/// `length.h:917` says 32 and is stale.
pub const WEAR_MAX_NUM: u16 = 64;

/// `DRAGON_SOUL_EQUIP_SLOT_START` = 244 = `INVENTORY_MAX_NUM + WEAR_MAX_NUM`.
pub const DRAGON_SOUL_EQUIP_SLOT_START: u16 = 244;

/// `DRAGON_SOUL_EQUIP_SLOT_END` = 256 = the start plus
/// `DS_SLOT_MAX * DRAGON_SOUL_DECK_MAX_NUM` = 6 * 2.
pub const DRAGON_SOUL_EQUIP_SLOT_END: u16 = 256;

/// `DRAGON_SOUL_EQUIP_RESERVED_SLOT_END` = 274 = the end plus
/// `DS_SLOT_MAX * DRAGON_SOUL_DECK_RESERVED_MAX_NUM` = 6 * 3. The legacy comment
/// says 242, which is the whole space up to here in the old 32-slot build.
pub const DRAGON_SOUL_EQUIP_RESERVED_SLOT_END: u16 = 274;

/// `BELT_INVENTORY_SLOT_START` = 274, the end of the dragon soul reserved range.
pub const BELT_INVENTORY_SLOT_START: u16 = 274;

/// `BELT_INVENTORY_SLOT_END` = 290 = the start plus
/// [`BELT_INVENTORY_SLOT_COUNT`], 16.
pub const BELT_INVENTORY_SLOT_END: u16 = 290;

/// `CUSTOM_INVENTORY_SLOT_START` = 290, the end of the belt range. This is where
/// the six custom-inventory categories begin.
pub const CUSTOM_INVENTORY_SLOT_START: u16 = 290;

/// `CUSTOM_INVENTORY_SLOT_END` = 1370 = the start plus
/// `CUSTOM_INVENTORY_MAX_NUM * CUSTOM_INVENTORY_CATEGORY_NUM` = 180 * 6.
pub const CUSTOM_INVENTORY_SLOT_END: u16 = 1370;

/// `INVENTORY_AND_EQUIP_SLOT_MAX` = 1370, the length of `pItems` and `bItemGrid`
/// (`char.h:458-459`). **This is the size of the whole flat slot space** and the
/// upper bound `IsValidItemPosition` applies to an inventory or equipment cell.
pub const INVENTORY_AND_EQUIP_SLOT_MAX: u16 = 1370;

/// `BELT_INVENTORY_SLOT_WIDTH` = 4 (`length.h:118`).
pub const BELT_INVENTORY_SLOT_WIDTH: u16 = 4;

/// `BELT_INVENTORY_SLOT_HEIGHT` = 4 (`length.h:119`).
pub const BELT_INVENTORY_SLOT_HEIGHT: u16 = 4;

/// `BELT_INVENTORY_SLOT_COUNT` = 16, the width times the height.
///
/// The belt cells are **stored** under [`EWindows::Inventory`], not under
/// [`EWindows::BeltInventory`]: `CHARACTER::SetCell` (`char_item.cpp:617-631`)
/// relabels a held item `INVENTORY` when its cell is in the base inventory, the
/// belt band, or a custom category, and `EQUIPMENT` for everything else in the
/// flat space -- the equipment, dragon soul, and reserved bands. So the window
/// byte a client sends and the window byte the database holds are not the same
/// field, and [`EWindows::BeltInventory`] survives only as the value a belt item
/// is persisted with before that relabelling.
pub const BELT_INVENTORY_SLOT_COUNT: u16 = 16;

/// `CUSTOM_INVENTORY_PAGE_SIZE` = 45 (`length.h:28`).
pub const CUSTOM_INVENTORY_PAGE_SIZE: u16 = 45;

/// `CUSTOM_INVENTORY_PAGE_COUNT` = 4 (`length.h:29`).
pub const CUSTOM_INVENTORY_PAGE_COUNT: u16 = 4;

/// `CUSTOM_INVENTORY_MAX_NUM` = 180, one category's worth of cells.
pub const CUSTOM_INVENTORY_MAX_NUM: u16 = 180;

/// `CUSTOM_INVENTORY_CATEGORY_NUM` = 6 (`length.h:31`). Six categories, so the
/// client's category index is 0 to 5 and anything 6 or above is refused.
pub const CUSTOM_INVENTORY_CATEGORY_NUM: u8 = 6;

/// `DS_SLOT_MAX` = 6 (`item_length.h:181`, auto-incremented after `DS_SLOT1` to
/// `DS_SLOT6`). The number of slots in one dragon soul deck.
pub const DS_SLOT_MAX: u16 = 6;

/// `DRAGON_SOUL_DECK_MAX_NUM` = 2 (`length.h:263`), the decks a character may fill.
pub const DRAGON_SOUL_DECK_MAX_NUM: u16 = 2;

/// `DRAGON_SOUL_DECK_RESERVED_MAX_NUM` = 3 (`length.h:265`).
pub const DRAGON_SOUL_DECK_RESERVED_MAX_NUM: u16 = 3;

/// `DRAGON_SOUL_BOX_SIZE` = 32 (`length.h:83`), the cells in one dragon soul box.
pub const DRAGON_SOUL_BOX_SIZE: u16 = 32;

/// `DRAGON_SOUL_GRADE_MAX` = 6. Five grades, plus `DRAGON_SOUL_GRADE_MYTHIC`
/// because `ENABLE_DRAGONSOUL_ALCHEMY_PLUS` is live (`item_length.h:191-194`).
pub const DRAGON_SOUL_GRADE_MAX: u16 = 6;

/// `DRAGON_SOUL_INVENTORY_MAX_NUM` = 1152 = `DS_SLOT_MAX * DRAGON_SOUL_GRADE_MAX *
/// DRAGON_SOUL_BOX_SIZE` = 6 * 6 * 32 (`item_length.h:211`). The length of
/// `pDSItems` and `wDSItemGrid` (`char.h:460-461`), and the bound
/// `IsValidItemPosition` applies to a dragon soul cell.
pub const DRAGON_SOUL_INVENTORY_MAX_NUM: u16 = 1152;

/// `ATTR67_ADD_SLOT_MAX` = 1 (`length.h:202`). The refinement window holds a
/// single slot.
pub const ATTR67_ADD_SLOT_MAX: u16 = 1;

/// `SWITCHBOT_SLOT_COUNT` = 5 (`length.h:937`).
pub const SWITCHBOT_SLOT_COUNT: u16 = 5;

/// `SAFEBOX_MAX_PAGE_COUNT` = 6 (`length.h:92`), behind `__EXTENDED_SAFEBOX__`.
pub const SAFEBOX_MAX_PAGE_COUNT: u16 = 6;

/// `SAFEBOX_MAX_NUM` = 270 = 45 pages times [`SAFEBOX_MAX_PAGE_COUNT`]
/// (`length.h:93`). **Not** part of the flat slot space.
pub const SAFEBOX_MAX_NUM: u16 = 270;

/// One contiguous range of cells in the flat slot space, `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotRange {
    /// The first cell in the range, inclusive.
    pub start: u16,
    /// One past the last cell in the range, exclusive.
    pub end: u16,
}

impl SlotRange {
    /// Whether `cell` falls inside the range.
    pub const fn contains(self, cell: u16) -> bool {
        cell >= self.start && cell < self.end
    }

    /// How many cells the range spans.
    pub const fn len(self) -> u16 {
        self.end - self.start
    }

    /// Whether the range spans no cells at all.
    ///
    /// None of the six ranges in this module is empty, so this exists for a caller
    /// that builds one from a character's own counts rather than from the
    /// compiled-in table.
    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

/// The base inventory, cells 0 to [`INVENTORY_MAX_NUM`].
pub const INVENTORY_RANGE: SlotRange = SlotRange {
    start: 0,
    end: INVENTORY_MAX_NUM,
};

/// The equipment window, [`INVENTORY_MAX_NUM`] to [`DRAGON_SOUL_EQUIP_SLOT_START`].
pub const EQUIPMENT_RANGE: SlotRange = SlotRange {
    start: INVENTORY_MAX_NUM,
    end: DRAGON_SOUL_EQUIP_SLOT_START,
};

/// The dragon soul equipment slots, 244 to [`DRAGON_SOUL_EQUIP_SLOT_END`].
pub const DRAGON_SOUL_EQUIP_RANGE: SlotRange = SlotRange {
    start: DRAGON_SOUL_EQUIP_SLOT_START,
    end: DRAGON_SOUL_EQUIP_SLOT_END,
};

/// The reserved dragon soul slots,
/// [`DRAGON_SOUL_EQUIP_SLOT_END`] to [`DRAGON_SOUL_EQUIP_RESERVED_SLOT_END`].
pub const DRAGON_SOUL_RESERVED_RANGE: SlotRange = SlotRange {
    start: DRAGON_SOUL_EQUIP_SLOT_END,
    end: DRAGON_SOUL_EQUIP_RESERVED_SLOT_END,
};

/// The belt inventory, [`BELT_INVENTORY_SLOT_START`] to
/// [`BELT_INVENTORY_SLOT_END`].
pub const BELT_INVENTORY_RANGE: SlotRange = SlotRange {
    start: BELT_INVENTORY_SLOT_START,
    end: BELT_INVENTORY_SLOT_END,
};

/// All six custom-inventory categories, [`CUSTOM_INVENTORY_SLOT_START`] to
/// [`CUSTOM_INVENTORY_SLOT_END`].
pub const CUSTOM_INVENTORY_RANGE: SlotRange = SlotRange {
    start: CUSTOM_INVENTORY_SLOT_START,
    end: CUSTOM_INVENTORY_SLOT_END,
};

/// The dragon soul box, cells 0 to [`DRAGON_SOUL_INVENTORY_MAX_NUM`].
///
/// This is the *second* array (`pDSItems`), indexed by a cell that is
/// independent of the flat space: a dragon soul item at cell 0 is a different
/// thing from a base-inventory item at cell 0, and the window byte is what tells
/// them apart.
pub const DRAGON_SOUL_INVENTORY_RANGE: SlotRange = SlotRange {
    start: 0,
    end: DRAGON_SOUL_INVENTORY_MAX_NUM,
};

/// The first cell of custom-inventory category `category`.
///
/// `char_item.cpp:314` computes `CUSTOM_INVENTORY_SLOT_START + (category *
/// CUSTOM_INVENTORY_MAX_NUM)`. The caller refuses a category at or above
/// [`CUSTOM_INVENTORY_CATEGORY_NUM`] first, so the product cannot overflow the
/// legacy `WORD`; see [`custom_inventory_start`], which does the check.
pub const fn custom_inventory_start(category: u8) -> u16 {
    CUSTOM_INVENTORY_SLOT_START + category as u16 * CUSTOM_INVENTORY_MAX_NUM
}

/// The first cell of custom-inventory category `category`, or `None` when
/// `category` is at or above [`CUSTOM_INVENTORY_CATEGORY_NUM`].
///
/// This is the bound every sibling accessor checks and
/// `CHARACTER::GetInventoryPageByPos` does **not**; see ledger 194.
pub const fn custom_inventory_start_checked(category: u8) -> Option<u16> {
    if category >= CUSTOM_INVENTORY_CATEGORY_NUM {
        None
    } else {
        Some(custom_inventory_start(category))
    }
}

/// Which custom-inventory category `cell` falls in, or `None` when it falls in
/// none of them.
///
/// Legacy `SItemPos::GetCustomInventoryCategory` at `length.h:1034-1043` returns
/// `-1` when no category matches. It loops rather than dividing, so it finds the
/// same answer, and it **does not look at the window byte**, matching the rest of
/// the custom-inventory rule: the category is a property of the cell alone.
pub const fn custom_inventory_category(cell: u16) -> Option<u8> {
    let mut category: u8 = 0;
    while category < CUSTOM_INVENTORY_CATEGORY_NUM {
        if cell >= custom_inventory_start(category) && cell < custom_inventory_start(category + 1) {
            return Some(category);
        }
        category += 1;
    }
    None
}

/// The upper bound `SItemPos::IsValidItemPosition` applies to a cell of `window`,
/// or `None` when the window is never a valid item position.
///
/// This is the legacy `switch` at `length.h:973-1000` as a table:
///
/// | window | bound |
/// |---|---|
/// | `INVENTORY`, `EQUIPMENT`, `BELT_INVENTORY` | [`INVENTORY_AND_EQUIP_SLOT_MAX`] |
/// | `DRAGON_SOUL_INVENTORY` | [`DRAGON_SOUL_INVENTORY_MAX_NUM`] |
/// | `ATTR67_ADD` | [`ATTR67_ADD_SLOT_MAX`] |
/// | `SWITCHBOT` | [`SWITCHBOT_SLOT_COUNT`] |
/// | `RESERVED_WINDOW`, `SAFEBOX`, `MALL`, `AURA_REFINE`, `GROUND`, anything else | never |
///
/// The two rejections that surprise are `SAFEBOX` and `MALL`: they are real
/// windows a client can open, but the legacy validity check refuses a safebox or
/// mall position outright, so a `TItemPos` naming one can never address a held
/// item. The safebox has its own page structure instead.
///
///
/// # This is the `SItemPos` check, not the one `GetItem` calls
///
/// Legacy has **two** functions of this name and they disagree. This one is
/// `SItemPos::IsValidItemPosition` (`length.h:973`), a member of the struct, and
/// it rejects a safebox or a mall position outright. The other is
/// `CHARACTER::IsValidItemPosition` (`char_item.cpp:10010-10049`), and it defers
/// those two to the live safebox and mall. `CHARACTER::GetItem` calls the
/// **second** one (`char_item.cpp:256`); `exchange.cpp:191,327` and
/// `DragonSoul.cpp:1163` call the first. `world::character` transcribes both --
/// see its `character_cell_bound` -- because a check that disagreed with the
/// caller above it would be the wrong one to copy.
///
/// They also differ on a third window, in the other direction: this table gives
/// [`EWindows::BeltInventory`] the full [`INVENTORY_AND_EQUIP_SLOT_MAX`] bound
/// because `length.h:981-982` says so, but the `CHARACTER` switch has no belt
/// case and reaches its `default: return false`. So a belt position is valid
/// here and refused there.
///
/// The match is on the raw byte rather than on [`EWindows`] because that is what
/// the legacy `switch` matches on, and because `try_from` is not a `const fn`.
/// The two therefore name the members twice, so
/// `the_cell_bound_table_agrees_with_the_enumeration` checks they cannot drift.
pub const fn cell_bound(window: u8) -> Option<u16> {
    match window {
        // EWindows::Inventory, EWindows::Equipment, EWindows::BeltInventory
        1 | 2 | 9 => Some(INVENTORY_AND_EQUIP_SLOT_MAX),
        // EWindows::DragonSoulInventory
        5 => Some(DRAGON_SOUL_INVENTORY_MAX_NUM),
        // EWindows::Attr67Add
        6 => Some(ATTR67_ADD_SLOT_MAX),
        // EWindows::Switchbot
        8 => Some(SWITCHBOT_SLOT_COUNT),
        // The named rejections and the `default:` arm: RESERVED_WINDOW 0,
        // SAFEBOX 3, MALL 4, AURA_REFINE 7, GROUND 10, and every unnamed byte.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features;

    /// Every `EWindows` enumerator and the value the C compiler gives it.
    ///
    /// Measured, not counted: three members sit behind feature switches that are
    /// all live in this snapshot, so counting the live members would give
    /// `BELT_INVENTORY` 7 and `GROUND` 8 where the compiler puts them at 9 and
    /// 10. That is not a hypothetical -- the table this replaces did exactly that,
    /// omitting the three gated members and misnumbering both. The probe is
    /// `.scratch/probe194/slot_space.cpp`.
    const PINNED_WINDOWS: [(&str, u8); 11] = [
        ("RESERVED_WINDOW", 0),
        ("INVENTORY", 1),
        ("EQUIPMENT", 2),
        ("SAFEBOX", 3),
        ("MALL", 4),
        ("DRAGON_SOUL_INVENTORY", 5),
        ("ATTR67_ADD", 6),
        ("AURA_REFINE", 7),
        ("SWITCHBOT", 8),
        ("BELT_INVENTORY", 9),
        ("GROUND", 10),
    ];

    #[test]
    fn every_window_byte_matches_the_compiler() {
        for (name, value) in PINNED_WINDOWS {
            let decoded = EWindows::try_from(value)
                .unwrap_or_else(|_| panic!("{name} = {value} is not an EWindows member"));
            assert_eq!(u8::from(decoded), value, "{name}");
        }
    }

    #[test]
    fn every_window_member_is_pinned_and_there_are_no_others() {
        // The count guards against a member being added without a pinned value:
        // a new member would decode from some byte and the loop below would find
        // it, and the array length would no longer agree with the enum's range.
        assert_eq!(PINNED_WINDOWS.len(), EWindows::Ground as usize + 1);
        for value in 0..=(EWindows::Ground as u8) {
            let decoded = EWindows::try_from(value).expect("every byte in range is a member");
            let (_, pinned) = PINNED_WINDOWS[value as usize];
            assert_eq!(decoded as u8, pinned);
        }
    }

    #[test]
    fn the_measured_slot_space_is_the_whole_flat_space() {
        // Each constant against the hand sum the probe re-checks, so a
        // mistranscribed constant fails here rather than shifting every later
        // range silently.
        assert_eq!(INVENTORY_PAGE_SIZE, 5 * 9);
        assert_eq!(INVENTORY_PAGE_COUNT, 4);
        assert_eq!(INVENTORY_MAX_NUM, 5 * 9 * 4);
        assert_eq!(WEAR_MAX_NUM, 64);
        assert_eq!(
            DRAGON_SOUL_EQUIP_SLOT_START,
            INVENTORY_MAX_NUM + WEAR_MAX_NUM
        );
        assert_eq!(
            DRAGON_SOUL_EQUIP_SLOT_END,
            DRAGON_SOUL_EQUIP_SLOT_START + 6 * 2
        );
        assert_eq!(
            DRAGON_SOUL_EQUIP_RESERVED_SLOT_END,
            DRAGON_SOUL_EQUIP_SLOT_END + 6 * 3
        );
        assert_eq!(BELT_INVENTORY_SLOT_COUNT, 4 * 4);
        assert_eq!(BELT_INVENTORY_SLOT_END, BELT_INVENTORY_SLOT_START + 4 * 4);
        assert_eq!(CUSTOM_INVENTORY_MAX_NUM, 45 * 4);
        assert_eq!(
            CUSTOM_INVENTORY_SLOT_END,
            CUSTOM_INVENTORY_SLOT_START + 180 * 6
        );
        assert_eq!(INVENTORY_AND_EQUIP_SLOT_MAX, 1370);
        assert_eq!(DRAGON_SOUL_INVENTORY_MAX_NUM, 6 * 6 * 32);
        assert_eq!(SAFEBOX_MAX_NUM, 45 * 6);
    }

    #[test]
    fn the_six_ranges_are_contiguous_and_end_at_the_space() {
        let ranges = [
            INVENTORY_RANGE,
            EQUIPMENT_RANGE,
            DRAGON_SOUL_EQUIP_RANGE,
            DRAGON_SOUL_RESERVED_RANGE,
            BELT_INVENTORY_RANGE,
            CUSTOM_INVENTORY_RANGE,
        ];
        assert_eq!(ranges[0].start, 0, "the base inventory starts at zero");
        for pair in ranges.windows(2) {
            assert_eq!(
                pair[0].end, pair[1].start,
                "{} ends where {} starts",
                pair[0].end, pair[1].start
            );
        }
        let last = ranges[ranges.len() - 1];
        assert_eq!(last.end, INVENTORY_AND_EQUIP_SLOT_MAX);
        // The belt range is exactly the belt slot count wide, which is the check
        // that the range end was derived rather than typed.
        assert_eq!(BELT_INVENTORY_RANGE.len(), BELT_INVENTORY_SLOT_COUNT);
        // The dragon soul equipment range is the two live decks.
        assert_eq!(
            DRAGON_SOUL_EQUIP_RANGE.len(),
            DS_SLOT_MAX * DRAGON_SOUL_DECK_MAX_NUM
        );
        // Each of the six custom categories is exactly one category wide.
        assert_eq!(CUSTOM_INVENTORY_RANGE.len(), CUSTOM_INVENTORY_MAX_NUM * 6);
    }

    #[test]
    fn without_the_custom_inventory_the_space_ends_at_the_belt() {
        // The `#else` arm of length.h:928-930. The Rewrite has the feature on,
        // so 290 is the value a deployment without it would compile -- which is
        // why it is pinned rather than merely noted.
        assert_eq!(BELT_INVENTORY_SLOT_END, 290);
        assert_ne!(INVENTORY_AND_EQUIP_SLOT_MAX, BELT_INVENTORY_SLOT_END);
    }

    #[test]
    fn every_custom_category_starts_where_the_previous_one_ends() {
        assert_eq!(custom_inventory_start(0), CUSTOM_INVENTORY_SLOT_START);
        for category in 1..CUSTOM_INVENTORY_CATEGORY_NUM {
            assert_eq!(
                custom_inventory_start(category),
                custom_inventory_start(category - 1) + CUSTOM_INVENTORY_MAX_NUM
            );
        }
        assert_eq!(
            custom_inventory_start(CUSTOM_INVENTORY_CATEGORY_NUM),
            CUSTOM_INVENTORY_SLOT_END
        );
    }

    #[test]
    fn a_category_at_or_above_the_count_is_refused() {
        for category in CUSTOM_INVENTORY_CATEGORY_NUM..=u8::MAX {
            assert_eq!(custom_inventory_start_checked(category), None);
        }
        for category in 0..CUSTOM_INVENTORY_CATEGORY_NUM {
            assert_eq!(
                custom_inventory_start_checked(category),
                Some(custom_inventory_start(category))
            );
        }
    }

    #[test]
    fn every_custom_cell_maps_back_to_its_category() {
        for category in 0..CUSTOM_INVENTORY_CATEGORY_NUM {
            let start = custom_inventory_start(category);
            for cell in start..start + CUSTOM_INVENTORY_MAX_NUM {
                assert_eq!(
                    custom_inventory_category(cell),
                    Some(category),
                    "cell {cell}"
                );
            }
        }
        // The last cell of the space and the cell before the space.
        assert_eq!(
            custom_inventory_category(CUSTOM_INVENTORY_SLOT_END - 1),
            Some(5)
        );
        assert_eq!(
            custom_inventory_category(CUSTOM_INVENTORY_SLOT_START - 1),
            None
        );
        assert_eq!(custom_inventory_category(CUSTOM_INVENTORY_SLOT_END), None);
        assert_eq!(custom_inventory_category(0), None);
    }

    #[test]
    fn a_custom_category_spans_every_cell_of_its_own_range() {
        // The loop in GetCustomInventoryCategory must find the same category a
        // division would, including at both ends of every category.
        for category in 0..CUSTOM_INVENTORY_CATEGORY_NUM {
            let start = custom_inventory_start(category);
            for offset in [
                0,
                1,
                44,
                45,
                CUSTOM_INVENTORY_MAX_NUM / 2,
                CUSTOM_INVENTORY_MAX_NUM - 2,
                CUSTOM_INVENTORY_MAX_NUM - 1,
            ] {
                let cell = start + offset;
                assert_eq!(
                    custom_inventory_category(cell),
                    Some(category),
                    "cell {cell}"
                );
            }
        }
    }

    #[test]
    fn the_cell_bound_table_is_the_legacy_switch() {
        assert_eq!(cell_bound(EWindows::Inventory as u8), Some(1370));
        assert_eq!(cell_bound(EWindows::Equipment as u8), Some(1370));
        assert_eq!(cell_bound(EWindows::BeltInventory as u8), Some(1370));
        assert_eq!(cell_bound(EWindows::DragonSoulInventory as u8), Some(1152));
        assert_eq!(cell_bound(EWindows::Attr67Add as u8), Some(1));
        assert_eq!(cell_bound(EWindows::Switchbot as u8), Some(5));
        // The `default: return false` arm, and every named rejection.
        for window in [
            EWindows::ReservedWindow as u8,
            EWindows::Safebox as u8,
            EWindows::Mall as u8,
            EWindows::AuraRefine as u8,
            EWindows::Ground as u8,
        ] {
            assert_eq!(cell_bound(window), None, "window {window}");
        }
    }

    #[test]
    fn a_byte_outside_the_enumeration_has_no_bound() {
        // `default:` in the legacy switch catches every byte the enumerator does
        // not name, so all 256 bytes are covered and the 245 unused ones are not
        // a gap in the table.
        for window in u8::MIN..=u8::MAX {
            let expected = EWindows::try_from(window)
                .ok()
                .and_then(|w| cell_bound(w as u8));
            assert_eq!(cell_bound(window), expected, "byte {window}");
        }
        assert_eq!(cell_bound(11), None);
        assert_eq!(cell_bound(255), None);
    }

    #[test]
    fn the_slots_a_character_owns_are_the_space_and_the_dragon_soul_box() {
        // char.h:458-461. Not a wire width, so it is asserted as arithmetic
        // rather than with size_of: the legacy target is 32-bit x86
        // (premake5.lua:12), where a pointer is 4 bytes and a WORD is 2.
        let pointers_and_words = 4 * usize::from(INVENTORY_AND_EQUIP_SLOT_MAX)
            + 2 * usize::from(INVENTORY_AND_EQUIP_SLOT_MAX)
            + 4 * usize::from(DRAGON_SOUL_INVENTORY_MAX_NUM)
            + 2 * usize::from(DRAGON_SOUL_INVENTORY_MAX_NUM);
        assert_eq!(pointers_and_words, 15132);
    }

    /// Every switch this module's numbers were measured with, and the constants
    /// each one moves.
    ///
    /// Read in a loop rather than as seven `assert!(CONST, ...)` lines because a
    /// bare constant is folded away before the assertion runs, so such a test
    /// would pass whatever the flag said. The loop makes the comparison a real
    /// one, and the failure names both the switch and what has to be redone.
    const GATES: [(&str, bool, &str); 7] = [
        (
            "ENABLE_EXTEND_INVEN_SYSTEM",
            features::ENABLE_EXTEND_INVEN_SYSTEM,
            "INVENTORY_PAGE_COUNT, INVENTORY_MAX_NUM",
        ),
        (
            "ENABLE_CUSTOM_INVENTORY",
            features::ENABLE_CUSTOM_INVENTORY,
            "CUSTOM_INVENTORY_SLOT_END, INVENTORY_AND_EQUIP_SLOT_MAX, cell_bound",
        ),
        (
            "ENABLE_DRAGONSOUL_ALCHEMY_PLUS",
            features::ENABLE_DRAGONSOUL_ALCHEMY_PLUS,
            "DRAGON_SOUL_GRADE_MAX, DRAGON_SOUL_INVENTORY_MAX_NUM",
        ),
        (
            "__EXTENDED_SAFEBOX__",
            features::EXTENDED_SAFEBOX,
            "SAFEBOX_MAX_NUM, SAFEBOX_MAX_PAGE_COUNT",
        ),
        (
            "__ATTR_6TH_7TH__",
            features::ATTR_6TH_7TH,
            "EWindows::Attr67Add",
        ),
        (
            "__AURA_SYSTEM__",
            features::AURA_SYSTEM,
            "EWindows::AuraRefine",
        ),
        (
            "ENABLE_SWITCHBOT",
            features::ENABLE_SWITCHBOT,
            "EWindows::Switchbot, cell_bound",
        ),
    ];

    #[test]
    fn every_measured_number_agrees_with_a_recorded_feature_switch() {
        // The module is only valid for the build its switches describe. If a
        // feature is turned off, this fails and says which constant to redo.
        for (switch, enabled, depends_on) in GATES {
            assert!(
                enabled,
                "{switch} is off, so {depends_on} must be regenerated"
            );
        }
    }

    #[test]
    fn a_gate_is_named_once() {
        let mut names: Vec<&str> = GATES.iter().map(|(switch, _, _)| *switch).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), GATES.len());
    }

    #[test]
    fn a_slot_range_contains_its_start_and_excludes_its_end() {
        let range = BELT_INVENTORY_RANGE;
        assert!(!range.contains(range.start - 1));
        assert!(range.contains(range.start));
        assert!(range.contains(range.end - 1));
        assert!(!range.contains(range.end));
    }
}
