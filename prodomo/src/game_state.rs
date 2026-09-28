//! The state the game thread owns: the characters, the item ids, and the item prototypes.
//!
//! ADR-0002 says one process hosts auth and every Channel, that each Channel is one
//! world, and that all worlds step on one game thread at 25 Pulses per second. The
//! thread existed before this module, but it owned nothing: `main.rs` started it
//! with `|_| {}`, so it ticked and discarded the count. This module is what the
//! thread holds.
//!
//! # Why the state is here and not in the accept loop
//!
//! A character is only touched by the game thread. The accept loop is Tokio-owned
//! and its descriptors are not reachable from the game thread yet, so nothing that
//! mutates a world may run there. Keeping the world in a value that the thread owns
//! outright, with no `Arc` and no lock, is what makes that checkable rather than a
//! convention: the only way to reach this state is a command, and commands are
//! drained between pulses.
//!
//! # What is deliberately absent
//!
//! There is no Channel map set, no script VM, and no NPC. This holds the pieces the
//! item path needs and nothing more, because each of the rest is a separate unit
//! with its own decision to record. [`PulseProcessor::process_pulse`](crate::game_loop::PulseProcessor::process_pulse) therefore
//! steps nothing yet; it counts, and the count is what proves the thread is running
//! this value rather than an empty closure.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use gamedata::item_proto::ItemProtos;
use world::character::{CharacterManager, CharacterManagerError, Rejected};
use world::item::{ItemIdRange, ItemIds};

use tracing::{debug, warn};

use crate::client_registry::ClientOutbox;
use crate::game_loop::PulseProcessor;
use crate::game_loop_messages::GameCommand;
use crate::item_grant::{grant_item, GrantOutcome, GrantRefusal, GrantRequest};

/// Counters the owning side can read while the game thread is running.
///
/// Shared rather than returned, because the state is **moved** into the thread and
/// cannot be read afterwards. That is the point of owning it: the only handle that
/// outlives the move is one the game thread agrees to update.
#[derive(Debug, Default)]
pub struct GameStateMetrics {
    pulses: AtomicU64,
}

impl GameStateMetrics {
    /// Pulses the game thread has stepped.
    ///
    /// Zero is the honest value before the thread is spawned, and it is what a
    /// caller sees if the thread never started. A test that waits for a non-zero
    /// value is waiting for the thread, not for the construction.
    pub fn pulses(&self) -> u64 {
        self.pulses.load(Ordering::SeqCst)
    }
}

/// An id range was installed over an allocator that is already installed.
///
/// Named rather than logged, because the consequence is a duplicate item id and no
/// store write will report it: the second allocator hands out the same numbers the
/// first one did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyInstalled;

impl std::fmt::Display for AlreadyInstalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an item id allocator is already installed")
    }
}

impl std::error::Error for AlreadyInstalled {}

/// Why taking a granted item back out of the world did not succeed.
///
/// Every variant leaves the world unchanged, so a caller that gets one may retry or
/// report. [`RevokeRefused::Rejected`] is the one that matters most: it means the
/// storage named a cell and then refused to release it, which is a bug in the storage
/// rather than a state a caller can wait out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevokeRefused {
    /// The character is not online, so the world holds nothing of theirs.
    NoSuchCharacter {
        /// The name the revoke named.
        name: String,
    },
    /// The world does not hold that item for that character.
    ///
    /// A bug rather than a state to recover from: the id came from a grant that mutated
    /// the world moments earlier, and only a pulse in between could have moved it.
    NotThere {
        /// The id the revoke named.
        id: u32,
    },
    /// The storage named a cell and would not release it.
    Rejected {
        /// The id the revoke named.
        id: u32,
        /// What the storage reported.
        error: Rejected,
    },
}

impl std::fmt::Display for RevokeRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchCharacter { name } => {
                write!(formatter, "no online character named {name}")
            }
            Self::NotThere { id } => write!(formatter, "the world does not hold item {id}"),
            Self::Rejected { id, error } => {
                write!(formatter, "item {id} could not be released: {error}")
            }
        }
    }
}

impl std::error::Error for RevokeRefused {}

/// Why the world refused to admit a live client's character.
///
/// Every variant leaves the world unchanged, so the descriptor can close without the
/// world and the client having disagreed about whether the character exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnterWorldRefused {
    /// Another character is already indexed under that VID.
    ///
    /// A bug rather than a state to recover from: a second client with the same wire
    /// VID would make every record the world sends go to whichever descriptor answered
    /// last, and a grant would land on the wrong player.
    DuplicateVid {
        /// The VID the client arrived with.
        vid: common::vid::Vid,
    },
    /// Another character is already indexed under that player id.
    DuplicatePlayerId {
        /// The player id the client arrived with.
        player_id: u32,
    },
    /// Another character already holds that Name, compared case-insensitively.
    DuplicateName {
        /// The Name the client arrived with.
        name: String,
    },
    /// The VID is zero, which is legacy's `VID::NULL`.
    ///
    /// Refused rather than stored, because a zero VID is what a record with no target
    /// carries, so a world that indexed it would answer a "broadcast to nobody" claim
    /// with a character.
    NullVid,
    /// The world could not be asked, because the manager refused for a reason the
    /// world has no name of its own for.
    ///
    /// Kept apart from the duplicates above on purpose. Those are states a caller
    /// could act on -- this name is taken, do not retry it -- and this one is a bug in
    /// the crossing. Reporting it as a name duplicate would send an operator looking
    /// at a player's Name when the real fault is in the world, and would hide the
    /// exhaustion case entirely, which is the one that needs attention.
    NotAdmitted {
        /// The manager's own reason, verbatim, so nothing is lost in translation.
        reason: String,
    },
    /// A loaded item would not go where the load placed it.
    ///
    /// The load placed the same items in the same order into an empty inventory, so
    /// this is a bug in the crossing rather than a state a player can reach. The
    /// character is taken back out, because a world holding it without that item
    /// would offer the item's cell to the next grant.
    ItemRefused {
        /// The item's id.
        id: u32,
        /// Why the inventory would not take it.
        reason: Rejected,
    },
}

impl std::fmt::Display for EnterWorldRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateVid { vid } => {
                write!(formatter, "the world already holds {vid}")
            }
            Self::DuplicatePlayerId { player_id } => {
                write!(formatter, "the world already holds player {player_id}")
            }
            Self::DuplicateName { name } => {
                write!(
                    formatter,
                    "the world already holds a character named {name}"
                )
            }
            Self::NullVid => {
                formatter.write_str("a character cannot enter the world with a null VID")
            }
            Self::NotAdmitted { reason } => {
                write!(formatter, "the world refused this character: {reason}")
            }
            Self::ItemRefused { id, reason } => {
                write!(formatter, "the world refused loaded item {id}: {reason}")
            }
        }
    }
}

