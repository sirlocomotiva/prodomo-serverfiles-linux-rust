//! The conqueror arms of `PointChange` (`G/char.cpp:3883-4002`): the champion level, its
//! experience, and the steps between.
//!
//! A gain of conqueror experience chats its amount, banks it, and raises the step as the
//! experience crosses the quarters of the next level's cost, exactly as the plain experience
//! arm raises `POINT_LEVEL_STEP` ([`Points::gain_exp`]). Steps 1 to 3 grant one conqueror
//! point each up to conqueror level 90; step 4 grants one, raises the conqueror level, and
//! the step falls back to 0. A gain that crosses the cost leaves its balance to a nested
//! call, which chats again and whose record shows no amount.
//!
//! One legacy Defect is not reproduced. A loss that takes the whole experience
//! (`amount < 0 && exp <= -amount`, `:3908-3915`) sets the conqueror experience to 0 but
//! writes `GetExp()`, the plain experience, as the record's value, so the client's champion
//! bar shows the wrong number until the next points record. The Rewrite writes the 0 it
//! stored. A loss smaller than the experience takes the gain path with a negative amount,
//! as legacy does, and its chat line carries the signed amount where legacy's `%lu` prints
//! the `int` as an unsigned number (also a Defect, not reproduced).

use common::levels;
use common::point_slot as point;

use super::{quarter_cost, PointRecord, Points};

/// One thing a conqueror experience change writes, in the order legacy writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConquerorEvent {
    /// `ChatPacket(CHAT_TYPE_INFO, "You have received %lu EXP for your Champion Level.")`
    /// with the amount the call was handed (`:3920`).
    Received(i64),
    /// A `GC_CHARACTER_POINT_CHANGE` record.
    Record(PointRecord),
}

/// The highest conqueror level whose steps 1 to 3 grant a point (`:3985`).
const POINT_GRANT_LEVEL: i32 = 90;

impl Points {
    /// `PointChange(POINT_CONQUEROR_EXP, amount, true)` (`G/char.cpp:3903-3972`), under a
    /// server whose highest conqueror level is `max_level`. Answers the chat lines and the
    /// records in order; the save is the caller's.
    pub fn gain_conqueror_exp(&mut self, amount: i64, max_level: i32) -> Vec<ConquerorEvent> {
        let mut events = Vec::new();
        self.conqueror_exp_arm(amount, true, max_level, &mut events);
        events
    }

    /// `PointChange(POINT_CONQUEROR_LEVEL, level - current)` followed by the resets
    /// `SetConqueror` and `do_conqueror_level` share (`G/char.cpp:11678-11697`,
    /// `G/cmd_gm.cpp:5168-5190`): the step and the four sungma become 0 in both arrays, the
    /// conqueror point becomes `conqueror_point` in both, and the experience 0. Answers the
    /// level arm's records. A level past `max_level` is refused by the arm, which writes
    /// nothing, and the resets still run, as in legacy. The compute, the points record and
    /// the trailing `UpdatePointsPacket(POINT_CONQUEROR_EXP)` are the caller's.
    pub fn reset_conqueror(
        &mut self,
        level: i32,
        conqueror_point: i32,
        max_level: i32,
    ) -> Vec<PointRecord> {
        let mut records = Vec::new();
        let amount = level - i32::from(self.conqueror_level);
        self.conqueror_level_arm(amount, false, max_level, &mut records);
        for slot in [
            point::POINT_CONQUEROR_LEVEL_STEP,
            point::POINT_SUNGMA_STR,
            point::POINT_SUNGMA_HP,
            point::POINT_SUNGMA_MOVE,
            point::POINT_SUNGMA_IMMUNE,
        ] {
            self.real[slot] = 0;
            self.instant[slot] = 0;
        }
        self.real[point::POINT_CONQUEROR_POINT] = conqueror_point;
        self.instant[point::POINT_CONQUEROR_POINT] = conqueror_point;
        self.conqueror_exp = 0;
        records
    }

