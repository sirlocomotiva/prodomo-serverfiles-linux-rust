//! The safebox and the item mall: `CSafebox` (`G/safebox.cpp`), and the arms of `CInputMain`
//! that act on one (`SafeboxCheckin`, `SafeboxCheckout`, `SafeboxItemMove`,
//! `G/input_main.cpp:2276-2473`).
//!
//! A [`Safebox`] is one open window of an account's storage (ADR-0005): the safebox proper, which
//! a password opens with [`SAFEBOX_ROWS`] rows, or the item mall, which opens with
//! [`MALL_ROWS`]. Both are five cells wide, and an item takes a column of cells as tall as its
//! size, as `CGrid(5, rows)` places it (`L/grid.cpp`).
//!
//! [`checkin`], [`checkout`] and [`move_stored`] are reducers, as [`super::move_item`] is: they
//! change the character's storage and the safebox, and answer with the records the client is sent
//! and the row changes the store has to make. Opening, closing and the password are the caller's.
//!
//! # The highlight
//!
//! `CItem::AddToCharacter` highlights an item whose last owner is another character
//! (`G/item.cpp:475-476`). A row loaded into a safebox has no last owner (`LoadSafebox` never sets
//! it), and a checkin keeps the depositor's, so an item taken out of the safebox it was put in
//! during the same opening is not highlighted, and every other is. A [`Safebox`] remembers which
//! of its items were deposited while it has been open.
//!
//! # Divergences
//!
//! - **A partial merge keeps its remainder.** `CSafebox::MoveItem` removes the source from the
//!   safebox before it takes the merged count off it (`G/safebox.cpp:216-220`), so the part that
//!   did not fit is left in no window and is lost, and a merge onto a full stack loses the whole
//!   source. Here a merge that moves every item destroys the source, one that moves some leaves
//!   the rest in its cell, and one that moves none changes nothing.
//! - **The merge count is 16 bits.** `CSafebox::MoveItem` takes a `BYTE` count, and the client
//!   sends a `WORD`; the count here is the whole `WORD`.
//! - **A loaded row that does not fit is skipped.** `CSafebox::Add` places a row whose cells are
//!   taken or run past the last row without marking the grid (`:69`, `Put`'s answer is
//!   ignored), so two items share a cell. [`Safebox::open`] skips such a row and answers it, for
//!   the caller to log.
//! - **A custom bank takes every item that belongs to it.** Legacy compares the bank with the
//!   item's first bank (`GetItemCategory`, `input_main.cpp:2392-2396`), the Defect
//!   [`super::move_item`] records as its second.
//! - **An item offered in a trade is refused.** Legacy cannot reach one, because the safebox and a
//!   trade exclude each other; the check is silent, as [`super::move_item`]'s is.
//! - **The set record carries highlight 0.** `CSafebox::Add` never writes it
//!   ([`protocol::gc_safebox`]).
//!
//! # Not ported
//!
//! A worn item, a dragon-soul item and an item outside the `INVENTORY` window cannot be put in,
//! and nothing is taken out into the dragon-soul inventory: each is [`Unported`], refused
//! silently. The item lock (`isLocked`), `CanHandleItem`, the running-quest check and the item log
//! are not ported either, nor are the six pages a premium account or the large-safebox item opens
//! ([`SAFEBOX_ROWS`]).

use std::collections::BTreeMap;

use common::item_slots::{
    EWindows, BELT_INVENTORY_SLOT_END, BELT_INVENTORY_SLOT_START, CUSTOM_INVENTORY_SLOT_END,
    CUSTOM_INVENTORY_SLOT_START,
};
use gamedata::belt_inventory::can_move_into_belt_inventory;
use gamedata::item_custom_category::is_custom_category;
use gamedata::item_kind::{ITEM_BELT, ITEM_DS};
use gamedata::item_proto::ItemProtos;
use protocol::gc_safebox::{GcStoreItemSet, StoreWindow};
use protocol::item_pos::ItemPos;

use super::dice::Dice;
use super::equip::roll_sash;
use super::inventory::{
    custom_inventory_category_of, is_belt_inventory_position, is_custom_inventory_position,
    is_equip_position,
};
use super::item_move::{
    ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, MoveRules, Unported,
};
use super::items::{CharacterItems, Rejected};
use super::quickslot::{QuickslotSync, SyncTo};
use crate::item::{gc_item_clear, Item, ItemId, ITEM_ANTIFLAG_SAFEBOX, ITEM_FLAG_IRREMOVABLE};

/// The width of a safebox: `CGrid(5, rows)` (`G/safebox.cpp:18`).
pub const SAFEBOX_WIDTH: u32 = 5;

/// `SAFEBOX_PAGE_SIZE`: the rows of one safebox page.
pub const SAFEBOX_PAGE_SIZE: u8 = 9;

/// The rows a safebox opens with: one page (`input_db.cpp:1135`), whatever its size column says.
/// The six pages of a premium account or the large-safebox item (`:1144-1150`) are not ported.
pub const SAFEBOX_ROWS: u8 = SAFEBOX_PAGE_SIZE;

/// The rows the item mall opens with: `3 * SAFEBOX_PAGE_SIZE` (`G/char.cpp:7224`, `LoadMall`).
pub const MALL_ROWS: u8 = 3 * SAFEBOX_PAGE_SIZE;

/// `UNIQUE_ITEM_SAFEBOX_EXPAND` (`G/unique_item.h:48`): the scroll that grows a safebox, which
/// may not go in one.
pub const UNIQUE_ITEM_SAFEBOX_EXPAND: u32 = 71_009;

/// The line `SafeboxCheckin` sends for an irremovable item outside the cells it may leave
/// (`input_main.cpp:2297`).
pub const IRREMOVABLE_NOTICE: &str =
    "@@tradus(input_main.cpp)dracu mai stie ce trebuia sa fie aici.";

/// The line `SafeboxCheckout` sends for a custom bank the item does not belong to
/// (`input_main.cpp:2394`).
pub const WRONG_BANK_NOTICE: &str = "Nu poti plasa acest obiect aici.";

/// A record about a safebox or mall cell, which only the owner is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreRecord {
    /// `GC_SAFEBOX_SET` or `GC_MALL_SET`: the cell holds this item (`CSafebox::Add`).
    Set(GcStoreItemSet),
    /// `GC_SAFEBOX_DEL` or `GC_MALL_DEL`: the cell is empty (`CSafebox::Remove`).
    Del {
        /// The window, which picks the header.
        window: StoreWindow,
        /// The cell.
        pos: u32,
    },
}

/// A change a move inside a safebox makes to the account's rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreChange {
    /// The item now sits in this cell of the same window.
    Moved {
        /// The item.
        id: ItemId,
        /// The cell.
        pos: u32,
    },
    /// The item's stack size changed.
    Count {
        /// The item.
        id: ItemId,
        /// Its new count.
        count: u16,
    },
    /// A merge used up this item.
    Destroyed {
        /// The item.
        id: ItemId,
    },
}

