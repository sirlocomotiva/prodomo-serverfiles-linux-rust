//! The arms of `CHARACTER::UseItemEx` that act on a second item, the one a
//! `CG_ITEM_USE_TO_ITEM` names (`G/char_item.cpp:2718-6990`).
//!
//! `G/` = `server/server/game/`.

use gamedata::item_kind::{
    ARMOR_BODY, ARMOR_EAR, ARMOR_GLOVE, ARMOR_NECK, ARMOR_WRIST, COSTUME_SASH, ITEM_ARMOR,
    ITEM_BELT, ITEM_COSTUME, ITEM_PICK, ITEM_RING, ITEM_ROD, ITEM_TALISMAN, ITEM_WEAPON,
    LIMIT_LEVEL, USE_ADD_ACCESSORY_SOCKET, USE_ADD_ATTRIBUTE, USE_ADD_ATTRIBUTE2,
    USE_ADD_ATTRIBUTE_GLOVE, USE_ADD_ATTRIBUTE_TALISMAN, USE_CHANGE_ATTRIBUTE,
    USE_CHANGE_ATTRIBUTE2, USE_CHANGE_ATTRIBUTE_GLOVE, USE_CHANGE_ATTRIBUTE_TALISMAN,
    USE_PUT_INTO_ACCESSORY_SOCKET, USE_PUT_INTO_BELT_SOCKET, WEARABLE_BODY, WEARABLE_WEAPON,
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
use super::equipment::is_accessory_for_socket;
use super::item_move::{done, MoveDone, MoveKind, MoveRecord};
use super::items::CharacterItems;
use super::{ItemChange, ItemRecord, MoveRefused};
use crate::item::{Item, ItemId, ITEM_FLAG_REFINEABLE, SOCKETS};

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
/// No stone sits in the target (`CHARACTER::RefineItem`, `G/char_item.cpp:2037`).
pub const NO_STONE_NOTICE: &str = "[LS;1108]";
/// `ITEM_BROKEN_METIN_VNUM` (`G/char_item.cpp:59`): what a detached stone's socket
/// keeps.
pub const BROKEN_STONE_SOCKET: i32 = 28_960;
/// `ITEM_STONES_MAX_NUM` with `ENABLE_EXTENDED_SOCKETS` (`C/item_length.h:13-15`):
/// the sockets a detachment scroll reads.
pub const STONE_SOCKETS: usize = 3;
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
/// The socket-adding scroll's success (`:6702`): the socket grade rose.
pub const SOCKET_ADDED_NOTICE: &str = "[LS;406]";
/// The socket-adding scroll's missed 50-in-100 (`:6707`, with
/// `ENABLE_ADDSTONE_FAILURE` defined): the grade stays.
pub const SOCKET_ADD_FAILED_NOTICE: &str = "[LS;407]";
/// The socket-adding scroll on an accessory whose grade is already full (`:6716`).
pub const SOCKET_FULL_NOTICE: &str = "[LS;408]";
/// The socket scroll on a target that takes no accessory socket (`:6721`, `:6758`).
pub const NO_ACCESSORY_SOCKET_NOTICE: &str = "[LS;409]";
/// The jewel sits in the accessory's socket (`:6740`).
pub const JEWEL_SET_NOTICE: &str = "[LS;410]";
/// The jewel's missed roll (`:6746`).
pub const JEWEL_SET_FAILED_NOTICE: &str = "[LS;411]";
/// The jewel on an accessory with no socket opened yet (max grade 0, `:6754`).
pub const NO_OPEN_SOCKET_NOTICE: &str = "[LS;412]";
/// The jewel on an accessory whose sockets are all full but more could open
/// (`:6756-6757`).
pub const OPEN_MORE_FIRST_NOTICES: [&str; 2] = ["[LS;413]", "[LS;415]"];
/// The jewel on a fully opened, fully set accessory (`:6760`).
pub const JEWELS_FULL_NOTICE: &str = "[LS;416]";
/// The jewel for another accessory (`CanPutInto`, `:6765`).
pub const WRONG_JEWEL_NOTICE: &str = "[LS;417]";
/// The heart extractor on a worn stone (`ExtractDragonHeart`, `G/DragonSoul.cpp:415`).
pub const HEART_WORN_NOTICE: &str = "You cannot extract Dragon Soul Stones while wearing them.";
/// The heart gamble's drawn charge is zero (`:459`).
pub const HEART_FAIL_NOTICE: &str = "Extraction of the dragon's heart has failed.";
/// The heart melted (`:485`).
pub const HEART_SUCCESS_NOTICE: &str = "Extraction of dragon heart was successful.";
/// The pull-out with no empty box cell (`PullOut`, `G/DragonSoul.cpp:505`).
pub const PULLOUT_FULL_NOTICE: &str = "There are no empty spaces in your belongings.";
/// The pull-out's draw passed (`:562`).
pub const PULLOUT_SUCCESS_NOTICE: &str = "You have succeeded in extracting the Dragon Soul Stone.";
/// The pull-out's draw missed with no by-product (`:585`, `:588`).
pub const PULLOUT_FAIL_NOTICE: &str = "Extraction of Dragon Soul Stone failed.";
/// A grade-6 stone (`GetBasePosition`, `G/DragonSoul.cpp:140-141`).
pub const DS_GRADES: u8 = 6;
/// A grade box's cells (`DRAGON_SOUL_BOX_SIZE`, `C/length.h:83`).
pub const DS_BOX_CELLS: u16 = 32;
/// The soul inventory's window byte (`DRAGON_SOUL_INVENTORY`, `C/length.h:664`).
pub const DS_WINDOW: u8 = 5;
/// A talisman under a normal bonus scroll (`:6396`, `:6562`).
pub const TALISMAN_ATTR_NOTICE: &str = "You cannot change attribute with this item.";
/// A talisman under a bonus add scroll (`:6562`).
pub const TALISMAN_ADD_NOTICE: &str = "You cannot add attribute with this item.";
/// A talisman already holding five bonuses under its add scroll (`:10674`).
pub const TALISMAN_FULL_NOTICE: &str = "You cannot add another attribute.";
/// The talisman's bonus landed (`:10684`, trailing space in the source).
pub const TALISMAN_ADDED_NOTICE: &str = "Upgrade successfully added. ";
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

/// Whether `stone` is a dragon soul stone: `CItem::IsDragonSoul`
/// (`G/item.cpp:2786-2789`) is the type alone.
#[must_use]
pub fn is_ds_stone(protos: &ItemProtos, stone: &Item) -> bool {
    protos
        .get(stone.vnum)
        .is_some_and(|proto| proto.item_type == gamedata::item_kind::ITEM_DS)
}

/// `DSManager::GetBasePosition` (`G/DragonSoul.cpp:130-147`) with
/// `ENABLE_DRAGONSOUL_ALCHEMY_PLUS`: the sub-type's column of six boxes and the
/// grade's row. A grade past 6 has no box (`row_type > DRAGON_SOUL_GRADE_MAX`).
#[must_use]
pub fn ds_base_cell(vnum: u32, sub_type: i32) -> Option<u16> {
    let grade = u16::try_from((vnum / 1000) % 10).ok()?;
    if grade > u16::from(DS_GRADES) {
        return None;
    }
    let column = u16::try_from(sub_type).ok()?;
    column
        .checked_mul(u16::from(DS_GRADES) * DS_BOX_CELLS)?
        .checked_add(grade * DS_BOX_CELLS)
}

/// Whether `cell` is a valid cell for `stone`: `IsValidCellForThisItem`
/// (`G/DragonSoul.cpp:113-127`) is the soul window and the stone's own box.
#[must_use]
pub fn ds_cell_valid(vnum: u32, sub_type: i32, cell: ItemPos) -> bool {
    if cell.window_type != DS_WINDOW {
        return false;
    }
    let Some(base) = ds_base_cell(vnum, sub_type) else {
        return false;
    };
    (base..base.saturating_add(DS_BOX_CELLS)).contains(&cell.cell)
}

/// The heart extractor's checks (`ExtractDragonHeart`, `G/DragonSoul.cpp:409-437`): the
/// target at `dest`, or `None` for an empty cell or a non-soul target. The worn line
/// is the caller's: this answers the target either way, with whether it is worn.
#[must_use]
pub fn heart_target(
    items: &CharacterItems,
    protos: &ItemProtos,
    dest: ItemPos,
) -> Option<(Item, bool)> {
    let target = items.item_at(dest)?.clone();
    if !is_ds_stone(protos, &target) {
        return None;
    }
    let worn = super::inventory::is_equip_position(target.pos);
    Some((target, worn))
}

/// Spend one of the `stone` and one of the `extractor`, both already read: the two
/// `SetCount(GetCount() - 1)` of the extracts. The kind is the extract's own.
///
/// # Errors
///
/// A storage refusal of a release or a count, which changes nothing.
pub fn spend_extract_pair(
    items: &mut CharacterItems,
    stone: &Item,
    extractor: &Item,
    extractor_proto: &ItemProto,
) -> Result<MoveDone, MoveRefused> {
    let mut trail = Trail::default();
    super::potion::use_one(items, stone, false, &mut trail)?;
    super::potion::use_one(
        items,
        extractor,
        super::quickslot::chains_when_used_up(extractor_proto),
        &mut trail,
    )?;
    Ok(done(MoveKind::Extracted, trail))
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

/// Whether the scroll's sub-type is one of the socket-scroll case's labels
/// (`G/char_item.cpp:6287-6295`, with the aura socket out of this port): the
/// costume-and-glove gate at `:6312-6321` is that case's head, not the kind pairs'.
fn scroll_subtype_uses_socket_case(sub_type: i32) -> bool {
    matches!(
        sub_type,
        USE_PUT_INTO_BELT_SOCKET
            | USE_PUT_INTO_ACCESSORY_SOCKET
            | USE_ADD_ACCESSORY_SOCKET
            | USE_CHANGE_ATTRIBUTE
            | USE_CHANGE_ATTRIBUTE2
            | USE_ADD_ATTRIBUTE
            | USE_ADD_ATTRIBUTE2
    )
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

/// What [`detach_stones`] found to give back: the socket index and the stone vnum
/// each held, in socket order.
pub type DetachedStones = Vec<(usize, u32)>;

/// The `USE_DETACHMENT` arm of `CHARACTER::RefineItem` (`G/char_item.cpp:1996-2040`)
/// once the use's checks passed: every live stone in the first [`STONE_SOCKETS`]
/// sockets of the item at `dest` is named for the give-back, its socket keeps
/// [`BROKEN_STONE_SOCKET`], and one scroll is spent. A stone is a socket past 2
/// that is not the broken stone — the same predicate the refine's keep uses.
///
/// A target [`use_target`] refuses, one without [`ITEM_FLAG_REFINEABLE`], and one
/// whose proto is unknown are refused without a line, as legacy's `else if` falls
/// through. A target with no live stone chats [`NO_STONE_NOTICE`] and spends
/// nothing. The caller gives each named stone back with `AutoGiveItem` and lays
/// what would not fit, in socket order, as legacy gives inside its own loop.
///
/// # Errors
///
/// A storage refusal, which changes nothing.
pub fn detach_stones(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    dest: ItemPos,
    protos: &ItemProtos,
) -> Result<(MoveDone, DetachedStones), MoveRefused> {
    let silent = || Ok((done(MoveKind::Used, Trail::default()), Vec::new()));
    let Some(target) = use_target(items, dest) else {
        return silent();
    };
    let Some(target_proto) = protos.get(target.vnum) else {
        return silent();
    };
    if target_proto.flags & ITEM_FLAG_REFINEABLE == 0 {
        return silent();
    }
    let mut stones = DetachedStones::new();
    for (index, socket) in target.sockets.iter().enumerate().take(STONE_SOCKETS) {
        if *socket > 2 && *socket != BROKEN_STONE_SOCKET {
            let Ok(vnum) = u32::try_from(*socket) else {
                continue;
            };
            stones.push((index, vnum));
        }
    }
    if stones.is_empty() {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(NO_STONE_NOTICE));
        return Ok((done(MoveKind::Used, trail), Vec::new()));
    }
    let mut trail = Trail::default();
    let mut sockets = target.sockets;
    for (index, _) in &stones {
        sockets = items
            .set_socket(target.id, *index, BROKEN_STONE_SOCKET)
            .map_err(|_| MoveRefused::Empty)?;
    }
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
        scroll,
        super::quickslot::chains_when_used_up(scroll_proto),
        &mut trail,
    )?;
    Ok((done(MoveKind::Used, trail), stones))
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
    if scroll_subtype_uses_socket_case(scroll_proto.sub_type)
        && (target_proto.item_type == ITEM_COSTUME
            || (target_proto.item_type == ITEM_ARMOR && target_proto.sub_type == ARMOR_GLOVE))
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
        USE_ADD_ACCESSORY_SOCKET => {
            add_accessory_socket(items, (scroll, scroll_proto), &target, target_proto, dice)
        }
        USE_PUT_INTO_ACCESSORY_SOCKET | USE_PUT_INTO_BELT_SOCKET => {
            put_into_socket(items, (scroll, scroll_proto), &target, target_proto, dice)
        }
        USE_ADD_ATTRIBUTE_TALISMAN => add_kind_target_attribute(
            items,
            (scroll, scroll_proto),
            &target,
            target_proto,
            dice,
            attrs,
            is_talisman,
        ),
        USE_ADD_ATTRIBUTE_GLOVE => add_kind_target_attribute(
            items,
            (scroll, scroll_proto),
            &target,
            target_proto,
            dice,
            attrs,
            is_glove,
        ),
        USE_CHANGE_ATTRIBUTE_TALISMAN => change_kind_target_attribute(
            items,
            (scroll, scroll_proto),
            (&target, target_proto),
            dice,
            gauss,
            attrs,
            is_talisman,
        ),
        USE_CHANGE_ATTRIBUTE_GLOVE => change_kind_target_attribute(
            items,
            (scroll, scroll_proto),
            (&target, target_proto),
            dice,
            gauss,
            attrs,
            is_glove,
        ),
        _ => silent(),
    }
}

