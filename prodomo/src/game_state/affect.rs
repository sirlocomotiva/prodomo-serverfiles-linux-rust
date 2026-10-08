//! The affect event of each character (`m_pkAffectEvent`, `G/char_affect.cpp:143-162`), and the
//! timed affects it counts down (`CHARACTER::AddAffect`, `CAffect`, `G/char_affect.cpp`).
//!
//! One event serves one character. Every 25 Pulses it runs the potion recovery, then the
//! stamina refill, then the countdown of each timed affect, and it runs again while any of
//! them is left. It stops when no affect is left, no recovery is left, and stamina is full
//! (`CHARACTER::StartAffectEvent` is started again by a later affect or potion).
//!
//! Legacy's stamina refill runs on the legacy clock, three seconds after the body stopped. The
//! Rewrite counts the same three seconds in Pulses, 75 of them, which the Divergence in
//! `docs/STATUS.md` records.
//!
//! Each change the event makes to the affect flags is sent to the character and to every
//! character that sees it, as `GC_CHARACTER_UPDATE` (`G/char.cpp:1315`).

use common::point_slot as point;
use common::vid::Vid;
use protocol::gc_vid::{GcHeaderAndDwordAndByte, HEADER_GC_AFFECT_REMOVE};
use world::character::{is_recovering, update_recovery};

use super::view::EntityKey;
use super::GameState;
use crate::item_move::Mover;
use crate::loading_phase::{
    point_changes, AFFECT_REVIVE_INVISIBLE, AFF_REVIVE_INVISIBLE, REVIVE_INVISIBLE_SECONDS,
};
use crate::save::PASSES_PER_SEC;

/// The Pulses a stopped body waits before the stamina refills: `3000` ms of legacy's clock at
/// 25 Pulses a second.
const STAMINA_REST_PULSES: u64 = 75;

/// One timed affect of a character, as legacy's `CAffect` keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Affect {
    /// The affect type, `wType`, such as [`AFFECT_REVIVE_INVISIBLE`].
    pub(super) kind: u32,
    /// `bApplyOn`: the point an affect applies to, 0 for none.
    pub(super) apply_on: u8,
    /// `dwFlag`, numbered from 1: the affect flag it sets, or 0 for none.
    pub(super) flag: u32,
    /// `lDuration` in ticks: each tick takes one away, and the affect ends at 0 or below.
    pub(super) remaining: i32,
}

impl Affect {
    /// `ReviveInvisible(5)`: `AddAffect(215, POINT_NONE, 0, flag 28, 5, 0, true)`.
    pub(super) const fn revive_invisible() -> Self {
        Self {
            kind: AFFECT_REVIVE_INVISIBLE,
            apply_on: 0,
            flag: AFF_REVIVE_INVISIBLE,
            remaining: REVIVE_INVISIBLE_SECONDS,
        }
    }
}

/// The affect event of one character, and what it counts down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AffectEvent {
    /// The Pulse the event fires on next.
    pub(super) due: u64,
    /// The order the event was started in. Two events due on one Pulse run in this order.
    pub(super) sequence: u64,
    /// The timed affects, in the order they were added.
    pub(super) affects: Vec<Affect>,
}

/// The affect flag words the affects set. Flag `n` is bit `n - 1` of word `(n - 1) / 32`, so
/// the revive-invisible flag 28 is `0x0800_0000` in word 0. A flag of 0 sets nothing.
#[must_use]
pub(super) fn flags_of(affects: &[Affect]) -> [u32; 2] {
    let mut words = [0_u32; 2];
    for affect in affects {
        let Some(bit) = affect.flag.checked_sub(1) else {
            continue;
        };
        if let Some(word) = usize::try_from(bit / 32)
            .ok()
            .and_then(|index| words.get_mut(index))
        {
            *word |= 1_u32 << (bit % 32);
        }
    }
    words
}

/// One tick of the countdown (`ProcessAffect`): every affect loses a tick, and those at 0 or
/// below are taken out. The taken-out affects come back in the order they were added.
fn count_down(affects: &mut Vec<Affect>) -> Vec<Affect> {
    let mut expired = Vec::new();
    affects.retain_mut(|affect| {
        affect.remaining -= 1;
        if affect.remaining > 0 {
            return true;
        }
        expired.push(*affect);
        false
    });
    expired
}

/// Takes the affects of `kind` out of `affects`, in the order they were added.
fn take_affects_of_kind(affects: &mut Vec<Affect>, kind: u32) -> Vec<Affect> {
    let mut taken = Vec::new();
    affects.retain(|affect| {
        if affect.kind != kind {
            return true;
        }
        taken.push(*affect);
        false
    });
    taken
}

