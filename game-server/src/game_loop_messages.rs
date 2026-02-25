//! Typed messages crossing between Tokio and the synchronous game loop.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::Thread;

use tokio::sync::{mpsc, oneshot};

/// Default capacity of the Tokio-to-game command queue.
pub const DEFAULT_COMMAND_CAPACITY: usize = 256;

/// Default capacity of the game-to-Tokio effect queue.
pub const DEFAULT_EFFECT_CAPACITY: usize = 256;

/// Default maximum commands drained at the start of each pulse.
pub const DEFAULT_MAX_COMMANDS_PER_PULSE: usize = 64;

/// Bounded channel and command-drain settings for a game-loop thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GameLoopConfig {
    command_capacity: NonZeroUsize,
    effect_capacity: NonZeroUsize,
    max_commands_per_pulse: NonZeroUsize,
}

impl GameLoopConfig {
    /// Creates a configuration, returning `None` if a bound is invalid.
    ///
    /// The command drain must be between one and
    /// [`DEFAULT_MAX_COMMANDS_PER_PULSE`] inclusive.
    pub const fn new(
        command_capacity: usize,
        effect_capacity: usize,
        max_commands_per_pulse: usize,
    ) -> Option<Self> {
        if max_commands_per_pulse > DEFAULT_MAX_COMMANDS_PER_PULSE {
            return None;
        }
        let Some(command_capacity) = NonZeroUsize::new(command_capacity) else {
            return None;
        };
        let Some(effect_capacity) = NonZeroUsize::new(effect_capacity) else {
            return None;
        };
        let Some(max_commands_per_pulse) = NonZeroUsize::new(max_commands_per_pulse) else {
            return None;
        };
        Some(Self {
            command_capacity,
            effect_capacity,
            max_commands_per_pulse,
        })
    }

    pub(crate) const fn command_capacity(self) -> NonZeroUsize {
        self.command_capacity
    }

    pub(crate) const fn effect_capacity(self) -> NonZeroUsize {
        self.effect_capacity
    }

    pub(crate) const fn max_commands_per_pulse(self) -> NonZeroUsize {
        self.max_commands_per_pulse
    }
}

impl Default for GameLoopConfig {
    fn default() -> Self {
        Self {
            command_capacity: NonZeroUsize::new(DEFAULT_COMMAND_CAPACITY)
                .unwrap_or(NonZeroUsize::MIN),
            effect_capacity: NonZeroUsize::new(DEFAULT_EFFECT_CAPACITY)
                .unwrap_or(NonZeroUsize::MIN),
            max_commands_per_pulse: NonZeroUsize::new(DEFAULT_MAX_COMMANDS_PER_PULSE)
                .unwrap_or(NonZeroUsize::MIN),
        }
    }
}

/// Correlates an asynchronous completion with the request known by game state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CompletionId(u64);

impl CompletionId {
    /// Creates a completion identifier.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the wire-independent numeric value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Result of asynchronous work completed on the Tokio side.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncCompletionStatus {
    /// The asynchronous operation completed successfully.
    Succeeded,
    /// The asynchronous operation completed with an error.
    Failed,
}

/// Commands accepted by the synchronous game loop at pulse boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameCommand {
    /// Applies a typed completion produced by Tokio-owned work.
    ApplyAsyncCompletion {
        /// Correlation identifier of the completed operation.
        id: CompletionId,
        /// Typed completion outcome.
        status: AsyncCompletionStatus,
    },
    /// Requests terminal loop shutdown.
    Stop,
}

/// Cloneable Tokio-side command endpoint for the game thread.
#[derive(Clone, Debug)]
pub struct GameLoopController {
    command_tx: mpsc::Sender<GameCommand>,
    stop_requested: Arc<AtomicBool>,
    game_thread: Thread,
}

impl GameLoopController {
    pub(crate) fn new(
        command_tx: mpsc::Sender<GameCommand>,
        stop_requested: Arc<AtomicBool>,
        game_thread: Thread,
    ) -> Self {
        Self {
            command_tx,
            stop_requested,
            game_thread,
        }
    }

    fn signal_stop(&self) {
        self.stop_requested.store(true, Ordering::SeqCst);
        self.game_thread.unpark();
    }

    /// Sends data through the bounded FIFO queue or signals priority stop.
    ///
    /// # Errors
    ///
    /// Returns the unsent command if the game thread has closed its receiver.
    pub async fn send_command(
        &self,
        command: GameCommand,
    ) -> Result<(), mpsc::error::SendError<GameCommand>> {
        match command {
            GameCommand::Stop if self.command_tx.is_closed() => {
                Err(mpsc::error::SendError(GameCommand::Stop))
            }
            GameCommand::Stop => {
                self.signal_stop();
                Ok(())
            }
            GameCommand::ApplyAsyncCompletion { id, status } => {
                self.command_tx
                    .send(GameCommand::ApplyAsyncCompletion { id, status })
                    .await
            }
        }
    }

