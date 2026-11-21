//! The champion (conqueror) progression on the world: the experience a conqueror earns on a
//! new-world map (`G/char_battle.cpp:2658-2667`), `pc.set_conqueror`'s reset
//! (`CHARACTER::SetConqueror`, `G/char.cpp:11659-11701`), the GM's `/conqueror_level`
//! (`G/cmd_gm.cpp:5154-5197`), and the player's `/conqueror_point`
//! (`G/cmd_general.cpp:4012-4052`). The arms themselves are
//! [`world::character::Points::gain_conqueror_exp`] and its siblings; this layer sends their
//! chat lines and records, and the points record legacy's `PointsPacket` writes. The step
//! arm's `Save` and the commands' character log are the periodic save's and nobody's: the
//! save writes what the world holds (`hold_points`), and the Rewrite keeps no character log.

use common::point_slot as point;
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use world::character::{ConquerorEvent, PointRecord};

use super::GameState;
use crate::chat_line::Arg;
use crate::command::one_argument;
use crate::game_state::whisper::str_to_int;
use crate::item_move::Mover;

/// `LC_TEXT("You have received %lu EXP for your Champion Level.")` (`G/char.cpp:3920`).
const RECEIVED: &[u8] = b"You have received %lu EXP for your Champion Level.";

/// The polymorph refusal of `do_conqueror_point` (`G/cmd_general.cpp:4021`), sent as is.
const POLYMORPHED: &[u8] =
    b"@@(cmd_general)tradus:Nu poti sa ti dai punct cat timp esti transformat!";

impl GameState {
    /// `PointChange(POINT_CONQUEROR_EXP, amount, true)` on the player under `to`: the chat
    /// lines and records go to its client in the order the arm wrote them.
    pub(super) fn grant_conqueror_exp(&mut self, to: Vid, amount: i64) {
        let max = self.max_conqueror_level;
        let Some(mover) = self.own_mover(to) else {
            return;
        };
        let Some(points) = self.pc_points_mut(to.raw()) else {
            return;
        };
        let events = points.gain_conqueror_exp(amount, max);
        let level = points.conqueror_level();
        self.set_card_conqueror(to, level);
        for event in events {
            match event {
                ConquerorEvent::Received(amount) => {
                    self.tell(to, mover, CHAT_TYPE_INFO, RECEIVED, &[Arg::Int(amount)]);
                }
                ConquerorEvent::Record(record) => self.send_point_records(to, &[record]),
            }
        }
    }

    /// The tail `SetConqueror` and `do_conqueror_level` share: the conqueror level becomes
    /// `level` with its records, the step, sungma and experience reset, the point becomes
    /// `conqueror_point`, then `ComputePoints`, `PointsPacket` and
    /// `UpdatePointsPacket(POINT_CONQUEROR_EXP)`.
    pub(super) fn reset_pc_conqueror(&mut self, vid: Vid, level: i32, conqueror_point: i32) {
        let max = self.max_conqueror_level;
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let mut records = points.reset_conqueror(level, conqueror_point, max);
        records.extend(points.compute_points());
        let level = points.conqueror_level();
        self.set_card_conqueror(vid, level);
        self.send_point_records(vid, &records);
        self.send_points_packet(vid);
        let kind = u8::try_from(point::POINT_CONQUEROR_EXP).unwrap_or(0);
        let value = self
            .pc_points_mut(vid.raw())
            .map_or(0, |points| points.conqueror_exp());
        let record = PointRecord {
            kind,
            amount: 0,
            value,
            broadcast: false,
        };
        self.send_point_records(vid, &[record]);
    }

    /// `ACMD(do_conqueror_level)`: an empty line hears the syntax; else the caller's
    /// conqueror level becomes the line's, clamped to `0..=max`, and its point is four per
    /// level past the first, from the unclamped number, clamped to `0..=4 (max - 1)`.
    pub(super) fn run_conqueror_level(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        let (word, _) = one_argument(rest);
        if word.is_empty() {
            let syntax = b"Syntax: conqueror_level <level>";
            self.tell(vid, mover, CHAT_TYPE_INFO, syntax, &[]);
            return;
        }
        let max = self.max_conqueror_level;
        let asked = str_to_int(&word);
        let level = asked.clamp(0, max.max(0));
        // `level * 4 - 4` is `int` arithmetic; the Rewrite takes it in 64 bits.
        let ceiling = 4 * (i64::from(max) - 1);
        let granted = (i64::from(asked) * 4 - 4).clamp(0, ceiling.max(0));
        let granted = i32::try_from(granted).unwrap_or(i32::MAX);
        self.reset_pc_conqueror(vid, level, granted);
    }

