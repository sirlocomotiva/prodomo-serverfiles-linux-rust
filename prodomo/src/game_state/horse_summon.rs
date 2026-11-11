//! The summoned horse: `CHARACTER::HorseSummon` (`G/char_horse.cpp:147-276`), the horse's
//! `StateHorse` (`G/char_state.cpp:1226-1283`), `horse_dead_event` (`G/char_horse.cpp:108-124`),
//! `ReviveHorse` (`:290-299`), the gifts a horse takes (`G/char_item.cpp:9483-9509`), the GM
//! lines `/horse_summon` and `/horse_unsummon` (`G/cmd_gm.cpp:3800-3813`), and the player's
//! `/user_horse_back` (`G/cmd_general.cpp:87-102`).
//!
//! A summon needs a horse of level 1 or more, none summoned and no ride. A living horse
//! called from far stands 2000 to 2500 away on each axis, any other 100 at most; it is the
//! level's race, one more in a guild and one more again for its master, named after its
//! rider with `'s ` (the horse names wait for their slice), and faces the rider's heading
//! turned half round. A dead horse is summoned lying dead: its view hears `GC_DEAD`, and it
//! goes again a minute later. The rider hears the `horse_state` line.
//!
//! An unsummon tells a rider who is not riding to hide the state. A dead horse, or one sent
//! off near, is destroyed at once; one sent off from far runs 3500 straight away from its
//! rider, and its next Idle, which finds no rider, destroys it. A horse destroyed by any path
//! clears its rider's horse. The rider's leaving unsummons it, and so does its death
//! (`CHARACTER::HorseDie`).
//!
//! Every half second a horse with its rider 400 or more away follows to 150 to 300 of it,
//! running when more than 700 away; nearer, it wanders 200 to 400 at a walk every 5 to 12
//! seconds. A horse with no rider is destroyed, as a horse race spawned by `/mob` is.
//!
//! The quest gates read `IsRidingMount` (a mount costume worn or `AFFECT_MOUNT`).

use common::cfloat::{f32_to_i32, i32_to_f32};
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_inventory::HEADER_GC_DEAD;
use protocol::gc_vid::GcSpecialEffect;
use world::character::{number, HORSE_STATS};

use super::fight::header_and_vid;
use super::mob_ai::delta_by_degree;
use super::motion::rotation_to_xy;
use super::view::EntityKey;
use super::GameState;
use super::GmRows;
use crate::chat_line::Arg;
use crate::item_move::{MovedItems, Mover};
use crate::save::PASSES_PER_SEC;
use crate::sync_position::distance_approx;

/// `PASSES_PER_SEC(60)`: a dead horse lies a minute before it goes.
const HORSE_DEAD_SECS: u64 = 60;
/// `(DWORD) PASSES_PER_SEC(0.5)`: the Pulses between two `StateHorse` runs.
const HORSE_STATE_PULSES: u64 = 12;
/// `START_FOLLOW_DISTANCE`: from here on the horse follows its rider.
const START_FOLLOW_DISTANCE: i32 = 400;
/// `START_RUN_DISTANCE`: past here it runs to follow.
const START_RUN_DISTANCE: i32 = 700;
/// The distance a horse sent off from far runs away.
const RUN_OFF_DISTANCE: f32 = 3500.0;
/// The OX map, where the quest's summon does nothing.
const OX_MAP: i32 = 113;
/// The summoned horse could not stand.
const SUMMON_FAILED: &[u8] = b"[LS;887]";
/// `/user_horse_back` with no horse.
const NO_HORSE: &[u8] = b"[LS;501]";
/// `/user_horse_back` sent the horse back.
const HORSE_SENT_BACK: &[u8] = b"[LS;502]";
/// `/user_horse_back` while riding.
const HORSE_RIDDEN: &[u8] = b"[LS;504]";
/// `/user_horse_feed` while riding, with no horse summoned.
const FEED_RIDDEN: &[u8] = b"[LS;505]";
/// `/user_horse_feed` fed the horse: the food's name and an empty second argument.
const FEED_DONE: &[u8] = b"[LS;506;%s;%s]";
/// `/user_horse_feed` without the food: its name.
const FEED_MISSING: &[u8] = b"[LS;507;%s]";
/// The grade-1 food (`G/cmd_general.cpp:119`): grade `g` eats `50054 + g - 1`.
const FIRST_FOOD: u32 = 50_054;
/// A revive herb given to a horse.
const REVIVED_LINE: &[u8] = b"[LS;452]";
/// A feed given to a horse.
const FED_LINE: &[u8] = b"[LS;453]";
/// `AFFECT_MOUNT` (`G/affect.h:47`).
const AFFECT_MOUNT: u32 = crate::loading_phase::AFFECT_MOUNT;