impl EnterWorldRefused {
    /// Names the world-side reason in the caller's terms.
    ///
    /// The two null-VID checks are the same rule seen from two places, and both are
    /// kept: the world checks before it calls the manager so a refusal costs no
    /// counter value, and the manager checks because it is also reachable directly.
    /// Mapping them onto one variant means a caller cannot tell which check fired,
    /// which is fine, because the repair is the same in both cases.
    fn from_manager(error: CharacterManagerError, name: &str) -> Self {
        match error {
            CharacterManagerError::DuplicateVid(vid) => Self::DuplicateVid { vid },
            CharacterManagerError::DuplicatePlayerId(player_id) => {
                Self::DuplicatePlayerId { player_id }
            }
            // The manager's variant carries the name it refused. The caller's copy is
            // used instead so a caller that passed a different spelling sees its own,
            // which is what the log line will print.
            CharacterManagerError::DuplicatePlayerName(_) => Self::DuplicateName {
                name: name.to_owned(),
            },
            CharacterManagerError::NullVid => Self::NullVid,
            // The remaining variants are lookups and a counter exhaustion, none of
            // which `create_player_with_vid` can produce. They are carried through by
            // their own text rather than folded into a duplicate: `VidExhausted` in
            // particular is the one case here an operator has to act on, and calling
            // it a name duplicate would bury it and send the search to a player's
            // Name instead of to the world.
            other => Self::NotAdmitted {
                reason: other.to_string(),
            },
        }
    }
}

impl std::error::Error for EnterWorldRefused {}

/// A grant was asked for before the world had an allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoItemIds;

impl std::fmt::Display for NoItemIds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the world has no item id allocator yet")
    }
}

impl std::error::Error for NoItemIds {}

/// The world-facing range for a range the store resolved.
///
/// The two range types are deliberately different: [`db::item_id_range::ItemIdRange`]
/// carries legacy's `dwMin`/`dwMax`/`dwUsableItemIDMin` and performs no checks, and
/// [`world::item::ItemIdRange`] refuses a range that would hand out id 0. The
/// conversion is here because `db` is below `world` and cannot see it, and a
/// `From` impl would be two foreign types. Both agree that `last` is never issued,
/// so the mapping is one to one.
///
/// # Errors
///
/// [`world::item::BadIdRange`] when `first_usable` is 0 or falls outside the span.
/// `db::items::resolve_item_id_range` cannot produce that for a span it accepted,
/// because it starts above the highest stored id, so this is a seam check rather than
/// an expected failure.
pub fn world_item_id_range(
    range: db::item_id_range::ItemIdRange,
) -> Result<world::item::ItemIdRange, world::item::BadIdRange> {
    world::item::ItemIdRange::new(range.min, range.max, range.usable_item_id_min)
}

/// The world state one game thread owns.
#[derive(Debug)]
pub struct GameState {
    characters: CharacterManager,
    item_ids: Option<ItemIds>,
    protos: Arc<ItemProtos>,
    metrics: Arc<GameStateMetrics>,
    last_pulse: u64,
    /// Where each online character's records go, keyed by the VID it entered under.
    ///
    /// Separate from the [`CharacterManager`] on purpose. The manager owns gameplay
    /// state that only the game thread touches; this map owns a **sender**, which is
    /// the one thing here that is not world state and must not be walked as if it
    /// were. Keeping them apart also means a departed client is removed from one map
    /// and cannot leave a stale sender behind in the other.
    outboxes: HashMap<common::vid::Vid, ClientOutbox>,
}
/// An item the world has taken back, together with whose it was.
///
/// Both halves are needed together and neither can be recovered from the other. The
/// cell is the only way to find the row, and the owner is half of that row's key; a
/// caller that looked either one up again could get a different answer than the world
/// acted on, and would then delete a row for an item that is still somewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Released {
    /// The cell the item was in, which the world has just freed.
    pub pos: protocol::item_pos::ItemPos,
    /// The store `player.id` of the character it belonged to.
    ///
    /// The world admitted this character under that id, so it is the same number the
    /// grant's row carried and the same number the descriptor publishes as the VID.
    pub owner_id: u32,
}

impl GameState {
    /// Build a state from the Game data alone.
    ///
    /// `protos` is read before the thread starts, not inside it, so a missing or
    /// malformed proto file stops `serve` before any port opens. That ordering is
    /// the same one `load_atlas` already uses, and it is the Rewrite's own: legacy
    /// accepts clients before its DB boot finishes, which AGENTS.md records as a
    /// Defect.
    ///
    /// There is no id range and no way to give this constructor one, because the
    /// start id is `MAX(id)` over the item table and that table is not readable
    /// until the store has migrated -- which happens inside the accept loop, after
    /// the listeners are bound. The range arrives later as
    /// [`GameCommand::InstallItemIdRange`].
    ///
    /// The alternative was a range in this constructor, and there is no honest value
    /// to put there: any fixed start would either reissue an id a stored item holds
    /// or leave a gap. An `Option` that is honestly absent is the only state that
    /// does not invent one.
    ///
    /// The prototypes may arrive shared. The item load at character select reads the
    /// same table on the descriptor's task before the world is asked, and one table in
    /// two places cannot disagree about an item's size or flags.
    #[must_use]
    pub fn new(protos: impl Into<Arc<ItemProtos>>) -> Self {
        Self {
            characters: CharacterManager::new(),
            item_ids: None,
            protos: protos.into(),
            metrics: Arc::new(GameStateMetrics::default()),
            last_pulse: 0,
            outboxes: HashMap::new(),
        }
    }

    /// A handle to the counters, which stays readable after the state is moved into
    /// the thread.
    #[must_use]
    pub fn metrics(&self) -> Arc<GameStateMetrics> {
        Arc::clone(&self.metrics)
    }

    /// The characters, for a caller that already holds `&mut GameState`.
    pub fn characters(&self) -> &CharacterManager {
        &self.characters
    }

    /// The characters, mutably.
    pub fn characters_mut(&mut self) -> &mut CharacterManager {
        &mut self.characters
    }

    /// The item id allocator, or `None` until one has been installed.
    ///
    /// `None` is a real state, not a placeholder: the start id is a fact about the
    /// stored items, and it is not known when this state is built.
    #[must_use]
    pub fn item_ids(&self) -> Option<&ItemIds> {
        self.item_ids.as_ref()
    }

