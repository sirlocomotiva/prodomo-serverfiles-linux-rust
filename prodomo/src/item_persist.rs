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

use db::items::{insert_item, ItemError, ItemRow};
use db::store::Store;

use crate::game_loop_messages::RevokeError;
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
