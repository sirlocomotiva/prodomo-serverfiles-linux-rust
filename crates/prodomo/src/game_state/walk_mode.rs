//! The walk mode lines and the stamina that walks a player: `/set_walk_mode` and
//! `/set_run_mode` (`G/cmd_general.cpp:2016-2026`), `SetNowWalking` and `ResetWalking`
//! (`G/char.cpp:7345-7380`, `G/char.h:954`), `StateMove`'s stamina arm
//! (`G/char_state.cpp:812-851`), and the consume gauge (`StartStaminaConsume` and
//! `StopStaminaConsume`, `G/char.cpp:7383-7402`).
//!
//! [`GameState::run_walk_mode`] is `ACMD(do_set_walk_mode)` and
//! `ACMD(do_set_run_mode)`: the line sets both of the caller's flags (`SetWalking`
//! and `SetNowWalking`) to the asked mode (`G/char.h:952-954`). Only the second
//! flag's change fans: `SetNowWalking` sends the walk-mode record to the caller's
//! view (`PacketView`) when the flag moves, stamps the five-second walk start only
//! on a walk, and says nothing when asked for the mode it holds. Nothing is
//! stored: the flags live in memory, as legacy's do.
//!
//! The flag also feeds `IsWalking` (`G/char.h:951`): a player walks when its
//! flag is set or its stamina is spent, and the move clip and the insert's
//! records read that. A step that empties the pool walks the flag at once, and a
//! refill that climbs off 0 restores the mode line's point (`ResetWalking`).
//!
//! The first spend starts the client's gauge once, and the emptying step, a step
//! past the fight window, an arrival, and a non-`MOVE` function stop it: each
//! with a zero-change record and a command line carrying the pool, the start
//! with the drain rate too.

use common::point_slot as point;
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_COMMAND;

use crate::chat_line::Arg;
use crate::game_state::mob_ai::WALKMODE_WALK;
use crate::game_state::motion::is_walking;
use crate::game_state::view::EntityKey;
use crate::game_state::view_encode::{walk_mode, WALKMODE_RUN};
use crate::game_state::GameState;
use crate::save::PASSES_PER_SEC;

/// The walk the stamina restore waits out (`G/char_state.cpp:818`): five seconds.
const WALK_RESTORE_MS: u32 = 5_000;
/// The fight window a run consumes stamina in (`:824`): twenty seconds.
const FIGHT_STAMINA_MS: u32 = 20_000;
/// `STAMINA_PER_STEP` (`G/char.h:80`): 1.
const STAMINA_PER_STEP: i32 = 1;
/// `UNIQUE_ITEM_HALF_STAMINA` (`G/unique_item.h:24`): 70040, worn, it halves the drain.
const HALF_STAMINA: u32 = 70_040;

impl GameState {
    /// `ACMD(do_set_walk_mode)` and `ACMD(do_set_run_mode)`: the caller walks or
    /// runs, and a change fans its walk-mode record to its view.
    pub(super) fn run_walk_mode(&mut self, vid: Vid, walk: bool) {
        let now = self.clock.now();
        let Some(body) = self.bodies.get_mut(&vid) else {
            return;
        };
        body.walk.walking = walk;
        if body.walk.now_walking == walk {
            return;
        }
        body.walk.now_walking = walk;
        if walk {
            body.walk_start_ms = Some(now);
        }
        let mode = if walk { WALKMODE_WALK } else { WALKMODE_RUN };
        let me = EntityKey::Character(vid.raw());
        self.packet_around(me, &walk_mode(vid.raw(), mode), None);
    }

