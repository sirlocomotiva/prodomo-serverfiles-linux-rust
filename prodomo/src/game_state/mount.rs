//! The costume mount (`ENABLE_MOUNT_COSTUME_SYSTEM`, `G/MountSystem.cpp`,
//! `G/char.cpp:11117-11237`): the follower a worn mount costume summons, its follow,
//! and the ride.
//!
//! Wearing the costume summons (`MountSummon`, `G/item.cpp:1481`): a morph hears the
//! Romanian line, map 113 and the arena maps stay silent, a horse ride stops and a
//! summoned horse goes, and the `VALUE1` mob (or the transmutation target's, as
//! `__CHANGELOOK_SYSTEM__` is defined) stands within 100 of the rider, named
//! `<rider> Mount`, of the rider's empire, facing the rider's heading turned half
//! round. Taking the costume off unsummons (`MountUnsummon`, `G/item.cpp:1526`): a
//! ridden mount of the same mob unmounts first, then the follower goes.
//!
//! `CheckMount` summons when none is summoned and the costume is worn, with no gates
//! of its own: the login gates it on map 113 and the arenas (`G/input_login.cpp:932-936`),
//! and the restart-now arm calls it bare (`G/cmd_general.cpp:515-517`). `/ride` on a worn
//! costume runs in legacy's order: a ridden mount with none summoned unmounts (the
//! follower stands again), an unridden one with exactly one summoned mounts (the follower
//! goes and the ride's affects land). `/unmount` runs the same ridden arm and returns, so
//! a worn costume keeps the unique-item arm from running.
//!
//! The ride is `AFFECT_MOUNT` on `POINT_MOUNT` for the item's socket 0 minus now — or
//! `INFINITE_AFFECT_DURATION` for a mount with no limits at all — plus one
//! `AFFECT_MOUNT_BONUS` per non-`NONE` proto apply, appended beside the old rows. The
//! point arm mounts the vnum, as the quest mount's does.
//!
//! Every 12 Pulses each follower runs `Update`: a dead owner, a dead follower, a lost
//! summon item or an item held by another unsummons it; past 4500 of the rider it
//! re-stands 200 behind the rider's heading; past 300 it runs to 200 of the rider;
//! nearer it waits. A destroyed follower drops its entry, as `DeleteMount` drops the
//! actor. The follow never wanders and never runs, unlike the horse's: legacy's
//! `START_RUN_DISTANCE` and `m_originalMoveSpeed` are write-only, and so is
//! `m_dwLastActionTime`.
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::sync::Arc;

use common::cfloat::{f32_to_i32, i32_to_f32};
use common::vid::Vid;
use gamedata::item_kind::{COSTUME_MOUNT, ITEM_COSTUME};
use protocol::gc_chat::CHAT_TYPE_INFO;
use world::character::{number, point_type, wear_pos, APPLY_NONE};

use super::mob_ai::delta_by_degree;
use super::motion::{distance_sqrt, rotation_to_xy};
use super::view::EntityKey;
use super::GameState;
use crate::item_move::Mover;
use crate::loading_phase::{AFFECT_MOUNT, AFFECT_MOUNT_BONUS, INFINITE_AFFECT_DURATION};
use crate::sync_position::distance_approx;

/// The Pulses between two mount updates: the `PASSES_PER_SEC(1) / 4` event (6 Pulses)
/// with the 400 ms (`m_dwUpdatePeriod`) gate, which skips every other fire, so the
/// follow runs every 12 Pulses (480 ms).
const MOUNT_STATE_PULSES: u64 = 12;
/// The OX map, where `MountSummon` and the login `CheckMount` do nothing.
const OX_MAP: i32 = 113;
/// `MountSummon` and `do_ride` on a morph (`G/char.cpp:11121-11127`,
/// `G/cmd_general.cpp:3524-3528`).
const POLYMORPH_LINE: &[u8] = b"Nu poti folosi un mount atat timp cat esti transformat.";
/// `RESPAWN_DISTANCE`: past here the follower re-stands by the rider.
const RESPAWN_DISTANCE: i32 = 4500;
/// `START_FOLLOW_DISTANCE`: past here the follower runs to the rider.
const START_FOLLOW_DISTANCE: i32 = 300;
/// `APPROACH`: how near the follower comes, and how far behind it re-stands.
const APPROACH: f32 = 200.0;