    /// The item id allocator, mutably.
    pub fn item_ids_mut(&mut self) -> Option<&mut ItemIds> {
        self.item_ids.as_mut()
    }

    /// Install the allocator.
    ///
    /// Refuses a second install. One allocator has to serve the world's whole life:
    /// a second one starts again at the same `usable_item_id_min` and reissues ids
    /// that live items already hold, and there is no database write that would
    /// notice. An allocator that is installed over cannot be recovered, so this
    /// returns what the caller would lose rather than taking the first answer.
    ///
    /// # Errors
    ///
    /// [`AlreadyInstalled`] when an allocator is already installed. The existing one
    /// is left untouched, so a caller that ignores this error has lost nothing.
    ///
    /// # Panics
    ///
    /// Never. The borrow of the freshly installed value is taken from the same
    /// `Option` that was just assigned, so the `expect` cannot fail; it is written
    /// out because `Option::as_mut` says `None` is possible and here it is not.
    pub fn install_item_ids(
        &mut self,
        range: ItemIdRange,
    ) -> Result<&mut ItemIds, AlreadyInstalled> {
        if self.item_ids.is_some() {
            return Err(AlreadyInstalled);
        }
        self.item_ids = Some(ItemIds::new(range));
        Ok(self.item_ids.as_mut().expect("just installed"))
    }

    /// The item prototypes.
    pub fn protos(&self) -> &ItemProtos {
        &self.protos
    }

    /// The last pulse this state stepped.
    pub fn last_pulse(&self) -> u64 {
        self.last_pulse
    }
}

impl GameState {
    /// Apply one command to the world this thread owns.
    ///
    /// Split out from the trait impl so it can be called directly, which is what
    /// the tests do: a test that had to spawn a thread to learn whether a grant
    /// worked would be testing the scheduler, not the grant.
    pub fn apply(&mut self, command: GameCommand) {
        match command {
            GameCommand::ApplyAsyncCompletion { .. } => {
                // There is nothing to apply it to yet. A completion answers work
                // that game state asked for, and nothing has asked. Dropping it
                // here keeps the variant from being handled twice, in the loop and
                // here, which is how the two drift apart.
                debug_assert!(
                    false,
                    "a completion arrived for work this game state never started"
                );
            }
            GameCommand::GrantItem { request, reply } => {
                let answer = self.grant(&request);
                // A closed reply means the caller gave up, which it may legitimately
                // do when the descriptor it was writing to closed. The grant still
                // happened, and that is not a reason to undo it: the item is in the
                // world and the caller is the one that went away.
                if reply.send(answer).is_err() {
                    warn!(
                        target = ?request.target,
                        vnum = request.vnum,
                        "the item was granted but nobody was left to hear about it"
                    );
                }
            }
            GameCommand::InstallItemIdRange { range, reply } => {
                let answer = self.install_item_ids(range);
                // A dropped install answer leaves the world without an allocator,
                // which is the state it started in, so the caller can ask again.
                if reply.send(answer.is_ok()).is_err() {
                    warn!(
                        first = range.first,
                        last = range.last,
                        "the item id range was installed but nobody was left to hear about it"
                    );
                }
            }
            GameCommand::RevokeGrant { target, id, reply } => {
                let removed = match self.revoke_grant(&target, id) {
                    Ok(_) => true,
                    Err(error) => {
                        // A revoke that could not happen is the state a player can end up
                        // holding, so it is logged at warn with both ids rather than being
                        // folded into the boolean the reply carries.
                        warn!(target = ?target, id, %error, "a granted item could not be taken back");
                        false
                    }
                };
                // A dropped revoke answer leaves the world in the state the caller is
                // about to report as wrong, so the log line has to say which id and who
                // it belonged to: this is the state a player can be holding.
                if reply.send(removed).is_err() {
                    warn!(
                        target = ?target,
                        id,
                        "a granted item was taken back but nobody was left to hear about it"
                    );
                }
            }
            GameCommand::ReleaseItem { target, id, reply } => {
                let answer = self.revoke_grant(&target, id);
                // The same answer as the revoke, and for the same reason, but the
                // consequence is different: a refused release here means a row that is
                // about to be deleted is still held by the world, so it is logged with
                // both names rather than folded into a boolean the caller cannot act on.
                if let Err(error) = &answer {
                    warn!(target = ?target, id, %error, "an item could not be released from the world");
                }
                if reply.send(answer).is_err() {
                    warn!(
                        target = ?target,
                        id,
                        "an item was released but nobody was left to hear about it"
                    );
                }
            }
            GameCommand::EnterWorld {
                vid,
                player_id,
                name,
                items,
                outbox,
                reply,
            } => {
                let answer = self.enter_world_with_items(vid, player_id, &name, &items, outbox);
                // A dropped admit answer means the descriptor is already closing, and a
                // character nobody will play is not a state worth warning about: the
                // close path runs the leave, which finds nobody and says so at debug.
                if reply.send(answer).is_err() {
                    debug!(?vid, player_id, name = ?name.as_str(), "the world admitted a character nobody was left to hear about");
                }
            }
            GameCommand::DeliverRecord { vid, record, reply } => {
                // A `false` here means the character is not online or its descriptor
                // is gone. It is not logged as a warning: a record that could not be
                // delivered is a normal outcome for anything sent after a disconnect,
                // and the caller is told in the answer.
                if reply.send(self.write_to_client(vid, record)).is_err() {
                    debug!(
                        ?vid,
                        "the world wrote a record nobody was left to hear about"
                    );
                }
            }
            GameCommand::LeaveWorld { vid, reply } => {
                let removed = self.leave_world(vid);
                // A leave that finds nobody is a double leave, not a failure: the
                // descriptor is ending and the world already agrees the character is
                // gone. Logging it at warn would make an ordinary reconnect noisy.
                if !removed {
                    debug!(
                        ?vid,
                        "the world was asked to remove a character it did not hold"
                    );
                }
                if reply.send(removed).is_err() {
                    debug!(
                        ?vid,
                        "the world removed a character nobody was left to hear about"
                    );
                }
            }
            GameCommand::Stop => {
                // The loop handles `Stop` itself, before a command ever reaches a
                // processor. Reaching here would mean the loop and the state
                // disagree about who owns shutdown, and quietly ignoring it would
                // hide exactly that.
                debug_assert!(false, "the game loop must handle Stop itself");
            }
        }
    }

