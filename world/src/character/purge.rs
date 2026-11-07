//! The GM's item lines: `/slow`, `/book` and `/ipurge` (`ACMD(do_slow)`, `do_book`,
//! `do_item_purge`, `G/cmd_gm.cpp:92-100`, `:2501-2526`, `:868-1016`).
//!
//! `slow` gives ten fresh +9 armours of heaven (vnum 12019) through `AutoGiveItem`,
//! each with its index in socket 4 (`SetSocket`, which sends `GC_ITEM_UPDATE` and
//! saves). The armour is `ITEM_TUNABLE` only, never stackable, so each give is a
//! fresh placement or nothing when no cell is free; the pickup line (`[LS;444]`)
//! rides per placed piece, as `bMsg` defaults to true. `ENABLE_SPECIAL_DROP_CHAT_RENEWAL`
//! is undefined, so the line is the chat packet, not the pickup packet.
//!
//! `ipurge` is the `#else` arm: `ENABLE_CMD_IPURGE_EX` is undefined, so the line takes
//! no argument and purges everything — cells `0..INVENTORY_AND_EQUIP_SLOT_MAX` (1370)
//! through `GetInventoryItem`, then `0..DRAGON_SOUL_INVENTORY_MAX_NUM` through `GetItem`
//! — with `ComputePoints` either side and the weapon-costume part reset (`SetPart`,
//! then `UpdatePacket`) under `ENABLE_WEAPON_COSTUME_SYSTEM`. Each flat removal syncs
//! the quickslots naming its cell twice: once by the purge loop, once by `RemoveItem`
//! itself (`G/item_manager.cpp:589`); the second sync finds no slot and sends nothing.
//! A worn piece leaves through the unequip point path, so the point changes ride as the
//! pieces go and the one final `ComputePoints` settles them.

use common::enums::EParts;
use common::item_slots::{EWindows, INVENTORY_AND_EQUIP_SLOT_MAX};
use gamedata::item_custom_category::{is_custom_category, CATEGORY_NUM};
use gamedata::item_proto::ItemProtos;
use protocol::item_pos::ItemPos;

use super::equip::{look_of, wear_of, Gear};
use super::equipment::{is_set_item, item_applies, removal_applies};
use super::item_move::{ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord};
use super::items::CharacterItems;
use super::quickslot::{QuickslotSync, SyncTo};
use crate::item::{gc_item_clear, Item, ItemIds};

/// The armour `/slow` gives: 12019, the +9 armour of heaven (`ITEM_ARMOR`,
/// `ARMOR_BODY`, `ITEM_TUNABLE`).
pub const SLOW_VNUM: u32 = 12019;

/// How many `/slow` gives: `for (BYTE i = 0; i < 10; i++)`.
pub const SLOW_COUNT: u8 = 10;

/// The socket `/slow` numbers: `pkItem->SetSocket(4, i)`.
pub const SLOW_SOCKET: usize = 4;

/// The book `/book` gives: 50300, the skill book (`ITEM_SKILLBOOK`).
pub const BOOK_VNUM: u32 = 50300;

/// The socket `/book` writes: `item->SetSocket(0, pkProto->dwVnum)`.
pub const BOOK_SOCKET: usize = 0;

/// The dragon soul cells `/ipurge` purges: `DRAGON_SOUL_INVENTORY_MAX_NUM` is
/// `DS_SLOT_MAX` (6) times `DRAGON_SOUL_GRADE_MAX` (6, with
/// `ENABLE_DRAGONSOUL_ALCHEMY_PLUS`) times `DRAGON_SOUL_BOX_SIZE` (32).
pub const DRAGON_SOUL_SLOTS: u16 = 1152;

