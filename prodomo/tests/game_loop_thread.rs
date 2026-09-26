//! Dedicated-thread lifecycle and failure propagation tests.

use std::thread;
use std::time::{Duration, Instant};

use prodomo::game_loop::{spawn_game_loop, GameLoopConfig, PULSE_PERIOD};
use prodomo::game_loop_messages::{
    AsyncCompletionStatus, CompletionId, GameCommand, GameLoopFailure, GameLoopTerminal,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn game_state_runs_on_a_dedicated_thread_and_acknowledges_stop() {
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

    // Then: state was owned elsewhere, stop was acknowledged, and join completes.
    assert_ne!(game_loop.thread_id(), supervisor_thread);
    assert!(matches!(terminal, GameLoopTerminal::Stopped(_)));
    assert_eq!(game_loop.join().await.unwrap(), terminal);
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
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
    ));
    assert_eq!(game_loop.join().await.unwrap(), terminal);
}
