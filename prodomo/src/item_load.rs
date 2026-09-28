//! Loading a character's items at character select.
//!
//! There is no client record that asks for an inventory. `CG_CHARACTER_SELECT` makes legacy
//! ask its DB process for the character, and the item rows arrive in the loading phase, where
//! `CInputDB::ItemLoad` (`G/input_db.cpp:1451-1567`) places each one and `CHARACTER::SetItem`
//! writes one 72-byte `GC_ITEM_SET` for it (`G/char_item.cpp:566-594`). Ledger 209.1 has the
//! search that shows the `CG_ITEM_LOAD` name belongs to the retired DB-peer protocol.
//!
//! [`plan_item_load`](crate::item_load::plan_item_load) is that placement as a transport-free reducer: it takes the rows the store
//! returned and answers with the items placed, the records to send, and the rows it refused. It
//! places into a scratch [`CharacterItems`](world::character::CharacterItems), because the character is not in the world yet;
//! [`GameState::enter_world`](crate::game_state::GameState::enter_world) later places the same
//! items, in the same order, into the character the world creates.
//!
//! # What legacy does, in order
//!
//! 1. For each row: `CreateItem(vnum, count, id)`, and a row whose vnum has no prototype is
//!    skipped without a word (`:1472-1478`).
//! 2. A `BELT_INVENTORY` row becomes `INVENTORY` at `pos + BELT_INVENTORY_SLOT_START`
//!    (`:1491-1495`).
//! 3. An `INVENTORY` row whose cell already holds an item is set aside (`:1498-1503`).
//! 4. Otherwise `AddToCharacter` places it where the row says (`:1506-1532`).
//! 5. The set-aside items go to the first free base-inventory cell, or onto the ground when
//!    there is none (`:1541-1561`). They are sent after every directly placed item.
//! 6. `CheckMaximumPoints` and a second `PointsPacket` (`:1563-1564`).
//!
//! The highlight byte of every record is 0: `AddToCharacter`'s default `bHighlight = true` is
//! replaced by `GetLastOwnerPID() != ch->GetPlayerID()` (`G/item.cpp:475-476`), and the load
//! has just set the last owner to the owner (`:1483`).
//!
//! `flags` and `anti_flags` come from the prototype, not the row: `CItem::SetProto` copies
//! `dwFlags` into `m_lFlag` (`G/item.cpp:221-226`), `GetAntiFlag` reads `dwAntiFlags` from the
//! prototype (`G/item.h:78`), and the legacy item table stores neither.
//!
//! # Where the Rewrite differs
//!
//! Each of these is a legacy Defect or a system not ported yet, and each refusal leaves the row
//! in the store untouched, so nothing a player owns is lost by being refused here.
//!
//! - **A refused row is reported.** Legacy skips an unknown vnum and an unknown window without
//!   a log line, and `SetItem` returns early for a cell it will not take while
//!   `AddToCharacter` saves the item as placed. Here every refusal is a [`LoadRefusal`](crate::item_load::LoadRefusal) the
//!   caller logs.
//! - **A wear cell in the `INVENTORY` window is refused.** `AddToCharacter` means to refuse
//!   cells 180 to 273 but tests `m_wCell`, the new item's cell, which is still 0
//!   (`G/item.cpp:447-453`), so the test never fires. The Rewrite applies the test it meant.
//! - **Equipment, the switchbot, and the attribute window are not loaded yet.** An
//!   `EQUIPMENT` row goes through `EquipTo`, which applies the item's bonuses and needs the
//!   level check; a `SWITCHBOT` row registers with the switchbot manager; neither system is
//!   ported, so those rows are refused as [`Refused::WindowNotPorted`](crate::item_load::Refused::WindowNotPorted) and left in the store.
//! - **Any overlap sets an item aside, not only an occupied anchor.** Legacy tests only the
//!   exact cell, so an item whose footprint overlaps a neighbour is placed over it and its
//!   marks overwrite the neighbour's. Here a [`Rejected::GridConflict`](world::character::Rejected::GridConflict) is set aside like an
//!   occupied cell.
//! - **A set-aside item that finds no room is refused, not dropped.** Legacy drops it on the
//!   ground with a three-minute owner and a destroy event, and the row is never rewritten.
//!   The ground is not ported, so the row stays where it was.
//! - **The order is the store's `ORDER BY window_type, pos, id`**. Legacy's order is a hash
//!   set's on a cache hit and an unordered `SELECT` on a miss; every record carries its cell,
//!   so the client cannot observe either.

