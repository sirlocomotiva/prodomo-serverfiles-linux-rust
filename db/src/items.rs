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
//! | where it is | `owner_id` or `account_id`, and `window_type` | `pos: ItemPos`, which bundles both |
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
//!
//! A `SAFEBOX` or `MALL` row belongs to the account and not to a character (ADR-0005): it
//! carries `account_id` and no `owner_id`. A checkin or a checkout is one Transfer between a
//! character and its account ([`RowChange::Stored`] and [`RowChange::Retrieved`]), and a move
//! inside the safebox is [`apply_account_changes`].

use std::error::Error;
use std::fmt;

use common::config::ItemIdSpan;
use common::constants::{ITEM_ATTRIBUTE_MAX_NUM, ITEM_MAX_COUNT, ITEM_SOCKET_MAX_NUM};
use sqlx::postgres::{PgArguments, PgRow};
use sqlx::query::Query;
use sqlx::{Postgres, Row};

use crate::item_id_range::MINIMUM_REMAIN_COUNT;
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

/// The window byte of an account's safebox (`EWindows::SAFEBOX`).
pub const SAFEBOX: u8 = 3;

/// The window byte of an account's item mall (`EWindows::MALL`).
pub const MALL: u8 = 4;

/// How many columns [`all_columns`] names, which is how many values an insert binds.
const COLUMN_COUNT: usize = 31;

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
    /// The owning character, or `None` for an item on the ground or in an account's window.
    pub owner_id: Option<u32>,
    /// The owning account of a `SAFEBOX` or `MALL` row (ADR-0005), and `None` for every other
    /// window. A row has at most one of `owner_id` and `account_id`.
    pub account_id: Option<u32>,
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
            account_id: None,
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
    /// A row for the id exists and belongs to a different character or account.
    NotOwned {
        /// The id.
        id: u32,
        /// The character that holds it, or `None` when it is on the ground or in an account's
        /// window.
        owner_id: Option<u32>,
        /// The account that holds it, or `None` when it is not in an account's window.
        account_id: Option<u32>,
    },
    /// A checkin or a checkout named an account the character does not belong to.
    ForeignAccount {
        /// The character.
        owner_id: u32,
        /// The account the change named.
        account: u32,
    },
    /// The configured item-id span has fewer than
    /// [`MINIMUM_REMAIN_COUNT`] ids left above the id the allocator would start at.
    ///
    /// Legacy `CItemIDRangeManager::BuildRange` logged this and returned false, which
    /// made `CClientManager::InitializeNowItemID` fail the boot.
    ItemIdRangeExhausted {
        /// `dwMin`, the configured first id.
        first: u32,
        /// `dwMax`, the configured last id.
        last: u32,
        /// `dwUsableItemIDMin`, the id the allocator would have started at.
        next: u32,
    },
    /// An item row sits between the id the allocator would start at and the last id.
    ///
    /// Unreachable through the two-step path that produces `next`, because `next` is
    /// `MAX(id) + 1`. It is legacy's belt-and-braces check and it catches the one
    /// thing the other two queries cannot: a row inserted between them.
    ItemIdRangeOccupied {
        /// `dwMin`, the configured first id.
        first: u32,
        /// `dwMax`, the configured last id.
        last: u32,
        /// `dwUsableItemIDMin`, the id the allocator would have started at.
        next: u32,
        /// How many rows the count query found.
        count: i64,
    },
    /// A row with this id already exists.
    ///
    /// Only [`insert_item`] reports it. `save_item` treats it as an update, which is
    /// right for a background write and wrong for a grant.
    ItemIdAlreadyStored {
        /// The id that is already taken.
        id: u32,
    },
    /// The cell this row names is already taken by another row.
    ///
    /// The second unique index, `(owner_id, window_type, pos)`, caught this. A grant that
    /// lands on a cell somebody already holds is a different defect from a reissued id,
    /// and it needs a different answer: the cell is the problem, and the id is fine.
    CellAlreadyTaken {
        /// The id the refused row wanted.
        id: u32,
        /// The window byte it wanted.
        window_type: u8,
        /// The cell it wanted.
        pos: u32,
    },
    /// A Transfer would leave a character with less than no gold.
    GoldBelowZero {
        /// The character.
        owner_id: u32,
        /// The gold its row holds.
        held: i64,
        /// The change the Transfer asked for.
        change: i64,
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
            Self::NotOwned {
                id,
                owner_id,
                account_id,
            } => match (owner_id, account_id) {
                (Some(owner), _) => write!(
                    f,
                    "item {id} is owned by character {owner}, not by this one"
                ),
                (None, Some(account)) => {
                    write!(f, "item {id} is held by account {account}, not by this one")
                }
                (None, None) => write!(f, "item {id} is on the ground"),
            },
            Self::ForeignAccount { owner_id, account } => write!(
                f,
                "character {owner_id} does not belong to account {account}"
            ),
            Self::ItemIdRangeExhausted { first, last, next } => write!(
                f,
                "item id range {first}..={last} would start at {next}, which leaves fewer than \
                 {MINIMUM_REMAIN_COUNT} ids"
            ),
            Self::ItemIdRangeOccupied {
                first,
                last,
                next,
                count,
            } => write!(
                f,
                "item id range {first}..={last} has {count} item rows from {next} to {last}"
            ),
            Self::ItemIdAlreadyStored { id } => {
                write!(f, "an item with id {id} is already stored")
            }
            Self::CellAlreadyTaken {
                id,
                window_type,
                pos,
            } => write!(
                f,
                "cell {window_type}:{pos} is already taken, so item {id} was not stored"
            ),
            Self::GoldBelowZero {
                owner_id,
                held,
                change,
            } => write!(
                f,
                "character {owner_id} holds {held} gold, so a change of {change} would leave \
                 less than none"
            ),
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

/// Legacy `CItemIDRangeManager::BuildRange` (`server/server/db/ItemIDRangeManager.cpp:87`),
/// with the configured span standing in for the two arguments the DB server passed it.
///
/// Legacy built 427 ten-million-wide blocks in
/// [`ItemIdRangePool::new`](crate::item_id_range::ItemIdRangePool::new) for the game
/// servers, skipped every block the configured span covered, and had each game server
/// check the range against its live peers before taking it. None of that exists here:
/// ADR-0002 puts every Channel in one process and gives the world one allocator, so
/// there is one span, one `MAX(id)`, and nobody to collide with. The two checks below
/// are the ones that changed whether the range was accepted at all.
///
/// # Errors
///
/// Returns [`ItemError::ItemIdRangeExhausted`] when the span would start within
/// [`MINIMUM_REMAIN_COUNT`] of its end, and [`ItemError::ItemIdRangeOccupied`] when a
/// row sits above the id the allocator would start at.
pub async fn resolve_item_id_range(
    store: &Store,
    span: ItemIdSpan,
) -> Result<crate::item_id_range::ItemIdRange, ItemError> {
    if !span.is_ordered() {
        // `ServerConfig::validate` already refuses this, and reaching it here would mean
        // a caller that skipped validation. The first id is used as the start either
        // way, so a reversed span is reported as exhausted rather than trusted.
        return Err(ItemError::ItemIdRangeExhausted {
            first: span.first,
            last: span.last,
            next: span.first,
        });
    }
    let highest: Option<i64> =
        sqlx::query_scalar("SELECT MAX(id) FROM item WHERE id >= $1 AND id <= $2")
            .bind(i64::from(span.first))
            .bind(i64::from(span.last))
            .fetch_one(store.pool())
            .await?;
    // `BuildRange` treats a NULL `MAX(id)` and a real 0 alike, and so does this: an
    // empty span is handed out from its first id.
    let next = match highest
        .map(|value| narrow::<i64, u32>(value, "MAX(id)"))
        .transpose()?
    {
        None | Some(0) => span.first,
        Some(max) => max.checked_add(1).ok_or(ItemError::ItemIdRangeExhausted {
            first: span.first,
            last: span.last,
            next: span.last,
        })?,
    };
    if span.remaining_from(next) < MINIMUM_REMAIN_COUNT {
        return Err(ItemError::ItemIdRangeExhausted {
            first: span.first,
            last: span.last,
            next,
        });
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM item WHERE id >= $1 AND id <= $2")
        .bind(i64::from(next))
        .bind(i64::from(span.last))
        .fetch_one(store.pool())
        .await?;
    if count > 0 {
        return Err(ItemError::ItemIdRangeOccupied {
            first: span.first,
            last: span.last,
            next,
            count,
        });
    }
    Ok(crate::item_id_range::ItemIdRange {
        min: span.first,
        max: span.last,
        usable_item_id_min: next,
    })
}

/// The plain columns, in the order the load reads them.
const PLAIN_COLUMNS: &str = "id, owner_id, account_id, window_type, pos, vnum, count, \
                            refine_element, transmutation, flags, anti_flags";

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

/// The one `SELECT` every load here runs: the thirty-one columns, named in the order
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
        account_id: row
            .try_get::<Option<i32>, _>("account_id")?
            .map(|id| narrow(id, "account_id"))
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
/// Legacy loads the same set (`db/ClientManagerPlayer.cpp:386`, the seven windows its
/// query names: `INVENTORY`, `EQUIPMENT`, `DRAGON_SOUL_INVENTORY`, `ATTR67_ADD`,
/// `SWITCHBOT`, `AURA_REFINE` and `BELT_INVENTORY`). It filters in the query (`row[0] = 0`
/// and the switch) and **silently drops** a row it does not recognise, so a character that
/// owns an item in a window this build does not have loses it at the next login. This
/// function does not repeat that: the only filter is the owner, and a row this build cannot
/// decode is an [`ItemError::Corrupt`] rather than a missing item.
///
/// Ground items are not returned, and that is not a filter: the holder rule
/// (`item_holder_check`) makes a row with an owner never a ground row, and never a `SAFEBOX` or
/// `MALL` row either; [`load_account_items`] loads those. The ground load is `sys.item.ground`, a
/// later unit.
///
/// # Errors
///
/// Returns [`ItemError::Corrupt`] for a row that cannot be decoded, or
/// [`ItemError::Database`].
pub async fn load_owner_items(store: &Store, owner_id: u32) -> Result<Vec<ItemRow>, ItemError> {
    // `owner_id = $1` is the only filter, and that is deliberate: the holder rule makes a
    // row with an owner never a ground row, so adding
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

/// Load one window of an account: its safebox (`SAFEBOX`) or its item mall (`MALL`).
///
/// Legacy reads the same rows with `owner_id = account AND window = SAFEBOX|MALL`
/// (`db/ClientManager.cpp:790-814`), where `owner_id` means an account for these two windows;
/// here the account has its own column (ADR-0005). The order is cell, then id.
///
/// # Errors
///
/// Returns [`ItemError::Corrupt`] for a row that cannot be decoded, or
/// [`ItemError::Database`].
pub async fn load_account_items(
    store: &Store,
    account_id: u32,
    window_type: u8,
) -> Result<Vec<ItemRow>, ItemError> {
    let sql = format!(
        "{} WHERE account_id = $1 AND window_type = $2 ORDER BY pos, id",
        select_sql()
    );
    let rows = sqlx::query(&sql)
        .bind(i64::from(account_id))
        .bind(i16::from(window_type))
        .fetch_all(store.pool())
        .await?;
    rows.iter().map(row_from).collect()
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
/// [`ItemError::CountOutOfRange`], [`ItemError::Corrupt`] for a row whose holders do not fit
/// its window, or [`ItemError::Database`] -- which is what an occupied cell is, since
/// the cell index is the database's rule and not a Rust one.
pub async fn save_item(store: &Store, item: &ItemRow) -> Result<(), ItemError> {
    let count = check(item)?;
    let sql = format!(
        "INSERT INTO item ({}) VALUES ({}) ON CONFLICT (id) DO UPDATE SET \
         owner_id = EXCLUDED.owner_id, account_id = EXCLUDED.account_id, \
         window_type = EXCLUDED.window_type, \
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
        placeholders(COLUMN_COUNT, 1)
    );
    bind_item(sqlx::query(&sql), item, count)
        .execute(store.pool())
        .await?;
    Ok(())
}

/// Insert one new item row, and refuse if its id is already in the table.
///
/// # Errors
///
/// Returns [`ItemError::ItemIdAlreadyStored`] when a row with this `id` exists,
/// [`ItemError::Corrupt`] when the row does not satisfy the Rust invariants, or
/// [`ItemError::Database`].
///
/// This exists beside [`save_item`] because the two disagree on purpose, and the
/// disagreement is the whole point of this function. `id` is the table's primary key
/// across **all** owners, so `save_item`'s `ON CONFLICT (id) DO UPDATE` on a row whose id
/// another character already holds would reassign that item: a new `owner_id`, a new cell,
/// a new vnum, over the top of a row that belongs to someone else. For a background
/// inventory write that is the right behaviour, because the caller is writing the state it
/// believes is current. For a grant it is a way to lose another player's item without any
/// error, so this has no conflict clause at all.
///
/// `CHECK ((window_type = 10) = (owner_id IS NULL))` and the signed socket and attribute
/// bounds still apply, so a row this function refuses is refused for the same reasons
/// [`save_item`] refuses one.
pub async fn insert_item(store: &Store, item: &ItemRow) -> Result<(), ItemError> {
    let count = check(item)?;
    let sql = format!(
        "INSERT INTO item ({}) VALUES ({})",
        all_columns(),
        placeholders(COLUMN_COUNT, 1)
    );
    bind_item(sqlx::query(&sql), item, count)
        .execute(store.pool())
        .await
        .map_err(|error| classify_insert(error, item))?;
    Ok(())
}

/// Turn a unique violation into the refusal that names it.
///
/// There are two unique indexes on this table and they mean different things, so this
/// checks the constraint's *name* and not only the SQLSTATE. The first draft matched
/// `23505` on its own and reported every violation as `ItemIdAlreadyStored`, so a grant
/// into a cell that was already taken was logged as "someone else has this id" -- an
/// Operator would then go looking for the wrong collision. `23505` is the code both
/// violations carry; the name is what separates them.
///
/// Anything else keeps its `ItemError::Database` shape, so a transport failure is never
/// dressed up as a gameplay refusal.
fn classify_insert(error: sqlx::Error, item: &ItemRow) -> ItemError {
    if let sqlx::Error::Database(database) = &error {
        if database.code().as_deref() == Some("23505") && database.constraint() == Some("item_pkey")
        {
            return ItemError::ItemIdAlreadyStored { id: item.id };
        }
        if database.code().as_deref() == Some("23505") {
            // The other unique indexes are the cell keys, `(owner_id, window_type, pos)`
            // and `(account_id, window_type, pos)`, so this is a cell that is already taken. It carries both facts an Operator needs and
            // neither one is guessable from the id alone.
            return ItemError::CellAlreadyTaken {
                id: item.id,
                window_type: item.window_type,
                pos: item.pos,
            };
        }
    }
    ItemError::Database(error)
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
         owner_id = EXCLUDED.owner_id, account_id = EXCLUDED.account_id, \
         window_type = EXCLUDED.window_type, \
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
        placeholders(COLUMN_COUNT, 1)
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
            account_id: item.account_id,
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
            account_id: item.account_id,
        }),
    }
}

/// One change a move, a stack merge, a split or a trade makes to a character's rows.
///
/// The world answers a `CG_ITEM_MOVE` with these, in the order they happened, and
/// [`apply_row_changes`] writes them as one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowChange {
    /// The item now sits in this window and cell, in stored terms.
    Moved {
        /// The item.
        id: u32,
        /// The window byte the row is stored under.
        window_type: u8,
        /// The cell the row is stored under.
        pos: u32,
    },
    /// The item's stack size changed.
    Count {
        /// The item.
        id: u32,
        /// Its new count.
        count: u16,
    },
    /// A split made this item.
    Created(ItemRow),
    /// A stack merge used this item up.
    Destroyed {
        /// The item.
        id: u32,
    },
    /// The item's sockets changed, which is what a sash's first equip does to its absorption
    /// share (`CItem::AddToCharacter`, `item.cpp:436-455`).
    Sockets {
        /// The item.
        id: u32,
        /// All of its sockets, as they are now.
        sockets: [i32; SOCKETS],
    },
    /// A trade gave the item to another character, into this window and cell, in stored
    /// terms (`CExchange::Done`, `G/exchange.cpp`). Only [`apply_exchange`] writes one, and
    /// only to another side of the same Transfer.
    Given {
        /// The item.
        id: u32,
        /// The character that holds it now.
        to: u32,
        /// The window byte the row is stored under.
        window_type: u8,
        /// The cell the row is stored under.
        pos: u32,
    },
    /// A checkin put the character's item into its account's safebox, at this cell
    /// (`CInputMain::SafeboxCheckin`, `input_main.cpp`; ADR-0005).
    Stored {
        /// The item.
        id: u32,
        /// The account the character belongs to.
        account: u32,
        /// The safebox cell.
        pos: u32,
    },
    /// A checkout took an item of the character's account out of its safebox or its mall,
    /// into this window and cell of the character, in stored terms
    /// (`CInputMain::SafeboxCheckout`, `input_main.cpp`; ADR-0005).
    Retrieved {
        /// The item.
        id: u32,
        /// The account the character belongs to.
        account: u32,
        /// The window byte the row is stored under.
        window_type: u8,
        /// The cell the row is stored under.
        pos: u32,
    },
}

