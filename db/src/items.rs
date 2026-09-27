//! Items, as the store reads and writes them.
//!
//! Every value reaches SQL as a bound parameter. The column list is built from
//! literals in this file, so a query never carries a name this file does not
//! already know; a value is never formatted into a statement.
//!
//! This crate cannot see `world::item`, because the world is above the store, so
//! [`ItemRow`] repeats the instance record instead of borrowing it. The two are
//! **not** the same shape, and the difference is deliberate, so the rule is stated
//! here rather than left to a reader:
//!
//! | | `ItemRow` | `world::item::Item` |
//! |---|---|---|
//! | the nine instance fields | the same names, the same widths | the same |
//! | where it is | `owner_id` and `window_type` separately | `pos: ItemPos`, which bundles both |
//! | the grid footprint | **absent** | `size: u8` |
//!
//! `Item::size` is a prototype fact (`TItemTable::bSize`), not a stored one, so the
//! table has no column for it and the load cannot fill it: the value comes from
//! `gamedata::item_proto` when the world builds the instance. `pos` is a
//! `protocol::item_pos::ItemPos` on one side and two columns on the other, so the
//! split is at the storage boundary and is not a drift.
//!
//! A field renamed on one side and not the other is a silent corruption rather than
//! a compile error, so `prodomo/tests/item_row.rs` pins the nine shared names and the
//! two widths that matter.
//!
//! Two rules the legacy server does not enforce, and this module does.
//!
//! * **Never `REPLACE`.** Legacy's save is `REPLACE INTO item` (`db/Cache.cpp:178`) and
//!   with one unique key that was safe. This table has a second one --
//!   `(owner_id, window_type, pos)` -- and MySQL's `REPLACE` answers a conflict by
//!   *deleting* the other row. A save that moves an item would destroy the item it
//!   landed on. Every write here is `INSERT ... ON CONFLICT (id) DO UPDATE`.
//! * **Delete on `id` and `owner_id`.** Legacy's destroy (`db/ClientManager.cpp:1838`) is
//!   `DELETE FROM item%s WHERE id=%u`. It is handed the owning pid, uses it only for
//!   the log line and the branch, and then deletes on the id alone across a global id
//!   space. [`destroy_item`] takes the owner and puts it in the `WHERE` clause.

use std::error::Error;
use std::fmt;

use common::constants::{ITEM_ATTRIBUTE_MAX_NUM, ITEM_MAX_COUNT, ITEM_SOCKET_MAX_NUM};
use sqlx::postgres::{PgArguments, PgRow};
use sqlx::query::Query;
use sqlx::{Postgres, Row};

use crate::store::Store;

/// [`ITEM_SOCKET_MAX_NUM`] as a `usize`, which array lengths and const generics need.
///
/// Public because a caller sizing a buffer wants the measured count, not a cast.
pub const SOCKETS: usize = ITEM_SOCKET_MAX_NUM as usize;

/// [`ITEM_ATTRIBUTE_MAX_NUM`] as a `usize`, which array lengths and const generics need.
const ATTRS: usize = ITEM_ATTRIBUTE_MAX_NUM as usize;

/// The socket columns, in index order.
pub const SOCKET_COLUMNS: [&str; SOCKETS] = [
    "socket0", "socket1", "socket2", "socket3", "socket4", "socket5",
];

/// The attribute type columns, in index order.
pub const ATTRTYPE_COLUMNS: [&str; ATTRS] = [
    "attrtype0",
    "attrtype1",
    "attrtype2",
    "attrtype3",
    "attrtype4",
    "attrtype5",
    "attrtype6",
];

/// The attribute value columns, in index order.
pub const ATTRVALUE_COLUMNS: [&str; ATTRS] = [
    "attrvalue0",
    "attrvalue1",
    "attrvalue2",
    "attrvalue3",
    "attrvalue4",
    "attrvalue5",
    "attrvalue6",
];

/// The largest an item id may be (`ItemIDRangeManager::cs_dwMaxItemID`, `ItemIDRangeManager.h:8`).
///
/// The migration's `item_id_check` uses the same value, and this is the one the Rust
/// side narrows against so a corrupt row is reported rather than truncated.
pub const MAX_ITEM_ID: u32 = 4_290_000_000;

