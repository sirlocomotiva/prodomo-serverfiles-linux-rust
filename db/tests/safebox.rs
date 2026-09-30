//! The safebox and the mall, held by the account (ADR-0005), against a real PostgreSQL 18
//! server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.
//!
//! What these tests are for is the holder rule (a row belongs to a character, an account, or
//! nobody, and its window says which), the account's cell key, the checkin and checkout
//! Transfers, and the password, which only the store can answer.

mod support;

use db::accounts::{create_account, AccountId, Name, NewAccount};
use db::credentials::{DeleteCode, Login, NewPassword};
use db::items::{
    apply_account_changes, apply_row_changes, insert_item, load_account_items, load_item,
    load_owner_items, save_item, AccountChange, ItemError, ItemRow, RowChange, GROUND, MALL,
    SAFEBOX,
};
use db::players::{create_player, Created, NewPlayer};
use db::safebox::{change_password, verify_password, SafeboxError, SafeboxPassword};
use db::store::Store;
use support::ScratchDatabase;

async fn account(store: &Store, login: &str) -> u32 {
    let account = NewAccount {
        login: Login::new(login).expect("a valid login"),
        password: NewPassword::new(b"pw").unwrap().hash().unwrap(),
        delete_code: DeleteCode::random(),
    };
    create_account(store, &account).await.unwrap().get()
}

