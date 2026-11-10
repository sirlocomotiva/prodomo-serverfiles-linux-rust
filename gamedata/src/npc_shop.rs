//! The NPC shops a keeper opens, built from the owner's Game data tables.
//!
//! [`shops_from_dump`] answers legacy's boot query ([`crate::shop::SHOP_TABLE_QUERY`],
//! `ClientManagerBoot.cpp:299-379`) from the `shop` and `shop_item` rows of the owner's
//! `player.sql` dump, and hands the joined rows to [`build_shop_table_legacy`].
//!
//! [`NpcShops::lay_out`] is legacy `CShopManager::Initialize` with `CShop::Create` and
//! `CShop::SetShopItems` (`shop_manager.cpp:39-69`, `shop.cpp:66-196`). A shop's items are
//! placed in table order on a grid 5 cells wide and 9 high. Each takes one column of as many
//! cells as its size, at the first cell `CGrid::FindBlank` finds (`grid.cpp:28-47`), and the
//! cell it starts at is its slot. Its price comes from its item proto.
//!
//! Legacy answers an item it cannot place with `continue`, which skips `++pTable`, so every
//! later pass tries the same item again and fails again: one item it cannot place loses every
//! item after it. The Rewrite skips only that item and reports it, a recorded Divergence. It
//! also skips an item with no size, which legacy writes over slot 0 with, and an item whose
//! cell is past the 40 slots, where legacy writes past the end of its slot vector.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::item_proto::{ItemProto, ItemProtos};
use crate::records::{ShopItemRecord, ShopTableRecord};
use crate::renewal_shop::{
    build_renewal_shop_table_legacy, RenewalShopLimits, RenewalShopQueryRow, RenewalShopQueryValue,
    RenewalShopTableError,
};
use crate::shop::{
    build_shop_table_legacy, ShopLimits, ShopQueryValue, ShopTableError, ShopTableQueryRow,
    SHOP_ITEM_MAX_NUM,
};
use crate::sql_dump::{read_table, SqlDumpError, SqlTable, SqlValue};

/// `ITEM_FLAG_COUNT_PER_1GOLD` (`item_length.h:362`): the shop prices this item as so many
/// units for one gold, and buys it back the same way.
pub const ITEM_FLAG_COUNT_PER_1GOLD: u32 = 1 << 3;

/// The width of a shop's grid: `CGrid(5, 9)` (`shop.cpp:31`).
pub const SHOP_GRID_WIDTH: usize = 5;

/// The height of a shop's grid.
pub const SHOP_GRID_HEIGHT: usize = 9;

const SHOP_GRID_CELLS: usize = SHOP_GRID_WIDTH * SHOP_GRID_HEIGHT;

/// One item a shop sells: legacy `CShop::SHOP_ITEM` for an NPC shop (`shop.h:15-36`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopSlot {
    /// `vnum`.
    pub vnum: u32,
    /// `count`, the `shop_item.count` column.
    pub count: u16,
    /// `price`, from [`shop_price`], or the `shopex_item.price` column for a renewal item.
    pub price: u64,
    /// `price_type`, the renewal price selector (`SHOPEX_*`, `C/tables.h:786-793`).
    /// A base item is always [`crate::records::SHOP_PRICE_TYPE_GOLD`].
    pub price_type: u8,
    /// `price_vnum`, the price item of a `SHOPEX_ITEM` renewal item.
    pub price_vnum: u32,
    /// `alSockets`: the renewal item's sockets, which a buy stamps on the bought item.
    pub sockets: [i32; crate::records::SHOP_ITEM_SOCKET_MAX_NUM],
    /// `aAttr`: the renewal item's attributes, stamped the same way.
    pub attrs: [crate::records::ShopItemAttribute; crate::records::SHOP_ITEM_ATTRIBUTE_MAX_NUM],
}

/// One NPC shop, laid out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpcShop {
    /// `m_dwVnum`, the `shop.vnum` column.
    pub vnum: u32,
    /// `m_dwNPCVnum`, the `shop.npc_vnum` column.
    pub npc_vnum: u32,
    /// `name`, the `shopex.name` tab bytes. A base shop holds no tab name.
    pub tab_name: Option<[u8; crate::records::SHOP_SIGN_BYTES]>,
    /// `m_itemVector`, by slot. `None` is a slot no item was placed in.
    pub slots: [Option<ShopSlot>; SHOP_ITEM_MAX_NUM],
}

impl ShopSlot {
    /// An empty slot as `CShop::Buy` prices it: 0 of nothing for 0 gold.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            vnum: 0,
            count: 0,
            price: 0,
            price_type: crate::records::SHOP_PRICE_TYPE_GOLD,
            price_vnum: 0,
            sockets: [0; crate::records::SHOP_ITEM_SOCKET_MAX_NUM],
            attrs: [crate::records::ShopItemAttribute {
                attr_type: 0,
                value: 0,
            }; crate::records::SHOP_ITEM_ATTRIBUTE_MAX_NUM],
        }
    }
}

impl NpcShop {
    /// Whether this is a renewal (`shopex`) tab: a base shop holds no tab name.
    #[must_use]
    pub const fn is_renewal(&self) -> bool {
        self.tab_name.is_some()
    }
}

/// Why an item of a shop's table was left out of the shop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopSkip {
    /// `GetTable` found no proto for the vnum (`shop.cpp:122-126`).
    NoProto,
    /// The proto's `bSize` is 0. Legacy's `FindBlank(1, 0)` answers cell 0, `Put` marks
    /// nothing, and the item takes slot 0 from whatever held it.
    NoSize,
    /// No column of free cells is as tall as the item (`shop.cpp:136-140`).
    NoRoom {
        /// The item's `bSize`.
        size: u8,
    },
    /// The item's cell is on the grid's last row, past the 40 slots, where legacy's
    /// `m_itemVector[iPos]` writes past the end of the vector.
    PastTheSlots {
        /// The grid cell `FindBlank` answered.
        cell: usize,
    },
}

