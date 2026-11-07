//! The premium players (`__ENABLE_PREMIUM_PLAYERS__`, `G/char.cpp:11448-11600`).
//!
//! The flag and its timer ride the points (`m_byPremium`, `m_iPremiumTime`, stored with the
//! row): 1 while premium runs, and the `time(0)` it ends at. An activation spends two MD
//! vouchers (80015), chats the notice to the Channel and the line to the player, sets the
//! flag with a 30-day timer, and lays the four affects (549-552): 2000 max HP, 15 monster,
//! 10 boss, 10 metin bonus. The expiry takes the flag and the affects off with the time-up
//! line. The open answers its window, the list names the flagged players highest level
//! first (at most a hundred); both are answered by the descriptor, which owns the store
//! read and the socket.
//!
//! # Order
//!
//! The activation answers the notice, then the line, before the flag moves
//! (`:11551-11558`); the viewer update of the affects' flags goes through the world's
//! own send, as `UpdatePacket` goes `PacketAround`. The expiry answers nothing but the
//! time-up line after the affects leave (`:11523-11525`).
//!
//! # Defects not reproduced
//!
//! - An activation creates the voucher item to read its name and leaks it
//!   (`ITEM_MANAGER::instance().CreateItem(itemVnum)` at `:11542` with no destroy). The
//!   Rewrite reads the proto's locale name instead.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::point_slot;
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;

use super::affect::Affect;
use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};
use crate::loading_phase::{
    AFFECT_PREMIUM_PLAYERS_01, AFFECT_PREMIUM_PLAYERS_02, AFFECT_PREMIUM_PLAYERS_03,
    AFFECT_PREMIUM_PLAYERS_04,
};

/// `PREMIUM_PLAYERS_SUBHEADER_CG_OPEN`: answer the window.
pub const PREMIUM_OPEN: u8 = 0;
/// `PREMIUM_PLAYERS_SUBHEADER_CG_LIST`: name the flagged players.
pub const PREMIUM_LIST: u8 = 1;
/// `PREMIUM_PLAYERS_SUBHEADER_CG_ACTIVATE`: spend two vouchers and start the premium.
pub const PREMIUM_ACTIVATE: u8 = 2;

/// `PREMIUM_PLAYERS_SUBHEADER_GC_OPEN`: the window opener.
pub const PREMIUM_GC_OPEN: u8 = 0;
/// `PREMIUM_PLAYERS_SUBHEADER_GC_LIST`: one flagged name.
pub const PREMIUM_GC_LIST: u8 = 1;

/// The voucher the activation spends (`G/char.cpp:11537`): the MD voucher (500), twice.
pub const PREMIUM_VOUCHER: u32 = 80015;
/// The activation's timer, in seconds (`SetPremiumPlayerTimer(2592000)`, `:11558`): 30 days.
pub const PREMIUM_DURATION: i32 = 2_592_000;
/// The premium affects' duration (`:11567-11573`): a year, which the expiry cuts short.
pub const PREMIUM_AFFECT_DURATION: i32 = 60 * 60 * 24 * 365;

/// The four affects an activation lays, in legacy's order (`G/char.cpp:11563-11576`): the
/// kind, the point and the value.
pub const PREMIUM_AFFECTS: [(u32, usize, i32); 4] = [
    (AFFECT_PREMIUM_PLAYERS_01, point_slot::POINT_MAX_HP, 2000),
    (
        AFFECT_PREMIUM_PLAYERS_02,
        point_slot::POINT_ATTBONUS_MONSTER,
        15,
    ),
    (
        AFFECT_PREMIUM_PLAYERS_03,
        point_slot::POINT_ATTBONUS_BOSS,
        10,
    ),
    (
        AFFECT_PREMIUM_PLAYERS_04,
        point_slot::POINT_ATTBONUS_METIN,
        10,
    ),
];

/// One decoded `CG_PREMIUM_PLAYERS` request, as `RecvPremiumPlayersPacket` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PremiumStep {
    /// `PREMIUM_PLAYERS_SUBHEADER_CG_OPEN`: answer the window.
    Open,
    /// `PREMIUM_PLAYERS_SUBHEADER_CG_LIST`: name the flagged players.
    List,
    /// `PREMIUM_PLAYERS_SUBHEADER_CG_ACTIVATE`: spend two vouchers and start the premium.
    Activate,
}