/// The window byte for an item lying on the ground (`EWindows::GROUND`).
pub const GROUND: u8 = 10;

/// One attribute: an unsigned type and a signed value (`TPlayerItemAttribute`,
/// `tables.h:426-430`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attribute {
    /// `bType`. 0 means the slot is empty.
    pub b_type: u8,
    /// `sValue`. Meaningless while `b_type` is 0, and not checked.
    ///
    /// Named `s_value` and not `value`, to match `protocol::gc_item_window::ItemAttribute`
    /// and the legacy `TPlayerItemAttribute::sValue` it carries. A store-side rename is
    /// exactly the drift `prodomo/tests/item_row.rs` exists to catch, and it is cheaper
    /// not to create one.
    pub s_value: i16,
}

/// One stored item, field for field what an instance is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    /// The id the client sees. Never 0.
    pub id: u32,
    /// The owning character, or `None` for an item on the ground.
    pub owner_id: Option<u32>,
    /// The window byte, 0 to 10.
    pub window_type: u8,
    /// The cell, or whatever the window says it is.
    pub pos: u32,
    /// `TItemData::vnum` (`packet.h:2973`).
    pub vnum: u32,
    /// `TItemData::count` (`packet.h:2974`), 1 to [`ITEM_MAX_COUNT`].
    pub count: u16,
    /// `TItemData::dwRefineElement` (`packet.h:2976`).
    pub refine_element: u32,
    /// `TItemData::transmutation` (`packet.h:2979`).
    pub transmutation: u32,
    /// `TItemData::flags` (`packet.h:2981`).
    pub flags: u32,
    /// `TItemData::anti_flags` (`packet.h:2982`).
    pub anti_flags: u32,
    /// `TItemData::alSockets[6]` (`packet.h:2983`).
    pub sockets: [i32; SOCKETS],
    /// `TItemData::aAttr[7]` (`packet.h:2984`).
    pub attributes: [Attribute; ATTRS],
}

impl ItemRow {
    /// An item lying on the ground at a world position.
    ///
    /// The world position is not stored in this table, so the `pos` a ground row
    /// carries is a map index and not a cell, and the ground system is `sys.item.ground`,
    /// a later unit. This constructor exists so the next unit does not invent a
    /// second shape for it.
    #[must_use]
    pub fn on_ground(id: u32, map_index: u32, vnum: u32, count: u16) -> Self {
        Self {
            id,
            owner_id: None,
            window_type: GROUND,
            pos: map_index,
            vnum,
            count,
            refine_element: 0,
            transmutation: 0,
            flags: 0,
            anti_flags: 0,
            sockets: [0; SOCKETS],
            attributes: [Attribute {
                b_type: 0,
                s_value: 0,
            }; ATTRS],
        }
    }

    /// The `(type, value)` pair at `index`, or `None` when the index is out of range.
    ///
    /// A `get`, not an index: a column name built from an index that was already
    /// checked is fine, but a caller-supplied one is not, and this is the one place
    /// a caller supplies one.
    #[must_use]
    pub fn attribute(&self, index: usize) -> Option<Attribute> {
        self.attributes.get(index).copied()
    }
}

/// Why an item operation failed.
#[derive(Debug)]
pub enum ItemError {
    /// The character does not exist, so no row can be owned by it.
    NoSuchOwner(u32),
    /// The window byte is outside 0 to 10.
    WindowOutOfRange(u8),
    /// The count is outside 1 to [`ITEM_MAX_COUNT`].
    CountOutOfRange(u16),
    /// The id is 0, or above [`MAX_ITEM_ID`].
    IdOutOfRange(u32),
    /// No row has the id. Distinct from [`ItemError::NotOwned`], which means the id
    /// exists and belongs to somebody else.
    NoSuchItem(u32),
    /// A row for the id exists and belongs to a different character.
    NotOwned {
        /// The id.
        id: u32,
        /// The character that holds it, or `None` when it is on the ground.
        owner_id: Option<u32>,
    },
    /// A stored value breaks a rule the schema should have enforced.
    Corrupt(String),
    /// The server refused or failed a query.
    Database(sqlx::Error),
}

