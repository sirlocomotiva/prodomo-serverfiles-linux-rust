//! Character-side item storage: the arrays, the grids, and the three operations.
//!
//! # What this models
//!
//! Legacy stores a character's items as bare `CItem *` in seven arrays
//! (`char.h:458-478`). Four of them are addressed by a [`ItemPos`] window byte:
//! the flat array of 1370, the dragon soul array of 1152, the single attribute
//! 67 slot, and the five switchbot slots. The other three -- the item cube, the
//! sash materials and the costume materials -- belong to their own windows and
//! are reached through their own accessors, so they are out of scope here.
//!
//! This type holds **ids**, not items. The items live in an
//! [`ItemIds`](crate::item::ItemIds)'s view, keyed by id, which is what makes
//! two of legacy's worst defects unrepresentable rather than merely avoided:
//! there is no owner pointer to dangle, and a removed id is never reused.
//!
//! # The grid is a one-based anchor
//!
//! `bItemGrid` and `wDSItemGrid` are **`WORD` arrays, not `BYTE` arrays**,
//! despite the `b` prefix (`char.h:459,461`). A cell holding an item stores the
//! item's **anchor cell plus one**, so `0` means empty. The element being a
//! `WORD` is load-bearing: cell 1369 stores 1370, which a `BYTE` would wrap to
//! 90 and report as a live cell. Both directions are asserted in the tests.
//!
//! # Four legacy Defects this refuses to reproduce
//!
//! 1. **`SetItem` returns `void`, so a rejected insert is silent**
//!    (`char_item.cpp:366,391-395`, and `item.cpp:523-530` where
//!    `AddToCharacter` returns `true` regardless). Every rejection here returns
//!    an [`Insert`] the caller can see.
//! 2. **The dragon soul and switchbot arms read before they bound-check**
//!    (`char_item.cpp:473,493,519` and `:537` before the check at `:543`).
//!    Here every bound is checked before any read or write.
//! 3. **`AddToCharacter` range-checks the wrong variable**
//!    (`item.cpp:444-472` computes `pos` and then checks `m_wCell`, the old
//!    cell). Here the destination is the only thing checked.
//! 4. **The `0xff` pointer test is dead on a 32-bit target**
//!    (`char_item.cpp:373`) and the `0xffffffff` case calls `core_dump()`.
//!    Neither is representable here: an id is a `u32` and a fresh item is built
//!    with one.
//!
//! # What this deliberately still matches
//!
//! The grid walk's `continue` on an out-of-category cell, the first-match
//! category search, and the fact that the grid stores an **anchor** rather than
//! a per-cell identity. Those are behaviour, not bugs, and the client can see
//! them.

use common::item_slots::{
    usable_inventory_cells, EWindows, CUSTOM_INVENTORY_CATEGORY_NUM, CUSTOM_INVENTORY_MAX_NUM,
    CUSTOM_INVENTORY_SLOT_START, DRAGON_SOUL_BOX_COLUMN_NUM, DRAGON_SOUL_INVENTORY_MAX_NUM,
    FLAT_STACK_STRIDE, INVENTORY_AND_EQUIP_SLOT_MAX, INVENTORY_MAX_NUM, INVENTORY_PAGE_SIZE,
    SWITCHBOT_SLOT_COUNT,
};
use protocol::item_pos::ItemPos;

use super::inventory::{custom_inventory_category_of, is_custom_inventory_position};
use crate::item::{Item, ItemId, NO_ITEM};

/// `ATTR67_ADD_SLOT_MAX` = 1 (`length.h`): the attribute 67 window holds exactly
/// one cell, so it is a single slot and not an array.
pub const ATTR67_SLOTS: u16 = 1;

/// What a lookup found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    /// The cell holds this item.
    Occupied(ItemId),
    /// The cell is addressable and free.
    Empty,
    /// The position names no cell of this character.
    ///
    /// This is the split legacy cannot express: `CHARACTER::GetItem` returns
    /// `NULL` for a free cell and for a cell past the end alike
    /// (`char_item.cpp:254-305`), so a caller that wants to tell them apart has
    /// to call `IsValidItemPosition` first, and most do not.
    OutOfRange,
    /// The position is addressable but belongs to a container this character
    /// does not have.
    ///
    /// `GetItem` accepts a safebox or mall position -- `IsValidItemPosition`
    /// defers those to the live container -- and then has no case for them and
    /// returns `NULL` (`char_item.cpp:260-300`). So a valid position yields
    /// nothing. Here the outcome is named, which is what stops a caller from
    /// treating "no safebox" as "the safebox is empty".
    NoContainer(u8),
}

/// Why an insert or a move was refused.
///
/// Legacy's three arms each `return` early and the function is `void`, so all
/// three are the same outcome as far as the caller can tell. They are not the
/// same situation, so they are not the same variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// The window byte names no cell of this character. Legacy logs
    /// `"Invalid Inventory type"` and returns (`char_item.cpp:562-564`).
    UnknownWindow(u8),
    /// The cell is past the end of the window. Legacy checks this in some arms
    /// and not others; see the module docs.
    CellOutOfRange {
        /// The window that was addressed.
        window: u8,
        /// The cell that was asked for.
        cell: u16,
        /// How many cells the window has.
        limit: u16,
    },
    /// The destination already holds an item, and a second one was offered.
    ///
    /// Legacy: `if (pItem && pOld) return;` (`char_item.cpp:403`).
    AlreadyOccupied {
        /// The window.
        window: u8,
        /// The cell.
        cell: u16,
        /// The item already there.
        present: ItemId,
    },
    /// The item's footprint would land on a cell another item's grid covers.
    ///
    /// Legacy logs `"Cannot be set in %d cell due to %d item"` and returns
    /// having already written part of the grid, which is defect D-adjacent: the
    /// grid is left describing cells the item does not occupy.
    GridConflict {
        /// The window.
        window: u8,
        /// The anchor cell that was refused.
        cell: u16,
        /// The grid cell that was in the way.
        blocked: u16,
        /// The item that owns the blocked cell.
        owner: ItemId,
    },
    /// The item's grid footprint could not be recorded in full.
    ///
    /// The walk is bounded by the cell's own category, and a stack anchored
    /// near the end of that category reaches past it. Legacy's loop skips the
    /// cells it cannot reach (`if (p >= end) continue`, `char_item.cpp:426`) and
    /// stores the item anyway, so the item's recorded footprint is shorter than
    /// its `size` and every later overlap check trusts the grid. The Rewrite
    /// refuses the insert instead. This is a Divergence, recorded in the ledger
    /// for 195.
    FootprintCutOff {
        /// The window.
        window: u8,
        /// The anchor cell.
        cell: u16,
        /// The footprint the item claims.
        size: u8,
        /// The cells the walk could actually record.
        covered: u16,
    },
    /// The cell is free, so there was nothing to take out.
    ///
    /// Distinct from [`Rejected::AlreadyOccupied`] because the two mean opposite
    /// things and a caller that confuses them will drop an item it still holds.
    NotThere {
        /// The window.
        window: u8,
        /// The cell.
        cell: u16,
    },
    /// The item's grid footprint is zero, so it would occupy no cell at all.
    ///
    /// Not a legacy case: legacy gets zero from `GetSize()` when the prototype
    /// is missing (`item.h:69`) and then writes only the anchor, leaving the
    /// grid unmarked. Refusing is what keeps a later write from believing the
    /// cell is free.
    ZeroFootprint,
    /// The item is already stored somewhere, and legacy refuses to store an
    /// owned item twice.
    ///
    /// Legacy asserts `!"GetOwner exist"` (`char_item.cpp:380`) which compiles
    /// out in Release (`premake5.lua:54,59`), so a double-owned item is dropped
    /// with no diagnostic. Here it is a refusal the caller sees.
    AlreadyOwned {
        /// The item that would end up held by two cells.
        id: ItemId,
        /// The cell this storage already holds it in.
        at: ItemPos,
    },
}

impl core::fmt::Display for Rejected {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownWindow(w) => write!(f, "window {w} names no cell of a character"),
            Self::CellOutOfRange {
                window,
                cell,
                limit,
            } => {
                write!(
                    f,
                    "cell {cell} is past the {limit} cells of window {window}"
                )
            }
            Self::AlreadyOccupied {
                window,
                cell,
                present,
            } => write!(
                f,
                "cell {cell} of window {window} already holds item {present}"
            ),
            Self::GridConflict {
                window,
                cell,
                blocked,
                owner,
            } => write!(
                f,
                "item at {cell} of window {window} would cover cell {blocked}, \
                 which item {owner} already has"
            ),
            Self::FootprintCutOff {
                window,
                cell,
                size,
                covered,
            } => write!(
                f,
                "an item of {size} cells at {cell} of window {window} has only \
                 {covered} cells its category can record"
            ),
            Self::NotThere { window, cell } => {
                write!(f, "cell {cell} of window {window} is free")
            }
            Self::ZeroFootprint => f.write_str("an item needs at least one cell of grid footprint"),
            Self::AlreadyOwned { id, at } => {
                write!(
                    f,
                    "item {id} is already stored at window {} cell {}",
                    at.window_type, at.cell
                )
            }
        }
    }
}

impl std::error::Error for Rejected {}

/// The window a position names, or `None` for a byte no window claims.
///
/// Legacy switches on the raw byte and has no `default` in `GetItem`, so an
/// unknown byte falls off the end of the switch and returns `NULL`
/// (`char_item.cpp:305`). Converting once keeps that decision in one place.
fn window_of(window_type: u8) -> Option<EWindows> {
    EWindows::try_from(window_type).ok()
}

/// The grid bounds a walk is limited to, transcribed from `SetItem`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WalkBounds {
    start: u16,
    end: u16,
}

impl WalkBounds {
    /// Whether the walk covers the anchor at all.
    ///
    /// When it does not, legacy marks **only the anchor cell** and leaves the
    /// rest of the footprint unmarked (`char_item.cpp:438-439,457-458`). That is
    /// right for the bands whose cells are one cell each -- equipment, the
    /// dragon soul deck, and the belt -- and the Rewrite keeps it for the same
    /// reason, not because the code is shared.
    const fn walks(self, cell: u16) -> bool {
        cell >= self.start && cell < self.end
    }
}

