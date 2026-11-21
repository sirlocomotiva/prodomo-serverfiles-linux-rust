//! The reward window's rows, against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.
//!
//! The store's rules are that a solo count loads back, that a missing pid loads
//! empty, that a global claim loads back, and that a second claim keeps the
//! first name. All four are the database's answers.

mod support;

use db::reward::{load_global, load_solo, store_global, store_solo};
use support::ScratchDatabase;

#[tokio::test]
async fn reward_solo_and_global_load_and_store() {
    let Some(database) = ScratchDatabase::create().await else {
        return;
    };
    assert!(load_solo(&database.store, 1)
        .await
        .expect("the load runs")
        .is_empty());
    assert!(load_global(&database.store)
        .await
        .expect("the load runs")
        .is_empty());
    store_solo(&database.store, 1, 6, 42)
        .await
        .expect("the write runs");
    store_global(&database.store, 5, b"Alpha")
        .await
        .expect("the write runs");
    store_global(&database.store, 5, b"Yankee")
        .await
        .expect("the write runs");
    assert_eq!(
        load_solo(&database.store, 1)
            .await
            .expect("the load runs")
            .get(&6),
        Some(&42)
    );
    assert_eq!(
        load_global(&database.store)
            .await
            .expect("the load runs")
            .get(&5),
        Some(&b"Alpha".to_vec())
    );
}