/// An item of a shop's table that was left out of the shop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkippedShopItem {
    /// The shop's vnum.
    pub shop_vnum: u32,
    /// The item's vnum.
    pub vnum: u32,
    /// Why it was left out.
    pub skip: ShopSkip,
}

/// Every NPC shop, found by the race of the keeper who opens it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NpcShops {
    shops: Vec<NpcShop>,
    by_npc: BTreeMap<u32, usize>,
    skipped: Vec<SkippedShopItem>,
}

impl NpcShops {
    /// Lay out every shop of the table, in the table's order.
    ///
    /// A keeper's race finds the first shop that names it, because legacy's
    /// `m_map_pkShopByNPCVnum.insert` keeps the first (`shop_manager.cpp:57`).
    #[must_use]
    pub fn lay_out(records: &[ShopTableRecord], protos: &ItemProtos) -> Self {
        let mut shops = Self::default();
        for record in records {
            let shop = lay_out_shop(record, protos, &mut shops.skipped);
            let index = shops.shops.len();
            shops.by_npc.entry(shop.npc_vnum).or_insert(index);
            shops.shops.push(shop);
        }
        shops
    }

    /// Lay out every renewal tab of the table, in the table's order
    /// (`CShopManager::InitializeShopEX`, `G/shop_manager.cpp:773-820`). Each record is
    /// one tab: the name is the record's, the coin type is gold, and the items are
    /// placed like a base shop's, but the price, the selector, the sockets and the
    /// attributes come from the row, not the proto.
    #[must_use]
    pub fn lay_out_renewal(records: &[ShopTableRecord], protos: &ItemProtos) -> Self {
        let mut shops = Self::default();
        for record in records {
            let shop = lay_out_renewal_shop(record, protos, &mut shops.skipped);
            let index = shops.shops.len();
            shops.by_npc.entry(shop.npc_vnum).or_insert(index);
            shops.shops.push(shop);
        }
        shops
    }

    /// Every renewal tab a keeper of this race opens, in the table's order.
    #[must_use]
    pub fn tabs_for_npc(&self, npc_vnum: u32) -> Vec<&NpcShop> {
        self.shops
            .iter()
            .filter(|shop| shop.npc_vnum == npc_vnum)
            .collect()
    }

    /// `CShopManager::GetByNPCVnum` (`shop_manager.cpp:100-108`): the shop a keeper of this
    /// race opens.
    #[must_use]
    pub fn for_npc(&self, npc_vnum: u32) -> Option<&NpcShop> {
        self.by_npc.get(&npc_vnum).map(|index| &self.shops[*index])
    }

    /// Every shop, in the table's order.
    #[must_use]
    pub fn shops(&self) -> &[NpcShop] {
        &self.shops
    }

    /// Every item left out, in the order the shops were laid out.
    #[must_use]
    pub fn skipped(&self) -> &[SkippedShopItem] {
        &self.skipped
    }
}

/// The price `SetShopItems` puts on `count` of an item (`shop.cpp:176-185`).
///
/// Legacy multiplies a `DWORD` by a `WORD` in 32 bits, so a price past `u32::MAX` wraps before
/// it is stored in the 64-bit field. The Rewrite multiplies in 64 bits, a recorded Divergence.
/// No owner shop item comes near it.
#[must_use]
pub fn shop_price(proto: &ItemProto, count: u16) -> u64 {
    let count = u64::from(count);
    if proto.flags & ITEM_FLAG_COUNT_PER_1GOLD == 0 {
        u64::from(proto.gold) * count
    } else if proto.gold == 0 {
        count
    } else {
        count / u64::from(proto.gold)
    }
}

fn lay_out_shop(
    record: &ShopTableRecord,
    protos: &ItemProtos,
    skipped: &mut Vec<SkippedShopItem>,
) -> NpcShop {
    let mut shop = NpcShop {
        vnum: record.vnum,
        npc_vnum: record.npc_vnum,
        tab_name: None,
        slots: [None; SHOP_ITEM_MAX_NUM],
    };
    let mut grid = [false; SHOP_GRID_CELLS];
    // `Create` counts the items up to the first empty vnum (`shop.cpp:81-83`).
    for item in record.items.iter().take_while(|item| item.vnum != 0) {
        match place(&mut grid, item, protos) {
            Ok((slot, placed)) => shop.slots[slot] = Some(placed),
            Err(skip) => skipped.push(SkippedShopItem {
                shop_vnum: record.vnum,
                vnum: item.vnum,
                skip,
            }),
        }
    }
    shop
}

fn lay_out_renewal_shop(
    record: &ShopTableRecord,
    protos: &ItemProtos,
    skipped: &mut Vec<SkippedShopItem>,
) -> NpcShop {
    let mut shop = NpcShop {
        vnum: record.vnum,
        npc_vnum: record.npc_vnum,
        tab_name: Some(record.shop_name),
        slots: [None; SHOP_ITEM_MAX_NUM],
    };
    let mut grid = [false; SHOP_GRID_CELLS];
    for item in record.items.iter().take_while(|item| item.vnum != 0) {
        match place_renewal(&mut grid, item, protos) {
            Ok((slot, placed)) => shop.slots[slot] = Some(placed),
            Err(skip) => skipped.push(SkippedShopItem {
                shop_vnum: record.vnum,
                vnum: item.vnum,
                skip,
            }),
        }
    }
    shop
}

