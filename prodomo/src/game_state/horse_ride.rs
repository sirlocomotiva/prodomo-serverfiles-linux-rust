//! The ride: `CHARACTER::StartRiding` and `StopRiding` (`G/char_horse.cpp:16-113`),
//! `MountVnum` (`G/char.cpp:7852-7883`), the GM line `/horse_ride`
//! (`G/cmd_gm.cpp:3800-3806`), the player's `/user_horse_ride`
//! (`G/cmd_general.cpp:58-86`), `/ride` and `/unmount`, and the quest mount (`pc.mount`,
//! `pc.mount_bonus`, `pc.unmount`).
//!
//! Starting a ride needs a living caller, no morph, no body armour 11901 to 11904, a horse
//! of level 1 or more with health and stamina, and none ridden: the failures say `[LS;1936]`,
//! `[LS;882]`, `[LS;883]`, `[LS;884]`, `[LS;885]` and `[LS;886]` in that order. The mount
//! vnum is the summoned horse's race, else the level's (`GetMyHorseVnum`); the horse goes,
//! the mount lands on the points, `MountVnum`'s insert pair re-shows the rider to itself
//! and to each entity it sees (no removals, no return inserts), and the full compute and
//! the update follow. The guild-war guard, the arena silence and the mount costume are
//! gaps: the Rewrite hosts no war map, no arena and no costume.
//!
//! Stopping a ride runs the horse's stop, and, while alive and unstunned, clears the mount,
//! re-shows, recomputes, updates and summons the horse back from far with the old vnum;
//! dead or stunned it clears the mount, recomputes and updates only. The four zero
//! attribute changes run either way. The quest `Unmount` hook is a gap: no live quest
//! names `unmount` (`quest_functions` lists it, nothing handles it).
//!
//! Everything here runs on the game thread only (ADR-0002).

use common::enums::EWearPositions;
use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
use common::point_slot as point;
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::item_pos::ItemPos;
use world::character::is_ride_item;

use super::affect::Affect;
use super::view::{Effect, EntityKey};
use super::GameState;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};
use crate::loading_phase::{AFFECT_MOUNT, AFFECT_MOUNT_BONUS};

