//! Account, GM grant, and currency operations against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set, and then creates its own database, migrates
//! it, and drops it afterwards, so tests can run in parallel and leave nothing behind.

mod support;

use support::ScratchDatabase;

use common::gm::GmAuthority;
use db::accounts::{
    adjust_balance, create_account, find_auth_account, find_credentials, gm_authority, grant_gm,
    list_gm_grants, record_login, revoke_gm, set_password, AccountError, Currency, GmGrant, Name,
    NewAccount,
};
use db::credentials::{DeleteCode, Login, NewPassword};
use db::sqlx;
use db::store::{schema_version, Store};

fn login(raw: &str) -> Login {
    Login::new(raw).expect("a valid login")
}

fn name(raw: &str) -> Name {
    Name::new(raw).expect("a valid Name")
}

async fn create(store: &Store, raw_login: &str, password: &[u8]) -> db::accounts::AccountId {
    let account = NewAccount {
        login: login(raw_login),
        password: NewPassword::new(password)
            .expect("a valid password")
            .hash()
            .expect("hashing should succeed"),
        delete_code: DeleteCode::new("1234567").expect("a valid delete code"),
    };
    create_account(store, &account)
        .await
        .expect("the account should be created")
}

async fn balance(store: &Store, raw_login: &str, currency: Currency) -> i64 {
    adjust_balance(store, &login(raw_login), currency, 0)
        .await
        .expect("the balance should be readable")
}

#[tokio::test]
async fn migrating_twice_changes_nothing_and_records_the_newest_version() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    scratch
        .store
        .migrate()
        .await
        .expect("a second run should be a no-op");
    let applied: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(scratch.store.pool())
        .await
        .expect("the migration table should exist");
    assert_eq!(applied, schema_version());
}

#[tokio::test]
async fn an_account_is_created_once_and_checks_only_its_own_password() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let first = create(store, "Alice", b"s3cret pass").await;
    let second = create(store, "bob", b"other").await;
    assert_ne!(first, second);

    let again = NewAccount {
        login: login("ALICE"),
        password: NewPassword::new(b"x").unwrap().hash().unwrap(),
        delete_code: DeleteCode::random(),
    };
    assert!(matches!(
        create_account(store, &again).await,
        Err(AccountError::LoginTaken(taken)) if taken.as_str() == "alice"
    ));

    let (id, digest) = find_credentials(store, &login("alice"))
        .await
        .unwrap()
        .expect("alice exists");
    assert_eq!(id, first);
    assert!(digest.verify(b"s3cret pass").unwrap());
    assert!(!digest.verify(b"other").unwrap());
    assert!(find_credentials(store, &login("carol"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_new_account_starts_with_the_legacy_defaults() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    create(store, "alice", b"pw").await;
    let (status, language, delete_code, coins, cash, played): (
        String,
        i16,
        String,
        i64,
        i64,
        Option<i64>,
    ) = sqlx::query_as(
        "SELECT status, language, delete_code, coins, cash, \
         extract(epoch FROM last_play_at)::bigint FROM account WHERE login = 'alice'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(
        (
            status.as_str(),
            language,
            delete_code.as_str(),
            coins,
            cash,
            played
        ),
        ("OK", 1, "1234567", 0, 0, None)
    );
}

#[tokio::test]
async fn the_auth_read_reports_status_availability_and_creation_date() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let id = create(store, "alice", b"pw").await;
    assert_eq!(find_auth_account(store, &login("bob")).await.unwrap(), None);
    let account = find_auth_account(store, &login("alice"))
        .await
        .unwrap()
        .expect("alice exists");
    assert_eq!(account.id, id);
    assert!(account.password.verify(b"pw").unwrap());
    assert_eq!(account.status, "OK");
    assert!(!account.unavailable);
    let today: String = sqlx::query_scalar("SELECT to_char(now(), 'YYYYMMDD')")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(account.created_on, today);

    sqlx::query(
        "UPDATE account SET status = 'BLOCK', available_at = now() + interval '1 day', \
         created_at = '2031-02-03 12:00:00' WHERE login = 'alice'",
    )
    .execute(store.pool())
    .await
    .unwrap();
    let account = find_auth_account(store, &login("alice"))
        .await
        .unwrap()
        .expect("alice exists");
    assert_eq!(account.status, "BLOCK");
    assert!(account.unavailable);
    assert_eq!(account.created_on, "20310203");
}

#[tokio::test]
async fn a_recorded_login_stores_the_time_and_the_language() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let id = create(store, "alice", b"pw").await;
    record_login(store, id, 7).await.unwrap();
    let (language, played): (i16, bool) = sqlx::query_as(
        "SELECT language, last_play_at IS NOT NULL FROM account WHERE login = 'alice'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!((language, played), (7, true));
    assert!(matches!(
        record_login(store, id, 12).await,
        Err(AccountError::Database(_))
    ));
}

#[tokio::test]
async fn a_password_change_replaces_the_old_password() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    create(store, "alice", b"old").await;
    let new = NewPassword::new(b"new").unwrap().hash().unwrap();
    set_password(store, &login("alice"), &new).await.unwrap();
    let (_, digest) = find_credentials(store, &login("alice"))
        .await
        .unwrap()
        .unwrap();
    assert!(digest.verify(b"new").unwrap());
    assert!(!digest.verify(b"old").unwrap());
    assert!(matches!(
        set_password(store, &login("carol"), &new).await,
        Err(AccountError::NoSuchAccount(missing)) if missing.as_str() == "carol"
    ));
}

