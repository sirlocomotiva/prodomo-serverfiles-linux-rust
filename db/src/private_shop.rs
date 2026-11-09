//! A character's personal shop: legacy's `player.private_shop` and
//! `player.private_shop_item` (`legacy/sql/schema/player.sql:995-1055`).
//!
//! Legacy moves a ware between tables: the build deletes the item from the player's
//! cache and rows (`D/ClientManagerPrivateShop.cpp:1184-1192`) and caches it under
//! the shop. Here the build is one transaction that inserts the shop row, deletes
//! the wares from `item`, and inserts them into `private_shop_item` with their
//! prices and checkin times. A build for a character that already has a shop row
//! is refused, as legacy's `GetPrivateShop` check refuses it (`:1116-1130`).

use common::constants::ITEM_ATTRIBUTE_MAX_NUM;
use sqlx::postgres::PgRow;
use sqlx::Row;

use crate::items::{Attribute, ItemError, ItemRow, SOCKETS};
use crate::store::Store;

/// One stored personal shop: the stand, the earnings, and the pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopRow {
    /// The owning character.
    pub owner_id: u32,
    /// The raw title bytes.
    pub title: Vec<u8>,
    /// The owner's name as the entity record shows it.
    pub owner_name: Vec<u8>,
    /// The shop's state: 0 unavailable, 1 closed, 2 open, 3 modify.
    pub state: u8,
    /// The stand's polymorph vnum.
    pub vnum: u32,
    /// The title's type byte.
    pub title_type: u8,
    /// The stand's map and position.
    pub map_index: i32,
    /// The stand's X.
    pub x: i32,
    /// The stand's Y.
    pub y: i32,
    /// The Channel the stand is on.
    pub channel: u8,
    /// The earnings in yang.
    pub gold: i64,
    /// The earnings in cheques.
    pub cheque: u32,
    /// The page count, 1 or 2.
    pub page_count: u8,
    /// The premium expiry as a unix time.
    pub premium_time: i64,
    /// The unlocked slots.
    pub unlocked_slots: i32,
}

/// One stored ware: the instance's full data at its shop slot, with its price.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopItemRow {
    /// The item, as it stood in the owner's window.
    pub item: ItemRow,
    /// The shop slot, 0 to 127.
    pub pos: u16,
    /// The asking price in yang.
    pub gold: i64,
    /// The asking price in cheques.
    pub cheque: u32,
    /// The checkin time.
    pub checkin: i64,
}

