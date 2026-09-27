//! The Channel login's read of an account's empire and characters, against a real PostgreSQL
//! 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.

mod support;

use db::accounts::{create_account, AccountError, AccountId, Name, NewAccount};
use db::credentials::{DeleteCode, Login, NewPassword};
use db::players::{
    change_name, create_player, delete_player, load_character, lobby, save_position, select_empire,
    Created, LobbyPlayer, NewPlayer, PlayerDelete,
};
use db::sqlx::{self, Row};
use db::store::Store;
use support::ScratchDatabase;

async fn account(store: &Store, raw_login: &str) -> AccountId {
    account_with_code(store, raw_login, DeleteCode::random()).await
}

async fn account_with_code(store: &Store, raw_login: &str, delete_code: DeleteCode) -> AccountId {
    let account = NewAccount {
        login: Login::new(raw_login).expect("a valid login"),
        password: NewPassword::new(b"pw").unwrap().hash().unwrap(),
        delete_code,
    };
    create_account(store, &account).await.unwrap()
}

fn column(id: AccountId) -> i32 {
    i32::try_from(id.get()).unwrap()
}

fn new_player(slot: u8, name: &str) -> NewPlayer {
    NewPlayer {
        slot,
        name: Name::new(name).unwrap(),
        job: 6,
        st: 0x15,
        ht: 0x16,
        dx: 0x17,
        iq: 0x18,
        hp: 0x0102_0304,
        sp: -0x0506,
        stamina: 0x0708,
        part_base: 1,
        x: 459_812,
        y: -953_877,
    }
}

