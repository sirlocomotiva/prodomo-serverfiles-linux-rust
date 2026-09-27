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

use crate::game_loop::PulseProcessor;

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

impl PulseProcessor for GameState {
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