/// `CItem::IsTalisman` (`G/item.h:347`): a talisman by item type.
fn is_talisman(proto: &ItemProto) -> bool {
    proto.item_type == ITEM_TALISMAN
}

/// `CItem::IsGloves` (`G/item.h:349`), behind `ENABLE_GLOVE_SYSTEM`: armour of the
/// glove sub-type.
fn is_glove(proto: &ItemProto) -> bool {
    proto.item_type == ITEM_ARMOR && proto.sub_type == ARMOR_GLOVE
}
/// The `USE_ADD_ATTRIBUTE_TALISMAN` and `USE_ADD_ATTRIBUTE_GLOVE` cases
/// (`UseItemTalismanAddAttribute`, `G/char_item.cpp:10670-10703`, and
/// `UseItemGloveAddAttribute`, `:10616-10648`, behind `ENABLE_GLOVE_SYSTEM`): on a
/// target of the scroll's kind with an attribute set and fewer than five bonuses, the
/// always-100 roll lands one more and chats the success line; the scroll is spent
/// either way. At five the full line chats and the scroll is kept. Any other target,
/// an exchanging or worn one, or one with no set is silent, as the `false` return is
/// (`use_target` already refused the exchanging and the worn).
///
/// The fail line's Korean is replacement characters in the frozen source
/// (`:10697-10698`, `:10642-10643`), so its bytes are lost; the roll cannot miss
/// (`aiItemAttributeAddPercent`, all 100, `G/constants.cpp:879-882`), which is why no
/// fail line exists here. The trailing `IsEquipped` re-check is dead (the head refused
/// the worn), as is the buff call behind it.
fn add_kind_target_attribute(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    target: &Item,
    target_proto: &ItemProto,
    dice: &mut dyn Dice,
    attrs: &[ItemAttrRecord],
    is_target: fn(&ItemProto) -> bool,
) -> Result<MoveDone, MoveRefused> {
    let silent = || Ok(done(MoveKind::Used, Trail::default()));
    if !is_target(target_proto)
        || attribute_set_index(target_proto.item_type, target_proto.sub_type).is_none()
    {
        return silent();
    }
    if normal_count(&target.attributes) >= 5 {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(TALISMAN_FULL_NOTICE));
        return Ok(done(MoveKind::Used, trail));
    }
    let mut attributes = target.attributes;
    let mut trail = Trail::default();
    if number(dice, 1, 100) <= 100 {
        add_rolled_attribute(dice, target_proto, &mut attributes, attrs);
        store_attributes(items, target.id, &attributes, target.sockets[CHANGE_SOCKET])?;
        let stored = items.item(target.id).ok_or(MoveRefused::Empty)?;
        trail
            .records
            .push(MoveRecord::Notice(TALISMAN_ADDED_NOTICE));
        trail.records.push(MoveRecord::Item(ItemRecord::Update(
            stored.gc_item_update(),
        )));
        trail.changes.push(ItemChange::Attributes {
            id: target.id,
            attributes: stored.attributes,
        });
    }
    spend_scroll(items, scroll, scroll_proto, &mut trail)?;
    Ok(done(MoveKind::Used, trail))
}

