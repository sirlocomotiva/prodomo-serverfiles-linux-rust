//! A character's fish-event board: legacy's `fish_slots` and `fishEventUseCount`.
//!
//! Each set cell is a row of `fish_event_slot` (`0031_fish_event.sql`), and an empty cell is
//! no row. The use count is the one row of `fish_event_count`; a board never played is no
//! row, which loads as 0. The store checks only the widths the columns hold; which cells a
//! shape covers is the world's rule (`FishEventPlaceShape`, `G/char.cpp:11001-11037`), and
//! the load runs it again.

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// One set cell, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredFishCell {
    /// The board cell, below 24.
    pub cell: u8,
    /// `bIsMain`: the cell the shape was placed at.
    pub is_main: bool,
    /// `bShape`: the shape on the cell, 1 to 7.
    pub shape: u8,
}

/// Load a character's set cells, in cell order, and its use count.
///
/// A board never played loads as no cells and 0, as the zeroed `TPlayerFishEventSlot`
/// array and `m_dwFishUseCount` do (`G/char.cpp:475-477`).
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise, including for a stored value outside a `BYTE`,
/// which the migration's checks make unreachable.
pub async fn load_board(
    store: &Store,
    player: u32,
) -> Result<(Vec<StoredFishCell>, u32), AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let rows = sqlx::query(
        "SELECT cell, is_main, shape FROM fish_event_slot WHERE player_id = $1 ORDER BY cell",
    )
    .bind(player_id)
    .fetch_all(store.pool())
    .await?;
    let cells: Vec<StoredFishCell> = rows
        .iter()
        .map(|row| {
            Ok(StoredFishCell {
                cell: byte(row.try_get("cell")?, "cell")?,
                is_main: row.try_get("is_main")?,
                shape: byte(row.try_get("shape")?, "shape")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()?;
    let count: Option<i32> =
        sqlx::query_scalar("SELECT use_count FROM fish_event_count WHERE player_id = $1")
            .bind(player_id)
            .fetch_optional(store.pool())
            .await?;
    Ok((
        cells,
        count.map_or(0, |count| u32::try_from(count).unwrap_or(u32::MAX)),
    ))
}

/// Replace a character's board with `cells` and `use_count`, in one transaction, the way
/// legacy's save writes the whole array and the count (`G/char.cpp:1613-1614`).
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise: a cell the migration refuses rolls the whole write
/// back, so the stored board is the old one or the new one, never a mix.
pub async fn save_board(
    store: &Store,
    player: u32,
    cells: &[StoredFishCell],
    use_count: u32,
) -> Result<(), AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let count = i32::try_from(use_count).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let mut transaction = store.pool().begin().await?;
    sqlx::query("DELETE FROM fish_event_slot WHERE player_id = $1")
        .bind(player_id)
        .execute(&mut *transaction)
        .await?;
    for cell in cells {
        sqlx::query(
            "INSERT INTO fish_event_slot (player_id, cell, is_main, shape) VALUES ($1, $2, $3, $4)",
        )
        .bind(player_id)
        .bind(i16::from(cell.cell))
        .bind(cell.is_main)
        .bind(i16::from(cell.shape))
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "INSERT INTO fish_event_count (player_id, use_count) VALUES ($1, $2) \
         ON CONFLICT (player_id) DO UPDATE SET use_count = EXCLUDED.use_count",
    )
    .bind(player_id)
    .bind(count)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

/// A stored `integer` as the `BYTE` it holds.
fn byte(value: i32, column: &str) -> Result<u8, sqlx::Error> {
    u8::try_from(value).map_err(|_| sqlx::Error::ColumnDecode {
        index: column.to_string(),
        source: format!("{value} is not a BYTE").into(),
    })
}
