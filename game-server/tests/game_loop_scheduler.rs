//! Deterministic tests for accumulated 25 Hz pulse deadlines.

use std::time::Duration;

use game_server::game_loop::{PulsePlanner, MAX_CATCH_UP_PULSES};

#[test]
fn first_pulse_is_due_only_after_40_milliseconds() {
    // Given: a new deterministic pulse planner.
    let mut planner = PulsePlanner::new();

    // When: elapsed time approaches and then reaches one pulse period.
    let before_deadline = planner.due_pulses(Duration::from_millis(39));
    let at_deadline = planner.due_pulses(Duration::from_millis(40));

    // Then: no immediate pulse occurs and the 40 ms deadline produces one pulse.
    assert_eq!(before_deadline.due, 0);
    assert!(!before_deadline.backlog_remaining);
    assert_eq!(at_deadline.due, 1);
    assert!(!at_deadline.backlog_remaining);
}

#[test]
fn accumulated_deadlines_cover_80_and_1000_milliseconds() {
    // Given: independent planners with the same 40 ms starting deadline.
    let mut at_80_ms = PulsePlanner::new();
    let mut at_1000_ms = PulsePlanner::new();

    // When: each planner observes an exact accumulated deadline.
    let first_two_pulses = at_80_ms.due_pulses(Duration::from_millis(80));
    let first_second = at_1000_ms.due_pulses(Duration::from_millis(1000));

    // Then: every elapsed 40 ms boundary is represented exactly once.
    assert_eq!(first_two_pulses.due, 2);
    assert!(!first_two_pulses.backlog_remaining);
    assert_eq!(first_second.due, 25);
    assert!(!first_second.backlog_remaining);
}

#[test]
fn late_processing_does_not_shift_the_next_deadline() {
    // Given: a planner first observed 20 ms after its second deadline.
    let mut planner = PulsePlanner::new();
    assert_eq!(planner.due_pulses(Duration::from_millis(100)).due, 2);

    // When: time advances to the original third deadline.
    let before_third_deadline = planner.due_pulses(Duration::from_millis(119));
    let at_third_deadline = planner.due_pulses(Duration::from_millis(120));

    // Then: processing delay did not move the accumulated 120 ms deadline.
    assert_eq!(before_third_deadline.due, 0);
    assert_eq!(at_third_deadline.due, 1);
}

#[test]
fn catch_up_is_bounded_to_30_seconds_per_batch_without_losing_backlog() {
    // Given: sixty seconds elapsed without planning a pulse batch.
    let mut planner = PulsePlanner::new();
    let elapsed = Duration::from_secs(60);

    // When: the same elapsed instant is planned repeatedly.
    let first_batch = planner.due_pulses(elapsed);
    let second_batch = planner.due_pulses(elapsed);
    let backlog_drained = planner.due_pulses(elapsed);

    // Then: each batch is capped at 750 and all 1500 pulses remain recoverable.
    assert_eq!(first_batch.due, MAX_CATCH_UP_PULSES);
    assert!(first_batch.backlog_remaining);
    assert_eq!(second_batch.due, MAX_CATCH_UP_PULSES);
    assert!(!second_batch.backlog_remaining);
    assert_eq!(backlog_drained.due, 0);
    assert!(!backlog_drained.backlog_remaining);
}

#[test]
fn exact_30_second_batch_is_not_reported_as_backlogged() {
    // Given: exactly one maximum catch-up window has elapsed.
    let mut planner = PulsePlanner::new();

    // When: the planner accounts for all 750 due pulses in one batch.
    let batch = planner.due_pulses(Duration::from_secs(30));

    // Then: the batch reaches the limit without claiming discarded backlog.
    assert_eq!(batch.due, MAX_CATCH_UP_PULSES);
    assert!(!batch.backlog_remaining);
}