    /// Take a granted item back out of the world.
    ///
    /// This is the undo for the grant, and it exists because the grant mutates the world
    /// before its row can be written: the world is the only thing that can choose a cell,
    /// and the cell is part of the row. The id is enough to find the item again, because
    /// [`world::character::CharacterItems::release`] answers from the storage rather than being
    /// handed a cell the caller might have stale.
    ///
    /// # Errors
    ///
    /// [`RevokeRefused::NoSuchCharacter`] when the target is not online, and
    /// [`RevokeRefused::NotThere`] when the world does not hold the id, or
    /// [`RevokeRefused::Rejected`] when the storage named a cell it then would not
    /// release. None of the three changes anything.
    pub fn revoke_grant(&mut self, target: &str, id: u32) -> Result<Released, RevokeRefused> {
        let character = self.characters.find_player_mut(target).ok_or_else(|| {
            RevokeRefused::NoSuchCharacter {
                name: target.to_owned(),
            }
        })?;
        // Read before the release, not after: `release` takes `&mut self` on the
        // storage, and the owner is a property of the character, not of the item.
        let owner_id = character.player_id();
        let pos = character
            .items_mut()
            .release(id)
            .map_err(|error| RevokeRefused::Rejected { id, error })?;
        Ok(Released { pos, owner_id })
    }

    /// Puts a live client's character into the world under the VID it already uses.
    ///
    /// The VID is not allocated here and that is the whole point. Legacy allocates it
    /// (`CHARACTER_MANAGER::AllocVID`) and adds a CRC that never reaches the wire,
    /// but the Rewrite's live descriptor has been publishing the store's `player.id`
    /// as the VID since the enter-game burst was written, and the client has been
    /// treating it as that character's identity ever since. Allocating a second
    /// number here would put one character in the world under a VID nothing else
    /// knows, so a record the world sends would be addressed to a client that never
    /// asked for it. So the caller supplies the number and this refuses a clash.
    ///
    /// # Errors
    ///
    /// [`EnterWorldRefused::NullVid`], [`EnterWorldRefused::DuplicateVid`],
    /// [`EnterWorldRefused::DuplicatePlayerId`], or
    /// [`EnterWorldRefused::DuplicateName`]. Every one leaves the world unchanged, so
    /// the descriptor can close without the two sides disagreeing about who exists.
    pub fn enter_world(
        &mut self,
        vid: common::vid::Vid,
        player_id: u32,
        name: &str,
        outbox: ClientOutbox,
    ) -> Result<(), EnterWorldRefused> {
        self.enter_world_with_items(vid, player_id, name, &[], outbox)
    }

    /// Puts a live client's character into the world holding the items its load placed.
    ///
    /// The items are set in the order given, which is the order the load placed them in
    /// its own empty inventory, so each one lands where the client was told it is.
    ///
    /// # Errors
    ///
    /// As [`GameState::enter_world`], and [`EnterWorldRefused::ItemRefused`] when an
    /// item would not go where the load put it. The character is taken back out in that
    /// case, so every refusal still leaves the world unchanged.
    pub fn enter_world_with_items(
        &mut self,
        vid: common::vid::Vid,
        player_id: u32,
        name: &str,
        items: &[(protocol::item_pos::ItemPos, world::item::Item)],
        outbox: ClientOutbox,
    ) -> Result<(), EnterWorldRefused> {
        if vid.is_null() {
            return Err(EnterWorldRefused::NullVid);
        }
        // Checked before the create, because `CharacterManager::create_player` reports
        // a duplicate player id or name but allocates a **fresh** VID, so a duplicate
        // VID would otherwise slip past it and consume a counter value silently.
        if self.characters.find_by_vid(vid).is_ok() {
            return Err(EnterWorldRefused::DuplicateVid { vid });
        }
        // The manager is the authority on the indexes it owns. A clone of the outbox
        // is taken only after both sides agree the character is new, so a refusal
        // cannot leave a sender for a character that was never admitted.
        self.characters
            .create_player_with_vid(player_id, name, vid)
            .map_err(|error| EnterWorldRefused::from_manager(error, name))?;
        if let Err(refused) = self.place_loaded_items(name, items) {
            let _destroyed = self.characters.destroy(vid);
            return Err(refused);
        }
        let _previous = self.outboxes.insert(vid, outbox);
        Ok(())
    }

    /// Sets each loaded item into the character just created under `name`.
    fn place_loaded_items(
        &mut self,
        name: &str,
        items: &[(protocol::item_pos::ItemPos, world::item::Item)],
    ) -> Result<(), EnterWorldRefused> {
        let Some(character) = self.characters.find_player_mut(name) else {
            return Err(EnterWorldRefused::NotAdmitted {
                reason: format!("the character named {name} was created and then not found"),
            });
        };
        for (pos, item) in items {
            character.items_mut().set(*pos, item).map_err(|reason| {
                EnterWorldRefused::ItemRefused {
                    id: item.id,
                    reason,
                }
            })?;
        }
        Ok(())
    }

    /// Takes a live client's character out of the world.
    ///
    /// The sender is dropped **first**, so a record the world writes after this point
    /// finds no client rather than a client whose character no longer exists. That is
    /// the ordering the reverse case depends on too: a disconnect writes the row for
    /// the last time after this returns, and anything the world still held would be
    /// released by the destruction and the row would still name it.
    ///
    /// Returns `false` when the world did not hold that character, which is reported
    /// rather than treated as fatal because a disconnect that finds nobody is
    /// already ending.
    pub fn leave_world(&mut self, vid: common::vid::Vid) -> bool {
        let _outbox = self.outboxes.remove(&vid);
        self.characters.destroy(vid).is_ok()
    }

    /// Whether a character is online under that VID.
    ///
    /// This is what decides whether a record the world is about to build has a
    /// client to go to, so it is deliberately a world question and not a registry
    /// one: the registry is keyed by lease and knows about broadcasts, while the
    /// world is keyed by the VID a grant and a record both use.
    #[must_use]
    pub fn is_online(&self, vid: common::vid::Vid) -> bool {
        self.characters.find_by_vid(vid).is_ok()
    }

    /// How many characters the world holds.
    ///
    /// Counts the characters, not the senders, so a test that joins and leaves sees
    /// the same number on both sides of the round trip.
    #[must_use]
    pub fn online_count(&self) -> usize {
        self.characters.len()
    }

    /// Writes one record to a character's client.
    ///
    /// The world owns no socket, so this is the only way a record the world produced
    /// reaches a client (ADR-0002). A `false` means the client is gone; the caller
    /// decides whether that matters, and for a grant it does not, because the row is
    /// already written and the item is in the world either way -- it is the item
    /// that must not be silently lost, not the notification.
    pub fn write_to_client(&self, vid: common::vid::Vid, record: Vec<u8>) -> bool {
        self.outboxes
            .get(&vid)
            .is_some_and(|outbox| outbox.send(record))
    }

