//! Deterministic pulse planning and synchronous game-loop ownership.

use std::any::Any;
use std::fmt;
use std::io;
use std::num::NonZeroUsize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Instant;

use tokio::sync::mpsc;

use crate::game_loop_messages::{
    bounded_channels, GameCommand, GameEffect, GameLoopFailure, GameLoopSummary, GameLoopTerminal,
    GameThreadChannelEnds, TokioChannelEnds,
};
pub use crate::game_loop_messages::{
    GameLoopConfig, GameLoopController, DEFAULT_MAX_COMMANDS_PER_PULSE,
};

mod pulse_planner;
pub use pulse_planner::{PulseBatch, PulsePlanner, MAX_CATCH_UP_PULSES, PULSE_PERIOD};

/// Synchronous pulse work exclusively owned by the dedicated thread.
pub trait PulseProcessor: Send + 'static {
    /// Processes one pulse without awaiting or performing asynchronous I/O.
    fn process_pulse(&mut self, pulse: u64);

    /// Applies one command on the thread that owns the world.
    ///
    /// The loop calls this between pulses and never inside one, so a command that
    /// mutates a world is applied at a point where no other world access is
    /// possible. That is the whole of the threading story: there is no lock, and
    /// the invariant is that only this method and [`Self::process_pulse`] can
    /// reach game state.
    ///
    /// The default drops the command, which closes any reply channel it carried.
    /// That is the honest default for a processor that has no world to act on, and
    /// it is why the error a caller sees is
    /// [`GrantError::NoAnswer`](crate::game_loop_messages::GrantError::NoAnswer) and
    /// not a refusal. A processor that *can* act must override this.
    fn apply_command(&mut self, command: GameCommand) {
        drop(command);
    }
}

impl<F> PulseProcessor for F
where
    F: FnMut(u64) + Send + 'static,
{
    fn process_pulse(&mut self, pulse: u64) {
        self(pulse);
    }
}

/// Failure while supervising acknowledgement or joining the OS thread.
#[derive(Debug)]
pub enum GameLoopSupervisionError {
    /// The thread ended without delivering its terminal acknowledgement.
    AcknowledgementLost,
    /// Tokio could not execute the blocking join adapter.
    JoinTask(tokio::task::JoinError),
    /// The OS thread panicked outside the guarded loop body.
    ThreadPanicked,
}

impl fmt::Display for GameLoopSupervisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AcknowledgementLost => formatter.write_str("game-loop acknowledgement lost"),
            Self::JoinTask(error) => write!(formatter, "game-loop join task failed: {error}"),
            Self::ThreadPanicked => formatter.write_str("game-loop thread panicked while joining"),
        }
    }
}

impl std::error::Error for GameLoopSupervisionError {}

/// Tokio-side ownership of effects, terminal acknowledgement, and thread join.
pub struct GameLoopHandle {
    controller: GameLoopController,
    effect_rx: mpsc::Receiver<GameEffect>,
    terminal_rx: tokio::sync::oneshot::Receiver<GameLoopTerminal>,
    terminal: Option<GameLoopTerminal>,
    thread: JoinHandle<()>,
}

impl GameLoopHandle {
    /// Returns a cloneable command endpoint.
    pub fn controller(&self) -> GameLoopController {
        self.controller.clone()
    }

    /// Returns the dedicated OS thread identifier.
    pub fn thread_id(&self) -> ThreadId {
        self.thread.thread().id()
    }

    /// Receives the next effect asynchronously.
    pub async fn recv_effect(&mut self) -> Option<GameEffect> {
        self.effect_rx.recv().await
    }

    /// Attempts to receive an effect without waiting.
    ///
    /// # Errors
    ///
    /// Returns [`mpsc::error::TryRecvError::Empty`] when no effect is ready, or
    /// [`mpsc::error::TryRecvError::Disconnected`] after the game thread exits.
    pub fn try_recv_effect(&mut self) -> Result<GameEffect, mpsc::error::TryRecvError> {
        self.effect_rx.try_recv()
    }

    /// Waits cancellation-safely for the one-shot terminal acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns [`GameLoopSupervisionError::AcknowledgementLost`] if the game
    /// thread exits without sending its terminal acknowledgement.
    pub async fn wait_for_terminal(
        &mut self,
    ) -> Result<GameLoopTerminal, GameLoopSupervisionError> {
        if let Some(terminal) = &self.terminal {
            return Ok(terminal.clone());
        }
        let terminal = (&mut self.terminal_rx)
            .await
            .map_err(|_| GameLoopSupervisionError::AcknowledgementLost)?;
        self.terminal = Some(terminal.clone());
        Ok(terminal)
    }

    /// Joins through Tokio's blocking adapter and returns the acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns an error if acknowledgement is lost, Tokio cannot complete the
    /// blocking join task, or the dedicated game-loop thread panics.
    pub async fn join(mut self) -> Result<GameLoopTerminal, GameLoopSupervisionError> {
        let terminal = self.wait_for_terminal().await;
        let joined = tokio::task::spawn_blocking(move || self.thread.join())
            .await
            .map_err(GameLoopSupervisionError::JoinTask)?;
        joined.map_err(|_| GameLoopSupervisionError::ThreadPanicked)?;
        terminal
    }
}