/// The `GC_AFFECT_REMOVE` of one affect: its type, then `bApplyOn` (`G/packet.h`).
fn affect_remove_frame(affect: &Affect) -> Vec<u8> {
    let mut frame = Vec::new();
    GcHeaderAndDwordAndByte::new(HEADER_GC_AFFECT_REMOVE, affect.kind, affect.apply_on)
        .encode_into(&mut frame);
    frame
}

impl GameState {
    /// Starts the affect event of `vid` if it has none, due one tick from now.
    pub(super) fn start_affect_event(&mut self, vid: Vid) {
        if self.affect_events.contains_key(&vid) {
            return;
        }
        let due = self.last_pulse.saturating_add(u64::from(PASSES_PER_SEC));
        self.affect_sequence = self.affect_sequence.saturating_add(1);
        let event = AffectEvent {
            due,
            sequence: self.affect_sequence,
            affects: Vec::new(),
        };
        let _started = self.affect_events.insert(vid, event);
    }

    /// Adds `affect` to the event of `vid`, which starts if it has none.
    pub(super) fn add_affect(&mut self, vid: Vid, affect: Affect) {
        self.start_affect_event(vid);
        if let Some(event) = self.affect_events.get_mut(&vid) {
            event.affects.push(affect);
        }
    }

    /// The affect flag words `vid` has now: the ones its affects set, or 0 with no event.
    pub(super) fn affect_flags_of(&self, vid: Vid) -> [u32; 2] {
        self.affect_events
            .get(&vid)
            .map_or([0; 2], |event| flags_of(&event.affects))
    }

