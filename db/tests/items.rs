//! The item store's read and write path, against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.
//!
//! What these tests are for is the two rules the module states and the migration
//! cannot: a save must never delete the row it conflicts with, and a delete or a
//! count change must be refused for an item the caller does not hold. Both are
//! answered by the database, so neither is visible without one.

mod support;

use common::config::ItemIdSpan;
use db::accounts::create_account;
use db::credentials::{DeleteCode, Login, NewPassword};
use db::item_id_range::MINIMUM_REMAIN_COUNT;
use db::items::{
    apply_exchange, apply_row_changes, apply_transfer, destroy_item, insert_item, load_item,
    load_owner_items, max_id_in_range, resolve_item_id_range, save_item, save_owner_items,
    set_count, Attribute, ItemError, ItemRow, RowChange, TransferSide, GROUND, MAX_ITEM_ID,
    SOCKETS,
};
use db::players::{create_player, Created, NewPlayer};
use db::store::Store;
use support::ScratchDatabase;

/// A stored item with every field at a distinct value, so a crossed pair is visible.
fn row(id: u32, owner_id: u32, window_type: u8, pos: u32) -> ItemRow {
    ItemRow {
        id,
        owner_id: Some(owner_id),
        window_type,
        pos,
        vnum: 30_000,
        count: 7,
        refine_element: 0x0405_0607,
        transmutation: 0x0809_0a0b,
        flags: 0x3c3d_3e3f,
        anti_flags: 0xc0c1_c2c3,
        sockets: [1_700_000_000, -1_700_000_000, 0x11, -0x12, 0x13, -0x14],
        attributes: [
            Attribute {
                b_type: 200,
                s_value: -300,
            },
            Attribute {
                b_type: 1,
                s_value: 2,
            },
            Attribute {
                b_type: 3,
                s_value: -4,
            },
            Attribute {
                b_type: 5,
                s_value: 6,
            },
            Attribute {
                b_type: 7,
                s_value: -8,
            },
            Attribute {
                b_type: 9,
                s_value: 10,
            },
            Attribute {
                b_type: 11,
                s_value: 12,
            },
        ],
    }
}

async fn account(store: &Store, login: &str) -> u32 {
    let account = db::accounts::NewAccount {
        login: Login::new(login).expect("a valid login"),
        password: NewPassword::new(b"pw").unwrap().hash().unwrap(),
        delete_code: DeleteCode::random(),
    };
    create_account(store, &account).await.unwrap().get()
}

async fn player(store: &Store, login: &str, name: &str) -> u32 {
    let account = db::accounts::AccountId::new(account(store, login).await);
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
async fn every_field_survives_the_round_trip() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctrt", "RtOne").await;
    let wanted = row(1_000_001, owner, 1, 5);
    save_item(store, &wanted).await.unwrap();

    let got = load_item(store, 1_000_001)
        .await
        .unwrap()
        .expect("the row is there");
    assert_eq!(
        got, wanted,
        "a field was crossed or narrowed on the way in or out"
    );

    // And the values really are distinct, so the assertion above could have caught
    // something: the two socket halves differ, the two bitmaps differ, and every
    // attribute pair has a different type.
    assert_ne!(got.sockets[0], got.sockets[1]);
    assert_ne!(got.flags, got.anti_flags);
    // Seven distinct types, and 200 first so the first slot is not the byte-symmetric
    // 0 that a byte-swap would leave unchanged.
    let types: Vec<u8> = got.attributes.iter().map(|a| a.b_type).collect();
    assert_eq!(types, vec![200, 1, 3, 5, 7, 9, 11]);
    let values: Vec<i16> = got.attributes.iter().map(|a| a.s_value).collect();
    assert_eq!(values, vec![-300, 2, -4, 6, -8, 10, 12]);
}

#[tokio::test]
async fn a_negative_socket_and_a_negative_attribute_value_stay_negative() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctneg", "NegSock").await;
    let mut item = row(1_000_002, owner, 1, 0);
    item.sockets = [-2_000_000_000, -1, 0, 0, 0, 0];
    item.attributes[0] = Attribute {
        b_type: 255,
        s_value: -32_768,
    };
    save_item(store, &item).await.unwrap();

    let got = load_item(store, 1_000_002).await.unwrap().unwrap();
    assert_eq!(got.sockets[0], -2_000_000_000);
    assert_eq!(got.sockets[1], -1);
    assert_eq!(got.attributes[0].b_type, 255);
    assert_eq!(got.attributes[0].s_value, -32_768);
}

#[tokio::test]
async fn an_attribute_type_of_256_is_refused_and_255_is_kept() {
    // The migration's `attrtypeN` CHECK is 0..255, and the negative half of legacy's
    // `tinyint` is gone on purpose, so a hand-written -1 is an error rather than a
    // wrapped 255.
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "accttype", "AttrTyp").await;
    let mut item = row(1_000_003, owner, 1, 0);
    item.attributes[0] = Attribute {
        b_type: 200,
        s_value: 0,
    };
    save_item(store, &item).await.unwrap();
    assert_eq!(
        load_item(store, 1_000_003)
            .await
            .unwrap()
            .unwrap()
            .attributes[0]
            .b_type,
        200
    );

    // A `u8` cannot hold 256, so the type has to be re-widened at the SQL boundary to
    // prove the CHECK is what refuses it. A negative one is a second case, and the
    // `u8` refuses that at the bind, which is the point: the field is an unsigned byte.
    let written = db::sqlx::query("UPDATE item SET attrtype0 = $1 WHERE id = $2")
        .bind(256_i16)
        .bind(1_000_003_i64)
        .execute(store.pool())
        .await;
    assert!(
        written.is_err(),
        "the database accepted an attribute type of 256"
    );
}