#[tokio::test]
async fn a_balance_moves_by_the_delta_and_never_leaves_its_range() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    create(store, "alice", b"pw").await;
    let alice = login("alice");

    assert_eq!(
        adjust_balance(store, &alice, Currency::Coins, 500)
            .await
            .unwrap(),
        500
    );
    assert_eq!(
        adjust_balance(store, &alice, Currency::Coins, -120)
            .await
            .unwrap(),
        380
    );
    assert!(matches!(
        adjust_balance(store, &alice, Currency::Coins, -381).await,
        Err(AccountError::BalanceOutOfRange {
            currency: Currency::Coins,
            balance: 380,
            delta: -381,
        })
    ));
    assert_eq!(balance(store, "alice", Currency::Coins).await, 380);
    assert_eq!(
        balance(store, "alice", Currency::Cash).await,
        0,
        "cash is separate"
    );

    let to_max = i64::MAX - 380;
    assert_eq!(
        adjust_balance(store, &alice, Currency::Coins, to_max)
            .await
            .unwrap(),
        i64::MAX
    );
    assert!(matches!(
        adjust_balance(store, &alice, Currency::Coins, 1).await,
        Err(AccountError::BalanceOutOfRange { .. })
    ));

    let dword_max = i64::from(u32::MAX);
    assert_eq!(
        adjust_balance(store, &alice, Currency::Cash, dword_max)
            .await
            .unwrap(),
        dword_max
    );
    assert!(matches!(
        adjust_balance(store, &alice, Currency::Cash, 1).await,
        Err(AccountError::BalanceOutOfRange {
            currency: Currency::Cash,
            ..
        })
    ));
    assert_eq!(balance(store, "alice", Currency::Cash).await, dword_max);

    assert!(matches!(
        adjust_balance(store, &login("carol"), Currency::Cash, 1).await,
        Err(AccountError::NoSuchAccount(_))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_balance_changes_all_count() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = scratch.store.clone();
    create(&store, "alice", b"pw").await;
    let changes: Vec<_> = (0..32)
        .map(|_| {
            let store = store.clone();
            tokio::spawn(async move {
                adjust_balance(&store, &login("alice"), Currency::Coins, 1)
                    .await
                    .expect("each change should apply")
            })
        })
        .collect();
    for change in changes {
        change.await.expect("the task should finish");
    }
    assert_eq!(balance(&store, "alice", Currency::Coins).await, 32);
}

#[tokio::test]
async fn a_gm_grant_belongs_to_one_account_and_matches_names_in_any_case() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let alice = create(store, "alice", b"pw").await;
    let bob = create(store, "bob", b"pw").await;

    grant_gm(store, &login("alice"), &name("Admin"), GmAuthority::God)
        .await
        .unwrap();
    assert_eq!(
        gm_authority(store, alice, "Admin").await.unwrap(),
        Some(GmAuthority::God)
    );
    assert_eq!(
        gm_authority(store, alice, "ADMIN").await.unwrap(),
        Some(GmAuthority::God)
    );
    assert_eq!(
        gm_authority(store, bob, "Admin").await.unwrap(),
        None,
        "wrong account"
    );
    assert_eq!(gm_authority(store, alice, "Other").await.unwrap(), None);

    // Regranting on the same account changes the authority and the spelling.
    grant_gm(
        store,
        &login("alice"),
        &name("ADMIN"),
        GmAuthority::Implementor,
    )
    .await
    .unwrap();
    assert_eq!(
        list_gm_grants(store).await.unwrap(),
        vec![GmGrant {
            login: "alice".to_owned(),
            name: "ADMIN".to_owned(),
            authority: GmAuthority::Implementor,
        }]
    );

    // Another account cannot take the Name, and the grant is untouched.
    assert!(matches!(
        grant_gm(store, &login("bob"), &name("admin"), GmAuthority::LowWizard).await,
        Err(AccountError::NameGrantedElsewhere { login, .. }) if login == "alice"
    ));
    assert_eq!(
        gm_authority(store, alice, "admin").await.unwrap(),
        Some(GmAuthority::Implementor)
    );
    assert!(matches!(
        grant_gm(store, &login("carol"), &name("Carol"), GmAuthority::God).await,
        Err(AccountError::NoSuchAccount(_))
    ));

    grant_gm(store, &login("bob"), &name("Builder"), GmAuthority::Wizard)
        .await
        .unwrap();
    let names: Vec<String> = list_gm_grants(store)
        .await
        .unwrap()
        .into_iter()
        .map(|grant| grant.name)
        .collect();
    assert_eq!(names, ["ADMIN", "Builder"]);

    revoke_gm(store, &name("admin")).await.unwrap();
    assert_eq!(gm_authority(store, alice, "ADMIN").await.unwrap(), None);
    assert!(matches!(
        revoke_gm(store, &name("admin")).await,
        Err(AccountError::NoSuchGrant(_))
    ));
}

