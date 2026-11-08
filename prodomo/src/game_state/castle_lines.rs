//! The castle GM lines: `/siege` (`do_siege`, `G/cmd_gm.cpp:4336-4365`) and `/frog`
//! (`do_frog`, `G/cmd_gm.cpp:4390-4419`).
//!
//! Legacy runs the lines against the castle maps 181 to 183 (`IS_CASTLE_MAP`,
//! `G/castle.h:22`). `/siege` parses its empire and tower words, sends the GG notice
//! (retired with the game-to-game protocol: the one process runs the siege once), and
//! asks `castle_siege`, which returns 0 with no word spoken when any of the three maps
//! is missing (`G/castle.cpp:565-572`) and the caller then hears `SIEGE FAILED`.
//! `/frog` answers `Usage: empire(1, 2, 3)` for a missing or out-of-range empire, and
//! a caller off the castle maps hears `You must spawn frog in castle`.
//!
//! None of 181-183 is in the owner's map index, and no Channel hosts one, so the lines
//! refuse on the owner's data: `/siege` always answers `SIEGE FAILED` and `/frog`
//! always answers the usage or the must-spawn line. The parse still runs first: the
//! empire word reads 0 when empty or garbage, a 0 draws 1-3, and a tower word outside
//! 5-10 draws 5-10. The siege itself (`G/castle.cpp`), its two timers and the remains
//! wait for the maps (ledger 471).

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use world::character::number;

use super::gm_lines::int_word;
use super::GameState;
use crate::command::{one_argument, two_arguments};
use crate::item_move::Mover;

/// The castle maps (`IS_CASTLE_MAP`, `G/castle.h:22`).
const CASTLE_MAPS: [i32; 3] = [181, 182, 183];

impl GameState {
    /// `do_siege`: the empire and tower words parse, the draws run, and the siege
    /// attempt fails on the missing maps, so the caller hears `SIEGE FAILED`.
    pub(super) fn run_siege_line(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        let (first, second) = two_arguments(rest);
        let mut empire = int_word(&first).unwrap_or(0);
        if empire == 0 {
            empire = number(&mut self.dice, 1, 3);
        }
        let mut towers = int_word(&second).unwrap_or(0);
        if !(5..=10).contains(&towers) {
            towers = number(&mut self.dice, 5, 10);
        }
        let _ = (empire, towers);
        if !CASTLE_MAPS
            .iter()
            .all(|map| !self.places_hosting(*map).is_empty())
        {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"SIEGE FAILED", &[]);
        }
    }

    /// `do_frog`: the usage answers a missing or out-of-range empire, else a caller
    /// off the castle maps hears the must-spawn line.
    pub(super) fn run_frog_line(&self, vid: Vid, rest: &[u8], mover: Mover) {
        let (word, _) = one_argument(rest);
        if word.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Usage: empire(1, 2, 3)", &[]);
            return;
        }
        let empire = int_word(&word).unwrap_or(0);
        if !(1..=3).contains(&empire) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Usage: empire(1, 2, 3)", &[]);
            return;
        }
        let on_castle = self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| CASTLE_MAPS.contains(&map));
        if !on_castle {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You must spawn frog in castle",
                &[],
            );
        }
    }
}
