//! The OX event's engine (`CEventManager::SetOXEvent` and `ox_event_process`,
//! `G/event_manager.cpp:1369-1525`, and `COXEventManager`, `G/OXEvent.cpp`): the quiz
//! the panel starts, its rounds, its answer checks and its audience warps.
//!
//! The panel's click starts (`start_game_event` 11) and ends (`end_game_event` 11)
//! the event; this module holds the pieces they call. The start refuses while the
//! status flag holds or the process runs, clears the quiz list, loads `oxquiz.lua`
//! beside the quests, chats the start, arms the process in its open state with five
//! registration minutes and three rounds, and opens the status. The process chats
//! the countdown each minute, closes the entries and spends a round, quizzes every
//! 35 s while more than five attenders stand, pays the 50109 reward and chats the
//! teleport line when five or fewer stand, and between rounds clears the map and
//! opens again. The end chats, warps the roster home, finishes the status and
//! disarms the process.
//!
//! A quiz draws one question of level 1, chats the header, the question and the
//! footer, and arms the timer with the answer: 15 s later the ask chats, 10 s
//! after that the check runs — a wrong foot or no foot in the answer's half fails
//! the attender with `SE_FAIL` and drops it to the miss list, a right foot hears
//! `[LS;980]`, cheers and succeeds — then 5 s later the misses warp to the audience
//! and the status closes. Without level 1 or
//! with no quiz at all the quiz phase errors, warps the roster home and ends the
//! event.
//!
//! The miss warp re-stands the body where `Show` puts it; an offline miss and a
//! roster entry offline at the close warp nowhere, as `FindByPID` answers NULL.
//! The timer's `static BYTE flag` is the armed step, and the process event's
//! `BYTE` counters wrap at 255 as legacy's do.

use common::vid::Vid;
use protocol::gc_chat::{
    GcChat, CHAT_TYPE_BIG_NOTICE, CHAT_TYPE_COMMAND, CHAT_TYPE_INFO, CHAT_TYPE_NOTICE,
};
use protocol::gc_vid::GcSpecialEffect;

use super::GameState;
use crate::chat_line::Arg;

/// The OX map (`OXEVENT_MAP_INDEX`).
pub(super) const OX_MAP: i32 = 113;
/// `OX_START_WAITING_TIME`: the registration minutes each round opens with.
const WAITING_MINUTES: u8 = 5;
/// `OX_ROUND_COUNT`: the rounds a start carries.
const ROUND_COUNT: u8 = 3;
/// `OX_PLAYER_WIN_COUNT`: five or fewer attenders end the quizzing.
const WIN_COUNT: usize = 5;
/// `OX_REWARD_VNUM` and `OX_REWARD_COUNT`: the winners' reward.
const REWARD_VNUM: u32 = 50109;
/// The quizzed level (`ox_event_process` quizzes 1).
const QUIZ_LEVEL: u8 = 1;
/// `SE_SUCCESS` (`C/length.h:862`).
const SE_SUCCESS: u8 = 12;
/// `SE_FAIL` (`C/length.h:863`).
const SE_FAIL: u8 = 13;
/// The O answer's half (`CheckAnswer`'s `rect` for false).
const O_RECT: [i32; 4] = [892_600, 22_900, 896_300, 26_400];
/// The X answer's half (`CheckAnswer`'s `rect` for true).
const X_RECT: [i32; 4] = [896_600, 22_900, 900_300, 26_400];
/// The audience spots (`WarpToAudience`'s four `Show` points).
const AUDIENCE_SPOTS: [(i32, i32); 4] = [
    (896_300, 28_900),
    (890_900, 28_100),
    (896_600, 20_500),
    (902_500, 28_100),
];
/// The process's open state (`OXEVENT_OPEN`).
const OPEN: u8 = 1;
/// The process's quiz state (`OXEVENT_CLOSE`).
const CLOSED: u8 = 2;
/// The process's end state (`OXEVENT_FINISH`).
const FINISHED: u8 = 0;

