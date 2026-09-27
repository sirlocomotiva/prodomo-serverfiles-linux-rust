//! A granted item's row against a real store.
//!
//! [`prodomo::item_persist::persist_grant`] is a crossing between the world, the game
//! thread, and PostgreSQL, so none of its three test homes can cover it. `item_persist.rs`
//! holds the repair decisions with a recording revoker; `game_loop_thread.rs` holds the
//! repair against the real game thread. This file holds the store, because the claim that
//! matters most -- a refused write must not overwrite somebody else's row -- is only
//! observable in the table.
//!
//! Every test is gated on `DATABASE_URL` and creates its own database, so the suite stays
//! green without a server.

mod support;

use std::time::Duration;

use prodomo::game_loop::spawn_game_loop;
use prodomo::game_loop_messages::GameLoopConfig;
use prodomo::game_state::GameState;
use prodomo::item_persist::persist_grant;
use support::ScratchDatabase;

/// A vnum for a one-cell item outside every custom bank.
fn a_plain_vnum() -> u32 {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
    let protos =
        gamedata::item_proto::ItemProtos::load(&dir).expect("the owner's item protos load");
    protos
        .rows()
        .iter()
        .find(|proto| {
            proto.size == 1
                && (0..6).all(|category| {
                    !gamedata::item_custom_category::is_custom_category(proto, category)
                })
        })
        .expect("a one-cell item outside every custom bank")
        .vnum
}

/// The stored row for `id`, or a panic.
///
/// A panic is right here: a test that continues without knowing what is in the table is
/// not testing anything.
async fn first_row_for(store: &db::store::Store, id: u32) -> db::items::ItemRow {
    db::items::load_item(store, id)
        .await
        .expect("the read")
        .unwrap_or_else(|| panic!("row {id} is not stored"))
}

/// The stored player id for `name`, creating the account and the character.
///
/// `item.owner_id` has a foreign key to `player`, and the game thread's world knows a
/// character the store has never heard of. That combination is not a defect: a world
/// character is a live session and a stored character is a row, and the two meet at
/// relog. A grant therefore cannot be stored for a world-only character, and this helper
/// creates the row so the owner exists before the grant does. The first draft of this
/// suite granted to a world-only character and read the failure as a bug in
/// `persist_grant`; it was the foreign key doing its job.
///
/// It goes through the real `create_account` and `create_player` rather than raw SQL, so
/// the test cannot drift from the schema's own constraints: a hand-written insert would
/// keep passing if a column were renamed.
async fn a_stored_player(store: &db::store::Store, name: &str) -> u32 {
    let account = db::accounts::create_account(
        store,
        &db::accounts::NewAccount {
            login: db::credentials::Login::new(name).expect("the character name is a valid login"),
            // A PHC string the schema accepts and nobody can verify against, because no
            // test here logs in. It is not a secret and it is not a credential.
            password: db::credentials::PasswordDigest::from_stored(
                "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHR2YWx1ZQ$\
                 Rw5gFcg3nG1hU0l6mS7Q8b0J1wY0k2x3pQ4rS5tU6vWA"
                    .to_owned(),
            ),
            delete_code: db::credentials::DeleteCode::new("1234567")
                .expect("seven digits is a delete code"),
        },
    )
    .await
    .expect("the account is created");
    let created = db::players::create_player(
        store,
        account,
        &db::players::NewPlayer {
            slot: 0,
            name: db::accounts::Name::new(name).expect("the name is a valid character name"),
            job: 1,
            st: 6,
            ht: 7,
            dx: 5,
            iq: 5,
            hp: 500,
            sp: 100,
            stamina: 100,
            part_base: 0,
            x: 0,
            y: 0,
        },
    )
    .await
    .expect("the character is created");
    match created {
        db::players::Created::Player(id) => id,
        db::players::Created::Taken => panic!("a fresh account has a free slot and a free name"),
    }
}

