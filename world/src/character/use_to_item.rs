//! The arms of `CHARACTER::UseItemEx` that act on a second item, the one a
//! `CG_ITEM_USE_TO_ITEM` names (`G/char_item.cpp:2718-6990`).
//!
//! `G/` = `server/server/game/`.

use gamedata::item_kind::{
    ARMOR_BODY, ARMOR_GLOVE, COSTUME_SASH, ITEM_ARMOR, ITEM_COSTUME, ITEM_PICK, ITEM_ROD,
    ITEM_TALISMAN, ITEM_WEAPON, LIMIT_LEVEL, USE_ADD_ATTRIBUTE, USE_ADD_ATTRIBUTE2,
    USE_CHANGE_ATTRIBUTE, USE_CHANGE_ATTRIBUTE2, WEARABLE_BODY, WEARABLE_WEAPON,
};
use gamedata::item_proto::{ItemProto, ItemProtos};
use gamedata::records::ItemAttrRecord;
use protocol::gc_item_window::ItemAttribute;
use protocol::item_pos::ItemPos;

use super::dice::{number, Dice};
use super::drop_item::{
    add_rolled_attribute, attribute_set_index, change_attribute_on, change_rare_attribute,
    golden_change_levels, Gauss,
};
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
/// No attribute set touches this target (`G/char_item.cpp:6383`): a costume or a glove,
/// or `GetAttributeSetIndex() == -1` for the four bonus scrolls.
pub const NO_ATTR_SET_NOTICE: &str = "[LS;396]";
/// The change scroll on an item with no attribute (`:6390`), the rare change on one
/// with no rare attribute (`:6516`).
pub const NO_ATTR_TO_CHANGE_NOTICE: &str = "[LS;397]";
/// A mortal's change before the cycle ends (`:6423`), with the cycle and the seconds
/// left.
pub const ATTR_CHANGE_WAIT_NOTICE: &str = "[LS;398;%d;%d]";
/// The bonuses changed (`:6490`), and the rare ones (`:6532`).
pub const ATTR_CHANGED_NOTICE: &str = "[LS;399]";
/// The bonus was added (`:6603`), and the fifth one (`:6654`).
pub const ATTR_ADDED_NOTICE: &str = "[LS;400]";
/// The add missed (`:6621`), and the fifth one (`:6669`).
pub const ATTR_ADD_FAILED_NOTICE: &str = "[LS;401]";
/// Four normal bonuses already (`:6633`).
pub const ATTR_FULL_NOTICE: &str = "[LS;402]";
/// Five normal bonuses already (`:6676`).
pub const ATTR_FIVE_NOTICE: &str = "[LS;404]";
/// Fewer than four normal bonuses for a fifth-bonus scroll (`:6680`).
pub const ATTR_NEED_FOUR_NOTICE: &str = "[LS;405]";
/// A shade scroll on an item past level 40 (`:6452`, `:6593`).
pub const ATTR_SHADE_LEVEL_NOTICE: &str = "[LS;1064]";
/// A shade scroll on a non-weapon, non-body target (`:6461`, `:6602`).
pub const ATTR_SHADE_TYPE_NOTICE: &str = "[LS;1065]";
/// A talisman under a normal bonus scroll (`:6396`, `:6562`).
pub const TALISMAN_ATTR_NOTICE: &str = "You cannot change attribute with this item.";
/// A talisman under a bonus add scroll (`:6562`).
pub const TALISMAN_ADD_NOTICE: &str = "You cannot add attribute with this item.";
/// The rare change of an item whose rare slots hold nothing to redraw (`:6539`).
pub const RARE_UNCHANGED_NOTICE: &str = "Nu exista bonusuri de schimbat";
/// The 72351's missed 10-in-100 (`:6544`).
pub const RARE_CHANGE_FAILED_NOTICE: &str = "Imbunatatirea a esuat.";
/// The change-count socket (`ChangeAttribute`, `G/item_attribute.cpp:264`): socket 3.
const CHANGE_SOCKET: usize = 3;
/// The golden change scroll (76014, `G/char_item.cpp:6455-6461`).
const GOLDEN_CHANGE_VNUM: u32 = 76_014;
/// The shade change scrolls (71151, 76023, `:6441`).
const SHADE_CHANGE_VNUMS: [u32; 2] = [71_151, 76_023];
/// The shade add scrolls (71152, 76024, `:6574`).
const SHADE_ADD_VNUMS: [u32; 2] = [71_152, 76_024];
/// The 10-in-100 rare change scroll (72351, `:6523`).
const WEAK_RARE_CHANGE_VNUM: u32 = 72_351;

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

