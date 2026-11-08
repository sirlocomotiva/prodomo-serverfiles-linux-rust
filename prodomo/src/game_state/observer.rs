//! Observer mode (`m_bIsObserver`, `G/entity.h:60`).
//!
//! `/observer` (`do_observer`, `G/cmd_gm.cpp:3226-3229`, `GM_IMPLEMENTOR`) toggles the flag;
//! `/observer_exit` (`do_observer_exit`, `G/cmd_general.cpp:2370-2393`, `GM_PLAYER`) leaves it.
//! A war map's watchers observe without a line (`CWarMap::IncMember`, `G/war_map.cpp:384`).
//!
//! `SetObserverMode` (`G/entity.cpp:107-118`) sets the flag and recomputes the view with the
//! change dance (the view layer's [`super::view::MapIndex::set_observer`]), then tells the
//! character `ObserverMode 1` or `ObserverMode 0` as a `CHAT_TYPE_COMMAND` line. While the
//! flag stands, the character's inserts, removals and re-encodes are never sent, its
//! `PacketAround` reaches only itself, and the descriptor refuses its item, exchange, attack
//! and skill records (`CInputMain::Analyze`, `G/input_main.cpp:3690-3740`).
//!
//! The flag lives in the view layer, beside the views it gates. The descriptor holds its own
//! copy for the record gates; the two are kept in step: `/observer` flips both (the line
//! cannot fail, so a pure toggle cannot desync), a war map's watch sets both at admission
//! (the flag rides the enter records), and leaving the world clears both.
//!
//! No owner quest calls `arena.add_observer` (ledger 469), so an arena visitor
//! never enters the observer roster. Its `/observer_exit` takes the plain branch:
//! `ExitToSavedLocation` (`G/char.cpp:6685-6692`), which warps to the pending Warp, then
//! clears the mode. The pending Warp is the descriptor's (`held.warp`), and the warp's own
//! outcome decides what the clear does: after a refused warp the character still stands, so
//! the clear dances it back into view before telling the line; after an accepted warp the
//! character is gone, so the clear is silent and the descriptor sends the line itself, after
//! `GC_WARP`, as legacy's `SetObserverMode` does. Two commands carry this out: `ObserverExit`
//! answers the sort lock when the character observes, and `ObserverSettled` runs the clear.

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_COMMAND;

use super::view::EntityKey;
use super::GameState;
use crate::chat_line::Arg;

impl GameState {
    /// Whether the character under `vid` observes: the view layer's flag.
    pub(super) fn is_observing(&self, vid: Vid) -> bool {
        self.place_of(vid.raw())
            .and_then(|place| self.maps.get(&place))
            .is_some_and(|index| index.is_observer(EntityKey::Character(vid.raw())))
    }

    /// `CEntity::SetObserverMode` (`G/entity.cpp:107-118`) for the character under `vid`: the
    /// flag, the dance, and the `ObserverMode %d` command line. Same value changes nothing
    /// (`:109-110`), and a caller with no body runs nothing.
    pub(super) fn set_observer_mode(&mut self, vid: Vid, flag: bool) {
        let Some(place) = self.place_of(vid.raw()) else {
            return;
        };
        if self.is_observing(vid) == flag {
            return;
        }
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&place) {
            index.set_observer(
                EntityKey::Character(vid.raw()),
                flag,
                self.view_radius,
                &mut effects,
            );
        }
        self.deliver(&effects, None);
        if let Some(mover) = self.own_mover(vid) {
            let mode = [Arg::Int(i64::from(u8::from(flag)))];
            self.tell(vid, mover, CHAT_TYPE_COMMAND, b"ObserverMode %d", &mode);
        }
    }

    /// `/observer`: the toggle. The descriptor flips its own copy when it sends the line.
    pub(super) fn run_observer(&mut self, vid: Vid) {
        let flag = !self.is_observing(vid);
        // A caller with no body runs nothing; without the check the toggle would tell the
        // line to a client the world cannot hear.
        if self.place_of(vid.raw()).is_none() {
            return;
        }
        self.set_observer_mode(vid, flag);
    }

    /// `ObserverExit`'s answer: the sort lock when the character under `vid` observes, else
    /// `None`. Nothing is set yet; the descriptor runs the warp first.
    pub(super) fn observer_exit_lock(&self, vid: Vid) -> Option<u32> {
        self.is_observing(vid).then(|| self.sort_lock(vid))
    }

    /// `ObserverSettled`: the exit's clear. After a refused warp the character still stands,
    /// so the clear dances it back into view before telling the line; after an accepted warp
    /// the character is gone and there is nothing to clear — the descriptor sent the line
    /// itself. A caller with no body runs nothing.
    pub(super) fn observer_settled(&mut self, vid: Vid) {
        if self.place_of(vid.raw()).is_none() {
            return;
        }
        if !self.is_observing(vid) {
            return;
        }
        self.set_observer_mode(vid, false);
    }

    /// `CWarMap::DecMember`'s watcher arm (`G/war_map.cpp:440-447`): the watch ends, so the
    /// mode ends with the dance, while the watcher still stands. The `ObserverMode 0` line
    /// goes nowhere: the leaver's outbox is already gone, and legacy sends it to a
    /// descriptor that is closing.
    pub(super) fn end_war_watch(&mut self, vid: Vid) {
        let Some(place) = self.place_of(vid.raw()) else {
            return;
        };
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&place) {
            index.set_observer(
                EntityKey::Character(vid.raw()),
                false,
                self.view_radius,
                &mut effects,
            );
        }
        self.deliver(&effects, None);
    }
}
