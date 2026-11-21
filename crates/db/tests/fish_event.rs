//! The fish-event board store, against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.
//!
//! The store's rules are that a board never played loads as no cells and 0, that a save
//! replaces the whole board at once, and that a cell the migration refuses leaves the old
//! rows in place. All three are the database's answers.

mod support;

use db::accounts::create_account;
use db::credentials::{DeleteCode, Login, NewPassword};
use db::fish_event::{load_board, save_board, StoredFishCell};
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

fn cell(cell: u8, is_main: bool, shape: u8) -> StoredFishCell {
    StoredFishCell {
        cell,
        is_main,
        shape,
    }
}

#[tokio::test]
async fn an_unplayed_board_loads_empty_with_a_zero_count() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctfe", "FeOne").await;
    assert_eq!(load_board(store, owner).await.unwrap(), (Vec::new(), 0));
}

#[tokio::test]
async fn a_save_replaces_the_board_and_the_load_reads_cells_in_order() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctfe2", "FeTwo").await;
    let other = player(store, "acctfe3", "FeThree").await;

    save_board(
        store,
        owner,
        &[cell(12, false, 1), cell(0, true, 1), cell(6, false, 1)],
        9,
    )
    .await
    .unwrap();
    save_board(store, other, &[cell(3, true, 2)], 1)
        .await
        .unwrap();
    assert_eq!(
        load_board(store, owner).await.unwrap(),
        (
            vec![cell(0, true, 1), cell(6, false, 1), cell(12, false, 1)],
            9
        )
    );

    // The second save is the whole board: the cells it leaves out are gone, and the count
    // it carries stands.
    save_board(store, owner, &[cell(23, true, 2)], 11)
        .await
        .unwrap();
    assert_eq!(
        load_board(store, owner).await.unwrap(),
        (vec![cell(23, true, 2)], 11)
    );
    assert_eq!(
        load_board(store, other).await.unwrap(),
        (vec![cell(3, true, 2)], 1)
    );
}

#[tokio::test]
async fn a_cell_the_migration_refuses_leaves_the_old_board() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctfe4", "FeFour").await;
    save_board(store, owner, &[cell(0, true, 3)], 2)
        .await
        .unwrap();
    save_board(store, owner, &[cell(24, true, 3)], 3)
        .await
        .unwrap_err();
    assert_eq!(
        load_board(store, owner).await.unwrap(),
        (vec![cell(0, true, 3)], 2)
    );
}
