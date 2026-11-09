//! The exit lines: `/logout`, `/quit` and `/phase_select` (`ACMD(do_cmd)`,
//! `G/cmd_general.cpp:395-448`) and their countdown (`timed_event`, `:320-393`).
//!
//! A line while the character's timed event runs cancels it with `[LS;511]`.
//! Otherwise the line tells its own info line (`[LS;512]` for logout, `[LS;513]`
//! for quit, `[LS;515]` for phase-select) and, past the `IsHack(false, true, 10)`
//! gate, starts the event at 3 seconds. The gate is skipped on a SungZi map
//! (`CThreeWayWar::IsSungZiMapIndex`, read from `forkedmapindex.txt`) and on a
//! war map: legacy's `GetWarMap()->GetType() == GUILD_WAR_TYPE_FLAG` compares
//! the war map's `bType` (0 or 1) against 2, so it never holds and every war-map
//! member skips the gate — the effective rule this build ports.
//!
//! Each fire tells `[LS;510;N]` with the seconds left and counts down, one second
//! on after the first fire one Pulse on (`event_create(timed_event, info, 1)`).
//! At 0 the arm runs: logout closes the descriptor (legacy's `SetPhase(PHASE_CLOSE)`
//! saves through the destroy), quit tells the bare `quit` command line and waits
//! for the client to close, and phase-select disconnects the character with a full
//! save and returns the descriptor to the select phase. The `VALID_LOGOUT` mark
//! only resets the DB login-data's last player id, and `szReason` is stored and
//! never read, so neither is ported.
//!
//! `m_pkTimedEvent` is one slot the channel change (`super::move_channel`) and the
//! private-shop warp share with these lines; the private-shop warp is unported. A
//! disconnect cancels the pending event (`CHARACTER::Disconnect`); a warp keeps it,
//! and the re-entered character (its VID is its id) resumes the countdown.

use common::vid::Vid;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};

use super::GameState;
use crate::chat_line::Arg;
use crate::save::PASSES_PER_SEC;

/// `info->left_second = 3` (`G/cmd_general.cpp:437`): the seconds the lines arm.
const EXIT_SECONDS: u8 = 3;

/// `quit` (`G/cmd_general.cpp:367`): the bare command line the quit arm tells. The
/// client runs it and closes itself; the server waits for the disconnect.
const QUIT_LINE: &[u8] = b"quit";

/// The info line `do_cmd` tells for `kind` before the gate (`:406-416`).
fn exit_notice(kind: crate::game_loop_messages::ExitKind) -> &'static [u8] {
    use crate::game_loop_messages::ExitKind;
    match kind {
        ExitKind::Logout => b"[LS;512]",
        ExitKind::Quit => b"[LS;513]",
        ExitKind::PhaseSelect => b"[LS;515]",
    }
}

/// `m_pkTimedEvent` (`G/char.h`): the character's one timed event. The exit lines
/// and the channel change hold it while their countdown runs; the private-shop warp
/// shares the slot when it is ported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TimedEvent {
    /// One exit line's countdown.
    Exit {
        /// Which arm runs at 0.
        subcmd: crate::game_loop_messages::ExitKind,
        /// The seconds left (`TimedEventInfo::left_second`).
        left: u8,
        /// The Pulse the next fire runs on.
        due: u64,
    },
    /// `/move_channel`'s countdown (`move_channel_info`, `G/char.cpp:8856-8882`).
    MoveChannel {
        /// The Channel the character moves to.
        channel: u8,
        /// The seconds left (`move_channel_info::iSec`).
        left: u8,
        /// The Pulse the next fire runs on.
        due: u64,
    },
    /// The shop warp's countdown (`private_shop_warp_info`, `G/char.cpp:12390-12433`).
    PrivateShopWarp {
        /// The seconds left (`m_bLeftSeconds`).
        left: u8,
        /// The Pulse the next fire runs on.
        due: u64,
    },
}

impl TimedEvent {
    /// The Pulse the next fire runs on.
    fn due(self) -> u64 {
        match self {
            Self::Exit { due, .. }
            | Self::MoveChannel { due, .. }
            | Self::PrivateShopWarp { due, .. } => due,
        }
    }
}

