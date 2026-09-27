//! Typed messages crossing between Tokio and the synchronous game loop.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::Thread;

use tokio::sync::{mpsc, oneshot};

use crate::item_grant::{GrantOutcome, GrantRefusal, GrantRequest};

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
///
/// This is neither `Copy` nor `Clone`, and that is a decision rather than an
/// accident. The first version was `Copy` because every payload was a `u64` and an
/// enum, and `Copy` is the right thing for a value that is inspected and then
/// handed on: the controller matches a command to decide whether it is a priority
/// stop and then puts the same value in the channel. [`GameCommand::GrantItem`]
/// carries a [`GrantRequest`] and a [`oneshot::Sender`], and a `oneshot::Sender` is
/// neither `Copy` nor `Clone` -- it is the only handle to its answer, which is
/// exactly why the answer can be correlated without a bookkeeping table. Rather
/// than box the payload to keep the old derives, the derives went and the
/// controller was changed to hand over the command it was given.
///
/// [`GrantRequest`]: crate::item_grant::GrantRequest
#[derive(Debug)]
pub enum GameCommand {
    /// Applies a typed completion produced by Tokio-owned work.
    ApplyAsyncCompletion {
        /// Correlation identifier of the completed operation.
        id: CompletionId,
        /// Typed completion outcome.
        status: AsyncCompletionStatus,
    },
    /// Gives an item to an online character, on the thread that owns the world.
    ///
    /// The command carries its own answer channel rather than reporting through
    /// [`GameEffect`]. Two reasons, and the second is the one that decided it.
    /// First, the answer is per-request: a shared effect queue would need a
    /// correlation id and a map of pending waiters, which is the bookkeeping a
    /// `oneshot` already does. Second, the answer is not [`GameEffect`]-shaped:
    /// it carries a store row and a client record, neither of which is `Copy`, and
    /// an effect that can fail to be delivered is not an answer a caller may act
    /// on. If the game thread drops this command without replying, the sender is
    /// closed and the caller learns there is no answer, which is the truth.
    GrantItem {
        /// What to give and to whom.
        request: GrantRequest,
        /// Where the game thread sends the outcome.
        ///
        /// Closed, not sent, when the world could not act on the request at all.
        /// The caller turns a closed channel into its own error rather than
        /// treating it as a refusal, because a refusal is an answer and a closed
        /// channel is a missing one.
        reply: oneshot::Sender<Result<GrantOutcome, GrantRefusal>>,
    },
    /// Installs the world's item id allocator.
    ///
    /// The range cannot be a constructor argument: its start id is `MAX(id)` over
    /// the item table, and that table is only readable once the store has migrated,
    /// which happens inside the accept loop after the listeners are bound. Sending
    /// it as a command is what lets the world be built, and the ports be opened,
    /// before that fact exists, without either step inventing a range.
    ///
    /// The answer is a `bool` rather than the [`AlreadyInstalled`](crate::game_state::AlreadyInstalled)
    /// error because the caller is `serve`, which treats a second install as a bug
    /// it must not paper over; the error type stays for callers that want it.
    InstallItemIdRange {
        /// The range, already resolved against the store.
        range: world::item::ItemIdRange,
        /// Where the game thread reports whether it installed.
        ///
        /// Closed, not sent, when the world could not act on the request at all.
        reply: oneshot::Sender<bool>,
    },
    /// Takes a granted item back out of the world after its row could not be written.
    ///
    /// The world is mutated before the row exists, because only the world can choose a
    /// cell and the cell is part of the row. This command is the other half of that
    /// arrangement: it runs on the thread that owns the world, so the cell is freed
    /// where it was taken and nowhere else.
    ///
    /// The answer is a `bool` for the same reason [`GameCommand::InstallItemIdRange`]'s
    /// is: a world that cannot take an item back is a bug, and the caller must be told
    /// rather than left to assume the repair worked.
    RevokeGrant {
        /// The character the item was granted to.
        target: String,
        /// The id the grant took.
        id: u32,
        /// Where the game thread reports whether it removed the item.
        reply: oneshot::Sender<bool>,
    },
    /// Requests terminal loop shutdown.
    Stop,
}