/// A summoned costume mount standing in the world.
#[derive(Debug, Clone, Copy)]
pub(super) struct MountNpc {
    /// The rider that wears the summoning costume.
    rider: Vid,
    /// The worn costume's item id (`m_dwSummonItemVID`, which is the item's id here).
    summon_item: u32,
    /// The summoned mob (`m_dwVnum`).
    mob_vnum: u32,
}

impl GameState {
    /// The worn mount costume of the character under `vid`, if it wears one.
    fn worn_mount(&self, vid: Vid) -> Option<world::item::Item> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let item = character
            .items()
            .item_at(wear_pos(common::enums::EWearPositions::CostumeMount as u16))
            .cloned()?;
        let proto = self.protos.get(item.vnum)?;
        (proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_MOUNT).then_some(item)
    }

    /// The mob `MountSummon` summons for `item` (`G/char.cpp:11135-11151`): the
    /// transmutation target's `VALUE1` when the costume wears a look whose proto the
    /// table holds, else the costume's own.
    fn mount_mob_vnum(&self, item: &world::item::Item) -> u32 {
        if item.transmutation != 0 {
            if let Some(target) = self.protos.get(item.transmutation) {
                return u32::from_ne_bytes(target.values[1].to_ne_bytes());
            }
        }
        self.protos
            .get(item.vnum)
            .map_or(0, |proto| u32::from_ne_bytes(proto.values[1].to_ne_bytes()))
    }

    /// How many followers the character under `vid` has standing (`CountSummoned`).
    fn count_summoned(&self, vid: Vid) -> usize {
        self.mount_npcs
            .values()
            .filter(|stood| stood.rider == vid)
            .count()
    }

    /// The follower of the character under `vid` with mob `mob`, if one stands.
    fn summoned_mount(&self, vid: Vid, mob: u32) -> Option<u32> {
        self.mount_npcs
            .iter()
            .find(|(_, stood)| stood.rider == vid && stood.mob_vnum == mob)
            .map(|(&stood, _)| stood)
    }

    /// `CHARACTER::MountSummon` (`G/char.cpp:11117-11163`) for the worn `item`: a morph
    /// hears the line, map 113 and the arena maps stay silent, a horse ride stops and
    /// a summoned horse goes, and the mob stands by the rider. A mob the spawn refuses
    /// (among them vnum 0) stands nothing and stays silent, as legacy's null `SpawnMob`
    /// does past its `sys_err`.
    pub(super) fn mount_summon(&mut self, vid: Vid, item: &world::item::Item) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        if self
            .points_of(vid.raw())
            .is_some_and(world::character::Points::is_polymorphed)
        {
            self.tell(vid, mover, CHAT_TYPE_INFO, POLYMORPH_LINE, &[]);
            return;
        }
        if self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| map == OX_MAP || self.is_arena_map(map))
        {
            return;
        }
        if self
            .points_of(vid.raw())
            .is_some_and(|points| points.horse().riding())
        {
            self.stop_riding(vid);
        }
        if self
            .points_of(vid.raw())
            .is_some_and(|points| points.horse().summoned())
        {
            self.horse_unsummon(vid, false);
        }
        let mob = self.mount_mob_vnum(item);
        self.summon_mount(vid, mob, item, mover);
    }

    /// `CMountSystem::Summon` (`G/MountSystem.cpp:432-464`) with `bSpawnFar` false: the
    /// mob stands within 100 of the rider on the rider's map, named `<rider> Mount`,
    /// of the rider's empire, facing the rider's heading turned half round. The update
    /// event is the mount tick ([`GameState::run_mount_ticks`]), which needs no arming.
    fn summon_mount(&mut self, vid: Vid, mob: u32, item: &world::item::Item, mover: Mover) {
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return;
        };
        let dx = number(&mut self.dice, -100, 100);
        let dy = number(&mut self.dice, -100, 100);
        let rotation = self.bodies.get(&vid).map_or(0.0, |body| body.rotation);
        let at = (spot.x.saturating_add(dx), spot.y.saturating_add(dy));
        let Some(mut stood) = self.spawn_horse(place, mob, at, f32_to_i32(rotation + 180.0)) else {
            return;
        };
        let stood_vid = stood.vid;
        stood.name = self.mount_name(vid);
        stood.empire = mover.empire;
        let pulse = self.last_pulse;
        self.stand_runtime(place, vec![stood], pulse);
        self.mount_npcs.insert(
            stood_vid,
            MountNpc {
                rider: vid,
                summon_item: item.id,
                mob_vnum: mob,
            },
        );
    }

    /// `<rider> Mount` (`CMountActor::SetName`, `G/MountSystem.cpp:63-74`).
    fn mount_name(&self, vid: Vid) -> Vec<u8> {
        let mut name = self
            .characters
            .find_by_vid(vid)
            .map_or_else(|_| Vec::new(), |rider| rider.name().as_bytes().to_vec());
        name.extend_from_slice(b" Mount");
        name
    }

    /// `CHARACTER::MountUnsummon` (`G/char.cpp:11165-11194`) for the taken-off `item`: a
    /// ridden mount of the same mob unmounts first, then the follower goes. A mob 0
    /// never unmounts: legacy's `Unmount(0)` finds no actor and only logs.
    pub(super) fn mount_unsummon(&mut self, vid: Vid, item: &world::item::Item) {
        let mob = self.mount_mob_vnum(item);
        if mob != 0
            && self
                .points_of(vid.raw())
                .is_some_and(|points| points.mount_vnum() == mob)
        {
            self.unmount_mount(vid, mob);
        }
        self.unsummon_mount(vid, mob);
    }

    /// `CMountSystem::Unsummon` (`G/MountSystem.cpp:406-430`): the rider's follower of
    /// `mob` goes. The kept actor and the cancelled event are the entry drop: nothing
    /// observable survives an un-summoned actor.
    fn unsummon_mount(&mut self, vid: Vid, mob: u32) {
        if let Some(stood) = self.summoned_mount(vid, mob) {
            self.destroy_npc(stood);
        }
    }

    /// `CMountSystem::GetByVID`'s drop (`DeleteMount`, `G/MountSystem.cpp:391-404`): a
    /// destroyed follower drops its entry.
    pub(super) fn forget_mount_npc(&mut self, stood: u32) {
        self.mount_npcs.remove(&stood);
    }

    /// `CHARACTER::CheckMount` (`G/char.cpp:11196-11226`): with the costume worn and
    /// none summoned, the mob stands. Ungated, as legacy's is; the callers gate.
    pub(super) fn check_mount(&mut self, vid: Vid) {
        let Some(item) = self.worn_mount(vid) else {
            return;
        };
        if self.count_summoned(vid) != 0 {
            return;
        }
        let mob = self.mount_mob_vnum(&item);
        if let Some(mover) = self.own_mover(vid) {
            self.summon_mount(vid, mob, &item, mover);
        }
    }

    /// The login's `CheckMount` (`G/input_login.cpp:932-936`): on map 113 or an arena
    /// map nothing stands.
    pub(super) fn enter_mount(&mut self, vid: Vid) {
        if self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| map == OX_MAP || self.is_arena_map(map))
        {
            return;
        }
        self.check_mount(vid);
    }

    /// After an item step: the taken-off mount unsummons, then the mount cell's new
    /// occupant summons (`CItem::Equip`, `CItem::Unequip`, `G/item.cpp:1478-1482`,
    /// `:1524-1527`), in legacy's unequip-first order. `before` is the item id the
    /// cell held before the step, if any.
    pub(super) fn sync_mount_costume(&mut self, vid: Vid, before: Option<u32>) {
        let after = self.worn_mount(vid);
        if after.as_ref().map(|item| item.id) == before {
            return;
        }
        if let Some(id) = before {
            self.unsummon_mount_item(vid, id);
        }
        if let Some(item) = after {
            self.mount_summon(vid, &item);
        }
    }

    /// The taken-off mount of item `id`: its mob unsummons, ridden or not. The item
    /// itself is read from the rider's storage, where the step left it; a step that
    /// destroyed it (the expiry) unsummons by the follower's own entry instead, with
    /// the same ridden-first order.
    fn unsummon_mount_item(&mut self, vid: Vid, id: u32) {
        let item = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item(id).cloned());
        if let Some(item) = item {
            self.mount_unsummon(vid, &item);
            return;
        }
        let Some(mob) = self
            .mount_npcs
            .values()
            .find(|stood| stood.rider == vid && stood.summon_item == id)
            .map(|stood| stood.mob_vnum)
        else {
            return;
        };
        if mob != 0
            && self
                .points_of(vid.raw())
                .is_some_and(|points| points.mount_vnum() == mob)
        {
            self.unmount_mount(vid, mob);
        }
        self.unsummon_mount(vid, mob);
    }

    /// The leave's mount drop (`CHARACTER::Destroy`, `G/char.cpp:719`): every follower
    /// of the character under `vid` goes with it.
    pub(super) fn leave_mounts(&mut self, vid: Vid) {
        let stood: Vec<u32> = self
            .mount_npcs
            .iter()
            .filter(|(_, stood)| stood.rider == vid)
            .map(|(&stood, _)| stood)
            .collect();
        for stood in stood {
            self.destroy_npc(stood);
        }
    }

    /// `do_ride`'s mount-costume arm (`G/cmd_general.cpp:3529-3570`) for the player under
    /// `vid`: a ridden mount with none summoned unmounts, an unridden one with exactly
    /// one summoned mounts. Answers whether the arm ran, so the caller returns.
    pub(super) fn ride_mount_costume(&mut self, vid: Vid) -> bool {
        let Some(item) = self.worn_mount(vid) else {
            return false;
        };
        let mob = self.mount_mob_vnum(&item);
        let ridden = self
            .points_of(vid.raw())
            .is_some_and(|points| points.mount_vnum() != 0);
        if ridden {
            if self.count_summoned(vid) == 0 {
                self.unmount_mount(vid, mob);
            }
        } else if self.count_summoned(vid) == 1 {
            self.mount_ride(vid, mob, &item);
        }
        true
    }

    /// `do_unmount`'s mount-costume arm (`G/cmd_general.cpp:2315-2350`) for the player
    /// under `vid`: a ridden mount with none summoned unmounts. Answers whether the
    /// arm ran, so the caller returns before the unique-item arm.
    pub(super) fn unmount_line(&mut self, vid: Vid) -> bool {
        let Some(item) = self.worn_mount(vid) else {
            return false;
        };
        let mob = self.mount_mob_vnum(&item);
        if self
            .points_of(vid.raw())
            .is_some_and(|points| points.mount_vnum() != 0)
            && self.count_summoned(vid) == 0
        {
            self.unmount_mount(vid, mob);
        }
        true
    }

    /// `CMountSystem::Mount` (`G/MountSystem.cpp:466-500`): the follower goes, then the
    /// ride's affects land.
    fn mount_ride(&mut self, vid: Vid, mob: u32, item: &world::item::Item) {
        self.unsummon_mount(vid, mob);
        if self
            .points_of(vid.raw())
            .is_some_and(|points| points.horse().riding())
        {
            self.stop_riding(vid);
        }
        let duration = self.mount_ride_duration(item);
        let mount = super::affect::Affect {
            kind: AFFECT_MOUNT,
            apply_on: u8::try_from(common::point_slot::POINT_MOUNT).unwrap_or(0),
            value: i32::from_ne_bytes(mob.to_ne_bytes()),
            flag: 0,
            remaining: duration,
            sp_cost: 0,
        };
        self.add_pc_affect(vid, mount, true);
        if let Some(proto) = self.protos.get(item.vnum) {
            for apply in proto.applies {
                let Ok(kind) = u8::try_from(apply.kind) else {
                    continue;
                };
                if kind == APPLY_NONE {
                    continue;
                }
                if let Some(point) = point_type(kind) {
                    let bonus = super::affect::Affect {
                        kind: AFFECT_MOUNT_BONUS,
                        apply_on: u8::try_from(point).unwrap_or(0),
                        value: apply.value,
                        flag: 0,
                        remaining: duration,
                        sp_cost: 0,
                    };
                    self.add_pc_affect(vid, bonus, false);
                }
            }
        }
    }

    /// The ride's duration (`CMountActor::Mount`, `G/MountSystem.cpp:88`): infinite for
    /// a mount with no limits at all (`IsPermanentMount`, `G/item.cpp:2428-2440`),
    /// else the item's socket 0 minus now, in legacy's `DWORD` wrap.
    fn mount_ride_duration(&self, item: &world::item::Item) -> i32 {
        let permanent = self
            .protos
            .get(item.vnum)
            .is_some_and(|proto| proto.limits.iter().all(|limit| limit.kind == 0));
        if permanent {
            return INFINITE_AFFECT_DURATION;
        }
        i32::from_ne_bytes(
            u32::from_ne_bytes(item.sockets[0].to_ne_bytes())
                .wrapping_sub(super::quests::quest_now())
                .to_ne_bytes(),
        )
    }

    /// `CMountSystem::Unmount` (`G/MountSystem.cpp:502-527`): with the costume worn the
    /// follower stands again, then the ride clears — the affects go, the vnum clears,
    /// a horse ride stops and a summoned horse goes, and the vnum clears again.
    fn unmount_mount(&mut self, vid: Vid, mob: u32) {
        if let Some(item) = self.worn_mount(vid) {
            if let Some(mover) = self.own_mover(vid) {
                self.summon_mount(vid, mob, &item, mover);
            }
        }
        let _mount = self.remove_pc_affect(vid, AFFECT_MOUNT);
        let _bonus = self.remove_pc_affect(vid, AFFECT_MOUNT_BONUS);
        self.mount_vnum(vid, 0);
        if self
            .points_of(vid.raw())
            .is_some_and(|points| points.horse().riding())
        {
            self.stop_riding(vid);
        }
        if self
            .points_of(vid.raw())
            .is_some_and(|points| points.horse().summoned())
        {
            self.horse_unsummon(vid, false);
        }
        self.mount_vnum(vid, 0);
    }

    /// The mount system's update event (`mountsystem_update_event`,
    /// `G/MountSystem.cpp:27-45`): every 12 Pulses each follower runs `Update`, in VID
    /// order. A follower whose NPC is gone drops its entry, as `DeleteMount` drops the
    /// actor.
    pub(super) fn run_mount_ticks(&mut self, pulse: u64) {
        if pulse % MOUNT_STATE_PULSES != 0 {
            return;
        }
        let mut stood: Vec<u32> = self.mount_npcs.keys().copied().collect();
        stood.sort_unstable();
        for stood in stood {
            if self.npc_by_vid(stood).is_none() {
                self.mount_npcs.remove(&stood);
            } else {
                self.idle_mount(stood);
            }
        }
    }

    /// The follower's `Update` (`G/MountSystem.cpp:243-260`) for the NPC under `stood`:
    /// a dead owner, a dead follower, a lost summon item or an item held by another
    /// unsummons it; else the follow runs.
    fn idle_mount(&mut self, stood: u32) {
        let Some(entry) = self.mount_npcs.get(&stood).copied() else {
            return;
        };
        let gone = self.is_dead(entry.rider.raw())
            || self.npc_by_vid(stood).is_some_and(|npc| npc.dead)
            || !self.mount_item_held(entry.rider, entry.summon_item);
        if gone {
            self.destroy_npc(stood);
            return;
        }
        self.follow_mount(stood, entry.rider);
    }

    /// Whether the summon item is still held by the rider (`FindByVID` and its owner,
    /// `G/MountSystem.cpp:250-251`).
    fn mount_item_held(&self, rider: Vid, id: u32) -> bool {
        self.characters
            .find_by_vid(rider)
            .ok()
            .and_then(|character| character.items().item(id))
            .is_some()
    }

    /// `_UpdateFollowAI` (`G/MountSystem.cpp:181-241`): past 4500 the follower re-stands
    /// 200 behind the rider's heading; past 300 it runs to 200 of the rider; nearer it
    /// waits. A re-stand the sectree refuses answers false and the follow runs instead.
    fn follow_mount(&mut self, stood: u32, rider: Vid) {
        let (Some(at), Some(spot)) = (self.npc_spot(stood), self.spot_of(rider.raw())) else {
            return;
        };
        let dist = distance_approx(at.0.saturating_sub(spot.x), at.1.saturating_sub(spot.y));
        if dist < START_FOLLOW_DISTANCE {
            self.send_move_wait(stood);
            return;
        }
        if dist >= RESPAWN_DISTANCE && self.show_mount(stood, rider, spot) {
            return;
        }
        self.set_now_walking(stood, false);
        self.follow_actor(stood, rider);
        self.set_last_attacked(stood);
    }

    /// The re-stand (`CHARACTER::Show`, `G/char.cpp:1847-1913`): 200 behind the rider's
    /// heading on the rider's map, written to the follower's last-attacked position
    /// (`:1878-1883`), whose time stays stale. A point no sectree holds stands nothing
    /// and answers false, so the follow runs instead. `Show`'s insert and view rebuild
    /// are the move's own.
    fn show_mount(&mut self, stood: u32, rider: Vid, spot: super::view::Spot) -> bool {
        let Some((channel, map)) = self.place_of(rider.raw()) else {
            return false;
        };

        let rotation = self.bodies.get(&rider).map_or(0.0, |body| body.rotation);
        let behind = rotation * core::f32::consts::PI / 180.0;
        let to = (
            spot.x.saturating_add(f32_to_i32(-APPROACH * behind.cos())),
            spot.y.saturating_add(f32_to_i32(-APPROACH * behind.sin())),
        );
        let Some(&(from_channel, from_map, at)) = self.npc_of.get(&stood) else {
            return false;
        };
        let Some(index) = self.maps.get_mut(&(channel, map)) else {
            return false;
        };
        let from_matches = self
            .npcs
            .get(&(from_channel, from_map))
            .is_some_and(|list| list.npcs.get(at).is_some_and(|npc| npc.vid == stood));
        if !from_matches {
            return false;
        }
        if from_map == map {
            let mut effects = Vec::new();
            if !index.show(
                EntityKey::Npc(stood),
                (to.0, to.1, 0),
                self.view_radius,
                &mut effects,
            ) {
                return false;
            }
            if let Some(list) = self.npcs.get_mut(&(channel, map)) {
                if let Some(npc) = Arc::make_mut(list).npcs.get_mut(at) {
                    debug_assert_eq!(npc.vid, stood);
                    npc.x = to.0;
                    npc.y = to.1;
                    npc.z = 0;
                }
            }
            self.last_attacked_at(stood, to);
            self.deliver(&effects, None);
            return true;
        }
        if index.tree_at(to.0, to.1).is_none() {
            return false;
        }
        let Some(list) = self.npcs.get_mut(&(from_channel, from_map)) else {
            return false;
        };
        let list = Arc::make_mut(list);
        let mut moved = list.npcs.swap_remove(at);
        if let Some(swap) = list.npcs.get(at) {
            self.npc_of.insert(swap.vid, (from_channel, from_map, at));
        }
        moved.x = to.0;
        moved.y = to.1;
        moved.z = 0;
        let mut effects = Vec::new();
        if let Some(index) = self.maps.get_mut(&(from_channel, from_map)) {
            index.remove(EntityKey::Npc(stood), &mut effects);
        }
        let list = self
            .npcs
            .entry((channel, map))
            .or_insert_with(|| Arc::new(world::npc::MapNpcs::default()));
        let list = Arc::make_mut(list);
        let at = list.npcs.len();
        list.npcs.push(moved);
        self.npc_of.insert(stood, (channel, map, at));
        if let Some(index) = self.maps.get_mut(&(channel, map)) {
            let _shown = index.show(
                EntityKey::Npc(stood),
                (to.0, to.1, 0),
                self.view_radius,
                &mut effects,
            );
        }
        self.last_attacked_at(stood, to);
        self.deliver(&effects, None);
        true
    }

    /// The last-attacked position write of `Show` (`G/char.cpp:1878-1883`), which moves
    /// the position only and leaves the time.
    fn last_attacked_at(&mut self, stood: u32, to: (i32, i32)) {
        self.npc_combat.entry(stood).or_default().last_attacked.at = to;
    }

    /// `CMountActor::Follow` (`G/MountSystem.cpp:262-292`): within 200 nothing moves;
    /// else the follower faces the rider and goes the distance past 200, then waits.
    fn follow_actor(&mut self, stood: u32, rider: Vid) {
        let (Some(at), Some(spot)) = (self.npc_spot(stood), self.spot_of(rider.raw())) else {
            return;
        };
        if let Some(mind) = self.minds.get_mut(stood) {
            mind.rotation =
                rotation_to_xy(spot.x.saturating_sub(at.0), spot.y.saturating_sub(at.1));
        }
        let dist = distance_sqrt(spot.x.saturating_sub(at.0), spot.y.saturating_sub(at.1));
        if dist <= APPROACH {
            return;
        }
        let rotation = self.minds.get(stood).map_or(0.0, |mind| mind.rotation);
        let (fx, fy) = delta_by_degree(rotation, dist - APPROACH);
        // `(int)(x + f + 0.5)`: C truncation after the half-unit rise rounds half up.
        let to = (
            f32_to_i32(i32_to_f32(at.0) + fx + 0.5),
            f32_to_i32(i32_to_f32(at.1) + fy + 0.5),
        );
        if !self.npc_goto(stood, to.0, to.1) {
            return;
        }
        self.send_move_wait(stood);
    }

    /// Whether the NPC under `vid` is a summoned costume mount (`IsMount`), which
    /// `do_purge` spares.
    pub(super) fn is_mount_npc(&self, vid: u32) -> bool {
        self.mount_npcs.contains_key(&vid)
    }
}

