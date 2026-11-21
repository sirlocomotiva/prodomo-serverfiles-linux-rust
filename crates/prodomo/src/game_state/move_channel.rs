//! `/move_channel` (`do_move_channel`, `G/cmd_general.cpp:3644-3697`, `ENABLE_MOVE_CHANNEL`)
//! and its countdown (`move_channel`, `G/char.cpp:8856-8962`).
//!
//! A line while the character's timed event runs cancels it with `[LS;511]`. Otherwise the
//! line refuses, in source order, an empty argument, a Channel outside 1 to 4, the caller's
//! own Channel, the Shared Channel and a dungeon or instance map. The DB's `FindChannel`
//! (`D/ClientManager.cpp:4503-4547`) then looks for a peer of the named Channel that hosts the
//! caller's map; the Rewrite asks the configured topology the same question, and a Channel
//! that does not host the map answers `Cannot change channel` (`CInputDB::MoveChannel`).
//!
//! `StartMoveChannel` refuses silently past `IsHack(false, true, 10)` and starts the event at
//! 3 seconds, or at 10 when the character is fighting. `CanWarp` is the other half of that
//! choice; the Rewrite reads it as `IsHack`, as the quest bridge does, so it always holds once
//! the gate has passed. Each fire tells `CHANGE_CHANNEL_IN_N` and counts down one second; at 0
//! `MoveChannel` runs its own gates again and the descriptor sends `GC_WARP` to the new
//! Channel with the character's own map and position. `MoveChannel` is not `WarpSet`, so the
//! inventory sort's warp lock does not hold it.

use common::config::SHARED_CHANNEL;
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;

use super::gm_lines::int_word;
use super::timed_exit::TimedEvent;
use super::GameState;
use crate::chat_line::Arg;
use crate::client_registry::ClientOrder;
use crate::command::one_argument;
use crate::item_move::Mover;
use crate::loading_phase::INSTANCE_MAP_BASE;
use crate::save::PASSES_PER_SEC;

/// `CanWarp() && !IsPosition(POS_FIGHTING) ? 3 : 10` (`G/char.cpp:8955`).
const CALM_SECONDS: u8 = 3;
/// The seconds a fighting character waits.
const FIGHTING_SECONDS: u8 = 10;

/// `str_to_number` on a `short`: `strtol`, whose low 16 bits the cast keeps. `None` for an
/// empty word.
fn short_word(word: &[u8]) -> Option<i16> {
    let value = int_word(word)?;
    let low = u16::try_from(value & 0xFFFF).unwrap_or(0);
    Some(i16::from_ne_bytes(low.to_ne_bytes()))
}

impl GameState {
    /// Runs `/move_channel` for `vid` with the rest of its line.
    pub(super) fn run_move_channel(&mut self, vid: Vid, mover: Mover, rest: &[u8]) {
        if self.timed_events.remove(&vid).is_some() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;511]", &[]);
            return;
        }
        let (word, _) = one_argument(rest);
        let Some(number) = short_word(&word) else {
            let line = b"Using: channel <channel number>";
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return;
        };
        let Some(channel) = u8::try_from(number).ok().filter(|n| (1..=4).contains(n)) else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Please enter a valid number",
                &[],
            );
            return;
        };
        let Some((here, map)) = self.place_of(vid.raw()) else {
            return;
        };
        if channel == here {
            let line = b"You are already on channel %d";
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[Arg::Int(here.into())]);
            return;
        }
        if here == SHARED_CHANNEL {
            let line = b"You can't change channel in a global map";
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return;
        }
        if map >= INSTANCE_MAP_BASE {
            let line = b"You can't change channel in a dungeon";
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return;
        }
        if !self.recall.hosts(channel, map) {
            let line = b"@@(input_db)tradus:Cannot change channel";
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return;
        }
        if self.is_hack(vid, mover).is_some() {
            return;
        }
        let fighting = self.bodies.get(&vid).is_some_and(|body| body.fighting);
        let left = if fighting {
            FIGHTING_SECONDS
        } else {
            CALM_SECONDS
        };
        self.timed_events.insert(
            vid,
            TimedEvent::MoveChannel {
                channel,
                left,
                due: self.last_pulse.saturating_add(1),
            },
        );
    }

    /// One fire of the countdown: a positive second tells `CHANGE_CHANNEL_IN_N` and waits a
    /// second more; 0 ends the event and runs `MoveChannel`.
    pub(super) fn fire_move_channel(&mut self, vid: Vid, channel: u8, left: u8, pulse: u64) {
        let Some(mover) = self.own_mover(vid) else {
            let _gone = self.timed_events.remove(&vid);
            return;
        };
        if left > 0 {
            let seconds = [Arg::Int(left.into())];
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"CHANGE_CHANNEL_IN_%d",
                &seconds,
            );
            self.timed_events.insert(
                vid,
                TimedEvent::MoveChannel {
                    channel,
                    left: left - 1,
                    due: pulse + u64::from(PASSES_PER_SEC),
                },
            );
            return;
        }
        let _done = self.timed_events.remove(&vid);
        self.move_channel(vid, mover, channel);
    }

    /// `CHARACTER::MoveChannel` (`G/char.cpp:8884-8946`): past `CanWarp` and the dungeon and
    /// Shared Channel refusals, the descriptor leaves the world with a save and sends
    /// `GC_WARP` to the Channel's address with the character's own position.
    fn move_channel(&mut self, vid: Vid, mover: Mover, channel: u8) {
        if self.is_hack(vid, mover).is_some() {
            return;
        }
        let raw = vid.raw();
        let (Some((here, map)), Some(spot)) = (self.place_of(raw), self.spot_of(raw)) else {
            return;
        };
        if map >= INSTANCE_MAP_BASE || here == SHARED_CHANNEL {
            return;
        }
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        let order = ClientOrder::Warp {
            x: spot.x,
            y: spot.y,
            private_map_index: 0,
            channel: Some(channel),
            exit: None,
            sorted_until: 0,
        };
        let ordered = clients
            .members_on_map(here, map)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| clients.order(here, id, order));
        if !ordered {
            tracing::warn!(
                vid = raw,
                channel,
                "a channel change found no client to order"
            );
        }
    }
}
