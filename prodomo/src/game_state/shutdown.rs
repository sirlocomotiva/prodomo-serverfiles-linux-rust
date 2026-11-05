//! The shutdown line: `/shutdown` (`ACMD(do_shutdown)`,
//! `G/cmd_general.cpp:265-275`) and its countdown (`shutdown_event`, `:198-242`).
//!
//! The line draws every open war map (`CWarMapManager::OnShutdown`, `G/war_map.cpp:1096-1102`),
//! tells the `[LS;509;10]` notice to every client, and starts the countdown at 10 seconds.
//! The GG fan (`HEADER_GG_SHUTDOWN`) is retired with the P2P protocol (ADR-0001): the one
//! process holds every Channel, so one countdown shuts them all.
//!
//! Each tick, one second on, tells the `[LS;508;N]` notice with the seconds left and counts
//! down. The tick at 0 sends every player the `quit` command (`SendDisconnectFunc`,
//! `:153-169`: `FLUSH_AT_SHUTDOWN`, which the owner defines, saves first) and bars new
//! clients (`g_bNoMoreClient`, answered `SHUTDOWN` at auth and login). The countdown runs
//! into the negatives; the tick at -10 force-closes every descriptor (`DisconnectFunc`,
//! `:171-187`), and the tick past it ends. GMs hear the notices but no `quit`: only a
//! `GM_PLAYER` level is sent one.

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_COMMAND;

use super::GameState;
use crate::chat_line::Arg;
use crate::command::GM_PLAYER;
use crate::save::PASSES_PER_SEC;

/// `Shutdown(10)` (`G/cmd_general.cpp:274`): the seconds the line starts the countdown at.
const SHUTDOWN_SECONDS: i32 = 10;

/// The negative second whose tick force-closes every descriptor (`:214`).
const FORCE_CLOSE_SECOND: i32 = -10;

/// `quit Shutdown(SendDisconnectFunc)` (`:166`): the command line that closes a player's
/// client. The client runs the `quit`; the reason rides for the log.
const QUIT_SHUTDOWN: &[u8] = b"quit Shutdown(SendDisconnectFunc)";

impl GameState {
    /// Runs one `/shutdown` line: every open war map draws, the 10's notice goes to
    /// every PC, and the countdown starts, its first tick one Pulse on
    /// (`event_create(shutdown_event, info, 1)`, `:261`: the `when` is Pulses).
    /// A caller with no body runs nothing. Once the bar is set the line restarts
    /// nothing: legacy exits the process there (`thecore_shutdown()`, `:246-249`),
    /// and the game thread must not; the operator stops the process with SIGTERM
    /// (a Divergence, ledger 360). A line while a countdown runs restarts it at
    /// 10: legacy starts a second concurrent event, whose doubled notices this
    /// build does not reproduce (ledger 360).
    pub(super) fn run_shutdown(&mut self, vid: Vid) {
        if self.own_mover(vid).is_none() {
            return;
        }
        if self.metrics.bars_clients() {
            return;
        }
        self.draw_war_maps();
        self.notice_every_pc(b"[LS;509;%d]", &[Arg::Int(SHUTDOWN_SECONDS.into())]);
        self.shutdown_left = Some(SHUTDOWN_SECONDS);
        self.shutdown_due = self.last_pulse.saturating_add(1);
    }

    /// `shutdown_event` (`G/cmd_general.cpp:198-242`): the countdown's tick, when due.
    ///
    /// A positive second tells the `[LS;508;N]` notice and counts down. The 0 tick sends
    /// every player the `quit` command and counts to -1; the -10 tick force-closes every
    /// descriptor and counts to -11, and the tick past -10 ends the countdown. Each tick
    /// that keeps the countdown rearms one second on.
    pub(super) fn run_shutdown_tick(&mut self, pulse: u64) {
        let Some(left) = self.shutdown_left else {
            return;
        };
        if pulse < self.shutdown_due {
            return;
        }
        if left > 0 {
            self.notice_every_pc(b"[LS;508;%d]", &[Arg::Int(left.into())]);
        } else if left == 0 {
            self.quit_every_player();
            self.metrics
                .bars_clients
                .store(true, std::sync::atomic::Ordering::SeqCst);
        } else if left == FORCE_CLOSE_SECOND {
            self.close_every_descriptor();
        } else if left < FORCE_CLOSE_SECOND {
            self.shutdown_left = None;
            return;
        }
        self.shutdown_left = Some(left - 1);
        self.shutdown_due = pulse + u64::from(PASSES_PER_SEC);
    }