/// What a move inside a safebox did. The move's `changes` are empty; its row changes are
/// [`StoreMove::changes`], which name the account's rows rather than the character's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreMove {
    /// The outcome and the records.
    pub done: MoveDone,
    /// What the store has to change, in order.
    pub changes: Vec<StoreChange>,
}

/// Why a safebox step changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeboxRefused {
    /// No item is at the position the client named.
    Empty,
    /// The item's vnum has no prototype.
    UnknownVnum(u32),
    /// The item is offered in a trade.
    Exchanging,
    /// `ITEM_FLAG_IRREMOVABLE`, and the item is outside the cells it may leave.
    Irremovable,
    /// The safebox cells a checkin would take are taken, or run past the last row (`[LS;666]`).
    NoRoom,
    /// The cells a move inside the safebox would take are taken, or run past the last row.
    /// Legacy refuses it without a line (`G/safebox.cpp:226-227`).
    DestinationTaken,
    /// The item may not go in a safebox: the expansion scroll or `ITEM_ANTIFLAG_SAFEBOX`
    /// (`[LS;667]`).
    NotStorable,
    /// A belt whose inventory holds items (`[LS;1095]`, `@fixme140`).
    BeltNotEmpty,
    /// The custom bank does not take the item.
    WrongBank,
    /// The inventory cells are taken, or are not cells an item can be put in.
    CellTaken,
    /// The belt cell does not take the item (`[LS;1097]`, `@fixme119`).
    NotForBelt,
    /// A cell outside the safebox.
    InvalidPosition,
    /// More were asked for than the stack holds.
    CountAboveStack {
        /// The stack's count.
        held: u16,
        /// The count asked for.
        asked: u16,
    },
    /// The two stacks have different sockets.
    SocketsDiffer,
    /// The stack the item would join is full.
    StackFull,
    /// Only the safebox takes items in; this is the mall.
    NotASafebox,
    /// The character's storage refused the item.
    Storage(Rejected),
    /// A path this build does not port.
    NotPorted(Unported),
}

impl SafeboxRefused {
    /// The `CHAT_TYPE_INFO` line legacy sends for this refusal, if it sends one.
    #[must_use]
    pub const fn notice(&self) -> Option<&'static str> {
        match self {
            Self::Irremovable => Some(IRREMOVABLE_NOTICE),
            // `input_main.cpp:2312`.
            Self::NoRoom => Some("[LS;666]"),
            // `input_main.cpp:2318`, `:2324`, `:2330`.
            Self::NotStorable => Some("[LS;667]"),
            // `input_main.cpp:2353`.
            Self::BeltNotEmpty => Some("[LS;1095]"),
            Self::WrongBank => Some(WRONG_BANK_NOTICE),
            // `input_main.cpp:2441`.
            Self::NotForBelt => Some("[LS;1097]"),
            _ => None,
        }
    }
}

impl core::fmt::Display for SafeboxRefused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => f.write_str("no item is at the position"),
            Self::UnknownVnum(vnum) => write!(f, "vnum {vnum} has no prototype"),
            Self::Exchanging => f.write_str("the item is offered in a trade"),
            Self::Irremovable => f.write_str("the item may not leave this cell"),
            Self::NoRoom => f.write_str("the safebox cells are taken"),
            Self::DestinationTaken => f.write_str("the cells the item would move to are taken"),
            Self::NotStorable => f.write_str("the item may not go in a safebox"),
            Self::BeltNotEmpty => f.write_str("the belt's inventory is not empty"),
            Self::WrongBank => f.write_str("the item does not belong in that bank"),
            Self::CellTaken => f.write_str("the inventory cells are taken"),
            Self::NotForBelt => f.write_str("the item may not go in the belt"),
            Self::InvalidPosition => f.write_str("the cell is outside the safebox"),
            Self::CountAboveStack { held, asked } => {
                write!(f, "{asked} were asked for and the stack holds {held}")
            }
            Self::SocketsDiffer => f.write_str("the two stacks have different sockets"),
            Self::StackFull => f.write_str("the stack the item would join is full"),
            Self::NotASafebox => f.write_str("only the safebox takes items in"),
            Self::Storage(reason) => write!(f, "the storage refused: {reason}"),
            Self::NotPorted(what) => write!(f, "not ported yet: {what:?}"),
        }
    }
}

impl std::error::Error for SafeboxRefused {}

/// An item in a safebox, and whether it was deposited while the safebox has been open.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    item: Item,
    deposited: bool,
}

/// One open safebox or mall window of an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Safebox {
    window: StoreWindow,
    rows: u8,
    account: u32,
    /// The items by the cell they are anchored at (`m_pkItems`).
    held: BTreeMap<u32, Held>,
    /// Which cells an item covers (`m_pkGrid`).
    grid: Vec<bool>,
}

impl Safebox {
    /// Open `window` of `account` with `rows` rows and the items loaded for it, each at its
    /// [`Item::pos`] (`CHARACTER::LoadSafebox`, `LoadMall`).
    ///
    /// An item of another window, at a cell outside the safebox, or whose cells are taken or run
    /// past the last row, is skipped and answered, in load order, for the caller to log.
    #[must_use]
    pub fn open(
        window: StoreWindow,
        rows: u8,
        account: u32,
        items: Vec<Item>,
    ) -> (Self, Vec<Item>) {
        let cells = SAFEBOX_WIDTH * u32::from(rows);
        let mut safebox = Self {
            window,
            rows,
            account,
            held: BTreeMap::new(),
            grid: vec![false; usize::try_from(cells).unwrap_or(0)],
        };
        let mut skipped = Vec::new();
        for item in items {
            let pos = u32::from(item.pos.cell);
            if item.pos.window_type == window.window_type() && safebox.has_room(pos, item.size) {
                safebox.put(pos, item, false);
            } else {
                skipped.push(item);
            }
        }
        (safebox, skipped)
    }

    /// Which window this is.
    #[must_use]
    pub const fn window(&self) -> StoreWindow {
        self.window
    }

    /// How many rows it has.
    #[must_use]
    pub const fn rows(&self) -> u8 {
        self.rows
    }

    /// The store id of the account it belongs to.
    #[must_use]
    pub const fn account(&self) -> u32 {
        self.account
    }

    /// How many cells it has.
    #[must_use]
    pub fn cells(&self) -> u32 {
        SAFEBOX_WIDTH * u32::from(self.rows)
    }

