//! The quest calls whose effects land after the step: `notice_all`, the Ochao
//! `initialize`, and `command` (`G/questlua_global.cpp:730-741,944-984,1521-1544`).
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

/// What a snapshot of a missing Lua slot holds: `snprintf("%s", NULL)` on glibc.
const NULL_SLOT: &[u8] = b"(null)";

/// `SendLocaleNotice`'s piece separator.
const ENTER: &[u8] = b"[ENTER]";

impl GameState {
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

    /// Run the quiet calls after the step, in call order: the notices, the temple
    /// re-arms, then the commands.
    pub(super) fn apply_quiet_calls(
        &mut self,
        reader: Vid,
        mover: Mover,
        notices: &[NoticeParts],
        prepares: usize,
        commands: &[Vec<u8>],
    ) {
        for parts in notices {
            self.broadcast_notice(parts);
        }
        for _ in 0..prepares {
            self.prepare_temple_ochao();
        }
        for line in commands {
            self.run_quest_command(reader, mover, line);
        }
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
    use super::NoticeParts;
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
}
