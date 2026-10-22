//! The quest flag store, against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.
//!
//! The store's rules are that a save replaces every flag of the player at once, that a
//! flag whose value is zero is no flag and is not written, and that the flags of one
//! character never touch another's. All three are the database's answers.

mod support;

use db::accounts::create_account;
use db::credentials::{DeleteCode, Login, NewPassword};
use db::players::{create_player, Created, NewPlayer};
use db::quest_flags::{load_flags, save_flags, StoredQuestFlag};
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

fn flag(name: &[u8], value: i32) -> StoredQuestFlag {
    StoredQuestFlag {
        name: name.to_vec(),
        value,
    }
}

#[tokio::test]
async fn a_save_replaces_every_flag_and_the_load_reads_them_in_name_order() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctqf", "QfOne").await;
    let other = player(store, "acctqf2", "QfTwo").await;
    assert!(load_flags(store, owner).await.unwrap().is_empty());

    save_flags(
        store,
        owner,
        &[
            flag(b"war.is_war_member", 1),
            flag(b"guild_manage.new_withdraw_time", 1_700_000_000),
        ],
    )
    .await
    .unwrap();
    save_flags(store, other, &[flag(b"war.is_war_member", 1)])
        .await
        .unwrap();
    assert_eq!(
        load_flags(store, owner).await.unwrap(),
        vec![
            flag(b"guild_manage.new_withdraw_time", 1_700_000_000),
            flag(b"war.is_war_member", 1),
        ],
        "the flags come back in name order"
    );

    // The second save is the whole set: the flags it leaves out are gone.
    save_flags(store, owner, &[flag(b"war.is_war_member", 1)])
        .await
        .unwrap();
    assert_eq!(
        load_flags(store, owner).await.unwrap(),
        vec![flag(b"war.is_war_member", 1)]
    );

    // Another character's flags are untouched.
    assert_eq!(
        load_flags(store, other).await.unwrap(),
        vec![flag(b"war.is_war_member", 1)]
    );
}

#[tokio::test]
async fn a_flag_whose_value_is_zero_is_no_flag() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctqz", "QzOne").await;
    save_flags(
        store,
        owner,
        &[flag(b"war.is_war_member", 0), flag(b"other.flag", 3)],
    )
    .await
    .unwrap();
    assert_eq!(
        load_flags(store, owner).await.unwrap(),
        vec![flag(b"other.flag", 3)],
        "the zero flag is not written"
    );
    // A flag the migration refuses (a zero value) leaves the old rows in place.
    save_flags(store, owner, &[flag(b"other.flag", 0)])
        .await
        .unwrap();
    assert!(load_flags(store, owner).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_flag_name_is_stored_as_its_exact_bytes() {
    let Some(db) = ScratchDatabase::create().await else {
        return;
    };
    let store = &db.store;
    let owner = player(store, "acctqb", "QbOne").await;
    // A name that is not text: the flag is free text, so it is stored byte for byte.
    let name = b"war.is_war_member\xff".to_vec();
    save_flags(store, owner, &[flag(&name, 5)]).await.unwrap();
    assert_eq!(
        load_flags(store, owner).await.unwrap(),
        vec![flag(&name, 5)]
    );
}
