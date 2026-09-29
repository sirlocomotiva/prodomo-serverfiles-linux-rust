//! The quickslot store, against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.
//!
//! The store's rules are that a save replaces every row of the player at once, and that a slot
//! the migration refuses leaves the old rows in place. Both are the database's answers.

mod support;

use db::accounts::create_account;
use db::credentials::{DeleteCode, Login, NewPassword};
use db::players::{create_player, Created, NewPlayer};
use db::quickslots::{load_quickslots, save_quickslots, StoredQuickslot};
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

fn slot(slot: u8, kind: u8, pos: u8) -> StoredQuickslot {
    StoredQuickslot { slot, kind, pos }
}

#[tokio::test]
async fn a_save_replaces_every_slot_and_the_load_reads_them_in_slot_order() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctqs", "QsOne").await;
    let other = player(store, "acctqs2", "QsTwo").await;
    assert!(load_quickslots(store, owner).await.unwrap().is_empty());

    save_quickslots(
        store,
        owner,
        &[slot(35, 3, 255), slot(0, 1, 179), slot(7, 2, 9)],
    )
    .await
    .unwrap();
    save_quickslots(store, other, &[slot(1, 2, 1)])
        .await
        .unwrap();
    assert_eq!(
        load_quickslots(store, owner).await.unwrap(),
        vec![slot(0, 1, 179), slot(7, 2, 9), slot(35, 3, 255)]
    );

    // The second save is the whole set: the slots it leaves out are gone.
    save_quickslots(store, owner, &[slot(7, 1, 4)])
        .await
        .unwrap();
    assert_eq!(
        load_quickslots(store, owner).await.unwrap(),
        vec![slot(7, 1, 4)]
    );
    save_quickslots(store, owner, &[]).await.unwrap();
    assert!(load_quickslots(store, owner).await.unwrap().is_empty());
    assert_eq!(
        load_quickslots(store, other).await.unwrap(),
        vec![slot(1, 2, 1)],
        "another player's slots are untouched"
    );
}

#[tokio::test]
async fn a_refused_slot_rolls_the_whole_save_back() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctqsbad", "QsBad").await;
    save_quickslots(store, owner, &[slot(2, 2, 5)])
        .await
        .unwrap();
    for refused in [slot(36, 1, 0), slot(3, 0, 0), slot(3, 4, 0)] {
        assert!(
            save_quickslots(store, owner, &[slot(1, 1, 1), refused])
                .await
                .is_err(),
            "{refused:?} is outside the migration's checks"
        );
        assert_eq!(
            load_quickslots(store, owner).await.unwrap(),
            vec![slot(2, 2, 5)],
            "the old slots stay"
        );
    }
}
