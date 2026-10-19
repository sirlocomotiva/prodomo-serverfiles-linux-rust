//! Guild rows against a real PostgreSQL 18 server.
//!
//! Every test runs only when `DATABASE_URL` is set; see `support`.

mod support;

use db::accounts::{create_account, AccountId, Name, NewAccount};
use db::credentials::{DeleteCode, Login, NewPassword};
use db::guilds::{
    add_member, add_offer, add_power, board, create_guild, delete_comment, delete_mark, load_all,
    load_marks, load_symbols, lobby_guilds, post_comment, remove_member, save_mark, save_symbol,
    set_grade_auth, set_grade_name, set_member_general, set_member_grade, set_skills, spend_power,
    CreateGuildError, GuildOffer,
};
use db::players::{create_player, lobby, Created, NewPlayer};
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

fn grades() -> [(&'static str, u8); 15] {
    std::array::from_fn(|index| {
        if index == 0 {
            ("Leader", 15)
        } else {
            ("...", 0)
        }
    })
}

#[tokio::test]
async fn a_second_creation_with_the_same_name_answers_taken() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let first = account(store, "firstmaster").await;
    let second = account(store, "secondmaster").await;
    let first_pid = created(store, first, 0, "FirstMaster").await;
    let second_pid = created(store, second, 0, "SecondMaster").await;
    let id = create_guild(store, "RaceGuild", first_pid, &grades())
        .await
        .expect("the first creation should store");
    assert!(id > 0, "the id is the store's answer");
    let error = create_guild(store, "RaceGuild", second_pid, &grades())
        .await
        .expect_err("the second creation should fail");
    assert!(
        matches!(error, CreateGuildError::GuildNameTaken { .. }),
        "a concurrent name answers taken, got {error:?}"
    );
}

#[tokio::test]
async fn a_second_creation_with_a_cased_name_answers_taken() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let first = account(store, "casemaster").await;
    let second = account(store, "caseother").await;
    let first_pid = created(store, first, 0, "CaseMaster").await;
    let second_pid = created(store, second, 0, "CaseOther").await;
    create_guild(store, "CaseGuild", first_pid, &grades())
        .await
        .expect("the first creation should store");
    let error = create_guild(store, "CASEGUILD", second_pid, &grades())
        .await
        .expect_err("the cased creation should fail");
    assert!(
        matches!(error, CreateGuildError::GuildNameTaken { .. }),
        "the lower index refuses a cased twin, got {error:?}"
    );
}

#[tokio::test]
async fn load_all_reads_what_create_guild_wrote() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let owner = account(store, "loadmaster").await;
    let pid = created(store, owner, 0, "LoadMaster").await;
    let id = create_guild(store, "LoadGuild", pid, &grades())
        .await
        .expect("the creation should store");
    let stored = load_all(store).await.expect("the load should read");
    assert_eq!(stored.guilds.len(), 1, "one guild row");
    let guild = &stored.guilds[0];
    assert_eq!(guild.id, id, "the id is the store's answer");
    assert_eq!(guild.name, "LoadGuild", "the name round-trips");
    assert_eq!(guild.master_pid, pid, "the master round-trips");
    assert_eq!(guild.level, 1, "a new guild is level 1");
    assert_eq!(stored.grades.len(), 15, "fifteen grade rows");
    assert_eq!(stored.members.len(), 1, "the master's member row");
    assert_eq!(stored.members[0].guild_id, id, "the member links the guild");
    assert_eq!(stored.members[0].pid, pid, "the member links the master");
    assert_eq!(stored.members[0].grade, 1, "the master is grade 1");
}

#[tokio::test]
async fn a_member_row_round_trips_through_add_and_remove() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let master = account(store, "joinmaster").await;
    let guest = account(store, "joinguest").await;
    let master_pid = created(store, master, 0, "JoinMaster").await;
    let guest_pid = created(store, guest, 0, "JoinGuest").await;
    let id = create_guild(store, "JoinGuild", master_pid, &grades())
        .await
        .expect("the creation should store");
    add_member(store, id, guest_pid, 15)
        .await
        .expect("the join should store");
    let stored = load_all(store).await.expect("the load should read");
    assert_eq!(stored.members.len(), 2, "two member rows");
    let joined = stored
        .members
        .iter()
        .find(|member| member.pid == guest_pid)
        .expect("the guest's row");
    assert_eq!(joined.guild_id, id, "the row links the guild");
    assert_eq!(joined.grade, 15, "an accepted invitee is grade 15");
    assert!(remove_member(store, guest_pid)
        .await
        .expect("the delete runs"));
    let stored = load_all(store).await.expect("the load should read");
    assert_eq!(stored.members.len(), 1, "the master's row alone");
    assert!(
        !remove_member(store, guest_pid)
            .await
            .expect("the delete runs"),
        "a second delete finds nothing"
    );
}

