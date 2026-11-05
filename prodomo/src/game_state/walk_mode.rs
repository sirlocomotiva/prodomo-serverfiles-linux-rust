//! The walk mode lines: `/set_walk_mode` and `/set_run_mode`
//! (`G/cmd_general.cpp:2016-2026`).
//!
//! [`GameState::run_walk_mode`] is `ACMD(do_set_walk_mode)` and
//! `ACMD(do_set_run_mode)`: the line sets both of the caller's flags (`SetWalking`
//! and `SetNowWalking`) to the asked mode (`G/char.h:952-954`). Only the second
//! flag's change fans: `SetNowWalking` (`G/char.cpp:7345-7380`) sends the walk-mode
//! record to the caller's view (`PacketView`) when the flag moves, starts the
//! five-second walk stamp it stamps only on a walk, and says nothing when asked
//! for the mode it holds. Nothing is stored: the flags live in memory, as
//! legacy's do.
//!
//! The flag also feeds `IsWalking` (`G/char.h:951`): a player walks when its
//! flag is set or its stamina is spent, and the move clip and the insert's
//! records read that. The stamina drain and the refill that restores
//! `m_bWalking` (`G/char_state.cpp:813-829`) are not ported, so the restore
//! point never differs from the flag in this build (V3).

use common::vid::Vid;

use crate::game_state::mob_ai::WALKMODE_WALK;
use crate::game_state::view::EntityKey;
use crate::game_state::view_encode::{walk_mode, WALKMODE_RUN};
use crate::game_state::GameState;

impl GameState {
    /// `ACMD(do_set_walk_mode)` and `ACMD(do_set_run_mode)`: the caller walks or
    /// runs, and a change fans its walk-mode record to its view.
    pub(super) fn run_walk_mode(&mut self, vid: Vid, walk: bool) {
        let Some(body) = self.bodies.get_mut(&vid) else {
            return;
        };
        if body.now_walking == walk {
            return;
        }
        body.now_walking = walk;
        let mode = if walk { WALKMODE_WALK } else { WALKMODE_RUN };
        let me = EntityKey::Character(vid.raw());
        self.packet_around(me, &walk_mode(vid.raw(), mode), None);
    }
}

#[cfg(test)]
mod tests {
    use common::vid::Vid;

    use crate::game_state::fixtures::{a_world, enter, TestClock};
    use crate::game_state::mob_ai::WALKMODE_WALK;
    use crate::game_state::view_encode::WALKMODE_RUN;

    const VID: u32 = 7;

    /// Drain the inbox down to the walk-mode records it holds.
    fn modes_of(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut records = Vec::new();
        while let Ok(record) = inbox.try_recv() {
            if record.first() == Some(&0x6f) {
                records.push(record);
            }
        }
        records
    }

    /// `/set_walk_mode` walks with a record to the caller's own view, and
    /// `/set_run_mode` runs with one; either repeated is silent.
    #[test]
    fn a_mode_change_fans_and_a_repeat_is_silent() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 820);
        let _ = modes_of(&mut inbox);
        world.run_walk_mode(Vid::new(VID), true);
        assert_eq!(
            modes_of(&mut inbox),
            [vec![0x6f, 7, 0, 0, 0, WALKMODE_WALK]]
        );
        world.run_walk_mode(Vid::new(VID), true);
        assert!(modes_of(&mut inbox).is_empty(), "the repeat is silent");
        world.run_walk_mode(Vid::new(VID), false);
        assert_eq!(modes_of(&mut inbox), [vec![0x6f, 7, 0, 0, 0, WALKMODE_RUN]]);
        world.run_walk_mode(Vid::new(VID), false);
        assert!(modes_of(&mut inbox).is_empty(), "the repeat is silent");
    }

    /// The line is quiet when the caller is not in the world.
    #[test]
    fn an_unknown_caller_runs_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world.run_walk_mode(Vid::new(VID), true);
    }
}