/// Why a command did not reach the game thread.
///
/// The previous signature returned `tokio`'s own `SendError<GameCommand>` and
/// `TrySendError<GameCommand>`, which handed the whole command back to the caller.
/// That stopped being the right shape the moment a command owned a reply channel:
/// the caller that got its command back could recover a `oneshot::Sender` it has no
/// use for, and a caller that only wanted to know "was this delivered" had to know
/// Tokio's error tree to ask. These say what happened and nothing else.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandSendError {
    /// The bounded queue was full. The command was not sent and is still the
    /// caller's to retry.
    QueueFull,
    /// The game thread has closed its receiver. No retry will ever be delivered,
    /// because the loop has ended. Not transient, and not a back-pressure signal.
    Closed,
}

impl std::fmt::Display for CommandSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull => write!(formatter, "the game thread command queue is full"),
            Self::Closed => write!(formatter, "the game thread has stopped accepting commands"),
        }
    }
}

impl std::error::Error for CommandSendError {}

/// Why a grant request did not produce an answer.
///
/// Neither variant is a [`GrantRefusal`]. A refusal is the world saying "no", and
/// the caller can act on it. These are the request never being asked, and the
/// answer being lost, and both leave the world exactly as it was.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrantError {
    /// The command never reached the game thread.
    NotSent,
    /// The command was sent and the game thread dropped it without answering,
    /// which is what a shutdown between the send and the next drain looks like.
    NoAnswer,
}

impl std::fmt::Display for GrantError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSent => write!(formatter, "the grant command was not delivered"),
            Self::NoAnswer => write!(formatter, "the game thread gave no answer"),
        }
    }
}

impl std::error::Error for GrantError {}

/// Why taking a granted item back out of the world did not succeed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevokeError {
    /// The command never reached the game thread.
    NotSent,
    /// The command was sent and the thread dropped it without answering.
    NoAnswer,
    /// The world did not hold that item for that character.
    ///
    /// A bug rather than a state to recover from: the id came from the grant that
    /// mutated the world moments earlier, and only a pulse in between could have moved
    /// it. A caller that gets this must not report a clean undo.
    NotThere {
        /// The id the revoke named.
        id: u32,
    },
}

impl std::fmt::Display for RevokeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSent => write!(formatter, "the revoke command was not delivered"),
            Self::NoAnswer => write!(formatter, "the game thread gave no revoke answer"),
            Self::NotThere { id } => {
                write!(
                    formatter,
                    "the world does not hold item {id} for that character"
                )
            }
        }
    }
}

impl std::error::Error for RevokeError {}

/// Why installing the item id allocator did not succeed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallError {
    /// The command never reached the game thread.
    NotSent,
    /// The command was sent and the thread dropped it without answering.
    NoAnswer,
    /// The world already had an allocator.
    ///
    /// A bug, not a state to recover from: a second allocator starts again at the
    /// same id and reissues ids live items hold.
    AlreadyInstalled,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSent => write!(formatter, "the install command was not delivered"),
            Self::NoAnswer => write!(formatter, "the game thread gave no install answer"),
            Self::AlreadyInstalled => {
                write!(formatter, "the world already has an item id allocator")
            }
        }
    }
}

