//! The `SItemPos` rules: which position addresses which item, and which do not.
//!
//! `protocol::item_pos::ItemPos` is the 3-byte wire struct. This module is the
//! policy above it: the predicates legacy puts in `SItemPos` itself
//! (`length.h:973-1056`), transcribed one for one. The numbers come from
//! [`common::item_slots`], which measured them from the compiler.
//!
//! # The rules, and the two that surprise
//!
//! Legacy has **two** functions called `IsValidItemPosition` and they disagree,
//! so both are here. [`is_valid_item_position`] is
//! `SItemPos::IsValidItemPosition` (`length.h:973`), a member of the struct, and
//! is the one `exchange.cpp:191,327` and `DragonSoul.cpp:1163` call.
//! [`character_cell_bound`] is `CHARACTER::IsValidItemPosition`
//! (`char_item.cpp:10010-10049`), and is the one `CHARACTER::GetItem` calls
//! (`char_item.cpp:256`). They differ on exactly two windows: the first rejects
//! safebox and mall outright, the second defers them to the live containers.
//!
//! Both reject a dragon soul *equip* or *reserved* cell, because those are only
//! reachable by window byte plus a cell that already names the base inventory or
//! the dragon soul box. Everything a player owns in the flat space is addressed
//! through [`EWindows::Inventory`] or [`EWindows::Equipment`]; the belt and the
//! custom inventory are the same array, not separate windows, and
//! [`stored_window`] is how a cell becomes the byte a row keeps.
//!
//! The second surprise is that [`is_dragon_soul_equip_position`] and
//! [`is_belt_inventory_position`] **ignore the window byte**. Legacy
//! (`length.h:1008-1016`) tests only the cell, so `{Inventory, 250}` is a dragon
//! soul equip position and `{Mall, 280}` is a belt position. That is faithful
//! here rather than a shortcut: a Rewrite that also read the window byte would
//! disagree with legacy on a cell the client can send.
//!
//! The third is [`inventory_type_by_pos`], which folds the base inventory and the
//! six custom categories into one `0..=6` answer and returns `None` for a cell in
//! the equipment, dragon soul, or belt ranges. Legacy spells that `-1` as an
//! `int` (`char_item.cpp:360`).
//!
//! # What is deliberately absent
//!
//! [`GetInventoryPageByPos`](`CHARACTER::GetInventoryPageByPos`) is not
//! transcribed, because it cannot be: it takes a signed category, and unlike all
//! three of its siblings it **never checks the category against
//! `CUSTOM_INVENTORY_CATEGORY_NUM`**, so `iCategory = -1` on a cell at or above
//! `INVENTORY_MAX_NUM` walks a range starting at `CUSTOM_INVENTORY_SLOT_START -
//! CUSTOM_INVENTORY_MAX_NUM` = 110 and can return a page index for a category that
//! does not exist. That is a Defect; see ledger 194. The safe transcription is
//! [`inventory_page_by_pos`], which refuses an out-of-range category and is the
//! one the Rewrite should use.
//!
//! Reading and writing an item out of these positions needs the item instance
//! itself, which is the next ledger unit, so nothing here touches storage.

use common::item_slots::{
    self, cell_bound, custom_inventory_category, custom_inventory_start,
    custom_inventory_start_checked, EWindows, SlotRange, BELT_INVENTORY_RANGE,
    CUSTOM_INVENTORY_SLOT_END, CUSTOM_INVENTORY_SLOT_START, DRAGON_SOUL_EQUIP_RANGE,
    DRAGON_SOUL_RESERVED_RANGE, EQUIPMENT_RANGE, INVENTORY_AND_EQUIP_SLOT_MAX, INVENTORY_MAX_NUM,
    INVENTORY_PAGE_SIZE, INVENTORY_RANGE,
};
use protocol::item_pos::ItemPos;

/// Legacy `TItemPos NPOS` (`length.h:1059`): the reserved window and `WORD_MAX`.
///
/// The second of the two "no position" values. [`INVENTORY_PLACEHOLDER`] is the
/// first: it is what `SItemPos`'s default constructor produces, and it names a
/// real window with an impossible cell. Both fail
/// [`is_valid_item_position`], for different reasons, which is why the
/// constructor lives here and not on the wire type.
pub const NPOS: ItemPos = ItemPos {
    window_type: EWindows::ReservedWindow as u8,
    cell: u16::MAX,
};

/// What `SItemPos::SItemPos()` builds: [`EWindows::Inventory`] with `WORD_MAX`.
///
/// `length.h:961-965`. Note the codec's `ItemPos::default()` is a zero/zero pair
/// instead, on purpose: a default is a Rust convenience, whereas this is legacy
/// behaviour, and the two disagree. The one that matters is that
/// `{Inventory, 0}` is a **valid** position addressing base-inventory cell 0,
/// so a caller that means "empty" must say so rather than fall back to
/// `ItemPos::default()`.
pub const INVENTORY_PLACEHOLDER: ItemPos = ItemPos {
    window_type: EWindows::Inventory as u8,
    cell: u16::MAX,
};

/// The legacy default constructor, [`INVENTORY_PLACEHOLDER`].
pub const fn placeholder() -> ItemPos {
    INVENTORY_PLACEHOLDER
}

/// Legacy `SItemPos::IsValidItemPosition` (`length.h:973-1000`).
///
/// A cell must be below the window's own bound. The bound is
/// [`common::item_slots::cell_bound`], which is the legacy `switch` as a table:
/// inventory, equipment, and belt cells must be below
/// [`INVENTORY_AND_EQUIP_SLOT_MAX`], a dragon soul cell below
/// [`common::item_slots::DRAGON_SOUL_INVENTORY_MAX_NUM`], an attribute slot below one, and a
/// switchbot slot below five. A safebox, mall, aura-refine, ground, or reserved
/// window is never valid, and so is any of the 245 bytes the enumerator does not
/// name.
pub fn is_valid_item_position(pos: ItemPos) -> bool {
    cell_bound(pos.window_type).is_some_and(|bound| pos.cell < bound)
}