/// A summoned horse standing in the world.
#[derive(Debug, Clone, Copy)]
pub(super) struct HorseNpc {
    /// `m_chRider`: `None` once the horse was sent off.
    rider: Option<Vid>,
    /// The Pulse `horse_dead_event` fires on, for a horse summoned dead.
    dead_due: Option<u64>,
    /// `m_dwLastAttackTime`, which `StateHorse` reads as the time of the next wander.
    wander_after_ms: u32,
}

/// What a quest step asked of its character's horse, which the world runs when the step
/// ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HorseOp {
    /// `horse.summon(from_far, vnum)`.
    Summon { from_far: bool, vnum: u32 },
    /// `horse.unsummon`.
    Unsummon,
    /// `horse.revive` and `pc.revive_horse`: `ReviveHorse`.
    Revive,
    /// `horse.ride`: `StartRiding`.
    Ride,
    /// `horse.unride`: `StopRiding`.
    Unride,
    /// `horse.set_name`: the manager rewrite with the re-summon around it.
    SetName(Vec<u8>),
    /// `pc.mount(vnum, seconds)`.
    Mount { vnum: u32, seconds: i32 },
    /// `pc.unmount`.
    Unmount,
    /// `pc.mount_bonus(apply, value, seconds)`.
    MountBonus { apply: u8, value: i32, seconds: i32 },
}

