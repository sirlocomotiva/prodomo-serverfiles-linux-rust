//! Event calendar rows against a real PostgreSQL 18 server.

mod support;

use db::events::{event_type, load_all};
use support::ScratchDatabase;

/// The rows come back in id order with every field, and the type names count as
/// `EEventTypes` does.
#[tokio::test]
async fn the_calendar_loads_every_row_in_id_order() {
    let Some(database) = ScratchDatabase::create().await else {
        return;
    };
    db::sqlx::query(
        "INSERT INTO event (id, type, start_at, end_at, value0, value1, completed) VALUES \
         (2, 'EVENT_TYPE_FISHING', to_timestamp(1769904000), to_timestamp(1772323199), 0, 0, \
         false), \
         (1, 'EVENT_TYPE_OX', to_timestamp(1772323200), to_timestamp(1775001600), 5, 6, true)",
    )
    .execute(database.store.pool())
    .await
    .unwrap();
    let events = load_all(&database.store).await.unwrap();
    assert_eq!(
        events.iter().map(|event| event.id).collect::<Vec<_>>(),
        [1, 2]
    );
    let ox = &events[0];
    assert_eq!(Some(ox.event_type), event_type("EVENT_TYPE_OX"));
    assert_eq!(
        (ox.start, ox.end, ox.value0, ox.value1, ox.completed),
        (1_772_323_200, 1_775_001_600, 5, 6, true)
    );
}

/// A row below the epoch or naming no event type is refused by the table.
#[tokio::test]
async fn a_bad_row_is_refused() {
    let Some(database) = ScratchDatabase::create().await else {
        return;
    };
    assert!(
        db::sqlx::query(
            "INSERT INTO event (type, start_at, end_at) VALUES ('EVENT_TYPE_NONE', \
             to_timestamp(1), to_timestamp(2))"
        )
        .execute(database.store.pool())
        .await
        .is_err(),
        "EVENT_TYPE_NONE is not a row"
    );
    assert!(
        db::sqlx::query(
            "INSERT INTO event (type, start_at, end_at) VALUES ('EVENT_TYPE_OX', \
             to_timestamp(0), to_timestamp(2))"
        )
        .execute(database.store.pool())
        .await
        .is_err(),
        "a zero start is not a row"
    );
    assert!(load_all(&database.store).await.unwrap().is_empty());
}