    /// `CHARACTER::SetNowWalking` on the player under `vid` (`G/char.cpp:7345-7380`):
    /// the flag moves, the walk stamps on a walk, and the change fans to the view.
    pub(super) fn set_pc_now_walking(&mut self, vid: Vid, walk: bool) {
        let now = self.clock.now();
        let Some(body) = self.bodies.get_mut(&vid) else {
            return;
        };
        if body.walk.now_walking == walk {
            return;
        }
        body.walk.now_walking = walk;
        if walk {
            body.walk_start_ms = Some(now);
        }
        let mode = if walk { WALKMODE_WALK } else { WALKMODE_RUN };
        let me = EntityKey::Character(vid.raw());
        self.packet_around(me, &walk_mode(vid.raw(), mode), None);
    }

    /// `CHARACTER::ResetWalking` (`G/char.h:954`): the flag returns to the mode line's
    /// point, fanning when it moves.
    pub(super) fn reset_walking(&mut self, vid: Vid) {
        let walking = self.bodies.get(&vid).is_some_and(|body| body.walk.walking);
        self.set_pc_now_walking(vid, walking);
    }

    /// `StateMove`'s stamina arm on the moving player under `vid`
    /// (`G/char_state.cpp:812-851`): a walk past five seconds restores the pool, and a
    /// run within twenty seconds of a fight spends a point a step (every other step
    /// with the half-stamina unique worn), stopping the consume past the window or at
    /// an emptied pool, which walks the flag at once.
    pub(super) fn step_stamina(&mut self, vid: Vid, pulse: u64) {
        let now = self.clock.now();
        let flag = self
            .bodies
            .get(&vid)
            .is_some_and(|body| body.walk.now_walking);
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        if is_walking(character, flag) {
            let stamped = self.bodies.get(&vid).and_then(|body| body.walk_start_ms);
            let Some(start) = stamped else {
                if let Some(body) = self.bodies.get_mut(&vid) {
                    body.walk_start_ms = Some(now);
                }
                return;
            };
            // Below the maximum only (`:814`): on a full pool legacy sends nothing.
            let below_max = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.points())
                .is_some_and(|points| points.stamina() < points.max_stamina());
            if below_max && now.wrapping_sub(start) > WALK_RESTORE_MS {
                let max = self
                    .characters
                    .find_by_vid(vid)
                    .ok()
                    .and_then(|character| character.points())
                    .map_or(0, world::character::Points::max_stamina);
                self.change_pc_point(vid, point::POINT_STAMINA, max, false);
            }
            return;
        }
        if self.is_riding(vid.raw()) {
            return;
        }
        let fought = self
            .bodies
            .get(&vid)
            .and_then(|body| body.last_attack)
            .is_some_and(|at| now.wrapping_sub(at) < FIGHT_STAMINA_MS);
        if !fought {
            // Past the window (`:846-849`): the stop runs its record and its line when
            // the flag is set.
            self.stop_stamina_consume(vid);
            return;
        }
        if self.wears_half_stamina(vid) && pulse & 1 == 0 {
            return;
        }
        // `StartAffectEvent` (`:826`): the drain runs the event, whose refill covers the
        // pool three idle seconds on; the walk arm below covers chained moves, whose
        // accepts keep the idle count short.
        self.start_affect_event(vid);
        self.change_pc_point(vid, point::POINT_STAMINA, -STAMINA_PER_STEP, false);
        self.start_stamina_consume(vid);
        let stamina = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::stamina);
        if stamina <= 0 {
            if let Some(points) = self.pc_points_mut(vid.raw()) {
                points.set_stamina_now(0);
            }
            self.set_pc_now_walking(vid, true);
            self.stop_stamina_consume(vid);
        }
    }

    /// `CHARACTER::StartStaminaConsume` (`G/char.cpp:7383-7393`): once, a zero-change
    /// record, the flag, and the client's gauge line with the drain rate
    /// (`STAMINA_PER_STEP` a Pulse, halved with the unique worn) and the pool.
    fn start_stamina_consume(&mut self, vid: Vid) {
        if self
            .bodies
            .get(&vid)
            .is_some_and(|body| body.stamina_consume)
        {
            return;
        }
        self.change_pc_point(vid, point::POINT_STAMINA, 0, false);
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.stamina_consume = true;
        }
        let rate = i64::from(STAMINA_PER_STEP) * i64::from(PASSES_PER_SEC)
            / if self.wears_half_stamina(vid) { 2 } else { 1 };
        let stamina = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::stamina);
        if let Some(mover) = self.own_mover(vid) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_COMMAND,
                b"StartStaminaConsume %d %d",
                &[Arg::Int(rate), Arg::Int(i64::from(stamina))],
            );
        }
    }

    /// `CHARACTER::StopStaminaConsume` (`G/char.cpp:7395-7402`): once, a zero-change
    /// record, the flag down, and the client's gauge line with the pool.
    pub(super) fn stop_stamina_consume(&mut self, vid: Vid) {
        if !self
            .bodies
            .get(&vid)
            .is_some_and(|body| body.stamina_consume)
        {
            return;
        }
        self.change_pc_point(vid, point::POINT_STAMINA, 0, false);
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.stamina_consume = false;
        }
        let stamina = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::stamina);
        if let Some(mover) = self.own_mover(vid) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_COMMAND,
                b"StopStaminaConsume %d",
                &[Arg::Int(i64::from(stamina))],
            );
        }
    }

    /// `IsStaminaHalfConsume` (`G/char.cpp:7409-7412`): the half-stamina unique worn.
    fn wears_half_stamina(&self, vid: Vid) -> bool {
        use common::enums::EWearPositions;
        use world::character::Equipment;

        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let equipment = Equipment::of(character.items(), &self.protos);
        [EWearPositions::Unique1, EWearPositions::Unique2]
            .into_iter()
            .filter_map(|wear| equipment.wear(wear as u16))
            .any(|worn| worn.item.vnum == HALF_STAMINA)
    }
}

