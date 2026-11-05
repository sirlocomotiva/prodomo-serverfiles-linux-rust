//! The PK mode line: `/pkmode` (`G/cmd_general.cpp:2252-2270`).
//!
//! [`GameState::run_pkmode`] is `ACMD(do_pkmode)`: the line's number becomes the
//! caller's mode, and its update fans to its view (`SetPKMode`,
//! `G/char_battle.cpp:3597-3616`). The number is `(unsigned char)strtoul(word)`
//! (`common/utils.h:20-26`): the low byte of the unsigned walk, so `257` reads 1
//! and `-1` reads 255. An empty word runs nothing, and every refusal is silent,
//! as legacy's bare `return`s are.
//!
//! The gates run in legacy order: the word (`PROTECT` never lands from a line),
//! the level (below `PK_PROTECT_LEVEL` only `PEACE` lands), the same mode (which
//! sends nothing), then `SetPKMode`'s folds — at and above `MAX_NUM` the line is
//! refused, on Meley's map (212, or its 2120000..2130000 private copies,
//! `MeleyLair.h:15`, `MeleyLair.cpp:1215-1222`) every mode folds to `GUILD`, and
//! `GUILD` without a guild folds to `FREE` — and the update fans. The caller
//! answers its own update too: `SetPKMode` calls `UpdatePacket`, which
//! `PacketAround`s, and the caller is in its own view.

use common::vid::Vid;

use crate::command::one_argument;
use crate::game_state::GameState;
use crate::loading_phase::{
    PK_MODE_FREE, PK_MODE_GUILD, PK_MODE_MAX_NUM, PK_MODE_PEACE, PK_MODE_PROTECT,
};

/// Meley's map (`MeleyLair.h:15`): `/pkmode` on it always lands `GUILD`.
const MELEY_MAP: i32 = 212;

/// `(unsigned char)strtoul(word)` (`common/utils.h:20-26`): the unsigned walk's low
/// byte. A `-` negates into the wrap, so `-1` reads 255. The walk is a `u8`
/// throughout, so the C cast's low byte falls out of the wrapping arithmetic.
/// The word arrives space-stripped (`two_arguments`), so no space skip.
pub(super) fn parse_mode(word: &[u8]) -> u8 {
    let (digits, negative) = match word.first() {
        Some(b'-') => (&word[1..], true),
        Some(b'+') => (&word[1..], false),
        _ => (word, false),
    };
    let mut value: u8 = 0;
    for byte in digits {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(byte - b'0');
    }
    if negative {
        value = value.wrapping_neg();
    }
    value
}

/// Whether `map` is Meley's (`CMgr::IsMeleyMap`, `MeleyLair.cpp:1215-1222`): 212
/// itself, or its 2120000..2130000 private copies.
fn is_meley_map(map: i32) -> bool {
    map == MELEY_MAP || (MELEY_MAP * 10_000..=MELEY_MAP * 10_000 + 10_000).contains(&map)
}

impl GameState {
    /// `ACMD(do_pkmode)`: the line's number becomes the caller's mode.
    pub(super) fn run_pkmode(&mut self, vid: Vid, rest: &[u8]) {
        let (word, _) = one_argument(rest);
        if word.is_empty() {
            return;
        }
        let mode = parse_mode(&word);
        if mode == PK_MODE_PROTECT {
            return;
        }
        let Some(body) = self.bodies.get(&vid) else {
            return;
        };
        let level = body.card.level;
        let current = body.card.pk_mode;
        let map = body.map;
        if level < self.protect_level && mode != PK_MODE_PEACE {
            return;
        }
        if current == mode {
            return;
        }
        if mode >= PK_MODE_MAX_NUM {
            return;
        }
        let mut mode = mode;
        // `SetPKMode`'s folds (`G/char_battle.cpp:3604-3610`): Meley's map forces
        // `GUILD`, and `GUILD` without a guild falls back to `FREE`.
        if is_meley_map(map) && mode != PK_MODE_GUILD {
            mode = PK_MODE_GUILD;
        }
        if mode == PK_MODE_GUILD {
            let pid = self
                .characters
                .find_by_vid(vid)
                .map_or(0, world::character::Character::player_id);
            if self.guild_of(pid).is_none() {
                mode = PK_MODE_FREE;
            }
        }
        if current == mode {
            return;
        }
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.card.pk_mode = mode;
        }
        let flags = self.affect_flags_of(vid);
        self.send_affect_update(vid, flags);
    }
}