#[tokio::test]
async fn grade_and_member_rows_round_trip_through_the_setters() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let master = account(store, "setmaster").await;
    let guest = account(store, "setguest").await;
    let master_pid = created(store, master, 0, "SetMaster").await;
    let guest_pid = created(store, guest, 0, "SetGuest").await;
    let id = create_guild(store, "SetGuild", master_pid, &grades())
        .await
        .expect("the creation should store");
    add_member(store, id, guest_pid, 15)
        .await
        .expect("the join should store");
    assert!(
        set_grade_name(store, id, 15, "Veteran")
            .await
            .expect("the rename runs"),
        "the grade row is there"
    );
    assert!(
        set_grade_auth(store, id, 15, 7)
            .await
            .expect("the re-auth runs"),
        "the grade row is there"
    );
    assert!(
        set_member_grade(store, guest_pid, 5)
            .await
            .expect("the re-grade runs"),
        "the member row is there"
    );
    assert!(
        set_member_general(store, guest_pid, true)
            .await
            .expect("the flag runs"),
        "the member row is there"
    );
    let stored = load_all(store).await.expect("the load should read");
    let grade = stored
        .grades
        .iter()
        .find(|grade| grade.grade == 15)
        .expect("the grade row");
    assert_eq!(grade.name, "Veteran", "the name lands");
    assert_eq!(grade.auth, 7, "the auth lands");
    let member = stored
        .members
        .iter()
        .find(|member| member.pid == guest_pid)
        .expect("the member row");
    assert_eq!(member.grade, 5, "the grade lands");
    assert!(member.is_general, "the flag lands");
    assert!(
        !set_member_grade(store, 987_654_321, 5)
            .await
            .expect("the re-grade runs"),
        "a missing member row answers false"
    );
}

#[tokio::test]
async fn an_offer_charges_and_comments_round_trip_through_their_writers() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let master = account(store, "offerboard").await;
    let master_pid = created(store, master, 0, "OfferBoard").await;
    let id = create_guild(store, "OfferGuild", master_pid, &grades())
        .await
        .expect("the creation should store");
    // The exp and gold a kill and a sale left, which the offer and charge spend.
    db::sqlx::query("UPDATE player SET exp = 60_000, gold = 50_000 WHERE id = $1")
        .bind(i32::try_from(master_pid).unwrap())
        .execute(store.pool())
        .await
        .unwrap();
    assert!(
        add_offer(
            store,
            GuildOffer {
                guild_id: id,
                pid: master_pid,
                floored: 20_400,
                offer: 204,
                level: 1,
                exp: 204,
                ladder: 0,
            },
        )
        .await
        .expect("the offer runs"),
        "the guild row is there"
    );
    assert!(
        add_power(store, id, master_pid, 5_000, 400)
            .await
            .expect("the charge runs"),
        "the guild row is there"
    );
    let first = post_comment(store, id, "OfferBoard", "hello all")
        .await
        .expect("the post runs");
    let second = post_comment(store, id, "OfferBoard", "!war at nine")
        .await
        .expect("the notice runs");
    assert!(second > first, "the ids rise");
    let comments = board(store, id, 12).await.expect("the board reads");
    assert_eq!(comments.len(), 2, "both posts");
    assert_eq!(comments[0].id, second, "the notice rides first");
    assert_eq!(comments[1].id, first);
    assert!(
        delete_comment(store, id, first, true, "OfferBoard")
            .await
            .expect("the delete runs"),
        "the leader names the id alone"
    );
    assert!(
        !delete_comment(store, id, 987_654, true, "OfferBoard")
            .await
            .expect("the delete runs"),
        "a missing comment answers false"
    );
    let stored = load_all(store).await.expect("the load should read");
    let guild = stored
        .guilds
        .iter()
        .find(|guild| guild.id == id)
        .expect("the guild row");
    assert_eq!(guild.exp, 204, "the levelled delta lands");
    assert_eq!(guild.sp, 400, "the power lands");
    let member = stored
        .members
        .iter()
        .find(|member| member.pid == master_pid)
        .expect("the member row");
    assert_eq!(member.offer, 204, "the member's offer lands");
    let held: (i64, i64) = db::sqlx::query_as("SELECT exp, gold FROM player WHERE id = $1")
        .bind(i32::try_from(master_pid).unwrap())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(held, (39_600, 45_000), "the player row keeps the debits");
    let comments = board(store, id, 12).await.expect("the board reads again");
    assert_eq!(comments.len(), 1, "the delete lands");
}

