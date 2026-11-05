//! What a smith's refine does to the item: `CHARACTER::DoRefine`'s swap
//! (`G/char_item.cpp:1421-1585`). A success makes the refined item with
//! `ITEM_MANAGER::CreateItem(result_vnum, 1, 0, false)` and copies the old one onto it
//! (`ITEM_MANAGER::CopyAllAttrTo`, `G/item_manager.cpp:1965-2013`); the old item then leaves
//! (`RemoveItem`) and the new one takes its cell (`AddToCharacter`). A failure only removes
//! the old item.
//!
//! `CreateItem` with `bTryMagic` false runs the addon, but `CopyAttributeTo` then replaces the
//! whole attribute array (`G/item_attribute.cpp:331-335,378-381`), so the addon is not rolled.
//!
//! `G/` = `server/server/game/`.

use gamedata::item_kind::{ARMOR_EAR, ARMOR_NECK, ARMOR_WRIST, ITEM_ARMOR, ITEM_BELT};
use gamedata::item_proto::ItemProto;
use protocol::item_pos::ItemPos;

use super::drop_item::stamp_created_sockets;
use super::item_move::{ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord};
use super::items::CharacterItems;
use super::quickslot::{QuickslotSync, SyncTo};
use crate::item::{gc_item_clear, Item, ItemId, SOCKETS};

/// `ITEM_BROKEN_METIN_VNUM` (`G/item_manager.cpp:1998`): a broken stone, which is not copied.
const ITEM_BROKEN_METIN_VNUM: i32 = 28_960;

/// `ITEM_STONES_MAX_NUM` with `ENABLE_EXTENDED_SOCKETS` (`C/item_length.h:13-15`): the
/// sockets a stone may sit in.
const ITEM_STONES_MAX_NUM: usize = 3;

/// `CItem::IsAccessoryForSocket` (`G/item.cpp:2273-2277`): a bracelet, necklace or earring,
/// or a belt.
fn is_accessory_for_socket(proto: &ItemProto) -> bool {
    (proto.item_type == ITEM_ARMOR
        && [ARMOR_WRIST, ARMOR_NECK, ARMOR_EAR].contains(&proto.sub_type))
        || proto.item_type == ITEM_BELT
}

/// The refined item `id` of `result_vnum` (with `made` its proto) that a success makes of
/// `old` (with `old_proto` its proto): `CreateItem`'s size, flags and sockets, then
/// `CopyAllAttrTo`.
///
/// An accessory keeps every socket. Any other item gets a 1 for each socket the old one
/// filled before its first empty one, and then its stones (a socket past 2 that is not
/// the broken stone) packed from socket 0. The refine element and the attributes follow.
#[must_use]
pub fn refined_item(
    old: &Item,
    old_proto: &ItemProto,
    (id, result_vnum): (ItemId, u32),
    made: &ItemProto,
    now_secs: i64,
) -> Item {
    let mut item = Item::new(id, result_vnum);
    item.size = {
        let [size, ..] = made.size.to_le_bytes();
        size
    };
    item.flags = made.flags;
    item.anti_flags = made.anti_flags;
    stamp_created_sockets(&mut item, made, now_secs);
    if is_accessory_for_socket(old_proto) {
        item.sockets = old.sockets;
    } else {
        for (socket, held) in item.sockets.iter_mut().zip(old.sockets) {
            if held == 0 {
                break;
            }
            *socket = 1;
        }
        let mut slot = 0;
        for &stone in old.sockets.iter().take(ITEM_STONES_MAX_NUM.min(SOCKETS)) {
            if stone > 2 && stone != ITEM_BROKEN_METIN_VNUM {
                item.sockets[slot] = stone;
                slot += 1;
            }
        }
    }
    if old.refine_element != 0 {
        item.refine_element = old.refine_element;
    }
    item.attributes = old.attributes;
    item
}

/// `RemoveItem` of the item in `pos`, and on a success `AddToCharacter` of `made` in the
/// same cell. The slots naming the cell are deleted first (`G/item_manager.cpp:566-598`).
/// Nothing changes when the cell is empty, and the new item is not placed when the
/// storage refuses it; the caller checks the fit first.
#[must_use]
pub fn swap_refined(items: &mut CharacterItems, pos: ItemPos, made: Option<&Item>) -> MoveDone {
    let mut done = MoveDone {
        kind: MoveKind::Refined,
        records: Vec::new(),
        changes: Vec::new(),
    };
    let Some(id) = items.item_at(pos).map(|held| held.id) else {
        return done;
    };
    done.records.push(MoveRecord::QuickslotSync(QuickslotSync {
        from: pos.cell,
        to: SyncTo::Delete,
    }));
    if items.release(id).is_err() {
        return done;
    }
    done.records
        .push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
    done.changes.push(ItemChange::Destroyed { id });
    let Some(made) = made else {
        return done;
    };
    let mut item = made.clone();
    item.pos = pos;
    if items.set(pos, &item).is_err() {
        return done;
    }
    done.records
        .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 0))));
    done.changes.push(ItemChange::Created(item));
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamedata::item_kind::{ITEM_WEAPON, WEAPON_SWORD};
    use protocol::gc_item_window::ItemAttribute;

    fn proto(vnum: u32, item_type: i32, sub_type: i32) -> ItemProto {
        let mut proto = ItemProto::for_category_rule(vnum, item_type, sub_type);
        proto.size = 2;
        proto
    }

    fn old(sockets: [i32; SOCKETS]) -> Item {
        let mut item = Item::new(9, 16);
        item.sockets = sockets;
        item.refine_element = 0x0102_0304;
        item.attributes[0] = ItemAttribute {
            b_type: 7,
            s_value: 0x1234,
        };
        item
    }

    #[test]
    fn a_weapon_keeps_its_open_sockets_and_packs_its_stones() {
        let sword = proto(17, ITEM_WEAPON, WEAPON_SWORD);
        let held = old([28_030, 1, ITEM_BROKEN_METIN_VNUM, 28_031, 0, 0]);
        let made = refined_item(&held, &sword, (10, 17), &sword, 0);
        assert_eq!(made.vnum, 17);
        assert_eq!(made.size, 2);
        // Four sockets were filled before the first empty one; the stones of the first
        // three sockets are packed from 0, the broken one skipped, the fourth not read.
        assert_eq!(made.sockets, [28_030, 1, 1, 1, 0, 0]);
        assert_eq!(made.refine_element, 0x0102_0304);
        assert_eq!(made.attributes, held.attributes);
    }

    #[test]
    fn an_accessory_keeps_every_socket() {
        let bracelet = proto(14_000, ITEM_ARMOR, ARMOR_WRIST);
        let held = old([28_030, 0, 5, 0, 7, 0]);
        let made = refined_item(&held, &bracelet, (10, 14_001), &bracelet, 0);
        assert_eq!(made.sockets, held.sockets);
    }
}