/// The four bonus scrolls (`USE_CHANGE_ATTRIBUTE`, `USE_ADD_ATTRIBUTE`,
/// `USE_ADD_ATTRIBUTE2`, `USE_CHANGE_ATTRIBUTE2`) of `UseItemEx`
/// (`G/char_item.cpp:6291-6688`) on the item at `dest`, with the shared gates of the
/// socket-scroll case (`:6296-6321`): a target [`use_target`] refuses is silent, and a
/// costume or a glove chats [`NO_ATTR_SET_NOTICE`].
///
/// The caller has already run the change scroll's cooldown (a mortal's
/// `Item.LastChangeItemAttr` flag against `g_dwItemBonusChangeTime`, `:6401-6430`), and
/// passes the seconds in `cooldown`: 0 runs the scroll, a longer wait chats
/// [`ATTR_CHANGE_WAIT_NOTICE`] with the cycle and the seconds left, and legacy's
/// flag-stamping before the roll is the caller's too. `dice` draws every roll in
/// legacy's order, and `gauss` feeds an addon target's redraw; `attrs` and `rare` are
/// the `item_attr` and `item_attr_rare` rows.
///
/// The cases, in legacy's order:
///
/// - A change scroll (`:6382-6506`) on a target with no attribute set or no attribute
///   chats its notice; on a talisman it chats [`TALISMAN_ATTR_NOTICE`]. Else the
///   attributes redraw on `tmpChangeProb`, 76014's on [`golden_change_levels`], the
///   shade scrolls only on a weapon or body armour whose level limits are at most 40,
///   and the change counts itself in socket 3. The scroll is spent unless `vrajeste`
///   holds. The dead `ENABLE_LEGENDARY_SWITCHERS` arms stay out (the flag is nowhere
///   defined) with the dead inner `USE_CHANGE_ATTRIBUTE2`-subtype test, which the
///   `switch` already sorted away.
/// - A rare change (`:6508-6548`, behind `__ATTR_6TH_7TH__`, which is defined) on a
///   target with no rare attribute chats [`NO_ATTR_TO_CHANGE_NOTICE`]; 72351 runs on
///   10 in 100, every other vnum always. A miss chats [`RARE_CHANGE_FAILED_NOTICE`];
///   a redraw that adds nothing back chats [`RARE_UNCHANGED_NOTICE`], else
///   [`ATTR_CHANGED_NOTICE`]. The scroll is always spent.
/// - An add (`:6550-6636`) on four bonuses chats [`ATTR_FULL_NOTICE`]; below four the
///   shade scrolls keep their gates and the add lands always (`aiItemAttributeAddPercent`
///   is all 100, `G/constants.cpp:879-882`) on the table's level walk, chatting
///   [`ATTR_ADDED_NOTICE`] or [`ATTR_ADD_FAILED_NOTICE`]. The scroll is spent unless
///   `intarire` holds.
/// - A fifth-bonus add (`:6638-6688`) wants four normal bonuses exactly: five chats
///   [`ATTR_FIVE_NOTICE`], fewer [`ATTR_NEED_FOUR_NOTICE`], and any other count (past
///   five, which creation never holds) only logs, drawing nothing and spending
///   nothing. At four the scroll may fail its roll, chatting [`ATTR_ADD_FAILED_NOTICE`];
///   on success the scroll's own attribute count sets the adds
///   (`abs(2 - count)`, so a plain scroll adds two — a legacy Defect: this runs
///   `AddAttribute()` past the fifth, whose write lands nowhere while the success
///   chats) and [`ATTR_ADDED_NOTICE`] chats. The scroll is always spent.
///
/// Every attribute write after a clear lands through `SetAttribute`'s update, and the
/// target's `Attributes` change stores the row. As the GM's attribute lines, the points
/// the target gives are not recomputed, because legacy does not.
///
/// # Errors
///
/// A storage refusal, which changes nothing.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
pub fn change_target_attributes(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    dest: ItemPos,
    protos: &ItemProtos,
    dice: &mut dyn Dice,
    gauss: &mut Gauss,
    attrs: &[ItemAttrRecord],
    rare: &[ItemAttrRecord],
    attribute_cooldown: (u32, u32),
    vrajeste: bool,
    intarire: bool,
) -> Result<MoveDone, MoveRefused> {
    let silent = || Ok(done(MoveKind::Used, Trail::default()));
    let Some(target) = use_target(items, dest) else {
        return silent();
    };
    let Some(target_proto) = protos.get(target.vnum) else {
        return silent();
    };
    if target_proto.item_type == ITEM_COSTUME
        || (target_proto.item_type == ITEM_ARMOR && target_proto.sub_type == ARMOR_GLOVE)
    {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(NO_ATTR_SET_NOTICE));
        return Ok(done(MoveKind::Used, trail));
    }
    match scroll_proto.sub_type {
        USE_CHANGE_ATTRIBUTE => change_normal_attribute(
            items,
            (scroll, scroll_proto),
            &target,
            target_proto,
            dice,
            gauss,
            attrs,
            attribute_cooldown,
            vrajeste,
        ),
        USE_CHANGE_ATTRIBUTE2 => change_rare_target_attribute(
            items,
            (scroll, scroll_proto),
            &target,
            target_proto,
            dice,
            rare,
        ),
        USE_ADD_ATTRIBUTE => add_normal_target_attribute(
            items,
            (scroll, scroll_proto),
            &target,
            target_proto,
            dice,
            attrs,
            intarire,
        ),
        USE_ADD_ATTRIBUTE2 => add_fifth_target_attribute(
            items,
            (scroll, scroll_proto),
            &target,
            target_proto,
            dice,
            attrs,
        ),
        _ => silent(),
    }
}