/// `set_skills` and `spend_power`: the `/gskillup` climb and the cast's consume.
#[tokio::test]
async fn skill_levels_and_spent_power_round_trip_through_their_writers() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let master = account(store, "skiller").await;
    let master_pid = created(store, master, 0, "Skiller").await;
    let id = create_guild(store, "SkillGuild", master_pid, &grades())
        .await
        .expect("the creation should store");
    let mut levels = [0u8; 12];
    levels[1] = 2;
    levels[7] = 1;
    assert!(
        set_skills(store, id, &levels, 0)
            .await
            .expect("the climb runs"),
        "the guild row is there"
    );
    assert!(
        spend_power(store, id, 229).await.expect("the cast runs"),
        "the guild row is there"
    );
    assert!(
        !set_skills(store, id + 1, &levels, 0)
            .await
            .expect("the missing climb runs"),
        "a missing guild answers false"
    );
    assert!(
        !spend_power(store, id + 1, 229)
            .await
            .expect("the missing cast runs"),
        "a missing guild answers false"
    );
    let stored = load_all(store).await.expect("the load should read");
    let guild = stored
        .guilds
        .iter()
        .find(|guild| guild.id == id)
        .expect("the guild row");
    assert_eq!(guild.skill_point, 0, "the point lands");
    assert_eq!(guild.skills, levels.to_vec(), "the bytes land");
    assert_eq!(guild.sp, 229, "the power lands");
}

#[tokio::test]
async fn marks_and_symbols_round_trip_through_their_writers() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let account = account(store, "markwriter").await;
    let pid = created(store, account, 0, "MarkWriter").await;
    let guild = create_guild(store, "Marked", pid, &grades()).await.unwrap();
    // Empty at first.
    assert!(load_marks(store).await.unwrap().is_empty());
    assert!(load_symbols(store).await.unwrap().is_empty());
    // A mark stores and reads back.
    let image = vec![0x11_u8; 768];
    save_mark(store, guild, 0, &image).await.unwrap();
    let marks = load_marks(store).await.unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].guild_id, guild);
    assert_eq!(marks[0].mark_id, 0);
    assert_eq!(marks[0].image, image);
    // A re-upload keeps the judged id and replaces the image.
    let image2 = vec![0x22_u8; 768];
    save_mark(store, guild, 0, &image2).await.unwrap();
    let marks = load_marks(store).await.unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].image, image2);
    // A delete removes the row and answers whether one was there.
    assert!(delete_mark(store, guild).await.unwrap());
    assert!(!delete_mark(store, guild).await.unwrap());
    assert!(load_marks(store).await.unwrap().is_empty());
    // A symbol stores and reads back, and a re-upload replaces the bytes.
    save_symbol(store, guild, b"symbol-bytes").await.unwrap();
    let symbols = load_symbols(store).await.unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].guild_id, guild);
    assert_eq!(symbols[0].data, b"symbol-bytes");
    save_symbol(store, guild, b"new-bytes").await.unwrap();
    let symbols = load_symbols(store).await.unwrap();
    assert_eq!(symbols[0].data, b"new-bytes");
}

#[tokio::test]
async fn lobby_guilds_answers_each_slot_s_linked_guild() {
    let Some(scratch) = ScratchDatabase::create().await else {
        return;
    };
    let store = &scratch.store;
    let owner = account(store, "lobbymaster").await;
    let master = created(store, owner, 0, "LobbyMaster").await;
    let _plain = created(store, owner, 2, "LobbyPlain").await;
    let id = create_guild(store, "LobbyGuild", master, &grades())
        .await
        .expect("the creation should store");
    let seen = lobby(store, owner).await.expect("the lobby should load");
    let linked = lobby_guilds(store, &seen)
        .await
        .expect("the links should read");
    assert_eq!(
        linked[0],
        Some((id, "LobbyGuild".to_owned())),
        "the guilded slot answers its id and name"
    );
    assert_eq!(linked[1], None, "an empty slot answers none");
    assert_eq!(linked[2], None, "an unguilded slot answers none");
    assert_eq!(linked[3], None, "an empty slot answers none");
}
