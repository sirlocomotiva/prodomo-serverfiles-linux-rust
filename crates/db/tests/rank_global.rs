//! The global rank store, against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.
//!
//! The store's rules are that a save writes the counters of one character, and refuses
//! a row of an ended season's generation; that a season's end writes the next season,
//! every prize and no counters in one transaction; and that a claim writes the items
//! and the `done` flag together.

mod support;

use db::accounts::create_account;
use db::accounts::Name;
use db::credentials::{DeleteCode, Login, NewPassword};
use db::items::{Attribute, ItemRow, RowChange};
use db::players::{create_player, Created, NewPlayer};
use db::rank_global::{claim, end_season, load, save_row, StoredRank, StoredWinner};
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
            name: Name::new(name).unwrap(),
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

fn rank(pid: u32, name: &[u8], counts: [i32; 9]) -> StoredRank {
    StoredRank {
        pid,
        name: name.to_vec(),
        level: 154,
        empire: 1,
        counts,
    }
}

fn row(id: u32, owner_id: u32, window_type: u8, pos: u32) -> ItemRow {
    ItemRow {
        id,
        owner_id: Some(owner_id),
        account_id: None,
        window_type,
        pos,
        vnum: 19,
        count: 1,
        refine_element: 0,
        transmutation: 0,
        flags: 0,
        anti_flags: 0,
        sockets: [0; 6],
        attributes: [Attribute {
            b_type: 0,
            s_value: 0,
        }; 7],
    }
}

#[tokio::test]
async fn a_save_writes_the_counters_and_the_load_reads_them_in_pid_order() {
    let Some(database) = ScratchDatabase::create().await else {
        return;
    };
    let store = &database.store;
    let first = player(store, "first", "First1").await;
    let second = player(store, "second", "Second2").await;
    assert!(save_row(
        store,
        0,
        &rank(second, b"Second2", [0, 2, 0, 0, 0, 0, 0, 0, 0])
    )
    .await
    .unwrap());
    assert!(save_row(
        store,
        0,
        &rank(first, b"First1", [1, 0, 0, 0, 0, 0, 0, 0, 0])
    )
    .await
    .unwrap());
    let boot = load(store).await.unwrap();
    assert_eq!(boot.season, 0);
    assert_eq!(boot.generation, 0);
    let pids: Vec<u32> = boot.rows.iter().map(|row| row.pid).collect();
    assert_eq!(pids, [first.min(second), first.max(second)]);
    let row = boot.rows.iter().find(|row| row.pid == second).unwrap();
    assert_eq!(row.counts[1], 2);
    // A save overwrites the level and the counters but keeps the first-scored Name.
    assert!(save_row(
        store,
        0,
        &rank(second, b"Changed", [0, 3, 0, 0, 0, 0, 0, 0, 0])
    )
    .await
    .unwrap());
    let boot = load(store).await.unwrap();
    let row = boot.rows.iter().find(|row| row.pid == second).unwrap();
    assert_eq!(row.name, b"Second2");
    assert_eq!(row.counts[1], 3);
}

#[tokio::test]
async fn a_seasons_end_writes_the_winners_and_refuses_the_old_rows() {
    let Some(database) = ScratchDatabase::create().await else {
        return;
    };
    let store = &database.store;
    let pid = player(store, "winner", "Winner").await;
    assert!(
        save_row(store, 0, &rank(pid, b"Winner", [5, 0, 0, 0, 0, 0, 0, 0, 0]))
            .await
            .unwrap()
    );
    end_season(
        store,
        1,
        1,
        &[StoredWinner {
            pid,
            name: b"Winner".to_vec(),
            position: 1,
            kind: 0,
            vnum: 19,
            count: 3,
        }],
    )
    .await
    .unwrap();
    let boot = load(store).await.unwrap();
    assert_eq!((boot.season, boot.generation), (1, 1));
    assert!(boot.rows.is_empty(), "the counters empty");
    assert_eq!(boot.unclaimed, [(pid, 19, 3)]);
    // A save taken before the end, written after it, puts no old row back.
    assert!(
        !save_row(store, 0, &rank(pid, b"Winner", [9, 0, 0, 0, 0, 0, 0, 0, 0]))
            .await
            .unwrap()
    );
    assert!(load(store).await.unwrap().rows.is_empty());
    // The new season's row writes.
    assert!(
        save_row(store, 1, &rank(pid, b"Winner", [1, 0, 0, 0, 0, 0, 0, 0, 0]))
            .await
            .unwrap()
    );
    assert_eq!(load(store).await.unwrap().rows.len(), 1);
}

#[tokio::test]
async fn a_claim_writes_the_done_flag_and_the_items_together() {
    let Some(database) = ScratchDatabase::create().await else {
        return;
    };
    let store = &database.store;
    let pid = player(store, "claimer", "Claimer").await;
    end_season(
        store,
        1,
        1,
        &[StoredWinner {
            pid,
            name: b"Claimer".to_vec(),
            position: 1,
            kind: 0,
            vnum: 19,
            count: 1,
        }],
    )
    .await
    .unwrap();
    // A claim with no items still marks the prize done.
    claim(store, pid, &[], None, &[]).await.unwrap();
    assert!(load(store).await.unwrap().unclaimed.is_empty());
    // A claim with items stores both in one transaction: the row lands.
    end_season(
        store,
        2,
        2,
        &[StoredWinner {
            pid,
            name: b"Claimer".to_vec(),
            position: 1,
            kind: 0,
            vnum: 19,
            count: 1,
        }],
    )
    .await
    .unwrap();
    let change = RowChange::Created(row(7, pid, 1, 0));
    claim(store, pid, std::slice::from_ref(&change), None, &[])
        .await
        .unwrap();
    let items: i64 = db::sqlx::query_scalar("SELECT COUNT(*) FROM item WHERE owner_id = $1")
        .bind(i32::try_from(pid).unwrap())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(items, 1);
    assert!(load(store).await.unwrap().unclaimed.is_empty());
    // A claim whose items collide stores neither.
    end_season(
        store,
        3,
        3,
        &[StoredWinner {
            pid,
            name: b"Claimer".to_vec(),
            position: 1,
            kind: 0,
            vnum: 19,
            count: 1,
        }],
    )
    .await
    .unwrap();
    let clash = RowChange::Created(row(8, pid, 1, 0));
    assert!(claim(store, pid, std::slice::from_ref(&clash), None, &[])
        .await
        .is_err());
    assert_eq!(load(store).await.unwrap().unclaimed, [(pid, 19, 1)]);
}
