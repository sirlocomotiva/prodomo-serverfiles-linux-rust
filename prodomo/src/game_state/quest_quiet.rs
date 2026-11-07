//! The quest calls whose effects land after the step: `notice_all`, the Ochao
//! `initialize`, `command`, `add_ox_quiz`, the Gaya pair, the aura trio and the event
//! pair (`G/questlua_global.cpp:730-741,944-984,1521-1544,1636-1686`,
//! `G/questlua_game.cpp:95-126,312-351`).
//!
//! `SendLocaleNotice` (`G/cmd_gm.cpp:1252-1313`) speaks to every in-game character: a
//! non-empty all-digit first part is looked up in the character's own language, `%d`
//! is rewritten to `%s`, `vsnprintf` renders the five parts, and each `[ENTER]` piece
//! goes out as its own notice. `command` runs `interpret_command` on the reader, and
//! `initialize` re-arms the temple. The dispatches run after the step, in call order.

use common::vid::Vid;

use super::GameState;
use crate::chat_line::Arg;
use crate::command::{self, Caller, Command, CommandFlood, Interpreted};
use crate::item_move::Mover;

/// The six notice slots legacy snapshots (`MAX_QUEST_NOTICE_ARGS + 1`, `C/length.h:1197`).
pub(super) type NoticeParts = [Vec<u8>; 6];

/// One Gaya market shelf (`<Name>_gaya_system.txt`, `G/char_gaya.cpp:175-273`): the
/// nine rolled items and the six slot unlocks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GayaMarket {
    /// The nine items: vnum, Gaya price, count.
    pub(super) items: Vec<(u32, u32, u32)>,
    /// The six slot unlocks: slots 3-8 open when nonzero.
    pub(super) slots: [u32; 6],
}

/// The owned window and event queues one step ran, in call order each. One struct so
/// `ReaderGains` stays small.
#[derive(Default)]
pub(super) struct QuietOwned {
    /// The `game.open_gaya` calls.
    pub(super) gaya_opens: u32,
    /// The `game.open_gaya_market` calls.
    pub(super) gaya_market_opens: u32,
    /// The aura opens: the window and the opener's VID.
    pub(super) aura_opens: Vec<(u8, u32)>,
    /// The `start_event` calls: the index and its two values.
    pub(super) event_starts: Vec<(i32, i32, i32)>,
    /// The `end_event` calls: the index.
    pub(super) event_ends: Vec<i32>,
}

/// The quiet queues one step ran, in call order each. One struct so the apply calls
/// stay under the argument lint.
#[derive(Clone, Copy)]
pub(super) struct QuietCalls<'a> {
    /// The `notice_all` snapshots.
    pub(super) notices: &'a [NoticeParts],
    /// The `temple_ochao.initialize` calls.
    pub(super) prepares: usize,
    /// The `command` lines.
    pub(super) commands: &'a [Vec<u8>],
    /// The `add_ox_quiz` calls: level, question, answer.
    pub(super) quizzes: &'a [(u8, Vec<u8>, bool)],
    /// The `game.open_gaya` calls.
    pub(super) gaya_opens: u32,
    /// The `game.open_gaya_market` calls.
    pub(super) gaya_market_opens: u32,
    /// The aura opens: the window and the opener's VID.
    pub(super) aura_opens: &'a [(u8, u32)],
    /// The `start_event` calls: the index and its two values.
    pub(super) event_starts: &'a [(i32, i32, i32)],
    /// The `end_event` calls: the index.
    pub(super) event_ends: &'a [i32],
}

/// One `tag_Quiz` (`G/OXEvent.h:4-9`): the level, the question as `strlcpy` keeps it
/// (255 bytes and the NUL), and the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OxQuiz {
    /// Its level.
    pub(super) level: u8,
    /// Its question, NUL-padded to 256.
    pub(super) question: [u8; 256],
    /// Its answer.
    pub(super) answer: bool,
}

/// What a snapshot of a missing Lua slot holds: `snprintf("%s", NULL)` on glibc.
const NULL_SLOT: &[u8] = b"(null)";

/// `SendLocaleNotice`'s piece separator.
const ENTER: &[u8] = b"[ENTER]";

impl GameState {
    /// Share the Gaya table boot read. Without it a fresh shelf rolls nothing: every
    /// market opens empty.
    #[must_use]
    pub fn with_gaya_table(mut self, table: std::sync::Arc<gamedata::gaya::GayaTable>) -> Self {
        self.gaya_table = table;
        self
    }

    /// Snapshot the six notice slots of a `notice_all` call: the call's parts, then
    /// `(null)` for each Lua slot the call does not pass.
    pub(super) fn snapshot_notice(parts: &[Vec<u8>]) -> NoticeParts {
        std::array::from_fn(|slot| {
            parts
                .get(slot)
                .map_or_else(|| NULL_SLOT.to_vec(), Vec::clone)
        })
    }

