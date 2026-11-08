//! The monarch cells: legacy's `TMonarchInfo` minus the windate
//! (`C/tables.h:1574-1580`).
//!
//! The four money and pid cells are the rows of `monarch_money`
//! (`0037_monarch_money.sql`, `0038_monarch_identity.sql`), one per empire slot 0 to 3,
//! seeded at 0. The game reads a treasury as an `int` (`CMonarch::GetMoney`,
//! `G/monarch.cpp:175-182`) and the DB server refuses a move outside 0 to 2000000000
//! (`D/Monarch.cpp:122-147`); the migration's checks hold the same range, so a
//! hand-written row cannot break the world's rule. The name is read back through
//! `player`, as `LoadMonarch` joins it (`D/Monarch.cpp:175-207`).

use crate::accounts::AccountError;
use crate::store::Store;

/// The four treasury cells, by empire slot.
pub type StoredMonarchMoney = [i64; 4];

/// The four monarch pids, by empire slot. 0 is no monarch, as the zeroed struct is.
pub type StoredMonarchPids = [u32; 4];

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
/// Load the four pid cells. A missing row loads as 0.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be read.
pub async fn load_pids(store: &Store) -> Result<StoredMonarchPids, AccountError> {
    let rows: Vec<(i16, i32)> =
        sqlx::query_as("SELECT empire, pid FROM monarch_money ORDER BY empire")
            .fetch_all(store.pool())
            .await?;
    let mut pids = [0; 4];
    for (empire, pid) in rows {
        if let Ok(slot) = usize::try_from(empire) {
            if let Some(cell) = pids.get_mut(slot) {
                *cell = u32::try_from(pid).unwrap_or(0);
            }
        }
    }
    Ok(pids)
}
/// Read the names behind the four pid cells back through `player`, as `LoadMonarch`
/// joins them (`D/Monarch.cpp:175-207`). A pid with no row reads as empty, as a name
/// a rename took away does.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be read.
pub async fn names_by_pids(
    store: &Store,
    pids: &StoredMonarchPids,
) -> Result<[Vec<u8>; 4], AccountError> {
    let rows: Vec<(i32, String)> = sqlx::query_as("SELECT id, name FROM player WHERE id = ANY($1)")
        .bind(pids.iter().map(|pid| i64::from(*pid)).collect::<Vec<i64>>())
        .fetch_all(store.pool())
        .await?;
    let mut names: [Vec<u8>; 4] = Default::default();
    for (slot, want) in pids.iter().enumerate() {
        if *want == 0 {
            continue;
        }
        if let Some((_, name)) = rows
            .iter()
            .find(|(id, _)| u32::try_from(*id).ok() == Some(*want))
        {
            names[slot] = name.as_bytes().to_vec();
        }
    }
    Ok(names)
}

/// The empire, pid and gold behind a character Name, as
/// `CMonarch::SetMonarch`'s query reads them (`D/Monarch.cpp:208-246`).
pub struct MonarchCandidate {
    /// The name's account's empire.
    pub empire: u8,
    /// The player ID.
    pub pid: u32,
    /// The held gold the treasury refills from.
    pub gold: i64,
}

/// Look the name up for `/setmonarch`. The match is case-insensitive, as names are
/// unique regardless of case.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot answer.
pub async fn candidate_by_name(
    store: &Store,
    name: &str,
) -> Result<Option<MonarchCandidate>, AccountError> {
    let row: Option<(i16, i32, i64)> = sqlx::query_as(
        "SELECT a.empire, p.id, p.gold FROM player AS p \
         JOIN account AS a ON a.id = p.account_id \
         WHERE lower(p.name) = lower($1)",
    )
    .bind(name)
    .fetch_optional(store.pool())
    .await?;
    Ok(row.and_then(|(empire, pid, gold)| {
        Some(MonarchCandidate {
            empire: u8::try_from(empire).ok()?,
            pid: u32::try_from(pid).ok()?,
            gold,
        })
    }))
}

/// Name the `empire` cell's monarch and refill the treasury, as
/// `CMonarch::SetMonarch`'s replace writes them.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be written.
pub async fn store_monarch(
    store: &Store,
    empire: u8,
    pid: u32,
    money: i64,
) -> Result<(), AccountError> {
    sqlx::query("UPDATE monarch_money SET pid = $1, money = $2 WHERE empire = $3")
        .bind(i64::from(pid))
        .bind(money)
        .bind(i16::from(empire))
        .execute(store.pool())
        .await?;
    Ok(())
}

/// Clear the `empire` cell's monarch and treasury. A row the update misses (a
/// hand-emptied slot) reads as cleared, as legacy's zeroed struct does.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the store cannot be written.
pub async fn clear_monarch(store: &Store, empire: u8) -> Result<(), AccountError> {
    sqlx::query("UPDATE monarch_money SET pid = 0, money = 0 WHERE empire = $1")
        .bind(i16::from(empire))
        .execute(store.pool())
        .await?;
    Ok(())
}