fn place_renewal(
    grid: &mut [bool; SHOP_GRID_CELLS],
    item: &ShopItemRecord,
    protos: &ItemProtos,
) -> Result<(usize, ShopSlot), ShopSkip> {
    let proto = protos.get(item.vnum).ok_or(ShopSkip::NoProto)?;
    let size = proto.size.to_le_bytes()[0];
    if size == 0 {
        return Err(ShopSkip::NoSize);
    }
    let height = usize::from(size);
    let cell = find_blank(grid, height).ok_or(ShopSkip::NoRoom { size })?;
    for row in 0..height {
        grid[cell + row * SHOP_GRID_WIDTH] = true;
    }
    if cell >= SHOP_ITEM_MAX_NUM {
        return Err(ShopSkip::PastTheSlots { cell });
    }
    let slot = ShopSlot {
        vnum: item.vnum,
        count: item.count,
        price: item.price,
        price_type: item.price_type,
        price_vnum: item.price_vnum,
        sockets: item.sockets,
        attrs: item.attrs,
    };
    Ok((cell, slot))
}

fn place(
    grid: &mut [bool; SHOP_GRID_CELLS],
    item: &ShopItemRecord,
    protos: &ItemProtos,
) -> Result<(usize, ShopSlot), ShopSkip> {
    let proto = protos.get(item.vnum).ok_or(ShopSkip::NoProto)?;
    // `bSize` is a `BYTE` assigned from the file's `int`, so it is the low byte.
    let size = proto.size.to_le_bytes()[0];
    if size == 0 {
        return Err(ShopSkip::NoSize);
    }
    let height = usize::from(size);
    let cell = find_blank(grid, height).ok_or(ShopSkip::NoRoom { size })?;
    // Legacy marks the cells before it writes the slot, so a later item finds them taken.
    for row in 0..height {
        grid[cell + row * SHOP_GRID_WIDTH] = true;
    }
    if cell >= SHOP_ITEM_MAX_NUM {
        return Err(ShopSkip::PastTheSlots { cell });
    }
    let slot = ShopSlot {
        vnum: item.vnum,
        count: item.count,
        price: shop_price(proto, item.count),
        price_type: crate::records::SHOP_PRICE_TYPE_GOLD,
        price_vnum: 0,
        sockets: [0; crate::records::SHOP_ITEM_SOCKET_MAX_NUM],
        attrs: [crate::records::ShopItemAttribute {
            attr_type: 0,
            value: 0,
        }; crate::records::SHOP_ITEM_ATTRIBUTE_MAX_NUM],
    };
    Ok((cell, slot))
}

/// `CGrid::FindBlank(1, height)`: the first cell, row by row, whose column of `height` cells
/// is on the grid and free (`grid.cpp:28-47`, `:87-112`).
fn find_blank(grid: &[bool; SHOP_GRID_CELLS], height: usize) -> Option<usize> {
    (0..SHOP_GRID_CELLS).find(|cell| {
        cell / SHOP_GRID_WIDTH + height <= SHOP_GRID_HEIGHT
            && (0..height).all(|row| !grid[cell + row * SHOP_GRID_WIDTH])
    })
}

/// A dump the shop tables cannot be read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShopDumpError {
    /// The dump could not be read.
    Dump(SqlDumpError),
    /// A table with rows lacks a column the query reads.
    NoColumn {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// A value the query joins or sorts on is not a whole number.
    NotANumber {
        /// The table.
        table: &'static str,
        /// The zero-based row of the table.
        row: usize,
        /// The column.
        column: &'static str,
    },
    /// The joined rows do not build a shop table.
    Table(ShopTableError),
    /// The joined renewal rows do not build a renewal table.
    RenewalTable(RenewalShopTableError),
    /// A `shopex_item` row's `price_type` names no enum member.
    BadPriceType {
        /// The zero-based row of the table.
        row: usize,
        /// The bytes the cell held.
        value: Vec<u8>,
    },
}

impl fmt::Display for ShopDumpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dump(error) => write!(formatter, "shop dump: {error}"),
            Self::NoColumn { table, column } => {
                write!(formatter, "`{table}` dump has no `{column}` column")
            }
            Self::NotANumber { table, row, column } => {
                write!(
                    formatter,
                    "`{table}` row {row} has a `{column}` that is not a number"
                )
            }
            Self::Table(error) => error.fmt(formatter),
            Self::RenewalTable(error) => error.fmt(formatter),
            Self::BadPriceType { row, value } => write!(
                formatter,
                "`shopex_item` row {row} has a `price_type` that is none of GOLD, SECONDCOIN, ITEM, EXP: {value:?}"
            ),
        }
    }
}

impl Error for ShopDumpError {}

/// One value of a joined column: its number, for the join and the sort, and its bytes, for the
/// legacy row decoder.
type Cell = (i64, Vec<u8>);

/// One joined row: the shop's vnum and NPC, and the item's vnum and count, or `NULL`s.
type Joined<'a> = (&'a Cell, &'a Cell, Option<(&'a Cell, &'a Cell)>);

/// Build the shop table from the `shop` and `shop_item` rows of the owner's `player.sql` dump.
///
/// Each shop row is joined with every `shop_item` row of its vnum, or with one row of `NULL`s
/// when it has none, which is `LEFT JOIN`. The joined rows are sorted by shop vnum and then by
/// item vnum, `NULL` first, as MySQL orders them; rows equal on both keep the dump's order.
///
/// # Errors
///
/// Returns [`ShopDumpError`] when the dump cannot be read, a column is missing or not a
/// number, or the joined rows are refused by [`build_shop_table_legacy`].
pub fn shops_from_dump(dump: &[u8]) -> Result<Vec<ShopTableRecord>, ShopDumpError> {
    let shops = read_table(dump, "shop").map_err(ShopDumpError::Dump)?;
    let items = read_table(dump, "shop_item").map_err(ShopDumpError::Dump)?;
    let shops = numbers(&shops, "shop", ["vnum", "npc_vnum"])?;
    let items = numbers(&items, "shop_item", ["shop_vnum", "item_vnum", "count"])?;
    let mut joined: Vec<Joined> = Vec::new();
    for [vnum, npc] in &shops {
        let before = joined.len();
        for [_, item, count] in items.iter().filter(|[shop, _, _]| shop.0 == vnum.0) {
            joined.push((vnum, npc, Some((item, count))));
        }
        if joined.len() == before {
            joined.push((vnum, npc, None));
        }
    }
    joined.sort_by_key(|(vnum, _, item)| (vnum.0, item.map(|(item, _)| item.0)));
    let rows: Vec<ShopTableQueryRow> = joined
        .into_iter()
        .map(|(vnum, npc, item)| {
            let (item, count) =
                item.map_or((ShopQueryValue::null(), ShopQueryValue::null()), |pair| {
                    (
                        ShopQueryValue::bytes(&pair.0 .1),
                        ShopQueryValue::bytes(&pair.1 .1),
                    )
                });
            ShopTableQueryRow::new([
                ShopQueryValue::bytes(&vnum.1),
                ShopQueryValue::bytes(&npc.1),
                item,
                count,
            ])
        })
        .collect();
    build_shop_table_legacy(&rows, ShopLimits::default()).map_err(ShopDumpError::Table)
}