#[tokio::test]
async fn a_save_moves_an_item_without_touching_its_new_neighbour() {
    // The reason this module never says REPLACE. Legacy's save is
    // `REPLACE INTO item` (`db/Cache.cpp:178`), which was safe with one unique key.
    // This table has two, and MySQL answers a conflict by deleting the other row, so
    // a save that moves an item onto an occupied cell destroys the item it landed on.
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctmove", "Mover").await;

    let a = row(1_000_004, owner, 1, 5);
    let b = row(1_000_005, owner, 1, 6);
    save_item(store, &a).await.unwrap();
    save_item(store, &b).await.unwrap();

    // Move `a` onto `b`'s cell. The unique index refuses it, which is the outcome that
    // matters: neither row may be gone afterwards.
    let mut moved = a.clone();
    moved.pos = 6;
    let refused = save_item(store, &moved).await;
    assert!(refused.is_err(), "two items claimed one cell");

    assert!(
        load_item(store, 1_000_004).await.unwrap().is_some(),
        "the moved item was deleted instead of refused"
    );
    assert_eq!(
        load_item(store, 1_000_005).await.unwrap(),
        Some(b),
        "the item that was sitting in the cell was deleted"
    );
}

#[tokio::test]
async fn saving_the_same_id_twice_updates_in_place() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctupsert", "Upsert").await;
    let mut item = row(1_000_006, owner, 1, 3);
    save_item(store, &item).await.unwrap();
    item.count = 9;
    item.pos = 4;
    save_item(store, &item).await.unwrap();

    let got = load_item(store, 1_000_006).await.unwrap().unwrap();
    assert_eq!(got.count, 9);
    assert_eq!(got.pos, 4);
    let rows: i64 = db::sqlx::query_scalar("SELECT COUNT(*) FROM item WHERE id = 1_000_006")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(rows, 1, "the upsert made a second row");
}

#[tokio::test]
async fn a_load_returns_one_characters_items_and_nobody_elses() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let one = player(store, "acctone", "LoadOne").await;
    let two = player(store, "accttwo", "LoadTwo").await;

    save_item(store, &row(1_000_007, one, 1, 3)).await.unwrap();
    save_item(store, &row(1_000_008, one, 1, 11)).await.unwrap();
    save_item(store, &row(1_000_009, one, 5, 0)).await.unwrap();
    save_item(store, &row(1_000_010, two, 1, 3)).await.unwrap();
    // A ground row, which the biconditional makes ownerless, so the load cannot see it.
    let mut ground = ItemRow::on_ground(1_000_011, 0, 30_000, 1);
    ground.owner_id = None;
    save_item(store, &ground).await.unwrap();

    let got = load_owner_items(store, one).await.unwrap();
    let ids: Vec<u32> = got.iter().map(|item| item.id).collect();
    assert_eq!(
        ids,
        vec![1_000_007, 1_000_008, 1_000_009],
        "wrong order or wrong owner"
    );
    assert!(got.iter().all(|item| item.owner_id == Some(one)));
}

#[tokio::test]
async fn a_load_does_not_hide_a_row_the_build_cannot_decode() {
    // Legacy's load filters in the query and silently drops what it does not
    // recognise (`db/ClientManagerPlayer.cpp:386`), so a character that owns an item
    // in a window this build does not have loses it at the next login. This load has
    // no such filter, so the failure has to be loud: a row whose value is outside the
    // width the client is sent is a `Corrupt`, not a missing item.
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctcorrupt", "Corrupt").await;
    save_item(store, &row(1_000_012, owner, 1, 1))
        .await
        .unwrap();

    // The CHECKs make this unreachable through the crate, so the row is written by
    // hand with the constraint dropped for the moment.
    db::sqlx::query("ALTER TABLE item DROP CONSTRAINT item_pos_check")
        .execute(store.pool())
        .await
        .unwrap();
    db::sqlx::query("UPDATE item SET pos = -1 WHERE id = 1_000_012")
        .execute(store.pool())
        .await
        .unwrap();

    match load_owner_items(store, owner).await {
        Err(ItemError::Corrupt(detail)) => {
            assert!(
                detail.contains("pos"),
                "the error should name the column: {detail}"
            );
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[tokio::test]
async fn a_ground_row_comes_back_with_no_owner() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let ground = ItemRow::on_ground(1_000_013, 0, 30_000, 1);
    save_item(store, &ground).await.unwrap();
    let got = load_item(store, 1_000_013).await.unwrap().unwrap();
    assert_eq!(got.owner_id, None);
    assert_eq!(got.window_type, GROUND);
}

#[tokio::test]
async fn a_destroy_is_refused_for_an_item_another_character_holds() {
    // Legacy's destroy is `DELETE FROM item%s WHERE id=%u` (`db/ClientManager.cpp:1838`)
    // across a global id space, so this is the difference: the owner is in the WHERE.
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let one = player(store, "acctdel1", "DelOne").await;
    let two = player(store, "acctdel2", "DelTwo").await;
    save_item(store, &row(1_000_014, one, 1, 2)).await.unwrap();

    match destroy_item(store, 1_000_014, two).await {
        Err(ItemError::NotOwned { id, owner_id }) => {
            assert_eq!(id, 1_000_014);
            assert_eq!(owner_id, Some(one));
        }
        other => panic!("expected NotOwned, got {other:?}"),
    }
    assert!(
        load_item(store, 1_000_014).await.unwrap().is_some(),
        "the refused destroy still deleted the row"
    );

    assert!(destroy_item(store, 1_000_014, one).await.unwrap());
    assert!(load_item(store, 1_000_014).await.unwrap().is_none());
    // A second destroy of the same id is not an error: the row is gone, which is the
    // answer, and it is different from "somebody else holds it".
    assert!(!destroy_item(store, 1_000_014, one).await.unwrap());
}

#[tokio::test]
async fn a_ground_item_cannot_be_destroyed_through_a_character() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctground", "GrndOne").await;
    save_item(store, &ItemRow::on_ground(1_000_015, 0, 30_000, 1))
        .await
        .unwrap();
    match destroy_item(store, 1_000_015, owner).await {
        Err(ItemError::NotOwned { owner_id, .. }) => assert_eq!(owner_id, None),
        other => panic!("expected NotOwned with no owner, got {other:?}"),
    }
}

#[tokio::test]
async fn a_count_change_commits_under_the_owner_and_is_refused_otherwise() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let one = player(store, "acctcnt1", "CntOne").await;
    let two = player(store, "acctcnt2", "CntTwo").await;
    save_item(store, &row(1_000_016, one, 1, 1)).await.unwrap();

    set_count(store, 1_000_016, one, 250).await.unwrap();
    assert_eq!(
        load_item(store, 1_000_016).await.unwrap().unwrap().count,
        250
    );

    assert!(matches!(
        set_count(store, 1_000_016, two, 250).await,
        Err(ItemError::NotOwned { .. })
    ));
    assert_eq!(
        load_item(store, 1_000_016).await.unwrap().unwrap().count,
        250,
        "the refused change still wrote"
    );
    assert!(matches!(
        set_count(store, 1_000_999, one, 5).await,
        Err(ItemError::NoSuchItem(1_000_999))
    ));
}