/// The shared normal-scroll refusals: no attribute set, or the talisman's own line.
fn normal_scroll_refusal(target_proto: &ItemProto, talisman: &'static str) -> Option<&'static str> {
    if attribute_set_index(target_proto.item_type, target_proto.sub_type).is_none() {
        return Some(NO_ATTR_SET_NOTICE);
    }
    if target_proto.item_type == ITEM_TALISMAN {
        return Some(talisman);
    }
    None
}

/// The shade scrolls' target gates (`G/char_item.cpp:6441-6464`, `:6573-6606`): a
/// weapon or body armour whose `LIMIT_LEVEL` rows are all at most 40.
fn shade_allowed(target_proto: &ItemProto, scroll: u32, shades: [u32; 2]) -> Option<&'static str> {
    if !shades.contains(&scroll) {
        return None;
    }
    let weapon_or_body = target_proto.item_type == ITEM_WEAPON
        || (target_proto.item_type == ITEM_ARMOR && target_proto.sub_type == ARMOR_BODY);
    if !weapon_or_body {
        return Some(ATTR_SHADE_TYPE_NOTICE);
    }
    if target_proto
        .limits
        .iter()
        .any(|limit| limit.kind == LIMIT_LEVEL && limit.value > 40)
    {
        return Some(ATTR_SHADE_LEVEL_NOTICE);
    }
    None
}

