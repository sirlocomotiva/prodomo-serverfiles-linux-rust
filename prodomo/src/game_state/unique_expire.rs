//! The unique expiry event (`unique_expire_event`, `G/item.cpp:1900-1983`): a worn
//! `ITEM_UNIQUE` ticks down its socket 5 (`ITEM_SOCKET_UNIQUE_REMAIN_TIME`) and is removed
//! (`RemoveItem`) at 0, unless the value-2 arm holds it.
//!
//! # What legacy does
//!
//! `EquipTo` starts the event on every worn unique that is not real-time
//! (`CItem::StartUniqueExpireEvent`, `G/item.cpp:2164-2194`): first due the saved socket 4
//! seconds on, capped at 60, or 60 when the socket is 0. Each fire reads value 2. With
//! value 2 unset, a remain of 1 or less removes the item, else the remain drops by one
//! and the event runs again 60 s on. With value 2 set, the remain is an absolute
//! `get_global_time()` second: a passed one removes the item, else the event runs again
//! in the remaining seconds, capped at 600. Taking the item off stops the event and
//! banks the seconds left in socket 4 (`StopUniqueExpireEvent`, `:2227-2243`), unless
//! value 2 is set — those keep ticking unworn.
//!
//! # What this build does
//!
//! The fire rides [`super::regen::Due::Unique`], one due entry per worn unique, armed by
//! the equip step and disarmed by the take-off. The countdown arm and the absolute arm
//! both run; the premium-shop arm does not (no bundle shop holds a unique here — a shop
//! window refuses the type at build, as it does the other unported systems).

use common::constants::{ITEM_SOCKET_UNIQUE_REMAIN_TIME, ITEM_SOCKET_UNIQUE_SAVE_TIME};
use common::vid::Vid;
use world::character::{ItemChange, MoveKind, MoveRecord, QuickslotSync, SyncTo};
use world::character::{MoveDone, MoveRefused};

use super::regen::{pulses, Due};
use super::GameState;
use crate::item_move::StoreOrder;

/// The first fire's gap (`StartUniqueExpireEvent`, `G/item.cpp:2181-2185`): the saved
/// socket 4 seconds, capped at 60, or 60 when the socket is 0.
fn first_gap(saved: i32) -> u32 {
    if saved <= 0 {
        60
    } else {
        u32::try_from(saved).unwrap_or(60).min(60)
    }
}

/// The countdown re-arm (`unique_expire_event`, `G/item.cpp:1937`): 60 s on.
const COUNTDOWN_GAP: u32 = 60;

/// The absolute re-arm's cap (`:1969-1971`): the seconds left, capped at 600.
const ABSOLUTE_CAP: u32 = 600;