async fn character(store: &Store, account: u32, slot: u8, name: &str) -> u32 {
    match create_player(
        store,
        AccountId::new(account),
        &NewPlayer {
            slot,
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

/// A row of character `owner` at `window_type` cell `pos`.
fn held(id: u32, owner: u32, window_type: u8, pos: u32) -> ItemRow {
    ItemRow {
        owner_id: Some(owner),
        window_type,
        pos,
        ..ItemRow::on_ground(id, 0, 27_001, 3)
    }
}

/// A row of account `account` at `window_type` cell `pos`.
fn kept(id: u32, account: u32, window_type: u8, pos: u32) -> ItemRow {
    ItemRow {
        account_id: Some(account),
        window_type,
        pos,
        ..ItemRow::on_ground(id, 0, 27_001, 3)
    }
}

/// Where row `id` is: its character, its account, its window and its cell.
async fn place_of(store: &Store, id: u32) -> (Option<u32>, Option<u32>, u8, u32) {
    let item = load_item(store, id)
        .await
        .unwrap()
        .expect("the row is stored");
    (item.owner_id, item.account_id, item.window_type, item.pos)
}

fn password(raw: &[u8]) -> SafeboxPassword {
    SafeboxPassword::new(raw).expect("1 to 6 bytes")
}

/// The SQLSTATE and the constraint a refused statement names.
fn refusal(error: &db::sqlx::Error) -> (Option<String>, Option<String>) {
    match error {
        db::sqlx::Error::Database(database) => (
            database.code().map(std::borrow::Cow::into_owned),
            database.constraint().map(str::to_owned),
        ),
        other => panic!("expected a database refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn the_holder_follows_the_window() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let account = account(store, "holderrule").await;
    let owner = character(store, account, 0, "Holder").await;

    // The three shapes the rule allows.
    save_item(store, &kept(1_000_300, account, SAFEBOX, 0))
        .await
        .unwrap();
    save_item(store, &kept(1_000_301, account, MALL, 0))
        .await
        .unwrap();
    save_item(store, &held(1_000_302, owner, 1, 0))
        .await
        .unwrap();
    save_item(store, &ItemRow::on_ground(1_000_303, 41, 27_001, 1))
        .await
        .unwrap();

    // The four it refuses, before the store is reached.
    let both = ItemRow {
        owner_id: Some(owner),
        ..kept(1_000_304, account, SAFEBOX, 1)
    };
    let refused = [
        held(1_000_305, owner, SAFEBOX, 2),
        held(1_000_306, owner, MALL, 2),
        kept(1_000_307, account, 1, 2),
        kept(1_000_308, account, GROUND, 2),
        both,
    ];
    for row in &refused {
        match save_item(store, row).await {
            Err(ItemError::Corrupt(detail)) => {
                assert!(detail.contains("held by"), "{detail}");
            }
            other => panic!("{row:?} gave {other:?}"),
        }
    }
    let rows: i64 = db::sqlx::query_scalar("SELECT COUNT(*) FROM item")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(rows, 4, "a refused save wrote a row");

    // The CHECK holds the same rule for a statement that skips the crate.
    let owner_column = i32::try_from(owner).unwrap();
    let account_column = i32::try_from(account).unwrap();
    let statements = [
        (
            "UPDATE item SET owner_id = $1 WHERE id = 1000300",
            owner_column,
        ),
        (
            "UPDATE item SET account_id = $1 WHERE id = 1000302",
            account_column,
        ),
        (
            "UPDATE item SET account_id = $1 WHERE id = 1000303",
            account_column,
        ),
        (
            "UPDATE item SET window_type = 1 WHERE id = 1000300 AND $1 > 0",
            1,
        ),
        (
            "UPDATE item SET window_type = 10 WHERE id = 1000300 AND $1 > 0",
            1,
        ),
        (
            "UPDATE item SET window_type = 3 WHERE id = 1000302 AND $1 > 0",
            1,
        ),
        (
            "UPDATE item SET window_type = 4 WHERE id = 1000303 AND $1 > 0",
            1,
        ),
    ];
    for (statement, value) in statements {
        let error = db::sqlx::query(statement)
            .bind(value)
            .execute(store.pool())
            .await
            .expect_err(statement);
        assert_eq!(
            refusal(&error),
            (
                Some("23514".to_owned()),
                Some("item_holder_check".to_owned())
            ),
            "{statement}"
        );
    }
}

#[tokio::test]
async fn one_item_per_safebox_cell() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let one = account(store, "cellone").await;
    let two = account(store, "celltwo").await;
    insert_item(store, &kept(1_000_310, one, SAFEBOX, 4))
        .await
        .unwrap();

    match insert_item(store, &kept(1_000_311, one, SAFEBOX, 4)).await {
        Err(ItemError::CellAlreadyTaken {
            id: 1_000_311,
            window_type: SAFEBOX,
            pos: 4,
        }) => {}
        other => panic!("expected CellAlreadyTaken, got {other:?}"),
    }
    // The same cell of the other window, and of another account, is a different cell.
    insert_item(store, &kept(1_000_312, one, MALL, 4))
        .await
        .unwrap();
    insert_item(store, &kept(1_000_313, two, SAFEBOX, 4))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_account_window_loads_alone_and_in_cell_order() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let one = account(store, "loadone").await;
    let two = account(store, "loadtwo").await;
    let owner = character(store, one, 0, "Loader").await;
    for row in [
        kept(1_000_320, one, SAFEBOX, 5),
        kept(1_000_321, one, SAFEBOX, 1),
        kept(1_000_322, one, MALL, 0),
        kept(1_000_323, two, SAFEBOX, 1),
        held(1_000_324, owner, 1, 1),
    ] {
        save_item(store, &row).await.unwrap();
    }

    let ids = |rows: Vec<ItemRow>| rows.iter().map(|row| row.id).collect::<Vec<_>>();
    assert_eq!(
        ids(load_account_items(store, one, SAFEBOX).await.unwrap()),
        [1_000_321, 1_000_320]
    );
    assert_eq!(
        ids(load_account_items(store, one, MALL).await.unwrap()),
        [1_000_322]
    );
    assert_eq!(
        ids(load_account_items(store, two, SAFEBOX).await.unwrap()),
        [1_000_323]
    );
    assert_eq!(
        ids(load_account_items(store, two, MALL).await.unwrap()),
        Vec::<u32>::new()
    );
    // The character's load never sees the account's rows.
    assert_eq!(
        ids(load_owner_items(store, owner).await.unwrap()),
        [1_000_324]
    );
    let loaded = load_account_items(store, one, MALL).await.unwrap();
    assert_eq!(
        loaded,
        [kept(1_000_322, one, MALL, 0)],
        "every column reads back"
    );
}

#[tokio::test]
async fn an_account_without_a_row_opens_with_six_zeroes() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let id = AccountId::new(account(store, "nopassword").await);
    assert!(verify_password(store, id, &password(b"000000"))
        .await
        .unwrap());
    for wrong in [&b"00000"[..], &b"000001"[..], &b"123456"[..], &b"0"[..]] {
        assert!(
            !verify_password(store, id, &password(wrong)).await.unwrap(),
            "{wrong:?}"
        );
    }
}

#[tokio::test]
async fn a_change_without_a_row_needs_the_default_and_creates_the_row() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let id = AccountId::new(account(store, "firstchange").await);
    let rows = || async {
        db::sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM safebox")
            .fetch_one(store.pool())
            .await
            .unwrap()
    };

    assert!(
        !change_password(store, id, &password(b"123456"), &password(b"abc"))
            .await
            .unwrap()
    );
    assert_eq!(rows().await, 0, "a wrong old password created the row");

    assert!(
        change_password(store, id, &password(b"000000"), &password(b"abc"))
            .await
            .unwrap()
    );
    assert_eq!(rows().await, 1);
    let stored: String = db::sqlx::query_scalar("SELECT password_hash FROM safebox")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(stored.starts_with("$argon2id$"), "{stored}");
    assert!(
        !stored.contains("abc"),
        "the password is stored in the clear"
    );
    assert!(verify_password(store, id, &password(b"abc")).await.unwrap());
    assert!(!verify_password(store, id, &password(b"000000"))
        .await
        .unwrap());

    // The default no longer opens it, so it no longer changes it either.
    assert!(
        !change_password(store, id, &password(b"000000"), &password(b"x"))
            .await
            .unwrap()
    );
    assert!(verify_password(store, id, &password(b"abc")).await.unwrap());
}

#[tokio::test]
async fn a_change_needs_the_exact_old_password() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let id = AccountId::new(account(store, "exactchange").await);
    let other = AccountId::new(account(store, "otherchange").await);
    assert!(
        change_password(store, id, &password(b"000000"), &password(b"AbC"))
            .await
            .unwrap()
    );
    // Legacy compares without case here (a Divergence, ADR-0005).
    assert!(
        !change_password(store, id, &password(b"abc"), &password(b"zz"))
            .await
            .unwrap()
    );
    assert!(
        change_password(store, id, &password(b"AbC"), &password(b"zz"))
            .await
            .unwrap()
    );
    assert!(verify_password(store, id, &password(b"zz")).await.unwrap());
    assert!(!verify_password(store, id, &password(b"AbC")).await.unwrap());
    // Another account's password is its own.
    assert!(verify_password(store, other, &password(b"000000"))
        .await
        .unwrap());
}

#[tokio::test]
async fn a_change_for_no_account_is_refused() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    for missing in [AccountId::new(999_999), AccountId::new(u32::MAX)] {
        match change_password(store, missing, &password(b"000000"), &password(b"abc")).await {
            Err(SafeboxError::NoSuchAccount(id)) => assert_eq!(id, missing),
            other => panic!("expected NoSuchAccount, got {other:?}"),
        }
    }
    // A plain password is refused by the CHECK, whoever writes it.
    let holder = account(store, "plainstored").await;
    let error = db::sqlx::query("INSERT INTO safebox (account_id, password_hash) VALUES ($1, $2)")
        .bind(i32::try_from(holder).unwrap())
        .bind("000000")
        .execute(store.pool())
        .await
        .expect_err("a plain password");
    assert_eq!(
        refusal(&error),
        (
            Some("23514".to_owned()),
            Some("safebox_password_hash_check".to_owned())
        )
    );
}