/// A `GrantOutcome` for a row that was not produced by this process's world.
///
/// Only the store behaviour is under test where this is used, not the placement, so the
/// row is built directly and the record is derived from it. The store keeps `pos` as
/// `u32` and the wire cell is a `WORD`; a real grant derives both from one number, so
/// the narrowing here cannot be wrong for a row the world chose, and asserting it rather
/// than casting keeps a hand-built row from describing a cell the client cannot address.
fn outcome_for(row: db::items::ItemRow) -> prodomo::item_grant::GrantOutcome {
    let cell = protocol::item_pos::ItemPos {
        window_type: row.window_type,
        cell: u16::try_from(row.pos).expect("a world-chosen cell is a WORD"),
    };
    prodomo::item_grant::GrantOutcome {
        record: protocol::gc_item_window::GcItemSet {
            cell,
            vnum: row.vnum,
            count: row.count,
            refine_element: row.refine_element,
            transmutation: row.transmutation,
            flags: row.flags,
            anti_flags: row.anti_flags,
            highlight: 0,
            sockets: row.sockets,
            attributes: row
                .attributes
                .map(|attribute| protocol::gc_item_window::ItemAttribute {
                    b_type: attribute.b_type,
                    s_value: attribute.s_value,
                }),
        },
        count: row.count,
        row,
        bank: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_written_row_is_in_the_table_and_the_world_is_untouched() {
    // Given: a store with the schema up, and a real world on the real game thread.
    let Some(database) = ScratchDatabase::create_async().await else {
        return;
    };
    let store = database.store().await;
    let owner = a_stored_player(&store, "Shaman").await;
    let mut state = GameState::new(
        gamedata::item_proto::ItemProtos::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto"),
        )
        .expect("the owner's item protos load"),
    );
    state
        .install_item_ids(world::item::ItemIdRange::new(1, 1_000_000, 1).expect("a range"))
        .expect("the first install");
    state
        .characters_mut()
        .create_player(owner, "Shaman")
        .expect("the name is free");
    let game_loop = spawn_game_loop(GameLoopConfig::default(), state).unwrap();
    let controller = game_loop.controller();

    // When: a grant is made and its row is written.
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        controller.request_grant(prodomo::item_grant::GrantRequest {
            target: "Shaman".to_owned(),
            vnum: a_plain_vnum(),
            count: None,
        }),
    )
    .await
    .expect("the game thread answered within five seconds")
    .expect("the answer crossed back")
    .expect("a refusal is not expected");
    let id = outcome.row.id;
    let cell = outcome.row.pos;

    let persisted = tokio::time::timeout(
        Duration::from_secs(15),
        persist_grant(&store, &controller, "Shaman", outcome),
    )
    .await
    .expect("the write finished within fifteen seconds");

    // Then: it is written, with the owner and cell the world chose.
    assert!(
        matches!(persisted, prodomo::item_persist::Persisted::Written),
        "{persisted}"
    );
    let read = db::items::load_item(&store, id)
        .await
        .expect("the read")
        .expect("the row is stored");
    assert_eq!(
        read.owner_id,
        Some(owner),
        "the row names the granting character"
    );
    assert_eq!(read.pos, cell, "the row names the cell the world chose");
    assert_eq!(
        read.window_type,
        common::item_slots::EWindows::Inventory as u8
    );
    assert_eq!(read.count, 1);

    // And: the world still holds it, because a successful write must not revoke. The
    // probe is a revoke, not a question: `Ok` is the positive answer, and it is the only
    // way to ask "does the world still hold this id" without a second query interface
    // that does not exist yet. The revoke is undone immediately after, so the world is
    // back where the test found it.
    let revoked = tokio::time::timeout(
        Duration::from_secs(5),
        controller.revoke_grant("Shaman", id),
    )
    .await
    .expect("the revoke answered");
    assert_eq!(
        revoked,
        Ok(()),
        "the world does not hold the item, so a successful write disturbed it"
    );

    let _ = controller.request_stop().await;
    let _ = game_loop.join().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_duplicate_id_is_refused_and_leaves_the_held_row_alone() {
    // Given: a store where another character already holds an id, and a real world that
    // has just been handed that same id by a fresh allocator.
    let Some(database) = ScratchDatabase::create_async().await else {
        return;
    };
    let store = database.store().await;
    let owner = a_stored_player(&store, "Shaman").await;
    let mut state = GameState::new(
        gamedata::item_proto::ItemProtos::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto"),
        )
        .expect("the owner's item protos load"),
    );
    // The allocator does not know about stored rows, which is exactly the collision this
    // test needs: a second write for the same id is what a restarted process with a
    // fresh range would produce.
    state
        .install_item_ids(world::item::ItemIdRange::new(1, 1_000_000, 1).expect("a range"))
        .expect("the first install");
    state
        .characters_mut()
        .create_player(owner, "Shaman")
        .expect("the name is free");
    let game_loop = spawn_game_loop(GameLoopConfig::default(), state).unwrap();
    let controller = game_loop.controller();

    // When: the first grant is granted and written.
    let first = tokio::time::timeout(
        Duration::from_secs(5),
        controller.request_grant(prodomo::item_grant::GrantRequest {
            target: "Shaman".to_owned(),
            vnum: a_plain_vnum(),
            count: None,
        }),
    )
    .await
    .expect("the game thread answered")
    .expect("the answer crossed back")
    .expect("a refusal is not expected");
    let held_id = first.row.id;
    let held_vnum = first.row.vnum;
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(15),
            persist_grant(&store, &controller, "Shaman", first)
        )
        .await
        .expect("the write finished"),
        prodomo::item_persist::Persisted::Written
    ));

    // And: a second grant is written under the *same* id, as a restarted process with a
    // fresh allocator would do.
    let mut collider = first_row_for(&store, held_id).await;
    collider.owner_id = Some(owner);
    collider.vnum = held_vnum.wrapping_add(1);
    collider.count = 7;
    let colliding = outcome_for(collider);
    let persisted = tokio::time::timeout(
        Duration::from_secs(15),
        persist_grant(&store, &controller, "Shaman", colliding),
    )
    .await
    .expect("the write finished within fifteen seconds");

    // Then: the write is refused, and the error names the id.
    match persisted {
        prodomo::item_persist::Persisted::Undone {
            error: db::items::ItemError::ItemIdAlreadyStored { id },
        } => assert_eq!(id, held_id),
        other => panic!("a duplicate id must be refused by id, got {other}"),
    }

    // And: the first row is untouched. This is the assertion that separates an insert
    // from an upsert. `save_item`'s `ON CONFLICT (id) DO UPDATE` would have left the
    // owner at 22, the count at 7, and reported success.
    let read = db::items::load_item(&store, held_id)
        .await
        .expect("the read")
        .expect("the row is still there");
    assert_eq!(read.owner_id, Some(owner), "the held item changed hands");
    assert_eq!(read.vnum, held_vnum, "the held item was overwritten");
    assert_eq!(read.count, 1, "the held stack was changed");

    let _ = controller.request_stop().await;
    let _ = game_loop.join().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_taken_cell_is_reported_as_a_taken_cell_and_not_as_a_taken_id() {
    // Given: a store with a row already in a cell.
    let Some(database) = ScratchDatabase::create_async().await else {
        return;
    };
    let store = database.store().await;
    let owner = a_stored_player(&store, "Shaman").await;
    let mut held = db::items::ItemRow::on_ground(31_000, 0, 30_000, 1);
    held.owner_id = Some(owner);
    held.window_type = common::item_slots::EWindows::Inventory as u8;
    held.pos = 0;
    db::items::insert_item(&store, &held)
        .await
        .expect("the first row");

    // When: a second item, with a fresh id, is written into that same cell.
    let mut collider = held.clone();
    collider.id = 31_001;
    collider.vnum = 30_001;
    let refused = db::items::insert_item(&store, &collider).await;

    // Then: the refusal names the cell, and does not blame the id.
    //
    // The first draft of `classify_insert` matched the SQLSTATE alone and reported every
    // unique violation as `ItemIdAlreadyStored`, so this exact write was logged as "an
    // item with id 31001 is already stored" -- pointing an Operator at an id that nothing
    // has ever used. The id is reported here only as the row that was refused.
    match refused {
        Err(db::items::ItemError::CellAlreadyTaken {
            id,
            window_type,
            pos,
        }) => {
            assert_eq!(id, 31_001, "the refused row's id is reported, not blamed");
            assert_eq!(window_type, common::item_slots::EWindows::Inventory as u8);
            assert_eq!(pos, 0);
        }
        other => panic!("a taken cell must be its own error, got {other:?}"),
    }

    // And: the first row is still the one in that cell.
    let read = db::items::load_item(&store, 31_000)
        .await
        .expect("the read")
        .expect("the row is still there");
    assert_eq!(read.vnum, 30_000, "the held item was replaced");
    assert_eq!(read.count, 1);
}

