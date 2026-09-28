//! Writing a granted item to the store, and taking the world back if the write fails.
//!
//! [`grant_item`](crate::item_grant::grant_item) mutates the world and answers with a row
//! nobody has written yet. That gap is this module's whole subject, and the order matters:
//! the world is mutated first, because the world is the only thing that can choose a cell,
//! and the cell is part of the row. So a failed write leaves an item in the world with no
//! row, and the honest repair is to put the world back rather than to leave a cell occupied
//! by something the player will not see.
//!
//! The repair is [`GameState::revoke_grant`](crate::game_state::GameState::revoke_grant),
//! and it runs on the thread that owns the world, like every other mutation. The caller
//! here is Tokio, so it asks through a command and waits for the answer, exactly as a grant
//! does.
//!
//! **A duplicate id is refused, not merged.** `db::items::save_item` upserts on `id`, which
//! is right for a background inventory write and wrong here: `id` is the table's primary key
//! across all owners, so an upsert with an id another character already holds moves that
//! item to the new owner and cell instead of failing. A grant that silently stole another
//! player's item would be worse than one that refused, so this module writes through
//! [`db::items::insert_item`], which has no `ON CONFLICT` clause at all.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::game_loop_messages::GameLoopController;
use crate::item_grant::GrantRequest;

use db::items::{insert_item, ItemError, ItemRow};
use db::store::Store;

use crate::game_loop_messages::{ReleaseError, RevokeError};
use crate::item_grant::GrantOutcome;

/// What happened to a granted item's row.
#[derive(Debug)]
pub enum Persisted {
    /// The row is in the store, and the client record may go out.
    Written,
    /// The row could not be written, and the world was put back.
    ///
    /// Carries the store's own error, because "it did not save" is not an answer an
    /// Operator can act on. The world is back to what it was before the grant, and the
    /// id is burned: the allocator is monotonic and handing it back would be the one
    /// thing that could make a later id collide with this one.
    Undone {
        /// The error the store reported.
        error: ItemError,
    },
    /// The row could not be written, and the world could not be put back.
    ///
    /// This is a bug, not a state to recover from, and it is a separate variant rather
    /// than a flag because the caller must do different things: the player has an item
    /// in a world that has no record of it, so the descriptor has to be closed rather
    /// than told the grant failed. A caller that treated this as [`Persisted::Undone`]
    /// would leave a player walking around with an item that vanishes at relog.
    Diverged {
        /// The error the store reported, which is the original problem.
        error: ItemError,
        /// Why the world could not be put back.
        revoke: RevokeError,
    },
}

impl fmt::Display for Persisted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Written => formatter.write_str("the item row was written"),
            Self::Undone { error } => write!(formatter, "the item was taken back: {error}"),
            Self::Diverged { error, revoke } => write!(
                formatter,
                "the item was neither saved nor taken back, and the world is now wrong: \
                 {error}, then {revoke}"
            ),
        }
    }
}

/// The part of the game thread that can take a granted item back out of the world.
///
/// This is a trait rather than a `&GameLoopController` for two reasons, and the second is
/// the one that decided it. The first is that the store half needs a database, so a test
/// of the repair path would otherwise have to have one; a trait lets the world half be
/// exercised against a recorded call. The second is that the repair path is the one that
/// must not be wrong, and a test that can drive it without a thread, a runtime, and a
/// database is a test that will actually be written.
pub trait GrantRevoker {
    /// Takes the item back out of the world, on the thread that owns it.
    ///
    /// This returns a boxed `Send` future rather than being an `async fn`, and that is
    /// not a style choice. A native `async fn` in a public trait cannot promise its future
    /// is `Send`, and this one has to be: the real implementation awaits a oneshot across
    /// a thread boundary, so a future that was not `Send` would compile here and fail to
    /// satisfy the caller that spawns it. Naming the bound here makes that a compile
    /// error at the implementation instead of a puzzling one at the spawn.
    ///
    /// # Errors
    ///
    /// Whatever the world reports. The caller treats every variant as "the world is
    /// still wrong" and says so.
    fn revoke_grant<'life, 'async_trait>(
        &'life self,
        target: &'life str,
        id: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), RevokeError>> + Send + 'async_trait>>
    where
        'life: 'async_trait,
        Self: 'async_trait;
}