    /// Run one grant against the world, taking the target's own `Inven_Point`.
    fn grant(&mut self, request: &GrantRequest) -> Result<GrantOutcome, GrantRefusal> {
        let Some(item_ids) = self.item_ids.as_mut() else {
            // Refused before anything is placed, so no id is burned and the world is
            // unchanged. A caller that retries after the install gets a fresh answer.
            return Err(GrantRefusal::NoAllocator);
        };
        let inven_point = self
            .characters
            .find_player_mut(&request.target)
            .map_or(0, |character| character.inven_point());
        grant_item(
            &mut self.characters,
            &self.protos,
            item_ids,
            request,
            inven_point,
        )
    }
}

impl PulseProcessor for GameState {
    fn apply_command(&mut self, command: GameCommand) {
        self.apply(command);
    }

    /// Step one pulse.
    ///
    /// Counts, and nothing else. An empty body that only counts is the honest state
    /// of this unit: the world has characters and item ids, but no map set to step
    /// and no script to run, so there is no work a pulse could do that a pulse does
    /// not already do. The count is not decoration either -- it is the only way a
    /// test can tell that the game thread owns this value rather than the empty
    /// closure it replaced.
    fn process_pulse(&mut self, pulse: u64) {
        self.last_pulse = pulse;
        self.metrics.pulses.store(pulse, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    use common::item_slots::{usable_inventory_cells, INVENTORY_MAX_EXTENDED, INVENTORY_MAX_NUM};
    use common::vid::Vid;
    use protocol::item_pos::ItemPos;
    use world::character::Lookup;

    fn owners() -> ItemProtos {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        ItemProtos::load(&dir).expect("the owner's item protos load")
    }

    fn a_range() -> ItemIdRange {
        ItemIdRange::new(1, 1_000_000, 1).expect("a range that can issue an id")
    }

    /// A state with an allocator installed, which is the shape every world test
    /// after the install wants.
    fn a_state() -> GameState {
        let mut state = GameState::new(owners());
        state
            .install_item_ids(a_range())
            .expect("the first install");
        state
    }

    /// A sender nobody is draining, which is what a world write to a departed
    /// client meets. Kept as a helper because three of the tests below need the
    /// closed case and building it by dropping the receiver is the only honest way
    /// to get one.
    fn a_dropped_outbox() -> ClientOutbox {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        drop(rx);
        ClientOutbox::new(tx)
    }

    /// A sender with a receiver the test keeps, so a write can be read back.
    fn a_live_outbox() -> (ClientOutbox, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        (ClientOutbox::new(tx), rx)
    }

    #[test]
    fn a_character_enters_the_world_under_the_vid_the_descriptor_is_using() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        // The VID here is the store's `player.id`, which is the number the
        // enter-game burst already put on the wire. A world that allocated its own
        // would hold the character under an identity the client has never seen.
        state
            .enter_world(Vid::new(4_242), 4_242, "Shaman", outbox)
            .expect("the character is admitted");
        assert_eq!(state.online_count(), 1);
        assert!(state.is_online(Vid::new(4_242)));
        let character = state
            .characters()
            .find_by_vid(Vid::new(4_242))
            .expect("the character is in the world");
        assert_eq!(character.player_id(), 4_242);
        assert_eq!(character.name(), "Shaman");
    }

    #[test]
    fn a_second_client_on_the_same_vid_is_refused_and_changes_nothing() {
        let mut state = a_state();
        let (outbox, _first) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the first character is admitted");
        let (second_outbox, _second) = a_live_outbox();
        assert_eq!(
            state.enter_world(Vid::new(7), 8, "Warrior", second_outbox),
            Err(EnterWorldRefused::DuplicateVid { vid: Vid::new(7) })
        );
        // The refusal left one character, not two, and the one that is there is
        // still the first one. A world that admitted both would send every record
        // to whichever descriptor answered last.
        assert_eq!(state.online_count(), 1);
        let character = state
            .characters()
            .find_by_vid(Vid::new(7))
            .expect("the first character is still the one there");
        assert_eq!(character.name(), "Shaman");
        assert_eq!(character.player_id(), 7);
    }

    #[test]
    fn a_duplicate_name_is_refused_case_insensitively_because_names_are_case_insensitive() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the first character is admitted");
        let (other, _inbox) = a_live_outbox();
        // Legacy lowercases into its name index (`str_lower` in
        // `CHARACTER_MANAGER::CreateCharacter`), so a second login as `shaman` is the
        // same character as far as the world is concerned.
        assert_eq!(
            state.enter_world(Vid::new(8), 8, "sHaMaN", other),
            Err(EnterWorldRefused::DuplicateName {
                name: "sHaMaN".to_owned()
            })
        );
        assert_eq!(state.online_count(), 1);
    }

