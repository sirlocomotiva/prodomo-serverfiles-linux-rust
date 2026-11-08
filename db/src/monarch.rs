//! The monarch treasuries: legacy's `TMonarchInfo.money` (`C/tables.h:1574-1580`).
//!
//! The four cells are the rows of `monarch_money` (`0037_monarch_money.sql`), one per empire
//! slot 0 to 3, seeded at 0. The game reads a cell as an `int` (`CMonarch::GetMoney`,
//! `G/monarch.cpp:175-182`) and the DB server refuses a move outside 0 to 2000000000
//! (`D/Monarch.cpp:122-147`); the migration's checks hold the same range, so a hand-written
//! row cannot break the world's rule.

use crate::accounts::AccountError;
use crate::store::Store;

/// The four treasury cells, by empire slot.
pub type StoredMonarchMoney = [i64; 4];

/// Load the four cells. A missing row loads as 0, as the zeroed `TMonarchInfo` does
/// (`D/Monarch.cpp:10`).
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be read.
pub async fn load_all(store: &Store) -> Result<StoredMonarchMoney, AccountError> {
    let rows: Vec<(i16, i64)> =
        sqlx::query_as("SELECT empire, money FROM monarch_money ORDER BY empire")
            .fetch_all(store.pool())
            .await?;
    let mut money = [0; 4];
    for (empire, cell) in rows {
        if let Ok(slot) = usize::try_from(empire) {
            if let Some(cell_ref) = money.get_mut(slot) {
                *cell_ref = cell;
            }
        }
    }
    Ok(money)
}

/// Move the `empire` cell to `money`. The world judged the move before it asked, so the
/// row it names is always in range.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be written, including for a
/// slot or a cell the migration refuses.
pub async fn store_money(store: &Store, empire: u8, money: i64) -> Result<(), AccountError> {
    sqlx::query("UPDATE monarch_money SET money = $1 WHERE empire = $2")
        .bind(money)
        .bind(i16::from(empire))
        .execute(store.pool())
        .await?;
    Ok(())
}