    /// `do_conqueror_point`'s spend (`G/cmd_general.cpp:4040-4051`) on one sungma slot:
    /// both arrays rise by one, the points recompute, the slot's record, then one conqueror
    /// point goes and the points recompute again. Answers the records, or `None` when the
    /// character holds no conqueror point or the slot's real value is 90 or more, which
    /// changes nothing. The polymorph refusal and the points record are the caller's.
    pub fn spend_conqueror_point(&mut self, slot: usize) -> Option<Vec<PointRecord>> {
        if self.instant[point::POINT_CONQUEROR_POINT] <= 0 || self.real[slot] >= 90 {
            return None;
        }
        self.real[slot] += 1;
        self.instant[slot] += 1;
        let mut records = self.compute_points();
        self.nested(slot, 0, &mut records);
        self.nested(point::POINT_CONQUEROR_POINT, -1, &mut records);
        records.extend(self.compute_points());
        Some(records)
    }

    /// The body of the `POINT_CONQUEROR_EXP` arm. `shown` is the call's `bAmount`.
    fn conqueror_exp_arm(
        &mut self,
        amount: i64,
        shown: bool,
        max_level: i32,
        events: &mut Vec<ConquerorEvent>,
    ) {
        let kind = u8::try_from(point::POINT_CONQUEROR_EXP).unwrap_or(0);
        let exp = self.conqueror_exp;
        if amount < 0 && exp <= -amount {
            self.conqueror_exp = 0;
            events.push(ConquerorEvent::Record(PointRecord {
                kind,
                amount: if shown { -exp } else { 0 },
                value: 0,
                broadcast: false,
            }));
            return;
        }
        if i32::from(self.conqueror_level) >= max_level {
            return;
        }
        events.push(ConquerorEvent::Received(amount));
        let next = levels::conqueror_next_exp(self.conqueror_level);
        let (compared, balance, amount) = if exp + amount >= next {
            self.conqueror_exp = 0;
            (next, exp + amount - next, next - exp)
        } else {
            self.conqueror_exp = exp + amount;
            (self.conqueror_exp, 0, amount)
        };
        self.raise_conqueror_steps(compared, next, max_level, events);
        if balance != 0 {
            self.conqueror_exp_arm(balance, false, max_level, events);
        }
        events.push(ConquerorEvent::Record(PointRecord {
            kind,
            amount: if shown { amount } else { 0 },
            value: self.conqueror_exp,
            broadcast: false,
        }));
    }

    /// The step checks of the experience arm (`:3940-3964`), against the cost `next` of
    /// the level the gain started on. As in [`Points::gain_exp`], the step read is the real
    /// slot's clamped to 4, and only the first satisfied arm runs its loop.
    fn raise_conqueror_steps(
        &mut self,
        compared: i64,
        next: i64,
        max_level: i32,
        events: &mut Vec<ConquerorEvent>,
    ) {
        let quarter = quarter_cost(next);
        let step = self.real[point::POINT_CONQUEROR_LEVEL_STEP].min(4);
        let (to, broadcast) = if compared >= next && step < 4 {
            (4, true)
        } else if compared >= quarter * 3 && step < 3 {
            (3, true)
        } else if compared >= quarter * 2 && step < 2 {
            (2, true)
        } else if compared >= quarter && step < 1 {
            (1, false)
        } else {
            return;
        };
        let mut records = Vec::new();
        for _ in step..to {
            self.conqueror_step_arm(broadcast, max_level, &mut records);
        }
        events.extend(records.into_iter().map(ConquerorEvent::Record));
    }

    /// The body of the `POINT_CONQUEROR_LEVEL_STEP` arm (`:3975-4002`), rising by one. Its
    /// record never shows the amount.
    fn conqueror_step_arm(
        &mut self,
        broadcast: bool,
        max_level: i32,
        records: &mut Vec<PointRecord>,
    ) {
        let mut value = self.instant[point::POINT_CONQUEROR_LEVEL_STEP] + 1;
        let level = i32::from(self.conqueror_level);
        match value {
            1..=3 => {
                if level <= POINT_GRANT_LEVEL && level <= max_level {
                    self.nested(point::POINT_CONQUEROR_POINT, 1, records);
                }
            }
            4 => {
                self.nested(point::POINT_CONQUEROR_POINT, 1, records);
                self.conqueror_level_arm(1, true, max_level, records);
                value = 0;
            }
            _ => {}
        }
        self.instant[point::POINT_CONQUEROR_LEVEL_STEP] = value;
        self.real[point::POINT_CONQUEROR_LEVEL_STEP] = value;
        let kind = u8::try_from(point::POINT_CONQUEROR_LEVEL_STEP).unwrap_or(0);
        self.emit(kind, 1, false, broadcast, i64::from(value), records);
    }