/// `/slow` for the character's storage: up to ten fresh 12019s in free cells, each
/// numbered in socket 4, with the pickup line, the set, the update and the stored
/// rows per placed piece. A give with no free cell, no id, or no proto gives nothing,
/// as `AutoGiveItem` answering `NULL` does.
pub fn slow_give(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    usable_cells: u16,
    protos: &ItemProtos,
) -> MoveDone {
    let mut done = MoveDone {
        kind: MoveKind::Given,
        records: Vec::new(),
        changes: Vec::new(),
    };
    let Some(proto) = protos.get(SLOW_VNUM) else {
        return done;
    };
    let Some(ids) = ids else {
        return done;
    };
    for index in 0..SLOW_COUNT {
        let size = u8::try_from(proto.size).unwrap_or(0);
        let cell = (0..CATEGORY_NUM)
            .filter(|bank| is_custom_category(proto, *bank))
            .find_map(|bank| items.find_free_custom_cell(bank, size))
            .or_else(|| items.find_free_inventory_cell(usable_cells, size));
        let Some(cell) = cell else {
            continue;
        };
        let Ok(id) = ids.allocate() else {
            continue;
        };
        let pos = ItemPos::new(EWindows::Inventory as u8, cell);
        let mut item = Item::new(id, SLOW_VNUM);
        item.pos = pos;
        let _ = item.set_size(size);
        if items.set(pos, &item).is_err() {
            continue;
        }
        done.records.push(MoveRecord::PickedUp { vnum: SLOW_VNUM });
        done.records
            .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 1))));
        done.changes.push(ItemChange::Created(item));
        if items.set_socket(id, SLOW_SOCKET, i32::from(index)).is_err() {
            continue;
        }
        let Some(stored) = items.item(id) else {
            continue;
        };
        done.records.push(MoveRecord::Item(ItemRecord::Update(
            stored.gc_item_update(),
        )));
        done.changes.push(ItemChange::Sockets {
            id,
            sockets: stored.sockets,
        });
    }
    done
}

/// `/book` for the character's storage: one fresh 50300 in a free cell, with the
/// skill's vnum in socket 0, the pickup line, the set, the update and the stored rows.
/// A give with no free cell, no id, or no proto gives nothing, as `AutoGiveItem`
/// answering `NULL` does; legacy's unguarded `SetSocket` on that `NULL` would crash,
/// which is the Defect the guard refuses to reproduce.
pub fn book_give(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    usable_cells: u16,
    protos: &ItemProtos,
    skill: u32,
) -> MoveDone {
    let mut done = MoveDone {
        kind: MoveKind::Given,
        records: Vec::new(),
        changes: Vec::new(),
    };
    let Some(proto) = protos.get(BOOK_VNUM) else {
        return done;
    };
    let Some(ids) = ids else {
        return done;
    };
    let size = u8::try_from(proto.size).unwrap_or(0);
    let cell = (0..CATEGORY_NUM)
        .filter(|bank| is_custom_category(proto, *bank))
        .find_map(|bank| items.find_free_custom_cell(bank, size))
        .or_else(|| items.find_free_inventory_cell(usable_cells, size));
    let Some(cell) = cell else {
        return done;
    };
    let Ok(id) = ids.allocate() else {
        return done;
    };
    let pos = ItemPos::new(EWindows::Inventory as u8, cell);
    let mut item = Item::new(id, BOOK_VNUM);
    item.pos = pos;
    let _ = item.set_size(size);
    if items.set(pos, &item).is_err() {
        return done;
    }
    done.records.push(MoveRecord::PickedUp { vnum: BOOK_VNUM });
    done.records
        .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 1))));
    done.changes.push(ItemChange::Created(item));
    let Ok(skill) = i32::try_from(skill) else {
        return done;
    };
    if items.set_socket(id, BOOK_SOCKET, skill).is_err() {
        return done;
    }
    let Some(stored) = items.item(id) else {
        return done;
    };
    done.records.push(MoveRecord::Item(ItemRecord::Update(
        stored.gc_item_update(),
    )));
    done.changes.push(ItemChange::Sockets {
        id,
        sockets: stored.sockets,
    });
    done
}