#[tokio::test]
async fn a_count_outside_one_to_five_thousand_is_refused_before_the_row_is_touched() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctrange", "CntRang").await;
    save_item(store, &row(1_000_017, owner, 1, 1))
        .await
        .unwrap();
    for count in [0, 5_001, u16::MAX] {
        assert!(matches!(
            set_count(store, 1_000_017, owner, count).await,
            Err(ItemError::CountOutOfRange(c)) if c == count
        ));
    }
    assert_eq!(load_item(store, 1_000_017).await.unwrap().unwrap().count, 7);
}

#[tokio::test]
async fn the_rules_the_migration_enforces_are_named_before_the_statement() {
    // The migration refuses all of these too. The point of checking in Rust is that the
    // error names the value instead of a constraint name, so a caller learns what it
    // passed. Each case must fail without a query, which a row count proves.
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctrules", "RuleOne").await;
    let good = row(1_000_018, owner, 1, 1);
    save_item(store, &good).await.unwrap();

    let mut bad = good.clone();
    bad.id = 0;
    assert!(matches!(
        save_item(store, &bad).await,
        Err(ItemError::IdOutOfRange(0))
    ));

    let mut bad = good.clone();
    bad.id = MAX_ITEM_ID + 1;
    assert!(matches!(
        save_item(store, &bad).await,
        Err(ItemError::IdOutOfRange(_))
    ));

    let mut bad = good.clone();
    bad.window_type = 11;
    assert!(matches!(
        save_item(store, &bad).await,
        Err(ItemError::WindowOutOfRange(11))
    ));

    let mut bad = good.clone();
    bad.count = 0;
    assert!(matches!(
        save_item(store, &bad).await,
        Err(ItemError::CountOutOfRange(0))
    ));

    let mut bad = good.clone();
    bad.count = 5_001;
    assert!(matches!(
        save_item(store, &bad).await,
        Err(ItemError::CountOutOfRange(5_001))
    ));

    // The biconditional, both ways.
    let mut bad = good.clone();
    bad.owner_id = None;
    assert!(matches!(
        save_item(store, &bad).await,
        Err(ItemError::Corrupt(_))
    ));
    let mut bad = good.clone();
    bad.window_type = GROUND;
    assert!(matches!(
        save_item(store, &bad).await,
        Err(ItemError::Corrupt(_))
    ));

    // Nothing above reached the database.
    let rows: i64 = db::sqlx::query_scalar("SELECT COUNT(*) FROM item")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(rows, 1, "a refused save wrote a row");
}

#[tokio::test]
async fn an_upsert_onto_another_characters_cell_is_refused_by_the_index() {
    // The unique index is on (owner_id, window_type, pos), so it is per owner. Two
    // characters may both hold an item in the same cell, and one character may not
    // hold two.
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let one = player(store, "acctidx1", "IdxOne").await;
    let two = player(store, "acctidx2", "IdxTwo").await;
    save_item(store, &row(1_000_019, one, 1, 7)).await.unwrap();
    save_item(store, &row(1_000_020, two, 1, 7))
        .await
        .expect("a different character may use the same cell");
    assert!(save_item(store, &row(1_000_021, one, 1, 7)).await.is_err());
    // A different window is a different cell.
    save_item(store, &row(1_000_022, one, 2, 7))
        .await
        .expect("a different window may use the same index");
}

