//! Typed messages crossing between Tokio and the synchronous game loop.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::Thread;

use tokio::sync::{mpsc, oneshot};

use protocol::item_pos::ItemPos;
use world::character::{Points, Quickslots};
use world::item::Item;
use world::npc::MapNpcs;

use crate::client_registry::ClientOutbox;
use crate::game_state::{EnterWorldRefused, Released, RevokeRefused};
use crate::item_grant::{GrantOutcome, GrantRefusal, GrantRequest};
use crate::item_move::{MoveItemRefused, MovedItems, Mover};
use crate::quickslot::{QuickslotAnswer, QuickslotStep};

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
    /// Takes an item out of the world for good, and says which cell it was in.
    ///
    /// This is not [`GameCommand::RevokeGrant`] renamed. Revoke is the *undo* for a write
    /// that failed: the row never existed, so the caller must not touch the store. This one
    /// is the front half of a real removal, where the row does exist and the caller deletes
    /// it next. Keeping them apart is what stops a failed write from being turned into a
    /// deletion, and a deletion from being run twice.
    ///
    /// The answer is the position rather than a `bool`, because the caller needs the cell
    /// to find the row it is about to remove: the store keys an item by
    /// `(owner_id, window_type, pos)`, so the cell is half the key. Asking the world is
    /// the only way to learn which cell a given id occupies without trusting a caller's
    /// memory of it.
    ReleaseItem {
        /// The character the item was granted to.
        target: String,
        /// The id to take out of the world.
        id: u32,
        /// Where the game thread reports the cell it freed and whose it was, or why it
        /// did not.
        reply: oneshot::Sender<Result<Released, RevokeRefused>>,
    },
    /// Puts a live client's character into the world.
    ///
    /// `DESC::SetPlayer` is the legacy step that makes a character visible to the
    /// process (`input_db.cpp`, `PlayerLoad`), and until a client went through it the
    /// world held nothing a grant could be addressed to. The client set and the
    /// position table were stand-ins written before a world existed on a thread;
    /// they still answer broadcasts, but they are not the world and they hold no
    /// items.
    ///
    /// The VID is an argument, never allocated here. Legacy allocates one from a
    /// process counter (`CHARACTER_MANAGER::AllocVID`) and adds a server-local CRC
    /// that never reaches the wire (`vid.h` converts to `DWORD` by returning `m_id`
    /// only), so the Rewrite has to be handed the number the descriptor is already
    /// using. Inventing a second number for the same character would put two VIDs on
    /// one client, which is the defect this command exists to prevent.
    EnterWorld {
        /// The wire VID the descriptor is already using, which is the store's
        /// `player.id`.
        vid: common::vid::Vid,
        /// The persistent player id, which the world indexes separately from the VID.
        player_id: u32,
        /// The character Name, which the world indexes case-insensitively.
        name: String,
        /// The items the load placed, in the order it placed them, each at the cell the
        /// client was told.
        ///
        /// They travel with the admission, not after it, so there is no pulse in which
        /// the world holds the character with an empty inventory while the client
        /// already shows its items: a grant in that gap would be offered a cell a loaded
        /// item holds.
        items: Vec<(ItemPos, Item)>,
        /// The points the load computed with those items worn, and the quickslots it set.
        loaded: Loaded,
        /// Where the game thread writes records addressed to this client.
        ///
        /// The world has no socket. It holds this sender and the descriptor drains
        /// it, which is the same shape as the existing per-client outbox in
        /// [`crate::client_registry`] and is what lets a grant be delivered without
        /// the game thread owning a file descriptor.
        outbox: ClientOutbox,
        /// Where the game thread reports whether it admitted the character.
        ///
        /// Closed, not sent, when the world could not act at all. The caller closes
        /// the descriptor rather than entering the game with a character the world
        /// does not hold, because a later grant would find nobody and the inventory
        /// would silently diverge.
        reply: oneshot::Sender<Result<(), EnterWorldRefused>>,
    },
    /// Takes a live client's character out of the world.
    ///
    /// `DESC::SetPlayer(NULL)` on a disconnect (`input_db.cpp`, `PlayerDestroy`).
    /// The ordering is the load-bearing part: the world must drop the character
    /// **before** the descriptor writes its row for the last time, or a save can
    /// flush an item the world has already released and a relog can find an item
    /// the world believes is free.
    LeaveWorld {
        /// The VID the character entered the world under.
        vid: common::vid::Vid,
        /// Where the game thread reports what the removed character left behind, or
        /// `None` when it did not hold the character.
        ///
        /// A `None` is not fatal to a disconnect: the descriptor is ending anyway,
        /// and legacy logs the same "already gone" case rather than refusing the
        /// close. It is still reported so a double leave is visible.
        reply: oneshot::Sender<Option<Departed>>,
    },
    /// Runs one client quickslot request for the character online under `vid`.
    Quickslot {
        /// The VID the character entered the world under.
        vid: common::vid::Vid,
        /// The request.
        step: QuickslotStep,
        /// Where the game thread reports the records and the slots, `None` when it does not
        /// hold the character.
        reply: oneshot::Sender<Option<QuickslotAnswer>>,
    },
    /// Answers the points of the character online under `vid`, which the descriptor's
    /// save writes from: the world changes them on its own pulse too (the potion
    /// recovery), so the copy the descriptor held after its last step can be stale.
    PointsOf {
        /// The VID the character entered the world under.
        vid: common::vid::Vid,
        /// Where the game thread reports the points, `None` when it does not hold the
        /// character or the character has none.
        reply: oneshot::Sender<Option<Points>>,
    },
    /// Writes one record to a character's client from the thread that owns the world.
    ///
    /// The alternative -- a caller writing to the outbox itself -- would need a copy
    /// of the sender map, and that map is world state the game thread mutates. So the
    /// write is a crossing, and the answer says whether there was a client at all.
    DeliverRecord {
        /// The character the record is addressed to.
        vid: common::vid::Vid,
        /// The already-encoded record. Encoding stays on the caller's side so the
        /// world never needs a codec, which is what keeps it free of `protocol`.
        record: Vec<u8>,
        /// Where the game thread reports whether a client received it.
        ///
        /// Closed, not sent, when the world could not act at all.
        reply: oneshot::Sender<bool>,
    },
    /// Runs one `CG_ITEM_MOVE` for the character online under `vid`.
    ///
    /// The world changes before the answer is sent, and the answer carries the records and
    /// the row changes, because the descriptor writes the rows and only then the records:
    /// the world is the only thing that can say where an item went, and the store is the
    /// only thing that can say whether that survives.
    MoveItem {
        /// The VID of the character whose client sent the move.
        vid: common::vid::Vid,
        /// The move as the client sent it.
        request: world::character::MoveRequest,
        /// What the descriptor knows of the character that the move reads.
        mover: Mover,
        /// Where the game thread reports what the move did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_ITEM_USE` for the character online under `vid`, answered as a move is.
    UseItem {
        /// The VID of the character whose client sent the use.
        vid: common::vid::Vid,
        /// The cell the client named.
        at: protocol::item_pos::ItemPos,
        /// What the descriptor knows of the character that the use reads.
        mover: Mover,
        /// Where the game thread reports what the use did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_ITEM_DROP` or `CG_ITEM_DROP2` for the character online under `vid`,
    /// answered as a move is.
    DropItem {
        /// The VID of the character whose client sent the drop.
        vid: common::vid::Vid,
        /// The cell the client named.
        at: protocol::item_pos::ItemPos,
        /// How many of the stack, where 0 is all of it.
        count: u16,
        /// Where the character stands.
        place: GroundPlace,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the drop did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_ITEM_PICKUP` for the character online under `vid`, answered as a move is.
    PickupItem {
        /// The VID of the character whose client sent the pick-up.
        vid: common::vid::Vid,
        /// The ground item's VID.
        ground: u32,
        /// Where the character stands.
        place: GroundPlace,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the pick-up did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Lists the `GC_ITEM_GROUND_ADD` records for every item on one map of one Channel.
    GroundItemsOn {
        /// The Channel.
        channel: u8,
        /// The map index.
        map: i32,
        /// Where the game thread sends the records.
        reply: oneshot::Sender<Vec<Vec<u8>>>,
    },
    /// Shares the NPCs the regen files stood up on one map of one Channel, with the map's
    /// mini-map list.
    NpcsOn {
        /// The Channel.
        channel: u8,
        /// The map index.
        map: i32,
        /// Where the game thread sends the map's NPCs.
        reply: oneshot::Sender<Arc<MapNpcs>>,
    },
    /// Requests terminal loop shutdown.
    Stop,
}

/// Where a character stands, as the descriptor holds it: the world keeps no position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroundPlace {
    /// The Channel.
    pub channel: u8,
    /// `GetMapIndex()`.
    pub map: i32,
    /// `GetX()`.
    pub x: i32,
    /// `GetY()`.
    pub y: i32,
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

impl std::fmt::Display for ReleaseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Each names what the caller must now believe about the world, because the
            // three are not the same and a caller that treated them as one would either
            // retry a removal that already happened or give up on one that did not.
            Self::NotSent => formatter.write_str(
                "the game thread could not be reached, so the world is not known to have moved",
            ),
            Self::NoAnswer => formatter.write_str(
                "the game thread did not answer, so the world may or may not have moved",
            ),
            Self::Refused(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ReleaseError {}

/// Why a request to take an item out of the world did not produce a cell.
///
/// This is deliberately not [`RevokeError`]. A revoke undoes a grant that just happened,
/// so the id came from the caller moments earlier and the world *not* holding it is a
/// bug. An Operator asks to destroy an id that may be any id at all, so "that character
/// does not hold it" is an ordinary answer, and folding it into the bug-shaped type would
/// make every honest refusal look like an invariant violation.
#[derive(Debug)]
pub enum ReleaseError {
    /// The command never reached the game thread, so the world is not known to have moved.
    NotSent,
    /// The command was sent and the thread dropped it without answering, so the world may
    /// or may not have moved.
    NoAnswer,
    /// The world refused and changed nothing.
    Refused(RevokeRefused),
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

/// Why a live client's character could not be put into the world, as distinct from
/// why the command did not arrive.
///
/// A refusal is an **answer**: the world is healthy and said no. These two are the
/// cases where there is no answer, and the descriptor has to treat both as "the world
/// is not running" rather than as a reason to keep going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnterWorldError {
    /// The command never reached the game thread.
    NotSent,
    /// The command was sent and the thread dropped it without answering.
    NoAnswer,
}

impl std::fmt::Display for EnterWorldError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSent => write!(formatter, "the world-entry command was not delivered"),
            Self::NoAnswer => write!(formatter, "the game thread gave no world-entry answer"),
        }
    }
}

impl std::error::Error for EnterWorldError {}

/// Why the world could not be asked to deliver a record.
///
/// A refusal is an answer, so these two are the cases where there is none. Both mean
/// the game thread is not running the world, which is a different thing from "that
/// character is offline".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliverError {
    /// The command never reached the game thread.
    NotSent,
    /// The command was sent and the thread dropped it without answering.
    NoAnswer,
}

impl std::fmt::Display for DeliverError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSent => write!(formatter, "the delivery command was not sent"),
            Self::NoAnswer => write!(formatter, "the game thread gave no delivery answer"),
        }
    }
}

impl std::error::Error for DeliverError {}

/// Why the world could not be asked to move an item.
///
/// A refusal is an answer, so these two are the cases where there is none, and both mean
/// the game thread is not running the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveItemError {
    /// The command never reached the game thread.
    NotSent,
    /// The command was sent and the thread dropped it without answering.
    NoAnswer,
}

impl std::fmt::Display for MoveItemError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSent => write!(formatter, "the item-move command was not sent"),
            Self::NoAnswer => write!(formatter, "the game thread gave no item-move answer"),
        }
    }
}

impl std::error::Error for MoveItemError {}

/// Why a live client's character could not be taken out of the world.
///
/// Distinct from [`EnterWorldError`] because the two are not symmetric: a leave that
/// never happened is a leak, and a disconnect can end anyway, so this is reported for
/// the log rather than to stop anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveWorldError {
    /// The command never reached the game thread.
    NotSent,
    /// The command was sent and the thread dropped it without answering.
    NoAnswer,
}

impl std::fmt::Display for LeaveWorldError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSent => write!(formatter, "the world-exit command was not delivered"),
            Self::NoAnswer => write!(formatter, "the game thread gave no world-exit answer"),
        }
    }
}

impl std::error::Error for LeaveWorldError {}

/// What the load gave a character besides its items, which the world holds from the
/// admission on.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    /// The points the load computed with the items worn, which wearing and taking off change.
    /// `None` for a character admitted without them.
    pub points: Option<Points>,
    /// The quickslots the load set.
    pub quickslots: Quickslots,
}

/// What the world hands back when a character leaves it.
#[derive(Debug, Clone, PartialEq)]
pub struct Departed {
    /// The character's points as the world last changed them, which the final save writes.
    /// `None` for a character that entered without them.
    pub points: Option<Points>,
}

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
    pulse: Arc<AtomicU64>,
    game_thread: Thread,
}

impl GameLoopController {
    pub(crate) fn new(
        command_tx: mpsc::Sender<GameCommand>,
        stop_requested: Arc<AtomicBool>,
        pulse: Arc<AtomicU64>,
        game_thread: Thread,
    ) -> Self {
        Self {
            command_tx,
            stop_requested,
            pulse,
            game_thread,
        }
    }

    /// The Pulse the game thread is on: the one it last started, or 0 before the first.
    ///
    /// This is legacy's `thecore_heart->pulse`, which a descriptor's input handler read on the
    /// same thread. A descriptor task here reads the value the game thread published, which is
    /// the Pulse the game thread is processing or has just finished.
    #[must_use]
    pub fn pulse(&self) -> u64 {
        self.pulse.load(Ordering::Acquire)
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

    /// Takes an item out of the world for good, and reports the cell it was in.
    ///
    /// The caller deletes the row next, so a refused release must leave the store alone.
    /// That is why the answer is split three ways rather than being a `bool`: a `false`
    /// could mean "the character does not hold that id" or "the game thread is gone",
    /// and only the first says the row is stale, while the second says the world may hold
    /// something the caller cannot see. Collapsing them is how a destroy ends up deleting
    /// a row for an item that is still in the world.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::NotSent`] when the game thread has closed its receiver,
    /// [`ReleaseError::NoAnswer`] when it closed the reply without sending, and
    /// [`ReleaseError::Refused`] carrying the world's own answer: the target is not
    /// online, or the character does not hold the id.
    pub async fn release_item(&self, target: &str, id: u32) -> Result<Released, ReleaseError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ReleaseItem {
            target: target.to_owned(),
            id,
            reply,
        })
        .await
        .map_err(|_| ReleaseError::NotSent)?;
        answer
            .await
            .map_err(|_| ReleaseError::NoAnswer)?
            .map_err(ReleaseError::Refused)
    }

    /// Puts a live client's character into the world, and waits for the answer.
    ///
    /// The caller must supply the VID the descriptor already publishes. A world that
    /// allocated its own would be unreachable by the records it writes, and the
    /// inventory the client shows and the inventory the world holds would be two
    /// different sets that agree only by accident.
    ///
    /// # Errors
    ///
    /// [`EnterWorldError::NotSent`] when the game thread has closed its receiver and
    /// [`EnterWorldError::NoAnswer`] when it closed the reply without sending. Both
    /// mean the world holds nothing, and the descriptor must close rather than enter
    /// the game: a client playing a character the world cannot see can be granted
    /// nothing and will not know.
    pub async fn enter_world(
        &self,
        vid: common::vid::Vid,
        player_id: u32,
        name: String,
        outbox: ClientOutbox,
    ) -> Result<Result<(), EnterWorldRefused>, EnterWorldError> {
        self.enter_world_with_items(vid, player_id, name, Vec::new(), Loaded::default(), outbox)
            .await
    }

    /// Puts a live client's character into the world holding the items its load placed.
    ///
    /// `items` is [`crate::item_load::ItemLoad::placed`] in its own order. The world
    /// places them before it answers, so the admission and the inventory are one step.
    ///
    /// # Errors
    ///
    /// As [`GameLoopController::enter_world`].
    pub async fn enter_world_with_items(
        &self,
        vid: common::vid::Vid,
        player_id: u32,
        name: String,
        items: Vec<(ItemPos, Item)>,
        loaded: Loaded,
        outbox: ClientOutbox,
    ) -> Result<Result<(), EnterWorldRefused>, EnterWorldError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::EnterWorld {
            vid,
            player_id,
            name,
            items,
            loaded,
            outbox,
            reply,
        })
        .await
        .map_err(|_| EnterWorldError::NotSent)?;
        answer.await.map_err(|_| EnterWorldError::NoAnswer)
    }

    /// Takes a live client's character out of the world, and waits for the answer.
    ///
    /// The descriptor must call this **before** its final save. A character released
    /// from the world frees the cells its items hold, and a save that runs afterwards
    /// writes a row naming a cell the world has already offered to somebody else.
    ///
    /// # Errors
    ///
    /// [`LeaveWorldError::NotSent`] and [`LeaveWorldError::NoAnswer`]. Neither is
    /// fatal to a disconnect -- the descriptor is ending and the process is losing the
    /// world with it -- but both are reported so a leave that never happened is
    /// visible rather than assumed.
    pub async fn leave_world(
        &self,
        vid: common::vid::Vid,
    ) -> Result<Option<Departed>, LeaveWorldError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::LeaveWorld { vid, reply })
            .await
            .map_err(|_| LeaveWorldError::NotSent)?;
        answer.await.map_err(|_| LeaveWorldError::NoAnswer)
    }

    /// Asks the world to run one client quickslot request for the character online under
    /// `vid`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn quickslot(
        &self,
        vid: common::vid::Vid,
        step: QuickslotStep,
    ) -> Result<Option<QuickslotAnswer>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Quickslot { vid, step, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world for the points of the character online under `vid`, which a save
    /// writes from.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn points_of(&self, vid: common::vid::Vid) -> Result<Option<Points>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::PointsOf { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to write one record to a character's client, and waits for the
    /// answer.
    ///
    /// The world owns no socket, so this is the only way a record the world produced
    /// reaches a client (ADR-0002). It is a command rather than a direct call because
    /// the sender map is world state, and reading it from a Tokio task while the game
    /// thread is mutating the world is exactly the race the dedicated thread exists
    /// to prevent.
    ///
    /// The `bool` distinguishes "delivered" from "there was no client", which a grant
    /// needs: the row is already written by then, so a `false` is a notification that
    /// could not be sent rather than an item that was lost.
    ///
    /// # Errors
    ///
    /// [`DeliverError::NotSent`] when the game thread has closed its receiver and
    /// [`DeliverError::NoAnswer`] when it closed the reply without sending.
    pub async fn deliver_record(
        &self,
        vid: common::vid::Vid,
        record: Vec<u8>,
    ) -> Result<bool, DeliverError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DeliverRecord { vid, record, reply })
            .await
            .map_err(|_| DeliverError::NotSent)?;
        answer.await.map_err(|_| DeliverError::NoAnswer)
    }

    /// Asks the world to run one `CG_ITEM_MOVE`, and waits for what it did.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver and
    /// [`MoveItemError::NoAnswer`] when it closed the reply without sending.
    pub async fn move_item(
        &self,
        vid: common::vid::Vid,
        request: world::character::MoveRequest,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MoveItem {
            vid,
            request,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `CG_ITEM_USE`, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn use_item(
        &self,
        vid: common::vid::Vid,
        at: protocol::item_pos::ItemPos,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::UseItem {
            vid,
            at,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to drop an item, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn drop_item(
        &self,
        vid: common::vid::Vid,
        at: protocol::item_pos::ItemPos,
        count: u16,
        place: GroundPlace,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DropItem {
            vid,
            at,
            count,
            place,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to pick a ground item up, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn pickup_item(
        &self,
        vid: common::vid::Vid,
        ground: u32,
        place: GroundPlace,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::PickupItem {
            vid,
            ground,
            place,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world for the items lying on one map, as the records a client entering it
    /// is sent.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn ground_items_on(
        &self,
        channel: u8,
        map: i32,
    ) -> Result<Vec<Vec<u8>>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GroundItemsOn {
            channel,
            map,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world for the NPCs standing on one map, which a client entering it is shown.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn npcs_on(&self, channel: u8, map: i32) -> Result<Arc<MapNpcs>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::NpcsOn {
            channel,
            map,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
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
    use std::sync::atomic::{AtomicBool, AtomicU64};
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
            Arc::new(AtomicU64::new(0)),
            thread::current(),
        );

        // When: a second command is attempted without draining the first.
        controller.try_send_command(completion(1)).unwrap();
        let result = controller.try_send_command(completion(2));

        // Then: bounded backpressure is reported synchronously.
        assert_eq!(result, Err(CommandSendError::QueueFull));
    }
}
