//! Verifies legacy-compatible event lifecycle behavior.

use std::{cell::RefCell, rc::Rc};

use world::event::{
    CancelOutcome, EventCallbackResult, EventQueue, Pulse, RescheduleDelay, ResetOutcome,
};

#[test]
fn creation_clamps_non_positive_delays_and_preserves_fifo() -> Result<(), Box<dyn std::error::Error>>
{
    let order = Rc::new(RefCell::new(Vec::new()));
    let mut queue = EventQueue::new();
    for (name, delay) in [("zero", 0), ("negative", -10)] {
        let observations = Rc::clone(&order);
        queue.schedule(Pulse::new(5), delay, move |_queue, context| {
            observations.borrow_mut().push((name, context.pulse()));
            EventCallbackResult::Complete
        })?;
    }

    assert_eq!(queue.process(Pulse::new(5)).callbacks_run(), 0);
    assert_eq!(queue.process(Pulse::new(6)).callbacks_run(), 2);
    assert_eq!(
        *order.borrow(),
        vec![("zero", Pulse::new(6)), ("negative", Pulse::new(6))]
    );
    Ok(())
}

#[test]
fn overdue_equal_deadlines_run_fifo_at_processing_pulse() -> Result<(), Box<dyn std::error::Error>>
{
    let order = Rc::new(RefCell::new(Vec::new()));
    let mut queue = EventQueue::new();
    for name in ["first", "second", "third"] {
        let observations = Rc::clone(&order);
        queue.schedule(Pulse::new(10), 5, move |_queue, context| {
            observations
                .borrow_mut()
                .push((name, context.pulse(), context.elapsed_pulses()));
            EventCallbackResult::Complete
        })?;
    }

    let report = queue.process(Pulse::new(30));

    assert_eq!(report.callbacks_run(), 3);
    assert_eq!(
        *order.borrow(),
        vec![
            ("first", Pulse::new(30), 20),
            ("second", Pulse::new(30), 20),
            ("third", Pulse::new(30), 20),
        ]
    );
    Ok(())
}

#[test]
fn cancellation_during_callback_overrides_reschedule() -> Result<(), Box<dyn std::error::Error>> {
    let outcomes = Rc::new(RefCell::new(Vec::new()));
    let callback_outcomes = Rc::clone(&outcomes);
    let delay = RescheduleDelay::try_new(1)?;
    let mut queue = EventQueue::new();
    queue.schedule(Pulse::new(0), 1, move |queue, context| {
        callback_outcomes
            .borrow_mut()
            .push(queue.cancel(context.id()));
        EventCallbackResult::RescheduleAfter(delay)
    })?;

    let first = queue.process(Pulse::new(1));
    let second = queue.process(Pulse::new(2));

    assert_eq!(
        *outcomes.borrow(),
        vec![CancelOutcome::CancellationRequested]
    );
    assert_eq!(first.callbacks_run(), 1);
    assert_eq!(first.events_rescheduled(), 0);
    assert_eq!(second.callbacks_run(), 0);
    assert_eq!(queue.active_count(), 0);
    Ok(())
}

#[test]
fn reset_during_callback_is_ignored_and_callback_result_wins(
) -> Result<(), Box<dyn std::error::Error>> {
    let outcomes = Rc::new(RefCell::new(Vec::new()));
    let callback_outcomes = Rc::clone(&outcomes);
    let delay = RescheduleDelay::try_new(2)?;
    let mut invocation = 0;
    let mut queue = EventQueue::new();
    queue.schedule(Pulse::new(0), 1, move |queue, context| {
        invocation += 1;
        callback_outcomes
            .borrow_mut()
            .push(queue.reset(context.id(), context.pulse(), 0));
        if invocation == 1 {
            EventCallbackResult::RescheduleAfter(delay)
        } else {
            EventCallbackResult::Complete
        }
    })?;

    let first = queue.process(Pulse::new(1));
    let before_due = queue.process(Pulse::new(2));
    let due = queue.process(Pulse::new(3));

    assert_eq!(first.events_rescheduled(), 1);
    assert_eq!(before_due.callbacks_run(), 0);
    assert_eq!(due.callbacks_run(), 1);
    assert_eq!(
        *outcomes.borrow(),
        vec![
            Ok(ResetOutcome::IgnoredWhileProcessing),
            Ok(ResetOutcome::IgnoredWhileProcessing),
        ]
    );
    Ok(())
}

#[test]
fn negative_reset_delay_runs_in_same_processing_pass() -> Result<(), Box<dyn std::error::Error>> {
    let fired = Rc::new(RefCell::new(Vec::new()));
    let target_fired = Rc::clone(&fired);
    let mut queue = EventQueue::new();
    let target = queue.schedule(Pulse::new(0), 20, move |_queue, context| {
        target_fired.borrow_mut().push(context.pulse());
        EventCallbackResult::Complete
    })?;
    queue.schedule(Pulse::new(0), 1, move |queue, context| {
        assert_eq!(
            queue.reset(target, context.pulse(), -5),
            Ok(ResetOutcome::Rescheduled)
        );
        EventCallbackResult::Complete
    })?;

    let report = queue.process(Pulse::new(1));

    assert_eq!(report.callbacks_run(), 2);
    assert_eq!(*fired.borrow(), vec![Pulse::new(1)]);
    Ok(())
}