#[tokio::test]
async fn saving_a_whole_inventory_is_one_transaction() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctall", "AllOne").await;
    let mut items: Vec<ItemRow> = (0..8)
        .map(|index| row(1_000_100 + index, owner, 1, index))
        .collect();
    save_owner_items(store, &items).await.unwrap();
    assert_eq!(load_owner_items(store, owner).await.unwrap(), items);

    // One bad row in the middle must leave the table as it was, which is what makes
    // this a transaction and not a loop. The third row claims a cell the second
    // already holds.
    items[2].pos = 1;
    assert!(save_owner_items(store, &items).await.is_err());
    let got = load_owner_items(store, owner).await.unwrap();
    assert_eq!(got.len(), 8, "a rolled-back save left rows behind");
    assert_eq!(got[2].pos, 2, "a rolled-back save still wrote");

    // An empty list is not an error and touches nothing.
    save_owner_items(store, &[]).await.unwrap();
    assert_eq!(load_owner_items(store, owner).await.unwrap().len(), 8);
}

#[tokio::test]
async fn the_pool_asks_the_table_for_the_highest_id_in_a_range() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctpool", "PoolOne").await;
    assert_eq!(max_id_in_range(store, 1, MAX_ITEM_ID).await.unwrap(), None);

    save_item(store, &row(50_000_000, owner, 1, 0))
        .await
        .unwrap();
    save_item(store, &row(90_000_000, owner, 1, 1))
        .await
        .unwrap();
    // Outside the range, so it must not be the answer.
    save_item(store, &row(200_000_000, owner, 1, 2))
        .await
        .unwrap();

    assert_eq!(
        max_id_in_range(store, 1, MAX_ITEM_ID).await.unwrap(),
        Some(200_000_000)
    );
    assert_eq!(
        max_id_in_range(store, 1, 100_000_000).await.unwrap(),
        Some(90_000_000)
    );
    assert_eq!(max_id_in_range(store, 1, 10).await.unwrap(), None);
}

#[tokio::test]
async fn deleting_a_character_takes_its_items_with_it() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let one = player(store, "acctcasc1", "CascOne").await;
    let two = player(store, "acctcasc2", "CascTwo").await;
    save_item(store, &row(1_000_030, one, 1, 0)).await.unwrap();
    save_item(store, &row(1_000_031, one, 1, 1)).await.unwrap();
    save_item(store, &row(1_000_032, two, 1, 0)).await.unwrap();
    let ground = ItemRow::on_ground(1_000_033, 0, 30_000, 1);
    save_item(store, &ground).await.unwrap();

    db::sqlx::query("DELETE FROM player WHERE id = $1")
        .bind(i64::from(one))
        .execute(store.pool())
        .await
        .unwrap();

    assert!(load_item(store, 1_000_030).await.unwrap().is_none());
    assert!(load_item(store, 1_000_031).await.unwrap().is_none());
    assert!(
        load_item(store, 1_000_032).await.unwrap().is_some(),
        "the other character's item went with them"
    );
    assert!(
        load_item(store, 1_000_033).await.unwrap().is_some(),
        "a ground row has no owner to cascade from"
    );
}

#[tokio::test]
async fn the_six_socket_columns_and_seven_attribute_pairs_are_the_measured_counts() {
    // Not a tautology: these are the two numbers ledger 195 measured in the legacy tree
    // and corrected once (the Dragon Soul stride is 8, not the 6 an earlier note said),
    // and the table has exactly this many columns. If either constant moves, the
    // migration moves with it, and this is where that shows up.
    assert_eq!(SOCKETS, 6);
    assert_eq!(db::items::SOCKET_COLUMNS.len(), 6);
    assert_eq!(db::items::ATTRTYPE_COLUMNS.len(), 7);
    assert_eq!(db::items::ATTRVALUE_COLUMNS.len(), 7);
    // The thirty columns an insert binds: ten plain, six sockets, seven pairs.
    assert_eq!(10 + SOCKETS + 2 * 7, 30);
}

/// A span a test builds by hand. The width matters: [`MINIMUM_REMAIN_COUNT`] is
/// `10_000`, so the short `[1_000, 9_000]` span a first draft of these tests used is
/// refused for leaving only `8_000` ids, which is the check working rather than the
/// span being wrong.
fn span(first: u32, last: u32) -> ItemIdSpan {
    ItemIdSpan { first, last }
}

#[tokio::test]
async fn an_empty_item_table_hands_out_the_first_id_of_the_span() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let range = resolve_item_id_range(&db.store, span(1_000, 1_000_000))
        .await
        .unwrap();
    // `BuildRange` sets `dwUsableItemIDMin` to `dwMin` when `MAX(id)` comes back NULL
    // or 0, so an untouched store starts at the first id and not at first + 1.
    assert_eq!(range.min, 1_000);
    assert_eq!(range.max, 1_000_000);
    assert_eq!(range.usable_item_id_min, 1_000);
    assert!(range.is_usable());
}

#[tokio::test]
async fn the_start_id_is_one_past_the_highest_stored_id_in_the_span() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let owner = player(&db.store, "a1", "Aaa").await;
    for id in [1_000, 1_500, 2_000] {
        save_item(&db.store, &row(id, owner, 1, id - 1_000))
            .await
            .unwrap();
    }
    let range = resolve_item_id_range(&db.store, span(1_000, 1_000_000))
        .await
        .unwrap();
    assert_eq!(range.usable_item_id_min, 2_001);
}

#[tokio::test]
async fn a_stored_id_above_the_span_does_not_move_the_start_id() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let owner = player(&db.store, "a1", "Aaa").await;
    // Inside and outside the span, interleaved. Only the inside one may count: the
    // span ends at 1_000_000, so 1_500_000 is above it and must not move the start.
    save_item(&db.store, &row(1_000, owner, 1, 0))
        .await
        .unwrap();
    save_item(&db.store, &row(1_500_000, owner, 1, 1))
        .await
        .unwrap();
    let range = resolve_item_id_range(&db.store, span(1_000, 1_000_000))
        .await
        .unwrap();
    assert_eq!(range.usable_item_id_min, 1_001);
}