    /// The body of the `POINT_CONQUEROR_LEVEL` arm (`:3884-3895`): past `max_level` the
    /// change is refused with no record; otherwise the level moves, the next cost is
    /// written, and the battle points are recomputed before the level's record.
    fn conqueror_level_arm(
        &mut self,
        amount: i32,
        broadcast: bool,
        max_level: i32,
        records: &mut Vec<PointRecord>,
    ) {
        let level = i32::from(self.conqueror_level) + amount;
        if level > max_level {
            return;
        }
        self.conqueror_level = u8::try_from(level.max(0)).unwrap_or(u8::MAX);
        self.nested(point::POINT_CONQUEROR_NEXT_EXP, 0, records);
        self.battle_points(records);
        let kind = u8::try_from(point::POINT_CONQUEROR_LEVEL).unwrap_or(0);
        let value = i64::from(self.conqueror_level);
        self.emit(kind, amount, false, broadcast, value, records);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::points::tests::fresh;

    /// `POINT_CONQUEROR_EXP` as the record's byte.
    fn exp() -> u8 {
        u8::try_from(point::POINT_CONQUEROR_EXP).expect("a point byte")
    }

    fn conqueror() -> Points {
        let mut points = Points::load(&fresh(0));
        let _ = points.compute_points();
        let _ = points.reset_conqueror(1, 0, 30);
        points
    }

    fn kinds(events: &[ConquerorEvent]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|event| match event {
                ConquerorEvent::Record(record) => Some(record.kind),
                ConquerorEvent::Received(_) => None,
            })
            .collect()
    }

    /// A gain under the first quarter banks the experience: the chat line, then the record
    /// showing the amount.
    #[test]
    fn a_small_gain_chats_and_banks() {
        let mut points = conqueror();
        let events = points.gain_conqueror_exp(1_000, 30);
        assert_eq!(
            events,
            [
                ConquerorEvent::Received(1_000),
                ConquerorEvent::Record(PointRecord {
                    kind: u8::try_from(point::POINT_CONQUEROR_EXP).unwrap(),
                    amount: 1_000,
                    value: 1_000,
                    broadcast: false,
                }),
            ]
        );
        assert_eq!(points.conqueror_exp(), 1_000);
        assert_eq!(kinds(&events), [exp()]);
    }