impl GameState {
    /// Runs one exit line: cancel with `[LS;511]` while an event runs, else the
    /// line's own notice and, past the silent `IsHack` gate, the 3-second event.
    /// A caller with no body runs nothing.
    pub(super) fn run_do_cmd(&mut self, vid: Vid, subcmd: crate::game_loop_messages::ExitKind) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        if self.timed_events.remove(&vid).is_some() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;511]", &[]);
            return;
        }
        self.tell(vid, mover, CHAT_TYPE_INFO, exit_notice(subcmd), &[]);
        if self.is_hack(vid, mover).is_some() && !self.on_sungzi_map(vid) && !self.on_war_map(vid) {
            return;
        }
        self.timed_events.insert(
            vid,
            TimedEvent::Exit {
                subcmd,
                left: EXIT_SECONDS,
                due: self.last_pulse.saturating_add(1),
            },
        );
    }

    /// `timed_event` (`G/cmd_general.cpp:320-393`): each due countdown's fire. A
    /// positive second tells `[LS;510;N]` and counts down, one second on; at 0
    /// the arm runs and the event is gone. Entries whose character left the world
    /// are dropped: a disconnect cancels the event, and only a body still here
    /// can fire.
    pub(super) fn run_timed_exits(&mut self, pulse: u64) {
        let due: Vec<Vid> = self
            .timed_events
            .iter()
            .filter_map(|(vid, event)| (pulse >= event.due()).then_some(*vid))
            .collect();
        for vid in due {
            let event = match self.timed_events.get(&vid) {
                Some(&TimedEvent::Exit { subcmd, left, .. }) => (subcmd, left),
                Some(&TimedEvent::MoveChannel { channel, left, .. }) => {
                    self.fire_move_channel(vid, channel, left, pulse);
                    continue;
                }
                Some(&TimedEvent::PrivateShopWarp { left, .. }) => {
                    self.fire_private_shop_warp(vid, left, pulse);
                    continue;
                }
                None => continue,
            };
            let (subcmd, left) = event;
            let Some(mover) = self.own_mover(vid) else {
                self.timed_events.remove(&vid);
                continue;
            };
            if left == 0 {
                self.timed_events.remove(&vid);
                self.run_exit_arm(vid, subcmd);
            } else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"[LS;510;%d]",
                    &[Arg::Int(left.into())],
                );
                self.timed_events.insert(
                    vid,
                    TimedEvent::Exit {
                        subcmd,
                        left: left - 1,
                        due: pulse + u64::from(PASSES_PER_SEC),
                    },
                );
            }
        }
    }

    /// The 0 fire's arm: logout orders the descriptor closed, quit tells the bare
    /// `quit` line, and phase-select orders the descriptor back to select.
    fn run_exit_arm(&mut self, vid: Vid, subcmd: crate::game_loop_messages::ExitKind) {
        use crate::game_loop_messages::ExitKind;
        match subcmd {
            ExitKind::Logout => self.order_close_leases(vid),
            ExitKind::Quit => {
                if let Some(mover) = self.own_mover(vid) {
                    self.tell(vid, mover, CHAT_TYPE_COMMAND, QUIT_LINE, &[]);
                }
            }
            ExitKind::PhaseSelect => self.order_select_leases(vid),
        }
    }

    /// Whether the body stands on a SungZi map (`IsSungZiMapIndex`).
    fn on_sungzi_map(&self, vid: Vid) -> bool {
        self.bodies
            .get(&vid)
            .is_some_and(|body| self.sungzi_maps.contains(body.map))
    }

    /// Whether the character stands on a war map: it is present on one
    /// (`CWarMap::m_set_pkChr`). Legacy's flag-type comparison never holds (the
    /// `bType` is 0 or 1, `GUILD_WAR_TYPE_FLAG` is 2), so every war-map member
    /// skips the gate, on the normal map as on the flag map.
    fn on_war_map(&self, vid: Vid) -> bool {
        self.war_maps.values().any(|map| map.present.contains(&vid))
    }

    /// Order `Close` to every lease holding `vid`: the descriptor leaves the
    /// world with a save and closes, as `SetPhase(PHASE_CLOSE)` with the
    /// destroy's `Disconnect` does.
    fn order_close_leases(&self, vid: Vid) {
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        for (channel, id) in clients.leases_of_vid(vid.raw()) {
            let _sent = clients.order(channel, id, crate::client_registry::ClientOrder::Close);
        }
    }

    /// Order `ToSelect` to every lease holding `vid`: the descriptor leaves the
    /// world with a save and returns to the select phase.
    fn order_select_leases(&self, vid: Vid) {
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        for (channel, id) in clients.leases_of_vid(vid.raw()) {
            let _sent = clients.order(channel, id, crate::client_registry::ClientOrder::ToSelect);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use common::vid::Vid;
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::super::fixtures::{a_world, enter, TestClock};
    use super::super::GameState;
    use crate::client_registry::{ChannelClients, ClientEntry, ClientOrder};
    use crate::game_loop_messages::ExitKind;

    const VID: u32 = 100;

    /// A world with a player past the enter queue.
    fn a_world_with_a_player() -> (GameState, TestClock, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        let _burst: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        (world, clock, inbox)
    }

    /// Drain what one inbox holds.
    fn heard(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The chat text of one `HEADER_GC_CHAT` frame.
    fn text_of(line: &[u8]) -> Vec<u8> {
        let size = usize::from(u16::from_le_bytes([line[1], line[2]]));
        assert_eq!(size, line.len());
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    /// Each line tells its own notice and arms the 3-second event one Pulse on; a
    /// caller with no body runs nothing.
    #[test]
    fn each_line_notices_and_arms_its_countdown() {
        for (kind, notice) in [
            (ExitKind::Logout, b"[LS;512]".as_slice()),
            (ExitKind::Quit, b"[LS;513]".as_slice()),
            (ExitKind::PhaseSelect, b"[LS;515]".as_slice()),
        ] {
            let (mut world, _clock, mut inbox) = a_world_with_a_player();
            world.run_do_cmd(Vid::new(VID), kind);
            assert!(
                heard(&mut inbox)
                    .iter()
                    .map(|line| text_of(line))
                    .any(|text| text == notice),
                "{kind:?} tells its notice"
            );
            assert_eq!(
                world.timed_events.get(&Vid::new(VID)),
                Some(&super::TimedEvent::Exit {
                    subcmd: kind,
                    left: 3,
                    due: world.last_pulse() + 1,
                }),
                "{kind:?} arms the event"
            );
        }
        let (mut world, _clock, mut inbox) = a_world_with_a_player();
        world.run_do_cmd(Vid::new(999), ExitKind::Logout);
        assert!(world.timed_events.is_empty(), "no body runs nothing");
        assert!(heard(&mut inbox).is_empty());
    }

    /// The countdown tells 3, 2 and 1 one second apart, then runs the arm: logout
    /// orders `Close`, quit tells the bare line, and phase-select orders
    /// `ToSelect`.
    #[test]
    fn the_countdown_counts_and_runs_each_arm() {
        for kind in [ExitKind::Logout, ExitKind::Quit, ExitKind::PhaseSelect] {
            let (world, _clock, mut inbox) = a_world_with_a_player();
            let clients = Arc::new(ChannelClients::new());
            let mut world = world.with_clients(Arc::clone(&clients));
            let mut lease = clients.join(ClientEntry {
                channel: 1,
                map: 41,
                name: format!("P{VID}"),
                vid: VID,
                empire: 1,
                language: 0,
            });
            let mut orders = lease.take_orders().expect("the queue is home");
            world.run_do_cmd(Vid::new(VID), kind);
            let _notice = heard(&mut inbox);
            let mut pulse = world.last_pulse() + 1;
            for left in (1..=3).rev() {
                world.run_timed_exits(pulse - 1);
                assert!(
                    world.timed_events.contains_key(&Vid::new(VID)),
                    "the early fire waits"
                );
                world.run_timed_exits(pulse);
                let want = format!("[LS;510;{left}]");
                assert!(
                    heard(&mut inbox)
                        .iter()
                        .map(|line| text_of(line))
                        .any(|text| text == want.as_bytes()),
                    "{kind:?} tells the {left}"
                );
                pulse += u64::from(super::PASSES_PER_SEC);
            }
            world.run_timed_exits(pulse);
            assert!(
                !world.timed_events.contains_key(&Vid::new(VID)),
                "the 0 fire ends the event"
            );
            match kind {
                ExitKind::Logout => {
                    assert_eq!(orders.try_recv().ok(), Some(ClientOrder::Close));
                    assert!(heard(&mut inbox).is_empty(), "logout tells nothing at 0");
                }
                ExitKind::Quit => {
                    assert!(
                        heard(&mut inbox)
                            .iter()
                            .map(|line| text_of(line))
                            .any(|text| text == b"quit"),
                        "quit tells the bare line"
                    );
                    assert!(orders.try_recv().is_err(), "quit orders nothing");
                }
                ExitKind::PhaseSelect => {
                    assert_eq!(orders.try_recv().ok(), Some(ClientOrder::ToSelect));
                    assert!(
                        heard(&mut inbox).is_empty(),
                        "phase-select tells nothing at 0"
                    );
                }
            }
        }
    }

    /// A line while the event runs cancels it with `[LS;511]`: no countdown
    /// follows, whatever the second line names.
    #[test]
    fn a_second_line_cancels_with_511() {
        let (mut world, _clock, mut inbox) = a_world_with_a_player();
        world.run_do_cmd(Vid::new(VID), ExitKind::Logout);
        let _notice = heard(&mut inbox);
        world.run_do_cmd(Vid::new(VID), ExitKind::Quit);
        assert!(
            heard(&mut inbox)
                .iter()
                .map(|line| text_of(line))
                .any(|text| text == b"[LS;511]"),
            "the second line cancels"
        );
        assert!(world.timed_events.is_empty());
        world.run_timed_exits(world.last_pulse() + 1);
        assert!(
            heard(&mut inbox).is_empty(),
            "no countdown follows the cancel"
        );
    }

    /// A hacked caller hears only the notice: the exchange timer holds the event.
    /// A SungZi map and a war map skip the gate, the normal map as the flag map.
    #[test]
    fn the_hack_gate_holds_and_the_maps_skip_it() {
        let (mut world, _clock, mut inbox) = a_world_with_a_player();
        world.set_exchange_time(Vid::new(VID));
        world.run_do_cmd(Vid::new(VID), ExitKind::Logout);
        assert!(
            heard(&mut inbox)
                .iter()
                .map(|line| text_of(line))
                .any(|text| text == b"[LS;512]"),
            "the notice goes out before the gate"
        );
        assert!(
            world.timed_events.is_empty(),
            "the hacked caller starts no event"
        );
        // A SungZi map skips the gate.
        let sungzi = Arc::new(gamedata::forked::SungziMaps::read(b"sungzi: 41\n"));
        let mut world = world.with_sungzi_maps(sungzi);
        world.run_do_cmd(Vid::new(VID), ExitKind::Logout);
        assert!(
            world.timed_events.contains_key(&Vid::new(VID)),
            "the SungZi map skips the gate"
        );
        world.timed_events.clear();
        // A war map skips the gate, whatever its type.
        for map_type in [
            super::super::war_map::WAR_MAP_TYPE_NORMAL,
            super::super::war_map::WAR_MAP_TYPE_FLAG,
        ] {
            let mut war =
                super::super::war_map::WarMap::new(1_100_000, 110, map_type, [7, 9], 0, 0);
            war.present.insert(Vid::new(VID));
            world.war_maps.insert(1_100_000, war);
            world.run_do_cmd(Vid::new(VID), ExitKind::Logout);
            assert!(
                world.timed_events.contains_key(&Vid::new(VID)),
                "the war map of type {map_type} skips the gate"
            );
            world.timed_events.clear();
            world.war_maps.clear();
        }
    }

    /// A disconnect cancels the pending event; a warp keeps it for the return.
    #[test]
    fn a_disconnect_cancels_and_a_warp_keeps() {
        let (mut world, _clock, _inbox) = a_world_with_a_player();
        world.run_do_cmd(Vid::new(VID), ExitKind::Logout);
        assert!(world.timed_events.contains_key(&Vid::new(VID)));
        world.leave_world_by(
            Vid::new(VID),
            crate::game_loop_messages::Departure::Disconnect,
        );
        assert!(world.timed_events.is_empty(), "the disconnect cancels");
        let mut inbox = enter(&mut world, VID, (3200, 3200), 0);
        let _burst: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        world.run_do_cmd(Vid::new(VID), ExitKind::Logout);
        world.leave_world_by(Vid::new(VID), crate::game_loop_messages::Departure::Warp);
        assert!(
            world.timed_events.contains_key(&Vid::new(VID)),
            "the warp keeps the event"
        );
    }
}
