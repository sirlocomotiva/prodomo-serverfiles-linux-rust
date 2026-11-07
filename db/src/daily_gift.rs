//! A character's daily-gift week: legacy's `daily_gift` row.
//!
//! One row per character (`0032_daily_gift.sql`): the week, the seven day statuses,
//! and the renew time. A character never opened loads as no row, which the world
//! reads as a week that matches nothing. The store checks only the widths the
//! columns hold; which row counts for the running week is the world's rule
//! (`GetDailyGiftStatus`, `G/char_daily_gifts.cpp:232-312`), and the load runs it
//! again.

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// The stored week, as loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredDailyGift {
    /// The week the statuses belong to.
    pub week: u8,
    /// The seven day statuses: 0 locked, 1 waiting, 2 collected.
    pub days: [u8; 7],
    /// The week's renew time, legacy's `datetime`.
    pub renew_time: i64,
}

/// Load a character's daily-gift row, if it has one.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise, including for a stored value outside a
/// `BYTE`, which the migration's checks make unreachable.
pub async fn load_week(
    store: &Store,
    player: u32,
) -> Result<Option<StoredDailyGift>, AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let row = sqlx::query(
        "SELECT week, day1, day2, day3, day4, day5, day6, day7, renew_time FROM daily_gift \
         WHERE player_id = $1",
    )
    .bind(player_id)
    .fetch_optional(store.pool())
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let week: i16 = row.try_get("week")?;
    let mut days = [0u8; 7];
    for (day, slot) in days.iter_mut().enumerate() {
        let column = format!("day{}", day + 1);
        let status: i16 = row.try_get(column.as_str())?;
        *slot = byte(status, &column)?;
    }
    Ok(Some(StoredDailyGift {
        week: byte(week, "week")?,
        days,
        renew_time: row.try_get("renew_time")?,
    }))
}

/// Replace a character's daily-gift row with `week`, the statuses and the renew time,
/// the way legacy's `REPLACE` writes the whole row (`G/char_daily_gifts.cpp:258`).
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn save_week(
    store: &Store,
    player: u32,
    week: &StoredDailyGift,
) -> Result<(), AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let days: Vec<i16> = week.days.iter().map(|day| i16::from(*day)).collect();
    sqlx::query(
        "INSERT INTO daily_gift (player_id, week, day1, day2, day3, day4, day5, day6, day7, \
         renew_time) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
         ON CONFLICT (player_id) DO UPDATE SET week = EXCLUDED.week, day1 = EXCLUDED.day1, \
         day2 = EXCLUDED.day2, day3 = EXCLUDED.day3, day4 = EXCLUDED.day4, day5 = EXCLUDED.day5, \
         day6 = EXCLUDED.day6, day7 = EXCLUDED.day7, renew_time = EXCLUDED.renew_time",
    )
    .bind(player_id)
    .bind(i16::from(week.week))
    .bind(days[0])
    .bind(days[1])
    .bind(days[2])
    .bind(days[3])
    .bind(days[4])
    .bind(days[5])
    .bind(days[6])
    .bind(week.renew_time)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Read an account's cash, the daily-gift currency legacy's `GetCash` selects
/// (`G/char_daily_gifts.cpp:314-342`).
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store fails. An account the store
/// never gave cash reads 0, as the column's default does.
pub async fn load_cash(store: &Store, login: &str) -> Result<u32, AccountError> {
    let cash: Option<i64> = sqlx::query_scalar("SELECT cash FROM account WHERE login = $1")
        .bind(login)
        .fetch_optional(store.pool())
        .await?;
    Ok(cash.map_or(0, |cash| u32::try_from(cash).unwrap_or(u32::MAX)))
}

/// A stored `smallint` as the `BYTE` it holds.
fn byte(value: i16, column: &str) -> Result<u8, sqlx::Error> {
    u8::try_from(value).map_err(|_| sqlx::Error::ColumnDecode {
        index: column.to_string(),
        source: format!("{value} is not a BYTE").into(),
    })
}