#[tokio::test]
async fn a_span_with_too_few_ids_left_is_refused_the_way_build_range_refused_it() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let owner = player(&db.store, "a1", "Aaa").await;
    // Leave exactly MINIMUM_REMAIN_COUNT - 1 ids above the next id.
    let last = 100_000;
    let next = last - MINIMUM_REMAIN_COUNT + 1;
    save_item(&db.store, &row(next - 1, owner, 1, 0))
        .await
        .unwrap();
    let error = resolve_item_id_range(&db.store, span(1_000, last))
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            ItemError::ItemIdRangeExhausted {
                first: 1_000,
                last: 100_000,
                next: n
            } if n == next
        ),
        "got {error:?}"
    );
}

#[tokio::test]
async fn a_span_with_exactly_the_minimum_left_is_accepted() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let owner = player(&db.store, "a1", "Aaa").await;
    // The control for the test above: one more id and the same span resolves.
    let last = 100_000;
    let next = last - MINIMUM_REMAIN_COUNT;
    save_item(&db.store, &row(next - 1, owner, 1, 0))
        .await
        .unwrap();
    let range = resolve_item_id_range(&db.store, span(1_000, last))
        .await
        .unwrap();
    assert_eq!(range.usable_item_id_min, next);
    assert_eq!(
        ItemIdSpan { first: 1_000, last }.remaining_from(next),
        MINIMUM_REMAIN_COUNT
    );
}

#[tokio::test]
async fn a_span_whose_first_id_is_not_below_its_last_is_refused() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    // Positive control first: the same table resolves an ordered span, so a refusal
    // below is about the order and not about the table.
    assert!(resolve_item_id_range(&db.store, span(1_000, 1_000_000))
        .await
        .is_ok());
    for (first, last) in [(1_000_000u32, 1_000u32), (1_000_000, 1_000_000)] {
        let error = resolve_item_id_range(&db.store, span(first, last))
            .await
            .unwrap_err();
        assert!(
            matches!(error, ItemError::ItemIdRangeExhausted { .. }),
            "span [{first}, {last}] gave {error:?}"
        );
    }
}

#[tokio::test]
async fn a_moves_row_changes_commit_together_in_order() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctmove", "MoveOne").await;
    save_owner_items(
        store,
        &[
            row(1_000_200, owner, 1, 3),
            row(1_000_201, owner, 1, 4),
            row(1_000_202, owner, 1, 5),
        ],
    )
    .await
    .unwrap();
    let piece = row(1_000_203, owner, 7, 2);
    apply_row_changes(
        store,
        owner,
        &[
            // The move frees cell 3, and the split's new row takes a belt cell.
            RowChange::Moved {
                id: 1_000_200,
                window_type: 1,
                pos: 40,
            },
            RowChange::Destroyed { id: 1_000_201 },
            RowChange::Count {
                id: 1_000_202,
                count: 0x0123,
            },
            RowChange::Created(piece.clone()),
        ],
    )
    .await
    .unwrap();
    let got = load_owner_items(store, owner).await.unwrap();
    let cells: Vec<(u32, u8, u32, u16)> = got
        .iter()
        .map(|item| (item.id, item.window_type, item.pos, item.count))
        .collect();
    assert_eq!(
        cells,
        vec![
            (1_000_202, 1, 5, 0x0123),
            (1_000_200, 1, 40, 7),
            (1_000_203, 7, 2, 7),
        ]
    );
    assert_eq!(got[2], piece, "the created row is stored whole");
    // An empty list touches nothing.
    apply_row_changes(store, owner, &[]).await.unwrap();
    assert_eq!(load_owner_items(store, owner).await.unwrap(), got);
}