/// One change a move inside an account's safebox or mall makes to the account's rows
/// (`CSafebox::MoveItem`, `safebox.cpp`), for [`apply_account_changes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountChange {
    /// The item now sits in this window and cell.
    Moved {
        /// The item.
        id: u32,
        /// `SAFEBOX` or `MALL`.
        window_type: u8,
        /// The cell.
        pos: u32,
    },
    /// The item's stack size changed.
    Count {
        /// The item.
        id: u32,
        /// Its new count.
        count: u16,
    },
    /// A stack merge used this item up.
    Destroyed {
        /// The item.
        id: u32,
    },
}

/// Whose rows a statement names: a character's, or an account's safebox and mall.
#[derive(Debug, Clone, Copy)]
enum Holder {
    /// `owner_id`.
    Character(u32),
    /// `account_id` (ADR-0005).
    Account(u32),
}

impl Holder {
    /// The holder's id, which is bound as `$2`.
    const fn id(self) -> u32 {
        match self {
            Self::Character(id) | Self::Account(id) => id,
        }
    }

    /// The column that names the holder. It is part of the statement's text and comes from
    /// this fixed pair, never from a value.
    const fn column(self) -> &'static str {
        match self {
            Self::Character(_) => "owner_id",
            Self::Account(_) => "account_id",
        }
    }
}

