//! Verifies typed event input and pulse-boundary failures.

use world::event::{
    EventCallbackResult, EventError, EventQueue, ProcessFailure, Pulse, RescheduleDelay,
};

#[test]
fn callback_reschedule_delay_must_be_positive() {
    assert_eq!(
        RescheduleDelay::try_new(0),
        Err(EventError::InvalidRescheduleDelay { delay: 0 })
    );
    assert_eq!(
        RescheduleDelay::try_new(-1),
        Err(EventError::InvalidRescheduleDelay { delay: -1 })
    );
}

#[test]
fn schedule_and_reset_report_deadline_overflow() -> Result<(), EventError> {
    let mut queue = EventQueue::new();
    let schedule = queue.schedule(Pulse::new(i64::MAX), 1, |_queue, _context| {
        EventCallbackResult::Complete
    });
    assert!(matches!(
        schedule,
        Err(EventError::DeadlineOverflow {
            pulse,
            delay: 1
        }) if pulse == Pulse::new(i64::MAX)
    ));

    let id = queue.schedule(Pulse::new(0), 1, |_queue, _context| {
        EventCallbackResult::Complete
    })?;
    let reset = queue.reset(id, Pulse::new(i64::MIN), -1);
    assert_eq!(
        reset,
        Err(EventError::DeadlineOverflow {
            pulse: Pulse::new(i64::MIN),
            delay: -1,
        })
    );
    Ok(())
}

#[test]
fn callback_deadline_overflow_completes_event_with_failure() -> Result<(), EventError> {
    let delay = RescheduleDelay::try_new(1)?;
    let mut queue = EventQueue::new();
    let id = queue.schedule(Pulse::new(i64::MAX - 1), 1, move |_queue, _context| {
        EventCallbackResult::RescheduleAfter(delay)
    })?;

    let report = queue.process(Pulse::new(i64::MAX));

    assert_eq!(report.callbacks_run(), 1);
    assert_eq!(report.events_rescheduled(), 0);
    assert_eq!(
        report.failures(),
        &[ProcessFailure::DeadlineOverflow {
            id,
            pulse: Pulse::new(i64::MAX),
            delay,
        }]
    );
    assert_eq!(queue.active_count(), 0);
    Ok(())
}
