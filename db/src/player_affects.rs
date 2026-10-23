//! A player's saved affects: the rows a relogin restores.
//!
//! Legacy restores a player's timed affects at login (`LoadAffect`,
//! `G/char_affect.cpp:573-666`) from the `affect` table, one row per `(dwPID, bType,
//! bApplyOn, lApplyValue)`. Here `player_affect` (`0018_player_affect.sql`) holds only
//! the silence (`AFFECT_BLOCK_CHAT`, 223): the one affect whose duration a relogin
//! must keep. A silence survives death and its minutes are the sentence; the
//! revive-invisible is no-save (`IS_NO_SAVE_AFFECT`, `G/char_affect.cpp:43-55`) and
//! the fight's poison, stun and slow die with the moment.
//!
//! Each row carries its own full element, so the table is ready when a later affect
//! needs its row; a command that holds no affect the table knows deletes that
//! character's rows, as legacy's no-save rules do.

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// One saved affect: its kind and the full element a restore loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredAffect {
    /// `bType`: the affect's kind, such as 223 for the silence.
    pub affect_type: u32,
    /// `bApplyOn`: the point the affect applies to, 0 for none.
    pub apply_on: u8,
    /// `lApplyValue`: what the affect added to its point.
    pub apply_value: i32,
    /// `dwFlag`: the affect flag it sets, numbered from 1, or 0 for none.
    pub flag: u32,
    /// `lDuration`: the remaining ticks, which a restore loads unchanged.
    pub duration: i32,
    /// `lSPCost`: the spell points each tick spends.
    pub sp_cost: i32,
}

/// Load a character's saved affects, in `(affect_type, apply_on, apply_value)` order.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn load_affects(store: &Store, player: u32) -> Result<Vec<StoredAffect>, AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let rows = sqlx::query(
        "SELECT affect_type, apply_on, apply_value, flag, duration, sp_cost FROM player_affect \
         WHERE player_id = $1 ORDER BY affect_type, apply_on, apply_value",
    )
    .bind(player_id)
    .fetch_all(store.pool())
    .await?;
    rows.iter()
        .map(|row| {
            let affect_type: i32 = row.try_get("affect_type")?;
            let apply_on: i32 = row.try_get("apply_on")?;
            let flag: i32 = row.try_get("flag")?;
            Ok(StoredAffect {
                affect_type: u32::try_from(affect_type).unwrap_or(u32::MAX),
                apply_on: u8::try_from(apply_on).unwrap_or(u8::MAX),
                apply_value: row.try_get("apply_value")?,
                flag: u32::try_from(flag).unwrap_or(u32::MAX),
                duration: row.try_get("duration")?,
                sp_cost: row.try_get("sp_cost")?,
            })
        })
        .collect()
}

/// Replace a character's saved affects with `affects`, in one transaction.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn save_affects(
    store: &Store,
    player: u32,
    affects: &[StoredAffect],
) -> Result<(), AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let mut transaction = store.pool().begin().await?;
    sqlx::query("DELETE FROM player_affect WHERE player_id = $1")
        .bind(player_id)
        .execute(&mut *transaction)
        .await?;
    for affect in affects {
        sqlx::query(
            "INSERT INTO player_affect (player_id, affect_type, apply_on, apply_value, flag, \
             duration, sp_cost) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(player_id)
        .bind(i32::try_from(affect.affect_type).unwrap_or(i32::MAX))
        .bind(i32::from(affect.apply_on))
        .bind(affect.apply_value)
        .bind(i32::try_from(affect.flag).unwrap_or(i32::MAX))
        .bind(affect.duration)
        .bind(affect.sp_cost)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// Silence the character `name` names for `seconds` ticks
/// (`CClientManager::BlockChat`, `server/server/db/ClientManager.cpp:2159-2186`).
///
/// Legacy looks the name up in the player table and `REPLACE`s the silence
/// row — the `QUERY_ADD_AFFECT` write, which a 0-spelling reaches too (its
/// row stands on the next load, as a live 0 lay stands on the next tick); a
/// name no row carries is quiet. The lookup ignores case, as the world's
/// does, and the write is one transaction: the old row goes and the new row
/// lands, like `REPLACE`. Answers whether a row carried the name.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot answer.
pub async fn lay_affect(store: &Store, name: &str, seconds: i32) -> Result<bool, AccountError> {
    let mut transaction = store.pool().begin().await?;
    let player_id: Option<i32> =
        sqlx::query_scalar("SELECT id FROM player WHERE lower(name) = lower($1)")
            .bind(name)
            .fetch_optional(&mut *transaction)
            .await?;
    let Some(player_id) = player_id else {
        return Ok(false);
    };
    sqlx::query(
        "DELETE FROM player_affect WHERE player_id = $1 AND affect_type = 223 AND apply_on = 0 \
         AND apply_value = 0",
    )
    .bind(player_id)
    .execute(&mut *transaction)
    .await?;
    if seconds > 0 {
        sqlx::query(
            "INSERT INTO player_affect (player_id, affect_type, apply_on, apply_value, flag, \
             duration, sp_cost) VALUES ($1, 223, 0, 0, 0, $2, 0)",
        )
        .bind(player_id)
        .bind(seconds)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(true)
}
