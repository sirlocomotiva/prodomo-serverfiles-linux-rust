//! The event calendar `CEventManager` keeps (`G/event_manager.cpp`): the rows loaded once at
//! boot (`Initialize`, `:506-561`), the start/end queues and upcoming announcements stepped once
//! per second (`Process`, `:342-456`), and the list a client asks for by month
//! (`CInputMain::RequestEventData`, `G/input_main.cpp:5466-5475`, and `SendEventInfo`,
//! `G/event_manager.cpp:613-646`).
//!
//! The month is the start's `tm_mon` (0 for January) as `localtime` reads it; the server
//! clock runs in UTC, so the month is UTC's. The year is not compared, so an event of the same
//! month in another year is listed, and a December one is never "a previous month" in
//! January, both as legacy has it.

use std::collections::VecDeque;

use common::vid::Vid;
use db::events::StoredEvent;
use protocol::gc_event::{encode_event_info, EventData};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use super::costume::civil_from_days;
use super::GameState;

/// The start and end queues `CEventManager` steps once per second (`G/event_manager.cpp:342-456`).
#[derive(Debug, Default)]
pub(super) struct EventSchedule {
    /// Upcoming unstarted event IDs (`m_dequeEventStart`), sorted by `start` ascending.
    start_queue: VecDeque<u32>,
    /// Active or upcoming event IDs (`m_dequeEventEnd`), sorted by `end` ascending.
    end_queue: VecDeque<u32>,
    /// Last wall-clock second for which upcoming-start notices were emitted.
    last_announce_sec: Option<i32>,
    /// Background channel that persists `UPDATE event SET completed = TRUE WHERE id = $1`.
    completed_tx: Option<mpsc::UnboundedSender<u32>>,
}

/// `CEventManager::GetEventName` (`G/event_manager.cpp:579-600`).
fn event_display_name(event_type: u8) -> Option<&'static str> {
    match event_type {
        1 => Some("Experience Event"),
        2 => Some("Item - Drop Boost Event"),
        3 => Some("Boss Event"),
        4 => Some("Metin Shower Event"),
        5 => Some("Mining Event"),
        6 => Some("Golden Frog Event"),
        7 => Some("Moonlight Chest Event"),
        8 => Some("Hexegonal Box Event"),
        9 => Some("Fishing Event"),
        10 => Some("Hide and Seek Event"),
        11 => Some("OX Event"),
        12 => Some("Tanka Event"),
        13 => Some("Siege War Event"),
        14 => Some("Kingdom War Event"),
        _ => None,
    }
}

/// The upcoming-start notice for `seconds_left` (`G/event_manager.cpp:368-398`), or `None`.
fn upcoming_notice(event_type: u8, seconds_left: i32) -> Option<Vec<u8>> {
    let suffix = match seconds_left {
        3600 => "1 hour",
        2700 => "45 minutes",
        1800 => "30 minutes",
        900 => "15 minutes",
        300 => "5 minutes",
        _ => return None,
    };
    let name = event_display_name(event_type)?;
    Some(format!("{name} will be starting in {suffix}.").into_bytes())
}

impl GameState {
    /// Install the stored calendar and tell the caller whether it took. A second install is
    /// refused, as a second guild install is.
    pub(super) fn answer_install_events(
        &mut self,
        events: Vec<StoredEvent>,
        completed_tx: Option<mpsc::UnboundedSender<u32>>,
        reply: oneshot::Sender<bool>,
    ) {
        let installed = if self.events.is_some() {
            false
        } else {
            self.events = Some(events);
            self.event_schedule.completed_tx = completed_tx;
            let now = i32::try_from(Self::global_time()).unwrap_or(i32::MAX);
            self.init_event_schedule(now);
            true
        };
        if reply.send(installed).is_err() {
            warn!("the event calendar was installed but nobody was left to hear about it");
        }
    }

    /// Populate `start_queue` and `end_queue` from `self.events` (`CEventManager::Initialize`,
    /// `G/event_manager.cpp:543-555`, and `Enqueue`, `:420-433`).
    fn init_event_schedule(&mut self, now: i32) {
        let mut to_start_now: Vec<StoredEvent> = Vec::new();
        let mut starts: Vec<(i32, u32)> = Vec::new();
        let mut ends: Vec<(i32, u32)> = Vec::new();
        if let Some(events) = self.events.as_deref() {
            for &event in events {
                if event.completed || event.start == 0 || event.end == 0 {
                    continue;
                }
                if event.start <= now {
                    to_start_now.push(event);
                } else {
                    starts.push((event.start, event.id));
                }
                ends.push((event.end, event.id));
            }
        }
        starts.sort_by_key(|&(t, _)| t);
        ends.sort_by_key(|&(t, _)| t);
        self.event_schedule.start_queue = starts.into_iter().map(|(_, id)| id).collect();
        self.event_schedule.end_queue = ends.into_iter().map(|(_, id)| id).collect();
        for event in to_start_now {
            let duration = event.end.saturating_sub(event.start);
            self.start_game_event_timed(
                i32::from(event.event_type),
                event.value0,
                event.value1,
                duration,
            );
        }
    }