use common::item_slots::{
    EWindows, BELT_INVENTORY_SLOT_COUNT, BELT_INVENTORY_SLOT_START, INVENTORY_MAX_NUM,
};
use db::items::ItemRow;
use gamedata::item_proto::ItemProtos;
use protocol::gc_item_window::{GcItemSet, ItemAttribute};
use protocol::item_pos::ItemPos;
use world::character::{CharacterItems, Rejected};
use world::item::{CountRejected, Item};

/// The highlight byte of a loaded item's record: the last owner is the owner.
const LOAD_HIGHLIGHT: u8 = 0;

/// One item the load placed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedItem {
    /// The item, with its prototype's size and flags.
    pub item: Item,
    /// Where it was placed, in the window and cell the client is told.
    pub pos: ItemPos,
    /// The row's own window and cell when the item was set aside and moved, so the caller
    /// writes the new cell; `None` when it stayed where the row said.
    pub moved_from: Option<(u8, u32)>,
}

impl LoadedItem {
    /// The `GC_ITEM_SET` record for this item.
    #[must_use]
    pub fn record(&self) -> GcItemSet {
        self.item.gc_item_set(self.pos, LOAD_HIGHLIGHT)
    }
}

/// Why a row was not loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The vnum has no prototype. Legacy skips the row silently.
    UnknownVnum,
    /// The prototype's `bSize` is not a positive footprint.
    NoFootprint,
    /// The stored count is not a count an item can have.
    BadCount(CountRejected),
    /// The window belongs to a system this build has not ported, or is not a window an
    /// owned item can be in.
    WindowNotPorted,
    /// An `INVENTORY` row names a cell in the equipment range.
    WearCell,
    /// The storage would not take the item.
    Rejected(Rejected),
    /// The item was set aside and no base-inventory cell had room for it.
    NoRoom,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownVnum => formatter.write_str("the vnum has no item prototype"),
            Self::NoFootprint => formatter.write_str("the prototype has no footprint"),
            Self::BadCount(error) => write!(formatter, "the count is refused: {error}"),
            Self::WindowNotPorted => {
                formatter.write_str("items in this window are not loaded by this build")
            }
            Self::WearCell => formatter.write_str("the cell is in the equipment range"),
            Self::Rejected(reason) => write!(formatter, "the inventory refused it: {reason:?}"),
            Self::NoRoom => formatter.write_str("its cell was taken and no free cell was found"),
        }
    }
}

/// One row the load refused. The row stays in the store as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadRefusal {
    /// The row's item id.
    pub id: u32,
    /// The row's window.
    pub window: u8,
    /// The row's cell.
    pub pos: u32,
    /// Why.
    pub why: Refused,
}

/// What the load decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemLoad {
    /// The placed items, in the order their records are sent: the items placed where their
    /// row said, then the ones that were set aside.
    pub placed: Vec<LoadedItem>,
    /// The rows that were not loaded.
    pub refused: Vec<LoadRefusal>,
}

