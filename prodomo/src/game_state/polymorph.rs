//! `/polymorph` (`do_polymorph`, `G/cmd_gm.cpp:3049-3068`): the caller shows as the
//! named mob vnum, or unmorphs on an empty or job vnum.
//!
//! `SetPolymorph` (`G/char.cpp:7680-7724`) sets the race and the maintain flag
//! (`world::character::Points::set_polymorph`), stops the ride, reencodes with the spawn
//! state bit and `AFF_SPAWN`, runs the four attribute changes with amount 0, resets the
//! combo, and recomputes the battle points: the morphed arm (`G/char.cpp:2688-2707`)
//! while a morph is on, the plain arm when it clears. The combo has no port yet, so its
//! reset is a gap; the ride is never on, so stopping it is a no-op.
//!
//! The spawn state bit is held only for the reencode (`REMOVE_BIT`, `:7705`), but
//! `AFF_SPAWN` is never reset (D5), so every later insert and update of the player carries
//! it, as each later insert carries the shown race (`GetRaceNum`).
//!
//! Everything here runs on the game thread only (ADR-0002).

use common::point_slot as point;
use common::vid::Vid;
use world::character::{polymorph_power, SKILL_POLYMORPH};

use super::view::EntityKey;
use super::GameState;
use crate::command::two_arguments;
use crate::game_state::skill_admin::str_to_dword;
use crate::game_state::whisper::str_to_int;
use crate::loading_phase::AFF_SPAWN;

impl GameState {
    /// `do_polymorph` (`G/cmd_gm.cpp:3049-3068`): two words (the vnum and the maintain
    /// flag); an empty line is silent.
    pub(super) fn run_polymorph(&mut self, vid: Vid, rest: &[u8]) {
        let (first, second) = two_arguments(rest);
        if first.is_empty() {
            return;
        }
        let vnum = str_to_dword(&first);
        let maintain = if second.is_empty() {
            false
        } else {
            str_to_int(&second) > 0
        };
        self.set_polymorph(vid, vnum, maintain);
    }

    /// `SetPolymorph` (`G/char.cpp:7680-7724`) on the player under `vid`: the race and
    /// the flag, the stopped ride, the spawn reencode, the four zero changes, and the
    /// battle compute. An unchanged vnum sends nothing.
    pub(super) fn set_polymorph(&mut self, vid: Vid, vnum: u32, maintain: bool) {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        if !points.set_polymorph(vnum, maintain) {
            return;
        }
        // `StopRiding` (`:7697-7698`): the Rewrite never rides, so nothing stops.
        self.reencode_with_spawn(vid);
        if !maintain {
            for kind in [
                point::POINT_ST,
                point::POINT_DX,
                point::POINT_IQ,
                point::POINT_HT,
            ] {
                self.change_pc_point(vid, kind, 0, false);
            }
        }
        // `SetValidComboInterval(0)` and `SetComboSequence(0)` (`:7720-7721`): the combo
        // has no port yet, so its reset is a gap.
        let records = self.compute_for_morph(vid);
        self.send_point_records(vid, &records);
    }