    /// How many items it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.held.len()
    }

    /// Whether it holds no item.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// The item anchored at `pos` (`CSafebox::Get`). A cell another item covers answers `None`.
    #[must_use]
    pub fn get(&self, pos: u32) -> Option<&Item> {
        self.held.get(&pos).map(|held| &held.item)
    }

    /// The items in cell order.
    pub fn items(&self) -> impl Iterator<Item = &Item> + '_ {
        self.held.values().map(|held| &held.item)
    }

    /// The set records of every item, in cell order: what opening the window sends.
    #[must_use]
    pub fn records(&self) -> Vec<StoreRecord> {
        self.held
            .values()
            .map(|held| self.set_record(&held.item))
            .collect()
    }

    /// Whether an item of `size` fits at `pos`: `CGrid::IsEmpty(pos, 1, size)`
    /// (`L/grid.cpp:84-112`). The column it would take must lie inside the safebox and be free.
    #[must_use]
    pub fn has_room(&self, pos: u32, size: u8) -> bool {
        let row = pos / SAFEBOX_WIDTH;
        if row + u32::from(size) > u32::from(self.rows) {
            return false;
        }
        (0..u32::from(size)).all(|y| self.cell(pos + y * SAFEBOX_WIDTH) == Some(false))
    }

    /// Whether a cell is covered, or `None` outside the grid.
    fn cell(&self, index: u32) -> Option<bool> {
        let index = usize::try_from(index).ok()?;
        self.grid.get(index).copied()
    }

    /// Mark or clear the column an item of `size` at `pos` covers (`CGrid::Put` and `Get`),
    /// skipping the cells outside the grid.
    fn mark(&mut self, pos: u32, size: u8, covered: bool) {
        for y in 0..u32::from(size) {
            let index = usize::try_from(pos + y * SAFEBOX_WIDTH).ok();
            if let Some(cell) = index.and_then(|index| self.grid.get_mut(index)) {
                *cell = covered;
            }
        }
    }

    /// Place an item whose room [`Self::has_room`] has checked (`CSafebox::Add`).
    fn put(&mut self, pos: u32, mut item: Item, deposited: bool) {
        self.mark(pos, item.size, true);
        item.pos = ItemPos::new(
            self.window.window_type(),
            u16::try_from(pos).unwrap_or(u16::MAX),
        );
        self.held.insert(pos, Held { item, deposited });
    }

    /// Take the item anchored at `pos` out (`CSafebox::Remove`).
    fn take(&mut self, pos: u32) -> Option<Held> {
        let held = self.held.remove(&pos)?;
        self.mark(pos, held.item.size, false);
        Some(held)
    }

    /// The set record of an item where it is anchored.
    fn set_record(&self, item: &Item) -> StoreRecord {
        StoreRecord::Set(GcStoreItemSet {
            window: self.window,
            item: item.gc_item_set(item.pos, 0),
        })
    }
}

/// `CInputMain::SafeboxCheckin` (`input_main.cpp:2276-2366`): the item at `from` goes into the
/// safebox at `safe_pos`.
///
/// Legacy's checks run in its order, each refusal with its line. The item leaves its cell, the
/// slots that named the cell are deleted, and the safebox shows it.
///
/// # Errors
///
/// Returns why nothing changed.
pub fn checkin(
    safebox: &mut Safebox,
    items: &mut CharacterItems,
    from: ItemPos,
    safe_pos: u32,
    rules: MoveRules,
    protos: &ItemProtos,
) -> Result<MoveDone, SafeboxRefused> {
    if safebox.window != StoreWindow::Safebox {
        return Err(SafeboxRefused::NotASafebox);
    }
    let item = items.item_at(from).cloned().ok_or(SafeboxRefused::Empty)?;
    let proto = protos
        .get(item.vnum)
        .ok_or(SafeboxRefused::UnknownVnum(item.vnum))?;
    if items.is_exchanging(item.id) {
        return Err(SafeboxRefused::Exchanging);
    }
    let cell = from.cell;
    let pinned = (rules.usable_cells..CUSTOM_INVENTORY_SLOT_START).contains(&cell)
        || cell >= CUSTOM_INVENTORY_SLOT_END;
    if proto.flags & ITEM_FLAG_IRREMOVABLE != 0 && pinned {
        return Err(SafeboxRefused::Irremovable);
    }
    if !safebox.has_room(safe_pos, item.size) {
        return Err(SafeboxRefused::NoRoom);
    }
    if item.vnum == UNIQUE_ITEM_SAFEBOX_EXPAND || proto.anti_flags & ITEM_ANTIFLAG_SAFEBOX != 0 {
        return Err(SafeboxRefused::NotStorable);
    }
    // Legacy's `IsEquipped` branch, then `RemoveFromCharacter`, which takes a worn item off and
    // sends a dragon-soul item back to its own window.
    if is_equip_position(from) {
        return Err(SafeboxRefused::NotPorted(Unported::Equipment));
    }
    if proto.item_type == ITEM_DS {
        return Err(SafeboxRefused::NotPorted(Unported::DragonSoul));
    }
    if from.window_type != EWindows::Inventory as u8 {
        return Err(SafeboxRefused::NotPorted(Unported::SourceWindow(
            from.window_type,
        )));
    }
    let belt_loaded = (BELT_INVENTORY_SLOT_START..BELT_INVENTORY_SLOT_END).any(|cell| {
        items
            .item_at(ItemPos::new(from.window_type, cell))
            .is_some()
    });
    if proto.item_type == ITEM_BELT && belt_loaded {
        return Err(SafeboxRefused::BeltNotEmpty);
    }
    let left = items.release(item.id).map_err(SafeboxRefused::Storage)?;
    let id = item.id;
    safebox.put(safe_pos, item, true);
    let mut records = vec![
        MoveRecord::Item(ItemRecord::Set(gc_item_clear(left))),
        MoveRecord::QuickslotSync(QuickslotSync {
            from: cell,
            to: SyncTo::Delete,
        }),
    ];
    if let Some(stored) = safebox.get(safe_pos) {
        records.push(MoveRecord::Store(safebox.set_record(stored)));
    }
    Ok(MoveDone {
        kind: MoveKind::Stored,
        records,
        changes: vec![ItemChange::Stored {
            id,
            account: safebox.account,
            pos: safe_pos,
        }],
    })
}