impl fmt::Display for ItemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchOwner(id) => write!(f, "no character with id {id}"),
            Self::WindowOutOfRange(window) => write!(f, "window {window} is not 0 to 10"),
            Self::CountOutOfRange(count) => {
                write!(f, "count {count} is not 1 to {ITEM_MAX_COUNT}")
            }
            Self::IdOutOfRange(id) => write!(f, "id {id} is not 1 to {MAX_ITEM_ID}"),
            Self::NoSuchItem(id) => write!(f, "no item with id {id}"),
            Self::NotOwned { id, owner_id } => match owner_id {
                Some(owner) => write!(
                    f,
                    "item {id} is owned by character {owner}, not by this one"
                ),
                None => write!(f, "item {id} is on the ground"),
            },
            Self::Corrupt(detail) => write!(f, "stored item data is invalid: {detail}"),
            Self::Database(error) => write!(f, "PostgreSQL error: {error}"),
        }
    }
}

impl Error for ItemError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for ItemError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// The plain columns, in the order the load reads them.
const PLAIN_COLUMNS: &str = "id, owner_id, window_type, pos, vnum, count, refine_element, \
                            transmutation, flags, anti_flags";

/// The six socket columns joined for a `SELECT`.
fn socket_columns() -> String {
    SOCKET_COLUMNS.join(", ")
}

/// The seven attribute type columns joined for a `SELECT`.
fn attrtype_columns() -> String {
    ATTRTYPE_COLUMNS.join(", ")
}

/// The seven attribute value columns joined for a `SELECT`.
fn attrvalue_columns() -> String {
    ATTRVALUE_COLUMNS.join(", ")
}

/// Every column except `created_at`, in the order the insert binds them.
fn all_columns() -> String {
    let mut columns = String::from(PLAIN_COLUMNS);
    for name in SOCKET_COLUMNS {
        columns.push_str(", ");
        columns.push_str(name);
    }
    for (index, name) in ATTRTYPE_COLUMNS.iter().enumerate() {
        columns.push_str(", ");
        columns.push_str(name);
        columns.push_str(", ");
        columns.push_str(ATTRVALUE_COLUMNS[index]);
    }
    columns
}

/// `$n` placeholders for `count` values, starting at `first`.
fn placeholders(count: usize, first: usize) -> String {
    let mut out = String::new();
    for index in 0..count {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('$');
        out.push_str(&(first + index).to_string());
    }
    out
}

/// The one `SELECT` every load here runs: the thirty columns, named in the order
/// [`row_from`] reads them.
///
/// # Ordered?
///
/// The `ORDER BY` lives with the query that needs it, not here, because two of the
/// three loads want a single row by id and a third orders by the owner's cells.
fn select_sql() -> String {
    let mut sql = format!("SELECT {PLAIN_COLUMNS}, ");
    sql.push_str(&socket_columns());
    sql.push_str(", ");
    sql.push_str(&attrtype_columns());
    sql.push_str(", ");
    sql.push_str(&attrvalue_columns());
    sql.push_str(" FROM item");
    sql
}

