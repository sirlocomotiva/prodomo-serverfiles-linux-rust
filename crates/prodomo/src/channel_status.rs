//! The Channel status list: the answer to `HEADER_CG_STATE_CHECKER`.
//!
//! Legacy spreads this over three processes. Every game Core reports its own port and a status to
//! the DB server (`CLIENT_DESC::UpdateChannelStatus`, `G/desc_client.cpp:292-313`) when it boots
//! and then every five minutes; the DB server keeps the last report per port
//! (`D/ClientManager.cpp:4442-4453`); and a descriptor in the handshake phase asks the DB server
//! for the whole list (`G/input.cpp:227-234`), which `CInputDB::RespondChannelStatus` relays.
//!
//! The Rewrite hosts every Channel in one process (ADR-0002), so the board answers from the
//! configured Channel ports and the current count at the moment of the request. Two recorded
//! Divergences follow: the status is never up to five minutes stale, and the entries are in port
//! order, where legacy used the iteration order of an `unordered_map`.

use std::sync::atomic::{AtomicI32, Ordering};

use common::config::GameSettings;
use protocol::gc_channel_status::{encode_channel_status, ChannelStatusEntry};

/// `bStatus` when the server takes no more clients (`g_bNoMoreClient`).
pub const STATUS_CLOSED: u8 = 0;
/// `bStatus` at or below the busy count.
pub const STATUS_NORMAL: u8 = 1;
/// `bStatus` above the busy count.
pub const STATUS_BUSY: u8 = 2;
/// `bStatus` above the full count.
pub const STATUS_FULL: u8 = 3;

/// The legacy status rule (`G/desc_client.cpp:306-307`): `iTotal > g_iFullUserCount ? 3 :
/// iTotal > g_iBusyUserCount ? 2 : 1`, all `int`.
///
/// `online` is every character in game on the whole server: legacy adds the P2P count, and the DB
/// server introduces every game Core to every other one, whatever its Channel
/// (`D/ClientManager.cpp:1415-1433`). Both comparisons are strict, and a negative
/// threshold keeps every non-negative count above it.
#[must_use]
pub const fn channel_status(online: i32, no_more_clients: bool, busy: i32, full: i32) -> u8 {
    if no_more_clients {
        STATUS_CLOSED
    } else if online > full {
        STATUS_FULL
    } else if online > busy {
        STATUS_BUSY
    } else {
        STATUS_NORMAL
    }
}

/// The Channel ports and the counts the status list is built from.
#[derive(Debug)]
pub struct ChannelStatusBoard {
    ports: Vec<u16>,
    busy: AtomicI32,
    full: AtomicI32,
    shutdowned: bool,
}

impl ChannelStatusBoard {
    /// A board for the bound Channel ports, with the thresholds and the `shutdowned` switch
    /// from `[game]`. Ports are listed once each, in ascending order.
    #[must_use]
    pub fn new(ports: impl IntoIterator<Item = u16>, game: &GameSettings) -> Self {
        let mut ports: Vec<u16> = ports.into_iter().collect();
        ports.sort_unstable();
        ports.dedup();
        Self {
            ports,
            busy: AtomicI32::new(i32::try_from(game.busy_user_count).unwrap_or(i32::MAX)),
            full: AtomicI32::new(i32::try_from(game.full_user_count).unwrap_or(i32::MAX)),
            shutdowned: game.shutdowned,
        }
    }

    /// Reread the two thresholds (`/reload u`, `LoadStateUserCount`,
    /// `G/config.cpp:1778-1788`), which the next status list answers from.
    pub fn set_counts(&self, busy: i32, full: i32) {
        self.busy.store(busy, Ordering::Relaxed);
        self.full.store(full, Ordering::Relaxed);
    }

    /// Reread the `full` threshold alone: the half of [`Self::set_counts`] a lone
    /// first word in `state_user_count` moves, while `busy` stands.
    pub fn set_full(&self, full: i32) {
        self.full.store(full, Ordering::Relaxed);
    }

    /// The `HEADER_GC_RESPOND_CHANNELSTATUS` record: `online` is every character in game
    /// on the whole server, which the caller counts fresh (legacy caches it on a
    /// five-second tick and adds the P2P cores'). Every port shares one status, as in
    /// legacy, where every Core computes it from the same server-wide count.
    #[must_use]
    pub fn respond(&self, online: i32, shutting_down: bool) -> Vec<u8> {
        let status = channel_status(
            online,
            self.shutdowned || shutting_down,
            self.busy.load(Ordering::Relaxed),
            self.full.load(Ordering::Relaxed),
        );
        let entries: Vec<ChannelStatusEntry> = self
            .ports
            .iter()
            .map(|&port| ChannelStatusEntry { port, status })
            .collect();
        encode_channel_status(&entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_follows_the_strict_legacy_thresholds() {
        assert_eq!(channel_status(0, false, 650, 1200), STATUS_NORMAL);
        assert_eq!(channel_status(650, false, 650, 1200), STATUS_NORMAL);
        assert_eq!(channel_status(651, false, 650, 1200), STATUS_BUSY);
        assert_eq!(channel_status(1200, false, 650, 1200), STATUS_BUSY);
        assert_eq!(channel_status(1201, false, 650, 1200), STATUS_FULL);
        assert_eq!(channel_status(1201, true, 650, 1200), STATUS_CLOSED);
        // Signed, as the `int` globals are: a negative threshold keeps even an
        // empty server above it.
        assert_eq!(channel_status(0, false, -1, 1200), STATUS_BUSY);
        assert_eq!(channel_status(0, false, -1, -1), STATUS_FULL);
        assert_eq!(channel_status(0, true, 650, 1200), STATUS_CLOSED);
    }

    #[test]
    fn the_board_lists_each_port_once_in_order_with_one_status() {
        let board = ChannelStatusBoard::new([30005, 30003, 30019, 30003], &GameSettings::default());
        assert_eq!(
            board.respond(0, false),
            [
                0xd2, 3, 0, 0, 0, // header and count
                0x33, 0x75, 1, // 30003
                0x35, 0x75, 1, // 30005
                0x43, 0x75, 1, // 30019
                1,
            ]
        );
        assert_eq!(board.respond(651, false)[7], STATUS_BUSY);
        assert_eq!(board.respond(651, true)[7], STATUS_CLOSED);
    }

    #[test]
    fn the_shutdowned_switch_closes_every_channel() {
        let game = GameSettings {
            shutdowned: true,
            ..GameSettings::default()
        };
        let board = ChannelStatusBoard::new([30003], &game);
        assert_eq!(board.respond(0, false)[7], STATUS_CLOSED);
    }
}
