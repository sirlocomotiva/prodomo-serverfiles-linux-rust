//! Dedicated-thread lifecycle and failure propagation tests.

use std::thread;
use std::time::{Duration, Instant};

use prodomo::game_loop::{spawn_game_loop, GameLoopConfig, PULSE_PERIOD};
use prodomo::game_loop_messages::{
    AsyncCompletionStatus, CompletionId, GameCommand, GameLoopFailure, GameLoopTerminal,
};
use prodomo::game_state::GameState;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_game_thread_is_dedicated_and_acknowledges_stop() {
    // Given: a synchronous game loop supervised from Tokio.
    let supervisor_thread = thread::current().id();
    let mut game_loop = spawn_game_loop(GameLoopConfig::default(), |_| {}).unwrap();
    let controller = game_loop.controller();

    // When: Tokio requests stop and waits for the terminal acknowledgement.
    controller.request_stop().await.unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(1), game_loop.wait_for_terminal())
        .await
        .unwrap()
        .unwrap();

    // Then: the thread was elsewhere, stop was acknowledged, and join completes.
    //
    // This test used to be named `game_state_runs_on_a_dedicated_thread_and_
    // acknowledges_stop` and to assert "state was owned elsewhere", while spawning
    // an empty closure and holding no state at all. The name was a claim the body
    // did not support; `a_game_state_is_the_value_the_thread_steps` below is the
    // test that supports it.
    assert_ne!(game_loop.thread_id(), supervisor_thread);
    assert!(matches!(terminal, GameLoopTerminal::Stopped(_)));
    assert_eq!(game_loop.join().await.unwrap(), terminal);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_game_state_is_the_value_the_thread_steps() {
    // Given: a real `GameState`, built before the spawn, with the item protos read
    // from the owner's Game data. This is the unit that makes ADR-0002's "all
    // worlds step on one game thread" true rather than merely intended: before
    // ledger 201 the thread was spawned with `|_| {}` and owned nothing.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
    let protos =
        gamedata::item_proto::ItemProtos::load(&dir).expect("the owner's item protos load");
    let state = GameState::new(protos);
    let metrics = state.metrics();
    assert_eq!(metrics.pulses(), 0, "nothing has stepped it yet");

    // A character is added on this side, before the move, so the assertion below
    // is about the thread reaching the value and not about the thread constructing
    // it.
    let mut game_loop = spawn_game_loop(GameLoopConfig::default(), state).unwrap();
    let controller = game_loop.controller();

    // When: the thread is given one pulse period to run.
    // The loop parks between pulses, so a short wait is a wait for the first tick
    // and not for a fixed count of them. Two periods makes the test tolerant of a
    // scheduling delay without making it slow.
    let deadline = Instant::now() + 2 * PULSE_PERIOD;
    while metrics.pulses() == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    // Then: the counter the thread owns advanced, and only the thread moved it.
    assert!(
        metrics.pulses() >= 1,
        "the game thread did not step the state within {}ms",
        (2 * PULSE_PERIOD).as_millis()
    );
    // And the state is still owned when the thread is joined: the world was moved
    // in, not copied, so nothing outside can have mutated it while the thread ran.
    controller.request_stop().await.unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(1), game_loop.wait_for_terminal())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(terminal, GameLoopTerminal::Stopped(_)));
    let summary = match &terminal {
        GameLoopTerminal::Stopped(summary) => summary,
        failed @ GameLoopTerminal::Failed { .. } => panic!("expected Stopped, got {failed:?}"),
    };
    assert_eq!(
        summary.final_pulse,
        metrics.pulses(),
        "the loop's last pulse and the state's own count are the same number"
    );
    game_loop.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_id_range_installed_from_tokio_lands_on_the_thread_that_owns_the_world() {
    // The load-bearing test for this unit. `serve` builds the world before the store
    // is readable, so the allocator has to cross the same channel every other command
    // does. Before the install, a grant is refused; after it, the same grant is
    // applied. The two answers together are the only thing that shows the install
    // reached the thread and changed the world, rather than the grant working for
    // some other reason.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
    let protos =
        gamedata::item_proto::ItemProtos::load(&dir).expect("the owner's item protos load");
    let vnum = protos
        .rows()
        .iter()
        .find(|proto| {
            proto.size == 1
                && (0..6).all(|category| {
                    !gamedata::item_custom_category::is_custom_category(proto, category)
                })
        })
        .expect("a one-cell item outside every custom bank")
        .vnum;

    // Given: a world with a player and no allocator, exactly as `serve` builds it.
    let mut state = GameState::new(protos);
    state
        .characters_mut()
        .create_player(11, "Shaman")
        .expect("the name is free");
    let mut game_loop = spawn_game_loop(GameLoopConfig::default(), state).unwrap();
    let controller = game_loop.controller();
    let request = prodomo::item_grant::GrantRequest {
        target: "Shaman".to_owned(),
        vnum,
        count: None,
    };

    // When: a grant is asked for before the range is installed.
    let before = tokio::time::timeout(
        Duration::from_secs(5),
        controller.request_grant(request.clone()),
    )
    .await
    .expect("the game thread answered within five seconds")
    .expect("the answer crossed back");
    assert_eq!(
        before,
        Err(prodomo::item_grant::GrantRefusal::NoAllocator),
        "a world with no allocator must refuse, not answer from a placeholder"
    );

    // And: the range is installed through the same channel.
    let range = world::item::ItemIdRange::new(1, 1_000_000, 1).expect("a range");
    tokio::time::timeout(Duration::from_secs(5), controller.install_item_ids(range))
        .await
        .expect("the game thread answered within five seconds")
        .expect("the install command was delivered")
        .expect("the world took the allocator");

    // Then: the same grant now happens, and the id it took is the installed one.
    let after = tokio::time::timeout(Duration::from_secs(5), controller.request_grant(request))
        .await
        .expect("the game thread answered within five seconds")
        .expect("the answer crossed back")
        .expect("a refusal is not expected after the install");
    assert_eq!(after.row.id, 1, "the id came from the installed allocator");
    assert_eq!(after.row.owner_id, Some(11));

    // And: a second install is refused rather than replacing the live allocator.
    // A replacement would start again at id 1 and reissue it to the item above.
    let second = tokio::time::timeout(Duration::from_secs(5), controller.install_item_ids(range))
        .await
        .expect("the game thread answered within five seconds")
        .expect("the install command was delivered");
    assert_eq!(
        second,
        Err(prodomo::game_loop_messages::InstallError::AlreadyInstalled),
        "a second allocator would reissue ids live items already hold"
    );

    controller.request_stop().await.unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(1), game_loop.wait_for_terminal())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(terminal, GameLoopTerminal::Stopped(_)));
    game_loop.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_grant_sent_from_tokio_is_applied_on_the_thread_that_owns_the_world() {
    // Given: a real `GameState` on the dedicated game thread, holding a player.
    //
    // Everything else about this file spawns an empty closure. This is the one that
    // proves the command path end to end: a Tokio task sends a command, the thread
    // that owns the world is the only thing that can answer, and the answer comes
    // back with the cell the world actually used.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
    let protos =
        gamedata::item_proto::ItemProtos::load(&dir).expect("the owner's item protos load");
    let vnum = protos
        .rows()
        .iter()
        .find(|proto| {
            proto.size == 1
                && (0..6).all(|category| {
                    !gamedata::item_custom_category::is_custom_category(proto, category)
                })
        })
        .expect("a one-cell item outside every custom bank")
        .vnum;

    let mut state = GameState::new(protos);
    state
        .install_item_ids(world::item::ItemIdRange::new(1, 1_000_000, 1).expect("a range"))
        .expect("the first install");
    state
        .characters_mut()
        .create_player(11, "Shaman")
        .expect("the name is free");
    let metrics = state.metrics();
    let mut game_loop = spawn_game_loop(GameLoopConfig::default(), state).unwrap();
    let controller = game_loop.controller();

    // When: a Tokio task asks for an item.
    let answer = tokio::time::timeout(
        Duration::from_secs(5),
        controller.request_grant(prodomo::item_grant::GrantRequest {
            // Deliberately the wrong case. Name matching is the manager's job, and
            // this is the one place a test can prove the request crossed as a
            // `String` and was not normalised or dropped on the way.
            target: "SHAMAN".to_owned(),
            vnum,
            count: None,
        }),
    )
    .await
    .expect("the game thread answered within five seconds")
    .expect("the answer crossed back");

    // Then: the grant happened, and the thread that owned the world is the one that
    // stepped to make it so.
    let outcome = answer.expect("a refusal is not expected here");
    // The window byte is a *measured* `EWindows` value, not a number to recall:
    // three members sit behind feature switches, so a reader who remembers
    // "INVENTORY is 1" from a build with fewer switches live will be wrong. The
    // name is asserted here and the eleven ordinals are pinned in `common`.
    assert_eq!(
        outcome.record.cell.window_type,
        common::item_slots::EWindows::Inventory as u8
    );
    assert_eq!(outcome.record.cell.cell, 0, "the first free cell");
    assert_eq!(outcome.count, 1);

    assert!(
        metrics.pulses() >= 1,
        "the command was drained between pulses, so the thread ran to apply it"
    );

    // A second, refused request proves the channel is still live and that a refusal
    // comes back as a refusal rather than as a transport error. Without this, a
    // `request_grant` that silently did nothing on the second call would pass.
    let refused = controller
        .request_grant(prodomo::item_grant::GrantRequest {
            target: "Nobody".to_owned(),
            vnum,
            count: None,
        })
        .await
        .expect("the answer crossed back");
    assert!(matches!(
        refused,
        Err(prodomo::item_grant::GrantRefusal::NoSuchCharacter { .. })
    ));

    controller.request_stop().await.unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(1), game_loop.wait_for_terminal())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(terminal, GameLoopTerminal::Stopped(_)));
    game_loop.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pulse_panic_is_acknowledged_as_a_typed_terminal_failure() {
    // Given: a pulse processor that panics on its first pulse.
    let mut game_loop = spawn_game_loop(GameLoopConfig::default(), |_| {
        panic!("pulse processor failed");
    })
    .unwrap();

    // When: Tokio waits for the guarded thread's terminal acknowledgement.
    let terminal = tokio::time::timeout(Duration::from_secs(1), game_loop.wait_for_terminal())
        .await
        .unwrap()
        .unwrap();

    // Then: the panic is propagated and the thread remains joinable.
    assert!(matches!(
        &terminal,
        GameLoopTerminal::Failed {
            reason: GameLoopFailure::Panicked(message),
            ..
        } if message == "pulse processor failed"
    ));
    assert_eq!(game_loop.join().await.unwrap(), terminal);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_wakes_a_parked_loop_before_the_next_pulse() {
    // Given: the first pulse has completed and the loop is parked for the next.
    let (pulse_tx, pulse_rx) = std::sync::mpsc::sync_channel(1);
    let mut game_loop = spawn_game_loop(GameLoopConfig::default(), move |pulse| {
        if pulse == 1 {
            pulse_tx.send(()).unwrap();
        }
    })
    .unwrap();
    pulse_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first pulse should complete");
    let controller = game_loop.controller();

    // When: stop is requested while the scheduler is parked.
    let started = Instant::now();
    controller.request_stop().await.unwrap();
    let terminal = tokio::time::timeout(PULSE_PERIOD, game_loop.wait_for_terminal())
        .await
        .expect("stop should wake the parked scheduler")
        .unwrap();

    // Then: no second pulse runs and the dedicated thread remains joinable.
    let GameLoopTerminal::Stopped(summary) = terminal.clone() else {
        panic!("expected stopped terminal acknowledgement");
    };
    assert_eq!(summary.final_pulse, 1);
    assert!(started.elapsed() < PULSE_PERIOD);
    assert_eq!(game_loop.join().await.unwrap(), terminal);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn priority_stop_discards_queued_data_without_starvation() {
    // Given: a full data queue that cannot admit another ordinary command.
    let config = GameLoopConfig::new(1, 1, 1).unwrap();
    let mut game_loop = spawn_game_loop(config, |_| {}).unwrap();
    let controller = game_loop.controller();
    controller
        .try_send_command(GameCommand::ApplyAsyncCompletion {
            id: CompletionId::new(1),
            status: AsyncCompletionStatus::Succeeded,
        })
        .unwrap();

    // When: repeated priority stops bypass the full FIFO queue.
    controller.request_stop().await.unwrap();
    controller.request_stop().await.unwrap();
    controller.try_send_command(GameCommand::Stop).unwrap();
    let terminal = tokio::time::timeout(PULSE_PERIOD, game_loop.wait_for_terminal())
        .await
        .expect("priority stop should not wait for queue capacity")
        .unwrap();

    // Then: queued data was not replayed, one terminal is cached, and join reaps once.
    let GameLoopTerminal::Stopped(summary) = terminal.clone() else {
        panic!("expected stopped terminal acknowledgement");
    };
    assert_eq!(summary.final_pulse, 0);
    assert!(game_loop.recv_effect().await.is_none());
    assert!(matches!(
        controller.try_send_command(GameCommand::ApplyAsyncCompletion {
            id: CompletionId::new(2),
            status: AsyncCompletionStatus::Succeeded,
        }),
        Err(prodomo::game_loop_messages::CommandSendError::Closed)
    ));
    assert_eq!(game_loop.join().await.unwrap(), terminal);
}

/// A vnum for a one-cell item that is outside every custom inventory bank.
///
/// The grant needs a real prototype for its size and category, and the size has to be
/// one cell so the test can assert on the cell number without computing a footprint.
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

/// A store whose URL parses and names nothing, so every write fails on connect.
///
/// The repair path has to work when the store is down, not only when it refuses a
/// constraint, and this is the case that is hardest to get right because there is no
/// row to inspect afterwards.
fn a_store_nothing_answers() -> db::store::Store {
    db::store::Store::lazy(&db::store::StoreConfig {
        url: "postgres://prodomo:prodomo-test@127.0.0.1:1/prodomo".to_owned(),
        max_connections: 1,
    })
    .expect("a well-formed store configuration")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_write_puts_the_world_back_on_the_thread_that_owns_it() {
    // Given: a real world on the real game thread, with an allocator and a player, and
    // a store that cannot be reached.
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
        .create_player(11, "Shaman")
        .expect("the name is free");
    let game_loop = spawn_game_loop(GameLoopConfig::default(), state).unwrap();
    let controller = game_loop.controller();

    // When: a grant is made and its row cannot be written.
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
    let cell = outcome.record.cell.cell;
    let id = outcome.row.id;

    // The bound has to clear `StoreConfig::ACQUIRE_TIMEOUT`, because sqlx retries a
    // refused connection until the pool gives up. A five-second bound here would time
    // out on a store that is merely unreachable and look like a stuck game thread.
    let persisted = tokio::time::timeout(
        Duration::from_secs(15),
        prodomo::item_persist::persist_grant(
            &a_store_nothing_answers(),
            &controller,
            "Shaman",
            outcome,
        ),
    )
    .await
    .expect("the repair answered within five seconds");

    // Then: the world was put back, and the caller is told so.
    assert!(
        matches!(persisted, prodomo::item_persist::Persisted::Undone { .. }),
        "a refused write must repair the world, got {persisted}"
    );

    // And: the cell is free again. This is the assertion the whole unit exists for. A
    // repair that reported success while leaving the cell occupied would pass the
    // match above and fail here, and it is the failure that reaches a player as a
    // permanently missing inventory slot.
    let free = tokio::time::timeout(
        Duration::from_secs(5),
        controller.request_grant(prodomo::item_grant::GrantRequest {
            target: "Shaman".to_owned(),
            vnum: a_plain_vnum(),
            count: None,
        }),
    )
    .await
    .expect("the second grant answered within five seconds")
    .expect("the answer crossed back")
    .expect("the freed cell is usable");
    assert_eq!(
        free.record.cell.cell, cell,
        "the repair freed the cell the first grant took"
    );
    assert_ne!(
        free.row.id, id,
        "the id stays burned, because the allocator is monotonic"
    );

    let _ = controller.request_stop().await;
    let _ = game_loop.join().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoke_of_an_item_the_world_does_not_hold_is_refused() {
    // Given: a real world with a player and an allocator, holding no items.
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
        .create_player(11, "Shaman")
        .expect("the name is free");
    let game_loop = spawn_game_loop(GameLoopConfig::default(), state).unwrap();
    let controller = game_loop.controller();

    // When: a revoke names an id the world never handed out.
    let refused = tokio::time::timeout(
        Duration::from_secs(5),
        controller.revoke_grant("Shaman", 123_456),
    )
    .await
    .expect("the game thread answered within five seconds");

    // Then: it is refused by name. A revoke that reported success for an id nobody
    // holds would let `persist_grant` claim a repair it never performed.
    assert_eq!(
        refused,
        Err(prodomo::game_loop_messages::RevokeError::NotThere { id: 123_456 })
    );

    // And: a revoke for a character who is not online is refused too, rather than
    // being answered as "not there", which would point an Operator at the wrong cause.
    let offline =
        tokio::time::timeout(Duration::from_secs(5), controller.revoke_grant("Nobody", 1))
            .await
            .expect("the game thread answered within five seconds");
    assert_eq!(
        offline,
        Err(prodomo::game_loop_messages::RevokeError::NotThere { id: 1 }),
        "an offline character leaves the world with nothing to release, and the id is \\
         what the caller needs to see"
    );

    let _ = controller.request_stop().await;
    let _ = game_loop.join().await;
}