async fn created(store: &Store, account: AccountId, slot: u8, name: &str) -> u32 {
    match create_player(store, account, &new_player(slot, name))
        .await
        .unwrap()
    {
        Created::Player(id) => id,
        Created::Taken => panic!("slot {slot} or {name} was taken"),
    }
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

/// `load_character` is the loading phase's read, and the one place a descriptor names a
/// character by ID. It must be scoped to the account: a character of another account is the
/// same refusal as a character that does not exist, so a client cannot probe for IDs.
/// Move the `player` id sequence above every account id.
///
/// The load query takes one value from each table, and both identity sequences start at 1, so
/// `p.account_id = <character id> AND p.id = <account id>` is still satisfiable on a fresh
/// scratch database and swapping the two binds survives. A deleted row still advances
/// `GENERATED ALWAYS AS IDENTITY`, so this burns one id per account by inserting a character and
/// deleting it. Every character created afterwards is above every account id, so the two ranges
/// are disjoint. The count comes from the sequence itself, so it does not assume where the
/// sequence starts.
async fn separate_id_spaces(store: &Store) {
    let owner = account(store, "burn").await;
    let gap: i64 = sqlx::query_scalar(
        "SELECT (SELECT COALESCE(MAX(id), 0) + 1 FROM account) - COALESCE((SELECT last_value \
         FROM pg_sequences WHERE schemaname = current_schema() AND sequencename = \
         split_part(pg_get_serial_sequence('player', 'id'), '.', 2)), 0)",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    for n in 0..gap.max(1) {
        let id = insert(store, owner, 0, &format!("Burn{n}")).await.unwrap();
        sqlx::query("DELETE FROM player WHERE id = $1")
            .bind(id)
            .execute(store.pool())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn load_character_reads_every_field_and_never_another_accounts_character() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    // The load query binds two parameters, and swapping them is only observable when the
    // account id and the character id differ. A fresh scratch database starts both sequences at
    // 1, where `p.account_id = <character id> AND p.id = <account id>` is still satisfiable, so
    // the swap survives. `separate_id_spaces` moves the player sequence clear of the account
    // sequence first, and the precondition it asserts below is what makes the swap killable.
    separate_id_spaces(store).await;
    let alice = account(store, "alice").await;
    let bob = account(store, "bob").await;
    sqlx::query("UPDATE account SET empire = 1 WHERE id = $1")
        .bind(column(alice))
        .execute(store.pool())
        .await
        .unwrap();
    let alpha = insert(store, alice, 0, "Alpha").await.unwrap();
    let other = insert(store, bob, 0, "Other").await.unwrap();
    let alpha_id = u32::try_from(alpha).unwrap();
    let other_id = u32::try_from(other).unwrap();
    let alice_id = u32::try_from(column(alice)).unwrap();
    assert_ne!(alpha_id, alice_id, "Alpha's id equals the account id");
    assert_ne!(other_id, alice_id, "Other's id equals the account id");

    let found = load_character(store, alice, alpha_id).await.unwrap();
    assert_eq!(found.id, alpha_id);
    assert_eq!(found.slot, 0);
    assert_eq!(found.name.as_str(), "Alpha");
    assert_eq!(
        found.empire, 1,
        "the empire comes from the account, not the player row"
    );
    assert_eq!(found.job, 3);
    assert_eq!(found.level, 0x9a);
    assert_eq!(found.st, 0x11);
    assert_eq!(found.ht, 0x12);
    assert_eq!(found.dx, 0x13);
    assert_eq!(found.iq, 0x14);
    assert_eq!(found.conqueror_level, 0x21);
    assert_eq!(found.main_part, 0xa1b2);
    assert_eq!(found.hair_part, 0xc3d4);
    assert_eq!(found.sash_part, 0xe5f6);
    assert_eq!((found.x, found.y), (-469_300, 964_200));
    assert_eq!(found.skill_group, 0x31);
    assert!(found.change_name);
    // The columns a fresh row leaves at their defaults come back as the column default, not as
    // stack garbage, which is what the points record needs to stay honest.
    assert_eq!(found.exp, 0, "exp");
    assert_eq!(found.conqueror_exp, 0, "conqueror_exp");
    assert_eq!(found.gold, 0, "gold");
    assert_eq!(found.voice, 0, "voice");
    assert_eq!(found.hp, 0, "hp");
    assert_eq!(found.sp, 0, "sp");
    assert_eq!(found.stamina, 0, "stamina");
    assert_eq!(found.part_base, 0, "part_base");

    // The other account's character is refused, and so is an ID no row has. The two must be
    // indistinguishable, or the refusal is an ID oracle. `AccountError` carries no `PartialEq`,
    // so each refusal is matched on its variant and its payload.
    for (label, id) in [
        ("another account's character", other_id),
        ("an id no row has", alpha_id + 1_000_000),
        ("id zero", 0),
    ] {
        assert!(
            matches!(
                load_character(store, alice, id).await,
                Err(AccountError::NoSuchPlayer(refused)) if refused == id
            ),
            "{label} ({id}) is not refused as NoSuchPlayer"
        );
    }
    let owned = load_character(store, bob, other_id)
        .await
        .expect("the owning account reads its own character");
    assert_eq!(owned.name.as_str(), "Other");
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

#[tokio::test]
async fn a_created_character_starts_at_level_one_wearing_its_shape() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = account(store, "alice").await;
    let id = created(store, alice, 2, "Hero").await;

    let row = sqlx::query(
        "SELECT account_id, slot, name, job, level, st, ht, dx, iq, hp, sp, stamina, \
         part_base, part_main, part_hair, part_sash, x, y, playtime_minutes, change_name \
         FROM player WHERE id = $1",
    )
    .bind(i32::try_from(id).unwrap())
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<i32, _>("account_id"), column(alice));
    assert_eq!(row.get::<i16, _>("slot"), 2);
    assert_eq!(row.get::<String, _>("name"), "Hero");
    assert_eq!(row.get::<i16, _>("job"), 6);
    assert_eq!(row.get::<i16, _>("level"), 1);
    let stats: [i16; 4] = ["st", "ht", "dx", "iq"].map(|c| row.get(c));
    assert_eq!(stats, [0x15, 0x16, 0x17, 0x18]);
    assert_eq!(row.get::<i32, _>("hp"), 0x0102_0304);
    assert_eq!(row.get::<i32, _>("sp"), -0x0506);
    assert_eq!(row.get::<i32, _>("stamina"), 0x0708);
    let parts: [i32; 3] = ["part_main", "part_hair", "part_sash"].map(|c| row.get(c));
    assert_eq!(
        parts,
        [1, 1, 0],
        "D/ClientManagerPlayer.cpp stores the shape as both parts"
    );
    assert_eq!(row.get::<i16, _>("part_base"), 1);
    assert_eq!(row.get::<i32, _>("x"), 459_812);
    assert_eq!(row.get::<i32, _>("y"), -953_877);
    assert_eq!(row.get::<i32, _>("playtime_minutes"), 0);
    assert!(!row.get::<bool, _>("change_name"));

    let found = lobby(store, alice).await.unwrap();
    assert_eq!(found.in_slot(2).map(|p| p.id), Some(id));
}

#[tokio::test]
async fn a_taken_slot_or_name_refuses_the_create() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = account(store, "alice").await;
    let bob = account(store, "bob").await;
    created(store, alice, 0, "Hero").await;
    for (who, slot, name) in [(alice, 0, "Other"), (alice, 1, "hERO"), (bob, 0, "HERO")] {
        assert_eq!(
            create_player(store, who, &new_player(slot, name))
                .await
                .unwrap(),
            Created::Taken,
            "{slot} {name}"
        );
    }
    created(store, bob, 0, "Other").await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM player")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 2);

    let missing = AccountId::new(0x7fff_fff0);
    assert!(matches!(
        create_player(store, missing, &new_player(0, "Nobody")).await,
        Err(AccountError::NoSuchAccountId(id)) if id == missing
    ));
}

fn delete(slot: u8, player: u32, code: &[u8]) -> PlayerDelete<'_> {
    PlayerDelete {
        slot,
        player,
        code,
        level_limit: 120,
        level_limit_lower: 0,
    }
}