// ---------------------------------------------------------------------------
// The delivery order. These are the tests that decide whether `sys.item.core`
// can leave `codec`, so they are written against a real store, a real game
// thread, and a real outbox a test can read back.
// ---------------------------------------------------------------------------

/// A store, a world on its own thread, and a client whose queue the test holds.
///
/// The queue is the observation point. A delivery path that wrote the record somewhere
/// the descriptor does not drain would pass every other test here and reach no client,
/// so the outbox is kept by the test rather than dropped.
struct AWorld {
    store: db::store::Store,
    controller: prodomo::game_loop_messages::GameLoopController,
    inbox: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    #[allow(dead_code)]
    game_loop: prodomo::game_loop::GameLoopHandle,
}

/// A world holding one live character called `name`, who owns a stored row, with a
/// client attached whose outbox this function keeps.
async fn a_world_with_a_client(database: &ScratchDatabase, name: &str) -> AWorld {
    let store = database.store().await;
    let owner = a_stored_player(&store, name).await;
    let mut state = GameState::new(
        gamedata::item_proto::ItemProtos::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto"),
        )
        .expect("the owner's item protos load"),
    );
    state
        .install_item_ids(world::item::ItemIdRange::new(1, 1_000_000, 1).expect("a range"))
        .expect("the first install");
    let (tx, inbox) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    state
        .enter_world(
            common::vid::Vid::new(owner),
            owner,
            name,
            prodomo::client_registry::ClientOutbox::new(tx),
        )
        .expect("the character is admitted");
    let game_loop = spawn_game_loop(GameLoopConfig::default(), state).unwrap();
    let controller = game_loop.controller();
    AWorld {
        store,
        controller,
        inbox,
        game_loop,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_granted_item_is_written_and_then_reaches_the_client() {
    // Given: a live client in the world, with a client queue this test reads.
    let Some(database) = ScratchDatabase::create_async().await else {
        return;
    };
    let AWorld {
        store,
        controller,
        mut inbox,
        ..
    } = a_world_with_a_client(&database, "Shaman").await;

    // When: the whole grant runs, from the world to the socket queue.
    let granted = tokio::time::timeout(
        Duration::from_secs(15),
        prodomo::item_persist::grant_and_deliver(
            &store,
            &controller,
            &prodomo::item_grant::GrantRequest {
                target: "Shaman".to_owned(),
                vnum: a_plain_vnum(),
                count: None,
            },
        ),
    )
    .await
    .expect("the grant finished within fifteen seconds");

    // Then: it was delivered, and the row is really there.
    let id = match granted {
        prodomo::item_persist::Granted::Delivered { id, .. } => id,
        other => panic!("the client must be told, got {other}"),
    };
    let stored = first_row_for(&store, id).await;
    assert_eq!(
        stored.vnum,
        a_plain_vnum(),
        "the row is the item that was granted"
    );

    // And: the record that reached the client is the `GC_ITEM_SET` for that item. Read
    // off the queue rather than trusted from the return value, because the queue is
    // the only thing a client can see.
    let record = inbox.try_recv().expect("the record was queued");
    assert_eq!(record.len(), 72, "TPacketGCItemSet is 72 bytes");
    assert_eq!(record[0], 21, "the header is GC_ITEM_SET");
    // `GC_ITEM_SET` carries **no item id**. Its fields are the window and cell, the
    // vnum, the stack count, the refine and transmutation words, the flags, the six
    // sockets, and the seven attributes -- there is no `dwID` in the wire form, so the
    // first draft of this test looked for the id at offset 1 and read `window_type` and
    // the low half of the cell as one number. The identity of an item on the wire is
    // the *cell*: the client re-reads a window and finds what is in each slot, and the
    // id is a server-side fact that never goes out.
    //
    // So the record is pinned against the row through the codec rather than by a
    // hand-decoded offset, which also keeps the test honest if `ItemPos` ever changes.
    let expected = protocol::item_pos::ItemPos::new(
        stored.window_type,
        u16::try_from(stored.pos).expect("a cell index is a u16"),
    )
    .encode();
    assert_eq!(
        &record[1..=3],
        &expected[..],
        "the record must name the row's cell"
    );
    assert_eq!(
        u32::from_le_bytes([record[4], record[5], record[6], record[7]]),
        stored.vnum,
        "the record must name the vnum the row recorded"
    );
    assert_ne!(
        id, 0,
        "the row keeps its own id even though the wire does not"
    );
}

// ---------------------------------------------------------------------------
// The destroy. The mirror of the grant, and the half that lets `sys.item.core`
// leave `codec`: an item an Operator created must be able to go away again, and
// its row must go with it.
// ---------------------------------------------------------------------------

/// Grant one item to `target` and answer with the id the world took.
async fn grant_one(
    store: &db::store::Store,
    controller: &prodomo::game_loop_messages::GameLoopController,
    target: &str,
) -> u32 {
    match tokio::time::timeout(
        Duration::from_secs(15),
        prodomo::item_persist::grant_and_deliver(
            store,
            controller,
            &prodomo::item_grant::GrantRequest {
                target: target.to_owned(),
                vnum: a_plain_vnum(),
                count: None,
            },
        ),
    )
    .await
    .expect("the grant finished within fifteen seconds")
    {
        // `Stored` counts: the point of this helper is that the item exists, and a
        // delivered-but-undeliverable row is a real item that a later test can still
        // destroy. Only `Failed` means there is nothing to work with.
        prodomo::item_persist::Granted::Delivered { id, .. }
        | prodomo::item_persist::Granted::Stored { id, .. } => id,
        failed @ prodomo::item_persist::Granted::Failed { .. } => {
            panic!("the grant should have produced an item, got {failed}")
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_destroyed_item_leaves_the_world_and_takes_its_row_with_it() {
    let Some(database) = ScratchDatabase::create_async().await else {
        return;
    };
    let AWorld {
        store, controller, ..
    } = a_world_with_a_client(&database, "Shaman").await;

    // Given: a granted, delivered, stored item.
    let id = grant_one(&store, &controller, "Shaman").await;
    assert!(
        db::items::load_item(&store, id)
            .await
            .expect("the read")
            .is_some(),
        "the row is there before the destroy"
    );

    // When: it is destroyed.
    let destroyed = tokio::time::timeout(
        Duration::from_secs(15),
        prodomo::item_persist::destroy_and_delete(&store, &controller, "Shaman", id),
    )
    .await
    .expect("the destroy finished within fifteen seconds");

    // Then: both halves are gone, and the answer says which cell was freed so an
    // Operator is not left guessing.
    let cell = match destroyed {
        prodomo::item_persist::Destroyed::Gone { id: gone, cell } => {
            assert_eq!(gone, id, "the answer names the item that was destroyed");
            cell
        }
        other => panic!("the item and its row should both be gone, got {other:?}"),
    };
    assert_eq!(
        cell.0,
        common::item_slots::EWindows::Inventory as u8,
        "a grant puts the item in the base inventory"
    );
    assert!(
        db::items::load_item(&store, id)
            .await
            .expect("the read")
            .is_none(),
        "the row must be gone, or the item comes back at the next login"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_destroyed_id_cannot_be_destroyed_again_and_does_not_touch_another_character() {
    // The point of this one is the `owner_id` in `destroy_item`'s `WHERE` clause. An id
    // the caller has forgotten is the ordinary Operator mistake, and the failure mode it
    // must not have is deleting a row that has since been granted to somebody else.
    let Some(database) = ScratchDatabase::create_async().await else {
        return;
    };
    let AWorld {
        store, controller, ..
    } = a_world_with_a_client(&database, "Shaman").await;
    let other = a_stored_player(&store, "Warrior").await;
    let _ = other;

    let id = grant_one(&store, &controller, "Shaman").await;
    let destroyed = tokio::time::timeout(
        Duration::from_secs(15),
        prodomo::item_persist::destroy_and_delete(&store, &controller, "Shaman", id),
    )
    .await
    .expect("the first destroy finished");
    assert!(
        matches!(destroyed, prodomo::item_persist::Destroyed::Gone { .. }),
        "the first destroy should succeed, got {destroyed:?}"
    );

    // And: the second one is refused by the world, because the world no longer holds
    // the id, and the store is never asked.
    let again = tokio::time::timeout(
        Duration::from_secs(15),
        prodomo::item_persist::destroy_and_delete(&store, &controller, "Shaman", id),
    )
    .await
    .expect("the second destroy answered");
    match again {
        prodomo::item_persist::Destroyed::Refused { id: named, .. } => {
            assert_eq!(named, id, "the refusal names the id that was asked for");
        }
        other => panic!("a second destroy must be refused, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_destroy_for_a_character_that_is_not_online_never_asks_the_store() {
    // The order has to be observable, not just intended: if the store were asked first,
    // an offline destroy would delete a row for an item a live character is holding,
    // and that item would vanish at the next login with no refusal anywhere.
    let Some(database) = ScratchDatabase::create_async().await else {
        return;
    };
    let AWorld {
        store, controller, ..
    } = a_world_with_a_client(&database, "Shaman").await;
    let id = grant_one(&store, &controller, "Shaman").await;

    let refused = tokio::time::timeout(
        Duration::from_secs(15),
        prodomo::item_persist::destroy_and_delete(&store, &controller, "Ghost", id),
    )
    .await
    .expect("the destroy answered");
    assert!(
        matches!(refused, prodomo::item_persist::Destroyed::Refused { .. }),
        "a character that is not online is refused, got {refused:?}"
    );

    // The row is untouched, which is the whole claim.
    assert!(
        db::items::load_item(&store, id)
            .await
            .expect("the read")
            .is_some(),
        "a refused destroy must leave the row alone"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_destroy_sends_nothing_to_the_client_and_says_so() {
    // The byte-20 question is open, so this pins the current behaviour rather than
    // blessing it: no record goes out, and the console's own answer tells the Operator
    // the cell stays drawn until the next login. When the owner answers the question and
    // a record is added, this test is the one that must change, and it must change on
    // purpose.
    let Some(database) = ScratchDatabase::create_async().await else {
        return;
    };
    let AWorld {
        store,
        controller,
        mut inbox,
        ..
    } = a_world_with_a_client(&database, "Shaman").await;

    // Drain the grant's `GC_ITEM_SET` so this test is looking at what the destroy adds.
    let _ = grant_one(&store, &controller, "Shaman").await;
    while inbox.try_recv().is_ok() {}

    let id = grant_one(&store, &controller, "Shaman").await;
    // Drained again, because this grant delivered a record of its own. The first draft
    // of this test drained only once, and then failed on the *grant's* `GC_ITEM_SET`
    // rather than on anything the destroy sent -- which would have let a destroy that
    // sent a record pass, if the assertion had been written a little more loosely.
    while inbox.try_recv().is_ok() {}

    let _ = tokio::time::timeout(
        Duration::from_secs(15),
        prodomo::item_persist::destroy_and_delete(&store, &controller, "Shaman", id),
    )
    .await
    .expect("the destroy finished");

    assert!(
        inbox.try_recv().is_err(),
        "no record may reach the client: the only record that clears a window slot is \
         byte 20, which the stock client drops, and sending it would reproduce a Defect"
    );
}