impl ItemLoad {
    /// The rows of the items that were set aside and moved, each with its new cell.
    ///
    /// Legacy writes these: `AddToCharacter` ends with `Save()` (`G/item.cpp:529`), and
    /// saving is back on by the time the set-aside items are placed (`:1538`). Every
    /// other column is the row's own. A row placed where it said is not rewritten, and
    /// that includes a belt row shown in the inventory window: legacy places those with
    /// saving off (`:1480`), so the row keeps its belt window.
    #[must_use]
    pub fn moved_rows(&self, rows: &[ItemRow]) -> Vec<ItemRow> {
        self.placed
            .iter()
            .filter(|placed| placed.moved_from.is_some())
            .filter_map(|placed| {
                let row = rows.iter().find(|row| row.id == placed.item.id)?;
                Some(ItemRow {
                    window_type: placed.pos.window_type,
                    pos: u32::from(placed.pos.cell),
                    ..row.clone()
                })
            })
            .collect()
    }
}

/// Place a character's stored items the way `CInputDB::ItemLoad` does.
///
/// `inven_point` is the character's `Inven_Point`, which bounds the base-inventory search a
/// set-aside item goes through (ledger 198.3).
#[must_use]
pub fn plan_item_load(rows: &[ItemRow], protos: &ItemProtos, inven_point: u16) -> ItemLoad {
    let mut storage = CharacterItems::new();
    let mut load = ItemLoad::default();
    let mut set_aside = Vec::new();
    for row in rows {
        let refuse = |why| LoadRefusal {
            id: row.id,
            window: row.window_type,
            pos: row.pos,
            why,
        };
        let item = match item_from_row(row, protos) {
            Ok(item) => item,
            Err(why) => {
                load.refused.push(refuse(why));
                continue;
            }
        };
        let pos = match load_position(row) {
            Ok(pos) => pos,
            Err(why) => {
                load.refused.push(refuse(why));
                continue;
            }
        };
        match storage.set(pos, &item) {
            Ok(()) => load.placed.push(LoadedItem {
                item,
                pos,
                moved_from: None,
            }),
            Err(Rejected::AlreadyOccupied { .. } | Rejected::GridConflict { .. })
                if pos.window_type == EWindows::Inventory as u8 =>
            {
                set_aside.push((row, item));
            }
            Err(reason) => load.refused.push(refuse(Refused::Rejected(reason))),
        }
    }
    for (row, item) in set_aside {
        let placed = storage
            .find_free_inventory_cell_for(inven_point, item.size())
            .map(|cell| ItemPos::new(EWindows::Inventory as u8, cell))
            .ok_or(Refused::NoRoom)
            .and_then(|pos| {
                storage
                    .set(pos, &item)
                    .map(|()| pos)
                    .map_err(Refused::Rejected)
            });
        match placed {
            Ok(pos) => load.placed.push(LoadedItem {
                item,
                pos,
                moved_from: Some((row.window_type, row.pos)),
            }),
            Err(why) => load.refused.push(LoadRefusal {
                id: row.id,
                window: row.window_type,
                pos: row.pos,
                why,
            }),
        }
    }
    load
}

/// The item a row describes, with its prototype's size and flags.
fn item_from_row(row: &ItemRow, protos: &ItemProtos) -> Result<Item, Refused> {
    let proto = protos.get(row.vnum).ok_or(Refused::UnknownVnum)?;
    let mut item = Item::new(row.id, row.vnum);
    let size = u8::try_from(proto.size).map_err(|_| Refused::NoFootprint)?;
    item.set_size(size).map_err(|_| Refused::NoFootprint)?;
    item.set_count(u32::from(row.count))
        .map_err(Refused::BadCount)?;
    item.refine_element = row.refine_element;
    item.transmutation = row.transmutation;
    item.flags = proto.flags;
    item.anti_flags = proto.anti_flags;
    item.sockets = row.sockets;
    item.attributes = std::array::from_fn(|index| {
        ItemAttribute::new(row.attributes[index].b_type, row.attributes[index].s_value)
    });
    Ok(item)
}