/// Decode one row, or report it as corrupt.
///
/// Each column is read as the Rust type that **matches the column's PostgreSQL
/// type**, not as one width for all of them. sqlx refuses a mismatch in both
/// directions -- reading an `integer` as `i64` is `mismatched types; Rust type i64
/// (as SQL type INT8) is not compatible with SQL type INT4` -- so the widths here
/// are the table's: `bigint` for the six `DWORD`s, `integer` for `pos` and the
/// sockets, `smallint` for the window, the count and both halves of every
/// attribute.
fn row_from(row: &PgRow) -> Result<ItemRow, ItemError> {
    let sockets = decode_array::<SOCKETS, i32>(row, &SOCKET_COLUMNS, read_i32)?;
    let types = decode_array::<ATTRS, i16>(row, &ATTRTYPE_COLUMNS, read_i16)?;
    let values = decode_array::<ATTRS, i16>(row, &ATTRVALUE_COLUMNS, read_i16)?;
    let mut attributes = [Attribute {
        b_type: 0,
        s_value: 0,
    }; ATTRS];
    for (slot, (b_type, s_value)) in attributes.iter_mut().zip(types.into_iter().zip(values)) {
        // A `smallint` column narrowed into a `u8`. The migration's CHECK already
        // refuses -1, so this cannot fire for a row this crate wrote, and it fires
        // loudly for one written by hand.
        *slot = Attribute {
            b_type: u8::try_from(b_type).map_err(|_| corrupt("attrtype", b_type))?,
            s_value,
        };
    }
    Ok(ItemRow {
        id: narrow(row.try_get::<i64, _>("id")?, "id")?,
        owner_id: row
            .try_get::<Option<i32>, _>("owner_id")?
            .map(|id| narrow(id, "owner_id"))
            .transpose()?,
        window_type: u8::try_from(row.try_get::<i16, _>("window_type")?).map_err(|_| {
            corrupt(
                "window_type",
                row.try_get::<i16, _>("window_type").unwrap_or(0),
            )
        })?,
        pos: narrow(row.try_get::<i32, _>("pos")?, "pos")?,
        vnum: narrow(row.try_get::<i64, _>("vnum")?, "vnum")?,
        count: u16::try_from(row.try_get::<i16, _>("count")?)
            .map_err(|_| corrupt("count", row.try_get::<i16, _>("count").unwrap_or(0)))?,
        refine_element: narrow(row.try_get::<i64, _>("refine_element")?, "refine_element")?,
        transmutation: narrow(row.try_get::<i64, _>("transmutation")?, "transmutation")?,
        flags: narrow(row.try_get::<i64, _>("flags")?, "flags")?,
        anti_flags: narrow(row.try_get::<i64, _>("anti_flags")?, "anti_flags")?,
        sockets,
        attributes,
    })
}

/// Read `N` columns by name with one reader, which is how the six sockets and the
/// seven attribute pairs are read without six and seven near-identical lines.
fn decode_array<const N: usize, T>(
    row: &PgRow,
    columns: &[&str; N],
    read: fn(&PgRow, &str) -> Result<T, ItemError>,
) -> Result<[T; N], ItemError> {
    let mut out = Vec::with_capacity(N);
    for name in columns {
        out.push(read(row, name)?);
    }
    out.try_into()
        .map_err(|_| ItemError::Corrupt(format!("{N} columns did not decode")))
}

/// A corrupt-row error naming the column and the value.
fn corrupt(column: &str, value: impl fmt::Display) -> ItemError {
    ItemError::Corrupt(format!("item column {column} holds {value}"))
}

/// Narrow a value to the width the client is sent, or report the row as corrupt.
fn narrow<From, To>(value: From, column: &str) -> Result<To, ItemError>
where
    From: Copy + fmt::Display,
    To: TryFrom<From>,
{
    To::try_from(value).map_err(|_| corrupt(column, value))
}

/// Read an `integer` column, which is how `pos` and the six sockets are stored.
fn read_i32(row: &PgRow, column: &str) -> Result<i32, ItemError> {
    Ok(row.try_get::<i32, _>(column)?)
}

/// Read a `smallint` column, which is how the window, the count and both halves of
/// every attribute are stored.
fn read_i16(row: &PgRow, column: &str) -> Result<i16, ItemError> {
    Ok(row.try_get::<i16, _>(column)?)
}

/// Load every item of one character.
///
/// Legacy loads the same set (`db/ClientManagerPlayer.cpp:386`, the six `INVENTORY`
/// windows). It filters in the query (`row[0] = 0` and the switch) and **silently drops**
/// a row it does not recognise, so a character that owns an item in a window this build
/// does not have loses it at the next login. This function does not repeat that: the only
/// filter is the owner, and a row this build cannot decode is an [`ItemError::Corrupt`]
/// rather than a missing item.
///
/// Ground items are not returned, and that is not a filter: the migration's biconditional
/// makes a row with an owner never a ground row. The ground load is `sys.item.ground`, a
/// later unit.
///
/// # Errors
///
/// Returns [`ItemError::Corrupt`] for a row that cannot be decoded, or
/// [`ItemError::Database`].
pub async fn load_owner_items(store: &Store, owner_id: u32) -> Result<Vec<ItemRow>, ItemError> {
    // `owner_id = $1` is the only filter, and that is deliberate: the migration's
    // biconditional makes a row with an owner never a ground row, so adding
    // `AND window_type <> 10` would be a second copy of a rule that already holds.
    //
    // The order is window, then cell, then id, so the load is deterministic: a world
    // that hands the client its items in load order needs exactly this order.
    let sql = format!(
        "{} WHERE owner_id = $1 ORDER BY window_type, pos, id",
        select_sql()
    );
    let rows = sqlx::query(&sql)
        .bind(i64::from(owner_id))
        .fetch_all(store.pool())
        .await?;
    rows.iter().map(row_from).collect()
}