#[tokio::test]
async fn a_checkin_hands_the_row_to_the_account() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    // A first account keeps the account's ID apart from its character's.
    let _spare = account(store, "spare").await;
    let account = account(store, "checkin").await;
    let owner = character(store, account, 0, "Checkin").await;
    assert_ne!(account, owner, "the row must be told which one holds it");
    save_item(store, &held(1_000_330, owner, 1, 3))
        .await
        .unwrap();

    let stored = RowChange::Stored {
        id: 1_000_330,
        account,
        pos: 7,
    };
    apply_row_changes(store, owner, &[stored]).await.unwrap();
    assert_eq!(
        place_of(store, 1_000_330).await,
        (None, Some(account), SAFEBOX, 7)
    );
    assert!(load_owner_items(store, owner).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_checkin_to_another_account_writes_nothing() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let mine = account(store, "checkinmine").await;
    let theirs = account(store, "checkintheirs").await;
    let owner = character(store, mine, 0, "Mine").await;
    save_item(store, &held(1_000_340, owner, 1, 0))
        .await
        .unwrap();
    save_item(store, &held(1_000_341, owner, 1, 1))
        .await
        .unwrap();

    let changes = [
        RowChange::Moved {
            id: 1_000_341,
            window_type: 1,
            pos: 9,
        },
        RowChange::Stored {
            id: 1_000_340,
            account: theirs,
            pos: 0,
        },
    ];
    match apply_row_changes(store, owner, &changes).await {
        Err(ItemError::ForeignAccount { owner_id, account }) => {
            assert_eq!((owner_id, account), (owner, theirs));
        }
        other => panic!("expected ForeignAccount, got {other:?}"),
    }
    assert_eq!(place_of(store, 1_000_340).await, (Some(owner), None, 1, 0));
    assert_eq!(place_of(store, 1_000_341).await, (Some(owner), None, 1, 1));

    // A checkout that names another account is refused the same way.
    save_item(store, &kept(1_000_342, theirs, SAFEBOX, 0))
        .await
        .unwrap();
    let retrieved = RowChange::Retrieved {
        id: 1_000_342,
        account: theirs,
        window_type: 1,
        pos: 5,
    };
    assert!(matches!(
        apply_row_changes(store, owner, &[retrieved]).await,
        Err(ItemError::ForeignAccount { .. })
    ));
    assert_eq!(
        place_of(store, 1_000_342).await,
        (None, Some(theirs), SAFEBOX, 0)
    );
}

