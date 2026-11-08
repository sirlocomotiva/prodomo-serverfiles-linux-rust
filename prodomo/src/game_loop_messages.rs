//! Typed messages crossing between Tokio and the synchronous game loop.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::Thread;

use tokio::sync::{mpsc, oneshot};

use protocol::cg_move::CgMove;
use protocol::cg_variable::SyncPositionPacket;
use protocol::item_pos::ItemPos;
use world::character::{Points, Quickslots};
use world::item::Item;
use world::npc::MapNpcs;

use crate::client_registry::ClientOutbox;
use crate::game_state::{
    EnterWorldRefused, QuestStep, Released, RevokeRefused, SafeboxAnswer, SafeboxStep, ShopAnswer,
    ShopStep, TradeAnswer, TradeStep,
};
use crate::item_grant::{GrantOutcome, GrantRefusal, GrantRequest};
use crate::item_move::{MoveItemRefused, MovedItems, Mover};
use crate::loading_phase::PcCard;
use crate::quickslot::{QuickslotAnswer, QuickslotStep};
use crate::sync_position::SyncPositionResult;

/// Default capacity of the Tokio-to-game command queue.
pub const DEFAULT_COMMAND_CAPACITY: usize = 1024;

/// Default capacity of the game-to-Tokio effect queue.
pub const DEFAULT_EFFECT_CAPACITY: usize = 256;

