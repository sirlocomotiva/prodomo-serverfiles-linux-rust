use std::sync::atomic::AtomicBool;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use super::{
    run_guarded, GameLoopHandle, GameLoopSupervisionError, LoopState, PulseBatch,
    MAX_CATCH_UP_PULSES,
};
use crate::game_loop_messages::{
    bounded_channels, GameCommand, GameLoopController, GameLoopFailure, GameLoopTerminal,
};
use std::num::NonZeroUsize;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn join_reaps_the_os_thread_when_acknowledgement_is_lost() {
    // Given: a closed acknowledgement channel and an OS thread still finishing.
    let (command_tx, _command_rx) = mpsc::channel(1);
    let (_effect_tx, effect_rx) = mpsc::channel(1);
    let (terminal_tx, terminal_rx) = oneshot::channel();
    drop(terminal_tx);
    let (finished_tx, finished_rx) = std_mpsc::sync_channel(1);
    let thread = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        finished_tx.send(()).unwrap();
    });
    let stop_requested = Arc::new(AtomicBool::new(false));
    let handle = GameLoopHandle {
        controller: GameLoopController::new(command_tx, stop_requested, thread.thread().clone()),
        effect_rx,
        terminal_rx,
        terminal: None,
        thread,
    };

    // When: Tokio joins the game-loop handle after acknowledgement loss.
    let result = handle.join().await;

    // Then: the acknowledgement error remains, but the OS thread was reaped first.
    assert!(matches!(
        result,
        Err(GameLoopSupervisionError::AcknowledgementLost)
    ));
    assert_eq!(finished_rx.try_recv(), Ok(()));
}

#[test]
fn capped_batch_is_exposed_in_the_terminal_summary() {
    // Given: loop state observes a batch with deadlines still overdue.
    let mut state = LoopState::default();

    // When: the capped batch is recorded.
    state.observe_batch(PulseBatch {
        due: MAX_CATCH_UP_PULSES,
        backlog_remaining: true,
    });

    // Then: terminal evidence retains the non-droppable cap count.
    assert_eq!(state.summary().capped_catch_up_batches, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closed_effect_receiver_becomes_a_typed_terminal_failure() {
    // Given: the Tokio effect receiver is closed before a queued completion.
    let one = NonZeroUsize::MIN;
    let (tokio_ends, thread_ends) = bounded_channels(one, one);
    let controller_tx = tokio_ends.command_tx;
    let terminal_rx = tokio_ends.terminal_rx;
    drop(tokio_ends.effect_rx);
    controller_tx
        .try_send(GameCommand::ApplyAsyncCompletion {
            id: crate::game_loop_messages::CompletionId::new(1),
            status: crate::game_loop_messages::AsyncCompletionStatus::Succeeded,
        })
        .unwrap();
    let stop_requested = Arc::new(AtomicBool::new(false));

    // When: the game thread attempts nonblocking effect delivery.
    let thread = thread::spawn(move || {
        run_guarded(thread_ends, one, stop_requested, |_| {});
    });
    let terminal = terminal_rx.await.unwrap();
    thread.join().unwrap();

    // Then: receiver closure is propagated rather than silently dropping output.
    assert!(matches!(
        terminal,
        GameLoopTerminal::Failed {
            reason: GameLoopFailure::EffectChannelClosed,
            ..
        }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closed_command_channel_becomes_a_typed_terminal_failure() {
    // Given: every command sender is dropped before the game thread starts.
    let one = NonZeroUsize::MIN;
    let (tokio_ends, thread_ends) = bounded_channels(one, one);
    let terminal_rx = tokio_ends.terminal_rx;
    drop(tokio_ends.command_tx);
    drop(tokio_ends.effect_rx);
    let stop_requested = Arc::new(AtomicBool::new(false));

    // When: the first pulse attempts to drain commands.
    let thread = thread::spawn(move || {
        run_guarded(thread_ends, one, stop_requested, |_| {});
    });
    let terminal = terminal_rx.await.unwrap();
    thread.join().unwrap();

    // Then: sender closure is represented as a typed failure.
    assert!(matches!(
        terminal,
        GameLoopTerminal::Failed {
            reason: GameLoopFailure::CommandChannelClosed,
            ..
        }
    ));
}