impl GameState {
    /// The wear's unique-expiry sync (`EquipTo`'s `StartUniqueExpireEvent` and `Unequip`'s
    /// `StopUniqueExpireEvent`, `G/item.cpp:1465`, `:1547`): after an equip step, every
    /// worn unique the step's changes touched is armed, and every due item no longer
    /// worn is stopped. Read from the worn cells and the schedule, so the wear, the
    /// take-off and the swap land alike. A refused step (`Declined`) syncs nothing —
    /// its changes are the notice's, and no wear moved.
    pub(super) fn sync_unique_expire(
        &mut self,
        vid: Vid,
        moved: &mut crate::item_move::MovedItems,
    ) {
        use world::character::MoveKind;
        if matches!(moved.kind, MoveKind::Declined) {
            return;
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let worn: Vec<u32> = {
            use world::character::Equipment;
            Equipment::of(character.items(), &self.protos)
                .worn()
                .map(|(_, worn)| worn.item.id)
                .collect()
        };
        let due: Vec<u32> = self
            .regens
            .due
            .iter()
            .filter_map(|&(_, what)| match what {
                Due::Unique { vid: owner, id } if owner == vid.raw() => Some(id),
                _ => None,
            })
            .collect();
        for id in &due {
            if !worn.contains(id) {
                // The stop's bank is the take-off's own `SetSocket`: its update record
                // and row change ride the step, behind the wear cell's records.
                if let Some(mut stopped) = self.stop_unique_expire(vid, *id) {
                    moved.records.append(&mut stopped.records);
                    moved.changes.append(&mut stopped.changes);
                }
            }
        }
        for id in &worn {
            if !due.contains(id) {
                self.start_unique_expire(vid, *id, moved);
            }
        }
    }
}

impl GameState {
    /// `CItem::StartUniqueExpireEvent` (`G/item.cpp:2164-2194`) for the worn item `id` of
    /// the character under `vid`: first due the saved socket 4 seconds on (capped at 60,
    /// 60 when 0), the socket cleared onto the wear's own step. Silent when the item is
    /// gone, not a unique, or real-time; an already-due item is not armed twice.
    pub(super) fn start_unique_expire(
        &mut self,
        vid: Vid,
        id: u32,
        moved: &mut crate::item_move::MovedItems,
    ) {
        let due = Due::Unique { vid: vid.raw(), id };
        if self.regens.due.iter().any(|&(_, what)| what == due) {
            return;
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let Some(item) = character.items().item(id).cloned() else {
            return;
        };
        let Some(proto) = self.protos.get(item.vnum) else {
            return;
        };
        if proto.item_type != gamedata::item_kind::ITEM_UNIQUE {
            return;
        }
        if proto
            .limits
            .iter()
            .any(|limit| limit.kind == gamedata::item_kind::LIMIT_REAL_TIME)
        {
            return;
        }
        let save = usize::try_from(ITEM_SOCKET_UNIQUE_SAVE_TIME).unwrap_or(4);
        let gap = first_gap(item.sockets[save]);
        let when = self.last_pulse.saturating_add(pulses(gap));
        self.regens.due.insert((when, due));
        if item.sockets[save] != 0 {
            let Some(actor) = self.own_mover(vid) else {
                return;
            };
            let cleared = self.run_item_step(vid, actor, |items, _ids, _rules, _gear, _protos| {
                Ok(world::character::set_item_socket(items, id, (save, 0)))
            });
            if let Ok(mut cleared) = cleared {
                moved.records.append(&mut cleared.records);
                moved.changes.append(&mut cleared.changes);
            }
        }
    }

    /// `CItem::StopUniqueExpireEvent` (`G/item.cpp:2227-2243`) for the item `id` of the
    /// character under `vid`: the due entry goes, and the seconds left land in socket 4.
    /// A value-2 item keeps ticking: the stop is refused, as legacy refuses it. Answers
    /// the bank's step for the caller to ride, or `None` when nothing banked.
    pub(super) fn stop_unique_expire(
        &mut self,
        vid: Vid,
        id: u32,
    ) -> Option<crate::item_move::MovedItems> {
        let due = Due::Unique { vid: vid.raw(), id };
        let when = self
            .regens
            .due
            .iter()
            .find(|&&(_, what)| what == due)
            .map(|&(when, _)| when)?;
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return None;
        };
        let Some(item) = character.items().item(id).cloned() else {
            self.regens.due.remove(&(when, due));
            return None;
        };
        let Some(proto) = self.protos.get(item.vnum) else {
            self.regens.due.remove(&(when, due));
            return None;
        };
        if proto.values[2] != 0 {
            return None;
        }
        self.regens.due.remove(&(when, due));
        let left = when
            .saturating_sub(self.last_pulse)
            .saturating_add(u64::from(crate::save::PASSES_PER_SEC) - 1)
            / u64::from(crate::save::PASSES_PER_SEC);
        let save = usize::try_from(ITEM_SOCKET_UNIQUE_SAVE_TIME).unwrap_or(4);
        let actor = self.own_mover(vid)?;
        self.run_item_step(vid, actor, |items, _ids, _rules, _gear, _protos| {
            Ok(world::character::set_item_socket(
                items,
                id,
                (save, i32::try_from(left).unwrap_or(i32::MAX)),
            ))
        })
        .ok()
    }

