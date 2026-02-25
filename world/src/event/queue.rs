use std::{
    cmp::Ordering,
    collections::{BTreeMap, BinaryHeap},
};

use super::{
    CancelOutcome, EventCallbackResult, EventContext, EventError, EventId, Pulse, ResetOutcome,
};

mod process;
#[cfg(test)]
mod tests;

type EventCallback = Box<dyn FnMut(&mut EventQueue, EventContext) -> EventCallbackResult>;

struct EventRecord {
    callback: Option<EventCallback>,
    state: EventState,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventState {
    Queued,
    Processing { cancel_requested: bool },
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueNode {
    deadline: Pulse,
    sequence: u64,
    start: Pulse,
    id: EventId,
    generation: u64,
}

impl Ord for QueueNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .deadline
            .cmp(&self.deadline)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl PartialOrd for QueueNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Synchronous pulse queue with stable ordering for equal deadlines.
#[derive(Default)]
pub struct EventQueue {
    nodes: BinaryHeap<QueueNode>,
    events: BTreeMap<EventId, EventRecord>,
    next_id: u64,
    next_sequence: u64,
}

impl EventQueue {
    /// Creates an empty event queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Schedules a callback relative to the supplied pulse.
    ///
    /// Delays below one pulse are clamped to one, matching legacy creation.
    ///
    /// # Errors
    /// Returns [`EventError`] when the deadline or internal monotonic counters
    /// cannot be represented.
    pub fn schedule<F>(
        &mut self,
        current: Pulse,
        delay: i64,
        callback: F,
    ) -> Result<EventId, EventError>
    where
        F: FnMut(&mut EventQueue, EventContext) -> EventCallbackResult + 'static,
    {
        let delay = delay.max(1);
        let deadline = current.raw().checked_add(delay).map(Pulse::new).ok_or(
            EventError::DeadlineOverflow {
                pulse: current,
                delay,
            },
        )?;
        let id = EventId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(EventError::IdentifierExhausted)?;
        let sequence = self.take_sequence()?;
        self.events.insert(
            id,
            EventRecord {
                callback: Some(Box::new(callback)),
                state: EventState::Queued,
                generation: 0,
            },
        );
        self.nodes.push(QueueNode {
            deadline,
            sequence,
            start: current,
            id,
            generation: 0,
        });
        Ok(id)
    }

    /// Cancels an event without removing its queued node.
    pub fn cancel(&mut self, id: EventId) -> CancelOutcome {
        let Some(event) = self.events.get_mut(&id) else {
            return CancelOutcome::NotFound;
        };
        match event.state {
            EventState::Queued => {
                event.state = EventState::Cancelled;
                CancelOutcome::Cancelled
            }
            EventState::Processing { .. } => {
                event.state = EventState::Processing {
                    cancel_requested: true,
                };
                CancelOutcome::CancellationRequested
            }
            EventState::Cancelled => CancelOutcome::AlreadyCancelled,
        }
    }

    /// Requeues a pending event relative to the supplied pulse without clamping.
    ///
    /// # Errors
    /// Returns [`EventError`] when the new deadline or monotonic queue metadata
    /// cannot be represented.
    pub fn reset(
        &mut self,
        id: EventId,
        current: Pulse,
        delay: i64,
    ) -> Result<ResetOutcome, EventError> {
        let Some(event) = self.events.get(&id) else {
            return Ok(ResetOutcome::NotFound);
        };
        match event.state {
            EventState::Cancelled => return Ok(ResetOutcome::NotFound),
            EventState::Processing { .. } => return Ok(ResetOutcome::IgnoredWhileProcessing),
            EventState::Queued => {}
        }
        let deadline = current.raw().checked_add(delay).map(Pulse::new).ok_or(
            EventError::DeadlineOverflow {
                pulse: current,
                delay,
            },
        )?;
        let generation = event
            .generation
            .checked_add(1)
            .ok_or(EventError::GenerationExhausted { id })?;
        let sequence = self.take_sequence()?;
        if let Some(event) = self.events.get_mut(&id) {
            event.generation = generation;
        }
        self.nodes.push(QueueNode {
            deadline,
            sequence,
            start: current,
            id,
            generation,
        });
        Ok(ResetOutcome::Rescheduled)
    }

    /// Returns the number of physical entries retained by the heap.
    #[must_use]
    pub fn queued_count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns the number of events that can still invoke a callback.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.events
            .values()
            .filter(|event| {
                matches!(
                    event.state,
                    EventState::Queued | EventState::Processing { .. }
                )
            })
            .count()
    }

    fn take_sequence(&mut self) -> Result<u64, EventError> {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(EventError::SequenceExhausted)?;
        Ok(sequence)
    }
}