#[tokio::test]
async fn the_schema_refuses_what_the_rust_types_refuse() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let pool = scratch.store.pool();
    let hash = NewPassword::new(b"pw").unwrap().hash().unwrap();
    for (login, password_hash, delete_code) in [
        ("Alice", hash.as_str(), "1234567"),
        ("a", hash.as_str(), "1234567"),
        (
            "alice",
            "$argon2i$v=19$m=16,t=2,p=1$c2FsdHNhbHQ$aGFzaA",
            "1234567",
        ),
        ("alice", hash.as_str(), "123456"),
    ] {
        let inserted = sqlx::query(
            "INSERT INTO account (login, password_hash, delete_code) VALUES ($1, $2, $3)",
        )
        .bind(login)
        .bind(password_hash)
        .bind(delete_code)
        .execute(pool)
        .await;
        assert!(
            inserted.is_err(),
            "{login:?} {delete_code:?} should be refused"
        );
    }

    create(&scratch.store, "alice", b"pw").await;
    for statement in [
        "UPDATE account SET cash = 4294967296",
        "UPDATE account SET coins = -1",
        "UPDATE account SET language = 12",
        "UPDATE account SET status = ''",
        "UPDATE account SET status = 'NO SPACE'",
        "INSERT INTO gm_grant (account_id, name, authority) \
         SELECT id, 'Admin', 'PLAYER' FROM account",
        "INSERT INTO gm_grant (account_id, name, authority) \
         SELECT id, 'Ad min', 'GOD' FROM account",
        "INSERT INTO gm_grant (account_id, name, authority) \
         SELECT id, 'A', 'GOD' FROM account",
    ] {
        assert!(
            sqlx::query(statement).execute(pool).await.is_err(),
            "{statement} should be refused"
        );
    }
    // The same statements with valid values succeed, so each refusal above came from its check.
    for statement in [
        "UPDATE account SET cash = 4294967295, coins = 0, language = 11, status = 'BLOCK'",
        "INSERT INTO gm_grant (account_id, name, authority) \
         SELECT id, 'Admin', 'GOD' FROM account",
    ] {
        sqlx::query(statement)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("{statement} should succeed: {error}"));
    }
}