async fn player_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM player")
        .fetch_one(store.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_delete_needs_the_code_the_slot_and_a_level_inside_the_limits() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = account_with_code(store, "alice", DeleteCode::new("Ab3dE5g").unwrap()).await;
    let bob = account_with_code(store, "bob", DeleteCode::new("Ab3dE5g").unwrap()).await;
    let hero = created(store, alice, 1, "Hero").await;
    let other = created(store, bob, 1, "Other").await;

    let refused = [
        (alice, delete(1, hero, b"ab3dE5g\0")),
        (alice, delete(1, hero, b"Ab3dE5\0\0")),
        (alice, delete(1, hero, b"Ab3dE5")),
        (alice, delete(0, hero, b"Ab3dE5g\0")),
        (alice, delete(1, other, b"Ab3dE5g\0")),
        (bob, delete(1, hero, b"Ab3dE5g\0")),
        (alice, delete(1, u32::MAX, b"Ab3dE5g\0")),
    ];
    for (who, request) in &refused {
        assert!(
            !delete_player(store, *who, request).await.unwrap(),
            "{request:?}"
        );
    }
    assert_eq!(player_count(store).await, 2);

    sqlx::query("UPDATE player SET level = 120 WHERE id = $1")
        .bind(i32::try_from(hero).unwrap())
        .execute(store.pool())
        .await
        .unwrap();
    assert!(!delete_player(store, alice, &delete(1, hero, b"Ab3dE5g\0"))
        .await
        .unwrap());
    sqlx::query("UPDATE player SET level = 119 WHERE id = $1")
        .bind(i32::try_from(hero).unwrap())
        .execute(store.pool())
        .await
        .unwrap();
    let lower = PlayerDelete {
        level_limit_lower: 120,
        ..delete(1, hero, b"Ab3dE5g\0")
    };
    assert!(!delete_player(store, alice, &lower).await.unwrap());
    assert_eq!(player_count(store).await, 2);

    assert!(delete_player(store, alice, &delete(1, hero, b"Ab3dE5gX"))
        .await
        .unwrap());
    assert_eq!(player_count(store).await, 1);
    let (archived_id, archived_name, level): (i32, String, i64) = sqlx::query_as(
        "SELECT player_id, name, (player ->> 'level')::bigint FROM player_deleted \
         WHERE account_id = $1",
    )
    .bind(column(alice))
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(u32::try_from(archived_id).ok(), Some(hero));
    assert_eq!((archived_name.as_str(), level), ("Hero", 119));

    assert!(
        !delete_player(store, alice, &delete(1, hero, b"Ab3dE5g\0"))
            .await
            .unwrap(),
        "a deleted character is gone"
    );
    created(store, alice, 1, "HERO").await;
}

async fn position(store: &Store, id: u32) -> (i32, i32) {
    sqlx::query_as("SELECT x, y FROM player WHERE id = $1")
        .bind(i32::try_from(id).unwrap())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn an_empire_is_chosen_until_the_account_has_a_character() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = account(store, "alice").await;
    assert!(select_empire(store, alice, 2, (55_700, 157_900))
        .await
        .unwrap());
    assert_eq!(lobby(store, alice).await.unwrap().empire, 2);
    assert!(
        select_empire(store, alice, 3, (969_600, 278_400))
            .await
            .unwrap(),
        "an empire without characters can still change"
    );
    let id = created(store, alice, 0, "Hero").await;
    assert!(!select_empire(store, alice, 1, (469_300, 964_200))
        .await
        .unwrap());
    assert_eq!(lobby(store, alice).await.unwrap().empire, 3);
    assert_eq!(position(store, id).await, (459_812, -953_877));

    let missing = AccountId::new(0x7fff_fff0);
    assert!(matches!(
        select_empire(store, missing, 1, (0, 0)).await,
        Err(AccountError::NoSuchAccountId(id)) if id == missing
    ));
}