impl GrantRevoker for GameLoopController {
    fn revoke_grant<'life, 'async_trait>(
        &'life self,
        target: &'life str,
        id: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), RevokeError>> + Send + 'async_trait>>
    where
        'life: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(GameLoopController::revoke_grant(self, target, id))
    }
}

/// Write `outcome`'s row, and put the world back if the store refuses it.
///
/// The order is write-then-deliver, not the reverse. A client that receives `GC_ITEM_SET`
/// for an item that is not stored sees something the next login takes away, and a stored
/// item nobody has seen yet is a state the client recovers from by asking again. The
/// record is only the caller's to send once this returns [`Persisted::Written`].
///
/// # Errors
///
/// Never. A store failure is an outcome, not an error, because the caller still has work
/// to do either way: send the record, or tell the player the grant failed. Returning
/// `Result` here would push that choice onto every caller and most of them would forget
/// it.
pub async fn persist_grant<R>(
    store: &Store,
    revoker: &R,
    target: &str,
    outcome: GrantOutcome,
) -> Persisted
where
    R: GrantRevoker,
{
    let row: ItemRow = outcome.row;
    match insert_item(store, &row).await {
        Ok(()) => Persisted::Written,
        Err(error) => match revoker.revoke_grant(target, row.id).await {
            Ok(()) => Persisted::Undone { error },
            Err(revoke) => Persisted::Diverged { error, revoke },
        },
    }
}

/// What happened to a grant that reached the store and then the client.
///
/// The three outcomes are named separately because a caller acts on each one
/// differently, and collapsing them is how a player ends up with an item nobody can
/// account for:
///
/// - [`Granted::Delivered`] is the only one where the client has seen the item.
/// - [`Granted::Stored`] is an item with a row that the client has not been told about.
///   The item is safe and the player will see it at relog, or a second grant of the
///   same vnum will produce a second row. It is not a failure and not a success.
/// - [`Granted::Failed`] is a grant the client was never told about, so the player sees
///   nothing at all. A failed write that was repaired, and a failed write that was not,
///   both land here; the difference is in the `Display` text and in the log the
///   caller writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Granted {
    /// The row was written and the client was told.
    Delivered {
        /// The item id, which is also the row's primary key and the id the world holds.
        id: u32,
        /// The cell the item was placed in, as `(window_type, pos)`.
        cell: (u8, u32),
    },
    /// The row was written, but the client could not be told.
    ///
    /// Distinct from [`Granted::Delivered`] because the item exists and will be there at
    /// relog, so the caller must not try to re-grant it. Re-granting would make a
    /// second row and a second cell for one Operator action.
    Stored {
        /// The item id, so the Operator can find the row and correct it.
        id: u32,
        /// The cell the item was placed in, as `(window_type, pos)`.
        cell: (u8, u32),
        /// Why the client was not told. Kept as text because the two causes -- an
        /// offline character and a gone descriptor -- are the caller's to report
        /// differently, and neither has a value this crate can name.
        why: String,
    },
    /// The grant did not happen.
    Failed {
        /// The full text of the [`Persisted`] outcome, so a `Diverged` -- where the
        /// player may hold an item with no row -- is visible in one log line rather
        /// than hidden behind a single word.
        why: String,
    },
}

impl std::fmt::Display for Granted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Delivered { id, cell } => {
                write!(
                    formatter,
                    "item {id} was given and the client was told (cell {}.{})",
                    cell.0, cell.1
                )
            }
            Self::Stored { id, cell, why } => write!(
                formatter,
                "item {id} was given and saved (cell {}.{}), but the client was not told: {why}",
                cell.0, cell.1
            ),
            Self::Failed { why } => {
                write!(formatter, "the grant did not happen: {why}")
            }
        }
    }
}

