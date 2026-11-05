//! The event calendar `CEventManager` keeps (`G/event_manager.cpp`): the rows loaded once at
//! boot (`Initialize`, `:506-561`) and the list a client asks for by month
//! (`CInputMain::RequestEventData`, `G/input_main.cpp:5466-5475`, and `SendEventInfo`,
//! `G/event_manager.cpp:613-646`).
//!
//! The month is the start's `tm_mon` (0 for January) as `localtime` reads it; the server
//! clock runs in UTC, so the month is UTC's. The year is not compared, so an event of the same
//! month in another year is listed, and a December one is never "a previous month" in
//! January, both as legacy has it.

use common::vid::Vid;
use db::events::StoredEvent;
use protocol::gc_event::{encode_event_info, EventData};
use tokio::sync::oneshot;
use tracing::warn;

use super::costume::civil_from_days;
use super::GameState;

impl GameState {
    /// Install the stored calendar and tell the caller whether it took. A second install is
    /// refused, as a second guild install is.
    pub(super) fn answer_install_events(
        &mut self,
        events: Vec<StoredEvent>,
        reply: oneshot::Sender<bool>,
    ) {
        let installed = if self.events.is_some() {
            false
        } else {
            self.events = Some(events);
            true
        };
        if reply.send(installed).is_err() {
            warn!("the event calendar was installed but nobody was left to hear about it");
        }
    }

    /// `SendEventInfo(ch, month)` for the character online under `vid`: one `GC_EVENT_INFO`
    /// listing the events that start in `month`, and the uncompleted ones that start in an
    /// earlier month.
    pub(super) fn send_event_info(&mut self, vid: Vid, month: u8) {
        let listed = listed(self.events.as_deref().unwrap_or_default(), month);
        match encode_event_info(&listed) {
            Ok(record) => {
                let _sent = self.write_to_client(vid, record);
            }
            Err(error) => warn!(%error, "the event calendar is not sent"),
        }
    }
}

/// The 0-based month (`tm_mon`) of `stamp` seconds after the Unix epoch, in UTC.
fn start_month(stamp: i32) -> u32 {
    civil_from_days(i64::from(stamp).div_euclid(86_400)).1 - 1
}

/// The entries `SendEventInfo` writes for `month`, in the calendar's order.
fn listed(events: &[StoredEvent], month: u8) -> Vec<EventData> {
    let month = u32::from(month);
    events
        .iter()
        .filter(|event| {
            let start = start_month(event.start);
            (start < month && !event.completed) || start == month
        })
        .map(|event| EventData {
            id: event.id,
            event_type: event.event_type,
            start: event.start,
            end: event.end,
            value0: event.value0,
            value1: event.value1,
            completed: event.completed,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{listed, start_month, StoredEvent};

    /// 2026-03-01 00:00:00 UTC.
    const MARCH: i32 = 1_772_323_200;
    /// 2026-02-28 23:59:59 UTC.
    const FEBRUARY: i32 = MARCH - 1;

    fn event(id: u32, start: i32, completed: bool) -> StoredEvent {
        StoredEvent {
            id,
            event_type: 1,
            start,
            end: start + 3600,
            value0: 0,
            value1: 0,
            completed,
        }
    }

    /// The month turns at UTC midnight and counts from 0.
    #[test]
    fn the_month_is_utc_and_counts_from_zero() {
        assert_eq!(start_month(1), 0);
        assert_eq!(start_month(FEBRUARY), 1);
        assert_eq!(start_month(MARCH), 2);
    }

    /// The asked month lists every event that starts in it; an earlier month lists only the
    /// uncompleted ones; a later month lists none.
    #[test]
    fn a_month_lists_its_own_and_the_earlier_open_ones() {
        let events = [
            event(1, FEBRUARY, true),
            event(2, FEBRUARY, false),
            event(3, MARCH, true),
            event(4, MARCH + 31 * 86_400, false),
        ];
        let ids = |month| -> Vec<u32> {
            listed(&events, month)
                .iter()
                .map(|entry| entry.id)
                .collect()
        };
        assert_eq!(ids(2), [2, 3]);
        assert_eq!(ids(1), [1, 2]);
        assert_eq!(ids(0), Vec::<u32>::new());
        assert_eq!(ids(3), [2, 4]);
    }
}
