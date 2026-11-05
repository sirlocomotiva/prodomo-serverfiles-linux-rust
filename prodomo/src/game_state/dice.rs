//! The dice and gift lines: `/dice` and `/gift` (`G/cmd_general.cpp:3251-3254,3455-3489`).

use common::vid::Vid;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use world::character::number;

use super::GameState;
use crate::chat_line::Arg;

/// `do_dice`'s bounds (`G/cmd_general.cpp:3458-3474`): both words parse with `atoi`, one word
/// is 1 to it, neither is 1 to 100, and the pair is ordered low to high.
fn dice_bounds(rest: &[u8]) -> (i32, i32) {
    let (first, back) = crate::command::one_argument(rest);
    let (second, _) = crate::command::one_argument(back);
    let atoi = super::skill_admin::str_to_wrapping_int;
    let (mut start, mut end) = if !first.is_empty() && !second.is_empty() {
        (atoi(&first), atoi(&second))
    } else if !first.is_empty() {
        (1, atoi(&first))
    } else {
        (1, 100)
    };
    // `end = MAX(start, end); start = MIN(start, end)`.
    if end < start {
        core::mem::swap(&mut start, &mut end);
    }
    (start, end)
}

impl GameState {
    /// Runs one `/dice` line (`do_dice`, `G/cmd_general.cpp:3455-3489`).
    ///
    /// The roll goes to the caller's party, or to the caller alone with none. Both arms use
    /// `CHAT_TYPE_INFO`: `ENABLE_DICE_SYSTEM` is undefined on this tree, so the `CHAT_TYPE_DICE_INFO`
    /// arms do not compile. Nothing is stored: the roll is a line, not state.
    pub(super) fn run_dice(&mut self, vid: Vid, rest: &[u8]) {
        let (start, end) = dice_bounds(rest);
        let rolled = number(&mut self.dice, start, end);
        let Some(party) = self.party_of_vid(vid) else {
            let Some(mover) = self.own_mover(vid) else {
                return;
            };
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[LS;1047;%d;%d;%d]",
                &[
                    Arg::Int(i64::from(rolled)),
                    Arg::Int(i64::from(start)),
                    Arg::Int(i64::from(end)),
                ],
            );
            return;
        };
        let name = self
            .characters
            .find_by_vid(vid)
            .map(|character| character.name().as_bytes().to_vec())
            .unwrap_or_default();
        for member in self.parties.linked_vids(party) {
            let Some(mover) = self.own_mover(member) else {
                continue;
            };
            self.tell(
                member,
                mover,
                CHAT_TYPE_INFO,
                b"[LS;1046;%s;%d;%d;%d]",
                &[
                    Arg::Text(&name),
                    Arg::Int(i64::from(rolled)),
                    Arg::Int(i64::from(start)),
                    Arg::Int(i64::from(end)),
                ],
            );
        }
    }

    /// Runs one `/gift` line (`do_gift`, `G/cmd_general.cpp:3251-3254`): the caller hears the
    /// `gift` command line, which opens its gift window. Nothing is stored.
    pub(super) fn run_gift(&mut self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        self.tell(vid, mover, CHAT_TYPE_COMMAND, b"gift", &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::dice_bounds;

    #[test]
    fn the_bounds_parse_both_one_and_neither() {
        assert_eq!(dice_bounds(b""), (1, 100));
        assert_eq!(dice_bounds(b"20"), (1, 20));
        assert_eq!(dice_bounds(b"5 10"), (5, 10));
        assert_eq!(dice_bounds(b"10 5"), (5, 10));
        assert_eq!(dice_bounds(b"abc"), (0, 1));
        assert_eq!(dice_bounds(b"7 abc"), (0, 7));
    }
}
