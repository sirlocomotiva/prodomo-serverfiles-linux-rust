//! The reward window's rows: legacy's `reward_solo/%u` and `reward_global/%u` files
//! (`G/char.cpp:12991-13049`, `G/char_manager.cpp:1831-1863`) as the tables of
//! `0039_reward.sql`.
//!
//! The solo counts are rows of `reward_solo` (pid, mission, count); the global
//! first-claim names are rows of `reward_global` (mission, name). The world
//! passes the mission's own max before it writes, so a stored count never
//! exceeds it, and the migration's checks hold the same ranges.

use std::collections::HashMap;

use crate::accounts::AccountError;
use crate::store::Store;

/// The solo counts of one character: the mission to its count.
pub type StoredRewardSolo = HashMap<u8, u32>;

/// The global claims: the mission to its first claimer's name.
pub type StoredRewardGlobal = HashMap<u8, Vec<u8>>;

/// Load the character's solo counts. Missing rows read as 0, as the absent file does.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be read.
pub async fn load_solo(store: &Store, pid: u32) -> Result<StoredRewardSolo, AccountError> {
    let pid = i32::try_from(pid).map_err(|_| AccountError::NoSuchPlayer(pid))?;
    let rows: Vec<(i16, i32)> =
        sqlx::query_as("SELECT mission, count FROM reward_solo WHERE player_id = $1")
            .bind(pid)
            .fetch_all(store.pool())
            .await?;
    let mut solo = StoredRewardSolo::new();
    for (mission, count) in rows {
        if let (Ok(mission), Ok(count)) = (u8::try_from(mission), u32::try_from(count)) {
            solo.insert(mission, count);
        }
    }
    Ok(solo)
}

/// Store the character's solo count for `mission`.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be written, including for a
/// mission or a count the migration refuses.
pub async fn store_solo(
    store: &Store,
    pid: u32,
    mission: u8,
    count: u32,
) -> Result<(), AccountError> {
    let pid = i32::try_from(pid).map_err(|_| AccountError::NoSuchPlayer(pid))?;
    let count = i32::try_from(count)
        .map_err(|_| AccountError::Corrupt(format!("a reward count above i32::MAX: {count}")))?;
    sqlx::query(
        "INSERT INTO reward_solo (player_id, mission, count) VALUES ($1, $2, $3) \
         ON CONFLICT (player_id, mission) DO UPDATE SET count = $3",
    )
    .bind(pid)
    .bind(i16::from(mission))
    .bind(count)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Load the global claims.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be read.
pub async fn load_global(store: &Store) -> Result<StoredRewardGlobal, AccountError> {
    let rows: Vec<(i16, Vec<u8>)> = sqlx::query_as("SELECT mission, name FROM reward_global")
        .fetch_all(store.pool())
        .await?;
    let mut global = StoredRewardGlobal::new();
    for (mission, name) in rows {
        if let Ok(mission) = u8::try_from(mission) {
            global.insert(mission, name);
        }
    }
    Ok(global)
}

/// Store the global claim of `mission` by `name`. The world claims once: a second claim
/// of the same mission keeps the first name, as `DoReward`'s `GetGlobalReward` miss does.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be written, including for a
/// mission the migration refuses.
pub async fn store_global(store: &Store, mission: u8, name: &[u8]) -> Result<(), AccountError> {
    sqlx::query("INSERT INTO reward_global (mission, name) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(i16::from(mission))
        .bind(name)
        .execute(store.pool())
        .await?;
    Ok(())
}