/// The OX process and timer: `m_pOXEvent` and `m_timedEvent` as Pulses, with the
/// process's `BYTE` state, minutes and rounds, the timer's answer and step, and
/// the misses `CheckAnswer` drops.
#[derive(Debug, Default)]
pub(super) struct OxEvent {
    /// The process's due Pulse, `None` with no event running.
    due: Option<u64>,
    /// The process's state (`OXEVENT_OPEN`, `OXEVENT_CLOSE`, `OXEVENT_FINISH`).
    state: u8,
    /// The registration minutes left.
    minutes: u8,
    /// The rounds left.
    rounds: u8,
    /// The quiz timer's answer, step and due Pulse, `None` with none armed.
    timer: Option<(bool, u8, u64)>,
    /// The misses (`m_map_miss`): the PIDs the last check failed, by PID.
    misses: std::collections::BTreeSet<u32>,
}

fn pulses(secs: u64) -> u64 {
    secs.saturating_mul(u64::from(crate::save::PASSES_PER_SEC))
}

impl GameState {
    /// `SetOXEvent(true)` (`:1482-1513`): refused while the status holds or the
    /// process runs. Else the quiz list clears, `oxquiz.lua` loads beside the
    /// quests, the start chats, the process arms open with five minutes and three
    /// rounds, and the status opens. A quiz file that fails to load ends the start
    /// with the list cleared, as legacy's `return false` does after `ClearQuiz`.
    pub(super) fn start_ox_event(&mut self) {
        if self.event_rate(super::gm_lines::OXEVENT_STATUS) != 0 || self.ox_event.due.is_some() {
            return;
        }
        self.ox_quiz.clear();
        let path = self.ox_quiz_path();
        let Ok(calls) = quest::ox_quiz::load(&path) else {
            return;
        };
        for call in calls {
            self.land_ox_quiz(call.level, &call.question, call.answer);
        }
        self.broadcast_notice_text(b"OX Event will be starting soon, prepare yourselves.");
        self.set_event_flag(super::gm_lines::OXEVENT_STATUS, 1);
        self.ox_event.due = Some(self.last_pulse.saturating_add(pulses(60)));
        self.ox_event.state = OPEN;
        self.ox_event.minutes = WAITING_MINUTES;
        self.ox_event.rounds = ROUND_COUNT;
        self.set_event_flag(super::gm_lines::OXEVENT_STATUS, i32::from(OPEN));
    }

    /// `SetOXEvent(false)` (`:1514-1525`): the end chats, the flag and the status
    /// finish, the roster warps home and the process disarms.
    pub(super) fn end_ox_event(&mut self) {
        self.broadcast_notice_text(b"OX Event has ended. Thank you for participating!");
        self.set_event_flag(super::gm_lines::OXEVENT_STATUS, 0);
        self.set_event_flag(super::gm_lines::OXEVENT_STATUS, i32::from(FINISHED));
        self.close_ox_event();
        self.ox_event.due = None;
    }

    /// The quiz file beside the quests: `oxquiz.lua` under the locale directory.
    fn ox_quiz_path(&self) -> std::path::PathBuf {
        self.locale_dir.join("oxquiz.lua")
    }

    /// `CloseEvent` (`G/OXEvent.cpp:418-436`): the timer disarms and every roster
    /// entry online warps to its empire start; then the roster clears. The
    /// attenders stay seated: only the roster clears.
    fn close_ox_event(&mut self) {
        self.ox_event.timer = None;
        let mut pids: Vec<u32> = self.ox_roster.iter().copied().collect();
        pids.sort_unstable();
        for pid in pids {
            let Some(vid) = self.vid_of_pid(pid) else {
                continue;
            };
            let Some(body) = self.bodies.get(&vid) else {
                continue;
            };
            if let Some(point) = crate::warp::go_home(body.card.empire) {
                self.order_warp(vid, point);
            }
        }
        self.ox_roster.clear();
    }

    /// `ox_event_process` (`:1376-1478`) and `oxevent_timer` (`G/OXEvent.cpp:226-272`)
    /// on `pulse`: the due process arm runs, then the due timer arm. The `BYTE`
    /// counters wrap, as legacy's do.
    pub(super) fn run_ox_event(&mut self, pulse: u64) {
        if self.ox_event.due.is_some_and(|due| pulse >= due) {
            self.ox_event.due = None;
            self.fire_ox_process(pulse);
        }
        if let Some((_, _, due)) = self.ox_event.timer {
            if pulse >= due {
                self.fire_ox_timer(pulse);
            }
        }
    }