    #[test]
    fn a_null_vid_is_refused_because_it_is_what_a_record_with_no_target_carries() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        assert_eq!(
            state.enter_world(Vid::NULL, 1, "Shaman", outbox),
            Err(EnterWorldRefused::NullVid)
        );
        // Nothing was indexed, not even under the player id, so a later admission of
        // the same character is not blocked by a half-made one.
        assert_eq!(state.online_count(), 0);
        let (retry, _inbox) = a_live_outbox();
        assert!(state.enter_world(Vid::new(1), 1, "Shaman", retry).is_ok());
    }

    #[test]
    fn a_refused_admission_leaves_no_sender_behind_for_a_character_that_never_existed() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the first character is admitted");
        let (second, _inbox) = a_live_outbox();
        let _refused = state.enter_world(Vid::new(7), 8, "Warrior", second);
        // The refused sender was never inserted, so leaving the real character is the
        // only entry the map had and it leaves cleanly. A sender for a character that
        // was never admitted would keep a record addressable to a client the world
        // has no character for.
        assert!(state.leave_world(Vid::new(7)));
        assert_eq!(state.online_count(), 0);
    }

    /// An item of `size` cells, the way the load builds one before it is placed.
    fn a_loaded_item(id: u32, size: u8) -> world::item::Item {
        let mut item = world::item::Item::new(id, 19);
        item.set_size(size).expect("a positive footprint");
        item
    }

    #[test]
    fn the_admitted_character_holds_the_items_its_load_placed() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        let inventory = common::item_slots::EWindows::Inventory as u8;
        let items = vec![
            (ItemPos::new(inventory, 0), a_loaded_item(11, 2)),
            (ItemPos::new(inventory, 277), a_loaded_item(12, 1)),
        ];
        state
            .enter_world_with_items(Vid::new(7), 7, "Shaman", &items, outbox)
            .expect("the character is admitted with its items");
        let character = state
            .characters()
            .find_by_vid(Vid::new(7))
            .expect("the character is in the world");
        assert_eq!(
            character.items().get(ItemPos::new(inventory, 0)),
            Lookup::Occupied(11)
        );
        assert_eq!(
            character.items().get(ItemPos::new(inventory, 277)),
            Lookup::Occupied(12)
        );
        // The next grant is offered the first cell the loaded items leave free. A world that
        // admitted the character empty would offer cell 0, which the client already shows
        // as taken and the store's cell key already holds.
        let request = GrantRequest {
            target: "Shaman".to_owned(),
            vnum: 19,
            count: None,
        };
        let outcome = state.grant(&request).expect("the grant is placed");
        assert_eq!(outcome.row.pos, 1);
    }

    #[test]
    fn an_item_the_inventory_refuses_takes_the_character_back_out() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        let inventory = common::item_slots::EWindows::Inventory as u8;
        let items = vec![
            (ItemPos::new(inventory, 0), a_loaded_item(11, 2)),
            (ItemPos::new(inventory, 0), a_loaded_item(12, 1)),
        ];
        assert_eq!(
            state.enter_world_with_items(Vid::new(7), 7, "Shaman", &items, outbox),
            Err(EnterWorldRefused::ItemRefused {
                id: 12,
                reason: Rejected::AlreadyOccupied {
                    window: inventory,
                    cell: 0,
                    present: 11,
                },
            })
        );
        // Nothing is left behind: no character, no sender, and no index under the VID or the
        // Name, so the same character can be admitted again.
        assert_eq!(state.online_count(), 0);
        assert_eq!(state.characters().len(), 0);
        assert!(!state.write_to_client(Vid::new(7), vec![0x2B]));
        let (retry, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", retry)
            .expect("the same character is admitted afterwards");
    }

    #[test]
    fn the_world_writes_a_record_to_the_client_that_joined_under_that_vid() {
        let mut state = a_state();
        let (outbox, mut inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the character is admitted");
        assert!(state.write_to_client(Vid::new(7), vec![0x2B, 0x01, 0x02]));
        // Read it back, because a `true` from `write_to_client` only proves the
        // channel accepted the record; what the client sees is the point.
        assert_eq!(
            inbox.try_recv().expect("the record is queued"),
            vec![0x2B, 0x01, 0x02]
        );
    }

    #[test]
    fn a_write_to_a_departed_character_reports_failure_rather_than_queueing_nothing_silently() {
        let mut state = a_state();
        let (outbox, mut inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the character is admitted");
        state.leave_world(Vid::new(7));
        assert!(!state.write_to_client(Vid::new(7), vec![0x2B]));
        assert!(inbox.try_recv().is_err());
    }

    #[test]
    fn leaving_removes_the_sender_before_the_character_so_a_later_write_fails() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the character is admitted");
        assert!(state.leave_world(Vid::new(7)));
        // Both halves are gone together, which is what a grant checks before it
        // builds a record for a character.
        assert!(!state.is_online(Vid::new(7)));
        assert!(!state.write_to_client(Vid::new(7), vec![0x2B]));
        assert_eq!(state.online_count(), 0);
    }

    #[test]
    fn leaving_a_character_the_world_never_had_reports_false_rather_than_panicking() {
        let mut state = a_state();
        // A descriptor that never entered the game still runs the close path, so a
        // double leave is reachable and must not be a panic.
        assert!(!state.leave_world(Vid::new(999)));
    }

    #[test]
    fn a_character_in_the_world_is_reachable_by_the_name_a_grant_addresses() {
        // This is the whole reason a live client has to be in the world: a grant
        // addresses a character by Name, and before this the world held nothing a
        // Name could find.
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the character is admitted");
        let request = GrantRequest {
            target: "shaman".to_owned(),
            vnum: 30_000,
            count: None,
        };
        // The grant is asked for by a differently-cased spelling, which the world's
        // index accepts, and it places rather than answering "no such character".
        assert!(state.grant(&request).is_ok());
    }

    #[test]
    fn a_write_to_a_dropped_client_is_reported_as_failed() {
        let mut state = a_state();
        state
            .enter_world(Vid::new(7), 7, "Shaman", a_dropped_outbox())
            .expect("the character is admitted even though its descriptor is gone");
        // The world holds the character, so a grant would still place an item; the
        // notification is what cannot be delivered. Reporting that separately is the
        // point: the item is safe, the client will see it at relog.
        assert!(state.is_online(Vid::new(7)));
        assert!(!state.write_to_client(Vid::new(7), vec![0x2B]));
    }

    #[test]
    fn a_new_state_starts_with_no_characters_and_no_allocator() {
        let state = GameState::new(owners());
        assert_eq!(state.characters().len(), 0);
        // No allocator, and not a placeholder one: the start id is a fact about the
        // stored items, and a state that is built before the store is ready does not
        // know it.
        assert!(state.item_ids().is_none());
        assert_eq!(state.protos().len(), 7_305);
        assert_eq!(state.last_pulse(), 0);
    }

    #[test]
    fn an_installed_allocator_starts_unissued_at_the_first_usable_id() {
        let state = a_state();
        let ids = state.item_ids().expect("the allocator was installed");
        assert_eq!(ids.issued(), 0);
        assert_eq!(ids.peek(), 1);
        assert_eq!(ids.range().first, 1);
        assert_eq!(ids.range().last, 1_000_000);
    }

    #[test]
    fn a_second_install_is_refused_rather_than_replacing_the_allocator() {
        // A second allocator starts again at the same first usable id and reissues
        // ids live items hold. Nothing in the store would notice, so the refusal is
        // the only thing standing between a bug and duplicate item ids.
        let mut state = a_state();
        let before = state.item_ids().expect("one allocator").peek();
        assert_eq!(state.install_item_ids(a_range()), Err(AlreadyInstalled));
        assert_eq!(
            state.item_ids().expect("still one allocator").peek(),
            before
        );
    }

    #[test]
    fn a_grant_is_refused_before_an_allocator_is_installed_and_changes_nothing() {
        let mut state = GameState::new(owners());
        state
            .characters_mut()
            .create_player(1, "Shaman")
            .expect("the character exists");
        let request = GrantRequest {
            target: "Shaman".to_owned(),
            vnum: 30_000,
            count: None,
        };
        assert_eq!(state.grant(&request), Err(GrantRefusal::NoAllocator));
        // The refused grant placed nothing: no cell is taken, and no id is burned,
        // because the allocator is read before the placement search runs.
        let character = state
            .characters_mut()
            .find_player_mut("Shaman")
            .expect("the character");
        assert_eq!(character.items().len(), 0);
        // The control for that claim: the same world with an allocator hands the
        // first usable id out, so the ids above really were still unissued.
        state
            .install_item_ids(a_range())
            .expect("the first install");
        assert_eq!(state.item_ids().expect("the allocator").peek(), 1);
    }

    #[test]
    fn the_same_grant_succeeds_once_an_allocator_is_installed() {
        // The control for the test above, and the reason a retry is the answer:
        // the refusal above left the world in a state this one can act on.
        let mut state = GameState::new(owners());
        state
            .characters_mut()
            .create_player(1, "Shaman")
            .expect("the character exists");
        let request = GrantRequest {
            target: "Shaman".to_owned(),
            vnum: 30_000,
            count: None,
        };
        assert_eq!(state.grant(&request), Err(GrantRefusal::NoAllocator));
        state
            .install_item_ids(a_range())
            .expect("the first install");
        let outcome = state.grant(&request).expect("the grant happened");
        assert_eq!(
            state
                .characters_mut()
                .find_player_mut("Shaman")
                .expect("the character")
                .items()
                .len(),
            1
        );
        // The id came from the allocator that was installed after the refusal, so
        // the retry really did use the new allocator.
        assert_eq!(outcome.row.id, 1);
        assert_eq!(state.item_ids().expect("the allocator").issued(), 1);
    }

    #[test]
    fn a_pulse_records_its_number_and_bumps_the_shared_counter() {
        let mut state = a_state();
        let metrics = state.metrics();
        assert_eq!(metrics.pulses(), 0, "nothing has stepped it yet");

        for pulse in 1..=5 {
            state.process_pulse(pulse);
        }
        assert_eq!(state.last_pulse(), 5);
        assert_eq!(metrics.pulses(), 5, "the handle sees the same state");
    }

    #[test]
    fn the_metrics_handle_survives_the_state_being_dropped() {
        // The state is moved into the game thread, so nothing can read it afterwards.
        // The handle is the only thing that outlives the move, and a test that
        // asserts on the handle after the drop is asserting it is a real handle.
        let metrics = {
            let mut state = a_state();
            let handle = state.metrics();
            state.process_pulse(9);
            handle
        };
        assert_eq!(metrics.pulses(), 9);
    }

    #[test]
    fn the_stored_number_is_the_pulse_just_stepped_and_not_a_count_of_pulses() {
        // The distinction matters because the two agree until a number is skipped or
        // repeated. An accumulating counter would pass a one-pulse test and then
        // report a different number from `GameLoopSummary::final_pulse`, which the
        // cross-thread test compares against. Both numbers are 7 after seven
        // consecutive pulses, and only this pair separates them.
        let mut state = a_state();
        for pulse in 1..=7 {
            state.process_pulse(pulse);
        }
        assert_eq!(state.last_pulse(), 7);
        assert_eq!(state.metrics().pulses(), 7);

        // A repeated pulse number, which the loop would not send, leaves the state
        // reporting that number and not a larger one. That is the whole content of
        // the property: the value is replaced, never added to.
        state.process_pulse(7);
        assert_eq!(state.last_pulse(), 7);
        assert_eq!(state.metrics().pulses(), 7);
    }

    #[test]
    fn the_state_is_send_because_moving_it_into_the_thread_is_the_whole_design() {
        // `spawn_game_loop` takes the processor by value and moves it onto a fresh
        // OS thread, so a `GameState` that is not `Send` would not compile at the
        // call site. Asserting it here means the failure names this module rather
        // than a caller's signature.
        fn assert_send<T: Send>() {}
        assert_send::<GameState>();
    }

    #[test]
    fn a_character_the_state_owns_is_reachable_by_name() {
        let mut state = a_state();
        assert!(
            state.characters().find_player_by_name("Shaman").is_err(),
            "an empty state resolves no name"
        );
        state
            .characters_mut()
            .create_player(7, "Shaman")
            .expect("the first player is free");
        assert!(
            state.characters().find_player_by_name("Shaman").is_ok(),
            "and resolves it once the character exists"
        );
    }

    #[test]
    fn the_pulse_period_is_the_legacy_forty_milliseconds() {
        // 25 Pulses per second is ADR-0002, and it is the number the loop already
        // runs at. This test exists so the state cannot be built against a different
        // rate later without this failing to be noticed.
        assert_eq!(crate::game_loop::PULSE_PERIOD, Duration::from_millis(40));
    }

    // ---- the crossing ----------------------------------------------------
    //
    // The tests below are the ones that matter for ledger 202. Each is written as
    // if the claim were false, so that a change that breaks the crossing has to
    // break a test rather than pass a weaker one.

    fn grant_for(target: &str, vnum: u32) -> GrantRequest {
        GrantRequest {
            target: target.to_owned(),
            vnum,
            count: None,
        }
    }

    /// A one-cell, non-custom vnum from the owner's table, so the grant lands in the
    /// base inventory and nothing else decides the cell.
    fn a_plain_vnum() -> u32 {
        let protos = owners();
        protos
            .rows()
            .iter()
            .find(|proto| {
                proto.size == 1
                    && (0..6).all(|category| {
                        !gamedata::item_custom_category::is_custom_category(proto, category)
                    })
            })
            .expect("the owner's table has a one-cell item outside every custom bank")
            .vnum
    }

    #[test]
    fn a_grant_command_reaches_the_world_and_answers_with_the_outcome() {
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let vnum = a_plain_vnum();
        let (reply, answer) = tokio::sync::oneshot::channel();
        let command = GameCommand::GrantItem {
            request: grant_for("Shaman", vnum),
            reply,
        };

        state.apply(command);

        // The answer crossed, and it names the cell the world actually used.
        let outcome = answer
            .blocking_recv()
            .expect("the game thread answered")
            .expect("the grant happened");
        assert_eq!(
            outcome.record.cell.cell, 0,
            "the first free base cell is cell 0"
        );
        assert_eq!(outcome.record.vnum, vnum);
        assert_eq!(outcome.count, 1, "None means one");
        assert_eq!(outcome.bank, None, "and it went to the base inventory");
    }

    #[test]
    fn the_item_exists_in_the_world_and_not_only_in_the_answer() {
        // The answer is a copy. If the world were unchanged and only the answer
        // were right, a client would be told about an item that is not there. This
        // reads the world back through the same path the reducer used.
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let vnum = a_plain_vnum();
        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::GrantItem {
            request: grant_for("Shaman", vnum),
            reply,
        });
        let outcome = answer.blocking_recv().unwrap().unwrap();

        let character = state
            .characters_mut()
            .find_player_mut("SHAMAN")
            .expect("name matching is case-insensitive");
        let items = character.items();
        assert_eq!(items.len(), 1, "the world holds the item");
        assert_eq!(
            items.get(outcome.record.cell),
            Lookup::Occupied(outcome.row.id),
            "the answer's id is the one sitting in the answer's cell"
        );
        assert_eq!(
            items.cell_of(outcome.row.id),
            Some(outcome.record.cell),
            "and the same id answers to the same cell from the other direction"
        );
        assert_eq!(outcome.row.vnum, vnum, "the same prototype");
        assert_eq!(
            outcome.row.owner_id,
            Some(character.player_id()),
            "owned by the target, and not on the ground"
        );
        assert_eq!(
            outcome.row.pos,
            u32::from(outcome.record.cell.cell),
            "one row, one cell"
        );
    }

    #[test]
    fn a_refusal_comes_back_as_a_refusal_and_leaves_the_world_alone() {
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let (reply, answer) = tokio::sync::oneshot::channel();

        state.apply(GameCommand::GrantItem {
            request: grant_for("Nobody", a_plain_vnum()),
            reply,
        });

        assert_eq!(
            answer.blocking_recv().unwrap(),
            Err(GrantRefusal::NoSuchCharacter {
                name: "Nobody".to_owned()
            }),
            "an unknown name is a refusal, not a missing answer"
        );
        assert_eq!(
            state
                .characters_mut()
                .find_player_mut("Shaman")
                .unwrap()
                .items()
                .len(),
            0,
            "and nothing was placed anywhere"
        );
    }

    #[test]
    fn the_target_characters_own_inven_point_bounds_the_search() {
        // This is the reason `envanter` was added to `Character`. A grant has to be
        // placed against the target's own stat rather than against a number the
        // caller supplied, or an operator could hand out a cell the character
        // cannot draw. The test goes through the command, which is the only path
        // that has an `inven_point` to read at all.
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let vnum = a_plain_vnum();

        {
            let character = state.characters_mut().find_player_mut("Shaman").unwrap();
            assert_eq!(
                character.inven_point(),
                0,
                "a new character has the stat at 0, which buys the legacy 90 cells"
            );
        }

        // Fill those 90 cells through the command. The cell numbers are what make
        // this a test rather than a smoke test.
        for cell in 0..90u16 {
            let outcome = ask(&mut state, &grant_for("Shaman", vnum))
                .expect("a one-cell item fits while the stat is 0");
            assert_eq!(outcome.record.cell.cell, cell, "cells fill in order from 0");
        }
        assert_eq!(
            ask(&mut state, &grant_for("Shaman", vnum)),
            Err(GrantRefusal::NoRoom { size: 1 }),
            "the 91st cell is out of reach at stat 0"
        );

        // Raising the stat opens the rest of the base inventory, and the command
        // picks that up without the caller passing anything.
        state
            .characters_mut()
            .find_player_mut("Shaman")
            .unwrap()
            .set_inven_point(18);
        let outcome =
            ask(&mut state, &grant_for("Shaman", vnum)).expect("cell 90 is free at stat 18");
        assert_eq!(
            outcome.record.cell.cell, 90,
            "the raised stat opened the 91st cell"
        );
    }

    #[test]
    fn an_inven_point_above_the_base_inventory_is_clamped_on_write() {
        // Divergence 202.1. Legacy copies the stat out of the stored blob and never
        // clamps, and `usable_inventory_cells` is a bare sum, so a hand-edited row
        // can name a cell in the equipment window. The Rewrite refuses to store a
        // stat that would do that.
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let character = state.characters_mut().find_player_mut("Shaman").unwrap();
        character.set_inven_point(u16::MAX);

        assert_eq!(
            character.inven_point(),
            INVENTORY_MAX_EXTENDED,
            "18 is the largest stat that still lands inside the base inventory"
        );
        assert_eq!(
            usable_inventory_cells(character.inven_point()),
            INVENTORY_MAX_NUM,
            "and at that stat the usable count is exactly the array length"
        );
        assert_eq!(INVENTORY_MAX_EXTENDED, 18, "the derived constant, pinned");
    }

    /// Sends one grant command to `state` on the calling thread and waits for it.
    ///
    /// Every crossing test goes through here so a change to the command shape breaks
    /// one place rather than six.
    fn ask(state: &mut GameState, request: &GrantRequest) -> Result<GrantOutcome, GrantRefusal> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::GrantItem {
            request: request.clone(),
            reply,
        });
        answer.blocking_recv().expect("the game thread answered")
    }

    #[test]
    fn a_dropped_reply_does_not_undo_the_grant() {
        // The caller is allowed to vanish: a descriptor can close while the world
        // is being changed. If that rolled the item back, a lost connection would
        // be a way to lose an item the operator was told was granted.
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let vnum = a_plain_vnum();
        let (reply, answer) = tokio::sync::oneshot::channel();
        drop(answer);

        state.apply(GameCommand::GrantItem {
            request: grant_for("Shaman", vnum),
            reply,
        });

        assert_eq!(
            state
                .characters_mut()
                .find_player_mut("Shaman")
                .unwrap()
                .items()
                .len(),
            1,
            "the item is still in the world; the listener went, not the item"
        );
    }

    #[test]
    fn apply_is_reachable_without_a_thread_so_a_grant_can_be_tested_directly() {
        // The method exists so the tests above do not have to spawn a thread. A
        // test that spawned one would be testing the scheduler, and a scheduler
        // bug would then be reported as a grant bug.
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let vnum = a_plain_vnum();
        let (reply, answer) = tokio::sync::oneshot::channel();
        GameState::apply(
            &mut state,
            GameCommand::GrantItem {
                request: grant_for("Shaman", vnum),
                reply,
            },
        );
        assert!(answer.blocking_recv().unwrap().is_ok());
    }

    #[test]
    fn metrics_start_at_zero_and_only_the_thread_advances_them() {
        // The negative control for "the game thread owns this state": before the
        // thread runs, the counter is zero, so a test that observes a non-zero value
        // has observed the thread and nothing else.
        let metrics = a_state().metrics();
        let deadline = Instant::now() + Duration::from_millis(50);
        while Instant::now() < deadline {
            assert_eq!(metrics.pulses(), 0, "no thread was started");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