#[cfg(test)]
mod tests {
    use common::vid::Vid;

    use crate::game_state::fixtures::{a_world, enter, enter_seeing_with, TestClock};
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

    /// Every record the inbox holds, in order.
    fn all_of(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut records = Vec::new();
        while let Ok(record) = inbox.try_recv() {
            records.push(record);
        }
        records
    }

    /// The zero-change stamina record at `value`, as the consume start and stop send it.
    fn zero_change(vid: u32, value: i64) -> Vec<u8> {
        crate::loading_phase::point_changes(
            &[world::character::PointRecord {
                kind: 9,
                amount: 0,
                value,
                broadcast: false,
            }],
            vid,
        )
        .pop()
        .expect("one record")
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

    /// The stamina a body holds now.
    fn stamina_of(world: &crate::game_state::GameState, vid: u32) -> i32 {
        world
            .characters
            .find_by_vid(Vid::new(vid))
            .expect("the entrant is in the world")
            .points()
            .expect("the entrant has points")
            .stamina()
    }

    /// A run within twenty seconds of a fight spends a point a step and raises the
    /// consume flag; a run that never fought spends nothing.
    #[test]
    fn a_run_after_a_fight_spends_a_point_a_step() {
        let clock = TestClock::default();
        clock.set(10_000);
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (3200, 3200), 820);
        world.bodies.get_mut(&Vid::new(VID)).unwrap().last_attack = Some(10_000);
        assert!(world.goto(VID, 4800, 3200));
        world.step_motion(1);
        assert_eq!(stamina_of(&world, VID), 819);
        assert!(world.bodies[&Vid::new(VID)].stamina_consume);
        world.step_motion(3);
        assert_eq!(stamina_of(&world, VID), 818);
    }