/// The statement that defers both cell keys to a transaction's commit.
const DEFER_CELL_KEYS: &str = "SET CONSTRAINTS item_owner_cell_key, item_account_cell_key DEFERRED";

/// One character's part of a Transfer: its row changes, and how far its gold moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferSide<'a> {
    /// The character whose rows the changes name.
    pub owner_id: u32,
    /// Its row changes, in the order they happened.
    pub changes: &'a [RowChange],
    /// What the Transfer adds to its gold, or takes away when negative.
    pub gold: i64,
}

/// Write one move's row changes as one transaction, under the owner.
///
/// Every value is checked before the store is reached, so a change this refuses was never
/// sent. Every statement names the owner, as [`destroy_item`] and [`set_count`] do, and a
/// statement that touches no row rolls the whole call back: a move whose rows disagree with
/// the character that made it has to be seen, not half-written.
///
/// # Errors
///
/// [`ItemError::CountOutOfRange`], [`ItemError::WindowOutOfRange`] or
/// [`ItemError::Corrupt`] for a change that cannot be stored, and the [`insert_item`] errors
/// for a created row. [`ItemError::NoSuchItem`] or [`ItemError::NotOwned`] when a change
/// names an id the owner does not hold, [`ItemError::CellAlreadyTaken`] when a moved or
/// created row ends on a cell another row holds, and [`ItemError::Database`].
///
/// The cell key is checked on the rows' **end** state, not after each statement. Equipping
/// onto a worn item is a swap (`char_item.cpp:8164-8264`), and its first move lands on a
/// cell the second move is about to vacate; migration `0006` made the key deferrable for
/// that, and this defers it for its own transaction only. The cells are then checked by a
/// query before the commit, because a violation raised by the commit itself names no row,
/// and the refusal has to name the change that caused it.
pub async fn apply_row_changes(
    store: &Store,
    owner_id: u32,
    changes: &[RowChange],
) -> Result<(), ItemError> {
    let side = TransferSide {
        owner_id,
        changes,
        gold: 0,
    };
    apply_sides(store, &[side], false).await
}