    /// `SendLocaleNotice` (`G/cmd_gm.cpp:1252-1313`): every online character hears the
    /// notice, in VID order. A non-empty all-digit first part is that character's own
    /// language's quest text; otherwise the part is literal. The rendered notice is
    /// split on `[ENTER]`, and each piece goes out as its own notice, the last even
    /// when it is empty.
    pub(super) fn broadcast_notice(&self, parts: &NoticeParts) {
        let mut vids: Vec<Vid> = self.bodies.keys().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let Some(mover) = self.own_mover(vid) else {
                continue;
            };
            let rendered = self.render_notice(parts, mover.language);
            let mut rest = rendered.as_slice();
            while let Some(at) = find_enter(rest) {
                self.tell(
                    vid,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_NOTICE,
                    b"%s",
                    &[Arg::Text(&rest[..at])],
                );
                rest = &rest[at + ENTER.len()..];
            }
            self.tell(
                vid,
                mover,
                protocol::gc_chat::CHAT_TYPE_NOTICE,
                b"%s",
                &[Arg::Text(rest)],
            );
        }
    }

    /// The rendered notice for one language: the digit-key lookup, the `%d` rewrite,
    /// and the raw `vsnprintf`, without the locale-strings lookup `ChatPacket` would
    /// run on a format (legacy formats the quest text with a bare `vsnprintf`).
    fn render_notice(&self, parts: &NoticeParts, language: u8) -> Vec<u8> {
        let first: &[u8] = &parts[0];
        let format = if !first.is_empty() && first.iter().all(u8::is_ascii_digit) {
            self.quest_translate.find(atoi(first), language)
        } else {
            first
        };
        let rewritten = rewrite_percent_d(format);
        let args = [
            Arg::Text(&parts[1]),
            Arg::Text(&parts[2]),
            Arg::Text(&parts[3]),
            Arg::Text(&parts[4]),
            Arg::Text(&parts[5]),
        ];
        crate::chat_line::notice_text(&rewritten, &args)
    }

    /// `command` (`G/questlua_global.cpp:730-741`): `interpret_command` on the reader,
    /// with the reader's GM level and its dead-or-standing position. The flood is
    /// fresh, so a quest line never trips it: the world has no delayed-disconnect
    /// path (a Divergence). The dispatch runs the battle-pass usage gate; any other
    /// line is logged and ignored, as an unported chat line is.
    pub(super) fn run_quest_command(&self, reader: Vid, mover: Mover, line: &[u8]) {
        let gm_level = self
            .characters
            .find_by_vid(reader)
            .ok()
            .and_then(|character| character.points())
            .map_or(command::GM_PLAYER, world::character::Points::gm_level);
        let position = if self.is_dead(reader.raw()) {
            command::POS_DEAD
        } else {
            command::POS_STANDING
        };
        let caller = Caller {
            pulse: self.last_pulse,
            gm_level,
            position,
        };
        match command::interpret(line, &mut CommandFlood::default(), caller) {
            Interpreted::Nothing => {}
            Interpreted::Disconnect => {
                tracing::warn!(
                    ?reader,
                    "a quest command flooded; the world cannot disconnect"
                );
            }
            Interpreted::Notice(text) => {
                self.tell(
                    reader,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_INFO,
                    text.as_bytes(),
                    &[],
                );
            }
            Interpreted::Run { entry, rest } => match entry.command() {
                Command::NotPorted("battle_pass") => {
                    self.run_battle_pass_line(reader, mover, &rest);
                }
                Command::NotPorted(handler) => {
                    tracing::info!(handler, "Command not ported; ignoring");
                }
                _ => {
                    tracing::info!("a quest command dispatches battle_pass only; ignoring");
                }
            },
        }
    }

    /// `do_battle_pass` (`G/cmd_general.cpp:4184-4219`): fewer than two words tells the
    /// usage; `open` and `premium` are unported (the pass never opens), and any other
    /// first word is quiet, as the `switch` falls through.
    fn run_battle_pass_line(&self, reader: Vid, mover: Mover, rest: &[u8]) {
        let (first, second) = command::two_arguments(rest);
        if first.is_empty() || second.is_empty() {
            self.tell(
                reader,
                mover,
                protocol::gc_chat::CHAT_TYPE_INFO,
                b"Usage: battle_pass <field> <value>",
                &[],
            );
            return;
        }
        for word in ["open", "premium"] {
            if word.as_bytes().starts_with(&first) {
                tracing::info!("Command not ported; ignoring");
                return;
            }
        }
    }

    /// An `add_ox_quiz` call after the step: the quiz lands in the OX list, as
    /// `COXEventManager::AddQuiz` lands it (`G/OXEvent.cpp:175-186`).
    fn land_ox_quiz(&mut self, level: u8, question: &[u8], answer: bool) {
        let slot = usize::from(level);
        if self.ox_quiz.len() < slot + 1 {
            self.ox_quiz.resize_with(slot + 1, Vec::new);
        }
        let mut kept = [0u8; 256];
        let take = question.len().min(255);
        kept[..take].copy_from_slice(&question[..take]);
        self.ox_quiz[slot].push(OxQuiz {
            level,
            question: kept,
            answer,
        });
    }

    /// `game_open_gaya_c` (`G/questlua_game.cpp:95-101`) after the step: the client
    /// opens the Gaya window.
    fn open_gaya_window(&self, reader: Vid, mover: Mover) {
        self.tell(
            reader,
            mover,
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            b"OpenGuiGaya",
            &[],
        );
    }

    /// `game_open_gaya_m` (`G/questlua_game.cpp:103-126`) after the step: a reader
    /// with no market row gets a fresh shelf and the 5-hour stamp (`CheckItemsFull`
    /// false), else the row's markers are read back (`InfoGayaMarker`), and the client
    /// opens the Gaya market.
    fn open_gaya_market_window(&mut self, reader: Vid, mover: Mover) {
        if !self.gaya_markets.contains_key(&reader) {
            self.restock_gaya_market(reader);
            self.set_gaya_stamp(reader);
        }
        self.send_gaya_market(reader, mover);
        self.tell(
            reader,
            mover,
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            b"OpenGuiGayaMarket",
            &[],
        );
    }

    /// `UpdateItemsGayaMarker` (`G/char_gaya.cpp:242-273`): nine rolls of
    /// `number(1, size - 1)` onto the reader's shelf, the slots all locked. An empty
    /// or one-row table rolls nothing: legacy's `number` faults on the empty range.
    fn restock_gaya_market(&mut self, reader: Vid) {
        let last = i32::try_from(self.gaya_table.items.len().saturating_sub(1)).unwrap_or(0);
        let mut items = Vec::new();
        if last >= 1 {
            for _ in 0..9 {
                let roll = world::character::number(&mut self.dice, 1, last);
                let row = &self.gaya_table.items[usize::try_from(roll).unwrap_or(1)];
                items.push((row.vnum, row.price, row.count));
            }
        }
        self.gaya_markets.insert(
            reader,
            GayaMarket {
                items,
                slots: [0; 6],
            },
        );
    }

    /// `SetGayaState("system_gaya.gaya_time_world_4", init_gayaTime() + 5 hours)`
    /// (`G/char_gaya.cpp:378-382`): the reader's market stamp, which the refresh reads.
    fn set_gaya_stamp(&mut self, reader: Vid) {
        let stamp = wall_time().saturating_add(5 * 60 * 60);
        let stamp = i32::try_from(stamp).unwrap_or(i32::MAX);
        let pid = self
            .characters
            .find_by_vid(reader)
            .ok()
            .map(world::character::Character::player_id);
        if let (Some(pid), Some(quests)) = (pid, self.quests.as_mut()) {
            quests
                .manager_mut()
                .set_quest_flag(pid, b"system_gaya.gaya_time_world_4", stamp);
        }
    }

    /// `check_time_market_event` (`G/char_gaya.cpp:386-409`): every 2 s (50 Pulses)
    /// each standing shelf whose stamp has lapsed refreshes — the stamp stamps 5 h
    /// out, the shelf re-rolls, and the client hears the markers again. Quiet with
    /// no shelves. The per-character event arms at the market open, so the tick runs
    /// only over open shelves; a lapsed stamp with no shelf refreshes at the next
    /// open instead (`open_gaya_market_window` keeps the first-stamp arm).
    pub(super) fn run_gaya_ticks(&mut self, pulse: u64) {
        if self.gaya_markets.is_empty() || pulse % 50 != 0 {
            return;
        }
        let now = wall_time();
        let lapsed: Vec<Vid> = self
            .gaya_markets
            .keys()
            .filter(|reader| {
                self.characters
                    .find_by_vid(**reader)
                    .ok()
                    .map(world::character::Character::player_id)
                    .and_then(|pid| {
                        self.quests.as_ref().and_then(|quests| {
                            quests
                                .manager()
                                .quest_flag(pid, b"system_gaya.gaya_time_world_4")
                        })
                    })
                    .is_some_and(|stamp| i64::from(stamp) - i64::from(now) <= 0)
            })
            .copied()
            .collect();
        for reader in lapsed {
            self.restock_gaya_market(reader);
            self.set_gaya_stamp(reader);
            if let Some(mover) = self.own_mover(reader) {
                self.send_gaya_market(reader, mover);
            }
        }
    }

    /// `InfoGayaMarker` (`G/char_gaya.cpp:94-110`): the reader's shelf reaches its
    /// client: the clear, each item, then the slots and the stamp when a row holds
    /// them. Quiet with no shelf.
    fn send_gaya_market(&self, reader: Vid, mover: Mover) {
        let Some(market) = self.gaya_markets.get(&reader) else {
            return;
        };
        let command = protocol::gc_chat::CHAT_TYPE_COMMAND;
        self.tell(reader, mover, command, b"GayaMarketClear", &[]);
        for (vnum, price, count) in &market.items {
            self.tell(
                reader,
                mover,
                command,
                b"GayaMarketItems %d %d %d",
                &[
                    crate::chat_line::Arg::Int(i64::from(*vnum)),
                    crate::chat_line::Arg::Int(i64::from(*price)),
                    crate::chat_line::Arg::Int(i64::from(*count)),
                ],
            );
        }
        let slots = market.slots;
        self.tell(
            reader,
            mover,
            command,
            b"GayaMarketSlotsDesblock %d %d %d %d %d %d",
            &[
                crate::chat_line::Arg::Int(i64::from(slots[0])),
                crate::chat_line::Arg::Int(i64::from(slots[1])),
                crate::chat_line::Arg::Int(i64::from(slots[2])),
                crate::chat_line::Arg::Int(i64::from(slots[3])),
                crate::chat_line::Arg::Int(i64::from(slots[4])),
                crate::chat_line::Arg::Int(i64::from(slots[5])),
            ],
        );
        let stamp = self
            .characters
            .find_by_vid(reader)
            .ok()
            .map(world::character::Character::player_id)
            .and_then(|pid| {
                self.quests.as_ref().and_then(|quests| {
                    quests
                        .manager()
                        .quest_flag(pid, b"system_gaya.gaya_time_world_4")
                })
            })
            .unwrap_or(0);
        self.tell(
            reader,
            mover,
            command,
            b"GayaMarketTime %d",
            &[crate::chat_line::Arg::Int(i64::from(stamp))],
        );
    }

    /// `game_open_aura_*_window` (`G/questlua_game.cpp:312-351`,
    /// `CHARACTER::OpenAuraRefineWindow`, `G/char_aura.cpp:14-71`) after the step: the
    /// already-open window chats `[LS;1014]`, an opener 1000 or more off chats
    /// `[LS;459]`, else the client hears the open for `window` (0 absorb, 1 growth, 2
    /// evolve) and the window stands on the reader.
    fn open_aura_window(&mut self, reader: Vid, mover: Mover, window: u8, opener: u32) {
        let info = protocol::gc_chat::CHAT_TYPE_INFO;
        if self.aura_windows.contains_key(&reader) {
            self.tell(reader, mover, info, b"[LS;1014]", &[]);
            return;
        }
        if !self.aura_opener_near(reader, opener) {
            self.tell(reader, mover, info, b"[LS;459]", &[]);
            return;
        }
        let record = protocol::gc_aura::GcAuraOpen { window }.encode();
        let _sent = self.write_to_client(reader, record.to_vec());
        self.aura_windows.insert(reader, (window, opener));
    }

    /// The aura opener's range: the NPC stands on the reader's map within
    /// `DISTANCE_APPROX` 1000 (`G/char_aura.cpp:22-27`). A missing NPC or a missing
    /// reader is out of range, as the dereference would fault.
    fn aura_opener_near(&self, reader: Vid, opener: u32) -> bool {
        let (Some(place), Some(kept)) = (self.place_of(reader.raw()), self.kept_place(reader))
        else {
            return false;
        };
        let npcs = self.npcs.get(&(place.0, place.1));
        let npc = npcs.and_then(|npcs| npcs.npcs.iter().find(|npc| npc.vid == opener));
        npc.is_some_and(|npc| {
            crate::sync_position::distance_approx(npc.x - kept.1, npc.y - kept.2) < 1000
        })
    }

    /// `_start_event` (`G/questlua_global.cpp:1636-1665`,
    /// `CEventManager::SetEventState`, `G/event_manager.cpp:719-779`) after the step:
    /// an index no event has is quiet, as `GetEventString` answers `EVENT_TYPE_NONE`
    /// and the switch falls through; so is an event whose state already holds, as
    /// legacy's early return does. Else the flag takes the state, and the exp and
    /// loot arms grant their empire privs (`SetExperienceEvent`, `SetItemDropEvent`,
    /// `:928-982`); every other arm waits for its system.
    fn start_game_event(&mut self, index: i32, value0: i32, value1: i32) {
        let _ = value1;
        let Some(flag) = event_flag_name(index) else {
            return;
        };
        if self.manager_event_flag(flag) != 0 {
            return;
        }
        self.set_event_flag(flag, 1);
        if index == 1 {
            self.grant_event_priv(4, value0);
            self.broadcast_notice_text(b"Experience Boost Event has started. Enjoy the leveling.");
        } else if index == 2 {
            self.grant_event_priv(1, value0);
            self.broadcast_notice_text(b"Item-Drop Boost Event has started. Enjoy the grinding.");
        }
    }

    /// `_end_event` (`G/questlua_global.cpp:1668-1686`) after the step: an index no
    /// event has, or whose state already cleared, is quiet. Else the flag clears,
    /// and the exp and loot arms chat their end.
    fn end_game_event(&mut self, index: i32) {
        let Some(flag) = event_flag_name(index) else {
            return;
        };
        if self.manager_event_flag(flag) == 0 {
            return;
        }
        self.set_event_flag(flag, 0);
        if index == 1 {
            self.broadcast_notice_text(b"Experience Boost Event has ended.");
        } else if index == 2 {
            self.broadcast_notice_text(b"Item-Drop Boost Event has ended.");
        }
    }

    /// The manager's event flag `flag` (`GetEventState`'s read,
    /// `G/event_manager.cpp:663-716`), 0 with no quests.
    fn manager_event_flag(&self, flag: &[u8]) -> i32 {
        self.quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(flag))
    }

    /// `RequestGiveEmpirePriv(ALL_EMPIRES, priv_type, value, week)`
    /// (`G/priv_manager.cpp:47-66`): the event's empire priv, clamped to 0-200, for
    /// the call's seven-day maximum. A zero value grants nothing after the flag, as
    /// `SetExperienceEvent` refuses it.
    fn grant_event_priv(&mut self, priv_type: u8, value: i32) {
        if value == 0 {
            return;
        }
        let pulses = 7 * 24 * 60 * 60 * u64::from(super::PASSES_PER_SEC);
        self.give_empire_priv(
            0,
            priv_type,
            value.clamp(0, 200),
            self.last_pulse.saturating_add(pulses),
        );
    }

    /// `BroadcastNotice(text)`: every online character hears `text` as a notice.
    fn broadcast_notice_text(&self, text: &[u8]) {
        let mut parts: NoticeParts = Default::default();
        parts[0] = text.to_vec();
        for slot in &mut parts[1..] {
            *slot = b"(null)".to_vec();
        }
        self.broadcast_notice(&parts);
    }

    /// The event pairs after the step, in call order. Split out so
    /// `apply_window_calls` stays under the argument lint.
    fn apply_event_calls(&mut self, event_starts: &[(i32, i32, i32)], event_ends: &[i32]) {
        for (index, value0, value1) in event_starts {
            self.start_game_event(*index, *value0, *value1);
        }
        for index in event_ends {
            self.end_game_event(*index);
        }
    }

    /// The window and event calls after the step, in call order: the Gaya pairs,
    /// the aura trio and the event pairs. Split out so `apply_quiet_calls` stays
    /// under the argument lint.
    fn apply_window_calls(&mut self, reader: Vid, mover: Mover, calls: QuietCalls<'_>) {
        for _ in 0..calls.gaya_opens {
            self.open_gaya_window(reader, mover);
        }
        for _ in 0..calls.gaya_market_opens {
            self.open_gaya_market_window(reader, mover);
        }
        for (window, opener) in calls.aura_opens {
            self.open_aura_window(reader, mover, *window, *opener);
        }
        self.apply_event_calls(calls.event_starts, calls.event_ends);
    }

    /// Run the first quiet calls after the step, in call order: the notices, the
    /// temple re-arms, then the commands and the OX quizzes.
    fn apply_first_calls(&mut self, reader: Vid, mover: Mover, calls: QuietCalls<'_>) {
        for parts in calls.notices {
            self.broadcast_notice(parts);
        }
        for _ in 0..calls.prepares {
            self.prepare_temple_ochao();
        }
        for line in calls.commands {
            self.run_quest_command(reader, mover, line);
        }
        for (level, question, answer) in calls.quizzes {
            self.land_ox_quiz(*level, question, *answer);
        }
    }

    /// Run the quiet calls after the step, in call order: the first calls, then the
    /// window calls.
    pub(super) fn apply_quiet_calls(&mut self, reader: Vid, mover: Mover, calls: QuietCalls<'_>) {
        self.apply_first_calls(reader, mover, calls);
        self.apply_window_calls(reader, mover, calls);
    }
}

