//! Deterministic accumulated-deadline pulse planning.

use std::time::Duration;

/// Fixed 25 Hz pulse period.
pub const PULSE_PERIOD: Duration = Duration::from_millis(40);

/// Maximum pulses processed in one catch-up batch, equivalent to 30 seconds.
pub const MAX_CATCH_UP_PULSES: u16 = 750;

/// One bounded set of due pulses and whether more deadlines remain due.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PulseBatch {
    /// Number of pulses to process in this batch.
    pub due: u16,
    /// Whether the same elapsed instant still contains unprocessed deadlines.
    pub backlog_remaining: bool,
}

/// Separates deterministic pulse deadlines from wall-clock scheduling.
#[derive(Debug)]
pub struct PulsePlanner {
    next_deadline: Duration,
}

impl PulsePlanner {
    /// Creates a planner whose first pulse is due after one full period.
    pub const fn new() -> Self {
        Self {
            next_deadline: PULSE_PERIOD,
        }
    }

    /// Advances accumulated deadlines and reports one bounded batch.
    pub fn due_pulses(&mut self, elapsed: Duration) -> PulseBatch {
        let mut due = 0;

        while elapsed >= self.next_deadline && due < MAX_CATCH_UP_PULSES {
            due += 1;
            self.next_deadline = self.next_deadline.saturating_add(PULSE_PERIOD);
        }

        PulseBatch {
            due,
            backlog_remaining: elapsed >= self.next_deadline,
        }
    }

    /// Returns how long the wall-clock scheduler should park before rechecking.
    pub fn time_until_next(&self, elapsed: Duration) -> Duration {
        self.next_deadline.saturating_sub(elapsed)
    }
}

impl Default for PulsePlanner {
    fn default() -> Self {
        Self::new()
    }
}