    /// Crossing the first quarter raises the step once, unbroadcast, with one point; the
    /// third quarter from step 1 raises twice, broadcast, with two more points.
    #[test]
    fn the_quarters_raise_the_steps_and_grant_points() {
        let mut points = conqueror();
        let next = levels::conqueror_next_exp(1);
        let quarter = quarter_cost(next);
        let events = points.gain_conqueror_exp(quarter, 30);
        let step = |events: &[ConquerorEvent]| {
            events
                .iter()
                .filter_map(|event| match event {
                    ConquerorEvent::Record(record)
                        if usize::from(record.kind) == point::POINT_CONQUEROR_LEVEL_STEP =>
                    {
                        Some((record.value, record.broadcast))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(step(&events), [(1, false)]);
        assert_eq!(points.get_point(point::POINT_CONQUEROR_POINT), 1);
        let events = points.gain_conqueror_exp(quarter * 2, 30);
        assert_eq!(step(&events), [(2, true), (3, true)]);
        assert_eq!(points.get_point(point::POINT_CONQUEROR_POINT), 3);
        assert_eq!(points.real_point(point::POINT_CONQUEROR_LEVEL_STEP), 3);
    }

    /// A gain crossing the cost raises the conqueror level through step 4, and the nested
    /// call takes the balance against the new level's cost: two chat lines, the nested
    /// record hiding its amount, then the outer record carrying the stored balance.
    #[test]
    fn a_crossing_levels_up_and_banks_the_balance() {
        let mut points = conqueror();
        let next = levels::conqueror_next_exp(1);
        let events = points.gain_conqueror_exp(next + 7, 30);
        assert_eq!(points.conqueror_level(), 2);
        assert_eq!(points.conqueror_exp(), 7);
        assert_eq!(points.get_point(point::POINT_CONQUEROR_POINT), 4);
        assert_eq!(points.real_point(point::POINT_CONQUEROR_LEVEL_STEP), 0);
        let received: Vec<i64> = events
            .iter()
            .filter_map(|event| match event {
                ConquerorEvent::Received(amount) => Some(*amount),
                ConquerorEvent::Record(_) => None,
            })
            .collect();
        assert_eq!(received, [next + 7, 7]);
        let exp_records: Vec<(i64, i64)> = events
            .iter()
            .filter_map(|event| match event {
                ConquerorEvent::Record(record) if record.kind == exp() => {
                    Some((record.amount, record.value))
                }
                _ => None,
            })
            .collect();
        assert_eq!(exp_records, [(0, 7), (next, 7)]);
        let level = u8::try_from(point::POINT_CONQUEROR_LEVEL).unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ConquerorEvent::Record(record) if record.kind == level && record.value == 2 && record.broadcast
        )));
    }

    /// At the highest conqueror level a gain is silent and banks nothing.
    #[test]
    fn the_highest_level_takes_nothing() {
        let mut points = conqueror();
        assert!(points.gain_conqueror_exp(500, 1).is_empty());
        assert_eq!(points.conqueror_exp(), 0);
    }

    /// A loss of the whole experience writes 0 as the record's value (legacy writes the
    /// plain experience: a Defect).
    #[test]
    fn a_whole_loss_writes_zero() {
        let mut points = conqueror();
        let _ = points.gain_conqueror_exp(300, 30);
        let events = points.gain_conqueror_exp(-500, 30);
        assert_eq!(
            events,
            [ConquerorEvent::Record(PointRecord {
                kind: exp(),
                amount: -300,
                value: 0,
                broadcast: false,
            })]
        );
    }

    /// The reset moves the level by the difference and leaves the point it was handed;
    /// a level past the max is refused while the resets still run.
    #[test]
    fn a_reset_sets_the_level_and_the_point() {
        let mut points = conqueror();
        points.set_both_points(point::POINT_SUNGMA_HP, 9);
        let records = points.reset_conqueror(5, 16, 30);
        assert_eq!(points.conqueror_level(), 5);
        assert_eq!(points.get_point(point::POINT_CONQUEROR_POINT), 16);
        assert_eq!(points.real_point(point::POINT_SUNGMA_HP), 0);
        let level = u8::try_from(point::POINT_CONQUEROR_LEVEL).unwrap();
        assert_eq!(
            records.last().map(|record| (record.kind, record.value)),
            Some((level, 5))
        );
        assert!(points.reset_conqueror(40, 0, 30).is_empty());
        assert_eq!(points.conqueror_level(), 5, "the arm refused");
    }

    /// A spend moves one point into the slot, and refuses with no point or at 90.
    #[test]
    fn a_spend_moves_one_point() {
        let mut points = conqueror();
        assert!(points
            .spend_conqueror_point(point::POINT_SUNGMA_STR)
            .is_none());
        let _ = points.reset_conqueror(2, 2, 30);
        let records = points
            .spend_conqueror_point(point::POINT_SUNGMA_STR)
            .expect("a point to spend");
        assert_eq!(points.real_point(point::POINT_SUNGMA_STR), 1);
        assert_eq!(points.get_point(point::POINT_CONQUEROR_POINT), 1);
        assert!(records.iter().any(|record| usize::from(record.kind)
            == point::POINT_CONQUEROR_POINT
            && record.value == 1));
        points.set_both_points(point::POINT_SUNGMA_STR, 90);
        assert!(points
            .spend_conqueror_point(point::POINT_SUNGMA_STR)
            .is_none());
    }
}
