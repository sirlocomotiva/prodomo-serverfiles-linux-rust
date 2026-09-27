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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use gamedata::item_proto::ItemProtos;
use world::character::CharacterManager;
use world::item::{ItemIdRange, ItemIds};

use tracing::warn;

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

/// The world state one game thread owns.
#[derive(Debug)]
pub struct GameState {
    characters: CharacterManager,
    item_ids: ItemIds,
    protos: ItemProtos,
    metrics: Arc<GameStateMetrics>,
    last_pulse: u64,
}

impl GameState {
    /// Build a state from the Game data and the id range.
    ///
    /// `protos` is read before the thread starts, not inside it, so a missing or
    /// malformed proto file stops `serve` before any port opens. That ordering is
    /// the same one `load_atlas` already uses, and it is the Rewrite's own: legacy
    /// accepts clients before its DB boot finishes, which AGENTS.md records as a
    /// Defect.
    pub fn new(protos: ItemProtos, id_range: ItemIdRange) -> Self {
        Self {
            characters: CharacterManager::new(),
            item_ids: ItemIds::new(id_range),
            protos,
            metrics: Arc::new(GameStateMetrics::default()),
            last_pulse: 0,
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

    /// The item id allocator.
    pub fn item_ids(&self) -> &ItemIds {
        &self.item_ids
    }

    /// The item id allocator, mutably.
    pub fn item_ids_mut(&mut self) -> &mut ItemIds {
        &mut self.item_ids
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
            GameCommand::Stop => {
                // The loop handles `Stop` itself, before a command ever reaches a
                // processor. Reaching here would mean the loop and the state
                // disagree about who owns shutdown, and quietly ignoring it would
                // hide exactly that.
                debug_assert!(false, "the game loop must handle Stop itself");
            }
        }
    }

    /// Run one grant against the world, taking the target's own `Inven_Point`.
    fn grant(&mut self, request: &GrantRequest) -> Result<GrantOutcome, GrantRefusal> {
        let inven_point = self
            .characters
            .find_player_mut(&request.target)
            .map_or(0, |character| character.inven_point());
        grant_item(
            &mut self.characters,
            &self.protos,
            &mut self.item_ids,
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
    use world::character::Lookup;

    fn owners() -> ItemProtos {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        ItemProtos::load(&dir).expect("the owner's item protos load")
    }

    fn a_range() -> ItemIdRange {
        ItemIdRange::new(1, 1_000_000, 1).expect("a range that can issue an id")
    }

    fn a_state() -> GameState {
        GameState::new(owners(), a_range())
    }

    #[test]
    fn a_new_state_starts_with_no_characters_and_an_unissued_allocator() {
        let state = a_state();
        assert_eq!(state.characters().len(), 0);
        assert_eq!(state.item_ids().issued(), 0);
        assert_eq!(state.protos().len(), 7_305);
        assert_eq!(state.last_pulse(), 0);
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