/// The `USE_CHANGE_ATTRIBUTE_TALISMAN` and `USE_CHANGE_ATTRIBUTE_GLOVE` cases
/// (`UseItemTalismanChangeAttribute`, `G/char_item.cpp:10705-10724`, and
/// `UseItemGloveChangeAttribute`, `:10650-10667`, behind `ENABLE_GLOVE_SYSTEM`): on a
/// target of the scroll's kind with an attribute set and at least one bonus, the
/// bonuses redraw on the plain change table and the scroll is spent. Any other target,
/// an exchanging or worn one, one with no set, or one with no bonus is silent. The
/// change-count socket climbs, as `ChangeAttribute` climbs it.
fn change_kind_target_attribute(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    (target, target_proto): (&Item, &ItemProto),
    dice: &mut dyn Dice,
    gauss: &mut Gauss,
    attrs: &[ItemAttrRecord],
    is_target: fn(&ItemProto) -> bool,
) -> Result<MoveDone, MoveRefused> {
    let silent = || Ok(done(MoveKind::Used, Trail::default()));
    if !is_target(target_proto)
        || attribute_set_index(target_proto.item_type, target_proto.sub_type).is_none()
        || normal_count(&target.attributes) == 0
    {
        return silent();
    }
    let mut attributes = target.attributes;
    let changed = change_attribute_on(
        dice,
        gauss,
        target_proto,
        &mut attributes,
        attrs,
        &[0, 10, 40, 35, 15],
    );
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
    spend_scroll(items, scroll, scroll_proto, &mut trail)?;
    Ok(done(MoveKind::Used, trail))
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
/// `aiAccessorySocketPutPct` (`constants.cpp:1172-1175`): the jewel's chance by the
/// accessory's current grade.
const PUT_SOCKET_PCT: [i32; 4] = [90, 80, 70, 0];
/// `aiAccessorySocketDegradeTime` (`constants.cpp:1166-1169`): the downgrade countdown
/// a set grade starts, in seconds.
const DEGRADE_TIME: [i32; 4] = [0, 86_400, 43_200, 21_600];
/// The jewel-accessory rows of `CanPutInto` (`G/item.cpp:2539-2546`): the jewel vnum
/// with the wrist, neck and ear vnum bases it fits.
const JEWEL_TABLE: [(u32, [u32; 3]); 6] = [
    (50_634, [14_420, 16_220, 17_220]),
    (50_635, [14_500, 16_500, 17_500]),
    (50_636, [14_520, 16_520, 17_520]),
    (50_637, [14_540, 16_540, 17_540]),
    (50_638, [14_560, 16_560, 17_560]),
    (50_639, [14_570, 16_570, 17_570]),
];
/// The first jewel of the fallback ladder (`:2611`, `:2629`).
const FIRST_FALLBACK_JEWEL: u32 = 50_623;
/// The two necklace runs with a fixed jewel (`:2613-2627`).
const FIXED_NECKLACES: [(u32, u32); 2] = [(50_625, 16_210), (50_626, 16_230)];

/// The `USE_ADD_ACCESSORY_SOCKET` arm of `UseItemEx` (`G/char_item.cpp:6687-6724`):
/// the scroll opens one more of the accessory's three sockets, and is spent. A
/// non-accessory chats [`NO_ACCESSORY_SOCKET_NOTICE`], a full one
/// [`SOCKET_FULL_NOTICE`]; the roll is 50 in 100 (`ENABLE_ADDSTONE_FAILURE` is
/// defined, `G/char_item.cpp:2717`), chatting [`SOCKET_ADDED_NOTICE`] or
/// [`SOCKET_ADD_FAILED_NOTICE`]. The caller ran the outer costume and glove gate.
fn add_accessory_socket(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    target: &Item,
    target_proto: &ItemProto,
    dice: &mut dyn Dice,
) -> Result<MoveDone, MoveRefused> {
    let refused = |text: &'static str| {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(text));
        Ok(done(MoveKind::Used, trail))
    };
    if !is_accessory_for_socket(target_proto) {
        return refused(NO_ACCESSORY_SOCKET_NOTICE);
    }
    if target.sockets[1].clamp(0, 3) >= 3 {
        return refused(SOCKET_FULL_NOTICE);
    }
    let mut trail = Trail::default();
    if number(dice, 1, 100) <= 50 {
        let sockets = items
            .set_socket(target.id, 1, target.sockets[1].clamp(0, 3) + 1)
            .map_err(|_| MoveRefused::Empty)?;
        let stored = items.item(target.id).ok_or(MoveRefused::Empty)?;
        trail.records.push(MoveRecord::Notice(SOCKET_ADDED_NOTICE));
        trail.records.push(MoveRecord::Item(ItemRecord::Update(
            stored.gc_item_update(),
        )));
        trail.changes.push(ItemChange::Sockets {
            id: target.id,
            sockets,
        });
    } else {
        trail
            .records
            .push(MoveRecord::Notice(SOCKET_ADD_FAILED_NOTICE));
    }
    spend_scroll(items, scroll, scroll_proto, &mut trail)?;
    Ok(done(MoveKind::Used, trail))
}

