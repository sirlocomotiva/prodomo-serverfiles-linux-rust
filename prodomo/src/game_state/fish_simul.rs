//! The fishing simulation line: `/fish_simul` (`ACMD(do_fishing_simul)`,
//! `G/cmd_gm.cpp:2227-2251`).
//!
//! The line always tells the usage first (`Usage: fishing_simul <level> <prob index>
//! <count>`, untranslated: the call passes no `LC_TEXT`), then reads up to three words as
//! `level` (100), `prob_idx` (0) and `count` (1000), each `str_to_number(int)` of its word
//! when the word is non-empty. Then `fishing::Simulation` (`G/fishing.cpp:747-769`) runs
//! `count` draws: each picks a fish by the `prob_idx` table (`DetermineFishByProbIndex`,
//! `:330-336`) and keeps it when `Compute` (`:549-575`) answers an item, and each kept
//! fish's lowercased name counts one. The lines go out in the map's byte order (`std::map`,
//! one `%s : %d fish` per name), then `%d types %d fish caught` with the name count and
//! the kept count.
//!
//! `Compute` refuses fish 0 and a fish past the table (both `sys_err`'d, answered `-2`),
//! refuses a time past 6000ms (`-1`), rolls the time step's chance from `aFishingTime`
//! (`:73-80`) at `MINMAX(0, (ms + 99) / 200, 30)`, and rolls `number(1, difficulty) <=
//! level` (`-3` on a miss). The simulation's time is the mean of two `number(2000,
//! 4000)` draws. Only an answer of 0 with a nonzero item keeps the fish.
//!
//! A `prob_idx` past the tables is a Defect: legacy indexes `g_prob_sum` and
//! `g_prob_accumulate` without a bound check. The Rewrite refuses past-end tables before
//! the first draw and tells only the usage, closing nothing.

use common::vid::Vid;
use gamedata::fishing::MAX_FISH;
use protocol::gc_chat::CHAT_TYPE_INFO;
use world::character::Dice;

use super::GameState;
use crate::chat_line::Arg;

impl GameState {
    /// Share the fishing table boot read. Without it `/fish_simul` tells only
    /// the usage and the empty summary.
    #[must_use]
    pub fn with_fishing(mut self, fishing: std::sync::Arc<gamedata::fishing::Fishing>) -> Self {
        self.fishing = fishing;
        self
    }
}

/// `do_fishing_simul`'s defaults: level 100, table 0, 1000 draws.
const DEFAULT_LEVEL: i32 = 100;
const DEFAULT_PROB_IDX: i32 = 0;
const DEFAULT_COUNT: i32 = 1000;

impl GameState {
    /// Runs one `/fish_simul` line: the usage, then the simulation's lines.
    pub(super) fn run_fish_simul(&mut self, vid: Vid, rest: &[u8]) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"Usage: fishing_simul <level> <prob index> <count>",
            &[],
        );
        let (level, prob_idx, count) = simul_args(rest);
        let Ok(prob) = usize::try_from(prob_idx) else {
            return;
        };
        let fishing = std::sync::Arc::clone(&self.fishing);
        let (Some(table), Some(sum)) = (fishing.accumulate(prob), fishing.sum(prob)) else {
            return;
        };
        let mut kept: std::collections::BTreeMap<Vec<u8>, i32> = std::collections::BTreeMap::new();
        let mut total = 0i32;
        for _ in 0..count.max(0) {
            // A draw past the table's last bound is `std::lower_bound`'s end, which
            // `Compute` refuses.
            let id = determine(&mut self.dice, table, sum).unwrap_or(MAX_FISH);
            // The time is drawn as `Compute`'s argument, before any of its guards.
            let ms = (draw(&mut self.dice, 2000, 4000) + draw(&mut self.dice, 2000, 4000)) / 2;
            let ms = u32::try_from(ms).unwrap_or(0);
            let caught = super::fishing::compute(&mut self.dice, fishing.fish(), id, ms, level);
            // Only an item keeps the fish (`if (item)`, `G/fishing.cpp:758`).
            let Some(name) = caught
                .ok()
                .filter(|&vnum| vnum != 0)
                .and_then(|_| fishing.fish().get(id))
                .map(|one| one.name.clone())
            else {
                continue;
            };
            *kept.entry(name).or_insert(0) += 1;
            total += 1;
        }
        for (name, count) in &kept {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"%s : %d fish",
                &[Arg::Text(name), Arg::Int(i64::from(*count))],
            );
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"%d types %d fish caught",
            &[
                Arg::Int(i64::try_from(kept.len()).unwrap_or(i64::MAX)),
                Arg::Int(i64::from(total)),
            ],
        );
    }
}

/// The line's three words as `level`, `prob_idx` and `count`, each defaulted when its word
/// is empty (`G/cmd_gm.cpp:2241-2248`).
fn simul_args(rest: &[u8]) -> (i32, i32, i32) {
    let (first, back) = crate::command::one_argument(rest);
    let (second, back) = crate::command::one_argument(back);
    let (third, _) = crate::command::one_argument(back);
    let atoi = super::skill_admin::str_to_wrapping_int;
    let level = if first.is_empty() {
        DEFAULT_LEVEL
    } else {
        atoi(&first)
    };
    let prob_idx = if second.is_empty() {
        DEFAULT_PROB_IDX
    } else {
        atoi(&second)
    };
    let count = if third.is_empty() {
        DEFAULT_COUNT
    } else {
        atoi(&third)
    };
    (level, prob_idx, count)
}

/// `DetermineFishByProbIndex` (`G/fishing.cpp:330-336`): the first fish whose accumulated
/// probability reaches the draw, or none when the draw overshoots a short table.
fn determine(dice: &mut dyn Dice, table: &[i32; 37], sum: i32) -> Option<usize> {
    let rv = draw(dice, 1, sum);
    table.iter().position(|bound| rv <= *bound)
}

/// `number(from, to)` on the game's generator.
fn draw(dice: &mut dyn Dice, from: i32, to: i32) -> i32 {
    world::character::number(dice, from, to)
}

#[cfg(test)]
mod tests {
    use super::simul_args;

    #[test]
    fn the_words_default_level_prob_and_count() {
        assert_eq!(simul_args(b""), (100, 0, 1000));
        assert_eq!(simul_args(b"50"), (50, 0, 1000));
        assert_eq!(simul_args(b"50 2"), (50, 2, 1000));
        assert_eq!(simul_args(b"50 2 10"), (50, 2, 10));
        assert_eq!(simul_args(b"abc"), (0, 0, 1000));
    }
}