    /// `SendDisconnectFunc` (`G/cmd_general.cpp:153-169`): every player hears the `quit`
    /// command. GMs hear nothing: only a `GM_PLAYER` level is sent one.
    fn quit_every_player(&mut self) {
        let mut vids: Vec<Vid> = self.outboxes.keys().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let is_player = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.points())
                .is_some_and(|points| points.gm_level() == GM_PLAYER);
            if is_player {
                if let Some(mover) = self.own_mover(vid) {
                    self.tell(vid, mover, CHAT_TYPE_COMMAND, QUIT_SHUTDOWN, &[]);
                }
            }
        }
    }

    /// `DisconnectFunc` (`G/cmd_general.cpp:171-187`): every descriptor force-closes.
    /// Each client hears the `quit` command, and its descriptor is ordered `Close`:
    /// the departure saves the character, as the destroyed descriptor's did.
    fn close_every_descriptor(&mut self) {
        let mut vids: Vec<Vid> = self.outboxes.keys().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_COMMAND, QUIT_SHUTDOWN, &[]);
            }
        }
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        for vid in self.outboxes.keys().copied().collect::<Vec<Vid>>() {
            let raw = vid.raw();
            for (channel, id) in clients.leases_of_vid(raw) {
                let _sent = clients.order(channel, id, crate::client_registry::ClientOrder::Close);
            }
        }
    }

    /// `CWarMapManager::OnShutdown` (`G/war_map.cpp:1096-1102`): every open war map
    /// draws. Each map's guild pair ends as a forced draw with its tallies, as the
    /// war's end at the map's own arm ends it, and the rows wait for the war-end
    /// poll, as the map's own end queues them.
    fn draw_war_maps(&mut self) {
        let pairs: Vec<(u32, u32)> = self
            .war_maps
            .values()
            .map(|map| {
                let [lo, hi] = map.guilds;
                if lo <= hi {
                    (lo, hi)
                } else {
                    (hi, lo)
                }
            })
            .collect();
        let now = super::quests::quest_now();
        for (lo, hi) in pairs {
            if let Some(outcome) =
                self.manager_end_war(lo, hi, super::guild::WarEndJudgement::ForcedDraw, true, now)
            {
                self.guilds.pending_war_ends.push(outcome);
            }
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
    use crate::command::{GM_HIGH_WIZARD, GM_PLAYER};

    const GM: u32 = 100;
    const PLAYER: u32 = 101;

    /// A world with a GM caller and a player, both past the enter queue.
    fn a_world_with_two() -> (
        GameState,
        TestClock,
        UnboundedReceiver<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut gm = enter(&mut world, GM, (3200, 3200), 0);
        let mut player = enter(&mut world, PLAYER, (3300, 3300), 0);
        let _gm_burst: Vec<Vec<u8>> = std::iter::from_fn(|| gm.try_recv().ok()).collect();
        let _player_burst: Vec<Vec<u8>> = std::iter::from_fn(|| player.try_recv().ok()).collect();
        let mut points = world
            .characters
            .find_by_vid(Vid::new(GM))
            .expect("the GM is online")
            .points()
            .expect("the GM has points")
            .clone();
        assert_eq!(points.gm_level(), GM_PLAYER, "the entrant is a player");
        points.set_gm_level(GM_HIGH_WIZARD);
        world
            .characters
            .find_by_vid_mut(Vid::new(GM))
            .expect("the GM is online")
            .set_points(Some(points));
        (world, clock, gm, player)
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

    /// The line notices the 10 and starts the countdown one Pulse on; a caller
    /// with no body runs nothing; a line mid-countdown restarts it at 10.
    #[test]
    fn the_line_notices_the_10_and_starts_the_countdown() {
        let (mut world, _clock, mut gm, mut player) = a_world_with_two();
        world.run_shutdown(Vid::new(GM));
        assert_eq!(world.shutdown_left, Some(10));
        assert_eq!(world.shutdown_due, world.last_pulse() + 1);
        assert!(
            heard(&mut gm)
                .iter()
                .map(|line| text_of(line))
                .any(|text| text == b"[LS;509;10]"),
            "the GM hears the 10's notice"
        );
        assert!(
            heard(&mut player)
                .iter()
                .map(|line| text_of(line))
                .any(|text| text == b"[LS;509;10]"),
            "the player hears the 10's notice"
        );
        world.run_shutdown(Vid::new(999));
        assert_eq!(world.shutdown_left, Some(10), "no body runs nothing");
        assert!(heard(&mut gm).is_empty());
        world.run_shutdown_tick(world.shutdown_due);
        assert_eq!(world.shutdown_left, Some(9));
        let _nine = heard(&mut gm);
        world.run_shutdown(Vid::new(GM));
        assert_eq!(
            world.shutdown_left,
            Some(10),
            "the line restarts the countdown"
        );
        assert!(
            heard(&mut gm)
                .iter()
                .map(|line| text_of(line))
                .any(|text| text == b"[LS;509;10]"),
            "the restart notices the 10 again"
        );
    }

    /// The countdown notices each second down, quits the players at 0 with the bar,
    /// and force-closes every descriptor at -10 with a `Close` order each.
    #[test]
    fn the_countdown_notices_quits_bars_and_force_closes() {
        let (world, _clock, mut gm, mut player) = a_world_with_two();
        let clients = Arc::new(ChannelClients::new());
        let mut world = world.with_clients(Arc::clone(&clients));
        let mut gm_lease = clients.join(ClientEntry {
            channel: 1,
            map: 41,
            name: format!("P{GM}"),
            vid: GM,
            empire: 1,
            language: 0,
        });
        let mut gm_orders = gm_lease.take_orders().expect("the queue is home");
        let mut player_lease = clients.join(ClientEntry {
            channel: 1,
            map: 41,
            name: format!("P{PLAYER}"),
            vid: PLAYER,
            empire: 1,
            language: 0,
        });
        let mut player_orders = player_lease.take_orders().expect("the queue is home");
        world.run_shutdown(Vid::new(GM));
        let _tens = heard(&mut gm);
        let _tens = heard(&mut player);
        let mut pulse = world.last_pulse() + 1;
        for left in (1..=10).rev() {
            world.run_shutdown_tick(pulse - 1);
            assert_eq!(world.shutdown_left, Some(left), "the early tick waits");
            world.run_shutdown_tick(pulse);
            assert_eq!(world.shutdown_left, Some(left - 1));
            let want = format!("[LS;508;{left}]");
            assert!(
                heard(&mut gm)
                    .iter()
                    .map(|line| text_of(line))
                    .any(|text| text == want.as_bytes()),
                "the GM hears the {left}'s notice"
            );
            assert!(
                heard(&mut player)
                    .iter()
                    .map(|line| text_of(line))
                    .any(|text| text == want.as_bytes()),
                "the player hears the {left}'s notice"
            );
            pulse += u64::from(super::PASSES_PER_SEC);
        }
        assert!(!world.metrics.bars_clients(), "no bar before the 0 tick");
        world.run_shutdown_tick(pulse);
        assert_eq!(world.shutdown_left, Some(-1));
        assert!(world.metrics.bars_clients(), "the 0 tick bars new clients");
        assert!(
            heard(&mut gm).is_empty(),
            "the GM hears no quit at the 0 tick"
        );
        assert!(
            heard(&mut player)
                .iter()
                .map(|line| text_of(line))
                .any(|text| text == b"quit Shutdown(SendDisconnectFunc)"),
            "the player hears the quit at the 0 tick"
        );
        for _ in -9..0 {
            pulse += u64::from(super::PASSES_PER_SEC);
            world.run_shutdown_tick(pulse);
        }
        assert_eq!(world.shutdown_left, Some(-10));
        pulse += u64::from(super::PASSES_PER_SEC);
        world.run_shutdown_tick(pulse);
        assert_eq!(world.shutdown_left, Some(-11));
        for (inbox, who) in [(&mut gm, "the GM"), (&mut player, "the player")] {
            assert!(
                heard(inbox)
                    .iter()
                    .map(|line| text_of(line))
                    .any(|text| text == b"quit Shutdown(SendDisconnectFunc)"),
                "{who} hears the quit at the force-close"
            );
        }
        assert_eq!(gm_orders.try_recv().ok(), Some(ClientOrder::Close));
        assert_eq!(player_orders.try_recv().ok(), Some(ClientOrder::Close));
        pulse += u64::from(super::PASSES_PER_SEC);
        world.run_shutdown_tick(pulse);
        assert_eq!(world.shutdown_left, None, "the tick past -10 ends it");
    }

    /// A line after the bar restarts nothing: legacy exits the process there, and
    /// the game thread must not.
    #[test]
    fn a_line_after_the_bar_restarts_nothing() {
        let (mut world, _clock, mut gm, _player) = a_world_with_two();
        world.run_shutdown(Vid::new(GM));
        let mut pulse = world.last_pulse() + 1;
        for _ in 0..=10 {
            world.run_shutdown_tick(pulse);
            pulse += u64::from(super::PASSES_PER_SEC);
        }
        assert!(world.metrics.bars_clients());
        let _burst = heard(&mut gm);
        world.run_shutdown(Vid::new(GM));
        assert_eq!(
            world.shutdown_left,
            Some(-1),
            "the countdown is not restarted"
        );
        assert!(heard(&mut gm).is_empty(), "no second 10's notice goes out");
    }
}
