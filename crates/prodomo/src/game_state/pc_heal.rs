//! A player's heal on its `recovery_event` (`G/char.cpp:3245-3284`).

use common::point_slot as point;
use world::character::Points;
use world::character::{distribute_sp, hp_recovery};

/// `DistributeSP` (`G/char_battle.cpp:141-217`), then the hit-point heal (`:3261-3283`),
/// both through `PointChange` so a dead player is refused and the pools clamp. Poison
/// blocks both (`:3252-3258`); the polymorph arm is not ported (no polymorph). Returns
/// the records the player's own client is sent, in legacy order: spirit first.
pub(super) fn pc_heal(
    points: &mut Points,
    skill_group: u8,
    attacking: bool,
    moving: bool,
    idle_ms: u32,
    poisoned: bool,
) -> Vec<world::character::PointRecord> {
    if poisoned {
        return Vec::new();
    }
    let mut records = Vec::new();
    if let Some(amount) = distribute_sp(points, skill_group, attacking, moving) {
        if let Ok(sp) = points.point_change(point::POINT_SP, amount, false, false) {
            records.extend(sp);
        }
    }
    if let Some(amount) = hp_recovery(points, idle_ms) {
        if let Ok(hp) = points.point_change(point::POINT_HP, amount, false, false) {
            records.extend(hp);
        }
    }
    records
}

#[cfg(test)]
mod tests {
    use common::point_slot as point;
    use common::vid::Vid;
    use gamedata::mob_proto::BATTLE_TYPE_MELEE;
    use protocol::gc_inventory::{HEADER_GC_PLAYER_POINT_CHANGE, HEADER_GC_TARGET};
    use protocol::gc_nested::GcPointChange;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::character::{distribute_sp, hp_recovery, Pcg32};
    use world::npc::FIRST_NPC_VID;

    use super::super::fixtures::{a_fighter, enter, open_cells, stand_field, TestClock};
    use super::super::GameState;
    use crate::game_loop::PulseProcessor;

    const ME: u32 = FIRST_NPC_VID;
    const SPOT: (i32, i32) = (3200, 3200);
    const NEAR: (i32, i32) = (3100, 3200);
    const T0: u32 = 1_000;
    const P0: u64 = 1;

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The player 1 at [`NEAR`] after Pulse [`P0`], with `npcs` stood; the inbox is drained.
    fn a_field(
        clock: &TestClock,
        npcs: Vec<world::npc::Npc>,
    ) -> (GameState, UnboundedReceiver<Vec<u8>>) {
        let mut world = stand_field(clock, npcs, open_cells(), true);
        let mut inbox = enter(&mut world, 1, NEAR, 12_345);
        world.start_pc_recovery(Vid::new(1));
        world.process_pulse(P0);
        drained(&mut inbox);
        (world, inbox)
    }

    fn points_of(world: &mut GameState) -> world::character::Points {
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .unwrap()
            .items_and_points_mut()
            .1
            .unwrap()
            .clone()
    }

    /// The heal sends spell points then hit points (G30).
    #[test]
    fn the_heal_sends_spell_points_then_hit_points() {
        let clock = TestClock::default();
        clock.set(T0);
        let (mut world, mut alpha) = a_field(&clock, Vec::new());
        world.damage_pc(None, Vid::new(1), 100, None);
        {
            let points = world
                .characters
                .find_by_vid_mut(Vid::new(1))
                .unwrap()
                .items_and_points_mut()
                .1
                .unwrap();
            let _ = points.point_change(point::POINT_SP, -50, false, false);
        }
        drained(&mut alpha);
        // The tick's amounts, from the pure rules: idle 0, still, unhurried.
        let snapshot = points_of(&mut world);
        let sp = distribute_sp(&snapshot, 0, false, true).expect("spell points below the maximum");
        let hp = hp_recovery(&snapshot, 0).expect("hit points below the maximum");
        let (hp_before, sp_before) = (snapshot.hp(), snapshot.sp());
        for pulse in P0 + 1..=P0 + 75 {
            world.process_pulse(pulse);
        }
        let heard: Vec<Vec<u8>> = drained(&mut alpha)
            .into_iter()
            .filter(|record| record[0] == HEADER_GC_PLAYER_POINT_CHANGE.value())
            .collect();
        assert_eq!(heard.len(), 2, "spirit first, then flesh: {heard:?}");
        let first = GcPointChange::decode(&heard[0]).expect("a point change");
        let second = GcPointChange::decode(&heard[1]).expect("a point change");
        // `show_amount` is false, so the records carry 0 with the new value.
        assert_eq!(
            (first.change_type, first.amount, first.value),
            (
                u8::try_from(point::POINT_SP).unwrap(),
                0,
                i64::from(sp_before + sp)
            )
        );
        assert_eq!(
            (second.change_type, second.amount, second.value),
            (
                u8::try_from(point::POINT_HP).unwrap(),
                0,
                i64::from(hp_before + hp)
            )
        );
    }

    /// The heal is silent at the maxima (G31).
    #[test]
    fn the_heal_is_silent_at_the_maxima() {
        let clock = TestClock::default();
        clock.set(T0);
        let (mut world, mut alpha) = a_field(&clock, Vec::new());
        for pulse in P0 + 1..=P0 + 75 {
            world.process_pulse(pulse);
        }
        assert!(
            !drained(&mut alpha)
                .iter()
                .any(|record| record[0] == HEADER_GC_PLAYER_POINT_CHANGE.value()),
            "nothing to heal, nothing sent"
        );
    }

    /// A monster hit resets the victim's heal to one percent (G32).
    #[test]
    fn a_monster_hit_resets_the_victims_heal_to_one_percent() {
        let clock = TestClock::default();
        clock.set(T0);
        let npc = a_fighter(ME, 406, BATTLE_TYPE_MELEE, (175, 2000), 0, SPOT, 10_000);
        let (mut world, mut alpha) = a_field(&clock, vec![npc]);
        world.damage_pc(None, Vid::new(1), 100, None);
        drained(&mut alpha);
        // Four seconds idle: the tick would heal the five-percent share. The swing stamps
        // `last_move`, so the tick heals the one-percent share instead.
        clock.set(T0 + 4000);
        world.dice = Pcg32::new(0, 0);
        assert!(world.mob_attack(ME, Vid::new(1)));
        let snapshot = points_of(&mut world);
        let one = hp_recovery(&snapshot, 0).expect("hit points below the maximum");
        let five = hp_recovery(&snapshot, 4000).expect("hit points below the maximum");
        assert_ne!(one, five, "the test tells the shares apart");
        drained(&mut alpha);
        for pulse in P0 + 1..=P0 + 75 {
            world.process_pulse(pulse);
        }
        let heard: Vec<Vec<u8>> = drained(&mut alpha)
            .into_iter()
            .filter(|record| record[0] == HEADER_GC_PLAYER_POINT_CHANGE.value())
            .collect();
        assert_eq!(heard.len(), 1, "the hit-point tick only: {heard:?}");
        let tick = GcPointChange::decode(&heard[0]).expect("a point change");
        assert_eq!(
            (tick.change_type, tick.amount, tick.value),
            (
                u8::try_from(point::POINT_HP).unwrap(),
                0,
                i64::from(snapshot.hp() + one)
            )
        );
        let _ = HEADER_GC_TARGET;
    }
}