    /// Attempts to send without waiting when the command queue is full.
    ///
    /// # Errors
    ///
    /// Returns the unsent command when the queue is full or its receiver is closed.
    pub fn try_send_command(
        &self,
        command: GameCommand,
    ) -> Result<(), mpsc::error::TrySendError<GameCommand>> {
        match command {
            GameCommand::Stop if self.command_tx.is_closed() => {
                Err(mpsc::error::TrySendError::Closed(GameCommand::Stop))
            }
            GameCommand::Stop => {
                self.signal_stop();
                Ok(())
            }
            GameCommand::ApplyAsyncCompletion { id, status } => self
                .command_tx
                .try_send(GameCommand::ApplyAsyncCompletion { id, status }),
        }
    }

    /// Requests idempotent priority stop and wakes the parked game thread.
    ///
    /// Data commands remain FIFO and bounded. Stop is observed before queued
    /// data, so commands not processed before shutdown are discarded. A pulse
    /// already executing cannot be preempted, but stop is checked between pulses.
    ///
    /// # Errors
    ///
    /// Returns the stop command if the game thread has already closed its receiver.
    pub async fn request_stop(&self) -> Result<(), mpsc::error::SendError<GameCommand>> {
        self.send_command(GameCommand::Stop).await
    }
}

/// Effects emitted by synchronous game state for Tokio-owned processing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameEffect {
    /// Confirms deterministic application of an asynchronous completion.
    AsyncCompletionApplied {
        /// Correlation identifier of the applied completion.
        id: CompletionId,
        /// Applied completion outcome.
        status: AsyncCompletionStatus,
    },
}

/// Final counters owned by the game-loop thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GameLoopSummary {
    /// Last pulse processed before termination.
    pub final_pulse: u64,
    /// Effects discarded because the bounded output queue was full.
    pub dropped_effects: u64,
    /// Catch-up batches that reached 750 pulses with backlog still remaining.
    pub capped_catch_up_batches: u64,
}

/// Failure propagated from the synchronous thread to the Tokio supervisor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameLoopFailure {
    /// Every Tokio command sender was dropped without a stop request.
    CommandChannelClosed,
    /// The Tokio effect receiver was dropped while an effect was emitted.
    EffectChannelClosed,
    /// Game-loop execution panicked; the payload is normalized to text.
    Panicked(String),
}

/// One-shot terminal acknowledgement sent exactly once by the game thread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameLoopTerminal {
    /// The loop observed the priority stop signal between synchronous pulses.
    Stopped(GameLoopSummary),
    /// The loop terminated after a failure that the supervisor must handle.
    Failed {
        /// Final counters at failure time.
        summary: GameLoopSummary,
        /// Typed reason for termination.
        reason: GameLoopFailure,
    },
}

pub(crate) struct TokioChannelEnds {
    pub(crate) command_tx: mpsc::Sender<GameCommand>,
    pub(crate) effect_rx: mpsc::Receiver<GameEffect>,
    pub(crate) terminal_rx: oneshot::Receiver<GameLoopTerminal>,
}

pub(crate) struct GameThreadChannelEnds {
    pub(crate) command_rx: mpsc::Receiver<GameCommand>,
    pub(crate) effect_tx: mpsc::Sender<GameEffect>,
    pub(crate) terminal_tx: oneshot::Sender<GameLoopTerminal>,
}

pub(crate) fn bounded_channels(
    command_capacity: NonZeroUsize,
    effect_capacity: NonZeroUsize,
) -> (TokioChannelEnds, GameThreadChannelEnds) {
    let (command_tx, command_rx) = mpsc::channel(command_capacity.get());
    let (effect_tx, effect_rx) = mpsc::channel(effect_capacity.get());
    let (terminal_tx, terminal_rx) = oneshot::channel();

    (
        TokioChannelEnds {
            command_tx,
            effect_rx,
            terminal_rx,
        },
        GameThreadChannelEnds {
            command_rx,
            effect_tx,
            terminal_tx,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{
        bounded_channels, CompletionId, GameCommand, GameLoopConfig, GameLoopController,
        DEFAULT_MAX_COMMANDS_PER_PULSE,
    };
    use std::num::NonZeroUsize;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::thread;
    use tokio::sync::mpsc::error::TrySendError;

    fn completion(id: u64) -> GameCommand {
        GameCommand::ApplyAsyncCompletion {
            id: CompletionId::new(id),
            status: super::AsyncCompletionStatus::Succeeded,
        }
    }

    #[test]
    fn config_rejects_a_command_drain_above_the_hard_limit() {
        // Given: a requested command drain above the fixed per-pulse bound.
        let requested = DEFAULT_MAX_COMMANDS_PER_PULSE + 1;

        // When: the public game-loop configuration validates the bounds.
        let config = GameLoopConfig::new(1, 1, requested);

        // Then: the configuration cannot weaken the hard 64-command limit.
        assert!(config.is_none());
    }

    #[test]
    fn bounded_command_queue_reports_full_without_blocking() {
        // Given: a one-slot command queue whose receiver remains connected.
        let one = NonZeroUsize::MIN;
        let (tokio_ends, _thread_ends) = bounded_channels(one, one);
        let controller = GameLoopController::new(
            tokio_ends.command_tx,
            Arc::new(AtomicBool::new(false)),
            thread::current(),
        );

        // When: a second command is attempted without draining the first.
        controller.try_send_command(completion(1)).unwrap();
        let result = controller.try_send_command(completion(2));

        // Then: bounded backpressure is reported synchronously.
        assert!(matches!(result, Err(TrySendError::Full(_))));
    }
}