/// Starts a named `std::thread` that exclusively owns synchronous pulse state.
///
/// # Errors
///
/// Returns an I/O error if the operating system cannot create the thread.
pub fn spawn_game_loop<P>(config: GameLoopConfig, processor: P) -> io::Result<GameLoopHandle>
where
    P: PulseProcessor,
{
    let (tokio_ends, thread_ends) =
        bounded_channels(config.command_capacity(), config.effect_capacity());
    let TokioChannelEnds {
        command_tx,
        effect_rx,
        terminal_rx,
    } = tokio_ends;
    let stop_requested = Arc::new(AtomicBool::new(false));
    let thread_stop_requested = Arc::clone(&stop_requested);
    let thread = thread::Builder::new()
        .name("game-loop".to_owned())
        .spawn(move || {
            run_guarded(
                thread_ends,
                config.max_commands_per_pulse(),
                thread_stop_requested,
                processor,
            );
        })?;
    let controller = GameLoopController::new(command_tx, stop_requested, thread.thread().clone());
    Ok(GameLoopHandle {
        controller,
        effect_rx,
        terminal_rx,
        terminal: None,
        thread,
    })
}

#[derive(Default)]
struct LoopState {
    pulse: u64,
    dropped_effects: u64,
    capped_catch_up_batches: u64,
}

impl LoopState {
    fn observe_batch(&mut self, batch: PulseBatch) {
        if batch.backlog_remaining {
            self.capped_catch_up_batches = self.capped_catch_up_batches.saturating_add(1);
        }
    }

    const fn summary(&self) -> GameLoopSummary {
        GameLoopSummary {
            final_pulse: self.pulse,
            dropped_effects: self.dropped_effects,
            capped_catch_up_batches: self.capped_catch_up_batches,
        }
    }
}

struct LoopRuntime {
    state: LoopState,
    command_rx: mpsc::Receiver<GameCommand>,
    effect_tx: mpsc::Sender<GameEffect>,
    max_commands_per_pulse: NonZeroUsize,
    stop_requested: Arc<AtomicBool>,
}

enum LoopControl {
    Continue,
    Stop,
    Fail(GameLoopFailure),
}

impl LoopRuntime {
    fn run<P: PulseProcessor>(&mut self, processor: &mut P) -> GameLoopTerminal {
        let started = Instant::now();
        let mut planner = PulsePlanner::new();
        loop {
            if self.stop_requested.load(Ordering::SeqCst) {
                return GameLoopTerminal::Stopped(self.state.summary());
            }
            let elapsed = started.elapsed();
            let batch = planner.due_pulses(elapsed);
            self.state.observe_batch(batch);
            if batch.due == 0 {
                thread::park_timeout(planner.time_until_next(elapsed));
                continue;
            }
            for _ in 0..batch.due {
                if self.stop_requested.load(Ordering::SeqCst) {
                    return GameLoopTerminal::Stopped(self.state.summary());
                }
                match self.drain_commands::<P>(processor) {
                    LoopControl::Continue => {}
                    LoopControl::Stop => return GameLoopTerminal::Stopped(self.state.summary()),
                    LoopControl::Fail(reason) => return self.failed(reason),
                }
                self.state.pulse = self.state.pulse.saturating_add(1);
                processor.process_pulse(self.state.pulse);
            }
        }
    }

    /// Applies up to `max_commands_per_pulse` queued commands.
    ///
    /// `Stop` is the only command the loop answers itself. Everything else goes to
    /// the processor, because only the processor can see the world: the loop holds
    /// channels and counters, and giving it a case per command variant is how the
    /// world would end up in the scheduler instead of in the thread.
    fn drain_commands<P: PulseProcessor>(&mut self, processor: &mut P) -> LoopControl {
        for _ in 0..self.max_commands_per_pulse.get() {
            match self.command_rx.try_recv() {
                Ok(GameCommand::Stop) => return LoopControl::Stop,
                Ok(GameCommand::ApplyAsyncCompletion { id, status }) => {
                    let effect = GameEffect::AsyncCompletionApplied { id, status };
                    match self.effect_tx.try_send(effect) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            self.state.dropped_effects =
                                self.state.dropped_effects.saturating_add(1);
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            return LoopControl::Fail(GameLoopFailure::EffectChannelClosed);
                        }
                    }
                }
                Ok(command) => processor.apply_command(command),
                Err(mpsc::error::TryRecvError::Empty) => return LoopControl::Continue,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return LoopControl::Fail(GameLoopFailure::CommandChannelClosed);
                }
            }
        }
        LoopControl::Continue
    }

    fn failed(&self, reason: GameLoopFailure) -> GameLoopTerminal {
        GameLoopTerminal::Failed {
            summary: self.state.summary(),
            reason,
        }
    }
}

fn run_guarded<P: PulseProcessor>(
    ends: GameThreadChannelEnds,
    max_commands_per_pulse: NonZeroUsize,
    stop_requested: Arc<AtomicBool>,
    mut processor: P,
) {
    let GameThreadChannelEnds {
        command_rx,
        effect_tx,
        terminal_tx,
    } = ends;
    let mut runtime = LoopRuntime {
        state: LoopState::default(),
        command_rx,
        effect_tx,
        max_commands_per_pulse,
        stop_requested,
    };
    let terminal = match catch_unwind(AssertUnwindSafe(|| runtime.run(&mut processor))) {
        Ok(terminal) => terminal,
        Err(payload) => runtime.failed(GameLoopFailure::Panicked(panic_message(payload.as_ref()))),
    };
    drop(terminal_tx.send(terminal));
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        "non-string panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests;
