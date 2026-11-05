//! The arms of `CHARACTER::UseItemEx` that act on a second item, the one a
//! `CG_ITEM_USE_TO_ITEM` names (`G/char_item.cpp:2718-6990`).
//!
//! `G/` = `server/server/game/`.

use gamedata::item_kind::{
    ITEM_ARMOR, ITEM_PICK, ITEM_ROD, ITEM_WEAPON, WEARABLE_BODY, WEARABLE_WEAPON,
};
use gamedata::item_proto::{ItemProto, ItemProtos};
use protocol::item_pos::ItemPos;

use super::equip::Trail;
use super::item_move::{done, MoveDone, MoveKind, MoveRecord};
use super::items::CharacterItems;
use super::{ItemChange, ItemRecord, MoveRefused};
use crate::item::{Item, SOCKETS};

/// A stone of the same kind already sits in the target (`G/char_item.cpp:6996`).
pub const SAME_STONE_NOTICE: &str = "[LS;426]";
/// A stone for armour put into armour, or armour that is not body armour.
pub const NOT_BODY_STONE_NOTICE: &str = "[LS;427]";
/// A stone that is not for weapons put into a weapon.
pub const NOT_WEAPON_STONE_NOTICE: &str = "[LS;428]";
/// No socket takes the stone, or the target takes no stone.
pub const NO_SOCKET_NOTICE: &str = "[LS;431]";
/// The stone sits in its socket (`ENABLE_ADDSTONE_FAILURE` is off, so it always does).
pub const STONE_SET_NOTICE: &str = "[LS;1613]";

/// `item->SetCount(item->GetCount() - 1)` for a used item: the last one leaves its cell,
/// with its slots following the next of its vnum when the proto chains them.
///
/// # Errors
///
/// A storage refusal of the release or the count, which changes nothing.
pub fn spend_one(
    items: &mut CharacterItems,
    item: &Item,
    proto: &ItemProto,
) -> Result<MoveDone, MoveRefused> {
    let mut trail = Trail::default();
    super::potion::use_one(
        items,
        item,
        super::quickslot::chains_when_used_up(proto),
        &mut trail,
    )?;
    Ok(done(MoveKind::Used, trail))
}

/// The target of an item used on another: the item at `dest`, unless the cell is not a
/// valid one, holds nothing, or holds an item offered in a trade or worn (`@fixme114`).
/// Legacy returns false for each without a line.
#[must_use]
pub fn use_target(items: &CharacterItems, dest: ItemPos) -> Option<Item> {
    if !super::inventory::is_valid_item_position(dest) {
        return None;
    }
    let target = items.item_at(dest)?.clone();
    if items.is_exchanging(target.id) || super::inventory::is_equip_position(target.pos) {
        return None;
    }
    Some(target)
}

/// The `ITEM_METIN` arm of `UseItemEx` (`G/char_item.cpp:6965-7034`): the `stone` goes into
/// the first free socket of the item at `dest` that its grade fits, and one stone is spent.
///
/// A target that [`use_target`] refuses, a pick and a rod are refused without a line. A
/// stone whose kind (`VALUE5`) a socket already holds is refused with [`SAME_STONE_NOTICE`];
/// armour takes only a body stone into body armour, a weapon only a weapon stone, and any
/// other item none. The free sockets are 1 and 2; the first that is at least the stone's
/// `VALUE2` takes the stone's vnum, with [`STONE_SET_NOTICE`]. With
/// `ENABLE_ADDSTONE_FAILURE` off the stone never breaks.
///
/// # Errors
///
/// A storage refusal, which changes nothing.
pub fn put_stone(
    items: &mut CharacterItems,
    (stone, stone_proto): (&Item, &ItemProto),
    dest: ItemPos,
    protos: &ItemProtos,
) -> Result<MoveDone, MoveRefused> {
    let silent = || Ok(done(MoveKind::Used, Trail::default()));
    let told = |text| {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(text));
        Ok(done(MoveKind::Used, trail))
    };
    let Some(target) = use_target(items, dest) else {
        return silent();
    };
    let Some(target_proto) = protos.get(target.vnum) else {
        return silent();
    };
    if target_proto.item_type == ITEM_PICK || target_proto.item_type == ITEM_ROD {
        return silent();
    }
    let kind = stone_proto.values[5];
    let doubled = target.sockets.iter().any(|&socket| {
        socket > 2
            && u32::try_from(socket)
                .ok()
                .and_then(|vnum| protos.get(vnum))
                .is_some_and(|held| held.values[5] == kind)
    });
    if doubled {
        return told(SAME_STONE_NOTICE);
    }
    if target_proto.item_type == ITEM_ARMOR {
        if stone_proto.wear_flags & WEARABLE_BODY == 0
            || target_proto.wear_flags & WEARABLE_BODY == 0
        {
            return told(NOT_BODY_STONE_NOTICE);
        }
    } else if target_proto.item_type == ITEM_WEAPON {
        if stone_proto.wear_flags & WEARABLE_WEAPON == 0 {
            return told(NOT_WEAPON_STONE_NOTICE);
        }
    } else {
        return told(NO_SOCKET_NOTICE);
    }
    let grade = stone_proto.values[2];
    let Some(index) = (0..SOCKETS).find(|&index| {
        let socket = target.sockets[index];
        (1..=2).contains(&socket) && socket >= grade
    }) else {
        return told(NO_SOCKET_NOTICE);
    };
    let vnum = i32::try_from(stone.vnum).map_err(|_| MoveRefused::UnknownVnum(stone.vnum))?;
    let mut trail = Trail::default();
    trail.records.push(MoveRecord::Notice(STONE_SET_NOTICE));
    let sockets = items
        .set_socket(target.id, index, vnum)
        .map_err(|_| MoveRefused::Empty)?;
    let stored = items.item(target.id).ok_or(MoveRefused::Empty)?;
    trail.records.push(MoveRecord::Item(ItemRecord::Update(
        stored.gc_item_update(),
    )));
    trail.changes.push(ItemChange::Sockets {
        id: target.id,
        sockets,
    });
    super::potion::use_one(
        items,
        stone,
        super::quickslot::chains_when_used_up(stone_proto),
        &mut trail,
    )?;
    Ok(done(MoveKind::Used, trail))
}