    /// `if (!(pulse % passes_per_sec)) CEventManager::Instance().Process()` (`G/main.cpp:295-296`).
    pub(super) fn run_event_calendar(&mut self, pulse: u64) {
        if !pulse.is_multiple_of(u64::from(super::PASSES_PER_SEC)) {
            return;
        }
        if self.event_schedule.start_queue.is_empty() && self.event_schedule.end_queue.is_empty() {
            return;
        }
        let now = i32::try_from(Self::global_time()).unwrap_or(i32::MAX);
        self.step_event_calendar(now);
    }

    /// One `CEventManager::Process()` step at wall-clock `now` (`G/event_manager.cpp:342-413`):
    /// starts the front due event, broadcasts upcoming-start notices for the first five queued
    /// events, and ends the front due event while marking it completed.
    pub(super) fn step_event_calendar(&mut self, now: i32) {
        if let Some(&front_id) = self.event_schedule.start_queue.front() {
            if let Some(event) = self.stored_event(front_id) {
                if event.start <= now {
                    let _ = self.event_schedule.start_queue.pop_front();
                    let duration = event.end.saturating_sub(event.start);
                    self.start_game_event_timed(
                        i32::from(event.event_type),
                        event.value0,
                        event.value1,
                        duration,
                    );
                }
            } else {
                let _ = self.event_schedule.start_queue.pop_front();
            }
        }
        if self.event_schedule.last_announce_sec != Some(now) {
            self.event_schedule.last_announce_sec = Some(now);
            let notices: Vec<Vec<u8>> = self
                .event_schedule
                .start_queue
                .iter()
                .take(5)
                .filter_map(|&id| {
                    let event = self.stored_event(id)?;
                    upcoming_notice(event.event_type, event.start.saturating_sub(now))
                })
                .collect();
            for notice in notices {
                self.broadcast_notice_text(&notice);
            }
        }
        if let Some(&front_id) = self.event_schedule.end_queue.front() {
            if let Some(event) = self.stored_event(front_id) {
                if event.end <= now {
                    let _ = self.event_schedule.end_queue.pop_front();
                    self.end_game_event(i32::from(event.event_type));
                    if let Some(events) = self.events.as_mut() {
                        if let Some(row) = events.iter_mut().find(|row| row.id == front_id) {
                            row.completed = true;
                        }
                    }
                    if let Some(tx) = &self.event_schedule.completed_tx {
                        let _ = tx.send(front_id);
                    }
                }
            } else {
                let _ = self.event_schedule.end_queue.pop_front();
            }
        }
    }

    /// Look up one stored event row by `id`.
    fn stored_event(&self, id: u32) -> Option<StoredEvent> {
        self.events
            .as_deref()?
            .iter()
            .find(|event| event.id == id)
            .copied()
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

    /// `CEventManager::Process()` (`G/event_manager.cpp:342-456`): upcoming notice at 300 s,
    /// start at `start`, end at `end` with `completed = true` and completion channel notification.
    #[test]
    fn the_schedule_announces_starts_ends_and_marks_completed() {
        use crate::game_state::fixtures::{a_world, enter, TestClock};
        use tokio::sync::mpsc;
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(crate::game_state::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            gamedata::mob_locale_names::MobNamesByLanguage::default(),
        ));
        let mut inbox = enter(&mut world, 0x6001, (3200, 3200), 0);
        while inbox.try_recv().is_ok() {}
        let (tx, mut rx) = mpsc::unbounded_channel();
        world.events = Some(vec![StoredEvent {
            id: 42,
            event_type: 7, // EVENT_TYPE_MOONLIGHT
            start: 10_000,
            end: 10_600,
            value0: 0,
            value1: 0,
            completed: false,
        }]);
        world.event_schedule.completed_tx = Some(tx);
        world.init_event_schedule(9_000);
        // 5 minutes before start -> upcoming notice.
        world.step_event_calendar(9_700);
        let mut texts = Vec::new();
        while let Ok(rec) = inbox.try_recv() {
            if rec.first() == Some(&4) && rec.len() > 10 {
                texts.push(rec[10..].to_vec());
            }
        }
        assert!(
            texts
                .iter()
                .any(|t| t == b"Moonlight Chest Event will be starting in 5 minutes."),
            "5-minute warning: {texts:?}"
        );
        // At start -> flag set + start broadcast.
        world.step_event_calendar(10_000);
        assert_eq!(world.manager_event_flag(b"moon_drop"), 1);
        assert!(rx.try_recv().is_err());
        // At end -> flag cleared + completed=true + id sent for DB persistence.
        world.step_event_calendar(10_600);
        assert_eq!(world.manager_event_flag(b"moon_drop"), 0);
        assert!(world.events.as_ref().unwrap()[0].completed);
        assert_eq!(rx.try_recv().ok(), Some(42));
    }
}