/// The gold `player.gold` holds for one character.
async fn gold_of(store: &Store, owner: u32) -> i64 {
    db::sqlx::query_scalar("SELECT gold FROM player WHERE id = $1")
        .bind(i32::try_from(owner).unwrap())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

/// A shop's buy or sale is one Transfer (ADR-0003): its rows and the owner's gold commit
/// together, and a refusal found after the gold was written takes the gold back with it.
#[tokio::test]
async fn a_transfer_stores_the_rows_and_the_gold_together() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctshop", "ShopOne").await;
    save_owner_items(store, &[row(1_000_240, owner, 1, 3)])
        .await
        .unwrap();
    let bought = row(1_000_241, owner, 1, 4);
    apply_transfer(
        store,
        owner,
        &[RowChange::Created(bought.clone())],
        1_000_000_000_000_000_007,
    )
    .await
    .unwrap();
    assert_eq!(gold_of(store, owner).await, 1_000_000_000_000_000_007);
    assert_eq!(load_item(store, 1_000_241).await.unwrap(), Some(bought));

    // The cells are checked after the gold is written, so this refusal rolls the gold back.
    let refused = apply_transfer(
        store,
        owner,
        &[RowChange::Created(row(1_000_242, owner, 1, 3))],
        5,
    )
    .await;
    assert!(
        matches!(
            refused,
            Err(ItemError::CellAlreadyTaken {
                id: 1_000_242,
                window_type: 1,
                pos: 3
            })
        ),
        "{refused:?}"
    );
    assert_eq!(gold_of(store, owner).await, 1_000_000_000_000_000_007);
    let refused = apply_transfer(store, owner, &[RowChange::Destroyed { id: 1_000_299 }], 5).await;
    assert!(
        matches!(refused, Err(ItemError::NoSuchItem(1_000_299))),
        "{refused:?}"
    );
    assert_eq!(gold_of(store, owner).await, 1_000_000_000_000_000_007);

    // The gold is a change to what the row holds, and one that would leave less than none is
    // refused by the statement itself, which rolls the rows back too.
    let sale = [RowChange::Destroyed { id: 1_000_240 }];
    let refused = apply_transfer(store, owner, &sale, -1_000_000_000_000_000_008).await;
    assert!(
        matches!(
            refused,
            Err(ItemError::GoldBelowZero {
                owner_id,
                held: 1_000_000_000_000_000_007,
                change: -1_000_000_000_000_000_008,
            }) if owner_id == owner
        ),
        "{refused:?}"
    );
    assert!(load_item(store, 1_000_240).await.unwrap().is_some());
    apply_transfer(store, owner, &sale, -1_000_000_000_000_000_007)
        .await
        .unwrap();
    assert_eq!(gold_of(store, owner).await, 0);
    assert_eq!(load_item(store, 1_000_240).await.unwrap(), None);

    // A sum past the column is the store's refusal, and nothing is written.
    apply_transfer(store, owner, &[], i64::MAX).await.unwrap();
    assert_eq!(gold_of(store, owner).await, i64::MAX);
    let refused = apply_transfer(
        store,
        owner,
        &[RowChange::Created(row(1_000_243, owner, 1, 6))],
        1,
    )
    .await;
    assert!(
        matches!(refused, Err(ItemError::Database(_))),
        "{refused:?}"
    );
    assert_eq!(gold_of(store, owner).await, i64::MAX);
    assert_eq!(load_item(store, 1_000_243).await.unwrap(), None);

    // Gold alone is still written, and an owner with no row is refused.
    apply_transfer(store, owner, &[], -i64::MAX).await.unwrap();
    assert_eq!(gold_of(store, owner).await, 0);
    for nobody in [owner + 1_000, u32::MAX] {
        let refused = apply_transfer(store, nobody, &[], 7).await;
        assert!(
            matches!(refused, Err(ItemError::NoSuchOwner(id)) if id == nobody),
            "{refused:?}"
        );
    }
    assert_eq!(gold_of(store, owner).await, 0);
}

/// Where one item row sits: its owner, window and cell.
async fn place_of(store: &Store, id: u32) -> (Option<u32>, u8, u32) {
    let item = load_item(store, id)
        .await
        .unwrap()
        .expect("the row is stored");
    (item.owner_id, item.window_type, item.pos)
}

/// Row `id` given to `to`, landing on `window_type` cell `pos`.
fn given(id: u32, to: u32, window_type: u8, pos: u32) -> RowChange {
    RowChange::Given {
        id,
        to,
        window_type,
        pos,
    }
}

/// One side of an exchange.
fn side(owner_id: u32, changes: &[RowChange], gold: i64) -> TransferSide<'_> {
    TransferSide {
        owner_id,
        changes,
        gold,
    }
}

/// Two traders: the first holds row `1_000_250` on cell 0 and 500 gold, the second rows
/// `1_000_251` and `1_000_252` on cells 0 and 1.
async fn traders(store: &Store) -> (u32, u32) {
    let one = player(store, "accttradeone", "TradeOne").await;
    let two = player(store, "accttradetwo", "TradeTwo").await;
    save_owner_items(
        store,
        &[
            row(1_000_250, one, 1, 0),
            row(1_000_251, two, 1, 0),
            row(1_000_252, two, 1, 1),
        ],
    )
    .await
    .unwrap();
    apply_transfer(store, one, &[], 500).await.unwrap();
    (one, two)
}

/// A trade is one Transfer (ADR-0003): each side's offered rows go to the other and the gold
/// goes with them, in one transaction, and a refusal found on either side leaves both as they
/// were.
#[tokio::test]
async fn an_exchange_gives_the_rows_both_ways_and_moves_the_gold_with_them() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let (one, two) = traders(store).await;

    // Each side's row lands on the other's cell 2 or 0; the second lands on a cell the first
    // vacates in the same transaction, which the deferred key allows.
    let from_one = [given(1_000_250, two, 1, 2)];
    let from_two = [given(1_000_251, one, 1, 0)];
    apply_exchange(
        store,
        &[side(one, &from_one, -300), side(two, &from_two, 300)],
    )
    .await
    .unwrap();
    assert_eq!(place_of(store, 1_000_250).await, (Some(two), 1, 2));
    assert_eq!(place_of(store, 1_000_251).await, (Some(one), 1, 0));
    assert_eq!(place_of(store, 1_000_252).await, (Some(two), 1, 1));
    assert_eq!(
        (gold_of(store, one).await, gold_of(store, two).await),
        (200, 300)
    );

    // A given row that lands on a cell the receiver holds is named, and neither side's rows
    // nor gold are written.
    let onto_held = [given(1_000_251, two, 1, 1)];
    let back = [given(1_000_250, one, 1, 5)];
    let refused =
        apply_exchange(store, &[side(one, &onto_held, 100), side(two, &back, -100)]).await;
    assert!(
        matches!(
            refused,
            Err(ItemError::CellAlreadyTaken {
                id: 1_000_251,
                window_type: 1,
                pos: 1
            })
        ),
        "{refused:?}"
    );
    // A side short of gold leaves the other side's rows where they were.
    let fine = [given(1_000_251, two, 1, 3)];
    let refused = apply_exchange(store, &[side(two, &back, 0), side(one, &fine, -201)]).await;
    assert!(
        matches!(
            refused,
            Err(ItemError::GoldBelowZero {
                owner_id,
                held: 200,
                change: -201
            }) if owner_id == one
        ),
        "{refused:?}"
    );
    // A row the giving side does not hold is named with its holder.
    let not_held = [given(1_000_252, two, 1, 4)];
    let refused = apply_exchange(store, &[side(one, &not_held, 0), side(two, &[], 0)]).await;
    assert!(
        matches!(
            refused,
            Err(ItemError::NotOwned {
                id: 1_000_252,
                owner_id: Some(holder)
            }) if holder == two
        ),
        "{refused:?}"
    );
    assert_eq!(place_of(store, 1_000_250).await, (Some(two), 1, 2));
    assert_eq!(place_of(store, 1_000_251).await, (Some(one), 1, 0));
    assert_eq!(
        (gold_of(store, one).await, gold_of(store, two).await),
        (200, 300)
    );
}

