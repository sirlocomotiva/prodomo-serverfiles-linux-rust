//! The arms of `CHARACTER::UseItemEx` that act on a second item, the one a
//! `CG_ITEM_USE_TO_ITEM` names (`G/char_item.cpp:2718-6990`).
//!
//! `G/` = `server/server/game/`.

use gamedata::item_kind::{
    COSTUME_SASH, ITEM_ARMOR, ITEM_COSTUME, ITEM_PICK, ITEM_ROD, ITEM_WEAPON, WEARABLE_BODY,
    WEARABLE_WEAPON,
};
use gamedata::item_proto::{ItemProto, ItemProtos};
use protocol::item_pos::ItemPos;

use super::equip::Trail;
use super::item_move::{done, MoveDone, MoveKind, MoveRecord};
use super::items::CharacterItems;
use super::{ItemChange, ItemRecord, MoveRefused};
use crate::item::{Item, ItemId, SOCKETS};

/// The locked target of a transmutation clean (`CleanTransmutation`, `G/char.cpp:10756-10760`).
pub const CLEAN_LOCKED_NOTICE: &str = "You can't remove the transmute because item is locked";

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
/// `SASH_ABSORBED_SOCKET` (`C/item_length.h:505`): socket 1.
const ABSORBED_SOCKET: usize = 1;

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

/// `CleanSashAttr` (`G/char.cpp:10481-10500`): the detachment scroll cleans the absorbed
/// socket and every attribute of the sash at `dest`, and one scroll is spent. The
/// `CanHandleItem` gate is the caller's; a target [`use_target`] refuses, one that is no
/// costume sash (legacy reads the target's type `&&` subtype, a Defect: every costume
/// that is no sash passes), and one with no absorbed socket are refused without a line.
///
/// The `ITEM_FLAG_REFINEABLE` check of the `USE_DETACHMENT` arm (`G/char_item.cpp:1996`)
/// is not here: the flag bit is not ported, and every owner proto the arm could clean
/// would pass it. Recorded in `docs/STATUS.md`.
///
/// # Errors
///
/// A storage refusal, which changes nothing.
pub fn clean_sash_attr(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    dest: ItemPos,
    protos: &ItemProtos,
) -> Result<MoveDone, MoveRefused> {
    let silent = || Ok(done(MoveKind::Used, Trail::default()));
    let Some(target) = use_target(items, dest) else {
        return silent();
    };
    let Some(target_proto) = protos.get(target.vnum) else {
        return silent();
    };
    if target_proto.item_type != ITEM_COSTUME && target_proto.sub_type != COSTUME_SASH {
        return silent();
    }
    if target.sockets[ABSORBED_SOCKET] <= 0 {
        return silent();
    }
    let mut trail = Trail::default();
    let sockets = items
        .set_socket(target.id, ABSORBED_SOCKET, 0)
        .map_err(|_| MoveRefused::Empty)?;
    let mut attributes = target.attributes;
    for (index, attribute) in attributes.iter_mut().enumerate() {
        *attribute = protocol::gc_item_window::ItemAttribute::new(0, 0);
        let _ = items.set_attribute(target.id, index, *attribute);
    }
    let stored = items.item(target.id).ok_or(MoveRefused::Empty)?;
    trail.records.push(MoveRecord::Item(ItemRecord::Update(
        stored.gc_item_update(),
    )));
    trail.changes.push(ItemChange::Sockets {
        id: target.id,
        sockets,
    });
    trail.changes.push(ItemChange::Attributes {
        id: target.id,
        attributes,
    });
    super::potion::use_one(
        items,
        scroll,
        super::quickslot::chains_when_used_up(scroll_proto),
        &mut trail,
    )?;
    Ok(done(MoveKind::Used, trail))
}