/// The `USE_CHANGE_ATTRIBUTE` case (`G/char_item.cpp:6382-6506`).
#[allow(clippy::too_many_arguments)]
fn change_normal_attribute(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    target: &Item,
    target_proto: &ItemProto,
    dice: &mut dyn Dice,
    gauss: &mut Gauss,
    attrs: &[ItemAttrRecord],
    (cycle, wait): (u32, u32),
    vrajeste: bool,
) -> Result<MoveDone, MoveRefused> {
    let refused = |text: &'static str| {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(text));
        Ok(done(MoveKind::Used, trail))
    };
    if let Some(line) = normal_scroll_refusal(target_proto, TALISMAN_ATTR_NOTICE) {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(line));
        return Ok(done(MoveKind::Used, trail));
    }
    let count = normal_count(&target.attributes);
    if count == 0 {
        return refused(NO_ATTR_TO_CHANGE_NOTICE);
    }
    if wait > 0 {
        return Err(MoveRefused::AttrChangeWait { cycle, wait });
    }
    if let Some(line) = shade_allowed(target_proto, scroll.vnum, SHADE_CHANGE_VNUMS) {
        return refused(line);
    }
    let mut attributes = target.attributes;
    let table = if scroll.vnum == GOLDEN_CHANGE_VNUM {
        golden_change_levels()
    } else {
        [0, 10, 40, 35, 15]
    };
    let changed = change_attribute_on(dice, gauss, target_proto, &mut attributes, attrs, &table);
    debug_assert!(changed, "the count was read above");
    store_attributes(
        items,
        target.id,
        &attributes,
        target.sockets[CHANGE_SOCKET] + 1,
    )?;
    let stored = items.item(target.id).ok_or(MoveRefused::Empty)?;
    let mut trail = Trail::default();
    trail.records.push(MoveRecord::Notice(ATTR_CHANGED_NOTICE));
    trail.records.push(MoveRecord::Item(ItemRecord::Update(
        stored.gc_item_update(),
    )));
    trail.changes.push(ItemChange::Sockets {
        id: target.id,
        sockets: stored.sockets,
    });
    trail.changes.push(ItemChange::Attributes {
        id: target.id,
        attributes: stored.attributes,
    });
    if !vrajeste {
        spend_scroll(items, scroll, scroll_proto, &mut trail)?;
    }
    Ok(done(MoveKind::Used, trail))
}

/// The `USE_CHANGE_ATTRIBUTE2` case (`G/char_item.cpp:6508-6548`).
fn change_rare_target_attribute(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    target: &Item,
    target_proto: &ItemProto,
    dice: &mut dyn Dice,
    rare: &[ItemAttrRecord],
) -> Result<MoveDone, MoveRefused> {
    let refused = |text: &'static str| {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(text));
        Ok(done(MoveKind::Used, trail))
    };
    if attribute_set_index(target_proto.item_type, target_proto.sub_type).is_none() {
        return refused(NO_ATTR_SET_NOTICE);
    }
    if rare_count_of(&target.attributes) == 0 {
        return refused(NO_ATTR_TO_CHANGE_NOTICE);
    }
    let chance = i32::from(scroll.vnum == WEAK_RARE_CHANGE_VNUM) * -90 + 100;
    if number(dice, 1, 100) > chance {
        let mut trail = Trail::default();
        trail
            .records
            .push(MoveRecord::Notice(RARE_CHANGE_FAILED_NOTICE));
        spend_scroll(items, scroll, scroll_proto, &mut trail)?;
        return Ok(done(MoveKind::Used, trail));
    }
    let mut attributes = target.attributes;
    let writes = change_rare_attribute(dice, target_proto, &mut attributes, rare);
    let Some(writes) = writes else {
        return refused(NO_ATTR_TO_CHANGE_NOTICE);
    };
    store_attributes(items, target.id, &attributes, target.sockets[CHANGE_SOCKET])?;
    let stored = items.item(target.id).ok_or(MoveRefused::Empty)?;
    let mut trail = Trail::default();
    trail.records.push(MoveRecord::Notice(if writes.is_empty() {
        RARE_UNCHANGED_NOTICE
    } else {
        ATTR_CHANGED_NOTICE
    }));
    trail.records.push(MoveRecord::Item(ItemRecord::Update(
        stored.gc_item_update(),
    )));
    trail.changes.push(ItemChange::Attributes {
        id: target.id,
        attributes: stored.attributes,
    });
    spend_scroll(items, scroll, scroll_proto, &mut trail)?;
    Ok(done(MoveKind::Used, trail))
}