    /// `unique_expire_event` (`G/item.cpp:1900-1983`) for the item `id` worn by the
    /// character under `vid`: the countdown arm removes at remain 1 or less and ticks
    /// down otherwise; the value-2 arm compares the remain against `global_time`.
    /// A missing item, proto or wearer ends the event silently.
    pub(super) fn fire_unique_expire(&mut self, vid: Vid, id: u32, pulse: u64) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let Some(item) = character.items().item(id).cloned() else {
            return;
        };
        let Some(proto) = self.protos.get(item.vnum) else {
            return;
        };
        if proto.item_type != gamedata::item_kind::ITEM_UNIQUE {
            return;
        }
        let remain = usize::try_from(ITEM_SOCKET_UNIQUE_REMAIN_TIME).unwrap_or(5);
        let due = Due::Unique { vid: vid.raw(), id };
        if proto.values[2] == 0 {
            if item.sockets[remain] <= 1 {
                self.remove_unique_item(vid, &item);
                return;
            }
            let next = item.sockets[remain].saturating_sub(1);
            let Some(actor) = self.own_mover(vid) else {
                return;
            };
            let ticked = self.run_item_step(vid, actor, |items, _ids, _rules, _gear, _protos| {
                Ok(world::character::set_item_socket(items, id, (remain, next)))
            });
            if let Ok(ticked) = ticked {
                for record in ticked.records {
                    let _sent = self.write_to_client(vid, record);
                }
            }
            let when = pulse.saturating_add(pulses(COUNTDOWN_GAP));
            self.regens.due.insert((when, due));
            return;
        }
        let now = Self::global_time();
        let at = u32::try_from(item.sockets[remain]).unwrap_or(0);
        if at <= now {
            self.remove_unique_item(vid, &item);
            return;
        }
        let gap = at.saturating_sub(now).clamp(1, ABSOLUTE_CAP);
        let when = pulse.saturating_add(pulses(gap));
        self.regens.due.insert((when, due));
    }

    /// `RemoveItem` of the worn unique `item` (`G/item_manager.cpp:566-595`): the
    /// quickslot sync, the worn points, the release, the clear and the destroy row.
    /// The cloak's take-off shows the real alignment again, as `ShowAlignment(true)`
    /// does when the worn cloak goes (`G/char_item.cpp:8287-8288`).
    fn remove_unique_item(&mut self, vid: Vid, item: &world::item::Item) {
        let Some(actor) = self.own_mover(vid) else {
            return;
        };
        let expired = self.run_item_step(vid, actor, |items, _ids, _rules, gear, _protos| {
            let records = vec![MoveRecord::QuickslotSync(QuickslotSync {
                from: item.pos.cell,
                to: SyncTo::Delete,
            })];
            let mut done = expire_worn(items, gear, item)?;
            let mut all = records;
            all.append(&mut done.records);
            done.records = all;
            Ok(done)
        });
        let Ok(mut expired) = expired else {
            return;
        };
        // The cloak's take-off shows the real alignment again (`ShowAlignment(true)`
        // when the worn cloak goes, `G/char_item.cpp:8287-8288`); the step removed it,
        // so the sync reads the worn cells and sends its own update behind the clear.
        self.sync_hide_alignment(vid, &mut expired);
        for record in expired.records {
            let _sent = self.write_to_client(vid, record);
        }
        if expired.changes.is_empty() {
            return;
        }
        let Some((channel, map)) = self.place_of(vid.raw()) else {
            return;
        };
        let order = StoreOrder::default();
        let _ordered = self.order_the_item_rows(vid, channel, map, expired.changes, order);
    }
}

