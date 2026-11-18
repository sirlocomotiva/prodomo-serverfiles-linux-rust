//! The silk bundle's price list: legacy's `myshop_pricelist` table
//! (`legacy/sql/schema/player.sql:744-749`, `0046_myshop_pricelist.sql`).
//!
//! A bundle shop opened while the seller holds the silk bundle 71049 sends the shop's unit
//! prices (`HEADER_GD_MYSHOP_PRICELIST_UPDATE`, `G/char.cpp:915-937`), and the botary's first
//! use of a session reads them back (`HEADER_GD_MYSHOP_PRICELIST_REQ`,
//! `G/char_item.cpp:2655-2664`). The DB server's cache answers both
//! (`CItemPriceListTableCache`, `server/server/db/Cache.cpp:226-307`); here the rows are the
//! cache, kept in its order by `slot`.

use crate::accounts::AccountError;
use crate::store::Store;

/// `SHOP_PRICELIST_MAX_NUM` (`C/length.h:47`): the most prices one owner keeps.
pub const SHOP_PRICELIST_MAX_NUM: usize = 40;

/// One price: the item vnum and its unit price in yang (`TItemPriceInfo`,
/// `C/tables.h:1521-1530`).
pub type StoredPrice = (u32, u64);

/// Load the prices `owner` keeps, in the cache's order (`MyshopPricelistRequest`,
/// `server/server/db/ClientManager.cpp:2273-2303`).
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be read.
pub async fn load(store: &Store, owner: u32) -> Result<Vec<StoredPrice>, AccountError> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT item_vnum, price FROM myshop_pricelist WHERE owner_id = $1 ORDER BY slot",
    )
    .bind(i64::from(owner))
    .fetch_all(store.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(|(vnum, price)| {
            (
                u32::try_from(vnum).unwrap_or(0),
                u64::try_from(price).unwrap_or(0),
            )
        })
        .collect())
}

/// `CItemPriceListTableCache::UpdateList` (`server/server/db/Cache.cpp:226-277`): the new
/// prices lead, then the kept ones whose vnum the new list does not name, in their order, up
/// to [`SHOP_PRICELIST_MAX_NUM`]. A list longer than the cap changes nothing, as the cache
/// refuses it ("Count overflow!").
#[must_use]
pub fn merged(kept: &[StoredPrice], fresh: &[StoredPrice]) -> Option<Vec<StoredPrice>> {
    if fresh.len() > SHOP_PRICELIST_MAX_NUM {
        return None;
    }
    let mut list = fresh.to_vec();
    list.extend(
        kept.iter()
            .filter(|(vnum, _)| !fresh.iter().any(|(named, _)| named == vnum))
            .take(SHOP_PRICELIST_MAX_NUM - fresh.len()),
    );
    Some(list)
}

/// Merge `fresh` into the prices `owner` keeps, as [`merged`] does, and store the result
/// in one transaction (`OnFlush`'s delete and rewrite, `server/server/db/Cache.cpp:282-307`).
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be read or written; a refused
/// row rolls the whole update back.
pub async fn update(store: &Store, owner: u32, fresh: &[StoredPrice]) -> Result<(), AccountError> {
    let kept = load(store, owner).await?;
    let Some(list) = merged(&kept, fresh) else {
        return Ok(());
    };
    let mut transaction = store.pool().begin().await?;
    sqlx::query("DELETE FROM myshop_pricelist WHERE owner_id = $1")
        .bind(i64::from(owner))
        .execute(&mut *transaction)
        .await?;
    for (slot, (vnum, price)) in (0i16..).zip(&list) {
        sqlx::query(
            "INSERT INTO myshop_pricelist (owner_id, slot, item_vnum, price) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(i64::from(owner))
        .bind(slot)
        .bind(i64::from(*vnum))
        .bind(i64::try_from(*price).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{merged, SHOP_PRICELIST_MAX_NUM};

    /// The new prices lead, a renamed vnum takes its new price, and the kept rest follow.
    #[test]
    fn fresh_prices_lead_and_the_kept_rest_follow() {
        let kept = [(10, 1), (20, 2), (30, 3)];
        let fresh = [(20, 9), (40, 4)];
        assert_eq!(
            merged(&kept, &fresh),
            Some(vec![(20, 9), (40, 4), (10, 1), (30, 3)])
        );
    }

    /// The kept rest is cut at the cap, and a list past the cap changes nothing.
    #[test]
    fn the_cap_cuts_the_kept_rest_and_refuses_a_longer_list() {
        let kept: Vec<_> = (1..=40).map(|vnum| (vnum, 1)).collect();
        let fresh = [(100, 5)];
        let list = merged(&kept, &fresh).expect("one fresh price fits");
        assert_eq!(list.len(), SHOP_PRICELIST_MAX_NUM);
        assert_eq!(list[0], (100, 5));
        assert_eq!(list[39], (39, 1));
        let long: Vec<_> = (1..=41).map(|vnum| (vnum, 1)).collect();
        assert_eq!(merged(&[], &long), None);
    }
}