/// Legacy `SItemPos::IsEquipPosition` (`length.h:1002-1006`).
///
/// True for a cell in [`EQUIPMENT_RANGE`] reached through the inventory or
/// equipment window, **or** for any cell in [`DRAGON_SOUL_EQUIP_RANGE`],
/// because the second clause is [`is_dragon_soul_equip_position`], which does not
/// look at the window byte.
pub fn is_equip_position(pos: ItemPos) -> bool {
    let is_equip_window = pos.window_type == EWindows::Inventory as u8
        || pos.window_type == EWindows::Equipment as u8;
    (is_equip_window && EQUIPMENT_RANGE.contains(pos.cell)) || is_dragon_soul_equip_position(pos)
}

/// Legacy `SItemPos::IsDragonSoulEquipPosition` (`length.h:1008-1011`).
///
/// **The window byte is not consulted.** Legacy tests the cell alone, so this
/// holds for any window, including one that is never a valid position. That is
/// reproduced, not tightened, because a cell in this range is sent through the
/// inventory window and tightening here would make a position legacy accepts
/// report false.
pub fn is_dragon_soul_equip_position(pos: ItemPos) -> bool {
    DRAGON_SOUL_EQUIP_RANGE.contains(pos.cell)
}

/// Legacy `SItemPos::IsBeltInventoryPosition` (`length.h:1013-1016`).
///
/// **The window byte is not consulted**, for the same reason as
/// [`is_dragon_soul_equip_position`].
///
/// [`EWindows::BeltInventory`] is 9, but the belt cells live in the flat array at
/// [`BELT_INVENTORY_RANGE`], and they are also reachable as
/// `{Inventory, 274}`. Both address the same slots.
pub fn is_belt_inventory_position(pos: ItemPos) -> bool {
    BELT_INVENTORY_RANGE.contains(pos.cell)
}

/// Legacy `SItemPos::IsDefaultInventoryPosition` (`length.h:1018-1021`).
///
/// The base inventory only: the inventory window and a cell below
/// [`INVENTORY_MAX_NUM`]. A custom-inventory cell is *not* in here even though
/// it shares the window byte, which is why
/// [`is_custom_inventory_position`] exists alongside this.
pub fn is_default_inventory_position(pos: ItemPos) -> bool {
    pos.window_type == EWindows::Inventory as u8 && INVENTORY_RANGE.contains(pos.cell)
}

/// Legacy `SItemPos::IsCustomInventoryPosition` (`length.h:1029-1032`).
///
/// The inventory window and a cell at or above [`CUSTOM_INVENTORY_SLOT_START`].
pub fn is_custom_inventory_position(pos: ItemPos) -> bool {
    pos.window_type == EWindows::Inventory as u8
        && (CUSTOM_INVENTORY_SLOT_START..CUSTOM_INVENTORY_SLOT_END).contains(&pos.cell)
}

/// Legacy `SItemPos::IsSwitchbotPosition` (`length.h:1023-1026`).
pub fn is_switchbot_position(pos: ItemPos) -> bool {
    pos.window_type == EWindows::Switchbot as u8 && pos.cell < item_slots::SWITCHBOT_SLOT_COUNT
}

/// Legacy `SItemPos::GetCustomInventoryCategory` (`length.h:1034-1043`).
///
/// The category `cell` falls in, or `None` where legacy returns `-1`. The window
/// byte is not consulted, matching legacy and the rest of the custom-inventory
/// rule.
pub fn custom_inventory_category_of(pos: ItemPos) -> Option<u8> {
    custom_inventory_category(pos.cell)
}

/// Legacy `CHARACTER::GetCustomInventoryItem`'s cell arithmetic
/// (`char_item.cpp:314-316`), without the item lookup.
///
/// The position that names cell `cell` of category `category`, or `None` when the
/// category is at or above [`common::item_slots::CUSTOM_INVENTORY_CATEGORY_NUM`], which is the check
/// `GetCustomInventoryItem` makes at `char_item.cpp:311`.
pub fn custom_inventory_position(category: u8, cell: u16) -> Option<ItemPos> {
    let start = custom_inventory_start_checked(category)?;
    let real_cell = start + cell;
    // The category check bounds `cell` at CUSTOM_INVENTORY_MAX_NUM - 1, so this
    // cannot leave the space. Legacy does not check it, and relies on the same
    // arithmetic, so a cell past the category's end silently addresses the next
    // category; that is recorded rather than fixed, because the client never
    // sends one.
    debug_assert!(real_cell < CUSTOM_INVENTORY_SLOT_END);
    Some(ItemPos::new(EWindows::Inventory as u8, real_cell))
}

/// Legacy `CHARACTER::GetInventoryTypeByPos` (`char_item.cpp:349-361`).
///
/// The base inventory is type 0, custom category `c` is type `c + 1`, so the
/// answer is `0..=6`, and `None` stands for legacy's `-1` for a cell in the
/// equipment, dragon soul, or belt ranges.
pub fn inventory_type_by_pos(pos: ItemPos) -> Option<u8> {
    inventory_type_of_cell(pos.cell)
}

/// [`inventory_type_by_pos`] on the cell alone.
///
/// The window byte is not consulted, because legacy's function takes a `WORD`
/// and has no window byte to consult.
pub fn inventory_type_of_cell(cell: u16) -> Option<u8> {
    if cell < INVENTORY_MAX_NUM {
        return Some(0);
    }
    custom_inventory_category(cell).map(|category| category + 1)
}