#[cfg(test)]
mod tests {
    use common::enums::EWearPositions;
    use common::item_slots::EWindows;
    use protocol::item_pos::ItemPos;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::item::{Item, ItemIdRange};

    use super::super::fixtures::{a_world, open_cells, region, TestClock, PLACE};
    use super::super::GameState;
    use super::*;
    use crate::item_move::Mover;

    /// The owner's first mount costume: timed, `VALUE1` 20201, no applies.
    const MOUNT: u32 = 52_001;
    /// Its mob.
    const MOB: u32 = 20_201;
    /// The rider.
    const RIDER: u32 = 1;

    const fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 0,
            pk_mode: 2,
            affect_flags: [0; 2],
            dead: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    /// A world hosting [`PLACE`] with the owner's mob protos, so the mount mob
    /// spawns, and the rider entered at `at` holding `items`.
    fn a_mount_world(
        clock: &TestClock,
        at: (i32, i32),
        items: &[(ItemPos, Item)],
    ) -> (GameState, UnboundedReceiver<Vec<u8>>) {
        let mut world = a_world(clock);
        let proto_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let respawner = world::npc::Respawner::new(
            std::sync::Arc::new(
                gamedata::mob_proto::MobProtos::load(&proto_dir).expect("the mob protos load"),
            ),
            std::sync::Arc::new(gamedata::mob_group::MobGroups::default()),
            std::sync::Arc::new(
                gamedata::mob_locale_names::MobLocaleNames::parse(b"VNUM\tNAME\n").unwrap(),
            ),
            world::npc::NpcVids::starting_at(world::npc::FIRST_NPC_VID),
        );
        world.install_respawner(respawner, open_cells());
        world
            .install_item_ids(ItemIdRange::new(1, 1_000_000, 1).expect("a range"))
            .expect("the ids install");
        world.stand_npcs(
            PLACE.0,
            &region(),
            world::npc::MapNpcs {
                npcs: Vec::new(),
                positions: Vec::new(),
                regens: Vec::new(),
            },
        );
        let inbox = enter_on_with(&mut world, at, items);
        (world, inbox)
    }