impl GameState {
    /// `HorseSummon(true, from_far, vnum)` for the character under `vid`.
    pub(super) fn horse_summon(&mut self, vid: Vid, from_far: bool, vnum: u32) {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let horse = points.horse();
        if horse.summoned() || horse.level() == 0 || horse.riding() {
            return;
        }
        let (level, dead) = (horse.level(), horse.health() <= 0);
        let (Some(place), Some(spot)) = (self.place_of(vid.raw()), self.spot_of(vid.raw())) else {
            return;
        };
        tracing::info!(?vid, level, from_far, "HorseSummon");
        let from_far = from_far && !dead;
        let (dx, dy) = if from_far {
            (self.far_offset(), self.far_offset())
        } else {
            let dx = number(&mut self.dice, -100, 100);
            (dx, number(&mut self.dice, -100, 100))
        };
        let race = if vnum == 0 {
            self.my_horse_vnum(vid, level)
        } else {
            vnum
        };
        let rotation = self.bodies.get(&vid).map_or(0.0, |body| body.rotation);
        let at = (spot.x.saturating_add(dx), spot.y.saturating_add(dy));
        let Some(mut stood) = self.spawn_horse(place, race, at, f32_to_i32(rotation + 180.0))
        else {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_INFO, SUMMON_FAILED, &[]);
            }
            return;
        };
        let horse_vid = stood.vid;
        // The named horse carries the manager's name (`G/char_horse.cpp:206-216`); any
        // other carries its rider's name with `'s `.
        let named = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|rider| self.horse_names.get(&rider.player_id()))
            .filter(|name| !name.is_empty())
            .cloned();
        stood.name = named.unwrap_or_else(|| {
            let mut name = self
                .characters
                .find_by_vid(vid)
                .map_or_else(|_| Vec::new(), |rider| rider.name().as_bytes().to_vec());
            name.extend_from_slice(b"'s ");
            name
        });
        stood.dead = dead;
        let pulse = self.last_pulse;
        self.stand_runtime(place, vec![stood], pulse);
        if dead {
            let record = header_and_vid(HEADER_GC_DEAD.value(), horse_vid);
            self.packet_around(EntityKey::Npc(horse_vid), &record, None);
        }
        let dead_due = dead.then(|| pulse + HORSE_DEAD_SECS * u64::from(PASSES_PER_SEC));
        self.horse_npcs.insert(
            horse_vid,
            HorseNpc {
                rider: Some(vid),
                dead_due,
                wander_after_ms: 0,
            },
        );
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            points.horse_mut().summon(horse_vid);
        }
        self.flush_horse(vid);
    }

    /// `(number(0, 1) * 2 - 1) * number(2000, 2500)`, drawn left to right.
    fn far_offset(&mut self) -> i32 {
        let sign = number(&mut self.dice, 0, 1) * 2 - 1;
        sign * number(&mut self.dice, 2000, 2500)
    }

    /// `GetMyHorseVnum` (`G/char_horse.cpp:278-291`): the level's race, one more in a guild
    /// and one more again for the guild's master.
    pub(super) fn my_horse_vnum(&self, vid: Vid, level: u8) -> u32 {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let (guild, master) = Self::guild_pair(&self.guilds, pid);
        let row = &HORSE_STATS[usize::from(level.min(world::character::HORSE_MAX_LEVEL))];
        row.npc_race + u32::from(guild != 0) + u32::from(master)
    }

    /// `HorseSummon(false, from_far)` for the character under `vid`.
    pub(super) fn horse_unsummon(&mut self, vid: Vid, from_far: bool) {
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let horse = points.horse_mut();
        let Some(horse_vid) = horse.summoned_vid() else {
            return;
        };
        let dead = horse.health() <= 0;
        horse.unsummon();
        if let Some(standing) = self.horse_npcs.get_mut(&horse_vid) {
            standing.rider = None;
        }
        self.flush_horse(vid);
        if dead || !from_far {
            self.destroy_npc(horse_vid);
            return;
        }
        self.set_now_walking(horse_vid, false);
        let (Some(at), Some(rider)) = (self.npc_spot(horse_vid), self.spot_of(vid.raw())) else {
            return;
        };
        let rotation =
            rotation_to_xy(rider.x.saturating_sub(at.0), rider.y.saturating_sub(at.1)) + 180.0;
        if let Some(mind) = self.minds.get_mut(horse_vid) {
            mind.rotation = rotation;
        }
        let (fx, fy) = delta_by_degree(rotation, RUN_OFF_DISTANCE);
        let target = (
            f32_to_i32(i32_to_f32(at.0) + fx),
            f32_to_i32(i32_to_f32(at.1) + fy),
        );
        let _moved = self.npc_goto(horse_vid, target.0, target.1);
        self.send_move_wait(horse_vid);
    }

    /// `Destroy` of a horse (`G/char.cpp:632-635`): its rider's horse is cleared.
    pub(super) fn forget_horse_npc(&mut self, horse_vid: u32) {
        let Some(standing) = self.horse_npcs.remove(&horse_vid) else {
            return;
        };
        if let Some(rider) = standing.rider {
            if let Some(points) = self.pc_points_mut(rider.raw()) {
                points.horse_mut().unsummon();
            }
            self.flush_horse(rider);
        }
    }

    /// Whether the NPC under `vid` is some character's summoned horse (`GetRider() != NULL`).
    pub(super) fn has_rider(&self, vid: u32) -> bool {
        self.horse_npcs
            .get(&vid)
            .is_some_and(|standing| standing.rider.is_some())
    }

    /// `horse_dead_event`: each dead horse whose minute ran out goes.
    pub(super) fn run_horse_dead_events(&mut self, pulse: u64) {
        let due: Vec<(u32, Option<Vid>)> = self
            .horse_npcs
            .iter()
            .filter(|(_, standing)| standing.dead_due.is_some_and(|due| pulse >= due))
            .map(|(&horse_vid, standing)| (horse_vid, standing.rider))
            .collect();
        for (horse_vid, rider) in due {
            if let Some(standing) = self.horse_npcs.get_mut(&horse_vid) {
                standing.dead_due = None;
            }
            if let Some(rider) = rider {
                self.horse_unsummon(rider, false);
            }
        }
    }

    /// `StateHorse` for the horse under `vid`.
    pub(super) fn idle_horse(&mut self, vid: u32) {
        self.set_duration(vid, HORSE_STATE_PULSES);
        let Some(rider) = self
            .horse_npcs
            .get(&vid)
            .and_then(|standing| standing.rider)
        else {
            self.destroy_npc(vid);
            return;
        };
        let (Some(at), Some(spot)) = (self.npc_spot(vid), self.spot_of(rider.raw())) else {
            return;
        };
        if let Some(combat) = self.npc_combat.get_mut(&vid) {
            combat.last_attacked.at = at;
        }
        let dist = distance_approx(at.0.saturating_sub(spot.x), at.1.saturating_sub(spot.y));
        if dist >= START_FOLLOW_DISTANCE {
            if dist > START_RUN_DISTANCE {
                self.set_now_walking(vid, false);
            }
            let min = i32_to_f32(number(&mut self.dice, 150, 300));
            let _followed = self.follow(vid, rider, min);
            self.set_duration(vid, HORSE_STATE_PULSES);
            return;
        }
        let now = self.clock.now();
        // The test switch that keeps monsters still keeps a near horse still too.
        if self.minds.wanders_off()
            || self
                .horse_npcs
                .get(&vid)
                .is_none_or(|standing| now <= standing.wander_after_ms)
        {
            return;
        }
        let wait = u32::try_from(number(&mut self.dice, 5000, 12000)).unwrap_or(0);
        if let Some(standing) = self.horse_npcs.get_mut(&vid) {
            standing.wander_after_ms = now.wrapping_add(wait);
        }
        self.turn_at_random(vid);
        let distance = i32_to_f32(number(&mut self.dice, 200, 400));
        let Some(&(_, map, _)) = self.npc_of.get(&vid) else {
            return;
        };
        let Some(target) = self.wander_target(vid, map, distance) else {
            return;
        };
        self.set_now_walking(vid, true);
        if self.npc_goto(vid, target.0, target.1) {
            self.send_move_wait(vid);
        }
    }

    /// `CHARACTER::ReviveHorse`: a dead horse refills, and a summoned one stands again.
    pub(super) fn revive_horse(&mut self, vid: Vid) {
        let now = Self::global_time();
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        if !points.horse_mut().revive(now) {
            return;
        }
        self.horse_unsummon(vid, false);
        self.horse_summon(vid, false, 0);
        self.settle_horse(vid);
    }

    /// `do_user_horse_back`: the summoned horse goes near, or the caller hears why not.
    pub(super) fn user_horse_back(&mut self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let Some(points) = self.pc_points_mut(vid.raw()) else {
            return;
        };
        let (summoned, riding) = (points.horse().summoned(), points.horse().riding());
        if summoned {
            self.horse_unsummon(vid, false);
            self.tell(vid, mover, CHAT_TYPE_INFO, HORSE_SENT_BACK, &[]);
        } else if riding {
            self.tell(vid, mover, CHAT_TYPE_INFO, HORSE_RIDDEN, &[]);
        } else {
            self.tell(vid, mover, CHAT_TYPE_INFO, NO_HORSE, &[]);
        }
    }

    /// `do_user_horse_feed` (`G/cmd_general.cpp:105-133`): with a horse summoned, one of the
    /// grade's food goes and the horse eats it (`FeedHorse`, which a dead horse ignores
    /// while the food is still spent); without the food the caller hears its name. No
    /// summoned horse hears `[LS;501]`, or `[LS;505]` while riding. The private-shop gate
    /// (`GetMyShop`) is not ported, so it never refuses. Answers the rows for the store.
    pub(super) fn user_horse_feed(&mut self, vid: Vid, mover: Mover) -> Option<GmRows> {
        let horse = self.pc_points_mut(vid.raw())?.horse();
        let (summoned, riding, grade) = (horse.summoned(), horse.riding(), horse.grade());
        if !summoned {
            let line = if riding { FEED_RIDDEN } else { NO_HORSE };
            self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            return None;
        }
        let food = FIRST_FOOD + u32::from(grade) - 1;
        let name = self
            .protos
            .get(food)
            .map(|proto| proto.locale_name.clone())
            .unwrap_or_default();
        if self.count_specify_item(vid, food) <= 0 {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                FEED_MISSING,
                &[Arg::Text(&name)],
            );
            return None;
        }
        let protos = std::sync::Arc::clone(&self.protos);
        let character = self.characters.find_by_vid_mut(vid).ok()?;
        let pid = character.player_id();
        let usable = common::item_slots::usable_inventory_cells(character.inven_point());
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let done = world::character::remove_specify_item(items, slots, &protos, usable, (food, 1));
        let packed = MovedItems::new(pid, vid.raw(), done, mover, &self.protos, &self.locale);
        for record in packed.records {
            let _sent = self.write_to_client(vid, record);
        }
        let now = Self::global_time();
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            let _fed = points.horse_mut().feed(now);
        }
        self.settle_horse(vid);
        let args = [Arg::Text(&name), Arg::Text(b"")];
        self.tell(vid, mover, CHAT_TYPE_INFO, FEED_DONE, &args);
        Some(GmRows {
            pid,
            changes: packed.changes,
            cleared_skills: Vec::new(),
            gold: 0,
            monarch: None,
            points: None,
            quickslots: None,
            buildings: Vec::new(),
            account: None,
        })
    }

    /// `IsRidingMount` (`G/char.cpp:11228-11231`): a mount costume worn or `AFFECT_MOUNT`.
    fn riding_mount(&self, vid: Vid) -> bool {
        let worn = self.characters.find_by_vid(vid).is_ok_and(|character| {
            character
                .items()
                .item_at(world::character::wear_pos(
                    common::enums::EWearPositions::CostumeMount as u16,
                ))
                .is_some()
        });
        worn || self.affect_events.get(&vid).is_some_and(|event| {
            event
                .affects
                .iter()
                .any(|affect| affect.kind == AFFECT_MOUNT)
        })
    }

    /// The horse calls a quest step queued, in order.
    pub(super) fn apply_horse_ops(&mut self, vid: Vid, ops: Vec<HorseOp>) {
        for op in ops {
            match op {
                HorseOp::Summon { from_far, vnum } => {
                    let map = self.place_of(vid.raw()).map_or(0, |(_, map)| map);
                    if self.riding_mount(vid) || map == OX_MAP || self.is_arena_map(map) {
                        continue;
                    }
                    self.horse_summon(vid, from_far, vnum);
                }
                HorseOp::Unsummon => self.horse_unsummon(vid, false),
                HorseOp::Revive => self.revive_horse(vid),
                HorseOp::Ride => {
                    let map = self.place_of(vid.raw()).map_or(0, |(_, map)| map);
                    if !self.riding_mount(vid) && map != OX_MAP && !self.is_arena_map(map) {
                        self.start_riding(vid);
                    }
                }
                HorseOp::Unride => self.stop_riding(vid),
                HorseOp::SetName(name) => self.name_horse(vid, &name),
                HorseOp::Mount { vnum, seconds } => self.quest_mount(vid, vnum, seconds),
                HorseOp::Unmount => self.unmount(vid),
                HorseOp::MountBonus {
                    apply,
                    value,
                    seconds,
                } => self.mount_bonus(vid, apply, value, seconds),
            }
        }
    }

    /// The horse arm of `ReceiveItem` (`G/char_item.cpp:9483-9509`): a revive herb revives the
    /// giver's horse, spends one and says so; a feed feeds it, says so, spends one and shows
    /// the red effect on the horse given to.
    pub(super) fn horse_gift(
        &mut self,
        vid: Vid,
        (horse_vid, item_id): (u32, u32),
        revive: bool,
        (actor, moved): (Mover, &mut MovedItems),
    ) {
        if revive {
            self.revive_horse(vid);
        } else {
            let now = Self::global_time();
            if let Some(points) = self.pc_points_mut(vid.raw()) {
                let _fed = points.horse_mut().feed(now);
            }
            self.settle_horse(vid);
            moved.records.push(self.give_line(actor, FED_LINE));
        }
        self.spend_gift(vid, item_id, actor, moved);
        if revive {
            moved.records.push(self.give_line(actor, REVIVED_LINE));
        } else {
            let mut record = Vec::new();
            GcSpecialEffect {
                effect_type: world::character::SE_HPUP_RED,
                vid: horse_vid,
            }
            .encode_into(&mut record);
            self.packet_around(EntityKey::Npc(horse_vid), &record, None);
        }
    }

    /// `item->SetCount(item->GetCount() - 1)` of the gift `item_id`, into `moved`.
    pub(super) fn spend_gift(
        &mut self,
        vid: Vid,
        item_id: u32,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let pid = character.player_id();
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let Some(item) = items.item(item_id).cloned() else {
            return;
        };
        let Some(proto) = self.protos.get(item.vnum) else {
            return;
        };
        let Ok(mut used) = world::character::spend_one(items, &item, proto) else {
            return;
        };
        world::character::sync_quickslots(&mut used, slots, items);
        let quickslots = character.quickslots().clone();
        let points = character.points().cloned();
        let spent = MovedItems::new(pid, vid.raw(), used, actor, &self.protos, &self.locale);
        moved.records.extend(spent.records);
        moved.changes.extend(spent.changes);
        moved.points = points;
        moved.quickslots = Some(quickslots);
    }

    /// `CHorseNameManager::Validate` (`G/horsename_manager.cpp:58-75`): with the
    /// `AFFECT_HORSE_NAME` row and a flag at or past now, the row lives one tick more;
    /// with the flag gone stale, the horse is unsummoned, the row removed, the name
    /// emptied, and the horse summoned back, so the nameless summon stands where the
    /// named one stood. No compute runs: legacy's resummon does not recompute either.
    pub(super) fn validate_horse_name(&mut self, vid: Vid) {
        let Some(event) = self.affect_events.get_mut(&vid) else {
            return;
        };
        let Some(slot) = event
            .affects
            .iter_mut()
            .find(|affect| affect.kind == crate::loading_phase::AFFECT_HORSE_NAME)
        else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let till = self
            .quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, b"horse_name.valid_till"))
            .unwrap_or(0);
        if till >= i32::from_ne_bytes(Self::global_time().to_ne_bytes()) {
            slot.remaining = slot.remaining.saturating_add(1);
            return;
        }
        let Some(event) = self.affect_events.get_mut(&vid) else {
            return;
        };
        let removed = super::affect::take_affects_of_kind(
            &mut event.affects,
            crate::loading_phase::AFFECT_HORSE_NAME,
        );
        for affect in &removed {
            let _stripped = self.compute_affect(vid, affect, false);
            let frame = super::affect::affect_remove_frame(affect);
            let _gone = self.write_to_client(vid, frame);
        }
        let _emptied = self.horse_names.insert(pid, Vec::new());
        self.horse_unsummon(vid, true);
        self.horse_summon(vid, true, 0);
    }
    /// The `horse.set_name` write (`G/questlua_horse.cpp:250-266`): the flag, the month
    /// affect, the manager's rewrite, and the summon back after the unsummon. The sync
    /// and compute run while the rewrite holds, before the unsummon: the summon back
    /// never computes, so only this compute prices the name's 20. The caller judged the
    /// name; `name` holds no zero byte and fits the field.
    pub(super) fn name_horse(&mut self, vid: Vid, name: &[u8]) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let month = if self.test_server {
            60 * 5
        } else {
            60 * 60 * 24 * 30
        };
        let till = Self::global_time().saturating_add(month);
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_flag(
                pid,
                b"horse_name.valid_till",
                i32::from_ne_bytes(till.to_ne_bytes()),
            );
        }
        self.add_pc_affect(
            vid,
            super::affect::Affect {
                kind: crate::loading_phase::AFFECT_HORSE_NAME,
                apply_on: 0,
                value: 0,
                flag: 0,
                remaining: i32::try_from(month * PASSES_PER_SEC).unwrap_or(i32::MAX),
                sp_cost: 0,
            },
            true,
        );
        let _named = self.horse_names.insert(pid, name.to_vec());
        self.sync_horse_named(vid);
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            let computed = points.compute_points();
            self.send_point_records(vid, &computed);
        }
        self.horse_unsummon(vid, true);
        self.horse_summon(vid, true, 0);
    }
}

#[cfg(test)]
impl HorseNpc {
    /// A living horse `rider` summoned, as a test stands it beside its rider.
    pub(super) const fn ridden_by(rider: Vid) -> Self {
        Self {
            rider: Some(rider),
            dead_due: None,
            wander_after_ms: 0,
        }
    }
}
