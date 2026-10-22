//! A character's quest flags: legacy's `quest::PC`'s flag map (`G/questpc.h:98-149`).
//!
//! A flag is a name and a value, and a flag whose value is zero is no flag at all
//! (`quest::PC::SetFlag`, `G/questpc.cpp:55-72`): the legacy deletes it and writes the
//! deletion to the store. Here each flag is a row of `quest_flag` (`0016_quest_flag.sql`)
//! and a flag with the value zero is no row, so the store holds what legacy's `quest`
//! table holds.
//!
//! The legacy writes the flags with the character (`quest::PC::Save`,
//! `G/questpc.cpp:519-590`, called by `CHARACTER::Save`, `G/char.cpp:1670-1678`) and reads
//! them back when the character enters the game (`CInputDB::QuestLoad`,
//! `G/input_db.cpp:1079-1101`), so a flag outlives the session that set it.

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// One flag, as stored: its whole name and its value.
///
/// The name is the quest's name and the flag's state, joined by a dot: the legacy splits
/// them at the first dot when it writes the row (`quest::PC::Save`,
/// `G/questpc.cpp:545-560`) and joins them again when it reads it
/// (`G/input_db.cpp:1089-1094`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredQuestFlag {
    /// The flag's whole name, such as `war.is_war_member`, as the exact bytes the quest
    /// script used.
    pub name: Vec<u8>,
    /// The flag's value, never zero.
    pub value: i32,
}

/// Load a character's flags, in name order.
///
/// The order is the legacy's: `quest::PC::m_FlagMap` is a `std::map`, and the load sets
/// the flags in the order the rows arrive (`G/input_db.cpp:1082-1096`), which the database
/// answers in the primary key's order.
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise.
pub async fn load_flags(store: &Store, player: u32) -> Result<Vec<StoredQuestFlag>, AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let rows = sqlx::query("SELECT name, value FROM quest_flag WHERE player_id = $1 ORDER BY name")
        .bind(player_id)
        .fetch_all(store.pool())
        .await?;
    rows.iter()
        .map(|row| {
            Ok(StoredQuestFlag {
                name: row.try_get("name")?,
                value: row.try_get("value")?,
            })
        })
        .collect()
}

/// Replace a character's flags with `flags`, in one transaction, the way legacy's save
/// writes the whole map (`quest::PC::Save`, `G/questpc.cpp:519-590`).
///
/// A flag whose value is zero is no flag and is not written: the legacy deletes it
/// instead (`quest::PC::SetFlag`, `G/questpc.cpp:60-64`).
///
/// # Errors
///
/// Returns [`AccountError::NoSuchPlayer`] for a player ID above `i32::MAX`, and
/// [`AccountError::Database`] otherwise: a flag the migration refuses rolls the whole
/// write back, so the stored flags are the old ones or the new ones, never a mix.
pub async fn save_flags(
    store: &Store,
    player: u32,
    flags: &[StoredQuestFlag],
) -> Result<(), AccountError> {
    let player_id = i32::try_from(player).map_err(|_| AccountError::NoSuchPlayer(player))?;
    let mut transaction = store.pool().begin().await?;
    sqlx::query("DELETE FROM quest_flag WHERE player_id = $1")
        .bind(player_id)
        .execute(&mut *transaction)
        .await?;
    for flag in flags {
        if flag.value == 0 {
            continue;
        }
        sqlx::query("INSERT INTO quest_flag (player_id, name, value) VALUES ($1, $2, $3)")
            .bind(player_id)
            .bind(&flag.name)
            .bind(flag.value)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}