    /// The `GC_CHARACTER_UPDATE` of `vid` with `flags` in its affect words, or `None` when the
    /// character is not online. Its look, language and PK mode come from the world and its body.
    pub(super) fn update_frame(&self, vid: Vid, flags: [u32; 2]) -> Option<Vec<u8>> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let body = self.bodies.get(&vid)?;
        let points = character.points()?;
        let look = world::character::look_of(character.items(), points, &self.protos);
        let mover = Mover {
            recently_fought: false,
            empire: body.card.empire,
            language: body.card.language,
            pk_mode: body.card.pk_mode,
            affect_flags: flags,
        };
        let mut frame = Vec::new();
        crate::item_move::character_update(vid.raw(), &look, mover).encode_into(&mut frame);
        Some(frame)
    }

    /// Sends `vid`'s affect-flag update to the character and to every character that sees it.
    pub(super) fn send_affect_update(&mut self, vid: Vid, flags: [u32; 2]) {
        let Some(frame) = self.update_frame(vid, flags) else {
            return;
        };
        let me = EntityKey::Character(vid.raw());
        self.packet_around(me, &frame, None);
    }

    /// `ch->ReviveInvisible(5)` at entry (`G/char.cpp:7490-7493`): the affect joins the event,
    /// and the new affect flags go to the viewers of the character, which the character's own
    /// update is also sent to. Returns that update, for the character's enter-game burst, or
    /// `None` when the character stands in no sectree: `PacketAround` sends such an entity
    /// nothing, not even its own copy (`G/entity.cpp:95`), so its burst carries none either.
    pub(super) fn enter_revive_invisible(&mut self, vid: Vid) -> Option<Vec<u8>> {
        self.add_affect(vid, Affect::revive_invisible());
        let flags = self.affect_flags_of(vid);
        let frame = self.update_frame(vid, flags)?;
        let me = EntityKey::Character(vid.raw());
        if !self.in_sectree(me) {
            return None;
        }
        self.packet_around(me, &frame, Some(me));
        Some(frame)
    }

    /// `OnMove(true)` (`G/char.cpp:6156`): an accepted attack or combo takes the revive-invisible
    /// affect off. The update goes to the viewers and the character first, then the removal to
    /// the character. The event keeps running and ends on its next run when nothing is left.
    pub(super) fn remove_revive_invisible(&mut self, vid: Vid) {
        let flags_before = self.affect_flags_of(vid);
        let removed = match self.affect_events.get_mut(&vid) {
            Some(event) => take_affects_of_kind(&mut event.affects, AFFECT_REVIVE_INVISIBLE),
            None => Vec::new(),
        };
        if removed.is_empty() {
            return;
        }
        let flags_after = self.affect_flags_of(vid);
        if flags_after != flags_before {
            self.send_affect_update(vid, flags_after);
        }
        for affect in &removed {
            let _delivered = self.write_to_client(vid, affect_remove_frame(affect));
        }
    }

    /// Runs the affect events due by `pulse`, in the order they were started.
    pub(super) fn run_affect_events(&mut self, pulse: u64) {
        let mut due: Vec<(u64, u64, Vid)> = self
            .affect_events
            .iter()
            .filter(|(_, event)| event.due <= pulse)
            .map(|(vid, event)| (event.due, event.sequence, *vid))
            .collect();
        due.sort_unstable_by_key(|&(_, sequence, _)| sequence);
        for (_, _, vid) in due {
            self.run_affect_event(vid, pulse);
        }
    }

    /// One run of the event of `vid`: the recovery, the stamina refill, the countdown, then
    /// the removals and the flag update, and the next run when something is left.
    fn run_affect_event(&mut self, vid: Vid, pulse: u64) {
        let flags_before = self.affect_flags_of(vid);
        let idle = self
            .bodies
            .get(&vid)
            .map(|body| pulse.saturating_sub(body.stop_pulse));
        let Some(points) = self
            .characters
            .find_by_vid_mut(vid)
            .ok()
            .and_then(|character| character.items_and_points_mut().1)
        else {
            let _gone = self.affect_events.remove(&vid);
            return;
        };
        let mut records = update_recovery(points);
        let recovering = is_recovering(points);
        let stamina_max = points.max_stamina();
        if points.stamina() < stamina_max && idle.is_some_and(|idle| idle >= STAMINA_REST_PULSES) {
            records.extend(
                points
                    .point_change(point::POINT_STAMINA, stamina_max, false, false)
                    .unwrap_or_default(),
            );
        }
        let stamina_full = points.stamina() == points.max_stamina();
        let Some(event) = self.affect_events.get_mut(&vid) else {
            return;
        };
        let expired = count_down(&mut event.affects);
        let again = recovering || !event.affects.is_empty() || !stamina_full;
        for frame in point_changes(&records, vid.raw()) {
            let _delivered = self.write_to_client(vid, frame);
        }
        for affect in &expired {
            let _delivered = self.write_to_client(vid, affect_remove_frame(affect));
        }
        let flags_after = self.affect_flags_of(vid);
        if flags_after != flags_before {
            self.send_affect_update(vid, flags_after);
        }
        if again {
            if let Some(event) = self.affect_events.get_mut(&vid) {
                event.due = pulse.saturating_add(u64::from(PASSES_PER_SEC));
            }
        } else {
            let _ended = self.affect_events.remove(&vid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{count_down, flags_of, Affect};
    use crate::game_loop::PulseProcessor;
    use crate::game_state::fixtures::{a_world, enter_on, TestClock, PLACE};
    use crate::game_state::GameState;
    use crate::loading_phase::AFFECT_REVIVE_INVISIBLE;
    use common::vid::Vid;

    #[test]
    fn the_revive_invisible_flag_is_bit_twenty_seven_of_word_zero() {
        let affects = [Affect::revive_invisible()];
        assert_eq!(flags_of(&affects), [0x0800_0000, 0]);
    }

    #[test]
    fn a_flag_past_thirty_two_sets_the_second_word() {
        let affect = Affect {
            flag: 33,
            ..Affect::revive_invisible()
        };
        assert_eq!(flags_of(&[affect]), [0, 1]);
    }

    #[test]
    fn a_flag_of_zero_sets_nothing_and_no_affect_sets_nothing() {
        let affect = Affect {
            flag: 0,
            ..Affect::revive_invisible()
        };
        assert_eq!(flags_of(&[affect]), [0, 0]);
        assert_eq!(flags_of(&[]), [0, 0]);
    }

    #[test]
    fn the_countdown_ends_the_revive_invisible_affect_on_the_fifth_tick() {
        let mut affects = vec![Affect::revive_invisible()];
        for _ in 0..4 {
            assert!(count_down(&mut affects).is_empty());
            assert_eq!(affects.len(), 1);
        }
        let expired = count_down(&mut affects);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].kind, AFFECT_REVIVE_INVISIBLE);
        assert_eq!(expired[0].remaining, 0);
        assert!(affects.is_empty());
    }

    #[test]
    fn a_treeless_entrant_gets_no_update_in_its_burst_and_its_affect_still_starts() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        // Map 77 is hosted by no index, so the body is placed treeless (V4).
        let _inbox = enter_on(&mut world, (PLACE.0, 77), 1, (5, 5), 12_345);
        assert_eq!(world.enter_revive_invisible(Vid::new(1)), None);
        assert_eq!(world.affect_flags_of(Vid::new(1)), [0x0800_0000, 0]);
    }

    /// The stamina `vid` has now, and the maximum it refills to.
    fn stamina_of(world: &GameState, vid: Vid) -> (i32, i32) {
        let points = world
            .characters()
            .find_by_vid(vid)
            .expect("the entrant is in the world")
            .points()
            .expect("the entrant has points");
        (points.stamina(), points.max_stamina())
    }

    #[test]
    fn the_stamina_refill_comes_on_the_seventy_fifth_idle_pulse_and_not_before() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let vid = Vid::new(1);
        // The body stops on Pulse 0 and its event starts there, due on 25, 50 and 75.
        let _inbox = enter_on(&mut world, PLACE, 1, (5, 5), 10);
        world.start_affect_event(vid);
        world.run_affect_events(25);
        world.run_affect_events(50);
        let (stamina, max) = stamina_of(&world, vid);
        assert!(stamina < max, "the entrant starts below its maximum");
        assert_eq!(
            stamina, 10,
            "50 idle Pulses is short of the 75 the refill waits"
        );
        world.run_affect_events(75);
        assert_eq!(
            stamina_of(&world, vid),
            (max, max),
            "75 idle Pulses refills it"
        );
    }

    #[test]
    fn a_body_stopped_a_pulse_late_is_refilled_on_the_hundredth_pulse_not_the_seventy_fifth() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let vid = Vid::new(1);
        // The event starts on Pulse 0, due on 25, 50, 75 and 100. The body stops on Pulse 1, so
        // its idle counts at those runs are 24, 49, 74 and 99.
        world.start_affect_event(vid);
        world.process_pulse(1);
        let _inbox = enter_on(&mut world, PLACE, 1, (5, 5), 10);
        world.run_affect_events(25);
        world.run_affect_events(50);
        world.run_affect_events(75);
        let (stamina, max) = stamina_of(&world, vid);
        assert_eq!(
            stamina, 10,
            "74 idle Pulses is short of the 75 the refill waits"
        );
        assert!(stamina < max, "the entrant is still below its maximum");
        world.run_affect_events(100);
        assert_eq!(
            stamina_of(&world, vid),
            (max, max),
            "99 idle Pulses refills it"
        );
    }

    #[test]
    fn two_revives_that_end_on_one_pulse_send_their_updates_in_the_order_they_started() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut watcher = enter_on(&mut world, PLACE, 3, (5, 5), 10_000);
        let _started_first = enter_on(&mut world, PLACE, 2, (6, 5), 10_000);
        let _started_second = enter_on(&mut world, PLACE, 1, (7, 5), 10_000);
        // Both start on Pulse 0, due on 25 and so on, and end together on the fifth tick, 125.
        // The VID order is the reverse of the order they started in.
        let _burst = world.enter_revive_invisible(Vid::new(2));
        let _burst = world.enter_revive_invisible(Vid::new(1));
        while watcher.try_recv().is_ok() {}
        for pulse in [25, 50, 75, 100, 125] {
            world.run_affect_events(pulse);
        }
        let updated: Vec<u32> = std::iter::from_fn(|| watcher.try_recv().ok())
            .filter(|frame| frame.first() == Some(&19))
            .map(|frame| u32::from_le_bytes(frame[1..5].try_into().expect("four bytes")))
            .collect();
        assert_eq!(updated, [2, 1], "the revive that started first ends first");
    }

    #[test]
    fn each_started_event_takes_the_next_sequence_number() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world.start_affect_event(Vid::new(1));
        world.start_affect_event(Vid::new(2));
        let first = world.affect_events[&Vid::new(1)].sequence;
        let second = world.affect_events[&Vid::new(2)].sequence;
        assert!(
            second > first,
            "a later event runs after an earlier one due on the same Pulse"
        );
    }

    #[test]
    fn two_affects_that_end_together_leave_in_the_order_they_were_added() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter_on(&mut world, PLACE, 1, (5, 5), 10_000);
        let vid = Vid::new(1);
        // Two affects of different kinds, both on their last tick, so one run ends both.
        world.add_affect(
            vid,
            Affect {
                kind: 7,
                apply_on: 1,
                flag: 0,
                remaining: 1,
            },
        );
        world.add_affect(
            vid,
            Affect {
                kind: 9,
                apply_on: 2,
                flag: 0,
                remaining: 1,
            },
        );
        while inbox.try_recv().is_ok() {}
        world.run_affect_events(25);
        let removed: Vec<(u32, u8)> = std::iter::from_fn(|| inbox.try_recv().ok())
            .filter(|frame| frame.first() == Some(&127))
            .map(|frame| {
                let kind = u32::from_le_bytes(frame[1..5].try_into().expect("four bytes"));
                (kind, frame[5])
            })
            .collect();
        assert_eq!(
            removed,
            [(7, 1), (9, 2)],
            "the affects leave in the order they were added"
        );
    }
}