/// Default maximum commands drained at the start of each pulse.
pub const DEFAULT_MAX_COMMANDS_PER_PULSE: usize = 512;
/// `SGuildMark::SIZE`: the 192 words of one mark image.
pub const MARK_PIXELS: usize = 16 * 12;
/// `CG_MARK_CRCLIST_WORDS`: the 80 CRC words one image slot holds.
pub const MARK_CRC_WORDS: usize = 80;

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
/// What a quest-running command carries of the reader's account, for `pc.change_empire`,
/// `pc.get_change_empire_count`, `pc.get_account_id` and the war bets: the account's ID
/// (`GetAccountTable().id`), its character PIDs, in slot order with 0 for an empty slot
/// (legacy's `player_index` row), how many times the account changed its empire (legacy's
/// `change_empire` row), and the account's login (`GetAccountTable().login`, which the bet
/// chat reads). The descriptor reads all four from its account table and the store; the
/// session's shadow judges the change's guild arm off the PIDs, and the world's judge
/// answers the landing and the bets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestAccount {
    /// The account's ID, as the account table holds it.
    pub id: u32,
    /// The account's character PIDs, in slot order, 0 for an empty slot.
    pub pids: [u32; 4],
    /// How many times the account changed its empire.
    pub changes: u32,
    /// The account's login, as stored.
    pub login: String,
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
        /// Where the game thread reports whether it installed.
        ///
        /// Closed, not sent, when the world could not act on the request at all.
        reply: oneshot::Sender<bool>,
    },
    /// Installs the stored guilds, grades and members, which `CGuildManager::Initialize`
    /// loads at boot (`G/guild_manager.cpp:212-243`). The rows cannot be a constructor
    /// argument: they are only readable once the store has migrated, which happens inside
    /// the accept loop after the listeners are bound, as the item id range is.
    ///
    /// The answer is a `bool` for the same reason
    /// [`GameCommand::InstallItemIdRange`]'s is: `serve` treats a second install as a
    /// bug it must not paper over.
    InstallGuilds {
        /// Every stored guild with its grade and member rows.
        stored: db::guilds::StoredGuilds,
        /// Every stored mark row.
        marks: Vec<db::guilds::StoredMark>,
        /// Every stored symbol row.
        symbols: Vec<db::guilds::StoredSymbol>,
        /// The boot's war reservations (`BootReserveWar`).
        reservations: Vec<db::guilds::StoredReservation>,
        /// The boot's war bets (`CGuildWarReserve::Initialize`).
        bets: Vec<db::guilds::StoredBet>,
        /// Every owned land and stored building object (`land.guild_id`,
        /// `D/ClientManagerBoot.cpp:919-983`), bundled so the command stays
        /// under the argument lint.
        land: db::guild_land::StoredLandInstall,
        /// Where the game thread reports whether it installed.
        reply: oneshot::Sender<bool>,
    },
    /// Installs the stored event calendar, once, after migration.
    ///
    /// The answer is a `bool` for the same reason
    /// [`GameCommand::InstallItemIdRange`]'s is: `serve` treats a second install as a
    /// bug it must not paper over.
    InstallEvents {
        /// Every stored event row, in id order.
        events: Vec<db::events::StoredEvent>,
        /// Where the game thread reports whether it installed.
        reply: oneshot::Sender<bool>,
    },
    /// Installs the global rank boot read (`db::rank_global::load`): the season, the
    /// counters, the prizes and the unclaimed awards. See `game_state::rank_global`.
    InstallRank {
        /// What boot read.
        boot: db::rank_global::RankBoot,
        /// Where the game thread reports that it installed.
        reply: oneshot::Sender<()>,
    },
    /// Installs the monarch treasuries and identities (`db::monarch::load_all`,
    /// `load_pids`, `names_by_pids`), once, after migration. See `game_state::monarch`.
    InstallMonarch {
        /// The four treasury cells, by empire slot.
        money: crate::game_state::InstalledMonarchMoney,
        /// The four pids with their names, by empire slot.
        identity: crate::game_state::InstalledMonarchIdentity,
        /// Where the game thread reports that it installed.
        reply: oneshot::Sender<()>,
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
        ///
        /// Boxed because it is most of the command's size, and every future that sends a
        /// command holds one.
        loaded: Box<Loaded>,
        /// Where the game thread writes records addressed to this client.
        ///
        /// The world has no socket. It holds this sender and the descriptor drains
        /// it, which is the same shape as the existing per-client outbox in
        /// [`crate::client_registry`] and is what lets a grant be delivered without
        /// the game thread owning a file descriptor.
        outbox: ClientOutbox,
        /// Where the game thread reports whether it admitted the character, and the
        /// records the show sent its own client.
        ///
        /// Closed, not sent, when the world could not act at all. The caller closes
        /// the descriptor rather than entering the game with a character the world
        /// does not hold, because a later grant would find nobody and the inventory
        /// would silently diverge.
        reply: oneshot::Sender<Result<Shown, EnterWorldRefused>>,
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
        /// Whether the character disconnects or warps.
        departure: Departure,
        /// Where the game thread reports what the removed character left behind, or
        /// `None` when it did not hold the character.
        ///
        /// A `None` is not fatal to a disconnect: the descriptor is ending anyway,
        /// and legacy logs the same "already gone" case rather than refusing the
        /// close. It is still reported so a double leave is visible.
        reply: oneshot::Sender<Option<Kept>>,
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
    /// The map a character is placed on, as `GetValidLocation` resolves it
    /// (`G/sectree_manager.cpp:816-842`): a private map the world does not host is the base
    /// map it was copied from, because the copy of a base map a guild war is fought on died
    /// with the process that held it.
    ///
    /// The world is asked, because only the world knows which private maps it holds: the
    /// descriptor reads the row, and the row cannot say whether the copy is still there.
    MapOf {
        /// The Channel the character would stand on.
        channel: u8,
        /// The map the row named.
        map_index: i32,
        /// Where the game thread reports the map.
        reply: oneshot::Sender<i32>,
    },
    /// The lines `BroadcastEventFlagOnLogin` sends a loading character
    /// (`G/input_db.cpp:437`), one per non-zero flag. See `game_state::event_flag`.
    LoginEventLines {
        /// Where the game thread reports the lines' texts.
        reply: oneshot::Sender<Vec<Vec<u8>>>,
    },
    /// Answers the points and quickslots of the character online under `vid`, which the
    /// descriptor's save writes from: the world changes them on its own too (the potion
    /// recovery on its pulse, a trade the partner closes), so the copy the descriptor held
    /// after its last step can be stale.
    KeptOf {
        /// The VID the character entered the world under.
        vid: common::vid::Vid,
        /// The minutes played the save banks, which `CreatePlayerProto` writes into the real
        /// `POINT_PLAYTIME` before it reads the points (`G/char.cpp:1519-1545`); `None`
        /// leaves them.
        playtime: Option<i32>,
        /// Where the game thread reports them, `None` when it does not hold the character.
        reply: oneshot::Sender<Option<Kept>>,
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
        /// The reader's account, for a used quest item's scripts.
        account: QuestAccount,
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
    /// Runs one `CG_ITEM_DESTROY` for the character online under `vid`, answered as a move is.
    DestroyItem {
        /// The VID of the character whose client sent the destroy.
        vid: common::vid::Vid,
        /// The cell the client named.
        at: protocol::item_pos::ItemPos,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the destroy did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_ITEM_USE_TO_ITEM` for the character online under `vid`, answered as a
    /// move is.
    UseItemToItem {
        /// The VID of the character whose client sent the use.
        vid: common::vid::Vid,
        /// The cell of the item used.
        at: protocol::item_pos::ItemPos,
        /// The cell of the item it is used on.
        dest: protocol::item_pos::ItemPos,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the use did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_REFINE` for the character online under `vid`, answered as a move is.
    Refine {
        /// The VID of the character whose client sent the request.
        vid: common::vid::Vid,
        /// The inventory cell the client named.
        pos: u8,
        /// The refine type, or 255 for a closed window.
        refine_type: u8,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the refine did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_REFINE_ELEMENT` for the character online under `vid`, answered as a
    /// move is.
    RefineElement {
        /// The VID of the character whose client sent the request.
        vid: common::vid::Vid,
        /// The request's argument: the change's element, or 255 for a closed window.
        arg: u8,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the refine did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_ATTR67_ADD` for the character online under `vid`, answered as a move is.
    Attr67 {
        /// The VID of the character whose client sent the request.
        vid: common::vid::Vid,
        /// The sub-header: open, close, or regist.
        sub_header: u8,
        /// The regist's named cells and counts.
        regist: crate::game_state::Attr67Regist,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the regist did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_SASH` for the character online under `vid`, answered as a move is.
    Sash {
        /// The VID of the character whose client sent the request.
        vid: common::vid::Vid,
        /// The sub-header: close, add, remove, or refine.
        sub_header: u8,
        /// The material slot the client named.
        pos: u8,
        /// The material position the client named.
        at: protocol::item_pos::ItemPos,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the sash step did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_CL` for the character online under `vid`, answered as a move is.
    Changelook {
        /// The VID of the character whose client sent the request.
        vid: common::vid::Vid,
        /// The sub-header: close, add, remove, or refine.
        sub_header: u8,
        /// The material slot the client named.
        pos: u8,
        /// The material position the client named.
        at: protocol::item_pos::ItemPos,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the changelook step did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_CUBE_RENEWAL` for the character online under `vid`, answered as a move is.
    CubeRenewal {
        /// The VID of the character whose client sent the request.
        vid: common::vid::Vid,
        /// The sub-header: make or close.
        sub_header: u8,
        /// The recipe index, the craft count, and the improve cell the client named.
        words: (u32, u32, u32),
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the cube step did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_GAYA_SYSTEM` for the character online under `vid`, answered as a move is.
    GayaSystem {
        /// The VID of the character whose client sent the request.
        vid: common::vid::Vid,
        /// The sub-header: craft, market, or refresh.
        sub_header: u8,
        /// The `pos` the client named.
        pos: i32,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the Gaya step did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `CG_BIOLOGIST` for the character online under `vid`: an open, a take,
    /// a pick, or a wait-shortening. The records reach the client through the answer;
    /// the reply carries the rows to store.
    Biologist {
        /// The VID of the character whose client sent the record.
        vid: common::vid::Vid,
        /// The decoded step.
        step: crate::game_state::BiologistStep,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports the rows the step changed.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_PREMIUM_PLAYERS` for the character online under `vid`: the open,
    /// the list, or the activation. The records reach the client through the answer;
    /// the reply carries the rows to store.
    Premium {
        /// The VID of the character whose client sent the record.
        vid: common::vid::Vid,
        /// The decoded step.
        step: crate::game_state::PremiumStep,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports the rows the step changed.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `ENVANTER_BLACK` for the character online under `vid`: the next
    /// inventory row unlocked for its price in keys, or the shortfall chatted. The
    /// records reach the client through the answer; the reply carries the rows to
    /// store.
    ExtendInven {
        /// The VID of the character whose client sent the record.
        vid: common::vid::Vid,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports the rows the step changed.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_INVENTORY_PROTECTED` for the character online under `vid`: the lock
    /// or the unlock, or the password change. The records reach the client through the
    /// answer; the reply carries the rows to store.
    Protected {
        /// The VID of the character whose client sent the record.
        vid: common::vid::Vid,
        /// The decoded step.
        step: crate::game_state::ProtectedStep,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports the rows the step changed.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_ITEM_GIVE` for the character online under `vid`, answered as a move is.
    GiveItem {
        /// The VID of the character whose client sent the gift.
        vid: common::vid::Vid,
        /// The VID the client named as the gift's victim.
        target: u32,
        /// The cell the client named.
        at: protocol::item_pos::ItemPos,
        /// Where the descriptor believes the character stands.
        place: GroundPlace,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the gift did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one gold `CG_ITEM_DROP` or `CG_ITEM_DROP2` for the character online under `vid`,
    /// answered as a move is.
    DropGold {
        /// The VID of the character whose client sent the drop.
        vid: common::vid::Vid,
        /// How much gold the client named.
        gold: u32,
        /// Where the character stands.
        place: GroundPlace,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the drop did.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },

    /// Runs one `CG_FISHING` for the character online under `vid`: the cast, or the take
    /// while its fishing event runs. The records reach the client through the world; the
    /// answer carries the rows to store.
    Fishing {
        /// The VID of the character whose client sent the record.
        vid: common::vid::Vid,
        /// The record's direction.
        dir: u8,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports the rows the step changed.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Runs one `CG_FISH_EVENT_SEND` for the character online under `vid`: a box use or
    /// a shape add. The records reach the client through the world; the answer carries
    /// the rows to store.
    FishEvent {
        /// The VID of the character whose client sent the record.
        vid: common::vid::Vid,
        /// The decoded step.
        step: crate::game_state::FishEventStep,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports the rows the step changed.
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
    /// Runs one shop step for the character online under `vid`: a click on a keeper, a buy, a
    /// sale or a close.
    Shop {
        /// The VID of the character whose client sent the step.
        vid: common::vid::Vid,
        /// The step.
        step: ShopStep,
        /// Where the character stands.
        place: GroundPlace,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// The reader's account, for a quest's scripts.
        account: QuestAccount,
        /// Where the game thread reports what the step did.
        reply: oneshot::Sender<Result<ShopAnswer, MoveItemRefused>>,
    },
    /// Judges one `CG_ANSWER_MAKE_GUILD` for the character online under `vid`
    /// (`CInputMain::AnswerMakeGuild`, `G/input_main.cpp:2672-2730`): the gold, the delay
    /// gates, the guildless state, the name, and the in-memory duplicate. The world answers
    /// what the descriptor must store, if anything; see `game_state::guild`.
    AnswerMakeGuild {
        /// The VID of the character whose client sent the answer.
        vid: common::vid::Vid,
        /// The raw 13 name bytes the client sent.
        field: [u8; protocol::cg_guild_answer::GUILD_NAME_FIELD_SIZE],
        /// `get_global_time()` in seconds, which the descriptor reads.
        now: u32,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildMakeAnswer>,
    },
    /// Judges one `/makeguild` word for the character online under `vid`
    /// (`ACMD(do_makeguild)`, `G/cmd_gm.cpp:2143-2167`): the guildless state, the name,
    /// and the in-memory duplicate. No fee, no item, no delays.
    JudgeMakeguild {
        /// The VID of the character whose client sent the line.
        vid: common::vid::Vid,
        /// The raw word the line carried.
        word: Vec<u8>,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildMakeAnswer>,
    },
    /// Judges one `/deleteguild` for the character online under `vid`
    /// (`ACMD(do_deleteguild)`, `G/cmd_gm.cpp:2169-2173`): the guild disbands when the
    /// caller leads it and no war holds it. Reports the quest-shaped judgement; the
    /// descriptor stores the disband half.
    JudgeDeleteguild {
        /// The VID of the character whose client sent the line.
        vid: common::vid::Vid,
        /// `get_global_time()` in seconds, which the descriptor reads.
        now: u32,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildQuestJudged>,
    },
    /// Commits one judged `/makeguild` for the character online under `vid`: the registry
    /// and the link, the records, and the confirmation. No fee, no item.
    CommitMakeguild {
        /// The VID of the character whose client sent the line.
        vid: common::vid::Vid,
        /// The id the store answered plus the judged name.
        commit: crate::game_state::GuildMakeCommit,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports the commit.
        reply: oneshot::Sender<crate::game_state::GuildMakeAnswer>,
    },
    /// Commits one judged creation for the character online under `vid` under the id the
    /// store answered: the registry and the link, the fee, the item, and every record for
    /// the descriptor to send and store. See `game_state::guild`.
    CommitMakeGuild {
        /// The VID of the character whose client sent the answer.
        vid: common::vid::Vid,
        /// The id the store answered plus the judged name.
        commit: crate::game_state::GuildMakeCommit,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports the commit.
        reply: oneshot::Sender<crate::game_state::GuildMakeAnswer>,
    },
    /// Runs one `GUILD_SUBHEADER_CG_ADD_MEMBER` for the character online under `vid`
    /// (`CInputMain::Guild`'s arm, `G/input_main.cpp:2880-2899`, plus `CGuild::Invite`,
    /// `G/guild.cpp:2034-2113`): the world judges and invites in one step; nothing is
    /// stored.
    GuildAddMember {
        /// The VID of the character whose client sent the invite.
        vid: common::vid::Vid,
        /// The guest's VID, as the client sent it.
        guest: u32,
        /// `get_global_time()` in seconds, which the descriptor reads.
        now: u32,
    },
    /// Judges one `GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER` for the character online
    /// under `vid` (`CInputMain::Guild`'s arm, `G/input_main.cpp:3137-3159`, plus
    /// `InviteAccept`/`InviteDeny`, `G/guild.cpp:2115-2158`): the world answers what
    /// the descriptor must store, if anything; see `game_state::guild`.
    AnswerGuildInvite {
        /// The VID of the character whose client sent the answer.
        vid: common::vid::Vid,
        /// The guild the invite named.
        guild_id: u32,
        /// The client's accept byte: 1 joins, 0 denies.
        accept: u8,
        /// `get_global_time()` in seconds, which the descriptor reads.
        now: u32,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildJoinAnswer>,
    },
    /// Commits one judged join under the stored member row: the row lands, the PID
    /// links, and the answer carries every record for the descriptor to send. See
    /// `game_state::guild`.
    CommitGuildJoin {
        /// The guild plus the joiner's PID.
        commit: crate::game_state::GuildJoinCommit,
        /// Where the game thread reports the commit.
        reply: oneshot::Sender<crate::game_state::GuildJoinAnswer>,
    },
    /// Judges one `GUILD_SUBHEADER_CG_REMOVE_MEMBER` for the character online under
    /// `vid` (`CInputMain::Guild`'s arm, `G/input_main.cpp:2901-2954`, plus
    /// `RequestRemoveMember`, `G/guild.cpp:195-211`): the world answers what the
    /// descriptor must delete, if anything; see `game_state::guild`.
    AnswerGuildRemove {
        /// The VID of the character whose client sent the removal.
        vid: common::vid::Vid,
        /// The victim's PID, as the client sent it.
        pid: u32,
        /// `get_global_time()` in seconds, which the descriptor reads.
        now: u32,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildRemoveAnswer>,
    },
    /// Commits one judged removal under the deleted member row: the victim unlinks and
    /// the answer carries the fan plus the after line, if any. See `game_state::guild`.
    CommitGuildRemove {
        /// The speaker's VID plus the victim's PID.
        commit: crate::game_state::GuildRemoveCommit,
        /// Where the game thread reports the commit.
        reply: oneshot::Sender<crate::game_state::GuildRemoveAnswer>,
    },
    /// Judges one `GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME` for the character online under
    /// `vid` (`CInputMain::Guild`'s arm, `G/input_main.cpp:2956-2979`, plus
    /// `ChangeGradeName`, `G/guild.cpp:822-857`): the world answers what the
    /// descriptor must store, if anything; see `game_state::guild`.
    AnswerGuildGradeName {
        /// The VID of the character whose client sent the rename.
        vid: common::vid::Vid,
        /// The 1-based grade, as the client sent it.
        grade: u8,
        /// The 9 name field bytes, as the client sent them.
        field: Vec<u8>,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildGradeAnswer>,
    },
    /// Judges one `GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY` for the character online
    /// under `vid` (`CInputMain::Guild`'s arm, `G/input_main.cpp:2982-3000`, plus
    /// `ChangeGradeAuth`, `G/guild.cpp:870-913`): the world answers what the
    /// descriptor must store, if anything; see `game_state::guild`.
    AnswerGuildGradeAuth {
        /// The VID of the character whose client sent the re-auth.
        vid: common::vid::Vid,
        /// The 1-based grade, as the client sent it.
        grade: u8,
        /// The auth byte, as the client sent it.
        auth: u8,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildGradeAnswer>,
    },
    /// Judges one `GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE` for the character online
    /// under `vid` (`CInputMain::Guild`'s arm, `G/input_main.cpp:3062-3081`, plus
    /// `ChangeMemberGrade`, `G/guild.cpp:1190-1229`): the world answers what the
    /// descriptor must store, if anything; see `game_state::guild`.
    AnswerGuildMemberGrade {
        /// The VID of the character whose client sent the re-grade.
        vid: common::vid::Vid,
        /// The victim's PID, as the client sent it.
        pid: u32,
        /// The grade, as the client sent it.
        grade: u8,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildGradeAnswer>,
    },
    /// Judges one `GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL` for the character online
    /// under `vid` (`CInputMain::Guild`'s arm, `G/input_main.cpp:3095-3116`, plus
    /// `ChangeMemberGeneral`, `G/guild.cpp:1144-1189`): the world answers what the
    /// descriptor must store, if anything; see `game_state::guild`.
    AnswerGuildMemberGeneral {
        /// The VID of the character whose client sent the flag.
        vid: common::vid::Vid,
        /// The victim's PID, as the client sent it.
        pid: u32,
        /// The flag byte, as the client sent it.
        is_general: u8,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildGradeAnswer>,
    },
    /// Judges one `GUILD_SUBHEADER_CG_OFFER` for the character online under `vid`
    /// (`CInputMain::Guild`'s arm, `G/input_main.cpp:3005-3029`, plus `OfferExp`,
    /// `G/guild.cpp:942-982`): the world answers what the descriptor must store, if
    /// anything; see `game_state::guild`.
    AnswerGuildOffer {
        /// The VID of the character whose client sent the offer.
        vid: common::vid::Vid,
        /// The offered experience, as the client sent it.
        offer: u32,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildGradeAnswer>,
    },
    /// Judges one `GUILD_SUBHEADER_CG_CHARGE_GSP` for the character online under `vid`
    /// (`CInputMain::Guild`'s arm, `G/input_main.cpp:3030-3047`, plus `ChargeSP`,
    /// `G/guild.cpp:1804-1831`): the world answers what the descriptor must store, if
    /// anything; see `game_state::guild`.
    AnswerGuildCharge {
        /// The VID of the character whose client sent the charge.
        vid: common::vid::Vid,
        /// The faith to charge, as the client sent it.
        offer: i32,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildGradeAnswer>,
    },
    /// Runs one `/greset` for the character online under `vid` (`do_greset`,
    /// `G/cmd_gm.cpp:2175-2179`, plus `CGuild::Reset`, `G/guild.h:175`): the
    /// world answers what the descriptor must store, if anything; see
    /// `game_state::guild`.
    RunGuildReset {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildGradeAnswer>,
    },
    /// Judges one `GUILD_SUBHEADER_CG_POST_COMMENT` for the character online under
    /// `vid` (`CInputMain::Guild`'s arm, `G/input_main.cpp:3048-3080`, plus
    /// `AddComment`, `G/guild.cpp:1049-1072`): the world answers what the descriptor
    /// must store, if anything; see `game_state::guild`.
    AnswerGuildPostComment {
        /// The VID of the character whose client posted.
        vid: common::vid::Vid,
        /// The content bytes, as the client sent them.
        content: Vec<u8>,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildCommentAnswer>,
    },
    /// Commits one judged post under the inserted row. See `game_state::guild`.
    CommitGuildPostComment {
        /// The speaker, the guild and the board after the post.
        commit: crate::game_state::GuildCommentCommit,
        /// Where the game thread reports the board.
        reply: oneshot::Sender<crate::game_state::GuildCommentAnswer>,
    },
    /// Runs one `GUILD_SUBHEADER_CG_REFRESH_COMMENT` for the character online under
    /// `vid` (`CInputMain::Guild`'s arm, `G/input_main.cpp:3089-3091`, plus
    /// `RefreshComment`, `G/guild.cpp:1092-1095`): the world answers the gates and the
    /// descriptor reads the board; see `game_state::guild`.
    AnswerGuildRefreshComment {
        /// The VID of the character whose client sent the refresh.
        vid: common::vid::Vid,
        /// Where the game thread reports the gates.
        reply: oneshot::Sender<crate::game_state::GuildCommentAnswer>,
    },
    /// Runs one `GUILD_SUBHEADER_CG_DELETE_COMMENT` for the character online under
    /// `vid` (`CInputMain::Guild`'s arm, `G/input_main.cpp:3081-3088`, plus
    /// `DeleteComment`, `G/guild.cpp:1075-1090`): the world answers the gates and the
    /// descriptor deletes the row, then reports; see `game_state::guild`.
    AnswerGuildDeleteComment {
        /// The VID of the character whose client sent the delete.
        vid: common::vid::Vid,
        /// The comment id, as the client sent it.
        comment_id: u32,
        /// Where the game thread reports the gates.
        reply: oneshot::Sender<crate::game_state::GuildCommentAnswer>,
    },
    /// Reports one judged delete's store half: whether a row was there, with the board
    /// after it. See `game_state::guild`.
    CommitGuildDeleteComment {
        /// The speaker, the guild, the board, and whether a row was there.
        commit: crate::game_state::GuildCommentCommit,
        /// Where the game thread reports the board or the line.
        reply: oneshot::Sender<crate::game_state::GuildCommentAnswer>,
    },
    /// Commits one judged grade-arm step under the stored row: the value lands, the
    /// fan goes out, and the answer carries the after line, if any. See
    /// `game_state::guild`.
    CommitGuildGrade {
        /// The speaker's VID plus the judged value.
        commit: crate::game_state::GuildGradeCommit,
        /// Where the game thread reports the commit.
        reply: oneshot::Sender<crate::game_state::GuildGradeAnswer>,
    },
    /// Judges one `GUILD_SUBHEADER_CG_USE_SKILL` for the character online under `vid`
    /// (`CGuild::UseSkill`, `G/guild.cpp:1267-1340`): the world answers what the
    /// descriptor must store, if anything; see `game_state::guild`.
    AnswerGuildUseSkill {
        /// The VID of the character whose client sent the cast.
        vid: common::vid::Vid,
        /// The cast vnum, as the client sent it.
        vnum: u32,
        /// The target PID, as the client sent it.
        pid: u32,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::GuildSkillUseAnswer>,
    },
    /// Commits one judged cast under the stored power: the power lands, the fan goes
    /// out, and the branch runs. See `game_state::guild`.
    CommitGuildUseSkill {
        /// The speaker, the guild, the cast, and the judged power.
        commit: crate::game_state::GuildSkillUseCommit,
        /// Where the game thread reports the commit.
        reply: oneshot::Sender<crate::game_state::GuildSkillUseAnswer>,
    },
    /// Runs one `/gskillup` line (`do_guildskillup`, `G/cmd_general.cpp:878-904`, plus
    /// `CGuild::SkillLevelUp`); see `game_state::guild`.
    GuildSkillUp {
        /// The VID of the skilling player.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the climbed skill, when the up lands.
        reply: oneshot::Sender<crate::game_state::GuildSkillUpAnswer>,
    },
    /// Runs one `/war` line (`do_war`, `G/cmd_general.cpp:2028-2210`); see
    /// `game_state::guild`.
    GuildWar {
        /// The VID of the declaring player.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// `get_global_time()` in seconds, which stamps an accept's start.
        now: u32,
        /// Where the game thread reports the declare notice, when one lands.
        reply: oneshot::Sender<crate::game_state::GuildWarAnswer>,
    },
    /// Runs one `/nowar` line (`do_nowar`, `G/cmd_general.cpp:2211-2243`); see
    /// `game_state::guild`.
    GuildNoWar {
        /// The VID of the refusing player.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the refusal ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `gwstop` line (`do_stop_guild_war`, `G/cmd_gm.cpp:2766-2790`); see
    /// `game_state::guild`.
    GuildWarStop {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// `get_global_time()` in seconds, which stamps the end.
        now: u32,
        /// Where the game thread reports the ended rows, when the pair was a live war.
        reply: oneshot::Sender<Option<crate::game_state::WarEndOutcome>>,
    },
    /// Runs one `gwcancel` line (`do_cancel_guild_war`, `G/cmd_gm.cpp:2791-2805`); see
    /// `game_state::guild`.
    GuildWarCancel {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// `get_global_time()` in seconds, which stamps the end.
        now: u32,
        /// Where the game thread reports the cancel ran, when the pair was a live war.
        reply: oneshot::Sender<bool>,
    },
    /// Runs one `gwlist` line (`do_gwlist`, `G/cmd_gm.cpp:2760-2764`, plus
    /// `ShowGuildWarList`, `G/guild_manager.cpp:785-800`); see `game_state::guild`.
    GuildWarList {
        /// `get_global_time()` in seconds, which ages each war.
        now: u32,
        /// Where the game thread reports the pairs, in set order.
        reply: oneshot::Sender<Vec<crate::game_state::GuildWarListLine>>,
    },
    /// Runs one `gstate` line (`do_guild_state`, `G/cmd_gm.cpp:2806-2822`); see
    /// `game_state::guild`.
    GuildState {
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the guild's numbers, or none.
        reply: oneshot::Sender<Option<crate::game_state::GuildStateLine>>,
    },
    /// Drains the war ends the guild tick queued: the tick has no descriptor, so the
    /// outcomes wait in the world until a poller asks. See `game_state::guild`.
    TakeWarEnds {
        /// Where the game thread reports the queued outcomes, in tick order.
        reply: oneshot::Sender<Vec<crate::game_state::WarEndOutcome>>,
    },
    /// or a cancel.
    Trade {
        /// The VID of the character whose client sent the step.
        vid: common::vid::Vid,
        /// The step.
        step: TradeStep,
        /// Where the character stands.
        place: GroundPlace,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the step did.
        reply: oneshot::Sender<Result<TradeAnswer, MoveItemRefused>>,
    },
    /// Runs one safebox or mall step for the character online under `vid`.
    Safebox {
        /// The VID of the character whose client sent the step.
        vid: common::vid::Vid,
        /// The step.
        step: SafeboxStep,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports what the step did.
        reply: oneshot::Sender<Result<SafeboxAnswer, MoveItemRefused>>,
    },
    /// Runs one quest step for the character online under `vid`.
    Quest {
        /// The VID of the character whose client sent the step.
        vid: common::vid::Vid,
        /// The step.
        step: QuestStep,
        /// Where the character stands, whose map's NPCs a script finds.
        place: GroundPlace,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// The reader's account, for the step's scripts.
        account: QuestAccount,
        /// Where the game thread reports the records the step's script sent and the
        /// GRAND climbs its reads landed.
        reply: oneshot::Sender<Result<super::game_state::QuestStepOutcome, MoveItemRefused>>,
    },
    /// Runs one `CG_MOVE` of the player online under `vid` (`G/input_main.cpp:1757-1891`): the
    /// world judges it against the body, moves the body and relays the record to the body's
    /// view. Nothing answers; the world logs what it did.
    Move {
        /// The VID of the character whose client sent the move.
        vid: common::vid::Vid,
        /// The move as the client sent it.
        record: CgMove,
        /// Dropped once the move has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// Runs one `CG_SYNC_POSITION` of the player online under `vid`
    /// (`G/input_main.cpp:2010-2165`).
    SyncPosition {
        /// The VID of the character whose client sent the claims.
        vid: common::vid::Vid,
        /// The claims as the client sent them.
        packet: SyncPositionPacket,
        /// Where the game thread reports what the frame did, `None` when the character has no
        /// body. A close reason in it closes the descriptor.
        reply: oneshot::Sender<Option<SyncPositionResult>>,
    },
    /// Sends records the descriptor built for the players around the character online under
    /// `vid`, in order. The world sends nothing for a character with no body.
    Relay {
        /// The VID of the character the records are about.
        vid: common::vid::Vid,
        /// Each record with whom it goes to.
        records: Vec<(RelayScope, Vec<u8>)>,
        /// Dropped once the records are sent, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::Whisper` (`G/input_main.cpp:340-649`): the character online under `vid`
    /// sends a private line to the character `name` names.
    ///
    /// The record is the one the descriptor built, because the sender's name and the type are
    /// the sender's own. The world finds the target in the Channel it owns and writes the
    /// record to it; the answer says whether the target was there, because a target on
    /// another Channel is found in the process-wide client registry instead, which is the
    /// legacy's P2P manager.
    Whisper {
        /// The VID of the sending character.
        vid: common::vid::Vid,
        /// The target's name, as the client sent it.
        name: Vec<u8>,
        /// The line itself, which the world needs on its own when it has to leave
        /// the line for a target that is not there.
        text: Vec<u8>,
        /// The line, as the descriptor built it.
        record: Vec<u8>,
        /// Where the game thread reports where the line went.
        reply: oneshot::Sender<WhisperOutcome>,
    },
    /// `MessengerManager::Login`'s load half, reached once the descriptor read the
    /// rows: the list's companions, the team's GM names and every grant's level. See
    /// `game_state::messenger`.
    MessengerLogin {
        /// The VID of the character that logged in.
        vid: common::vid::Vid,
        /// The character Name bytes.
        name: Vec<u8>,
        /// The companions the store's rows carry, in the pair key's order.
        companions: Vec<Vec<u8>>,
        /// The GM names other than this character's own.
        team: Vec<Vec<u8>>,
        /// Every grant's name-to-level map, which the GM gates read.
        levels: std::collections::BTreeMap<Vec<u8>, u8>,
    },
    /// `CInputMain::PartyInvite`: the character online under `vid` invites the character
    /// `guest` names to its party. See `game_state::party`.
    PartyInvite {
        /// The VID of the inviting character.
        vid: common::vid::Vid,
        /// The guest VID the client sent.
        guest: u32,
        /// Dropped once the invite has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::PartyInviteAnswer`: the character online under `vid` answers the
    /// invite from the character `inviter` names. See `game_state::party`.
    PartyInviteAnswer {
        /// The VID of the answering character.
        vid: common::vid::Vid,
        /// The inviter VID the client sent.
        inviter: u32,
        /// The answer byte the client sent; non-zero accepts.
        accept: u8,
        /// Dropped once the answer has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `/party_request`: the character online under `vid` asks the VID `rest` names to
    /// take it into that player's party. See `game_state::party`.
    PartyRequest {
        /// The VID of the requesting character.
        vid: common::vid::Vid,
        /// The command line after the command word.
        rest: Vec<u8>,
        /// Dropped once the request has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `/party_request_accept`: the leader online under `vid` accepts the request from the
    /// guest `rest` names. See `game_state::party`.
    PartyRequestAccept {
        /// The VID of the accepting leader.
        vid: common::vid::Vid,
        /// The command line after the command word.
        rest: Vec<u8>,
        /// Dropped once the accept has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `/party_request_deny`: the leader online under `vid` denies the request from the
    /// guest `rest` names. See `game_state::party`.
    PartyRequestDeny {
        /// The VID of the denying leader.
        vid: common::vid::Vid,
        /// The command line after the command word.
        rest: Vec<u8>,
        /// Dropped once the deny has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `/ungroup`: the character online under `vid` leaves its party.
    /// See `game_state::party`.
    Ungroup {
        /// The VID of the leaving character.
        vid: common::vid::Vid,
        /// The caller's own PID, which the descriptor loaded.
        pid: u32,
        /// Dropped once the leave has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::PartyRemove`: the character online under `vid` removes the PID
    /// `pid` names from its party. See `game_state::party`.
    PartyRemove {
        /// The VID of the removing character.
        vid: common::vid::Vid,
        /// The PID the client sent.
        pid: u32,
        /// Dropped once the removal has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::PartySetState`: the character online under `vid` sets the role `role`
    /// names for the PID `pid` names. See `game_state::party`.
    PartySetState {
        /// The VID of the setting character.
        vid: common::vid::Vid,
        /// The PID the client sent.
        pid: u32,
        /// The role byte the client sent.
        role: u8,
        /// The flag byte the client sent.
        flag: u8,
        /// Dropped once the set has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::PartyParameter`: the character online under `vid` sets the
    /// distribution mode `mode` names. See `game_state::party`.
    PartyParameter {
        /// The VID of the setting character.
        vid: common::vid::Vid,
        /// The mode the client sent.
        mode: u8,
        /// Dropped once the set has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `MessengerManager::AuthToAdd`: the character online under `vid` answers the
    /// friend request the `requester` name named. See `game_state::messenger`.
    MessengerAuth {
        /// The VID of the answering character.
        vid: common::vid::Vid,
        /// The requester name the answer named.
        requester: Vec<u8>,
        /// Whether the answer denies: `deny` is every answer whose first byte is
        /// not `y` (`do_messenger_auth`, `cmd_general.cpp:2283-2284`).
        deny: bool,
        /// Answers whether the request became a friendship, which is when the caller
        /// writes the pair's rows. A deny never becomes one, even though the consume
        /// answers `true` (`G/messenger_manager.cpp:330-334`).
        reply: oneshot::Sender<bool>,
    },
    /// `MessengerManager::RequestToAdd` through the `ADD_BY_VID` arm: the character online
    /// under `vid` asks the character whose VID `target` names. See
    /// `game_state::messenger`.
    MessengerAddByVid {
        /// The VID of the asking character.
        vid: common::vid::Vid,
        /// The companion's VID the client sent.
        target_vid: u32,
        /// Dropped once the request has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `MessengerManager::RequestToAdd` through the `ADD_BY_NAME` arm, after the caller
    /// read the name's own grant level, which the GM gate reads. See
    /// `game_state::messenger`.
    MessengerAddByName {
        /// The VID of the asking character.
        vid: common::vid::Vid,
        /// The companion name the client sent, as a C string.
        name: Vec<u8>,
        /// The grant level of the name the caller read, and `None` in the test-server
        /// mode, where every level is `GM_IMPLEMENTOR` and the gate is silent.
        companion_level: Option<u8>,
        /// Dropped once the request has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `MessengerManager::RemoveFromList` through the REMOVE arm: the character online
    /// under `vid` takes the `companion` name away. See `game_state::messenger`.
    MessengerRemove {
        /// The VID of the removing character.
        vid: common::vid::Vid,
        /// The companion name the client sent, as a C string.
        companion: Vec<u8>,
        /// Dropped once the remove has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::PartyUseSkill`: the character online under `vid` uses the party
    /// skill `skill` names on the VID `target` names. See `game_state::party`.
    PartyUseSkill {
        /// The VID of the using character.
        vid: common::vid::Vid,
        /// The skill index the client sent.
        skill: u8,
        /// The target VID the client sent.
        target: u32,
        /// Dropped once the use has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// Whether the character online under `vid` holds a party (`ch->GetParty() !=
    /// NULL`, `G/input_main.cpp:959`): the descriptor asks before judging a party
    /// chat line. See `game_state::party`.
    PartyOf {
        /// The VID of the asking character.
        vid: common::vid::Vid,
        /// Where the game thread reports the membership.
        reply: oneshot::Sender<bool>,
    },
    /// Whether the character online under `vid` holds a guild (`ch->GetGuild() !=
    /// NULL`, `G/input_main.cpp:976`): the descriptor asks before judging a guild
    /// chat line. See `game_state::guild`.
    GuildOf {
        /// The VID of the asking character.
        vid: common::vid::Vid,
        /// Where the game thread reports the membership.
        reply: oneshot::Sender<bool>,
    },
    /// Runs one `CG_QUEST_CONFIRM` (`CInputMain::QuestConfirm`,
    /// `G/input_main.cpp:2236-2248`) for the character online under `vid`: the waiter
    /// `requestPID` names hears the answer in its own script, and its client hears what
    /// that script sent. The sender hears nothing.
    QuestConfirm {
        /// The VID of the character whose client answered.
        vid: common::vid::Vid,
        /// The answer, folded to `CONFIRM_YES` (1) unless it was 0.
        answer: u8,
        /// The `requestPID` the client sent: the waiter's store id.
        request_pid: u32,
        /// Where the game thread reports that the answer ran.
        reply: oneshot::Sender<()>,
    },
    /// Judges one `MARK_UPLOAD` (`GuildMarkUpload`, `G/input_login.cpp:1059-1090`): the
    /// guild, its level against `min_level`, and the image. An all-zero image deletes;
    /// anything else commits into the mark manager. Carries the 192 native words the
    /// upload frame decodes to, as `SaveMark`'s empty test reads them.
    MarkUpload {
        /// The opaque guild id the image belongs to.
        guild_id: u32,
        /// The 192 image words, in wire order. Boxed: 768 bytes would dwarf every
        /// other variant.
        image: Box<[u32; MARK_PIXELS]>,
        /// `[game] guild_mark_min_level`, clamped to `GUILD_MAX_LEVEL` at parse.
        min_level: u8,
        /// Where the game thread reports the judgement.
        reply: oneshot::Sender<crate::game_state::MarkUploadAnswer>,
    },
    /// Lists every (guild, mark) pair (`GuildMarkIDXList`, `G/input_login.cpp:1092-1114`):
    /// the world's `CopyMarkIdx`, in guild-id order, each truncated to its low 16 bits.
    MarkIdxList {
        /// Where the game thread reports the pairs.
        reply: oneshot::Sender<Vec<(u16, u16)>>,
    },
    /// Diffs one image slot against the client's CRC list (`GuildMarkCRCList`,
    /// `G/input_login.cpp:1116-1162`): the blocks whose CRC differs, in ascending
    /// position order with their compressed bytes. Unknown slots yield nothing.
    MarkCrcList {
        /// The opaque image-slot index the client asked about.
        img_idx: u8,
        /// The client's 80 CRC words, in wire order. Boxed with the upload image.
        crc_list: Box<[u32; MARK_CRC_WORDS]>,
        /// Where the game thread reports the diff triples.
        reply: oneshot::Sender<Vec<(u8, Vec<u8>)>>,
    },
    /// Commits one judged symbol upload (`GuildSymbolUpload`'s store half,
    /// `G/input_login.cpp:1025-1026`): the bytes go into the map, replacing any earlier
    /// ones. Unknown guilds are refused.
    SymbolUpload {
        /// The opaque guild id the symbol belongs to.
        guild_id: u32,
        /// The uploaded symbol bytes.
        data: Vec<u8>,
        /// Where the game thread reports whether the guild held it.
        reply: oneshot::Sender<bool>,
    },
    /// Answers one `SYMBOL_CRC` (`GuildSymbolCRC`, `G/input_login.cpp:1030-1057`): the
    /// stored bytes when the size or the CRC mismatches, `None` on a match or an
    /// unknown guild.
    SymbolCrc {
        /// The guild the client claims to hold a symbol for.
        guild_id: u32,
        /// The client's own CRC of its local symbol file.
        crc: u32,
        /// The client's own size of that file.
        size: u32,
        /// Where the game thread reports the mismatch bytes, if any.
        reply: oneshot::Sender<Option<Vec<u8>>>,
    },
    /// `CInputMain::Target`: the character online under `vid` targets the character `target`
    /// names, or none for VID 0. The world answers it with `GC_TARGET` when the target changes.
    Target {
        /// The VID of the targeting character.
        vid: common::vid::Vid,
        /// The VID the client sent.
        target: u32,
        /// Dropped once the answer is sent, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::TargetInfoLoad`: the character online under `vid` asks for the drop list
    /// of the character `target` names. The world answers with one `GC_TARGET_INFO` per entry.
    TargetInfo {
        /// The VID of the asking character.
        vid: common::vid::Vid,
        /// The VID the client sent.
        target: u32,
        /// Dropped once the answer is sent, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::RequestEventData`: the character online under `vid` asks for the event
    /// calendar of `month`. The world answers with one `GC_EVENT_INFO`.
    EventInfo {
        /// The VID of the asking character.
        vid: common::vid::Vid,
        /// The 0-based month the client sent.
        month: u8,
        /// Dropped once the answer is sent, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::UseSkill`: the character online under `vid` casts the skill `record`
    /// names on its target. The world sends nothing back to the caster on the wire for
    /// the cast itself (only the SP point change); see `game_state::pc_skill`.
    UseSkill {
        /// The VID of the casting character.
        vid: common::vid::Vid,
        /// The `CG_USE_SKILL` the client sent.
        record: protocol::cg_use_skill::CgUseSkill,
        /// Dropped once the cast has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::Attack`: the character online under `vid` attacks the VID `record` names.
    /// The world sends what the hit shows; see `game_state::fight`.
    Attack {
        /// The VID of the attacking character.
        vid: common::vid::Vid,
        /// The `CG_ATTACK` the client sent.
        record: protocol::cg_attack::CgAttack,
        /// Where the game thread reports the gold the hit stole.
        reply: oneshot::Sender<AttackAnswer>,
    },
    /// `CInputMain::Attack`'s `CG_(ADD_)FLY_TARGETING` arms: the character online
    /// under `vid` stores (or appends) the VID `record` names. The world relays
    /// what it stored; see `game_state::pc_skill`.
    FlyTarget {
        /// The VID of the targeting character.
        vid: common::vid::Vid,
        /// The `CG_FLY_TARGETING` or `CG_ADD_FLY_TARGETING` the client sent.
        record: protocol::cg_fly_target::CgFlyTarget,
        /// Dropped once the store has run, see [`Settled`].
        settled: oneshot::Sender<()>,
    },
    /// `CInputMain::Attack`'s `CG_SHOOT` arm: the character online under `vid`
    /// shoots the VIDs it stored. The world sends what the shot shows; see
    /// `game_state::pc_skill`.
    Shoot {
        /// The VID of the shooting character.
        vid: common::vid::Vid,
        /// The `CG_SHOOT` the client sent.
        record: protocol::cg_shoot::CgShoot,
        /// Where the game thread reports the gold the shot stole.
        reply: oneshot::Sender<AttackAnswer>,
    },
    /// `do_restart` or `do_restart_now` of the player online under `vid`
    /// (`G/cmd_general.cpp:471-706`); see `game_state::death`.
    Restart {
        /// The VID of the restarting player.
        vid: common::vid::Vid,
        /// Which restart the command asks for.
        kind: RestartKind,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports the gold the restart took.
        reply: oneshot::Sender<RestartAnswer>,
    },
    /// `do_skillup` of the player online under `vid`
    /// (`G/cmd_general.cpp:904-972`); see `game_state::pc_skill`.
    SkillUp {
        /// The VID of the skilling player.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the climbed slot, when the up lands.
        reply: oneshot::Sender<SkillUpAnswer>,
    },
    /// Runs one `/setblockmode` line (`do_setblockmode`, `G/cmd_general.cpp:2300-2309`):
    /// the byte becomes the character's own block mode, and the world's answer carries
    /// the `setblockmode %d` command line for the client's own queue plus the seven
    /// `game_option.block_*` quest flags for the store. See
    /// `game_state::whisper`.
    SetBlockMode {
        /// The VID of the blocking player.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line and the flag writes, when the line runs.
        reply: oneshot::Sender<SetBlockModeAnswer>,
    },
    /// Run one `/hide_costume` line: the line's part hides or shows its costume, the
    /// client hears `SetXCostumeHidden`, and the world's answer carries the
    /// `costume_option.hide_*` quest flag for the store. See `game_state::whisper`.
    HideCostume {
        /// The VID of the hiding player.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line and the flag write, when the line runs.
        reply: oneshot::Sender<HideCostumeAnswer>,
    },
    /// Run `CHARACTER::ChangeLanguage`'s world half for the player online under `vid`:
    /// the body's language becomes `language` and its look update goes to its view.
    /// See `game_state::affect`.
    ChangeLanguage {
        /// The VID of the switching player.
        vid: common::vid::Vid,
        /// The language the client switched to, 1 to 11.
        language: u8,
        /// Where the game thread reports whether the player is online.
        reply: oneshot::Sender<bool>,
    },
    /// Run one `/emotion_allow` or `/do_emotion` line: the consent insert or the
    /// emotion's gates and its broadcast to the caller's view. What the line shows
    /// rode the outbox already; the answer carries nothing. See `game_state::emotion`.
    Emotion {
        /// The VID of the emoting player.
        vid: common::vid::Vid,
        /// The command entry's name: `emotion_allow` or the emotion.
        command: String,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/priv_empire` line: the usage lines or the grant and its notice to
    /// every client. What the line shows rode the outbox already; the answer carries
    /// nothing. See `game_state::empire_priv`.
    PrivEmpire {
        /// The VID of the granting GM.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/eventflag` line: the flag takes the value, and the caller hears
    /// the `RequestSetEventFlag` line. Nothing otherwise. See `game_state::event_flag`.
    EventFlag {
        /// The VID of the setting GM.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `CG_DAILY_GIFT` for the character online under `vid`: a close, an open
    /// or a collect. The records reach the client through the world; the answer carries
    /// the rows to store.
    DailyGift {
        /// The VID of the character whose client sent the record.
        vid: common::vid::Vid,
        /// The decoded step.
        step: crate::game_state::DailyGiftStep,
        /// What the descriptor knows of the character.
        mover: Mover,
        /// Where the game thread reports the rows the step changed.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/daily_gift_event` line: the usage, or the event starts or stops. See
    /// `game_state::daily_gift`.
    DailyGiftEvent {
        /// The VID of the commanding GM.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/geteventflag` line: every flag is listed on the caller, one
    /// `name value` line each. See `game_state::event_flag`.
    GetEventFlag {
        /// The VID of the asking GM.
        vid: common::vid::Vid,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/horse_state`, `/horse_level` or `/horse_set_stat` line. See
    /// `game_state::horse`. Answers what `/horse_level` planted in the target's horse
    /// skill, for the store.
    HorseLine {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Which of the three lines.
        line: HorseLine,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<Option<crate::game_state::SetSkillOtherLanded>>,
    },
    /// Run one `/xmas_boom`, `/xmas_snow`, `/xmas_santa`, `/eclipse` or `/eventhelper`
    /// line: the named flag is set, or the firework sellers stand or go. See
    /// `game_state::event_flag`.
    EventLine {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Which of the five lines.
        line: EventLine,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/open_global_rank` or `/rank_end_season` line. Answers the season's end the
    /// descriptor stores, or `None`. See `game_state::rank_global`.
    RankLine {
        /// The VID of the character that typed the line.
        vid: common::vid::Vid,
        /// Which line.
        line: RankLine,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the season's end.
        reply: oneshot::Sender<Option<crate::game_state::SeasonEnd>>,
    },
    /// Run one `/give_reward` line: the busy, event and letter gates first, then the
    /// six letters go and the box draw lands. Answers the stored rows, or `None`
    /// for a refusal the caller already heard. See `game_state::letters`.
    GiveReward {
        /// The VID of the trading character.
        vid: common::vid::Vid,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the trade's rows.
        reply: oneshot::Sender<Option<crate::game_state::LettersTrade>>,
    },
    /// Run one `/socketitem` line's give: one item of `vnum` by `AutoGiveItem` with its
    /// first `sockets` sockets set to 1. Answers the stored rows, or `None` when nothing
    /// was made, which the caller already heard. See `game_state::socket_item`.
    SocketItem {
        /// The VID of the GM.
        vid: common::vid::Vid,
        /// The item vnum.
        vnum: u32,
        /// How many leading sockets take 1.
        sockets: usize,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the give's rows.
        reply: oneshot::Sender<Option<crate::game_state::LettersTrade>>,
    },
    /// Run one of the small GM lines in `game_state::gm_lines`: the item ID list, the mob
    /// count, the effect, and the two socket writes. What a line tells rides the outbox;
    /// the answer is the rows a socket write changed, for the caller to store.
    GmLine {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Which line.
        line: GmLine,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the rows, or `None` when nothing is stored.
        reply: oneshot::Sender<Option<crate::game_state::GmRows>>,
    },
    /// Asks the world for one dungeon rank's flag (`/dungeon_info rank <mob> <rank>`,
    /// `do_dungeon_info`, `G/cmd_general.cpp:4099-4107`): silent past a parse or table
    /// miss, else the flag the descriptor reads and fans. See
    /// `game_state::dungeon_info`.
    DungeonRank {
        /// The VID of the asking character.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the flag to read, or `None` on silence.
        reply: oneshot::Sender<Option<crate::game_state::DungeonRankAsk>>,
    },
    /// Run one `/letters_open` line: the caller hears the box's table and opens the
    /// window. See `game_state::letters`.
    LettersOpen {
        /// The VID of the opening character.
        vid: common::vid::Vid,
        /// What the descriptor knows of the character.
        mover: crate::item_move::Mover,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/stat` or `/stat-` line: the named attribute rises or falls by 1,
    /// the records ride the outbox, and the held row's numbers follow. See
    /// `game_state::stat`.
    Stat {
        /// The VID of the raising character.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Whether the line lowers (`stat-`) rather than raises (`stat`).
        lower: bool,
        /// Where the game thread reports the new held numbers, when the line runs.
        reply: oneshot::Sender<Option<crate::game_state::StatHeld>>,
    },
    /// Run one `/stat_val` line: the named attribute rises by the line's
    /// amount for stat points, the records ride the outbox, and the held
    /// row's numbers follow. See `game_state::stat`.
    StatVal {
        /// The VID of the raising character.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the new held numbers, when the line runs.
        reply: oneshot::Sender<Option<crate::game_state::StatHeld>>,
    },
    /// Run one `*+` line: the entry's attribute rises by the line's amount
    /// for stat points, the records ride the outbox, and the held row's
    /// numbers follow. See `game_state::stat`.
    StatPlus {
        /// The VID of the raising character.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// The attribute the entry names, as a point slot.
        kind: usize,
        /// Where the game thread reports the new held numbers, when the line runs.
        reply: oneshot::Sender<Option<crate::game_state::StatHeld>>,
    },
    /// Run one `t*` line: the named character's entry attribute moves by the
    /// line's delta for the named character's stat points, the records ride
    /// the outboxes, and the target's held numbers follow for its row. See
    /// `game_state::stat`.
    SetStat {
        /// The VID of the calling character.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// The attribute the entry names, as a point slot.
        kind: usize,
        /// Where the game thread reports the target's name and new held
        /// numbers, when the line runs.
        reply: oneshot::Sender<Option<crate::game_state::SetStatHeld>>,
    },
    /// Run one `/stat_reset` line: the reset count rises to 12, the record
    /// rides the outbox, and the held row's numbers follow. See
    /// `game_state::stat`.
    StatReset {
        /// The VID of the resetting character.
        vid: common::vid::Vid,
        /// Where the game thread reports the new held numbers, when the line runs.
        reply: oneshot::Sender<Option<crate::game_state::StatHeld>>,
    },
    /// Run one `/pkmode` line: the caller's PK mode becomes the line's number,
    /// and its update fans to its view. See `game_state::pk_mode`.
    PkMode {
        /// The VID of the switching character.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/set_walk_mode` or `/set_run_mode` line: the caller walks or
    /// runs, and a change fans its walk-mode record to its view. See
    /// `game_state::walk_mode`.
    WalkMode {
        /// The VID of the walking character.
        vid: common::vid::Vid,
        /// Whether the line walks rather than runs.
        walk: bool,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/can_dead` or `/cannot_dead` line: the caller's
    /// `cannot_dead` clears or sets. Nothing rides, nothing is stored: the
    /// flag lives in memory, as legacy's does. See `game_state::death`.
    CanDead {
        /// The VID of the flagged character.
        vid: common::vid::Vid,
        /// Whether the line clears (`can_dead`) rather than sets.
        can: bool,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/all_skill_master` line: every usable skill becomes 40, and
    /// the planted vnums with their left-alone next-reads ride back for the
    /// store. See `game_state::skill_admin`.
    AllSkillMaster {
        /// The VID of the mastered character.
        vid: common::vid::Vid,
        /// Where the game thread reports what the line planted.
        reply: oneshot::Sender<Vec<crate::game_state::SetSkillLanded>>,
    },
    /// Run one `/item_full_set` line: the strips and the job's kit. See
    /// `game_state::full_set`.
    ItemFullSet {
        /// The VID of the kitted character.
        vid: common::vid::Vid,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/attr_full_set` line: the forced pairs on each worn piece. See
    /// `game_state::full_set`.
    AttrFullSet {
        /// The VID of the kitted character.
        vid: common::vid::Vid,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/full_set` line: the master line, the kit, the forced pairs. See
    /// `game_state::full_set`.
    FullSet {
        /// The VID of the kitted character.
        vid: common::vid::Vid,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Where the game thread reports the planted skills with the moved items.
        reply: oneshot::Sender<
            Result<(Vec<crate::game_state::SetSkillLanded>, MovedItems), MoveItemRefused>,
        >,
    },
    /// Run one `/slow` line: ten fresh 12019s, numbered in socket 4. See
    /// `game_state::item_purge`.
    Slow {
        /// The VID of the given character.
        vid: common::vid::Vid,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/ipurge` line: every flat and dragon soul cell cleared. See
    /// `game_state::item_purge`.
    ItemPurge {
        /// The VID of the purged character.
        vid: common::vid::Vid,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/book` line: one fresh 50300 with the named skill in socket 0. See
    /// `game_state::item_purge`.
    Book {
        /// The VID of the given character.
        vid: common::vid::Vid,
        /// The line past the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/console` line: the `ConsoleEnable` command line. See
    /// `game_state::item_purge`.
    Console {
        /// The VID of the told character.
        vid: common::vid::Vid,
        /// Where the game thread reports the telling.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/set` line: the named PC's field set. See `game_state::set_attr`.
    SetAttr {
        /// The VID of the calling character.
        vid: common::vid::Vid,
        /// The line past the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the gold Transfer, if the arm stored one.
        reply: oneshot::Sender<Option<crate::game_state::SetAttrStored>>,
    },
    /// Run one `/refine_rod`, `/refine_pick` or `/max_pick` line: the named cell's
    /// tool refines, maxes, or both. See `GameState::run_tool_refine_line`.
    ToolRefineLine {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Which tool line runs.
        line: world::character::ToolRefine,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/use_item` line: the named cell through `UseItem`, or the no-item
    /// line with an empty step. See `GameState::run_use_item_line`.
    UseItemLine {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/cards` line: the Okey game's open, pull, end, destroy, accept and
    /// restore arms. See `GameState::run_cards_line`.
    CardsLine {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/ride` line: the ride toggle, or the use of the ride item or mount seal it
    /// finds. See `GameState::run_ride_line`.
    RideLine {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/dragon_soul` line (`do_dragon_soul`, `G/cmd_gm.cpp:5024-5047`): the
    /// named deck activates, or every deck deactivates. See
    /// `game_state::dragon_soul::run_dragon_soul`.
    DragonSoul {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/ds_list` line (`do_ds_list`, `G/cmd_gm.cpp:5049-5059`): the caller
    /// hears one line per dragon soul stone it holds. See
    /// `game_state::dragon_soul::run_ds_list`.
    DsList {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/ds_change_attr` line (`do_ds_change_attr`, `G/cmd_general.cpp:3864-3931`):
    /// the named soul-inventory stone's attributes redrawn. See
    /// `game_state::dragon_soul::run_ds_change_attr`.
    DsChangeAttr {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/multi_farm` line (`do_multi_farm`, `G/cmd_general.cpp:304-317`): the
    /// caller's farm status toggles past its 10 s protect. See
    /// `game_state::multi_farm::run_multi_farm`.
    MultiFarm {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The descriptor's host address, which the registry keys on.
        addr: String,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/dice` line (`do_dice`, `G/cmd_general.cpp:3455-3489`): the roll to the
    /// caller, or the party. See `game_state::dice::run_dice`.
    Dice {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/gift` line (`do_gift`, `G/cmd_general.cpp:3251-3254`): the caller hears
    /// the `gift` command line. See `game_state::dice::run_gift`.
    Gift {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/state` line (`do_state`, `G/cmd_gm.cpp:1017-1202`): the target's
    /// dump, told to the caller. See `game_state::state::run_state`.
    State {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/inventory` line (`do_inventory`, `G/cmd_general.cpp:3210-3250`):
    /// the caller's own cells, told to the caller. See
    /// `game_state::state::run_inventory`.
    Inventory {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/hair` line (`do_hair`, `G/cmd_general.cpp:3200-3208`): the caller's
    /// hair affect as one line. See `game_state::costume::run_hair`.
    Hair {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/advance_skill_select` line (`do_advance_skill_select`,
    /// `G/cmd_general.cpp:3982-4002`): the caller's skill group becomes the
    /// line's, then the skills clear. See `game_state::skill_admin`.
    AdvanceSkill {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the group with the clear.
        reply: oneshot::Sender<Option<crate::game_state::SetJobLanded>>,
    },
    /// Run one `/do_click_special_inv` line (`do_click_special_inv`,
    /// `G/cmd_general.cpp:4006-4009`): the `ShowSpecialInventory` command
    /// line. See `game_state::skill_admin::run_click_special_inv`.
    ClickSpecialInv {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/shutdown` line (`do_shutdown`, `G/cmd_general.cpp:265-275`): the 10's
    /// notice goes out and its countdown starts. See `game_state::shutdown::run_shutdown`.
    Shutdown {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one exit line (`do_cmd`, `G/cmd_general.cpp:395-448`): the cancel or the
    /// notice, and past the gate the countdown. See `game_state::timed_exit::run_do_cmd`.
    DoCmd {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// Which exit the line asks for.
        kind: ExitKind,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/pvp` line (`do_pvp`, `G/cmd_general.cpp:848-879`): the challenge or
    /// the accept of the word's VID. See `game_state::duel::run_pvp`.
    Pvp {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The line past the command word, as the client sent it.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/reload` line (`do_reload`, `G/cmd_gm.cpp:2639-2733`): the tells and the
    /// table swaps over the files the descriptor read. See `game_state::reload`.
    Reload {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The line past the command word, as the client sent it.
        rest: Vec<u8>,
        /// What the descriptor read for the line: each file's bytes, or `None` when the
        /// file could not be read.
        read: crate::game_state::ReloadRead,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/fish_simul` line (`do_fishing_simul`, `G/cmd_gm.cpp:2227-2251`):
    /// the always-told usage, then the simulation's lines. See
    /// `game_state::fish_simul::run_fish_simul`.
    FishSimul {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The line past the command word, as the client sent it.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Run one `/costume` line (`do_costume`, `G/cmd_general.cpp:3089-3198`): the
    /// costume status lines, and the named piece comes off. See
    /// `game_state::costume::run_costume`.
    Costume {
        /// The VID of the player whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// What the descriptor knows of the character that the line reads.
        mover: Mover,
        /// Where the game thread reports the moved items.
        reply: oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    },
    /// Run one `/do_clear_affect` line: `ClearAffect(true)` on the caller. See
    /// `game_state::affect::run_clear_affect`.
    ClearAffect {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// The remaining `AFFECT_BLOCK_CHAT` duration of the character online under
    /// `vid` (`FindAffect`, `G/char_affect.cpp:129-142`), or `None` when it holds
    /// none: the descriptor asks before judging a chat line. See
    /// `game_state::affect`.
    BlockChatOf {
        /// The VID of the speaking character.
        vid: common::vid::Vid,
        /// Where the game thread reports the remaining duration.
        reply: oneshot::Sender<Option<i32>>,
    },
    /// Runs one `/block_chat` or `/jy` line (`do_block_chat`,
    /// `G/cmd_gm.cpp:3364-3433`): the named character is silenced for the parsed
    /// time, `0` included. See `game_state::whisper`.
    BlockChat {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports whether a row moved.
        reply: oneshot::Sender<BlockChatAnswer>,
    },
    /// Runs one `/block_chat_list` line (`do_block_chat_list`,
    /// `G/cmd_gm.cpp:3298-3310`): the caller hears one `%s %s sec` line per
    /// silenced character, or `No one currently blocked`. See
    /// `game_state::whisper`.
    BlockChatList {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/dc` line (`do_disconnect`, `G/cmd_gm.cpp:1755-1782`): the
    /// named character's descriptor closes, or the caller hears the usage,
    /// the no-such-player line, or the self refusal. See
    /// `game_state::whisper`.
    Dc {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/transfer` line (`do_transfer`, `G/cmd_gm.cpp:102-151`): the
    /// named character warps to the caller's side, or the caller hears the
    /// usage, the no-character line, the cross-Channel line, or the self
    /// refusal. See `game_state::whisper`.
    Transfer {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/kill` line (`do_kill`, `G/cmd_gm.cpp:1784-1805`): the named
    /// character dies, or the caller hears the usage or the no-such-player
    /// line. See `game_state::whisper`.
    Kill {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/poison` line (`do_poison`, `G/cmd_gm.cpp:1808-1829`): the
    /// named character is poisoned, or the caller hears the usage or the
    /// no-such-player line. See `game_state::whisper`.
    Poison {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/stun` line (`do_stun`, `G/cmd_gm.cpp:87-90`): the named
    /// character is stunned, or the caller hears the usage, the miss, or the
    /// confirmation. See `game_state::whisper`.
    Stun {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/getqf` line (`do_getqf`, `G/cmd_gm.cpp:2863-2888`): the
    /// target's quest flags, one line each. See `game_state::whisper`.
    GetQf {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/setqf` line (`do_setqf`, `G/cmd_gm.cpp:2958-2992`): sets the
    /// target's quest flag and chats the confirmation. See `game_state::whisper`.
    SetQf {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/delqf` line (`do_delqf`, `G/cmd_gm.cpp:2994-3027`): deletes
    /// the target's quest flag and chats the outcome. See `game_state::whisper`.
    DelQf {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/set_state` line (`do_set_state`, `G/cmd_gm.cpp:2891-2956`):
    /// the target's quest moves to the named state. See `game_state::whisper`.
    SetState {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/qf` line (`do_qf`, `G/cmd_gm.cpp:2326-2364`): the
    /// caller's current quest moves to the named state. See
    /// `game_state::whisper`.
    Qf {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/clear_quest` line (`do_clear_quest`,
    /// `G/cmd_gm.cpp:3732-3743`): the caller's quest forgets every flag and
    /// rests at 0. See `game_state::whisper`.
    ClearQuest {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/view_equip` line (`do_view_equip`,
    /// `G/cmd_general.cpp:2395-2426`): the caller views the VID's worn
    /// equipment. See `game_state::whisper`.
    ViewEquip {
        /// The VID of the character whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/warp` line (`do_warp`, `G/cmd_gm.cpp:399-457`): the caller
    /// warps to the named character or the metres, or hears the usage or the
    /// miss. See `game_state::whisper`.
    Warp {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/private` line (`do_private`, `G/cmd_gm.cpp:2301-2325`): the
    /// caller warps to a private copy of the named map, or hears the usage or
    /// the miss. See `game_state::whisper`.
    Private {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/rewarp` line (`do_rewarp`, `G/cmd_gm.cpp:459-465`): the
    /// caller rewarps in place. See `game_state::whisper`.
    Rewarp {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// The asking half of one `/observer_exit` line (`do_observer_exit`,
    /// `G/cmd_general.cpp:2370-2393`): the sort lock when the caller observes, else `None`.
    /// The descriptor runs `ExitToSavedLocation`'s warp first. See `game_state::observer`.
    ObserverExit {
        /// The VID of the character whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the lock, or that the caller observes nothing.
        reply: oneshot::Sender<Option<u32>>,
    },
    /// The clearing half of one `/observer_exit` line: the exit's clear, which dances a
    /// refused warp's character back into view and tells the line. After an accepted warp
    /// the character is gone and there is nothing to clear. See `game_state::observer`.
    ObserverSettled {
        /// The VID of the character whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the clear ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/goto` line (`do_goto`, `G/cmd_gm.cpp:313-397`): the caller
    /// shows at the metres or warps to the named map, or hears the usage.
    /// See `game_state::whisper`.
    Goto {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/level` line (`do_level`, `G/cmd_gm.cpp:2740-2758`): the
    /// caller's level becomes the line's, or it hears the syntax.
    /// See `game_state::level`.
    Level {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// The row's gold, which the points pair carries.
        gold: i64,
        /// The row's voice, which the points record carries.
        voice: u8,
        /// Where the game thread reports what the line reset.
        reply: oneshot::Sender<Option<crate::game_state::LevelHeld>>,
    },
    /// Runs one `/advance` line (`do_advance`, `G/cmd_gm.cpp:2083-2106`): the
    /// named character's level becomes the line's, or the caller hears the
    /// syntax or the miss. See `game_state::level`.
    Advance {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// The caller's row's gold, which the points pair carries.
        gold: i64,
        /// The caller's row's voice, which the points record carries.
        voice: u8,
        /// Where the game thread reports the target with what the line reset.
        reply: oneshot::Sender<Option<crate::game_state::AdvanceLanded>>,
    },
    /// Runs one `/reset` line (`do_reset`, `G/cmd_gm.cpp:2076-2081`): the
    /// caller's hit and spell pools refill. See `game_state::level`.
    Reset {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the filled pools, when the line runs.
        reply: oneshot::Sender<Option<crate::game_state::ResetHeld>>,
    },
    /// Grants the `rank`th top-player affect (1 to 3) after a `/level` reached
    /// 120 (`SetTopPlayerEffect`, `G/char.cpp:11255-11284`). The step counted
    /// the store's `player_top` rows and stood this caller's.
    GrantTopAffect {
        /// The VID of the character that reached 120.
        vid: common::vid::Vid,
        /// The standing row's rank: 1, 2 or 3.
        rank: u8,
        /// Where the game thread reports the grant landed.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/setskill` line (`do_setskill`, `G/cmd_gm.cpp:2574-2605`):
    /// the caller's named skill becomes the line's level, or it hears the
    /// syntax or the miss. See `game_state::skill_admin`.
    SetSkill {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports what the line set.
        reply: oneshot::Sender<Option<crate::game_state::SetSkillLanded>>,
    },
    /// Runs one `/setskillother` line (`do_setskillother`,
    /// `G/cmd_gm.cpp:2528-2572`): the named character's skill becomes the
    /// line's level. See `game_state::skill_admin`.
    SetSkillOther {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the target with the set.
        reply: oneshot::Sender<Option<crate::game_state::SetSkillOtherLanded>>,
    },
    /// Runs one `/setskillpoint` line (`do_set_skill_point`,
    /// `G/cmd_gm.cpp:2607-2618`): the caller's skill counter becomes the
    /// line's number. See `game_state::skill_admin`.
    SetSkillPoint {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the counter.
        reply: oneshot::Sender<Option<i32>>,
    },
    /// Runs one `/setjob` line (`do_set_skill_group`,
    /// `G/cmd_gm.cpp:2620-2631`): the caller's skill group becomes the
    /// line's, then the skills clear. See `game_state::skill_admin`.
    SetJob {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the group with the clear.
        reply: oneshot::Sender<Option<crate::game_state::SetJobLanded>>,
    },
    /// Runs one `/temp` line (`do_temp`, `G/cmd_gm.cpp:4368-4388`): the empire's
    /// treasury moves. See `game_state::monarch`.
    Temp {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the slot with the landed cell.
        reply: oneshot::Sender<Option<(u8, i64)>>,
    },
    /// Runs one `/setmonarch` line (`do_setmonarch`, `G/cmd_gm.cpp:4267-4281`): the name's
    /// account's empire names the name its monarch and refills its treasury from the
    /// name's gold. See `game_state::monarch`.
    SetMonarch {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// The store's candidate lookup: the empire, pid and gold behind the name.
        candidate: Option<(u8, u32, i64)>,
        /// Where the game thread reports what the line named.
        reply: oneshot::Sender<Option<crate::game_state::SetMonarchLanded>>,
    },
    /// Runs one `/rmmonarch` line (`do_rmmonarch`, `G/cmd_gm.cpp:4283-4297`): the
    /// caller's empire clears when the name is its monarch. See `game_state::monarch`.
    RmMonarch {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the cleared empire.
        reply: oneshot::Sender<Option<u8>>,
    },
    /// Runs one `/invisible` line (`do_invisibility`,
    /// `G/cmd_gm.cpp:2253-2264`): the caller's invisibility toggles. See
    /// `game_state::affect::toggle_invisible`.
    Invisible {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/cooltime` line (`do_cooltime`, `G/cmd_gm.cpp:2735-2738`):
    /// the caller's skill cooldowns stop gating. See
    /// `game_state::pc_skill::disable_cooltime`.
    Cooltime {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/affect_remove` line (`do_affect_remove`,
    /// `G/cmd_gm.cpp:3865-3918`): the caller's affect list, or the named
    /// affect's removal. See `game_state::affect::affect_remove`.
    AffectRemove {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/purge` or `/weaken` line (`do_purge`, `do_weaken`,
    /// `G/cmd_gm.cpp:816-868`, `:2824-2861`): every NPC of the sectrees around
    /// the caller is destroyed or set to 10 hit points. See
    /// `game_state::regen::purge_around`.
    Purge {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Whether the line weakens rather than destroys.
        weaken: bool,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/respawn` line (`do_respawn`, `G/cmd_gm.cpp:2108-2123`): the
    /// regens holding the caller's position, or every regen when the line
    /// says `all`, fire one Pulse on. See `game_state::regen::respawn_around`.
    Respawn {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/mob` line (`do_mob`, `G/cmd_gm.cpp:699-758`): the named
    /// mob stands around the caller, or the usage or the miss answers. See
    /// `game_state::regen::mob_around`.
    Mob {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/mob_ld` line (`do_mob_ld`, `G/cmd_gm.cpp:760-815`): the
    /// named mob at the named metres, or the usage or the miss answers. See
    /// `game_state::regen::mob_ld_at`.
    MobLd {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/ma` line (`do_mob_aggresive`, `G/cmd_gm.cpp:639-698`): the
    /// named mob stands around the caller with the aggressive bit set, or
    /// the usage or the miss answers. See `game_state::regen::mob_flagged`.
    MobAggressive {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/mc` line (`do_mob_coward`, `G/cmd_gm.cpp:557-617`): the
    /// named mob stands around the caller with the coward bit set, or the
    /// usage or the miss answers. See `game_state::regen::mob_flagged`.
    MobCoward {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/mm` line (`do_mob_map`, `G/cmd_gm.cpp:618-637`): the
    /// numbered mob at a random point of the caller's map, or the syntax,
    /// the stood line, or the failure answers. See
    /// `game_state::regen::mob_somewhere`.
    MobMap {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/detaillog` line (`do_detaillog`, `G/cmd_general.cpp:2242-2245`):
    /// the caller's detail flag flips. See `game_state::detail_log::toggle_detail_log`.
    DetailLog {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/monsterlog` line (`do_monsterlog`, `G/cmd_general.cpp:2247-2250`):
    /// the caller's monster subscription flips. See
    /// `game_state::detail_log::toggle_monster_log`.
    MonsterLog {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/polymorph` line (`do_polymorph`, `G/cmd_gm.cpp:3049-3068`):
    /// the caller shows as the named mob vnum, or unmorphs on an empty or job
    /// vnum. See `game_state::polymorph::run_polymorph`.
    Polymorph {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The line's words after the command name.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/forgetme` line (`do_forgetme`, `G/cmd_gm.cpp:3029-3032`):
    /// every monster around whose victim is the caller drops it, then the
    /// caller hides for 5 seconds. See `game_state::mob_battle::forget_my_attacker`.
    ForgetMe {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/aggregate` line (`do_aggregate`, `G/cmd_gm.cpp:3034-3037`):
    /// each victimless monster around within 5000, with a one-in-two draw,
    /// begins to fight the caller. See `game_state::mob_battle::aggregate_monster`.
    Aggregate {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/attract_ranger` line (`do_attract_ranger`, `G/cmd_gm.cpp:3039-3042`):
    /// each monster around whose victim is none or the caller and whose attack range
    /// is past 150 is cut to 150 for 3 minutes. See
    /// `game_state::mob_battle::attract_ranger`.
    AttractRanger {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/pull_monster` line (`do_pull_monster`, `G/cmd_gm.cpp:3044-3047`):
    /// each monster around 100 to 3000 out is pulled 300 toward the caller. See
    /// `game_state::mob_battle::pull_monster`.
    PullMonster {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/group` line (`do_group`, `G/cmd_gm.cpp:537-555`): the group
    /// in the 500-box around the caller, or the usage answers. See
    /// `game_state::regen::group_around`.
    Group {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Runs one `/grrandom` line (`do_group_random`, `G/cmd_gm.cpp:521-536`):
    /// one group drawn uniformly from the group-group, spawned as `/group`
    /// spawns it, or the usage answers. See
    /// `game_state::regen::group_random_around`.
    GroupRandom {
        /// The VID of the GM whose client sent the line.
        vid: common::vid::Vid,
        /// The chat line after the command word.
        rest: Vec<u8>,
        /// Where the game thread reports the line ran.
        reply: oneshot::Sender<()>,
    },
    /// Requests terminal loop shutdown.
    Stop,
}

impl GameCommand {
    /// Whether the dispatch runs this through `apply_chat_line`: the emotion
    /// and the priv, silence, disconnect, transfer, kill, poison, stun, warp,
    /// rewarp, goto, level, advance, top grant, skill-admin, invisible,
    /// cooltime, affect-remove, flag, letters, stat, PK, walk, master, use and
    /// clear lines.
    pub fn is_chat_line(&self) -> bool {
        matches!(
            self,
            GameCommand::Emotion { .. }
                | GameCommand::PrivEmpire { .. }
                | GameCommand::BlockChatOf { .. }
                | GameCommand::BlockChat { .. }
                | GameCommand::BlockChatList { .. }
                | GameCommand::Dc { .. }
                | GameCommand::Transfer { .. }
                | GameCommand::Kill { .. }
                | GameCommand::Poison { .. }
                | GameCommand::Stun { .. }
                | GameCommand::GetQf { .. }
                | GameCommand::SetQf { .. }
                | GameCommand::DelQf { .. }
                | GameCommand::SetState { .. }
                | GameCommand::Qf { .. }
                | GameCommand::ClearQuest { .. }
                | GameCommand::ViewEquip { .. }
                | GameCommand::Warp { .. }
                | GameCommand::Private { .. }
                | GameCommand::Rewarp { .. }
                | GameCommand::ObserverExit { .. }
                | GameCommand::ObserverSettled { .. }
                | GameCommand::Goto { .. }
                | GameCommand::Level { .. }
                | GameCommand::Advance { .. }
                | GameCommand::Reset { .. }
                | GameCommand::GrantTopAffect { .. }
                | GameCommand::SetSkill { .. }
                | GameCommand::SetSkillOther { .. }
                | GameCommand::SetSkillPoint { .. }
                | GameCommand::SetJob { .. }
                | GameCommand::Temp { .. }
                | GameCommand::SetMonarch { .. }
                | GameCommand::RmMonarch { .. }
                | GameCommand::Invisible { .. }
                | GameCommand::Cooltime { .. }
                | GameCommand::AffectRemove { .. }
        ) || self.is_chat_line_tail()
    }

    /// The tail of [`GameCommand::is_chat_line`]: the purge, mob, log, flag,
    /// letters, stat, PK, walk, master, use and clear lines. Split out so the head
    /// stays under the line lint.
    fn is_chat_line_tail(&self) -> bool {
        matches!(
            self,
            GameCommand::Purge { .. }
                | GameCommand::Respawn { .. }
                | GameCommand::Mob { .. }
                | GameCommand::MobLd { .. }
                | GameCommand::MobAggressive { .. }
                | GameCommand::MobCoward { .. }
                | GameCommand::MobMap { .. }
                | GameCommand::DetailLog { .. }
                | GameCommand::MonsterLog { .. }
                | GameCommand::Polymorph { .. }
                | GameCommand::ForgetMe { .. }
                | GameCommand::Aggregate { .. }
                | GameCommand::AttractRanger { .. }
                | GameCommand::PullMonster { .. }
                | GameCommand::Group { .. }
                | GameCommand::GroupRandom { .. }
                | GameCommand::EventFlag { .. }
                | GameCommand::GetEventFlag { .. }
                | GameCommand::DailyGiftEvent { .. }
                | GameCommand::EventLine { .. }
                | GameCommand::RankLine { .. }
                | GameCommand::GiveReward { .. }
                | GameCommand::SocketItem { .. }
                | GameCommand::GmLine { .. }
                | GameCommand::DungeonRank { .. }
                | GameCommand::LettersOpen { .. }
                | GameCommand::Stat { .. }
                | GameCommand::StatVal { .. }
                | GameCommand::StatPlus { .. }
                | GameCommand::SetStat { .. }
                | GameCommand::StatReset { .. }
                | GameCommand::PkMode { .. }
                | GameCommand::WalkMode { .. }
                | GameCommand::CanDead { .. }
                | GameCommand::AllSkillMaster { .. }
                | GameCommand::ItemFullSet { .. }
                | GameCommand::AttrFullSet { .. }
                | GameCommand::FullSet { .. }
                | GameCommand::ToolRefineLine { .. }
                | GameCommand::UseItemLine { .. }
                | GameCommand::CardsLine { .. }
                | GameCommand::RideLine { .. }
                | GameCommand::ClearAffect { .. }
                | GameCommand::DragonSoul { .. }
                | GameCommand::Costume { .. }
                | GameCommand::DsList { .. }
                | GameCommand::DsChangeAttr { .. }
                | GameCommand::Dice { .. }
                | GameCommand::MultiFarm { .. }
                | GameCommand::Gift { .. }
                | GameCommand::Hair { .. }
                | GameCommand::AdvanceSkill { .. }
                | GameCommand::ClickSpecialInv { .. }
                | GameCommand::FishSimul { .. }
                | GameCommand::Shutdown { .. }
                | GameCommand::DoCmd { .. }
                | GameCommand::Pvp { .. }
                | GameCommand::Reload { .. }
                | GameCommand::State { .. }
                | GameCommand::Inventory { .. }
                | GameCommand::Slow { .. }
                | GameCommand::ItemPurge { .. }
                | GameCommand::Book { .. }
                | GameCommand::Console { .. }
                | GameCommand::SetAttr { .. }
        )
    }
}
/// How a character leaves the world.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Departure {
    /// Its client disconnects.
    Disconnect,
    /// Its client warps, which takes the body out of its sectree first (`G/char.cpp:6751-6752`).
    Warp,
}

/// The global rank's lines (`G/cmd.cpp:308-309`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RankLine {
    /// `/open_global_rank` (`do_open_global_rank`): the board.
    Open,
    /// `/rank_end_season` (`do_global_rank_end_season`): the season's end.
    EndSeason,
}

/// The commands that set one named flag, or spawn the event helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventLine {
    /// `/xmas_boom` (`do_xmas`, `SCMD_XMAS_BOOM`).
    XmasBoom,
    /// `/xmas_snow` (`SCMD_XMAS_SNOW`).
    XmasSnow,
    /// `/xmas_santa` (`SCMD_XMAS_SANTA`).
    XmasSanta,
    /// `/eclipse` (`do_eclipse`).
    Eclipse,
    /// `/eventhelper` (`do_event_helper`).
    EventHelper,
}

/// The GM horse lines (`G/cmd_gm.cpp:3745-3833`) and the player's `/user_horse_back`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HorseLine {
    /// `/horse_state` (`do_horse_state`).
    State,
    /// `/horse_level` (`do_horse_level`).
    Level,
    /// `/horse_set_stat` (`do_horse_set_stat`).
    SetStat,
    /// `/horse_summon` (`do_horse_summon`).
    Summon,
    /// `/horse_unsummon` (`do_horse_unsummon`).
    Unsummon,
    /// `/user_horse_back` (`do_user_horse_back`).
    Back,
    /// `/horse_ride` (`do_horse_ride`).
    Ride,
    /// `/user_horse_ride` (`do_user_horse_ride`).
    UserRide,
    /// `/unmount` (`do_unmount`).
    Unmount,
    /// `/mount_test` (`do_mount_test`).
    MountTest,
}

/// The small GM lines `game_state::gm_lines` runs, and the player's `/conqueror_point`,
/// `/sort_inventory` and `/pickup_affect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GmLine {
    /// `/item_id_list` (`do_get_item_id_list`).
    ItemIdList,
    /// `/get_mob_count` (`do_get_mob_count`).
    MobCount,
    /// `/effect` (`do_effect`).
    Effect,
    /// `/add_socket` (`do_add_socket`).
    AddSocket,
    /// `/set_socket` (`do_set_socket`).
    SetSocket,
    /// `/change_attr` (`do_change_attr`).
    ChangeAttr,
    /// `/add_attr` (`do_add_attr`).
    AddAttr,
    /// `/change_rare_attr` (`do_change_rare_attr`).
    ChangeRareAttr,
    /// `/add_rare_attr` (`do_add_rare_attr`).
    AddRareAttr,
    /// `/force_logout` (`do_force_logout`).
    ForceLogout,
    /// `/reset_subskill` (`do_reset_subskill`).
    ResetSubSkill,
    /// `/conqueror_level` (`do_conqueror_level`).
    ConquerorLevel,
    /// `/conqueror_point` (`do_conqueror_point`), a player's line.
    ConquerorPoint,
    /// `/sort_inventory` (`do_sort_inventory`), a player's line.
    SortInventory,
    /// `/pickup_affect` (`do_pickup_affect`), a player's line.
    PickupAffect,
    /// `/move_channel` (`do_move_channel`), a player's line.
    MoveChannel,
    /// `/flush` (`do_flush`), an implementor's line.
    Flush,
    /// `/observer` (`do_observer`), an implementor's line.
    Observer,
    /// `/user_horse_feed` (`do_user_horse_feed`), a player's line.
    UserHorseFeed,
    /// `/show_quiz` (`do_oxevent_show_quiz`).
    ShowQuiz,
    /// `/w_gaya` (`do_gaya_system`), a player's line.
    GayaSystem,
    /// `/cube` (`do_cube`), a player's line.
    Cube,
    /// `/fetch_casket_items` (`do_fetch_casket_items`), a player's line.
    FetchCasket,
    /// `/break_marriage` (`do_break_marriage`).
    BreakMarriage,
    /// `/close_shop` (`do_close_shop`), a player's line.
    CloseShop,
    /// `/check_mmoney` (`do_check_monarch_money`).
    CheckMmoney,
    /// `/mto` (`do_monarch_warpto`), a player's line.
    MonarchWarp,
    /// `/mtr` (`do_monarch_transfer`), a player's line.
    MonarchTransfer,
    /// `/minfo` (`do_monarch_info`), a player's line.
    MonarchInfo,
    /// `/mtax` (`do_monarch_tax`), a player's line.
    MonarchTax,
    /// `/mmob` (`do_monarch_mob`), a player's line.
    MonarchMob,
    /// `/mnotice` (`do_monarch_notice`), a player's line.
    MonarchNotice,
    /// `/reward_system` (`do_reward_system`), a player's line.
    RewardSystem,
    /// `/log_oxevent` (`do_oxevent_log`), a GM's line.
    LogOxevent,
    /// `/get_oxevent_att` (`do_oxevent_get_attender`), a GM's line.
    GetOxeventAtt,
    /// `/dungeon_info` (`do_dungeon_info`), a player's line.
    DungeonInfo,
    /// `/threeway_info` (`do_threeway_war_info`), a GM's line.
    ThreewayInfo,
    /// `/threeway_myinfo` (`do_threeway_war_myinfo`), a GM's line.
    ThreewayMyinfo,
    /// The GM arena list and duel lines (`G/cmd_gm.cpp:3957-4060`).
    ShowArenaList,
    /// Start a GM arena duel.
    ArenaDuel,
    /// End one GM arena duel.
    EndArenaDuel,
    /// End every GM arena duel.
    EndAllArenaDuel,
}

/// Which restart a dead player asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartKind {
    /// `/restart_here`: where it lies, with 50 hit points.
    Here,
    /// `/restart_town`: in its map's town, with 50 hit points.
    Town,
    /// `/restart_now`: where it lies, at full hit points, for 300 000 gold.
    Now,
}

/// Which exit a player asks for (`do_cmd`'s `SCMD_*`, `G/cmd.h:34-36`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitKind {
    /// `/logout`: the descriptor closes with a save.
    Logout,
    /// `/quit`: the client is told to quit itself.
    Quit,
    /// `/phase_select`: the character disconnects and the descriptor returns to select.
    PhaseSelect,
}

/// What a restart changed that the descriptor keeps.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RestartAnswer {
    /// The gold `/restart_now` took, as the signed change the store applies, or `None` when it
    /// took none.
    pub gold: Option<i64>,
}

/// What a skill-up changed that the descriptor keeps.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SkillUpAnswer {
    /// The climbed slot, or `None` when the up refused.
    pub climbed: Option<(u8, u8)>,
}

/// What a `/setblockmode` line changed that the descriptor keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetBlockModeAnswer {
    /// The `setblockmode %d` command line for the client's own queue, or `None`
    /// when the character was not online.
    pub line: Option<Vec<u8>>,
    /// The seven `game_option.block_*` flag writes for the store.
    pub flags: Vec<(Vec<u8>, i32)>,
}
/// What a `/hide_costume` line changed that the descriptor keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HideCostumeAnswer {
    /// The `costume_option.hide_*` flag write for the store, or `None` when the line
    /// hid nothing (empty, moving, or no such part). The `SetXCostumeHidden` line and
    /// the look update rode the outbox already.
    pub flag: Option<(Vec<u8>, i32)>,
}

/// What a `/block_chat` line did that the descriptor keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockChatAnswer {
    /// The usage, the format lines, the privilege refusal, or the caller's own
    /// name: everything the line showed already rode an outbox.
    Shown,
    /// A live character's silence moved, with its name and player id: the
    /// descriptor writes its row, as `AddAffect` writes it for a PC
    /// (`G/char_affect.cpp:756-765`), 0 included.
    Moved {
        /// The silenced character's name.
        name: Vec<u8>,
        /// Its player id.
        player: u32,
        /// The ticks the line laid; a 0 lay stands on the next tick.
        seconds: i32,
    },
    /// The name named nobody online: the descriptor stores the silence of an
    /// offline name's row (`BlockChat`, `db/ClientManager.cpp:2159-2186`).
    Offline {
        /// The name as the caller sent it.
        name: Vec<u8>,
        /// The ticks the line laid; a 0 lay stands on the next load.
        seconds: i32,
    },
}

/// What a hit changed that the descriptor keeps.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AttackAnswer {
    /// The gold the hit stole, as the signed change the store applies, or `None` when it
    /// stole none.
    pub gold: Option<i64>,
    /// The row changes the hit's spends made, in the order they happened: a bow shot's
    /// arrow (`UseArrow`), and the restack arm's destroy + move when the shot emptied the
    /// stack. The store applies these with the gold as one Transfer, because a save writes
    /// no item rows.
    pub spent: Vec<db::items::RowChange>,
}

/// Where a whisper went.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WhisperOutcome {
    /// The line reached the target, on this Channel or on another one.
    Delivered,
    /// The line went nowhere: the sender named itself, or the line was empty and
    /// nothing was left for the target.
    ///
    /// `if (pkChr == ch) return (iExtraLen);` (`G/input_main.cpp:372`) and
    /// `else return (iExtraLen);` (`G/input_main.cpp:474`).
    Dropped,
    /// The server answered the sender instead of the target hearing the line: the
    /// line was left for the target, or the sender must wait before it may leave
    /// another one.
    Answered,
}

/// Resolves, its sender dropped, once the world has run a command that answers nothing: every
/// record the command sent the client is then queued in that client's outbox, so a descriptor
/// that drains its outbox after it writes them before anything it answers later.
pub type Settled = oneshot::Receiver<()>;

/// Whom a relayed record goes to, around the character it is about.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayScope {
    /// Every player standing on the character's map of its Channel, the character included,
    /// as `FEmpireChatPacket` walks the client set by map (`G/input_main.cpp:926-953`).
    Map,
    /// `PacketAround(record)`: the character's view, then the character (`G/entity.cpp:88-105`).
    ViewAndSelf,
    /// `PacketAround(record, this)`: the character's view, without the character.
    ViewExceptSelf,
    /// Every linked member of the character's party in PID order, the character
    /// included, as `ForEachOnlineMember` walks the member map
    /// (`G/party.h:277-285`). Nothing when the character has no party. The order is
    /// legacy's own `std::map` order, so no Divergence.
    Party,
    /// Every linked member of the character's guild in PID order, the character
    /// included, as the member map is walked in PID order (`G/guild.h` `m_member`, a
    /// `std::map<DWORD, TGuildMember>`). Nothing when the character has no guild.
    Guild,
}

/// Where a character enters the world.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnterPlace {
    /// The Channel.
    pub channel: u8,
    /// `GetMapIndex()`.
    pub map: i32,
    /// `GetX()`.
    pub x: i32,
    /// `GetY()`.
    pub y: i32,
    /// `GetZ()`.
    pub z: i32,
}

/// The body an entering character shows: where it stands, and what its insert names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Showing {
    /// Where it enters.
    pub place: EnterPlace,
    /// The parts of its ADD and INFO that walking does not change.
    pub card: PcCard,
}

/// The records the show of an entering character sent its own client: its own insert, then
/// every entity its view took in, in walk order (`G/sectree.h:100-160`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Shown {
    /// The records, in order.
    pub records: Vec<Vec<u8>>,
    /// The character's own `GC_CHARACTER_UPDATE`, with the affect flags its revive-invisible
    /// affect sets. Its enter-game burst carries it; `None` when the character has no place.
    pub update: Option<Vec<u8>>,
    /// The `GC_AFFECT_ADD` of each restored saved affect (`LoadAffect`,
    /// `G/char_affect.cpp:573-666`). The burst carries them behind the update, as
    /// the load's add packets ride the queue they found.
    pub silence: Vec<Vec<u8>>,
    /// The five `SetXCostumeHidden` lines of the enter-game hide restore
    /// (`CInputLogin::Entergame`, `G/input_login.cpp:612-638`), in part order. The burst
    /// carries them after the phase change, where legacy sends them after `SetPhase`.
    pub hide_lines: Vec<Vec<u8>>,
    /// The live priv bonus notices (`_send_bonus_info`, `G/input_login.cpp:39-66`), in
    /// type order. The burst carries them after the Channel record, where legacy sends
    /// them after the greet.
    pub priv_lines: Vec<Vec<u8>>,
    /// Whether the character is in the letters event (`SEventLetters`,
    /// `G/input_login.cpp:69-82`): the `letters_event` flag is not 0. The burst's line
    /// reads 1 then, else 0.
    pub letters_event: bool,
    /// The fish-event login report (`FishEventGeneralInfo`, `G/char.cpp:10880-10913`):
    /// the enable record, then one `SHAPE_ADD` per main cell. The burst carries them
    /// last, where legacy sends them at the end of `Entergame` (`G/input_login.cpp:939`).
    pub fish_lines: Vec<Vec<u8>>,
    /// Whether the character observes: a war map's watch sets the flag at admission
    /// (`CWarMap::IncMember`, `G/war_map.cpp:384`). The descriptor gates the records by it.
    pub observer: bool,
    /// The empire a login script's `pc.change_empire` call landed, if one did (for
    /// the descriptor's account store, as a dialog step's).
    pub landed: Option<u8>,
}

/// Where a character stands, as the descriptor holds it. The world reads the point of a
/// character with a body from the body, and only the Channel and map from here.
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
    /// The gold the load read, `player.gold`, which a shop spends and pays.
    pub gold: u64,
    /// The body the character shows, `CHARACTER::Show` at the end of `PlayerLoad`
    /// (`G/input_login.cpp:590`). `None` admits the character with no body, which nobody
    /// sees and which sees nobody.
    pub show: Option<Showing>,
    /// The nonzero skill slots the load read (`player_skill`), which the world
    /// holds as the character's levels from the admission on.
    pub skills: Vec<db::player_skills::StoredSkill>,
    /// The quest flags the load read (`quest_flag`), which the world holds as the
    /// character's quest record from the admission on. Legacy reads them with the
    /// character (`CInputDB::QuestLoad`, `G/input_db.cpp:1079-1101`) and writes them with
    /// it (`quest::PC::Save`, `G/questpc.cpp:519-590`), so a flag outlives the session
    /// that set it.
    pub quest_flags: Vec<(Vec<u8>, i32)>,
    /// The friend names the store's `messenger_list` holds for this character, in the pair
    /// key's order, which `answer_enter_world` logs in before it delivers the lines that
    /// waited (`MessengerManager::Login` before `ReadOfflineMessages`,
    /// `G/input_login.cpp:671,728`).
    pub messenger_companions: Vec<Vec<u8>>,
    /// The GM names other than this character's own (`LoadTeamList`,
    /// `G/messenger_manager.cpp:32-46`).
    pub messenger_team: Vec<Vec<u8>>,
    /// Every grant's name-to-level map, which the GM gates read.
    pub messenger_levels: std::collections::BTreeMap<Vec<u8>, u8>,
    /// The saved affects the load read (`player_affect`), which the world holds
    /// as the character's timed affects from the admission on. Legacy restores
    /// them with the character (`LoadAffect`, `G/char_affect.cpp:573-666`).
    pub affects: Vec<db::player_affects::StoredAffect>,
    /// The horse name the load read (`player.horse_name`), which the world holds from the
    /// admission on (`HEADER_GD_REQ_HORSE_NAME`, `G/input_login.cpp:891-892`).
    pub horse_name: Vec<u8>,
    /// The fish-event cells the load read (`fish_event_slot`) with the use count
    /// (`fish_event_count`), which the world holds as the character's jigsaw board from
    /// the admission on. The held shape is memory-only and never loads.
    pub fish_board: (Vec<db::fish_event::StoredFishCell>, u32),
    /// The daily-gift row the load read (`daily_gift`), if the week was ever opened,
    /// with the account's cash, which the world holds as the character's gift week from
    /// the admission on. The open and loaded flags are memory-only and never load.
    pub daily_gift: (Option<db::daily_gift::StoredDailyGift>, u32),
    /// The account's character PIDs and empire-change count, which the login event's
    /// scripts judge by. The descriptor reads them before it sends the admission, as
    /// the dialog steps do before theirs.
    pub account: QuestAccount,
    /// The descriptor's host address, which the multi-farm registry keys on
    /// (`GetHostName`, `G/desc.h:55`).
    pub addr: String,
}

/// What the world holds of a character that a save writes: asked for before a save, and
/// handed back when the character leaves.
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    /// The character's points as the world last changed them. `None` for a character that
    /// entered without them.
    pub points: Option<Points>,
    /// Its quickslots, which a trade the partner closes changes.
    pub quickslots: Quickslots,
    /// Where its body stands, as (map, x, y), which a save writes. `None` for a character
    /// that entered with no body.
    pub place: Option<(i32, i32, i32)>,
    /// The alignment the world holds, which a save writes. Legacy's `SaveReal` writes the
    /// real alignment (`G/char.cpp:1546`), and the kills raise it.
    pub alignment: i32,
    /// The quest flags the world holds, which a save writes: legacy's `quest::PC` writes
    /// them with the character (`G/char.cpp:1670-1678`).
    pub quest_flags: Vec<(Vec<u8>, i32)>,
    /// The saved affects the world holds, which a save writes: the silence rows
    /// (`AFFECT_BLOCK_CHAT`) the character still carries.
    pub affects: Vec<db::player_affects::StoredAffect>,
    /// The fish-event board the world holds, as the set cells with the use count, which
    /// a save writes (`G/char.cpp:1613-1614`). The held shape is memory-only and never
    /// keeps.
    pub fish_board: (Vec<db::fish_event::StoredFishCell>, u32),
    /// The daily-gift row the world holds, if the week was ever opened, with the
    /// account's cash, which a save writes (`G/char_daily_gifts.cpp:258`, `:368-370`).
    /// The open and loaded flags are memory-only and never keep.
    pub daily_gift: (Option<db::daily_gift::StoredDailyGift>, u32),
    /// The horse name the world holds (`CHorseNameManager`), which a save writes with the
    /// character row. `None` for a character that entered without points.
    pub horse_name: Option<Vec<u8>>,
    /// The character's global rank row and the generation to save it under, which a save
    /// writes (`db::rank_global::save_row`). `None` for a character that never scored.
    pub rank: Option<(i64, db::rank_global::StoredRank)>,
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

    /// Gives the world its stored guilds, and waits for the answer.
    ///
    /// # Errors
    ///
    /// [`InstallError::NotSent`] when the game thread has closed its receiver and
    /// [`InstallError::NoAnswer`] when it closed the reply without sending. The
    /// caller must not open the ready gate on either: a world without its guilds
    /// judges a concurrent name free, so a silently missing install is a server that
    /// looks up and lets two guilds share a name.
    pub async fn install_guilds(
        &self,
        stored: db::guilds::StoredGuilds,
        marks: Vec<db::guilds::StoredMark>,
        symbols: Vec<db::guilds::StoredSymbol>,
        reservations: Vec<db::guilds::StoredReservation>,
        bets: Vec<db::guilds::StoredBet>,
        land: db::guild_land::StoredLandInstall,
    ) -> Result<Result<(), InstallError>, InstallError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::InstallGuilds {
            stored,
            marks,
            symbols,
            reservations,
            bets,
            land,
            reply,
        })
        .await
        .map_err(|_| InstallError::NotSent)?;
        if answer.await.map_err(|_| InstallError::NoAnswer)? {
            Ok(Ok(()))
        } else {
            Ok(Err(InstallError::AlreadyInstalled))
        }
    }

    /// Installs the world's event calendar, and waits for the answer.
    ///
    /// # Errors
    ///
    /// [`InstallError::NotSent`] when the game thread has closed its receiver and
    /// [`InstallError::NoAnswer`] when it closed the reply without sending. The
    /// caller must not open the ready gate on either: a world without its calendar
    /// answers every event board empty, so a silently missing install is a server
    /// that looks up and denies every scheduled event.
    pub async fn install_events(
        &self,
        events: Vec<db::events::StoredEvent>,
    ) -> Result<Result<(), InstallError>, InstallError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::InstallEvents { events, reply })
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
    ) -> Result<Result<Shown, EnterWorldRefused>, EnterWorldError> {
        self.enter_world_with_items(vid, player_id, name, Vec::new(), Loaded::default(), outbox)
            .await
    }

    /// Puts a live client's character into the world holding the items its load placed.
    ///
    /// `items` is [`crate::item_load::ItemLoad::placed`] in its own order. The world
    /// places them before it answers, so the admission and the inventory are one step.
    /// [`Loaded::show`] places the character's body, and the answer carries what its own
    /// client was sent.
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
    ) -> Result<Result<Shown, EnterWorldRefused>, EnterWorldError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::EnterWorld {
            vid,
            player_id,
            name,
            items,
            loaded: Box::new(loaded),
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
        departure: Departure,
    ) -> Result<Option<Kept>, LeaveWorldError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::LeaveWorld {
            vid,
            departure,
            reply,
        })
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

    /// Asks the world for the map a character is placed on, as `GetValidLocation` resolves
    /// it (`G/sectree_manager.cpp:816-842`).
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn map_of(&self, channel: u8, map_index: i32) -> Result<i32, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MapOf {
            channel,
            map_index,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world for the event-flag lines a loading character hears
    /// (`BroadcastEventFlagOnLogin`, `G/questmanager.cpp:1701-1744`).
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn login_event_lines(&self) -> Result<Vec<Vec<u8>>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::LoginEventLines { reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world for the points and quickslots of the character online under `vid`,
    /// which a save writes from, after it banks `playtime` minutes played when there are any.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn kept_of(
        &self,
        vid: common::vid::Vid,
        playtime: Option<i32>,
    ) -> Result<Option<Kept>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::KeptOf {
            vid,
            playtime,
            reply,
        })
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
        account: QuestAccount,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::UseItem {
            vid,
            at,
            mover,
            account,
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

    /// Asks the world to destroy the item, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn destroy_item(
        &self,
        vid: common::vid::Vid,
        at: protocol::item_pos::ItemPos,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DestroyItem {
            vid,
            at,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to use one item on another, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn use_item_to_item(
        &self,
        vid: common::vid::Vid,
        (at, dest): (protocol::item_pos::ItemPos, protocol::item_pos::ItemPos),
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::UseItemToItem {
            vid,
            at,
            dest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run a refine request, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn refine(
        &self,
        vid: common::vid::Vid,
        (pos, refine_type): (u8, u8),
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Refine {
            vid,
            pos,
            refine_type,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run an element-refine request, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn refine_element(
        &self,
        vid: common::vid::Vid,
        arg: u8,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::RefineElement {
            vid,
            arg,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run an attr67 request, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn attr67(
        &self,
        vid: common::vid::Vid,
        sub_header: u8,
        regist: crate::game_state::Attr67Regist,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Attr67 {
            vid,
            sub_header,
            regist,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run a sash request, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn sash(
        &self,
        vid: common::vid::Vid,
        sub_header: u8,
        (pos, at): (u8, protocol::item_pos::ItemPos),
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Sash {
            vid,
            sub_header,
            pos,
            at,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run a changelook request, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn changelook(
        &self,
        vid: common::vid::Vid,
        sub_header: u8,
        (pos, at): (u8, protocol::item_pos::ItemPos),
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Changelook {
            vid,
            sub_header,
            pos,
            at,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run a cube request, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn cube_renewal(
        &self,
        vid: common::vid::Vid,
        sub_header: u8,
        words: (u32, u32, u32),
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CubeRenewal {
            vid,
            sub_header,
            words,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run a Gaya request, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn gaya_system(
        &self,
        vid: common::vid::Vid,
        sub_header: u8,
        pos: i32,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GayaSystem {
            vid,
            sub_header,
            pos,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run a biologist request, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn biologist(
        &self,
        vid: common::vid::Vid,
        step: crate::game_state::BiologistStep,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Biologist {
            vid,
            step,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to give the item, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    /// Asks the world to run one premium-players step, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn premium(
        &self,
        vid: common::vid::Vid,
        step: crate::game_state::PremiumStep,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Premium {
            vid,
            step,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one inventory unlock, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn extend_inven(
        &self,
        vid: common::vid::Vid,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ExtendInven { vid, mover, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one protected-inventory step, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn protected(
        &self,
        vid: common::vid::Vid,
        step: crate::game_state::ProtectedStep,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Protected {
            vid,
            step,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to give the item, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn give_item(
        &self,
        vid: common::vid::Vid,
        target: u32,
        at: protocol::item_pos::ItemPos,
        place: GroundPlace,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GiveItem {
            vid,
            target,
            at,
            place,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to drop gold, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn drop_gold(
        &self,
        vid: common::vid::Vid,
        gold: u32,
        place: GroundPlace,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DropGold {
            vid,
            gold,
            place,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `CG_FISHING`, and waits for the rows it changed.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn fishing(
        &self,
        vid: common::vid::Vid,
        dir: u8,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Fishing {
            vid,
            dir,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `CG_FISH_EVENT_SEND`, and waits for the rows it changed.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn fish_event(
        &self,
        vid: common::vid::Vid,
        step: crate::game_state::FishEventStep,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::FishEvent {
            vid,
            step,
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

    /// Asks the world to run one shop step, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn shop(
        &self,
        vid: common::vid::Vid,
        step: ShopStep,
        place: GroundPlace,
        mover: Mover,
        account: QuestAccount,
    ) -> Result<Result<ShopAnswer, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Shop {
            vid,
            step,
            place,
            mover,
            account,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `CG_ANSWER_MAKE_GUILD`, and waits for the judgement.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_make_guild(
        &self,
        vid: common::vid::Vid,
        field: [u8; protocol::cg_guild_answer::GUILD_NAME_FIELD_SIZE],
        now: u32,
    ) -> Result<crate::game_state::GuildMakeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerMakeGuild {
            vid,
            field,
            now,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `/makeguild` word, and waits for the judgement.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn judge_makeguild(
        &self,
        vid: common::vid::Vid,
        word: Vec<u8>,
    ) -> Result<crate::game_state::GuildMakeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::JudgeMakeguild { vid, word, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `/deleteguild`, and waits for the judgement.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn judge_deleteguild(
        &self,
        vid: common::vid::Vid,
        now: u32,
    ) -> Result<crate::game_state::GuildQuestJudged, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::JudgeDeleteguild { vid, now, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to commit one judged `/makeguild`, and waits for the commit.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn commit_makeguild(
        &self,
        vid: common::vid::Vid,
        commit: crate::game_state::GuildMakeCommit,
        mover: Mover,
    ) -> Result<crate::game_state::GuildMakeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CommitMakeguild {
            vid,
            commit,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to commit one judged creation, and waits for the commit.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn commit_make_guild(
        &self,
        vid: common::vid::Vid,
        commit: crate::game_state::GuildMakeCommit,
        mover: Mover,
    ) -> Result<crate::game_state::GuildMakeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CommitMakeGuild {
            vid,
            commit,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `GUILD_SUBHEADER_CG_ADD_MEMBER`, without waiting: the
    /// invite lands in memory and the guest hears the record on its own client.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn guild_add_member(
        &self,
        vid: common::vid::Vid,
        guest: u32,
        now: u32,
    ) -> Result<(), MoveItemError> {
        self.send_command(GameCommand::GuildAddMember { vid, guest, now })
            .await
            .map_err(|_| MoveItemError::NotSent)
    }

    /// Asks the world to judge one `GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER`, and waits
    /// for the judgement.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_invite(
        &self,
        vid: common::vid::Vid,
        guild_id: u32,
        accept: u8,
        now: u32,
    ) -> Result<crate::game_state::GuildJoinAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildInvite {
            vid,
            guild_id,
            accept,
            now,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to commit one judged join, and waits for the commit.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn commit_guild_join(
        &self,
        commit: crate::game_state::GuildJoinCommit,
    ) -> Result<crate::game_state::GuildJoinAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CommitGuildJoin { commit, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `GUILD_SUBHEADER_CG_REMOVE_MEMBER`, and waits for
    /// the judgement.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_remove(
        &self,
        vid: common::vid::Vid,
        pid: u32,
        now: u32,
    ) -> Result<crate::game_state::GuildRemoveAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildRemove {
            vid,
            pid,
            now,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to commit one judged removal, and waits for the commit.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn commit_guild_remove(
        &self,
        commit: crate::game_state::GuildRemoveCommit,
    ) -> Result<crate::game_state::GuildRemoveAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CommitGuildRemove { commit, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME`, and waits
    /// for the judgement.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_grade_name(
        &self,
        vid: common::vid::Vid,
        grade: u8,
        field: Vec<u8>,
    ) -> Result<crate::game_state::GuildGradeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildGradeName {
            vid,
            grade,
            field,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY`, and
    /// waits for the judgement.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_grade_auth(
        &self,
        vid: common::vid::Vid,
        grade: u8,
        auth: u8,
    ) -> Result<crate::game_state::GuildGradeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildGradeAuth {
            vid,
            grade,
            auth,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE`, and waits
    /// for the judgement.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_member_grade(
        &self,
        vid: common::vid::Vid,
        pid: u32,
        grade: u8,
    ) -> Result<crate::game_state::GuildGradeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildMemberGrade {
            vid,
            pid,
            grade,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL`, and
    /// waits for the judgement.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_member_general(
        &self,
        vid: common::vid::Vid,
        pid: u32,
        is_general: u8,
    ) -> Result<crate::game_state::GuildGradeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildMemberGeneral {
            vid,
            pid,
            is_general,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `GUILD_SUBHEADER_CG_OFFER`, and waits for what the
    /// step stores. See `game_state::guild`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_offer(
        &self,
        vid: common::vid::Vid,
        offer: u32,
    ) -> Result<crate::game_state::GuildGradeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildOffer { vid, offer, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `GUILD_SUBHEADER_CG_CHARGE_GSP`, and waits for what
    /// the step stores. See `game_state::guild`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_charge(
        &self,
        vid: common::vid::Vid,
        offer: i32,
    ) -> Result<crate::game_state::GuildGradeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildCharge { vid, offer, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/greset`, and waits for what the step
    /// stores. See `game_state::guild`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_guild_reset(
        &self,
        vid: common::vid::Vid,
    ) -> Result<crate::game_state::GuildGradeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::RunGuildReset { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `GUILD_SUBHEADER_CG_POST_COMMENT`, and waits for
    /// what the step stores. See `game_state::guild`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_post_comment(
        &self,
        vid: common::vid::Vid,
        content: Vec<u8>,
    ) -> Result<crate::game_state::GuildCommentAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildPostComment {
            vid,
            content,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to commit one judged post, and waits for the board. See
    /// `game_state::guild`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn commit_guild_post_comment(
        &self,
        commit: crate::game_state::GuildCommentCommit,
    ) -> Result<crate::game_state::GuildCommentAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CommitGuildPostComment { commit, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `GUILD_SUBHEADER_CG_REFRESH_COMMENT`, and waits for
    /// the gates. See `game_state::guild`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_refresh_comment(
        &self,
        vid: common::vid::Vid,
    ) -> Result<crate::game_state::GuildCommentAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildRefreshComment { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `GUILD_SUBHEADER_CG_DELETE_COMMENT`, and waits for
    /// the gates. See `game_state::guild`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_delete_comment(
        &self,
        vid: common::vid::Vid,
        comment_id: u32,
    ) -> Result<crate::game_state::GuildCommentAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildDeleteComment {
            vid,
            comment_id,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to report one judged delete's store half, and waits for the
    /// board or the line. See `game_state::guild`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn commit_guild_delete_comment(
        &self,
        commit: crate::game_state::GuildCommentCommit,
    ) -> Result<crate::game_state::GuildCommentAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CommitGuildDeleteComment { commit, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to commit one judged grade-arm step, and waits for the commit.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn commit_guild_grade(
        &self,
        commit: crate::game_state::GuildGradeCommit,
    ) -> Result<crate::game_state::GuildGradeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CommitGuildGrade { commit, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `GUILD_SUBHEADER_CG_USE_SKILL`, and waits for
    /// what the step stores. See `game_state::guild`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn answer_guild_use_skill(
        &self,
        vid: common::vid::Vid,
        vnum: u32,
        pid: u32,
    ) -> Result<crate::game_state::GuildSkillUseAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AnswerGuildUseSkill {
            vid,
            vnum,
            pid,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to commit one judged cast, and waits for the commit.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn commit_guild_use_skill(
        &self,
        commit: crate::game_state::GuildSkillUseCommit,
    ) -> Result<crate::game_state::GuildSkillUseAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CommitGuildUseSkill { commit, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/gskillup` line, and waits for the climbed skill.
    /// What the up shows is written to the player's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn guild_skill_up(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<crate::game_state::GuildSkillUpAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GuildSkillUp { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/war` line, and waits for the declare notice.
    /// What the line shows is written to the players' outboxes before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn guild_war(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        now: u32,
    ) -> Result<crate::game_state::GuildWarAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GuildWar {
            vid,
            rest,
            now,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/nowar` line, and waits for the refusal to run.
    /// What the line shows is written to the players' outboxes before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn guild_nowar(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GuildNoWar { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `gwstop` line, and waits for the ended rows. What the
    /// line shows is written to the players' outboxes before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn guild_war_stop(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        now: u32,
    ) -> Result<Option<crate::game_state::WarEndOutcome>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GuildWarStop {
            vid,
            rest,
            now,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `gwcancel` line, and waits for whether the pair was a
    /// live war. What the line shows is written to the players' outboxes before the
    /// answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn guild_war_cancel(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        now: u32,
    ) -> Result<bool, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GuildWarCancel {
            vid,
            rest,
            now,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world for its live wars, in pair order, with each pair's age at `now`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn guild_war_list(
        &self,
        now: u32,
    ) -> Result<Vec<crate::game_state::GuildWarListLine>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GuildWarList { now, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world for the `/gstate` answer for `rest`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn guild_state(
        &self,
        rest: Vec<u8>,
    ) -> Result<Option<crate::game_state::GuildStateLine>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GuildState { rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world for the war ends its guild tick queued, draining them.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn take_war_ends(
        &self,
    ) -> Result<Vec<crate::game_state::WarEndOutcome>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::TakeWarEnds { reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one trade step, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn trade(
        &self,
        vid: common::vid::Vid,
        step: TradeStep,
        place: GroundPlace,
        mover: Mover,
    ) -> Result<Result<TradeAnswer, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Trade {
            vid,
            step,
            place,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one safebox or mall step, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn safebox(
        &self,
        vid: common::vid::Vid,
        step: SafeboxStep,
        mover: Mover,
    ) -> Result<Result<SafeboxAnswer, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Safebox {
            vid,
            step,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one quest step, and waits for the records its script sent.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn quest(
        &self,
        vid: common::vid::Vid,
        step: QuestStep,
        place: GroundPlace,
        mover: Mover,
        account: QuestAccount,
    ) -> Result<Result<super::game_state::QuestStepOutcome, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Quest {
            vid,
            step,
            place,
            mover,
            account,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Hands the world one `CG_MOVE`, without waiting: the move has no answer. The
    /// [`Settled`] resolves once the world has run it.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn move_character(
        &self,
        vid: common::vid::Vid,
        record: CgMove,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::Move {
            vid,
            record,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Asks the world to run one `CG_SYNC_POSITION`, and waits for what it did.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn sync_positions(
        &self,
        vid: common::vid::Vid,
        packet: SyncPositionPacket,
    ) -> Result<Option<SyncPositionResult>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SyncPosition { vid, packet, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Hands the world records to send around the character online under `vid`, without
    /// waiting. The [`Settled`] resolves once they are sent.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn relay(
        &self,
        vid: common::vid::Vid,
        records: Vec<(RelayScope, Vec<u8>)>,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::Relay {
            vid,
            records,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a whisper from the character online under `vid`, and waits for where
    /// the line went.
    ///
    /// The record is the one the caller built: the sender's name and the type are the
    /// sender's own, and the world has no say in them.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver and
    /// [`MoveItemError::NoAnswer`] when it closed the reply without sending.
    pub async fn whisper(
        &self,
        vid: common::vid::Vid,
        name: Vec<u8>,
        text: Vec<u8>,
        record: Vec<u8>,
    ) -> Result<WhisperOutcome, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Whisper {
            vid,
            name,
            text,
            record,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Hands the world a `CG_PARTY_INVITE` from the character online under `vid`, without
    /// waiting. The [`Settled`] resolves once the invite has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn party_invite(
        &self,
        vid: common::vid::Vid,
        guest: u32,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::PartyInvite {
            vid,
            guest,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `CG_PARTY_INVITE_ANSWER` from the character online under `vid`,
    /// without waiting. The [`Settled`] resolves once the answer has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn party_invite_answer(
        &self,
        vid: common::vid::Vid,
        inviter: u32,
        accept: u8,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::PartyInviteAnswer {
            vid,
            inviter,
            accept,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `/party_request` line from the character online under `vid`,
    /// without waiting. The [`Settled`] resolves once the line has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn party_request(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::PartyRequest { vid, rest, settled })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `/party_request_accept` line from the character online under
    /// `vid`, without waiting. The [`Settled`] resolves once the line has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn party_request_accept(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::PartyRequestAccept { vid, rest, settled })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `/party_request_deny` line from the character online under
    /// `vid`, without waiting. The [`Settled`] resolves once the line has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn party_request_deny(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::PartyRequestDeny { vid, rest, settled })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `/ungroup` line from the character online under `vid`, without
    /// waiting. The [`Settled`] resolves once the leave has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn ungroup(&self, vid: common::vid::Vid, pid: u32) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::Ungroup { vid, pid, settled })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `CG_PARTY_REMOVE` from the character online under `vid`, without
    /// waiting. The [`Settled`] resolves once the removal has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn party_remove(
        &self,
        vid: common::vid::Vid,
        pid: u32,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::PartyRemove { vid, pid, settled })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `CG_PARTY_SET_STATE` from the character online under `vid`, without
    /// waiting. The [`Settled`] resolves once the set has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn party_set_state(
        &self,
        vid: common::vid::Vid,
        pid: u32,
        role: u8,
        flag: u8,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::PartySetState {
            vid,
            pid,
            role,
            flag,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `CG_PARTY_PARAMETER` from the character online under `vid`, without
    /// waiting. The [`Settled`] resolves once the set has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn party_parameter(
        &self,
        vid: common::vid::Vid,
        mode: u8,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::PartyParameter { vid, mode, settled })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `CG_PARTY_USE_SKILL` from the character online under `vid`, without
    /// waiting. The [`Settled`] resolves once the use has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn party_use_skill(
        &self,
        vid: common::vid::Vid,
        skill: u8,
        target: u32,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::PartyUseSkill {
            vid,
            skill,
            target,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }
    /// Hands the world a `CG_MESSENGER` `ADD_BY_VID` from the character online under `vid`,
    /// without waiting. The visible gates ran in the CG step's own walk; the world runs
    /// `RequestToAdd`. The [`Settled`] resolves once the add has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn messenger_add_by_vid(
        &self,
        vid: common::vid::Vid,
        target_vid: u32,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::MessengerAddByVid {
            vid,
            target_vid,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `CG_MESSENGER` `ADD_BY_NAME` from the character online under
    /// `vid`, without waiting. The caller's GM-gate read (`gm_get_level`) ran in the CG
    /// step's own walk and rides `companion_level`. The [`Settled`] resolves once the
    /// add has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn messenger_add_by_name(
        &self,
        vid: common::vid::Vid,
        name: Vec<u8>,
        companion_level: Option<u8>,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::MessengerAddByName {
            vid,
            name,
            companion_level,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `CG_MESSENGER` `REMOVE` from the character online under `vid`,
    /// without waiting. The caller writes the pair's rows once the remove has run. The
    /// [`Settled`] resolves once the remove has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn messenger_remove(
        &self,
        vid: common::vid::Vid,
        companion: Vec<u8>,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::MessengerRemove {
            vid,
            companion,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `/messenger_auth` answer from the character online under `vid`,
    /// and waits for whether the pair became one, which is when the caller writes the
    /// pair's rows.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver, and
    /// [`MoveItemError::NoAnswer`] when it never answered.
    pub async fn messenger_auth(
        &self,
        vid: common::vid::Vid,
        requester: Vec<u8>,
        deny: bool,
    ) -> Result<bool, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MessengerAuth {
            vid,
            requester,
            deny,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world whether the character online under `vid` holds a party, and
    /// waits for the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::sync_positions`].
    pub async fn party_of(&self, vid: common::vid::Vid) -> Result<bool, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::PartyOf { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `CG_QUEST_CONFIRM` for the character online under
    /// `vid`, and waits until it ran.
    ///
    /// # Errors
    ///
    /// As [`Self::sync_positions`].
    pub async fn quest_confirm(
        &self,
        vid: common::vid::Vid,
        answer: u8,
        request_pid: u32,
    ) -> Result<(), MoveItemError> {
        let (reply, answered) = oneshot::channel();
        self.send_command(GameCommand::QuestConfirm {
            vid,
            answer,
            request_pid,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answered.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world whether the character online under `vid` holds a guild, and
    /// waits for the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::sync_positions`].
    pub async fn guild_of(&self, vid: common::vid::Vid) -> Result<bool, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GuildOf { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to judge one `MARK_UPLOAD`, and waits for the judgement.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn mark_upload(
        &self,
        guild_id: u32,
        image: [u32; MARK_PIXELS],
        min_level: u8,
    ) -> Result<crate::game_state::MarkUploadAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MarkUpload {
            guild_id,
            image: Box::new(image),
            min_level,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world for every (guild, mark) pair, and waits for the list.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn mark_idx_list(&self) -> Result<Vec<(u16, u16)>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MarkIdxList { reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to diff one image slot, and waits for the blocks.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn mark_crc_list(
        &self,
        img_idx: u8,
        crc_list: [u32; MARK_CRC_WORDS],
    ) -> Result<Vec<(u8, Vec<u8>)>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MarkCrcList {
            img_idx,
            crc_list: Box::new(crc_list),
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to commit one judged symbol upload, and waits for the verdict.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn symbol_upload(&self, guild_id: u32, data: Vec<u8>) -> Result<bool, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SymbolUpload {
            guild_id,
            data,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to answer one `SYMBOL_CRC`, and waits for the bytes.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn symbol_crc(
        &self,
        guild_id: u32,
        crc: u32,
        size: u32,
    ) -> Result<Option<Vec<u8>>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SymbolCrc {
            guild_id,
            crc,
            size,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Hands the world a `CG_TARGET` from the character online under `vid`, without waiting.
    /// The [`Settled`] resolves once its answer is sent.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn target(
        &self,
        vid: common::vid::Vid,
        target: u32,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::Target {
            vid,
            target,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `CG_TARGET_INFO_LOAD` from the character online under `vid`, without
    /// waiting. The [`Settled`] resolves once its answer is sent.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn target_info(
        &self,
        vid: common::vid::Vid,
        target: u32,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::TargetInfo {
            vid,
            target,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `CG_REQUEST_EVENT_DATA` from the character online under `vid`,
    /// without waiting. The [`Settled`] resolves once its answer is sent.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn event_info(
        &self,
        vid: common::vid::Vid,
        month: u8,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::EventInfo {
            vid,
            month,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Hands the world a `CG_USE_SKILL` from the character online under `vid`, without
    /// waiting. The [`Settled`] resolves once the cast has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn use_skill(
        &self,
        vid: common::vid::Vid,
        record: protocol::cg_use_skill::CgUseSkill,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::UseSkill {
            vid,
            record,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Asks the world to run one `CG_ATTACK` from the character online under `vid`, and waits
    /// for the gold it stole. What the hit shows is written to the player's outbox before the
    /// answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn attack(
        &self,
        vid: common::vid::Vid,
        record: protocol::cg_attack::CgAttack,
    ) -> Result<AttackAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Attack { vid, record, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Hands the world a `CG_(ADD_)FLY_TARGETING` from the character online under `vid`,
    /// without waiting. The [`Settled`] resolves once the store has run.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the game thread has closed its receiver.
    pub async fn fly_target(
        &self,
        vid: common::vid::Vid,
        record: protocol::cg_fly_target::CgFlyTarget,
    ) -> Result<Settled, MoveItemError> {
        let (settled, settling) = oneshot::channel();
        self.send_command(GameCommand::FlyTarget {
            vid,
            record,
            settled,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        Ok(settling)
    }

    /// Asks the world to run one `CG_SHOOT` from the character online under `vid`, and waits
    /// for the gold it stole. What the shot shows is written to the player's outbox before the
    /// answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn shoot(
        &self,
        vid: common::vid::Vid,
        record: protocol::cg_shoot::CgShoot,
    ) -> Result<AttackAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Shoot { vid, record, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to switch the language of the player online under `vid` and show
    /// its new look, and waits for whether the player is online.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn change_language(
        &self,
        vid: common::vid::Vid,
        language: u8,
    ) -> Result<bool, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ChangeLanguage {
            vid,
            language,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one restart command of a player, and waits for the gold it took.
    /// What it shows is written to the player's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn restart(
        &self,
        vid: common::vid::Vid,
        kind: RestartKind,
        mover: Mover,
    ) -> Result<RestartAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Restart {
            vid,
            kind,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/skillup` line of a player, and waits for the
    /// climbed slot. What the up shows is written to the player's outbox before
    /// the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn skill_up(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<SkillUpAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SkillUp { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/setblockmode` line of a player, and waits for
    /// the command line and the flag writes. The line is written to the player's
    /// outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn set_block_mode(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<SetBlockModeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SetBlockMode { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/hide_costume` line of a player, and waits for the
    /// flag write. The `SetXCostumeHidden` line and the look update ride the outbox
    /// before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn hide_costume(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<HideCostumeAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::HideCostume { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/emotion_allow` or `/do_emotion` line of a player.
    /// What the line shows rides the outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn emotion(
        &self,
        vid: common::vid::Vid,
        command: String,
        rest: Vec<u8>,
        mover: crate::item_move::Mover,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Emotion {
            vid,
            command,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/priv_empire` line of a GM. What the line shows rides
    /// the outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn priv_empire(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        mover: crate::item_move::Mover,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::PrivEmpire {
            vid,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/eventflag` line of a GM. What the line shows rides
    /// the outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn event_flag(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        mover: crate::item_move::Mover,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::EventFlag {
            vid,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `CG_DAILY_GIFT`, and waits for the rows it changed.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn daily_gift(
        &self,
        vid: common::vid::Vid,
        step: crate::game_state::DailyGiftStep,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DailyGift {
            vid,
            step,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/daily_gift_event` line of a GM. What the line shows
    /// rides the outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn daily_gift_event(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        mover: crate::item_move::Mover,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DailyGiftEvent {
            vid,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/geteventflag` line of a GM. What the line shows
    /// rides the outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn get_event_flag(
        &self,
        vid: common::vid::Vid,
        mover: crate::item_move::Mover,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GetEventFlag { vid, mover, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/horse_state`, `/horse_level` or `/horse_set_stat` line
    /// of a GM. What the line shows rides the outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn horse_line(
        &self,
        vid: common::vid::Vid,
        line: HorseLine,
        rest: Vec<u8>,
    ) -> Result<Option<crate::game_state::SetSkillOtherLanded>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::HorseLine {
            vid,
            line,
            rest,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/xmas_*`, `/eclipse` or `/eventhelper` line of a GM.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn event_line(
        &self,
        vid: common::vid::Vid,
        line: EventLine,
        rest: Vec<u8>,
        mover: crate::item_move::Mover,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::EventLine {
            vid,
            line,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one rank line. What the line shows rides the outbox before the
    /// answer, which is the season's end to store, or `None`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn rank_line(
        &self,
        vid: common::vid::Vid,
        line: RankLine,
        mover: crate::item_move::Mover,
    ) -> Result<Option<crate::game_state::SeasonEnd>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::RankLine {
            vid,
            line,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Installs the global rank boot read.
    ///
    /// # Errors
    ///
    /// [`InstallError::NotSent`] when the command never reached the game thread and
    /// [`InstallError::NoAnswer`] when it closed the reply without sending.
    pub async fn install_rank(&self, boot: db::rank_global::RankBoot) -> Result<(), InstallError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::InstallRank { boot, reply })
            .await
            .map_err(|_| InstallError::NotSent)?;
        answer.await.map_err(|_| InstallError::NoAnswer)
    }
    /// Installs the monarch treasuries.
    ///
    /// # Errors
    ///
    /// [`InstallError::NotSent`] when the command never reached the game thread and
    /// [`InstallError::NoAnswer`] when it closed the reply without sending.
    pub async fn install_monarch(
        &self,
        money: crate::game_state::InstalledMonarchMoney,
        identity: crate::game_state::InstalledMonarchIdentity,
    ) -> Result<(), InstallError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::InstallMonarch {
            money,
            identity,
            reply,
        })
        .await
        .map_err(|_| InstallError::NotSent)?;
        answer.await.map_err(|_| InstallError::NoAnswer)
    }

    /// Asks the world to run one `/give_reward` line. What the line shows rides the
    /// outbox before the answer, which is the stored rows or `None` for a refusal
    /// the caller already heard.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn give_reward(
        &self,
        vid: common::vid::Vid,
        mover: crate::item_move::Mover,
    ) -> Result<Option<crate::game_state::LettersTrade>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GiveReward { vid, mover, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/socketitem` give. The records ride back with the
    /// stored rows, which the caller stores before the client hears of them; `None` is a
    /// give that made nothing, whose line rode the outbox.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn socket_item(
        &self,
        vid: common::vid::Vid,
        (vnum, sockets): (u32, usize),
        mover: crate::item_move::Mover,
    ) -> Result<Option<crate::game_state::LettersTrade>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SocketItem {
            vid,
            vnum,
            sockets,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one small GM line. What it tells rides the outbox before the
    /// answer, which is the rows a socket write changed.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn gm_line(
        &self,
        vid: common::vid::Vid,
        line: GmLine,
        rest: Vec<u8>,
        mover: crate::item_move::Mover,
    ) -> Result<Option<crate::game_state::GmRows>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GmLine {
            vid,
            line,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world for one dungeon rank's flag. The answer is the flag the caller
    /// reads and fans, or `None` on silence.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn dungeon_rank(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        mover: crate::item_move::Mover,
    ) -> Result<Option<crate::game_state::DungeonRankAsk>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DungeonRank {
            vid,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/letters_open` line. What the line shows rides the
    /// outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn letters_open(
        &self,
        vid: common::vid::Vid,
        mover: crate::item_move::Mover,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::LettersOpen { vid, mover, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/stat` or `/stat-` line. What the line shows
    /// rides the outbox before the answer, which is the new held numbers or
    /// `None` for a line that runs nothing.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_stat(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        lower: bool,
    ) -> Result<Option<crate::game_state::StatHeld>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Stat {
            vid,
            rest,
            lower,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/stat_val` line. What the line shows
    /// rides the outbox before the answer, which is the new held numbers or
    /// `None` for a line that runs nothing.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_stat_val(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Option<crate::game_state::StatHeld>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::StatVal { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `*+` line. What the line shows rides the
    /// outbox before the answer, which is the new held numbers or `None` for
    /// a line that runs nothing.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_stat_plus(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        kind: usize,
    ) -> Result<Option<crate::game_state::StatHeld>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::StatPlus {
            vid,
            rest,
            kind,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `t*` line. What the line shows rides the
    /// outboxes before the answer, which is the target's name and new held
    /// numbers, or `None` for a line that runs nothing.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_set_stat(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        kind: usize,
    ) -> Result<Option<crate::game_state::SetStatHeld>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SetStat {
            vid,
            rest,
            kind,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/stat_reset` line. What the line shows rides
    /// the outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_stat_reset(
        &self,
        vid: common::vid::Vid,
    ) -> Result<Option<crate::game_state::StatHeld>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::StatReset { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/pkmode` line. What the line shows rides the
    /// outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_pkmode(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::PkMode { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/set_walk_mode` or `/set_run_mode` line. What
    /// the line shows rides the outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_walk_mode(
        &self,
        vid: common::vid::Vid,
        walk: bool,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::WalkMode { vid, walk, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/can_dead` or `/cannot_dead` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_can_dead(
        &self,
        vid: common::vid::Vid,
        can: bool,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CanDead { vid, can, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/all_skill_master` line, and hears what
    /// the line planted for the store.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_all_skill_master(
        &self,
        vid: common::vid::Vid,
    ) -> Result<Vec<crate::game_state::SetSkillLanded>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AllSkillMaster { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/item_full_set` line, and hears the moved
    /// items for the store and the client.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_item_full_set(
        &self,
        vid: common::vid::Vid,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ItemFullSet { vid, mover, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/attr_full_set` line, and hears the moved
    /// items for the store and the client.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_attr_full_set(
        &self,
        vid: common::vid::Vid,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AttrFullSet { vid, mover, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/full_set` line, and hears the planted skills
    /// with the moved items.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_full_set(
        &self,
        vid: common::vid::Vid,
        mover: Mover,
    ) -> Result<
        Result<(Vec<crate::game_state::SetSkillLanded>, MovedItems), MoveItemRefused>,
        MoveItemError,
    > {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::FullSet { vid, mover, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/slow` line, and hears the moved items
    /// for the store and the client.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_slow(
        &self,
        vid: common::vid::Vid,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Slow { vid, mover, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/ipurge` line, and hears the moved items
    /// for the store and the client.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_item_purge(
        &self,
        vid: common::vid::Vid,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ItemPurge { vid, mover, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/book` line, and hears the moved items
    /// for the store and the client.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_book(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Book {
            vid,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/console` line.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the world is gone, [`MoveItemError::NoAnswer`]
    /// when it gives none.
    pub async fn run_console(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Console { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/set` line, and hears the gold Transfer for the
    /// store, if the arm stored one.
    ///
    /// # Errors
    ///
    /// [`MoveItemError::NotSent`] when the world is gone, [`MoveItemError::NoAnswer`]
    /// when it gives none.
    pub async fn run_set_attr(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Option<crate::game_state::SetAttrStored>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SetAttr { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/refine_rod`, `/refine_pick` or `/max_pick` line,
    /// and hears the moved items for the store and the client.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_tool_refine_line(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        mover: Mover,
        line: world::character::ToolRefine,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ToolRefineLine {
            vid,
            rest,
            mover,
            line,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/use_item` line, and hears the moved items
    /// for the store and the client.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_use_item_line(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::UseItemLine {
            vid,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/cards` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_cards_line(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::CardsLine { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/ride` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_ride_line(
        &self,
        vid: common::vid::Vid,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::RideLine { vid, mover, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/do_clear_affect` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_clear_affect(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ClearAffect { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/dragon_soul` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_dragon_soul(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DragonSoul { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/ds_list` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_ds_list(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DsList { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/ds_change_attr` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_ds_change_attr(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DsChangeAttr {
            vid,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/multi_farm` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_multi_farm(
        &self,
        vid: common::vid::Vid,
        addr: String,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MultiFarm { vid, addr, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/dice` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_dice(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Dice { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/gift` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_gift(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Gift { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/state` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_state(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::State { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/inventory` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_inventory(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Inventory { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/hair` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_hair(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Hair { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/advance_skill_select` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn advance_skill_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Option<crate::game_state::SetJobLanded>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AdvanceSkill { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/do_click_special_inv` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_click_special_inv(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ClickSpecialInv { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/fish_simul` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_fish_simul(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::FishSimul { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/shutdown` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_shutdown(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Shutdown { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one exit line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_do_cmd(
        &self,
        vid: common::vid::Vid,
        kind: ExitKind,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DoCmd { vid, kind, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/pvp` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_pvp(&self, vid: common::vid::Vid, rest: Vec<u8>) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Pvp { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/reload` line over the files the caller read.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_reload(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        read: crate::game_state::ReloadRead,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Reload {
            vid,
            rest,
            read,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/costume` line, and waits for the moved items.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn run_costume(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        mover: Mover,
    ) -> Result<Result<MovedItems, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Costume {
            vid,
            rest,
            mover,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world for the remaining `AFFECT_BLOCK_CHAT` duration of the
    /// character online under `vid`, or `None` when it holds none.
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn block_chat_of(&self, vid: common::vid::Vid) -> Result<Option<i32>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::BlockChatOf { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/block_chat` or `/jy` line, and waits for the
    /// moved silence. What the line shows is written to the players' outboxes
    /// before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn block_chat(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<BlockChatAnswer, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::BlockChat { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/block_chat_list` line. What the line shows
    /// is written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn block_chat_list(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::BlockChatList { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/dc` line. What the line shows is written to
    /// the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn dc_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Dc { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/transfer` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn transfer_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Transfer { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/kill` line. What the line shows is written
    /// to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn kill_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Kill { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/poison` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn poison_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Poison { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/stun` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn stun_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Stun { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/getqf` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn getqf_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GetQf { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/setqf` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn setqf_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SetQf { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/delqf` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn delqf_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DelQf { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/set_state` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn set_state_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SetState { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/qf` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn qf_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Qf { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/clear_quest` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn clear_quest_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ClearQuest { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/view_equip` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn view_equip_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ViewEquip { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/warp` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn warp_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Warp { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/private` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn private_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Private { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/rewarp` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn rewarp_command(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Rewarp { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world for `/observer_exit`'s asking half: the sort lock when `vid`
    /// observes, else `None`.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn observer_exit(&self, vid: common::vid::Vid) -> Result<Option<u32>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ObserverExit { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run `/observer_exit`'s clearing half. What the clear tells is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn observer_settled(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ObserverSettled { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/goto` line. What the line shows is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn goto_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Goto { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/level` line. What the line resets is
    /// written to the caller's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn level_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        gold: i64,
        voice: u8,
    ) -> Result<Option<crate::game_state::LevelHeld>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Level {
            vid,
            rest,
            gold,
            voice,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/advance` line. What the line resets is
    /// written to the target's outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn advance_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        gold: i64,
        voice: u8,
    ) -> Result<Option<crate::game_state::AdvanceLanded>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Advance {
            vid,
            rest,
            gold,
            voice,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/reset` line. What the line refills rides
    /// the outbox before the answer.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn reset_command(
        &self,
        vid: common::vid::Vid,
    ) -> Result<Option<crate::game_state::ResetHeld>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Reset { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to grant the `rank`th top-player affect after a `/level`
    /// reached 120. The step counted the store's rows and stood this caller's.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn grant_top_affect(
        &self,
        vid: common::vid::Vid,
        rank: u8,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GrantTopAffect { vid, rank, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/setskill` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn set_skill_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Option<crate::game_state::SetSkillLanded>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SetSkill { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/setskillother` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn set_skill_other_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Option<crate::game_state::SetSkillOtherLanded>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SetSkillOther { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/setskillpoint` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn set_skill_point_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Option<i32>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SetSkillPoint { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/setjob` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn set_job_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Option<crate::game_state::SetJobLanded>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SetJob { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/temp` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn temp_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Option<(u8, i64)>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Temp { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/setmonarch` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn set_monarch_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        candidate: Option<(u8, u32, i64)>,
    ) -> Result<Option<crate::game_state::SetMonarchLanded>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::SetMonarch {
            vid,
            rest,
            candidate,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/rmmonarch` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn rm_monarch_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<Option<u8>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::RmMonarch { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/invisible` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn invisible_command(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Invisible { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/purge` or `/weaken` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn purge_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
        weaken: bool,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Purge {
            vid,
            rest,
            weaken,
            reply,
        })
        .await
        .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/respawn` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn respawn_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Respawn { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/mob` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn mob_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Mob { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/mob_ld` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn mob_ld_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MobLd { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/ma` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn mob_aggressive_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MobAggressive { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/mc` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn mob_coward_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MobCoward { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/mm` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn mob_map_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MobMap { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/detaillog` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn detail_log_command(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::DetailLog { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/monsterlog` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn monster_log_command(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::MonsterLog { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/polymorph` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn polymorph_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Polymorph { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/forgetme` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn forget_me_command(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::ForgetMe { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/aggregate` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn aggregate_command(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Aggregate { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/attract_ranger` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn attract_ranger_command(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AttractRanger { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/pull_monster` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn pull_monster_command(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::PullMonster { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/group` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn group_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Group { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/grrandom` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn group_random_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::GroupRandom { vid, rest, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }

    /// Asks the world to run one `/cooltime` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn cooltime_command(&self, vid: common::vid::Vid) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Cooltime { vid, reply })
            .await
            .map_err(|_| MoveItemError::NotSent)?;
        answer.await.map_err(|_| MoveItemError::NoAnswer)
    }
    /// Asks the world to run one `/affect_remove` line.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn affect_remove_command(
        &self,
        vid: common::vid::Vid,
        rest: Vec<u8>,
    ) -> Result<(), MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::AffectRemove { vid, rest, reply })
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
