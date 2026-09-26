//! The Channel login's read of an account's empire and characters, against a real PostgreSQL
//! 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.

mod support;

use db::accounts::{create_account, AccountError, AccountId, NewAccount};
use db::credentials::{DeleteCode, Login, NewPassword};
use db::players::{lobby, LobbyPlayer};
use db::sqlx;
use db::store::Store;
use support::ScratchDatabase;

async fn account(store: &Store, raw_login: &str) -> AccountId {
    let account = NewAccount {
        login: Login::new(raw_login).expect("a valid login"),
        password: NewPassword::new(b"pw").unwrap().hash().unwrap(),
        delete_code: DeleteCode::random(),
    };
    create_account(store, &account).await.unwrap()
}

async fn insert(store: &Store, account: AccountId, slot: i16, name: &str) -> sqlx::Result<i32> {
    sqlx::query_scalar(
        "INSERT INTO player (account_id, slot, name, job, level, playtime_minutes, st, ht, dx, \
         iq, conqueror_level, sungma_str, sungma_hp, sungma_move, sungma_immune, part_main, \
         part_hair, part_sash, x, y, skill_group, change_name) VALUES ($1, $2, $3, 3, 0x9a, \
         0x0102_0304, 0x11, 0x12, 0x13, 0x14, 0x21, 0x22, 0x23, 0x24, 0x25, 0xa1b2, 0xc3d4, \
         0xe5f6, -469300, 964200, 0x31, true) RETURNING id",
    )
    .bind(i32::try_from(account.get()).unwrap())
    .bind(slot)
    .bind(name)
    .fetch_one(store.pool())
    .await
}

#[tokio::test]
async fn a_new_account_has_no_empire_and_no_characters() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let id = account(&scratch.store, "alice").await;
    let found = lobby(&scratch.store, id).await.unwrap();
    assert_eq!(found.empire, 0);
    assert!(found.players.is_empty());
}

#[tokio::test]
async fn the_lobby_lists_only_the_accounts_characters_in_slot_order() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = account(store, "alice").await;
    let bob = account(store, "bob").await;
    sqlx::query("UPDATE account SET empire = 2 WHERE id = $1")
        .bind(i32::try_from(alice.get()).unwrap())
        .execute(store.pool())
        .await
        .unwrap();
    let third = insert(store, alice, 3, "Third").await.unwrap();
    let first = insert(store, alice, 0, "First").await.unwrap();
    insert(store, bob, 0, "Other").await.unwrap();

    let found = lobby(store, alice).await.unwrap();
    assert_eq!(found.empire, 2);
    let slots: Vec<_> = found
        .players
        .iter()
        .map(|p| (p.slot, p.name.as_str()))
        .collect();
    assert_eq!(slots, [(0, "First"), (3, "Third")]);
    assert_eq!(found.in_slot(3).map(|p| p.id), u32::try_from(third).ok());
    assert!(found.in_slot(1).is_none());
    assert_eq!(
        found.in_slot(0),
        Some(&LobbyPlayer {
            slot: 0,
            id: u32::try_from(first).unwrap(),
            name: "First".to_owned(),
            job: 3,
            level: 0x9a,
            play_minutes: 0x0102_0304,
            st: 0x11,
            ht: 0x12,
            dx: 0x13,
            iq: 0x14,
            conqueror_level: 0x21,
            sungma_str: 0x22,
            sungma_hp: 0x23,
            sungma_move: 0x24,
            sungma_immune: 0x25,
            main_part: 0xa1b2,
            hair_part: 0xc3d4,
            sash_part: 0xe5f6,
            x: -469_300,
            y: 964_200,
            skill_group: 0x31,
            change_name: true,
        })
    );
}

#[tokio::test]
async fn the_schema_refuses_a_fifth_slot_a_taken_slot_and_a_taken_name() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = account(store, "alice").await;
    let bob = account(store, "bob").await;
    insert(store, alice, 0, "Hero").await.unwrap();
    assert!(insert(store, alice, 4, "Fifth").await.is_err());
    assert!(insert(store, alice, 0, "Again").await.is_err());
    assert!(insert(store, bob, 0, "HERO").await.is_err());
    assert!(insert(store, bob, 0, "no space").await.is_err());
    assert!(sqlx::query("UPDATE player SET job = 8")
        .execute(store.pool())
        .await
        .is_err());
    assert!(sqlx::query("UPDATE account SET empire = 4")
        .execute(store.pool())
        .await
        .is_err());
}

#[tokio::test]
async fn an_unknown_account_is_reported() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let missing = AccountId::new(0x7fff_fff0);
    assert!(matches!(
        lobby(&scratch.store, missing).await,
        Err(AccountError::NoSuchAccountId(id)) if id == missing
    ));
}
