//! The costume pet (`ENABLE_PET_COSTUME_SYSTEM`, `G/PetSystem.cpp`,
//! `G/char.cpp:11281-11324`): the follower a worn pet costume summons and its follow.
//!
//! Wearing the costume summons (`PetSummon`, `G/item.cpp:1486`): the `VALUE1` mob stands
//! within 100 of the owner, keeping its own name, of the owner's empire, facing the
//! owner's heading turned half round — with the same observer flag as the owner. Unlike
//! the mount, a summon needs no gates: no morph line, no OX/arena check, and the horse
//! rides on. Taking the costume off unsummons (`PetUnsummon`, `G/item.cpp:1531`): the
//! owner's follower of that mob goes.
//!
//! `CheckPet` summons when none is summoned and the costume is worn, with no gates of
//! its own: the login gates it on map 113 and the arenas
//! (`G/input_login.cpp:927-931`), and the plain restart-here arm calls it bare
//! (`G/cmd_general.cpp:709-711`, before the revive, unlike the mount's absence there).
//! The war-map and threeway arms are unreached here and call nothing; restart-now and
//! restart-town never did.
//!
//! A pet never rides: no mount arm, no `AFFECT_MOUNT`, no ride item. Its proto applies
//! land through the worn loop (`ComputePoints`, `G/char.cpp:3070-3080`), which the mount
//! skips; the unsummon's `ClearBuff` and the summon's recompute are that loop's own.
//!
//! Every 12 Pulses each pet runs `Update`: a pet whose summon item is gone — its item
//! VID lost, dropped, or held by another — goes, as `M2_DESTROY_CHARACTER` drops the
//! NPC. (No death arm: `CPetActor::Update` reads no `IsDead`, unlike the mount's.)
//! Past 4500 of the owner the pet re-stands 200 behind the owner's heading; past 300
//! it walks or runs to 200 of the owner (running past 900, unlike the mount's run);
//! nearer it waits. A destroyed pet drops its entry, as `DeletePet` drops the actor.
//! Legacy's write-only fields (`m_originalMoveSpeed`, `m_dwLastActionTime`) and the
//! unused walk speed (`START_RUN_DISTANCE` only picks the motion) stay out, and so do
//! the declared-never-defined members (`SetName`, `GiveBuff`, `AddPet`), the
//! unreferenced `PET_COUNT_LIMIT` and `SetUpdatePeriod`, and the unread `_UpdatAloneActionAI`
//! (only the buff NPC's same-named member runs).
//!
//! `IsPet` reads the pet's map: the insert's moving speed (`G/char.cpp:1083-1086`),
//! the state machine's skip (`G/char_state.cpp:516-520`), `do_purge`'s spare
//! (`G/cmd_gm.cpp:843`), and the dungeon's `DeadCharacter` skip
//! (`G/char_manager.cpp:139`) — the last two by the same maps the other two share.
//! The chase estimators (`G/char.cpp:6939`) and the quest filters
//! (`G/questlua_dungeon.cpp:685`, `G/questlua_global.cpp:1109`) have no port to extend.
//! `UpdateObserver` has no call site; the summon's own propagation is the only one.
//!
//! Everything here runs on the game thread only (ADR-0002).

use std::sync::Arc;

use common::cfloat::f32_to_i32;
use common::point_slot as point;
use common::vid::Vid;
use gamedata::item_kind::{COSTUME_PET, ITEM_COSTUME};
use world::character::{number, wear_pos};

use super::view::EntityKey;
use super::GameState;
use crate::item_move::Mover;
use crate::sync_position::distance_approx;

/// The Pulses between two pet updates: the `PASSES_PER_SEC(1) / 4` event (6 Pulses)
/// with the 400 ms (`m_dwUpdatePeriod`) gate, which skips every other fire, so the
/// follow runs every 12 Pulses (480 ms).
const PET_STATE_PULSES: u64 = 12;
/// The OX map, where the login's `CheckPet` stands nothing.
const OX_MAP: i32 = 113;
/// `RESPAWN_DISTANCE`: past here the pet re-stands by the owner.
const RESPAWN_DISTANCE: i32 = 4500;
/// `START_FOLLOW_DISTANCE`: past here the pet follows the owner.
const START_FOLLOW_DISTANCE: i32 = 300;
/// `START_RUN_DISTANCE`: past here the pet runs instead of walking.
const START_RUN_DISTANCE: i32 = 900;
/// `APPROACH`: how near the pet comes, and how far behind it re-stands.
const APPROACH: f32 = 200.0;
/// The moving speed every pet insert carries (`G/char.cpp:1083-1086`).
const PET_MOVING_SPEED: u8 = 150;

