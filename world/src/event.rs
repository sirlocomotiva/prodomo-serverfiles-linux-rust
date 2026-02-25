//! Deterministic synchronous events driven by caller-supplied pulses.

use std::{error::Error, fmt};

mod queue;

pub use queue::EventQueue;

/// Absolute game-loop pulse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pulse(i64);

impl Pulse {
    /// Creates an absolute pulse value.
    #[must_use]
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Returns the underlying integer pulse.
    #[must_use]
    pub const fn raw(self) -> i64 {
        self.0
    }
}

/// Stable identity assigned to a scheduled event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventId(u64);

/// Values available to an event callback while it is processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventContext {
    pub(super) id: EventId,
    pub(super) pulse: Pulse,
    pub(super) elapsed_pulses: i128,
}

impl EventContext {
    /// Returns the event being processed.
    #[must_use]
    pub const fn id(self) -> EventId {
        self.id
    }

    /// Returns the pulse supplied to the current processing pass.
    #[must_use]
    pub const fn pulse(self) -> Pulse {
        self.pulse
    }

    /// Returns elapsed pulses since this event was most recently queued.
    #[must_use]
    pub const fn elapsed_pulses(self) -> i128 {
        self.elapsed_pulses
    }
}

/// Instruction returned by an event callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventCallbackResult {
    /// Finish the event without scheduling another invocation.
    Complete,
    /// Queue another invocation after a positive number of pulses.
    RescheduleAfter(RescheduleDelay),
}

/// Positive callback-result delay that cannot represent legacy termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RescheduleDelay(i64);

impl RescheduleDelay {
    /// Parses a strictly positive callback reschedule delay.
    ///
    /// # Errors
    /// Returns [`EventError::InvalidRescheduleDelay`] for zero or negative input.
    pub const fn try_new(pulses: i64) -> Result<Self, EventError> {
        if pulses > 0 {
            Ok(Self(pulses))
        } else {
            Err(EventError::InvalidRescheduleDelay { delay: pulses })
        }
    }

    pub(super) const fn raw(self) -> i64 {
        self.0
    }
}

/// Result of requesting cancellation by event identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// A pending event became a queued tombstone.
    Cancelled,
    /// A running event was marked to finish after its callback returns.
    CancellationRequested,
    /// The event was already canceled.
    AlreadyCancelled,
    /// No event has the supplied identity.
    NotFound,
}

/// Result of requesting a new deadline by event identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetOutcome {
    /// The event was queued at the new deadline.
    Rescheduled,
    /// A running event ignored reset so its callback result remains authoritative.
    IgnoredWhileProcessing,
    /// The event does not exist or is already canceled.
    NotFound,
}

/// Failure to schedule an event safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventError {
    /// Adding a delay to the supplied pulse exceeds the pulse range.
    DeadlineOverflow {
        /// Pulse used as the scheduling base.
        pulse: Pulse,
        /// Delay that could not be represented.
        delay: i64,
    },
    /// No further event identity can be represented.
    IdentifierExhausted,
    /// No further stable queue position can be represented.
    SequenceExhausted,
    /// An event cannot invalidate any more stale queue entries.
    GenerationExhausted {
        /// Event whose generation is exhausted.
        id: EventId,
    },
    /// A callback reschedule delay was not strictly positive.
    InvalidRescheduleDelay {
        /// Rejected delay.
        delay: i64,
    },
}

impl fmt::Display for EventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeadlineOverflow { pulse, delay } => write!(
                formatter,
                "event deadline overflows from pulse {} with delay {delay}",
                pulse.raw()
            ),
            Self::IdentifierExhausted => formatter.write_str("event identifiers are exhausted"),
            Self::SequenceExhausted => formatter.write_str("event queue sequence is exhausted"),
            Self::GenerationExhausted { id } => {
                write!(formatter, "event {id:?} generation is exhausted")
            }
            Self::InvalidRescheduleDelay { delay } => {
                write!(
                    formatter,
                    "callback reschedule delay must be positive, got {delay}"
                )
            }
        }
    }
}

impl Error for EventError {}

/// Observable work performed by one processing pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessReport {
    pub(super) callbacks_run: usize,
    pub(super) tombstones_discarded: usize,
    pub(super) events_rescheduled: usize,
    pub(super) failures: Vec<ProcessFailure>,
}

impl ProcessReport {
    /// Returns the number of callbacks invoked by this pass.
    #[must_use]
    pub const fn callbacks_run(&self) -> usize {
        self.callbacks_run
    }

    /// Returns the number of canceled or stale queue entries discarded.
    #[must_use]
    pub const fn tombstones_discarded(&self) -> usize {
        self.tombstones_discarded
    }

    /// Returns the number of callbacks successfully requeued.
    #[must_use]
    pub const fn events_rescheduled(&self) -> usize {
        self.events_rescheduled
    }

    /// Returns callback reschedules that could not be represented safely.
    #[must_use]
    pub fn failures(&self) -> &[ProcessFailure] {
        &self.failures
    }
}

/// Failure produced while applying a callback result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessFailure {
    /// The callback's next deadline exceeded the pulse range.
    DeadlineOverflow {
        /// Event that could not be requeued.
        id: EventId,
        /// Actual pulse used as the reschedule base.
        pulse: Pulse,
        /// Positive callback delay.
        delay: RescheduleDelay,
    },
    /// The stable queue sequence was exhausted.
    SequenceExhausted {
        /// Event that could not be requeued.
        id: EventId,
    },
}