/// `CInputMain::SafeboxCheckout` (`input_main.cpp:2368-2460`): the item at `safe_pos` goes to
/// `to` in the character's inventory.
///
/// Legacy's checks run in its order. The safebox forgets the item, and the character holds it,
/// highlighted unless it was deposited while the safebox has been open, a sash rolling its
/// absorption as `AddToCharacter` does.
///
/// # Errors
///
/// Returns why nothing changed.
pub fn checkout(
    safebox: &mut Safebox,
    items: &mut CharacterItems,
    safe_pos: u32,
    to: ItemPos,
    rules: MoveRules,
    protos: &ItemProtos,
    dice: &mut dyn Dice,
) -> Result<MoveDone, SafeboxRefused> {
    let held = safebox.held.get(&safe_pos).ok_or(SafeboxRefused::Empty)?;
    let (mut item, deposited) = (held.item.clone(), held.deposited);
    let proto = protos
        .get(item.vnum)
        .ok_or(SafeboxRefused::UnknownVnum(item.vnum))?;
    if is_custom_inventory_position(to)
        && !custom_inventory_category_of(to).is_some_and(|bank| is_custom_category(proto, bank))
    {
        return Err(SafeboxRefused::WrongBank);
    }
    // This build has no dragon-soul grid for `IsEmptyItemGrid` to read.
    if to.window_type == EWindows::DragonSoulInventory as u8 {
        return Err(SafeboxRefused::NotPorted(Unported::DragonSoul));
    }
    if !items.is_empty_item_grid(to, item.size, None, rules.usable_cells, rules.belt_grade) {
        return Err(SafeboxRefused::CellTaken);
    }
    if proto.item_type == ITEM_DS {
        return Err(SafeboxRefused::NotPorted(Unported::DragonSoul));
    }
    if is_belt_inventory_position(to) && !can_move_into_belt_inventory(proto) {
        return Err(SafeboxRefused::NotForBelt);
    }
    let rolled = roll_sash(&mut item, proto, dice);
    items.set(to, &item).map_err(SafeboxRefused::Storage)?;
    let _taken = safebox.take(safe_pos);
    let mut changes = vec![ItemChange::Retrieved {
        id: item.id,
        account: safebox.account,
        pos: to,
    }];
    if rolled {
        changes.push(ItemChange::Sockets {
            id: item.id,
            sockets: item.sockets,
        });
    }
    Ok(MoveDone {
        kind: MoveKind::Retrieved,
        records: vec![
            MoveRecord::Store(StoreRecord::Del {
                window: safebox.window,
                pos: safe_pos,
            }),
            MoveRecord::Item(ItemRecord::Set(item.gc_item_set(to, u8::from(!deposited)))),
        ],
        changes,
    })
}

/// `CSafebox::MoveItem` (`G/safebox.cpp:179-249`): the item anchored at `from` goes to `to`, or
/// `count` of it joins the stack anchored there. A `count` of 0 is the whole stack.
///
/// # Errors
///
/// Returns why nothing changed.
pub fn move_stored(
    safebox: &mut Safebox,
    from: u32,
    to: u32,
    count: u16,
    count_limit: u16,
) -> Result<StoreMove, SafeboxRefused> {
    let cells = safebox.cells();
    if from >= cells || to >= cells {
        return Err(SafeboxRefused::InvalidPosition);
    }
    let item = safebox.get(from).cloned().ok_or(SafeboxRefused::Empty)?;
    if item.count < count {
        return Err(SafeboxRefused::CountAboveStack {
            held: item.count,
            asked: count,
        });
    }
    let target = safebox
        .get(to)
        .filter(|target| target.id != item.id && target.stacks() && target.vnum == item.vnum)
        .cloned();
    if let Some(target) = target {
        return merge_stored(safebox, &item, &target, count, count_limit);
    }
    if !safebox.has_room(to, item.size) {
        return Err(SafeboxRefused::DestinationTaken);
    }
    let Some(held) = safebox.take(from) else {
        return Err(SafeboxRefused::Empty);
    };
    safebox.put(to, held.item, held.deposited);
    let mut records = vec![MoveRecord::Store(StoreRecord::Del {
        window: safebox.window,
        pos: from,
    })];
    if let Some(moved) = safebox.get(to) {
        records.push(MoveRecord::Store(safebox.set_record(moved)));
    }
    Ok(StoreMove {
        done: MoveDone {
            kind: MoveKind::Moved,
            records,
            changes: Vec::new(),
        },
        changes: vec![StoreChange::Moved {
            id: item.id,
            pos: to,
        }],
    })
}