/// The `USE_ADD_ATTRIBUTE` case (`G/char_item.cpp:6550-6636`).
#[allow(clippy::too_many_arguments)]
fn add_normal_target_attribute(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    target: &Item,
    target_proto: &ItemProto,
    dice: &mut dyn Dice,
    attrs: &[ItemAttrRecord],
    intarire: bool,
) -> Result<MoveDone, MoveRefused> {
    let refused = |text: &'static str| {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(text));
        Ok(done(MoveKind::Used, trail))
    };
    if let Some(line) = normal_scroll_refusal(target_proto, TALISMAN_ADD_NOTICE) {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(line));
        return Ok(done(MoveKind::Used, trail));
    }
    let count = normal_count(&target.attributes);
    if count >= 4 {
        return refused(ATTR_FULL_NOTICE);
    }
    if let Some(line) = shade_allowed(target_proto, scroll.vnum, SHADE_ADD_VNUMS) {
        return refused(line);
    }
    let mut attributes = target.attributes;
    let mut trail = Trail::default();
    if number(dice, 1, 100) <= 100 {
        add_rolled_attribute(dice, target_proto, &mut attributes, attrs);
        store_attributes(items, target.id, &attributes, target.sockets[CHANGE_SOCKET])?;
        let stored = items.item(target.id).ok_or(MoveRefused::Empty)?;
        trail.records.push(MoveRecord::Notice(ATTR_ADDED_NOTICE));
        trail.records.push(MoveRecord::Item(ItemRecord::Update(
            stored.gc_item_update(),
        )));
        trail.changes.push(ItemChange::Attributes {
            id: target.id,
            attributes: stored.attributes,
        });
    } else {
        trail
            .records
            .push(MoveRecord::Notice(ATTR_ADD_FAILED_NOTICE));
    }
    if !intarire {
        spend_scroll(items, scroll, scroll_proto, &mut trail)?;
    }
    Ok(done(MoveKind::Used, trail))
}

/// The `USE_ADD_ATTRIBUTE2` case (`G/char_item.cpp:6638-6688`).
fn add_fifth_target_attribute(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    target: &Item,
    target_proto: &ItemProto,
    dice: &mut dyn Dice,
    attrs: &[ItemAttrRecord],
) -> Result<MoveDone, MoveRefused> {
    let refused = |text: &'static str| {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(text));
        Ok(done(MoveKind::Used, trail))
    };
    if attribute_set_index(target_proto.item_type, target_proto.sub_type).is_none() {
        return refused(NO_ATTR_SET_NOTICE);
    }
    let count = normal_count(&target.attributes);
    if count == 5 {
        return refused(ATTR_FIVE_NOTICE);
    }
    if count < 4 {
        return refused(ATTR_NEED_FOUR_NOTICE);
    }
    if count != 4 {
        return Ok(done(MoveKind::Used, Trail::default()));
    }
    let mut attributes = target.attributes;
    let mut trail = Trail::default();
    if number(dice, 1, 100) <= 100 {
        let held = items.item(scroll.id).map(|held| held.attributes);
        let scroll_count = held.map_or(0, |attrs| normal_count(&attrs));
        for _ in 0..2i32.saturating_sub(scroll_count).unsigned_abs() {
            add_rolled_attribute(dice, target_proto, &mut attributes, attrs);
        }
        store_attributes(items, target.id, &attributes, target.sockets[CHANGE_SOCKET])?;
        let stored = items.item(target.id).ok_or(MoveRefused::Empty)?;
        trail.records.push(MoveRecord::Notice(ATTR_ADDED_NOTICE));
        trail.records.push(MoveRecord::Item(ItemRecord::Update(
            stored.gc_item_update(),
        )));
        trail.changes.push(ItemChange::Attributes {
            id: target.id,
            attributes: stored.attributes,
        });
    } else {
        trail
            .records
            .push(MoveRecord::Notice(ATTR_ADD_FAILED_NOTICE));
    }
    spend_scroll(items, scroll, scroll_proto, &mut trail)?;
    Ok(done(MoveKind::Used, trail))
}