    /// [`fixtures::enter`], with `items` the entrant holds.
    fn enter_on_with(
        world: &mut GameState,
        at: (i32, i32),
        items: &[(ItemPos, Item)],
    ) -> UnboundedReceiver<Vec<u8>> {
        let (tx, mut inbox) = tokio::sync::mpsc::unbounded_channel();
        let name = format!("P{RIDER}");
        world
            .enter_world_with_items(
                "",
                Vid::new(RIDER),
                RIDER,
                &name,
                items,
                crate::client_registry::ClientOutbox::new(tx),
            )
            .expect("the player enters");
        while inbox.try_recv().is_ok() {}
        world
            .characters
            .find_by_vid_mut(Vid::new(RIDER))
            .expect("the player is in the world")
            .set_points(Some(super::super::fixtures::points(12_345)));
        let card = crate::loading_phase::PcCard {
            name,
            job: 0,
            empire: 1,
            level: 10,
            conqueror_level: 0,
            language: 0,
            pk_mode: crate::loading_phase::PK_MODE_PROTECT,
            skill_group: 0,
            alignment: 0,
            voice: 0,
            premium: 0,
            premium_time: 0,
        };
        let _shown = world.place_body(
            Vid::new(RIDER),
            crate::game_loop_messages::EnterPlace {
                channel: PLACE.0,
                map: PLACE.1,
                x: at.0,
                y: at.1,
                z: 0,
            },
            card,
        );
        inbox
    }