/// Where a row's item goes, after legacy's belt translation.
fn load_position(row: &ItemRow) -> Result<ItemPos, Refused> {
    let cell = |pos: u32| u16::try_from(pos).map_err(|_| Refused::WindowNotPorted);
    match EWindows::try_from(row.window_type) {
        Ok(EWindows::Inventory) => {
            let cell = cell(row.pos)?;
            if (INVENTORY_MAX_NUM..BELT_INVENTORY_SLOT_START).contains(&cell) {
                return Err(Refused::WearCell);
            }
            Ok(ItemPos::new(row.window_type, cell))
        }
        Ok(EWindows::BeltInventory) => {
            let cell = cell(row.pos)?;
            if cell >= BELT_INVENTORY_SLOT_COUNT {
                return Err(Refused::Rejected(Rejected::CellOutOfRange {
                    window: row.window_type,
                    cell,
                    limit: BELT_INVENTORY_SLOT_COUNT,
                }));
            }
            Ok(ItemPos::new(
                EWindows::Inventory as u8,
                BELT_INVENTORY_SLOT_START + cell,
            ))
        }
        Ok(EWindows::DragonSoulInventory) => Ok(ItemPos::new(row.window_type, cell(row.pos)?)),
        _ => Err(Refused::WindowNotPorted),
    }
}