/// `init_gayaTime()` (`G/utils.cpp:23-26`): the wall clock in seconds.
fn wall_time() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u32::try_from(since.as_secs()).unwrap_or(u32::MAX)
        })
}

/// The `GetEventState` flag of event `index` (`G/event_manager.cpp:663-716`), and
/// `None` for an index no event has.
fn event_flag_name(index: i32) -> Option<&'static [u8]> {
    match index {
        1 => Some(b"exp_event"),
        2 => Some(b"item_drop_event"),
        3 => Some(b"boss_event"),
        4 => Some(b"metin_event"),
        5 => Some(b"mining_event"),
        6 => Some(b"golden_frog_event"),
        7 => Some(b"moon_drop"),
        8 => Some(b"hexegonal_drop"),
        9 => Some(b"fish_event"),
        10 => Some(b"hide_seek_event"),
        11 => Some(b"oxevent_status"),
        12 => Some(b"tanaka_event"),
        13 => Some(b"siege_war_event"),
        14 => Some(b"kingdom_war_event"),
        _ => None,
    }
}

/// The first `[ENTER]` in `text`, if any.
fn find_enter(text: &[u8]) -> Option<usize> {
    text.windows(ENTER.len()).position(|window| window == ENTER)
}

/// Every `%d` rewritten to `%s` (`SendLocaleNotice`'s filter).
fn rewrite_percent_d(format: &[u8]) -> Vec<u8> {
    let mut rewritten = Vec::with_capacity(format.len());
    let mut rest = format;
    while let Some(at) = rest.windows(2).position(|window| window == b"%d") {
        rewritten.extend_from_slice(&rest[..at]);
        rewritten.extend_from_slice(b"%s");
        rest = &rest[at + 2..];
    }
    rewritten.extend_from_slice(rest);
    rewritten
}