/// Grant an item to `target`, save it, and tell the client -- in that order, and only
/// then.
///
/// This is the whole grant end to end, and it is one function so the order cannot be
/// got wrong by a caller. The three steps are not interchangeable:
///
/// 1. The world places the item, because the world alone chooses the cell and the cell
///    is part of the row.
/// 2. The row is written, because a client that is told about an item with no row sees
///    something the next login takes away.
/// 3. The client is told, and only then. Sending before step 2 is the failure this
///    ordering exists to prevent.
///
/// The record is encoded here rather than in the world, so `world` never depends on
/// `protocol` and the encode stays next to the tests that pin its bytes.
///
/// # Errors
///
/// Never. Every failure is an outcome, because a caller has to report all three and a
/// `Result` would let a caller report only two.
pub async fn grant_and_deliver(
    store: &Store,
    controller: &GameLoopController,
    request: &GrantRequest,
) -> Granted {
    let outcome = match controller.request_grant(request.clone()).await {
        Ok(Ok(outcome)) => outcome,
        // The world refused. The item was not placed, so there is nothing to write and
        // nothing to tell. The refusal is the answer.
        Ok(Err(refusal)) => {
            return Granted::Failed {
                why: refusal.to_string(),
            }
        }
        // No answer at all means the game thread is gone. Reporting this as a refusal
        // would be a lie: the item may or may not have been placed, and only the world
        // knows. It is reported as its own failure so nobody retries blindly.
        Err(error) => {
            return Granted::Failed {
                why: format!("the world could not be asked: {error}"),
            }
        }
    };

    // The id and the cell are read before the outcome is moved, because a
    // `Diverged` still has to name the item the player may be holding.
    let id = outcome.row.id;
    let cell = (outcome.row.window_type, outcome.row.pos);
    // The world admitted this character under its store `player.id` (ledger 205), and
    // the row's owner is that same id, so the owner is the address the record goes to.
    // `None` cannot happen here: a grant always has an owner, and a row without one
    // would have been refused by the store's own foreign key.
    let Some(owner) = outcome.row.owner_id else {
        return Granted::Failed {
            why: "the granted row has no owner, so there is no client to tell".to_owned(),
        };
    };
    let record = outcome.record.encode();

    match persist_grant(store, controller, &request.target, outcome).await {
        Persisted::Written => {
            match controller
                .deliver_record(common::vid::Vid::new(owner), record)
                .await
            {
                // Delivered. The client has the item and the store has the row.
                Ok(true) => Granted::Delivered { id, cell },
                // The row is written and the item is real, but the client was not told.
                // Not a failure and not retried: re-granting would write a second row.
                Ok(false) => Granted::Stored {
                    id,
                    cell,
                    why: "the character has no connected client".to_owned(),
                },
                Err(error) => Granted::Stored {
                    id,
                    cell,
                    why: format!("the world could not deliver the record: {error}"),
                },
            }
        }
        // The write was refused. The world has already been told to take the item back,
        // so there is nothing to tell the client. Both `Undone` and `Diverged` land
        // here, and they carry different text, because a `Diverged` is the one case
        // where a player is holding something the store has no row for.
        refused => Granted::Failed {
            why: refused.to_string(),
        },
    }
}