/// The window and position a stored row gives an item the world holds at `pos`.
///
/// The inverse of the load's belt translation, and `CItemManager::SaveSingleItem`'s
/// (`G/item_manager.cpp:503-520`): an `INVENTORY` cell in the belt range is stored as
/// `BELT_INVENTORY` at `cell - BELT_INVENTORY_SLOT_START`, because `ENABLE_BELT_INVENTORY_EX`
/// is defined. Every other position is stored as the world holds it. The `EQUIPMENT` arm
/// (`pos = cell - INVENTORY_MAX_NUM`) is not here, because no item reaches a wear cell until
/// equipping is ported, and the load refuses one.
#[must_use]
pub fn stored_row_position(pos: ItemPos) -> (u8, u32) {
    let belt = BELT_INVENTORY_SLOT_START..BELT_INVENTORY_SLOT_START + BELT_INVENTORY_SLOT_COUNT;
    if pos.window_type == EWindows::Inventory as u8 && belt.contains(&pos.cell) {
        return (
            EWindows::BeltInventory as u8,
            u32::from(pos.cell - BELT_INVENTORY_SLOT_START),
        );
    }
    (pos.window_type, u32::from(pos.cell))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::item_slots::CUSTOM_INVENTORY_SLOT_START;

    fn owners() -> ItemProtos {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        ItemProtos::load(&dir).expect("the owner's item protos load")
    }

    /// A vnum of the owner's data with this size, and with flags set when asked.
    fn a_vnum(protos: &ItemProtos, size: i32, flagged: bool) -> u32 {
        protos
            .rows()
            .iter()
            .find(|proto| {
                proto.size == size && (proto.flags != 0 && proto.anti_flags != 0) == flagged
            })
            .map(|proto| proto.vnum)
            .expect("the owner's data has such a prototype")
    }

    fn row(id: u32, window: u8, pos: u32, vnum: u32) -> ItemRow {
        ItemRow {
            id,
            owner_id: Some(7),
            window_type: window,
            pos,
            vnum,
            count: 1,
            refine_element: 0,
            transmutation: 0,
            flags: 0,
            anti_flags: 0,
            sockets: [0; 6],
            attributes: std::array::from_fn(|_| db::items::Attribute {
                b_type: 0,
                s_value: 0,
            }),
        }
    }

    const INVENTORY: u8 = EWindows::Inventory as u8;

    /// A placed item's id and cell, and the window and cell it was set aside from.
    type PlacedAt = (u32, u16, Option<(u8, u32)>);

    #[test]
    fn a_row_is_placed_where_it_says_with_the_prototype_flags_and_no_highlight() {
        let protos = owners();
        let vnum = a_vnum(&protos, 1, true);
        let proto = protos.get(vnum).expect("the vnum was just found");
        let mut stored = row(0x0102_0304, INVENTORY, 7, vnum);
        stored.count = 0x0123;
        stored.refine_element = 0x0506_0708;
        stored.transmutation = 0x090A_0B0C;
        stored.flags = 0xDEAD;
        stored.anti_flags = 0xBEEF;
        stored.sockets = [1, -2, 0x0304_0506, 4, 5, 6];
        stored.attributes[6] = db::items::Attribute {
            b_type: 0x21,
            s_value: -0x0102,
        };
        let load = plan_item_load(&[stored], &protos, 0);
        assert!(load.refused.is_empty(), "{:?}", load.refused);
        let [placed] = load.placed.as_slice() else {
            panic!("one item: {:?}", load.placed);
        };
        assert_eq!(placed.pos, ItemPos::new(INVENTORY, 7));
        assert_eq!(placed.moved_from, None);
        let record = placed.record();
        assert_eq!(record.cell, ItemPos::new(INVENTORY, 7));
        assert_eq!(record.vnum, vnum);
        assert_eq!(record.count, 0x0123);
        assert_eq!(record.refine_element, 0x0506_0708);
        assert_eq!(record.transmutation, 0x090A_0B0C);
        assert_eq!(record.flags, proto.flags, "flags come from the prototype");
        assert_eq!(record.anti_flags, proto.anti_flags);
        assert_eq!(record.highlight, 0);
        assert_eq!(record.sockets, [1, -2, 0x0304_0506, 4, 5, 6]);
        assert_eq!(record.attributes[6], ItemAttribute::new(0x21, -0x0102));
        assert_eq!(record.attributes[0], ItemAttribute::new(0, 0));
    }

    #[test]
    fn a_belt_row_travels_as_the_inventory_window() {
        let protos = owners();
        let vnum = a_vnum(&protos, 1, false);
        let load = plan_item_load(
            &[
                row(1, EWindows::BeltInventory as u8, 3, vnum),
                row(2, EWindows::BeltInventory as u8, 16, vnum),
            ],
            &protos,
            0,
        );
        assert_eq!(load.placed.len(), 1);
        assert_eq!(load.placed[0].pos, ItemPos::new(INVENTORY, 277));
        assert_eq!(load.placed[0].moved_from, None);
        assert_eq!(load.refused.len(), 1);
        assert_eq!(load.refused[0].id, 2);
        assert!(matches!(
            load.refused[0].why,
            Refused::Rejected(Rejected::CellOutOfRange { cell: 16, .. })
        ));
    }

    #[test]
    fn custom_and_dragon_soul_rows_pass_through() {
        let protos = owners();
        let vnum = a_vnum(&protos, 1, false);
        let custom = u32::from(CUSTOM_INVENTORY_SLOT_START) + 45;
        let dragon_soul = EWindows::DragonSoulInventory as u8;
        let load = plan_item_load(
            &[
                row(1, INVENTORY, custom, vnum),
                row(2, dragon_soul, 9, vnum),
            ],
            &protos,
            0,
        );
        assert!(load.refused.is_empty(), "{:?}", load.refused);
        let cells: Vec<ItemPos> = load.placed.iter().map(|placed| placed.pos).collect();
        assert_eq!(
            cells,
            [
                ItemPos::new(INVENTORY, CUSTOM_INVENTORY_SLOT_START + 45),
                ItemPos::new(dragon_soul, 9)
            ]
        );
    }

    #[test]
    fn a_taken_cell_moves_the_item_to_the_first_free_cell_after_the_others() {
        let protos = owners();
        let small = a_vnum(&protos, 1, false);
        let tall = a_vnum(&protos, 2, false);
        // Cell 0 holds a 1-cell item, and a 2-cell item at cell 5 is overlapped by nothing.
        // Row 3 names the occupied cell 0, and row 4 names cell 10, which the 2-cell item's
        // lower cell covers: legacy tests only the anchor and would place it over it.
        let mut rows = [
            row(1, INVENTORY, 0, small),
            row(2, INVENTORY, 5, tall),
            row(3, INVENTORY, 0, small),
            row(4, INVENTORY, 10, small),
            row(5, INVENTORY, 20, small),
        ];
        rows[2].sockets = [0x0102, 0, 0, 0, 0, -3];
        let load = plan_item_load(&rows, &protos, 0);
        let rewritten = load.moved_rows(&rows);
        assert_eq!(
            rewritten[0],
            ItemRow {
                pos: 1,
                ..rows[2].clone()
            },
            "only the cell changes"
        );
        assert!(load.refused.is_empty(), "{:?}", load.refused);
        let moved: Vec<(u32, u8, u32)> = load
            .moved_rows(&rows)
            .iter()
            .map(|moved| (moved.id, moved.window_type, moved.pos))
            .collect();
        assert_eq!(moved, [(3, INVENTORY, 1), (4, INVENTORY, 2)]);
        let order: Vec<PlacedAt> = load
            .placed
            .iter()
            .map(|placed| (placed.item.id, placed.pos.cell, placed.moved_from))
            .collect();
        assert_eq!(
            order,
            [
                (1, 0, None),
                (2, 5, None),
                (5, 20, None),
                (3, 1, Some((INVENTORY, 0))),
                (4, 2, Some((INVENTORY, 10))),
            ]
        );
    }

    #[test]
    fn a_belt_row_set_aside_is_rewritten_in_the_inventory_window() {
        let protos = owners();
        let small = a_vnum(&protos, 1, false);
        // The store's cell key is per window, so an `INVENTORY` row on cell 277 and a belt row
        // on cell 3 can both exist, and both name `INVENTORY` 277 once the belt row is
        // translated. The store's order puts window 1 first, so the belt row is the one set
        // aside, and it moves to the base inventory: its rewritten row is in window 1, not 9.
        let rows = [
            row(1, INVENTORY, 277, small),
            row(2, EWindows::BeltInventory as u8, 3, small),
        ];
        let load = plan_item_load(&rows, &protos, 0);
        assert!(load.refused.is_empty(), "{:?}", load.refused);
        assert_eq!(
            load.placed[1].moved_from,
            Some((EWindows::BeltInventory as u8, 3))
        );
        assert_eq!(
            load.moved_rows(&rows),
            [ItemRow {
                window_type: INVENTORY,
                pos: 0,
                ..rows[1].clone()
            }]
        );
    }

    #[test]
    fn a_set_aside_item_with_no_room_is_refused_and_kept() {
        let protos = owners();
        let small = a_vnum(&protos, 1, false);
        // `Inven_Point` 0 buys 90 usable cells; fill them, then offer one more on cell 0.
        let mut rows: Vec<ItemRow> = (0..90)
            .map(|cell| row(cell + 1, INVENTORY, cell, small))
            .collect();
        rows.push(row(1000, INVENTORY, 0, small));
        let load = plan_item_load(&rows, &protos, 0);
        assert_eq!(load.placed.len(), 90);
        assert_eq!(
            load.refused,
            [LoadRefusal {
                id: 1000,
                window: INVENTORY,
                pos: 0,
                why: Refused::NoRoom
            }]
        );
    }

    #[test]
    fn rows_this_build_cannot_load_are_refused_and_named() {
        let protos = owners();
        let small = a_vnum(&protos, 1, false);
        let mut zero = row(8, INVENTORY, 3, small);
        zero.count = 0;
        let rows = [
            row(1, INVENTORY, 1, 0xFFFF_FFF0),
            row(2, EWindows::Equipment as u8, 4, small),
            row(3, EWindows::Switchbot as u8, 0, small),
            row(4, EWindows::Attr67Add as u8, 0, small),
            row(5, EWindows::Safebox as u8, 0, small),
            row(6, INVENTORY, 180, small),
            row(7, INVENTORY, 273, small),
            zero,
            row(9, INVENTORY, 1370, small),
            row(10, INVENTORY, 70_000, small),
            row(11, 11, 0, small),
            row(12, INVENTORY, 179, small),
            row(13, INVENTORY, 274, small),
        ];
        let load = plan_item_load(&rows, &protos, 0);
        let placed: Vec<u32> = load.placed.iter().map(|placed| placed.item.id).collect();
        assert_eq!(
            placed,
            [12, 13],
            "the cells either side of the wear range load"
        );
        let refused: Vec<(u32, &Refused)> = load
            .refused
            .iter()
            .map(|refusal| (refusal.id, &refusal.why))
            .collect();
        assert_eq!(refused[0], (1, &Refused::UnknownVnum));
        for (index, id) in [(1, 2), (2, 3), (3, 4), (4, 5), (9, 10), (10, 11)] {
            assert_eq!(refused[index], (id, &Refused::WindowNotPorted), "row {id}");
        }
        assert_eq!(refused[5], (6, &Refused::WearCell));
        assert_eq!(refused[6], (7, &Refused::WearCell));
        assert_eq!(refused[7], (8, &Refused::BadCount(CountRejected::Zero)));
        assert!(matches!(
            refused[8],
            (
                9,
                Refused::Rejected(Rejected::CellOutOfRange { cell: 1370, .. })
            )
        ));
        assert_eq!(refused.len(), 11);
    }

    #[test]
    fn a_belt_cell_is_stored_in_the_belt_window_and_loads_back_to_the_same_cell() {
        // The first and last belt cells, and the cells either side of the range, which
        // must stay in the INVENTORY window as the world holds them.
        let belt = EWindows::BeltInventory as u8;
        let end = BELT_INVENTORY_SLOT_START + BELT_INVENTORY_SLOT_COUNT;
        assert_eq!(
            stored_row_position(ItemPos::new(INVENTORY, BELT_INVENTORY_SLOT_START)),
            (belt, 0)
        );
        assert_eq!(
            stored_row_position(ItemPos::new(INVENTORY, end - 1)),
            (belt, u32::from(BELT_INVENTORY_SLOT_COUNT - 1))
        );
        assert_eq!(
            stored_row_position(ItemPos::new(INVENTORY, BELT_INVENTORY_SLOT_START - 1)),
            (INVENTORY, u32::from(BELT_INVENTORY_SLOT_START - 1))
        );
        assert_eq!(
            stored_row_position(ItemPos::new(INVENTORY, end)),
            (INVENTORY, u32::from(end))
        );
        // The hand values: 274 is cell 0 of the belt and 289 is cell 15.
        assert_eq!(stored_row_position(ItemPos::new(1, 289)), (9, 15));
        assert_eq!(stored_row_position(ItemPos::new(1, 290)), (1, 290));
    }

    #[test]
    fn every_position_a_move_can_store_loads_back_where_it_was() {
        let vnum = a_vnum(&owners(), 1, false);
        let cells = (0..INVENTORY_MAX_NUM)
            .chain(BELT_INVENTORY_SLOT_START..BELT_INVENTORY_SLOT_START + BELT_INVENTORY_SLOT_COUNT)
            .chain(CUSTOM_INVENTORY_SLOT_START..common::item_slots::CUSTOM_INVENTORY_SLOT_END);
        for cell in cells {
            let pos = ItemPos::new(INVENTORY, cell);
            let (window, stored) = stored_row_position(pos);
            assert_eq!(
                load_position(&row(1, window, stored, vnum)),
                Ok(pos),
                "cell {cell} did not survive the store"
            );
        }
        // A window the world holds under its own number is stored under it too.
        let dragon_soul = ItemPos::new(EWindows::DragonSoulInventory as u8, 3);
        assert_eq!(
            stored_row_position(dragon_soul),
            (EWindows::DragonSoulInventory as u8, 3)
        );
    }
}