    /// `ACMD(do_conqueror_point)`: one conqueror point goes into the named sungma
    /// (`sstr`, `shp`, `smove` or `simmune`), up to 90. An empty word, no point to spend,
    /// an unknown word, or a sungma at 90 changes nothing and says nothing; a polymorphed
    /// character hears the refusal.
    pub(super) fn run_conqueror_point(&mut self, vid: Vid, rest: &[u8], mover: Mover) {
        let (word, _) = one_argument(rest);
        if word.is_empty() {
            return;
        }
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        if points.is_polymorphed() {
            self.tell(vid, mover, CHAT_TYPE_INFO, POLYMORPHED, &[]);
            return;
        }
        let slot = match word.as_slice() {
            b"sstr" => point::POINT_SUNGMA_STR,
            b"shp" => point::POINT_SUNGMA_HP,
            b"smove" => point::POINT_SUNGMA_MOVE,
            b"simmune" => point::POINT_SUNGMA_IMMUNE,
            _ => return,
        };
        let Some(records) = points.spend_conqueror_point(slot) else {
            return;
        };
        self.send_point_records(vid, &records);
        self.send_points_packet(vid);
    }

    /// `PointsPacket` (`G/char.cpp:2031-2087`) to the player under `vid`, with the gold the
    /// world holds and the voice its card carries.
    fn send_points_packet(&self, vid: Vid) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let Some(points) = character.points() else {
            return;
        };
        let gold = i64::try_from(character.gold()).unwrap_or(i64::MAX);
        let voice = self.bodies.get(&vid).map_or(0, |body| body.card.voice);
        for frame in crate::loading_phase::mid_game_points(points, gold, voice) {
            let _sent = self.write_to_client(vid, frame);
        }
    }

    /// The conqueror level the body's card shows others when it is next inserted.
    fn set_card_conqueror(&mut self, vid: Vid, level: u8) {
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.card.conqueror_level = level;
        }
    }
}

#[cfg(test)]
mod tests {
    use gamedata::server_attr::SectreeGrid;

    use super::super::fixtures::{a_world, enter_on, TestClock};
    use super::*;
    use crate::chat_line::chat_packet;
    use crate::loading_phase::point_changes;

    /// A new-world map (`SUNGMA_WILL_MAPS`), hosted as the fixture's map is.
    const NEW_WORLD: (u8, i32) = (1, 373);

    const VID: u32 = 7;

    /// A conqueror on `place` after a level-10 kill worth 75, with what its client heard.
    fn a_conqueror_on(place: (u8, i32)) -> (GameState, Vec<Vec<u8>>) {
        let clock = TestClock::default();
        let mut world = a_world(&clock).with_max_level(120);
        world.host_map(
            NEW_WORLD.0,
            NEW_WORLD.1,
            SectreeGrid {
                x: 0,
                y: 0,
                columns: 10,
                rows: 10,
            },
        );
        let mut inbox = enter_on(&mut world, place, VID, (3200, 3200), 820);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        let _records = points.become_conqueror(30);
        while inbox.try_recv().is_ok() {}
        // A level-10 kill worth 75: the delta row pays it whole, under the level's cap.
        world.give_exp(0x8000_0001, Vid::new(VID), 10, 75);
        (
            world,
            std::iter::from_fn(|| inbox.try_recv().ok()).collect(),
        )
    }

    /// A kill on a new-world map pays a conqueror champion experience: the chat line with
    /// the amount, then the record showing it (`G/char_battle.cpp:2658-2662`).
    #[test]
    fn a_kill_on_a_new_world_map_pays_champion_experience() {
        let (mut world, heard) = a_conqueror_on(NEW_WORLD);
        let mover = world.own_mover(Vid::new(VID)).expect("the entrant moves");
        let line = chat_packet(
            mover.recipient(&world.locale),
            CHAT_TYPE_INFO,
            RECEIVED,
            &[Arg::Int(75)],
        );
        let record = PointRecord {
            kind: u8::try_from(point::POINT_CONQUEROR_EXP).unwrap(),
            amount: 75,
            value: 75,
            broadcast: false,
        };
        assert_eq!(heard, [line, point_changes(&[record], VID).concat()]);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!((points.conqueror_exp(), points.exp()), (75, 0));
    }

    /// Elsewhere the same conqueror earns plain experience.
    #[test]
    fn a_kill_elsewhere_pays_plain_experience() {
        let (mut world, _heard) = a_conqueror_on(super::super::fixtures::PLACE);
        let points = world.pc_points_mut(VID).expect("the entrant has points");
        assert_eq!(points.conqueror_exp(), 0);
        assert_eq!(points.exp(), 75);
    }
}