/// `atoi` of an all-digit key.
fn atoi(key: &[u8]) -> u32 {
    let mut value: u32 = 0;
    for &byte in key {
        value = value
            .saturating_mul(10)
            .saturating_add(u32::from(byte - b'0'));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{event_flag_name, NoticeParts};
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use crate::game_state::GameState;
    use common::vid::Vid;

    /// The snapshot keeps the call's parts and pads the missing slots with `(null)`.
    #[test]
    fn the_snapshot_pads_the_missing_slots() {
        let parts = GameState::snapshot_notice(&[b"hi".to_vec()]);
        assert_eq!(parts[0], b"hi");
        assert!(parts[1..].iter().all(|part| part == b"(null)"));
        let full: Vec<Vec<u8>> = (0..6).map(|slot| vec![slot]).collect();
        let snapshot = GameState::snapshot_notice(&full);
        assert_eq!(snapshot.as_slice(), full.as_slice());
    }

    /// The `%d` rewrite and the substitution: the parts render in order, a digit key
    /// answers `NoName` with no translations, and the missing slots read `(null)`.
    #[test]
    fn the_render_rewrites_substitutes_and_misses() {
        let clock = TestClock::default();
        let world = a_world(&clock);
        let parts: NoticeParts =
            GameState::snapshot_notice(&[b"%d and %s".to_vec(), b"one".to_vec()]);
        assert_eq!(world.render_notice(&parts, 1), b"one and (null)");
        let parts: NoticeParts = GameState::snapshot_notice(&[b"1".to_vec()]);
        assert_eq!(world.render_notice(&parts, 1), b"NoName");
        let parts: NoticeParts = GameState::snapshot_notice(&[b"".to_vec()]);
        assert_eq!(world.render_notice(&parts, 1), b"");
    }

    /// The broadcast speaks to every online character in VID order, each `[ENTER]`
    /// piece its own notice.
    #[test]
    fn the_broadcast_speaks_every_piece_to_every_character() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut low = enter(&mut world, 0x6001, (3200, 3200), 0);
        let mut high = enter(&mut world, 0x6002, (3300, 3300), 0);
        while low.try_recv().is_ok() {}
        while high.try_recv().is_ok() {}
        let parts: NoticeParts = GameState::snapshot_notice(&[b"a[ENTER]b".to_vec()]);
        world.broadcast_notice(&parts);
        for inbox in [&mut low, &mut high] {
            assert_eq!(text_of(&inbox.try_recv().expect("first")), b"a");
            assert_eq!(text_of(&inbox.try_recv().expect("last")), b"b");
            assert!(inbox.try_recv().is_err());
        }
        while low.try_recv().is_ok() {}
        while high.try_recv().is_ok() {}
    }

    /// A bare `battle_pass` tells the usage; `open` and `premium` are unported and
    /// quiet; another first word is quiet too.
    #[test]
    fn the_quest_command_runs_the_battle_pass_usage() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, 0x6001, (3200, 3200), 0);
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(0x6001)).expect("entered");
        world.run_quest_command(Vid::new(0x6001), mover, b"battle_pass");
        assert_eq!(
            text_of(&inbox.try_recv().expect("the usage")),
            b"Usage: battle_pass <field> <value>"
        );
        world.run_quest_command(Vid::new(0x6001), mover, b"battle_pass open 1");
        world.run_quest_command(Vid::new(0x6001), mover, b"battle_pass bogus 1");
        assert!(inbox.try_recv().is_err(), "nothing unported speaks");
        while inbox.try_recv().is_ok() {}
    }

    /// The text of a `GC_CHAT` line.
    fn text_of(line: &[u8]) -> Vec<u8> {
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    /// An unknown event index and a repeat start are quiet; a start sets the flag,
    /// grants the exp priv and speaks the start; the end clears the flag and speaks
    /// the end. Loot (2) grants item-drop and speaks its own lines; boss (3) moves
    /// the flag alone.
    #[test]
    fn the_event_pair_moves_flags_grants_and_notices() {
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
        world.start_game_event(99, 50, 0);
        world.end_game_event(99);
        assert!(inbox.try_recv().is_err(), "an unknown index is quiet");
        world.start_game_event(1, 50, 0);
        assert_eq!(
            text_of(&inbox.try_recv().expect("the start")),
            b"Experience Boost Event has started. Enjoy the leveling."
        );
        world.start_game_event(1, 50, 0);
        assert!(inbox.try_recv().is_err(), "a repeat start is quiet");
        world.end_game_event(1);
        assert_eq!(
            text_of(&inbox.try_recv().expect("the end")),
            b"Experience Boost Event has ended."
        );
        world.end_game_event(1);
        assert!(inbox.try_recv().is_err(), "a repeat end is quiet");
        world.start_game_event(2, 30, 0);
        assert_eq!(
            text_of(&inbox.try_recv().expect("loot start")),
            b"Item-Drop Boost Event has started. Enjoy the grinding."
        );
        world.end_game_event(2);
        assert_eq!(
            text_of(&inbox.try_recv().expect("loot end")),
            b"Item-Drop Boost Event has ended."
        );
        world.start_game_event(3, 0, 0);
        world.end_game_event(3);
        assert!(inbox.try_recv().is_err(), "boss (3) moves the flag alone");
        for (index, flag) in [
            (1, b"exp_event".as_slice()),
            (2, b"item_drop_event".as_slice()),
            (3, b"boss_event".as_slice()),
            (4, b"metin_event".as_slice()),
            (5, b"mining_event".as_slice()),
            (6, b"golden_frog_event".as_slice()),
            (7, b"moon_drop".as_slice()),
            (8, b"hexegonal_drop".as_slice()),
            (9, b"fish_event".as_slice()),
            (10, b"hide_seek_event".as_slice()),
            (11, b"oxevent_status".as_slice()),
            (12, b"tanaka_event".as_slice()),
            (13, b"siege_war_event".as_slice()),
            (14, b"kingdom_war_event".as_slice()),
        ] {
            assert_eq!(event_flag_name(index), Some(flag), "index {index}");
        }
        assert_eq!(event_flag_name(0), None);
        assert_eq!(event_flag_name(15), None);
        assert_eq!(event_flag_name(-1), None);
        let manager = &world.quests.as_ref().expect("quests").manager();
        assert_eq!(manager.event_flag(b"exp_event"), 0);
        assert_eq!(manager.event_flag(b"item_drop_event"), 0);
        assert_eq!(manager.event_flag(b"boss_event"), 0);
        assert!(inbox.try_recv().is_err());
    }

    /// The aura gate: a near opener opens, a second open chats `[LS;1014]`, and a
    /// far opener chats `[LS;459]`.
    #[test]
    fn the_aura_gate_measures_a_thousand() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let region = crate::game_state::fixtures::region();
        let npc = |vid: u32, x: i32| world::npc::Npc {
            vid,
            vnum: 60003,
            race: 60003,
            x,
            y: 3200,
            hp: 100,
            max_hp: 100,
            ..world::npc::Npc::default()
        };
        world.stand_npcs(
            crate::game_state::fixtures::PLACE.0,
            &region,
            world::npc::MapNpcs {
                npcs: vec![npc(7001, 3900), npc(7002, 5000)],
                positions: Vec::new(),
                regens: Vec::new(),
            },
        );
        let mut inbox = enter(&mut world, 0x6001, (3200, 3200), 0);
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(0x6001)).expect("entered");
        // 700 east: `DISTANCE_APPROX` reads ~671, inside.
        world.open_aura_window(Vid::new(0x6001), mover, 0, 7001);
        assert_eq!(inbox.try_recv().expect("the open"), vec![214, 5, 0, 0, 0]);
        world.open_aura_window(Vid::new(0x6001), mover, 1, 7001);
        assert_eq!(
            text_of(&inbox.try_recv().expect("the refusal")),
            b"[LS;1014]"
        );
        world.aura_windows.clear();
        // 1800 east: `DISTANCE_APPROX` reads ~1728, outside.
        world.open_aura_window(Vid::new(0x6001), mover, 2, 7002);
        assert_eq!(text_of(&inbox.try_recv().expect("the range")), b"[LS;459]");
        assert!(inbox.try_recv().is_err());
        assert!(!world.aura_windows.contains_key(&Vid::new(0x6001)));
    }

    /// A fresh market rolls nine shelf rows from the table and locks the slots; the
    /// client hears the clear, the nine items, the slots and the stamp.
    #[test]
    fn a_fresh_market_rolls_nine_and_sends_its_markers() {
        let clock = TestClock::default();
        let world =
            a_world(&clock).with_gaya_table(std::sync::Arc::new(gamedata::gaya::GayaTable {
                items: vec![
                    gamedata::gaya::GayaItem {
                        vnum: 70036,
                        price: 102,
                        count: 1,
                    },
                    gamedata::gaya::GayaItem {
                        vnum: 70038,
                        price: 212,
                        count: 2,
                    },
                ],
            }));
        let mut world = world;
        let mut inbox = enter(&mut world, 0x6001, (3200, 3200), 0);
        while inbox.try_recv().is_ok() {}
        let mover = world.own_mover(Vid::new(0x6001)).expect("entered");
        // A one-row table rolls nothing: legacy's `number(1, 0)` faults on the range.
        world.gaya_table = std::sync::Arc::new(gamedata::gaya::GayaTable {
            items: vec![gamedata::gaya::GayaItem {
                vnum: 7,
                price: 1,
                count: 1,
            }],
        });
        world.restock_gaya_market(Vid::new(0x6001));
        assert!(world
            .gaya_markets
            .get(&Vid::new(0x6001))
            .is_some_and(|market| market.items.is_empty()));
        world.gaya_table = std::sync::Arc::new(gamedata::gaya::GayaTable {
            items: (70036..70038)
                .map(|vnum| gamedata::gaya::GayaItem {
                    vnum,
                    price: 102,
                    count: 1,
                })
                .collect(),
        });
        world.restock_gaya_market(Vid::new(0x6001));
        let market = world.gaya_markets.get(&Vid::new(0x6001)).expect("stocked");
        assert_eq!(market.items.len(), 9);
        assert!(market
            .items
            .iter()
            .all(|row| row.0 == 70036 || row.0 == 70037));
        assert_eq!(market.slots, [0; 6]);
        world.send_gaya_market(Vid::new(0x6001), mover);
        assert_eq!(
            text_of(&inbox.try_recv().expect("clear")),
            b"GayaMarketClear"
        );
        for _ in 0..9 {
            let line = text_of(&inbox.try_recv().expect("an item"));
            assert!(line.starts_with(b"GayaMarketItems "), "{line:?}");
        }
        assert_eq!(
            text_of(&inbox.try_recv().expect("slots")),
            b"GayaMarketSlotsDesblock 0 0 0 0 0 0"
        );
        let stamp = text_of(&inbox.try_recv().expect("stamp"));
        assert!(stamp.starts_with(b"GayaMarketTime "), "{stamp:?}");
        assert!(inbox.try_recv().is_err());
        let _ = mover;
    }

    /// A lapsed market stamp refreshes on the 2 s tick: the shelf re-rolls, the
    /// stamp stamps 5 h out, and the client hears the markers again. A live stamp
    /// holds, and an off-tick Pulse does nothing.
    #[test]
    fn a_lapsed_market_refreshes_on_the_two_second_tick() {
        let clock = TestClock::default();
        let mut world =
            a_world(&clock).with_gaya_table(std::sync::Arc::new(gamedata::gaya::GayaTable {
                items: (70036..70040)
                    .map(|vnum| gamedata::gaya::GayaItem {
                        vnum,
                        price: 102,
                        count: 1,
                    })
                    .collect(),
            }));
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
        let mover = world.own_mover(Vid::new(0x6001)).expect("entered");
        world.open_gaya_market_window(Vid::new(0x6001), mover);
        while inbox.try_recv().is_ok() {}
        let pid = world
            .characters
            .find_by_vid(Vid::new(0x6001))
            .expect("entered")
            .player_id();
        // An off-tick Pulse does nothing, even lapsed.
        world
            .quests
            .as_mut()
            .expect("quests")
            .manager_mut()
            .set_quest_flag(pid, b"system_gaya.gaya_time_world_4", 1);
        world.run_gaya_ticks(51);
        assert!(inbox.try_recv().is_err(), "off-tick silence");
        // The tick refreshes: clear, nine items, slots, stamp.
        world.run_gaya_ticks(100);
        assert_eq!(
            text_of(&inbox.try_recv().expect("clear")),
            b"GayaMarketClear"
        );
        for _ in 0..9 {
            let line = text_of(&inbox.try_recv().expect("an item"));
            assert!(line.starts_with(b"GayaMarketItems "), "{line:?}");
        }
        assert_eq!(
            text_of(&inbox.try_recv().expect("slots")),
            b"GayaMarketSlotsDesblock 0 0 0 0 0 0"
        );
        assert!(text_of(&inbox.try_recv().expect("stamp")).starts_with(b"GayaMarketTime "));
        assert!(inbox.try_recv().is_err());
        let stamp = world
            .quests
            .as_ref()
            .expect("quests")
            .manager()
            .quest_flag(pid, b"system_gaya.gaya_time_world_4")
            .expect("stamped");
        assert!(stamp > 1, "the stamp moves 5 h out: {stamp}");
        // A live stamp holds on the next tick.
        world.run_gaya_ticks(150);
        assert!(inbox.try_recv().is_err(), "a live stamp holds");
        let _ = mover;
    }

    /// A quiz lands at its level with the answer, a 300-byte question keeps 255,
    /// and a higher level opens the gap between.
    #[test]
    fn the_quiz_lands_truncated_at_its_level() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world.land_ox_quiz(1, b"Steel cuts?", true);
        world.land_ox_quiz(3, &[b'q'; 300], false);
        assert_eq!(world.ox_quiz.len(), 4);
        assert!(world.ox_quiz[0].is_empty());
        assert!(world.ox_quiz[2].is_empty());
        let quiz = &world.ox_quiz[1][0];
        assert_eq!(quiz.level, 1);
        assert!(quiz.answer);
        assert_eq!(&quiz.question[..11], b"Steel cuts?");
        assert!(quiz.question[11..].iter().all(|byte| *byte == 0));
        let long = &world.ox_quiz[3][0];
        assert!(!long.answer);
        assert!(long.question[..255].iter().all(|byte| *byte == b'q'));
        assert_eq!(long.question[255], 0);
    }
}