    /// Without a fight in the window the run is free and the flag stays down, even
    /// on the window's edge.
    #[test]
    fn a_run_without_a_fight_spends_nothing() {
        let clock = TestClock::default();
        clock.set(30_000);
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (3200, 3200), 820);
        world.bodies.get_mut(&Vid::new(VID)).unwrap().last_attack = Some(10_000);
        assert!(world.goto(VID, 4800, 3200));
        world.step_motion(1);
        assert_eq!(stamina_of(&world, VID), 820);
        assert!(!world.bodies[&Vid::new(VID)].stamina_consume);
    }

    /// A step that empties the pool walks the flag at once with one mode record, and
    /// the consume stops.
    #[test]
    fn an_emptied_pool_walks_the_flag_and_fans_once() {
        let clock = TestClock::default();
        clock.set(10_000);
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 1);
        world.bodies.get_mut(&Vid::new(VID)).unwrap().last_attack = Some(10_000);
        assert!(world.goto(VID, 4800, 3200));
        let _ = modes_of(&mut inbox);
        world.step_motion(1);
        assert_eq!(stamina_of(&world, VID), 0);
        assert!(world.bodies[&Vid::new(VID)].walk.now_walking);
        assert!(!world.bodies[&Vid::new(VID)].stamina_consume);
        assert_eq!(
            modes_of(&mut inbox),
            [vec![0x6f, 7, 0, 0, 0, WALKMODE_WALK]]
        );
    }

    /// The first spend starts the gauge once: a zero-change record and the client's
    /// `StartStaminaConsume` line with the rate and the pool; the next spend is silent.
    #[test]
    fn the_first_spend_starts_the_gauge_once() {
        let clock = TestClock::default();
        clock.set(10_000);
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 820);
        world.bodies.get_mut(&Vid::new(VID)).unwrap().last_attack = Some(10_000);
        assert!(world.goto(VID, 4800, 3200));
        let _ = all_of(&mut inbox);
        world.step_motion(1);
        world.step_motion(3);
        let mover = world.own_mover(Vid::new(VID)).unwrap();
        let start = crate::chat_line::chat_packet(
            mover.recipient(&world.locale),
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            b"StartStaminaConsume %d %d",
            &[
                crate::chat_line::Arg::Int(25),
                crate::chat_line::Arg::Int(819),
            ],
        );
        assert_eq!(
            all_of(&mut inbox),
            [zero_change(VID, 819), start],
            "each spend is silent; the start runs once"
        );
    }

    /// The stop runs its record and its line once: a second stop is silent.
    #[test]
    fn the_stop_runs_its_record_and_line_once() {
        let clock = TestClock::default();
        clock.set(10_000);
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 820);
        world.bodies.get_mut(&Vid::new(VID)).unwrap().last_attack = Some(10_000);
        assert!(world.goto(VID, 4800, 3200));
        let _ = all_of(&mut inbox);
        world.step_motion(1);
        let _ = all_of(&mut inbox);
        world.stop_stamina_consume(Vid::new(VID));
        world.stop_stamina_consume(Vid::new(VID));
        let mover = world.own_mover(Vid::new(VID)).unwrap();
        let stop = crate::chat_line::chat_packet(
            mover.recipient(&world.locale),
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            b"StopStaminaConsume %d",
            &[crate::chat_line::Arg::Int(819)],
        );
        assert_eq!(all_of(&mut inbox), [zero_change(VID, 819), stop]);
        assert!(!world.bodies[&Vid::new(VID)].stamina_consume);
    }

    /// A pool that climbs off 0 restores the mode line's point: a runner runs with a
    /// record, a walker stays walking in silence.
    #[test]
    fn a_climb_off_zero_restores_the_mode_lines_point() {
        use common::point_slot as point;

        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 820);
        world.change_pc_point(Vid::new(VID), point::POINT_STAMINA, -820, false);
        assert_eq!(stamina_of(&world, VID), 0);
        assert!(world.bodies[&Vid::new(VID)].walk.now_walking);
        let _ = modes_of(&mut inbox);
        world.change_pc_point(Vid::new(VID), point::POINT_STAMINA, 100, false);
        assert_eq!(stamina_of(&world, VID), 100);
        assert!(!world.bodies[&Vid::new(VID)].walk.now_walking);
        assert_eq!(modes_of(&mut inbox), [vec![0x6f, 7, 0, 0, 0, WALKMODE_RUN]]);
        world.run_walk_mode(Vid::new(VID), true);
        world.change_pc_point(Vid::new(VID), point::POINT_STAMINA, -100, false);
        assert_eq!(stamina_of(&world, VID), 0);
        let _ = modes_of(&mut inbox);
        world.change_pc_point(Vid::new(VID), point::POINT_STAMINA, 100, false);
        assert!(world.bodies[&Vid::new(VID)].walk.now_walking);
        assert!(modes_of(&mut inbox).is_empty(), "already walking: silent");
    }

    /// A walk past five seconds restores the pool; short of it nothing moves.
    #[test]
    fn a_walk_past_five_seconds_restores_the_pool() {
        use common::point_slot as point;

        let clock = TestClock::default();
        clock.set(10_000);
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (3200, 3200), 820);
        world.change_pc_point(Vid::new(VID), point::POINT_STAMINA, -20, false);
        world.run_walk_mode(Vid::new(VID), true);
        assert!(world.goto(VID, 4800, 3200));
        clock.set(15_000);
        world.step_motion(1);
        assert_eq!(
            stamina_of(&world, VID),
            800,
            "five seconds is not past five"
        );
        clock.set(15_001);
        world.step_motion(3);
        let max = world
            .characters
            .find_by_vid(Vid::new(VID))
            .unwrap()
            .points()
            .unwrap()
            .max_stamina();
        assert_eq!(stamina_of(&world, VID), max);
    }

    /// A walk past five seconds on a full pool sends nothing (`:814`).
    #[test]
    fn a_walk_past_five_seconds_on_a_full_pool_is_silent() {
        let clock = TestClock::default();
        clock.set(10_000);
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 820);
        assert_eq!(stamina_of(&world, VID), 820);
        assert_eq!(
            world
                .characters
                .find_by_vid(Vid::new(VID))
                .unwrap()
                .points()
                .unwrap()
                .max_stamina(),
            820,
            "the pool starts full"
        );
        world.run_walk_mode(Vid::new(VID), true);
        assert!(world.goto(VID, 4800, 3200));
        let _ = all_of(&mut inbox);
        clock.set(15_001);
        world.step_motion(1);
        assert!(all_of(&mut inbox).is_empty(), "a full pool sends nothing");
    }

    /// Past the fight window a step stops the gauge: a zero-change record and the
    /// client's `StopStaminaConsume` line with the pool.
    #[test]
    fn a_step_past_the_window_stops_the_gauge() {
        let clock = TestClock::default();
        clock.set(10_000);
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 820);
        world.bodies.get_mut(&Vid::new(VID)).unwrap().last_attack = Some(10_000);
        assert!(world.goto(VID, 4800, 3200));
        let _ = all_of(&mut inbox);
        world.step_motion(1);
        let _ = all_of(&mut inbox);
        clock.set(30_001);
        world.step_motion(3);
        let mover = world.own_mover(Vid::new(VID)).unwrap();
        let stop = crate::chat_line::chat_packet(
            mover.recipient(&world.locale),
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            b"StopStaminaConsume %d",
            &[crate::chat_line::Arg::Int(819)],
        );
        assert_eq!(all_of(&mut inbox), [zero_change(VID, 819), stop]);
        assert_eq!(
            stamina_of(&world, VID),
            819,
            "past the window nothing spends"
        );
        assert!(!world.bodies[&Vid::new(VID)].stamina_consume);
    }

    /// An arrival stops the gauge, with the same record and line.
    #[test]
    fn an_arrival_stops_the_gauge() {
        let clock = TestClock::default();
        clock.set(10_000);
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, VID, (3200, 3200), 820);
        world.bodies.get_mut(&Vid::new(VID)).unwrap().last_attack = Some(10_000);
        assert!(world.goto(VID, 4800, 3200));
        let _ = all_of(&mut inbox);
        world.step_motion(1);
        let _ = all_of(&mut inbox);
        clock.set(20_000);
        world.step_motion(3);
        let mover = world.own_mover(Vid::new(VID)).unwrap();
        let stop = crate::chat_line::chat_packet(
            mover.recipient(&world.locale),
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            b"StopStaminaConsume %d",
            &[crate::chat_line::Arg::Int(818)],
        );
        assert_eq!(all_of(&mut inbox), [zero_change(VID, 818), stop]);
        assert!(!world.bodies[&Vid::new(VID)].stamina_consume);
    }

    /// A walker with no stamp starts the five seconds on its first step: legacy leaves
    /// `m_dwWalkStartTime` uninitialized, so a stamina-0 walker reads garbage (a Defect).
    #[test]
    fn an_unstamped_walk_starts_its_five_seconds_on_the_first_step() {
        let clock = TestClock::default();
        clock.set(60_000);
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (3200, 3200), 0);
        assert!(world.bodies[&Vid::new(VID)].walk_start_ms.is_none());
        assert!(world.goto(VID, 4800, 3200));
        world.step_motion(1);
        assert_eq!(
            world.bodies[&Vid::new(VID)].walk_start_ms,
            Some(60_000),
            "the first walking step stamps"
        );
        assert_eq!(stamina_of(&world, VID), 0, "no restore on the first step");
        clock.set(65_001);
        world.step_motion(3);
        assert_eq!(
            stamina_of(&world, VID),
            world
                .characters
                .find_by_vid(Vid::new(VID))
                .unwrap()
                .points()
                .unwrap()
                .max_stamina()
        );
    }

    /// A rider spends nothing, fight or no.
    #[test]
    fn a_ride_spends_nothing() {
        let clock = TestClock::default();
        clock.set(10_000);
        let mut world = a_world(&clock);
        let _inbox = enter(&mut world, VID, (3200, 3200), 820);
        let horse = world.pc_points_mut(VID).unwrap().horse_mut();
        horse.set_level(5, 0);
        assert!(horse.start_riding(), "the horse carries");
        world.bodies.get_mut(&Vid::new(VID)).unwrap().last_attack = Some(10_000);
        assert!(world.goto(VID, 4800, 3200));
        world.step_motion(1);
        assert_eq!(stamina_of(&world, VID), 820);
    }

    /// With the half-stamina unique worn, odd Pulses spend and even Pulses skip.
    #[test]
    fn the_half_stamina_unique_skips_even_pulses() {
        use common::enums::EWearPositions;
        use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
        use protocol::item_pos::ItemPos;
        use world::item::Item;

        let clock = TestClock::default();
        clock.set(10_000);
        let mut world = a_world(&clock);
        let mut charm = Item::new(11, super::HALF_STAMINA);
        charm.set_size(1).unwrap();
        let cell = INVENTORY_MAX_NUM + EWearPositions::Unique1 as u16;
        let _seen = enter_seeing_with(
            &mut world,
            crate::game_state::fixtures::PLACE,
            VID,
            (3200, 3200),
            820,
            &[(ItemPos::new(EWindows::Inventory as u8, cell), charm)],
        );
        assert!(world.wears_half_stamina(Vid::new(VID)), "the charm is worn");
        world.bodies.get_mut(&Vid::new(VID)).unwrap().last_attack = Some(10_000);
        assert!(world.goto(VID, 4800, 3200));
        world.step_motion(2);
        assert_eq!(stamina_of(&world, VID), 820, "an even Pulse skips");
        world.step_motion(3);
        assert_eq!(stamina_of(&world, VID), 819, "an odd Pulse spends");
    }

    /// The line is quiet when the caller is not in the world.
    #[test]
    fn an_unknown_caller_runs_nothing() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        world.run_walk_mode(Vid::new(VID), true);
    }
}