/// Legacy `CHARACTER::GetInventoryPageByPos` (`char_item.cpp:334-347`), with the
/// missing bound check added.
///
/// Legacy's first arm maps a base-inventory cell to its page by division. Its
/// second arm walks the four pages of a category -- but never checks the
/// category, so a negative or oversized `iCategory` reads a range outside the
/// space. This refuses a category at or above
/// [`common::item_slots::CUSTOM_INVENTORY_CATEGORY_NUM`] instead, and returns `None` where legacy
/// returns `-1`. The four-page walk is kept rather than a division, so the
/// result matches legacy for every input legacy handled correctly.
pub fn inventory_page_by_pos(category: Option<u8>, pos: ItemPos) -> Option<u8> {
    // A base-inventory cell with no category is the one input legacy's first arm
    // absorbs, and it is why the Defect below only bites above the base inventory.
    if category.is_none() && pos.cell < INVENTORY_MAX_NUM {
        // Legacy returns a BYTE from this int division. The quotient is 0..=3
        // here, so the narrowing cannot truncate; the conversion is checked
        // anyway, which turns "cannot happen" into a refusal rather than a
        // silent wrap. `the_base_inventory_page_is_the_cell_divided_by_the_page_size`
        // pins the range.
        return u8::try_from(pos.cell / INVENTORY_PAGE_SIZE).ok();
    }
    let category = category?;
    let start = custom_inventory_start(category);
    for page in 0..u8::try_from(item_slots::CUSTOM_INVENTORY_PAGE_COUNT).ok()? {
        let page_start = start + u16::from(page) * item_slots::CUSTOM_INVENTORY_PAGE_SIZE;
        if pos.cell >= page_start && pos.cell < page_start + item_slots::CUSTOM_INVENTORY_PAGE_SIZE
        {
            return Some(page);
        }
    }
    None
}

/// The six contiguous ranges of the flat slot space, in order.
///
/// Useful for a test or a debug print that needs to say where a cell lies. The
/// base inventory first, then the equipment, the two dragon soul ranges, the
/// belt, and the custom inventory.
pub const FLAT_RANGES: [SlotRange; 6] = [
    INVENTORY_RANGE,
    EQUIPMENT_RANGE,
    DRAGON_SOUL_EQUIP_RANGE,
    DRAGON_SOUL_RESERVED_RANGE,
    BELT_INVENTORY_RANGE,
    custom_inventory_range(),
];

/// The custom-inventory range.
///
/// A function rather than a constant only because this file already has
/// [`custom_inventory_position`]; the value is [`CUSTOM_INVENTORY_SLOT_START`]
/// to [`CUSTOM_INVENTORY_SLOT_END`].
const fn custom_inventory_range() -> SlotRange {
    SlotRange {
        start: CUSTOM_INVENTORY_SLOT_START,
        end: CUSTOM_INVENTORY_SLOT_END,
    }
}

/// What `CHARACTER::IsValidItemPosition` can say about a window without a
/// character.
///
/// Legacy `char_item.cpp:10010-10049`. This is the check `CHARACTER::GetItem`
/// actually calls (`char_item.cpp:256`), and it is **not** the same check as
/// [`is_valid_item_position`]. They differ on exactly two windows, safebox and
/// mall, and the difference matters:
///
/// * [`SItemPos::IsValidItemPosition`](`length.h:973`) rejects both outright.
/// * `CHARACTER::IsValidItemPosition` defers both to the live safebox or mall.
///
/// So a safebox position is refused by the first and admitted by the second.
/// [`exchange.cpp:191,327`](`server/server/game/exchange.cpp`) and
/// [`DragonSoul.cpp:1163`](`server/server/game/DragonSoul.cpp`) use the first, so
/// a trade position naming a safebox window is refused, while `GetItem` would
/// have looked inside. Both are transcribed, because picking one would silently
/// change which of them a caller gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellBound {
    /// The window's cells run `0..bound` and nothing above is valid.
    Bounded(u16),
    /// The bound is a runtime property of a container this character may not
    /// have. Legacy calls `m_pkSafebox->IsValidPosition(cell)`, which returns
    /// false when there is no safebox and otherwise compares against the live
    /// grid size (`safebox.cpp:251-260`). It cannot be a constant.
    Deferred,
    /// The window is never a valid item position on a character.
    Never,
}

impl CellBound {
    /// Whether a `cell` is in range, given the safebox or mall size when the
    /// bound is [`CellBound::Deferred`].
    ///
    /// `container_cells` is the live grid size, or `None` when the character has
    /// no such container -- which legacy treats as invalid rather than as an
    /// error, so it is `None` here too and not a refusal.
    pub const fn accepts(self, cell: u16, container_cells: Option<u16>) -> bool {
        match self {
            Self::Bounded(bound) => cell < bound,
            Self::Deferred => match container_cells {
                Some(size) => cell < size,
                None => false,
            },
            Self::Never => false,
        }
    }
}

/// Legacy `CHARACTER::IsValidItemPosition` (`char_item.cpp:10010-10049`), as far
/// as it can be decided from the window byte alone.
///
/// Note the three windows this rejects that [`is_valid_item_position`] also
/// rejects, for the same reason -- the legacy `default: return false` arm --
/// plus the two it defers. [`EWindows::BeltInventory`] is among them: the belt
/// cells are reached through [`EWindows::Inventory`], and the belt window byte
/// names no cell at all.
pub const fn character_cell_bound(window: u8) -> CellBound {
    match window {
        1 | 2 => CellBound::Bounded(INVENTORY_AND_EQUIP_SLOT_MAX),
        5 => CellBound::Bounded(common::item_slots::DRAGON_SOUL_INVENTORY_MAX_NUM),
        6 => CellBound::Bounded(common::item_slots::ATTR67_ADD_SLOT_MAX),
        8 => CellBound::Bounded(common::item_slots::SWITCHBOT_SLOT_COUNT),
        3 | 4 => CellBound::Deferred,
        _ => CellBound::Never,
    }
}