/// Build a personal shop: insert the shop row, delete the wares from `item`, and
/// insert them into `private_shop_item`, all in one transaction.
///
/// # Errors
///
/// Returns [`ItemError::Corrupt`] when a ware's id is already a ware (a shop row
/// already names it), or [`ItemError::Database`] -- which is what an existing
/// shop row is, since the owner is the table's primary key.
pub async fn build_shop(
    store: &Store,
    shop: &ShopRow,
    wares: &[ShopItemRow],
) -> Result<(), ItemError> {
    let mut tx = store.pool().begin().await?;
    sqlx::query(
        "INSERT INTO private_shop (owner_id, title, owner_name, state, vnum, title_type, \
         map_index, x, y, channel, gold, cheque, page_count, premium_time, unlocked_slots) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
    )
    .bind(i32::try_from(shop.owner_id).map_err(|_| corrupt("owner_id", shop.owner_id))?)
    .bind(&shop.title)
    .bind(&shop.owner_name)
    .bind(i16::from(shop.state))
    .bind(i64::from(shop.vnum))
    .bind(i16::from(shop.title_type))
    .bind(shop.map_index)
    .bind(shop.x)
    .bind(shop.y)
    .bind(i16::from(shop.channel))
    .bind(shop.gold)
    .bind(i64::from(shop.cheque))
    .bind(i16::from(shop.page_count))
    .bind(shop.premium_time)
    .bind(shop.unlocked_slots)
    .execute(&mut *tx)
    .await?;
    for ware in wares {
        let deleted: u64 = sqlx::query("DELETE FROM item WHERE id = $1 AND owner_id = $2")
            .bind(i64::from(ware.item.id))
            .bind(i32::try_from(ware.item.owner_id.unwrap_or(0)).unwrap_or(0))
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if deleted == 0 {
            return Err(ItemError::NoSuchItem(ware.item.id));
        }
        sqlx::query(
            "INSERT INTO private_shop_item (id, owner_id, pos, count, vnum, gold, cheque, \
             checkin, socket0, socket1, socket2, socket3, socket4, socket5, attrtype0, \
             attrvalue0, attrtype1, attrvalue1, attrtype2, attrvalue2, attrtype3, \
             attrvalue3, attrtype4, attrvalue4, attrtype5, attrvalue5, attrtype6, \
             attrvalue6) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
             $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28)",
        )
        .bind(i64::from(ware.item.id))
        .bind(i32::try_from(shop.owner_id).map_err(|_| corrupt("owner_id", shop.owner_id))?)
        .bind(i16::try_from(ware.pos).map_err(|_| corrupt("pos", ware.pos))?)
        .bind(i16::try_from(ware.item.count).map_err(|_| corrupt("count", ware.item.count))?)
        .bind(i64::from(ware.item.vnum))
        .bind(ware.gold)
        .bind(i64::from(ware.cheque))
        .bind(ware.checkin)
        .bind(ware.item.sockets[0])
        .bind(ware.item.sockets[1])
        .bind(ware.item.sockets[2])
        .bind(ware.item.sockets[3])
        .bind(ware.item.sockets[4])
        .bind(ware.item.sockets[5])
        .bind(i16::from(ware.item.attributes[0].b_type))
        .bind(ware.item.attributes[0].s_value)
        .bind(i16::from(ware.item.attributes[1].b_type))
        .bind(ware.item.attributes[1].s_value)
        .bind(i16::from(ware.item.attributes[2].b_type))
        .bind(ware.item.attributes[2].s_value)
        .bind(i16::from(ware.item.attributes[3].b_type))
        .bind(ware.item.attributes[3].s_value)
        .bind(i16::from(ware.item.attributes[4].b_type))
        .bind(ware.item.attributes[4].s_value)
        .bind(i16::from(ware.item.attributes[5].b_type))
        .bind(ware.item.attributes[5].s_value)
        .bind(i16::from(ware.item.attributes[6].b_type))
        .bind(ware.item.attributes[6].s_value)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// One settled personal-shop buy: the ware, the buyer, and the stash's growth.
#[derive(Debug, Clone, Copy)]
pub struct BoughtWare {
    /// The shop owner's player id.
    pub owner_id: u32,
    /// The ware's item id, leaving `private_shop_item` for `item`.
    pub ware_id: u32,
    /// The buyer's player id.
    pub buyer_id: u32,
    /// The window the world placed the ware at.
    pub window_type: u8,
    /// The cell the world placed the ware at.
    pub pos: u16,
    /// The ware's count.
    pub count: u16,
    /// The price in yang, joining the shop's stash.
    pub price: i64,
}

/// Settle a personal-shop buy: the ware leaves `private_shop_item` for `item` under
/// the buyer at the window and cell the world placed it at, and the price joins the
/// shop's stash, all in one transaction.
///
/// # Errors
///
/// Returns [`ItemError::NoSuchItem`] when the ware is gone, or [`ItemError::Database`].
pub async fn buy_ware(store: &Store, bought: BoughtWare) -> Result<(), ItemError> {
    let BoughtWare {
        owner_id,
        ware_id,
        buyer_id,
        window_type,
        pos,
        count,
        price,
    } = bought;
    let mut tx = store.pool().begin().await?;
    let rows = sqlx::query(
        "SELECT id, owner_id, pos, count, vnum, gold, cheque, checkin, socket0, socket1,          socket2, socket3, socket4, socket5, attrtype0, attrvalue0, attrtype1, attrvalue1,          attrtype2, attrvalue2, attrtype3, attrvalue3, attrtype4, attrvalue4, attrtype5,          attrvalue5, attrtype6, attrvalue6 FROM private_shop_item WHERE id = $1 AND          owner_id = $2",
    )
    .bind(i64::from(ware_id))
    .bind(i32::try_from(owner_id).map_err(|_| corrupt("owner_id", owner_id))?)
    .fetch_all(&mut *tx)
    .await?;
    let [row] = rows.as_slice() else {
        return Err(ItemError::NoSuchItem(ware_id));
    };
    let ware = ware_from(row)?;
    sqlx::query("DELETE FROM private_shop_item WHERE id = $1")
        .bind(i64::from(ware_id))
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO item (id, owner_id, window_type, pos, vnum, count, socket0, socket1,          socket2, socket3, socket4, socket5, attrtype0, attrvalue0, attrtype1,          attrvalue1, attrtype2, attrvalue2, attrtype3, attrvalue3, attrtype4,          attrvalue4, attrtype5, attrvalue5, attrtype6, attrvalue6) VALUES ($1, $2, $3,          $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,          $21, $22, $23, $24, $25, $26)",
    )
    .bind(i64::from(ware_id))
    .bind(i32::try_from(buyer_id).map_err(|_| corrupt("buyer_id", buyer_id))?)
    .bind(i16::from(window_type))
    .bind(i32::from(pos))
    .bind(i64::from(ware.item.vnum))
    .bind(i16::try_from(count).map_err(|_| corrupt("count", count))?)
    .bind(ware.item.sockets[0])
    .bind(ware.item.sockets[1])
    .bind(ware.item.sockets[2])
    .bind(ware.item.sockets[3])
    .bind(ware.item.sockets[4])
    .bind(ware.item.sockets[5])
    .bind(i16::from(ware.item.attributes[0].b_type))
    .bind(ware.item.attributes[0].s_value)
    .bind(i16::from(ware.item.attributes[1].b_type))
    .bind(ware.item.attributes[1].s_value)
    .bind(i16::from(ware.item.attributes[2].b_type))
    .bind(ware.item.attributes[2].s_value)
    .bind(i16::from(ware.item.attributes[3].b_type))
    .bind(ware.item.attributes[3].s_value)
    .bind(i16::from(ware.item.attributes[4].b_type))
    .bind(ware.item.attributes[4].s_value)
    .bind(i16::from(ware.item.attributes[5].b_type))
    .bind(ware.item.attributes[5].s_value)
    .bind(i16::from(ware.item.attributes[6].b_type))
    .bind(ware.item.attributes[6].s_value)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE private_shop SET gold = gold + $1 WHERE owner_id = $2")
        .bind(price)
        .bind(i32::try_from(owner_id).map_err(|_| corrupt("owner_id", owner_id))?)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Load one character's shop row, or `None` when it has no shop.
///
/// # Errors
///
/// Returns [`ItemError::Corrupt`] for a row that cannot be decoded, or
/// [`ItemError::Database`].
pub async fn load_shop(store: &Store, owner_id: u32) -> Result<Option<ShopRow>, ItemError> {
    let row = sqlx::query(
        "SELECT owner_id, title, owner_name, state, vnum, title_type, map_index, x, y, \
         channel, gold, cheque, page_count, premium_time, unlocked_slots FROM private_shop \
         WHERE owner_id = $1",
    )
    .bind(i32::try_from(owner_id).map_err(|_| corrupt("owner_id", owner_id))?)
    .fetch_optional(store.pool())
    .await?;
    row.as_ref().map(shop_from).transpose()
}

/// Load one character's wares, in slot order.
///
/// # Errors
///
/// Returns [`ItemError::Corrupt`] for a row that cannot be decoded, or
/// [`ItemError::Database`].
pub async fn load_shop_items(store: &Store, owner_id: u32) -> Result<Vec<ShopItemRow>, ItemError> {
    let rows = sqlx::query(
        "SELECT id, owner_id, pos, count, vnum, gold, cheque, checkin, socket0, socket1, \
         socket2, socket3, socket4, socket5, attrtype0, attrvalue0, attrtype1, attrvalue1, \
         attrtype2, attrvalue2, attrtype3, attrvalue3, attrtype4, attrvalue4, attrtype5, \
         attrvalue5, attrtype6, attrvalue6 FROM private_shop_item WHERE owner_id = $1 ORDER \
         BY pos, id",
    )
    .bind(i32::try_from(owner_id).map_err(|_| corrupt("owner_id", owner_id))?)
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(ware_from).collect()
}

/// Decode one shop row, or report it as corrupt.
fn shop_from(row: &PgRow) -> Result<ShopRow, ItemError> {
    Ok(ShopRow {
        owner_id: narrow(row.try_get::<i32, _>("owner_id")?, "owner_id")?,
        title: row.try_get::<Vec<u8>, _>("title")?,
        owner_name: row.try_get::<Vec<u8>, _>("owner_name")?,
        state: u8::try_from(row.try_get::<i16, _>("state")?)
            .map_err(|_| corrupt("state", row.try_get::<i16, _>("state").unwrap_or(0)))?,
        vnum: narrow(row.try_get::<i64, _>("vnum")?, "vnum")?,
        title_type: u8::try_from(row.try_get::<i16, _>("title_type")?).map_err(|_| {
            corrupt(
                "title_type",
                row.try_get::<i16, _>("title_type").unwrap_or(0),
            )
        })?,
        map_index: row.try_get::<i32, _>("map_index")?,
        x: row.try_get::<i32, _>("x")?,
        y: row.try_get::<i32, _>("y")?,
        channel: u8::try_from(row.try_get::<i16, _>("channel")?)
            .map_err(|_| corrupt("channel", row.try_get::<i16, _>("channel").unwrap_or(0)))?,
        gold: row.try_get::<i64, _>("gold")?,
        cheque: narrow(row.try_get::<i64, _>("cheque")?, "cheque")?,
        page_count: u8::try_from(row.try_get::<i16, _>("page_count")?).map_err(|_| {
            corrupt(
                "page_count",
                row.try_get::<i16, _>("page_count").unwrap_or(0),
            )
        })?,
        premium_time: row.try_get::<i64, _>("premium_time")?,
        unlocked_slots: row.try_get::<i32, _>("unlocked_slots")?,
    })
}

/// Decode one ware row, or report it as corrupt.
fn ware_from(row: &PgRow) -> Result<ShopItemRow, ItemError> {
    let sockets = [
        row.try_get::<i32, _>("socket0")?,
        row.try_get::<i32, _>("socket1")?,
        row.try_get::<i32, _>("socket2")?,
        row.try_get::<i32, _>("socket3")?,
        row.try_get::<i32, _>("socket4")?,
        row.try_get::<i32, _>("socket5")?,
    ];
    debug_assert_eq!(sockets.len(), SOCKETS);
    let mut attributes = [Attribute {
        b_type: 0,
        s_value: 0,
    }; ITEM_ATTRIBUTE_MAX_NUM as usize];
    for (index, slot) in attributes.iter_mut().enumerate() {
        let b_type: i16 = row.try_get(format!("attrtype{index}").as_str())?;
        let s_value: i16 = row.try_get(format!("attrvalue{index}").as_str())?;
        *slot = Attribute {
            b_type: u8::try_from(b_type).map_err(|_| corrupt("attrtype", b_type))?,
            s_value,
        };
    }
    Ok(ShopItemRow {
        item: ItemRow {
            id: narrow(row.try_get::<i64, _>("id")?, "id")?,
            owner_id: Some(narrow(row.try_get::<i32, _>("owner_id")?, "owner_id")?),
            account_id: None,
            window_type: 1,
            pos: narrow(row.try_get::<i16, _>("pos")?, "pos")?,
            vnum: narrow(row.try_get::<i64, _>("vnum")?, "vnum")?,
            count: u16::try_from(row.try_get::<i16, _>("count")?)
                .map_err(|_| corrupt("count", row.try_get::<i16, _>("count").unwrap_or(0)))?,
            refine_element: 0,
            transmutation: 0,
            flags: 0,
            anti_flags: 0,
            sockets,
            attributes,
        },
        pos: u16::try_from(row.try_get::<i16, _>("pos")?)
            .map_err(|_| corrupt("pos", row.try_get::<i16, _>("pos").unwrap_or(0)))?,
        gold: row.try_get::<i64, _>("gold")?,
        cheque: narrow(row.try_get::<i64, _>("cheque")?, "cheque")?,
        checkin: row.try_get::<i64, _>("checkin")?,
    })
}

/// A corrupt-row error naming the column and the value.
fn corrupt(column: &str, value: impl std::fmt::Display) -> ItemError {
    ItemError::Corrupt(format!("private shop column {column} holds {value}"))
}

/// Narrow a value to the width the game is sent, or report the row as corrupt.
fn narrow<From, To>(value: From, column: &str) -> Result<To, ItemError>
where
    From: Copy + std::fmt::Display,
    To: TryFrom<From>,
{
    To::try_from(value).map_err(|_| corrupt(column, value))
}