#[tokio::test]
async fn choosing_an_empire_moves_every_slot_to_its_start() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = account(store, "alice").await;
    let bob = account(store, "bob").await;
    let mut ids = Vec::new();
    for (slot, name) in [(0, "Zero"), (1, "One"), (2, "Two"), (3, "Three")] {
        ids.push(created(store, alice, slot, name).await);
    }
    let bystander = created(store, bob, 0, "Bystander").await;
    assert!(select_empire(store, alice, 1, (469_300, -964_201))
        .await
        .unwrap());
    for id in ids {
        assert_eq!(position(store, id).await, (469_300, -964_201), "{id}");
    }
    assert_eq!(position(store, bystander).await, (459_812, -953_877));
    assert_eq!(lobby(store, alice).await.unwrap().empire, 1);
    assert_eq!(lobby(store, bob).await.unwrap().empire, 0);
}

#[tokio::test]
async fn a_new_name_is_unique_regardless_of_case_and_clears_the_request() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = account(store, "alice").await;
    let bob = account(store, "bob").await;
    let hero = created(store, alice, 0, "Hero").await;
    created(store, bob, 0, "Taken").await;
    sqlx::query("UPDATE player SET change_name = true")
        .execute(store.pool())
        .await
        .unwrap();

    let taken = Name::new("tAKEN").unwrap();
    assert!(!change_name(store, alice, hero, &taken).await.unwrap());
    let found = lobby(store, alice).await.unwrap();
    assert_eq!(found.players[0].name, "Hero");
    assert!(found.players[0].change_name);

    let again = Name::new("HERO").unwrap();
    assert!(
        change_name(store, alice, hero, &again).await.unwrap(),
        "a character may keep its own Name in another case"
    );
    let fresh = Name::new("Fresh").unwrap();
    assert!(change_name(store, alice, hero, &fresh).await.unwrap());
    let found = lobby(store, alice).await.unwrap();
    assert_eq!(found.players[0].name, "Fresh");
    assert!(!found.players[0].change_name);
    assert!(lobby(store, bob).await.unwrap().players[0].change_name);

    for (who, id) in [(bob, hero), (alice, u32::MAX), (alice, hero + 1000)] {
        assert!(matches!(
            change_name(store, who, id, &fresh).await,
            Err(AccountError::NoSuchPlayer(missing)) if missing == id
        ));
    }
}

/// The Warp write. A character's stored position is what a login loads, so moving it is the
/// Warp: the next login must see the new coordinates, and another account's character must not
/// move when a Warp names a row that account does not hold.
#[tokio::test]
async fn a_saved_position_is_the_one_the_next_load_reads() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = account(store, "alice").await;
    let bob = account(store, "bob").await;
    let hero = created(store, alice, 0, "Hero").await;
    let other = created(store, bob, 0, "Other").await;
    let (before_x, before_y) = (459_812, -953_877);

    assert!(
        save_position(store, alice, hero, 469_300, 964_200)
            .await
            .unwrap(),
        "the row belongs to the account"
    );
    let moved = load_character(store, alice, hero).await.unwrap();
    assert_eq!((moved.x, moved.y), (469_300, 964_200));

    // Another account's character keeps its position: the Warp names an account and an ID.
    let untouched = load_character(store, bob, other).await.unwrap();
    assert_eq!((untouched.x, untouched.y), (before_x, before_y));

    // A character ID the account does not hold moves nothing and says so.
    assert!(!save_position(store, alice, other, 1, 2).await.unwrap());
    assert_eq!(
        load_character(store, bob, other).await.unwrap().x,
        before_x,
        "another account's row is still where it was"
    );
    // An ID inside the column's range that names no row is a miss, not an error.
    assert!(!save_position(store, alice, 1_000_000, 1, 2).await.unwrap());
    // An ID above i32::MAX cannot name a row, and it is reported rather than wrapped: a cast
    // would turn it into a negative ID that could match a real character.
    for too_wide in [i32::MAX as u32 + 1, u32::MAX] {
        assert!(matches!(
            save_position(store, alice, too_wide, 1, 2).await,
            Err(AccountError::NoSuchPlayer(missing)) if missing == too_wide
        ));
    }
}

/// A Warp can move a character to the origin and to negative coordinates, so neither is
/// special. Legacy writes both through the same `m_posWarp` fields.
#[tokio::test]
async fn a_saved_position_takes_the_origin_and_negative_coordinates() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = account(store, "alice").await;
    let hero = created(store, alice, 0, "Hero").await;

    for (x, y) in [(0, 0), (-1, -1), (i32::MAX, i32::MIN)] {
        assert!(
            save_position(store, alice, hero, x, y).await.unwrap(),
            "{x},{y}"
        );
        let moved = load_character(store, alice, hero).await.unwrap();
        assert_eq!((moved.x, moved.y), (x, y));
    }
}