/// `CItem::GetAttributeCount` (`G/item_attribute.cpp:287-298`): the set normal
/// attributes from slot 0.
fn normal_count(attributes: &[ItemAttribute]) -> i32 {
    let count = attributes
        .iter()
        .take(5)
        .take_while(|attr| attr.b_type != 0)
        .count();
    i32::try_from(count).unwrap_or(i32::MAX)
}

/// `CItem::GetRareAttrCount` (`G/item_attribute.cpp:383-394`): the set rare attributes,
/// wherever they stand in the two slots.
fn rare_count_of(attributes: &[ItemAttribute]) -> usize {
    attributes
        .iter()
        .skip(5)
        .take(2)
        .filter(|attr| attr.b_type != 0)
        .count()
}

/// The silent attribute clear and the socket count of a scroll's redraw: every slot of
/// `attributes` lands, and socket 3 takes `counted`.
fn store_attributes(
    items: &mut CharacterItems,
    id: ItemId,
    attributes: &[ItemAttribute; 7],
    counted: i32,
) -> Result<(), MoveRefused> {
    for (index, attribute) in attributes.iter().enumerate() {
        items
            .set_attribute(id, index, *attribute)
            .map_err(|_| MoveRefused::Empty)?;
    }
    items
        .set_socket(id, CHANGE_SOCKET, counted)
        .map_err(|_| MoveRefused::Empty)?;
    Ok(())
}

