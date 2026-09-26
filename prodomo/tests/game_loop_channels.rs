//! Ordering, backpressure, and terminal channel tests.

use std::time::Duration;

use prodomo::game_loop::{spawn_game_loop, GameLoopConfig};
use prodomo::game_loop_messages::{
    AsyncCompletionStatus, CompletionId, GameCommand, GameEffect, GameLoopTerminal,
};

fn completion(id: u64) -> GameCommand {
    GameCommand::ApplyAsyncCompletion {
        id: CompletionId::new(id),
        status: AsyncCompletionStatus::Succeeded,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn commands_produce_effects_in_fifo_order_at_a_pulse_boundary() {
    // Given: bounded channels containing two FIFO data completions.
    let config = GameLoopConfig::new(2, 2, 2).unwrap();
    let mut game_loop = spawn_game_loop(config, |_| {}).unwrap();
    let controller = game_loop.controller();
    controller.try_send_command(completion(11)).unwrap();
    controller.try_send_command(completion(12)).unwrap();

    // When: the game thread drains one deterministic data-command batch.
    let first = game_loop.recv_effect().await.unwrap();
    let second = game_loop.recv_effect().await.unwrap();
    controller.request_stop().await.unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(1), game_loop.wait_for_terminal())
        .await
        .unwrap()
        .unwrap();

    // Then: output order matches input order and no post-terminal command replays.
    assert_eq!(
        first,
        GameEffect::AsyncCompletionApplied {
            id: CompletionId::new(11),
            status: AsyncCompletionStatus::Succeeded,
        }
    );
    assert_eq!(
        second,
        GameEffect::AsyncCompletionApplied {
            id: CompletionId::new(12),
            status: AsyncCompletionStatus::Succeeded,
        }
    );
    assert!(matches!(terminal, GameLoopTerminal::Stopped(_)));
    assert!(game_loop.recv_effect().await.is_none());
    assert!(matches!(
        controller.try_send_command(completion(13)),
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
    ));
    assert_eq!(game_loop.join().await.unwrap(), terminal);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_effect_channel_is_counted_without_blocking_the_game_thread() {
    // Given: one output slot and two commands processed in the same pulse.
    let config = GameLoopConfig::new(2, 1, 2).unwrap();
    let (pulse_tx, pulse_rx) = std::sync::mpsc::sync_channel(1);
    let mut game_loop = spawn_game_loop(config, move |_| {
        let _ = pulse_tx.try_send(());
    })
    .unwrap();
    let controller = game_loop.controller();
    controller.try_send_command(completion(21)).unwrap();
    controller.try_send_command(completion(22)).unwrap();

    // When: nonblocking effect delivery reaches the bounded output capacity.
    pulse_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first pulse should finish");
    controller.request_stop().await.unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(1), game_loop.wait_for_terminal())
        .await
        .unwrap()
        .unwrap();
    let retained = game_loop.recv_effect().await.unwrap();

    // Then: the first effect remains, one is dropped, and shutdown is acknowledged.
    assert_eq!(
        retained,
        GameEffect::AsyncCompletionApplied {
            id: CompletionId::new(21),
            status: AsyncCompletionStatus::Succeeded,
        }
    );
    let GameLoopTerminal::Stopped(summary) = terminal.clone() else {
        panic!("expected stopped terminal acknowledgement");
    };
    assert_eq!(summary.dropped_effects, 1);
    assert_eq!(game_loop.join().await.unwrap(), terminal);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn command_drain_limit_defers_excess_commands_to_later_pulses() {
    // Given: two completions with a one-command per-pulse drain limit.
    let config = GameLoopConfig::new(2, 2, 1).unwrap();
    let mut game_loop = spawn_game_loop(config, |_| {}).unwrap();
    let controller = game_loop.controller();
    controller.try_send_command(completion(31)).unwrap();
    controller.try_send_command(completion(32)).unwrap();

    // When: both FIFO data commands have crossed separate pulse boundaries.
    let first = game_loop.recv_effect().await.unwrap();
    let second = game_loop.recv_effect().await.unwrap();
    controller.request_stop().await.unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(1), game_loop.wait_for_terminal())
        .await
        .unwrap()
        .unwrap();

    // Then: exactly one completion was drained before each of two pulses.
    let GameLoopTerminal::Stopped(summary) = terminal.clone() else {
        panic!("expected stopped terminal acknowledgement");
    };
    assert_eq!(summary.final_pulse, 2);
    assert_eq!(
        first,
        GameEffect::AsyncCompletionApplied {
            id: CompletionId::new(31),
            status: AsyncCompletionStatus::Succeeded,
        }
    );
    assert_eq!(
        second,
        GameEffect::AsyncCompletionApplied {
            id: CompletionId::new(32),
            status: AsyncCompletionStatus::Succeeded,
        }
    );
    assert_eq!(game_loop.join().await.unwrap(), terminal);
}