/// What happened to a destroyed item.
#[derive(Debug)]
pub enum Destroyed {
    /// The world no longer holds it and the row is gone.
    Gone {
        /// The id that was destroyed.
        id: u32,
        /// The cell the world freed, as `(window_type, cell)`.
        ///
        /// Kept so the caller can log or answer with the same numbers the world used,
        /// rather than with a cell it guessed.
        cell: (u8, u32),
        /// Whether the client was sent the record that empties the cell.
        ///
        /// `false` means the character has no connected client, or the game thread did
        /// not answer. Neither undoes the destroy: the row is already gone, so the next
        /// login draws the cell empty.
        told: bool,
    },
    /// The world released the item but the row is still there.
    ///
    /// This is a bug, and it is its own variant because the caller must do something about
    /// it rather than report a clean destroy. The item is in no world and still in the
    /// store, so a player who logs in sees an item that no longer exists, and a second
    /// destroy of the same id would release nothing and delete the row. The store
    /// failed to do exactly what it was asked, so this is logged at `error` and reported
    /// as a failure: the id is not burned, because the row it names is still real.
    StillStored {
        /// The id that was asked for.
        id: u32,
        /// The store's own reason.
        error: ItemError,
    },
    /// The world would not release it, so the store was not asked.
    ///
    /// The refusal is kept whole rather than collapsed into a message, because a caller
    /// answers an Operator differently for "that character is not online" than for "that
    /// id is not an item this character holds", and the second one is the ordinary answer
    /// to an Operator who typed an id from memory.
    Refused {
        /// The id that was asked for.
        id: u32,
        /// What the world said.
        error: ReleaseError,
    },
}

