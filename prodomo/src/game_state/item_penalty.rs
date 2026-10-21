//! `CHARACTER::ItemDropPenalty` (`G/char_battle.cpp:1016-1199`): the items a player loses
//! when it dies.
//!
//! Legacy's `Dead` calls the penalty for a victim with a killer, outside an agreed duel, a
//! guild war, a duel, a forked road and a castle map, and only for a character that is not a
//! GM (`G/char_battle.cpp:1333-1345`). The penalty itself:
//!
//! - refuses a character in a shop, one under level 50, and one on a battle arena map;
//! - reads the victim's **real** alignment and takes the row of the table it falls in;
//! - draws once for the inventory and once for the equipment, and drops what the draws allow;
//! - shuffles the slots that hold items and drops a random number of them;
//! - refuses the items whose anti-flags forbid it, and the protected inventory this build
//!   does not have;
//! - lays the items on the ground where the victim fell, scattering each one after the first;
//! - and, when the victim wears the unique item that skips the penalty, drops that item
//!   instead of the rest.
//!
//! # What this build does not have
//!
//! The private shop (`GetMyShop`, `:1019-1021`) and the protected inventory (`IsSecured`,
//! `:1101-1104` and `:1145-1148`) are not ported, so those two refusals cannot happen here.
//! The agreed PvP (`isAgreedPVP`, `:1280`), the arena duel (`isDuel`, `:1295`) and the forked
//! road (`isForked`, `:1329`) are not ported either, so those three flags are always false,
//! which is what keeps the penalty running.
//!
//! # The draws
//!
//! Legacy shuffles the slots with a **fresh** `std::mt19937` seeded from `std::random_device`
//! (`:1085-1087` and `:1124-1126`), not with the process-wide `number()`. The Rewrite uses the
//! game's one generator for every draw, so the numbers a penalty draws are not the numbers
//! legacy's would draw. Which items a penalty drops is a Divergence of the same kind as the
//! generator's own (see the note on [`crate::game_state::GameState::dice`]).
//!
//! # The rows
//!
//! The dropped items' rows are deleted by the descriptor that holds the victim, which is
//! the same way the character row is written (ADR-0003): the game thread cannot write to
//! the store, and the descriptor writes what the world holds. Legacy deletes the row in
//! the delayed-save queue's flush, up to 127 Pulses (about 5.1 seconds) after the drop
//! (`CItem::AddToGround` puts the item in the queue at `G/item.cpp:582`, and
//! `ITEM_MANAGER::SaveSingleItem` deletes the row of an item that has no owner at
//! `G/item_manager.cpp:485-495`), so for that time the rows still name the victim. The
//! Rewrite's rows go at once, and the pick-ups of the dropped items wait for that write.
//! The delete is written on the `owner_id` the item had (`db/src/items.rs`), so a pick-up
//! that takes the item before the delete is written is not undone by it.

use common::enums::EWearPositions;
use common::item_slots::{EWindows, INVENTORY_MAX_NUM, WEAR_MAX_NUM};
use common::vid::Vid;
use protocol::item_pos::ItemPos;
use world::character::number;
use world::character::{
    drop_penalty_item, wear_pos, DropAt, GroundItem, ItemChange, MoveDone, MoveKind, MoveRecord,
};

use crate::command::GM_PLAYER;

use super::death::Attacker;
use super::GameState;

/// `UNIQUE_ITEM_SKIP_ITEM_DROP_PENALTY` (`G/unique_item.h:29`): worn, it makes the penalty
/// drop that item and nothing else.
const UNIQUE_ITEM_SKIP_ITEM_DROP_PENALTY: u32 = 70_049;

/// `nBATTLE_ARENA_MAP` (`G/BattleArena.h:2`), the maps the battle arena is on. The arena
/// itself is not ported, but the map list is, so a character that stands on one of them is
/// spared the penalty as legacy spares it (`G/char_battle.cpp:1025-1029`).
const BATTLE_ARENA_MAPS: [i32; 3] = [190, 191, 192];

/// `IS_CASTLE_MAP` (`G/castle.h:22`): the castle maps, where the penalty does not run
/// (`G/char_battle.cpp:1339`).
const CASTLE_MAPS: [i32; 3] = [181, 182, 183];

/// One row of `aItemDropPenalty_kor` (`G/char_battle.cpp:995-1014`).
///
/// The inventory percent is compared with a draw from 1 to 1000 and the equipment percent with
/// a draw from 1 to 100, so the inventory drops on a hundredth of the time the percent says
/// and the equipment on a hundredth more often.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    /// The chance the inventory drops, in percent.
    inventory_pct: i32,
    /// The most items the inventory may drop.
    inventory_qty: i32,
    /// The chance the equipment drops, in percent.
    equipment_pct: i32,
    /// The most items the equipment may drop.
    equipment_qty: i32,
}

