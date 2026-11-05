//! The event calendar: legacy's `event` table (`legacy/sql/schema/player.sql:120-129`,
//! migration `0023_event.sql`).
//!
//! Legacy's DB server reads every row at boot (`D/ClientManagerBoot.cpp:1690-1730`) and each
//! game process keeps them in `CEventManager::m_mapEvent` (`G/event_manager.cpp:506-561`), so
//! the world loads them once and answers the client's calendar from memory.

use sqlx::Row;

use crate::accounts::AccountError;
use crate::store::Store;

/// The `EEventTypes` names in member order from `EVENT_TYPE_EXPERIENCE` (1)
/// (`G/event_manager.h:5-22`, `CEventManager::BuildEventNameMap`, `G/event_manager.cpp:648-665`).
/// `EVENT_TYPE_NONE` (0) is not a row: `Initialize` skips it.
pub const EVENT_TYPE_NAMES: [&str; 14] = [
    "EVENT_TYPE_EXPERIENCE",
    "EVENT_TYPE_ITEM_DROP",
    "EVENT_TYPE_BOSS",
    "EVENT_TYPE_METIN",
    "EVENT_TYPE_MINING",
    "EVENT_TYPE_GOLD_FROG",
    "EVENT_TYPE_MOONLIGHT",
    "EVENT_TYPE_HEXEGONAL_CHEST",
    "EVENT_TYPE_FISHING",
    "EVENT_TYPE_HIDE_AND_SEEK",
    "EVENT_TYPE_OX",
    "EVENT_TYPE_TANAKA",
    "EVENT_TYPE_SIEGE_WAR",
    "EVENT_TYPE_KINGDOM_WAR",
];

/// The `EEventTypes` member `name` is, or `None` for a name that is none of them.
pub fn event_type(name: &str) -> Option<u8> {
    EVENT_TYPE_NAMES
        .iter()
        .position(|known| *known == name)
        .and_then(|index| u8::try_from(index + 1).ok())
}

/// One event row, as `TEventTable` holds it (`C/tables.h:2244-2255`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredEvent {
    /// `dwID`.
    pub id: u32,
    /// The `EEventTypes` member of `szType`.
    pub event_type: u8,
    /// `startTime`, in seconds after the Unix epoch.
    pub start: i32,
    /// `endTime`, in seconds after the Unix epoch.
    pub end: i32,
    /// `iValue0`.
    pub value0: i32,
    /// `iValue1`.
    pub value1: i32,
    /// `bCompleted`.
    pub completed: bool,
}

/// Every event row, in id order (`m_mapEvent`'s order).
///
/// # Errors
///
/// [`AccountError::Corrupt`] for a row the schema's checks should have refused, and
/// [`AccountError::Database`] when the store cannot answer.
pub async fn load_all(store: &Store) -> Result<Vec<StoredEvent>, AccountError> {
    let rows = sqlx::query(
        "SELECT id, type, floor(EXTRACT(EPOCH FROM start_at))::bigint AS start_at, \
         floor(EXTRACT(EPOCH FROM end_at))::bigint AS end_at, value0, value1, completed \
         FROM event ORDER BY id",
    )
    .fetch_all(store.pool())
    .await?;
    let mut events = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: i32 = row.try_get("id")?;
        let name: String = row.try_get("type")?;
        let start: i64 = row.try_get("start_at")?;
        let end: i64 = row.try_get("end_at")?;
        let corrupt = || AccountError::Corrupt(format!("event row {id}"));
        events.push(StoredEvent {
            id: u32::try_from(id).map_err(|_| corrupt())?,
            event_type: event_type(&name).ok_or_else(corrupt)?,
            start: i32::try_from(start).map_err(|_| corrupt())?,
            end: i32::try_from(end).map_err(|_| corrupt())?,
            value0: row.try_get("value0")?,
            value1: row.try_get("value1")?,
            completed: row.try_get("completed")?,
        });
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::event_type;

    /// The names count from 1 in `EEventTypes` order; `EVENT_TYPE_NONE` and a stranger are none.
    #[test]
    fn the_type_names_count_from_experience() {
        assert_eq!(event_type("EVENT_TYPE_EXPERIENCE"), Some(1));
        assert_eq!(event_type("EVENT_TYPE_OX"), Some(11));
        assert_eq!(event_type("EVENT_TYPE_KINGDOM_WAR"), Some(14));
        assert_eq!(event_type("EVENT_TYPE_NONE"), None);
        assert_eq!(event_type("event_type_ox"), None);
    }
}
