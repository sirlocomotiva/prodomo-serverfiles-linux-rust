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
        /// Where the game thread reports whether it installed.
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
    /// Answers the points and quickslots of the character online under `vid`, which the
    /// descriptor's save writes from: the world changes them on its own too (the potion
    /// recovery on its pulse, a trade the partner closes), so the copy the descriptor held
    /// after its last step can be stale.
    KeptOf {
        /// The VID the character entered the world under.
        vid: common::vid::Vid,
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
    /// Requests terminal loop shutdown.
    Stop,
}

/// How a character leaves the world.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Departure {
    /// Its client disconnects.
    Disconnect,
    /// Its client warps, which takes the body out of its sectree first (`G/char.cpp:6751-6752`).
    Warp,
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

/// What a hit changed that the descriptor keeps.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AttackAnswer {
    /// The gold the hit stole, as the signed change the store applies, or `None` when it
    /// stole none.
    pub gold: Option<i64>,
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
    ) -> Result<Result<(), InstallError>, InstallError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::InstallGuilds {
            stored,
            marks,
            symbols,
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

    /// Asks the world for the points and quickslots of the character online under `vid`,
    /// which a save writes from.
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub async fn kept_of(&self, vid: common::vid::Vid) -> Result<Option<Kept>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::KeptOf { vid, reply })
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
    ) -> Result<Result<ShopAnswer, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Shop {
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
    ) -> Result<Result<super::game_state::QuestStepOutcome, MoveItemRefused>, MoveItemError> {
        let (reply, answer) = oneshot::channel();
        self.send_command(GameCommand::Quest {
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
