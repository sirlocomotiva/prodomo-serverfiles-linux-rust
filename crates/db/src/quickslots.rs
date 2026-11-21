//! A character's quickslots: legacy's `TPlayerTable::quickslot` (`common/tables.h:615`).
//!
//! Each set slot is a row of `quickslot` (`0007_quickslots.sql`), and an empty slot is no row.
//! The store checks only the widths the columns hold; which slots are legal is the world's rule
//! (`CHARACTER::SetQuickslot`, `G/char_quickslot.cpp:45-98`), and the load runs it again.

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// One set slot, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredQuickslot {
    /// The slot's index, below 36.
    pub slot: u8,
    /// `TQuickslot::type`, 1 to 3.
    pub kind: u8,
    /// `TQuickslot::pos`.
    pub pos: u8,
}

/// Load a character's set slots, in slot order, which is the order legacy's load sets them in
/// (`G/input_db.cpp:439-440`).
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise, including for a stored value outside a `BYTE`, which
/// the migration's checks make unreachable.
pub async fn load_quickslots(
    store: &Store,
    player: u32,
) -> Result<Vec<StoredQuickslot>, AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let rows =
        sqlx::query("SELECT slot, kind, pos FROM quickslot WHERE player_id = $1 ORDER BY slot")
            .bind(player_id)
            .fetch_all(store.pool())
            .await?;
    rows.iter()
        .map(|row| {
            Ok(StoredQuickslot {
                slot: byte(row.try_get("slot")?, "slot")?,
                kind: byte(row.try_get("kind")?, "kind")?,
                pos: byte(row.try_get("pos")?, "pos")?,
            })
        })
        .collect()
}

/// Replace a character's set slots with `slots`, in one transaction, the way legacy's save
/// writes the whole array (`G/char.cpp:1603-1604`).
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise: a slot the migration refuses rolls the whole write
/// back, so the stored slots are the old ones or the new ones, never a mix.
pub async fn save_quickslots(
    store: &Store,
    player: u32,
    slots: &[StoredQuickslot],
) -> Result<(), AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let mut transaction = store.pool().begin().await?;
    sqlx::query("DELETE FROM quickslot WHERE player_id = $1")
        .bind(player_id)
        .execute(&mut *transaction)
        .await?;
    for slot in slots {
        sqlx::query("INSERT INTO quickslot (player_id, slot, kind, pos) VALUES ($1, $2, $3, $4)")
            .bind(player_id)
            .bind(i16::from(slot.slot))
            .bind(i16::from(slot.kind))
            .bind(i16::from(slot.pos))
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// A stored `smallint` as the `BYTE` it holds.
fn byte(value: i16, column: &str) -> Result<u8, sqlx::Error> {
    u8::try_from(value).map_err(|_| sqlx::Error::ColumnDecode {
        index: column.to_string(),
        source: format!("{value} is not a BYTE").into(),
    })
}