/// The `USE_PUT_INTO_BELT_SOCKET` and `USE_PUT_INTO_ACCESSORY_SOCKET` arm of
/// `UseItemEx` (`G/char_item.cpp:6725-6766`): the jewel sets one more of the
/// accessory's opened sockets, and is spent. A non-accessory or a jewel for another
/// accessory (`CanPutInto`, `G/item.cpp:2514-2631`) chats [`WRONG_JEWEL_NOTICE`].
/// Else the grade climbs only below its maximum: with no socket opened
/// [`NO_OPEN_SOCKET_NOTICE`] chats, with all set but more to open the two
/// [`OPEN_MORE_FIRST_NOTICES`], and on a full accessory [`JEWELS_FULL_NOTICE`]. A
/// set rolls [`PUT_SOCKET_PCT`] by the current grade (`SetAccessorySocketGrade`,
/// `G/item.cpp:2279-2289`), chatting [`JEWEL_SET_NOTICE`] or
/// [`JEWEL_SET_FAILED_NOTICE`]. The downgrade countdown is socket 2; the expiry
/// timer that spends it (`accessory_socket_expire_event`) stays `unused`, and a
/// graded accessory refuses the wear (`WornSystem::AccessoryTimer`).
fn put_into_socket(
    items: &mut CharacterItems,
    (scroll, scroll_proto): (&Item, &ItemProto),
    target: &Item,
    target_proto: &ItemProto,
    dice: &mut dyn Dice,
) -> Result<MoveDone, MoveRefused> {
    let refused = |text: &'static str| {
        let mut trail = Trail::default();
        trail.records.push(MoveRecord::Notice(text));
        Ok(done(MoveKind::Used, trail))
    };
    if !is_accessory_for_socket(target_proto)
        || !can_put_into(scroll, scroll_proto, target, target_proto)
    {
        return refused(WRONG_JEWEL_NOTICE);
    }
    let max_grade = target.sockets[1].clamp(0, 3);
    let grade = target.sockets[0].clamp(0, max_grade);
    if grade >= max_grade {
        if max_grade == 0 {
            return refused(NO_OPEN_SOCKET_NOTICE);
        }
        if max_grade < 3 {
            let mut trail = Trail::default();
            for line in OPEN_MORE_FIRST_NOTICES {
                trail.records.push(MoveRecord::Notice(line));
            }
            return Ok(done(MoveKind::Used, trail));
        }
        return refused(JEWELS_FULL_NOTICE);
    }
    let mut trail = Trail::default();
    let pct = PUT_SOCKET_PCT[usize::try_from(grade).unwrap_or(3)];
    if number(dice, 1, 100) <= pct {
        let grade = grade + 1;
        items
            .set_socket(target.id, 0, grade)
            .map_err(|_| MoveRefused::Empty)?;
        let countdown = DEGRADE_TIME[usize::try_from(grade).unwrap_or(0)];
        let sockets = items
            .set_socket(target.id, 2, countdown)
            .map_err(|_| MoveRefused::Empty)?;
        let stored = items.item(target.id).ok_or(MoveRefused::Empty)?;
        trail.records.push(MoveRecord::Notice(JEWEL_SET_NOTICE));
        trail.records.push(MoveRecord::Item(ItemRecord::Update(
            stored.gc_item_update(),
        )));
        trail.changes.push(ItemChange::Sockets {
            id: target.id,
            sockets,
        });
    } else {
        trail
            .records
            .push(MoveRecord::Notice(JEWEL_SET_FAILED_NOTICE));
    }
    spend_scroll(items, scroll, scroll_proto, &mut trail)?;
    Ok(done(MoveKind::Used, trail))
}

/// `CItem::CanPutInto` (`G/item.cpp:2514-2631`): whether the `jewel` fits the
/// `target`. A belt takes a belt jewel (`USE_PUT_INTO_BELT_SOCKET`); a ring never
/// takes one (`CanPutIntoRing` always fails); an aura's arm is its system's, and any
/// other non-armour refuses. On a bracelet, necklace or earring the jewel table
/// decides first (the target's vnum rounded down to ten picks the row, and a wrong
/// jewel for a row refuses before the ladder runs), then the fallback ladder: the
/// target's offset from its kind's base over 20 picks the jewel past
/// [`FIRST_FALLBACK_JEWEL`], the two necklace runs take their fixed jewels, and an
/// offset past 11 (`type < 0`, which `u32` never reaches, or past 11) shifts by 170
/// first.
fn can_put_into(
    jewel: &Item,
    jewel_proto: &ItemProto,
    target: &Item,
    target_proto: &ItemProto,
) -> bool {
    if target_proto.item_type == ITEM_BELT {
        return jewel_proto.sub_type == USE_PUT_INTO_BELT_SOCKET;
    }
    if target_proto.item_type == ITEM_RING {
        return false;
    }
    if target_proto.item_type != ITEM_ARMOR {
        return false;
    }
    let slot = match target_proto.sub_type {
        ARMOR_WRIST => 0,
        ARMOR_NECK => 1,
        ARMOR_EAR => 2,
        _ => return false,
    };
    let base = target.vnum / 10 * 10;
    for (fitted, bases) in JEWEL_TABLE {
        if bases[slot] == base {
            return fitted == jewel.vnum;
        }
    }
    let kind_base = [14_000, 16_000, 17_000][slot];
    let Some(offset) = target.vnum.checked_sub(kind_base) else {
        return false;
    };
    let shifted = offset.saturating_sub(170);
    let ladder = |step: u32| FIRST_FALLBACK_JEWEL.saturating_add(step) == jewel.vnum;
    let kind = offset / 20;
    if kind > 11 {
        return ladder(shifted / 20);
    }
    for (fitted, run) in FIXED_NECKLACES {
        if (run..run + 10).contains(&target.vnum) {
            return fitted == jewel.vnum;
        }
    }
    ladder(kind)
}

