//! The top-player rows `SetTopPlayerEffect` keeps (`player.sql:974-985`).
//!
//! One row per character that reached `TOP_PLAYER_MAX_LEVEL` (120) while fewer than
//! three rows stood (`0020_player_top.sql`). The first, second and third row grant
//! `AFFECT_PLAYER_TOP_1`, `_2` and `_3`; a fourth arrival and every later one is
//! granted nothing. A row is keyed by its player, so a second reaching inserts
//! nothing new.

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// How many `player_top` rows stand.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot answer.
pub async fn count(store: &Store) -> Result<i64, AccountError> {
    let row = sqlx::query("SELECT COUNT(*) AS stood FROM player_top")
        .fetch_one(store.pool())
        .await?;
    Ok(row.try_get("stood")?)
}

/// Stand the row of `player` at `level`: an insert, or nothing when one stands.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn stand(store: &Store, player: u32, level: u8) -> Result<(), AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    sqlx::query(
        "INSERT INTO player_top (player_id, level) VALUES ($1, $2) ON CONFLICT (player_id) DO \
         NOTHING",
    )
    .bind(player_id)
    .bind(i32::from(level))
    .execute(store.pool())
    .await?;
    Ok(())
}