/// The penalty table (`G/char_battle.cpp:1003-1014`), one row per alignment band.
const TABLE: [Row; 9] = [
    Row {
        inventory_pct: 0,
        inventory_qty: 0,
        equipment_pct: 0,
        equipment_qty: 0,
    },
    Row {
        inventory_pct: 0,
        inventory_qty: 0,
        equipment_pct: 0,
        equipment_qty: 0,
    },
    Row {
        inventory_pct: 0,
        inventory_qty: 0,
        equipment_pct: 0,
        equipment_qty: 0,
    },
    Row {
        inventory_pct: 0,
        inventory_qty: 0,
        equipment_pct: 0,
        equipment_qty: 0,
    },
    Row {
        inventory_pct: 0,
        inventory_qty: 0,
        equipment_pct: 0,
        equipment_qty: 0,
    },
    Row {
        inventory_pct: 25,
        inventory_qty: 1,
        equipment_pct: 5,
        equipment_qty: 1,
    },
    Row {
        inventory_pct: 50,
        inventory_qty: 2,
        equipment_pct: 10,
        equipment_qty: 1,
    },
    Row {
        inventory_pct: 75,
        inventory_qty: 4,
        equipment_pct: 15,
        equipment_qty: 1,
    },
    Row {
        inventory_pct: 100,
        inventory_qty: 8,
        equipment_pct: 20,
        equipment_qty: 1,
    },
];

/// The band the victim's real alignment falls in (`G/char_battle.cpp:1040-1058`).
///
/// The upper bounds are `>=` and the lower ones are `>`, as legacy writes them, so the band
/// of exactly 0 is the fifth and the band of exactly -40 000 is the sixth.
#[must_use]
pub(crate) fn band(alignment: i32) -> usize {
    if alignment >= 120_000 {
        0
    } else if alignment >= 80_000 {
        1
    } else if alignment >= 40_000 {
        2
    } else if alignment >= 10_000 {
        3
    } else if alignment >= 0 {
        4
    } else if alignment > -40_000 {
        5
    } else if alignment > -80_000 {
        6
    } else if alignment > -120_000 {
        7
    } else {
        8
    }
}

/// One item the penalty takes, and where it lands.
struct Planned {
    /// The cell the item is in.
    at: ItemPos,
    /// Where the item lands, in the map's units.
    x: i32,
    y: i32,
}

