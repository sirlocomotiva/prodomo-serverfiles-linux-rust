//! Pulse-based event queue behavior tests.

use std::{cell::RefCell, rc::Rc};

use world::event::{
    CancelOutcome, EventCallbackResult, EventQueue, Pulse, RescheduleDelay, ResetOutcome,
};

#[test]
fn scheduled_event_fires_at_its_exact_deadline() -> Result<(), Box<dyn std::error::Error>> {
    // Given: an event scheduled five pulses after pulse ten.
    let fired_at = Rc::new(RefCell::new(Vec::new()));
    let callback_observations = Rc::clone(&fired_at);
    let mut queue = EventQueue::new();
    queue.schedule(Pulse::new(10), 5, move |_queue, context| {
        callback_observations.borrow_mut().push(context.pulse());
        EventCallbackResult::Complete
    })?;

    // When: the queue is processed immediately before and at the deadline.
    let before_deadline = queue.process(Pulse::new(14));
    let at_deadline = queue.process(Pulse::new(15));

    // Then: the callback fires once, exactly at pulse fifteen.
    assert_eq!(before_deadline.callbacks_run(), 0);
    assert_eq!(at_deadline.callbacks_run(), 1);
    assert_eq!(*fired_at.borrow(), vec![Pulse::new(15)]);
    Ok(())
}

#[test]
fn cancellation_before_due_leaves_a_safe_tombstone() -> Result<(), Box<dyn std::error::Error>> {
    // Given: one pending event whose callback must never run.
    let callbacks = Rc::new(RefCell::new(0));
    let callback_count = Rc::clone(&callbacks);
    let mut queue = EventQueue::new();
    let event = queue.schedule(Pulse::new(20), 5, move |_queue, _context| {
        *callback_count.borrow_mut() += 1;
        EventCallbackResult::Complete
    })?;

    // When: it is canceled before due and the original deadline is processed.
    let cancellation = queue.cancel(event);
    let queued_after_cancel = queue.queued_count();
    let active_after_cancel = queue.active_count();
    let report = queue.process(Pulse::new(25));

    // Then: cancellation is logical first and physical cleanup happens when due.
    assert_eq!(cancellation, CancelOutcome::Cancelled);
    assert_eq!(queued_after_cancel, 1);
    assert_eq!(active_after_cancel, 0);
    assert_eq!(report.callbacks_run(), 0);
    assert_eq!(report.tombstones_discarded(), 1);
    assert_eq!(*callbacks.borrow(), 0);
    Ok(())
}

#[test]
fn zero_delay_reset_runs_in_same_pass_without_stale_double_fire(
) -> Result<(), Box<dyn std::error::Error>> {
    // Given: a late event and an earlier event that will reset it.
    let observations = Rc::new(RefCell::new(Vec::new()));
    let target_observations = Rc::clone(&observations);
    let mut queue = EventQueue::new();
    let target = queue.schedule(Pulse::new(10), 20, move |_queue, context| {
        target_observations
            .borrow_mut()
            .push(("target", context.pulse()));
        EventCallbackResult::Complete
    })?;
    let reset_observations = Rc::clone(&observations);
    queue.schedule(Pulse::new(10), 1, move |queue, context| {
        let reset = queue.reset(target, context.pulse(), 0);
        reset_observations
            .borrow_mut()
            .push(("reset", context.pulse()));
        assert_eq!(reset, Ok(ResetOutcome::Rescheduled));
        EventCallbackResult::Complete
    })?;

    // When: the reset callback runs and the original target deadline later passes.
    let same_pass = queue.process(Pulse::new(11));
    let stale_deadline = queue.process(Pulse::new(30));

    // Then: the reset target runs in the same pass and the old node is only a tombstone.
    assert_eq!(same_pass.callbacks_run(), 2);
    assert_eq!(stale_deadline.callbacks_run(), 0);
    assert_eq!(stale_deadline.tombstones_discarded(), 1);
    assert_eq!(
        *observations.borrow(),
        vec![("reset", Pulse::new(11)), ("target", Pulse::new(11))]
    );
    Ok(())
}

#[test]
fn callback_reschedule_uses_actual_overdue_processing_pulse(
) -> Result<(), Box<dyn std::error::Error>> {
    // Given: an event due at one that requests three pulses after its first invocation.
    let fired_at = Rc::new(RefCell::new(Vec::new()));
    let callback_observations = Rc::clone(&fired_at);
    let reschedule_delay = RescheduleDelay::try_new(3)?;
    let mut invocation = 0;
    let mut queue = EventQueue::new();
    queue.schedule(Pulse::new(0), 1, move |_queue, context| {
        callback_observations.borrow_mut().push(context.pulse());
        invocation += 1;
        if invocation == 1 {
            EventCallbackResult::RescheduleAfter(reschedule_delay)
        } else {
            EventCallbackResult::Complete
        }
    })?;

    // When: the first call is overdue at ten and later passes cover twelve and thirteen.
    let overdue = queue.process(Pulse::new(10));
    let before_reschedule = queue.process(Pulse::new(12));
    let rescheduled = queue.process(Pulse::new(13));

    // Then: the second deadline is thirteen, based on the actual processing pulse ten.
    assert_eq!(overdue.callbacks_run(), 1);
    assert_eq!(overdue.events_rescheduled(), 1);
    assert_eq!(before_reschedule.callbacks_run(), 0);
    assert_eq!(rescheduled.callbacks_run(), 1);
    assert_eq!(*fired_at.borrow(), vec![Pulse::new(10), Pulse::new(13)]);
    Ok(())
}