/// Build the renewal (`shopex`) table from the `shopex` and `shopex_item` rows of the
/// owner's `player.sql` dump.
///
/// The join and the sort are the boot query's (`InitializeShopEXTable`,
/// `D/ClientManagerBoot.cpp:384-466`): each `shopex` row with every `shopex_item` row of
/// its vnum, or with one row of `NULL`s when it has none, sorted by shop vnum and then by
/// item vnum, `NULL` first. The joined rows are decoded with the legacy policy. The one
/// column the query transforms is handed over transformed: `price_type+0` maps the enum
/// to its 1-based index (`player.sql:1296`, `enum('GOLD','SECONDCOIN','ITEM','EXP')`),
/// which is the `SHOPEX_*` selector, so the decoder is given the index bytes (`b"1"` for
/// the owner's row) rather than the word.
///
/// # Errors
///
/// Returns [`ShopDumpError`] when the dump cannot be read, a column is missing or not a
/// number, a `price_type` names no enum member, or the joined rows are refused by
/// [`build_renewal_shop_table_legacy`].
pub fn renewal_shops_from_dump(dump: &[u8]) -> Result<Vec<ShopTableRecord>, ShopDumpError> {
    let shops = read_table(dump, "shopex").map_err(ShopDumpError::Dump)?;
    let items = read_table(dump, "shopex_item").map_err(ShopDumpError::Dump)?;
    let (shop_keys, shop_names) = renewal_shops(&shops)?;
    let item_rows = renewal_items(&items)?;
    let mut joined: Vec<(usize, Option<usize>)> = Vec::new();
    for (shop, keys) in shop_keys.iter().enumerate() {
        let before = joined.len();
        for (row, item) in item_rows.iter().enumerate() {
            if item.keys[0].0 == keys[0].0 {
                joined.push((shop, Some(row)));
            }
        }
        if joined.len() == before {
            joined.push((shop, None));
        }
    }
    joined.sort_by_key(|(shop, item)| {
        (
            shop_keys[*shop][0].0,
            item.map(|row| item_rows[row].keys[1].0),
        )
    });
    let columns = renewal_item_columns(&items)?;
    let mut rows = Vec::with_capacity(joined.len());
    for (shop, item) in joined {
        rows.push(renewal_row(
            &shop_keys,
            &shop_names,
            &item_rows,
            &columns,
            shop,
            item,
        )?);
    }
    build_renewal_shop_table_legacy(&rows, RenewalShopLimits::default())
        .map_err(ShopDumpError::RenewalTable)
}

/// One `shopex_item` row: the joined numbers and every cell the boot query reads.
struct RenewalItem {
    /// `[shop_vnum, item_vnum, count]`, for the join and the sort.
    keys: [Cell; 3],
    /// Every value of the row, by column position.
    values: Vec<SqlValue>,
}

/// The `vnum`/`npc_vnum` numbers and the `name` bytes of every `shopex` row.
type RenewalShops = (Vec<[Cell; 2]>, Vec<Vec<u8>>);

/// The `vnum`/`npc_vnum` numbers and the `name` bytes of every `shopex` row.
fn renewal_shops(table: &SqlTable) -> Result<RenewalShops, ShopDumpError> {
    let keys = numbers(table, "shopex", ["vnum", "npc_vnum"])?;
    if table.rows.is_empty() {
        return Ok((keys, Vec::new()));
    }
    let at = table.column("name").ok_or(ShopDumpError::NoColumn {
        table: "shopex",
        column: "name",
    })?;
    let mut names = Vec::with_capacity(table.rows.len());
    for values in &table.rows {
        let SqlValue::Text(bytes) = &values[at] else {
            return Err(ShopDumpError::NotANumber {
                table: "shopex",
                row: names.len(),
                column: "name",
            });
        };
        names.push(bytes.clone());
    }
    Ok((keys, names))
}

/// The joined numbers and the whole rows of `shopex_item`.
fn renewal_items(table: &SqlTable) -> Result<Vec<RenewalItem>, ShopDumpError> {
    let keys = numbers(table, "shopex_item", ["shop_vnum", "item_vnum", "count"])?;
    Ok(keys
        .into_iter()
        .zip(&table.rows)
        .map(|(keys, values)| RenewalItem {
            keys,
            values: values.clone(),
        })
        .collect())
}

/// The column positions the boot query reads from `shopex_item`, in the query's order:
/// `price`, `price_vnum`, `price_type`, the six sockets, and the seven attribute pairs.
struct RenewalItemColumns {
    /// `price`, `price_vnum`, `price_type`.
    price: [usize; 3],
    /// `socket0` through `socket5`.
    sockets: [usize; 6],
    /// `attrtypeN`, `attrvalueN`.
    attrs: [(usize, usize); 7],
}

