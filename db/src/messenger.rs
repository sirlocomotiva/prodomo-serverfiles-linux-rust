//! The messenger list: legacy's `messenger_list` table
//! (`legacy/sql/schema/player.sql:593-607`).
//!
//! A friendship is two rows, one per direction, because `MessengerManager::AuthToAdd`
//! calls `AddToList` for each direction (`G/messenger_manager.cpp:330-332`) and each call
//! inserts its own row (`G/messenger_manager.cpp:348-350`). The world loads one account's
//! rows at its login (`MessengerManager::Login`, `G/messenger_manager.cpp:12-13`), and
//! the deletes name the pair either way:
//! `__FIX_DELETE_FRIEND_REFRESH__`'s own DELETE
//! (`G/messenger_manager.cpp:447-448`) and the character deletion
//! (`server/server/db/ClientManagerPlayer.cpp:1541`).
//!
//! Every value is a bound parameter: legacy interpolated the names into the SQL
//! statements (`G/messenger_manager.cpp:58,349,447-449,474-475`), which the ``EscapeString``
//! gate before them (`:51-55`, `:341-345`) tried to make safe. That gate is a Defect this
//! crate does not reproduce; the statements here are parameterised instead (AGENTS.md).

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// Error for a load that carries names in a shape the store refuses: the character Name's
/// rule
/// (`db::accounts::Name::check_name_alphabet`, `length.h:15`) in SQL.
///
/// The migration's `CHECK` constraints refuse it first, so every error is a database
/// error already rolled back.
///
/// # Errors
///
/// Returned by [`load_list`], [`add_pair`] and [`remove_pair`] as
/// [`AccountError::Database`] for anything the pool refused.
pub type MessengerError = AccountError;

/// Load the companions the character named on `account` holds, in the pair key's order.
///
/// The order is the legacy's set order: `m_Relation[account]` is a `std::set<std::string>`
/// (`G/messenger_manager.h:79`), and the rows come back in the primary key's own order,
/// which is the byte order of `(account, companion)`.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the pool refused.
pub async fn load_list(store: &Store, account: &str) -> Result<Vec<Vec<u8>>, AccountError> {
    let rows =
        sqlx::query("SELECT companion FROM messenger_list WHERE account = $1 ORDER BY companion")
            .bind(account)
            .fetch_all(store.pool())
            .await?;
    rows.iter()
        .map(|row| {
            let name: String = row.try_get("companion")?;
            Ok(name.into_bytes())
        })
        .collect()
}

/// Insert both rows of a friendship, one per direction, where the rows the pair already
/// holds are left as they are.
///
/// `MessengerManager::AuthToAdd` calls `AddToList` twice (`:330-332`), and each call
/// inserts its own row only when the relation is fresh (`AddToList`'s own check,
/// `:337-339`). Two inserts with the pair's own conflict rule are the same stored facts.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the pool refused. A refused row rolls the
/// whole insert back, so the stored pair is the old one or the new one, never a mix.
pub async fn add_pair(store: &Store, one: &str, other: &str) -> Result<(), AccountError> {
    let mut transaction = store.pool().begin().await?;
    sqlx::query(
        "INSERT INTO messenger_list (account, companion) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(one)
    .bind(other)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO messenger_list (account, companion) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(other)
    .bind(one)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

/// Delete both rows of the pair, in either key order, and answer how many went.
///
/// `MessengerManager::RemoveFromList`'s own DELETE
/// (`G/messenger_manager.cpp:447-448`) names both legs; the character deletion
/// (`ClientManagerPlayer.cpp:1541`) names every row the name is in, either as the
/// account or as the companion.
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the pool refused.
pub async fn remove_pair(store: &Store, one: &str, other: &str) -> Result<u64, AccountError> {
    let result = sqlx::query(
        "DELETE FROM messenger_list \
         WHERE (account = $1 AND companion = $2) OR (account = $2 AND companion = $1)",
    )
    .bind(one)
    .bind(other)
    .execute(store.pool())
    .await?;
    Ok(result.rows_affected())
}

/// Delete every row the character's Name is in, either as the account or the companion,
/// the way the deletion's own DELETE does (`ClientManagerPlayer.cpp:1541`).
///
/// # Errors
///
/// Returns [`AccountError::Database`] when the pool refused.
pub async fn wipe_name(store: &Store, name: &str) -> Result<u64, AccountError> {
    let result = sqlx::query("DELETE FROM messenger_list WHERE account = $1 OR companion = $1")
        .bind(name)
        .execute(store.pool())
        .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    // The statements take every name as a bound parameter, so a name with a quote
    // is a value, never SQL.
}
