//! Character and manager pulse-update behavior tests.

use common::vid::Vid;
use world::character::{Character, CharacterManager, CoreState};

#[test]
fn not_due_update_is_inert_and_due_update_schedules_from_supplied_pulse() {
    // Given: an initialized character with a three-pulse state duration.
    let mut character = Character::new(Vid::new(3));
    character.set_state_duration(3);
    assert!(character.update(0));
    character.request_state(CoreState::Move);

    // When: one pulse is early and a later supplied pulse is due.
    let early_update = character.update(2);
    let state_after_early_update = character.core_state();
    let pending_after_early_update = character.pending_state();
    let deadline_after_early_update = character.next_state_pulse();
    let late_update = character.update(5);

    // Then: only the due call mutates state and it schedules from pulse five.
    assert!(!early_update);
    assert_eq!(state_after_early_update, CoreState::Idle);
    assert_eq!(pending_after_early_update, Some(CoreState::Move));
    assert_eq!(deadline_after_early_update, 3);
    assert!(late_update);
    assert_eq!(character.core_state(), CoreState::Move);
    assert_eq!(character.next_state_pulse(), 8);
}

#[test]
fn manager_processes_exactly_twenty_five_supplied_pulses() {
    // Given: one active character with the legacy one-pulse duration.
    let mut manager = CharacterManager::new();
    let vid = manager.create_player(7, "Pulse").unwrap();

    // When: the caller supplies exactly twenty-five consecutive pulses.
    let processed = (0..25)
        .map(|pulse| manager.process_pulse(pulse).updated())
        .sum::<usize>();

    // Then: each supplied pulse performs one update and schedules the next.
    assert_eq!(processed, 25);
    assert_eq!(manager.find_by_vid(vid).unwrap().next_state_pulse(), 25);
}