    /// The battle compute `SetPolymorph` ends on: the morphed arm over the vnum's row
    /// while a morph is on (`G/char.cpp:2688-2707`), the plain arm when it clears.
    /// An unknown vnum zeroes the grades, as `CMobManager::Get` missing does.
    fn compute_for_morph(&mut self, vid: Vid) -> Vec<world::character::PointRecord> {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return Vec::new();
        };
        if !points.is_polymorphed() {
            return points.compute_battle_points();
        }
        let vnum = points.polymorph_race();
        let row = self
            .regens
            .respawner()
            .and_then(|respawner| respawner.mob(vnum))
            .map(|mob| (mob.str, mob.con, mob.def));
        let power = self.polymorph_power(vid);
        let group = self
            .bodies
            .get(&vid)
            .map_or(0, |body| body.card.skill_group);
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return Vec::new();
        };
        points.compute_polymorph_battle_points(row, power, group)
    }

    /// `GetPolymorphPower` (`G/char.cpp:7669-7678`): the `poly` event flag on the test
    /// server when it is set, else the skill-129 climb through the power table.
    fn polymorph_power(&self, vid: Vid) -> i32 {
        if self.test_server {
            if let Some(quests) = &self.quests {
                let flag = quests.manager().event_flag(b"poly");
                if flag != 0 {
                    return flag;
                }
            }
        }
        polymorph_power(self.pc_skill_level(vid.raw(), SKILL_POLYMORPH))
    }

    /// The reencode `SetPolymorph` runs (`G/char.cpp:7700-7705`): `AFF_SPAWN` lands on the
    /// caller's base words for good, the spawn state bit is held, the view around the
    /// caller re-shows it (`ViewReencode`), and the bit is dropped again.
    fn reencode_with_spawn(&mut self, vid: Vid) {
        let Some(body) = self.bodies.get_mut(&vid) else {
            return;
        };
        let word = usize::try_from((AFF_SPAWN - 1) / 32).unwrap_or(0);
        body.base_affect_flags[word] |= 1 << ((AFF_SPAWN - 1) % 32);
        body.add_spawn = true;
        let mut effects = Vec::new();
        if let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) {
            if let Some(index) = self.maps.get_mut(&place) {
                // A same-spot `show` is `ViewReencode` (`G/entity_view.cpp:23-45`).
                let _shown = index.show(
                    EntityKey::Character(vid.raw()),
                    (spot.x, spot.y, spot.z),
                    self.view_radius,
                    &mut effects,
                );
            }
        }
        self.deliver(&effects, None);
        if let Some(body) = self.bodies.get_mut(&vid) {
            body.add_spawn = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use protocol::gc_actors::GcCharacterAdd;
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::super::fixtures::{enter, enter_seeing, open_cells, stand_field, TestClock, PLACE};
    use super::*;
    use crate::loading_phase::{ADD_CHARACTER_STATE_DEAD, ADD_CHARACTER_STATE_SPAWN};

    /// The caller's VID.
    const ME: u32 = 1;

    /// Where it stands.
    const SPOT: (i32, i32) = (3200, 3200);

    /// Near it: the witness sees every reencode.
    const NEAR: (i32, i32) = (3100, 3200);

    /// A dog's vnum: the race the witness reads.
    const DOG: u32 = 101;

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// A world with the caller at [`SPOT`] and a witness [`NEAR`], both drained.
    fn a_morph() -> (
        TestClock,
        GameState,
        UnboundedReceiver<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let clock = TestClock::default();
        clock.set(1_000);
        let mut world = stand_field(&clock, Vec::new(), open_cells(), true);
        let mut mine = enter(&mut world, ME, SPOT, 12_345);
        let mut witness = enter(&mut world, 2, NEAR, 12_345);
        drained(&mut mine);
        drained(&mut witness);
        (clock, world, mine, witness)
    }

    /// The caller's `GC_CHARACTER_ADD` among `heard`.
    fn alpha_add(heard: &[Vec<u8>]) -> GcCharacterAdd {
        heard
            .iter()
            .filter_map(|record| GcCharacterAdd::decode(record).ok())
            .find(|add| add.dw_vid == ME)
            .expect("the caller's own add")
    }

    /// `do_polymorph`'s own words: empty is silent, two words name the vnum and the
    /// maintain flag, and a non-positive second word does not maintain.
    #[test]
    fn the_line_is_silent_when_empty_and_names_the_vnum() {
        let (_clock, mut world, mut mine, mut witness) = a_morph();
        world.run_polymorph(Vid::new(ME), b"");
        assert!(drained(&mut mine).is_empty(), "an empty line is silent");
        assert!(
            drained(&mut witness).is_empty(),
            "the witness hears nothing"
        );
        assert!(!world.points_of(ME).expect("points").is_polymorphed());
        world.run_polymorph(Vid::new(ME), b"101 1");
        let points = world.points_of(ME).expect("points");
        assert!(points.is_polymorphed());
        assert_eq!(points.polymorph_race(), DOG);
        world.run_polymorph(Vid::new(ME), b"0");
        assert!(!world.points_of(ME).expect("points").is_polymorphed());
        world.run_polymorph(Vid::new(ME), b"101 0");
        let points = world.points_of(ME).expect("points");
        assert!(points.is_polymorphed());
        assert_eq!(points.polymorph_race(), DOG);
    }

    /// `SetPolymorph`'s vnum gate: a job vnum unmorphs, and an unchanged vnum sends
    /// nothing (`G/char.cpp:7683-7690`).
    #[test]
    fn a_job_vnum_unmorphs_and_an_unchanged_vnum_is_silent() {
        let (_clock, mut world, mut mine, mut witness) = a_morph();
        world.run_polymorph(Vid::new(ME), b"101");
        drained(&mut mine);
        drained(&mut witness);
        world.run_polymorph(Vid::new(ME), b"2");
        assert!(!world.points_of(ME).expect("points").is_polymorphed());
        assert!(!drained(&mut mine).is_empty(), "the clear reencodes");
        drained(&mut witness);
        world.run_polymorph(Vid::new(ME), b"3");
        assert!(drained(&mut mine).is_empty(), "still clear, so silent");
        assert!(
            drained(&mut witness).is_empty(),
            "the witness hears nothing"
        );
    }

    /// The spawn reencode: the caller's own add carries the vnum's low 16 bits, the
    /// spawn state flag, and `AFF_SPAWN` on its affect words (`G/char.cpp:7700-7705`).
    #[test]
    fn the_spawn_add_carries_the_vnum_race_and_the_spawn_words() {
        let (_clock, mut world, _mine, mut witness) = a_morph();
        world.run_polymorph(Vid::new(ME), b"101");
        let heard = drained(&mut witness);
        let add = alpha_add(&heard);
        assert_eq!(add.w_race_num, u16::try_from(DOG).unwrap());
        assert_eq!(add.b_state_flag, ADD_CHARACTER_STATE_SPAWN);
        let slot = usize::try_from((AFF_SPAWN - 1) / 32).unwrap();
        assert_eq!(
            add.dw_affect_flag[slot] & (1 << ((AFF_SPAWN - 1) % 32)),
            1 << ((AFF_SPAWN - 1) % 32)
        );
    }

    /// The witness's own insert stays plain: a re-show that is not the caller carries
    /// no spawn state flag.
    #[test]
    fn a_re_shown_witness_carries_no_spawn_flag() {
        let (_clock, mut world, mut mine, _witness) = a_morph();
        world.run_polymorph(Vid::new(ME), b"101");
        let heard = drained(&mut mine);
        let other = heard
            .iter()
            .filter_map(|record| GcCharacterAdd::decode(record).ok())
            .find(|add| add.dw_vid == 2)
            .expect("the witness's re-show to the caller");
        assert_eq!(other.b_state_flag & ADD_CHARACTER_STATE_SPAWN, 0);
    }

    /// Whether `add` carries `AFF_SPAWN` on its affect words.
    fn has_spawn_affect(add: &GcCharacterAdd) -> bool {
        let slot = usize::try_from((AFF_SPAWN - 1) / 32).unwrap();
        add.dw_affect_flag[slot] & (1 << ((AFF_SPAWN - 1) % 32)) != 0
    }

    /// After the reencode, a late viewer's insert shows the morph's race with no spawn
    /// state bit (`REMOVE_BIT`, `G/char.cpp:7705`) and `AFF_SPAWN` still set, which nothing
    /// resets (D5): not even the clear, after which the late insert shows the job again.
    #[test]
    fn a_late_viewer_sees_the_race_and_the_kept_spawn_affect() {
        let (_clock, mut world, _mine, _witness) = a_morph();
        world.run_polymorph(Vid::new(ME), b"101");
        let (_late, seen) = enter_seeing(&mut world, PLACE, 3, (3300, 3200), 12_345);
        let add = alpha_add(&seen);
        assert_eq!(add.w_race_num, u16::try_from(DOG).unwrap());
        assert_eq!(add.b_state_flag, 0, "the spawn bit was dropped");
        assert!(has_spawn_affect(&add), "AFF_SPAWN stays");

        world.run_polymorph(Vid::new(ME), b"0");
        let (_later, seen) = enter_seeing(&mut world, PLACE, 4, (3200, 3300), 12_345);
        let add = alpha_add(&seen);
        let job = world.points_of(ME).expect("points").race();
        assert_eq!(add.w_race_num, u16::from(job), "the clear shows the job");
        assert!(has_spawn_affect(&add), "AFF_SPAWN outlives the clear");
    }

    /// `/polymorph` runs at `POS_DEAD`: a dead caller's reencode carries the dead bit
    /// beside the spawn bit, as `m_bAddChrState` holds both.
    #[test]
    fn a_dead_callers_reencode_keeps_the_dead_bit() {
        let (_clock, mut world, _mine, mut witness) = a_morph();
        world.pc_combat.entry(ME).or_default().dead = true;
        world.run_polymorph(Vid::new(ME), b"101");
        let add = alpha_add(&drained(&mut witness));
        assert_eq!(
            add.b_state_flag,
            ADD_CHARACTER_STATE_DEAD | ADD_CHARACTER_STATE_SPAWN
        );
    }
}