/// `/ipurge` for the character's storage and gear: every flat cell then every dragon
/// soul cell is cleared, the quickslots naming a flat cell are synced (the loop's sync;
/// `RemoveItem`'s own second sync finds nothing), and `ComputePoints` runs before and
/// after with the weapon part reset and the look sent, as the `#else` arm does.
pub fn purge_all(items: &mut CharacterItems, gear: &mut Gear<'_>) -> MoveDone {
    let mut done = MoveDone {
        kind: MoveKind::Removed,
        records: Vec::new(),
        changes: Vec::new(),
    };
    push_compute(&mut done, gear);
    for cell in 0..INVENTORY_AND_EQUIP_SLOT_MAX {
        let pos = ItemPos::new(EWindows::Inventory as u8, cell);
        let Some(held) = items.item_at(pos) else {
            continue;
        };
        let id = held.id;
        done.records.push(MoveRecord::QuickslotSync(QuickslotSync {
            from: cell,
            to: SyncTo::Delete,
        }));
        release_flat(items, gear, &mut done, pos, id);
    }
    for cell in 0..DRAGON_SOUL_SLOTS {
        let pos = ItemPos::new(EWindows::DragonSoulInventory as u8, cell);
        let Some(held) = items.item_at(pos) else {
            continue;
        };
        let id = held.id;
        if items.release(id).is_err() {
            continue;
        }
        done.records
            .push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
        done.changes.push(ItemChange::Destroyed { id });
    }
    gear.points.set_part(EParts::Weapon, 0);
    push_compute(&mut done, gear);
    let look = look_of(items, gear.points, gear.protos);
    done.records.push(MoveRecord::Look(look));
    done
}

/// `ComputePoints` (`G/char.cpp`), whose point records ride.
fn push_compute(done: &mut MoveDone, gear: &mut Gear<'_>) {
    let records = gear.points.compute_points();
    done.records
        .extend(records.into_iter().map(MoveRecord::Point));
}

/// `RemoveItem` for a flat cell: a worn piece through the unequip point path, anything
/// else cleared from its cell, and the row destroyed either way.
fn release_flat(
    items: &mut CharacterItems,
    gear: &mut Gear<'_>,
    done: &mut MoveDone,
    pos: ItemPos,
    id: u32,
) {
    let Some(held) = items.item(id) else {
        return;
    };
    let Some(proto) = gear.protos.get(held.vnum) else {
        return;
    };
    let worn = wear_of(pos);
    if worn.is_some() {
        let applies = removal_applies(&item_applies(held, proto, gear.protos, gear.groups));
        for (apply, value) in applies {
            let records = gear.points.apply_point(apply, value);
            done.records
                .extend(records.into_iter().map(MoveRecord::Point));
        }
    }
    let vnum = held.vnum;
    if items.release(id).is_err() {
        return;
    }
    done.records
        .push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
    done.changes.push(ItemChange::Destroyed { id });
    if worn.is_some() {
        let records = if is_set_item(vnum) {
            gear.points.compute_points()
        } else {
            gear.points.compute_battle_points()
        };
        done.records
            .extend(records.into_iter().map(MoveRecord::Point));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::ItemIdRange;
    use gamedata::item_kind::ITEM_ARMOR;
    use gamedata::item_proto::ItemProto;

    const INV: u8 = EWindows::Inventory as u8;

    struct Fixed(u32);

    impl super::super::dice::Dice for Fixed {
        fn random31(&mut self) -> u32 {
            self.0
        }
    }

    fn protos() -> ItemProtos {
        ItemProtos::from_rows(vec![ItemProto::for_category_rule(SLOW_VNUM, ITEM_ARMOR, 0)])
    }

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(INV, cell)
    }

    #[test]
    fn slow_numbers_ten_sockets_and_reports_their_updates() {
        let protos = protos();
        let mut items = CharacterItems::default();
        let mut ids = ItemIds::new(ItemIdRange::new(1, 100, 1).expect("a range stands"));
        let done = slow_give(&mut items, Some(&mut ids), 90, &protos);
        assert_eq!(done.kind, MoveKind::Given);
        for index in 0..SLOW_COUNT {
            let held = items
                .item_at(inv(u16::from(index)))
                .expect("each give lands in the next free cell");
            assert_eq!(held.vnum, SLOW_VNUM);
            assert_eq!(held.sockets[SLOW_SOCKET], i32::from(index));
        }
        let notices = done
            .records
            .iter()
            .filter(|record| matches!(record, MoveRecord::PickedUp { .. }))
            .count();
        let updates = done
            .records
            .iter()
            .filter(|record| matches!(record, MoveRecord::Item(ItemRecord::Update(_))))
            .count();
        assert_eq!(notices, 10, "a pickup line per placed piece");
        assert_eq!(updates, 10, "an update per numbered socket");
        assert_eq!(
            done.changes.len(),
            20,
            "a create and a socket write per piece"
        );
    }

    #[test]
    fn slow_without_a_proto_or_an_id_gives_nothing() {
        let protos = ItemProtos::from_rows(Vec::new());
        let mut items = CharacterItems::default();
        let done = slow_give(&mut items, None, 90, &protos);
        assert!(done.records.is_empty());
        assert!(done.changes.is_empty());
    }

    #[test]
    fn purge_empties_both_arrays_resets_the_weapon_and_looks_once() {
        use super::super::equip::Gear;
        use super::super::points::{Points, PointsRow};
        let protos = protos();
        let mut items = CharacterItems::default();
        let mut ids = ItemIds::new(ItemIdRange::new(1, 100, 1).expect("a range stands"));
        let done = slow_give(&mut items, Some(&mut ids), 90, &protos);
        assert_eq!(done.changes.len(), 20);
        let mut points = Points::load(&PointsRow {
            race: 0,
            level: 10,
            exp: 0,
            conqueror_level: 0,
            st: 6,
            ht: 4,
            dx: 3,
            iq: 3,
            sungma: [0; 4],
            hp: 760,
            sp: 260,
            stamina: 820,
            inven_point: 0,
            random_hp: 0,
            random_sp: 0,
            stat_point: 0,
            skill_point: 0,
            sub_skill_point: 0,
            level_step: 0,
            stat_reset_count: 0,
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
            playtime: 0,
            conqueror_exp: 0,
            gaya: 0,
            biologist: [0; 3],
            secured: (0, 999_999),
        });
        points.set_part(EParts::Weapon, 12019);
        let groups = gamedata::special_item_group::SpecialItemGroups::default();
        let mut dice = Fixed(5);
        let mut gear = Gear {
            points: &mut points,
            protos: &protos,
            groups: &groups,
            dice: &mut dice,
            recently_fought: false,
        };
        let done = purge_all(&mut items, &mut gear);
        assert_eq!(done.kind, MoveKind::Removed);
        for index in 0..SLOW_COUNT {
            assert!(
                items.item_at(inv(u16::from(index))).is_none(),
                "cell {index} is clear"
            );
        }
        assert_eq!(gear.points.parts()[EParts::Weapon as usize], 0);
        let looks = done
            .records
            .iter()
            .filter(|record| matches!(record, MoveRecord::Look(_)))
            .count();
        assert_eq!(looks, 1, "the one UpdatePacket");
        let destroyed = done
            .changes
            .iter()
            .filter(|change| matches!(change, ItemChange::Destroyed { .. }))
            .count();
        assert_eq!(destroyed, 10);
    }
}