    /// One `ox_event_process` fire: the open state counts a minute down and chats,
    /// closing and spending a round at zero; the quiz state pays the winners or
    /// quizzes; the end state opens the next round or ends the event.
    fn fire_ox_process(&mut self, pulse: u64) {
        match self.ox_event.state {
            OPEN => {
                self.ox_event.minutes = self.ox_event.minutes.wrapping_sub(1);
                if self.ox_event.minutes != 0 {
                    let mut line = b"OX Event will be starting in ".to_vec();
                    line.extend_from_slice(self.ox_event.minutes.to_string().as_bytes());
                    line.extend_from_slice(b" minute(s).");
                    self.broadcast_notice_text(&line);
                    self.ox_event.due = Some(pulse.saturating_add(pulses(60)));
                } else {
                    self.ox_event.state = CLOSED;
                    self.ox_event.rounds = self.ox_event.rounds.wrapping_sub(1);
                    self.set_event_flag(super::gm_lines::OXEVENT_STATUS, i32::from(CLOSED));
                    self.ox_event.due = Some(pulse.saturating_add(pulses(15)));
                }
            }
            CLOSED => {
                if self.ox_attenders.len() <= WIN_COUNT {
                    for pid in self.ox_attenders.clone() {
                        self.ox_reward(pid);
                    }
                    self.ox_event.state = FINISHED;
                    self.tell_ox_map(b"Participants will now be teleported to the city.", true);
                    self.ox_event.due = Some(pulse.saturating_add(pulses(15)));
                } else if self.ox_quiz() {
                    self.ox_event.due = Some(pulse.saturating_add(pulses(35)));
                } else {
                    self.close_ox_event();
                    self.end_ox_event();
                }
            }
            _ => {
                if self.ox_event.rounds != 0 {
                    self.broadcast_notice_text(b"The next round of OX will start again shortly.");
                    self.close_ox_event();
                    self.ox_event.state = OPEN;
                    self.ox_event.minutes = WAITING_MINUTES;
                    self.set_event_flag(super::gm_lines::OXEVENT_STATUS, i32::from(OPEN));
                    self.ox_event.due = Some(pulse.saturating_add(pulses(60)));
                } else {
                    self.end_ox_event();
                }
            }
        }
    }

    /// `Quiz(1, 30)` (`G/OXEvent.cpp:274-301`): one drawn question of level 1 chats
    /// its header, text and footer, the old timer disarms, the answer arms the new
    /// timer 15 s out, the status quizzes and the question leaves the list. False
    /// with no list, no level 1, or no question left there.
    fn ox_quiz(&mut self) -> bool {
        if self.ox_quiz.is_empty() {
            return false;
        }
        let top = self.ox_quiz.len().saturating_sub(1);
        let slot = usize::from(QUIZ_LEVEL).min(top);
        if self.ox_quiz[slot].is_empty() {
            return false;
        }
        let last = self.ox_quiz[slot].len() - 1;
        let Ok(wide) = i32::try_from(last) else {
            return false;
        };
        let at = world::character::number(&mut self.dice, 0, wide);
        let at = usize::try_from(at).unwrap_or(0).min(last);
        let quiz = self.ox_quiz[slot][at].clone();
        let end = quiz
            .question
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(quiz.question.len());
        self.tell_ox_map(b"[LS;958]", true);
        self.tell_ox_map(&quiz.question[..end], true);
        self.tell_ox_map(b"[LS;969]", true);
        self.ox_event.timer = None;
        let due = self.last_pulse.saturating_add(pulses(15));
        self.ox_event.timer = Some((quiz.answer, 0, due));
        self.set_event_flag(super::gm_lines::OXEVENT_STATUS, 3);
        self.ox_quiz[slot].remove(at);
        true
    }

