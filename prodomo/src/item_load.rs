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
//! 3. An `INVENTORY` row whose cell already holds an item, and an `EQUIPMENT` row whose wear
//!    cell is already worn, is set aside (`:1498-1503`).
//! 4. Otherwise `AddToCharacter` places an `INVENTORY` row where the row says (`:1506-1517`).
//! 5. An `EQUIPMENT` row is set aside when `CheckItemUseLevel` fails, and otherwise goes to
//!    `EquipTo` with its cell, which sets it aside in turn when it refuses the cell
//!    (`:1519-1531`). `EquipTo` wants a cell below `WEAR_MAX_NUM` for any item but a dragon soul
//!    stone, wears the item in the inventory window at `INVENTORY_MAX_NUM` plus the cell, and
//!    applies its bonuses (`G/item.cpp:1402-1505`).
//! 6. The set-aside items go to the first free base-inventory cell, or onto the ground when
//!    there is none (`:1541-1561`). They are sent after every directly placed item.
//! 7. `CheckMaximumPoints` and a second `PointsPacket` (`:1563-1564`).
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
//! - **The switchbot and the attribute window are not loaded yet.** A `SWITCHBOT` row registers
//!   with the switchbot manager and an `ATTR67_ADD` row belongs to the refinement window;
//!   neither system is ported, so those rows are refused as
//!   [`Refused::WindowNotPorted`](crate::item_load::Refused::WindowNotPorted) and left in the
//!   store.
//! - **A worn item that starts a system not ported is refused, not worn.** A dragon soul stone,
//!   an aura, mount or pet costume, a unique item, an item with a running timer, an accessory
//!   with stones, an item with immunity flags, and an item with a bonus type whose point is not
//!   ported are each a [`Refused::WornNotPorted`](crate::item_load::Refused::WornNotPorted). A
//!   dragon soul stone is refused before its cell is read, because its deck is not ported.
//! - **An `EQUIPMENT` cell is read whole.** `EquipTo` and `GetWear` take a `BYTE`, so legacy
//!   wears a row at cell 256 in cell 0. Here any cell from `WEAR_MAX_NUM` up is set aside.
//! - **The bonuses are applied once, with everything worn.** `EquipTo` applies each item's
//!   bonuses as it is placed and computes the battle points after each; the Rewrite computes
//!   the points once after the load
//!   ([`Points::compute_loaded`](world::character::Points::compute_loaded)). No record is sent
//!   between the two in legacy either, because the character is in no sector yet and
//!   `UpdatePacket` returns (`G/char.cpp:1283`), so the client sees the same points record.
//! - **The stored pools are kept.** Legacy's `ApplyPoint` keeps a pool's share of a maximum an
//!   item raises, so a relog heals a character saved below full. That is a Defect; the pools
//!   come back as stored.
//! - **`OnAfterCreatedItem` is not run, for any window.** It locks a blend item and starts its
//!   expiry, loads a toggle item, and starts the timer of an item whose real time runs from its
//!   first use (`G/item.cpp:2757-2784`). None of those systems is ported: a worn item that
//!   needs one is refused above, and an unworn one keeps its sockets as stored.
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
    EWindows, BELT_INVENTORY_SLOT_COUNT, BELT_INVENTORY_SLOT_START, INVENTORY_MAX_NUM, WEAR_MAX_NUM,
};
use db::items::ItemRow;
use gamedata::item_kind::{
    COSTUME_AURA, COSTUME_MOUNT, COSTUME_PET, ITEM_COSTUME, ITEM_DS, ITEM_UNIQUE, LIMIT_LEVEL,
    LIMIT_REAL_TIME, LIMIT_REAL_TIME_START_FIRST_USE,
};
use gamedata::item_proto::{ItemProto, ItemProtos};
use protocol::gc_item_window::{GcItemSet, ItemAttribute};
use protocol::item_pos::ItemPos;
use world::character::{
    accessory_socket_grade, apply_is_ported, item_applies, CharacterItems, Rejected,
};
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
    /// Wearing the item starts a system this build has not ported.
    WornNotPorted(WornSystem),
}

