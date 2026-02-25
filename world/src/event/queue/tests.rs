use super::*;
use crate::event::{EventCallbackResult, ProcessFailure, RescheduleDelay};

#[test]
fn identifier_exhaustion_is_typed() {
    let mut queue = EventQueue::new();
    queue.next_id = u64::MAX;

    let result = queue.schedule(Pulse::new(0), 1, |_queue, _context| {
        EventCallbackResult::Complete
    });

    assert!(matches!(result, Err(EventError::IdentifierExhausted)));
    assert_eq!(queue.queued_count(), 0);
}

#[test]
fn schedule_sequence_exhaustion_is_typed() {
    let mut queue = EventQueue::new();
    queue.next_sequence = u64::MAX;

    let result = queue.schedule(Pulse::new(0), 1, |_queue, _context| {
        EventCallbackResult::Complete
    });

    assert!(matches!(result, Err(EventError::SequenceExhausted)));
    assert_eq!(queue.queued_count(), 0);
}

#[test]
fn reset_generation_exhaustion_is_typed() -> Result<(), EventError> {
    let mut queue = EventQueue::new();
    let id = queue.schedule(Pulse::new(0), 1, |_queue, _context| {
        EventCallbackResult::Complete
    })?;
    if let Some(event) = queue.events.get_mut(&id) {
        event.generation = u64::MAX;
    }

    let result = queue.reset(id, Pulse::new(0), 1);

    assert_eq!(result, Err(EventError::GenerationExhausted { id }));
    assert_eq!(queue.queued_count(), 1);
    Ok(())
}

#[test]
fn callback_sequence_exhaustion_is_reported() -> Result<(), EventError> {
    let delay = RescheduleDelay::try_new(1)?;
    let mut queue = EventQueue::new();
    let id = queue.schedule(Pulse::new(0), 1, move |_queue, _context| {
        EventCallbackResult::RescheduleAfter(delay)
    })?;
    queue.next_sequence = u64::MAX;

    let report = queue.process(Pulse::new(1));

    assert_eq!(
        report.failures(),
        &[ProcessFailure::SequenceExhausted { id }]
    );
    assert_eq!(queue.active_count(), 0);
    Ok(())
}