#[tokio::test]
async fn a_checkin_onto_a_taken_cell_names_the_change() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let account = account(store, "checkintaken").await;
    let owner = character(store, account, 0, "Taken").await;
    save_item(store, &kept(1_000_350, account, SAFEBOX, 2))
        .await
        .unwrap();
    save_item(store, &held(1_000_351, owner, 1, 0))
        .await
        .unwrap();
    save_item(store, &held(1_000_352, owner, 1, 1))
        .await
        .unwrap();

    let changes = [
        RowChange::Stored {
            id: 1_000_352,
            account,
            pos: 6,
        },
        RowChange::Stored {
            id: 1_000_351,
            account,
            pos: 2,
        },
    ];
    match apply_row_changes(store, owner, &changes).await {
        Err(ItemError::CellAlreadyTaken {
            id: 1_000_351,
            window_type: SAFEBOX,
            pos: 2,
        }) => {}
        other => panic!("expected CellAlreadyTaken, got {other:?}"),
    }
    assert_eq!(place_of(store, 1_000_351).await, (Some(owner), None, 1, 0));
    assert_eq!(place_of(store, 1_000_352).await, (Some(owner), None, 1, 1));

    // A checkin of an item the character does not hold touches no row.
    let stored = RowChange::Stored {
        id: 1_000_350,
        account,
        pos: 8,
    };
    match apply_row_changes(store, owner, &[stored]).await {
        Err(ItemError::NotOwned {
            id: 1_000_350,
            owner_id: None,
            account_id: Some(holder),
        }) => assert_eq!(holder, account),
        other => panic!("expected NotOwned, got {other:?}"),
    }
}