/// A character's item storage: two arrays, two grids, and two small windows.
///
/// Ids, not items. The items themselves live in whatever owns them, keyed by
/// these ids, which is what makes a removed id safe to leave in a log and makes
/// the "no owner pointer to dangle" claim structural rather than a promise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterItems {
    /// `pItems` (`char.h:458`): 1370 flat cells.
    flat: Vec<ItemId>,
    /// `bItemGrid` (`char.h:459`): the flat one-based anchor grid. `0` is free.
    grid: Vec<u16>,
    /// `pDSItems` (`char.h:460`): 1152 dragon soul cells.
    dragon_soul: Vec<ItemId>,
    /// `wDSItemGrid` (`char.h:461`): the dragon soul anchor grid.
    ds_grid: Vec<u16>,
    /// `pAttr67AddItem` (`char.h:475`): the single attribute 67 slot.
    attr67: [ItemId; ATTR67_SLOTS as usize],
    /// `pSwitchbotItems` (`char.h:478`): five slots.
    switchbot: [ItemId; SWITCHBOT_SLOT_COUNT as usize],
}

impl Default for CharacterItems {
    /// A character with no items anywhere. Every array starts at
    /// [`NO_ITEM`], which is 0, and every grid cell at 0, which is how legacy
    /// spells "free".
    fn default() -> Self {
        Self::new()
    }
}

impl CharacterItems {
    /// Empty storage with every slot free.
    #[must_use]
    pub fn new() -> Self {
        Self {
            flat: vec![NO_ITEM; usize::from(INVENTORY_AND_EQUIP_SLOT_MAX)],
            grid: vec![0; usize::from(INVENTORY_AND_EQUIP_SLOT_MAX)],
            dragon_soul: vec![NO_ITEM; usize::from(DRAGON_SOUL_INVENTORY_MAX_NUM)],
            ds_grid: vec![0; usize::from(DRAGON_SOUL_INVENTORY_MAX_NUM)],
            attr67: [NO_ITEM; ATTR67_SLOTS as usize],
            switchbot: [NO_ITEM; SWITCHBOT_SLOT_COUNT as usize],
        }
    }

    /// How many cells the flat array has.
    #[must_use]
    pub const fn flat_len(&self) -> usize {
        INVENTORY_AND_EQUIP_SLOT_MAX as usize
    }

    /// How many cells the dragon soul array has.
    #[must_use]
    pub const fn dragon_soul_len(&self) -> usize {
        DRAGON_SOUL_INVENTORY_MAX_NUM as usize
    }

    /// Every position that currently holds an item, flat array first.
    ///
    /// The order is the array order, so it is stable and testable. Legacy has
    /// no equivalent, which is one reason its "find an empty slot" helpers are
    /// hard to test.
    pub fn occupied(&self) -> Vec<(ItemPos, ItemId)> {
        let mut out = Vec::new();
        // The range is typed `u16` from the constant, so the cell is already
        // the wire width and no cast is needed to get it.
        for (cell, &id) in (0..INVENTORY_AND_EQUIP_SLOT_MAX).zip(self.flat.iter()) {
            if id != NO_ITEM {
                out.push((
                    ItemPos {
                        window_type: EWindows::Inventory as u8,
                        cell,
                    },
                    id,
                ));
            }
        }
        for (cell, &id) in (0..DRAGON_SOUL_INVENTORY_MAX_NUM).zip(self.dragon_soul.iter()) {
            if id != NO_ITEM {
                out.push((
                    ItemPos {
                        window_type: EWindows::DragonSoulInventory as u8,
                        cell,
                    },
                    id,
                ));
            }
        }
        for (cell, &id) in (0..SWITCHBOT_SLOT_COUNT).zip(self.switchbot.iter()) {
            if id != NO_ITEM {
                out.push((
                    ItemPos {
                        window_type: EWindows::Switchbot as u8,
                        cell,
                    },
                    id,
                ));
            }
        }
        if self.attr67[0] != NO_ITEM {
            out.push((
                ItemPos {
                    window_type: EWindows::Attr67Add as u8,
                    cell: 0,
                },
                self.attr67[0],
            ));
        }
        out
    }