/// Write one Transfer: a step's row changes and the owner's gold, as one transaction
/// (ADR-0003).
///
/// A shop's buy or sale changes a character's items and its gold together, and one stored
/// without the other is an item lost or gold made. The rows are written as
/// [`apply_row_changes`] writes them, under the same checks, and `gold` is added to
/// `player.gold` in the same transaction. The change is relative because only a Transfer
/// writes the gold: the save leaves the column alone, so the row always holds what the last
/// Transfer left.
///
/// # Errors
///
/// As [`apply_row_changes`]; [`ItemError::GoldBelowZero`] for a change that would leave the
/// character less than no gold, and [`ItemError::NoSuchOwner`] when no character has the id.
/// Either rolls the rows back too.
pub async fn apply_transfer(
    store: &Store,
    owner_id: u32,
    changes: &[RowChange],
    gold: i64,
) -> Result<(), ItemError> {
    let side = TransferSide {
        owner_id,
        changes,
        gold,
    };
    apply_sides(store, &[side], true).await
}

/// Write a trade: every side's row changes and gold, as one transaction (ADR-0003).
///
/// `CExchange::Done` (`G/exchange.cpp`) moves each side's items to the other and the gold
/// with them, and a trade stored for one side and not the other is an item or gold made or
/// lost. Each side is written as [`apply_transfer`] writes it, and a
/// [`RowChange::Given`] hands a row to another side, whose cells are then checked as its own.
///
/// # Errors
///
/// As [`apply_transfer`], for any side; [`ItemError::Corrupt`] for two sides with one owner,
/// or a row given to a character that is not another side, both before the store is reached.
pub async fn apply_exchange(store: &Store, sides: &[TransferSide<'_>]) -> Result<(), ItemError> {
    apply_sides(store, sides, true).await
}

/// [`apply_exchange`], and without `gold` no `player` row is touched.
///
/// A [`RowChange::Stored`] or [`RowChange::Retrieved`] names the account its side's character
/// belongs to, which is read inside the transaction before any row is written: a change that
/// names another account is [`ItemError::ForeignAccount`], and nothing is written.
async fn apply_sides(
    store: &Store,
    sides: &[TransferSide<'_>],
    gold: bool,
) -> Result<(), ItemError> {
    let owners: Vec<u32> = sides.iter().map(|side| side.owner_id).collect();
    for (index, side) in sides.iter().enumerate() {
        if owners[..index].contains(&side.owner_id) {
            return Err(ItemError::Corrupt(format!(
                "character {} is two sides of one transfer",
                side.owner_id
            )));
        }
        for change in side.changes {
            check_change(side.owner_id, change, &owners)?;
        }
    }
    if !gold && sides.iter().all(|side| side.changes.is_empty()) {
        return Ok(());
    }
    let insert = format!(
        "INSERT INTO item ({}) VALUES ({})",
        all_columns(),
        placeholders(COLUMN_COUNT, 1)
    );
    let set_sockets = set_sockets_statement();
    let mut transaction = store.pool().begin().await?;
    sqlx::query(DEFER_CELL_KEYS)
        .execute(&mut *transaction)
        .await?;
    for side in sides {
        if let Err(error) = check_accounts(&mut transaction, side).await {
            transaction.rollback().await?;
            return Err(error);
        }
    }
    for side in sides {
        for change in side.changes {
            let (id, touched) = write_change(
                &mut transaction,
                side.owner_id,
                change,
                &insert,
                &set_sockets,
            )
            .await?;
            if touched == 0 {
                transaction.rollback().await?;
                return Err(missing(store, id).await);
            }
        }
    }
    if gold {
        for side in sides {
            if let Err(error) = add_gold(&mut transaction, side.owner_id, side.gold).await {
                transaction.rollback().await?;
                return Err(error);
            }
        }
    }
    for &owner_id in &owners {
        let shared: Vec<(i16, i64)> = sqlx::query_as(
            "SELECT window_type, pos::bigint FROM item WHERE owner_id = $1 \
             GROUP BY window_type, pos HAVING count(*) > 1",
        )
        .bind(i64::from(owner_id))
        .fetch_all(&mut *transaction)
        .await?;
        if !shared.is_empty() {
            transaction.rollback().await?;
            return Err(taken_cell(owner_id, sides, &shared));
        }
    }
    let stored = sides
        .iter()
        .flat_map(|side| side.changes)
        .filter_map(|change| match change {
            RowChange::Stored { id, account, pos } => Some((*account, *id, SAFEBOX, *pos)),
            _ => None,
        });
    let stored: Vec<(u32, u32, u8, u32)> = stored.collect();
    let mut accounts: Vec<u32> = stored.iter().map(|(account, ..)| *account).collect();
    accounts.sort_unstable();
    accounts.dedup();
    for account in accounts {
        let shared = shared_account_cells(&mut transaction, account).await?;
        if !shared.is_empty() {
            transaction.rollback().await?;
            return Err(taken_account_cell(account, &stored, &shared));
        }
    }
    transaction.commit().await?;
    Ok(())
}

/// Refuse a side whose checkin or checkout names an account its character does not belong to.
async fn check_accounts(
    connection: &mut sqlx::PgConnection,
    side: &TransferSide<'_>,
) -> Result<(), ItemError> {
    let named = side.changes.iter().filter_map(|change| match change {
        RowChange::Stored { account, .. } | RowChange::Retrieved { account, .. } => Some(*account),
        _ => None,
    });
    let named: Vec<u32> = named.collect();
    if named.is_empty() {
        return Ok(());
    }
    let player = i32::try_from(side.owner_id).map_err(|_| ItemError::NoSuchOwner(side.owner_id))?;
    let held: Option<i32> = sqlx::query_scalar("SELECT account_id FROM player WHERE id = $1")
        .bind(player)
        .fetch_optional(&mut *connection)
        .await?;
    let held = held.ok_or(ItemError::NoSuchOwner(side.owner_id))?;
    let held: u32 = narrow(held, "player.account_id")?;
    match named.into_iter().find(|account| *account != held) {
        Some(account) => Err(ItemError::ForeignAccount {
            owner_id: side.owner_id,
            account,
        }),
        None => Ok(()),
    }
}

/// The cells of one account that hold more than one row, inside a transaction whose cell keys
/// are deferred.
async fn shared_account_cells(
    connection: &mut sqlx::PgConnection,
    account: u32,
) -> Result<Vec<(i16, i64)>, ItemError> {
    Ok(sqlx::query_as(
        "SELECT window_type, pos::bigint FROM item WHERE account_id = $1 \
         GROUP BY window_type, pos HAVING count(*) > 1",
    )
    .bind(i64::from(account))
    .fetch_all(&mut *connection)
    .await?)
}

/// The change that put a row on a cell another row of `account` holds: the last one to land
/// there, as [`taken_cell`] names for a character. `landed` is every `(account, id, window,
/// cell)` a change put a row on, in order.
fn taken_account_cell(
    account: u32,
    landed: &[(u32, u32, u8, u32)],
    shared: &[(i16, i64)],
) -> ItemError {
    let last = landed.iter().rev().find(|(holder, _, window_type, pos)| {
        *holder == account && shared.contains(&(i16::from(*window_type), i64::from(*pos)))
    });
    match last {
        Some((_, id, window_type, pos)) => ItemError::CellAlreadyTaken {
            id: *id,
            window_type: *window_type,
            pos: *pos,
        },
        None => ItemError::Corrupt(format!(
            "account {account} holds two rows in one cell that no change touched: {shared:?}"
        )),
    }
}

/// Write one move inside an account's safebox or mall as one transaction, under the account
/// (ADR-0005).
///
/// `CSafebox::MoveItem` (`safebox.cpp`) moves a row to an empty cell or merges it into a
/// stack, and the rows it touches belong to the account, not to the character that moved
/// them. Each change is checked before the store is reached, each statement names the
/// account, and a statement that touches no row rolls the call back, as
/// [`apply_row_changes`] does for a character.
///
/// # Errors
///
/// [`ItemError::WindowOutOfRange`] or [`ItemError::Corrupt`] for a move out of the account's
/// windows and [`ItemError::CountOutOfRange`] for a count that cannot be stored, all before
/// the store is reached. [`ItemError::NoSuchItem`] or [`ItemError::NotOwned`] when a change
/// names an id the account does not hold, [`ItemError::CellAlreadyTaken`] when a moved row
/// ends on a cell another row holds, and [`ItemError::Database`].
pub async fn apply_account_changes(
    store: &Store,
    account_id: u32,
    changes: &[AccountChange],
) -> Result<(), ItemError> {
    for change in changes {
        match change {
            AccountChange::Moved {
                id, window_type, ..
            } => {
                if *window_type > GROUND {
                    return Err(ItemError::WindowOutOfRange(*window_type));
                }
                if *window_type != SAFEBOX && *window_type != MALL {
                    return Err(ItemError::Corrupt(format!(
                        "item {id} of account {account_id} would be moved to window \
                         {window_type}, which an account does not hold"
                    )));
                }
            }
            AccountChange::Count { count, .. } => check_count(*count)?,
            AccountChange::Destroyed { .. } => {}
        }
    }
    if changes.is_empty() {
        return Ok(());
    }
    let holder = Holder::Account(account_id);
    let mut transaction = store.pool().begin().await?;
    sqlx::query(DEFER_CELL_KEYS)
        .execute(&mut *transaction)
        .await?;
    for change in changes {
        let (id, touched) = match *change {
            AccountChange::Moved {
                id,
                window_type,
                pos,
            } => (
                id,
                move_row(&mut transaction, holder, id, window_type, pos).await?,
            ),
            AccountChange::Count { id, count } => {
                (id, count_row(&mut transaction, holder, id, count).await?)
            }
            AccountChange::Destroyed { id } => {
                (id, destroy_row(&mut transaction, holder, id).await?)
            }
        };
        if touched == 0 {
            transaction.rollback().await?;
            return Err(missing(store, id).await);
        }
    }
    let shared = shared_account_cells(&mut transaction, account_id).await?;
    if !shared.is_empty() {
        transaction.rollback().await?;
        let landed: Vec<(u32, u32, u8, u32)> = changes
            .iter()
            .filter_map(|change| match *change {
                AccountChange::Moved {
                    id,
                    window_type,
                    pos,
                } => Some((account_id, id, window_type, pos)),
                AccountChange::Count { .. } | AccountChange::Destroyed { .. } => None,
            })
            .collect();
        return Err(taken_account_cell(account_id, &landed, &shared));
    }
    transaction.commit().await?;
    Ok(())
}

/// Move one row of `holder` to a window and cell: how many rows it touched.
async fn move_row(
    connection: &mut sqlx::PgConnection,
    holder: Holder,
    id: u32,
    window_type: u8,
    pos: u32,
) -> Result<u64, ItemError> {
    let sql = format!(
        "UPDATE item SET window_type = $3, pos = $4 WHERE id = $1 AND {} = $2",
        holder.column()
    );
    Ok(sqlx::query(&sql)
        .bind(i64::from(id))
        .bind(i64::from(holder.id()))
        .bind(i16::from(window_type))
        .bind(i64::from(pos))
        .execute(&mut *connection)
        .await?
        .rows_affected())
}

/// Set the count of one row of `holder`: how many rows it touched.
async fn count_row(
    connection: &mut sqlx::PgConnection,
    holder: Holder,
    id: u32,
    count: u16,
) -> Result<u64, ItemError> {
    // A `smallint` bind for a `smallint` column, as `set_count` explains.
    let column = i16::try_from(count).map_err(|_| ItemError::CountOutOfRange(count))?;
    let sql = format!(
        "UPDATE item SET count = $3 WHERE id = $1 AND {} = $2",
        holder.column()
    );
    Ok(sqlx::query(&sql)
        .bind(i64::from(id))
        .bind(i64::from(holder.id()))
        .bind(column)
        .execute(&mut *connection)
        .await?
        .rows_affected())
}

/// Delete one row of `holder`: how many rows it touched.
async fn destroy_row(
    connection: &mut sqlx::PgConnection,
    holder: Holder,
    id: u32,
) -> Result<u64, ItemError> {
    let sql = format!(
        "DELETE FROM item WHERE id = $1 AND {} = $2",
        holder.column()
    );
    Ok(sqlx::query(&sql)
        .bind(i64::from(id))
        .bind(i64::from(holder.id()))
        .execute(&mut *connection)
        .await?
        .rows_affected())
}

/// Add `change` to one character's `player.gold` inside a Transfer's transaction.
///
/// The statement refuses a result below zero itself, so the check and the write cannot be
/// split by another writer; the row is read again only to say why nothing changed.
async fn add_gold(
    connection: &mut sqlx::PgConnection,
    owner_id: u32,
    change: i64,
) -> Result<(), ItemError> {
    let player = i32::try_from(owner_id).map_err(|_| ItemError::NoSuchOwner(owner_id))?;
    let touched =
        sqlx::query("UPDATE player SET gold = gold + $2 WHERE id = $1 AND gold + $2 >= 0")
            .bind(player)
            .bind(change)
            .execute(&mut *connection)
            .await?
            .rows_affected();
    if touched > 0 {
        return Ok(());
    }
    let held: Option<i64> = sqlx::query_scalar("SELECT gold FROM player WHERE id = $1")
        .bind(player)
        .fetch_optional(&mut *connection)
        .await?;
    Err(match held {
        None => ItemError::NoSuchOwner(owner_id),
        Some(held) => ItemError::GoldBelowZero {
            owner_id,
            held,
            change,
        },
    })
}

/// Run one change's statement inside the move's transaction: the id it names, and how many
/// rows it touched.
async fn write_change(
    connection: &mut sqlx::PgConnection,
    owner_id: u32,
    change: &RowChange,
    insert: &str,
    set_sockets: &str,
) -> Result<(u32, u64), ItemError> {
    let holder = Holder::Character(owner_id);
    Ok(match change {
        RowChange::Moved {
            id,
            window_type,
            pos,
        } => (
            *id,
            move_row(connection, holder, *id, *window_type, *pos).await?,
        ),
        RowChange::Count { id, count } => (*id, count_row(connection, holder, *id, *count).await?),
        RowChange::Destroyed { id } => (*id, destroy_row(connection, holder, *id).await?),
        RowChange::Created(row) => {
            let count = check(row)?;
            bind_item(sqlx::query(insert), row, count)
                .execute(&mut *connection)
                .await
                .map_err(|error| classify_insert(error, row))?;
            (row.id, 1)
        }
        RowChange::Sockets { id, sockets } => {
            let mut query = sqlx::query(set_sockets)
                .bind(i64::from(*id))
                .bind(i64::from(owner_id));
            for socket in sockets {
                query = query.bind(*socket);
            }
            (*id, query.execute(&mut *connection).await?.rows_affected())
        }
        RowChange::Given {
            id,
            to,
            window_type,
            pos,
        } => {
            let receiver = i32::try_from(*to).map_err(|_| ItemError::NoSuchOwner(*to))?;
            let touched = sqlx::query(
                "UPDATE item SET owner_id = $3, window_type = $4, pos = $5 \
                 WHERE id = $1 AND owner_id = $2",
            )
            .bind(i64::from(*id))
            .bind(i64::from(owner_id))
            .bind(receiver)
            .bind(i16::from(*window_type))
            .bind(i64::from(*pos))
            .execute(&mut *connection)
            .await?
            .rows_affected();
            (*id, touched)
        }
        RowChange::Stored { id, account, pos } => {
            let touched = sqlx::query(
                "UPDATE item SET owner_id = NULL, account_id = $3, window_type = $4, pos = $5 \
                 WHERE id = $1 AND owner_id = $2",
            )
            .bind(i64::from(*id))
            .bind(i64::from(owner_id))
            .bind(i64::from(*account))
            .bind(i16::from(SAFEBOX))
            .bind(i64::from(*pos))
            .execute(&mut *connection)
            .await?
            .rows_affected();
            (*id, touched)
        }
        RowChange::Retrieved {
            id,
            account,
            window_type,
            pos,
        } => {
            let touched = sqlx::query(
                "UPDATE item SET account_id = NULL, owner_id = $2, window_type = $4, pos = $5 \
                 WHERE id = $1 AND account_id = $3",
            )
            .bind(i64::from(*id))
            .bind(i64::from(owner_id))
            .bind(i64::from(*account))
            .bind(i16::from(*window_type))
            .bind(i64::from(*pos))
            .execute(&mut *connection)
            .await?
            .rows_affected();
            (*id, touched)
        }
    })
}

/// The statement a [`RowChange::Sockets`] runs: every socket column of one owned row, bound
/// from `$3` on.
fn set_sockets_statement() -> String {
    format!(
        "UPDATE item SET {} WHERE id = $1 AND owner_id = $2",
        SOCKET_COLUMNS
            .iter()
            .zip(3..)
            .map(|(name, index)| format!("{name} = ${index}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The change that put a row on a cell another row of `owner_id` holds, for
/// [`apply_row_changes`] and [`apply_exchange`].
///
/// The last change to land on a shared cell is the one named: an earlier change that
/// landed there may have been legal until the later one arrived, and the later one is
/// the change a caller can do something about. A row given to `owner_id` lands among its
/// cells as its own moves do. A shared cell no change touched cannot happen while the key
/// is enforced, so it is reported as corrupt rather than guessed at.
fn taken_cell(owner_id: u32, sides: &[TransferSide<'_>], shared: &[(i16, i64)]) -> ItemError {
    let changes = sides.iter().flat_map(|side| {
        side.changes
            .iter()
            .map(move |change| (side.owner_id, change))
    });
    let landed: Vec<(u32, u8, u32)> = changes
        .filter_map(|(side, change)| {
            let (holder, id, window_type, pos) = match change {
                RowChange::Moved {
                    id,
                    window_type,
                    pos,
                }
                | RowChange::Retrieved {
                    id,
                    window_type,
                    pos,
                    ..
                } => (side, *id, *window_type, *pos),
                RowChange::Created(row) => (side, row.id, row.window_type, row.pos),
                RowChange::Given {
                    id,
                    to,
                    window_type,
                    pos,
                } => (*to, *id, *window_type, *pos),
                RowChange::Count { .. }
                | RowChange::Destroyed { .. }
                | RowChange::Sockets { .. }
                | RowChange::Stored { .. } => return None,
            };
            (holder == owner_id).then_some((id, window_type, pos))
        })
        .collect();
    let landed = landed
        .into_iter()
        .rev()
        .find(|(_, window_type, pos)| shared.contains(&(i16::from(*window_type), i64::from(*pos))));
    match landed {
        Some((id, window_type, pos)) => ItemError::CellAlreadyTaken {
            id,
            window_type,
            pos,
        },
        None => ItemError::Corrupt(format!(
            "character {owner_id} holds two rows in one cell that no change touched: {shared:?}"
        )),
    }
}

/// The checks [`apply_row_changes`] makes before it reaches the store. A row may only be
/// given to one of `owners`, the characters the Transfer writes, and not to its own holder.
fn check_change(owner_id: u32, change: &RowChange, owners: &[u32]) -> Result<(), ItemError> {
    match change {
        RowChange::Moved {
            id, window_type, ..
        } => check_window(*id, *window_type)?,
        RowChange::Given {
            id,
            to,
            window_type,
            ..
        } => {
            check_window(*id, *window_type)?;
            if *to == owner_id || !owners.contains(to) {
                return Err(ItemError::Corrupt(format!(
                    "item {id} of character {owner_id} would be given to {to}, which is not \
                     another side of the transfer"
                )));
            }
        }
        RowChange::Retrieved {
            id, window_type, ..
        } => check_window(*id, *window_type)?,
        RowChange::Count { count, .. } => check_count(*count)?,
        RowChange::Created(row) => {
            let _count = check(row)?;
            if row.owner_id != Some(owner_id) {
                return Err(ItemError::Corrupt(format!(
                    "item {} was created for {:?} by character {owner_id}",
                    row.id, row.owner_id
                )));
            }
        }
        RowChange::Destroyed { .. } | RowChange::Sockets { .. } | RowChange::Stored { .. } => {}
    }
    Ok(())
}

/// A stack size a row can hold: 1 to [`ITEM_MAX_COUNT`].
fn check_count(count: u16) -> Result<(), ItemError> {
    if count == 0 || u32::from(count) > ITEM_MAX_COUNT {
        return Err(ItemError::CountOutOfRange(count));
    }
    Ok(())
}

/// A moved, given or retrieved row stays in a window a character holds: not past the ground,
/// not on it, and not in an account's safebox or mall, which only a checkin reaches.
fn check_window(id: u32, window_type: u8) -> Result<(), ItemError> {
    if window_type > GROUND {
        return Err(ItemError::WindowOutOfRange(window_type));
    }
    if window_type == GROUND {
        return Err(ItemError::Corrupt(format!(
            "item {id} would be moved to the ground window through its owner"
        )));
    }
    if window_type == SAFEBOX || window_type == MALL {
        return Err(ItemError::Corrupt(format!(
            "item {id} would be moved to the account window {window_type} through its owner"
        )));
    }
    Ok(())
}

/// Why a change touched no row: the id is not stored, or somebody else holds it.
async fn missing(store: &Store, id: u32) -> ItemError {
    match load_item(store, id).await {
        Ok(None) => ItemError::NoSuchItem(id),
        Ok(Some(item)) => ItemError::NotOwned {
            id,
            owner_id: item.owner_id,
            account_id: item.account_id,
        },
        Err(error) => error,
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

/// Bind one item's thirty-one columns, in the order [`all_columns`] lists them.
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
        .bind(item.account_id.map(i64::from))
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
    // The holder rule (`item_holder_check`), checked before the statement so a caller
    // learns which holder it broke rather than reading a constraint name.
    if !holder_fits(item.owner_id, item.account_id, item.window_type) {
        return Err(ItemError::Corrupt(format!(
            "item {} is held by character {:?} and account {:?} but its window is {}",
            item.id, item.owner_id, item.account_id, item.window_type
        )));
    }
    Ok(count)
}

/// Whether a row's holders fit its window: a ground row has none, a `SAFEBOX` or `MALL` row
/// has only the account, and every other window has only the character (migration `0008`).
const fn holder_fits(owner_id: Option<u32>, account_id: Option<u32>, window_type: u8) -> bool {
    match (owner_id, account_id) {
        (None, None) => window_type == GROUND,
        (None, Some(_)) => window_type == SAFEBOX || window_type == MALL,
        (Some(_), None) => window_type != GROUND && window_type != SAFEBOX && window_type != MALL,
        (Some(_), Some(_)) => false,
    }
}
