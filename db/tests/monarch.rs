//! The monarch treasuries, against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.
//!
//! The store's rules are that the four cells seed at 0, that a stored cell loads back,
//! and that a missing row loads as 0. All three are the database's answers.

mod support;

use db::monarch::{load_all, store_money};
use db::sqlx;
use support::ScratchDatabase;
#[tokio::test]
async fn treasuries_seed_load_and_store() {
    let Some(database) = ScratchDatabase::create().await else {
        return;
    };
    assert_eq!(
        load_all(&database.store).await.expect("the load runs"),
        [0, 0, 0, 0]
    );
    store_money(&database.store, 1, 500)
        .await
        .expect("the write runs");
    sqlx::query("DELETE FROM monarch_money WHERE empire = 3")
        .execute(database.store.pool())
        .await
        .expect("the delete runs");
    assert_eq!(
        load_all(&database.store).await.expect("the load runs"),
        [0, 500, 0, 0]
    );
}