impl std::error::Error for InstallError {}

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
    /// `Stop` never enters the queue: it sets the priority flag and unparks the
    /// thread, which is what lets a shutdown interrupt a batch of queued work. The
    /// data commands go through the bounded channel, so a caller waits when the
    /// queue is full.
    ///
    /// # Errors
    ///
    /// Returns the command, unsent, if the game thread has closed its receiver.
    pub async fn send_command(&self, command: GameCommand) -> Result<(), CommandSendError> {
        if let GameCommand::Stop = command {
            return if self.command_tx.is_closed() {
                Err(CommandSendError::Closed)
            } else {
                self.signal_stop();
                Ok(())
            };
        }
        // The command is matched by reference rather than moved into a rebuilt
        // value. The previous version rebuilt `GameCommand::ApplyAsyncCompletion`
        // from its own fields, which was free while every variant was `Copy` and
        // would have been a second move of a `GrantRequest` and a reply channel.
        self.command_tx
            .send(command)
            .await
            .map_err(|_| CommandSendError::Closed)
    }

    /// Asks the game thread to give an item, and waits for its answer.
    ///
    /// This is the whole crossing in one call: the request travels to the thread
    /// that owns the world, the world is mutated there and nowhere else, and the
    /// caller gets the refusal or the grant back.
    ///
    /// # Errors
    ///
    /// [`GrantError::NotSent`] when the game thread has closed its receiver, and
    /// [`GrantError::NoAnswer`] when the thread closed the reply without sending,
    /// which is what a shutdown between the send and the drain looks like. Neither
    /// is a [`GrantRefusal`], and neither leaves the world changed.
    pub async fn request_grant(
        &self,
        request: GrantRequest,
    ) -> Result<Result<GrantOutcome, GrantRefusal>, GrantError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GrantItem { request, reply })
            .await
            .map_err(|_| GrantError::NotSent)?;
        answer.await.map_err(|_| GrantError::NoAnswer)
    }

    /// Gives the world its item id allocator, and waits for the answer.
    ///
    /// # Errors
    ///
    /// [`InstallError::NotSent`] when the game thread has closed its receiver and
    /// [`InstallError::NoAnswer`] when it closed the reply without sending. The
    /// caller must not open the ready gate on either: a world with no allocator
    /// refuses every grant, so a silently missing install is a server that looks up
    /// and cannot give anything out.
    pub async fn install_item_ids(
        &self,
        range: world::item::ItemIdRange,
    ) -> Result<Result<(), InstallError>, InstallError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::InstallItemIdRange { range, reply })
            .await
            .map_err(|_| InstallError::NotSent)?;
        if answer.await.map_err(|_| InstallError::NoAnswer)? {
            Ok(Ok(()))
        } else {
            Ok(Err(InstallError::AlreadyInstalled))
        }
    }

    /// Takes a granted item back out of the world, and waits for the answer.
    ///
    /// # Errors
    ///
    /// [`RevokeError::NotSent`] when the game thread has closed its receiver,
    /// [`RevokeError::NoAnswer`] when it closed the reply without sending, and
    /// [`RevokeError::NotThere`] when the world does not hold the item. The caller must
    /// treat all three as "the world is still wrong": a player holding an item with no
    /// row sees it vanish at the next login.
    pub async fn revoke_grant(&self, target: &str, id: u32) -> Result<(), RevokeError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::RevokeGrant {
            target: target.to_owned(),
            id,
            reply,
        })
        .await
        .map_err(|_| RevokeError::NotSent)?;
        if answer.await.map_err(|_| RevokeError::NoAnswer)? {
            Ok(())
        } else {
            Err(RevokeError::NotThere { id })
        }
    }

    /// Attempts to send without waiting when the command queue is full.
    ///
    /// # Errors
    ///
    /// Returns the unsent command when the queue is full or its receiver is closed.
    pub fn try_send_command(&self, command: GameCommand) -> Result<(), CommandSendError> {
        if let GameCommand::Stop = command {
            return if self.command_tx.is_closed() {
                Err(CommandSendError::Closed)
            } else {
                self.signal_stop();
                Ok(())
            };
        }
        self.command_tx
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => CommandSendError::QueueFull,
                mpsc::error::TrySendError::Closed(_) => CommandSendError::Closed,
            })
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
    pub async fn request_stop(&self) -> Result<(), CommandSendError> {
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
        bounded_channels, CommandSendError, CompletionId, GameCommand, GameLoopConfig,
        GameLoopController, DEFAULT_MAX_COMMANDS_PER_PULSE,
    };
    use std::num::NonZeroUsize;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::thread;

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
        assert_eq!(result, Err(CommandSendError::QueueFull));
    }
}