/// The window byte `CHARACTER::SetCell` stores an item under
/// (`char_item.cpp:617-631`).
///
/// This is a normalisation, not a copy. An item whose cell is in the base
/// inventory, the belt band, or a custom category is stored as
/// [`EWindows::Inventory`]; everything else in the flat space -- the equipment,
/// dragon soul, and reserved bands -- is stored as [`EWindows::Equipment`]. A
/// dragon soul cell is stored as [`EWindows::DragonSoulInventory`], an attribute
/// slot as [`EWindows::Attr67Add`], and a switchbot slot as
/// [`EWindows::Switchbot`].
///
/// It matters because the byte a client sends and the byte a row holds are
/// different fields. A client may address the equipment as
/// `{Equipment, 200}`; the row says `EQUIPMENT` for that, but `{Inventory, 200}`
/// for a dragon soul deck cell, because the band decides and the byte does not.
pub fn stored_window(window_type: u8, cell: u16) -> EWindows {
    match EWindows::try_from(window_type) {
        Ok(EWindows::DragonSoulInventory) => EWindows::DragonSoulInventory,
        Ok(EWindows::Attr67Add) => EWindows::Attr67Add,
        Ok(EWindows::Switchbot) => EWindows::Switchbot,
        Ok(EWindows::Inventory | EWindows::Equipment) => {
            if INVENTORY_RANGE.contains(cell)
                || BELT_INVENTORY_RANGE.contains(cell)
                || (CUSTOM_INVENTORY_SLOT_START..CUSTOM_INVENTORY_SLOT_END).contains(&cell)
            {
                EWindows::Inventory
            } else {
                EWindows::Equipment
            }
        }
        _ => EWindows::ReservedWindow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::item_slots::{
        usable_inventory_cells, ATTR67_ADD_SLOT_MAX, CUSTOM_INVENTORY_CATEGORY_NUM,
        CUSTOM_INVENTORY_MAX_NUM, DRAGON_SOUL_EQUIP_SLOT_START, DRAGON_SOUL_INVENTORY_MAX_NUM,
        DRAGON_SOUL_INVENTORY_RANGE, INVENTORY_AND_EQUIP_SLOT_MAX, INVENTORY_OPEN_PAGE_SIZE,
        INVENTORY_WIDTH, SAFEBOX_MAX_NUM, SWITCHBOT_SLOT_COUNT, WEAR_MAX_NUM,
    };

    const INV: u8 = EWindows::Inventory as u8;
    const EQUIP: u8 = EWindows::Equipment as u8;
    const SAFEBOX: u8 = EWindows::Safebox as u8;
    const MALL: u8 = EWindows::Mall as u8;
    const DS: u8 = EWindows::DragonSoulInventory as u8;
    const AURA: u8 = EWindows::AuraRefine as u8;
    const SWITCH: u8 = EWindows::Switchbot as u8;
    const BELT: u8 = EWindows::BeltInventory as u8;
    const GROUND: u8 = EWindows::Ground as u8;
    const RESERVED: u8 = EWindows::ReservedWindow as u8;

    #[test]
    fn the_placeholder_and_npos_both_fail_the_legacy_check() {
        // Two different "no position" values, for two different reasons: the
        // constructor's names a real window with an impossible cell, and NPOS
        // names the reserved window.
        assert!(!is_valid_item_position(INVENTORY_PLACEHOLDER));
        assert!(!is_valid_item_position(NPOS));
        assert_eq!(placeholder(), INVENTORY_PLACEHOLDER);
        assert_eq!(NPOS.window_type, RESERVED);
        assert_eq!(NPOS.cell, u16::MAX);
    }

    #[test]
    fn the_codec_default_is_a_valid_position_and_placeholder_is_not() {
        // The reason the legacy constructor is not the codec's Default: a
        // zero/zero ItemPos is `{Reserved, 0}`, which is invalid, but
        // `{Inventory, 0}` -- one byte different -- is the base inventory's
        // first cell and is perfectly valid.
        assert!(!is_valid_item_position(ItemPos::new(RESERVED, 0)));
        assert!(is_valid_item_position(ItemPos::new(INV, 0)));
        assert!(!is_valid_item_position(ItemPos::new(INV, u16::MAX)));
    }

    #[test]
    fn the_validity_rule_is_the_cell_below_the_windows_bound() {
        // The three windows that share the flat space, at and just past the bound.
        for window in [INV, EQUIP, BELT] {
            assert!(is_valid_item_position(ItemPos::new(window, 0)));
            assert!(is_valid_item_position(ItemPos::new(
                window,
                INVENTORY_AND_EQUIP_SLOT_MAX - 1
            )));
            assert!(!is_valid_item_position(ItemPos::new(
                window,
                INVENTORY_AND_EQUIP_SLOT_MAX
            )));
            assert!(!is_valid_item_position(ItemPos::new(window, u16::MAX)));
        }
        assert!(is_valid_item_position(ItemPos::new(
            DS,
            DRAGON_SOUL_INVENTORY_MAX_NUM - 1
        )));
        assert!(!is_valid_item_position(ItemPos::new(
            DS,
            DRAGON_SOUL_INVENTORY_MAX_NUM
        )));
        assert!(is_valid_item_position(ItemPos::new(
            SWITCH,
            SWITCHBOT_SLOT_COUNT - 1
        )));
        assert!(!is_valid_item_position(ItemPos::new(
            SWITCH,
            SWITCHBOT_SLOT_COUNT
        )));
        assert!(is_valid_item_position(ItemPos::new(
            EWindows::Attr67Add as u8,
            ATTR67_ADD_SLOT_MAX - 1
        )));
        assert!(!is_valid_item_position(ItemPos::new(
            EWindows::Attr67Add as u8,
            ATTR67_ADD_SLOT_MAX
        )));
    }

    #[test]
    fn a_safebox_or_mall_position_is_never_valid_however_small_the_cell() {
        // length.h:985-987 rejects both outright. The safebox has SAFEBOX_MAX_NUM
        // pages of its own and is not addressed by a TItemPos at all, so a
        // Rewrite that let these through would be inventing an addressing mode
        // the client has no way to have learned.
        for window in [SAFEBOX, MALL, AURA, GROUND, RESERVED] {
            for cell in [0, 1, 44, 179, 180, 273, 289, 290, 1369] {
                assert!(
                    !is_valid_item_position(ItemPos::new(window, cell)),
                    "window {window} cell {cell}"
                );
            }
        }
        // Including cells the safebox would otherwise accept, which is the point:
        // the bound is not what refuses these, the window is.
        assert_eq!(SAFEBOX_MAX_NUM, 270);
        assert!(!is_valid_item_position(ItemPos::new(
            SAFEBOX,
            SAFEBOX_MAX_NUM - 1
        )));
    }

    #[test]
    fn an_unnamed_window_byte_is_never_valid() {
        for byte in [11u8, 12, 100, 200, 255] {
            for cell in [0u16, 1, 179, 289, 1369] {
                assert!(
                    !is_valid_item_position(ItemPos::new(byte, cell)),
                    "byte {byte} cell {cell}"
                );
            }
        }
    }

    #[test]
    fn the_validity_rule_agrees_with_the_bound_table_for_the_whole_cell_space() {
        // Exhaustive over every cell the flat space holds, for every window that
        // can hold one. This is the property the two rules must share, and it is
        // cheap enough to state exhaustively.
        for window in [INV, EQUIP, BELT] {
            for cell in 0..INVENTORY_AND_EQUIP_SLOT_MAX {
                assert!(
                    is_valid_item_position(ItemPos::new(window, cell)),
                    "{window}/{cell}"
                );
            }
            for cell in INVENTORY_AND_EQUIP_SLOT_MAX..=u16::MAX {
                assert!(
                    !is_valid_item_position(ItemPos::new(window, cell)),
                    "{window}/{cell}"
                );
            }
        }
    }

    #[test]
    fn the_equipment_rule_keeps_legacys_ignored_window_byte() {
        // length.h:1008-1011 tests the cell alone. Faithfulness is the point: a
        // Rewrite that also read the window byte would disagree on a cell the
        // client can send.
        for window in [0u8, 1, 2, 3, 4, 7, 9, 10, 200] {
            assert!(is_dragon_soul_equip_position(ItemPos::new(window, 244)));
            assert!(is_dragon_soul_equip_position(ItemPos::new(window, 255)));
            assert!(!is_dragon_soul_equip_position(ItemPos::new(window, 243)));
            assert!(!is_dragon_soul_equip_position(ItemPos::new(window, 256)));
        }
    }

    #[test]
    fn the_belt_rule_keeps_legacys_ignored_window_byte_too() {
        for window in [0u8, 1, 2, 3, 4, 7, 9, 10, 255] {
            assert!(is_belt_inventory_position(ItemPos::new(window, 274)));
            assert!(is_belt_inventory_position(ItemPos::new(window, 289)));
            assert!(!is_belt_inventory_position(ItemPos::new(window, 273)));
            assert!(!is_belt_inventory_position(ItemPos::new(window, 290)));
        }
    }

    #[test]
    fn the_belt_cells_are_the_same_slots_reached_two_ways() {
        // The belt window byte and the inventory window byte name the same array
        // at the same cells, so the positions are equal even though the bytes
        // differ. That is why the belt needs no second array.
        for cell in 274..290 {
            let by_belt = ItemPos::new(BELT, cell);
            let by_inventory = ItemPos::new(INV, cell);
            assert_ne!(by_belt, by_inventory, "the bytes differ");
            assert!(is_valid_item_position(by_belt));
            assert!(is_valid_item_position(by_inventory));
            assert!(is_belt_inventory_position(by_belt));
            assert!(is_belt_inventory_position(by_inventory));
        }
    }

    #[test]
    fn the_equipment_window_and_the_inventory_window_agree_on_the_same_cells() {
        for cell in INVENTORY_MAX_NUM..DRAGON_SOUL_EQUIP_SLOT_START {
            let by_inventory = ItemPos::new(INV, cell);
            let by_equipment = ItemPos::new(EQUIP, cell);
            assert!(is_equip_position(by_inventory));
            assert!(is_equip_position(by_equipment));
            assert!(is_valid_item_position(by_inventory));
            assert!(is_valid_item_position(by_equipment));
            // Neither is the base inventory, and neither is the custom inventory.
            assert!(!is_default_inventory_position(by_inventory));
            assert!(!is_custom_inventory_position(by_inventory));
        }
    }

    #[test]
    fn the_base_inventory_and_the_custom_inventory_share_the_window_byte() {
        // A base cell is a default-inventory position and not a custom one; a
        // custom cell is the reverse. Both are valid and both are inventory
        // cells, which is the whole reason the two rules exist separately.
        for cell in 0..INVENTORY_MAX_NUM {
            let pos = ItemPos::new(INV, cell);
            assert!(is_default_inventory_position(pos), "cell {cell}");
            assert!(!is_custom_inventory_position(pos), "cell {cell}");
            assert!(is_valid_item_position(pos));
        }
        for cell in CUSTOM_INVENTORY_SLOT_START..CUSTOM_INVENTORY_SLOT_END {
            let pos = ItemPos::new(INV, cell);
            assert!(is_custom_inventory_position(pos), "cell {cell}");
            assert!(!is_default_inventory_position(pos), "cell {cell}");
            assert!(is_valid_item_position(pos));
        }
        // The gap between the base inventory and the custom inventory is the
        // equipment and dragon soul and belt ranges, which are none of the three.
        for cell in INVENTORY_MAX_NUM..CUSTOM_INVENTORY_SLOT_START {
            let pos = ItemPos::new(INV, cell);
            assert!(!is_default_inventory_position(pos), "cell {cell}");
            assert!(!is_custom_inventory_position(pos), "cell {cell}");
        }
    }

    #[test]
    fn the_custom_inventory_starts_after_the_belt_and_ends_at_the_space() {
        assert_eq!(CUSTOM_INVENTORY_SLOT_START, BELT_INVENTORY_RANGE.end);
        assert_eq!(CUSTOM_INVENTORY_SLOT_END, INVENTORY_AND_EQUIP_SLOT_MAX);
    }

    #[test]
    fn a_custom_position_is_built_from_its_category_and_cell() {
        for category in 0..CUSTOM_INVENTORY_CATEGORY_NUM {
            for cell in [0u16, 1, 44, 45, 89, 179] {
                let pos = custom_inventory_position(category, cell).expect("a valid category");
                assert_eq!(pos.window_type, INV);
                assert_eq!(pos.cell, custom_inventory_start(category) + cell);
                assert!(is_custom_inventory_position(pos));
                assert!(is_valid_item_position(pos));
                assert_eq!(custom_inventory_category_of(pos), Some(category));
            }
        }
    }

    #[test]
    fn a_custom_position_round_trips_through_its_category() {
        for cell in CUSTOM_INVENTORY_SLOT_START..CUSTOM_INVENTORY_SLOT_END {
            let pos = ItemPos::new(INV, cell);
            let category = custom_inventory_category_of(pos).expect("a custom cell has a category");
            let offset = cell - custom_inventory_start(category);
            assert_eq!(custom_inventory_position(category, offset), Some(pos));
        }
    }

    #[test]
    fn a_category_at_or_above_the_count_builds_nothing() {
        for category in CUSTOM_INVENTORY_CATEGORY_NUM..=u8::MAX {
            assert_eq!(
                custom_inventory_position(category, 0),
                None,
                "category {category}"
            );
        }
        assert_eq!(
            custom_inventory_position(0, 0).map(|p| p.cell),
            Some(CUSTOM_INVENTORY_SLOT_START)
        );
    }

    #[test]
    fn the_inventory_type_folds_the_base_inventory_and_the_six_categories() {
        assert_eq!(inventory_type_of_cell(0), Some(0));
        assert_eq!(inventory_type_of_cell(INVENTORY_MAX_NUM - 1), Some(0));
        // The equipment, dragon soul, and belt ranges are no inventory type.
        for cell in INVENTORY_MAX_NUM..CUSTOM_INVENTORY_SLOT_START {
            assert_eq!(inventory_type_of_cell(cell), None, "cell {cell}");
        }
        for category in 0..CUSTOM_INVENTORY_CATEGORY_NUM {
            let start = custom_inventory_start(category);
            assert_eq!(inventory_type_of_cell(start), Some(category + 1));
            assert_eq!(
                inventory_type_of_cell(start + CUSTOM_INVENTORY_MAX_NUM - 1),
                Some(category + 1)
            );
        }
        // Any answer is 0..=6 and nothing else, and the cells with no answer are
        // exactly the equipment, dragon soul, and reserved bands. Checked as a
        // partition of the whole space rather than one loop, because the gap
        // between the base inventory and the custom inventory is the point.
        let with_type = (0..INVENTORY_AND_EQUIP_SLOT_MAX)
            .filter(|cell| inventory_type_of_cell(*cell).is_some())
            .count();
        let expected = usize::from(INVENTORY_MAX_NUM)
            + usize::from(CUSTOM_INVENTORY_CATEGORY_NUM) * usize::from(CUSTOM_INVENTORY_MAX_NUM);
        assert_eq!(with_type, expected, "180 base plus 6 categories of 180");
        for cell in 0..INVENTORY_AND_EQUIP_SLOT_MAX {
            if let Some(kind) = inventory_type_of_cell(cell) {
                assert!(
                    kind <= CUSTOM_INVENTORY_CATEGORY_NUM,
                    "cell {cell} type {kind}"
                );
            }
        }
    }

    #[test]
    fn the_base_inventory_page_is_the_cell_divided_by_the_page_size() {
        for page in 0..4u8 {
            let start = u16::from(page) * INVENTORY_PAGE_SIZE;
            for offset in 0..INVENTORY_PAGE_SIZE {
                assert_eq!(
                    inventory_page_by_pos(None, ItemPos::new(INV, start + offset)),
                    Some(page),
                    "cell {}",
                    start + offset
                );
            }
        }
        assert_eq!(
            inventory_page_by_pos(None, ItemPos::new(INV, INVENTORY_MAX_NUM)),
            None
        );
    }

    #[test]
    fn a_custom_page_comes_from_the_category() {
        for category in 0..CUSTOM_INVENTORY_CATEGORY_NUM {
            let start = custom_inventory_start(category);
            for page in 0..4u8 {
                let page_start = start + u16::from(page) * item_slots::CUSTOM_INVENTORY_PAGE_SIZE;
                for offset in [0u16, 1, item_slots::CUSTOM_INVENTORY_PAGE_SIZE - 1] {
                    assert_eq!(
                        inventory_page_by_pos(
                            Some(category),
                            ItemPos::new(INV, page_start + offset)
                        ),
                        Some(page),
                        "category {category} page {page}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_missing_bound_check_would_have_read_outside_the_space() {
        // The Defect this module records. Legacy takes a *signed* category and
        // never checks it against CUSTOM_INVENTORY_CATEGORY_NUM, so category -1
        // starts its page walk at CUSTOM_INVENTORY_SLOT_START -
        // CUSTOM_INVENTORY_MAX_NUM = 110, which is inside the base inventory.
        // The arithmetic is done in i32 here because the category is signed in
        // legacy, and 110 does not fit the "custom cell" reading at all.
        let legacy_start =
            i32::from(CUSTOM_INVENTORY_SLOT_START) - i32::from(CUSTOM_INVENTORY_MAX_NUM);
        assert_eq!(
            legacy_start, 110,
            "legacy category -1 starts its walk inside the base"
        );

        // A base-inventory cell is answered by the first arm, exactly as legacy.
        assert_eq!(inventory_page_by_pos(None, ItemPos::new(INV, 110)), Some(2));

        // An equipment cell is not, so legacy falls into the unchecked walk.
        // Its pages would start at 110, 155, 200, 245, so 200 is page 2.
        let cell = 200u16;
        assert!(!INVENTORY_RANGE.contains(cell), "200 is an equipment cell");
        let legacy_page_start =
            legacy_start + 2 * i32::from(item_slots::CUSTOM_INVENTORY_PAGE_SIZE);
        assert_eq!(legacy_page_start, i32::from(cell));
        assert!(
            i32::from(cell) < legacy_page_start + i32::from(item_slots::CUSTOM_INVENTORY_PAGE_SIZE)
        );
        // What the Rewrite returns instead, and the cell legacy would have named.
        assert_eq!(inventory_page_by_pos(None, ItemPos::new(INV, cell)), None);
        assert_eq!(custom_inventory_category_of(ItemPos::new(INV, cell)), None);
    }

    #[test]
    fn the_flat_ranges_cover_the_space_once_and_in_order() {
        assert_eq!(FLAT_RANGES[0].start, 0);
        for pair in FLAT_RANGES.windows(2) {
            assert_eq!(
                pair[0].end, pair[1].start,
                "gap or overlap at {}",
                pair[0].end
            );
        }
        assert_eq!(
            FLAT_RANGES[FLAT_RANGES.len() - 1].end,
            INVENTORY_AND_EQUIP_SLOT_MAX
        );
        let total: u32 = FLAT_RANGES.iter().map(|r| u32::from(r.len())).sum();
        assert_eq!(total, u32::from(INVENTORY_AND_EQUIP_SLOT_MAX));
    }

    #[test]
    fn the_two_legacy_validity_functions_differ_on_exactly_two_windows() {
        // `SItemPos::IsValidItemPosition` refuses a safebox or a mall outright;
        // `CHARACTER::IsValidItemPosition` defers them to the live container.
        // Every other window must agree, or one of the two is mistranscribed.
        for window in 0u8..=255 {
            let mine = is_valid_item_position(ItemPos::new(window, 0));
            let theirs = match character_cell_bound(window) {
                CellBound::Bounded(bound) => 0 < bound,
                // Cell 0 with a live container is accepted, which is the point:
                // the second check admits what the first refuses.
                CellBound::Deferred => true,
                CellBound::Never => false,
            };
            match EWindows::try_from(window) {
                // The two the SItemPos check refuses and the CHARACTER check defers.
                Ok(EWindows::Safebox | EWindows::Mall) => {
                    assert!(!mine, "the SItemPos check refuses window {window}");
                    assert!(theirs, "the CHARACTER check defers window {window}");
                }
                // The third, and the one that is easy to miss: the SItemPos switch
                // has `case BELT_INVENTORY: return cell < INVENTORY_AND_EQUIP_SLOT_MAX`
                // at length.h:981-982, but CHARACTER::IsValidItemPosition has no
                // belt case at all, so it reaches `default: return false`. A belt
                // position is therefore accepted by exchange.cpp and
                // DragonSoul.cpp and refused by GetItem.
                Ok(EWindows::BeltInventory) => {
                    assert!(mine, "the SItemPos check accepts window {window}");
                    assert!(!theirs, "the CHARACTER check refuses window {window}");
                }
                _ => assert_eq!(mine, theirs, "window {window} must agree"),
            }
        }
    }

    #[test]
    fn a_deferred_bound_is_false_without_a_container() {
        // CSafebox::IsValidPosition returns false when there is no grid, and
        // otherwise compares against the live size (safebox.cpp:251-260). The
        // Rewrite has no safebox yet, so only the false half is reachable, and it
        // is the half that keeps a character with no safebox from reading one.
        for window in [SAFEBOX, MALL] {
            assert_eq!(character_cell_bound(window), CellBound::Deferred);
            assert!(!CellBound::Deferred.accepts(0, None));
            assert!(CellBound::Deferred.accepts(0, Some(1)));
            assert!(!CellBound::Deferred.accepts(1, Some(1)));
            assert!(CellBound::Deferred.accepts(269, Some(270)));
            assert!(!CellBound::Deferred.accepts(270, Some(270)));
            // The SItemPos check refuses the same positions outright.
            assert!(!is_valid_item_position(ItemPos::new(window, 0)));
        }
    }

    #[test]
    fn a_never_bound_is_false_however_large_the_cell() {
        for window in [RESERVED, AURA, BELT, GROUND, 11, 255] {
            assert_eq!(
                character_cell_bound(window),
                CellBound::Never,
                "window {window}"
            );
            for cell in [0u16, 1, 179, 1151, 1369, u16::MAX] {
                assert!(
                    !CellBound::Never.accepts(cell, Some(270)),
                    "window {window} cell {cell}"
                );
            }
        }
        // The belt window byte names no cell even though the belt band is in the
        // flat space and is reached through the inventory byte instead.
        assert_eq!(character_cell_bound(BELT), CellBound::Never);
        assert!(CellBound::Bounded(INVENTORY_AND_EQUIP_SLOT_MAX)
            .accepts(BELT_INVENTORY_RANGE.start, None));
    }

    #[test]
    fn a_bounded_window_accepts_up_to_its_bound_and_no_further() {
        let cases = [
            (INV, INVENTORY_AND_EQUIP_SLOT_MAX),
            (EQUIP, INVENTORY_AND_EQUIP_SLOT_MAX),
            (DS, DRAGON_SOUL_INVENTORY_MAX_NUM),
            (SWITCH, SWITCHBOT_SLOT_COUNT),
            (EWindows::Attr67Add as u8, ATTR67_ADD_SLOT_MAX),
        ];
        for (window, bound) in cases {
            assert_eq!(
                character_cell_bound(window),
                CellBound::Bounded(bound),
                "window {window}"
            );
            let accepts = |cell: u16| CellBound::Bounded(bound).accepts(cell, None);
            assert!(accepts(0), "window {window} cell 0");
            assert!(accepts(bound - 1), "window {window} cell {}", bound - 1);
            assert!(!accepts(bound), "window {window} cell {bound}");
            assert!(!accepts(u16::MAX), "window {window} cell 65535");
        }
    }

    #[test]
    fn the_stored_window_is_decided_by_the_band_not_by_the_byte() {
        // Base inventory, belt, and the custom categories are stored INVENTORY.
        for cell in 0..INVENTORY_MAX_NUM {
            assert_eq!(stored_window(INV, cell), EWindows::Inventory, "cell {cell}");
            // The equipment byte naming the same cell stores the same thing.
            assert_eq!(
                stored_window(EQUIP, cell),
                EWindows::Inventory,
                "cell {cell}"
            );
        }
        for cell in BELT_INVENTORY_RANGE.start..BELT_INVENTORY_RANGE.end {
            assert_eq!(
                stored_window(INV, cell),
                EWindows::Inventory,
                "belt cell {cell}"
            );
        }
        for cell in CUSTOM_INVENTORY_SLOT_START..CUSTOM_INVENTORY_SLOT_END {
            assert_eq!(
                stored_window(INV, cell),
                EWindows::Inventory,
                "custom cell {cell}"
            );
        }
        // Everything else in the flat space is stored EQUIPMENT: the equipment
        // band and both dragon soul bands.
        for cell in INVENTORY_MAX_NUM..CUSTOM_INVENTORY_SLOT_START {
            if BELT_INVENTORY_RANGE.contains(cell) {
                continue;
            }
            assert_eq!(stored_window(INV, cell), EWindows::Equipment, "cell {cell}");
        }
        // The bands that keep their own byte.
        assert_eq!(stored_window(DS, 0), EWindows::DragonSoulInventory);
        assert_eq!(
            stored_window(EWindows::Attr67Add as u8, 0),
            EWindows::Attr67Add
        );
        assert_eq!(stored_window(SWITCH, 0), EWindows::Switchbot);
        // A window with no storage of its own stores nothing.
        for window in [SAFEBOX, MALL, AURA, GROUND, BELT, RESERVED, 11, 255] {
            assert_eq!(
                stored_window(window, 0),
                EWindows::ReservedWindow,
                "window {window}"
            );
        }
    }

    #[test]
    fn the_stored_window_is_what_makes_a_row_readable_again() {
        // A row stores a window byte and a cell. Reading it back has to land on
        // the same slot, so for every flat cell the stored byte must be accepted
        // by the validity check and must still contain the cell.
        for cell in 0..INVENTORY_AND_EQUIP_SLOT_MAX {
            let window = stored_window(INV, cell);
            let pos = ItemPos::new(window as u8, cell);
            assert!(
                is_valid_item_position(pos),
                "cell {cell} stored as {window:?}"
            );
            // And the byte the client may send for the same cell is also valid.
            assert!(
                is_valid_item_position(ItemPos::new(INV, cell)),
                "cell {cell}"
            );
        }
        // The belt is the case that makes the normalisation visible: a belt item
        // is stored INVENTORY, so BELT_INVENTORY never appears in a row.
        assert_eq!(
            stored_window(INV, BELT_INVENTORY_RANGE.start),
            EWindows::Inventory
        );
        assert_eq!(
            character_cell_bound(EWindows::BeltInventory as u8),
            CellBound::Never
        );
    }

    #[test]
    fn the_usable_inventory_count_is_a_stat_and_not_the_array_length() {
        // char.h:1285. The array is always 180 cells in the base inventory; the
        // usable count starts at two pages and grows five cells per stat point.
        assert_eq!(usable_inventory_cells(0), 90);
        assert_eq!(usable_inventory_cells(1), 95);
        assert_eq!(usable_inventory_cells(18), 180);
        for stat in 0..=18u16 {
            let usable = usable_inventory_cells(stat);
            assert!(usable <= INVENTORY_MAX_NUM, "stat {stat} gives {usable}");
            assert_eq!(
                usable % INVENTORY_WIDTH,
                INVENTORY_OPEN_PAGE_SIZE % INVENTORY_WIDTH
            );
        }
        // A cell past the usable count is inside the array and still passes the
        // bound check, which is why the two numbers have to be kept apart.
        let usable = usable_inventory_cells(0);
        assert!(usable < INVENTORY_MAX_NUM);
        assert!(is_valid_item_position(ItemPos::new(INV, usable)));
    }

    #[test]
    fn the_counts_agree_with_the_slot_space() {
        assert_eq!(INVENTORY_AND_EQUIP_SLOT_MAX, 1370);
        assert_eq!(WEAR_MAX_NUM, 64);
        assert_eq!(CUSTOM_INVENTORY_MAX_NUM, 180);
        assert_eq!(DRAGON_SOUL_INVENTORY_RANGE.len(), 1152);
    }
}