#[tokio::test]
async fn a_checkout_hands_the_row_to_any_character_of_the_account() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let account = account(store, "checkout").await;
    let first = character(store, account, 0, "First").await;
    let second = character(store, account, 1, "Second").await;
    save_item(store, &held(1_000_360, first, 1, 0))
        .await
        .unwrap();
    save_item(store, &kept(1_000_361, account, MALL, 3))
        .await
        .unwrap();

    let stored = RowChange::Stored {
        id: 1_000_360,
        account,
        pos: 0,
    };
    apply_row_changes(store, first, &[stored]).await.unwrap();
    let changes = [
        RowChange::Retrieved {
            id: 1_000_360,
            account,
            window_type: 1,
            pos: 4,
        },
        RowChange::Retrieved {
            id: 1_000_361,
            account,
            window_type: 1,
            pos: 5,
        },
    ];
    apply_row_changes(store, second, &changes).await.unwrap();
    assert_eq!(place_of(store, 1_000_360).await, (Some(second), None, 1, 4));
    assert_eq!(place_of(store, 1_000_361).await, (Some(second), None, 1, 5));
    assert!(load_account_items(store, account, SAFEBOX)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_checkout_is_refused_for_a_row_the_account_does_not_hold() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let mine = account(store, "checkoutmine").await;
    let theirs = account(store, "checkouttheirs").await;
    let owner = character(store, mine, 0, "Outer").await;
    save_item(store, &kept(1_000_370, theirs, SAFEBOX, 0))
        .await
        .unwrap();
    save_item(store, &kept(1_000_371, mine, SAFEBOX, 1))
        .await
        .unwrap();
    save_item(store, &held(1_000_372, owner, 1, 4))
        .await
        .unwrap();

    let theirs_out = RowChange::Retrieved {
        id: 1_000_370,
        account: mine,
        window_type: 1,
        pos: 0,
    };
    match apply_row_changes(store, owner, &[theirs_out]).await {
        Err(ItemError::NotOwned {
            id: 1_000_370,
            owner_id: None,
            account_id: Some(holder),
        }) => assert_eq!(holder, theirs),
        other => panic!("expected NotOwned, got {other:?}"),
    }

    // Onto a taken cell: the change that landed there is named, and nothing moves.
    let onto_taken = RowChange::Retrieved {
        id: 1_000_371,
        account: mine,
        window_type: 1,
        pos: 4,
    };
    match apply_row_changes(store, owner, &[onto_taken]).await {
        Err(ItemError::CellAlreadyTaken {
            id: 1_000_371,
            window_type: 1,
            pos: 4,
        }) => {}
        other => panic!("expected CellAlreadyTaken, got {other:?}"),
    }
    assert_eq!(
        place_of(store, 1_000_371).await,
        (None, Some(mine), SAFEBOX, 1)
    );

    // Into an account window, or past the ground, before the store is reached.
    for window_type in [SAFEBOX, MALL, GROUND] {
        let change = RowChange::Retrieved {
            id: 1_000_371,
            account: mine,
            window_type,
            pos: 0,
        };
        assert!(
            matches!(
                apply_row_changes(store, owner, &[change]).await,
                Err(ItemError::Corrupt(_))
            ),
            "{window_type}"
        );
    }
    let change = RowChange::Retrieved {
        id: 1_000_371,
        account: mine,
        window_type: GROUND + 1,
        pos: 0,
    };
    assert!(matches!(
        apply_row_changes(store, owner, &[change]).await,
        Err(ItemError::WindowOutOfRange(11))
    ));
    // A character's own move into an account window is refused the same way.
    let moved = RowChange::Moved {
        id: 1_000_372,
        window_type: SAFEBOX,
        pos: 9,
    };
    assert!(matches!(
        apply_row_changes(store, owner, &[moved]).await,
        Err(ItemError::Corrupt(_))
    ));
    assert_eq!(place_of(store, 1_000_372).await, (Some(owner), None, 1, 4));
}

#[tokio::test]
async fn a_move_inside_the_safebox_is_one_transaction_under_the_account() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let (account, other) = safebox_rows(store).await;

    // A swap: the first move lands on the cell the second one vacates.
    let swap = [
        AccountChange::Moved {
            id: 1_000_380,
            window_type: SAFEBOX,
            pos: 1,
        },
        AccountChange::Moved {
            id: 1_000_381,
            window_type: SAFEBOX,
            pos: 0,
        },
    ];
    apply_account_changes(store, account, &swap).await.unwrap();
    assert_eq!(
        place_of(store, 1_000_380).await,
        (None, Some(account), SAFEBOX, 1)
    );
    assert_eq!(
        place_of(store, 1_000_381).await,
        (None, Some(account), SAFEBOX, 0)
    );

    // A whole merge: the source is used up and the stack grows.
    let merge = [
        AccountChange::Count {
            id: 1_000_381,
            count: 6,
        },
        AccountChange::Destroyed { id: 1_000_382 },
    ];
    apply_account_changes(store, account, &merge).await.unwrap();
    assert_eq!(load_item(store, 1_000_381).await.unwrap().unwrap().count, 6);
    assert_eq!(load_item(store, 1_000_382).await.unwrap(), None);

    // Onto a taken cell: named, and nothing moves.
    let onto_taken = [AccountChange::Moved {
        id: 1_000_381,
        window_type: SAFEBOX,
        pos: 1,
    }];
    match apply_account_changes(store, account, &onto_taken).await {
        Err(ItemError::CellAlreadyTaken {
            id: 1_000_381,
            window_type: SAFEBOX,
            pos: 1,
        }) => {}
        other => panic!("expected CellAlreadyTaken, got {other:?}"),
    }
    assert_eq!(
        place_of(store, 1_000_381).await,
        (None, Some(account), SAFEBOX, 0)
    );

    // Another account's row, and a statement that touched nothing rolls back the first.
    let foreign = [
        AccountChange::Moved {
            id: 1_000_381,
            window_type: SAFEBOX,
            pos: 20,
        },
        AccountChange::Destroyed { id: 1_000_383 },
    ];
    match apply_account_changes(store, account, &foreign).await {
        Err(ItemError::NotOwned {
            id: 1_000_383,
            owner_id: None,
            account_id: Some(holder),
        }) => assert_eq!(holder, other),
        other => panic!("expected NotOwned, got {other:?}"),
    }
    assert_eq!(
        place_of(store, 1_000_381).await,
        (None, Some(account), SAFEBOX, 0)
    );
    assert!(load_item(store, 1_000_383).await.unwrap().is_some());
}

