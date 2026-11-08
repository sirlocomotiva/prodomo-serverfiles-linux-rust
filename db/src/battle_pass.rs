//! A character's battle-pass missions: the ten stored `TPlayerBattlePass` rows.
//!
//! Each mission is a row of `battle_pass` (`0040_battle_pass.sql`), keyed by
//! slot 0-9: daily in slots 0-4, weekly in 5-9, as `CreatePlayerProto` lays
//! them (`G/char.cpp:1635-1650`). A character that never opened the pass
//! holds no rows, which loads as an empty mission set. The store checks only
//! the widths the columns hold; which rows count for the running buckets is
//! the world's rule (`GetBattlePass`, `G/char.cpp:12826-12850`), and the
//! load runs it again.

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// One stored mission, as loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredPassMission {
    /// The stored slot, 0-9.
    pub slot: u8,
    /// `missionID`.
    pub mission_id: u16,
    /// `progress`.
    pub progress: u32,
    /// `type`: 0 daily, 1 weekly.
    pub kind: u8,
    /// `endTime`.
    pub end_time: u32,
}

/// Load a character's mission rows, in slot order.
///
/// A character that never opened the pass loads as no rows, as the zeroed
/// `tab.battlePass` blob does (`C/tables.h:551`).
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise, including for a stored value outside
/// its width, which the migration's checks make unreachable.
pub async fn load_missions(
    store: &Store,
    player: u32,
) -> Result<Vec<StoredPassMission>, AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let rows = sqlx::query(
        "SELECT slot, mission_id, progress, kind, end_time FROM battle_pass WHERE player_id = \
         $1 ORDER BY slot",
    )
    .bind(player_id)
    .fetch_all(store.pool())
    .await?;
    rows.iter()
        .map(|row| {
            let slot: i16 = row.try_get("slot")?;
            let mission_id: i32 = row.try_get("mission_id")?;
            let progress: i64 = row.try_get("progress")?;
            let kind: i16 = row.try_get("kind")?;
            let end_time: i64 = row.try_get("end_time")?;
            Ok(StoredPassMission {
                slot: u8::try_from(slot)
                    .map_err(|_| sqlx::Error::Decode("slot outside a byte".into()))?,
                mission_id: u16::try_from(mission_id)
                    .map_err(|_| sqlx::Error::Decode("mission_id outside a word".into()))?,
                progress: u32::try_from(progress)
                    .map_err(|_| sqlx::Error::Decode("progress outside a dword".into()))?,
                kind: u8::try_from(kind)
                    .map_err(|_| sqlx::Error::Decode("kind outside a byte".into()))?,
                end_time: u32::try_from(end_time)
                    .map_err(|_| sqlx::Error::Decode("end_time outside a dword".into()))?,
            })
        })
        .collect()
}

/// Save a character's mission rows, replacing the ten stored slots.
///
/// An empty set deletes every row, as the zeroed blob does.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be written.
pub async fn save_missions(
    store: &Store,
    player: u32,
    missions: &[StoredPassMission],
) -> Result<(), AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let mut tx = store.pool().begin().await?;
    sqlx::query("DELETE FROM battle_pass WHERE player_id = $1")
        .bind(player_id)
        .execute(&mut *tx)
        .await?;
    for mission in missions {
        sqlx::query(
            "INSERT INTO battle_pass (player_id, slot, mission_id, progress, kind, end_time) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(player_id)
        .bind(i16::from(mission.slot))
        .bind(i32::from(mission.mission_id))
        .bind(i64::from(mission.progress))
        .bind(i16::from(mission.kind))
        .bind(i64::from(mission.end_time))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
