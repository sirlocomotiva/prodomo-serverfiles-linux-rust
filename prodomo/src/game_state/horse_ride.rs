//! The ride: `CHARACTER::StartRiding` and `StopRiding` (`G/char_horse.cpp:16-113`),
//! `MountVnum` (`G/char.cpp:7852-7883`), the GM line `/horse_ride`
//! (`G/cmd_gm.cpp:3800-3806`) and the player's `/user_horse_ride`
//! (`G/cmd_general.cpp:58-86`).
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

use super::view::{Effect, EntityKey};
use super::GameState;

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
    fn mount_vnum(&mut self, vid: Vid, vnum: u32) {
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