/// The positions of the boot query's `shopex_item` columns.
fn renewal_item_columns(table: &SqlTable) -> Result<RenewalItemColumns, ShopDumpError> {
    if table.rows.is_empty() {
        return Ok(RenewalItemColumns {
            price: [0; 3],
            sockets: [0; 6],
            attrs: [(0, 0); 7],
        });
    }
    let at = |column: &'static str| {
        table.column(column).ok_or(ShopDumpError::NoColumn {
            table: "shopex_item",
            column,
        })
    };
    Ok(RenewalItemColumns {
        price: [at("price")?, at("price_vnum")?, at("price_type")?],
        sockets: [
            at("socket0")?,
            at("socket1")?,
            at("socket2")?,
            at("socket3")?,
            at("socket4")?,
            at("socket5")?,
        ],
        attrs: [
            (at("attrtype0")?, at("attrvalue0")?),
            (at("attrtype1")?, at("attrvalue1")?),
            (at("attrtype2")?, at("attrvalue2")?),
            (at("attrtype3")?, at("attrvalue3")?),
            (at("attrtype4")?, at("attrvalue4")?),
            (at("attrtype5")?, at("attrvalue5")?),
            (at("attrtype6")?, at("attrvalue6")?),
        ],
    })
}

/// One 28-column renewal row, in the boot query's column order.
fn renewal_row(
    shops: &[[Cell; 2]],
    names: &[Vec<u8>],
    items: &[RenewalItem],
    columns: &RenewalItemColumns,
    shop: usize,
    item: Option<usize>,
) -> Result<RenewalShopQueryRow, ShopDumpError> {
    let mut cells = Vec::with_capacity(28);
    cells.push(RenewalShopQueryValue::bytes(&shops[shop][0].1));
    cells.push(RenewalShopQueryValue::bytes(&names[shop]));
    cells.push(RenewalShopQueryValue::bytes(&shops[shop][1].1));
    let Some(row) = item.map(|row| &items[row]) else {
        cells.extend((0..25).map(|_| RenewalShopQueryValue::null()));
        return RenewalShopQueryRow::try_new(cells).map_err(|source| {
            ShopDumpError::RenewalTable(RenewalShopTableError::Row {
                index: shop,
                source,
            })
        });
    };
    let number = |value: &SqlValue| match value {
        SqlValue::Bare(bytes) | SqlValue::Text(bytes) => RenewalShopQueryValue::bytes(bytes),
        SqlValue::Null => RenewalShopQueryValue::null(),
    };
    cells.push(number(&SqlValue::Bare(row.keys[1].1.clone())));
    cells.push(number(&SqlValue::Bare(row.keys[2].1.clone())));
    for at in [columns.price[0], columns.price[1]] {
        cells.push(number(&row.values[at]));
    }
    cells.push(renewal_price_type(&row.values[columns.price[2]], shop)?);
    for at in columns.sockets {
        cells.push(number(&row.values[at]));
    }
    for (attr, value) in columns.attrs {
        cells.push(number(&row.values[attr]));
        cells.push(number(&row.values[value]));
    }
    RenewalShopQueryRow::try_new(cells).map_err(|source| {
        ShopDumpError::RenewalTable(RenewalShopTableError::Row {
            index: shop,
            source,
        })
    })
}

/// The `price_type+0` the boot query hands the decoder: the enum's 1-based index as
/// decimal bytes (`player.sql:1296`, `enum('GOLD','SECONDCOIN','ITEM','EXP')`), which is
/// the `SHOPEX_*` selector (`C/tables.h:786-793`).
fn renewal_price_type(
    value: &SqlValue,
    shop: usize,
) -> Result<RenewalShopQueryValue, ShopDumpError> {
    let word = match value {
        SqlValue::Text(bytes) | SqlValue::Bare(bytes) => bytes.as_slice(),
        SqlValue::Null => b"0",
    };
    let index = match word {
        b"GOLD" => b"1",
        b"SECONDCOIN" => b"2",
        b"ITEM" => b"3",
        b"EXP" => b"4",
        b"0" => b"0",
        _ => {
            return Err(ShopDumpError::BadPriceType {
                row: shop,
                value: word.to_vec(),
            });
        }
    };
    Ok(RenewalShopQueryValue::bytes(index))
}