    fn drained(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// A started mount costume: socket 0 in the future, socket 1 counting the use.
    fn started_mount(id: u32) -> Item {
        let mut item = Item::new(id, MOUNT);
        item.sockets[0] = i32::from_ne_bytes(
            super::super::quests::quest_now()
                .wrapping_add(10_000)
                .to_ne_bytes(),
        );
        item.sockets[1] = 1;
        item
    }

    fn mount_cell() -> ItemPos {
        wear_pos(EWearPositions::CostumeMount as u16)
    }

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    /// Wearing the costume summons 20201 within 100, named `<rider> Mount` of the
    /// rider's empire; taking it off unsummons; `do_ride` mounts the standing
    /// follower and unmounts the ride, and `/unmount` takes the ride off too.
    #[test]
    fn a_worn_mount_summons_a_ride_mounts_and_an_unmount_takes_it_off() {
        let clock = TestClock::default();
        clock.set(1_000);
        let (mut world, mut inbox) =
            a_mount_world(&clock, (3200, 3200), &[(inv(0), started_mount(7))]);
        let _ = drained(&mut inbox);
        assert!(world.mount_npcs.is_empty());
        let request = world::character::MoveRequest {
            from: inv(0),
            to: mount_cell(),
            count: 0,
        };
        world
            .move_item(Vid::new(RIDER), request, mover())
            .expect("the costume wears");
        assert_eq!(world.mount_npcs.len(), 1);
        let stood = *world.mount_npcs.keys().next().expect("the follower");
        let follower = world
            .npc_by_vid(stood)
            .expect("the follower stands")
            .clone();
        assert_eq!(follower.vnum, MOB);
        assert_eq!(follower.name, b"P1 Mount");
        assert_eq!(follower.empire, 1);
        assert!((follower.x - 3200).abs() <= 100);
        assert!((follower.y - 3200).abs() <= 100);
        let _ = drained(&mut inbox);
        assert!(world.ride_mount_costume(Vid::new(RIDER)));
        assert!(world.mount_npcs.is_empty(), "the ride takes the follower");
        assert_eq!(
            world
                .points_of(RIDER)
                .map(world::character::Points::mount_vnum),
            Some(MOB)
        );
        assert!(world.unmount_line(Vid::new(RIDER)));
        assert_eq!(world.mount_npcs.len(), 1, "the follower stands again");
        assert_eq!(
            world
                .points_of(RIDER)
                .map(world::character::Points::mount_vnum),
            Some(0)
        );
        assert!(world.ride_mount_costume(Vid::new(RIDER)));
        assert_eq!(
            world
                .points_of(RIDER)
                .map(world::character::Points::mount_vnum),
            Some(MOB),
            "the ride mounts again"
        );
        assert!(world.ride_mount_costume(Vid::new(RIDER)));
        assert_eq!(
            world
                .points_of(RIDER)
                .map(world::character::Points::mount_vnum),
            Some(0),
            "the ride unmounts"
        );
        let request = world::character::MoveRequest {
            from: mount_cell(),
            to: inv(0),
            count: 0,
        };
        world
            .move_item(Vid::new(RIDER), request, mover())
            .expect("the costume comes off");
        assert!(world.mount_npcs.is_empty());
        assert!(world.npc_by_vid(stood).is_none());
    }

    /// Past 4500 the follower re-stands 200 behind the rider; past 300 it runs
    /// toward the rider; nearer it waits. A dead owner unsummons it.
    #[test]
    fn a_far_follower_stands_by_its_rider_and_a_dead_one_goes() {
        let clock = TestClock::default();
        clock.set(1_000);
        let (mut world, mut inbox) =
            a_mount_world(&clock, (3200, 3200), &[(mount_cell(), started_mount(7))]);
        let _ = drained(&mut inbox);
        world.mount_summon(Vid::new(RIDER), &started_mount(7));
        let stood = *world.mount_npcs.keys().next().expect("the follower");
        // Past 4500: the re-stand lands 200 behind the rider's heading (at 0,
        // `-200 * cos` west).
        world.npc_move(stood, 3200 + 5000, 3200);
        world.run_mount_ticks(MOUNT_STATE_PULSES);
        let follower = world
            .npc_by_vid(stood)
            .expect("the follower stands")
            .clone();
        assert_eq!((follower.x, follower.y), (3000, 3200));
        // Past 300: the follower runs toward the rider and waits at 200.
        world.npc_move(stood, 3200 + 1000, 3200);
        world.wake_minds();
        world.run_mount_ticks(2 * MOUNT_STATE_PULSES);
        let follower = world
            .npc_by_vid(stood)
            .expect("the follower stands")
            .clone();
        assert_eq!(
            (follower.x, follower.y),
            (3200 + 1000, 3200),
            "the Goto walks"
        );
        let mind = world.minds.get(stood).expect("the follower walks");
        assert_eq!(mind.motion.dest, (3400, 3200));
        // A dead owner unsummons it.
        world.pc_combat.entry(RIDER).or_default().dead = true;
        world.run_mount_ticks(3 * MOUNT_STATE_PULSES);
        assert!(world.mount_npcs.is_empty());
        assert!(world.npc_by_vid(stood).is_none());
    }

    /// `CheckMount` at login stands nothing on map 113 or an arena map, and stands
    /// the follower anywhere else.
    #[test]
    fn the_login_stands_no_mount_on_the_ox_or_arena_maps() {
        let clock = TestClock::default();
        clock.set(1_000);
        let (mut world, mut inbox) =
            a_mount_world(&clock, (3200, 3200), &[(mount_cell(), started_mount(7))]);
        let _ = drained(&mut inbox);
        world.enter_mount(Vid::new(RIDER));
        assert_eq!(world.mount_npcs.len(), 1);
        world.leave_mounts(Vid::new(RIDER));
        assert!(world.mount_npcs.is_empty());
    }
}