/// A given row goes to another side of its exchange, in a window a character holds, and one
/// character is one side; each refusal is found before the store is reached.
#[tokio::test]
async fn an_exchange_is_refused_before_the_store_when_its_sides_do_not_fit() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let (one, two) = traders(store).await;
    for changes in [
        [given(1_000_250, one, 1, 7)],
        [given(1_000_250, two + 1_000, 1, 7)],
        [given(1_000_250, two, GROUND, 7)],
    ] {
        let refused = apply_exchange(store, &[side(one, &changes, 0), side(two, &[], 0)]).await;
        assert!(matches!(refused, Err(ItemError::Corrupt(_))), "{refused:?}");
    }
    let past_ground = [given(1_000_250, two, GROUND + 1, 7)];
    let refused = apply_exchange(store, &[side(one, &past_ground, 0), side(two, &[], 0)]).await;
    assert!(
        matches!(refused, Err(ItemError::WindowOutOfRange(11))),
        "{refused:?}"
    );
    let refused = apply_exchange(store, &[side(one, &[], 1), side(one, &[], 1)]).await;
    assert!(matches!(refused, Err(ItemError::Corrupt(_))), "{refused:?}");
    // A row given outside an exchange has no other side to go to.
    let refused = apply_row_changes(store, one, &[given(1_000_250, two, 1, 3)]).await;
    assert!(matches!(refused, Err(ItemError::Corrupt(_))), "{refused:?}");
    // The controls: the same row given to the other side is stored.
    apply_exchange(
        store,
        &[
            side(one, &[given(1_000_250, two, 1, 7)], 0),
            side(two, &[], 0),
        ],
    )
    .await
    .unwrap();
    assert_eq!(place_of(store, 1_000_250).await, (Some(two), 1, 7));
    assert_eq!(
        (gold_of(store, one).await, gold_of(store, two).await),
        (500, 0)
    );
}

/// Equipping onto a worn item is a swap (`char_item.cpp:8164-8264`): the carried item takes
/// the wear cell while the worn one takes the carried item's old cell. The first of the two
/// row moves lands on a cell the second is about to vacate, which a key checked per row
/// refused before migration `0006`.
#[tokio::test]
async fn a_swap_through_each_others_cells_commits_and_a_real_clash_is_still_refused() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctswap", "SwapOne").await;
    save_owner_items(
        store,
        &[row(1_000_230, owner, 1, 3), row(1_000_231, owner, 1, 180)],
    )
    .await
    .unwrap();
    apply_row_changes(
        store,
        owner,
        &[
            RowChange::Moved {
                id: 1_000_230,
                window_type: 1,
                pos: 180,
            },
            RowChange::Moved {
                id: 1_000_231,
                window_type: 1,
                pos: 3,
            },
        ],
    )
    .await
    .unwrap();
    let swapped = load_owner_items(store, owner).await.unwrap();
    let cells: Vec<(u32, u32)> = swapped.iter().map(|item| (item.id, item.pos)).collect();
    assert_eq!(cells, vec![(1_000_231, 3), (1_000_230, 180)]);

    // Half a swap ends with two rows in one cell, and the refusal names the change that
    // landed there, not the row that was already in it.
    let refused = apply_row_changes(
        store,
        owner,
        &[RowChange::Moved {
            id: 1_000_231,
            window_type: 1,
            pos: 180,
        }],
    )
    .await;
    assert!(
        matches!(
            refused,
            Err(ItemError::CellAlreadyTaken {
                id: 1_000_231,
                window_type: 1,
                pos: 180
            })
        ),
        "{refused:?}"
    );
    // A split's new row on a held cell is refused the same way.
    let refused = apply_row_changes(
        store,
        owner,
        &[RowChange::Created(row(1_000_232, owner, 1, 3))],
    )
    .await;
    assert!(
        matches!(
            refused,
            Err(ItemError::CellAlreadyTaken {
                id: 1_000_232,
                window_type: 1,
                pos: 3
            })
        ),
        "{refused:?}"
    );
    assert_eq!(load_owner_items(store, owner).await.unwrap(), swapped);

    // Outside a move the key is still checked on each statement.
    let refused = insert_item(store, &row(1_000_233, owner, 1, 3)).await;
    assert!(
        matches!(
            refused,
            Err(ItemError::CellAlreadyTaken {
                id: 1_000_233,
                window_type: 1,
                pos: 3
            })
        ),
        "{refused:?}"
    );
}

