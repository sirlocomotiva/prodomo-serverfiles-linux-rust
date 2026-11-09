//! The real-time item expiry (`real_time_expire_event`, `G/item.cpp:2027-2087`): a held
//! `LIMIT_REAL_TIME` or started `LIMIT_REAL_TIME_START_FIRST_USE` item whose socket 0 has
//! passed is removed (`RemoveItem`). The first-use starts live in `world`'s use.

use common::vid::Vid;
use gamedata::item_kind::LIMIT_REAL_TIME;
use world::character::{ItemChange, MoveDone, MoveKind, MoveRecord, QuickslotSync, SyncTo};
use world::item::{gc_item_clear, Item};

use super::GameState;
use crate::item_move::StoreOrder;
use crate::save::PASSES_PER_SEC;

/// The expiry socket (`ITEM_SOCKET_REMAIN_SEC`, `C/item_length.h:55`): the `get_global_time()`
/// second the item expires at.
const EXPIRY_SOCKET: usize = 0;

/// The use counter (`UseItemEx`'s socket 1, `G/char_item.cpp:2809-2819`): a first-use item
/// whose counter is 0 has no clock yet, so the expiry skips it.
const USE_SOCKET: usize = 1;

impl GameState {
    /// One Pulse of real-time expiry (`real_time_expire_event` on `PASSES_PER_SEC(1)`):
    /// every held item whose limit is real-time and whose socket 0 has passed is removed,
    /// as `RemoveItem` removes it with the owner logged on (`G/item_manager.cpp:566-595`).
    ///
    /// A worn expiry unequips first, so the points go as `RemoveFromCharacter` takes them
    /// (`G/item.cpp:369-377`); then the row is ordered and the client hears the clear. An
    /// expiry while the item is offered in a trade waits: legacy's one caller is the
    /// character's own event, and a dying trade item would strand the trade window.
    pub(super) fn run_real_time_expiry(&mut self, pulse: u64) {
        if pulse % u64::from(PASSES_PER_SEC) != 0 {
            return;
        }
        let now = Self::global_time();
        let mut expired: Vec<(Vid, u32)> = Vec::new();
        for (_, vid) in self.characters.player_names() {
            let Ok(character) = self.characters.find_by_vid(vid) else {
                continue;
            };
            for (pos, _) in character.items().occupied() {
                let Some(item) = character.items().item_at(pos) else {
                    continue;
                };
                if character.items().is_exchanging(item.id) {
                    continue;
                }
                if real_time_passed(&self.protos, item, now) {
                    expired.push((vid, item.id));
                }
            }
        }
        for (vid, id) in expired {
            self.expire_real_time_item(vid, id);
        }
        let mut stored: Vec<(Vid, bool, u32)> = Vec::new();
        for (vid, storage) in &self.storages {
            for (mall, window) in [(false, storage.safebox()), (true, storage.mall())]
                .into_iter()
                .filter_map(|(mall, window)| window.map(|window| (mall, window)))
            {
                for item in window.items() {
                    if real_time_passed(&self.protos, item, now) {
                        stored.push((*vid, mall, u32::from(item.pos.cell)));
                    }
                }
            }
        }
        for (vid, mall, pos) in stored {
            self.expire_stored_item(vid, mall, pos);
        }
    }

    /// Remove the expired item `id` of the character under `vid`: the quickslot sync,
    /// then the wear-cell points when it is worn, then the clear and the destroy row.
    fn expire_real_time_item(&mut self, vid: Vid, id: u32) {
        let Some(actor) = self.own_mover(vid) else {
            return;
        };
        let expired = self.run_item_step(vid, actor, |items, _ids, _rules, gear, _protos| {
            let item = items
                .item(id)
                .cloned()
                .ok_or(world::character::MoveRefused::Empty)?;
            let records = vec![MoveRecord::QuickslotSync(QuickslotSync {
                from: item.pos.cell,
                to: SyncTo::Delete,
            })];
            let mut done = expire_item(items, gear, &item)?;
            let mut all = records;
            all.append(&mut done.records);
            done.records = all;
            Ok(done)
        });
        let Ok(expired) = expired else {
            return;
        };
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

    /// Remove the expired item at `pos` of the open safebox or mall `mall` names: the
    /// DEL record to the owner, and the rows to its descriptor.
    fn expire_stored_item(&mut self, vid: Vid, mall: bool, pos: u32) {
        let Some(storage) = self.storages.get_mut(&vid) else {
            return;
        };
        let window = storage.window_mut(mall);
        let Some(open) = window.as_mut() else {
            return;
        };
        let account = open.account();
        let Some(removed) = open.remove_expired(pos) else {
            return;
        };
        let records: Vec<Vec<u8>> = removed
            .done
            .records
            .iter()
            .filter_map(|record| match record {
                MoveRecord::Store(store) => Some(crate::item_move::store_record(*store)),
                _ => None,
            })
            .collect();
        for record in records {
            let _sent = self.write_to_client(vid, record);
        }
        let changes: Vec<db::items::AccountChange> = removed
            .changes
            .iter()
            .filter_map(|change| match change {
                world::character::StoreChange::Destroyed { id } => {
                    Some(db::items::AccountChange::Destroyed { id: *id })
                }
                _ => None,
            })
            .collect();
        if changes.is_empty() {
            return;
        }
        let Some((channel, map)) = self.place_of(vid.raw()) else {
            return;
        };
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        let raw = vid.raw();
        let found = clients
            .members_on_map(channel, map)
            .into_iter()
            .find(|(_, entry)| entry.vid == raw)
            .map(|(id, _)| id);
        if let Some(id) = found {
            clients.order(
                channel,
                id,
                crate::client_registry::ClientOrder::AccountRows { account, changes },
            );
        }
    }
}

/// Whether `item`'s real-time clock has passed at `now`: a real-time limit, or a started
/// first-use one, whose socket 0 lies behind. An unstamped first-use item (`G/item.cpp:2775-2780`)
/// waits for its clock.
fn real_time_passed(protos: &gamedata::item_proto::ItemProtos, item: &Item, now: u32) -> bool {
    let Some(proto) = protos.get(item.vnum) else {
        return false;
    };
    let real_time = proto
        .limits
        .iter()
        .any(|limit| limit.kind == LIMIT_REAL_TIME);
    let first_use = proto.real_time_first_use.is_some();
    if !real_time && !first_use {
        return false;
    }
    if first_use && item.sockets[USE_SOCKET] == 0 {
        return false;
    }
    #[allow(clippy::cast_sign_loss)]
    let at = item.sockets[EXPIRY_SOCKET] as u32;
    now > at
}

/// `RemoveItem` of one held item with the owner logged on (`G/item_manager.cpp:566-595`):
/// the quickslot sync is the caller's, then the worn points, the release, the clear, and
/// the recompute, with the destroy row. `gear` is `None` for a character without points.
fn expire_item(
    items: &mut world::character::CharacterItems,
    gear: Option<&mut world::character::Gear<'_>>,
    item: &Item,
) -> Result<MoveDone, world::character::MoveRefused> {
    use world::character::ItemRecord;
    use world::character::{recompute_points, unequip_points, wear_of, MoveRefused};

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
    use common::item_slots::EWindows;
    use protocol::item_pos::ItemPos;

    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock};

