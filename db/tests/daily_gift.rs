//! The daily-gift store, against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.
//!
//! The store's rules are that a week never opened loads as no row, that a save
//! replaces the whole row at once, and that cash reads 0 for an account the store
//! never gave cash. All three are the database's answers.

mod support;

use db::accounts::create_account;
use db::credentials::{DeleteCode, Login, NewPassword};
use db::daily_gift::{load_cash, load_week, save_week, StoredDailyGift};
use db::players::{create_player, Created, NewPlayer};
use db::store::Store;
use support::ScratchDatabase;

async fn player(store: &Store, login: &str, name: &str) -> u32 {
    let account = db::accounts::NewAccount {
        login: Login::new(login).expect("a valid login"),
        password: NewPassword::new(b"pw").unwrap().hash().unwrap(),
        delete_code: DeleteCode::random(),
    };
    let account = create_account(store, &account).await.unwrap();
    match create_player(
        store,
        account,
        &NewPlayer {
            slot: 0,
            name: db::accounts::Name::new(name).unwrap(),
            job: 6,
            st: 1,
            ht: 2,
            dx: 3,
            iq: 4,
            hp: 100,
            sp: 100,
            stamina: 100,
            part_base: 1,
            x: 0,
            y: 0,
        },
    )
    .await
    .unwrap()
    {
        Created::Player(id) => id,
        Created::Taken => panic!("{name} was taken"),
    }
}

#[tokio::test]
async fn a_week_never_opened_loads_as_no_row() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let player = player(store, "alice", "Alpha").await;
    assert_eq!(load_week(store, player).await.unwrap(), None);
    assert_eq!(load_cash(store, "alice").await.unwrap(), 0);
}

#[tokio::test]
async fn a_save_replaces_the_row_and_the_load_reads_it_back() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let player = player(store, "alice", "Alpha").await;
    let week = StoredDailyGift {
        week: 2,
        days: [2, 1, 0, 0, 0, 0, 0],
        renew_time: 1_800_000_000,
    };
    save_week(store, player, &week).await.unwrap();
    assert_eq!(load_week(store, player).await.unwrap(), Some(week));
    let week = StoredDailyGift {
        week: 3,
        days: [1, 0, 0, 0, 0, 0, 0],
        renew_time: 1_800_060_000,
    };
    save_week(store, player, &week).await.unwrap();
    assert_eq!(load_week(store, player).await.unwrap(), Some(week));
}

#[tokio::test]
async fn a_status_the_migration_refuses_leaves_the_old_row() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let player = player(store, "alice", "Alpha").await;
    let week = StoredDailyGift {
        week: 1,
        days: [1, 0, 0, 0, 0, 0, 0],
        renew_time: 100,
    };
    save_week(store, player, &week).await.unwrap();
    let bad = StoredDailyGift {
        week: 1,
        days: [3, 0, 0, 0, 0, 0, 0],
        renew_time: 100,
    };
    save_week(store, player, &bad).await.unwrap_err();
    assert_eq!(load_week(store, player).await.unwrap(), Some(week));
}