/// Load one item by id, whoever owns it.
///
/// # Errors
///
/// Returns [`ItemError::Corrupt`] for a row that cannot be decoded, or
/// [`ItemError::Database`].
pub async fn load_item(store: &Store, id: u32) -> Result<Option<ItemRow>, ItemError> {
    let sql = format!("{} WHERE id = $1", select_sql());
    let row = sqlx::query(&sql)
        .bind(i64::from(id))
        .fetch_optional(store.pool())
        .await?;
    row.as_ref().map(row_from).transpose()
}

/// Write one item, creating it when the id is new and replacing it when it is not.
///
/// This is an upsert on the primary key, so an item that moves between cells and keeps
/// its id is updated in place. It is deliberately **not** `REPLACE`; see the module note.
/// A second unique key makes the difference load-bearing: a save that lands on an occupied
/// cell is **refused**, and neither row is touched.
///
/// # Errors
///
/// Returns [`ItemError::IdOutOfRange`], [`ItemError::WindowOutOfRange`],
/// [`ItemError::CountOutOfRange`], [`ItemError::Corrupt`] for a broken ground/owner
/// biconditional, or [`ItemError::Database`] -- which is what an occupied cell is, since
/// the cell index is the database's rule and not a Rust one.
pub async fn save_item(store: &Store, item: &ItemRow) -> Result<(), ItemError> {
    let count = check(item)?;
    let sql = format!(
        "INSERT INTO item ({}) VALUES ({}) ON CONFLICT (id) DO UPDATE SET \
         owner_id = EXCLUDED.owner_id, window_type = EXCLUDED.window_type, \
         pos = EXCLUDED.pos, count = EXCLUDED.count, vnum = EXCLUDED.vnum, \
         refine_element = EXCLUDED.refine_element, transmutation = EXCLUDED.transmutation, \
         flags = EXCLUDED.flags, anti_flags = EXCLUDED.anti_flags, \
         socket0 = EXCLUDED.socket0, socket1 = EXCLUDED.socket1, \
         socket2 = EXCLUDED.socket2, socket3 = EXCLUDED.socket3, \
         socket4 = EXCLUDED.socket4, socket5 = EXCLUDED.socket5, \
         attrtype0 = EXCLUDED.attrtype0, attrvalue0 = EXCLUDED.attrvalue0, \
         attrtype1 = EXCLUDED.attrtype1, attrvalue1 = EXCLUDED.attrvalue1, \
         attrtype2 = EXCLUDED.attrtype2, attrvalue2 = EXCLUDED.attrvalue2, \
         attrtype3 = EXCLUDED.attrtype3, attrvalue3 = EXCLUDED.attrvalue3, \
         attrtype4 = EXCLUDED.attrtype4, attrvalue4 = EXCLUDED.attrvalue4, \
         attrtype5 = EXCLUDED.attrtype5, attrvalue5 = EXCLUDED.attrvalue5, \
         attrtype6 = EXCLUDED.attrtype6, attrvalue6 = EXCLUDED.attrvalue6",
        all_columns(),
        placeholders(30, 1)
    );
    bind_item(sqlx::query(&sql), item, count)
        .execute(store.pool())
        .await?;
    Ok(())
}