/// The named columns of every row, each read as a whole number.
fn numbers<const N: usize>(
    table: &SqlTable,
    name: &'static str,
    columns: [&'static str; N],
) -> Result<Vec<[Cell; N]>, ShopDumpError> {
    if table.rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut at = [0; N];
    for (index, column) in columns.iter().enumerate() {
        at[index] = table.column(column).ok_or(ShopDumpError::NoColumn {
            table: name,
            column,
        })?;
    }
    table
        .rows
        .iter()
        .enumerate()
        .map(|(row, values)| {
            let mut cells: [Cell; N] = std::array::from_fn(|_| (0, Vec::new()));
            for (index, column) in columns.iter().enumerate() {
                let not_a_number = ShopDumpError::NotANumber {
                    table: name,
                    row,
                    column,
                };
                let SqlValue::Bare(bytes) = &values[at[index]] else {
                    return Err(not_a_number);
                };
                let number = std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|text| text.parse::<i64>().ok())
                    .ok_or(not_a_number)?;
                cells[index] = (number, bytes.clone());
            }
            Ok(cells)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn owner_dump() -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../legacy/sql/gamedata/player.sql"
        );
        std::fs::read(path).unwrap()
    }

    fn owner_protos() -> ItemProtos {
        ItemProtos::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto"))
            .unwrap()
    }

    fn owner_shops() -> NpcShops {
        NpcShops::lay_out(&shops_from_dump(&owner_dump()).unwrap(), &owner_protos())
    }

    /// A proto with a size, a `dwGold` and a flag mask.
    fn proto(vnum: u32, size: i32, gold: u32, flags: u32) -> ItemProto {
        let mut proto = ItemProto::for_category_rule(vnum, 0, 0);
        proto.size = size;
        proto.gold = gold;
        proto.flags = flags;
        proto
    }

    /// A dump of shops `(vnum, npc)` and items `(shop, item, count)`.
    fn dump(shops: &[(i64, i64)], items: &[(i64, i64, i64)]) -> Vec<u8> {
        let mut text = String::new();
        for (vnum, npc) in shops {
            text.push_str(&format!(
                "INSERT INTO `shop` (`vnum`, `name`, `npc_vnum`) VALUES ({vnum},'s',{npc});\n"
            ));
        }
        for (shop, item, count) in items {
            text.push_str(&format!(
                "INSERT INTO `shop_item` (`shop_vnum`, `item_vnum`, `count`) \
                 VALUES ({shop},{item},{count});\n"
            ));
        }
        text.into_bytes()
    }

    #[test]
    fn the_owner_s_renewal_shop_reads_its_tab_and_its_item() {
        let records = renewal_shops_from_dump(&owner_dump()).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!((record.vnum, record.npc_vnum), (1, 9001));
        assert_eq!(record.item_count, 1);
        assert_eq!(&record.shop_name[..4], b"test");
        assert_eq!(&record.shop_name[4..], &[0; 29]);
        let item = &record.items[0];
        assert_eq!((item.vnum, item.count, item.price), (50011, 1, 0));
        assert_eq!((item.price_type, item.price_vnum), (1, 0));
        let laid = NpcShops::lay_out_renewal(&records, &owner_protos());
        assert_eq!(laid.shops().len(), 1);
        assert!(laid.skipped().is_empty());
        let shop = &laid.shops()[0];
        assert!(shop.is_renewal());
        assert_eq!(shop.tab_name.unwrap()[..4], *b"test");
        let slot = shop.slots[0].expect("the size-1 item takes slot 0");
        assert_eq!((slot.vnum, slot.count, slot.price), (50011, 1, 0));
        assert_eq!((slot.price_type, slot.price_vnum), (1, 0));
        assert_eq!(laid.for_npc(9001).map(|shop| shop.vnum), Some(1));
    }

    /// A record's `(vnum, npc, [(item, count)])`.
    type Listed = (u32, u32, Vec<(u32, u16)>);

    fn listed(records: &[ShopTableRecord]) -> Vec<Listed> {
        records
            .iter()
            .map(|record| {
                let items = record.items[..usize::from(record.item_count)]
                    .iter()
                    .map(|item| (item.vnum, item.count))
                    .collect();
                (record.vnum, record.npc_vnum, items)
            })
            .collect()
    }

    /// Each filled slot's `(slot, vnum, price)`.
    fn filled(shop: &NpcShop) -> Vec<(usize, u32, u64)> {
        shop.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, item)| item.map(|item| (slot, item.vnum, item.price)))
            .collect()
    }

    fn lay_out_one(items: &[(i64, i64)], protos: Vec<ItemProto>) -> NpcShops {
        let items: Vec<_> = items
            .iter()
            .map(|(item, count)| (1, *item, *count))
            .collect();
        let records = shops_from_dump(&dump(&[(1, 100)], &items)).unwrap();
        NpcShops::lay_out(&records, &ItemProtos::from_rows(protos))
    }

    #[test]
    fn the_owners_shops_are_joined_and_sorted_as_the_query_sorts_them() {
        let records = shops_from_dump(&owner_dump()).unwrap();
        assert_eq!(
            records.len(),
            29,
            "shops 1 and 11 have items but no shop row"
        );
        let vnums: Vec<u32> = records.iter().map(|record| record.vnum).collect();
        let mut sorted = vnums.clone();
        sorted.sort_unstable();
        assert_eq!(vnums, sorted);
        let listed = listed(&records);
        let shop = |vnum| listed.iter().find(|shop| shop.0 == vnum).unwrap();
        assert_eq!(
            shop(8),
            &(
                8,
                9004,
                (50100..=50105).map(|vnum| (vnum, 100)).collect::<Vec<_>>()
            )
        );
        assert_eq!(
            shop(9),
            &(9, 20042, vec![(11901, 1), (11903, 1), (50201, 1)])
        );
        assert_eq!(shop(5).2.len(), 12);
        assert_eq!(shop(6).2.len(), 24);
        let items: usize = listed.iter().map(|shop| shop.2.len()).sum();
        assert_eq!(items, 262, "268 item rows, less the six of shops 1 and 11");
    }

    #[test]
    fn the_owners_keepers_open_their_shops_laid_out() {
        let shops = owner_shops();
        assert_eq!(shops.skipped(), &[]);
        let weapons = shops.for_npc(9007).unwrap();
        assert_eq!(weapons.vnum, 5);
        assert_eq!(
            filled(weapons),
            vec![
                (0, 3100, 110_000),
                (1, 5020, 3000),
                (2, 5030, 8000),
                (3, 5040, 15000),
                (4, 5050, 20000),
                (6, 5070, 140_000),
                (7, 5080, 180_000),
                (8, 7020, 600),
                (9, 7050, 5000),
                (11, 7060, 8000),
                (12, 7090, 80000),
                (13, 7100, 110_000),
            ],
            "3100 is 3 tall, so cells 5 and 10 are its own"
        );
        let stones = shops.for_npc(20042).unwrap();
        assert_eq!(
            filled(stones),
            vec![
                (0, 11901, 2_000_000),
                (1, 11903, 2_500_000),
                (2, 50201, 100_000)
            ]
        );
        let fireworks = shops.for_npc(9004).unwrap();
        assert_eq!(
            filled(fireworks),
            (0..6)
                .map(|slot| (slot, 50100 + u32::try_from(slot).unwrap(), 500_000))
                .collect::<Vec<_>>()
        );
        assert!(fireworks.slots[..6]
            .iter()
            .all(|slot| slot.unwrap().count == 100));
        let armours = filled(shops.for_npc(9008).unwrap());
        assert_eq!(armours.len(), 24);
        assert!(armours
            .iter()
            .enumerate()
            .all(|(index, item)| item.0 == index));
        assert_eq!((armours[0].1, armours[0].2), (14000, 500));
        assert_eq!((armours[11].1, armours[11].2), (15160, 4000));
        assert_eq!((armours[23].1, armours[23].2), (17180, 2000));
        let swords = filled(shops.shops().iter().find(|shop| shop.vnum == 1001).unwrap());
        let cells: Vec<usize> = swords.iter().map(|item| item.0).collect();
        assert_eq!(
            cells,
            [0, 1, 2, 3, 4, 10, 11, 12, 13, 14, 20, 21, 22],
            "a 2-tall item takes the row below it"
        );
    }

    #[test]
    fn a_keeper_opens_the_first_shop_that_names_it() {
        let shops = owner_shops();
        assert_eq!(shops.for_npc(20094).unwrap().vnum, 2000);
        assert_eq!(shops.for_npc(0).unwrap().vnum, 1002);
        assert!(shops.for_npc(9001).is_none(), "9001 is the shopex's keeper");
        assert!(shops.for_npc(9002).is_none());
    }

    #[test]
    fn the_join_keeps_a_shop_with_no_items_and_drops_an_item_with_no_shop() {
        let records = shops_from_dump(&dump(&[(2, 20), (1, 10)], &[(3, 7, 1), (1, 5, 2)])).unwrap();
        assert_eq!(
            listed(&records),
            vec![(1, 10, vec![(5, 2)]), (2, 20, vec![])]
        );
    }

    #[test]
    fn the_sort_is_numeric_and_keeps_the_dumps_order_between_equals() {
        let records = shops_from_dump(&dump(
            &[(10, 1), (-1, 2), (9, 3)],
            &[
                (10, 100, 1),
                (10, 20, 1),
                (10, 20, 3),
                (10, 20, 2),
                (-1, 5, 1),
                (9, -4, 1),
                (9, 4, 1),
            ],
        ))
        .unwrap();
        let vnums: Vec<i32> = records
            .iter()
            .map(|record| i32::from_ne_bytes(record.vnum.to_ne_bytes()))
            .collect();
        assert_eq!(vnums, [-1, 9, 10]);
        let items: Vec<Vec<(u32, u16)>> = listed(&records).into_iter().map(|shop| shop.2).collect();
        assert_eq!(items[2], [(20, 1), (20, 3), (20, 2), (100, 1)]);
        assert_eq!(
            items[1][0].0,
            u32::MAX - 3,
            "-4 sorts first, and strtoul negates it"
        );
        assert_eq!(items[1][1].0, 4);
    }

    #[test]
    fn a_value_the_query_reads_must_be_a_number() {
        let text = b"INSERT INTO `shop` (`vnum`, `name`, `npc_vnum`) VALUES (1,'s','9');";
        assert_eq!(
            shops_from_dump(text),
            Err(ShopDumpError::NotANumber {
                table: "shop",
                row: 0,
                column: "npc_vnum"
            })
        );
        let mut text = dump(&[(1, 2)], &[]);
        text.extend_from_slice(
            b"INSERT INTO `shop_item` (`shop_vnum`, `item_vnum`, `count`) \
              VALUES (1,5,1),(1,NULL,1),(1,x,1);",
        );
        assert_eq!(
            shops_from_dump(&text),
            Err(ShopDumpError::NotANumber {
                table: "shop_item",
                row: 1,
                column: "item_vnum"
            })
        );
        let text = b"INSERT INTO `shop` (`vnum`, `name`, `npc_vnum`) VALUES (1e3,'s',9);";
        assert!(matches!(
            shops_from_dump(text),
            Err(ShopDumpError::NotANumber { column: "vnum", .. })
        ));
    }

    #[test]
    fn a_missing_column_an_empty_table_and_a_bad_dump_are_refused() {
        let text = b"INSERT INTO `shop` (`vnum`, `name`) VALUES (1,'s');";
        assert_eq!(
            shops_from_dump(text),
            Err(ShopDumpError::NoColumn {
                table: "shop",
                column: "npc_vnum"
            })
        );
        let mut text = dump(&[(1, 2)], &[]);
        text.extend_from_slice(b"INSERT INTO `shop_item` (`shop_vnum`, `count`) VALUES (1,1);");
        assert_eq!(
            shops_from_dump(&text),
            Err(ShopDumpError::NoColumn {
                table: "shop_item",
                column: "item_vnum"
            })
        );
        assert_eq!(
            shops_from_dump(b"-- nothing\n"),
            Err(ShopDumpError::Table(ShopTableError::EmptySource))
        );
        assert!(matches!(
            shops_from_dump(b"INSERT INTO `shop` VALUES (1);"),
            Err(ShopDumpError::Dump(_))
        ));
        assert!(matches!(
            shops_from_dump(b"INSERT INTO `shop_item` VALUES (1);"),
            Err(ShopDumpError::Dump(_))
        ));
    }

    #[test]
    fn a_price_is_gold_per_count_or_count_per_gold() {
        let plain = proto(1, 1, 7, 0);
        assert_eq!(shop_price(&plain, 3), 21);
        let wide = proto(1, 1, u32::MAX, ITEM_FLAG_COUNT_PER_1GOLD >> 1);
        assert_eq!(
            shop_price(&wide, 2),
            u64::from(u32::MAX) * 2,
            "the Rewrite does not wrap at 32 bits"
        );
        let per_gold = proto(1, 1, 3, ITEM_FLAG_COUNT_PER_1GOLD);
        assert_eq!(shop_price(&per_gold, 10), 3);
        assert_eq!(shop_price(&per_gold, 2), 0);
        let free = proto(1, 1, 0, ITEM_FLAG_COUNT_PER_1GOLD);
        assert_eq!(shop_price(&free, 10), 10);
        assert_eq!(ITEM_FLAG_COUNT_PER_1GOLD, 8);
    }

    #[test]
    fn an_item_that_cannot_be_placed_is_skipped_alone() {
        let shops = lay_out_one(
            &[(1, 1), (2, 1), (3, 1), (4, 1), (5, 1), (6, 1)],
            vec![
                proto(1, 1, 1, 0),
                proto(3, 0, 1, 0),
                proto(4, 10, 1, 0),
                proto(5, 257, 2, 0),
                proto(6, -1, 1, 0),
            ],
        );
        let shop = shops.for_npc(100).unwrap();
        assert_eq!(
            filled(shop),
            vec![(0, 1, 1), (1, 5, 2)],
            "257's low byte is 1"
        );
        let skip = |vnum, skip| SkippedShopItem {
            shop_vnum: 1,
            vnum,
            skip,
        };
        assert_eq!(
            shops.skipped(),
            &[
                skip(2, ShopSkip::NoProto),
                skip(3, ShopSkip::NoSize),
                skip(4, ShopSkip::NoRoom { size: 10 }),
                skip(6, ShopSkip::NoRoom { size: 255 }),
            ]
        );
    }

    #[test]
    fn a_column_must_fit_above_the_last_row() {
        // A 9-tall item holds all of column 0, so the 1-tall items after it pass over
        // cells 5 and 10.
        let mut items = vec![(9, 1)];
        items.extend((10..16).map(|vnum| (vnum, 1)));
        let mut protos = vec![proto(9, 9, 1, 0)];
        protos.extend((10..16).map(|vnum| proto(vnum, 1, 1, 0)));
        let shops = lay_out_one(&items, protos);
        let cells: Vec<usize> = filled(shops.for_npc(100).unwrap())
            .iter()
            .map(|item| item.0)
            .collect();
        assert_eq!(
            cells,
            [0, 1, 2, 3, 4, 6, 7],
            "the 9-tall item holds column 0"
        );
        // With row 0 full, a 9-tall item would end past the last row wherever it started.
        let mut items: Vec<(i64, i64)> = (1..=5).map(|vnum| (vnum, 1)).collect();
        items.extend([(6, 1), (7, 1)]);
        let mut protos: Vec<ItemProto> = (1..=5).map(|vnum| proto(vnum, 1, 1, 0)).collect();
        protos.extend([proto(6, 9, 1, 0), proto(7, 8, 1, 0)]);
        let shops = lay_out_one(&items, protos);
        assert_eq!(filled(shops.for_npc(100).unwrap())[5], (5, 7, 1));
        assert_eq!(
            shops.skipped(),
            &[SkippedShopItem {
                shop_vnum: 1,
                vnum: 6,
                skip: ShopSkip::NoRoom { size: 9 }
            }]
        );
    }

    #[test]
    fn a_cell_past_the_slots_is_skipped_and_still_taken() {
        // Three 8-tall items hold columns 0 to 2 down to row 7, and 14 1-tall items fill
        // columns 3 and 4 down to row 6. A 2-tall item then takes 38 and 43, and a 1-tall item
        // 39. The next 1-tall item finds 40, past the slots, and the last finds 41, since
        // legacy marked 40 taken.
        let mut items: Vec<(i64, i64)> = (1..=17).map(|vnum| (vnum, 1)).collect();
        items.extend([(100, 1), (101, 1), (102, 1), (103, 1)]);
        let mut protos: Vec<ItemProto> = (1..=3).map(|vnum| proto(vnum, 8, 1, 0)).collect();
        protos.extend((4..=17).map(|vnum| proto(vnum, 1, 1, 0)));
        protos.extend([
            proto(100, 2, 1, 0),
            proto(101, 1, 1, 0),
            proto(102, 1, 1, 0),
            proto(103, 1, 1, 0),
        ]);
        let shops = lay_out_one(&items, protos);
        let shop = shops.for_npc(100).unwrap();
        assert_eq!(shop.slots[38].unwrap().vnum, 100);
        assert_eq!(shop.slots[39].unwrap().vnum, 101);
        assert_eq!(filled(shop).len(), 19);
        assert_eq!(
            shops.skipped(),
            &[
                SkippedShopItem {
                    shop_vnum: 1,
                    vnum: 102,
                    skip: ShopSkip::PastTheSlots { cell: 40 }
                },
                SkippedShopItem {
                    shop_vnum: 1,
                    vnum: 103,
                    skip: ShopSkip::PastTheSlots { cell: 41 }
                },
            ]
        );
    }

    #[test]
    fn a_blank_needs_every_cell_of_its_column_free() {
        // The layout fills each column from the top, so it never leaves a free cell above a
        // taken one; this grid is built by hand. Cell 5 is taken and cell 0 is free.
        let mut grid = [false; SHOP_GRID_CELLS];
        grid[5] = true;
        assert_eq!(find_blank(&grid, 1), Some(0));
        assert_eq!(
            find_blank(&grid, 2),
            Some(1),
            "column 0 is blocked at row 1"
        );
        grid[36] = true;
        assert_eq!(
            find_blank(&grid, 8),
            Some(2),
            "column 1 is blocked at row 7"
        );
        assert_eq!(find_blank(&grid, 9), Some(2));
    }

    #[test]
    fn the_items_stop_at_the_first_empty_vnum() {
        // Item 0 sorts first, so `Create` counts no items at all.
        let shops = lay_out_one(&[(5, 1), (0, 1)], vec![proto(5, 1, 1, 0)]);
        assert_eq!(filled(shops.for_npc(100).unwrap()), vec![]);
        assert_eq!(shops.skipped(), &[]);
        assert_eq!(shops.shops().len(), 1);
    }
}