#[cfg(test)]
mod book_tests {
    use super::super::items::CharacterItems;
    use super::*;
    use crate::item::ItemIdRange;
    use gamedata::item_kind::ITEM_SKILLBOOK;
    use gamedata::item_proto::ItemProto;

    fn protos() -> ItemProtos {
        ItemProtos::from_rows(vec![ItemProto::for_category_rule(
            BOOK_VNUM,
            ITEM_SKILLBOOK,
            0,
        )])
    }

    #[test]
    fn book_writes_the_skill_into_socket_zero() {
        let protos = protos();
        let mut items = CharacterItems::default();
        let mut ids = ItemIds::new(ItemIdRange::new(1, 100, 1).expect("a range stands"));
        let done = book_give(&mut items, Some(&mut ids), 90, &protos, 3);
        assert_eq!(done.kind, MoveKind::Given);
        let held = (0..INVENTORY_AND_EQUIP_SLOT_MAX)
            .find_map(|cell| items.item_at(ItemPos::new(EWindows::Inventory as u8, cell)))
            .expect("the book lands in a free cell");
        assert_eq!(held.vnum, BOOK_VNUM);
        assert_eq!(held.sockets[BOOK_SOCKET], 3);
        assert_eq!(done.records.len(), 3, "the pickup, the set, the update");
        assert_eq!(done.changes.len(), 2, "the create and the socket write");
    }

    #[test]
    fn book_without_a_proto_gives_nothing() {
        let protos = ItemProtos::from_rows(Vec::new());
        let mut items = CharacterItems::default();
        let done = book_give(&mut items, None, 90, &protos, 3);
        assert!(done.records.is_empty());
        assert!(done.changes.is_empty());
    }
}