    /// The wearer.
    const VID: u32 = 0x6002;

    /// The owner's skin: `Skin-Aquarian-Pumnal`, vnum 40130, `LIMIT_REAL_TIME` 86400.
    const SKIN: u32 = 40_130;

    /// A world with the wearer entered holding `item` in cell 1.
    fn held(item: Item) -> (GameState, u64) {
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
        (world, u64::from(PASSES_PER_SEC))
    }

    /// A passed skin leaves on the second's tick: the clear and the destroy row. A
    /// skin with its day ahead stays, and an unstamped first-use item waits for its
    /// clock, as the load arm does (`G/item.cpp:2775-2780`).
    #[test]
    fn a_passed_skin_leaves_and_a_live_one_stays() {
        let now = GameState::global_time();
        let mut passed = Item::new(7, SKIN);
        passed.sockets[0] = i32::try_from(now.saturating_sub(1)).unwrap_or(i32::MAX);
        let (mut world, tick) = held(passed);
        world.run_real_time_expiry(tick);
        assert!(
            world
                .characters
                .find_by_vid(Vid::new(VID))
                .expect("the wearer")
                .items()
                .item(7)
                .is_none(),
            "the passed skin leaves"
        );
        let mut live = Item::new(8, SKIN);
        live.sockets[0] = i32::try_from(now.saturating_add(86_400)).unwrap_or(i32::MAX);
        let (mut world, tick) = held(live);
        world.run_real_time_expiry(tick);
        assert!(
            world
                .characters
                .find_by_vid(Vid::new(VID))
                .expect("the wearer")
                .items()
                .item(8)
                .is_some(),
            "the live skin stays"
        );
        let mut unstarted = Item::new(9, 45_055);
        unstarted.sockets = [0; world::item::SOCKETS];
        let (mut world, tick) = held(unstarted);
        world.run_real_time_expiry(tick);
        assert!(
            world
                .characters
                .find_by_vid(Vid::new(VID))
                .expect("the wearer")
                .items()
                .item(9)
                .is_some(),
            "an unstamped first-use item waits for its clock"
        );
    }

    /// A passed skin stored in the open safebox leaves on the tick too
    /// (`RemoveItem`'s safebox arm, `G/item_manager.cpp:574-585`).
    #[test]
    fn a_passed_stored_skin_leaves() {
        use db::items::SAFEBOX;

        use crate::game_state::safebox::SafeboxStep;

        let now = GameState::global_time();
        let mut passed = Item::new(11, SKIN);
        passed.sockets[0] = i32::try_from(now.saturating_sub(1)).unwrap_or(i32::MAX);
        passed.pos = ItemPos::new(SAFEBOX, 0);
        let (mut world, tick) = held(Item::new(12, 27_001));
        let mover = world.own_mover(Vid::new(VID)).expect("the wearer moves");
        assert!(matches!(
            world.safebox(Vid::new(VID), SafeboxStep::BeginOpen, mover),
            Ok(crate::game_state::safebox::SafeboxAnswer::Proceed)
        ));
        let open = world.safebox(
            Vid::new(VID),
            SafeboxStep::Open {
                account: 9,
                items: vec![passed],
            },
            mover,
        );
        assert!(
            matches!(open, Ok(crate::game_state::safebox::SafeboxAnswer::Sent(_))),
            "the safebox opens"
        );
        world.run_real_time_expiry(tick);
        let storage = world.storages.get(&Vid::new(VID)).expect("the windows");
        assert!(
            storage
                .safebox()
                .expect("the safebox")
                .items()
                .next()
                .is_none(),
            "the passed stored skin leaves"
        );
    }
}