/// `item->SetCount(item->GetCount() - 1)` for a spent scroll: the last one leaves its
/// cell, with its slots following the next of its vnum when the proto chains them.
fn spend_scroll(
    items: &mut CharacterItems,
    scroll: &Item,
    scroll_proto: &ItemProto,
    trail: &mut Trail,
) -> Result<(), MoveRefused> {
    super::potion::use_one(
        items,
        scroll,
        super::quickslot::chains_when_used_up(scroll_proto),
        trail,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamedata::item_kind::{COSTUME_BODY, ITEM_ROD, ITEM_USE, ITEM_WEAPON, USE_DETACHMENT};
    use gamedata::item_proto::ItemProtos;
    use protocol::gc_item_window::ItemAttribute;

    const SASH: u32 = 85_001;
    const SCROLL: u32 = 71_999;
    const SWORD: u32 = 3000;
    const CHANGE: u32 = 71_084;
    const ADD: u32 = 71_085;
    const FIFTH: u32 = 39_004;
    const RARE_CHANGE: u32 = 72_346;

    /// The small attribute tables the scroll tests roll on: one normal row capping the
    /// weapon set at 5, one rare row capping it at 5.
    fn attr_rows() -> (Vec<ItemAttrRecord>, Vec<ItemAttrRecord>) {
        let mut normal_levels = [0u8; 10];
        normal_levels[0] = 5;
        let normal = ItemAttrRecord {
            apply_index: 1,
            prob: 100,
            values: [6, 6, 8, 10, 12],
            max_level_by_set: normal_levels,
            ..ItemAttrRecord::default()
        };
        let mut rare_levels = [0u8; 10];
        rare_levels[0] = 5;
        let rare = ItemAttrRecord {
            apply_index: 2,
            values: [100, 200, 300, 400, 500],
            max_level_by_set: rare_levels,
            ..ItemAttrRecord::default()
        };
        (vec![normal], vec![rare])
    }

    fn scroll_protos() -> ItemProtos {
        let mut change = ItemProto::for_category_rule(CHANGE, ITEM_USE, USE_CHANGE_ATTRIBUTE);
        change.values[0] = 0;
        let add = ItemProto::for_category_rule(ADD, ITEM_USE, USE_ADD_ATTRIBUTE);
        let fifth = ItemProto::for_category_rule(FIFTH, ITEM_USE, USE_ADD_ATTRIBUTE2);
        let rare = ItemProto::for_category_rule(RARE_CHANGE, ITEM_USE, USE_CHANGE_ATTRIBUTE2);
        let sword = ItemProto::for_category_rule(SWORD, ITEM_WEAPON, 0);
        ItemProtos::from_rows(vec![change, add, fifth, rare, sword])
    }

    fn bonused_sword(id: u32) -> Item {
        let mut sword = Item::new(id, SWORD);
        sword.attributes[0] = ItemAttribute::new(7, 26);
        sword.attributes[1] = ItemAttribute::new(3, 8);
        sword
    }

    /// A source that answers a fixed sequence, then repeats its last number.
    struct Script {
        draws: Vec<u32>,
        at: usize,
    }

    impl Script {
        fn new(draws: Vec<u32>) -> Self {
            Self { draws, at: 0 }
        }
    }

    impl Dice for Script {
        fn random31(&mut self) -> u32 {
            let draw = self.draws[self.at.min(self.draws.len() - 1)];
            self.at += 1;
            draw
        }
    }

    /// A change scroll redraws the sword's two bonuses, counts the change in socket 3,
    /// and spends itself; a second change on the same item chats the wait refusal when
    /// the caller passes a wait.
    #[test]
    fn a_change_scroll_redraws_and_counts_the_change() {
        let protos = scroll_protos();
        let (attrs, rare) = attr_rows();
        let mut items = holding(&[(0, bonused_sword(9)), (1, Item::new(10, CHANGE))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = protos.get(CHANGE).expect("the scroll proto").clone();
        let mut dice = Script::new(vec![49, 0, 49, 0]);
        let mut gauss = Gauss::new();
        let before = items.item(9).expect("the sword").attributes;
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (60, 0),
            false,
            false,
        )
        .expect("the change lands");
        let kept = items.item(9).expect("the sword stays");
        assert_eq!(kept.sockets[3], 1, "the change counts itself");
        assert_ne!(kept.attributes, before, "the bonuses redraw");
        assert!(items.item(10).is_none(), "the last scroll leaves");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(ATTR_CHANGED_NOTICE))),
            "the change chats"
        );
        assert!(
            done.changes
                .iter()
                .any(|change| matches!(change, ItemChange::Attributes { id: 9, .. })),
            "the attributes store"
        );
        // The cooldown the caller passes refuses before the draw.
        let mut items = holding(&[(0, bonused_sword(9)), (1, Item::new(10, CHANGE))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let err = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (60, 41),
            false,
            false,
        )
        .expect_err("the wait refuses");
        assert_eq!(
            err,
            MoveRefused::AttrChangeWait {
                cycle: 60,
                wait: 41
            },
            "the wait rides the refusal"
        );
        assert!(items.item(10).is_some(), "the refused scroll stays");
    }

    /// An add scroll lands a third bonus on the sword and spends itself; a costume
    /// target chats the no-set line and spends nothing.
    #[test]
    fn an_add_scroll_lands_and_a_costume_refuses() {
        let protos = scroll_protos();
        let (attrs, rare) = attr_rows();
        let mut items = holding(&[(0, bonused_sword(9)), (1, Item::new(10, ADD))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = protos.get(ADD).expect("the scroll proto").clone();
        let mut dice = Script::new(vec![49, 29, 0]);
        let mut gauss = Gauss::new();
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (60, 0),
            false,
            false,
        )
        .expect("the add lands");
        let kept = items.item(9).expect("the sword stays");
        assert_ne!(
            kept.attributes[2],
            ItemAttribute::new(0, 0),
            "the third bonus lands"
        );
        assert!(items.item(10).is_none(), "the last scroll leaves");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(ATTR_ADDED_NOTICE))),
            "the add chats"
        );
        // A costume has no attribute set for these scrolls.
        let mut body_proto = ItemProto::for_category_rule(11, ITEM_COSTUME, COSTUME_BODY);
        body_proto.values[0] = 0;
        let costume_protos = ItemProtos::from_rows(vec![
            body_proto,
            ItemProto::for_category_rule(ADD, ITEM_USE, USE_ADD_ATTRIBUTE),
        ]);
        let mut items = holding(&[(2, Item::new(11, 11)), (1, Item::new(10, ADD))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = costume_protos.get(ADD).expect("the add proto").clone();
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(2),
            &costume_protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (60, 0),
            false,
            false,
        )
        .expect("the refusal chats");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(NO_ATTR_SET_NOTICE))),
            "the costume chats the no-set line"
        );
        assert!(done.changes.is_empty(), "no change for a costume");
    }

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

    /// A fifth-bonus scroll on a four-bonus sword adds one more (the scroll holds no
    /// attributes, so `abs(2 - 0)` runs `AddAttribute` twice, and the second lands
    /// nowhere); on a two-bonus sword it chats the need-four line and spends nothing.
    #[test]
    fn a_fifth_scroll_wants_four_bonuses() {
        let protos = scroll_protos();
        let (attrs, rare) = attr_rows();
        let mut sword = bonused_sword(9);
        sword.attributes[2] = ItemAttribute::new(5, 10);
        sword.attributes[3] = ItemAttribute::new(9, 12);
        let mut items = holding(&[(0, sword), (1, Item::new(10, FIFTH))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = protos.get(FIFTH).expect("the scroll proto").clone();
        let mut dice = Script::new(vec![49, 29, 0, 29, 0]);
        let mut gauss = Gauss::new();
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (60, 0),
            false,
            false,
        )
        .expect("the fifth lands");
        let kept = items.item(9).expect("the sword stays");
        assert_eq!(
            kept.attributes[4],
            ItemAttribute::new(1, 6),
            "the fifth bonus lands"
        );
        assert!(items.item(10).is_none(), "the last scroll leaves");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(ATTR_ADDED_NOTICE))),
            "the add chats"
        );
        // Two bonuses: the need-four line, and the scroll stays.
        let mut items = holding(&[(0, bonused_sword(9)), (1, Item::new(10, FIFTH))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (60, 0),
            false,
            false,
        )
        .expect("the refusal chats");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(ATTR_NEED_FOUR_NOTICE))),
            "the need-four line chats"
        );
        assert!(items.item(10).is_some(), "the refused scroll stays");
    }

    /// A rare change redraws the sword's rare slot; a sword with no rare attribute chats
    /// the no-attribute line and spends nothing.
    #[test]
    fn a_rare_change_redraws_the_rare_slot() {
        let protos = scroll_protos();
        let (attrs, rare) = attr_rows();
        let mut sword = bonused_sword(9);
        sword.attributes[5] = ItemAttribute::new(2, 100);
        let mut items = holding(&[(0, sword), (1, Item::new(10, RARE_CHANGE))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = protos.get(RARE_CHANGE).expect("the scroll proto").clone();
        let mut dice = Script::new(vec![49, 0]);
        let mut gauss = Gauss::new();
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (60, 0),
            false,
            false,
        )
        .expect("the rare change lands");
        let kept = items.item(9).expect("the sword stays");
        assert_eq!(kept.attributes[5].b_type, 2, "the rare slot redraws");
        assert!(items.item(10).is_none(), "the last scroll leaves");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(ATTR_CHANGED_NOTICE))),
            "the change chats"
        );
        // No rare attribute: the no-attribute line, and the scroll stays.
        let mut items = holding(&[(0, bonused_sword(9)), (1, Item::new(10, RARE_CHANGE))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (60, 0),
            false,
            false,
        )
        .expect("the refusal chats");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(NO_ATTR_TO_CHANGE_NOTICE))),
            "the no-attribute line chats"
        );
        assert!(items.item(10).is_some(), "the refused scroll stays");
    }
}