#[cfg(test)]
mod tests {
    use super::{is_meley_map, parse_mode};
    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use crate::loading_phase::{PK_MODE_FREE, PK_MODE_PEACE};
    use common::vid::Vid;
    use protocol::gc_actors::GcCharacterUpdate;

    const VID: u32 = 0x6001;

    /// The unsigned walk's low byte: the sign, the digit run, and the wrap.
    #[test]
    fn the_mode_number_reads_strtoul_low_byte() {
        assert_eq!(parse_mode(b"2"), 2);
        assert_eq!(parse_mode(b"+2"), 2);
        assert_eq!(parse_mode(b"2x"), 2, "the run ends at the letter");
        assert_eq!(parse_mode(b""), 0, "no digits is 0");
        assert_eq!(parse_mode(b"257"), 1, "the low byte");
        assert_eq!(parse_mode(b"-1"), 255, "the negate wraps");
    }

    /// Meley's map: 212 itself and its 2120000..2130000 private copies.
    #[test]
    fn meleys_map_is_212_and_its_copies() {
        assert!(is_meley_map(212));
        assert!(is_meley_map(2_120_000));
        assert!(is_meley_map(2_130_000));
        assert!(!is_meley_map(211));
        assert!(!is_meley_map(213));
        assert!(!is_meley_map(2_119_999));
        assert!(!is_meley_map(2_130_001));
    }

    /// The update of one line: the `bPKMode` it carries, or `None` when silent.
    fn mode_of(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Option<u8> {
        let frame = inbox.try_recv().ok()?;
        let update = GcCharacterUpdate::decode(&frame).expect("an update");
        assert!(inbox.try_recv().is_err(), "one line, one update");
        Some(update.b_pk_mode)
    }

    /// Raise the fixture body (level 10, below the protect level of 15) to 20.
    fn grown(world: &mut crate::game_state::GameState) {
        if let Some(body) = world.bodies.get_mut(&Vid::new(VID)) {
            body.card.level = 20;
        }
    }

    /// A line lands its mode with an update.
    #[test]
    fn a_line_lands_its_mode() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        grown(&mut world);
        world.run_pkmode(Vid::new(VID), b"2");
        assert_eq!(mode_of(&mut inbox), Some(PK_MODE_FREE));
        assert_eq!(world.pk_mode_of(VID), PK_MODE_FREE);
    }

    /// Below the protect level only `PEACE` lands; the rest is silent.
    #[test]
    fn below_protect_only_peace_lands() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        world.run_pkmode(Vid::new(VID), b"2");
        assert_eq!(mode_of(&mut inbox), None, "free is silent below protect");
        world.run_pkmode(Vid::new(VID), b"0");
        assert_eq!(mode_of(&mut inbox), Some(PK_MODE_PEACE));
    }

    /// `PROTECT` never lands from a line, the same mode sends nothing, at and
    /// above `MAX_NUM` the line is refused, and no word runs nothing.
    #[test]
    fn protect_same_past_max_and_empty_are_silent() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        grown(&mut world);
        world.run_pkmode(Vid::new(VID), b"3");
        assert_eq!(mode_of(&mut inbox), None, "protect never lands");
        world.run_pkmode(Vid::new(VID), b"5");
        assert_eq!(mode_of(&mut inbox), None, "max refuses");
        world.run_pkmode(Vid::new(VID), b"255");
        assert_eq!(mode_of(&mut inbox), None, "the wrapped byte refuses");
        // The fixture body starts in `PROTECT`, so landing `FREE` first makes
        // the second `FREE` the same mode.
        world.run_pkmode(Vid::new(VID), b"2");
        assert_eq!(mode_of(&mut inbox), Some(PK_MODE_FREE));
        world.run_pkmode(Vid::new(VID), b"2");
        assert_eq!(mode_of(&mut inbox), None, "the same mode sends nothing");
        world.run_pkmode(Vid::new(VID), b"");
        assert_eq!(mode_of(&mut inbox), None, "no word runs nothing");
    }

    /// `GUILD` without a guild falls back to `FREE`.
    #[test]
    fn guildless_guild_falls_back_to_free() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        grown(&mut world);
        world.run_pkmode(Vid::new(VID), b"4");
        assert_eq!(mode_of(&mut inbox), Some(PK_MODE_FREE));
    }
}