/// The merge arm of `CSafebox::MoveItem` (`G/safebox.cpp:203-224`), with the remainder kept.
fn merge_stored(
    safebox: &mut Safebox,
    item: &Item,
    target: &Item,
    count: u16,
    count_limit: u16,
) -> Result<StoreMove, SafeboxRefused> {
    if target.sockets != item.sockets {
        return Err(SafeboxRefused::SocketsDiffer);
    }
    let asked = if count == 0 { item.count } else { count };
    let moved = count_limit.saturating_sub(target.count).min(asked);
    if moved == 0 {
        return Err(SafeboxRefused::StackFull);
    }
    let from = u32::from(item.pos.cell);
    let to = u32::from(target.pos.cell);
    let left = item.count - moved;
    let joined = target.count + moved;
    let mut records = Vec::with_capacity(2);
    let mut changes = Vec::with_capacity(2);
    if left == 0 {
        let _taken = safebox.take(from);
        records.push(MoveRecord::Store(StoreRecord::Del {
            window: safebox.window,
            pos: from,
        }));
        changes.push(StoreChange::Destroyed { id: item.id });
    } else if let Some(source) = safebox.held.get_mut(&from) {
        source.item.count = left;
        let record = safebox.set_record(&safebox.held[&from].item);
        records.push(MoveRecord::Store(record));
        changes.push(StoreChange::Count {
            id: item.id,
            count: left,
        });
    }
    if let Some(stack) = safebox.held.get_mut(&to) {
        stack.item.count = joined;
        records.push(MoveRecord::Item(ItemRecord::Update(
            stack.item.gc_item_update(),
        )));
        changes.push(StoreChange::Count {
            id: target.id,
            count: joined,
        });
    }
    Ok(StoreMove {
        done: MoveDone {
            kind: MoveKind::Merged,
            records,
            changes: Vec::new(),
        },
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{ITEM_ANTIFLAG_STACK, ITEM_FLAG_STACKABLE};
    use common::item_slots::{CUSTOM_INVENTORY_MAX_NUM, INVENTORY_MAX_NUM, ITEM_COUNT_LIMIT};
    use gamedata::item_kind::{COSTUME_SASH, ITEM_COSTUME};
    use gamedata::item_proto::ItemProto;

    const INV: u8 = EWindows::Inventory as u8;
    const SAFE: u8 = protocol::gc_item_window::window::SAFEBOX;
    const MALL: u8 = protocol::gc_item_window::window::MALL;
    const SWORD: u32 = 19;
    const TALL: u32 = 11_400;
    const POTION: u32 = 27_001;
    const PINNED: u32 = 50_001;
    const LOCKED: u32 = 50_002;
    const BELT: u32 = 18_000;
    const STONE: u32 = 110_000;
    const SASH: u32 = 85_004;
    /// In bank 0 by its vnum.
    const BANKED: u32 = 50_301;
    /// In bank 0 by its vnum, and in bank 2 as a piece of leather.
    const LEATHER: u32 = 55_003;
    const NO_PROTO: u32 = 4;
    const LIMIT: u16 = 200;
    const RULES: MoveRules = MoveRules {
        count_limit: LIMIT,
        usable_cells: 90,
        belt_grade: None,
    };

    struct Fixed(u32);

    impl Dice for Fixed {
        fn random31(&mut self) -> u32 {
            self.0
        }
    }

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(INV, cell)
    }

    fn protos() -> ItemProtos {
        let mut tall = ItemProto::for_category_rule(TALL, 2, 0);
        tall.size = 3;
        let mut pinned = ItemProto::for_category_rule(PINNED, 0, 0);
        pinned.flags = ITEM_FLAG_IRREMOVABLE;
        let mut locked = ItemProto::for_category_rule(LOCKED, 0, 0);
        locked.anti_flags = ITEM_ANTIFLAG_SAFEBOX;
        let mut sash = ItemProto::for_category_rule(SASH, ITEM_COSTUME, COSTUME_SASH);
        sash.values[0] = 4;
        ItemProtos::from_rows(vec![
            ItemProto::for_category_rule(SWORD, 1, 0),
            tall,
            ItemProto::for_category_rule(POTION, 3, 0),
            pinned,
            locked,
            ItemProto::for_category_rule(UNIQUE_ITEM_SAFEBOX_EXPAND, 0, 0),
            ItemProto::for_category_rule(BELT, ITEM_BELT, 0),
            ItemProto::for_category_rule(STONE, ITEM_DS, 0),
            sash,
            ItemProto::for_category_rule(BANKED, 0, 0),
            // `ITEM_MATERIAL`, `MATERIAL_LEATHER`.
            ItemProto::for_category_rule(LEATHER, 5, 0),
        ])
    }

    fn item(id: u32, vnum: u32, size: u8) -> Item {
        let mut item = Item::new(id, vnum);
        item.size = size;
        item
    }

    fn stack(id: u32, count: u16) -> Item {
        let mut item = item(id, POTION, 1);
        item.flags = ITEM_FLAG_STACKABLE;
        item.count = count;
        item
    }

    fn stored(mut item: Item, pos: u16) -> Item {
        item.pos = ItemPos::new(SAFE, pos);
        item
    }

    fn safebox(items: Vec<Item>) -> Safebox {
        let (safebox, skipped) = Safebox::open(StoreWindow::Safebox, SAFEBOX_ROWS, 7, items);
        assert!(skipped.is_empty(), "{skipped:?}");
        safebox
    }

    fn holding(placed: &[(u16, Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (cell, item) in placed {
            items.set(inv(*cell), item).expect("the fixture places");
        }
        items
    }

    fn set(window: StoreWindow, item: &Item, pos: u16) -> MoveRecord {
        let mut shown = item.clone();
        shown.pos = ItemPos::new(window.window_type(), pos);
        MoveRecord::Store(StoreRecord::Set(GcStoreItemSet {
            window,
            item: shown.gc_item_set(shown.pos, 0),
        }))
    }

    fn del(pos: u32) -> MoveRecord {
        MoveRecord::Store(StoreRecord::Del {
            window: StoreWindow::Safebox,
            pos,
        })
    }

    #[test]
    fn the_windows_have_legacys_sizes() {
        assert_eq!(SAFEBOX_ROWS, 9);
        assert_eq!(MALL_ROWS, 27);
        let (mall, _) = Safebox::open(StoreWindow::Mall, MALL_ROWS, 7, Vec::new());
        assert_eq!(mall.cells(), 135);
        assert_eq!(safebox(Vec::new()).cells(), 45);
        assert!(safebox(Vec::new()).is_empty());
    }

    #[test]
    fn an_item_takes_a_column_as_tall_as_its_size() {
        let tall = safebox(vec![stored(item(1, TALL, 3), 1)]);
        assert_eq!(tall.len(), 1);
        for covered in [1, 6, 11] {
            assert!(!tall.has_room(covered, 1), "{covered}");
            let anchored = tall.get(covered).map(|item| item.id);
            assert_eq!(anchored, (covered == 1).then_some(1));
        }
        for free in [0, 2, 16, 44] {
            assert!(tall.has_room(free, 1), "{free}");
        }
        // Three tall from cell 21 (row 4) reaches row 6; from 31 (row 6) it would pass row 8.
        assert!(tall.has_room(21, 3));
        assert!(tall.has_room(30, 3));
        assert!(!tall.has_room(35, 3));
        assert!(!tall.has_room(44, 2));
        assert!(!tall.has_room(45, 1));
        assert!(!tall.has_room(u32::MAX, 1));
        // A column that meets another item's lower cell has no room.
        assert!(!tall.has_room(u32::from(1_u8), 1));
        assert!(!safebox(Vec::new()).has_room(40, 2));
        assert!(safebox(Vec::new()).has_room(40, 1));
        assert!(safebox(Vec::new()).has_room(3, 0));
    }

    #[test]
    fn room_is_looked_for_down_the_column_not_along_the_row() {
        // `CSafebox::IsEmpty` asks the grid for a box one cell wide (`G/safebox.cpp:140-145`).
        let below = safebox(vec![stored(item(1, SWORD, 1), 6)]);
        assert!(!below.has_room(1, 2));
        let beside = safebox(vec![stored(item(1, SWORD, 1), 2)]);
        assert!(beside.has_room(1, 2));
    }

    #[test]
    fn a_loaded_row_that_does_not_fit_is_skipped_in_load_order() {
        let rows = vec![
            stored(item(1, TALL, 3), 0),
            stored(item(2, SWORD, 1), 5),
            stored(item(3, SWORD, 1), 44),
            stored(item(4, TALL, 3), 39),
            stored(item(5, SWORD, 1), 45),
            item(6, SWORD, 1),
            {
                // A row of the other window, at a cell that is free.
                let mut mall = item(7, SWORD, 1);
                mall.pos = ItemPos::new(MALL, 2);
                mall
            },
        ];
        let (safebox, skipped) = Safebox::open(StoreWindow::Safebox, SAFEBOX_ROWS, 7, rows);
        assert_eq!(
            safebox.items().map(|item| item.id).collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(
            skipped.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![2, 4, 5, 6, 7]
        );
        assert_eq!(safebox.account(), 7);
        assert_eq!(safebox.rows(), SAFEBOX_ROWS);
        assert_eq!(safebox.window(), StoreWindow::Safebox);
    }

    #[test]
    fn opening_shows_every_item_in_cell_order_without_a_highlight() {
        let mut sword = item(2, SWORD, 1);
        sword.count = 1;
        let (mall, _) = Safebox::open(
            StoreWindow::Mall,
            MALL_ROWS,
            7,
            vec![
                {
                    let mut late = item(3, SWORD, 1);
                    late.pos = ItemPos::new(MALL, 100);
                    late
                },
                {
                    let mut early = sword.clone();
                    early.pos = ItemPos::new(MALL, 4);
                    early
                },
            ],
        );
        let records = mall.records();
        assert_eq!(records.len(), 2);
        let StoreRecord::Set(first) = records[0] else {
            panic!("a set record");
        };
        assert_eq!(first.header(), 128);
        assert_eq!(first.item.cell, ItemPos::new(MALL, 4));
        assert_eq!(first.item.highlight, 0);
        assert_eq!(mall.get(100).map(|item| item.id), Some(3));
    }

    #[test]
    fn a_checkin_clears_the_cell_syncs_the_slots_and_shows_the_item() {
        let mut items = holding(&[(3, item(9, SWORD, 1))]);
        let mut safebox = safebox(Vec::new());
        let done = checkin(&mut safebox, &mut items, inv(3), 12, RULES, &protos())
            .expect("the checkin is taken");
        assert_eq!(done.kind, MoveKind::Stored);
        assert_eq!(
            done.records,
            vec![
                MoveRecord::Item(ItemRecord::Set(gc_item_clear(inv(3)))),
                MoveRecord::QuickslotSync(QuickslotSync {
                    from: 3,
                    to: SyncTo::Delete
                }),
                set(StoreWindow::Safebox, &item(9, SWORD, 1), 12),
            ]
        );
        assert_eq!(
            done.changes,
            vec![ItemChange::Stored {
                id: 9,
                account: 7,
                pos: 12
            }]
        );
        assert!(items.item_at(inv(3)).is_none());
        assert_eq!(
            safebox.get(12).map(|item| item.pos),
            Some(ItemPos::new(SAFE, 12))
        );
        assert!(!safebox.has_room(12, 1));
    }

    fn refused_checkin(
        items: &mut CharacterItems,
        safebox: &mut Safebox,
        from: ItemPos,
        safe_pos: u32,
    ) -> SafeboxRefused {
        let before = (items.clone(), safebox.clone());
        let refused = checkin(safebox, items, from, safe_pos, RULES, &protos())
            .expect_err("the checkin is refused");
        assert_eq!((items.clone(), safebox.clone()), before, "{refused}");
        refused
    }

    #[test]
    fn every_refused_checkin_changes_nothing_and_says_legacys_line() {
        let mut safebox = safebox(vec![stored(item(1, SWORD, 1), 0)]);
        let mut items = holding(&[
            (0, item(2, SWORD, 1)),
            (1, item(3, TALL, 3)),
            (2, item(4, UNIQUE_ITEM_SAFEBOX_EXPAND, 1)),
            (3, item(5, LOCKED, 1)),
            (4, item(6, NO_PROTO, 1)),
            (13, item(7, PINNED, 1)),
            (95, item(8, PINNED, 1)),
            (90, item(13, PINNED, 1)),
            (89, item(14, PINNED, 1)),
            (7, item(9, STONE, 1)),
            (8, item(10, BELT, 1)),
            (BELT_INVENTORY_SLOT_START, item(11, SWORD, 1)),
            (CUSTOM_INVENTORY_SLOT_START, item(12, PINNED, 1)),
        ]);
        let mut refuse = |from: u16, safe_pos: u32| {
            refused_checkin(&mut items, &mut safebox, inv(from), safe_pos)
        };
        assert_eq!(refuse(10, 5), SafeboxRefused::Empty);
        assert_eq!(refuse(4, 5), SafeboxRefused::UnknownVnum(NO_PROTO));
        assert_eq!(refuse(0, 0), SafeboxRefused::NoRoom);
        assert_eq!(refuse(1, 35), SafeboxRefused::NoRoom);
        assert_eq!(refuse(2, 5), SafeboxRefused::NotStorable);
        assert_eq!(refuse(3, 5), SafeboxRefused::NotStorable);
        // Pinned past the unlocked cells; inside them it may go.
        assert_eq!(refuse(95, 5), SafeboxRefused::Irremovable);
        assert_eq!(refuse(95, 0), SafeboxRefused::Irremovable);
        // The first locked cell pins it; the last unlocked one does not.
        assert_eq!(refuse(90, 5), SafeboxRefused::Irremovable);
        assert_eq!(
            refuse(7, 5),
            SafeboxRefused::NotPorted(Unported::DragonSoul)
        );
        assert_eq!(refuse(8, 5), SafeboxRefused::BeltNotEmpty);
        assert_eq!(SafeboxRefused::NoRoom.notice(), Some("[LS;666]"));
        assert_eq!(SafeboxRefused::NotStorable.notice(), Some("[LS;667]"));
        assert_eq!(SafeboxRefused::BeltNotEmpty.notice(), Some("[LS;1095]"));
        assert_eq!(
            SafeboxRefused::Irremovable.notice(),
            Some(IRREMOVABLE_NOTICE)
        );
        assert_eq!(SafeboxRefused::Empty.notice(), None);
        assert!(checkin(&mut safebox, &mut items, inv(13), 5, RULES, &protos()).is_ok());
        assert!(checkin(&mut safebox, &mut items, inv(89), 7, RULES, &protos()).is_ok());
        // A custom bank is not a cell an irremovable item is pinned to (`input_main.cpp:2295`).
        let bank = inv(CUSTOM_INVENTORY_SLOT_START);
        assert!(checkin(&mut safebox, &mut items, bank, 6, RULES, &protos()).is_ok());
    }

    #[test]
    fn a_custom_bank_takes_every_item_that_belongs_to_it() {
        // Legacy takes the leather only into bank 0, its first (`input_main.cpp:2392-2396`).
        let mut safebox = safebox(vec![
            stored(item(1, BANKED, 1), 0),
            stored(item(2, LEATHER, 1), 1),
        ]);
        let mut items = CharacterItems::new();
        let second_bank = CUSTOM_INVENTORY_SLOT_START + 2 * CUSTOM_INVENTORY_MAX_NUM;
        for (safe_pos, cell) in [(0, CUSTOM_INVENTORY_SLOT_START), (1, second_bank)] {
            let done = checkout(
                &mut safebox,
                &mut items,
                safe_pos,
                inv(cell),
                RULES,
                &protos(),
                &mut Fixed(0),
            )
            .expect("the bank takes it");
            assert_eq!(done.kind, MoveKind::Retrieved);
            assert!(items.item_at(inv(cell)).is_some(), "{cell}");
        }
        assert!(safebox.is_empty());
    }

    #[test]
    fn a_belt_goes_in_once_its_inventory_is_empty() {
        let mut items = holding(&[(8, item(10, BELT, 1))]);
        let mut safebox = safebox(Vec::new());
        assert!(checkin(&mut safebox, &mut items, inv(8), 5, RULES, &protos()).is_ok());
    }

    #[test]
    fn a_worn_offered_or_foreign_window_item_is_not_taken_in() {
        let mut items = holding(&[(0, item(2, SWORD, 1))]);
        let worn = ItemPos::new(INV, INVENTORY_MAX_NUM + 4);
        items
            .set(worn, &item(3, SWORD, 1))
            .expect("the fixture wears");
        let mut safebox = safebox(Vec::new());
        assert_eq!(
            refused_checkin(&mut items, &mut safebox, worn, 5),
            SafeboxRefused::NotPorted(Unported::Equipment)
        );
        assert!(items.set_exchanging(2, true));
        assert_eq!(
            refused_checkin(&mut items, &mut safebox, inv(0), 5),
            SafeboxRefused::Exchanging
        );
        let (mut mall, _) = Safebox::open(StoreWindow::Mall, MALL_ROWS, 7, Vec::new());
        assert_eq!(
            refused_checkin(&mut items, &mut mall, inv(0), 5),
            SafeboxRefused::NotASafebox
        );
    }

    #[test]
    fn a_checkout_takes_the_item_out_highlighted_unless_deposited_this_opening() {
        let mut items = holding(&[(0, item(2, SWORD, 1))]);
        let mut safebox = safebox(vec![stored(item(1, SWORD, 1), 4)]);
        checkin(&mut safebox, &mut items, inv(0), 9, RULES, &protos()).expect("in");
        let loaded = checkout(
            &mut safebox,
            &mut items,
            4,
            inv(5),
            RULES,
            &protos(),
            &mut Fixed(0),
        )
        .expect("the loaded item comes out");
        assert_eq!(loaded.kind, MoveKind::Retrieved);
        let mut shown = item(1, SWORD, 1);
        shown.pos = inv(5);
        assert_eq!(
            loaded.records,
            vec![
                del(4),
                MoveRecord::Item(ItemRecord::Set(shown.gc_item_set(inv(5), 1))),
            ]
        );
        assert_eq!(
            loaded.changes,
            vec![ItemChange::Retrieved {
                id: 1,
                account: 7,
                pos: inv(5)
            }]
        );
        let deposited = checkout(
            &mut safebox,
            &mut items,
            9,
            inv(6),
            RULES,
            &protos(),
            &mut Fixed(0),
        )
        .expect("the deposited item comes out");
        let MoveRecord::Item(ItemRecord::Set(set)) = deposited.records[1] else {
            panic!("an item set");
        };
        assert_eq!(set.highlight, 0);
        assert!(safebox.is_empty());
        assert!(safebox.has_room(4, 1) && safebox.has_room(9, 1));
        assert_eq!(items.item_at(inv(5)).map(|item| item.id), Some(1));
        assert_eq!(items.item_at(inv(6)).map(|item| item.id), Some(2));
    }

    #[test]
    fn a_sash_rolls_as_it_comes_out_and_its_sockets_follow_the_move() {
        let mut items = CharacterItems::new();
        let mut safebox = safebox(vec![stored(item(1, SASH, 1), 0)]);
        let done = checkout(
            &mut safebox,
            &mut items,
            0,
            inv(0),
            RULES,
            &protos(),
            &mut Fixed(0),
        )
        .expect("out");
        let rolled = items.item_at(inv(0)).expect("held").sockets;
        assert_ne!(rolled[0], 0);
        assert_eq!(
            done.changes,
            vec![
                ItemChange::Retrieved {
                    id: 1,
                    account: 7,
                    pos: inv(0)
                },
                ItemChange::Sockets {
                    id: 1,
                    sockets: rolled
                },
            ]
        );
    }

    #[test]
    fn every_refused_checkout_changes_nothing_and_says_legacys_line() {
        let mut safebox = safebox(vec![
            stored(item(1, SWORD, 1), 0),
            stored(item(2, STONE, 1), 1),
            stored(item(3, NO_PROTO, 1), 2),
            stored(item(4, TALL, 3), 3),
            stored(item(5, BANKED, 1), 4),
        ]);
        let mut items = holding(&[(0, item(9, SWORD, 1)), (15, item(10, SWORD, 1))]);
        let mut refuse = |safe_pos: u32, to: ItemPos| {
            let before = (items.clone(), safebox.clone());
            let refused = checkout(
                &mut safebox,
                &mut items,
                safe_pos,
                to,
                RULES,
                &protos(),
                &mut Fixed(0),
            )
            .expect_err("refused");
            assert_eq!((items.clone(), safebox.clone()), before, "{refused}");
            refused
        };
        assert_eq!(refuse(7, inv(1)), SafeboxRefused::Empty);
        assert_eq!(refuse(2, inv(1)), SafeboxRefused::UnknownVnum(NO_PROTO));
        assert_eq!(refuse(0, inv(0)), SafeboxRefused::CellTaken);
        assert_eq!(refuse(3, inv(5)), SafeboxRefused::CellTaken);
        assert_eq!(refuse(0, inv(95)), SafeboxRefused::CellTaken);
        assert_eq!(refuse(0, inv(INVENTORY_MAX_NUM)), SafeboxRefused::CellTaken);
        assert_eq!(
            refuse(0, inv(CUSTOM_INVENTORY_SLOT_START)),
            SafeboxRefused::WrongBank
        );
        // The leather's bank takes no item that belongs to bank 0 alone.
        let leather_bank = CUSTOM_INVENTORY_SLOT_START + 2 * CUSTOM_INVENTORY_MAX_NUM;
        assert_eq!(refuse(4, inv(leather_bank)), SafeboxRefused::WrongBank);
        assert_eq!(
            refuse(1, inv(1)),
            SafeboxRefused::NotPorted(Unported::DragonSoul)
        );
        assert_eq!(
            refuse(0, ItemPos::new(EWindows::DragonSoulInventory as u8, 0)),
            SafeboxRefused::NotPorted(Unported::DragonSoul)
        );
        assert_eq!(SafeboxRefused::WrongBank.notice(), Some(WRONG_BANK_NOTICE));
        assert_eq!(SafeboxRefused::NotForBelt.notice(), Some("[LS;1097]"));
        assert_eq!(SafeboxRefused::CellTaken.notice(), None);
    }

    #[test]
    fn a_belt_cell_takes_only_what_may_go_in_the_belt() {
        let mut safebox = safebox(vec![stored(item(1, TALL, 1), 0)]);
        let mut items = CharacterItems::new();
        let rules = MoveRules {
            belt_grade: Some(7),
            ..RULES
        };
        let belt = inv(BELT_INVENTORY_SLOT_START);
        assert_eq!(
            checkout(
                &mut safebox,
                &mut items,
                0,
                belt,
                rules,
                &protos(),
                &mut Fixed(0)
            ),
            Err(SafeboxRefused::NotForBelt)
        );
    }

    #[test]
    fn a_move_to_a_free_column_takes_the_item_there() {
        let mut safebox = safebox(vec![stored(item(1, TALL, 2), 0)]);
        let moved = move_stored(&mut safebox, 0, 30, 0, LIMIT).expect("moved");
        assert_eq!(moved.done.kind, MoveKind::Moved);
        assert_eq!(
            moved.done.records,
            vec![del(0), set(StoreWindow::Safebox, &item(1, TALL, 2), 30)]
        );
        assert!(moved.done.changes.is_empty());
        assert_eq!(moved.changes, vec![StoreChange::Moved { id: 1, pos: 30 }]);
        assert!(safebox.has_room(0, 2));
        assert!(!safebox.has_room(35, 1));
    }

    #[test]
    fn a_refused_move_changes_nothing() {
        let mut safebox = safebox(vec![
            stored(item(1, TALL, 2), 0),
            stored(stack(2, 5), 1),
            stored(stack(3, LIMIT), 2),
            {
                let mut other = stack(4, 5);
                other.sockets[2] = 9;
                stored(other, 3)
            },
            {
                let mut lone = stack(6, 5);
                lone.anti_flags = ITEM_ANTIFLAG_STACK;
                stored(lone, 4)
            },
        ]);
        let before = safebox.clone();
        let mut refuse = |from: u32, to: u32, count: u16| {
            let refused = move_stored(&mut safebox, from, to, count, LIMIT).expect_err("refused");
            assert_eq!(safebox, before, "{refused}");
            refused
        };
        assert_eq!(refuse(45, 0, 0), SafeboxRefused::InvalidPosition);
        assert_eq!(refuse(0, 45, 0), SafeboxRefused::InvalidPosition);
        assert_eq!(refuse(10, 20, 0), SafeboxRefused::Empty);
        assert_eq!(
            refuse(1, 20, 6),
            SafeboxRefused::CountAboveStack { held: 5, asked: 6 }
        );
        // Onto itself, onto its own lower cell, onto another item, past the last row.
        assert_eq!(refuse(0, 0, 0), SafeboxRefused::DestinationTaken);
        assert_eq!(refuse(0, 5, 0), SafeboxRefused::DestinationTaken);
        assert_eq!(refuse(1, 0, 0), SafeboxRefused::DestinationTaken);
        assert_eq!(refuse(0, 40, 0), SafeboxRefused::DestinationTaken);
        assert_eq!(refuse(1, 3, 0), SafeboxRefused::SocketsDiffer);
        assert_eq!(refuse(1, 2, 0), SafeboxRefused::StackFull);
        // A stack that refuses merging is only an item in the way.
        assert_eq!(refuse(1, 4, 0), SafeboxRefused::DestinationTaken);
        // `SafeboxItemMove` ignores what `MoveItem` answers, so the player is told nothing.
        assert_eq!(SafeboxRefused::DestinationTaken.notice(), None);
    }

    #[test]
    fn a_whole_merge_destroys_the_source_and_updates_the_stack() {
        let mut safebox = safebox(vec![stored(stack(1, 5), 0), stored(stack(2, 7), 6)]);
        let merged = move_stored(&mut safebox, 0, 6, 0, LIMIT).expect("merged");
        assert_eq!(merged.done.kind, MoveKind::Merged);
        let mut joined = stored(stack(2, 12), 6);
        joined.count = 12;
        assert_eq!(
            merged.done.records,
            vec![
                del(0),
                MoveRecord::Item(ItemRecord::Update(joined.gc_item_update())),
            ]
        );
        assert_eq!(
            merged.changes,
            vec![
                StoreChange::Destroyed { id: 1 },
                StoreChange::Count { id: 2, count: 12 },
            ]
        );
        assert_eq!(safebox.len(), 1);
        assert!(safebox.has_room(0, 1));
        assert_eq!(merged.done.records[1], {
            let MoveRecord::Item(ItemRecord::Update(update)) = merged.done.records[1] else {
                panic!("an update");
            };
            assert_eq!(update.cell, ItemPos::new(SAFE, 6));
            merged.done.records[1]
        });
    }

    #[test]
    fn a_partial_merge_keeps_the_rest_where_it_was() {
        let mut safebox = safebox(vec![stored(stack(1, 50), 0), stored(stack(2, 180), 6)]);
        let merged = move_stored(&mut safebox, 0, 6, 30, LIMIT).expect("merged");
        assert_eq!(
            merged.done.records[0],
            set(StoreWindow::Safebox, &stack(1, 30), 0)
        );
        assert_eq!(
            merged.changes,
            vec![
                StoreChange::Count { id: 1, count: 30 },
                StoreChange::Count { id: 2, count: 200 },
            ]
        );
        assert_eq!(safebox.get(0).map(|item| item.count), Some(30));
        assert_eq!(safebox.get(6).map(|item| item.count), Some(200));
    }

    #[test]
    fn an_asked_count_below_the_room_moves_only_that_many() {
        let mut safebox = safebox(vec![stored(stack(1, 50), 0), stored(stack(2, 10), 6)]);
        let merged = move_stored(&mut safebox, 0, 6, 15, LIMIT).expect("merged");
        assert_eq!(
            merged.changes,
            vec![
                StoreChange::Count { id: 1, count: 35 },
                StoreChange::Count { id: 2, count: 25 },
            ]
        );
    }

    #[test]
    fn a_rest_smaller_than_the_moved_count_stays_where_it_was() {
        let (from, to) = (stored(stack(1, 10), 0), stored(stack(2, LIMIT - 8), 6));
        let mut safebox = safebox(vec![from, to]);
        let merged = move_stored(&mut safebox, 0, 6, 0, LIMIT).expect("merged");
        assert_eq!(
            merged.changes,
            vec![
                StoreChange::Count { id: 1, count: 2 },
                StoreChange::Count {
                    id: 2,
                    count: LIMIT
                },
            ]
        );
        assert_eq!(safebox.get(0).map(|item| item.count), Some(2));
    }

    #[test]
    fn an_asked_count_past_a_byte_is_not_cut_to_one() {
        // `CSafebox::MoveItem` takes the `WORD` count as a `BYTE` (`G/safebox.cpp:179-181`), a
        // Defect not reproduced: 300 would be 44 there.
        let (from, to) = (stored(stack(1, 400), 0), stored(stack(2, 10), 6));
        let mut safebox = safebox(vec![from, to]);
        let merged = move_stored(&mut safebox, 0, 6, 300, ITEM_COUNT_LIMIT).expect("merged");
        assert_eq!(
            merged.changes,
            vec![
                StoreChange::Count { id: 1, count: 100 },
                StoreChange::Count { id: 2, count: 310 },
            ]
        );
    }

    #[test]
    fn a_moved_item_keeps_whether_it_was_deposited() {
        let mut items = holding(&[(0, item(2, SWORD, 1))]);
        let mut safebox = safebox(Vec::new());
        checkin(&mut safebox, &mut items, inv(0), 0, RULES, &protos()).expect("in");
        move_stored(&mut safebox, 0, 1, 0, LIMIT).expect("moved");
        let out = checkout(
            &mut safebox,
            &mut items,
            1,
            inv(0),
            RULES,
            &protos(),
            &mut Fixed(0),
        )
        .expect("out");
        let MoveRecord::Item(ItemRecord::Set(set)) = out.records[1] else {
            panic!("an item set");
        };
        assert_eq!(set.highlight, 0);
    }

    #[test]
    fn every_refusal_reads_as_a_sentence() {
        for refused in [
            SafeboxRefused::Empty,
            SafeboxRefused::UnknownVnum(4),
            SafeboxRefused::Exchanging,
            SafeboxRefused::Irremovable,
            SafeboxRefused::NoRoom,
            SafeboxRefused::DestinationTaken,
            SafeboxRefused::NotStorable,
            SafeboxRefused::BeltNotEmpty,
            SafeboxRefused::WrongBank,
            SafeboxRefused::CellTaken,
            SafeboxRefused::NotForBelt,
            SafeboxRefused::InvalidPosition,
            SafeboxRefused::CountAboveStack { held: 1, asked: 2 },
            SafeboxRefused::SocketsDiffer,
            SafeboxRefused::StackFull,
            SafeboxRefused::NotASafebox,
            SafeboxRefused::NotPorted(Unported::DragonSoul),
        ] {
            assert!(!refused.to_string().is_empty());
        }
    }
}