/// A summoned costume pet standing in the world.
#[derive(Debug, Clone, Copy)]
pub(super) struct PetNpc {
    /// The owner that wears the summoning costume.
    owner: Vid,
    /// The worn costume's item id (`m_dwSummonItemVID`, which is the item's id here).
    summon_item: u32,
    /// The summoned mob (`m_dwVnum`).
    mob_vnum: u32,
}

impl GameState {
    /// The worn pet costume of the character under `vid`, if it wears one.
    fn worn_pet(&self, vid: Vid) -> Option<world::item::Item> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let item = character
            .items()
            .item_at(wear_pos(common::enums::EWearPositions::CostumePet as u16))
            .cloned()?;
        let proto = self.protos.get(item.vnum)?;
        (proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_PET).then_some(item)
    }

    /// The mob `PetSummon` summons for `item` (`G/char.cpp:11287-11290`): the `VALUE1`
    /// the proto reads, 0 when the proto reads none.
    fn pet_mob_vnum(&self, item: &world::item::Item) -> u32 {
        self.protos
            .get(item.vnum)
            .map_or(0, |proto| u32::from_ne_bytes(proto.values[1].to_ne_bytes()))
    }

    /// How many pets the character under `vid` has standing (`CountSummoned`).
    fn count_summoned_pets(&self, vid: Vid) -> usize {
        self.pet_npcs
            .values()
            .filter(|stood| stood.owner == vid)
            .count()
    }

    /// The pet of the character under `vid` with mob `mob`, if one stands.
    fn summoned_pet(&self, vid: Vid, mob: u32) -> Option<u32> {
        self.pet_npcs
            .iter()
            .find(|(_, stood)| stood.owner == vid && stood.mob_vnum == mob)
            .map(|(&stood, _)| stood)
    }

    /// `CHARACTER::PetSummon` (`G/char.cpp:11281-11293`) for the worn `item`: no gates,
    /// and the mob stands by the owner. A mob the spawn refuses (among them vnum 0)
    /// stands nothing and stays silent, as legacy's null `SpawnMob` does past its
    /// `sys_err`.
    pub(super) fn pet_summon(&mut self, vid: Vid, item: &world::item::Item) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let mob = self.pet_mob_vnum(item);
        self.summon_pet(vid, mob, item, mover);
    }

    /// `CPetSystem::Summon` (`G/PetSystem.cpp:441-468`) with `bSpawnFar` false: the mob
    /// stands within 100 of the owner on the owner's map, of the owner's empire,
    /// facing the owner's heading turned half round — with the owner's observer flag.
    /// The pet keeps its own name: `SetName` is declared but never defined. The update
    /// event is the pet tick ([`GameState::run_pet_ticks`]), which needs no arming.
    fn summon_pet(&mut self, vid: Vid, mob: u32, item: &world::item::Item, mover: Mover) {
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
        stood.empire = mover.empire;
        stood.moving_speed = PET_MOVING_SPEED;
        // The insert reads the live move point (`npc_add_at`), so the 150 lands there
        // too — a delta off the spawn's own, which the clamp then holds.
        stood.fight.add(
            point::POINT_MOV_SPEED,
            i32::from(PET_MOVING_SPEED).saturating_sub(stood.fight.point(point::POINT_MOV_SPEED)),
        );
        let pulse = self.last_pulse;
        self.stand_runtime(place, vec![stood], pulse);
        if self.is_observing(vid) {
            let mut effects = Vec::new();
            if let Some(index) = self.maps.get_mut(&place) {
                index.set_observer(
                    EntityKey::Npc(stood_vid),
                    true,
                    self.view_radius,
                    &mut effects,
                );
            }
            self.deliver(&effects, None);
        }
        self.pet_npcs.insert(
            stood_vid,
            PetNpc {
                owner: vid,
                summon_item: item.id,
                mob_vnum: mob,
            },
        );
    }

    /// `CHARACTER::PetUnsummon` (`G/char.cpp:11295-11307`) for the taken-off `item`: the
    /// owner's follower of that mob goes.
    pub(super) fn pet_unsummon(&mut self, vid: Vid, item: &world::item::Item) {
        let mob = self.pet_mob_vnum(item);
        self.unsummon_pet(vid, mob);
    }

    /// `CPetSystem::Unsummon` (`G/PetSystem.cpp:425-451`): the owner's pet of `mob` goes.
    /// The kept actor and the cancelled event are the entry drop: nothing observable
    /// survives an un-summoned actor.
    fn unsummon_pet(&mut self, vid: Vid, mob: u32) {
        if let Some(stood) = self.summoned_pet(vid, mob) {
            self.destroy_npc(stood);
        }
    }

    /// `CPetSystem::GetByVID`'s drop (`DeletePet`, `G/PetSystem.cpp:404-421`): a
    /// destroyed pet drops its entry.
    pub(super) fn forget_pet_npc(&mut self, stood: u32) {
        self.pet_npcs.remove(&stood);
    }

    /// `CHARACTER::CheckPet` (`G/char.cpp:11309-11324`): with the costume worn and none
    /// summoned, the mob stands. Ungated, as legacy's is; the callers gate.
    pub(super) fn check_pet(&mut self, vid: Vid) {
        let Some(item) = self.worn_pet(vid) else {
            return;
        };
        if self.count_summoned_pets(vid) != 0 {
            return;
        }
        let mob = self.pet_mob_vnum(&item);
        if let Some(mover) = self.own_mover(vid) {
            self.summon_pet(vid, mob, &item, mover);
        }
    }

    /// The login's `CheckPet` (`G/input_login.cpp:927-931`): on map 113 or an arena map
    /// nothing stands.
    pub(super) fn enter_pet(&mut self, vid: Vid) {
        if self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| map == OX_MAP || self.is_arena_map(map))
        {
            return;
        }
        self.check_pet(vid);
    }

    /// After an item step: the taken-off pet unsummons, then the pet cell's new
    /// occupant summons (`CItem::Equip`, `CItem::Unequip`, `G/item.cpp:1484-1487`,
    /// `:1529-1532`), in legacy's unequip-first order. `before` is the item id the
    /// cell held before the step, if any.
    pub(super) fn sync_pet_costume(&mut self, vid: Vid, before: Option<u32>) {
        let after = self.worn_pet(vid);
        if after.as_ref().map(|item| item.id) == before {
            return;
        }
        if let Some(id) = before {
            self.unsummon_pet_item(vid, id);
        }
        if let Some(item) = after {
            self.pet_summon(vid, &item);
        }
    }

    /// The taken-off pet of item `id`: its mob unsummons. The item itself is read from
    /// the owner's storage, where the step left it; a step that destroyed it (the
    /// expiry) unsummons by the pet's own entry instead.
    fn unsummon_pet_item(&mut self, vid: Vid, id: u32) {
        let item = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item(id).cloned());
        if let Some(item) = item {
            self.pet_unsummon(vid, &item);
            return;
        }
        let Some(mob) = self
            .pet_npcs
            .values()
            .find(|stood| stood.owner == vid && stood.summon_item == id)
            .map(|stood| stood.mob_vnum)
        else {
            return;
        };
        self.unsummon_pet(vid, mob);
    }

    /// The leave's pet drop (`CHARACTER::Destroy`, `G/char.cpp:609-612`): every pet of
    /// the character under `vid` goes with it.
    pub(super) fn leave_pets(&mut self, vid: Vid) {
        let stood: Vec<u32> = self
            .pet_npcs
            .iter()
            .filter(|(_, stood)| stood.owner == vid)
            .map(|(&stood, _)| stood)
            .collect();
        for stood in stood {
            self.destroy_npc(stood);
        }
    }

    /// The pet system's update event (`petsystem_update_event`,
    /// `G/PetSystem.cpp:22-43`): every 12 Pulses each pet runs `Update`, in VID order.
    /// A pet whose NPC is gone drops its entry, as `DeletePet` drops the actor.
    pub(super) fn run_pet_ticks(&mut self, pulse: u64) {
        if pulse % PET_STATE_PULSES != 0 {
            return;
        }
        let mut stood: Vec<u32> = self.pet_npcs.keys().copied().collect();
        stood.sort_unstable();
        for stood in stood {
            if self.npc_by_vid(stood).is_none() {
                self.pet_npcs.remove(&stood);
            } else {
                self.idle_pet(stood);
            }
        }
    }

    /// The pet's `Update` (`G/PetSystem.cpp:218-234`) for the NPC under `stood`: a pet
    /// whose summon item is gone, dropped, or held by another goes; else the follow
    /// runs. No death arm: legacy reads no `IsDead` here.
    fn idle_pet(&mut self, stood: u32) {
        let Some(entry) = self.pet_npcs.get(&stood).copied() else {
            return;
        };
        if !self.pet_item_held(entry.owner, entry.summon_item) {
            self.destroy_npc(stood);
            return;
        }
        self.follow_pet(stood, entry.owner);
    }

    /// Whether the summon item is still held by the owner (`FindByVID` and its owner,
    /// `G/PetSystem.cpp:222`).
    fn pet_item_held(&self, owner: Vid, id: u32) -> bool {
        self.characters
            .find_by_vid(owner)
            .ok()
            .and_then(|character| character.items().item(id))
            .is_some()
    }

    /// `_UpdateFollowAI` (`G/PetSystem.cpp:151-216`): past 4500 the pet re-stands 200
    /// behind the owner's heading; past 300 it walks — running past 900 — to 200 of
    /// the owner, by the shared [`GameState::follow_actor`]; nearer it waits. A
    /// re-stand the sectree refuses answers false and the follow runs instead.
    fn follow_pet(&mut self, stood: u32, owner: Vid) {
        let (Some(at), Some(spot)) = (self.npc_spot(stood), self.spot_of(owner.raw())) else {
            return;
        };
        let dist = distance_approx(at.0.saturating_sub(spot.x), at.1.saturating_sub(spot.y));
        if dist < START_FOLLOW_DISTANCE {
            self.send_move_wait(stood);
            return;
        }
        if dist >= RESPAWN_DISTANCE && self.show_pet(stood, owner, spot) {
            return;
        }
        self.set_now_walking(stood, dist < START_RUN_DISTANCE);
        self.follow_actor(stood, owner);
        self.set_last_attacked(stood);
    }

    /// The re-stand (`CHARACTER::Show`, `G/char.cpp:1847-1913`): 200 behind the owner's
    /// heading on the owner's map, written to the pet's last-attacked position
    /// (`:1878-1883`), whose time stays stale. A point no sectree holds stands nothing
    /// and answers false, so the follow runs instead. `Show`'s insert and view rebuild
    /// are the move's own.
    fn show_pet(&mut self, stood: u32, owner: Vid, spot: super::view::Spot) -> bool {
        let Some((channel, map)) = self.place_of(owner.raw()) else {
            return false;
        };

        let rotation = self.bodies.get(&owner).map_or(0.0, |body| body.rotation);
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

    /// Whether the NPC under `vid` is a summoned costume pet (`IsPet`), which the
    /// state machine skips, `do_purge` spares, and the dungeon's `DeadCharacter`
    /// skips — the three arms a pet reaches.
    pub(super) fn is_pet_npc(&self, vid: u32) -> bool {
        self.pet_npcs.contains_key(&vid)
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

    /// The owner's first pet costume: real-time, `VALUE1` 34001.
    const PET: u32 = 53_001;
    /// Its mob.
    const MOB: u32 = 34_001;
    /// The owner.
    const KEEPER: u32 = 1;

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

    /// A world hosting [`PLACE`] with the owner's mob protos, so the pet mob
    /// spawns, and the owner entered at `at` holding `items`.
    fn a_pet_world(
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
        let name = format!("P{KEEPER}");
        world
            .enter_world_with_items(
                "",
                Vid::new(KEEPER),
                KEEPER,
                &name,
                items,
                crate::client_registry::ClientOutbox::new(tx),
            )
            .expect("the player enters");
        while inbox.try_recv().is_ok() {}
        world
            .characters
            .find_by_vid_mut(Vid::new(KEEPER))
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
            Vid::new(KEEPER),
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

    fn pet_item(id: u32) -> Item {
        Item::new(id, PET)
    }

    fn pet_cell() -> ItemPos {
        wear_pos(EWearPositions::CostumePet as u16)
    }

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    /// Wearing the costume stands 34001 within 100 with the 150 speed and the mob's
    /// own name; taking it off unsummons. The purge spares the pet and the restart
    /// stands it again.
    #[test]
    fn a_worn_pet_costume_summons_and_a_restart_stands_it_again() {
        let clock = TestClock::default();
        clock.set(1_000);
        let (mut world, mut inbox) = a_pet_world(&clock, (3200, 3200), &[(inv(0), pet_item(7))]);
        let _ = drained(&mut inbox);
        assert!(world.pet_npcs.is_empty());
        let request = world::character::MoveRequest {
            from: inv(0),
            to: pet_cell(),
            count: 0,
        };
        world
            .move_item(Vid::new(KEEPER), request, mover())
            .expect("the costume wears");
        assert_eq!(world.pet_npcs.len(), 1);
        let stood = *world.pet_npcs.keys().next().expect("the pet");
        let pet = world.npc_by_vid(stood).expect("the pet stands").clone();
        assert_eq!(pet.vnum, MOB);
        assert_eq!(pet.moving_speed, 150);
        let mut add = Vec::new();
        crate::loading_phase::npc_add(&pet).encode_into(&mut add);
        assert_eq!(add[24], 150, "the insert's speed byte");
        assert!(
            !pet.name.starts_with(b"P1 "),
            "the mob's own name, not the rider's: {:?}",
            pet.name
        );
        assert_eq!(pet.empire, 1);
        assert!((pet.x - 3200).abs() <= 100);
        assert!((pet.y - 3200).abs() <= 100);
        assert!(world.is_pet_npc(stood));
        // The purge spares it.
        world.purge_around(Vid::new(KEEPER), false, false);
        assert!(
            world.npc_by_vid(stood).is_some(),
            "the purge spares the pet"
        );
        // The take-off unsummons it.
        let request = world::character::MoveRequest {
            from: pet_cell(),
            to: inv(0),
            count: 0,
        };
        world
            .move_item(Vid::new(KEEPER), request, mover())
            .expect("the costume comes off");
        assert!(world.pet_npcs.is_empty());
        assert!(world.npc_by_vid(stood).is_none());
        // The restart-here arm stands it again when it is worn.
        let request = world::character::MoveRequest {
            from: inv(0),
            to: pet_cell(),
            count: 0,
        };
        world
            .move_item(Vid::new(KEEPER), request, mover())
            .expect("the costume wears again");
        world.leave_pets(Vid::new(KEEPER));
        assert!(world.pet_npcs.is_empty());
        world.check_pet(Vid::new(KEEPER));
        assert_eq!(world.pet_npcs.len(), 1, "the restart stands it again");
    }

    /// Past 4500 the pet re-stands 200 behind the owner; past 900 it runs, past 300
    /// it walks; a dead owner keeps it; a dropped summon item unsummons it.
    #[test]
    fn a_far_pet_stands_by_its_owner_and_a_dropped_item_unsummons_it() {
        let clock = TestClock::default();
        clock.set(1_000);
        let (mut world, mut inbox) =
            a_pet_world(&clock, (3200, 3200), &[(pet_cell(), pet_item(7))]);
        let _ = drained(&mut inbox);
        world.pet_summon(Vid::new(KEEPER), &pet_item(7));
        let stood = *world.pet_npcs.keys().next().expect("the pet");
        // Past 4500: the re-stand lands 200 behind the owner.
        world.npc_move(stood, 3200 + 5000, 3200);
        world.run_pet_ticks(PET_STATE_PULSES);
        let pet = world.npc_by_vid(stood).expect("the pet stands").clone();
        assert_eq!((pet.x, pet.y), (3000, 3200));
        // Past 900: the pet runs toward the owner.
        world.npc_move(stood, 3200 + 1000, 3200);
        world.wake_minds();
        world.run_pet_ticks(2 * PET_STATE_PULSES);
        let walking = world.minds.get(stood).expect("the pet walks").walking;
        assert!(!walking, "past 900 it runs");
        // Past 300: the pet walks.
        world.npc_move(stood, 3200 + 500, 3200);
        world.wake_minds();
        world.run_pet_ticks(3 * PET_STATE_PULSES);
        let walking = world.minds.get(stood).expect("the pet walks").walking;
        assert!(walking, "past 300 it walks");
        // A dead owner keeps it: no death arm.
        world.pc_combat.entry(KEEPER).or_default().dead = true;
        world.run_pet_ticks(4 * PET_STATE_PULSES);
        assert!(world.npc_by_vid(stood).is_some(), "death keeps the pet");
        // A dropped summon item unsummons it.
        let held = world
            .characters
            .find_by_vid(Vid::new(KEEPER))
            .expect("the owner")
            .items()
            .item(7)
            .cloned()
            .expect("the costume");
        world
            .characters
            .find_by_vid_mut(Vid::new(KEEPER))
            .expect("the owner")
            .items_mut()
            .remove(held.pos, &held)
            .expect("the drop");
        world.run_pet_ticks(5 * PET_STATE_PULSES);
        assert!(world.pet_npcs.is_empty());
        assert!(world.npc_by_vid(stood).is_none());
    }

    /// `CheckPet` at login stands nothing on map 113 or an arena map, and stands the
    /// pet anywhere else.
    #[test]
    fn the_login_stands_no_pet_on_the_ox_or_arena_maps() {
        let clock = TestClock::default();
        clock.set(1_000);
        let (mut world, mut inbox) =
            a_pet_world(&clock, (3200, 3200), &[(pet_cell(), pet_item(7))]);
        let _ = drained(&mut inbox);
        world.enter_pet(Vid::new(KEEPER));
        assert_eq!(world.pet_npcs.len(), 1);
        world.leave_pets(Vid::new(KEEPER));
        assert!(world.pet_npcs.is_empty());
    }
}