impl GameState {
    /// `do_user_horse_ride` (`G/cmd_general.cpp:58-86`): an observer, the dead and the
    /// stunned are silent; a rider stops; a mount costume says `[LS;1054]`; no summoned
    /// horse says `[LS;501]`; else the caller mounts.
    pub(super) fn user_horse_ride(&mut self, vid: Vid) {
        if self.is_observing(vid) || self.is_dead(vid.raw()) || self.is_stunned(vid.raw()) {
            return;
        }
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        if points.horse().riding() {
            self.stop_riding(vid);
            return;
        }
        if points.mount_vnum() != 0 {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1054]", &[]);
            }
            return;
        }
        if self
            .pc_points_mut(vid.raw())
            .is_none_or(|points| !points.horse().summoned())
        {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;501]", &[]);
            }
            return;
        }
        self.start_riding(vid);
    }

    /// `do_horse_ride` (`G/cmd_gm.cpp:3800-3806`): the caller stops its ride, or starts it.
    pub(super) fn run_horse_ride(&mut self, vid: Vid) {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        if points.horse().riding() {
            self.stop_riding(vid);
        } else {
            self.start_riding(vid);
        }
    }

    /// `CHARACTER::StartRiding` (`G/char_horse.cpp:16-76`): the gated mount. The caller's
    /// mount lands under the summoned horse's race, else under the level's vnum.
    pub(super) fn start_riding(&mut self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        if self.is_dead(vid.raw()) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1936]", &[]);
            return;
        }
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        if points.is_polymorphed() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;882]", &[]);
            return;
        }
        if self.body_armor_11901_to_11904(vid) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;883]", &[]);
            return;
        }
        let horse_vnum = self
            .pc_points_mut(vid.raw())
            .and_then(|points| points.horse().summoned_vid())
            .and_then(|horse_vid| self.npc_by_vid(horse_vid))
            .map(|npc| npc.vnum);
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let horse = points.horse_mut();
        let gate = if horse.level() == 0 {
            Some(b"[LS;884]".as_slice())
        } else if horse.health() <= 0 {
            Some(b"[LS;885]".as_slice())
        } else if horse.stamina() <= 0 || horse.riding() {
            Some(b"[LS;886]".as_slice())
        } else {
            None
        };
        if let Some(line) = gate {
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return;
        }
        let level = self
            .pc_points_mut(vid.raw())
            .map_or(0, |points| points.horse().level());
        let mount = horse_vnum.unwrap_or_else(|| self.my_horse_vnum(vid, level));
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            let _mounted = points.horse_mut().start_riding();
        }
        self.horse_unsummon(vid, false);
        self.sync_horse_timer(vid);
        self.mount_vnum(vid, mount);
    }

    /// `CHARACTER::StopRiding` (`G/char_horse.cpp:79-113`): the rider gets off. While alive
    /// and unstunned the horse is summoned back from far with the old mount vnum.
    pub(super) fn stop_riding(&mut self, vid: Vid) {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        if !points.horse_mut().stop_riding() {
            return;
        }
        self.sync_horse_timer(vid);
        self.flush_horse(vid);
        let down = self.is_dead(vid.raw()) || self.is_stunned(vid.raw());
        let old = self
            .pc_points_mut(vid.raw())
            .map_or(0, |points| points.mount_vnum());
        if down {
            self.set_mount(vid, 0);
            self.recompute_and_update(vid);
        } else {
            self.mount_vnum(vid, 0);
            self.horse_summon(vid, true, old);
        }
        for kind in [
            point::POINT_ST,
            point::POINT_DX,
            point::POINT_HT,
            point::POINT_IQ,
        ] {
            self.change_pc_point(vid, kind, 0, false);
        }
    }

    /// `MountVnum` (`G/char.cpp:7852-7883`): the mount lands, the rider is re-shown to
    /// itself and to each entity it sees (`MountVnum` sends no removals and no return
    /// inserts, unlike `ViewReencode`), and the points recompute with their update. An
    /// unchanged vnum sends nothing, as a rider in no sectree sends nothing either.
    pub(super) fn mount_vnum(&mut self, vid: Vid, vnum: u32) {
        let changed = self
            .pc_points_mut(vid.raw())
            .is_some_and(|points| points.mount_vnum() != vnum);
        if !changed {
            return;
        }
        self.mount_swaps(vid, vnum);
        self.mount_land(vid, vnum);
    }

    /// The rest of `StopRiding` when the horse's ride already ended by an update
    /// ([`world::character::Horse::take_stopped`]): the lines, the mount, the re-show,
    /// the recompute with its update, the far summon of the horse under the old vnum,
    /// and the four zero changes. The world calls this at most once per stop, so the
    /// reentrant stop inside is already spent.
    pub(super) fn finish_riding_stop(&mut self, vid: Vid) {
        self.sync_horse_timer(vid);
        self.flush_horse(vid);
        let down = self.is_dead(vid.raw()) || self.is_stunned(vid.raw());
        let old = self
            .pc_points_mut(vid.raw())
            .map_or(0, |points| points.mount_vnum());
        if down {
            self.set_mount(vid, 0);
            self.recompute_and_update(vid);
        } else {
            self.mount_vnum(vid, 0);
            self.horse_summon(vid, true, old);
        }
        for kind in [
            point::POINT_ST,
            point::POINT_DX,
            point::POINT_HT,
            point::POINT_IQ,
        ] {
            self.change_pc_point(vid, kind, 0, false);
        }
    }

    /// `pc_mount` (`G/questlua_pc.cpp:1392-1458`) for the player under `vid`: the mount
    /// affect and its bonus go, and a summoned horse goes (`HorseSummon(false)`). Then, for
    /// a vnum, `AFFECT_MOUNT` lays `POINT_MOUNT` for `seconds`, whose `MountVnum` mounts;
    /// the eighteen mount races of the three speed bands add 30, 40 or 50 movement speed
    /// as a second `AFFECT_MOUNT` row, which `IsCube` keys on its point. No war map is
    /// hosted, so the `g_NoMountAtGuildWar` arm never runs.
    pub(super) fn quest_mount(&mut self, vid: Vid, vnum: u32, seconds: i32) {
        let _mount = self.remove_pc_affect(vid, AFFECT_MOUNT);
        let _bonus = self.remove_pc_affect(vid, AFFECT_MOUNT_BONUS);
        if self
            .pc_points_mut(vid.raw())
            .is_some_and(|points| points.horse().summoned())
        {
            self.horse_unsummon(vid, false);
        }
        if vnum == 0 {
            return;
        }
        let row = |apply_on: usize, value: i32| Affect {
            kind: AFFECT_MOUNT,
            apply_on: u8::try_from(apply_on).unwrap_or(0),
            value,
            flag: 0,
            remaining: seconds,
            sp_cost: 0,
        };
        let mount = row(point::POINT_MOUNT, i32::from_ne_bytes(vnum.to_ne_bytes()));
        self.add_pc_affect(vid, mount, true);
        let speed = match vnum {
            20_201..=20_204 | 20_213 | 20_216 => 30,
            20_205..=20_208 | 20_214 | 20_217 => 40,
            20_209..=20_212 | 20_215 | 20_218 => 50,
            _ => return,
        };
        self.add_pc_affect_keyed(vid, row(point::POINT_MOV_SPEED, speed), true, true);
    }

    /// `ACMD(do_ride)` (`G/cmd_general.cpp:3516-3642`) for the player under `vid`: the dead,
    /// the stunned and the OX map (113) are silent and a morph hears it may not mount; a
    /// horse rider stops, a mount rider dismounts (`do_unmount`), and with a summoned horse
    /// the caller rides. Otherwise the first inventory cell holding a ride item, one of the
    /// four quest seals 71114-71120 or a mount seal 52001-52090 runs through `UseItem`, and
    /// with none `[LS;501]` answers.
    ///
    /// The mount-costume arm never runs: every mount costume in the owner's data carries the
    /// first-use timer the use refuses, so none is worn, and the ride item's three-cell
    /// condition always holds for the same reason.
    ///
    /// # Errors
    ///
    /// As [`GameState::use_item`].
    pub(super) fn run_ride_line(
        &mut self,
        vid: Vid,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        match self.ride_toggle(vid) {
            Some(cell) => self.use_item(vid, ItemPos::new(EWindows::Inventory as u8, cell), actor),
            None => self.nothing_used(vid, actor),
        }
    }

    /// `do_ride`'s arms before its item use: the inventory cell to use, if any.
    fn ride_toggle(&mut self, vid: Vid) -> Option<u16> {
        if self.is_down(vid.raw()) || self.place_of(vid.raw()).is_some_and(|(_, map)| map == 113) {
            return None;
        }
        let points = self.pc_points_mut(vid.raw())?;
        let (polymorphed, mount) = (points.is_polymorphed(), points.mount_vnum());
        let (riding, summoned) = (points.horse().riding(), points.horse().summoned());
        if polymorphed {
            if let Some(mover) = self.own_mover(vid) {
                let line = b"Nu poti folosi un mount atat timp cat esti transformat.";
                self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            }
            return None;
        }
        if riding {
            self.stop_riding(vid);
            return None;
        }
        if mount != 0 {
            self.unmount(vid);
            return None;
        }
        if summoned {
            self.start_riding(vid);
            return None;
        }
        let found = self.characters.find_by_vid(vid).ok().and_then(|character| {
            (0..INVENTORY_MAX_NUM).find(|&cell| {
                let at = ItemPos::new(EWindows::Inventory as u8, cell);
                character.items().item_at(at).is_some_and(|item| {
                    matches!(
                        item.vnum,
                        71_114 | 71_116 | 71_118 | 71_120 | 52_001..=52_090
                    ) || self.protos.get(item.vnum).is_some_and(is_ride_item)
                })
            })
        });
        if found.is_none() {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;501]", &[]);
            }
        }
        found
    }

    /// `pc_unmount` (`G/questlua_pc.cpp:1481-1489`) for the player under `vid`: the mount
    /// affect and its bonus go, and a horse ride stops.
    pub(super) fn unmount(&mut self, vid: Vid) {
        let _mount = self.remove_pc_affect(vid, AFFECT_MOUNT);
        let _bonus = self.remove_pc_affect(vid, AFFECT_MOUNT_BONUS);
        if self.is_horse_riding(vid.raw()) {
            self.stop_riding(vid);
        }
    }

    /// `pc_mount_bonus` (`G/questlua_pc.cpp:1461-1479`) for the player under `vid`: without
    /// a mount nothing (`@fixme134`); else the bonus goes and comes back as
    /// `AFFECT_MOUNT_BONUS` on `aApplyInfo[apply].bPointType` for `seconds`. An apply type
    /// past the table is refused: legacy reads `aApplyInfo` out of bounds there, a Defect.
    pub(super) fn mount_bonus(&mut self, vid: Vid, apply: u8, value: i32, seconds: i32) {
        if self
            .pc_points_mut(vid.raw())
            .is_none_or(|points| points.mount_vnum() == 0)
        {
            return;
        }
        let Some(point) = world::character::point_type(apply) else {
            return;
        };
        let _bonus = self.remove_pc_affect(vid, AFFECT_MOUNT_BONUS);
        let bonus = Affect {
            kind: AFFECT_MOUNT_BONUS,
            apply_on: u8::try_from(point).unwrap_or(0),
            value,
            flag: 0,
            remaining: seconds,
            sp_cost: 0,
        };
        self.add_pc_affect(vid, bonus, false);
    }

    /// `CHARACTER::IsRiding` (`G/char.cpp:8783-8786`) of the player under `vid`: its horse
    /// ridden, or a mount vnum. False for anything that is not a player.
    pub(super) fn is_riding(&self, vid: u32) -> bool {
        self.points_of(vid)
            .is_some_and(|points| points.horse().riding() || points.mount_vnum() != 0)
    }

    /// `IsHorseRiding` (`G/horse_rider.h:54`) of the player under `vid`: its horse ridden.
    pub(super) fn is_horse_riding(&self, vid: u32) -> bool {
        self.points_of(vid)
            .is_some_and(|points| points.horse().riding())
    }

    /// `CanAttack`'s horse gate (`G/pvp.cpp:394-398`): a player riding its horse at grade 1
    /// attacks nothing, monsters included.
    pub(super) fn rides_a_grade_one_horse(&self, vid: u32) -> bool {
        self.points_of(vid).is_some_and(|points| {
            let horse = points.horse();
            horse.riding() && horse.level() > 0 && horse.grade() == 1
        })
    }

    /// `CanUseHorseSkill` (`G/char_horse.cpp:369-393`): riding on a grade-3 horse. The mount
    /// arms after the grade's `return` never run.
    pub(super) fn can_use_horse_skill(&self, vid: u32) -> bool {
        self.is_riding(vid)
            && self
                .points_of(vid)
                .is_some_and(|points| points.horse().grade() == 3)
    }

    /// `MountVnum`'s recursion (`G/char.cpp:7857-7860`): a mount swapped for another
    /// clears first.
    fn mount_swaps(&mut self, vid: Vid, vnum: u32) {
        let swaps = vnum != 0
            && self
                .pc_points_mut(vid.raw())
                .is_some_and(|points| points.mount_vnum() != 0);
        if swaps {
            self.mount_vnum(vid, 0);
        }
    }

    /// `MountVnum` past its guards: the mount lands, the re-show goes out and the
    /// points recompute with their update.
    fn mount_land(&mut self, vid: Vid, vnum: u32) {
        let Some(seen) = self
            .place_of(vid.raw())
            .and_then(|place| self.maps.get(&place))
            .and_then(|index| index.viewers(EntityKey::Character(vid.raw())))
        else {
            return;
        };
        self.set_mount(vid, vnum);
        let me = EntityKey::Character(vid.raw());
        let mut effects = vec![Effect::Insert { of: me, to: me }];
        for ent in seen {
            effects.push(Effect::Insert { of: me, to: ent });
        }
        self.deliver(&effects, None);
        self.recompute_and_update(vid);
    }

    /// The mount's state half on the points, with the rider's skill group in hand.
    fn set_mount(&mut self, vid: Vid, vnum: u32) {
        let group = self
            .bodies
            .get(&vid)
            .map_or(0, |body| body.card.skill_group);
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            points.set_mount_vnum(vnum, group);
        }
    }

    /// `ComputePoints` with its records, then `UpdatePacket` to the neighbourhood.
    fn recompute_and_update(&mut self, vid: Vid) {
        self.sync_horse_named(vid);
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let computed = points.compute_points();
        self.send_point_records(vid, &computed);
        let flags = self.affect_flags_of(vid);
        if let Some(frame) = self.update_frame(vid, flags) {
            self.packet_around(EntityKey::Character(vid.raw()), &frame, None);
        }
    }

    /// The mount refused by a body armour of vnum 11901 to 11904 (`G/char_horse.cpp:36-44`).
    fn body_armor_11901_to_11904(&self, vid: Vid) -> bool {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        character
            .items()
            .item_at(protocol::item_pos::ItemPos::new(
                EWindows::Inventory as u8,
                INVENTORY_MAX_NUM + EWearPositions::Body as u16,
            ))
            .is_some_and(|armor| (11_901..=11_904).contains(&armor.vnum))
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::super::fixtures::{enter, open_cells, stand_field, TestClock};
    use super::*;

    /// The caller's VID.
    const ME: u32 = 1;

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// `do_ride`'s silent gates (`G/cmd_general.cpp:3519-3523`): with nothing to ride the
    /// caller hears `[LS;501]`, but dead (`IsDead() || IsStun()`) or on the OX map (113) it
    /// hears nothing and is handed no cell.
    #[test]
    fn the_down_and_the_ox_map_ride_nothing() {
        let clock = TestClock::default();
        clock.set(1_000);
        let mut world = stand_field(&clock, Vec::new(), open_cells(), true);
        let mut mine = enter(&mut world, ME, (3200, 3200), 12_345);
        drained(&mut mine);
        assert_eq!(world.ride_toggle(Vid::new(ME)), None);
        assert_eq!(drained(&mut mine).len(), 1, "the [LS;501] line");
        world.pc_combat.entry(ME).or_default().dead = true;
        assert_eq!(world.ride_toggle(Vid::new(ME)), None);
        assert!(drained(&mut mine).is_empty(), "the dead are silent");
        world.pc_combat.entry(ME).or_default().dead = false;
        world.bodies.get_mut(&Vid::new(ME)).expect("a body").map = 113;
        assert_eq!(world.ride_toggle(Vid::new(ME)), None);
        assert!(drained(&mut mine).is_empty(), "the OX map is silent");
    }
}