/// `CleanTransmutation` (`G/char.cpp:10746-10768`): the detachment scroll cleans the
/// transmutation of the weapon, armour or costume at `dest` back to none, and one scroll
/// is spent. The `CanHandleItem` gate is the caller's; a target [`use_target`] refuses,
/// one that is no weapon, armour or costume, and one with no transmutation are refused
/// without a line. A held target chats [`CLEAN_LOCKED_NOTICE`].
///
/// The caller answers the lock: this reports it through [`MoveRefused::Locked`], which no
/// other arm of the use-on-an-item path raises.
///
/// The `ITEM_FLAG_REFINEABLE` check of the `USE_DETACHMENT` arm (`G/char_item.cpp:1996`)
/// is not here: the flag bit is not ported, and every owner proto the arm could clean
/// would pass it. Recorded in `docs/STATUS.md`.
///
/// # Errors
///
/// [`MoveRefused::Locked`] for a held target, and a storage refusal, which changes
/// nothing.
pub fn clean_transmutation(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    dest: ItemPos,
    protos: &ItemProtos,
    locked: impl Fn(ItemId) -> bool,
) -> Result<MoveDone, MoveRefused> {
    let silent = || Ok(done(MoveKind::Used, Trail::default()));
    let Some(target) = use_target(items, dest) else {
        return silent();
    };
    let Some(target_proto) = protos.get(target.vnum) else {
        return silent();
    };
    if target_proto.item_type != ITEM_WEAPON
        && target_proto.item_type != ITEM_ARMOR
        && target_proto.item_type != ITEM_COSTUME
    {
        return silent();
    }
    if locked(target.id) {
        return Err(MoveRefused::Locked);
    }
    if target.transmutation == 0 {
        return silent();
    }
    let mut trail = Trail::default();
    let _ = items.set_transmutation(target.id, 0);
    let stored = items.item(target.id).ok_or(MoveRefused::Empty)?;
    trail.records.push(MoveRecord::Item(ItemRecord::Update(
        stored.gc_item_update(),
    )));
    trail.changes.push(ItemChange::Transmuted {
        id: target.id,
        look: 0,
    });
    super::potion::use_one(
        items,
        scroll,
        super::quickslot::chains_when_used_up(scroll_proto),
        &mut trail,
    )?;
    Ok(done(MoveKind::Used, trail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamedata::item_kind::{COSTUME_BODY, ITEM_ROD, ITEM_USE, ITEM_WEAPON, USE_DETACHMENT};
    use gamedata::item_proto::ItemProtos;
    use protocol::gc_item_window::ItemAttribute;

    const SASH: u32 = 85_001;
    const SCROLL: u32 = 71_999;

    fn protos() -> ItemProtos {
        let sash = ItemProto::for_category_rule(SASH, ITEM_COSTUME, COSTUME_SASH);
        let mut scroll = ItemProto::for_category_rule(SCROLL, ITEM_USE, USE_DETACHMENT);
        scroll.values[0] = 7;
        let body = ItemProto::for_category_rule(11, ITEM_COSTUME, COSTUME_BODY);
        let sword = ItemProto::for_category_rule(3000, ITEM_WEAPON, 0);
        ItemProtos::from_rows(vec![sash, scroll, body, sword])
    }

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(1, cell)
    }

    fn holding(placed: &[(u16, Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (cell, item) in placed {
            items.set(inv(*cell), item).expect("the fixture places");
        }
        items
    }

    fn sash(id: u32, absorbed: i32) -> Item {
        let mut item = Item::new(id, SASH);
        item.sockets[1] = absorbed;
        item.attributes[0] = ItemAttribute::new(7, 26);
        item
    }

    /// The scroll on an absorbed sash zeroes socket 1 and every attribute, and one
    /// scroll is spent.
    #[test]
    fn a_detachment_scroll_cleans_an_absorbed_sash() {
        let protos = protos();
        let mut items = holding(&[(0, sash(9, 5000)), (1, Item::new(10, SCROLL))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = protos.get(SCROLL).expect("the scroll proto").clone();
        let done = clean_sash_attr(&mut items, (&scroll, &proto), inv(0), &protos)
            .expect("the clean lands");
        let kept = items.item(9).expect("the sash stays");
        assert_eq!(kept.sockets[1], 0);
        assert!(kept
            .attributes
            .iter()
            .all(|attr| *attr == ItemAttribute::new(0, 0)));
        assert!(items.item(10).is_none(), "the last scroll leaves");
        assert!(
            done.changes
                .iter()
                .any(|change| matches!(change, ItemChange::Sockets { id: 9, .. })),
            "the sockets store"
        );
        assert!(
            done.changes
                .iter()
                .any(|change| matches!(change, ItemChange::Attributes { id: 9, .. })),
            "the attributes store"
        );
    }

    /// A transmuted weapon cleans back to none: the update record, the `Transmuted`
    /// change, and one scroll spent. A locked target refuses through `Locked`; an
    /// untransmuted weapon and a non-weapon/armour/costume refuse silently.
    #[test]
    fn a_clean_takes_the_transmutation_off() {
        let protos = protos();
        let mut sword = Item::new(20, 3000);
        sword.transmutation = 3010;
        let mut items = holding(&[(0, sword), (1, Item::new(10, SCROLL))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = protos.get(SCROLL).expect("the scroll proto").clone();
        let done = clean_transmutation(&mut items, (&scroll, &proto), inv(0), &protos, |_| false)
            .expect("the clean lands");
        assert!(!done.records.is_empty(), "the clean sends its update");
        assert_eq!(items.item(20).expect("the sword stays").transmutation, 0);
        assert!(
            done.changes
                .iter()
                .any(|change| matches!(change, ItemChange::Transmuted { id: 20, look: 0 })),
            "the look stores"
        );
        let done = clean_transmutation(&mut items, (&scroll, &proto), inv(0), &protos, |_| false)
            .expect("a silent refusal");
        assert!(done.records.is_empty(), "no record for a clean sword");
        assert!(done.changes.is_empty(), "no change for a clean sword");
    }

    /// A held target refuses through `Locked`, and a rod (no weapon, armour or costume)
    /// refuses silently.
    #[test]
    fn a_clean_refuses_a_locked_target_and_a_rod() {
        let mut rod_proto = ItemProto::for_category_rule(31, ITEM_ROD, 0);
        rod_proto.values[0] = 0;
        let protos = ItemProtos::from_rows(vec![
            ItemProto::for_category_rule(3000, ITEM_WEAPON, 0),
            ItemProto::for_category_rule(SCROLL, ITEM_USE, USE_DETACHMENT),
            rod_proto,
        ]);
        let mut sword = Item::new(20, 3000);
        sword.transmutation = 3010;
        let mut rod = Item::new(21, 31);
        rod.transmutation = 3010;
        let mut items = holding(&[(0, sword), (1, Item::new(10, SCROLL)), (2, rod)]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = protos.get(SCROLL).expect("the scroll proto").clone();
        assert_eq!(
            clean_transmutation(&mut items, (&scroll, &proto), inv(0), &protos, |id| id
                == 20),
            Err(MoveRefused::Locked),
            "the held sword locks"
        );
        let done = clean_transmutation(&mut items, (&scroll, &proto), inv(2), &protos, |_| false)
            .expect("a silent refusal");
        assert!(done.records.is_empty(), "no record for a rod");
        assert!(done.changes.is_empty(), "no change for a rod");
    }

    /// A sash with no absorbed socket and a non-costume refuse, each silently: no
    /// record, no change. A costume that is no sash passes the gate (legacy's `&&`),
    /// then refuses on its empty absorbed socket — the class the `&&`-to-`||` mutant
    /// hides is the absorbed non-sash costume, which the clean must take.
    #[test]
    fn a_clean_refuses_a_non_costume_and_an_unabsorbed_sash() {
        let protos = protos();
        let mut body = Item::new(11, 11);
        body.sockets[1] = 5000;
        let mut items = holding(&[(0, sash(9, 0)), (1, Item::new(10, SCROLL)), (2, body)]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = protos.get(SCROLL).expect("the scroll proto").clone();
        for dest in [inv(0), inv(2)] {
            if dest == inv(2) {
                // The absorbed body passes the `&&` gate and is cleaned, as legacy
                // cleans it.
                let done = clean_sash_attr(&mut items, (&scroll, &proto), dest, &protos)
                    .expect("the non-sash costume cleans");
                assert!(!done.records.is_empty(), "the clean sends its update");
                assert_eq!(items.item(11).expect("the body stays").sockets[1], 0);
                continue;
            }
            let done = clean_sash_attr(&mut items, (&scroll, &proto), dest, &protos)
                .expect("a silent refusal");
            assert!(done.records.is_empty(), "no record for {dest:?}");
            assert!(done.changes.is_empty(), "no change for {dest:?}");
        }
    }
}
