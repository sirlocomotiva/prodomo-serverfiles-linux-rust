use super::{EventCallback, EventQueue, EventState, QueueNode};
use crate::event::{
    EventCallbackResult, EventContext, ProcessFailure, ProcessReport, Pulse, RescheduleDelay,
};

struct CallbackReschedule {
    node: QueueNode,
    pulse: Pulse,
    delay: RescheduleDelay,
    callback: EventCallback,
}

impl EventQueue {
    /// Runs every event due at or before the supplied pulse.
    pub fn process(&mut self, pulse: Pulse) -> ProcessReport {
        let mut report = ProcessReport::default();
        while self.nodes.peek().is_some_and(|node| node.deadline <= pulse) {
            let Some(node) = self.nodes.pop() else {
                break;
            };
            if !self.prepare_callback(node, &mut report) {
                continue;
            }
            let callback = self
                .events
                .get_mut(&node.id)
                .and_then(|event| event.callback.take());
            let Some(mut callback) = callback else {
                continue;
            };
            let context = EventContext {
                id: node.id,
                pulse,
                elapsed_pulses: i128::from(pulse.raw()) - i128::from(node.start.raw()),
            };
            let callback_result = callback(self, context);
            let cancellation_requested = self.events.get(&node.id).is_some_and(|event| {
                matches!(
                    event.state,
                    EventState::Processing {
                        cancel_requested: true
                    }
                )
            });
            if cancellation_requested {
                self.events.remove(&node.id);
            } else {
                match callback_result {
                    EventCallbackResult::Complete => {
                        self.events.remove(&node.id);
                    }
                    EventCallbackResult::RescheduleAfter(delay) => self.apply_callback_reschedule(
                        CallbackReschedule {
                            node,
                            pulse,
                            delay,
                            callback,
                        },
                        &mut report,
                    ),
                }
            }
            report.callbacks_run += 1;
        }
        report
    }

    fn prepare_callback(&mut self, node: QueueNode, report: &mut ProcessReport) -> bool {
        let state = self.events.get(&node.id).map(|event| {
            if event.generation == node.generation {
                Some(event.state)
            } else {
                None
            }
        });
        match state {
            None | Some(None | Some(EventState::Processing { .. })) => {
                report.tombstones_discarded += 1;
                false
            }
            Some(Some(EventState::Cancelled)) => {
                self.events.remove(&node.id);
                report.tombstones_discarded += 1;
                false
            }
            Some(Some(EventState::Queued)) => {
                if let Some(event) = self.events.get_mut(&node.id) {
                    event.state = EventState::Processing {
                        cancel_requested: false,
                    };
                }
                true
            }
        }
    }

    fn apply_callback_reschedule(
        &mut self,
        reschedule: CallbackReschedule,
        report: &mut ProcessReport,
    ) {
        let CallbackReschedule {
            node,
            pulse,
            delay,
            callback,
        } = reschedule;
        let Some(deadline) = pulse.raw().checked_add(delay.raw()).map(Pulse::new) else {
            self.events.remove(&node.id);
            report.failures.push(ProcessFailure::DeadlineOverflow {
                id: node.id,
                pulse,
                delay,
            });
            return;
        };
        let Ok(sequence) = self.take_sequence() else {
            self.events.remove(&node.id);
            report
                .failures
                .push(ProcessFailure::SequenceExhausted { id: node.id });
            return;
        };
        if let Some(event) = self.events.get_mut(&node.id) {
            event.callback = Some(callback);
            event.state = EventState::Queued;
        }
        self.nodes.push(QueueNode {
            deadline,
            sequence,
            start: pulse,
            id: node.id,
            generation: node.generation,
        });
        report.events_rescheduled += 1;
    }
}