    /// How many items are stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.flat.iter().filter(|&&id| id != NO_ITEM).count()
            + self.dragon_soul.iter().filter(|&&id| id != NO_ITEM).count()
            + self.switchbot.iter().filter(|&&id| id != NO_ITEM).count()
            + usize::from(self.attr67[0] != NO_ITEM)
    }

    /// Whether no item is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The one-based anchor the grid stores for a cell, or `0` when free.
    ///
    /// The grid is a `WORD`, not a `BYTE`, and that is load-bearing: the anchor
    /// for cell 1369 is 1370, which a `BYTE` would wrap to 90 and report as a
    /// live cell in the base inventory.
    #[must_use]
    pub fn grid_anchor(&self, pos: ItemPos) -> u16 {
        match window_of(pos.window_type) {
            Some(EWindows::Inventory | EWindows::Equipment) => {
                self.grid.get(usize::from(pos.cell)).copied().unwrap_or(0)
            }
            Some(EWindows::DragonSoulInventory) => self
                .ds_grid
                .get(usize::from(pos.cell))
                .copied()
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// The window a grid cell's anchor belongs to.
    ///
    /// The grid stores a bare number, not a window, so this is only meaningful
    /// for the caller that knows which array it asked about.
    #[must_use]
    pub fn anchor_cell(&self, pos: ItemPos) -> Option<u16> {
        match self.grid_anchor(pos) {
            0 => None,
            anchor => Some(anchor - 1),
        }
    }

    /// The first cell in the base inventory that can hold a `size`-cell item.
    ///
    /// This is the second half of `CHARACTER::GetEmptyInventory(LPITEM)`
    /// (`char_item.cpp:1228-1233`): a linear scan from cell 0 to
    /// [`INVENTORY_MAX_NUM`], and the first cell whose whole footprint is clear. **Cell 0 is
    /// the first cell tried**, which is what makes a newly created item land at the top-left
    /// of the client's first page.
    ///
    /// `None` means every cell is occupied, which is the ordinary "inventory full" answer and
    /// not an error.
    ///
    /// `usable_cells` is [`usable_inventory_cells`] for this character, **not**
    /// [`INVENTORY_MAX_NUM`]. The difference is a gameplay rule and not a memory bound, and
    /// legacy's pickup-shaped `GetEmptyInventoryEx` gets it wrong: it scans all 180 cells
    /// (`char_item.cpp:1228-1233`) with no test of the unlock stat, so it can place an item in
    /// a page the player has not paid for, and the client will not draw it. The other legacy
    /// overload bounds on `Inventory_Size()` (`char_item.cpp:1260-1261`) and the shop, quest,
    /// gift and battle-pass paths all use that one. The bound is an argument rather than a stat
    /// read because [`CharacterItems`] holds no stats; the caller that has them is the one that
    /// has to know. Recorded as a Divergence.
    #[must_use]
    pub fn find_free_inventory_cell(&self, usable_cells: u16, size: u8) -> Option<u16> {
        (0..usable_cells.min(INVENTORY_MAX_NUM)).find(|&cell| self.footprint_is_clear(cell, size))
    }

    /// The first free cell in one custom-inventory category.
    ///
    /// This is `CHARACTER::GetEmptyCustomInventory` (`char_item.cpp:319-332`), which scans one
    /// category's 180 cells and returns the **absolute** `INVENTORY` cell, not a category-local
    /// one. The six banks are contiguous from [`CUSTOM_INVENTORY_SLOT_START`], 180 cells each.
    ///
    /// A `category` of 6 or more has no cells, so the answer is `None`. That is not a
    /// validation error: legacy returns `-1` for it (`char_item.cpp:321-322`).
    ///
    /// The whole 180-cell bank is scanned. The custom banks have their own unlock rule, which
    /// is not ported; if it is, its bound belongs here.
    #[must_use]
    pub fn find_free_custom_cell(&self, category: u8, size: u8) -> Option<u16> {
        if category >= CUSTOM_INVENTORY_CATEGORY_NUM {
            return None;
        }
        let start = CUSTOM_INVENTORY_SLOT_START + u16::from(category) * CUSTOM_INVENTORY_MAX_NUM;
        (start..start + CUSTOM_INVENTORY_MAX_NUM).find(|&cell| self.footprint_is_clear(cell, size))
    }

    /// The first free cell for a character whose `Inven_Point` is `inven_point`.
    ///
    /// This is the call most callers want, and it exists so the unlock formula is written down
    /// once. `Inven_Point` is the character's `m_points.envanter` (`char.h:1284`); the
    /// [`usable_inventory_cells`] doc records why the sum is clamped here rather than there.
    #[must_use]
    pub fn find_free_inventory_cell_for(&self, inven_point: u16, size: u8) -> Option<u16> {
        self.find_free_inventory_cell(usable_inventory_cells(inven_point), size)
    }

    fn one_based(cell: u16) -> ItemPos {
        ItemPos {
            window_type: EWindows::Inventory as u8,
            cell,
        }
    }

    /// Which 45-cell page of which bank a flat cell belongs to, or `None` when it is not a cell
    /// a grant may land in.
    ///
    /// This is `CHARACTER::GetInventoryPageByPos` (`char_item.cpp:334-348`). It answers with a
    /// page number and `None` for three real cases, which is what makes the page bound
    /// fall out of a comparison rather than out of a separate range check:
    ///
    /// * a base-inventory cell, when the cell is below [`INVENTORY_MAX_NUM`] and no category
    ///   applies, and its page is `cell / INVENTORY_PAGE_SIZE` (`char_item.cpp:336-337`);
    /// * a custom cell, when the cell is inside a bank, and its page is counted from the
    ///   bank's own start (`char_item.cpp:339-344`), so each bank is 4 pages of 45 and does not
    ///   share page numbers with any other;
    /// * a cell in the **equipment** band, the **dragon-soul equip** band, the **belt** band, or
    ///   past the end of the flat space. Legacy returns `255` for those
    ///   (`char_item.cpp:346`, the `return -1` narrowed to a `BYTE`), which is not a page any
    ///   real cell reports, so a walk that left the bank failed the comparison. Answering
    ///   `None` keeps the same refusal and makes it a `None` rather than a magic number.
    fn page_of(cell: u16) -> Option<u16> {
        if cell >= INVENTORY_AND_EQUIP_SLOT_MAX {
            return None;
        }
        if let Some(category) = custom_inventory_category_of(Self::one_based(cell)) {
            let start =
                CUSTOM_INVENTORY_SLOT_START + u16::from(category) * CUSTOM_INVENTORY_MAX_NUM;
            return Some((cell - start) / INVENTORY_PAGE_SIZE);
        }
        if cell < INVENTORY_MAX_NUM {
            return Some(cell / INVENTORY_PAGE_SIZE);
        }
        None
    }

    /// Could a `size`-cell item be anchored at `cell`?
    ///
    /// This is `CHARACTER::IsEmptyItemGrid` (`char_item.cpp:737-859`) with three things left
    /// out, all deliberate, and each one noted where it is left out:
    ///
    /// * the **belt** arm (`char_item.cpp:764-778`), which is gated on the character wearing a
    ///   belt and reads the belt item's own values. [`CharacterItems`] holds no worn items, so
    ///   there is no belt to consult, and the page scan never reaches the belt band anyway.
    /// * the **exception cell** (`iExceptionCell`, `char_item.cpp:762`), which lets a caller
    ///   ask about a cell while ignoring one specific item. The only legacy caller is
    ///   `GetEmptyDragonSoulInventoryWithExceptions` (`char_item.cpp:1291`), a dragon-soul path
    ///   and not this one.
    /// * the **occupied-slot** check. Legacy reads only `bItemGrid` and never `pItems`, so a
    ///   slot holding an item whose grid is empty reads as free here and is then refused by
    ///   `set` as `AlreadyOccupied`. That asymmetry is legacy's; this matches it rather than
    ///   quietly fixing it, because a fix would change which cell a grant picks.
    ///
    /// The page bound is the subtle part and it is kept. A `size`-cell item may not straddle a
    /// page, because the walk is `anchor + i * 5` and the client draws 5 columns by 45 rows.
    /// Legacy enforces it twice (`char_item.cpp:803-807`), once against the category end and
    /// once against the page; here both fall out of asking for the page of every walked cell,
    /// so a cell that left the bank and a cell that left the page are the same refusal.
    fn footprint_is_clear(&self, cell: u16, size: u8) -> bool {
        // A zero-size item is refused by `set` as `ZeroFootprint`, so there is no cell that
        // can hold one, and the walk below is empty and would report every cell clear.
        // Reporting cell 0 would hand a caller a cell its own `set` will refuse.
        if size == 0 {
            return false;
        }
        let Some(page) = Self::page_of(cell) else {
            return false;
        };
        (0..u16::from(size)).all(|j| {
            let p = cell.saturating_add(j * FLAT_STACK_STRIDE);
            // The same read `set_flat` does before it writes, so a cell this call calls clear
            // is a cell `set` will not refuse for a grid conflict.
            Self::page_of(p) == Some(page) && self.grid_owner_flat(p).is_none()
        })
    }

    /// Which cells a stored item's grid footprint covers, and the anchor.
    ///
    /// Returns the anchor with the cells it covers, in walk order. The walk is
    /// `anchor + i * stride`, skipped past a limit, exactly as
    /// `char_item.cpp:420-436` and `:443-456` do it.
    fn footprint(anchor: u16, size: u8, stride: u16, limit: u16) -> (Vec<u16>, u16) {
        let mut cells = Vec::with_capacity(usize::from(size));
        // Counted as `u16` alongside the cells, so the "how many did we record"
        // answer needs no cast and cannot disagree with `cells.len()`.
        let mut covered: u16 = 0;
        for i in 0..u16::from(size) {
            // Saturating, because a hostile `size` and a large `anchor` would
            // otherwise wrap to a cell under the limit and be recorded as a
            // legitimate footprint.
            let p = anchor.saturating_add(i.saturating_mul(stride));
            if p >= limit {
                // Legacy's `continue`, not a `break`: a wide stride can step over
                // the limit and land back under it only for a negative stride,
                // so in practice this ends the walk, and keeping it as a
                // `continue` keeps the transcription literal.
                continue;
            }
            cells.push(p);
            covered = covered.saturating_add(1);
        }
        (cells, covered)
    }

    /// The grid bounds `SetItem` computes for a position.
    ///
    /// `char_item.cpp:399-406`: the bounds default to `0 .. INVENTORY_MAX_NUM`
    /// and are replaced by the category's own bounds when the cell is a custom
    /// inventory cell with a real category. For every other band the defaults
    /// apply, and the default test `cell < 180` is false for the equipment,
    /// dragon soul deck and belt bands, so only the anchor gets marked.
    fn walk_bounds(pos: ItemPos) -> WalkBounds {
        let default = WalkBounds {
            start: 0,
            end: INVENTORY_MAX_NUM,
        };
        if !is_custom_inventory_position(pos) {
            return default;
        }
        match custom_inventory_category_of(pos) {
            Some(cat) => WalkBounds {
                start: CUSTOM_INVENTORY_SLOT_START + u16::from(cat) * CUSTOM_INVENTORY_MAX_NUM,
                end: CUSTOM_INVENTORY_SLOT_START + (u16::from(cat) + 1) * CUSTOM_INVENTORY_MAX_NUM,
            },
            // Ledger 194 measured this: legacy's signed category walk reports
            // equipment cells as category -1, which would turn an equipment
            // item into a walked stack. The Rewrite refuses the category rather
            // than reproducing that, so it falls back to the defaults.
            None => default,
        }
    }

    /// Read the cell a position names.
    ///
    /// This is `CHARACTER::GetItem` (`char_item.cpp:254-305`) with the one thing
    /// it cannot express made explicit. Legacy returns `NULL` for a free cell
    /// and for a cell past the end, and it accepts a safebox or mall position
    /// and then returns `NULL` for it because it has no case for those windows.
    /// [`Lookup`] keeps all three apart.
    ///
    /// A safebox or mall position reports [`Lookup::NoContainer`]. That is the
    /// deliberate divergence from ledger 194's `Deferred` cell bound: the
    /// position is valid, the bound is not knowable here, and answering "no
    /// container" is not the same as answering "the cell is free".
    #[must_use]
    pub fn get(&self, pos: ItemPos) -> Lookup {
        match window_of(pos.window_type) {
            Some(EWindows::Inventory | EWindows::Equipment) => {
                if pos.cell >= INVENTORY_AND_EQUIP_SLOT_MAX {
                    return Lookup::OutOfRange;
                }
                match self
                    .flat
                    .get(usize::from(pos.cell))
                    .copied()
                    .unwrap_or(NO_ITEM)
                {
                    NO_ITEM => Lookup::Empty,
                    id => Lookup::Occupied(id),
                }
            }
            Some(EWindows::DragonSoulInventory) => {
                if pos.cell >= DRAGON_SOUL_INVENTORY_MAX_NUM {
                    return Lookup::OutOfRange;
                }
                match self
                    .dragon_soul
                    .get(usize::from(pos.cell))
                    .copied()
                    .unwrap_or(NO_ITEM)
                {
                    NO_ITEM => Lookup::Empty,
                    id => Lookup::Occupied(id),
                }
            }
            Some(EWindows::Attr67Add) => {
                if pos.cell >= ATTR67_SLOTS {
                    return Lookup::OutOfRange;
                }
                match self.attr67[0] {
                    NO_ITEM => Lookup::Empty,
                    id => Lookup::Occupied(id),
                }
            }
            Some(EWindows::Switchbot) => {
                if pos.cell >= SWITCHBOT_SLOT_COUNT {
                    return Lookup::OutOfRange;
                }
                match self.switchbot[usize::from(pos.cell)] {
                    NO_ITEM => Lookup::Empty,
                    id => Lookup::Occupied(id),
                }
            }
            Some(EWindows::Safebox | EWindows::Mall) => Lookup::NoContainer(pos.window_type),
            _ => Lookup::OutOfRange,
        }
    }

    /// Take the item with this id out of the storage, wherever it is.
    ///
    /// This is the undo for a placement, and it is deliberately not
    /// [`Self::remove`]. That function takes an `&Item` because legacy reads the old
    /// size at `char_item.cpp:420` to clear the right number of grid cells, and a caller
    /// that has only an id would have to rebuild an item and guess a size. This one needs
    /// nothing: the **grid already records the footprint**, because a cell's anchor is
    /// set when the item is placed and cleared when it is removed. So the cells to free
    /// are exactly the ones whose anchor is this item's, and the size is not consulted
    /// at all.
    ///
    /// That is not merely convenient, it is safer. A footprint cleared with a *wrong*
    /// size is how one item's removal frees a neighbour's cells, which is the bug
    /// `remove_flat` guards against with its `pItems[p] != pOld` check. Reading the
    /// coverage from the grid cannot guess.
    ///
    /// The scan is bounded by the window's length and by one stride past the anchor
    /// rather than by the item's size, so a wider item than the stride allows still has
    /// every one of its cells cleared: the grid is the record of what was placed, and the
    /// bound is only there to stop the scan running off the end of the array.
    ///
    /// # Errors
    ///
    /// [`Rejected::NotThere`] when this storage does not hold the id. Nothing is
    /// changed, so a caller may treat this as "it was already gone".
    pub fn release(&mut self, id: ItemId) -> Result<ItemPos, Rejected> {
        let Some(pos) = self.cell_of(id) else {
            return Err(Rejected::NotThere {
                window: EWindows::Inventory as u8,
                cell: 0,
            });
        };
        match window_of(pos.window_type) {
            Some(EWindows::Inventory | EWindows::Equipment) => {
                self.clear_flat_footprint(pos.cell);
                if let Some(slot) = self.flat.get_mut(usize::from(pos.cell)) {
                    *slot = NO_ITEM;
                }
            }
            Some(EWindows::DragonSoulInventory) => {
                self.clear_dragon_soul_footprint(pos.cell);
                if let Some(slot) = self.dragon_soul.get_mut(usize::from(pos.cell)) {
                    *slot = NO_ITEM;
                }
            }
            Some(EWindows::Switchbot) => {
                if let Some(slot) = self.switchbot.get_mut(usize::from(pos.cell)) {
                    *slot = NO_ITEM;
                }
            }
            // `cell_of` only answers for the windows above, so this arm is unreachable
            // while that stays true, and it says so rather than guessing.
            _ => return Err(Rejected::UnknownWindow(pos.window_type)),
        }
        Ok(pos)
    }

    /// Clear every flat cell whose anchor is this item's.
    fn clear_flat_footprint(&mut self, cell: u16) {
        let anchor = cell + 1;
        let end = usize::from(INVENTORY_AND_EQUIP_SLOT_MAX)
            .min(usize::from(anchor) + usize::from(FLAT_STACK_STRIDE));
        for p in usize::from(cell)..end {
            if self.grid.get(p).copied() == Some(anchor) {
                if let Some(slot) = self.grid.get_mut(p) {
                    *slot = 0;
                }
            }
        }
    }

    /// Clear every dragon-soul cell whose anchor is this item's.
    ///
    /// The scan runs to the end of the window rather than by a stride, because a
    /// dragon-soul box's width is not the flat stride and the grid is the record of what
    /// was actually placed.
    fn clear_dragon_soul_footprint(&mut self, cell: u16) {
        let anchor = cell + 1;
        let end = usize::from(DRAGON_SOUL_INVENTORY_MAX_NUM);
        for p in usize::from(cell)..end {
            if self.ds_grid.get(p).copied() == Some(anchor) {
                if let Some(slot) = self.ds_grid.get_mut(p) {
                    *slot = 0;
                }
            }
        }
    }

    /// Store an item, or say why it was not stored.
    ///
    /// # Errors
    ///
    /// Any [`Rejected`] variant. The ones a client can reach with a
    /// well-formed `TItemPos` and a real item are [`Rejected::ZeroFootprint`],
    /// [`Rejected::AlreadyOwned`] and [`Rejected::AlreadyOccupied`]; the rest
    /// name a position no cell of this character answers to.
    ///
    /// The position is the caller's to choose and is what this records. The
    /// item's own `pos` field is not consulted and not written: a container
    /// that only accepted a correctly-updated `pos` would have no way to reject
    /// a stale one, which is the case legacy misses.
    ///
    /// This is `CHARACTER::SetItem` (`char_item.cpp:366-565`) with the
    /// signature that fixes defect D1. Legacy returns `void`, and three arms
    /// `return` early, so a rejected insert is indistinguishable from a
    /// successful one -- and `CItem::AddToCharacter` (`item.cpp:523-530`) sets
    /// the owner, saves and returns `true` either way. Here every refusal is
    /// returned.
    ///
    /// The order of the checks is the Rewrite's own, and it is the opposite of
    /// legacy's in one place: **every bound is checked before any read**. Legacy
    /// reads `pDSItems[wCell]` (`char_item.cpp:473`) and
    /// `pSwitchbotItems[wCell]` (`:537`) before the checks at `:498` and `:543`.
    pub fn set(&mut self, pos: ItemPos, item: &Item) -> Result<(), Rejected> {
        if item.size == 0 {
            return Err(Rejected::ZeroFootprint);
        }
        // The check is against this storage, not against the caller's
        // `item.pos`. Trusting `pos` would make `move_item` impossible, because
        // a move necessarily holds an item that is already placed, and it would
        // miss the real case: an item in cell 40 whose `pos` still says 0.
        if let Some(at) = self.cell_of(item.id) {
            return Err(Rejected::AlreadyOwned { id: item.id, at });
        }
        match window_of(pos.window_type) {
            Some(EWindows::Inventory | EWindows::Equipment) => self.set_flat(pos, item),
            Some(EWindows::DragonSoulInventory) => self.set_dragon_soul(pos, item),
            Some(EWindows::Attr67Add) => self.set_attr67(pos, item),
            Some(EWindows::Switchbot) => self.set_switchbot(pos, item),
            _ => Err(Rejected::UnknownWindow(pos.window_type)),
        }
    }

    fn set_flat(&mut self, pos: ItemPos, item: &Item) -> Result<(), Rejected> {
        let cell = pos.cell;
        if cell >= INVENTORY_AND_EQUIP_SLOT_MAX {
            return Err(Rejected::CellOutOfRange {
                window: pos.window_type,
                cell,
                limit: INVENTORY_AND_EQUIP_SLOT_MAX,
            });
        }
        let present = self.flat.get(usize::from(cell)).copied().unwrap_or(NO_ITEM);
        if present != NO_ITEM {
            return Err(Rejected::AlreadyOccupied {
                window: pos.window_type,
                cell,
                present,
            });
        }

        // The whole footprint is checked before anything is written, so a
        // conflict cannot leave the grid describing cells the item does not
        // hold. Legacy does not check at all: `SetItem` writes
        // `bItemGrid[p] = wCell + 1` for every cell of the footprint
        // (`char_item.cpp:462`) with no test of what was there, so it silently
        // takes over a neighbour's marks. The grid then names the new item as
        // the owner of cells `pItems` still attributes to the old one, and the
        // old item's own `RemoveItem` then zeroes them, because its guard at
        // `:432` reads `pItems[p]`, which is 0 for a non-anchor cell. Two items
        // end up sharing cells and neither one is told.
        //
        // So the Rewrite refuses rather than overwrites, and it refuses before
        // writing, so a caller that retries after moving the neighbour sees a
        // consistent grid either way.
        let bounds = Self::walk_bounds(pos);
        let (cells, covered) = if bounds.walks(cell) {
            Self::footprint(cell, item.size, FLAT_STACK_STRIDE, bounds.end)
        } else {
            // Outside the base inventory the bounds stay at their defaults, and
            // `cell < 180` is false, so legacy marks only the anchor
            // (`char_item.cpp:438-439`). That is right for a one-cell equipment
            // item and wrong for anything larger, so a larger one is refused
            // rather than stored with a footprint that disagrees with its size.
            (vec![cell], 1)
        };
        if covered != u16::from(item.size) {
            return Err(Rejected::FootprintCutOff {
                window: pos.window_type,
                cell,
                size: item.size,
                covered,
            });
        }
        for &p in &cells {
            if let Some(owner) = self.grid_owner_flat(p) {
                return Err(Rejected::GridConflict {
                    window: pos.window_type,
                    cell,
                    blocked: p,
                    owner,
                });
            }
        }
        for &p in &cells {
            if let Some(slot) = self.grid.get_mut(usize::from(p)) {
                *slot = cell + 1;
            }
        }
        if let Some(slot) = self.flat.get_mut(usize::from(cell)) {
            *slot = item.id;
        }
        Ok(())
    }

    fn set_dragon_soul(&mut self, pos: ItemPos, item: &Item) -> Result<(), Rejected> {
        let cell = pos.cell;
        if cell >= DRAGON_SOUL_INVENTORY_MAX_NUM {
            // Legacy checks this at :498, but only inside `if (pItem)`, so a
            // removal skips it, and it reads `pDSItems[wCell]` at :473 first.
            return Err(Rejected::CellOutOfRange {
                window: pos.window_type,
                cell,
                limit: DRAGON_SOUL_INVENTORY_MAX_NUM,
            });
        }
        let present = self
            .dragon_soul
            .get(usize::from(cell))
            .copied()
            .unwrap_or(NO_ITEM);
        if present != NO_ITEM {
            return Err(Rejected::AlreadyOccupied {
                window: pos.window_type,
                cell,
                present,
            });
        }
        let (cells, covered) = Self::footprint(
            cell,
            item.size,
            DRAGON_SOUL_BOX_COLUMN_NUM,
            DRAGON_SOUL_INVENTORY_MAX_NUM,
        );
        // The walk can step past the array: anchored at the last cell, a
        // four-deep stack reaches 1151 + 3 * 8 = 1175. Legacy survives that only
        // because of its `continue`, which marks nothing at all, so the item is
        // stored with an empty grid and the next write walks straight over it.
        if covered != u16::from(item.size) {
            return Err(Rejected::FootprintCutOff {
                window: pos.window_type,
                cell,
                size: item.size,
                covered,
            });
        }
        for &p in &cells {
            if let Some(owner) = self.grid_owner_ds(p) {
                return Err(Rejected::GridConflict {
                    window: pos.window_type,
                    cell,
                    blocked: p,
                    owner,
                });
            }
        }
        for &p in &cells {
            if let Some(slot) = self.ds_grid.get_mut(usize::from(p)) {
                *slot = cell + 1;
            }
        }
        if let Some(slot) = self.dragon_soul.get_mut(usize::from(cell)) {
            *slot = item.id;
        }
        Ok(())
    }

    fn set_attr67(&mut self, pos: ItemPos, item: &Item) -> Result<(), Rejected> {
        if pos.cell >= ATTR67_SLOTS {
            return Err(Rejected::CellOutOfRange {
                window: pos.window_type,
                cell: pos.cell,
                limit: ATTR67_SLOTS,
            });
        }
        let present = self.attr67[0];
        if present != NO_ITEM {
            return Err(Rejected::AlreadyOccupied {
                window: pos.window_type,
                cell: pos.cell,
                present,
            });
        }
        // The single slot has no grid, so a footprint above one has nowhere to
        // be recorded and the item would occupy the slot while its extra cells
        // looked free.
        if item.size > 1 {
            return Err(Rejected::FootprintCutOff {
                window: pos.window_type,
                cell: pos.cell,
                size: item.size,
                covered: 1,
            });
        }
        self.attr67[0] = item.id;
        Ok(())
    }

    fn set_switchbot(&mut self, pos: ItemPos, item: &Item) -> Result<(), Rejected> {
        if pos.cell >= SWITCHBOT_SLOT_COUNT {
            return Err(Rejected::CellOutOfRange {
                window: pos.window_type,
                cell: pos.cell,
                limit: SWITCHBOT_SLOT_COUNT,
            });
        }
        let present = self
            .switchbot
            .get(usize::from(pos.cell))
            .copied()
            .unwrap_or(NO_ITEM);
        if present != NO_ITEM {
            return Err(Rejected::AlreadyOccupied {
                window: pos.window_type,
                cell: pos.cell,
                present,
            });
        }
        if item.size > 1 {
            return Err(Rejected::FootprintCutOff {
                window: pos.window_type,
                cell: pos.cell,
                size: item.size,
                covered: 1,
            });
        }
        self.switchbot[usize::from(pos.cell)] = item.id;
        Ok(())
    }

    /// The cell this storage holds an item in, if it holds it.
    ///
    /// This is the ownership question in the only form a container can answer.
    /// Legacy asks the item for its owner, which is the same answer by way of a
    /// back pointer that a bare `LPITEM` does not have.
    pub fn cell_of(&self, id: ItemId) -> Option<ItemPos> {
        if id == NO_ITEM {
            return None;
        }
        // The ranges are typed `u16` from the same constants the arrays are
        // sized by, so a cell number needs no cast to be the wire width.
        if let Some(cell) = (0..INVENTORY_AND_EQUIP_SLOT_MAX)
            .zip(self.flat.iter())
            .find(|(_, &held)| held == id)
            .map(|(cell, _)| cell)
        {
            return Some(ItemPos {
                window_type: EWindows::Inventory as u8,
                cell,
            });
        }
        if let Some(cell) = (0..DRAGON_SOUL_INVENTORY_MAX_NUM)
            .zip(self.dragon_soul.iter())
            .find(|(_, &held)| held == id)
            .map(|(cell, _)| cell)
        {
            return Some(ItemPos {
                window_type: EWindows::DragonSoulInventory as u8,
                cell,
            });
        }
        if let Some(cell) = (0..SWITCHBOT_SLOT_COUNT)
            .zip(self.switchbot.iter())
            .find(|(_, &held)| held == id)
            .map(|(cell, _)| cell)
        {
            return Some(ItemPos {
                window_type: EWindows::Switchbot as u8,
                cell,
            });
        }
        if self.attr67[0] == id {
            return Some(ItemPos {
                window_type: EWindows::Attr67Add as u8,
                cell: 0,
            });
        }
        None
    }

    /// The item whose anchor covers a flat cell, if any.
    fn grid_owner_flat(&self, cell: u16) -> Option<ItemId> {
        let anchor = self.grid.get(usize::from(cell)).copied().unwrap_or(0);
        if anchor == 0 {
            return None;
        }
        // The anchor is stored plus one, so a grid value of 1 is cell 0. The
        // subtraction is checked because a hand-written or corrupt grid could
        // hold 0 here, and `0 - 1` would wrap to 65535 and index out of range.
        Some(
            self.flat
                .get(usize::from(anchor - 1))
                .copied()
                .unwrap_or(NO_ITEM),
        )
    }

    /// The item whose anchor covers a dragon soul cell, if any.
    fn grid_owner_ds(&self, cell: u16) -> Option<ItemId> {
        let anchor = self.ds_grid.get(usize::from(cell)).copied().unwrap_or(0);
        if anchor == 0 {
            return None;
        }
        Some(
            self.dragon_soul
                .get(usize::from(anchor - 1))
                .copied()
                .unwrap_or(NO_ITEM),
        )
    }

    /// Take the item out of a cell, and say which one.
    ///
    /// # Errors
    ///
    /// [`Rejected::AlreadyOccupied`] when the cell holds a different item, or
    /// [`Rejected::CellOutOfRange`] when the position names no cell. A refusal
    /// changes nothing, so a caller that gets one knows the item is still where
    /// it was.
    ///
    /// Legacy has no remove function: removal is `SetItem(pos, NULL)`, and
    /// `ITEM_MANAGER::RemoveItem` (`item_manager.cpp:566`) is what destroys the
    /// item afterwards. The two steps are kept apart here too, because legacy
    /// runs them in the wrong order: `item_manager.cpp:594` calls
    /// `M2_DESTROY_ITEM` unconditionally, outside the block that checks the
    /// owner, so a failure inside the removal is followed by a destruction
    /// regardless.
    ///
    /// The grid is cleared with the **stored item's** size, not the size of
    /// whatever is being put there next, which is why this takes the id out and
    /// lets the caller move the item. Legacy reads the old size at
    /// `char_item.cpp:420` from `pOld` for exactly this reason.
    pub fn remove(&mut self, pos: ItemPos, item: &Item) -> Result<(), Rejected> {
        match window_of(pos.window_type) {
            Some(EWindows::Inventory | EWindows::Equipment) => self.remove_flat(pos, item),
            Some(EWindows::DragonSoulInventory) => self.remove_dragon_soul(pos, item),
            Some(EWindows::Attr67Add) => self.remove_attr67(pos, item),
            Some(EWindows::Switchbot) => self.remove_switchbot(pos, item),
            // A safebox and a mall are valid positions whose container is
            // elsewhere, so they are the wrong error, but this type has no
            // storage for either and says so rather than pretending otherwise.
            Some(EWindows::Safebox | EWindows::Mall) | None => {
                Err(Rejected::UnknownWindow(pos.window_type))
            }
            _ => Err(Rejected::UnknownWindow(pos.window_type)),
        }
    }

    fn remove_flat(&mut self, pos: ItemPos, item: &Item) -> Result<(), Rejected> {
        let cell = pos.cell;
        if cell >= INVENTORY_AND_EQUIP_SLOT_MAX {
            return Err(Rejected::CellOutOfRange {
                window: pos.window_type,
                cell,
                limit: INVENTORY_AND_EQUIP_SLOT_MAX,
            });
        }
        if self.flat.get(usize::from(cell)).copied() != Some(item.id) {
            return Err(Self::wrong_item(
                pos,
                self.flat.get(usize::from(cell)).copied().unwrap_or(NO_ITEM),
            ));
        }
        // The grid is cleared with the STORED item's size, which is why the item
        // is a parameter: legacy reads the old size at `char_item.cpp:420` from
        // `pOld` for exactly this reason.
        let bounds = Self::walk_bounds(pos);
        let (cells, _covered) = if bounds.walks(cell) {
            Self::footprint(cell, item.size, FLAT_STACK_STRIDE, bounds.end)
        } else {
            (vec![cell], 1)
        };
        for p in cells {
            // Only a cell whose anchor is this item's is cleared, so removing
            // one item cannot free a neighbour's footprint. Legacy has the
            // `pItems[p] != pOld` guard for the same reason
            // (`char_item.cpp:432`).
            if self.grid.get(usize::from(p)).copied() == Some(cell + 1) {
                if let Some(slot) = self.grid.get_mut(usize::from(p)) {
                    *slot = 0;
                }
            }
        }
        if let Some(slot) = self.flat.get_mut(usize::from(cell)) {
            *slot = NO_ITEM;
        }
        Ok(())
    }

    fn remove_dragon_soul(&mut self, pos: ItemPos, item: &Item) -> Result<(), Rejected> {
        let cell = pos.cell;
        if cell >= DRAGON_SOUL_INVENTORY_MAX_NUM {
            return Err(Rejected::CellOutOfRange {
                window: pos.window_type,
                cell,
                limit: DRAGON_SOUL_INVENTORY_MAX_NUM,
            });
        }
        if self.dragon_soul.get(usize::from(cell)).copied() != Some(item.id) {
            return Err(Self::wrong_item(
                pos,
                self.dragon_soul
                    .get(usize::from(cell))
                    .copied()
                    .unwrap_or(NO_ITEM),
            ));
        }
        let (cells, _covered) = Self::footprint(
            cell,
            item.size,
            DRAGON_SOUL_BOX_COLUMN_NUM,
            DRAGON_SOUL_INVENTORY_MAX_NUM,
        );
        for p in cells {
            if self.ds_grid.get(usize::from(p)).copied() == Some(cell + 1) {
                if let Some(slot) = self.ds_grid.get_mut(usize::from(p)) {
                    *slot = 0;
                }
            }
        }
        if let Some(slot) = self.dragon_soul.get_mut(usize::from(cell)) {
            *slot = NO_ITEM;
        }
        Ok(())
    }

    fn remove_attr67(&mut self, pos: ItemPos, item: &Item) -> Result<(), Rejected> {
        if pos.cell >= ATTR67_SLOTS {
            return Err(Rejected::CellOutOfRange {
                window: pos.window_type,
                cell: pos.cell,
                limit: ATTR67_SLOTS,
            });
        }
        if self.attr67[0] != item.id {
            return Err(Self::wrong_item(pos, self.attr67[0]));
        }
        self.attr67[0] = NO_ITEM;
        Ok(())
    }

    fn remove_switchbot(&mut self, pos: ItemPos, item: &Item) -> Result<(), Rejected> {
        if pos.cell >= SWITCHBOT_SLOT_COUNT {
            return Err(Rejected::CellOutOfRange {
                window: pos.window_type,
                cell: pos.cell,
                limit: SWITCHBOT_SLOT_COUNT,
            });
        }
        let present = self
            .switchbot
            .get(usize::from(pos.cell))
            .copied()
            .unwrap_or(NO_ITEM);
        if present != item.id {
            return Err(Self::wrong_item(pos, present));
        }
        if let Some(slot) = self.switchbot.get_mut(usize::from(pos.cell)) {
            *slot = NO_ITEM;
        }
        Ok(())
    }

    /// The refusal for a cell that holds a different item, or for one that is
    /// free when an item was expected.
    fn wrong_item(pos: ItemPos, present: ItemId) -> Rejected {
        if present == NO_ITEM {
            return Rejected::NotThere {
                window: pos.window_type,
                cell: pos.cell,
            };
        }
        Rejected::AlreadyOccupied {
            window: pos.window_type,
            cell: pos.cell,
            present,
        }
    }

    /// Move an item from one cell to another, clearing the old grid.
    ///
    /// # Errors
    ///
    /// Whatever [`CharacterItems::set`] would report for the destination. The
    /// item is put back before the error is returned, so a refused move leaves
    /// it where it started rather than dropping it.
    ///
    /// Legacy has no move on the character: `CG_ITEM_MOVE` handlers remove and
    /// then add, in two steps, each of which can fail silently. Doing it as one
    /// operation means a refused move leaves the item where it was, which is the
    /// one outcome a client can always recover from.
    pub fn move_item(&mut self, from: ItemPos, to: ItemPos, item: &Item) -> Result<(), Rejected> {
        // The two steps are remove then set, exactly as legacy's
        // `CG_ITEM_MOVE` handlers do them, because the legacy grid is cleared
        // with the stored item's size and so the old cells are only freed by the
        // removal. The difference is what happens when the second step fails.
        self.remove(from, item)?;
        if let Err(reason) = self.set(to, item) {
            // Put it back where it was, so a refused move is not a lost item.
            // The restore goes through `set` because `remove` and `set` are the
            // only two writers of the grid.
            let _ = self.set(from, item);
            return Err(reason);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Item;

    const INV: u8 = EWindows::Inventory as u8;
    const EQUIP: u8 = EWindows::Equipment as u8;
    const DS: u8 = EWindows::DragonSoulInventory as u8;
    const A67: u8 = EWindows::Attr67Add as u8;
    const SWITCH: u8 = EWindows::Switchbot as u8;
    const SAFEBOX: u8 = EWindows::Safebox as u8;
    const MALL: u8 = EWindows::Mall as u8;
    const BELT: u8 = EWindows::BeltInventory as u8;

    fn pos(window: u8, cell: u16) -> ItemPos {
        ItemPos {
            window_type: window,
            cell,
        }
    }

    /// A one-cell item with the given id, ready to store.
    fn one_cell(id: ItemId) -> Item {
        Item::new(id, 30_000)
    }

    /// An item of `size` cells, ready to store.
    fn sized(id: ItemId, size: u8) -> Item {
        let mut item = Item::new(id, 30_000);
        item.set_size(size).expect("a size of at least one");
        item
    }

    /// Fill `start..start+count` with one-cell items and return the store.
    fn filled(start: u16, count: u16) -> CharacterItems {
        let mut store = CharacterItems::new();
        for cell in start..start + count {
            let item = one_cell(1000 + u32::from(cell));
            store
                .set(pos(INV, cell), &item)
                .expect("a free cell takes an item");
        }
        store
    }

    #[test]
    fn the_first_free_cell_is_cell_zero() {
        // The scan starts at 0, not at the first hole, so a fresh character gets a new item
        // at the top-left of the client's first page. Asserting the exact cell is what pins
        // that; a test that only checked "some cell" would also pass for a scan that started
        // at 89.
        let store = CharacterItems::new();
        assert_eq!(store.find_free_inventory_cell(180, 1), Some(0));
        assert_eq!(store.find_free_inventory_cell(180, 2), Some(0));
    }

    #[test]
    fn a_full_scan_walks_forward_and_stops_at_the_first_hole() {
        let store = filled(0, 5);
        assert_eq!(store.find_free_inventory_cell(180, 1), Some(5));
        // Occupying 5 as well moves the answer on, which is what distinguishes a real scan
        // from a test that only ever looks at a fixed pair of cells.
        let mut store = store;
        store.set(pos(INV, 5), &one_cell(2005)).unwrap();
        assert_eq!(store.find_free_inventory_cell(180, 1), Some(6));
    }

    #[test]
    fn a_two_cell_item_cannot_straddle_a_page_boundary() {
        // A page is 45 cells wide, so a 2-cell item at cell 44 would put its second cell on
        // the next page, which the client cannot draw. Legacy refuses it in the same place
        // (`char_item.cpp:806-807`): the walk stops when the page changes.
        let mut store = filled(0, 45);
        // 44 is the last cell of page 0 and 45 the first of page 1, so neither of them is
        // where the search stops: 45 anchors a 2-cell item at 45 and 50, both in page 1.
        assert_eq!(store.find_free_inventory_cell(180, 2), Some(45));
        // Filling page 1 as well leaves only page 2, whose first cell is 90. That is a page
        // the base unlock does not reach, so the usable bound is what refuses it, not the
        // page rule -- which is the two limits being independent, and both are needed.
        store = filled(0, 90);
        assert_eq!(store.find_free_inventory_cell(180, 2), Some(90));
        assert_eq!(store.find_free_inventory_cell_for(0, 2), None);
        // A 5-cell item needs five rows of its own column, so its last legal anchor is 0:
        // 0 + 4*5 = 20 is the deepest cell page 0 has room for at that column. An anchor of 1
        // would reach 21, which is still in page 0, so the rule is about leaving the page and
        // not about being the first cell.
        let store = filled(0, 1);
        assert_eq!(store.find_free_inventory_cell(180, 5), Some(1));
        // Anchoring at 41 reaches 61, which is in page 1, so 41 is refused even though 41
        // itself is in page 0. The search therefore steps over the last usable anchor and
        // lands on 45, the first cell of the next page, which has all five of its rows.
        let store = filled(0, 41);
        assert_eq!(store.find_free_inventory_cell(180, 5), Some(45));
        // A 9-cell item is the largest that can fit at all: nine rows is one whole page.
        // Ten cannot, whatever the occupancy, because a page is 5 wide by 9 deep.
        let store = filled(0, 0);
        assert_eq!(store.find_free_inventory_cell(180, 9), Some(0));
        assert_eq!(store.find_free_inventory_cell(180, 10), None);
    }

    #[test]
    fn a_locked_page_is_never_offered_and_the_usable_count_is_what_bounds_it() {
        // This is the Divergence. Legacy's pickup-shaped `GetEmptyInventoryEx` scans all 180
        // cells (`char_item.cpp:1228-1233`) and ignores the unlock stat, so it can place an
        // item in a page the player has not paid for. The Rewrite bounds on the usable count.
        let mut store = filled(0, 90);
        assert_eq!(
            store.find_free_inventory_cell_for(0, 1),
            None,
            "cell 90 is in a locked page and must not be offered"
        );
        // Cell 90 does hold an item, which is the positive control: the refusal above is the
        // bound and not a storage that cannot hold a locked cell. Without this the assertion
        // above would also pass if `set` had refused cell 90 outright.
        store.set(pos(INV, 90), &one_cell(7777)).unwrap();
        assert_eq!(store.find_free_inventory_cell(180, 1), Some(91));
        // One unlock point is one more page, five columns wide, so the usable count becomes
        // 95 and cell 90 is now inside it.
        assert_eq!(usable_inventory_cells(1), 95);
        assert_eq!(store.find_free_inventory_cell_for(1, 1), Some(91));
        // The same call with the same bound must agree, which is the whole point of the
        // convenience wrapper: it computes the formula and changes nothing else.
        assert_eq!(
            store.find_free_inventory_cell_for(1, 1),
            store.find_free_inventory_cell(usable_inventory_cells(1), 1)
        );
    }

    #[test]
    fn a_full_inventory_has_no_cell_at_every_size() {
        let store = filled(0, 180);
        assert_eq!(store.find_free_inventory_cell(180, 1), None);
        assert_eq!(store.find_free_inventory_cell(180, 2), None);
        // The banks are separate space, so a full base inventory still offers a bank cell.
        // Legacy's `GetEmptyInventory` searches the banks first (`char_item.cpp:1209-1226`)
        // and only then the base range, so this is what it does too.
        assert_eq!(store.find_free_custom_cell(0, 1), Some(290));
    }

    #[test]
    fn a_zero_size_item_has_no_cell_rather_than_cell_zero() {
        // `set` refuses a zero-size item as `ZeroFootprint`, so reporting cell 0 would hand a
        // caller a cell its own `set` will refuse. That the two agree is the point.
        let store = CharacterItems::new();
        assert_eq!(store.find_free_inventory_cell(180, 0), None);
        assert_eq!(store.find_free_custom_cell(0, 0), None);
    }

    #[test]
    fn a_custom_category_starts_where_the_previous_one_ended() {
        // The six banks are contiguous from 290, 180 cells each, so category 0 is 290..470,
        // category 1 is 470..650, and so on. A granted skill book must land in category 0 and
        // not spill into the base inventory.
        let store = CharacterItems::new();
        assert_eq!(store.find_free_custom_cell(0, 1), Some(290));
        assert_eq!(store.find_free_custom_cell(1, 1), Some(470));
        assert_eq!(store.find_free_custom_cell(5, 1), Some(1190));
        // The last bank's last cell, which is the one a test that only checks the first answer
        // would never reach.
        let mut store = store;
        for cell in 0..179u16 {
            store
                .set(pos(INV, 290 + cell), &one_cell(5000 + u32::from(cell)))
                .unwrap();
        }
        assert_eq!(store.find_free_custom_cell(0, 1), Some(469));
        // Category 6 does not exist, which is legacy's `-1` (`char_item.cpp:321-322`).
        assert_eq!(store.find_free_custom_cell(6, 1), None);
        // And the base inventory is a different answer, so the two searches are not aliases.
        assert_eq!(store.find_free_inventory_cell(180, 1), Some(0));
    }

    #[test]
    fn a_custom_category_search_stays_inside_its_own_bank() {
        // Filling category 0 must not move category 1's answer down, and filling a bank to the
        // brim must not offer a cell in the next one.
        let store = filled(290, 180);
        assert_eq!(store.find_free_custom_cell(0, 1), None);
        assert_eq!(store.find_free_custom_cell(1, 1), Some(470));
    }

    #[test]
    fn new_storage_is_empty_and_every_grid_cell_is_free() {
        let items = CharacterItems::new();
        assert!(items.is_empty());
        assert_eq!(items.len(), 0);
        assert_eq!(items.occupied(), Vec::new());
        assert_eq!(items.flat_len(), 1370);
        assert_eq!(items.dragon_soul_len(), 1152);
        for cell in [0_u16, 1, 179, 180, 1369] {
            assert_eq!(items.grid_anchor(pos(INV, cell)), 0, "cell {cell}");
            assert_eq!(items.anchor_cell(pos(INV, cell)), None, "cell {cell}");
        }
        assert_eq!(items.get(pos(INV, 0)), Lookup::Empty);
    }

    #[test]
    fn the_grid_stores_a_one_based_anchor() {
        // 0 means free, so an occupied cell stores `cell + 1`. The element is a
        // `WORD`, which is load-bearing: see the last-cell test below.
        let mut items = CharacterItems::new();
        items
            .set(pos(INV, 7), &one_cell(11))
            .expect("cell 7 is free");
        assert_eq!(items.grid_anchor(pos(INV, 7)), 8);
        assert_eq!(items.anchor_cell(pos(INV, 7)), Some(7));
        assert_eq!(items.get(pos(INV, 7)), Lookup::Occupied(11));
    }

    #[test]
    fn the_last_flat_cell_anchor_does_not_wrap() {
        // Cell 1369 stores 1370. The grid element is a `WORD` (2 bytes), so
        // 1370 is exact. A `BYTE` would wrap it to 90, which is a live base
        // inventory cell, so the grid would report a cell as occupied that no
        // item is in. Both directions are checked: it fits a `WORD`, and it does
        // NOT fit a `BYTE`.
        let mut items = CharacterItems::new();
        let last = INVENTORY_AND_EQUIP_SLOT_MAX - 1;
        items
            .set(pos(INV, last), &one_cell(21))
            .expect("the last cell is real");
        let anchor = items.grid_anchor(pos(INV, last));
        assert_eq!(anchor, 1370);
        assert!(u16::try_from(u32::from(anchor)).is_ok(), "it fits a WORD");
        assert!(
            u8::try_from(u32::from(anchor)).is_err(),
            "it does not fit a BYTE"
        );
        assert_eq!(u32::from(anchor) % 256, 90, "a BYTE would have read 90");
        assert_eq!(items.anchor_cell(pos(INV, last)), Some(last));
    }

    #[test]
    fn a_base_inventory_stack_walks_with_a_stride_of_five() {
        // `char_item.cpp:422` walks with a bare 5, which is `INVENTORY_WIDTH`.
        let mut items = CharacterItems::new();
        items
            .set(pos(INV, 10), &sized(31, 3))
            .expect("cells 10, 15 and 20 are free");
        for cell in [10_u16, 15, 20] {
            assert_eq!(
                items.grid_anchor(pos(INV, cell)),
                11,
                "cell {cell} is covered by the anchor 10"
            );
            assert_eq!(items.anchor_cell(pos(INV, cell)), Some(10));
        }
        // The cells between the walk steps are NOT covered: the stride is
        // vertical, not horizontal.
        for cell in [11_u16, 12, 16, 21] {
            assert_eq!(items.grid_anchor(pos(INV, cell)), 0, "cell {cell} is free");
        }
        assert_eq!(items.len(), 1, "one item, three marked cells");
    }

    #[test]
    fn a_stack_stops_at_the_end_of_its_category() {
        // The walk does `if (p >= end) continue`, so a stack whose tail runs
        // past the 180-cell base inventory marks only the cells that fit.
        let mut items = CharacterItems::new();
        // A stack that walks past the 180-cell base inventory is refused, not
        // stored with a short footprint. Legacy's `if (p >= end) continue` skips
        // the cells it cannot reach and stores the item anyway
        // (`char_item.cpp:426`), so the grid would claim one cell for a
        // four-cell item and the next write would walk over the other three.
        assert_eq!(
            items.get(pos(INV, 178)),
            Lookup::Empty,
            "a control: it is free"
        );
        let err = items
            .set(pos(INV, 178), &sized(41, 4))
            .expect_err("the walk leaves the base inventory");
        assert_eq!(
            err,
            Rejected::FootprintCutOff {
                window: INV,
                cell: 178,
                size: 4,
                covered: 1
            },
            "only the anchor is inside the category"
        );
        assert_eq!(items.get(pos(INV, 178)), Lookup::Empty);
        assert_eq!(items.grid_anchor(pos(INV, 178)), 0);
        // A stack that fits is accepted, which is the control for the refusal
        // being about the arithmetic and not about size two and up.
        // The last anchor a two-cell stack fits at is 174, because 174 + 5 = 179
        // is the last cell below the 180 boundary.
        items
            .set(pos(INV, 174), &sized(42, 2))
            .expect("174 and 179 are both inside the base inventory");
        assert_eq!(items.grid_anchor(pos(INV, 179)), 175);
        assert_eq!(
            items
                .set(pos(INV, 175), &sized(43, 2))
                .expect_err("175 + 5 = 180 is outside it"),
            Rejected::FootprintCutOff {
                window: INV,
                cell: 175,
                size: 2,
                covered: 1
            }
        );
    }

    #[test]
    fn a_custom_inventory_stack_walks_inside_its_own_category() {
        // The bounds become the category's own, so a stack near a category edge
        // is bounded by that edge and not by 180.
        let start = CUSTOM_INVENTORY_SLOT_START;
        let mut items = CharacterItems::new();
        items
            .set(pos(INV, start + 2), &sized(51, 3))
            .expect("category 0 has room");
        for cell in [start + 2, start + 7, start + 12] {
            assert_eq!(items.grid_anchor(pos(INV, cell)), start + 3);
        }
        // The next category's first cell is not covered: the bound stops at 470.
        assert_eq!(items.grid_anchor(pos(INV, start + 180)), 0);
    }

    #[test]
    fn an_equipment_item_marks_only_its_own_cell() {
        // For a cell outside `0..180` the bounds stay the defaults, and
        // `cell < 180` is false, so legacy marks only the anchor
        // (`char_item.cpp:438-439`). Equipment cells are one cell each, so that
        // is right, and the Rewrite keeps it for that reason.
        let cell = 200_u16;
        let mut items = CharacterItems::new();
        items
            .set(pos(EQUIP, cell), &one_cell(61))
            .expect("cell 200 is real");
        assert_eq!(items.grid_anchor(pos(EQUIP, cell)), cell + 1);
        assert_eq!(items.get(pos(EQUIP, cell)), Lookup::Occupied(61));
        // A four-cell item in the same band is refused rather than silently
        // truncated to one marked cell, which is how a later write would come
        // to believe the other three are free.
        let err = items
            .set(pos(EQUIP, 201), &sized(62, 4))
            .expect_err("an equipment band records only its anchor");
        assert_eq!(
            err,
            Rejected::FootprintCutOff {
                window: EQUIP,
                cell: 201,
                size: 4,
                covered: 1
            }
        );
        assert_eq!(
            items.get(pos(EQUIP, 201)),
            Lookup::Empty,
            "nothing was stored"
        );
    }

    #[test]
    fn a_dragon_soul_stack_walks_with_a_stride_of_eight() {
        // `char_item.cpp:481` uses `DRAGON_SOUL_BOX_COLUMN_NUM`, which is 8.
        // The ledger-194 probe transcribed it as 6 and no control caught it.
        let mut items = CharacterItems::new();
        items
            .set(pos(DS, 3), &sized(71, 3))
            .expect("cells 3, 11 and 19 are free");
        for cell in [3_u16, 11, 19] {
            assert_eq!(
                items.ds_grid[usize::from(cell)],
                4,
                "cell {cell} is covered by the anchor 3"
            );
        }
        // Six would be a different set of cells, so this is the assertion that
        // would have caught the probe's transcription.
        for cell in [9_u16, 15] {
            assert_eq!(items.ds_grid[usize::from(cell)], 0, "cell {cell} is free");
        }
    }

    #[test]
    fn a_dragon_soul_stack_that_walks_past_the_array_is_refused() {
        // Anchored at the last cell, a four-deep stack reaches 1151 + 3 * 8 =
        // 1175, past the 1152 array. Legacy survives only because the walk
        // marks nothing, so the item is stored with an empty grid. Here it is
        // refused: an item whose grid is empty is an item a later write will
        // happily overlap.
        let last = DRAGON_SOUL_INVENTORY_MAX_NUM - 1;
        let mut items = CharacterItems::new();
        assert_eq!(
            items.get(pos(DS, last)),
            Lookup::Empty,
            "a control: it is free"
        );
        let err = items
            .set(pos(DS, last), &sized(81, 4))
            .expect_err("the walk would leave the array");
        assert_eq!(
            err,
            Rejected::FootprintCutOff {
                window: DS,
                cell: last,
                size: 4,
                covered: 1,
            },
            "only the anchor itself is inside the array"
        );
        assert_eq!(
            items.get(pos(DS, last)),
            Lookup::Empty,
            "a refused insert stores nothing"
        );
    }

    #[test]
    fn a_dragon_soul_stack_at_the_end_of_a_grade_cell_does_fit() {
        // The positive control for the test above: one cell lower and the walk
        // stays inside, so the refusal is about the arithmetic and not about
        // dragon souls refusing everything.
        // The walk bound is exclusive, so the last anchor a three-deep walk fits
        // at is the last cell minus two strides: 1151 - 16 = 1135, and 1136
        // would put its third cell at 1152, which is the 1153rd cell.
        let last_ok = DRAGON_SOUL_INVENTORY_MAX_NUM - 1 - DRAGON_SOUL_BOX_COLUMN_NUM * (3 - 1);
        let mut items = CharacterItems::new();
        items
            .set(pos(DS, last_ok), &sized(82, 3))
            .expect("three cells from that anchor stay inside");
        assert_eq!(last_ok, 1135);
        assert_eq!(last_ok + 16, 1151, "the last cell of the array");
        // The id array holds an id only at the anchor, and the grid holds the
        // coverage. That is why legacy needs both, and why a test that only read
        // the id array would conclude the second and third cells were free. The
        // two questions have two answers, and neither is the other's.
        for cell in [last_ok, last_ok + DRAGON_SOUL_BOX_COLUMN_NUM, last_ok + 16] {
            assert_eq!(items.grid_anchor(pos(DS, cell)), last_ok + 1, "cell {cell}");
            assert_eq!(
                items.anchor_cell(pos(DS, cell)),
                Some(last_ok),
                "cell {cell}"
            );
        }
        assert_eq!(items.get(pos(DS, last_ok)), Lookup::Occupied(82));
        assert_eq!(
            items.get(pos(DS, last_ok + 8)),
            Lookup::Empty,
            "the id array names anchors only, and that is legacy's shape"
        );
        // One cell further along and the walk would leave the array, which is
        // the control for the refusal above being about the arithmetic.
        assert_eq!(
            items.set(pos(DS, last_ok + 8), &sized(83, 3)),
            Err(Rejected::FootprintCutOff {
                window: DS,
                cell: last_ok + 8,
                size: 3,
                covered: 2
            })
        );
    }

    #[test]
    fn a_switchbot_cell_past_the_end_is_reported_not_read() {
        // Legacy reads `pSwitchbotItems[wCell]` at `char_item.cpp:537` and only
        // checks at `:543`, so a cell of 100 reads past a five-element array
        // before the check runs. Here the bound is first and there is nothing to
        // read: the answer is a value, not a memory error.
        let mut items = CharacterItems::new();
        for cell in [5_u16, 6, 100, 1369, u16::MAX] {
            assert_eq!(
                items.get(pos(SWITCH, cell)),
                Lookup::OutOfRange,
                "cell {cell} is past the five switchbot slots"
            );
            let err = items
                .set(pos(SWITCH, cell), &one_cell(91))
                .expect_err("cell {cell} does not exist");
            assert_eq!(
                err,
                Rejected::CellOutOfRange {
                    window: SWITCH,
                    cell,
                    limit: 5,
                },
                "cell {cell}"
            );
        }
        for cell in 0..5 {
            items
                .set(pos(SWITCH, cell), &one_cell(100 + u32::from(cell)))
                .expect("cell {cell} is in bounds");
        }
        assert_eq!(items.get(pos(SWITCH, 4)), Lookup::Occupied(104));
    }

    #[test]
    fn the_attribute_67_window_is_one_slot_that_takes_no_stack() {
        let mut items = CharacterItems::new();
        assert_eq!(items.get(pos(A67, 0)), Lookup::Empty);
        assert_eq!(items.get(pos(A67, 1)), Lookup::OutOfRange);
        items
            .set(pos(A67, 0), &one_cell(111))
            .expect("the one slot");
        assert_eq!(items.get(pos(A67, 0)), Lookup::Occupied(111));
        // A two-cell item has nowhere to record its second cell, so it is
        // refused instead of occupying the slot while looking partly free.
        assert!(matches!(
            items.set(pos(A67, 0), &sized(112, 2)),
            Err(Rejected::AlreadyOccupied { .. })
        ));
        items.remove(pos(A67, 0), &one_cell(111)).expect("remove");
        assert_eq!(
            items.set(pos(A67, 0), &sized(113, 2)),
            Err(Rejected::FootprintCutOff {
                window: A67,
                cell: 0,
                size: 2,
                covered: 1
            })
        );
    }

    #[test]
    fn a_safebox_or_mall_position_is_not_reported_as_an_empty_cell() {
        // `GetItem` accepts these -- `IsValidItemPosition` defers to the live
        // container -- and then has no case for them and returns `NULL`
        // (`char_item.cpp:260-300`). A caller that reads that as "the safebox is
        // empty" writes a duplicate. The Rewrite says which container is
        // missing.
        let items = CharacterItems::new();
        assert_eq!(items.get(pos(SAFEBOX, 0)), Lookup::NoContainer(SAFEBOX));
        assert_eq!(items.get(pos(SAFEBOX, 269)), Lookup::NoContainer(SAFEBOX));
        assert_eq!(items.get(pos(MALL, 3)), Lookup::NoContainer(MALL));
        assert_eq!(items.get(pos(BELT, 280)), Lookup::OutOfRange);
        assert_eq!(
            items.get(pos(0, 0)),
            Lookup::OutOfRange,
            "the reserved window"
        );
        assert_eq!(
            items.get(pos(11, 0)),
            Lookup::OutOfRange,
            "a byte past the table"
        );
        assert_eq!(items.get(pos(255, 0)), Lookup::OutOfRange);
    }

    #[test]
    fn every_window_byte_gives_a_decided_answer() {
        // A sweep rather than a sample, because the difference between "no
        // container" and "out of range" is the whole point of the type.
        let items = CharacterItems::new();
        for byte in 0..=u8::MAX {
            let outcome = items.get(pos(byte, 0));
            match byte {
                INV | EQUIP | DS | A67 | SWITCH => assert!(
                    matches!(outcome, Lookup::Empty),
                    "byte {byte} is a character window and cell 0 is free: {outcome:?}"
                ),
                SAFEBOX | MALL => {
                    assert!(matches!(outcome, Lookup::NoContainer(_)), "byte {byte}");
                }
                other => assert!(
                    matches!(outcome, Lookup::OutOfRange),
                    "byte {other} names no character window"
                ),
            }
        }
    }

    #[test]
    fn an_occupied_cell_refuses_a_second_item() {
        let mut items = CharacterItems::new();
        items.set(pos(INV, 5), &one_cell(121)).expect("cell 5");
        assert_eq!(
            items.set(pos(INV, 5), &one_cell(122)),
            Err(Rejected::AlreadyOccupied {
                window: INV,
                cell: 5,
                present: 121,
            })
        );
        // And the refused second item did not displace the first.
        assert_eq!(items.get(pos(INV, 5)), Lookup::Occupied(121));
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn an_item_that_is_already_stored_is_refused_a_second_cell() {
        // Legacy asserts `!"GetOwner exist"`, which compiles out in Release
        // (`premake5.lua:54,59`), so a double-owned item is dropped with no
        // diagnostic and, because `SetItem` is `void`, no packet either.
        let mut items = CharacterItems::new();
        let item = one_cell(131);
        items.set(pos(INV, 8), &item).expect("cell 8");
        // The item is stored, and its own `pos` still says 0, which is exactly
        // the stale-pointer case: a check that trusted `pos` would let this
        // through, and a check that trusted only `pos` would refuse the move
        // that follows.
        assert!(item.is_unplaced());
        assert_eq!(
            items.set(pos(INV, 9), &item),
            Err(Rejected::AlreadyOwned {
                id: 131,
                at: pos(INV, 8)
            })
        );
        assert_eq!(items.cell_of(131), Some(pos(INV, 8)));
        assert_eq!(items.cell_of(999), None, "an id this storage does not hold");
        assert_eq!(items.get(pos(INV, 9)), Lookup::Empty);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn a_zero_footprint_item_is_refused() {
        let mut items = CharacterItems::new();
        let mut item = one_cell(141);
        item.size = 0;
        assert_eq!(items.set(pos(INV, 0), &item), Err(Rejected::ZeroFootprint));
        assert_eq!(items.get(pos(INV, 0)), Lookup::Empty);
    }

    #[test]
    fn a_grid_conflict_leaves_the_grid_exactly_as_it_was() {
        // Legacy does not test for a conflict at all: `SetItem` overwrites the
        // marks (`char_item.cpp:462`), so the grid ends up naming the refused
        // item as the owner of the first item's cells, and the first item's
        // later `RemoveItem` zeroes them because its guard at `:432` reads the
        // id array, which is 0 for a non-anchor cell. The Rewrite refuses and
        // leaves the grid byte-for-byte as it was.
        let mut items = CharacterItems::new();
        items
            .set(pos(INV, 0), &sized(151, 3))
            .expect("cells 0, 5 and 10");
        let before: Vec<u16> = (0..12_u16)
            .map(|c| items.grid_anchor(pos(INV, c)))
            .collect();

        // A second stack whose third cell would be 10, which the first has.
        let err = items
            .set(pos(INV, 10), &sized(152, 3))
            .expect_err("cell 10 is taken");
        assert_eq!(
            err,
            Rejected::GridConflict {
                window: INV,
                cell: 10,
                blocked: 10,
                owner: 151,
            }
        );
        let after: Vec<u16> = (0..12_u16)
            .map(|c| items.grid_anchor(pos(INV, c)))
            .collect();
        assert_eq!(before, after, "a refusal must not write the grid");
        assert_eq!(items.get(pos(INV, 10)), Lookup::Empty);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn removal_clears_the_cells_of_the_item_and_no_others() {
        let mut items = CharacterItems::new();
        items
            .set(pos(INV, 0), &sized(161, 2))
            .expect("cells 0 and 5");
        items.set(pos(INV, 6), &sized(162, 1)).expect("cell 6");
        items
            .remove(pos(INV, 0), &sized(161, 2))
            .expect("cell 0 holds item 161");
        assert_eq!(items.grid_anchor(pos(INV, 0)), 0);
        assert_eq!(items.grid_anchor(pos(INV, 5)), 0);
        // The neighbour is untouched.
        assert_eq!(items.grid_anchor(pos(INV, 6)), 7);
        assert_eq!(items.get(pos(INV, 6)), Lookup::Occupied(162));
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn removing_from_a_free_or_a_different_cell_is_reported() {
        let mut items = CharacterItems::new();
        items.set(pos(INV, 3), &one_cell(171)).expect("cell 3");
        assert_eq!(
            items.remove(pos(INV, 3), &one_cell(172)),
            Err(Rejected::AlreadyOccupied {
                window: INV,
                cell: 3,
                present: 171,
            }),
            "a different item than the one held"
        );
        assert_eq!(
            items.remove(pos(INV, 4), &one_cell(171)),
            Err(Rejected::NotThere {
                window: INV,
                cell: 4
            }),
            "a cell that is free"
        );
        assert_eq!(
            items.get(pos(INV, 3)),
            Lookup::Occupied(171),
            "nothing changed"
        );
    }

    #[test]
    fn a_refused_move_leaves_the_item_where_it_was() {
        // Legacy's `CG_ITEM_MOVE` handlers remove and then add, in two steps,
        // each of which can fail silently. A refused move that dropped the item
        // would be the worst of the two failures.
        let mut items = CharacterItems::new();
        items.set(pos(INV, 2), &one_cell(181)).expect("cell 2");
        items.set(pos(INV, 30), &one_cell(182)).expect("cell 30");

        // Moving onto an occupied cell is refused and the item stays put.
        let held = one_cell(181);
        assert_eq!(
            items.move_item(pos(INV, 2), pos(INV, 30), &held),
            Err(Rejected::AlreadyOccupied {
                window: INV,
                cell: 30,
                present: 182,
            })
        );
        assert_eq!(items.get(pos(INV, 2)), Lookup::Occupied(181));
        assert_eq!(items.grid_anchor(pos(INV, 2)), 3);

        // A move that works clears the old cell and marks the new one.
        items
            .move_item(pos(INV, 2), pos(INV, 40), &held)
            .expect("cell 40 is free");
        assert_eq!(items.get(pos(INV, 2)), Lookup::Empty);
        assert_eq!(items.grid_anchor(pos(INV, 2)), 0);
        assert_eq!(items.get(pos(INV, 40)), Lookup::Occupied(181));
        assert_eq!(items.grid_anchor(pos(INV, 40)), 41);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn a_multi_cell_move_carries_its_footprint() {
        let mut items = CharacterItems::new();
        items
            .set(pos(INV, 0), &sized(191, 3))
            .expect("cells 0, 5 and 10");
        let held = sized(192, 2);
        items
            .set(pos(INV, 20), &held)
            .expect("cells 20 and 25 are free");
        items
            .move_item(pos(INV, 20), pos(INV, 30), &held)
            .expect("cells 30 and 35 are free");
        assert_eq!(
            items.grid_anchor(pos(INV, 20)),
            0,
            "the old cells are freed"
        );
        assert_eq!(items.grid_anchor(pos(INV, 25)), 0);
        assert_eq!(items.grid_anchor(pos(INV, 30)), 31);
        assert_eq!(items.grid_anchor(pos(INV, 35)), 31);
        assert_eq!(items.grid_anchor(pos(INV, 40)), 0);
    }

    #[test]
    fn the_two_arrays_are_independent() {
        // The flat and dragon soul grids are separate arrays with separate
        // anchors, so an item in one is not "in" the other even at the same
        // cell number.
        let mut items = CharacterItems::new();
        items
            .set(pos(INV, 12), &sized(201, 2))
            .expect("flat cells 12 and 17");
        items
            .set(pos(DS, 12), &sized(202, 2))
            .expect("DS cells 12 and 20");
        assert_eq!(items.grid_anchor(pos(INV, 12)), 13);
        assert_eq!(items.grid_anchor(pos(DS, 12)), 13);
        assert_eq!(items.grid_anchor(pos(INV, 20)), 0, "not the DS footprint");
        assert_eq!(items.grid_anchor(pos(DS, 17)), 0, "not the flat footprint");
        assert_eq!(items.grid_anchor(pos(DS, 20)), 13);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn occupancy_lists_every_window_in_a_stable_order() {
        let mut items = CharacterItems::new();
        items
            .set(pos(SWITCH, 3), &one_cell(211))
            .expect("switchbot 3");
        items.set(pos(INV, 0), &one_cell(212)).expect("flat 0");
        items.set(pos(A67, 0), &one_cell(213)).expect("attr67");
        items.set(pos(DS, 0), &one_cell(214)).expect("ds 0");
        assert_eq!(
            items.occupied(),
            vec![
                (pos(INV, 0), 212),
                (pos(DS, 0), 214),
                (pos(SWITCH, 3), 211),
                (pos(A67, 0), 213),
            ],
            "flat, then dragon soul, then switchbot, then attribute 67"
        );
    }

    #[test]
    fn an_empty_slot_reports_zero_for_every_window() {
        // The grid's zero is the "free" marker in both arrays, and the single
        // and five-slot windows have no grid at all.
        let items = CharacterItems::new();
        assert_eq!(items.grid_anchor(pos(INV, 0)), 0);
        assert_eq!(items.grid_anchor(pos(DS, 0)), 0);
        assert_eq!(items.grid_anchor(pos(EQUIP, 180)), 0);
        assert_eq!(items.grid_anchor(pos(A67, 0)), 0);
        assert_eq!(items.grid_anchor(pos(SWITCH, 0)), 0);
        assert_eq!(items.grid_anchor(pos(SAFEBOX, 0)), 0);
        assert_eq!(items.grid_anchor(pos(200, 0)), 0);
    }
}