/// Take an item out of the world, delete its row, and report what happened.
///
/// This is the mirror of [`grant_and_deliver`], and it is a separate function rather than
/// a flag on it because the two orders are opposites and neither can be derived from the
/// other. The world goes first, for the same reason it does on a grant and not the
/// opposite: the world is the only thing that knows which cell an id occupies, and the
/// store's key needs that cell. So the row is deleted **second**, and a store that refuses
/// the delete leaves an item with no row -- the exact state `grant_and_deliver` exists to
/// prevent, which is why [`Destroyed::StillStored`] is a separate variant and not a
/// successful destroy with a note.
///
/// Once the row is gone, the client is sent the record that empties the cell, which is
/// the order a move keeps too: the store first, the client second, so a client is never
/// shown a change the store could still refuse.
///
/// Legacy clears a cell with `HEADER_GC_ITEM_DEL` (byte 20) in its deprecated 62-byte
/// layout. The Reference client frames byte 20 at the width of its own item-set record, so
/// it drops that frame, and sending it would reproduce the Defect ledger 193.8 records. The
/// Rewrite sends a `GC_ITEM_SET` (byte 21) with vnum 0 instead, which the client reads as
/// an empty cell ([`world::item::gc_item_clear`]). Ledger 211.3 records it as a Divergence
/// and supersedes 208.2, which sent nothing; the play test calibrates it.
///
/// # Errors
///
/// Never. Every failure is an outcome, for the same reason as [`grant_and_deliver`].
pub async fn destroy_and_delete(
    store: &Store,
    controller: &GameLoopController,
    target: &str,
    id: u32,
) -> Destroyed {
    // Step 1: the world. A refusal here means the store is not touched, so there is no
    // window in which a row describes an item the world has already lost.
    let released = match controller.release_item(target, id).await {
        Ok(released) => released,
        Err(error) => {
            return Destroyed::Refused { id, error };
        }
    };
    let cell = (released.pos.window_type, u32::from(released.pos.cell));
    // The owner comes from the same release rather than from a second lookup. The world
    // admitted the character under its store `player.id` (ledger 205), so asking the
    // world is the only way to get an owner that cannot disagree with the cell the world
    // just freed. Looking it up again could answer with a different character if a name
    // had been reused in between, and the delete would then remove somebody else's row.
    let owner = released.owner_id;

    match db::items::destroy_item(store, id, owner).await {
        Ok(true) => {
            let mut record = Vec::new();
            world::item::gc_item_clear(released.pos).encode_into(&mut record);
            let told = controller
                .deliver_record(common::vid::Vid::new(owner), record)
                .await
                .unwrap_or(false);
            Destroyed::Gone { id, cell, told }
        }
        // `destroy_item` answers `false` only when the id does not exist at all, which
        // cannot be true here: the world held it and the world is the only thing that
        // grants ids. It is reported rather than assumed away.
        Ok(false) => Destroyed::StillStored {
            id,
            error: ItemError::NotOwned {
                id,
                owner_id: Some(owner),
            },
        },
        Err(error) => {
            tracing::error!(target = ?target, id, %error, "an item left the world but its row is still in the store");
            Destroyed::StillStored { id, error }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{insert_item, persist_grant, GrantRevoker, Persisted};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use db::items::{ItemError, ItemRow};
    use db::store::{Store, StoreConfig};

    use crate::game_loop_messages::RevokeError;
    use crate::item_grant::GrantOutcome;

    /// A revoker that records what it was asked to do, and can be told to fail.
    ///
    /// This is the control the repair path needs. Without a positive control -- a
    /// revoker that actually succeeds and is observed doing the work -- a test that
    /// passed because the revoker was never called would be indistinguishable from one
    /// that passed because the world was put back.
    struct Recording {
        calls: Arc<Mutex<Vec<(String, u32)>>>,
        answer: Result<(), RevokeError>,
    }

    impl Recording {
        fn succeeding(calls: &Arc<Mutex<Vec<(String, u32)>>>) -> Self {
            Self {
                calls: Arc::clone(calls),
                answer: Ok(()),
            }
        }

        fn refusing(calls: &Arc<Mutex<Vec<(String, u32)>>>) -> Self {
            Self {
                calls: Arc::clone(calls),
                answer: Err(RevokeError::NoAnswer),
            }
        }
    }

    impl GrantRevoker for Recording {
        fn revoke_grant<'life, 'async_trait>(
            &'life self,
            target: &'life str,
            id: u32,
        ) -> Pin<Box<dyn Future<Output = Result<(), RevokeError>> + Send + 'async_trait>>
        where
            'life: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("the call log lock should not be poisoned")
                    .push((target.to_owned(), id));
                self.answer
            })
        }
    }

    /// A store whose URL parses and names nothing, so every query fails on connect.
    ///
    /// A transport failure is the honest default for these tests: they are about what
    /// happens to the world after a write is refused, and they must not depend on which
    /// kind of refusal it was.
    fn a_store_nothing_answers() -> Store {
        Store::lazy(&StoreConfig {
            url: "postgres://prodomo:prodomo-test@127.0.0.1:1/prodomo".to_owned(),
            max_connections: 1,
        })
        .expect("a well-formed store configuration")
    }

    /// A `GrantOutcome` for an item that is not in any store.
    ///
    /// The row is built by hand rather than taken from a real grant, because these tests
    /// are about the store and the repair and not about the placement. `window_type` is
    /// the measured `EWindows::Inventory` value rather than a remembered ordinal, and
    /// `pos` is the absolute cell the store keys on.
    fn an_outcome(id: u32, owner: u32, vnum: u32) -> GrantOutcome {
        let mut row = ItemRow::on_ground(id, 0, vnum, 1);
        row.owner_id = Some(owner);
        row.window_type = common::item_slots::EWindows::Inventory as u8;
        row.pos = 0;
        GrantOutcome {
            record: an_item_set(vnum),
            row,
            count: 1,
            bank: None,
        }
    }

    /// The 72-byte `GC_ITEM_SET` for a one-cell item, which is what a grant hands back.
    fn an_item_set(vnum: u32) -> protocol::gc_item_window::GcItemSet {
        protocol::gc_item_window::GcItemSet {
            cell: protocol::item_pos::ItemPos {
                window_type: common::item_slots::EWindows::Inventory as u8,
                cell: 0,
            },
            vnum,
            count: 1,
            refine_element: 0,
            transmutation: 0,
            flags: 0,
            anti_flags: 0,
            // `bHighlight` is a raw byte the codec's own note calls opaque, and every
            // value round-trips. Zero is a value, not a stand-in for "unset".
            highlight: 0,
            sockets: [0; 6],
            attributes: [protocol::gc_item_window::ItemAttribute {
                b_type: 0,
                s_value: 0,
            }; 7],
        }
    }

    #[tokio::test]
    async fn a_store_nothing_answers_actually_answers_something() {
        // A helper that never returns would hang every test that leans on it, and a
        // timeout is a much worse failure to read than an assertion. This is the
        // positive control for the negative control.
        let store = a_store_nothing_answers();
        // The bound is `StoreConfig::ACQUIRE_TIMEOUT` plus slack, not two seconds.
        // sqlx keeps retrying a refused connection until the pool gives up, so a store
        // that is merely *down* takes that long to be reported -- which is the store's
        // documented behaviour and not something this unit should try to shorten.
        let answer = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            insert_item(&store, &an_outcome(1, 2, 3).row),
        )
        .await;
        assert!(
            answer.is_ok(),
            "a refused connection must return in bounded time, not hang the caller"
        );
        assert!(answer.expect("the timeout did not fire").is_err());
    }

    #[tokio::test]
    async fn a_refused_write_puts_the_world_back_by_id() {
        // Given: a grant whose row cannot be written.
        let store = a_store_nothing_answers();
        let calls = Arc::new(Mutex::new(Vec::new()));

        // When: the write is attempted.
        let outcome = persist_grant(
            &store,
            &Recording::succeeding(&calls),
            "Aaa",
            an_outcome(9, 7, 10),
        )
        .await;

        // Then: the world was put back, and it was the right id for the right character.
        // Naming the id matters: a repair that released *some* item would pass a test
        // that only counted calls, and would leave the player holding the wrong thing.
        assert!(matches!(outcome, Persisted::Undone { .. }), "{outcome}");
        assert_eq!(
            *calls.lock().expect("the lock"),
            vec![("Aaa".to_owned(), 9u32)],
            "exactly the granted item was released, for the granted character"
        );
    }

    #[tokio::test]
    async fn a_world_that_cannot_be_put_back_is_reported_separately() {
        // Given: a store that refuses the write and a world that refuses the repair.
        let store = a_store_nothing_answers();
        let calls = Arc::new(Mutex::new(Vec::new()));

        // When: the write is attempted.
        let outcome = persist_grant(
            &store,
            &Recording::refusing(&calls),
            "Aaa",
            an_outcome(9, 7, 10),
        )
        .await;

        // Then: the caller is told the world is wrong, not merely that the save failed.
        // Collapsing the two would let a player keep an item that vanishes at relog.
        match outcome {
            Persisted::Diverged { error, revoke } => {
                assert!(
                    matches!(error, ItemError::Database(_)),
                    "the original failure is carried through, not replaced: {error}"
                );
                assert_eq!(revoke, RevokeError::NoAnswer);
            }
            other => panic!("a failed repair must be its own outcome, got {other}"),
        }
        assert_eq!(calls.lock().expect("the lock").len(), 1);
    }

    #[tokio::test]
    async fn the_two_failure_outcomes_read_differently() {
        // Given: the same store failure, repaired and not repaired.
        let repaired = Persisted::Undone {
            error: ItemError::NoSuchItem(9),
        };
        let not_repaired = Persisted::Diverged {
            error: ItemError::NoSuchItem(9),
            revoke: RevokeError::NotThere { id: 9 },
        };

        // When: both are written out.
        let good = repaired.to_string();
        let bad = not_repaired.to_string();

        // Then: they do not read the same. These two lines are the only record an
        // Operator has of whether a player is holding an item with no row, so text that
        // reads alike in both cases would make the second case invisible.
        assert_ne!(good, bad);
        assert!(
            !good.contains("world is now wrong"),
            "a repaired write must not read as a broken world: {good}"
        );
        assert!(bad.contains("world is now wrong"), "{bad}");
    }
}