/// Write a character's whole inventory in one transaction.
///
/// ADR-0003: a Transfer commits when it happens, and other state is written in the
/// background. This is the background write, so it is one transaction: a half-written
/// inventory is worse than a late one, and a partial one is what a process killed
/// mid-loop would leave. One refused row rolls the whole call back.
///
/// # Errors
///
/// Returns the first [`ItemError`], leaving the transaction to roll back, or
/// [`ItemError::Database`].
pub async fn save_owner_items(store: &Store, items: &[ItemRow]) -> Result<(), ItemError> {
    if items.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "INSERT INTO item ({}) VALUES ({}) ON CONFLICT (id) DO UPDATE SET \
         owner_id = EXCLUDED.owner_id, window_type = EXCLUDED.window_type, \
         pos = EXCLUDED.pos, count = EXCLUDED.count, vnum = EXCLUDED.vnum, \
         refine_element = EXCLUDED.refine_element, transmutation = EXCLUDED.transmutation, \
         flags = EXCLUDED.flags, anti_flags = EXCLUDED.anti_flags, \
         socket0 = EXCLUDED.socket0, socket1 = EXCLUDED.socket1, \
         socket2 = EXCLUDED.socket2, socket3 = EXCLUDED.socket3, \
         socket4 = EXCLUDED.socket4, socket5 = EXCLUDED.socket5, \
         attrtype0 = EXCLUDED.attrtype0, attrvalue0 = EXCLUDED.attrvalue0, \
         attrtype1 = EXCLUDED.attrtype1, attrvalue1 = EXCLUDED.attrvalue1, \
         attrtype2 = EXCLUDED.attrtype2, attrvalue2 = EXCLUDED.attrvalue2, \
         attrtype3 = EXCLUDED.attrtype3, attrvalue3 = EXCLUDED.attrvalue3, \
         attrtype4 = EXCLUDED.attrtype4, attrvalue4 = EXCLUDED.attrvalue4, \
         attrtype5 = EXCLUDED.attrtype5, attrvalue5 = EXCLUDED.attrvalue5, \
         attrtype6 = EXCLUDED.attrtype6, attrvalue6 = EXCLUDED.attrvalue6",
        all_columns(),
        placeholders(30, 1)
    );
    let mut transaction = store.pool().begin().await?;
    for item in items {
        let count = check(item)?;
        bind_item(sqlx::query(&sql), item, count)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// Delete one item, and only if `owner_id` still holds it.
///
/// The owner is in the `WHERE` clause, which is the difference from legacy's
/// `DELETE FROM item%s WHERE id=%u` (`db/ClientManager.cpp:1838`). Legacy is handed the
/// owning pid, uses it only for the log line and the branch, and then deletes on the id
/// alone across a global id space, so a stale id in a client request can delete an item
/// that has since been handed to somebody else.
///
/// # Errors
///
/// Returns [`ItemError::NotOwned`] when the id exists and is not this character's, or
/// [`ItemError::Database`].
pub async fn destroy_item(store: &Store, id: u32, owner_id: u32) -> Result<bool, ItemError> {
    let deleted = sqlx::query("DELETE FROM item WHERE id = $1 AND owner_id = $2")
        .bind(i64::from(id))
        .bind(i64::from(owner_id))
        .execute(store.pool())
        .await?
        .rows_affected();
    if deleted > 0 {
        return Ok(true);
    }
    match load_item(store, id).await? {
        None => Ok(false),
        Some(item) => Err(ItemError::NotOwned {
            id,
            owner_id: item.owner_id,
        }),
    }
}

/// Set one item's stack size.
///
/// This is the shape a Transfer takes: one statement that either changes the count or
/// does not, so the client and the row never disagree about a stack. The owner is in the
/// `WHERE` clause for the same reason as [`destroy_item`].
///
/// # Errors
///
/// Returns [`ItemError::CountOutOfRange`] without touching the row,
/// [`ItemError::NotOwned`] when the id is not this character's,
/// [`ItemError::NoSuchItem`] when there is no such id, or [`ItemError::Database`].
pub async fn set_count(store: &Store, id: u32, owner_id: u32, count: u16) -> Result<(), ItemError> {
    if count == 0 || u32::from(count) > ITEM_MAX_COUNT {
        return Err(ItemError::CountOutOfRange(count));
    }
    // The column is a `smallint`, so the bind has to be a `smallint` too: sqlx encodes a
    // bound value as the type it is given and the server parses `$3` as the column's
    // type, so an `i64` bind would put 8 bytes where it reads 2.
    let column = i16::try_from(count).map_err(|_| ItemError::CountOutOfRange(count))?;
    let updated = sqlx::query("UPDATE item SET count = $3 WHERE id = $1 AND owner_id = $2")
        .bind(i64::from(id))
        .bind(i64::from(owner_id))
        .bind(column)
        .execute(store.pool())
        .await?
        .rows_affected();
    if updated > 0 {
        return Ok(());
    }
    match load_item(store, id).await? {
        None => Err(ItemError::NoSuchItem(id)),
        Some(item) => Err(ItemError::NotOwned {
            id,
            owner_id: item.owner_id,
        }),
    }
}

/// The highest allocated id in a range, or `None` when the range holds nothing.
///
/// Legacy answers this with `SELECT MAX(id) FROM item%s WHERE id >= %u and id <= %u`
/// (`db/ItemIDRangeManager.cpp:94`) so its pool starts above what is already written.
/// In one process that is the same question [`crate::item_id_range`] already models
/// arithmetically for the game thread, so this exists for the Operator path, which has to
/// answer it once at startup rather than on every allocation.
///
/// # Errors
///
/// Returns [`ItemError::Database`].
pub async fn max_id_in_range(
    store: &Store,
    lowest: u32,
    highest: u32,
) -> Result<Option<u32>, ItemError> {
    let max: Option<i64> =
        sqlx::query_scalar("SELECT MAX(id) FROM item WHERE id >= $1 AND id <= $2")
            .bind(i64::from(lowest))
            .bind(i64::from(highest))
            .fetch_one(store.pool())
            .await?;
    max.map(|value| narrow(value, "MAX(id)")).transpose()
}

/// Bind one item's thirty columns, in the order [`all_columns`] lists them.
///
/// `count` is the validated `smallint` that [`check`] returned. It is passed in
/// rather than converted here so there is one conversion, and it is the value that
/// was checked, not a re-derivation of it.
fn bind_item<'q>(
    query: Query<'q, Postgres, PgArguments>,
    item: &ItemRow,
    count: i16,
) -> Query<'q, Postgres, PgArguments> {
    let mut query = query
        .bind(i64::from(item.id))
        .bind(item.owner_id.map(i64::from))
        .bind(i16::from(item.window_type))
        .bind(i64::from(item.pos))
        .bind(i64::from(item.vnum))
        .bind(count)
        .bind(i64::from(item.refine_element))
        .bind(i64::from(item.transmutation))
        .bind(i64::from(item.flags))
        .bind(i64::from(item.anti_flags));
    for socket in item.sockets {
        query = query.bind(socket);
    }
    for attribute in item.attributes {
        query = query
            .bind(i16::from(attribute.b_type))
            .bind(attribute.s_value);
    }
    query
}