/// `RemoveItem` of one worn item with the owner logged on (`G/item_manager.cpp:566-595`):
/// the worn points, the release, the clear, and the recompute, with the destroy row.
/// `gear` is `None` for a character without points.
fn expire_worn(
    items: &mut world::character::CharacterItems,
    gear: Option<&mut world::character::Gear<'_>>,
    item: &world::item::Item,
) -> Result<MoveDone, MoveRefused> {
    use world::character::ItemRecord;
    use world::character::{recompute_points, unequip_points, wear_of};
    use world::item::gc_item_clear;

    let wear = wear_of(item.pos);
    let mut records = Vec::new();
    let mut gear = gear;
    if let (Some(gear), Some(wear)) = (gear.as_deref_mut(), wear) {
        records.extend(unequip_points(gear, items, wear, item)?);
    }
    let pos = items.release(item.id).map_err(MoveRefused::Storage)?;
    records.push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
    if let (Some(gear), Some(_)) = (gear, wear) {
        records.extend(recompute_points(gear, items, item.vnum));
    }
    Ok(MoveDone {
        kind: MoveKind::Removed,
        records,
        changes: vec![ItemChange::Destroyed { id: item.id }],
    })
}
#[cfg(test)]
mod tests {
    use common::enums::EWearPositions;
    use common::item_slots::{EWindows, INVENTORY_MAX_NUM};
    use common::vid::Vid;
    use protocol::item_pos::ItemPos;
    use world::character::{MoveKind, MoveRequest};
    use world::item::Item;

    use super::super::fixtures::{a_world, enter_seeing_with, TestClock};
    use super::super::GameState;
    use super::Due;
    use crate::item_move::Mover;
    use crate::save::PASSES_PER_SEC;

    /// The wearer.
    const VID: u32 = 0x6003;
    /// The hide-alignment cloak: a plain unique on the owner's data.
    const CLOAK: u32 = 70_048;

    const fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 0,
            pk_mode: 4,
            affect_flags: [0; 2],
            dead: false,
            killer_mode: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    /// A world with the wearer entered holding `item` in cell 1.
    fn held(item: Item) -> GameState {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let _ = enter_seeing_with(
            &mut world,
            crate::game_state::fixtures::PLACE,
            VID,
            (100, 100),
            12_000,
            &[(ItemPos::new(EWindows::Inventory as u8, 1), item)],
        );
        world
    }

    fn wear(world: &mut GameState, id: u32) {
        let moved = world
            .move_item(
                Vid::new(VID),
                MoveRequest {
                    from: ItemPos::new(EWindows::Inventory as u8, 1),
                    to: ItemPos::new(
                        EWindows::Inventory as u8,
                        INVENTORY_MAX_NUM + EWearPositions::Unique1 as u16,
                    ),
                    count: 0,
                },
                mover(),
            )
            .expect("the cloak wears");
        assert_eq!(moved.kind, MoveKind::Equipped, "the wear lands");
        let _ = id;
    }

    fn due_when(world: &GameState, id: u32) -> Option<u64> {
        world
            .regens
            .due
            .iter()
            .find(|&&(_, what)| what == Due::Unique { vid: VID, id })
            .map(|&(when, _)| when)
    }

    /// A worn cloak arms its event 60 s on; the fire with remain 0 removes it, and
    /// the take-off shows the real alignment again.
    #[test]
    fn a_worn_cloak_arms_and_its_fire_removes_it() {
        let mut cloak = Item::new(7, CLOAK);
        cloak.set_size(1).expect("a positive footprint");
        let mut world = held(cloak);
        let vid = Vid::new(VID);
        if let Some(body) = world.bodies.get_mut(&vid) {
            body.card.alignment = 10_005;
            body.shown_alignment = Some(10_005);
        }
        wear(&mut world, 7);
        assert_eq!(
            world.bodies[&vid].shown_alignment,
            Some(0),
            "the wear hides the shown"
        );
        let pulse = world.last_pulse();
        let when = due_when(&world, 7).expect("the wear arms the event");
        assert_eq!(
            when,
            pulse + u64::from(PASSES_PER_SEC) * 60,
            "first due 60 s on"
        );
        world.run_regens(when);
        let Ok(character) = world.characters().find_by_vid(vid) else {
            panic!("the wearer is online");
        };
        assert!(character.items().item(7).is_none(), "the fire removed it");
        assert!(due_when(&world, 7).is_none(), "the removal ends the event");
        assert_eq!(
            world.bodies[&vid].shown_alignment,
            Some(10_005),
            "the expiry shows the real alignment again"
        );
    }