/// A system that wearing an item starts, which this build has not ported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WornSystem {
    /// A dragon soul stone: `EquipTo` activates it in the dragon soul deck.
    DragonSoul,
    /// An aura costume: its drain, its armour and its booster timer.
    Aura,
    /// A mount costume: `EquipTo` summons the mount.
    Mount,
    /// A pet costume: `EquipTo` summons the pet.
    Pet,
    /// An `ITEM_UNIQUE` item: its expiry timer, and the alignment title it can hide.
    Unique,
    /// An item whose time runs: a real-time limit, a real-time limit from first use, or a
    /// timer that runs while it is worn.
    Timer,
    /// An accessory with stones in its sockets, which lose one on a timer while it is worn.
    AccessoryTimer,
    /// A bonus type whose point is not ported (`world::character::apply_is_ported`).
    Apply(u8),
    /// A prototype with immunity flags, which the Rewrite does not grant (see
    /// `world::character::Points`).
    Immunity,
}

impl std::fmt::Display for WornSystem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DragonSoul => formatter.write_str("the dragon soul deck"),
            Self::Aura => formatter.write_str("the aura costume"),
            Self::Mount => formatter.write_str("the mount costume"),
            Self::Pet => formatter.write_str("the pet costume"),
            Self::Unique => formatter.write_str("the unique item expiry"),
            Self::Timer => formatter.write_str("the item timers"),
            Self::AccessoryTimer => formatter.write_str("the accessory stone timer"),
            Self::Apply(apply) => write!(formatter, "the bonus type {apply}"),
            Self::Immunity => formatter.write_str("the item immunities"),
        }
    }
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
            Self::WornNotPorted(system) => {
                write!(formatter, "wearing it needs {system}, which is not ported")
            }
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
    /// The placed items in a storage of their own, as the character will hold them.
    #[must_use]
    pub fn storage(&self) -> CharacterItems {
        let mut storage = CharacterItems::new();
        for placed in &self.placed {
            // The load placed each item into an empty storage in this order, so each fits.
            let fitted = storage.set(placed.pos, &placed.item).is_ok();
            debug_assert!(fitted, "item {} fits where the load put it", placed.item.id);
        }
        storage
    }

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
/// set-aside item goes through (ledger 198.3), and `level` is the level a worn item's level
/// limit is checked against.
#[must_use]
pub fn plan_item_load(
    rows: &[ItemRow],
    protos: &ItemProtos,
    inven_point: u16,
    level: u8,
) -> ItemLoad {
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
        let placement = if row.window_type == EWindows::Equipment as u8 {
            wear_position(row, &item, protos, level, &storage)
        } else {
            load_position(row).map(Some)
        };
        let pos = match placement {
            Ok(Some(pos)) => pos,
            Ok(None) => {
                set_aside.push((row, item));
                continue;
            }
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

/// Where an `EQUIPMENT` row's item is worn, `None` when legacy sets it aside, or why it is
/// refused.
///
/// Legacy sets the item aside when its wear cell is taken, when `CheckItemUseLevel` fails, and
/// when `EquipTo` refuses the cell (`G/input_db.cpp:1498-1531`); otherwise `EquipTo` puts it in
/// the inventory window at `INVENTORY_MAX_NUM` plus the cell (`G/item.cpp:1402-1441`). Nothing
/// checks that the item can be worn in that cell: the load trusts the row.
fn wear_position(
    row: &ItemRow,
    item: &Item,
    protos: &ItemProtos,
    level: u8,
    storage: &CharacterItems,
) -> Result<Option<ItemPos>, Refused> {
    let proto = protos.get(item.vnum).ok_or(Refused::UnknownVnum)?;
    if proto.item_type == ITEM_DS {
        return Err(Refused::WornNotPorted(WornSystem::DragonSoul));
    }
    let Some(wear) = u16::try_from(row.pos)
        .ok()
        .filter(|wear| *wear < WEAR_MAX_NUM)
    else {
        return Ok(None);
    };
    let pos = ItemPos::new(EWindows::Inventory as u8, INVENTORY_MAX_NUM + wear);
    if storage.item_at(pos).is_some() || !meets_level_limit(proto, level) {
        return Ok(None);
    }
    if let Some(system) = worn_system_not_ported(item, proto, protos) {
        return Err(Refused::WornNotPorted(system));
    }
    Ok(Some(pos))
}

/// `CItem::CheckItemUseLevel` (`G/item.cpp:2634-2645`): the first level limit decides, and an
/// item with none passes.
fn meets_level_limit(proto: &ItemProto, level: u8) -> bool {
    proto
        .limits
        .iter()
        .find(|limit| limit.kind == LIMIT_LEVEL)
        .is_none_or(|limit| limit.value <= i32::from(level))
}

/// The first system `EquipTo` would start for this item that the Rewrite has not ported.
///
/// `EquipTo` activates a dragon soul stone, applies the item's bonuses (`ModifyPoints`), starts
/// the unique, wear-timer, accessory and aura-booster timers, and summons a mount or pet
/// costume (`G/item.cpp:1458-1487`); `OnAfterCreatedItem` starts the real-time timer
/// (`:2775-2781`). `BuffOnAttr_AddBuffsFromItem` does nothing, because only the two bonus types
/// this build refuses fill its table.
fn worn_system_not_ported(
    item: &Item,
    proto: &ItemProto,
    protos: &ItemProtos,
) -> Option<WornSystem> {
    if proto.item_type == ITEM_COSTUME {
        match proto.sub_type {
            COSTUME_AURA => return Some(WornSystem::Aura),
            COSTUME_MOUNT => return Some(WornSystem::Mount),
            COSTUME_PET => return Some(WornSystem::Pet),
            _ => {}
        }
    }
    if proto.item_type == ITEM_UNIQUE {
        return Some(WornSystem::Unique);
    }
    let timed = proto
        .limits
        .iter()
        .any(|limit| [LIMIT_REAL_TIME, LIMIT_REAL_TIME_START_FIRST_USE].contains(&limit.kind));
    if timed || proto.timer_based_on_wear.is_some() {
        return Some(WornSystem::Timer);
    }
    if accessory_socket_grade(item, proto) > 0 {
        return Some(WornSystem::AccessoryTimer);
    }
    if proto.immune_flags != 0 {
        return Some(WornSystem::Immunity);
    }
    item_applies(item, proto, protos)
        .into_iter()
        .find(|(apply, _)| !apply_is_ported(*apply))
        .map(|(apply, _)| WornSystem::Apply(apply))
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
/// The inverse of the load's belt and wear translations, and `CItemManager::SaveSingleItem`'s
/// (`G/item_manager.cpp:503-520`). `CHARACTER::SetItem` gives an item in the inventory window
/// the `EQUIPMENT` window when its cell is past the base inventory and outside the belt range
/// (`G/char_item.cpp:619-629`), and the save stores it at `cell - INVENTORY_MAX_NUM`. An
/// `INVENTORY` cell in the belt range is stored as `BELT_INVENTORY` at
/// `cell - BELT_INVENTORY_SLOT_START`, because `ENABLE_BELT_INVENTORY_EX` is defined. Every
/// other position is stored as the world holds it.
#[must_use]
pub fn stored_row_position(pos: ItemPos) -> (u8, u32) {
    if pos.window_type == EWindows::Inventory as u8 {
        let belt = BELT_INVENTORY_SLOT_START..BELT_INVENTORY_SLOT_START + BELT_INVENTORY_SLOT_COUNT;
        if belt.contains(&pos.cell) {
            return (
                EWindows::BeltInventory as u8,
                u32::from(pos.cell - BELT_INVENTORY_SLOT_START),
            );
        }
        if (INVENTORY_MAX_NUM..BELT_INVENTORY_SLOT_START).contains(&pos.cell) {
            return (
                EWindows::Equipment as u8,
                u32::from(pos.cell - INVENTORY_MAX_NUM),
            );
        }
    }
    (pos.window_type, u32::from(pos.cell))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::item_slots::CUSTOM_INVENTORY_SLOT_START;
    use gamedata::item_kind::{
        ARMOR_NECK, COSTUME_BODY, ITEM_ARMOR, ITEM_SPECIAL_DS, ITEM_WEAPON,
        LIMIT_TIMER_BASED_ON_WEAR,
    };
    use gamedata::item_proto::ItemValue;
    use world::character::APPLY_ENERGY;

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
    const EQUIPMENT: u8 = EWindows::Equipment as u8;

    /// A 1-cell prototype of this type and nothing else.
    fn shaped(vnum: u32, item_type: i32, sub_type: i32) -> ItemProto {
        ItemProto::for_category_rule(vnum, item_type, sub_type)
    }

    /// `proto` with this limit in its first limit slot.
    fn limited(mut proto: ItemProto, kind: i32, value: i32) -> ItemProto {
        proto.limits[0] = ItemValue { kind, value };
        proto
    }

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
        let load = plan_item_load(&[stored], &protos, 0, 1);
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
            1,
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
            1,
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
        let load = plan_item_load(&rows, &protos, 0, 1);
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
        let load = plan_item_load(&rows, &protos, 0, 1);
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
        let load = plan_item_load(&rows, &protos, 0, 1);
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
        let load = plan_item_load(&rows, &protos, 0, 1);
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
        for (index, id) in [(1, 3), (2, 4), (3, 5), (8, 10), (9, 11)] {
            assert_eq!(refused[index], (id, &Refused::WindowNotPorted), "row {id}");
        }
        assert_eq!(refused[4], (6, &Refused::WearCell));
        assert_eq!(refused[5], (7, &Refused::WearCell));
        assert_eq!(refused[6], (8, &Refused::BadCount(CountRejected::Zero)));
        assert!(matches!(
            refused[7],
            (
                9,
                Refused::Rejected(Rejected::CellOutOfRange { cell: 1370, .. })
            )
        ));
        assert_eq!(refused.len(), 10);
    }

    #[test]
    fn a_belt_cell_is_stored_in_the_belt_window_and_loads_back_to_the_same_cell() {
        // The first and last belt cells, and the cells either side of the range: the one
        // before is the last of the equipment window's, and the one after stays in the
        // INVENTORY window as the world holds it.
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
            (
                EQUIPMENT,
                u32::from(BELT_INVENTORY_SLOT_START - 1 - INVENTORY_MAX_NUM)
            )
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

    #[test]
    fn a_worn_row_is_placed_at_its_wear_cell_and_stored_back_there() {
        // A worn item takes its one wear cell whatever its size, and nothing checks that the
        // item can be worn there: the load trusts the row.
        let mut tall = shaped(100, ITEM_WEAPON, 0);
        tall.size = 3;
        let protos = ItemProtos::from_rows(vec![tall, shaped(101, ITEM_ARMOR, 0)]);
        let rows = [
            row(1, EQUIPMENT, 4, 100),
            row(2, EQUIPMENT, 5, 101),
            row(3, EQUIPMENT, u32::from(WEAR_MAX_NUM) - 1, 101),
        ];
        let load = plan_item_load(&rows, &protos, 0, 1);
        assert!(load.refused.is_empty(), "{:?}", load.refused);
        let placed: Vec<PlacedAt> = load
            .placed
            .iter()
            .map(|placed| (placed.item.id, placed.pos.cell, placed.moved_from))
            .collect();
        assert_eq!(placed, [(1, 184, None), (2, 185, None), (3, 243, None)]);
        let record = load.placed[0].record();
        assert_eq!(record.cell, ItemPos::new(INVENTORY, 184));
        assert_eq!(record.highlight, 0);
        assert!(
            load.moved_rows(&rows).is_empty(),
            "a worn row is not rewritten"
        );
        for (placed, stored) in load.placed.iter().zip(&rows) {
            assert_eq!(
                stored_row_position(placed.pos),
                (EQUIPMENT, stored.pos),
                "item {}",
                placed.item.id
            );
        }
    }

    #[test]
    fn a_worn_row_under_its_level_is_set_aside_into_the_inventory() {
        // `CheckItemUseLevel` reads the first level limit only, and passes at exactly the
        // level; a second level limit is never read.
        let mut twice = limited(shaped(101, ITEM_ARMOR, 0), LIMIT_LEVEL, 30);
        twice.limits[1] = ItemValue {
            kind: LIMIT_LEVEL,
            value: 99,
        };
        let protos = ItemProtos::from_rows(vec![
            limited(shaped(100, ITEM_WEAPON, 0), LIMIT_LEVEL, 30),
            twice,
        ]);
        let rows = [row(1, EQUIPMENT, 4, 100), row(2, EQUIPMENT, 5, 101)];
        let at_level = plan_item_load(&rows, &protos, 0, 30);
        assert!(at_level.refused.is_empty(), "{:?}", at_level.refused);
        let cells: Vec<u16> = at_level
            .placed
            .iter()
            .map(|placed| placed.pos.cell)
            .collect();
        assert_eq!(cells, [184, 185]);
        let under = plan_item_load(&rows, &protos, 0, 29);
        assert!(under.refused.is_empty(), "{:?}", under.refused);
        let placed: Vec<PlacedAt> = under
            .placed
            .iter()
            .map(|placed| (placed.item.id, placed.pos.cell, placed.moved_from))
            .collect();
        assert_eq!(
            placed,
            [(1, 0, Some((EQUIPMENT, 4))), (2, 1, Some((EQUIPMENT, 5)))]
        );
        assert_eq!(
            under.moved_rows(&rows),
            [
                ItemRow {
                    window_type: INVENTORY,
                    pos: 0,
                    ..rows[0].clone()
                },
                ItemRow {
                    window_type: INVENTORY,
                    pos: 1,
                    ..rows[1].clone()
                },
            ]
        );
    }

    #[test]
    fn a_worn_row_past_the_wear_cells_or_on_a_taken_one_is_set_aside() {
        // Legacy passes the row's `pos` to `GetWear` and `EquipTo` as a `BYTE`, so pos 260
        // would be worn at cell 4; the Rewrite reads the whole value and sets it aside. The
        // taken cell is tested before `EquipTo` (`G/input_db.cpp:1498-1503`), so the unique
        // item on it is set aside like any other, not refused for the system it would start.
        let protos = ItemProtos::from_rows(vec![
            shaped(100, ITEM_WEAPON, 0),
            shaped(101, ITEM_UNIQUE, 0),
        ]);
        let rows = [
            row(1, EQUIPMENT, 4, 100),
            row(2, EQUIPMENT, 4, 101),
            row(3, EQUIPMENT, u32::from(WEAR_MAX_NUM), 100),
            row(4, EQUIPMENT, 260, 100),
            row(5, EQUIPMENT, 70_000, 100),
        ];
        let load = plan_item_load(&rows, &protos, 0, 1);
        assert!(load.refused.is_empty(), "{:?}", load.refused);
        let placed: Vec<PlacedAt> = load
            .placed
            .iter()
            .map(|placed| (placed.item.id, placed.pos.cell, placed.moved_from))
            .collect();
        assert_eq!(
            placed,
            [
                (1, 184, None),
                (2, 0, Some((EQUIPMENT, 4))),
                (3, 1, Some((EQUIPMENT, 64))),
                (4, 2, Some((EQUIPMENT, 260))),
                (5, 3, Some((EQUIPMENT, 70_000))),
            ]
        );
    }

    #[test]
    fn a_worn_item_that_needs_a_system_not_ported_is_refused_and_kept() {
        let mut immune = shaped(108, ITEM_ARMOR, 0);
        immune.immune_flags = 1;
        let mut timer = limited(shaped(107, ITEM_ARMOR, 0), LIMIT_TIMER_BASED_ON_WEAR, 60);
        timer.timer_based_on_wear = Some(0);
        let mut energy = shaped(109, ITEM_ARMOR, 0);
        energy.applies[0] = ItemValue {
            kind: i32::from(APPLY_ENERGY),
            value: 5,
        };
        let protos = ItemProtos::from_rows(vec![
            shaped(100, ITEM_DS, 0),
            shaped(101, ITEM_COSTUME, COSTUME_AURA),
            shaped(102, ITEM_COSTUME, COSTUME_MOUNT),
            shaped(103, ITEM_COSTUME, COSTUME_PET),
            shaped(104, ITEM_UNIQUE, 0),
            limited(shaped(105, ITEM_ARMOR, 0), LIMIT_REAL_TIME, 60),
            limited(
                shaped(106, ITEM_ARMOR, 0),
                LIMIT_REAL_TIME_START_FIRST_USE,
                60,
            ),
            timer,
            immune,
            energy,
            shaped(110, ITEM_ARMOR, ARMOR_NECK),
        ]);
        let mut accessory = row(11, EQUIPMENT, 10, 110);
        accessory.sockets = [1, 3, 0, 0, 0, 0];
        let rows: Vec<ItemRow> = (0..10)
            .map(|index| row(index + 1, EQUIPMENT, index + 1, 100 + index))
            .chain([accessory])
            .collect();
        let load = plan_item_load(&rows, &protos, 0, 1);
        assert!(load.placed.is_empty(), "{:?}", load.placed);
        let refused: Vec<(u32, Refused)> = load
            .refused
            .iter()
            .map(|refusal| (refusal.id, refusal.why.clone()))
            .collect();
        let expected = [
            WornSystem::DragonSoul,
            WornSystem::Aura,
            WornSystem::Mount,
            WornSystem::Pet,
            WornSystem::Unique,
            WornSystem::Timer,
            WornSystem::Timer,
            WornSystem::Timer,
            WornSystem::Immunity,
            WornSystem::Apply(APPLY_ENERGY),
            WornSystem::AccessoryTimer,
        ];
        let expected: Vec<(u32, Refused)> = (1..)
            .zip(expected)
            .map(|(id, system)| (id, Refused::WornNotPorted(system)))
            .collect();
        assert_eq!(refused, expected);
        assert!(
            load.moved_rows(&rows).is_empty(),
            "a refused row is not rewritten"
        );
    }

    #[test]
    fn a_worn_item_whose_systems_are_ported_loads() {
        // The near neighbours of every refusal: an ordinary costume, a special dragon soul
        // item (a normal item to `EquipTo`), a level limit, and an accessory with no stone.
        let protos = ItemProtos::from_rows(vec![
            shaped(100, ITEM_COSTUME, COSTUME_BODY),
            shaped(101, ITEM_SPECIAL_DS, 0),
            limited(shaped(102, ITEM_ARMOR, 0), LIMIT_LEVEL, 1),
            shaped(103, ITEM_ARMOR, ARMOR_NECK),
        ]);
        let mut accessory = row(4, EQUIPMENT, 4, 103);
        accessory.sockets = [0, 3, 0, 0, 0, 0];
        let rows = [
            row(1, EQUIPMENT, 1, 100),
            row(2, EQUIPMENT, 2, 101),
            row(3, EQUIPMENT, 3, 102),
            accessory,
        ];
        let load = plan_item_load(&rows, &protos, 0, 1);
        assert!(load.refused.is_empty(), "{:?}", load.refused);
        let cells: Vec<u16> = load.placed.iter().map(|placed| placed.pos.cell).collect();
        assert_eq!(cells, [181, 182, 183, 184]);
    }

    #[test]
    fn a_dragon_soul_stone_in_the_equipment_window_is_refused_before_its_cell_is_read() {
        let protos = ItemProtos::from_rows(vec![shaped(100, ITEM_DS, 0)]);
        for pos in [0, 63, 64, 70_000] {
            let load = plan_item_load(&[row(1, EQUIPMENT, pos, 100)], &protos, 0, 1);
            assert!(load.placed.is_empty(), "pos {pos}");
            assert_eq!(
                load.refused[0].why,
                Refused::WornNotPorted(WornSystem::DragonSoul),
                "pos {pos}"
            );
        }
    }

    #[test]
    fn a_wear_cell_is_stored_in_the_equipment_window() {
        // `SetItem` gives the equipment window to every inventory cell past the base inventory
        // and before the belt, and the save subtracts `INVENTORY_MAX_NUM`.
        assert_eq!(
            stored_row_position(ItemPos::new(INVENTORY, 180)),
            (EQUIPMENT, 0)
        );
        assert_eq!(
            stored_row_position(ItemPos::new(INVENTORY, 184)),
            (EQUIPMENT, 4)
        );
        assert_eq!(
            stored_row_position(ItemPos::new(INVENTORY, 273)),
            (EQUIPMENT, 93)
        );
        assert_eq!(
            stored_row_position(ItemPos::new(INVENTORY, 179)),
            (INVENTORY, 179)
        );
        assert_eq!(
            stored_row_position(ItemPos::new(INVENTORY, 274)),
            (EWindows::BeltInventory as u8, 0)
        );
        // The equipment window's own number is stored under it as it is.
        assert_eq!(
            stored_row_position(ItemPos::new(EQUIPMENT, 4)),
            (EQUIPMENT, 4)
        );
    }
}