/// The rules the migration also enforces, checked here so the error names the value.
///
/// It returns the count as the `smallint` the column is, because two `smallint` binds
/// need one and a second conversion is a second place to get it wrong. A `u16` wider
/// than `i16` cannot reach here: the column is 1 to 5000.
fn check(item: &ItemRow) -> Result<i16, ItemError> {
    if item.id == 0 || item.id > MAX_ITEM_ID {
        return Err(ItemError::IdOutOfRange(item.id));
    }
    if item.window_type > GROUND {
        return Err(ItemError::WindowOutOfRange(item.window_type));
    }
    let count = if item.count == 0 || u32::from(item.count) > ITEM_MAX_COUNT {
        return Err(ItemError::CountOutOfRange(item.count));
    } else {
        i16::try_from(item.count).map_err(|_| ItemError::CountOutOfRange(item.count))?
    };
    // The biconditional, checked before the statement so a caller learns which half
    // it broke rather than reading a constraint name.
    if item.owner_id.is_none() != (item.window_type == GROUND) {
        return Err(ItemError::Corrupt(format!(
            "item {} is {} but its window is {}",
            item.id,
            if item.owner_id.is_none() {
                "on the ground"
            } else {
                "owned"
            },
            item.window_type
        )));
    }
    Ok(count)
}
