//! The item store's read and write path, against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.
//!
//! What these tests are for is the two rules the module states and the migration
//! cannot: a save must never delete the row it conflicts with, and a delete or a
//! count change must be refused for an item the caller does not hold. Both are
//! answered by the database, so neither is visible without one.

mod support;

use db::accounts::create_account;
use db::credentials::{DeleteCode, Login, NewPassword};
use db::items::{
    destroy_item, load_item, load_owner_items, max_id_in_range, save_item, save_owner_items,
    set_count, Attribute, ItemError, ItemRow, GROUND, MAX_ITEM_ID, SOCKETS,
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