/// `CItem::GetAttributeCount` (`G/item_attribute.cpp:287-298`): the set normal
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

    /// The scroll on a stoned sword names each live stone in socket order, breaks
    /// each socket, and spends one scroll. A fourth-socket stone is out of the
    /// scroll's reach, and a broken socket stays broken.
    #[test]
    fn a_detachment_scroll_names_the_stones_and_breaks_their_sockets() {
        let mut sword_proto = ItemProto::for_category_rule(SWORD, ITEM_WEAPON, 0);
        sword_proto.flags = ITEM_FLAG_REFINEABLE;
        let scroll_proto = ItemProto::for_category_rule(SCROLL, ITEM_USE, USE_DETACHMENT);
        let protos = ItemProtos::from_rows(vec![sword_proto, scroll_proto]);
        let mut sword = Item::new(20, SWORD);
        sword.sockets[0] = 28_030;
        sword.sockets[1] = BROKEN_STONE_SOCKET;
        sword.sockets[2] = 28_031;
        sword.sockets[3] = 28_032;
        let mut items = holding(&[(0, sword), (1, Item::new(10, SCROLL))]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = protos.get(SCROLL).expect("the scroll proto").clone();
        let (done, stones) = detach_stones(&mut items, (&scroll, &proto), inv(0), &protos)
            .expect("the detach lands");
        assert_eq!(stones, vec![(0, 28_030), (2, 28_031)]);
        let kept = items.item(20).expect("the sword stays");
        assert_eq!(
            kept.sockets[0], BROKEN_STONE_SOCKET,
            "the first socket breaks"
        );
        assert_eq!(
            kept.sockets[2], BROKEN_STONE_SOCKET,
            "the third socket breaks"
        );
        assert_eq!(kept.sockets[3], 28_032, "the fourth socket is out of reach");
        assert!(items.item(10).is_none(), "the last scroll leaves");
        assert!(
            done.changes
                .iter()
                .any(|change| matches!(change, ItemChange::Sockets { id: 20, .. })),
            "the sockets store"
        );
    }

    /// No live stone chats `[LS;1108]` and spends nothing; a target without the
    /// refineable flag refuses silently, as the `else if` falls through.
    #[test]
    fn a_stoneless_sword_chats_and_an_unflagged_one_is_silent() {
        let mut sword_proto = ItemProto::for_category_rule(SWORD, ITEM_WEAPON, 0);
        sword_proto.flags = ITEM_FLAG_REFINEABLE;
        let scroll_proto = ItemProto::for_category_rule(SCROLL, ITEM_USE, USE_DETACHMENT);
        let plain_proto = ItemProto::for_category_rule(3001, ITEM_WEAPON, 0);
        let protos = ItemProtos::from_rows(vec![sword_proto, scroll_proto, plain_proto]);
        let mut items = holding(&[
            (0, Item::new(20, SWORD)),
            (1, Item::new(10, SCROLL)),
            (2, Item::new(21, 3001)),
        ]);
        let scroll = items.item_at(inv(1)).expect("the scroll").clone();
        let proto = protos.get(SCROLL).expect("the scroll proto").clone();
        let (done, stones) = detach_stones(&mut items, (&scroll, &proto), inv(0), &protos)
            .expect("the refusal chats");
        assert!(stones.is_empty(), "nothing to give back");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(NO_STONE_NOTICE))),
            "the stoneless sword chats"
        );
        assert!(items.item(10).is_some(), "the scroll stays");
        let (done, stones) = detach_stones(&mut items, (&scroll, &proto), inv(2), &protos)
            .expect("a silent refusal");
        assert!(stones.is_empty());
        assert!(done.records.is_empty(), "no record for an unflagged target");
        assert!(done.changes.is_empty(), "no change for an unflagged target");
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

    const ADD_SOCKET: u32 = 50_621;
    const JEWEL: u32 = 50_635;
    const WRONG_JEWEL: u32 = 50_636;
    const BRACELET: u32 = 14_502;
    const BELT: u32 = 18_900;
    const BELT_JEWEL: u32 = 18_901;

    fn socket_protos() -> ItemProtos {
        let add = ItemProto::for_category_rule(ADD_SOCKET, ITEM_USE, USE_ADD_ACCESSORY_SOCKET);
        let jewel = ItemProto::for_category_rule(JEWEL, ITEM_USE, USE_PUT_INTO_ACCESSORY_SOCKET);
        let wrong =
            ItemProto::for_category_rule(WRONG_JEWEL, ITEM_USE, USE_PUT_INTO_ACCESSORY_SOCKET);
        let bracelet = ItemProto::for_category_rule(BRACELET, ITEM_ARMOR, ARMOR_WRIST);
        let sword = ItemProto::for_category_rule(SWORD, ITEM_WEAPON, 0);
        let belt = ItemProto::for_category_rule(BELT, ITEM_BELT, 0);
        let belt_proto =
            ItemProto::for_category_rule(BELT_JEWEL, ITEM_USE, USE_PUT_INTO_BELT_SOCKET);
        ItemProtos::from_rows(vec![add, jewel, wrong, bracelet, sword, belt, belt_proto])
    }

    fn bracelet(id: u32, grade: i32, max_grade: i32) -> Item {
        let mut item = Item::new(id, BRACELET);
        item.sockets[0] = grade;
        item.sockets[1] = max_grade;
        item
    }

    fn socket_scroll(
        items: &CharacterItems,
        cell: u16,
        vnum: u32,
        protos: &ItemProtos,
    ) -> (Item, ItemProto) {
        let scroll = items.item_at(inv(cell)).expect("the scroll").clone();
        let proto = protos.get(vnum).expect("the scroll proto").clone();
        (scroll, proto)
    }

    /// A socket scroll on a bracelet with two sockets opened rolls its 50 in 100: a
    /// hit opens the third and spends the scroll, a miss keeps two and spends it; a
    /// full bracelet and a sword refuse without drawing.
    #[test]
    fn a_socket_scroll_opens_one_more_unless_full() {
        let protos = socket_protos();
        let (attrs, rare) = attr_rows();
        let mut gauss = Gauss::new();
        let mut dice = Script::new(vec![49, 50]);
        let mut items = holding(&[(0, bracelet(9, 0, 2)), (1, Item::new(10, ADD_SOCKET))]);
        let (scroll, proto) = socket_scroll(&items, 1, ADD_SOCKET, &protos);
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (0, 0),
            false,
            false,
        )
        .expect("the add runs");
        let kept = items.item(9).expect("the bracelet stays");
        assert_eq!(kept.sockets[1], 3, "the third socket opens");
        assert!(items.item(10).is_none(), "the scroll is spent");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(SOCKET_ADDED_NOTICE))),
            "the success chats"
        );
    }

    /// The add arm's other three answers: a miss spends the scroll and keeps the two
    /// sockets, a full bracelet chats the full line, and a sword (no accessory)
    /// chats the no-socket line; neither refusal spends.
    #[test]
    fn a_socket_scroll_fails_spent_and_refuses_the_full_and_the_sword() {
        let protos = socket_protos();
        let (attrs, rare) = attr_rows();
        // A miss: draw 51 against the 50-in-100.
        let mut gauss = Gauss::new();
        let mut dice = Script::new(vec![50]);
        let mut items = holding(&[(0, bracelet(9, 0, 2)), (1, Item::new(10, ADD_SOCKET))]);
        let (scroll, proto) = socket_scroll(&items, 1, ADD_SOCKET, &protos);
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (0, 0),
            false,
            false,
        )
        .expect("the miss runs");
        assert_eq!(items.item(9).expect("the bracelet stays").sockets[1], 2);
        assert!(items.item(10).is_none(), "the missed scroll is spent");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(SOCKET_ADD_FAILED_NOTICE))),
            "the miss chats"
        );
        // A full bracelet: no draw, no spend.
        let mut items = holding(&[(0, bracelet(9, 0, 3)), (1, Item::new(10, ADD_SOCKET))]);
        let (scroll, proto) = socket_scroll(&items, 1, ADD_SOCKET, &protos);
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (0, 0),
            false,
            false,
        )
        .expect("the full refusal runs");
        assert_eq!(dice.at, 1, "the full bracelet draws nothing");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(SOCKET_FULL_NOTICE))),
            "the full line chats"
        );
        assert!(items.item(10).is_some(), "the refused scroll stays");
        // A sword is no accessory.
        let mut sword = Item::new(9, SWORD);
        sword.sockets[1] = 0;
        let mut items = holding(&[(0, sword), (1, Item::new(10, ADD_SOCKET))]);
        let (scroll, proto) = socket_scroll(&items, 1, ADD_SOCKET, &protos);
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (0, 0),
            false,
            false,
        )
        .expect("the sword refusal runs");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(NO_ACCESSORY_SOCKET_NOTICE))),
            "the no-socket line chats"
        );
        assert!(items.item(10).is_some(), "the refused scroll stays");
    }

    /// A fitted jewel on a bracelet with two sockets opened and none set rolls 90 in
    /// 100: a hit sets the first, starts the 24-hour countdown and spends the jewel.
    #[test]
    fn a_fitted_jewel_sets_the_first_socket() {
        let protos = socket_protos();
        let (attrs, rare) = attr_rows();
        let mut gauss = Gauss::new();
        let mut dice = Script::new(vec![89]);
        let mut items = holding(&[(0, bracelet(9, 0, 2)), (1, Item::new(10, JEWEL))]);
        let (scroll, proto) = socket_scroll(&items, 1, JEWEL, &protos);
        let done = change_target_attributes(
            &mut items,
            (&scroll, &proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &rare,
            (0, 0),
            false,
            false,
        )
        .expect("the set runs");
        let kept = items.item(9).expect("the bracelet stays");
        assert_eq!(kept.sockets[0], 1, "the first socket sets");
        assert_eq!(kept.sockets[2], 86_400, "the countdown starts");
        assert!(items.item(10).is_none(), "the jewel is spent");
        assert!(
            done.changes.iter().any(|change| matches!(
                change,
                ItemChange::Sockets { id: 9, sockets } if sockets[0] == 1
            )),
            "the grade stores"
        );
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(JEWEL_SET_NOTICE))),
            "the success chats"
        );
    }

    /// The jewel arm's refusals: a wrong jewel for the row, an unopened accessory,
    /// a set-full one with more to open, a full one, and a missed roll; only the
    /// miss spends.
    #[test]
    fn a_jewel_refuses_the_wrong_the_unopened_and_the_full() {
        let protos = socket_protos();
        let (attrs, rare) = attr_rows();
        let mut gauss = Gauss::new();
        let mut dice = Script::new(vec![90]);
        let run = |items: &mut CharacterItems,
                   scroll: &Item,
                   proto: &ItemProto,
                   dice: &mut Script,
                   gauss: &mut Gauss,
                   attrs: &[ItemAttrRecord],
                   rare: &[ItemAttrRecord]| {
            change_target_attributes(
                items,
                (scroll, proto),
                inv(0),
                &protos,
                dice,
                gauss,
                attrs,
                rare,
                (0, 0),
                false,
                false,
            )
            .expect("the jewel arm runs")
        };
        // A jewel for another row (50636 against the 14500 base).
        let mut items = holding(&[(0, bracelet(9, 0, 2)), (1, Item::new(10, WRONG_JEWEL))]);
        let (scroll, proto) = socket_scroll(&items, 1, WRONG_JEWEL, &protos);
        let done = run(
            &mut items, &scroll, &proto, &mut dice, &mut gauss, &attrs, &rare,
        );
        assert_eq!(dice.at, 0, "the wrong jewel draws nothing");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(WRONG_JEWEL_NOTICE))),
            "the wrong line chats"
        );
        assert!(items.item(10).is_some(), "the wrong jewel stays");
        // No socket opened yet.
        let mut items = holding(&[(0, bracelet(9, 0, 0)), (1, Item::new(10, JEWEL))]);
        let (scroll, proto) = socket_scroll(&items, 1, JEWEL, &protos);
        let done = run(
            &mut items, &scroll, &proto, &mut dice, &mut gauss, &attrs, &rare,
        );
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(NO_OPEN_SOCKET_NOTICE))),
            "the unopened line chats"
        );
        assert!(items.item(10).is_some(), "the jewel stays");
        // Both opened sockets set, a third still to open: the two lines.
        let mut items = holding(&[(0, bracelet(9, 2, 2)), (1, Item::new(10, JEWEL))]);
        let (scroll, proto) = socket_scroll(&items, 1, JEWEL, &protos);
        let done = run(
            &mut items, &scroll, &proto, &mut dice, &mut gauss, &attrs, &rare,
        );
        for line in OPEN_MORE_FIRST_NOTICES {
            assert!(
                done.records
                    .iter()
                    .any(|record| matches!(record, MoveRecord::Notice(found) if *found == line)),
                "the open-more line {line} chats"
            );
        }
        assert!(items.item(10).is_some(), "the jewel stays");
        // Three set on three opened: full.
        let mut items = holding(&[(0, bracelet(9, 3, 3)), (1, Item::new(10, JEWEL))]);
        let (scroll, proto) = socket_scroll(&items, 1, JEWEL, &protos);
        let done = run(
            &mut items, &scroll, &proto, &mut dice, &mut gauss, &attrs, &rare,
        );
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(JEWELS_FULL_NOTICE))),
            "the full line chats"
        );
        assert!(items.item(10).is_some(), "the jewel stays");
        // A missed 90 in 100 spends the jewel and sets nothing.
        let mut items = holding(&[(0, bracelet(9, 0, 2)), (1, Item::new(10, JEWEL))]);
        let (scroll, proto) = socket_scroll(&items, 1, JEWEL, &protos);
        let done = run(
            &mut items, &scroll, &proto, &mut dice, &mut gauss, &attrs, &rare,
        );
        assert_eq!(items.item(9).expect("the bracelet stays").sockets[0], 0);
        assert!(items.item(10).is_none(), "the missed jewel is spent");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(JEWEL_SET_FAILED_NOTICE))),
            "the miss chats"
        );
    }

    /// The `CanPutInto` ladder past the jewel table: a belt takes a belt jewel and
    /// refuses an accessory one, and the fallback ladder pairs a bracelet below the
    /// table's rows with its computed jewel.
    #[test]
    fn a_belt_and_the_ladder_take_their_jewels() {
        let protos = socket_protos();
        let belt_proto = protos.get(BELT).expect("the belt proto");
        assert!(is_accessory_for_socket(belt_proto));
        let mut belt = Item::new(9, BELT);
        belt.sockets[1] = 1;
        let belt_jewel = Item::new(10, BELT_JEWEL);
        let belt_jewel_proto = protos.get(BELT_JEWEL).expect("the belt jewel proto");
        assert!(can_put_into(
            &belt_jewel,
            belt_jewel_proto,
            &belt,
            belt_proto
        ));
        let jewel = Item::new(11, JEWEL);
        let jewel_proto = protos.get(JEWEL).expect("the jewel proto");
        assert!(!can_put_into(&jewel, jewel_proto, &belt, belt_proto));
        // Below the table: 14020 rounds to the 14020 base, no row matches, and the
        // offset (20) over 20 is 1, so 50624 fits and 50623 does not.
        let low = Item::new(12, 14_020);
        let low_proto = ItemProto::for_category_rule(14_020, ITEM_ARMOR, ARMOR_WRIST);
        let fitted = Item::new(13, 50_624);
        let fitted_proto =
            ItemProto::for_category_rule(50_624, ITEM_USE, USE_PUT_INTO_ACCESSORY_SOCKET);
        assert!(can_put_into(&fitted, &fitted_proto, &low, &low_proto));
        let other = Item::new(14, 50_623);
        let other_proto =
            ItemProto::for_category_rule(50_623, ITEM_USE, USE_PUT_INTO_ACCESSORY_SOCKET);
        assert!(!can_put_into(&other, &other_proto, &low, &low_proto));
    }

    const TALISMAN: u32 = 9600;
    const TALISMAN_ADD: u32 = 50_890;
    const TALISMAN_CHANGE: u32 = 50_889;
    const GLOVE: u32 = 23_000;
    const GLOVE_ADD: u32 = 50_891;
    const GLOVE_CHANGE: u32 = 50_892;

    /// The talisman table: the two scrolls, a fire talisman, and a sword for the
    /// non-talisman silence.
    fn talisman_protos() -> ItemProtos {
        let add = ItemProto::for_category_rule(TALISMAN_ADD, ITEM_USE, USE_ADD_ATTRIBUTE_TALISMAN);
        let change =
            ItemProto::for_category_rule(TALISMAN_CHANGE, ITEM_USE, USE_CHANGE_ATTRIBUTE_TALISMAN);
        let talisman = ItemProto::for_category_rule(TALISMAN, ITEM_TALISMAN, 0);
        let sword = ItemProto::for_category_rule(SWORD, ITEM_WEAPON, 0);
        ItemProtos::from_rows(vec![add, change, talisman, sword])
    }

    /// The glove table: the two scrolls and a glove.
    fn glove_protos() -> ItemProtos {
        let add = ItemProto::for_category_rule(GLOVE_ADD, ITEM_USE, USE_ADD_ATTRIBUTE_GLOVE);
        let change =
            ItemProto::for_category_rule(GLOVE_CHANGE, ITEM_USE, USE_CHANGE_ATTRIBUTE_GLOVE);
        let glove = ItemProto::for_category_rule(GLOVE, ITEM_ARMOR, ARMOR_GLOVE);
        ItemProtos::from_rows(vec![add, change, glove])
    }

    /// The talisman set's row: apply 1 caps set 8 at 5 with the whole probability.
    fn talisman_attrs() -> Vec<ItemAttrRecord> {
        set_attrs(8)
    }

    /// The glove set's row: apply 1 caps set 9 at 5 with the whole probability.
    fn glove_attrs() -> Vec<ItemAttrRecord> {
        set_attrs(9)
    }

    /// One attr row capping `set` at 5 for apply 1 with the whole probability.
    fn set_attrs(set: usize) -> Vec<ItemAttrRecord> {
        let mut levels = [0u8; 10];
        levels[set] = 5;
        vec![ItemAttrRecord {
            apply_index: 1,
            prob: 100,
            values: [6, 6, 8, 10, 12],
            max_level_by_set: levels,
            ..ItemAttrRecord::default()
        }]
    }

    fn bonused_talisman(id: u32) -> Item {
        let mut talisman = Item::new(id, TALISMAN);
        talisman.attributes[0] = ItemAttribute::new(7, 26);
        talisman
    }

    fn bonused_glove(id: u32) -> Item {
        let mut glove = Item::new(id, GLOVE);
        glove.attributes[0] = ItemAttribute::new(7, 26);
        glove
    }

    /// The talisman add lands a second bonus with the success line and spends the
    /// scroll; the change redraws the two with the change line, counts socket 3, and
    /// spends its scroll too.
    #[test]
    fn a_talisman_add_lands_and_its_change_redraws_and_counts() {
        let protos = talisman_protos();
        let attrs = talisman_attrs();
        let mut items = holding(&[
            (0, bonused_talisman(9)),
            (1, Item::new(10, TALISMAN_ADD)),
            (2, Item::new(11, TALISMAN_CHANGE)),
        ]);
        let add = items.item_at(inv(1)).expect("the add scroll").clone();
        let add_proto = protos.get(TALISMAN_ADD).expect("the add proto").clone();
        let mut dice = Script::new(vec![1, 1, 1]);
        let mut gauss = Gauss::new();
        let done = change_target_attributes(
            &mut items,
            (&add, &add_proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &[],
            (0, 0),
            false,
            false,
        )
        .expect("the add lands");
        let kept = items.item(9).expect("the talisman stays");
        assert_eq!(normal_count(&kept.attributes), 2, "the second bonus lands");
        assert!(items.item(10).is_none(), "the last add scroll leaves");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(TALISMAN_ADDED_NOTICE))),
            "the add chats its line"
        );
        let change = items.item_at(inv(2)).expect("the change scroll").clone();
        let change_proto = protos
            .get(TALISMAN_CHANGE)
            .expect("the change proto")
            .clone();
        let before = items.item(9).expect("the talisman").attributes;
        let done = change_target_attributes(
            &mut items,
            (&change, &change_proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &[],
            (0, 0),
            false,
            false,
        )
        .expect("the change lands");
        let kept = items.item(9).expect("the talisman stays");
        assert_ne!(kept.attributes, before, "the bonuses redraw");
        assert_eq!(kept.sockets[3], 1, "the change counts itself");
        assert!(items.item(11).is_none(), "the last change scroll leaves");
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
    }

    /// At five the add chats the full line and keeps the scroll; at four the fifth
    /// still lands, because the cap is five, not four.
    #[test]
    fn a_full_talisman_refuses_while_four_still_lands() {
        let protos = talisman_protos();
        let attrs = talisman_attrs();
        let mut full = bonused_talisman(9);
        for (index, attribute) in full.attributes.iter_mut().enumerate().take(5) {
            let apply = u8::try_from(index + 1).expect("a small apply");
            *attribute = ItemAttribute::new(apply, 6);
        }
        let mut items = holding(&[
            (0, full),
            (1, Item::new(10, TALISMAN_ADD)),
            (3, bonused_sword(12)),
            (4, Item::new(13, TALISMAN_CHANGE)),
        ]);
        let add = items.item_at(inv(1)).expect("the add scroll").clone();
        let add_proto = protos.get(TALISMAN_ADD).expect("the add proto").clone();
        let mut dice = Script::new(vec![1]);
        let mut gauss = Gauss::new();
        let done = change_target_attributes(
            &mut items,
            (&add, &add_proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &[],
            (0, 0),
            false,
            false,
        )
        .expect("the full talisman answers");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(TALISMAN_FULL_NOTICE))),
            "the full line chats"
        );
        assert!(items.item(10).is_some(), "the refused scroll stays");
        // At four the fifth still lands: the cap is five, not four.
        let mut four = bonused_talisman(14);
        for (index, attribute) in four.attributes.iter_mut().enumerate().take(4) {
            let apply = u8::try_from(index + 2).expect("a small apply");
            *attribute = ItemAttribute::new(apply, 6);
        }
        let mut items = holding(&[(0, four), (1, Item::new(10, TALISMAN_ADD))]);
        let add = items.item_at(inv(1)).expect("the add scroll").clone();
        let done = change_target_attributes(
            &mut items,
            (&add, &add_proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &[],
            (0, 0),
            false,
            false,
        )
        .expect("the add lands at four");
        let kept = items.item(14).expect("the talisman stays");
        assert_eq!(normal_count(&kept.attributes), 5, "the fifth lands");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(TALISMAN_ADDED_NOTICE))),
            "the add chats its line"
        );
    }

    /// A sword under the change scroll, and a bonus-less talisman under it too, are
    /// silent with the scroll kept: the first is no talisman, the second has nothing
    /// to redraw.
    #[test]
    fn a_sword_and_an_empty_talisman_are_silent_under_the_change() {
        let protos = talisman_protos();
        let attrs = talisman_attrs();
        let mut dice = Script::new(vec![1]);
        let mut gauss = Gauss::new();
        let mut items = holding(&[(3, bonused_sword(12)), (4, Item::new(13, TALISMAN_CHANGE))]);
        let change = items.item_at(inv(4)).expect("the change scroll").clone();
        let change_proto = protos
            .get(TALISMAN_CHANGE)
            .expect("the change proto")
            .clone();
        let done = change_target_attributes(
            &mut items,
            (&change, &change_proto),
            inv(3),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &[],
            (0, 0),
            false,
            false,
        )
        .expect("the sword answers silently");
        assert!(done.records.is_empty(), "no line for a sword");
        assert!(items.item(13).is_some(), "the silent scroll stays");
        // A bonus-less talisman under the change scroll is silent too, with the
        // scroll kept: there is nothing to redraw.
        let mut items = holding(&[
            (0, Item::new(15, TALISMAN)),
            (4, Item::new(13, TALISMAN_CHANGE)),
        ]);
        let change = items.item_at(inv(4)).expect("the change scroll").clone();
        let done = change_target_attributes(
            &mut items,
            (&change, &change_proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &[],
            (0, 0),
            false,
            false,
        )
        .expect("the empty talisman answers silently");
        assert!(done.records.is_empty(), "no line for no bonus");
        assert!(items.item(13).is_some(), "the silent scroll stays");
    }

    /// The glove add lands a second bonus with the success line and spends the
    /// scroll; the change redraws the two with the change line, counts socket 3, and
    /// spends its scroll too. A talisman under the glove change is silent with the
    /// scroll kept: it is no glove.
    #[test]
    fn a_glove_add_lands_and_its_change_redraws_and_counts() {
        let mut protos = glove_protos();
        let talisman_proto = ItemProto::for_category_rule(TALISMAN, ITEM_TALISMAN, 0);
        protos = ItemProtos::from_rows(vec![
            protos.get(GLOVE_ADD).expect("the add proto").clone(),
            protos.get(GLOVE_CHANGE).expect("the change proto").clone(),
            protos.get(GLOVE).expect("the glove proto").clone(),
            talisman_proto,
        ]);
        let attrs = glove_attrs();
        let mut items = holding(&[
            (0, bonused_glove(9)),
            (1, Item::new(10, GLOVE_ADD)),
            (2, Item::new(11, GLOVE_CHANGE)),
            (3, bonused_talisman(12)),
            (4, Item::new(13, GLOVE_CHANGE)),
        ]);
        let add = items.item_at(inv(1)).expect("the add scroll").clone();
        let add_proto = protos.get(GLOVE_ADD).expect("the add proto").clone();
        let mut dice = Script::new(vec![1, 1, 1]);
        let mut gauss = Gauss::new();
        let done = change_target_attributes(
            &mut items,
            (&add, &add_proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &[],
            (0, 0),
            false,
            false,
        )
        .expect("the add lands");
        let kept = items.item(9).expect("the glove stays");
        assert_eq!(normal_count(&kept.attributes), 2, "the second bonus lands");
        assert!(items.item(10).is_none(), "the last add scroll leaves");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(TALISMAN_ADDED_NOTICE))),
            "the add chats its line"
        );
        let change = items.item_at(inv(2)).expect("the change scroll").clone();
        let change_proto = protos.get(GLOVE_CHANGE).expect("the change proto").clone();
        let before = items.item(9).expect("the glove").attributes;
        let done = change_target_attributes(
            &mut items,
            (&change, &change_proto),
            inv(0),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &[],
            (0, 0),
            false,
            false,
        )
        .expect("the change lands");
        let kept = items.item(9).expect("the glove stays");
        assert_ne!(kept.attributes, before, "the bonuses redraw");
        assert_eq!(kept.sockets[3], 1, "the change counts itself");
        assert!(items.item(11).is_none(), "the last change scroll leaves");
        assert!(
            done.records
                .iter()
                .any(|record| matches!(record, MoveRecord::Notice(ATTR_CHANGED_NOTICE))),
            "the change chats"
        );
        let change = items.item_at(inv(4)).expect("the change scroll").clone();
        let done = change_target_attributes(
            &mut items,
            (&change, &change_proto),
            inv(3),
            &protos,
            &mut dice,
            &mut gauss,
            &attrs,
            &[],
            (0, 0),
            false,
            false,
        )
        .expect("the talisman answers silently");
        assert!(done.records.is_empty(), "no line for a talisman");
        assert!(items.item(13).is_some(), "the silent scroll stays");
    }
}