impl GameState {
    /// `CHARACTER::ItemDropPenalty` (`G/char_battle.cpp:1016-1199`): the victim under `vid`
    /// loses the items its alignment and the draws choose.
    ///
    /// The killer is read for the war only: legacy's `ItemDropPenalty` takes it and never uses
    /// it (`:1016`), and what the victim loses depends on the victim's own alignment.
    pub(super) fn item_drop_penalty(&mut self, vid: Vid, killer: Option<Attacker>) {
        if !self.penalty_applies(vid, killer) {
            return;
        }
        let Some(points) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned())
        else {
            return;
        };
        // `GetLevel() < 50` (`:1022-1024`).
        if points.level() < 50 {
            return;
        }
        // `GetGMLevel() == GM_PLAYER || test_server` (`:1341-1343`): a GM loses nothing.
        if points.gm_level() != GM_PLAYER {
            return;
        }
        // `GetMapIndex()` (`:1025` and `:1339`).
        let Some((channel, map)) = self.place_of(vid.raw()) else {
            return;
        };
        // `CBattleArena::instance().IsBattleArenaMap(GetMapIndex())` (`:1025-1029`).
        if BATTLE_ARENA_MAPS.contains(&map) {
            return;
        }
        // `IS_CASTLE_MAP(GetMapIndex())` (`:1339`).
        if CASTLE_MAPS.contains(&map) {
            return;
        }
        // `GetRealAlignment()` (`:1040-1058`), the alignment the rules read and the store
        // keeps, not the one the client is shown.
        let alignment = self.real_alignment_of(vid.raw());
        let mut planned = self.plan_the_dropped_items(vid, alignment);
        if planned.is_empty() {
            return;
        }
        // The ground (`G/char_battle.cpp:1176-1198`): the first item lands where the victim
        // fell, and every one after it a random offset away.
        let Some(spot) = self.spot_of(vid.raw()) else {
            return;
        };
        let (x, y, z) = (spot.x, spot.y, spot.z);
        for (index, item) in planned.iter_mut().enumerate() {
            if index > 0 {
                item.x = x + number(&mut self.dice, -7, 7) * 20;
                item.y = y + number(&mut self.dice, -7, 7) * 20;
            } else {
                item.x = x;
                item.y = y;
            }
        }
        self.lay_the_dropped_items(vid, &planned, (channel, map), (x, y, z));
    }

    /// The items the penalty takes (`:1064-1174`): the inventory's, the equipment's, and the
    /// unique item that skips the penalty.
    fn plan_the_dropped_items(&mut self, vid: Vid, alignment: i32) -> Vec<Planned> {
        let row = TABLE[band(alignment)];
        // `bDropInventory = r.iInventoryPct >= number(1, 1000)` and
        // `bDropEquipment = r.iEquipmentPct >= number(1, 100)` (`:1064-1065`).
        let draw_inventory = number(&mut self.dice, 1, 1_000);
        let draw_equipment = number(&mut self.dice, 1, 100);
        let mut drop_inventory = row.inventory_pct >= draw_inventory;
        let mut drop_equipment = row.equipment_pct >= draw_equipment;
        // `IsEquipUniqueItem(UNIQUE_ITEM_SKIP_ITEM_DROP_PENALTY)` (`:1068-1072`): the unique
        // item cancels both drops and is dropped instead.
        let mut drop_the_unique = false;
        if (drop_inventory || drop_equipment) && self.wears_the_skip_unique(vid) {
            drop_inventory = false;
            drop_equipment = false;
            drop_the_unique = true;
        }
        // The slots that hold an item (`:1078-1087` and `:1117-1126`).
        let inventory_slots = self.slots_with_items(vid, INVENTORY_MAX_NUM, EWindows::Inventory);
        let equipment_slots = self.worn_slots(vid);
        let inventory_empty = inventory_slots.is_empty();
        let mut planned = Vec::new();
        // The inventory's count (`:1089-1092`): at most the row's quantity, and a number from
        // 1 to that.
        if drop_inventory && !inventory_empty {
            let mut slots = inventory_slots;
            self.shuffle(&mut slots);
            let count = self.drawn_count(row.inventory_qty, slots.len());
            for slot in slots.iter().take(count) {
                let at = ItemPos {
                    window_type: EWindows::Inventory as u8,
                    cell: *slot,
                };
                if self.may_drop(vid, at) {
                    planned.push(Planned { at, x: 0, y: 0 });
                }
            }
        }
        // The equipment's count (`:1130-1136`): a number from 1 to the row's quantity, at most
        // the number of worn items, and then a number from 1 to that. The band of the worst
        // alignment drops every worn item when the inventory holds none (`:1110-1111`).
        let drop_all_equipment = drop_inventory && inventory_empty && band(alignment) == 8;
        if drop_equipment && !equipment_slots.is_empty() {
            let mut slots = equipment_slots;
            self.shuffle(&mut slots);
            let size = i32::try_from(slots.len()).unwrap_or(i32::MAX);
            // `iQty = MIN(vec_bSlots.size(), number(1, r.iEquipmentQty))` (`:1133`), and
            // then `iQty = number(1, iQty)` (`:1135-1136`): two draws, as legacy makes.
            let count = if drop_all_equipment {
                size
            } else {
                size.min(number(&mut self.dice, 1, row.equipment_qty))
            };
            let count = if count > 0 {
                usize::try_from(number(&mut self.dice, 1, count).max(0)).unwrap_or(0)
            } else {
                0
            };
            for slot in slots.iter().take(count) {
                let at = wear_pos(*slot);
                if self.may_drop(vid, at) {
                    planned.push(Planned { at, x: 0, y: 0 });
                }
            }
        }
        // The unique item that skips the penalty (`:1155-1174`): the two unique slots, and
        // only when they hold that item. Legacy does not read the anti-flags here; the item's
        // own anti-flags are empty (`item_proto.txt`, vnum 70049), so the world's check and
        // legacy's silence agree.
        if drop_the_unique {
            for wear in [EWearPositions::Unique1, EWearPositions::Unique2] {
                if self.unique_holds(vid, wear) {
                    planned.push(Planned {
                        at: wear_pos(wear as u16),
                        x: 0,
                        y: 0,
                    });
                }
            }
        }
        planned
    }

    /// The count the inventory's drop takes (`:1089-1092`): at most the row's quantity and
    /// the number of slots, and a number from 1 to that.
    fn drawn_count(&mut self, quantity: i32, slots: usize) -> usize {
        let at_most = quantity.min(i32::try_from(slots).unwrap_or(i32::MAX));
        if at_most <= 0 {
            return 0;
        }
        usize::try_from(number(&mut self.dice, 1, at_most).max(0)).unwrap_or(0)
    }

    /// Whether the death is the one legacy's `Dead` hands to the penalty
    /// (`G/char_battle.cpp:1333-1345`).
    ///
    /// The agreed PvP, the arena duel and the forked road are not ported, so their flags are
    /// always false; the guild war is.
    fn penalty_applies(&self, vid: Vid, killer: Option<Attacker>) -> bool {
        // `pkKiller` (`:1333`): a death with no killer is not the penalty's.
        let Some(killer) = killer else {
            return false;
        };
        // `!isUnderGuildWar` (`:1304-1306`): the war's own fight does not drop items. The
        // flag is read only when the killer is a player, as legacy reads it.
        if killer.pc && self.under_guild_war(vid, killer.vid) {
            return false;
        }
        true
    }

    /// The wear positions that hold an item, in the order legacy collects them
    /// (`G/char_battle.cpp:1117-1126`).
    ///
    /// Legacy's `GetWear(i)` reads the flat cell `INVENTORY_MAX_NUM + i` of the inventory
    /// (`G/char_item.cpp:647-656`), which is where the load puts a worn item
    /// (`prodomo/src/item_load.rs`).
    fn worn_slots(&self, vid: Vid) -> Vec<u16> {
        let Some(character) = self.characters.find_by_vid(vid).ok() else {
            return Vec::new();
        };
        let items = character.items();
        (0..WEAR_MAX_NUM)
            .filter(|wear| items.item_at(wear_pos(*wear)).is_some())
            .collect()
    }

    /// Whether the victim's guild is at war with the killer's (`isUnderGuildWar`,
    /// `G/char_battle.cpp:1304-1306`).
    fn under_guild_war(&self, victim: Vid, killer: Vid) -> bool {
        let guild_of = |vid: Vid| {
            self.characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| self.guild_of(character.player_id()))
        };
        let (Some(victim_guild), Some(killer_guild)) = (guild_of(victim), guild_of(killer)) else {
            return false;
        };
        self.guild_under_war(victim_guild, killer_guild)
    }

    /// The slots of one window that hold an item, in the order legacy collects them
    /// (`G/char_battle.cpp:1078-1087` and `:1117-1126`).
    ///
    /// The inventory's cells are the base inventory's, and the equipment's are the wear
    /// positions, which the flat space keeps after the inventory.
    fn slots_with_items(&self, vid: Vid, count: u16, window: EWindows) -> Vec<u16> {
        let Some(character) = self.characters.find_by_vid(vid).ok() else {
            return Vec::new();
        };
        let items = character.items();
        (0..count)
            .filter(|cell| {
                items
                    .item_at(ItemPos {
                        window_type: window as u8,
                        cell: *cell,
                    })
                    .is_some()
            })
            .collect()
    }

    /// Whether the item in the cell may be dropped by the penalty: the anti-flags
    /// `ITEM_ANTIFLAG_GIVE` and `ITEM_ANTIFLAG_PKDROP` forbid it (`G/char_battle.cpp:1098`
    /// and `:1142`).
    fn may_drop(&self, vid: Vid, at: ItemPos) -> bool {
        let Some(character) = self.characters.find_by_vid(vid).ok() else {
            return false;
        };
        character.items().item_at(at).is_none_or(|item| {
            item.anti_flags & (world::item::ITEM_ANTIFLAG_GIVE | world::item::ITEM_ANTIFLAG_PKDROP)
                == 0
        })
    }

    /// Whether the victim wears the unique item that skips the penalty
    /// (`IsEquipUniqueItem`, `G/char_item.cpp:9519-9540`).
    fn wears_the_skip_unique(&self, vid: Vid) -> bool {
        self.unique_holds(vid, EWearPositions::Unique1)
            || self.unique_holds(vid, EWearPositions::Unique2)
    }

    /// Whether one of the two unique slots holds the unique item that skips the penalty.
    fn unique_holds(&self, vid: Vid, wear: EWearPositions) -> bool {
        let Some(character) = self.characters.find_by_vid(vid).ok() else {
            return false;
        };
        character
            .items()
            .item_at(wear_pos(wear as u16))
            .is_some_and(|item| item.vnum == UNIQUE_ITEM_SKIP_ITEM_DROP_PENALTY)
    }

    /// A shuffle of the slots, as `std::shuffle` gives (`G/char_battle.cpp:1085-1087`).
    ///
    /// Legacy draws from a generator of its own here; the Rewrite draws from the game's one.
    /// The distribution is the same, the numbers are not.
    fn shuffle(&mut self, slots: &mut [u16]) {
        for index in (1..slots.len()).rev() {
            let other = number(&mut self.dice, 0, i32::try_from(index).unwrap_or(i32::MAX));
            let other = usize::try_from(other.max(0)).unwrap_or(0);
            slots.swap(index, other);
        }
    }

    /// Takes the planned items out of the victim and lays them on the ground
    /// (`G/char_battle.cpp:1176-1198`).
    fn lay_the_dropped_items(
        &mut self,
        vid: Vid,
        planned: &[Planned],
        key: (u8, i32),
        at: (i32, i32, i32),
    ) {
        let Some(character) = self.characters.find_by_vid(vid).ok() else {
            return;
        };
        let owner_id = character.player_id();
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        // The landing of each item is read before the step, because the step borrows the
        // world and the map index with it.
        let lands: Vec<bool> = planned
            .iter()
            .map(|item| {
                self.maps
                    .get(&key)
                    .and_then(|index| index.tree_at(item.x, item.y))
                    .is_some()
            })
            .collect();
        let mut records: Vec<MoveRecord> = Vec::new();
        let mut changes: Vec<ItemChange> = Vec::new();
        let mut ground_items: Vec<GroundItem> = Vec::new();
        // The next ground vid is taken by the item that lands, and a refused drop takes
        // none, so it moves only when an item was laid.
        let mut next_vid = self.next_ground_vid;
        let step = self.run_item_step(vid, mover, |items, _ids, _rules, mut gear, _protos| {
            for (index, item) in planned.iter().enumerate() {
                let to = DropAt {
                    vid: next_vid,
                    x: item.x,
                    y: item.y,
                    owner_id,
                    questing: false,
                    lands: lands[index],
                };
                match drop_penalty_item(items, gear.as_deref_mut(), item.at, to) {
                    Ok((done, ground)) => {
                        next_vid = next_vid.wrapping_add(1).max(1);
                        records.extend(done.records);
                        changes.extend(done.changes);
                        ground_items.push(ground);
                    }
                    // `AddToGround` finds no tree for the point
                    // (`G/item.cpp:569-574`; the call is `G/char_battle.cpp:1189`), a
                    // Defect. The Rewrite keeps it.
                    Err(_) => {}
                }
            }
            Ok(MoveDone {
                kind: MoveKind::Dropped,
                records,
                changes,
            })
        });
        let Ok(mut outcome) = step else {
            return;
        };
        self.next_ground_vid = next_vid;
        // The rows the drop takes are written by the victim's descriptor, and the pick-ups
        // of the dropped items wait for that: a pick-up writes the item's own row, and the
        // two rows would collide while the victim's row is still there. One order serves
        // every item of the drop, because one transaction writes them all.
        let (order, stored) = crate::item_move::StoreOrder::laid();
        for ground in ground_items {
            let laid = ground.vid;
            self.ground.insert(
                laid,
                super::Lying {
                    ground,
                    channel: key.0,
                    map: key.1,
                    expires: self.last_pulse.saturating_add(self.drop_lifetime),
                    owner: None,
                    stored: stored.clone(),
                },
            );
            let radius = self.view_radius;
            let mut effects = Vec::new();
            if let Some(index) = self.maps.get_mut(&key) {
                index.add_to_ground(
                    super::view::EntityKey::Ground(laid),
                    at,
                    radius,
                    &mut effects,
                );
            }
            let mut own = Vec::new();
            self.deliver(&effects, Some((vid.raw(), &mut own)));
            outcome.place_ground(own);
        }
        // The victim's client hears the quickslot deletions and the item clears first, and
        // the ground adds after them, which is the order legacy's two loops send them in.
        for record in &outcome.records {
            let _sent = self.write_to_client(vid, record.clone());
        }
        // The rows go to the descriptor that holds the victim, which writes them at once.
        self.order_the_item_rows(vid, key.0, key.1, outcome.changes, order);
    }

    /// Hands the rows the death penalty took to the descriptor that holds the victim.
    ///
    /// The game thread cannot write the store (ADR-0002), and the victim is dying, so the
    /// descriptor writes them and tells the pick-ups of the dropped items that they are
    /// gone. A victim whose descriptor has left leaves the rows where they are, and the
    /// pick-ups wait for a tell that never comes, which is what a drop whose dropper left
    /// does.
    fn order_the_item_rows(
        &self,
        vid: Vid,
        channel: u8,
        map: i32,
        changes: Vec<db::items::RowChange>,
        order: crate::item_move::StoreOrder,
    ) -> bool {
        let Some(clients) = self.clients.as_ref() else {
            return false;
        };
        let raw = vid.raw();
        clients
            .members_on_map(channel, map)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .is_some_and(|(id, _)| {
                clients.order(
                    channel,
                    id,
                    crate::client_registry::ClientOrder::ItemRows { changes, order },
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use common::item_slots::EWindows;
    use common::point_slot as point;
    use common::vid::Vid;
    use protocol::item_pos::ItemPos;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::character::Pcg32;
    use world::item::{Item, ITEM_ANTIFLAG_PKDROP};

    use super::*;
    use crate::client_registry::{ChannelClients, ClientEntry, ClientOrder, Lease};
    use crate::game_loop_messages::EnterPlace;
    use crate::game_state::fixtures::{a_world, points_at, TestClock, PLACE};
    use crate::loading_phase::{PcCard, PK_MODE_PROTECT};

    /// The player the penalty is tried on.
    const VICTIM: u32 = 0x1001;

    /// The killer: a monster, so the guild war never fires.
    const KILLER: u32 = 0x8000_0001;

    /// The alignment of the worst band (`G/char_battle.cpp:1013`), where the table drops the
    /// whole inventory and a fifth of the equipment.
    const WORST: i32 = -130_000;

    /// An item of `vnum` with `anti_flags`, as the load builds one.
    fn an_item(id: u32, vnum: u32, anti_flags: u32) -> Item {
        let mut item = Item::new(id, vnum);
        item.anti_flags = anti_flags;
        item
    }

    /// The victim's item in the inventory's `cell`.
    fn in_inventory(cell: u16) -> (ItemPos, Item) {
        (
            ItemPos::new(EWindows::Inventory as u8, cell),
            an_item(11, 19, 0),
        )
    }

    /// The victim's item in the equipment's `wear` slot, where the load puts it.
    fn in_equipment(wear: EWearPositions, vnum: u32) -> (ItemPos, Item) {
        (wear_pos(wear as u16), an_item(12, vnum, 0))
    }

    /// A world with a client registry, and the players that joined it.
    struct Field {
        world: GameState,
        clients: Arc<ChannelClients>,
        players: BTreeMap<u32, (Lease, UnboundedReceiver<Vec<u8>>)>,
    }

    impl Field {
        /// A world on [`PLACE`] with a client registry.
        fn a_field() -> Field {
            let clock = TestClock::default();
            let clients = Arc::new(ChannelClients::new());
            let world = a_world(&clock).with_clients(Arc::clone(&clients));
            Field {
                world,
                clients,
                players: BTreeMap::new(),
            }
        }

        /// The player `vid` of `level` and `alignment` joins `map` holding `items`.
        fn join_on(
            &mut self,
            vid: u32,
            map: i32,
            level: u8,
            alignment: i32,
            items: &[(ItemPos, Item)],
        ) {
            let name = format!("P{vid}");
            let mut lease = self.clients.join(ClientEntry {
                channel: PLACE.0,
                map,
                name: name.clone(),
                vid,
                empire: 1,
                language: 0,
            });
            let inbox = lease.take_receiver().expect("the inbox is home");
            self.world
                .enter_world_with_items(Vid::new(vid), vid, &name, items, lease.outbox())
                .expect("the player enters the world");
            self.world
                .characters
                .find_by_vid_mut(Vid::new(vid))
                .expect("the player is online")
                .set_points(Some(points_at(level, 0, 0)));
            let card = PcCard {
                name,
                job: 0,
                empire: 1,
                level,
                conqueror_level: 0,
                language: 0,
                pk_mode: PK_MODE_PROTECT,
                skill_group: 0,
                alignment,
            };
            self.world.place_body(
                Vid::new(vid),
                EnterPlace {
                    channel: PLACE.0,
                    map,
                    x: 0,
                    y: 0,
                    z: 0,
                },
                card,
            );
            let _joined = self.players.insert(vid, (lease, inbox));
        }

        /// The victim of `level` and `alignment` joins [`PLACE`] holding `items`.
        fn join(&mut self, level: u8, alignment: i32, items: &[(ItemPos, Item)]) {
            self.join_on(VICTIM, PLACE.1, level, alignment, items);
        }

        /// The orders the victim's descriptor was handed.
        fn orders(&mut self) -> Vec<ClientOrder> {
            let (lease, _) = self.players.get_mut(&VICTIM).expect("the player joined");
            let mut orders = lease.take_orders().expect("the queue is home");
            let got = std::iter::from_fn(|| orders.try_recv().ok()).collect();
            lease.put_orders(orders);
            got
        }

        /// Whether the victim's `window` window holds an item in `cell`.
        fn holds(&self, window: EWindows, cell: u16) -> bool {
            self.world
                .characters
                .find_by_vid(Vid::new(VICTIM))
                .expect("the player is online")
                .items()
                .item_at(ItemPos {
                    window_type: window as u8,
                    cell,
                })
                .is_some()
        }

        /// The items the penalty took, in the order it took them.
        fn lost(&self) -> Vec<u32> {
            self.world
                .ground
                .values()
                .map(|lying| lying.ground.item.id)
                .collect()
        }

        /// The victim's constitution (`POINT_HT`).
        fn vitality(&self) -> i32 {
            self.world
                .characters
                .find_by_vid(Vid::new(VICTIM))
                .expect("the player is online")
                .points()
                .expect("the player has points")
                .get_point(point::POINT_HT)
        }
    }

    /// A seed whose draws fall the way the worst band needs: the inventory's draw is at
    /// most 100 and the equipment's at most 20 (`G/char_battle.cpp:1064-1065`), and the
    /// scatter of the second item names a point a sectree holds.
    ///
    /// The scatter is the last pair of draws the penalty makes, and a negative coordinate
    /// names no sectree in the Rewrite (`world::cells` records legacy's aliasing of a
    /// position past the map's grid as a Defect): legacy passes the position as an
    /// unsigned number (`SECTREE_MANAGER::Get`, `G/sectree_manager.cpp:166-172`), whose
    /// 16-bit coordinates alias a sectree near the origin.
    fn a_dropping_seed() -> u64 {
        (0..100_000_u64)
            .find(|seed| {
                let mut dice = Pcg32::new(*seed, 0);
                // The inventory's and the equipment's draws.
                if number(&mut dice, 1, 1_000) > 100 || number(&mut dice, 1, 100) > 20 {
                    return false;
                }
                // The inventory's count, then the equipment's two counts.
                number(&mut dice, 1, 1);
                number(&mut dice, 1, 1);
                number(&mut dice, 1, 1);
                // The scatter of the second item.
                number(&mut dice, -7, 7) >= 0 && number(&mut dice, -7, 7) >= 0
            })
            .expect("a seed that drops both")
    }

    /// The band of every alignment, at the boundaries legacy writes
    /// (`G/char_battle.cpp:1040-1058`).
    #[test]
    fn the_alignment_bands_are_legacy_s() {
        assert_eq!(band(120_000), 0);
        assert_eq!(band(119_999), 1);
        assert_eq!(band(80_000), 1);
        assert_eq!(band(79_999), 2);
        assert_eq!(band(40_000), 2);
        assert_eq!(band(39_999), 3);
        assert_eq!(band(10_000), 3);
        assert_eq!(band(9_999), 4);
        assert_eq!(band(0), 4);
        assert_eq!(band(-1), 5);
        assert_eq!(band(-39_999), 5);
        assert_eq!(band(-40_000), 6);
        assert_eq!(band(-79_999), 6);
        assert_eq!(band(-80_000), 7);
        assert_eq!(band(-119_999), 7);
        assert_eq!(band(-120_000), 8);
        assert_eq!(band(-120_001), 8);
    }

    /// A victim under level 50 loses nothing (`GetLevel() < 50`, `G/char_battle.cpp:1022`).
    #[test]
    fn a_victim_under_level_fifty_keeps_its_items() {
        let mut field = Field::a_field();
        field.join(49, WORST, &[in_inventory(0)]);
        field.world.item_drop_penalty(
            Vid::new(VICTIM),
            Some(Attacker {
                vid: Vid::new(KILLER),
                pc: false,
            }),
        );
        assert!(
            field.holds(EWindows::Inventory, 0),
            "the item keeps its slot"
        );
        assert!(field.orders().is_empty(), "no rows are written");
    }

    /// A GM loses nothing (`GetGMLevel() == GM_PLAYER`, `G/char_battle.cpp:1341`).
    #[test]
    fn a_gm_keeps_its_items() {
        let mut field = Field::a_field();
        field.join(60, WORST, &[in_inventory(0)]);
        {
            let character = field
                .world
                .characters
                .find_by_vid_mut(Vid::new(VICTIM))
                .expect("the player is online");
            let (_, mut points) = character.items_and_points_mut();
            points
                .as_mut()
                .expect("the player has points")
                .set_gm_level(crate::command::GM_IMPLEMENTOR);
        }
        field.world.item_drop_penalty(
            Vid::new(VICTIM),
            Some(Attacker {
                vid: Vid::new(KILLER),
                pc: false,
            }),
        );
        assert!(
            field.holds(EWindows::Inventory, 0),
            "the item keeps its slot"
        );
    }

    /// A death with no killer is not the penalty's (`pkKiller`, `G/char_battle.cpp:1333`).
    #[test]
    fn a_death_without_a_killer_keeps_its_items() {
        let mut field = Field::a_field();
        field.join(60, WORST, &[in_inventory(0)]);
        field.world.item_drop_penalty(Vid::new(VICTIM), None);
        assert!(
            field.holds(EWindows::Inventory, 0),
            "the item keeps its slot"
        );
    }

    /// A victim on a battle arena map or a castle map loses nothing
    /// (`G/char_battle.cpp:1025-1029` and `:1339`).
    #[test]
    fn a_victim_on_a_spared_map_keeps_its_items() {
        for map in [BATTLE_ARENA_MAPS[0], CASTLE_MAPS[0]] {
            let mut field = Field::a_field();
            field.world.host_map(
                PLACE.0,
                map,
                gamedata::server_attr::SectreeGrid {
                    x: 0,
                    y: 0,
                    columns: 10,
                    rows: 10,
                },
            );
            field.join_on(VICTIM, map, 60, WORST, &[in_inventory(0)]);
            field.world.item_drop_penalty(
                Vid::new(VICTIM),
                Some(Attacker {
                    vid: Vid::new(KILLER),
                    pc: false,
                }),
            );
            assert!(
                field.holds(EWindows::Inventory, 0),
                "the item keeps its slot on {map}"
            );
        }
    }

    /// The worst band drops the whole inventory and a part of the equipment
    /// (`G/char_battle.cpp:1013`), and the rows go to the victim's descriptor.
    #[test]
    fn the_worst_band_drops_the_inventory_and_the_equipment() {
        let mut field = Field::a_field();
        field.join(
            60,
            WORST,
            &[in_inventory(0), in_equipment(EWearPositions::Body, 19)],
        );
        field.world.dice = Pcg32::new(a_dropping_seed(), 0);
        field.world.item_drop_penalty(
            Vid::new(VICTIM),
            Some(Attacker {
                vid: Vid::new(KILLER),
                pc: false,
            }),
        );
        assert!(
            !field.holds(EWindows::Inventory, 0),
            "the inventory item is on the ground"
        );
        assert!(
            !field.holds(EWindows::Inventory, INVENTORY_MAX_NUM),
            "the worn item is on the ground"
        );
        assert_eq!(field.lost(), vec![11, 12], "both items fell");
        let orders = field.orders();
        assert_eq!(orders.len(), 1, "one order carries the rows");
        let ClientOrder::ItemRows { changes, .. } = &orders[0] else {
            panic!("the order is the rows of the items that fell");
        };
        assert_eq!(changes.len(), 2, "both rows are written");
        assert!(
            changes.iter().any(
                |change| matches!(change, db::items::RowChange::Destroyed { id } if *id == 11)
            ),
            "the inventory item's row is deleted"
        );
        assert!(
            changes.iter().any(
                |change| matches!(change, db::items::RowChange::Destroyed { id } if *id == 12)
            ),
            "the worn item's row is deleted"
        );
    }

    /// An item whose anti-flags forbid it keeps its slot
    /// (`ITEM_ANTIFLAG_PKDROP`, `G/char_battle.cpp:1098`).
    #[test]
    fn an_item_that_may_not_be_dropped_keeps_its_slot() {
        let mut field = Field::a_field();
        let (at, _) = in_inventory(0);
        field.join(60, WORST, &[(at, an_item(11, 19, ITEM_ANTIFLAG_PKDROP))]);
        field.world.dice = Pcg32::new(a_dropping_seed(), 0);
        field.world.item_drop_penalty(
            Vid::new(VICTIM),
            Some(Attacker {
                vid: Vid::new(KILLER),
                pc: false,
            }),
        );
        assert!(
            field.holds(EWindows::Inventory, 0),
            "the item keeps its slot"
        );
        assert!(field.lost().is_empty(), "nothing fell");
    }

    /// A worn item takes its points with it (`CItem::Unequip`, `G/item.cpp:1544`): the
    /// earring's constitution goes when the penalty drops it.
    #[test]
    fn a_worn_item_takes_its_points_with_it() {
        let mut field = Field::a_field();
        field.join(
            60,
            WORST,
            &[in_inventory(0), in_equipment(EWearPositions::Ear, 17_040)],
        );
        let before = field.vitality();
        field.world.dice = Pcg32::new(a_dropping_seed(), 0);
        field.world.item_drop_penalty(
            Vid::new(VICTIM),
            Some(Attacker {
                vid: Vid::new(KILLER),
                pc: false,
            }),
        );
        assert!(
            !field.holds(
                EWindows::Inventory,
                INVENTORY_MAX_NUM + EWearPositions::Ear as u16
            ),
            "the earring is on the ground"
        );
        assert_eq!(
            field.vitality(),
            before - 1,
            "the earring's constitution goes with it"
        );
    }

    /// The unique item that skips the penalty is dropped instead of the rest
    /// (`UNIQUE_ITEM_SKIP_ITEM_DROP_PENALTY`, `G/char_battle.cpp:1068-1072`).
    #[test]
    fn the_unique_item_that_skips_the_penalty_is_dropped_instead() {
        let mut field = Field::a_field();
        field.join(
            60,
            WORST,
            &[
                in_inventory(0),
                in_equipment(EWearPositions::Unique1, UNIQUE_ITEM_SKIP_ITEM_DROP_PENALTY),
            ],
        );
        field.world.dice = Pcg32::new(a_dropping_seed(), 0);
        field.world.item_drop_penalty(
            Vid::new(VICTIM),
            Some(Attacker {
                vid: Vid::new(KILLER),
                pc: false,
            }),
        );
        assert!(
            field.holds(EWindows::Inventory, 0),
            "the inventory item keeps its slot"
        );
        assert!(
            !field.holds(
                EWindows::Inventory,
                INVENTORY_MAX_NUM + EWearPositions::Unique1 as u16
            ),
            "the unique item is on the ground"
        );
        assert_eq!(field.lost(), vec![12], "only the unique item fell");
    }
}