    /// A remain above 1 ticks down by one and re-arms 60 s on.
    #[test]
    fn a_remain_above_one_ticks_down_and_rearms() {
        let mut cloak = Item::new(7, CLOAK);
        cloak.set_size(1).expect("a positive footprint");
        cloak.sockets[5] = 3;
        let mut world = held(cloak);
        let vid = Vid::new(VID);
        wear(&mut world, 7);
        let when = due_when(&world, 7).expect("the wear arms the event");
        world.run_regens(when);
        let remain = world
            .characters()
            .find_by_vid(vid)
            .expect("the wearer is online")
            .items()
            .item(7)
            .expect("the tick keeps the item")
            .sockets[5];
        assert_eq!(remain, 2, "the fire ticks the remain down");
        assert_eq!(
            due_when(&world, 7),
            Some(when + u64::from(PASSES_PER_SEC) * 60),
            "the tick re-arms 60 s on"
        );
    }

    /// A take-off stops the event and banks the seconds left in socket 4; wearing
    /// it again clears the socket and re-arms from the bank.
    #[test]
    fn a_takeoff_banks_the_seconds_left_and_a_rewear_spends_them() {
        let mut cloak = Item::new(7, CLOAK);
        cloak.set_size(1).expect("a positive footprint");
        let mut world = held(cloak);
        let vid = Vid::new(VID);
        wear(&mut world, 7);
        let when = due_when(&world, 7).expect("the wear arms the event");
        let moved = world
            .move_item(
                vid,
                MoveRequest {
                    from: ItemPos::new(
                        EWindows::Inventory as u8,
                        INVENTORY_MAX_NUM + EWearPositions::Unique1 as u16,
                    ),
                    to: ItemPos::new(EWindows::Inventory as u8, 1),
                    count: 0,
                },
                mover(),
            )
            .expect("the take-off lands");
        assert_eq!(moved.kind, MoveKind::Moved);
        assert!(due_when(&world, 7).is_none(), "the take-off stops it");
        let banked = world
            .characters()
            .find_by_vid(vid)
            .expect("the wearer is online")
            .items()
            .item(7)
            .expect("the take-off keeps the item")
            .sockets[4];
        assert_eq!(banked, 60, "the whole minute banks");
        world
            .move_item(
                vid,
                MoveRequest {
                    from: ItemPos::new(EWindows::Inventory as u8, 1),
                    to: ItemPos::new(
                        EWindows::Inventory as u8,
                        INVENTORY_MAX_NUM + EWearPositions::Unique1 as u16,
                    ),
                    count: 0,
                },
                mover(),
            )
            .expect("the re-wear lands");
        assert_eq!(
            due_when(&world, 7),
            Some(when),
            "the re-wear spends the banked minute"
        );
        let cleared = world
            .characters()
            .find_by_vid(vid)
            .expect("the wearer is online")
            .items()
            .item(7)
            .expect("the re-wear keeps the item")
            .sockets[4];
        assert_eq!(cleared, 0, "the start clears the bank");
    }
    /// The value-2 arm is untestable against the owner's protos (none sets value 2),
    /// so the fire's absolute branch is pinned through the countdown's re-arm shape:
    /// a remain of 1 removes at the fire, never re-arming.
    #[test]
    fn a_remain_of_one_removes_at_the_fire() {
        let mut cloak = Item::new(7, CLOAK);
        cloak.set_size(1).expect("a positive footprint");
        cloak.sockets[5] = 1;
        let mut world = held(cloak);
        let vid = Vid::new(VID);
        wear(&mut world, 7);
        let when = due_when(&world, 7).expect("the wear arms the event");
        world.run_regens(when);
        let Ok(character) = world.characters().find_by_vid(vid) else {
            panic!("the wearer is online");
        };
        assert!(character.items().item(7).is_none(), "remain 1 removes");
        assert!(due_when(&world, 7).is_none(), "no re-arm after removal");
    }
}