    /// One `oxevent_timer` fire: step 0 chats `[LS;892]` and waits 10 s; step 1
    /// checks the answer, chats the result and waits 5 s; step 2 warps the misses
    /// to the audience, closes the status and disarms.
    fn fire_ox_timer(&mut self, pulse: u64) {
        let Some((answer, step, _)) = self.ox_event.timer else {
            return;
        };
        match step {
            0 => {
                self.tell_ox_map(b"[LS;892]", true);
                self.ox_event.timer = Some((answer, 1, pulse.saturating_add(pulses(10))));
            }
            1 => {
                self.tell_ox_map(b"[LS;903]", true);
                self.check_ox_answer(answer);
                self.tell_ox_map(if answer { b"[LS;914]" } else { b"[LS;925]" }, true);
                self.tell_ox_map(b"[LS;936]", true);
                self.ox_event.timer = Some((answer, 2, pulse.saturating_add(pulses(5))));
            }
            _ => {
                self.warp_ox_misses();
                self.set_event_flag(super::gm_lines::OXEVENT_STATUS, i32::from(CLOSED));
                self.tell_ox_map(b"[LS;947]", true);
                self.ox_event.timer = None;
            }
        }
    }

    /// `CheckAnswer` (`G/OXEvent.cpp:303-412`): every attender online outside the
    /// answer's half fails with `SE_FAIL`, leaves the attenders and joins the
    /// misses; every attender inside hears `[LS;980]`, cheers and succeeds with
    /// `SE_SUCCESS`. An attender offline drops from the attenders, the roster and
    /// the misses. The misses clear first.
    fn check_ox_answer(&mut self, answer: bool) {
        self.ox_misses().clear();
        let rect = if answer { X_RECT } else { O_RECT };
        let mut pids: Vec<u32> = self.ox_attenders.iter().copied().collect();
        pids.sort_unstable();
        for pid in pids {
            let Some(vid) = self.vid_of_pid(pid) else {
                self.ox_attenders.remove(&pid);
                self.ox_roster.remove(&pid);
                self.ox_misses().remove(&pid);
                continue;
            };
            let inside = self.kept_place(vid).is_some_and(|(_, x, y)| {
                x >= rect[0] && x <= rect[2] && y >= rect[1] && y <= rect[3]
            });
            if inside {
                if let Some(mover) = self.own_mover(vid) {
                    self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;980]", &[]);
                }
                self.send_ox_cheer(vid);
                self.send_ox_effect(vid, SE_SUCCESS);
            } else {
                self.send_ox_effect(vid, SE_FAIL);
                self.ox_attenders.remove(&pid);
                self.ox_misses().insert(pid);
            }
        }
    }

    /// `WarpToAudience` (`G/OXEvent.cpp:414-416`): every miss online re-stands on
    /// one of the four audience spots; then the misses clear.
    fn warp_ox_misses(&mut self) {
        let mut pids: Vec<u32> = self.ox_misses().iter().copied().collect();
        pids.sort_unstable();
        for pid in pids {
            let Some(vid) = self.vid_of_pid(pid) else {
                continue;
            };
            let at = world::character::number(&mut self.dice, 0, 3);
            let spot = AUDIENCE_SPOTS[usize::try_from(at).unwrap_or(0).min(3)];
            let _shown = self.show_body(vid.raw(), (spot.0, spot.1, 0), None);
        }
        self.ox_misses().clear();
    }

    /// `GiveItemToAttender` (`G/OXEvent.cpp:456-471`) for one PID: the reward lands
    /// as `AutoGiveItem` lands it, loud (`bMsg` is true), unplaced at the feet; the
    /// item log rides the tracing log, as the game's logs do. The tick has no
    /// descriptor, so the rows are ordered to the one that holds the winner, as the
    /// direct pick-up orders its rows; unwritten rows would vanish on relog.
    fn ox_reward(&mut self, pid: u32) {
        let Some(vid) = self.vid_of_pid(pid) else {
            return;
        };
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let questing = self.quest_running(vid);
        let Some((outcome, given)) = self.auto_give(vid, (REWARD_VNUM, 1), -1, questing) else {
            return;
        };
        if let world::character::AutoGiven::Unplaced(item) = outcome {
            self.lay_auto_given(vid, item);
        }
        let packed = crate::item_move::MovedItems::new(
            pid,
            vid.raw(),
            given,
            mover,
            &self.protos,
            &self.locale,
        );
        for record in packed.records {
            let _sent = self.write_to_client(vid, record);
        }
        if !packed.changes.is_empty() {
            if let Some((channel, map)) = self.place_of(vid.raw()) {
                let order = crate::item_move::StoreOrder::default();
                let _ordered = self.order_the_item_rows(vid, channel, map, packed.changes, order);
            }
        }
        tracing::info!(pid, vnum = REWARD_VNUM, "ITEM_LOG OXEVENT_REWARD");
    }

    /// `EffectPacket` (`G/char.cpp:7533-7542`) around `vid`.
    fn send_ox_effect(&self, vid: Vid, effect: u8) {
        let mut record = Vec::new();
        GcSpecialEffect {
            effect_type: effect,
            vid: vid.raw(),
        }
        .encode_into(&mut record);
        self.packet_around(super::view::EntityKey::Character(vid.raw()), &record, None);
    }

    /// The pass's cheer (`:350-374`): `cheer1` or `cheer2` with the VID fans around
    /// as a `CHAT_TYPE_COMMAND` line, with its terminator.
    fn send_ox_cheer(&mut self, vid: Vid) {
        let cheer = if world::character::number(&mut self.dice, 0, 1) == 1 {
            b"cheer1".as_slice()
        } else {
            b"cheer2".as_slice()
        };
        let mut text = cheer.to_vec();
        text.push(b' ');
        text.extend_from_slice(vid.raw().to_string().as_bytes());
        text.extend_from_slice(b" 0");
        text.push(0);
        let chat = GcChat {
            chat_type: CHAT_TYPE_COMMAND,
            id: 0,
            empire: 0,
            can_format: true,
            text,
        };
        let Ok(record) = chat.encode() else {
            return;
        };
        self.packet_around(super::view::EntityKey::Character(vid.raw()), &record, None);
    }

    /// `SendNoticeMap(line, 113, big)` (`G/cmd_gm.cpp:1453-1500`): every PC on 113
    /// hears the line as a notice, or a big notice.
    fn tell_ox_map(&self, line: &[u8], big: bool) {
        let chat = if big {
            CHAT_TYPE_BIG_NOTICE
        } else {
            CHAT_TYPE_NOTICE
        };
        let mut vids: Vec<Vid> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.map == OX_MAP)
            .map(|(vid, _)| *vid)
            .collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, chat, b"%s", &[Arg::Text(line)]);
            }
        }
    }

    /// The misses (`m_map_miss`).
    fn ox_misses(&mut self) -> &mut std::collections::BTreeSet<u32> {
        &mut self.ox_event.misses
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gamedata::mob_locale_names::MobNamesByLanguage;
    use gamedata::server_attr::SectreeGrid;

    use super::{
        CLOSED, FINISHED, OPEN, OX_MAP, QUIZ_LEVEL, ROUND_COUNT, WAITING_MINUTES, WIN_COUNT,
    };
    use tokio::sync::mpsc::UnboundedReceiver;

    use crate::game_state::fixtures::{a_world, enter_on, TestClock};

    /// What the inbox holds, oldest first.
    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The OX map on Channel 1 with the quest runtime, whose locale dir is the
    /// owner's, so the start reads the real quiz file.
    fn ox_world(clock: &TestClock) -> crate::game_state::GameState {
        let mut world = a_world(clock)
            .with_clients(Arc::new(crate::client_registry::ChannelClients::new()))
            .with_quests(quests());
        world
            .install_item_ids(world::item::ItemIdRange::new(1, 1_000_000, 1).expect("a range"))
            .expect("the first install");
        world.host_map(
            1,
            OX_MAP,
            SectreeGrid {
                x: 0,
                y: 0,
                columns: 200,
                rows: 200,
            },
        );
        world.with_locale_dir(owner_locale())
    }

    /// The owner's locale directory.
    fn owner_locale() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe")
    }

    /// The quest runtime over the legacy locale.
    fn quests() -> crate::game_state::quests::Quests {
        let manager = quest::manager::Manager::load(&owner_locale())
            .unwrap_or_else(|error| panic!("{error}"));
        crate::game_state::quests::Quests::new(
            manager,
            Arc::default(),
            MobNamesByLanguage::default(),
        )
    }

    /// The status flag the world reads.
    fn status(world: &crate::game_state::GameState) -> i32 {
        world.event_rate(super::super::gm_lines::OXEVENT_STATUS)
    }

    /// The start loads the owner's 567 quizzes to level 1, opens the status and
    /// arms the open process; the minutes count down and chat, the close spends
    /// a round, and a second start refuses.
    #[test]
    fn the_start_loads_the_quizzes_and_the_minutes_count_down() {
        let clock = TestClock::default();
        let mut world = ox_world(&clock);
        world.start_game_event(11, 0, 0);
        assert_eq!(world.ox_quiz.len(), usize::from(QUIZ_LEVEL) + 1);
        assert_eq!(world.ox_quiz[usize::from(QUIZ_LEVEL)].len(), 567);
        assert_eq!(status(&world), i32::from(OPEN));
        assert_eq!(
            (
                world.ox_event.state,
                world.ox_event.minutes,
                world.ox_event.rounds
            ),
            (OPEN, WAITING_MINUTES, ROUND_COUNT)
        );
        let before = world.ox_event.due;
        world.start_game_event(11, 0, 0);
        assert_eq!(world.ox_event.due, before, "a second start refuses");
        for left in (1..WAITING_MINUTES).rev() {
            let due = world.ox_event.due.expect("the minute is armed");
            world.run_ox_event(due);
            assert_eq!(world.ox_event.minutes, left);
            assert_eq!(world.ox_event.state, OPEN);
        }
        let due = world.ox_event.due.expect("the close is armed");
        world.run_ox_event(due);
        assert_eq!(world.ox_event.state, CLOSED);
        assert_eq!(world.ox_event.rounds, ROUND_COUNT - 1);
        assert_eq!(status(&world), i32::from(CLOSED));
    }

    /// Six attenders quiz: the question chats its three lines, leaves the list and
    /// arms the timer; the check fails the wrong foot and the absent, passes the
    /// right foot, and the misses warp to the audience.
    #[test]
    fn the_quiz_checks_the_feet_and_warps_the_misses() {
        let clock = TestClock::default();
        let mut world = ox_world(&clock);
        world.start_game_event(11, 0, 0);
        for vid in 1..=6u32 {
            let _inbox = enter_on(&mut world, (1, OX_MAP), vid, (896_500, 24_600), 820);
            world.ox_attenders.insert(vid);
            world.ox_roster.insert(vid);
        }
        // The sixth stands outside before the check.
        let _shown = world.show_body(6, (800_000, 10_000, 0), None);
        for _ in 0..WAITING_MINUTES {
            let due = world.ox_event.due.expect("the minute is armed");
            world.run_ox_event(due);
        }
        assert_eq!(world.ox_event.state, CLOSED);
        let quizzes = world.ox_quiz[usize::from(QUIZ_LEVEL)].len();
        let fire = world.ox_event.due.expect("the quiz is armed");
        world.run_ox_event(fire);
        assert_eq!(
            world.ox_quiz[usize::from(QUIZ_LEVEL)].len(),
            quizzes - 1,
            "the question leaves the list"
        );
        assert_eq!(status(&world), 3, "the status quizzes");
        // In test-time `last_pulse` never steps, so the quiz's timer is already
        // due when the quiz fires and step 0 runs inside the same call: the armed
        // timer is step 1.
        let (answer, step, check) = world.ox_event.timer.expect("the timer is armed");
        assert_eq!(step, 1, "step 0 ran with the quiz");
        // The seat is in the gap between the halves, so five feet walk into the
        // answer's half before the check; the sixth stays outside.
        let half = if answer { 898_450 } else { 894_450 };
        for vid in 1..=5u32 {
            let _shown = world.show_body(vid, (half, 24_650, 0), None);
        }
        world.run_ox_event(check);
        assert_eq!(world.ox_attenders.len(), 5, "the outside foot drops");
        assert!(!world.ox_misses().is_empty(), "a miss landed");
        let (_, _, warp) = world.ox_event.timer.expect("the warp is armed");
        world.run_ox_event(warp);
        assert!(world.ox_event.timer.is_none(), "the timer disarms");
        assert!(world.ox_misses().is_empty(), "the misses clear");
        assert_eq!(status(&world), i32::from(CLOSED));
    }

    /// Five attenders end the quizzing: each hears the reward land, the teleport
    /// line chats, and the end state opens the next round. Each winner's
    /// descriptor is ordered the reward's stored row, as the direct pick-up's
    /// descriptor is ordered its given rows.
    #[test]
    fn five_attenders_take_the_reward_and_the_round_turns() {
        let clock = TestClock::default();
        let mut world = ox_world(&clock);
        world.start_game_event(11, 0, 0);
        let registry = Arc::clone(world.clients.as_ref().expect("the registry rides"));
        let mut inboxes = Vec::new();
        let mut leases = Vec::new();
        let winners = u32::try_from(WIN_COUNT).unwrap_or(0);
        for vid in 1..=winners {
            inboxes.push(enter_on(
                &mut world,
                (1, OX_MAP),
                vid,
                (896_500, 24_600),
                820,
            ));
            leases.push(registry.join(crate::client_registry::ClientEntry {
                channel: 1,
                map: OX_MAP,
                name: format!("P{vid}"),
                vid,
                empire: 1,
                language: 0,
            }));
            world.ox_attenders.insert(vid);
            world.ox_roster.insert(vid);
        }
        for _ in 0..WAITING_MINUTES {
            let due = world.ox_event.due.expect("the minute is armed");
            world.run_ox_event(due);
        }
        let fire = world.ox_event.due.expect("the quiz is armed");
        world.run_ox_event(fire);
        assert_eq!(world.ox_event.state, FINISHED);
        for inbox in &mut inboxes {
            let records = drained(inbox);
            let set = records
                .iter()
                .find(|record| {
                    record.first() == Some(&protocol::gc_item_window::HEADER_GC_ITEM_SET)
                })
                .expect("the reward sets");
            assert_eq!(
                u32::from_le_bytes(set[4..8].try_into().expect("four bytes")),
                50109,
                "the winners' 50109"
            );
        }
        for (vid, lease) in (1..=winners).zip(leases.iter_mut()) {
            let mut orders = lease.take_orders().expect("the queue is home");
            let got: Vec<_> = std::iter::from_fn(|| orders.try_recv().ok()).collect();
            lease.put_orders(orders);
            assert_eq!(got.len(), 1, "one order carries the rows of {vid}");
            let crate::client_registry::ClientOrder::ItemRows { changes, .. } = &got[0] else {
                panic!("the order is the reward's rows");
            };
            assert!(
                changes.iter().any(
                    |change| matches!(change, db::items::RowChange::Created(row) if row.vnum == 50109)
                ),
                "the 50109 row is written"
            );
        }
        let turn = world.ox_event.due.expect("the turn is armed");
        world.run_ox_event(turn);
        assert_eq!(world.ox_event.state, OPEN);
        assert!(world.ox_roster.is_empty(), "the close clears the roster");
        assert!(!world.ox_attenders.is_empty(), "the attenders stay seated");
    }

    /// The panel's end chats, finishes the status, warps the roster home and
    /// disarms the process.
    #[test]
    fn the_end_warps_the_roster_home_and_disarms() {
        let clock = TestClock::default();
        let mut world = ox_world(&clock);
        world.start_game_event(11, 0, 0);
        let _inbox = enter_on(&mut world, (1, OX_MAP), 7, (896_500, 24_600), 820);
        world.ox_attenders.insert(7);
        world.ox_roster.insert(7);
        world.end_game_event(11);
        assert_eq!(status(&world), i32::from(FINISHED));
        assert!(world.ox_event.due.is_none(), "the process disarms");
        assert!(world.ox_event.timer.is_none(), "the timer disarms");
        assert!(world.ox_roster.is_empty(), "the roster clears");
    }
}