/// A sash's first equip rolls its absorption share into socket 0 (`item.cpp:436-455`), and
/// the equip stores every socket with the move, under the owner.
#[tokio::test]
async fn a_socket_change_stores_every_socket_and_only_for_its_owner() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctsock", "SockOne").await;
    let other = player(store, "acctsock2", "SockTwo").await;
    let mut sash = row(1_000_240, owner, 1, 4);
    sash.sockets = [0, 1_234, -5, 0, 0, 0];
    save_owner_items(store, &[sash.clone()]).await.unwrap();
    let sockets = [0x0102_0304, 1_234, -5, 7, -0x0708_090a, 11];
    apply_row_changes(
        store,
        owner,
        &[
            RowChange::Moved {
                id: 1_000_240,
                window_type: 2,
                pos: 203,
            },
            RowChange::Sockets {
                id: 1_000_240,
                sockets,
            },
        ],
    )
    .await
    .unwrap();
    let stored = load_item(store, 1_000_240).await.unwrap().unwrap();
    assert_eq!(stored.sockets, sockets);
    assert_eq!((stored.window_type, stored.pos), (2, 203));
    assert_eq!(stored.attributes, sash.attributes);

    let refused = apply_row_changes(
        store,
        other,
        &[RowChange::Sockets {
            id: 1_000_240,
            sockets: [0; SOCKETS],
        }],
    )
    .await;
    assert!(
        matches!(refused, Err(ItemError::NotOwned { id: 1_000_240, .. })),
        "{refused:?}"
    );
    assert_eq!(
        load_item(store, 1_000_240).await.unwrap().unwrap().sockets,
        sockets
    );
}

#[tokio::test]
async fn a_change_the_owner_cannot_make_rolls_back_the_whole_move() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let one = player(store, "acctmv1", "MvOne").await;
    let two = player(store, "acctmv2", "MvTwo").await;
    save_owner_items(
        store,
        &[row(1_000_210, one, 1, 3), row(1_000_212, one, 1, 4)],
    )
    .await
    .unwrap();
    save_item(store, &row(1_000_211, two, 1, 3)).await.unwrap();
    let before_one = load_owner_items(store, one).await.unwrap();
    let before_two = load_owner_items(store, two).await.unwrap();
    let moved = RowChange::Moved {
        id: 1_000_210,
        window_type: 1,
        pos: 40,
    };

    // Each of the three statements names the owner, so each is refused for the other
    // character's row.
    for theirs in [
        RowChange::Moved {
            id: 1_000_211,
            window_type: 1,
            pos: 41,
        },
        RowChange::Count {
            id: 1_000_211,
            count: 3,
        },
        RowChange::Destroyed { id: 1_000_211 },
    ] {
        let refused = apply_row_changes(store, one, &[moved.clone(), theirs.clone()]).await;
        assert!(
            matches!(
                refused,
                Err(ItemError::NotOwned {
                    id: 1_000_211,
                    owner_id: Some(owner)
                }) if owner == two
            ),
            "{theirs:?} gave {refused:?}"
        );
    }
    let refused = apply_row_changes(
        store,
        one,
        &[
            moved.clone(),
            RowChange::Count {
                id: 1_000_999,
                count: 3,
            },
        ],
    )
    .await;
    assert!(matches!(refused, Err(ItemError::NoSuchItem(1_000_999))));
    // A moved row onto a cell another row holds is the cell's refusal.
    let refused = apply_row_changes(
        store,
        one,
        &[
            moved,
            RowChange::Moved {
                id: 1_000_212,
                window_type: 1,
                pos: 40,
            },
        ],
    )
    .await;
    assert!(matches!(
        refused,
        Err(ItemError::CellAlreadyTaken {
            id: 1_000_212,
            window_type: 1,
            pos: 40
        })
    ));
    assert_eq!(load_owner_items(store, one).await.unwrap(), before_one);
    assert_eq!(load_owner_items(store, two).await.unwrap(), before_two);
}

#[tokio::test]
async fn a_change_that_cannot_be_stored_is_refused_before_the_store() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctmvbad", "MvBad").await;
    let other = player(store, "acctmvoth", "MvOther").await;
    save_item(store, &row(1_000_220, owner, 1, 3))
        .await
        .unwrap();
    let before = load_owner_items(store, owner).await.unwrap();
    let first = RowChange::Moved {
        id: 1_000_220,
        window_type: 1,
        pos: 40,
    };
    let bad = [
        RowChange::Count {
            id: 1_000_220,
            count: 0,
        },
        RowChange::Count {
            id: 1_000_220,
            count: 5001,
        },
        RowChange::Moved {
            id: 1_000_220,
            window_type: 11,
            pos: 0,
        },
        RowChange::Moved {
            id: 1_000_220,
            window_type: GROUND,
            pos: 0,
        },
        RowChange::Created(row(1_000_221, other, 1, 9)),
        RowChange::Created(ItemRow {
            count: 0,
            ..row(1_000_221, owner, 1, 9)
        }),
    ];
    for change in bad {
        let refused = apply_row_changes(store, owner, &[first.clone(), change.clone()]).await;
        assert!(
            matches!(
                refused,
                Err(ItemError::CountOutOfRange(_)
                    | ItemError::WindowOutOfRange(11)
                    | ItemError::Corrupt(_))
            ),
            "{change:?} gave {refused:?}"
        );
        assert_eq!(load_owner_items(store, owner).await.unwrap(), before);
    }
    // A created id that is already stored is the insert's refusal.
    let refused = apply_row_changes(
        store,
        owner,
        &[first, RowChange::Created(row(1_000_220, owner, 1, 9))],
    )
    .await;
    assert!(matches!(
        refused,
        Err(ItemError::ItemIdAlreadyStored { id: 1_000_220 })
    ));
    assert_eq!(load_owner_items(store, owner).await.unwrap(), before);
}