impl GameState {
    /// Run one `CG_PREMIUM_PLAYERS` for the character online under `vid`
    /// (`CInputMain::RecvPremiumPlayersPacket`, `G/input_main.cpp:5370-5386`), answered
    /// as a move is. An unknown sub-header is absorbed, as legacy's `default` arm does.
    /// The open and the list answer nothing here: the descriptor owns the store read and
    /// the socket, so it sends the window and the names itself.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when no character is online under `vid`.
    pub fn premium(
        &mut self,
        vid: Vid,
        step: PremiumStep,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let moved = MovedItems::new(
            pid,
            vid.raw(),
            world::character::MoveDone {
                kind: world::character::MoveKind::Given,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        match step {
            PremiumStep::Open | PremiumStep::List => Ok(moved),
            PremiumStep::Activate => Ok(self.premium_activate(vid, actor, moved)),
        }
    }

    /// A premium line as an `INFO` chat.
    fn premium_line(&self, actor: Mover, key: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
        crate::chat_line::chat_packet(actor.recipient(&self.locale), CHAT_TYPE_INFO, key, args)
    }

    /// `PremiumPlayersActivatePacket` (`G/char.cpp:11529-11561`): the already line, the
    /// voucher check with the proto's locale name, the spend, the notice then the line,
    /// and the flag with its 30-day timer. The four affects lay through
    /// [`GameState::check_premium_affects`], and their update goes around with the flag.
    fn premium_activate(&mut self, vid: Vid, actor: Mover, mut moved: MovedItems) -> MovedItems {
        let premium = self.premium_of(vid.raw()).0;
        if premium == 1 {
            moved.records.push(self.premium_line(
                actor,
                b"PREMIUM_YOU_ALREADY_HAVE_PREMIUM_ON_THIS_CHARACTER",
                &[],
            ));
            return moved;
        }
        if self.count_specify_item(vid, PREMIUM_VOUCHER) < 2 {
            let name: &[u8] = self
                .protos
                .get(PREMIUM_VOUCHER)
                .map_or(&[], |proto| proto.locale_name.as_slice());
            moved.records.push(self.premium_line(
                actor,
                b"Ai nevoie de %s pentru a putea activa statusul de Premium",
                &[Arg::Text(name)],
            ));
            return moved;
        }
        self.premium_spend(vid, actor, &mut moved);
        let name = self
            .characters
            .find_by_vid(vid)
            .map(|character| character.name().to_owned())
            .unwrap_or_default();
        let notice = crate::chat_line::format_line(
            b"Jucatorul %s si-a activat statusul de Premium.",
            &[Arg::Text(name.as_bytes())],
        );
        // `BroadcastNotice` (`G/cmd_gm.cpp:1517-1540`): the notice to every client of
        // the Channel. The peer half is retired (ADR-0001); the Channel is one core.
        let vids: Vec<Vid> = self.outboxes.keys().copied().collect();
        for to in vids {
            let Some(peer) = self.own_mover(to) else {
                continue;
            };
            let line = crate::chat_line::chat_packet(
                peer.recipient(&self.locale),
                protocol::gc_chat::CHAT_TYPE_NOTICE,
                b"%s",
                &[Arg::Text(&notice)],
            );
            if to == vid {
                moved.records.push(line);
            } else {
                let _sent = self.write_to_client(to, line);
            }
        }
        moved.records.push(self.premium_line(
            actor,
            b"Statusul tau de Premium a fost activat!",
            &[],
        ));
        let ends_at = super::affect::wall_now().wrapping_add(PREMIUM_DURATION);
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let (_, points) = character.items_and_points_mut();
            if let Some(points) = points {
                points.set_premium(1);
                points.set_premium_timer(ends_at);
            }
        }
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.card.premium = 1;
            body.card.premium_time = ends_at;
        }
        // `SetPremiumPlayer` and `SetPremiumPlayerTimer` each send `UpdatePacket`
        // (`:11589-11599`): the flag goes around before the affects lay. One send
        // carries both fields, as both are already set.
        let flags = self.affect_flags_of(vid);
        self.send_affect_update(vid, flags);
        self.check_premium_affects(vid);
        moved.points = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned());
        moved
    }

    /// Two `RemoveSpecifyItem(80015, 1)` for the player under `vid`, their records and
    /// changes folded into `moved`.
    fn premium_spend(&mut self, vid: Vid, actor: Mover, moved: &mut MovedItems) {
        let protos = self.protos.clone();
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let usable = common::item_slots::usable_inventory_cells(character.inven_point());
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        for _ in 0..2 {
            let done = world::character::remove_specify_item(
                items,
                slots,
                &protos,
                usable,
                (PREMIUM_VOUCHER, 1),
            );
            let packed = MovedItems::new(
                moved.owner_id,
                vid.raw(),
                world::character::MoveDone {
                    kind: world::character::MoveKind::Removed,
                    records: done.records,
                    changes: done.changes,
                },
                actor,
                &protos,
                &self.locale,
            );
            moved.records.extend(packed.records);
            moved.changes.extend(packed.changes);
        }
    }

    /// `CheckPremiumPlayersAffects` (`G/char.cpp:11563-11587`): with the flag, the four
    /// affects lay where none stands; without it, the four leave. The update of the
    /// carries the new flag, as the points already hold it.
    pub(super) fn check_premium_affects(&mut self, vid: Vid) {
        let premium = self.premium_of(vid.raw()).0 == 1;
        let flags_before = self.affect_flags_of(vid);
        for (kind, on, value) in PREMIUM_AFFECTS {
            let held = self
                .affect_events
                .get(&vid)
                .is_some_and(|event| event.affects.iter().any(|affect| affect.kind == kind));
            if premium && !held {
                self.add_pc_affect(
                    vid,
                    Affect {
                        kind,
                        apply_on: u8::try_from(on).unwrap_or(u8::MAX),
                        value,
                        flag: 0,
                        remaining: PREMIUM_AFFECT_DURATION,
                        sp_cost: 0,
                    },
                    true,
                );
            } else if !premium && held {
                let removed = self
                    .affect_events
                    .get_mut(&vid)
                    .map(|event| super::affect::take_affects_of_kind(&mut event.affects, kind))
                    .unwrap_or_default();
                for affect in &removed {
                    self.compute_affect(vid, affect, false);
                    let frame = super::affect::affect_remove_frame(affect);
                    let _retired = self.write_to_client(vid, frame);
                }
            }
        }
        let flags_after = self.affect_flags_of(vid);
        if flags_after != flags_before {
            self.send_affect_update(vid, flags_after);
        }
    }

    /// `premium_players_update_event` (`G/char.cpp:11478-11504`), once a minute: every
    /// online character whose timer has passed loses the flag and its affects through
    /// `StopPremiumPlayersUpdateEvent` (`:11518-11527`). Legacy runs one event per
    /// character; the sweep is the same check over the outboxes.
    pub(super) fn run_premium_ticks(&mut self, pulse: u64) {
        if pulse % (60 * u64::from(crate::save::PASSES_PER_SEC)) != 0 {
            return;
        }
        let vids: Vec<Vid> = self.outboxes.keys().copied().collect();
        for vid in vids {
            let Some(actor) = self.own_mover(vid) else {
                continue;
            };
            self.premium_expired(vid, actor);
        }
    }

    /// The expiry's half of `premium_players_update_event`
    /// (`G/char.cpp:11478-11504`) and `StopPremiumPlayersUpdateEvent` (`:11518-11527`):
    /// when the timer has passed, the flag and the timer clear, the affects leave
    /// through [`GameState::check_premium_affects`], and the time-up line answers.
    /// Answers whether the premium ended.
    pub(super) fn premium_expired(&mut self, vid: Vid, actor: Mover) -> bool {
        let (flag, ends_at) = self.premium_of(vid.raw());
        if flag != 1 || super::affect::wall_now() < ends_at {
            return false;
        }
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            let (_, points) = character.items_and_points_mut();
            if let Some(points) = points {
                points.set_premium(0);
                points.set_premium_timer(0);
            }
        }
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.card.premium = 0;
            body.card.premium_time = 0;
        }
        self.check_premium_affects(vid);
        self.tell(
            vid,
            actor,
            CHAT_TYPE_INFO,
            b"PREMIUM_YOUR_TIME_IS_UP_ALL_YOUR_FACILITIES_HAVE_BEEN_WITHDRAWN",
            &[],
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, points_at, TestClock, PLACE};
    use protocol::item_pos::ItemPos;
    use world::item::Item;

    /// A world with two vouchers at cells 0 and 1, the dice seeded, and one activation run.
    fn activated() -> (GameState, MovedItems, Vid) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let items: Vec<(ItemPos, Item)> = [0, 1]
            .iter()
            .map(|cell| {
                let mut voucher = Item::new(11 + u32::from(*cell), PREMIUM_VOUCHER);
                voucher.set_size(1).expect("a positive footprint");
                (ItemPos::new(1, *cell), voucher)
            })
            .collect();
        let vid = Vid::new(1);
        let _ = enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, &items);
        world
            .characters
            .find_by_vid_mut(vid)
            .expect("the player is in the world")
            .set_points(Some(points_at(30, 10_000, 0)));
        let actor = world.own_mover(vid).expect("the body stands");
        let moved = world
            .premium(vid, PremiumStep::Activate, actor)
            .expect("the player is in the world");
        (world, moved, vid)
    }

    /// The activation spends the two vouchers, sets the flag with a 30-day timer, lays
    /// the four affects, and chats the line after the packed removes.
    #[test]
    fn the_activation_spends_sets_and_lays() {
        let (world, moved, vid) = activated();
        assert_eq!(
            world.count_specify_item(vid, PREMIUM_VOUCHER),
            0,
            "the two vouchers leave"
        );
        let points = world
            .characters
            .find_by_vid(vid)
            .expect("the player is in the world")
            .points()
            .expect("the player has points");
        assert_eq!(points.premium(), 1, "the flag sets");
        let left = points.premium_time() - super::super::affect::wall_now();
        assert!(
            (2_592_000 - 5..=2_592_000).contains(&left),
            "the timer is 30 days out: {left}"
        );
        let kinds: Vec<u32> = world
            .affect_events
            .get(&vid)
            .expect("the event runs")
            .affects
            .iter()
            .map(|affect| affect.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                AFFECT_PREMIUM_PLAYERS_01,
                AFFECT_PREMIUM_PLAYERS_02,
                AFFECT_PREMIUM_PLAYERS_03,
                AFFECT_PREMIUM_PLAYERS_04,
            ],
            "the four affects lay in order"
        );
        let last = moved.records.last().expect("the line answers");
        assert!(
            last.windows(b"Premium a fost activat!".len())
                .any(|at| at == b"Premium a fost activat!"),
            "the success line answers"
        );
    }

    /// A flagged player hears the already line and keeps its vouchers; a lapsed timer
    /// ends at the sweep with the time-up line, the flag cleared and the affects gone.
    #[test]
    fn the_flagged_hear_the_already_and_the_lapsed_end() {
        let (mut world, _, vid) = activated();
        let actor = world.own_mover(vid).expect("the body stands");
        assert!(
            !world.premium_expired(vid, actor),
            "a fresh timer does not end"
        );
        let actor = world.own_mover(vid).expect("the body stands");
        let again = world
            .premium(vid, PremiumStep::Activate, actor)
            .expect("the player is in the world");
        assert_eq!(again.records.len(), 1, "only the already line answers");
        assert!(
            again.records[0]
                .windows(b"PREMIUM_YOU_ALREADY_HAVE_PREMIUM_ON_THIS_CHARACTER".len())
                .any(|at| at == b"PREMIUM_YOU_ALREADY_HAVE_PREMIUM_ON_THIS_CHARACTER"),
            "the already line answers"
        );
        let (_, points) = world
            .characters
            .find_by_vid_mut(vid)
            .expect("the player is in the world")
            .items_and_points_mut();
        points
            .expect("the player has points")
            .set_premium_timer(super::super::affect::wall_now() - 1);
        let actor = world.own_mover(vid).expect("the body stands");
        assert!(world.premium_expired(vid, actor), "the lapsed timer ends");
        let points = world
            .characters
            .find_by_vid(vid)
            .expect("the player is in the world")
            .points()
            .expect("the player has points");
        assert_eq!((points.premium(), points.premium_time()), (0, 0));
        assert!(
            world
                .affect_events
                .get(&vid)
                .is_none_or(|event| event.affects.is_empty()),
            "the affects leave"
        );
    }
}
