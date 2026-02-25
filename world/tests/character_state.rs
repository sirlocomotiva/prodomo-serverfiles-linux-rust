//! Character state-machine behavior tests.

use common::vid::Vid;
use world::character::{Activity, Character, CoreState, Posture};

#[test]
fn state_transitions_are_deferred_until_due_update() {
    // Given: a character with more than one queued transition before an update.
    let mut character = Character::new(Vid::new(1));
    character.request_state(CoreState::Idle);
    character.request_state(CoreState::Move);

    // When: the queued state has not yet been processed.
    let state_before_update = character.core_state();
    let pending_before_update = character.pending_state();
    let updated = character.update(0);

    // Then: the last request wins and is applied only by the due update.
    assert_eq!(state_before_update, CoreState::Initial);
    assert_eq!(pending_before_update, Some(CoreState::Move));
    assert!(updated);
    assert_eq!(character.core_state(), CoreState::Move);
    assert_eq!(character.pending_state(), None);
}

#[test]
fn dead_character_retains_pending_transition_and_deadline() {
    // Given: a due character whose transition was queued before death.
    let mut character = Character::new(Vid::new(2));
    character.request_state(CoreState::Battle);
    character.set_posture(Posture::Dead);

    // When: the due pulse is processed while dead and again after revival.
    let updated_while_dead = character.update(0);
    let pending_while_dead = character.pending_state();
    let deadline_while_dead = character.next_state_pulse();
    character.set_posture(Posture::Standing);
    let updated_after_revival = character.update(0);

    // Then: death advances neither the transition nor its deadline.
    assert!(!updated_while_dead);
    assert_eq!(pending_while_dead, Some(CoreState::Battle));
    assert_eq!(deadline_while_dead, 0);
    assert!(updated_after_revival);
    assert_eq!(character.core_state(), CoreState::Battle);
    assert_eq!(character.next_state_pulse(), 1);
}

#[test]
fn character_activity_changes_independently_from_state_and_posture() {
    // Given: a character with a queued core transition and unchanged deadline.
    let mut character = Character::new(Vid::new(4));
    character.request_state(CoreState::Battle);
    let initial_deadline = character.next_state_pulse();

    // When: each reserved activity seam is activated and then cleared.
    character.set_activity(Activity::Fishing);
    let while_fishing = character.activity();
    character.set_activity(Activity::Mining);
    let while_mining = character.activity();
    character.set_activity(Activity::None);

    // Then: activity never consumes or changes FSM, posture, or scheduling state.
    assert_eq!(while_fishing, Activity::Fishing);
    assert_eq!(while_mining, Activity::Mining);
    assert_eq!(character.activity(), Activity::None);
    assert_eq!(character.core_state(), CoreState::Initial);
    assert_eq!(character.pending_state(), Some(CoreState::Battle));
    assert_eq!(character.posture(), Posture::Standing);
    assert_eq!(character.next_state_pulse(), initial_deadline);
}