#[tokio::test]
async fn a_safebox_move_is_checked_before_the_store() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let (account, _) = safebox_rows(store).await;
    // Refused before the store is reached.
    let before_the_store = [
        (
            AccountChange::Moved {
                id: 1_000_381,
                window_type: 1,
                pos: 0,
            },
            "Corrupt",
        ),
        (
            AccountChange::Moved {
                id: 1_000_381,
                window_type: GROUND,
                pos: 0,
            },
            "Corrupt",
        ),
        (
            AccountChange::Moved {
                id: 1_000_381,
                window_type: GROUND + 1,
                pos: 0,
            },
            "WindowOutOfRange",
        ),
        (
            AccountChange::Count {
                id: 1_000_381,
                count: 0,
            },
            "CountOutOfRange",
        ),
        (
            AccountChange::Count {
                id: 1_000_381,
                count: 5_001,
            },
            "CountOutOfRange",
        ),
    ];
    for (change, expected) in before_the_store {
        let refused = apply_account_changes(store, account, &[change]).await;
        let name = match refused {
            Err(ItemError::Corrupt(_)) => "Corrupt",
            Err(ItemError::WindowOutOfRange(_)) => "WindowOutOfRange",
            Err(ItemError::CountOutOfRange(_)) => "CountOutOfRange",
            other => panic!("{change:?} gave {other:?}"),
        };
        assert_eq!(name, expected, "{change:?}");
    }
    assert_eq!(load_item(store, 1_000_381).await.unwrap().unwrap().count, 3);

    // A move to the mall stays with the account.
    let to_mall = [AccountChange::Moved {
        id: 1_000_381,
        window_type: MALL,
        pos: 0,
    }];
    apply_account_changes(store, account, &to_mall)
        .await
        .unwrap();
    assert_eq!(
        place_of(store, 1_000_381).await,
        (None, Some(account), MALL, 0)
    );
    apply_account_changes(store, account, &[]).await.unwrap();
}

/// Two accounts: the first holds rows `1_000_380` to `1_000_382` on safebox cells 0 to 2, the
/// second row `1_000_383` on cell 0.
async fn safebox_rows(store: &Store) -> (u32, u32) {
    let mine = account(store, "safeboxmove").await;
    let other = account(store, "safeboxother").await;
    for row in [
        kept(1_000_380, mine, SAFEBOX, 0),
        kept(1_000_381, mine, SAFEBOX, 1),
        kept(1_000_382, mine, SAFEBOX, 2),
        kept(1_000_383, other, SAFEBOX, 0),
    ] {
        save_item(store, &row).await.unwrap();
    }
    (mine, other)
}

#[tokio::test]
async fn deleting_the_account_deletes_its_safebox() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let gone = account(store, "cascadegone").await;
    let kept_account = account(store, "cascadekept").await;
    save_item(store, &kept(1_000_390, gone, SAFEBOX, 0))
        .await
        .unwrap();
    save_item(store, &kept(1_000_391, kept_account, SAFEBOX, 0))
        .await
        .unwrap();
    assert!(change_password(
        store,
        AccountId::new(gone),
        &password(b"000000"),
        &password(b"abc")
    )
    .await
    .unwrap());

    db::sqlx::query("DELETE FROM account WHERE id = $1")
        .bind(i32::try_from(gone).unwrap())
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(load_item(store, 1_000_390).await.unwrap(), None);
    assert!(load_item(store, 1_000_391).await.unwrap().is_some());
    let rows: i64 = db::sqlx::query_scalar("SELECT COUNT(*) FROM safebox")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0);
}
