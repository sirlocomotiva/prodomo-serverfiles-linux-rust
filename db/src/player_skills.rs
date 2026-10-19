//! A character's skill levels: legacy's `TPlayerTable::skills` (`common/tables.h:612`).
//!
//! Each nonzero slot is a row of `player_skill` (`0010_player_skill.sql`), and a zero slot is
//! no row. The master type is derived from the level, so only the level and the read time
//! are stored.

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// One nonzero slot, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredSkill {
    /// The skill's vnum, below 255.
    pub vnum: u8,
    /// `bLevel`, 1 to 40.
    pub level: u8,
    /// `tNextRead`.
    pub next_read: i32,
}

/// Load a character's nonzero slots, in vnum order.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise, including for a stored value outside its range,
/// which the migration's checks make unreachable.
pub async fn load_skills(store: &Store, player: u32) -> Result<Vec<StoredSkill>, AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let rows = sqlx::query(
        "SELECT vnum, level, next_read FROM player_skill WHERE player_id = $1 ORDER BY vnum",
    )
    .bind(player_id)
    .fetch_all(store.pool())
    .await?;
    rows.iter()
        .map(|row| {
            let vnum: i16 = row.try_get("vnum")?;
            let level: i16 = row.try_get("level")?;
            let next_read: i32 = row.try_get("next_read")?;
            let vnum = u8::try_from(vnum).map_err(|_| AccountError::NoSuchPlayer(player))?;
            let level = u8::try_from(level).map_err(|_| AccountError::NoSuchPlayer(player))?;
            Ok(StoredSkill {
                vnum,
                level,
                next_read,
            })
        })
        .collect()
}

/// Write one slot: an upsert for a nonzero level, a delete for zero.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn store_skill(
    store: &Store,
    player: u32,
    vnum: u8,
    level: u8,
    next_read: i32,
) -> Result<(), AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    if level == 0 {
        sqlx::query("DELETE FROM player_skill WHERE player_id = $1 AND vnum = $2")
            .bind(player_id)
            .bind(i16::from(vnum))
            .execute(store.pool())
            .await?;
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO player_skill (player_id, vnum, level, next_read) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (player_id, vnum) DO UPDATE SET level = $3, next_read = $4",
    )
    .bind(player_id)
    .bind(i16::from(vnum))
    .bind(i16::from(level))
    .bind(next_read)
    .execute(store.pool())
    .await?;
    Ok(())
}
